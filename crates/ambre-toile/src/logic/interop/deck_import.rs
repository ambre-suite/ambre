//! A presentation file as frames of the deck (`toile-pptx-import`): the
//! board side of the map from the model of `ambre-pptx`.
//!
//! A reader of the Office module reads the file into a deck. Each slide
//! carries what its master and its layout draw, so the map knows no master.
//! Every reader of a presentation format answers one deck and one report.
//! So a PowerPoint file and an OpenDocument presentation of the same content
//! land as the same frames.
//!
//! | In the deck | On the board | Outcome |
//! | :--- | :--- | :--- |
//! | a slide | a frame of the slide size, its background as its fill, at the end of the deck | carried |
//! | the notes of a slide | the speaker notes of its frame | carried |
//! | a rectangle, an ellipse, a triangle, a line | the shape of the same name, with its fill and its line | carried |
//! | a rounded rectangle | a rectangle with that corner radius | carried |
//! | a freeform path | a vector path of the same steps | carried |
//! | the text of a shape | a text element over the shape, left editable | carried |
//! | a picture, PNG or JPEG | a picture and its blob | carried |
//! | a connector bound to two shapes | a connector between them | carried |
//! | a connector with a free end | a line between its two ends | approximated |
//! | a centred or a middle text | a text at the top left | approximated |
//! | a JPEG picture the module could not decode | nothing | dropped |
//!
//! Every slide lands one point to one world unit, the frames side by side
//! in deck order. The report of the reader joins the report of the map, so
//! the user reads one list.

use ambre_bus::interop::{FileFormat, Outcome};
use ambre_core::hash::HashMap;
use ambre_core::index::Pos;
use ambre_core::{Count, Elements, FontSize, Px, Rgba, StoredBoard};
use ambre_pptx::model::{
    Cap, Connector, Geometry, ImportedDeck, Item, Line, PathSegment, Picture, PictureFormat,
    Placement, Route, Shape, Site, SlidePoint, TextAlign, TextAnchor, TextBody,
};
use ambre_pptx::report::DeckOutcome;
use kurbo::{BezPath, Point, Rect, Shape as _, Size};

use super::raster::{decode_media, jpeg_blob};
use super::{
    ImportBatch, ImportConnector, ImportElement, ImportError, ImportFrame, ImportRef, Limits,
};
use crate::logic::command::{ImportSlot, InitContent, MediaRef, ToileCommand};
use crate::logic::model::{Anchor, ElementKind, ElementStyle, EndCaps, PathType, ShapeKind};
use crate::message::{Cap as EndCap, FontSizeChoice, ToileConstruct};

/// The space between two frames of an imported deck, as a share of the
/// slide width.
const SLIDE_GAP: f64 = 0.1;

/// The board side of a deck a reader read: its frames, in deck order, and
/// what the reader and the map lost. `thumbs` holds one entry per picture
/// of the deck, in slide order then item order, as the module answered it.
///
/// # Errors
///
/// [`ImportError::TooManyElements`] past the element bound of `limits`,
/// [`ImportError::ImageTooLarge`] for a picture past its size bound, and
/// [`ImportError::Decode`] for a PNG picture that does not decode. No
/// command survives an error: the batch is atomic.
pub fn import_deck(
    read: &ImportedDeck,
    thumbs: &[Option<ambre_convert::images::Pixels>],
    format: FileFormat,
    limits: &Limits,
) -> Result<ImportBatch, ImportError> {
    let mut map = DeckMap {
        batch: ImportBatch::new(format),
        limits,
        thumbs: thumbs.iter(),
        media: HashMap::default(),
    };
    for entry in read.report.entries() {
        let outcome = match entry.outcome {
            DeckOutcome::Approximated => Outcome::Approximated,
            DeckOutcome::Dropped => Outcome::Dropped,
        };
        map.batch.report.record_count(
            ToileConstruct::Deck(entry.construct),
            outcome,
            Count::<ambre_bus::interop::Occurrences>::new(entry.count.get()),
        );
    }
    let width = f64::from(read.deck.size.width.get());
    let height = f64::from(read.deck.size.height.get());
    for (index, slide) in read.deck.slides.iter().enumerate() {
        let origin = Point::new(index as f64 * width * (1.0 + SLIDE_GAP), 0.0);
        let frame = map.push(
            ElementKind::Frame,
            map.limits
                .clamp_rect(Rect::from_origin_size(origin, Size::new(width, height))),
            InitContent::None,
            Some(ElementStyle {
                fill: Some(slide.background.unwrap_or(Rgba::from_bits(0xFFFF_FFFF))),
                ..ElementStyle::default()
            }),
        )?;
        let mut members = Vec::with_capacity(slide.items.len());
        // The slot each item of the slide landed in, the one a connector
        // names its end by.
        let mut slots: Vec<Option<Pos<ImportElement>>> = Vec::with_capacity(slide.items.len());
        for item in &slide.items {
            let landed = match item {
                Item::Shape(shape) => map.shape(shape, origin, &mut members)?,
                Item::Picture(picture) => {
                    let landed = map.picture(picture, origin)?;
                    members.extend(landed);
                    landed
                }
                Item::Connector(connector) => {
                    map.connector(connector, origin, &slots)?;
                    None
                }
            };
            slots.push(landed);
        }
        if !slide.notes.trim().is_empty() {
            map.batch
                .report
                .record(ToileConstruct::SpeakerNotes, Outcome::Carried);
        }
        map.batch.frames.push(ImportFrame {
            frame,
            members: members.into_iter().map(ImportRef::Slot).collect(),
            name: slide.name.clone(),
            notes: Some(slide.notes.clone()),
        });
    }
    if map.batch.commands.is_empty() {
        return Err(ImportError::Empty);
    }
    Ok(map.batch)
}

/// The walk of one deck: the batch it fills, and the pictures it met.
struct DeckMap<'a> {
    batch: ImportBatch,
    limits: &'a Limits,
    thumbs: std::slice::Iter<'a, Option<ambre_convert::images::Pixels>>,
    /// The media slot of each picture already in the batch, by its bytes.
    /// A logo on the master lands as one blob, on any number of slides.
    media: HashMap<&'a ambre_core::Payload<ambre_pptx::model::PictureBytes>, Pos<ImportSlot>>,
}

impl<'a> DeckMap<'a> {
    /// Add the create of one element, and answer its slot.
    fn push(
        &mut self,
        kind: ElementKind,
        rect: Rect,
        content: InitContent,
        style: Option<ElementStyle>,
    ) -> Result<Pos<ImportElement>, ImportError> {
        self.limits
            .check_element_count(Count::<Elements>::of(self.batch.commands.len()))?;
        let slot = Pos::<ImportElement>::from_usize(self.batch.commands.len());
        self.batch.commands.push(ToileCommand::CreateElement {
            kind,
            at: rect.origin(),
            size: Size::new(rect.width(), rect.height()),
            content,
            fill: None,
            correlate: None,
        });
        self.batch.styles.push(style);
        Ok(slot)
    }

    /// Turn the element at `slot` by the rotation of `at`.
    fn turn(&mut self, slot: Pos<ImportElement>, at: &Placement) {
        if at.rotation == ambre_core::Radians::ZERO {
            return;
        }
        let rank = slot.to_usize();
        if self.batch.turns.len() <= rank {
            self.batch.turns.resize(rank + 1, ambre_core::Radians::ZERO);
        }
        if let Some(turn) = self.batch.turns.get_mut(rank) {
            *turn = at.rotation;
        }
    }

    /// The world rect of `at` on the slide whose corner is `origin`.
    fn rect(&self, at: &Placement, origin: Point) -> Rect {
        let corner = point(at.origin, origin);
        self.limits.clamp_rect(Rect::from_origin_size(
            corner,
            Size::new(f64::from(at.width.get()), f64::from(at.height.get())),
        ))
    }

    /// A shape, and the text it holds as a text element above it. A text
    /// box with no paint lands as its text alone. Answers the slot a
    /// connector holds on to: the shape's, or the text's for a text box.
    fn shape(
        &mut self,
        shape: &Shape,
        origin: Point,
        members: &mut Vec<Pos<ImportElement>>,
    ) -> Result<Option<Pos<ImportElement>>, ImportError> {
        let rect = self.rect(&shape.at, origin);
        let painted = shape.fill.is_some() || shape.line.is_some();
        let mut held = None;
        if painted || shape.text.is_none() {
            let (kind, content, corner) = match &shape.geometry {
                Geometry::Rect => (ShapeKind::Rect, InitContent::None, None),
                Geometry::RoundRect { radius } => {
                    (ShapeKind::Rect, InitContent::None, Some(radius.get()))
                }
                Geometry::Ellipse => (ShapeKind::Ellipse, InitContent::None, None),
                Geometry::Triangle => (ShapeKind::Triangle, InitContent::None, None),
                Geometry::Line => (ShapeKind::Line, InitContent::None, None),
                Geometry::Freeform(steps) => (
                    ShapeKind::Path,
                    InitContent::Path(self.path(steps, origin)),
                    None,
                ),
            };
            let rect = match &content {
                InitContent::Path(path) => self.limits.clamp_rect(path.bounding_box()),
                _ => rect,
            };
            let style = ElementStyle {
                fill: shape.fill,
                stroke: shape.line.map(|l| l.colour),
                stroke_w: shape.line.map_or(Px::<StoredBoard>::ZERO, |l| {
                    Px::<StoredBoard>::new(l.width.get())
                }),
                corner: corner.map_or(Px::<StoredBoard>::ZERO, Px::<StoredBoard>::new),
                ..ElementStyle::default()
            };
            let slot = self.push(ElementKind::Shape(kind), rect, content, Some(style))?;
            self.turn(slot, &shape.at);
            members.push(slot);
            held = Some(slot);
        }
        if let Some(text) = &shape.text {
            let slot = self.text(text, rect)?;
            self.turn(slot, &shape.at);
            members.push(slot);
            held.get_or_insert(slot);
        }
        Ok(held)
    }

    /// A text element in the style of `text`, filling `rect`.
    fn text(&mut self, text: &TextBody, rect: Rect) -> Result<Pos<ImportElement>, ImportError> {
        if text.align != TextAlign::Left || text.anchor != TextAnchor::Top {
            self.batch
                .report
                .record(ToileConstruct::TextAlignment, Outcome::Approximated);
        }
        let mut style = ElementStyle {
            fill: None,
            stroke: None,
            stroke_w: Px::<StoredBoard>::ZERO,
            text_color: Some(text.colour),
            ..ElementStyle::default()
        };
        let font = &mut style.font;
        if !text.typeface.is_empty() {
            font.family = Some(ambre_core::FontFamily::from_label(&text.typeface));
        }
        // One slide point is one world unit, and the board draws a size at
        // its document pixels: the text keeps its share of the slide.
        font.size = FontSize::from_px(ambre_core::Px::<ambre_core::Doc>::new(text.size.get()))
            .and_then(FontSizeChoice::fixed)
            .unwrap_or(FontSizeChoice::DEFAULT);
        font.weight = match text.weight {
            ambre_core::Weight::Bold => ambre_core::Bounded::<ambre_core::FontWeight>::BOLD,
            ambre_core::Weight::Regular => ambre_core::Bounded::<ambre_core::FontWeight>::REGULAR,
        };
        font.italic = text.slant;
        font.underline = text.underline;
        font.strikethrough = text.strikethrough;
        self.push(
            ElementKind::Text,
            rect,
            InitContent::Text(text.text.clone()),
            Some(style),
        )
    }

    /// The world path of the steps of a freeform, on the slide whose corner
    /// is `origin`.
    fn path(&self, steps: &[PathSegment], origin: Point) -> BezPath {
        let at = |p: SlidePoint| self.limits.clamp_point(point(p, origin));
        let mut path = BezPath::new();
        for step in steps {
            match *step {
                PathSegment::MoveTo(p) => path.move_to(at(p)),
                PathSegment::LineTo(p) => path.line_to(at(p)),
                PathSegment::QuadTo { control, end } => path.quad_to(at(control), at(end)),
                PathSegment::CubicTo { first, second, end } => {
                    path.curve_to(at(first), at(second), at(end));
                }
                PathSegment::Close => path.close_path(),
            }
        }
        path
    }

    /// A picture and its blob. A blob the batch already holds is shared. A
    /// JPEG whose thumbnail the module could not decode lands as nothing.
    fn picture(
        &mut self,
        picture: &'a Picture,
        origin: Point,
    ) -> Result<Option<Pos<ImportElement>>, ImportError> {
        let thumb = self.thumbs.next().and_then(Option::as_ref);
        let bytes = &picture.bytes;
        let media = match self.media.get(bytes) {
            Some(slot) => *slot,
            None => {
                let slot = Pos::<ImportSlot>::from_usize(self.batch.media.len());
                let file: ambre_core::Payload<ambre_client_core::FileBytes> =
                    picture.bytes.clone().into_role();
                let reference = MediaRef::Pending(slot);
                let blob = match (picture.format, thumb) {
                    (PictureFormat::Png, _) => decode_media(&file, self.limits, reference)?,
                    (PictureFormat::Jpeg, Some(thumb)) => {
                        let Some(size) = ambre_convert::jpeg::stored_size(&file) else {
                            return Ok(self.unread());
                        };
                        jpeg_blob(file, size, thumb, self.limits, reference)?
                    }
                    (PictureFormat::Jpeg, None) => return Ok(self.unread()),
                };
                self.batch.media.push(blob);
                self.media.insert(bytes, slot);
                slot
            }
        };
        let rect = self.rect(&picture.at, origin);
        let slot = self.push(
            ElementKind::Image,
            rect,
            InitContent::Image(MediaRef::Pending(media)),
            None,
        )?;
        self.turn(slot, &picture.at);
        Ok(Some(slot))
    }

    /// Record a picture the board could not decode, and land nothing.
    fn unread(&mut self) -> Option<Pos<ImportElement>> {
        self.batch.report.record(
            ToileConstruct::Deck(ambre_pptx::report::DeckConstruct::UnreadPicture),
            Outcome::Dropped,
        );
        None
    }

    /// A connector between the two shapes it holds on to. One with a free
    /// end lands as a line between its two ends.
    fn connector(
        &mut self,
        connector: &Connector,
        origin: Point,
        slots: &[Option<Pos<ImportElement>>],
    ) -> Result<(), ImportError> {
        let end_slot = |end: &ambre_pptx::model::ConnectorEnd| {
            end.attached.and_then(|a| {
                slots
                    .get(a.item.to_usize())
                    .copied()
                    .flatten()
                    .map(|s| (s, a.site))
            })
        };
        if let (Some((from, from_site)), Some((to, to_site))) =
            (end_slot(&connector.start), end_slot(&connector.end))
        {
            self.batch.connectors.push(ImportConnector {
                from: ImportRef::Slot(from),
                from_anch: anchor(from_site),
                to: ImportRef::Slot(to),
                to_anch: anchor(to_site),
                path_type: match connector.route {
                    Route::Straight => PathType::Straight,
                    Route::Elbow => PathType::Elbow,
                    Route::Curved => PathType::Curved,
                },
                arrows: EndCaps {
                    start: cap(connector.start_cap),
                    end: cap(connector.end_cap),
                },
            });
            return Ok(());
        }
        self.batch
            .report
            .record(ToileConstruct::Connector, Outcome::Approximated);
        let steps = [
            PathSegment::MoveTo(connector.start.at),
            PathSegment::LineTo(connector.end.at),
        ];
        let path = self.path(&steps, origin);
        let Line { colour, width } = connector.line;
        let style = ElementStyle {
            fill: None,
            stroke: Some(colour),
            stroke_w: Px::<StoredBoard>::new(width.get()),
            ..ElementStyle::default()
        };
        let rect = self.limits.clamp_rect(path.bounding_box());
        let _ = self.push(
            ElementKind::Shape(ShapeKind::Path),
            rect,
            InitContent::Path(path),
            Some(style),
        )?;
        Ok(())
    }
}

/// The world point of a slide point, on the slide whose corner is `origin`.
fn point(p: SlidePoint, origin: Point) -> Point {
    Point::new(
        origin.x + f64::from(p.x.get()),
        origin.y + f64::from(p.y.get()),
    )
}

/// The anchor of the board at the site of a box a connector holds on.
const fn anchor(site: Site) -> Anchor {
    match site {
        Site::Top => Anchor::Top,
        Site::Left => Anchor::Left,
        Site::Bottom => Anchor::Bottom,
        Site::Right => Anchor::Right,
    }
}

/// The end of a board connector a file's end draws.
const fn cap(cap: Cap) -> EndCap {
    match cap {
        Cap::Plain => EndCap::Plain,
        Cap::Arrow => EndCap::Arrow,
    }
}

#[cfg(test)]
mod tests {
    use ambre_bus::interop::{FileFormat, Outcome};
    use ambre_core::index::Pos;
    use ambre_core::{
        Payload, PrintLength, Pt, Radians, Rgba, Slant, Strikethrough, Underline, Weight,
    };
    use ambre_pptx::model::{
        Attachment, Cap, Connector, ConnectorEnd, Deck, Geometry, ImportedDeck, Item, Line,
        Picture, PictureFormat, Placement, Route, Shape, Site, Slide, SlideItems, SlidePoint,
        SlideSize, TextAlign, TextAnchor, TextBody,
    };
    use ambre_pptx::report::{DeckConstruct, DeckOutcome, DeckReport};
    use kurbo::Point;

    use super::import_deck;
    use crate::logic::command::{InitContent, ToileCommand};
    use crate::logic::interop::{ImportError, ImportRef, Limits};
    use crate::logic::model::{ElementKind, ShapeKind};
    use crate::message::ToileConstruct;

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

    fn square(name: &str, x: f32, text: Option<&str>) -> Item {
        Item::Shape(Shape {
            name: name.to_owned(),
            at: placed(x, 10.0, 50.0, 50.0),
            geometry: Geometry::Rect,
            fill: Some(Rgba::from_bits(0xFF00_00FF)),
            line: None,
            text: text.map(|t| TextBody {
                text: t.to_owned(),
                typeface: String::new(),
                size: pt(18.0),
                colour: Rgba::from_bits(0x0000_00FF),
                weight: Weight::Regular,
                slant: Slant::Upright,
                underline: Underline::Plain,
                strikethrough: Strikethrough::Plain,
                anchor: TextAnchor::Middle,
                align: TextAlign::Centre,
                inset: pt(7.2),
            }),
        })
    }

    fn connector(start: Option<usize>, end: Option<usize>) -> Item {
        let end_of = |at: Option<usize>, x: f32| ConnectorEnd {
            at: SlidePoint {
                x: pt(x),
                y: pt(35.0),
            },
            attached: at.map(|i| Attachment {
                item: Pos::<SlideItems>::from_usize(i),
                site: Site::Right,
            }),
        };
        Item::Connector(Connector {
            name: "Link".to_owned(),
            route: Route::Elbow,
            start: end_of(start, 60.0),
            end: end_of(end, 200.0),
            line: Line {
                colour: Rgba::from_bits(0x0000_00FF),
                width: pt(1.0),
            },
            start_cap: Cap::Plain,
            end_cap: Cap::Arrow,
        })
    }

    fn png_picture(x: f32) -> Item {
        let mut png = Vec::new();
        image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
            4,
            4,
            image::Rgba([1, 2, 3, 255]),
        ))
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .expect("the picture encodes");
        Item::Picture(Picture {
            name: "Logo".to_owned(),
            at: placed(x, 100.0, 20.0, 20.0),
            format: PictureFormat::Png,
            bytes: Payload::new(png),
        })
    }

    fn deck(slides: Vec<Slide>) -> ImportedDeck {
        ImportedDeck {
            deck: Deck {
                size: SlideSize::WIDESCREEN,
                slides,
            },
            report: DeckReport::default(),
        }
    }

    fn slide(items: Vec<Item>) -> Slide {
        Slide {
            name: "Slide".to_owned(),
            items,
            notes: String::new(),
            background: Some(Rgba::from_bits(0x1122_33FF)),
        }
    }

    /// The corner and the kind of every create of a batch.
    fn creates(batch: &crate::logic::interop::ImportBatch) -> Vec<(ElementKind, Point)> {
        batch
            .commands
            .iter()
            .filter_map(|c| match c {
                ToileCommand::CreateElement { kind, at, .. } => Some((*kind, *at)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn each_slide_is_a_frame_side_by_side_with_its_background_as_its_fill() {
        let read = deck(vec![slide(vec![]), slide(vec![])]);
        let batch =
            import_deck(&read, &[], FileFormat::Pptx, &Limits::default()).expect("the deck maps");
        assert_eq!(
            creates(&batch),
            [
                (ElementKind::Frame, Point::ZERO),
                (ElementKind::Frame, Point::new(1056.0, 0.0))
            ]
        );
        assert_eq!(
            batch.styles.first().cloned().flatten().and_then(|s| s.fill),
            Some(Rgba::from_bits(0x1122_33FF))
        );
        assert_eq!(batch.frames.len(), 2);
    }

    /// The text of a painted shape lands above it as a text element; both
    /// join the frame, and the shape is what a connector holds.
    #[test]
    fn a_painted_shape_keeps_its_text_as_a_text_element_above_it() {
        let read = deck(vec![slide(vec![
            square("A", 10.0, Some("Hello")),
            square("B", 200.0, None),
            connector(Some(0), Some(1)),
        ])]);
        let batch =
            import_deck(&read, &[], FileFormat::Pptx, &Limits::default()).expect("the deck maps");
        let kinds: Vec<ElementKind> = creates(&batch).into_iter().map(|(k, _)| k).collect();
        assert_eq!(
            kinds,
            [
                ElementKind::Frame,
                ElementKind::Shape(ShapeKind::Rect),
                ElementKind::Text,
                ElementKind::Shape(ShapeKind::Rect)
            ]
        );
        let frame = batch.frames.first().expect("one frame");
        assert_eq!(frame.members.len(), 3);
        let connector = batch.connectors.first().expect("one connector");
        assert_eq!(connector.from, ImportRef::Slot(Pos::from_usize(1)));
        assert_eq!(connector.to, ImportRef::Slot(Pos::from_usize(3)));
        assert_eq!(
            batch
                .report
                .count(ToileConstruct::TextAlignment, Outcome::Approximated)
                .get(),
            1
        );
    }

    /// One slide point is one world unit. So a text of 18 pt is drawn 18
    /// world units tall, and keeps its share of the slide. Found by the
    /// fixture round trip: the size landed as 18 board points, drawn a third
    /// larger, and each export and import grew it again.
    #[test]
    fn a_text_is_drawn_at_its_slide_size_in_world_units() {
        let read = deck(vec![slide(vec![square("A", 10.0, Some("Hello"))])]);
        let batch =
            import_deck(&read, &[], FileFormat::Pptx, &Limits::default()).expect("the deck maps");
        let drawn: Vec<f32> = batch
            .styles
            .iter()
            .flatten()
            .filter(|style| style.text_color.is_some())
            .map(|style| style.font.size.or_default().to_px().get())
            .collect();
        assert_eq!(drawn, [18.0]);
    }

    /// A connector with a free end has no shape to hold. It lands as a line
    /// between its two ends, and the report names it.
    #[test]
    fn a_connector_with_a_free_end_lands_as_a_line() {
        let read = deck(vec![slide(vec![
            square("A", 10.0, None),
            connector(Some(0), None),
        ])]);
        let batch =
            import_deck(&read, &[], FileFormat::Pptx, &Limits::default()).expect("the deck maps");
        assert!(batch.connectors.is_empty());
        assert!(batch.commands.iter().any(|c| matches!(
            c,
            ToileCommand::CreateElement {
                content: InitContent::Path(_),
                ..
            }
        )));
        assert_eq!(
            batch
                .report
                .count(ToileConstruct::Connector, Outcome::Approximated)
                .get(),
            1
        );
    }

    /// A logo on the master lands on every slide as one blob.
    #[test]
    fn a_picture_on_every_slide_is_one_blob() {
        let read = deck(vec![
            slide(vec![png_picture(0.0)]),
            slide(vec![png_picture(0.0)]),
        ]);
        let batch = import_deck(&read, &[None, None], FileFormat::Pptx, &Limits::default())
            .expect("the deck maps");
        assert_eq!(batch.media.len(), 1);
        let pictures = creates(&batch)
            .into_iter()
            .filter(|(k, _)| *k == ElementKind::Image)
            .count();
        assert_eq!(pictures, 2);
    }

    /// A JPEG whose thumbnail the module could not decode lands as nothing,
    /// and the report names it.
    #[test]
    fn a_jpeg_without_its_thumbnail_is_named_as_dropped() {
        let jpeg = Item::Picture(Picture {
            name: "Photo".to_owned(),
            at: placed(0.0, 0.0, 10.0, 10.0),
            format: PictureFormat::Jpeg,
            bytes: Payload::new(vec![0xFF, 0xD8, 0xFF, 0xE0]),
        });
        let read = deck(vec![slide(vec![jpeg])]);
        let batch = import_deck(&read, &[None], FileFormat::Pptx, &Limits::default())
            .expect("the deck maps");
        assert!(batch.media.is_empty());
        assert_eq!(
            batch
                .report
                .count(
                    ToileConstruct::Deck(DeckConstruct::UnreadPicture),
                    Outcome::Dropped
                )
                .get(),
            1
        );
    }

    /// The report of the reader reaches the report of the board, entry by
    /// entry.
    #[test]
    fn the_report_of_the_reader_joins_the_report_of_the_map() {
        let mut read = deck(vec![slide(vec![])]);
        read.report
            .record(DeckConstruct::Transition, DeckOutcome::Approximated);
        read.report
            .record(DeckConstruct::Chart, DeckOutcome::Dropped);
        read.report
            .record(DeckConstruct::Chart, DeckOutcome::Dropped);
        let batch =
            import_deck(&read, &[], FileFormat::Pptx, &Limits::default()).expect("the deck maps");
        let report = &batch.report;
        assert_eq!(
            report
                .count(
                    ToileConstruct::Deck(DeckConstruct::Transition),
                    Outcome::Approximated
                )
                .get(),
            1
        );
        assert_eq!(
            report
                .count(ToileConstruct::Deck(DeckConstruct::Chart), Outcome::Dropped)
                .get(),
            2
        );
    }

    #[test]
    fn a_deck_past_the_element_bound_is_refused_whole() {
        let items = (0..20).map(|i| square("S", i as f32, None)).collect();
        let read = deck(vec![slide(items)]);
        let limits = Limits {
            max_elements: ambre_core::Count::new(10),
            ..Limits::default()
        };
        assert!(matches!(
            import_deck(&read, &[], FileFormat::Pptx, &limits),
            Err(ImportError::TooManyElements { .. })
        ));
    }

    /// A turned item lands turned.
    #[test]
    fn a_turned_item_keeps_its_turn() {
        let mut turned = square("T", 10.0, None);
        if let Item::Shape(shape) = &mut turned {
            shape.at.rotation = Radians::new(0.5);
        }
        let batch = import_deck(
            &deck(vec![slide(vec![turned])]),
            &[],
            FileFormat::Pptx,
            &Limits::default(),
        )
        .expect("the deck maps");
        assert_eq!(batch.turns.get(1).copied(), Some(Radians::new(0.5)));
    }
}
