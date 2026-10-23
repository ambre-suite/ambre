//! Atomic foreign-file import into [`ToileState`](super::ToileState)
//! (`toile-import-export`). An [`ImportBatch`](crate::logic::interop::ImportBatch)
//! produced by a pure converter is applied here as ONE undo group. The pass
//! order resolves every slot / group reference before it is used. It is
//! creates -> styles -> groups -> connectors -> media -> `rebuild_caches`.
//!
//! A large batch is applied in slices of [`IMPORT_SLICE`] units, one commit
//! each, so the Logic thread answers other messages between them
//! (`TOILE-IO-028`). One Loro undo group spans every slice.

use std::collections::HashMap;

use ambre_bus::RestEvent;
use kurbo::Point;
use loro::{LoroMap, LoroValue};

use crate::logic::command::{
    InitContent, RequestId, ToileCommand, ToileEvent, ToileOutcome, ToileRequest,
};
use crate::logic::interop::{
    ContentHash, ImportBatch, ImportConnector, ImportError, ImportGroup, ImportRef, ImportReport,
    Limits, MediaBlob, import_raster, import_svg, place_batch_at,
};
use crate::logic::model::{ElementId, ElementStyle};

use super::ToileState;
use super::names::{Fields, MediaKey, Root};

/// Rewrite the `pending:{slot}` placeholder to the real blake2b `hash` in both
/// the `media` blob at `slot` and every `image` element that referenced it
/// (`toile-media`). Called as each upload confirmation lands, so by the time
/// the batch is applied no placeholder survives.
fn rewrite_hash(batch: &mut ImportBatch, slot: usize, hash: &ContentHash) {
    let placeholder = format!("pending:{slot}");
    if let Some(blob) = batch.media.get_mut(slot) {
        blob.hash = hash.clone();
    }
    for cmd in &mut batch.commands {
        if let ToileCommand::CreateElement {
            content: InitContent::Image(h),
            ..
        } = cmd
            && h.as_str() == placeholder
        {
            *h = hash.clone();
        }
    }
}

/// An import applied only once every embedded media blob has a real content hash
/// back from Sync (`toile-media`). Holds the fully converted batch and the set
/// of upload `RequestId`s still outstanding. Each id maps to the `media` slot
/// (and the `pending:{slot}` placeholder) it will resolve.
pub(super) struct PendingImport {
    batch: ImportBatch,
    /// Outstanding `UploadMedia` request -> the `batch.media` index it fills.
    outstanding: HashMap<RequestId, usize>,
}

/// How many units one slice of an import applies (`TOILE-IO-028`). A unit is
/// one create, one style, one group or one connector. The `import` bench reads
/// the cost of a slice against the keystroke pass row of
/// `ambre-spec-budgets.md`.
pub(super) const IMPORT_SLICE: usize = 64;

/// An import being applied one slice at a time (`TOILE-IO-028`). The Logic
/// thread resumes it between its messages, so a large file never holds the
/// board's other commands back for longer than one slice.
pub(super) struct ImportJob {
    /// The creates not applied yet, in slot order.
    commands: std::vec::IntoIter<ToileCommand>,
    /// How many creates the batch carried.
    creates: usize,
    styles: Vec<Option<ElementStyle>>,
    groups: Vec<ImportGroup>,
    connectors: Vec<ImportConnector>,
    media: Vec<MediaBlob>,
    report: ImportReport,
    /// The element each create slot minted, `None` when it was rejected.
    slot_ids: Vec<Option<ElementId>>,
    /// The group each group entry built, `None` when it was not.
    group_ids: Vec<Option<ElementId>>,
    /// Units applied so far, across the four phases in order.
    done: usize,
}

impl ImportJob {
    fn new(batch: ImportBatch) -> Self {
        let creates = batch.commands.len();
        Self {
            commands: batch.commands.into_iter(),
            creates,
            styles: batch.styles,
            group_ids: Vec::with_capacity(batch.groups.len()),
            groups: batch.groups,
            connectors: batch.connectors,
            media: batch.media,
            report: batch.report,
            slot_ids: Vec::with_capacity(creates),
            done: 0,
        }
    }

    fn total(&self) -> usize {
        self.creates + self.styles.len() + self.groups.len() + self.connectors.len()
    }
}

impl ToileState {
    /// Convert a dropped SVG document and apply it (`toile-import-export`).
    pub(super) fn import_svg_bytes(&mut self, bytes: &[u8], at: Point) -> ToileOutcome {
        self.begin_import(import_svg(bytes, &Limits::default()), at)
    }

    /// Convert a dropped raster image and apply it (`toile-import-export`). The mime
    /// is re-derived from the bytes by the decoder; the parameter is accepted for
    /// the command surface but not needed here.
    pub(super) fn import_image_bytes(&mut self, bytes: &[u8], at: Point) -> ToileOutcome {
        self.begin_import(import_raster(bytes, &Limits::default()), at)
    }

    /// Route a converted batch. A parse / limit failure aborts with `ImportFailed`
    /// and nothing is applied. A media-free batch takes the FAST PATH and lands
    /// now in one undo group. A media-bearing batch is stashed as a
    /// [`PendingImport`] and issues one `UploadMedia` request per blob, applying
    /// nothing until every real hash returns (`toile-media`).
    fn begin_import(
        &mut self,
        converted: Result<ImportBatch, ImportError>,
        at: Point,
    ) -> ToileOutcome {
        let mut batch = match converted {
            Ok(b) => b,
            Err(e) => {
                return ToileOutcome::display_only(vec![ToileEvent::ImportFailed(e.to_string())]);
            }
        };
        // The file lands where the user put it, before anything is applied
        // (`TOILE-IO-023`). The media round trip may hold the batch for a while,
        // and the drop point is a fact of the gesture, not of the answer.
        place_batch_at(&mut batch, at);
        if batch.media.is_empty() {
            return self.finish_import(batch);
        }
        // Async media: one upload per blob, nothing applied yet.
        let mut outstanding = HashMap::new();
        let mut requests = Vec::with_capacity(batch.media.len());
        for (slot, blob) in batch.media.iter().enumerate() {
            let id = RequestId::next();
            outstanding.insert(id, slot);
            requests.push(ToileRequest::UploadMedia {
                id,
                data: blob.plaintext.clone(),
            });
        }
        self.pending_imports
            .push(PendingImport { batch, outstanding });
        ToileOutcome {
            display: Vec::new(),
            update: None,
            ephemeral: None,
            requests,
        }
    }

    /// Start applying a fully-resolved batch, and apply its first slice now
    /// (`TOILE-IO-028`). A batch of one slice or less lands whole in this
    /// call; a larger one leaves a job the Logic thread resumes between its
    /// messages.
    ///
    /// An import already running is finished first, so two imports never share
    /// an undo group. One Loro undo group spans every slice, which keeps the
    /// whole file one undo step (`TOILE-IO-001`).
    fn finish_import(&mut self, batch: ImportBatch) -> ToileOutcome {
        self.start_import(batch, IMPORT_SLICE)
    }

    /// Open the import's undo group and apply its first `budget` units.
    fn start_import(&mut self, batch: ImportBatch, budget: usize) -> ToileOutcome {
        let mut display = self.finish_running_import();
        if let Err(e) = self.undo.group_start() {
            tracing::warn!("[Toile] the import could not open its undo group: {e:?}");
        }
        self.import_job = Some(Box::new(ImportJob::new(batch)));
        display.extend(self.run_import(budget));
        self.emit_with_update(display)
    }

    /// Apply a batch whole, in one call, and hand back its report: the tests'
    /// way past the media round trip and the slicing.
    #[cfg(test)]
    pub(super) fn apply_import(&mut self, batch: ImportBatch) -> (ImportReport, ToileOutcome) {
        let outcome = self.start_import(batch, usize::MAX);
        let report = outcome
            .display
            .iter()
            .find_map(|e| match e {
                ToileEvent::ImportReport { imported, skipped } => Some(ImportReport {
                    imported: *imported,
                    skipped: skipped.clone(),
                }),
                _ => None,
            })
            .expect("a whole import closes with its report");
        (report, outcome)
    }

    /// Whether an import is still being applied (`TOILE-IO-028`). The Logic
    /// thread reads it to come back to this board without waiting for a
    /// message.
    #[must_use]
    pub fn has_pending_work(&self) -> bool {
        self.import_job.is_some()
    }

    /// Apply the next slice of the running import: at most [`IMPORT_SLICE`]
    /// units, one commit and one delta (`TOILE-IO-028`). The last slice closes
    /// the import and carries its report. With no import running it answers
    /// nothing.
    pub fn resume(&mut self) -> ToileOutcome {
        if self.import_job.is_none() {
            return ToileOutcome::display_only(Vec::new());
        }
        let display = self.run_import(IMPORT_SLICE);
        let outcome = self.emit_with_update(display);
        self.announce_write(outcome)
    }

    /// Apply the rest of a running import at once, without flushing its delta.
    ///
    /// Called before a command that writes the board, and before a second
    /// import. The command's own flush then carries both, in one delta. An
    /// edit therefore never lands inside the import's undo group, and never
    /// between two of its slices.
    pub(super) fn finish_running_import(&mut self) -> Vec<ToileEvent> {
        if self.import_job.is_none() {
            return Vec::new();
        }
        self.run_import(usize::MAX)
    }

    /// Apply up to `budget` units of the running import, then commit. The
    /// caller flushes the delta. A slice short of the end repaints, and the
    /// last one closes the import.
    fn run_import(&mut self, budget: usize) -> Vec<ToileEvent> {
        let Some(mut job) = self.import_job.take() else {
            return Vec::new();
        };
        let end = job.done.saturating_add(budget).min(job.total());
        while job.done < end {
            self.apply_unit(&mut job);
            job.done += 1;
        }
        self.doc.commit();
        if job.done < job.total() {
            self.import_job = Some(job);
            return vec![self.viewport_event()];
        }
        self.close_import(*job)
    }

    /// The import's last step: the media map, the commit that ends the undo
    /// group, and one rebuild of every derived cache.
    fn close_import(&mut self, job: ImportJob) -> Vec<ToileEvent> {
        let ImportJob {
            media,
            mut report,
            slot_ids,
            ..
        } = job;
        self.seed_media(&media);
        self.doc.commit();
        self.undo.group_end();
        self.rebuild_caches();
        report.imported = slot_ids.iter().filter(|s| s.is_some()).count();
        vec![
            self.viewport_event(),
            ToileEvent::ImportReport {
                imported: report.imported,
                skipped: report.skipped,
            },
        ]
    }

    /// Handle a service-plane response routed back to this document
    /// (`toile-media`): a `MediaUploaded` fills one outstanding upload of a
    /// pending import with its real hash. When the last blob resolves, the batch
    /// lands atomically. An id matching no pending import is a stale response
    /// (the import was undone / the doc reopened) and is dropped.
    ///
    /// # Panics
    ///
    /// When the outstanding upload it just matched is absent from the pending
    /// import. The position lookup above establishes it.
    pub fn handle_service_event(&mut self, id: RequestId, evt: RestEvent) -> ToileOutcome {
        let RestEvent::MediaUploaded { content_hash, .. } = evt else {
            // Only upload confirmations concern an import; ignore anything else.
            return ToileOutcome::display_only(Vec::new());
        };
        let Some(pos) = self
            .pending_imports
            .iter()
            .position(|p| p.outstanding.contains_key(&id))
        else {
            return ToileOutcome::display_only(Vec::new());
        };
        let Some(pending) = self.pending_imports.get_mut(pos) else {
            return ToileOutcome::display_only(Vec::new());
        };
        if let Some(slot) = pending.outstanding.remove(&id) {
            rewrite_hash(&mut pending.batch, slot, &content_hash);
        }
        let ready = pending.outstanding.is_empty();
        if !ready {
            // More uploads still in flight; apply nothing yet.
            return ToileOutcome::display_only(Vec::new());
        }
        let pending = self.pending_imports.remove(pos);
        self.finish_import(pending.batch)
    }

    /// Handle a service-plane failure routed back to this document
    /// (`toile-media`): a failed media upload aborts the whole pending import
    /// (atomicity), leaving the board untouched. An id matching no pending
    /// import is dropped.
    pub fn handle_service_error(&mut self, id: RequestId, reason: String) -> ToileOutcome {
        let Some(pos) = self
            .pending_imports
            .iter()
            .position(|p| p.outstanding.contains_key(&id))
        else {
            return ToileOutcome::display_only(Vec::new());
        };
        self.pending_imports.remove(pos);
        ToileOutcome::display_only(vec![ToileEvent::ImportFailed(format!(
            "media upload failed: {reason}"
        ))])
    }

    /// Number of imports awaiting media uploads. Test / integration accessor.
    pub fn pending_import_count(&self) -> usize {
        self.pending_imports.len()
    }

    /// Apply one unit of the running import. The units run in a strict order,
    /// so every slot or group reference resolves before it is used:
    ///
    /// 1. **creates** -> `create_element_inner`, minting an [`ElementId`] per slot;
    /// 2. **styles** -> `restyle_inner` on the slot's fresh element (style is an
    ///    independent LWW register, so it cannot ride the create);
    /// 3. **groups** -> `group_inner`, innermost-first, so a `Group(i)` member
    ///    always resolves to an already-built group;
    /// 4. **connectors** -> `create_connector_inner`, endpoints resolved by ref.
    ///
    /// The media map and the cache rebuild follow in
    /// [`close_import`](Self::close_import). A create that unexpectedly fails is
    /// recorded in the report and its slot left unresolved, so any group or
    /// connector referencing it is skipped. A trusted converter should never
    /// emit one.
    fn apply_unit(&mut self, job: &mut ImportJob) {
        let mut unit = job.done;
        if unit < job.creates {
            let created = match job.commands.next() {
                Some(ToileCommand::CreateElement {
                    kind,
                    at,
                    size,
                    content,
                    fill,
                    ..
                }) => self
                    .create_element_inner(kind, at, size, content, fill)
                    .map_err(|_| format!("element slot {unit}: rejected by the engine")),
                // The batch contract is creates-only; anything else is a
                // converter bug. Drop it rather than corrupt slot indices.
                _ => Err(format!("import slot {unit}: non-create command dropped")),
            };
            match created {
                Ok(id) => job.slot_ids.push(Some(id)),
                Err(reason) => {
                    job.report.skip(reason);
                    job.slot_ids.push(None);
                }
            }
            return;
        }
        unit -= job.creates;
        if unit < job.styles.len() {
            if let (Some(Some(id)), Some(Some(style))) =
                (job.slot_ids.get(unit), job.styles.get(unit))
            {
                self.restyle_inner(std::slice::from_ref(id), style);
            }
            return;
        }
        unit -= job.styles.len();
        if let Some(group) = job.groups.get(unit) {
            let members: Vec<ElementId> = group
                .members
                .iter()
                .filter_map(|r| self.resolve_ref(*r, &job.slot_ids, &job.group_ids))
                .collect();
            let built = if members.len() < 2 {
                None
            } else {
                self.group_inner(&members)
            };
            if built.is_some() {
                // Seed the fresh group's derived bounds now, so a later
                // `Group(i)` member passes `group_inner`'s existence check
                // (bounds-keyed) before the final `rebuild_caches`.
                self.recompute_groups();
            }
            job.group_ids.push(built);
            return;
        }
        unit -= job.groups.len();
        // `done` stops at the last connector, so a unit past them is no unit.
        let Some(conn) = job.connectors.get(unit) else {
            return;
        };
        let (Some(from), Some(to)) = (
            self.resolve_ref(conn.from, &job.slot_ids, &job.group_ids),
            self.resolve_ref(conn.to, &job.slot_ids, &job.group_ids),
        ) else {
            job.report.skip("connector: endpoint did not resolve");
            return;
        };
        self.create_connector_inner(
            from,
            conn.from_anch,
            to,
            conn.to_anch,
            conn.path_type,
            conn.arrows,
        );
    }

    /// Resolve an [`ImportRef`] to a live [`ElementId`], or `None` if its slot or
    /// group was not created, or a peer deleted it between two slices.
    fn resolve_ref(
        &self,
        r: ImportRef,
        slot_ids: &[Option<ElementId>],
        group_ids: &[Option<ElementId>],
    ) -> Option<ElementId> {
        let id = match r {
            ImportRef::Slot(i) => slot_ids.get(i).cloned().flatten(),
            ImportRef::Group(i) => group_ids.get(i).cloned().flatten(),
        }?;
        self.bounds.contains_key(&id).then_some(id)
    }
    /// Populate the `media` root map from a batch's plaintext blobs
    /// (`toile-elements`), **without** committing. Each blob's `hash` (the real
    /// blake2b, resolved Sync-side by the time this runs) keys a `{w, h, mime,
    /// thumb}` map. Dedup is content-addressed: a hash already present is
    /// left untouched, so N elements sharing one image seed exactly one entry.
    /// Returns the count of NEW entries created (existing keys are skipped).
    ///
    /// Commit-free by design: the caller (`apply_import`) folds this into the one
    /// import undo group alongside the element creates.
    pub(super) fn seed_media(&mut self, blobs: &[MediaBlob]) -> usize {
        let media = self.doc.get_map(Root::Media.as_str());
        let mut created = 0;
        for blob in blobs {
            let key = blob.hash.as_str();
            if !crate::logic::interop::media::is_resolved(&blob.hash) || media.get(key).is_some() {
                // No address from Sync yet, or the entry already exists
                // (dedup). A placeholder token never becomes a key.
                continue;
            }
            let Ok(entry) = media
                .insert_container(key, LoroMap::new())
                .map(Fields::<MediaKey>::of)
            else {
                continue;
            };
            let _ = entry.insert(MediaKey::W, blob.width as i64);
            let _ = entry.insert(MediaKey::H, blob.height as i64);
            let _ = entry.insert(MediaKey::Mime, blob.mime.as_str());
            let _ = entry.insert(
                MediaKey::Thumb,
                LoroValue::Binary(blob.thumb.clone().into()),
            );
            created += 1;
        }
        created
    }

    /// Number of entries in the `media` map. Test / integration accessor.
    pub fn media_count(&self) -> usize {
        match self.doc.get_map(Root::Media.as_str()).get_deep_value() {
            LoroValue::Map(m) => m.len(),
            _ => 0,
        }
    }

    /// The `(w, h, mime)` load-info of a media entry, if present. Test accessor.
    #[cfg(test)]
    pub(super) fn media_meta(&self, hash: &str) -> Option<(i64, i64, String)> {
        let LoroValue::Map(m) = self.doc.get_map(Root::Media.as_str()).get_deep_value() else {
            return None;
        };
        let LoroValue::Map(entry) = m.get(hash)? else {
            return None;
        };
        let entry = super::names::FieldValues::<MediaKey>::of(entry);
        let w = match entry.get(MediaKey::W)? {
            LoroValue::I64(v) => *v,
            _ => return None,
        };
        let h = match entry.get(MediaKey::H)? {
            LoroValue::I64(v) => *v,
            _ => return None,
        };
        let mime = match entry.get(MediaKey::Mime)? {
            LoroValue::String(s) => s.to_string(),
            _ => return None,
        };
        Some((w, h, mime))
    }
}

#[cfg(test)]
mod tests {
    use ambre_core::Rgba;
    use kurbo::{Point, Size};

    use crate::logic::command::{InitContent, ToileCommand};
    use crate::logic::interop::{ContentHash, ImportBatch, ImportGroup, ImportRef, MediaBlob};
    use crate::logic::model::{ElementKind, ElementStyle, ShapeKind};
    use crate::logic::state::ToileState;

    /// A `CreateElement` for a plain rect at `x`, used to fill import slots.
    fn rect_cmd(x: f64) -> ToileCommand {
        ToileCommand::CreateElement {
            kind: ElementKind::Shape(ShapeKind::Rect),
            at: Point::new(x, 0.0),
            size: Size::new(10.0, 10.0),
            content: InitContent::None,
            fill: None,
            correlate: 0,
        }
    }

    #[test]
    fn import_is_one_undo_group() {
        let mut st = ToileState::new();
        let batch = ImportBatch {
            commands: (0..5).map(|i| rect_cmd(i as f64 * 20.0)).collect(),
            styles: vec![None; 5],
            ..Default::default()
        };
        let (report, outcome) = st.apply_import(batch);
        assert_eq!(report.imported, 5);
        assert_eq!(st.element_count(), 5);
        assert!(outcome.update.is_some(), "one CRDT delta is broadcast");
        assert!(
            outcome.requests.is_empty(),
            "no media -> no service request"
        );
        // The single strongest invariant: one Undo empties the whole import.
        st.handle_command(ToileCommand::Undo);
        assert_eq!(st.element_count(), 0);
    }

    #[test]
    fn import_applies_styles_per_slot() {
        let mut st = ToileState::new();
        let style = ElementStyle {
            fill: Some(Rgba::from_bits(0x3366_CCFF)),
            ..ElementStyle::default()
        };
        let batch = ImportBatch {
            commands: vec![rect_cmd(0.0)],
            styles: vec![Some(style.clone())],
            ..Default::default()
        };
        let (report, _) = st.apply_import(batch);
        assert_eq!(report.imported, 1);
        // The one element carries the imported fill, not the default.
        let vp = st.derive_viewport();
        let fills: Vec<Option<Rgba>> = vp
            .nodes
            .iter()
            .filter_map(|n| match n {
                crate::logic::command::DisplayNode::Element(e) => Some(e.style.fill),
                _ => None,
            })
            .collect();
        assert_eq!(fills, vec![Some(Rgba::from_bits(0x3366_CCFF))]);
    }

    #[test]
    fn import_rebuilds_nested_groups() {
        let mut st = ToileState::new();
        // Slots 0,1,2 are rects; group 0 = {0,1}; group 1 = {2, group 0}.
        let batch = ImportBatch {
            commands: vec![rect_cmd(0.0), rect_cmd(20.0), rect_cmd(40.0)],
            styles: vec![None; 3],
            groups: vec![
                ImportGroup {
                    members: vec![ImportRef::Slot(0), ImportRef::Slot(1)],
                },
                ImportGroup {
                    members: vec![ImportRef::Slot(2), ImportRef::Group(0)],
                },
            ],
            ..Default::default()
        };
        let (report, _) = st.apply_import(batch);
        assert_eq!(report.imported, 3);
        // Three rects plus two group elements.
        assert_eq!(st.element_count(), 5);
        // Still one undo group: a single Undo removes every element AND group.
        st.handle_command(ToileCommand::Undo);
        assert_eq!(st.element_count(), 0);
    }

    #[test]
    fn import_seeds_the_media_map() {
        let mut st = ToileState::new();
        let batch = ImportBatch {
            commands: vec![rect_cmd(0.0)],
            styles: vec![None],
            media: vec![blob("img-hash", 32, 32, "image/png")],
            ..Default::default()
        };
        st.apply_import(batch);
        assert_eq!(st.media_count(), 1);
        assert_eq!(
            st.media_meta("img-hash"),
            Some((32, 32, "image/png".to_owned()))
        );
    }

    fn blob(hash: &str, w: u32, h: u32, mime: &str) -> MediaBlob {
        MediaBlob {
            hash: ContentHash(hash.to_owned()),
            plaintext: vec![0u8; 4],
            mime: ambre_core::MediaType::from(mime),
            width: w,
            height: h,
            thumb: vec![1, 2, 3],
        }
    }

    #[test]
    fn seed_media_writes_load_info() {
        let mut st = ToileState::new();
        let n = st.seed_media(&[blob("abc", 640, 480, "image/png")]);
        assert_eq!(n, 1);
        assert_eq!(st.media_count(), 1);
        assert_eq!(
            st.media_meta("abc"),
            Some((640, 480, "image/png".to_owned()))
        );
    }

    #[test]
    fn seed_media_dedups_repeated_hashes() {
        let mut st = ToileState::new();
        // Two blobs with the same content hash collapse to one entry (dedup).
        let n = st.seed_media(&[
            blob("dup", 10, 10, "image/png"),
            blob("dup", 10, 10, "image/png"),
        ]);
        assert_eq!(n, 1, "identical hashes seed exactly one entry");
        assert_eq!(st.media_count(), 1);
        // A later seed of the same hash is a no-op (already present).
        assert_eq!(st.seed_media(&[blob("dup", 10, 10, "image/png")]), 0);
        assert_eq!(st.media_count(), 1);
    }

    #[test]
    fn seed_media_skips_unresolved_hash() {
        let mut st = ToileState::new();
        // An empty hash means Sync has not returned the blake2b yet: never seed a
        // placeholder key collaborators could not resolve.
        let n = st.seed_media(&[blob("", 10, 10, "image/png")]);
        assert_eq!(n, 0);
        assert_eq!(st.media_count(), 0);
    }

    /// `TOILE-IO-059`: the token an import names a blob by until Sync answers
    /// is refused as a key, the very token the decoder mints included.
    #[test]
    fn a_placeholder_token_never_becomes_a_key() {
        let mut st = ToileState::new();
        let decoded = crate::logic::interop::import_raster(
            &solid_png(4, 4),
            &crate::logic::interop::Limits::default(),
        )
        .expect("a picture");
        let minted = decoded.media[0].clone();
        assert!(
            !crate::logic::interop::media::is_resolved(&minted.hash),
            "{:?}",
            minted.hash
        );
        assert_eq!(
            st.seed_media(&[minted, blob("pending:7", 1, 1, "image/png")]),
            0
        );
        assert_eq!(st.media_count(), 0);
        assert_eq!(st.seed_media(&[blob("realhash", 1, 1, "image/png")]), 1);
    }

    // -- command path (fast / async / failure) --------------------------------

    use ambre_bus::RestEvent;
    use ambre_core::DocumentId;
    use std::io::Cursor;

    use crate::logic::command::{RequestId, ToileEvent, ToileOutcome, ToileRequest};

    /// A minimal solid-color PNG produced with the `image` crate (no fixture).
    fn solid_png(w: u32, h: u32) -> Vec<u8> {
        let img = image::RgbaImage::from_pixel(w, h, image::Rgba([10, 20, 30, 255]));
        let mut bytes = Vec::new();
        image::DynamicImage::ImageRgba8(img)
            .write_to(&mut Cursor::new(&mut bytes), image::ImageFormat::Png)
            .unwrap();
        bytes
    }

    fn first_upload_id(out: &ToileOutcome) -> RequestId {
        match out.requests.first().expect("an upload request") {
            ToileRequest::UploadMedia { id, .. } => *id,
            _ => panic!("expected UploadMedia"),
        }
    }

    fn has_report(out: &ToileOutcome) -> bool {
        out.display
            .iter()
            .any(|e| matches!(e, ToileEvent::ImportReport { .. }))
    }

    #[test]
    fn svg_import_takes_the_fast_path() {
        let mut st = ToileState::new();
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><rect x="0" y="0" width="5" height="5" fill="#ff0000"/></svg>"##;
        let out = st.handle_command(ToileCommand::ImportSvg {
            bytes: svg.to_vec(),
            at: Point::ZERO,
        });
        // No media -> applied synchronously, one CRDT delta, no service request.
        assert!(out.update.is_some());
        assert!(out.requests.is_empty());
        assert!(has_report(&out), "an ImportReport is surfaced");
        assert_eq!(st.element_count(), 1);
        assert_eq!(st.pending_import_count(), 0);
    }

    /// A file lands where the user dropped it, not at the coordinates it carried
    /// (`TOILE-IO-023`).
    #[test]
    fn an_import_lands_on_the_drop_point() {
        let mut st = ToileState::new();
        // The rect sits at (40, 40) inside the drawing; dropped at (300, 200) it
        // must be the batch's top-left corner that moves there.
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100"><rect x="40" y="40" width="5" height="5"/></svg>"##;
        st.handle_command(ToileCommand::ImportSvg {
            bytes: svg.to_vec(),
            at: Point::new(300.0, 200.0),
        });
        let id = st
            .layer_order()
            .iter()
            .find_map(|n| match n {
                crate::logic::model::NodeId::Element(e) => Some(e.clone()),
                _ => None,
            })
            .expect("the import created one element");
        let r = st.bounds_of(&id).expect("the element has bounds");
        assert_eq!((r.x0, r.y0), (300.0, 200.0));
    }

    #[test]
    fn a_broken_file_fails_without_touching_the_board() {
        let mut st = ToileState::new();
        let out = st.handle_command(ToileCommand::ImportSvg {
            bytes: b"this is not svg".to_vec(),
            at: Point::ZERO,
        });
        assert!(
            out.display
                .iter()
                .any(|e| matches!(e, ToileEvent::ImportFailed(_)))
        );
        assert!(out.update.is_none());
        assert_eq!(st.element_count(), 0);
    }

    /// Covers `TOILE-IO-071`: an import onto a populated board leaves each
    /// element it found there as it was.
    ///
    /// The file carries a group, so the apply pass regroups and restyles. The
    /// stored map of the earlier element is compared whole, before and after.
    #[test]
    fn an_import_leaves_the_board_it_lands_on_alone() {
        let mut st = ToileState::new();
        let held = st
            .handle_command(ToileCommand::CreateElement {
                kind: crate::logic::model::ElementKind::Shape(crate::logic::model::ShapeKind::Rect),
                at: Point::new(5.0, 5.0),
                size: kurbo::Size::new(40.0, 40.0),
                content: crate::logic::command::InitContent::None,
                fill: None,
                correlate: 0,
            })
            .display
            .into_iter()
            .find_map(|e| match e {
                ToileEvent::ElementCreated { id, .. } => Some(id),
                _ => None,
            })
            .expect("a minted id");
        let before = st.element_map(&held).expect("held").map().get_deep_value();

        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100"><g><rect x="0" y="0" width="5" height="5" fill="#ff0000"/><rect x="10" y="0" width="5" height="5"/></g></svg>"##;
        st.handle_command(ToileCommand::ImportSvg {
            bytes: svg.to_vec(),
            at: Point::ZERO,
        });
        assert!(st.element_count() > 1, "the file landed");
        assert_eq!(
            st.element_map(&held).expect("held").map().get_deep_value(),
            before
        );
    }

    /// Covers `TOILE-IO-094`: each format no converter reads is refused by
    /// its own name, and the board is left as it was.
    #[test]
    fn figma_and_miro_are_refused_by_name() {
        let mut st = ToileState::new();
        for (cmd, name) in [
            (
                ToileCommand::ImportFigma {
                    bytes: b"{}".to_vec(),
                    at: Point::ZERO,
                },
                "Figma",
            ),
            (
                ToileCommand::ImportMiro {
                    bytes: b"{}".to_vec(),
                    at: Point::ZERO,
                },
                "Miro",
            ),
        ] {
            let out = st.handle_command(cmd);
            assert!(out.update.is_none(), "{name} wrote to the board");
            assert!(
                out.display
                    .iter()
                    .any(|e| matches!(e, ToileEvent::ImportFailed(why) if why.starts_with(name))),
                "{name}: {:?}",
                out.display
            );
        }
        assert_eq!(st.element_count(), 0);
    }

    #[test]
    fn image_import_defers_until_media_uploads() {
        let mut st = ToileState::new();
        let out = st.handle_command(ToileCommand::ImportImage {
            bytes: solid_png(8, 8),
            mime: ambre_core::MediaType::from("image/png"),
            at: Point::ZERO,
        });
        // Async: one upload issued, nothing applied yet.
        assert_eq!(out.requests.len(), 1);
        assert!(out.update.is_none(), "board unchanged until media resolves");
        assert_eq!(st.element_count(), 0);
        assert_eq!(st.pending_import_count(), 1);

        // Sync returns the real hash: the batch lands with it, no `pending:` left.
        let id = first_upload_id(&out);
        let done = st.handle_service_event(
            id,
            RestEvent::MediaUploaded {
                document_id: DocumentId::new(),
                content_hash: ambre_core::ContentHash("realhash".to_owned()),
            },
        );
        assert!(has_report(&done));
        assert!(
            done.update.is_some(),
            "the applied batch broadcasts a delta"
        );
        assert_eq!(st.element_count(), 1);
        assert_eq!(st.media_count(), 1);
        assert!(st.media_meta("realhash").is_some());
        assert_eq!(st.pending_import_count(), 0);
        // One Undo removes the whole (now-resolved) import.
        st.handle_command(ToileCommand::Undo);
        assert_eq!(st.element_count(), 0);
    }

    /// Covers `TOILE-IO-031`: a picture on the board reaches the page as its
    /// pixels.
    ///
    /// The page reads the store the export fills from the board. With that
    /// store it draws one 8x8 image; with an empty one it draws none.
    #[cfg(feature = "pdf")]
    #[test]
    fn a_picture_reaches_the_page_as_pixels() {
        use ambre_draw::testing::{DrawOp, RecordingSurface};

        let mut st = ToileState::new();
        let out = st.handle_command(ToileCommand::ImportImage {
            bytes: solid_png(8, 8),
            mime: ambre_core::MediaType::from("image/png"),
            at: Point::ZERO,
        });
        st.handle_service_event(
            first_upload_id(&out),
            RestEvent::MediaUploaded {
                document_id: DocumentId::new(),
                content_hash: ambre_core::ContentHash("realhash".to_owned()),
            },
        );
        let pages = st.pdf_pages(crate::logic::interop::PageSize::default());
        let paint = |images: &crate::ui::render::ImageStore| {
            let mut s = RecordingSurface::new();
            crate::logic::interop::paint_page(
                &mut s,
                &pages[0],
                ambre_canvas::Theme::Light.palette(),
                &mut ambre_draw::FontContext::default(),
                &mut ambre_canvas::TextCache::new(1.0),
                images,
            );
            s.count(|op| {
                matches!(
                    op,
                    DrawOp::DrawImage {
                        width: 8,
                        height: 8,
                        ..
                    }
                )
            })
        };
        assert_eq!(paint(&st.decoded_media()), 1, "the picture's pixels");
        assert_eq!(paint(&crate::ui::render::ImageStore::new()), 0);
    }

    /// `TOILE-IO-036`: the undo of a picture import takes back the entry it
    /// seeded with its element, and a redo brings both back.
    #[test]
    fn undoing_an_image_import_takes_its_media_entry_back() {
        let mut st = ToileState::new();
        let out = st.handle_command(ToileCommand::ImportImage {
            bytes: solid_png(8, 8),
            mime: ambre_core::MediaType::from("image/png"),
            at: Point::ZERO,
        });
        st.handle_service_event(
            first_upload_id(&out),
            RestEvent::MediaUploaded {
                document_id: DocumentId::new(),
                content_hash: ambre_core::ContentHash("realhash".to_owned()),
            },
        );
        assert_eq!((st.element_count(), st.media_count()), (1, 1));
        st.handle_command(ToileCommand::Undo);
        assert_eq!((st.element_count(), st.media_count()), (0, 0));
        st.handle_command(ToileCommand::Redo);
        assert_eq!((st.element_count(), st.media_count()), (1, 1));
        assert!(st.media_meta("realhash").is_some());
    }

    #[test]
    fn a_failed_upload_aborts_the_pending_import() {
        let mut st = ToileState::new();
        let out = st.handle_command(ToileCommand::ImportImage {
            bytes: solid_png(8, 8),
            mime: ambre_core::MediaType::from("image/png"),
            at: Point::ZERO,
        });
        let id = first_upload_id(&out);
        let failed = st.handle_service_error(id, "S3 PUT 500".to_owned());
        assert!(
            failed
                .display
                .iter()
                .any(|e| matches!(e, ToileEvent::ImportFailed(_)))
        );
        assert_eq!(st.element_count(), 0, "board untouched (atomicity)");
        assert_eq!(st.pending_import_count(), 0, "the registry is drained");
    }

    #[test]
    fn a_stale_service_response_is_dropped() {
        let mut st = ToileState::new();
        // No pending import: a late MediaUploaded (import undone / doc reopened)
        // is dropped without panic and changes nothing.
        let out = st.handle_service_event(
            RequestId::next(),
            RestEvent::MediaUploaded {
                document_id: DocumentId::new(),
                content_hash: ambre_core::ContentHash("orphan".to_owned()),
            },
        );
        assert!(out.display.is_empty());
        assert!(out.update.is_none());
        assert_eq!(st.element_count(), 0);
    }

    // -- slicing (`TOILE-IO-028`) -------------------------------------------

    /// Elements a large import carries: three slices and a bit.
    const LARGE: usize = super::IMPORT_SLICE * 3 + 7;

    fn large_import() -> ToileCommand {
        let mut svg = String::from(r#"<svg xmlns="http://www.w3.org/2000/svg">"#);
        for i in 0..LARGE {
            svg.push_str(&format!(r#"<rect x="{i}" y="0" width="5" height="5"/>"#));
        }
        svg.push_str("</svg>");
        ToileCommand::ImportSvg {
            bytes: svg.into_bytes(),
            at: Point::ZERO,
        }
    }

    /// A large import applies one slice in the command, and the rest one slice
    /// per `resume`. Each slice is a delta of its own, and only the last one
    /// reports.
    #[test]
    fn a_large_import_lands_one_slice_at_a_time() {
        let mut st = ToileState::new();
        let first = st.handle_command(large_import());
        assert!(first.update.is_some(), "the first slice is broadcast");
        assert!(!has_report(&first), "the import is not over");
        assert_eq!(st.element_count(), super::IMPORT_SLICE);
        assert!(st.has_pending_work());
        let mut resumes = 0;
        let mut last = None;
        while st.has_pending_work() {
            let out = st.resume();
            assert!(out.update.is_some(), "every slice is broadcast");
            resumes += 1;
            last = Some(out);
        }
        // A unit is a create or a style, and the SVG converter styles every slot.
        assert_eq!(resumes, (2 * LARGE).div_ceil(super::IMPORT_SLICE) - 1);
        assert!(has_report(&last.expect("a last slice")));
        assert_eq!(st.element_count(), LARGE);
        assert!(st.resume().update.is_none(), "nothing left to resume");
    }

    /// A camera move passes a running import: it reads nothing the import is
    /// still writing, and the board stays responsive.
    #[test]
    fn a_camera_move_does_not_wait_for_the_import() {
        let mut st = ToileState::new();
        st.handle_command(large_import());
        let out = st.handle_command(ToileCommand::SetViewport(crate::logic::index::Viewport {
            center: Point::ZERO,
            zoom: ambre_core::ZoomFactor::positive(0.5),
            size: Size::new(800.0, 600.0),
        }));
        assert!(out.update.is_none());
        assert_eq!(st.element_count(), super::IMPORT_SLICE);
        assert!(st.has_pending_work());
    }

    /// A command that writes the board lands the import whole first. Its one
    /// delta carries the rest of the import and the edit, and a peer that
    /// applies the deltas in order holds both.
    #[test]
    fn an_edit_lands_the_import_first_in_one_delta() {
        let mut st = ToileState::new();
        let mut peer = ToileState::new();
        let first = st.handle_command(large_import());
        peer.apply_remote(&first.update.expect("first slice"));
        let edit = st.handle_command(rect_cmd(-100.0));
        assert!(!st.has_pending_work(), "the edit finished the import");
        assert!(has_report(&edit), "the edit's answer closes the import");
        peer.apply_remote(&edit.update.expect("one delta"));
        assert_eq!(st.element_count(), LARGE + 1);
        assert_eq!(peer.element_count(), LARGE + 1);
    }

    /// One Undo removes a sliced import whole: every slice shares one undo
    /// group (`TOILE-IO-001`).
    #[test]
    fn undo_removes_a_sliced_import_whole() {
        let mut st = ToileState::new();
        st.handle_command(rect_cmd(-100.0));
        st.handle_command(large_import());
        while st.has_pending_work() {
            st.resume();
        }
        assert_eq!(st.element_count(), LARGE + 1);
        st.handle_command(ToileCommand::Undo);
        assert_eq!(st.element_count(), 1, "the earlier rect stays");
    }

    /// An Undo that arrives mid-import lands the import first, then removes
    /// it whole, never half of it.
    #[test]
    fn undo_mid_import_removes_it_whole() {
        let mut st = ToileState::new();
        st.handle_command(rect_cmd(-100.0));
        st.handle_command(large_import());
        st.handle_command(ToileCommand::Undo);
        assert!(!st.has_pending_work());
        assert_eq!(st.element_count(), 1);
    }
}
