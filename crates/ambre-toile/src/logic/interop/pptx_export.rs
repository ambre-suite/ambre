//! The deck as a PowerPoint file (`toile-pptx-export`): the board side of the
//! map onto the model of `ambre-pptx`.
//!
//! The board walk hands one [`PptxSlideSource`] per slide, in play order. This
//! module fits each frame onto the slide, and maps every element onto the
//! nearest item PowerPoint holds. The report names each construct it
//! approximates or drops (`INTEROP-003`). The writer of `ambre-pptx` then
//! carries every item, so this report is the whole report.
//!
//! | On the board | In the file | Outcome |
//! | :--- | :--- | :--- |
//! | a shape, a vector path, a text | the same shape, text left editable | carried |
//! | a picture with its pixels | an embedded PNG | carried |
//! | a straight connector | a connection shape bound to its two shapes | carried |
//! | a speaker note | the notes page of its slide | carried |
//! | a sticky note | a rounded rectangle holding its text | approximated |
//! | a freehand stroke | a freeform line of one width | approximated |
//! | an elbow or a curved connector | the connector PowerPoint routes itself | approximated |
//! | a connector caption | a text box at the middle of the line | approximated |
//! | a group | its members, ungrouped | approximated |
//! | an embed, a document embed, a frame off the deck | nothing | dropped |
//! | a picture without its pixels | nothing | dropped |

use ambre_bus::interop::{Direction, FileFormat, Outcome};
use ambre_core::id::{Element, Id};
use ambre_core::index::Pos;
use ambre_core::{Board, FontFamily, PrintLength, Pt, Px, Rgba, Weight};
use ambre_pptx::model::{
    Attachment, Cap as PptxCap, Connector, ConnectorEnd, Deck, Geometry, Item, Line, PathSegment,
    Picture, Placement, Route, Shape, Site, Slide, SlideItems, SlidePoint, SlideSize, TextAlign,
    TextAnchor, TextBody,
};
use kurbo::{BezPath, PathEl, Point, Rect, Shape as _};

use super::ExportElement;
use crate::logic::model::{Anchor, ElementKind, ElementStyle, EndCaps, PathType, ShapeKind};
use crate::message::{Cap, STICKY_DEFAULT_FILL, ToileConstruct, ToileReport};
use crate::ui::label::{LABEL_H, LABEL_W};

/// The width of every slide of the deck. PowerPoint opens a new deck on it.
const SLIDE_WIDTH: PrintLength<Pt> = SlideSize::WIDESCREEN.width;

/// The ink of a text whose style names none: the theme ink of a light board.
const DEFAULT_INK: Rgba = Rgba::from_bits(0x1F29_37FF);

/// The space between the edge of a sticky note and its text, in world units:
/// the padding the board draws a note body with.
const STICKY_INSET: Px<Board> = Px::<Board>::new(12.0);

/// One slide of the board walk: the region it frames and what lies in it.
#[derive(Clone, PartialEq)]
pub struct PptxSlideSource {
    /// The world rect the slide frames: the frame of the slide, or the
    /// content of the whole board for a deck with no slide.
    pub frame: Rect,
    /// The title of the frame, empty for an untitled one.
    pub name: String,
    /// The speaker notes of the frame.
    pub notes: String,
    /// Every element that meets the frame, in paint order, bottom first.
    pub elements: Vec<PptxElement>,
    /// Every connector whose line meets the frame.
    pub connectors: Vec<PptxConnector>,
}

/// The title and the notes print as their lengths alone (`ZK-006`).
impl std::fmt::Debug for PptxSlideSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PptxSlideSource")
            .field("frame", &self.frame)
            .field("name", &ambre_core::Redacted::new(self.name.as_str()))
            .field("notes", &ambre_core::Redacted::new(self.notes.as_str()))
            .field("elements", &self.elements)
            .field("connectors", &self.connectors)
            .finish()
    }
}

/// One element of a slide, with the id a connector names it by.
#[derive(Clone, PartialEq, Debug)]
pub struct PptxElement {
    pub id: Id<Element>,
    pub element: ExportElement,
    /// For a freehand stroke: the smoothed samples, in world units. Empty for
    /// every other kind.
    pub stroke: Vec<Point>,
}

/// One connector of a slide: its two ends, its route and its look.
#[derive(Clone, PartialEq)]
pub struct PptxConnector {
    pub from: Id<Element>,
    pub from_anchor: Anchor,
    /// The world point the line starts at.
    pub start: Point,
    pub to: Id<Element>,
    pub to_anchor: Anchor,
    /// The world point the line ends at.
    pub end: Point,
    pub path: PathType,
    pub caps: EndCaps,
    pub style: ElementStyle,
    /// The caption on the line, empty for none.
    pub caption: String,
}

/// The caption prints as its length alone (`ZK-006`).
impl std::fmt::Debug for PptxConnector {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PptxConnector")
            .field("from", &self.from)
            .field("to", &self.to)
            .field("path", &self.path)
            .field("caption", &ambre_core::Redacted::new(self.caption.as_str()))
            .finish_non_exhaustive()
    }
}

/// The deck to write, and what the map could not carry whole.
#[derive(Clone, Debug, PartialEq)]
pub struct PptxExport {
    pub deck: Deck,
    pub report: ToileReport,
}

/// The deck of `slides`, in their order, and its report.
///
/// The first slide sets the slide size: 960 pt wide, at the ratio of its
/// frame. Every frame then fits into that slide, centred.
#[must_use]
pub fn export_pptx(slides: &[PptxSlideSource]) -> PptxExport {
    let size = slides.first().map_or(SlideSize::WIDESCREEN, |first| {
        SlideSize::with_aspect_of(
            SLIDE_WIDTH,
            SlideSize {
                width: narrow(first.frame.width()),
                height: narrow(first.frame.height()),
            },
        )
    });
    let mut report = ToileReport::new(Direction::Export, FileFormat::Pptx);
    let slides = slides
        .iter()
        .map(|source| map_slide(source, size, &mut report))
        .collect();
    PptxExport {
        deck: Deck { size, slides },
        report,
    }
}

/// How a frame lands on a slide: one uniform scale, and the offset that
/// centres it.
#[derive(Clone, Copy)]
struct Fit {
    /// The world corner of the frame.
    origin: Point,
    /// Points per world unit.
    scale: f64,
    /// Where the corner of the frame lands on the slide, in points.
    offset: kurbo::Vec2,
}

impl Fit {
    fn new(frame: Rect, size: SlideSize) -> Self {
        let (w, h) = (f64::from(size.width.get()), f64::from(size.height.get()));
        let scale = (w / frame.width()).min(h / frame.height());
        let scale = if scale.is_finite() && scale > 0.0 {
            scale
        } else {
            1.0
        };
        Self {
            origin: frame.origin(),
            scale,
            offset: kurbo::Vec2::new(
                (w - frame.width() * scale) / 2.0,
                (h - frame.height() * scale) / 2.0,
            ),
        }
    }

    fn point(&self, p: Point) -> SlidePoint {
        let at = (p - self.origin) * self.scale + self.offset;
        SlidePoint {
            x: narrow(at.x),
            y: narrow(at.y),
        }
    }

    fn length(&self, world: Px<Board>) -> PrintLength<Pt> {
        narrow(world.get() * self.scale)
    }

    fn placement(&self, rect: Rect, rotation: ambre_core::Radians) -> Placement {
        Placement {
            origin: self.point(rect.origin()),
            width: self.length(Px::<Board>::new(rect.width())),
            height: self.length(Px::<Board>::new(rect.height())),
            rotation,
        }
    }
}

/// A print length of `value` points. A slide is at most 4 032 pt, so the
/// narrowing loses nothing a reader sees.
#[expect(
    clippy::cast_possible_truncation,
    reason = "a slide coordinate fits an f32"
)]
fn narrow(value: f64) -> PrintLength<Pt> {
    PrintLength::<Pt>::new(value as f32)
}

/// One slide: its elements, then its connectors, then their captions.
fn map_slide(source: &PptxSlideSource, size: SlideSize, report: &mut ToileReport) -> Slide {
    let fit = Fit::new(source.frame, size);
    let mut items = Vec::new();
    // Where each element landed among the items, for the connectors that
    // name it.
    let mut placed: Vec<(&Id<Element>, Pos<SlideItems>)> = Vec::new();
    for el in &source.elements {
        if let Some(item) = map_element(el, &fit, report) {
            placed.push((&el.id, Pos::<SlideItems>::from_usize(items.len())));
            items.push(item);
        }
    }
    let mut captions = Vec::new();
    for connector in &source.connectors {
        let attach = |id: &Id<Element>, anchor: Anchor| {
            let item = placed.iter().find(|(e, _)| *e == id).map(|(_, at)| *at)?;
            Some(Attachment {
                item,
                site: site_of(anchor)?,
            })
        };
        let route = match connector.path {
            PathType::Straight => Route::Straight,
            PathType::Elbow => Route::Elbow,
            PathType::Curved => Route::Curved,
        };
        let outcome = match route {
            Route::Straight => Outcome::Carried,
            Route::Elbow | Route::Curved => Outcome::Approximated,
        };
        report.record(ToileConstruct::Connector, outcome);
        let style = &connector.style;
        items.push(Item::Connector(Connector {
            name: "Connector".to_owned(),
            route,
            start: ConnectorEnd {
                at: fit.point(connector.start),
                attached: attach(&connector.from, connector.from_anchor),
            },
            end: ConnectorEnd {
                at: fit.point(connector.end),
                attached: attach(&connector.to, connector.to_anchor),
            },
            line: Line {
                colour: faded(style.stroke.unwrap_or(DEFAULT_INK), style),
                width: fit.length(Px::<Board>::new(style.stroke_w.widened().get().max(1.0))),
            },
            start_cap: cap(connector.caps.start),
            end_cap: cap(connector.caps.end),
        }));
        if !connector.caption.trim().is_empty() {
            report.record(ToileConstruct::ConnectorLabel, Outcome::Approximated);
            captions.push(caption_box(connector, &fit));
        }
    }
    items.extend(captions);
    if !source.notes.trim().is_empty() {
        report.record(ToileConstruct::SpeakerNotes, Outcome::Carried);
    }
    Slide {
        name: source.name.clone(),
        items,
        notes: source.notes.clone(),
    }
}

/// The site of a box where a connector holds on, for an anchor that names
/// one. The centre and a free point hold on nowhere PowerPoint knows, so
/// the end stays where it lies.
const fn site_of(anchor: Anchor) -> Option<Site> {
    match anchor {
        Anchor::Top => Some(Site::Top),
        Anchor::Left => Some(Site::Left),
        Anchor::Bottom => Some(Site::Bottom),
        Anchor::Right => Some(Site::Right),
        Anchor::Center | Anchor::Free { .. } => None,
    }
}

const fn cap(cap: Cap) -> PptxCap {
    match cap {
        Cap::Plain => PptxCap::Plain,
        Cap::Arrow => PptxCap::Arrow,
    }
}

/// The caption of a connector, as a text box centred on the middle of its
/// line.
fn caption_box(connector: &PptxConnector, fit: &Fit) -> Item {
    let middle = connector.start.midpoint(connector.end);
    let rect = Rect::from_center_size(middle, (LABEL_W.get(), LABEL_H.get()));
    Item::Shape(Shape {
        name: "Connector label".to_owned(),
        at: fit.placement(rect, ambre_core::Radians::ZERO),
        geometry: Geometry::Rect,
        fill: None,
        line: None,
        text: Some(text_body(
            connector.caption.clone(),
            &connector.style,
            fit,
            TextLayout {
                anchor: TextAnchor::Middle,
                align: TextAlign::Centre,
                inset: PrintLength::<Pt>::ZERO,
            },
        )),
    })
}

/// The item of one element, or `None` for a kind the file cannot hold. The
/// report names each one either way but the plain carried shape.
fn map_element(el: &PptxElement, fit: &Fit, report: &mut ToileReport) -> Option<Item> {
    let e = &el.element;
    let at = fit.placement(e.rect, e.rotation);
    let style = &e.style;
    let outline = || {
        style.stroke.map(|colour| Line {
            colour: faded(colour, style),
            width: fit.length(style.stroke_w.widened()),
        })
    };
    let fill = || style.fill.map(|colour| faded(colour, style));
    let shape = |name: &str, geometry: Geometry| Shape {
        name: name.to_owned(),
        at,
        geometry,
        fill: fill(),
        line: outline(),
        text: None,
    };
    let item = match e.kind {
        ElementKind::Shape(ShapeKind::Rect) => {
            let corner = style.corner.widened();
            let geometry = if corner.get() > 0.0 {
                Geometry::RoundRect {
                    radius: fit.length(corner),
                }
            } else {
                Geometry::Rect
            };
            Item::Shape(shape("Rectangle", geometry))
        }
        ElementKind::Shape(ShapeKind::Ellipse) => Item::Shape(shape("Ellipse", Geometry::Ellipse)),
        ElementKind::Shape(ShapeKind::Triangle) => {
            Item::Shape(shape("Triangle", Geometry::Triangle))
        }
        // A line has no inside to fill.
        ElementKind::Shape(ShapeKind::Line) => Item::Shape(Shape {
            fill: None,
            ..shape("Line", Geometry::Line)
        }),
        ElementKind::Shape(ShapeKind::Path) => {
            let path = e.path.as_ref()?;
            Item::Shape(Shape {
                at: fit.placement(path.bounding_box(), e.rotation),
                ..shape("Path", Geometry::Freeform(segments(path, fit)))
            })
        }
        ElementKind::Text => Item::Shape(Shape {
            name: "Text".to_owned(),
            at,
            geometry: Geometry::Rect,
            fill: None,
            line: None,
            text: Some(text_body(
                e.text.clone().unwrap_or_default(),
                style,
                fit,
                TextLayout {
                    anchor: TextAnchor::Top,
                    align: TextAlign::Left,
                    inset: PrintLength::<Pt>::ZERO,
                },
            )),
        }),
        ElementKind::Sticky => {
            report.record(ToileConstruct::StickyNote, Outcome::Approximated);
            Item::Shape(Shape {
                name: "Sticky note".to_owned(),
                at,
                geometry: Geometry::RoundRect {
                    radius: fit.length(style.corner.widened()),
                },
                fill: Some(faded(style.fill.unwrap_or(STICKY_DEFAULT_FILL), style)),
                line: None,
                text: Some(text_body(
                    e.text.clone().unwrap_or_default(),
                    style,
                    fit,
                    TextLayout {
                        anchor: TextAnchor::Middle,
                        align: TextAlign::Centre,
                        inset: fit.length(STICKY_INSET),
                    },
                )),
            })
        }
        ElementKind::Drawing => {
            let mut path = BezPath::new();
            let mut points = el.stroke.iter().copied();
            let first = points.next()?;
            path.move_to(first);
            points.for_each(|p| path.line_to(p));
            report.record(ToileConstruct::FreehandStroke, Outcome::Approximated);
            Item::Shape(Shape {
                name: "Freehand stroke".to_owned(),
                at: fit.placement(path.bounding_box(), e.rotation),
                geometry: Geometry::Freeform(segments(&path, fit)),
                fill: None,
                line: Some(Line {
                    colour: faded(style.stroke.unwrap_or(DEFAULT_INK), style),
                    width: fit.length(Px::<Board>::new(style.stroke_w.widened().get().max(1.0))),
                }),
                text: None,
            })
        }
        ElementKind::Image => {
            let Some(png) = &e.image_png else {
                report.record(ToileConstruct::Picture, Outcome::Dropped);
                return None;
            };
            Item::Picture(Picture {
                name: "Picture".to_owned(),
                at,
                png: png.clone().into_role(),
            })
        }
        ElementKind::Group => {
            // The members land one by one, ungrouped.
            report.record(ToileConstruct::Group, Outcome::Approximated);
            return None;
        }
        ElementKind::Frame | ElementKind::Embed | ElementKind::DocEmbed => {
            let construct = match e.kind {
                ElementKind::Frame => ToileConstruct::Frame,
                ElementKind::Embed => ToileConstruct::Embed,
                _ => ToileConstruct::EmbeddedDocument,
            };
            report.record(construct, Outcome::Dropped);
            return None;
        }
    };
    report.record(ToileConstruct::Element, Outcome::Carried);
    Some(item)
}

/// The steps of a path, in slide points.
fn segments(path: &BezPath, fit: &Fit) -> Vec<PathSegment> {
    path.elements()
        .iter()
        .map(|el| match *el {
            PathEl::MoveTo(p) => PathSegment::MoveTo(fit.point(p)),
            PathEl::LineTo(p) => PathSegment::LineTo(fit.point(p)),
            PathEl::QuadTo(c, p) => PathSegment::QuadTo {
                control: fit.point(c),
                end: fit.point(p),
            },
            PathEl::CurveTo(a, b, p) => PathSegment::CubicTo {
                first: fit.point(a),
                second: fit.point(b),
                end: fit.point(p),
            },
            PathEl::ClosePath => PathSegment::Close,
        })
        .collect()
}

/// Where a text sits in its box.
#[derive(Clone, Copy)]
struct TextLayout {
    anchor: TextAnchor,
    align: TextAlign,
    inset: PrintLength<Pt>,
}

/// The text `text` in the face, the size, the ink and the emphasis of
/// `style`.
fn text_body(text: String, style: &ElementStyle, fit: &Fit, layout: TextLayout) -> TextBody {
    let font = &style.font;
    TextBody {
        text,
        typeface: typeface(font.family.as_ref()),
        size: text_size(style, fit),
        colour: style.text_color.unwrap_or(DEFAULT_INK),
        weight: if font.weight.is_bold() {
            Weight::Bold
        } else {
            Weight::Regular
        },
        slant: font.italic,
        underline: font.underline,
        strikethrough: font.strikethrough,
        anchor: layout.anchor,
        align: layout.align,
        inset: layout.inset,
    }
}

/// The face PowerPoint draws a family in. The bundled faces of the board
/// are rare on a desktop, so each takes the common face of its kind. The
/// sans takes the face of the theme.
fn typeface(family: Option<&FontFamily>) -> String {
    match family {
        None | Some(FontFamily::Sans) => String::new(),
        Some(FontFamily::Serif) => "Times New Roman".to_owned(),
        Some(FontFamily::Mono) => "Courier New".to_owned(),
        Some(FontFamily::Other(name)) => name.clone(),
    }
}

/// The size a text is drawn at, on the slide. An automatic size is the
/// default the board draws a text at.
fn text_size(style: &ElementStyle, fit: &Fit) -> PrintLength<Pt> {
    fit.length(Px::<Board>::new(f64::from(
        style.font.size.or_default().to_px().get(),
    )))
}

/// `colour` under the opacity of `style`.
fn faded(colour: Rgba, style: &ElementStyle) -> Rgba {
    let [r, g, b, a] = colour.channels();
    let opacity = style.opacity.get().clamp(0.0, 1.0);
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "the product lies in 0 ..= 255"
    )]
    let alpha = (f32::from(a) * opacity).round() as u8;
    Rgba::from_channels(r, g, b, alpha)
}

/// The count of occurrences a report entry reads, for the tests.
#[cfg(test)]
fn count(report: &ToileReport, construct: ToileConstruct, outcome: Outcome) -> u32 {
    report.count(construct, outcome).get()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pt(value: f64) -> PrintLength<Pt> {
        narrow(value)
    }

    fn element(kind: ElementKind, rect: Rect) -> PptxElement {
        PptxElement {
            id: Id::<Element>::from_text(format!("e{}", rect.x0)),
            element: ExportElement::new(kind, rect),
            stroke: Vec::new(),
        }
    }

    fn source(frame: Rect, elements: Vec<PptxElement>) -> PptxSlideSource {
        PptxSlideSource {
            frame,
            name: String::new(),
            notes: String::new(),
            elements,
            connectors: Vec::new(),
        }
    }

    fn shape_at(item: &Item) -> Placement {
        match item {
            Item::Shape(s) => s.at,
            Item::Picture(p) => p.at,
            Item::Connector(_) => Placement::default(),
        }
    }

    #[test]
    fn a_frame_fills_the_slide_and_its_content_lands_in_place() {
        let frame = Rect::new(100.0, 100.0, 1060.0, 640.0);
        let export = export_pptx(&[source(
            frame,
            vec![element(
                ElementKind::Shape(ShapeKind::Rect),
                Rect::new(100.0, 100.0, 200.0, 150.0),
            )],
        )]);
        assert_eq!(export.deck.size, SlideSize::WIDESCREEN);
        let item = export
            .deck
            .slides
            .first()
            .and_then(|s| s.items.first())
            .expect("one item");
        let at = shape_at(item);
        assert_eq!(
            at.origin,
            SlidePoint::default(),
            "the frame corner is the slide corner"
        );
        assert_eq!(at.width, pt(100.0));
        assert_eq!(at.height, pt(50.0));
    }

    #[test]
    fn a_frame_of_another_ratio_lands_centred() {
        let first = Rect::new(0.0, 0.0, 1600.0, 900.0);
        let square = Rect::new(0.0, 0.0, 900.0, 900.0);
        let export = export_pptx(&[
            source(first, Vec::new()),
            source(
                square,
                vec![element(ElementKind::Shape(ShapeKind::Rect), square)],
            ),
        ]);
        let item = export
            .deck
            .slides
            .get(1)
            .and_then(|s| s.items.first())
            .expect("one item");
        let at = shape_at(item);
        assert_eq!(at.height, pt(540.0));
        assert_eq!(at.origin.x, pt(210.0), "the square sits in the middle");
    }

    #[test]
    fn a_text_stays_text_with_its_size_on_the_slide() {
        let mut text = element(ElementKind::Text, Rect::new(0.0, 0.0, 200.0, 40.0));
        text.element.text = Some("Hello".to_owned());
        let export = export_pptx(&[source(Rect::new(0.0, 0.0, 1920.0, 1080.0), vec![text])]);
        let Some(Item::Shape(shape)) = export.deck.slides.first().and_then(|s| s.items.first())
        else {
            panic!("a text is a shape");
        };
        let body = shape.text.as_ref().expect("the text is kept");
        assert_eq!(body.text, "Hello");
        assert!(
            shape.fill.is_none() && shape.line.is_none(),
            "a bare text box"
        );
        let board = f64::from(ElementStyle::default().font.size.or_default().to_px().get());
        assert_eq!(body.size, pt(board / 2.0), "the slide is half the frame");
        assert_eq!(
            body.typeface, "",
            "the default family takes the face of the theme"
        );
        assert_eq!(body.weight, Weight::Regular);
        assert_eq!(
            count(&export.report, ToileConstruct::Element, Outcome::Carried),
            1
        );
    }

    #[test]
    fn a_text_keeps_its_face_and_its_emphasis() {
        let mut text = element(ElementKind::Text, Rect::new(0.0, 0.0, 200.0, 40.0));
        text.element.text = Some("Loud".to_owned());
        let font = &mut text.element.style.font;
        font.family = Some(FontFamily::Mono);
        font.weight = ambre_core::Bounded::<ambre_core::FontWeight>::BOLD;
        font.italic = ambre_core::Slant::Italic;
        font.underline = ambre_core::Underline::Underlined;
        font.strikethrough = ambre_core::Strikethrough::Struck;
        let export = export_pptx(&[source(Rect::new(0.0, 0.0, 960.0, 540.0), vec![text])]);
        let Some(Item::Shape(shape)) = export.deck.slides.first().and_then(|s| s.items.first())
        else {
            panic!("a text is a shape");
        };
        let body = shape.text.as_ref().expect("the text is kept");
        assert_eq!(body.typeface, "Courier New");
        assert_eq!(body.weight, Weight::Bold);
        assert_eq!(body.slant, ambre_core::Slant::Italic);
        assert_eq!(body.underline, ambre_core::Underline::Underlined);
        assert_eq!(body.strikethrough, ambre_core::Strikethrough::Struck);
    }

    #[test]
    fn a_sticky_and_a_stroke_are_approximated_and_named() {
        let mut sticky = element(ElementKind::Sticky, Rect::new(0.0, 0.0, 100.0, 100.0));
        sticky.element.text = Some("Idea".to_owned());
        // A note that stores no colour takes the yellow the board paints.
        sticky.element.style.fill = None;
        let mut stroke = element(ElementKind::Drawing, Rect::new(10.0, 10.0, 60.0, 60.0));
        stroke.stroke = vec![Point::new(10.0, 10.0), Point::new(60.0, 60.0)];
        let export = export_pptx(&[source(
            Rect::new(0.0, 0.0, 960.0, 540.0),
            vec![sticky, stroke],
        )]);
        let r = &export.report;
        assert_eq!(
            count(r, ToileConstruct::StickyNote, Outcome::Approximated),
            1
        );
        assert_eq!(
            count(r, ToileConstruct::FreehandStroke, Outcome::Approximated),
            1
        );
        let sentence = r.sentence().expect("the report speaks");
        assert!(sentence.contains("sticky note"), "{sentence}");
        assert!(sentence.contains("freehand stroke"), "{sentence}");
        let items = &export.deck.slides.first().expect("a slide").items;
        let Some(Item::Shape(note)) = items.first() else {
            panic!("the sticky is a shape");
        };
        assert_eq!(note.text.as_ref().map(|t| t.text.as_str()), Some("Idea"));
        assert_eq!(note.fill, Some(STICKY_DEFAULT_FILL));
        let Some(Item::Shape(line)) = items.get(1) else {
            panic!("the stroke is a shape");
        };
        assert!(matches!(&line.geometry, Geometry::Freeform(steps) if steps.len() == 2));
    }

    #[test]
    fn what_the_file_cannot_hold_is_dropped_and_named() {
        let export = export_pptx(&[source(
            Rect::new(0.0, 0.0, 960.0, 540.0),
            vec![
                element(ElementKind::Embed, Rect::new(0.0, 0.0, 10.0, 10.0)),
                element(ElementKind::DocEmbed, Rect::new(1.0, 0.0, 10.0, 10.0)),
                element(ElementKind::Image, Rect::new(2.0, 0.0, 10.0, 10.0)),
                element(ElementKind::Group, Rect::new(3.0, 0.0, 10.0, 10.0)),
            ],
        )]);
        let r = &export.report;
        assert_eq!(count(r, ToileConstruct::Embed, Outcome::Dropped), 1);
        assert_eq!(
            count(r, ToileConstruct::EmbeddedDocument, Outcome::Dropped),
            1
        );
        assert_eq!(
            count(r, ToileConstruct::Picture, Outcome::Dropped),
            1,
            "a picture without its pixels"
        );
        assert_eq!(count(r, ToileConstruct::Group, Outcome::Approximated), 1);
        assert!(
            export
                .deck
                .slides
                .first()
                .is_some_and(|s| s.items.is_empty())
        );
    }

    #[test]
    fn a_connector_holds_on_to_the_shapes_it_joins() {
        let a = element(
            ElementKind::Shape(ShapeKind::Rect),
            Rect::new(0.0, 0.0, 100.0, 100.0),
        );
        let b = element(
            ElementKind::Shape(ShapeKind::Ellipse),
            Rect::new(300.0, 0.0, 400.0, 100.0),
        );
        let mut slide = source(
            Rect::new(0.0, 0.0, 960.0, 540.0),
            vec![a.clone(), b.clone()],
        );
        let connector = |path: PathType, caption: &str| PptxConnector {
            from: a.id.clone(),
            from_anchor: Anchor::Right,
            start: Point::new(100.0, 50.0),
            to: b.id.clone(),
            to_anchor: Anchor::Center,
            end: Point::new(350.0, 50.0),
            path,
            caps: EndCaps {
                start: Cap::Plain,
                end: Cap::Arrow,
            },
            style: ElementStyle::default(),
            caption: caption.to_owned(),
        };
        slide.connectors = vec![
            connector(PathType::Straight, ""),
            connector(PathType::Elbow, "yes"),
        ];
        let export = export_pptx(&[slide]);
        let items = &export.deck.slides.first().expect("a slide").items;
        let Some(Item::Connector(straight)) = items.get(2) else {
            panic!("the connectors follow the elements");
        };
        assert_eq!(
            straight.start.attached,
            Some(Attachment {
                item: Pos::<SlideItems>::new(0),
                site: Site::Right,
            })
        );
        assert_eq!(straight.end.attached, None, "the centre holds on nowhere");
        assert_eq!(straight.end_cap, PptxCap::Arrow);
        assert!(
            matches!(items.get(4), Some(Item::Shape(s)) if s.text.as_ref().is_some_and(|t| t.text == "yes")),
            "the caption lands as a text box after the connectors"
        );
        let r = &export.report;
        assert_eq!(count(r, ToileConstruct::Connector, Outcome::Carried), 1);
        assert_eq!(
            count(r, ToileConstruct::Connector, Outcome::Approximated),
            1
        );
        assert_eq!(
            count(r, ToileConstruct::ConnectorLabel, Outcome::Approximated),
            1
        );
    }

    #[test]
    fn the_notes_of_a_slide_reach_its_notes_page() {
        let mut slide = source(Rect::new(0.0, 0.0, 960.0, 540.0), Vec::new());
        slide.notes = "Say hello".to_owned();
        slide.name = "Intro".to_owned();
        let export = export_pptx(&[slide]);
        let out = export.deck.slides.first().expect("a slide");
        assert_eq!(out.notes, "Say hello");
        assert_eq!(out.name, "Intro");
        assert_eq!(
            count(
                &export.report,
                ToileConstruct::SpeakerNotes,
                Outcome::Carried
            ),
            1
        );
    }
}
