// ambre-trame/src/logic/metrics.rs Column-width / row-height data plane
// (`trame-resize`, `trame-engine`).
//
// Resize is a synchronised CRDT mutation. Widths/heights live in the per-sheet
// `col_widths_{sheet}` / `row_heights_{sheet}` LWW maps, keyed by the axis's
// stable `Uid`. A resized column then keeps its width across inserts/deletes,
// and two peers resizing the same column converge by LWW, like any cell write.
// The derived `col_widths` / `row_heights` position caches feed the viewport
// response, which hands the UI a complete per-axis slice for its
// variable-metric `GridLayout`.
//
// Part of the `TrameState` impl; reaches the parent's fields/helpers through
// `use super::*`.

use super::*;

impl TrameState {
    /// Set column `col`'s width (logical px) on the active sheet, clamped to
    /// [`MIN_COL_WIDTH`]. Pins the column's identity (materializes it, like
    /// formatting a cell does) and writes its stable `Uid` -> width into
    /// `col_widths_{sheet}`. Undoable (a plain value commit under the default
    /// origin); replies with a [`TrameEvent::DataReset`] so the UI re-fetches the
    /// viewport with the new metrics. A no-op when `sheet_id` is not the active
    /// sheet (the index/caches only track the active sheet -- the MVP UI never
    /// resizes a background sheet).
    pub(super) fn resize_col(
        &mut self,
        sheet_id: &SheetId,
        col: Col,
        width: DocPx,
    ) -> TrameOutcome {
        if *sheet_id != self.active_sheet {
            return TrameOutcome::display_only(vec![]);
        }
        let width = width.max(MIN_COL_WIDTH);
        // Materialize the column to mint/resolve its stable id (materialize keys
        // on a cell; row 0 is the cheapest anchor). Refused: say why.
        let col_uid = match self.materialize(CellPos::new(Row::ZERO, col)) {
            Ok((_, uid)) => uid,
            Err(why) => return TrameOutcome::display_only(vec![why.notice()]),
        };
        let _ = self
            .doc
            .get_map(SheetContainer::of(SheetRoot::ColWidths, &self.active_sheet).as_str())
            .insert(&col_uid.to_key_part(), f64::from(width.get()));
        self.doc.commit();
        self.col_widths.insert(col, width);
        let update = self.flush_delta();
        TrameOutcome {
            display: vec![TrameEvent::DataReset],
            update,
            ephemeral: None,
        }
    }

    /// Set row `row`'s height (logical px) on the active sheet, clamped to
    /// [`MIN_ROW_HEIGHT`]. Row analogue of [`resize_col`](Self::resize_col):
    /// writes `row_heights_{sheet}`.
    pub(super) fn resize_row(
        &mut self,
        sheet_id: &SheetId,
        row: Row,
        height: DocPx,
    ) -> TrameOutcome {
        if *sheet_id != self.active_sheet {
            return TrameOutcome::display_only(vec![]);
        }
        let height = height.max(MIN_ROW_HEIGHT);
        let row_uid = match self.materialize(CellPos::new(row, Col::ZERO)) {
            Ok((uid, _)) => uid,
            Err(why) => return TrameOutcome::display_only(vec![why.notice()]),
        };
        let _ = self
            .doc
            .get_map(SheetContainer::of(SheetRoot::RowHeights, &self.active_sheet).as_str())
            .insert(&row_uid.to_key_part(), f64::from(height.get()));
        self.doc.commit();
        self.row_heights.insert(row, height);
        let update = self.flush_delta();
        TrameOutcome {
            display: vec![TrameEvent::DataReset],
            update,
            ephemeral: None,
        }
    }

    /// Rebuild the in-memory [`col_widths`](Self::col_widths) /
    /// [`row_heights`](Self::row_heights) position caches from the active sheet's
    /// `col_widths_{sheet}` / `row_heights_{sheet}` maps. Keys are stable `Uid`s,
    /// mapped back to grid positions through [`Self::index`] (which must be
    /// current); an entry for a deleted column/row is skipped. Called from the
    /// reseed chain after any import or structural op.
    pub(super) fn reseed_metrics(&mut self) {
        self.col_widths.clear();
        self.row_heights.clear();
        if let LoroValue::Map(widths) = self
            .doc
            .get_map(SheetContainer::of(SheetRoot::ColWidths, &self.active_sheet).as_str())
            .get_value()
        {
            for (key, value) in widths.iter() {
                let (Some(uid), LoroValue::Double(w)) = (Uid::parse(key), value) else {
                    continue;
                };
                if let Some(pos) = self.index.col_pos(&uid) {
                    self.col_widths.insert(pos, DocPx::new(*w as f32));
                }
            }
        }
        if let LoroValue::Map(heights) = self
            .doc
            .get_map(SheetContainer::of(SheetRoot::RowHeights, &self.active_sheet).as_str())
            .get_value()
        {
            for (key, value) in heights.iter() {
                let (Some(uid), LoroValue::Double(h)) = (Uid::parse(key), value) else {
                    continue;
                };
                if let Some(pos) = self.index.row_pos(&uid) {
                    self.row_heights.insert(pos, DocPx::new(*h as f32));
                }
            }
        }
    }

    /// Width (logical px) of column `col` on the active sheet: its resized value
    /// or the uniform [`COL_WIDTH`]. Used to pack the viewport slice.
    pub(super) fn col_width(&self, col: Col) -> DocPx {
        self.col_widths.get(&col).copied().unwrap_or(COL_WIDTH)
    }
    /// Height (logical px) of row `row`: its resized value or the uniform
    /// [`ROW_HEIGHT`].
    pub(super) fn row_height(&self, row: Row) -> DocPx {
        self.row_heights.get(&row).copied().unwrap_or(ROW_HEIGHT)
    }
}
