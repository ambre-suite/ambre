//! The board walk of the PowerPoint export (`toile-pptx-export`): one source
//! per slide, in play order, for the map of
//! [`export_pptx`](crate::logic::interop::export_pptx).
//!
//! Read-only by construction: every access here is a `get`, and nothing
//! commits (`TOILE-IO-024`).

use ambre_core::hash::HashSet;

use ambre_core::id::{Element, Id};
use kurbo::Rect;

use super::ToileState;
use super::codec::{bin_of, str_of};
use super::names::{ConnectorKey, ElementKey};
use crate::logic::command::{ToileEvent, ToileOutcome};
use crate::logic::connector::anchor_point;
use crate::logic::interop::{PptxConnector, PptxElement, PptxExport, PptxSlideSource, export_pptx};
use crate::logic::model::{ElementKind, NodeId};
use ambre_bus::interop::{Direction, ExportedFile, InteropAnswer, InteropError};

/// The stem of the name a PowerPoint export carries. The PDF of the deck
/// carries the same stem: the Logic thread holds a document, not its title.
const PPTX_STEM: &str = "deck";

/// The media type of a PowerPoint package.
const PPTX_MEDIA_TYPE: &str =
    "application/vnd.openxmlformats-officedocument.presentationml.presentation";

impl ToileState {
    /// The deck as the PowerPoint model, with what it approximates and drops.
    ///
    /// One slide per frame the deck still resolves, in play order. A board
    /// with no deck gives one slide framing its whole content, as the PDF
    /// export does. `None` when the board holds no element: a file of no
    /// slide is a file the user cannot tell from a broken one.
    #[must_use]
    pub fn export_pptx_deck(&self) -> Option<PptxExport> {
        let order = self.slide_order();
        let deck: HashSet<Id<Element>> = order.iter().map(|f| f.clone().into()).collect();
        let mut regions: Vec<(Option<Id<Element>>, Rect)> = order
            .into_iter()
            .filter_map(|frame| {
                let id: Id<Element> = frame.into();
                self.element_rect(&id).map(|rect| (Some(id), rect))
            })
            .collect();
        if regions.is_empty() {
            let whole = self.bounds.values().copied().reduce(|a, b| a.union(b))?;
            regions.push((None, whole));
        }
        let elements = self.pptx_elements();
        let slides: Vec<PptxSlideSource> = regions
            .into_iter()
            .map(|(frame, region)| {
                let on_slide = elements
                    .iter()
                    .filter(|el| meets(el.element.rect, region))
                    // A frame of the deck is a slide of its own.
                    .filter(|el| el.element.kind != ElementKind::Frame || !deck.contains(&el.id))
                    .cloned()
                    .collect();
                let (name, notes) = frame.as_ref().map_or_else(Default::default, |id| {
                    let name = self
                        .element_map(id)
                        .and_then(|em| str_of(&em, ElementKey::Name))
                        .unwrap_or_default();
                    let notes = self
                        .frame(id)
                        .and_then(|f| self.notes(&f))
                        .unwrap_or_default();
                    (name, notes)
                });
                PptxSlideSource {
                    frame: region,
                    name,
                    notes,
                    elements: on_slide,
                    connectors: self.pptx_connectors(region),
                }
            })
            .collect();
        Some(export_pptx(&slides))
    }

    /// The deck written as a PowerPoint file by the Office module, as one
    /// [`InteropAnswer::ExportReady`] the UI thread saves and reports
    /// (`TOILE-IO-126`). The report is the map's: the writer carries every
    /// item it receives (`TOILE-IO-125`). Read-only: the outcome carries no
    /// update (`TOILE-IO-127`).
    ///
    /// Three cases answer a failure with its reason and no file
    /// (`TOILE-IO-128`). The board holds no element, the module did not
    /// load, or the writer refused.
    pub(super) fn export_pptx_file(
        &self,
        converter: Result<&ambre_convert::Converter, ambre_core::StopCause>,
    ) -> ToileOutcome {
        let Some(PptxExport { deck, report }) = self.export_pptx_deck() else {
            return Self::interop_failed(
                Direction::Export,
                InteropError::Stopped(ambre_core::StopCause::NothingToConvert),
            );
        };
        let written = converter.and_then(|converter| {
            let written = ambre_convert::encode_deck(&deck)
                .and_then(|deck| converter.convert(&ambre_convert::Conversion::WritePptx { deck }));
            match written {
                Ok(ambre_convert::Converted::File(data)) => Ok(data),
                Ok(_) => Err(ambre_core::StopCause::ConverterAnswered),
                Err(e) => {
                    tracing::warn!("Toile: the PowerPoint export stopped: {e}");
                    Err(e.stop_cause())
                }
            }
        });
        match written {
            Ok(data) => {
                ToileOutcome::display_only(vec![ToileEvent::Interop(InteropAnswer::ExportReady {
                    file: ExportedFile {
                        filename: ambre_core::ExportFilename::named(
                            PPTX_STEM,
                            ambre_core::ExportExtension::Pptx,
                        ),
                        media_type: ambre_core::MediaType::from(PPTX_MEDIA_TYPE),
                        data,
                    },
                    report,
                })])
            }
            Err(cause) => Self::interop_failed(Direction::Export, InteropError::Stopped(cause)),
        }
    }

    /// Every element of the board, in paint order, with the samples of a
    /// freehand stroke.
    fn pptx_elements(&self) -> Vec<PptxElement> {
        self.layer_order()
            .iter()
            .filter_map(|node| match node {
                NodeId::Element(id) => {
                    let element = self.export_element(id)?;
                    let stroke = match element.kind {
                        ElementKind::Drawing => self
                            .element_map(id)
                            .map(|em| self.stroke_of(&em))
                            .unwrap_or_default(),
                        _ => Vec::new(),
                    };
                    Some(PptxElement {
                        id: id.clone(),
                        element,
                        stroke,
                    })
                }
                NodeId::Connector(_) => None,
            })
            .collect()
    }

    /// Every connector whose line meets `region`, in the paint order of the
    /// board. A connector whose end lost its element draws nowhere, and
    /// leaves nothing.
    fn pptx_connectors(&self, region: Rect) -> Vec<PptxConnector> {
        let wires = self.wires();
        self.layer_order()
            .iter()
            .filter_map(|node| match node {
                NodeId::Connector(id) => wires.get(id).map(|w| (id, &w.conn)),
                NodeId::Element(_) => None,
            })
            .filter_map(|(id, conn)| {
                let hull = conn.hull(&self.bounds)?;
                if !meets(hull, region) {
                    return None;
                }
                let cm = self.connector_map(id)?;
                let caption = cm
                    .get(ConnectorKey::Label)
                    .and_then(|v| v.into_container().ok())
                    .and_then(|c| c.into_text().ok())
                    .map(|t| t.to_string())
                    .unwrap_or_default();
                Some(PptxConnector {
                    from: conn.from.clone(),
                    from_anchor: conn.from_anch,
                    start: anchor_point(self.bounds.get(&conn.from)?, conn.from_anch),
                    to: conn.to.clone(),
                    to_anchor: conn.to_anch,
                    end: anchor_point(self.bounds.get(&conn.to)?, conn.to_anch),
                    path: conn.path_type,
                    caps: conn.arrows,
                    style: self.style_read(bin_of(&cm, ConnectorKey::Style).as_deref()),
                    caption,
                })
            })
            .collect()
    }
}

/// Whether two boxes meet, edges included: a line of no height still lies on
/// the slide it crosses.
fn meets(a: Rect, b: Rect) -> bool {
    a.x0 <= b.x1 && a.x1 >= b.x0 && a.y0 <= b.y1 && a.y1 >= b.y0
}

#[cfg(test)]
mod tests {
    use ambre_bus::interop::Outcome;
    use ambre_core::id::{Element, Frame, Id};
    use ambre_core::index::{Len, Pos};
    use ambre_core::{Char, FileBytes, Payload, Px, StoredBoard};
    use ambre_pptx::model::Item;
    use ambre_pptx::opc::{Package, PartName};
    use kurbo::{Point, Size};

    use crate::logic::command::{InitContent, ToileCommand, ToileEvent};
    use crate::logic::model::{ElementKind, ElementStyle, ShapeKind};
    use crate::logic::state::ToileState;
    use crate::message::{StrokePoint, TextDelta, ToileConstruct};

    fn create(
        st: &mut ToileState,
        kind: ElementKind,
        x: f64,
        y: f64,
        w: f64,
        h: f64,
    ) -> Id<Element> {
        st.handle_command(ToileCommand::CreateElement {
            kind,
            at: Point::new(x, y),
            size: Size::new(w, h),
            content: InitContent::None,
            fill: None,
            correlate: None,
        })
        .display
        .into_iter()
        .find_map(|e| match e {
            ToileEvent::ElementCreated { id, .. } => Some(id),
            _ => None,
        })
        .expect("creation yields an id")
    }

    fn frame(st: &mut ToileState, x: f64, name: &str) -> Id<Frame> {
        let id = create(st, ElementKind::Frame, x, 0.0, 960.0, 540.0);
        st.handle_command(ToileCommand::RenameFrame {
            frame: id.clone(),
            name: name.to_owned(),
        });
        st.frame(&id).expect("a frame")
    }

    fn rect(st: &mut ToileState, x: f64) -> Id<Element> {
        create(
            st,
            ElementKind::Shape(ShapeKind::Rect),
            x,
            100.0,
            100.0,
            50.0,
        )
    }

    fn names(st: &ToileState) -> Vec<String> {
        st.export_pptx_deck()
            .expect("a deck")
            .deck
            .slides
            .into_iter()
            .map(|s| s.name)
            .collect()
    }

    #[test]
    fn one_slide_per_frame_in_play_order() {
        let mut st = ToileState::new();
        let a = frame(&mut st, 0.0, "A");
        let b = frame(&mut st, 2000.0, "B");
        st.handle_command(ToileCommand::SetSlideOrder {
            order: vec![b.clone(), a.clone()],
        });
        assert_eq!(
            names(&st),
            ["B", "A"],
            "the play order, never the creation order"
        );
        st.handle_command(ToileCommand::SetSlideOrder {
            order: vec![a, b.clone()],
        });
        assert_eq!(names(&st), ["A", "B"]);
        st.handle_command(ToileCommand::DeleteElements {
            ids: vec![b.into()],
        });
        assert_eq!(
            names(&st),
            ["A"],
            "a slide whose frame is gone leaves no slide"
        );
    }

    #[test]
    fn each_slide_holds_what_its_frame_shows() {
        let mut st = ToileState::new();
        let a = frame(&mut st, 0.0, "A");
        let b = frame(&mut st, 2000.0, "B");
        rect(&mut st, 10.0);
        rect(&mut st, 2010.0);
        rect(&mut st, 2200.0);
        st.handle_command(ToileCommand::SetSlideOrder { order: vec![a, b] });
        let export = st.export_pptx_deck().expect("a deck");
        let counts: Vec<usize> = export.deck.slides.iter().map(|s| s.items.len()).collect();
        assert_eq!(
            counts,
            [1, 2],
            "a frame of the deck is no item of another slide"
        );
        assert_eq!(
            export
                .report
                .count(ToileConstruct::Frame, Outcome::Dropped)
                .get(),
            0
        );
    }

    #[test]
    fn an_empty_deck_exports_the_whole_board_as_one_slide() {
        let mut st = ToileState::new();
        rect(&mut st, 0.0);
        rect(&mut st, 5000.0);
        let export = st.export_pptx_deck().expect("a deck");
        let [only] = export.deck.slides.as_slice() else {
            panic!("one slide");
        };
        assert_eq!(only.items.len(), 2, "the slide frames the whole board");
    }

    #[test]
    fn an_empty_board_has_no_deck_to_export() {
        assert!(ToileState::new().export_pptx_deck().is_none());
    }

    #[test]
    fn the_notes_of_a_frame_reach_its_slide() {
        let mut st = ToileState::new();
        let a = frame(&mut st, 0.0, "A");
        st.handle_command(ToileCommand::SetSlideOrder {
            order: vec![a.clone()],
        });
        st.handle_command(ToileCommand::EditNotes {
            frame: a,
            base: String::new(),
            delta: TextDelta {
                at: Pos::<Char>::ZERO,
                delete: Len::<Char>::ZERO,
                insert: "say this".to_owned(),
            },
        });
        let export = st.export_pptx_deck().expect("a deck");
        assert_eq!(
            export.deck.slides.first().map(|s| s.notes.as_str()),
            Some("say this")
        );
    }

    #[test]
    fn a_stroke_leaves_as_a_freeform_line() {
        let mut st = ToileState::new();
        st.handle_command(ToileCommand::CommitStroke {
            points: (0..5)
                .map(|i| {
                    let v = Px::<StoredBoard>::new(f32::from(u8::try_from(i).unwrap_or(0)) * 10.0);
                    StrokePoint::full_pressure(v, v)
                })
                .collect(),
            style: ElementStyle::default(),
        });
        let export = st.export_pptx_deck().expect("a deck");
        let item = export
            .deck
            .slides
            .first()
            .and_then(|s| s.items.first())
            .expect("the stroke lands");
        assert!(
            matches!(item, Item::Shape(s) if matches!(&s.geometry, ambre_pptx::model::Geometry::Freeform(steps) if steps.len() > 1))
        );
        assert_eq!(
            export
                .report
                .count(ToileConstruct::FreehandStroke, Outcome::Approximated)
                .get(),
            1
        );
    }

    /// An export writes nothing: the version vector of the oplog is the same
    /// after it, so no delta can leave for a peer.
    #[test]
    fn a_pptx_export_leaves_the_oplog_unchanged() {
        let mut st = ToileState::new();
        let a = frame(&mut st, 0.0, "A");
        rect(&mut st, 10.0);
        st.handle_command(ToileCommand::SetSlideOrder { order: vec![a] });
        let before = st.doc.oplog_vv();
        st.export_pptx_deck().expect("a deck");
        assert_eq!(st.doc.oplog_vv(), before, "the export wrote to the board");
        assert_eq!(st.doc.state_vv(), before);
    }

    /// The whole path on the host. A board of two frames, two shapes, a
    /// connector and a note goes through the writer and back out of the file.
    #[test]
    fn a_board_opens_back_from_its_file() {
        let mut st = ToileState::new();
        let a = frame(&mut st, 0.0, "Intro");
        let b = frame(&mut st, 2000.0, "Plan");
        let left = rect(&mut st, 10.0);
        let right = rect(&mut st, 400.0);
        st.handle_command(ToileCommand::CreateConnector {
            from: left,
            from_anch: crate::logic::model::Anchor::Right,
            to: right,
            to_anch: crate::logic::model::Anchor::Left,
            path_type: crate::logic::model::PathType::Straight,
            arrows: crate::logic::model::EndCaps {
                start: crate::message::Cap::Plain,
                end: crate::message::Cap::Arrow,
            },
        });
        st.handle_command(ToileCommand::SetSlideOrder {
            order: vec![a.clone(), b],
        });
        st.handle_command(ToileCommand::EditNotes {
            frame: a,
            base: String::new(),
            delta: TextDelta {
                at: Pos::<Char>::ZERO,
                delete: Len::<Char>::ZERO,
                insert: "hello".to_owned(),
            },
        });
        let export = st.export_pptx_deck().expect("a deck");
        let bytes: Payload<FileBytes> = ambre_pptx::write(&export.deck).expect("the deck writes");
        let mut package = Package::open(bytes.as_bytes()).expect("the file opens back");
        let part = |p: &str| PartName::new(p).expect("a part name");
        let slide1 = String::from_utf8(
            package
                .part(&part("/ppt/slides/slide1.xml"))
                .expect("the part reads")
                .expect("a first slide")
                .as_bytes()
                .to_vec(),
        )
        .expect("UTF-8");
        assert!(slide1.contains("p:cxnSp"), "the connector leaves");
        assert!(slide1.contains("stCxn"), "bound to its first shape");
        assert!(package.contains(&part("/ppt/slides/slide2.xml")));
        assert!(!package.contains(&part("/ppt/slides/slide3.xml")));
        let rels = package
            .relationships(&part("/ppt/slides/slide1.xml"))
            .expect("read");
        assert!(
            rels.iter()
                .any(|r| r.kind.as_str().ends_with("/notesSlide"))
        );
    }
}
