//! The reader against the writer of this crate, and against hostile
//! packages a test assembles in memory.

use ambre_core::index::Pos;
use ambre_core::{
    Payload, PrintLength, Pt, Radians, Rgba, Slant, Strikethrough, Underline, Weight,
};

use crate::PptxError;
use crate::model::{
    Attachment, Cap, Connector, ConnectorEnd, Deck, Geometry, Item, Line, PathSegment, Picture,
    PictureFormat, Placement, Route, Shape, Site, Slide, SlideItems, SlidePoint, SlideSize,
    TextAlign, TextAnchor, TextBody,
};
use crate::opc::{self, PackageWriter, PartName, RelKind, RelTarget};
use crate::report::{DeckConstruct, DeckOutcome};

/// The smallest PNG file: one transparent pixel.
pub(in crate::read) const PIXEL_PNG: &[u8] = &[
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1F, 0x15, 0xC4,
    0x89, 0x00, 0x00, 0x00, 0x0B, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9C, 0x63, 0x60, 0x00, 0x02, 0x00,
    0x00, 0x05, 0x00, 0x01, 0x7A, 0x5E, 0xAB, 0x3F, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44,
    0xAE, 0x42, 0x60, 0x82,
];

/// The first bytes of a JPEG file: the reader names it by them alone.
pub(in crate::read) const JPEG_HEAD: &[u8] =
    &[0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10, b'J', b'F', b'I', b'F'];

fn pt(value: f32) -> PrintLength<Pt> {
    PrintLength::<Pt>::new(value)
}

fn placed(x: f32, y: f32, w: f32, h: f32) -> Placement {
    Placement {
        origin: SlidePoint { x: pt(x), y: pt(y) },
        width: pt(w),
        height: pt(h),
        rotation: Radians::ZERO,
    }
}

fn body(text: &str) -> TextBody {
    TextBody {
        text: text.to_owned(),
        typeface: "Georgia".to_owned(),
        size: pt(20.0),
        colour: Rgba::from_bits(0x1122_33FF),
        weight: Weight::Bold,
        slant: Slant::Italic,
        underline: Underline::Underlined,
        strikethrough: Strikethrough::Plain,
        anchor: TextAnchor::Middle,
        align: TextAlign::Centre,
        inset: pt(7.2),
    }
}

/// A deck of every item the writer writes, on two slides.
pub(in crate::read) fn sample() -> Deck {
    let square = Item::Shape(Shape {
        name: "Square".to_owned(),
        at: placed(10.0, 20.0, 100.0, 50.0),
        geometry: Geometry::Rect,
        fill: Some(Rgba::from_bits(0xFF00_00FF)),
        line: Some(Line {
            colour: Rgba::from_bits(0x0000_00FF),
            width: pt(2.0),
        }),
        text: Some(body("Inside\nTwo lines")),
    });
    let disc = Item::Shape(Shape {
        name: "Disc".to_owned(),
        at: Placement {
            rotation: Radians::new(std::f64::consts::FRAC_PI_2),
            ..placed(300.0, 20.0, 80.0, 80.0)
        },
        geometry: Geometry::Ellipse,
        fill: Some(Rgba::from_bits(0x00FF_00FF)),
        line: None,
        text: None,
    });
    let link = Item::Connector(Connector {
        name: "Link".to_owned(),
        route: Route::Straight,
        start: ConnectorEnd {
            at: SlidePoint {
                x: pt(110.0),
                y: pt(45.0),
            },
            attached: Some(Attachment {
                item: Pos::<SlideItems>::from_usize(0),
                site: Site::Right,
            }),
        },
        end: ConnectorEnd {
            at: SlidePoint {
                x: pt(300.0),
                y: pt(60.0),
            },
            attached: Some(Attachment {
                item: Pos::<SlideItems>::from_usize(1),
                site: Site::Left,
            }),
        },
        line: Line {
            colour: Rgba::from_bits(0x3333_33FF),
            width: pt(1.0),
        },
        start_cap: Cap::Plain,
        end_cap: Cap::Arrow,
    });
    let path = Item::Shape(Shape {
        name: "Path".to_owned(),
        at: placed(10.0, 200.0, 100.0, 100.0),
        geometry: Geometry::Freeform(vec![
            PathSegment::MoveTo(SlidePoint {
                x: pt(10.0),
                y: pt(200.0),
            }),
            PathSegment::LineTo(SlidePoint {
                x: pt(110.0),
                y: pt(300.0),
            }),
            PathSegment::Close,
        ]),
        fill: None,
        line: Some(Line {
            colour: Rgba::from_bits(0x0000_FFFF),
            width: pt(1.0),
        }),
        text: None,
    });
    let png = Item::Picture(Picture {
        name: "Png".to_owned(),
        at: placed(0.0, 0.0, 10.0, 10.0),
        format: PictureFormat::Png,
        bytes: Payload::new(PIXEL_PNG.to_vec()),
    });
    let jpeg = Item::Picture(Picture {
        name: "Jpeg".to_owned(),
        at: placed(20.0, 0.0, 10.0, 10.0),
        format: PictureFormat::Jpeg,
        bytes: Payload::new(JPEG_HEAD.to_vec()),
    });
    Deck {
        size: SlideSize::WIDESCREEN,
        slides: vec![
            Slide {
                name: "First".to_owned(),
                items: vec![square, disc, link, path],
                notes: "Say this.\nThen that.".to_owned(),
                background: Some(Rgba::from_bits(0xEEEE_DDFF)),
            },
            Slide {
                name: String::new(),
                items: vec![png, jpeg],
                notes: String::new(),
                background: None,
            },
        ],
    }
}

/// The deck the writer's file of `deck` reads back into.
fn round_trip(deck: &Deck) -> Deck {
    let file = crate::write(deck).expect("the sample writes");
    crate::read(file.as_bytes()).expect("the file reads").deck
}

/// The writer and the reader are two paths over one model: what one writes
/// the other reads back, item by item.
#[test]
fn a_written_deck_reads_back_as_it_left() {
    let deck = sample();
    let back = round_trip(&deck);
    assert_eq!(back.size, deck.size);
    assert_eq!(back.slides.len(), 2);
    for (left, came) in deck.slides.iter().zip(&back.slides) {
        assert_eq!(came.name, left.name);
        assert_eq!(came.notes, left.notes);
        assert_eq!(came.items, left.items);
    }
    assert_eq!(
        back.slides[0].background,
        Some(Rgba::from_bits(0xEEEE_DDFF))
    );
}

#[test]
fn bytes_that_are_no_package_are_refused() {
    assert!(matches!(
        crate::read(b"not a zip"),
        Err(PptxError::Package(_))
    ));
}

/// A package of the parts `parts` and a presentation relationship to the
/// first.
fn package(parts: &[(&str, &str, String)]) -> Vec<u8> {
    let mut w = PackageWriter::new();
    for (index, (name, content_type, xml)) in parts.iter().enumerate() {
        let part = PartName::new(name).expect("a part name");
        w.part(
            part.clone(),
            Some(content_type),
            Payload::new(xml.as_bytes().to_vec()),
        );
        if index == 0 {
            w.relate(
                &opc::root(),
                RelKind::OFFICE_DOCUMENT,
                RelTarget::Part(part),
            );
        }
    }
    w.finish().expect("the package writes").as_bytes().to_vec()
}

const PRESENTATION: &str =
    "application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml";

#[test]
fn a_package_of_another_document_is_refused() {
    let word = package(&[(
        "/word/document.xml",
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml",
        "<w:document xmlns:w=\"urn:w\"/>".to_owned(),
    )]);
    assert_eq!(crate::read(&word), Err(PptxError::NoPresentation));
}

#[test]
fn a_presentation_of_no_slide_is_refused() {
    let empty = package(&[(
        "/ppt/presentation.xml",
        PRESENTATION,
        "<p:presentation xmlns:p=\"http://schemas.openxmlformats.org/presentationml/2006/main\"/>"
            .to_owned(),
    )]);
    assert_eq!(crate::read(&empty), Err(PptxError::NoSlide));
}

/// An entity a part declares could expand past any bound, so a document
/// type is refused whole.
#[test]
fn a_part_that_declares_a_document_type_is_refused() {
    let hostile = package(&[(
        "/ppt/presentation.xml",
        PRESENTATION,
        "<!DOCTYPE p [<!ENTITY a \"aaaa\">]><p:presentation \
         xmlns:p=\"http://schemas.openxmlformats.org/presentationml/2006/main\"/>"
            .to_owned(),
    )]);
    assert_eq!(crate::read(&hostile), Err(PptxError::Unreadable));
}

#[test]
fn a_part_nested_past_the_bound_is_refused() {
    let deep = format!(
        "<p:presentation xmlns:p=\"http://schemas.openxmlformats.org/presentationml/2006/main\">{}{}</p:presentation>",
        "<p:x>".repeat(super::xml::MAX_DEPTH + 1),
        "</p:x>".repeat(super::xml::MAX_DEPTH + 1)
    );
    let hostile = package(&[("/ppt/presentation.xml", PRESENTATION, deep)]);
    assert_eq!(crate::read(&hostile), Err(PptxError::Unreadable));
}

/// A malformed part of each shape a parser meets reads to an error, never
/// to a panic.
#[test]
fn malformed_parts_read_to_an_error() {
    for xml in [
        "",
        "<p:presentation",
        "<p:presentation xmlns:p=\"x\"></p:other>",
        "<a>&undefined;</a>",
        "<p:presentation xmlns:p=\"http://schemas.openxmlformats.org/presentationml/2006/main\"><p:sldIdLst><p:sldId r:id=\"rId9\"/></p:sldIdLst></p:presentation>",
    ] {
        let file = package(&[("/ppt/presentation.xml", PRESENTATION, xml.to_owned())]);
        assert!(crate::read(&file).is_err(), "{xml}");
    }
}

/// A master drawn on every slide multiplies its shapes, so the bound is
/// read on the flattened deck, never on one part.
#[test]
fn a_master_that_flattens_past_the_item_bound_is_refused() {
    const SHAPES: usize = 25_001;
    let shape = "<p:sp><p:nvSpPr><p:cNvPr id=\"2\" name=\"s\"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr>\
                 <p:spPr><a:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"1\" cy=\"1\"/></a:xfrm></p:spPr></p:sp>";
    let ns = "xmlns:p=\"http://schemas.openxmlformats.org/presentationml/2006/main\" \
              xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\" \
              xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\"";
    let master = format!(
        "<p:sldMaster {ns}><p:cSld><p:spTree>{}</p:spTree></p:cSld></p:sldMaster>",
        shape.repeat(SHAPES)
    );
    let mut w = PackageWriter::new();
    let presentation = PartName::new("/ppt/presentation.xml").expect("a name");
    let master_part = PartName::new("/ppt/slideMasters/slideMaster1.xml").expect("a name");
    let layout = PartName::new("/ppt/slideLayouts/slideLayout1.xml").expect("a name");
    w.relate(
        &opc::root(),
        RelKind::OFFICE_DOCUMENT,
        RelTarget::Part(presentation.clone()),
    );
    let mut ids = String::new();
    for n in 1..=4 {
        let slide = PartName::new(&format!("/ppt/slides/slide{n}.xml")).expect("a name");
        let rid = w.relate(
            &presentation,
            RelKind::new(
                "http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide",
            ),
            RelTarget::Part(slide.clone()),
        );
        ids.push_str(&format!(
            "<p:sldId id=\"{}\" r:id=\"{}\"/>",
            255 + n,
            rid.as_str()
        ));
        w.relate(
            &slide,
            RelKind::new(
                "http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideLayout",
            ),
            RelTarget::Part(layout.clone()),
        );
        w.part(
            slide,
            None,
            Payload::new(format!("<p:sld {ns}><p:cSld><p:spTree/></p:cSld></p:sld>").into_bytes()),
        );
    }
    w.relate(
        &layout,
        RelKind::new(
            "http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideMaster",
        ),
        RelTarget::Part(master_part.clone()),
    );
    w.part(
        layout,
        None,
        Payload::new(
            format!("<p:sldLayout {ns}><p:cSld><p:spTree/></p:cSld></p:sldLayout>").into_bytes(),
        ),
    );
    w.part(master_part, None, Payload::new(master.into_bytes()));
    w.part(
        presentation,
        Some(PRESENTATION),
        Payload::new(
            format!("<p:presentation {ns}><p:sldIdLst>{ids}</p:sldIdLst></p:presentation>")
                .into_bytes(),
        ),
    );
    let file = w.finish().expect("the package writes");
    assert_eq!(crate::read(file.as_bytes()), Err(PptxError::TooLarge));
}

/// A picture in an encoding the deck does not hold is named, never kept
/// under a format it is not.
#[test]
fn a_picture_of_another_encoding_is_named_as_dropped() {
    let ns = "xmlns:p=\"http://schemas.openxmlformats.org/presentationml/2006/main\" \
              xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\" \
              xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\"";
    let rel = |name: &str| {
        RelKind::new(match name {
            "slide" => "http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide",
            _ => "http://schemas.openxmlformats.org/officeDocument/2006/relationships/image",
        })
    };
    let mut w = PackageWriter::new();
    let presentation = PartName::new("/ppt/presentation.xml").expect("a name");
    let slide = PartName::new("/ppt/slides/slide1.xml").expect("a name");
    let gif = PartName::new("/ppt/media/image1.gif").expect("a name");
    w.relate(
        &opc::root(),
        RelKind::OFFICE_DOCUMENT,
        RelTarget::Part(presentation.clone()),
    );
    let sid = w.relate(&presentation, rel("slide"), RelTarget::Part(slide.clone()));
    let pid = w.relate(&slide, rel("image"), RelTarget::Part(gif.clone()));
    w.part(gif, Some("image/gif"), Payload::new(b"GIF89a....".to_vec()));
    let pic = format!(
        "<p:pic><p:nvPicPr><p:cNvPr id=\"2\" name=\"Gif\"/><p:cNvPicPr/><p:nvPr/></p:nvPicPr>\
         <p:blipFill><a:blip r:embed=\"{}\"/></p:blipFill><p:spPr><a:xfrm><a:off x=\"0\" y=\"0\"/>\
         <a:ext cx=\"12700\" cy=\"12700\"/></a:xfrm></p:spPr></p:pic>",
        pid.as_str()
    );
    w.part(
        slide,
        None,
        Payload::new(
            format!("<p:sld {ns}><p:cSld><p:spTree>{pic}</p:spTree></p:cSld></p:sld>").into_bytes(),
        ),
    );
    w.part(presentation, Some(PRESENTATION), Payload::new(format!("<p:presentation {ns}><p:sldIdLst><p:sldId id=\"256\" r:id=\"{}\"/></p:sldIdLst></p:presentation>", sid.as_str()).into_bytes()));
    let file = w.finish().expect("the package writes");
    let read = crate::read(file.as_bytes()).expect("the file reads");
    assert!(read.deck.slides.first().is_some_and(|s| s.items.is_empty()));
    assert_eq!(
        read.report
            .count(DeckConstruct::UnreadPicture, DeckOutcome::Dropped)
            .get(),
        1
    );
}

/// A file of one slide whose shape tree holds `tree`, and the parts
/// `extra` beside it, each related from the slide under its id.
fn one_slide(tree: &str, extra: &[(&str, &'static str, &str, String)]) -> Vec<u8> {
    let ns = "xmlns:p=\"http://schemas.openxmlformats.org/presentationml/2006/main\" \
              xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\" \
              xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\"";
    let mut w = PackageWriter::new();
    let presentation = PartName::new("/ppt/presentation.xml").expect("a name");
    let slide = PartName::new("/ppt/slides/slide1.xml").expect("a name");
    w.relate(
        &opc::root(),
        RelKind::OFFICE_DOCUMENT,
        RelTarget::Part(presentation.clone()),
    );
    let sid = w.relate(
        &presentation,
        RelKind::new("http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide"),
        RelTarget::Part(slide.clone()),
    );
    let mut tree = tree.to_owned();
    for (name, kind, placeholder, xml) in extra {
        let part = PartName::new(name).expect("a name");
        let id = w.relate(&slide, RelKind::new(kind), RelTarget::Part(part.clone()));
        tree = tree.replace(placeholder, id.as_str());
        w.part(part, None, Payload::new(xml.as_bytes().to_vec()));
    }
    w.part(
        slide,
        None,
        Payload::new(
            format!("<p:sld {ns}><p:cSld><p:spTree>{tree}</p:spTree></p:cSld></p:sld>")
                .into_bytes(),
        ),
    );
    w.part(
        presentation,
        Some(PRESENTATION),
        Payload::new(
            format!(
                "<p:presentation {ns}><p:sldIdLst><p:sldId id=\"256\" r:id=\"{}\"/></p:sldIdLst></p:presentation>",
                sid.as_str()
            )
            .into_bytes(),
        ),
    );
    w.finish().expect("the package writes").as_bytes().to_vec()
}

/// The shapes of the first slide of `file`.
fn shapes_of(file: &[u8]) -> (Vec<Shape>, crate::report::DeckReport) {
    let read = crate::read(file).expect("the file reads");
    let shapes = read
        .deck
        .slides
        .first()
        .map(|s| {
            s.items
                .iter()
                .filter_map(|item| match item {
                    Item::Shape(shape) => Some(shape.clone()),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default();
    (shapes, read.report)
}

/// A graphic frame at 1 inch, 2 by 1 inch, around `data`.
fn graphic_frame(uri: &str, data: &str) -> String {
    format!(
        "<p:graphicFrame><p:nvGraphicFramePr><p:cNvPr id=\"5\" name=\"Grid\"/>\
         <p:cNvGraphicFramePr/><p:nvPr/></p:nvGraphicFramePr><p:xfrm>\
         <a:off x=\"914400\" y=\"914400\"/><a:ext cx=\"1828800\" cy=\"914400\"/></p:xfrm>\
         <a:graphic><a:graphicData uri=\"{uri}\">{data}</a:graphicData></a:graphic>\
         </p:graphicFrame>"
    )
}

const TABLE_URI: &str = "http://schemas.openxmlformats.org/drawingml/2006/table";
const CHART_URI: &str = "http://schemas.openxmlformats.org/drawingml/2006/chart";
const CHART_REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/chart";

/// A cell of text `text`, with the attributes `attrs` and the cell
/// properties `props`.
fn cell(attrs: &str, text: &str, props: &str) -> String {
    format!(
        "<a:tc {attrs}><a:txBody><a:bodyPr/><a:p><a:r><a:t>{text}</a:t></a:r></a:p></a:txBody>\
         <a:tcPr {props}</a:tc>"
    )
}

/// Each cell lands as a shape of its own at its place in the grid, with its
/// text, its fill and its border; a merged cell spans what it names, and
/// the cells it covers land nothing.
#[test]
fn a_table_lands_as_one_shape_per_cell_in_its_grid() {
    let rows = format!(
        "<a:tr h=\"457200\">{}{}</a:tr><a:tr h=\"457200\">{}{}</a:tr>",
        cell(
            "gridSpan=\"2\"",
            "Head",
            "anchor=\"ctr\" marL=\"25400\"><a:lnB w=\"25400\"><a:solidFill>\
             <a:srgbClr val=\"FF0000\"/></a:solidFill></a:lnB><a:solidFill>\
             <a:srgbClr val=\"0000FF\"/></a:solidFill></a:tcPr>"
        ),
        cell("hMerge=\"1\"", "", "/>"),
        cell("", "Left", "/>"),
        cell("", "Right", "/>"),
    );
    let table = format!(
        "<a:tbl><a:tblGrid><a:gridCol w=\"914400\"/><a:gridCol w=\"914400\"/></a:tblGrid>{rows}</a:tbl>"
    );
    let file = one_slide(&graphic_frame(TABLE_URI, &table), &[]);
    let (shapes, report) = shapes_of(&file);
    assert_eq!(shapes.len(), 3, "{shapes:?}");
    let text = |s: &Shape| s.text.as_ref().map(|t| t.text.clone()).unwrap_or_default();
    let head = &shapes[0];
    assert_eq!(text(head), "Head");
    assert_eq!(head.at, placed(72.0, 72.0, 144.0, 36.0));
    assert_eq!(head.fill, Some(Rgba::from_bits(0x0000_FFFF)));
    assert_eq!(
        head.line,
        Some(Line {
            colour: Rgba::from_bits(0xFF00_00FF),
            width: pt(2.0)
        })
    );
    let body = head.text.as_ref().expect("the head holds text");
    assert_eq!(body.anchor, TextAnchor::Middle);
    assert_eq!(body.inset, pt(2.0));
    assert_eq!(text(&shapes[1]), "Left");
    assert_eq!(shapes[1].at, placed(72.0, 108.0, 72.0, 36.0));
    assert_eq!(text(&shapes[2]), "Right");
    assert_eq!(shapes[2].at, placed(144.0, 108.0, 72.0, 36.0));
    assert!(
        shapes[2].line.is_some(),
        "a cell of no border takes the grid"
    );
    assert_eq!(
        report
            .count(DeckConstruct::Table, DeckOutcome::Approximated)
            .get(),
        1
    );
    assert_eq!(
        report
            .count(DeckConstruct::Table, DeckOutcome::Dropped)
            .get(),
        0
    );
}

/// A chart lands as a text box in its frame: its title, then each series
/// with the values its part caches, category by category.
#[test]
fn a_chart_lands_as_the_text_of_its_cached_values() {
    let chart = "<c:chartSpace xmlns:c=\"http://schemas.openxmlformats.org/drawingml/2006/chart\" \
         xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\"><c:chart>\
         <c:title><c:tx><c:rich><a:bodyPr/><a:p><a:r><a:t>Sales</a:t></a:r></a:p></c:rich></c:tx></c:title>\
         <c:plotArea><c:barChart><c:ser><c:tx><c:strRef><c:strCache><c:pt idx=\"0\"><c:v>Units</c:v>\
         </c:pt></c:strCache></c:strRef></c:tx><c:cat><c:strRef><c:strCache>\
         <c:pt idx=\"0\"><c:v>North</c:v></c:pt><c:pt idx=\"1\"><c:v>South</c:v></c:pt>\
         </c:strCache></c:strRef></c:cat><c:val><c:numRef><c:numCache>\
         <c:pt idx=\"0\"><c:v>3</c:v></c:pt><c:pt idx=\"1\"><c:v>5</c:v></c:pt>\
         </c:numCache></c:numRef></c:val></c:ser></c:barChart></c:plotArea></c:chart>\
         <c:txPr><a:bodyPr/><a:p><a:pPr><a:defRPr sz=\"1400\"/></a:pPr></a:p></c:txPr></c:chartSpace>";
    let frame = graphic_frame(
        CHART_URI,
        "<c:chart xmlns:c=\"http://schemas.openxmlformats.org/drawingml/2006/chart\" r:id=\"CHART\"/>",
    );
    let file = one_slide(
        &frame,
        &[(
            "/ppt/charts/chart1.xml",
            CHART_REL,
            "CHART",
            chart.to_owned(),
        )],
    );
    let (shapes, report) = shapes_of(&file);
    assert_eq!(shapes.len(), 1, "{shapes:?}");
    assert_eq!(shapes[0].at, placed(72.0, 72.0, 144.0, 72.0));
    let text = shapes[0].text.as_ref().expect("the chart lands as text");
    assert_eq!(text.text, "Sales\nUnits: North 3, South 5");
    assert_eq!(text.size, pt(14.0));
    assert_eq!(
        report
            .count(DeckConstruct::Chart, DeckOutcome::Approximated)
            .get(),
        1
    );
}

/// A chart whose part is missing lands nothing, and the report names it.
#[test]
fn a_chart_without_its_part_is_named_as_dropped() {
    let frame = graphic_frame(
        CHART_URI,
        "<c:chart xmlns:c=\"http://schemas.openxmlformats.org/drawingml/2006/chart\" r:id=\"rId9\"/>",
    );
    let (shapes, report) = shapes_of(&one_slide(&frame, &[]));
    assert!(shapes.is_empty());
    assert_eq!(
        report
            .count(DeckConstruct::Chart, DeckOutcome::Dropped)
            .get(),
        1
    );
}
