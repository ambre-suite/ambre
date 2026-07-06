use super::*;

impl SyncState {
    // -- Causal quarantine (ambre-spec-sync.md §E) -----------------------------

    /// Ingest one server-sequenced [`Update`]. If its parent snapshot is known,
    /// it is decrypted and delivered immediately; otherwise it is held in
    /// quarantine until the parent snapshot arrives (then released in `clock`
    /// order by [`release_quarantine`]).
    #[allow(dead_code)]
    pub(crate) fn receive_update(&mut self, seq: Seq, update: Update) {
        let document_id = update.public_data.document_id;
        if let Some(opened) = self.ingest_update(seq, update) {
            self.deliver_remote(document_id, Some(opened), |payload| {
                LogicEvent::RemoteUpdate {
                    document_id,
                    payload,
                }
            });
        }
    }

    /// Bookkeeping + decrypt for one server-sequenced [`Update`], WITHOUT
    /// delivering it to Logic. Returns the decrypted result when the update is
    /// ready (its parent snapshot is known), or `None` when it was held in
    /// quarantine (or the doc is not open). Split out of [`receive_update`] so a
    /// batched replay ([`WsServerMsg::MissingUpdates`]) can collect many decrypted
    /// payloads and deliver them as one [`LogicEvent::RemoteUpdateBatch`].
    pub(crate) fn ingest_update(
        &mut self,
        seq: Seq,
        update: Update,
    ) -> Option<Result<Vec<u8>, pipeline::PipelineError>> {
        let document_id = update.public_data.document_id;
        // The per-author clock counter is not persisted across a reload; after F5
        // it restarts at 0 and would re-emit a clock the server already consumed
        // (CLOCK_MISMATCH). The server echoes our own updates back to us, so we
        // treat every update authored by THIS client as a high-water-mark and
        // advance `local_clock` past it -- the next `seal_update` then continues
        // from the right value.
        let authored_by_us = self.signing_key.as_ref().is_some_and(|sk| {
            sk.verifying_key().to_bytes() == *update.public_data.pub_key.as_array()
        });
        let Some(doc_state) = self.documents.get_mut(&document_id) else {
            tracing::warn!("Sync: Update for unopened doc {document_id:?}, dropping");
            return None;
        };
        doc_state.last_seq = seq;
        if authored_by_us {
            doc_state.local_clock = doc_state.local_clock.max(update.public_data.clock.0);
        }

        let snapshot_id = update.public_data.snapshot_id;
        if doc_state.known_snapshots.contains(&snapshot_id) {
            let opened = doc_state.open_update_fallback(&update);
            tracing::info!(
                "[Sync→Logic] decrypted update doc={document_id} seq={} clock={} size={}",
                seq.0,
                update.public_data.clock.0,
                opened.as_ref().map(|b| b.len()).unwrap_or(0),
            );
            return Some(opened);
        }

        doc_state
            .quarantine
            .entry(snapshot_id)
            .or_default()
            .push(update);
        tracing::debug!(
            "[Sync] quarantined update doc={document_id} seq={} snapshot={snapshot_id:?}",
            seq.0,
        );
        self.enforce_quarantine_bounds(document_id);
        None
    }

    /// Keep a document's quarantine within [`QUARANTINE_LIMIT`]: request the
    /// missing history at the soft threshold, and drop the oldest update (by
    /// `clock`) past the hard cap, raising a `SyncWarning`.
    pub(crate) fn enforce_quarantine_bounds(&mut self, document_id: DocumentId) {
        let Some(doc_state) = self.documents.get_mut(&document_id) else {
            return;
        };

        if doc_state.quarantine_len() >= QUARANTINE_ALERT {
            let after_seq = doc_state.last_seq;
            if self.ws_connected() {
                self.ws_send(&WsClientMsg::RequestMissing {
                    document_id,
                    after_seq,
                });
            }
        }

        let doc_state = self.documents.get_mut(&document_id).expect("doc present");
        if doc_state.quarantine_len() >= QUARANTINE_LIMIT {
            // Drop the globally oldest quarantined update (smallest clock).
            let oldest = doc_state
                .quarantine
                .iter()
                .filter_map(|(snap, updates)| {
                    updates
                        .iter()
                        .enumerate()
                        .min_by_key(|(_, u)| u.public_data.clock.0)
                        .map(|(idx, u)| (*snap, idx, u.public_data.clock.0))
                })
                .min_by_key(|&(_, _, clock)| clock);

            if let Some((snap, idx, clock)) = oldest {
                if let Some(updates) = doc_state.quarantine.get_mut(&snap) {
                    updates.remove(idx);
                    if updates.is_empty() {
                        doc_state.quarantine.remove(&snap);
                    }
                }
                self.send_to_logic(LogicEvent::SyncWarning {
                    document_id,
                    code: SyncWarningCode::QuarantineOverflow,
                    message: format!("quarantine overflow, dropped oldest update (clock {clock})"),
                });
            }
        }
    }

    /// Mark `snapshot_id` as known for `document_id` and release any updates that
    /// were quarantined waiting for it, in ascending `clock` order.
    pub(crate) fn snapshot_known(&mut self, document_id: DocumentId, snapshot_id: SnapshotId) {
        let Some(doc_state) = self.documents.get_mut(&document_id) else {
            tracing::warn!("Sync: SnapshotAvailable for unopened doc {document_id:?}");
            return;
        };
        doc_state.known_snapshots.insert(snapshot_id);
        doc_state.current_snapshot_id = snapshot_id;
        self.release_quarantine(document_id, snapshot_id);
    }

    /// Decrypt and deliver every update quarantined under `snapshot_id`, oldest
    /// `clock` first.
    pub(crate) fn release_quarantine(&mut self, document_id: DocumentId, snapshot_id: SnapshotId) {
        let mut pending = {
            let Some(doc_state) = self.documents.get_mut(&document_id) else {
                return;
            };
            doc_state
                .quarantine
                .remove(&snapshot_id)
                .unwrap_or_default()
        };
        pending.sort_by_key(|u| u.public_data.clock.0);

        let results: Vec<Result<Vec<u8>, pipeline::PipelineError>> = {
            let Some(doc_state) = self.documents.get(&document_id) else {
                return;
            };
            pending
                .iter()
                .map(|u| doc_state.open_update_fallback(u))
                .collect()
        };
        for opened in results {
            self.deliver_remote(document_id, Some(opened), |payload| {
                LogicEvent::RemoteUpdate {
                    document_id,
                    payload,
                }
            });
        }
    }

    // -- Document lifecycle ----------------------------------------------------

    /// Replay every open that arrived before the session was ready. Called once
    /// the session keys land (`SessionReady` / `AccountCreated`), after the WS is
    /// (re)opened, so each queued document fetches its real key and hydrates.
    pub(crate) fn flush_pending_opens(&mut self) {
        for (document_id, doc_type) in std::mem::take(&mut self.pending_opens) {
            self.handle_open_document(document_id, doc_type);
        }
    }

    pub(crate) fn handle_open_document(&mut self, document_id: DocumentId, doc_type: DocType) {
        // Without session keys we cannot authenticate the members fetch or unwrap
        // the document key: opening now would insert a local-only doc with a
        // throwaway key and never hydrate. Queue it until the session is ready.
        // (Only in the real wasm path; the mock/host path has `event_tx == None`
        // and seeds keys synchronously.)
        if self.event_tx.is_some() && self.signing_key.is_none() {
            tracing::info!("Sync: queuing OpenDocument {document_id:?} until session ready");
            self.pending_opens.push((document_id, doc_type));
            return;
        }

        let event_tx = self.event_tx.clone();

        if let Some(tx) = event_tx {
            let api_base = self.api_base.clone();
            let url_scheme = self.url_scheme;
            let doc_id = document_id;
            let exchange_key = self.exchange_key.clone();
            let auth = self.auth_context();
            wasm_bindgen_futures::spawn_local(async move {
                let members_url =
                    format!("{url_scheme}://{api_base}/api/documents/{doc_id}/members");
                // 404 -> the doc does not exist server-side yet (local-only):
                // generate a fresh key and defer creation to the first update.
                let (doc_key, server_created) =
                    match http::get_json_opt_auth::<http::MembersResponse>(
                        &members_url,
                        auth.clone(),
                    )
                    .await
                    {
                        Ok(Some(resp)) => {
                            // The document exists server-side: our wrapped
                            // DocumentKey MUST unwrap cleanly. A failure here is a
                            // genuine auth error (wrong exchange key, tampered row,
                            // or a real attack) -- never substitute a random key,
                            // which would "open" the document and then fail every
                            // subsequent decrypt with a confusing commitment
                            // mismatch instead of a clear auth error (H3).
                            match &exchange_key {
                                Some(ek) => match Self::unwrap_document_key(ek, &resp.members) {
                                    Ok(key) => (key, true),
                                    Err(_) => {
                                        let _ = tx.send(SyncEvent::OpenDocumentFailed {
                                            document_id: doc_id,
                                            message: "cannot unwrap document key".into(),
                                        });
                                        return;
                                    }
                                },
                                None => {
                                    let _ = tx.send(SyncEvent::OpenDocumentFailed {
                                        document_id: doc_id,
                                        message: "session not ready".into(),
                                    });
                                    return;
                                }
                            }
                        }
                        // Not found, or a transient error: treat as local-only.
                        // A transient error simply means no remote history this
                        // open; the doc still works locally and reconciles later.
                        Ok(None) | Err(_) => (DocumentKey::generate(), false),
                    };
                // Download the latest snapshot (when one exists) so the open
                // seeds from compacted state instead of replaying every delta
                // since genesis. Two server round-trips: `GET /snapshot` for the
                // latest id + key + covered seq (404 when never snapshotted), then
                // `POST /snapshot/download-url` for a presigned S3 GET. Returns the
                // sealed blob, the real snapshot id (to register as a known parent)
                // and the covered seq (to gate the post-open `RequestMissing`).
                let (snapshot_payload, downloaded_snapshot_id, covered_seq) = if server_created {
                    let meta_url =
                        format!("{url_scheme}://{api_base}/api/documents/{doc_id}/snapshot");
                    match http::get_json_opt_auth::<http::SnapshotMetadataResponse>(
                        &meta_url,
                        auth.clone(),
                    )
                    .await
                    {
                        Ok(Some(meta)) => {
                            let snap_id = SnapshotId::parse_str(&meta.snapshot_id).ok();
                            let dl_url = format!(
                                "{url_scheme}://{api_base}/api/documents/{doc_id}/snapshot/download-url"
                            );
                            let body = http::SnapshotDownloadUrlRequest {
                                snapshot_id: meta.snapshot_id,
                            };
                            let payload = match http::post_json_auth::<
                                _,
                                http::SnapshotDownloadUrlResponse,
                            >(&dl_url, &body, auth)
                            .await
                            {
                                Ok(resp) => http::fetch_bytes(&resp.download_url).await.ok(),
                                Err(_) => None,
                            };
                            // Only gate on the covered seq if the blob actually
                            // downloaded; otherwise a failed fetch would skip
                            // history that was never applied (data loss).
                            match payload {
                                Some(_) => (payload, snap_id, meta.covered_seq),
                                None => (None, None, 0),
                            }
                        }
                        // No snapshot yet, or a transient error: fall back to the
                        // full delta replay (RequestMissing from seq 0) on open.
                        Ok(None) | Err(_) => (None, None, 0),
                    }
                } else {
                    (None, None, 0)
                };
                // Register the downloaded snapshot's real id as the known parent so
                // post-snapshot deltas are not quarantined. Without a download, fall
                // back to the deterministic genesis id (deltas authored before any
                // snapshot upload are all parented on it). Genesis stays seeded in
                // `SyncDocumentState::new` regardless, covering mixed histories.
                let snapshot_id =
                    downloaded_snapshot_id.unwrap_or_else(|| SnapshotId::genesis(doc_id));
                let _ = tx.send(SyncEvent::OpenDocumentReady {
                    document_id: doc_id,
                    doc_key,
                    snapshot_payload,
                    snapshot_id,
                    covered_seq: Seq(covered_seq as u64),
                    doc_type,
                    server_created,
                });
            });
        } else {
            let doc_key = DocumentKey::generate();
            self.documents.insert(
                document_id,
                SyncDocumentState::new(document_id, doc_key, doc_type, false),
            );
            if self.ws_connected() {
                self.ws_send(&WsClientMsg::Subscribe { document_id });
            }
            tracing::info!("Sync: OpenDocument {document_id:?}");
        }
    }

    /// Decrypts the DocumentKey from the server's member list using X25519 ECDH.
    ///
    /// 1. Computes the local exchange public key from `exchange_key`.
    /// 2. Finds our own [`MemberEntry`] by matching `exchange_pub_key`.
    /// 3. Hex-decodes the `doc_key_enc` blob (24-byte nonce + AEAD ciphertext
    ///    with Poly1305 tag).
    /// 4. Calls [`ambre_crypto::ecdh::unwrap_document_key`] to perform the
    ///    ECDH shared-secret derivation, KEK derivation, and AEAD decryption.
    pub(crate) fn unwrap_document_key(
        exchange_key: &x25519_dalek::StaticSecret,
        members: &[http::MemberEntry],
    ) -> Result<DocumentKey, String> {
        let our_exchange_pub = {
            let pk = x25519_dalek::PublicKey::from(exchange_key);
            hex::encode(pk.as_bytes())
        };

        let entry = members
            .iter()
            .find(|m| m.exchange_pub_key == our_exchange_pub)
            .ok_or("own member entry not found")?;

        let encrypted = base64::engine::general_purpose::URL_SAFE
            .decode(&entry.doc_key_enc)
            .map_err(|e| format!("base64 decode doc_key_enc: {e}"))?;
        if encrypted.len() < 24 {
            return Err("wrapped key too short".into());
        }
        let nonce: [u8; 24] = encrypted[..24]
            .try_into()
            .map_err(|_| "invalid nonce length")?;
        let ciphertext = encrypted[24..].to_vec();

        // The DH peer is whoever WRAPPED our key (the inviter, or ourselves at
        // creation) -- not our own key. Fall back to the member's own key if an
        // older server omits the wrapper field (e.g. a self-wrapped owner row).
        let wrapper_pub_hex = if entry.wrapper_exchange_pub_key.is_empty() {
            &entry.exchange_pub_key
        } else {
            &entry.wrapper_exchange_pub_key
        };
        let peer_pub_bytes: [u8; 32] = hex::decode(wrapper_pub_hex)
            .map_err(|e| format!("hex decode wrapper_exchange_pub_key: {e}"))?
            .try_into()
            .map_err(|_| "invalid exchange pubkey length")?;

        let wrapped = ambre_crypto::WrappedKey { nonce, ciphertext };
        ambre_crypto::ecdh::unwrap_document_key(exchange_key, &peer_pub_bytes, &wrapped)
            .map_err(|e| format!("ECDH unwrap: {e}"))
    }

    pub(crate) fn handle_close_document(&mut self, document_id: DocumentId) {
        if self.ws_connected() {
            self.ws_send(&WsClientMsg::Unsubscribe { document_id });
        }
        self.documents.remove(&document_id);
        tracing::info!("Sync: CloseDocument {document_id:?}");
    }

    pub(crate) fn handle_reconnect(&mut self, document_id: DocumentId) {
        if self.ws_connected() {
            let last_seq = self
                .documents
                .get(&document_id)
                .map(|d| d.last_seq)
                .unwrap_or(Seq(0));
            self.ws_send(&WsClientMsg::Subscribe { document_id });
            self.ws_send(&WsClientMsg::RequestMissing {
                document_id,
                after_seq: last_seq,
            });
        }
    }

    // -- Outbound sync ---------------------------------------------------------

    /// The author identity (Ed25519 verifying key) for outbound messages, or
    /// `None` before Unlock has seeded the session keys.
    pub(crate) fn author_pub_key(&self) -> Option<PublicKey> {
        self.signing_key
            .as_ref()
            .map(|sk| PublicKey::from_array(sk.verifying_key().to_bytes()))
    }

    /// Replays updates that were buffered before the session/document were ready
    /// (see [`pending_pre_session_updates`](Self::pending_pre_session_updates)).
    /// Drains only the entries for `document_id`, preserving their order.
    pub(crate) fn flush_pre_session_updates(&mut self, document_id: DocumentId) {
        if self.signing_key.is_none() || self.pending_pre_session_updates.is_empty() {
            return;
        }
        let mut drained = Vec::new();
        self.pending_pre_session_updates.retain(|(doc, payload)| {
            if *doc == document_id {
                drained.push(payload.clone());
                false
            } else {
                true
            }
        });
        if !drained.is_empty() {
            tracing::info!(
                "[Sync] flushing {} buffered pre-session update(s) doc={document_id}",
                drained.len()
            );
        }
        for payload in drained {
            self.handle_send_update(document_id, payload);
        }
    }

    pub(crate) fn handle_send_update(&mut self, document_id: DocumentId, payload: Vec<u8>) {
        let Some(signing_key) = self.signing_key.clone() else {
            // Session not established yet (e.g. an edit landed in the gap between
            // an F5 and DevLogin completing). Buffer instead of dropping; flushed
            // once the document opens (see `pending_pre_session_updates`).
            tracing::info!("[Sync] SendUpdate before session ready, buffering doc={document_id}");
            self.pending_pre_session_updates
                .push((document_id, payload));
            return;
        };
        let Some(pub_key) = self.author_pub_key() else {
            return;
        };
        // Lazy server creation: a local-only doc materializes on its first update.
        // Buffer the payload and fire the create POST once; the buffer is flushed
        // in order from the `DocumentMaterialized` handler.
        if self
            .documents
            .get(&document_id)
            .is_some_and(|d| !d.server_created)
        {
            if let Some(doc_state) = self.documents.get_mut(&document_id) {
                doc_state.pending_updates.push(payload);
            }
            self.trigger_lazy_create(document_id);
            return;
        }
        let Some(doc_state) = self.documents.get_mut(&document_id) else {
            tracing::warn!("[Sync] SendUpdate for unknown doc {document_id:?}");
            return;
        };
        doc_state.local_clock += 1;
        let public_data = UpdatePublicData {
            document_id,
            pub_key,
            snapshot_id: doc_state.current_snapshot_id,
            clock: Clock(doc_state.local_clock),
        };
        match pipeline::seal_update(&payload, public_data, &doc_state.doc_key, &signing_key) {
            Ok(update) => {
                tracing::info!(
                    "[Sync] seal_update ok doc={document_id} clock={} → {} bytes ciphertext",
                    doc_state.local_clock,
                    update.ciphertext.0.len(),
                );
                self.ws_send(&WsClientMsg::Update(update));
            }
            Err(e) => tracing::error!("[Sync] SendUpdate seal failed for {document_id:?}: {e}"),
        }
    }

    /// Lazily materialize a local-only document server-side (first-modification
    /// path): wrap its DocumentKey for the owner and POST `/api/documents` with
    /// the client-generated id. Idempotent server-side and guarded by the
    /// per-doc `creating` flag, so concurrent updates issue exactly one POST.
    /// On success a [`SyncEvent::DocumentMaterialized`] flushes the buffer.
    pub(crate) fn trigger_lazy_create(&mut self, document_id: DocumentId) {
        let Some(doc_state) = self.documents.get_mut(&document_id) else {
            return;
        };
        if doc_state.creating {
            return;
        }
        let doc_type = doc_state.doc_type;
        let doc_key = doc_state.doc_key.clone();
        let Some(tx) = self.event_tx.clone() else {
            return;
        };
        let Some(exchange_key) = self.exchange_key.clone() else {
            tracing::warn!("[Sync] lazy create for {document_id:?} without exchange key");
            return;
        };
        doc_state.creating = true;
        let api_base = self.api_base.clone();
        let url_scheme = self.url_scheme;
        let auth = self.auth_context();
        wasm_bindgen_futures::spawn_local(async move {
            let own_pub = x25519_dalek::PublicKey::from(&exchange_key);
            let wrapped =
                ambre_crypto::ecdh::wrap_document_key(&exchange_key, own_pub.as_bytes(), &doc_key);
            let mut enc = wrapped.nonce.to_vec();
            enc.extend_from_slice(&wrapped.ciphertext);
            let doc_key_enc = base64::engine::general_purpose::URL_SAFE.encode(&enc);

            let url = format!("{url_scheme}://{api_base}/api/documents");
            let body = http::CreateDocumentRequest {
                doc_type: doc_type_to_api_string(doc_type),
                name_enc: None,
                doc_key_enc,
                document_id: Some(document_id.to_string()),
            };
            match http::post_json_auth::<_, http::CreateDocumentResponse>(&url, &body, auth).await {
                Ok(_) => {
                    let _ = tx.send(SyncEvent::DocumentMaterialized { document_id });
                }
                Err(e) => {
                    let _ = tx.send(SyncEvent::DocumentCreateFailed {
                        document_id,
                        message: e,
                    });
                }
            }
        });
    }

    pub(crate) fn handle_send_snapshot(&mut self, document_id: DocumentId, payload: Vec<u8>) {
        let Some(signing_key) = self.signing_key.clone() else {
            tracing::warn!("Sync: SendSnapshot before session keys ready, dropping");
            return;
        };
        let Some(pub_key) = self.author_pub_key() else {
            return;
        };
        let author_pubkey = hex::encode(pub_key.as_array());
        let Some(doc_state) = self.documents.get(&document_id) else {
            tracing::warn!("Sync: SendSnapshot for unknown doc {document_id:?}");
            return;
        };
        let public_data = SnapshotPublicData {
            document_id,
            pub_key,
            snapshot_id: doc_state.current_snapshot_id,
            // The BLAKE2b parent chain is a post-MVP hardening (see
            // ambre-spec-sync.md): an empty proof seals a standalone snapshot,
            // which the compaction model accepts for the genesis-parented MVP.
            parent_snapshot_proof: Vec::new(),
            parent_snapshot_update_clocks: Vec::new(),
        };
        let snapshot = match pipeline::seal_snapshot(
            &payload,
            public_data,
            &doc_state.doc_key,
            &signing_key,
        ) {
            Ok(s) => s,
            Err(e) => {
                tracing::error!("Sync: SendSnapshot seal failed for {document_id:?}: {e}");
                return;
            }
        };

        // Off the wasm path (host tests / mock relay) there is no event channel
        // and no real S3: sealing is the observable behavior, stop here.
        let Some(tx) = self.event_tx.clone() else {
            tracing::info!(
                "Sync: SendSnapshot {document_id:?} sealed ({} bytes), no event channel (test path)",
                snapshot.ciphertext.0.len()
            );
            return;
        };

        // Serialize the full sealed `Snapshot` as the S3 blob (the inverse of the
        // open-path `postcard::from_bytes::<Snapshot>` + `open_snapshot`).
        let blob = match postcard::to_allocvec(&snapshot) {
            Ok(b) => b,
            Err(e) => {
                tracing::error!("Sync: SendSnapshot serialize failed for {document_id:?}: {e}");
                return;
            }
        };
        // The snapshot folds in every delta this client had received, i.e. up to
        // its high-water `last_seq`. The server stores it so other clients can
        // gate their post-open replay and skip this history.
        let covered_seq = doc_state.last_seq.0 as i64;
        let api_base = self.api_base.clone();
        let url_scheme = self.url_scheme;
        let auth = self.auth_context();

        // upload-url -> S3 PUT -> confirm metadata, on the media-upload template.
        wasm_bindgen_futures::spawn_local(async move {
            let upload_url_req = format!(
                "{url_scheme}://{api_base}/api/documents/{document_id}/snapshot/upload-url"
            );
            let (upload_url, s3_key) = match http::post_json_auth::<
                serde_json::Value,
                http::SnapshotUploadUrlResponse,
            >(
                &upload_url_req, &serde_json::json!({}), auth.clone()
            )
            .await
            {
                Ok(resp) => (resp.upload_url, resp.s3_key),
                Err(e) => {
                    tracing::error!(
                        "Sync: SendSnapshot upload-url failed for {document_id:?}: {e}"
                    );
                    return;
                }
            };

            // Presigned PUT goes straight to S3 (no session, no CSRF header).
            if let Err(e) = http::put_bytes(&upload_url, &blob, "application/octet-stream").await {
                tracing::error!("Sync: SendSnapshot S3 PUT failed for {document_id:?}: {e}");
                return;
            }

            let confirm_url =
                format!("{url_scheme}://{api_base}/api/documents/{document_id}/snapshot");
            let confirm_body = http::CreateSnapshotRequest {
                snapshot_id: SnapshotId::new().to_string(),
                author_pubkey,
                parent_hash: None,
                parent_snapshot_proof: None,
                update_clocks: serde_json::json!({}),
                s3_key,
                covered_seq,
            };
            let new_snapshot_id = match http::post_json_auth::<_, http::CreateSnapshotResponse>(
                &confirm_url,
                &confirm_body,
                auth,
            )
            .await
            {
                Ok(resp) => match SnapshotId::parse_str(&resp.snapshot_id) {
                    Ok(id) => id,
                    Err(e) => {
                        tracing::error!(
                            "Sync: SendSnapshot bad snapshot_id for {document_id:?}: {e}"
                        );
                        return;
                    }
                },
                Err(e) => {
                    tracing::error!("Sync: SendSnapshot confirm failed for {document_id:?}: {e}");
                    return;
                }
            };

            tracing::info!(
                "Sync: SendSnapshot {document_id:?} uploaded ({} bytes), snapshot_id={new_snapshot_id:?}",
                blob.len()
            );
            let _ = tx.send(SyncEvent::SnapshotUploaded {
                document_id,
                snapshot_id: new_snapshot_id,
            });
        });
    }

    pub(crate) fn handle_send_ephemeral(&mut self, document_id: DocumentId, payload: Vec<u8>) {
        let Some(signing_key) = self.signing_key.clone() else {
            tracing::warn!("Sync: SendEphemeral before session keys ready, dropping");
            return;
        };
        let Some(pub_key) = self.author_pub_key() else {
            return;
        };
        let Some(doc_state) = self.documents.get_mut(&document_id) else {
            tracing::warn!("Sync: SendEphemeral for unknown doc {document_id:?}");
            return;
        };
        doc_state.ephemeral_counter += 1;
        let public_data = EphemeralPublicData {
            document_id,
            pub_key,
            counter: Counter(doc_state.ephemeral_counter),
        };
        match pipeline::seal_ephemeral(&payload, public_data, &doc_state.doc_key, &signing_key) {
            Ok(ephemeral) => self.ws_send(&WsClientMsg::Ephemeral(ephemeral)),
            Err(e) => tracing::error!("Sync: SendEphemeral seal failed for {document_id:?}: {e}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A `SyncState` with one opened document, a controlled `doc_key` and a known
    /// signing key, plus the drainable Logic receiver and the document's known
    /// snapshot id.
    fn setup() -> (
        SyncState,
        flume::Receiver<LogicEvent>,
        DocumentId,
        DocumentKey,
        SigningKey,
        SnapshotId,
    ) {
        let (sink, rx) = LogicSink::channel();
        let mut state = SyncState::new(sink, String::new(), None);
        let document_id = DocumentId::new();
        let doc_key = DocumentKey::generate();
        let signing_key = MasterSeed::generate().derive_signing_key();
        let doc_state = SyncDocumentState::new(document_id, doc_key.clone(), DocType::Trame, true);
        let known_snapshot = doc_state.current_snapshot_id;
        state.documents.insert(document_id, doc_state);
        (state, rx, document_id, doc_key, signing_key, known_snapshot)
    }

    fn make_update(
        document_id: DocumentId,
        snapshot_id: SnapshotId,
        clock: u64,
        payload: &[u8],
        doc_key: &DocumentKey,
        signing_key: &SigningKey,
    ) -> Update {
        let pub_key = PublicKey::from_array(signing_key.verifying_key().to_bytes());
        let public_data = UpdatePublicData {
            document_id,
            pub_key,
            snapshot_id,
            clock: Clock(clock),
        };
        pipeline::seal_update(payload, public_data, doc_key, signing_key).expect("seal")
    }

    fn drain(rx: &flume::Receiver<LogicEvent>) -> Vec<LogicEvent> {
        rx.try_iter().collect()
    }

    #[test]
    fn update_with_known_snapshot_delivered_immediately() {
        let (mut state, rx, doc, key, sk, snap) = setup();
        let update = make_update(doc, snap, 1, b"hello", &key, &sk);

        state.receive_update(Seq(1), update);

        let events = drain(&rx);
        assert_eq!(events.len(), 1);
        match &events[0] {
            LogicEvent::RemoteUpdate {
                document_id,
                payload,
            } => {
                assert_eq!(*document_id, doc);
                assert_eq!(payload, b"hello");
            }
            other => panic!("expected RemoteUpdate, got {other:?}"),
        }
        assert_eq!(state.documents[&doc].quarantine_len(), 0);
    }

    #[test]
    fn update_with_unknown_snapshot_quarantined() {
        let (mut state, rx, doc, key, sk, _snap) = setup();
        let unknown = SnapshotId::new();
        let update = make_update(doc, unknown, 1, b"later", &key, &sk);

        state.receive_update(Seq(1), update);

        assert!(drain(&rx).is_empty(), "nothing should be delivered yet");
        assert_eq!(state.documents[&doc].quarantine_len(), 1);
    }

    #[test]
    fn quarantine_released_in_clock_order() {
        let (mut state, rx, doc, key, sk, _snap) = setup();
        let parent = SnapshotId::new();
        // Arrive out of clock order: clock 5 then clock 2.
        state.receive_update(Seq(1), make_update(doc, parent, 5, b"five", &key, &sk));
        state.receive_update(Seq(2), make_update(doc, parent, 2, b"two", &key, &sk));
        assert!(drain(&rx).is_empty());
        assert_eq!(state.documents[&doc].quarantine_len(), 2);

        state.snapshot_known(doc, parent);

        let payloads: Vec<Vec<u8>> = drain(&rx)
            .into_iter()
            .map(|e| match e {
                LogicEvent::RemoteUpdate { payload, .. } => payload,
                other => panic!("expected RemoteUpdate, got {other:?}"),
            })
            .collect();
        assert_eq!(payloads, vec![b"two".to_vec(), b"five".to_vec()]);
        assert_eq!(state.documents[&doc].quarantine_len(), 0);
    }

    #[test]
    fn quarantine_overflow_drops_oldest_and_warns() {
        let (mut state, rx, doc, key, sk, _snap) = setup();
        let parent = SnapshotId::new();
        for clock in 1..=(QUARANTINE_LIMIT as u64) {
            state.receive_update(Seq(clock), make_update(doc, parent, clock, b"x", &key, &sk));
        }

        let warnings = drain(&rx)
            .into_iter()
            .filter(|e| matches!(e, LogicEvent::SyncWarning { code, .. } if *code == SyncWarningCode::QuarantineOverflow))
            .count();
        assert_eq!(warnings, 1, "exactly one overflow warning at the hard cap");
        assert_eq!(state.documents[&doc].quarantine_len(), QUARANTINE_LIMIT - 1);
    }

    // ── Snapshot lifecycle (upload adoption + cold-start seq gating) ──────────

    /// Drain the WsClientMsg this state sent to its mock WS, decoded.
    fn drain_ws(rx: &flume::Receiver<Vec<u8>>) -> Vec<WsClientMsg> {
        rx.try_iter()
            .filter_map(|wire| ambre_protocol::ws::decode_client_msg(&wire).ok())
            .collect()
    }

    /// A downloaded snapshot covering up to seq N must make the post-open
    /// `RequestMissing` start AFTER N -- replaying only the deltas the snapshot
    /// does not already contain, not the whole history since genesis.
    #[test]
    fn open_document_ready_gates_request_missing_on_covered_seq() {
        let (mut state, _rx, doc, key, _sk, _snap) = setup();
        let (ws_tx, ws_rx) = flume::unbounded();
        state.set_mock_ws(ws_tx);
        let snapshot_id = SnapshotId::new();

        state.handle_sync_event(SyncEvent::OpenDocumentReady {
            document_id: doc,
            doc_key: key,
            snapshot_payload: Some(b"snap-blob".to_vec()),
            snapshot_id,
            covered_seq: Seq(1500),
            doc_type: DocType::Trame,
            server_created: true,
        });

        // The replay cursor advanced to the snapshot's covered seq.
        assert_eq!(state.documents[&doc].last_seq, Seq(1500));
        // The downloaded snapshot id is registered as a known parent.
        assert!(state.documents[&doc].known_snapshots.contains(&snapshot_id));

        let msgs = drain_ws(&ws_rx);
        let after = msgs.iter().find_map(|m| match m {
            WsClientMsg::RequestMissing { after_seq, .. } => Some(*after_seq),
            _ => None,
        });
        assert_eq!(
            after,
            Some(Seq(1500)),
            "RequestMissing must skip the history folded into the snapshot"
        );
    }

    /// With no snapshot to download (`covered_seq == 0`), the open still requests
    /// the full history from seq 0 -- the prior, replay-everything behavior.
    #[test]
    fn open_document_ready_without_snapshot_requests_full_history() {
        let (mut state, _rx, doc, key, _sk, _snap) = setup();
        let (ws_tx, ws_rx) = flume::unbounded();
        state.set_mock_ws(ws_tx);

        state.handle_sync_event(SyncEvent::OpenDocumentReady {
            document_id: doc,
            doc_key: key,
            snapshot_payload: None,
            snapshot_id: SnapshotId::genesis(doc),
            covered_seq: Seq(0),
            doc_type: DocType::Trame,
            server_created: true,
        });

        assert_eq!(state.documents[&doc].last_seq, Seq(0));
        let msgs = drain_ws(&ws_rx);
        let after = msgs.iter().find_map(|m| match m {
            WsClientMsg::RequestMissing { after_seq, .. } => Some(*after_seq),
            _ => None,
        });
        assert_eq!(after, Some(Seq(0)));
    }

    /// Count of `DocumentReady` events for `doc` in a drained event list.
    fn ready_count(events: &[LogicEvent], doc: DocumentId) -> usize {
        events
            .iter()
            .filter(
                |e| matches!(e, LogicEvent::DocumentReady { document_id } if *document_id == doc),
            )
            .count()
    }

    /// A server document opened over a live WS is only fully hydrated once its
    /// post-open catch-up replay (`MissingUpdates`) lands -- the snapshot does
    /// not cover deltas committed after it, and a delta-only document has NO
    /// snapshot at all. So `DocumentReady` (which dismisses the UI loading
    /// overlay) must be DEFERRED to that first replay, not fired at open: firing
    /// at open would clear the overlay while the grid is still empty, exactly the
    /// 5s-empty-grid bug. The snapshot is still emitted at open.
    #[test]
    fn open_document_ready_deferred_to_initial_replay() {
        let (mut state, rx, doc, key, _sk, _snap) = setup();
        let (ws_tx, _ws_rx) = flume::unbounded();
        state.set_mock_ws(ws_tx);

        state.handle_sync_event(SyncEvent::OpenDocumentReady {
            document_id: doc,
            doc_key: key,
            snapshot_payload: Some(b"snap-blob".to_vec()),
            snapshot_id: SnapshotId::new(),
            covered_seq: Seq(0),
            doc_type: DocType::Trame,
            server_created: true,
        });
        let at_open = drain(&rx);
        assert!(
            at_open
                .iter()
                .any(|e| matches!(e, LogicEvent::RemoteSnapshot { .. })),
            "snapshot still emitted at open"
        );
        assert_eq!(
            ready_count(&at_open, doc),
            0,
            "DocumentReady must NOT fire at open while the replay is pending"
        );

        // The initial catch-up reply (empty here -- nothing newer than the
        // snapshot) marks hydration complete and releases exactly one
        // DocumentReady.
        state.handle_ws_message(WsServerMsg::MissingUpdates {
            document_id: doc,
            updates: Vec::new(),
        });
        let after_replay = drain(&rx);
        assert_eq!(
            ready_count(&after_replay, doc),
            1,
            "exactly one DocumentReady once the initial replay lands"
        );

        // A later reconnect replay must not fire a duplicate.
        state.handle_ws_message(WsServerMsg::MissingUpdates {
            document_id: doc,
            updates: Vec::new(),
        });
        assert_eq!(
            ready_count(&drain(&rx), doc),
            0,
            "no duplicate DocumentReady on a subsequent replay"
        );
    }

    /// A local-only (not yet server-created) or offline open has no replay to
    /// wait for, so `DocumentReady` fires immediately at open -- otherwise the
    /// overlay would hang forever with no `MissingUpdates` ever coming.
    #[test]
    fn open_document_ready_immediate_without_replay() {
        // Offline open (no WS): server_created but not connected.
        let (mut state, rx, doc, key, _sk, _snap) = setup();
        state.handle_sync_event(SyncEvent::OpenDocumentReady {
            document_id: doc,
            doc_key: key,
            snapshot_payload: None,
            snapshot_id: SnapshotId::genesis(doc),
            covered_seq: Seq(0),
            doc_type: DocType::Trame,
            server_created: true,
        });
        assert_eq!(
            ready_count(&drain(&rx), doc),
            1,
            "offline open signals readiness immediately (no replay to await)"
        );
    }

    /// A completed snapshot upload adopts the server-assigned snapshot id as the
    /// document's current parent and registers it as known (so later deltas
    /// parented on it are delivered, not quarantined).
    #[test]
    fn snapshot_uploaded_adopts_snapshot_id() {
        let (mut state, _rx, doc, _key, _sk, _snap) = setup();
        let new_id = SnapshotId::new();

        state.handle_sync_event(SyncEvent::SnapshotUploaded {
            document_id: doc,
            snapshot_id: new_id,
        });

        assert_eq!(state.documents[&doc].current_snapshot_id, new_id);
        assert!(state.documents[&doc].known_snapshots.contains(&new_id));
    }

    /// On the host (no event channel), `SendSnapshot` seals the payload without
    /// attempting any network upload and leaves the document state untouched.
    #[test]
    fn send_snapshot_on_host_path_seals_without_upload() {
        let (mut state, _rx, doc, _key, _sk, snap) = setup();
        // Session keys are required to reach the seal step.
        state.signing_key = Some(MasterSeed::generate().derive_signing_key());

        state.handle_send_snapshot(doc, b"crdt-snapshot-state".to_vec());

        // No event_tx in this setup: the upload path is skipped, the current
        // snapshot id is unchanged (no SnapshotUploaded round-trip).
        assert_eq!(state.documents[&doc].current_snapshot_id, snap);
    }
}
