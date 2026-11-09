//! An undo that puts a block back, against a page rebuilt by hand
//! (`PROSE-133`, `PROSE-134`).
//!
//! A deletion hides the block and keeps its node (`PROSE-FEAT-133`), so an undo
//! of it unhides the same node. An undo of an insertion hides the node too, so
//! its redo shows the same block, its body still keyed by it (`PROSE-134`).
//! The oracle is a page kept by hand. It holds the root blocks in order, each
//! with its text, and a stack of the pages an undo returns to.

use ambre_core::index::Pos;
use proptest::prelude::*;

use crate::events::BlockId;
use crate::logic::ProseState;
use crate::message::{BlockInit, DocChar, ProseCommand};

/// What the page shows: each root block in order, with its text.
type Page = Vec<(BlockId, String)>;

fn page_of(state: &ProseState) -> Page {
    state
        .root_blocks()
        .into_iter()
        .map(|id| {
            let text = state.text_of(&id).expect("a live block has a body");
            (id, text)
        })
        .collect()
}

/// Insert a paragraph after `after` and return the block the engine minted.
fn insert_after(state: &mut ProseState, after: Option<BlockId>) -> (BlockId, bool) {
    let before = state.root_blocks();
    let out = state.handle_command(ProseCommand::InsertBlock {
        parent_id: None,
        after_id: after,
        init: BlockInit::Paragraph,
        style_id: None,
    });
    let id = state
        .root_blocks()
        .into_iter()
        .find(|id| !before.contains(id))
        .expect("an insert adds a root block");
    (id, out.update.is_some())
}

fn type_into(state: &mut ProseState, block: BlockId, text: &str) -> bool {
    state
        .handle_command(ProseCommand::InsertText {
            block_id: block,
            pos: Pos::<DocChar>::ZERO,
            text: text.to_owned(),
        })
        .update
        .is_some()
}

fn walk(state: &mut ProseState, steps: Vec<(ProseCommand, Page)>) {
    for (step, (command, expected)) in steps.into_iter().enumerate() {
        assert!(
            state.handle_command(command).update.is_some(),
            "step {step} writes"
        );
        assert_eq!(page_of(state), expected, "step {step}");
    }
}

/// `PROSE-133`: insert, delete, undo, undo puts the deleted block back, then
/// takes the insertion back.
#[test]
fn an_undone_delete_then_an_undone_insert_leaves_no_block() {
    let mut state = ProseState::new().expect("a document is built");
    let first = state.root_blocks()[0];
    let (second, _) = insert_after(&mut state, Some(first));
    state.handle_command(ProseCommand::DeleteBlock { block_id: second });
    let one = vec![(first, String::new())];
    let two = vec![(first, String::new()), (second, String::new())];
    walk(
        &mut state,
        vec![(ProseCommand::Undo, two), (ProseCommand::Undo, one)],
    );
}

/// `PROSE-134`: a typed block inserted, undone twice and redone twice, comes
/// back as the same block with its text.
#[test]
fn a_redone_insert_puts_back_the_same_block() {
    let mut state = ProseState::new().expect("a document is built");
    let first = state.root_blocks()[0];
    let (second, _) = insert_after(&mut state, Some(first));
    type_into(&mut state, second, "kept");
    let one = vec![(first, String::new())];
    let blank = vec![(first, String::new()), (second, String::new())];
    let typed = vec![(first, String::new()), (second, "kept".to_owned())];
    walk(
        &mut state,
        vec![
            (ProseCommand::Undo, blank.clone()),
            (ProseCommand::Undo, one),
            (ProseCommand::Redo, blank),
            (ProseCommand::Redo, typed),
        ],
    );
}

/// `PROSE-133`: a block is typed, moved and deleted, then undone three times.
/// It comes back at its new place, then at its old place, then without its
/// text. The redo chain walks the same pages forward.
#[test]
fn an_undone_delete_after_a_move_restores_the_place_and_the_text() {
    let mut state = ProseState::new().expect("a document is built");
    let a = state.root_blocks()[0];
    let (b, _) = insert_after(&mut state, Some(a));
    let (c, _) = insert_after(&mut state, Some(b));
    type_into(&mut state, a, "kept");
    state.handle_command(ProseCommand::MoveBlock {
        block_id: a,
        new_parent: None,
        after_id: Some(c),
    });
    state.handle_command(ProseCommand::DeleteBlock { block_id: a });
    let empty = String::new;
    let kept = || "kept".to_owned();
    let typed = vec![(a, kept()), (b, empty()), (c, empty())];
    let moved = vec![(b, empty()), (c, empty()), (a, kept())];
    let blank = vec![(a, empty()), (b, empty()), (c, empty())];
    let gone = vec![(b, empty()), (c, empty())];
    walk(
        &mut state,
        vec![
            (ProseCommand::Undo, moved.clone()),
            (ProseCommand::Undo, typed.clone()),
            (ProseCommand::Undo, blank),
            (ProseCommand::Redo, typed),
            (ProseCommand::Redo, moved),
            (ProseCommand::Redo, gone),
        ],
    );
}

/// One step of the property. An index picks a live root block modulo the page.
#[derive(Clone, Debug)]
enum Op {
    Insert(usize),
    Delete(usize),
    Move(usize, usize),
    Type(usize, char),
    Undo,
    Redo,
}

fn op() -> impl Strategy<Value = Op> {
    prop_oneof![
        3 => any::<usize>().prop_map(Op::Insert),
        2 => any::<usize>().prop_map(Op::Delete),
        2 => (any::<usize>(), any::<usize>()).prop_map(|(i, j)| Op::Move(i, j)),
        2 => (any::<usize>(), prop::char::range('a', 'z')).prop_map(|(i, c)| Op::Type(i, c)),
        3 => Just(Op::Undo),
        2 => Just(Op::Redo),
    ]
}

/// The page kept by hand, and the two stacks a history step walks.
struct Oracle {
    page: Page,
    undo: Vec<Page>,
    redo: Vec<Page>,
}

impl Oracle {
    /// Record a forward step: the old page goes on the undo stack, and the
    /// redo stack empties, as a new edit after an undo does.
    fn edit(&mut self, next: Page) {
        let before = std::mem::replace(&mut self.page, next);
        self.undo.push(before);
        self.redo.clear();
    }

    /// Walk one history step, or `None` on an empty stack.
    fn history(&mut self, undo: bool) -> Option<()> {
        let (from, to) = if undo {
            (&mut self.undo, &mut self.redo)
        } else {
            (&mut self.redo, &mut self.undo)
        };
        let page = from.pop()?;
        to.push(std::mem::replace(&mut self.page, page));
        Some(())
    }
}

/// Run one step on the engine and on the oracle. The oracle rebuilds the page
/// by hand from the step alone, and reads from the engine only the id it
/// minted. Returns whether the engine wrote, and whether the oracle expected
/// a write. A step the engine refuses by design is not run.
fn run(state: &mut ProseState, oracle: &mut Oracle, op: &Op) -> (bool, bool) {
    let len = oracle.page.len();
    let pick = |i: usize| oracle.page[i % len].0;
    match *op {
        Op::Insert(i) => {
            let at = (len > 0).then(|| i % len);
            let (id, wrote) = insert_after(state, at.map(pick));
            let mut next = oracle.page.clone();
            next.insert(at.map_or(0, |at| at + 1), (id, String::new()));
            oracle.edit(next);
            (wrote, true)
        }
        // A page keeps one block here: a delete of the last block is a rule
        // of its own, and this property is about identity.
        Op::Delete(i) if len > 1 => {
            let wrote = state
                .handle_command(ProseCommand::DeleteBlock { block_id: pick(i) })
                .update
                .is_some();
            let mut next = oracle.page.clone();
            next.remove(i % len);
            oracle.edit(next);
            (wrote, true)
        }
        Op::Move(i, j) if len > 1 && i % len != j % len => {
            let (block, after) = (pick(i), pick(j));
            let wrote = state
                .handle_command(ProseCommand::MoveBlock {
                    block_id: block,
                    new_parent: None,
                    after_id: Some(after),
                })
                .update
                .is_some();
            let mut next = oracle.page.clone();
            let entry = next.remove(i % len);
            let at = next
                .iter()
                .position(|(id, _)| *id == after)
                .expect("the sibling stays");
            next.insert(at + 1, entry);
            // A move onto the place the block already holds may write or not.
            // It changes no page, so the oracle follows the engine there.
            if wrote {
                oracle.edit(next);
            }
            (wrote, wrote)
        }
        Op::Type(i, c) if len > 0 => {
            let wrote = type_into(state, pick(i), &c.to_string());
            let mut next = oracle.page.clone();
            next[i % len].1.insert(0, c);
            oracle.edit(next);
            (wrote, true)
        }
        Op::Undo | Op::Redo => {
            let undo = matches!(op, Op::Undo);
            let command = if undo {
                ProseCommand::Undo
            } else {
                ProseCommand::Redo
            };
            let wrote = state.handle_command(command).update.is_some();
            (wrote, oracle.history(undo).is_some())
        }
        _ => (false, false),
    }
}

/// A page of four typed root blocks, with no history on this peer. The peer
/// loads them from another peer, and a load is no step of the undo stack.
fn seeded() -> ProseState {
    let mut seed = ProseState::new().expect("a document is built");
    let mut last = seed.root_blocks()[0];
    type_into(&mut seed, last, "a");
    for text in ["b", "c", "d"] {
        let (id, _) = insert_after(&mut seed, Some(last));
        type_into(&mut seed, id, text);
        last = id;
    }
    let mut state =
        ProseState::with_peer(ambre_core::PeerId::new(41)).expect("a document is built");
    state
        .import_snapshot(&seed.export_snapshot())
        .expect("the seed imports");
    state
}

fn check(state: &mut ProseState, ops: &[Op]) -> Result<(), TestCaseError> {
    let mut oracle = Oracle {
        page: page_of(state),
        undo: Vec::new(),
        redo: Vec::new(),
    };
    for (step, op) in ops.iter().enumerate() {
        let (wrote, expected) = run(state, &mut oracle, op);
        prop_assert_eq!(wrote, expected, "step {} {:?}: a write", step, op);
        prop_assert_eq!(&page_of(state), &oracle.page, "step {} {:?}", step, op);
    }
    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// `PROSE-133`: on a loaded page, any run of deletes, moves, typing, undos
    /// and redos leaves the page the oracle rebuilt by hand. No step inserts,
    /// so no redo creates a block.
    #[test]
    fn undo_and_redo_follow_a_loaded_page_rebuilt_by_hand(
        ops in prop::collection::vec(op().prop_filter("no insert", |op| !matches!(op, Op::Insert(_))), 1..32)
    ) {
        let mut state = seeded();
        prop_assert_eq!(state.root_blocks().len(), 4);
        check(&mut state, &ops)?;
    }

    /// `PROSE-134`: the same run with inserts, from a fresh page.
    #[test]
    fn undo_and_redo_follow_a_page_rebuilt_by_hand(ops in prop::collection::vec(op(), 1..32)) {
        let mut state = ProseState::new().expect("a document is built");
        check(&mut state, &ops)?;
    }
}
