//! The PresentationML reader (ECMA-376 Part 1, sections 13 and 19). It
//! reads the parts of a `.pptx` package into a [`Deck`], and reports what
//! the deck could not hold.
//!
//! The reader flattens the inheritance of a presentation. Each slide
//! carries what its master and its layout draw, and its background. Each
//! placeholder takes the place and the text style its chain gives it. The deck thus holds every item where PowerPoint shows it, and
//! the application that maps it needs no master.
//!
//! | In the file | In the deck | Outcome in the report |
//! | :--- | :--- | :--- |
//! | a shape, a text box, a placeholder | a shape with its text | carried |
//! | a picture, PNG or JPEG | a picture | carried |
//! | a connector | a connector bound to its shapes | carried |
//! | the notes page | the notes of its slide | carried |
//! | a group | its members, ungrouped | approximated |
//! | a text of several styles | the style of its first run | approximated |
//! | a gradient, pattern or picture fill | its first colour | approximated |
//! | an outline the model lacks | its bounding box | approximated |
//! | a video | its poster frame | approximated |
//! | a transition | nothing: the deck plays its own | approximated |
//! | an embedded object | its preview picture | approximated |
//! | a table | a shape per cell, with its text | approximated |
//! | a chart | a text box of its title and its cached values | approximated |
//! | SmartArt | the picture of its fallback, or nothing | approximated or dropped |
//! | an animation, an audio clip, an ink drawing | nothing | dropped |
//! | a picture in another encoding | nothing | dropped |

mod graphic;
mod style;
mod xml;

use std::collections::BTreeMap;

use ambre_core::index::Pos;
use ambre_core::{Count, NarrowCount, Octets};
use ambre_core::{
    Payload, PrintLength, Pt, Radians, Rgba, Slant, Strikethrough, Underline, Weight,
};

use crate::PptxError;
use crate::model::{
    Attachment, Cap, Connector, ConnectorEnd, Deck, Geometry, ImportedDeck, Item, Line,
    PathSegment, Picture, PictureBytes, PictureFormat, Placement, Route, Shape, Site, Slide,
    SlideItems, SlidePoint, SlideSize, TextAlign, TextAnchor, TextBody,
};
use crate::opc::{Package, PartBytes, PartName, RelKind, RelTarget, Relationship};
use crate::report::{DeckConstruct, DeckOutcome, DeckReport};
use style::{ColourMap, Palette, RunStyle, Theme};
use xml::{Element, Ns};

/// The English Metric Units in one point.
const EMU_PER_PT: f32 = 12_700.0;

/// The most items a deck may hold once its masters are flattened onto every
/// slide. A master drawn on a thousand slides multiplies its shapes by a
/// thousand, so the bound is read on the flattened deck.
const MAX_ITEMS: Count<DeckItems> = Count::new(100_000);

/// The most bytes the pictures of a flattened deck may hold together: a
/// logo on the master is copied onto every slide. The package bound of
/// `ambre-opc`, 64 MiB.
const MAX_PICTURE_BYTES: Count<Octets> = Count::new(64 * 1024 * 1024);

/// The items of a deck, counted against [`MAX_ITEMS`].
pub enum DeckItems {}

impl NarrowCount for DeckItems {
    const NAME: ambre_core::StaticKey<ambre_core::DebugNames> =
        ambre_core::StaticKey::new("DeckItemCount");
}

/// The bytes of a picture part are the bytes of a picture of the deck.
impl ambre_core::Becomes<PictureBytes> for PartBytes {}

/// The size of text a level of the chain leaves unset: the default of a
/// DrawingML run.
const DEFAULT_TEXT_SIZE: PrintLength<Pt> = PrintLength::<Pt>::new(18.0);

/// The space between the edge of a text box and its text, when its body
/// names none: 0.1 inch.
const DEFAULT_INSET: PrintLength<Pt> = PrintLength::<Pt>::new(7.2);

/// The relationship type `name` of the office document namespace.
macro_rules! rel {
    ($name:literal) => {
        RelKind::new(concat!(
            "http://schemas.openxmlformats.org/officeDocument/2006/relationships/",
            $name
        ))
    };
}

/// The deck a PowerPoint file holds, and the report of what it lost.
///
/// # Errors
///
/// * [`PptxError::Package`] for bytes that open as no package, or past a
///   bound of the package layer;
/// * [`PptxError::NoPresentation`] for a package of no presentation;
/// * [`PptxError::Unreadable`] for a part that is no well-formed XML;
/// * [`PptxError::NoSlide`] for a presentation of no slide;
/// * [`PptxError::TooLarge`] past the bounds of a flattened deck.
pub fn read(bytes: &[u8]) -> Result<ImportedDeck, PptxError> {
    let mut reader = Reader {
        package: Package::open(bytes)?,
        parts: BTreeMap::new(),
        report: DeckReport::default(),
        items: Count::ZERO,
        picture_bytes: Count::ZERO,
    };
    let presentation = reader
        .package
        .related(&crate::opc::root(), RelKind::OFFICE_DOCUMENT)?
        .ok_or(PptxError::NoPresentation)?;
    if !reader
        .package
        .content_type(&presentation)
        .is_some_and(|t| t.contains("presentationml"))
    {
        return Err(PptxError::NoPresentation);
    }
    let root = reader.tree(&presentation)?;
    let size = slide_size(&root);
    let defaults = root.child(Ns::P, "defaultTextStyle").cloned();
    let rels = reader.package.relationships(&presentation)?;
    let slide_parts: Vec<PartName> = root
        .child(Ns::P, "sldIdLst")
        .map(|list| {
            list.elements()
                .filter(|e| e.is(Ns::P, "sldId"))
                .filter_map(|e| target(&rels, e.attr_in(Ns::R, "id")?))
                .collect()
        })
        .unwrap_or_default();
    if slide_parts.is_empty() {
        return Err(PptxError::NoSlide);
    }
    let mut slides = Vec::with_capacity(slide_parts.len());
    for part in &slide_parts {
        slides.push(reader.slide(part, defaults.as_ref())?);
    }
    Ok(ImportedDeck {
        deck: Deck { size, slides },
        report: reader.report,
    })
}

/// The part a relationship id names among `rels`.
fn target(rels: &[Relationship], id: &str) -> Option<PartName> {
    rels.iter()
        .find(|rel| rel.id.as_str() == id)
        .and_then(|rel| match &rel.target {
            RelTarget::Part(name) => Some(name.clone()),
            RelTarget::External(_) => None,
        })
}

/// The slide size `p:sldSz` declares, held to the bounds a presentation
/// accepts. A file that names none takes the widescreen slide.
fn slide_size(root: &Element) -> SlideSize {
    let Some(size) = root.child(Ns::P, "sldSz") else {
        return SlideSize::WIDESCREEN;
    };
    match (length(size, "cx"), length(size, "cy")) {
        (Some(width), Some(height)) => SlideSize {
            width: width.clamp(SlideSize::MIN_EDGE, SlideSize::MAX_EDGE),
            height: height.clamp(SlideSize::MIN_EDGE, SlideSize::MAX_EDGE),
        },
        _ => SlideSize::WIDESCREEN,
    }
}

/// The length an attribute holds in English Metric Units, as points.
fn length(element: &Element, name: &str) -> Option<PrintLength<Pt>> {
    element
        .attr(name)
        .and_then(|v| v.parse::<f64>().ok())
        .map(|emu| (emu / f64::from(EMU_PER_PT)) as f32)
        .filter(|pt| pt.is_finite())
        .map(PrintLength::<Pt>::new)
}

/// One part, read, with the relationships it holds.
struct Part {
    root: Element,
    rels: Vec<Relationship>,
}

/// The walk of one package: the parts read so far, each read once, and the
/// report.
struct Reader<'a> {
    package: Package<'a>,
    /// The masters, the layouts and the themes, each read once for every
    /// slide that names it.
    parts: BTreeMap<PartName, std::rc::Rc<Part>>,
    report: DeckReport,
    /// The items the deck holds so far, against [`MAX_ITEMS`].
    items: Count<DeckItems>,
    /// The picture bytes the deck holds so far, against [`MAX_PICTURE_BYTES`].
    picture_bytes: Count<Octets>,
}

/// The parts one slide inherits from, its own first.
struct Chain {
    slide: std::rc::Rc<Part>,
    layout: Option<std::rc::Rc<Part>>,
    master: Option<std::rc::Rc<Part>>,
}

impl Reader<'_> {
    /// The tree of the part `name`.
    fn tree(&mut self, name: &PartName) -> Result<Element, PptxError> {
        let bytes = self.package.part(name)?.ok_or(PptxError::NoPresentation)?;
        xml::parse(&bytes).map_err(|_| PptxError::Unreadable)
    }

    /// The part `name` with its relationships, read once.
    fn shared(&mut self, name: &PartName) -> Result<std::rc::Rc<Part>, PptxError> {
        if let Some(part) = self.parts.get(name) {
            return Ok(std::rc::Rc::clone(part));
        }
        let part = std::rc::Rc::new(Part {
            root: self.tree(name)?,
            rels: self.package.relationships(name)?,
        });
        self.parts.insert(name.clone(), std::rc::Rc::clone(&part));
        Ok(part)
    }

    /// The part `part` relates to by `kind`, read once.
    fn related(
        &mut self,
        part: &Part,
        kind: RelKind,
    ) -> Result<Option<std::rc::Rc<Part>>, PptxError> {
        let name = part.rels.iter().find_map(|rel| match &rel.target {
            RelTarget::Part(name) if rel.is(kind) => Some(name.clone()),
            _ => None,
        });
        match name {
            Some(name) if self.package.contains(&name) => self.shared(&name).map(Some),
            _ => Ok(None),
        }
    }

    /// The slide `part`, with what its layout and its master draw.
    fn slide(&mut self, part: &PartName, defaults: Option<&Element>) -> Result<Slide, PptxError> {
        let slide = std::rc::Rc::new(Part {
            root: self.tree(part)?,
            rels: self.package.relationships(part)?,
        });
        let layout = self.related(&slide, rel!("slideLayout"))?;
        let master = match &layout {
            Some(layout) => self.related(layout, rel!("slideMaster"))?,
            None => None,
        };
        let theme = match &master {
            Some(master) => self
                .related(master, rel!("theme"))?
                .map(|theme| Theme::read(&theme.root)),
            None => None,
        }
        .unwrap_or_default();
        let chain = Chain {
            slide,
            layout,
            master,
        };
        let map = chain.colour_map();
        let palette = Palette {
            theme: &theme,
            map: &map,
        };
        let mut out = SlideOut::default();
        let background = self.background(&chain, palette);
        // A master and a layout draw their own shapes under the slide's,
        // unless the slide or the layout hides them.
        let shows = |part: &Part| part.root.attr("showMasterSp") != Some("0");
        if shows(&chain.slide) {
            if let (Some(master), true) = (&chain.master, chain.layout.as_deref().is_none_or(shows))
            {
                let master = std::rc::Rc::clone(master);
                self.tree_items(
                    &master,
                    &chain,
                    palette,
                    defaults,
                    Inherited::Master,
                    &mut out,
                )?;
            }
            if let Some(layout) = &chain.layout {
                let layout = std::rc::Rc::clone(layout);
                self.tree_items(
                    &layout,
                    &chain,
                    palette,
                    defaults,
                    Inherited::Layout,
                    &mut out,
                )?;
            }
        }
        let own = std::rc::Rc::clone(&chain.slide);
        self.tree_items(&own, &chain, palette, defaults, Inherited::Own, &mut out)?;
        self.effects(&chain.slide.root);
        let notes = self.notes(&chain.slide)?;
        let name = chain
            .slide
            .root
            .child(Ns::P, "cSld")
            .and_then(|c| c.attr("name"))
            .filter(|n| !n.trim().is_empty())
            .map(str::to_owned)
            .or(out.title)
            .unwrap_or_default();
        Ok(Slide {
            name,
            items: out.items,
            notes,
            background,
        })
    }

    /// A transition and the animations of one slide: neither reaches the
    /// deck, and the report names each.
    fn effects(&mut self, root: &Element) {
        if root.child(Ns::P, "transition").is_some() || alternate_holds(root, "transition") {
            self.report
                .record(DeckConstruct::Transition, DeckOutcome::Approximated);
        }
        if root
            .child(Ns::P, "timing")
            .is_some_and(|timing| timing.holds(Ns::P, "par"))
        {
            self.report
                .record(DeckConstruct::Animation, DeckOutcome::Dropped);
        }
    }

    /// The speaker notes of `slide`: the text of the body placeholder of
    /// its notes page, one line per paragraph.
    fn notes(&mut self, slide: &Part) -> Result<String, PptxError> {
        let Some(page) = self.related_once(slide, rel!("notesSlide"))? else {
            return Ok(String::new());
        };
        let body = page
            .child(Ns::P, "cSld")
            .and_then(|c| c.child(Ns::P, "spTree"))
            .and_then(|tree| {
                tree.elements()
                    .filter(|e| e.is(Ns::P, "sp"))
                    .find(|sp| placeholder(sp).is_some_and(|ph| ph.kind == PlaceholderKind::Body))
            })
            .and_then(|sp| sp.child(Ns::P, "txBody"))
            .map(|body| paragraphs(body).join("\n"))
            .unwrap_or_default();
        Ok(body.trim_end().to_owned())
    }

    /// The tree of the part `part` relates to by `kind`, read without
    /// keeping it: a notes page belongs to one slide.
    fn related_once(&mut self, part: &Part, kind: RelKind) -> Result<Option<Element>, PptxError> {
        let name = part.rels.iter().find_map(|rel| match &rel.target {
            RelTarget::Part(name) if rel.is(kind) => Some(name.clone()),
            _ => None,
        });
        match name {
            Some(name) if self.package.contains(&name) => self.tree(&name).map(Some),
            _ => Ok(None),
        }
    }

    /// The background of the slide: the first of the chain to set one.
    fn background(&mut self, chain: &Chain, palette: Palette<'_>) -> Option<Rgba> {
        let bg = chain
            .parts()
            .find_map(|part| part.root.child(Ns::P, "cSld")?.child(Ns::P, "bg"))?;
        if let Some(props) = bg.child(Ns::P, "bgPr") {
            self.fill(props, palette).flatten()
        } else {
            bg.child(Ns::P, "bgRef").and_then(|r| palette.first(r))
        }
    }

    /// The fill `props`, a `p:spPr` or a `p:bgPr`, names: `Some(None)` for
    /// no fill, `None` when it names none and leaves the fill to a style.
    /// A gradient, a pattern or a picture fill takes its first colour, and
    /// the report names it.
    fn fill(&mut self, props: &Element, palette: Palette<'_>) -> Option<Option<Rgba>> {
        for child in props.elements() {
            match child.name() {
                "noFill" => return Some(None),
                "solidFill" => return Some(palette.first(child)),
                "gradFill" => {
                    self.report
                        .record(DeckConstruct::NonSolidFill, DeckOutcome::Approximated);
                    return Some(
                        child
                            .child(Ns::A, "gsLst")
                            .and_then(|stops| stops.elements().next())
                            .and_then(|stop| palette.first(stop)),
                    );
                }
                "pattFill" => {
                    self.report
                        .record(DeckConstruct::NonSolidFill, DeckOutcome::Approximated);
                    return Some(child.child(Ns::A, "fgClr").and_then(|c| palette.first(c)));
                }
                "blipFill" | "grpFill" => {
                    self.report
                        .record(DeckConstruct::NonSolidFill, DeckOutcome::Approximated);
                    return Some(None);
                }
                _ => {}
            }
        }
        None
    }

    /// Every item the shape tree of `part` draws, in paint order. A master
    /// and a layout draw their shapes alone: a placeholder of theirs is a
    /// prompt PowerPoint shows in its editor, never on the slide.
    fn tree_items(
        &mut self,
        part: &Part,
        chain: &Chain,
        palette: Palette<'_>,
        defaults: Option<&Element>,
        inherited: Inherited,
        out: &mut SlideOut,
    ) -> Result<(), PptxError> {
        let Some(tree) = part
            .root
            .child(Ns::P, "cSld")
            .and_then(|c| c.child(Ns::P, "spTree"))
        else {
            return Ok(());
        };
        let context = Context {
            part,
            chain,
            palette,
            defaults,
            inherited,
        };
        self.children(tree, &context, Transform::IDENTITY, out)
    }

    /// The items of the children of a shape tree or of a group, under
    /// `transform`.
    fn children(
        &mut self,
        parent: &Element,
        context: &Context<'_>,
        transform: Transform,
        out: &mut SlideOut,
    ) -> Result<(), PptxError> {
        for child in parent.elements() {
            match (child.ns, child.name()) {
                (Ns::P, "sp") => self.shape(child, context, transform, out)?,
                (Ns::P, "pic") => self.picture(child, context, transform, out)?,
                (Ns::P, "cxnSp") => self.connector(child, context, transform, out)?,
                (Ns::P, "grpSp") => {
                    self.report
                        .record(DeckConstruct::Group, DeckOutcome::Approximated);
                    let inner = child
                        .child(Ns::P, "grpSpPr")
                        .and_then(|props| props.child(Ns::A, "xfrm"))
                        .map_or(transform, |xfrm| transform.then(Transform::group(xfrm)));
                    self.children(child, context, inner, out)?;
                }
                (Ns::P, "graphicFrame") => self.frame(child, context, transform, out)?,
                (Ns::P, "contentPart") => {
                    self.report.record(DeckConstruct::Ink, DeckOutcome::Dropped)
                }
                (Ns::Mc, "AlternateContent") => {
                    // The fallback is what a reader that knows no extension
                    // draws: a picture of a SmartArt or of an equation.
                    if let Some(fallback) = child.child(Ns::Mc, "Fallback") {
                        self.children(fallback, context, transform, out)?;
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// A shape: its outline, its paint and its text. A placeholder of the
    /// slide takes the place and the paint its layout and master give it.
    fn shape(
        &mut self,
        sp: &Element,
        context: &Context<'_>,
        transform: Transform,
        out: &mut SlideOut,
    ) -> Result<(), PptxError> {
        let ph = placeholder(sp);
        if ph.is_some() && context.inherited != Inherited::Own {
            return Ok(());
        }
        let inherited: Vec<&Element> = ph
            .as_ref()
            .map(|ph| context.chain.placeholders(ph))
            .unwrap_or_default();
        let props = sp.child(Ns::P, "spPr");
        let xfrm = props.and_then(|p| p.child(Ns::A, "xfrm")).or_else(|| {
            inherited
                .iter()
                .find_map(|s| s.child(Ns::P, "spPr")?.child(Ns::A, "xfrm"))
        });
        let Some(box_) = xfrm.and_then(Frame::of) else {
            return Ok(());
        };
        let box_ = transform.frame(box_);
        let style = sp.child(Ns::P, "style");
        let mut fill = props.and_then(|p| self.fill(p, context.palette));
        for s in &inherited {
            if fill.is_none() {
                fill = s
                    .child(Ns::P, "spPr")
                    .and_then(|p| self.fill(p, context.palette));
            }
        }
        let fill = fill.unwrap_or_else(|| {
            style
                .and_then(|s| s.child(Ns::A, "fillRef"))
                .filter(|r| r.attr("idx").is_some_and(|i| i != "0"))
                .and_then(|r| context.palette.first(r))
        });
        let line = line_of(props, style, context.palette);
        let text = sp
            .child(Ns::P, "txBody")
            .and_then(|body| self.text(body, ph.as_ref(), &inherited, style, context));
        if let (Some(ph), Some(text)) = (&ph, &text)
            && matches!(
                ph.kind,
                PlaceholderKind::Title | PlaceholderKind::CentredTitle
            )
            && out.title.is_none()
        {
            out.title = text.text.lines().next().map(str::to_owned);
        }
        // A placeholder the slide leaves empty draws nothing on the slide.
        if ph.is_some() && text.is_none() && fill.is_none() && line.is_none() {
            return Ok(());
        }
        let geometry = self.geometry(props, &box_);
        let name = name_of(sp, "nvSpPr");
        let id = shape_id(sp, "nvSpPr");
        let at = out.push(Item::Shape(Shape {
            name,
            at: box_.placement(),
            geometry,
            fill,
            line,
            text,
        }));
        self.count()?;
        if context.inherited == Inherited::Own
            && let Some(id) = id
        {
            out.ids.push((id, at));
        }
        Ok(())
    }

    /// The outline `props` names, in the box it fills. An outline the model
    /// lacks lands as its box, and the report names it.
    fn geometry(&mut self, props: Option<&Element>, frame: &Frame) -> Geometry {
        let Some(props) = props else {
            return Geometry::Rect;
        };
        if let Some(custom) = props.child(Ns::A, "custGeom") {
            return match custom_path(custom, frame) {
                Some((path, exact)) => {
                    if !exact {
                        self.report
                            .record(DeckConstruct::PresetShape, DeckOutcome::Approximated);
                    }
                    Geometry::Freeform(path)
                }
                None => Geometry::Rect,
            };
        }
        let Some(preset) = props.child(Ns::A, "prstGeom") else {
            return Geometry::Rect;
        };
        match preset.attr("prst").unwrap_or("rect") {
            "rect" => Geometry::Rect,
            "roundRect" => {
                let adjust = preset
                    .child(Ns::A, "avLst")
                    .and_then(|list| list.elements().find(|g| g.attr("name") == Some("adj")))
                    .and_then(|g| g.attr("fmla"))
                    .and_then(|f| f.strip_prefix("val "))
                    .and_then(|v| v.trim().parse::<f32>().ok())
                    .unwrap_or(16_667.0);
                let shorter = frame.width.min(frame.height);
                Geometry::RoundRect {
                    radius: PrintLength::<Pt>::new(shorter.get() * adjust / 100_000.0),
                }
            }
            "ellipse" => Geometry::Ellipse,
            "triangle" => Geometry::Triangle,
            "line" | "straightConnector1" => frame.line(),
            _ => {
                self.report
                    .record(DeckConstruct::PresetShape, DeckOutcome::Approximated);
                Geometry::Rect
            }
        }
    }

    /// The text of a shape, in the style its chain gives its first run.
    /// `None` for a body with no character to show.
    fn text(
        &mut self,
        body: &Element,
        ph: Option<&Placeholder>,
        inherited: &[&Element],
        style: Option<&Element>,
        context: &Context<'_>,
    ) -> Option<TextBody> {
        let text = paragraphs(body).join("\n");
        if text.trim().is_empty() {
            return None;
        }
        let palette = context.palette;
        let first_paragraph = body.elements().find(|p| p.is(Ns::A, "p"));
        let level = first_paragraph
            .and_then(|p| p.child(Ns::A, "pPr"))
            .and_then(|p| p.attr("lvl"))
            .and_then(|l| l.parse::<usize>().ok())
            .unwrap_or(0);
        let level_name = style::level_name(level);
        // The level elements of the chain, highest first. The paragraph's
        // own comes first, then the list styles of the shape, the layout and
        // the master, then the defaults and the master's text styles.
        let list_level = |b: &Element| -> Option<Element> {
            b.child(Ns::A, "lstStyle")?
                .child(Ns::A, level_name)
                .cloned()
        };
        let mut levels: Vec<Element> = Vec::new();
        if let Some(own) = first_paragraph.and_then(|p| p.child(Ns::A, "pPr")) {
            levels.push(own.clone());
        }
        levels.extend(list_level(body));
        for s in inherited {
            if let Some(b) = s.child(Ns::P, "txBody") {
                levels.extend(list_level(b));
            }
        }
        if ph.is_none()
            && let Some(defaults) = context.defaults.and_then(|d| d.child(Ns::A, level_name))
        {
            levels.push(defaults.clone());
        }
        let master_style = context.chain.master.as_ref().and_then(|m| {
            let styles = m.root.child(Ns::P, "txStyles")?;
            let which = match ph.map(|ph| ph.kind) {
                Some(PlaceholderKind::Title | PlaceholderKind::CentredTitle) => "titleStyle",
                Some(PlaceholderKind::Body | PlaceholderKind::Other) => "bodyStyle",
                None => "otherStyle",
            };
            styles
                .child(Ns::P, which)?
                .child(Ns::A, level_name)
                .cloned()
        });
        levels.extend(master_style);
        let mut chain_style = RunStyle::default();
        for level in levels.iter().rev() {
            if let Some(def) = level.child(Ns::A, "defRPr") {
                chain_style = RunStyle::read(def, palette).over(chain_style);
            }
        }
        if let Some(colour) = style
            .and_then(|s| s.child(Ns::A, "fontRef"))
            .and_then(|r| palette.first(r))
        {
            chain_style.colour = chain_style.colour.or(Some(colour));
        }
        let runs: Vec<RunStyle> = body
            .elements()
            .filter(|p| p.is(Ns::A, "p"))
            .flat_map(|p| {
                p.elements()
                    .filter(|r| r.is(Ns::A, "r") || r.is(Ns::A, "fld"))
            })
            .filter(|r| {
                r.child(Ns::A, "t")
                    .is_some_and(|t| !t.text().trim().is_empty())
            })
            .map(|r| {
                r.child(Ns::A, "rPr")
                    .map(|props| RunStyle::read(props, palette))
                    .unwrap_or_default()
            })
            .collect();
        let first = runs.first().cloned().unwrap_or_default();
        if runs.iter().skip(1).any(|r| !same_look(r, &first)) {
            self.report
                .record(DeckConstruct::RichText, DeckOutcome::Approximated);
        }
        let resolved = first.over(chain_style);
        let scale = body
            .child(Ns::A, "bodyPr")
            .and_then(|b| b.child(Ns::A, "normAutofit"))
            .and_then(|a| a.attr("fontScale"))
            .and_then(|s| s.parse::<f32>().ok())
            .map_or(1.0, |s| (s / 100_000.0).clamp(0.01, 1.0));
        let size = resolved.size.unwrap_or(DEFAULT_TEXT_SIZE);
        let align = levels
            .iter()
            .find_map(|l| l.attr("algn"))
            .map_or(TextAlign::Left, |a| {
                if a == "ctr" {
                    TextAlign::Centre
                } else {
                    TextAlign::Left
                }
            });
        let body_props = std::iter::once(body)
            .chain(inherited.iter().filter_map(|s| s.child(Ns::P, "txBody")))
            .filter_map(|b| b.child(Ns::A, "bodyPr"));
        let anchor =
            body_props
                .clone()
                .find_map(|b| b.attr("anchor"))
                .map_or(TextAnchor::Top, |a| {
                    if a == "ctr" {
                        TextAnchor::Middle
                    } else {
                        TextAnchor::Top
                    }
                });
        let inset = body_props
            .clone()
            .find_map(|b| length(b, "lIns"))
            .unwrap_or(DEFAULT_INSET);
        Some(TextBody {
            text,
            typeface: resolved.typeface.unwrap_or_default(),
            size: PrintLength::<Pt>::new(size.get() * scale),
            colour: resolved.colour.unwrap_or(Rgba::from_bits(0x0000_00FF)),
            weight: resolved.weight.unwrap_or(Weight::Regular),
            slant: resolved.slant.unwrap_or(Slant::Upright),
            underline: resolved.underline.unwrap_or(Underline::Plain),
            strikethrough: resolved.strikethrough.unwrap_or(Strikethrough::Plain),
            anchor,
            align,
            inset,
        })
    }

    /// A picture, PNG or JPEG. A video lands as its poster frame, an audio
    /// clip as nothing, and a picture of another encoding as nothing.
    fn picture(
        &mut self,
        pic: &Element,
        context: &Context<'_>,
        transform: Transform,
        out: &mut SlideOut,
    ) -> Result<(), PptxError> {
        let media = pic.path(Ns::P, &["nvPicPr", "nvPr"]);
        if media.is_some_and(|m| m.child(Ns::A, "audioFile").is_some()) {
            self.report
                .record(DeckConstruct::Audio, DeckOutcome::Dropped);
            return Ok(());
        }
        let video = media.is_some_and(|m| {
            m.child(Ns::A, "videoFile").is_some() || m.child(Ns::A, "quickTimeFile").is_some()
        });
        let Some(frame) = pic
            .child(Ns::P, "spPr")
            .and_then(|p| p.child(Ns::A, "xfrm"))
            .and_then(Frame::of)
        else {
            return Ok(());
        };
        let embed = pic
            .child(Ns::P, "blipFill")
            .and_then(|b| b.child(Ns::A, "blip"))
            .and_then(|b| b.attr_in(Ns::R, "embed"));
        let picture = match embed.and_then(|id| target(&context.part.rels, id)) {
            Some(name) => self.package.part(&name)?,
            None => None,
        };
        let format = picture.as_ref().and_then(picture_format);
        let (Some(bytes), Some(format)) = (picture, format) else {
            let construct = if video {
                DeckConstruct::Video
            } else {
                DeckConstruct::UnreadPicture
            };
            self.report.record(construct, DeckOutcome::Dropped);
            return Ok(());
        };
        if video {
            self.report
                .record(DeckConstruct::Video, DeckOutcome::Approximated);
        }
        self.picture_bytes = self.picture_bytes.plus(Count::of(bytes.as_bytes().len()));
        if self.picture_bytes > MAX_PICTURE_BYTES {
            return Err(PptxError::TooLarge);
        }
        let _ = out.push(Item::Picture(Picture {
            name: name_of(pic, "nvPicPr"),
            at: transform.frame(frame).placement(),
            format,
            bytes: bytes.into_role(),
        }));
        self.count()
    }

    /// A graphic frame: a table, a chart, a SmartArt or an embedded object.
    /// A table lands as its cells and a chart as the text of its values; an
    /// embedded object lands as its preview, and a SmartArt as nothing.
    fn frame(
        &mut self,
        frame: &Element,
        context: &Context<'_>,
        transform: Transform,
        out: &mut SlideOut,
    ) -> Result<(), PptxError> {
        let data = frame.path(Ns::A, &["graphic", "graphicData"]);
        let uri = data.and_then(|d| d.attr("uri")).unwrap_or_default();
        let construct = match uri.rsplit('/').next().unwrap_or_default() {
            "table" => DeckConstruct::Table,
            "chart" => DeckConstruct::Chart,
            "diagram" => DeckConstruct::SmartArt,
            "ole" => DeckConstruct::EmbeddedObject,
            _ => {
                if uri.contains("chart") {
                    DeckConstruct::Chart
                } else {
                    DeckConstruct::EmbeddedObject
                }
            }
        };
        let held = match (construct, data) {
            (DeckConstruct::Table, Some(data)) => match data.child(Ns::A, "tbl") {
                Some(tbl) => self.table(frame, tbl, context, transform, out)?,
                None => graphic::Landed::Nothing,
            },
            (DeckConstruct::Chart, Some(data)) => {
                self.chart(frame, data, context, transform, out)?
            }
            _ => graphic::Landed::Nothing,
        };
        if held == graphic::Landed::Held {
            self.report.record(construct, DeckOutcome::Approximated);
            return Ok(());
        }
        // An embedded object written by a newer PowerPoint carries its
        // preview as a picture inside the object.
        let preview = data
            .and_then(|d| d.child(Ns::P, "oleObj"))
            .and_then(|o| o.child(Ns::P, "pic"));
        if let (Some(pic), Some(xfrm)) = (preview, frame.child(Ns::P, "xfrm")) {
            let before = out.items.len();
            let mut placed = pic.clone();
            // The preview fills the frame of the object.
            if let Some(Node::Element(props)) = placed
                .children
                .iter_mut()
                .find(|n| matches!(n, Node::Element(e) if e.is(Ns::P, "spPr")))
            {
                props
                    .children
                    .retain(|n| !matches!(n, Node::Element(e) if e.is(Ns::A, "xfrm")));
                props.children.insert(0, Node::Element(xfrm.clone()));
            }
            self.picture(&placed, context, transform, out)?;
            if out.items.len() > before {
                self.report.record(construct, DeckOutcome::Approximated);
                return Ok(());
            }
        }
        self.report.record(construct, DeckOutcome::Dropped);
        Ok(())
    }

    /// A connector: its route, its two ends and the shapes they hold.
    fn connector(
        &mut self,
        cxn: &Element,
        context: &Context<'_>,
        transform: Transform,
        out: &mut SlideOut,
    ) -> Result<(), PptxError> {
        let props = cxn.child(Ns::P, "spPr");
        let Some(raw) = props.and_then(|p| p.child(Ns::A, "xfrm")) else {
            return Ok(());
        };
        let Some(frame) = Frame::of(raw) else {
            return Ok(());
        };
        let frame = transform.frame(frame);
        let route = match props
            .and_then(|p| p.child(Ns::A, "prstGeom"))
            .and_then(|g| g.attr("prst"))
            .unwrap_or("straightConnector1")
        {
            p if p.starts_with("bentConnector") => Route::Elbow,
            p if p.starts_with("curvedConnector") => Route::Curved,
            _ => Route::Straight,
        };
        let style = cxn.child(Ns::P, "style");
        let line = line_of(props, style, context.palette).unwrap_or(Line {
            colour: Rgba::from_bits(0x0000_00FF),
            width: PrintLength::<Pt>::new(0.75),
        });
        let ln = props.and_then(|p| p.child(Ns::A, "ln"));
        let cap = |which: &str| {
            ln.and_then(|l| l.child(Ns::A, which))
                .and_then(|e| e.attr("type"))
                .map_or(Cap::Plain, |t| {
                    if t == "none" { Cap::Plain } else { Cap::Arrow }
                })
        };
        let ends = cxn.path(Ns::P, &["nvCxnSpPr", "cNvCxnSpPr"]);
        let attach = |which: &str| -> Option<Attachment> {
            if context.inherited != Inherited::Own {
                return None;
            }
            let end = ends?.child(Ns::A, which)?;
            let id = end.attr("id")?;
            let item = out
                .ids
                .iter()
                .find(|(held, _)| held == id)
                .map(|(_, at)| *at)?;
            let index = end
                .attr("idx")
                .and_then(|i| i.parse::<u8>().ok())
                .unwrap_or(0);
            let geometry = match out.items.get(item.to_usize()) {
                Some(Item::Shape(shape)) => Some(&shape.geometry),
                _ => None,
            };
            Some(Attachment {
                item,
                site: site_of(geometry, index),
            })
        };
        let (start, end) = frame.ends();
        let connector = Connector {
            name: name_of(cxn, "nvCxnSpPr"),
            route,
            start: ConnectorEnd {
                at: start,
                attached: attach("stCxn"),
            },
            end: ConnectorEnd {
                at: end,
                attached: attach("endCxn"),
            },
            line,
            start_cap: cap("headEnd"),
            end_cap: cap("tailEnd"),
        };
        let _ = out.push(Item::Connector(connector));
        self.count()
    }

    /// Count one more item in the deck, against [`MAX_ITEMS`].
    fn count(&mut self) -> Result<(), PptxError> {
        self.items = self.items.incremented();
        if self.items > MAX_ITEMS {
            Err(PptxError::TooLarge)
        } else {
            Ok(())
        }
    }
}

use xml::Node;

/// The edge a connection site of a preset sits on. Its index counts the
/// sites of the preset counterclockwise from the top, as ECMA-376 lists
/// them. A diagonal site takes the nearest edge in that order.
fn site_of(geometry: Option<&Geometry>, index: u8) -> Site {
    let edges: &[Site] = match geometry {
        Some(Geometry::Ellipse) => &[
            Site::Top,
            Site::Left,
            Site::Left,
            Site::Bottom,
            Site::Bottom,
            Site::Right,
            Site::Right,
            Site::Top,
        ],
        Some(Geometry::Triangle) => &[
            Site::Top,
            Site::Left,
            Site::Left,
            Site::Bottom,
            Site::Right,
            Site::Right,
        ],
        _ => &[Site::Top, Site::Left, Site::Bottom, Site::Right],
    };
    edges.get(usize::from(index)).copied().unwrap_or(Site::Top)
}

/// Whether the slide holds `name` inside an alternate content block: a
/// newer PowerPoint wraps its transitions in one.
fn alternate_holds(root: &Element, name: &str) -> bool {
    root.elements()
        .filter(|e| e.is(Ns::Mc, "AlternateContent"))
        .any(|e| e.holds(Ns::P, name))
}

/// Whether two runs look alike: the properties the deck keeps per text.
fn same_look(a: &RunStyle, b: &RunStyle) -> bool {
    a.size == b.size
        && a.weight == b.weight
        && a.slant == b.slant
        && a.underline == b.underline
        && a.strikethrough == b.strikethrough
        && a.colour == b.colour
        && a.typeface == b.typeface
}

/// The text of each paragraph of a text body: its runs, its fields, and a
/// line break as a new line.
fn paragraphs(body: &Element) -> Vec<String> {
    body.elements()
        .filter(|p| p.is(Ns::A, "p"))
        .map(|p| {
            p.elements()
                .map(|run| match run.name() {
                    "r" | "fld" => run.child(Ns::A, "t").map(Element::text).unwrap_or_default(),
                    "br" => String::from("\n"),
                    _ => String::new(),
                })
                .collect()
        })
        .collect()
}

/// The encoding a picture's bytes open on.
fn picture_format(part: &Payload<PartBytes>) -> Option<PictureFormat> {
    let bytes = part.as_bytes();
    if bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]) {
        Some(PictureFormat::Png)
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some(PictureFormat::Jpeg)
    } else {
        None
    }
}

/// The name the selection pane lists an item under.
fn name_of(item: &Element, properties: &str) -> String {
    item.path(Ns::P, &[properties, "cNvPr"])
        .and_then(|c| c.attr("name"))
        .unwrap_or_default()
        .to_owned()
}

/// The id a connector of the slide names an item by.
fn shape_id(item: &Element, properties: &str) -> Option<String> {
    item.path(Ns::P, &[properties, "cNvPr"])
        .and_then(|c| c.attr("id"))
        .map(str::to_owned)
}

/// The line `props` names, or the line of the style reference when it
/// names none. `None` draws no outline.
fn line_of(props: Option<&Element>, style: Option<&Element>, palette: Palette<'_>) -> Option<Line> {
    let ln = props.and_then(|p| p.child(Ns::A, "ln"));
    let reference = style
        .and_then(|s| s.child(Ns::A, "lnRef"))
        .filter(|r| r.attr("idx").is_some_and(|i| i != "0"));
    if ln.is_some_and(|l| l.child(Ns::A, "noFill").is_some()) {
        return None;
    }
    let colour = ln
        .and_then(|l| l.child(Ns::A, "solidFill"))
        .and_then(|f| palette.first(f))
        .or_else(|| reference.and_then(|r| palette.first(r)))?;
    // A style reference names a width of the theme by its rank: the
    // Office theme widths are 0.5, 1 and 1.5 pt.
    let by_rank = reference
        .and_then(|r| r.attr("idx"))
        .and_then(|i| i.parse::<f32>().ok())
        .map_or(0.75, |rank| rank.clamp(1.0, 3.0) * 0.5);
    let width = ln
        .and_then(|l| length(l, "w"))
        .unwrap_or(PrintLength::<Pt>::new(by_rank));
    Some(Line { colour, width })
}

/// The kind of a placeholder, as its master and its text style group it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PlaceholderKind {
    Title,
    CentredTitle,
    Body,
    /// A date, a footer, a slide number, a picture or an object
    /// placeholder.
    Other,
}

/// A placeholder of a slide, a layout or a master: its kind and its
/// index, which the layout matches it by.
#[derive(Clone, Debug)]
struct Placeholder {
    kind: PlaceholderKind,
    /// The type as the file spells it, which a master matches.
    spelled: String,
    index: Option<String>,
}

/// The placeholder `sp` is, if it is one.
fn placeholder(sp: &Element) -> Option<Placeholder> {
    let ph = sp
        .elements()
        .find(|e| e.name().starts_with("nv"))?
        .child(Ns::P, "nvPr")?
        .child(Ns::P, "ph")?;
    let spelled = ph.attr("type").unwrap_or("body").to_owned();
    let kind = match spelled.as_str() {
        "title" => PlaceholderKind::Title,
        "ctrTitle" => PlaceholderKind::CentredTitle,
        "body" | "subTitle" | "obj" => PlaceholderKind::Body,
        _ => PlaceholderKind::Other,
    };
    Some(Placeholder {
        kind,
        spelled,
        index: ph.attr("idx").map(str::to_owned),
    })
}

/// Which part of the chain a shape tree belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Inherited {
    Master,
    Layout,
    Own,
}

/// What every item of one shape tree reads.
struct Context<'a> {
    /// The part whose relationships the items cite.
    part: &'a Part,
    chain: &'a Chain,
    palette: Palette<'a>,
    defaults: Option<&'a Element>,
    inherited: Inherited,
}

impl Chain {
    /// The parts of the chain, the slide first.
    fn parts(&self) -> impl Iterator<Item = &Part> {
        std::iter::once(self.slide.as_ref())
            .chain(self.layout.as_deref())
            .chain(self.master.as_deref())
    }

    /// The colour map of the slide: the master's, under the override the
    /// layout or the slide carries.
    fn colour_map(&self) -> ColourMap {
        let overridden = [Some(&self.slide), self.layout.as_ref()]
            .into_iter()
            .flatten()
            .find_map(|p| {
                p.root
                    .child(Ns::P, "clrMapOvr")?
                    .child(Ns::A, "overrideClrMapping")
            });
        overridden
            .or_else(|| self.master.as_ref()?.root.child(Ns::P, "clrMap"))
            .map_or_else(ColourMap::default, ColourMap::read)
    }

    /// The shapes of the layout and of the master that `ph` inherits from,
    /// the layout's first. The layout matches by index, then by type; the
    /// master by type.
    fn placeholders(&self, ph: &Placeholder) -> Vec<&Element> {
        let mut out = Vec::new();
        let mut layout_type = None;
        if let Some(layout) = &self.layout {
            let candidates = placeholder_shapes(layout);
            let found = candidates
                .iter()
                .find(|(c, _)| ph.index.is_some() && c.index == ph.index)
                .or_else(|| candidates.iter().find(|(c, _)| c.spelled == ph.spelled));
            if let Some((c, sp)) = found {
                layout_type = Some(c.spelled.clone());
                out.push(*sp);
            }
        }
        if let Some(master) = &self.master {
            let wanted = master_type(layout_type.as_deref().unwrap_or(&ph.spelled));
            if let Some((_, sp)) = placeholder_shapes(master)
                .into_iter()
                .find(|(c, _)| master_type(&c.spelled) == wanted)
            {
                out.push(sp);
            }
        }
        out
    }
}

/// Every placeholder of the shape tree of `part`, with its shape.
fn placeholder_shapes(part: &Part) -> Vec<(Placeholder, &Element)> {
    part.root
        .child(Ns::P, "cSld")
        .and_then(|c| c.child(Ns::P, "spTree"))
        .map(|tree| {
            tree.elements()
                .filter(|e| e.is(Ns::P, "sp"))
                .filter_map(|sp| Some((placeholder(sp)?, sp)))
                .collect()
        })
        .unwrap_or_default()
}

/// The type a master matches a placeholder by: a centred title is a title,
/// a subtitle and an object are a body.
fn master_type(spelled: &str) -> &str {
    match spelled {
        "ctrTitle" => "title",
        "subTitle" | "obj" => "body",
        other => other,
    }
}

/// The items of one slide as the reader collects them, and the ids a
/// connector names its shapes by.
#[derive(Default)]
struct SlideOut {
    items: Vec<Item>,
    ids: Vec<(String, Pos<SlideItems>)>,
    /// The text of the title placeholder, the slide's name when its own
    /// carries none.
    title: Option<String>,
}

impl SlideOut {
    /// Add `item`, and answer its position.
    fn push(&mut self, item: Item) -> Pos<SlideItems> {
        let at = Pos::<SlideItems>::from_usize(self.items.len());
        self.items.push(item);
        at
    }
}

/// The box of an item: its corner, its size, its rotation and its flips.
#[derive(Clone, Copy, Debug)]
struct Frame {
    x: PrintLength<Pt>,
    y: PrintLength<Pt>,
    width: PrintLength<Pt>,
    height: PrintLength<Pt>,
    rotation: Radians,
    flip: Flip,
}

/// Whether an item is mirrored across each axis.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Flip {
    horizontal: Mirror,
    vertical: Mirror,
}

/// Whether an item is mirrored across one axis.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Mirror {
    #[default]
    Kept,
    Mirrored,
}

impl Mirror {
    /// The mirror the flip attribute `name` of `xfrm` names.
    fn of(xfrm: &Element, name: &str) -> Self {
        if xfrm.attr(name) == Some("1") {
            Self::Mirrored
        } else {
            Self::Kept
        }
    }
}

impl Frame {
    /// The box `xfrm`, an `a:xfrm` or a `p:xfrm`, holds.
    fn of(xfrm: &Element) -> Option<Self> {
        let off = xfrm.child(Ns::A, "off")?;
        let ext = xfrm.child(Ns::A, "ext")?;
        let rotation = xfrm
            .attr("rot")
            .and_then(|r| r.parse::<f64>().ok())
            .filter(|r| r.is_finite())
            .map_or(Radians::ZERO, |r| Radians::new((r / 60_000.0).to_radians()));
        Some(Self {
            x: length(off, "x")?,
            y: length(off, "y")?,
            width: length(ext, "cx")?.max(PrintLength::<Pt>::ZERO),
            height: length(ext, "cy")?.max(PrintLength::<Pt>::ZERO),
            rotation,
            flip: Flip {
                horizontal: Mirror::of(xfrm, "flipH"),
                vertical: Mirror::of(xfrm, "flipV"),
            },
        })
    }

    fn placement(&self) -> Placement {
        Placement {
            origin: SlidePoint {
                x: self.x,
                y: self.y,
            },
            width: self.width,
            height: self.height,
            rotation: self.rotation,
        }
    }

    /// The two ends of a line drawn corner to corner in this box, the
    /// flips applied.
    fn ends(&self) -> (SlidePoint, SlidePoint) {
        let (left, right) = (
            self.x,
            PrintLength::<Pt>::new(self.x.get() + self.width.get()),
        );
        let (top, bottom) = (
            self.y,
            PrintLength::<Pt>::new(self.y.get() + self.height.get()),
        );
        let (x0, x1) = match self.flip.horizontal {
            Mirror::Mirrored => (right, left),
            Mirror::Kept => (left, right),
        };
        let (y0, y1) = match self.flip.vertical {
            Mirror::Mirrored => (bottom, top),
            Mirror::Kept => (top, bottom),
        };
        (SlidePoint { x: x0, y: y0 }, SlidePoint { x: x1, y: y1 })
    }

    /// A line across this box. The model's line runs from the top-left
    /// corner; a flipped one is a path of its two ends.
    fn line(&self) -> Geometry {
        if self.flip.horizontal == self.flip.vertical {
            return Geometry::Line;
        }
        let (start, end) = self.ends();
        Geometry::Freeform(vec![PathSegment::MoveTo(start), PathSegment::LineTo(end)])
    }
}

/// The map from the coordinates of a group's children onto the slide: a
/// scale and an offset per axis.
#[derive(Clone, Copy, Debug)]
struct Transform {
    scale_x: f32,
    scale_y: f32,
    dx: PrintLength<Pt>,
    dy: PrintLength<Pt>,
}

impl Transform {
    const IDENTITY: Self = Self {
        scale_x: 1.0,
        scale_y: 1.0,
        dx: PrintLength::<Pt>::ZERO,
        dy: PrintLength::<Pt>::ZERO,
    };

    /// The map a group's `a:xfrm` makes from its child space, `chOff` and
    /// `chExt`, onto its own box, `off` and `ext`.
    fn group(xfrm: &Element) -> Self {
        let pair = |name: &str, x: &str, y: &str| {
            let e = xfrm.child(Ns::A, name)?;
            Some((length(e, x)?.get(), length(e, y)?.get()))
        };
        let (Some((ox, oy)), Some((ew, eh)), Some((cx, cy)), Some((cw, ch))) = (
            pair("off", "x", "y"),
            pair("ext", "cx", "cy"),
            pair("chOff", "x", "y"),
            pair("chExt", "cx", "cy"),
        ) else {
            return Self::IDENTITY;
        };
        let ratio = |own: f32, child: f32| if child > 0.0 { own / child } else { 1.0 };
        let (sx, sy) = (ratio(ew, cw), ratio(eh, ch));
        Self {
            scale_x: sx,
            scale_y: sy,
            dx: PrintLength::<Pt>::new(ox - cx * sx),
            dy: PrintLength::<Pt>::new(oy - cy * sy),
        }
    }

    /// This map, after `inner`: a child of a nested group goes through the
    /// inner group first.
    fn then(self, inner: Self) -> Self {
        Self {
            scale_x: self.scale_x * inner.scale_x,
            scale_y: self.scale_y * inner.scale_y,
            dx: PrintLength::<Pt>::new(self.dx.get() + self.scale_x * inner.dx.get()),
            dy: PrintLength::<Pt>::new(self.dy.get() + self.scale_y * inner.dy.get()),
        }
    }

    fn frame(self, frame: Frame) -> Frame {
        Frame {
            x: PrintLength::<Pt>::new(self.dx.get() + frame.x.get() * self.scale_x),
            y: PrintLength::<Pt>::new(self.dy.get() + frame.y.get() * self.scale_y),
            width: PrintLength::<Pt>::new(frame.width.get() * self.scale_x),
            height: PrintLength::<Pt>::new(frame.height.get() * self.scale_y),
            ..frame
        }
    }
}

/// The path of a custom geometry, in slide coordinates, and whether every
/// step of it is exact. An arc lands as a line to its end.
fn custom_path(custom: &Element, frame: &Frame) -> Option<(Vec<PathSegment>, bool)> {
    let mut out = Vec::new();
    let mut exact = true;
    for path in custom
        .child(Ns::A, "pathLst")?
        .elements()
        .filter(|e| e.is(Ns::A, "path"))
    {
        let units = |name: &str, side: PrintLength<Pt>| {
            path.attr(name)
                .and_then(|v| v.parse::<f32>().ok())
                .filter(|v| v.is_finite() && *v > 0.0)
                .map_or(1.0, |v| side.get() / v)
        };
        // A path names its own coordinate space, `w` by `h`; zero maps it
        // in English Metric Units.
        let sx = if path.attr("w").is_some() {
            units("w", frame.width)
        } else {
            1.0 / EMU_PER_PT
        };
        let sy = if path.attr("h").is_some() {
            units("h", frame.height)
        } else {
            1.0 / EMU_PER_PT
        };
        let point = |pt: &Element| -> Option<SlidePoint> {
            let x = pt
                .attr("x")?
                .parse::<f32>()
                .ok()
                .filter(|v| v.is_finite())?;
            let y = pt
                .attr("y")?
                .parse::<f32>()
                .ok()
                .filter(|v| v.is_finite())?;
            Some(SlidePoint {
                x: PrintLength::<Pt>::new(frame.x.get() + x * sx),
                y: PrintLength::<Pt>::new(frame.y.get() + y * sy),
            })
        };
        let points = |step: &Element| -> Vec<SlidePoint> {
            step.elements()
                .filter(|e| e.is(Ns::A, "pt"))
                .filter_map(point)
                .collect()
        };
        for step in path.elements() {
            let pts = points(step);
            match (step.name(), pts.as_slice()) {
                ("moveTo", [p]) => out.push(PathSegment::MoveTo(*p)),
                ("lnTo", [p]) => out.push(PathSegment::LineTo(*p)),
                ("quadBezTo", [control, end]) => out.push(PathSegment::QuadTo {
                    control: *control,
                    end: *end,
                }),
                ("cubicBezTo", [first, second, end]) => out.push(PathSegment::CubicTo {
                    first: *first,
                    second: *second,
                    end: *end,
                }),
                ("close", []) => out.push(PathSegment::Close),
                _ => exact = false,
            }
        }
    }
    (!out.is_empty()).then_some((out, exact))
}

#[cfg(test)]
mod tests;
