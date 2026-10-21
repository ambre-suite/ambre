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

use kurbo::{BezPath, Rect};

use crate::logic::command::{ToileEvent, ToileOutcome};
use crate::logic::interop::{ExportElement, export_svg};
use crate::logic::model::{ElementId, ElementKind, NodeId, ShapeKind};

use super::ToileState;
use super::codec::{bin_of, body_of, f64_of, kind_from_tags, str_of};

/// The name an SVG export carries. The board's own title is a launcher concern:
/// the Logic thread holds a document, not its listing entry. The name therefore
/// says what the file holds.
const SVG_FILENAME: &str = "board.svg";

impl ToileState {
    /// Decode the thumbnail stored for one media address, if the document
    /// carries one and it parses (`toile-media`).
    ///
    /// The thumbnail is every pixel this thread holds. The full blob is
    /// ciphertext on the relay, and no path here fetches it. Both the screen and
    /// an exported page therefore get this resolution (`TOILE-IO-100`).
    pub(super) fn decoded_thumbnail(&self, hash: &str) -> Option<(u32, u32, Vec<u8>)> {
        let loro::LoroValue::Map(entries) = self.doc.get_map(super::MEDIA).get_deep_value() else {
            return None;
        };
        let loro::LoroValue::Map(fields) = entries.get(hash)? else {
            return None;
        };
        let loro::LoroValue::Binary(thumb) = fields.get("thumb")? else {
            return None;
        };
        let rgba = image::load_from_memory(thumb).ok()?.to_rgba8();
        Some((rgba.width(), rgba.height(), rgba.into_raw()))
    }

    /// The PNG bytes stored as one media address's thumbnail, if the document
    /// carries one (`toile-media`). Handed to the SVG export, which writes them
    /// inline (`TOILE-IO-087`).
    fn thumbnail_png(&self, hash: &str) -> Option<Vec<u8>> {
        let loro::LoroValue::Map(entries) = self.doc.get_map(super::MEDIA).get_deep_value() else {
            return None;
        };
        let loro::LoroValue::Map(fields) = entries.get(hash)? else {
            return None;
        };
        let loro::LoroValue::Binary(thumb) = fields.get("thumb")? else {
            return None;
        };
        Some(thumb.to_vec())
    }

    /// Serialize the board to an SVG document (`TOILE-IO-021`). Emits one
    /// [`ExportComplete`](ToileEvent::ExportComplete) carrying the bytes, which
    /// the UI thread hands to the browser (`TOILE-IO-085`).
    ///
    /// An empty board still exports: a valid, empty SVG is an answer, and
    /// refusing one would leave the user wondering whether the gesture arrived.
    pub(super) fn export_svg_document(&self) -> ToileOutcome {
        let mut export = export_svg(&self.export_elements());
        // A connector never reaches the serializer, so the board walk names
        // each one itself rather than lose it in silence (`TOILE-IO-025`).
        let connectors = self
            .layer_order()
            .iter()
            .filter(|node| matches!(node, NodeId::Connector(_)))
            .count();
        export.skipped.extend(
            std::iter::repeat_n("connector: no SVG shape expresses it", connectors)
                .map(str::to_owned),
        );
        ToileOutcome::display_only(vec![ToileEvent::ExportComplete {
            filename: SVG_FILENAME.to_owned(),
            data: export.document.into_bytes(),
            // What no SVG shape expresses, named rather than dropped in
            // silence (`TOILE-IO-025`).
            skipped: export.skipped,
        }])
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
    fn export_element(&self, id: &ElementId) -> Option<ExportElement> {
        let em = self.element_map(id)?;
        let type_tag = str_of(&em, "type");
        let kind = kind_from_tags(&type_tag, ShapeKind::from_tag(&str_of(&em, "shape")));
        let rect = if type_tag == "group" {
            self.bounds.get(id).copied()?
        } else {
            let (x, y) = (f64_of(&em, "x"), f64_of(&em, "y"));
            Rect::new(x, y, x + f64_of(&em, "w"), y + f64_of(&em, "h"))
        };
        Some(ExportElement {
            kind,
            rect,
            rotation: f64_of(&em, "rot"),
            style: self.style_read(&bin_of(&em, "style")),
            path: match kind {
                ElementKind::Shape(ShapeKind::Path) => BezPath::from_svg(&str_of(&em, "path")).ok(),
                _ => None,
            },
            text: match kind {
                ElementKind::Text | ElementKind::Sticky => Some(body_of(&em)),
                _ => None,
            },
            media_hash: match kind {
                ElementKind::Image => Some(ambre_core::ContentHash(str_of(&em, "hash"))),
                _ => None,
            },
            // The plaintext thumbnail, re-encoded as the PNG the export writes
            // inline (`TOILE-IO-087`).
            image_png: match kind {
                ElementKind::Image => self.thumbnail_png(&str_of(&em, "hash")),
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
    /// carries no update (`TOILE-IO-046`).
    ///
    /// The pictures come from the `media` map's inline thumbnails, which is every
    /// pixel this thread holds. The full-resolution blobs are ciphertext on the
    /// relay, and no cache on this side has them yet (`toile-media`). A page is
    /// therefore no sharper than the board is on screen.
    ///
    /// # Errors
    ///
    /// [`PdfExportError::Empty`] when the board yields no page, and
    /// [`PdfExportError::Serialize`] when krilla refuses to assemble the bytes.
    pub fn export_pdf_document(
        &self,
        page: crate::logic::interop::PageSize,
        font_cx: &mut ambre_draw::FontContext,
    ) -> Result<Vec<u8>, crate::logic::interop::PdfExportError> {
        let pages = self.pdf_pages(page);
        // The page camera maps world units onto PDF points 1:1, so the text
        // cache runs at unit scale like the export walk itself.
        let mut text_cache = ambre_canvas::TextCache::new(1.0);
        crate::logic::interop::export_pdf(
            &pages,
            page,
            ambre_canvas::Theme::Light.palette(),
            font_cx,
            &mut text_cache,
            &self.decoded_media(),
        )
    }

    /// Every picture the board can draw, keyed by content hash: each media
    /// entry's inline thumbnail, decoded to pixels. An entry whose bytes do not
    /// decode is left out, so its element paints the placeholder box rather than
    /// failing the export.
    pub(super) fn decoded_media(&self) -> crate::ui::render::ImageStore {
        let mut out = crate::ui::render::ImageStore::new();
        let loro::LoroValue::Map(entries) = self.doc.get_map(super::MEDIA).get_deep_value() else {
            return out;
        };
        for (hash, entry) in entries.iter() {
            let loro::LoroValue::Map(fields) = entry else {
                continue;
            };
            let Some(loro::LoroValue::Binary(thumb)) = fields.get("thumb") else {
                continue;
            };
            let Ok(img) = image::load_from_memory(thumb) else {
                continue;
            };
            let rgba = img.to_rgba8();
            out.insert(
                hash.to_string(),
                crate::ui::render::DecodedImage {
                    width: rgba.width(),
                    height: rgba.height(),
                    pixels: rgba.into_raw(),
                },
            );
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use kurbo::{Point, Size};

    use crate::logic::command::{InitContent, ToileCommand, ToileEvent};
    use crate::logic::model::{ElementKind, ShapeKind};

    use super::super::ToileState;

    fn board_with_a_rect() -> ToileState {
        let mut st = ToileState::new();
        st.handle_command(ToileCommand::CreateElement {
            kind: ElementKind::Shape(ShapeKind::Rect),
            at: Point::new(10.0, 20.0),
            size: Size::new(30.0, 40.0),
            content: InitContent::None,
            fill: None,
            correlate: 0,
        });
        st
    }

    fn exported(st: &mut ToileState) -> (String, Vec<u8>) {
        let out = st.handle_command(ToileCommand::ExportSvg);
        assert!(
            out.update.is_none(),
            "an export must emit no CRDT delta (`TOILE-IO-024`)"
        );
        out.display
            .iter()
            .find_map(|e| match e {
                ToileEvent::ExportComplete { filename, data, .. } => {
                    Some((filename.clone(), data.clone()))
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
                    correlate: 0,
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
                start: false,
                end: true,
            },
        });

        let out = st.handle_command(ToileCommand::ExportSvg);
        let Some(ToileEvent::ExportComplete { data, skipped, .. }) = out.display.first() else {
            panic!("the export answers with the file: {:?}", out.display);
        };
        assert_eq!(skipped.len(), 1, "{skipped:?}");
        assert!(skipped[0].contains("connector"), "{skipped:?}");
        let svg = String::from_utf8(data.clone()).expect("the document is text");
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
            .export_pdf_document(Default::default(), &mut fonts)
            .expect("a board with content renders");
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
        st.export_pdf_document(Default::default(), &mut fonts)
            .expect("a board with content renders");
        assert_eq!(st.doc.oplog_vv(), before, "the export wrote to the board");
        assert_eq!(st.doc.state_vv(), before);
    }

    /// A board with nothing on it has no page to draw. The caller turns the
    /// refusal into a message rather than handing the user an empty file.
    #[cfg(feature = "pdf")]
    #[test]
    fn an_empty_board_has_no_pdf_to_export() {
        let st = ToileState::new();
        let mut fonts = ambre_draw::FontContext::default();
        assert!(matches!(
            st.export_pdf_document(Default::default(), &mut fonts),
            Err(crate::logic::interop::PdfExportError::Empty)
        ));
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

    /// A media entry whose thumbnail is a real PNG, so the decode path under
    /// test is the one that runs in the application.
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
            hash: ContentHash(hash.to_owned()),
            plaintext: vec![0u8; 4],
            mime: ambre_core::MediaType::from("image/png"),
            width: w,
            height: h,
            thumb,
        }
    }

    fn board_with_media(hash: &str, w: u32, h: u32) -> ToileState {
        let mut st = ToileState::new();
        st.apply_import(ImportBatch {
            media: vec![entry_with_thumb(hash, w, h)],
            ..Default::default()
        });
        st
    }

    /// The board answers a request with the thumbnail's own pixels
    /// (`TOILE-IO-034`). This is the crossing the screen path was missing. The
    /// pixels live on the Logic thread, and the store the render walk reads
    /// lives on the UI one.
    #[test]
    fn a_media_request_answers_with_decoded_pixels() {
        let mut st = board_with_media("img", 4, 2);
        let out = st.handle_command(ToileCommand::RequestMedia {
            hashes: vec!["img".to_owned()],
        });
        let answer = out
            .display
            .iter()
            .find_map(|e| match e {
                ToileEvent::MediaDecoded {
                    hash,
                    width,
                    height,
                    pixels,
                } => Some((hash.clone(), *width, *height, pixels.len())),
                _ => None,
            })
            .expect("the request is answered");
        assert_eq!(answer.0, "img");
        assert_eq!((answer.1, answer.2), (4, 2));
        // RGBA8, row-major, no padding: exactly four bytes a texel.
        assert_eq!(answer.3, 4 * 2 * 4);
    }

    /// The board answers an address it does not carry with nothing. The element
    /// then keeps the placeholder it was already painting, instead of leaving
    /// the view to wait on a reply that never comes.
    #[test]
    fn an_unknown_address_is_answered_with_nothing() {
        let mut st = board_with_media("img", 2, 2);
        let out = st.handle_command(ToileCommand::RequestMedia {
            hashes: vec!["absent".to_owned()],
        });
        assert!(out.display.is_empty());
    }

    /// Asking for a picture changes no document state (`TOILE-IO-024`). The
    /// command only reads, so it emits no CRDT update and nothing to persist.
    #[test]
    fn a_media_request_writes_nothing() {
        let mut st = board_with_media("img", 2, 2);
        let out = st.handle_command(ToileCommand::RequestMedia {
            hashes: vec!["img".to_owned()],
        });
        assert!(out.update.is_none(), "a read-only command exports no delta");
        assert!(!ToileCommand::RequestMedia { hashes: Vec::new() }.is_mutating());
    }

    /// `TOILE-IO-037`: a decoded picture never enters what local persistence
    /// stores. The pixels go to the view, and the snapshot after the answer is
    /// the snapshot before it, byte for byte, with no delta owed.
    #[test]
    fn a_decoded_picture_never_enters_the_persisted_document() {
        let mut st = board_with_media("img", 4, 2);
        let before = st.export_snapshot();
        let out = st.handle_command(ToileCommand::RequestMedia {
            hashes: vec!["img".to_owned()],
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
