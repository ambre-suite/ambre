//! The whole PDF of a recorded [`PrintDocument`]: each page's ops replayed on
//! a [`KrillaSurface`].
//!
//! The replay is the converter module's half of the PDF export
//! (`ambre-spec-architecture.md section 2`, the converter module). The Logic
//! thread records the pages through `ambre_draw::list::ListRecorder`, and this
//! function writes them. A page drawn live on a [`KrillaSurface`] and the same
//! page recorded then replayed reach krilla through the same calls.

use ambre_core::{FileBytes, Payload};
use ambre_draw::list::{DrawOp, PrintDocument, color_of};
use ambre_draw::{DrawSurface, ImageKey, ImageSource};
use krilla::Data;
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
    for ops in &document.pages {
        let mut page = pdf.start_page_with(settings.clone());
        {
            let mut surface = page.surface();
            let mut s = KrillaSurface::new(&mut surface);
            replay(&mut s, document, &fonts, ops);
        }
        page.finish();
    }
    pdf.finish()
        .map(Payload::<FileBytes>::new)
        .map_err(|e| PdfWriteError::Serialize(format!("{e:?}")))
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

    #[test]
    fn the_same_document_writes_the_same_bytes() {
        let doc = document(vec![vec![DrawOp::FillRect {
            rect: Rect::new(1.0, 1.0, 2.0, 2.0),
            color: Rgba::from_channels(0, 0, 0, 255),
        }]]);
        assert_eq!(write_document(&doc), write_document(&doc));
    }
}
