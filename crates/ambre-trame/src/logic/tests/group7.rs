// ambre-trame/src/logic/tests/group7.rs
// TrameState unit tests (group 7/7). Helpers via `use super::*`;
// logic-module items are reached with `super::super::` paths.

use super::*;

const COL_1: Col = Col::new(1);

/// The user's real scenario: type 6 columns x 10 rows (A1:F10), then export.
/// The output must be exactly 6 columns wide, no trailing empties.
#[test]
fn csv_export_typed_6x10_has_no_trailing_columns() {
    let mut state = TrameState::new();
    for row in 0..10u32 {
        for col in 0..6u32 {
            state.handle_command(TrameCommand::SetCellText {
                row: Row::new(row),
                col: Col::new(col),
                text: (14 + col).to_string(),
            });
        }
    }
    let out = state.handle_command(TrameCommand::ExportActiveSheetCsv);
    let text = String::from_utf8(export_bytes(&out)).unwrap();
    for line in text.lines() {
        assert_eq!(
            line, "14,15,16,17,18,19",
            "line had trailing empties: {line:?}"
        );
    }
}

/// Export emits formula results, not the formula source.
#[test]
fn csv_export_evaluates_formulas() {
    let mut state = TrameState::new();
    state.handle_command(TrameCommand::SetCellText {
        row: Row::new(0),
        col: Col::new(0),
        text: "2".to_owned(),
    });
    state.handle_command(TrameCommand::SetCellText {
        row: Row::new(0),
        col: Col::new(1),
        text: "=A1*3".to_owned(),
    });
    let out = state.handle_command(TrameCommand::ExportCsv {
        sheet_id: SheetId("s0".to_owned()),
        delimiter: None,
        range: None,
    });
    let bytes = export_bytes(&out);
    assert_eq!(String::from_utf8(bytes).unwrap(), "2,6\n");
}

/// Repro: a brand-new document where the author types a value, then autofills a
/// numeric series down. A peer that re-opens the document hydrates from the
/// replayed incremental `SendUpdate` deltas, not a full snapshot. That replay is
/// what "going back to the home page and reopening" does on the wire. The
/// reopened peer must see the full filled series, not just the seed cell.
#[test]
fn autofill_series_survives_incremental_delta_replay() {
    use ambre_bus::FillMode;

    let mut author = TrameState::new();

    // A1 = 1 (the seed).
    let u1 = author
        .handle_command(TrameCommand::SetCellText {
            row: Row::new(0),
            col: Col::new(0),
            text: "1".to_owned(),
        })
        .update
        .expect("seed write produces a delta");

    // Autofill A1 down to A4: Auto mode on a single number -> 1,2,3,4.
    let u2 = author
        .handle_command(TrameCommand::FillRange {
            sheet_id: SheetId("s0".to_owned()),
            src: crate::CellRect::spanning(
                crate::CellPos::new(Row::new(0), Col::new(0)),
                crate::CellPos::new(Row::new(0), Col::new(0)),
            ),
            dest: crate::CellRect::spanning(
                crate::CellPos::new(Row::new(0), Col::new(0)),
                crate::CellPos::new(Row::new(3), Col::new(0)),
            ),
            mode: FillMode::Series,
        })
        .update
        .expect("fill produces a delta");

    // Author sees the full series locally.
    assert_eq!(read_cell(&mut author, 0, 0), "1");
    assert_eq!(read_cell(&mut author, 1, 0), "2");
    assert_eq!(read_cell(&mut author, 2, 0), "3");
    assert_eq!(read_cell(&mut author, 3, 0), "4");

    // Reopen: a fresh peer replays the broadcast deltas in order.
    let mut reopened = TrameState::new();
    reopened.apply_remote(&u1);
    reopened.apply_remote(&u2);

    assert_eq!(read_cell(&mut reopened, 0, 0), "1", "seed survives reopen");
    assert_eq!(read_cell(&mut reopened, 1, 0), "2", "A2 survives reopen");
    assert_eq!(read_cell(&mut reopened, 2, 0), "3", "A3 survives reopen");
    assert_eq!(read_cell(&mut reopened, 3, 0), "4", "A4 survives reopen");
}

/// Generalized guard against the autofill-class bug: any batched command that
/// formats cells must broadcast a self-contained incremental delta. The
/// convergence proptest exchanges full snapshots, so it cannot catch an
/// incremental delta that drops ops. That is the failure mode where a per-cell
/// helper flushed mid-batch and dropped the delta. Here a single author formats
/// a 3x3 selection whose rows/columns are NOT yet materialized, forcing
/// `persist_format` to scaffold lines mid-loop. A fresh peer then replays only
/// the broadcast delta. Every formatted cell must survive the reopen.
#[test]
fn batched_formatting_broadcasts_complete_delta() {
    let mut author = TrameState::new();

    // Select A1:C3 on an empty sheet: nothing is materialized yet, so the bold
    // pass must create rows 0..2 and columns 0..2 as it goes.
    author.handle_command(TrameCommand::SetActiveCell {
        row: Row::new(0),
        col: Col::new(0),
        anchor_row: Row::new(2),
        anchor_col: Col::new(2),
        extra: vec![],
        kind: crate::SelectionKind::Cells,
    });
    let delta = author
        .handle_command(TrameCommand::SetCellStyle {
            patch: bold_patch(true),
        })
        .update
        .expect("formatting a selection produces a delta");

    // Reopen: a fresh peer replays only the broadcast delta.
    let mut reopened = TrameState::new();
    reopened.apply_remote(&delta);

    for row in 0..=2 {
        for col in 0..=2 {
            assert!(
                read_display(&mut reopened, row, col).bold,
                "bold on ({row},{col}) must survive an incremental reopen"
            );
        }
    }
}

/// Repro: edits made AFTER a reopen must broadcast a causally self-contained
/// delta. A reopened session starts from a fresh `LoroDoc` (new random PeerID)
/// whose constructor mints local genesis ops, THEN imports the document history.
/// If the export cursor is advanced past those never-broadcast local ops, a
/// later edit's delta depends on them. That is a dangling causal dependency a
/// third peer, the next reopen, cannot apply, so the edit silently vanishes.
/// `TRAME-231`: a reader that has sent nothing holds exactly its author's
/// history. Its scaffolding is the author's, so a later change finds no
/// concurrent operation to import against.
#[test]
fn a_reader_that_sent_nothing_holds_no_history_of_its_own() {
    let mut author = TrameState::new();
    author.handle_command(TrameCommand::SetCellText {
        row: Row::new(0),
        col: Col::new(0),
        text: "1".to_owned(),
    });
    let mut reader = TrameState::new();
    reader
        .import_snapshot(&author.export_snapshot())
        .expect("the snapshot imports");
    assert_eq!(
        reader.doc.oplog_vv(),
        author.doc.oplog_vv(),
        "the scaffolding is shared"
    );
    let delta = author
        .handle_command(TrameCommand::SetCellText {
            row: Row::new(1),
            col: Col::new(0),
            text: "2".to_owned(),
        })
        .update
        .expect("a write produces a delta");
    reader.apply_remote(&delta);
    assert_eq!(reader.doc.oplog_vv(), author.doc.oplog_vv());
    assert_eq!(read_cell(&mut reader, 1, 0), "2");
}

#[test]
fn edit_after_reopen_broadcasts_causally_complete_delta() {
    // Original author creates the doc and writes A1.
    let mut author = TrameState::new();
    let u_a1 = author
        .handle_command(TrameCommand::SetCellText {
            row: Row::new(0),
            col: Col::new(0),
            text: "1".to_owned(),
        })
        .update
        .expect("A1 write produces a delta");

    // Reopen #1: fresh peer hydrates from the broadcast history, then writes A2.
    let mut reopened = TrameState::new();
    reopened.apply_remote(&u_a1);
    let u_a2 = reopened
        .handle_command(TrameCommand::SetCellText {
            row: Row::new(1),
            col: Col::new(0),
            text: "1".to_owned(),
        })
        .update
        .expect("A2 write produces a delta");

    // Reopen #2: another fresh peer replays the full broadcast history.
    let mut reopened2 = TrameState::new();
    reopened2.apply_remote(&u_a1);
    reopened2.apply_remote(&u_a2);

    assert_eq!(
        read_cell(&mut reopened2, 0, 0),
        "1",
        "A1 survives reopen #2"
    );
    assert_eq!(
        read_cell(&mut reopened2, 1, 0),
        "1",
        "A2 survives reopen #2"
    );
}

/// A server history replay applied as a batch repaints once. `apply_remote_batch`
/// merges every delta, then emits a single `DataReset` rather than one per delta,
/// while reconstructing the full grid. This is the fix for the reopen flicker
/// where a cell present only in the last delta flashed in across N intermediate
/// renders.
#[test]
fn apply_remote_batch_emits_single_datareset_with_full_state() {
    let mut author = TrameState::new();
    let u1 = author
        .handle_command(TrameCommand::SetCellText {
            row: Row::new(0),
            col: Col::new(0),
            text: "1".to_owned(),
        })
        .update
        .expect("A1 delta");
    let u2 = author
        .handle_command(TrameCommand::SetCellText {
            row: Row::new(1),
            col: Col::new(0),
            text: "2".to_owned(),
        })
        .update
        .expect("A2 delta");

    let mut reopened = TrameState::new();
    let events = reopened.apply_remote_batch(&[u1, u2]);

    let resets = events
        .iter()
        .filter(|e| matches!(e, TrameEvent::DataReset))
        .count();
    assert_eq!(resets, 1, "the whole replay repaints exactly once");

    assert_eq!(read_cell(&mut reopened, 0, 0), "1");
    assert_eq!(read_cell(&mut reopened, 1, 0), "2");
}

#[test]
fn resize_col_round_trips_through_viewport() {
    let mut state = TrameState::new();
    let evts = state.handle_command(TrameCommand::ResizeCol {
        version: ambre_bus::ViewportVersion(1),
        sheet_id: default_sheet(),
        col: Col::new(1),
        width: ambre_core::DocPx::new(150.0),
    });
    assert!(
        matches!(evts.display.first(), Some(TrameEvent::DataReset)),
        "resize replies a DataReset so the UI re-fetches"
    );
    assert!(
        evts.update.is_some(),
        "resize is a synchronised CRDT mutation"
    );

    // Columns 0..=2: only col 1 carries the new width; the rest stay default.
    let (widths, _) = viewport_metrics(&mut state, 0, 0, 0, 2);
    assert_eq!(widths, vec![80.0, 150.0, 80.0]);
}

#[test]
fn resize_row_round_trips_and_clamps_to_minimum() {
    let mut state = TrameState::new();
    // A sub-minimum request is pinned to MIN_ROW_HEIGHT (12.0).
    state.handle_command(TrameCommand::ResizeRow {
        version: ambre_bus::ViewportVersion(1),
        sheet_id: default_sheet(),
        row: Row::new(2),
        height: ambre_core::DocPx::new(1.0),
    });
    let (_, heights) = viewport_metrics(&mut state, 0, 3, 0, 0);
    assert_eq!(heights, vec![20.0, 20.0, 12.0, 20.0]);
}

#[test]
fn resize_survives_snapshot_roundtrip() {
    let mut state = TrameState::new();
    state.handle_command(TrameCommand::ResizeCol {
        version: ambre_bus::ViewportVersion(1),
        sheet_id: default_sheet(),
        col: Col::new(0),
        width: ambre_core::DocPx::new(200.0),
    });
    let snapshot = state.export_snapshot();
    let mut restored = TrameState::new();
    restored
        .import_snapshot(&snapshot)
        .expect("a snapshot this build wrote reads back at the current schema");
    let (widths, _) = viewport_metrics(&mut restored, 0, 0, 0, 0);
    assert_eq!(widths, vec![200.0], "the persisted width reloads");
}

#[test]
fn concurrent_resize_of_same_col_converges_lww() {
    // Two peers resize the same column to different widths, then exchange full
    // snapshots both ways: LWW must leave them on one identical value.
    let mut a = TrameState::new();
    let mut b = TrameState::new();
    a.handle_command(TrameCommand::ResizeCol {
        version: ambre_bus::ViewportVersion(1),
        sheet_id: default_sheet(),
        col: Col::new(0),
        width: ambre_core::DocPx::new(120.0),
    });
    b.handle_command(TrameCommand::ResizeCol {
        version: ambre_bus::ViewportVersion(1),
        sheet_id: default_sheet(),
        col: Col::new(0),
        width: ambre_core::DocPx::new(300.0),
    });
    let (sa, sb) = (a.export_snapshot(), b.export_snapshot());
    a.import_snapshot(&sb)
        .expect("a snapshot this build wrote reads back at the current schema");
    b.import_snapshot(&sa)
        .expect("a snapshot this build wrote reads back at the current schema");
    let (wa, _) = viewport_metrics(&mut a, 0, 0, 0, 0);
    let (wb, _) = viewport_metrics(&mut b, 0, 0, 0, 0);
    assert_eq!(wa, wb, "both peers converge to one width");
    assert!(
        wa[0] == 120.0 || wa[0] == 300.0,
        "converged on one of the writes"
    );
}

/// `CopyRange` builds the clipboard payload from the whole document, not a
/// viewport cache (`trame-clipboard`). The selection spans a range far larger
/// than any single viewport, with a populated cell near its bottom edge. The
/// authoritative Logic-side build must include it, proving the old best-effort
/// out-of-viewport limit is gone.
#[test]
fn copy_range_covers_the_full_selection_not_a_viewport() {
    let mut state = TrameState::new();
    // Two literals far apart: A1 (0,0) and a cell hundreds of rows down.
    state.handle_command(TrameCommand::SetCellText {
        row: Row::new(0),
        col: Col::new(0),
        text: "top".to_owned(),
    });
    state.handle_command(TrameCommand::SetCellText {
        row: Row::new(500),
        col: Col::new(3),
        text: "bottom".to_owned(),
    });
    // Select the whole rectangle spanning both (top-left A1 -> row 500, col 3).
    state.handle_command(TrameCommand::SetActiveCell {
        row: Row::new(0),
        col: Col::new(0),
        anchor_row: Row::new(500),
        anchor_col: Col::new(3),
        extra: vec![],
        kind: crate::SelectionKind::Cells,
    });

    let out = state.handle_command(TrameCommand::CopyRange);
    let TrameEvent::Clipboard(payload) = &out.display[0] else {
        panic!("expected a Clipboard event");
    };
    assert_eq!(payload.src_row, Row::new(0));
    assert_eq!(payload.src_col, Col::new(0));
    assert_eq!(payload.rows, RowCount::new(501));
    assert_eq!(payload.cols, ColCount::new(4));
    // Both non-empty cells are present, including the far one a viewport-based
    // build would have dropped; blanks in between are omitted.
    assert_eq!(payload.cells.len(), 2);
    let top = payload
        .cells
        .iter()
        .find(|c| c.dr == RowCount::new(0) && c.dc == ColCount::new(0))
        .unwrap();
    assert_eq!(top.raw, "top");
    let bottom = payload
        .cells
        .iter()
        .find(|c| c.dr == RowCount::new(500) && c.dc == ColCount::new(3))
        .unwrap();
    assert_eq!(bottom.raw, "bottom");
    // Non-mutating: no CRDT delta emitted.
    assert!(out.update.is_none(), "copy must not mutate the document");
}

/// A1 holds 0.5 shown as a percentage with one decimal.
fn percent_sheet() -> TrameState {
    let mut state = TrameState::new();
    state.handle_command(TrameCommand::SetCellText {
        row: Row::new(0),
        col: Col::new(0),
        text: "0.5".to_owned(),
    });
    state.handle_command(TrameCommand::SetCellStyle {
        patch: ambre_bus::StylePatch {
            format: Some(ambre_bus::CellFormat::Percent),
            decimals: Some(crate::Decimals::new(1)),
            ..Default::default()
        },
    });
    state
}

/// Find reads a cell as the grid shows it (`TRAME-UI-078`, `TRAME-108`). The
/// percentage is found by what the user reads, not by the value behind it.
#[test]
fn find_matches_the_formatted_value() {
    let mut state = percent_sheet();
    let hits = |state: &mut TrameState, query: &str| {
        let out = state.handle_command(TrameCommand::FindNext {
            query: ambre_bus::app::trame::FindQuery::new(query.to_owned(), crate::FindOptions {
                extent: crate::MatchExtent::WholeCell,
                ..Default::default()
            }),
        });
        match &out.display[..] {
            [TrameEvent::FindResult { matches, .. }] => matches.len(),
            other => panic!("expected FindResult, got {other:?}"),
        }
    };
    assert_eq!(hits(&mut state, "50.0%"), 1);
    assert_eq!(hits(&mut state, "0.5"), 0);
}

/// CSV export writes a cell as the grid shows it (`TRAME-108`).
#[test]
fn csv_export_writes_the_formatted_value() {
    let mut state = percent_sheet();
    let out = state.handle_command(TrameCommand::ExportCsv {
        sheet_id: SheetId("s0".to_owned()),
        delimiter: None,
        range: None,
    });
    assert_eq!(String::from_utf8(export_bytes(&out)).unwrap(), "50.0%\n");
}

/// An import reports the records it could not read (`TRAME-IO-039`): the
/// readable ones land, and the user is told how many were skipped.
#[test]
fn import_reports_unreadable_records() {
    let mut state = TrameState::new();
    let out = state.handle_command(TrameCommand::ImportCsv {
        data: ambre_core::FileBytes::new(b"a,b\n\xff\xfe,c\nd,e\n".to_vec()),
        delimiter: None,
        header: HeaderRow::Absent,
        target_sheet_id: Some(SheetId("s0".to_owned())),
        start_row: Row::new(0),
        start_col: Col::new(0),
    });
    assert!(
        out.display
            .iter()
            .any(|e| matches!(e, TrameEvent::Error { message } if message.starts_with("1 record"))),
        "the skipped record is reported: {:?}",
        out.display
    );
    assert_eq!(read_cell(&mut state, 0, 0), "a");
    assert_eq!(read_cell(&mut state, 1, 0), "d");
}

/// An export with nothing to write says so (`TRAME-IO-041`): an empty sheet
/// and an unknown one each answer with a notice, never with silence.
#[test]
fn export_with_nothing_to_write_says_so() {
    let mut state = TrameState::new();
    for sheet_id in ["s0", "nope"] {
        let out = state.handle_command(TrameCommand::ExportCsv {
            sheet_id: SheetId(sheet_id.to_owned()),
            delimiter: None,
            range: None,
        });
        assert!(
            matches!(&out.display[..], [TrameEvent::Error { .. }]),
            "{sheet_id}: {:?}",
            out.display
        );
    }
}

/// An auto-fit measures the whole column (`TRAME-UI-128`): the content Logic
/// returns reaches the rows no viewport has loaded, and leaves out the blanks.
#[test]
fn column_content_reaches_every_row() {
    let mut state = TrameState::new();
    state.handle_command(TrameCommand::SetCellText {
        row: Row::new(0),
        col: Col::new(1),
        text: "top".to_owned(),
    });
    state.handle_command(TrameCommand::SetCellText {
        row: Row::new(900),
        col: Col::new(1),
        text: "a long label far below the loaded window".to_owned(),
    });
    let out = state.handle_command(TrameCommand::RequestColumnContent { col: Col::new(1) });
    assert!(out.update.is_none(), "a query writes nothing");
    let [TrameEvent::ColumnContent { col: COL_1, cells }] = &out.display[..] else {
        panic!("expected ColumnContent, got {:?}", out.display)
    };
    let rows: Vec<u32> = cells.iter().map(|c| c.row.get()).collect();
    assert_eq!(rows, vec![0, 900]);
}
