//! The number drawn in front of a heading, computed from the document order
//! (`features/ambre-spec-prose-headings.md section 3`).
//!
//! Nothing here writes. The number is a vector of counters, one per level, and
//! the document holds none of it (`PROSE-FEAT-090`). Two peers whose trees have
//! converged read the same order, so they draw the same numbers without ever
//! agreeing on one.
//!
//! The list ordinal is the same idea one level down: [`compute_ordinal`] counts
//! the unbroken run of neighbours an ordered item belongs to
//! (`features/ambre-spec-prose-lists.md section 3`). It is read the same way
//! and stored just as little (`PROSE-FEAT-002`).
//!
//! [`section_numbers`] is the shape a reader of the whole document wants: one
//! walk, every heading. [`compute_section_number`] answers for one heading and
//! stops there, so neither form turns a numbering pass into a walk per heading.

use loro::{LoroTree, TreeID};

use crate::doc;
use crate::events::{ListStyle, OrderedStyle};

/// The levels the format declares, and the width of the counter vector.
const LEVELS: usize = 6;

/// The counters of a numbering walk, one per level.
///
/// A heading of level `n` bumps the `n`th counter and clears every counter
/// below it, so a new chapter restarts its sections.
struct Counters([u32; LEVELS]);

impl Counters {
    fn new() -> Self {
        Self([0; LEVELS])
    }

    /// Count one heading of `level`, and answer the number it is drawn with.
    ///
    /// `level` is one-based and already inside the range.
    fn bump(&mut self, level: usize) -> Vec<u32> {
        if let Some(counter) = self.0.get_mut(level - 1) {
            *counter = counter.saturating_add(1);
        }
        if let Some(below) = self.0.get_mut(level..) {
            below.fill(0);
        }
        self.0.iter().copied().take(level).collect()
    }
}

/// One heading level, brought inside the six the format declares
/// (`PROSE-FEAT-029`).
///
/// Read-side only: the stored level is never rewritten (`PROSE-FEAT-091`). A
/// level of zero would index the counter vector below its first component, and
/// a level above six outside its last.
fn numbering_level(stored: u8) -> usize {
    usize::from(stored.clamp(1, 6))
}

/// Every live heading of the document, in document order, with the number it is
/// drawn with (`PROSE-FEAT-030`).
///
/// One walk of the block tree, which is what the budget of the layout pass pays
/// for (`ambre-spec-budgets.md section 1`). A document with no heading answers
/// an empty vector.
#[must_use]
pub fn section_numbers(tree: &LoroTree) -> Vec<(TreeID, Vec<u32>)> {
    let mut counters = Counters::new();
    let mut out = Vec::new();
    walk(tree, &mut |tree, id| {
        if let Some(stored) = doc::heading_level_of(tree, id) {
            out.push((*id, counters.bump(numbering_level(stored))));
        }
        false
    });
    out
}

/// The number one heading is drawn with, or an empty vector when the id names
/// no live heading (`PROSE-FEAT-030`).
///
/// The walk stops at the heading rather than numbering the rest of the
/// document. Numbering every heading is [`section_numbers`], which costs one
/// walk for all of them.
#[must_use]
pub fn compute_section_number(tree: &LoroTree, heading_id: &TreeID) -> Vec<u32> {
    let mut counters = Counters::new();
    let mut found = Vec::new();
    walk(tree, &mut |tree, id| {
        let Some(stored) = doc::heading_level_of(tree, id) else {
            return false;
        };
        let number = counters.bump(numbering_level(stored));
        if id == heading_id {
            found = number;
            return true;
        }
        false
    });
    found
}

/// The numbering style of a block, when it is an ordered list item.
///
/// A bullet, a checkbox, and a block that is no list item all answer `None`,
/// and each of those breaks a run (`PROSE-FEAT-026`).
fn ordered_style_of(tree: &LoroTree, id: &TreeID) -> Option<OrderedStyle> {
    match doc::list_style_of(tree, id) {
        Some(ListStyle::Ordered(style)) => Some(style),
        _ => None,
    }
}

/// The one-based ordinal an ordered item is drawn with, or zero when the id
/// names no ordered item (`PROSE-FEAT-002`).
///
/// A run is the unbroken stretch of neighbours under one parent that share one
/// numbering style. Any other neighbour ends it: a paragraph, a bullet, a
/// checkbox, or an item numbered differently (`PROSE-FEAT-026`). A run that
/// resumes after one of those starts again at one (`PROSE-FEAT-136`).
///
/// The count reads the sibling list backwards from the item, so it costs the
/// length of the run rather than the length of the document.
#[must_use]
pub fn compute_ordinal(tree: &LoroTree, item_id: &TreeID) -> u32 {
    let Some(style) = ordered_style_of(tree, item_id) else {
        return 0;
    };
    let parent = match tree.parent(*item_id) {
        Some(loro::TreeParentId::Node(parent)) => Some(parent),
        Some(loro::TreeParentId::Root) => None,
        _ => return 0,
    };
    let siblings = doc::children_of(tree, parent);
    let Some(here) = siblings.iter().position(|sibling| sibling == item_id) else {
        return 0;
    };
    let mut ordinal = 1_u32;
    for before in siblings[..here].iter().rev() {
        if ordered_style_of(tree, before) != Some(style) {
            break;
        }
        ordinal = ordinal.saturating_add(1);
    }
    ordinal
}

/// How deep an item sits in its list, counting the list items above it in the
/// parent chain (`PROSE-005`).
///
/// A top-level item is at depth zero. The document stores none of this: the
/// answer is the tree's own parent chain, so it cannot disagree with the shape
/// the peers converged on.
#[must_use]
pub fn list_depth(tree: &LoroTree, item_id: &TreeID) -> u32 {
    let mut depth = 0_u32;
    let mut here = *item_id;
    while let Some(loro::TreeParentId::Node(parent)) = tree.parent(here) {
        if doc::block_type_of(tree, &parent) != Some(crate::events::BlockType::ListItem) {
            break;
        }
        depth = depth.saturating_add(1);
        here = parent;
    }
    depth
}

/// Visit every live block in document order, depth first and parents before
/// children, until `visit` answers `true`.
///
/// The same order the layout pass walks in, and the order the tree itself
/// holds, so no reader sorts anything.
fn walk(tree: &LoroTree, visit: &mut impl FnMut(&LoroTree, &TreeID) -> bool) {
    fn descend(
        tree: &LoroTree,
        parent: Option<TreeID>,
        visit: &mut impl FnMut(&LoroTree, &TreeID) -> bool,
    ) -> bool {
        for id in doc::children_of(tree, parent) {
            if visit(tree, &id) || descend(tree, Some(id), visit) {
                return true;
            }
        }
        false
    }
    descend(tree, None, visit);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::{BlockId, BlockInit, ListStyle, OrderedStyle, ProseCommand};
    use crate::logic::ProseState;

    /// Append one heading of `level` at the end of the document.
    fn heading(state: &mut ProseState, level: u8) -> BlockId {
        let last = *state.root_blocks().last().expect("the document has a root");
        state.handle_command(ProseCommand::InsertBlock {
            parent_id: None,
            after_id: Some(last),
            init: BlockInit::Heading { level },
            style_id: None,
        });
        *state.root_blocks().last().expect("the heading landed")
    }

    /// Write a level no command can carry, the way an import leaves one
    /// (`PROSE-FEAT-094`).
    fn store_raw_level(state: &ProseState, id: &BlockId, level: i64) {
        let tree = doc::blocks(state.document());
        let node = doc::node_of(&tree, &crate::events::tree_id(*id)).expect("a live block");
        let attrs = doc::map_at(&node, doc::K_ATTRS).expect("the attribute map");
        attrs
            .insert(doc::A_LEVEL, level)
            .expect("the level is stored");
        state.document().commit();
    }

    /// The numbers of the document, in document order.
    fn numbers(state: &ProseState) -> Vec<Vec<u32>> {
        section_numbers(&doc::blocks(state.document()))
            .into_iter()
            .map(|(_, number)| number)
            .collect()
    }

    /// The number a heading is drawn with is the one its position implies
    /// (`PROSE-FEAT-030`).
    #[test]
    fn a_heading_is_numbered_by_the_headings_before_it() {
        let mut state = ProseState::new().expect("a document is built");
        for level in [1, 2, 2, 1, 2, 3] {
            heading(&mut state, level);
        }
        assert_eq!(
            numbers(&state),
            vec![
                vec![1],
                vec![1, 1],
                vec![1, 2],
                vec![2],
                vec![2, 1],
                vec![2, 1, 1],
            ]
        );
    }

    /// A heading inserted in the middle renumbers the ones after it, and
    /// nothing is written to them (`PROSE-FEAT-030`, `PROSE-FEAT-090`).
    #[test]
    fn an_insertion_renumbers_what_follows_it() {
        let mut state = ProseState::new().expect("a document is built");
        let first = heading(&mut state, 1);
        heading(&mut state, 1);
        assert_eq!(numbers(&state), vec![vec![1], vec![2]]);

        state.handle_command(ProseCommand::InsertBlock {
            parent_id: None,
            after_id: Some(first),
            init: BlockInit::Heading { level: 1 },
            style_id: None,
        });
        assert_eq!(numbers(&state), vec![vec![1], vec![2], vec![3]]);
    }

    /// A stored level outside one to six is numbered as the nearest level
    /// inside it, and stays as it was stored (`PROSE-FEAT-029`,
    /// `PROSE-FEAT-091`).
    #[test]
    fn a_level_outside_the_range_is_numbered_at_the_nearest_one() {
        let mut state = ProseState::new().expect("a document is built");
        let low = heading(&mut state, 1);
        let high = heading(&mut state, 1);
        store_raw_level(&state, &low, 0);
        store_raw_level(&state, &high, 9);

        // Zero numbers as a level one, and nine as a level six under it.
        assert_eq!(numbers(&state), vec![vec![1], vec![1, 0, 0, 0, 0, 1]]);
        assert_eq!(state.heading_level_of(&low), Some(0));
        assert_eq!(state.heading_level_of(&high), Some(9));
    }

    /// Numbering writes nothing: the document after a numbering pass is the
    /// document before it (`PROSE-FEAT-090`, `PROSE-FEAT-091`).
    ///
    /// The number and the place in the outline are derived on every read. A
    /// document that held either would have to rewrite them on each insertion,
    /// and two peers would then merge two numberings.
    #[test]
    fn numbering_leaves_the_document_untouched() {
        let mut state = ProseState::new().expect("a document is built");
        for level in [1, 3, 2] {
            heading(&mut state, level);
        }
        let before = state.document().get_deep_value();
        let _ = numbers(&state);
        let _ = compute_section_number(
            &doc::blocks(state.document()),
            &crate::events::tree_id(state.root_blocks()[1]),
        );
        assert_eq!(state.document().get_deep_value(), before);
    }

    /// One heading's number is the one the whole walk gives it
    /// (`PROSE-FEAT-030`).
    #[test]
    fn one_heading_reads_the_same_number_as_the_walk() {
        let mut state = ProseState::new().expect("a document is built");
        for level in [1, 2, 2, 1] {
            heading(&mut state, level);
        }
        let tree = doc::blocks(state.document());
        for (id, number) in section_numbers(&tree) {
            assert_eq!(compute_section_number(&tree, &id), number);
        }
    }

    /// A block that is not a live heading has no number (`PROSE-FEAT-030`).
    #[test]
    fn a_block_that_is_no_heading_has_no_number() {
        let mut state = ProseState::new().expect("a document is built");
        let paragraph = state.root_blocks()[0];
        heading(&mut state, 1);
        let tree = doc::blocks(state.document());
        assert!(
            compute_section_number(&tree, &crate::events::tree_id(paragraph)).is_empty(),
            "a paragraph carries no section number"
        );
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

    /// Append one paragraph at the end of the document.
    fn paragraph(state: &mut ProseState) -> BlockId {
        let last = *state.root_blocks().last().expect("the document has a root");
        state.handle_command(ProseCommand::InsertBlock {
            parent_id: None,
            after_id: Some(last),
            init: BlockInit::Paragraph,
            style_id: None,
        });
        *state.root_blocks().last().expect("the paragraph landed")
    }

    /// The ordinal of one block.
    fn ordinal(state: &ProseState, id: &BlockId) -> u32 {
        compute_ordinal(&doc::blocks(state.document()), &crate::events::tree_id(*id))
    }

    /// The decimal style, which most of these tests number with.
    const DECIMAL: ListStyle = ListStyle::Ordered(OrderedStyle::Decimal);

    /// Neighbours of one style are numbered one after the other, and a
    /// neighbour of any other kind ends the run (`PROSE-FEAT-026`).
    #[test]
    fn an_ordinal_counts_the_unbroken_run_of_one_style() {
        let mut state = ProseState::new().expect("a document is built");
        let first = item(&mut state, DECIMAL);
        let second = item(&mut state, DECIMAL);
        let bullet = item(&mut state, ListStyle::Bullet);
        let roman = item(&mut state, ListStyle::Ordered(OrderedStyle::LowerRoman));

        assert_eq!(ordinal(&state, &first), 1);
        assert_eq!(ordinal(&state, &second), 2);
        // A bullet carries no ordinal, and it is what ends the run.
        assert_eq!(ordinal(&state, &bullet), 0);
        // Another numbering style is another run, so it starts again at one.
        assert_eq!(ordinal(&state, &roman), 1);
    }

    /// A run that resumes after an interruption starts again at one
    /// (`PROSE-FEAT-136`).
    #[test]
    fn a_run_that_resumes_after_an_interruption_starts_at_one() {
        let mut state = ProseState::new().expect("a document is built");
        item(&mut state, DECIMAL);
        item(&mut state, DECIMAL);
        paragraph(&mut state);
        let resumed = item(&mut state, DECIMAL);
        let after = item(&mut state, DECIMAL);

        assert_eq!(ordinal(&state, &resumed), 1);
        assert_eq!(ordinal(&state, &after), 2);
    }

    /// A nested item is numbered among its own siblings, and its depth is the
    /// parent chain rather than a stored number (`PROSE-005`,
    /// `PROSE-FEAT-026`).
    #[test]
    fn a_nested_run_is_numbered_and_measured_on_its_own_level() {
        let mut state = ProseState::new().expect("a document is built");
        let first = item(&mut state, DECIMAL);
        let second = item(&mut state, DECIMAL);
        state.handle_command(ProseCommand::IndentList { block_id: second });

        let tree = doc::blocks(state.document());
        assert_eq!(list_depth(&tree, &crate::events::tree_id(first)), 0);
        assert_eq!(list_depth(&tree, &crate::events::tree_id(second)), 1);
        // The nested item opens a run of its own under its new parent.
        assert_eq!(ordinal(&state, &second), 1);
    }

    /// Reading an ordinal writes nothing, and no ordinal is stored
    /// (`PROSE-FEAT-002`).
    #[test]
    fn reading_the_ordinals_leaves_the_document_untouched() {
        let mut state = ProseState::new().expect("a document is built");
        let first = item(&mut state, DECIMAL);
        let second = item(&mut state, DECIMAL);
        let before = state.document().get_deep_value();
        assert_eq!(ordinal(&state, &first), 1);
        assert_eq!(ordinal(&state, &second), 2);
        assert_eq!(state.document().get_deep_value(), before);
    }

    /// An item inserted in the middle renumbers the ones after it, and no write
    /// reaches them (`PROSE-FEAT-002`, `PROSE-FEAT-139`).
    #[test]
    fn an_inserted_item_renumbers_the_ones_after_it() {
        let mut state = ProseState::new().expect("a document is built");
        let first = item(&mut state, DECIMAL);
        let last = item(&mut state, DECIMAL);
        assert_eq!(ordinal(&state, &last), 2);

        state.handle_command(ProseCommand::InsertBlock {
            parent_id: None,
            after_id: Some(first),
            init: BlockInit::ListItem { style: DECIMAL },
            style_id: None,
        });
        assert_eq!(ordinal(&state, &last), 3);
    }

    /// A block that is no ordered item has no ordinal (`PROSE-FEAT-002`).
    #[test]
    fn a_block_that_is_no_ordered_item_has_no_ordinal() {
        let mut state = ProseState::new().expect("a document is built");
        let bullet = item(&mut state, ListStyle::Bullet);
        let ticked = item(&mut state, ListStyle::Todo(false));
        let text = paragraph(&mut state);

        assert_eq!(ordinal(&state, &bullet), 0);
        assert_eq!(ordinal(&state, &ticked), 0);
        assert_eq!(ordinal(&state, &text), 0);
    }
}
