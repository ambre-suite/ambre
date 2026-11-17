use super::*;
use crate::pipeline::UpdateFrame;
use crate::rekey::SentFrame;
use ambre_core::FixedBytes;
use ambre_core::{Ms, Payload, TimeSpan};
use ambre_protocol::messages::Signature;

/// The delay before Sync sends again the first delta the relay failed to
/// sequence. Each later failure doubles it (`ambre-spec-budgets.md` section 8).
const RESEND_RETRY_BASE: TimeSpan<Ms> = TimeSpan::<Ms>::new(1_000);

/// The failures Sync sends again after. One more holds the document
/// (`ambre-spec-budgets.md` section 8, `SYNC-159`).
const RESEND_RETRY_LIMIT: ambre_core::Count<ambre_core::Failures> =
    ambre_core::Count::<ambre_core::Failures>::new(5);

/// Why a document holds its edits: the relay refused one, or failed to
/// sequence one past the retry budget. It prints the cause the warning reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HoldCause {
    /// A refusal the relay named.
    Refused(ambre_core::RefusalCode),
    /// The failures counted since the last echo.
    Failed(ambre_core::Count<ambre_core::Failures>),
}

impl std::fmt::Display for HoldCause {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Refused(code) => write!(f, "{code}"),
            Self::Failed(failure) => write!(f, "sequencing failed {failure} times"),
        }
    }
}

/// The delay before sending again after the `failure`-th failure since the
/// last echo, counted from one. Past the budget there is none.
fn resend_retry_delay(failure: ambre_core::Count<ambre_core::Failures>) -> Option<TimeSpan<Ms>> {
    let first = ambre_core::Count::<ambre_core::Failures>::new(1);
    (first..=RESEND_RETRY_LIMIT)
        .contains(&failure)
        .then(|| RESEND_RETRY_BASE.doubled(failure.minus(first)))
}

impl SyncState {
    // -- Causal quarantine (`ambre-spec-delta-order.md`) ------------

    /// Ingest one server-sequenced [`Update`]. If its parent snapshot is known,
    /// it is decrypted and delivered immediately. Otherwise it is held in
    /// quarantine until the parent snapshot arrives, then released in `clock`
    /// order by [`release_quarantine`].
    pub(crate) fn receive_update(&mut self, seq: Seq<Relay>, update: Update) {
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
        seq: Seq<Relay>,
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
                seq.get()
            );
            return None;
        }
        doc_state.last_seq = seq;
        // A peer's head past the log this client read asks for the log
        // again (`SYNC-207`). Our own head names nothing new.
        if !authored_by_us {
            self.note_signed_head(&update);
        }
        let doc_state = self.documents.get_mut(&document_id)?;
        if authored_by_us {
            doc_state.local_clock = doc_state.local_clock.max(update.public_data.clock);
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
            // A walk owed holds every delta, whoever signed it (`SYNC-232`).
            if !matches!(doc_state.gate, crate::channels::LogGate::Open)
                || !doc_state.writers.admits(&update.public_data.pub_key)
            {
                self.hold_unvouched(document_id, [(seq, update)]);
                return None;
            }
            let opened = doc_state.open_signed_update(&update, seq);
            tracing::info!(
                "[Sync->Logic] decrypted update doc={document_id} seq={} clock={} size={}",
                seq.get(),
                update.public_data.clock.get(),
                opened.as_ref().map_or(0, |d| d.payload.len().get()),
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
            seq.get(),
        );
        self.enforce_quarantine_bounds(document_id);
        None
    }

    /// Keep a document's quarantine within [`QUARANTINE_LIMIT`]. Request the
    /// missing history at the soft threshold, and drop the oldest update (by
    /// `clock`) past the hard cap, raising a `SyncWarning`.
    pub(crate) fn enforce_quarantine_bounds(&mut self, document_id: Id<Document>) {
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
                        .min_by_key(|(_, (_, u))| u.public_data.clock.get())
                        .map(|(idx, (_, u))| (*snap, idx, u.public_data.clock.get()))
                })
                .min_by_key(|&(_, _, clock)| clock);

            if let Some((snap, idx, clock)) = oldest {
                if let Some(updates) = doc_state.quarantine.get_mut(&snap) {
                    updates.remove(idx);
                    if updates.is_empty() {
                        doc_state.quarantine.remove(&snap);
                    }
                }
                tracing::warn!(
                    "Sync: quarantine overflow on {document_id:?}, dropped oldest update (clock {clock})"
                );
                self.send_to_logic(LogicEvent::SyncWarning {
                    document_id,
                    code: SyncWarningCode::QuarantineOverflow,
                });
            }
        }
    }

    /// Mark `snapshot_id` as known for `document_id` and release any updates that
    /// were quarantined waiting for it, in ascending `clock` order.
    pub(crate) fn snapshot_known(&mut self, document_id: Id<Document>, snapshot_id: Id<Snapshot>) {
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
        document_id: Id<Document>,
        snapshot_id: Id<Snapshot>,
        covered_seq: Seq<Relay>,
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
            tracing::info!("Sync: snapshot {snapshot_id:?} announced, no storage");
            return;
        };
        let api_base = self.api_base.clone();
        let url_scheme = self.url_scheme;
        let auth = self.auth_context();
        wasm_bindgen_futures::spawn_local(async move {
            let dl_url = crate::http::Url::<crate::http::ApiRoot>::of(url_scheme, &api_base)
                .route(format!("/documents/{document_id}/snapshot/download-url"));
            let body = http::SnapshotDownloadUrlRequest { snapshot_id };
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
        document_id: Id<Document>,
        snapshot_id: Id<Snapshot>,
        covered_seq: Seq<Relay>,
        blob: Option<Payload<pipeline::FetchedBlob>>,
    ) {
        let Some(doc_state) = self.documents.get_mut(&document_id) else {
            return;
        };
        let opened = match blob {
            Some(blob) => doc_state
                .open_signed_snapshot(
                    &blob,
                    pipeline::ServedSnapshot {
                        snapshot_id,
                        covered_seq,
                    },
                )
                .map_err(Some),
            None => Err(None),
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
                    covered_seq,
                    payload,
                    purpose: SnapshotPurpose::Confirm,
                });
                self.snapshot_known(document_id, snapshot_id);
            }
            // The walk of the log owed decides the writers first: the
            // snapshot is fetched again once it answers (`SYNC-234`).
            Err(Some(pipeline::PipelineError::LogUnsettled)) => {
                match doc_state.gate.defer_snapshot(snapshot_id, covered_seq) {
                    crate::channels::Deferral::Held | crate::channels::Deferral::Dropped => {}
                    crate::channels::Deferral::Overflow => {
                        tracing::warn!(
                            "Sync: snapshot {snapshot_id:?} dropped while the log walks"
                        );
                        self.send_to_logic(LogicEvent::SyncWarning {
                            document_id,
                            code: SyncWarningCode::QuarantineOverflow,
                        });
                    }
                }
            }
            Err(error) => {
                match error {
                    Some(e) => {
                        tracing::warn!("Sync: a snapshot of {document_id:?} does not open: {e}")
                    }
                    None => tracing::warn!("Sync: a snapshot of {document_id:?} did not download"),
                }
                self.send_to_logic(LogicEvent::SyncWarning {
                    document_id,
                    code: SyncWarningCode::DecryptFailed,
                });
            }
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
        document_id: Id<Document>,
        snapshot_id: Id<Snapshot>,
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
                bytes.read().is_some_and(|entry| {
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
                        covered_seq: check.covered_seq,
                    });
                }
                // The open waited on this verdict (`check_held_snapshot`).
                self.send_to_logic(LogicEvent::DocumentReady { document_id, role });
                None
            }
            SnapshotPurpose::Open => {
                tracing::warn!(
                    "Sync: snapshot {snapshot_id:?} of {document_id:?} refused \
                     (mismatch: {mismatch})"
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
                let signature =
                    ed25519_dalek::Signer::sign(signing_key, transcript.as_bytes()).to_bytes();
                tracing::warn!(
                    "Sync: snapshot {snapshot_id:?} of {document_id:?} contradicts its deltas"
                );
                Some(Vouch::Report(FixedBytes::from_array(signature)))
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
        document_id: Id<Document>,
        snapshot_id: Id<Snapshot>,
        vouch: Vouch,
    ) {
        let Some(tx) = self.event_tx.clone() else {
            tracing::info!("Sync: snapshot {snapshot_id:?} vouch, no relay");
            return;
        };
        let base = crate::http::Url::<crate::http::ApiRoot>::of(self.url_scheme, &self.api_base);
        let auth = self.auth_context();
        wasm_bindgen_futures::spawn_local(async move {
            let sent =
                send_vouch(&http::Fetch, &auth, &base, document_id, snapshot_id, &vouch).await;
            match sent {
                Ok(Some(amendments)) => {
                    let _ = tx.send(SyncEvent::AmendmentsFolded {
                        document_id,
                        amendments,
                    });
                }
                Ok(None) => {}
                Err(e) => tracing::warn!("Sync: snapshot {snapshot_id:?} vouch failed: {e}"),
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
    pub(crate) fn check_held_snapshot(&mut self, document_id: Id<Document>) -> bool {
        let Some(doc_state) = self.documents.get_mut(&document_id) else {
            return false;
        };
        let Some(held) = doc_state.held_snapshot.take() else {
            return false;
        };
        let served = pipeline::ServedSnapshot {
            snapshot_id: held.snapshot_id,
            covered_seq: held.covered_seq,
        };
        match doc_state.open_signed_snapshot(&held.blob, served) {
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
                    covered_seq: held.covered_seq,
                    payload,
                    purpose: SnapshotPurpose::Open,
                });
                true
            }
            Err(e) => {
                tracing::warn!("Sync: latest snapshot of {document_id:?} does not open ({e})");
                false
            }
        }
    }

    /// Decrypt and deliver every update quarantined under `snapshot_id`, oldest
    /// `clock` first.
    pub(crate) fn release_quarantine(
        &mut self,
        document_id: Id<Document>,
        snapshot_id: Id<Snapshot>,
    ) {
        let mut pending = {
            let Some(doc_state) = self.documents.get_mut(&document_id) else {
                return;
            };
            doc_state
                .quarantine
                .remove(&snapshot_id)
                .unwrap_or_default()
        };
        pending.sort_by_key(|(_, u)| u.public_data.clock.get());
        if let Some(doc_state) = self.documents.get_mut(&document_id)
            && doc_state.quarantine_len() < QUARANTINE_ALERT
        {
            // The gap closed: a later one asks again (`SYNC-031`).
            doc_state.gap_requested = false;
        }

        let (results, unvouched): (Vec<Result<SignedDelta, pipeline::PipelineError>>, Vec<_>) = {
            let Some(doc_state) = self.documents.get(&document_id) else {
                return;
            };
            let open = matches!(doc_state.gate, crate::channels::LogGate::Open);
            let (admitted, unvouched): (Vec<_>, Vec<_>) = pending
                .into_iter()
                .partition(|(_, u)| open && doc_state.writers.admits(&u.public_data.pub_key));
            let results = admitted
                .iter()
                .map(|(seq, u)| doc_state.open_signed_update(u, *seq))
                .collect();
            (results, unvouched)
        };
        self.hold_unvouched(document_id, unvouched);
        for opened in results {
            self.deliver_remote(document_id, Some(opened), |delta| {
                LogicEvent::RemoteUpdate { document_id, delta }
            });
        }
    }

    /// Holds `updates`, whose signer no writer of the latest walk names,
    /// until a listing asked after each arrived answers (`SYNC-211`). A
    /// newcomer's first delta can outrun the word that adds them, after a
    /// reconnect above all.
    pub(crate) fn hold_unvouched(
        &mut self,
        document_id: Id<Document>,
        updates: impl IntoIterator<Item = (Seq<Relay>, Update)>,
    ) {
        let Some(doc_state) = self.documents.get_mut(&document_id) else {
            return;
        };
        // A delta held while the log walks again waits for that walk alone.
        let seen = doc_state
            .gate
            .floor()
            .unwrap_or_else(|| doc_state.directory_asked.latest());
        doc_state
            .unvouched
            .extend(updates.into_iter().map(|(seq, update)| crate::Unvouched {
                seen,
                seq,
                update,
            }));
        let overflow = doc_state
            .unvouched
            .len()
            .saturating_sub(usize::try_from(QUARANTINE_LIMIT.get()).unwrap_or(usize::MAX));
        doc_state.unvouched.drain(..overflow);
        if overflow > 0 {
            self.send_to_logic(LogicEvent::SyncWarning {
                document_id,
                code: SyncWarningCode::WriterRefused,
            });
        }
        self.ask_for_unvouched(document_id);
    }

    /// Lists the members again when a held delta arrived after the latest
    /// listing was asked, so the answer can name its signer. A listing still
    /// in flight is waited for, and its answer asks again for what it left.
    /// A replay of many deltas thus costs two listings, not one per delta.
    fn ask_for_unvouched(&mut self, document_id: Id<Document>) {
        let waits = self.documents.get(&document_id).is_some_and(|doc| {
            let latest = doc.directory_asked.latest();
            doc.directory_held >= latest && doc.unvouched.iter().any(|held| held.seen >= latest)
        });
        if waits {
            self.handle_list_document_members(None, document_id);
        }
    }

    /// Settles each delta held before the listing `asked` was sent, once its
    /// answer replaced the writers (`SYNC-211`). A delta its signer now passes
    /// reaches Logic, and any other is refused (`SYNC-211`).
    ///
    /// A document of a channel keeps each delta it refuses as no writer's,
    /// sealed. A later walk of its log under the rule of a space decides it
    /// again (`SYNC-230`).
    pub(crate) fn settle_unvouched(
        &mut self,
        document_id: Id<Document>,
        asked: ambre_core::Seq<crate::generation::Listing>,
    ) {
        let of_channel = self.channel_metas.contains_key(&document_id);
        let Some(doc_state) = self.documents.get_mut(&document_id) else {
            return;
        };
        // An older answer settles nothing while a walk is owed: only its own
        // answer decides what the document held (`SYNC-232`).
        if !matches!(doc_state.gate, crate::channels::LogGate::Open) {
            return;
        }
        let (due, kept): (Vec<crate::Unvouched>, Vec<crate::Unvouched>) =
            std::mem::take(&mut doc_state.unvouched)
                .into_iter()
                .partition(|held| held.seen < asked);
        doc_state.unvouched = kept;
        let mut results = Vec::with_capacity(due.len());
        for held in due {
            let opened = doc_state.open_signed_update(&held.update, held.seq);
            if of_channel && matches!(opened, Err(pipeline::PipelineError::NotAWriter)) {
                doc_state.refused.push(held);
            }
            results.push(opened);
        }
        let overflow = doc_state
            .refused
            .len()
            .saturating_sub(usize::try_from(QUARANTINE_LIMIT.get()).unwrap_or(usize::MAX));
        doc_state.refused.drain(..overflow);
        for opened in results {
            self.deliver_remote(document_id, Some(opened), |delta| {
                LogicEvent::RemoteUpdate { document_id, delta }
            });
        }
        self.ask_for_unvouched(document_id);
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

    pub(crate) fn handle_open_document(&mut self, document_id: Id<Document>, doc_type: DocType) {
        // The type comes from the relay's word, and a Murmure label checks
        // the key against a channel's log. An open under another type claims the document for the
        // session and drops a key a Murmure open installed (`SYNC-260`).
        if doc_type == DocType::Murmure && self.typed_opens.contains(&document_id) {
            tracing::warn!(
                "Sync: open of {document_id:?} as Murmure refused: held as another type"
            );
            return;
        }
        if doc_type != DocType::Murmure {
            self.typed_opens.insert(document_id);
            if self
                .documents
                .get(&document_id)
                .is_some_and(|doc| doc.doc_type == DocType::Murmure)
            {
                self.documents.remove(&document_id);
            }
        }
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
            let rule = self.log_rule(document_id);
            let log_document = self.log_document(document_id);
            // A snapshot this user confirmed alone is this user's own work, and
            // opens as it is (`AUTHOR-022` reads another user's).
            let own_key = self.author_pub_key();
            let roots = self.roots(document_id, doc_type);
            wasm_bindgen_futures::spawn_local(Self::open_from_relay(
                RelayOpen {
                    api_base,
                    url_scheme,
                    doc_id,
                    doc_type,
                    exchange_key,
                    auth,
                    my_user_id,
                    rule,
                    log_document,
                    own_key,
                    resume,
                    roots,
                    logic_tx: self.logic_tx.clone(),
                },
                tx,
            ));
        } else {
            let doc_key = DocumentKey::generate();
            self.documents.insert(
                document_id,
                SyncDocumentState::new(document_id, doc_key, doc_type, crate::Existence::LocalOnly),
            );
            if self.ws_connected() {
                self.ws_send(&WsClientMsg::Subscribe { document_id });
            }
            tracing::info!("Sync: OpenDocument {document_id:?}");
        }
    }

    /// The open of `open.doc_id` from the relay, off the select loop. It
    /// fetches the members, the key links and the snapshot, then answers
    /// through `tx` with [`SyncEvent::OpenDocumentReady`] or
    /// [`SyncEvent::OpenDocumentFailed`].
    // The task runs in the `spawn_local` of the browser path, which no host
    // test reaches, so cargo-mutants skips it.
    #[cfg_attr(test, mutants::skip)]
    async fn open_from_relay(open: RelayOpen, tx: ambre_bus::conduit::UnboundedSender<SyncEvent>) {
        let RelayOpen {
            api_base,
            url_scheme,
            doc_id,
            doc_type,
            exchange_key,
            auth,
            my_user_id,
            rule,
            log_document,
            own_key,
            resume,
            roots,
            logic_tx,
        } = open;
        let members_url = crate::http::Url::<crate::http::ApiRoot>::of(url_scheme, &api_base)
            .route(format!("/documents/{doc_id}/members"));
        // 404 -> the doc does not exist server-side yet (local-only):
        // generate a fresh key and defer creation to the first update.
        let mut writers = membership_log::Writers::Unchecked;
        let mut departed = None;
        let mut link_signers = membership_log::LinkSigners::Unchecked;
        let mut named_key = None;
        let mut vouch = None;
        let mut word = None;
        let mut members = Vec::new();
        let answer = MembersAnswer::classify(
            http::get_json_opt_auth::<http::MembersResponse>(&members_url, auth.clone()).await,
        );
        let (doc_key, existence, signing_keys, role, rotation_owed) = match answer {
            MembersAnswer::Exists(resp) => match &exchange_key {
                Some(ek) => match Self::opened_from_members(
                    &resp,
                    log_document,
                    ek,
                    my_user_id,
                    &rule,
                    &roots,
                ) {
                    Ok(o) => {
                        writers = o.writers;
                        departed = o.departed;
                        link_signers = o.link_signers;
                        members = o.members;
                        named_key = o.named_key;
                        vouch = Some(o.vouch);
                        word = Some((o.word, o.heads));
                        (
                            o.doc_key,
                            crate::Existence::OnRelay,
                            o.signing_keys,
                            o.role,
                            o.rotation_owed,
                        )
                    }
                    Err((cause, _)) => {
                        tracing::warn!("Sync: cannot unwrap the key of {doc_id:?}");
                        let _ = tx.send(SyncEvent::OpenDocumentFailed {
                            document_id: doc_id,
                            cause,
                        });
                        return;
                    }
                },
                None => {
                    tracing::warn!("Sync: open of {doc_id:?}: session not ready");
                    let _ = tx.send(SyncEvent::OpenDocumentFailed {
                        document_id: doc_id,
                        cause: OpenFailure::Transient,
                    });
                    return;
                }
            },
            // Not found: the document is local-only. No members response
            // means no directory: a local-only doc has no peers to resolve
            // presence for anyway, and its creator owns it.
            MembersAnswer::Absent => (
                DocumentKey::generate(),
                crate::Existence::LocalOnly,
                Vec::new(),
                Role::Owner,
                ambre_core::RotationDebt::Settled,
            ),
            // A failed request says nothing about the document. Opening it
            // local-only would mint a throwaway key and edit a copy nobody
            // receives, so the open fails and the user may retry (`SYNC-040`).
            MembersAnswer::Failed(error) => {
                log_members_failure(doc_id, &error);
                let _ = tx.send(SyncEvent::OpenDocumentFailed {
                    document_id: doc_id,
                    cause: members_failure_cause(&error),
                });
                return;
            }
        };
        // The chain opens what was sealed before the last rotation,
        // the snapshot among it (`SYNC-117`, `SYNC-119`).
        // A link counts only when a writer the log names signed it
        // (`SYNC-121`).
        // The key must be one a writer named (`SYNC-200`). A failed fetch of
        // the links thus fails the open as one a later open may clear.
        let links = if existence == crate::Existence::OnRelay {
            match http::try_fetch_key_links(url_scheme, &api_base, doc_id, auth.clone()).await {
                Ok(links) => crate::chain::signed_by_writers(links, &link_signers),
                Err(error) => {
                    tracing::warn!("Sync: key links of {doc_id:?} unavailable: {error}");
                    let _ = tx.send(SyncEvent::OpenDocumentFailed {
                        document_id: doc_id,
                        cause: OpenFailure::Transient,
                    });
                    return;
                }
            }
        } else {
            Vec::new()
        };
        if let Err((cause, message)) =
            Self::named_key_holds(log_document, &doc_key, named_key, &links)
        {
            tracing::warn!("Sync: open of {doc_id:?}: {message}");
            let _ = tx.send(SyncEvent::OpenDocumentFailed {
                document_id: doc_id,
                cause,
            });
            return;
        }
        match vouch.map(|v| v.check(log_document, &doc_key, &links).map(|()| v.proved())) {
            Some(Err(refusal)) => {
                tracing::warn!("Sync: open of {doc_id:?}: key refused: {refusal:?}");
                let cause = refusal.open_failure();
                let _ = tx.send(SyncEvent::OpenDocumentFailed {
                    document_id: doc_id,
                    cause,
                });
                return;
            }
            // The members of an accepted log land before the open does, so
            // no open the Logic thread asks after it misses them.
            Some(Ok(proved)) => {
                if let Some(vouched) = roots_vouched(doc_type, members) {
                    let _ = tx.send(vouched);
                }
                // The log proved the pin, or the root of a Murmure open, which
                // then outlives the session (`SYNC-238`, `SYNC-242`).
                if let Some(inviter) = proved {
                    let _ = logic_tx.send(LogicEvent::InviterProven {
                        document_id: doc_id,
                        inviter,
                    });
                }
            }
            None => {}
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
        let (snapshot_payload, downloaded_snapshot_id, covered_seq) = if existence
            == crate::Existence::OnRelay
        {
            let meta_url = crate::http::Url::<crate::http::ApiRoot>::of(url_scheme, &api_base)
                .route(format!("/documents/{doc_id}/snapshot"));
            match http::get_json_opt_auth::<http::SnapshotMetadataResponse>(&meta_url, auth.clone())
                .await
            {
                Ok(Some(meta)) => {
                    let others = meta.status == ambre_core::SnapshotStatus::SelfConfirmed
                        && Some(meta.author_pubkey) != own_key;
                    let (start_id, start_covered) = match (others, &meta.reference) {
                        (false, _) => (Some(meta.snapshot_id), meta.covered_seq),
                        (true, Some(r)) => (Some(r.snapshot_id), r.covered_seq),
                        (true, None) => (None, Seq::<Relay>::ZERO),
                    };
                    if others {
                        let blob = http::download_snapshot(
                            url_scheme,
                            &api_base,
                            doc_id,
                            meta.snapshot_id,
                            auth.clone(),
                        )
                        .await;
                        held_snapshot = blob.map(|blob| HeldSnapshot {
                            snapshot_id: meta.snapshot_id,
                            covered_seq: meta.covered_seq,
                            blob,
                        });
                    }
                    // The local copy already folds in the relay's
                    // snapshot: no download, and the replay starts
                    // after the copy's horizon (`RESID-010`).
                    let resumed = resume.filter(|horizon| light::resumes(*horizon, start_covered));
                    let payload = match (start_id, resumed) {
                        (Some(id), None) => {
                            http::download_snapshot(url_scheme, &api_base, doc_id, id, auth).await
                        }
                        _ => None,
                    };
                    let snap_id = start_id;
                    // Only gate on the covered seq if the blob actually
                    // downloaded; otherwise a failed fetch would skip
                    // history that was never applied (data loss).
                    match (payload, resumed) {
                        (Some(payload), _) => (Some(payload), snap_id, start_covered),
                        (None, Some(horizon)) => (None, snap_id, horizon),
                        (None, None) => (None, None, Seq::<Relay>::ZERO),
                    }
                }
                // No snapshot yet, or a transient error: fall back to the
                // full delta replay (RequestMissing from seq 0) on open.
                // With no snapshot on the relay, a local copy still
                // spares the replay of what it holds (`RESID-010`).
                Ok(None) => match resume {
                    Some(horizon) => (None, None, horizon),
                    None => (None, None, Seq::<Relay>::ZERO),
                },
                Err(_) => (None, None, Seq::<Relay>::ZERO),
            }
        } else {
            (None, None, Seq::<Relay>::ZERO)
        };
        // Register the downloaded snapshot's real id as the known parent so
        // post-snapshot deltas are not quarantined. Without a download, fall
        // back to the deterministic genesis id (deltas authored before any
        // snapshot upload are all parented on it). Genesis stays seeded in
        // `SyncDocumentState::new` regardless, covering mixed histories.
        let snapshot_id = downloaded_snapshot_id.unwrap_or_else(|| Id::<Snapshot>::genesis(doc_id));
        let _ = tx.send(SyncEvent::OpenDocumentReady {
            document_id: doc_id,
            doc_key,
            snapshot_payload,
            snapshot_id,
            covered_seq,
            held_snapshot,
            doc_type,
            existence,
            signing_keys,
            role,
            rotation_owed,
            links,
            writers,
            departed,
        });
        // After the open, which made the session the word lands on.
        if let Some((word, heads)) = word {
            let _ = tx.send(SyncEvent::LogWalked {
                document_id: doc_id,
                word,
                heads,
            });
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
    ) -> Result<DocumentKey, crate::error::SyncError> {
        Self::unwrap_anchored(exchange_key, members).map(|(key, _)| key)
    }

    /// Unwraps our own copy of the key, and the anchor of the membership log
    /// sealed beside it (`SYNC-104`).
    pub(crate) fn unwrap_anchored(
        exchange_key: &x25519_dalek::StaticSecret,
        members: &[http::MemberEntry],
    ) -> Result<
        (DocumentKey, Option<FixedBytes<ambre_protocol::EntryDigest>>),
        crate::error::SyncError,
    > {
        crate::sharing::unwrap_own(
            exchange_key,
            // A row whose exchange key does not parse is no row of ours.
            members.iter().filter_map(|m| {
                Some((
                    m.exchange_pub_key.key()?,
                    &m.doc_key_enc,
                    m.wrapper_exchange_pub_key.key(),
                ))
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
    /// under `rule`, the rule of the document's log (`SYNC-106`), as the log
    /// of `document_id`: the `meta` of a document of a channel (`SYNC-229`).
    /// `roots` is what this client holds out of band for the open. It holds
    /// the inviter the invite link pinned (`SYNC-205`), the roots of the
    /// session for a Murmure open (`SYNC-235`), and this client's own key
    /// (`SYNC-210`).
    ///
    /// # Errors
    ///
    /// No wrap opens under `exchange_key`: the key has not reached this
    /// device (`SYNC-173`).
    pub(crate) fn opened_from_members(
        resp: &http::MembersResponse,
        document_id: Id<Document>,
        exchange_key: &x25519_dalek::StaticSecret,
        my_user_id: Option<Id<User>>,
        rule: &membership_log::LogRule,
        roots: &crate::key_vouch::Roots,
    ) -> Result<OpenedMembers, (OpenFailure, String)> {
        let (doc_key, anchor) = Self::unwrap_anchored(exchange_key, &resp.members)
            .map_err(|error| (OpenFailure::KeyNotReceived, error.to_string()))?;
        let log = crate::sharing::walk_served_under(document_id, anchor, &resp.log, rule);
        let role =
            crate::sharing::own_role(resp.members.iter().map(|m| (m.user_id, m.role)), my_user_id);
        let signing_keys = crate::sharing::presence_directory_under(
            &log,
            resp.members
                .iter()
                .map(|m| (m.user_id, m.signing_pub_key.key())),
        );
        Ok(OpenedMembers {
            doc_key,
            signing_keys,
            role,
            rotation_owed: resp.rotation_owed,
            writers: membership_log::Writers::of(&log),
            departed: log.departed_at(),
            link_signers: log.link_signers(),
            named_key: my_user_id.and_then(|me| log.wraps(me)),
            vouch: crate::key_vouch::KeyVouch::of(
                &log,
                rule,
                &crate::sharing::own_exchange_key(exchange_key),
                &resp.members,
                roots,
                resp.rotation_owed,
            ),
            word: membership_log::SpaceWord::of(&log),
            heads: log.heads(),
            members: log.member_pins(),
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
    /// The reason the key is refused: the chain does not reach the named
    /// key, so the key has not reached this device (`SYNC-173`).
    pub(crate) fn named_key_holds(
        document_id: Id<Document>,
        doc_key: &DocumentKey,
        named_key: Option<FixedBytes<ambre_core::KeyId>>,
        links: &[ambre_protocol::KeyLink],
    ) -> Result<(), (OpenFailure, String)> {
        let Some(named) = named_key else {
            return Ok(());
        };
        let chain = crate::chain::Chain::new(document_id, links.iter().cloned());
        match chain.recover(doc_key, named) {
            Some(_) => Ok(()),
            None => Err((
                OpenFailure::KeyNotReceived,
                "the document key is not the one the invitation names".into(),
            )),
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
    pub(crate) fn issue_session_identity(&mut self, document_id: Id<Document>) {
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
            entry,
            nonce,
        });
    }

    /// Open a new space as a document, marked so its lazy creation tells
    /// the relay it is a space (`SYNC-170`).
    pub(crate) fn handle_create_space(&mut self, space_id: Id<Space>) {
        let document_id = space_id.document_id();
        self.new_spaces.insert(document_id);
        self.handle_open_document(document_id, DocType::Murmure);
    }

    pub(crate) fn handle_close_document(&mut self, document_id: Id<Document>) {
        self.new_spaces.remove(&document_id);
        self.channel_metas.remove(&document_id);
        if self.ws_connected() {
            self.ws_send(&WsClientMsg::Unsubscribe { document_id });
        }
        self.documents.remove(&document_id);
        self.session_nonces.remove(&document_id);
        self.light.remove(&document_id);
        tracing::info!("Sync: CloseDocument {document_id:?}");
    }

    pub(crate) fn handle_reconnect(&mut self, document_id: Id<Document>) {
        if self.ws_connected() {
            let last_seq = self
                .documents
                .get(&document_id)
                .map(|d| d.last_seq)
                .unwrap_or(Seq::<Relay>::new(0));
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
    pub(crate) fn author_pub_key(&self) -> Option<FixedBytes<PublicKey>> {
        self.signing_key
            .as_ref()
            .map(|sk| FixedBytes::from_array(sk.verifying_key().to_bytes()))
    }

    /// Replays updates that were buffered before the session/document were ready
    /// (see [`pending_pre_session_updates`](Self::pending_pre_session_updates)).
    /// Drains only the entries for `document_id`, preserving their order.
    pub(crate) fn flush_pre_session_updates(&mut self, document_id: Id<Document>) {
        if self.signing_key.is_none() || self.pending_pre_session_updates.is_empty() {
            return;
        }
        let (drained, kept) = std::mem::take(&mut self.pending_pre_session_updates)
            .into_iter()
            .partition::<Vec<_>, _>(|(doc, ..)| *doc == document_id);
        self.pending_pre_session_updates = kept;
        if !drained.is_empty() {
            tracing::info!(
                "[Sync] flushing {} buffered pre-session update(s) doc={document_id}",
                drained.len()
            );
        }
        for (_, payload, summary) in drained {
            self.send_summarized_update(document_id, payload, summary);
        }
    }

    /// An update with no summary, as most tests send one.
    #[cfg(test)]
    pub(crate) fn handle_send_update(&mut self, document_id: Id<Document>, payload: Vec<u8>) {
        self.send_summarized_update(document_id, payload.into(), None);
    }

    /// Sends a delta the Logic thread wrote, with the summary of what it
    /// holds sealed beside it (`SYNC-113`).
    pub(crate) fn send_summarized_update(
        &mut self,
        document_id: Id<Document>,
        payload: ambre_core::Payload<ambre_core::DeltaBytes>,
        summary: Option<ActivitySummary>,
    ) {
        if self.signing_key.is_none() {
            // Session not established yet (for example an edit landed in the gap between
            // an F5 and DevLogin completing). Buffer instead of dropping; flushed
            // once the document opens (see `pending_pre_session_updates`).
            tracing::info!("[Sync] SendUpdate before session ready, buffering doc={document_id}");
            self.pending_pre_session_updates
                .push((document_id, payload, summary));
            return;
        }
        // The nonce rides inside the ciphertext, ahead of the delta
        // (`SYNC-112`). A document opened with no identity has written nothing
        // the Logic thread would send.
        let Some(nonce) = self.session_nonces.get(&document_id) else {
            if !self.documents.contains_key(&document_id) {
                // No session of this document: the update cannot reach the
                // relay, so the refusal is said aloud (`SYNC-069`).
                self.send_to_logic(LogicEvent::SyncWarning {
                    document_id,
                    code: SyncWarningCode::UpdateRefused,
                });
                return;
            }
            tracing::warn!("[Sync] SendUpdate with no session nonce doc={document_id}, dropped");
            return;
        };
        let summary = summary.as_ref().map(ActivitySummary::to_sealed);
        self.send_update_body(document_id, *nonce, summary.as_ref(), &payload);
    }

    /// Builds the body of a delta this client writes and sends it.
    fn send_update_body(
        &mut self,
        document_id: Id<Document>,
        nonce: FixedBytes<SessionNonce>,
        summary: Option<&ambre_core::activity::ActivitySummary>,
        delta: &ambre_core::Payload<ambre_core::DeltaBytes>,
    ) {
        match pipeline::update_body(&nonce, summary, delta) {
            Ok(body) => self.send_body(document_id, body),
            Err(error) => tracing::warn!("[Sync] update body refused doc={document_id}: {error}"),
        }
    }

    /// Sends what an earlier session of this user wrote and never sent. The
    /// body carries that session's nonce, so a reader derives the PeerID that
    /// wrote it (`AUTHOR-028`).
    pub(crate) fn handle_send_unsent(
        &mut self,
        document_id: Id<Document>,
        payload: ambre_core::Payload<ambre_core::DeltaBytes>,
        nonce: FixedBytes<SessionNonce>,
    ) {
        if self.signing_key.is_none() {
            // Sent after the document is ready, so after the session is.
            tracing::warn!("[Sync] SendUnsent before session ready doc={document_id}, dropped");
            return;
        }
        // What an earlier session left unsent is replayed with no summary:
        // the Logic thread no longer knows what it held.
        self.send_update_body(document_id, nonce, None, &payload);
    }

    /// Seals an update body and sends it, or holds it while the document is
    /// created or owes a rotation. The body carries its nonce, so a frame
    /// sent again later keeps the PeerID it was written under.
    pub(crate) fn send_body(&mut self, document_id: Id<Document>, body: UpdateFrame) {
        // Each arm that drops the frame wipes it (`UpdateFrame::wipe`).
        let (Some(signing_key), Some(pub_key)) = (self.signing_key.clone(), self.author_pub_key())
        else {
            body.wipe();
            return;
        };
        // Lazy server creation: a local-only doc materializes on its first update.
        // Buffer the body and fire the create POST once; the buffer is flushed
        // in order from the `DocumentMaterialized` handler.
        if self
            .documents
            .get(&document_id)
            .is_some_and(|d| d.existence == crate::Existence::LocalOnly)
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
            body.wipe();
            self.send_to_logic(LogicEvent::SyncWarning {
                document_id,
                code: SyncWarningCode::UpdateRefused,
            });
            return;
        };
        // A delta refused by name holds the document: a later one would build
        // on an edit the relay lacks (`SYNC-156`).
        if doc_state.rekey.holds_refused() {
            tracing::debug!("[Sync] edit held: doc={document_id} has a refused delta");
            body.wipe();
            return;
        }
        // A walk of the log owed holds the edit unsealed: the key may be
        // the relay's (`SYNC-232`). Past its bound, the walk drops what it
        // held, and the document sends nothing more until it reopens
        // (`SYNC-234`).
        let body = match doc_state.gate.hold(body) {
            crate::channels::Hold::Seal(body) => body,
            crate::channels::Hold::Held | crate::channels::Hold::Ended => return,
            crate::channels::Hold::Overflow => {
                tracing::warn!("[Sync] edits dropped: doc={document_id} walks its log too long");
                // The refusal checked above was not held yet, so this drop is
                // the first one to report.
                doc_state.rekey.edit_lost();
                self.send_to_logic(LogicEvent::SyncWarning {
                    document_id,
                    code: SyncWarningCode::EditRefused,
                });
                return;
            }
        };
        // An owed rotation is paid before any frame: the edit waits unsealed,
        // and takes its clock once the new key lands (`SYNC-128`).
        if doc_state.rekey.owes() {
            if doc_state.rekey.hold(body) {
                self.start_settle(document_id);
            }
            return;
        }
        // A peer signed a log head this client's log does not pass through.
        // The edit waits unsealed while the head is under check: a hidden
        // revoke may have retired the key it would take (`SYNC-220`). It
        // comes after the owed rotation, whose waiting edits come back
        // through here, so the two holds keep the order of the edits.
        if doc_state.head_watch.holds_edits() {
            tracing::debug!("[Sync] edit held: doc={document_id} checks a signed log head");
            doc_state.head_watch.hold(body);
            return;
        }
        doc_state.local_clock = doc_state.local_clock.next();
        let clock = doc_state.local_clock;
        let Some((key_id, signature)) =
            self.seal_and_send(document_id, clock, &body, &signing_key, pub_key)
        else {
            body.wipe();
            return;
        };
        let sent = SentFrame {
            clock,
            key_id,
            signature,
            payload: body,
        };
        match self.documents.get_mut(&document_id) {
            Some(doc_state) => {
                if doc_state.rekey.record_sent(sent) {
                    tracing::warn!(
                        "[Sync] unconfirmed log full doc={document_id}, oldest forgotten"
                    );
                }
            }
            None => sent.payload.wipe(),
        }
    }

    /// Seals an update `body` at `clock` under the current key and sends it.
    /// Returns the key it was sealed under and its signature, `None` when the
    /// seal failed.
    fn seal_and_send(
        &self,
        document_id: Id<Document>,
        clock: Seq<DeltaAuthor>,
        body: &UpdateFrame,
        signing_key: &SigningKey,
        pub_key: FixedBytes<PublicKey>,
    ) -> Option<(FixedBytes<KeyId>, FixedBytes<Signature>)> {
        let doc_state = self.documents.get(&document_id)?;
        if doc_state.rekey.holds_refused() {
            return None;
        }
        // The one door of an update to the key (`SYNC-234`).
        let doc_key = doc_state.sealing_key().ok()?;
        let key_id = doc_key.id();
        let public_data = UpdatePublicData {
            document_id,
            pub_key,
            key_id,
            snapshot_id: doc_state.current_snapshot_id,
            clock,
            // The head of the log this client last read (`SYNC-206`).
            log_head: doc_state.head_watch.head(),
            // The entry that head names, which proves it (`SYNC-218`).
            log_entry: doc_state.head_watch.head_entry(),
        };
        match pipeline::update_frame(body, public_data, doc_key, signing_key) {
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
    /// take the next clocks. A walk of the log owed holds the refused frames
    /// until its answer, as it holds the edits (`SYNC-234`).
    pub(crate) fn resend_refused(
        &mut self,
        document_id: Id<Document>,
        refused: Vec<(Seq<DeltaAuthor>, UpdateFrame)>,
        waiting: Vec<UpdateFrame>,
    ) {
        let refused = match self.documents.get_mut(&document_id) {
            Some(doc_state) => doc_state.gate.hold_resend(refused),
            None => refused,
        };
        if let (Some(signing_key), Some(pub_key)) =
            (self.signing_key.clone(), self.author_pub_key())
        {
            for (clock, body) in refused {
                if let Some((_, signature)) =
                    self.seal_and_send(document_id, clock, &body, &signing_key, pub_key)
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
        document_id: Id<Document>,
        expected: Seq<DeltaAuthor>,
        signature: FixedBytes<Signature>,
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
        doc_state.local_clock = expected.before();
        for body in bodies {
            self.send_body(document_id, body);
        }
    }

    /// The relay refused our delta signed `signature` before sequencing it
    /// (`SRV-185`). It leaves the unconfirmed log with every later delta, and
    /// the document holds every edit from here until it reopens. The first
    /// refusal is reported, so the user learns the relay lacks an edit
    /// (`SYNC-155`, `SYNC-156`).
    pub(crate) fn handle_delta_refused(
        &mut self,
        document_id: Id<Document>,
        signature: FixedBytes<Signature>,
        code: ambre_core::RefusalCode,
    ) {
        match code {
            ambre_core::RefusalCode::InternalError => {
                self.handle_delta_failed(document_id, signature);
            }
            ambre_core::RefusalCode::SegmentLocked => {
                self.handle_delta_locked(document_id, &signature);
            }
            _ => self.hold_refused(document_id, &signature, HoldCause::Refused(code)),
        }
    }

    /// The relay refused our delta signed `signature` for the lock of its
    /// segment (`SRV-115`). It and every later delta of the document return
    /// to the Logic thread, which moves their new messages to the active
    /// segment (`SYNC-165`). The document is not held.
    fn handle_delta_locked(
        &mut self,
        document_id: Id<Document>,
        signature: &FixedBytes<Signature>,
    ) {
        let Some(doc_state) = self.documents.get_mut(&document_id) else {
            return;
        };
        let Some(bodies) = doc_state.rekey.delta_locked(signature) else {
            return;
        };
        let deltas = bodies
            .into_iter()
            .filter_map(|body| pipeline::split_update_body(body).ok())
            .map(|body| body.delta)
            .collect();
        tracing::warn!("[Sync] delta refused for a locked segment doc={document_id}");
        self.send_to_logic(LogicEvent::DeltasLocked {
            document_id,
            deltas,
        });
    }

    /// Drops the delta signed `signature` and every later one, and holds the
    /// document. The first hold is reported (`SYNC-155`, `SYNC-156`).
    fn hold_refused(
        &mut self,
        document_id: Id<Document>,
        signature: &FixedBytes<Signature>,
        cause: HoldCause,
    ) {
        let Some(doc_state) = self.documents.get_mut(&document_id) else {
            return;
        };
        if !doc_state.rekey.delta_refused(signature) {
            return;
        }
        tracing::warn!("[Sync] delta refused doc={document_id}: {cause}");
        self.send_to_logic(LogicEvent::SyncWarning {
            document_id,
            code: SyncWarningCode::EditRefused,
        });
    }

    /// The relay failed to sequence our delta signed `signature` through a
    /// fault of its own (`SRV-186`). It leaves again after a delay that
    /// doubles at each failure since the last echo. Past the budget the
    /// document holds it, as for a refusal (`SYNC-159`).
    fn handle_delta_failed(&mut self, document_id: Id<Document>, signature: FixedBytes<Signature>) {
        let Some(doc_state) = self.documents.get_mut(&document_id) else {
            return;
        };
        // A number issued for a delta this log forgot is never armed, and
        // the next failure takes the one after it: only order is compared.
        let ticket = self.resend_ticket.issue();
        let Some(failure) = doc_state.rekey.failed(&signature, ticket) else {
            // A delta this log forgot: answer, so the fence never stands.
            self.ws_send(&WsClientMsg::ClockResync {
                document_id,
                signature,
            });
            return;
        };
        match resend_retry_delay(failure) {
            Some(delay) => {
                tracing::warn!("[Sync] delta failed doc={document_id} (failure {failure})");
                let retry = SyncEvent::ResendRetry {
                    document_id,
                    ticket,
                };
                self.arm_event_timer(retry, delay);
            }
            None => {
                self.hold_refused(document_id, &signature, HoldCause::Failed(failure));
            }
        }
    }

    /// A resend timer fired. Unless a later failure or a refusal made it
    /// stale, the answer lifts the relay's fence (`SRV-169`). Then the failed
    /// delta and every later one leave again from its own clock, which the
    /// relay never took (`SYNC-159`).
    pub(crate) fn resend_failed(
        &mut self,
        document_id: Id<Document>,
        ticket: ambre_core::Seq<crate::generation::ResendTicket>,
    ) {
        let Some(doc_state) = self.documents.get_mut(&document_id) else {
            return;
        };
        let Some((signature, clock, bodies)) = doc_state.rekey.resend_failed(ticket) else {
            return;
        };
        doc_state.local_clock = clock.before();
        tracing::info!(
            "[Sync] resend doc={document_id}, {} delta(s) leave again from {clock}",
            bodies.len()
        );
        self.ws_send(&WsClientMsg::ClockResync {
            document_id,
            signature,
        });
        for body in bodies {
            self.send_body(document_id, body);
        }
    }

    /// Installs `new_key` as the document's current key, then sends what the
    /// owed rotation held back. A key this client already holds as current is
    /// not pushed into the cache a second time. `links` join the chain, so a
    /// key the cache drops still opens through them (`SYNC-119`).
    ///
    /// A key this client held before the current one, in its cache or down
    /// its chain, is refused (`SYNC-213`). The relay that withholds the link
    /// past it cannot roll the document back to a key a revoke retired.
    ///
    /// The current key pays no owed rotation: a debt is paid with a fresh
    /// key, so the debt stands for the next edit (`SYNC-223`). `departed` is
    /// the latest departure the key was checked against.
    ///
    /// Edits can wait for the key a revoke installed. A key the chain does
    /// not lead back to that key is then refused, and the edits stay held
    /// (`SYNC-224`).
    pub(crate) fn install_key(
        &mut self,
        document_id: Id<Document>,
        new_key: DocumentKey,
        links: impl IntoIterator<Item = ambre_protocol::KeyLink>,
        departed: Option<Seq<ambre_protocol::LogPlaces>>,
    ) {
        let Some(doc_state) = self.documents.get_mut(&document_id) else {
            self.rekey_light(document_id, new_key);
            return;
        };
        if new_key.id() != doc_state.doc_key.id() && doc_state.key_named(new_key.id()).is_some() {
            tracing::warn!(
                "[Sync] doc={document_id}: a rotation names a key the current one replaced; kept the current key"
            );
            return;
        }
        let links: Vec<_> = links.into_iter().collect();
        // While edits wait for the key a revoke installed, only a key that
        // leads back to it ends the wait. Any other one, the current key
        // served again included, the revoked member may hold (`SYNC-224`).
        if let Some(awaited) = doc_state.rekey.awaited() {
            let mut chain = doc_state.chain.clone();
            for link in &links {
                chain.push(link.clone());
            }
            if chain.recover(&new_key, awaited).is_none() {
                tracing::warn!(
                    "[Sync] doc={document_id}: a rotation names a key that does not follow the latest revoke; edits stay held"
                );
                doc_state.rekey.payment_failed();
                return;
            }
        }
        if new_key.id() == doc_state.doc_key.id() && doc_state.rekey.owes() {
            tracing::warn!(
                "[Sync] doc={document_id}: a rotation names the current key while one is owed; the debt stands"
            );
            doc_state.rekey.payment_failed();
            return;
        }
        doc_state.departed = doc_state.departed.max(departed);
        for link in links {
            doc_state.chain.push(link);
        }
        if doc_state.doc_key.id() != new_key.id() {
            doc_state.rotate_key(new_key);
        }
        let current = doc_state.doc_key.id();
        let (refused, waiting) = doc_state.rekey.key_installed(current);
        self.resend_refused(document_id, refused, waiting);
        // A new space key leaves each catalogue entry under the old one stale.
        self.drive_space_rotation(document_id);
    }

    /// Pays the rotation a document owes, in a spawned task (`SYNC-128`).
    pub(crate) fn start_settle(&mut self, document_id: Id<Document>) {
        // The payment links the key the walk of the log settled to the new
        // one. While the log walks, the debt stands (`SYNC-234`).
        let previous = self
            .documents
            .get(&document_id)
            .and_then(|doc_state| doc_state.sealing_key().ok().cloned());
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
    pub(crate) fn trigger_lazy_create(&mut self, document_id: Id<Document>) {
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
        // A space is marked so on its creation, so each member lists it
        // (`SYNC-170`).
        let space = if self.new_spaces.contains(&document_id) {
            ambre_core::DocumentScope::Space
        } else {
            ambre_core::DocumentScope::Personal
        };
        let api_base = self.api_base.clone();
        let url_scheme = self.url_scheme;
        let auth = self.auth_context();
        wasm_bindgen_futures::spawn_local(async move {
            // The copy carries the anchor of the log its creator starts
            // (`SYNC-104`, `SYNC-132`).
            let (doc_key_enc, entry) =
                match sharing::creation(document_id, &doc_key, &exchange_key, &author) {
                    Ok(created) => created,
                    Err(error) => {
                        let _ = tx.send(SyncEvent::DocumentCreateFailed {
                            document_id,
                            failure: http::Reported::at(&error, "creation"),
                        });
                        return;
                    }
                };

            let url = crate::http::Url::<crate::http::ApiRoot>::of(url_scheme, &api_base)
                .route("/documents");
            let body = http::CreateDocumentRequest {
                doc_type,
                name_enc: None,
                doc_key_enc,
                document_id: Some(document_id),
                entry: Some(entry),
                space,
            };
            match http::post_json_auth::<_, http::CreateDocumentResponse>(&url, &body, auth).await {
                Ok(_) => {
                    let _ = tx.send(SyncEvent::DocumentMaterialized { document_id });
                }
                Err(e) => {
                    let _ = tx.send(SyncEvent::DocumentCreateFailed {
                        document_id,
                        failure: http::Failed {
                            failure: e.failure(),
                        },
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
        document_id: Id<Document>,
        payload: ambre_core::Payload<ambre_core::SnapshotBytes>,
        covered_seq: Seq<Relay>,
    ) {
        let upload = match self.snapshot_upload(document_id, &payload, covered_seq) {
            Ok(upload) => upload,
            Err(e) => {
                tracing::warn!("Sync: SendSnapshot for {document_id:?} dropped: {e}");
                return;
            }
        };
        // Off the wasm path (host tests / mock relay) there is no event queue
        // and no real S3: sealing is the observable behavior, stop here.
        let Some(tx) = self.event_tx.clone() else {
            tracing::info!(
                "Sync: SendSnapshot {document_id:?} sealed ({} bytes), no channel",
                upload.draft.size().get()
            );
            return;
        };
        let base = crate::http::Url::<crate::http::ApiRoot>::of(self.url_scheme, &self.api_base);
        let auth = self.auth_context();
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

    /// Seal `payload` as a snapshot of `document_id` that folds in every delta
    /// up to `covered_seq`, ready to upload.
    ///
    /// # Errors
    /// Before the session keys, for a document this worker does not hold, or
    /// when the seal fails.
    pub(crate) fn snapshot_upload(
        &self,
        document_id: Id<Document>,
        payload: &ambre_core::Payload<ambre_core::SnapshotBytes>,
        covered_seq: Seq<Relay>,
    ) -> Result<SnapshotUpload, crate::error::SyncError> {
        use crate::error::SyncError;
        let signing_key = self.signing_key.as_ref().ok_or(SyncError::SessionLocked)?;
        let pub_key = self.author_pub_key().ok_or(SyncError::SessionLocked)?;
        let author_pubkey = pub_key;
        let doc_state = self.documents.get(&document_id).ok_or(SyncError::NotOpen)?;
        // No snapshot is sealed under a key a walk owed may disown
        // (`SYNC-232`). The Logic thread folds again later.
        let doc_key = doc_state.sealing_key()?;
        let public_data = SnapshotPublicData {
            document_id,
            pub_key,
            key_id: doc_key.id(),
            // The draft measures under this placeholder. The upload seals
            // the identifier the relay mints for its link (`SYNC-258`).
            snapshot_id: doc_state.current_snapshot_id,
            // The relay records this horizon in the row, and an open refuses
            // a row that names another (`SYNC-259`).
            covered_seq,
            // The BLAKE2b parent chain is a post-MVP hardening (see
            // ambre-spec-sealed-envelope.md). An empty proof seals a standalone snapshot,
            // which the compaction model accepts for the genesis-parented MVP.
            parent_snapshot_proof: None,
            parent_snapshot_update_clocks: Vec::new(),
        };
        // The full sealed `Snapshot`, serialized as the S3 blob (the inverse of
        // `decode_snapshot_blob` + `open_snapshot`).
        let draft = pipeline::SnapshotDraft::new(
            payload.clone(),
            public_data,
            doc_key.clone(),
            signing_key.clone(),
        )?;
        // The Logic thread names the last delta it holds with no gap below it,
        // not the high-water `last_seq`. A snapshot claims every delta up to
        // its horizon, and a checker holds it to that claim (`AUTHOR-016`).
        Ok(SnapshotUpload {
            document_id,
            author_pubkey,
            draft,
        })
    }

    pub(crate) fn handle_send_ephemeral(
        &mut self,
        document_id: Id<Document>,
        payload: ambre_core::Payload<ambre_core::EphemeralBytes>,
    ) {
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
        // A presence sample is not worth a hold: one sealed while the log
        // walks again is dropped (`SYNC-232`).
        let Ok(doc_key) = doc_state.sealing_key().cloned() else {
            return;
        };
        // A wrapped counter would read as a replay to every peer, so an
        // exhausted counter drops the ephemeral instead.
        let Some(next) = doc_state.ephemeral_counter.get().checked_add(1) else {
            tracing::warn!("Sync: ephemeral counter exhausted for {document_id:?}, dropping");
            return;
        };
        doc_state.ephemeral_counter =
            ambre_protocol::messages::Seq::<ambre_protocol::messages::Ephemerals>::new(next);
        let public_data = EphemeralPublicData {
            document_id,
            pub_key,
            key_id: doc_key.id(),
            counter: doc_state.ephemeral_counter,
        };
        match pipeline::ephemeral_frame(&payload, public_data, &doc_key, &signing_key) {
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
    /// It contradicts them: the signature of the report (`SRV-133`).
    Report(FixedBytes<Signature>),
}

/// Posts `vouch` on `snapshot_id` to the relay at `base`, under the
/// session's token. A confirmation that settled a fold names the amendments
/// the relay erased (`SRV-211`).
pub(crate) async fn send_vouch(
    transport: &impl http::Transport,
    auth: &http::AuthContext,
    base: &crate::http::Url<crate::http::ApiRoot>,
    document_id: Id<Document>,
    snapshot_id: Id<Snapshot>,
    vouch: &Vouch,
) -> Result<Option<Id<Document>>, http::HttpError> {
    let url = base.route(format!("/documents/{document_id}/snapshots/{snapshot_id}"));
    let post = http::Method::Post;
    match vouch {
        Vouch::Confirm => {
            let body = serde_json::json!({});
            let confirmed: http::ConfirmSnapshotResponse =
                http::post_json_auth_via(transport, &url.join("/confirm"), &body, auth).await?;
            Ok(confirmed.folded)
        }
        Vouch::Report(signature) => {
            let body = http::ReportSnapshotRequest {
                signature: *signature,
            };
            http::send_json_auth_via(transport, post, &url.join("/report"), &body, auth).await?;
            Ok(None)
        }
    }
}

/// One sealed snapshot on its way to the relay.
pub(crate) struct SnapshotUpload {
    /// The document it compacts.
    pub document_id: Id<Document>,
    /// Its author's signing key.
    pub author_pubkey: FixedBytes<ambre_core::PublicKey>,
    /// The snapshot, measured, to seal under the identifier the relay mints.
    pub draft: pipeline::SnapshotDraft,
}

/// Uploads a sealed snapshot, then confirms it as its author (`AUTHOR-021`).
/// The steps run in order: the upload link, the object, the metadata, the
/// confirmation. Returns the id the relay gave it, or the first step that
/// failed. A failed confirmation leaves the snapshot pending, so it is
/// logged and the id is still returned.
pub(crate) async fn upload_snapshot(
    transport: &impl http::Transport,
    auth: &http::AuthContext,
    base: &crate::http::Url<crate::http::ApiRoot>,
    upload: SnapshotUpload,
) -> Result<Id<Snapshot>, http::HttpError> {
    let SnapshotUpload {
        document_id,
        author_pubkey,
        draft,
    } = upload;
    let link: http::SnapshotUploadUrlResponse = http::post_json_auth_via(
        transport,
        &base.route(format!("/documents/{document_id}/snapshot/upload-url")),
        &serde_json::json!({ "size_bytes": draft.size() }),
        auth,
    )
    .await?;
    // The blob seals the identifier the relay minted for this link, so an
    // open can refuse another snapshot served in its place (`SYNC-258`).
    let blob = draft
        .seal(link.snapshot_id)
        .map_err(|_| http::HttpError::Local(ambre_core::LocalFailure::Crypto))?;
    // The presigned link is its own credential: no session token goes there.
    http::put_sealed_via(transport, &link.upload_url, &blob).await?;
    let metadata = http::CreateSnapshotRequest {
        author_pubkey,
        s3_key: link.s3_key,
        // The metadata names the horizon the blob seals, from the one
        // source, so the row the relay serves matches it (`SYNC-259`).
        covered_seq: draft.covered_seq(),
    };
    let created: http::CreateSnapshotResponse = http::post_json_auth_via(
        transport,
        &base.route(format!("/documents/{document_id}/snapshot")),
        &metadata,
        auth,
    )
    .await?;
    let snapshot_id = created.snapshot_id;
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

/// What an open whose log passed adds to the roots of the session: the
/// members the log keeps, for a Murmure open alone (`SYNC-237`). A Prose,
/// Trame or Toile open stands on its own link, and roots nothing.
fn roots_vouched(
    doc_type: DocType,
    members: Vec<FixedBytes<ambre_core::InviterPin>>,
) -> Option<SyncEvent> {
    match doc_type {
        DocType::Murmure => Some(SyncEvent::RootsVouched { roots: members }),
        DocType::Prose | DocType::Trame | DocType::Toile => None,
    }
}

/// What the open of a document from the relay carries off the select loop.
struct RelayOpen {
    api_base: crate::http::Url<crate::http::RelayHost>,
    url_scheme: crate::http::UrlScheme,
    doc_id: Id<Document>,
    doc_type: DocType,
    exchange_key: Option<StaticSecret>,
    auth: http::AuthContext,
    /// Picks our own row out of the members answer. `None` only before the
    /// session is ready, which the open waits for on the wasm path.
    my_user_id: Option<Id<User>>,
    rule: membership_log::LogRule,
    /// The document whose log and links vouch for the key: the `meta` of the
    /// channel that groups the document, or the document itself (`SYNC-229`).
    log_document: Id<Document>,
    /// The key of a snapshot this user confirmed alone (`AUTHOR-022`).
    own_key: Option<FixedBytes<PublicKey>>,
    /// The horizon of the local copy, when one exists (`RESID-010`).
    resume: Option<Seq<Relay>>,
    /// What this client holds out of band to root the log on (`SYNC-205`,
    /// `SYNC-210`, `SYNC-235`).
    roots: crate::key_vouch::Roots,
    /// The Logic thread, which takes the pin the log proved to the local
    /// store (`SYNC-238`).
    logic_tx: LogicSink,
}

/// What an open reads out of one members answer (`SYNC-092`).
pub(crate) struct OpenedMembers {
    pub doc_key: DocumentKey,
    pub signing_keys: Vec<(FixedBytes<ambre_core::PublicKey>, Id<User>)>,
    pub role: Role,
    pub rotation_owed: ambre_core::RotationDebt,
    /// The keys the membership log lets sign a delta or a snapshot
    /// (`SYNC-211`).
    pub writers: membership_log::Writers,
    /// The place of the latest departure the log holds (`SYNC-223`).
    pub departed: Option<Seq<ambre_protocol::LogPlaces>>,
    /// Who may sign a key link, head by head (`SYNC-204`).
    pub link_signers: membership_log::LinkSigners,
    /// The key the entry that added us names, when the log verifies our row
    /// (`SYNC-123`).
    pub named_key: Option<FixedBytes<ambre_core::KeyId>>,
    /// What the log says of the key and of its wrapper (`SYNC-200`,
    /// `SYNC-201`).
    pub vouch: crate::key_vouch::KeyVouch,
    /// What the log says a channel listed in this space must heed.
    pub word: membership_log::SpaceWord,
    /// The heads of the log the answer served (`SYNC-207`).
    pub heads: membership_log::ServedHeads,
    /// The pins of the members the log keeps (`SYNC-237`).
    pub members: Vec<FixedBytes<ambre_core::InviterPin>>,
}

/// What the members request says about a document being opened (`SYNC-040`).
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum MembersAnswer<T> {
    /// The server holds the document, and its members ride the answer.
    Exists(T),
    /// The server answered that it holds no such document: the open is local-only.
    Absent,
    /// The request failed, so nothing is known about the document. The
    /// relay's answer, a refusal among them, is kept whole.
    Failed(crate::http::HttpError),
}

impl<T> MembersAnswer<T> {
    /// Classifies the outcome of the members request. Only a `404` is an absent
    /// document; a transport or decoding failure is never read as one.
    pub(crate) fn classify(outcome: Result<Option<T>, crate::http::HttpError>) -> Self {
        match outcome {
            Ok(Some(members)) => Self::Exists(members),
            Ok(None) => Self::Absent,
            Err(error) => Self::Failed(error),
        }
    }
}

/// Why an open that met `error` on its members request failed. A relay that
/// refused the request itself refused the open. Any other failure may clear
/// on a later open (`SYNC-173`).
pub(crate) fn members_failure_cause(error: &crate::http::HttpError) -> OpenFailure {
    match error.failure() {
        ApiFailure::Relay { status, code } if status.is_client_error() => {
            OpenFailure::Refused { code }
        }
        _ => OpenFailure::Transient,
    }
}

/// What an open that met `error` on its members request logs. A relay that
/// refused, or whose answer does not read, was reached.
pub(crate) fn log_members_failure(document_id: Id<Document>, error: &crate::http::HttpError) {
    match error.failure() {
        ApiFailure::Relay { .. } => {
            tracing::warn!(
                "Sync: the relay refused the members request of {document_id:?}: {error}"
            );
        }
        ApiFailure::Unreadable { .. } => {
            tracing::warn!("Sync: the members answer of {document_id:?} does not read: {error}");
        }
        ApiFailure::Local(_) | ApiFailure::NoAnswer => {
            tracing::warn!("Sync: the server could not be reached for {document_id:?}: {error}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The quarantine bounds as the tests count updates.
    const QUARANTINE_LIMIT: usize = crate::QUARANTINE_LIMIT.get() as usize;
    const QUARANTINE_ALERT: usize = crate::QUARANTINE_ALERT.get() as usize;
    use crate::http::scripted::{Scripted, session};
    use ambre_core::{Ms, TimeSpan};
    use ambre_protocol::SeqUpdate;

    /// The root a relay call under test reaches: `https://relay.test/api`.
    fn root() -> crate::http::Url<crate::http::ApiRoot> {
        crate::http::Url::<crate::http::ApiRoot>::of(
            crate::http::UrlScheme::Https,
            &crate::http::Url::<crate::http::RelayHost>::new("relay.test".to_owned()),
        )
    }

    #[test]
    fn a_failed_members_request_is_not_an_absent_document() {
        let lost = crate::http::HttpError::Local(ambre_core::LocalFailure::NoRelay);
        let failed = MembersAnswer::<()>::classify(Err(lost.clone()));
        assert_eq!(failed, MembersAnswer::Failed(lost));
        assert_eq!(
            MembersAnswer::<()>::classify(Ok(None)),
            MembersAnswer::Absent
        );
        assert_eq!(
            MembersAnswer::classify(Ok(Some(7))),
            MembersAnswer::Exists(7)
        );
    }

    /// A relay that refuses the members request did answer. The open keeps
    /// its refusal, and never tells the user the relay could not be reached
    /// (`SYNC-040`).
    #[test]
    fn a_refused_members_request_keeps_the_relay_refusal() {
        let refused = crate::http::HttpError::refused(
            crate::http::Method::Get,
            &crate::http::Target::relay("https://relay.test/api/documents/d/members"),
            ambre_core::HttpStatus::new(403),
            Some(ambre_core::RefusalCode::Forbidden),
        );
        let answer = MembersAnswer::<()>::classify(Err(refused.clone()));
        assert_eq!(answer, MembersAnswer::Failed(refused.clone()));
        assert_eq!(
            members_failure_cause(&refused),
            OpenFailure::Refused {
                code: Some(ambre_core::RefusalCode::Forbidden)
            }
        );
        assert!(matches!(refused.failure(), ApiFailure::Relay { .. }));
        let lost = crate::http::HttpError::Local(ambre_core::LocalFailure::NoRelay);
        assert_eq!(members_failure_cause(&lost), OpenFailure::Transient);
    }

    /// A relay that refuses the members request with a `4xx` refused the
    /// open. Asking again meets the same answer, so the open fails as
    /// refused, with the relay's code. A lost answer, or a `5xx`, stays
    /// transient (`SYNC-173`).
    #[test]
    fn a_refused_members_request_fails_the_open_as_refused() {
        let at = crate::http::Target::relay("https://relay.test/api/documents/d/members");
        let refused = crate::http::HttpError::refused(
            crate::http::Method::Get,
            &at,
            ambre_core::HttpStatus::new(403),
            Some(ambre_core::RefusalCode::Forbidden),
        );
        assert_eq!(
            members_failure_cause(&refused),
            OpenFailure::Refused {
                code: Some(ambre_core::RefusalCode::Forbidden)
            }
        );
        let down = crate::http::HttpError::refused(
            crate::http::Method::Get,
            &at,
            ambre_core::HttpStatus::new(503),
            None,
        );
        assert_eq!(members_failure_cause(&down), OpenFailure::Transient);
        let lost = crate::http::HttpError::Local(ambre_core::LocalFailure::NoRelay);
        assert_eq!(members_failure_cause(&lost), OpenFailure::Transient);
    }

    /// One members row for `uid`, its key wrapped by `wrapper` for `holder`.
    fn member_row(
        uid: Id<User>,
        holder: &x25519_dalek::StaticSecret,
        wrapper: &x25519_dalek::StaticSecret,
        doc_key: &DocumentKey,
        role: ambre_core::Role,
    ) -> http::MemberEntry {
        let holder_pub = crate::sharing::own_exchange_key(holder);
        let wrapper_pub = crate::sharing::own_exchange_key(wrapper);
        http::MemberEntry {
            user_id: uid,
            signing_pub_key: http::ServedKey::of(FixedBytes::from_array([uid.as_bytes()[0]; 32])),
            exchange_pub_key: http::ServedKey::of(holder_pub),
            doc_key_enc: crate::sharing::wrap_anchored_b64(wrapper, &holder_pub, doc_key, None)
                .unwrap(),
            role,
            wrapper_exchange_pub_key: http::ServedKey::of(wrapper_pub),
        }
    }

    /// Holds `SYNC-092`: the key, the directory and the role all come out of
    /// the one answer handed in, and nothing else is consulted.
    #[test]
    fn the_key_the_directory_and_the_role_come_from_one_answer() {
        let me = Id::<User>::mint();
        let peer = Id::<User>::mint();
        let my_xk = ambre_crypto::MasterSeed::from_array([3; 32]).derive_exchange_key();
        let peer_xk = ambre_crypto::MasterSeed::from_array([4; 32]).derive_exchange_key();
        let doc_key = DocumentKey::generate();
        let resp = http::MembersResponse {
            members: vec![
                member_row(me, &my_xk, &peer_xk, &doc_key, ambre_core::Role::Editor),
                member_row(peer, &peer_xk, &peer_xk, &doc_key, ambre_core::Role::Owner),
            ],
            rotation_owed: ambre_core::RotationDebt::Owed,
            log: Vec::new(),
        };
        let opened = SyncState::opened_from_members(
            &resp,
            Id::<Document>::mint(),
            &my_xk,
            Some(me),
            &membership_log::LogRule::document(),
            &crate::key_vouch::Roots::default(),
        )
        .unwrap();
        assert!(opened.doc_key.id() == doc_key.id());
        assert_eq!(opened.role, Role::Editor);
        let ids: Vec<Id<User>> = opened.signing_keys.iter().map(|(_, id)| *id).collect();
        assert_eq!(ids, vec![me, peer]);
        assert_eq!(opened.rotation_owed, ambre_core::RotationDebt::Owed);
    }

    /// Holds `SYNC-012`: a key row that does not unwrap fails the open, and no
    /// key is generated in its place.
    #[test]
    fn a_key_that_does_not_unwrap_opens_nothing() {
        let me = Id::<User>::mint();
        let my_xk = ambre_crypto::MasterSeed::from_array([3; 32]).derive_exchange_key();
        let peer_xk = ambre_crypto::MasterSeed::from_array([4; 32]).derive_exchange_key();
        let mut row = member_row(
            me,
            &my_xk,
            &peer_xk,
            &DocumentKey::generate(),
            ambre_core::Role::Editor,
        );
        let mut raw = row.doc_key_enc.as_bytes().to_vec();
        let last = raw.len() - 1;
        raw[last] ^= 1;
        row.doc_key_enc = ambre_core::Encoded::new(raw.to_vec());
        let resp = http::MembersResponse {
            members: vec![row],
            rotation_owed: ambre_core::RotationDebt::Settled,
            log: Vec::new(),
        };
        let refused = SyncState::opened_from_members(
            &resp,
            Id::<Document>::mint(),
            &my_xk,
            Some(me),
            &membership_log::LogRule::document(),
            &crate::key_vouch::Roots::default(),
        );
        // The open fails as a key that has not reached this device
        // (`SYNC-173`).
        assert!(matches!(refused, Err((OpenFailure::KeyNotReceived, _))));
    }

    /// Holds `SYNC-124`: a key the invitation does not name is refused, unless
    /// a link reaches the named key from it, as after a rotation. A log that
    /// does not verify our row names no key, and the open goes on (`SYNC-111`).
    #[test]
    fn a_key_the_invitation_does_not_name_is_refused() {
        let document_id = Id::<Document>::mint();
        let named = DocumentKey::generate();
        let rotated = DocumentKey::generate();
        let signer = ambre_crypto::MasterSeed::from_array([5; 32]).derive_signing_key();
        let link = crate::pipeline::seal_link(
            document_id,
            &named,
            &rotated,
            ambre_protocol::LogHead::EMPTY,
            &signer,
        )
        .unwrap();
        let held = |key: &DocumentKey, links: &[ambre_protocol::KeyLink]| {
            SyncState::named_key_holds(document_id, key, Some(named.id()), links)
        };
        assert!(held(&named, &[]).is_ok());
        assert!(held(&rotated, std::slice::from_ref(&link)).is_ok());
        let swapped = held(&DocumentKey::generate(), &[link]);
        assert!(matches!(swapped, Err((OpenFailure::KeyNotReceived, _))));
        assert!(SyncState::named_key_holds(document_id, &rotated, None, &[]).is_ok());
    }

    /// Holds `SYNC-200` and `SYNC-201` at the open. A relay wraps a key of its
    /// own for Bob with the genuine anchor. It puts its exchange key on
    /// Alice's row and drops Bob's invite, so the invite check stays silent. The
    /// unwrap succeeds, and the open still refuses the key. The same swap under
    /// the whole log, for the key Alice named, is refused too.
    #[test]
    fn a_key_the_relay_substitutes_is_refused_at_open() {
        use crate::membership_log::tests::Person;
        use crate::membership_log::{Draft, LogHead, digest, sign};
        let document_id = Id::<Document>::mint();
        let (alice, bob) = (Person::new(0x31), Person::new(0x32));
        let alice_xk = ambre_crypto::MasterSeed::from_array([0x31; 32]).derive_exchange_key();
        let bob_xk = ambre_crypto::MasterSeed::from_array([0x32; 32]).derive_exchange_key();
        let k0 = DocumentKey::generate();
        let draft = |member: &Person, xk: &x25519_dalek::StaticSecret, act, role| Draft {
            document: document_id,
            member: member.id,
            member_key: member.public(),
            member_exchange_key: crate::sharing::own_exchange_key(xk),
            act,
            role,
            key: Some(k0.id()),
        };
        let create = sign(
            draft(
                &alice,
                &alice_xk,
                ambre_protocol::MembershipAct::Create,
                Role::Owner,
            ),
            LogHead::EMPTY,
            alice.id,
            &alice.key,
        )
        .unwrap();
        let anchor = digest(&create).unwrap();
        let invite = sign(
            draft(
                &bob,
                &bob_xk,
                ambre_protocol::MembershipAct::Invite,
                Role::Editor,
            ),
            LogHead::EMPTY.after(anchor),
            alice.id,
            &alice.key,
        )
        .unwrap();
        let row = |who: &Person, holder: &x25519_dalek::StaticSecret, wrapper, key| {
            let holder_pub = crate::sharing::own_exchange_key(holder);
            http::MemberEntry {
                user_id: who.id,
                signing_pub_key: http::ServedKey::of(who.public()),
                exchange_pub_key: http::ServedKey::of(holder_pub),
                doc_key_enc: crate::sharing::wrap_anchored_b64(
                    wrapper,
                    &holder_pub,
                    key,
                    Some(anchor),
                )
                .unwrap(),
                role: Role::Editor,
                wrapper_exchange_pub_key: http::ServedKey::of(crate::sharing::own_exchange_key(
                    wrapper,
                )),
            }
        };
        // Bob opens from the link Alice sent him (`SYNC-205`).
        let alice_pin = ambre_crypto::inviter_pin(&alice.public());
        let opens = |resp: &http::MembersResponse| {
            let opened = SyncState::opened_from_members(
                resp,
                document_id,
                &bob_xk,
                Some(bob.id),
                &membership_log::LogRule::document(),
                &crate::key_vouch::Roots {
                    pin: Some(alice_pin),
                    ..Default::default()
                },
            )
            .unwrap();
            SyncState::named_key_holds(document_id, &opened.doc_key, opened.named_key, &[])
                .map_err(|(cause, _)| cause)?;
            opened
                .vouch
                .check(document_id, &opened.doc_key, &[])
                .map_err(crate::key_vouch::KeyRefusal::open_failure)
        };
        let encoded = |entries: &[&ambre_protocol::MembershipEntry]| {
            entries
                .iter()
                .map(|e| membership_log::encode(e).unwrap())
                .collect()
        };
        let honest = http::MembersResponse {
            members: vec![
                row(&alice, &alice_xk, &alice_xk, &k0),
                row(&bob, &bob_xk, &alice_xk, &k0),
            ],
            rotation_owed: ambre_core::RotationDebt::Settled,
            log: encoded(&[&create, &invite]),
        };
        assert_eq!(opens(&honest), Ok(()));

        let relay_xk = ambre_crypto::MasterSeed::from_array([0x66; 32]).derive_exchange_key();
        let chosen = DocumentKey::generate();
        let mut alice_row = row(&alice, &alice_xk, &alice_xk, &k0);
        alice_row.exchange_pub_key =
            http::ServedKey::of(crate::sharing::own_exchange_key(&relay_xk));
        let substituted = http::MembersResponse {
            members: vec![alice_row, row(&bob, &bob_xk, &relay_xk, &chosen)],
            rotation_owed: ambre_core::RotationDebt::Settled,
            log: encoded(&[&create]),
        };
        // The log signs Alice's true exchange key, so the wrapper the relay
        // put on her row is refused first (`SYNC-201`).
        assert_eq!(opens(&substituted), Err(OpenFailure::KeyNotReceived));

        // A copy of a key no writer named, under a wrapper the log verifies.
        // No member signed that key, so the open names the relay's fault,
        // never a key still to arrive (`SYNC-212`).
        let unnamed = http::MembersResponse {
            members: vec![
                row(&alice, &alice_xk, &alice_xk, &k0),
                row(&bob, &bob_xk, &alice_xk, &chosen),
            ],
            rotation_owed: ambre_core::RotationDebt::Settled,
            log: encoded(&[&create]),
        };
        assert_eq!(opens(&unnamed), Err(OpenFailure::KeyUnsigned));

        // The relay serves the whole log, and swaps the exchange key on
        // Alice's row for its own. The log signs Alice's true key, so the
        // copy the relay wrapped is refused, even for the key Alice named.
        let mut swapped_row = row(&alice, &alice_xk, &alice_xk, &k0);
        swapped_row.exchange_pub_key =
            http::ServedKey::of(crate::sharing::own_exchange_key(&relay_xk));
        let swapped = http::MembersResponse {
            members: vec![swapped_row, row(&bob, &bob_xk, &relay_xk, &k0)],
            rotation_owed: ambre_core::RotationDebt::Settled,
            log: encoded(&[&create, &invite]),
        };
        assert_eq!(opens(&swapped), Err(OpenFailure::KeyNotReceived));
    }

    /// Holds `SYNC-226` at the open (audit `P14`). Alice invites Bob and
    /// Carol under `k0`, then revokes Carol under `k1`. At Bob's first open
    /// the relay serves his genuine copy of `k0` and withholds the link from
    /// `k1`. The open fails as a key not received, and the honest answer,
    /// `k1` with its link, opens.
    #[test]
    fn a_first_open_refuses_a_key_behind_the_latest_key_the_log_names() {
        use crate::membership_log::tests::Person;
        use crate::membership_log::{Draft, LogHead, digest, sign};
        use ambre_protocol::MembershipAct;
        let document_id = Id::<Document>::mint();
        let people = [0x31, 0x32, 0x33].map(|seed| {
            let xk = ambre_crypto::MasterSeed::from_array([seed; 32]).derive_exchange_key();
            (Person::new(seed), xk)
        });
        let [(alice, alice_xk), (bob, bob_xk), (carol, carol_xk)] = &people;
        let (k0, k1) = (DocumentKey::generate(), DocumentKey::generate());
        let mut entries: Vec<ambre_protocol::MembershipEntry> = Vec::new();
        let steps = [
            (alice, alice_xk, MembershipAct::Create, Role::Owner, &k0),
            (bob, bob_xk, MembershipAct::Invite, Role::Editor, &k0),
            (carol, carol_xk, MembershipAct::Invite, Role::Editor, &k0),
            (carol, carol_xk, MembershipAct::Revoke, Role::Editor, &k1),
        ];
        for (member, xk, act, role, key) in steps {
            let draft = Draft {
                document: document_id,
                member: member.id,
                member_key: member.public(),
                member_exchange_key: crate::sharing::own_exchange_key(xk),
                act,
                role,
                key: Some(key.id()),
            };
            let head = entries
                .iter()
                .fold(LogHead::EMPTY, |head, e| head.after(digest(e).unwrap()));
            entries.push(sign(draft, head, alice.id, &alice.key).unwrap());
        }
        let anchor = digest(&entries[0]).unwrap();
        let row = |who: &Person, holder: &x25519_dalek::StaticSecret, key: &DocumentKey| {
            let holder_pub = crate::sharing::own_exchange_key(holder);
            http::MemberEntry {
                user_id: who.id,
                signing_pub_key: http::ServedKey::of(who.public()),
                exchange_pub_key: http::ServedKey::of(holder_pub),
                doc_key_enc: crate::sharing::wrap_anchored_b64(
                    alice_xk,
                    &holder_pub,
                    key,
                    Some(anchor),
                )
                .unwrap(),
                role: Role::Editor,
                wrapper_exchange_pub_key: http::ServedKey::of(crate::sharing::own_exchange_key(
                    alice_xk,
                )),
            }
        };
        let alice_pin = ambre_crypto::inviter_pin(&alice.public());
        let opens = |key: &DocumentKey, links: &[ambre_protocol::KeyLink]| {
            let resp = http::MembersResponse {
                members: vec![row(alice, alice_xk, key), row(bob, bob_xk, key)],
                rotation_owed: ambre_core::RotationDebt::Settled,
                log: entries
                    .iter()
                    .map(|e| membership_log::encode(e).unwrap())
                    .collect(),
            };
            let opened = SyncState::opened_from_members(
                &resp,
                document_id,
                bob_xk,
                Some(bob.id),
                &membership_log::LogRule::document(),
                &crate::key_vouch::Roots {
                    pin: Some(alice_pin),
                    signer: None,
                    ..Default::default()
                },
            )
            .unwrap();
            SyncState::named_key_holds(document_id, &opened.doc_key, opened.named_key, links)
                .map_err(|(cause, _)| cause)?;
            opened
                .vouch
                .check(document_id, &opened.doc_key, links)
                .map_err(|_| OpenFailure::KeyNotReceived)
        };
        assert_eq!(opens(&k0, &[]), Err(OpenFailure::KeyNotReceived));
        let head = entries
            .iter()
            .fold(LogHead::EMPTY, |head, e| head.after(digest(e).unwrap()));
        let link = crate::pipeline::seal_link(document_id, &k0, &k1, head, &alice.key).unwrap();
        assert_eq!(opens(&k1, &[link]), Ok(()));
    }

    /// Holds `SYNC-229`. A segment serves the rows, the links and the log of
    /// its `meta`. Its open names that `meta`. So the walk grants the meta's
    /// writers, and the chain keeps the meta's link back to the key the
    /// invite named. The key checks pass.
    ///
    /// A key the relay wraps itself is refused. An open that names no `meta`
    /// walks nothing and reaches no key.
    #[test]
    fn a_channel_document_opens_under_the_log_and_links_of_its_meta() {
        use crate::membership_log::tests::Person;
        use crate::membership_log::{Draft, LogHead, digest, sign};
        let (sink, _rx) = LogicSink::channel();
        let mut state = SyncState::new(
            sink,
            crate::http::Url::<crate::RelayHost>::new(String::new()),
        );
        let meta = Id::<Meta>::mint();
        let segment = Id::<Document>::mint();
        let (alice, bob) = (Person::new(0x41), Person::new(0x42));
        let alice_xk = ambre_crypto::MasterSeed::from_array([0x41; 32]).derive_exchange_key();
        let bob_xk = ambre_crypto::MasterSeed::from_array([0x42; 32]).derive_exchange_key();
        let (k0, k1) = (DocumentKey::generate(), DocumentKey::generate());
        let draft = |member: &Person, xk: &x25519_dalek::StaticSecret, act, role| Draft {
            document: meta.document_id(),
            member: member.id,
            member_key: member.public(),
            member_exchange_key: crate::sharing::own_exchange_key(xk),
            act,
            role,
            key: Some(k0.id()),
        };
        let create = sign(
            draft(
                &alice,
                &alice_xk,
                ambre_protocol::MembershipAct::Create,
                Role::Owner,
            ),
            LogHead::EMPTY,
            alice.id,
            &alice.key,
        )
        .unwrap();
        let anchor = digest(&create).unwrap();
        let invite = sign(
            draft(
                &bob,
                &bob_xk,
                ambre_protocol::MembershipAct::Invite,
                Role::Editor,
            ),
            LogHead::EMPTY.after(anchor),
            alice.id,
            &alice.key,
        )
        .unwrap();
        // Alice rotated the channel: the link lives under the meta.
        let head = LogHead::EMPTY
            .after(anchor)
            .after(membership_log::digest(&invite).unwrap());
        let link = pipeline::seal_link(meta.document_id(), &k0, &k1, head, &alice.key).unwrap();
        let row = |who: &Person, holder: &x25519_dalek::StaticSecret, wrapper, key| {
            let holder_pub = crate::sharing::own_exchange_key(holder);
            http::MemberEntry {
                user_id: who.id,
                signing_pub_key: http::ServedKey::of(who.public()),
                exchange_pub_key: http::ServedKey::of(holder_pub),
                doc_key_enc: crate::sharing::wrap_anchored_b64(
                    wrapper,
                    &holder_pub,
                    key,
                    Some(anchor),
                )
                .unwrap(),
                role: Role::Editor,
                wrapper_exchange_pub_key: http::ServedKey::of(crate::sharing::own_exchange_key(
                    wrapper,
                )),
            }
        };
        let log = vec![
            membership_log::encode(&create).unwrap(),
            membership_log::encode(&invite).unwrap(),
        ];
        let served = |members| http::MembersResponse {
            members,
            rotation_owed: ambre_core::RotationDebt::Settled,
            log: log.clone(),
        };
        let honest = served(vec![
            row(&alice, &alice_xk, &alice_xk, &k1),
            row(&bob, &bob_xk, &alice_xk, &k1),
        ]);
        // Bob opened the Murmure link Alice sent him (`SYNC-235`).
        state.handle_command(ambre_bus::SyncCommand::PinMurmureInviter {
            inviter: ambre_crypto::inviter_pin(&alice.public()),
        });
        let roots = state.roots(segment, DocType::Murmure);
        let opens = |resp: &http::MembersResponse, log_document: Id<Document>| {
            let opened = SyncState::opened_from_members(
                resp,
                log_document,
                &bob_xk,
                Some(bob.id),
                &membership_log::LogRule::document(),
                &roots,
            )
            .unwrap();
            let links = crate::chain::signed_by_writers(vec![link.clone()], &opened.link_signers);
            SyncState::named_key_holds(log_document, &opened.doc_key, opened.named_key, &links)
                .map_err(|(cause, _)| cause)?;
            opened
                .vouch
                .check(log_document, &opened.doc_key, &links)
                .map_err(|_| OpenFailure::KeyNotReceived)?;
            Ok::<_, OpenFailure>(opened.writers)
        };

        // The open names the meta, as the Logic thread read it.
        state.handle_command(ambre_bus::SyncCommand::OpenDocument {
            document_id: segment,
            doc_type: DocType::Murmure,
            meta: Some(meta),
        });
        assert_eq!(state.log_document(segment), meta.document_id());
        let writers = opens(&honest, state.log_document(segment)).unwrap();
        assert!(writers.admits(&alice.public()) && writers.admits(&bob.public()));
        assert!(!writers.admits(&Person::new(0x43).public()));

        // The relay re-wraps a key of its own for Bob, on Alice's row.
        let relay_xk = ambre_crypto::MasterSeed::from_array([0x66; 32]).derive_exchange_key();
        let chosen = DocumentKey::generate();
        let mut alice_row = row(&alice, &alice_xk, &alice_xk, &chosen);
        alice_row.exchange_pub_key =
            http::ServedKey::of(crate::sharing::own_exchange_key(&relay_xk));
        let substituted = served(vec![alice_row, row(&bob, &bob_xk, &relay_xk, &chosen)]);
        assert_eq!(
            opens(&substituted, meta.document_id()).err(),
            Some(OpenFailure::KeyNotReceived)
        );

        // Walked under the segment itself, the log grants nobody and no
        // link opens: no log names the key.
        assert_eq!(
            opens(&honest, segment).err(),
            Some(OpenFailure::KeyNotReceived)
        );

        // The session the open makes keeps the meta's link in its chain.
        state.handle_sync_event(SyncEvent::OpenDocumentReady {
            document_id: segment,
            doc_key: k1.clone(),
            snapshot_payload: None,
            snapshot_id: Id::<Snapshot>::genesis(segment),
            covered_seq: Seq::<Relay>::ZERO,
            held_snapshot: None,
            doc_type: DocType::Murmure,
            existence: crate::Existence::OnRelay,
            signing_keys: Vec::new(),
            role: Role::Editor,
            rotation_owed: ambre_core::RotationDebt::Settled,
            links: vec![link.clone()],
            writers,
            departed: None,
        });
        let opened = state.documents[&segment].key_named(k0.id());
        assert_eq!(opened.map(|key| key.id()), Some(k0.id()));
        // A close forgets the meta: a later open names it again.
        state.handle_command(ambre_bus::SyncCommand::CloseDocument {
            document_id: segment,
        });
        assert_eq!(state.log_document(segment), segment);
    }

    /// Holds `SYNC-236` and `SYNC-237` on the Sync loop. The pin of a Murmure
    /// link roots every Murmure open of the session, and no Prose open. The
    /// members of an accepted Murmure log join the roots, while an accepted
    /// Prose log adds none.
    #[test]
    fn a_murmure_link_and_an_accepted_murmure_log_root_the_murmure_opens_alone() {
        let (sink, _rx) = LogicSink::channel();
        let mut state = SyncState::new(
            sink,
            crate::http::Url::<crate::RelayHost>::new(String::new()),
        );
        let document_id = Id::<Document>::mint();
        let pin = FixedBytes::<ambre_core::InviterPin>::from_array([4; 32]);
        state.handle_command(ambre_bus::SyncCommand::PinMurmureInviter { inviter: pin });
        assert_eq!(
            state.roots(document_id, DocType::Murmure).session,
            vec![pin]
        );
        for doc_type in [DocType::Prose, DocType::Trame, DocType::Toile] {
            let roots = state.roots(document_id, doc_type);
            assert!(roots.session.is_empty() && roots.pin.is_none());
        }
        let member = FixedBytes::<ambre_core::InviterPin>::from_array([5; 32]);
        assert!(roots_vouched(DocType::Prose, vec![member]).is_none());
        let Some(vouched) = roots_vouched(DocType::Murmure, vec![member]) else {
            panic!("a Murmure log roots its members");
        };
        state.handle_sync_event(vouched);
        let mut session = state.roots(document_id, DocType::Murmure).session;
        session.sort();
        let mut expected = vec![pin, member];
        expected.sort();
        assert_eq!(session, expected);

        // An invite this client signed into a Murmure document roots its
        // invitee. One into a Prose document roots nobody.
        let invited = |state: &mut SyncState, doc_type, seed| {
            let document_id = Id::<Document>::mint();
            state.documents.insert(
                document_id,
                SyncDocumentState::new(
                    document_id,
                    DocumentKey::generate(),
                    doc_type,
                    crate::Existence::OnRelay,
                ),
            );
            let invitee = FixedBytes::<ambre_core::InviterPin>::from_array([seed; 32]);
            state.handle_sync_event(SyncEvent::InviteSent {
                id: ambre_bus::next_request(),
                document_id,
                user_id: Id::<User>::mint(),
                display_name: ambre_core::display_handle(&Id::<User>::mint()),
                inviter: pin,
                invitee,
            });
            state
                .roots(document_id, DocType::Murmure)
                .session
                .contains(&invitee)
        };
        assert!(invited(&mut state, DocType::Murmure, 6));
        assert!(!invited(&mut state, DocType::Prose, 7));
    }

    /// Holds `SYNC-241` on the Sync loop. A restarted worker holds no root
    /// of the session. The pin the Main thread hands back from the local
    /// store roots the Murmure open of its own document, and of no other.
    #[test]
    fn a_kept_murmure_pin_roots_its_document_alone_after_a_restart() {
        let (sink, _rx) = LogicSink::channel();
        let mut state = SyncState::new(
            sink,
            crate::http::Url::<crate::RelayHost>::new(String::new()),
        );
        let (space, other) = (Id::<Document>::mint(), Id::<Document>::mint());
        let kept = FixedBytes::<ambre_core::InviterPin>::from_array([9; 32]);
        let roots = state.roots(space, DocType::Murmure);
        assert!(roots.pin.is_none() && roots.session.is_empty());
        state.handle_command(ambre_bus::SyncCommand::PinInviter {
            document_id: space,
            inviter: kept,
        });
        assert_eq!(state.roots(space, DocType::Murmure).pin, Some(kept));
        let roots = state.roots(other, DocType::Murmure);
        assert!(roots.pin.is_none() && roots.session.is_empty());
    }

    /// Holds `SYNC-205` at the open. The relay builds a whole log of its own.
    /// A creator whose keys it holds invites Bob. The relay wraps a key of its
    /// own for him, under the anchor of its own creation. Pinned to Alice from
    /// her link, the open refuses that log and opens hers. Unpinned, it refuses
    /// both (`SYNC-210`).
    #[test]
    fn an_open_pinned_to_its_inviter_refuses_a_log_the_relay_rooted() {
        use crate::membership_log::tests::Person;
        use crate::membership_log::{Draft, LogHead, digest, sign};
        let document_id = Id::<Document>::mint();
        let bob = Person::new(0x32);
        let bob_xk = ambre_crypto::MasterSeed::from_array([0x32; 32]).derive_exchange_key();
        // The answer a creator of seed `seed` serves for Bob's open.
        let served_by = |seed: u8| {
            let creator = Person::new(seed);
            let creator_xk = ambre_crypto::MasterSeed::from_array([seed; 32]).derive_exchange_key();
            let key = DocumentKey::generate();
            let draft = |member: &Person, xk: &x25519_dalek::StaticSecret, act, role| Draft {
                document: document_id,
                member: member.id,
                member_key: member.public(),
                member_exchange_key: crate::sharing::own_exchange_key(xk),
                act,
                role,
                key: Some(key.id()),
            };
            let create = sign(
                draft(
                    &creator,
                    &creator_xk,
                    ambre_protocol::MembershipAct::Create,
                    Role::Owner,
                ),
                LogHead::EMPTY,
                creator.id,
                &creator.key,
            )
            .unwrap();
            let anchor = digest(&create).unwrap();
            let invite = sign(
                draft(
                    &bob,
                    &bob_xk,
                    ambre_protocol::MembershipAct::Invite,
                    Role::Editor,
                ),
                LogHead::EMPTY.after(anchor),
                creator.id,
                &creator.key,
            )
            .unwrap();
            let row = |who: &Person, holder: &x25519_dalek::StaticSecret| {
                let holder_pub = crate::sharing::own_exchange_key(holder);
                http::MemberEntry {
                    user_id: who.id,
                    signing_pub_key: http::ServedKey::of(who.public()),
                    exchange_pub_key: http::ServedKey::of(holder_pub),
                    doc_key_enc: crate::sharing::wrap_anchored_b64(
                        &creator_xk,
                        &holder_pub,
                        &key,
                        Some(anchor),
                    )
                    .unwrap(),
                    role: Role::Editor,
                    wrapper_exchange_pub_key: http::ServedKey::of(
                        crate::sharing::own_exchange_key(&creator_xk),
                    ),
                }
            };
            let resp = http::MembersResponse {
                members: vec![row(&creator, &creator_xk), row(&bob, &bob_xk)],
                rotation_owed: ambre_core::RotationDebt::Settled,
                log: [&create, &invite]
                    .iter()
                    .map(|e| membership_log::encode(e).unwrap())
                    .collect(),
            };
            (resp, ambre_crypto::inviter_pin(&creator.public()))
        };
        let opens = |resp: &http::MembersResponse,
                     pin: Option<&FixedBytes<ambre_core::InviterPin>>| {
            let opened = SyncState::opened_from_members(
                resp,
                document_id,
                &bob_xk,
                Some(bob.id),
                &membership_log::LogRule::document(),
                &crate::key_vouch::Roots {
                    pin: pin.copied(),
                    session: Vec::new(),
                    signer: Some(bob.public()),
                },
            )
            .unwrap();
            SyncState::named_key_holds(document_id, &opened.doc_key, opened.named_key, &[])
                .map_err(|(cause, _)| cause)?;
            opened
                .vouch
                .check(document_id, &opened.doc_key, &[])
                .map_err(crate::key_vouch::KeyRefusal::open_failure)
        };
        let (honest, alice_pin) = served_by(0x31);
        let (forged, _) = served_by(0x66);
        // With no link, the open fails closed on either log, since Bob
        // signed no entry of it (`SYNC-210`).
        assert_eq!(opens(&forged, None), Err(OpenFailure::KeyNotReceived));
        assert_eq!(opens(&honest, None), Err(OpenFailure::KeyNotReceived));
        // Pinned, the log Alice never signed names a key no member signed
        // (`SYNC-212`).
        assert_eq!(
            opens(&forged, Some(&alice_pin)),
            Err(OpenFailure::KeyUnsigned)
        );
        assert_eq!(opens(&honest, Some(&alice_pin)), Ok(()));
    }

    /// A `SyncState` with one opened document, a controlled `doc_key` and a known
    /// signing key, plus the drainable Logic receiver and the document's known
    /// snapshot id.
    fn setup() -> (
        SyncState,
        ambre_bus::conduit::Receiver<LogicEvent>,
        Id<Document>,
        DocumentKey,
        SigningKey,
        Id<Snapshot>,
    ) {
        let (sink, rx) = LogicSink::channel();
        let mut state = SyncState::new(
            sink,
            crate::http::Url::<crate::RelayHost>::new(String::new()),
        );
        let document_id = Id::<Document>::mint();
        let doc_key = DocumentKey::generate();
        let signing_key = MasterSeed::generate().derive_signing_key();
        let doc_state = SyncDocumentState::new(
            document_id,
            doc_key.clone(),
            DocType::Trame,
            crate::Existence::OnRelay,
        );
        let known_snapshot = doc_state.current_snapshot_id;
        state.documents.insert(document_id, doc_state);
        state.session_nonces.insert(document_id, TEST_NONCE);
        (state, rx, document_id, doc_key, signing_key, known_snapshot)
    }

    fn make_update(
        document_id: Id<Document>,
        snapshot_id: Id<Snapshot>,
        clock: Seq<DeltaAuthor>,
        payload: &[u8],
        doc_key: &DocumentKey,
        signing_key: &SigningKey,
    ) -> Update {
        let pub_key = FixedBytes::<PublicKey>::from_array(signing_key.verifying_key().to_bytes());
        let public_data = UpdatePublicData {
            document_id,
            pub_key,
            key_id: doc_key.id(),
            snapshot_id,
            clock,
            log_head: ambre_protocol::LogHead::EMPTY,
            log_entry: None,
        };
        let body =
            pipeline::update_body(&TEST_NONCE, None, &payload.to_vec().into()).expect("no summary");
        pipeline::seal_update(&body, public_data, doc_key, signing_key).expect("seal")
    }

    /// The session nonce every update of these tests carries.
    const TEST_NONCE: FixedBytes<SessionNonce> = FixedBytes::from_array([6; 16]);

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

        let under = |key: &DocumentKey| {
            make_update(
                document_id,
                snapshot,
                Seq::<DeltaAuthor>::new(1),
                b"d",
                key,
                &signing_key,
            )
        };
        let opened = |key: &DocumentKey| {
            doc_state
                .open_signed_update(&under(key), Seq::<Relay>::new(1))
                .unwrap()
        };
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
            .map(|pair| {
                pipeline::seal_link(doc, &pair[0], &pair[1], ambre_protocol::LogHead::EMPTY, &sk)
                    .unwrap()
            })
            .collect();
        let current = keys.last().unwrap().clone();
        let snapshot_id = Id::<Snapshot>::mint();
        let blob = snapshot_blob(doc, served(snapshot_id, 3), b"snap", &keys[0], &sk);
        state.handle_sync_event(SyncEvent::OpenDocumentReady {
            document_id: doc,
            doc_key: current.clone(),
            snapshot_payload: Some(blob),
            snapshot_id,
            covered_seq: Seq::<Relay>::new(3),
            held_snapshot: None,
            doc_type: DocType::Trame,
            existence: crate::Existence::OnRelay,
            signing_keys: Vec::new(),
            role: Role::Owner,
            rotation_owed: ambre_core::RotationDebt::Settled,
            links,
            writers: Default::default(),
            departed: None,
        });
        let opened = drain(&rx).into_iter().any(
            |e| matches!(e, LogicEvent::RemoteSnapshot { ref payload, .. } if payload == b"snap"),
        );
        assert!(opened, "the snapshot opens under a key the chain recovers");

        let doc_state = &state.documents[&doc];
        let old = make_update(
            doc,
            snapshot_id,
            Seq::<DeltaAuthor>::new(1),
            b"d",
            &keys[1],
            &sk,
        );
        assert_eq!(
            doc_state
                .open_signed_update(&old, Seq::<Relay>::new(1))
                .unwrap()
                .payload,
            b"d"
        );
        assert_eq!(doc_state.doc_key.id(), current.id());
        assert!(
            doc_state.old_keys.is_empty(),
            "a recovered key is not cached"
        );
    }

    /// What a relay names for `snapshot_id` at the horizon `covered`.
    fn served(snapshot_id: Id<Snapshot>, covered: u64) -> pipeline::ServedSnapshot {
        pipeline::ServedSnapshot {
            snapshot_id,
            covered_seq: Seq::<Relay>::new(covered),
        }
    }

    /// The blob object storage holds for a snapshot of `document_id`, sealing
    /// the identifier and the horizon `sealed` names.
    fn snapshot_blob(
        document_id: Id<Document>,
        sealed: pipeline::ServedSnapshot,
        payload: &[u8],
        doc_key: &DocumentKey,
        signing_key: &SigningKey,
    ) -> Payload<pipeline::FetchedBlob> {
        let public_data = SnapshotPublicData {
            document_id,
            pub_key: FixedBytes::<PublicKey>::from_array(signing_key.verifying_key().to_bytes()),
            key_id: doc_key.id(),
            snapshot_id: sealed.snapshot_id,
            covered_seq: sealed.covered_seq,
            parent_snapshot_proof: None,
            parent_snapshot_update_clocks: Vec::new(),
        };
        let snapshot = pipeline::seal_snapshot(
            &ambre_core::Payload::new(payload.to_vec()),
            public_data,
            doc_key,
            signing_key,
        )
        .expect("seal");
        Payload::<pipeline::FetchedBlob>::new(postcard::to_allocvec(&snapshot).expect("encode"))
    }

    fn drain(rx: &ambre_bus::conduit::Receiver<LogicEvent>) -> Vec<LogicEvent> {
        rx.try_iter().collect()
    }

    #[test]
    fn update_with_known_snapshot_delivered_immediately() {
        let (mut state, rx, doc, key, sk, snap) = setup();
        let update = make_update(doc, snap, Seq::<DeltaAuthor>::new(1), b"hello", &key, &sk);

        state.receive_update(Seq::<Relay>::new(1), update);

        let events = drain(&rx);
        assert_eq!(events.len(), 1);
        match &events[0] {
            LogicEvent::RemoteUpdate { document_id, delta } => {
                assert_eq!(*document_id, doc);
                assert_eq!(delta.payload, b"hello");
            }
            other => panic!("expected RemoteUpdate, got {other:?}"),
        }
        assert_eq!(state.documents[&doc].quarantine_len().get(), 0);
    }

    /// Holds `SYNC-144`: an opened update hands the Logic thread the summary
    /// its body carried, beside the delta. One with none hands none.
    #[test]
    fn an_opened_update_hands_logic_its_summary_beside_the_delta() {
        use ambre_core::activity::{ActivityKind, ActivitySummary};
        let (mut state, rx, doc, key, sk, snap) = setup();
        let summary = ActivitySummary {
            kind: ActivityKind::Reaction,
            mentions: Vec::new(),
            mentions_all: ambre_bus::MentionScope::All,
            anchor: None,
            call: None,
        };
        let public_data = UpdatePublicData {
            document_id: doc,
            pub_key: FixedBytes::<PublicKey>::from_array(sk.verifying_key().to_bytes()),
            key_id: key.id(),
            snapshot_id: snap,
            clock: Seq::<DeltaAuthor>::new(1),
            log_head: ambre_protocol::LogHead::EMPTY,
            log_entry: None,
        };
        let body = pipeline::update_body(&TEST_NONCE, Some(&summary), &b"liked".to_vec().into())
            .expect("bounds");
        let sealed = pipeline::seal_update(&body, public_data, &key, &sk).expect("seal");
        state.receive_update(Seq::<Relay>::new(1), sealed);
        state.receive_update(
            Seq::<Relay>::new(2),
            make_update(doc, snap, Seq::<DeltaAuthor>::new(2), b"plain", &key, &sk),
        );
        let handed: Vec<_> = drain(&rx)
            .into_iter()
            .filter_map(|e| match e {
                LogicEvent::RemoteUpdate { delta, .. } => {
                    Some((delta.payload.into_bytes(), delta.summary))
                }
                _ => None,
            })
            .collect();
        assert_eq!(
            handed,
            vec![
                (b"liked".to_vec(), Some(summary.into())),
                (b"plain".to_vec(), None)
            ]
        );
    }

    /// An opened update reaches the Logic thread with the nonce its body
    /// carried and the PeerID its signer and that nonce derive. It also names
    /// the member the directory holds for its signer (`SYNC-114`, `AUTHOR-013`). A signer the
    /// directory does not name resolves to no member.
    #[test]
    fn an_opened_update_reaches_logic_with_its_nonce_its_peer_and_its_signer() {
        let (mut state, rx, doc, key, sk, snap) = setup();
        let alice = Id::<User>::mint();
        let signer = FixedBytes::<PublicKey>::from_array(sk.verifying_key().to_bytes());
        state
            .documents
            .get_mut(&doc)
            .unwrap()
            .set_signing_keys([(signer, alice)]);
        state.receive_update(
            Seq::<Relay>::new(1),
            make_update(doc, snap, Seq::<DeltaAuthor>::new(1), b"hello", &key, &sk),
        );
        let stranger = MasterSeed::generate().derive_signing_key();
        state.receive_update(
            Seq::<Relay>::new(2),
            make_update(
                doc,
                snap,
                Seq::<DeltaAuthor>::new(1),
                b"hi",
                &key,
                &stranger,
            ),
        );

        let deltas: Vec<SignedDelta> = drain(&rx)
            .into_iter()
            .map(|e| match e {
                LogicEvent::RemoteUpdate { delta, .. } => delta,
                other => panic!("expected RemoteUpdate, got {other:?}"),
            })
            .collect();
        let expected = SignedDelta {
            seq: ambre_bus::Seq::<ambre_bus::Relay>::new(1),
            payload: b"hello".to_vec().into(),
            nonce: TEST_NONCE,
            peer: ambre_crypto::peer::derive_peer_id(&signer, &TEST_NONCE),
            user: Some(alice),
            summary: None,
        };
        assert_eq!(deltas.first(), Some(&expected));
        let unknown = deltas.get(1).unwrap();
        assert_eq!(unknown.user, None);
        assert_eq!(
            unknown.peer,
            ambre_crypto::peer::derive_peer_id(
                &FixedBytes::<PublicKey>::from_array(stranger.verifying_key().to_bytes()),
                &TEST_NONCE
            )
        );
    }

    /// Each open of a document draws a new PeerID (`AUTHOR-002`). A second
    /// open of a document still open keeps the one its operations derive.
    #[test]
    fn each_open_draws_a_new_peer_and_a_repeated_open_keeps_it() {
        let (mut state, rx, doc, _key, sk, _snap) = setup();
        state.session_nonces.clear();
        state.signing_key = Some(sk);
        state.user_id = Some(Id::<User>::mint());
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
            pub_key: FixedBytes::<PublicKey>::from_array(sk.verifying_key().to_bytes()),
            key_id: key.id(),
            snapshot_id: snap,
            clock: Seq::<DeltaAuthor>::new(1),
            log_head: ambre_protocol::LogHead::EMPTY,
            log_entry: None,
        };
        let short = UpdateFrame::from_bytes(vec![6; 16]);
        let short = pipeline::seal_update(&short, public_data, &key, &sk).unwrap();
        state.receive_update(Seq::<Relay>::new(1), short);
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
        let unknown = Id::<Snapshot>::mint();
        let update = make_update(
            doc,
            unknown,
            Seq::<DeltaAuthor>::new(1),
            b"later",
            &key,
            &sk,
        );

        state.receive_update(Seq::<Relay>::new(1), update);

        assert!(drain(&rx).is_empty(), "nothing should be delivered yet");
        assert_eq!(state.documents[&doc].quarantine_len().get(), 1);
    }

    #[test]
    fn quarantine_released_in_clock_order() {
        let (mut state, rx, doc, key, sk, _snap) = setup();
        let parent = Id::<Snapshot>::mint();
        // Arrive out of clock order: clock 5 then clock 2.
        state.receive_update(
            Seq::<Relay>::new(1),
            make_update(doc, parent, Seq::<DeltaAuthor>::new(5), b"five", &key, &sk),
        );
        state.receive_update(
            Seq::<Relay>::new(2),
            make_update(doc, parent, Seq::<DeltaAuthor>::new(2), b"two", &key, &sk),
        );
        assert!(drain(&rx).is_empty());
        assert_eq!(state.documents[&doc].quarantine_len().get(), 2);

        state.snapshot_known(doc, parent);

        let payloads: Vec<Vec<u8>> = drain(&rx)
            .into_iter()
            .map(|e| match e {
                LogicEvent::RemoteUpdate { delta, .. } => delta.payload.into_bytes(),
                other => panic!("expected RemoteUpdate, got {other:?}"),
            })
            .collect();
        assert_eq!(payloads, vec![b"two".to_vec(), b"five".to_vec()]);
        assert_eq!(state.documents[&doc].quarantine_len().get(), 0);
    }

    /// A Murmure document takes the quarantine and the delta order of the
    /// document plane (the Murmure segments spec).
    #[test]
    fn a_murmure_document_takes_the_quarantine_and_the_delta_order() {
        let (mut state, rx, _doc, key, sk, _snap) = setup();
        let doc = Id::<Document>::mint();
        state.documents.insert(
            doc,
            SyncDocumentState::new(
                doc,
                key.clone(),
                DocType::Murmure,
                crate::Existence::OnRelay,
            ),
        );
        let parent = Id::<Snapshot>::mint();
        state.receive_update(
            Seq::<Relay>::new(1),
            make_update(doc, parent, Seq::<DeltaAuthor>::new(5), b"five", &key, &sk),
        );
        state.receive_update(
            Seq::<Relay>::new(2),
            make_update(doc, parent, Seq::<DeltaAuthor>::new(2), b"two", &key, &sk),
        );
        assert!(drain(&rx).is_empty(), "an unknown parent holds both");
        assert_eq!(state.documents[&doc].quarantine_len().get(), 2);

        state.snapshot_known(doc, parent);
        let payloads: Vec<Vec<u8>> = drain(&rx)
            .into_iter()
            .filter_map(|e| match e {
                LogicEvent::RemoteUpdate { delta, .. } => Some(delta.payload.into_bytes()),
                _ => None,
            })
            .collect();
        assert_eq!(payloads, vec![b"two".to_vec(), b"five".to_vec()]);
        assert_eq!(state.documents[&doc].quarantine_len().get(), 0);
    }

    #[test]
    fn quarantine_overflow_drops_oldest_and_warns() {
        let (mut state, rx, doc, key, sk, _snap) = setup();
        let parent = Id::<Snapshot>::mint();
        for clock in 1..=(QUARANTINE_LIMIT as u64) {
            state.receive_update(
                Seq::<Relay>::new(clock),
                make_update(doc, parent, Seq::<DeltaAuthor>::new(clock), b"x", &key, &sk),
            );
        }

        let warnings = drain(&rx)
            .into_iter()
            .filter(|e| matches!(e, LogicEvent::SyncWarning { code, .. } if *code == SyncWarningCode::QuarantineOverflow))
            .count();
        assert_eq!(warnings, 1, "exactly one overflow warning at the hard cap");
        assert_eq!(
            state.documents[&doc].quarantine_len().get() as usize,
            QUARANTINE_LIMIT - 1
        );
    }

    /// `SYNC-007`: the overflow drops the update with the lowest clock, across
    /// every parent it waits on. That update arrives neither first nor last,
    /// so dropping by arrival at either end fails.
    #[test]
    fn a_quarantine_overflow_drops_the_lowest_clock() {
        let (mut state, _rx, doc, key, sk, _snap) = setup();
        let parents = [Id::<Snapshot>::mint(), Id::<Snapshot>::mint()];
        // Clock 2 first, clock 1 second, then 3 up to the cap, alternating
        // between the two parents.
        let clocks = [2, 1].into_iter().chain(3..=(QUARANTINE_LIMIT as u64));
        for (i, clock) in clocks.enumerate() {
            let parent = parents[i % 2];
            state.receive_update(
                Seq::<Relay>::new(i as u64 + 1),
                make_update(doc, parent, Seq::<DeltaAuthor>::new(clock), b"x", &key, &sk),
            );
        }
        let held: std::collections::BTreeSet<u64> = state.documents[&doc]
            .quarantine
            .values()
            .flatten()
            .map(|(_, u)| u.public_data.clock.get())
            .collect();
        assert_eq!(held.len(), QUARANTINE_LIMIT - 1);
        assert!(!held.contains(&1), "the lowest clock left");
        assert!(held.contains(&2) && held.contains(&(QUARANTINE_LIMIT as u64)));
    }

    // -- Snapshot lifecycle (upload adoption + cold-start seq gating) ----------

    /// Drain the WsClientMsg this state sent to its mock WS, decoded.
    fn drain_ws(
        rx: &ambre_bus::conduit::Receiver<Payload<ambre_protocol::WireFrame>>,
    ) -> Vec<WsClientMsg> {
        rx.try_iter()
            .filter_map(|wire| ambre_protocol::ws::decode_client_msg(&wire).ok())
            .collect()
    }

    /// An edit buffered before the session waits, for a relay document, until
    /// the replay brought its clock up: the open sends nothing of it.
    #[test]
    fn an_edit_buffered_before_the_session_waits_for_the_replay() {
        let (mut state, _rx, doc, key, sk, _snap) = setup();
        let (ws_tx, ws_rx) = ambre_bus::conduit::unbounded();
        state.set_mock_ws(ws_tx);
        state.handle_send_update(doc, b"early".to_vec());
        state.signing_key = Some(sk);
        state.handle_sync_event(SyncEvent::OpenDocumentReady {
            document_id: doc,
            doc_key: key,
            snapshot_payload: None,
            snapshot_id: Id::<Snapshot>::mint(),
            covered_seq: Seq::<Relay>::new(0),
            held_snapshot: None,
            doc_type: DocType::Trame,
            existence: crate::Existence::OnRelay,
            signing_keys: Vec::new(),
            role: Role::Owner,
            rotation_owed: ambre_core::RotationDebt::Settled,
            links: Vec::new(),
            writers: Default::default(),
            departed: None,
        });
        assert!(
            !drain_ws(&ws_rx)
                .iter()
                .any(|m| matches!(m, WsClientMsg::Update(_))),
            "the edit left before the replay"
        );
        assert_eq!(state.pending_pre_session_updates.len(), 1);
    }

    /// The answer of an open of `document_id` labelled `doc_type`, which
    /// installs `doc_key`.
    fn ready(document_id: Id<Document>, doc_key: DocumentKey, doc_type: DocType) -> SyncEvent {
        SyncEvent::OpenDocumentReady {
            document_id,
            doc_key,
            snapshot_payload: None,
            snapshot_id: Id::<Snapshot>::genesis(document_id),
            covered_seq: Seq::<Relay>::ZERO,
            held_snapshot: None,
            doc_type,
            existence: crate::Existence::OnRelay,
            signing_keys: Vec::new(),
            role: Role::Owner,
            rotation_owed: ambre_core::RotationDebt::Settled,
            links: Vec::new(),
            writers: Default::default(),
            departed: None,
        }
    }

    /// Holds `SYNC-260`. The relay lists a Toile document as a channel, so
    /// an open labels it Murmure and checks the key against a channel. The
    /// open the Toile view made keeps the document and its key, and the
    /// Murmure open installs nothing, as a command or as an answer. A
    /// document no other open named still opens as Murmure.
    #[test]
    fn a_document_relabelled_murmure_keeps_the_key_its_own_open_installed() {
        let (sink, _rx) = LogicSink::channel();
        let mut state = SyncState::new(
            sink,
            crate::http::Url::<crate::RelayHost>::new(String::new()),
        );
        let toile = Id::<Document>::mint();
        state.handle_open_document(toile, DocType::Toile);
        let checked = state.documents[&toile].doc_key.id();

        let forged = DocumentKey::generate();
        state.handle_open_document(toile, DocType::Murmure);
        state.handle_sync_event(ready(toile, forged.clone(), DocType::Murmure));
        let held = &state.documents[&toile];
        assert_eq!(held.doc_type, DocType::Toile);
        assert_eq!(
            held.doc_key.id(),
            checked,
            "the Murmure label replaced the key"
        );

        let channel = Id::<Document>::mint();
        state.handle_sync_event(ready(channel, forged.clone(), DocType::Murmure));
        assert_eq!(state.documents[&channel].doc_key.id(), forged.id());
    }

    /// Holds `SYNC-260` in the other order. A Murmure open installed a key
    /// first, then the Toile view opens the same document. The key the
    /// Murmure open installed is gone before the Toile open answers, and a
    /// late Murmure answer installs nothing.
    #[test]
    fn an_open_under_another_type_drops_the_key_a_murmure_open_installed() {
        let (sink, _rx) = LogicSink::channel();
        let mut state = SyncState::new(
            sink,
            crate::http::Url::<crate::RelayHost>::new(String::new()),
        );
        let doc = Id::<Document>::mint();
        let forged = DocumentKey::generate();
        state.handle_sync_event(ready(doc, forged.clone(), DocType::Murmure));
        assert_eq!(state.documents[&doc].doc_key.id(), forged.id());

        // The Toile open waits for the session, as it waits for the relay.
        let (event_tx, _events) = ambre_bus::conduit::unbounded();
        state.set_event_tx(event_tx);
        state.handle_open_document(doc, DocType::Toile);
        assert!(
            !state.documents.contains_key(&doc),
            "the unchecked key outlived the open of the Toile view"
        );

        state.handle_sync_event(ready(doc, forged.clone(), DocType::Murmure));
        assert!(!state.documents.contains_key(&doc));
    }

    /// A downloaded snapshot covering up to seq N must make the post-open
    /// `RequestMissing` start AFTER N. Only the deltas the snapshot does not
    /// already contain are replayed, not the whole history since genesis.
    #[test]
    fn open_document_ready_gates_request_missing_on_covered_seq() {
        let (mut state, _rx, doc, key, _sk, _snap) = setup();
        let (ws_tx, ws_rx) = ambre_bus::conduit::unbounded();
        state.set_mock_ws(ws_tx);
        let snapshot_id = Id::<Snapshot>::mint();

        let blob = snapshot_blob(doc, served(snapshot_id, 1500), b"snap", &key, &_sk);
        state.handle_sync_event(SyncEvent::OpenDocumentReady {
            document_id: doc,
            doc_key: key,
            snapshot_payload: Some(blob),
            snapshot_id,
            covered_seq: Seq::<Relay>::new(1500),
            held_snapshot: None,
            doc_type: DocType::Trame,
            existence: crate::Existence::OnRelay,
            signing_keys: Vec::new(),
            role: Role::Owner,
            rotation_owed: ambre_core::RotationDebt::Settled,
            links: Vec::new(),
            writers: Default::default(),
            departed: None,
        });

        // The replay cursor advanced to the snapshot's covered seq.
        assert_eq!(state.documents[&doc].last_seq, Seq::<Relay>::new(1500));
        // The downloaded snapshot id is registered as a known parent.
        assert!(state.documents[&doc].known_snapshots.contains(&snapshot_id));

        let msgs = drain_ws(&ws_rx);
        let after = msgs.iter().find_map(|m| match m {
            WsClientMsg::RequestMissing { after_seq, .. } => Some(*after_seq),
            _ => None,
        });
        assert_eq!(
            after,
            Some(Seq::<Relay>::new(1500)),
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
            snapshot_id: Id::<Snapshot>::genesis(doc),
            covered_seq: Seq::<Relay>::new(0),
            held_snapshot: None,
            doc_type: DocType::Trame,
            existence: crate::Existence::OnRelay,
            signing_keys: Vec::new(),
            role: Role::Owner,
            rotation_owed: ambre_core::RotationDebt::Settled,
            links: Vec::new(),
            writers: Default::default(),
            departed: None,
        });

        assert_eq!(state.documents[&doc].last_seq, Seq::<Relay>::new(0));
        let msgs = drain_ws(&ws_rx);
        let after = msgs.iter().find_map(|m| match m {
            WsClientMsg::RequestMissing { after_seq, .. } => Some(*after_seq),
            _ => None,
        });
        assert_eq!(after, Some(Seq::<Relay>::new(0)));
    }

    /// Count of `DocumentReady` events for `doc` in a drained event list.
    fn ready_count(events: &[LogicEvent], doc: Id<Document>) -> usize {
        events
            .iter()
            .filter(
                |e| matches!(e, LogicEvent::DocumentReady { document_id, .. } if *document_id == doc),
            )
            .count()
    }

    /// A departure is attributed by the member directory, exactly like a cursor
    /// (`PRESENCE-014`). The relay names the key, and the key resolves to the
    /// `Id<User>` the view keys presence by. A key nobody knows is dropped rather
    /// than attributed to a guess -- the peer then ages out of the live set.
    #[test]
    fn peer_left_resolves_the_key_the_relay_named() {
        let (mut state, rx, doc, _key, signing_key, _snap) = setup();
        let pub_key = FixedBytes::<PublicKey>::from_array(signing_key.verifying_key().to_bytes());
        let alice = Id::<User>::mint();
        state
            .documents
            .get_mut(&doc)
            .expect("document")
            .signing_keys
            .insert(pub_key, alice);

        state.handle_ws_message(WsServerMsg::PeerLeft {
            document_id: doc,
            pub_key,
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
        let stranger = FixedBytes::<PublicKey>::from_array([7u8; 32]);
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

    /// A dropped snapshot names its author and its reporter as users. Each
    /// member but the author finalizes the segment again when the relay asks.
    #[test]
    fn a_dropped_snapshot_names_its_author_and_reporter() {
        let (mut state, rx, doc, _key, signing_key, snapshot) = setup();
        let author = FixedBytes::<PublicKey>::from_array(signing_key.verifying_key().to_bytes());
        let reporter = FixedBytes::<PublicKey>::from_array([7u8; 32]);
        let (alice, bob) = (Id::<User>::mint(), Id::<User>::mint());
        let keys = &mut state
            .documents
            .get_mut(&doc)
            .expect("document")
            .signing_keys;
        keys.insert(author, alice);
        keys.insert(reporter, bob);
        let dropped = |state: &mut SyncState| {
            state.handle_ws_message(WsServerMsg::SnapshotDropped {
                document_id: doc,
                snapshot_id: snapshot,
                author,
                reporter,
                after: ambre_core::AfterDrop::Refinalize,
            });
        };
        dropped(&mut state);
        assert!(
            drain(&rx).iter().any(|e| matches!(
                e,
                LogicEvent::SnapshotDropped { document_id, author: Some(a), reporter: Some(r), after: ambre_core::AfterDrop::Refinalize }
                    if *document_id == doc && *a == alice && *r == bob
            )),
            "both keys resolve, and this member finalizes again"
        );
        state.signing_key = Some(signing_key);
        dropped(&mut state);
        assert!(
            drain(&rx).iter().any(|e| matches!(
                e,
                LogicEvent::SnapshotDropped {
                    after: ambre_core::AfterDrop::Keep,
                    ..
                }
            )),
            "the author leaves the segment to another member"
        );
    }

    /// The rows a `MembersChanged` event hands Logic, if one was sent.
    fn rows_handed(events: &[LogicEvent]) -> Option<Vec<Id<User>>> {
        events.iter().find_map(|e| match e {
            LogicEvent::MembersChanged { members, .. } => {
                Some(members.iter().map(|m| m.user_id).collect())
            }
            _ => None,
        })
    }

    fn listed(user_id: Id<User>) -> MemberInfo {
        MemberInfo {
            user_id,
            display_name: ambre_core::Validated::<ambre_core::DisplayName>::parse("Carol").unwrap(),
            email: "carol@example.org".into(),
            role: Role::Editor,
            standing: ambre_bus::MemberStanding::Verified,
        }
    }

    /// Holds `SYNC-151`: the relay's word that the members changed costs one
    /// listing. Its answer reaches Logic with the rows, so a view asks for
    /// nothing more. A crossed older answer hands nothing on, and a document
    /// not open here lists nothing.
    #[test]
    fn a_change_of_members_costs_one_listing_whose_rows_reach_logic() {
        let (mut state, rx, doc, _key, _signing_key, _snap) = setup();
        state.handle_ws_message(WsServerMsg::MembersChanged { document_id: doc });
        assert_eq!(
            state.documents[&doc].directory_asked.latest(),
            ambre_core::Seq::nth(1),
            "one listing"
        );
        assert_eq!(rows_handed(&drain(&rx)), None, "nothing before the answer");

        let carol = Id::<User>::mint();
        state.handle_sync_event(SyncEvent::DocumentMembersReady {
            id: None,
            asked: ambre_core::Seq::nth(1),
            document_id: doc,
            word: Default::default(),
            rotation_owed: ambre_core::RotationDebt::Settled,
            members: vec![listed(carol)],
            signing_keys: vec![(FixedBytes::<PublicKey>::from_array([2u8; 32]), carol)],
            heads: Default::default(),
            writers: Default::default(),
            departed: None,
            revoked: None,
            root: Default::default(),
        });
        let events = drain(&rx);
        assert_eq!(
            rows_handed(&events),
            Some(vec![carol]),
            "the rows reach Logic"
        );
        assert!(!events.iter().any(|e| matches!(e, LogicEvent::Rest { .. })));

        state.handle_sync_event(SyncEvent::DocumentMembersReady {
            id: None,
            asked: ambre_core::Seq::nth(1),
            document_id: doc,
            word: Default::default(),
            rotation_owed: ambre_core::RotationDebt::Settled,
            members: Vec::new(),
            signing_keys: Vec::new(),
            heads: Default::default(),
            writers: Default::default(),
            departed: None,
            revoked: None,
            root: Default::default(),
        });
        assert_eq!(
            rows_handed(&drain(&rx)),
            None,
            "a crossed answer hands nothing"
        );

        let elsewhere = Id::<Document>::mint();
        state.handle_ws_message(WsServerMsg::MembersChanged {
            document_id: elsewhere,
        });
        assert_eq!(
            state.documents[&doc].directory_asked.latest(),
            ambre_core::Seq::nth(1)
        );
        assert_eq!(rows_handed(&drain(&rx)), None, "a document not open here");
    }

    /// Closing a document ends its session here. Its state goes, with the
    /// document key and every older key, so nothing of it is read again.
    #[test]
    fn closing_a_document_forgets_its_keys() {
        let (mut state, _rx, doc, _key, _signing_key, _snap) = setup();
        state.handle_close_document(doc);
        assert!(!state.documents.contains_key(&doc));
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
            state.documents[&doc].directory_asked.latest(),
            ambre_core::Seq::nth(2),
            "each word lists again"
        );
        drain(&rx);

        let carol = Id::<User>::mint();
        state.handle_sync_event(SyncEvent::DocumentMembersReady {
            id: None,
            asked: ambre_core::Seq::nth(2),
            document_id: doc,
            word: Default::default(),
            rotation_owed: ambre_core::RotationDebt::Settled,
            members: Vec::new(),
            signing_keys: vec![(FixedBytes::<PublicKey>::from_array([2u8; 32]), carol)],
            heads: Default::default(),
            writers: Default::default(),
            departed: None,
            revoked: None,
            root: Default::default(),
        });
        let bob = Id::<User>::mint();
        state.handle_sync_event(SyncEvent::DocumentMembersReady {
            id: None,
            asked: ambre_core::Seq::nth(1),
            document_id: doc,
            word: Default::default(),
            rotation_owed: ambre_core::RotationDebt::Settled,
            members: Vec::new(),
            signing_keys: vec![(FixedBytes::<PublicKey>::from_array([1u8; 32]), bob)],
            heads: Default::default(),
            writers: Default::default(),
            departed: None,
            revoked: None,
            root: Default::default(),
        });

        let keys = &state.documents[&doc].signing_keys;
        assert_eq!(
            keys.get(&FixedBytes::<PublicKey>::from_array([2u8; 32])),
            Some(&carol),
            "the newer answer holds"
        );
        assert!(
            !keys.contains_key(&FixedBytes::<PublicKey>::from_array([1u8; 32])),
            "the crossed answer changes nothing"
        );
        assert!(
            !drain(&rx)
                .iter()
                .any(|e| matches!(e, LogicEvent::Rest { .. })),
            "a refresh of the directory alone answers no request"
        );
    }

    /// Holds `SYNC-150`: the relay's acknowledgement of a subscription lists
    /// the members again. A newcomer committed after the open read its members,
    /// and before the relay sent this client any word, resolves. A document not
    /// open here, or not yet on the relay, lists nothing.
    #[test]
    fn a_subscription_reads_the_members_a_change_during_the_open_left_out() {
        let (mut state, rx, doc, _key, _signing_key, _snap) = setup();
        let carol = Id::<User>::mint();
        assert!(
            !state.documents[&doc]
                .signing_keys
                .contains_key(&FixedBytes::<PublicKey>::from_array([3u8; 32]))
        );

        state.handle_ws_message(WsServerMsg::Subscribed { document_id: doc });
        assert_eq!(
            state.documents[&doc].directory_asked.latest(),
            ambre_core::Seq::nth(1),
            "one listing"
        );
        drain(&rx);
        state.handle_sync_event(SyncEvent::DocumentMembersReady {
            id: None,
            asked: ambre_core::Seq::nth(1),
            document_id: doc,
            word: Default::default(),
            rotation_owed: ambre_core::RotationDebt::Settled,
            members: vec![listed(carol)],
            signing_keys: vec![(FixedBytes::<PublicKey>::from_array([3u8; 32]), carol)],
            heads: Default::default(),
            writers: Default::default(),
            departed: None,
            revoked: None,
            root: Default::default(),
        });
        let keys = &state.documents[&doc].signing_keys;
        assert_eq!(
            keys.get(&FixedBytes::<PublicKey>::from_array([3u8; 32])),
            Some(&carol),
            "the newcomer resolves"
        );
        let handed = rows_handed(&drain(&rx));
        assert_eq!(handed, Some(vec![carol]), "an open view gets the rows too");

        let local = Id::<Document>::mint();
        state.documents.insert(
            local,
            SyncDocumentState::new(
                local,
                DocumentKey::generate(),
                DocType::Trame,
                crate::Existence::LocalOnly,
            ),
        );
        for document_id in [local, Id::<Document>::mint()] {
            state.handle_ws_message(WsServerMsg::Subscribed { document_id });
        }
        assert_eq!(
            state.documents[&local].directory_asked.latest(),
            ambre_core::Seq::nth(0)
        );
        assert!(
            !drain(&rx)
                .iter()
                .any(|e| matches!(e, LogicEvent::MembersChanged { .. })),
            "nothing to list for a document the relay does not serve here"
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
        let snapshot_id = Id::<Snapshot>::mint();
        let blob = snapshot_blob(doc, served(snapshot_id, 0), b"snap", &key, &sk);

        state.handle_sync_event(SyncEvent::OpenDocumentReady {
            document_id: doc,
            doc_key: key,
            snapshot_payload: Some(blob),
            snapshot_id,
            covered_seq: Seq::<Relay>::new(0),
            held_snapshot: None,
            doc_type: DocType::Trame,
            existence: crate::Existence::OnRelay,
            signing_keys: Vec::new(),
            role: Role::Owner,
            rotation_owed: ambre_core::RotationDebt::Settled,
            links: Vec::new(),
            writers: Default::default(),
            departed: None,
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
        let reference = Id::<Snapshot>::mint();
        let held = Id::<Snapshot>::mint();
        state.handle_sync_event(SyncEvent::OpenDocumentReady {
            document_id: doc,
            doc_key: key.clone(),
            snapshot_payload: Some(snapshot_blob(
                doc,
                served(reference, 3),
                b"reference",
                &key,
                &sk,
            )),
            snapshot_id: reference,
            covered_seq: Seq::<Relay>::new(3),
            held_snapshot: Some(HeldSnapshot {
                snapshot_id: held,
                covered_seq: Seq::<Relay>::new(9),
                blob: snapshot_blob(doc, served(held, 9), b"latest", &key, &sk),
            }),
            doc_type: DocType::Trame,
            existence: crate::Existence::OnRelay,
            signing_keys: Vec::new(),
            role: Role::Owner,
            rotation_owed: ambre_core::RotationDebt::Settled,
            links: Vec::new(),
            writers: Default::default(),
            departed: None,
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
                LogicEvent::PendingSnapshot { payload, covered_seq, purpose: SnapshotPurpose::Open, .. }
                    if payload == b"latest" && covered_seq.get() == 9
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

    /// `SYNC-257`, audit P6. The relay holds two snapshots of one document,
    /// each signed by a writer. It serves one in place of the other. The open, the held snapshot and an announced snapshot each
    /// refuse it with a warning. Nothing of it reaches the Logic thread, and
    /// the identifier asked for stays unknown.
    #[test]
    fn a_snapshot_whose_sealed_identifier_differs_is_refused() {
        let (mut state, rx, doc, key, sk, _snap) = setup();
        let (ws_tx, _ws_rx) = ambre_bus::conduit::unbounded();
        state.set_mock_ws(ws_tx);
        let (asked, other) = (Id::<Snapshot>::mint(), Id::<Snapshot>::mint());
        let swapped = |covered| snapshot_blob(doc, served(other, covered), b"other", &key, &sk);
        let applied = |events: &[LogicEvent]| {
            events.iter().any(|e| {
                matches!(
                    e,
                    LogicEvent::RemoteSnapshot { .. } | LogicEvent::PendingSnapshot { .. }
                )
            })
        };
        let refused = |events: &[LogicEvent]| {
            events.iter().any(|e| {
                matches!(
                    e,
                    LogicEvent::SyncWarning {
                        code: SyncWarningCode::DecryptFailed,
                        ..
                    }
                )
            })
        };

        // The open asks for `asked` and the relay serves `other`, as the
        // start and as the held snapshot.
        state.handle_sync_event(SyncEvent::OpenDocumentReady {
            document_id: doc,
            doc_key: key.clone(),
            snapshot_payload: Some(swapped(3)),
            snapshot_id: asked,
            covered_seq: Seq::<Relay>::new(3),
            held_snapshot: Some(HeldSnapshot {
                snapshot_id: asked,
                covered_seq: Seq::<Relay>::new(9),
                blob: swapped(9),
            }),
            doc_type: DocType::Trame,
            existence: crate::Existence::OnRelay,
            signing_keys: Vec::new(),
            role: Role::Owner,
            rotation_owed: ambre_core::RotationDebt::Settled,
            links: Vec::new(),
            writers: Default::default(),
            departed: None,
        });
        let at_open = drain(&rx);
        assert!(
            !applied(&at_open),
            "a swapped start was applied: {at_open:?}"
        );
        assert!(
            refused(&at_open),
            "a swapped start raised no warning: {at_open:?}"
        );
        assert_eq!(state.documents[&doc].last_seq, Seq::<Relay>::ZERO);
        assert_ne!(state.documents[&doc].current_snapshot_id, asked);

        assert!(!state.check_held_snapshot(doc));
        let held = drain(&rx);
        assert!(
            !applied(&held),
            "a swapped held snapshot was checked: {held:?}"
        );
        assert!(!state.documents[&doc].checks.contains_key(&asked));

        // An announced snapshot, fetched by its id, comes back as the other.
        let announced = Id::<Snapshot>::mint();
        state.handle_sync_event(SyncEvent::SnapshotFetched {
            document_id: doc,
            snapshot_id: announced,
            covered_seq: Seq::<Relay>::new(12),
            blob: Some(swapped(12)),
        });
        let fetched = drain(&rx);
        assert!(
            !applied(&fetched),
            "a swapped fetch was applied: {fetched:?}"
        );
        assert!(
            refused(&fetched),
            "a swapped fetch raised no warning: {fetched:?}"
        );
        assert!(!state.documents[&doc].known_snapshots.contains(&announced));
    }

    /// `SYNC-259`. Each snapshot seals the horizon 7. A relay serves it at
    /// a horizon short of that, then past it, at the open, as the held
    /// snapshot and on a fetch. Each raises a warning and hands the Logic
    /// thread nothing. The replay cursor stays at zero, so no delta is
    /// skipped on the relay's word. Served at its own horizon, the open
    /// adopts it and the replay starts past 7.
    #[test]
    fn a_snapshot_whose_sealed_horizon_differs_is_refused() {
        const SEALED: u64 = 7;
        let applied = |events: &[LogicEvent]| {
            events.iter().any(|e| {
                matches!(
                    e,
                    LogicEvent::RemoteSnapshot { .. } | LogicEvent::PendingSnapshot { .. }
                )
            })
        };
        let refused = |events: &[LogicEvent]| {
            events.iter().any(|e| {
                matches!(
                    e,
                    LogicEvent::SyncWarning {
                        code: SyncWarningCode::DecryptFailed,
                        ..
                    }
                )
            })
        };
        for covered in [SEALED - 1, SEALED + 1, SEALED] {
            let (mut state, rx, doc, key, sk, _snap) = setup();
            let (ws_tx, _ws_rx) = ambre_bus::conduit::unbounded();
            state.set_mock_ws(ws_tx);
            let (start, held, announced) = (
                Id::<Snapshot>::mint(),
                Id::<Snapshot>::mint(),
                Id::<Snapshot>::mint(),
            );
            let sealed = |id| snapshot_blob(doc, served(id, SEALED), b"base", &key, &sk);
            let honest = covered == SEALED;

            state.handle_sync_event(SyncEvent::OpenDocumentReady {
                document_id: doc,
                doc_key: key.clone(),
                snapshot_payload: Some(sealed(start)),
                snapshot_id: start,
                covered_seq: Seq::<Relay>::new(covered),
                held_snapshot: Some(HeldSnapshot {
                    snapshot_id: held,
                    covered_seq: Seq::<Relay>::new(covered),
                    blob: sealed(held),
                }),
                doc_type: DocType::Trame,
                existence: crate::Existence::OnRelay,
                signing_keys: Vec::new(),
                role: Role::Owner,
                rotation_owed: ambre_core::RotationDebt::Settled,
                links: Vec::new(),
                writers: Default::default(),
                departed: None,
            });
            let at_open = drain(&rx);
            assert_eq!(applied(&at_open), honest, "{covered}: {at_open:?}");
            assert_eq!(refused(&at_open), !honest, "{covered}: {at_open:?}");
            let cursor = if honest { SEALED } else { 0 };
            assert_eq!(
                state.documents[&doc].last_seq,
                Seq::<Relay>::new(cursor),
                "{covered}: the replay starts past the sealed horizon alone"
            );

            assert_eq!(state.check_held_snapshot(doc), honest, "{covered}");
            let at_check = drain(&rx);
            assert_eq!(applied(&at_check), honest, "{covered}: {at_check:?}");
            assert_eq!(
                state.documents[&doc].checks.contains_key(&held),
                honest,
                "{covered}"
            );

            state.handle_sync_event(SyncEvent::SnapshotFetched {
                document_id: doc,
                snapshot_id: announced,
                covered_seq: Seq::<Relay>::new(covered),
                blob: Some(sealed(announced)),
            });
            let fetched = drain(&rx);
            assert_eq!(applied(&fetched), honest, "{covered}: {fetched:?}");
            assert_eq!(refused(&fetched), !honest, "{covered}: {fetched:?}");
            assert_eq!(
                state.documents[&doc].known_snapshots.contains(&announced),
                honest,
                "{covered}"
            );
        }
    }

    /// A local-only (not yet server-created) or offline open has no replay to
    /// wait for, so `DocumentReady` fires immediately at open. Otherwise the
    /// overlay would hang forever with no `MissingUpdates` ever coming.
    #[test]
    fn open_document_ready_immediate_without_replay() {
        // Offline open (no WS): on the relay but not connected.
        let (mut state, rx, doc, key, _sk, _snap) = setup();
        state.handle_sync_event(SyncEvent::OpenDocumentReady {
            document_id: doc,
            doc_key: key,
            snapshot_payload: None,
            snapshot_id: Id::<Snapshot>::genesis(doc),
            covered_seq: Seq::<Relay>::new(0),
            held_snapshot: None,
            doc_type: DocType::Trame,
            existence: crate::Existence::OnRelay,
            signing_keys: Vec::new(),
            role: Role::Owner,
            rotation_owed: ambre_core::RotationDebt::Settled,
            links: Vec::new(),
            writers: Default::default(),
            departed: None,
        });
        assert_eq!(
            ready_count(&drain(&rx), doc),
            1,
            "offline open signals readiness immediately (no replay to await)"
        );
    }

    /// Extract the role carried by the single `DocumentReady` for `doc`.
    fn ready_role(events: &[LogicEvent], doc: Id<Document>) -> Option<Role> {
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
            snapshot_id: Id::<Snapshot>::genesis(doc),
            covered_seq: Seq::<Relay>::new(0),
            held_snapshot: None,
            doc_type: DocType::Trame,
            existence: crate::Existence::OnRelay,
            signing_keys: Vec::new(),
            role: Role::Viewer,
            rotation_owed: ambre_core::RotationDebt::Settled,
            links: Vec::new(),
            writers: Default::default(),
            departed: None,
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
            snapshot_id: Id::<Snapshot>::genesis(doc),
            covered_seq: Seq::<Relay>::new(0),
            held_snapshot: None,
            doc_type: DocType::Trame,
            existence: crate::Existence::OnRelay,
            signing_keys: Vec::new(),
            role: Role::Editor,
            rotation_owed: ambre_core::RotationDebt::Settled,
            links: Vec::new(),
            writers: Default::default(),
            departed: None,
        });
        assert_eq!(ready_role(&drain(&rx), doc), Some(Role::Editor));
    }

    /// A completed snapshot upload adopts the server-assigned snapshot id as the
    /// document's current parent and registers it as known. Later deltas
    /// parented on it are then delivered, not quarantined.
    #[test]
    fn snapshot_uploaded_adopts_snapshot_id() {
        let (mut state, _rx, doc, _key, _sk, _snap) = setup();
        let new_id = Id::<Snapshot>::mint();

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

        state.handle_send_snapshot(
            doc,
            b"crdt-snapshot-state".to_vec().into(),
            Seq::<Relay>::new(0),
        );

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
        let announced = Id::<Snapshot>::mint();
        state.receive_update(
            Seq::<Relay>::new(1),
            make_update(
                doc,
                announced,
                Seq::<DeltaAuthor>::new(1),
                b"after",
                &key,
                &sk,
            ),
        );
        state.handle_ws_message(WsServerMsg::SnapshotAvailable {
            document_id: doc,
            snapshot_id: announced,
            covered_seq: Seq::<Relay>::new(0),
        });
        assert!(
            drain(&rx).is_empty(),
            "nothing delivered on the announcement"
        );
        assert!(!state.documents[&doc].known_snapshots.contains(&announced));

        let blob = snapshot_blob(doc, served(announced, 0), b"base", &key, &sk);
        state.handle_sync_event(SyncEvent::SnapshotFetched {
            document_id: doc,
            snapshot_id: announced,
            covered_seq: Seq::<Relay>::new(0),
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
        let nonce = FixedBytes::<ambre_core::SessionNonce>::from_array([7; 16]);
        let mallory = MasterSeed::generate().derive_signing_key();
        for (signer, verdict_is_match, adopted) in [
            (true, true, true),
            (false, true, false),
            (true, false, false),
        ] {
            let (mut state, rx, doc, key, sk, reference) = setup();
            let held = Id::<Snapshot>::mint();
            let blob = snapshot_blob(doc, served(held, 7), b"base", &key, &sk);
            state.documents.get_mut(&doc).expect("open").held_snapshot = Some(HeldSnapshot {
                snapshot_id: held,
                covered_seq: Seq::<Relay>::new(7),
                blob,
            });
            state.check_held_snapshot(doc);
            assert!(matches!(
                &drain(&rx)[..],
                [LogicEvent::PendingSnapshot { payload, covered_seq, purpose: SnapshotPurpose::Open, .. }]
                    if payload == b"base" && covered_seq.get() == 7
            ));

            let author = if signer { &sk } else { &mallory };
            let (peer, entry) =
                ambre_crypto::peer::session_identity(author, Id::<User>::mint(), doc, nonce);
            let verdict = if verdict_is_match {
                SnapshotVerdict::Match {
                    extras: vec![(
                        peer,
                        ambre_core::Payload::<ambre_core::PeerEntryBytes>::from(&entry),
                    )],
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
                        LogicEvent::RemoteSnapshot { payload, covered_seq, .. },
                        LogicEvent::DocumentReady { .. },
                    ] if payload == b"base" && covered_seq.get() == 7
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
            let pending = Id::<Snapshot>::mint();
            let blob = snapshot_blob(doc, served(pending, 3), b"base", &key, &author);
            state.adopt_fetched_snapshot(doc, pending, Seq::<Relay>::new(3), Some(blob));
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
            let answer = match vouch {
                Vouch::Confirm => (200, serde_json::json!({ "folded": null })),
                Vouch::Report(_) => (204, serde_json::Value::Null),
            };
            let transport = Scripted::new([Ok(answer)]);
            let auth = session("access-1", "refresh-1");
            let folded = send_vouch(&transport, &auth, &root(), doc, pending, &vouch)
                .await
                .expect("posted");
            assert_eq!(folded, None, "no fold settles here");
            let url = format!("{base}/api/documents/{doc}/snapshots/{pending}/{route}");
            assert_eq!(transport.calls(), [(http::Method::Post, url)]);
            if let Vouch::Report(signature) = vouch {
                let transcript = ambre_protocol::snapshot_report::report_transcript(
                    doc,
                    pending,
                    &FixedBytes::<PublicKey>::from_array(author.verifying_key().to_bytes()),
                );
                let bytes: [u8; 64] = *signature.as_array();
                let signature = ed25519_dalek::Signature::from_bytes(&bytes);
                assert!(
                    reporter
                        .verifying_key()
                        .verify_strict(transcript.as_bytes(), &signature)
                        .is_ok()
                );
                let sent = transport.sent.borrow();
                let body: serde_json::Value =
                    serde_json::from_slice(sent[0].body.as_ref().unwrap().1.as_bytes()).unwrap();
                assert_eq!(body, serde_json::json!({ "signature": hex::encode(bytes) }));
            }
        }
    }

    /// `AUTHOR-021`: a snapshot its author uploads is confirmed by that
    /// author, under the id the relay gave it, once the relay holds its
    /// metadata. A step that fails confirms nothing. `SYNC-182`: the metadata
    /// names no id of the client's own.
    #[tokio::test]
    async fn an_uploaded_snapshot_is_confirmed_by_its_author_once_stored() {
        let base = "https://relay.test";
        let (_state, _rx, doc, key, sk, reference) = setup();
        let upload = || {
            let public_data = SnapshotPublicData {
                document_id: doc,
                pub_key: FixedBytes::<PublicKey>::from_array(sk.verifying_key().to_bytes()),
                key_id: key.id(),
                snapshot_id: reference,
                covered_seq: Seq::<Relay>::new(3),
                parent_snapshot_proof: None,
                parent_snapshot_update_clocks: Vec::new(),
            };
            SnapshotUpload {
                document_id: doc,
                author_pubkey: FixedBytes::from_array(sk.verifying_key().to_bytes()),
                draft: pipeline::SnapshotDraft::new(
                    ambre_core::Payload::new(b"base".to_vec()),
                    public_data,
                    key.clone(),
                    sk.clone(),
                )
                .unwrap(),
            }
        };
        let given = Id::<Snapshot>::mint();
        let link = serde_json::json!({
            "upload_url": "https://store.test/o?sig=x",
            "s3_key": "k",
            "snapshot_id": given.to_string(),
        });
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
                Ok((200, serde_json::json!({ "folded": null }))),
            ]);
            let auth = session("access-1", "refresh-1");

            let uploaded = upload_snapshot(&transport, &auth, &root(), upload()).await;

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
            // `SYNC-258`: the object seals the id the relay minted for its
            // link, at the size the link was asked for.
            let size: serde_json::Value =
                serde_json::from_slice(sent[0].body.as_ref().unwrap().1.as_bytes()).unwrap();
            let object = sent[1].body.as_ref().unwrap().1.as_bytes();
            assert_eq!(size["size_bytes"], object.len());
            let stored = Payload::<pipeline::FetchedBlob>::new(object.to_vec());
            // `SYNC-259`: it seals the horizon the metadata names.
            assert!(pipeline::decode_snapshot_blob(&stored, doc, served(given, 3)).is_ok());
            // `SYNC-182`: the metadata names no snapshot id and no clocks.
            let metadata: serde_json::Value =
                serde_json::from_slice(sent[2].body.as_ref().unwrap().1.as_bytes()).unwrap();
            let fields: Vec<&str> = metadata
                .as_object()
                .unwrap()
                .keys()
                .map(String::as_str)
                .collect();
            assert_eq!(fields, ["author_pubkey", "covered_seq", "s3_key"]);
        }
    }

    /// A snapshot that does not open, or never arrived, stays unknown and is
    /// reported (`SYNC-030`); a blob sealed for another document is refused.
    #[test]
    fn a_snapshot_that_does_not_open_stays_unknown() {
        let (mut state, rx, doc, key, sk, _snap) = setup();
        let announced = Id::<Snapshot>::mint();
        let foreign = snapshot_blob(
            Id::<Document>::mint(),
            served(announced, 0),
            b"x",
            &key,
            &sk,
        );
        for blob in [
            None,
            Some(Payload::<pipeline::FetchedBlob>::new(b"garbage".to_vec())),
            Some(foreign),
        ] {
            state.handle_sync_event(SyncEvent::SnapshotFetched {
                document_id: doc,
                snapshot_id: announced,
                covered_seq: Seq::<Relay>::new(0),
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

    /// `SYNC-031`: a gap that stays past the threshold once one snapshot
    /// arrives asks no second time.
    #[test]
    fn a_gap_still_past_the_threshold_asks_once() {
        let (mut state, _rx, doc, key, sk, _snap) = setup();
        let (ws_tx, ws_rx) = ambre_bus::conduit::unbounded();
        state.set_mock_ws(ws_tx);
        let (first, second) = (Id::<Snapshot>::mint(), Id::<Snapshot>::mint());
        let mut seq = 0;
        let mut arrive = |state: &mut SyncState, parent, clock| {
            seq += 1;
            let update = make_update(doc, parent, Seq::<DeltaAuthor>::new(clock), b"x", &key, &sk);
            state.receive_update(Seq::<Relay>::new(seq), update);
        };
        arrive(&mut state, first, 1);
        for clock in 2..=(QUARANTINE_ALERT as u64 + 1) {
            arrive(&mut state, second, clock);
        }
        state.snapshot_known(doc, first);
        assert_eq!(
            state.documents[&doc].quarantine_len().get() as usize,
            QUARANTINE_ALERT
        );
        assert!(
            state.documents[&doc].gap_requested,
            "a quarantine still at the threshold keeps its ask"
        );
        arrive(&mut state, second, QUARANTINE_ALERT as u64 + 2);
        let asks = drain_ws(&ws_rx)
            .iter()
            .filter(|m| matches!(m, WsClientMsg::RequestMissing { .. }))
            .count();
        assert_eq!(asks, 1, "the gap was asked for again");
    }

    /// `SYNC-128`: a payment that cannot start leaves the debt owed, so the
    /// next edit pays it.
    #[test]
    fn a_payment_that_cannot_start_leaves_the_debt_owed() {
        let (mut state, _rx, doc, _key, _sk, _snap) = setup();
        let rekey = &mut state.documents.get_mut(&doc).expect("open").rekey;
        rekey.found_debt();
        assert!(rekey.pay_now());
        state.start_settle(doc);
        let rekey = &mut state.documents.get_mut(&doc).expect("open").rekey;
        assert!(
            rekey.pay_now(),
            "the debt waits on a payment that never left"
        );
    }

    /// `SYNC-033`, `SYNC-086`: a reconnect subscribes the document again and
    /// asks for what came after the last number it holds.
    #[test]
    fn a_reconnect_subscribes_again_and_asks_after_the_last_number() {
        let (mut state, _rx, doc, _key, _sk, _snap) = setup();
        let (ws_tx, ws_rx) = ambre_bus::conduit::unbounded();
        state.set_mock_ws(ws_tx);
        state.documents.get_mut(&doc).expect("open").last_seq = Seq::<Relay>::new(7);
        state.handle_reconnect(doc);
        let sent = drain_ws(&ws_rx);
        assert!(
            matches!(
                sent.as_slice(),
                [
                    WsClientMsg::Subscribe { document_id: a },
                    WsClientMsg::RequestMissing { document_id: b, after_seq },
                ] if *a == doc && *b == doc && after_seq.get() == 7
            ),
            "{sent:?}"
        );
    }

    /// The cause a held document warns with names the refusal, or the
    /// failures counted.
    #[test]
    fn a_hold_cause_prints_what_the_warning_reads() {
        let refused = HoldCause::Refused(ambre_core::RefusalCode::LogMoved);
        assert_eq!(
            refused.to_string(),
            ambre_core::RefusalCode::LogMoved.to_string()
        );
        assert!(!refused.to_string().is_empty());
        let failed = HoldCause::Failed(ambre_core::Count::<ambre_core::Failures>::new(5));
        assert_eq!(failed.to_string(), "sequencing failed 5 times");
    }

    /// The quarantine asks for the missing history once per gap, however many
    /// arrivals follow past the threshold (`SYNC-031`).
    #[test]
    fn the_quarantine_asks_once_per_gap() {
        let (mut state, _rx, doc, key, sk, _snap) = setup();
        let (ws_tx, ws_rx) = ambre_bus::conduit::unbounded();
        state.set_mock_ws(ws_tx);
        let parent = Id::<Snapshot>::mint();
        let asks = |rx: &ambre_bus::conduit::Receiver<Payload<ambre_protocol::WireFrame>>| {
            drain_ws(rx)
                .iter()
                .filter(|m| matches!(m, WsClientMsg::RequestMissing { .. }))
                .count()
        };
        for clock in 1..=(QUARANTINE_ALERT as u64 + 50) {
            if clock == QUARANTINE_ALERT as u64 {
                assert_eq!(asks(&ws_rx), 0, "nothing asked below the threshold");
            }
            state.receive_update(
                Seq::<Relay>::new(clock),
                make_update(doc, parent, Seq::<DeltaAuthor>::new(clock), b"x", &key, &sk),
            );
        }
        assert_eq!(asks(&ws_rx), 1, "one request for the gap");
        // The gap closes; the next one asks again.
        state.snapshot_known(doc, parent);
        let other = Id::<Snapshot>::mint();
        // The relay numbers every delta once, so the next gap takes new numbers.
        let first = QUARANTINE_ALERT as u64 + 50;
        state.pass_replay_window();
        for clock in 1..=(QUARANTINE_ALERT as u64) {
            let update = make_update(doc, other, Seq::<DeltaAuthor>::new(clock), b"x", &key, &sk);
            state.receive_update(Seq::<Relay>::new(first + clock), update);
        }
        assert_eq!(asks(&ws_rx), 1, "one request for the next gap");
    }

    /// Each update sent, as its clock, the key it names and its plaintext.
    fn sent_updates(
        rx: &ambre_bus::conduit::Receiver<Payload<ambre_protocol::WireFrame>>,
        keys: &[&DocumentKey],
    ) -> Vec<(u64, FixedBytes<KeyId>, Vec<u8>)> {
        opened_updates(drain_ws(rx), keys)
    }

    /// Each update among `msgs`, as its clock, the key it names and its
    /// plaintext.
    fn opened_updates(
        msgs: Vec<WsClientMsg>,
        keys: &[&DocumentKey],
    ) -> Vec<(u64, FixedBytes<KeyId>, Vec<u8>)> {
        msgs.into_iter()
            .filter_map(|msg| match msg {
                WsClientMsg::Update(u) => {
                    let key = keys.iter().find(|k| k.id() == u.public_data.key_id)?;
                    let body = pipeline::open_update(&u, key).expect("opens");
                    let delta = pipeline::split_update_body(body).expect("splits").delta;
                    Some((
                        u.public_data.clock.get(),
                        u.public_data.key_id,
                        delta.into_bytes(),
                    ))
                }
                _ => None,
            })
            .collect()
    }

    /// The signature of each update among `msgs`, in order.
    fn signatures(msgs: &[WsClientMsg]) -> Vec<FixedBytes<Signature>> {
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
            clock: Seq::<DeltaAuthor>::new(1),
            expected: Seq::<DeltaAuthor>::new(2),
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
            clock: Seq::<DeltaAuthor>::new(2),
            expected: Seq::<DeltaAuthor>::new(2),
            signature: first[1],
        });
        let late = drain_ws(&ws_rx);
        assert!(
            matches!(late.as_slice(), [WsClientMsg::ClockResync { signature, .. }] if *signature == first[1]),
            "a late refusal sends its answer alone: {late:?}"
        );
    }

    /// The relay refuses the delta signed `signature`, naming it.
    fn refuse_by_name(state: &mut SyncState, doc: Id<Document>, signature: FixedBytes<Signature>) {
        state.handle_ws_message(WsServerMsg::DeltaRefused {
            document_id: doc,
            signature,
            code: ambre_core::RefusalCode::PayloadTooLarge,
        });
    }

    /// `SYNC-165`: a delta refused for a locked segment returns to the Logic
    /// thread with the delta sent behind it, once, in order. The document is
    /// not held: its next delta leaves.
    #[test]
    fn a_delta_refused_for_the_lock_returns_to_logic_with_every_later_one() {
        let (mut state, rx, doc, _key, sk, _snap) = setup();
        state.signing_key = Some(sk);
        let (ws_tx, ws_rx) = ambre_bus::conduit::unbounded();
        state.set_mock_ws(ws_tx);
        state.handle_send_update(doc, b"a".to_vec());
        state.handle_send_update(doc, b"b".to_vec());
        let sent = signatures(&drain_ws(&ws_rx));
        let _ = drain(&rx);

        for signature in [sent[0], sent[1]] {
            state.handle_ws_message(WsServerMsg::DeltaRefused {
                document_id: doc,
                signature,
                code: ambre_core::RefusalCode::SegmentLocked,
            });
        }
        let told = drain(&rx);
        assert!(
            matches!(
                told.as_slice(),
                [LogicEvent::DeltasLocked { document_id, deltas }]
                    if *document_id == doc && *deltas == [b"a".to_vec(), b"b".to_vec()]
            ),
            "the refusal was told as {told:?}"
        );
        let rekey = &state.documents[&doc].rekey;
        assert_eq!(rekey.unconfirmed(), 0);
        assert!(!rekey.holds_refused(), "a locked segment held its document");
        state.handle_send_update(doc, b"c".to_vec());
        assert_eq!(
            signatures(&drain_ws(&ws_rx)).len(),
            1,
            "the next delta left"
        );
    }

    /// `SYNC-155`: a delta refused by name leaves the unconfirmed log with the
    /// delta sent behind it. The document is reported once as holding an edit
    /// the relay lacks, and it no longer reads as settled.
    #[test]
    fn a_delta_refused_by_name_leaves_the_log_and_is_reported_once() {
        let (mut state, rx, doc, _key, sk, _snap) = setup();
        state.signing_key = Some(sk);
        let (ws_tx, ws_rx) = ambre_bus::conduit::unbounded();
        state.set_mock_ws(ws_tx);
        state.handle_send_update(doc, b"a".to_vec());
        state.handle_send_update(doc, b"b".to_vec());
        let sent = signatures(&drain_ws(&ws_rx));
        let _ = drain(&rx);

        refuse_by_name(&mut state, doc, sent[0]);
        refuse_by_name(&mut state, doc, sent[1]);
        let rekey = &state.documents[&doc].rekey;
        assert_eq!(rekey.unconfirmed(), 0, "a refused delta stayed unconfirmed");
        assert!(
            !rekey.settled(),
            "a document missing an edit read as settled"
        );
        let told = drain(&rx);
        assert!(
            matches!(
                told.as_slice(),
                [LogicEvent::SyncWarning { document_id, code: SyncWarningCode::EditRefused, .. }]
                    if *document_id == doc
            ),
            "the refusal was told as {told:?}"
        );
    }

    /// A copy whose edit is unanswered waits to finalize, and one the relay
    /// refused an edit of withdraws (the Murmure segments spec). Neither
    /// seals a snapshot.
    #[test]
    fn a_copy_the_relay_lacks_an_edit_of_never_finalizes() {
        let (mut state, rx, doc, _key, sk, _snap) = setup();
        state.signing_key = Some(sk);
        let (ws_tx, ws_rx) = ambre_bus::conduit::unbounded();
        state.set_mock_ws(ws_tx);
        let (channel, segment) = (Id::<Channel>::mint(), Id::<Segment>::from_document(doc));
        let finalize = |state: &SyncState| {
            state.handle_finalize_segment(
                channel,
                segment,
                Id::<Lease>::mint(),
                ambre_bus::Seq::<ambre_bus::Relay>::new(3),
                &ambre_core::Payload::new(b"s".to_vec()),
            );
        };
        let ends = |events: &[LogicEvent]| -> Vec<ambre_core::FinalizeEnd> {
            events
                .iter()
                .filter_map(|e| match e {
                    LogicEvent::SegmentFinalizeFailed { end, .. } => Some(*end),
                    _ => None,
                })
                .collect()
        };

        let messages = |events: &[LogicEvent]| -> Vec<ApiFailure> {
            events
                .iter()
                .filter_map(|e| match e {
                    LogicEvent::SegmentFinalizeFailed { failure, .. } => Some(*failure),
                    _ => None,
                })
                .collect()
        };
        // A settled copy seals its snapshot, and stops only for want of a
        // relay on this transport.
        let _ = drain(&rx);
        finalize(&state);
        assert_eq!(
            messages(&drain(&rx)),
            [ApiFailure::Local(ambre_core::LocalFailure::NoRelay)]
        );

        state.handle_send_update(doc, b"a".to_vec());
        let sent = signatures(&drain_ws(&ws_rx));
        let _ = drain(&rx);
        finalize(&state);
        assert_eq!(
            messages(&drain(&rx)),
            [ApiFailure::Local(ambre_core::LocalFailure::EditPending)],
            "it waits for the echo"
        );

        refuse_by_name(&mut state, doc, sent[0]);
        let _ = drain(&rx);
        finalize(&state);
        assert_eq!(
            ends(&drain(&rx)),
            vec![ambre_core::FinalizeEnd::Withdrawn],
            "it withdraws"
        );
    }

    /// `SYNC-156`: once a delta is refused by name, no later delta of its
    /// document leaves. Neither a new edit nor one a refused clock sends again
    /// gets out. The document opened afresh sends once more.
    #[test]
    fn a_document_with_a_refused_delta_sends_no_further_delta() {
        let (mut state, _rx, doc, key, sk, _snap) = setup();
        state.signing_key = Some(sk);
        let (ws_tx, ws_rx) = ambre_bus::conduit::unbounded();
        state.set_mock_ws(ws_tx);
        state.handle_send_update(doc, b"a".to_vec());
        state.handle_send_update(doc, b"b".to_vec());
        let sent = signatures(&drain_ws(&ws_rx));

        refuse_by_name(&mut state, doc, sent[0]);
        state.handle_send_update(doc, b"c".to_vec());
        state.handle_ws_message(WsServerMsg::ClockRefused {
            document_id: doc,
            clock: Seq::<DeltaAuthor>::new(2),
            expected: Seq::<DeltaAuthor>::new(1),
            signature: sent[1],
        });
        assert!(
            signatures(&drain_ws(&ws_rx)).is_empty(),
            "a delta left a document holding a refused one"
        );

        let reopened = SyncDocumentState::new(doc, key, DocType::Trame, crate::Existence::OnRelay);
        state.documents.insert(doc, reopened);
        state.handle_send_update(doc, b"d".to_vec());
        assert_eq!(signatures(&drain_ws(&ws_rx)).len(), 1);
    }

    /// The relay failed to sequence the delta signed `signature`.
    fn fail_sequencing(
        state: &mut SyncState,
        doc: Id<Document>,
        signature: FixedBytes<Signature>,
    ) -> ambre_core::Seq<crate::generation::ResendTicket> {
        state.handle_ws_message(WsServerMsg::DeltaRefused {
            document_id: doc,
            signature,
            code: ambre_core::RefusalCode::InternalError,
        });
        state.resend_ticket.latest()
    }

    /// `SYNC-159`: a delta the relay failed to sequence waits for its timer.
    /// Then the answer lifts the fence, and the delta leaves again at its own
    /// clock with the one sent behind it. A stale timer sends nothing.
    #[test]
    fn a_delta_the_relay_failed_to_sequence_leaves_again_with_every_later_one() {
        let (mut state, rx, doc, key, sk, _snap) = setup();
        state.signing_key = Some(sk);
        let (ws_tx, ws_rx) = ambre_bus::conduit::unbounded();
        state.set_mock_ws(ws_tx);
        state.handle_send_update(doc, b"a".to_vec());
        state.handle_send_update(doc, b"b".to_vec());
        let sent = signatures(&drain_ws(&ws_rx));
        let _ = drain(&rx);

        let ticket = fail_sequencing(&mut state, doc, sent[0]);
        assert!(
            drain_ws(&ws_rx).is_empty(),
            "a failed delta left before its timer"
        );
        state.handle_sync_event(SyncEvent::ResendRetry {
            document_id: doc,
            ticket: ticket.next(),
        });
        assert!(drain_ws(&ws_rx).is_empty(), "a stale timer sent a delta");

        state.handle_sync_event(SyncEvent::ResendRetry {
            document_id: doc,
            ticket,
        });
        let mut msgs = drain_ws(&ws_rx);
        assert!(
            matches!(msgs.first(), Some(WsClientMsg::ClockResync { signature, .. }) if *signature == sent[0]),
            "the answer leaves first: {msgs:?}"
        );
        let resent = opened_updates(msgs.split_off(1), &[&key]);
        assert_eq!(
            resent,
            vec![(1, key.id(), b"a".to_vec()), (2, key.id(), b"b".to_vec())]
        );
        assert_eq!(state.documents[&doc].rekey.unconfirmed(), 2);
        assert!(
            drain(&rx).is_empty(),
            "a failure within the budget was reported"
        );
    }

    /// A replay timer and a resend timer count apart. Each starts from the same first ticket, and each fires its own work
    /// alone: the replay asks the missing deltas, the resend lifts the fence.
    #[test]
    fn a_replay_timer_and_a_resend_timer_fire_their_own_work_alone() {
        let (mut state, _rx, doc, _key, sk, _snap) = setup();
        state.signing_key = Some(sk);
        let (ws_tx, ws_rx) = ambre_bus::conduit::unbounded();
        state.set_mock_ws(ws_tx);
        state.handle_send_update(doc, b"a".to_vec());
        let sent = signatures(&drain_ws(&ws_rx));
        let resend = fail_sequencing(&mut state, doc, sent[0]);
        state.handle_ws_message(WsServerMsg::ReplayStopped {
            document_id: doc,
            after_seq: Seq::<Relay>::new(0),
        });
        let replay = state.replay_ticket.latest();
        assert!(drain_ws(&ws_rx).is_empty(), "no timer fired yet");

        state.handle_sync_event(SyncEvent::ReplayRetry {
            document_id: doc,
            ticket: replay,
        });
        let asked = drain_ws(&ws_rx);
        assert!(
            matches!(asked.as_slice(), [WsClientMsg::RequestMissing { .. }]),
            "the replay timer asks the missing deltas alone: {asked:?}"
        );

        state.handle_sync_event(SyncEvent::ResendRetry {
            document_id: doc,
            ticket: resend,
        });
        let resent = drain_ws(&ws_rx);
        assert!(
            matches!(resent.first(), Some(WsClientMsg::ClockResync { signature, .. }) if *signature == sent[0]),
            "the resend timer lifts the fence: {resent:?}"
        );
        assert!(
            !resent
                .iter()
                .any(|m| matches!(m, WsClientMsg::RequestMissing { .. })),
            "the resend timer asks no replay"
        );
    }

    /// `SYNC-159`: a delta that keeps failing waits longer each time, and past
    /// the budget the document holds it, reported once as a refused edit.
    #[test]
    fn a_delta_failing_past_the_budget_holds_its_document() {
        let failure = ambre_core::Count::<ambre_core::Failures>::new;
        assert_eq!(resend_retry_delay(failure(0)), None);
        assert_eq!(resend_retry_delay(failure(1)), Some(RESEND_RETRY_BASE));
        assert_eq!(
            resend_retry_delay(failure(2)),
            Some(TimeSpan::<Ms>::new(2 * RESEND_RETRY_BASE.get()))
        );
        assert_eq!(resend_retry_delay(RESEND_RETRY_LIMIT.incremented()), None);

        let (mut state, rx, doc, _key, sk, _snap) = setup();
        state.signing_key = Some(sk);
        let (ws_tx, ws_rx) = ambre_bus::conduit::unbounded();
        state.set_mock_ws(ws_tx);
        state.handle_send_update(doc, b"a".to_vec());
        let mut signature = signatures(&drain_ws(&ws_rx))[0];
        let _ = drain(&rx);
        for _ in 0..RESEND_RETRY_LIMIT.get() {
            let ticket = fail_sequencing(&mut state, doc, signature);
            state.handle_sync_event(SyncEvent::ResendRetry {
                document_id: doc,
                ticket,
            });
            signature = signatures(&drain_ws(&ws_rx))[0];
        }
        assert!(drain(&rx).is_empty());

        fail_sequencing(&mut state, doc, signature);
        let rekey = &state.documents[&doc].rekey;
        assert!(rekey.holds_refused() && rekey.unconfirmed() == 0);
        let told = drain(&rx);
        assert!(
            matches!(
                told.as_slice(),
                [LogicEvent::SyncWarning {
                    code: SyncWarningCode::EditRefused,
                    ..
                }]
            ),
            "the held delta was told as {told:?}"
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

        let theirs = make_update(
            doc,
            snap,
            Seq::<DeltaAuthor>::new(1),
            b"other session",
            &key,
            &sk,
        );
        state.receive_update(Seq::<Relay>::new(1), theirs);
        assert_eq!(
            state.documents[&doc].rekey.sent_len(),
            1,
            "still unconfirmed"
        );

        let Some(WsClientMsg::Update(echo)) = ours.into_iter().next() else {
            panic!("an update left");
        };
        state.receive_update(Seq::<Relay>::new(2), echo);
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
            clock: Seq::<DeltaAuthor>::new(1),
        });
        state.handle_send_update(doc, b"b".to_vec());
        assert!(drain_ws(&ws_rx).is_empty(), "nothing leaves before the key");

        let k2 = DocumentKey::generate();
        let link = pipeline::seal_link(
            doc,
            &k1,
            &k2,
            ambre_protocol::LogHead::EMPTY,
            state.signing_key.as_ref().unwrap(),
        )
        .unwrap();
        state.handle_sync_event(SyncEvent::DebtSettled {
            document_id: doc,
            new_doc_key: k2.clone(),
            link,
            departed: None,
        });
        assert_eq!(
            sent_updates(&ws_rx, &[&k1, &k2]),
            vec![(1, k2.id(), b"a".to_vec()), (2, k2.id(), b"b".to_vec())]
        );

        state.handle_ws_message(WsServerMsg::KeyRefused {
            document_id: doc,
            key_id: k1.id(),
            clock: Seq::<DeltaAuthor>::new(1),
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
            departed: None,
        });
        let _ = drain_ws(&ws_rx);

        state.handle_ws_message(WsServerMsg::KeyRefused {
            document_id: doc,
            key_id: k1.id(),
            clock: Seq::<DeltaAuthor>::new(1),
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
            departed: None,
        });
        let doc_state = &state.documents[&doc];
        assert_eq!(doc_state.doc_key.id(), k2.id());
        let cached: Vec<_> = doc_state.old_keys.iter().map(DocumentKey::id).collect();
        assert_eq!(cached, vec![k1.id()]);
        let late = make_update(doc, snap, Seq::<DeltaAuthor>::new(1), b"late", &k1, &sk);
        assert_eq!(
            doc_state
                .open_signed_update(&late, Seq::<Relay>::new(1))
                .unwrap()
                .payload,
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

    /// Holds `SYNC-213`: a rotation that names a key the current one
    /// replaced installs nothing. The relay served K1 again with no link,
    /// and K2 stays current, whether K1 sits in the cache or down the chain.
    #[test]
    fn a_rotation_back_to_a_replaced_key_is_refused() {
        let (mut state, _rx, doc, k1, _sk, _snap) = setup();
        let k2 = DocumentKey::generate();
        state.handle_sync_event(SyncEvent::KeyRotated {
            document_id: doc,
            new_doc_key: k2.clone(),
            links: Vec::new(),
            departed: None,
        });
        state.handle_sync_event(SyncEvent::KeyRotated {
            document_id: doc,
            new_doc_key: k1.clone(),
            links: Vec::new(),
            departed: None,
        });
        let doc_state = &state.documents[&doc];
        assert_eq!(doc_state.doc_key.id(), k2.id());
        // Down the chain alone: the cache no longer holds K1.
        let doc_state = state.documents.get_mut(&doc).unwrap();
        doc_state.old_keys.clear();
        let signing = ed25519_dalek::SigningKey::from_bytes(&[5; 32]);
        let link =
            crate::pipeline::seal_link(doc, &k1, &k2, ambre_protocol::LogHead::EMPTY, &signing)
                .unwrap();
        doc_state.chain.push(link);
        state.handle_sync_event(SyncEvent::KeyRotated {
            document_id: doc,
            new_doc_key: k1.clone(),
            links: Vec::new(),
            departed: None,
        });
        assert_eq!(state.documents[&doc].doc_key.id(), k2.id());
    }

    /// Holds `SYNC-223`. The relay answers each listing settled. A walk that
    /// holds a departure past the one the current key was checked against
    /// leaves an editor owing, and a viewer nothing. A payment the relay
    /// answers with the current key leaves the debt standing. Only a fresh
    /// key checked against that departure pays it, and the same walk then
    /// owes nothing more.
    #[test]
    fn a_departure_the_relay_calls_settled_still_owes_a_rotation() {
        let left = Some(ambre_core::Seq::<ambre_protocol::LogPlaces>::nth(3));
        let answer = |doc, asked, departed| SyncEvent::DocumentMembersReady {
            id: None,
            asked: ambre_core::Seq::nth(asked),
            document_id: doc,
            word: Default::default(),
            rotation_owed: ambre_core::RotationDebt::Settled,
            members: Vec::new(),
            signing_keys: Vec::new(),
            heads: Default::default(),
            writers: Default::default(),
            departed,
            revoked: None,
            root: Default::default(),
        };
        let (mut viewer, _rx, doc, _k, _sk, _snap) = setup();
        viewer.documents.get_mut(&doc).unwrap().role = Role::Viewer;
        viewer.handle_sync_event(answer(doc, 1, left));
        assert!(
            !viewer.documents[&doc].rekey.owes(),
            "a viewer seals nothing"
        );

        let (mut state, _rx, doc, k1, _sk, _snap) = setup();
        state.handle_sync_event(answer(doc, 1, None));
        assert!(!state.documents[&doc].rekey.owes());
        state.handle_sync_event(answer(doc, 2, left));
        assert!(state.documents[&doc].rekey.owes(), "the leaver holds k1");

        // The payment met a settled answer, and the relay served k1 again.
        state.handle_sync_event(SyncEvent::KeyRotated {
            document_id: doc,
            new_doc_key: k1.clone(),
            links: Vec::new(),
            departed: left,
        });
        assert_eq!(state.documents[&doc].doc_key.id(), k1.id());
        assert!(state.documents[&doc].rekey.owes(), "k1 pays no debt");

        let k2 = DocumentKey::generate();
        state.handle_sync_event(SyncEvent::KeyRotated {
            document_id: doc,
            new_doc_key: k2.clone(),
            links: Vec::new(),
            departed: left,
        });
        assert_eq!(state.documents[&doc].doc_key.id(), k2.id());
        assert!(!state.documents[&doc].rekey.owes());
        state.handle_sync_event(answer(doc, 3, left));
        assert!(!state.documents[&doc].rekey.owes(), "k2 postdates it");
    }

    /// Holds `SYNC-224`. An editor seals under `k0` when a walk holds a revoke
    /// that installed `k1`, and the relay withholds the rotation. Every edit
    /// then waits, and no payment starts. The relay serving `k0` again, or a
    /// key whose link leads back to `k0` alone, ends nothing. `k1` ends the
    /// wait, and the held edit leaves under it at the next clock. A walk
    /// whose revoke the current key leads to holds nothing, and a viewer
    /// holds nothing either.
    #[test]
    fn a_revoke_whose_key_is_withheld_holds_every_edit() {
        let place = ambre_core::Seq::<ambre_protocol::LogPlaces>::nth(4);
        let answer = |doc, asked, key: &DocumentKey| SyncEvent::DocumentMembersReady {
            id: None,
            asked: ambre_core::Seq::nth(asked),
            document_id: doc,
            word: Default::default(),
            rotation_owed: ambre_core::RotationDebt::Settled,
            members: Vec::new(),
            signing_keys: Vec::new(),
            heads: Default::default(),
            writers: Default::default(),
            departed: None,
            revoked: Some(membership_log::NamedKey {
                place,
                key: key.id(),
            }),
            root: Default::default(),
        };
        let rotated = |doc, key: &DocumentKey, links| SyncEvent::KeyRotated {
            document_id: doc,
            new_doc_key: key.clone(),
            links,
            departed: None,
        };
        let k1 = DocumentKey::generate();

        let (mut viewer, _rx, doc, _k0, _sk, _snap) = setup();
        viewer.documents.get_mut(&doc).unwrap().role = Role::Viewer;
        viewer.handle_sync_event(answer(doc, 1, &k1));
        assert!(
            !viewer.documents[&doc].rekey.owes(),
            "a viewer seals nothing"
        );

        let (mut state, _rx, doc, k0, sk, _snap) = setup();
        state.signing_key = Some(sk);
        let (ws_tx, ws_rx) = ambre_bus::conduit::unbounded();
        state.set_mock_ws(ws_tx);
        state.handle_send_update(doc, b"a".to_vec());
        assert_eq!(
            sent_updates(&ws_rx, &[&k0]),
            vec![(1, k0.id(), b"a".to_vec())]
        );

        state.handle_sync_event(answer(doc, 1, &k1));
        state.handle_send_update(doc, b"b".to_vec());
        assert!(drain_ws(&ws_rx).is_empty(), "the revoked member holds k0");

        state.handle_sync_event(rotated(doc, &k0, Vec::new()));
        let astray = DocumentKey::generate();
        let sk = state.signing_key.clone().unwrap();
        let from_k0 =
            pipeline::seal_link(doc, &k0, &astray, ambre_protocol::LogHead::EMPTY, &sk).unwrap();
        state.handle_sync_event(rotated(doc, &astray, vec![from_k0]));
        assert_eq!(state.documents[&doc].doc_key.id(), k0.id());
        assert!(drain_ws(&ws_rx).is_empty(), "no key past k1 came");

        let link = pipeline::seal_link(doc, &k0, &k1, ambre_protocol::LogHead::EMPTY, &sk).unwrap();
        state.handle_sync_event(rotated(doc, &k1, vec![link]));
        assert_eq!(
            sent_updates(&ws_rx, &[&k0, &k1]),
            vec![(2, k1.id(), b"b".to_vec())]
        );
        state.handle_sync_event(answer(doc, 2, &k1));
        state.handle_send_update(doc, b"c".to_vec());
        assert_eq!(
            sent_updates(&ws_rx, &[&k1]),
            vec![(3, k1.id(), b"c".to_vec())],
            "k1 follows the revoke"
        );
    }

    /// Holds `SYNC-224`. The wait for the key a revoke installed ends on a
    /// later key too, when its link leads back to that key.
    #[test]
    fn a_key_past_the_withheld_revoke_ends_the_wait() {
        let (mut state, _rx, doc, k0, sk, _snap) = setup();
        state.signing_key = Some(sk.clone());
        let (ws_tx, ws_rx) = ambre_bus::conduit::unbounded();
        state.set_mock_ws(ws_tx);
        let k1 = DocumentKey::generate();
        let k2 = DocumentKey::generate();
        state.handle_sync_event(SyncEvent::DocumentMembersReady {
            id: None,
            asked: ambre_core::Seq::nth(1),
            document_id: doc,
            word: Default::default(),
            rotation_owed: ambre_core::RotationDebt::Settled,
            members: Vec::new(),
            signing_keys: Vec::new(),
            heads: Default::default(),
            writers: Default::default(),
            departed: None,
            revoked: Some(membership_log::NamedKey {
                place: ambre_core::Seq::nth(4),
                key: k1.id(),
            }),
            root: Default::default(),
        });
        state.handle_send_update(doc, b"a".to_vec());
        assert!(drain_ws(&ws_rx).is_empty());

        let links = vec![
            pipeline::seal_link(doc, &k0, &k1, ambre_protocol::LogHead::EMPTY, &sk).unwrap(),
            pipeline::seal_link(doc, &k1, &k2, ambre_protocol::LogHead::EMPTY, &sk).unwrap(),
        ];
        state.handle_sync_event(SyncEvent::KeyRotated {
            document_id: doc,
            new_doc_key: k2.clone(),
            links,
            departed: None,
        });
        assert_eq!(
            sent_updates(&ws_rx, &[&k2]),
            vec![(1, k2.id(), b"a".to_vec())]
        );
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
                departed: None,
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
    /// against its holder's (`SYNC-034`). A row that names the holder as its
    /// wrapper opens only a copy the holder wrapped itself.
    #[test]
    fn a_copy_unwraps_against_its_wrapper_and_never_its_holder() {
        let me = Id::<User>::mint();
        let my_xk = ambre_crypto::MasterSeed::from_array([3; 32]).derive_exchange_key();
        let peer_xk = ambre_crypto::MasterSeed::from_array([4; 32]).derive_exchange_key();
        let doc_key = DocumentKey::generate();
        // The row `wrapper` wrapped, its wrapper field rewritten to `named`.
        let unwrapped =
            |wrapper: &x25519_dalek::StaticSecret,
             named: Option<FixedBytes<ambre_core::ExchangePublicKey>>| {
                let mut row = member_row(me, &my_xk, wrapper, &doc_key, ambre_core::Role::Editor);
                if let Some(named) = named {
                    row.wrapper_exchange_pub_key = http::ServedKey::of(named);
                }
                SyncState::unwrap_document_key(&my_xk, &[row])
                    .map(|key| key.id())
                    .ok()
            };
        let mine = crate::sharing::own_exchange_key(&my_xk);

        assert_eq!(unwrapped(&peer_xk, None), Some(doc_key.id()));
        assert!(unwrapped(&peer_xk, Some(mine)).is_none());
        assert_eq!(unwrapped(&my_xk, Some(mine)), Some(doc_key.id()));
    }

    /// Every refusal of a live frame reaches the Logic thread as one warning
    /// naming the document (`SYNC-091`). Its code names the check that
    /// failed, so a missed re-key reads apart from a corrupt frame.
    #[test]
    fn every_refused_frame_reaches_logic_as_a_warning() {
        let (mut state, rx, doc, key, sk, snap) = setup();
        let mut unsigned = make_update(doc, snap, Seq::<DeltaAuthor>::new(1), b"a", &key, &sk);
        unsigned.signature = FixedBytes::from_array([0; 64]);
        let stranger = DocumentKey::generate();
        let under_stranger =
            make_update(doc, snap, Seq::<DeltaAuthor>::new(2), b"b", &stranger, &sk);
        let mut corrupt = make_update(doc, snap, Seq::<DeltaAuthor>::new(3), b"c", &key, &sk);
        corrupt.commitment_tag = FixedBytes::from_array([0; 32]);

        let mut messages = Vec::new();
        for (seq, update) in [(1, unsigned), (2, under_stranger), (3, corrupt)] {
            state.receive_update(Seq::<Relay>::new(seq), update);
            match drain(&rx).as_slice() {
                [LogicEvent::SyncWarning { document_id, code }] if *document_id == doc => {
                    messages.push(*code)
                }
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
            let doc = Id::<Document>::mint();
            state.handle_sync_event(SyncEvent::OpenDocumentReady {
                document_id: doc,
                doc_key: DocumentKey::generate(),
                snapshot_payload: None,
                snapshot_id: Id::<Snapshot>::genesis(doc),
                covered_seq: Seq::<Relay>::new(0),
                held_snapshot: None,
                doc_type: DocType::Trame,
                existence: crate::Existence::OnRelay,
                signing_keys: Vec::new(),
                role,
                rotation_owed: ambre_core::RotationDebt::Owed,
                links: Vec::new(),
                writers: Default::default(),
                departed: None,
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
            SyncDocumentState::new(doc, key.clone(), DocType::Trame, crate::Existence::OnRelay),
        );
        state.receive_update(
            Seq::<Relay>::new(1),
            make_update(doc, snap, Seq::<DeltaAuthor>::new(1), b"a", &key, &sk),
        );
        state.receive_update(
            Seq::<Relay>::new(2),
            make_update(doc, snap, Seq::<DeltaAuthor>::new(2), b"b", &key, &sk),
        );
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
            let doc = Id::<Document>::mint();
            let key = DocumentKey::generate();
            let snapshot_id = Id::<Snapshot>::mint();
            let blob = with_snapshot.then(|| {
                snapshot_blob(
                    doc,
                    served(snapshot_id, u64::from(with_snapshot)),
                    b"s",
                    &key,
                    &sk,
                )
            });
            state.handle_sync_event(SyncEvent::OpenDocumentReady {
                document_id: doc,
                doc_key: key,
                snapshot_payload: blob,
                snapshot_id: if with_snapshot {
                    snapshot_id
                } else {
                    Id::<Snapshot>::genesis(doc)
                },
                covered_seq: Seq::<Relay>::new(u64::from(with_snapshot)),
                held_snapshot: None,
                doc_type: DocType::Trame,
                existence: crate::Existence::OnRelay,
                signing_keys: Vec::new(),
                role: Role::Owner,
                rotation_owed: ambre_core::RotationDebt::Settled,
                links: Vec::new(),
                writers: Default::default(),
                departed: None,
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

    /// `SYNC-087`: the record of applied numbers keeps the latest
    /// `APPLIED_WINDOW` of them. The oldest leaves once the window is full,
    /// and every number inside it still counts as applied.
    #[test]
    fn the_applied_window_keeps_the_latest_numbers() {
        let mut doc = SyncDocumentState::new(
            Id::<Document>::mint(),
            DocumentKey::generate(),
            DocType::Prose,
            crate::Existence::LocalOnly,
        );
        let window = u64::from(crate::APPLIED_WINDOW.get());
        for seq in 1..=window + 1 {
            assert!(doc.first_arrival(Seq::<Relay>::new(seq)));
        }
        assert_eq!(doc.ingested.len(), crate::APPLIED_WINDOW.get() as usize);
        assert!(
            !doc.first_arrival(Seq::<Relay>::new(2)),
            "inside the window"
        );
        assert!(doc.first_arrival(Seq::<Relay>::new(1)), "left the window");
    }

    /// Holds `SYNC-087`: a reconnect replay that overlaps a broadcast already
    /// received delivers the overlap once. A copy held in quarantine is not
    /// held a second time either.
    #[test]
    fn a_reconnect_replay_applies_nothing_twice() {
        let (mut state, rx, doc, key, sk, snap) = setup();
        let update = |seq: u64, parent| SeqUpdate {
            seq: Seq::<Relay>::new(seq),
            update: make_update(
                doc,
                parent,
                Seq::<DeltaAuthor>::new(seq),
                &[seq as u8],
                &key,
                &sk,
            ),
        };
        state.handle_ws_message(WsServerMsg::Update {
            seq: Seq::<Relay>::new(3),
            update: update(3, snap).update,
        });
        state.handle_ws_message(WsServerMsg::MissingUpdates {
            document_id: doc,
            updates: vec![update(2, snap), update(3, snap)],
        });
        let delivered: Vec<Vec<u8>> = drain(&rx)
            .into_iter()
            .flat_map(|e| match e {
                LogicEvent::RemoteUpdate { delta, .. } => vec![delta.payload.into_bytes()],
                LogicEvent::RemoteUpdateBatch { deltas, .. } => {
                    deltas.into_iter().map(|d| d.payload.into_bytes()).collect()
                }
                _ => Vec::new(),
            })
            .collect();
        assert_eq!(delivered, [vec![3], vec![2]]);

        let parent = Id::<Snapshot>::mint();
        state.receive_update(Seq::<Relay>::new(9), update(9, parent).update);
        state.receive_update(Seq::<Relay>::new(9), update(9, parent).update);
        assert_eq!(state.documents[&doc].quarantine_len().get(), 1);
    }

    /// `SYNC-203`: a replay the relay files under another document delivers
    /// nothing of an update sealed for this one, under either document.
    #[test]
    fn a_replay_filed_under_another_document_delivers_nothing() {
        let (mut state, rx, doc, key, sk, snap) = setup();
        let other = Id::<Document>::mint();
        state.handle_ws_message(WsServerMsg::MissingUpdates {
            document_id: other,
            updates: vec![SeqUpdate {
                seq: Seq::<Relay>::new(1),
                update: make_update(doc, snap, Seq::<DeltaAuthor>::new(1), b"A", &key, &sk),
            }],
        });
        let delivered = drain(&rx)
            .into_iter()
            .filter(|e| {
                matches!(
                    e,
                    LogicEvent::RemoteUpdate { .. } | LogicEvent::RemoteUpdateBatch { .. }
                )
            })
            .count();
        assert_eq!(delivered, 0);
        // The same update, filed under its own document, still delivers.
        state.handle_ws_message(WsServerMsg::MissingUpdates {
            document_id: doc,
            updates: vec![SeqUpdate {
                seq: Seq::<Relay>::new(1),
                update: make_update(doc, snap, Seq::<DeltaAuthor>::new(1), b"A", &key, &sk),
            }],
        });
        assert!(drain(&rx).into_iter().any(|e| matches!(
            e,
            LogicEvent::RemoteUpdateBatch { document_id, .. } if document_id == doc
        )));
    }

    /// `SYNC-027`: a replay of several deltas reaches the Logic thread as one
    /// batch, in the order of their numbers, and as no single event.
    #[test]
    fn a_history_replay_reaches_logic_as_one_batch() {
        let (mut state, rx, doc, key, sk, snap) = setup();
        let updates = (1..=5)
            .map(|seq| SeqUpdate {
                seq: Seq::<Relay>::new(seq),
                update: make_update(
                    doc,
                    snap,
                    Seq::<DeltaAuthor>::new(seq),
                    &[seq as u8],
                    &key,
                    &sk,
                ),
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
                    Some(deltas.into_iter().map(|d| d.payload.into_bytes()).collect())
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
        state.receive_update(
            Seq::<Relay>::new(1),
            make_update(doc, snap, Seq::<DeltaAuthor>::new(41), b"mine", &key, &sk),
        );
        state.receive_update(
            Seq::<Relay>::new(2),
            make_update(doc, snap, Seq::<DeltaAuthor>::new(99), b"peer", &key, &peer),
        );
        assert_eq!(
            state.documents[&doc].local_clock,
            Seq::<DeltaAuthor>::new(41)
        );

        state.handle_command(SyncCommand::SendUpdate {
            document_id: doc,
            payload: b"next".to_vec().into(),
            summary: None,
        });
        let clocks: Vec<u64> = drain_ws(&ws_rx)
            .into_iter()
            .filter_map(|m| match m {
                WsClientMsg::Update(u) => Some(u.public_data.clock.get()),
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
        let update = make_update(
            doc,
            Id::<Snapshot>::mint(),
            Seq::<DeltaAuthor>::new(1),
            b"x",
            &stranger,
            &sk,
        );
        state.receive_update(Seq::<Relay>::new(1), update);
        assert_eq!(state.documents[&doc].quarantine_len().get(), 1);
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
        let before = make_update(doc, snap, Seq::<DeltaAuthor>::new(1), b"hello", &k1, &sk);
        state.receive_update(Seq::<Relay>::new(1), before.clone());
        let _ = drain(&rx);

        let k2 = DocumentKey::generate();
        let link = pipeline::seal_link(doc, &k1, &k2, ambre_protocol::LogHead::EMPTY, &sk).unwrap();
        state.handle_sync_event(SyncEvent::RevocationComplete {
            id: ambre_bus::next_request(),
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
                .open_signed_update(&before, Seq::<Relay>::new(1))
                .unwrap()
                .payload,
            b"hello"
        );
    }

    /// `SYNC-168`: each rotation this client makes reaches Logic, a revoke
    /// as well as an owed rotation it paid.
    #[test]
    fn each_rotation_this_client_makes_reaches_logic() {
        let (mut state, rx, doc, k1, sk, _snap) = setup();
        state.signing_key = Some(sk.clone());
        let rotated = |rx: &ambre_bus::conduit::Receiver<LogicEvent>| {
            drain(rx)
                .into_iter()
                .filter(
                    |e| matches!(e, LogicEvent::KeyRotated { document_id } if *document_id == doc),
                )
                .count()
        };

        let k2 = DocumentKey::generate();
        state.handle_sync_event(SyncEvent::RevocationComplete {
            id: ambre_bus::next_request(),
            document_id: doc,
            new_doc_key: k2.clone(),
            link: pipeline::seal_link(doc, &k1, &k2, ambre_protocol::LogHead::EMPTY, &sk).unwrap(),
        });
        assert_eq!(rotated(&rx), 1, "a revoke");

        let k3 = DocumentKey::generate();
        state.handle_sync_event(SyncEvent::DebtSettled {
            document_id: doc,
            new_doc_key: k3.clone(),
            link: pipeline::seal_link(doc, &k2, &k3, ambre_protocol::LogHead::EMPTY, &sk).unwrap(),
            departed: None,
        });
        assert_eq!(rotated(&rx), 1, "an owed rotation paid");
    }

    /// Holds `SYNC-205` for the session: the pin a link hands over stays the
    /// one a later open of the document reads, past a close.
    #[test]
    fn a_pinned_inviter_outlives_a_close() {
        let (mut state, _rx, doc, ..) = setup();
        let pin = ambre_crypto::inviter_pin(&FixedBytes::from_array([4; 32]));
        state.handle_command(ambre_bus::SyncCommand::PinInviter {
            document_id: doc,
            inviter: pin,
        });
        state.handle_command(ambre_bus::SyncCommand::CloseDocument { document_id: doc });
        assert_eq!(state.pinned_inviters.get(&doc), Some(&pin));
    }

    /// Holds `SYNC-097`: what the start held is released in the order it
    /// came. The opens subscribe in order, and the edits leave in order at
    /// rising clocks once the replay has caught the clock up.
    #[test]
    fn held_commands_are_released_in_order() {
        let (mut state, _rx, doc, key, sk, _snap) = setup();
        let (ws_tx, ws_rx) = ambre_bus::conduit::unbounded();
        state.set_mock_ws(ws_tx);

        let opens: Vec<Id<Document>> = (0..3).map(|_| Id::<Document>::mint()).collect();
        state.pending_opens = opens.iter().map(|d| (*d, DocType::Trame)).collect();
        state.flush_pending_opens();
        let subscribed: Vec<Id<Document>> = drain_ws(&ws_rx)
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
        rx: &ambre_bus::conduit::Receiver<Payload<ambre_protocol::WireFrame>>,
        key: &DocumentKey,
    ) -> Vec<(FixedBytes<SessionNonce>, Vec<u8>)> {
        drain_ws(rx)
            .into_iter()
            .filter_map(|msg| match msg {
                WsClientMsg::Update(u) => {
                    let body = pipeline::open_update(&u, key).expect("opens");
                    let body = pipeline::split_update_body(body).expect("splits");
                    Some((body.nonce, body.delta.into_bytes()))
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
        let earlier = FixedBytes::<SessionNonce>::from_array([7; 16]);

        state.handle_send_unsent(doc, b"old".to_vec().into(), earlier);
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
            departed: None,
        });
        let _ = drain_ws(&ws_rx);
        state.handle_ws_message(WsServerMsg::KeyRefused {
            document_id: doc,
            key_id: k1.id(),
            clock: Seq::<DeltaAuthor>::new(1),
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
        let mut forged = make_update(doc, snap, Seq::<DeltaAuthor>::new(2), b"forged", &key, &sk);
        let mut bytes = forged.ciphertext.as_bytes().to_vec();
        bytes[0] ^= 1;
        forged.ciphertext = Payload::<ambre_core::Ciphertext>::new(bytes);
        let mut unsigned = make_update(
            doc,
            snap,
            Seq::<DeltaAuthor>::new(3),
            b"unsigned",
            &key,
            &sk,
        );
        unsigned.signature = FixedBytes::from_array([0; 64]);
        let updates = [
            (
                1,
                make_update(doc, snap, Seq::<DeltaAuthor>::new(1), b"one", &key, &sk),
            ),
            (2, forged),
            (3, unsigned),
            (
                4,
                make_update(doc, snap, Seq::<DeltaAuthor>::new(4), b"four", &key, &sk),
            ),
        ]
        .into_iter()
        .map(|(seq, update)| SeqUpdate {
            seq: Seq::<Relay>::new(seq),
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
                    assert!(matches!(
                        code,
                        SyncWarningCode::DecryptFailed | SyncWarningCode::SignatureRefused
                    ));
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
        let unknown = Id::<Document>::mint();

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
        assert_eq!(state.read_up_to(Id::<Document>::mint()), None, "not open");

        state.receive_update(
            Seq::<Relay>::new(4),
            make_update(doc, snap, Seq::<DeltaAuthor>::new(1), b"shown", &key, &sk),
        );
        assert_eq!(state.read_up_to(doc), Some(Seq::<Relay>::new(4)));

        let held = make_update(
            doc,
            Id::<Snapshot>::mint(),
            Seq::<DeltaAuthor>::new(2),
            b"held",
            &key,
            &sk,
        );
        state.receive_update(Seq::<Relay>::new(5), held);
        assert_eq!(state.read_up_to(doc), None, "a held update stays unread");
    }

    /// The writers a log grants where Alice owns the document, Bob writes,
    /// Carol reads and Dave wrote until Alice removed him (`SYNC-211`).
    fn writers_of_a_shared_log() -> (
        crate::membership_log::Writers,
        [crate::membership_log::tests::Person; 4],
    ) {
        use crate::membership_log::tests::{Person, append, created};
        use ambre_protocol::MembershipAct;
        let people = [1, 2, 3, 4].map(Person::new);
        let [alice, bob, carol, dave] = &people;
        let mut log = created(alice, bob);
        append(&mut log, alice, carol, MembershipAct::Invite, Role::Viewer);
        append(&mut log, alice, dave, MembershipAct::Invite, Role::Editor);
        append(&mut log, alice, dave, MembershipAct::Revoke, Role::Editor);
        let walked = crate::membership_log::Log::walk(
            crate::membership_log::tests::DOC,
            log.first().and_then(crate::membership_log::digest),
            &log,
            crate::membership_log::no_self_join,
        );
        (crate::membership_log::Writers::of(&walked), people)
    }

    /// The members answer to the listing `asked`, carrying `writers` alone.
    fn listing(
        doc: Id<Document>,
        asked: u64,
        writers: crate::membership_log::Writers,
    ) -> SyncEvent {
        rooted_listing(doc, asked, writers, crate::membership_log::Root::Free)
    }

    /// The members answer to the listing `asked`, carrying `writers` and the
    /// `root` its walk found.
    fn rooted_listing(
        doc: Id<Document>,
        asked: u64,
        writers: crate::membership_log::Writers,
        root: crate::membership_log::Root,
    ) -> SyncEvent {
        SyncEvent::DocumentMembersReady {
            id: None,
            asked: ambre_core::Seq::nth(asked),
            document_id: doc,
            word: Default::default(),
            rotation_owed: ambre_core::RotationDebt::Settled,
            members: Vec::new(),
            signing_keys: Vec::new(),
            heads: Default::default(),
            writers,
            departed: None,
            revoked: None,
            root,
        }
    }

    /// The payloads that reached the document, and the warnings raised.
    fn applied_and_warned(events: &[LogicEvent]) -> (Vec<Vec<u8>>, Vec<SyncWarningCode>) {
        let applied = events
            .iter()
            .filter_map(|e| match e {
                LogicEvent::RemoteUpdate { delta, .. } => Some(delta.payload.as_bytes().to_vec()),
                _ => None,
            })
            .collect();
        let warned = events
            .iter()
            .filter_map(|e| match e {
                LogicEvent::SyncWarning { code, .. } => Some(*code),
                _ => None,
            })
            .collect();
        (applied, warned)
    }

    /// Holds `SYNC-211` against a relay that forwards what it is given. A
    /// reader holds the current key and a retired one, and a removed reader
    /// would hold the retired one. Each delta the reader seals, under either
    /// key, reaches no document once the listing it waited for answers. A
    /// writer's delta lands at once, and so does a removed writer's: the log
    /// granted him a writing role.
    #[test]
    fn a_relay_forwarding_a_readers_delta_reaches_no_document() {
        let (mut state, rx, doc, retired, _sk, snap) = setup();
        let (writers, [alice, _bob, carol, dave]) = writers_of_a_shared_log();
        let current = DocumentKey::generate();
        let doc_state = state.documents.get_mut(&doc).unwrap();
        doc_state.writers = writers.clone();
        doc_state.rotate_key(current.clone());

        let frame = |clock: u64, payload: &[u8], key: &DocumentKey, by: &SigningKey| {
            make_update(doc, snap, Seq::<DeltaAuthor>::new(clock), payload, key, by)
        };
        let forwarded = [
            frame(1, b"reader, current key", &current, &carol.key),
            frame(2, b"reader, retired key", &retired, &carol.key),
            frame(3, b"owner", &current, &alice.key),
            frame(4, b"removed writer, retired key", &retired, &dave.key),
        ];
        for (seq, update) in (1..).zip(forwarded) {
            state.handle_ws_message(WsServerMsg::Update {
                seq: Seq::<Relay>::new(seq),
                update,
            });
        }
        let (applied, warned) = applied_and_warned(&drain(&rx));
        assert_eq!(
            applied,
            vec![b"owner".to_vec(), b"removed writer, retired key".to_vec()]
        );
        assert!(warned.is_empty(), "the reader's deltas wait for a listing");
        assert_eq!(
            state.documents[&doc].directory_asked.latest(),
            ambre_core::Seq::nth(1),
            "one listing in flight"
        );

        // Each answer settles the deltas that arrived before its listing was
        // asked, and the first asks once more for the rest.
        for asked in [1, 2] {
            state.handle_sync_event(listing(doc, asked, writers.clone()));
            let (applied, warned) = applied_and_warned(&drain(&rx));
            assert!(applied.is_empty(), "no reader's delta reaches the document");
            assert_eq!(warned, vec![SyncWarningCode::WriterRefused]);
        }
        assert!(state.documents[&doc].unvouched.is_empty());
        assert_eq!(
            state.documents[&doc].directory_asked.latest(),
            ambre_core::Seq::nth(2)
        );
    }

    /// A reader's delta that waited in the quarantine for its parent does
    /// not reach the document when the parent lands: it is held for a
    /// listing, as on arrival. The owner's delta beside it lands.
    #[test]
    fn a_readers_delta_released_from_the_quarantine_is_held_for_a_listing() {
        let (mut state, rx, doc, key, _sk, _snap) = setup();
        let (writers, [alice, _bob, carol, _dave]) = writers_of_a_shared_log();
        state.documents.get_mut(&doc).unwrap().writers = writers;
        let parent = Id::<Snapshot>::mint();
        let waiting = [(b"reader".as_slice(), &carol.key), (b"owner", &alice.key)];
        for (clock, (payload, by)) in (1..).zip(waiting) {
            let update = make_update(
                doc,
                parent,
                Seq::<DeltaAuthor>::new(clock),
                payload,
                &key,
                by,
            );
            state.receive_update(Seq::<Relay>::new(clock), update);
        }
        assert!(drain(&rx).is_empty(), "an unknown parent holds both");

        state.snapshot_known(doc, parent);
        let (applied, _) = applied_and_warned(&drain(&rx));
        assert_eq!(applied, vec![b"owner".to_vec()]);
        assert_eq!(state.documents[&doc].unvouched.len(), 1);
    }

    /// Past the bound of the held deltas, the oldest one is dropped and Logic
    /// is warned once. Under the bound, nothing is dropped and no warning
    /// goes.
    #[test]
    fn held_deltas_past_their_bound_drop_the_oldest_and_warn() {
        let (mut state, rx, doc, key, sk, snap) = setup();
        let held = |from: u64, to: u64| {
            (from..to)
                .map(|clock| {
                    let update =
                        make_update(doc, snap, Seq::<DeltaAuthor>::new(clock), b"x", &key, &sk);
                    (Seq::<Relay>::new(clock), update)
                })
                .collect::<Vec<_>>()
        };
        let limit = QUARANTINE_LIMIT as u64;
        state.hold_unvouched(doc, held(1, limit + 1));
        let (_, warned) = applied_and_warned(&drain(&rx));
        assert!(warned.is_empty(), "the bound itself drops nothing");
        assert_eq!(state.documents[&doc].unvouched.len(), QUARANTINE_LIMIT);

        state.hold_unvouched(doc, held(limit + 1, limit + 2));
        let (_, warned) = applied_and_warned(&drain(&rx));
        assert_eq!(warned, vec![SyncWarningCode::WriterRefused]);
        let unvouched = &state.documents[&doc].unvouched;
        assert_eq!(unvouched.len(), QUARANTINE_LIMIT);
        assert_eq!(
            unvouched.first().map(|held| held.seq),
            Some(Seq::<Relay>::new(2)),
            "the oldest held delta is the one dropped"
        );
    }

    /// Holds `SYNC-214`. A reader's delta waits for a listing. The relay
    /// answers it with a walk that drops our row, so it walks unanchored, or
    /// with a log of its own. Neither answer replaces the writers the open
    /// walked, and the reader's delta reaches no document.
    #[test]
    fn a_listing_that_drops_a_writer_leaves_the_writers_of_the_open() {
        use crate::membership_log::tests::{Person, created};
        let (writers, [_alice, _bob, carol, _dave]) = writers_of_a_shared_log();
        let relays = created(&Person::new(9), &carol);
        let rerooted = crate::membership_log::Writers::of(&crate::membership_log::Log::walk(
            crate::membership_log::tests::DOC,
            relays.first().and_then(crate::membership_log::digest),
            &relays,
            crate::membership_log::no_self_join,
        ));
        assert!(rerooted.admits(&carol.public()));
        for answer in [crate::membership_log::Writers::Unchecked, rerooted] {
            let (mut state, rx, doc, key, _sk, snap) = setup();
            state.documents.get_mut(&doc).unwrap().writers = writers.clone();
            state.handle_ws_message(WsServerMsg::Update {
                seq: Seq::<Relay>::new(1),
                update: make_update(
                    doc,
                    snap,
                    Seq::<DeltaAuthor>::new(1),
                    b"reader",
                    &key,
                    &carol.key,
                ),
            });
            state.handle_sync_event(listing(doc, 1, answer));
            let (applied, _) = applied_and_warned(&drain(&rx));
            assert!(applied.is_empty(), "the reader's delta reaches no document");
            assert!(state.documents[&doc].writers == writers);
        }
    }

    /// Audit `N16`, at the delta. A space document checks its writers
    /// against its own log, as any document does. A viewer of the space who
    /// writes a catalogue entry, forwarded by the relay, reaches no document;
    /// the owner's entry lands.
    #[test]
    fn a_viewers_catalogue_entry_reaches_no_space_document() {
        let (mut state, rx, _doc, _key, _sk, _snap) = setup();
        let (writers, [alice, _bob, carol, _dave]) = writers_of_a_shared_log();
        let space = Id::<Space>::mint().document_id();
        let key = DocumentKey::generate();
        let snapshot_id = Id::<Snapshot>::genesis(space);
        state.handle_command(ambre_bus::SyncCommand::OpenDocument {
            document_id: space,
            doc_type: DocType::Murmure,
            meta: None,
        });
        state.handle_sync_event(SyncEvent::OpenDocumentReady {
            document_id: space,
            doc_key: key.clone(),
            snapshot_payload: None,
            snapshot_id,
            covered_seq: Seq::<Relay>::ZERO,
            held_snapshot: None,
            doc_type: DocType::Murmure,
            existence: crate::Existence::OnRelay,
            signing_keys: Vec::new(),
            role: Role::Owner,
            rotation_owed: ambre_core::RotationDebt::Settled,
            links: Vec::new(),
            writers: writers.clone(),
            departed: None,
        });
        drain(&rx);
        let entries = [
            (b"viewer's catalogue entry".as_slice(), &carol.key),
            (b"owner's catalogue entry".as_slice(), &alice.key),
        ];
        for (clock, (payload, by)) in (1..).zip(entries) {
            let update = make_update(
                space,
                snapshot_id,
                Seq::<DeltaAuthor>::new(clock),
                payload,
                &key,
                by,
            );
            state.handle_ws_message(WsServerMsg::Update {
                seq: Seq::<Relay>::new(clock),
                update,
            });
        }
        let (applied, _) = applied_and_warned(&drain(&rx));
        assert_eq!(applied, vec![b"owner's catalogue entry".to_vec()]);
        state.handle_sync_event(listing(space, 1, writers));
        let (applied, warned) = applied_and_warned(&drain(&rx));
        assert!(applied.is_empty(), "the viewer's entry reaches no document");
        assert_eq!(warned, vec![SyncWarningCode::WriterRefused]);
    }

    /// Holds `SYNC-211` for a snapshot: one the reader sealed opens nothing,
    /// and the open falls back to the full replay. The owner's snapshot opens.
    #[test]
    fn a_snapshot_no_writer_signed_opens_nothing() {
        let (writers, [alice, _bob, carol, _dave]) = writers_of_a_shared_log();
        for (author, opens) in [(&carol, false), (&alice, true)] {
            let (mut state, rx, _doc, _key, _sk, _snap) = setup();
            let doc = Id::<Document>::mint();
            let key = DocumentKey::generate();
            let snapshot_id = Id::<Snapshot>::mint();
            state.handle_sync_event(SyncEvent::OpenDocumentReady {
                document_id: doc,
                doc_key: key.clone(),
                snapshot_payload: Some(snapshot_blob(
                    doc,
                    served(snapshot_id, 3),
                    b"snap",
                    &key,
                    &author.key,
                )),
                snapshot_id,
                covered_seq: Seq::<Relay>::new(3),
                held_snapshot: None,
                doc_type: DocType::Trame,
                existence: crate::Existence::OnRelay,
                signing_keys: Vec::new(),
                role: Role::Owner,
                rotation_owed: ambre_core::RotationDebt::Settled,
                links: Vec::new(),
                writers: writers.clone(),
                departed: None,
            });
            let events = drain(&rx);
            let opened = events
                .iter()
                .any(|e| matches!(e, LogicEvent::RemoteSnapshot { .. }));
            assert_eq!(opened, opens);
            let expected_seq = if opens { 3 } else { 0 };
            assert_eq!(
                state.documents[&doc].last_seq,
                Seq::<Relay>::new(expected_seq)
            );
            if !opens {
                assert!(
                    events
                        .iter()
                        .any(|e| matches!(e, LogicEvent::SyncWarning { .. }))
                );
            }
        }
        let doc_state = SyncDocumentState::new(
            Id::<Document>::mint(),
            DocumentKey::generate(),
            DocType::Trame,
            crate::Existence::OnRelay,
        );
        assert!(
            doc_state.writers == crate::membership_log::Writers::Unchecked,
            "a document with no anchored log checks no signer"
        );
    }

    /// Holds `SYNC-211`: a newcomer's delta can outrun the word that adds
    /// them. It waits for a listing asked after it arrived, and only that
    /// listing decides it. An answer to an earlier listing leaves it waiting.
    #[test]
    fn a_newcomers_delta_waits_for_the_listing_asked_after_it() {
        use crate::membership_log::tests::{Person, append, created};
        use ambre_protocol::MembershipAct;
        let (mut state, rx, doc, key, _sk, snap) = setup();
        let (alice, bob, erin) = (Person::new(1), Person::new(2), Person::new(5));
        let walk = |log: &[ambre_protocol::MembershipEntry]| {
            crate::membership_log::Writers::of(&crate::membership_log::Log::walk(
                crate::membership_log::tests::DOC,
                log.first().and_then(crate::membership_log::digest),
                log,
                crate::membership_log::no_self_join,
            ))
        };
        let mut log = created(&alice, &bob);
        let before = walk(&log);
        append(&mut log, &alice, &erin, MembershipAct::Invite, Role::Editor);
        let after = walk(&log);
        state.documents.get_mut(&doc).unwrap().writers = before.clone();

        let erins = |clock: u64, payload: &[u8]| {
            make_update(
                doc,
                snap,
                Seq::<DeltaAuthor>::new(clock),
                payload,
                &key,
                &erin.key,
            )
        };
        state.receive_update(Seq::<Relay>::new(1), erins(1, b"first"));
        state.receive_update(Seq::<Relay>::new(2), erins(2, b"second"));
        // The first asked listing 1. The second arrived after it was asked,
        // and waits for its answer before it asks again.
        assert_eq!(
            state.documents[&doc].directory_asked.latest(),
            ambre_core::Seq::nth(1)
        );
        assert_eq!(applied_and_warned(&drain(&rx)), (Vec::new(), Vec::new()));

        state.handle_sync_event(listing(doc, 1, before));
        assert_eq!(
            applied_and_warned(&drain(&rx)),
            (Vec::new(), vec![SyncWarningCode::WriterRefused]),
            "listing 1 settles the first delta alone"
        );
        assert_eq!(
            state.documents[&doc].directory_asked.latest(),
            ambre_core::Seq::nth(2),
            "and asks once more for the second"
        );
        state.handle_sync_event(listing(doc, 2, after));
        assert_eq!(
            applied_and_warned(&drain(&rx)),
            (vec![b"second".to_vec()], Vec::new()),
            "listing 2 names Erin, and her second delta lands"
        );
        assert!(state.documents[&doc].unvouched.is_empty());
    }

    /// Holds `SYNC-230`. A channel opens before the space whose catalogue
    /// lists it. Its log walks under the rule of a document, so Erin, who
    /// joined alone, is no writer and her delta is refused. The space opens
    /// and the Logic thread marks the channel. The log walks again under the
    /// space's rule, and the refused delta lands. A word of the space that
    /// names no writer of the channel's creation grants nobody.
    #[test]
    fn a_channel_opened_before_its_space_walks_its_log_again_under_the_spaces_rule() {
        use crate::membership_log::tests::{DOC, Person, append};
        use crate::membership_log::{Log, SpaceWord, Writers, digest};
        use ambre_protocol::MembershipAct;
        let (mut state, rx, doc, key, _sk, snap) = setup();
        let (alice, erin, mallory) = (Person::new(1), Person::new(5), Person::new(9));
        let mut log = Vec::new();
        append(&mut log, &alice, &alice, MembershipAct::Create, Role::Owner);
        append(
            &mut log,
            &erin,
            &erin,
            MembershipAct::SelfJoin,
            Role::Editor,
        );
        let walk = |state: &SyncState| {
            let rule = state.log_rule(doc);
            let walked = Log::walk_under(DOC, log.first().and_then(digest), &log, &rule);
            Writers::under(&walked, &rule)
        };
        // The segment opens naming its meta, and the channel lists with no
        // space: the space is not open yet.
        let (channel_id, space_id) = (Id::<Channel>::mint(), Id::<Space>::mint());
        let meta = Id::<Meta>::mint();
        state.channel_metas.insert(doc, meta);
        let documents = vec![ambre_bus::ChannelDocument {
            document_id: meta.document_id(),
            kind: ambre_bus::ChannelDocumentKind::Meta,
        }];
        state.handle_command(ambre_bus::SyncCommand::OpenChannel {
            channel_id,
            listed_in: None,
        });
        state.channel_listed(channel_id, None, Ok(documents));
        drain(&rx);
        let before = walk(&state);
        assert!(
            !before.admits(&erin.public()),
            "the self-join counts nowhere"
        );
        state.documents.get_mut(&doc).unwrap().writers = before.clone();

        let erins = make_update(
            doc,
            snap,
            Seq::<DeltaAuthor>::new(1),
            b"erin's message",
            &key,
            &erin.key,
        );
        state.receive_update(Seq::<Relay>::new(1), erins);
        state.handle_sync_event(listing(doc, 1, before));
        assert_eq!(
            applied_and_warned(&drain(&rx)),
            (Vec::new(), vec![SyncWarningCode::WriterRefused])
        );

        // The space opens: Logic marks the channel, and the log walks again.
        // Erin is a present member of the space, under the keys her
        // self-join names (`SYNC-227`).
        let word = SpaceWord {
            writers: HashSet::from([alice.public()]),
            present: std::collections::HashMap::from([(
                erin.id,
                crate::membership_log::MemberKeys {
                    key: erin.public(),
                    exchange: erin.exchange(),
                },
            )]),
            ..SpaceWord::default()
        };
        state.documents.insert(
            space_id.document_id(),
            SyncDocumentState::new(
                space_id.document_id(),
                DocumentKey::generate(),
                DocType::Murmure,
                crate::Existence::OnRelay,
            ),
        );
        state.handle_sync_event(SyncEvent::LogWalked {
            document_id: space_id.document_id(),
            word: word.clone(),
            heads: Default::default(),
        });
        assert_eq!(
            state.documents[&doc].directory_asked.latest(),
            ambre_core::Seq::nth(1),
            "no catalogue lists the channel yet: its log heeds no space"
        );
        state.handle_command(ambre_bus::SyncCommand::MarkChannelListed {
            channel_id,
            space_id,
        });
        assert_eq!(
            state.documents[&doc].directory_asked.latest(),
            ambre_core::Seq::nth(2),
            "the mark lists the members again"
        );
        let after = walk(&state);
        assert!(after.admits(&erin.public()) && after.admits(&alice.public()));
        state.handle_sync_event(listing(doc, 2, after));
        assert_eq!(
            applied_and_warned(&drain(&rx)),
            (vec![b"erin's message".to_vec()], Vec::new()),
            "the second walk lands the refused delta"
        );
        assert!(state.documents[&doc].refused.is_empty());

        // The same mark again walks nothing. A new word of the space walks
        // the channel again, and one no creator of the channel wrote in
        // grants nobody.
        state.handle_command(ambre_bus::SyncCommand::MarkChannelListed {
            channel_id,
            space_id,
        });
        assert_eq!(
            state.documents[&doc].directory_asked.latest(),
            ambre_core::Seq::nth(2)
        );
        state.handle_sync_event(SyncEvent::LogWalked {
            document_id: space_id.document_id(),
            word: SpaceWord {
                writers: HashSet::from([mallory.public()]),
                ..SpaceWord::default()
            },
            heads: Default::default(),
        });
        assert_eq!(
            state.documents[&doc].directory_asked.latest(),
            ambre_core::Seq::nth(3)
        );
        assert_eq!(walk(&state), Writers::Granted(HashSet::new()));
    }

    /// Holds `SYNC-231`, fail closed. Mallory forges the log of a channel
    /// that Alice's space lists. The channel opens before the space, so the
    /// first walk runs under the rule of a document: it grants Mallory, and
    /// her delta lands. Once the space opens, the second walk finds the
    /// creation signed by no writer of the space. The open fails as
    /// `RootOutsideSpace`, and her later deltas land nowhere.
    #[test]
    fn a_channel_log_rooted_outside_its_space_fails_the_open_at_the_second_walk() {
        use crate::membership_log::tests::{DOC, Person, append};
        use crate::membership_log::{Log, Root, SpaceWord, Writers, digest};
        use ambre_protocol::MembershipAct;
        let (mut state, rx, doc, key, _sk, snap) = setup();
        let (alice, mallory) = (Person::new(1), Person::new(9));
        let mut log = Vec::new();
        append(
            &mut log,
            &mallory,
            &mallory,
            MembershipAct::Create,
            Role::Owner,
        );
        let walk = |state: &SyncState| {
            let rule = state.log_rule(doc);
            let walked = Log::walk_under(DOC, log.first().and_then(digest), &log, &rule);
            (Writers::under(&walked, &rule), rule.root_of(&walked))
        };
        let answer = |asked: u64, (writers, root): (Writers, Root)| {
            rooted_listing(doc, asked, writers, root)
        };
        let (channel_id, space_id, meta) = (
            Id::<Channel>::mint(),
            Id::<Space>::mint(),
            Id::<Meta>::mint(),
        );
        state.channel_metas.insert(doc, meta);
        state.handle_command(ambre_bus::SyncCommand::OpenChannel {
            channel_id,
            listed_in: None,
        });
        state.channel_listed(
            channel_id,
            None,
            Ok(vec![ambre_bus::ChannelDocument {
                document_id: meta.document_id(),
                kind: ambre_bus::ChannelDocumentKind::Meta,
            }]),
        );
        drain(&rx);
        let first = walk(&state);
        assert_eq!(
            first.1,
            Root::Free,
            "no space word: the first walk roots freely"
        );
        state.handle_sync_event(answer(1, first));
        let mallorys = |clock: u64, payload: &[u8]| {
            make_update(
                doc,
                snap,
                Seq::<DeltaAuthor>::new(clock),
                payload,
                &key,
                &mallory.key,
            )
        };
        state.receive_update(Seq::<Relay>::new(1), mallorys(1, b"forged"));
        assert_eq!(
            applied_and_warned(&drain(&rx)).0,
            vec![b"forged".to_vec()],
            "the first walk lets the forged delta land"
        );

        // The space opens with Alice its writer, and the channel walks again.
        state.documents.insert(
            space_id.document_id(),
            SyncDocumentState::new(
                space_id.document_id(),
                DocumentKey::generate(),
                DocType::Murmure,
                crate::Existence::OnRelay,
            ),
        );
        state.handle_sync_event(SyncEvent::LogWalked {
            document_id: space_id.document_id(),
            word: SpaceWord {
                writers: HashSet::from([alice.public()]),
                ..SpaceWord::default()
            },
            heads: Default::default(),
        });
        state.handle_command(ambre_bus::SyncCommand::MarkChannelListed {
            channel_id,
            space_id,
        });
        let second = walk(&state);
        assert_eq!(second.1, Root::Stranger);
        state.handle_sync_event(answer(2, second));
        let events = drain(&rx);
        assert!(
            events.iter().any(|e| matches!(
                e,
                LogicEvent::DocumentOpenFailed {
                    document_id,
                    cause: ambre_bus::OpenFailure::RootOutsideSpace,
                } if *document_id == doc
            )),
            "the second walk fails the open"
        );
        assert!(state.documents[&doc].unvouched.is_empty());
        assert!(state.documents[&doc].refused.is_empty());
        state.receive_update(Seq::<Relay>::new(2), mallorys(2, b"later"));
        assert!(
            applied_and_warned(&drain(&rx)).0.is_empty(),
            "nor does a later one"
        );
    }

    /// A channel that `creator` created, and `joiner` joined alone, opened
    /// before its space, as `SYNC-231` sets it up. The first walk runs under
    /// the rule of a document. The space opens with Alice its writer, and
    /// the Logic thread marks the channel listed, so the log walks again.
    fn walking_channel(
        creator: &crate::membership_log::tests::Person,
        joiner: Option<&crate::membership_log::tests::Person>,
    ) -> WalkingChannel {
        use crate::membership_log::tests::{Person, append};
        use crate::membership_log::{Log, SpaceWord, Writers, digest};
        use ambre_protocol::MembershipAct;
        let (mut state, rx, doc, key, sk, snap) = setup();
        state.signing_key = Some(sk);
        let (ws_tx, ws_rx) = ambre_bus::conduit::unbounded();
        state.set_mock_ws(ws_tx);
        let alice = Person::new(1);
        let mut log = Vec::new();
        append(
            &mut log,
            creator,
            creator,
            MembershipAct::Create,
            Role::Owner,
        );
        if let Some(joiner) = joiner {
            append(
                &mut log,
                joiner,
                joiner,
                MembershipAct::SelfJoin,
                Role::Editor,
            );
        }
        let (channel_id, space_id, meta) = (
            Id::<Channel>::mint(),
            Id::<Space>::mint(),
            Id::<Meta>::mint(),
        );
        state.channel_metas.insert(doc, meta);
        state.handle_command(ambre_bus::SyncCommand::OpenChannel {
            channel_id,
            listed_in: None,
        });
        state.channel_listed(
            channel_id,
            None,
            Ok(vec![ambre_bus::ChannelDocument {
                document_id: meta.document_id(),
                kind: ambre_bus::ChannelDocumentKind::Meta,
            }]),
        );
        // The open lists the members, as the first listing.
        state.handle_list_document_members(None, doc);
        let rule = state.log_rule(doc);
        let walked = Log::walk_under(
            crate::membership_log::tests::DOC,
            log.first().and_then(digest),
            &log,
            &rule,
        );
        state.handle_sync_event(rooted_listing(
            doc,
            1,
            Writers::under(&walked, &rule),
            rule.root_of(&walked),
        ));
        state.documents.insert(
            space_id.document_id(),
            SyncDocumentState::new(
                space_id.document_id(),
                DocumentKey::generate(),
                DocType::Murmure,
                crate::Existence::OnRelay,
            ),
        );
        state.handle_sync_event(SyncEvent::LogWalked {
            document_id: space_id.document_id(),
            word: SpaceWord {
                writers: HashSet::from([alice.public()]),
                ..SpaceWord::default()
            },
            heads: Default::default(),
        });
        state.handle_command(ambre_bus::SyncCommand::MarkChannelListed {
            channel_id,
            space_id,
        });
        drain(&rx);
        drain_ws(&ws_rx);
        WalkingChannel {
            state,
            rx,
            ws_rx,
            doc,
            key,
            snap,
            log,
        }
    }

    /// What [`walking_channel`] hands a test.
    struct WalkingChannel {
        state: SyncState,
        rx: ambre_bus::conduit::Receiver<LogicEvent>,
        ws_rx: ambre_bus::conduit::Receiver<Payload<ambre_protocol::WireFrame>>,
        doc: Id<Document>,
        key: DocumentKey,
        snap: Id<Snapshot>,
        log: Vec<ambre_protocol::MembershipEntry>,
    }

    /// The updates `ws_rx` carried to the relay, by their sealed bytes.
    fn sealed_updates(
        ws_rx: &ambre_bus::conduit::Receiver<Payload<ambre_protocol::WireFrame>>,
    ) -> Vec<Update> {
        drain_ws(ws_rx)
            .into_iter()
            .filter_map(|m| match m {
                WsClientMsg::Update(u) => Some(u),
                _ => None,
            })
            .collect()
    }

    /// Holds `SYNC-232`, fail closed. Between the mark and the answer of the
    /// walk, Mallory's delta lands nowhere, and the user's edit, snapshot
    /// and presence seal nothing under her key. The walk finds the log
    /// rooted outside the space: the held delta and the held edit are gone,
    /// and a later edit seals nothing either.
    #[test]
    fn a_channel_log_walking_again_lands_and_seals_nothing_until_its_answer() {
        use crate::membership_log::tests::Person;
        use crate::membership_log::{Log, Writers, digest};
        let mallory = Person::new(9);
        let WalkingChannel {
            mut state,
            rx,
            ws_rx,
            doc,
            key,
            snap,
            log,
        } = walking_channel(&mallory, None);
        assert_eq!(
            state.documents[&doc].directory_asked.latest(),
            ambre_core::Seq::nth(2),
            "the mark walks the log again"
        );
        state.receive_update(
            Seq::<Relay>::new(1),
            make_update(
                doc,
                snap,
                Seq::<DeltaAuthor>::new(1),
                b"forged",
                &key,
                &mallory.key,
            ),
        );
        assert!(
            applied_and_warned(&drain(&rx)).0.is_empty(),
            "a writer of the first walk lands nothing while the log walks"
        );
        state.handle_command(SyncCommand::SendUpdate {
            document_id: doc,
            payload: b"secret".to_vec().into(),
            summary: None,
        });
        state.handle_command(SyncCommand::SendEphemeral {
            document_id: doc,
            payload: b"cursor".to_vec().into(),
        });
        assert!(
            drain_ws(&ws_rx).is_empty(),
            "no frame is sealed under the key the walk may disown"
        );
        assert!(matches!(
            state.snapshot_upload(doc, &b"fold".to_vec().into(), Seq::<Relay>::new(1)),
            Err(crate::error::SyncError::LogUnsettled)
        ));

        // An answer to the first listing, late, decides nothing.
        state.handle_sync_event(listing(doc, 1, Writers::Granted(HashSet::new())));
        assert!(applied_and_warned(&drain(&rx)).0.is_empty());

        let rule = state.log_rule(doc);
        let walked = Log::walk_under(
            crate::membership_log::tests::DOC,
            log.first().and_then(digest),
            &log,
            &rule,
        );
        state.handle_sync_event(rooted_listing(
            doc,
            2,
            Writers::under(&walked, &rule),
            rule.root_of(&walked),
        ));
        let events = drain(&rx);
        assert!(events.iter().any(|e| matches!(
            e,
            LogicEvent::DocumentOpenFailed {
                cause: ambre_bus::OpenFailure::RootOutsideSpace,
                ..
            }
        )));
        assert!(applied_and_warned(&events).0.is_empty());
        assert!(drain_ws(&ws_rx).is_empty(), "the held edit is wiped");
        state.handle_command(SyncCommand::SendUpdate {
            document_id: doc,
            payload: b"later".to_vec().into(),
            summary: None,
        });
        assert!(drain_ws(&ws_rx).is_empty(), "nor is a later edit sealed");
    }

    /// Holds `SYNC-232` when the walk confirms the log. Alice created the
    /// channel and Erin joined alone. The delta Alice sends and the edit of
    /// this user wait for the walk. Its answer roots the log in the space:
    /// the delta lands and the edit leaves, sealed, in the order written.
    #[test]
    fn a_channel_log_walked_again_and_confirmed_lands_and_seals_what_it_held() {
        use crate::membership_log::tests::Person;
        use crate::membership_log::{Log, Root, Writers, digest};
        let (alice, erin) = (Person::new(1), Person::new(5));
        let WalkingChannel {
            mut state,
            rx,
            ws_rx,
            doc,
            key,
            snap,
            log,
        } = walking_channel(&alice, Some(&erin));
        state.receive_update(
            Seq::<Relay>::new(1),
            make_update(
                doc,
                snap,
                Seq::<DeltaAuthor>::new(1),
                b"alice's",
                &key,
                &alice.key,
            ),
        );
        for payload in [b"first".to_vec(), b"second".to_vec()] {
            state.handle_command(SyncCommand::SendUpdate {
                document_id: doc,
                payload: payload.into(),
                summary: None,
            });
        }
        assert!(applied_and_warned(&drain(&rx)).0.is_empty());
        assert!(sealed_updates(&ws_rx).is_empty());

        let rule = state.log_rule(doc);
        let walked = Log::walk_under(
            crate::membership_log::tests::DOC,
            log.first().and_then(digest),
            &log,
            &rule,
        );
        assert_eq!(rule.root_of(&walked), Root::SpaceWriter);
        state.handle_sync_event(rooted_listing(
            doc,
            2,
            Writers::under(&walked, &rule),
            Root::SpaceWriter,
        ));
        assert_eq!(
            applied_and_warned(&drain(&rx)),
            (vec![b"alice's".to_vec()], Vec::new()),
            "the held delta lands"
        );
        let sent = sealed_updates(&ws_rx);
        let opened: Vec<Vec<u8>> = sent
            .iter()
            .map(|u| {
                let body = pipeline::open_update(u, &key).unwrap();
                pipeline::split_update_body(body)
                    .unwrap()
                    .delta
                    .into_bytes()
            })
            .collect();
        assert_eq!(opened, [b"first".to_vec(), b"second".to_vec()]);
        assert!(matches!(
            state.documents[&doc].gate,
            crate::channels::LogGate::Open
        ));
    }

    /// The walk of the log of `doc` under the rule of its space, answered as
    /// the listing `2`.
    fn answer_walk(
        state: &mut SyncState,
        doc: Id<Document>,
        log: &[ambre_protocol::MembershipEntry],
    ) {
        use crate::membership_log::{Log, Writers, digest};
        let rule = state.log_rule(doc);
        let walked = Log::walk_under(
            crate::membership_log::tests::DOC,
            log.first().and_then(digest),
            log,
            &rule,
        );
        state.handle_sync_event(rooted_listing(
            doc,
            2,
            Writers::under(&walked, &rule),
            rule.root_of(&walked),
        ));
    }

    /// A refused frame, as the sent log hands it back to seal again.
    fn refused_frame(payload: &[u8]) -> UpdateFrame {
        pipeline::update_body(&TEST_NONCE, None, &payload.to_vec().into()).unwrap()
    }

    /// Holds `SYNC-234` on the resend path, fail closed. A frame the relay
    /// refused, handed back to seal again while the log walks, seals
    /// nothing. A walk that roots the log in the space seals it at its own
    /// clock. One that finds the log rooted outside never seals it.
    #[test]
    fn a_refused_frame_waits_for_the_walk_and_leaves_at_its_own_clock() {
        use crate::membership_log::tests::Person;
        let (alice, erin, mallory) = (Person::new(1), Person::new(5), Person::new(9));
        let clock = Seq::<DeltaAuthor>::new(7);

        let WalkingChannel {
            mut state,
            ws_rx,
            doc,
            key,
            log,
            ..
        } = walking_channel(&alice, Some(&erin));
        state.resend_refused(doc, vec![(clock, refused_frame(b"refused"))], Vec::new());
        assert!(
            sealed_updates(&ws_rx).is_empty(),
            "no frame is sealed while the log walks"
        );
        answer_walk(&mut state, doc, &log);
        let sent = sealed_updates(&ws_rx);
        let [update] = sent.as_slice() else {
            panic!("one frame leaves, got {}", sent.len());
        };
        assert_eq!(update.public_data.clock, clock, "at its own clock");
        let body = pipeline::open_update(update, &key).unwrap();
        assert_eq!(
            pipeline::split_update_body(body)
                .unwrap()
                .delta
                .into_bytes(),
            b"refused"
        );

        let WalkingChannel {
            mut state,
            ws_rx,
            doc,
            log,
            ..
        } = walking_channel(&mallory, None);
        state.resend_refused(doc, vec![(clock, refused_frame(b"refused"))], Vec::new());
        answer_walk(&mut state, doc, &log);
        state.resend_refused(doc, vec![(clock, refused_frame(b"again"))], Vec::new());
        assert!(
            sealed_updates(&ws_rx).is_empty(),
            "a log rooted outside seals nothing"
        );
    }

    /// Holds `SYNC-234` on the bound of the walk, fail closed. Past
    /// [`QUARANTINE_LIMIT`] held edits, the walk drops every edit it held,
    /// the user learns it, and no edit leaves even once the walk answers.
    #[test]
    fn a_walk_past_its_bound_drops_its_edits_aloud_and_sends_none() {
        use crate::membership_log::tests::Person;
        let (alice, erin) = (Person::new(1), Person::new(5));
        let WalkingChannel {
            mut state,
            rx,
            ws_rx,
            doc,
            log,
            ..
        } = walking_channel(&alice, Some(&erin));
        let edit = |state: &mut SyncState| {
            state.handle_command(SyncCommand::SendUpdate {
                document_id: doc,
                payload: b"edit".to_vec().into(),
                summary: None,
            });
        };
        for _ in 0..QUARANTINE_LIMIT {
            edit(&mut state);
        }
        assert!(
            applied_and_warned(&drain(&rx)).1.is_empty(),
            "the bound holds every edit"
        );
        edit(&mut state);
        assert_eq!(
            applied_and_warned(&drain(&rx)).1,
            [SyncWarningCode::EditRefused],
            "the drop is said aloud"
        );
        answer_walk(&mut state, doc, &log);
        edit(&mut state);
        assert!(
            sealed_updates(&ws_rx).is_empty(),
            "no edit builds on one the relay lacks"
        );
    }

    /// Holds `SYNC-234` on the adoption path, fail closed. A writer of the
    /// first walk signed a snapshot, fetched while the log walks. It reaches
    /// no check of the Logic thread and is no known parent. A walk that finds
    /// the log rooted outside adopts it never.
    #[test]
    fn a_snapshot_fetched_while_the_log_walks_is_not_adopted() {
        use crate::membership_log::tests::Person;
        let mallory = Person::new(9);
        let WalkingChannel {
            mut state,
            rx,
            doc,
            key,
            log,
            ..
        } = walking_channel(&mallory, None);
        let forged = Id::<Snapshot>::mint();
        let blob = snapshot_blob(doc, served(forged, 3), b"forged", &key, &mallory.key);
        state.adopt_fetched_snapshot(doc, forged, Seq::<Relay>::new(3), Some(blob.clone()));
        let events = drain(&rx);
        assert!(
            !events
                .iter()
                .any(|e| matches!(e, LogicEvent::PendingSnapshot { .. })),
            "the snapshot reaches no check"
        );
        assert!(
            applied_and_warned(&events).1.is_empty(),
            "nor is it reported broken"
        );
        assert!(!state.documents[&doc].known_snapshots.contains(&forged));

        answer_walk(&mut state, doc, &log);
        drain(&rx);
        state.adopt_fetched_snapshot(doc, forged, Seq::<Relay>::new(3), Some(blob));
        assert!(
            !drain(&rx)
                .iter()
                .any(|e| matches!(e, LogicEvent::PendingSnapshot { .. }))
        );
        assert!(!state.documents[&doc].known_snapshots.contains(&forged));
    }
}
