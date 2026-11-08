// The workbook locale and the reader's convention (`ambre-spec-locale.md`).

use super::*;
use ambre_core::{DocumentLocale, NumberConvention, ReaderLocale};

fn locale(tag: &str) -> DocumentLocale {
    DocumentLocale::parse(tag).unwrap()
}

fn set_locale(state: &mut TrameState, tag: &str) -> TrameOutcome {
    state.handle_command(TrameCommand::SetDocumentLocale {
        locale: locale(tag),
    })
}

fn set_reader(state: &mut TrameState, own: &str, convention: NumberConvention) {
    state.handle_command(TrameCommand::SetReaderLocale {
        reader: ReaderLocale {
            own: locale(own),
            convention,
        },
    });
}

fn type_at(state: &mut TrameState, row: u32, col: u32, text: &str) {
    state.handle_command(TrameCommand::SetCellText {
        row: Pos::<Row>::new(row),
        col: Pos::<Col>::new(col),
        text: text.to_owned(),
    });
}

fn style(state: &mut TrameState, row: u32, col: u32, format: crate::CellFormat) {
    state.handle_command(TrameCommand::SetActiveCell {
        row: Pos::<Row>::new(row),
        col: Pos::<Col>::new(col),
        anchor_row: Pos::<Row>::new(row),
        anchor_col: Pos::<Col>::new(col),
        extra: vec![],
        kind: crate::SelectionKind::Cells,
    });
    state.handle_command(TrameCommand::SetCellStyle {
        patch: crate::StylePatch {
            format: Some(format),
            ..Default::default()
        },
    });
}

/// What the CRDT holds at `(row, col)`, verbatim.
fn stored(state: &TrameState, row: u32, col: u32) -> String {
    state.cell_raw(at(row, col)).into_string()
}

/// Deliver every update of `out` from one peer to another.
fn deliver(out: &TrameOutcome, to: &mut TrameState) -> Vec<TrameEvent> {
    out.update
        .as_ref()
        .map(|update| to.apply_remote(update))
        .unwrap_or_default()
}

/// `LOCALE-001`, `LOCALE-005`: the workbook carries one locale, a set moves
/// it, and a save and an open keep it.
#[test]
fn the_locale_is_one_register_that_survives_a_save_and_an_open() {
    let mut state = TrameState::new();
    assert_eq!(state.document_locale(), DocumentLocale::NONE);
    let out = set_locale(&mut state, "fr");
    assert!(out.update.is_some(), "a set is one delta");
    set_locale(&mut state, "de-CH");
    assert_eq!(state.document_locale(), locale("de-CH"));
    let mut reopened = TrameState::new();
    reopened.import_snapshot(&state.export_snapshot()).unwrap();
    assert_eq!(reopened.document_locale(), locale("de-CH"));
    // The same locale again writes nothing.
    assert!(set_locale(&mut state, "de-CH").update.is_none());
}

/// `LOCALE-004`: two peers setting the locale at once converge on one tag.
#[test]
fn two_peers_setting_the_locale_at_once_converge() {
    let mut alice = TrameState::new();
    let mut bob = TrameState::new();
    let from_alice = set_locale(&mut alice, "fr");
    let from_bob = set_locale(&mut bob, "de");
    deliver(&from_bob, &mut alice);
    deliver(&from_alice, &mut bob);
    assert_eq!(alice.document_locale(), bob.document_locale());
    assert!([locale("fr"), locale("de")].contains(&alice.document_locale()));
}

/// `LOCALE-008`, `LOCALE-009`: a locale set on one peer
/// reformats the other's view, and no stored value moves.
#[test]
fn a_locale_set_on_one_peer_reformats_every_peer_and_moves_no_value() {
    let mut alice = TrameState::new();
    let mut bob = TrameState::new();
    for (col, text) in [(0, "1234.5"), (1, "=A1*2")] {
        let typed = alice.handle_command(TrameCommand::SetCellText {
            row: Pos::<Row>::new(0),
            col: Pos::<Col>::new(col),
            text: text.to_owned(),
        });
        deliver(&typed, &mut bob);
    }
    let before = alice.export_snapshot();
    let set = set_locale(&mut alice, "fr");
    let events = deliver(&set, &mut bob);
    assert!(
        events.iter().any(
            |e| matches!(e, TrameEvent::LocaleState { document, .. } if *document == locale("fr"))
        ),
        "the peer is told the new locale"
    );
    assert!(events.iter().any(|e| matches!(e, TrameEvent::DataReset)));
    for state in [&mut alice, &mut bob] {
        assert_eq!(read_cell(state, 0, 0), "1234,5");
        assert_eq!(read_cell(state, 0, 1), "2469");
        assert_eq!(stored(state, 0, 0), "1234.5");
    }
    // The cells hold what they held before the set: only `meta` moved.
    let mut earlier = TrameState::new();
    earlier.import_snapshot(&before).unwrap();
    assert_eq!(stored(&earlier, 0, 0), stored(&alice, 0, 0));
    assert_eq!(stored(&earlier, 0, 1), stored(&alice, 0, 1));
}

/// `TRAME-110`: a currency and a date take the workbook's forms, and the
/// symbol stays the format's.
#[test]
fn a_currency_and_a_date_follow_the_document_locale() {
    let mut state = TrameState::new();
    type_at(&mut state, 0, 0, "1234.5");
    style(&mut state, 0, 0, crate::CellFormat::Currency);
    // 2026-09-29 as a sheet day serial.
    type_at(&mut state, 1, 0, "46294");
    style(&mut state, 1, 0, crate::CellFormat::Date);
    assert_eq!(read_cell(&mut state, 0, 0), "1234.50 \u{20ac}");
    assert_eq!(read_cell(&mut state, 1, 0), "2026-09-29");
    set_locale(&mut state, "fr");
    assert_eq!(read_cell(&mut state, 0, 0), "1\u{202f}234,50\u{a0}\u{20ac}");
    assert_eq!(read_cell(&mut state, 1, 0), "29/09/2026");
    set_locale(&mut state, "en-US");
    assert_eq!(read_cell(&mut state, 0, 0), "\u{20ac}1,234.50");
    assert_eq!(read_cell(&mut state, 1, 0), "9/29/2026");
    set_locale(&mut state, "de");
    assert_eq!(read_cell(&mut state, 1, 0), "29.09.2026");
}

/// `TRAME-237`, `TRAME-238`, `LOCALE-014`, `LOCALE-016`: a French reader
/// under the default convention types `3,5` and stores 3.5, in no locale.
/// The English workbook shows its own form.
#[test]
fn a_number_typed_in_the_readers_locale_is_stored_in_none() {
    let mut state = TrameState::new();
    set_locale(&mut state, "en");
    set_reader(&mut state, "fr", ReaderLocale::default().convention);
    type_at(&mut state, 0, 0, "3,5");
    type_at(&mut state, 1, 0, "1 234,5");
    type_at(&mut state, 2, 0, "un mot");
    assert_eq!(stored(&state, 0, 0), "3.5");
    assert_eq!(stored(&state, 1, 0), "1234.5");
    assert_eq!(stored(&state, 2, 0), "un mot");
    assert_eq!(read_cell(&mut state, 0, 0), "3.5");
    // The edit bar shows the reader's own form, which commits back unchanged.
    assert_eq!(read_raw(&mut state, 0, 0), "3,5");
    // An English reader of the same workbook types the English form.
    let mut english = TrameState::new();
    set_reader(&mut english, "en", NumberConvention::TypeMine);
    type_at(&mut english, 0, 0, "1,234.5");
    type_at(&mut english, 1, 0, "3,5");
    assert_eq!(stored(&english, 0, 0), "1234.5");
    assert_eq!(stored(&english, 1, 0), "3,5");
}

/// `LOCALE-015`: under `Document`, the reader types the workbook's form.
#[test]
fn under_the_document_convention_the_reader_types_the_documents_form() {
    let mut state = TrameState::new();
    set_locale(&mut state, "de");
    set_reader(&mut state, "en", NumberConvention::Document);
    type_at(&mut state, 0, 0, "1.234,5");
    assert_eq!(stored(&state, 0, 0), "1234.5");
    assert_eq!(read_cell(&mut state, 0, 0), "1234,5");
}

/// `LOCALE-017`, `LOCALE-021`: the reader's half writes nothing, and a view
/// in another locale than the workbook's is told so.
#[test]
fn the_readers_convention_writes_nothing_and_is_announced() {
    let mut state = TrameState::new();
    set_locale(&mut state, "en");
    type_at(&mut state, 0, 0, "3.5");
    let before = state.crdt().oplog_vv();
    let out = state.handle_command(TrameCommand::SetReaderLocale {
        reader: ReaderLocale {
            own: locale("fr"),
            convention: NumberConvention::AllMine,
        },
    });
    assert!(out.update.is_none());
    assert_eq!(state.crdt().oplog_vv(), before);
    assert!(out.display.iter().any(|e| matches!(
        e,
        TrameEvent::LocaleState { document, reader }
            if *document == locale("en") && *reader.shown(document) == locale("fr")
    )));
    assert_eq!(read_cell(&mut state, 0, 0), "3,5");
    assert_eq!(state.document_locale(), locale("en"));
}

/// `LOCALE-020`: a CSV export writes the workbook's forms, whatever the
/// reader sees.
#[test]
fn an_export_writes_the_documents_locale_whoever_runs_it() {
    let mut state = TrameState::new();
    set_locale(&mut state, "en");
    set_reader(&mut state, "fr", NumberConvention::AllMine);
    type_at(&mut state, 0, 0, "1234.5");
    style(&mut state, 0, 0, crate::CellFormat::Currency);
    assert_eq!(read_cell(&mut state, 0, 0), "1\u{202f}234,50\u{a0}\u{20ac}");
    let out = state.handle_command(TrameCommand::export(crate::message::TrameExport::Csv {
        sheet_id: Some(Id::<Sheet>::from_text("s0".to_owned())),
        delimiter: None,
        range: None,
    }));
    let csv = String::from_utf8(export_bytes(&out)).unwrap();
    assert!(csv.contains("\u{20ac}1,234.50"), "{csv:?}");
}

/// `LOCALE-018`, `LOCALE-023`: the edit bar shows a formula in the reader's
/// separators, and a commit stores the canonical form.
#[test]
fn a_formula_is_shown_in_the_readers_form_and_stored_canonical() {
    let mut state = TrameState::new();
    set_reader(&mut state, "fr", NumberConvention::TypeMine);
    type_at(&mut state, 0, 0, "=ROUND(2,345;2)");
    assert_eq!(stored(&state, 0, 0), "=ROUND(2.345,2)");
    assert_eq!(read_raw(&mut state, 0, 0), "=ROUND(2,345;2)");
    assert_eq!(read_cell(&mut state, 0, 0), "2.35");
    // A reader of the canonical form reads what the CRDT holds.
    let mut english = TrameState::new();
    english.import_snapshot(&state.export_snapshot()).unwrap();
    assert_eq!(read_raw(&mut english, 0, 0), "=ROUND(2.345,2)");
}

/// `LOCALE-014`, `LOCALE-022`: a numeric date typed in the reader's order is
/// stored as its day serial and shows as a date. A day past its month stays
/// text.
#[test]
fn a_date_typed_in_the_readers_order_is_stored_as_its_serial() {
    let mut state = TrameState::new();
    set_locale(&mut state, "en");
    set_reader(&mut state, "fr", NumberConvention::TypeMine);
    type_at(&mut state, 0, 0, "29/09/2026");
    assert_eq!(stored(&state, 0, 0), "46294");
    assert_eq!(read_cell(&mut state, 0, 0), "9/29/2026");
    type_at(&mut state, 1, 0, "31/02/2026");
    assert_eq!(stored(&state, 1, 0), "31/02/2026");
    // A cell with a format of its own keeps it.
    style(&mut state, 2, 0, crate::CellFormat::Number);
    type_at(&mut state, 2, 0, "01/01/2026");
    assert_eq!(read_cell(&mut state, 2, 0), "46023.00");
}
