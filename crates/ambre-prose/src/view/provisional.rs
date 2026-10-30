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

use crate::events::grid::{RowCount, RowVec};
use crate::events::{BlockId, PageIndex, tree_id};
use crate::input::ProseInput;
use crate::layout::measure::{MarkedSpan, reshape_paragraph, shape_paragraph};
use crate::layout::run::LayoutVersion;
use ambre_bus::app::prose::DocOffset;
use ambre_core::{ByteCount, ByteOffset, ByteSpan, DocPx, char_len, char_to_byte};

use crate::layout::run::{RunOffset, ShapedRun, TableShape};
use crate::ui::ProseUiState;
use crate::view::hit::{Fragment, fragment};

/// A split sent and not yet laid out. It names the block it parts and the
/// offset it parts at, then the block it minted once the document answers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Splitting {
    pub block: BlockId,
    pub at: DocOffset,
    pub new_block: Option<BlockId>,
}

/// What the provisional runs were shaped from: the draft, the layout, the
/// split and the text held behind it.
///
/// The held text is document content, so `Debug` prints its length alone.
#[derive(Clone, PartialEq)]
struct Key {
    generation: crate::ui::DraftGeneration,
    version: LayoutVersion,
    splitting: Option<Splitting>,
    held: String,
}

impl std::fmt::Debug for Key {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Key")
            .field("generation", &self.generation)
            .field("version", &self.version)
            .field("splitting", &self.splitting)
            .field("held", &crate::redact::Redacted(&self.held))
            .finish()
    }
}

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
    pub caret: Option<RunOffset>,
}

/// One block a page shows, where the view draws it.
#[derive(Clone, Debug)]
pub(crate) struct Piece {
    pub index: crate::layout::run::FlowIndex,
    pub fragment: Fragment,
    pub run: Option<ShapedRun>,
    pub table: Option<Arc<TableShape>>,
    /// Whether `run` is provisional, or `table` holds the provisional runs of
    /// its cell blocks: their offsets are then the document's.
    pub provisional: PieceRuns,
}

ambre_core::define_flag! {
    /// Whether a piece draws the runs the layout carries, or provisional
    /// ones shaped on this thread.
    pub(crate) enum PieceRuns {
        /// The runs the layout carries: their offsets are the run's.
        Laid,
        /// Provisional runs: their offsets are the document's.
        Provisional,
    }
}

/// The marks over `bytes` of a text, counted from its start.
fn clip_marks(marks: &[MarkedSpan], bytes: ByteSpan) -> Vec<MarkedSpan> {
    marks
        .iter()
        .filter(|mark| mark.end > bytes.start && mark.start < bytes.end)
        .map(|mark| MarkedSpan {
            start: ByteOffset::ZERO + mark.start.max(bytes.start).count_from(bytes.start),
            end: ByteOffset::ZERO + mark.end.min(bytes.end).count_from(bytes.start),
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
        Key {
            generation: self.draft.generation(),
            version: self.layout_version,
            splitting: self.splitting,
            held: self.held_text(),
        }
    }

    /// Whether the provisional runs read what the view shows now.
    fn provisional_fresh(&self) -> bool {
        self.provisional.key.as_ref() == Some(&self.provisional_key())
    }

    /// Whether the provisional runs were shaped over the layout in force. A
    /// frame draws them then, so a press reads them even when an input
    /// replayed since has made them stale.
    fn provisional_drawn(&self) -> bool {
        self.provisional.key.as_ref().map(|key| key.version) == Some(self.layout_version)
    }

    /// The provisional run a frame draws for `block`, fresh or not.
    pub(crate) fn drawn_provisional_run(&self, block: BlockId) -> Option<&ShapedRun> {
        self.provisional
            .runs
            .get(&block)
            .filter(|_| self.provisional_drawn())
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

    /// The run a cell block is laid out in, and the width of its cell's
    /// text: the column's, inside the padding.
    pub(crate) fn cell_run(&self, block: BlockId) -> Option<(&ShapedRun, DocPx)> {
        let (cell, run) = self.shaped.cell_run(&tree_id(block))?;
        Some((
            run,
            (cell.width - cell.style.padding * 2.0).max(DocPx::ZERO),
        ))
    }

    /// `table` with the cell blocks drawn provisionally in place of their
    /// runs, and what it grew. A block pushes the blocks below it in its
    /// cell, and a cell its last row when it outgrows the rows it spans. A
    /// row never shrinks before the answer.
    fn provisional_table(&self, table: &TableShape) -> (TableShape, DocPx) {
        let mut shape = table.clone();
        let rows = table.row_tops.len().saturating_sub(RowCount::ONE);
        let mut growth: RowVec<DocPx> = vec![DocPx::ZERO; rows.to_usize()].into();
        for cell in &mut shape.cells {
            let mut shift = DocPx::ZERO;
            for (block, top, run) in &mut cell.blocks {
                *top += shift;
                if let Some(new) = self.provisional.runs.get(&crate::events::block_id(*block)) {
                    shift += new.data().height() - run.data().height();
                    *run = new.clone();
                }
            }
            if shift == DocPx::ZERO {
                continue;
            }
            cell.content_height = (cell.content_height + shift).max(DocPx::ZERO);
            let spanned = match (table.row_top(cell.rows.start), table.row_top(cell.rows.end)) {
                (Some(y0), Some(y1)) => y1 - y0,
                _ => continue,
            };
            let lack = cell.content_height + cell.style.padding * 2.0 - spanned;
            if let Some(slot) = cell.rows.last().and_then(|last| growth.get_mut(last)) {
                *slot = slot.max(lack);
            }
        }
        let mut grown = DocPx::ZERO;
        for (at, lack) in growth.iter_enumerated() {
            grown += lack.max(DocPx::ZERO);
            if let Some(top) = shape.row_tops.get_mut(at.next()) {
                *top += grown;
            }
        }
        (shape, grown)
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
        let width = self.shaped.layout.content_width;
        let mut blocks = self.draft.blocks();
        if let Some(split) = self.splitting
            && !blocks.contains(&split.block)
        {
            blocks.push(split.block);
        }
        let mut runs = HashMap::new();
        let mut ghost = None;
        for block in blocks {
            // A block of the flow, a band or a note, at the width of the
            // text; a cell's, at its column's inside the padding.
            let (laid, width) = if self.shaped.carries(&tree_id(block)) || self.is_band(block) {
                let Some(laid) = self.shaped.blocks.get(&tree_id(block)) else {
                    continue;
                };
                (laid.data(), width)
            } else {
                let Some((laid, text_width)) = self.cell_run(block) else {
                    continue;
                };
                (laid.data(), text_width)
            };
            let Some(style) = laid.style().cloned() else {
                continue;
            };
            let mut text = self.draft.text_of(block, laid.text());
            let mut marks = self.draft.marks_of(block, laid.text(), laid.marks());
            if let Some(split) = self.splitting.filter(|split| split.block == block) {
                let cut = char_to_byte(&text, split.at.as_char());
                let end = ByteCount::from_len(text.len()).as_end();
                let tail = text.get(cut.to_usize()..).unwrap_or("").to_owned();
                let tail_marks = clip_marks(&marks, ByteSpan::new(cut, end));
                text.truncate(cut.to_usize());
                marks = clip_marks(&marks, ByteSpan::new(ByteOffset::ZERO, cut));
                let (ghost_text, ghost_marks, caret) = match split.new_block {
                    Some(new_block) => (
                        self.draft.text_of(new_block, &tail),
                        self.draft.marks_of(new_block, &tail, &tail_marks),
                        None,
                    ),
                    None => {
                        let held = self.held_text();
                        let n = ByteCount::from_len(held.len());
                        let mut ghost_marks = tail_marks;
                        for mark in &mut ghost_marks {
                            mark.start += n;
                            mark.end += n;
                        }
                        // The text held takes the marks before the split.
                        let before = laid.marks_before(laid.len().as_end());
                        if !n.is_zero() {
                            ghost_marks.insert(
                                0,
                                MarkedSpan {
                                    start: ByteOffset::ZERO,
                                    end: n.as_end(),
                                    ..before
                                },
                            );
                        }
                        let caret = RunOffset::new(char_len(&held).get());
                        (format!("{held}{tail}"), ghost_marks, Some(caret))
                    }
                };
                let layout = shape_paragraph(
                    font_cx,
                    layout_cx,
                    (&ghost_text, &ghost_marks),
                    &style,
                    style.text_width(width),
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
            // Only the lines around the draft are shaped again; the rest of
            // the laid run is kept (`PROSE-UI-215`).
            let run = reshape_paragraph(
                font_cx,
                layout_cx,
                laid,
                (&text, &marks),
                &style,
                style.text_width(width),
            );
            runs.insert(block, run);
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
    pub(crate) fn pieces(&self, page: PageIndex) -> (Vec<Piece>, Option<Fragment>) {
        let body = self.shaped.layout.body_height();
        let drawn = self.provisional_drawn();
        let ghost = self.provisional.ghost.as_ref().filter(|_| drawn);
        let mut shift = DocPx::ZERO;
        let mut ghost_at = None;
        let mut pieces = Vec::new();
        for index in self.blocks_on(page) {
            let Some(block) = self.shaped.block(*index) else {
                continue;
            };
            if let Some(table) = self.shaped.tables.get(&block.block_id) {
                let Some(mut piece) = fragment(block, page, table.row_tops.as_slice()) else {
                    continue;
                };
                piece.top += shift;
                let mut table = Arc::clone(table);
                let mut provisional = false;
                if drawn && page == block.last_page() {
                    let (grown_table, grown) = self.provisional_table(&table);
                    if grown > DocPx::ZERO || grown_table.cells != table.cells {
                        piece.height += grown;
                        shift += grown;
                        table = Arc::new(grown_table);
                        provisional = true;
                    }
                }
                if piece.top >= body {
                    continue;
                }
                piece.height = piece.height.min(body - piece.top);
                pieces.push(Piece {
                    index: *index,
                    fragment: piece,
                    run: None,
                    table: Some(table),
                    provisional: PieceRuns::from_bool(provisional),
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
            let last = page == block.last_page();
            if last && let (Some(new), Some(old)) = (provisional, laid) {
                let grown = new.data().height() - old.data().height();
                piece.height = (piece.height + grown).max(DocPx::ZERO);
                shift += grown;
            }
            if last && let Some(ghost) = ghost.filter(|ghost| ghost.below == id) {
                let height = ghost.run.data().height();
                ghost_at = Some(Fragment {
                    top: piece.top + piece.height,
                    height,
                    run_top: DocPx::ZERO,
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
                provisional: PieceRuns::from_bool(provisional.is_some()),
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

#[cfg(test)]
mod tests {
    #[test]
    fn a_shaping_key_prints_its_held_text_by_length_alone() {
        let key = super::Key {
            generation: crate::ui::DraftGeneration::default(),
            version: super::LayoutVersion::FIRST,
            splitting: None,
            held: "held behind".into(),
        };
        assert!(crate::redact::tests::hides(&key, "held behind"));
    }

    #[test]
    fn a_slash_menu_prints_its_query_by_length_alone() {
        let menu = crate::view::slash::SlashMenu {
            origin: (
                crate::events::block_id(loro::TreeID::new(1, 0)),
                super::DocOffset::ZERO,
            ),
            query: "headi".into(),
            selected: crate::view::slash::SlashChoice::default(),
        };
        assert!(crate::redact::tests::hides(&menu, "headi"));
        let state = crate::view::slash::SlashMenuState {
            query: "headi".into(),
            ..crate::view::slash::SlashMenuState::default()
        };
        assert!(crate::redact::tests::hides(&state, "headi"));
    }
}
