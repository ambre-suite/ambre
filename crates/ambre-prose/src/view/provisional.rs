//! The paragraphs the view draws before the document answers
//! (`features/ambre-spec-prose-text-editing.md`).
//!
//! A block the draft changes is shaped again, whole, on the UI thread. Its
//! text is the one in force with the draft laid over it, shaped in the style
//! and at the width of its run. It wraps as the paragraph will
//! (`PROSE-UI-201`), and the text deleted is gone from it (`PROSE-UI-202`). A
//! split draws its block parted and the paragraph it opens below it, the text
//! typed into it included (`PROSE-UI-203`).
//!
//! The runs are kept with what they were shaped from. A frame reads them only
//! while that still holds, and reads the layout in force otherwise.

use std::collections::HashMap;
use std::sync::Arc;

use crate::events::{BlockId, tree_id};
use crate::input::ProseInput;
use crate::layout::measure::{MarkedSpan, shape_paragraph};
use crate::layout::run::{ShapedRun, TableShape};
use crate::ui::{ProseUiState, chars};
use crate::view::hit::{Fragment, fragment};

/// A split sent and not yet laid out. It names the block it parts and the
/// offset it parts at, then the block it minted once the document answers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Splitting {
    pub block: BlockId,
    pub at: u32,
    pub new_block: Option<BlockId>,
}

/// What the provisional runs were shaped from: the draft, the layout, the
/// split and the text held behind it.
type Key = (u64, u64, Option<Splitting>, String);

/// The provisional runs, and what they were shaped from.
#[derive(Clone, Debug, Default)]
pub struct Provisional {
    key: Option<Key>,
    runs: HashMap<BlockId, ShapedRun>,
    ghost: Option<Ghost>,
}

impl Provisional {
    /// The paragraph a split opens as last drawn, fresh or not.
    pub(crate) fn ghost(&self) -> Option<&Ghost> {
        self.ghost.as_ref()
    }
}

/// The paragraph a split opens, drawn below the block it parts until a
/// layout carries it.
#[derive(Clone, Debug)]
pub(crate) struct Ghost {
    pub below: BlockId,
    pub run: ShapedRun,
    /// Where the caret is drawn in it, while the document has not named it.
    pub caret: Option<u32>,
}

/// One block a page shows, where the view draws it.
#[derive(Clone, Debug)]
pub(crate) struct Piece {
    pub index: usize,
    pub fragment: Fragment,
    pub run: Option<ShapedRun>,
    pub table: Option<Arc<TableShape>>,
    /// Whether `run` is provisional: its offsets are then the document's.
    pub provisional: bool,
}

/// The byte a code-point offset starts at, the length past the end.
fn byte_at(text: &str, offset: u32) -> usize {
    text.char_indices()
        .nth(offset as usize)
        .map_or(text.len(), |(at, _)| at)
}

/// The marks of `text[from..to]`, counted from `from`.
fn clip_marks(marks: &[MarkedSpan], from: usize, to: usize) -> Vec<MarkedSpan> {
    marks
        .iter()
        .filter(|mark| mark.end > from && mark.start < to)
        .map(|mark| MarkedSpan {
            start: mark.start.max(from) - from,
            end: mark.end.min(to) - from,
            ..*mark
        })
        .collect()
}

impl ProseUiState {
    /// The text held behind a structural command: typed, not yet sent.
    fn held_text(&self) -> String {
        self.queued
            .iter()
            .map_while(|input| match input {
                ProseInput::Text(text) | ProseInput::ComposeEnd(text) => Some(text.as_str()),
                _ => None,
            })
            .collect()
    }

    fn provisional_key(&self) -> Key {
        (
            self.draft.generation(),
            self.layout_version,
            self.splitting,
            self.held_text(),
        )
    }

    /// Whether the provisional runs read what the view shows now.
    fn provisional_fresh(&self) -> bool {
        self.provisional.key.as_ref() == Some(&self.provisional_key())
    }

    /// Whether the provisional runs were shaped over the layout in force. A
    /// frame draws them then, so a press reads them even when an input
    /// replayed since has made them stale.
    fn provisional_drawn(&self) -> bool {
        self.provisional.key.as_ref().map(|key| key.1) == Some(self.layout_version)
    }

    /// The provisional run of `block`, while it is fresh.
    pub(crate) fn provisional_run(&self, block: BlockId) -> Option<&ShapedRun> {
        if !self.provisional_fresh() {
            return None;
        }
        self.provisional.runs.get(&block)
    }

    /// The run drawn for a block that has one: the provisional one while it
    /// is fresh, the laid one otherwise.
    pub(crate) fn drawn_of(&self, id: loro::TreeID) -> Option<&ShapedRun> {
        self.provisional_run(crate::events::block_id(id))
            .or_else(|| self.shaped.blocks.get(&id))
    }

    /// The paragraph a split opens, while it is fresh.
    pub(crate) fn ghost(&self) -> Option<&Ghost> {
        self.provisional
            .ghost
            .as_ref()
            .filter(|_| self.provisional_fresh())
    }

    /// Shape again every block the draft or a split changes, on the
    /// contexts of the thread that paints. Runs already shaped from the
    /// same state are kept.
    pub(crate) fn shape_provisional(
        &mut self,
        font_cx: &mut parley::FontContext,
        layout_cx: &mut parley::LayoutContext<ambre_draw::ParleyBrush>,
    ) {
        let key = self.provisional_key();
        if self.provisional.key.as_ref() == Some(&key) {
            return;
        }
        let width = self.shaped.layout.content_width_px;
        let mut blocks = self.draft.blocks();
        if let Some(split) = self.splitting
            && !blocks.contains(&split.block)
        {
            blocks.push(split.block);
        }
        let mut runs = HashMap::new();
        let mut ghost = None;
        for block in blocks {
            // A block of the flow, a band or a note. A cell keeps its run
            // until the answer: its width is its column's.
            if !self.shaped.carries(&tree_id(block)) && !self.is_band(block) {
                continue;
            }
            let Some(laid) = self.shaped.blocks.get(&tree_id(block)).map(ShapedRun::data) else {
                continue;
            };
            let Some(style) = laid.style().cloned() else {
                continue;
            };
            let mut text = self.draft.text_of(block, laid.text());
            let mut marks = self.draft.marks_of(block, laid.text(), laid.marks());
            if let Some(split) = self.splitting.filter(|split| split.block == block) {
                let cut = byte_at(&text, split.at);
                let tail = text.get(cut..).unwrap_or("").to_owned();
                let tail_marks = clip_marks(&marks, cut, text.len());
                text.truncate(cut);
                marks = clip_marks(&marks, 0, cut);
                let (ghost_text, ghost_marks, caret) = match split.new_block {
                    Some(new_block) => (
                        self.draft.text_of(new_block, &tail),
                        self.draft.marks_of(new_block, &tail, &tail_marks),
                        None,
                    ),
                    None => {
                        let held = self.held_text();
                        let n = held.len();
                        let mut ghost_marks = tail_marks;
                        for mark in &mut ghost_marks {
                            mark.start += n;
                            mark.end += n;
                        }
                        // The text held takes the marks before the split.
                        let before = laid.marks_before(laid.len());
                        if n > 0 {
                            ghost_marks.insert(
                                0,
                                MarkedSpan {
                                    start: 0,
                                    end: n,
                                    ..before
                                },
                            );
                        }
                        (format!("{held}{tail}"), ghost_marks, Some(chars(&held)))
                    }
                };
                let layout = shape_paragraph(
                    font_cx,
                    layout_cx,
                    (&ghost_text, &ghost_marks),
                    &style,
                    style.text_width_px(width),
                );
                let run = ShapedRun::marked(&ghost_text, layout, &style, &ghost_marks);
                if let Some(new_block) = split.new_block {
                    runs.insert(new_block, run.clone());
                }
                ghost = Some(Ghost {
                    below: block,
                    run,
                    caret,
                });
            }
            let layout = shape_paragraph(
                font_cx,
                layout_cx,
                (&text, &marks),
                &style,
                style.text_width_px(width),
            );
            runs.insert(block, ShapedRun::marked(&text, layout, &style, &marks));
        }
        self.provisional = Provisional {
            key: Some(key),
            runs,
            ghost,
        };
    }

    /// The blocks `page` shows, each where the view draws it, and the
    /// paragraph a split opens when it is drawn there.
    ///
    /// The blocks after a block drawn provisionally move on its last page. They
    /// move by what it grew, and by the paragraph a split opens below it. A
    /// block pushed past the foot of the body is cut there.
    pub(crate) fn pieces(&self, page: u32) -> (Vec<Piece>, Option<Fragment>) {
        let body = self.shaped.layout.body_height_px();
        let drawn = self.provisional_drawn();
        let ghost = self.provisional.ghost.as_ref().filter(|_| drawn);
        let mut shift = 0.0_f32;
        let mut ghost_at = None;
        let mut pieces = Vec::new();
        for index in self.blocks_on(page) {
            let Some(block) = self.shaped.block(*index) else {
                continue;
            };
            if let Some(table) = self.shaped.tables.get(&block.block_id) {
                let Some(mut piece) = fragment(block, page, &table.row_tops) else {
                    continue;
                };
                piece.top += shift;
                if piece.top >= body {
                    continue;
                }
                piece.height = piece.height.min(body - piece.top);
                pieces.push(Piece {
                    index: *index,
                    fragment: piece,
                    run: None,
                    table: Some(Arc::clone(table)),
                    provisional: false,
                });
                continue;
            }
            let id = crate::events::block_id(block.block_id);
            let laid = self.shaped.blocks.get(&block.block_id);
            let provisional = self.provisional.runs.get(&id).filter(|_| drawn);
            let run = provisional.or(laid).cloned();
            let tops = run
                .as_ref()
                .map(|run| run.data().line_tops())
                .unwrap_or_default();
            let Some(mut piece) = fragment(block, page, &tops) else {
                continue;
            };
            piece.top += shift;
            let last = page == *block.page_span().end();
            if last && let (Some(new), Some(old)) = (provisional, laid) {
                let grown = new.data().height() - old.data().height();
                piece.height = (piece.height + grown).max(0.0);
                shift += grown;
            }
            if last && let Some(ghost) = ghost.filter(|ghost| ghost.below == id) {
                let height = ghost.run.data().height();
                ghost_at = Some(Fragment {
                    top: piece.top + piece.height,
                    height,
                    run_top: 0.0,
                });
                shift += height;
            }
            if piece.top >= body {
                continue;
            }
            piece.height = piece.height.min(body - piece.top);
            pieces.push(Piece {
                index: *index,
                fragment: piece,
                run,
                table: None,
                provisional: provisional.is_some(),
            });
        }
        let ghost_at = ghost_at
            .filter(|piece| piece.top < body)
            .map(|piece| Fragment {
                height: piece.height.min(body - piece.top),
                ..piece
            });
        (pieces, ghost_at)
    }
}
