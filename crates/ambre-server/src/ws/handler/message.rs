// ambre-server/src/ws/handler/message.rs
// Per-message WebSocket handlers (subscribe/update/ephemeral/request-missing),
// access/signature verification, and their unit tests.

use super::*;

/// The answer to a refused wire message: its code and its document alone
/// (`SRV-183`). The detail reaches the log at debug, never the wire.
pub(crate) fn refusal(document_id: Option<DocumentId>, code: RefusalCode) -> WsServerMsg {
    WsServerMsg::Error { document_id, code }
}

/// Sends [`refusal`] and logs `detail`, which holds no content (`SRV-170`).
pub(crate) async fn refuse(
    outbound_tx: &mpsc::Sender<Payload>,
    document_id: Option<DocumentId>,
    code: RefusalCode,
    detail: &str,
) {
    tracing::debug!(?document_id, %code, detail, "wire message refused");
    let _ = outbound_tx
        .send(Payload::server_msg(&refusal(document_id, code)))
        .await;
}

/// The answer to a delta refused before it took a sequence number: its
/// document, its signature and its code (`SRV-185`). The author learns which
/// edit the relay lacks.
pub(crate) fn delta_refusal(update: &ambre_protocol::Update, code: RefusalCode) -> WsServerMsg {
    WsServerMsg::DeltaRefused {
        document_id: update.public_data.document_id,
        signature: update.signature,
        code,
    }
}

/// Sends [`delta_refusal`] and logs `detail`, which holds no content
/// (`SRV-170`).
async fn refuse_delta(
    outbound_tx: &mpsc::Sender<Payload>,
    update: &ambre_protocol::Update,
    code: RefusalCode,
    detail: &str,
) {
    let document_id = update.public_data.document_id;
    tracing::debug!(%document_id, %code, detail, "delta refused");
    let _ = outbound_tx
        .send(Payload::server_msg(&delta_refusal(update, code)))
        .await;
}

/// Orders a replay page and keeps the run of deltas before the first one that
/// could not be read. Returns that run and the unreadable seq, if any, so the
/// page never carries a hole the caller cannot see (`SRV-041`).
pub(crate) fn contiguous_prefix(
    mut fetched: Vec<(ambre_core::Seq, Option<ambre_protocol::SeqUpdate>)>,
) -> (Vec<ambre_protocol::SeqUpdate>, Option<ambre_core::Seq>) {
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

/// The documents whose later deltas this socket drops after a refused clock,
/// each with the signature of the delta refused (`SRV-169`).
///
/// Two sessions of one user share one clock. When one session's delta is
/// refused, the deltas it sent behind it may fit the clock the other session
/// left. Accepted, they would land ahead of the refused delta they build on.
/// So the socket drops them until the author answers the refusal.
#[derive(Debug, Default)]
pub(crate) struct ClockFences(HashMap<DocumentId, ambre_protocol::messages::Signature>);

impl ClockFences {
    /// The delta signed `signature` was refused for its clock.
    pub(crate) fn raise(
        &mut self,
        document_id: DocumentId,
        signature: ambre_protocol::messages::Signature,
    ) {
        self.0.insert(document_id, signature);
    }

    /// Raises the fence when `answer` names a delta that took no clock: one
    /// refused for its clock, or one the relay failed to sequence. The deltas
    /// behind it would land ahead of it once sent again (`SRV-169`, `SRV-186`).
    pub(crate) fn raise_for(&mut self, answer: &WsServerMsg) {
        match answer {
            WsServerMsg::ClockRefused {
                document_id,
                signature,
                ..
            }
            | WsServerMsg::DeltaRefused {
                document_id,
                signature,
                ..
            } => self.raise(*document_id, *signature),
            _ => {}
        }
    }

    /// Whether the socket drops this document's deltas.
    pub(crate) fn holds(&self, document_id: &DocumentId) -> bool {
        self.0.contains_key(document_id)
    }

    /// The author answered the refusal of the delta signed `signature`. Only
    /// the answer to the delta that raised the fence lifts it, so a late
    /// answer to an earlier refusal never opens a later fence.
    pub(crate) fn answer(
        &mut self,
        document_id: DocumentId,
        signature: &ambre_protocol::messages::Signature,
    ) {
        if self.0.get(&document_id) == Some(signature) {
            self.0.remove(&document_id);
        }
    }

    /// The socket left the document: nothing it sends there is read.
    pub(crate) fn forget(&mut self, document_id: &DocumentId) {
        self.0.remove(document_id);
    }
}

/// The frame an update refused at clock validation sends back.
///
/// The client acts on a refused clock. So the refusal names the delta by its
/// signature, and carries the expected and the received clock (`SRV-037`).
/// A store failure names the delta with a bare internal code, so the author
/// sends it again (`SRV-186`). The store's name and text stay in the log
/// (`SRV-081`, `SRV-020`).
///
/// A document that owes a rotation refuses the frame for its key. The refusal
/// names the key and the clock, so the author seals that frame again under the
/// next key (`SRV-123`, `ambre-spec-sync.md`, `SYNC-129`).
pub(crate) fn update_refusal(update: &ambre_protocol::Update, e: &AppError) -> WsServerMsg {
    let doc_id = &update.public_data.document_id;
    match e {
        AppError::RotationOwed => WsServerMsg::KeyRefused {
            document_id: *doc_id,
            key_id: update.public_data.key_id,
            clock: update.public_data.clock,
        },
        AppError::ClockMismatch { expected, got } => WsServerMsg::ClockRefused {
            document_id: *doc_id,
            clock: *got,
            expected: *expected,
            signature: update.signature,
        },
        // The document is being deleted (`SRV-017`).
        AppError::NotFound { .. } => refusal(Some(*doc_id), RefusalCode::NotFound),
        // A finalize lease locked the segment: the refusal names the lock
        // and the delta, so the author sends it elsewhere (`SRV-115`).
        AppError::SegmentLocked => delta_refusal(update, RefusalCode::SegmentLocked),
        _ => {
            tracing::error!("clock validation failed doc={doc_id}: {e}");
            delta_refusal(update, RefusalCode::InternalError)
        }
    }
}

/// The frame a refused subscription answers. A store failure reaches the
/// client as a bare internal error, its cause kept to the log (`SRV-096`).
pub(crate) fn subscribe_refusal(document_id: DocumentId, e: &AppError) -> WsServerMsg {
    if e.is_infrastructure() {
        tracing::error!("subscribe access check failed doc={document_id}: {e}");
        return refusal(Some(document_id), RefusalCode::InternalError);
    }
    // One answer whatever the cause, so a refusal never tells a stranger the
    // document exists (`SRV-005`).
    tracing::debug!("subscribe refused doc={document_id}: {e}");
    refusal(Some(document_id), RefusalCode::Forbidden)
}

pub(crate) async fn handle_subscribe(
    state: &Arc<AppState>,
    conn: &mut WsConnectionState,
    document_id: DocumentId,
    outbound_tx: &mpsc::Sender<Payload>,
) {
    let role = match verify_document_access(&state.db, document_id, conn.user_id).await {
        Ok(role) => role,
        Err(e) => {
            let err = Payload::server_msg(&subscribe_refusal(document_id, &e));
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
                let author_clocks: Vec<(
                    ambre_core::SigningKeyHex,
                    ambre_protocol::messages::Clock,
                )> = clock_state
                    .author_clocks
                    .iter()
                    .map(|(pk, clock)| (ambre_core::SigningKeyHex::encode(pk), *clock))
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
    // Rewrite the mirror of an owed rotation from the register, before this
    // client can send. A reset of the ephemeral store then loses the debt only
    // until the next subscribe (`SRV-123`).
    let mirrored = match db::rotation_owed(&state.db, &document_id).await {
        Ok(owed) => {
            let debt = redis_ops::RotationDebt::owed_if(owed);
            redis_ops::mirror_rotation_owed(&state.redis, &document_id, debt).await
        }
        Err(e) => Err(e),
    };
    if let Err(e) = mirrored {
        tracing::error!(error = %e, "subscribe: rotation debt mirror failed");
    }
    // The same for the lock of a segment: a reset of the ephemeral store
    // lets deltas in only until the next subscribe (`SRV-114`). A lock holds
    // for good, so the mirror is only ever set.
    let locked = match db::segment_locked(&state.db, &document_id).await {
        Ok(true) => redis_ops::mirror_segment_lock(&state.redis, &document_id).await,
        Ok(false) => Ok(()),
        Err(e) => Err(e),
    };
    if let Err(e) = locked {
        tracing::error!(error = %e, "subscribe: segment lock mirror failed");
    }

    // Count this session once, however often it re-subscribes: its departure
    // is announced only when no other session of the same key still watches
    // (`PRESENCE-015`). A failed count is logged; the departure then fails open.
    if conn.doc_subscriptions.insert(document_id) {
        let pub_key_hex = ambre_core::SigningKeyHex::encode(&conn.signing_pubkey);
        if let Err(e) = redis_ops::doc_session_open(&state.redis, &document_id, &pub_key_hex).await
        {
            tracing::error!(error = %e, "subscribe: session count failed");
        }
    }
    conn.doc_access_verified_at
        .insert(document_id, Instant::now());
    // Where the document sits in a channel is read once. The socket then hears
    // the activity of the channel's threads, and a thread's delta signals it
    // (`SRV-161`). A failed read leaves the document outside every
    // channel, so it signals nothing.
    match db::channel_place(&state.db, &document_id).await {
        Ok(Some(place)) => {
            conn.doc_channels.insert(document_id, place);
        }
        Ok(None) => {}
        Err(e) => tracing::error!(error = %e, "subscribe: channel place read failed"),
    }
    // Cache the write capability so `handle_update` can reject a read-only
    // (`viewer`) member's delta without re-hitting the database per message.
    conn.doc_write.insert(document_id, DocWrite::of(role));
    let ack = Payload::server_msg(&WsServerMsg::Subscribed { document_id });
    let _ = outbound_tx.send(ack).await;
}

pub(crate) fn handle_unsubscribe(conn: &mut WsConnectionState, document_id: DocumentId) {
    conn.doc_subscriptions.remove(&document_id);
    conn.doc_access_verified_at.remove(&document_id);
    conn.doc_write.remove(&document_id);
    conn.clock_fences.forget(&document_id);
    conn.doc_channels.remove(&document_id);
}

/// Tell everyone still watching `document_id` that this connection's peer
/// stopped (`PRESENCE-014`): drop their presence field and broadcast one
/// `PeerLeft`. Called on a clean unsubscribe and once per subscription when the
/// connection closes.
///
/// One user signs every session with the same key. A session that closes while
/// another of that key still watches announces nothing. Its presence field then
/// stays for the session that remains (`PRESENCE-015`).
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
    let pub_key_hex = ambre_core::SigningKeyHex::encode(signing_pubkey);
    // A count the store cannot answer fails open: the departure is announced,
    // and the session that remains re-enters the set with its next sample.
    let last = redis_ops::doc_session_close(&state.redis, document_id, &pub_key_hex)
        .await
        .unwrap_or(true);
    if !last {
        return;
    }
    let _ = redis_ops::doc_presence_remove(&state.redis, document_id, &pub_key_hex).await;
    let broadcast = Payload::server_msg(&WsServerMsg::PeerLeft {
        document_id: *document_id,
        pub_key: signing_pubkey.clone(),
    });
    let _ = redis_ops::doc_broadcast(&state.redis, document_id, &broadcast).await;
}

pub(crate) async fn handle_update(
    state: &Arc<AppState>,
    conn: &mut WsConnectionState,
    update: ambre_protocol::Update,
    outbound_tx: &mpsc::Sender<Payload>,
) {
    let doc_id = update.public_data.document_id;
    if !conn.doc_subscriptions.contains(&doc_id) {
        refuse_delta(
            outbound_tx,
            &update,
            RefusalCode::Forbidden,
            "not subscribed",
        )
        .await;
        return;
    }

    // Cap the inline delta ciphertext before any signature check or durable
    // write. A CRDT delta is small; a large blob belongs on the media path. This
    // stops an authorized writer from pushing oversized deltas. Such deltas are
    // persisted durably (S3 + Redis pending) and fanned out to every subscriber.
    // That is the same storage/cost abuse the media and snapshot caps close, on
    // the hot path.
    let cipher_len = ambre_core::ByteSize::of_len(update.ciphertext.len());
    if cipher_len > state.config.delta_max_size {
        let detail = format!(
            "delta ciphertext {} bytes exceeds max {}",
            cipher_len.0, state.config.delta_max_size.0
        );
        refuse_delta(outbound_tx, &update, RefusalCode::PayloadTooLarge, &detail).await;
        return;
    }

    // Reject a structurally valid but semantically meaningless delta (empty
    // ciphertext, zero clock) before any DB/Redis work. Cheap, and it keeps the
    // durable index and the per-author clock hash free of no-op entries.
    if let Err(e) = update.validate() {
        let detail = format!("invalid update: {e}");
        refuse_delta(outbound_tx, &update, RefusalCode::BadRequest, &detail).await;
        return;
    }

    // Re-validate membership on a short TTL (H8). The `Subscribe`-time check is
    // stale after a mid-session revoke, so a revoked member could otherwise keep
    // writing accepted, sequenced updates until they disconnect. On failure the
    // helper drops the subscription so subsequent messages are rejected without a
    // DB hit; tell this client its access was revoked.
    if !membership_still_valid(state, conn, doc_id).await {
        let detail = "document access revoked";
        refuse_delta(outbound_tx, &update, RefusalCode::Forbidden, detail).await;
        return;
    }

    // Enforce the write role (owner/editor). Every REST write path gates on the
    // same set. Without this the hot WS write path would let a `viewer` author
    // sequenced, persisted, broadcast deltas. A viewer is a legitimately invited
    // read-only member who holds the doc key and its own signing key. The
    // capability is cached at Subscribe and refreshed by `membership_still_valid`
    // above, so this is a map lookup, not a DB hit.
    if conn.doc_write.get(&doc_id) != Some(&DocWrite::Allowed) {
        let detail = "read-only role cannot write to this document";
        refuse_delta(outbound_tx, &update, RefusalCode::Forbidden, detail).await;
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
        let detail = "author key does not match session identity";
        refuse_delta(outbound_tx, &update, RefusalCode::Forbidden, detail).await;
        return;
    }
    if let Err(e) = verify_update_signature(&author_pubkey, &update) {
        let detail = format!("invalid signature: {e}");
        refuse_delta(outbound_tx, &update, RefusalCode::InvalidSignature, &detail).await;
        return;
    }

    let pub_key_hex = ambre_core::SigningKeyHex::encode(&author_pubkey);
    let clock = update.public_data.clock;

    // A delta sent behind a refused one waits for the author's answer: it
    // takes no clock and no sequence number (`SRV-169`). The author sends it
    // again after its answer, so dropping it loses nothing.
    if conn.clock_fences.holds(&doc_id) {
        tracing::debug!("[Server] fenced delta dropped doc={doc_id} clock={clock}");
        return;
    }

    let seq =
        match redis_ops::verify_clock_and_assign_seq(&state.redis, &doc_id, &pub_key_hex, clock)
            .await
        {
            Ok(s) => {
                tracing::debug!(
                    "[Server] seq assigned doc={doc_id} author={} clock={clock} seq={s}",
                    pub_key_hex.as_str()
                );
                s
            }
            Err(e) => {
                let answer = update_refusal(&update, &e);
                conn.clock_fences.raise_for(&answer);
                let _ = outbound_tx.send(Payload::server_msg(&answer)).await;
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
    let update_wire = match Payload::seal(&update) {
        Ok(wire) => wire,
        Err(e) => {
            tracing::error!("postcard encode Update failed doc={doc_id} seq={seq}: {e}");
            return;
        }
    };

    // The index write is retried. A refused statement is most often the pool or
    // the register briefly unavailable. The number is already spent, so a write
    // given up on is a delta the index never names (`SRV-033`).
    tracing::debug!("[Server->Postgres] insert_update doc={doc_id} seq={seq} clock={clock}");
    let mut attempt = 0;
    let stored = loop {
        match store_index_row(state, doc_id, &update, seq, &s3_key, &update_wire).await {
            Err(e) if attempt < INDEX_WRITE_BACKOFF.len() => {
                tracing::warn!("insert_update refused doc={doc_id} seq={seq}, retrying: {e}");
                if let Some(pause) = INDEX_WRITE_BACKOFF.get(attempt) {
                    tokio::time::sleep(*pause).await;
                }
                attempt += 1;
            }
            other => break other,
        }
    };
    let inlined = matches!(stored, Ok(true));
    if let Err(e) = stored {
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
    let queued = if inlined {
        Ok(())
    } else {
        redis_ops::append_pending_delta(&state.redis, &doc_id, seq, &s3_key, &update_wire).await
    };
    match queued {
        Ok(()) => {}
        // The document is being deleted: the delta has nowhere to go and must
        // not be kept anywhere, nor broadcast (`SRV-017`).
        Err(AppError::NotFound { .. }) => {
            tracing::debug!("update dropped: document being deleted doc={doc_id} seq={seq}");
            return;
        }
        Err(e) => {
            // The pending list is the only path this sequenced delta takes to S3. A
            // failed push would lose it after the seq was already assigned and
            // broadcast. Its index row takes the frame instead, so a replay still
            // finds it (`SRV-034`). Log loudly and dead-letter the full wire as well,
            // so an operator sees the failure and can replay it (H1).
            tracing::error!("append_pending_delta failed doc={doc_id} seq={seq}: {e}");
            match db::attach_wire(&state.db, &doc_id, seq, &update_wire).await {
                Ok(true) => {}
                Ok(false) => tracing::error!("no index row took the frame doc={doc_id} seq={seq}"),
                Err(e) => tracing::error!("attach_wire failed doc={doc_id} seq={seq}: {e}"),
            }
            if let Err(dl) =
                redis_ops::push_dead_letter(&state.redis, &doc_id, seq, &s3_key, &update_wire).await
            {
                tracing::error!("dead-letter push failed doc={doc_id} seq={seq}: {dl}");
            }
        }
    }

    let broadcast = Payload::server_msg(&WsServerMsg::Update { seq, update });
    tracing::debug!("[Server->RedisPubSub] broadcast Update doc={doc_id} seq={seq}");
    if let Err(e) = redis_ops::doc_broadcast(&state.redis, &doc_id, &broadcast).await {
        // A dropped broadcast only delays live delivery: subscribers replay the
        // missed seq via RequestMissing on reconnect, and the delta is already
        // persisted above. Still surface it rather than swallowing (H1).
        tracing::error!("doc_broadcast failed doc={doc_id} seq={seq}: {e}");
    }
    // A thread is subscribed only by those who hold it open. Every other
    // socket on its channel hears one signal that names the thread alone
    // (`SRV-161`, `SRV-162`).
    if let Some((channel, ambre_core::ChannelDocumentKind::Thread)) = conn.doc_channels.get(&doc_id)
        && let Err(e) = redis_ops::thread_activity(&state.redis, channel, &doc_id).await
    {
        tracing::error!(error = %e, "thread activity publish failed");
    }
    // The writer has the document open and holds every delta it covers, so it
    // is the one asked once the history passes the threshold (`SRV-038`).
    crate::compaction::ask_when_due(state, doc_id, seq, outbound_tx);
}

/// The pauses between the attempts of an index write: three retries, a third
/// of a second in all. That bounds how long one delta holds its connection.
const INDEX_WRITE_BACKOFF: [Duration; 3] = [
    Duration::from_millis(20),
    Duration::from_millis(80),
    Duration::from_millis(240),
];

/// Writes the index row of delta `seq`. A small delta lives in its row, in the
/// same write as the row, and takes no other path (`SRV-148`). A large one, or
/// one that meets a full inline tail, keeps an object of its own (`SRV-149`,
/// `SRV-150`). Returns whether the row holds the delta. Both inserts ignore a
/// row already written, so a retry never doubles one.
async fn store_index_row(
    state: &Arc<AppState>,
    doc_id: DocumentId,
    update: &ambre_protocol::Update,
    seq: ambre_core::Seq,
    s3_key: &ambre_core::ObjectKey,
    wire: &Payload,
) -> Result<bool, AppError> {
    let author = &update.public_data.pub_key;
    let snapshot_id = &update.public_data.snapshot_id;
    let clock = update.public_data.clock;
    if ambre_core::ByteSize::of_len(wire.len()) < crate::hot_tail::INLINE_DELTA_MAX_SIZE {
        let inline = db::insert_inline_update(
            &state.db,
            &doc_id,
            author,
            snapshot_id,
            clock,
            seq,
            wire,
            crate::hot_tail::INLINE_CAP,
        )
        .await?;
        match inline {
            db::InlineInsert::Stored { deltas, bytes } => {
                if crate::hot_tail::past_watermark(deltas, bytes) {
                    crate::hot_tail::spawn_move(state, doc_id);
                }
                return Ok(true);
            }
            db::InlineInsert::Full => crate::hot_tail::spawn_move(state, doc_id),
        }
    }
    db::insert_update(&state.db, &doc_id, author, snapshot_id, clock, seq, s3_key).await?;
    Ok(false)
}

pub(crate) async fn handle_ephemeral(
    state: &Arc<AppState>,
    conn: &mut WsConnectionState,
    eph: ambre_protocol::Ephemeral,
) {
    let doc_id = eph.public_data.document_id;
    if !conn.doc_subscriptions.contains(&doc_id) {
        return;
    }

    // Cap the ephemeral ciphertext before any broadcast or Redis write. A presence
    // blob is tiny. This mirrors the `handle_update` delta cap for the
    // fire-and-forget path. An oversized ephemeral cannot be fanned out to every
    // subscriber and stored in the presence hash. Fire-and-forget: drop silently.
    if ambre_core::ByteSize::of_len(eph.ciphertext.len()) > state.config.ephemeral_max_size {
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

    let pub_key_hex = ambre_core::SigningKeyHex::encode(&eph.public_data.pub_key);
    // A refused write is not broadcast either: a document that owes a rotation
    // carries no presence sealed under a key a departed member holds (`SRV-123`).
    if redis_ops::doc_presence_update(
        &state.redis,
        &doc_id,
        &pub_key_hex,
        &eph.ciphertext,
        ambre_core::Seconds(15),
    )
    .await
    .is_err()
    {
        return;
    }

    let broadcast = Payload::server_msg(&WsServerMsg::Ephemeral(eph));
    let _ = redis_ops::doc_broadcast(&state.redis, &doc_id, &broadcast).await;
}

pub(crate) async fn handle_request_missing(
    state: &Arc<AppState>,
    conn: &mut WsConnectionState,
    document_id: DocumentId,
    after_seq: ambre_protocol::messages::Seq,
    outbound_tx: &mpsc::Sender<Payload>,
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
    const PAGE_SIZE: db::QueryLimit = db::QueryLimit::MAX;
    let mut cursor = after_seq;
    let mut first_page = true;
    let mut batches = crate::hot_tail::BatchCache::default();
    loop {
        let updates = match db::get_updates_after(&state.db, &document_id, cursor, PAGE_SIZE).await
        {
            Ok(u) => u,
            Err(e) => {
                // An accepted request is always answered (`SRV-040`). The pages
                // already sent stand, and the stop names the last delta they
                // delivered, so the client asks again from there (`SRV-184`).
                tracing::error!(error = %e, "failed to get updates after seq");
                let msg = Payload::server_msg(&WsServerMsg::ReplayStopped {
                    document_id,
                    after_seq: cursor,
                });
                let _ = outbound_tx.send(msg).await;
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
                let msg = Payload::server_msg(&WsServerMsg::MissingUpdates {
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
        // and the cursor for the next page. The page is not empty, checked above.
        let Some(next_cursor) = updates.last().map(|u| u.seq) else {
            return;
        };

        // Rehydrate each indexed delta into the full sealed Update the client
        // needs to decrypt and verify. A small delta's bytes are in its row,
        // and a moved one's in a batch object, fetched once (`SRV-155`). A large
        // one's are in its own object once flushed, or in the pending map until
        // then. A delta we cannot locate or decode ends the replay at that seq,
        // and the caller is told (`SRV-041`). A page with a hole in it is never
        // sent.
        //
        // The fetches run CONCURRENTLY. A full replay (no snapshot upload yet)
        // pulls every delta since genesis. A sequential `.await` per object
        // turned the open into N serial S3 round-trips.
        batches.load(state, &updates).await;
        let pending = &pending;
        let held = &batches;
        let fetches = updates.into_iter().map(|row| {
            let seq = row.seq;
            async move {
                let wire = match (row.wire, row.batch_key, row.s3_key) {
                    (Some(wire), _, _) => Some(wire),
                    (None, Some(key), _) => held.frame(&key, row.seq),
                    (None, None, Some(key)) => {
                        match crate::s3::fetch_object(&state.s3, &state.s3_bucket, &key).await {
                            Ok(Some(bytes)) => Some(bytes),
                            Ok(None) => pending.get(&seq).cloned(),
                            Err(e) => {
                                tracing::error!(seq = seq.0, error = %e, "replay: S3 fetch failed");
                                pending.get(&seq).cloned()
                            }
                        }
                    }
                    (None, None, None) => None,
                };
                let Some(wire) = wire else {
                    tracing::error!(
                        seq = seq.0,
                        "replay: delta not found in its row, its batch, S3 or pending"
                    );
                    return (seq, None);
                };
                match wire.open::<ambre_protocol::Update>() {
                    Ok(update) => (seq, Some(ambre_protocol::SeqUpdate { seq, update })),
                    Err(e) => {
                        tracing::error!(seq = seq.0, error = %e, "replay: Update decode failed");
                        (seq, None)
                    }
                }
            }
        });

        let fetched = futures_util::future::join_all(fetches).await;
        let (seq_updates, unreadable) = contiguous_prefix(fetched);
        // The last delta this page delivers, or the cursor it started past.
        let delivered = seq_updates.last().map_or(cursor, |u| u.seq);

        let msg = Payload::server_msg(&WsServerMsg::MissingUpdates {
            document_id,
            updates: seq_updates,
        });
        if outbound_tx.send(msg).await.is_err() {
            return;
        }
        if let Some(seq) = unreadable {
            tracing::warn!(seq = seq.0, "replay stopped at a delta it could not read");
            let msg = Payload::server_msg(&WsServerMsg::ReplayStopped {
                document_id,
                after_seq: delivered,
            });
            let _ = outbound_tx.send(msg).await;
            return;
        }

        // A short page means the index is exhausted.
        if page_len < PAGE_SIZE.as_usize() {
            return;
        }
        cursor = next_cursor;
        batches.forget_through(cursor);
    }
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
            conn.doc_write.insert(doc_id, DocWrite::of(role));
            true
        }
        Err(_) => {
            // The session no longer watches, so it leaves the count; a member
            // added back later then starts from the right number (`PRESENCE-015`).
            if conn.doc_subscriptions.remove(&doc_id) {
                let pub_key_hex = ambre_core::SigningKeyHex::encode(&conn.signing_pubkey);
                let _ = redis_ops::doc_session_close(&state.redis, &doc_id, &pub_key_hex).await;
            }
            conn.doc_access_verified_at.remove(&doc_id);
            conn.doc_write.remove(&doc_id);
            // The channel stays known until the unsubscribe, which leaves its
            // topic as it leaves the document's.
            false
        }
    }
}

/// Verifies the caller has a membership row for the document and returns their
/// role, so callers can gate write operations on it.
pub(crate) async fn verify_document_access(
    pool: &sqlx::PgPool,
    document_id: DocumentId,
    user_id: UserId,
) -> Result<ambre_core::Role, AppError> {
    let m = db::get_document_member(pool, &document_id, &user_id)
        .await?
        .ok_or(AppError::Forbidden)?;
    Ok(m.role)
}

#[cfg(test)]
mod postcard_tests {
    use super::refusal;
    use ambre_core::{DocumentId, PublicKey, RefusalCode, SnapshotId};
    use ambre_protocol::messages::{
        Ciphertext, Clock, CommitmentTag, Counter, Ephemeral, Nonce, Seq, SeqUpdate, Signature,
        Update,
    };
    use ambre_protocol::public_data::{EphemeralPublicData, UpdatePublicData};
    use ambre_protocol::ws::{
        WireFrame, WsClientMsg, WsServerMsg, decode_client_msg, decode_server_msg,
        encode_client_msg, encode_server_msg,
    };

    fn make_update() -> Update {
        Update {
            ciphertext: Ciphertext(vec![1, 2, 3]),
            nonce: Nonce([4u8; 24]),
            public_data: UpdatePublicData {
                document_id: DocumentId::from_bytes([5u8; 16]),
                pub_key: PublicKey::from_array([6u8; 32]),
                key_id: ambre_core::KeyId::from_array([0u8; 16]),
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
                key_id: ambre_core::KeyId::from_array([0u8; 16]),
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
        assert_eq!(bytes.as_bytes()[0], ambre_protocol::CURRENT_VERSION.as_u8());
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
            covered_seq: Seq(9),
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
        let doc = DocumentId::from_bytes([3u8; 16]);
        let msg = refusal(Some(doc), RefusalCode::Conflict);
        let decoded = decode_server_msg(&encode_server_msg(&msg)).unwrap();
        match decoded {
            WsServerMsg::Error { document_id, code } => {
                assert_eq!(document_id, Some(doc));
                assert_eq!(code, RefusalCode::Conflict);
            }
            _ => panic!("expected Error variant"),
        }
    }

    #[test]
    fn version_byte_is_the_current_version() {
        let msg = WsClientMsg::Subscribe {
            document_id: DocumentId::from_bytes([0u8; 16]),
        };
        let bytes = encode_client_msg(&msg);
        assert_eq!(bytes.as_bytes()[0], 0x01);
    }

    #[test]
    fn empty_frame_is_error() {
        assert!(decode_client_msg(&WireFrame::new(Vec::new())).is_err());
    }

    #[test]
    fn wrong_version_byte_is_error() {
        let msg = WireFrame::new(vec![0x02, 0x00]);
        assert!(decode_client_msg(&msg).is_err());
    }

    #[test]
    fn garbage_data_returns_error_not_panics() {
        let result = decode_client_msg(&WireFrame::new(vec![0x01, 0xFF, 0xFE, 0xFD]));
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
            s3_bucket: crate::s3::BucketName::new("test-bucket".to_owned()),
            config: Arc::new(crate::state::Config::default()),
            jwt: Arc::new(crate::jwt::JwtKeys::from_seed(
                &crate::jwt::JwtSeed::from_array([7u8; 32]),
            )),
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
            doc_write: HashMap::from([(doc_id, DocWrite::Allowed)]),
            last_replay_at: HashMap::new(),
            // A wide bucket so these tests exercise their own concern, not the
            // rate limit (which the loop enforces, covered by its own test).
            msg_bucket: TokenBucket::of(1000, 1000),
            clock_fences: ClockFences::default(),
            doc_channels: HashMap::new(),
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
                key_id: ambre_core::KeyId::from_array([0u8; 16]),
                counter: Counter(1),
            },
            commitment_tag: CommitmentTag([0u8; 32]),
            signature: Signature([0u8; 64]),
        }
    }

    /// A properly signed ephemeral: the transcript matches
    /// `verify_ephemeral_signature`, so the relay accepts and records it.
    pub(super) fn signed_ephemeral(
        signing_key: &ed25519_dalek::SigningKey,
        pub_key: &PublicKey,
        doc_id: DocumentId,
        ciphertext: Vec<u8>,
    ) -> Ephemeral {
        use ed25519_dalek::Signer;
        let public_data = EphemeralPublicData {
            document_id: doc_id,
            pub_key: pub_key.clone(),
            key_id: ambre_core::KeyId::from_array([0u8; 16]),
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

    /// `SRV-050`: the presence entry is the frame's ciphertext, byte for byte.
    /// The relay decodes, trims and adds nothing.
    #[tokio::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs redis; run with --features db")]
    async fn a_presence_entry_is_the_ciphertext_as_received() {
        use base64::Engine;
        let Some(state) = test_state().await else {
            return;
        };
        let doc_id = DocumentId::new();
        let signing_key = ed25519_dalek::SigningKey::from_bytes(&[23u8; 32]);
        let session_key = PublicKey::from_array(signing_key.verifying_key().to_bytes());
        let presence_key = format!("doc:{doc_id}:presence");
        let _ = state.redis.del([&presence_key]).await;
        let mut conn = conn_with(session_key.clone(), doc_id);

        let sealed: Vec<u8> = (0..=255u8).rev().collect();
        let eph = signed_ephemeral(&signing_key, &session_key, doc_id, sealed.clone());
        handle_ephemeral(&state, &mut conn, eph).await;
        let entries: std::collections::HashMap<String, String> =
            state.redis.hgetall(&presence_key).await.unwrap();
        let stored = base64::engine::general_purpose::URL_SAFE
            .decode(&entries[&session_key.to_hex()])
            .unwrap();
        assert_eq!(stored, sealed);

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
            ephemeral_max_size: ambre_core::ByteSize(8),
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

    /// PRESENCE-015: one user signs both their sessions with one key. The first
    /// to close announces nothing and keeps the presence entry; the last
    /// announces the departure.
    #[tokio::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs redis; run with --features db")]
    async fn a_departure_waits_for_the_last_session_of_its_key() {
        let Some(state) = test_state().await else {
            return;
        };
        let doc_id = DocumentId::new();
        let key = PublicKey::from_array([31u8; 32]);
        let key_hex = ambre_core::SigningKeyHex::encode(&key);
        let presence_key = format!("doc:{doc_id}:presence");
        let (tx, mut rx) = mpsc::channel(8);
        state
            .pubsub
            .subscribe(
                crate::ws::Topic::document(&doc_id),
                crate::ws::ConnectionId::named("watcher"),
                tx,
            )
            .await
            .unwrap();
        for _ in 0..2 {
            redis_ops::doc_session_open(&state.redis, &doc_id, &key_hex)
                .await
                .unwrap();
        }
        let blob = ambre_protocol::messages::Ciphertext(b"blob".to_vec());
        redis_ops::doc_presence_update(
            &state.redis,
            &doc_id,
            &key_hex,
            &blob,
            ambre_core::Seconds(15),
        )
        .await
        .unwrap();

        announce_departure(&state, &doc_id, &key).await;
        let kept: bool = state
            .redis
            .hexists(&presence_key, key_hex.as_str())
            .await
            .unwrap();
        assert!(kept, "a session of the same key still watches");

        announce_departure(&state, &doc_id, &key).await;
        let kept: bool = state
            .redis
            .hexists(&presence_key, key_hex.as_str())
            .await
            .unwrap();
        assert!(!kept, "the last session took the entry with it");

        // Exactly one departure reaches the watcher: the second.
        let frame = tokio::time::timeout(Duration::from_secs(5), rx.recv())
            .await
            .unwrap()
            .unwrap();
        let msg = ambre_protocol::ws::decode_server_msg(&ambre_protocol::WireFrame::new(
            frame.expose_bytes().to_vec(),
        ))
        .unwrap();
        assert!(matches!(msg, WsServerMsg::PeerLeft { pub_key, .. } if pub_key == key));
        let more = tokio::time::timeout(Duration::from_millis(300), rx.recv()).await;
        assert!(more.is_err(), "the first close announced nothing");

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

    async fn test_state(delta_max_bytes: u64) -> Option<Arc<AppState>> {
        let redis = rustis::client::Client::connect("redis://localhost:6379")
            .await
            .ok()?;
        let s3 = aws_sdk_s3::Client::from_conf(aws_sdk_s3::Config::builder().build());
        let pubsub = crate::ws::PubSubHub::spawn(redis.clone());
        let db = sqlx::postgres::PgPoolOptions::new()
            .connect_lazy("postgres://localhost/ambre")
            .ok()?;
        let config = crate::state::Config {
            delta_max_size: ambre_core::ByteSize(delta_max_bytes),
            ..Default::default()
        };
        Some(Arc::new(AppState {
            db,
            redis,
            pubsub,
            s3,
            s3_bucket: crate::s3::BucketName::new("test-bucket".to_owned()),
            config: Arc::new(config),
            jwt: Arc::new(crate::jwt::JwtKeys::from_seed(
                &crate::jwt::JwtSeed::from_array([7u8; 32]),
            )),
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
            doc_write: HashMap::from([(doc_id, DocWrite::Allowed)]),
            last_replay_at: HashMap::new(),
            // A wide bucket so these tests exercise their own concern, not the
            // rate limit (which the loop enforces, covered by its own test).
            msg_bucket: TokenBucket::of(1000, 1000),
            clock_fences: ClockFences::default(),
            doc_channels: HashMap::new(),
        }
    }

    fn update_with(pub_key: PublicKey, doc_id: DocumentId, cipher_len: usize) -> Update {
        Update {
            ciphertext: Ciphertext(vec![0u8; cipher_len]),
            nonce: Nonce([0u8; 24]),
            public_data: UpdatePublicData {
                document_id: doc_id,
                pub_key,
                key_id: ambre_core::KeyId::from_array([0u8; 16]),
                snapshot_id: SnapshotId::from_bytes([0u8; 16]),
                clock: Clock(1),
            },
            commitment_tag: CommitmentTag([0u8; 32]),
            signature: Signature([0u8; 64]),
        }
    }

    /// `SRV-183`, `SRV-185`: each refusal of the write path answers its code,
    /// its document and the delta it refused, and carries nothing else to leak.
    #[tokio::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs redis; run with --features db")]
    async fn each_write_refusal_names_its_delta_and_its_code() {
        let Some(state) = test_state(1024).await else {
            return;
        };
        let doc_id = DocumentId::new();
        let session_key = PublicKey::from_array([1u8; 32]);
        let mut empty = update_with(session_key.clone(), doc_id, 16);
        empty.ciphertext = Ciphertext(Vec::new());
        let cases = [
            (empty, RefusalCode::BadRequest),
            (
                update_with(PublicKey::from_array([2u8; 32]), doc_id, 16),
                RefusalCode::Forbidden,
            ),
            (
                update_with(session_key.clone(), doc_id, 16),
                RefusalCode::InvalidSignature,
            ),
        ];
        for (fill, (mut update, expected)) in (1u8..).zip(cases) {
            update.signature = Signature([fill; 64]);
            let mut conn = conn_with(session_key.clone(), doc_id);
            let (tx, mut rx) = mpsc::channel::<Payload>(8);
            handle_update(&state, &mut conn, update, &tx).await;
            let reply = rx.try_recv().expect("a refusal");
            let decoded = ambre_protocol::ws::decode_server_msg(&ambre_protocol::WireFrame::new(
                reply.expose_bytes().to_vec(),
            ))
            .unwrap();
            assert!(
                matches!(
                    decoded,
                    WsServerMsg::DeltaRefused { document_id, signature, code }
                        if document_id == doc_id && signature == Signature([fill; 64]) && code == expected
                ),
                "expected {expected}, got {decoded:?}"
            );
            assert!(rx.try_recv().is_err(), "one answer per refusal");
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
        let (tx, mut rx) = mpsc::channel::<Payload>(8);

        // 9 bytes > the 8-byte cap: rejected before any durable write.
        handle_update(&state, &mut conn, update_with(session_key, doc_id, 9), &tx).await;

        let pending_len: usize = state.redis.llen(&pending_key).await.unwrap();
        assert_eq!(
            pending_len, 0,
            "an oversized delta must not be pushed to the pending list"
        );
        // The client is told why (413), so it can move the blob to the media path.
        let reply = rx.try_recv().expect("a 413 error reply must be sent");
        let decoded = ambre_protocol::ws::decode_server_msg(&ambre_protocol::WireFrame::new(
            reply.expose_bytes().to_vec(),
        ))
        .unwrap();
        assert!(matches!(
            decoded,
            ambre_protocol::ws::WsServerMsg::DeltaRefused {
                code: RefusalCode::PayloadTooLarge,
                document_id: d,
                ..
            } if d == doc_id
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

        // Fresh membership, but a read-only role: the write permission is cached
        // read-only, exactly as `handle_subscribe` would set it for a `viewer`.
        let mut conn = conn_with(session_key.clone(), doc_id);
        conn.doc_write.insert(doc_id, DocWrite::ReadOnly);
        let (tx, mut rx) = mpsc::channel::<Payload>(8);

        handle_update(&state, &mut conn, update_with(session_key, doc_id, 16), &tx).await;

        let pending_len: usize = state.redis.llen(&pending_key).await.unwrap();
        assert_eq!(
            pending_len, 0,
            "a viewer's delta must not be pushed to the pending list"
        );
        let reply = rx.try_recv().expect("a 403 error reply must be sent");
        let decoded = ambre_protocol::ws::decode_server_msg(&ambre_protocol::WireFrame::new(
            reply.expose_bytes().to_vec(),
        ))
        .unwrap();
        assert!(matches!(
            decoded,
            ambre_protocol::ws::WsServerMsg::DeltaRefused {
                code: RefusalCode::Forbidden,
                ..
            }
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
            s3_bucket: crate::s3::BucketName::new("test-bucket".to_owned()),
            config: Arc::new(crate::state::Config::default()),
            jwt: Arc::new(crate::jwt::JwtKeys::from_seed(
                &crate::jwt::JwtSeed::from_array([7u8; 32]),
            )),
            oidc_clients: Arc::new(std::collections::HashMap::new()),
        });
        let doc_id = DocumentId::new();
        let session_key = PublicKey::from_array([1u8; 32]);

        // A fresh doc_id with no rows in the durable index: the very first page
        // of the replay is empty.
        let mut conn = conn_with(session_key, doc_id);
        let (tx, mut rx) = mpsc::channel::<Payload>(8);

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
        let decoded = ambre_protocol::ws::decode_server_msg(&ambre_protocol::WireFrame::new(
            reply.expose_bytes().to_vec(),
        ))
        .unwrap();
        match decoded {
            ambre_protocol::ws::WsServerMsg::MissingUpdates { updates, .. } => {
                assert!(updates.is_empty(), "no deltas to replay for a fresh doc");
            }
            other => panic!("expected an empty MissingUpdates, got {other:?}"),
        }
    }
}

// Who the relay serves on a live socket: the subscription set, the membership
// recheck, and the presence path kept apart from the history. Each test reads
// the stores back, so it needs Postgres and Redis.
#[cfg(test)]
mod access_tests {
    use super::*;
    use crate::test_helpers::{create_test_document, create_test_user};
    use ambre_core::{DocumentId, PublicKey, SnapshotId};
    use ambre_protocol::messages::{Ciphertext, Clock, CommitmentTag, Nonce, Signature, Update};
    use ambre_protocol::public_data::UpdatePublicData;
    use ambre_protocol::ws::{WsServerMsg, decode_server_msg};
    use rustis::commands::{GenericCommands, HashCommands, ListCommands};

    async fn state_on(pool: sqlx::PgPool) -> Arc<AppState> {
        let redis = rustis::client::Client::connect("redis://localhost:6379")
            .await
            .expect("need Redis for tests");
        let pubsub = crate::ws::PubSubHub::spawn(redis.clone());
        Arc::new(AppState {
            db: pool,
            redis,
            pubsub,
            s3: aws_sdk_s3::Client::from_conf(aws_sdk_s3::Config::builder().build()),
            s3_bucket: crate::s3::BucketName::new("test-bucket".to_owned()),
            config: Arc::new(crate::state::Config::default()),
            jwt: Arc::new(crate::jwt::JwtKeys::from_seed(
                &crate::jwt::JwtSeed::from_array([7u8; 32]),
            )),
            oidc_clients: Arc::new(HashMap::new()),
        })
    }

    /// A connection of `user_id`, subscribed to `doc_id` as a writer, whose
    /// membership was verified `age` ago.
    fn conn_of(
        user_id: UserId,
        key: PublicKey,
        doc_id: DocumentId,
        age: Duration,
    ) -> WsConnectionState {
        let verified = Instant::now()
            .checked_sub(age)
            .expect("a monotonic clock past the age");
        WsConnectionState {
            user_id,
            signing_pubkey: key,
            doc_subscriptions: HashSet::from([doc_id]),
            doc_access_verified_at: HashMap::from([(doc_id, verified)]),
            doc_write: HashMap::from([(doc_id, DocWrite::Allowed)]),
            last_replay_at: HashMap::new(),
            msg_bucket: TokenBucket::of(1000, 1000),
            clock_fences: ClockFences::default(),
            doc_channels: HashMap::new(),
        }
    }

    fn update_for(pub_key: PublicKey, doc_id: DocumentId) -> Update {
        Update {
            ciphertext: Ciphertext(vec![1, 2, 3]),
            nonce: Nonce([0u8; 24]),
            public_data: UpdatePublicData {
                document_id: doc_id,
                pub_key,
                key_id: ambre_core::KeyId::from_array([0u8; 16]),
                snapshot_id: SnapshotId::from_bytes([0u8; 16]),
                clock: Clock(1),
            },
            commitment_tag: CommitmentTag([0u8; 32]),
            signature: Signature([0u8; 64]),
        }
    }

    fn replies(rx: &mut mpsc::Receiver<Payload>) -> Vec<WsServerMsg> {
        std::iter::from_fn(|| rx.try_recv().ok())
            .map(|wire| {
                decode_server_msg(&ambre_protocol::WireFrame::new(
                    wire.expose_bytes().to_vec(),
                ))
                .unwrap()
            })
            .collect()
    }

    /// A document with its owner and one editor, the editor's id returned.
    async fn shared_document(pool: &sqlx::PgPool) -> (DocumentId, UserId) {
        let owner = create_test_user(pool, "owner@ambre.app").await;
        let editor = create_test_user(pool, "editor@ambre.app").await;
        let doc_id = create_test_document(pool, owner, "toile").await;
        db::add_document_member(
            pool,
            &doc_id,
            &editor,
            ambre_core::Role::Editor,
            &ambre_core::WrappedDocKey::new([5u8; 32].to_vec()),
            &owner,
            None,
        )
        .await
        .unwrap();
        (doc_id, editor)
    }

    /// `SRV-042`: a frame for a document this connection never subscribed to is
    /// refused before any store is read or written, on all three paths.
    #[sqlx::test]
    #[cfg_attr(
        not(feature = "db"),
        ignore = "needs postgres+redis; run with --features db"
    )]
    async fn a_frame_for_an_unsubscribed_document_is_refused(pool: sqlx::PgPool) {
        let state = state_on(pool.clone()).await;
        let (doc_id, editor) = shared_document(&pool).await;
        let signing_key = ed25519_dalek::SigningKey::from_bytes(&[31u8; 32]);
        let key = PublicKey::from_array(signing_key.verifying_key().to_bytes());
        let mut conn = conn_of(editor, key.clone(), doc_id, Duration::ZERO);
        conn.doc_subscriptions.clear();
        let (tx, mut rx) = mpsc::channel::<Payload>(8);

        handle_update(&state, &mut conn, update_for(key.clone(), doc_id), &tx).await;
        assert!(matches!(
            replies(&mut rx).as_slice(),
            [WsServerMsg::DeltaRefused {
                code: RefusalCode::Forbidden,
                ..
            }]
        ));
        let eph =
            super::ephemeral_binding_tests::signed_ephemeral(&signing_key, &key, doc_id, vec![1]);
        handle_ephemeral(&state, &mut conn, eph).await;
        handle_request_missing(
            &state,
            &mut conn,
            doc_id,
            ambre_protocol::messages::Seq(0),
            &tx,
        )
        .await;
        assert!(replies(&mut rx).is_empty(), "a replay for it sends nothing");

        let presence: HashMap<String, String> = state
            .redis
            .hgetall(format!("doc:{doc_id}:presence"))
            .await
            .unwrap();
        let pending: usize = state
            .redis
            .llen(format!("doc:{doc_id}:pending"))
            .await
            .unwrap();
        assert!(presence.is_empty() && pending == 0);
    }

    /// `SRV-043`: a member removed while connected is served until the cached
    /// check ages past the thirty-second mark, and refused on the next frame.
    #[sqlx::test]
    #[cfg_attr(
        not(feature = "db"),
        ignore = "needs postgres+redis; run with --features db"
    )]
    async fn a_member_removed_mid_session_is_no_longer_served(pool: sqlx::PgPool) {
        assert!(MEMBERSHIP_RECHECK_TTL <= Duration::from_secs(30));
        let state = state_on(pool.clone()).await;
        let (doc_id, editor) = shared_document(&pool).await;
        let key = PublicKey::from_array([32u8; 32]);
        db::remove_document_member(&pool, &doc_id, &editor, None, ambre_core::UnixSeconds(0))
            .await
            .unwrap();
        let (tx, mut rx) = mpsc::channel::<Payload>(8);

        let mut conn = conn_of(editor, key.clone(), doc_id, MEMBERSHIP_RECHECK_TTL);
        handle_update(&state, &mut conn, update_for(key, doc_id), &tx).await;
        match replies(&mut rx).as_slice() {
            [
                WsServerMsg::DeltaRefused {
                    code: RefusalCode::Forbidden,
                    document_id: d,
                    ..
                },
            ] => {
                assert_eq!(*d, doc_id);
            }
            other => panic!("expected a revocation, got {other:?}"),
        }
        assert!(
            !conn.doc_subscriptions.contains(&doc_id),
            "the subscription is dropped"
        );
        let pending: usize = state
            .redis
            .llen(format!("doc:{doc_id}:pending"))
            .await
            .unwrap();
        assert_eq!(pending, 0);
    }

    /// `SRV-085`: a replay asked after the membership was removed is refused
    /// and answered with nothing, not even an empty page.
    #[sqlx::test]
    #[cfg_attr(
        not(feature = "db"),
        ignore = "needs postgres+redis; run with --features db"
    )]
    async fn a_replay_after_a_removal_is_answered_with_nothing(pool: sqlx::PgPool) {
        let state = state_on(pool.clone()).await;
        let (doc_id, editor) = shared_document(&pool).await;
        let key = PublicKey::from_array([33u8; 32]);
        let (tx, mut rx) = mpsc::channel::<Payload>(8);

        let mut kept = conn_of(editor, key.clone(), doc_id, MEMBERSHIP_RECHECK_TTL);
        handle_request_missing(
            &state,
            &mut kept,
            doc_id,
            ambre_protocol::messages::Seq(0),
            &tx,
        )
        .await;
        assert!(
            matches!(
                replies(&mut rx).as_slice(),
                [WsServerMsg::MissingUpdates { .. }]
            ),
            "a member is answered"
        );

        db::remove_document_member(&pool, &doc_id, &editor, None, ambre_core::UnixSeconds(0))
            .await
            .unwrap();
        let mut removed = conn_of(editor, key, doc_id, MEMBERSHIP_RECHECK_TTL);
        handle_request_missing(
            &state,
            &mut removed,
            doc_id,
            ambre_protocol::messages::Seq(0),
            &tx,
        )
        .await;
        assert!(replies(&mut rx).is_empty());
        assert!(!removed.doc_subscriptions.contains(&doc_id));
    }

    /// `SRV-089`: a presence sample is recorded, and takes no sequence number,
    /// no clock and no place in the pending list or the delta index.
    #[sqlx::test]
    #[cfg_attr(
        not(feature = "db"),
        ignore = "needs postgres+redis; run with --features db"
    )]
    async fn a_presence_frame_leaves_the_history_untouched(pool: sqlx::PgPool) {
        let state = state_on(pool.clone()).await;
        let (doc_id, editor) = shared_document(&pool).await;
        let signing_key = ed25519_dalek::SigningKey::from_bytes(&[34u8; 32]);
        let key = PublicKey::from_array(signing_key.verifying_key().to_bytes());
        let mut conn = conn_of(editor, key.clone(), doc_id, Duration::ZERO);

        let eph =
            super::ephemeral_binding_tests::signed_ephemeral(&signing_key, &key, doc_id, vec![7]);
        handle_ephemeral(&state, &mut conn, eph).await;

        let presence: HashMap<String, String> = state
            .redis
            .hgetall(format!("doc:{doc_id}:presence"))
            .await
            .unwrap();
        assert!(
            presence.contains_key(&key.to_hex()),
            "the frame was recorded"
        );
        let history: usize = state
            .redis
            .exists([
                format!("doc:{doc_id}:seq"),
                format!("doc:{doc_id}:clocks"),
                format!("doc:{doc_id}:pending"),
            ])
            .await
            .unwrap();
        assert_eq!(history, 0, "no number, no clock, no pending entry");
        assert!(
            db::get_updates_after(
                &pool,
                &doc_id,
                ambre_core::Seq(0),
                crate::db::QueryLimit::saturating(10)
            )
            .await
            .unwrap()
            .is_empty()
        );
        let _ = state.redis.del(format!("doc:{doc_id}:presence")).await;
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
            key_id: ambre_core::KeyId::from_array([0u8; 16]),
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

    /// Holds `SYNC-072` on the relay: the second encoding of a valid signature
    /// is refused, and so is a small-order key with its universal forgery.
    #[test]
    fn only_the_canonical_signature_is_accepted() {
        // The order of the Ed25519 group, little-endian.
        const GROUP_ORDER: [u8; 32] = [
            0xed, 0xd3, 0xf5, 0x5c, 0x1a, 0x63, 0x12, 0x58, 0xd6, 0x9c, 0xf7, 0xa2, 0xde, 0xf9,
            0xde, 0x14, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x10,
        ];
        let signing_key = SigningKey::from_bytes(&[9u8; 32]);
        let pubkey = PublicKey::from_array(signing_key.verifying_key().to_bytes());
        let nonce = ambre_protocol::messages::Nonce([1; 24]);
        let ciphertext = ambre_protocol::messages::Ciphertext(b"c".to_vec());
        let tag = ambre_protocol::messages::CommitmentTag([2; 32]);
        let mut signed = Vec::new();
        for field in [&nonce.0[..], &ciphertext.0, b"p", &tag.0] {
            push_framed(&mut signed, field);
        }
        let sig = signing_key.sign(&signed).to_bytes();
        let check = |key: &PublicKey, signature: [u8; 64]| {
            verify_sealed_signature(
                key,
                &nonce,
                &ciphertext,
                &ambre_core::Aad::of(b"p"),
                &tag,
                &ambre_protocol::messages::Signature(signature),
            )
        };
        assert!(check(&pubkey, sig).is_ok());

        // The scalar raised by the group order: the same point equation holds.
        let mut malleated = sig;
        let mut carry = 0u16;
        for (s, l) in malleated[32..].iter_mut().zip(GROUP_ORDER) {
            let sum = u16::from(*s) + u16::from(l) + carry;
            *s = sum as u8;
            carry = sum >> 8;
        }
        assert!(check(&pubkey, malleated).is_err());

        // The identity point, and (R = identity, S = 0), valid for any message
        // under a lax check.
        let mut identity = [0u8; 32];
        identity[0] = 1;
        let mut forged = [0u8; 64];
        forged[..32].copy_from_slice(&identity);
        let small = PublicKey::from_array(identity);
        assert!(check(&small, forged).is_err());
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
                key_id: ambre_core::KeyId::from_array([0u8; 16]),
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
            key_id: ambre_core::KeyId::from_array([0u8; 16]),
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
            key_id: ambre_core::KeyId::from_array([0u8; 16]),
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
                key_id: ambre_core::KeyId::from_array([0u8; 16]),
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
                    key_id: ambre_core::KeyId::from_array([0u8; 16]),
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
        use ambre_core::Seq;
        let fetched = vec![
            (Seq(4), Some(seq_update(4))),
            (Seq(2), Some(seq_update(2))),
            (Seq(3), None),
            (Seq(1), Some(seq_update(1))),
        ];
        let (page, unreadable) = contiguous_prefix(fetched);
        let seqs: Vec<u64> = page.iter().map(|u| u.seq.0).collect();
        assert_eq!(seqs, [1, 2], "seq 4 is held back behind the hole at 3");
        assert_eq!(unreadable, Some(Seq(3)));

        let (page, unreadable) = contiguous_prefix(vec![
            (ambre_core::Seq(2), Some(seq_update(2))),
            (ambre_core::Seq(1), Some(seq_update(1))),
        ]);
        assert_eq!(page.len(), 2);
        assert_eq!(unreadable, None);
    }

    #[test]
    fn a_refused_clock_carries_the_expected_and_the_received_value() {
        let update = seq_update(9).update;
        let refusal = update_refusal(
            &update,
            &AppError::ClockMismatch {
                expected: ambre_protocol::messages::Clock(5),
                got: ambre_protocol::messages::Clock(9),
            },
        );
        let WsServerMsg::ClockRefused {
            document_id,
            clock,
            expected,
            signature,
        } = refusal
        else {
            panic!("expected a clock refusal, got {refusal:?}");
        };
        assert_eq!(document_id, update.public_data.document_id);
        assert_eq!((clock.0, expected.0), (9, 5));
        assert_eq!(signature, update.signature, "the refusal names the delta");
    }

    /// `SRV-169`: after a refused clock, the socket drops the document's later
    /// deltas. Only the author's answer to the delta that raised the fence
    /// lifts it, and leaving the document forgets it.
    #[test]
    fn a_refused_clock_fences_the_socket_until_its_answer() {
        use ambre_protocol::messages::Signature;
        let doc = DocumentId::new();
        let other = DocumentId::new();
        let mut fences = ClockFences::default();
        fences.raise(doc, Signature([1; 64]));
        assert!(fences.holds(&doc) && !fences.holds(&other));
        fences.answer(doc, &Signature([2; 64]));
        assert!(
            fences.holds(&doc),
            "an answer to another delta lifts nothing"
        );
        fences.answer(doc, &Signature([1; 64]));
        assert!(!fences.holds(&doc));
        fences.raise(doc, Signature([3; 64]));
        fences.forget(&doc);
        assert!(!fences.holds(&doc));
    }

    #[test]
    fn a_store_failure_reaches_the_client_without_its_name_or_text() {
        let cause = AppError::Redis("clock validation failed: ERR connection refused".into());
        let update = seq_update(1).update;
        let doc = update.public_data.document_id;
        assert!(matches!(
            update_refusal(&update, &cause),
            WsServerMsg::DeltaRefused {
                document_id,
                signature,
                code: RefusalCode::InternalError,
            } if document_id == doc && signature == update.signature
        ));
    }

    /// `SRV-115`: a delta for a locked segment is refused with the lock's
    /// code, and the refusal names the delta.
    #[test]
    fn a_delta_for_a_locked_segment_is_refused_with_the_lock() {
        let update = seq_update(2).update;
        let doc = update.public_data.document_id;
        assert!(matches!(
            update_refusal(&update, &AppError::SegmentLocked),
            WsServerMsg::DeltaRefused {
                document_id,
                signature,
                code: RefusalCode::SegmentLocked,
            } if document_id == doc && signature == update.signature
        ));
    }

    /// `SRV-186`: a delta the relay failed to sequence is named, and fences
    /// the socket like a refused clock. A refusal that took a clock's place
    /// for another cause raises nothing.
    #[test]
    fn a_delta_the_relay_failed_to_sequence_is_named_and_fences_the_socket() {
        let update = seq_update(3).update;
        let doc = update.public_data.document_id;
        let failed = update_refusal(&update, &AppError::Redis("down".into()));
        let mut fences = ClockFences::default();
        fences.raise_for(&failed);
        assert!(fences.holds(&doc), "the failed delta fences its document");
        fences.answer(doc, &update.signature);
        assert!(!fences.holds(&doc), "the author's answer lifts it");

        let deleted = AppError::NotFound {
            resource: crate::error::Resource::Document,
        };
        fences.raise_for(&update_refusal(&update, &deleted));
        fences.raise_for(&update_refusal(&update, &AppError::RotationOwed));
        assert!(
            !fences.holds(&doc),
            "a deleted document or an owed key fences nothing"
        );
    }

    /// `SRV-096`: a subscription refused on a store failure answers without the
    /// store's text. A plain refusal still reads as forbidden.
    #[test]
    fn a_refused_subscription_carries_no_store_text() {
        let doc = DocumentId::new();
        let cause = AppError::Database(sqlx::Error::Protocol(
            "relation \"document_members\" does not exist".into(),
        ));
        let code_of = |answer: WsServerMsg| match answer {
            WsServerMsg::Error {
                document_id: Some(d),
                code,
            } if d == doc => Some(code),
            _ => None,
        };
        assert_eq!(
            code_of(subscribe_refusal(doc, &cause)),
            Some(RefusalCode::InternalError)
        );
        assert_eq!(
            code_of(subscribe_refusal(doc, &AppError::Forbidden)),
            Some(RefusalCode::Forbidden)
        );
        let unseen = AppError::NotFound {
            resource: crate::error::Resource::Document,
        };
        assert_eq!(
            code_of(subscribe_refusal(doc, &unseen)),
            Some(RefusalCode::Forbidden),
            "a document the caller cannot see answers as one it may not join"
        );
    }

    /// `SRV-123`: a frame refused while a rotation is owed is answered by name.
    /// The author learns which key and which clock to seal again.
    #[test]
    fn a_frame_refused_for_its_key_names_the_key_and_the_clock() {
        let update = seq_update(4).update;
        let refusal = update_refusal(&update, &AppError::RotationOwed);
        let WsServerMsg::KeyRefused {
            document_id,
            key_id,
            clock,
        } = refusal
        else {
            panic!("expected a key refusal");
        };
        assert_eq!(document_id, update.public_data.document_id);
        assert_eq!(key_id, update.public_data.key_id);
        assert_eq!(clock.0, 4);
    }
}

// Where a delta lives once sequenced: in its index row, in a batch object, or
// in an object of its own. Each test reads the stores back, so it needs
// Postgres and Redis.
#[cfg(test)]
mod delivery_tests;

#[cfg(test)]
mod hot_tail_tests {
    use super::*;
    use crate::test_helpers::{
        S3Stub, create_test_document, create_test_user, offline_s3, test_app_state, with_s3,
    };
    use ambre_core::{PublicKey, SnapshotId};
    use ambre_protocol::messages::{Ciphertext, Clock, CommitmentTag, Nonce, Signature, Update};
    use ambre_protocol::public_data::UpdatePublicData;
    use rustis::commands::{GenericCommands, ListCommands};

    /// A writer of a fresh document, over a state whose object store is `stub`.
    pub(super) async fn writer(
        pool: sqlx::PgPool,
        stub: &S3Stub,
        key: &ed25519_dalek::SigningKey,
    ) -> (Arc<AppState>, WsConnectionState, DocumentId) {
        let state = with_s3(
            &*test_app_state(pool.clone()).await,
            offline_s3(Some(&stub.endpoint)),
        );
        let owner = create_test_user(&pool, "writer@ambre.app").await;
        let doc_id = create_test_document(&pool, owner, "prose").await;
        let conn = WsConnectionState {
            user_id: owner,
            signing_pubkey: PublicKey::from_array(key.verifying_key().to_bytes()),
            doc_subscriptions: HashSet::from([doc_id]),
            doc_access_verified_at: HashMap::from([(doc_id, Instant::now())]),
            doc_write: HashMap::from([(doc_id, DocWrite::Allowed)]),
            last_replay_at: HashMap::new(),
            msg_bucket: TokenBucket::of(1000, 1000),
            clock_fences: ClockFences::default(),
            doc_channels: HashMap::new(),
        };
        (state, conn, doc_id)
    }

    /// Delta `clock` of `doc_id`, `len` bytes of ciphertext, signed by `key`.
    pub(super) fn signed_update(
        key: &ed25519_dalek::SigningKey,
        doc_id: DocumentId,
        clock: u64,
        len: usize,
    ) -> Update {
        use ed25519_dalek::Signer;
        let public_data = UpdatePublicData {
            document_id: doc_id,
            pub_key: PublicKey::from_array(key.verifying_key().to_bytes()),
            key_id: ambre_core::KeyId::from_array([0u8; 16]),
            snapshot_id: SnapshotId::from_bytes([0u8; 16]),
            clock: Clock(clock),
        };
        let ciphertext = vec![u8::try_from(clock % 256).unwrap(); len];
        let nonce = Nonce([5u8; 24]);
        let commitment_tag = CommitmentTag([6u8; 32]);
        let mut signed = Vec::new();
        push_framed(&mut signed, &nonce.0);
        push_framed(&mut signed, &ciphertext);
        push_framed(&mut signed, &postcard::to_allocvec(&public_data).unwrap());
        push_framed(&mut signed, &commitment_tag.0);
        Update {
            ciphertext: Ciphertext(ciphertext),
            nonce,
            public_data,
            commitment_tag,
            signature: Signature(key.sign(&signed).to_bytes()),
        }
    }

    /// The frames a full replay of `doc_id` sends, as they leave the relay.
    pub(super) async fn replay(
        state: &Arc<AppState>,
        conn: &mut WsConnectionState,
        doc_id: DocumentId,
    ) -> Vec<Vec<u8>> {
        conn.last_replay_at.clear();
        let (tx, mut rx) = mpsc::channel::<Payload>(8);
        handle_request_missing(state, conn, doc_id, ambre_protocol::messages::Seq(0), &tx).await;
        std::iter::from_fn(|| rx.try_recv().ok())
            .map(|frame| frame.expose_bytes().to_vec())
            .collect()
    }

    /// `SRV-148`, `SRV-149`: a small delta is stored in its index row and takes
    /// no other path. A delta past the inline size keeps an object of its own,
    /// reached through the pending list.
    #[sqlx::test]
    #[cfg_attr(
        not(feature = "db"),
        ignore = "needs postgres+redis; run with --features db"
    )]
    async fn a_small_delta_lives_in_its_row_and_a_large_one_in_its_own_object(pool: sqlx::PgPool) {
        let stub = S3Stub::spawn([]).await;
        let key = ed25519_dalek::SigningKey::from_bytes(&[41u8; 32]);
        let (state, mut conn, doc_id) = writer(pool, &stub, &key).await;
        let (tx, _rx) = mpsc::channel::<Payload>(8);
        let pending_key = format!("doc:{doc_id}:pending");

        handle_update(&state, &mut conn, signed_update(&key, doc_id, 1, 16), &tx).await;
        let rows = db::get_updates_after(
            &state.db,
            &doc_id,
            ambre_core::Seq(0),
            crate::db::QueryLimit::saturating(10),
        )
        .await
        .unwrap();
        assert!(matches!(rows.as_slice(), [r] if r.wire.is_some() && r.s3_key.is_none()));
        let pending: usize = state.redis.llen(pending_key.as_str()).await.unwrap();
        assert_eq!(pending, 0, "an inline delta takes no pending entry");

        let large = crate::hot_tail::INLINE_DELTA_MAX_SIZE.as_len() + 1024;
        handle_update(
            &state,
            &mut conn,
            signed_update(&key, doc_id, 2, large),
            &tx,
        )
        .await;
        let rows = db::get_updates_after(
            &state.db,
            &doc_id,
            ambre_core::Seq(1),
            crate::db::QueryLimit::saturating(10),
        )
        .await
        .unwrap();
        assert!(matches!(rows.as_slice(), [r] if r.wire.is_none() && r.s3_key.is_some()));
        let pending: usize = state.redis.llen(pending_key.as_str()).await.unwrap();
        assert_eq!(pending, 1, "a large delta waits for the flush");
        let _: usize = state.redis.del(pending_key).await.unwrap();
    }

    /// `SRV-037`, `SRV-169`: two sessions of one key race for clock 1. The
    /// loser hears a refusal naming its delta. Its clock-2 delta, sent behind
    /// it, fits the clock the winner left and is still dropped. After the
    /// answer, the delta sealed again at clock 2 lands.
    #[sqlx::test]
    #[cfg_attr(
        not(feature = "db"),
        ignore = "needs postgres+redis; run with --features db"
    )]
    async fn a_delta_behind_a_refused_clock_waits_for_the_answer(pool: sqlx::PgPool) {
        let stub = S3Stub::spawn([]).await;
        let key = ed25519_dalek::SigningKey::from_bytes(&[43u8; 32]);
        let (state, mut first, doc_id) = writer(pool, &stub, &key).await;
        let mut second = WsConnectionState {
            user_id: first.user_id,
            signing_pubkey: first.signing_pubkey.clone(),
            doc_subscriptions: HashSet::from([doc_id]),
            doc_access_verified_at: HashMap::from([(doc_id, Instant::now())]),
            doc_write: HashMap::from([(doc_id, DocWrite::Allowed)]),
            last_replay_at: HashMap::new(),
            msg_bucket: TokenBucket::of(1000, 1000),
            clock_fences: ClockFences::default(),
            doc_channels: HashMap::new(),
        };
        let (tx, _rx) = mpsc::channel::<Payload>(8);
        let (second_tx, mut second_rx) = mpsc::channel::<Payload>(8);
        let stored = |state: Arc<AppState>| async move {
            db::get_updates_after(
                &state.db,
                &doc_id,
                ambre_core::Seq(0),
                crate::db::QueryLimit::saturating(10),
            )
            .await
            .unwrap()
            .len()
        };

        handle_update(&state, &mut first, signed_update(&key, doc_id, 1, 16), &tx).await;
        let lost = signed_update(&key, doc_id, 1, 20);
        let lost_signature = lost.signature;
        handle_update(&state, &mut second, lost, &second_tx).await;
        let refusal = ambre_protocol::ws::decode_server_msg(&ambre_protocol::WireFrame::new(
            second_rx.try_recv().unwrap().expose_bytes().to_vec(),
        ))
        .unwrap();
        let WsServerMsg::ClockRefused {
            clock,
            expected,
            signature,
            ..
        } = refusal
        else {
            panic!("expected a clock refusal, got {refusal:?}");
        };
        assert_eq!((clock.0, expected.0), (1, 2));
        assert_eq!(signature, lost_signature);

        let behind = signed_update(&key, doc_id, 2, 20);
        handle_update(&state, &mut second, behind, &second_tx).await;
        assert!(
            second_rx.try_recv().is_err(),
            "a fenced delta is dropped unanswered"
        );
        assert_eq!(stored(state.clone()).await, 1, "clock 2 is still free");

        second.clock_fences.answer(doc_id, &lost_signature);
        let again = signed_update(&key, doc_id, 2, 24);
        handle_update(&state, &mut second, again, &second_tx).await;
        assert_eq!(stored(state.clone()).await, 2, "the answer lifts the fence");
        let pending_key = format!("doc:{doc_id}:pending");
        let _: usize = state.redis.del(pending_key).await.unwrap();
    }

    /// `SRV-153`, `SRV-155`: a replay across a move sends the same frames as
    /// before it, and reads the batch object once.
    #[sqlx::test]
    #[cfg_attr(
        not(feature = "db"),
        ignore = "needs postgres+redis; run with --features db"
    )]
    async fn a_replay_reads_the_rows_and_each_batch_once(pool: sqlx::PgPool) {
        let stub = S3Stub::spawn([]).await;
        let key = ed25519_dalek::SigningKey::from_bytes(&[42u8; 32]);
        let (state, mut conn, doc_id) = writer(pool, &stub, &key).await;
        let (tx, _rx) = mpsc::channel::<Payload>(8);
        for clock in 1..=6 {
            handle_update(
                &state,
                &mut conn,
                signed_update(&key, doc_id, clock, 16),
                &tx,
            )
            .await;
        }
        let before = replay(&state, &mut conn, doc_id).await;
        assert_eq!(before.len(), 1, "one page");

        assert_eq!(
            crate::hot_tail::move_to_batch(&state, &doc_id, Some(2))
                .await
                .unwrap(),
            4
        );
        let batch = crate::hot_tail::batch_key(
            &state.config.s3_env_prefix,
            &doc_id,
            ambre_core::Seq(1),
            ambre_core::Seq(4),
        );
        assert_eq!(replay(&state, &mut conn, doc_id).await, before);
        assert_eq!(
            stub.gets(batch.as_str()),
            1,
            "the batch is read once for its four deltas"
        );
    }
}

// The gates of a channel document are the gates of any document (`SRV-047`,
// `SRV-048`). The access check resolves a segment to the member rows of its
// channel's `meta` document, so the handlers need no channel branch.
#[cfg(test)]
mod channel_gate_tests {
    use super::*;
    use crate::test_helpers::test_app_state;
    use ambre_core::{ChannelId, DocumentId, PublicKey, SnapshotId};
    use ambre_protocol::messages::{Ciphertext, Clock, CommitmentTag, Nonce, Signature, Update};
    use ambre_protocol::public_data::UpdatePublicData;
    use rustis::commands::{GenericCommands, ListCommands};

    fn uuid_of(id: impl Into<ambre_core::Uuid>) -> uuid::Uuid {
        uuid::Uuid::from_bytes(*id.into().as_bytes())
    }

    fn conn_of(user_id: UserId, session_key: PublicKey) -> WsConnectionState {
        WsConnectionState {
            user_id,
            signing_pubkey: session_key,
            doc_subscriptions: HashSet::new(),
            doc_access_verified_at: HashMap::new(),
            doc_write: HashMap::new(),
            last_replay_at: HashMap::new(),
            msg_bucket: TokenBucket::of(1000, 1000),
            clock_fences: ClockFences::default(),
            doc_channels: HashMap::new(),
        }
    }

    /// A channel of `owner` with its `meta` and one segment, and `member`
    /// granted `role` on `meta`. Returns the segment.
    async fn segment_of_channel(
        pool: &sqlx::PgPool,
        owner: UserId,
        member: Option<(UserId, ambre_core::Role)>,
    ) -> DocumentId {
        let channel = ChannelId::new();
        sqlx::query("INSERT INTO channels (id) VALUES ($1)")
            .bind(uuid_of(channel))
            .execute(pool)
            .await
            .expect("channel");
        let meta = crate::test_helpers::create_test_document(pool, owner, "murmure").await;
        let segment = crate::test_helpers::create_test_document(pool, owner, "murmure").await;
        for (document, kind, segment_state) in [
            (meta, ambre_core::ChannelDocumentKind::Meta, None),
            (
                segment,
                ambre_core::ChannelDocumentKind::Segment,
                Some("active"),
            ),
        ] {
            sqlx::query(
                "INSERT INTO channel_documents (document_id, channel_id, kind, state) \
                 VALUES ($1, $2, $3, $4)",
            )
            .bind(uuid_of(document))
            .bind(uuid_of(channel))
            .bind(kind.as_str())
            .bind(segment_state)
            .execute(pool)
            .await
            .expect("group");
        }
        if let Some((member, role)) = member {
            db::add_document_member(
                pool,
                &meta,
                &member,
                role,
                &ambre_core::WrappedDocKey::new([6u8; 32].to_vec()),
                &owner,
                None,
            )
            .await
            .expect("member");
        }
        segment
    }

    fn update_on(pub_key: PublicKey, doc_id: DocumentId) -> Update {
        Update {
            ciphertext: Ciphertext(vec![0u8; 16]),
            nonce: Nonce([0u8; 24]),
            public_data: UpdatePublicData {
                document_id: doc_id,
                pub_key,
                key_id: ambre_core::KeyId::from_array([0u8; 16]),
                snapshot_id: SnapshotId::from_bytes([0u8; 16]),
                clock: Clock(1),
            },
            commitment_tag: CommitmentTag([0u8; 32]),
            signature: Signature([0u8; 64]),
        }
    }

    /// The code of a refusal, of a wire message or of a delta.
    fn error_code(frame: &Payload) -> Option<RefusalCode> {
        match ambre_protocol::ws::decode_server_msg(&ambre_protocol::WireFrame::new(
            frame.expose_bytes().to_vec(),
        ))
        .ok()?
        {
            WsServerMsg::Error { code, .. } | WsServerMsg::DeltaRefused { code, .. } => Some(code),
            _ => None,
        }
    }

    /// `SRV-047`: a caller who is no member of the channel is refused a
    /// subscription to its segment, and holds none.
    #[sqlx::test]
    #[cfg_attr(
        not(feature = "db"),
        ignore = "needs postgres+redis; run with --features db"
    )]
    async fn a_segment_refuses_a_caller_outside_its_channel(pool: sqlx::PgPool) {
        let owner = crate::test_helpers::create_test_user(&pool, "owner@test.dev").await;
        let eve = crate::test_helpers::create_test_user(&pool, "eve@test.dev").await;
        let segment = segment_of_channel(&pool, owner, None).await;
        let state = test_app_state(pool).await;
        let mut conn = conn_of(eve, PublicKey::from_array([2u8; 32]));
        let (tx, mut rx) = mpsc::channel::<Payload>(8);

        handle_subscribe(&state, &mut conn, segment, &tx).await;

        assert_eq!(
            error_code(&rx.try_recv().expect("a reply")),
            Some(RefusalCode::Forbidden)
        );
        assert!(!conn.doc_subscriptions.contains(&segment));
    }

    /// `SRV-047`: a member of the channel subscribes to its segment through
    /// the member row of `meta`, with the write capability of that row.
    #[sqlx::test]
    #[cfg_attr(
        not(feature = "db"),
        ignore = "needs postgres+redis; run with --features db"
    )]
    async fn a_member_of_the_channel_subscribes_to_its_segment(pool: sqlx::PgPool) {
        let owner = crate::test_helpers::create_test_user(&pool, "owner@test.dev").await;
        let bob = crate::test_helpers::create_test_user(&pool, "bob@test.dev").await;
        let segment = segment_of_channel(&pool, owner, Some((bob, ambre_core::Role::Editor))).await;
        let state = test_app_state(pool).await;
        let mut conn = conn_of(bob, PublicKey::from_array([3u8; 32]));
        let (tx, mut rx) = mpsc::channel::<Payload>(8);

        handle_subscribe(&state, &mut conn, segment, &tx).await;

        let reply = rx.try_recv().expect("a reply");
        let ack = ambre_protocol::ws::decode_server_msg(&ambre_protocol::WireFrame::new(
            reply.expose_bytes().to_vec(),
        ));
        assert!(
            matches!(ack, Ok(WsServerMsg::Subscribed { document_id }) if document_id == segment)
        );
        assert_eq!(conn.doc_write.get(&segment), Some(&DocWrite::Allowed));
    }

    /// `SRV-048`: a wire message on a segment passes the write gate of any document.
    /// A viewer of the channel is refused a delta before any durable write.
    #[sqlx::test]
    #[cfg_attr(
        not(feature = "db"),
        ignore = "needs postgres+redis; run with --features db"
    )]
    async fn a_viewer_of_the_channel_writes_no_delta_on_its_segment(pool: sqlx::PgPool) {
        let owner = crate::test_helpers::create_test_user(&pool, "owner@test.dev").await;
        let bob = crate::test_helpers::create_test_user(&pool, "bob@test.dev").await;
        let segment = segment_of_channel(&pool, owner, Some((bob, ambre_core::Role::Viewer))).await;
        let state = test_app_state(pool).await;
        let session_key = PublicKey::from_array([4u8; 32]);
        let mut conn = conn_of(bob, session_key.clone());
        let (tx, mut rx) = mpsc::channel::<Payload>(8);
        handle_subscribe(&state, &mut conn, segment, &tx).await;
        let _ = rx.try_recv();
        let pending_key = format!("doc:{segment}:pending");

        handle_update(&state, &mut conn, update_on(session_key, segment), &tx).await;

        assert_eq!(
            error_code(&rx.try_recv().expect("a reply")),
            Some(RefusalCode::Forbidden)
        );
        let pending: usize = state.redis.llen(&pending_key).await.expect("llen");
        assert_eq!(pending, 0, "a viewer's delta must not be pushed");
        let _ = state.redis.del([&pending_key]).await;
    }

    /// `SRV-161`, `SRV-162`: a delta on a thread document publishes one
    /// signal on the topic of its channel. The signal is the thread's
    /// identifier and nothing else. A delta on a segment publishes none.
    #[sqlx::test]
    #[cfg_attr(
        not(feature = "db"),
        ignore = "needs postgres+redis; run with --features db"
    )]
    async fn a_delta_on_a_thread_signals_its_channel_and_names_it_alone(pool: sqlx::PgPool) {
        use futures_util::StreamExt;
        use rustis::commands::PubSubCommands;
        let owner = crate::test_helpers::create_test_user(&pool, "owner@test.dev").await;
        let segment = segment_of_channel(&pool, owner, None).await;
        let channel = db::channel_of_document(&pool, &segment)
            .await
            .expect("read")
            .expect("a channel");
        let thread = crate::test_helpers::create_test_document(&pool, owner, "murmure").await;
        db::register_thread(&pool, &channel, ambre_core::MessageId::new(), thread)
            .await
            .expect("registered");
        let state = test_app_state(pool).await;
        let key = ed25519_dalek::SigningKey::from_bytes(&[61u8; 32]);
        let mut conn = conn_of(owner, PublicKey::from_array(key.verifying_key().to_bytes()));
        let (tx, mut rx) = mpsc::channel::<Payload>(16);
        for document in [thread, segment] {
            handle_subscribe(&state, &mut conn, document, &tx).await;
        }
        while rx.try_recv().is_ok() {}
        // A connection of its own: a subscriber sharing the relay's client
        // would desynchronise its replies (see `PubSubHub`).
        let subscriber = rustis::client::Client::connect("redis://localhost:6379")
            .await
            .expect("redis");
        let mut stream = subscriber
            .subscribe(format!("channel:{channel}"))
            .await
            .expect("a subscription");
        let wait = std::time::Duration::from_millis(500);

        let update = super::hot_tail_tests::signed_update(&key, thread, 1, 16);
        handle_update(&state, &mut conn, update, &tx).await;

        let msg = tokio::time::timeout(wait, stream.next())
            .await
            .expect("a signal")
            .expect("an open stream")
            .expect("a message");
        let alone = ambre_protocol::ws::encode_server_msg(&WsServerMsg::ThreadActivity {
            document_id: thread,
        });
        assert_eq!(msg.payload(), alone.as_bytes());

        let update = super::hot_tail_tests::signed_update(&key, segment, 1, 16);
        handle_update(&state, &mut conn, update, &tx).await;
        assert!(
            tokio::time::timeout(wait, stream.next()).await.is_err(),
            "a segment delta signals nothing"
        );
        for document in [thread, segment] {
            let _: Result<usize, _> = state
                .redis
                .del(crate::redis_keys::DocumentStateKeys::of(&document).texts())
                .await;
        }
    }
}
