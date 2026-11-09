//! A recorded draw list: the [`DrawSurface`] calls of a page, as data.
//!
//! The PDF writer lives in the converter module, a second wasm instance the
//! Logic thread loads on the first conversion (`ambre-spec-architecture.md
//! section 2`, the converter module). That instance shares no memory with the
//! Logic thread, so a page cannot cross as borrowed glyph runs and pixels. The
//! [`ListRecorder`] turns each call into a [`DrawOp`] that owns its data, and
//! the writer replays the ops on its own surface.
//!
//! A glyph run keeps its shaped glyphs and an index into the face table of
//! the [`PrintDocument`]. Each face thus crosses once, whatever the run count.
//! An image keeps an index into the image table for the same reason.
//!
//! Only a solid brush records. The PDF surface draws nothing for a gradient
//! today, so the list drops what the writer would drop.

use std::collections::HashMap;

use ambre_core::index::{Axis, Pos};
use ambre_core::{FontBytes, Payload, PrintLength, Pt, Px, Rgba, RgbaPixels, Surface};
use kurbo::{Affine, BezPath, Line, Rect, Stroke};
use peniko::{Brush, Color};
use serde::{Deserialize, Serialize};

use crate::cluster::{GlyphCluster, run_text};
use crate::{DrawSurface, GlyphRun, ImageKey, ImageSource, ParleyBrush};

/// The faces of a [`PrintDocument`], counted from zero.
#[derive(Debug, PartialEq, Eq)]
pub enum Faces {}

impl Axis for Faces {
    const NAME: &'static str = "Face";
}

/// The images of a [`PrintDocument`], counted from zero.
#[derive(Debug, PartialEq, Eq)]
pub enum Pictures {}

impl Axis for Pictures {
    const NAME: &'static str = "Picture";
}

/// One font file and the face inside it, as the shaper read it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PrintFace {
    /// The whole font file.
    pub bytes: Payload<FontBytes>,
    /// The face inside a collection file, zero for a single-face file.
    pub index: Pos<FaceInFile>,
}

/// The faces inside one font collection file, counted from zero.
#[derive(Debug, PartialEq, Eq)]
pub enum FaceInFile {}

impl Axis for FaceInFile {
    const NAME: &'static str = "FaceInFile";
}

/// Decoded pixels an image op paints.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PrintImage {
    /// Non-premultiplied RGBA8 samples, row-major, `width * height * 4` bytes.
    pub pixels: Payload<RgbaPixels>,
    /// The native pixel size of the image.
    pub size: ambre_core::Size<Px<ambre_core::Image>>,
}

/// One shaped glyph of a run, at the run's font size.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PrintGlyph {
    /// The glyph id inside its face.
    pub id: Pos<Glyphs>,
    /// How far the pen moves after the glyph.
    pub advance: Px<Surface>,
    /// The glyph's position from the run origin, along the line.
    pub x: Px<Surface>,
    /// The glyph's position from the baseline, downward.
    pub y: Px<Surface>,
    /// The bytes of the run's text that the glyph draws (`PDF-005`).
    pub cluster: GlyphCluster,
}

/// The glyph ids inside one face, as the font numbers them.
#[derive(Debug, PartialEq, Eq)]
pub enum Glyphs {}

impl Axis for Glyphs {
    const NAME: &'static str = "Glyph";
}

/// One recorded [`DrawSurface`] call. Each variant owns what its call borrowed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum DrawOp {
    /// [`DrawSurface::fill_rect`].
    FillRect { rect: Rect, color: Rgba },
    /// [`DrawSurface::stroke_rect`].
    StrokeRect {
        rect: Rect,
        color: Rgba,
        width: Px<Surface>,
    },
    /// [`DrawSurface::stroke_line`].
    StrokeLine {
        line: Line,
        color: Rgba,
        width: Px<Surface>,
    },
    /// [`DrawSurface::draw_glyphs`], with a solid brush.
    Glyphs {
        face: Pos<Faces>,
        size: Px<Surface>,
        glyphs: Vec<PrintGlyph>,
        /// The text the glyphs draw, which each glyph's cluster cuts.
        text: String,
        color: Rgba,
        transform: Affine,
    },
    /// [`DrawSurface::push_clip`].
    PushClip(Rect),
    /// [`DrawSurface::pop_clip`].
    PopClip,
    /// [`DrawSurface::fill_path`], with a solid brush.
    FillPath { path: BezPath, color: Rgba },
    /// [`DrawSurface::stroke_path`], with a solid brush.
    StrokePath {
        path: BezPath,
        color: Rgba,
        stroke: Stroke,
    },
    /// [`DrawSurface::draw_image`].
    Image { image: Pos<Pictures>, dest: Rect },
    /// [`DrawSurface::push_transform`].
    PushTransform(Affine),
    /// [`DrawSurface::pop_transform`].
    PopTransform,
}

/// A page size, in PDF points.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PrintPage {
    pub width: PrintLength<Pt>,
    pub height: PrintLength<Pt>,
}

/// A whole document to print: one op list per page, all pages of one size,
/// and the faces and images the ops name by index.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct PrintDocument {
    /// The size of every page. `None` only for a document with no page.
    pub page: Option<PrintPage>,
    /// The op list of each page, in order.
    pub pages: Vec<Vec<DrawOp>>,
    /// The faces the glyph ops name.
    pub faces: Vec<PrintFace>,
    /// The images the image ops name.
    pub images: Vec<PrintImage>,
}

impl PrintDocument {
    /// An empty document whose pages are `page` large.
    pub fn new(page: PrintPage) -> Self {
        Self {
            page: Some(page),
            ..Self::default()
        }
    }

    /// The face an op names, or `None` for an index past the table.
    pub fn face(&self, face: Pos<Faces>) -> Option<&PrintFace> {
        self.faces.get(face.to_usize())
    }

    /// The image an op names, or `None` for an index past the table.
    pub fn image(&self, image: Pos<Pictures>) -> Option<&PrintImage> {
        self.images.get(image.to_usize())
    }
}

/// A [`DrawSurface`] that records one page of a [`PrintDocument`].
///
/// Each face and each image enters the document's table once. The recorder
/// keys a face by the identity of its shared blob, and an image by its
/// [`ImageKey`].
pub struct ListRecorder<'a> {
    document: &'a mut PrintDocument,
    ops: Vec<DrawOp>,
    faces: &'a mut HashMap<FaceKey, Pos<Faces>>,
    images: &'a mut HashMap<ImageKey, Pos<Pictures>>,
}

/// The identity of a face the shaper used: its blob and the face inside it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct FaceKey {
    blob: u64,
    index: Pos<FaceInFile>,
}

/// The tables a run of [`ListRecorder`]s shares across the pages of one
/// document, so a face used on every page enters once.
#[derive(Default)]
pub struct RecorderTables {
    faces: HashMap<FaceKey, Pos<Faces>>,
    images: HashMap<ImageKey, Pos<Pictures>>,
}

impl<'a> ListRecorder<'a> {
    /// A recorder of one page of `document`, sharing `tables` with the other
    /// pages of the same document.
    pub fn new(document: &'a mut PrintDocument, tables: &'a mut RecorderTables) -> Self {
        Self {
            document,
            ops: Vec::new(),
            faces: &mut tables.faces,
            images: &mut tables.images,
        }
    }

    /// Close the page: append its ops to the document.
    pub fn finish(self) {
        self.document.pages.push(self.ops);
    }

    fn face_of(&mut self, run: &GlyphRun<'_, ParleyBrush>) -> Pos<Faces> {
        let data = run.run().font();
        let key = FaceKey {
            blob: data.data.id(),
            index: Pos::new(data.index),
        };
        if let Some(face) = self.faces.get(&key) {
            return *face;
        }
        let face = Pos::<Faces>::from_usize(self.document.faces.len());
        self.document.faces.push(PrintFace {
            bytes: Payload::new(data.data.data().to_vec()),
            index: key.index,
        });
        self.faces.insert(key, face);
        face
    }
}

/// The glyphs of `run` with the cluster each one draws, and the text those
/// clusters cut. `source` is the text the run's layout was built from.
///
/// This is the one glyph-to-character mapping of the workspace (`PDF-005`):
/// the recorder and the live PDF surface both call it. A `source` that is
/// not the layout's text yields an empty text and empty clusters, so the
/// glyphs still draw.
pub fn print_glyphs(
    run: &GlyphRun<'_, ParleyBrush>,
    source: crate::LayoutText<'_>,
) -> (Vec<PrintGlyph>, String) {
    let mapped = run_text(run, source).unwrap_or_default();
    let mut clusters = mapped.clusters.into_iter();
    let glyphs: Vec<PrintGlyph> = run
        .glyphs()
        .map(|g| PrintGlyph {
            id: Pos::<Glyphs>::new(g.id),
            advance: Px::new(f64::from(g.advance)),
            x: Px::new(f64::from(g.x)),
            y: Px::new(f64::from(g.y)),
            cluster: clusters.next().unwrap_or_default(),
        })
        .collect();
    (glyphs, mapped.text)
}

/// The packed colour of a peniko colour, each channel rounded to a byte. The
/// PDF surface writes bytes too, so the round trip loses nothing it keeps.
pub fn rgba_of(color: Color) -> Rgba {
    let [r, g, b, a] = color.to_rgba8().to_u8_array();
    Rgba::from_channels(r, g, b, a)
}

/// The peniko colour of a packed colour.
pub fn color_of(rgba: Rgba) -> Color {
    let [r, g, b, a] = rgba.channels();
    Color::from_rgba8(r, g, b, a)
}

/// The colour of a solid brush, `None` for a gradient or an image brush.
fn solid(brush: &Brush) -> Option<Rgba> {
    match brush {
        Brush::Solid(c) => Some(rgba_of(*c)),
        _ => None,
    }
}

impl DrawSurface for ListRecorder<'_> {
    fn fill_rect(&mut self, rect: Rect, color: Color) {
        self.ops.push(DrawOp::FillRect {
            rect,
            color: rgba_of(color),
        });
    }

    fn stroke_rect(&mut self, rect: Rect, color: Color, width: Px<Surface>) {
        self.ops.push(DrawOp::StrokeRect {
            rect,
            color: rgba_of(color),
            width,
        });
    }

    fn stroke_line(&mut self, line: Line, color: Color, width: Px<Surface>) {
        self.ops.push(DrawOp::StrokeLine {
            line,
            color: rgba_of(color),
            width,
        });
    }

    fn draw_glyphs(
        &mut self,
        run: &GlyphRun<'_, ParleyBrush>,
        text: crate::LayoutText<'_>,
        brush: &Brush,
        transform: Affine,
    ) {
        let Some(color) = solid(brush) else {
            return;
        };
        let (glyphs, text) = print_glyphs(run, text);
        if glyphs.is_empty() {
            return;
        }
        let face = self.face_of(run);
        self.ops.push(DrawOp::Glyphs {
            face,
            size: Px::new(f64::from(run.run().font_size())),
            glyphs,
            text,
            color,
            transform,
        });
    }

    fn push_clip(&mut self, rect: Rect) {
        self.ops.push(DrawOp::PushClip(rect));
    }

    fn pop_clip(&mut self) {
        self.ops.push(DrawOp::PopClip);
    }

    fn fill_path(&mut self, path: &BezPath, brush: &Brush) {
        if let Some(color) = solid(brush) {
            self.ops.push(DrawOp::FillPath {
                path: path.clone(),
                color,
            });
        }
    }

    fn stroke_path(&mut self, path: &BezPath, brush: &Brush, stroke: &Stroke) {
        if let Some(color) = solid(brush) {
            self.ops.push(DrawOp::StrokePath {
                path: path.clone(),
                color,
                stroke: stroke.clone(),
            });
        }
    }

    fn draw_image(&mut self, image: &ImageSource<'_>, dest: Rect) {
        let index = match self.images.get(&image.key) {
            Some(index) => *index,
            None => {
                let index = Pos::<Pictures>::from_usize(self.document.images.len());
                self.document.images.push(PrintImage {
                    pixels: image.pixels.clone(),
                    size: image.size,
                });
                self.images.insert(image.key, index);
                index
            }
        };
        self.ops.push(DrawOp::Image { image: index, dest });
    }

    fn push_transform(&mut self, transform: Affine) {
        self.ops.push(DrawOp::PushTransform(transform));
    }

    fn pop_transform(&mut self) {
        self.ops.push(DrawOp::PopTransform);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page() -> PrintPage {
        PrintPage {
            width: PrintLength::new(100.0),
            height: PrintLength::new(50.0),
        }
    }

    #[test]
    fn a_page_records_each_call_in_order() {
        let mut doc = PrintDocument::new(page());
        let mut tables = RecorderTables::default();
        let mut rec = ListRecorder::new(&mut doc, &mut tables);
        rec.push_clip(Rect::new(0.0, 0.0, 10.0, 10.0));
        rec.fill_rect(
            Rect::new(1.0, 1.0, 2.0, 2.0),
            Color::from_rgba8(1, 2, 3, 255),
        );
        rec.pop_clip();
        rec.finish();
        assert_eq!(
            doc.pages,
            vec![vec![
                DrawOp::PushClip(Rect::new(0.0, 0.0, 10.0, 10.0)),
                DrawOp::FillRect {
                    rect: Rect::new(1.0, 1.0, 2.0, 2.0),
                    color: Rgba::from_channels(1, 2, 3, 255),
                },
                DrawOp::PopClip,
            ]]
        );
    }

    #[test]
    fn one_image_enters_the_table_once_across_pages() {
        let pixels = Payload::<RgbaPixels>::new(vec![255; 4]);
        let source = ImageSource {
            key: ImageKey::new(7),
            pixels: &pixels,
            size: ambre_core::Size::new(Px::new(1), Px::new(1)),
        };
        let mut doc = PrintDocument::new(page());
        let mut tables = RecorderTables::default();
        for _ in 0..2 {
            let mut rec = ListRecorder::new(&mut doc, &mut tables);
            rec.draw_image(&source, Rect::new(0.0, 0.0, 1.0, 1.0));
            rec.finish();
        }
        assert_eq!(doc.images.len(), 1);
        assert_eq!(doc.pages.len(), 2);
    }

    #[test]
    fn a_gradient_records_nothing() {
        let mut doc = PrintDocument::new(page());
        let mut tables = RecorderTables::default();
        let mut rec = ListRecorder::new(&mut doc, &mut tables);
        let gradient = Brush::Gradient(peniko::Gradient::default());
        rec.fill_path(&BezPath::new(), &gradient);
        rec.finish();
        assert_eq!(doc.pages, vec![Vec::new()]);
    }

    #[test]
    fn a_colour_survives_the_packing() {
        let color = Color::from_rgba8(10, 20, 30, 40);
        assert_eq!(color_of(rgba_of(color)).to_rgba8(), color.to_rgba8());
    }
}
