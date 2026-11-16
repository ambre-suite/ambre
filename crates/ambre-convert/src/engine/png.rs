//! The PNG writer: the one page of a recorded [`PrintDocument`] rasterized
//! and encoded, on the converter thread (`toile-import-export`).
//!
//! The Logic thread records the page through the screen's own walk, as it
//! records a page of the PDF export. This module replays the ops onto a
//! pixel buffer instead of a krilla page. The replay follows the PDF
//! replay op for op:
//!
//! * either pop closes the last push;
//! * an op naming a face or a picture past its table draws nothing;
//! * the end of the replay closes a page left unbalanced.
//!
//! The module rasterizes each path itself: it flattens the path into lines
//! and accumulates the signed area each line covers, as a font rasterizer
//! does. It blends that coverage into premultiplied RGBA samples. A stroke
//! is widened into its outline by the stroker krilla links. A full 2D
//! renderer, or a path rasterizer crate, costs the module 100 KB to 200 KB
//! more for the same five ops (`ambre-spec-budgets.md section 5`).
//!
//! The page fills the raster: the writer scales the page's points to the
//! pixel size asked. So the caller records at the board's own scale, and
//! the outlines of a shape or a glyph stay sharp at any size.
//!
//! A gradient never reaches the list (`ambre_draw::list`). An encoded
//! picture draws nothing: a page of Toile carries decoded pictures alone,
//! and this module links no JPEG decoder.

use ambre_client_core::FileBytes;
use ambre_core::{FileRefusal, Image, Payload, Point, Px, Rgba, Size, Surface};
use ambre_draw::list::{DrawOp, PrintDocument, PrintGlyph, PrintImage};
use kurbo::{Affine, BezPath, Line, PathEl, Rect, Shape};
use peniko::color::PremulRgba8;
use skrifa::instance::{LocationRef, Size as FontSize};
use skrifa::outline::DrawSettings;
use skrifa::raw::model::pen::PathElement;
use skrifa::{FontRef, GlyphId, MetadataProvider};
use tiny_skia_path::{LineCap, LineJoin, PathBuilder, PathSegment, StrokeDash};

use crate::RASTER_EDGE;

/// Rasterize the one page of `document` at `size`, and encode it as PNG.
///
/// # Errors
///
/// [`FileRefusal::PastBound`] for an empty edge or an edge past
/// [`RASTER_EDGE`]. [`FileRefusal::Empty`] for a document of other than one
/// page. [`FileRefusal::Unwritable`] when the encoder fails.
pub(super) fn write(
    document: &PrintDocument,
    size: Size<Px<Image>>,
) -> Result<Payload<FileBytes>, FileRefusal> {
    let zero = Px::<Image>::new(0);
    if size.width == zero
        || size.height == zero
        || size.width > RASTER_EDGE
        || size.height > RASTER_EDGE
    {
        return Err(FileRefusal::PastBound);
    }
    let (Some(page), [ops]) = (document.page, document.pages.as_slice()) else {
        return Err(FileRefusal::Empty);
    };
    let fit = Affine::scale_non_uniform(
        f64::from(size.width.get()) / f64::from(page.width.get()).max(f64::EPSILON),
        f64::from(size.height.get()) / f64::from(page.height.get()).max(f64::EPSILON),
    );
    let faces = document
        .faces
        .iter()
        .map(|face| FontRef::from_index(face.bytes.as_bytes(), face.index.get()).ok())
        .collect();
    let mut raster = Raster {
        canvas: Canvas::new(size),
        document,
        faces,
        layers: vec![Layer::Transform(fit)],
    };
    for op in ops {
        raster.draw(op);
    }
    raster.canvas.encode().map_err(|_| FileRefusal::Unwritable)
}

/// A rect of whole pixels inside the canvas.
#[derive(Clone, Copy)]
struct Area {
    origin: Point<Px<Image>>,
    size: Size<Px<Image>>,
}

impl Area {
    /// The pixels of `size` that `bounds` touches, or `None` when it touches
    /// none or a corner is no number.
    fn within(bounds: Rect, size: Size<Px<Image>>) -> Option<Self> {
        let (width, height) = (f64::from(size.width.get()), f64::from(size.height.get()));
        let x0 = bounds.x0.floor().max(0.0);
        let y0 = bounds.y0.floor().max(0.0);
        let x1 = bounds.x1.ceil().min(width);
        let y1 = bounds.y1.ceil().min(height);
        // A NaN fails each comparison, so it yields no area.
        if !(x1 > x0 && y1 > y0) {
            return None;
        }
        let px = |v: f64| Px::<Image>::new(u32::try_from(v as i64).unwrap_or(0));
        Some(Self {
            origin: Point::new(px(x0), px(y0)),
            size: Size::new(px(x1 - x0), px(y1 - y0)),
        })
    }

    /// The pixels this area and `other` share.
    fn and(self, other: Self) -> Option<Self> {
        let x0 = self.origin.x.get().max(other.origin.x.get());
        let y0 = self.origin.y.get().max(other.origin.y.get());
        let x1 = (self.origin.x.get() + self.size.width.get())
            .min(other.origin.x.get() + other.size.width.get());
        let y1 = (self.origin.y.get() + self.size.height.get())
            .min(other.origin.y.get() + other.size.height.get());
        (x1 > x0 && y1 > y0).then(|| Self {
            origin: Point::new(Px::new(x0), Px::new(y0)),
            size: Size::new(Px::new(x1 - x0), Px::new(y1 - y0)),
        })
    }
}

/// The coverage of a shape over an area: one byte a pixel, row by row.
struct Coverage {
    area: Area,
    alpha: Vec<u8>,
}

impl Coverage {
    /// The coverage of `device`, a path in canvas pixels, over `area`
    /// alone. A pixel's coverage is the area the path fills in it, under
    /// the nonzero rule as far as the sum of signed areas tells it.
    fn of(device: &BezPath, area: Area) -> Self {
        let width = area.size.width.get() as usize;
        let height = area.size.height.get() as usize;
        // Two cells past each row take the area a line deposits at the right
        // edge, so no row spills into the next.
        let stride = width + 2;
        let mut cells = vec![0.0f32; stride * height];
        let (left, top) = (
            f64::from(area.origin.x.get()),
            f64::from(area.origin.y.get()),
        );
        let (right, bottom) = (width as f64, height as f64);
        let mut line = |a: kurbo::Point, b: kurbo::Point| {
            let (a, b) = ((a.x - left, a.y - top), (b.x - left, b.y - top));
            // A horizontal line covers no area, and a NaN fails the test.
            if !(a.1 != b.1) {
                return;
            }
            let (dir, p0, p1) = if a.1 < b.1 { (1.0, a, b) } else { (-1.0, b, a) };
            let dxdy = (p1.0 - p0.0) / (p1.1 - p0.1);
            let (start, end) = (p0.1.max(0.0), p1.1.min(bottom));
            if start >= end {
                return;
            }
            let mut x = p0.0 + (start - p0.1) * dxdy;
            let mut row = start.floor() as usize;
            while (row as f64) < end && row < height {
                let dy = ((row + 1) as f64).min(end) - (row as f64).max(start);
                let next = x + dxdy * dy;
                let d = (dy * dir) as f32;
                let (x0, x1) = {
                    let (u, v) = (x.clamp(0.0, right), next.clamp(0.0, right));
                    if u < v { (u, v) } else { (v, u) }
                };
                let base = row * stride;
                let x0_floor = x0.floor();
                let (i0, i1) = (x0_floor as usize, x1.ceil() as usize);
                let mut deposit = |i: usize, value: f32| {
                    if let Some(cell) = cells.get_mut(base + i) {
                        *cell += value;
                    }
                };
                if i1 <= i0 + 1 {
                    let mid = (0.5 * (x + next)).clamp(0.0, right) - x0_floor;
                    deposit(i0, d - d * mid as f32);
                    deposit(i0 + 1, d * mid as f32);
                } else {
                    let s = (1.0 / (x1 - x0)) as f32;
                    let f0 = (x0 - x0_floor) as f32;
                    let a0 = 0.5 * s * (1.0 - f0) * (1.0 - f0);
                    let f1 = (x1 - x1.ceil() + 1.0) as f32;
                    let am = 0.5 * s * f1 * f1;
                    deposit(i0, d * a0);
                    if i1 == i0 + 2 {
                        deposit(i0 + 1, d * (1.0 - a0 - am));
                    } else {
                        let a1 = s * (1.5 - f0);
                        deposit(i0 + 1, d * (a1 - a0));
                        for i in i0 + 2..i1 - 1 {
                            deposit(i, d * s);
                        }
                        let a2 = a1 + (i1 - i0 - 3) as f32 * s;
                        deposit(i1 - 1, d * (1.0 - a2 - am));
                    }
                    deposit(i1, d * am);
                }
                x = next;
                row += 1;
            }
        };
        // A fill closes each open subpath.
        let (mut first, mut last) = (None, None);
        kurbo::flatten(
            device.elements().iter().copied(),
            0.1,
            |element| match element {
                PathEl::MoveTo(p) => {
                    if let (Some(f), Some(l)) = (first, last) {
                        line(l, f);
                    }
                    (first, last) = (Some(p), Some(p));
                }
                PathEl::LineTo(p) => {
                    if let Some(l) = last {
                        line(l, p);
                    }
                    last = Some(p);
                }
                PathEl::ClosePath => {
                    if let (Some(f), Some(l)) = (first, last) {
                        line(l, f);
                    }
                    last = first;
                }
                PathEl::QuadTo(..) | PathEl::CurveTo(..) => {}
            },
        );
        if let (Some(f), Some(l)) = (first, last) {
            line(l, f);
        }
        let mut alpha = Vec::with_capacity(width * height);
        for row in cells.chunks_exact(stride) {
            let mut sum = 0.0f32;
            alpha.extend(row.iter().take(width).map(|cell| {
                sum += cell;
                (sum.abs().min(1.0) * 255.0).round() as u8
            }));
        }
        Self { area, alpha }
    }

    /// This coverage cut by `clip`: each pixel scaled by the clip's, over
    /// the pixels both hold.
    fn within(mut self, clip: Option<&Self>) -> Option<Self> {
        let Some(clip) = clip else {
            return Some(self);
        };
        let shared = self.area.and(clip.area)?;
        let mut alpha = Vec::with_capacity(
            shared.size.width.get() as usize * shared.size.height.get() as usize,
        );
        for y in 0..shared.size.height.get() {
            for x in 0..shared.size.width.get() {
                let at = |c: &Self| {
                    let cx = shared.origin.x.get() + x - c.area.origin.x.get();
                    let cy = shared.origin.y.get() + y - c.area.origin.y.get();
                    let i = cy as usize * c.area.size.width.get() as usize + cx as usize;
                    c.alpha.get(i).copied().map_or(0, u16::from)
                };
                alpha.push(u8::try_from(at(&self) * at(clip) / 255).unwrap_or(u8::MAX));
            }
        }
        self.area = shared;
        self.alpha = alpha;
        Some(self)
    }
}

/// The page's pixels, premultiplied RGBA, row by row.
struct Canvas {
    size: Size<Px<Image>>,
    rgba: Vec<PremulRgba8>,
}

impl Canvas {
    fn new(size: Size<Px<Image>>) -> Self {
        let len = size.width.get() as usize * size.height.get() as usize;
        let clear = PremulRgba8 {
            r: 0,
            g: 0,
            b: 0,
            a: 0,
        };
        Self {
            size,
            rgba: vec![clear; len],
        }
    }

    /// Blend `color` over the canvas, through `coverage`.
    fn paint(&mut self, coverage: &Coverage, color: Rgba) {
        let [r, g, b, a] = color.channels();
        let scale = |c: u8| u8::try_from(u32::from(c) * u32::from(a) / 255).unwrap_or(u8::MAX);
        let source = PremulRgba8 {
            r: scale(r),
            g: scale(g),
            b: scale(b),
            a,
        };
        self.blend(coverage, |_| source);
    }

    /// Blend the premultiplied sample `source` gives at the centre of each
    /// pixel over the canvas, through `coverage`.
    fn blend(&mut self, coverage: &Coverage, source: impl Fn(kurbo::Point) -> PremulRgba8) {
        let area = coverage.area;
        let stride = self.size.width.get() as usize;
        for y in 0..area.size.height.get() {
            for x in 0..area.size.width.get() {
                let i = y as usize * area.size.width.get() as usize + x as usize;
                let cover = coverage.alpha.get(i).copied().map_or(0, u32::from);
                if cover == 0 {
                    continue;
                }
                let (px, py) = (area.origin.x.get() + x, area.origin.y.get() + y);
                let sample = source(kurbo::Point::new(f64::from(px) + 0.5, f64::from(py) + 0.5));
                let src =
                    [sample.r, sample.g, sample.b, sample.a].map(|c| u32::from(c) * cover / 255);
                if let Some(dst) = self.rgba.get_mut(py as usize * stride + px as usize) {
                    let keep = 255 - src[3].min(255);
                    let mix = |s: u32, d: u8| {
                        u8::try_from(s + u32::from(d) * keep / 255).unwrap_or(u8::MAX)
                    };
                    *dst = PremulRgba8 {
                        r: mix(src[0], dst.r),
                        g: mix(src[1], dst.g),
                        b: mix(src[2], dst.b),
                        a: mix(src[3], dst.a),
                    };
                }
            }
        }
    }

    /// The PNG file of the canvas, its samples straight again. Each row is
    /// filtered against the one above it. The zlib stream is the one krilla
    /// writes a PDF stream with, so the module carries one deflate.
    fn encode(self) -> std::io::Result<Payload<FileBytes>> {
        use std::io::Write;
        let row = self.size.width.get() as usize * 4;
        let straight: Vec<u8> = self
            .rgba
            .iter()
            .flat_map(|&PremulRgba8 { r, g, b, a }| {
                let un = |c: u8| match a {
                    0 => 0,
                    _ => u8::try_from(u32::from(c) * 255 / u32::from(a)).unwrap_or(u8::MAX),
                };
                [un(r), un(g), un(b), a]
            })
            .collect();
        // Filter type 2, "Up": each byte less the byte above it. A board is
        // mostly flat colour, which this turns into runs of zeros.
        let mut filtered = Vec::with_capacity(straight.len() + straight.len() / row.max(1));
        let mut above: &[u8] = &[];
        for line in straight.chunks_exact(row.max(1)) {
            filtered.push(2);
            filtered.extend(
                line.iter()
                    .enumerate()
                    .map(|(i, b)| b.wrapping_sub(above.get(i).copied().unwrap_or(0))),
            );
            above = line;
        }
        let mut zlib = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::new(6));
        zlib.write_all(&filtered)?;
        let image = zlib.finish()?;
        let mut header = Vec::with_capacity(13);
        header.extend(self.size.width.get().to_be_bytes());
        header.extend(self.size.height.get().to_be_bytes());
        // Eight bits a sample, RGBA, deflate, adaptive filters, no interlace.
        header.extend([8, 6, 0, 0, 0]);
        let mut file = b"\x89PNG\r\n\x1a\n".to_vec();
        for (kind, data) in [
            (b"IHDR", &header),
            (b"IDAT", &image),
            (b"IEND", &Vec::new()),
        ] {
            let len = u32::try_from(data.len()).map_err(std::io::Error::other)?;
            file.extend(len.to_be_bytes());
            let mut crc = flate2::Crc::new();
            crc.update(kind);
            crc.update(data);
            file.extend(kind);
            file.extend(data.iter());
            file.extend(crc.sum().to_be_bytes());
        }
        Ok(Payload::new(file))
    }
}

/// One entry of the replay's push stack. A clip keeps the coverage it
/// leaves in force: its parent's cut by its rect.
enum Layer {
    Transform(Affine),
    Clip(Affine, Option<Coverage>),
}

impl Layer {
    fn transform(&self) -> Affine {
        match self {
            Self::Transform(transform) | Self::Clip(transform, _) => *transform,
        }
    }
}

/// What a clip in force leaves drawable: everything, or a coverage, which
/// may be empty.
enum Drawable<'a> {
    All,
    Within(&'a Coverage),
    Nothing,
}

/// The replay of one page onto its canvas.
struct Raster<'a> {
    canvas: Canvas,
    document: &'a PrintDocument,
    /// Each face of the document, read once. A face skrifa cannot read
    /// draws nothing.
    faces: Vec<Option<FontRef<'a>>>,
    /// The page's fit at the bottom, then each push the ops made.
    layers: Vec<Layer>,
}

impl Raster<'_> {
    /// The transform in force: the page's fit and every push above it.
    fn transform(&self) -> Affine {
        self.layers
            .last()
            .map_or(Affine::IDENTITY, Layer::transform)
    }

    /// What the clips in force leave drawable.
    fn drawable(&self) -> Drawable<'_> {
        self.layers
            .iter()
            .rev()
            .find_map(|layer| match layer {
                Layer::Clip(_, Some(coverage)) => Some(Drawable::Within(coverage)),
                Layer::Clip(_, None) => Some(Drawable::Nothing),
                Layer::Transform(_) => None,
            })
            .unwrap_or(Drawable::All)
    }

    fn draw(&mut self, op: &DrawOp) {
        match op {
            DrawOp::FillRect { rect, color } => {
                self.fill(&rect.to_path(0.1), *color, self.transform())
            }
            DrawOp::StrokeRect { rect, color, width } => {
                self.stroke(&rect.to_path(0.1), *color, &plain_stroke(*width), None);
            }
            DrawOp::StrokeLine { line, color, width } => {
                self.stroke(&line_path(*line), *color, &plain_stroke(*width), None);
            }
            DrawOp::FillPath { path, color } => self.fill(path, *color, self.transform()),
            DrawOp::StrokePath {
                path,
                color,
                stroke,
            } => {
                let dashes: Vec<f32> = stroke.dash_pattern.iter().map(|d| *d as f32).collect();
                let dash = StrokeDash::new(dashes, stroke.dash_offset as f32);
                self.stroke(path, *color, &stroke_of(stroke), dash.as_ref());
            }
            DrawOp::Glyphs {
                face,
                size,
                glyphs,
                color,
                transform,
                ..
            } => {
                let run = self
                    .faces
                    .get(face.to_usize())
                    .and_then(Option::as_ref)
                    .map(|font| run_path(font, *size, glyphs));
                if let Some(run) = run.filter(|run| !run.elements().is_empty()) {
                    self.fill(&run, *color, self.transform() * *transform);
                }
            }
            DrawOp::Image { image, dest } => {
                if let Some(image) = self.document.image(*image) {
                    self.picture(image, *dest);
                }
            }
            // A page of Toile carries decoded pictures alone (module docs).
            DrawOp::EncodedImage { .. } => {}
            DrawOp::PushTransform(transform) => {
                let transform = self.transform() * *transform;
                self.layers.push(Layer::Transform(transform));
            }
            DrawOp::PushClip(rect) => self.push_clip(*rect),
            // Either pop closes the last push, as on the PDF page. The fit of
            // the page is never popped.
            DrawOp::PopClip | DrawOp::PopTransform => {
                if self.layers.len() > 1 {
                    self.layers.pop();
                }
            }
        }
    }

    /// The coverage of `path` under `transform` and the clips in force.
    /// `None` when nothing is drawable.
    fn coverage(&self, path: &BezPath, transform: Affine) -> Option<Coverage> {
        let device = transform * path.clone();
        let area = Area::within(device.bounding_box().inflate(1.0, 1.0), self.canvas.size)?;
        match self.drawable() {
            Drawable::Nothing => None,
            Drawable::All => Some(Coverage::of(&device, area)),
            Drawable::Within(clip) => {
                Coverage::of(&device, area.and(clip.area)?).within(Some(clip))
            }
        }
    }

    fn fill(&mut self, path: &BezPath, color: Rgba, transform: Affine) {
        if let Some(coverage) = self.coverage(path, transform) {
            self.canvas.paint(&coverage, color);
        }
    }

    /// Widen `path` into its outline by `stroke`, dashed by `dash`, in the
    /// local space the transform in force maps, then fill the outline.
    fn stroke(
        &mut self,
        path: &BezPath,
        color: Rgba,
        stroke: &tiny_skia_path::Stroke,
        dash: Option<&StrokeDash>,
    ) {
        let transform = self.transform();
        // The stroker flattens its curves finer as the page scales them up.
        let scale = transform.determinant().abs().sqrt().max(f64::EPSILON) as f32;
        let outline = local_path(path)
            .and_then(|p| match dash {
                Some(dash) => p.dash(dash, scale),
                None => Some(p),
            })
            .and_then(|p| p.stroke(stroke, scale));
        if let Some(outline) = outline {
            self.fill(&bez_path(&outline), color, transform);
        }
    }

    /// Push the coverage in force cut by `rect`. A rect with no area clips
    /// everything out, as on the PDF page.
    fn push_clip(&mut self, rect: Rect) {
        let transform = self.transform();
        let cut = (rect.width() > 0.0 && rect.height() > 0.0)
            .then(|| self.coverage(&rect.to_path(0.1), transform))
            .flatten();
        self.layers.push(Layer::Clip(transform, cut));
    }

    /// Paint `image` over `dest`, sampled bilinearly. A picture whose bytes
    /// do not match its size draws nothing.
    fn picture(&mut self, image: &PrintImage, dest: Rect) {
        let (width, height) = (image.size.width.get(), image.size.height.get());
        let samples = image.pixels.as_bytes();
        if width == 0
            || height == 0
            || samples.len() != width as usize * height as usize * 4
            || dest.width() <= 0.0
            || dest.height() <= 0.0
        {
            return;
        }
        let place = self.transform()
            * Affine::translate((dest.x0, dest.y0))
            * Affine::scale_non_uniform(
                dest.width() / f64::from(width),
                dest.height() / f64::from(height),
            );
        let frame = Rect::new(0.0, 0.0, f64::from(width), f64::from(height));
        let Some(coverage) = self.coverage(&frame.to_path(0.1), place) else {
            return;
        };
        let to_image = place.inverse();
        // The premultiplied sample at a pixel of the picture, its edges
        // repeated past the frame.
        let texel = |x: i64, y: i64| {
            let x = x.clamp(0, i64::from(width) - 1) as usize;
            let y = y.clamp(0, i64::from(height) - 1) as usize;
            let at = (y * width as usize + x) * 4;
            let [r, g, b, a] = samples
                .get(at..at + 4)
                .and_then(|s| <[u8; 4]>::try_from(s).ok())
                .unwrap_or_default();
            let a32 = f64::from(a) / 255.0;
            [
                f64::from(r) * a32,
                f64::from(g) * a32,
                f64::from(b) * a32,
                f64::from(a),
            ]
        };
        self.canvas.blend(&coverage, |at| {
            let p = to_image * at;
            let (u, v) = (p.x - 0.5, p.y - 0.5);
            let (x0, y0) = (u.floor(), v.floor());
            let (fx, fy) = (u - x0, v - y0);
            let (x0, y0) = (x0 as i64, y0 as i64);
            let corners = [
                texel(x0, y0),
                texel(x0 + 1, y0),
                texel(x0, y0 + 1),
                texel(x0 + 1, y0 + 1),
            ];
            let weights = [
                (1.0 - fx) * (1.0 - fy),
                fx * (1.0 - fy),
                (1.0 - fx) * fy,
                fx * fy,
            ];
            let channel = |channel: usize| {
                let value: f64 = corners
                    .iter()
                    .zip(weights)
                    .map(|(corner, w)| corner.get(channel).copied().unwrap_or(0.0) * w)
                    .sum();
                value.round().clamp(0.0, 255.0) as u8
            };
            PremulRgba8 {
                r: channel(0),
                g: channel(1),
                b: channel(2),
                a: channel(3),
            }
        });
    }
}

/// The outline of a whole glyph run, in the run's own space. Each glyph sits
/// at its pen position, the baseline at zero and y down, as the GPU surface
/// places it. A glyph the face cannot draw adds nothing.
fn run_path(font: &FontRef<'_>, size: Px<Surface>, glyphs: &[PrintGlyph]) -> BezPath {
    let mut run = BezPath::new();
    if size.get() <= 0.0 {
        return run;
    }
    let outlines = font.outline_glyphs();
    let font_size = FontSize::new(size.get() as f32);
    let mut advance = 0.0;
    for glyph in glyphs {
        // The font draws y up, the page y down.
        let place = Affine::translate((advance + glyph.x.get(), glyph.y.get())) * Affine::FLIP_Y;
        advance += glyph.advance.get();
        let Some(outline) = outlines.get(GlyphId::new(glyph.id.get())) else {
            continue;
        };
        let mut elements: Vec<PathElement> = Vec::new();
        let settings = DrawSettings::unhinted(font_size, LocationRef::default());
        if outline.draw(settings, &mut elements).is_err() {
            continue;
        }
        let point = |x: f32, y: f32| place * kurbo::Point::new(f64::from(x), f64::from(y));
        for element in elements {
            match element {
                PathElement::MoveTo { x, y } => run.move_to(point(x, y)),
                PathElement::LineTo { x, y } => run.line_to(point(x, y)),
                PathElement::QuadTo { cx0, cy0, x, y } => run.quad_to(point(cx0, cy0), point(x, y)),
                PathElement::CurveTo {
                    cx0,
                    cy0,
                    cx1,
                    cy1,
                    x,
                    y,
                } => run.curve_to(point(cx0, cy0), point(cx1, cy1), point(x, y)),
                PathElement::Close => run.close_path(),
            }
        }
    }
    run
}

/// The stroke of a rect or a line: butt caps and miter joins, as on the PDF
/// page.
fn plain_stroke(width: Px<Surface>) -> tiny_skia_path::Stroke {
    tiny_skia_path::Stroke {
        width: width.get() as f32,
        ..tiny_skia_path::Stroke::default()
    }
}

/// The stroke a path op names: its width, caps and join. The dashes cut
/// the path before it widens.
fn stroke_of(stroke: &kurbo::Stroke) -> tiny_skia_path::Stroke {
    tiny_skia_path::Stroke {
        width: stroke.width as f32,
        miter_limit: stroke.miter_limit as f32,
        line_cap: match stroke.start_cap {
            kurbo::Cap::Butt => LineCap::Butt,
            kurbo::Cap::Square => LineCap::Square,
            kurbo::Cap::Round => LineCap::Round,
        },
        line_join: match stroke.join {
            kurbo::Join::Bevel => LineJoin::Bevel,
            kurbo::Join::Miter => LineJoin::Miter,
            kurbo::Join::Round => LineJoin::Round,
        },
        dash: None,
    }
}

fn line_path(line: Line) -> BezPath {
    let mut path = BezPath::new();
    path.move_to(line.p0);
    path.line_to(line.p1);
    path
}

/// The stroker's form of `path`, or `None` for a path with no segment.
fn local_path(path: &BezPath) -> Option<tiny_skia_path::Path> {
    let mut builder = PathBuilder::new();
    for element in path.elements() {
        match *element {
            PathEl::MoveTo(p) => builder.move_to(p.x as f32, p.y as f32),
            PathEl::LineTo(p) => builder.line_to(p.x as f32, p.y as f32),
            PathEl::QuadTo(c, p) => builder.quad_to(c.x as f32, c.y as f32, p.x as f32, p.y as f32),
            PathEl::CurveTo(c0, c1, p) => builder.cubic_to(
                c0.x as f32,
                c0.y as f32,
                c1.x as f32,
                c1.y as f32,
                p.x as f32,
                p.y as f32,
            ),
            PathEl::ClosePath => builder.close(),
        }
    }
    builder.finish()
}

/// The outline the stroker gave, as a path the rasterizer fills.
fn bez_path(path: &tiny_skia_path::Path) -> BezPath {
    let point = |p: tiny_skia_path::Point| kurbo::Point::new(f64::from(p.x), f64::from(p.y));
    let mut out = BezPath::new();
    for segment in path.segments() {
        match segment {
            PathSegment::MoveTo(p) => out.move_to(point(p)),
            PathSegment::LineTo(p) => out.line_to(point(p)),
            PathSegment::QuadTo(c, p) => out.quad_to(point(c), point(p)),
            PathSegment::CubicTo(c0, c1, p) => out.curve_to(point(c0), point(c1), point(p)),
            PathSegment::Close => out.close_path(),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use ambre_core::index::Pos;
    use ambre_core::{PrintLength, Pt};
    use ambre_draw::list::{PrintFace, PrintPage};

    fn page(ops: Vec<DrawOp>) -> PrintDocument {
        let mut document = PrintDocument::new(PrintPage {
            width: PrintLength::<Pt>::new(100.0),
            height: PrintLength::<Pt>::new(50.0),
        });
        document.pages.push(ops);
        document
    }

    fn size(width: u32, height: u32) -> Size<Px<Image>> {
        Size::new(Px::new(width), Px::new(height))
    }

    fn decoded(bytes: &Payload<FileBytes>) -> image::RgbaImage {
        image::load_from_memory_with_format(bytes.as_bytes(), image::ImageFormat::Png)
            .expect("the writer writes a PNG")
            .to_rgba8()
    }

    fn rgba_at(image: &image::RgbaImage, x: u32, y: u32) -> [u8; 4] {
        image.get_pixel(x, y).0
    }

    const RED: Rgba = Rgba::from_channels(255, 0, 0, 255);
    const WHITE: Rgba = Rgba::from_channels(255, 255, 255, 255);

    /// The file is a PNG of the size asked, and the page fills
    /// it. A rect over the left half of the page inks the left half of the
    /// raster at twice the scale. The right half keeps its paint.
    #[test]
    fn a_page_rasterizes_at_the_size_asked_and_fills_it() {
        let document = page(vec![
            DrawOp::FillRect {
                rect: Rect::new(0.0, 0.0, 100.0, 50.0),
                color: WHITE,
            },
            DrawOp::FillRect {
                rect: Rect::new(0.0, 0.0, 50.0, 50.0),
                color: RED,
            },
        ]);
        let file = write(&document, size(200, 100)).expect("the page writes");
        assert!(file.as_bytes().starts_with(b"\x89PNG\r\n\x1a\n"));
        let pixmap = decoded(&file);
        assert_eq!(pixmap.dimensions(), (200, 100));
        assert_eq!(rgba_at(&pixmap, 10, 50), [255, 0, 0, 255]);
        assert_eq!(rgba_at(&pixmap, 95, 50), [255, 0, 0, 255]);
        assert_eq!(rgba_at(&pixmap, 105, 50), [255, 255, 255, 255]);
    }

    /// A clip keeps what it holds, and what lies outside it
    /// is never painted; its pop gives the page back.
    #[test]
    fn a_clip_keeps_its_rect_alone_until_it_pops() {
        let document = page(vec![
            DrawOp::PushClip(Rect::new(0.0, 0.0, 20.0, 50.0)),
            DrawOp::FillRect {
                rect: Rect::new(0.0, 0.0, 100.0, 50.0),
                color: RED,
            },
            DrawOp::PopClip,
            DrawOp::FillRect {
                rect: Rect::new(80.0, 0.0, 100.0, 50.0),
                color: RED,
            },
        ]);
        let pixmap = decoded(&write(&document, size(100, 50)).expect("the page writes"));
        assert_eq!(rgba_at(&pixmap, 10, 25), [255, 0, 0, 255]);
        assert_eq!(rgba_at(&pixmap, 50, 25)[3], 0, "outside the clip");
        assert_eq!(rgba_at(&pixmap, 90, 25), [255, 0, 0, 255]);
    }

    /// A glyph run inks the raster where the GPU surface places
    /// it: at its pen position, above its baseline.
    #[test]
    fn a_glyph_run_inks_above_its_baseline() {
        const FACE: &str = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../ambre-app/fonts/NotoSans-Regular.ttf"
        );
        let bytes = std::fs::read(FACE).expect("the workspace ships its faces");
        let font = FontRef::new(&bytes).expect("the face reads");
        let id = font.charmap().map('H').expect("the face maps H");
        let mut document = page(vec![DrawOp::Glyphs {
            face: Pos::new(0),
            size: Px::new(40.0),
            glyphs: vec![PrintGlyph {
                id: Pos::new(id.to_u32()),
                advance: Px::new(30.0),
                x: Px::new(0.0),
                y: Px::new(0.0),
                cluster: Default::default(),
            }],
            text: "H".to_owned(),
            color: RED,
            transform: Affine::translate((10.0, 40.0)),
        }]);
        document.faces.push(PrintFace {
            bytes: Payload::new(bytes.clone()),
            index: Pos::new(0),
        });
        let pixmap = decoded(&write(&document, size(100, 50)).expect("the page writes"));
        let inked = |x0: u32, x1: u32, y0: u32, y1: u32| {
            (x0..x1).any(|x| (y0..y1).any(|y| rgba_at(&pixmap, x, y)[3] > 128))
        };
        // The stem of the H, left of its pen, between its cap height and its
        // baseline.
        assert!(inked(10, 20, 15, 40), "the glyph inks above its baseline");
        assert!(!inked(0, 100, 42, 50), "no ink below the baseline");
        assert!(!inked(60, 100, 0, 50), "no ink past the run");
    }

    /// A picture fills its destination rect with its pixels.
    #[test]
    fn a_picture_fills_its_rect_with_its_pixels() {
        let mut document = page(vec![DrawOp::Image {
            image: Pos::new(0),
            dest: Rect::new(50.0, 0.0, 100.0, 50.0),
        }]);
        document.images.push(PrintImage {
            pixels: Payload::new([0, 0, 255, 255].repeat(4)),
            size: size(2, 2),
        });
        let pixmap = decoded(&write(&document, size(100, 50)).expect("the page writes"));
        assert_eq!(rgba_at(&pixmap, 75, 25), [0, 0, 255, 255]);
        assert_eq!(rgba_at(&pixmap, 25, 25)[3], 0);
    }

    /// An edge past the bound, an empty edge and a document of
    /// other than one page are refused before any pixel is allocated.
    #[test]
    fn a_size_past_the_bound_or_a_document_not_of_one_page_is_refused() {
        let one = page(Vec::new());
        let past = Px::<Image>::new(RASTER_EDGE.get() + 1);
        assert_eq!(
            write(&one, Size::new(past, Px::new(1))),
            Err(FileRefusal::PastBound)
        );
        assert_eq!(write(&one, size(0, 10)), Err(FileRefusal::PastBound));
        let mut two = page(Vec::new());
        two.pages.push(Vec::new());
        assert_eq!(write(&two, size(10, 10)), Err(FileRefusal::Empty));
        let none = PrintDocument::default();
        assert_eq!(write(&none, size(10, 10)), Err(FileRefusal::Empty));
    }
}
