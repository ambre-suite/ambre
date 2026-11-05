//! Typing, deleting, the cross-paragraph join and the split
//! (`features/ambre-spec-prose-text-editing.md`).
//!
//! Every offset here is a count of Unicode code points into the body of the
//! block the command names (`PROSE-045`). That is the unit Loro indexes a text
//! container with.
//!
//! **The join is the deletion.** A deletion across paragraphs keeps the head
//! block (`PROSE-041`), appends the tail's surviving suffix to it
//! (`PROSE-037`), and removes what it emptied (`PROSE-101`). Backspace at
//! offset zero is that deletion with both endpoints at the boundary. A merge
//! and a cross-paragraph delete are therefore one code path and one convergence
//! case (`PROSE-039`).

use crate::message::{DocChar, MarkKey};
use ambre_core::index::{Len, Pos, Span};
use loro::{LoroText, TreeID, TreeParentId};

use crate::doc;
use crate::events::{BlockId, BlockType, ProseEvent, ProseOutcome, block_id, tree_id};

use super::ProseState;

impl ProseState {
    /// The live blocks in the order a reader meets them: a prefix walk of the
    /// tree.
    ///
    /// A selection's two endpoints arrive in either order, and this is what
    /// tells the head from the tail.
    pub(super) fn document_order(&self) -> Vec<TreeID> {
        doc::flow(&doc::blocks(&self.doc))
    }

    /// The body text of a live block that carries one.
    pub(super) fn body_of(&self, id: &TreeID) -> Option<LoroText> {
        let tree = doc::blocks(&self.doc);
        doc::block_type_of(&tree, id)
            .filter(|t| t.has_body())
            .map(|_| doc::get_block_text(&self.doc, id))
    }

    /// The text a live block carries, for a test and for the view's own reads.
    #[must_use]
    pub fn text_of(&self, id: &BlockId) -> Option<String> {
        self.body_of(&tree_id(*id)).map(|text| text.to_string())
    }

    /// The live root blocks, in document order.
    #[must_use]
    pub fn root_blocks(&self) -> Vec<BlockId> {
        doc::roots(&doc::blocks(&self.doc))
            .into_iter()
            .map(block_id)
            .collect()
    }

    // -- the four editing commands -----------------------------------------

    /// Insert one committed run of text at one position in one block.
    pub(super) fn insert_text(
        &mut self,
        id: &TreeID,
        pos: Pos<DocChar>,
        text: &str,
    ) -> ProseOutcome {
        let Some(body) = self.body_of(id) else {
            return Self::err_no_block(id);
        };
        if pos > Len::<DocChar>::of(body.len_unicode()).as_end() {
            return Self::err_bad_range();
        }
        if text.is_empty() {
            // Nothing to write, so nothing to broadcast (`PROSE-047`).
            return ProseOutcome::nothing();
        }
        if let Err(e) = body.insert(pos.to_usize(), text) {
            return Self::err_write(e);
        }
        // Plain text typed inside a suggestion is no part of it
        // (`PROSE-FEAT-307`).
        if let Err(e) = crate::suggest::strip_inherited(
            &body,
            Span::<DocChar>::from_len(pos, ambre_core::char_len(text).as_axis::<DocChar>()),
        ) {
            return Self::err_write(e);
        }
        self.commit_with(Vec::new())
    }

    /// Delete a range inside one block.
    pub(super) fn delete_text(&mut self, id: &TreeID, range: Span<DocChar>) -> ProseOutcome {
        let Some(body) = self.body_of(id) else {
            return Self::err_no_block(id);
        };
        let range = normalize(range);
        if range.end > Len::<DocChar>::of(body.len_unicode()).as_end() {
            return Self::err_bad_range();
        }
        if range.is_empty() {
            return ProseOutcome::nothing();
        }
        // The view reads its clusters on a layout that may be older than the
        // body, so the range is widened against the body itself (`PROSE-124`).
        let range = whole_graphemes(&body.to_string(), range);
        if let Err(e) = body.delete(range.start.to_usize(), range.len().to_usize()) {
            return Self::err_write(e);
        }
        self.commit_with(Vec::new())
    }

    /// Delete one arbitrary, possibly cross-block, selection.
    ///
    /// The endpoints arrive in either order and this normalizes them against
    /// the document order. It is the one primitive of the four editing paths,
    /// and the one join operation there is (`PROSE-039`).
    pub(super) fn delete_range(
        &mut self,
        anchor: (TreeID, Pos<DocChar>),
        focus: (TreeID, Pos<DocChar>),
    ) -> ProseOutcome {
        if anchor.0 == focus.0 {
            return self.delete_text(&anchor.0, Span::<DocChar>::new(anchor.1, focus.1));
        }
        let order = self.document_order();
        let anchor_at = order.iter().position(|id| *id == anchor.0);
        let focus_at = order.iter().position(|id| *id == focus.0);
        let (Some(anchor_at), Some(focus_at)) = (anchor_at, focus_at) else {
            return Self::err_no_block(if anchor_at.is_none() {
                &anchor.0
            } else {
                &focus.0
            });
        };
        let ((head, head_off), (tail, tail_off)) = if anchor_at <= focus_at {
            (anchor, focus)
        } else {
            (focus, anchor)
        };
        let (Some(head_body), Some(tail_body)) = (self.body_of(&head), self.body_of(&tail)) else {
            return Self::err_no_block(&head);
        };
        let head_len = Len::<DocChar>::of(head_body.len_unicode()).as_end();
        let tail_len = Len::<DocChar>::of(tail_body.len_unicode()).as_end();
        if head_off > head_len || tail_off > tail_len {
            return Self::err_bad_range();
        }
        // Neither endpoint cuts a cluster in two (`PROSE-124`).
        let head_off = whole_graphemes(
            &head_body.to_string(),
            Span::<DocChar>::new(head_off, head_off),
        )
        .start;
        let tail_off = whole_graphemes(
            &tail_body.to_string(),
            Span::<DocChar>::new(tail_off, tail_off),
        )
        .end;
        // What survives of the last paragraph, before anything is written
        // (`PROSE-037`), with the marks it carries (`PROSE-042`).
        let surviving = marked_runs(&tail_body, Span::<DocChar>::new(tail_off, tail_len));
        if let Err(e) =
            head_body.delete(head_off.to_usize(), head_len.len_from(head_off).to_usize())
        {
            return Self::err_write(e);
        }
        if let Err(e) = write_runs(&head_body, head_off, &surviving) {
            return Self::err_write(e);
        }
        // Every block the deletion emptied goes (`PROSE-101`). The head is the
        // exception the frame condition names: it keeps its identity and its
        // paragraph style whether or not it is left empty (`PROSE-041`). An
        // ancestor of either endpoint stays too, or the deletion would take the
        // head out of the tree with it.
        //
        // Two further blocks stay. A block that carries no body was never
        // emptied by a text deletion. A table the selection merely crossed is
        // structure, and dropping it takes rows the reader never selected.
        // Nor does a block whose subtree reaches past the selection go, because
        // deleting it would take the blocks beyond the focus with it.
        let tree = doc::blocks(&self.doc);
        let (first, last) = (anchor_at.min(focus_at), anchor_at.max(focus_at));
        // Both endpoints index `order`, so neither range comes back empty.
        let beyond = order.get(last + 1..).unwrap_or_default();
        let emptied: Vec<TreeID> = order
            .get(first..=last)
            .unwrap_or_default()
            .iter()
            .copied()
            .filter(|id| *id != head)
            .filter(|id| !doc::is_ancestor_of(&tree, id, &head))
            .filter(|id| doc::block_type_of(&tree, id).is_some_and(BlockType::has_body))
            .filter(|id| {
                !beyond
                    .iter()
                    .any(|later| doc::is_ancestor_of(&tree, id, later))
            })
            .collect();
        for id in emptied {
            if let Err(e) = doc::hide_block(&self.doc, &tree, &id) {
                tracing::warn!("Prose: the tree refused to hide an emptied block: {e}");
            }
        }
        self.commit_with(Vec::new())
    }

    /// Split a paragraph at the caret.
    ///
    /// The new paragraph carries the source's type and paragraph style
    /// (`PROSE-038`). The text the two hold between them is unchanged, and the
    /// source keeps its identity (`PROSE-042`). The event names the new block,
    /// because the view cannot mint the id and needs it to place the caret
    /// (`PROSE-UI-141`).
    pub(super) fn split_block(&mut self, id: &TreeID, pos: Pos<DocChar>) -> ProseOutcome {
        let Some(body) = self.body_of(id) else {
            return Self::err_no_block(id);
        };
        // A band or a note is one block the metadata names. A split would
        // put its tail in the body flow.
        if doc::is_aside(&doc::blocks(&self.doc), id) {
            return Self::err_nesting();
        }
        let len = Len::<DocChar>::of(body.len_unicode()).as_end();
        if pos > len {
            return Self::err_bad_range();
        }
        let tail = marked_runs(&body, Span::<DocChar>::new(pos, len));
        let tree = doc::blocks(&self.doc);
        let Some(block_type) = doc::block_type_of(&tree, id) else {
            return Self::err_no_block(id);
        };
        let parent = match tree.parent(*id) {
            Some(TreeParentId::Node(parent)) => Some(parent),
            Some(TreeParentId::Root) => None,
            // A block whose parent is the deleted root, or none at all, is not
            // one a command reaches.
            _ => return Self::err_no_block(id),
        };
        let Some(new_block) = doc::create_block(&self.doc, parent, block_type) else {
            return Self::err_write(doc::WriteError::Refused(
                doc::RefusalCause::TreeRefusedBlock,
            ));
        };
        if let Err(e) = tree.mov_after(new_block, *id) {
            return Self::err_write(e);
        }
        if let Err(e) = self.carry_style(id, &new_block) {
            return Self::err_write(e);
        }
        if pos < len {
            if let Err(e) = body.delete(pos.to_usize(), len.len_from(pos).to_usize()) {
                return Self::err_write(e);
            }
            let new_body = doc::get_block_text(&self.doc, &new_block);
            if let Err(e) = write_runs(&new_body, Pos::<DocChar>::ZERO, &tail) {
                return Self::err_write(e);
            }
        }
        self.commit_with(vec![ProseEvent::BlockSplit {
            source: block_id(*id),
            new_block: block_id(new_block),
        }])
    }
}

/// The span of every code point `body` holds.
pub(super) fn body_span(body: &LoroText) -> Span<DocChar> {
    Span::<DocChar>::from_len(Pos::<DocChar>::ZERO, Len::<DocChar>::of(body.len_unicode()))
}

/// One run of body text and the marks it carries. The text is content, so
/// `Debug` prints its length alone.
#[derive(Clone, PartialEq)]
pub(super) struct MarkedRun {
    pub(super) text: String,
    pub(super) marks: Vec<(MarkKey, loro::LoroValue)>,
}

impl std::fmt::Debug for MarkedRun {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MarkedRun")
            .field("text", &ambre_core::Redacted::new(&self.text))
            .field("mark_count", &self.marks.len())
            .finish()
    }
}

/// The marked runs a body holds between two code-point offsets.
///
/// `LoroText::slice` answers a plain `String`, so a suffix moved through it
/// would reach its new block with every inline mark stripped. The text the two
/// blocks carry between them must be unchanged (`PROSE-042`), and a mark is
/// part of it. This reads the rich-text value instead and keeps each run's
/// attributes beside its text.
pub(super) fn marked_runs(body: &LoroText, span: Span<DocChar>) -> Vec<MarkedRun> {
    let (start, end) = (span.start.to_usize(), span.end.to_usize());
    let loro::LoroValue::List(runs) = body.get_richtext_value() else {
        return Vec::new();
    };
    let mut taken = Vec::new();
    let mut run_start = 0usize;
    for run in runs.iter() {
        let loro::LoroValue::Map(fields) = run else {
            continue;
        };
        let Some(loro::LoroValue::String(text)) = fields.get("insert") else {
            continue;
        };
        let run_end = run_start + text.chars().count();
        let (from, to) = (run_start.max(start), run_end.min(end));
        if from < to {
            let marks = match fields.get("attributes") {
                Some(loro::LoroValue::Map(attrs)) => attrs
                    .iter()
                    // A null attribute is a mark the run does not carry.
                    .filter(|(_, value)| !matches!(value, loro::LoroValue::Null))
                    .map(|(key, value)| (MarkKey::new(key.clone()), value.clone()))
                    .collect(),
                _ => Vec::new(),
            };
            taken.push(MarkedRun {
                text: text
                    .chars()
                    .skip(from - run_start)
                    .take(to - from)
                    .collect(),
                marks,
            });
        }
        run_start = run_end;
    }
    taken
}

/// Write runs into `target` at `dest`, marks and all.
///
/// Every run lands first and the marks follow, so no expansion policy pulls a
/// neighbouring run into a range that was never marked (`PROSE-007`).
///
/// # Errors
/// The CRDT engine's own refusal of an insertion. A mark the engine refuses is
/// logged rather than raised. The text is the content, and losing it to a
/// rejected attribute would be the worse answer.
pub(super) fn write_runs(
    target: &LoroText,
    dest: Pos<DocChar>,
    runs: &[MarkedRun],
) -> Result<(), doc::WriteError> {
    let dest = dest.to_usize();
    let mut at = dest;
    for run in runs {
        target.insert(at, &run.text)?;
        at += run.text.chars().count();
    }
    let mut at = dest;
    for run in runs {
        let len = run.text.chars().count();
        for (key, value) in &run.marks {
            if let Err(e) = target.mark(at..at + len, key.as_str(), value.clone()) {
                tracing::warn!("Prose: the moved text lost the mark {key}: {e:?}");
            }
        }
        at += len;
    }
    Ok(())
}

/// A range of code points widened to the grapheme clusters it cuts
/// (`PROSE-124`). The start moves back to the start of its cluster, and the
/// end forward to the end of its own. A range that cuts none comes back as it
/// was.
fn whole_graphemes(text: &str, span: Span<DocChar>) -> Span<DocChar> {
    use unicode_segmentation::UnicodeSegmentation;
    let (start, end) = (span.start, span.end);
    let (mut from, mut to) = (start, end);
    let mut at = Pos::<DocChar>::ZERO;
    for cluster in text.graphemes(true) {
        let next = at + ambre_core::char_len(cluster).as_axis::<DocChar>();
        if at < start && start < next {
            from = at;
        }
        if at < end && end < next {
            to = next;
        }
        if next >= end {
            break;
        }
        at = next;
    }
    Span::<DocChar>::new(from, to)
}

/// The two ends of a range, smaller first. A selection is dragged in either
/// direction, and every caller wants the same pair.
fn normalize(range: Span<DocChar>) -> Span<DocChar> {
    let (a, b) = (range.start, range.end);
    if a <= b {
        range
    } else {
        Span::<DocChar>::new(b, a)
    }
}

#[cfg(test)]
mod tests {
    use crate::events::HeadingLevel;
    use crate::events::{BlockInit, ProseCommand};
    use crate::logic::ProseState;
    use crate::message::DocChar;
    use ambre_core::Validated;
    use ambre_core::index::{Pos, Span};

    #[test]
    fn a_marked_run_prints_its_text_by_length_alone() {
        let run = super::MarkedRun {
            text: "body".into(),
            marks: Vec::new(),
        };
        assert!(crate::redact::tests::hides(&run, "body"));
    }

    /// A document of three root paragraphs holding `texts`.
    fn document_of(texts: &[&str]) -> (ProseState, Vec<crate::events::BlockId>) {
        let mut state = ProseState::new().expect("a document is built");
        let first = state.root_blocks()[0];
        let mut ids = vec![first];
        let mut previous = first;
        for _ in 1..texts.len() {
            state.handle_command(ProseCommand::InsertBlock {
                parent_id: None,
                after_id: Some(previous),
                init: BlockInit::Paragraph,
                style_id: None,
            });
            let roots = state.root_blocks();
            let next = roots[ids.len()];
            ids.push(next);
            previous = next;
        }
        for (id, text) in ids.iter().zip(texts) {
            state.handle_command(ProseCommand::InsertText {
                block_id: *id,
                pos: Pos::<DocChar>::ZERO,
                text: (*text).to_owned(),
            });
        }
        (state, ids)
    }

    /// `PROSE-124`: a range that ends or starts inside a cluster takes the
    /// whole of it, in one paragraph and across two.
    #[test]
    fn a_deletion_never_splits_a_grapheme() {
        let (mut state, ids) = document_of(&["xe\u{301}z", "e\u{301}y"]);
        state.handle_command(ProseCommand::DeleteText {
            block_id: ids[0],
            range: Span::<DocChar>::from_usize_range(1..2),
        });
        assert_eq!(state.text_of(&ids[0]).as_deref(), Some("xz"));
        state.handle_command(ProseCommand::DeleteRange {
            anchor: (ids[0], Pos::<DocChar>::new(2)),
            focus: (ids[1], Pos::<DocChar>::new(1)),
        });
        assert_eq!(state.text_of(&ids[0]).as_deref(), Some("xzy"));
    }

    #[test]
    fn an_offset_past_the_body_is_refused() {
        let (mut state, ids) = document_of(&["abc"]);
        let outcome = state.handle_command(ProseCommand::InsertText {
            block_id: ids[0],
            pos: Pos::<DocChar>::new(99),
            text: "x".to_owned(),
        });
        assert!(outcome.update.is_none());
        assert_eq!(state.text_of(&ids[0]).as_deref(), Some("abc"));
    }

    #[test]
    fn a_deletion_across_a_boundary_joins_the_two_paragraphs() {
        let (mut state, ids) = document_of(&["hello", "world"]);
        state.handle_command(ProseCommand::DeleteRange {
            anchor: (ids[0], Pos::<DocChar>::new(5)),
            focus: (ids[1], Pos::<DocChar>::ZERO),
        });
        assert_eq!(state.text_of(&ids[0]).as_deref(), Some("helloworld"));
        assert_eq!(state.root_blocks(), vec![ids[0]]);
    }

    #[test]
    fn the_endpoints_arrive_in_either_order() {
        let (mut state, ids) = document_of(&["hello", "world"]);
        state.handle_command(ProseCommand::DeleteRange {
            anchor: (ids[1], Pos::<DocChar>::ZERO),
            focus: (ids[0], Pos::<DocChar>::new(5)),
        });
        assert_eq!(state.text_of(&ids[0]).as_deref(), Some("helloworld"));
    }

    #[test]
    fn a_deletion_removes_every_paragraph_it_emptied() {
        let (mut state, ids) = document_of(&["aaa", "bbb", "ccc"]);
        state.handle_command(ProseCommand::DeleteRange {
            anchor: (ids[0], Pos::<DocChar>::new(1)),
            focus: (ids[2], Pos::<DocChar>::new(2)),
        });
        assert_eq!(state.text_of(&ids[0]).as_deref(), Some("ac"));
        assert_eq!(state.root_blocks(), vec![ids[0]]);
    }

    /// The one paragraph a deletion does not remove, even when it empties it.
    /// The first of the range keeps its identity and its style (`PROSE-041`),
    /// which is the exception `PROSE-101` names.
    #[test]
    fn a_deletion_that_empties_the_first_paragraph_keeps_it() {
        let (mut state, ids) = document_of(&["head", "tail"]);
        state.handle_command(ProseCommand::DeleteRange {
            anchor: (ids[0], Pos::<DocChar>::ZERO),
            focus: (ids[1], Pos::<DocChar>::new(4)),
        });
        assert_eq!(state.root_blocks(), vec![ids[0]]);
        assert_eq!(state.text_of(&ids[0]).as_deref(), Some(""));
    }

    #[test]
    fn the_join_keeps_the_first_paragraphs_identity_and_style() {
        let (mut state, ids) = document_of(&["one", "two"]);
        state.handle_command(ProseCommand::SetBlockStyle {
            block_id: ids[0],
            style_id: crate::message::StyleId::new("quote".to_owned()),
        });
        state.handle_command(ProseCommand::DeleteRange {
            anchor: (ids[0], Pos::<DocChar>::new(3)),
            focus: (ids[1], Pos::<DocChar>::ZERO),
        });
        assert_eq!(state.root_blocks(), vec![ids[0]]);
        assert_eq!(
            state.style_of(&ids[0]),
            Some(crate::message::StyleId::new("quote".to_owned()))
        );
    }

    #[test]
    fn a_split_leaves_the_text_the_two_paragraphs_carry_between_them() {
        let (mut state, ids) = document_of(&["hello world"]);
        let outcome = state.handle_command(ProseCommand::SplitBlock {
            block_id: ids[0],
            pos: Pos::<DocChar>::new(5),
        });
        let new_block = outcome
            .display
            .iter()
            .find_map(|e| match e {
                crate::events::ProseEvent::BlockSplit { new_block, .. } => Some(*new_block),
                _ => None,
            })
            .expect("the split names the new block");
        assert_eq!(state.text_of(&ids[0]).as_deref(), Some("hello"));
        assert_eq!(state.text_of(&new_block).as_deref(), Some(" world"));
        assert_eq!(state.root_blocks(), vec![ids[0], new_block]);
    }

    #[test]
    fn the_new_paragraph_carries_the_sources_type_and_style() {
        let mut state = ProseState::new().expect("a document is built");
        let first = state.root_blocks()[0];
        state.handle_command(ProseCommand::InsertBlock {
            parent_id: None,
            after_id: Some(first),
            init: BlockInit::Heading {
                level: HeadingLevel::H3,
            },
            style_id: None,
        });
        let heading = state.root_blocks()[1];
        state.handle_command(ProseCommand::InsertText {
            block_id: heading,
            pos: Pos::<DocChar>::ZERO,
            text: "a title".to_owned(),
        });
        state.handle_command(ProseCommand::SetParagraphStyle {
            block_id: heading,
            style: crate::events::ParagraphStyle {
                align: Some(crate::events::Align::Center),
                ..crate::events::ParagraphStyle::default()
            },
        });
        let outcome = state.handle_command(ProseCommand::SplitBlock {
            block_id: heading,
            pos: Pos::<DocChar>::new(1),
        });
        let new_block = outcome
            .display
            .iter()
            .find_map(|e| match e {
                crate::events::ProseEvent::BlockSplit { new_block, .. } => Some(*new_block),
                _ => None,
            })
            .expect("the split names the new block");
        assert_eq!(
            state.block_type_of(&new_block),
            state.block_type_of(&heading)
        );
        assert_eq!(
            state.paragraph_style_of(&new_block).align,
            Some(crate::events::Align::Center)
        );
        assert_eq!(
            state.heading_level_of(&new_block),
            Some(crate::doc::StoredLevel::of(HeadingLevel::H3))
        );
    }

    #[test]
    fn a_deletion_that_empties_the_tree_keeps_one_block() {
        let (mut state, ids) = document_of(&["only"]);
        state.handle_command(ProseCommand::DeleteBlock { block_id: ids[0] });
        assert_eq!(state.root_blocks().len(), 1);
    }

    /// A selection that merely crossed a table used to take the table with it:
    /// every node in the document-order span was deleted, structure included.
    /// A block that carries no body was never emptied by a text deletion.
    #[test]
    fn a_deletion_crossing_a_table_leaves_the_table_standing() {
        use crate::events::BlockType;
        let (mut state, ids) = document_of(&["hello"]);
        let table = state
            .insert_for_test(None, BlockType::Table)
            .expect("a table");
        let row = state
            .insert_for_test(Some(table), BlockType::TableRow)
            .expect("a row");
        let cell = state
            .insert_for_test(Some(row), BlockType::TableCell)
            .expect("a cell");
        let inner = state
            .insert_for_test(Some(cell), BlockType::Paragraph)
            .expect("a paragraph in the cell");
        state.handle_command(ProseCommand::InsertText {
            block_id: inner,
            pos: Pos::<DocChar>::ZERO,
            text: "inside".to_owned(),
        });
        state.handle_command(ProseCommand::DeleteRange {
            anchor: (ids[0], Pos::<DocChar>::new(1)),
            focus: (inner, Pos::<DocChar>::new(2)),
        });
        assert!(state.root_blocks().contains(&table));
        assert_eq!(state.parent_of(&cell), Some(row));
        assert!(state.parentages_are_declared());
    }

    /// `LoroText::slice` answers a plain `String`, so the suffix a split moved
    /// used to reach its new block with every inline mark stripped.
    #[test]
    fn a_split_carries_the_marks_of_the_text_it_moves() {
        let (mut state, ids) = document_of(&["hello world"]);
        state.handle_command(ProseCommand::ApplyMark {
            block_id: ids[0],
            range: Span::<DocChar>::from_usize_range(0..11),
            key: "b".into(),
            value: None,
        });
        let outcome = state.handle_command(ProseCommand::SplitBlock {
            block_id: ids[0],
            pos: Pos::<DocChar>::new(5),
        });
        let new_block = outcome
            .display
            .iter()
            .find_map(|e| match e {
                crate::events::ProseEvent::BlockSplit { new_block, .. } => Some(*new_block),
                _ => None,
            })
            .expect("the split names the new block");
        assert_eq!(state.text_of(&new_block).as_deref(), Some(" world"));
        let rendered = format!("{:?}", state.marks_of(&new_block));
        assert!(rendered.contains("\"b\""), "{rendered}");
    }

    /// A mark that covered the point the paragraph parts covers, on each half,
    /// exactly the characters it covered before (`PROSE-116`).
    ///
    /// An anchor is held beside a formatting mark. A comment spanning the cut
    /// must reach both halves, or the range it was written over is lost.
    #[test]
    fn a_mark_across_the_cut_reaches_both_halves() {
        let (mut state, ids) = document_of(&["hello world"]);
        state.handle_command(ProseCommand::ApplyMark {
            block_id: ids[0],
            range: Span::<DocChar>::from_usize_range(0..11),
            key: "b".into(),
            value: None,
        });
        crate::logic::marks::tests::anchor(&mut state, &ids[0], 0..11);
        let outcome = state.handle_command(ProseCommand::SplitBlock {
            block_id: ids[0],
            pos: Pos::<DocChar>::new(5),
        });
        let new_block = outcome
            .display
            .iter()
            .find_map(|e| match e {
                crate::events::ProseEvent::BlockSplit { new_block, .. } => Some(*new_block),
                _ => None,
            })
            .expect("the split names the new block");
        for key in ["\"b\"", "\"cm\""] {
            for block in [ids[0], new_block] {
                let rendered = format!("{:?}", state.marks_of(&block));
                assert!(rendered.contains(key), "{key} in {rendered}");
            }
        }
    }

    /// The deletion joined the two halves, and the marks the tail carried are
    /// part of the text the head must end up holding.
    #[test]
    fn a_join_carries_the_marks_of_the_surviving_tail() {
        let (mut state, ids) = document_of(&["hello", "world"]);
        state.handle_command(ProseCommand::ApplyMark {
            block_id: ids[1],
            range: Span::<DocChar>::from_usize_range(0..5),
            key: "i".into(),
            value: None,
        });
        state.handle_command(ProseCommand::DeleteRange {
            anchor: (ids[0], Pos::<DocChar>::new(5)),
            focus: (ids[1], Pos::<DocChar>::ZERO),
        });
        assert_eq!(state.text_of(&ids[0]).as_deref(), Some("helloworld"));
        let rendered = format!("{:?}", state.marks_of(&ids[0]));
        assert!(rendered.contains("\"i\""), "{rendered}");
    }

    /// A split used to drop the language of the half it minted: `carry_style`
    /// carried the heading level and nothing else.
    #[test]
    fn a_split_code_block_carries_its_language() {
        let mut state = ProseState::new().expect("a document is built");
        let first = state.root_blocks()[0];
        state.handle_command(ProseCommand::InsertBlock {
            parent_id: None,
            after_id: Some(first),
            init: BlockInit::CodeBlock {
                lang: ambre_core::Validated::<crate::message::CodeLang>::parse("rust").ok(),
            },
            style_id: None,
        });
        let code = state.root_blocks()[1];
        state.handle_command(ProseCommand::InsertText {
            block_id: code,
            pos: Pos::<DocChar>::ZERO,
            text: "fn main".to_owned(),
        });
        let outcome = state.handle_command(ProseCommand::SplitBlock {
            block_id: code,
            pos: Pos::<DocChar>::new(2),
        });
        let new_block = outcome
            .display
            .iter()
            .find_map(|e| match e {
                crate::events::ProseEvent::BlockSplit { new_block, .. } => Some(*new_block),
                _ => None,
            })
            .expect("the split names the new block");
        assert_eq!(
            state
                .code_lang_of(&new_block)
                .as_ref()
                .map(Validated::as_str),
            Some("rust")
        );
    }
}
