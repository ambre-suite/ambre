// ambre-trame message tests (tier `host`, `ambre-spec-message-bus.md`).
// ============================================================================
// Pure, native coverage of the helpers and bounded values Trame's message
// vocabulary carries: `ClipboardPayload::to_tsv` and `::to_html`,
// `CellFormat::has_decimals` and the decimal count. They moved here with the vocabulary (`BUS-032`).
// ============================================================================

use ambre_bus::{Col, Row};
use ambre_core::Bounded;
use ambre_core::index::Len;
use ambre_trame::message::{CellFormat, ClipboardCell, ClipboardPayload, Decimals, DecimalsStep};

// -- Content plane: ClipboardPayload::to_tsv ---------------------------------

fn cell(dr: u32, dc: u32, raw: &str) -> ClipboardCell {
    ClipboardCell {
        dr: Len::<Row>::new(dr),
        dc: Len::<Col>::new(dc),
        raw: raw.to_string(),
        ..Default::default()
    }
}

#[test]
fn to_tsv_full_grid() {
    let payload = ClipboardPayload {
        rows: Len::<Row>::new(2),
        cols: Len::<Col>::new(2),
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
        rows: Len::<Row>::new(2),
        cols: Len::<Col>::new(2),
        cells: vec![cell(0, 0, "a"), cell(1, 1, "d")],
        ..Default::default()
    };
    assert_eq!(payload.to_tsv(), "a\t\n\td");
}

#[test]
fn to_tsv_formula_cell_keeps_raw_text() {
    let payload = ClipboardPayload {
        rows: Len::<Row>::new(1),
        cols: Len::<Col>::new(1),
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
        rows: Len::<Row>::new(1),
        cols: Len::<Col>::new(1),
        cells: vec![cell(0, 0, "a"), cell(5, 5, "stray")],
        ..Default::default()
    };
    assert_eq!(payload.to_tsv(), "a");
}

// -- Content plane: ClipboardPayload::to_html --------------------------------

/// `TRAME-UI-186`: the HTML table lays out the grid `to_tsv` lays out, a gap
/// as an empty cell, and each special character escaped.
#[test]
fn to_html_lays_out_the_tsv_grid_escaped() {
    let payload = ClipboardPayload {
        rows: Len::<Row>::new(2),
        cols: Len::<Col>::new(2),
        cells: vec![cell(0, 0, "<b>&\"'"), cell(1, 1, "=A1*2")],
        ..Default::default()
    };
    assert_eq!(
        payload.to_html(),
        "<table><tr><td>&lt;b&gt;&amp;&quot;&#39;</td><td></td></tr>\
         <tr><td></td><td>=A1*2</td></tr></table>"
    );
    assert_eq!(
        ClipboardPayload::default().to_html(),
        "<table><tr><td></td></tr></table>"
    );
}

// -- Content plane: CellFormat::has_decimals (exhaustive) ---------------------

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

/// A decimal count holds between zero and fifteen places, stepped or decoded.
#[test]
fn a_decimal_count_stays_within_fifteen_places() {
    let two = Bounded::<Decimals>::new(2).unwrap();
    assert_eq!(Bounded::<Decimals>::new(16), None);
    assert_eq!(DecimalsStep::MORE.apply_to(Bounded::MAX), Bounded::MAX);
    assert_eq!(DecimalsStep::FEWER.apply_to(Bounded::MIN), Bounded::MIN);
    assert_eq!(DecimalsStep::MORE.apply_to(two).get(), 3);
    assert_eq!(DecimalsStep::new(i8::MAX).apply_to(two), Bounded::MAX);
    assert_eq!(DecimalsStep::new(i8::MIN).apply_to(two), Bounded::MIN);
    let past: Result<Bounded<Decimals>, _> = postcard::from_bytes(&[16]);
    assert!(past.is_err(), "a decoded count past fifteen is refused");
    let bytes = postcard::to_allocvec(&two).unwrap();
    assert_eq!(bytes, [2], "the count travels as one byte");
}

/// A decimal count keeps the bytes and the words of its hand-written type. It
/// travels as one bare byte in postcard and a bare number in JSON. `Debug`
/// prints `Decimals(n)`, and a refusal keeps its text.
#[test]
fn a_decimal_count_keeps_its_wire_form_and_its_refusal() {
    for places in 0..=15_u8 {
        let count = Bounded::<Decimals>::new(places).unwrap();
        let bytes = postcard::to_allocvec(&count).unwrap();
        assert_eq!(bytes, [places]);
        assert_eq!(
            postcard::from_bytes::<Bounded<Decimals>>(&bytes).unwrap(),
            count
        );
        let json = serde_json::to_string(&count).unwrap();
        assert_eq!(json, places.to_string());
        assert_eq!(
            serde_json::from_str::<Bounded<Decimals>>(&json).unwrap(),
            count
        );
    }
    let refused = serde_json::from_str::<Bounded<Decimals>>("16").unwrap_err();
    assert!(
        refused
            .to_string()
            .starts_with("a cell carries at most 15 decimal places, not 16"),
        "{refused}"
    );
    assert!(postcard::from_bytes::<Bounded<Decimals>>(&[255]).is_err());
    assert_eq!(
        Bounded::<Decimals>::try_new(16).unwrap_err().to_string(),
        "a cell carries at most 15 decimal places, not 16"
    );
    assert_eq!(format!("{:?}", Bounded::<Decimals>::MAX), "Decimals(15)");
}

/// A conditional-format rank keeps the `Debug` text the hand-written type
/// printed. A log reads that form alone, since the UI to Logic hop moves the
/// command and serializes nothing.
#[test]
fn a_cond_format_rank_keeps_its_debug_text() {
    use ambre_bus::Rank;
    use ambre_trame::message::CondFormatPriority;
    let rank = Rank::<CondFormatPriority>::new(-2);
    assert_eq!(rank.get(), -2_i64);
    assert!(rank < Rank::new(0));
    assert_eq!(format!("{rank:?}"), "CondFormatPriority(-2)");
}
