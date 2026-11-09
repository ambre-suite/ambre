//! `UNDO-013`: the undo of a format leaves a mark a peer set since. Two peers
//! on one text: one formats, the other formats over it, and the first undoes.

use ambre_core::HistoryStep;
use ambre_core::crdt::CommitOrigin;
use loro::{LoroDoc, LoroText, LoroValue};
use proptest::prelude::*;

use super::UndoHistory;

/// The text every case starts from.
const SEED: &str = "abcdefghijkl";

/// The mark keys a case writes.
const KEYS: [&str; 2] = ["w", "c"];

/// A document whose marks extend after their end, as Prose configures it.
fn doc() -> LoroDoc {
    let doc = LoroDoc::new();
    doc.config_default_text_style(Some(loro::StyleConfig {
        expand: loro::ExpandType::After,
    }));
    doc
}

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

/// The value of the mark `key` on the character at `at`, read alone.
fn value_at(text: &LoroText, at: usize, key: &str) -> Option<LoroValue> {
    let delta = text
        .slice_delta(at, at + 1, loro::cursor::PosType::Unicode)
        .expect("a delta");
    delta.iter().find_map(|item| match item {
        loro::TextDelta::Insert { attributes, .. } => attributes
            .as_ref()
            .and_then(|marks| marks.get(key))
            .filter(|value| !matches!(value, LoroValue::Null))
            .cloned(),
        _ => None,
    })
}

/// Every mark of every character, character by character.
fn marks(text: &LoroText) -> Vec<[Option<LoroValue>; 2]> {
    (0..text.len_unicode())
        .map(|at| KEYS.map(|key| value_at(text, at, key)))
        .collect()
}

/// Set `key` to `value` over `range`, or take it off with `None`.
fn format(text: &LoroText, range: std::ops::Range<usize>, key: &str, value: Option<i64>) {
    match value {
        Some(value) => text.mark(range, key, value).expect("a mark"),
        None => text.unmark(range, key).expect("an unmark"),
    }
}

/// The weight Alice sets and Bob changes, one character in five.
#[test]
fn an_undone_format_keeps_a_peer_s_format() {
    let alice = doc();
    let history = UndoHistory::watch_nested(&alice, &[], None);
    let text = alice.get_text("t");
    text.insert(0, "abcdef").expect("a seed");
    alice.commit();
    history.seal();
    format(&text, 0..6, "w", Some(700));
    alice.commit();
    let bob = doc();
    sync(&alice, &bob);
    let theirs = bob.get_text("t");
    format(&theirs, 2..4, "w", Some(900));
    format(&theirs, 0..6, "c", Some(1));
    bob.commit();
    sync(&bob, &alice);
    replay(&alice, &history, HistoryStep::Undo);
    let weights: Vec<_> = (0..6).map(|at| value_at(&text, at, "w")).collect();
    let bold = Some(LoroValue::I64(900));
    assert_eq!(
        weights,
        [None, None, bold.clone(), bold.clone(), None, None]
    );
    assert!(
        (0..6).all(|at| value_at(&text, at, "c") == Some(LoroValue::I64(1))),
        "the undo took off a key it never wrote"
    );
    replay(&alice, &history, HistoryStep::Redo);
    let weights: Vec<_> = (0..6).map(|at| value_at(&text, at, "w")).collect();
    let set = Some(LoroValue::I64(700));
    let expected = [
        set.clone(),
        set.clone(),
        bold.clone(),
        bold,
        set.clone(),
        set,
    ];
    assert_eq!(weights, expected);
}

/// One format: a key, a range of `SEED`, and a value, `None` to take it off.
#[derive(Debug, Clone)]
struct Format {
    key: usize,
    start: usize,
    len: usize,
    value: Option<i64>,
}

fn format_op() -> impl Strategy<Value = Format> {
    let size = SEED.len();
    (
        0..KEYS.len(),
        0..size,
        1..=size,
        prop::option::of(1_i64..=2),
    )
        .prop_map(move |(key, start, len, value)| Format {
            key,
            start,
            len: len.min(size - start),
            value,
        })
}

fn apply(text: &LoroText, op: &Format) {
    let key = KEYS.get(op.key).copied().unwrap_or("w");
    format(text, op.start..op.start + op.len, key, op.value);
}

/// What one character holds after a replay that writes `to` where the text
/// still holds `from`: the model, read character by character.
fn expected(
    now: &[[Option<LoroValue>; 2]],
    from: &[[Option<LoroValue>; 2]],
    to: &[[Option<LoroValue>; 2]],
    op: &Format,
) -> Vec<[Option<LoroValue>; 2]> {
    now.iter()
        .zip(from)
        .zip(to)
        .enumerate()
        .map(|(at, ((now, from), to))| {
            let mut out = now.clone();
            let inside = (op.start..op.start + op.len).contains(&at);
            if let (true, Some(held), Some(want), Some(value)) =
                (inside, now.get(op.key), from.get(op.key), to.get(op.key))
                && held == want
                && let Some(slot) = out.get_mut(op.key)
            {
                *slot = value.clone();
            }
            out
        })
        .collect()
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// `UNDO-013`: an undo, then a redo, of one format writes each character
    /// whose mark still holds what the step left, and no other. The model reads
    /// one character at a time; the history walks runs.
    #[test]
    fn a_format_undo_writes_only_where_no_peer_formatted_since(
        seed in prop::collection::vec(format_op(), 0..4),
        step in format_op(),
        peer in prop::collection::vec(format_op(), 0..5),
    ) {
        let alice = doc();
        let history = UndoHistory::watch_nested(&alice, &[], None);
        let text = alice.get_text("t");
        text.insert(0, SEED).expect("a seed");
        for op in &seed {
            apply(&text, op);
        }
        alice.commit();
        // The seed is no step: an undo of a format that wrote nothing must
        // find nothing to take back.
        history.clear();
        let before = marks(&text);
        history.seal();
        apply(&text, &step);
        alice.commit();
        let after = marks(&text);
        let bob = doc();
        sync(&alice, &bob);
        let theirs = bob.get_text("t");
        for op in &peer {
            apply(&theirs, op);
        }
        bob.commit();
        sync(&bob, &alice);
        let merged = marks(&text);
        replay(&alice, &history, HistoryStep::Undo);
        let undone = marks(&text);
        prop_assert_eq!(&undone, &expected(&merged, &after, &before, &step));
        replay(&alice, &history, HistoryStep::Redo);
        prop_assert_eq!(marks(&text), expected(&undone, &before, &after, &step));
    }
}
