//! Where the annotations sit on the canvas, for the chrome that stands beside
//! them (`features/ambre-spec-prose-comments.md section 4`,
//! `features/ambre-spec-prose-track-changes.md section 4`).
//!
//! The comment panel and the accept or reject popover are DOM chrome. They
//! read the canvas positions from here and never from the paint pass.

use ambre_core::Uuid;

use crate::events::{BlockId, CommentThread, LinkSpan, ProseCommand, SuggestionKind, tree_id};
use crate::ui::ProseUiState;
use crate::view::column::DocPos;
use crate::view::hit::{Fragment, fragment};

/// A rectangle on the canvas, in CSS pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScreenRect {
    pub x0: f32,
    pub y0: f32,
    pub x1: f32,
    pub y1: f32,
}

impl ScreenRect {
    fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x0 && x < self.x1 && y >= self.y0 && y < self.y1
    }
}

/// The suggestion under the pointer, and the piece of it the pointer is on.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HoveredSuggestion {
    pub suggestion_id: Uuid,
    pub kind: SuggestionKind,
    pub rect: ScreenRect,
}

/// Where a comment thread's anchor starts, for the panel to line it up with.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ThreadAnchor {
    pub comment_id: Uuid,
    pub y_px: f32,
}

/// The least room a thread takes in the panel, so two anchors on one line
/// stack rather than overlap.
pub const THREAD_ROOM_PX: f32 = 64.0;

/// One thread as the panel shows it: the thread, and the height it sits at.
#[derive(Clone, Debug, PartialEq)]
pub struct PanelThread {
    pub thread: CommentThread,
    pub y_px: f32,
}

/// What the comment panel or the suggestion popover asks the view to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AnnotationIntent {
    /// Bring a thread's anchor into view.
    Focus {
        comment_id: Uuid,
    },
    Reply {
        comment_id: Uuid,
        text: String,
    },
    Resolve {
        comment_id: Uuid,
    },
    Accept {
        suggestion_id: Uuid,
    },
    Reject {
        suggestion_id: Uuid,
    },
}

/// Push each height down until it keeps `room` from the one above it. The
/// heights come in anchor order, so the first thread never moves.
#[must_use]
pub fn stack(heights: &[f32], room: f32) -> Vec<f32> {
    let mut out = Vec::with_capacity(heights.len());
    let mut floor = f32::NEG_INFINITY;
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
    pub(crate) fn fragment_at(&self, block: BlockId, run_y: f32) -> Option<(u32, Fragment)> {
        let index = self.flow_index(&tree_id(block))?;
        let layout = self.shaped.layout.blocks.get(index)?;
        let tops = self.shaped.blocks.get(&layout.block_id)?.data().line_tops();
        let last = layout.page_start + layout.continuation_heights.len() as u32;
        (layout.page_start..=last)
            .filter_map(|page| Some((page, fragment(layout, page, &tops)?)))
            .take_while(|(_, piece)| piece.run_top <= run_y)
            .last()
    }

    /// A point of a block's run, moved onto the canvas.
    pub(crate) fn to_screen(&self, block: BlockId, page: u32, piece: Fragment, x: f32, y: f32) -> (f32, f32) {
        let layout = &self.shaped.layout;
        let indent = self
            .flow_index(&tree_id(block))
            .and_then(|index| layout.blocks.get(index))
            .and_then(|b| self.shaped.blocks.get(&b.block_id))
            .map_or(0.0, |run| run.data().indent_px());
        let scale = self.viewport.scale();
        let page_x = layout.margin_left_px + indent + x;
        let page_y = layout.body_top_px() + piece.top + (y - piece.run_top);
        (
            self.column.left_px(&self.viewport) + page_x * scale,
            (self.column.page_top_y(page) + page_y - self.viewport.scroll_y) * scale,
        )
    }

    /// The piece of the text `start..end` of `block` drawn under a canvas
    /// point on `page`, if the point is on it. Only the text answers: the
    /// rest of its line does not.
    fn span_under(
        &self,
        block: BlockId,
        (start, end): (u32, u32),
        page: u32,
        (x_px, y_px): (f32, f32),
    ) -> Option<ScreenRect> {
        let index = self.flow_index(&tree_id(block))?;
        let layout = self.shaped.layout.blocks.get(index)?;
        let run = self.shaped.blocks.get(&layout.block_id)?.data();
        let piece = fragment(layout, page, &run.line_tops())?;
        let bottom = piece.run_top + piece.height;
        run.range_boxes(start, end).into_iter().find_map(|part| {
            let (y0, y1) = (part.y0 as f32, part.y1 as f32);
            if y1 <= piece.run_top || y0 >= bottom {
                return None;
            }
            let (x0, top) = self.to_screen(block, page, piece, part.x0 as f32, y0);
            let (x1, low) = self.to_screen(block, page, piece, part.x1 as f32, y1);
            let rect = ScreenRect {
                x0,
                y0: top,
                x1,
                y1: low,
            };
            rect.contains(x_px, y_px).then_some(rect)
        })
    }

    /// The page under a canvas point, if any.
    pub(crate) fn page_under(&self, x_px: f32, y_px: f32) -> Option<u32> {
        match self.column.screen_to_doc(&self.viewport, x_px, y_px) {
            DocPos::OnPage { page, .. } => Some(page),
            _ => None,
        }
    }

    /// The suggestion drawn under a canvas point, if any (`PROSE-UI-148`).
    #[must_use]
    pub fn suggestion_at(&self, x_px: f32, y_px: f32) -> Option<HoveredSuggestion> {
        let page = self.page_under(x_px, y_px)?;
        self.suggestions.iter().find_map(|suggestion| {
            suggestion.spans.iter().find_map(|span| {
                let rect =
                    self.span_under(span.block_id, (span.start, span.end), page, (x_px, y_px))?;
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
    pub fn link_at(&self, x_px: f32, y_px: f32) -> Option<&LinkSpan> {
        let page = self.page_under(x_px, y_px)?;
        self.links.iter().find(|link| {
            self.span_under(link.block_id, (link.start, link.end), page, (x_px, y_px))
                .is_some()
        })
    }

    /// Track the pointer: the suggestion under it becomes the hovered one.
    /// Answers whether that changed.
    pub fn hover(&mut self, x_px: f32, y_px: f32) -> bool {
        let now = self.suggestion_at(x_px, y_px);
        let changed = now != self.hovered;
        self.hovered = now;
        changed
    }

    /// The threads the panel lists, each at its anchor's height and stacked
    /// so none hides another (`PROSE-UI-050`). Top to bottom.
    #[must_use]
    pub fn panel_threads(&self) -> Vec<PanelThread> {
        let mut anchors = self.thread_anchors();
        anchors.sort_by(|a, b| a.y_px.total_cmp(&b.y_px));
        let heights: Vec<f32> = anchors.iter().map(|anchor| anchor.y_px).collect();
        anchors
            .iter()
            .zip(stack(&heights, THREAD_ROOM_PX))
            .filter_map(|(anchor, y_px)| {
                let thread = self
                    .comments
                    .iter()
                    .find(|thread| thread.comment_id == anchor.comment_id)?;
                Some(PanelThread {
                    thread: thread.clone(),
                    y_px,
                })
            })
            .collect()
    }

    /// Act on what the chrome asked. Answers the command to send, if any; a
    /// focus moves the view and sends nothing.
    pub fn take_intent(&mut self, intent: AnnotationIntent, now_ms: u64) -> Option<ProseCommand> {
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
                    now_ms,
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
            .filter(|thread| !thread.orphaned && !thread.resolved)
            .filter_map(|thread| {
                let block = thread.anchor_block?;
                let index = self.flow_index(&tree_id(block))?;
                let layout = self.shaped.layout.blocks.get(index)?;
                let caret = self
                    .shaped
                    .blocks
                    .get(&layout.block_id)?
                    .data()
                    .caret_box(thread.anchor_start);
                let (page, piece) = self.fragment_at(block, caret.y0 as f32)?;
                let (_, y_px) = self.to_screen(block, page, piece, 0.0, caret.y0 as f32);
                Some(ThreadAnchor {
                    comment_id: thread.comment_id,
                    y_px,
                })
            })
            .collect()
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use ambre_core::{UserId, Uuid};

    use crate::events::{
        BlockId, CommentThread, ProseEvent, Suggestion, SuggestionKind, SuggestionSpan, block_id,
    };
    use crate::ui::ProseUiState;
    use crate::view::tests::laid_out;

    fn view(paragraphs: usize) -> ProseUiState {
        let (_, shaped) = laid_out(paragraphs, 20);
        let mut ui = ProseUiState::default();
        ui.resize(1200.0, 900.0);
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
            suggestion_id: Uuid::new(),
            kind,
            author: "Ann".to_owned(),
            user_id: UserId::from_bytes([3; 16]),
            spans: vec![SuggestionSpan {
                block_id: block,
                start,
                end,
            }],
        }
    }

    /// The middle of the characters `start..end` of `block`, on the canvas.
    fn middle(ui: &ProseUiState, block: BlockId, start: u32, end: u32) -> (f32, f32) {
        let index = ui
            .flow_index(&crate::events::tree_id(block))
            .expect("laid out");
        let layout = &ui.shaped.layout.blocks[index];
        let run = ui.shaped.blocks[&layout.block_id].data();
        let part = run.range_boxes(start, end)[0];
        let piece = crate::view::hit::fragment(layout, layout.page_start, &run.line_tops())
            .expect("a fragment");
        ui.to_screen(
            block,
            layout.page_start,
            piece,
            ((part.x0 + part.x1) / 2.0) as f32,
            ((part.y0 + part.y1) / 2.0) as f32,
        )
    }

    #[test]
    fn a_hover_names_the_suggestion_under_the_pointer_and_no_other() {
        let mut ui = view(2);
        let (a, b) = (text_block(&ui, 0), text_block(&ui, 1));
        let first = suggestion(a, SuggestionKind::Insertion, 2, 6);
        let second = suggestion(b, SuggestionKind::Deletion, 3, 8);
        ui.apply_event(&ProseEvent::SuggestionsUpdated {
            suggesting: false,
            suggestions: vec![first.clone(), second.clone()],
        });
        let (x, y) = middle(&ui, a, 2, 6);
        assert!(ui.hover(x, y));
        let hovered = ui.hovered.expect("a suggestion");
        assert_eq!(hovered.suggestion_id, first.suggestion_id);
        assert!(hovered.rect.x0 <= x && x < hovered.rect.x1);
        // Moving within it changes nothing; moving onto the other names it.
        assert!(!ui.hover(x + 0.5, y));
        let (x, y) = middle(&ui, b, 3, 8);
        assert!(ui.hover(x, y));
        assert_eq!(
            ui.hovered.map(|h| h.suggestion_id),
            Some(second.suggestion_id)
        );
        // Plain text beside a suggestion names none.
        let (x, y) = middle(&ui, a, 12, 16);
        assert!(ui.hover(x, y));
        assert_eq!(ui.hovered, None);
    }

    #[test]
    fn each_open_thread_is_lined_up_with_its_anchor() {
        let mut ui = view(3);
        let (a, c) = (text_block(&ui, 0), text_block(&ui, 2));
        let thread = |block, resolved| CommentThread {
            comment_id: Uuid::new(),
            anchor_block: Some(block),
            anchor_start: 4,
            anchor_end: 9,
            orphaned: false,
            resolved,
            replies: Vec::new(),
        };
        let (first, last, done) = (thread(a, false), thread(c, false), thread(a, true));
        ui.apply_event(&ProseEvent::CommentsUpdated {
            threads: vec![first.clone(), done, last.clone()],
        });
        let anchors = ui.thread_anchors();
        let ids: Vec<_> = anchors.iter().map(|anchor| anchor.comment_id).collect();
        assert_eq!(ids, vec![first.comment_id, last.comment_id]);
        let (_, y) = middle(&ui, a, 4, 9);
        assert!(anchors[0].y_px <= y && y - anchors[0].y_px < 40.0);
        assert!(anchors[1].y_px > anchors[0].y_px);
        // Scrolling moves the anchors with the text.
        ui.scroll_by(100.0);
        let moved = ui.thread_anchors();
        assert!((anchors[0].y_px - moved[0].y_px - 100.0 * ui.viewport.scale()).abs() < 0.5);
    }

    #[test]
    fn two_anchors_on_one_line_are_stacked_and_the_first_stays() {
        assert_eq!(
            super::stack(&[10.0, 10.0, 200.0], 64.0),
            vec![10.0, 74.0, 200.0]
        );
    }

    #[test]
    fn the_panel_lists_open_threads_top_to_bottom() {
        let mut ui = view(3);
        let (a, c) = (text_block(&ui, 0), text_block(&ui, 2));
        let thread = |block| CommentThread {
            comment_id: Uuid::new(),
            anchor_block: Some(block),
            anchor_start: 0,
            anchor_end: 3,
            orphaned: false,
            resolved: false,
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
        let id = Uuid::new();
        ui.apply_event(&ProseEvent::CommentsUpdated {
            threads: vec![CommentThread {
                comment_id: id,
                anchor_block: Some(far),
                anchor_start: 0,
                anchor_end: 3,
                orphaned: false,
                resolved: false,
                replies: Vec::new(),
            }],
        });
        assert_eq!(
            ui.take_intent(AnnotationIntent::Focus { comment_id: id }, 5),
            None
        );
        assert!(ui.viewport.scroll_y > 0.0);
        let reply = AnnotationIntent::Reply {
            comment_id: id,
            text: "ok".to_owned(),
        };
        assert_eq!(
            ui.take_intent(reply, 5),
            Some(ProseCommand::ReplyToComment {
                comment_id: id,
                text: "ok".to_owned(),
                now_ms: 5
            })
        );
        let blank = AnnotationIntent::Reply {
            comment_id: id,
            text: "  ".to_owned(),
        };
        assert_eq!(ui.take_intent(blank, 5), None);
        assert_eq!(
            ui.take_intent(AnnotationIntent::Resolve { comment_id: id }, 5),
            Some(ProseCommand::ResolveComment { comment_id: id })
        );
        assert_eq!(
            ui.take_intent(AnnotationIntent::Accept { suggestion_id: id }, 5),
            Some(ProseCommand::AcceptSuggestion { suggestion_id: id })
        );
        assert_eq!(
            ui.take_intent(AnnotationIntent::Reject { suggestion_id: id }, 5),
            Some(ProseCommand::RejectSuggestion { suggestion_id: id })
        );
    }

    #[test]
    fn a_suggestion_that_leaves_the_list_takes_its_popover_along() {
        let mut ui = view(1);
        let a = text_block(&ui, 0);
        let one = suggestion(a, SuggestionKind::Insertion, 2, 6);
        ui.apply_event(&ProseEvent::SuggestionsUpdated {
            suggesting: false,
            suggestions: vec![one.clone()],
        });
        let (x, y) = middle(&ui, a, 2, 6);
        ui.hover(x, y);
        assert!(ui.hovered.is_some());
        ui.apply_event(&ProseEvent::SuggestionsUpdated {
            suggesting: false,
            suggestions: Vec::new(),
        });
        assert_eq!(ui.hovered, None);
    }
}
