// ambre-trame/src/logic/sync.rs.rs
// CRDT delta flush, remote apply, snapshot import/export
//
// Part of the `TrameState` impl, split out of the original logic.rs by concern.
// Submodule of `logic`, so it reaches the private fields/helpers of the parent
// module through the `use super::*` glob.

use ambre_core::LoroPayloadRef;
use ambre_core::schema::SchemaError;

use super::*;

/// What a merge came in as. The log lines name it, so a failed import says
/// which path drew it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum MergeOrigin {
    /// One remote delta.
    Delta,
    /// A remote snapshot.
    Snapshot,
    /// A batch of remote deltas.
    Batch,
    /// A snapshot imported from storage.
    Import,
    /// The plaintext of an `.ambre` file.
    Plaintext,
}

impl std::fmt::Display for MergeOrigin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Delta => "apply_remote",
            Self::Snapshot => "apply_remote_snapshot",
            Self::Batch => "apply_remote_batch",
            Self::Import => "import_snapshot",
            Self::Plaintext => "import_ambre_plaintext",
        })
    }
}

/// Write the registry entry of `peer` under its own origin, which the undo
/// manager excludes (`AUTHOR-009`). Answers the PeerID's counter past the
/// entry, or `None` when the engine refused the write.
fn write_entry(
    doc: &LoroDoc,
    peer: ambre_core::PeerId,
    entry: &ambre_core::PeerEntry,
) -> Option<ambre_core::OpCounter> {
    if let Err(e) = doc
        .get_map(ambre_core::crdt::FoundationRoot::Peers.as_str())
        .insert(ambre_core::PeerKey::of(peer).as_str(), entry.to_bytes())
    {
        tracing::error!("[Logic] the registry entry was refused: {e:?}");
        return None;
    }
    doc.commit_with(
        loro::CommitOptions::new().origin(ambre_core::crdt::CommitOrigin::Peers.as_str()),
    );
    doc.oplog_vv()
        .get(&peer.0)
        .copied()
        .map(ambre_core::OpCounter)
}

impl TrameState {
    /// Export any new CRDT ops since `last_exported_vv` and bump the tracking
    /// cursor. Returns `Some` binary delta if there were new ops.
    pub(super) fn flush_delta(&mut self) -> Option<ambre_core::DeltaBytes> {
        let current = self.doc.oplog_vv();
        if current == self.last_exported_vv {
            return None;
        }
        // The registry entry alone is no edit: it waits for the first one and
        // travels with it (`AUTHOR-009`).
        if let Some(end) = self.entry_end {
            if current
                .get(&self.doc.peer_id())
                .copied()
                .map(ambre_core::OpCounter)
                == Some(end)
            {
                return None;
            }
            self.entry_end = None;
        }
        // A local edit on the default sheet the workbook only reads stores it
        // (`TRAME-077`), in the same delta.
        if self.hold_default_sheet() {
            self.doc.commit();
        }
        let current = self.doc.oplog_vv();
        // Export the incremental ops. On failure we must NOT advance the export
        // cursor. Swallowing the error (the old `.ok()`) dropped the local delta
        // for good. Since remote apply is now live, a lost local delta means
        // permanent divergence, not just a skipped frame. Leaving the cursor put
        // means the next flush re-attempts the same range.
        let delta = match self.doc.export(ExportMode::updates(&self.last_exported_vv)) {
            Ok(delta) => delta,
            Err(e) => {
                tracing::error!("[Logic] flush_delta export failed, delta retained: {e:?}");
                return None;
            }
        };
        self.last_exported_vv = current;
        tracing::info!("[Logic] flush_delta -> {} bytes of CRDT ops", delta.len());
        Some(ambre_core::DeltaBytes::new(delta))
    }

    /// Write under the session identity Sync issued for this workbook
    /// (`AUTHOR-002`), and write its registry entry first (`AUTHOR-009`).
    ///
    /// The export cursor moves past everything the workbook held before. Those
    /// operations are not this session's to send (`AUTHOR-008`). The entry is
    /// written now, before any edit opens an undo group it would split. `false`
    /// when the CRDT engine refused the identity, and then nothing changed.
    pub fn adopt_session(
        &mut self,
        peer: ambre_core::PeerId,
        entry: ambre_core::PeerEntry,
    ) -> bool {
        self.doc.commit();
        if let Err(e) = self.doc.set_peer_id(peer.0) {
            tracing::error!("[Logic] the session identity was refused: {e:?}");
            return false;
        }
        self.last_exported_vv = self.doc.oplog_vv();
        self.entry_end = write_entry(&self.doc, peer, &entry);
        true
    }

    /// Apply a remote CRDT delta and return display events.
    ///
    /// `payload` is the raw (decrypted) Loro binary update forwarded by the Sync
    /// thread (a peer's edit, or our own history replayed on open). Loro's
    /// `import` ingests both incremental updates and full snapshots, so the same
    /// reseed path as [`import_snapshot`](Self::import_snapshot) applies. That
    /// path merges the ops, reseeds the formula engine, and tells the UI to
    /// re-fetch.
    pub fn apply_remote(&mut self, payload: &ambre_core::DeltaBytes) -> Vec<TrameEvent> {
        self.import_and_reseed(LoroPayloadRef::Delta(payload), MergeOrigin::Delta)
    }

    /// Apply a snapshot whose contents are trusted the way a remote delta is
    /// applied. Unlike [`import_snapshot`](Self::import_snapshot), it runs no
    /// schema gate.
    pub fn apply_remote_snapshot(
        &mut self,
        payload: &ambre_core::SnapshotBytes,
    ) -> Vec<TrameEvent> {
        self.import_and_reseed(LoroPayloadRef::Snapshot(payload), MergeOrigin::Snapshot)
    }

    /// Apply a remote ephemeral (presence) payload and return presence events.
    ///
    /// Stubbed until collaborator presence is wired.
    pub fn apply_ephemeral(&mut self, _payload: &ambre_core::EphemeralBytes) -> Vec<TrameEvent> {
        vec![]
    }

    /// Export the full CRDT state for snapshot compaction. A failed export is
    /// logged, not silently swallowed. The caller persists the returned bytes to
    /// IndexedDB. An empty snapshot from a dropped error would silently wipe the
    /// stored document on the next compaction.
    /// The CRDT document, read-only. A snapshot check compares its history
    /// with a peer's snapshot and never writes (`AUTHOR-018`).
    #[must_use]
    pub fn crdt(&self) -> &loro::LoroDoc {
        &self.doc
    }

    pub fn export_snapshot(&self) -> ambre_core::SnapshotBytes {
        ambre_core::SnapshotBytes::new(self.doc.export(ExportMode::Snapshot).unwrap_or_else(|e| {
            tracing::error!("[Logic] export_snapshot failed, returning empty snapshot: {e:?}");
            Vec::new()
        }))
    }

    /// Import a full CRDT snapshot (local restore from IndexedDB at startup).
    ///
    /// Imports the persisted Loro state into the live document, and advances the
    /// export cursor so the next edit still produces a correct incremental
    /// delta. It also reseeds the formula engine from the restored cells, so
    /// formula cells evaluate instead of showing their literal source. Returns
    /// no display events: the next viewport request re-renders the restored
    /// grid.
    ///
    /// # Errors
    /// Every failure of the schema gate: a workbook written by a newer client
    /// (`SCHEMA-008`), or a migration the registry cannot run. Bytes the CRDT
    /// engine refuses fail it too. On any of them the workbook is untouched
    /// (`SCHEMA-012`). The gate reads the payload in a scratch document
    /// precisely so a refusal costs the caller nothing.
    ///
    /// The remote-delta path ([`apply_remote`](Self::apply_remote)) carries no
    /// version and applies blind: a delta is not a full-state read, and there is
    /// nothing in it to compare.
    pub fn import_snapshot(
        &mut self,
        payload: &ambre_core::SnapshotBytes,
    ) -> Result<Vec<TrameEvent>, SchemaError> {
        // Before a single container is used (`SCHEMA-006`). What comes back is
        // the payload itself at the current version, or a migrated snapshot.
        // A gate that keeps the bytes hands back the payload itself.
        let migrated = match schema::gate(payload.as_bytes())? {
            std::borrow::Cow::Borrowed(_) => None,
            std::borrow::Cow::Owned(bytes) => Some(ambre_core::SnapshotBytes::new(bytes)),
        };
        let snapshot = migrated.as_ref().unwrap_or(payload);
        Ok(self.import_and_reseed(LoroPayloadRef::Snapshot(snapshot), MergeOrigin::Import))
    }

    /// Resolve a sheet name to its stable sheet id (for example `"Sheet1"` -> `"s0"`).
    /// Groundwork for cross-sheet formula references (`Sheet2!A1`). Linear scan
    /// over the `sheets` map -- there are only a handful of sheets.
    pub fn sheet_name_to_id(&self, name: &SheetName) -> Option<SheetId> {
        let key = name.key();
        self.sheet_names()
            .into_iter()
            .find_map(|(id, n)| (n.key() == key).then_some(id))
    }

    /// Shared import path for both a local snapshot restore and a remote delta.
    /// It merges the Loro bytes and advances the export cursor past the imported
    /// ops, so they are not re-broadcast. It then reseeds the formula engine
    /// from the resulting cell values and emits a `DataReset` so the UI
    /// re-fetches its viewport. The original request may predate this import and
    /// show an empty grid.
    pub(super) fn import_and_reseed(
        &mut self,
        payload: LoroPayloadRef<'_>,
        origin: MergeOrigin,
    ) -> Vec<TrameEvent> {
        let order = self.sheet_order_ids();
        if !self.merge_payload(payload, origin) {
            return Vec::new();
        }
        self.reseed_after_merge(origin, &order)
    }

    /// Apply a batch of decrypted remote deltas as a SINGLE merge+reseed, so the
    /// UI sees one `DataReset` instead of one per delta. A server history replay
    /// on (re)open arrives as N sequenced updates. Delivering them one by one
    /// re-fetches and repaints the grid N times. A cell that only appears in the
    /// last delta visibly flickers in from blank. Merging the whole batch first,
    /// then reseeding once, renders the final state in a single repaint. A no-op
    /// (no display events) if nothing imported.
    pub fn apply_remote_batch(&mut self, payloads: &[ambre_core::DeltaBytes]) -> Vec<TrameEvent> {
        let order = self.sheet_order_ids();
        let mut merged_any = false;
        for payload in payloads {
            merged_any |= self.merge_payload(LoroPayloadRef::Delta(payload), MergeOrigin::Batch);
        }
        if !merged_any {
            return Vec::new();
        }
        self.reseed_after_merge(MergeOrigin::Batch, &order)
    }

    /// Merge one decrypted Loro payload into the live document, returning whether
    /// it actually imported (non-empty and accepted). Does NOT reseed: callers
    /// batch the reseed over one or many merges via [`reseed_after_merge`].
    fn merge_payload(&mut self, payload: LoroPayloadRef<'_>, origin: MergeOrigin) -> bool {
        let payload = payload.as_bytes();
        if payload.is_empty() {
            return false;
        }
        if let Err(e) = self.doc.import(payload) {
            tracing::warn!("[Logic] {origin} import failed: {e:?}");
            return false;
        }
        true
    }

    /// Rebuild every derived cache from the merged document and emit the
    /// `DataReset` (+ sheet list, style state, conflicts) the UI needs to repaint.
    /// Shared tail of the single-delta [`import_and_reseed`] and the batched
    /// [`apply_remote_batch`] / quarantine-release paths. `order` is the tab
    /// order before the merge, which a deleted active sheet falls back from.
    pub(super) fn reseed_after_merge(
        &mut self,
        origin: MergeOrigin,
        order: &[SheetId],
    ) -> Vec<TrameEvent> {
        // A peer may have deleted the sheet this reader is on (`TRAME-075`).
        // Leave it before any cache is rebuilt from it.
        self.leave_a_deleted_sheet(order);
        // Rebuild the position <-> id cache first: the merged doc may carry rows
        // or columns this cache does not yet know about. Both the engine reseed
        // and the formatting reseed below resolve cell keys through it.
        self.rebuild_index();
        let cells_map = self
            .doc
            .get_map(SheetContainer::of(SheetRoot::Cells, &self.active_sheet).as_str());
        let cell_count = match cells_map.get_value() {
            LoroValue::Map(cells) => cells.len(),
            _ => 0,
        };
        tracing::info!("[Logic] {origin} merged, {cell_count} cell(s) now present");
        // Advance the cursor to include the imported ops: we did not author
        // them, or already persisted them. The next `flush_delta` must not
        // export them back. BUT preserve our own peer's cursor.
        //
        // A freshly constructed doc (every reopen mints a new random PeerID)
        // commits local genesis ops in `new()` that have never been broadcast.
        // The first import on a reopen runs before any flush. Swallowing those
        // here would make a later edit's delta causally depend on ops no other
        // peer has. That is a dangling dependency that silently drops the edit
        // on the next reopen. Keeping our prior cursor re-exports our own
        // un-broadcast ops with the next flush; for already-flushed peers it is
        // a no-op (the cursor already matches).
        let our_peer = self.doc.peer_id();
        let prior_ours = self.last_exported_vv.get(&our_peer).copied();
        self.last_exported_vv = self.doc.oplog_vv();
        match prior_ours {
            Some(counter) => {
                self.last_exported_vv.insert(our_peer, counter);
            }
            None => {
                self.last_exported_vv.remove(&our_peer);
            }
        }
        // Rebuild the merge cache (and re-evaluate masking) before the engine
        // reseed so masked cells are excluded; conflict events ride along below.
        let conflicts = self.rebuild_merges();
        // Reseed the formula engine from the resulting cell values, mapping each
        // stable key pair back to its grid position via the rebuilt index. A
        // merge may have moved cells under their stable keys, so reset first.
        self.reseed_engine();
        // Rebuild bold/italic + rich styles from the merged `cellstyle_{sheet}` map.
        self.reseed_styles();
        // Rebuild the column-width / row-height caches from the merged metric maps.
        self.reseed_metrics();
        // Adopt the (possibly remote) hidden-row set directly. A peer's filter
        // arrives as `filter_hidden_{sheet}` ops and is applied without recompute
        // (`trame-shared-filter` -- only the filter author recomputes).
        self.rebuild_filter();
        // Refresh the toolbar / context-menu toggles for the active cell: after a
        // reload its formatting is only known once the `format` map is restored.
        let mut events = vec![
            TrameEvent::DataReset,
            self.sheet_list_event(),
            self.style_state_event(),
        ];
        events.extend(conflicts);
        // The open session's cell, moved or deleted by the merge (`TRAME-UI-176`).
        events.extend(self.follow_edit_anchor());
        // A register the reseed could not decode is reported here rather than at
        // each import site. This is the one body every full-state read and every
        // remote delta passes through (`SCHEMA-003`).
        events.extend(self.undecodable_notice());
        events
    }

    /// The schema version this workbook carries (`SCHEMA-004`), for a report rather
    /// than for a decision. The fixture manifest records the version its artefact
    /// was written at.
    ///
    /// A register this build cannot read comes back as
    /// [`SCHEMA_VERSION_ABSENT`](ambre_core::schema::SCHEMA_VERSION_ABSENT) here.
    /// Every path that *decides* anything goes through `schema::gate`, which
    /// refuses it instead (`SCHEMA-013`). This one is called on a document the
    /// caller just generated, where the case cannot arise.
    #[must_use]
    pub fn schema_version(&self) -> ambre_core::schema::SchemaVersion {
        schema::read_version(&self.doc).unwrap_or(ambre_core::schema::SCHEMA_VERSION_ABSENT)
    }

    /// Record that a register could not be decoded (`SCHEMA-003`).
    ///
    /// An **empty** register is not a failure: a cell with no formatting carries
    /// no bytes. Only a non-empty blob the decoder refuses is a record this
    /// build cannot read.
    pub(super) fn note_undecodable(&self, bytes: &[u8]) {
        if !bytes.is_empty() {
            self.undecodable.set(true);
        }
    }

    /// Drain the flag into the one event that carries it to the user, or nothing
    /// when every register read cleanly (`SCHEMA-003`).
    ///
    /// Drained rather than read so the notice is raised once per import instead
    /// of once per reseed.
    fn undecodable_notice(&mut self) -> Option<TrameEvent> {
        self.undecodable
            .replace(false)
            .then_some(TrameEvent::UndecodableRecords)
    }
}
