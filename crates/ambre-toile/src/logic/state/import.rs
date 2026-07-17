//! Atomic foreign-file import into [`ToileState`](super::ToileState)
//! (`toile-interop` §1/§5). An [`ImportBatch`](crate::logic::interop::ImportBatch)
//! produced by a pure converter is applied here as ONE undo group: every
//! `CreateElement`, style, group, connector and media seed shares a single
//! `doc.commit()` via the commit-free `*_inner` helpers (§2.E). The pass order
//! (creates -> styles -> groups -> connectors -> media -> one commit ->
//! `rebuild_caches`) resolves every slot / group reference before it is used.

use loro::{LoroMap, LoroValue};

use crate::logic::command::{ToileCommand, ToileOutcome};
use crate::logic::interop::{ImportBatch, ImportRef, ImportReport, MediaBlob};
use crate::logic::model::ElementId;

use super::{MEDIA, ToileState};

impl ToileState {
    /// Apply a converted [`ImportBatch`] as ONE atomic undo group
    /// (`toile-interop` §1/§2.E). The pass is strictly ordered so every slot /
    /// group reference resolves before it is used:
    ///
    /// 1. **creates** -> `create_element_inner`, minting an [`ElementId`] per slot;
    /// 2. **styles** -> `restyle_inner` on the slot's fresh element (style is an
    ///    independent LWW register, §2.E, so it cannot ride the create);
    /// 3. **groups** -> `group_inner`, innermost-first, so a `Group(i)` member
    ///    always resolves to an already-built group (§2.A);
    /// 4. **connectors** -> `create_connector_inner`, endpoints resolved by ref;
    /// 5. **media** -> `seed_media` (dedup, §5);
    /// 6. a single `doc.commit()` + one `rebuild_caches`.
    ///
    /// Returns the finalized [`ImportReport`] (its `imported` count is the number
    /// of elements actually created) and a [`ToileOutcome`] carrying the one CRDT
    /// delta plus the repaint viewport. A create that unexpectedly fails (a
    /// trusted converter should never emit one) is recorded in the report and its
    /// slot left unresolved, so any group / connector referencing it is skipped.
    // The production caller (the import command handler) lands in step 6/8;
    // exercised by tests meanwhile.
    #[allow(dead_code)]
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
                } => match self.create_element_inner(kind, at, size, content) {
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
                // (bounds-keyed) before the final `rebuild_caches` (§2.F bis).
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
    /// (`toile-logic §2.B`), **without** committing. Each blob's `hash` (the real
    /// blake2b, resolved Sync-side by the time this runs) keys a `{w, h, mime,
    /// thumb}` map. Dedup is content-addressed (§5): a hash already present is
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
                // No real hash yet, or the entry already exists (dedup, §5).
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
        // Two blobs with the same content hash collapse to one entry (§5 dedup).
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
}
