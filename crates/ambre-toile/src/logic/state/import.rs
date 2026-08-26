//! Atomic foreign-file import into [`ToileState`](super::ToileState)
//! (`toile-import-export`). An [`ImportBatch`](crate::logic::interop::ImportBatch)
//! produced by a pure converter is applied here as ONE undo group: every
//! `CreateElement`, style, group, connector and media seed shares a single
//! `doc.commit()` via the commit-free `*_inner` helpers. The pass order
//! (creates -> styles -> groups -> connectors -> media -> one commit ->
//! `rebuild_caches`) resolves every slot / group reference before it is used.

use std::collections::HashMap;

use ambre_bus::RestEvent;
use loro::{LoroMap, LoroValue};

use crate::logic::command::{
    InitContent, RequestId, ToileCommand, ToileEvent, ToileOutcome, ToileRequest,
};
use crate::logic::interop::{
    ContentHash, ImportBatch, ImportError, ImportRef, ImportReport, Limits, MediaBlob,
    import_raster, import_svg,
};
use crate::logic::model::ElementId;

use super::{MEDIA, ToileState};

/// Rewrite the `pending:{slot}` placeholder to the real blake2b `hash` in both
/// the `media` blob at `slot` and every `image` element that referenced it
/// (`toile-media`). Called as each upload confirmation lands, so by the time
/// the batch is applied no placeholder survives.
fn rewrite_hash(batch: &mut ImportBatch, slot: usize, hash: &str) {
    let placeholder = format!("pending:{slot}");
    if let Some(blob) = batch.media.get_mut(slot) {
        blob.hash = ContentHash(hash.to_owned());
    }
    for cmd in &mut batch.commands {
        if let ToileCommand::CreateElement {
            content: InitContent::Image(h),
            ..
        } = cmd
            && *h == placeholder
        {
            *h = hash.to_owned();
        }
    }
}

/// An import applied only once every embedded media blob has a real content hash
/// back from Sync (`toile-media`). Holds the fully converted batch and the
/// set of upload `RequestId`s still outstanding, each mapped to the `media` slot
/// (and the `pending:{slot}` placeholder) it will resolve.
pub(super) struct PendingImport {
    batch: ImportBatch,
    /// Outstanding `UploadMedia` request -> the `batch.media` index it fills.
    outstanding: HashMap<RequestId, usize>,
}

impl ToileState {
    /// Convert a dropped SVG document and apply it (`toile-import-export`).
    pub(super) fn import_svg_bytes(&mut self, bytes: &[u8]) -> ToileOutcome {
        self.begin_import(import_svg(bytes, &Limits::default()))
    }

    /// Convert a dropped raster image and apply it (`toile-import-export`). The mime
    /// is re-derived from the bytes by the decoder; the parameter is accepted for
    /// the command surface but not needed here.
    pub(super) fn import_image_bytes(&mut self, bytes: &[u8]) -> ToileOutcome {
        self.begin_import(import_raster(bytes, &Limits::default()))
    }

    /// Route a converted batch: a parse / limit failure aborts with `ImportFailed`
    /// (nothing applied,); a media-free batch takes the FAST PATH and lands
    /// now in one undo group; a media-bearing batch is stashed as a
    /// [`PendingImport`] and issues one `UploadMedia` request per blob, applying
    /// nothing until every real hash returns (`toile-media`).
    fn begin_import(&mut self, converted: Result<ImportBatch, ImportError>) -> ToileOutcome {
        let batch = match converted {
            Ok(b) => b,
            Err(e) => {
                return ToileOutcome::display_only(vec![ToileEvent::ImportFailed(e.to_string())]);
            }
        };
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
                mime: blob.mime.clone(),
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

    /// Apply a fully-resolved batch and wrap the [`ImportReport`] into a display
    /// event alongside the repaint viewport.
    fn finish_import(&mut self, batch: ImportBatch) -> ToileOutcome {
        let (report, mut outcome) = self.apply_import(batch);
        outcome.display.push(ToileEvent::ImportReport {
            imported: report.imported,
            skipped: report.skipped,
        });
        outcome
    }

    /// Handle a service-plane response routed back to this document
    /// (`toile-media`): a `MediaUploaded` fills one outstanding upload of a
    /// pending import with its real hash. When the last blob resolves, the batch
    /// lands atomically. An id matching no pending import is a stale response
    /// (the import was undone / the doc reopened) and is dropped.
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
        let ready = {
            let pending = &mut self.pending_imports[pos];
            let slot = pending.outstanding.remove(&id).expect("id present");
            rewrite_hash(&mut pending.batch, slot, &content_hash);
            pending.outstanding.is_empty()
        };
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

    /// Apply a converted [`ImportBatch`] as ONE atomic undo group
    /// (`toile-import-export`). The pass is strictly ordered so every slot /
    /// group reference resolves before it is used:
    ///
    /// 1. **creates** -> `create_element_inner`, minting an [`ElementId`] per slot;
    /// 2. **styles** -> `restyle_inner` on the slot's fresh element (style is an
    ///    independent LWW register, so it cannot ride the create);
    /// 3. **groups** -> `group_inner`, innermost-first, so a `Group(i)` member
    ///    always resolves to an already-built group;
    /// 4. **connectors** -> `create_connector_inner`, endpoints resolved by ref;
    /// 5. **media** -> `seed_media` (dedup);
    /// 6. a single `doc.commit()` + one `rebuild_caches`.
    ///
    /// Returns the finalized [`ImportReport`] (its `imported` count is the number
    /// of elements actually created) and a [`ToileOutcome`] carrying the one CRDT
    /// delta plus the repaint viewport. A create that unexpectedly fails (a
    /// trusted converter should never emit one) is recorded in the report and its
    /// slot left unresolved, so any group / connector referencing it is skipped.
    pub(super) fn apply_import(&mut self, batch: ImportBatch) -> (ImportReport, ToileOutcome) {
        let ImportBatch {
            commands,
            styles,
            groups,
            connectors,
            media,
            mut report,
        } = batch;

        // 1. Creates -> one ElementId per slot (None = the create was rejected).
        let mut slot_ids: Vec<Option<ElementId>> = Vec::with_capacity(commands.len());
        for (i, cmd) in commands.into_iter().enumerate() {
            match cmd {
                ToileCommand::CreateElement {
                    kind,
                    at,
                    size,
                    content,
                    fill,
                } => match self.create_element_inner(kind, at, size, content, fill) {
                    Ok(id) => slot_ids.push(Some(id)),
                    Err(_) => {
                        report.skip(format!("element slot {i}: rejected by the engine"));
                        slot_ids.push(None);
                    }
                },
                _ => {
                    // The batch contract is creates-only; anything else is a
                    // converter bug. Drop it rather than corrupt slot indices.
                    report.skip(format!("import slot {i}: non-create command dropped"));
                    slot_ids.push(None);
                }
            }
        }

        // 2. Styles, index-aligned with the create slots.
        for (slot, style) in styles.iter().enumerate() {
            if let (Some(Some(id)), Some(style)) = (slot_ids.get(slot), style) {
                self.restyle_inner(std::slice::from_ref(id), style);
            }
        }

        // 3. Groups, innermost-first: resolve each member ref, then build.
        let mut group_ids: Vec<Option<ElementId>> = Vec::with_capacity(groups.len());
        for group in &groups {
            let members: Vec<ElementId> = group
                .members
                .iter()
                .filter_map(|r| self.resolve_ref(*r, &slot_ids, &group_ids))
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
            group_ids.push(built);
        }

        // 4. Connectors: both endpoints resolved by ref, else dropped.
        for conn in &connectors {
            let (Some(from), Some(to)) = (
                self.resolve_ref(conn.from, &slot_ids, &group_ids),
                self.resolve_ref(conn.to, &slot_ids, &group_ids),
            ) else {
                report.skip("connector: endpoint did not resolve");
                continue;
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

        // 5. Media map (content-addressed dedup).
        self.seed_media(&media);

        // 6. One commit = one undo item; then reseed every derived cache once.
        self.doc.commit();
        self.rebuild_caches();

        report.imported = slot_ids.iter().filter(|s| s.is_some()).count();
        let outcome = self.emit_with_update(vec![self.viewport_event()]);
        (report, outcome)
    }

    /// Resolve an [`ImportRef`] to a live [`ElementId`], or `None` if its slot /
    /// group was not created.
    fn resolve_ref(
        &self,
        r: ImportRef,
        slot_ids: &[Option<ElementId>],
        group_ids: &[Option<ElementId>],
    ) -> Option<ElementId> {
        match r {
            ImportRef::Slot(i) => slot_ids.get(i).cloned().flatten(),
            ImportRef::Group(i) => group_ids.get(i).cloned().flatten(),
        }
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
        let media = self.doc.get_map(MEDIA);
        let mut created = 0;
        for blob in blobs {
            let key = blob.hash.as_str();
            if key.is_empty() || media.get(key).is_some() {
                // No real hash yet, or the entry already exists (dedup,).
                continue;
            }
            let Ok(entry) = media.insert_container(key, LoroMap::new()) else {
                continue;
            };
            let _ = entry.insert("w", blob.width as i64);
            let _ = entry.insert("h", blob.height as i64);
            let _ = entry.insert("mime", blob.mime.as_str());
            let _ = entry.insert("thumb", LoroValue::Binary(blob.thumb.clone().into()));
            created += 1;
        }
        created
    }

    /// Number of entries in the `media` map. Test / integration accessor.
    pub fn media_count(&self) -> usize {
        match self.doc.get_map(MEDIA).get_deep_value() {
            LoroValue::Map(m) => m.len(),
            _ => 0,
        }
    }

    /// The `(w, h, mime)` load-info of a media entry, if present. Test accessor.
    #[cfg(test)]
    pub(super) fn media_meta(&self, hash: &str) -> Option<(i64, i64, String)> {
        let LoroValue::Map(m) = self.doc.get_map(MEDIA).get_deep_value() else {
            return None;
        };
        let LoroValue::Map(entry) = m.get(hash)? else {
            return None;
        };
        let w = match entry.get("w")? {
            LoroValue::I64(v) => *v,
            _ => return None,
        };
        let h = match entry.get("h")? {
            LoroValue::I64(v) => *v,
            _ => return None,
        };
        let mime = match entry.get("mime")? {
            LoroValue::String(s) => s.to_string(),
            _ => return None,
        };
        Some((w, h, mime))
    }
}

#[cfg(test)]
mod tests {
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
            fill: Some(0xFF_3366CC),
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
        let fills: Vec<Option<u32>> = vp
            .nodes
            .iter()
            .filter_map(|n| match n {
                crate::logic::command::DisplayNode::Element(e) => Some(e.style.fill),
                _ => None,
            })
            .collect();
        assert_eq!(fills, vec![Some(0xFF_3366CC)]);
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
            mime: mime.to_owned(),
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
        });
        // No media -> applied synchronously, one CRDT delta, no service request.
        assert!(out.update.is_some());
        assert!(out.requests.is_empty());
        assert!(has_report(&out), "an ImportReport is surfaced");
        assert_eq!(st.element_count(), 1);
        assert_eq!(st.pending_import_count(), 0);
    }

    #[test]
    fn a_broken_file_fails_without_touching_the_board() {
        let mut st = ToileState::new();
        let out = st.handle_command(ToileCommand::ImportSvg {
            bytes: b"this is not svg".to_vec(),
        });
        assert!(
            out.display
                .iter()
                .any(|e| matches!(e, ToileEvent::ImportFailed(_)))
        );
        assert!(out.update.is_none());
        assert_eq!(st.element_count(), 0);
    }

    #[test]
    fn image_import_defers_until_media_uploads() {
        let mut st = ToileState::new();
        let out = st.handle_command(ToileCommand::ImportImage {
            bytes: solid_png(8, 8),
            mime: "image/png".to_owned(),
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
                content_hash: "realhash".to_owned(),
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

    #[test]
    fn a_failed_upload_aborts_the_pending_import() {
        let mut st = ToileState::new();
        let out = st.handle_command(ToileCommand::ImportImage {
            bytes: solid_png(8, 8),
            mime: "image/png".to_owned(),
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
                content_hash: "orphan".to_owned(),
            },
        );
        assert!(out.display.is_empty());
        assert!(out.update.is_none());
        assert_eq!(st.element_count(), 0);
    }
}
