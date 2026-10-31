//! The view's commands caught up with the changes it had not seen when it sent
//! them (`features/ambre-spec-prose-text-editing.md section 3`).
//!
//! The view draws its own edits before the Logic thread takes them, so its
//! positions count a text that lags behind the document. The Logic thread
//! keeps every change the view did not draw, numbered, until the view says it
//! has seen it. A text command is transformed through the ones it had not
//! seen, and each of those through it, so both sides land on one text. A
//! structural command moves its positions through them and draws nothing
//! ahead: the view waits for [`ProseEvent::Synced`] before it edits again.

use std::collections::VecDeque;

use ambre_bus::app::prose::{DocCount, DocDelta, DocOffset, DocSpan};

use crate::events::{ProseCommand, ProseEvent, ProseOutcome, TextEdit};
use crate::rebase;

use super::ProseState;

/// Where a change of the text came from. Only a command's change may be one
/// the view drew already.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ChangeSource {
    /// An import or a remote update: the view never drew it.
    Import,
    /// A command of this view.
    Command,
}

/// What the view drew for the command being dispatched.
#[derive(Debug, Default)]
pub(super) enum Prediction {
    /// Nothing: the change is new to the view.
    #[default]
    None,
    /// A structural change the view took in whole.
    Barrier,
    /// The runs the view drew, in the text the Logic thread holds.
    Text(Vec<TextEdit>),
}

impl Prediction {
    /// What a view would have drawn for `command`, for a caller that holds
    /// no view and keeps no count.
    pub(super) fn drawn_by(command: &ProseCommand) -> Self {
        match command {
            ProseCommand::InsertText { .. }
            | ProseCommand::DeleteText { .. }
            | ProseCommand::SplitBlock { .. }
            | ProseCommand::DeleteRange { .. }
            | ProseCommand::InsertBlock { .. } => Self::Barrier,
            _ => Self::None,
        }
    }
}

/// The Logic thread's side of the exchange.
#[derive(Debug, Default)]
pub(super) struct CatchUp {
    /// How many changes the view did not draw so far.
    seq: ambre_core::Generation<ambre_bus::app::prose::TextChangeSeq>,
    /// The last of them the view said it applied.
    seen: ambre_core::Generation<ambre_bus::app::prose::TextChangeSeq>,
    /// How many text commands the view sent so far.
    acked: ambre_bus::app::prose::TextAck,
    /// The runs of the changes the view has not said it applied, each with
    /// its number. Each counts positions after every text command taken.
    queue: VecDeque<(
        ambre_core::Generation<ambre_bus::app::prose::TextChangeSeq>,
        TextEdit,
    )>,
    /// What the view drew for the command in dispatch.
    pub(super) predicting: Prediction,
}

impl CatchUp {
    /// `edit` rewritten to apply after every change the view had not seen,
    /// and those rewritten to apply after it. `None` when nothing is left.
    fn rebase(&mut self, edit: TextEdit) -> Option<TextEdit> {
        let seen = self.seen;
        let mut edit = Some(edit);
        let mut queue = VecDeque::with_capacity(self.queue.len());
        for (seq, other) in self.queue.drain(..) {
            match edit {
                Some(held) if seq > seen => {
                    let (after, parts) = rebase::transform(&held, &other);
                    edit = after;
                    queue.extend(parts.into_iter().map(|part| (seq, part)));
                }
                _ => queue.push_back((seq, other)),
            }
        }
        self.queue = queue;
        edit
    }

    /// The changes the view had not seen, in the order they applied.
    fn unseen(&self) -> Vec<TextEdit> {
        self.queue
            .iter()
            .filter(|(seq, _)| *seq > self.seen)
            .map(|(_, edit)| *edit)
            .collect()
    }

    /// A position the view sent, moved through what it had not seen.
    fn position(&self, pos: DocOffset, block: crate::events::BlockId) -> DocOffset {
        rebase::through_all(pos, block, &self.unseen())
    }

    /// The end of a range the view sent: an insertion at it stays outside.
    fn end(&self, pos: DocOffset, block: crate::events::BlockId) -> DocOffset {
        rebase::end_through_all(pos, block, &self.unseen())
    }

    /// A range the view sent, both ends moved through what it had not seen.
    fn range(&self, block: crate::events::BlockId, range: DocSpan) -> DocSpan {
        let (start, end) = (range.start.min(range.end), range.start.max(range.end));
        DocSpan::new(self.position(start, block), self.end(end, block))
    }

    /// Stamp a change of the text: whether the view drew it, its number, and
    /// the runs the view applies. A change the view did not draw is queued.
    pub(super) fn stamp(
        &mut self,
        source: ChangeSource,
        edits: Vec<TextEdit>,
    ) -> (
        crate::events::ChangeOrigin,
        ambre_core::Generation<ambre_bus::app::prose::TextChangeSeq>,
        ambre_bus::app::prose::TextAck,
        Vec<TextEdit>,
    ) {
        let prediction = match source {
            ChangeSource::Command => std::mem::take(&mut self.predicting),
            ChangeSource::Import => Prediction::None,
        };
        use crate::events::ChangeOrigin;
        let (origin, edits) = match prediction {
            Prediction::None => (ChangeOrigin::Remote, edits),
            Prediction::Barrier => (ChangeOrigin::Local, edits),
            Prediction::Text(drawn) if drawn == edits => (ChangeOrigin::Local, edits),
            // The document did not do what the view drew: take the drawing
            // back, then show what it did.
            Prediction::Text(drawn) => {
                let mut runs = rebase::inverse(&drawn);
                runs.extend(edits);
                (ChangeOrigin::Remote, runs)
            }
        };
        if origin == ChangeOrigin::Remote && !edits.is_empty() {
            self.seq = self.seq.next();
            let seq = self.seq;
            self.queue.extend(edits.iter().map(|edit| (seq, *edit)));
        }
        (origin, self.seq, self.acked, edits)
    }
}

impl ProseState {
    /// Bring a command up to the text the Logic thread holds. `Err` carries
    /// the whole answer when there is nothing left to dispatch.
    pub(super) fn catch_up(&mut self, command: ProseCommand) -> Result<ProseCommand, ProseOutcome> {
        let sync = &mut self.catchup;
        sync.predicting = Prediction::None;
        Ok(match command {
            ProseCommand::Sync { seen } => {
                sync.seen = sync.seen.max(seen);
                let seen = sync.seen;
                sync.queue.retain(|(seq, _)| *seq > seen);
                let mut outcome = ProseOutcome::nothing();
                outcome
                    .display
                    .push(ProseEvent::Synced { acked: sync.acked });
                return Err(outcome);
            }
            ProseCommand::InsertText {
                block_id,
                pos,
                text,
            } => {
                sync.acked = sync.acked.next();
                let n = DocCount::of(&text);
                let drawn = TextEdit {
                    block_id,
                    at: pos,
                    delta: DocDelta::from_count(n),
                };
                let Some(edit) = sync.rebase(drawn) else {
                    return Err(ProseOutcome::nothing());
                };
                sync.predicting =
                    Prediction::Text(if n.is_zero() { Vec::new() } else { vec![edit] });
                ProseCommand::InsertText {
                    block_id,
                    pos: edit.at,
                    text,
                }
            }
            ProseCommand::DeleteText { block_id, range } => {
                sync.acked = sync.acked.next();
                let (start, end) = (range.start.min(range.end), range.start.max(range.end));
                if start == end {
                    sync.predicting = Prediction::Text(Vec::new());
                    return Ok(ProseCommand::DeleteText { block_id, range });
                }
                let drawn = TextEdit {
                    block_id,
                    at: start,
                    delta: DocDelta::from_count(end.count_from(start)).reversed(),
                };
                // Everything it was to delete is gone already.
                let Some(edit) = sync.rebase(drawn) else {
                    return Err(ProseOutcome::nothing());
                };
                sync.predicting = Prediction::Text(vec![edit]);
                ProseCommand::DeleteText {
                    block_id,
                    range: DocSpan::from_count(edit.at, edit.delta.magnitude()),
                }
            }
            ProseCommand::SplitBlock { block_id, pos } => {
                sync.predicting = Prediction::Barrier;
                ProseCommand::SplitBlock {
                    block_id,
                    pos: sync.position(pos, block_id),
                }
            }
            ProseCommand::DeleteRange { anchor, focus } => {
                sync.predicting = Prediction::Barrier;
                // The view sends the start first. Within one block the order
                // is read off the offsets.
                let (start, end) = if anchor.0 == focus.0 && focus.1 < anchor.1 {
                    (focus, anchor)
                } else {
                    (anchor, focus)
                };
                ProseCommand::DeleteRange {
                    anchor: (start.0, sync.position(start.1, start.0)),
                    focus: (end.0, sync.end(end.1, end.0)),
                }
            }
            ProseCommand::InsertBlock { .. } => {
                sync.predicting = Prediction::Barrier;
                command
            }
            // A range the view does not draw: its ends move through what the
            // view had not seen. Text typed at its end stays outside it
            // (`PROSE-UI-185`).
            ProseCommand::ApplyMark {
                block_id,
                range,
                key,
                value,
            } => ProseCommand::ApplyMark {
                block_id,
                range: sync.range(block_id, range),
                key,
                value,
            },
            ProseCommand::RemoveMark {
                block_id,
                range,
                key,
            } => ProseCommand::RemoveMark {
                block_id,
                range: sync.range(block_id, range),
                key,
            },
            ProseCommand::ClearInlineMarks { block_id, range } => ProseCommand::ClearInlineMarks {
                block_id,
                range: sync.range(block_id, range),
            },
            ProseCommand::FormatSelection {
                anchor,
                focus,
                key,
                action,
            } => {
                // The view sends the selection head first, so its tail is an
                // end: text typed there stays outside.
                let (anchor, focus) = if anchor.0 == focus.0 {
                    let span = sync.range(anchor.0, DocSpan::new(anchor.1, focus.1));
                    ((anchor.0, span.start), (focus.0, span.end))
                } else {
                    (
                        (anchor.0, sync.position(anchor.1, anchor.0)),
                        (focus.0, sync.end(focus.1, focus.0)),
                    )
                };
                ProseCommand::FormatSelection {
                    anchor,
                    focus,
                    key,
                    action,
                }
            }
            ProseCommand::ReadMarks { anchor, focus } => {
                // The answer names the selection as the view sent it, so the
                // view can tell it from a later one (`PROSE-UI-057`).
                let moved = (
                    (anchor.0, sync.position(anchor.1, anchor.0)),
                    (focus.0, sync.position(focus.1, focus.0)),
                );
                let mut outcome = self.read_marks(moved.0, moved.1);
                for event in &mut outcome.display {
                    if let ProseEvent::SelectionMarks {
                        anchor: a,
                        focus: f,
                        ..
                    } = event
                    {
                        (*a, *f) = (anchor, focus);
                    }
                }
                return Err(outcome);
            }
            other => other,
        })
    }

    /// What is left of a text command that wrote nothing: the view drew it,
    /// so it takes the drawing back.
    pub(super) fn settle(&mut self, outcome: &mut ProseOutcome) {
        let prediction = std::mem::take(&mut self.catchup.predicting);
        if let Prediction::Text(drawn) = prediction
            && !drawn.is_empty()
        {
            self.catchup.predicting = Prediction::Text(drawn);
            let (origin, seq, acked, edits) = self.catchup.stamp(ChangeSource::Command, Vec::new());
            outcome.display.push(ProseEvent::TextChanged {
                origin,
                seq,
                acked,
                edits,
                removed: Vec::new(),
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::BlockId;

    const A: BlockId = BlockId::new(ambre_core::PeerId::new(1), ambre_core::OpCounter::new(0));

    fn run(at: u32, delta: i64) -> TextEdit {
        TextEdit {
            block_id: A,
            at: DocOffset::new(at),
            delta: DocDelta::new(delta),
        }
    }

    #[test]
    fn a_change_the_view_did_not_draw_is_numbered_and_queued() {
        let mut sync = CatchUp::default();
        let (origin, seq, _, _) = sync.stamp(ChangeSource::Import, vec![run(0, 3)]);
        assert_eq!(origin, crate::events::ChangeOrigin::Remote);
        assert_eq!(seq, ambre_core::Generation::nth(1));
        assert_eq!(sync.rebase(run(2, 1)), Some(run(5, 1)));
    }

    #[test]
    fn a_change_the_view_has_seen_moves_nothing() {
        let mut sync = CatchUp::default();
        sync.stamp(ChangeSource::Import, vec![run(0, 3)]);
        sync.seen = ambre_core::Generation::nth(1);
        assert_eq!(sync.rebase(run(2, 1)), Some(run(2, 1)));
    }

    #[test]
    fn a_drawing_the_document_did_not_follow_is_taken_back() {
        let mut sync = CatchUp {
            predicting: Prediction::Text(vec![run(4, 2)]),
            ..CatchUp::default()
        };
        let (origin, seq, _, edits) = sync.stamp(ChangeSource::Command, Vec::new());
        assert_eq!(origin, crate::events::ChangeOrigin::Remote);
        assert_eq!(seq, ambre_core::Generation::nth(1));
        assert_eq!(edits, vec![run(4, -2)]);
    }
}
