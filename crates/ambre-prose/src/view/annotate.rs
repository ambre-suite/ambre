//! Where the annotations sit on the canvas, for the chrome that stands beside
//! them (`features/ambre-spec-prose-comments.md section 4`,
//! `features/ambre-spec-prose-track-changes.md section 4`).
//!
//! The comment panel and the accept or reject popover are DOM chrome. They
//! read the canvas positions from here and never from the paint pass.

use std::collections::HashMap;

use ambre_core::id::{Comment, Id, Suggestion};
use ambre_core::{DocPoint, DocPx, ViewPoint, ViewPx};

use crate::events::{
    BlockId, CommentThread, LinkSpan, PageIndex, ProseCommand, SuggestionKind, tree_id,
};
use crate::layout::run::{RunOffset, RunSpan};
use crate::ui::ProseUiState;
use crate::view::answered_span;
use crate::view::column::DocPos;
use crate::view::hit::{Fragment, fragment};

/// A rectangle on the canvas, in pixels of the view.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScreenRect {
    pub x0: ViewPx,
    pub y0: ViewPx,
    pub x1: ViewPx,
    pub y1: ViewPx,
}

impl ScreenRect {
    fn contains(&self, at: ViewPoint) -> bool {
        at.x >= self.x0 && at.x < self.x1 && at.y >= self.y0 && at.y < self.y1
    }
}

/// The suggestion under the pointer, and the piece of it the pointer is on.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HoveredSuggestion {
    pub suggestion_id: Id<Suggestion>,
    pub kind: SuggestionKind,
    pub rect: ScreenRect,
}

/// Where a comment thread's anchor starts, for the panel to line it up with.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ThreadAnchor {
    pub comment_id: Id<Comment>,
    /// The anchor's top on the canvas.
    pub y: ViewPx,
}

/// The least room a thread takes in the panel, so two anchors on one line
/// stack rather than overlap.
pub const THREAD_ROOM: ViewPx = ViewPx::new(64.0);

/// One thread as the panel shows it: the thread, and the height it sits at.
#[derive(Clone, Debug, PartialEq)]
pub struct PanelThread {
    pub thread: CommentThread,
    /// The card's top, level with the canvas.
    pub y: ViewPx,
}

/// What the comment panel or the suggestion popover asks the view to do. A
/// reply's text is the user's, so `Debug` prints its length alone.
#[derive(Clone, PartialEq, Eq)]
pub enum AnnotationIntent {
    /// Bring a thread's anchor into view.
    Focus {
        comment_id: Id<Comment>,
    },
    Reply {
        comment_id: Id<Comment>,
        text: String,
    },
    Resolve {
        comment_id: Id<Comment>,
    },
    Accept {
        suggestion_id: Id<Suggestion>,
    },
    Reject {
        suggestion_id: Id<Suggestion>,
    },
}

impl std::fmt::Debug for AnnotationIntent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Focus { comment_id } => f
                .debug_struct("Focus")
                .field("comment_id", comment_id)
                .finish(),
            Self::Reply { comment_id, text } => f
                .debug_struct("Reply")
                .field("comment_id", comment_id)
                .field("text", &crate::redact::Redacted(text))
                .finish(),
            Self::Resolve { comment_id } => f
                .debug_struct("Resolve")
                .field("comment_id", comment_id)
                .finish(),
            Self::Accept { suggestion_id } => f
                .debug_struct("Accept")
                .field("suggestion_id", suggestion_id)
                .finish(),
            Self::Reject { suggestion_id } => f
                .debug_struct("Reject")
                .field("suggestion_id", suggestion_id)
                .finish(),
        }
    }
}

/// How a peer took a thread out of the panel while its reply field held a
/// draft (`PROSE-UI-232`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LostReply {
    /// The thread itself was deleted.
    Deleted,
    /// The thread was resolved.
    Resolved,
    /// The text the thread was anchored to was deleted.
    Orphaned,
}

impl LostReply {
    /// What the user is told when the draft held text.
    #[must_use]
    pub const fn notice(self) -> &'static str {
        match self {
            Self::Deleted => {
                "A collaborator deleted the thread you were replying to. Your reply was not posted."
            }
            Self::Resolved => {
                "A collaborator resolved the thread you were replying to. Your reply was not posted."
            }
            Self::Orphaned => {
                "A collaborator deleted the text of the thread you were replying to. Your reply was not posted."
            }
        }
    }
}

/// Push each height down until it keeps `room` from the one above it. The
/// heights come in anchor order, so the first thread never moves.
#[must_use]
pub fn stack(heights: &[ViewPx], room: ViewPx) -> Vec<ViewPx> {
    let mut out = Vec::with_capacity(heights.len());
    let mut floor = ViewPx::new(f32::NEG_INFINITY);
    for &y in heights {
        let placed = y.max(floor);
        out.push(placed);
        floor = placed + room;
    }
    out
}

impl ProseUiState {
    /// The page and fragment of `block` that show `run_y`, a height in its
    /// run, and the top of that fragment in the page.
    pub(crate) fn fragment_at(
        &self,
        block: BlockId,
        run_y: DocPx,
    ) -> Option<(PageIndex, Fragment)> {
        let index = self.flow_index(&tree_id(block))?;
        let layout = self.shaped.block(index)?;
        let tops = self.shaped.blocks.get(&layout.block_id)?.data().line_tops();
        layout
            .page_span()
            .into_iter()
            .filter_map(|page| Some((page, fragment(layout, page, &tops)?)))
            .take_while(|(_, piece)| piece.run_top <= run_y)
            .last()
    }

    /// A point of a block's run, moved onto the canvas.
    pub(crate) fn to_screen(
        &self,
        block: BlockId,
        page: PageIndex,
        piece: Fragment,
        at: DocPoint,
    ) -> ViewPoint {
        let layout = &self.shaped.layout;
        let indent = self
            .flow_index(&tree_id(block))
            .and_then(|index| self.shaped.block(index))
            .and_then(|b| self.shaped.blocks.get(&b.block_id))
            .map_or(DocPx::ZERO, |run| run.data().indent());
        let scale = self.viewport.scale();
        let page_x = layout.margin_left + indent + at.x;
        let page_y = layout.body_top() + piece.top + (at.y - piece.run_top);
        ViewPoint::new(
            self.column.left(&self.viewport) + ViewPx::new(page_x.get() * scale),
            ViewPx::new(
                (self.column.page_top_y(page) + page_y - self.viewport.scroll_y).get() * scale,
            ),
        )
    }

    /// The piece of the text `span` of `block`'s laid run drawn under a canvas
    /// point on `page`, if the point is on it. Only the text answers: the
    /// rest of its line does not.
    fn span_under(
        &self,
        block: BlockId,
        span: RunSpan,
        page: PageIndex,
        at: ViewPoint,
    ) -> Option<ScreenRect> {
        let index = self.flow_index(&tree_id(block))?;
        let layout = self.shaped.block(index)?;
        let run = self.shaped.blocks.get(&layout.block_id)?.data();
        let piece = fragment(layout, page, &run.line_tops())?;
        let bottom = piece.run_top + piece.height;
        run.range_boxes(span).into_iter().find_map(|part| {
            let (y0, y1) = (DocPx::new(part.y0 as f32), DocPx::new(part.y1 as f32));
            if y1 <= piece.run_top || y0 >= bottom {
                return None;
            }
            let corner = |x: f64, y: DocPx| DocPoint::new(DocPx::new(x as f32), y);

            let top_left = self.to_screen(block, page, piece, corner(part.x0, y0));
            let bottom_right = self.to_screen(block, page, piece, corner(part.x1, y1));
            let rect = ScreenRect {
                x0: top_left.x,
                y0: top_left.y,
                x1: bottom_right.x,
                y1: bottom_right.y,
            };
            rect.contains(at).then_some(rect)
        })
    }

    /// The page under a canvas point, if any.
    pub(crate) fn page_under(&self, at: ViewPoint) -> Option<PageIndex> {
        match self.column.screen_to_doc(&self.viewport, at) {
            DocPos::OnPage { page, .. } => Some(page),
            _ => None,
        }
    }

    /// The suggestion drawn under a canvas point, if any (`PROSE-UI-148`).
    #[must_use]
    pub fn suggestion_at(&self, at: ViewPoint) -> Option<HoveredSuggestion> {
        let page = self.page_under(at)?;
        self.suggestions.iter().find_map(|suggestion| {
            suggestion.spans.iter().find_map(|span| {
                let rect =
                    self.span_under(span.block_id, answered_span(span.start, span.end), page, at)?;
                Some(HoveredSuggestion {
                    suggestion_id: suggestion.suggestion_id,
                    kind: suggestion.kind,
                    rect,
                })
            })
        })
    }

    /// The link drawn under a canvas point, if any: over its marked text
    /// alone (`PROSE-UI-153`).
    #[must_use]
    pub fn link_at(&self, at: ViewPoint) -> Option<&LinkSpan> {
        let page = self.page_under(at)?;
        self.links.iter().find(|link| {
            self.span_under(link.block_id, answered_span(link.start, link.end), page, at)
                .is_some()
        })
    }

    /// Track the pointer: the suggestion under it becomes the hovered one.
    /// Answers whether that changed.
    pub fn hover(&mut self, at: ViewPoint) -> bool {
        let now = self.suggestion_at(at);
        let changed = now != self.hovered;
        self.hovered = now;
        changed
    }

    /// The threads the panel lists, each at its anchor's height and stacked
    /// so none hides another (`PROSE-UI-050`). Top to bottom.
    #[must_use]
    pub fn panel_threads(&self) -> Vec<PanelThread> {
        let mut anchors = self.thread_anchors();
        anchors.sort_by(|a, b| a.y.get().total_cmp(&b.y.get()));
        let heights: Vec<ViewPx> = anchors.iter().map(|anchor| anchor.y).collect();
        anchors
            .iter()
            .zip(stack(&heights, THREAD_ROOM))
            .filter_map(|(anchor, y)| {
                let thread = self
                    .comments
                    .iter()
                    .find(|thread| thread.comment_id == anchor.comment_id)?;
                Some(PanelThread {
                    thread: thread.clone(),
                    y,
                })
            })
            .collect()
    }

    /// The reply drafts whose thread left the panel, each with how it left
    /// when the draft held text, so a notice is owed (`PROSE-UI-232`). A draft whose thread is still open
    /// stays, wherever the thread moved (`PROSE-UI-231`).
    #[must_use]
    pub fn lost_drafts(
        &self,
        drafts: &HashMap<Id<Comment>, String>,
    ) -> Vec<(Id<Comment>, Option<LostReply>)> {
        drafts
            .iter()
            .filter_map(|(id, text)| {
                let why = match self.comments.iter().find(|t| t.comment_id == *id) {
                    None => LostReply::Deleted,
                    Some(thread) if thread.resolved == ambre_core::ThreadState::Resolved => {
                        LostReply::Resolved
                    }
                    Some(thread) if thread.orphaned == crate::events::AnchorState::Orphaned => {
                        LostReply::Orphaned
                    }
                    Some(_) => return None,
                };
                Some((*id, (!text.trim().is_empty()).then_some(why)))
            })
            .collect()
    }

    /// Act on what the chrome asked. Answers the command to send, if any; a
    /// focus moves the view and sends nothing.
    pub fn take_intent(
        &mut self,
        intent: AnnotationIntent,
        now: ambre_core::Timestamp,
    ) -> Option<ProseCommand> {
        Some(match intent {
            AnnotationIntent::Focus { comment_id } => {
                let thread = self.comments.iter().find(|t| t.comment_id == comment_id)?;
                let block = thread.anchor_block?;
                self.reveal_block(block);
                return None;
            }
            AnnotationIntent::Reply { comment_id, text } => {
                if text.trim().is_empty() {
                    return None;
                }
                ProseCommand::ReplyToComment {
                    comment_id,
                    text,
                    now,
                }
            }
            AnnotationIntent::Resolve { comment_id } => ProseCommand::ResolveComment { comment_id },
            AnnotationIntent::Accept { suggestion_id } => {
                self.hovered = None;
                ProseCommand::AcceptSuggestion { suggestion_id }
            }
            AnnotationIntent::Reject { suggestion_id } => {
                self.hovered = None;
                ProseCommand::RejectSuggestion { suggestion_id }
            }
        })
    }

    /// Where each open, anchored thread starts on the canvas, in the order
    /// the Logic thread listed them. A thread whose block is not laid out
    /// is left out.
    #[must_use]
    pub fn thread_anchors(&self) -> Vec<ThreadAnchor> {
        self.comments
            .iter()
            .filter(|thread| {
                thread.orphaned == crate::events::AnchorState::Anchored
                    && thread.resolved == ambre_core::ThreadState::Open
            })
            .filter_map(|thread| {
                let block = thread.anchor_block?;
                let index = self.flow_index(&tree_id(block))?;
                let layout = self.shaped.block(index)?;
                let caret = self
                    .shaped
                    .blocks
                    .get(&layout.block_id)?
                    .data()
                    .caret_box(RunOffset::from_doc(thread.anchor_start));
                let top = DocPx::new(caret.y0 as f32);
                let (page, piece) = self.fragment_at(block, top)?;
                let at = self.to_screen(block, page, piece, DocPoint::new(DocPx::ZERO, top));
                Some(ThreadAnchor {
                    comment_id: thread.comment_id,
                    y: at.y,
                })
            })
            .collect()
    }
}

#[cfg(test)]
#[cfg(not(target_arch = "wasm32"))]
mod tests {
    use crate::layout::run::{RunOffset, RunSpan};
    use ambre_bus::app::prose::DocOffset;
    use ambre_core::id::{self, Comment, Id, User};
    use ambre_core::{DocPoint, DocPx, ViewPoint, ViewPx};

    fn px(value: f32) -> ViewPx {
        ViewPx::new(value)
    }

    use crate::events::{
        BlockId, CommentThread, ProseEvent, Suggestion, SuggestionKind, SuggestionSpan, block_id,
    };
    use crate::ui::ProseUiState;
    use crate::view::tests::{down, laid_out};

    fn view(paragraphs: usize) -> ProseUiState {
        let (_, shaped) = laid_out(paragraphs, 20);
        let mut ui = ProseUiState::default();
        ui.resize(ViewPx::new(1200.0), ViewPx::new(900.0));
        ui.apply_layout(shaped);
        ui
    }

    fn text_block(ui: &ProseUiState, nth: usize) -> BlockId {
        let block = ui
            .shaped
            .layout
            .blocks
            .iter()
            .filter(|block| !ui.shaped.blocks[&block.block_id].data().is_empty())
            .nth(nth)
            .expect("a block with text");
        block_id(block.block_id)
    }

    fn suggestion(block: BlockId, kind: SuggestionKind, start: u32, end: u32) -> Suggestion {
        Suggestion {
            suggestion_id: Id::<id::Suggestion>::mint(),
            kind,
            author: ambre_core::DisplayName::new("Ann").ok(),
            user_id: Id::<User>::from_bytes([3; 16]),
            spans: vec![SuggestionSpan {
                block_id: block,
                start: DocOffset::new(start),
                end: DocOffset::new(end),
            }],
        }
    }

    /// The middle of the characters `start..end` of `block`, on the canvas.
    fn middle(ui: &ProseUiState, block: BlockId, start: u32, end: u32) -> ViewPoint {
        let index = ui
            .flow_index(&crate::events::tree_id(block))
            .expect("laid out");
        let layout = &ui.shaped.layout.blocks[index.to_usize()];
        let run = ui.shaped.blocks[&layout.block_id].data();
        let part = run.range_boxes(RunSpan::new(RunOffset::new(start), RunOffset::new(end)))[0];
        let piece = crate::view::hit::fragment(layout, layout.page_start, &run.line_tops())
            .expect("a fragment");
        ui.to_screen(
            block,
            layout.page_start,
            piece,
            DocPoint::new(
                DocPx::new(((part.x0 + part.x1) / 2.0) as f32),
                DocPx::new(((part.y0 + part.y1) / 2.0) as f32),
            ),
        )
    }

    #[test]
    fn a_hover_names_the_suggestion_under_the_pointer_and_no_other() {
        let mut ui = view(2);
        let (a, b) = (text_block(&ui, 0), text_block(&ui, 1));
        let first = suggestion(a, SuggestionKind::Insertion, 2, 6);
        let second = suggestion(b, SuggestionKind::Deletion, 3, 8);
        ui.apply_event(&ProseEvent::SuggestionsUpdated {
            mode: crate::events::EditMode::Editing,
            suggestions: vec![first.clone(), second.clone()],
        });
        let at = middle(&ui, a, 2, 6);
        assert!(ui.hover(at));
        let hovered = ui.hovered.expect("a suggestion");
        assert_eq!(hovered.suggestion_id, first.suggestion_id);
        assert!(hovered.rect.x0 <= at.x && at.x < hovered.rect.x1);
        // Moving within it changes nothing; moving onto the other names it.
        assert!(!ui.hover(ViewPoint::new(at.x + ViewPx::new(0.5), at.y)));
        assert!(ui.hover(middle(&ui, b, 3, 8)));
        assert_eq!(
            ui.hovered.map(|h| h.suggestion_id),
            Some(second.suggestion_id)
        );
        // Plain text beside a suggestion names none.
        assert!(ui.hover(middle(&ui, a, 12, 16)));
        assert_eq!(ui.hovered, None);
    }

    #[test]
    fn each_open_thread_is_lined_up_with_its_anchor() {
        let mut ui = view(3);
        let (a, c) = (text_block(&ui, 0), text_block(&ui, 2));
        let thread = |block, resolved| CommentThread {
            comment_id: Id::<Comment>::mint(),
            anchor_block: Some(block),
            anchor_start: DocOffset::new(4),
            anchor_end: DocOffset::new(9),
            orphaned: crate::events::AnchorState::Anchored,
            resolved,
            replies: Vec::new(),
        };
        let (first, last, done) = (
            thread(a, ambre_core::ThreadState::Open),
            thread(c, ambre_core::ThreadState::Open),
            thread(a, ambre_core::ThreadState::Resolved),
        );
        ui.apply_event(&ProseEvent::CommentsUpdated {
            threads: vec![first.clone(), done, last.clone()],
        });
        let anchors = ui.thread_anchors();
        let ids: Vec<_> = anchors.iter().map(|anchor| anchor.comment_id).collect();
        assert_eq!(ids, vec![first.comment_id, last.comment_id]);
        let y = middle(&ui, a, 4, 9).y;
        assert!(anchors[0].y <= y && y - anchors[0].y < ViewPx::new(40.0));
        assert!(anchors[1].y > anchors[0].y);
        // Scrolling moves the anchors with the text.
        ui.scroll_by(down(100.0));
        let moved = ui.thread_anchors();
        assert!(((anchors[0].y - moved[0].y).get() - 100.0 * ui.viewport.scale()).abs() < 0.5);
    }

    #[test]
    fn two_anchors_on_one_line_are_stacked_and_the_first_stays() {
        assert_eq!(
            super::stack(&[px(10.0), px(10.0), px(200.0)], px(64.0)),
            vec![px(10.0), px(74.0), px(200.0)]
        );
    }

    #[test]
    fn the_panel_lists_open_threads_top_to_bottom() {
        let mut ui = view(3);
        let (a, c) = (text_block(&ui, 0), text_block(&ui, 2));
        let thread = |block| CommentThread {
            comment_id: Id::<Comment>::mint(),
            anchor_block: Some(block),
            anchor_start: DocOffset::ZERO,
            anchor_end: DocOffset::new(3),
            orphaned: crate::events::AnchorState::Anchored,
            resolved: ambre_core::ThreadState::Open,
            replies: Vec::new(),
        };
        let (low, high) = (thread(c), thread(a));
        ui.apply_event(&ProseEvent::CommentsUpdated {
            threads: vec![low.clone(), high.clone()],
        });
        let listed: Vec<_> = ui
            .panel_threads()
            .into_iter()
            .map(|entry| entry.thread.comment_id)
            .collect();
        assert_eq!(listed, vec![high.comment_id, low.comment_id]);
    }

    #[test]
    fn each_intent_becomes_its_command_and_a_focus_scrolls() {
        use super::AnnotationIntent;
        use crate::events::ProseCommand;
        let mut ui = view(80);
        let far = text_block(&ui, 60);
        let id = Id::<Comment>::mint();
        ui.apply_event(&ProseEvent::CommentsUpdated {
            threads: vec![CommentThread {
                comment_id: id,
                anchor_block: Some(far),
                anchor_start: DocOffset::ZERO,
                anchor_end: DocOffset::new(3),
                orphaned: crate::events::AnchorState::Anchored,
                resolved: ambre_core::ThreadState::Open,
                replies: Vec::new(),
            }],
        });
        assert_eq!(
            ui.take_intent(
                AnnotationIntent::Focus { comment_id: id },
                ambre_core::Timestamp(5)
            ),
            None
        );
        assert!(ui.viewport.scroll_y > DocPx::new(0.0));
        let reply = AnnotationIntent::Reply {
            comment_id: id,
            text: "ok".to_owned(),
        };
        assert_eq!(
            ui.take_intent(reply, ambre_core::Timestamp(5)),
            Some(ProseCommand::ReplyToComment {
                comment_id: id,
                text: "ok".to_owned(),
                now: ambre_core::Timestamp(5)
            })
        );
        let blank = AnnotationIntent::Reply {
            comment_id: id,
            text: "  ".to_owned(),
        };
        assert_eq!(ui.take_intent(blank, ambre_core::Timestamp(5)), None);
        assert_eq!(
            ui.take_intent(
                AnnotationIntent::Resolve { comment_id: id },
                ambre_core::Timestamp(5)
            ),
            Some(ProseCommand::ResolveComment { comment_id: id })
        );
        let suggestion = Id::<id::Suggestion>::mint();
        assert_eq!(
            ui.take_intent(
                AnnotationIntent::Accept {
                    suggestion_id: suggestion
                },
                ambre_core::Timestamp(5)
            ),
            Some(ProseCommand::AcceptSuggestion {
                suggestion_id: suggestion
            })
        );
        assert_eq!(
            ui.take_intent(
                AnnotationIntent::Reject {
                    suggestion_id: suggestion
                },
                ambre_core::Timestamp(5)
            ),
            Some(ProseCommand::RejectSuggestion {
                suggestion_id: suggestion
            })
        );
    }

    #[test]
    fn a_draft_outlives_a_move_and_its_thread_leaving_is_told() {
        use super::LostReply;
        let mut ui = view(3);
        let a = text_block(&ui, 0);
        let thread = || CommentThread {
            comment_id: Id::<Comment>::mint(),
            anchor_block: Some(a),
            anchor_start: DocOffset::new(4),
            anchor_end: DocOffset::new(9),
            orphaned: crate::events::AnchorState::Anchored,
            resolved: ambre_core::ThreadState::Open,
            replies: Vec::new(),
        };
        let (moved, gone, closed, cut, blank) = (thread(), thread(), thread(), thread(), thread());
        let drafts: std::collections::HashMap<_, _> = [
            (moved.comment_id, "half a reply".to_owned()),
            (gone.comment_id, "lost".to_owned()),
            (closed.comment_id, "lost".to_owned()),
            (cut.comment_id, "lost".to_owned()),
            (blank.comment_id, " ".to_owned()),
        ]
        .into();
        let all = vec![
            moved.clone(),
            gone.clone(),
            closed.clone(),
            cut.clone(),
            blank.clone(),
        ];
        ui.apply_event(&ProseEvent::CommentsUpdated {
            threads: all.clone(),
        });
        assert!(ui.lost_drafts(&drafts).is_empty(), "every thread is open");

        // A scroll and a peer's text above move the thread: its draft stays.
        ui.scroll_by(down(120.0));
        let mut shifted = all.clone();
        shifted[0].anchor_start = DocOffset::new(7);
        ui.apply_event(&ProseEvent::CommentsUpdated { threads: shifted });
        assert!(ui.lost_drafts(&drafts).is_empty(), "a move keeps the draft");

        // A peer deletes one, resolves one and cuts the text of another.
        let mut after = vec![moved.clone(), closed, cut];
        after[1].resolved = ambre_core::ThreadState::Resolved;
        after[2].orphaned = crate::events::AnchorState::Orphaned;
        ui.apply_event(&ProseEvent::CommentsUpdated { threads: after });
        let mut lost = ui.lost_drafts(&drafts);
        lost.sort_by_key(|(id, _)| *id.as_bytes());
        let mut expected = vec![
            (gone.comment_id, Some(LostReply::Deleted)),
            (all[2].comment_id, Some(LostReply::Resolved)),
            (all[3].comment_id, Some(LostReply::Orphaned)),
            (blank.comment_id, None),
        ];
        expected.sort_by_key(|(id, _)| *id.as_bytes());
        assert_eq!(
            lost, expected,
            "each departure is told, an empty field is not"
        );
    }

    #[test]
    fn a_suggestion_that_leaves_the_list_takes_its_popover_along() {
        let mut ui = view(1);
        let a = text_block(&ui, 0);
        let one = suggestion(a, SuggestionKind::Insertion, 2, 6);
        ui.apply_event(&ProseEvent::SuggestionsUpdated {
            mode: crate::events::EditMode::Editing,
            suggestions: vec![one.clone()],
        });
        ui.hover(middle(&ui, a, 2, 6));
        assert!(ui.hovered.is_some());
        ui.apply_event(&ProseEvent::SuggestionsUpdated {
            mode: crate::events::EditMode::Editing,
            suggestions: Vec::new(),
        });
        assert_eq!(ui.hovered, None);
    }
}
