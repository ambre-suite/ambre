// ambre-server/src/ws/handler/message.rs
// Per-message WebSocket handlers (subscribe/update/ephemeral/request-missing +
// channel variants), access/signature verification, and their unit tests.

use super::*;

/// Orders a replay page and keeps the run of deltas before the first one that
/// could not be read. Returns that run and the unreadable seq, if any, so the
/// page never carries a hole the caller cannot see (`SRV-041`).
pub(crate) fn contiguous_prefix(
    mut fetched: Vec<(u64, Option<ambre_protocol::SeqUpdate>)>,
) -> (Vec<ambre_protocol::SeqUpdate>, Option<u64>) {
    fetched.sort_by_key(|(seq, _)| *seq);
    let mut prefix = Vec::with_capacity(fetched.len());
    for (seq, update) in fetched {
        match update {
            Some(update) => prefix.push(update),
            None => return (prefix, Some(seq)),
        }
    }
    (prefix, None)
}

/// The frame an update refused at clock validation sends back.
///
/// A refused clock is a conflict the client acts on, so it carries the expected
/// and the received clock (`SRV-037`). A store failure reaches the client as one
/// generic message, and its name and text stay in the log (`SRV-081`, `SRV-020`).
pub(crate) fn update_refusal(doc_id: &DocumentId, e: &AppError) -> WsServerMsg {
    match e {
        AppError::ClockMismatch { .. } => WsServerMsg::Error {
            code: 409,
            message: e.to_string(),
        },
        // The document is being deleted (`SRV-017`).
        AppError::NotFound { .. } => WsServerMsg::Error {
            code: 404,
            message: e.to_string(),
        },
        _ => {
            tracing::error!("clock validation failed doc={doc_id}: {e}");
            WsServerMsg::Error {
                code: 500,
                message: "Internal server error".into(),
            }
        }
    }
}

pub(crate) async fn handle_subscribe(
    state: &Arc<AppState>,
    conn: &mut WsConnectionState,
    document_id: DocumentId,
    outbound_tx: &mpsc::Sender<Vec<u8>>,
) {
    let role = match verify_document_access(&state.db, document_id, conn.user_id).await {
        Ok(role) => role,
        Err(e) => {
            let err = encode_server_msg(&WsServerMsg::Error {
                code: 403,
                message: e.to_string(),
            });
            let _ = outbound_tx.send(err).await;
            return;
        }
    };
    // Rehydrate the clock-validation state if Redis is cold but Postgres kept the delta
    // history (for example a Redis restart). Clients replay their persisted deltas on
    // reconnect, and advance their local clock past what a freshly-empty Redis knows.
    // Without reseeding, their next update fails CLOCK_MISMATCH forever. Done before the
    // client can send any update (subscribe precedes updates), and idempotent (MAX
    // semantics), so concurrent subscribers cannot lower a value.
    match redis_ops::clock_state_is_cold(&state.redis, &document_id).await {
        Ok(true) => match db::get_clock_state(&state.db, &document_id).await {
            Ok(clock_state) => {
                let author_clocks: Vec<(String, i64)> = clock_state
                    .author_clocks
                    .iter()
                    .map(|(pk, clock)| (hex::encode(pk), *clock))
                    .collect();
                if let Err(e) = redis_ops::seed_clock_state(
                    &state.redis,
                    &document_id,
                    clock_state.max_seq,
                    &author_clocks,
                )
                .await
                {
                    tracing::error!(error = %e, "subscribe: clock state rehydrate failed");
                }
            }
            Err(e) => tracing::error!(error = %e, "subscribe: clock state load failed"),
        },
        Ok(false) => {}
        Err(e) => tracing::error!(error = %e, "subscribe: clock state cold check failed"),
    }

    conn.doc_subscriptions.insert(document_id);
    conn.doc_access_verified_at
        .insert(document_id, Instant::now());
    // Cache the write capability so `handle_update` can reject a read-only
    // (`viewer`) member's delta without re-hitting the database per message.
    conn.doc_write_allowed
        .insert(document_id, role_can_write(&role));
    let ack = encode_server_msg(&WsServerMsg::Subscribed { document_id });
    let _ = outbound_tx.send(ack).await;
}

pub(crate) fn handle_unsubscribe(conn: &mut WsConnectionState, document_id: DocumentId) {
    conn.doc_subscriptions.remove(&document_id);
    conn.doc_access_verified_at.remove(&document_id);
    conn.doc_write_allowed.remove(&document_id);
}

/// Tell everyone still watching `document_id` that this connection's peer
/// stopped (`PRESENCE-014`): drop their presence field and broadcast one
/// `PeerLeft`. Called on a clean unsubscribe and once per subscription when the
/// connection closes.
///
/// The message carries the document id and the author key, both of which the
/// relay already routes on, and nothing else (`ZK-001`). Fire-and-forget: a
/// failed Redis call leaves the peer to age out of the live set, which is what
/// happened before this message existed.
pub(crate) async fn announce_departure(
    state: &Arc<AppState>,
    document_id: &DocumentId,
    signing_pubkey: &ambre_core::PublicKey,
) {
    let _ =
        redis_ops::doc_presence_remove(&state.redis, document_id, &signing_pubkey.to_hex()).await;
    let broadcast = encode_server_msg(&WsServerMsg::PeerLeft {
        document_id: *document_id,
        pub_key: signing_pubkey.clone(),
    });
    let _ = redis_ops::doc_broadcast(&state.redis, document_id, &broadcast).await;
}

pub(crate) async fn handle_update(
    state: &Arc<AppState>,
    conn: &mut WsConnectionState,
    update: ambre_protocol::Update,
    outbound_tx: &mpsc::Sender<Vec<u8>>,
) {
    let doc_id = update.public_data.document_id;
    if !conn.doc_subscriptions.contains(&doc_id) {
        let err = encode_server_msg(&WsServerMsg::Error {
            code: 403,
            message: "not subscribed".into(),
        });
        let _ = outbound_tx.send(err).await;
        return;
    }

    // Cap the inline delta ciphertext before any signature check or durable
    // write. A CRDT delta is small; a large blob belongs on the media path. This
    // stops an authorized writer from pushing oversized deltas. Such deltas are
    // persisted durably (S3 + Redis pending) and fanned out to every subscriber.
    // That is the same storage/cost abuse the media and snapshot caps close, on
    // the hot path.
    let cipher_len = update.ciphertext.0.len();
    if cipher_len > state.config.delta_max_bytes {
        let err = encode_server_msg(&WsServerMsg::Error {
            code: 413,
            message: format!(
                "delta ciphertext {} bytes exceeds max {}",
                cipher_len, state.config.delta_max_bytes
            ),
        });
        let _ = outbound_tx.send(err).await;
        return;
    }

    // Reject a structurally valid but semantically meaningless delta (empty
    // ciphertext, zero clock) before any DB/Redis work. Cheap, and it keeps the
    // durable index and the per-author clock hash free of no-op entries.
    if let Err(e) = update.validate() {
        let err = encode_server_msg(&WsServerMsg::Error {
            code: 400,
            message: format!("invalid update: {e}"),
        });
        let _ = outbound_tx.send(err).await;
        return;
    }

    // Re-validate membership on a short TTL (H8). The `Subscribe`-time check is
    // stale after a mid-session revoke, so a revoked member could otherwise keep
    // writing accepted, sequenced updates until they disconnect. On failure the
    // helper drops the subscription so subsequent messages are rejected without a
    // DB hit; tell this client its access was revoked.
    if !membership_still_valid(state, conn, doc_id).await {
        let err = encode_server_msg(&WsServerMsg::Error {
            code: 403,
            message: "document access revoked".into(),
        });
        let _ = outbound_tx.send(err).await;
        return;
    }

    // Enforce the write role (owner/editor). Every REST write path gates on the
    // same set. Without this the hot WS write path would let a `viewer` author
    // sequenced, persisted, broadcast deltas. A viewer is a legitimately invited
    // read-only member who holds the doc key and its own signing key. The
    // capability is cached at Subscribe and refreshed by `membership_still_valid`
    // above, so this is a map lookup, not a DB hit.
    if conn.doc_write_allowed.get(&doc_id) != Some(&true) {
        let err = encode_server_msg(&WsServerMsg::Error {
            code: 403,
            message: "read-only role cannot write to this document".into(),
        });
        let _ = outbound_tx.send(err).await;
        return;
    }

    let author_pubkey = update.public_data.pub_key.clone();
    // Bind the authored key to the session identity. The current model is one
    // signing key per user. That key is `users.signing_pubkey`, carried in the
    // token's `spk` claim, so a member may only author under their own key.
    // Without this a member could sign valid updates under unlimited
    // self-generated keys. Each key adds a field to the never-pruned
    // `doc:{id}:clocks` hash (unbounded Redis growth), and attributes edits to a
    // forged identity. Reject the mismatch before the signature check so the
    // clock hash only ever sees known keys.
    if author_pubkey != conn.signing_pubkey {
        let err = encode_server_msg(&WsServerMsg::Error {
            code: 403,
            message: "author key does not match session identity".into(),
        });
        let _ = outbound_tx.send(err).await;
        return;
    }
    if let Err(e) = verify_update_signature(&author_pubkey, &update) {
        let err = encode_server_msg(&WsServerMsg::Error {
            code: 401,
            message: format!("invalid signature: {e}"),
        });
        let _ = outbound_tx.send(err).await;
        return;
    }

    let pub_key_hex = author_pubkey.to_hex();
    let clock = update.public_data.clock.0;
    let snapshot_id = update.public_data.snapshot_id;

    let seq =
        match redis_ops::verify_clock_and_assign_seq(&state.redis, &doc_id, &pub_key_hex, clock)
            .await
        {
            Ok(s) => {
                tracing::debug!(
                    "[Server] seq assigned doc={doc_id} author={pub_key_hex} clock={clock} seq={s}"
                );
                s
            }
            Err(e) => {
                let _ = outbound_tx
                    .send(encode_server_msg(&update_refusal(&doc_id, &e)))
                    .await;
                return;
            }
        };

    let s3_key = crate::s3::format_s3_delta_key(&state.config.s3_env_prefix, &doc_id, seq);

    // Serialize the full sealed Update up front: it is both what we dead-letter on
    // an index-write failure and what we persist for verbatim replay below.
    // Encoding a just-decoded Update is effectively infallible. But a request-path
    // `.expect()` would panic the whole connection task on the off chance it
    // isn't. Log and abort just this update instead: the seq is already assigned,
    // so a reconnecting client refetches it from the durable index.
    let update_wire = match postcard::to_allocvec(&update) {
        Ok(wire) => wire,
        Err(e) => {
            tracing::error!("postcard encode Update failed doc={doc_id} seq={seq}: {e}");
            return;
        }
    };

    tracing::debug!("[Server->Postgres] insert_update doc={doc_id} seq={seq} clock={clock}");
    if let Err(e) = db::insert_update(
        &state.db,
        &doc_id,
        author_pubkey.as_array(),
        &snapshot_id,
        clock as i64,
        seq as i64,
        &s3_key,
    )
    .await
    {
        // The delta index (Postgres) is the ONLY thing `handle_request_missing`
        // reads to replay this seq. The clock was already consumed atomically in
        // Redis, so we cannot cleanly reject and let the client retry. A retry at
        // the same clock would fail CLOCK_MISMATCH. If we merely logged, the delta
        // would still reach S3 via the pending list but no index row would ever
        // reference it. That is an orphaned, unreplayable object: live subscribers
        // get it while a reconnecting client misses it forever. Dead-letter the
        // full wire so an operator can rebuild the index row (H1) instead of losing
        // it.
        tracing::error!("insert_update failed doc={doc_id} seq={seq}: {e}");
        if let Err(dl) =
            redis_ops::push_dead_letter(&state.redis, &doc_id, seq, &s3_key, &update_wire).await
        {
            tracing::error!("dead-letter push failed doc={doc_id} seq={seq}: {dl}");
        }
    }

    tracing::debug!("[Server->Redis] append_pending_delta doc={doc_id} seq={seq} s3_key={s3_key}");
    match redis_ops::append_pending_delta(&state.redis, &doc_id, seq, &s3_key, &update_wire).await {
        Ok(()) => {}
        // The document is being deleted: the delta has nowhere to go and must
        // not be kept anywhere, nor broadcast (`SRV-017`).
        Err(AppError::NotFound { .. }) => {
            tracing::debug!("update dropped: document being deleted doc={doc_id} seq={seq}");
            return;
        }
        Err(e) => {
            // The pending list is the only path this sequenced delta takes to S3. A
            // failed push loses it durably after the seq was already assigned and
            // will be acknowledged. Log loudly and dead-letter the full wire so an
            // operator can replay it (H1).
            tracing::error!("append_pending_delta failed doc={doc_id} seq={seq}: {e}");
            if let Err(dl) =
                redis_ops::push_dead_letter(&state.redis, &doc_id, seq, &s3_key, &update_wire).await
            {
                tracing::error!("dead-letter push failed doc={doc_id} seq={seq}: {dl}");
            }
        }
    }

    let broadcast = encode_server_msg(&WsServerMsg::Update {
        seq: ambre_protocol::messages::Seq(seq),
        update,
    });
    tracing::debug!("[Server->RedisPubSub] broadcast Update doc={doc_id} seq={seq}");
    if let Err(e) = redis_ops::doc_broadcast(&state.redis, &doc_id, &broadcast).await {
        // A dropped broadcast only delays live delivery: subscribers replay the
        // missed seq via RequestMissing on reconnect, and the delta is already
        // persisted above. Still surface it rather than swallowing (H1).
        tracing::error!("doc_broadcast failed doc={doc_id} seq={seq}: {e}");
    }
}

pub(crate) async fn handle_ephemeral(
    state: &Arc<AppState>,
    conn: &mut WsConnectionState,
    eph: ambre_protocol::Ephemeral,
) {
    use base64::Engine;
    let doc_id = eph.public_data.document_id;
    if !conn.doc_subscriptions.contains(&doc_id) {
        return;
    }

    // Cap the ephemeral ciphertext before any broadcast or Redis write. A presence
    // blob is tiny. This mirrors the `handle_update` delta cap for the
    // fire-and-forget path. An oversized ephemeral cannot be fanned out to every
    // subscriber and stored in the presence hash. Fire-and-forget: drop silently.
    if eph.ciphertext.0.len() > state.config.ephemeral_max_bytes {
        return;
    }

    // Reject a structurally valid but meaningless presence blob (empty ciphertext,
    // zero counter) before broadcasting or writing the presence hash. Fire-and-
    // forget, so drop silently like the other ephemeral rejections.
    if eph.validate().is_err() {
        return;
    }

    // Re-check membership on the same short TTL as `handle_update` (H8): a member
    // revoked mid-session must stop broadcasting presence too, not only sequenced
    // updates. Fire-and-forget, so a failed recheck just drops silently.
    if !membership_still_valid(state, conn, doc_id).await {
        return;
    }

    // Check the ephemeral author key against the session identity, like the
    // `handle_update` check. The presence hash `doc:{id}:presence` is indexed by
    // `pub_key_hex` and each field self-expires via per-field `HEXPIRE`. Suppose
    // a member could use any key. They could then attribute a cursor to another
    // member's identity, overwrite that member's presence entry, and add
    // unlimited fields to the presence hash. Each user has one signing key (`spk`
    // claim), so a member may only send presence under their own key. Ephemerals
    // are fire-and-forget (the client expects no reply here), so drop a mismatch
    // silently without an error message.
    if eph.public_data.pub_key != conn.signing_pubkey {
        return;
    }

    // Verify the envelope signature, like `handle_update`. The relay must not
    // broadcast (or record in the presence hash) a ciphertext the author key did
    // not actually sign. An unsigned blob would be rejected by every recipient on
    // open. Fire-and-forget: a bad signature is dropped silently.
    if verify_ephemeral_signature(&eph.public_data.pub_key, &eph).is_err() {
        return;
    }

    let pub_key_hex = eph.public_data.pub_key.to_hex();
    let presence_enc_b64 = base64::engine::general_purpose::URL_SAFE.encode(&eph.ciphertext.0);
    let _ =
        redis_ops::doc_presence_update(&state.redis, &doc_id, &pub_key_hex, &presence_enc_b64, 15)
            .await;

    let broadcast = encode_server_msg(&WsServerMsg::Ephemeral(eph));
    let _ = redis_ops::doc_broadcast(&state.redis, &doc_id, &broadcast).await;
}

pub(crate) async fn handle_request_missing(
    state: &Arc<AppState>,
    conn: &mut WsConnectionState,
    document_id: DocumentId,
    after_seq: ambre_protocol::messages::Seq,
    outbound_tx: &mpsc::Sender<Vec<u8>>,
) {
    if !conn.doc_subscriptions.contains(&document_id) {
        return;
    }

    // Throttle replays (M-amp): one full paged replay per reconnect is legitimate,
    // but an unthrottled `RequestMissing{after_seq:0}` is an O(history) S3-read
    // amplification. Drop calls that arrive within REPLAY_MIN_INTERVAL of the last
    // served replay for this document.
    if let Some(last) = conn.last_replay_at.get(&document_id)
        && last.elapsed() < REPLAY_MIN_INTERVAL
    {
        return;
    }
    conn.last_replay_at.insert(document_id, Instant::now());

    // Re-check membership on the short TTL (H8): a member revoked mid-session must
    // not be able to keep pulling the full history down from storage.
    if !membership_still_valid(state, conn, document_id).await {
        return;
    }

    // Load the still-pending (not-yet-flushed) deltas ONCE as a `seq -> wire` map.
    // Each replayed seq is resolved from S3 first, and falls back to this map. The
    // old per-seq lookup issued a full `LRANGE` of the whole pending list for
    // every missed seq inside the concurrent fan-out. That is O(N^2) on the
    // pre-flush window. A single load makes the fallback a map lookup.
    let pending = match redis_ops::get_pending_deltas_map(&state.redis, &document_id).await {
        Ok(m) => m,
        Err(e) => {
            tracing::error!(error = %e, "replay: pending map load failed");
            HashMap::new()
        }
    };

    // Page through the durable delta index in bounded chunks, emitting one
    // `MissingUpdates` per page. A single hard-capped batch silently truncated a
    // client more than PAGE_SIZE deltas behind. The client applies the batch and
    // declares itself hydrated, never re-requesting the tail. Paging replays the
    // full history in order across several messages instead.
    const PAGE_SIZE: i64 = 1000;
    let mut cursor = after_seq.0 as i64;
    let mut first_page = true;
    loop {
        let updates = match db::get_updates_after(&state.db, &document_id, cursor, PAGE_SIZE).await
        {
            Ok(u) => u,
            Err(e) => {
                tracing::error!(error = %e, "failed to get updates after seq");
                return;
            }
        };
        if updates.is_empty() {
            // Nothing left to replay. A later page simply stops here. But an
            // EMPTY first page must still be answered: that is the common
            // F5-on-an-existing-document case, where the client's snapshot
            // already covers every delta. The client gates its
            // hydration-complete signal on receiving a `MissingUpdates`, so
            // silence here leaves its loading overlay spinning forever. Send
            // an empty page to release it.
            if first_page {
                let msg = encode_server_msg(&WsServerMsg::MissingUpdates {
                    document_id,
                    updates: Vec::new(),
                });
                let _ = outbound_tx.send(msg).await;
            }
            return;
        }
        first_page = false;
        let page_len = updates.len();
        // Rows are ordered by seq; the last one is this page's high-water mark
        // and the cursor for the next page.
        let next_cursor = updates[page_len - 1].seq;

        // Rehydrate each indexed delta into the full sealed Update the client
        // needs to decrypt and verify. The bytes live in S3 once flushed, or in
        // the pending map until then; together they cover every seq. A delta we
        // cannot locate or decode ends the replay at that seq, and the caller is
        // told (`SRV-041`). A page with a hole in it is never sent.
        //
        // The fetches run CONCURRENTLY. A full replay (no snapshot upload yet)
        // pulls every delta since genesis. A sequential `.await` per object
        // turned the open into N serial S3 round-trips.
        let pending = &pending;
        let fetches = updates.into_iter().map(|row| {
            let seq = row.seq as u64;
            async move {
                let wire =
                    match crate::s3::fetch_object(&state.s3, &state.s3_bucket, &row.s3_key).await {
                        Ok(Some(bytes)) => Some(bytes),
                        Ok(None) => pending.get(&seq).cloned(),
                        Err(e) => {
                            tracing::error!(seq, error = %e, "replay: S3 fetch failed");
                            pending.get(&seq).cloned()
                        }
                    };
                let Some(wire) = wire else {
                    tracing::error!(seq, s3_key = %row.s3_key, "replay: delta not found in S3 or pending");
                    return (seq, None);
                };
                match postcard::from_bytes::<ambre_protocol::Update>(&wire) {
                    Ok(update) => (
                        seq,
                        Some(ambre_protocol::SeqUpdate {
                            seq: ambre_protocol::messages::Seq(seq),
                            update,
                        }),
                    ),
                    Err(e) => {
                        tracing::error!(seq, error = %e, "replay: Update decode failed");
                        (seq, None)
                    }
                }
            }
        });

        let fetched = futures_util::future::join_all(fetches).await;
        let (seq_updates, unreadable) = contiguous_prefix(fetched);

        let msg = encode_server_msg(&WsServerMsg::MissingUpdates {
            document_id,
            updates: seq_updates,
        });
        if outbound_tx.send(msg).await.is_err() {
            return;
        }
        if let Some(seq) = unreadable {
            let msg = encode_server_msg(&WsServerMsg::Error {
                code: 503,
                message: format!("replay incomplete: delta {seq} could not be read, retry later"),
            });
            let _ = outbound_tx.send(msg).await;
            return;
        }

        // A short page means the index is exhausted.
        if (page_len as i64) < PAGE_SIZE {
            return;
        }
        cursor = next_cursor;
    }
}

// NOTE (Murmure not implemented): the channel handlers below deliberately skip
// the subscription ACL, signature verification, and clock/seq path that the
// document handlers enforce. Murmure is unstarted and no channel path is wired
// to real data. This is intentional, NOT an oversight: do not re-flag it in
// audits. The MANDATORY hardening before Murmure lands is `SRV-047` and `SRV-048`
// in `ambre-spec-broadcast.md`. A channel is a set of documents, so its frames take
// the document gates: the channel_members ACL, the signature check and the clock/seq
// path. The channel design is in `ambre-spec-murmure-engine.md`.
pub(crate) async fn handle_subscribe_channel(
    _state: &Arc<AppState>,
    conn: &mut WsConnectionState,
    channel_id: ChannelId,
    outbound_tx: &mpsc::Sender<Vec<u8>>,
) {
    // TODO(murmure): enforce the channel_members subscription ACL here, mirroring
    // `handle_subscribe`/`verify_document_access`. `SRV-047`.
    conn.channel_subscriptions.insert(channel_id);
    let ack = encode_server_msg(&WsServerMsg::ChannelSubscribed { channel_id });
    let _ = outbound_tx.send(ack).await;
}

pub(crate) async fn handle_channel_update(
    state: &Arc<AppState>,
    channel_id: ChannelId,
    update: ambre_protocol::Update,
) {
    // TODO(murmure): before Murmure lands, gate this on `verify_update_signature`
    // and the `verify_clock_and_assign_seq` path exactly like `handle_update`, and
    // persist the delta. Rebroadcasting unsigned, unsequenced updates is only safe
    // while no channel is wired to real data. `SRV-048`.
    let broadcast = encode_server_msg(&WsServerMsg::ChannelUpdate {
        channel_id,
        seq: ambre_protocol::messages::Seq(0),
        update,
    });
    let _ = redis_ops::chan_broadcast(&state.redis, &channel_id, &broadcast).await;
}

pub(crate) async fn handle_channel_ephemeral(
    state: &Arc<AppState>,
    channel_id: ChannelId,
    ephemeral: ambre_protocol::Ephemeral,
) {
    // TODO(murmure): signature-check the ephemeral (typing/presence carries the
    // anti-replay counter) once channel ACLs exist. `SRV-048`.
    let broadcast = encode_server_msg(&WsServerMsg::ChannelEphemeral {
        channel_id,
        ephemeral,
    });
    let _ = redis_ops::chan_broadcast(&state.redis, &channel_id, &broadcast).await;
}

// -- ACL ----------------------------------------------------------

/// Re-verifies membership using the short [`MEMBERSHIP_RECHECK_TTL`] cache (H8)
/// and reports whether the caller still has access to `doc_id`.
///
/// Returns `true` immediately while the cached check is fresh. Once it is stale
/// it hits `document_members`. On a revoked membership it drops the subscription,
/// clears the cached timestamp, and unsubscribes the Redis session. Every hot
/// path (update, presence, replay) then stops serving a member revoked
/// mid-session. The caller decides whether to notify the client.
async fn membership_still_valid(
    state: &Arc<AppState>,
    conn: &mut WsConnectionState,
    doc_id: DocumentId,
) -> bool {
    let fresh = conn
        .doc_access_verified_at
        .get(&doc_id)
        .is_some_and(|t| t.elapsed() < MEMBERSHIP_RECHECK_TTL);
    if fresh {
        return true;
    }
    match verify_document_access(&state.db, doc_id, conn.user_id).await {
        Ok(role) => {
            conn.doc_access_verified_at.insert(doc_id, Instant::now());
            // Refresh the cached write capability too: a mid-session role change
            // (for example editor demoted to viewer) takes effect within the TTL.
            conn.doc_write_allowed.insert(doc_id, role_can_write(&role));
            true
        }
        Err(_) => {
            conn.doc_subscriptions.remove(&doc_id);
            conn.doc_access_verified_at.remove(&doc_id);
            conn.doc_write_allowed.remove(&doc_id);
            false
        }
    }
}

/// Whether a document role may author content. Mirrors the `["owner","editor"]`
/// gate every REST write path enforces; a `viewer` (or unknown role) is read-only.
fn role_can_write(role: &str) -> bool {
    matches!(role, "owner" | "editor")
}

/// Verifies the caller has a membership row for the document and returns their
/// role, so callers can gate write operations on it.
pub(crate) async fn verify_document_access(
    pool: &sqlx::PgPool,
    document_id: DocumentId,
    user_id: UserId,
) -> Result<String, AppError> {
    let m = db::get_document_member(pool, &document_id, &user_id)
        .await?
        .ok_or(AppError::Forbidden)?;
    Ok(m.role)
}

/// Appends `len(field) as u32 LE || field` to `msg`.
///
/// MUST match `ambre_crypto::signature::push_framed` byte-for-byte: the client
/// signs with that framing and the server (which does not link `ambre-crypto`)
/// reconstructs it here to verify.
fn push_framed(msg: &mut Vec<u8>, field: &[u8]) {
    let len = u32::try_from(field.len()).expect("signed field exceeds u32::MAX");
    msg.extend_from_slice(&len.to_le_bytes());
    msg.extend_from_slice(field);
}

/// Verifies an Ed25519 signature over the length-prefixed transcript
/// `len(nonce)||nonce || len(ciphertext)||ciphertext ||
/// len(public_data_bytes)||public_data_bytes || len(commitment_tag)||commitment_tag`
/// (every `len` a `u32` LE). The framing MUST match
/// `ambre_crypto::signature::message`; positional (unframed) concatenation would
/// let a distinct `(ciphertext, aad)` split forge the same transcript.
///
/// This is the ONLY cryptographic operation performed by the Blind Relay.
/// The server never decrypts content -- it only checks that the sender
/// owns the private key matching the public key in the message.
pub(crate) fn verify_update_signature(
    pubkey: &ambre_core::PublicKey,
    update: &ambre_protocol::Update,
) -> Result<(), AppError> {
    let public_data_bytes = postcard::to_allocvec(&update.public_data).map_err(AppError::Decode)?;
    verify_sealed_signature(
        pubkey,
        &update.nonce.0,
        &update.ciphertext.0,
        &public_data_bytes,
        &update.commitment_tag.0,
        &update.signature.0,
    )
}

/// Verifies the Ed25519 signature over an [`ambre_protocol::Ephemeral`] envelope.
///
/// Presence/cursor blobs are relayed and written to the Redis presence hash. The
/// durable `Update` path is signature-checked. Without the same check here the
/// relay would broadcast an unauthenticated ciphertext under an author key. Every
/// recipient's client would then reject it on open, a presence-display DoS. The
/// transcript matches `verify_update_signature` byte-for-byte, with the `aad`
/// being the postcard of the EPHEMERAL public data.
pub(crate) fn verify_ephemeral_signature(
    pubkey: &ambre_core::PublicKey,
    ephemeral: &ambre_protocol::Ephemeral,
) -> Result<(), AppError> {
    let public_data_bytes =
        postcard::to_allocvec(&ephemeral.public_data).map_err(AppError::Decode)?;
    verify_sealed_signature(
        pubkey,
        &ephemeral.nonce.0,
        &ephemeral.ciphertext.0,
        &public_data_bytes,
        &ephemeral.commitment_tag.0,
        &ephemeral.signature.0,
    )
}

/// Shared Ed25519 verification over the length-prefixed sealed-envelope transcript
/// `len(nonce)||nonce || len(ciphertext)||ciphertext || len(public_data)||public_data
/// || len(commitment_tag)||commitment_tag` (every `len` a `u32` LE), matching
/// `ambre_crypto::signature::message`. Factored out so the `Update` and
/// `Ephemeral` paths cannot drift apart.
fn verify_sealed_signature(
    pubkey: &ambre_core::PublicKey,
    nonce: &[u8],
    ciphertext: &[u8],
    public_data_bytes: &[u8],
    commitment_tag: &[u8],
    signature: &[u8; 64],
) -> Result<(), AppError> {
    use ed25519_dalek::{Signature as DalekSig, VerifyingKey};

    let mut signed_data = Vec::new();
    push_framed(&mut signed_data, nonce);
    push_framed(&mut signed_data, ciphertext);
    push_framed(&mut signed_data, public_data_bytes);
    push_framed(&mut signed_data, commitment_tag);

    let vk = VerifyingKey::from_bytes(pubkey.as_array()).map_err(|_| AppError::InvalidSignature)?;
    let sig = DalekSig::from_slice(signature).map_err(|_| AppError::InvalidSignature)?;

    // Strict verification (canonical S, no small-order A) so a relayed signature
    // cannot be malleated into a second valid signature over the same envelope.
    vk.verify_strict(&signed_data, &sig)
        .map_err(|_| AppError::InvalidSignature)?;

    Ok(())
}

#[cfg(test)]
mod postcard_tests {
    use ambre_core::{DocumentId, PublicKey, SnapshotId};
    use ambre_protocol::messages::{
        Ciphertext, Clock, CommitmentTag, Counter, Ephemeral, Nonce, Seq, SeqUpdate, Signature,
        Update,
    };
    use ambre_protocol::public_data::{EphemeralPublicData, UpdatePublicData};
    use ambre_protocol::ws::{
        WsClientMsg, WsServerMsg, decode_client_msg, decode_server_msg, encode_client_msg,
        encode_server_msg,
    };

    fn make_update() -> Update {
        Update {
            ciphertext: Ciphertext(vec![1, 2, 3]),
            nonce: Nonce([4u8; 24]),
            public_data: UpdatePublicData {
                document_id: DocumentId::from_bytes([5u8; 16]),
                pub_key: PublicKey::from_array([6u8; 32]),
                snapshot_id: SnapshotId::from_bytes([7u8; 16]),
                clock: Clock(42),
            },
            commitment_tag: CommitmentTag([8u8; 32]),
            signature: Signature([9u8; 64]),
        }
    }

    fn make_ephemeral() -> Ephemeral {
        Ephemeral {
            ciphertext: Ciphertext(vec![10, 11]),
            nonce: Nonce([12u8; 24]),
            public_data: EphemeralPublicData {
                document_id: DocumentId::from_bytes([13u8; 16]),
                pub_key: PublicKey::from_array([14u8; 32]),
                counter: Counter(15),
            },
            commitment_tag: CommitmentTag([16u8; 32]),
            signature: Signature([17u8; 64]),
        }
    }

    #[test]
    fn roundtrip_client_subscribe() {
        let msg = WsClientMsg::Subscribe {
            document_id: DocumentId::from_bytes([0u8; 16]),
        };
        let bytes = encode_client_msg(&msg);
        assert_eq!(bytes[0], 0x01);
        let decoded = decode_client_msg(&bytes).unwrap();
        assert!(matches!(decoded, WsClientMsg::Subscribe { .. }));
    }

    #[test]
    fn roundtrip_client_unsubscribe() {
        let msg = WsClientMsg::Unsubscribe {
            document_id: DocumentId::from_bytes([1u8; 16]),
        };
        let decoded = decode_client_msg(&encode_client_msg(&msg)).unwrap();
        assert!(matches!(decoded, WsClientMsg::Unsubscribe { .. }));
    }

    #[test]
    fn roundtrip_client_update() {
        let update = make_update();
        let msg = WsClientMsg::Update(update);
        let decoded = decode_client_msg(&encode_client_msg(&msg)).unwrap();
        assert!(matches!(decoded, WsClientMsg::Update(_)));
    }

    #[test]
    fn roundtrip_client_ephemeral() {
        let eph = make_ephemeral();
        let msg = WsClientMsg::Ephemeral(eph);
        let decoded = decode_client_msg(&encode_client_msg(&msg)).unwrap();
        assert!(matches!(decoded, WsClientMsg::Ephemeral(_)));
    }

    #[test]
    fn roundtrip_client_request_missing() {
        let msg = WsClientMsg::RequestMissing {
            document_id: DocumentId::from_bytes([1u8; 16]),
            after_seq: Seq(5),
        };
        let decoded = decode_client_msg(&encode_client_msg(&msg)).unwrap();
        assert!(matches!(decoded, WsClientMsg::RequestMissing { .. }));
    }

    #[test]
    fn roundtrip_server_subscribed() {
        let msg = WsServerMsg::Subscribed {
            document_id: DocumentId::from_bytes([2u8; 16]),
        };
        let decoded = decode_server_msg(&encode_server_msg(&msg)).unwrap();
        assert!(matches!(decoded, WsServerMsg::Subscribed { .. }));
    }

    #[test]
    fn roundtrip_server_update() {
        let msg = WsServerMsg::Update {
            seq: Seq(100),
            update: make_update(),
        };
        let decoded = decode_server_msg(&encode_server_msg(&msg)).unwrap();
        assert!(matches!(decoded, WsServerMsg::Update { seq: _, update: _ }));
    }

    #[test]
    fn roundtrip_server_ephemeral() {
        let msg = WsServerMsg::Ephemeral(make_ephemeral());
        let decoded = decode_server_msg(&encode_server_msg(&msg)).unwrap();
        assert!(matches!(decoded, WsServerMsg::Ephemeral(_)));
    }

    #[test]
    fn roundtrip_server_missing_updates() {
        let msg = WsServerMsg::MissingUpdates {
            document_id: DocumentId::from_bytes([3u8; 16]),
            updates: vec![SeqUpdate {
                seq: Seq(1),
                update: make_update(),
            }],
        };
        let decoded = decode_server_msg(&encode_server_msg(&msg)).unwrap();
        assert!(matches!(decoded, WsServerMsg::MissingUpdates { .. }));
    }

    #[test]
    fn roundtrip_server_snapshot_required() {
        let msg = WsServerMsg::SnapshotRequired {
            document_id: DocumentId::from_bytes([4u8; 16]),
        };
        let decoded = decode_server_msg(&encode_server_msg(&msg)).unwrap();
        assert!(matches!(decoded, WsServerMsg::SnapshotRequired { .. }));
    }

    #[test]
    fn roundtrip_server_snapshot_available() {
        let msg = WsServerMsg::SnapshotAvailable {
            document_id: DocumentId::from_bytes([5u8; 16]),
            snapshot_id: SnapshotId::from_bytes([6u8; 16]),
        };
        let decoded = decode_server_msg(&encode_server_msg(&msg)).unwrap();
        assert!(matches!(decoded, WsServerMsg::SnapshotAvailable { .. }));
    }

    #[test]
    fn roundtrip_server_key_rotation_required() {
        let msg = WsServerMsg::KeyRotationRequired {
            document_id: DocumentId::from_bytes([7u8; 16]),
        };
        let decoded = decode_server_msg(&encode_server_msg(&msg)).unwrap();
        assert!(matches!(decoded, WsServerMsg::KeyRotationRequired { .. }));
    }

    #[test]
    fn roundtrip_server_document_revoked() {
        let msg = WsServerMsg::DocumentRevoked {
            document_id: DocumentId::from_bytes([9u8; 16]),
        };
        let decoded = decode_server_msg(&encode_server_msg(&msg)).unwrap();
        assert!(matches!(decoded, WsServerMsg::DocumentRevoked { .. }));
    }

    #[test]
    fn roundtrip_server_error() {
        let msg = WsServerMsg::Error {
            code: 409,
            message: "conflict".into(),
        };
        let decoded = decode_server_msg(&encode_server_msg(&msg)).unwrap();
        match decoded {
            WsServerMsg::Error { code, message } => {
                assert_eq!(code, 409);
                assert_eq!(message, "conflict");
            }
            _ => panic!("expected Error variant"),
        }
    }

    #[test]
    fn version_byte_is_01() {
        let msg = WsClientMsg::Subscribe {
            document_id: DocumentId::from_bytes([0u8; 16]),
        };
        let bytes = encode_client_msg(&msg);
        assert_eq!(bytes[0], 0x01);
    }

    #[test]
    fn empty_frame_is_error() {
        assert!(decode_client_msg(&[]).is_err());
    }

    #[test]
    fn wrong_version_byte_is_error() {
        let msg = vec![0x02, 0x00];
        assert!(decode_client_msg(&msg).is_err());
    }

    #[test]
    fn garbage_data_returns_error_not_panics() {
        let result = decode_client_msg(&[0x01, 0xFF, 0xFE, 0xFD]);
        assert!(result.is_err());
    }
}

// Test for the ephemeral author-key check (same idea as the `handle_update`
// check). It needs a live Redis to observe the presence-hash side effect, so it
// is gated by the `db` feature like the other integration tests.
#[cfg(test)]
mod ephemeral_binding_tests {
    use super::*;
    use ambre_core::{DocumentId, PublicKey};
    use ambre_protocol::messages::{
        Ciphertext, CommitmentTag, Counter, Ephemeral, Nonce, Signature,
    };
    use ambre_protocol::public_data::EphemeralPublicData;
    use rustis::commands::{GenericCommands, HashCommands};

    async fn test_state() -> Option<Arc<AppState>> {
        let redis = rustis::client::Client::connect("redis://localhost:6379")
            .await
            .ok()?;
        let s3 = aws_sdk_s3::Client::from_conf(aws_sdk_s3::Config::builder().build());
        let pubsub = crate::ws::PubSubHub::spawn(redis.clone());
        // The ephemeral path never touches Postgres, so a lazy pool (never
        // connected) is enough to satisfy the AppState shape.
        let db = sqlx::postgres::PgPoolOptions::new()
            .connect_lazy("postgres://localhost/ambre")
            .ok()?;
        Some(Arc::new(AppState {
            db,
            redis,
            pubsub,
            s3,
            s3_bucket: "test-bucket".into(),
            config: Arc::new(crate::state::Config::default()),
            jwt: Arc::new(crate::jwt::JwtKeys::from_seed(&[7u8; 32])),
            oidc_clients: Arc::new(std::collections::HashMap::new()),
        }))
    }

    fn conn_with(session_key: PublicKey, doc_id: DocumentId) -> WsConnectionState {
        let mut doc_subscriptions = HashSet::new();
        doc_subscriptions.insert(doc_id);
        WsConnectionState {
            user_id: UserId::new(),
            signing_pubkey: session_key,
            doc_subscriptions,
            // Seed the membership cache as freshly verified. These unit tests then
            // exercise their own concern (author-key binding / size cap) without
            // the `membership_still_valid` recheck reaching for a real Postgres.
            doc_access_verified_at: HashMap::from([(doc_id, Instant::now())]),
            // A writer role, so the size-cap / author-key tests exercise their own
            // concern rather than tripping the read-only gate.
            doc_write_allowed: HashMap::from([(doc_id, true)]),
            last_replay_at: HashMap::new(),
            channel_subscriptions: HashSet::new(),
            // A wide bucket so these tests exercise their own concern, not the
            // rate limit (which the loop enforces, covered by its own test).
            msg_bucket: TokenBucket::new(1000, 1000),
        }
    }

    /// An unsigned ephemeral (all-zero signature) under `pub_key`. Used only for
    /// the forged-author-key path, which is dropped by the identity check before
    /// the signature is ever examined.
    fn unsigned_ephemeral(pub_key: PublicKey, doc_id: DocumentId) -> Ephemeral {
        Ephemeral {
            ciphertext: Ciphertext(vec![1, 2, 3]),
            nonce: Nonce([0u8; 24]),
            public_data: EphemeralPublicData {
                document_id: doc_id,
                pub_key,
                counter: Counter(1),
            },
            commitment_tag: CommitmentTag([0u8; 32]),
            signature: Signature([0u8; 64]),
        }
    }

    /// A properly signed ephemeral: the transcript matches
    /// `verify_ephemeral_signature`, so the relay accepts and records it.
    fn signed_ephemeral(
        signing_key: &ed25519_dalek::SigningKey,
        pub_key: &PublicKey,
        doc_id: DocumentId,
        ciphertext: Vec<u8>,
    ) -> Ephemeral {
        use ed25519_dalek::Signer;
        let public_data = EphemeralPublicData {
            document_id: doc_id,
            pub_key: pub_key.clone(),
            counter: Counter(1),
        };
        let nonce = Nonce([5u8; 24]);
        let commitment_tag = CommitmentTag([6u8; 32]);
        let public_data_bytes = postcard::to_allocvec(&public_data).unwrap();
        let mut signed_data = Vec::new();
        push_framed(&mut signed_data, &nonce.0);
        push_framed(&mut signed_data, &ciphertext);
        push_framed(&mut signed_data, &public_data_bytes);
        push_framed(&mut signed_data, &commitment_tag.0);
        let sig = signing_key.sign(&signed_data);
        Ephemeral {
            ciphertext: Ciphertext(ciphertext),
            nonce,
            public_data,
            commitment_tag,
            signature: Signature(sig.to_bytes()),
        }
    }

    #[tokio::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs redis; run with --features db")]
    async fn forged_author_key_writes_no_presence() {
        let Some(state) = test_state().await else {
            return;
        };
        let doc_id = DocumentId::new();
        let signing_key = ed25519_dalek::SigningKey::from_bytes(&[21u8; 32]);
        let session_key = PublicKey::from_array(signing_key.verifying_key().to_bytes());
        let forged_key = PublicKey::from_array([2u8; 32]);
        let presence_key = format!("doc:{doc_id}:presence");
        let _ = state.redis.del([&presence_key]).await;

        let mut conn = conn_with(session_key.clone(), doc_id);

        // A forged author key must be dropped before any Redis write.
        handle_ephemeral(&state, &mut conn, unsigned_ephemeral(forged_key, doc_id)).await;
        let after_forged: std::collections::HashMap<String, String> =
            state.redis.hgetall(&presence_key).await.unwrap();
        assert!(
            after_forged.is_empty(),
            "forged author key must not write a presence entry"
        );

        // The session's own key with a valid signature is accepted and recorded.
        handle_ephemeral(
            &state,
            &mut conn,
            signed_ephemeral(&signing_key, &session_key, doc_id, vec![1, 2, 3]),
        )
        .await;
        let after_valid: std::collections::HashMap<String, String> =
            state.redis.hgetall(&presence_key).await.unwrap();
        assert_eq!(
            after_valid.len(),
            1,
            "the session key must record exactly one presence entry"
        );
        assert!(after_valid.contains_key(&session_key.to_hex()));

        let _ = state.redis.del([&presence_key]).await;
    }

    /// An ephemeral carrying the correct author key but an invalid signature must
    /// be dropped before any presence write. The relay must not broadcast an
    /// unauthenticated blob.
    #[tokio::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs redis; run with --features db")]
    async fn bad_signature_writes_no_presence() {
        let Some(state) = test_state().await else {
            return;
        };
        let doc_id = DocumentId::new();
        let signing_key = ed25519_dalek::SigningKey::from_bytes(&[22u8; 32]);
        let session_key = PublicKey::from_array(signing_key.verifying_key().to_bytes());
        let presence_key = format!("doc:{doc_id}:presence");
        let _ = state.redis.del([&presence_key]).await;

        let mut conn = conn_with(session_key.clone(), doc_id);

        // Correct author key, but tamper the ciphertext after signing.
        let mut eph = signed_ephemeral(&signing_key, &session_key, doc_id, vec![1, 2, 3]);
        eph.ciphertext = Ciphertext(vec![9, 9, 9]);
        handle_ephemeral(&state, &mut conn, eph).await;

        let after: std::collections::HashMap<String, String> =
            state.redis.hgetall(&presence_key).await.unwrap();
        assert!(
            after.is_empty(),
            "an invalid signature must not write a presence entry"
        );

        let _ = state.redis.del([&presence_key]).await;
    }

    /// An oversized ephemeral ciphertext must be dropped before any presence
    /// write, mirroring the delta size cap on the durable path.
    #[tokio::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs redis; run with --features db")]
    async fn oversized_ephemeral_writes_no_presence() {
        let Some(mut state) = test_state().await else {
            return;
        };
        // Tighten the cap so the test payload is unambiguously oversized.
        Arc::get_mut(&mut state).unwrap().config = Arc::new(crate::state::Config {
            ephemeral_max_bytes: 8,
            ..Default::default()
        });
        let doc_id = DocumentId::new();
        let signing_key = ed25519_dalek::SigningKey::from_bytes(&[23u8; 32]);
        let session_key = PublicKey::from_array(signing_key.verifying_key().to_bytes());
        let presence_key = format!("doc:{doc_id}:presence");
        let _ = state.redis.del([&presence_key]).await;

        let mut conn = conn_with(session_key.clone(), doc_id);
        // 9 bytes > the 8-byte cap: dropped before the signature check even runs.
        let eph = signed_ephemeral(&signing_key, &session_key, doc_id, vec![0u8; 9]);
        handle_ephemeral(&state, &mut conn, eph).await;

        let after: std::collections::HashMap<String, String> =
            state.redis.hgetall(&presence_key).await.unwrap();
        assert!(
            after.is_empty(),
            "an oversized ephemeral must not write a presence entry"
        );

        let _ = state.redis.del([&presence_key]).await;
    }
}

// Test for the delta size cap in `handle_update`. The cap is checked before any
// durable write, so an oversized ciphertext must leave the Redis pending list
// empty. Needs a live Redis to observe the (non-)write, so it is gated by `db`.
#[cfg(test)]
mod delta_size_cap_tests {
    use super::*;
    use ambre_core::{DocumentId, PublicKey, SnapshotId};
    use ambre_protocol::messages::{Ciphertext, Clock, CommitmentTag, Nonce, Signature, Update};
    use ambre_protocol::public_data::UpdatePublicData;
    use rustis::commands::{GenericCommands, ListCommands};

    async fn test_state(delta_max_bytes: usize) -> Option<Arc<AppState>> {
        let redis = rustis::client::Client::connect("redis://localhost:6379")
            .await
            .ok()?;
        let s3 = aws_sdk_s3::Client::from_conf(aws_sdk_s3::Config::builder().build());
        let pubsub = crate::ws::PubSubHub::spawn(redis.clone());
        let db = sqlx::postgres::PgPoolOptions::new()
            .connect_lazy("postgres://localhost/ambre")
            .ok()?;
        let config = crate::state::Config {
            delta_max_bytes,
            ..Default::default()
        };
        Some(Arc::new(AppState {
            db,
            redis,
            pubsub,
            s3,
            s3_bucket: "test-bucket".into(),
            config: Arc::new(config),
            jwt: Arc::new(crate::jwt::JwtKeys::from_seed(&[7u8; 32])),
            oidc_clients: Arc::new(std::collections::HashMap::new()),
        }))
    }

    fn conn_with(session_key: PublicKey, doc_id: DocumentId) -> WsConnectionState {
        let mut doc_subscriptions = HashSet::new();
        doc_subscriptions.insert(doc_id);
        WsConnectionState {
            user_id: UserId::new(),
            signing_pubkey: session_key,
            doc_subscriptions,
            // Seed the membership cache as freshly verified. These unit tests then
            // exercise their own concern (author-key binding / size cap) without
            // the `membership_still_valid` recheck reaching for a real Postgres.
            doc_access_verified_at: HashMap::from([(doc_id, Instant::now())]),
            // A writer role, so the size-cap / author-key tests exercise their own
            // concern rather than tripping the read-only gate.
            doc_write_allowed: HashMap::from([(doc_id, true)]),
            last_replay_at: HashMap::new(),
            channel_subscriptions: HashSet::new(),
            // A wide bucket so these tests exercise their own concern, not the
            // rate limit (which the loop enforces, covered by its own test).
            msg_bucket: TokenBucket::new(1000, 1000),
        }
    }

    fn update_with(pub_key: PublicKey, doc_id: DocumentId, cipher_len: usize) -> Update {
        Update {
            ciphertext: Ciphertext(vec![0u8; cipher_len]),
            nonce: Nonce([0u8; 24]),
            public_data: UpdatePublicData {
                document_id: doc_id,
                pub_key,
                snapshot_id: SnapshotId::from_bytes([0u8; 16]),
                clock: Clock(1),
            },
            commitment_tag: CommitmentTag([0u8; 32]),
            signature: Signature([0u8; 64]),
        }
    }

    #[tokio::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs redis; run with --features db")]
    async fn oversized_delta_writes_no_pending() {
        let Some(state) = test_state(8).await else {
            return;
        };
        let doc_id = DocumentId::new();
        let session_key = PublicKey::from_array([1u8; 32]);
        let pending_key = format!("doc:{doc_id}:pending");
        let _ = state.redis.del([&pending_key]).await;

        let mut conn = conn_with(session_key.clone(), doc_id);
        let (tx, mut rx) = mpsc::channel::<Vec<u8>>(8);

        // 9 bytes > the 8-byte cap: rejected before any durable write.
        handle_update(&state, &mut conn, update_with(session_key, doc_id, 9), &tx).await;

        let pending_len: usize = state.redis.llen(&pending_key).await.unwrap();
        assert_eq!(
            pending_len, 0,
            "an oversized delta must not be pushed to the pending list"
        );
        // The client is told why (413), so it can move the blob to the media path.
        let reply = rx.try_recv().expect("a 413 error reply must be sent");
        let decoded = ambre_protocol::ws::decode_server_msg(&reply).unwrap();
        assert!(matches!(
            decoded,
            ambre_protocol::ws::WsServerMsg::Error { code: 413, .. }
        ));

        let _ = state.redis.del([&pending_key]).await;
    }

    /// A read-only (`viewer`) member must not be able to author a delta over the
    /// WS write path. It is rejected with 403 before any durable write, so the
    /// pending list stays empty. Without the role gate a viewer could push
    /// sequenced, persisted, broadcast updates, despite REST write paths refusing
    /// them. A viewer is a legitimately invited member holding the doc key and
    /// its own signing key.
    #[tokio::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs redis; run with --features db")]
    async fn viewer_role_delta_writes_no_pending() {
        let Some(state) = test_state(1024).await else {
            return;
        };
        let doc_id = DocumentId::new();
        let session_key = PublicKey::from_array([1u8; 32]);
        let pending_key = format!("doc:{doc_id}:pending");
        let _ = state.redis.del([&pending_key]).await;

        // Fresh membership, but a read-only role: the write capability is cached
        // false, exactly as `handle_subscribe` would set it for a `viewer`.
        let mut conn = conn_with(session_key.clone(), doc_id);
        conn.doc_write_allowed.insert(doc_id, false);
        let (tx, mut rx) = mpsc::channel::<Vec<u8>>(8);

        handle_update(&state, &mut conn, update_with(session_key, doc_id, 16), &tx).await;

        let pending_len: usize = state.redis.llen(&pending_key).await.unwrap();
        assert_eq!(
            pending_len, 0,
            "a viewer's delta must not be pushed to the pending list"
        );
        let reply = rx.try_recv().expect("a 403 error reply must be sent");
        let decoded = ambre_protocol::ws::decode_server_msg(&reply).unwrap();
        assert!(matches!(
            decoded,
            ambre_protocol::ws::WsServerMsg::Error { code: 403, .. }
        ));

        let _ = state.redis.del([&pending_key]).await;
    }

    /// F5 on an existing document: the client's downloaded snapshot already
    /// covers every delta, so its post-open `RequestMissing{after_seq: covered}`
    /// finds no newer rows in the durable index. The server must STILL answer
    /// with an (empty) `MissingUpdates` -- the client gates its
    /// hydration-complete signal (dismissing the loading overlay) on receiving
    /// one. A silent return left the overlay spinning forever.
    #[sqlx::test]
    #[cfg_attr(
        not(feature = "db"),
        ignore = "needs postgres+redis; run with --features db"
    )]
    async fn empty_replay_still_sends_missing_updates(pool: sqlx::PgPool) {
        let Ok(redis) = rustis::client::Client::connect("redis://localhost:6379").await else {
            return;
        };
        let pubsub = crate::ws::PubSubHub::spawn(redis.clone());
        let s3 = aws_sdk_s3::Client::from_conf(aws_sdk_s3::Config::builder().build());
        let state = Arc::new(AppState {
            db: pool,
            redis,
            pubsub,
            s3,
            s3_bucket: "test-bucket".into(),
            config: Arc::new(crate::state::Config::default()),
            jwt: Arc::new(crate::jwt::JwtKeys::from_seed(&[7u8; 32])),
            oidc_clients: Arc::new(std::collections::HashMap::new()),
        });
        let doc_id = DocumentId::new();
        let session_key = PublicKey::from_array([1u8; 32]);

        // A fresh doc_id with no rows in the durable index: the very first page
        // of the replay is empty.
        let mut conn = conn_with(session_key, doc_id);
        let (tx, mut rx) = mpsc::channel::<Vec<u8>>(8);

        handle_request_missing(
            &state,
            &mut conn,
            doc_id,
            ambre_protocol::messages::Seq(0),
            &tx,
        )
        .await;

        let reply = rx
            .try_recv()
            .expect("an empty replay must still send a MissingUpdates to release the client");
        let decoded = ambre_protocol::ws::decode_server_msg(&reply).unwrap();
        match decoded {
            ambre_protocol::ws::WsServerMsg::MissingUpdates { updates, .. } => {
                assert!(updates.is_empty(), "no deltas to replay for a fresh doc");
            }
            other => panic!("expected an empty MissingUpdates, got {other:?}"),
        }
    }
}

#[cfg(test)]
mod ed25519_tests {
    use super::*;
    use ambre_core::{DocumentId, PublicKey, SnapshotId};
    use ambre_protocol::messages::{Ciphertext, Clock, CommitmentTag, Nonce, Signature, Update};
    use ambre_protocol::public_data::UpdatePublicData;
    use ed25519_dalek::{Signer, SigningKey};

    #[test]
    fn valid_signature_is_accepted() {
        let secret_bytes: [u8; 32] = [
            0xcd, 0x06, 0x85, 0x43, 0x6b, 0xa1, 0xf7, 0x1e, 0x32, 0xce, 0x9a, 0x84, 0xbd, 0xc6,
            0xfe, 0xa0, 0xb6, 0x49, 0x12, 0x1d, 0x8b, 0xf8, 0x43, 0xa2, 0xdd, 0xc6, 0x9f, 0x14,
            0xf3, 0x5a, 0x3c, 0xb9,
        ];
        let signing_key = SigningKey::from_bytes(&secret_bytes);
        let pubkey_bytes: [u8; 32] = signing_key.verifying_key().to_bytes();
        let pubkey = PublicKey::from_array(pubkey_bytes);

        let public_data = UpdatePublicData {
            document_id: DocumentId::from_bytes([0u8; 16]),
            pub_key: pubkey.clone(),
            snapshot_id: SnapshotId::from_bytes([1u8; 16]),
            clock: Clock(1),
        };
        let nonce = Nonce([3u8; 24]);
        let ciphertext = Ciphertext(vec![4, 5, 6]);
        let commitment_tag = CommitmentTag([7u8; 32]);

        let public_data_bytes = postcard::to_allocvec(&public_data).unwrap();
        let mut signed_data = Vec::new();
        push_framed(&mut signed_data, &nonce.0);
        push_framed(&mut signed_data, &ciphertext.0);
        push_framed(&mut signed_data, &public_data_bytes);
        push_framed(&mut signed_data, &commitment_tag.0);

        let sig = signing_key.sign(&signed_data);
        let signature = Signature(sig.to_bytes());

        let update = Update {
            ciphertext,
            nonce,
            public_data,
            commitment_tag,
            signature,
        };

        verify_update_signature(&pubkey, &update).expect("valid signature must be accepted");
    }

    #[test]
    fn invalid_signature_is_rejected() {
        let pubkey = PublicKey::from_array([42u8; 32]);

        let update = Update {
            ciphertext: Ciphertext(vec![1u8]),
            nonce: Nonce([0u8; 24]),
            public_data: UpdatePublicData {
                document_id: DocumentId::from_bytes([0u8; 16]),
                pub_key: pubkey,
                snapshot_id: SnapshotId::from_bytes([0u8; 16]),
                clock: Clock(0),
            },
            commitment_tag: CommitmentTag([0u8; 32]),
            signature: Signature([0u8; 64]),
        };

        assert!(verify_update_signature(&update.public_data.pub_key, &update).is_err());
    }

    #[test]
    fn tampered_ciphertext_rejects_signature() {
        let secret_bytes: [u8; 32] = [
            0xab, 0x12, 0x34, 0x56, 0x78, 0x9a, 0xbc, 0xde, 0xf0, 0x11, 0x22, 0x33, 0x44, 0x55,
            0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff, 0x00, 0x01, 0x02, 0x03,
            0x04, 0x05, 0x06, 0x07,
        ];
        let signing_key = SigningKey::from_bytes(&secret_bytes);
        let pubkey = PublicKey::from_array(signing_key.verifying_key().to_bytes());

        let public_data = UpdatePublicData {
            document_id: DocumentId::from_bytes([0u8; 16]),
            pub_key: pubkey.clone(),
            snapshot_id: SnapshotId::from_bytes([0u8; 16]),
            clock: Clock(0),
        };
        let nonce = Nonce([0u8; 24]);
        let ciphertext = Ciphertext(vec![1, 2, 3]);
        let commitment_tag = CommitmentTag([0u8; 32]);

        let public_data_bytes = postcard::to_allocvec(&public_data).unwrap();
        let mut signed_data = vec![];
        push_framed(&mut signed_data, &nonce.0);
        push_framed(&mut signed_data, &ciphertext.0);
        push_framed(&mut signed_data, &public_data_bytes);
        push_framed(&mut signed_data, &commitment_tag.0);

        let sig = signing_key.sign(&signed_data);

        let tampered = Update {
            ciphertext: Ciphertext(vec![9, 9, 9]),
            nonce,
            public_data,
            commitment_tag,
            signature: Signature(sig.to_bytes()),
        };

        assert!(verify_update_signature(&pubkey, &tampered).is_err());
    }

    fn sign_ephemeral(
        signing_key: &SigningKey,
        pubkey: &PublicKey,
        doc_id: DocumentId,
        ciphertext: Vec<u8>,
    ) -> ambre_protocol::Ephemeral {
        use ambre_protocol::messages::Counter;
        use ambre_protocol::public_data::EphemeralPublicData;
        let public_data = EphemeralPublicData {
            document_id: doc_id,
            pub_key: pubkey.clone(),
            counter: Counter(1),
        };
        let nonce = Nonce([2u8; 24]);
        let commitment_tag = CommitmentTag([3u8; 32]);
        let public_data_bytes = postcard::to_allocvec(&public_data).unwrap();
        let mut signed_data = Vec::new();
        push_framed(&mut signed_data, &nonce.0);
        push_framed(&mut signed_data, &ciphertext);
        push_framed(&mut signed_data, &public_data_bytes);
        push_framed(&mut signed_data, &commitment_tag.0);
        let sig = signing_key.sign(&signed_data);
        ambre_protocol::Ephemeral {
            ciphertext: Ciphertext(ciphertext),
            nonce,
            public_data,
            commitment_tag,
            signature: Signature(sig.to_bytes()),
        }
    }

    #[test]
    fn valid_ephemeral_signature_is_accepted() {
        let signing_key = SigningKey::from_bytes(&[11u8; 32]);
        let pubkey = PublicKey::from_array(signing_key.verifying_key().to_bytes());
        let doc_id = DocumentId::from_bytes([9u8; 16]);
        let eph = sign_ephemeral(&signing_key, &pubkey, doc_id, vec![1, 2, 3, 4]);
        verify_ephemeral_signature(&pubkey, &eph).expect("valid ephemeral signature accepted");
    }

    #[test]
    fn tampered_ephemeral_ciphertext_rejects_signature() {
        let signing_key = SigningKey::from_bytes(&[12u8; 32]);
        let pubkey = PublicKey::from_array(signing_key.verifying_key().to_bytes());
        let doc_id = DocumentId::from_bytes([9u8; 16]);
        let mut eph = sign_ephemeral(&signing_key, &pubkey, doc_id, vec![1, 2, 3, 4]);
        // Flip the ciphertext after signing: the signature must no longer verify.
        eph.ciphertext = Ciphertext(vec![9, 9, 9, 9]);
        assert!(verify_ephemeral_signature(&pubkey, &eph).is_err());
    }

    #[test]
    fn unsigned_ephemeral_is_rejected() {
        // The old handler broadcast this (all-zero signature) unverified.
        let pubkey = PublicKey::from_array([7u8; 32]);
        let eph = ambre_protocol::Ephemeral {
            ciphertext: Ciphertext(vec![1, 2, 3]),
            nonce: Nonce([0u8; 24]),
            public_data: ambre_protocol::public_data::EphemeralPublicData {
                document_id: DocumentId::from_bytes([0u8; 16]),
                pub_key: pubkey.clone(),
                counter: ambre_protocol::messages::Counter(1),
            },
            commitment_tag: CommitmentTag([0u8; 32]),
            signature: Signature([0u8; 64]),
        };
        assert!(verify_ephemeral_signature(&pubkey, &eph).is_err());
    }
}

#[cfg(test)]
mod refusal_tests {
    use super::*;

    fn seq_update(seq: u64) -> ambre_protocol::SeqUpdate {
        use ambre_protocol::messages::{Ciphertext, Clock, CommitmentTag, Nonce, Seq, Signature};
        ambre_protocol::SeqUpdate {
            seq: Seq(seq),
            update: ambre_protocol::Update {
                ciphertext: Ciphertext(vec![seq as u8]),
                nonce: Nonce([0u8; 24]),
                public_data: ambre_protocol::public_data::UpdatePublicData {
                    document_id: DocumentId::new(),
                    pub_key: ambre_core::PublicKey::from_array([1u8; 32]),
                    snapshot_id: ambre_core::SnapshotId::from_bytes([0u8; 16]),
                    clock: Clock(seq),
                },
                commitment_tag: CommitmentTag([0u8; 32]),
                signature: Signature([0u8; 64]),
            },
        }
    }

    #[test]
    fn a_replay_page_stops_at_the_first_unreadable_delta() {
        let fetched = vec![
            (4, Some(seq_update(4))),
            (2, Some(seq_update(2))),
            (3, None),
            (1, Some(seq_update(1))),
        ];
        let (page, unreadable) = contiguous_prefix(fetched);
        let seqs: Vec<u64> = page.iter().map(|u| u.seq.0).collect();
        assert_eq!(seqs, [1, 2], "seq 4 is held back behind the hole at 3");
        assert_eq!(unreadable, Some(3));

        let (page, unreadable) =
            contiguous_prefix(vec![(2, Some(seq_update(2))), (1, Some(seq_update(1)))]);
        assert_eq!(page.len(), 2);
        assert_eq!(unreadable, None);
    }

    #[test]
    fn a_refused_clock_carries_the_expected_and_the_received_value() {
        let doc_id = DocumentId::new();
        let refusal = update_refusal(
            &doc_id,
            &AppError::ClockMismatch {
                expected: 5,
                got: 9,
            },
        );
        let WsServerMsg::Error { code, message } = refusal else {
            panic!("expected an error frame");
        };
        assert_eq!(code, 409);
        assert!(
            message.contains("expected 5") && message.contains("got 9"),
            "{message}"
        );
    }

    #[test]
    fn a_store_failure_reaches_the_client_without_its_name_or_text() {
        let doc_id = DocumentId::new();
        let cause = AppError::Redis("clock validation failed: ERR connection refused".into());
        let WsServerMsg::Error { code, message } = update_refusal(&doc_id, &cause) else {
            panic!("expected an error frame");
        };
        assert_eq!(code, 500);
        assert_eq!(message, "Internal server error");
        for leaked in ["Redis", "redis", "refused", "clock validation"] {
            assert!(!message.contains(leaked), "{message}");
        }
    }
}
