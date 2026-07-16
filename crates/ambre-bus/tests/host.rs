// ambre-bus host tests (tier `host`, ambre-spec-architecture.md sec.4).
// ============================================================================
// Pure, native (`cargo test -p ambre-bus`) coverage of the bus data plane:
//   - the two pure helpers carried on the content plane (`to_tsv`,
//     `has_decimals`),
//   - the `RequestId` allocator invariant (monotonic, shared `AtomicU64`),
//   - postcard round-trips of the Logic <-> Sync envelopes -- the contract that
//     `SyncCommand` / `LogicEvent` (and the `Rest*` family they carry) survive
//     the `postMessage` hop unchanged, while application payloads stay `Vec<u8>`.
// No DOM, no serialization on the UI <-> Logic path (that link is `flume` by
// move and intentionally carries no `Serialize` derive).
// ============================================================================

use ambre_bus::app::toile::{ElementId, PresencePayload, ToileCommand, Viewport as ToileViewport};
use ambre_bus::{
    AppCommand, CellFormat, ChannelId, ClipboardCell, ClipboardPayload, DocType, DocumentId,
    LogicEvent, RequestId, RestCommand, RestEvent, Role, SyncCommand, TrameCommand, UserId,
};

// ── Content plane: ClipboardPayload::to_tsv ─────────────────────────────────

fn cell(dr: u32, dc: u32, raw: &str) -> ClipboardCell {
    ClipboardCell {
        dr,
        dc,
        raw: raw.to_string(),
        ..Default::default()
    }
}

#[test]
fn to_tsv_full_grid() {
    let payload = ClipboardPayload {
        rows: 2,
        cols: 2,
        cells: vec![
            cell(0, 0, "a"),
            cell(0, 1, "b"),
            cell(1, 0, "c"),
            cell(1, 1, "d"),
        ],
        ..Default::default()
    };
    assert_eq!(payload.to_tsv(), "a\tb\nc\td");
}

#[test]
fn to_tsv_gaps_render_as_empty_fields() {
    // Blank source cells are omitted from `cells`; they must appear as empty
    // fields, not collapse the row.
    let payload = ClipboardPayload {
        rows: 2,
        cols: 2,
        cells: vec![cell(0, 0, "a"), cell(1, 1, "d")],
        ..Default::default()
    };
    assert_eq!(payload.to_tsv(), "a\t\n\td");
}

#[test]
fn to_tsv_formula_cell_keeps_raw_text() {
    let payload = ClipboardPayload {
        rows: 1,
        cols: 1,
        cells: vec![cell(0, 0, "=SUM(A1:B1)")],
        ..Default::default()
    };
    assert_eq!(payload.to_tsv(), "=SUM(A1:B1)");
}

#[test]
fn to_tsv_zero_dimensions_clamp_to_one() {
    // `rows`/`cols` of 0 still produce a 1x1 grid (the `.max(1)`), never panic.
    let payload = ClipboardPayload::default();
    assert_eq!(payload.to_tsv(), "");
}

#[test]
fn to_tsv_out_of_bounds_offsets_are_ignored() {
    // A cell whose offset exceeds the declared range must not panic or grow the
    // grid; `get_mut` returns `None` and the cell is dropped.
    let payload = ClipboardPayload {
        rows: 1,
        cols: 1,
        cells: vec![cell(0, 0, "a"), cell(5, 5, "stray")],
        ..Default::default()
    };
    assert_eq!(payload.to_tsv(), "a");
}

// ── Content plane: CellFormat::has_decimals (exhaustive) ─────────────────────

#[test]
fn has_decimals_is_true_only_for_numeric_families() {
    for fmt in [
        CellFormat::Number,
        CellFormat::Currency,
        CellFormat::Percent,
        CellFormat::Scientific,
    ] {
        assert!(fmt.has_decimals(), "{fmt:?} should honour decimals");
    }
    for fmt in [
        CellFormat::General,
        CellFormat::Fraction,
        CellFormat::Date,
        CellFormat::Time,
        CellFormat::Text,
        CellFormat::Custom("dd/mm".to_string()),
    ] {
        assert!(!fmt.has_decimals(), "{fmt:?} should ignore decimals");
    }
}

// ── Service plane: RequestId allocator ──────────────────────────────────────

#[test]
fn request_id_is_strictly_monotonic() {
    let a = RequestId::next();
    let b = RequestId::next();
    let c = RequestId::next();
    assert!(a.0 < b.0, "ids must increase");
    assert!(b.0 < c.0);
}

#[test]
fn request_id_is_unique_under_concurrency() {
    use std::collections::HashSet;
    use std::sync::Arc;
    use std::sync::Mutex;

    let seen = Arc::new(Mutex::new(HashSet::new()));
    let mut handles = Vec::new();
    for _ in 0..8 {
        let seen = Arc::clone(&seen);
        handles.push(std::thread::spawn(move || {
            for _ in 0..1000 {
                let id = RequestId::next().0;
                assert!(seen.lock().unwrap().insert(id), "duplicate id {id}");
            }
        }));
    }
    for h in handles {
        h.join().unwrap();
    }
    assert_eq!(seen.lock().unwrap().len(), 8 * 1000);
}

// ── Logic <-> Sync envelope round-trips (postcard, the postMessage codec) ────

fn roundtrip_sync(cmd: &SyncCommand) {
    let bytes = postcard::to_allocvec(cmd).expect("encode SyncCommand");
    let back: SyncCommand = postcard::from_bytes(&bytes).expect("decode SyncCommand");
    // SyncCommand is not PartialEq (carries RestCommand); compare via Debug.
    assert_eq!(format!("{cmd:?}"), format!("{back:?}"));
}

fn roundtrip_event(evt: &LogicEvent) {
    let bytes = postcard::to_allocvec(evt).expect("encode LogicEvent");
    let back: LogicEvent = postcard::from_bytes(&bytes).expect("decode LogicEvent");
    assert_eq!(format!("{evt:?}"), format!("{back:?}"));
}

#[test]
fn sync_command_roundtrips_through_postcard() {
    let doc = DocumentId::new();
    let chan = ChannelId::new();
    roundtrip_sync(&SyncCommand::Rest {
        id: RequestId(42),
        cmd: RestCommand::CreateDocument {
            doc_type: DocType::Trame,
            name: "Budget".to_string(),
        },
    });
    roundtrip_sync(&SyncCommand::OpenDocument {
        document_id: doc,
        doc_type: DocType::Trame,
    });
    // A genuine network-byte payload survives verbatim.
    roundtrip_sync(&SyncCommand::SendUpdate {
        document_id: doc,
        payload: vec![0, 1, 2, 250, 255],
    });
    roundtrip_sync(&SyncCommand::SendChannelEphemeral {
        channel_id: chan,
        payload: vec![9, 8, 7],
    });
    roundtrip_sync(&SyncCommand::Shutdown);
}

#[test]
fn logic_event_roundtrips_through_postcard() {
    let doc = DocumentId::new();
    let user = UserId::new();
    roundtrip_event(&LogicEvent::Rest {
        id: RequestId(7),
        evt: RestEvent::DocumentCreated {
            document_id: doc,
            doc_type: DocType::Trame,
        },
    });
    roundtrip_event(&LogicEvent::RemoteUpdate {
        document_id: doc,
        payload: vec![3, 1, 4, 1, 5, 9],
    });
    roundtrip_event(&LogicEvent::ConnectionStatus {
        document_id: doc,
        connected: true,
    });
    roundtrip_event(&LogicEvent::MemberRevoked {
        document_id: doc,
        revoked_user_id: user,
    });
}

#[test]
fn request_id_echo_is_preserved_across_the_hop() {
    // The id allocated by the UI must be byte-identical after a Sync round-trip
    // (the whole point of the correlation token).
    let id = RequestId::next();
    let cmd = SyncCommand::Rest {
        id,
        cmd: RestCommand::InviteToDocument {
            document_id: DocumentId::new(),
            invitee_email: "x@y.z".to_string(),
            role: Role::Editor,
        },
    };
    let bytes = postcard::to_allocvec(&cmd).unwrap();
    let SyncCommand::Rest { id: back, .. } = postcard::from_bytes(&bytes).unwrap() else {
        panic!("variant changed");
    };
    assert_eq!(id.0, back.0);
}

// ── Role gate: AppCommand::is_mutating (architecture sec.2.D) ───────────────
//
// The classification decides what a viewer may still do. Getting it wrong is
// silent in both directions -- a misclassified read-only command costs a viewer
// its navigation, a misclassified write slips past the client gate -- so the
// two ends of each family are pinned here rather than trusted to review.

#[test]
fn read_only_and_local_commands_are_not_mutating() {
    let cases: Vec<AppCommand> = vec![
        // Trame: local view state.
        AppCommand::Trame(TrameCommand::ScrollViewport {
            version: 1,
            row_start: 0,
            row_end: 4,
            col_start: 0,
            col_end: 4,
        }),
        AppCommand::Trame(TrameCommand::SetActiveCell {
            row: 0,
            col: 0,
            anchor_row: 0,
            anchor_col: 0,
        }),
        AppCommand::Trame(TrameCommand::ActivateSheet {
            sheet_id: "s1".into(),
        }),
        // Trame: read-only queries.
        AppCommand::Trame(TrameCommand::RequestSelectionStats),
        AppCommand::Trame(TrameCommand::CopyRange),
        AppCommand::Trame(TrameCommand::ExportActiveSheetCsv),
        // Toile: local playback and view state.
        AppCommand::Toile(ToileCommand::NextSlide),
        AppCommand::Toile(ToileCommand::ExitPresent),
        AppCommand::Toile(ToileCommand::SetViewport(ToileViewport::default())),
        AppCommand::Toile(ToileCommand::RequestSnapshot),
        // Toile: ephemeral only -- a viewer still shows a cursor (sec.7).
        AppCommand::Toile(ToileCommand::Presence(PresencePayload::default())),
    ];
    for cmd in cases {
        assert!(
            !cmd.is_mutating(),
            "a viewer must keep this command: {cmd:?}"
        );
    }
}

#[test]
fn writing_commands_are_mutating() {
    let cases: Vec<AppCommand> = vec![
        AppCommand::Trame(TrameCommand::SetCellText {
            row: 0,
            col: 0,
            text: "x".into(),
        }),
        AppCommand::Trame(TrameCommand::InsertRowAbove),
        AppCommand::Trame(TrameCommand::PasteText { text: "x".into() }),
        AppCommand::Trame(TrameCommand::Undo),
        AppCommand::Toile(ToileCommand::DeleteElements { ids: Vec::new() }),
        AppCommand::Toile(ToileCommand::DuplicateElements {
            ids: Vec::new(),
            delta: Default::default(),
        }),
        AppCommand::Toile(ToileCommand::Undo),
        // Reacting writes a per-user CRDT entry (sec.4.A): not presence.
        AppCommand::Toile(ToileCommand::React {
            target: ElementId::mint(1, 1),
            reaction: None,
        }),
    ];
    for cmd in cases {
        assert!(cmd.is_mutating(), "this command writes the CRDT: {cmd:?}");
    }
}
