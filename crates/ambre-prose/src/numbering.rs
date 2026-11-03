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
//! [`heading_numbers`] is the shape a reader of the whole document wants: one
//! walk, every heading. [`compute_heading_number`] answers for one heading and
//! stops there, so neither form turns a numbering pass into a walk per heading.

use ambre_core::Ordinal;
use ambre_core::Tally;
use std::ops::ControlFlow;

use loro::{LoroTree, TreeID};

use crate::doc;
use crate::events::{HeadingLevel, HeadingNumber, ListStyle, OrderedStyle};

/// The number an ordered list item is drawn with, one for the first item
/// of its run. It is counted, never stored.
pub enum ListItem {}

impl ambre_core::OrdinalKind for ListItem {
    const NAME: &'static str = "ListOrdinal";
}

/// The width of the counter vector: one counter per level the format declares.
const LEVELS: usize = HeadingLevel::ALL.len();

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
    /// A stored level outside one to six is brought inside before it gets
    /// here (`PROSE-FEAT-029`): it would index the counters outside them.
    fn bump(&mut self, level: HeadingLevel) -> Option<HeadingNumber> {
        let depth = HeadingLevel::ALL.iter().position(|each| *each == level)?;
        if let Some(counter) = self.0.get_mut(depth) {
            *counter = counter.saturating_add(1);
        }
        if let Some(below) = self.0.get_mut(depth + 1..) {
            below.fill(0);
        }
        HeadingNumber::new(self.0.iter().copied().take(depth + 1).collect())
    }
}

/// Every live heading of the document, in document order, with the number it is
/// drawn with (`PROSE-FEAT-030`).
///
/// One walk of the block tree, which is what the budget of the layout pass pays
/// for (`ambre-spec-budgets.md section 1`). A document with no heading answers
/// an empty vector.
#[must_use]
pub fn heading_numbers(tree: &LoroTree) -> Vec<(TreeID, HeadingNumber)> {
    let mut counters = Counters::new();
    let mut out = Vec::new();
    walk(tree, &mut |tree, id| {
        if let Some(stored) = doc::heading_level_of(tree, id)
            && let Some(number) = counters.bump(stored.clamped())
        {
            out.push((*id, number));
        }
        ControlFlow::Continue(())
    });
    out
}

/// The number one heading is drawn with, or `None` when the id names no live
/// heading (`PROSE-FEAT-030`).
///
/// The walk stops at the heading rather than numbering the rest of the
/// document. Numbering every heading is [`heading_numbers`], which costs one
/// walk for all of them.
#[must_use]
pub fn compute_heading_number(tree: &LoroTree, heading_id: &TreeID) -> Option<HeadingNumber> {
    let mut counters = Counters::new();
    let mut found = None;
    walk(tree, &mut |tree, id| {
        let Some(stored) = doc::heading_level_of(tree, id) else {
            return ControlFlow::Continue(());
        };
        let number = counters.bump(stored.clamped());
        if id == heading_id {
            found = number;
            return ControlFlow::Break(());
        }
        ControlFlow::Continue(())
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

/// The ordinal an ordered item is drawn with, `None` when the id names no
/// ordered item (`PROSE-FEAT-002`).
///
/// A run is the unbroken stretch of neighbours under one parent that share one
/// numbering style. Any other neighbour ends it: a paragraph, a bullet, a
/// checkbox, or an item numbered differently (`PROSE-FEAT-026`). A run that
/// resumes after one of those starts again at one (`PROSE-FEAT-136`).
///
/// The count reads the sibling list backwards from the item, so it costs the
/// length of the run rather than the length of the document.
#[must_use]
pub fn compute_ordinal(tree: &LoroTree, item_id: &TreeID) -> Option<Ordinal<ListItem>> {
    let style = ordered_style_of(tree, item_id)?;
    let parent = match tree.parent(*item_id) {
        Some(loro::TreeParentId::Node(parent)) => Some(parent),
        Some(loro::TreeParentId::Root) => None,
        _ => return None,
    };
    let siblings = doc::children_of(tree, parent);
    let here = siblings.iter().position(|sibling| sibling == item_id)?;
    let mut ordinal = Ordinal::<ListItem>::FIRST;
    for before in siblings.iter().take(here).rev() {
        if ordered_style_of(tree, before) != Some(style) {
            break;
        }
        ordinal = ordinal.next();
    }
    Some(ordinal)
}

/// How deep an item sits in its list, counting the list items above it in the
/// parent chain (`PROSE-005`).
///
/// A top-level item is at depth zero. The document stores none of this: the
/// answer is the tree's own parent chain, so it cannot disagree with the shape
/// the peers converged on.
#[must_use]
pub fn list_depth(tree: &LoroTree, item_id: &TreeID) -> Tally<ListDepth> {
    let mut depth = Tally::<ListDepth>::ZERO;
    let mut here = *item_id;
    while let Some(loro::TreeParentId::Node(parent)) = tree.parent(here) {
        if doc::block_type_of(tree, &parent) != Some(crate::events::BlockType::ListItem) {
            break;
        }
        depth = depth.incremented();
        here = parent;
    }
    depth
}

/// How many list items an item nests inside: zero for a top item.
pub enum ListDepth {}

impl ambre_core::TallyKind for ListDepth {
    type Repr = u32;
    const NAME: &'static str = "ListDepth";
}

/// Visit every live block in document order, depth first and parents before
/// children, until `visit` answers `Break`.
///
/// The same order the layout pass walks in, and the order the tree itself
/// holds, so no reader sorts anything.
fn walk(tree: &LoroTree, visit: &mut impl FnMut(&LoroTree, &TreeID) -> ControlFlow<()>) {
    fn descend(
        tree: &LoroTree,
        parent: Option<TreeID>,
        visit: &mut impl FnMut(&LoroTree, &TreeID) -> ControlFlow<()>,
    ) -> ControlFlow<()> {
        for id in doc::children_of(tree, parent) {
            visit(tree, &id)?;
            descend(tree, Some(id), visit)?;
        }
        ControlFlow::Continue(())
    }
    let _ = descend(tree, None, visit);
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
            init: BlockInit::Heading {
                level: HeadingLevel::new(level).expect("a level of one to six"),
            },
            style_id: None,
        });
        *state.root_blocks().last().expect("the heading landed")
    }

    /// Write a level no command can carry, the way an import leaves one
    /// (`PROSE-FEAT-094`).
    fn store_raw_level(state: &ProseState, id: &BlockId, level: i64) {
        let tree = doc::blocks(state.document());
        let node = doc::node_of(&tree, &crate::events::tree_id(*id)).expect("a live block");
        let attrs = doc::attrs_at(&node).expect("the attribute map");
        attrs
            .insert(doc::AttrKey::Level, level)
            .expect("the level is stored");
        state.document().commit();
    }

    /// Everything the document stores on one block.
    fn stored(state: &ProseState, id: &BlockId) -> loro::LoroValue {
        let tree = doc::blocks(state.document());
        doc::node_of(&tree, &crate::events::tree_id(*id))
            .expect("a live block")
            .map()
            .get_deep_value()
    }

    /// The numbers of the document, in document order.
    fn numbers(state: &ProseState) -> Vec<Vec<u32>> {
        heading_numbers(&doc::blocks(state.document()))
            .into_iter()
            .map(|(_, number)| number.parts().to_vec())
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
        let last = heading(&mut state, 1);
        assert_eq!(numbers(&state), vec![vec![1], vec![2]]);
        let before = stored(&state, &last);

        state.handle_command(ProseCommand::InsertBlock {
            parent_id: None,
            after_id: Some(first),
            init: BlockInit::Heading {
                level: HeadingLevel::H1,
            },
            style_id: None,
        });
        assert_eq!(numbers(&state), vec![vec![1], vec![2], vec![3]]);
        assert_eq!(
            stored(&state, &last),
            before,
            "the renumbered heading was written to"
        );
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
        assert_eq!(
            state.heading_level_of(&low).map(doc::StoredLevel::get),
            Some(0)
        );
        assert_eq!(
            state.heading_level_of(&high).map(doc::StoredLevel::get),
            Some(9)
        );
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
        let _ = compute_heading_number(
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
        for (id, number) in heading_numbers(&tree) {
            assert_eq!(compute_heading_number(&tree, &id), Some(number));
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
            compute_heading_number(&tree, &crate::events::tree_id(paragraph)).is_none(),
            "a paragraph carries no heading number"
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
    fn ordinal(state: &ProseState, id: &BlockId) -> Option<u32> {
        compute_ordinal(&doc::blocks(state.document()), &crate::events::tree_id(*id))
            .map(Ordinal::<ListItem>::get)
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

        assert_eq!(ordinal(&state, &first), Some(1));
        assert_eq!(ordinal(&state, &second), Some(2));
        // A bullet carries no ordinal, and it is what ends the run.
        assert_eq!(ordinal(&state, &bullet), None);
        // Another numbering style is another run, so it starts again at one.
        assert_eq!(ordinal(&state, &roman), Some(1));
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

        assert_eq!(ordinal(&state, &resumed), Some(1));
        assert_eq!(ordinal(&state, &after), Some(2));
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
        assert_eq!(
            list_depth(&tree, &crate::events::tree_id(first)),
            Tally::<ListDepth>::ZERO
        );
        assert_eq!(
            list_depth(&tree, &crate::events::tree_id(second)),
            Tally::<ListDepth>::new(1)
        );
        // The nested item opens a run of its own under its new parent.
        assert_eq!(ordinal(&state, &second), Some(1));
    }

    /// Reading an ordinal writes nothing, and no ordinal is stored
    /// (`PROSE-FEAT-002`).
    #[test]
    fn reading_the_ordinals_leaves_the_document_untouched() {
        let mut state = ProseState::new().expect("a document is built");
        let first = item(&mut state, DECIMAL);
        let second = item(&mut state, DECIMAL);
        let before = state.document().get_deep_value();
        assert_eq!(ordinal(&state, &first), Some(1));
        assert_eq!(ordinal(&state, &second), Some(2));
        assert_eq!(state.document().get_deep_value(), before);
    }

    /// An item inserted in the middle renumbers the ones after it, and no write
    /// reaches them (`PROSE-FEAT-002`, `PROSE-FEAT-139`).
    #[test]
    fn an_inserted_item_renumbers_the_ones_after_it() {
        let mut state = ProseState::new().expect("a document is built");
        let first = item(&mut state, DECIMAL);
        let last = item(&mut state, DECIMAL);
        assert_eq!(ordinal(&state, &last), Some(2));
        let before = stored(&state, &last);

        state.handle_command(ProseCommand::InsertBlock {
            parent_id: None,
            after_id: Some(first),
            init: BlockInit::ListItem { style: DECIMAL },
            style_id: None,
        });
        assert_eq!(ordinal(&state, &last), Some(3));
        assert_eq!(
            stored(&state, &last),
            before,
            "the renumbered item was written to"
        );
    }

    /// A block that is no ordered item has no ordinal.
    #[test]
    fn a_block_that_is_no_ordered_item_has_no_ordinal() {
        let mut state = ProseState::new().expect("a document is built");
        let bullet = item(&mut state, ListStyle::Bullet);
        let ticked = item(&mut state, ListStyle::Todo(crate::events::TodoMark::Open));
        let text = paragraph(&mut state);

        assert_eq!(ordinal(&state, &bullet), None);
        assert_eq!(ordinal(&state, &ticked), None);
        assert_eq!(ordinal(&state, &text), None);
    }
}
