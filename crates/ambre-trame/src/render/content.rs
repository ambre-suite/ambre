// ambre-trame/src/render/content.rs.rs
//
// Part of the Trame software renderer, split out of render.rs by concern.
// Submodule of `render`; reaches sibling helpers, constants and imports through
// the `use super::*` glob (the parent re-exports every renderer item).

use super::*;
use ambre_core::Flag as _;
use ambre_core::Rgb;

// -----------------------------------------------------------------------------
// Cell content view
// -----------------------------------------------------------------------------

/// A borrowed view of one cell's renderable content. Both the screen cache
/// (`ui::grid::CellData`) and a future `DisplayViewport` map onto this, so the
/// primitives never depend on either concrete type.
#[derive(Clone, Copy)]
pub struct CellContent<'a> {
    pub text: &'a str,
    pub bold: Weight,
    pub italic: Slant,
    /// Underline run drawn under the glyphs (`trame-cell-styles`).
    pub underline: Underline,
    /// Strikethrough run drawn through the glyphs.
    pub strikethrough: Strikethrough,
    /// Horizontal alignment ([`HAlign::Auto`] resolves numbers right, text left).
    pub h_align: crate::message::HAlign,
    /// What the computed value is: what [`HAlign::Auto`] resolves on.
    pub numeric: CellValueKind,
    /// Vertical alignment within the cell (or the span rect for a merge master).
    pub v_align: crate::message::VAlign,
    /// Text colour, packed `0xRRGGBB` (`None` = the palette default).
    pub fg: Option<Rgb>,
    /// Fill colour, packed `0xRRGGBB` (`None` = the palette `content_bg`).
    pub bg: Option<Rgb>,
    /// Font family (`None` = the grid default).
    pub font: Option<&'a ambre_core::FontFamily>,
    /// Font size (`None` = the grid default [`BODY_FONT`](super::BODY_FONT)).
    pub size: Option<ambre_core::FontSize>,
    /// Wrap onto multiple lines inside the cell instead of overflowing.
    pub wrap: WordBreak,
    /// Per-edge border (style + colour each); `None` edges are not drawn.
    pub borders: crate::message::Borders,
}

impl<'a> CellContent<'a> {
    /// A plain cell view, with no style: the common case for tests. A caller
    /// that styles the text sets the fields over it.
    pub fn plain(text: &'a str) -> Self {
        Self {
            text,
            bold: Weight::Regular,
            italic: Slant::Upright,
            underline: Underline::Plain,
            strikethrough: Strikethrough::Plain,
            h_align: crate::message::HAlign::Auto,
            numeric: CellValueKind::from_bool(crate::formula::types::finite_number(text).is_some()),
            v_align: crate::message::VAlign::Middle,
            fg: None,
            bg: None,
            font: None,
            size: None,
            wrap: WordBreak::Whole,
            borders: crate::message::Borders::default(),
        }
    }
}

/// The lines a run of cell text carries over its glyphs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Decoration {
    pub underline: Underline,
    pub strikethrough: Strikethrough,
}

use crate::message::CellValueKind;
use ambre_core::{Px, Surface};
use ambre_core::{Slant, Weight, WordBreak};
pub use ambre_core::{Strikethrough, Underline};

impl Decoration {
    /// The lines the style of `content` asks for.
    pub fn of(content: &CellContent<'_>) -> Self {
        Self {
            underline: content.underline,
            strikethrough: content.strikethrough,
        }
    }
}

/// Unpack a `0xRRGGBB` colour into a [`Color`].
pub(super) fn rgb(packed: Rgb) -> Color {
    let [r, g, b] = packed.channels();
    Color::from_rgb8(r, g, b)
}

/// Resolve a cell's parley alignment from its [`HAlign`], deferring `Auto` to
/// whether the cell computes to a number. The formatted text is not asked: a
/// currency or a fraction no longer parses as one (`TRAME-UI-093`).
pub(super) fn cell_alignment(h: crate::message::HAlign, numeric: CellValueKind) -> Alignment {
    match (h, numeric) {
        (crate::message::HAlign::Left, _) | (crate::message::HAlign::Auto, CellValueKind::Text) => {
            Alignment::Start
        }
        (crate::message::HAlign::Center, _) => Alignment::Center,
        (crate::message::HAlign::Right, _)
        | (crate::message::HAlign::Auto, CellValueKind::Number) => Alignment::End,
    }
}

/// The two heights a vertical alignment places a text block by. Named
/// fields, so the box and the text cannot trade places.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct TextFit {
    /// The height of the box the text sits in.
    pub(super) box_height: Px<Surface>,
    /// The height of the laid-out text block.
    pub(super) text_height: Px<Surface>,
}

/// Vertical text offset of a text block inside its box, honouring the cell's
/// [`VAlign`](crate::message::VAlign). Clamped to `>= 0` so oversized text never
/// lifts above the box top.
pub(super) fn cell_v_offset(v: crate::message::VAlign, fit: TextFit) -> Px<Surface> {
    let room = fit.box_height - fit.text_height;
    match v {
        crate::message::VAlign::Top => Px::<Surface>::ZERO,
        crate::message::VAlign::Middle => (room / 2.0).max(Px::<Surface>::ZERO),
        crate::message::VAlign::Bottom => room.max(Px::<Surface>::ZERO),
    }
}

/// The stroke a [`BorderStyle`] draws with. `Double` is handled by the
/// caller (two passes).
#[derive(Debug, Clone, Copy, PartialEq)]
struct BorderStroke {
    width: Px<Surface>,
    /// The length of one dash and its gap; zero draws a solid stroke.
    dash_period: Px<Surface>,
}

/// Resolve a [`BorderStyle`] into the stroke it draws with.
fn border_stroke(style: crate::message::BorderStyle) -> BorderStroke {
    use crate::message::BorderStyle as Bs;
    let (width, dash_period) = match style {
        Bs::Solid | Bs::Double => (1.0, 0.0),
        Bs::Dashed => (1.0, 6.0),
        Bs::Dotted => (1.0, 2.5),
        Bs::Medium => (2.0, 0.0),
        Bs::Thick => (3.0, 0.0),
    };
    BorderStroke {
        width: Px::<Surface>::new(width),
        dash_period: Px::<Surface>::new(dash_period),
    }
}

/// Stroke a straight axis-aligned segment in the given border `style`. Dashed and
/// dotted patterns are drawn as a run of short segments, because [`DrawSurface`]
/// has no dash parameter. Solid, medium and thick are a single stroke. `Double`
/// draws two thin parallel lines straddling the edge.
fn styled_segment<S: DrawSurface>(
    surface: &mut S,
    segment: ambre_draw::Line,
    color: Color,
    style: crate::message::BorderStyle,
) {
    use crate::message::BorderStyle as Bs;
    let (x0, y0, x1, y1) = (segment.p0.x, segment.p0.y, segment.p1.x, segment.p1.y);
    let stroke = border_stroke(style);
    // The surface draws in bare f64: the period unwraps once, here.
    let period = stroke.dash_period.get();
    if style == Bs::Double {
        // Two 1px lines offset by 1px on each side of the edge.
        let (dx, dy) = if (y1 - y0).abs() < f64::EPSILON {
            (0.0, 1.0)
        } else {
            (1.0, 0.0)
        };
        surface.stroke_line(
            ambre_draw::Line::new((x0 - dx, y0 - dy), (x1 - dx, y1 - dy)),
            color,
            ambre_core::Px::<ambre_core::Surface>::new(1.0),
        );
        surface.stroke_line(
            ambre_draw::Line::new((x0 + dx, y0 + dy), (x1 + dx, y1 + dy)),
            color,
            ambre_core::Px::<ambre_core::Surface>::new(1.0),
        );
        return;
    }
    if period <= 0.0 {
        surface.stroke_line(
            ambre_draw::Line::new((x0, y0), (x1, y1)),
            color,
            stroke.width,
        );
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
            ambre_draw::Line::new((x0 + ux * s, y0 + uy * s), (x0 + ux * e, y0 + uy * e)),
            color,
            stroke.width,
        );
        t += period;
    }
}

/// Stroke a cell's set border edges, each in its own style/colour, over `rect`.
/// An edge the user did not colour takes `default_color`, the theme's
/// `cell_border` (`TRAME-101`).
fn draw_cell_border<S: DrawSurface>(
    surface: &mut S,
    rect: Rect,
    content: &CellContent<'_>,
    default_color: Color,
) {
    let b = &content.borders;
    let mut edge = |cb: Option<crate::message::CellBorder>, from: (f64, f64), to: (f64, f64)| {
        if let Some(cb) = cb {
            let color = cb.color.map(rgb).unwrap_or(default_color);
            styled_segment(surface, ambre_draw::Line::new(from, to), color, cb.style);
        }
    };
    edge(b.top, (rect.x0, rect.y0), (rect.x1, rect.y0));
    edge(b.bottom, (rect.x0, rect.y1), (rect.x1, rect.y1));
    edge(b.left, (rect.x0, rect.y0), (rect.x0, rect.y1));
    edge(b.right, (rect.x1, rect.y0), (rect.x1, rect.y1));
}

/// Whether any of the four edges carries a border (drives the border pass).
fn has_border(b: &crate::message::Borders) -> bool {
    b.top.is_some() || b.right.is_some() || b.bottom.is_some() || b.left.is_some()
}

/// Draw the border layer for each supplied `(rect, content)`. Called **after** the
/// grid lines (and the merge repaint), so a deliberate border is never
/// overpainted. `rect` is the owning cell's rect, or the full span rect for a
/// merge master. An edge the user did not colour takes `default_color`, the
/// active theme's `cell_border` (`TRAME-101`).
pub fn draw_cell_borders_on<'a, S, I>(surface: &mut S, cells: I, default_color: Color)
where
    S: DrawSurface,
    I: IntoIterator<Item = (Rect, CellContent<'a>)>,
{
    for (rect, content) in cells {
        if has_border(&content.borders) {
            draw_cell_border(surface, rect, &content, default_color);
        }
    }
}
