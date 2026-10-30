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

/// The size range of one axis, and the size of a line never resized.
#[derive(Clone, Copy)]
struct AxisSizes {
    default: DocPx,
    min: DocPx,
    max: DocPx,
}

const COL_SIZES: AxisSizes = AxisSizes {
    default: COL_WIDTH,
    min: MIN_COL_WIDTH,
    max: MAX_COL_WIDTH,
};

const ROW_SIZES: AxisSizes = AxisSizes {
    default: ROW_HEIGHT,
    min: MIN_ROW_HEIGHT,
    max: MAX_ROW_HEIGHT,
};

impl AxisSizes {
    /// Clamp a requested size into the range. A non-finite request takes the
    /// default size.
    fn clamp(self, size: DocPx) -> DocPx {
        if size.is_finite() {
            size.clamp(self.min, self.max)
        } else {
            self.default
        }
    }

    /// Read a stored size. Any peer writes this map, so a size out of range
    /// reads clamped and a non-finite one reads as the default (`TRAME-UI-181`):
    /// the layout walks lines until their sizes cover the screen. `None` when
    /// the value is no number.
    fn read(self, value: &LoroValue) -> Option<DocPx> {
        let LoroValue::Double(size) = value else {
            return None;
        };
        // Clamp before the cast: a finite `f64` past the `f32` range would
        // cast to an infinity and read as the default, not as the bound.
        if !size.is_finite() {
            return Some(self.default);
        }
        let size = size.clamp(f64::from(self.min.get()), f64::from(self.max.get()));
        Some(DocPx::new(size as f32))
    }
}

impl TrameState {
    /// Set column `col`'s width (logical px) on the active sheet, clamped to
    /// [`MIN_COL_WIDTH`] ..= [`MAX_COL_WIDTH`]. Pins the column's identity (materializes it, like
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
        let width = COL_SIZES.clamp(width);
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
    /// [`MIN_ROW_HEIGHT`] ..= [`MAX_ROW_HEIGHT`]. Row analogue of [`resize_col`](Self::resize_col):
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
        let height = ROW_SIZES.clamp(height);
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
                let Some(uid) = Uid::parse(key) else {
                    // A key no build writes is said (`TRAME-248`).
                    self.note_unreadable_key();
                    continue;
                };
                let Some(width) = COL_SIZES.read(value) else {
                    continue;
                };
                if let Some(pos) = self.index.col_pos(&uid) {
                    self.col_widths.insert(pos, width);
                }
            }
        }
        if let LoroValue::Map(heights) = self
            .doc
            .get_map(SheetContainer::of(SheetRoot::RowHeights, &self.active_sheet).as_str())
            .get_value()
        {
            for (key, value) in heights.iter() {
                let Some(uid) = Uid::parse(key) else {
                    // A key no build writes is said (`TRAME-248`).
                    self.note_unreadable_key();
                    continue;
                };
                let Some(height) = ROW_SIZES.read(value) else {
                    continue;
                };
                if let Some(pos) = self.index.row_pos(&uid) {
                    self.row_heights.insert(pos, height);
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
