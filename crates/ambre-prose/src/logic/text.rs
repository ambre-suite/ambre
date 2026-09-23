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
    pub(super) fn insert_text(&mut self, id: &TreeID, pos: u32, text: &str) -> ProseOutcome {
        let Some(body) = self.body_of(id) else {
            return Self::err_no_block(id);
        };
        let pos = pos as usize;
        if pos > body.len_unicode() {
            return Self::err_bad_range();
        }
        if text.is_empty() {
            // Nothing to write, so nothing to broadcast (`PROSE-047`).
            return ProseOutcome::nothing();
        }
        if let Err(e) = body.insert(pos, text) {
            return Self::err_write(&format!("{e:?}"));
        }
        // Plain text typed inside a suggestion is no part of it
        // (`PROSE-FEAT-304`).
        if let Err(e) = crate::suggest::strip_inherited(&body, pos, pos + text.chars().count()) {
            return Self::err_write(&e);
        }
        self.commit_with(Vec::new())
    }

    /// Delete a range inside one block.
    pub(super) fn delete_text(&mut self, id: &TreeID, range: (u32, u32)) -> ProseOutcome {
        let Some(body) = self.body_of(id) else {
            return Self::err_no_block(id);
        };
        let (start, end) = normalize(range);
        if end > body.len_unicode() {
            return Self::err_bad_range();
        }
        if start == end {
            return ProseOutcome::nothing();
        }
        if let Err(e) = body.delete(start, end - start) {
            return Self::err_write(&format!("{e:?}"));
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
        anchor: (TreeID, u32),
        focus: (TreeID, u32),
    ) -> ProseOutcome {
        if anchor.0 == focus.0 {
            return self.delete_text(&anchor.0, (anchor.1, focus.1));
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
        let head_off = head_off as usize;
        let tail_off = tail_off as usize;
        let head_len = head_body.len_unicode();
        let tail_len = tail_body.len_unicode();
        if head_off > head_len || tail_off > tail_len {
            return Self::err_bad_range();
        }
        // What survives of the last paragraph, before anything is written
        // (`PROSE-037`), with the marks it carries (`PROSE-042`).
        let surviving = marked_runs(&tail_body, tail_off, tail_len);
        if let Err(e) = head_body.delete(head_off, head_len - head_off) {
            return Self::err_write(&format!("{e:?}"));
        }
        if let Err(e) = write_runs(&head_body, head_off, &surviving) {
            return Self::err_write(&e);
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
        let beyond = &order[last + 1..];
        let emptied: Vec<TreeID> = order[first..=last]
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
    pub(super) fn split_block(&mut self, id: &TreeID, pos: u32) -> ProseOutcome {
        let Some(body) = self.body_of(id) else {
            return Self::err_no_block(id);
        };
        // A band is one block the metadata names. A split would put its tail
        // in the body flow.
        if doc::is_band(&doc::blocks(&self.doc), id) {
            return Self::err_nesting();
        }
        let pos = pos as usize;
        let len = body.len_unicode();
        if pos > len {
            return Self::err_bad_range();
        }
        let tail = marked_runs(&body, pos, len);
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
            return Self::err_write("the tree refused a block");
        };
        if let Err(e) = tree.mov_after(new_block, *id) {
            return Self::err_write(&format!("{e:?}"));
        }
        if let Err(e) = self.carry_style(id, &new_block) {
            return Self::err_write(&e);
        }
        if pos < len {
            if let Err(e) = body.delete(pos, len - pos) {
                return Self::err_write(&format!("{e:?}"));
            }
            let new_body = doc::get_block_text(&self.doc, &new_block);
            if let Err(e) = write_runs(&new_body, 0, &tail) {
                return Self::err_write(&e);
            }
        }
        self.commit_with(vec![ProseEvent::BlockSplit {
            source: block_id(*id),
            new_block: block_id(new_block),
        }])
    }
}

/// One run of body text and the marks it carries.
struct MarkedRun {
    text: String,
    marks: Vec<(String, loro::LoroValue)>,
}

/// The marked runs a body holds between two code-point offsets.
///
/// `LoroText::slice` answers a plain `String`, so a suffix moved through it
/// would reach its new block with every inline mark stripped. The text the two
/// blocks carry between them must be unchanged (`PROSE-042`), and a mark is
/// part of it. This reads the rich-text value instead and keeps each run's
/// attributes beside its text.
fn marked_runs(body: &LoroText, start: usize, end: usize) -> Vec<MarkedRun> {
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
                    .map(|(key, value)| (key.clone(), value.clone()))
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
fn write_runs(target: &LoroText, dest: usize, runs: &[MarkedRun]) -> Result<(), String> {
    let mut at = dest;
    for run in runs {
        target.insert(at, &run.text).map_err(|e| format!("{e:?}"))?;
        at += run.text.chars().count();
    }
    let mut at = dest;
    for run in runs {
        let len = run.text.chars().count();
        for (key, value) in &run.marks {
            if let Err(e) = target.mark(at..at + len, key, value.clone()) {
                tracing::warn!("Prose: the moved text lost the mark {key}: {e:?}");
            }
        }
        at += len;
    }
    Ok(())
}

/// The two ends of a range, smaller first. A selection is dragged in either
/// direction, and every caller wants the same pair.
fn normalize(range: (u32, u32)) -> (usize, usize) {
    let (a, b) = (range.0 as usize, range.1 as usize);
    if a <= b { (a, b) } else { (b, a) }
}

#[cfg(test)]
mod tests {
    use crate::events::{BlockInit, ProseCommand};
    use crate::logic::ProseState;

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
                pos: 0,
                text: (*text).to_owned(),
            });
        }
        (state, ids)
    }

    #[test]
    fn an_offset_past_the_body_is_refused() {
        let (mut state, ids) = document_of(&["abc"]);
        let outcome = state.handle_command(ProseCommand::InsertText {
            block_id: ids[0],
            pos: 99,
            text: "x".to_owned(),
        });
        assert!(outcome.update.is_none());
        assert_eq!(state.text_of(&ids[0]).as_deref(), Some("abc"));
    }

    #[test]
    fn a_deletion_across_a_boundary_joins_the_two_paragraphs() {
        let (mut state, ids) = document_of(&["hello", "world"]);
        state.handle_command(ProseCommand::DeleteRange {
            anchor: (ids[0], 5),
            focus: (ids[1], 0),
        });
        assert_eq!(state.text_of(&ids[0]).as_deref(), Some("helloworld"));
        assert_eq!(state.root_blocks(), vec![ids[0]]);
    }

    #[test]
    fn the_endpoints_arrive_in_either_order() {
        let (mut state, ids) = document_of(&["hello", "world"]);
        state.handle_command(ProseCommand::DeleteRange {
            anchor: (ids[1], 0),
            focus: (ids[0], 5),
        });
        assert_eq!(state.text_of(&ids[0]).as_deref(), Some("helloworld"));
    }

    #[test]
    fn a_deletion_removes_every_paragraph_it_emptied() {
        let (mut state, ids) = document_of(&["aaa", "bbb", "ccc"]);
        state.handle_command(ProseCommand::DeleteRange {
            anchor: (ids[0], 1),
            focus: (ids[2], 2),
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
            anchor: (ids[0], 0),
            focus: (ids[1], 4),
        });
        assert_eq!(state.root_blocks(), vec![ids[0]]);
        assert_eq!(state.text_of(&ids[0]).as_deref(), Some(""));
    }

    #[test]
    fn the_join_keeps_the_first_paragraphs_identity_and_style() {
        let (mut state, ids) = document_of(&["one", "two"]);
        state.handle_command(ProseCommand::SetBlockStyle {
            block_id: ids[0],
            style_id: "quote".to_owned(),
        });
        state.handle_command(ProseCommand::DeleteRange {
            anchor: (ids[0], 3),
            focus: (ids[1], 0),
        });
        assert_eq!(state.root_blocks(), vec![ids[0]]);
        assert_eq!(state.style_of(&ids[0]).as_deref(), Some("quote"));
    }

    #[test]
    fn a_split_leaves_the_text_the_two_paragraphs_carry_between_them() {
        let (mut state, ids) = document_of(&["hello world"]);
        let outcome = state.handle_command(ProseCommand::SplitBlock {
            block_id: ids[0],
            pos: 5,
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
            init: BlockInit::Heading { level: 3 },
            style_id: None,
        });
        let heading = state.root_blocks()[1];
        state.handle_command(ProseCommand::InsertText {
            block_id: heading,
            pos: 0,
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
            pos: 1,
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
        assert_eq!(state.heading_level_of(&new_block), Some(3));
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
            pos: 0,
            text: "inside".to_owned(),
        });
        state.handle_command(ProseCommand::DeleteRange {
            anchor: (ids[0], 1),
            focus: (inner, 2),
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
            range: (0, 11),
            key: "b".to_owned(),
            value: None,
        });
        let outcome = state.handle_command(ProseCommand::SplitBlock {
            block_id: ids[0],
            pos: 5,
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
            range: (0, 11),
            key: "b".to_owned(),
            value: None,
        });
        crate::logic::marks::tests::anchor(&mut state, &ids[0], 0..11);
        let outcome = state.handle_command(ProseCommand::SplitBlock {
            block_id: ids[0],
            pos: 5,
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
            range: (0, 5),
            key: "i".to_owned(),
            value: None,
        });
        state.handle_command(ProseCommand::DeleteRange {
            anchor: (ids[0], 5),
            focus: (ids[1], 0),
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
                lang: "rust".to_owned(),
            },
            style_id: None,
        });
        let code = state.root_blocks()[1];
        state.handle_command(ProseCommand::InsertText {
            block_id: code,
            pos: 0,
            text: "fn main".to_owned(),
        });
        let outcome = state.handle_command(ProseCommand::SplitBlock {
            block_id: code,
            pos: 2,
        });
        let new_block = outcome
            .display
            .iter()
            .find_map(|e| match e {
                crate::events::ProseEvent::BlockSplit { new_block, .. } => Some(*new_block),
                _ => None,
            })
            .expect("the split names the new block");
        assert_eq!(state.code_lang_of(&new_block).as_deref(), Some("rust"));
    }
}
