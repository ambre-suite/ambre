// ambre-trame/src/logic/metrics.rs
// Column-width / row-height data plane (`trame-resize`, `trame-engine`).
//
// Resize is a synchronised CRDT mutation: widths/heights live in the per-sheet
// `col_widths_{sheet}` / `row_heights_{sheet}` LWW maps keyed by the axis's
// stable `Uid`, so a resized column keeps its width across inserts/deletes and
// two peers resizing the same column converge by LWW (like any cell write). The
// derived `col_widths` / `row_heights` position caches feed the viewport
// response, which hands the UI a complete per-axis slice for its
// variable-metric `GridLayout`.
//
// Part of the `TrameState` impl; reaches the parent's fields/helpers through
// `use super::*`.

use super::*;

impl TrameState {
    /// Set column `col`'s width (physical px) on the active sheet, clamped to
    /// [`MIN_COL_WIDTH`]. Pins the column's identity (materializes it, like
    /// formatting a cell does) and writes its stable `Uid` -> width into
    /// `col_widths_{sheet}`. Undoable (a plain value commit under the default
    /// origin); replies with a [`TrameEvent::DataReset`] so the UI re-fetches the
    /// viewport with the new metrics. A no-op when `sheet_id` is not the active
    /// sheet (the index/caches only track the active sheet -- the MVP UI never
    /// resizes a background sheet).
    pub(super) fn resize_col(&mut self, sheet_id: &str, col: u32, width_px: f32) -> TrameOutcome {
        if sheet_id != self.active_sheet {
            return TrameOutcome::display_only(vec![]);
        }
        let width = width_px.max(MIN_COL_WIDTH);
        // Materialize the column to mint/resolve its stable id (materialize keys
        // on a cell; row 0 is the cheapest anchor). Beyond the cap: drop.
        let Some((_, col_uid)) = self.materialize(0, col) else {
            return TrameOutcome::display_only(vec![]);
        };
        let _ = self
            .doc
            .get_map(col_widths_map_name(&self.active_sheet))
            .insert(&col_uid.to_key_part(), width as f64);
        self.doc.commit();
        self.col_widths.insert(col, width);
        let update = self.flush_delta();
        TrameOutcome {
            display: vec![TrameEvent::DataReset],
            update,
            ephemeral: None,
        }
    }

    /// Set row `row`'s height (physical px) on the active sheet, clamped to
    /// [`MIN_ROW_HEIGHT`]. Row analogue of [`resize_col`](Self::resize_col):
    /// writes `row_heights_{sheet}`.
    pub(super) fn resize_row(&mut self, sheet_id: &str, row: u32, height_px: f32) -> TrameOutcome {
        if sheet_id != self.active_sheet {
            return TrameOutcome::display_only(vec![]);
        }
        let height = height_px.max(MIN_ROW_HEIGHT);
        let Some((row_uid, _)) = self.materialize(row, 0) else {
            return TrameOutcome::display_only(vec![]);
        };
        let _ = self
            .doc
            .get_map(row_heights_map_name(&self.active_sheet))
            .insert(&row_uid.to_key_part(), height as f64);
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
            .get_map(col_widths_map_name(&self.active_sheet))
            .get_value()
        {
            for (key, value) in widths.iter() {
                let (Some(uid), LoroValue::Double(w)) = (Uid::parse(key), value) else {
                    continue;
                };
                if let Some(pos) = self.index.col_pos(&uid) {
                    self.col_widths.insert(pos as u32, *w as f32);
                }
            }
        }
        if let LoroValue::Map(heights) = self
            .doc
            .get_map(row_heights_map_name(&self.active_sheet))
            .get_value()
        {
            for (key, value) in heights.iter() {
                let (Some(uid), LoroValue::Double(h)) = (Uid::parse(key), value) else {
                    continue;
                };
                if let Some(pos) = self.index.row_pos(&uid) {
                    self.row_heights.insert(pos as u32, *h as f32);
                }
            }
        }
    }

    /// Width (physical px) of column `col` on the active sheet: its resized value
    /// or the uniform [`DEFAULT_COL_WIDTH`]. Used to pack the viewport slice.
    pub(super) fn col_width(&self, col: u32) -> f32 {
        self.col_widths
            .get(&col)
            .copied()
            .unwrap_or(DEFAULT_COL_WIDTH)
    }
    /// Height (physical px) of row `row`: its resized value or the uniform
    /// [`DEFAULT_ROW_HEIGHT`].
    pub(super) fn row_height(&self, row: u32) -> f32 {
        self.row_heights
            .get(&row)
            .copied()
            .unwrap_or(DEFAULT_ROW_HEIGHT)
    }
}
