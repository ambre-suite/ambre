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
/// Per-sheet cell-style map (`cellstyle_{sheet}`, `-ui.md` sec.5bis): one atomic
/// LWW register per styled cell, keyed by the stable id pair so the style follows
/// its cell across structural ops. The value is the whole visual style of the
/// cell -- bold/italic plus the full [`CellStyle`] (alignments, wrap, colours,
/// borders, font, number format, decimals, size) -- postcard-encoded into the
/// register's binary value. One register per cell (not one per attribute) is the
/// convergence invariant: two peers that concurrently restyle the same cell race
/// exactly once, so the merged style is always one writer's, never a per-attribute
/// blend across containers (`ambre-spec-trame-logic.md` sec.2.B). Absence = the
/// grid default style.
pub(crate) fn cellstyle_map_name(sheet: &str) -> String {
    format!("cellstyle_{sheet}")
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
