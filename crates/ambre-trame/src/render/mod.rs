// ambre-trame/src/render.rs
// ============================================================================
// Shared, surface-generic rendering primitives for the Trame grid.
//
// These functions are parameterized over the `DrawSurface` trait (defined in
// `ambre-draw`) so the *same* drawing code serves both:
//
//   - the UI thread, via `WgpuSurface` (live GPU rendering), called from
//     `ui::renderer` inside its dirty-rect / layer orchestration;
//   - the Logic thread, via `KrillaSurface` (vector PDF export, lazy-loaded),
//     once the PDF pipeline lands (see `trame-pdf-export`).
//
// Everything here is geometry-driven through `GridLayout` and plain cell data
// (`CellContent`). It knows nothing of scroll state, dirty rects, the HUD, or
// any other screen-only concern. Those live in `ui::renderer`, which composes
// these primitives in the right z-order.
//
// `GridLayout` is uniform-metrics for the MVP: one column width, one row
// height. It therefore stays O(1) over the virtual 100k grid, never
// materializing a per-index offset vector. Variable column widths / row heights
// (resize) will extend `GridLayout` later without changing these primitive
// signatures.
// ============================================================================

use ambre_bus::{
    CellPos, Col, ColCount, ColDelta, ColSpan, MergeSpan, Row, RowCount, RowDelta, RowSpan,
};
use ambre_canvas::{Palette, TextCache};
use ambre_core::FontSize;
use ambre_draw::{
    Affine, Alignment, Brush, Color, DrawSurface, FontContext, Layout, ParleyBrush,
    PositionedLayoutItem, Rect,
};

/// Cell text left padding (also the caret's x origin inside a cell).
pub const CELL_PAD_X: f64 = 4.0;
/// Body font size of a cell that carries none, in points.
pub const BODY_FONT: FontSize = FontSize::from_whole_points(9);
/// Body font size in px, shared by the cell renderer and the edit overlay. It
/// is [`BODY_FONT`] laid out.
pub const BODY_SIZE: f32 = 12.0;

/// The px size parley lays a cell's text out in, before the zoom. The bus
/// carries points, so an explicit size goes through [`FontSize::to_px`] as in
/// every other view.
#[must_use]
pub fn cell_font_px(size: Option<FontSize>) -> f32 {
    size.map_or(BODY_SIZE, FontSize::to_px)
}
/// Header label font size (column letters + row numbers).
pub const HEADER_SIZE: f32 = 11.0;

// -----------------------------------------------------------------------------
// Renderer split across submodules by concern (all re-exported below, so call
// sites keep naming `render::draw_cells_on`, `render::GridLayout`, ...):
//   - layout  : `GridLayout` geometry (cell/merge rects, hidden-row collapse);
//   - content : `CellContent` + colour/alignment/border helpers;
//   - text     : text drawing + column labels;
//   - cells    : the cell-body pass;
//   - grid     : grid lines;
//   - merges   : merged-cell overlays;
//   - headers  : row/column header bands;
//   - walk     : the sheet's document walk, composing the passes above in
//                sheet coordinates (`CANVAS-074`).
// -----------------------------------------------------------------------------
mod cells;
mod content;
mod grid;
mod headers;
mod layout;
mod merges;
mod text;
mod walk;

pub use cells::draw_cells_on;
pub use content::{CellContent, Decoration, Strikethrough, Underline, draw_cell_borders_on};
pub use grid::draw_grid_lines_on;
pub use headers::{draw_col_headers_on, draw_row_headers_on};
pub use layout::GridLayout;
pub use merges::draw_merges_on;
pub use text::{col_label, draw_layout_runs, draw_layout_runs_decorated, draw_text, row_label};
pub use walk::{SheetAxes, SheetCell, SheetCells, SheetView, SheetWalk, TextPass};

// -----------------------------------------------------------------------------
// Formula bar action types (shared between the UI thread and the Leptos chrome)
// -----------------------------------------------------------------------------

/// Direction the active cell moves after a formula bar commit via Enter or Tab.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CommitDirection {
    Down,
    Right,
    Stay,
}

/// Action emitted by the HTML formula bar input. It commits the edit with
/// the cursor-navigation direction, fills the whole selection with the
/// value (Ctrl/Cmd+Enter), or cancels it without writing.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum FormulaAction {
    Commit(CommitDirection),
    /// Ctrl/Cmd+Enter: write the value to every cell of the current selection,
    /// keeping the selection and active cell put.
    CommitFill,
    Cancel,
}

#[cfg(test)]
mod tests;
