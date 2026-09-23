//! The list-item gestures: the marker, and the two directions of nesting
//! (`features/ambre-spec-prose-lists.md section 3`).
//!
//! There is no list container. An item is a block of the movable tree, and
//! nesting is the tree's own parent chain. Indenting is therefore one move of
//! one subtree (`PROSE-005`). Nothing here writes a depth or an ordinal: both are
//! read back from the tree when the list is drawn (`PROSE-FEAT-002`).

use loro::{LoroMap, TreeID, TreeParentId};

use crate::doc;
use crate::events::{BlockType, ListStyle, ProseOutcome};

use super::ProseState;

/// The parent a block sits under, or `None` at the root of the body tree.
///
/// A block the tree does not hold answers `None` as well, which every caller
/// here has already refused.
fn parent_of(tree: &loro::LoroTree, id: &TreeID) -> Option<TreeID> {
    match tree.parent(*id) {
        Some(TreeParentId::Node(parent)) => Some(parent),
        _ => None,
    }
}

/// The block just before `id` among its siblings, or `None` when it is first.
fn previous_sibling(tree: &loro::LoroTree, id: &TreeID) -> Option<TreeID> {
    let siblings = doc::children_of(tree, parent_of(tree, id));
    let here = siblings.iter().position(|sibling| sibling == id)?;
    siblings.get(here.checked_sub(1)?).copied()
}

impl ProseState {
    /// The marker a live list item carries, for a test and for the view.
    #[must_use]
    pub fn list_style_of(&self, id: &crate::events::BlockId) -> Option<ListStyle> {
        doc::list_style_of(&doc::blocks(&self.doc), &crate::events::tree_id(*id))
    }

    /// Write the three marker keys of one style, and remove the ones that
    /// style does not carry.
    ///
    /// A bullet keeps no ordinal spelling and no ticked state. Absence is the
    /// value, so a key that no longer applies is deleted rather than left at a
    /// stale one (`PROSE-096`).
    pub(super) fn write_list_style(attrs: &LoroMap, style: ListStyle) -> Result<(), String> {
        let fail = |e: loro::LoroError| format!("{e:?}");
        match style {
            ListStyle::Bullet => {
                attrs.insert(doc::A_MARKER, 0_i64).map_err(fail)?;
                attrs.delete(doc::A_SPELLING).map_err(fail)?;
                attrs.delete(doc::A_TICKED).map_err(fail)?;
            }
            ListStyle::Ordered(ordered) => {
                attrs.insert(doc::A_MARKER, 1_i64).map_err(fail)?;
                attrs
                    .insert(doc::A_SPELLING, i64::from(ordered.code()))
                    .map_err(fail)?;
                attrs.delete(doc::A_TICKED).map_err(fail)?;
            }
            ListStyle::Todo(ticked) => {
                attrs.insert(doc::A_MARKER, 2_i64).map_err(fail)?;
                attrs.delete(doc::A_SPELLING).map_err(fail)?;
                attrs.insert(doc::A_TICKED, ticked).map_err(fail)?;
            }
        }
        Ok(())
    }

    /// Set an item's marker (`PROSE-FEAT-140`, `PROSE-FEAT-278`).
    ///
    /// The text, the place in the tree and every other item are left alone
    /// (`PROSE-FEAT-141`). Changing one item's marker may break a numbering run
    /// in two, which is read back rather than written (`PROSE-FEAT-026`).
    pub(super) fn set_list_style(&mut self, id: &TreeID, style: ListStyle) -> ProseOutcome {
        let tree = doc::blocks(&self.doc);
        if doc::block_type_of(&tree, id) != Some(BlockType::ListItem) {
            return Self::err_no_block(id);
        }
        let Some(node) = doc::node_of(&tree, id) else {
            return Self::err_no_block(id);
        };
        let Some(attrs) = doc::map_at(&node, doc::K_ATTRS) else {
            return Self::err_write("the attribute map refused a container");
        };
        if let Err(e) = Self::write_list_style(&attrs, style) {
            return Self::err_write(&e);
        }
        self.commit_with(Vec::new())
    }

    /// Nest an item under the item before it (`PROSE-FEAT-142`).
    ///
    /// The descendants follow, because the tree moves a subtree as one
    /// (`PROSE-FEAT-143`). The first item of a run has no previous neighbour to
    /// become its parent. The command is refused and reported there, rather
    /// than silently dropped (`PROSE-FEAT-028`).
    pub(super) fn indent_list(&mut self, id: &TreeID) -> ProseOutcome {
        let tree = doc::blocks(&self.doc);
        if doc::block_type_of(&tree, id) != Some(BlockType::ListItem) {
            return Self::err_no_block(id);
        }
        let Some(previous) = previous_sibling(&tree, id) else {
            return Self::err_nesting();
        };
        if doc::block_type_of(&tree, &previous) != Some(BlockType::ListItem) {
            return Self::err_nesting();
        }
        if let Err(e) = tree.mov(*id, TreeParentId::Node(previous)) {
            return Self::err_write(&format!("{e:?}"));
        }
        self.commit_with(Vec::new())
    }

    /// Raise an item one level, or take it out of the list
    /// (`PROSE-FEAT-145`, `PROSE-FEAT-027`).
    ///
    /// A nested item becomes a child of its grandparent, placed after the
    /// subtree of its former parent. An item whose parent is no list item is
    /// already at the top level. It becomes an ordinary paragraph, which is how
    /// a user leaves a list. Its text stays (`PROSE-FEAT-146`).
    pub(super) fn unindent_list(&mut self, id: &TreeID) -> ProseOutcome {
        let tree = doc::blocks(&self.doc);
        if doc::block_type_of(&tree, id) != Some(BlockType::ListItem) {
            return Self::err_no_block(id);
        }
        let parent = parent_of(&tree, id)
            .filter(|parent| doc::block_type_of(&tree, parent) == Some(BlockType::ListItem));
        let Some(parent) = parent else {
            return self.leave_the_list(id);
        };
        if let Err(e) = tree.mov_after(*id, parent) {
            return Self::err_write(&format!("{e:?}"));
        }
        self.commit_with(Vec::new())
    }

    /// Turn a top-level item into an ordinary paragraph, keeping its text and
    /// its place (`PROSE-FEAT-027`).
    fn leave_the_list(&mut self, id: &TreeID) -> ProseOutcome {
        let tree = doc::blocks(&self.doc);
        let Some(node) = doc::node_of(&tree, id) else {
            return Self::err_no_block(id);
        };
        if let Err(e) = node.insert(doc::K_TYPE, i64::from(BlockType::Paragraph.code())) {
            return Self::err_write(&format!("{e:?}"));
        }
        if let Err(e) = node.delete(doc::K_ATTRS) {
            return Self::err_write(&format!("{e:?}"));
        }
        self.commit_with(Vec::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::{BlockId, BlockInit, OrderedStyle, ProseCommand, ProseEvent};

    /// Every live block in document order, with what a list gesture is allowed
    /// to leave alone: its type, its marker, its text and its parent.
    ///
    /// Comparing two of these is how the `FRAME` rows are held. A write that
    /// reached a block it had no business in moves one of the four.
    type Census = Vec<(
        BlockId,
        Option<BlockType>,
        Option<ListStyle>,
        String,
        Option<BlockId>,
    )>;

    fn census(state: &ProseState) -> Census {
        fn descend(state: &ProseState, ids: Vec<BlockId>, out: &mut Census) {
            for id in ids {
                out.push((
                    id,
                    state.block_type_of(&id),
                    state.list_style_of(&id),
                    state.text_of(&id).unwrap_or_default(),
                    state.parent_of(&id),
                ));
                descend(state, state.child_blocks(&id), out);
            }
        }
        let mut out = Census::new();
        descend(state, state.root_blocks(), &mut out);
        out
    }

    /// The census with one block left out, for a comparison across a change
    /// that block was the target of.
    fn census_without(state: &ProseState, id: BlockId) -> Census {
        census_without_row(&census(state), id)
    }

    /// The same, over a census taken earlier.
    fn census_without_row(taken: &Census, id: BlockId) -> Census {
        taken.iter().filter(|row| row.0 != id).cloned().collect()
    }

    /// Append one list item of `style` at the end of the document.
    fn item(state: &mut ProseState, style: ListStyle) -> BlockId {
        let last = *state.root_blocks().last().expect("the document has a root");
        state.handle_command(ProseCommand::InsertBlock {
            parent_id: None,
            after_id: Some(last),
            init: BlockInit::ListItem { style },
            style_id: None,
        });
        *state.root_blocks().last().expect("the item landed")
    }

    /// Whether an outcome is the refusal rather than a change.
    fn refused(outcome: &crate::events::ProseOutcome) -> bool {
        matches!(outcome.display.as_slice(), [ProseEvent::Error(_)])
    }

    /// The decimal style, which most of these tests number with.
    const DECIMAL: ListStyle = ListStyle::Ordered(OrderedStyle::Decimal);

    /// A new item lands under the given parent, after the given sibling, with
    /// the style it was asked for. Nothing else is written
    /// (`PROSE-FEAT-138`, `PROSE-FEAT-139`).
    #[test]
    fn a_new_item_takes_its_place_and_its_style_and_writes_nothing_else() {
        let mut state = ProseState::new().expect("a document is built");
        let host = item(&mut state, DECIMAL);
        let tail = item(&mut state, DECIMAL);
        let before = census(&state);

        state.handle_command(ProseCommand::InsertBlock {
            parent_id: Some(host),
            after_id: None,
            init: BlockInit::ListItem {
                style: ListStyle::Todo(true),
            },
            style_id: None,
        });
        let child = *state
            .child_blocks(&host)
            .first()
            .expect("the item landed under its parent");

        assert_eq!(state.parent_of(&child), Some(host));
        assert_eq!(state.list_style_of(&child), Some(ListStyle::Todo(true)));
        assert_eq!(state.block_type_of(&tail), Some(BlockType::ListItem));
        assert_eq!(census_without(&state, child), before);
    }

    /// A style change reaches the item's marker and nothing else
    /// (`PROSE-FEAT-140`, `PROSE-FEAT-278`, `PROSE-FEAT-141`).
    #[test]
    fn a_style_change_reaches_the_marker_alone() {
        let mut state = ProseState::new().expect("a document is built");
        let first = item(&mut state, DECIMAL);
        let second = item(&mut state, ListStyle::Bullet);
        state.handle_command(ProseCommand::InsertText {
            block_id: first,
            pos: 0,
            text: "kept".to_owned(),
        });
        let before = census(&state);

        state.handle_command(ProseCommand::SetListStyle {
            block_id: first,
            style: ListStyle::Todo(true),
        });
        assert_eq!(state.list_style_of(&first), Some(ListStyle::Todo(true)));
        // The ticked state is the one a checkbox carries, and it is read back.
        state.handle_command(ProseCommand::SetListStyle {
            block_id: first,
            style: ListStyle::Todo(false),
        });
        assert_eq!(state.list_style_of(&first), Some(ListStyle::Todo(false)));

        assert_eq!(state.text_of(&first).unwrap_or_default(), "kept");
        assert_eq!(state.parent_of(&first), None);
        assert_eq!(state.list_style_of(&second), Some(ListStyle::Bullet));
        // Only the first item's marker moved, so every other row is as it was.
        assert_eq!(
            census_without(&state, first),
            census_without_row(&before, first)
        );
    }

    /// An indented item becomes a child of its previous neighbour, its
    /// descendants follow it, and its text and style are untouched
    /// (`PROSE-FEAT-142`, `PROSE-FEAT-143`, `PROSE-FEAT-144`).
    #[test]
    fn an_indented_item_carries_its_subtree_under_its_neighbour() {
        let mut state = ProseState::new().expect("a document is built");
        let first = item(&mut state, DECIMAL);
        let second = item(&mut state, DECIMAL);
        state.handle_command(ProseCommand::InsertText {
            block_id: second,
            pos: 0,
            text: "kept".to_owned(),
        });
        state.handle_command(ProseCommand::InsertBlock {
            parent_id: Some(second),
            after_id: None,
            init: BlockInit::ListItem {
                style: ListStyle::Bullet,
            },
            style_id: None,
        });
        let child = *state.child_blocks(&second).first().expect("a child item");

        state.handle_command(ProseCommand::IndentList { block_id: second });

        assert_eq!(state.parent_of(&second), Some(first));
        // The subtree moved as one: the child kept its parent, which moved.
        assert_eq!(state.parent_of(&child), Some(second));
        assert_eq!(state.text_of(&second).unwrap_or_default(), "kept");
        assert_eq!(state.list_style_of(&second), Some(DECIMAL));
    }

    /// Indenting the first item of a run is refused and changes nothing
    /// (`PROSE-FEAT-028`).
    #[test]
    fn indenting_the_first_item_of_a_run_is_refused() {
        let mut state = ProseState::new().expect("a document is built");
        let first = item(&mut state, DECIMAL);
        item(&mut state, DECIMAL);
        // The root paragraph sits before the first item, and a paragraph is no
        // item to nest under. The first item of the run has no target.
        let before = census(&state);

        let outcome = state.handle_command(ProseCommand::IndentList { block_id: first });

        assert!(refused(&outcome), "{:?}", outcome.display);
        assert!(outcome.update.is_none(), "a refusal exports no delta");
        assert_eq!(census(&state), before);
    }

    /// A nested item rises to its grandparent, after its former parent's
    /// subtree, keeping its text (`PROSE-FEAT-145`, `PROSE-FEAT-146`).
    #[test]
    fn an_outdented_item_rises_after_the_subtree_it_leaves() {
        let mut state = ProseState::new().expect("a document is built");
        let first = item(&mut state, DECIMAL);
        let second = item(&mut state, DECIMAL);
        let third = item(&mut state, DECIMAL);
        state.handle_command(ProseCommand::IndentList { block_id: second });
        // Twice: the first indent puts the third item beside the second, under
        // the first, and the second indent nests it under the second.
        state.handle_command(ProseCommand::IndentList { block_id: third });
        state.handle_command(ProseCommand::IndentList { block_id: third });
        state.handle_command(ProseCommand::InsertText {
            block_id: third,
            pos: 0,
            text: "kept".to_owned(),
        });
        assert_eq!(state.parent_of(&third), Some(second));

        state.handle_command(ProseCommand::UnindentList { block_id: third });

        assert_eq!(state.parent_of(&third), Some(first));
        assert_eq!(state.child_blocks(&first), vec![second, third]);
        assert_eq!(state.text_of(&third).unwrap_or_default(), "kept");
        assert_eq!(state.list_style_of(&second), Some(DECIMAL));
    }

    /// Outdenting a top-level item turns it into an ordinary paragraph, and
    /// its text stays (`PROSE-FEAT-027`, `PROSE-FEAT-146`).
    #[test]
    fn outdenting_a_top_level_item_leaves_the_list() {
        let mut state = ProseState::new().expect("a document is built");
        let only = item(&mut state, DECIMAL);
        let neighbour = item(&mut state, DECIMAL);
        state.handle_command(ProseCommand::InsertText {
            block_id: only,
            pos: 0,
            text: "kept".to_owned(),
        });

        state.handle_command(ProseCommand::UnindentList { block_id: only });

        assert_eq!(state.block_type_of(&only), Some(BlockType::Paragraph));
        assert_eq!(state.list_style_of(&only), None);
        assert_eq!(state.text_of(&only).unwrap_or_default(), "kept");
        // The item it leaves behind keeps its own marker.
        assert_eq!(state.list_style_of(&neighbour), Some(DECIMAL));
    }

    /// A list gesture is refused on a block that is no list item.
    #[test]
    fn a_list_gesture_is_refused_on_a_block_that_is_no_item() {
        let mut state = ProseState::new().expect("a document is built");
        let paragraph = state.root_blocks()[0];
        for command in [
            ProseCommand::SetListStyle {
                block_id: paragraph,
                style: DECIMAL,
            },
            ProseCommand::IndentList {
                block_id: paragraph,
            },
            ProseCommand::UnindentList {
                block_id: paragraph,
            },
        ] {
            let outcome = state.handle_command(command);
            assert!(refused(&outcome), "{:?}", outcome.display);
        }
        assert_eq!(state.block_type_of(&paragraph), Some(BlockType::Paragraph));
    }
}
