// ambre-trame/src/logic/edit_anchor.rs
// The open edit session's cell, held by identity (`trame-cell-editing`).
//
// Part of the `TrameState` impl. Submodule of `logic`, so it reaches the private
// fields/helpers of the parent module through the `use super::*` glob.

use super::*;

/// One axis of the cell an edit session opened on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Line {
    /// A materialized line, followed wherever a peer moves it.
    At(Uid),
    /// A line past the used range, `offset` lines after the last materialized
    /// one, or at `offset` itself when the axis had none. A line a peer appends
    /// past `last` moves nothing, so the position holds.
    Past { last: Option<Uid>, offset: usize },
}

impl Line {
    /// Pin grid position `pos` on an axis holding `len` materialized lines.
    fn pin(pos: usize, len: usize, id_at: impl Fn(usize) -> Option<Uid>) -> Self {
        match id_at(pos) {
            Some(id) => Self::At(id),
            None if len == 0 => Self::Past {
                last: None,
                offset: pos,
            },
            None => Self::Past {
                last: id_at(len - 1),
                offset: pos - (len - 1),
            },
        }
    }

    /// The line's position now, or `None` when a peer deleted it.
    ///
    /// A `Past` line whose `last` was deleted keeps its distance from the end
    /// of the used range. That is exact unless a peer also appended lines.
    fn locate(self, len: usize, pos_of: impl Fn(&Uid) -> Option<usize>) -> Option<usize> {
        match self {
            Self::At(id) => pos_of(&id),
            Self::Past { last: None, offset } => Some(offset),
            Self::Past {
                last: Some(id),
                offset,
            } => Some(pos_of(&id).map_or(len + offset - 1, |p| p + offset)),
        }
    }
}

/// The cell an open edit session writes to (`TRAME-UI-160`).
#[derive(Debug, Clone)]
pub(super) struct EditAnchor {
    /// The UI's token for the session, echoed in every event about it.
    session: u64,
    /// The sheet the session opened on.
    sheet: String,
    row: Line,
    col: Line,
    /// The position the UI last heard of, so a move is reported once.
    at: (u32, u32),
}

impl TrameState {
    /// Open edit session `session` on `(row, col)` of the active sheet. Writes
    /// nothing (`TRAME-UI-158`).
    pub(super) fn begin_cell_edit(&mut self, session: u64, row: u32, col: u32) -> TrameOutcome {
        let index = &self.index;
        self.edit_anchor = Some(EditAnchor {
            session,
            sheet: self.active_sheet.clone(),
            row: Line::pin(row as usize, index.row_len(), |p| index.row_id(p)),
            col: Line::pin(col as usize, index.col_len(), |p| index.col_id(p)),
            at: (row, col),
        });
        TrameOutcome::display_only(vec![])
    }

    /// Close edit session `session` with no write. A no-op for another session.
    pub(super) fn end_cell_edit(&mut self, session: u64) -> TrameOutcome {
        if self
            .edit_anchor
            .as_ref()
            .is_some_and(|a| a.session == session)
        {
            self.edit_anchor = None;
        }
        TrameOutcome::display_only(vec![])
    }

    /// Commit edit session `session` with `text`, to the cell it opened on
    /// wherever that cell now stands (`TRAME-UI-160`, `TRAME-UI-162`). A session
    /// this state no longer holds writes nothing (`TRAME-UI-176`).
    pub(super) fn commit_cell_edit(&mut self, session: u64, text: &str) -> TrameOutcome {
        let Some(anchor) = self.edit_anchor.take_if(|a| a.session == session) else {
            // Dropped when its cell went, and the UI was told then.
            return TrameOutcome::display_only(vec![]);
        };
        if anchor.sheet == self.active_sheet {
            return self.write_at_anchor(&anchor, text);
        }
        if !self.all_sheet_ids().contains(&anchor.sheet) {
            return TrameOutcome::display_only(vec![TrameEvent::EditTargetGone { session }]);
        }
        // The reader switched tabs before the commit arrived. The text still
        // lands on the session's own sheet, then the reader's tab comes back.
        let reading = std::mem::replace(&mut self.active_sheet, anchor.sheet.clone());
        self.reseed_active_sheet();
        let mut out = self.write_at_anchor(&anchor, text);
        self.active_sheet = reading;
        out.display.extend(self.reseed_active_sheet());
        out
    }

    /// Write `text` to the anchored cell of the active sheet, or report it gone.
    fn write_at_anchor(&mut self, anchor: &EditAnchor, text: &str) -> TrameOutcome {
        let Some((row, col)) = self.locate_anchor(anchor) else {
            let session = anchor.session;
            return TrameOutcome::display_only(vec![TrameEvent::EditTargetGone { session }]);
        };
        // The cell write and its derived commits are one undo item, which the
        // next `CommitEdit` seals.
        let _ = self.undo_manager.group_start();
        self.write_cell(row, col, text);
        self.finish_cell_writes()
    }

    /// Where the anchored cell stands now on the active sheet, or `None` when a
    /// peer deleted its row or its column. The caller checks the sheet.
    fn locate_anchor(&self, anchor: &EditAnchor) -> Option<(u32, u32)> {
        let index = &self.index;
        let row = anchor.row.locate(index.row_len(), |id| index.row_pos(id))?;
        let col = anchor.col.locate(index.col_len(), |id| index.col_pos(id))?;
        Some((u32::try_from(row).ok()?, u32::try_from(col).ok()?))
    }

    /// Follow the open session's cell after a merge: report where a peer moved
    /// it, or drop the session when a peer deleted it (`TRAME-UI-176`).
    pub(super) fn follow_edit_anchor(&mut self) -> Option<TrameEvent> {
        let anchor = self.edit_anchor.as_ref()?;
        let session = anchor.session;
        let located = if anchor.sheet == self.active_sheet {
            self.locate_anchor(anchor)
        } else if self.all_sheet_ids().contains(&anchor.sheet) {
            // The reader is on another tab: the cell is not on screen to
            // follow, and the commit locates it on its own sheet.
            return None;
        } else {
            None
        };
        match located {
            None => {
                self.edit_anchor = None;
                Some(TrameEvent::EditTargetGone { session })
            }
            Some(at) if at != anchor.at => {
                if let Some(anchor) = self.edit_anchor.as_mut() {
                    anchor.at = at;
                }
                Some(TrameEvent::EditTargetMoved {
                    session,
                    row: at.0,
                    col: at.1,
                })
            }
            Some(_) => None,
        }
    }
}
