// Delegation regression for the `CellEditor` wrapper over the shared
// `ambre_canvas::TextEditor` core (axis 1). Each wrapper method is a shim that
// queues one `EditIntent`. This replays an op corpus through the wrapper AND
// through the core driven with the intent directly. It flushes both, and asserts
// the buffer text + selected text match after EVERY step. A wrong shim->intent
// mapping (for example Home wired to the wrong intent) is therefore caught.
//
// The corpus is single-line ASCII: that is what a spreadsheet cell holds today,
// and where the wrapper's Home/End (visual line) coincide with whole-buffer. The
// original hand-rolled-vs-core parity was proven in the commit that introduced
// this file, before the migration; this is its living continuation.

use super::CellEditor;
use crate::{CellPos, Col, Row};

use ambre_canvas::{EditIntent, TextEditor, TextEditorStyle};
use ambre_draw::{Color, FontContext};

/// One keyboard editing action, expressed once and replayed on both editors.
#[derive(Clone, Debug)]
enum Op {
    Ins(&'static str),
    Backspace,
    Delete,
    Left,
    Right,
    ShiftLeft,
    ShiftRight,
    Home,
    End,
    ShiftHome,
    ShiftEnd,
    SelectAll,
}

fn style() -> TextEditorStyle {
    TextEditorStyle {
        weight: ambre_core::FontWeight::REGULAR,
        italic: false,
        family: "Sans".to_owned(),
        base_size: ambre_core::DocPx::new(16.0),
        color: Color::from_rgba8(0, 0, 0, 255),
        underline: false,
        strikethrough: false,
    }
}

/// Apply an op to the hand-rolled `CellEditor`.
fn apply_cell(ed: &mut CellEditor, op: &Op) {
    match op {
        Op::Ins(s) => ed.insert(s),
        Op::Backspace => ed.backspace(),
        Op::Delete => {
            ed.delete_forward();
        }
        Op::Left => ed.move_left(),
        Op::Right => ed.move_right(),
        Op::ShiftLeft => ed.extend_left(),
        Op::ShiftRight => ed.extend_right(),
        Op::Home => ed.move_home(),
        Op::End => ed.move_end(),
        Op::ShiftHome => ed.extend_home(),
        Op::ShiftEnd => ed.extend_end(),
        Op::SelectAll => ed.select_all(),
    }
}

/// The equivalent shared-core intent for an op.
fn intent(op: &Op) -> EditIntent {
    match op {
        Op::Ins(s) => EditIntent::Insert((*s).to_owned()),
        Op::Backspace => EditIntent::Backspace,
        Op::Delete => EditIntent::Delete,
        Op::Left => EditIntent::Left {
            select: false,
            word: false,
        },
        Op::Right => EditIntent::Right {
            select: false,
            word: false,
        },
        Op::ShiftLeft => EditIntent::Left {
            select: true,
            word: false,
        },
        Op::ShiftRight => EditIntent::Right {
            select: true,
            word: false,
        },
        Op::Home => EditIntent::LineStart { select: false },
        Op::End => EditIntent::LineEnd { select: false },
        Op::ShiftHome => EditIntent::LineStart { select: true },
        Op::ShiftEnd => EditIntent::LineEnd { select: true },
        Op::SelectAll => EditIntent::SelectAll,
    }
}

/// Build the wrapper and a directly-driven core on `seed`, replay `ops`, flush
/// both each step, and assert the buffer text and selected text match.
fn assert_parity(seed: &str, ops: &[Op]) {
    let mut fcx = FontContext::default();
    let mut cell = CellEditor::new(
        CellPos::new(Row::ZERO, Col::ZERO),
        crate::EditSessionId::FIRST,
        seed.to_owned(),
    );
    let mut core = TextEditor::new(seed, style());
    // First flush seeds both carets at text end.
    cell.core
        .flush(&mut fcx, 1000.0, ambre_core::ZoomFactor::ONE);
    core.flush(&mut fcx, 1000.0, ambre_core::ZoomFactor::ONE);

    for (i, op) in ops.iter().enumerate() {
        apply_cell(&mut cell, op);
        core.push(intent(op));
        cell.core
            .flush(&mut fcx, 1000.0, ambre_core::ZoomFactor::ONE);
        core.flush(&mut fcx, 1000.0, ambre_core::ZoomFactor::ONE);

        assert_eq!(
            cell.text(),
            core.text(),
            "buffer diverged after step {i} ({op:?})"
        );
        assert_eq!(
            cell.selected_text().as_str(),
            core.selected_text().as_deref().unwrap_or(""),
            "selection diverged after step {i} ({op:?})"
        );
    }
}

#[test]
fn parity_caret_motion_and_insert() {
    assert_parity(
        "hello world",
        &[
            Op::End,
            Op::Left,
            Op::Left,
            Op::Ins("Z"),
            Op::Home,
            Op::Right,
            Op::Ins("_"),
        ],
    );
}

#[test]
fn parity_shift_selection() {
    assert_parity(
        "hello world",
        &[
            Op::Home,
            Op::ShiftRight,
            Op::ShiftRight,
            Op::ShiftRight,
            Op::End,
            Op::ShiftHome,
            Op::End,
            Op::ShiftLeft,
            Op::ShiftLeft,
        ],
    );
}

#[test]
fn parity_select_replace_and_delete() {
    assert_parity(
        "hello world",
        &[
            Op::SelectAll,
            Op::Ins("new"),
            Op::Home,
            Op::ShiftEnd,
            Op::Delete,
            Op::Ins("x"),
        ],
    );
}

#[test]
fn parity_backspace_and_delete_forward() {
    assert_parity(
        "abcdef",
        &[
            Op::End,
            Op::Backspace,
            Op::Backspace,
            Op::Home,
            Op::Delete,
            Op::Delete,
        ],
    );
}

#[test]
fn parity_selection_collapse_on_arrows() {
    // A selection then a bare arrow collapses to the correct edge on both.
    assert_parity(
        "spreadsheet",
        &[
            Op::Home,
            Op::ShiftRight,
            Op::ShiftRight,
            Op::ShiftRight,
            Op::Left, // collapse to left edge
            Op::Ins("|"),
            Op::End,
            Op::ShiftHome,
            Op::Right, // collapse to right edge
            Op::Ins("#"),
        ],
    );
}
