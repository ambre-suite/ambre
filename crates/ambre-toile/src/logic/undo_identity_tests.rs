//! An undo that puts a deleted element back, against a board rebuilt by hand
//! (`TOILE-290`).
//!
//! An undo of a delete writes a new element record and a new entry of the
//! paint order. The undo manager names each step by the Loro identity it wrote,
//! so a later step could miss the element the undo put back. These tests hold
//! that every later undo and redo still reaches it. The oracle is a board kept
//! by hand. It holds the ids in paint order, each with its box, and a stack of
//! the boards an undo returns to.

use ambre_core::id::{Element, Id};
use ambre_core::index::Pos;
use kurbo::{Point, Rect, Size, Vec2};
use proptest::prelude::*;

use crate::logic::command::{InitContent, ToileCommand, ToileEvent, ToileOutcome, ZTarget};
use crate::logic::model::{ElementKind, NodeId, ShapeKind};
use crate::logic::state::ToileState;

/// What the board shows: each node in paint order, with its box.
type Board = Vec<(Id<Element>, Rect)>;

fn created_id(out: &ToileOutcome) -> Id<Element> {
    out.display
        .iter()
        .find_map(|e| match e {
            ToileEvent::ElementCreated { id, .. } => Some(id.clone()),
            _ => None,
        })
        .expect("a create announces its element")
}

fn create(st: &mut ToileState, x: f64) -> (Id<Element>, ToileOutcome) {
    let out = st.handle_command(ToileCommand::CreateElement {
        kind: ElementKind::Shape(ShapeKind::Rect),
        at: Point::new(x, 0.0),
        size: Size::new(10.0, 10.0),
        content: InitContent::None,
        fill: None,
        correlate: None,
    });
    (created_id(&out), out)
}

/// The board the engine shows. A node with no box reads as a defect, since
/// every element these tests write has one.
fn board_of(st: &ToileState) -> Board {
    st.layer_order()
        .iter()
        .map(|node| match node {
            NodeId::Element(id) => (
                id.clone(),
                st.bounds_of(id).expect("a painted element has a box"),
            ),
            NodeId::Connector(_) => panic!("these tests write no connector"),
        })
        .collect()
}

fn rect_at(x: f64) -> Rect {
    Rect::new(x, 0.0, x + 10.0, 10.0)
}

/// `TOILE-290`: insert, delete, undo, undo leaves the board empty, and the
/// redo chain walks the same boards forward.
#[test]
fn an_undone_delete_then_an_undone_create_leaves_no_element() {
    let mut st = ToileState::new();
    let (a, _) = create(&mut st, 0.0);
    st.handle_command(ToileCommand::DeleteElements {
        ids: vec![a.clone()],
    });
    let walk = [
        (ToileCommand::Undo, vec![(a.clone(), rect_at(0.0))]),
        (ToileCommand::Undo, vec![]),
        (ToileCommand::Redo, vec![(a.clone(), rect_at(0.0))]),
        (ToileCommand::Redo, vec![]),
    ];
    for (step, (command, expected)) in walk.into_iter().enumerate() {
        assert!(
            st.handle_command(command).update.is_some(),
            "step {step} writes"
        );
        assert_eq!(board_of(&st), expected, "step {step}");
    }
    assert!(st.handle_command(ToileCommand::Redo).update.is_none());
}

/// `TOILE-290`: an element is reordered, moved and deleted, then undone three
/// times. It comes back with its new box, then its old box, then its old
/// place. The redo chain walks the same boards forward.
#[test]
fn an_undone_delete_after_a_reorder_and_a_move_restores_both() {
    let mut st = ToileState::new();
    let (a, _) = create(&mut st, 0.0);
    let (b, _) = create(&mut st, 20.0);
    let (c, _) = create(&mut st, 40.0);
    st.handle_command(ToileCommand::Reorder {
        node: NodeId::Element(a.clone()),
        to: ZTarget::Front,
    });
    st.handle_command(ToileCommand::MoveElements {
        ids: vec![a.clone()],
        delta: Vec2::new(5.0, 0.0),
    });
    st.handle_command(ToileCommand::DeleteElements {
        ids: vec![a.clone()],
    });
    let (ra, rb, rc) = (rect_at(0.0), rect_at(20.0), rect_at(40.0));
    let moved = rect_at(5.0);
    let start = vec![(a.clone(), ra), (b.clone(), rb), (c.clone(), rc)];
    let fronted = vec![(b.clone(), rb), (c.clone(), rc), (a.clone(), ra)];
    let shifted = vec![(b.clone(), rb), (c.clone(), rc), (a.clone(), moved)];
    let gone = vec![(b.clone(), rb), (c.clone(), rc)];
    let walk = [
        (ToileCommand::Undo, shifted.clone()),
        (ToileCommand::Undo, fronted.clone()),
        (ToileCommand::Undo, start),
        (ToileCommand::Redo, fronted),
        (ToileCommand::Redo, shifted),
        (ToileCommand::Redo, gone),
    ];
    for (step, (command, expected)) in walk.into_iter().enumerate() {
        assert!(
            st.handle_command(command).update.is_some(),
            "step {step} writes"
        );
        assert_eq!(board_of(&st), expected, "step {step}");
    }
}

/// One step of the property. An index picks a live element modulo the board,
/// and a delete takes one or two of them.
#[derive(Clone, Debug)]
enum Op {
    Create(u8),
    Delete(usize, usize),
    Reorder(usize, Target),
    Move(usize, i8),
    Undo,
    Redo,
}

#[derive(Clone, Copy, Debug)]
enum Target {
    Front,
    Back,
    Forward,
    Backward,
    Index(u32),
}

fn op() -> impl Strategy<Value = Op> {
    let target = prop_oneof![
        Just(Target::Front),
        Just(Target::Back),
        Just(Target::Forward),
        Just(Target::Backward),
        (0u32..6).prop_map(Target::Index),
    ];
    prop_oneof![
        3 => any::<u8>().prop_map(Op::Create),
        2 => (any::<usize>(), any::<usize>()).prop_map(|(i, j)| Op::Delete(i, j)),
        2 => (any::<usize>(), target).prop_map(|(i, t)| Op::Reorder(i, t)),
        2 => (any::<usize>(), any::<i8>()).prop_map(|(i, d)| Op::Move(i, d)),
        3 => Just(Op::Undo),
        2 => Just(Op::Redo),
    ]
}

/// The board kept by hand, and the two stacks a history step walks.
#[derive(Default)]
struct Oracle {
    board: Board,
    undo: Vec<Board>,
    redo: Vec<Board>,
}

impl Oracle {
    /// Record a forward step: the old board goes on the undo stack, and the
    /// redo stack empties, as a new edit after an undo does.
    fn edit(&mut self, next: Board) {
        let before = std::mem::replace(&mut self.board, next);
        self.undo.push(before);
        self.redo.clear();
    }

    /// The board the step leads to, by hand, or `None` when it changes
    /// nothing. A history step moves the stacks itself.
    fn step(&mut self, op: &Op, created: Option<Id<Element>>) -> Option<Board> {
        let len = self.board.len();
        let mut next = self.board.clone();
        match *op {
            Op::Create(x) => {
                let id = created.expect("a create names its element");
                next.push((id, rect_at(f64::from(x))));
            }
            Op::Delete(i, j) if len > 0 => {
                let (first, second) = (i % len, j % len);
                next.remove(first.max(second));
                if first != second {
                    next.remove(first.min(second));
                }
            }
            Op::Reorder(i, target) if len > 0 => {
                let from = i % len;
                let last = len - 1;
                let to = match target {
                    Target::Front => last,
                    Target::Back => 0,
                    Target::Forward => (from + 1).min(last),
                    Target::Backward => from.saturating_sub(1),
                    Target::Index(t) => (t as usize).min(last),
                };
                if to == from {
                    return None;
                }
                let node = next.remove(from);
                next.insert(to, node);
            }
            Op::Move(i, d) if len > 0 && d != 0 => {
                let entry = &mut next[i % len];
                entry.1 = entry.1 + Vec2::new(f64::from(d), 0.0);
            }
            Op::Undo => {
                let back = self.undo.pop()?;
                self.redo.push(std::mem::replace(&mut self.board, back));
                return Some(self.board.clone());
            }
            Op::Redo => {
                let forward = self.redo.pop()?;
                self.undo.push(std::mem::replace(&mut self.board, forward));
                return Some(self.board.clone());
            }
            _ => return None,
        }
        self.edit(next.clone());
        Some(next)
    }
}

/// The command the engine runs for one step, against the board it shows.
fn command(op: &Op, board: &Board) -> Option<ToileCommand> {
    let pick = |i: usize| board[i % board.len()].0.clone();
    Some(match *op {
        Op::Create(_) => return None,
        Op::Delete(i, j) if !board.is_empty() => {
            let mut ids = vec![pick(i), pick(j)];
            ids.dedup();
            ToileCommand::DeleteElements { ids }
        }
        Op::Reorder(i, t) if !board.is_empty() => ToileCommand::Reorder {
            node: NodeId::Element(pick(i)),
            to: match t {
                Target::Front => ZTarget::Front,
                Target::Back => ZTarget::Back,
                Target::Forward => ZTarget::Forward,
                Target::Backward => ZTarget::Backward,
                Target::Index(t) => ZTarget::Index(Pos::new(t)),
            },
        },
        Op::Move(i, d) if !board.is_empty() && d != 0 => ToileCommand::MoveElements {
            ids: vec![pick(i)],
            delta: Vec2::new(f64::from(d), 0.0),
        },
        Op::Undo => ToileCommand::Undo,
        Op::Redo => ToileCommand::Redo,
        _ => return None,
    })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// `TOILE-290`: any run of creates, deletes, reorders, moves, undos and
    /// redos leaves the board the oracle rebuilt by hand. A second peer takes
    /// every delta and shows the same board.
    #[test]
    fn undo_and_redo_follow_a_board_rebuilt_by_hand(ops in prop::collection::vec(op(), 1..32)) {
        let mut st = ToileState::new();
        let mut peer = ToileState::new();
        peer.apply_remote_snapshot(&st.export_snapshot());
        let mut oracle = Oracle::default();
        for (step, op) in ops.iter().enumerate() {
            let (created, out) = match op {
                Op::Create(x) => {
                    let (id, out) = create(&mut st, f64::from(*x));
                    (Some(id), Some(out))
                }
                _ => (None, command(op, &oracle.board).map(|c| st.handle_command(c))),
            };
            let expected = oracle.step(op, created);
            let wrote = out.as_ref().is_some_and(|o| o.update.is_some());
            prop_assert_eq!(wrote, expected.is_some(), "step {} {:?}: a write", step, op);
            if let Some(update) = out.and_then(|o| o.update) {
                peer.apply_remote(&update);
            }
            prop_assert_eq!(&board_of(&st), &oracle.board, "step {} {:?}", step, op);
            prop_assert_eq!(&board_of(&peer), &oracle.board, "step {} {:?}: peer", step, op);
        }
    }
}
