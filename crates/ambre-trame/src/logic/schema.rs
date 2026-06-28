// ambre-trame/src/logic/schema.rs
// CRDT container naming and the packed style bit-layout constants. Pure
// (free) data with no `TrameState` dependency; re-exported by the parent so the
// impl submodules reach it through their `use super::*`.

/// Stable id of the implicit default sheet created by [`TrameState::new`].
///
/// A single constant (not a per-instance generated id) so that two fresh
/// `TrameState`s -- the document creator and a peer/reload restoring its
/// snapshot -- address the *same* per-sheet containers (`cells_s0`, `rows_s0`,
/// ...) and merge cleanly. Explicit additional sheets (a later step) get unique
/// ids and populate `sheet_order`; with one sheet, `sheet_order` is left empty
/// to avoid a duplicate `push` on merge (see `ambre-spec-trame-logic.md` §2).
pub(crate) const DEFAULT_SHEET: &str = "s0";

/// Loro map holding sheet metadata: `{sheet_id -> name}`. Step 1 stores the
/// name flat; the `{name: LoroText, hidden, tab_color}` sub-map of §2.B lands
/// with the sheet-tabs UI.
pub(crate) const SHEETS_MAP: &str = "sheets";

/// `LoroMovableList` holding the tab-strip order as a sequence of `sheet_id`s
/// (§2.B/§2.D). The implicit [`DEFAULT_SHEET`] is *not* eagerly pushed here (two
/// fresh peers would each push it and converge to a duplicate); it is treated as
/// the implicit first tab by [`TrameState::sheet_order_ids`]. Explicit sheets
/// are appended on [`TrameCommand::CreateSheet`].
pub(crate) const SHEET_ORDER: &str = "sheet_order";

/// Loro map holding workbook named ranges: `{name -> reference}` where the
/// reference is an A1-syntax string, optionally sheet-qualified (`Sheet2!A1:B2`).
/// LWW: the last writer of a given name wins. Resolved positionally at reseed
/// time into [`TrameState::named`] (`ambre-spec-trame-logic.md` §8, Phase 2).
pub(crate) const NAMED_RANGES_MAP: &str = "named_ranges";

/// Bit flags packed into the per-sheet `format_{sheet}` map's i64 value.
pub(crate) const FMT_BOLD: i64 = 1 << 0;
pub(crate) const FMT_ITALIC: i64 = 1 << 1;

// Bit layout of the `style_{sheet}` map's packed i64 (`-ui.md` sec.5bis). Each
// field occupies a fixed slice; a zero mask is the default style (and is deleted
// from the map so it leaves no dead entry).
pub(crate) const STYLE_UNDERLINE: i64 = 1 << 0;
pub(crate) const STYLE_WRAP: i64 = 1 << 1;
/// Horizontal align: bits 2-3 (`0` Auto, `1` Left, `2` Center, `3` Right).
pub(crate) const STYLE_HALIGN_SHIFT: i64 = 2;
pub(crate) const STYLE_HALIGN_MASK: i64 = 0b11 << STYLE_HALIGN_SHIFT;
/// Vertical align: bits 4-5 (`0` Top, `1` Middle, `2` Bottom).
pub(crate) const STYLE_VALIGN_SHIFT: i64 = 4;
pub(crate) const STYLE_VALIGN_MASK: i64 = 0b11 << STYLE_VALIGN_SHIFT;
/// Number-format family: bits 9-12 (`0` General .. `9` Custom).
pub(crate) const STYLE_NUMFMT_SHIFT: i64 = 9;
pub(crate) const STYLE_NUMFMT_MASK: i64 = 0b1111 << STYLE_NUMFMT_SHIFT;
/// Decimals + 1: bits 13-17 (`0` = unset/auto, else `value - 1` decimals).
pub(crate) const STYLE_DECIMALS_SHIFT: i64 = 13;
pub(crate) const STYLE_DECIMALS_MASK: i64 = 0b11111 << STYLE_DECIMALS_SHIFT;
/// Font size in points: bits 18-24 (`0` = default, else the size, max 127pt).
pub(crate) const STYLE_SIZE_SHIFT: i64 = 18;
pub(crate) const STYLE_SIZE_MASK: i64 = 0b1111111 << STYLE_SIZE_SHIFT;
/// Strikethrough run: bit 25.
pub(crate) const STYLE_STRIKE: i64 = 1 << 25;
/// Per-edge border presence: bits 30-33, one bit per edge in the order
/// top, right, bottom, left. Each present edge also has a 3-bit stroke style
/// in [`STYLE_BORDER_STYLE_SHIFT`]; its colour lives in the per-edge
/// `bordcolor{edge}_{sheet}` map.
pub(crate) const STYLE_BORDER_PRESENT_SHIFT: i64 = 30;
/// Per-edge stroke style: bits 34-45 (4 edges * 3 bits, same edge order as the
/// presence bits, see [`pack_bordstyle`]).
pub(crate) const STYLE_BORDER_STYLE_SHIFT: i64 = 34;

/// Maximum number of rows/columns a single write may materialize past the
/// current used-range high-water mark. Bounds the pathological "lone value far
/// down the grid" case that would otherwise create ~100k placeholder elements
/// (see `ambre-spec-trame-logic.md` §2.A bis). A write beyond it is dropped.
pub(crate) const MATERIALIZE_CAP: usize = 10_000;

/// Commit origin tagging the implicit row/column scaffolding that a cell edit
/// forces into existence (see [`TrameState::materialize`]). The `UndoManager` is
/// told to exclude this prefix, so that scaffolding never lands on the undo
/// stack -- only the user-intended value change does. Explicit structural ops
/// (`InsertRow`, etc.) use the default origin and stay undoable.
pub(crate) const MATERIALIZE_ORIGIN: &str = "mat";

/// Per-sheet container names (`cells_{sheet}`, etc.). Keyed by sheet id so a
/// multi-sheet document keeps each sheet's cells/rows/columns/formatting
/// isolated (§2.B).
pub(crate) fn cells_map_name(sheet: &str) -> String {
    format!("cells_{sheet}")
}
pub(crate) fn format_map_name(sheet: &str) -> String {
    format!("format_{sheet}")
}
/// Per-sheet rich-style map (`style_{sheet}`, `-ui.md` sec.5bis): one packed i64
/// mask per cell (underline / wrap / alignments / border / number-format family /
/// decimals / font size), keyed by the stable id pair so the style follows its
/// cell across structural ops. Colours and the font family / custom pattern --
/// which do not fit the mask -- live in the sibling maps below.
pub(crate) fn style_map_name(sheet: &str) -> String {
    format!("style_{sheet}")
}
/// Per-sheet text-colour map (`fgcolor_{sheet}`): `0xRRGGBB` packed as i64,
/// keyed by the cell id pair. Absence = theme default.
pub(crate) fn fgcolor_map_name(sheet: &str) -> String {
    format!("fgcolor_{sheet}")
}
/// Per-sheet fill-colour map (`bgcolor_{sheet}`): see [`fgcolor_map_name`].
pub(crate) fn bgcolor_map_name(sheet: &str) -> String {
    format!("bgcolor_{sheet}")
}
/// Per-sheet, per-edge border-colour map (`bordcolor{edge}_{sheet}`): `0xRRGGBB`
/// packed as i64, keyed by the cell id pair. One map per edge (`edge` in
/// `0..4`, ordered top, right, bottom, left) so each edge keeps its own colour;
/// see [`fgcolor_map_name`]. Absence = the default grid-line colour.
pub(crate) fn bordcolor_map_name(sheet: &str, edge: usize) -> String {
    format!("bordcolor{edge}_{sheet}")
}
/// Edge order for the packed presence/style bits and the per-edge colour maps:
/// top, right, bottom, left. Index into this drives both.
pub(crate) const BORDER_EDGE_ORDER: usize = 4;
/// Per-sheet font-family map (`font_{sheet}`): the family name string keyed by
/// the cell id pair. Absence = the grid default font.
pub(crate) fn font_map_name(sheet: &str) -> String {
    format!("font_{sheet}")
}
/// Per-sheet custom number-format map (`numfmt_{sheet}`): the [`CellFormat::Custom`]
/// pattern string, present only for cells whose mask selects the custom family.
pub(crate) fn numfmt_map_name(sheet: &str) -> String {
    format!("numfmt_{sheet}")
}
pub(crate) fn rows_list_name(sheet: &str) -> String {
    format!("rows_{sheet}")
}
pub(crate) fn cols_list_name(sheet: &str) -> String {
    format!("cols_{sheet}")
}
/// Per-sheet merge map (`merges_{sheet}`): a LWW `LoroMap` keyed by the master
/// (top-left) cell's stable id pair, with the bottom-right corner's id pair as
/// value (`ambre-spec-trame-logic.md` §6). Keyed by stable ids so a merge
/// follows its cells across inserts/deletes.
pub(crate) fn merges_map_name(sheet: &str) -> String {
    format!("merges_{sheet}")
}

/// Per-sheet shared-filter definition map (`filters_{sheet}`): the active filter
/// (column uid + kind), stored flat so it converges via LWW
/// (`ambre-spec-trame-features.md` §1.A). Empty map = no filter.
pub(crate) fn filters_map_name(sheet: &str) -> String {
    format!("filters_{sheet}")
}

/// Per-sheet hidden-row set (`filter_hidden_{sheet}`): `{row_uid -> true}` for
/// every row the active filter hides. Kept separate from `rows_{sheet}` to avoid
/// polluting the order list, and keyed by stable `row_uid` so the hidden set
/// follows the data across a sort/insert (§1.A).
pub(crate) fn filter_hidden_map_name(sheet: &str) -> String {
    format!("filter_hidden_{sheet}")
}

/// Per-sheet column-width map (`col_widths_{sheet}`): `{col_uid -> width_px}` as
/// an f64, keyed by the column's stable `Uid` so the width follows its column
/// across inserts/deletes/reorders (`-logic.md` §9). Absence = the uniform
/// default [`DEFAULT_COL_WIDTH`]. LWW: concurrent resizes of the same column
/// converge on the last writer, like any cell.
pub(crate) fn col_widths_map_name(sheet: &str) -> String {
    format!("col_widths_{sheet}")
}
/// Per-sheet row-height map (`row_heights_{sheet}`): `{row_uid -> height_px}`.
/// Row analogue of [`col_widths_map_name`]; absence = [`DEFAULT_ROW_HEIGHT`].
pub(crate) fn row_heights_map_name(sheet: &str) -> String {
    format!("row_heights_{sheet}")
}

/// Uniform fallback metrics (physical px) for never-resized axes. Mirror the UI
/// `COL_WIDTH` / `ROW_HEIGHT` defaults (`ui/grid.rs`); the viewport response
/// fills unset columns/rows with these so the UI gets a complete slice.
pub(crate) const DEFAULT_COL_WIDTH: f32 = 80.0;
pub(crate) const DEFAULT_ROW_HEIGHT: f32 = 20.0;
/// Lower clamp on a resize (physical px), matching the UI `MIN_COL_WIDTH` /
/// `MIN_ROW_HEIGHT`. A drag that would shrink past this is pinned here.
pub(crate) const MIN_COL_WIDTH: f32 = 24.0;
pub(crate) const MIN_ROW_HEIGHT: f32 = 12.0;

/// Which axis a structural op targets. Lets [`TrameState::insert_axis`] /
/// [`delete_axis`](TrameState::delete_axis) share one body across rows/columns.
#[derive(Clone, Copy)]
pub(crate) enum Axis {
    Row,
    Col,
}
