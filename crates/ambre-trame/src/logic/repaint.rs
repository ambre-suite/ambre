// ambre-trame/src/logic/repaint.rs
// What a cell write or a remote batch asks the view to repaint (`TRAME-284`)
//
// Part of the `TrameState` impl. Submodule of `logic`, so it reaches the
// private fields of the parent module through `use super::*`.
//
// A change that reached a known set of cells names them as one rectangle. The
// view re-fetches the part of its window the rectangle covers. A
// change the state rebuilt every cache for asks for the whole window.

use super::*;
use crate::formula::ACTIVE_SHEET;

/// What the view repaints after a change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Repaint {
    /// The whole window: the engine reseeded, or a cache of the whole sheet
    /// moved.
    Whole,
    /// The cells of the rectangle alone, or nothing when no cell changed.
    Cells(Option<CellRect>),
}

impl Repaint {
    /// The repaint that covers both `self` and the cell `at`.
    pub(super) fn with(self, at: CellPos) -> Self {
        match self {
            Self::Whole => Self::Whole,
            Self::Cells(None) => Self::Cells(Some(CellRect::spanning(at, at))),
            Self::Cells(Some(rect)) => Self::Cells(Some(rect.union(CellRect::spanning(at, at)))),
        }
    }

    /// The repaint that covers both `self` and the rectangle `rect`.
    pub(super) fn with_rect(self, rect: CellRect) -> Self {
        match self {
            Self::Whole => Self::Whole,
            Self::Cells(None) => Self::Cells(Some(rect)),
            Self::Cells(Some(held)) => Self::Cells(Some(held.union(rect))),
        }
    }
}

/// Every cell of the grid.
pub(super) fn whole_sheet() -> CellRect {
    CellRect::spanning(
        CellPos::ORIGIN,
        CellPos::new(crate::LAST_ROW, crate::LAST_COL),
    )
}

impl TrameState {
    /// The cells of the active sheet the engine took or computed since the
    /// set was last cleared, added to `repaint`. The set is cleared. A cell
    /// on a gone line places nowhere and is left out: the line's removal is
    /// what repaints it.
    pub(super) fn take_engine_changes(&mut self, repaint: Repaint) -> Repaint {
        let changed = std::mem::take(&mut self.engine.changed);
        changed
            .into_iter()
            .filter(|cell| cell.sheet == ACTIVE_SHEET)
            .filter_map(|cell| {
                let row = match cell.row {
                    AxisId::Bound(uid) => self.index.row_pos(&uid)?,
                    AxisId::Virtual(pos) => pos,
                };
                let col = match cell.col {
                    AxisId::Bound(uid) => self.index.col_pos(&uid)?,
                    AxisId::Virtual(pos) => pos,
                };
                Some(CellPos::new(row, col))
            })
            .fold(repaint, Repaint::with)
    }

    /// The event that carries `repaint` to the view, or `None` when nothing
    /// changed.
    pub(super) fn repaint_event(&self, repaint: Repaint) -> Option<TrameEvent> {
        match repaint {
            Repaint::Whole => Some(TrameEvent::DataReset),
            Repaint::Cells(None) => None,
            Repaint::Cells(Some(range)) => Some(TrameEvent::CellsChanged {
                version: self.served_version,
                range,
            }),
        }
    }
}
