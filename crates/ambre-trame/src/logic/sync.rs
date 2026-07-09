// ambre-trame/src/logic/sync.rs.rs
// CRDT delta flush, remote apply, snapshot import/export
//
// Part of the `TrameState` impl, split out of the original logic.rs by concern.
// Submodule of `logic`, so it reaches the private fields/helpers of the parent
// module through the `use super::*` glob.

use super::*;

impl TrameState {
    /// Export any new CRDT ops since `last_exported_vv` and bump the tracking
    /// cursor. Returns `Some` binary delta if there were new ops.
    pub(super) fn flush_delta(&mut self) -> Option<Vec<u8>> {
        let current = self.doc.oplog_vv();
        if current == self.last_exported_vv {
            return None;
        }
        // Export the incremental ops. On failure we must NOT advance the export
        // cursor: swallowing the error (the old `.ok()`) dropped the local delta
        // for good, and since remote apply is now live a lost local delta means
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
        tracing::info!("[Logic] flush_delta → {} bytes of CRDT ops", delta.len());
        Some(delta)
    }

    /// Apply a remote CRDT delta and return display events.
    ///
    /// `payload` is the raw (decrypted) Loro binary update forwarded by the Sync
    /// thread (a peer's edit, or our own history replayed on open). Loro's
    /// `import` ingests both incremental updates and full snapshots, so the same
    /// reseed path as [`import_snapshot`](Self::import_snapshot) applies: merge
    /// the ops, reseed the formula engine, and tell the UI to re-fetch.
    pub fn apply_remote(&mut self, payload: &[u8]) -> Vec<TrameEvent> {
        self.import_and_reseed(payload, "apply_remote")
    }

    /// Apply a remote ephemeral (presence) payload and return presence events.
    ///
    /// Stubbed until collaborator presence is wired.
    pub fn apply_ephemeral(&mut self, _payload: &[u8]) -> Vec<TrameEvent> {
        vec![]
    }

    /// Export the full CRDT state for snapshot compaction. A failed export is
    /// logged (not silently swallowed): the caller persists the returned bytes to
    /// IndexedDB, so an empty snapshot from a dropped error would silently wipe the
    /// stored document on the next compaction.
    pub fn export_snapshot(&self) -> Vec<u8> {
        self.doc.export(ExportMode::Snapshot).unwrap_or_else(|e| {
            tracing::error!("[Logic] export_snapshot failed, returning empty snapshot: {e:?}");
            Vec::new()
        })
    }

    /// Import a full CRDT snapshot (local restore from IndexedDB at startup).
    ///
    /// Imports the persisted Loro state into the live document, advances the
    /// export cursor so the next edit still produces a correct incremental
    /// delta, and reseeds the formula engine from the restored cells (so formula
    /// cells evaluate instead of showing their literal source). Returns no
    /// display events: the next viewport request re-renders the restored grid.
    pub fn import_snapshot(&mut self, payload: &[u8]) -> Vec<TrameEvent> {
        self.import_and_reseed(payload, "import_snapshot")
    }

    /// Resolve a sheet name to its stable sheet id (e.g. `"Sheet1"` -> `"s0"`).
    /// Groundwork for cross-sheet formula references (`Sheet2!A1`). Linear scan
    /// over the `sheets` map -- there are only a handful of sheets.
    pub fn sheet_name_to_id(&self, name: &str) -> Option<String> {
        match self.doc.get_map(SHEETS_MAP).get_value() {
            LoroValue::Map(sheets) => sheets.iter().find_map(|(id, value)| {
                matches!(value, LoroValue::String(n) if n.as_str() == name).then(|| id.to_string())
            }),
            _ => None,
        }
    }

    /// Shared import path for both a local snapshot restore and a remote delta:
    /// merge the Loro bytes, advance the export cursor past the imported ops (so
    /// they are not re-broadcast), reseed the formula engine from the resulting
    /// cell values, and emit a `DataReset` so the UI re-fetches its viewport
    /// (the original request may predate this import and show an empty grid).
    pub(super) fn import_and_reseed(&mut self, payload: &[u8], origin: &str) -> Vec<TrameEvent> {
        if !self.merge_payload(payload, origin) {
            return Vec::new();
        }
        self.reseed_after_merge(origin)
    }

    /// Apply a batch of decrypted remote deltas as a SINGLE merge+reseed, so the
    /// UI sees one `DataReset` instead of one per delta. A server history replay
    /// on (re)open arrives as N sequenced updates; delivering them one by one
    /// re-fetches and repaints the grid N times, and a cell that only appears in
    /// the last delta visibly flickers in from blank. Merging the whole batch
    /// first, then reseeding once, renders the final state in a single repaint.
    /// A no-op (no display events) if nothing imported.
    pub fn apply_remote_batch(&mut self, payloads: &[Vec<u8>]) -> Vec<TrameEvent> {
        let mut merged_any = false;
        for payload in payloads {
            merged_any |= self.merge_payload(payload, "apply_remote_batch");
        }
        if !merged_any {
            return Vec::new();
        }
        self.reseed_after_merge("apply_remote_batch")
    }

    /// Merge one decrypted Loro payload into the live document, returning whether
    /// it actually imported (non-empty and accepted). Does NOT reseed: callers
    /// batch the reseed over one or many merges via [`reseed_after_merge`].
    fn merge_payload(&mut self, payload: &[u8], origin: &str) -> bool {
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
    /// [`apply_remote_batch`] / quarantine-release paths.
    fn reseed_after_merge(&mut self, origin: &str) -> Vec<TrameEvent> {
        // Rebuild the position <-> id cache first: the merged doc may carry rows
        // or columns this cache does not yet know about, and both the engine
        // reseed and the formatting reseed below resolve cell keys through it.
        self.rebuild_index();
        let cells_map = self.doc.get_map(cells_map_name(&self.active_sheet));
        let cell_count = match cells_map.get_value() {
            LoroValue::Map(cells) => cells.len(),
            _ => 0,
        };
        tracing::info!("[Logic] {origin} merged, {cell_count} cell(s) now present");
        // Advance the cursor to include the imported ops: we did not author them
        // (or already persisted them), so the next `flush_delta` must not export
        // them back. BUT preserve our own peer's cursor: a freshly constructed
        // doc (every reopen mints a new random PeerID) commits local genesis ops
        // in `new()` that have never been broadcast, and the first import on a
        // reopen runs before any flush. Swallowing those here would make a later
        // edit's delta causally depend on ops no other peer has -- a dangling
        // dependency that silently drops the edit on the next reopen. Keeping our
        // prior cursor re-exports our own un-broadcast ops with the next flush;
        // for already-flushed peers it is a no-op (the cursor already matches).
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
        // Adopt the (possibly remote) hidden-row set directly: a peer's filter
        // arrives as `filter_hidden_{sheet}` ops and is applied without recompute
        // (§1.A -- only the filter author recomputes).
        self.rebuild_filter();
        // Refresh the toolbar / context-menu toggles for the active cell: after a
        // reload its formatting is only known once the `format` map is restored.
        let mut events = vec![
            TrameEvent::DataReset,
            self.sheet_list_event(),
            self.style_state_event(),
        ];
        events.extend(conflicts);
        events
    }
}
