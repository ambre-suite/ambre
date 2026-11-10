//! The PresentationML writer: a [`Deck`] to the parts of a `.pptx` package
//! (ECMA-376 Part 1, sections 13 and 19).
//!
//! The package holds one master, one blank layout and one theme. Every slide
//! thus carries its items itself, and a reader sees them where the deck put
//! them. A slide with notes gets a notes page under one notes master.
//!
//! Every item the model holds reaches the file:
//!
//! | Item | What the file holds |
//! | :--- | :--- |
//! | a shape | a preset or a custom geometry, with its text |
//! | a picture | an embedded PNG |
//! | a connector | a connection shape bound to the shapes at its ends |
//!
//! The model is the whole of what the writer carries, so it reports nothing
//! of its own.

use ambre_core::index::Pos;
use ambre_core::{
    Becomes, FileBytes, Payload, PrintLength, Pt, Radians, Rgba, Slant, Strikethrough, Underline,
    Weight,
};

use crate::PptxError;
use crate::model::{
    Cap, Connector, ConnectorEnd, Deck, Geometry, Item, Line, PathSegment, Picture, PictureBytes,
    Placement, Route, Shape, Site, Slide, SlideItems, SlidePoint, TextAlign, TextAnchor, TextBody,
};
use crate::opc::{self, PackageWriter, PartBytes, PartName, RelId, RelKind, RelTarget};
use crate::xml::Xml;

/// The DrawingML namespace.
const NS_A: &str = "http://schemas.openxmlformats.org/drawingml/2006/main";
/// The namespace of a relationship id an element cites.
const NS_R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
/// The PresentationML namespace.
const NS_P: &str = "http://schemas.openxmlformats.org/presentationml/2006/main";

/// The relationship type `name` of the office document namespace, the one
/// every relationship of a presentation uses but the core properties.
macro_rules! rel {
    ($name:literal) => {
        RelKind::new(concat!(
            "http://schemas.openxmlformats.org/officeDocument/2006/relationships/",
            $name
        ))
    };
}
/// The prefix of every PresentationML content type.
const CT_PML: &str = "application/vnd.openxmlformats-officedocument.presentationml.";

/// The static parts, written once per package whatever the deck.
const THEME_XML: &str = include_str!("parts/theme.xml");
const MASTER_XML: &str = include_str!("parts/slide_master.xml");
const LAYOUT_XML: &str = include_str!("parts/slide_layout.xml");
const NOTES_MASTER_XML: &str = include_str!("parts/notes_master.xml");
const PRES_PROPS_XML: &str = include_str!("parts/pres_props.xml");
const VIEW_PROPS_XML: &str = include_str!("parts/view_props.xml");
const TABLE_STYLES_XML: &str = include_str!("parts/table_styles.xml");
const CORE_XML: &str = include_str!("parts/core.xml");
const APP_XML: &str = include_str!("parts/app.xml");

/// The English Metric Units in one point: 914 400 per inch, 72 points per
/// inch.
const EMU_PER_PT: f64 = 12_700.0;

/// One English Metric Unit, the smallest length a part writes.
const ONE_EMU: PrintLength<Pt> = PrintLength::<Pt>::new(1.0 / 12_700.0);

/// The eight bytes every PNG file opens on.
const PNG_SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];

/// The text sizes a run may carry: 1 pt to 4 000 pt.
const MIN_TEXT_SIZE: PrintLength<Pt> = PrintLength::<Pt>::new(1.0);
const MAX_TEXT_SIZE: PrintLength<Pt> = PrintLength::<Pt>::new(4_000.0);

/// The writer of a deck.
///
/// # Errors
///
/// [`PptxError::NoSlide`] for a deck with no slide, since a presentation of
/// no slide is a file the user cannot tell from a broken one.
/// [`PptxError::NotPng`] for a picture whose bytes are no PNG.
/// [`PptxError::Package`] when the package layer refuses.
pub fn write(deck: &Deck) -> Result<Payload<FileBytes>, PptxError> {
    if deck.slides.is_empty() {
        return Err(PptxError::NoSlide);
    }
    let mut w = PackageWriter::new();
    let presentation = part("/ppt/presentation.xml")?;
    let master = part("/ppt/slideMasters/slideMaster1.xml")?;
    let layout = part("/ppt/slideLayouts/slideLayout1.xml")?;
    let theme = part("/ppt/theme/theme1.xml")?;
    let core = part("/docProps/core.xml")?;
    let app = part("/docProps/app.xml")?;

    w.relate(
        &opc::root(),
        rel!("officeDocument"),
        RelTarget::Part(presentation.clone()),
    );
    w.relate(
        &opc::root(),
        RelKind::CORE_PROPERTIES,
        RelTarget::Part(core.clone()),
    );
    w.relate(
        &opc::root(),
        rel!("extended-properties"),
        RelTarget::Part(app.clone()),
    );
    w.part(
        core,
        Some("application/vnd.openxmlformats-package.core-properties+xml"),
        Payload::new(CORE_XML.as_bytes().to_vec()),
    );
    w.part(
        app,
        Some("application/vnd.openxmlformats-officedocument.extended-properties+xml"),
        Payload::new(APP_XML.as_bytes().to_vec()),
    );

    // The static master names its layout `rId1`, so that relationship comes
    // first.
    w.relate(
        &master.clone(),
        rel!("slideLayout"),
        RelTarget::Part(layout.clone()),
    );
    w.relate(
        &master.clone(),
        rel!("theme"),
        RelTarget::Part(theme.clone()),
    );
    w.relate(
        &layout.clone(),
        rel!("slideMaster"),
        RelTarget::Part(master.clone()),
    );
    w.part(
        master.clone(),
        Some(&pml_type("slideMaster")),
        Payload::new(MASTER_XML.as_bytes().to_vec()),
    );
    w.part(
        layout.clone(),
        Some(&pml_type("slideLayout")),
        Payload::new(LAYOUT_XML.as_bytes().to_vec()),
    );
    w.part(
        theme.clone(),
        Some(THEME_TYPE),
        Payload::new(THEME_XML.as_bytes().to_vec()),
    );

    let pres = presentation.clone();
    let master_rel = w.relate(&pres, rel!("slideMaster"), RelTarget::Part(master.clone()));

    let has_notes = deck.slides.iter().any(|s| !s.notes.trim().is_empty());
    let notes_master = part("/ppt/notesMasters/notesMaster1.xml")?;
    if has_notes {
        let notes_theme = part("/ppt/theme/theme2.xml")?;
        w.relate(
            &notes_master.clone(),
            rel!("theme"),
            RelTarget::Part(notes_theme.clone()),
        );
        w.part(
            notes_theme,
            Some(THEME_TYPE),
            Payload::new(THEME_XML.as_bytes().to_vec()),
        );
        w.part(
            notes_master.clone(),
            Some(&pml_type("notesMaster")),
            Payload::new(NOTES_MASTER_XML.as_bytes().to_vec()),
        );
    }

    let mut media = Media::default();
    let mut slide_rels = Vec::with_capacity(deck.slides.len());
    for (index, slide) in deck.slides.iter().enumerate() {
        let number = index + 1;
        let name = part(&format!("/ppt/slides/slide{number}.xml"))?;
        let source = name.clone();
        w.relate(
            &source,
            rel!("slideLayout"),
            RelTarget::Part(layout.clone()),
        );
        if !slide.notes.trim().is_empty() {
            let notes = part(&format!("/ppt/notesSlides/notesSlide{number}.xml"))?;
            w.relate(&source, rel!("notesSlide"), RelTarget::Part(notes.clone()));
            let notes_source = notes.clone();
            w.relate(
                &notes_source,
                rel!("notesMaster"),
                RelTarget::Part(notes_master.clone()),
            );
            w.relate(&notes_source, rel!("slide"), RelTarget::Part(name.clone()));
            w.part(
                notes,
                Some(&pml_type("notesSlide")),
                notes_xml(&slide.notes)?,
            );
        }
        let xml = slide_xml(slide, &mut w, &source, &mut media)?;
        w.part(name.clone(), Some(&pml_type("slide")), xml);
        slide_rels.push(w.relate(&pres, rel!("slide"), RelTarget::Part(name.clone())));
    }
    let notes_rel = if has_notes {
        Some(w.relate(
            &pres,
            rel!("notesMaster"),
            RelTarget::Part(notes_master.clone()),
        ))
    } else {
        None
    };
    w.relate(&pres, rel!("theme"), RelTarget::Part(theme.clone()));
    for (kind, path, xml) in [
        ("presProps", "/ppt/presProps.xml", PRES_PROPS_XML),
        ("viewProps", "/ppt/viewProps.xml", VIEW_PROPS_XML),
        ("tableStyles", "/ppt/tableStyles.xml", TABLE_STYLES_XML),
    ] {
        let name = part(path)?;
        w.relate(&pres, rel_kind(kind)?, RelTarget::Part(name.clone()));
        w.part(
            name,
            Some(&pml_type(kind)),
            Payload::new(xml.as_bytes().to_vec()),
        );
    }
    let xml = presentation_xml(deck, &master_rel, notes_rel.as_ref(), &slide_rels)?;
    w.part(presentation, Some(&pml_type("presentation.main")), xml);
    Ok(Payload::new(w.finish()?.into_bytes()))
}

/// The pictures of the deck, each one part however many items show it.
#[derive(Default)]
struct Media {
    parts: Vec<(Payload<PictureBytes>, PartName)>,
}

/// The bytes of a picture go into the package as the bytes of its part.
impl Becomes<PartBytes> for PictureBytes {}

impl Media {
    /// The part of the picture `png`, added to the package on its first use.
    fn part_of(
        &mut self,
        w: &mut PackageWriter,
        png: &Payload<PictureBytes>,
    ) -> Result<PartName, PptxError> {
        if let Some((_, name)) = self.parts.iter().find(|(held, _)| held == png) {
            return Ok(name.clone());
        }
        if !png.as_bytes().starts_with(&PNG_SIGNATURE) {
            return Err(PptxError::NotPng);
        }
        let name = part(&format!("/ppt/media/image{}.png", self.parts.len() + 1))?;
        w.part(name.clone(), Some("image/png"), png.clone().into_role());
        self.parts.push((png.clone(), name.clone()));
        Ok(name)
    }
}

/// The presentation part: the master, the notes master, the slides in play
/// order, and the two sizes.
fn presentation_xml(
    deck: &Deck,
    master: &RelId,
    notes_master: Option<&RelId>,
    slides: &[RelId],
) -> Result<Payload<PartBytes>, PptxError> {
    let mut x = Xml::part();
    x.open(
        "p:presentation",
        &[
            ("xmlns:a", NS_A),
            ("xmlns:r", NS_R),
            ("xmlns:p", NS_P),
            ("saveSubsetFonts", "1"),
        ],
    );
    x.open("p:sldMasterIdLst", &[]);
    x.leaf(
        "p:sldMasterId",
        // A master id starts at 2^31; the static layout takes the next one.
        &[("id", "2147483648"), ("r:id", master.as_str())],
    );
    x.close("p:sldMasterIdLst");
    if let Some(notes) = notes_master {
        x.open("p:notesMasterIdLst", &[]);
        x.leaf("p:notesMasterId", &[("r:id", notes.as_str())]);
        x.close("p:notesMasterIdLst");
    }
    x.open("p:sldIdLst", &[]);
    for (index, rid) in slides.iter().enumerate() {
        // A slide id lies at 256 or above.
        let id = (256 + index).to_string();
        x.leaf("p:sldId", &[("id", &id), ("r:id", rid.as_str())]);
    }
    x.close("p:sldIdLst");
    x.leaf(
        "p:sldSz",
        &[
            ("cx", &emu(deck.size.width)),
            ("cy", &emu(deck.size.height)),
        ],
    );
    // The portrait letter page every notes page prints on.
    x.leaf("p:notesSz", &[("cx", "6858000"), ("cy", "9144000")]);
    x.close("p:presentation");
    x.finish()
        .map(Payload::new)
        .map_err(|e| PptxError::Xml(e.to_string()))
}

/// One slide part. A picture adds its media part and the relationship to it.
fn slide_xml(
    slide: &Slide,
    w: &mut PackageWriter,
    source: &PartName,
    media: &mut Media,
) -> Result<Payload<PartBytes>, PptxError> {
    let mut x = Xml::part();
    x.open(
        "p:sld",
        &[("xmlns:a", NS_A), ("xmlns:r", NS_R), ("xmlns:p", NS_P)],
    );
    if slide.name.is_empty() {
        x.open("p:cSld", &[]);
    } else {
        x.open("p:cSld", &[("name", &slide.name)]);
    }
    open_tree(&mut x);
    for (index, item) in slide.items.iter().enumerate() {
        let at = Pos::<SlideItems>::from_usize(index);
        match item {
            Item::Shape(shape) => write_shape(&mut x, at, shape),
            Item::Picture(picture) => {
                let target = media.part_of(w, &picture.png)?;
                let rid = w.relate(source, rel!("image"), RelTarget::Part(target.clone()));
                write_picture(&mut x, at, picture, &rid);
            }
            Item::Connector(connector) => write_connector(&mut x, at, connector, &slide.items),
        }
    }
    x.close("p:spTree");
    x.close("p:cSld");
    x.open("p:clrMapOvr", &[]);
    x.leaf("a:masterClrMapping", &[]);
    x.close("p:clrMapOvr");
    x.close("p:sld");
    x.finish()
        .map(Payload::new)
        .map_err(|e| PptxError::Xml(e.to_string()))
}

/// The id of the item at `at` of a slide, as a part spells it. The tree
/// itself is 1, so the first item is 2.
fn shape_id(at: Pos<SlideItems>) -> String {
    (at.to_usize() + 2).to_string()
}

/// Open the shape tree of a slide or a notes page, with the empty group
/// properties every tree opens on.
fn open_tree(x: &mut Xml) {
    x.open("p:spTree", &[]);
    x.open("p:nvGrpSpPr", &[]);
    x.leaf("p:cNvPr", &[("id", "1"), ("name", "")]);
    x.leaf("p:cNvGrpSpPr", &[]);
    x.leaf("p:nvPr", &[]);
    x.close("p:nvGrpSpPr");
    x.open("p:grpSpPr", &[]);
    x.open("a:xfrm", &[]);
    for (name, a, b) in [
        ("a:off", "x", "y"),
        ("a:ext", "cx", "cy"),
        ("a:chOff", "x", "y"),
        ("a:chExt", "cx", "cy"),
    ] {
        x.leaf(name, &[(a, "0"), (b, "0")]);
    }
    x.close("a:xfrm");
    x.close("p:grpSpPr");
}

/// One shape: its outline, its paint and its text.
fn write_shape(x: &mut Xml, at: Pos<SlideItems>, shape: &Shape) {
    let text_box = shape.fill.is_none() && shape.line.is_none() && shape.text.is_some();
    x.open("p:sp", &[]);
    x.open("p:nvSpPr", &[]);
    x.leaf("p:cNvPr", &[("id", &shape_id(at)), ("name", &shape.name)]);
    if text_box {
        x.leaf("p:cNvSpPr", &[("txBox", "1")]);
    } else {
        x.leaf("p:cNvSpPr", &[]);
    }
    x.leaf("p:nvPr", &[]);
    x.close("p:nvSpPr");
    x.open("p:spPr", &[]);
    write_xfrm(x, &shape.at, Flip::None);
    write_geometry(x, &shape.geometry, &shape.at);
    match shape.fill {
        Some(colour) => write_solid_fill(x, colour),
        None => x.leaf("a:noFill", &[]),
    }
    write_line(x, shape.line.as_ref(), Ends::None);
    x.close("p:spPr");
    if let Some(text) = &shape.text {
        write_text(x, text);
    }
    x.close("p:sp");
}

/// Which way a connection shape mirrors its box, so that its line runs from
/// its start to its end.
#[derive(Clone, Copy)]
enum Flip {
    None,
    Horizontal,
    Vertical,
    Both,
}

/// The transform of an item: its box, and its rotation when it has one.
fn write_xfrm(x: &mut Xml, at: &Placement, flip: Flip) {
    let mut attrs: Vec<(&str, String)> = Vec::new();
    if let Some(rot) = rotation(at.rotation) {
        attrs.push(("rot", rot));
    }
    if matches!(flip, Flip::Horizontal | Flip::Both) {
        attrs.push(("flipH", "1".to_owned()));
    }
    if matches!(flip, Flip::Vertical | Flip::Both) {
        attrs.push(("flipV", "1".to_owned()));
    }
    let attrs: Vec<(&str, &str)> = attrs.iter().map(|(k, v)| (*k, v.as_str())).collect();
    x.open("a:xfrm", &attrs);
    x.leaf(
        "a:off",
        &[("x", &emu(at.origin.x)), ("y", &emu(at.origin.y))],
    );
    x.leaf(
        "a:ext",
        &[("cx", &emu(at.width.abs())), ("cy", &emu(at.height.abs()))],
    );
    x.close("a:xfrm");
}

/// The rotation attribute of an angle: 60 000ths of a degree, clockwise,
/// within one turn. `None` for an upright item or an angle that is no number.
fn rotation(angle: Radians) -> Option<String> {
    if !angle.is_finite() || angle == Radians::ZERO {
        return None;
    }
    let turn = 21_600_000.0;
    let units = (angle.to_degrees() * 60_000.0).round().rem_euclid(turn);
    // A float cast saturates, and the value lies in one turn.
    #[expect(
        clippy::cast_possible_truncation,
        reason = "the value lies in one turn"
    )]
    let units = units as i64;
    (units != 0).then(|| units.to_string())
}

/// The outline of a shape: a preset, or a custom path in the box of the
/// placement.
fn write_geometry(x: &mut Xml, geometry: &Geometry, at: &Placement) {
    let preset = |x: &mut Xml, name: &str| {
        x.open("a:prstGeom", &[("prst", name)]);
        x.leaf("a:avLst", &[]);
        x.close("a:prstGeom");
    };
    match geometry {
        Geometry::Rect => preset(x, "rect"),
        Geometry::Ellipse => preset(x, "ellipse"),
        Geometry::Triangle => preset(x, "triangle"),
        Geometry::Line => preset(x, "line"),
        Geometry::RoundRect { radius } => {
            let shorter = at.width.abs().min(at.height.abs()).get();
            // The adjust value is the radius as a share of the shorter edge,
            // in 100 000ths. A radius past half that edge rounds no more.
            let share = if shorter > 0.0 {
                (f64::from(radius.get()) / f64::from(shorter) * 100_000.0).clamp(0.0, 50_000.0)
            } else {
                0.0
            };
            x.open("a:prstGeom", &[("prst", "roundRect")]);
            x.open("a:avLst", &[]);
            x.leaf(
                "a:gd",
                &[("name", "adj"), ("fmla", &format!("val {}", share.round()))],
            );
            x.close("a:avLst");
            x.close("a:prstGeom");
        }
        Geometry::Freeform(path) => write_custom(x, path, at),
    }
}

/// A custom geometry: the path, drawn in the box of the placement.
fn write_custom(x: &mut Xml, path: &[PathSegment], at: &Placement) {
    // A path of no width or no height still draws: a box of one unit keeps
    // the scale of its points finite.
    let w = emu(at.width.abs().max(ONE_EMU));
    let h = emu(at.height.abs().max(ONE_EMU));
    x.open("a:custGeom", &[]);
    for list in ["a:avLst", "a:gdLst", "a:ahLst", "a:cxnLst"] {
        x.leaf(list, &[]);
    }
    x.leaf("a:rect", &[("l", "0"), ("t", "0"), ("r", "r"), ("b", "b")]);
    x.open("a:pathLst", &[]);
    x.open("a:path", &[("w", &w), ("h", &h)]);
    let point = |x: &mut Xml, p: SlidePoint| {
        let px = emu(p.x - at.origin.x);
        let py = emu(p.y - at.origin.y);
        x.leaf("a:pt", &[("x", &px), ("y", &py)]);
    };
    for segment in path {
        match *segment {
            PathSegment::MoveTo(p) => {
                x.open("a:moveTo", &[]);
                point(x, p);
                x.close("a:moveTo");
            }
            PathSegment::LineTo(p) => {
                x.open("a:lnTo", &[]);
                point(x, p);
                x.close("a:lnTo");
            }
            PathSegment::QuadTo { control, end } => {
                x.open("a:quadBezTo", &[]);
                point(x, control);
                point(x, end);
                x.close("a:quadBezTo");
            }
            PathSegment::CubicTo { first, second, end } => {
                x.open("a:cubicBezTo", &[]);
                point(x, first);
                point(x, second);
                point(x, end);
                x.close("a:cubicBezTo");
            }
            PathSegment::Close => x.leaf("a:close", &[]),
        }
    }
    x.close("a:path");
    x.close("a:pathLst");
    x.close("a:custGeom");
}

/// A solid fill of `colour`, its alpha written when it is not opaque.
fn write_solid_fill(x: &mut Xml, colour: Rgba) {
    x.open("a:solidFill", &[]);
    write_colour(x, colour);
    x.close("a:solidFill");
}

/// One colour, as six hexadecimal digits and its alpha.
fn write_colour(x: &mut Xml, colour: Rgba) {
    let [r, g, b, a] = colour.channels();
    let hex = format!("{r:02X}{g:02X}{b:02X}");
    if a == u8::MAX {
        x.leaf("a:srgbClr", &[("val", &hex)]);
    } else {
        let alpha = (u32::from(a) * 100_000 / 255).to_string();
        x.open("a:srgbClr", &[("val", &hex)]);
        x.leaf("a:alpha", &[("val", &alpha)]);
        x.close("a:srgbClr");
    }
}

/// The arrowheads a line draws, start then end.
#[derive(Clone, Copy)]
struct EndsDrawn {
    start: Cap,
    end: Cap,
}

/// The arrowheads of a line: none for a shape outline, the caps of a
/// connector otherwise.
#[derive(Clone, Copy)]
enum Ends {
    None,
    Caps(EndsDrawn),
}

/// The outline of an item, or no outline at all.
fn write_line(x: &mut Xml, line: Option<&Line>, ends: Ends) {
    let Some(line) = line else {
        x.open("a:ln", &[]);
        x.leaf("a:noFill", &[]);
        x.close("a:ln");
        return;
    };
    x.open("a:ln", &[("w", &emu(line.width.abs()))]);
    write_solid_fill(x, line.colour);
    if let Ends::Caps(caps) = ends {
        // DrawingML calls the start of a line its head.
        for (name, cap) in [("a:headEnd", caps.start), ("a:tailEnd", caps.end)] {
            if cap == Cap::Arrow {
                x.leaf(name, &[("type", "triangle")]);
            }
        }
    }
    x.close("a:ln");
}

/// The text of a shape: one paragraph per line, each in one run.
fn write_text(x: &mut Xml, text: &TextBody) {
    let inset = emu(text.inset.abs());
    let anchor = match text.anchor {
        TextAnchor::Top => "t",
        TextAnchor::Middle => "ctr",
    };
    let align = match text.align {
        TextAlign::Left => "l",
        TextAlign::Centre => "ctr",
    };
    // A run size is in hundredths of a point.
    let size = (f64::from(text.size.clamp(MIN_TEXT_SIZE, MAX_TEXT_SIZE).get()) * 100.0)
        .round()
        .to_string();
    x.open("p:txBody", &[]);
    x.open(
        "a:bodyPr",
        &[
            ("wrap", "square"),
            ("lIns", &inset),
            ("tIns", &inset),
            ("rIns", &inset),
            ("bIns", &inset),
            ("rtlCol", "0"),
            ("anchor", anchor),
        ],
    );
    x.leaf("a:noAutofit", &[]);
    x.close("a:bodyPr");
    x.leaf("a:lstStyle", &[]);
    for line in text.text.split('\n') {
        x.open("a:p", &[]);
        x.leaf("a:pPr", &[("algn", align)]);
        if line.is_empty() {
            x.leaf("a:endParaRPr", &[("lang", "en-US"), ("sz", &size)]);
        } else {
            x.open("a:r", &[]);
            x.open("a:rPr", &run_attributes(text, &size));
            write_solid_fill(x, text.colour);
            if !text.typeface.is_empty() {
                x.leaf("a:latin", &[("typeface", &text.typeface)]);
            }
            x.close("a:rPr");
            x.open("a:t", &[]);
            x.text(line);
            x.close("a:t");
            x.close("a:r");
        }
        x.close("a:p");
    }
    x.close("p:txBody");
}

/// The attributes of a run: its size, then each emphasis it carries.
fn run_attributes<'a>(text: &TextBody, size: &'a str) -> Vec<(&'static str, &'a str)> {
    let mut attrs = vec![("lang", "en-US"), ("sz", size)];
    if text.weight == Weight::Bold {
        attrs.push(("b", "1"));
    }
    if text.slant == Slant::Italic {
        attrs.push(("i", "1"));
    }
    if text.underline == Underline::Underlined {
        attrs.push(("u", "sng"));
    }
    if text.strikethrough == Strikethrough::Struck {
        attrs.push(("strike", "sngStrike"));
    }
    attrs.push(("dirty", "0"));
    attrs
}

/// One picture, stretched to its box.
fn write_picture(x: &mut Xml, at: Pos<SlideItems>, picture: &Picture, rid: &RelId) {
    x.open("p:pic", &[]);
    x.open("p:nvPicPr", &[]);
    x.leaf("p:cNvPr", &[("id", &shape_id(at)), ("name", &picture.name)]);
    x.open("p:cNvPicPr", &[]);
    x.leaf("a:picLocks", &[("noChangeAspect", "1")]);
    x.close("p:cNvPicPr");
    x.leaf("p:nvPr", &[]);
    x.close("p:nvPicPr");
    x.open("p:blipFill", &[]);
    x.leaf("a:blip", &[("r:embed", rid.as_str())]);
    x.open("a:stretch", &[]);
    x.leaf("a:fillRect", &[]);
    x.close("a:stretch");
    x.close("p:blipFill");
    x.open("p:spPr", &[]);
    write_xfrm(x, &picture.at, Flip::None);
    write_geometry(x, &Geometry::Rect, &picture.at);
    x.close("p:spPr");
    x.close("p:pic");
}

/// One connector: a connection shape over the box of its two ends. The shape
/// mirrors so that its line runs from its start to its end, and it binds to
/// the shapes it holds on to.
fn write_connector(x: &mut Xml, at: Pos<SlideItems>, connector: &Connector, items: &[Item]) {
    let (a, b) = (connector.start.at, connector.end.at);
    let flip = match (b.x < a.x, b.y < a.y) {
        (false, false) => Flip::None,
        (true, false) => Flip::Horizontal,
        (false, true) => Flip::Vertical,
        (true, true) => Flip::Both,
    };
    let box_of = Placement {
        origin: SlidePoint {
            x: a.x.min(b.x),
            y: a.y.min(b.y),
        },
        width: (b.x - a.x).abs(),
        height: (b.y - a.y).abs(),
        rotation: Radians::ZERO,
    };
    let preset = match connector.route {
        Route::Straight => "straightConnector1",
        Route::Elbow => "bentConnector3",
        Route::Curved => "curvedConnector3",
    };
    x.open("p:cxnSp", &[]);
    x.open("p:nvCxnSpPr", &[]);
    x.leaf(
        "p:cNvPr",
        &[("id", &shape_id(at)), ("name", &connector.name)],
    );
    x.open("p:cNvCxnSpPr", &[]);
    write_connection(x, ConnectionEnd::Start, &connector.start, items);
    write_connection(x, ConnectionEnd::End, &connector.end, items);
    x.close("p:cNvCxnSpPr");
    x.leaf("p:nvPr", &[]);
    x.close("p:nvCxnSpPr");
    x.open("p:spPr", &[]);
    write_xfrm(x, &box_of, flip);
    x.open("a:prstGeom", &[("prst", preset)]);
    x.leaf("a:avLst", &[]);
    x.close("a:prstGeom");
    write_line(
        x,
        Some(&connector.line),
        Ends::Caps(EndsDrawn {
            start: connector.start_cap,
            end: connector.end_cap,
        }),
    );
    x.close("p:spPr");
    x.close("p:cxnSp");
}

/// Which end of a connection shape a binding names.
#[derive(Clone, Copy)]
enum ConnectionEnd {
    Start,
    End,
}

/// The binding of one end to the shape and the site it holds on to. A free
/// end writes nothing. An end is free when it names no item, an item that is
/// no shape, or a shape whose outline has no such site.
fn write_connection(x: &mut Xml, which: ConnectionEnd, end: &ConnectorEnd, items: &[Item]) {
    let Some(attachment) = end.attached else {
        return;
    };
    let Some(Item::Shape(shape)) = attachment.item.to_index().and_then(|i| items.get(i)) else {
        return;
    };
    // The site order of each preset, ECMA-376 Part 1, the preset shape
    // definitions.
    let site = match (&shape.geometry, attachment.site) {
        (Geometry::Rect | Geometry::RoundRect { .. }, site) => match site {
            Site::Top => 0,
            Site::Left => 1,
            Site::Bottom => 2,
            Site::Right => 3,
        },
        (Geometry::Ellipse, site) => match site {
            Site::Top => 0,
            Site::Left => 2,
            Site::Bottom => 4,
            Site::Right => 6,
        },
        (Geometry::Triangle, site) => match site {
            Site::Top => 0,
            Site::Left => 1,
            Site::Bottom => 3,
            Site::Right => 5,
        },
        (Geometry::Line | Geometry::Freeform(_), _) => return,
    };
    let name = match which {
        ConnectionEnd::Start => "a:stCxn",
        ConnectionEnd::End => "a:endCxn",
    };
    x.leaf(
        name,
        &[
            ("id", &shape_id(attachment.item)),
            ("idx", &site.to_string()),
        ],
    );
}

/// The notes page of a slide: the slide image and the notes, one paragraph
/// per line.
fn notes_xml(notes: &str) -> Result<Payload<PartBytes>, PptxError> {
    let mut x = Xml::part();
    x.open(
        "p:notes",
        &[("xmlns:a", NS_A), ("xmlns:r", NS_R), ("xmlns:p", NS_P)],
    );
    x.open("p:cSld", &[]);
    open_tree(&mut x);
    x.open("p:sp", &[]);
    x.open("p:nvSpPr", &[]);
    x.leaf(
        "p:cNvPr",
        &[("id", "2"), ("name", "Slide Image Placeholder 1")],
    );
    x.open("p:cNvSpPr", &[]);
    x.leaf(
        "a:spLocks",
        &[("noGrp", "1"), ("noRot", "1"), ("noChangeAspect", "1")],
    );
    x.close("p:cNvSpPr");
    x.open("p:nvPr", &[]);
    x.leaf("p:ph", &[("type", "sldImg")]);
    x.close("p:nvPr");
    x.close("p:nvSpPr");
    x.leaf("p:spPr", &[]);
    x.close("p:sp");
    x.open("p:sp", &[]);
    x.open("p:nvSpPr", &[]);
    x.leaf("p:cNvPr", &[("id", "3"), ("name", "Notes Placeholder 2")]);
    x.open("p:cNvSpPr", &[]);
    x.leaf("a:spLocks", &[("noGrp", "1")]);
    x.close("p:cNvSpPr");
    x.open("p:nvPr", &[]);
    x.leaf("p:ph", &[("type", "body"), ("idx", "1")]);
    x.close("p:nvPr");
    x.close("p:nvSpPr");
    x.leaf("p:spPr", &[]);
    x.open("p:txBody", &[]);
    x.leaf("a:bodyPr", &[]);
    x.leaf("a:lstStyle", &[]);
    for line in notes.split('\n') {
        x.open("a:p", &[]);
        if line.is_empty() {
            x.leaf("a:endParaRPr", &[("lang", "en-US")]);
        } else {
            x.open("a:r", &[]);
            x.leaf("a:rPr", &[("lang", "en-US"), ("dirty", "0")]);
            x.open("a:t", &[]);
            x.text(line);
            x.close("a:t");
            x.close("a:r");
        }
        x.close("a:p");
    }
    x.close("p:txBody");
    x.close("p:sp");
    x.close("p:spTree");
    x.close("p:cSld");
    x.open("p:clrMapOvr", &[]);
    x.leaf("a:masterClrMapping", &[]);
    x.close("p:clrMapOvr");
    x.close("p:notes");
    x.finish()
        .map(Payload::new)
        .map_err(|e| PptxError::Xml(e.to_string()))
}

/// A length as a part spells it: whole English Metric Units. A length that
/// is no number reads as zero, and the float cast saturates past the range.
fn emu(length: PrintLength<Pt>) -> String {
    let value = (f64::from(length.get()) * EMU_PER_PT).round();
    if value.is_finite() {
        #[expect(clippy::cast_possible_truncation, reason = "the cast saturates")]
        let units = value as i64;
        units.to_string()
    } else {
        "0".to_owned()
    }
}

/// The part name `path`.
fn part(path: &str) -> Result<PartName, PptxError> {
    PartName::new(path).ok_or(PptxError::PartName)
}

/// The PresentationML content type of the part kind `kind`.
fn pml_type(kind: &str) -> String {
    format!("{CT_PML}{kind}+xml")
}

/// The content type of a theme part.
const THEME_TYPE: &str = "application/vnd.openxmlformats-officedocument.theme+xml";

/// The relationship type of a presentation property part.
fn rel_kind(kind: &str) -> Result<RelKind, PptxError> {
    match kind {
        "presProps" => Ok(rel!("presProps")),
        "viewProps" => Ok(rel!("viewProps")),
        "tableStyles" => Ok(rel!("tableStyles")),
        _ => Err(PptxError::PartName),
    }
}
