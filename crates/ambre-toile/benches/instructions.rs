//! Operations whose budget is held by a count of instructions, not by a time.
//!
//! A time measured on a shared CI runner moves by tens of percent between two
//! runs, so a threshold on it fails on noise. An instruction count under
//! `callgrind` is stable, so `scripts/instructions.py` fails the build when an
//! operation outgrows its ceiling (`ambre-spec-testing.md section 2`).
//!
//! Each operation runs once, inside a function that is never inlined and whose
//! name is unique. The script names the operation on the command line and tells
//! `callgrind` to count only inside that function, so the setup costs nothing.
//! Run with no operation named, as `cargo bench` does, every operation runs
//! once: a smoke test.

#![allow(
    clippy::expect_used,
    reason = "a bench aborts on a fixture it cannot run"
)]

use std::hint::black_box;

use ambre_toile::{
    ElementId, ElementKind, InitContent, ToileCommand, ToileEvent, ToileOutcome, ToileState,
};
use kurbo::{Point, Size};

/// Characters in the body the "Toile body commit" row names: 4 KB of ASCII.
const BODY_LEN: usize = 4096;

/// A body of `BODY_LEN` characters, words of five letters and a space.
fn body() -> String {
    (0..BODY_LEN)
        .map(|i| {
            if i % 6 == 5 {
                ' '
            } else {
                char::from(b'a' + (i % 26) as u8)
            }
        })
        .collect()
}

/// `text` with the characters `range` replaced by `with`.
fn replace(text: &str, range: std::ops::Range<usize>, with: &str) -> String {
    let mut out: Vec<char> = text.chars().collect();
    out.splice(range, with.chars());
    out.into_iter().collect()
}

/// `text` with every `stride`-th character of `range` replaced by `with`.
fn scatter(text: &str, range: std::ops::Range<usize>, stride: usize, with: char) -> String {
    text.chars()
        .enumerate()
        .map(|(i, c)| {
            if range.contains(&i) && i % stride == 0 {
                with
            } else {
                c
            }
        })
        .collect()
}

/// A board holding one sticky whose body is `base`, then a peer's edit of it.
fn board(base: &str, stored: &str) -> (ToileState, ElementId) {
    let mut st = ToileState::new();
    let out = st.handle_command(ToileCommand::CreateElement {
        kind: ElementKind::Sticky,
        at: Point::new(0.0, 0.0),
        size: Size::new(400.0, 400.0),
        content: InitContent::Text(base.to_owned()),
        fill: None,
        correlate: 0,
    });
    let id = out
        .display
        .iter()
        .find_map(|e| match e {
            ToileEvent::ElementCreated { id, .. } => Some(id.clone()),
            _ => None,
        })
        .expect("a create names its element");
    st.handle_command(ToileCommand::SetText {
        id: id.clone(),
        base: base.to_owned(),
        text: stored.to_owned(),
    });
    (st, id)
}

/// Close an editor on `typed`, opened on `base`, over a board a peer moved to
/// `stored` meanwhile.
fn close(base: String, stored: &str, typed: String, measured: fn(&mut ToileState, ToileCommand)) {
    let (mut st, id) = board(&base, stored);
    measured(
        &mut st,
        black_box(ToileCommand::SetText {
            id,
            base,
            text: typed,
        }),
    );
}

/// One `SetText`, derivation and delta included. Counted alone by `callgrind`.
#[inline(never)]
fn ambre_budget_toile_body_commit(st: &mut ToileState, command: ToileCommand) {
    let out: ToileOutcome = st.handle_command(command);
    assert!(out.update.is_some(), "the close writes the merged body");
}

/// The same close, counted under another name for the bound of the cap.
#[inline(never)]
fn ambre_bound_toile_body_commit(st: &mut ToileState, command: ToileCommand) {
    let out: ToileOutcome = st.handle_command(command);
    assert!(out.update.is_some(), "the close writes the merged body");
}

/// The close the budget row names. While the editor was open, a peer retyped
/// a word and added a sentence in the first quarter. The author fixed a word
/// in the second half and added a sentence at the end.
fn body_commit() {
    let base = body();
    let stored = replace(&base, 600..605, "PEERS");
    let stored = replace(&stored, 900..900, "A sentence a peer added. ");
    let typed = replace(&base, 2400..2405, "FIXED");
    let typed = format!("{typed} And a sentence the author typed before leaving.");
    close(base, &stored, typed, ambre_budget_toile_body_commit);
}

/// The worst close the cap lets through: every 13th character of each half
/// changes, one half per side. Each change is two edit steps, so each diff
/// runs to its 256-step cap before it gives up (`TOILE-UI-381`).
fn body_commit_at_cap() {
    let base = body();
    let stored = scatter(&base, 0..BODY_LEN / 2, 13, '#');
    let typed = scatter(&base, BODY_LEN / 2..BODY_LEN, 13, '@');
    close(base, &stored, typed, ambre_bound_toile_body_commit);
}

/// Each operation, by the name `scripts/instructions.py` passes.
const OPERATIONS: [(&str, fn()); 2] = [
    ("body_commit", body_commit),
    ("body_commit_at_cap", body_commit_at_cap),
];

fn main() {
    // `cargo bench` passes flags such as `--bench`; only a bare word names an
    // operation.
    let named: Vec<String> = std::env::args()
        .skip(1)
        .filter(|a| !a.starts_with('-'))
        .collect();
    for (name, run) in OPERATIONS {
        if named.is_empty() || named.iter().any(|n| n == name) {
            run();
        }
    }
}
