//! The whole PDF of a recorded [`PrintDocument`]: each page's ops replayed on
//! a [`KrillaSurface`].
//!
//! The replay is the converter module's half of the PDF export
//! (`ambre-spec-architecture.md section 2`, the converter module). The Logic
//! thread records the pages through `ambre_draw::list::ListRecorder`, and this
//! function writes them. A page drawn live on a [`KrillaSurface`] and the same
//! page recorded then replayed reach krilla through the same calls.

use ambre_client_core::FileBytes;
use ambre_core::Payload;
use ambre_core::index::{Len, Pos};
use ambre_draw::list::{
    DrawOp, PrintDocument, PrintLink, PrintMeta, PrintTarget, PrintedPages, color_of,
};
use ambre_draw::{DrawSurface, EncodedImage, ImageKey, ImageSource};
use krilla::Data;
use krilla::action::{Action, LinkAction};
use krilla::annotation::{Annotation, LinkAnnotation, Target};
use krilla::destination::XyzDestination;
use krilla::geom::{Point, Rect as KRect};
use krilla::metadata::Metadata;
use krilla::page::PageSettings;
use krilla::text::Font;

use crate::KrillaSurface;

/// Why a recorded document wrote no PDF.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PdfWriteError {
    /// The document holds no page.
    #[error("no pages to export")]
    Empty,
    /// krilla refused to assemble the bytes.
    #[error("failed to serialize the PDF: {0}")]
    Serialize(String),
}

/// Write `document` to PDF bytes, one page per op list.
///
/// An op that names a face or an image past its table draws nothing. A face
/// krilla cannot read draws nothing on the live surface either. The clip and
/// transform stacks stay balanced whatever the ops: the replay closes what a
/// page left open.
///
/// Each link becomes an annotation of its page. Three links write nothing:
/// one on a page past the last, one that leads there, and one with no area.
/// krilla panics on a place past the last page. The metadata writes
/// the title and the language the document names, and nothing else
/// (`ambre-spec-pdf-export.md section 3`).
///
/// # Errors
///
/// [`PdfWriteError::Empty`] for a document with no page, and
/// [`PdfWriteError::Serialize`] when krilla refuses the bytes.
pub fn write_document(document: &PrintDocument) -> Result<Payload<FileBytes>, PdfWriteError> {
    let Some(size) = document.page.filter(|_| !document.pages.is_empty()) else {
        return Err(PdfWriteError::Empty);
    };
    // One krilla font per face, built once: krilla subsets each font once
    // per document, whatever the run count.
    let fonts: Vec<Option<Font>> = document
        .faces
        .iter()
        .map(|face| Font::new(Data::from(face.bytes.as_bytes().to_vec()), face.index.get()))
        .collect();
    let settings = PageSettings::from_wh(size.width.get(), size.height.get())
        .or_else(|| PageSettings::from_wh(1.0, 1.0))
        .unwrap_or_default();
    let mut pdf = krilla::Document::new();
    if let Some(metadata) = metadata_of(&document.meta) {
        pdf.set_metadata(metadata);
    }
    let count = Len::<PrintedPages>::of(document.pages.len());
    for (index, ops) in document.pages.iter().enumerate() {
        let here = Pos::<PrintedPages>::from_usize(index);
        let mut page = pdf.start_page_with(settings.clone());
        {
            let mut surface = page.surface();
            let mut s = KrillaSurface::new(&mut surface);
            replay(&mut s, document, &fonts, ops);
        }
        for link in document.links.iter().filter(|link| link.page == here) {
            if let Some(annotation) = annotation_of(link, count) {
                page.add_annotation(annotation);
            }
        }
        page.finish();
    }
    pdf.finish()
        .map(Payload::<FileBytes>::new)
        .map_err(|e| PdfWriteError::Serialize(format!("{e:?}")))
}

/// The metadata `meta` names, or `None` when it names nothing. krilla then
/// writes no information dictionary and no XMP stream.
fn metadata_of(meta: &PrintMeta) -> Option<Metadata> {
    let title = meta.title.clone().filter(|title| !title.is_empty());
    let language = meta.language.as_str().map(str::to_owned);
    if title.is_none() && language.is_none() {
        return None;
    }
    let metadata = Metadata::new();
    let metadata = match title {
        Some(title) => metadata.title(title),
        None => metadata,
    };
    Some(match language {
        Some(language) => metadata.language(language),
        None => metadata,
    })
}

/// The annotation of `link` in a file of `count` pages, or `None` when it
/// has no area or leads past the last page.
fn annotation_of(link: &PrintLink, count: Len<PrintedPages>) -> Option<Annotation> {
    let area = link.area;
    if area.width() <= 0.0 || area.height() <= 0.0 {
        return None;
    }
    // krilla takes the rectangle as `f32`.
    let rect = KRect::from_ltrb(
        area.x0 as f32,
        area.y0 as f32,
        area.x1 as f32,
        area.y1 as f32,
    )?;
    let target = match &link.target {
        PrintTarget::Address(url) => {
            Target::Action(Action::Link(LinkAction::new(url.as_str().to_owned())))
        }
        PrintTarget::Place { page, y } => {
            if *page >= count.as_end() {
                return None;
            }
            let point = Point::from_xy(0.0, y.get());
            Target::Destination(XyzDestination::new(page.to_usize(), point).into())
        }
    };
    Some(LinkAnnotation::new(rect, target).into())
}

/// Replay one page's ops, then pop whatever the page left pushed.
fn replay(
    s: &mut KrillaSurface<'_, '_>,
    document: &PrintDocument,
    fonts: &[Option<Font>],
    ops: &[DrawOp],
) {
    // Each push the replay forwarded, so an unbalanced list cannot pop the
    // page's own state.
    let mut depth = 0usize;
    for op in ops {
        match op {
            DrawOp::FillRect { rect, color } => s.fill_rect(*rect, color_of(*color)),
            DrawOp::StrokeRect { rect, color, width } => {
                s.stroke_rect(*rect, color_of(*color), *width)
            }
            DrawOp::StrokeLine { line, color, width } => {
                s.stroke_line(*line, color_of(*color), *width)
            }
            DrawOp::Glyphs {
                face,
                size,
                glyphs,
                text,
                color,
                transform,
            } => {
                if let Some(Some(font)) = fonts.get(face.to_usize()) {
                    s.draw_glyph_list(
                        font.clone(),
                        *size,
                        glyphs,
                        text,
                        color_of(*color),
                        *transform,
                    );
                }
            }
            DrawOp::PushClip(rect) => {
                depth += 1;
                s.push_clip(*rect);
            }
            DrawOp::PushTransform(transform) => {
                depth += 1;
                s.push_transform(*transform);
            }
            // Both stacks share krilla's one push stack, so either pop closes
            // the last push.
            DrawOp::PopClip | DrawOp::PopTransform => {
                if depth > 0 {
                    depth -= 1;
                    s.pop_clip();
                }
            }
            DrawOp::FillPath { path, color } => s.fill_path(path, &color_of(*color).into()),
            DrawOp::StrokePath {
                path,
                color,
                stroke,
            } => {
                s.stroke_path(path, &color_of(*color).into(), stroke);
            }
            DrawOp::EncodedImage { image, dest } => {
                if let Some(picture) = document.encoded_image(*image) {
                    let source = EncodedImage {
                        key: ImageKey::new(u64::from(image.get())),
                        format: picture.format,
                        bytes: &picture.bytes,
                        size: picture.size,
                    };
                    s.draw_encoded_image(&source, *dest);
                }
            }
            DrawOp::Image { image, dest } => {
                if let Some(picture) = document.image(*image) {
                    let source = ImageSource {
                        key: ImageKey::new(u64::from(image.get())),
                        pixels: &picture.pixels,
                        size: picture.size,
                    };
                    s.draw_image(&source, *dest);
                }
            }
        }
    }
    for _ in 0..depth {
        s.pop_clip();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ambre_core::{PrintLength, Rgba};
    use ambre_draw::Rect;
    use ambre_draw::list::PrintPage;

    fn document(pages: Vec<Vec<DrawOp>>) -> PrintDocument {
        PrintDocument {
            pages,
            ..PrintDocument::new(PrintPage {
                width: PrintLength::new(200.0),
                height: PrintLength::new(100.0),
            })
        }
    }

    #[test]
    fn a_recorded_page_writes_a_pdf() {
        let doc = document(vec![vec![DrawOp::FillRect {
            rect: Rect::new(10.0, 10.0, 50.0, 50.0),
            color: Rgba::from_channels(200, 0, 0, 255),
        }]]);
        let bytes = write_document(&doc).expect("a page writes");
        assert!(bytes.as_bytes().starts_with(b"%PDF-"));
    }

    #[test]
    fn a_document_with_no_page_is_refused() {
        assert_eq!(
            write_document(&document(Vec::new())),
            Err(PdfWriteError::Empty)
        );
    }

    #[test]
    fn an_unbalanced_page_and_a_dangling_index_still_write() {
        let doc = document(vec![vec![
            DrawOp::PopClip,
            DrawOp::PushClip(Rect::new(0.0, 0.0, 10.0, 10.0)),
            DrawOp::Image {
                image: ambre_core::index::Pos::new(3),
                dest: Rect::new(0.0, 0.0, 5.0, 5.0),
            },
        ]]);
        assert!(write_document(&doc).is_ok());
    }

    /// A recorded run arrives from another wasm instance as bytes. A cluster
    /// that cuts its text badly must not reach krilla, which slices the text
    /// with it and would panic (`PDF-018`).
    #[test]
    fn a_cluster_past_its_text_writes_the_shapes_alone() {
        use ambre_core::index::Len;
        use ambre_draw::GlyphCluster;
        use ambre_draw::list::{PrintFace, PrintGlyph};
        const FACE: &str = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../ambre-app/fonts/NotoSans-Regular.ttf"
        );
        let face = std::fs::read(FACE).expect("the workspace ships its faces");
        let glyph = |start: usize, len: usize| PrintGlyph {
            id: ambre_core::index::Pos::new(36),
            advance: ambre_draw::Px::new(10.0),
            x: ambre_draw::Px::new(0.0),
            y: ambre_draw::Px::new(0.0),
            cluster: GlyphCluster {
                start: ambre_core::index::Pos::from_usize(start),
                len: Len::of(len),
            },
        };
        let mut doc = document(vec![vec![DrawOp::Glyphs {
            face: ambre_core::index::Pos::new(0),
            size: ambre_draw::Px::new(12.0),
            // Past the end, then inside the two bytes of "e" with an acute.
            glyphs: vec![glyph(0, 9), glyph(1, 1)],
            text: "\u{e9}".to_owned(),
            color: Rgba::from_channels(0, 0, 0, 255),
            transform: ambre_draw::Affine::IDENTITY,
        }]]);
        doc.faces.push(PrintFace {
            bytes: Payload::new(face),
            index: ambre_core::index::Pos::new(0),
        });
        let bytes = write_document(&doc).expect("a bad cluster still writes");
        assert!(bytes.as_bytes().starts_with(b"%PDF-"));
    }

    /// The written file, parsed back by a reader that shares no code with
    /// krilla.
    fn parsed(bytes: &Payload<FileBytes>) -> lopdf::Document {
        lopdf::Document::load_mem(bytes.as_bytes()).expect("the file parses")
    }

    /// Every annotation of every page: its subtype dictionary as lopdf
    /// reads it.
    fn annotations(file: &lopdf::Document) -> Vec<lopdf::Dictionary> {
        file.get_pages()
            .values()
            .flat_map(|page| file.get_page_annotations(*page).unwrap_or_default())
            .cloned()
            .collect()
    }

    /// The URI an annotation opens, if it opens one.
    fn uri_of(file: &lopdf::Document, annotation: &lopdf::Dictionary) -> Option<String> {
        let action = annotation.get(b"A").ok()?;
        let action = match action {
            lopdf::Object::Reference(id) => file.get_dictionary(*id).ok()?,
            lopdf::Object::Dictionary(d) => d,
            _ => return None,
        };
        let uri = action.get(b"URI").ok()?.as_str().ok()?;
        Some(String::from_utf8_lossy(uri).into_owned())
    }

    /// The catalog entry `key`, as text.
    fn catalog_text(file: &lopdf::Document, key: &[u8]) -> Option<String> {
        let value = file.catalog().ok()?.get(key).ok()?;
        Some(String::from_utf8_lossy(value.as_str().ok()?).into_owned())
    }

    fn square() -> Vec<DrawOp> {
        vec![DrawOp::FillRect {
            rect: Rect::new(1.0, 1.0, 2.0, 2.0),
            color: Rgba::from_channels(0, 0, 0, 255),
        }]
    }

    /// Three links write nothing: one that leads past the last page, one on
    /// a page past the last, and one with no area. krilla would panic on the
    /// first. The address and the place in range each write their annotation.
    #[test]
    fn a_link_past_the_file_writes_nothing_and_the_others_write() {
        use ambre_core::index::Pos;
        use ambre_draw::list::{PrintLink, PrintTarget, PrintedPages};
        let mut doc = document(vec![square(), square()]);
        let place = |page: u32| PrintTarget::Place {
            page: Pos::<PrintedPages>::new(page),
            y: PrintLength::new(40.0),
        };
        let link = |page: u32, area: Rect, target: PrintTarget| PrintLink {
            page: Pos::<PrintedPages>::new(page),
            area,
            target,
        };
        let area = Rect::new(10.0, 10.0, 60.0, 20.0);
        doc.links = vec![
            link(0, area, place(7)),
            link(9, area, place(1)),
            link(0, Rect::new(10.0, 10.0, 10.0, 10.0), place(1)),
        ];
        let none = write_document(&doc).expect("a file with stray links writes");
        assert!(annotations(&parsed(&none)).is_empty(), "a stray link wrote");
        doc.links.push(link(
            0,
            area,
            PrintTarget::Address(ambre_core::LinkUrl::new("https://example.org/".to_owned())),
        ));
        doc.links.push(link(1, area, place(0)));
        let both = parsed(&write_document(&doc).expect("a file with links writes"));
        let written = annotations(&both);
        assert_eq!(written.len(), 2, "{written:?}");
        let uris: Vec<String> = written.iter().filter_map(|a| uri_of(&both, a)).collect();
        assert_eq!(uris, ["https://example.org/"]);
        assert!(written.iter().any(|a| a.has(b"Dest")), "{written:?}");
    }

    /// A document that names neither a title nor a language writes neither,
    /// and one that names a language writes it as the catalog's `/Lang`.
    #[test]
    fn the_file_carries_the_language_it_is_given_and_none_of_its_own() {
        let mut doc = document(vec![square()]);
        let bare = parsed(&write_document(&doc).expect("a page writes"));
        assert_eq!(catalog_text(&bare, b"Lang"), None);
        assert!(
            bare.trailer.get(b"Info").is_err(),
            "an empty meta wrote an Info"
        );
        doc.meta.language = ambre_client_core::DocumentLocale::parse("fr-CA").expect("a tag");
        doc.meta.title = Some("Notes".to_owned());
        let named = parsed(&write_document(&doc).expect("a page writes"));
        assert_eq!(catalog_text(&named, b"Lang").as_deref(), Some("fr-CA"));
    }

    #[test]
    fn the_same_document_writes_the_same_bytes() {
        let doc = document(vec![vec![DrawOp::FillRect {
            rect: Rect::new(1.0, 1.0, 2.0, 2.0),
            color: Rgba::from_channels(0, 0, 0, 255),
        }]]);
        assert_eq!(write_document(&doc), write_document(&doc));
    }
}
