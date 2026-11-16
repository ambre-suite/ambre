//! Board -> file, the outbound half of `toile-import-export`.
//!
//! The SVG serializer is a pure function over a slice of
//! [`ExportElement`](crate::logic::interop::ExportElement). Nothing built that
//! slice from a live board, so no user could export anything. This is that
//! reader: it walks the paint order, projects each element to the export shape,
//! and hands the bytes back as one display event.
//!
//! Read-only by construction (`TOILE-IO-024`): every access here is a `get`, and
//! the outcome carries no CRDT update.

use crate::logic::command::{ToileEvent, ToileOutcome};
use crate::logic::interop::{ExportElement, SvgExport, export_svg, showcase_page};
use crate::logic::model::{ElementKind, NodeId, ShapeKind};
use crate::message::{ToileConstruct, ToileReport};
use ambre_bus::interop::{
    Direction, ExportedFile, FileFormat, InteropAnswer, Occurrences, Outcome,
};
use ambre_core::Count;
use ambre_core::Px;
use ambre_core::id::{Element, Id};

use super::ToileState;
use super::codec::{bin_of, body_of, kind_of, path_of, rotation_of, stored_rect, str_of};
use super::names::{ElementKey, FieldValues, MediaKey, Root};
use ambre_core::crdt::CrdtName as _;

/// The stem of the name an SVG export and a showcase page carry. The board's
/// own title is a launcher concern: the Logic thread holds a document, not its
/// listing entry. The name therefore says what the file holds.
const BOARD_STEM: &str = "board";

impl ToileState {
    /// Decode the thumbnail stored for one media address, if the document
    /// carries one and it parses (`toile-media`).
    ///
    /// The thumbnail is every pixel this thread holds. The full blob is
    /// ciphertext on the relay, and no path here fetches it. Both the screen and
    /// an exported page therefore get this resolution (`TOILE-IO-100`).
    pub(super) fn decoded_thumbnail(
        &self,
        hash: &ambre_core::Validated<ambre_core::ContentHash>,
    ) -> Option<crate::ui::render::DecodedImage> {
        let loro::LoroValue::Map(entries) = self.doc.get_map(Root::Media.as_str()).get_deep_value()
        else {
            return None;
        };
        let loro::LoroValue::Map(fields) = entries.get(hash.as_str())? else {
            return None;
        };
        let loro::LoroValue::Binary(thumb) =
            FieldValues::<MediaKey>::new(fields).get(MediaKey::Thumb)?
        else {
            return None;
        };
        let rgba = image::load_from_memory(thumb).ok()?.to_rgba8();
        let size = ambre_core::Size::new(Px::new(rgba.width()), Px::new(rgba.height()));
        Some(crate::ui::render::DecodedImage {
            pixels: rgba.into_raw().into(),
            size,
        })
    }

    /// The PNG bytes stored as one media address's thumbnail, if the document
    /// carries one (`toile-media`). Handed to the SVG export, which writes them
    /// inline (`TOILE-IO-087`).
    fn thumbnail_png(
        &self,
        hash: &ambre_core::Validated<ambre_core::ContentHash>,
    ) -> Option<ambre_core::Payload<ambre_core::ThumbnailPng>> {
        let loro::LoroValue::Map(entries) = self.doc.get_map(Root::Media.as_str()).get_deep_value()
        else {
            return None;
        };
        let loro::LoroValue::Map(fields) = entries.get(hash.as_str())? else {
            return None;
        };
        let loro::LoroValue::Binary(thumb) =
            FieldValues::<MediaKey>::new(fields).get(MediaKey::Thumb)?
        else {
            return None;
        };
        Some(ambre_core::Payload::<ambre_core::ThumbnailPng>::new(
            thumb.to_vec(),
        ))
    }

    /// Serialize the board to an SVG document (`TOILE-IO-021`). Emits one
    /// [`InteropAnswer::ExportReady`] carrying the bytes, which
    /// the UI thread hands to the browser (`TOILE-IO-085`).
    ///
    /// An empty board still exports: a valid, empty SVG is an answer, and
    /// refusing one would leave the user wondering whether the gesture arrived.
    pub(super) fn export_svg_document(&self) -> ToileOutcome {
        let export = self.svg_of_board();
        ToileOutcome::display_only(vec![ToileEvent::Interop(InteropAnswer::ExportReady {
            file: ExportedFile {
                filename: ambre_client_core::ExportFilename::named(
                    BOARD_STEM,
                    ambre_client_core::ExportExtension::Svg,
                ),
                media_type: ambre_core::MediaType::from("image/svg+xml"),
                data: ambre_core::Payload::<ambre_client_core::FileBytes>::new(
                    export.document.into_bytes(),
                ),
            },
            // What no SVG shape expresses, named rather than dropped in
            // silence (`TOILE-IO-025`).
            report: export.report,
        })])
    }

    /// Write the board as a showcase page (`TOILE-IO-138`): the SVG export
    /// inline, and no board state beside it (`TOILE-IO-018`).
    ///
    /// The answer is a display event alone. The page reaches the UI thread's
    /// download path and no other, so neither the Sync worker nor the relay
    /// ever holds it (`ZK-037`).
    pub(super) fn export_showcase_document(&self) -> ToileOutcome {
        let svg = self.svg_of_board();
        let data = showcase_page(&svg);
        // The page draws what the SVG export draws, so it drops what that
        // export drops, and says so under its own format (`TOILE-IO-139`).
        let mut report = ToileReport::new(Direction::Export, FileFormat::Html);
        report.absorb(svg.report);
        ToileOutcome::display_only(vec![ToileEvent::Interop(InteropAnswer::ExportReady {
            file: ExportedFile {
                filename: ambre_client_core::ExportFilename::named(
                    BOARD_STEM,
                    ambre_client_core::ExportExtension::Html,
                ),
                media_type: ambre_core::MediaType::from("text/html"),
                data,
            },
            report,
        })])
    }

    /// The SVG export of the board, connectors named in its report.
    fn svg_of_board(&self) -> SvgExport {
        let mut export = export_svg(&self.export_elements());
        // A connector never reaches the serializer, so the board walk names
        // each one itself rather than lose it in silence (`TOILE-IO-025`).
        let connectors = self
            .layer_order()
            .iter()
            .filter(|node| matches!(node, NodeId::Connector(_)))
            .count();
        export.report.record_count(
            ToileConstruct::Connector,
            Outcome::Dropped,
            Count::<Occurrences>::of(connectors),
        );
        export
    }

    /// Every element of the board, in paint order, as the serializer's shape.
    ///
    /// * Connectors are not elements, so they do not appear here. The caller
    ///   reports them (`TOILE-IO-025`).
    /// * A group carries no geometry of its own, and the serializer skips it.
    ///   Its members therefore export flat.
    fn export_elements(&self) -> Vec<ExportElement> {
        self.layer_order()
            .iter()
            .filter_map(|node| match node {
                NodeId::Element(id) => self.export_element(id),
                NodeId::Connector(_) => None,
            })
            .collect()
    }

    /// One element, read from the CRDT exactly as the display pass reads it.
    /// That means the stored box for anything with geometry, and the derived
    /// union for a group.
    pub(super) fn export_element(&self, id: &Id<Element>) -> Option<ExportElement> {
        let em = self.element_map(id)?;
        let kind = kind_of(&em);
        let rect = if kind == ElementKind::Group {
            self.bounds.get(id).copied()?
        } else {
            stored_rect(&em)
        };
        Some(ExportElement {
            kind,
            rect,
            rotation: rotation_of(&em),
            style: self.style_read(bin_of(&em, ElementKey::Style).as_deref()),
            path: match kind {
                ElementKind::Shape(ShapeKind::Path) => path_of(&em),
                _ => None,
            },
            text: match kind {
                // A body the map lacks exports as an empty one.
                ElementKind::Text | ElementKind::Sticky => Some(body_of(&em).unwrap_or_default()),
                _ => None,
            },
            media_hash: match kind {
                ElementKind::Image => str_of(&em, ElementKey::Hash).and_then(|hash| {
                    ambre_core::Validated::<ambre_core::ContentHash>::parse(&hash).ok()
                }),
                _ => None,
            },
            // The plaintext thumbnail, re-encoded as the PNG the export writes
            // inline (`TOILE-IO-087`).
            image_png: match kind {
                ElementKind::Image => str_of(&em, ElementKey::Hash)
                    .and_then(|hash| {
                        ambre_core::Validated::<ambre_core::ContentHash>::parse(&hash).ok()
                    })
                    .and_then(|hash| self.thumbnail_png(&hash)),
                _ => None,
            },
        })
    }
}

/// The deck as a PDF (`toile-pdf-export`), one page per slide.
///
/// Gated behind the `pdf` feature, and served by the Logic thread rather than by
/// the board alone. The render walk needs the base font faces the Main thread
/// fetched at boot. Those faces live beside the documents and not inside one.
#[cfg(feature = "pdf")]
impl ToileState {
    /// Render the deck at `page` size, or say why there is nothing to render.
    /// Read-only: the walk takes the board by shared reference and the outcome
    /// carries no update (`TOILE-IO-046`). Every page is recorded in one call:
    /// the Logic thread records them over several passes through
    /// [`record_deck`](Self::record_deck) instead.
    ///
    /// # Errors
    ///
    /// [`PdfExportError::Empty`] when the board yields no page, and
    /// [`PdfExportError::Convert`] when the converter refuses or cannot run.
    ///
    /// [`PdfExportError::Empty`]: crate::logic::interop::PdfExportError::Empty
    /// [`PdfExportError::Convert`]: crate::logic::interop::PdfExportError::Convert
    pub fn export_pdf_document(
        &self,
        page: crate::logic::interop::PageSize,
        font_cx: &mut ambre_draw::FontContext,
        converter: &ambre_convert::Converter,
    ) -> Result<
        ambre_core::Payload<ambre_client_core::FileBytes>,
        crate::logic::interop::PdfExportError,
    > {
        let mut recording = self.record_deck(page)?;
        while !recording.is_recorded() {
            recording.record_next(font_cx);
        }
        crate::logic::interop::write_pdf(recording.into_document(), converter)
    }

    /// Start a deck export at `page` size: the pages and the pictures, read
    /// now, and no page recorded yet (`TOILE-IO-099`). The recording holds
    /// no reference to the board, so an edit after this call reaches no page
    /// (`TOILE-IO-133`).
    ///
    /// The pictures come from the `media` map's inline thumbnails, which is every
    /// pixel this thread holds. The full-resolution blobs are ciphertext on the
    /// relay, and no cache on this side has them yet (`toile-media`). A page is
    /// therefore no sharper than the board is on screen.
    ///
    /// # Errors
    ///
    /// [`PdfExportError::Empty`](crate::logic::interop::PdfExportError::Empty)
    /// when the board yields no page.
    pub fn record_deck(
        &self,
        page: crate::logic::interop::PageSize,
    ) -> Result<crate::logic::interop::DeckRecording, crate::logic::interop::PdfExportError> {
        // The page camera maps world units onto PDF points 1:1, so the text
        // cache of the recording runs at unit scale like the export walk.
        crate::logic::interop::DeckRecording::new(
            self.pdf_pages(page),
            page,
            ambre_canvas::Theme::Light.palette(),
            self.media_thumbnails(),
        )
    }

    /// Every picture the board can draw, keyed by content hash: each media
    /// entry's inline thumbnail, decoded to pixels. An entry whose bytes do not
    /// decode is left out, so its element paints the placeholder box rather than
    /// failing the export. The tests read the pixels through it; an export
    /// decodes each picture in the step of its page instead.
    #[cfg(test)]
    pub(super) fn decoded_media(
        &self,
    ) -> ambre_core::hash::HashMap<
        ambre_core::Validated<ambre_core::ContentHash>,
        crate::ui::render::DecodedImage,
    > {
        self.media_thumbnails()
            .into_iter()
            .filter_map(|(hash, thumb)| {
                crate::logic::interop::decode_thumbnail(&thumb).map(|image| (hash, image))
            })
            .collect()
    }

    /// Every picture the board can draw, keyed by content hash: each media
    /// entry's inline thumbnail, still encoded. A key that spells no address
    /// is left out.
    fn media_thumbnails(&self) -> crate::logic::interop::Thumbnails {
        let mut out = crate::logic::interop::Thumbnails::default();
        let loro::LoroValue::Map(entries) = self.doc.get_map(Root::Media.as_str()).get_deep_value()
        else {
            return out;
        };
        for (hash, entry) in entries.iter() {
            // A peer wrote the key, so a text that spells no address is left
            // out like bytes that do not decode.
            let Some(hash) = ambre_core::Validated::<ambre_core::ContentHash>::parse(hash).ok()
            else {
                continue;
            };
            let loro::LoroValue::Map(fields) = entry else {
                continue;
            };
            let Some(loro::LoroValue::Binary(thumb)) =
                FieldValues::<MediaKey>::new(fields).get(MediaKey::Thumb)
            else {
                continue;
            };
            out.insert(
                hash,
                ambre_core::Payload::<ambre_core::ThumbnailPng>::new(thumb.to_vec()),
            );
        }
        out
    }
}

#[cfg(test)]
mod tests {
    /// The converter the Logic thread's host build hands in: the engine,
    /// linked in place.
    #[cfg(feature = "pdf")]
    fn host_converter() -> ambre_convert::Converter {
        ambre_convert::Converter::load(
            ambre_core::ConverterFamily::Pdf,
            &ambre_core::Payload::new(Vec::new()),
        )
        .expect("the host links the engine")
    }

    use kurbo::{Point, Size};

    use crate::logic::command::{InitContent, ToileCommand, ToileEvent};
    use crate::logic::model::{ElementKind, ShapeKind};
    use crate::message::{ToileConstruct, ToileExport};
    use ambre_bus::interop::{InteropAnswer, InteropRequest, Outcome};

    use super::super::ToileState;

    fn board_with_a_rect() -> ToileState {
        let mut st = ToileState::new();
        st.handle_command(ToileCommand::CreateElement {
            kind: ElementKind::Shape(ShapeKind::Rect),
            at: Point::new(10.0, 20.0),
            size: Size::new(30.0, 40.0),
            content: InitContent::None,
            fill: None,
            correlate: None,
        });
        st
    }

    fn exported(st: &mut ToileState) -> (String, Vec<u8>) {
        let out = st.handle_command(ToileCommand::Interop(InteropRequest::Export(
            ToileExport::Svg,
        )));
        assert!(
            out.update.is_none(),
            "an export must emit no CRDT delta (`TOILE-IO-024`)"
        );
        out.display
            .iter()
            .find_map(|e| match e {
                ToileEvent::Interop(InteropAnswer::ExportReady { file, .. }) => {
                    Some((file.filename.to_string(), file.data.as_bytes().to_vec()))
                }
                _ => None,
            })
            .expect("the export answers with the file")
    }

    /// The board reaches the serializer: an element on the board is an element in
    /// the file, at the coordinates it holds (`TOILE-IO-021`).
    #[test]
    fn the_board_exports_as_an_svg_document() {
        let mut st = board_with_a_rect();
        let (name, data) = exported(&mut st);
        let svg = String::from_utf8(data).expect("the document is text");
        assert_eq!(name, "board.svg");
        assert!(svg.starts_with("<svg "), "a standalone document: {svg}");
        assert!(
            svg.contains("<rect x=\"10\" y=\"20\" width=\"30\" height=\"40\""),
            "the rect leaves at its own place: {svg}"
        );
    }

    /// Covers `TOILE-IO-025`: each connector on the board is named in the
    /// report, and the two shapes it joins still leave.
    #[test]
    fn a_connector_is_named_in_the_report() {
        use crate::logic::model::{Anchor, EndCaps, PathType};

        let mut st = ToileState::new();
        let ids: Vec<_> = [0.0, 100.0]
            .into_iter()
            .map(|x| {
                st.handle_command(ToileCommand::CreateElement {
                    kind: ElementKind::Shape(ShapeKind::Rect),
                    at: Point::new(x, 0.0),
                    size: Size::new(30.0, 30.0),
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
                .expect("a minted id")
            })
            .collect();
        st.handle_command(ToileCommand::CreateConnector {
            from: ids[0].clone(),
            from_anch: Anchor::Right,
            to: ids[1].clone(),
            to_anch: Anchor::Left,
            path_type: PathType::Straight,
            arrows: EndCaps {
                start: crate::message::Cap::Plain,
                end: crate::message::Cap::Arrow,
            },
        });

        let out = st.handle_command(ToileCommand::Interop(InteropRequest::Export(
            ToileExport::Svg,
        )));
        let Some(ToileEvent::Interop(InteropAnswer::ExportReady { file, report })) =
            out.display.first()
        else {
            panic!("the export answers with the file: {:?}", out.display);
        };
        assert_eq!(report.entries().len(), 1, "{report:?}");
        assert_eq!(
            report
                .count(ToileConstruct::Connector, Outcome::Dropped)
                .get(),
            1,
            "{report:?}"
        );
        let svg = String::from_utf8(file.data.as_bytes().to_vec()).expect("the document is text");
        assert_eq!(svg.matches("<rect").count(), 2, "{svg}");
    }

    /// Reading the board twice yields the same bytes, and leaves the board as it
    /// was (`TOILE-IO-004`, `TOILE-IO-024`).
    #[test]
    fn exporting_twice_yields_the_same_bytes_and_changes_nothing() {
        let mut st = board_with_a_rect();
        let before = st.element_count();
        let (_, first) = exported(&mut st);
        let (_, second) = exported(&mut st);
        assert_eq!(first, second);
        assert_eq!(st.element_count(), before);
    }

    /// The deck reaches the PDF serializer and comes back as a document
    /// (`TOILE-IO-029`). The bytes are opaque here, because `logic::interop`
    /// covers the page walk. This test holds the seam instead: a live board, the
    /// real font faces, and a file at the end of it.
    #[cfg(feature = "pdf")]
    #[test]
    fn the_deck_exports_as_a_pdf_document() {
        let st = board_with_a_rect();
        let mut fonts = ambre_draw::FontContext::default();
        let pdf = st
            .export_pdf_document(Default::default(), &mut fonts, &host_converter())
            .expect("a board with content renders");
        let pdf = pdf.as_bytes();
        assert!(pdf.starts_with(b"%PDF-"), "a PDF header: {:?}", &pdf[..8]);
    }

    /// A PDF export writes nothing: the oplog's version vector is the same after
    /// it, so no delta can leave for a peer (`PDF-016`). The receiver is shared,
    /// but the `LoroDoc` mutates through one, so the type does not hold this.
    #[cfg(feature = "pdf")]
    #[test]
    fn a_pdf_export_leaves_the_oplog_unchanged() {
        let st = board_with_a_rect();
        let before = st.doc.oplog_vv();
        let mut fonts = ambre_draw::FontContext::default();
        st.export_pdf_document(Default::default(), &mut fonts, &host_converter())
            .expect("a board with content renders");
        assert_eq!(st.doc.oplog_vv(), before, "the export wrote to the board");
        assert_eq!(st.doc.state_vv(), before);
    }

    /// A board holding one text element whose body is `body`.
    #[cfg(feature = "pdf")]
    fn board_with_text(body: &str) -> ToileState {
        let mut st = ToileState::new();
        st.handle_command(ToileCommand::CreateElement {
            kind: ElementKind::Text,
            at: Point::new(10.0, 20.0),
            size: Size::new(300.0, 80.0),
            content: InitContent::Text(body.to_owned()),
            fill: None,
            correlate: None,
        });
        st
    }

    /// A sealed context holding the shipped faces in `files`, as the Logic
    /// thread holds what the UI thread delivered to it.
    #[cfg(feature = "pdf")]
    fn held_faces(files: &[&str]) -> ambre_draw::FontContext {
        let mut font_cx = ambre_draw::sealed_font_context();
        for file in files {
            ambre_draw::register_face(
                &mut font_cx,
                &ambre_draw::FontFace::new(ambre_core::Payload::new(shipped_face(file))),
            );
        }
        font_cx
    }

    /// The bytes of one face the application ships.
    #[cfg(feature = "pdf")]
    fn shipped_face(file: &str) -> Vec<u8> {
        const DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../ambre-app/fonts/");
        std::fs::read(format!("{DIR}{file}")).expect("the workspace ships the face")
    }

    /// The deck of `st`, recorded one page per step as the Logic thread
    /// records it.
    #[cfg(feature = "pdf")]
    fn recorded(
        st: &ToileState,
        font_cx: &mut ambre_draw::FontContext,
    ) -> ambre_draw::list::PrintDocument {
        let mut recording = st
            .record_deck(Default::default())
            .expect("the board records");
        while !recording.is_recorded() {
            recording.record_next(font_cx);
        }
        recording.into_document()
    }

    /// The glyph ids of every run of `document` whose text holds `c`, each
    /// with the bytes of the face the run draws in.
    #[cfg(feature = "pdf")]
    fn runs_holding(
        document: &ambre_draw::list::PrintDocument,
        c: char,
    ) -> Vec<(Vec<u8>, Vec<usize>)> {
        document
            .pages
            .iter()
            .flatten()
            .filter_map(|op| match op {
                ambre_draw::list::DrawOp::Glyphs {
                    face, glyphs, text, ..
                } if text.contains(c) => Some((
                    document.face(*face)?.bytes.as_bytes().to_vec(),
                    glyphs.iter().map(|g| g.id.to_usize()).collect(),
                )),
                _ => None,
            })
            .collect()
    }

    /// An export over a text no held face covers arms no fetch, at each
    /// export, and draws the missing glyph of a held face (`PDF-019`).
    ///
    /// The thread's count of armed fetches is the spy. The export runs on the
    /// calling thread, so the count reads what it armed. A fetching cache
    /// meeting the same text on the same thread is the control.
    #[cfg(feature = "pdf")]
    #[test]
    fn an_export_arms_no_fetch_for_a_face_the_thread_lacks() {
        let armed = ambre_canvas::fonts::fetches_armed_on_this_thread;
        let st = board_with_text("\u{4e2d}\u{6587}");
        let mut font_cx = held_faces(&["NotoSans-Regular.ttf"]);
        let before = armed();
        for _ in 0..2 {
            let pdf = st
                .export_pdf_document(Default::default(), &mut font_cx, &host_converter())
                .expect("a text no face covers still exports");
            assert!(pdf.as_bytes().starts_with(b"%PDF-"));
            assert_eq!(armed(), before, "the export armed a fetch");
        }
        let document = recorded(&st, &mut font_cx);
        let runs = runs_holding(&document, '\u{4e2d}');
        assert!(!runs.is_empty(), "the text drew no run");
        assert!(
            runs.iter().all(|(_, ids)| ids.iter().all(|&id| id == 0)),
            "a codepoint no held face covers draws the missing glyph: {runs:?}"
        );
        assert_eq!(armed(), before);

        ambre_canvas::TextCache::new().get_or_build(
            &mut font_cx,
            "\u{4e2d}\u{6587}",
            ambre_canvas::Look::body(
                ambre_core::Px::<ambre_core::View>::new(14.0),
                ambre_core::Px::<ambre_core::View>::new(200.0),
            ),
        );
        assert_eq!(armed(), before.incremented(), "the spy counts a fetch");
    }

    /// An export draws a script in the face the thread holds for it, and
    /// arms no fetch for it either (`PDF-019`).
    #[cfg(feature = "pdf")]
    #[test]
    fn an_export_draws_a_script_in_the_face_the_thread_holds() {
        let armed = ambre_canvas::fonts::fetches_armed_on_this_thread;
        let st = board_with_text("\u{4e2d}\u{6587}");
        let mut font_cx = held_faces(&["NotoSans-Regular.ttf", "NotoSansSC-Regular.otf"]);
        let before = armed();
        let document = recorded(&st, &mut font_cx);
        let runs = runs_holding(&document, '\u{4e2d}');
        let cjk = shipped_face("NotoSansSC-Regular.otf");
        assert!(!runs.is_empty(), "the text drew no run");
        for (face, ids) in &runs {
            assert!(face == &cjk, "the run is not in the held CJK face");
            assert!(
                !ids.is_empty() && !ids.contains(&0),
                "the run drew tofu: {ids:?}"
            );
        }
        assert_eq!(armed(), before, "the export armed a fetch");
    }

    /// `TOILE-IO-048`: a board with nothing on it has no page to draw. The
    /// caller turns the refusal into a message rather than handing the user an
    /// empty file.
    #[cfg(feature = "pdf")]
    #[test]
    fn an_empty_board_has_no_pdf_to_export() {
        let st = ToileState::new();
        let mut fonts = ambre_draw::FontContext::default();
        assert!(matches!(
            st.export_pdf_document(Default::default(), &mut fonts, &host_converter()),
            Err(crate::logic::interop::PdfExportError::Empty)
        ));
    }

    /// A deck of `n` slides side by side, each frame holding one rect.
    #[cfg(feature = "pdf")]
    fn deck(n: usize) -> ToileState {
        let mut st = ToileState::with_peer(ambre_core::PeerId::new(1));
        let mut order = Vec::new();
        for i in 0..n {
            let x = i as f64 * 400.0;
            let out = st.handle_command(ToileCommand::CreateElement {
                kind: ElementKind::Frame,
                at: Point::new(x, 0.0),
                size: Size::new(320.0, 180.0),
                content: InitContent::None,
                fill: None,
                correlate: None,
            });
            let id = out
                .display
                .into_iter()
                .find_map(|e| match e {
                    ToileEvent::ElementCreated { id, .. } => Some(id),
                    _ => None,
                })
                .expect("a create names its element");
            order.push(st.frame(&id).expect("a frame"));
            st.handle_command(ToileCommand::CreateElement {
                kind: ElementKind::Shape(ShapeKind::Rect),
                at: Point::new(x + 20.0, 20.0),
                size: Size::new(100.0, 60.0),
                content: InitContent::None,
                fill: None,
                correlate: None,
            });
        }
        st.handle_command(ToileCommand::SetSlideOrder { order });
        st
    }

    /// `TOILE-IO-099`: a recording takes one page per step and says how far
    /// it got after each. It ends on the document one call records.
    #[cfg(feature = "pdf")]
    #[test]
    fn a_deck_records_one_page_per_step() {
        let st = deck(3);
        let mut fonts = ambre_draw::FontContext::default();
        let mut recording = st.record_deck(Default::default()).expect("three pages");
        let mut seen = Vec::new();
        while !recording.is_recorded() {
            recording.record_next(&mut fonts);
            let progress = recording.progress();
            seen.push((progress.done().get(), progress.total().get()));
        }
        assert_eq!(seen, vec![(1, 3), (2, 3), (3, 3)]);
        let stepped = recording.into_document();
        assert_eq!(stepped.pages.len(), 3, "one page per slide");
        let whole = crate::logic::interop::record_pdf(
            &st.pdf_pages(Default::default()),
            Default::default(),
            ambre_canvas::Theme::Light.palette(),
            &mut fonts,
            &mut ambre_canvas::TextCache::new(),
            &st.decoded_media(),
        )
        .expect("three pages");
        assert_eq!(stepped, whole, "the steps record what one call records");
    }

    /// `TOILE-IO-133`: an edit made while a recording runs reaches the board
    /// and no page of the file. The file is the deck as it stood at the
    /// request.
    #[cfg(feature = "pdf")]
    #[test]
    fn an_edit_mid_export_reaches_the_board_and_no_page() {
        let mut st = deck(2);
        let mut fonts = ambre_draw::FontContext::default();
        let asked = st.record_deck(Default::default()).expect("two pages");
        let before = st.element_count();
        let mut recording = asked;
        recording.record_next(&mut fonts);
        // A shape dropped on the second slide, between the two pages.
        st.handle_command(ToileCommand::CreateElement {
            kind: ElementKind::Shape(ShapeKind::Rect),
            at: Point::new(500.0, 100.0),
            size: Size::new(50.0, 50.0),
            content: InitContent::None,
            fill: None,
            correlate: None,
        });
        assert!(st.element_count() > before, "the edit landed on the board");
        recording.record_next(&mut fonts);
        assert!(recording.is_recorded());
        let mid_export = recording.into_document();
        let record_now = |st: &ToileState, fonts: &mut ambre_draw::FontContext| {
            let mut recording = st.record_deck(Default::default()).expect("two pages");
            while !recording.is_recorded() {
                recording.record_next(fonts);
            }
            recording.into_document()
        };
        let after_edit = record_now(&st, &mut fonts);
        assert_ne!(
            mid_export.pages.get(1),
            after_edit.pages.get(1),
            "the edit changes the second slide"
        );
        st.handle_command(ToileCommand::Undo);
        assert_eq!(
            mid_export,
            record_now(&st, &mut fonts),
            "the file is the deck as asked"
        );
    }

    /// An empty board still answers with a file: a gesture that produces nothing
    /// leaves the user unable to tell it arrived.
    #[test]
    fn an_empty_board_exports_an_empty_document() {
        let mut st = ToileState::new();
        let (_, data) = exported(&mut st);
        let svg = String::from_utf8(data).expect("the document is text");
        assert!(svg.starts_with("<svg "), "{svg}");
        assert!(svg.ends_with("</svg>\n"), "{svg}");
    }
}

#[cfg(test)]
mod media_request_tests {
    use crate::logic::command::{ToileCommand, ToileEvent};
    use crate::logic::interop::{ContentHash, ImportBatch, MediaBlob};

    use super::super::ToileState;
    use ambre_core::Validated;

    /// A media entry whose thumbnail is a real PNG, so the decode path under
    /// test is the one that runs in the application.
    /// The address of the one picture these boards carry.
    const IMG: &str = "1111111111111111111111111111111111111111111111111111111111111111";

    fn entry_with_thumb(hash: &str, w: u32, h: u32) -> MediaBlob {
        let img = image::RgbaImage::from_pixel(w, h, image::Rgba([9, 8, 7, 255]));
        let mut thumb = Vec::new();
        image::DynamicImage::ImageRgba8(img)
            .write_to(
                &mut std::io::Cursor::new(&mut thumb),
                image::ImageFormat::Png,
            )
            .unwrap();
        MediaBlob {
            media: crate::logic::command::MediaRef::Stored(
                Validated::<ContentHash>::parse(hash).expect("a hash"),
            ),
            plaintext: ambre_core::Payload::<ambre_core::MediaBytes>::new(vec![0u8; 4]),
            mime: ambre_core::MediaType::from("image/png"),
            size: ambre_core::Size::new(ambre_core::Px::new(w), ambre_core::Px::new(h)),
            thumb: ambre_core::Payload::<ambre_core::ThumbnailPng>::new(thumb),
        }
    }

    fn board_with_media(hash: &str, w: u32, h: u32) -> ToileState {
        let mut st = ToileState::new();
        st.apply_import(ImportBatch {
            media: vec![entry_with_thumb(hash, w, h)],
            ..ImportBatch::new(ambre_bus::interop::FileFormat::Svg)
        });
        st
    }

    /// The board answers a request with the thumbnail's own pixels
    /// (`TOILE-IO-034`). This is the crossing the screen path was missing. The
    /// pixels live on the Logic thread, and the store the render walk reads
    /// lives on the UI one.
    #[test]
    fn a_media_request_answers_with_decoded_pixels() {
        let mut st = board_with_media(IMG, 4, 2);
        let out = st.handle_command(ToileCommand::RequestMedia {
            hashes: vec![Validated::<ContentHash>::parse(IMG).expect("a hash")],
        });
        let answer = out
            .display
            .iter()
            .find_map(|e| match e {
                ToileEvent::MediaDecoded { hash, size, pixels } => {
                    Some((hash.clone(), *size, pixels.len()))
                }
                _ => None,
            })
            .expect("the request is answered");
        assert_eq!(answer.0.as_str(), IMG);
        assert_eq!(
            answer.1,
            ambre_core::Size::new(ambre_core::Px::new(4), ambre_core::Px::new(2))
        );
        // RGBA8, row-major, no padding: exactly four bytes a texel.
        assert_eq!(
            answer.2,
            ambre_core::Count::<ambre_core::Octets>::new(4 * 2 * 4)
        );
    }

    /// `TOILE-IO-103`: the board answers an address it does not carry with nothing. The element
    /// then keeps the placeholder it was already painting, instead of leaving
    /// the view to wait on a reply that never comes.
    #[test]
    fn an_unknown_address_is_answered_with_nothing() {
        let mut st = board_with_media(IMG, 2, 2);
        let out = st.handle_command(ToileCommand::RequestMedia {
            hashes: vec![Validated::<ContentHash>::parse("2".repeat(64)).expect("a hash")],
        });
        assert!(out.display.is_empty());
    }

    /// Asking for a picture changes no document state (`TOILE-IO-024`). The
    /// command only reads, so it emits no CRDT update and nothing to persist.
    #[test]
    fn a_media_request_writes_nothing() {
        let mut st = board_with_media(IMG, 2, 2);
        let out = st.handle_command(ToileCommand::RequestMedia {
            hashes: vec![Validated::<ContentHash>::parse(IMG).expect("a hash")],
        });
        assert!(out.update.is_none(), "a read-only command exports no delta");
        assert!(!ToileCommand::RequestMedia { hashes: Vec::new() }.is_mutating());
    }

    /// `TOILE-IO-037`: a decoded picture never enters what local persistence
    /// stores. The pixels go to the view, and the snapshot after the answer is
    /// the snapshot before it, byte for byte, with no delta owed.
    #[test]
    fn a_decoded_picture_never_enters_the_persisted_document() {
        let mut st = board_with_media(IMG, 4, 2);
        let before = st.export_snapshot();
        let out = st.handle_command(ToileCommand::RequestMedia {
            hashes: vec![Validated::<ContentHash>::parse(IMG).expect("a hash")],
        });
        assert!(
            out.display
                .iter()
                .any(|e| matches!(e, ToileEvent::MediaDecoded { .. })),
            "the picture was decoded"
        );
        assert!(out.update.is_none(), "the answer owes persistence a delta");
        assert_eq!(
            st.export_snapshot(),
            before,
            "the decoded pixels reached the persisted document"
        );
    }
}

/// The showcase bundle (`toile-import-export` section 1.7).
#[cfg(test)]
mod showcase_tests {
    use kurbo::{Point, Size};

    use crate::logic::command::{InitContent, ToileCommand, ToileEvent};
    use crate::logic::model::{ElementKind, ShapeKind};
    use crate::message::{ToileConstruct, ToileExport};
    use ambre_bus::interop::{InteropAnswer, InteropRequest, Outcome};
    use ambre_core::crdt::CrdtName as _;

    use super::super::ToileState;

    /// A board that holds one rectangle.
    fn board_with_a_rect() -> ToileState {
        let mut st = ToileState::new();
        created(
            &mut st,
            ElementKind::Shape(ShapeKind::Rect),
            InitContent::None,
        );
        st
    }

    /// A board that carries a text element, a shape and a sticky note.
    fn showcase_board() -> (ToileState, ambre_core::id::Id<ambre_core::id::Element>) {
        let mut st = board_with_a_rect();
        st.set_user_id(ambre_core::id::Id::<ambre_core::id::User>::mint());
        let text = created(
            &mut st,
            ElementKind::Text,
            InitContent::Text("quarterly plan".to_owned()),
        );
        created(
            &mut st,
            ElementKind::Sticky,
            InitContent::Text("a sticky".to_owned()),
        );
        (st, text)
    }

    /// Create one element of `kind` and answer the id the board minted.
    fn created(
        st: &mut ToileState,
        kind: ElementKind,
        content: InitContent,
    ) -> ambre_core::id::Id<ambre_core::id::Element> {
        st.handle_command(ToileCommand::CreateElement {
            kind,
            at: Point::new(200.0, 20.0),
            size: Size::new(120.0, 40.0),
            content,
            fill: None,
            correlate: None,
        })
        .display
        .into_iter()
        .find_map(|e| match e {
            ToileEvent::ElementCreated { id, .. } => Some(id),
            _ => None,
        })
        .expect("a minted id")
    }

    /// The whole answer of a showcase export: the outcome, then the page.
    fn showcase(
        st: &mut ToileState,
    ) -> (
        crate::logic::command::ToileOutcome,
        String,
        crate::message::ToileReport,
    ) {
        let out = st.handle_command(ToileCommand::Interop(InteropRequest::Export(
            ToileExport::Showcase,
        )));
        let (name, page, report) = out
            .display
            .iter()
            .find_map(|e| match e {
                ToileEvent::Interop(InteropAnswer::ExportReady { file, report }) => Some((
                    file.filename.to_string(),
                    String::from_utf8(file.data.as_bytes().to_vec()).expect("a page is text"),
                    report.clone(),
                )),
                _ => None,
            })
            .expect("the export answers with the page");
        assert_eq!(name, "board.html");
        (out, page, report)
    }

    /// Every value of an attribute that names an address. The scan reads
    /// `href`, `xlink:href`, `src`, `srcset`, `action`, `data`, `poster` and
    /// a CSS `url(`.
    fn addresses(page: &str) -> Vec<String> {
        let mut out = Vec::new();
        for name in [
            " href=\"",
            ":href=\"",
            " src=\"",
            " srcset=\"",
            " action=\"",
            " data=\"",
            " poster=\"",
            "url(",
        ] {
            let mut rest = page;
            while let Some(at) = rest.find(name) {
                let tail = &rest[at + name.len()..];
                let end = tail.find(['"', ')']).unwrap_or(tail.len());
                out.push(tail[..end].to_owned());
                rest = &tail[end..];
            }
        }
        out
    }

    /// `TOILE-IO-019`, `TOILE-IO-138`: the page carries the board in the
    /// clear. The SVG export of the same board draws it, and no script does.
    #[test]
    fn a_showcase_page_draws_the_board_in_the_clear_with_no_script() {
        let (mut st, _) = showcase_board();
        let (_, page, _) = showcase(&mut st);
        let svg = st.svg_of_board().document;
        assert!(page.contains(&svg), "the page draws the SVG export whole");
        assert!(page.contains(">quarterly plan</text>"), "{page}");
        assert!(!page.contains("<script"), "no script draws the page");
        assert!(page.starts_with("<!DOCTYPE html>"), "a standalone page");
    }

    /// `TOILE-IO-095`: the page names no address a browser would load, and
    /// its content policy refuses every fetch before any content is read.
    #[test]
    fn a_showcase_page_loads_nothing_from_the_network() {
        let (mut st, _) = showcase_board();
        st.handle_command(ToileCommand::Interop(InteropRequest::Import(
            crate::message::ToileImport {
                format: crate::message::ToileImportFormat::Svg,
                data: ambre_core::Payload::new(
                    br#"<svg xmlns="http://www.w3.org/2000/svg"><rect width="5" height="5"/></svg>"#
                        .to_vec(),
                ),
                at: Point::ZERO,
            },
        )));
        let (_, page, _) = showcase(&mut st);
        let head = page.split("</head>").next().unwrap_or_default();
        let policy = head
            .find("Content-Security-Policy")
            .expect("the head declares a policy");
        assert!(
            head.contains("content=\"default-src 'none'; img-src data:;"),
            "{head}"
        );
        for element in ["<style", "<title", "<link", "<base"] {
            assert!(
                head.find(element).is_none_or(|at| at > policy),
                "{element} precedes the policy: {head}"
            );
        }
        for address in addresses(&page) {
            assert!(
                address.starts_with("data:"),
                "an address to load: {address}"
            );
        }
        for element in [
            "<link", "<iframe", "<object", "<embed", "<img", "<base", "<script",
        ] {
            assert!(!page.contains(element), "{element} in {page}");
        }
    }

    /// The address scan of `TOILE-IO-095` finds what it looks for: a page with
    /// a remote picture fails it.
    #[test]
    fn the_address_scan_finds_a_remote_picture() {
        let page = "<svg><image href=\"https://relay.example/x.png\"/></svg>";
        assert_eq!(addresses(page), ["https://relay.example/x.png"]);
    }

    /// `TOILE-IO-018`, `TOILE-IO-137`: the page is the picture alone. A sticky note leaves
    /// no word in it, since the SVG export draws no sticky note. Neither
    /// does a deleted element, a comment or a reaction, and no template or
    /// script carries any state. The board's own snapshot holds the deleted
    /// text in the clear, so a page that embedded it would show it.
    #[test]
    fn a_showcase_page_embeds_no_board_state() {
        let (mut st, text) = showcase_board();
        let gone = created(
            &mut st,
            ElementKind::Text,
            InitContent::Text("zx-deleted-before-export".to_owned()),
        );
        st.handle_command(ToileCommand::DeleteElements { ids: vec![gone] });
        st.handle_command(ToileCommand::CreateComment {
            anchor: crate::logic::command::CommentAnchor::Element(text.clone()),
            body: "zx-comment-body".to_owned(),
            ts: ambre_core::Instant::<ambre_bus::Authored>::new(0),
        });
        st.handle_command(ToileCommand::React {
            target: text,
            reaction: Some(crate::logic::command::Reaction {
                kind: crate::logic::command::ReactionKind::Emoji(ambre_bus::Emoji::from(
                    "zx-reaction",
                )),
                ts: ambre_core::Instant::<ambre_bus::Authored>::new(0),
            }),
        });
        assert!(!st.doc.get_map(super::Root::Comments.as_str()).is_empty());
        assert!(!st.doc.get_map(super::Root::Reactions.as_str()).is_empty());
        let full = st
            .doc
            .export(loro::ExportMode::Snapshot)
            .expect("the full snapshot exports");
        let needle = b"zx-deleted-before-export";
        assert!(
            full.windows(needle.len()).any(|w| w == needle),
            "the board's snapshot holds the deleted text, so the scan can see it"
        );
        let (_, page, _) = showcase(&mut st);
        for absent in [
            "a sticky",
            "zx-deleted-before-export",
            "zx-comment-body",
            "zx-reaction",
            "<template",
            "<script",
        ] {
            assert!(!page.contains(absent), "the page holds {absent}");
        }
        let body = page
            .split("<body>")
            .nth(1)
            .and_then(|rest| rest.split("</body>").next())
            .expect("the page has a body");
        assert_eq!(
            body.trim(),
            st.svg_of_board().document.trim(),
            "the body is the picture and nothing else"
        );
    }

    /// `ZK-037`, `TOILE-IO-024`: the page reaches the UI thread's download path
    /// alone. The outcome carries no delta, no presence sample and no request
    /// for the Sync worker, and the board keeps its comment thread.
    #[test]
    fn a_showcase_page_reaches_the_download_path_alone() {
        let (mut st, text) = showcase_board();
        st.handle_command(ToileCommand::CreateComment {
            anchor: crate::logic::command::CommentAnchor::Element(text),
            body: "kept".to_owned(),
            ts: ambre_core::Instant::<ambre_bus::Authored>::new(0),
        });
        let before = st.doc.get_deep_value();
        let (out, _, _) = showcase(&mut st);
        assert!(out.update.is_none(), "a showcase export writes no delta");
        assert!(out.ephemeral.is_none(), "nor a presence sample");
        assert!(out.requests.is_empty(), "nor a request for the Sync worker");
        assert_eq!(out.display.len(), 1, "one answer, for the UI thread");
        assert_eq!(st.doc.get_deep_value(), before, "the board is unchanged");
    }

    /// `TOILE-IO-139`: what the page cannot draw is named in its report,
    /// under the page's own format.
    #[test]
    fn a_showcase_page_names_what_it_cannot_draw() {
        let (mut st, _) = showcase_board();
        let (_, _, report) = showcase(&mut st);
        assert_eq!(report.format(), ambre_bus::interop::FileFormat::Html);
        assert_eq!(
            report.count(ToileConstruct::StickyNote, Outcome::Dropped),
            ambre_core::Count::new(1)
        );
    }
}
