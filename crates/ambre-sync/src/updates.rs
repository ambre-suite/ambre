use super::*;
use crate::rekey::SentFrame;
use ambre_protocol::messages::Signature;

impl SyncState {
    // -- Causal quarantine (`ambre-spec-delta-order.md`) ------------

    /// Ingest one server-sequenced [`Update`]. If its parent snapshot is known,
    /// it is decrypted and delivered immediately. Otherwise it is held in
    /// quarantine until the parent snapshot arrives, then released in `clock`
    /// order by [`release_quarantine`].
    #[allow(dead_code)]
    pub(crate) fn receive_update(&mut self, seq: Seq, update: Update) {
        let document_id = update.public_data.document_id;
        if let Some(opened) = self.ingest_update(seq, update) {
            self.deliver_remote(document_id, Some(opened), |delta| {
                LogicEvent::RemoteUpdate { document_id, delta }
            });
        }
    }

    /// Bookkeeping + decrypt for one server-sequenced [`Update`], WITHOUT
    /// delivering it to Logic. Returns the decrypted result when the update is
    /// ready, that is when its parent snapshot is known. `None` when it was held
    /// in quarantine, or the doc is not open. Split out of [`receive_update`] so a
    /// batched replay ([`WsServerMsg::MissingUpdates`]) can collect many decrypted
    /// payloads and deliver them as one [`LogicEvent::RemoteUpdateBatch`].
    pub(crate) fn ingest_update(
        &mut self,
        seq: Seq,
        update: Update,
    ) -> Option<Result<SignedDelta, pipeline::PipelineError>> {
        let document_id = update.public_data.document_id;
        // The per-author clock counter is not persisted across a reload. After F5
        // it restarts at 0 and would re-emit a clock the server already consumed
        // (CLOCK_MISMATCH). The server echoes our own updates back to us. Every
        // update authored by THIS client is treated as a high-water-mark, and
        // `local_clock` is advanced past it. The next `seal_update` then
        // continues from the right value.
        let authored_by_us = self.signing_key.as_ref().is_some_and(|sk| {
            sk.verifying_key().to_bytes() == *update.public_data.pub_key.as_array()
        });
        // An evicted document counts the update and loads nothing
        // (`RESID-014`, `RESID-016`).
        if self.count_light(seq, &update) {
            return None;
        }
        let Some(doc_state) = self.documents.get_mut(&document_id) else {
            tracing::warn!("Sync: Update for unopened doc {document_id:?}, dropping");
            return None;
        };
        if !doc_state.first_arrival(seq) {
            tracing::debug!(
                "[Sync] dropping a second copy of seq={} doc={document_id}",
                seq.0
            );
            return None;
        }
        doc_state.last_seq = seq;
        if authored_by_us {
            doc_state.local_clock = doc_state.local_clock.max(update.public_data.clock.0);
            // The echo is the relay's receipt: that frame and every earlier
            // one need no resend (`SYNC-129`). Another session of this user
            // signs its own deltas, so its echo confirms nothing (`SYNC-148`).
            doc_state.rekey.confirm(&update.signature);
            // A release waiting on this echo lands now (`RESID-009`).
            if doc_state.released.is_some() {
                self.settle_release(document_id);
            }
        }
        // A release that just landed turned the document light.
        let doc_state = self.documents.get_mut(&document_id)?;

        let snapshot_id = update.public_data.snapshot_id;
        if doc_state.known_snapshots.contains(&snapshot_id) {
            let opened = doc_state.open_signed_update(&update, seq);
            tracing::info!(
                "[Sync->Logic] decrypted update doc={document_id} seq={} clock={} size={}",
                seq.0,
                update.public_data.clock.0,
                opened.as_ref().map(|d| d.payload.len()).unwrap_or(0),
            );
            return Some(opened);
        }

        doc_state
            .quarantine
            .entry(snapshot_id)
            .or_default()
            .push((seq, update));
        tracing::debug!(
            "[Sync] quarantined update doc={document_id} seq={} snapshot={snapshot_id:?}",
            seq.0,
        );
        self.enforce_quarantine_bounds(document_id);
        None
    }

    /// Keep a document's quarantine within [`QUARANTINE_LIMIT`]. Request the
    /// missing history at the soft threshold, and drop the oldest update (by
    /// `clock`) past the hard cap, raising a `SyncWarning`.
    pub(crate) fn enforce_quarantine_bounds(&mut self, document_id: DocumentId) {
        let Some(doc_state) = self.documents.get_mut(&document_id) else {
            return;
        };

        let ask = doc_state.quarantine_len() >= QUARANTINE_ALERT && !doc_state.gap_requested;
        let after_seq = doc_state.last_seq;
        if ask && self.ws_connected() {
            if let Some(doc_state) = self.documents.get_mut(&document_id) {
                doc_state.gap_requested = true;
            }
            self.ws_send(&WsClientMsg::RequestMissing {
                document_id,
                after_seq,
            });
        }

        let Some(doc_state) = self.documents.get_mut(&document_id) else {
            return;
        };
        if doc_state.quarantine_len() >= QUARANTINE_LIMIT {
            // Drop the globally oldest quarantined update (smallest clock).
            let oldest = doc_state
                .quarantine
                .iter()
                .filter_map(|(snap, updates)| {
                    updates
                        .iter()
                        .enumerate()
                        .min_by_key(|(_, (_, u))| u.public_data.clock.0)
                        .map(|(idx, (_, u))| (*snap, idx, u.public_data.clock.0))
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

    /// Fetch a snapshot a peer announced, so its contents reach the Logic thread
    /// before it counts as a known parent (`SYNC-030`). The answer comes back as
    /// [`SyncEvent::SnapshotFetched`]. Off the wasm path there is no object
    /// storage: the snapshot stays unknown, and its updates stay quarantined.
    pub(crate) fn fetch_announced_snapshot(
        &mut self,
        document_id: DocumentId,
        snapshot_id: SnapshotId,
        covered_seq: Seq,
    ) {
        let Some(doc_state) = self.documents.get(&document_id) else {
            tracing::warn!("Sync: SnapshotAvailable for unopened doc {document_id:?}");
            return;
        };
        if doc_state.known_snapshots.contains(&snapshot_id) {
            // Our own upload, or one already applied.
            return;
        }
        let Some(tx) = self.event_tx.clone() else {
            tracing::info!(
                "Sync: snapshot {snapshot_id:?} announced, no object storage (test path)"
            );
            return;
        };
        let api_base = self.api_base.clone();
        let url_scheme = self.url_scheme;
        let auth = self.auth_context();
        wasm_bindgen_futures::spawn_local(async move {
            let dl_url = format!(
                "{url_scheme}://{api_base}/api/documents/{document_id}/snapshot/download-url"
            );
            let body = http::SnapshotDownloadUrlRequest {
                snapshot_id: snapshot_id.to_string(),
            };
            let blob = match http::post_json_auth::<_, http::SnapshotDownloadUrlResponse>(
                &dl_url, &body, auth,
            )
            .await
            {
                Ok(resp) => http::fetch_bytes(&resp.download_url).await.ok(),
                Err(_) => None,
            };
            let _ = tx.send(SyncEvent::SnapshotFetched {
                document_id,
                snapshot_id,
                covered_seq,
                blob,
            });
        });
    }

    /// Hand a fetched snapshot to the Logic thread to check, then register it
    /// as known and release what waited on it (`SYNC-030`). The snapshot is
    /// pending: the client holds the deltas it covers, so it checks them
    /// rather than import it (`AUTHOR-016`). Each delta parented on it is
    /// signed, so none waits on the verdict. A snapshot that did not arrive,
    /// or does not open, is reported and stays unknown.
    pub(crate) fn adopt_fetched_snapshot(
        &mut self,
        document_id: DocumentId,
        snapshot_id: SnapshotId,
        covered_seq: Seq,
        blob: Option<Vec<u8>>,
    ) {
        let Some(doc_state) = self.documents.get_mut(&document_id) else {
            return;
        };
        let opened = match blob {
            Some(blob) => doc_state
                .open_signed_snapshot(&blob)
                .map_err(|e| e.to_string()),
            None => Err("the snapshot could not be downloaded".to_owned()),
        };
        match opened {
            Ok((payload, author)) => {
                doc_state.checks.insert(
                    snapshot_id,
                    PendingCheck {
                        author,
                        covered_seq,
                        payload: None,
                    },
                );
                self.send_to_logic(LogicEvent::PendingSnapshot {
                    document_id,
                    snapshot_id,
                    covered_seq: covered_seq.0,
                    payload,
                    purpose: SnapshotPurpose::Confirm,
                });
                self.snapshot_known(document_id, snapshot_id);
            }
            Err(message) => self.send_to_logic(LogicEvent::SyncWarning {
                document_id,
                code: SyncWarningCode::DecryptFailed,
                message,
            }),
        }
    }

    /// Act on the Logic thread's verdict on a snapshot (`AUTHOR-016`,
    /// `AUTHOR-017`, `AUTHOR-022`). Operations only the snapshot holds pass
    /// when their registry entry resolves to its author, and to nobody else
    /// (`AUTHOR-021`). A pending snapshot that passes is confirmed, and one
    /// that contradicts a delta is reported. On open, one that passes is
    /// adopted, and any other leaves the document on its reference.
    ///
    /// Returns what the relay is to be told about a pending snapshot, which
    /// the caller posts with [`send_vouch`].
    pub(crate) fn handle_snapshot_verdict(
        &mut self,
        document_id: DocumentId,
        snapshot_id: SnapshotId,
        purpose: SnapshotPurpose,
        verdict: SnapshotVerdict,
    ) -> Option<Vouch> {
        let doc_state = self.documents.get_mut(&document_id)?;
        let Some(check) = doc_state.checks.remove(&snapshot_id) else {
            tracing::warn!("Sync: a verdict on snapshot {snapshot_id:?} nobody asked for");
            return None;
        };
        let passes = match &verdict {
            SnapshotVerdict::Match { extras } => extras.iter().all(|(peer, bytes)| {
                ambre_core::authorship::PeerEntry::from_bytes(bytes).is_some_and(|entry| {
                    ambre_crypto::peer::entry_resolves(&entry, document_id, *peer, &check.author)
                })
            }),
            SnapshotVerdict::Mismatch | SnapshotVerdict::Unchecked => false,
        };
        let mismatch = verdict == SnapshotVerdict::Mismatch;
        let role = doc_state.role;
        match purpose {
            SnapshotPurpose::Open if passes => {
                if let Some(payload) = check.payload {
                    doc_state.current_snapshot_id = snapshot_id;
                    self.send_to_logic(LogicEvent::RemoteSnapshot {
                        document_id,
                        payload,
                        covered_seq: check.covered_seq.0,
                    });
                }
                // The open waited on this verdict (`check_held_snapshot`).
                self.send_to_logic(LogicEvent::DocumentReady { document_id, role });
                None
            }
            SnapshotPurpose::Open => {
                tracing::warn!(
                    "Sync: snapshot {snapshot_id:?} of doc {document_id:?} refused on open \
                     (mismatch: {mismatch}), staying on its reference"
                );
                self.send_to_logic(LogicEvent::DocumentReady { document_id, role });
                None
            }
            SnapshotPurpose::Confirm if passes => Some(Vouch::Confirm),
            SnapshotPurpose::Confirm if mismatch => {
                let signing_key = self.signing_key.as_ref()?;
                let transcript = ambre_protocol::snapshot_report::report_transcript(
                    document_id,
                    snapshot_id,
                    &check.author,
                );
                let signature = ed25519_dalek::Signer::sign(signing_key, &transcript).to_bytes();
                tracing::warn!(
                    "Sync: snapshot {snapshot_id:?} of doc {document_id:?} contradicts its \
                     deltas, reporting it"
                );
                Some(Vouch::Report(hex::encode(signature)))
            }
            SnapshotPurpose::Confirm => {
                tracing::info!("Sync: snapshot {snapshot_id:?} left unchecked");
                None
            }
        }
    }

    /// Posts `vouch` on a snapshot. Off the wasm path there is no relay to
    /// tell.
    pub(crate) fn post_snapshot_vouch(
        &mut self,
        document_id: DocumentId,
        snapshot_id: SnapshotId,
        vouch: Vouch,
    ) {
        if self.event_tx.is_none() {
            tracing::info!("Sync: snapshot {snapshot_id:?} vouch, no relay (test path)");
            return;
        }
        let base = format!("{}://{}", self.url_scheme, self.api_base);
        let auth = self.auth_context();
        wasm_bindgen_futures::spawn_local(async move {
            let sent =
                send_vouch(&http::Fetch, &auth, &base, document_id, snapshot_id, &vouch).await;
            if let Err(e) = sent {
                tracing::warn!("Sync: snapshot {snapshot_id:?} vouch failed: {e}");
            }
        });
    }

    /// Check the snapshot the open held back, now that every delta the relay
    /// kept was replayed (`AUTHOR-022`). One that does not open is dropped,
    /// and the document stays on its reference.
    ///
    /// `true` when a check is now in flight. The verdict then tells Logic the
    /// document is ready, since a delta past the snapshot waits on it until it
    /// is adopted (`AUTHOR-025`).
    pub(crate) fn check_held_snapshot(&mut self, document_id: DocumentId) -> bool {
        let Some(doc_state) = self.documents.get_mut(&document_id) else {
            return false;
        };
        let Some(held) = doc_state.held_snapshot.take() else {
            return false;
        };
        match doc_state.open_signed_snapshot(&held.blob) {
            Ok((payload, author)) => {
                doc_state.checks.insert(
                    held.snapshot_id,
                    PendingCheck {
                        author,
                        covered_seq: held.covered_seq,
                        payload: Some(payload.clone()),
                    },
                );
                self.send_to_logic(LogicEvent::PendingSnapshot {
                    document_id,
                    snapshot_id: held.snapshot_id,
                    covered_seq: held.covered_seq.0,
                    payload,
                    purpose: SnapshotPurpose::Open,
                });
                true
            }
            Err(e) => {
                tracing::warn!(
                    "Sync: the latest snapshot of doc {document_id:?} does not open ({e}), \
                     staying on its reference"
                );
                false
            }
        }
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
        pending.sort_by_key(|(_, u)| u.public_data.clock.0);
        if let Some(doc_state) = self.documents.get_mut(&document_id)
            && doc_state.quarantine_len() < QUARANTINE_ALERT
        {
            // The gap closed: a later one asks again (`SYNC-031`).
            doc_state.gap_requested = false;
        }

        let results: Vec<Result<SignedDelta, pipeline::PipelineError>> = {
            let Some(doc_state) = self.documents.get(&document_id) else {
                return;
            };
            pending
                .iter()
                .map(|(seq, u)| doc_state.open_signed_update(u, *seq))
                .collect()
        };
        for opened in results {
            self.deliver_remote(document_id, Some(opened), |delta| {
                LogicEvent::RemoteUpdate { document_id, delta }
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
        // the document key. Opening now would insert a local-only doc with a
        // throwaway key and never hydrate. Queue it until the session is ready.
        // (Only in the real wasm path; the mock/host path has `event_tx == None`
        // and seeds keys synchronously.)
        if self.event_tx.is_some() && self.signing_key.is_none() {
            tracing::info!("Sync: queuing OpenDocument {document_id:?} until session ready");
            self.pending_opens.push((document_id, doc_type));
            return;
        }
        self.issue_session_identity(document_id);
        // An evicted document resumes after what its local copy holds
        // (`RESID-010`).
        let resume = self.take_light(document_id);

        let event_tx = self.event_tx.clone();

        if let Some(tx) = event_tx {
            let api_base = self.api_base.clone();
            let url_scheme = self.url_scheme;
            let doc_id = document_id;
            let exchange_key = self.exchange_key.clone();
            let auth = self.auth_context();
            // Needed inside the task to pick our own row out of the members
            // response. `None` only before the session is ready, which the guard
            // above already excluded on the wasm path.
            let my_user_id = self.user_id;
            let self_join = self.self_join_rule(document_id);
            // A snapshot this user confirmed alone is this user's own work, and
            // opens as it is (`AUTHOR-022` reads another user's).
            let own_key = self
                .author_pub_key()
                .map(|key| hex::encode(key.as_array()))
                .unwrap_or_default();
            wasm_bindgen_futures::spawn_local(async move {
                let members_url =
                    format!("{url_scheme}://{api_base}/api/documents/{doc_id}/members");
                // 404 -> the doc does not exist server-side yet (local-only):
                // generate a fresh key and defer creation to the first update.
                let mut signers = None;
                let mut named_key = None;
                let answer = MembersAnswer::classify(
                    http::get_json_opt_auth::<http::MembersResponse>(&members_url, auth.clone())
                        .await,
                );
                let (doc_key, server_created, signing_keys, role, rotation_owed) = match answer {
                    MembersAnswer::Exists(resp) => match &exchange_key {
                        Some(ek) => {
                            match Self::opened_from_members(
                                &resp, doc_id, ek, my_user_id, self_join,
                            ) {
                                Ok(o) => {
                                    signers = o.signers;
                                    named_key = o.named_key;
                                    (o.doc_key, true, o.signing_keys, o.role, o.rotation_owed)
                                }
                                Err(_) => {
                                    let _ = tx.send(SyncEvent::OpenDocumentFailed {
                                        document_id: doc_id,
                                        message: "cannot unwrap document key".into(),
                                    });
                                    return;
                                }
                            }
                        }
                        None => {
                            let _ = tx.send(SyncEvent::OpenDocumentFailed {
                                document_id: doc_id,
                                message: "session not ready".into(),
                            });
                            return;
                        }
                    },
                    // Not found: the document is local-only. No members response
                    // means no directory: a local-only doc has no peers to resolve
                    // presence for anyway, and its creator owns it.
                    MembersAnswer::Absent => (
                        DocumentKey::generate(),
                        false,
                        Vec::new(),
                        Role::Owner,
                        false,
                    ),
                    // A failed request says nothing about the document. Opening it
                    // local-only would mint a throwaway key and edit a copy nobody
                    // receives, so the open fails and the user may retry (`SYNC-040`).
                    MembersAnswer::Unreachable(message) => {
                        let _ = tx.send(SyncEvent::OpenDocumentFailed {
                            document_id: doc_id,
                            message: format!("the server could not be reached: {message}"),
                        });
                        return;
                    }
                };
                // The chain opens what was sealed before the last rotation,
                // the snapshot among it (`SYNC-117`, `SYNC-119`).
                // A link counts only when a writer the log names signed it
                // (`SYNC-121`).
                let links = if server_created {
                    let links =
                        http::fetch_key_links(url_scheme, &api_base, doc_id, auth.clone()).await;
                    crate::chain::signed_by_writers(links, signers.as_ref())
                } else {
                    Vec::new()
                };
                if let Err(message) = Self::named_key_holds(doc_id, &doc_key, named_key, &links) {
                    let _ = tx.send(SyncEvent::OpenDocumentFailed {
                        document_id: doc_id,
                        message,
                    });
                    return;
                }
                // Download the latest snapshot (when one exists) so the open
                // seeds from compacted state instead of replaying every delta
                // since genesis. Two server round-trips: `GET /snapshot` for the
                // latest id + key + covered seq (404 when never snapshotted), then
                // `POST /snapshot/download-url` for a presigned S3 GET. Returns the
                // sealed blob, the real snapshot id (to register as a known parent)
                // and the covered seq (to gate the post-open `RequestMissing`).
                //
                // A snapshot only its author confirmed is not opened from when
                // another user built it. The open starts from the reference, the
                // latest one another member confirmed, and holds the latest one
                // back until the replay is done (`AUTHOR-022`).
                let mut held_snapshot = None;
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
                            let others = meta.status == "self" && meta.author_pubkey != own_key;
                            let (start_id, start_covered) = match (others, &meta.reference) {
                                (false, _) => (Some(meta.snapshot_id.clone()), meta.covered_seq),
                                (true, Some(r)) => (Some(r.snapshot_id.clone()), r.covered_seq),
                                (true, None) => (None, 0),
                            };
                            if others {
                                let blob = http::download_snapshot(
                                    url_scheme,
                                    &api_base,
                                    doc_id,
                                    &meta.snapshot_id,
                                    auth.clone(),
                                )
                                .await;
                                held_snapshot = SnapshotId::parse_str(&meta.snapshot_id)
                                    .ok()
                                    .zip(blob)
                                    .map(|(snapshot_id, blob)| HeldSnapshot {
                                        snapshot_id,
                                        covered_seq: Seq(
                                            u64::try_from(meta.covered_seq).unwrap_or(0)
                                        ),
                                        blob,
                                    });
                            }
                            // The local copy already folds in the relay's
                            // snapshot: no download, and the replay starts
                            // after the copy's horizon (`RESID-010`).
                            let resumed = resume.filter(|horizon| {
                                light::resumes(*horizon, u64::try_from(start_covered).unwrap_or(0))
                            });
                            let payload = match (&start_id, resumed) {
                                (Some(id), None) => {
                                    http::download_snapshot(url_scheme, &api_base, doc_id, id, auth)
                                        .await
                                }
                                _ => None,
                            };
                            let snap_id = start_id.and_then(|id| SnapshotId::parse_str(&id).ok());
                            // Only gate on the covered seq if the blob actually
                            // downloaded; otherwise a failed fetch would skip
                            // history that was never applied (data loss).
                            match (payload, resumed) {
                                (Some(payload), _) => (Some(payload), snap_id, start_covered),
                                (None, Some(horizon)) => {
                                    (None, snap_id, i64::try_from(horizon.0).unwrap_or(i64::MAX))
                                }
                                (None, None) => (None, None, 0),
                            }
                        }
                        // No snapshot yet, or a transient error: fall back to the
                        // full delta replay (RequestMissing from seq 0) on open.
                        // With no snapshot on the relay, a local copy still
                        // spares the replay of what it holds (`RESID-010`).
                        Ok(None) => match resume {
                            Some(horizon) => {
                                (None, None, i64::try_from(horizon.0).unwrap_or(i64::MAX))
                            }
                            None => (None, None, 0),
                        },
                        Err(_) => (None, None, 0),
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
                    held_snapshot,
                    doc_type,
                    server_created,
                    signing_keys,
                    role,
                    rotation_owed,
                    links,
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
        Self::unwrap_anchored(exchange_key, members).map(|(key, _)| key)
    }

    /// Unwraps our own copy of the key, and the anchor of the membership log
    /// sealed beside it (`SYNC-104`).
    pub(crate) fn unwrap_anchored(
        exchange_key: &x25519_dalek::StaticSecret,
        members: &[http::MemberEntry],
    ) -> Result<(DocumentKey, Option<ambre_protocol::EntryDigest>), String> {
        crate::sharing::unwrap_own(
            exchange_key,
            members.iter().map(|m| {
                (
                    m.exchange_pub_key.as_str(),
                    m.doc_key_enc.as_str(),
                    m.wrapper_exchange_pub_key.as_str(),
                )
            }),
        )
    }

    /// Reads the key, the directory and our role out of ONE members answer
    /// (`SYNC-092`).
    ///
    /// A key that fails to unwrap is an authentication error, and no key is
    /// generated in its place (`SYNC-012`). A generated key would fork the
    /// document in silence. An absent or unparsable own row reads as `Viewer`,
    /// the safe direction for a gate the server enforces anyway.
    ///
    /// The log walks from the anchor sealed with our copy (`SYNC-104`). A log
    /// that does not verify leaves the open as it is (`SYNC-111`). It walks
    /// under `self_join`, the rule of the document's log (`SYNC-106`).
    pub(crate) fn opened_from_members(
        resp: &http::MembersResponse,
        document_id: DocumentId,
        exchange_key: &x25519_dalek::StaticSecret,
        my_user_id: Option<UserId>,
        self_join: membership_log::SelfJoinRule,
    ) -> Result<OpenedMembers, String> {
        let (doc_key, anchor) = Self::unwrap_anchored(exchange_key, &resp.members)?;
        let log = crate::sharing::walk_served_under(document_id, anchor, &resp.log, self_join);
        let role = crate::sharing::own_role(
            resp.members
                .iter()
                .map(|m| (m.user_id.as_str(), m.role.as_str())),
            my_user_id,
        );
        let signing_keys = crate::sharing::presence_directory(
            resp.members
                .iter()
                .map(|m| (m.user_id.as_str(), m.signing_pub_key.as_str())),
        );
        Ok(OpenedMembers {
            doc_key,
            signing_keys,
            role,
            rotation_owed: resp.rotation_owed,
            signers: log.signers().cloned(),
            named_key: my_user_id.and_then(|me| log.wraps(me)),
        })
    }

    /// Whether the key we unwrapped is the one our invitation names, or one
    /// the chain walks back to it from (`SYNC-124`). A rotation since the
    /// invitation installed the newer key. A key the relay swapped in reaches
    /// no link back. With no named key, the log does not verify our row, and
    /// the open goes on (`SYNC-111`).
    ///
    /// # Errors
    ///
    /// The reason the key is refused.
    pub(crate) fn named_key_holds(
        document_id: DocumentId,
        doc_key: &DocumentKey,
        named_key: Option<ambre_core::KeyId>,
        links: &[ambre_protocol::KeyLink],
    ) -> Result<(), String> {
        let Some(named) = named_key else {
            return Ok(());
        };
        let chain = crate::chain::Chain::new(document_id, links.iter().cloned());
        match chain.recover(document_id, doc_key, named) {
            Some(_) => Ok(()),
            None => Err("the document key is not the one the invitation names".into()),
        }
    }

    /// Tells the Logic thread the PeerID this session writes `document_id`
    /// under, with its signed registry entry (`AUTHOR-001`, `AUTHOR-019`).
    ///
    /// The nonce is drawn on the first open and kept until the close, so each
    /// open draws a new one (`AUTHOR-002`). A second open of a document still
    /// open keeps it: the operations already written must keep deriving. Without
    /// a signing key or a user, nothing is sent, and the Logic thread applies
    /// no edit.
    pub(crate) fn issue_session_identity(&mut self, document_id: DocumentId) {
        let (Some(signing_key), Some(user)) = (self.signing_key.as_ref(), self.user_id) else {
            return;
        };
        let nonce = *self
            .session_nonces
            .entry(document_id)
            .or_insert_with(ambre_crypto::peer::draw_nonce);
        let (peer, entry) =
            ambre_crypto::peer::session_identity(signing_key, user, document_id, nonce);
        self.send_to_logic(LogicEvent::SessionIdentity {
            document_id,
            peer,
            entry: entry.to_bytes(),
            nonce,
        });
    }

    pub(crate) fn handle_close_document(&mut self, document_id: DocumentId) {
        if self.ws_connected() {
            self.ws_send(&WsClientMsg::Unsubscribe { document_id });
        }
        self.documents.remove(&document_id);
        self.session_nonces.remove(&document_id);
        self.light.remove(&document_id);
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
        if self.signing_key.is_none() {
            // Session not established yet (for example an edit landed in the gap between
            // an F5 and DevLogin completing). Buffer instead of dropping; flushed
            // once the document opens (see `pending_pre_session_updates`).
            tracing::info!("[Sync] SendUpdate before session ready, buffering doc={document_id}");
            self.pending_pre_session_updates
                .push((document_id, payload));
            return;
        }
        // The nonce rides inside the ciphertext, ahead of the delta
        // (`SYNC-112`). A document opened with no identity has written nothing
        // the Logic thread would send.
        let Some(nonce) = self.session_nonces.get(&document_id) else {
            tracing::warn!("[Sync] SendUpdate with no session nonce doc={document_id}, dropped");
            return;
        };
        self.send_update_body(document_id, *nonce, &payload);
    }

    /// Builds the body of a delta this client writes and sends it. No
    /// application of this build writes a summary yet, so none rides along.
    fn send_update_body(&mut self, document_id: DocumentId, nonce: SessionNonce, delta: &[u8]) {
        match pipeline::update_body(&nonce, None, delta) {
            Ok(body) => self.send_body(document_id, body),
            Err(error) => tracing::warn!("[Sync] update body refused doc={document_id}: {error}"),
        }
    }

    /// Sends what an earlier session of this user wrote and never sent. The
    /// body carries that session's nonce, so a reader derives the PeerID that
    /// wrote it (`AUTHOR-028`).
    pub(crate) fn handle_send_unsent(
        &mut self,
        document_id: DocumentId,
        payload: Vec<u8>,
        nonce: SessionNonce,
    ) {
        if self.signing_key.is_none() {
            // Sent after the document is ready, so after the session is.
            tracing::warn!("[Sync] SendUnsent before session ready doc={document_id}, dropped");
            return;
        }
        self.send_update_body(document_id, nonce, &payload);
    }

    /// Seals an update body and sends it, or holds it while the document is
    /// created or owes a rotation. The body carries its nonce, so a frame
    /// sent again later keeps the PeerID it was written under.
    pub(crate) fn send_body(&mut self, document_id: DocumentId, body: Vec<u8>) {
        let (Some(signing_key), Some(pub_key)) = (self.signing_key.clone(), self.author_pub_key())
        else {
            return;
        };
        // Lazy server creation: a local-only doc materializes on its first update.
        // Buffer the body and fire the create POST once; the buffer is flushed
        // in order from the `DocumentMaterialized` handler.
        if self
            .documents
            .get(&document_id)
            .is_some_and(|d| !d.server_created)
        {
            if let Some(doc_state) = self.documents.get_mut(&document_id) {
                doc_state.pending_updates.push(body);
            }
            self.trigger_lazy_create(document_id);
            return;
        }
        let Some(doc_state) = self.documents.get_mut(&document_id) else {
            // No session of this document: the update cannot reach the relay,
            // so the refusal is said aloud (`SYNC-069`).
            tracing::warn!("[Sync] SendUpdate for unknown doc {document_id:?}");
            self.send_to_logic(LogicEvent::SyncWarning {
                document_id,
                code: SyncWarningCode::UpdateRefused,
                message: "update for a document with no open session".into(),
            });
            return;
        };
        // An owed rotation is paid before any frame: the edit waits unsealed,
        // and takes its clock once the new key lands (`SYNC-128`).
        if doc_state.rekey.owes() {
            if doc_state.rekey.hold(body) {
                self.start_settle(document_id);
            }
            return;
        }
        doc_state.local_clock += 1;
        let clock = doc_state.local_clock;
        if let Some((key_id, signature)) =
            self.seal_and_send(document_id, clock, &body, &signing_key, pub_key)
            && let Some(doc_state) = self.documents.get_mut(&document_id)
            && doc_state.rekey.record_sent(SentFrame {
                clock,
                key_id,
                signature,
                payload: body,
            })
        {
            tracing::warn!("[Sync] unconfirmed log full doc={document_id}, oldest forgotten");
        }
    }

    /// Seals an update `body` at `clock` under the current key and sends it.
    /// Returns the key it was sealed under and its signature, `None` when the
    /// seal failed.
    fn seal_and_send(
        &self,
        document_id: DocumentId,
        clock: u64,
        body: &[u8],
        signing_key: &SigningKey,
        pub_key: PublicKey,
    ) -> Option<(KeyId, Signature)> {
        let doc_state = self.documents.get(&document_id)?;
        let key_id = doc_state.doc_key.id();
        let public_data = UpdatePublicData {
            document_id,
            pub_key,
            key_id,
            snapshot_id: doc_state.current_snapshot_id,
            clock: Clock(clock),
        };
        match pipeline::update_frame(body, public_data, &doc_state.doc_key, signing_key) {
            Ok(frame) => {
                tracing::info!("[Sync] seal_update ok doc={document_id} clock={clock}");
                self.ws_send_sealed(&frame);
                frame
                    .update_signature()
                    .map(|signature| (key_id, signature))
            }
            Err(e) => {
                tracing::error!("[Sync] SendUpdate seal failed for {document_id:?}: {e}");
                None
            }
        }
    }

    /// Seals the refused frames again under the current key, at their own
    /// clocks and in order (`SYNC-129`). Then the edits that waited for the key
    /// take the next clocks.
    pub(crate) fn resend_refused(
        &mut self,
        document_id: DocumentId,
        refused: Vec<(u64, Vec<u8>)>,
        waiting: Vec<Vec<u8>>,
    ) {
        if let (Some(signing_key), Some(pub_key)) =
            (self.signing_key.clone(), self.author_pub_key())
        {
            for (clock, body) in refused {
                if let Some((_, signature)) =
                    self.seal_and_send(document_id, clock, &body, &signing_key, pub_key.clone())
                    && let Some(doc_state) = self.documents.get_mut(&document_id)
                {
                    doc_state.rekey.resealed(clock, signature);
                }
            }
        }
        for body in waiting {
            self.send_body(document_id, body);
        }
    }

    /// The relay refused our delta signed `signature` for its clock: another
    /// session of this user took it first (`SRV-037`). The answer lifts the
    /// relay's fence on this socket (`SRV-169`). The delta and every later one
    /// then leave again in order, from the `expected` clock on (`SYNC-147`).
    pub(crate) fn handle_clock_refused(
        &mut self,
        document_id: DocumentId,
        expected: u64,
        signature: Signature,
    ) {
        // Answered even when no delta here carries it, so a delta this log
        // forgot never leaves the fence standing.
        self.ws_send(&WsClientMsg::ClockResync {
            document_id,
            signature,
        });
        let Some(doc_state) = self.documents.get_mut(&document_id) else {
            return;
        };
        let Some(bodies) = doc_state.rekey.clock_refused(&signature) else {
            return;
        };
        tracing::info!(
            "[Sync] clock refused doc={document_id}, {} delta(s) leave again from {expected}",
            bodies.len()
        );
        // The clocks of the deltas taken back are void: the relay took none.
        doc_state.local_clock = expected.saturating_sub(1);
        for body in bodies {
            self.send_body(document_id, body);
        }
    }

    /// Installs `new_key` as the document's current key, then sends what the
    /// owed rotation held back. A key this client already holds as current is
    /// not pushed into the cache a second time. `links` join the chain, so a
    /// key the cache drops still opens through them (`SYNC-119`).
    pub(crate) fn install_key(
        &mut self,
        document_id: DocumentId,
        new_key: DocumentKey,
        links: impl IntoIterator<Item = ambre_protocol::KeyLink>,
    ) {
        let Some(doc_state) = self.documents.get_mut(&document_id) else {
            return;
        };
        for link in links {
            doc_state.chain.push(document_id, link);
        }
        if doc_state.doc_key.id() != new_key.id() {
            doc_state.rotate_key(new_key);
        }
        let current = doc_state.doc_key.id();
        let (refused, waiting) = doc_state.rekey.key_installed(current);
        self.resend_refused(document_id, refused, waiting);
    }

    /// Pays the rotation a document owes, in a spawned task (`SYNC-128`).
    pub(crate) fn start_settle(&mut self, document_id: DocumentId) {
        let previous = self
            .documents
            .get(&document_id)
            .map(|doc_state| doc_state.doc_key.clone());
        let (Some(tx), Some(exchange_key), Some(signing_key), Some(previous)) = (
            self.event_tx.clone(),
            self.exchange_key.clone(),
            self.signing_key.clone(),
            previous,
        ) else {
            if let Some(doc_state) = self.documents.get_mut(&document_id) {
                doc_state.rekey.payment_failed();
            }
            return;
        };
        let params = crate::sharing::SettleParams {
            document_id,
            url_scheme: self.url_scheme,
            api_base: self.api_base.clone(),
            exchange_key,
            previous,
            signing_key,
        };
        let http = crate::sharing::GlooHttp {
            auth: self.auth_context(),
        };
        wasm_bindgen_futures::spawn_local(async move {
            crate::sharing::settle_task(&http, params, &tx).await;
        });
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
        let (Some(exchange_key), Some(author)) = (self.exchange_key.clone(), self.author()) else {
            tracing::warn!("[Sync] lazy create for {document_id:?} without session keys");
            return;
        };
        let Some(doc_state) = self.documents.get_mut(&document_id) else {
            return;
        };
        doc_state.creating = true;
        let api_base = self.api_base.clone();
        let url_scheme = self.url_scheme;
        let auth = self.auth_context();
        wasm_bindgen_futures::spawn_local(async move {
            // The copy carries the anchor of the log its creator starts
            // (`SYNC-104`, `SYNC-132`).
            let (doc_key_enc, entry) =
                match sharing::creation(document_id, &doc_key, &exchange_key, &author) {
                    Ok(created) => created,
                    Err(message) => {
                        let _ = tx.send(SyncEvent::DocumentCreateFailed {
                            document_id,
                            message,
                        });
                        return;
                    }
                };

            let url = format!("{url_scheme}://{api_base}/api/documents");
            let body = http::CreateDocumentRequest {
                doc_type,
                name_enc: None,
                doc_key_enc,
                document_id: Some(document_id.to_string()),
                entry: Some(entry),
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

    /// Seal and upload a snapshot that folds in every delta up to
    /// `covered_seq`, then confirm it as its author (`AUTHOR-021`).
    ///
    /// The author may vouch for it. Every operation of another user in it came
    /// from a delta that user signed, or from a snapshot this client checked.
    pub(crate) fn handle_send_snapshot(
        &mut self,
        document_id: DocumentId,
        payload: Vec<u8>,
        covered_seq: Seq,
    ) {
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
            key_id: doc_state.doc_key.id(),
            snapshot_id: doc_state.current_snapshot_id,
            // The BLAKE2b parent chain is a post-MVP hardening (see
            // ambre-spec-sealed-envelope.md). An empty proof seals a standalone snapshot,
            // which the compaction model accepts for the genesis-parented MVP.
            parent_snapshot_proof: Vec::new(),
            parent_snapshot_update_clocks: Vec::new(),
        };
        // The full sealed `Snapshot`, serialized as the S3 blob (the inverse of
        // `decode_snapshot_blob` + `open_snapshot`).
        let blob = match pipeline::snapshot_blob(
            &payload,
            public_data,
            &doc_state.doc_key,
            &signing_key,
        ) {
            Ok(blob) => blob,
            Err(e) => {
                tracing::error!("Sync: SendSnapshot seal failed for {document_id:?}: {e}");
                return;
            }
        };

        // Off the wasm path (host tests / mock relay) there is no event queue
        // and no real S3: sealing is the observable behavior, stop here.
        let Some(tx) = self.event_tx.clone() else {
            tracing::info!(
                "Sync: SendSnapshot {document_id:?} sealed ({} bytes), no event channel (test path)",
                blob.len()
            );
            return;
        };
        // The Logic thread names the last delta it holds with no gap below it,
        // not the high-water `last_seq`. A snapshot claims every delta up to
        // its horizon, and a checker holds it to that claim (`AUTHOR-016`).
        let covered_seq = i64::try_from(covered_seq.0).unwrap_or(i64::MAX);
        let base = format!("{}://{}", self.url_scheme, self.api_base);
        let auth = self.auth_context();
        let upload = SnapshotUpload {
            document_id,
            author_pubkey,
            covered_seq,
            blob,
        };
        wasm_bindgen_futures::spawn_local(async move {
            match upload_snapshot(&http::Fetch, &auth, &base, upload).await {
                Ok(snapshot_id) => {
                    let _ = tx.send(SyncEvent::SnapshotUploaded {
                        document_id,
                        snapshot_id,
                    });
                }
                Err(e) => tracing::error!("Sync: SendSnapshot failed for {document_id:?}: {e}"),
            }
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
            key_id: doc_state.doc_key.id(),
            counter: Counter(doc_state.ephemeral_counter),
        };
        match pipeline::ephemeral_frame(&payload, public_data, &doc_state.doc_key, &signing_key) {
            Ok(frame) => self.ws_send_sealed(&frame),
            Err(e) => tracing::error!("Sync: SendEphemeral seal failed for {document_id:?}: {e}"),
        }
    }
}

/// What a client tells the relay about another author's pending snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Vouch {
    /// The snapshot matches the deltas it covers (`SRV-126`).
    Confirm,
    /// It contradicts them: the hex signature of the report (`SRV-133`).
    Report(String),
}

/// Posts `vouch` on `snapshot_id` to the relay at `base`, under the
/// session's token.
pub(crate) async fn send_vouch(
    transport: &impl http::Transport,
    auth: &http::AuthContext,
    base: &str,
    document_id: DocumentId,
    snapshot_id: SnapshotId,
    vouch: &Vouch,
) -> Result<(), String> {
    let url = format!("{base}/api/documents/{document_id}/snapshots/{snapshot_id}");
    let post = http::Method::Post;
    match vouch {
        Vouch::Confirm => {
            let body = serde_json::json!({});
            http::send_json_auth_via(transport, post, &format!("{url}/confirm"), &body, auth).await
        }
        Vouch::Report(signature) => {
            let body = http::ReportSnapshotRequest {
                signature: signature.clone(),
            };
            http::send_json_auth_via(transport, post, &format!("{url}/report"), &body, auth).await
        }
    }
}

/// One sealed snapshot on its way to the relay.
pub(crate) struct SnapshotUpload {
    /// The document it compacts.
    pub document_id: DocumentId,
    /// Its author's signing key, in hex.
    pub author_pubkey: String,
    /// The last delta it covers.
    pub covered_seq: i64,
    /// The sealed blob.
    pub blob: pipeline::SealedBlob,
}

/// Uploads a sealed snapshot, then confirms it as its author (`AUTHOR-021`).
/// The steps run in order: the upload link, the object, the metadata, the
/// confirmation. Returns the id the relay gave it, or the first step that
/// failed. A failed confirmation leaves the snapshot pending, so it is
/// logged and the id is still returned.
pub(crate) async fn upload_snapshot(
    transport: &impl http::Transport,
    auth: &http::AuthContext,
    base: &str,
    upload: SnapshotUpload,
) -> Result<SnapshotId, String> {
    let SnapshotUpload {
        document_id,
        author_pubkey,
        covered_seq,
        blob,
    } = upload;
    let link: http::SnapshotUploadUrlResponse = http::post_json_auth_via(
        transport,
        &format!("{base}/api/documents/{document_id}/snapshot/upload-url"),
        &serde_json::json!({ "size_bytes": blob.len() }),
        auth,
    )
    .await?;
    // The presigned link is its own credential: no session token goes there.
    http::put_sealed_via(transport, &link.upload_url, &blob).await?;
    let metadata = http::CreateSnapshotRequest {
        snapshot_id: SnapshotId::new().to_string(),
        author_pubkey,
        parent_hash: None,
        parent_snapshot_proof: None,
        update_clocks: serde_json::json!({}),
        s3_key: link.s3_key,
        covered_seq,
    };
    let created: http::CreateSnapshotResponse = http::post_json_auth_via(
        transport,
        &format!("{base}/api/documents/{document_id}/snapshot"),
        &metadata,
        auth,
    )
    .await?;
    let snapshot_id =
        SnapshotId::parse_str(&created.snapshot_id).map_err(|e| format!("bad snapshot_id: {e}"))?;
    // The author's own confirmation makes it `self` (`AUTHOR-021`).
    if let Err(e) = send_vouch(
        transport,
        auth,
        base,
        document_id,
        snapshot_id,
        &Vouch::Confirm,
    )
    .await
    {
        tracing::warn!("Sync: SendSnapshot self-confirm failed for {document_id:?}: {e}");
    }
    Ok(snapshot_id)
}

/// What an open reads out of one members answer (`SYNC-092`).
pub(crate) struct OpenedMembers {
    pub doc_key: DocumentKey,
    pub signing_keys: Vec<([u8; 32], UserId)>,
    pub role: Role,
    pub rotation_owed: bool,
    /// The keys the membership log lets sign a key link (`SYNC-121`).
    /// `None` for a document older than the log.
    pub signers: Option<std::collections::HashSet<[u8; 32]>>,
    /// The key the entry that added us names, when the log verifies our row
    /// (`SYNC-123`).
    pub named_key: Option<ambre_core::KeyId>,
}

/// What the members request says about a document being opened (`SYNC-040`).
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum MembersAnswer<T> {
    /// The server holds the document, and its members ride the answer.
    Exists(T),
    /// The server answered that it holds no such document: the open is local-only.
    Absent,
    /// The request failed, so nothing is known about the document.
    Unreachable(String),
}

impl<T> MembersAnswer<T> {
    /// Classifies the outcome of the members request. Only a `404` is an absent
    /// document; a transport or decoding failure is never read as one.
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    pub(crate) fn classify(outcome: Result<Option<T>, String>) -> Self {
        match outcome {
            Ok(Some(members)) => Self::Exists(members),
            Ok(None) => Self::Absent,
            Err(message) => Self::Unreachable(message),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::scripted::{Scripted, session};
    use ambre_protocol::SeqUpdate;

    #[test]
    fn a_failed_members_request_is_not_an_absent_document() {
        let failed = MembersAnswer::<()>::classify(Err("GET /members: timeout".into()));
        assert_eq!(
            failed,
            MembersAnswer::Unreachable("GET /members: timeout".into())
        );
        assert_eq!(
            MembersAnswer::<()>::classify(Ok(None)),
            MembersAnswer::Absent
        );
        assert_eq!(
            MembersAnswer::classify(Ok(Some(7))),
            MembersAnswer::Exists(7)
        );
    }

    /// One members row for `uid`, its key wrapped by `wrapper` for `holder`.
    fn member_row(
        uid: UserId,
        holder: &x25519_dalek::StaticSecret,
        wrapper: &x25519_dalek::StaticSecret,
        doc_key: &DocumentKey,
        role: &str,
    ) -> http::MemberEntry {
        let holder_pub = x25519_dalek::PublicKey::from(holder).to_bytes();
        let wrapper_pub = x25519_dalek::PublicKey::from(wrapper).to_bytes();
        http::MemberEntry {
            user_id: uid.to_string(),
            signing_pub_key: hex::encode([uid.as_bytes()[0]; 32]),
            exchange_pub_key: hex::encode(holder_pub),
            doc_key_enc: crate::sharing::wrap_anchored_b64(wrapper, &holder_pub, doc_key, None)
                .unwrap(),
            role: role.into(),
            wrapper_exchange_pub_key: hex::encode(wrapper_pub),
        }
    }

    /// Holds `SYNC-092`: the key, the directory and the role all come out of
    /// the one answer handed in, and nothing else is consulted.
    #[test]
    fn the_key_the_directory_and_the_role_come_from_one_answer() {
        let me = UserId::new();
        let peer = UserId::new();
        let my_xk = ambre_crypto::MasterSeed::from_array([3; 32]).derive_exchange_key();
        let peer_xk = ambre_crypto::MasterSeed::from_array([4; 32]).derive_exchange_key();
        let doc_key = DocumentKey::generate();
        let resp = http::MembersResponse {
            members: vec![
                member_row(me, &my_xk, &peer_xk, &doc_key, "editor"),
                member_row(peer, &peer_xk, &peer_xk, &doc_key, "owner"),
            ],
            rotation_owed: true,
            log: Vec::new(),
        };
        let opened = SyncState::opened_from_members(
            &resp,
            DocumentId::new(),
            &my_xk,
            Some(me),
            membership_log::no_self_join,
        )
        .unwrap();
        assert!(opened.doc_key.id() == doc_key.id());
        assert_eq!(opened.role, Role::Editor);
        let ids: Vec<UserId> = opened.signing_keys.iter().map(|(_, id)| *id).collect();
        assert_eq!(ids, vec![me, peer]);
        assert!(opened.rotation_owed);
    }

    /// Holds `SYNC-012`: a key row that does not unwrap fails the open, and no
    /// key is generated in its place.
    #[test]
    fn a_key_that_does_not_unwrap_opens_nothing() {
        let me = UserId::new();
        let my_xk = ambre_crypto::MasterSeed::from_array([3; 32]).derive_exchange_key();
        let peer_xk = ambre_crypto::MasterSeed::from_array([4; 32]).derive_exchange_key();
        let mut row = member_row(me, &my_xk, &peer_xk, &DocumentKey::generate(), "editor");
        let mut raw = base64::engine::general_purpose::URL_SAFE
            .decode(&row.doc_key_enc)
            .unwrap();
        let last = raw.len() - 1;
        raw[last] ^= 1;
        row.doc_key_enc = base64::engine::general_purpose::URL_SAFE.encode(&raw);
        let resp = http::MembersResponse {
            members: vec![row],
            rotation_owed: false,
            log: Vec::new(),
        };
        assert!(
            SyncState::opened_from_members(
                &resp,
                DocumentId::new(),
                &my_xk,
                Some(me),
                membership_log::no_self_join,
            )
            .is_err()
        );
    }

    /// Holds `SYNC-124`: a key the invitation does not name is refused, unless
    /// a link reaches the named key from it, as after a rotation. A log that
    /// does not verify our row names no key, and the open goes on (`SYNC-111`).
    #[test]
    fn a_key_the_invitation_does_not_name_is_refused() {
        let document_id = DocumentId::new();
        let named = DocumentKey::generate();
        let rotated = DocumentKey::generate();
        let signer = ambre_crypto::MasterSeed::from_array([5; 32]).derive_signing_key();
        let link = crate::pipeline::seal_link(document_id, &named, &rotated, &signer).unwrap();
        let held = |key: &DocumentKey, links: &[ambre_protocol::KeyLink]| {
            SyncState::named_key_holds(document_id, key, Some(named.id()), links)
        };
        assert!(held(&named, &[]).is_ok());
        assert!(held(&rotated, std::slice::from_ref(&link)).is_ok());
        assert!(held(&DocumentKey::generate(), &[link]).is_err());
        assert!(SyncState::named_key_holds(document_id, &rotated, None, &[]).is_ok());
    }

    /// A `SyncState` with one opened document, a controlled `doc_key` and a known
    /// signing key, plus the drainable Logic receiver and the document's known
    /// snapshot id.
    fn setup() -> (
        SyncState,
        ambre_bus::conduit::Receiver<LogicEvent>,
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
        state.session_nonces.insert(document_id, TEST_NONCE);
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
            key_id: doc_key.id(),
            snapshot_id,
            clock: Clock(clock),
        };
        let body = pipeline::update_body(&TEST_NONCE, None, payload).expect("no summary");
        pipeline::seal_update(&body, public_data, doc_key, signing_key).expect("seal")
    }

    /// The session nonce every update of these tests carries.
    const TEST_NONCE: SessionNonce = SessionNonce([6; 16]);

    /// A frame opens with the one key it names (`SYNC-045`). A previous key
    /// still cached opens its own frames, and a key this client never held
    /// is refused before any open is attempted.
    #[test]
    fn a_frame_opens_with_the_key_it_names_and_no_other() {
        let (mut state, _rx, document_id, old_key, signing_key, snapshot) = setup();
        let new_key = DocumentKey::generate();
        let stranger = DocumentKey::generate();
        let doc_state = state.documents.get_mut(&document_id).unwrap();
        doc_state.rotate_key(new_key.clone());

        let under =
            |key: &DocumentKey| make_update(document_id, snapshot, 1, b"d", key, &signing_key);
        let opened = |key: &DocumentKey| doc_state.open_signed_update(&under(key), Seq(1)).unwrap();
        assert_eq!(opened(&new_key).payload, b"d");
        assert_eq!(opened(&old_key).payload, b"d");
        assert_eq!(
            doc_state.key_named(old_key.id()).map(|key| key.id()),
            Some(old_key.id())
        );

        assert!(doc_state.key_named(stranger.id()).is_none());
        assert!(matches!(
            doc_state.open_update_named(&under(&stranger)),
            Err(pipeline::PipelineError::WrongKey)
        ));
    }

    /// A document opened twelve rotations after its snapshot, past the
    /// ten-key cache, opens the snapshot and an old frame through the chain
    /// the relay served. The current key stays the one the open installed
    /// (`SYNC-119`, `SYNC-122`).
    #[test]
    fn an_open_walks_the_chain_to_a_snapshot_older_than_the_cache() {
        let (mut state, rx, doc, first, sk, _snap) = setup();
        state.documents.remove(&doc);
        let mut keys = vec![first];
        keys.extend((0..12).map(|_| DocumentKey::generate()));
        let links: Vec<_> = keys
            .windows(2)
            .map(|pair| pipeline::seal_link(doc, &pair[0], &pair[1], &sk).unwrap())
            .collect();
        let current = keys.last().unwrap().clone();
        let snapshot_id = SnapshotId::new();
        let blob = snapshot_blob(doc, snapshot_id, b"snap", &keys[0], &sk);
        state.handle_sync_event(SyncEvent::OpenDocumentReady {
            document_id: doc,
            doc_key: current.clone(),
            snapshot_payload: Some(blob),
            snapshot_id,
            covered_seq: Seq(3),
            held_snapshot: None,
            doc_type: DocType::Trame,
            server_created: true,
            signing_keys: Vec::new(),
            role: Role::Owner,
            rotation_owed: false,
            links,
        });
        let opened = drain(&rx).into_iter().any(
            |e| matches!(e, LogicEvent::RemoteSnapshot { ref payload, .. } if payload == b"snap"),
        );
        assert!(opened, "the snapshot opens under a key the chain recovers");

        let doc_state = &state.documents[&doc];
        let old = make_update(doc, snapshot_id, 1, b"d", &keys[1], &sk);
        assert_eq!(
            doc_state.open_signed_update(&old, Seq(1)).unwrap().payload,
            b"d"
        );
        assert_eq!(doc_state.doc_key.id(), current.id());
        assert!(
            doc_state.old_keys.is_empty(),
            "a recovered key is not cached"
        );
    }

    /// The blob object storage holds for a snapshot of `document_id`.
    fn snapshot_blob(
        document_id: DocumentId,
        snapshot_id: SnapshotId,
        payload: &[u8],
        doc_key: &DocumentKey,
        signing_key: &SigningKey,
    ) -> Vec<u8> {
        let public_data = SnapshotPublicData {
            document_id,
            pub_key: PublicKey::from_array(signing_key.verifying_key().to_bytes()),
            key_id: doc_key.id(),
            snapshot_id,
            parent_snapshot_proof: Vec::new(),
            parent_snapshot_update_clocks: Vec::new(),
        };
        let snapshot =
            pipeline::seal_snapshot(payload, public_data, doc_key, signing_key).expect("seal");
        postcard::to_allocvec(&snapshot).expect("encode")
    }

    fn drain(rx: &ambre_bus::conduit::Receiver<LogicEvent>) -> Vec<LogicEvent> {
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
            LogicEvent::RemoteUpdate { document_id, delta } => {
                assert_eq!(*document_id, doc);
                assert_eq!(delta.payload, b"hello");
            }
            other => panic!("expected RemoteUpdate, got {other:?}"),
        }
        assert_eq!(state.documents[&doc].quarantine_len(), 0);
    }

    /// An opened update reaches the Logic thread with the nonce its body
    /// carried and the PeerID its signer and that nonce derive. It also names
    /// the member the directory holds for its signer (`SYNC-114`, `AUTHOR-013`). A signer the
    /// directory does not name resolves to no member.
    #[test]
    fn an_opened_update_reaches_logic_with_its_nonce_its_peer_and_its_signer() {
        let (mut state, rx, doc, key, sk, snap) = setup();
        let alice = UserId::new();
        let signer = sk.verifying_key().to_bytes();
        state
            .documents
            .get_mut(&doc)
            .unwrap()
            .set_signing_keys([(signer, alice)]);
        state.receive_update(Seq(1), make_update(doc, snap, 1, b"hello", &key, &sk));
        let stranger = MasterSeed::generate().derive_signing_key();
        state.receive_update(Seq(2), make_update(doc, snap, 1, b"hi", &key, &stranger));

        let deltas: Vec<SignedDelta> = drain(&rx)
            .into_iter()
            .map(|e| match e {
                LogicEvent::RemoteUpdate { delta, .. } => delta,
                other => panic!("expected RemoteUpdate, got {other:?}"),
            })
            .collect();
        let expected = SignedDelta {
            seq: 1,
            payload: b"hello".to_vec(),
            nonce: TEST_NONCE,
            peer: ambre_crypto::peer::derive_peer_id(&signer, &TEST_NONCE),
            user: Some(alice),
        };
        assert_eq!(deltas.first(), Some(&expected));
        let unknown = deltas.get(1).unwrap();
        assert_eq!(unknown.user, None);
        assert_eq!(
            unknown.peer,
            ambre_crypto::peer::derive_peer_id(&stranger.verifying_key().to_bytes(), &TEST_NONCE)
        );
    }

    /// Each open of a document draws a new PeerID (`AUTHOR-002`). A second
    /// open of a document still open keeps the one its operations derive.
    #[test]
    fn each_open_draws_a_new_peer_and_a_repeated_open_keeps_it() {
        let (mut state, rx, doc, _key, sk, _snap) = setup();
        state.session_nonces.clear();
        state.signing_key = Some(sk);
        state.user_id = Some(UserId::new());
        let issue = |state: &mut SyncState| {
            state.issue_session_identity(doc);
            drain(&rx)
                .into_iter()
                .find_map(|e| match e {
                    LogicEvent::SessionIdentity { peer, .. } => Some(peer),
                    _ => None,
                })
                .unwrap()
        };
        let first = issue(&mut state);
        assert_eq!(issue(&mut state), first);
        state.handle_close_document(doc);
        assert_ne!(issue(&mut state), first);
    }

    /// An update whose body is too short for the nonce and the marker is
    /// dropped with a warning, and nothing reaches the document.
    #[test]
    fn an_update_too_short_for_its_parts_is_dropped() {
        let (mut state, rx, doc, key, sk, snap) = setup();
        let public_data = UpdatePublicData {
            document_id: doc,
            pub_key: PublicKey::from_array(sk.verifying_key().to_bytes()),
            key_id: key.id(),
            snapshot_id: snap,
            clock: Clock(1),
        };
        let short = pipeline::seal_update(&[6; 16], public_data, &key, &sk).unwrap();
        state.receive_update(Seq(1), short);
        let events = drain(&rx);
        assert!(matches!(
            events.as_slice(),
            [LogicEvent::SyncWarning {
                code: SyncWarningCode::DecryptFailed,
                ..
            }]
        ));
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
                LogicEvent::RemoteUpdate { delta, .. } => delta.payload,
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

    /// `SYNC-007`: the overflow drops the update with the lowest clock, across
    /// every parent it waits on. That update arrives neither first nor last,
    /// so dropping by arrival at either end fails.
    #[test]
    fn a_quarantine_overflow_drops_the_lowest_clock() {
        let (mut state, _rx, doc, key, sk, _snap) = setup();
        let parents = [SnapshotId::new(), SnapshotId::new()];
        // Clock 2 first, clock 1 second, then 3 up to the cap, alternating
        // between the two parents.
        let clocks = [2, 1].into_iter().chain(3..=(QUARANTINE_LIMIT as u64));
        for (i, clock) in clocks.enumerate() {
            let parent = parents[i % 2];
            state.receive_update(
                Seq(i as u64 + 1),
                make_update(doc, parent, clock, b"x", &key, &sk),
            );
        }
        let held: std::collections::BTreeSet<u64> = state.documents[&doc]
            .quarantine
            .values()
            .flatten()
            .map(|(_, u)| u.public_data.clock.0)
            .collect();
        assert_eq!(held.len(), QUARANTINE_LIMIT - 1);
        assert!(!held.contains(&1), "the lowest clock left");
        assert!(held.contains(&2) && held.contains(&(QUARANTINE_LIMIT as u64)));
    }

    // -- Snapshot lifecycle (upload adoption + cold-start seq gating) ----------

    /// Drain the WsClientMsg this state sent to its mock WS, decoded.
    fn drain_ws(rx: &ambre_bus::conduit::Receiver<Vec<u8>>) -> Vec<WsClientMsg> {
        rx.try_iter()
            .filter_map(|wire| ambre_protocol::ws::decode_client_msg(&wire).ok())
            .collect()
    }

    /// A downloaded snapshot covering up to seq N must make the post-open
    /// `RequestMissing` start AFTER N. Only the deltas the snapshot does not
    /// already contain are replayed, not the whole history since genesis.
    #[test]
    fn open_document_ready_gates_request_missing_on_covered_seq() {
        let (mut state, _rx, doc, key, _sk, _snap) = setup();
        let (ws_tx, ws_rx) = ambre_bus::conduit::unbounded();
        state.set_mock_ws(ws_tx);
        let snapshot_id = SnapshotId::new();

        let blob = snapshot_blob(doc, snapshot_id, b"snap", &key, &_sk);
        state.handle_sync_event(SyncEvent::OpenDocumentReady {
            document_id: doc,
            doc_key: key,
            snapshot_payload: Some(blob),
            snapshot_id,
            covered_seq: Seq(1500),
            held_snapshot: None,
            doc_type: DocType::Trame,
            server_created: true,
            signing_keys: Vec::new(),
            role: Role::Owner,
            rotation_owed: false,
            links: Vec::new(),
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
        let (ws_tx, ws_rx) = ambre_bus::conduit::unbounded();
        state.set_mock_ws(ws_tx);

        state.handle_sync_event(SyncEvent::OpenDocumentReady {
            document_id: doc,
            doc_key: key,
            snapshot_payload: None,
            snapshot_id: SnapshotId::genesis(doc),
            covered_seq: Seq(0),
            held_snapshot: None,
            doc_type: DocType::Trame,
            server_created: true,
            signing_keys: Vec::new(),
            role: Role::Owner,
            rotation_owed: false,
            links: Vec::new(),
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
                |e| matches!(e, LogicEvent::DocumentReady { document_id, .. } if *document_id == doc),
            )
            .count()
    }

    /// A departure is attributed by the member directory, exactly like a cursor
    /// (`PRESENCE-014`). The relay names the key, and the key resolves to the
    /// `UserId` the view keys presence by. A key nobody knows is dropped rather
    /// than attributed to a guess -- the peer then ages out of the live set.
    #[test]
    fn peer_left_resolves_the_key_the_relay_named() {
        let (mut state, rx, doc, _key, signing_key, _snap) = setup();
        let pub_key = PublicKey::from_array(signing_key.verifying_key().to_bytes());
        let alice = UserId::new();
        state
            .documents
            .get_mut(&doc)
            .expect("document")
            .signing_keys
            .insert(*pub_key.as_array(), alice);

        state.handle_ws_message(WsServerMsg::PeerLeft {
            document_id: doc,
            pub_key: pub_key.clone(),
        });
        let events = drain(&rx);
        assert!(
            events.iter().any(|e| matches!(
                e,
                LogicEvent::PeerLeft { document_id, user_id }
                    if *document_id == doc && *user_id == alice
            )),
            "the departure names the resolved user: {events:?}"
        );

        // A key the member directory does not carry is dropped in silence.
        let stranger = PublicKey::from_array([7u8; 32]);
        state.handle_ws_message(WsServerMsg::PeerLeft {
            document_id: doc,
            pub_key: stranger,
        });
        assert!(
            !drain(&rx)
                .iter()
                .any(|e| matches!(e, LogicEvent::PeerLeft { .. })),
            "an unattributable departure is dropped"
        );
    }

    /// The relay's word that the members of an open document changed reaches
    /// the Logic thread, which asks for the rows again. A document not open
    /// here has no view to refresh, so the word stops at Sync.
    #[test]
    fn a_change_of_members_of_an_open_document_reaches_logic() {
        let (mut state, rx, doc, _key, _signing_key, _snap) = setup();
        state.handle_ws_message(WsServerMsg::MembersChanged { document_id: doc });
        assert!(
            drain(&rx).iter().any(|e| matches!(
                e,
                LogicEvent::MembersChanged { document_id } if *document_id == doc
            )),
            "the change reaches Logic"
        );

        state.handle_ws_message(WsServerMsg::MembersChanged {
            document_id: DocumentId::new(),
        });
        assert!(
            !drain(&rx)
                .iter()
                .any(|e| matches!(e, LogicEvent::MembersChanged { .. })),
            "a document not open here is not reported"
        );
    }

    /// The relay's word that the members changed makes Sync list them again,
    /// so a newcomer's key resolves without a view asking (`SYNC-037`). Two
    /// listings may cross: the answer to the older one changes nothing.
    #[test]
    fn a_change_of_members_reads_the_directory_again_and_a_crossed_answer_changes_nothing() {
        let (mut state, rx, doc, _key, _signing_key, _snap) = setup();
        state.handle_ws_message(WsServerMsg::MembersChanged { document_id: doc });
        state.handle_ws_message(WsServerMsg::MembersChanged { document_id: doc });
        assert_eq!(
            state.documents[&doc].directory_asked, 2,
            "each word lists again"
        );
        drain(&rx);

        let carol = UserId::new();
        state.handle_sync_event(SyncEvent::DocumentMembersReady {
            id: None,
            asked: 2,
            document_id: doc,
            members: Vec::new(),
            signing_keys: vec![([2u8; 32], carol)],
        });
        let bob = UserId::new();
        state.handle_sync_event(SyncEvent::DocumentMembersReady {
            id: None,
            asked: 1,
            document_id: doc,
            members: Vec::new(),
            signing_keys: vec![([1u8; 32], bob)],
        });

        let keys = &state.documents[&doc].signing_keys;
        assert_eq!(keys.get(&[2u8; 32]), Some(&carol), "the newer answer holds");
        assert!(
            !keys.contains_key(&[1u8; 32]),
            "the crossed answer changes nothing"
        );
        assert!(
            !drain(&rx)
                .iter()
                .any(|e| matches!(e, LogicEvent::Rest { .. })),
            "a refresh of the directory alone answers no request"
        );
    }

    /// A server document opened over a live WS is only fully hydrated once its
    /// post-open catch-up replay (`MissingUpdates`) lands. The snapshot does not
    /// cover deltas committed after it, and a delta-only document has NO snapshot
    /// at all. So `DocumentReady`, which dismisses the UI loading overlay, must
    /// be DEFERRED to that first replay rather than fired at open. Firing at open
    /// would clear the overlay while the grid is still empty, exactly the
    /// 5-second empty-grid bug. The snapshot is still emitted at open.
    #[test]
    fn open_document_ready_deferred_to_initial_replay() {
        let (mut state, rx, doc, key, sk, _snap) = setup();
        let (ws_tx, _ws_rx) = ambre_bus::conduit::unbounded();
        state.set_mock_ws(ws_tx);
        let snapshot_id = SnapshotId::new();
        let blob = snapshot_blob(doc, snapshot_id, b"snap", &key, &sk);

        state.handle_sync_event(SyncEvent::OpenDocumentReady {
            document_id: doc,
            doc_key: key,
            snapshot_payload: Some(blob),
            snapshot_id,
            covered_seq: Seq(0),
            held_snapshot: None,
            doc_type: DocType::Trame,
            server_created: true,
            signing_keys: Vec::new(),
            role: Role::Owner,
            rotation_owed: false,
            links: Vec::new(),
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

    /// `AUTHOR-022`: an open that holds back its latest snapshot starts from
    /// the reference, and checks the held one only once the replay is done.
    #[test]
    fn a_held_snapshot_is_checked_once_the_replay_is_done() {
        let (mut state, rx, doc, key, sk, _snap) = setup();
        let (ws_tx, _ws_rx) = ambre_bus::conduit::unbounded();
        state.set_mock_ws(ws_tx);
        let reference = SnapshotId::new();
        let held = SnapshotId::new();
        state.handle_sync_event(SyncEvent::OpenDocumentReady {
            document_id: doc,
            doc_key: key.clone(),
            snapshot_payload: Some(snapshot_blob(doc, reference, b"reference", &key, &sk)),
            snapshot_id: reference,
            covered_seq: Seq(3),
            held_snapshot: Some(HeldSnapshot {
                snapshot_id: held,
                covered_seq: Seq(9),
                blob: snapshot_blob(doc, held, b"latest", &key, &sk),
            }),
            doc_type: DocType::Trame,
            server_created: true,
            signing_keys: Vec::new(),
            role: Role::Owner,
            rotation_owed: false,
            links: Vec::new(),
        });
        let at_open = drain(&rx);
        assert!(
            !at_open
                .iter()
                .any(|e| matches!(e, LogicEvent::PendingSnapshot { .. })),
            "the held snapshot was checked before the replay: {at_open:?}"
        );
        assert!(state.documents[&doc].known_snapshots.contains(&held));

        state.handle_ws_message(WsServerMsg::MissingUpdates {
            document_id: doc,
            updates: Vec::new(),
        });
        let after_replay = drain(&rx);
        assert!(
            after_replay.iter().any(|e| matches!(
                e,
                LogicEvent::PendingSnapshot { payload, covered_seq: 9, purpose: SnapshotPurpose::Open, .. }
                    if payload == b"latest"
            )),
            "the held snapshot is checked after the replay: {after_replay:?}"
        );
        // A delta past the held snapshot waits on it, so the open is ready
        // only once the verdict is in (`AUTHOR-025`).
        assert!(
            !after_replay
                .iter()
                .any(|e| matches!(e, LogicEvent::DocumentReady { .. })),
            "the open was ready before the held snapshot's verdict: {after_replay:?}"
        );
    }

    /// A local-only (not yet server-created) or offline open has no replay to
    /// wait for, so `DocumentReady` fires immediately at open. Otherwise the
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
            held_snapshot: None,
            doc_type: DocType::Trame,
            server_created: true,
            signing_keys: Vec::new(),
            role: Role::Owner,
            rotation_owed: false,
            links: Vec::new(),
        });
        assert_eq!(
            ready_count(&drain(&rx), doc),
            1,
            "offline open signals readiness immediately (no replay to await)"
        );
    }

    /// Extract the role carried by the single `DocumentReady` for `doc`.
    fn ready_role(events: &[LogicEvent], doc: DocumentId) -> Option<Role> {
        events.iter().find_map(|e| match e {
            LogicEvent::DocumentReady { document_id, role } if *document_id == doc => Some(*role),
            _ => None,
        })
    }

    /// The role resolved at open must reach Logic on `DocumentReady`
    /// (`architecture`) -- on BOTH emission paths. The deferred one is the
    /// fragile case. The event fires from the replay handler, long after the
    /// open, so the role has to survive on `SyncDocumentState` rather than ride
    /// the call stack.
    #[test]
    fn document_ready_carries_the_role_on_both_paths() {
        // Deferred path: server document over a live WS, ready only after replay.
        let (mut state, rx, doc, key, _sk, _snap) = setup();
        let (ws_tx, _ws_rx) = ambre_bus::conduit::unbounded();
        state.set_mock_ws(ws_tx);
        state.handle_sync_event(SyncEvent::OpenDocumentReady {
            document_id: doc,
            doc_key: key,
            snapshot_payload: None,
            snapshot_id: SnapshotId::genesis(doc),
            covered_seq: Seq(0),
            held_snapshot: None,
            doc_type: DocType::Trame,
            server_created: true,
            signing_keys: Vec::new(),
            role: Role::Viewer,
            rotation_owed: false,
            links: Vec::new(),
        });
        let _ = drain(&rx);
        state.handle_ws_message(WsServerMsg::MissingUpdates {
            document_id: doc,
            updates: Vec::new(),
        });
        assert_eq!(
            ready_role(&drain(&rx), doc),
            Some(Role::Viewer),
            "the role resolved at open must survive to the deferred ready"
        );

        // Immediate path: offline open, ready fires at open.
        let (mut state, rx, doc, key, _sk, _snap) = setup();
        state.handle_sync_event(SyncEvent::OpenDocumentReady {
            document_id: doc,
            doc_key: key,
            snapshot_payload: None,
            snapshot_id: SnapshotId::genesis(doc),
            covered_seq: Seq(0),
            held_snapshot: None,
            doc_type: DocType::Trame,
            server_created: true,
            signing_keys: Vec::new(),
            role: Role::Editor,
            rotation_owed: false,
            links: Vec::new(),
        });
        assert_eq!(ready_role(&drain(&rx), doc), Some(Role::Editor));
    }

    /// A completed snapshot upload adopts the server-assigned snapshot id as the
    /// document's current parent and registers it as known. Later deltas
    /// parented on it are then delivered, not quarantined.
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

    /// On the host (no event queue), `SendSnapshot` seals the payload without
    /// attempting any network upload and leaves the document state untouched.
    #[test]
    fn send_snapshot_on_host_path_seals_without_upload() {
        let (mut state, _rx, doc, _key, _sk, snap) = setup();
        // Session keys are required to reach the seal step.
        state.signing_key = Some(MasterSeed::generate().derive_signing_key());

        state.handle_send_snapshot(doc, b"crdt-snapshot-state".to_vec(), Seq(0));

        // No event_tx in this setup: the upload path is skipped, the current
        // snapshot id is unchanged (no SnapshotUploaded round-trip).
        assert_eq!(state.documents[&doc].current_snapshot_id, snap);
    }

    /// A snapshot a peer announced is not a known parent until its contents
    /// reach the Logic thread (`SYNC-030`): the announcement alone releases
    /// nothing.
    #[test]
    fn an_announced_snapshot_releases_nothing_until_fetched() {
        let (mut state, rx, doc, key, sk, _snap) = setup();
        let announced = SnapshotId::new();
        state.receive_update(Seq(1), make_update(doc, announced, 1, b"after", &key, &sk));
        state.handle_ws_message(WsServerMsg::SnapshotAvailable {
            document_id: doc,
            snapshot_id: announced,
            covered_seq: Seq(0),
        });
        assert!(
            drain(&rx).is_empty(),
            "nothing delivered on the announcement"
        );
        assert!(!state.documents[&doc].known_snapshots.contains(&announced));

        let blob = snapshot_blob(doc, announced, b"base", &key, &sk);
        state.handle_sync_event(SyncEvent::SnapshotFetched {
            document_id: doc,
            snapshot_id: announced,
            covered_seq: Seq(0),
            blob: Some(blob),
        });
        let events = drain(&rx);
        assert!(
            matches!(&events[..], [
                LogicEvent::PendingSnapshot {
                    payload: base,
                    purpose: SnapshotPurpose::Confirm,
                    ..
                },
                LogicEvent::RemoteUpdate { delta: after, .. },
            ] if base == b"base" && after.payload == b"after"),
            "the snapshot reaches Logic to check before what rests on it: {events:?}"
        );
    }

    /// `AUTHOR-021`, `AUTHOR-022`: the snapshot an open held back is checked
    /// once the replay is done. It is adopted when it matches and every
    /// operation only it holds resolves to its author; otherwise the document
    /// stays on its reference.
    #[test]
    fn a_held_snapshot_is_adopted_only_when_its_extras_resolve_to_its_author() {
        let nonce = ambre_core::SessionNonce([7; 16]);
        let mallory = MasterSeed::generate().derive_signing_key();
        for (signer, verdict_is_match, adopted) in [
            (true, true, true),
            (false, true, false),
            (true, false, false),
        ] {
            let (mut state, rx, doc, key, sk, reference) = setup();
            let held = SnapshotId::new();
            let blob = snapshot_blob(doc, held, b"base", &key, &sk);
            state.documents.get_mut(&doc).expect("open").held_snapshot = Some(HeldSnapshot {
                snapshot_id: held,
                covered_seq: Seq(7),
                blob,
            });
            state.check_held_snapshot(doc);
            assert!(matches!(
                &drain(&rx)[..],
                [LogicEvent::PendingSnapshot { payload, covered_seq: 7, purpose: SnapshotPurpose::Open, .. }]
                    if payload == b"base"
            ));

            let author = if signer { &sk } else { &mallory };
            let (peer, entry) =
                ambre_crypto::peer::session_identity(author, UserId::new(), doc, nonce);
            let verdict = if verdict_is_match {
                SnapshotVerdict::Match {
                    extras: vec![(peer, entry.to_bytes())],
                }
            } else {
                SnapshotVerdict::Mismatch
            };
            state.handle_snapshot_verdict(doc, held, SnapshotPurpose::Open, verdict);
            let events = drain(&rx);
            // Either way the open that waited on the verdict is ready, after
            // the adopted snapshot if any (`AUTHOR-025`).
            if adopted {
                assert!(matches!(
                    &events[..],
                    [
                        LogicEvent::RemoteSnapshot { payload, covered_seq: 7, .. },
                        LogicEvent::DocumentReady { .. },
                    ] if payload == b"base"
                ));
                assert_eq!(state.documents[&doc].current_snapshot_id, held);
            } else {
                assert!(
                    matches!(&events[..], [LogicEvent::DocumentReady { .. }]),
                    "a refused snapshot reached Logic: {events:?}"
                );
                assert_eq!(state.documents[&doc].current_snapshot_id, reference);
            }
        }
    }

    /// `AUTHOR-016`, `AUTHOR-017`: a pending snapshot the Logic thread matched
    /// is confirmed to the relay. One that contradicts its deltas is reported
    /// instead, signed over its author, and one left unchecked is neither.
    #[tokio::test]
    async fn a_checked_snapshot_is_confirmed_or_reported_to_the_relay() {
        let base = "https://relay.test";
        for verdict in [
            SnapshotVerdict::Match { extras: Vec::new() },
            SnapshotVerdict::Mismatch,
            SnapshotVerdict::Unchecked,
        ] {
            let (mut state, rx, doc, key, author, _) = setup();
            let reporter = MasterSeed::generate().derive_signing_key();
            state.signing_key = Some(reporter.clone());
            let pending = SnapshotId::new();
            let blob = snapshot_blob(doc, pending, b"base", &key, &author);
            state.adopt_fetched_snapshot(doc, pending, Seq(3), Some(blob));
            drain(&rx);
            let expected = match verdict {
                SnapshotVerdict::Match { .. } => Some("confirm"),
                SnapshotVerdict::Mismatch => Some("report"),
                SnapshotVerdict::Unchecked => None,
            };

            let vouch =
                state.handle_snapshot_verdict(doc, pending, SnapshotPurpose::Confirm, verdict);

            let Some(route) = expected else {
                assert_eq!(
                    vouch, None,
                    "an unchecked snapshot is neither confirmed nor reported"
                );
                continue;
            };
            let vouch = vouch.expect("a checked snapshot is answered");
            let transport = Scripted::new([Ok((204, serde_json::Value::Null))]);
            let auth = session("access-1", "refresh-1");
            send_vouch(&transport, &auth, base, doc, pending, &vouch)
                .await
                .expect("posted");
            let url = format!("{base}/api/documents/{doc}/snapshots/{pending}/{route}");
            assert_eq!(transport.calls(), [(http::Method::Post, url)]);
            if let Vouch::Report(signature) = vouch {
                let transcript = ambre_protocol::snapshot_report::report_transcript(
                    doc,
                    pending,
                    &author.verifying_key().to_bytes(),
                );
                let bytes: [u8; 64] = hex::decode(&signature).unwrap().try_into().unwrap();
                let signature = ed25519_dalek::Signature::from_bytes(&bytes);
                assert!(
                    reporter
                        .verifying_key()
                        .verify_strict(&transcript, &signature)
                        .is_ok()
                );
                let sent = transport.sent.borrow();
                let body: serde_json::Value =
                    serde_json::from_slice(&sent[0].body.as_ref().unwrap().1).unwrap();
                assert_eq!(body, serde_json::json!({ "signature": hex::encode(bytes) }));
            }
        }
    }

    /// `AUTHOR-021`: a snapshot its author uploads is confirmed by that
    /// author, under the id the relay gave it, once the relay holds its
    /// metadata. A step that fails confirms nothing.
    #[tokio::test]
    async fn an_uploaded_snapshot_is_confirmed_by_its_author_once_stored() {
        let base = "https://relay.test";
        let (_state, _rx, doc, key, sk, reference) = setup();
        let upload = || {
            let public_data = SnapshotPublicData {
                document_id: doc,
                pub_key: PublicKey::from_array(sk.verifying_key().to_bytes()),
                key_id: key.id(),
                snapshot_id: reference,
                parent_snapshot_proof: Vec::new(),
                parent_snapshot_update_clocks: Vec::new(),
            };
            SnapshotUpload {
                document_id: doc,
                author_pubkey: hex::encode(sk.verifying_key().to_bytes()),
                covered_seq: 3,
                blob: pipeline::snapshot_blob(b"base", public_data, &key, &sk).unwrap(),
            }
        };
        let link = serde_json::json!({ "upload_url": "https://store.test/o?sig=x", "s3_key": "k" });
        let given = SnapshotId::new();
        let steps = |stored: u16| {
            vec![
                (
                    http::Method::Post,
                    format!("{base}/api/documents/{doc}/snapshot/upload-url"),
                ),
                (http::Method::Put, "https://store.test/o?sig=x".to_owned()),
                (
                    http::Method::Post,
                    format!("{base}/api/documents/{doc}/snapshot"),
                ),
            ]
            .into_iter()
            .chain((stored == 200).then(|| {
                let url = format!("{base}/api/documents/{doc}/snapshots/{given}/confirm");
                (http::Method::Post, url)
            }))
            .collect::<Vec<_>>()
        };

        for stored in [200, 500] {
            let transport = Scripted::new([
                Ok((200, link.clone())),
                Ok((200, serde_json::Value::Null)),
                Ok((
                    stored,
                    serde_json::json!({ "snapshot_id": given.to_string() }),
                )),
                Ok((204, serde_json::Value::Null)),
            ]);
            let auth = session("access-1", "refresh-1");

            let uploaded = upload_snapshot(&transport, &auth, base, upload()).await;

            assert_eq!(uploaded.is_ok(), stored == 200, "{uploaded:?}");
            if stored == 200 {
                assert_eq!(uploaded, Ok(given));
            }
            assert_eq!(transport.calls(), steps(stored));
            let sent = transport.sent.borrow();
            assert!(
                sent[1].bearer.is_none(),
                "the presigned link carries no session token"
            );
        }
    }

    /// A snapshot that does not open, or never arrived, stays unknown and is
    /// reported (`SYNC-030`); a blob sealed for another document is refused.
    #[test]
    fn a_snapshot_that_does_not_open_stays_unknown() {
        let (mut state, rx, doc, key, sk, _snap) = setup();
        let announced = SnapshotId::new();
        let foreign = snapshot_blob(DocumentId::new(), announced, b"x", &key, &sk);
        for blob in [None, Some(b"garbage".to_vec()), Some(foreign)] {
            state.handle_sync_event(SyncEvent::SnapshotFetched {
                document_id: doc,
                snapshot_id: announced,
                covered_seq: Seq(0),
                blob,
            });
        }
        let events = drain(&rx);
        assert_eq!(events.len(), 3);
        assert!(
            events
                .iter()
                .all(|e| matches!(e, LogicEvent::SyncWarning { .. }))
        );
        assert!(!state.documents[&doc].known_snapshots.contains(&announced));
    }

    /// The quarantine asks for the missing history once per gap, however many
    /// arrivals follow past the threshold (`SYNC-031`).
    #[test]
    fn the_quarantine_asks_once_per_gap() {
        let (mut state, _rx, doc, key, sk, _snap) = setup();
        let (ws_tx, ws_rx) = ambre_bus::conduit::unbounded();
        state.set_mock_ws(ws_tx);
        let parent = SnapshotId::new();
        for clock in 1..=(QUARANTINE_ALERT as u64 + 50) {
            state.receive_update(Seq(clock), make_update(doc, parent, clock, b"x", &key, &sk));
        }
        let asks = |rx: &ambre_bus::conduit::Receiver<Vec<u8>>| {
            drain_ws(rx)
                .iter()
                .filter(|m| matches!(m, WsClientMsg::RequestMissing { .. }))
                .count()
        };
        assert_eq!(asks(&ws_rx), 1, "one request for the gap");
        // The gap closes; the next one asks again.
        state.snapshot_known(doc, parent);
        let other = SnapshotId::new();
        // The relay numbers every delta once, so the next gap takes new numbers.
        let first = QUARANTINE_ALERT as u64 + 50;
        for clock in 1..=(QUARANTINE_ALERT as u64) {
            let update = make_update(doc, other, clock, b"x", &key, &sk);
            state.receive_update(Seq(first + clock), update);
        }
        assert_eq!(asks(&ws_rx), 1, "one request for the next gap");
    }

    /// Each update sent, as its clock, the key it names and its plaintext.
    fn sent_updates(
        rx: &ambre_bus::conduit::Receiver<Vec<u8>>,
        keys: &[&DocumentKey],
    ) -> Vec<(u64, KeyId, Vec<u8>)> {
        opened_updates(drain_ws(rx), keys)
    }

    /// Each update among `msgs`, as its clock, the key it names and its
    /// plaintext.
    fn opened_updates(msgs: Vec<WsClientMsg>, keys: &[&DocumentKey]) -> Vec<(u64, KeyId, Vec<u8>)> {
        msgs.into_iter()
            .filter_map(|msg| match msg {
                WsClientMsg::Update(u) => {
                    let key = keys.iter().find(|k| k.id() == u.public_data.key_id)?;
                    let body = pipeline::open_update(&u, key).expect("opens");
                    let delta = pipeline::split_update_body(body).expect("splits").delta;
                    Some((u.public_data.clock.0, u.public_data.key_id, delta))
                }
                _ => None,
            })
            .collect()
    }

    /// The signature of each update among `msgs`, in order.
    fn signatures(msgs: &[WsClientMsg]) -> Vec<Signature> {
        msgs.iter()
            .filter_map(|msg| match msg {
                WsClientMsg::Update(u) => Some(u.signature),
                _ => None,
            })
            .collect()
    }

    /// `SYNC-147`: another session of this user took clock 1. The relay
    /// refuses this session's delta 1 and drops delta 2 behind it. The answer
    /// names the refused delta, then both leave again in order from clock 2.
    /// A late refusal of delta 2's first seal is answered and sends nothing.
    #[test]
    fn a_delta_refused_for_its_clock_leaves_again_with_every_later_one() {
        let (mut state, _rx, doc, key, sk, _snap) = setup();
        state.signing_key = Some(sk);
        let (ws_tx, ws_rx) = ambre_bus::conduit::unbounded();
        state.set_mock_ws(ws_tx);
        state.handle_send_update(doc, b"a".to_vec());
        state.handle_send_update(doc, b"b".to_vec());
        let first = signatures(&drain_ws(&ws_rx));
        assert_eq!(first.len(), 2);

        state.handle_ws_message(WsServerMsg::ClockRefused {
            document_id: doc,
            clock: Clock(1),
            expected: Clock(2),
            signature: first[0],
        });
        let mut msgs = drain_ws(&ws_rx);
        assert!(
            matches!(msgs.first(), Some(WsClientMsg::ClockResync { signature, .. }) if *signature == first[0]),
            "the answer leaves first: {msgs:?}"
        );
        let resent = opened_updates(msgs.split_off(1), &[&key]);
        assert_eq!(
            resent,
            vec![(2, key.id(), b"a".to_vec()), (3, key.id(), b"b".to_vec())]
        );

        state.handle_ws_message(WsServerMsg::ClockRefused {
            document_id: doc,
            clock: Clock(2),
            expected: Clock(2),
            signature: first[1],
        });
        let late = drain_ws(&ws_rx);
        assert!(
            matches!(late.as_slice(), [WsClientMsg::ClockResync { signature, .. }] if *signature == first[1]),
            "a late refusal sends its answer alone: {late:?}"
        );
    }

    /// `SYNC-148`: the echo of another session's delta, under this user's key
    /// and at the clock of a delta this session sent, confirms nothing. The
    /// echo of this session's own delta does.
    #[test]
    fn the_echo_of_another_sessions_delta_confirms_nothing_here() {
        let (mut state, _rx, doc, key, sk, snap) = setup();
        state.signing_key = Some(sk.clone());
        let (ws_tx, ws_rx) = ambre_bus::conduit::unbounded();
        state.set_mock_ws(ws_tx);
        state.handle_send_update(doc, b"a".to_vec());
        let ours = drain_ws(&ws_rx);
        assert_eq!(state.documents[&doc].rekey.sent_len(), 1);

        let theirs = make_update(doc, snap, 1, b"other session", &key, &sk);
        state.receive_update(Seq(1), theirs);
        assert_eq!(
            state.documents[&doc].rekey.sent_len(),
            1,
            "still unconfirmed"
        );

        let Some(WsClientMsg::Update(echo)) = ours.into_iter().next() else {
            panic!("an update left");
        };
        state.receive_update(Seq(2), echo);
        assert_eq!(state.documents[&doc].rekey.sent_len(), 0);
    }

    /// A frame refused while a rotation is owed waits for the rotation, then is
    /// sealed again at its own clock under the new key. An edit made meanwhile
    /// takes the next clock, and a late refusal of the first sealing sends
    /// nothing (`SYNC-128`, `SYNC-129`).
    #[test]
    fn a_frame_refused_for_an_owed_rotation_is_sent_again_at_its_clock() {
        let (mut state, _rx, doc, k1, sk, _snap) = setup();
        state.signing_key = Some(sk);
        let (ws_tx, ws_rx) = ambre_bus::conduit::unbounded();
        state.set_mock_ws(ws_tx);

        state.handle_send_update(doc, b"a".to_vec());
        assert_eq!(
            sent_updates(&ws_rx, &[&k1]),
            vec![(1, k1.id(), b"a".to_vec())]
        );

        state.handle_ws_message(WsServerMsg::KeyRefused {
            document_id: doc,
            key_id: k1.id(),
            clock: Clock(1),
        });
        state.handle_send_update(doc, b"b".to_vec());
        assert!(drain_ws(&ws_rx).is_empty(), "nothing leaves before the key");

        let k2 = DocumentKey::generate();
        let link = pipeline::seal_link(doc, &k1, &k2, state.signing_key.as_ref().unwrap()).unwrap();
        state.handle_sync_event(SyncEvent::DebtSettled {
            document_id: doc,
            new_doc_key: k2.clone(),
            link,
        });
        assert_eq!(
            sent_updates(&ws_rx, &[&k1, &k2]),
            vec![(1, k2.id(), b"a".to_vec()), (2, k2.id(), b"b".to_vec())]
        );

        state.handle_ws_message(WsServerMsg::KeyRefused {
            document_id: doc,
            key_id: k1.id(),
            clock: Clock(1),
        });
        assert!(drain_ws(&ws_rx).is_empty(), "a late refusal is stale");
    }

    /// A refusal that names a key older than the current one is answered at
    /// once. The frame is sealed again under the current key, at its clock
    /// (`SYNC-129`).
    #[test]
    fn a_frame_refused_under_an_older_key_is_sent_again_at_once() {
        let (mut state, _rx, doc, k1, sk, _snap) = setup();
        state.signing_key = Some(sk);
        let (ws_tx, ws_rx) = ambre_bus::conduit::unbounded();
        state.set_mock_ws(ws_tx);

        state.handle_send_update(doc, b"a".to_vec());
        let k2 = DocumentKey::generate();
        state.handle_sync_event(SyncEvent::KeyRotated {
            document_id: doc,
            new_doc_key: k2.clone(),
            links: Vec::new(),
        });
        let _ = drain_ws(&ws_rx);

        state.handle_ws_message(WsServerMsg::KeyRefused {
            document_id: doc,
            key_id: k1.id(),
            clock: Clock(1),
        });
        assert_eq!(
            sent_updates(&ws_rx, &[&k1, &k2]),
            vec![(1, k2.id(), b"a".to_vec())]
        );
    }

    /// The rotation notice closes nothing (`SYNC-081`), and taking on the new
    /// key installs it and caches the previous one (`SYNC-080`). The document
    /// stays open throughout, and no departure reaches the Logic thread.
    #[test]
    fn a_rotation_installs_the_new_key_and_keeps_the_document_open() {
        let (mut state, rx, doc, k1, sk, snap) = setup();
        state.handle_ws_message(WsServerMsg::KeyRotationRequired { document_id: doc });
        assert!(state.documents.contains_key(&doc));

        let k2 = DocumentKey::generate();
        state.handle_sync_event(SyncEvent::KeyRotated {
            document_id: doc,
            new_doc_key: k2.clone(),
            links: Vec::new(),
        });
        let doc_state = &state.documents[&doc];
        assert_eq!(doc_state.doc_key.id(), k2.id());
        let cached: Vec<_> = doc_state.old_keys.iter().map(DocumentKey::id).collect();
        assert_eq!(cached, vec![k1.id()]);
        let late = make_update(doc, snap, 1, b"late", &k1, &sk);
        assert_eq!(
            doc_state.open_signed_update(&late, Seq(1)).unwrap().payload,
            b"late"
        );
        let departed = drain(&rx).into_iter().any(|e| {
            matches!(
                e,
                LogicEvent::MemberRevoked { .. } | LogicEvent::ApiError { .. }
            )
        });
        assert!(!departed, "a rotation reports no departure");
    }

    /// The cache of previous keys holds ten (`SYNC-044`). Eleven rotations
    /// keep the ten latest previous keys, newest first, and the first key
    /// no longer opens anything without a link.
    #[test]
    fn the_previous_key_cache_holds_ten() {
        let (mut state, _rx, doc, first, _sk, _snap) = setup();
        let mut held = vec![first.clone()];
        for _ in 0..11 {
            let next = DocumentKey::generate();
            state.handle_sync_event(SyncEvent::KeyRotated {
                document_id: doc,
                new_doc_key: next.clone(),
                links: Vec::new(),
            });
            held.push(next);
        }
        let doc_state = &state.documents[&doc];
        let cached: Vec<_> = doc_state.old_keys.iter().map(DocumentKey::id).collect();
        let expected: Vec<_> = held[1..11].iter().rev().map(DocumentKey::id).collect();
        assert_eq!(cached, expected);
        assert!(doc_state.key_named(first.id()).is_none());
    }

    /// A copy unwraps against the exchange key of whoever wrapped it, never
    /// against its holder's (`SYNC-034`). A row from an older relay names no
    /// wrapper: it reads as self-wrapped, so a copy a peer wrapped fails.
    #[test]
    fn a_copy_unwraps_against_its_wrapper_and_never_its_holder() {
        let me = UserId::new();
        let my_xk = ambre_crypto::MasterSeed::from_array([3; 32]).derive_exchange_key();
        let peer_xk = ambre_crypto::MasterSeed::from_array([4; 32]).derive_exchange_key();
        let doc_key = DocumentKey::generate();
        // The row `wrapper` wrapped, its wrapper field rewritten to `named`.
        let unwrapped = |wrapper: &x25519_dalek::StaticSecret, named: Option<String>| {
            let mut row = member_row(me, &my_xk, wrapper, &doc_key, "editor");
            if let Some(named) = named {
                row.wrapper_exchange_pub_key = named;
            }
            SyncState::unwrap_document_key(&my_xk, &[row]).map(|key| key.id())
        };
        let mine = hex::encode(x25519_dalek::PublicKey::from(&my_xk).to_bytes());

        assert_eq!(unwrapped(&peer_xk, None), Ok(doc_key.id()));
        assert!(unwrapped(&peer_xk, Some(mine)).is_err());
        assert!(unwrapped(&peer_xk, Some(String::new())).is_err());
        assert_eq!(unwrapped(&my_xk, Some(String::new())), Ok(doc_key.id()));
    }

    /// Every refusal of a live frame reaches the Logic thread as one warning
    /// naming the document (`SYNC-091`). Its message names the check that
    /// failed, so a missed re-key reads apart from a corrupt frame.
    #[test]
    fn every_refused_frame_reaches_logic_as_a_warning() {
        let (mut state, rx, doc, key, sk, snap) = setup();
        let mut unsigned = make_update(doc, snap, 1, b"a", &key, &sk);
        unsigned.signature.0 = [0; 64];
        let stranger = DocumentKey::generate();
        let under_stranger = make_update(doc, snap, 2, b"b", &stranger, &sk);
        let mut corrupt = make_update(doc, snap, 3, b"c", &key, &sk);
        corrupt.commitment_tag.0 = [0; 32];

        let mut messages = Vec::new();
        for (seq, update) in [(1, unsigned), (2, under_stranger), (3, corrupt)] {
            state.receive_update(Seq(seq), update);
            match drain(&rx).as_slice() {
                [
                    LogicEvent::SyncWarning {
                        document_id,
                        code: SyncWarningCode::DecryptFailed,
                        message,
                    },
                ] if *document_id == doc => messages.push(message.clone()),
                other => panic!("one warning per refused frame, got {other:?}"),
            }
        }
        assert_ne!(messages[0], messages[1]);
        assert_ne!(messages[1], messages[2]);
    }

    /// A writer that opens a document owing a rotation sends nothing before it
    /// pays; a reader records no debt (`SYNC-128`).
    #[test]
    fn a_writer_opening_an_owed_document_pays_before_its_first_frame() {
        for (role, held) in [(Role::Editor, true), (Role::Viewer, false)] {
            let (mut state, _rx, _doc, _k, sk, _snap) = setup();
            state.signing_key = Some(sk);
            let (ws_tx, ws_rx) = ambre_bus::conduit::unbounded();
            state.set_mock_ws(ws_tx);
            let doc = DocumentId::new();
            state.handle_sync_event(SyncEvent::OpenDocumentReady {
                document_id: doc,
                doc_key: DocumentKey::generate(),
                snapshot_payload: None,
                snapshot_id: SnapshotId::genesis(doc),
                covered_seq: Seq(0),
                held_snapshot: None,
                doc_type: DocType::Trame,
                server_created: true,
                signing_keys: Vec::new(),
                role,
                rotation_owed: true,
                links: Vec::new(),
            });
            let _ = drain_ws(&ws_rx);
            state.session_nonces.insert(doc, TEST_NONCE);
            state.handle_send_update(doc, b"a".to_vec());
            assert_eq!(drain_ws(&ws_rx).is_empty(), held, "{role:?}");
        }
    }

    /// Holds `SYNC-024`: each update takes a clock above every one this client
    /// used. A reopened document starts again from the echoes of its own
    /// frames, never from one.
    #[test]
    fn each_update_takes_a_clock_above_every_one_used() {
        let (mut state, _rx, doc, key, sk, snap) = setup();
        state.signing_key = Some(sk.clone());
        let (ws_tx, ws_rx) = ambre_bus::conduit::unbounded();
        state.set_mock_ws(ws_tx);

        state.handle_send_update(doc, b"a".to_vec());
        state.handle_send_update(doc, b"b".to_vec());
        let clocks: Vec<u64> = sent_updates(&ws_rx, &[&key])
            .into_iter()
            .map(|(clock, ..)| clock)
            .collect();
        assert_eq!(clocks, [1, 2]);

        // A reload forgets the counter; the replay echoes the two frames back.
        state.documents.insert(
            doc,
            SyncDocumentState::new(doc, key.clone(), DocType::Trame, true),
        );
        state.receive_update(Seq(1), make_update(doc, snap, 1, b"a", &key, &sk));
        state.receive_update(Seq(2), make_update(doc, snap, 2, b"b", &key, &sk));
        state.handle_send_update(doc, b"c".to_vec());
        assert_eq!(
            sent_updates(&ws_rx, &[&key]),
            vec![(3, key.id(), b"c".to_vec())]
        );
    }

    /// Holds `SYNC-085`: an open sends a history request and no update, with
    /// or without a snapshot to read.
    #[test]
    fn an_open_sends_no_update() {
        for with_snapshot in [false, true] {
            let (mut state, _rx, _doc, _key, sk, _snap) = setup();
            state.signing_key = Some(sk.clone());
            let (ws_tx, ws_rx) = ambre_bus::conduit::unbounded();
            state.set_mock_ws(ws_tx);
            let doc = DocumentId::new();
            let key = DocumentKey::generate();
            let snapshot_id = SnapshotId::new();
            let blob = with_snapshot.then(|| snapshot_blob(doc, snapshot_id, b"s", &key, &sk));
            state.handle_sync_event(SyncEvent::OpenDocumentReady {
                document_id: doc,
                doc_key: key,
                snapshot_payload: blob,
                snapshot_id: if with_snapshot {
                    snapshot_id
                } else {
                    SnapshotId::genesis(doc)
                },
                covered_seq: Seq(u64::from(with_snapshot)),
                held_snapshot: None,
                doc_type: DocType::Trame,
                server_created: true,
                signing_keys: Vec::new(),
                role: Role::Owner,
                rotation_owed: false,
                links: Vec::new(),
            });
            let sent = drain_ws(&ws_rx);
            assert!(
                sent.iter()
                    .any(|m| matches!(m, WsClientMsg::RequestMissing { .. })),
                "the open asks for the history"
            );
            assert!(
                !sent.iter().any(|m| matches!(m, WsClientMsg::Update(_))),
                "the open sent an update: {sent:?}"
            );
        }
    }

    /// Holds `SYNC-087`: a reconnect replay that overlaps a broadcast already
    /// received delivers the overlap once. A copy held in quarantine is not
    /// held a second time either.
    #[test]
    fn a_reconnect_replay_applies_nothing_twice() {
        let (mut state, rx, doc, key, sk, snap) = setup();
        let update = |seq: u64, parent| SeqUpdate {
            seq: Seq(seq),
            update: make_update(doc, parent, seq, &[seq as u8], &key, &sk),
        };
        state.handle_ws_message(WsServerMsg::Update {
            seq: Seq(3),
            update: update(3, snap).update,
        });
        state.handle_ws_message(WsServerMsg::MissingUpdates {
            document_id: doc,
            updates: vec![update(2, snap), update(3, snap)],
        });
        let delivered: Vec<Vec<u8>> = drain(&rx)
            .into_iter()
            .flat_map(|e| match e {
                LogicEvent::RemoteUpdate { delta, .. } => vec![delta.payload],
                LogicEvent::RemoteUpdateBatch { deltas, .. } => {
                    deltas.into_iter().map(|d| d.payload).collect()
                }
                _ => Vec::new(),
            })
            .collect();
        assert_eq!(delivered, [vec![3], vec![2]]);

        let parent = SnapshotId::new();
        state.receive_update(Seq(9), update(9, parent).update);
        state.receive_update(Seq(9), update(9, parent).update);
        assert_eq!(state.documents[&doc].quarantine_len(), 1);
    }

    /// `SYNC-027`: a replay of several deltas reaches the Logic thread as one
    /// batch, in the order of their numbers, and as no single event.
    #[test]
    fn a_history_replay_reaches_logic_as_one_batch() {
        let (mut state, rx, doc, key, sk, snap) = setup();
        let updates = (1..=5)
            .map(|seq| SeqUpdate {
                seq: Seq(seq),
                update: make_update(doc, snap, seq, &[seq as u8], &key, &sk),
            })
            .collect();
        state.handle_ws_message(WsServerMsg::MissingUpdates {
            document_id: doc,
            updates,
        });
        let deltas: Vec<Vec<Vec<u8>>> = drain(&rx)
            .into_iter()
            .filter_map(|e| match e {
                LogicEvent::RemoteUpdate { .. } => Some(Vec::new()),
                LogicEvent::RemoteUpdateBatch { deltas, .. } => {
                    Some(deltas.into_iter().map(|d| d.payload).collect())
                }
                _ => None,
            })
            .collect();
        assert_eq!(deltas, [vec![vec![1], vec![2], vec![3], vec![4], vec![5]]]);
    }

    /// `SYNC-025`: after a reload, the relay's echo of this client's own
    /// update carries its clock forward. The next frame takes the clock after
    /// it. A peer's higher clock moves nothing.
    #[test]
    fn a_reload_takes_its_clock_from_the_echo_of_its_own_update() {
        let (mut state, _rx, doc, key, sk, snap) = setup();
        state.signing_key = Some(sk.clone());
        let (ws_tx, ws_rx) = ambre_bus::conduit::unbounded();
        state.set_mock_ws(ws_tx);
        let peer = MasterSeed::generate().derive_signing_key();
        state.receive_update(Seq(1), make_update(doc, snap, 41, b"mine", &key, &sk));
        state.receive_update(Seq(2), make_update(doc, snap, 99, b"peer", &key, &peer));
        assert_eq!(state.documents[&doc].local_clock, 41);

        state.handle_command(SyncCommand::SendUpdate {
            document_id: doc,
            payload: b"next".to_vec(),
            summary: None,
        });
        let clocks: Vec<u64> = drain_ws(&ws_rx)
            .into_iter()
            .filter_map(|m| match m {
                WsClientMsg::Update(u) => Some(u.public_data.clock.0),
                _ => None,
            })
            .collect();
        assert_eq!(clocks, [42]);
    }

    /// Holds `SYNC-032`: an update held in quarantine is not opened on arrival.
    /// One sealed under a key this client lacks would fail to open, and its
    /// arrival raises no failure.
    #[test]
    fn a_quarantined_update_is_not_opened_on_arrival() {
        let (mut state, rx, doc, _key, sk, _snap) = setup();
        let stranger = DocumentKey::generate();
        let update = make_update(doc, SnapshotId::new(), 1, b"x", &stranger, &sk);
        state.receive_update(Seq(1), update);
        assert_eq!(state.documents[&doc].quarantine_len(), 1);
        assert!(drain(&rx).is_empty(), "the arrival opened the update");
    }

    /// Holds `SYNC-079`: a revoke swaps the key and changes no content. The
    /// worker sends no delta or snapshot for it, tells the document nothing
    /// new, and what was written before still opens to the same bytes.
    #[test]
    fn a_revoke_changes_no_content() {
        let (mut state, rx, doc, k1, sk, snap) = setup();
        state.signing_key = Some(sk.clone());
        let (ws_tx, ws_rx) = ambre_bus::conduit::unbounded();
        state.set_mock_ws(ws_tx);
        let before = make_update(doc, snap, 1, b"hello", &k1, &sk);
        state.receive_update(Seq(1), before.clone());
        let _ = drain(&rx);

        let k2 = DocumentKey::generate();
        let link = pipeline::seal_link(doc, &k1, &k2, &sk).unwrap();
        state.handle_sync_event(SyncEvent::RevocationComplete {
            id: RequestId::next(),
            document_id: doc,
            new_doc_key: k2.clone(),
            link,
        });

        assert!(drain_ws(&ws_rx).is_empty(), "the revoke sent a frame");
        let content = drain(&rx).into_iter().find(|e| {
            matches!(
                e,
                LogicEvent::RemoteUpdate { .. }
                    | LogicEvent::RemoteUpdateBatch { .. }
                    | LogicEvent::RemoteSnapshot { .. }
            )
        });
        assert!(
            content.is_none(),
            "the revoke changed the content: {content:?}"
        );
        let doc_state = &state.documents[&doc];
        assert_eq!(doc_state.doc_key.id(), k2.id());
        assert_eq!(
            doc_state
                .open_signed_update(&before, Seq(1))
                .unwrap()
                .payload,
            b"hello"
        );
    }

    /// Holds `SYNC-097`: what the start held is released in the order it
    /// came. The opens subscribe in order, and the edits leave in order at
    /// rising clocks once the replay has caught the clock up.
    #[test]
    fn held_commands_are_released_in_order() {
        let (mut state, _rx, doc, key, sk, _snap) = setup();
        let (ws_tx, ws_rx) = ambre_bus::conduit::unbounded();
        state.set_mock_ws(ws_tx);

        let opens: Vec<DocumentId> = (0..3).map(|_| DocumentId::new()).collect();
        state.pending_opens = opens.iter().map(|d| (*d, DocType::Trame)).collect();
        state.flush_pending_opens();
        let subscribed: Vec<DocumentId> = drain_ws(&ws_rx)
            .into_iter()
            .filter_map(|m| match m {
                WsClientMsg::Subscribe { document_id } => Some(document_id),
                _ => None,
            })
            .collect();
        assert_eq!(subscribed, opens);

        // No session yet: the edits wait, interleaved with another document's.
        let other = opens[0];
        for payload in [b"a", b"b", b"c"] {
            state.handle_send_update(doc, payload.to_vec());
            state.handle_send_update(other, b"x".to_vec());
        }
        assert!(drain_ws(&ws_rx).is_empty());
        state.signing_key = Some(sk);
        state.handle_ws_message(WsServerMsg::MissingUpdates {
            document_id: doc,
            updates: Vec::new(),
        });
        assert_eq!(
            sent_updates(&ws_rx, &[&key]),
            vec![
                (1, key.id(), b"a".to_vec()),
                (2, key.id(), b"b".to_vec()),
                (3, key.id(), b"c".to_vec()),
            ]
        );
    }

    /// Each update sent, as the nonce and the delta its body carries.
    fn sent_bodies(
        rx: &ambre_bus::conduit::Receiver<Vec<u8>>,
        key: &DocumentKey,
    ) -> Vec<(SessionNonce, Vec<u8>)> {
        drain_ws(rx)
            .into_iter()
            .filter_map(|msg| match msg {
                WsClientMsg::Update(u) => {
                    let body = pipeline::open_update(&u, key).expect("opens");
                    let body = pipeline::split_update_body(body).expect("splits");
                    Some((body.nonce, body.delta))
                }
                _ => None,
            })
            .collect()
    }

    /// `AUTHOR-028`: an earlier session's unsent delta is sealed with that
    /// session's nonce, not the current one. A current edit beside it keeps
    /// the current nonce, and a refusal sends both again with their own.
    #[test]
    fn an_unsent_delta_is_sealed_with_its_sessions_nonce() {
        let (mut state, _rx, doc, k1, sk, _snap) = setup();
        state.signing_key = Some(sk);
        let (ws_tx, ws_rx) = ambre_bus::conduit::unbounded();
        state.set_mock_ws(ws_tx);
        let earlier = SessionNonce([7; 16]);

        state.handle_send_unsent(doc, b"old".to_vec(), earlier);
        state.handle_send_update(doc, b"new".to_vec());
        let sent = sent_bodies(&ws_rx, &k1);
        let [(old_nonce, old), (new_nonce, new)] = sent.as_slice() else {
            panic!("expected two updates, got {}", sent.len());
        };
        assert_eq!((*old_nonce, old.as_slice()), (earlier, b"old".as_slice()));
        assert_eq!(new.as_slice(), b"new");
        assert_ne!(*new_nonce, earlier, "a current edit took the earlier nonce");

        let k2 = DocumentKey::generate();
        state.handle_sync_event(SyncEvent::KeyRotated {
            document_id: doc,
            new_doc_key: k2.clone(),
            links: Vec::new(),
        });
        let _ = drain_ws(&ws_rx);
        state.handle_ws_message(WsServerMsg::KeyRefused {
            document_id: doc,
            key_id: k1.id(),
            clock: Clock(1),
        });
        assert_eq!(
            sent_bodies(&ws_rx, &k2),
            vec![(earlier, b"old".to_vec()), (*new_nonce, b"new".to_vec())],
            "the resend changed a nonce"
        );
    }

    /// `SYNC-065`: an edit made before the session is ready is held. It is
    /// sent, in order, once the session is up and the document's replay
    /// has landed.
    #[test]
    fn an_edit_before_the_session_is_held_then_sent() {
        let (mut state, _rx, doc, key, sk, _snap) = setup();
        let (ws_tx, ws_rx) = ambre_bus::conduit::unbounded();
        state.set_mock_ws(ws_tx);

        state.handle_send_update(doc, b"a".to_vec());
        state.handle_send_update(doc, b"b".to_vec());
        assert!(sent_bodies(&ws_rx, &key).is_empty(), "no session to sign");

        state.signing_key = Some(sk);
        state.handle_ws_message(WsServerMsg::MissingUpdates {
            document_id: doc,
            updates: Vec::new(),
        });
        let deltas: Vec<Vec<u8>> = sent_bodies(&ws_rx, &key)
            .into_iter()
            .map(|(_, delta)| delta)
            .collect();
        assert_eq!(deltas, [b"a".to_vec(), b"b".to_vec()]);
    }

    /// Holds `SYNC-023`: a frame that fails a check delivers none of its
    /// payload, and its neighbours in the same replay arrive whole.
    #[test]
    fn a_refused_frame_is_applied_in_no_part() {
        let (mut state, rx, doc, key, sk, snap) = setup();
        let mut forged = make_update(doc, snap, 2, b"forged", &key, &sk);
        forged.ciphertext.0[0] ^= 1;
        let mut unsigned = make_update(doc, snap, 3, b"unsigned", &key, &sk);
        unsigned.signature.0 = [0; 64];
        let updates = [
            (1, make_update(doc, snap, 1, b"one", &key, &sk)),
            (2, forged),
            (3, unsigned),
            (4, make_update(doc, snap, 4, b"four", &key, &sk)),
        ]
        .into_iter()
        .map(|(seq, update)| SeqUpdate {
            seq: Seq(seq),
            update,
        })
        .collect();
        state.handle_ws_message(WsServerMsg::MissingUpdates {
            document_id: doc,
            updates,
        });

        let mut payloads = Vec::new();
        let mut refused = 0;
        for event in drain(&rx) {
            match event {
                LogicEvent::RemoteUpdateBatch { deltas, .. } => {
                    payloads.extend(deltas.into_iter().map(|d| d.payload));
                }
                LogicEvent::RemoteUpdate { delta, .. } => payloads.push(delta.payload),
                LogicEvent::SyncWarning { code, .. } => {
                    assert_eq!(code, SyncWarningCode::DecryptFailed);
                    refused += 1;
                }
                _ => {}
            }
        }
        assert_eq!(payloads, [b"one".to_vec(), b"four".to_vec()]);
        assert_eq!(refused, 2);
    }

    /// `SYNC-069`: an update for a document with no open session cannot reach
    /// the relay, so it is refused aloud rather than dropped in silence.
    #[test]
    fn an_update_for_a_document_with_no_session_is_refused_aloud() {
        let (mut state, rx, _doc, _key, sk, _snap) = setup();
        state.signing_key = Some(sk);
        let unknown = DocumentId::new();

        state.handle_send_update(unknown, b"a channel message".to_vec());

        let events = drain(&rx);
        assert!(
            matches!(
                events.as_slice(),
                [LogicEvent::SyncWarning { document_id, code: SyncWarningCode::UpdateRefused, .. }]
                    if *document_id == unknown
            ),
            "one refusal naming the document, got {events:?}"
        );
    }

    /// A read reaches the last update delivered to the Logic thread. It stops
    /// short of an update held in quarantine: past it, the cursor would mark
    /// read a message never shown (the Murmure unread spec).
    #[test]
    fn a_read_reaches_the_last_update_delivered_and_no_further() {
        let (mut state, _rx, doc, key, sk, snap) = setup();
        assert_eq!(state.read_up_to(doc), None, "nothing delivered yet");
        assert_eq!(state.read_up_to(DocumentId::new()), None, "not open");

        state.receive_update(Seq(4), make_update(doc, snap, 1, b"shown", &key, &sk));
        assert_eq!(state.read_up_to(doc), Some(Seq(4)));

        let held = make_update(doc, SnapshotId::new(), 2, b"held", &key, &sk);
        state.receive_update(Seq(5), held);
        assert_eq!(state.read_up_to(doc), None, "a held update stays unread");
    }
}
