//! The frozen pane a sheet stores (`features/ambre-spec-trame-grid-viewport.md
//! section 5`).
//!
//! `frozen_{sheet}` holds one key per axis: the count of leading rows, and
//! the count of leading columns, that stay on screen while the rest scrolls.
//! A key that is absent, or that holds a value no build writes, freezes
//! nothing on its axis. A write touches the axis it changes alone, so two
//! peers that freeze rows and columns at once keep both (`TRAME-UI-191`).

use loro::LoroValue;

use super::schema::{Fields, FrozenKey, SheetContainer, SheetRoot};
use super::*;
use crate::message::{FreezeIntent, FrozenPane};
use crate::{TOTAL_COLS, TOTAL_ROWS};
use ambre_core::index::{Axis, Len};

/// A stored count, bounded so the frozen band leaves at least one line of
/// its axis to scroll in a grid of `total` lines. A value that is no whole
/// number of zero or more reads as zero.
fn read_count<A: Axis>(value: Option<LoroValue>, total: Len<A>) -> Len<A> {
    let Some(LoroValue::I64(count)) = value else {
        return Len::<A>::ZERO;
    };
    let count = u32::try_from(count.max(0)).unwrap_or(u32::MAX);
    Len::<A>::new(count).min(total.saturating_sub(Len::<A>::ONE))
}

impl FrozenPane {
    /// The pane cut to the grid: each band leaves one line of its axis to
    /// scroll.
    #[must_use]
    pub(crate) fn within_grid(self) -> Self {
        Self {
            rows: self.rows.min(TOTAL_ROWS.saturating_sub(Len::<Row>::ONE)),
            cols: self.cols.min(TOTAL_COLS.saturating_sub(Len::<Col>::ONE)),
        }
    }
}

impl TrameState {
    /// The map that holds the frozen pane of `sheet`.
    fn frozen_fields(&self, sheet: &Id<Sheet>) -> Fields<FrozenKey> {
        Fields::new(
            self.doc
                .get_map(SheetContainer::of(SheetRoot::Frozen, sheet).as_str()),
        )
    }

    /// The frozen pane `sheet` stores. An absent or unreadable key freezes
    /// nothing on its axis.
    #[must_use]
    pub(crate) fn frozen_pane(&self, sheet: &Id<Sheet>) -> FrozenPane {
        let fields = self.frozen_fields(sheet);
        FrozenPane {
            rows: read_count(fields.value(FrozenKey::Rows), TOTAL_ROWS),
            cols: read_count(fields.value(FrozenKey::Cols), TOTAL_COLS),
        }
    }

    /// Run `intent` against the active cell of the active sheet
    /// (`TRAME-UI-029`).
    pub(super) fn freeze(&mut self, intent: FreezeIntent) -> TrameOutcome {
        let sheet = self.active_sheet.clone();
        let stored = self.frozen_pane(&sheet);
        let next = match intent {
            FreezeIntent::RowsThroughActive => FrozenPane {
                rows: Len::through(self.active.row),
                ..stored
            },
            FreezeIntent::ColsThroughActive => FrozenPane {
                cols: Len::through(self.active.col),
                ..stored
            },
            FreezeIntent::Clear => FrozenPane::NONE,
        };
        self.set_frozen_pane(&sheet, next)
    }

    /// Write `pane`, cut to the grid, into the frozen pane map of `sheet`,
    /// with no commit. Only the axes that change are written, and an axis
    /// that freezes nothing is deleted: absence is the value
    /// (`ambre-spec-trame-schema.md section 3`).
    pub(super) fn write_frozen_pane(&self, sheet: &Id<Sheet>, pane: FrozenPane) {
        let next = pane.within_grid();
        let stored = self.frozen_pane(sheet);
        // A Loro map refuses a write only on a detached handler. A root map
        // is never detached, so the result is dropped, as the page setup does.
        let fields = self.frozen_fields(sheet);
        let write = |key: FrozenKey, count: u32, before: u32| {
            if count == before {
                return;
            }
            let _ = if count == 0 {
                fields.delete(key)
            } else {
                fields.insert(key, i64::from(count))
            };
        };
        write(FrozenKey::Rows, next.rows.get(), stored.rows.get());
        write(FrozenKey::Cols, next.cols.get(), stored.cols.get());
    }

    /// Store `pane` as the frozen pane of `sheet`, cut to the grid. One
    /// commit, so one undo item.
    pub(super) fn set_frozen_pane(&mut self, sheet: &Id<Sheet>, pane: FrozenPane) -> TrameOutcome {
        let next = pane.within_grid();
        let stored = self.frozen_pane(sheet);
        if next == stored {
            return TrameOutcome::display_only(vec![]);
        }
        self.write_frozen_pane(sheet, next);
        self.doc.commit();
        let update = self.flush_delta();
        // The window re-fetches, and its answer carries the new pane.
        let display = if *sheet == self.active_sheet {
            vec![TrameEvent::DataReset]
        } else {
            vec![]
        };
        TrameOutcome {
            display,
            update,
            ephemeral: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::FreezeIntent;
    use crate::{TrameCommand, TrameEvent};

    /// Move the active cell of `state` to `(row, col)`.
    fn select(state: &mut TrameState, row: u32, col: u32) {
        let (row, col) = (Pos::<Row>::new(row), Pos::<Col>::new(col));
        state.handle_command(TrameCommand::SetActiveCell {
            row,
            col,
            anchor_row: row,
            anchor_col: col,
            extra: vec![],
            kind: crate::message::SelectionKind::Cells,
        });
    }

    /// Run `intent` on `state` with the active cell at `(row, col)`, and
    /// return the delta it wrote.
    fn freeze(
        state: &mut TrameState,
        row: u32,
        col: u32,
        intent: FreezeIntent,
    ) -> ambre_core::Payload<ambre_core::DeltaBytes> {
        select(state, row, col);
        state
            .handle_command(TrameCommand::Freeze(intent))
            .update
            .expect("a freeze writes the document")
    }

    fn pane(rows: u32, cols: u32) -> FrozenPane {
        FrozenPane {
            rows: Len::<Row>::new(rows),
            cols: Len::<Col>::new(cols),
        }
    }

    /// The answer of `state` to a request for rows `rows` and columns `cols`.
    fn answer(
        state: &mut TrameState,
        rows: (u32, u32),
        cols: (u32, u32),
    ) -> (Vec<CellPos>, crate::message::FrozenView) {
        let outcome = state.handle_command(TrameCommand::ScrollViewport {
            version: ambre_core::Seq::nth(1),
            row_start: Pos::<Row>::new(rows.0),
            row_end: Pos::<Row>::new(rows.1),
            col_start: Pos::<Col>::new(cols.0),
            col_end: Pos::<Col>::new(cols.1),
        });
        outcome
            .display
            .into_iter()
            .find_map(|event| match event {
                TrameEvent::ViewportData { cells, frozen, .. } => Some((
                    cells.iter().map(|c| CellPos::new(c.row, c.col)).collect(),
                    frozen,
                )),
                _ => None,
            })
            .expect("a viewport answer")
    }

    /// `TRAME-UI-029`: a freeze is stored in the document. A peer that
    /// applies the delta reads the same pane, and its viewport answer
    /// carries it. One undo takes it back.
    #[test]
    fn a_freeze_reaches_a_peer_and_one_undo_takes_it_back() {
        let mut a = TrameState::new();
        let mut b = TrameState::new();
        b.import_snapshot(&a.export_snapshot())
            .expect("a snapshot this build wrote");
        let rows = freeze(&mut a, 1, 4, FreezeIntent::RowsThroughActive);
        let cols = freeze(&mut a, 7, 2, FreezeIntent::ColsThroughActive);
        b.apply_remote(&rows);
        b.apply_remote(&cols);
        let sheet = b.active_sheet.clone();
        assert_eq!(b.frozen_pane(&sheet), pane(2, 3));
        assert_eq!(answer(&mut b, (40, 60), (20, 30)).1.pane, pane(2, 3));
        let undo = a
            .handle_command(TrameCommand::Undo)
            .update
            .expect("an undo writes the document");
        b.apply_remote(&undo);
        assert_eq!(a.frozen_pane(&a.active_sheet.clone()), pane(2, 0));
        assert_eq!(b.frozen_pane(&sheet), pane(2, 0));
        let clear = freeze(&mut a, 0, 0, FreezeIntent::Clear);
        b.apply_remote(&clear);
        assert_eq!(b.frozen_pane(&sheet), FrozenPane::NONE);
        assert!(
            b.doc
                .get_map(SheetContainer::of(SheetRoot::Frozen, &sheet).as_str())
                .is_empty(),
            "a pane that freezes nothing leaves no key"
        );
    }

    /// `TRAME-UI-194`: a freeze writes the frozen pane map of the active
    /// sheet and nothing else of the document.
    #[test]
    fn a_freeze_writes_the_frozen_pane_alone() {
        let mut state = TrameState::new();
        state.handle_command(TrameCommand::CreateSheet {
            name: crate::test_support::sheet("Two"),
        });
        for (row, col) in [(0, 0), (4, 2)] {
            state.handle_command(TrameCommand::SetCellText {
                row: Pos::<Row>::new(row),
                col: Pos::<Col>::new(col),
                text: "kept".to_owned(),
            });
        }
        let pane_map = SheetContainer::of(SheetRoot::Frozen, &state.active_sheet.clone());
        let rest = |state: &TrameState| {
            let loro::LoroValue::Map(roots) = state.doc.get_deep_value() else {
                panic!("a document is a map of roots");
            };
            let mut roots = (*roots).clone();
            roots.remove(pane_map.as_str());
            roots
        };
        let before = rest(&state);
        freeze(&mut state, 3, 1, FreezeIntent::RowsThroughActive);
        freeze(&mut state, 3, 1, FreezeIntent::ColsThroughActive);
        assert_eq!(state.frozen_pane(&state.active_sheet.clone()), pane(4, 2));
        assert_eq!(rest(&state), before);
    }

    /// `TRAME-UI-191`: one peer freezes rows while another freezes columns.
    /// Both peers end on the pane that holds both bands.
    #[test]
    fn rows_and_columns_frozen_at_once_both_hold() {
        let mut a = TrameState::new();
        let mut b = TrameState::new();
        b.import_snapshot(&a.export_snapshot())
            .expect("a snapshot this build wrote");
        let rows = freeze(&mut a, 2, 0, FreezeIntent::RowsThroughActive);
        let cols = freeze(&mut b, 0, 1, FreezeIntent::ColsThroughActive);
        a.apply_remote(&cols);
        b.apply_remote(&rows);
        let sheet = a.active_sheet.clone();
        assert_eq!(a.frozen_pane(&sheet), pane(3, 2));
        assert_eq!(b.frozen_pane(&sheet), pane(3, 2));
    }

    /// `TRAME-UI-192`, the Logic half: an answer for a window far from the
    /// first line holds the cells of each frozen band over the window. It
    /// also carries the sizes of the frozen lines before the window.
    #[test]
    fn an_answer_far_from_the_top_carries_the_frozen_bands() {
        let mut state = TrameState::new();
        for (row, col) in [(0, 0), (0, 25), (50, 0), (50, 25), (1, 9), (30, 25)] {
            state.handle_command(TrameCommand::SetCellText {
                row: Pos::<Row>::new(row),
                col: Pos::<Col>::new(col),
                text: format!("r{row}c{col}"),
            });
        }
        freeze(&mut state, 0, 0, FreezeIntent::RowsThroughActive);
        freeze(&mut state, 0, 0, FreezeIntent::ColsThroughActive);
        let (cells, frozen) = answer(&mut state, (40, 60), (20, 30));
        let at = |row, col| CellPos::new(Pos::<Row>::new(row), Pos::<Col>::new(col));
        for kept in [at(0, 0), at(0, 25), at(50, 0), at(50, 25)] {
            assert!(cells.contains(&kept), "{kept:?} is in the answer");
        }
        for left in [at(1, 9), at(30, 25)] {
            assert!(!cells.contains(&left), "{left:?} is outside every band");
        }
        assert_eq!(frozen.pane, pane(1, 1));
        assert_eq!((frozen.row_heights.len(), frozen.col_widths.len()), (1, 1));
    }

    /// A stored count past the grid reads cut to it, so one line of each
    /// axis is left to scroll.
    #[test]
    fn a_stored_count_past_the_grid_leaves_a_line_to_scroll() {
        let state = TrameState::new();
        let sheet = state.active_sheet.clone();
        let fields = state.frozen_fields(&sheet);
        let _ = fields.insert(FrozenKey::Rows, i64::MAX);
        let _ = fields.insert(FrozenKey::Cols, -4_i64);
        assert_eq!(
            state.frozen_pane(&sheet),
            FrozenPane {
                rows: TOTAL_ROWS.saturating_sub(Len::<Row>::ONE),
                cols: Len::<Col>::ZERO,
            }
        );
    }
}
