//! The writer, held by opening its file back: the package layer reads the
//! zip, and an XML parser reads each part the deck produced.

use ambre_core::index::Pos;
use ambre_core::{
    FileBytes, Payload, PrintLength, Pt, Radians, Rgba, Slant, Strikethrough, Underline, Validated,
    Weight,
};

use crate::model::{
    Attachment, Cap, Connector, ConnectorEnd, Deck, Geometry, Item, Line, PathSegment, Picture,
    PictureBytes, Placement, Route, Shape, Site, Slide, SlideItems, SlidePoint, SlideSize,
    TextAlign, TextAnchor, TextBody,
};
use crate::opc::{Package, PartName, Source};
use crate::{PptxError, write};

/// The smallest PNG file: one transparent pixel.
const PIXEL_PNG: &[u8] = &[
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1F, 0x15, 0xC4,
    0x89, 0x00, 0x00, 0x00, 0x0B, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9C, 0x63, 0x60, 0x00, 0x02, 0x00,
    0x00, 0x05, 0x00, 0x01, 0x7A, 0x5E, 0xAB, 0x3F, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44,
    0xAE, 0x42, 0x60, 0x82,
];

const NS_A: &str = "http://schemas.openxmlformats.org/drawingml/2006/main";
const NS_P: &str = "http://schemas.openxmlformats.org/presentationml/2006/main";
const NS_R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";

fn pt(value: f32) -> PrintLength<Pt> {
    PrintLength::<Pt>::new(value)
}

fn point(x: f32, y: f32) -> SlidePoint {
    SlidePoint { x: pt(x), y: pt(y) }
}

fn placed(x: f32, y: f32, w: f32, h: f32) -> Placement {
    Placement {
        origin: point(x, y),
        width: pt(w),
        height: pt(h),
        rotation: Radians::ZERO,
    }
}

fn black_line() -> Line {
    Line {
        colour: Rgba::from_bits(0x0000_00FF),
        width: pt(1.0),
    }
}

fn shape(name: &str, geometry: Geometry, at: Placement) -> Item {
    Item::Shape(Shape {
        name: name.to_owned(),
        at,
        geometry,
        fill: Some(Rgba::from_bits(0xFF00_00FF)),
        line: Some(black_line()),
        text: None,
    })
}

fn text_box(body: &str) -> Item {
    Item::Shape(Shape {
        name: "Text".to_owned(),
        at: placed(10.0, 10.0, 200.0, 40.0),
        geometry: Geometry::Rect,
        fill: None,
        line: None,
        text: Some(TextBody {
            text: body.to_owned(),
            typeface: String::new(),
            size: pt(18.0),
            colour: Rgba::from_bits(0x1122_33FF),
            weight: Weight::Regular,
            slant: Slant::Upright,
            underline: Underline::Plain,
            strikethrough: Strikethrough::Plain,
            anchor: TextAnchor::Top,
            align: TextAlign::Left,
            inset: PrintLength::<Pt>::ZERO,
        }),
    })
}

fn picture(png: &[u8]) -> Item {
    Item::Picture(Picture {
        name: "Picture".to_owned(),
        at: placed(0.0, 0.0, 50.0, 50.0),
        png: Payload::<PictureBytes>::new(png.to_vec()),
    })
}

fn slide(items: Vec<Item>, notes: &str) -> Slide {
    Slide {
        name: String::new(),
        items,
        notes: notes.to_owned(),
    }
}

fn deck(slides: Vec<Slide>) -> Deck {
    Deck {
        size: SlideSize::WIDESCREEN,
        slides,
    }
}

fn name(text: &str) -> Validated<PartName> {
    Validated::<PartName>::parse(text).expect("a part name")
}

fn written(deck: &Deck) -> (Payload<FileBytes>, Package) {
    let bytes = write(deck).expect("the deck writes");
    let package = Package::open(&bytes).expect("the file opens back");
    (bytes, package)
}

fn part_text(package: &Package, part: &str) -> String {
    let bytes = package.part(&name(part)).expect("the part exists");
    String::from_utf8(bytes.as_bytes().to_vec()).expect("a part is UTF-8")
}

/// The slide parts the presentation lists, in the order of its slide list.
fn slide_parts(package: &Package) -> Vec<Validated<PartName>> {
    let pres = name("/ppt/presentation.xml");
    let rels = package
        .relationships(&Source::Part(pres))
        .expect("the presentation relationships read");
    let xml = part_text(package, "/ppt/presentation.xml");
    let doc = roxmltree::Document::parse(&xml).expect("the presentation parses");
    doc.descendants()
        .filter(|n| n.has_tag_name((NS_P, "sldId")))
        .map(|n| {
            let rid = n
                .attribute((NS_R, "id"))
                .expect("a slide id cites a relationship");
            rels.iter()
                .find(|r| r.id.as_str() == rid)
                .map(|r| r.target.clone())
                .expect("the relationship exists")
        })
        .collect()
}

/// Every run text of a part, in document order.
fn texts(xml: &str) -> Vec<String> {
    let doc = roxmltree::Document::parse(xml).expect("the part parses");
    doc.descendants()
        .filter(|n| n.has_tag_name((NS_A, "t")))
        .map(|n| n.text().unwrap_or_default().to_owned())
        .collect()
}

#[test]
fn every_relationship_reaches_a_typed_part() {
    let (_, package) = written(&deck(vec![
        slide(vec![text_box("one"), picture(PIXEL_PNG)], "first notes"),
        slide(
            vec![shape("Box", Geometry::Rect, placed(0.0, 0.0, 9.0, 9.0))],
            "",
        ),
    ]));
    let mut sources = vec![Source::Package];
    sources.extend(package.part_names().cloned().map(Source::Part));
    for source in &sources {
        for rel in package.relationships(source).expect("relationships read") {
            assert!(
                package.part(&rel.target).is_some(),
                "{:?} reaches {:?}, which the package lacks",
                source,
                rel.target
            );
        }
    }
    for part in package.part_names() {
        assert!(
            package.content_type(part).is_some(),
            "{part:?} carries no content type"
        );
    }
    for part in package.part_names() {
        if part.as_str().ends_with(".xml") || part.as_str().ends_with(".rels") {
            let text = String::from_utf8(package.part(part).expect("held").as_bytes().to_vec())
                .expect("UTF-8");
            assert!(
                roxmltree::Document::parse(&text).is_ok(),
                "{part:?} does not parse"
            );
        }
    }
}

#[test]
fn one_slide_per_slide_in_deck_order() {
    let (_, package) = written(&deck(vec![
        slide(vec![text_box("alpha")], ""),
        slide(vec![text_box("beta")], ""),
        slide(vec![text_box("gamma")], ""),
    ]));
    let order: Vec<Vec<String>> = slide_parts(&package)
        .iter()
        .map(|p| texts(&part_text(&package, p.as_str())))
        .collect();
    assert_eq!(
        order,
        vec![
            vec!["alpha".to_owned()],
            vec!["beta".to_owned()],
            vec!["gamma".to_owned()]
        ]
    );
}

#[test]
fn text_stays_editable_text_with_its_size_and_colour() {
    let (_, package) = written(&deck(vec![slide(
        vec![text_box("Q3 <plan> & \"goals\"\nsecond line")],
        "",
    )]));
    let xml = part_text(&package, "/ppt/slides/slide1.xml");
    assert_eq!(
        texts(&xml),
        vec!["Q3 <plan> & \"goals\"".to_owned(), "second line".to_owned()]
    );
    let doc = roxmltree::Document::parse(&xml).expect("parses");
    let run = doc
        .descendants()
        .find(|n| n.has_tag_name((NS_A, "rPr")))
        .expect("a run carries its properties");
    assert_eq!(run.attribute("sz"), Some("1800"));
    let colour = run
        .descendants()
        .find(|n| n.has_tag_name((NS_A, "srgbClr")))
        .and_then(|n| n.attribute("val"));
    assert_eq!(colour, Some("112233"));
    let marked = doc
        .descendants()
        .find(|n| n.has_tag_name((NS_P, "cNvSpPr")))
        .and_then(|n| n.attribute("txBox"));
    assert_eq!(marked, Some("1"), "a text with no paint is a text box");
}

#[test]
fn a_run_carries_its_face_and_its_emphasis() {
    let mut item = text_box("Bold claim");
    if let Item::Shape(Shape {
        text: Some(body), ..
    }) = &mut item
    {
        body.typeface = "Courier New".to_owned();
        body.weight = Weight::Bold;
        body.slant = Slant::Italic;
        body.underline = Underline::Underlined;
        body.strikethrough = Strikethrough::Struck;
    }
    let (_, package) = written(&deck(vec![slide(vec![item, text_box("plain")], "")]));
    let xml = part_text(&package, "/ppt/slides/slide1.xml");
    let doc = roxmltree::Document::parse(&xml).expect("parses");
    let runs: Vec<roxmltree::Node> = doc
        .descendants()
        .filter(|n| n.has_tag_name((NS_A, "rPr")))
        .collect();
    let [emphatic, plain] = runs.as_slice() else {
        panic!("two runs");
    };
    for (key, value) in [
        ("b", "1"),
        ("i", "1"),
        ("u", "sng"),
        ("strike", "sngStrike"),
    ] {
        assert_eq!(emphatic.attribute(key), Some(value), "{key}");
        assert_eq!(plain.attribute(key), None, "{key} on a plain run");
    }
    let face = emphatic
        .children()
        .find(|n| n.has_tag_name((NS_A, "latin")))
        .and_then(|n| n.attribute("typeface"));
    assert_eq!(face, Some("Courier New"));
    assert!(
        plain.children().all(|n| !n.has_tag_name((NS_A, "latin"))),
        "a run with no face takes the face of the theme"
    );
}

#[test]
fn each_geometry_writes_its_outline() {
    let freeform = Geometry::Freeform(vec![
        PathSegment::MoveTo(point(0.0, 0.0)),
        PathSegment::LineTo(point(10.0, 0.0)),
        PathSegment::QuadTo {
            control: point(10.0, 10.0),
            end: point(0.0, 10.0),
        },
        PathSegment::CubicTo {
            first: point(0.0, 5.0),
            second: point(1.0, 1.0),
            end: point(0.0, 0.0),
        },
        PathSegment::Close,
    ]);
    let (_, package) = written(&deck(vec![slide(
        vec![
            shape("A", Geometry::Rect, placed(0.0, 0.0, 10.0, 10.0)),
            shape("B", Geometry::Ellipse, placed(0.0, 0.0, 10.0, 10.0)),
            shape("C", Geometry::Triangle, placed(0.0, 0.0, 10.0, 10.0)),
            shape("D", Geometry::Line, placed(0.0, 0.0, 10.0, 10.0)),
            shape(
                "E",
                Geometry::RoundRect { radius: pt(2.0) },
                placed(0.0, 0.0, 10.0, 20.0),
            ),
            shape("F", freeform, placed(0.0, 0.0, 10.0, 10.0)),
        ],
        "",
    )]));
    let xml = part_text(&package, "/ppt/slides/slide1.xml");
    let doc = roxmltree::Document::parse(&xml).expect("parses");
    let presets: Vec<&str> = doc
        .descendants()
        .filter(|n| n.has_tag_name((NS_A, "prstGeom")))
        .filter_map(|n| n.attribute("prst"))
        .collect();
    assert_eq!(
        presets,
        ["rect", "ellipse", "triangle", "line", "roundRect"]
    );
    let adjust = doc
        .descendants()
        .find(|n| n.has_tag_name((NS_A, "gd")))
        .and_then(|n| n.attribute("fmla"));
    assert_eq!(adjust, Some("val 20000"), "a 2 pt radius on a 10 pt edge");
    let steps: Vec<&str> = doc
        .descendants()
        .find(|n| n.has_tag_name((NS_A, "path")))
        .expect("the freeform carries a path")
        .children()
        .filter(roxmltree::Node::is_element)
        .map(|n| n.tag_name().name())
        .collect();
    assert_eq!(
        steps,
        ["moveTo", "lnTo", "quadBezTo", "cubicBezTo", "close"]
    );
}

#[test]
fn a_rotated_item_carries_its_angle() {
    let mut at = placed(0.0, 0.0, 10.0, 10.0);
    at.rotation = Radians::new(std::f64::consts::FRAC_PI_2);
    let (_, package) = written(&deck(vec![slide(
        vec![
            shape("Turned", Geometry::Rect, at),
            shape("Upright", Geometry::Rect, placed(0.0, 0.0, 10.0, 10.0)),
        ],
        "",
    )]));
    let xml = part_text(&package, "/ppt/slides/slide1.xml");
    let doc = roxmltree::Document::parse(&xml).expect("parses");
    let rotations: Vec<Option<&str>> = doc
        .descendants()
        .filter(|n| n.has_tag_name((NS_A, "xfrm")))
        .skip(1)
        .map(|n| n.attribute("rot"))
        .collect();
    assert_eq!(rotations, [Some("5400000"), None]);
}

#[test]
fn a_picture_leaves_as_one_embedded_png() {
    let (_, package) = written(&deck(vec![
        slide(vec![picture(PIXEL_PNG), picture(PIXEL_PNG)], ""),
        slide(vec![picture(PIXEL_PNG)], ""),
    ]));
    let media: Vec<&Validated<PartName>> = package
        .part_names()
        .filter(|p| p.as_str().starts_with("/ppt/media/"))
        .collect();
    assert_eq!(media.len(), 1, "one picture shown three times is one part");
    let image = media.first().copied().expect("one media part");
    assert_eq!(
        package.content_type(image).map(Validated::as_str),
        Some("image/png")
    );
    let rels = package
        .relationships(&Source::Part(name("/ppt/slides/slide1.xml")))
        .expect("read");
    let xml = part_text(&package, "/ppt/slides/slide1.xml");
    let doc = roxmltree::Document::parse(&xml).expect("parses");
    for blip in doc.descendants().filter(|n| n.has_tag_name((NS_A, "blip"))) {
        let rid = blip
            .attribute((NS_R, "embed"))
            .expect("a blip cites its picture");
        let target = rels
            .iter()
            .find(|r| r.id.as_str() == rid)
            .map(|r| &r.target);
        assert_eq!(target, Some(image));
    }
}

#[test]
fn bytes_that_are_no_png_are_refused() {
    let refused = write(&deck(vec![slide(vec![picture(b"GIF89a")], "")]));
    assert_eq!(refused, Err(PptxError::NotPng));
}

#[test]
fn a_deck_of_no_slide_is_refused() {
    assert_eq!(write(&deck(Vec::new())), Err(PptxError::NoSlide));
}

#[test]
fn a_connector_holds_on_to_the_shapes_at_its_ends() {
    let end = |x: f32, y: f32, item: u32, site: Site| ConnectorEnd {
        at: point(x, y),
        attached: Some(Attachment {
            item: Pos::<SlideItems>::new(item),
            site,
        }),
    };
    let arrow = Item::Connector(Connector {
        name: "Connector".to_owned(),
        route: Route::Straight,
        // From the right of the second box back to the left of the first: the
        // line runs right to left, so the shape mirrors.
        start: end(100.0, 50.0, 1, Site::Left),
        end: end(40.0, 50.0, 0, Site::Right),
        line: black_line(),
        start_cap: Cap::Plain,
        end_cap: Cap::Arrow,
    });
    let free = Item::Connector(Connector {
        name: "Loose".to_owned(),
        route: Route::Elbow,
        start: ConnectorEnd {
            at: point(0.0, 0.0),
            attached: Some(Attachment {
                item: Pos::<SlideItems>::new(9),
                site: Site::Top,
            }),
        },
        end: ConnectorEnd {
            at: point(5.0, 5.0),
            attached: None,
        },
        line: black_line(),
        start_cap: Cap::Plain,
        end_cap: Cap::Plain,
    });
    let (_, package) = written(&deck(vec![slide(
        vec![
            shape("Left", Geometry::Rect, placed(0.0, 30.0, 40.0, 40.0)),
            shape("Right", Geometry::Ellipse, placed(100.0, 30.0, 40.0, 40.0)),
            arrow,
            free,
        ],
        "",
    )]));
    let xml = part_text(&package, "/ppt/slides/slide1.xml");
    let doc = roxmltree::Document::parse(&xml).expect("parses");
    let connectors: Vec<roxmltree::Node> = doc
        .descendants()
        .filter(|n| n.has_tag_name((NS_P, "cxnSp")))
        .collect();
    let [bound, loose] = connectors.as_slice() else {
        panic!("two connection shapes");
    };
    let attr = |node: &roxmltree::Node, tag: &str, key: &str| {
        node.descendants()
            .find(|n| n.has_tag_name((NS_A, tag)))
            .and_then(|n| n.attribute(key).map(str::to_owned))
    };
    // Shape ids count from 2: the first item is 2, the ellipse 3. Site 2 is the
    // left of an ellipse, site 3 the right of a rectangle.
    assert_eq!(attr(bound, "stCxn", "id").as_deref(), Some("3"));
    assert_eq!(attr(bound, "stCxn", "idx").as_deref(), Some("2"));
    assert_eq!(attr(bound, "endCxn", "id").as_deref(), Some("2"));
    assert_eq!(attr(bound, "endCxn", "idx").as_deref(), Some("3"));
    assert_eq!(attr(bound, "xfrm", "flipH").as_deref(), Some("1"));
    assert_eq!(attr(bound, "tailEnd", "type").as_deref(), Some("triangle"));
    assert_eq!(attr(bound, "headEnd", "type"), None);
    assert_eq!(
        attr(bound, "prstGeom", "prst").as_deref(),
        Some("straightConnector1")
    );
    assert_eq!(
        attr(loose, "stCxn", "id"),
        None,
        "an end naming no item stays free"
    );
    assert_eq!(
        attr(loose, "prstGeom", "prst").as_deref(),
        Some("bentConnector3")
    );
}

#[test]
fn speaker_notes_land_on_the_notes_page_of_their_slide() {
    let (_, package) = written(&deck(vec![
        slide(vec![text_box("a")], "Say hello\nthen <the> plan"),
        slide(vec![text_box("b")], "   "),
    ]));
    let first = package
        .relationships(&Source::Part(name("/ppt/slides/slide1.xml")))
        .expect("read");
    let notes = first
        .iter()
        .find(|r| r.kind.as_str().ends_with("/notesSlide"))
        .map(|r| r.target.clone())
        .expect("the first slide has a notes page");
    assert_eq!(
        texts(&part_text(&package, notes.as_str())),
        vec!["Say hello".to_owned(), "then <the> plan".to_owned()]
    );
    let second = package
        .relationships(&Source::Part(name("/ppt/slides/slide2.xml")))
        .expect("read");
    assert!(
        second
            .iter()
            .all(|r| !r.kind.as_str().ends_with("/notesSlide")),
        "blank notes write no notes page"
    );
    let pres = part_text(&package, "/ppt/presentation.xml");
    assert!(pres.contains("notesMasterIdLst"));
}

#[test]
fn a_deck_without_notes_writes_no_notes_master() {
    let (_, package) = written(&deck(vec![slide(vec![text_box("a")], "")]));
    assert!(
        package
            .part_names()
            .all(|p| !p.as_str().starts_with("/ppt/notes")),
        "no notes page, no notes master"
    );
}

#[test]
fn the_file_names_no_author_no_date_and_no_document() {
    let (_, package) = written(&deck(vec![slide(vec![text_box("x")], "")]));
    let core = part_text(&package, "/docProps/core.xml");
    let doc = roxmltree::Document::parse(&core).expect("parses");
    let fields: Vec<&str> = doc
        .root_element()
        .children()
        .filter(roxmltree::Node::is_element)
        .map(|n| n.tag_name().name())
        .collect();
    assert_eq!(
        fields,
        ["title"],
        "the core properties hold a fixed title alone"
    );
    assert_eq!(texts_of(&doc, "title"), ["Ambre deck"]);
}

fn texts_of(doc: &roxmltree::Document, local: &str) -> Vec<String> {
    doc.descendants()
        .filter(|n| n.tag_name().name() == local)
        .map(|n| n.text().unwrap_or_default().to_owned())
        .collect()
}

#[test]
fn the_same_deck_writes_the_same_bytes() {
    let d = deck(vec![slide(
        vec![text_box("x"), picture(PIXEL_PNG)],
        "notes",
    )]);
    let (first, _) = written(&d);
    let (second, _) = written(&d);
    assert_eq!(first, second);
}

#[test]
fn the_slide_size_reaches_the_presentation() {
    let mut d = deck(vec![slide(vec![text_box("x")], "")]);
    d.size = SlideSize::with_aspect_of(
        pt(960.0),
        SlideSize {
            width: pt(400.0),
            height: pt(300.0),
        },
    );
    let (_, package) = written(&d);
    let xml = part_text(&package, "/ppt/presentation.xml");
    let doc = roxmltree::Document::parse(&xml).expect("parses");
    let size = doc
        .descendants()
        .find(|n| n.has_tag_name((NS_P, "sldSz")))
        .map(|n| (n.attribute("cx"), n.attribute("cy")));
    assert_eq!(size, Some((Some("12192000"), Some("9144000"))));
}

#[test]
fn a_slide_size_stays_inside_the_bounds_a_presentation_accepts() {
    let tall = SlideSize::with_aspect_of(
        pt(960.0),
        SlideSize {
            width: pt(1.0),
            height: pt(1000.0),
        },
    );
    assert_eq!(tall.height, SlideSize::MAX_EDGE);
    let nothing = SlideSize::with_aspect_of(
        pt(960.0),
        SlideSize {
            width: pt(0.0),
            height: pt(0.0),
        },
    );
    assert_eq!(nothing, SlideSize::WIDESCREEN);
}

#[cfg(not(target_arch = "wasm32"))]
mod properties {
    use proptest::prelude::*;

    use super::*;

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(64))]

        /// Any text the user typed reads back from the file as it left, line
        /// by line: the escaping is the one path between the two.
        #[test]
        fn any_text_reads_back_as_it_left(lines in proptest::collection::vec("[^\n\r\u{0}-\u{8}\u{b}\u{c}\u{e}-\u{1f}\u{fffe}\u{ffff}]{1,40}", 1..5)) {
            let body = lines.join("\n");
            let (_, package) = written(&deck(vec![slide(vec![text_box(&body)], &body)]));
            let slide_texts = texts(&part_text(&package, "/ppt/slides/slide1.xml"));
            prop_assert_eq!(&slide_texts, &lines);
            let notes_texts = texts(&part_text(&package, "/ppt/notesSlides/notesSlide1.xml"));
            prop_assert_eq!(&notes_texts, &lines);
        }
    }
}
