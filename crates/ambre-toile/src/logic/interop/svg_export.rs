//! SVG export (`toile-import-export`).
//!
//! Serializes a slice of [`ExportElement`] back to a resolution-independent SVG
//! document -- the inverse of [`super::svg_import`]. Output is **deterministic**
//! ("export is deterministic"): fixed attribute order and a single float
//! formatter, so the same board yields byte-identical SVG. Shapes map to SVG
//! primitives and `Shape(Path)` to `<path d=...>`, reusing `BezPath::to_svg`, the
//! same encoding the CRDT payload stores. Text maps to `<text>` with the size
//! and the paint it was drawn with (`TOILE-IO-088`). A picture maps to an
//! `<image>` carrying its pixels inline, so a reader outside Ambre resolves it
//! (`TOILE-IO-087`).
//!
//! A kind the serializer cannot express writes nothing and is named in the
//! report the export returns (`TOILE-IO-025`). Dropping one in silence is a
//! file that reads as a board the user does not have.

use std::fmt::Write as _;

use ambre_core::Rgba;
use base64::Engine as _;
use kurbo::Rect;

use super::ExportElement;
use crate::logic::model::{ElementKind, ShapeKind};

/// One SVG export: the document, and what it could not express
/// (`TOILE-IO-025`).
#[derive(Clone, Default, PartialEq, Eq)]
pub struct SvgExport {
    pub document: String,
    /// One line per element the serializer has no shape for, in board order.
    pub skipped: Vec<String>,
}

/// The document prints as its length alone (`ZK-006`). A skipped line names
/// an element kind, never its content, so it prints whole.
impl std::fmt::Debug for SvgExport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SvgExport")
            .field("document", &crate::logic::ByteLen(&self.document))
            .field("skipped", &self.skipped)
            .finish()
    }
}

/// Serialize `elements` to a standalone SVG document. The `viewBox` spans the
/// union of every element's bounds (a 1-unit box for an empty input).
pub fn export_svg(elements: &[ExportElement]) -> SvgExport {
    let bounds = union_bounds(elements).unwrap_or(Rect::new(0.0, 0.0, 1.0, 1.0));
    // The frame runs from the origin, or from the content's own corner when the
    // board reaches left of or above it. No element then falls outside the
    // frame a reader crops to (`TOILE-IO-086`). A board in the positive
    // quadrant still writes `0 0 W H`. A reader that would normalize a non-zero
    // origin away has nothing to normalize, so a re-import lands where it
    // left.
    let x0 = bounds.x0.min(0.0);
    let y0 = bounds.y0.min(0.0);
    let w = (bounds.x1 - x0).max(1.0);
    let h = (bounds.y1 - y0).max(1.0);
    let mut out = String::new();
    // Header: fixed attribute order for byte-stability.
    let _ = write!(
        out,
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"{} {} {} {}\">",
        f(x0),
        f(y0),
        f(w),
        f(h)
    );
    out.push('\n');
    let mut skipped = Vec::new();
    for el in elements {
        match element_svg(el) {
            Some(frag) => {
                out.push_str("  ");
                out.push_str(&frag);
                out.push('\n');
            }
            None => skipped.push(format!("{}: no SVG shape expresses it", kind_name(el.kind))),
        }
    }
    out.push_str("</svg>\n");
    SvgExport {
        document: out,
        skipped,
    }
}

/// What a skipped element is called in the report. A user reads this, so it
/// names the thing on the board rather than the variant.
fn kind_name(kind: ElementKind) -> &'static str {
    match kind {
        ElementKind::Sticky => "a sticky note",
        ElementKind::Group => "a group",
        ElementKind::Frame => "a frame",
        ElementKind::Embed => "an embed",
        ElementKind::DocEmbed => "an embedded document",
        ElementKind::Drawing => "a freehand stroke",
        _ => "an element",
    }
}

/// One element's SVG fragment, or `None` for a kind no SVG shape expresses.
/// The caller reports what it skipped (`TOILE-IO-025`).
///
/// A rotated element carries a `transform` about its own centre
/// (`TOILE-IO-026`); the board stores the box unrotated, as the render pass
/// reads it.
fn element_svg(el: &ExportElement) -> Option<String> {
    let body = element_shape(el)?;
    Some(match rotation_attr(el) {
        Some(tf) => format!("<g {tf}>{body}</g>"),
        None => body,
    })
}

/// The `transform` that carries an element's rotation, or `None` when it is
/// axis-aligned. Degrees about the rect centre, which is the angle the board
/// stores (`TOILE-IO-026`).
fn rotation_attr(el: &ExportElement) -> Option<String> {
    if !el.rotation.is_finite() || el.rotation == ambre_core::Radians::ZERO {
        return None;
    }
    let c = el.rect.center();
    Some(format!(
        "transform=\"rotate({} {} {})\"",
        f(el.rotation.to_degrees()),
        f(c.x),
        f(c.y)
    ))
}

/// The shape alone, without the rotation wrapper.
fn element_shape(el: &ExportElement) -> Option<String> {
    let r = el.rect;
    match el.kind {
        ElementKind::Shape(ShapeKind::Rect) => Some(format!(
            "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\"{}/>",
            f(r.x0),
            f(r.y0),
            f(r.width()),
            f(r.height()),
            paint_attrs(el)
        )),
        ElementKind::Shape(ShapeKind::Ellipse) => Some(format!(
            "<ellipse cx=\"{}\" cy=\"{}\" rx=\"{}\" ry=\"{}\"{}/>",
            f(r.center().x),
            f(r.center().y),
            f(r.width() / 2.0),
            f(r.height() / 2.0),
            paint_attrs(el)
        )),
        ElementKind::Shape(ShapeKind::Line) => Some(format!(
            "<line x1=\"{}\" y1=\"{}\" x2=\"{}\" y2=\"{}\"{}/>",
            f(r.x0),
            f(r.y0),
            f(r.x1),
            f(r.y1),
            paint_attrs(el)
        )),
        ElementKind::Shape(ShapeKind::Triangle) => {
            // Isoceles triangle inscribed in the AABB (apex top-center).
            let pts = format!(
                "{},{} {},{} {},{}",
                f(r.center().x),
                f(r.y0),
                f(r.x1),
                f(r.y1),
                f(r.x0),
                f(r.y1)
            );
            Some(format!("<polygon points=\"{pts}\"{}/>", paint_attrs(el)))
        }
        ElementKind::Shape(ShapeKind::Path) => {
            let d = el.path.as_ref()?.to_svg();
            Some(format!("<path d=\"{d}\"{}/>", paint_attrs(el)))
        }
        ElementKind::Text => {
            // The size and the ink the board drew it with, so the text comes
            // back as it left rather than in the reader's default
            // (`TOILE-IO-088`). A size the style leaves automatic is the
            // default the board draws a free text at.
            let body = xml_escape(el.text.as_deref().unwrap_or(""));
            let size = f64::from(el.style.font.size.or_default().to_px().get());
            let paint = SvgPaint {
                attribute: PaintAttribute::Fill,
                colour: el.style.text_color.unwrap_or(Rgba::from_bits(0x0000_00FF)),
            };
            let attrs = format!(" font-size=\"{}\"{paint}", f(size));
            Some(format!(
                "<text x=\"{}\" y=\"{}\"{attrs}>{body}</text>",
                f(r.x0),
                f(r.y1)
            ))
        }
        ElementKind::Image => {
            // The pixels ride inline. A content address is Ambre's own name for
            // a blob, and no reader outside it resolves one (`TOILE-IO-087`).
            // What the board holds in plaintext is the thumbnail, so that is
            // the resolution an exported page gets (`TOILE-IO-100`).
            let png = el.image_png.as_ref()?;
            let data = base64::engine::general_purpose::STANDARD.encode(png.as_bytes());
            Some(format!(
                "<image x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" href=\"data:image/png;base64,{data}\"/>",
                f(r.x0),
                f(r.y0),
                f(r.width()),
                f(r.height())
            ))
        }
        // A sticky, a group, a frame, an embed and a freehand stroke have no
        // SVG shape here. The caller names each one in the report.
        _ => None,
    }
}

/// The `fill`/`stroke`/`stroke-width` attribute string for a styled shape, in a
/// fixed order for byte-stability. Colors come from the RGBA `ElementStyle`.
fn paint_attrs(el: &ExportElement) -> String {
    let mut s = String::new();
    match el.style.fill {
        Some(colour) => {
            let paint = SvgPaint {
                attribute: PaintAttribute::Fill,
                colour,
            };
            let _ = write!(s, "{paint}");
        }
        None => s.push_str(" fill=\"none\""),
    }
    if let Some(colour) = el.style.stroke {
        let paint = SvgPaint {
            attribute: PaintAttribute::Stroke,
            colour,
        };
        let _ = write!(s, "{paint}");
        let _ = write!(
            s,
            " stroke-width=\"{}\"",
            f(f64::from(el.style.stroke_w.get()))
        );
    }
    s
}

/// The SVG attribute a colour paints.
#[derive(Clone, Copy)]
enum PaintAttribute {
    Fill,
    Stroke,
}

/// One colour as SVG attributes: the `#rrggbb` paint, then its opacity when
/// the colour is translucent. SVG 1.1 readers take no alpha in the paint.
struct SvgPaint {
    attribute: PaintAttribute,
    colour: Rgba,
}

impl std::fmt::Display for SvgPaint {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let (paint, opacity) = match self.attribute {
            PaintAttribute::Fill => ("fill", "fill-opacity"),
            PaintAttribute::Stroke => ("stroke", "stroke-opacity"),
        };
        write!(out, " {paint}=\"{}\"", self.colour.rgb())?;
        let [.., alpha] = self.colour.channels();
        if alpha < u8::MAX {
            // The ratio is taken in `f32`, so the digits stay those an export
            // has always written.
            write!(
                out,
                " {opacity}=\"{}\"",
                f(f64::from(f32::from(alpha) / 255.0))
            )?;
        }
        Ok(())
    }
}

/// Union AABB of every element's rect, or `None` if the slice is empty.
fn union_bounds(elements: &[ExportElement]) -> Option<Rect> {
    elements
        .iter()
        .map(|e| e.rect)
        .reduce(|acc, r| acc.union(r))
}

/// Deterministic float formatting: up to 3 decimals, trailing zeros and a bare
/// decimal point trimmed, negative zero normalized. Keeps SVG output byte-stable.
fn f(v: f64) -> String {
    if v == 0.0 {
        return "0".to_owned();
    }
    let mut s = format!("{v:.3}");
    if s.contains('.') {
        while s.ends_with('0') {
            s.pop();
        }
        if s.ends_with('.') {
            s.pop();
        }
    }
    if s == "-0" { "0".to_owned() } else { s }
}

/// Minimal XML text escaping for a text body.
fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logic::model::ElementStyle;
    use ambre_bus::app::toile::FontSizeChoice;
    use ambre_core::{Px, StoredBoard};

    fn rect_el() -> ExportElement {
        let mut el = ExportElement::new(
            ElementKind::Shape(ShapeKind::Rect),
            Rect::new(10.0, 20.0, 110.0, 70.0),
        );
        el.style = ElementStyle {
            fill: Some(Rgba::from_bits(0x3366_CCFF)),
            stroke: Some(Rgba::from_bits(0x0000_00FF)),
            stroke_w: Px::<StoredBoard>::new(2.0),
            ..ElementStyle::default()
        };
        el
    }

    /// `TOILE-289`: the debug forms of an export element and of an export print
    /// sizes, none of the body, the outline or the document.
    #[test]
    fn an_export_debug_form_prints_no_content() {
        let mut el = ExportElement::new(ElementKind::Text, Rect::new(0.0, 0.0, 50.0, 20.0));
        el.text = Some("acquire rival".to_owned());
        el.path = Some(kurbo::BezPath::from_svg("M 13 17 L 2468.25 31").expect("a path"));
        let element = format!("{el:?}");
        assert!(!element.contains("rival"), "{element}");
        assert!(!element.contains("2468"), "{element}");
        assert!(element.contains("<13 chars>"), "{element}");
        assert!(element.contains("<2 path elements>"), "{element}");
        let export = export_svg(&[el]);
        let form = format!("{export:?}");
        assert!(!form.contains("rival") && !form.contains("<svg"), "{form}");
        let size = format!("<{} bytes>", export.document.len());
        assert!(form.contains(&size), "{form}");
    }

    #[test]
    fn export_is_deterministic() {
        let els = vec![rect_el()];
        assert_eq!(export_svg(&els), export_svg(&els));
    }

    #[test]
    fn rect_serializes_with_fixed_attrs() {
        let svg = export_svg(&[rect_el()]).document;
        assert!(svg.contains("<rect x=\"10\" y=\"20\" width=\"100\" height=\"50\""));
        assert!(svg.contains("fill=\"#3366cc\""));
        assert!(svg.contains("stroke=\"#000000\""));
        assert!(svg.contains("stroke-width=\"2\""));
    }

    #[test]
    fn text_body_is_escaped() {
        let mut el = ExportElement::new(ElementKind::Text, Rect::new(0.0, 0.0, 50.0, 20.0));
        el.text = Some("a < b & c".to_owned());
        let svg = export_svg(&[el]).document;
        assert!(svg.contains(">a &lt; b &amp; c</text>"));
    }

    #[test]
    fn float_formatter_trims() {
        assert_eq!(f(10.0), "10");
        assert_eq!(f(10.5), "10.5");
        assert_eq!(f(-0.0), "0");
        assert_eq!(f(1.2345), "1.234");
    }

    /// `TOILE-IO-025`: a kind no SVG shape expresses is named, never dropped in
    /// silence. A file that quietly holds less than the board is a file the
    /// user takes away believing it whole.
    #[test]
    fn a_kind_the_writer_cannot_express_is_reported() {
        let els = vec![
            rect_el(),
            ExportElement::new(ElementKind::Sticky, Rect::new(0.0, 0.0, 10.0, 10.0)),
            ExportElement::new(ElementKind::Group, Rect::new(0.0, 0.0, 10.0, 10.0)),
            ExportElement::new(ElementKind::Drawing, Rect::new(0.0, 0.0, 10.0, 10.0)),
        ];
        let export = export_svg(&els);
        assert_eq!(export.skipped.len(), 3, "{:?}", export.skipped);
        assert!(export.skipped[0].contains("sticky"));
        assert!(export.skipped[1].contains("group"));
        assert!(export.skipped[2].contains("stroke"));
        // The rect still left, and nothing it wrote names the skipped kinds.
        assert!(export.document.contains("<rect"));
    }

    /// `TOILE-IO-026`: an exported element carries its rotation. The board
    /// stores an unrotated box and an angle, so a writer that reads only the
    /// box exports every element upright.
    #[test]
    fn a_rotated_element_carries_its_angle() {
        let mut el = rect_el();
        el.rotation = ambre_core::Radians::new(std::f64::consts::FRAC_PI_2);
        let svg = export_svg(&[el]).document;
        assert!(
            svg.contains("transform=\"rotate(90 60 45)\""),
            "the rotation was dropped: {svg}"
        );
        // Unrotated elements write no wrapper at all, so the common board keeps
        // the shape it had.
        assert!(!export_svg(&[rect_el()]).document.contains("rotate("));
    }

    /// `TOILE-IO-086`: an element left of or above the origin is inside the
    /// frame. A board in the positive quadrant still writes `0 0 W H`, so a
    /// reader has no origin to normalize away.
    #[test]
    fn the_frame_holds_an_element_placed_before_the_origin() {
        let mut el = rect_el();
        el.rect = Rect::new(-40.0, -20.0, 60.0, 30.0);
        let svg = export_svg(&[el]).document;
        assert!(
            svg.contains("viewBox=\"-40 -20 100 50\""),
            "the element fell outside the frame: {svg}"
        );
        assert!(
            export_svg(&[rect_el()])
                .document
                .contains("viewBox=\"0 0 110 70\"")
        );
    }

    /// `TOILE-IO-087`: a picture leaves with its pixels. A content address is
    /// Ambre's own name for a blob, and no reader outside it resolves one.
    #[test]
    fn a_picture_leaves_with_its_pixels() {
        let mut el = ExportElement::new(ElementKind::Image, Rect::new(0.0, 0.0, 8.0, 8.0));
        el.media_hash =
            Some(ambre_core::ContentHash::parse(&"b2b0ffee".repeat(8)).expect("a hash"));
        el.image_png = Some(ambre_core::Payload::<ambre_core::ThumbnailPng>::new(vec![
            0x89, 0x50, 0x4E, 0x47,
        ]));
        let export = export_svg(&[el]);
        assert!(
            export
                .document
                .contains("href=\"data:image/png;base64,iVBORw==\""),
            "the pixels did not leave: {}",
            export.document
        );
        assert!(
            !export.document.contains("b2b0ffee"),
            "the file names an address no reader resolves"
        );
    }

    /// `TOILE-IO-087`, the other side: a picture whose pixels this thread does
    /// not hold writes nothing and is reported, rather than leaving an address.
    #[test]
    fn a_picture_without_pixels_is_reported() {
        let mut el = ExportElement::new(ElementKind::Image, Rect::new(0.0, 0.0, 8.0, 8.0));
        el.media_hash =
            Some(ambre_core::ContentHash::parse(&"b2b0ffee".repeat(8)).expect("a hash"));
        let export = export_svg(&[el]);
        assert!(!export.document.contains("<image"));
        assert_eq!(export.skipped.len(), 1, "{:?}", export.skipped);
    }

    /// `TOILE-IO-088`: text carries the size and the ink it was drawn with,
    /// instead of coming back in the default of whatever opens the file.
    #[test]
    fn text_carries_its_size_and_its_ink() {
        let mut el = ExportElement::new(ElementKind::Text, Rect::new(0.0, 0.0, 50.0, 20.0));
        el.text = Some("sized".to_owned());
        el.style.font.size = FontSizeChoice::from_whole_points(Some(24));
        el.style.text_color = Some(Rgba::from_bits(0x1122_33FF));
        let svg = export_svg(&[el]).document;
        assert!(
            svg.contains("font-size=\"32\" fill=\"#112233\""),
            "the text left bare: {svg}"
        );
    }

    /// `TOILE-IO-088`: a text whose size is automatic leaves at the size the
    /// board draws it with, the default 12 points, not at its box height.
    #[test]
    fn auto_sized_text_leaves_at_the_size_it_was_drawn_with() {
        let mut el = ExportElement::new(ElementKind::Text, Rect::new(0.0, 0.0, 50.0, 20.0));
        el.text = Some("auto".to_owned());
        el.style.font.size = FontSizeChoice::AUTO;
        let svg = export_svg(&[el]).document;
        assert!(svg.contains("font-size=\"16\""), "the size drifted: {svg}");
    }
}
