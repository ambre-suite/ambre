//! Board -> file, the outbound half of `toile-import-export`.
//!
//! The SVG serializer is a pure function over a slice of
//! [`ExportElement`](crate::logic::interop::ExportElement) and nothing built that
//! slice from a live board, so no user could export anything. This is that
//! reader: it walks the paint order, projects each element to the export shape,
//! and hands the bytes back as one display event.
//!
//! Read-only by construction (`TOILE-IO-024`): every access here is a `get`, and
//! the outcome carries no CRDT update.

use kurbo::{BezPath, Rect};

use crate::logic::command::{ToileEvent, ToileOutcome};
use crate::logic::interop::{ExportElement, export_svg};
use crate::logic::model::{ElementId, ElementKind, ElementStyle, NodeId, ShapeKind};

use super::ToileState;
use super::codec::{bin_of, f64_of, kind_from_tags, str_of};

/// The name an SVG export carries. The board's own title is a launcher concern
/// (the Logic thread holds a document, not its listing entry), so the file is
/// named for what it holds.
const SVG_FILENAME: &str = "board.svg";

impl ToileState {
    /// Serialize the board to an SVG document (`TOILE-IO-021`). Emits one
    /// [`ExportComplete`](ToileEvent::ExportComplete) carrying the bytes, which
    /// the UI thread hands to the browser (`TOILE-IO-085`).
    ///
    /// An empty board still exports: a valid, empty SVG is an answer, and
    /// refusing one would leave the user wondering whether the gesture arrived.
    pub(super) fn export_svg_document(&self) -> ToileOutcome {
        let svg = export_svg(&self.export_elements());
        ToileOutcome::display_only(vec![ToileEvent::ExportComplete {
            filename: SVG_FILENAME.to_owned(),
            data: svg.into_bytes(),
        }])
    }

    /// Every element of the board, in paint order, as the serializer's shape.
    /// Connectors are not elements and do not appear here; a group carries no
    /// geometry of its own and the serializer skips it, so its members export
    /// flat -- both are gaps `TOILE-IO-025` records.
    fn export_elements(&self) -> Vec<ExportElement> {
        self.layer_order()
            .iter()
            .filter_map(|node| match node {
                NodeId::Element(id) => self.export_element(id),
                NodeId::Connector(_) => None,
            })
            .collect()
    }

    /// One element, read from the CRDT exactly as the display pass reads it: the
    /// stored box for anything with geometry, the derived union for a group.
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
            style: ElementStyle::from_bytes(&bin_of(&em, "style")),
            path: match kind {
                ElementKind::Shape(ShapeKind::Path) => BezPath::from_svg(&str_of(&em, "path")).ok(),
                _ => None,
            },
            text: match kind {
                ElementKind::Text | ElementKind::Sticky => Some(str_of(&em, "text")),
                _ => None,
            },
            media_hash: match kind {
                ElementKind::Image => Some(str_of(&em, "hash")),
                _ => None,
            },
        })
    }
}

/// The deck as a PDF (`toile-pdf-export`), one page per slide.
///
/// Gated behind the `pdf` feature, and served by the Logic thread rather than by
/// the board alone: the render walk needs the base font faces the Main thread
/// fetched at boot, which live beside the documents and not inside one.
#[cfg(feature = "pdf")]
impl ToileState {
    /// Render the deck at `page` size, or say why there is nothing to render.
    /// Read-only: the walk takes the board by shared reference and the outcome
    /// carries no update (`TOILE-IO-046`).
    ///
    /// The pictures come from the `media` map's inline thumbnails, which is every
    /// pixel this thread holds -- the full-resolution blobs are ciphertext on the
    /// relay, and no cache on this side has them yet (`toile-media`). A page is
    /// therefore no sharper than the board is on screen.
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
    fn decoded_media(&self) -> crate::ui::render::ImageStore {
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
                ToileEvent::ExportComplete { filename, data } => {
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
    /// (`TOILE-IO-029`). The bytes are opaque here -- the page walk is covered
    /// in `logic::interop` -- so this holds the seam: a live board, the real
    /// font faces, and a file at the end of it.
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

    /// A board with nothing on it has no page to draw, and the caller turns the
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
