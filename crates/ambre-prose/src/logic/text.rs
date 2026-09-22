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
use crate::events::{BlockId, ProseEvent, ProseOutcome, block_id, tree_id};

use super::ProseState;

impl ProseState {
    /// The live blocks in the order a reader meets them: a prefix walk of the
    /// tree.
    ///
    /// A selection's two endpoints arrive in either order, and this is what
    /// tells the head from the tail.
    pub(super) fn document_order(&self) -> Vec<TreeID> {
        let tree = doc::blocks(&self.doc);
        let mut order = Vec::new();
        let mut stack: Vec<TreeID> = doc::roots(&tree);
        stack.reverse();
        while let Some(id) = stack.pop() {
            order.push(id);
            let mut children = doc::children_of(&tree, Some(id));
            children.reverse();
            stack.extend(children);
        }
        order
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
        // (`PROSE-037`).
        let Ok(surviving) = tail_body.slice(tail_off, tail_len) else {
            return Self::err_bad_range();
        };
        if let Err(e) = head_body.delete(head_off, head_len - head_off) {
            return Self::err_write(&format!("{e:?}"));
        }
        if !surviving.is_empty()
            && let Err(e) = head_body.insert(head_off, &surviving)
        {
            return Self::err_write(&format!("{e:?}"));
        }
        // Every block the deletion emptied goes (`PROSE-101`). The head is the
        // exception the frame condition names: it keeps its identity and its
        // paragraph style whether or not it is left empty (`PROSE-041`). An
        // ancestor of either endpoint stays too, or the deletion would take the
        // head out of the tree with it.
        let tree = doc::blocks(&self.doc);
        let emptied: Vec<TreeID> = order[anchor_at.min(focus_at)..=anchor_at.max(focus_at)]
            .iter()
            .copied()
            .filter(|id| *id != head)
            .filter(|id| !doc::is_ancestor_of(&tree, id, &head))
            .collect();
        for id in emptied {
            if let Err(e) = tree.delete(id) {
                tracing::warn!("Prose: the tree refused to drop an emptied block: {e:?}");
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
        let pos = pos as usize;
        let len = body.len_unicode();
        if pos > len {
            return Self::err_bad_range();
        }
        let Ok(tail) = body.slice(pos, len) else {
            return Self::err_bad_range();
        };
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
        if !tail.is_empty() {
            if let Err(e) = body.delete(pos, len - pos) {
                return Self::err_write(&format!("{e:?}"));
            }
            let new_body = doc::get_block_text(&self.doc, &new_block);
            if let Err(e) = new_body.insert(0, &tail) {
                return Self::err_write(&format!("{e:?}"));
            }
        }
        self.commit_with(vec![ProseEvent::BlockSplit {
            source: block_id(*id),
            new_block: block_id(new_block),
        }])
    }
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
}
