//! Integration tests for the interop converters against golden fixtures
//! (`toile-import-export`). Host tier only: pure functions, no network, no store.
//!
//! Covers each mapped SVG node type -> expected command batch, and an SVG
//! round-trip (import -> export -> import stability). It also covers every
//! safety-limit reject path aborting atomically, embedded raster import, and the
//! report of dropped constructs.

use ambre_core::Rgba;
use ambre_core::WholePx;
use std::io::Cursor;

use image::ImageFormat;
use kurbo::{Point, Rect, Shape, Size};

use super::*;
use crate::logic::command::{InitContent, ToileCommand};
use crate::logic::model::{ElementKind, ShapeKind};

const RECT_SVG: &str = include_str!("fixtures/rect.svg");
const ELLIPSE_SVG: &str = include_str!("fixtures/ellipse.svg");
const PATH_SVG: &str = include_str!("fixtures/path.svg");
const TEXT_SVG: &str = include_str!("fixtures/text.svg");
const GROUP_SVG: &str = include_str!("fixtures/group_nested.svg");

/// Import SVG text, held as the file bytes a drop hands the importer. It
/// shadows the glob import, so each test passes its literal as it is.
fn import_svg(svg: impl AsRef<[u8]>, limits: &Limits) -> Result<ImportBatch, ImportError> {
    super::import_svg(
        &ambre_core::Payload::<ambre_core::FileBytes>::new(svg.as_ref().to_vec()),
        limits,
    )
}

/// Convenience: the single command of a one-element import.
fn only_cmd(batch: &ImportBatch) -> &ToileCommand {
    assert_eq!(batch.commands.len(), 1, "expected exactly one command");
    &batch.commands[0]
}

// -----------------------------------------------------------------------------
// Node-type mapping
// -----------------------------------------------------------------------------

#[test]
fn rect_maps_to_shape_rect() {
    let batch = import_svg(RECT_SVG.as_bytes(), &Limits::default()).unwrap();
    match only_cmd(&batch) {
        ToileCommand::CreateElement {
            kind: ElementKind::Shape(ShapeKind::Rect),
            at,
            size,
            content: InitContent::None,
            ..
        } => {
            assert_eq!(*at, kurbo::Point::new(10.0, 20.0));
            assert_eq!(*size, Size::new(100.0, 50.0));
        }
        other => panic!("expected Shape(Rect), got {other:?}"),
    }
    assert_eq!(batch.report.imported.get(), 1);
}

#[test]
fn ellipse_maps_to_shape_path() {
    // usvg flattens an ellipse to cubic bezier segments -> not an axis-aligned
    // rect, so it keeps exact geometry as Shape(Path).
    let batch = import_svg(ELLIPSE_SVG.as_bytes(), &Limits::default()).unwrap();
    assert!(matches!(
        only_cmd(&batch),
        ToileCommand::CreateElement {
            kind: ElementKind::Shape(ShapeKind::Path),
            content: InitContent::Path(_),
            ..
        }
    ));
}

#[test]
fn path_maps_to_shape_path_with_geometry() {
    let batch = import_svg(PATH_SVG.as_bytes(), &Limits::default()).unwrap();
    match only_cmd(&batch) {
        ToileCommand::CreateElement {
            kind: ElementKind::Shape(ShapeKind::Path),
            content: InitContent::Path(bez),
            ..
        } => assert!(!bez.elements().is_empty(), "path carries geometry"),
        other => panic!("expected Shape(Path), got {other:?}"),
    }
}

#[test]
fn text_maps_to_text_element_preserving_body() {
    let batch = import_svg(TEXT_SVG.as_bytes(), &Limits::default()).unwrap();
    match only_cmd(&batch) {
        ToileCommand::CreateElement {
            kind: ElementKind::Text,
            content: InitContent::Text(body),
            ..
        } => assert_eq!(body, "Hello Toile"),
        other => panic!("expected Text, got {other:?}"),
    }
}

/// `TOILE-IO-027`: the paint order the drawing had. A text node between two
/// shapes lands between them, not after every shape the file carries.
#[test]
fn text_lands_in_the_paint_order_the_drawing_had() {
    const SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="200" height="200">
  <rect x="0" y="0" width="10" height="10" fill="#111111"/>
  <text x="20" y="20">between</text>
  <rect x="40" y="0" width="10" height="10" fill="#222222"/>
</svg>"##;
    let batch = import_svg(SVG.as_bytes(), &Limits::default()).unwrap();
    let kinds: Vec<&ElementKind> = batch
        .commands
        .iter()
        .map(|c| match c {
            ToileCommand::CreateElement { kind, .. } => kind,
            other => panic!("expected a create, got {other:?}"),
        })
        .collect();
    assert_eq!(
        kinds,
        vec![
            &ElementKind::Shape(ShapeKind::Rect),
            &ElementKind::Text,
            &ElementKind::Shape(ShapeKind::Rect),
        ]
    );
}

/// `TOILE-IO-080` and `TOILE-IO-083`: a text node inside a moved group lands
/// under that group's transform, and stays a member of it.
#[test]
fn text_inside_a_group_keeps_its_place_and_its_transform() {
    const SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="400" height="400">
  <g transform="translate(100,200)">
    <rect x="0" y="0" width="10" height="10" fill="#111111"/>
    <text x="0" y="16" font-size="16">moved</text>
  </g>
</svg>"##;
    let batch = import_svg(SVG.as_bytes(), &Limits::default()).unwrap();
    let text_slot = batch
        .commands
        .iter()
        .position(|c| {
            matches!(
                c,
                ToileCommand::CreateElement {
                    kind: ElementKind::Text,
                    ..
                }
            )
        })
        .expect("the text imported");
    match &batch.commands[text_slot] {
        // The group translates by (100,200); the baseline sits one em below the
        // box top, so the top lands exactly on the group's own origin.
        ToileCommand::CreateElement { at, .. } => {
            assert_eq!((at.x, at.y), (100.0, 200.0), "the transform was dropped");
        }
        other => panic!("expected a create, got {other:?}"),
    }
    assert_eq!(batch.groups.len(), 1, "one group for one <g>");
    assert!(
        batch.groups[0]
            .members
            .contains(&ImportRef::Slot(ImportElementSlot::from_usize(text_slot))),
        "the text left its group: {:?}",
        batch.groups[0].members
    );
}

/// `TOILE-IO-079`: a group the board cannot rebuild is reported. `GroupElements`
/// refuses a group of one, so the member keeps its geometry and the structure
/// the drawing had is named as lost.
#[test]
fn a_group_of_one_member_is_reported_rather_than_dropped() {
    const SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="200" height="200">
  <g transform="translate(5,5)">
    <rect x="0" y="0" width="10" height="10" fill="#111111"/>
  </g>
</svg>"##;
    let batch = import_svg(SVG.as_bytes(), &Limits::default()).unwrap();
    assert_eq!(batch.commands.len(), 1, "the member still imports");
    assert!(batch.groups.is_empty(), "no group the board would refuse");
    assert!(
        batch
            .report
            .skipped
            .iter()
            .any(|s| s.as_str().contains("group")),
        "the lost structure was not reported: {:?}",
        batch.report.skipped
    );
}

/// `TOILE-IO-081`: text the reader cannot read is named in the report. The
/// shapes still import, and a silent import would be a drawing missing its
/// words with nothing to read.
#[test]
fn text_the_reader_cannot_parse_is_reported() {
    // The body sits in a construct this pass does not follow, so the reader
    // finds no string to import.
    const SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="200" height="200">
  <rect x="0" y="0" width="10" height="10" fill="#111111"/>
  <text x="20" y="20"><!-- the words live in a comment --></text>
</svg>"##;
    let batch = import_svg(SVG.as_bytes(), &Limits::default()).unwrap();
    assert_eq!(batch.commands.len(), 1, "the shape still imports");
    assert!(
        batch
            .report
            .skipped
            .iter()
            .any(|s| s.as_str().contains("text")),
        "the lost text was not reported: {:?}",
        batch.report.skipped
    );
}

#[test]
fn nested_groups_are_rebuilt_innermost_first() {
    let batch = import_svg(GROUP_SVG.as_bytes(), &Limits::default()).unwrap();
    // Three rects are created; groups are rebuilt post-apply, not flattened.
    assert_eq!(batch.commands.len(), 3);
    assert!(batch.commands.iter().all(|c| matches!(
        c,
        ToileCommand::CreateElement {
            kind: ElementKind::Shape(ShapeKind::Rect),
            ..
        }
    )));
    // No group is reported as lost.
    assert!(
        !batch
            .report
            .skipped
            .iter()
            .any(|s| s.as_str().contains("flatten")),
        "no group should be flattened: {:?}",
        batch.report.skipped
    );
    // Two groups: the inner group first (post-order), the outer group second.
    assert_eq!(batch.groups.len(), 2, "one ImportGroup per <g>");
    // groups[0] is the innermost <g> (its two rect slots).
    assert_eq!(
        batch.groups[0].members,
        vec![
            ImportRef::Slot(ImportElementSlot::new(1)),
            ImportRef::Slot(ImportElementSlot::new(2))
        ]
    );
    // groups[1] is the outer <g>: a rect slot plus the already-resolved inner group.
    assert_eq!(
        batch.groups[1].members,
        vec![
            ImportRef::Slot(ImportElementSlot::ZERO),
            ImportRef::Group(ImportGroupSlot::ZERO)
        ]
    );
    // Transforms are still baked into geometry: inner rect at (5+50, 5+50)=(55,55).
    let ats: Vec<_> = batch
        .commands
        .iter()
        .filter_map(|c| match c {
            ToileCommand::CreateElement { at, .. } => Some((at.x, at.y)),
            _ => None,
        })
        .collect();
    assert!(ats.contains(&(5.0, 5.0)), "outer rect at (5,5): {ats:?}");
    assert!(
        ats.contains(&(55.0, 55.0)),
        "inner rect at (55,55): {ats:?}"
    );
}

#[test]
fn rect_import_preserves_fill_and_stroke_style() {
    let batch = import_svg(RECT_SVG.as_bytes(), &Limits::default()).unwrap();
    // Style rides an index-aligned `styles` slot, not the CreateElement command.
    assert_eq!(batch.styles.len(), batch.commands.len());
    let style = batch.styles[0]
        .as_ref()
        .expect("rect carries an imported style");
    // fill="#3366cc" -> opaque RGBA; stroke="#000000" width 2.
    assert_eq!(style.fill, Some(Rgba::from_bits(0x3366_CCFF)));
    assert_eq!(style.stroke, Some(Rgba::from_bits(0x0000_00FF)));
    assert_eq!(style.stroke_w, ambre_core::StoredBoardLength::new(2.0));
}

#[test]
fn unstyled_kinds_keep_the_default_style() {
    // Text imports with no style slot (None keeps the engine default).
    let batch = import_svg(TEXT_SVG.as_bytes(), &Limits::default()).unwrap();
    assert_eq!(batch.styles.len(), batch.commands.len());
    assert!(batch.styles.iter().all(|s| s.is_none()));
}

// -----------------------------------------------------------------------------
// Safety limits -- each reject path aborts atomically (no commands)
// -----------------------------------------------------------------------------

#[test]
fn too_many_elements_aborts_atomically() {
    let mut svg = String::from("<svg xmlns=\"http://www.w3.org/2000/svg\">");
    for i in 0..10 {
        svg.push_str(&format!(
            "<rect x=\"{i}\" y=\"0\" width=\"5\" height=\"5\"/>"
        ));
    }
    svg.push_str("</svg>");
    let limits = Limits {
        max_elements: ambre_core::ElementCount::new(4),
        ..Limits::default()
    };
    let err = import_svg(svg.as_bytes(), &limits).unwrap_err();
    assert_eq!(
        err,
        ImportError::TooManyElements {
            limit: ambre_core::ElementCount::new(4)
        }
    );
}

#[test]
fn too_deep_nesting_aborts_atomically() {
    // Six nested groups, each with a rect; a depth cap of 3 rejects the file.
    let mut svg = String::from("<svg xmlns=\"http://www.w3.org/2000/svg\">");
    for _ in 0..6 {
        svg.push_str("<g>");
    }
    svg.push_str("<rect x=\"0\" y=\"0\" width=\"5\" height=\"5\"/>");
    for _ in 0..6 {
        svg.push_str("</g>");
    }
    svg.push_str("</svg>");
    let limits = Limits {
        max_depth: super::ImportDepth::new(3),
        ..Limits::default()
    };
    let err = import_svg(svg.as_bytes(), &limits).unwrap_err();
    assert_eq!(
        err,
        ImportError::TooDeep {
            limit: super::ImportDepth::new(3)
        }
    );
}

#[test]
fn coordinates_are_clamped_to_the_world_bound() {
    let svg = "<svg xmlns=\"http://www.w3.org/2000/svg\">\
        <rect x=\"0\" y=\"0\" width=\"100000\" height=\"100000\"/></svg>";
    let limits = Limits {
        coord_clamp: super::limits::CoordBound::new(ambre_core::BoardLength::new(500.0))
            .expect("a valid bound"),
        ..Limits::default()
    };
    let batch = import_svg(svg.as_bytes(), &limits).unwrap();
    match only_cmd(&batch) {
        ToileCommand::CreateElement { at, size, .. } => {
            assert!(at.x.abs() <= 500.0 && at.y.abs() <= 500.0);
            assert!(size.width <= 1000.0 && size.height <= 1000.0);
        }
        _ => panic!("the import produced no CreateElement"),
    }
}

#[test]
fn empty_svg_reports_empty() {
    let svg = "<svg xmlns=\"http://www.w3.org/2000/svg\"></svg>";
    assert_eq!(
        import_svg(svg.as_bytes(), &Limits::default()).unwrap_err(),
        ImportError::Empty
    );
}

#[test]
fn malformed_svg_fails_to_parse() {
    let err = import_svg(b"not an svg at all", &Limits::default()).unwrap_err();
    assert!(matches!(err, ImportError::Parse(_)));
}

// -----------------------------------------------------------------------------
// Embedded raster in SVG -> Image element + MediaBlob
// -----------------------------------------------------------------------------

#[test]
fn embedded_png_maps_to_image_and_media_blob() {
    let svg = svg_with_embedded_png(32, 24);
    let batch = import_svg(svg.as_bytes(), &Limits::default()).unwrap();
    assert_eq!(batch.media.len(), 1, "one media blob produced");
    assert_eq!(
        batch.media[0].size,
        ambre_core::ImageSize::new(WholePx::new(32), WholePx::new(24))
    );
    assert!(!batch.media[0].thumb.is_empty());
    assert!(batch.commands.iter().any(|c| matches!(
        c,
        ToileCommand::CreateElement {
            kind: ElementKind::Image,
            content: InitContent::Image(_),
            ..
        }
    )));
}

// -----------------------------------------------------------------------------
// SVG round-trip stability ("deterministic export")
// -----------------------------------------------------------------------------

#[test]
fn round_trip_rect_is_stable() {
    // Build an export model, serialize, re-import, and assert the kind + geometry
    // survive the round-trip.
    let mut el = ExportElement::new(
        ElementKind::Shape(ShapeKind::Rect),
        Rect::new(10.0, 20.0, 110.0, 70.0),
    );
    el.style.fill = Some(Rgba::from_bits(0x3366_CCFF));

    let svg1 = export_svg(std::slice::from_ref(&el)).document;
    let batch = import_svg(svg1.as_bytes(), &Limits::default()).unwrap();
    match only_cmd(&batch) {
        ToileCommand::CreateElement {
            kind: ElementKind::Shape(ShapeKind::Rect),
            at,
            size,
            ..
        } => {
            assert_eq!(*at, kurbo::Point::new(10.0, 20.0));
            assert_eq!(*size, Size::new(100.0, 50.0));
        }
        other => panic!("rect did not round-trip: {other:?}"),
    }

    // Export is byte-stable across runs.
    let svg2 = export_svg(std::slice::from_ref(&el)).document;
    assert_eq!(svg1, svg2);
}

#[test]
fn round_trip_path_geometry_survives() {
    let mut bez = kurbo::BezPath::new();
    bez.move_to((0.0, 0.0));
    bez.curve_to((10.0, 30.0), (40.0, 30.0), (50.0, 0.0));
    let mut el = ExportElement::new(ElementKind::Shape(ShapeKind::Path), bez.bounding_box());
    el.path = Some(bez);

    let svg = export_svg(std::slice::from_ref(&el)).document;
    let batch = import_svg(svg.as_bytes(), &Limits::default()).unwrap();
    assert!(matches!(
        only_cmd(&batch),
        ToileCommand::CreateElement {
            kind: ElementKind::Shape(ShapeKind::Path),
            content: InitContent::Path(_),
            ..
        }
    ));
}

// -----------------------------------------------------------------------------
// Helpers
// -----------------------------------------------------------------------------

/// Build an SVG string embedding a solid PNG as a base64 data URI.
fn svg_with_embedded_png(w: u32, h: u32) -> String {
    let img = image::RgbaImage::from_pixel(w, h, image::Rgba([200, 100, 50, 255]));
    let mut png = Vec::new();
    image::DynamicImage::ImageRgba8(img)
        .write_to(&mut Cursor::new(&mut png), ImageFormat::Png)
        .unwrap();
    let b64 = base64_encode(&png);
    format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"200\" height=\"200\">\
         <image x=\"0\" y=\"0\" width=\"{w}\" height=\"{h}\" \
         href=\"data:image/png;base64,{b64}\"/></svg>"
    )
}

/// Minimal standard base64 encoder (no external dep in the test).
fn base64_encode(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in data.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        out.push(T[((n >> 18) & 63) as usize] as char);
        out.push(T[((n >> 12) & 63) as usize] as char);
        out.push(if chunk.len() > 1 {
            T[((n >> 6) & 63) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            T[(n & 63) as usize] as char
        } else {
            '='
        });
    }
    out
}

// -- The drop point (`TOILE-IO-023`) ------------------------------------------

/// The batch moves as a whole: its top-left corner lands on the drop point and
/// the drawing keeps its own internal geometry.
#[test]
fn a_batch_lands_with_its_corner_on_the_drop_point() {
    let mut batch = import_svg(RECT_SVG.as_bytes(), &Limits::default()).expect("rect.svg maps");
    let before: Vec<Point> = create_points(&batch);
    assert!(
        !before.is_empty(),
        "the fixture creates at least one element"
    );
    let corner = before
        .iter()
        .fold(before[0], |a, p| Point::new(a.x.min(p.x), a.y.min(p.y)));

    place_batch_at(&mut batch, Point::new(500.0, 400.0));

    let after = create_points(&batch);
    let delta = Point::new(500.0, 400.0) - corner;
    for (b, a) in before.iter().zip(&after) {
        assert_eq!(*a, *b + delta, "every slot moves by the same offset");
    }
    let new_corner = after
        .iter()
        .fold(after[0], |a, p| Point::new(a.x.min(p.x), a.y.min(p.y)));
    assert_eq!(new_corner, Point::new(500.0, 400.0));
}

/// A batch that creates nothing has no corner to place, and placing it writes
/// nothing rather than inventing an origin.
#[test]
fn placing_a_batch_that_creates_nothing_is_a_no_op() {
    let mut batch = ImportBatch::default();
    place_batch_at(&mut batch, Point::new(10.0, 10.0));
    assert!(batch.commands.is_empty());
}

/// The `at` of every `CreateElement` the batch carries, in slot order.
fn create_points(batch: &ImportBatch) -> Vec<Point> {
    batch
        .commands
        .iter()
        .filter_map(|c| match c {
            ToileCommand::CreateElement { at, .. } => Some(*at),
            _ => None,
        })
        .collect()
}
