//! `UNDO-024`: a step reaches the characters that a replay of this peer put
//! back under new identities. `UNDO-025`: an undo of a deletion puts its
//! characters back at their place when a neighbour is gone.

use ambre_core::HistoryStep;
use ambre_core::crdt::CommitOrigin;
use loro::{LoroDoc, LoroText};
use proptest::prelude::*;

use super::UndoHistory;

/// Sync `from` into `to`: every update `to` lacks.
fn sync(from: &LoroDoc, to: &LoroDoc) {
    to.import(
        &from
            .export(loro::ExportMode::all_updates())
            .expect("an export"),
    )
    .expect("an import");
}

/// Replay one step and commit it under the replay origin.
fn replay(doc: &LoroDoc, history: &UndoHistory, step: HistoryStep) {
    let _ = history.write(doc, step);
    doc.set_next_commit_origin(CommitOrigin::Recorded.as_str());
    doc.commit();
}

/// A document that loaded `texts` from another peer, and the history that
/// watches it. No character of the seed is its own.
fn seeded(texts: &[&str]) -> (LoroDoc, UndoHistory) {
    let seed = LoroDoc::new();
    for (index, text) in texts.iter().enumerate() {
        seed.get_text(format!("t{index}"))
            .insert(0, text)
            .expect("a seed");
    }
    seed.commit();
    let doc = LoroDoc::new();
    let history = UndoHistory::watch_nested(&doc, &[], None);
    sync(&seed, &doc);
    (doc, history)
}

/// One user edit: a sealed step, then a commit.
fn edit(doc: &LoroDoc, history: &UndoHistory, write: impl FnOnce()) {
    history.seal();
    write();
    doc.commit();
}

/// `UNDO-024`: delete a word, type a letter over it, delete the letter, then
/// undo three times. The second undo takes out the letter that the first
/// undo put back under a new identity.
#[test]
fn an_undo_of_typing_reaches_a_letter_its_own_undo_put_back() {
    let (doc, history) = seeded(&["hello world"]);
    let text = doc.get_text("t0");
    edit(&doc, &history, || text.delete(6, 5).expect("the word goes"));
    edit(&doc, &history, || text.insert(6, "x").expect("a letter"));
    edit(&doc, &history, || {
        text.delete(6, 1).expect("the letter goes")
    });
    let mut seen = Vec::new();
    for _ in 0..3 {
        replay(&doc, &history, HistoryStep::Undo);
        seen.push(text.to_string());
    }
    assert_eq!(seen, ["hello x", "hello ", "hello world"]);
    for _ in 0..3 {
        replay(&doc, &history, HistoryStep::Redo);
    }
    assert_eq!(text.to_string(), "hello ");
}

/// `UNDO-024`: a delete across two texts joins them, as Prose joins two
/// blocks. A split at the join then moves the joined tail into a third text.
/// Two undos give back both texts as they stood.
#[test]
fn an_undo_across_a_split_at_a_join_restores_both_texts() {
    let (doc, history) = seeded(&["hello world", "second"]);
    let (first, second, third) = (doc.get_text("t0"), doc.get_text("t1"), doc.get_text("t2"));
    edit(&doc, &history, || {
        first.delete(6, 5).expect("the tail of the first goes");
        first
            .insert(6, "ond")
            .expect("the tail of the second joins");
        second.delete(0, 6).expect("the second empties");
    });
    assert_eq!(first.to_string(), "hello ond");
    edit(&doc, &history, || {
        first.delete(6, 3).expect("the split takes the tail");
        third.insert(0, "ond").expect("into a new text");
    });
    replay(&doc, &history, HistoryStep::Undo);
    assert_eq!(first.to_string(), "hello ond");
    assert_eq!(third.to_string(), "");
    replay(&doc, &history, HistoryStep::Undo);
    assert_eq!(first.to_string(), "hello world");
    assert_eq!(second.to_string(), "second");
}

/// `UNDO-024`: the join and the split land in one step, as a paste of
/// several lines over two blocks does. One undo gives back both texts: the
/// undo of the join reaches the tail the undo of the split put back.
#[test]
fn one_undo_of_a_join_and_a_split_restores_both_texts() {
    let (doc, history) = seeded(&["hello world", "second"]);
    let (first, second, third) = (doc.get_text("t0"), doc.get_text("t1"), doc.get_text("t2"));
    history.seal();
    first.delete(6, 5).expect("the tail of the first goes");
    first
        .insert(6, "ond")
        .expect("the tail of the second joins");
    second.delete(0, 6).expect("the second empties");
    doc.commit();
    first.insert(6, "one").expect("the first line");
    first.delete(9, 3).expect("the split takes the tail");
    third.insert(0, "twoond").expect("into a new text");
    doc.commit();
    assert_eq!(first.to_string(), "hello one");
    replay(&doc, &history, HistoryStep::Undo);
    assert_eq!(first.to_string(), "hello world");
    assert_eq!(second.to_string(), "second");
    assert_eq!(third.to_string(), "");
    replay(&doc, &history, HistoryStep::Redo);
    assert_eq!(first.to_string(), "hello one");
    assert_eq!(third.to_string(), "twoond");
}

/// `UNDO-025`: a peer takes out the character that stood right after a
/// deletion. The undo puts the characters back right after the one that
/// stood before them, and not before it.
#[test]
fn an_undone_deletion_goes_back_after_its_left_neighbour() {
    let (doc, history) = seeded(&["hello world!"]);
    let text = doc.get_text("t0");
    edit(&doc, &history, || text.delete(6, 5).expect("the word goes"));
    let peer = LoroDoc::new();
    sync(&doc, &peer);
    peer.get_text("t0")
        .delete(6, 1)
        .expect("a peer takes `!` out");
    peer.commit();
    sync(&peer, &doc);
    assert_eq!(text.to_string(), "hello ");
    replay(&doc, &history, HistoryStep::Undo);
    assert_eq!(text.to_string(), "hello world");
}

/// One edit of the property. Each index is taken modulo what the text holds.
#[derive(Clone, Debug)]
enum Op {
    /// Type `text` into a text at a place.
    Type(usize, usize, String),
    /// Take a run of characters out of a text.
    Delete(usize, usize, usize),
    /// Move the tail of one text to the start of another, in one commit: a
    /// split or a join of two blocks.
    Move(usize, usize, usize),
    Undo,
    Redo,
}

/// How many texts a case edits.
const TEXTS: usize = 3;

fn op() -> impl Strategy<Value = Op> {
    prop_oneof![
        3 => (any::<usize>(), any::<usize>(), "[a-z]{1,3}").prop_map(|(t, at, s)| Op::Type(t, at, s)),
        3 => (any::<usize>(), any::<usize>(), 1..4_usize).prop_map(|(t, at, n)| Op::Delete(t, at, n)),
        2 => (any::<usize>(), any::<usize>(), any::<usize>()).prop_map(|(a, b, at)| Op::Move(a, b, at)),
        3 => Just(Op::Undo),
        2 => Just(Op::Redo),
    ]
}

fn texts(doc: &LoroDoc) -> Vec<LoroText> {
    (0..TEXTS).map(|n| doc.get_text(format!("t{n}"))).collect()
}

fn read(texts: &[LoroText]) -> Vec<String> {
    texts.iter().map(LoroText::to_string).collect()
}

/// Run one edit. Returns whether it wrote, so the oracle records a step.
fn run(doc: &LoroDoc, history: &UndoHistory, texts: &[LoroText], op: &Op) -> bool {
    let pick = |t: usize| &texts[t % TEXTS];
    match op {
        Op::Type(t, at, s) => {
            let text = pick(*t);
            let at = at % (text.len_unicode() + 1);
            edit(doc, history, || text.insert(at, s).expect("typing"));
            true
        }
        Op::Delete(t, at, n) => {
            let text = pick(*t);
            let len = text.len_unicode();
            if len == 0 {
                return false;
            }
            let at = at % len;
            let n = (*n).min(len - at);
            edit(doc, history, || text.delete(at, n).expect("a deletion"));
            true
        }
        Op::Move(a, b, at) => {
            let (from, to) = (pick(*a), pick(*b));
            let len = from.len_unicode();
            if a % TEXTS == b % TEXTS || len == 0 {
                return false;
            }
            let at = at % len;
            let tail = from.slice(at, len).expect("the tail");
            edit(doc, history, || {
                from.delete(at, len - at).expect("the tail goes");
                to.insert(0, &tail).expect("the tail lands");
            });
            true
        }
        Op::Undo | Op::Redo => false,
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    /// `UNDO-024`: on one peer, take any run of typing, deletions, moves
    /// between texts, undos and redos. It walks through the exact texts that
    /// each edit left. The oracle is the stack of those texts, kept by hand.
    #[test]
    fn undo_and_redo_walk_the_texts_each_edit_left(ops in prop::collection::vec(op(), 1..40)) {
        let (doc, history) = seeded(&["hello world", "second", ""]);
        let texts = texts(&doc);
        let mut now = read(&texts);
        let mut past: Vec<Vec<String>> = Vec::new();
        let mut future: Vec<Vec<String>> = Vec::new();
        for (step, op) in ops.iter().enumerate() {
            match op {
                Op::Undo | Op::Redo => {
                    let undo = matches!(op, Op::Undo);
                    let (from, to) = if undo {
                        (&mut past, &mut future)
                    } else {
                        (&mut future, &mut past)
                    };
                    let Some(page) = from.pop() else { continue };
                    to.push(std::mem::replace(&mut now, page));
                    let direction = if undo { HistoryStep::Undo } else { HistoryStep::Redo };
                    replay(&doc, &history, direction);
                }
                _ => {
                    if run(&doc, &history, &texts, op) {
                        past.push(std::mem::replace(&mut now, read(&texts)));
                        future.clear();
                    }
                }
            }
            prop_assert_eq!(&read(&texts), &now, "step {} {:?}", step, op);
        }
    }
}
