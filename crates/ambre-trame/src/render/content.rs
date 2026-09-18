// ambre-trame/src/render/content.rs.rs
//
// Part of the Trame software renderer, split out of render.rs by concern.
// Submodule of `render`; reaches sibling helpers, constants and imports through
// the `use super::*` glob (the parent re-exports every renderer item).

use super::*;

// -----------------------------------------------------------------------------
// Cell content view
// -----------------------------------------------------------------------------

/// A borrowed view of one cell's renderable content. Both the screen cache
/// (`ui::grid::CellData`) and a future `DisplayViewport` map onto this, so the
/// primitives never depend on either concrete type.
#[derive(Clone, Copy)]
pub struct CellContent<'a> {
    pub text: &'a str,
    pub bold: bool,
    pub italic: bool,
    /// Underline run drawn under the glyphs (`trame-cell-styles`).
    pub underline: bool,
    /// Strikethrough run drawn through the glyphs.
    pub strikethrough: bool,
    /// Horizontal alignment ([`HAlign::Auto`] resolves numbers right, text left).
    pub h_align: ambre_bus::HAlign,
    /// Vertical alignment within the cell (or the span rect for a merge master).
    pub v_align: ambre_bus::VAlign,
    /// Text colour, packed `0xRRGGBB` (`None` = the palette default).
    pub fg: Option<u32>,
    /// Fill colour, packed `0xRRGGBB` (`None` = the palette `content_bg`).
    pub bg: Option<u32>,
    /// Font family name (`None` = the grid default).
    pub font: Option<&'a str>,
    /// Font size in points (`None` = the grid default [`BODY_SIZE`]).
    pub size: Option<u16>,
    /// Wrap onto multiple lines inside the cell instead of overflowing.
    pub wrap: bool,
    /// Per-edge border (style + colour each); `None` edges are not drawn.
    pub borders: ambre_bus::Borders,
}

impl<'a> CellContent<'a> {
    /// A plain cell view (no rich style): the common case for tests and any caller
    /// that only carries text + bold/italic.
    pub fn plain(text: &'a str, bold: bool, italic: bool) -> Self {
        Self {
            text,
            bold,
            italic,
            underline: false,
            strikethrough: false,
            h_align: ambre_bus::HAlign::Auto,
            v_align: ambre_bus::VAlign::Middle,
            fg: None,
            bg: None,
            font: None,
            size: None,
            wrap: false,
            borders: ambre_bus::Borders::default(),
        }
    }
}

/// Unpack a `0xRRGGBB` colour into a [`Color`].
pub(super) fn rgb(packed: u32) -> Color {
    Color::from_rgb8(
        ((packed >> 16) & 0xFF) as u8,
        ((packed >> 8) & 0xFF) as u8,
        (packed & 0xFF) as u8,
    )
}

/// Resolve a cell's parley alignment from its [`HAlign`], deferring `Auto` to the
/// numeric-vs-text heuristic on `text`.
pub(super) fn cell_alignment(h: ambre_bus::HAlign, text: &str) -> Alignment {
    match h {
        ambre_bus::HAlign::Left => Alignment::Start,
        ambre_bus::HAlign::Center => Alignment::Center,
        ambre_bus::HAlign::Right => Alignment::End,
        ambre_bus::HAlign::Auto => {
            if text.parse::<f64>().is_ok() {
                Alignment::End
            } else {
                Alignment::Start
            }
        }
    }
}

/// Vertical text offset inside a box of height `box_h` for a text block of height
/// `text_h`, honouring the cell's [`VAlign`](ambre_bus::VAlign). Clamped to `>= 0`
/// so oversized text never lifts above the box top.
pub(super) fn cell_v_offset(v: ambre_bus::VAlign, box_h: f64, text_h: f64) -> f64 {
    match v {
        ambre_bus::VAlign::Top => 0.0,
        ambre_bus::VAlign::Middle => ((box_h - text_h) / 2.0).max(0.0),
        ambre_bus::VAlign::Bottom => (box_h - text_h).max(0.0),
    }
}

/// The default border colour (a mid grey), darker than the grid line so a border
/// reads as a deliberate edge.
const BORDER_DEFAULT: Color = Color::from_rgb8(0x60, 0x60, 0x60);

/// Resolve a [`BorderStyle`] into a stroke `(width, dash_period)`; `dash_period`
/// of `0.0` means a solid stroke. `Double` is handled by the caller (two passes).
fn border_stroke(style: ambre_bus::BorderStyle) -> (f64, f64) {
    use ambre_bus::BorderStyle as Bs;
    match style {
        Bs::Solid => (1.0, 0.0),
        Bs::Dashed => (1.0, 6.0),
        Bs::Dotted => (1.0, 2.5),
        Bs::Medium => (2.0, 0.0),
        Bs::Thick => (3.0, 0.0),
        Bs::Double => (1.0, 0.0),
    }
}

/// Stroke a straight axis-aligned segment in the given border `style`. Dashed and
/// dotted patterns are drawn as a run of short segments, because [`DrawSurface`]
/// has no dash parameter. Solid, medium and thick are a single stroke. `Double`
/// draws two thin parallel lines straddling the edge.
fn styled_segment<S: DrawSurface>(
    surface: &mut S,
    x0: f64,
    y0: f64,
    x1: f64,
    y1: f64,
    color: Color,
    style: ambre_bus::BorderStyle,
) {
    use ambre_bus::BorderStyle as Bs;
    let (width, period) = border_stroke(style);
    if style == Bs::Double {
        // Two 1px lines offset by 1px on each side of the edge.
        let (dx, dy) = if (y1 - y0).abs() < f64::EPSILON {
            (0.0, 1.0)
        } else {
            (1.0, 0.0)
        };
        surface.stroke_line(x0 - dx, y0 - dy, x1 - dx, y1 - dy, color, 1.0);
        surface.stroke_line(x0 + dx, y0 + dy, x1 + dx, y1 + dy, color, 1.0);
        return;
    }
    if period <= 0.0 {
        surface.stroke_line(x0, y0, x1, y1, color, width);
        return;
    }
    // Dashed / dotted: step along the segment drawing `on` runs.
    let len = ((x1 - x0).powi(2) + (y1 - y0).powi(2)).sqrt();
    if len <= 0.0 {
        return;
    }
    let (ux, uy) = ((x1 - x0) / len, (y1 - y0) / len);
    let on = period * 0.6;
    let mut t = 0.0;
    while t < len {
        let s = t;
        let e = (t + on).min(len);
        surface.stroke_line(
            x0 + ux * s,
            y0 + uy * s,
            x0 + ux * e,
            y0 + uy * e,
            color,
            width,
        );
        t += period;
    }
}

/// Stroke a cell's set border edges, each in its own style/colour, over `rect`.
fn draw_cell_border<S: DrawSurface>(surface: &mut S, rect: Rect, content: &CellContent<'_>) {
    let b = &content.borders;
    let mut edge = |cb: Option<ambre_bus::CellBorder>, x0, y0, x1, y1| {
        if let Some(cb) = cb {
            let color = cb.color.map(rgb).unwrap_or(BORDER_DEFAULT);
            styled_segment(surface, x0, y0, x1, y1, color, cb.style);
        }
    };
    edge(b.top, rect.x0, rect.y0, rect.x1, rect.y0);
    edge(b.bottom, rect.x0, rect.y1, rect.x1, rect.y1);
    edge(b.left, rect.x0, rect.y0, rect.x0, rect.y1);
    edge(b.right, rect.x1, rect.y0, rect.x1, rect.y1);
}

/// Whether any of the four edges carries a border (drives the border pass).
fn has_border(b: &ambre_bus::Borders) -> bool {
    b.top.is_some() || b.right.is_some() || b.bottom.is_some() || b.left.is_some()
}

/// Draw the border layer for each supplied `(rect, content)`. Called **after** the
/// grid lines (and the merge repaint), so a deliberate border is never
/// overpainted. `rect` is the owning cell's rect, or the full span rect for a
/// merge master.
pub fn draw_cell_borders_on<'a, S, I>(surface: &mut S, cells: I)
where
    S: DrawSurface,
    I: IntoIterator<Item = (Rect, CellContent<'a>)>,
{
    for (rect, content) in cells {
        if has_border(&content.borders) {
            draw_cell_border(surface, rect, &content);
        }
    }
}
