// ambre-trame/src/logic/tests/group9.rs
// Edit sessions held by identity (`TRAME-UI-160`, `TRAME-UI-162`).
//
// A commit lands on the cell the session opened on, wherever a peer moved it.
// Once a peer deleted it, it lands nowhere (`TRAME-UI-176`, `TRAME-075`).

use super::*;
use crate::test_support::session;
use ambre_core::index::Pos;

const COL_1: Pos<Col> = Pos::<Col>::new(1);
const ROW_4: Pos<Row> = Pos::<Row>::new(4);

/// Every non-empty cell of the active sheet in rows and columns `0..8`, as
/// `(row, col, raw)`.
fn cells(state: &mut TrameState) -> Vec<(u32, u32, String)> {
    let out = state.handle_command(TrameCommand::ScrollViewport {
        version: ambre_core::Seq::nth(9),
        row_start: Pos::<Row>::new(0),
        row_end: Pos::<Row>::new(7),
        col_start: Pos::<Col>::new(0),
        col_end: Pos::<Col>::new(7),
    });
    let Some(TrameEvent::ViewportData { cells, .. }) = out.display.into_iter().next() else {
        panic!("expected ViewportData");
    };
    let mut cells: Vec<_> = cells
        .into_iter()
        .filter(|c| !c.raw.is_empty())
        .map(|c| (c.row.get(), c.col.get(), c.raw))
        .collect();
    cells.sort();
    cells
}

/// Two peers on one workbook whose column B holds `b1`..`b4`.
fn two_peers() -> (TrameState, TrameState) {
    let mut base = TrameState::new();
    for row in 0..4 {
        base.handle_command(TrameCommand::SetCellText {
            row: Pos::<Row>::new(row),
            col: Pos::<Col>::new(1),
            text: format!("b{}", row + 1),
        });
    }
    let snapshot = base.export_snapshot();
    let mut a = TrameState::new();
    a.import_snapshot(&snapshot)
        .expect("a fresh snapshot reads");
    let mut b = TrameState::new();
    b.import_snapshot(&snapshot)
        .expect("a fresh snapshot reads");
    (a, b)
}

/// Run `cmd` on `from` and merge its delta into `to`, returning `to`'s events.
fn relay(from: &mut TrameState, to: &mut TrameState, cmd: TrameCommand) -> Vec<TrameEvent> {
    let update = from
        .handle_command(cmd)
        .update
        .expect("the command wrote a delta");
    to.apply_remote(&update)
}

fn commit(
    state: &mut TrameState,
    session: ambre_bus::Token<crate::EditSession>,
    text: &str,
) -> TrameOutcome {
    let out = state.handle_command(TrameCommand::CommitCellEdit {
        session,
        text: text.to_owned(),
    });
    state.handle_command(TrameCommand::CommitEdit);
    out
}

#[test]
fn a_commit_lands_on_its_cell_after_a_peer_inserts_a_row_above() {
    let (mut a, mut b) = two_peers();
    a.handle_command(TrameCommand::BeginCellEdit {
        session: session(1),
        row: Pos::<Row>::new(3),
        col: Pos::<Col>::new(1),
    });

    let events = relay(
        &mut b,
        &mut a,
        TrameCommand::InsertRow {
            at: Pos::<Row>::new(1),
        },
    );
    assert!(
        events.iter().any(|e| matches!(
            e,
            TrameEvent::EditTargetMoved { session: s, row: ROW_4, col: COL_1 }
                if *s == session(1)
        )),
        "the editor is told its cell moved down one row: {events:?}"
    );
    let before = cells(&mut a);
    commit(&mut a, session(1), "typed");

    // `b4` now sits on row 5 and takes the text; `b3`, which the insert moved
    // onto the position the session opened on, keeps its value.
    let mut expected: Vec<_> = before
        .into_iter()
        .map(|(r, c, v)| (r, c, if (r, c) == (4, 1) { "typed".into() } else { v }))
        .collect();
    expected.sort();
    assert_eq!(
        cells(&mut a),
        expected,
        "the commit wrote one cell, its own"
    );
}

#[test]
fn a_commit_past_the_used_range_follows_an_insert_above() {
    let (mut a, mut b) = two_peers();
    a.handle_command(TrameCommand::BeginCellEdit {
        session: session(2),
        row: Pos::<Row>::new(6),
        col: Pos::<Col>::new(3),
    });

    relay(
        &mut b,
        &mut a,
        TrameCommand::InsertRow {
            at: Pos::<Row>::new(0),
        },
    );
    relay(
        &mut b,
        &mut a,
        TrameCommand::InsertColumn {
            at: Pos::<Col>::new(0),
        },
    );
    commit(&mut a, session(2), "far");

    assert!(
        cells(&mut a).contains(&(7, 4, "far".into())),
        "a cell no line held yet moves with the lines above and left of it"
    );
}

#[test]
fn a_peer_delete_of_the_row_or_column_closes_the_session_with_no_write() {
    for delete in [
        TrameCommand::DeleteRow {
            at: Pos::<Row>::new(2),
        },
        TrameCommand::DeleteColumn {
            at: Pos::<Col>::new(1),
        },
    ] {
        let (mut a, mut b) = two_peers();
        a.handle_command(TrameCommand::BeginCellEdit {
            session: session(3),
            row: Pos::<Row>::new(2),
            col: Pos::<Col>::new(1),
        });

        let events = relay(&mut b, &mut a, delete);
        assert!(
            events.iter().any(
                |e| matches!(e, TrameEvent::EditTargetGone { session: s } if *s == session(3))
            ),
            "the session is told its cell is gone: {events:?}"
        );
        let before = cells(&mut a);
        let out = commit(&mut a, session(3), "lost");
        assert!(out.update.is_none(), "the commit wrote nothing");
        assert_eq!(cells(&mut a), before, "no cell took the text");
    }
}

#[test]
fn a_commit_after_a_tab_switch_lands_on_the_session_sheet() {
    let (mut a, _) = two_peers();
    a.handle_command(TrameCommand::CreateSheet {
        name: crate::test_support::sheet("Two"),
    });
    let two = second_sheet_id(&a);
    a.handle_command(TrameCommand::BeginCellEdit {
        session: session(6),
        row: Pos::<Row>::new(0),
        col: Pos::<Col>::new(1),
    });
    a.handle_command(TrameCommand::ActivateSheet {
        sheet_id: two.clone(),
    });

    let out = commit(&mut a, session(6), "kept");
    assert!(
        !out.display
            .iter()
            .any(|e| matches!(e, TrameEvent::EditTargetGone { .. })),
        "a tab switch is not a delete"
    );
    assert!(cells(&mut a).is_empty(), "the tab being read took nothing");
    a.handle_command(TrameCommand::ActivateSheet {
        sheet_id: default_sheet(),
    });
    assert!(
        cells(&mut a).contains(&(0, 1, "kept".into())),
        "the text landed on the sheet the session opened on"
    );
}

#[test]
fn an_ended_session_writes_nothing() {
    let (mut a, _) = two_peers();
    a.handle_command(TrameCommand::BeginCellEdit {
        session: session(4),
        row: Pos::<Row>::new(0),
        col: Pos::<Col>::new(0),
    });
    a.handle_command(TrameCommand::EndCellEdit {
        session: session(4),
    });

    let out = commit(&mut a, session(4), "late");
    assert!(out.update.is_none(), "a closed session holds no cell");
}

#[test]
fn a_peer_delete_of_the_sheet_being_read_leaves_the_reader_on_another() {
    let (mut a, mut b) = two_peers();
    relay(
        &mut b,
        &mut a,
        TrameCommand::CreateSheet {
            name: crate::test_support::sheet("Two"),
        },
    );
    let two = second_sheet_id(&a);
    a.handle_command(TrameCommand::ActivateSheet {
        sheet_id: two.clone(),
    });
    a.handle_command(TrameCommand::BeginCellEdit {
        session: session(5),
        row: Pos::<Row>::new(0),
        col: Pos::<Col>::new(0),
    });

    let events = relay(
        &mut b,
        &mut a,
        TrameCommand::DeleteSheet {
            sheet_id: two.clone(),
        },
    );
    let active = events
        .iter()
        .find_map(|e| match e {
            TrameEvent::SheetList { active, .. } => Some(active.clone()),
            _ => None,
        })
        .expect("the merge reports the sheet list");
    assert_eq!(
        active,
        default_sheet(),
        "the reader lands on the tab before it"
    );
    assert!(
        events
            .iter()
            .any(|e| matches!(e, TrameEvent::EditTargetGone { session: s } if *s == session(5))),
        "the open session on the deleted sheet closes: {events:?}"
    );
    let out = commit(&mut a, session(5), "ghost");
    assert!(out.update.is_none(), "nothing lands on the deleted sheet");
    assert!(
        cells(&mut a).iter().all(|(_, _, v)| v != "ghost"),
        "the sheet the reader is on did not take it either"
    );
}
