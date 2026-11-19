//! The OpenDocument presentation reader (ODF 1.3, part 3, sections 9 and
//! 10). It reads an `.odp` package into the [`Deck`] the PowerPoint
//! reader answers. The bounds and the report are the same, so one map lays
//! both onto the board.
//!
//! The reader flattens the inheritance of a presentation as the PowerPoint
//! reader does. Each page carries what its master page draws, and the
//! background its chain gives it. Each shape takes each property from the
//! first style of its chain that sets it. Each length lands on the grid of
//! English Metric Units. Each turn lands on the grid of a 60 000th of a
//! degree. A file thus reads as its PowerPoint twin reads.
//!
//! | In the file | In the deck | Outcome in the report |
//! | :--- | :--- | :--- |
//! | a rectangle, an ellipse, a custom shape, a text frame | a shape with its text | carried |
//! | a path, a polygon, a polyline | a freeform shape | carried |
//! | a picture, PNG or JPEG | a picture | carried |
//! | a connector | a connector bound to its shapes | carried |
//! | the notes page | the notes of its slide | carried |
//! | a group | its members, ungrouped | approximated |
//! | a text of several styles | the style of its first run | approximated |
//! | a gradient, hatch or bitmap fill | its first colour | approximated |
//! | an outline the model lacks, an arc of a path | its bounding box, a line | approximated |
//! | a video | its poster frame | approximated |
//! | a transition | nothing: the deck plays its own | approximated |
//! | an embedded object | its replacement picture | approximated |
//! | a chart, a table | the picture of its replacement, or nothing | approximated or dropped |
//! | an animation, an audio clip | nothing | dropped |
//! | a picture in another encoding | nothing | dropped |

use std::collections::BTreeMap;

use ambre_core::{Count, Octets};
use ambre_core::{PrintLength, Pt, Radians, Rgba, Slant, Strikethrough, Underline, Weight};

use super::style::RunStyle;
use super::xml::{Element, Ns};
use super::{
    DEFAULT_INSET, DEFAULT_TEXT_SIZE, DeckItems, EMU_PER_PT, Flip, Frame, MAX_PICTURE_BYTES,
    Mirror, SlideOut, charge, picture_format, same_look, unreadable,
};
use crate::PptxError;
use crate::model::{
    Attachment, Cap, Connector, ConnectorEnd, Deck, Geometry, ImportedDeck, Item, Line,
    PathSegment, Picture, Route, Shape, Site, Slide, SlidePoint, SlideSize, TextAlign, TextAnchor,
    TextBody,
};
use crate::opc::odf::{self, OdfPackage, OdfType};
use crate::opc::{OpcError, PartName};
use crate::report::{DeckConstruct, DeckOutcome, DeckReport};

/// The deck an OpenDocument presentation holds, and the report of what it
/// lost. The deck is the one the PowerPoint reader answers for the same
/// presentation saved as `.pptx`.
///
/// # Errors
///
/// * [`PptxError::Package`] for bytes that open as no package, or past a
///   bound of the package layer;
/// * [`PptxError::NoPresentation`] for a package of another document, or one
///   with no content part;
/// * [`PptxError::Unreadable`] for a part that is no well-formed XML;
/// * [`PptxError::NoSlide`] for a presentation of no page;
/// * [`PptxError::TooLarge`] past the bounds of a flattened deck.
pub fn read_odp(bytes: &[u8]) -> Result<ImportedDeck, PptxError> {
    let mut package = odf::open(bytes, OdfType::PRESENTATION).map_err(|e| match e {
        OpcError::OtherDocument | OpcError::MissingPart(_) => PptxError::NoPresentation,
        other => PptxError::Package(other),
    })?;
    let part = |name: &str| PartName::new(name).ok_or(PptxError::NoPresentation);
    let content = tree(&mut package, &part("content.xml")?)?.ok_or(PptxError::NoPresentation)?;
    let styles = tree(&mut package, &part("styles.xml")?)?;
    let index = StyleIndex::of(&content, styles.as_ref());
    let pages: Vec<&Element> = content
        .path(Ns::Office, &["body", "presentation"])
        .map(|p| p.elements().filter(|e| e.is(Ns::Draw, "page")).collect())
        .unwrap_or_default();
    let Some(first) = pages.first() else {
        return Err(PptxError::NoSlide);
    };
    let size = index.slide_size(first);
    let mut reader = OdpReader {
        package,
        index,
        report: DeckReport::default(),
        items: Count::ZERO,
        picture_bytes: Count::ZERO,
        text_bytes: Count::ZERO,
    };
    let mut slides = Vec::with_capacity(pages.len());
    for page in pages {
        slides.push(reader.slide(page)?);
    }
    Ok(ImportedDeck {
        deck: Deck { size, slides },
        report: reader.report,
    })
}

/// The tree of the part `name`, or `None` when the package holds none.
fn tree(package: &mut OdfPackage<'_>, name: &PartName) -> Result<Option<Element>, PptxError> {
    let Some(bytes) = package.part(name)? else {
        return Ok(None);
    };
    super::xml::parse(&bytes).map(Some).map_err(unreadable)
}

/// The document a style name is looked up in: an automatic style of the
/// content part is unknown to the styles part, and the reverse.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Doc {
    Content,
    Styles,
}

/// The family of a style, as `style:family` names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Family {
    Graphic,
    Presentation,
    Paragraph,
    Text,
    DrawingPage,
}

impl Family {
    /// The family `style:family` spells, if the reader follows it.
    fn of(spelled: &str) -> Option<Self> {
        match spelled {
            "graphic" => Some(Self::Graphic),
            "presentation" => Some(Self::Presentation),
            "paragraph" => Some(Self::Paragraph),
            "text" => Some(Self::Text),
            "drawing-page" => Some(Self::DrawingPage),
            _ => None,
        }
    }
}

/// The group of properties of a style one property belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Props {
    Graphic,
    Paragraph,
    Text,
    DrawingPage,
}

impl Props {
    /// The element of a style that holds the group.
    const fn element(self) -> &'static str {
        match self {
            Self::Graphic => "graphic-properties",
            Self::Paragraph => "paragraph-properties",
            Self::Text => "text-properties",
            Self::DrawingPage => "drawing-page-properties",
        }
    }
}

/// One property of a style: its group, its namespace and its name.
#[derive(Clone, Copy, Debug)]
struct Prop {
    group: Props,
    ns: Ns,
    name: &'static str,
}

impl Prop {
    const fn new(group: Props, ns: Ns, name: &'static str) -> Self {
        Self { group, ns, name }
    }
}

/// One style a shape names, where a property lookup starts.
#[derive(Clone, Copy, Debug)]
struct Layer<'a> {
    doc: Doc,
    family: Family,
    name: &'a str,
}

/// Where a style is declared: among the automatic styles of one part, or
/// among the common styles both parts see.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Scope {
    Content,
    Styles,
    Common,
}

/// A thing of the styles part a name looks up, beside the styles.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Named {
    Master,
    Layout,
    Gradient,
    Hatch,
    Font,
}

/// Every style, master page, page layout, gradient, hatch and font face of
/// the two parts, by name.
struct StyleIndex<'a> {
    styles: BTreeMap<(Scope, Family, &'a str), &'a Element>,
    named: BTreeMap<(Named, &'a str), &'a Element>,
    defaults: BTreeMap<Family, &'a Element>,
}

impl<'a> StyleIndex<'a> {
    fn of(content: &'a Element, styles: Option<&'a Element>) -> Self {
        let mut index = Self {
            styles: BTreeMap::new(),
            named: BTreeMap::new(),
            defaults: BTreeMap::new(),
        };
        let keyed = |scope: Scope, e: &'a Element| {
            let family = Family::of(e.attr_in(Ns::Style, "family")?)?;
            Some(((scope, family, e.attr_in(Ns::Style, "name")?), e))
        };
        let name = |kind: Named, ns: Ns, e: &'a Element| Some(((kind, e.attr_in(ns, "name")?), e));
        let styles_of = |root: &'a Element, name: &str| -> Vec<&'a Element> {
            root.child(Ns::Office, name)
                .map(|s| s.elements().collect())
                .unwrap_or_default()
        };
        for e in styles_of(content, "automatic-styles") {
            if e.is(Ns::Style, "style") {
                index.styles.extend(keyed(Scope::Content, e));
            }
        }
        for root in std::iter::once(content).chain(styles) {
            for decl in styles_of(root, "font-face-decls") {
                index.named.extend(name(Named::Font, Ns::Style, decl));
            }
        }
        if let Some(styles) = styles {
            for e in styles_of(styles, "automatic-styles") {
                if e.is(Ns::Style, "style") {
                    index.styles.extend(keyed(Scope::Styles, e));
                } else if e.is(Ns::Style, "page-layout") {
                    index.named.extend(name(Named::Layout, Ns::Style, e));
                }
            }
            for e in styles_of(styles, "styles") {
                match (e.ns, e.name()) {
                    (Ns::Style, "style") => index.styles.extend(keyed(Scope::Common, e)),
                    (Ns::Style, "default-style") => {
                        if let Some(family) = e.attr_in(Ns::Style, "family").and_then(Family::of) {
                            index.defaults.insert(family, e);
                        }
                    }
                    (Ns::Draw, "gradient") => {
                        index.named.extend(name(Named::Gradient, Ns::Draw, e));
                    }
                    (Ns::Draw, "hatch") => index.named.extend(name(Named::Hatch, Ns::Draw, e)),
                    _ => {}
                }
            }
            for e in styles_of(styles, "master-styles") {
                if e.is(Ns::Style, "master-page") {
                    index.named.extend(name(Named::Master, Ns::Style, e));
                }
            }
        }
        index
    }

    /// The style `name` of `family`, as `doc` sees it: its own automatic
    /// styles first, then the common styles.
    fn style(&self, doc: Doc, family: Family, name: &str) -> Option<&'a Element> {
        let automatic = match doc {
            Doc::Content => Scope::Content,
            Doc::Styles => Scope::Styles,
        };
        self.styles
            .get(&(automatic, family, name))
            .or_else(|| self.styles.get(&(Scope::Common, family, name)))
            .copied()
    }

    /// The value of `prop` that `layer` sets, through its parent styles.
    fn in_chain(&self, layer: Layer<'_>, prop: Prop) -> Option<&'a str> {
        std::iter::successors(self.style(layer.doc, layer.family, layer.name), |style| {
            style
                .attr_in(Ns::Style, "parent-style-name")
                .and_then(|parent| self.style(layer.doc, layer.family, parent))
        })
        // A chain that loops stops after 32 styles, far past any real chain.
        .take(32)
        .find_map(|style| {
            style
                .child(Ns::Style, prop.group.element())
                .and_then(|g| g.attr_in(prop.ns, prop.name))
        })
    }

    /// The value of `prop` the first of `layers` sets, or the default style
    /// of the family of the last layer, or the graphic default.
    fn first(&self, layers: &[Layer<'_>], prop: Prop) -> Option<&'a str> {
        layers
            .iter()
            .find_map(|layer| self.in_chain(*layer, prop))
            .or_else(|| {
                let family = layers.last().map_or(Family::Graphic, |l| l.family);
                [family, Family::Graphic]
                    .iter()
                    .filter_map(|f| self.defaults.get(f))
                    .find_map(|d| {
                        d.child(Ns::Style, prop.group.element())?
                            .attr_in(prop.ns, prop.name)
                    })
            })
    }

    /// The thing of the kind `kind` the file names `name`.
    fn lookup(&self, kind: Named, name: &str) -> Option<&'a Element> {
        self.named.get(&(kind, name)).copied()
    }

    /// The master page `page` names.
    fn master(&self, page: &Element) -> Option<&'a Element> {
        self.lookup(Named::Master, page.attr_in(Ns::Draw, "master-page-name")?)
    }

    /// The slide size the page layout of the master of `page` declares,
    /// held to the bounds a presentation accepts.
    fn slide_size(&self, page: &Element) -> SlideSize {
        let props = self
            .master(page)
            .and_then(|m| m.attr_in(Ns::Style, "page-layout-name"))
            .and_then(|name| self.lookup(Named::Layout, name))
            .and_then(|layout| layout.child(Ns::Style, "page-layout-properties"));
        let edge = |name: &str| props?.attr_in(Ns::Fo, name).and_then(length);
        match (edge("page-width"), edge("page-height")) {
            (Some(width), Some(height)) => SlideSize {
                width: width.clamp(SlideSize::MIN_EDGE, SlideSize::MAX_EDGE),
                height: height.clamp(SlideSize::MIN_EDGE, SlideSize::MAX_EDGE),
            },
            _ => SlideSize::WIDESCREEN,
        }
    }

    /// The face a font face declaration names, without its quotes.
    fn face(&self, layers: &[Layer<'_>]) -> Option<String> {
        let declared = self
            .first(layers, Prop::new(Props::Text, Ns::Style, "font-name"))
            .and_then(|name| self.lookup(Named::Font, name))
            .and_then(|decl| decl.attr_in(Ns::Svg, "font-family"));
        declared
            .or_else(|| self.first(layers, Prop::new(Props::Text, Ns::Fo, "font-family")))
            .map(|f| f.trim().trim_matches(['\'', '"']).to_owned())
            .filter(|f| !f.is_empty())
    }
}

/// The walk of one package: the style index, the report and the bounds.
struct OdpReader<'p, 'a> {
    package: OdfPackage<'p>,
    index: StyleIndex<'a>,
    report: DeckReport,
    /// The items the deck holds so far, against [`super::MAX_ITEMS`].
    items: Count<DeckItems>,
    /// The picture bytes the deck holds so far, against [`MAX_PICTURE_BYTES`].
    picture_bytes: Count<Octets>,
    /// The text bytes the deck holds so far, against the text bound of a
    /// flattened deck (`W303`).
    text_bytes: Count<Octets>,
}

/// Whether a frame landed a picture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Landed {
    Picture,
    Nothing,
}

/// Which page an item belongs to: a master page draws its own shapes and
/// never its placeholders.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Owner {
    Master,
    Own,
}

/// What every item of one page reads.
#[derive(Clone, Copy)]
struct PageContext<'e> {
    doc: Doc,
    owner: Owner,
    master: Option<&'e Element>,
}

impl OdpReader<'_, '_> {
    /// The page `page`, with what its master page draws.
    fn slide(&mut self, page: &Element) -> Result<Slide, PptxError> {
        let master = self.index.master(page);
        let mut out = SlideOut::default();
        if let Some(master) = master {
            let context = PageContext {
                doc: Doc::Styles,
                owner: Owner::Master,
                master: None,
            };
            self.children(master, context, &mut out)?;
        }
        let context = PageContext {
            doc: Doc::Content,
            owner: Owner::Own,
            master,
        };
        self.children(page, context, &mut out)?;
        let background = self.background(page, master);
        self.effects(page);
        let notes = page
            .child(Ns::Presentation, "notes")
            .and_then(|notes| {
                notes
                    .elements()
                    .find(|e| e.attr_in(Ns::Presentation, "class") == Some("notes"))
            })
            .and_then(|frame| frame.child(Ns::Draw, "text-box"))
            .map(|body| {
                paragraphs(body)
                    .iter()
                    .map(|p| paragraph_text(p))
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .unwrap_or_default();
        // A page LibreOffice names itself, `page1`, carries no title of the
        // user's.
        let generated = |n: &str| {
            n.strip_prefix("page")
                .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
        };
        let name = page
            .attr_in(Ns::Draw, "name")
            .filter(|n| !n.trim().is_empty() && !generated(n))
            .map(str::to_owned)
            .or(out.title)
            .unwrap_or_default();
        Ok(Slide {
            name,
            items: out.items,
            notes: notes.trim_end().to_owned(),
            background,
        })
    }

    /// The background of the page: its own drawing page style first, then
    /// its master's.
    fn background(&mut self, page: &Element, master: Option<&Element>) -> Option<Rgba> {
        let layers: Vec<Layer<'_>> = [(Doc::Content, Some(page)), (Doc::Styles, master)]
            .into_iter()
            .filter_map(|(doc, e)| {
                Some(Layer {
                    doc,
                    family: Family::DrawingPage,
                    name: e?.attr_in(Ns::Draw, "style-name")?,
                })
            })
            .collect();
        layers
            .iter()
            .find_map(|layer| self.fill(&[*layer], Props::DrawingPage))
            .flatten()
    }

    /// A transition and the animations of one page: neither reaches the
    /// deck, and the report names each.
    fn effects(&mut self, page: &Element) {
        let style = page.attr_in(Ns::Draw, "style-name").map(|name| Layer {
            doc: Doc::Content,
            family: Family::DrawingPage,
            name,
        });
        let set = |prop: Prop| {
            style
                .and_then(|l| self.index.in_chain(l, prop))
                .is_some_and(|v| v != "none")
        };
        let styled = set(Prop::new(
            Props::DrawingPage,
            Ns::Presentation,
            "transition-type",
        )) || set(Prop::new(
            Props::DrawingPage,
            Ns::Presentation,
            "transition-style",
        )) || set(Prop::new(Props::DrawingPage, Ns::Smil, "type"));
        let timing = page.child(Ns::Anim, "par");
        let filtered = timing.is_some_and(|t| t.holds(Ns::Anim, "transitionFilter"));
        if styled || filtered {
            self.report
                .record(DeckConstruct::Transition, DeckOutcome::Approximated);
        }
        let animated = timing.is_some_and(|t| {
            descendants(t).any(|e| {
                matches!(
                    e.attr_in(Ns::Presentation, "node-type"),
                    Some("main-sequence" | "interactive-sequence")
                ) && e.elements().next().is_some()
            })
        });
        if animated {
            self.report
                .record(DeckConstruct::Animation, DeckOutcome::Dropped);
        }
    }

    /// The items of the children of a page, a master page or a group.
    fn children(
        &mut self,
        parent: &Element,
        context: PageContext<'_>,
        out: &mut SlideOut,
    ) -> Result<(), PptxError> {
        for child in parent.elements() {
            if child.ns != Ns::Draw {
                continue;
            }
            match child.name() {
                "rect" | "ellipse" | "circle" | "custom-shape" | "line" | "path" | "polygon"
                | "polyline" | "regular-polygon" | "caption" => {
                    self.shape(child, child, context, out)?;
                }
                "frame" => self.frame(child, context, out)?,
                "connector" => self.connector(child, context, out)?,
                "g" => {
                    self.report
                        .record(DeckConstruct::Group, DeckOutcome::Approximated);
                    self.children(child, context, out)?;
                }
                "a" => self.children(child, context, out)?,
                _ => {}
            }
        }
        Ok(())
    }

    /// The styles a shape names: its graphic or presentation style, then
    /// the paragraph style of its text.
    fn layers<'e>(shape: &'e Element, doc: Doc) -> Vec<Layer<'e>> {
        let graphic = shape
            .attr_in(Ns::Presentation, "style-name")
            .map(|name| Layer {
                doc,
                family: Family::Presentation,
                name,
            })
            .or_else(|| {
                shape.attr_in(Ns::Draw, "style-name").map(|name| Layer {
                    doc,
                    family: Family::Graphic,
                    name,
                })
            });
        graphic.into_iter().collect()
    }

    /// A shape: its outline, its paint and its text. `holder` holds the
    /// paragraphs: the shape itself, or the text box of a frame.
    fn shape(
        &mut self,
        shape: &Element,
        holder: &Element,
        context: PageContext<'_>,
        out: &mut SlideOut,
    ) -> Result<(), PptxError> {
        let class = shape.attr_in(Ns::Presentation, "class");
        if class.is_some() && context.owner == Owner::Master {
            return Ok(());
        }
        // An empty placeholder is a prompt LibreOffice shows in its editor.
        if shape.attr_in(Ns::Presentation, "placeholder") == Some("true") {
            return Ok(());
        }
        let Some(frame) = placed(shape).or_else(|| {
            let class = class?;
            context.master?.elements().find_map(|e| {
                (e.attr_in(Ns::Presentation, "class") == Some(class))
                    .then(|| placed(e))
                    .flatten()
            })
        }) else {
            return Ok(());
        };
        let layers = Self::layers(shape, context.doc);
        let fill = self.fill(&layers, Props::Graphic).flatten();
        let line = self.line(&layers);
        let text = self.text(holder, shape, &layers, context.doc);
        if matches!(class, Some("title")) && out.title.is_none() {
            out.title = text
                .as_ref()
                .and_then(|t| t.text.lines().next().map(str::to_owned));
        }
        if class.is_some() && text.is_none() && fill.is_none() && line.is_none() {
            return Ok(());
        }
        let geometry = self.geometry(shape, &frame);
        let at = out.push(Item::Shape(Shape {
            name: shape
                .attr_in(Ns::Draw, "name")
                .unwrap_or_default()
                .to_owned(),
            at: frame.placement(),
            geometry,
            fill,
            line,
            text,
        }));
        self.count(out)?;
        if context.owner == Owner::Own
            && let Some(id) = item_id(shape)
        {
            out.ids.push((id.to_owned(), at));
        }
        Ok(())
    }

    /// The outline of `shape` in the box it fills. An outline the model
    /// lacks lands as its box, and the report names it.
    fn geometry(&mut self, shape: &Element, frame: &Frame) -> Geometry {
        let approximated = |reader: &mut Self| {
            reader
                .report
                .record(DeckConstruct::PresetShape, DeckOutcome::Approximated);
            Geometry::Rect
        };
        match shape.name() {
            // A text frame is a box.
            "frame" => Geometry::Rect,
            "rect" => match shape.attr_in(Ns::Draw, "corner-radius").and_then(length) {
                Some(radius) if radius > PrintLength::<Pt>::ZERO => Geometry::RoundRect { radius },
                _ => Geometry::Rect,
            },
            "ellipse" | "circle" => match shape.attr_in(Ns::Draw, "kind") {
                None | Some("full") => Geometry::Ellipse,
                Some(_) => approximated(self),
            },
            "line" => frame.line(),
            "path" | "polygon" | "polyline" => match self.path(shape, frame) {
                Some(path) => Geometry::Freeform(path),
                None => Geometry::Rect,
            },
            "custom-shape" => self.custom(shape, frame),
            _ => approximated(self),
        }
    }

    /// The outline of a custom shape: a preset the model holds, or the path
    /// of its enhanced geometry when every step is a plain point.
    fn custom(&mut self, shape: &Element, frame: &Frame) -> Geometry {
        let enhanced = shape.child(Ns::Draw, "enhanced-geometry");
        let kind = enhanced
            .and_then(|g| g.attr_in(Ns::Draw, "type"))
            .unwrap_or("non-primitive");
        let kind = kind.strip_prefix("ooxml-").unwrap_or(kind);
        match kind {
            "rectangle" | "rect" => Geometry::Rect,
            "round-rectangle" | "roundRect" => {
                // The modifier of the rounded rectangle of LibreOffice is the
                // radius in 21 600ths of the shorter edge, 3 600 by default.
                let adjust = enhanced
                    .and_then(|g| g.attr_in(Ns::Draw, "modifiers"))
                    .and_then(|m| m.split_whitespace().next())
                    .and_then(|v| v.parse::<f32>().ok())
                    .filter(|v| v.is_finite())
                    .unwrap_or(3_600.0);
                let shorter = frame.width.min(frame.height);
                Geometry::RoundRect {
                    radius: PrintLength::<Pt>::new(shorter.get() * adjust / 21_600.0),
                }
            }
            "ellipse" => Geometry::Ellipse,
            "isosceles-triangle" | "triangle" => Geometry::Triangle,
            "line" | "straightConnector1" => frame.line(),
            _ => match enhanced.and_then(|g| enhanced_path(g, frame)) {
                Some(path) => Geometry::Freeform(path),
                None => {
                    self.report
                        .record(DeckConstruct::PresetShape, DeckOutcome::Approximated);
                    Geometry::Rect
                }
            },
        }
    }

    /// The path of a path, a polygon or a polyline, in slide coordinates.
    /// An arc lands as a line to its end, and the report names it.
    fn path(&mut self, shape: &Element, frame: &Frame) -> Option<Vec<PathSegment>> {
        let view: Vec<f32> = shape
            .attr_in(Ns::Svg, "viewBox")
            .map(numbers)
            .unwrap_or_default();
        let [min_x, min_y, view_w, view_h] = view.as_slice() else {
            return None;
        };
        let (min_x, min_y) = (*min_x, *min_y);
        let sx = if *view_w > 0.0 {
            frame.width.get() / view_w
        } else {
            1.0
        };
        let sy = if *view_h > 0.0 {
            frame.height.get() / view_h
        } else {
            1.0
        };
        let map = |x: f32, y: f32| SlidePoint {
            x: PrintLength::<Pt>::new(frame.x.get() + (x - min_x) * sx),
            y: PrintLength::<Pt>::new(frame.y.get() + (y - min_y) * sy),
        };
        if shape.name() == "path" {
            let ReadPath { steps, exactness } = svg_path(shape.attr_in(Ns::Svg, "d")?, map);
            if exactness == Exactness::Approximated {
                self.report
                    .record(DeckConstruct::PresetShape, DeckOutcome::Approximated);
            }
            return (!steps.is_empty()).then_some(steps);
        }
        let points = numbers(shape.attr_in(Ns::Draw, "points")?);
        let (pairs, _) = points.as_chunks::<2>();
        let mut steps: Vec<PathSegment> = pairs
            .iter()
            .enumerate()
            .map(|(i, [x, y])| {
                let at = map(*x, *y);
                if i == 0 {
                    PathSegment::MoveTo(at)
                } else {
                    PathSegment::LineTo(at)
                }
            })
            .collect();
        if shape.name() == "polygon" && !steps.is_empty() {
            steps.push(PathSegment::Close);
        }
        (!steps.is_empty()).then_some(steps)
    }

    /// The fill the first of `layers` sets in `group`: `Some(None)` for no
    /// fill, `None` when none sets one. A gradient or a hatch takes its
    /// first colour, a bitmap none, and the report names each.
    fn fill(&mut self, layers: &[Layer<'_>], group: Props) -> Option<Option<Rgba>> {
        let get = |name: &'static str| self.index.first(layers, Prop::new(group, Ns::Draw, name));
        match get("fill")? {
            "none" => Some(None),
            "solid" => Some(get("fill-color").and_then(colour)),
            "gradient" => {
                let first = get("fill-gradient-name")
                    .and_then(|n| self.index.lookup(Named::Gradient, n))
                    .and_then(|g| g.attr_in(Ns::Draw, "start-color"))
                    .and_then(colour);
                self.report
                    .record(DeckConstruct::NonSolidFill, DeckOutcome::Approximated);
                Some(first)
            }
            "hatch" => {
                let first = get("fill-hatch-name")
                    .and_then(|n| self.index.lookup(Named::Hatch, n))
                    .and_then(|h| h.attr_in(Ns::Draw, "color"))
                    .and_then(colour);
                self.report
                    .record(DeckConstruct::NonSolidFill, DeckOutcome::Approximated);
                Some(first)
            }
            "bitmap" => {
                self.report
                    .record(DeckConstruct::NonSolidFill, DeckOutcome::Approximated);
                Some(None)
            }
            _ => None,
        }
    }

    /// The stroke `layers` set. `None` draws no outline. A width of zero is
    /// a hairline, which lands at the width PowerPoint gives a line with no
    /// width of its own.
    fn line(&self, layers: &[Layer<'_>]) -> Option<Line> {
        let get = |ns: Ns, name: &'static str| {
            self.index
                .first(layers, Prop::new(Props::Graphic, ns, name))
        };
        match get(Ns::Draw, "stroke") {
            None | Some("none") => None,
            Some(_) => Some(Line {
                colour: get(Ns::Svg, "stroke-color")
                    .and_then(colour)
                    .unwrap_or(Rgba::from_bits(0x0000_00FF)),
                width: get(Ns::Svg, "stroke-width")
                    .and_then(length)
                    .filter(|w| *w > PrintLength::<Pt>::ZERO)
                    .unwrap_or(PrintLength::<Pt>::new(0.75)),
            }),
        }
    }

    /// The text `holder` holds, in the style its chain gives its first run.
    /// `None` for a body with no character to show.
    fn text<'e>(
        &mut self,
        holder: &'e Element,
        shape: &'e Element,
        layers: &[Layer<'e>],
        doc: Doc,
    ) -> Option<TextBody> {
        let paragraphs = paragraphs(holder);
        let text = paragraphs
            .iter()
            .map(|p| paragraph_text(p))
            .collect::<Vec<_>>()
            .join("\n");
        if text.trim().is_empty() {
            return None;
        }
        let shape_text = shape
            .attr_in(Ns::Draw, "text-style-name")
            .map(|name| Layer {
                doc,
                family: Family::Paragraph,
                name,
            });
        let paragraph_layers = |p: &'e Element| -> Vec<Layer<'e>> {
            let own = p.attr_in(Ns::Text, "style-name").map(|name| Layer {
                doc,
                family: Family::Paragraph,
                name,
            });
            own.into_iter()
                .chain(shape_text)
                .chain(layers.iter().copied())
                .collect()
        };
        let runs: Vec<RunStyle> = paragraphs
            .iter()
            .flat_map(|p| {
                let under = paragraph_layers(p);
                runs(p)
                    .into_iter()
                    .filter(|(_, text)| !text.trim().is_empty())
                    .map(|(span, _)| {
                        let span = span.map(|name| Layer {
                            doc,
                            family: Family::Text,
                            name,
                        });
                        let all: Vec<Layer<'_>> =
                            span.into_iter().chain(under.iter().copied()).collect();
                        self.run_style(&all)
                    })
                    .collect::<Vec<_>>()
            })
            .collect();
        let first = runs.first().cloned().unwrap_or_default();
        if runs.iter().skip(1).any(|r| !same_look(r, &first)) {
            self.report
                .record(DeckConstruct::RichText, DeckOutcome::Approximated);
        }
        let first_paragraph = paragraphs
            .first()
            .map(|p| paragraph_layers(p))
            .unwrap_or_default();
        let align = match self.index.first(
            &first_paragraph,
            Prop::new(Props::Paragraph, Ns::Fo, "text-align"),
        ) {
            Some("center") => TextAlign::Centre,
            _ => TextAlign::Left,
        };
        let graphic = |ns: Ns, name: &'static str| {
            self.index
                .first(layers, Prop::new(Props::Graphic, ns, name))
        };
        let anchor = match graphic(Ns::Draw, "textarea-vertical-align") {
            Some("middle") => TextAnchor::Middle,
            _ => TextAnchor::Top,
        };
        let inset = graphic(Ns::Fo, "padding-left")
            .or_else(|| graphic(Ns::Fo, "padding"))
            .and_then(length)
            .unwrap_or(DEFAULT_INSET);
        Some(TextBody {
            text,
            typeface: first.typeface.unwrap_or_default(),
            size: first.size.unwrap_or(DEFAULT_TEXT_SIZE),
            colour: first.colour.unwrap_or(Rgba::from_bits(0x0000_00FF)),
            weight: first.weight.unwrap_or(Weight::Regular),
            slant: first.slant.unwrap_or(Slant::Upright),
            underline: first.underline.unwrap_or(Underline::Plain),
            strikethrough: first.strikethrough.unwrap_or(Strikethrough::Plain),
            anchor,
            align,
            inset,
        })
    }

    /// The look of one run, each property from the first of `layers` that
    /// sets it.
    fn run_style(&self, layers: &[Layer<'_>]) -> RunStyle {
        let get =
            |ns: Ns, name: &'static str| self.index.first(layers, Prop::new(Props::Text, ns, name));
        let drawn = |v: &str| v != "none";
        RunStyle {
            // A size on the grid of a hundredth of a point, as a run of
            // PowerPoint spells it.
            size: get(Ns::Fo, "font-size")
                .and_then(length)
                .filter(|v| *v > PrintLength::<Pt>::ZERO)
                .map(|v| {
                    PrintLength::<Pt>::new((f64::from(v.get()) * 100.0).round() as f32 / 100.0)
                }),
            weight: get(Ns::Fo, "font-weight").map(|w| match w {
                "bold" | "600" | "700" | "800" | "900" => Weight::Bold,
                _ => Weight::Regular,
            }),
            slant: get(Ns::Fo, "font-style").map(|s| match s {
                "italic" | "oblique" => Slant::Italic,
                _ => Slant::Upright,
            }),
            underline: get(Ns::Style, "text-underline-style").map(|u| {
                if drawn(u) {
                    Underline::Underlined
                } else {
                    Underline::Plain
                }
            }),
            strikethrough: get(Ns::Style, "text-line-through-style").map(|s| {
                if drawn(s) {
                    Strikethrough::Struck
                } else {
                    Strikethrough::Plain
                }
            }),
            colour: get(Ns::Fo, "color").and_then(colour),
            typeface: self.index.face(layers),
        }
    }

    /// A frame: a text box, a picture, a table, a chart, an embedded object
    /// or a media clip. Each construct the deck lacks lands as its
    /// replacement picture where the file carries one.
    fn frame(
        &mut self,
        frame: &Element,
        context: PageContext<'_>,
        out: &mut SlideOut,
    ) -> Result<(), PptxError> {
        let construct = if frame.child(Ns::Table, "table").is_some() {
            Some(DeckConstruct::Table)
        } else if let Some(object) = frame
            .child(Ns::Draw, "object")
            .or_else(|| frame.child(Ns::Draw, "object-ole"))
        {
            Some(self.object_kind(frame, object)?)
        } else if let Some(plugin) = frame.child(Ns::Draw, "plugin") {
            let mime = plugin.attr_in(Ns::Draw, "mime-type").unwrap_or_default();
            let href = plugin.attr_in(Ns::Xlink, "href").unwrap_or_default();
            let lower = href.to_ascii_lowercase();
            let audio_file = [
                ".mp3", ".wav", ".ogg", ".oga", ".m4a", ".flac", ".aac", ".wma",
            ]
            .iter()
            .any(|ext| lower.ends_with(ext));
            if mime.starts_with("audio/") || audio_file {
                self.report
                    .record(DeckConstruct::Audio, DeckOutcome::Dropped);
                return Ok(());
            }
            Some(DeckConstruct::Video)
        } else {
            None
        };
        if let Some(construct) = construct {
            let outcome = match self.picture(frame, out)? {
                Landed::Picture => DeckOutcome::Approximated,
                Landed::Nothing => DeckOutcome::Dropped,
            };
            self.report.record(construct, outcome);
            return Ok(());
        }
        if frame.child(Ns::Draw, "image").is_some() {
            if self.picture(frame, out)? == Landed::Nothing {
                self.report
                    .record(DeckConstruct::UnreadPicture, DeckOutcome::Dropped);
            }
            return Ok(());
        }
        if let Some(body) = frame.child(Ns::Draw, "text-box") {
            self.shape(frame, body, context, out)?;
        }
        Ok(())
    }

    /// What the object `object` of `frame` is. It is a chart when its class
    /// says so, or when its own content part holds one.
    fn object_kind(
        &mut self,
        frame: &Element,
        object: &Element,
    ) -> Result<DeckConstruct, PptxError> {
        if frame.attr_in(Ns::Presentation, "class") == Some("chart") {
            return Ok(DeckConstruct::Chart);
        }
        let dir = object
            .attr_in(Ns::Xlink, "href")
            .map(|href| href.trim_start_matches("./").trim_end_matches('/'))
            .filter(|dir| !dir.is_empty() && !dir.contains(':'));
        let part = dir.and_then(|dir| PartName::new(&format!("{dir}/content.xml")));
        let chart = match part {
            Some(part) => {
                tree(&mut self.package, &part)?.is_some_and(|root| root.holds(Ns::Office, "chart"))
            }
            None => false,
        };
        Ok(if chart {
            DeckConstruct::Chart
        } else {
            DeckConstruct::EmbeddedObject
        })
    }

    /// The first PNG or JPEG picture of `frame`, stretched to its box.
    /// [`Landed::Nothing`] when the frame holds none the deck can carry.
    fn picture(&mut self, frame: &Element, out: &mut SlideOut) -> Result<Landed, PptxError> {
        let Some(at) = placed(frame) else {
            return Ok(Landed::Nothing);
        };
        for image in frame.elements().filter(|e| e.is(Ns::Draw, "image")) {
            let Some(href) = image.attr_in(Ns::Xlink, "href") else {
                continue;
            };
            let name = href.trim_start_matches("./");
            // A link to another file or to the web is no part of the package.
            if name.contains(':') {
                continue;
            }
            let Some(name) = PartName::new(name) else {
                continue;
            };
            let Some(bytes) = self.package.part(&name)? else {
                continue;
            };
            let Some(format) = picture_format(&bytes) else {
                continue;
            };
            self.picture_bytes = self.picture_bytes.plus(Count::of(bytes.as_bytes().len()));
            if self.picture_bytes > MAX_PICTURE_BYTES {
                return Err(PptxError::TooLarge);
            }
            let _ = out.push(Item::Picture(Picture {
                name: frame
                    .attr_in(Ns::Draw, "name")
                    .unwrap_or_default()
                    .to_owned(),
                at: at.placement(),
                format,
                bytes: bytes.into_role(),
            }));
            self.count(out)?;
            return Ok(Landed::Picture);
        }
        Ok(Landed::Nothing)
    }

    /// A connector: its route, its two ends and the shapes they hold.
    fn connector(
        &mut self,
        connector: &Element,
        context: PageContext<'_>,
        out: &mut SlideOut,
    ) -> Result<(), PptxError> {
        let Some(frame) = placed(connector) else {
            return Ok(());
        };
        let route = match connector.attr_in(Ns::Draw, "type").unwrap_or("standard") {
            "line" => Route::Straight,
            "curve" => Route::Curved,
            _ => Route::Elbow,
        };
        let layers = Self::layers(connector, context.doc);
        let line = self.line(&layers).unwrap_or(Line {
            colour: Rgba::from_bits(0x0000_00FF),
            width: PrintLength::<Pt>::new(0.75),
        });
        let cap = |name: &'static str| match self
            .index
            .first(&layers, Prop::new(Props::Graphic, Ns::Draw, name))
        {
            Some(marker) if !marker.is_empty() => Cap::Arrow,
            _ => Cap::Plain,
        };
        let (start, end) = frame.ends();
        let attach = |shape: &str, glue: &str, at: SlidePoint| -> Option<Attachment> {
            if context.owner != Owner::Own {
                return None;
            }
            let id = connector.attr_in(Ns::Draw, shape)?;
            let item = out
                .ids
                .iter()
                .find(|(held, _)| held == id)
                .map(|(_, at)| *at)?;
            let placement = match out.items.get(item.to_usize()) {
                Some(Item::Shape(shape)) => shape.at,
                _ => return None,
            };
            let site = match connector.attr_in(Ns::Draw, glue) {
                Some("0") => Site::Top,
                Some("1") => Site::Right,
                Some("2") => Site::Bottom,
                Some("3") => Site::Left,
                _ => nearest_site(&placement, at),
            };
            Some(Attachment { item, site })
        };
        let connector = Connector {
            name: connector
                .attr_in(Ns::Draw, "name")
                .unwrap_or_default()
                .to_owned(),
            route,
            start: ConnectorEnd {
                at: start,
                attached: attach("start-shape", "start-glue-point", start),
            },
            end: ConnectorEnd {
                at: end,
                attached: attach("end-shape", "end-glue-point", end),
            },
            line,
            start_cap: cap("marker-start"),
            end_cap: cap("marker-end"),
        };
        let _ = out.push(Item::Connector(connector));
        self.count(out)
    }

    /// Count the last item of `out` in the deck, against the item and the
    /// text bounds of a flattened deck.
    fn count(&mut self, out: &SlideOut) -> Result<(), PptxError> {
        charge(&mut self.items, &mut self.text_bytes, out)
    }
}

/// The id a connector names an item by: `draw:id`, or the `xml:id` of a
/// newer writer.
fn item_id(item: &Element) -> Option<&str> {
    item.attr_in(Ns::Draw, "id")
        .or_else(|| item.attr_in(Ns::Xml, "id"))
}

/// The site of `placement` whose edge middle lies nearest `at`: where a
/// connector holds on when the file names no glue point.
fn nearest_site(placement: &crate::model::Placement, at: SlidePoint) -> Site {
    let (x, y) = (placement.origin.x.get(), placement.origin.y.get());
    let (w, h) = (placement.width.get(), placement.height.get());
    let sites = [
        (Site::Top, x + w / 2.0, y),
        (Site::Right, x + w, y + h / 2.0),
        (Site::Bottom, x + w / 2.0, y + h),
        (Site::Left, x, y + h / 2.0),
    ];
    let distance = |sx: f32, sy: f32| (sx - at.x.get()).powi(2) + (sy - at.y.get()).powi(2);
    sites
        .iter()
        .min_by(|a, b| distance(a.1, a.2).total_cmp(&distance(b.1, b.2)))
        .map_or(Site::Top, |s| s.0)
}

/// Every element under `root`, at any depth.
fn descendants(root: &Element) -> impl Iterator<Item = &Element> {
    let mut stack: Vec<&Element> = root.elements().collect();
    std::iter::from_fn(move || {
        let next = stack.pop()?;
        stack.extend(next.elements());
        Some(next)
    })
}

/// The paragraphs and headings `holder` holds, a list item's included, in
/// order.
fn paragraphs(holder: &Element) -> Vec<&Element> {
    let mut out = Vec::new();
    let mut stack: Vec<&Element> = holder.elements().collect();
    stack.reverse();
    while let Some(e) = stack.pop() {
        match (e.ns, e.name()) {
            (Ns::Text, "p" | "h") => out.push(e),
            (Ns::Text, "list" | "list-item" | "list-header") => {
                stack.extend(e.elements().collect::<Vec<_>>().into_iter().rev());
            }
            _ => {}
        }
    }
    out
}

/// The runs of a paragraph: the style of the span each one sits in, and
/// its text, white space collapsed as ODF section 6.1.2 states.
fn runs<'a>(paragraph: &'a Element) -> Vec<(Option<&'a str>, String)> {
    let mut out: Vec<(Option<&'a str>, String)> = Vec::new();
    // The open spans, each with the style it names and the index of the
    // child to read next.
    let mut stack: Vec<(&'a Element, Option<&'a str>, usize)> = vec![(paragraph, None, 0)];
    while let Some((element, style, index)) = stack.pop() {
        let Some(node) = element.children.get(index) else {
            continue;
        };
        stack.push((element, style, index + 1));
        let push = |text: &str, out: &mut Vec<(Option<&'a str>, String)>| match out.last_mut() {
            Some((held, run)) if *held == style => run.push_str(text),
            _ => out.push((style, text.to_owned())),
        };
        match node {
            super::xml::Node::Text(text) => {
                let collapsed: String = text
                    .split([' ', '\t', '\n', '\r'])
                    .collect::<Vec<_>>()
                    .join(" ");
                let mut squeezed = String::with_capacity(collapsed.len());
                for c in collapsed.chars() {
                    if !(c == ' ' && squeezed.ends_with(' ')) {
                        squeezed.push(c);
                    }
                }
                push(&squeezed, &mut out);
            }
            super::xml::Node::Element(child) if child.ns == Ns::Text => match child.name() {
                "span" => {
                    let own = child.attr_in(Ns::Text, "style-name").or(style);
                    stack.push((child, own, 0));
                }
                "a" | "meta" | "ruby" | "ruby-base" => stack.push((child, style, 0)),
                "s" => {
                    let spaces = child
                        .attr_in(Ns::Text, "c")
                        .and_then(|c| c.parse::<usize>().ok())
                        .unwrap_or(1)
                        .min(1_024);
                    push(&" ".repeat(spaces), &mut out);
                }
                "tab" => push("\t", &mut out),
                "line-break" => push("\n", &mut out),
                // A field, a date or a page number, shows the text it holds.
                "page-number" | "date" | "time" | "title" | "file-name" | "author-name"
                | "sender-name" | "page-count" => push(&child.text(), &mut out),
                _ => {}
            },
            super::xml::Node::Element(_) => {}
        }
    }
    out
}

/// The text of one paragraph, its runs joined, a space at either end
/// dropped as ODF section 6.1.2 states. A space `text:s` spells stays.
fn paragraph_text(paragraph: &Element) -> String {
    let joined: String = runs(paragraph).into_iter().map(|(_, t)| t).collect();
    joined.trim_matches(' ').to_owned()
}

/// A length ODF spells, a number and its unit, on the grid of English
/// Metric Units. A PowerPoint part spells every length on that grid, so
/// the same file saved as `.pptx` reads the same.
fn length(spelled: &str) -> Option<PrintLength<Pt>> {
    let spelled = spelled.trim();
    let split = spelled
        .find(|c: char| c.is_ascii_alphabetic() || c == '%')
        .unwrap_or(spelled.len());
    let (number, unit) = spelled.split_at(split);
    let value = number
        .trim()
        .parse::<f64>()
        .ok()
        .filter(|v| v.is_finite())?;
    let factor = match unit.trim() {
        "pt" => 1.0,
        "cm" => 72.0 / 2.54,
        "mm" => 72.0 / 25.4,
        "in" | "inch" => 72.0,
        "pc" => 12.0,
        "px" => 0.75,
        _ => return None,
    };
    let emu = (value * factor * f64::from(EMU_PER_PT)).round();
    Some(PrintLength::<Pt>::new((emu / f64::from(EMU_PER_PT)) as f32))
}

/// A colour ODF spells, `#rrggbb`, opaque.
fn colour(spelled: &str) -> Option<Rgba> {
    let hex = spelled.trim().strip_prefix('#')?;
    if hex.len() != 6 {
        return None;
    }
    u32::from_str_radix(hex, 16)
        .ok()
        .map(|rgb| Rgba::from_bits((rgb << 8) | 0xFF))
}

/// The numbers of a list ODF spells, split on white space and commas.
fn numbers(spelled: &str) -> Vec<f32> {
    spelled
        .split(|c: char| c.is_whitespace() || c == ',')
        .filter(|s| !s.is_empty())
        .filter_map(|s| s.parse::<f32>().ok())
        .filter(|v| v.is_finite())
        .collect()
}

/// The box `item` fills on the page, unrotated, and its turn about its
/// centre, clockwise. A line and a connector give their two ends, and the
/// box between them carries the flips that order them.
///
/// `draw:transform` applies its steps in the order it lists them: LibreOffice
/// writes a turn about the corner, then the place of that corner. The box
/// keeps its centre, and the turn lands on the grid of a 60 000th of a
/// degree, as a PowerPoint part spells it.
fn placed(item: &Element) -> Option<Frame> {
    let points = |spelled: &str| length(spelled).map(|l| f64::from(l.get()));
    // A point the map moved, back on the grid of English Metric Units.
    let on_grid = |points: f64| {
        let emu = (points * f64::from(EMU_PER_PT)).round();
        PrintLength::<Pt>::new((emu / f64::from(EMU_PER_PT)) as f32)
    };
    // The affine map of `draw:transform`: x' = a x + c y + e, y' = b x + d y + f.
    let mut m = [1.0_f64, 0.0, 0.0, 1.0, 0.0, 0.0];
    if let Some(spelled) = item.attr_in(Ns::Draw, "transform") {
        let mut rest = spelled;
        while let Some(open) = rest.find('(') {
            let name = rest[..open].trim().trim_start_matches(',').trim();
            let close = rest[open..].find(')')? + open;
            let args: Vec<&str> = rest[open + 1..close]
                .split(|c: char| c.is_whitespace() || c == ',')
                .filter(|s| !s.is_empty())
                .collect();
            let number = |i: usize| args.get(i).and_then(|v| v.parse::<f64>().ok());
            let len = |i: usize| {
                args.get(i)
                    .and_then(|v| points(v).or_else(|| v.parse::<f64>().ok()))
            };
            let step = match name {
                "rotate" => {
                    let angle = number(0)?;
                    let (sin, cos) = angle.sin_cos();
                    [cos, -sin, sin, cos, 0.0, 0.0]
                }
                "translate" => [1.0, 0.0, 0.0, 1.0, len(0)?, len(1).unwrap_or(0.0)],
                "scale" => {
                    let sx = number(0)?;
                    [sx, 0.0, 0.0, number(1).unwrap_or(sx), 0.0, 0.0]
                }
                "skewX" => [1.0, 0.0, number(0)?.tan(), 1.0, 0.0, 0.0],
                "skewY" => [1.0, number(0)?.tan(), 0.0, 1.0, 0.0, 0.0],
                "matrix" => [
                    number(0)?,
                    number(1)?,
                    number(2)?,
                    number(3)?,
                    len(4)?,
                    len(5)?,
                ],
                _ => return None,
            };
            // The step applies after the steps before it.
            m = [
                step[0] * m[0] + step[2] * m[1],
                step[1] * m[0] + step[3] * m[1],
                step[0] * m[2] + step[2] * m[3],
                step[1] * m[2] + step[3] * m[3],
                step[0] * m[4] + step[2] * m[5] + step[4],
                step[1] * m[4] + step[3] * m[5] + step[5],
            ];
            rest = &rest[close + 1..];
        }
    }
    if m.iter().any(|v| !v.is_finite()) {
        return None;
    }
    let apply = |x: f64, y: f64| (m[0] * x + m[2] * y + m[4], m[1] * x + m[3] * y + m[5]);
    let attr = |ns: Ns, name: &str| item.attr_in(ns, name).and_then(points);
    if let (Some(x1), Some(y1), Some(x2), Some(y2)) = (
        attr(Ns::Svg, "x1"),
        attr(Ns::Svg, "y1"),
        attr(Ns::Svg, "x2"),
        attr(Ns::Svg, "y2"),
    ) {
        let (x1, y1) = apply(x1, y1);
        let (x2, y2) = apply(x2, y2);
        let (x1, y1, x2, y2) = (on_grid(x1), on_grid(y1), on_grid(x2), on_grid(y2));
        let mirror = |flipped: bool| {
            if flipped {
                Mirror::Mirrored
            } else {
                Mirror::Kept
            }
        };
        return Some(Frame {
            x: x1.min(x2),
            y: y1.min(y2),
            width: PrintLength::<Pt>::new((x2.get() - x1.get()).abs()),
            height: PrintLength::<Pt>::new((y2.get() - y1.get()).abs()),
            rotation: Radians::ZERO,
            flip: Flip {
                horizontal: mirror(x2 < x1),
                vertical: mirror(y2 < y1),
            },
        });
    }
    let (width, height) = match (attr(Ns::Svg, "width"), attr(Ns::Svg, "height")) {
        (Some(w), Some(h)) => (w.max(0.0), h.max(0.0)),
        _ => {
            // A circle names its centre and its radius.
            let r = attr(Ns::Svg, "r")?;
            let (cx, cy) = (attr(Ns::Svg, "cx")?, attr(Ns::Svg, "cy")?);
            return Some(Frame {
                x: on_grid(cx - r),
                y: on_grid(cy - r),
                width: on_grid(2.0 * r),
                height: on_grid(2.0 * r),
                rotation: Radians::ZERO,
                flip: Flip::default(),
            });
        }
    };
    let x = attr(Ns::Svg, "x").unwrap_or(0.0);
    let y = attr(Ns::Svg, "y").unwrap_or(0.0);
    let (cx, cy) = apply(x + width / 2.0, y + height / 2.0);
    // A scale stretches the box; a turn of the map is the turn of the item,
    // counterclockwise in ODF and clockwise in the deck.
    let width = width * m[0].hypot(m[1]);
    let height = height * m[2].hypot(m[3]);
    let degrees = m[1].atan2(m[0]).to_degrees().rem_euclid(360.0);
    let sixtieths = (degrees * 60_000.0).round() % 21_600_000.0;
    let rotation = if sixtieths == 0.0 {
        Radians::ZERO
    } else {
        Radians::new((sixtieths / 60_000.0).to_radians())
    };
    let mirror = |name: &str| {
        let flipped = item
            .child(Ns::Draw, "enhanced-geometry")
            .and_then(|g| g.attr_in(Ns::Draw, name))
            == Some("true");
        if flipped {
            Mirror::Mirrored
        } else {
            Mirror::Kept
        }
    };
    let width_on_grid = on_grid(width);
    let height_on_grid = on_grid(height);
    Some(Frame {
        x: on_grid(cx - width / 2.0),
        y: on_grid(cy - height / 2.0),
        width: width_on_grid,
        height: height_on_grid,
        rotation,
        flip: Flip {
            horizontal: mirror("mirror-horizontal"),
            vertical: mirror("mirror-vertical"),
        },
    })
}

/// Whether every step of a path read as the file spells it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Exactness {
    Exact,
    /// An arc landed as a line to its end.
    Approximated,
}

/// The steps of a path the reader read, and whether each read exactly.
struct ReadPath {
    steps: Vec<PathSegment>,
    exactness: Exactness,
}

/// The steps of an SVG path, `svg:d`, each point through `map`. An arc
/// lands as a line to its end. A step that breaks the grammar ends the
/// path there.
fn svg_path(spelled: &str, map: impl Fn(f32, f32) -> SlidePoint) -> ReadPath {
    let mut out = ReadPath {
        steps: Vec::new(),
        exactness: Exactness::Exact,
    };
    let tokens = path_tokens(spelled);
    let mut rest = tokens.as_slice();
    let (mut x, mut y) = (0.0_f32, 0.0_f32);
    let (mut start_x, mut start_y) = (0.0_f32, 0.0_f32);
    // The control point a smooth curve reflects.
    let mut last_control: Option<(f32, f32)> = None;
    let mut command = 'M';
    while let Some(token) = rest.first() {
        if let PathToken::Command(c) = *token {
            command = c;
            rest = rest.get(1..).unwrap_or_default();
            if c.eq_ignore_ascii_case(&'Z') {
                out.steps.push(PathSegment::Close);
                (x, y) = (start_x, start_y);
                last_control = None;
                continue;
            }
        }
        let arity = match command.to_ascii_uppercase() {
            'M' | 'L' | 'T' => 2,
            'H' | 'V' => 1,
            'C' => 6,
            'S' | 'Q' => 4,
            'A' => 7,
            _ => return out,
        };
        let args: Vec<f32> = rest
            .iter()
            .take(arity)
            .map_while(|t| match t {
                PathToken::Number(n) => Some(*n),
                PathToken::Command(_) => None,
            })
            .collect();
        if args.len() != arity {
            return out;
        }
        rest = rest.get(arity..).unwrap_or_default();
        let relative = command.is_ascii_lowercase();
        let abs = |px: f32, py: f32| {
            if relative { (x + px, y + py) } else { (px, py) }
        };
        let reflected = last_control.map_or((x, y), |(cx, cy)| (2.0 * x - cx, 2.0 * y - cy));
        let cubic = |first: (f32, f32), second: (f32, f32), end: (f32, f32)| PathSegment::CubicTo {
            first: map(first.0, first.1),
            second: map(second.0, second.1),
            end: map(end.0, end.1),
        };
        let quad = |control: (f32, f32), end: (f32, f32)| PathSegment::QuadTo {
            control: map(control.0, control.1),
            end: map(end.0, end.1),
        };
        match (command.to_ascii_uppercase(), args.as_slice()) {
            ('M', [px, py]) => {
                (x, y) = abs(*px, *py);
                (start_x, start_y) = (x, y);
                out.steps.push(PathSegment::MoveTo(map(x, y)));
                // Pairs after a move are lines.
                command = if relative { 'l' } else { 'L' };
                last_control = None;
            }
            ('L', [px, py]) => {
                (x, y) = abs(*px, *py);
                out.steps.push(PathSegment::LineTo(map(x, y)));
                last_control = None;
            }
            ('H', [px]) => {
                x = if relative { x + px } else { *px };
                out.steps.push(PathSegment::LineTo(map(x, y)));
                last_control = None;
            }
            ('V', [py]) => {
                y = if relative { y + py } else { *py };
                out.steps.push(PathSegment::LineTo(map(x, y)));
                last_control = None;
            }
            ('C', [x1, y1, x2, y2, ex, ey]) => {
                let (first, second, end) = (abs(*x1, *y1), abs(*x2, *y2), abs(*ex, *ey));
                out.steps.push(cubic(first, second, end));
                last_control = Some(second);
                (x, y) = end;
            }
            ('S', [x2, y2, ex, ey]) => {
                let (second, end) = (abs(*x2, *y2), abs(*ex, *ey));
                out.steps.push(cubic(reflected, second, end));
                last_control = Some(second);
                (x, y) = end;
            }
            ('Q', [cx, cy, ex, ey]) => {
                let (control, end) = (abs(*cx, *cy), abs(*ex, *ey));
                out.steps.push(quad(control, end));
                last_control = Some(control);
                (x, y) = end;
            }
            ('T', [ex, ey]) => {
                let end = abs(*ex, *ey);
                out.steps.push(quad(reflected, end));
                last_control = Some(reflected);
                (x, y) = end;
            }
            ('A', [.., ex, ey]) => {
                (x, y) = abs(*ex, *ey);
                out.steps.push(PathSegment::LineTo(map(x, y)));
                out.exactness = Exactness::Approximated;
                last_control = None;
            }
            _ => return out,
        }
    }
    out
}

/// One token of an SVG path: a command letter, or a number.
#[derive(Clone, Copy, Debug, PartialEq)]
enum PathToken {
    Command(char),
    Number(f32),
}

/// The tokens of an SVG path. A number ends where a sign, a second point,
/// a letter or a separator starts the next one.
fn path_tokens(spelled: &str) -> Vec<PathToken> {
    let mut out = Vec::new();
    let mut number = String::new();
    let flush = |number: &mut String, out: &mut Vec<PathToken>| {
        if let Ok(v) = number.parse::<f32>()
            && v.is_finite()
        {
            out.push(PathToken::Number(v));
        }
        number.clear();
    };
    for c in spelled.chars() {
        match c {
            '0'..='9' => number.push(c),
            '.' => {
                if number.contains('.') && !number.contains(['e', 'E']) {
                    flush(&mut number, &mut out);
                }
                number.push(c);
            }
            '-' | '+' => {
                if !(number.ends_with('e') || number.ends_with('E')) {
                    flush(&mut number, &mut out);
                }
                number.push(c);
            }
            'e' | 'E' if !number.is_empty() => number.push(c),
            c if c.is_ascii_alphabetic() => {
                flush(&mut number, &mut out);
                out.push(PathToken::Command(c));
            }
            _ => flush(&mut number, &mut out),
        }
    }
    flush(&mut number, &mut out);
    out
}

/// The path of an enhanced geometry, in the view box of the geometry. It
/// reads when every step is a plain point: `M`, `L`, `C`, `Q`, `Z` or `N`.
/// A formula, a modifier or an arc leaves the path unread.
fn enhanced_path(geometry: &Element, frame: &Frame) -> Option<Vec<PathSegment>> {
    let view = numbers(geometry.attr_in(Ns::Svg, "viewBox")?);
    let [min_x, min_y, view_w, view_h] = view.as_slice() else {
        return None;
    };
    if *view_w <= 0.0 || *view_h <= 0.0 {
        return None;
    }
    let (sx, sy) = (frame.width.get() / view_w, frame.height.get() / view_h);
    let horizontal = frame.flip.horizontal == Mirror::Mirrored;
    let vertical = frame.flip.vertical == Mirror::Mirrored;
    let map = |x: f32, y: f32| {
        let (mut px, mut py) = ((x - min_x) * sx, (y - min_y) * sy);
        if horizontal {
            px = frame.width.get() - px;
        }
        if vertical {
            py = frame.height.get() - py;
        }
        SlidePoint {
            x: PrintLength::<Pt>::new(frame.x.get() + px),
            y: PrintLength::<Pt>::new(frame.y.get() + py),
        }
    };
    let spelled = geometry.attr_in(Ns::Draw, "enhanced-path")?;
    let mut out = Vec::new();
    let mut command = None;
    let mut pending: Vec<f32> = Vec::new();
    for token in spelled.split_whitespace() {
        if let Ok(v) = token.parse::<f32>() {
            if !v.is_finite() {
                return None;
            }
            pending.push(v);
        } else {
            match token {
                "M" | "L" | "C" | "Q" => command = token.chars().next(),
                "Z" => {
                    out.push(PathSegment::Close);
                    command = None;
                }
                "N" | "F" | "S" => command = None,
                _ => return None,
            }
            continue;
        }
        let arity = match command? {
            'C' => 6,
            'Q' => 4,
            _ => 2,
        };
        if pending.len() < arity {
            continue;
        }
        let p = std::mem::take(&mut pending);
        out.push(match (command?, p.as_slice()) {
            ('M', [x, y]) => PathSegment::MoveTo(map(*x, *y)),
            ('L', [x, y]) => PathSegment::LineTo(map(*x, *y)),
            ('C', [x1, y1, x2, y2, x, y]) => PathSegment::CubicTo {
                first: map(*x1, *y1),
                second: map(*x2, *y2),
                end: map(*x, *y),
            },
            ('Q', [cx, cy, x, y]) => PathSegment::QuadTo {
                control: map(*cx, *cy),
                end: map(*x, *y),
            },
            _ => return None,
        });
        if command == Some('M') {
            command = Some('L');
        }
    }
    (!out.is_empty() && pending.is_empty()).then_some(out)
}

#[cfg(test)]
mod tests;
