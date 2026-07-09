// ambre-server/src/ws/handler/message.rs
// Per-message WebSocket handlers (subscribe/update/ephemeral/request-missing +
// channel variants), access/signature verification, and their unit tests.

use super::*;

pub(crate) async fn handle_subscribe(
    state: &Arc<AppState>,
    conn: &mut WsConnectionState,
    document_id: DocumentId,
    outbound_tx: &mpsc::Sender<Vec<u8>>,
) {
    if let Err(e) = verify_document_access(&state.db, document_id, conn.user_id).await {
        let err = encode_server_msg(&WsServerMsg::Error {
            code: 403,
            message: e.to_string(),
        });
        let _ = outbound_tx.send(err).await;
        return;
    }
    conn.doc_subscriptions.insert(document_id);
    conn.doc_access_verified_at
        .insert(document_id, Instant::now());
    let _ = redis_ops::doc_subscribe_session(&state.redis, &document_id, &conn.connection_id).await;
    let ack = encode_server_msg(&WsServerMsg::Subscribed { document_id });
    let _ = outbound_tx.send(ack).await;
}

pub(crate) async fn handle_unsubscribe(
    state: &Arc<AppState>,
    conn: &mut WsConnectionState,
    document_id: DocumentId,
) {
    conn.doc_subscriptions.remove(&document_id);
    conn.doc_access_verified_at.remove(&document_id);
    let _ =
        redis_ops::doc_unsubscribe_session(&state.redis, &document_id, &conn.connection_id).await;
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

    // Re-validate membership on a short TTL (H8): the `Subscribe`-time check is
    // stale after a mid-session revoke, so a revoked member could otherwise keep
    // writing accepted, sequenced updates until they disconnect. On failure, drop
    // the subscription so subsequent messages are rejected without a DB hit.
    let stale = conn
        .doc_access_verified_at
        .get(&doc_id)
        .is_none_or(|t| t.elapsed() >= MEMBERSHIP_RECHECK_TTL);
    if stale {
        if let Err(e) = verify_document_access(&state.db, doc_id, conn.user_id).await {
            conn.doc_subscriptions.remove(&doc_id);
            conn.doc_access_verified_at.remove(&doc_id);
            let _ = redis_ops::doc_unsubscribe_session(&state.redis, &doc_id, &conn.connection_id)
                .await;
            let err = encode_server_msg(&WsServerMsg::Error {
                code: 403,
                message: e.to_string(),
            });
            let _ = outbound_tx.send(err).await;
            return;
        }
        conn.doc_access_verified_at.insert(doc_id, Instant::now());
    }

    let author_pubkey = update.public_data.pub_key.clone();
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
                let err = encode_server_msg(&WsServerMsg::Error {
                    code: 409,
                    message: e.to_string(),
                });
                let _ = outbound_tx.send(err).await;
                return;
            }
        };

    let s3_key = crate::s3::format_s3_delta_key(&state.config.s3_env_prefix, &doc_id, seq);
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
        // The delta index is how reconnecting clients replay missed updates;
        // a failed insert silently loses durability, so it must be loud.
        tracing::error!("insert_update failed doc={doc_id} seq={seq}: {e}");
    }

    // Persist the full sealed Update so it can be replayed verbatim later
    // (`handle_request_missing`). Serialize before the broadcast moves `update`.
    // Encoding a just-decoded Update is effectively infallible, but a request-
    // path `.expect()` would panic the whole connection task on the off chance
    // it isn't; log and abort just this update instead (the seq is already
    // assigned, so a reconnecting client refetches it from the durable index).
    let update_wire = match postcard::to_allocvec(&update) {
        Ok(wire) => wire,
        Err(e) => {
            tracing::error!("postcard encode Update failed doc={doc_id} seq={seq}: {e}");
            return;
        }
    };
    tracing::debug!("[Server->Redis] append_pending_delta doc={doc_id} seq={seq} s3_key={s3_key}");
    if let Err(e) =
        redis_ops::append_pending_delta(&state.redis, &doc_id, seq, &s3_key, &update_wire).await
    {
        // The pending list is the only path this sequenced delta takes to S3, so
        // a failed push loses it durably after the seq was already assigned and
        // will be acknowledged. Log loudly and dead-letter the full wire so an
        // operator can replay it (H1).
        tracing::error!("append_pending_delta failed doc={doc_id} seq={seq}: {e}");
        if let Err(dl) =
            redis_ops::push_dead_letter(&state.redis, &doc_id, seq, &s3_key, &update_wire).await
        {
            tracing::error!("dead-letter push failed doc={doc_id} seq={seq}: {dl}");
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
    conn: &WsConnectionState,
    eph: ambre_protocol::Ephemeral,
) {
    use base64::Engine;
    let doc_id = eph.public_data.document_id;
    if !conn.doc_subscriptions.contains(&doc_id) {
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
    conn: &WsConnectionState,
    document_id: DocumentId,
    after_seq: ambre_protocol::messages::Seq,
    outbound_tx: &mpsc::Sender<Vec<u8>>,
) {
    if !conn.doc_subscriptions.contains(&document_id) {
        return;
    }

    let updates =
        match db::get_updates_after(&state.db, &document_id, after_seq.0 as i64, 1000).await {
            Ok(u) => u,
            Err(e) => {
                tracing::error!(error = %e, "failed to get updates after seq");
                return;
            }
        };

    // Rehydrate each indexed delta into the full sealed Update the client needs
    // to decrypt and verify. The bytes live in S3 once flushed, or in the Redis
    // pending list until then; together they cover every seq. A delta we cannot
    // locate or decode is skipped (logged) rather than dropping the whole batch.
    //
    // The fetches run CONCURRENTLY: a full replay (no snapshot upload yet) pulls
    // every delta since genesis, and a sequential `.await` per object turned the
    // open into N serial S3 round-trips (seconds for a doc with hundreds of edits
    // behind only a few dozen cells). `get_updates_after` returns rows ordered by
    // seq; we resolve them in parallel, then re-sort by seq to restore that order.
    let fetches = updates.into_iter().map(|row| {
        let seq = row.seq as u64;
        async move {
            let wire = match crate::s3::fetch_object(&state.s3, &state.s3_bucket, &row.s3_key).await
            {
                Ok(Some(bytes)) => Some(bytes),
                Ok(None) => {
                    match redis_ops::get_pending_delta(&state.redis, &document_id, seq).await {
                        Ok(b) => b,
                        Err(e) => {
                            tracing::error!(seq, error = %e, "replay: pending lookup failed");
                            None
                        }
                    }
                }
                Err(e) => {
                    tracing::error!(seq, error = %e, "replay: S3 fetch failed");
                    None
                }
            };
            let Some(wire) = wire else {
                tracing::error!(seq, s3_key = %row.s3_key, "replay: delta not found in S3 or pending");
                return None;
            };
            match postcard::from_bytes::<ambre_protocol::Update>(&wire) {
                Ok(update) => Some(ambre_protocol::SeqUpdate {
                    seq: ambre_protocol::messages::Seq(seq),
                    update,
                }),
                Err(e) => {
                    tracing::error!(seq, error = %e, "replay: Update decode failed");
                    None
                }
            }
        }
    });

    let mut seq_updates: Vec<ambre_protocol::SeqUpdate> = futures_util::future::join_all(fetches)
        .await
        .into_iter()
        .flatten()
        .collect();
    seq_updates.sort_by_key(|u| u.seq.0);

    let msg = encode_server_msg(&WsServerMsg::MissingUpdates {
        document_id,
        updates: seq_updates,
    });
    let _ = outbound_tx.send(msg).await;
}

pub(crate) async fn handle_subscribe_channel(
    _state: &Arc<AppState>,
    conn: &mut WsConnectionState,
    channel_id: ChannelId,
    outbound_tx: &mpsc::Sender<Vec<u8>>,
) {
    conn.channel_subscriptions.insert(channel_id);
    let ack = encode_server_msg(&WsServerMsg::ChannelSubscribed { channel_id });
    let _ = outbound_tx.send(ack).await;
}

pub(crate) async fn handle_channel_update(
    state: &Arc<AppState>,
    channel_id: ChannelId,
    update: ambre_protocol::Update,
) {
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
    let broadcast = encode_server_msg(&WsServerMsg::ChannelEphemeral {
        channel_id,
        ephemeral,
    });
    let _ = redis_ops::chan_broadcast(&state.redis, &channel_id, &broadcast).await;
}

// -- ACL ----------------------------------------------------------

/// Verifies the caller has a membership row for the document.
pub(crate) async fn verify_document_access(
    pool: &sqlx::PgPool,
    document_id: DocumentId,
    user_id: UserId,
) -> Result<(), AppError> {
    db::get_document_member(pool, &document_id, &user_id)
        .await?
        .ok_or(AppError::Forbidden)?;
    Ok(())
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
    use ed25519_dalek::{Signature as DalekSig, VerifyingKey};

    let public_data_bytes = postcard::to_allocvec(&update.public_data).map_err(AppError::Decode)?;

    let mut signed_data = Vec::new();
    push_framed(&mut signed_data, &update.nonce.0);
    push_framed(&mut signed_data, &update.ciphertext.0);
    push_framed(&mut signed_data, &public_data_bytes);
    push_framed(&mut signed_data, &update.commitment_tag.0);

    let vk = VerifyingKey::from_bytes(pubkey.as_array()).map_err(|_| AppError::InvalidSignature)?;
    let sig = DalekSig::from_slice(&update.signature.0).map_err(|_| AppError::InvalidSignature)?;

    // Strict verification (canonical S, no small-order A) so a relayed signature
    // cannot be malleated into a second valid signature over the same update.
    vk.verify_strict(&signed_data, &sig)
        .map_err(|_| AppError::InvalidSignature)?;

    Ok(())
}

#[cfg(test)]
mod postcard_tests {
    use ambre_core::{DocumentId, PublicKey, SnapshotId, UserId};
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
            revoked_user_id: UserId::from_bytes([8u8; 16]),
        };
        let decoded = decode_server_msg(&encode_server_msg(&msg)).unwrap();
        assert!(matches!(decoded, WsServerMsg::KeyRotationRequired { .. }));
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
}
