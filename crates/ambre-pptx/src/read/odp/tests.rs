//! The OpenDocument reader against its PowerPoint twin, and against
//! packages a test assembles in memory in the markup LibreOffice Impress
//! writes.

use std::fmt::Write as _;

use ambre_core::{
    Payload, PrintLength, Pt, Radians, Rgba, Slant, Strikethrough, Underline, Weight,
};

use crate::PptxError;
use crate::model::{
    Cap, Deck, Geometry, Item, PathSegment, PictureFormat, Route, Site, SlideSize, TextAlign,
    TextAnchor,
};
use crate::opc::PartName;
use crate::opc::odf::{OdfType, OdfWriter};
use crate::read::tests::{JPEG_HEAD, PIXEL_PNG, sample};
use crate::report::{DeckConstruct, DeckOutcome};

const NS: &str = concat!(
    r##"xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" "##,
    r##"xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0" "##,
    r##"xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" "##,
    r##"xmlns:draw="urn:oasis:names:tc:opendocument:xmlns:drawing:1.0" "##,
    r##"xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0" "##,
    r##"xmlns:svg="urn:oasis:names:tc:opendocument:xmlns:svg-compatible:1.0" "##,
    r##"xmlns:presentation="urn:oasis:names:tc:opendocument:xmlns:presentation:1.0" "##,
    r##"xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0" "##,
    r##"xmlns:anim="urn:oasis:names:tc:opendocument:xmlns:animation:1.0" "##,
    r##"xmlns:smil="urn:oasis:names:tc:opendocument:xmlns:smil-compatible:1.0" "##,
    r##"xmlns:xlink="http://www.w3.org/1999/xlink""##,
);

/// The default styles of LibreOffice Impress that matter to a reader: a
/// page layout, a master page and the default graphic style.
fn styles_xml(size: SlideSize, master_items: &str, extra: &str) -> String {
    format!(
        r##"<?xml version="1.0" encoding="UTF-8"?>
<office:document-styles {NS} office:version="1.3">
<office:font-face-decls>
<style:font-face style:name="Georgia" svg:font-family="Georgia"/>
<style:font-face style:name="Liberation Sans" svg:font-family="'Liberation Sans'"/>
</office:font-face-decls>
<office:styles>
<style:default-style style:family="graphic"><style:graphic-properties draw:stroke="none"/></style:default-style>
<draw:gradient draw:name="Sunset" draw:start-color="#ff8000" draw:end-color="#0000ff"/>
<style:style style:name="Default-title" style:family="presentation"><style:graphic-properties draw:fill="none" draw:stroke="none" draw:textarea-vertical-align="middle"/><style:paragraph-properties fo:text-align="center"/><style:text-properties fo:font-size="44pt" fo:color="#202020" style:font-name="Liberation Sans"/></style:style>
{extra}
</office:styles>
<office:automatic-styles>
<style:page-layout style:name="PM1"><style:page-layout-properties fo:page-width="{w}pt" fo:page-height="{h}pt"/></style:page-layout>
<style:style style:name="Mdp1" style:family="drawing-page"><style:drawing-page-properties draw:fill="solid" draw:fill-color="#336699"/></style:style>
<style:style style:name="Mgr1" style:family="graphic"><style:graphic-properties draw:fill="solid" draw:fill-color="#ff0000" draw:stroke="none"/></style:style>
</office:automatic-styles>
<office:master-styles>
<style:master-page style:name="Default" style:page-layout-name="PM1" draw:style-name="Mdp1">
<draw:frame presentation:class="title" presentation:style-name="Default-title" svg:x="40pt" svg:y="20pt" svg:width="800pt" svg:height="80pt" presentation:placeholder="true"><draw:text-box/></draw:frame>
{master_items}
</style:master-page>
</office:master-styles>
</office:document-styles>"##,
        w = size.width.get(),
        h = size.height.get(),
    )
}

/// The content part of `pages` under the automatic styles `styles`.
fn content_xml(styles: &str, pages: &str) -> String {
    format!(
        r##"<?xml version="1.0" encoding="UTF-8"?>
<office:document-content {NS} office:version="1.3">
<office:automatic-styles>{styles}</office:automatic-styles>
<office:body><office:presentation>{pages}</office:presentation></office:body>
</office:document-content>"##
    )
}

/// A presentation package of `content`, `styles` and the files `files`.
fn package(content: &str, styles: &str, files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut w = OdfWriter::new(OdfType::PRESENTATION);
    let part = |name: &str| PartName::new(name).expect("a part name");
    w.part(
        part("content.xml"),
        OdfType::XML,
        Payload::new(content.as_bytes().to_vec()),
    );
    w.part(
        part("styles.xml"),
        OdfType::XML,
        Payload::new(styles.as_bytes().to_vec()),
    );
    for (name, bytes) in files {
        w.part(
            part(name),
            OdfType::picture("image/png"),
            Payload::new(bytes.to_vec()),
        );
    }
    w.finish().expect("the package writes").as_bytes().to_vec()
}

/// A one-page presentation of `items` on the master page of no shape.
fn one_page(styles: &str, items: &str, files: &[(&str, &[u8])]) -> Vec<u8> {
    let page = format!(
        r##"<draw:page draw:name="page1" draw:master-page-name="Default">{items}</draw:page>"##
    );
    package(
        &content_xml(styles, &page),
        &styles_xml(SlideSize::WIDESCREEN, "", ""),
        files,
    )
}

/// A colour as ODF spells it, `#rrggbb`.
fn hex(colour: Rgba) -> String {
    format!("#{:06x}", colour.bits() >> 8)
}

/// A length in whole English Metric Units, the unit of a view box here.
fn emu(length: PrintLength<Pt>) -> i64 {
    (f64::from(length.get()) * 12_700.0).round() as i64
}

/// The `draw:transform` LibreOffice writes for a box turned clockwise
/// about its centre. It turns counterclockwise about the corner, then
/// places the corner.
fn transform(at: &crate::model::Placement) -> String {
    let a = (std::f64::consts::TAU - at.rotation.get()).rem_euclid(std::f64::consts::TAU);
    let (sin, cos) = a.sin_cos();
    let (w, h) = (f64::from(at.width.get()), f64::from(at.height.get()));
    let (cx, cy) = (
        f64::from(at.origin.x.get()) + w / 2.0,
        f64::from(at.origin.y.get()) + h / 2.0,
    );
    let tx = cx - (cos * w / 2.0 + sin * h / 2.0);
    let ty = cy - (-sin * w / 2.0 + cos * h / 2.0);
    format!(r##"draw:transform="rotate ({a:.15}) translate ({tx:.9}pt {ty:.9}pt)""##)
}

/// The box of a placement, as an unturned item spells it, or its
/// transform.
fn boxed(at: &crate::model::Placement) -> String {
    let size = format!(
        r##"svg:width="{}pt" svg:height="{}pt""##,
        at.width.get(),
        at.height.get()
    );
    if at.rotation == Radians::ZERO {
        format!(
            r##"svg:x="{}pt" svg:y="{}pt" {size}"##,
            at.origin.x.get(),
            at.origin.y.get()
        )
    } else {
        format!("{size} {}", transform(at))
    }
}

/// The deck `deck` as LibreOffice Impress would save it. Each slide is a
/// page, and each item takes the markup Impress writes for it. Every
/// property sits in an automatic style.
fn odp_of(deck: &Deck) -> Vec<u8> {
    let mut styles = String::new();
    let mut pages = String::new();
    let mut files: Vec<(String, Vec<u8>)> = Vec::new();
    let mut next = 0_usize;
    let mut style = |styles: &mut String, family: &str, body: &str| -> String {
        next += 1;
        let name = format!("s{next}");
        let _ = write!(
            styles,
            r##"<style:style style:name="{name}" style:family="{family}">{body}</style:style>"##
        );
        name
    };
    for (index, slide) in deck.slides.iter().enumerate() {
        let page_style = style(
            &mut styles,
            "drawing-page",
            &match slide.background {
                Some(c) => format!(
                    r##"<style:drawing-page-properties draw:fill="solid" draw:fill-color="{}"/>"##,
                    hex(c)
                ),
                // A page with no background of its own shows its master's.
                None => r##"<style:drawing-page-properties/>"##.to_owned(),
            },
        );
        let name = if slide.name.is_empty() {
            format!("page{}", index + 1)
        } else {
            slide.name.clone()
        };
        let _ = write!(
            pages,
            r##"<draw:page draw:name="{name}" draw:style-name="{page_style}" draw:master-page-name="Blank">"##
        );
        for (i, item) in slide.items.iter().enumerate() {
            match item {
                Item::Shape(shape) => {
                    let fill = shape.fill.map_or(r##"draw:fill="none""##.to_owned(), |c| {
                        format!(r##"draw:fill="solid" draw:fill-color="{}""##, hex(c))
                    });
                    let stroke = shape.line.map_or(r##"draw:stroke="none""##.to_owned(), |l| {
                        format!(
                            r##"draw:stroke="solid" svg:stroke-color="{}" svg:stroke-width="{}pt""##,
                            hex(l.colour),
                            l.width.get()
                        )
                    });
                    let (anchor, inset) =
                        shape
                            .text
                            .as_ref()
                            .map_or((String::new(), String::new()), |t| {
                                (
                                    match t.anchor {
                                        TextAnchor::Middle => {
                                            r##"draw:textarea-vertical-align="middle""##
                                        }
                                        TextAnchor::Top => {
                                            r##"draw:textarea-vertical-align="top""##
                                        }
                                    }
                                    .to_owned(),
                                    format!(r##"fo:padding-left="{}pt""##, t.inset.get()),
                                )
                            });
                    let graphic = style(
                        &mut styles,
                        "graphic",
                        &format!(
                            r##"<style:graphic-properties {fill} {stroke} {anchor} {inset}/>"##
                        ),
                    );
                    let text = shape.text.as_ref().map_or(String::new(), |t| {
                        let paragraph = style(
                            &mut styles,
                            "paragraph",
                            &format!(
                                r##"<style:paragraph-properties fo:text-align="{}"/>"##,
                                match t.align {
                                    TextAlign::Centre => "center",
                                    TextAlign::Left => "start",
                                }
                            ),
                        );
                        let run = style(
                            &mut styles,
                            "text",
                            &format!(
                                r##"<style:text-properties style:font-name="{}" fo:font-size="{}pt" fo:color="{}" fo:font-weight="{}" fo:font-style="{}" style:text-underline-style="{}" style:text-line-through-style="{}"/>"##,
                                t.typeface,
                                t.size.get(),
                                hex(t.colour),
                                if t.weight == Weight::Bold { "bold" } else { "normal" },
                                if t.slant == Slant::Italic { "italic" } else { "normal" },
                                if t.underline == Underline::Underlined { "solid" } else { "none" },
                                if t.strikethrough == Strikethrough::Struck { "solid" } else { "none" },
                            ),
                        );
                        t.text
                            .lines()
                            .map(|line| {
                                format!(
                                    r##"<text:p text:style-name="{paragraph}"><text:span text:style-name="{run}">{line}</text:span></text:p>"##
                                )
                            })
                            .collect()
                    });
                    let common = format!(
                        r##"draw:name="{}" draw:id="id{i}" draw:style-name="{graphic}""##,
                        shape.name
                    );
                    let markup = match &shape.geometry {
                        Geometry::Line => format!(
                            r##"<draw:line {common} svg:x1="{}pt" svg:y1="{}pt" svg:x2="{}pt" svg:y2="{}pt">{text}</draw:line>"##,
                            shape.at.origin.x.get(),
                            shape.at.origin.y.get(),
                            shape.at.origin.x.get() + shape.at.width.get(),
                            shape.at.origin.y.get() + shape.at.height.get(),
                        ),
                        Geometry::Freeform(path) => {
                            let p = |pt: &crate::model::SlidePoint| {
                                format!(
                                    "{} {}",
                                    emu(pt.x - shape.at.origin.x),
                                    emu(pt.y - shape.at.origin.y)
                                )
                            };
                            let d: Vec<String> = path
                                .iter()
                                .map(|s| match s {
                                    PathSegment::MoveTo(a) => format!("M{}", p(a)),
                                    PathSegment::LineTo(a) => format!("L{}", p(a)),
                                    PathSegment::QuadTo { control, end } => {
                                        format!("Q{} {}", p(control), p(end))
                                    }
                                    PathSegment::CubicTo { first, second, end } => {
                                        format!("C{} {} {}", p(first), p(second), p(end))
                                    }
                                    PathSegment::Close => "Z".to_owned(),
                                })
                                .collect();
                            format!(
                                r##"<draw:path {common} {} svg:viewBox="0 0 {} {}" svg:d="{}">{text}</draw:path>"##,
                                boxed(&shape.at),
                                emu(shape.at.width),
                                emu(shape.at.height),
                                d.join(" ")
                            )
                        }
                        geometry => {
                            let (kind, modifiers) = match geometry {
                                Geometry::Ellipse => ("ellipse", String::new()),
                                Geometry::Triangle => ("isosceles-triangle", String::new()),
                                Geometry::RoundRect { radius } => (
                                    "round-rectangle",
                                    format!(
                                        r##" draw:modifiers="{}""##,
                                        radius.get() * 21_600.0
                                            / shape.at.width.min(shape.at.height).get()
                                    ),
                                ),
                                _ => ("rectangle", String::new()),
                            };
                            format!(
                                r##"<draw:custom-shape {common} {}>{text}<draw:enhanced-geometry svg:viewBox="0 0 21600 21600" draw:type="{kind}"{modifiers}/></draw:custom-shape>"##,
                                boxed(&shape.at)
                            )
                        }
                    };
                    pages.push_str(&markup);
                }
                Item::Picture(picture) => {
                    let ext = match picture.format {
                        PictureFormat::Png => "png",
                        PictureFormat::Jpeg => "jpg",
                    };
                    let file = format!("Pictures/{}.{ext}", files.len());
                    let _ = write!(
                        pages,
                        r##"<draw:frame draw:name="{}" {}><draw:image xlink:href="{file}" xlink:type="simple"/></draw:frame>"##,
                        picture.name,
                        boxed(&picture.at)
                    );
                    files.push((file, picture.bytes.as_bytes().to_vec()));
                }
                Item::Connector(connector) => {
                    let marker = |cap: Cap, which: &str| match cap {
                        Cap::Arrow => format!(r##"draw:marker-{which}="Arrow""##),
                        Cap::Plain => String::new(),
                    };
                    let graphic = style(
                        &mut styles,
                        "graphic",
                        &format!(
                            r##"<style:graphic-properties draw:stroke="solid" svg:stroke-color="{}" svg:stroke-width="{}pt" {} {}/>"##,
                            hex(connector.line.colour),
                            connector.line.width.get(),
                            marker(connector.start_cap, "start"),
                            marker(connector.end_cap, "end"),
                        ),
                    );
                    let end = |which: &str, end: &crate::model::ConnectorEnd| {
                        end.attached.map_or(String::new(), |a| {
                            let glue = match a.site {
                                Site::Top => 0,
                                Site::Right => 1,
                                Site::Bottom => 2,
                                Site::Left => 3,
                            };
                            format!(
                                r##"draw:{which}-shape="id{}" draw:{which}-glue-point="{glue}""##,
                                a.item.to_usize()
                            )
                        })
                    };
                    let kind = match connector.route {
                        Route::Straight => "line",
                        Route::Elbow => "standard",
                        Route::Curved => "curve",
                    };
                    let _ = write!(
                        pages,
                        r##"<draw:connector draw:name="{}" draw:style-name="{graphic}" draw:type="{kind}" svg:x1="{}pt" svg:y1="{}pt" svg:x2="{}pt" svg:y2="{}pt" {} {}/>"##,
                        connector.name,
                        connector.start.at.x.get(),
                        connector.start.at.y.get(),
                        connector.end.at.x.get(),
                        connector.end.at.y.get(),
                        end("start", &connector.start),
                        end("end", &connector.end),
                    );
                }
            }
        }
        if !slide.notes.is_empty() {
            let lines: String = slide
                .notes
                .lines()
                .map(|l| format!("<text:p>{l}</text:p>"))
                .collect();
            let _ = write!(
                pages,
                r##"<presentation:notes><draw:page-thumbnail draw:page-number="{}"/><draw:frame presentation:class="notes" svg:x="2cm" svg:y="14cm" svg:width="17cm" svg:height="12cm"><draw:text-box>{lines}</draw:text-box></draw:frame></presentation:notes>"##,
                index + 1
            );
        }
        pages.push_str("</draw:page>");
    }
    let styles_part = styles_xml(deck.size, "", "")
        .replace(r##"style:name="Default""##, r##"style:name="Blank""##)
        .replace("#336699", "#ffffff");
    let files: Vec<(&str, &[u8])> = files
        .iter()
        .map(|(n, b)| (n.as_str(), b.as_slice()))
        .collect();
    package(&content_xml(&styles, &pages), &styles_part, &files)
}

/// The rule of the story: an OpenDocument presentation lands as the deck
/// its PowerPoint twin lands as, item by item, with the same report.
#[test]
fn an_odp_reads_as_the_deck_its_powerpoint_twin_reads() {
    let deck = sample();
    let twin = crate::read(crate::write(&deck).expect("the deck writes").as_bytes())
        .expect("the twin reads");
    let read = crate::read_odp(&odp_of(&deck)).expect("the presentation reads");
    assert_eq!(read.deck.size, twin.deck.size);
    assert_eq!(read.deck.slides.len(), twin.deck.slides.len());
    for (odp, pptx) in read.deck.slides.iter().zip(&twin.deck.slides) {
        assert_eq!(odp.name, pptx.name);
        assert_eq!(odp.notes, pptx.notes);
        assert_eq!(odp.background, pptx.background);
        for (left, right) in odp.items.iter().zip(&pptx.items) {
            assert_eq!(left, right);
        }
        assert_eq!(odp.items.len(), pptx.items.len());
    }
    assert_eq!(read, twin);
}

/// Every geometry of the model, each turned, reads as its twin.
#[test]
fn every_outline_and_turn_reads_as_its_twin() {
    let mut deck = sample();
    let first = &mut deck.slides[0];
    for (i, item) in first.items.iter_mut().enumerate() {
        if let Item::Shape(shape) = item {
            shape.at.rotation = Radians::new(0.3 * (i as f64 + 1.0));
        }
    }
    let Item::Shape(mut extra) = first.items[0].clone() else {
        panic!("the first item is a shape");
    };
    for geometry in [
        Geometry::Triangle,
        Geometry::RoundRect {
            radius: PrintLength::<Pt>::new(10.0),
        },
        Geometry::Line,
    ] {
        extra.geometry = geometry;
        extra.text = None;
        extra.at.rotation = Radians::ZERO;
        first.items.push(Item::Shape(extra.clone()));
    }
    let twin = crate::read(crate::write(&deck).expect("the deck writes").as_bytes())
        .expect("the twin reads");
    let read = crate::read_odp(&odp_of(&deck)).expect("the presentation reads");
    assert_eq!(read, twin);
}

#[test]
fn bytes_that_are_no_package_are_refused() {
    assert!(matches!(
        crate::read_odp(b"not a zip"),
        Err(PptxError::Package(_))
    ));
}

#[test]
fn a_package_of_another_document_is_refused() {
    let mut w = OdfWriter::new(OdfType::TEXT);
    w.part(
        PartName::new("content.xml").expect("a part name"),
        OdfType::XML,
        Payload::new(content_xml("", "").into_bytes()),
    );
    let text = w.finish().expect("the package writes");
    assert_eq!(
        crate::read_odp(text.as_bytes()),
        Err(PptxError::NoPresentation)
    );
}

#[test]
fn a_presentation_of_no_page_is_refused() {
    let file = package(
        &content_xml("", ""),
        &styles_xml(SlideSize::WIDESCREEN, "", ""),
        &[],
    );
    assert_eq!(crate::read_odp(&file), Err(PptxError::NoSlide));
}

#[test]
fn a_part_that_declares_a_document_type_is_refused() {
    let content = content_xml("", r##"<draw:page draw:name="a"/>"##).replacen(
        "<office:document-content",
        "<!DOCTYPE x [<!ENTITY a \"aaaa\">]><office:document-content",
        1,
    );
    let file = package(&content, &styles_xml(SlideSize::WIDESCREEN, "", ""), &[]);
    assert_eq!(crate::read_odp(&file), Err(PptxError::Unreadable));
}

#[test]
fn a_part_nested_past_the_bound_is_refused() {
    let deep = format!("{}{}", "<draw:g>".repeat(300), "</draw:g>".repeat(300));
    let file = one_page("", &deep, &[]);
    assert_eq!(crate::read_odp(&file), Err(PptxError::Unreadable));
}

#[test]
fn a_master_that_flattens_past_the_item_bound_is_refused() {
    let shapes = r##"<draw:rect draw:style-name="Mgr1" svg:x="0pt" svg:y="0pt" svg:width="1pt" svg:height="1pt"/>"##
        .repeat(1_000);
    let page = r##"<draw:page draw:master-page-name="Default"/>"##.repeat(101);
    let file = package(
        &content_xml("", &page),
        &styles_xml(SlideSize::WIDESCREEN, &shapes, ""),
        &[],
    );
    assert_eq!(crate::read_odp(&file), Err(PptxError::TooLarge));
}

/// The master page draws its shapes on every page, under the page's own.
/// It gives its background to a page with none. Its title placeholder
/// gives its box and its style to the title of a page.
#[test]
fn the_master_page_gives_its_shapes_its_background_and_its_title_style() {
    let master_logo = r##"<draw:rect draw:name="Logo" draw:style-name="Mgr1" svg:x="900pt" svg:y="500pt" svg:width="40pt" svg:height="20pt"/>"##;
    let page = |title: &str| {
        format!(
            r##"<draw:page draw:name="page1" draw:master-page-name="Default"><draw:frame presentation:class="title" presentation:style-name="Default-title"><draw:text-box><text:p>{title}</text:p></draw:text-box></draw:frame><draw:frame presentation:class="outline" presentation:placeholder="true" svg:x="1cm" svg:y="1cm" svg:width="1cm" svg:height="1cm"><draw:text-box/></draw:frame></draw:page>"##
        )
    };
    let file = package(
        &content_xml("", &format!("{}{}", page("Opening"), page("Close"))),
        &styles_xml(SlideSize::WIDESCREEN, master_logo, ""),
        &[],
    );
    let read = crate::read_odp(&file).expect("the presentation reads");
    assert_eq!(read.deck.slides.len(), 2);
    for (slide, title) in read.deck.slides.iter().zip(["Opening", "Close"]) {
        assert_eq!(slide.name, title);
        assert_eq!(slide.background, Some(Rgba::from_bits(0x3366_99FF)));
        // The logo, then the title: the empty outline draws nothing.
        assert_eq!(slide.items.len(), 2);
        let Item::Shape(logo) = &slide.items[0] else {
            panic!("the logo is a shape");
        };
        assert_eq!(logo.name, "Logo");
        assert_eq!(logo.fill, Some(Rgba::from_bits(0xFF00_00FF)));
        let Item::Shape(heading) = &slide.items[1] else {
            panic!("the title is a shape");
        };
        assert_eq!(heading.at.origin.x.get(), 40.0);
        assert_eq!(heading.at.width.get(), 800.0);
        let text = heading.text.as_ref().expect("the title holds text");
        assert_eq!(text.text, title);
        assert_eq!(text.size.get(), 44.0);
        assert_eq!(text.colour, Rgba::from_bits(0x2020_20FF));
        assert_eq!(text.typeface, "Liberation Sans");
        assert_eq!(text.align, TextAlign::Centre);
        assert_eq!(text.anchor, TextAnchor::Middle);
        assert_eq!(heading.fill, None);
        assert_eq!(heading.line, None);
    }
    assert_eq!(read.report.entries(), &[]);
}

/// Each construct the deck cannot hold is named as the PowerPoint reader
/// names it: the same construct, the same outcome.
#[test]
fn what_the_deck_cannot_hold_is_named_as_the_powerpoint_reader_names_it() {
    let styles = concat!(
        r##"<style:style style:name="dp1" style:family="drawing-page"><style:drawing-page-properties presentation:transition-type="automatic" smil:type="slideWipe"/></style:style>"##,
        r##"<style:style style:name="gr1" style:family="graphic"><style:graphic-properties draw:fill="gradient" draw:fill-gradient-name="Sunset"/></style:style>"##,
        r##"<style:style style:name="T1" style:family="text"><style:text-properties fo:font-weight="bold"/></style:style>"##,
    );
    let items = concat!(
        r##"<draw:g><draw:rect draw:style-name="gr1" svg:x="0pt" svg:y="0pt" svg:width="10pt" svg:height="10pt"/></draw:g>"##,
        r##"<draw:frame svg:x="0pt" svg:y="0pt" svg:width="10pt" svg:height="10pt"><table:table/><draw:image xlink:href="Pictures/table.png"/></draw:frame>"##,
        r##"<draw:frame svg:x="0pt" svg:y="0pt" svg:width="10pt" svg:height="10pt" presentation:class="chart"><draw:object xlink:href="./Object 1"/></draw:frame>"##,
        r##"<draw:frame svg:x="0pt" svg:y="0pt" svg:width="10pt" svg:height="10pt"><draw:object-ole xlink:href="./Object 2"/><draw:image xlink:href="Pictures/table.png"/></draw:frame>"##,
        r##"<draw:frame svg:x="0pt" svg:y="0pt" svg:width="10pt" svg:height="10pt"><draw:plugin xlink:href="Media/clip.mp4" draw:mime-type="application/vnd.sun.star.media"/><draw:image xlink:href="Pictures/table.png"/></draw:frame>"##,
        r##"<draw:frame svg:x="0pt" svg:y="0pt" svg:width="10pt" svg:height="10pt"><draw:plugin xlink:href="Media/song.mp3" draw:mime-type="application/vnd.sun.star.media"/></draw:frame>"##,
        r##"<draw:frame svg:x="0pt" svg:y="0pt" svg:width="10pt" svg:height="10pt"><draw:image xlink:href="Pictures/drawing.svg"/></draw:frame>"##,
        r##"<draw:custom-shape svg:x="0pt" svg:y="0pt" svg:width="10pt" svg:height="10pt"><draw:enhanced-geometry svg:viewBox="0 0 21600 21600" draw:type="star5" draw:enhanced-path="M ?f0 0 L $0 10 Z"/></draw:custom-shape>"##,
        r##"<draw:frame svg:x="0pt" svg:y="0pt" svg:width="10pt" svg:height="10pt"><draw:text-box><text:p>plain <text:span text:style-name="T1">bold</text:span></text:p></draw:text-box></draw:frame>"##,
        r##"<anim:par presentation:node-type="timing-root"><anim:seq presentation:node-type="main-sequence"><anim:par smil:begin="indefinite"/></anim:seq></anim:par>"##,
    );
    let page = format!(
        r##"<draw:page draw:name="page1" draw:style-name="dp1" draw:master-page-name="Default">{items}</draw:page>"##
    );
    let file = package(
        &content_xml(styles, &page),
        &styles_xml(SlideSize::WIDESCREEN, "", ""),
        &[
            ("Pictures/table.png", PIXEL_PNG),
            ("Pictures/drawing.svg", b"<svg/>"),
        ],
    );
    let read = crate::read_odp(&file).expect("the presentation reads");
    let count = |construct, outcome| read.report.count(construct, outcome).get();
    use DeckConstruct as C;
    use DeckOutcome::{Approximated, Dropped};
    assert_eq!(count(C::Group, Approximated), 1);
    assert_eq!(count(C::NonSolidFill, Approximated), 1);
    assert_eq!(count(C::Table, Approximated), 1);
    assert_eq!(count(C::Chart, Dropped), 1);
    assert_eq!(count(C::EmbeddedObject, Approximated), 1);
    assert_eq!(count(C::Video, Approximated), 1);
    assert_eq!(count(C::Audio, Dropped), 1);
    assert_eq!(count(C::UnreadPicture, Dropped), 1);
    assert_eq!(count(C::PresetShape, Approximated), 1);
    assert_eq!(count(C::RichText, Approximated), 1);
    assert_eq!(count(C::Transition, Approximated), 1);
    assert_eq!(count(C::Animation, Dropped), 1);
    let slide = &read.deck.slides[0];
    // The gradient takes its first colour.
    let Item::Shape(grouped) = &slide.items[0] else {
        panic!("the member of the group is a shape");
    };
    assert_eq!(grouped.fill, Some(Rgba::from_bits(0xFF80_00FF)));
    // Three replacement pictures, the star's box and the text box.
    assert_eq!(slide.items.len(), 6);
}

/// LibreOffice writes a timing root on every page, with an empty main
/// sequence: that is no animation.
#[test]
fn an_empty_timing_root_is_no_animation() {
    let file = one_page(
        "",
        r##"<anim:par presentation:node-type="timing-root"><anim:seq presentation:node-type="main-sequence"/></anim:par>"##,
        &[],
    );
    let read = crate::read_odp(&file).expect("the presentation reads");
    assert!(read.report.entries().is_empty());
}

/// Lengths in centimetres, millimetres and inches land on the grid of
/// English Metric Units, and a text keeps the spaces ODF spells.
#[test]
fn lengths_in_every_unit_and_spelled_spaces_read() {
    let file = one_page(
        "",
        r##"<draw:frame draw:name="Box" svg:x="2.54cm" svg:y="25.4mm" svg:width="1in" svg:height="12pc"><draw:text-box><text:p>  two<text:s text:c="2"/>words  </text:p><text:p>next<text:line-break/>line<text:tab/>end</text:p></draw:text-box></draw:frame>"##,
        &[],
    );
    let read = crate::read_odp(&file).expect("the presentation reads");
    let Item::Shape(shape) = &read.deck.slides[0].items[0] else {
        panic!("the frame is a shape");
    };
    assert_eq!(shape.at.origin.x.get(), 72.0);
    assert_eq!(shape.at.origin.y.get(), 72.0);
    assert_eq!(shape.at.width.get(), 72.0);
    assert_eq!(shape.at.height.get(), 144.0);
    assert_eq!(shape.geometry, Geometry::Rect);
    let text = shape.text.as_ref().expect("the frame holds text");
    assert_eq!(text.text, "two  words\nnext\nline\tend");
    assert_eq!(read.deck.slides[0].name, "");
}

/// A connector with no glue point holds on at the edge nearest its end,
/// and a polygon closes its path.
#[test]
fn a_connector_with_no_glue_point_holds_on_at_the_nearest_edge() {
    let file = one_page(
        "",
        concat!(
            r##"<draw:rect draw:id="a" svg:x="0pt" svg:y="0pt" svg:width="100pt" svg:height="100pt"/>"##,
            r##"<draw:polygon xml:id="b" svg:x="200pt" svg:y="0pt" svg:width="100pt" svg:height="100pt" svg:viewBox="0 0 10 10" draw:points="0,0 10,0 10,10"/>"##,
            r##"<draw:connector draw:type="line" draw:start-shape="a" draw:end-shape="b" svg:x1="100pt" svg:y1="50pt" svg:x2="200pt" svg:y2="50pt"/>"##,
        ),
        &[],
    );
    let read = crate::read_odp(&file).expect("the presentation reads");
    let items = &read.deck.slides[0].items;
    let Item::Shape(polygon) = &items[1] else {
        panic!("the polygon is a shape");
    };
    let Geometry::Freeform(path) = &polygon.geometry else {
        panic!("the polygon is a path");
    };
    assert_eq!(path.len(), 4);
    assert_eq!(path.last(), Some(&PathSegment::Close));
    let Item::Connector(link) = &items[2] else {
        panic!("the third item is a connector");
    };
    assert_eq!(link.route, Route::Straight);
    assert_eq!(link.start.attached.map(|a| a.site), Some(Site::Right));
    assert_eq!(link.end.attached.map(|a| a.site), Some(Site::Left));
    assert_eq!(link.end.attached.map(|a| a.item.to_usize()), Some(1));
}

/// A JPEG picture lands with its bytes, and a page name the user typed is
/// the slide's name.
#[test]
fn a_jpeg_picture_lands_and_a_typed_page_name_names_the_slide() {
    let page = r##"<draw:page draw:name="Results" draw:master-page-name="Default"><draw:frame draw:name="Photo" svg:x="0pt" svg:y="0pt" svg:width="10pt" svg:height="10pt"><draw:image xlink:href="Pictures/a.jpg"/></draw:frame></draw:page>"##;
    let file = package(
        &content_xml("", page),
        &styles_xml(SlideSize::WIDESCREEN, "", ""),
        &[("Pictures/a.jpg", JPEG_HEAD)],
    );
    let read = crate::read_odp(&file).expect("the presentation reads");
    assert_eq!(read.deck.slides[0].name, "Results");
    let Item::Picture(picture) = &read.deck.slides[0].items[0] else {
        panic!("the frame is a picture");
    };
    assert_eq!(picture.format, PictureFormat::Jpeg);
    assert_eq!(picture.bytes.as_bytes(), JPEG_HEAD);
}
