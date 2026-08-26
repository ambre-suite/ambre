//! SVG export (`toile-import-export`).
//!
//! Serializes a slice of [`ExportElement`] back to a resolution-independent SVG
//! document -- the inverse of [`super::svg_import`]. Output is **deterministic**
//! ("export is deterministic"): fixed attribute order and a single float
//! formatter, so the same board yields byte-identical SVG. Shapes map to SVG
//! primitives, `Shape(Path)` to `<path d=...>` (reusing `BezPath::to_svg`, the
//! same encoding the CRDT payload stores), text to `<text>`, and an image to an
//! `<image>` referencing its content hash (blob bytes stay on MinIO,).

use std::fmt::Write as _;

use kurbo::Rect;

use super::ExportElement;
use crate::logic::model::{ElementKind, ShapeKind};

/// Serialize `elements` to a standalone SVG document string. The `viewBox` spans
/// the union of every element's bounds (a 1-unit box for an empty input).
pub fn export_svg(elements: &[ExportElement]) -> String {
    let bounds = union_bounds(elements).unwrap_or(Rect::new(0.0, 0.0, 1.0, 1.0));
    // The viewBox is anchored at the origin (0 0 W H), never at the content's
    // min corner: a non-zero viewBox origin makes an SVG reader translate the
    // whole document, which would shift every element on re-import and break the
    // round-trip. W/H span to the content's far corner (min 1 to stay valid).
    let w = bounds.x1.max(1.0);
    let h = bounds.y1.max(1.0);
    let mut out = String::new();
    // Header: fixed attribute order for byte-stability.
    let _ = write!(
        out,
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {} {}\">",
        f(w),
        f(h)
    );
    out.push('\n');
    for el in elements {
        if let Some(frag) = element_svg(el) {
            out.push_str("  ");
            out.push_str(&frag);
            out.push('\n');
        }
    }
    out.push_str("</svg>\n");
    out
}

/// One element's SVG fragment, or `None` for a kind not serialized in Phase 0
/// (a flattened group, an embed).
fn element_svg(el: &ExportElement) -> Option<String> {
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
            let body = xml_escape(el.text.as_deref().unwrap_or(""));
            Some(format!(
                "<text x=\"{}\" y=\"{}\">{body}</text>",
                f(r.x0),
                f(r.y1)
            ))
        }
        ElementKind::Image => {
            let href = el.media_hash.as_deref().unwrap_or("");
            Some(format!(
                "<image x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" href=\"media:{href}\"/>",
                f(r.x0),
                f(r.y0),
                f(r.width()),
                f(r.height())
            ))
        }
        // Groups are flattened on import; sticky/embed/doc_embed/drawing are not
        // part of the Phase 0 SVG round-trip.
        _ => None,
    }
}

/// The `fill`/`stroke`/`stroke-width` attribute string for a styled shape, in a
/// fixed order for byte-stability. Colors come from the ARGB `ElementStyle`.
fn paint_attrs(el: &ExportElement) -> String {
    let mut s = String::new();
    match el.style.fill {
        Some(argb) => {
            let (hex, op) = argb_hex_opacity(argb);
            let _ = write!(s, " fill=\"{hex}\"");
            if op < 1.0 {
                let _ = write!(s, " fill-opacity=\"{}\"", f(f64::from(op)));
            }
        }
        None => s.push_str(" fill=\"none\""),
    }
    if let Some(argb) = el.style.stroke {
        let (hex, op) = argb_hex_opacity(argb);
        let _ = write!(s, " stroke=\"{hex}\"");
        if op < 1.0 {
            let _ = write!(s, " stroke-opacity=\"{}\"", f(f64::from(op)));
        }
        let _ = write!(s, " stroke-width=\"{}\"", f(f64::from(el.style.stroke_w)));
    }
    s
}

/// Split a packed ARGB into a `#rrggbb` string and a `0.0..=1.0` alpha.
fn argb_hex_opacity(argb: u32) -> (String, f32) {
    let a = ((argb >> 24) & 0xFF) as f32 / 255.0;
    let r = (argb >> 16) & 0xFF;
    let g = (argb >> 8) & 0xFF;
    let b = argb & 0xFF;
    (format!("#{r:02x}{g:02x}{b:02x}"), a)
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

    fn rect_el() -> ExportElement {
        let mut el = ExportElement::new(
            ElementKind::Shape(ShapeKind::Rect),
            Rect::new(10.0, 20.0, 110.0, 70.0),
        );
        el.style = ElementStyle {
            fill: Some(0xFF33_66CC),
            stroke: Some(0xFF00_0000),
            stroke_w: 2.0,
            ..ElementStyle::default()
        };
        el
    }

    #[test]
    fn export_is_deterministic() {
        let els = vec![rect_el()];
        assert_eq!(export_svg(&els), export_svg(&els));
    }

    #[test]
    fn rect_serializes_with_fixed_attrs() {
        let svg = export_svg(&[rect_el()]);
        assert!(svg.contains("<rect x=\"10\" y=\"20\" width=\"100\" height=\"50\""));
        assert!(svg.contains("fill=\"#3366cc\""));
        assert!(svg.contains("stroke=\"#000000\""));
        assert!(svg.contains("stroke-width=\"2\""));
    }

    #[test]
    fn text_body_is_escaped() {
        let mut el = ExportElement::new(ElementKind::Text, Rect::new(0.0, 0.0, 50.0, 20.0));
        el.text = Some("a < b & c".to_owned());
        let svg = export_svg(&[el]);
        assert!(svg.contains(">a &lt; b &amp; c</text>"));
    }

    #[test]
    fn float_formatter_trims() {
        assert_eq!(f(10.0), "10");
        assert_eq!(f(10.5), "10.5");
        assert_eq!(f(-0.0), "0");
        assert_eq!(f(1.2345), "1.234");
    }
}
