// ambre-trame/src/logic/edit_anchor.rs
// The open edit session's cell, held by identity (`trame-cell-editing`).
//
// Part of the `TrameState` impl. Submodule of `logic`, so it reaches the private
// fields/helpers of the parent module through the `use super::*` glob.

use super::*;

/// One axis of the cell an edit session opened on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pin<P: Line> {
    /// A materialized line, followed wherever a peer moves it.
    At(Uid),
    /// A line past the used range, `offset` lines after the last materialized
    /// one, or at `offset` itself when the axis had none. A line a peer appends
    /// past `last` moves nothing, so the position holds.
    Past { last: Option<Uid>, offset: P::Count },
}

impl<P: Line> Pin<P> {
    /// Pin grid position `pos` of `index`.
    fn new(index: &SheetIndex, pos: P) -> Self {
        if let Some(id) = P::uid(index, pos) {
            return Self::At(id);
        }
        match P::last(index) {
            None => Self::Past {
                last: None,
                offset: pos.count_from(P::FIRST),
            },
            Some(last) => Self::Past {
                last: P::uid(index, last),
                offset: pos.count_from(last),
            },
        }
    }

    /// The line's position now, or `None` when a peer deleted it.
    ///
    /// A `Past` line whose `last` was deleted keeps its distance from the end
    /// of the used range. That is exact unless a peer also appended lines.
    fn locate(self, index: &SheetIndex) -> Option<P> {
        match self {
            Self::At(id) => P::pos(index, &id),
            Self::Past { last: None, offset } => Some(P::FIRST.forward(offset)),
            Self::Past {
                last: Some(id),
                offset,
            } => match P::pos(index, &id) {
                Some(p) => Some(p.forward(offset)),
                None => P::end(index).forward(offset).before(),
            },
        }
    }
}

/// The cell an open edit session writes to (`TRAME-UI-160`).
#[derive(Debug, Clone)]
pub(super) struct EditAnchor {
    /// The UI's token for the session, echoed in every event about it.
    session: crate::EditSessionId,
    /// The sheet the session opened on.
    sheet: SheetId,
    row: Pin<Row>,
    col: Pin<Col>,
    /// The position the UI last heard of, so a move is reported once.
    at: CellPos,
}

impl TrameState {
    /// Open edit session `session` on cell `at` of the active sheet. Writes
    /// nothing (`TRAME-UI-158`).
    pub(super) fn begin_cell_edit(
        &mut self,
        session: crate::EditSessionId,
        at: CellPos,
    ) -> TrameOutcome {
        self.edit_anchor = Some(EditAnchor {
            session,
            sheet: self.active_sheet.clone(),
            row: Pin::new(&self.index, at.row),
            col: Pin::new(&self.index, at.col),
            at,
        });
        TrameOutcome::display_only(vec![])
    }

    /// Close edit session `session` with no write. A no-op for another session.
    pub(super) fn end_cell_edit(&mut self, session: crate::EditSessionId) -> TrameOutcome {
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
    pub(super) fn commit_cell_edit(
        &mut self,
        session: crate::EditSessionId,
        text: &FormulaSource,
    ) -> TrameOutcome {
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
    fn write_at_anchor(&mut self, anchor: &EditAnchor, text: &FormulaSource) -> TrameOutcome {
        let Some(at) = self.locate_anchor(anchor) else {
            let session = anchor.session;
            return TrameOutcome::display_only(vec![TrameEvent::EditTargetGone { session }]);
        };
        // The cell write and its derived commits are one undo item, which the
        // next `CommitEdit` seals.
        let _ = self.undo_manager.group_start();
        self.write_cell(at, text);
        self.finish_cell_writes()
    }

    /// Where the anchored cell stands now on the active sheet, or `None` when a
    /// peer deleted its row or its column. The caller checks the sheet.
    fn locate_anchor(&self, anchor: &EditAnchor) -> Option<CellPos> {
        let row = anchor.row.locate(&self.index)?;
        let col = anchor.col.locate(&self.index)?;
        Some(CellPos::new(row, col))
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
                    row: at.row,
                    col: at.col,
                })
            }
            Some(_) => None,
        }
    }
}
