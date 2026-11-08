// ambre-trame/src/logic/growth_tests.rs
// ============================================================================
// A local write that adds lines, against a full reseed (`TRAME-289`), the
// reference-model regime of `ambre-spec-testing.md` section 3.
//
// A formula that reads a position past the used range compiles that position
// to a virtual axis. A local write past the used range adds the lines. A full
// reseed then binds the position to the line now at it. The carried engine
// must hold the same binding at once, so the next edit that trusts it reads
// the line, and not a stale position.
//
// Two states run one program in lockstep, under one peer id, so both mint the
// same uids. The second reseeds its engine after every step. After each step
// the two must agree on the stored text, each edge, each range, each value.
//
// The generator writes formulas into columns B to D alone, and those formulas
// read columns A, E, F and G alone. So no program closes a cycle. Which cell of
// a cycle a local write refuses is outside this story.
// ============================================================================

use ambre_core::index::{Len, Pos};
use proptest::prelude::*;

use super::TrameState;
use super::structural::StructuralPath;
use super::structural_tests::{engine, same_values, seeded, stored, write};
use crate::message::{ClipboardCell, ClipboardPayload};
use crate::{Col, Row, TrameCommand};

/// The window the generator addresses. The seed uses four rows and two
/// columns, so most writes add lines.
const ROWS: u32 = 14;
const COLS: u32 = 7;

/// One step of a program: each one writes, and may add lines.
#[derive(Debug, Clone)]
enum Step {
    Write {
        row: u32,
        col: u32,
        text: String,
    },
    /// A paste of a block of numbers, through the batch path.
    Paste {
        row: u32,
        col: u32,
    },
}

/// A formula of columns B to D. Each one reads a position the seed leaves
/// past the used range, a range across it, or the other sheet.
fn formula() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("=A9+1".to_owned()),
        Just("=A12*2".to_owned()),
        Just("=E3+A1".to_owned()),
        Just("=F14".to_owned()),
        Just("=G2+A10".to_owned()),
        Just("=SUM(A1:A12)".to_owned()),
        Just("=SUM(E1:F6)".to_owned()),
        Just("=ROW()*10+COLUMN()".to_owned()),
        Just("=Sheet2!A1+1".to_owned()),
    ]
}

/// A number, or an empty text that clears the cell.
fn plain() -> impl Strategy<Value = String> {
    prop_oneof![
        3 => (0i64..9).prop_map(|n| n.to_string()),
        1 => Just(String::new()),
    ]
}

fn step() -> impl Strategy<Value = Step> {
    prop_oneof![
        4 => (0..ROWS, 1..4u32, prop_oneof![formula(), plain()])
            .prop_map(|(row, col, text)| Step::Write { row, col, text }),
        3 => (0..ROWS, prop_oneof![Just(0u32), 4..COLS], plain())
            .prop_map(|(row, col, text)| Step::Write { row, col, text }),
        1 => (0..ROWS, prop_oneof![Just(0u32), 4..COLS - 1])
            .prop_map(|(row, col)| Step::Paste { row, col }),
    ]
}

/// A paste of a two-by-two block of numbers at `row`, `col`.
fn paste(row: u32, col: u32) -> TrameCommand {
    let cells = (0..2)
        .flat_map(|dr| (0..2).map(move |dc| (dr, dc)))
        .map(|(dr, dc)| ClipboardCell {
            dr: Len::<Row>::new(dr),
            dc: Len::<Col>::new(dc),
            raw: format!("{}", dr * 2 + dc + 1),
            ..ClipboardCell::default()
        })
        .collect();
    TrameCommand::PasteRange {
        target_row: Pos::<Row>::new(row),
        target_col: Pos::<Col>::new(col),
        payload: ClipboardPayload {
            src_row: Pos::<Row>::new(0),
            src_col: Pos::<Col>::new(0),
            rows: Len::<Row>::new(2),
            cols: Len::<Col>::new(2),
            cells,
        },
    }
}

fn command(step: &Step) -> TrameCommand {
    match step {
        Step::Write { row, col, text } => write(*row, *col, text),
        Step::Paste { row, col } => paste(*row, *col),
    }
}

/// Run `command` on the carried state, and on the reference model, which
/// reseeds after it. Answer what differs, or `Ok` when nothing does.
fn run_both(
    carried: &mut TrameState,
    rebuilt: &mut TrameState,
    command: &TrameCommand,
) -> Result<(), String> {
    carried.handle_command(command.clone());
    rebuilt.handle_command(command.clone());
    rebuilt.reseed_engine();
    if stored(carried) != stored(rebuilt) {
        return Err("the stored text differs".to_owned());
    }
    let (a, b) = (engine(carried), engine(rebuilt));
    if a != b {
        return Err(format!("the engine differs: carried {a:?}, rebuilt {b:?}"));
    }
    same_values(carried, rebuilt)
}

/// The shrunk case: a formula reads a row past the used range, and a write
/// adds that row. The formula reads the row by its line at once, as a reseed
/// gives it.
#[test]
fn a_write_that_adds_a_row_binds_a_formula_that_reads_it() {
    let mut carried = seeded(StructuralPath::Carry);
    let mut rebuilt = seeded(StructuralPath::Carry);
    for command in [write(0, 2, "=A12*2"), write(11, 0, "5")] {
        let differs = run_both(&mut carried, &mut rebuilt, &command);
        assert!(differs.is_ok(), "after {command:?}: {differs:?}");
    }
}

/// A formula on the other sheet reads a row past the used range of the first,
/// and a write on the first adds that row.
#[test]
fn a_write_that_adds_a_row_binds_a_formula_of_another_sheet() {
    let mut carried = seeded(StructuralPath::Carry);
    let mut rebuilt = seeded(StructuralPath::Carry);
    let command = write(8, 1, "3");
    let differs = run_both(&mut carried, &mut rebuilt, &command);
    assert!(differs.is_ok(), "after {command:?}: {differs:?}");
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 256, ..ProptestConfig::default() })]

    /// After each local write, each formula holds the binding, the edges, the
    /// ranges and the value that a full reseed gives it (`TRAME-289`).
    #[test]
    fn a_local_write_that_adds_lines_equals_a_full_reseed(
        program in proptest::collection::vec(step(), 1..16)
    ) {
        let mut carried = seeded(StructuralPath::Carry);
        let mut rebuilt = seeded(StructuralPath::Carry);
        rebuilt.reseed_engine();
        prop_assert_eq!(engine(&carried), engine(&rebuilt), "the seed differs");
        for step in &program {
            let differs = run_both(&mut carried, &mut rebuilt, &command(step));
            prop_assert!(differs.is_ok(), "after {:?}: {:?}", step, differs);
        }
    }
}
