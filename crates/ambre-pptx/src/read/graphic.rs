//! The graphic frames the deck holds in another form. A table lands as one
//! shape per cell, with its text, its fill and its border. A chart lands as
//! a text of the values its part caches, as a chart part carries no picture.
//! Both stay text a user edits, and the report names each as approximated.

use ambre_core::{PrintLength, Pt, Radians, Rgba, Slant, Strikethrough, Underline, Weight};

use super::xml::{Element, Ns};
use super::{
    Context, DEFAULT_INSET, Flip, Frame, Reader, SlideOut, Transform, length, name_of, paragraphs,
    target,
};
use crate::PptxError;
use crate::model::{Geometry, Item, Line, Shape, TextAlign, TextAnchor, TextBody};

/// The outline a cell takes when it names no border of its own. The reader
/// does not resolve a table style, so a thin grid keeps the cells apart.
const STYLE_GRID: Line = Line {
    colour: Rgba::from_bits(0x8080_80FF),
    width: PrintLength::<Pt>::new(0.75),
};

/// The size of a chart text whose part names none: the default of a chart.
const CHART_TEXT_SIZE: PrintLength<Pt> = PrintLength::<Pt>::new(10.0);

/// Whether a graphic frame landed in the deck in another form.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Landed {
    /// Nothing of the frame reached the deck.
    Nothing,
    /// The frame reached the deck as shapes and text.
    Held,
}

/// The ink of a chart text and of a cell text the chain leaves unset.
const BLACK: Rgba = Rgba::from_bits(0x0000_00FF);

impl Reader<'_> {
    /// A table `tbl` of the graphic frame `frame`: one shape per cell that
    /// no merge covers, at its place in the grid. A merged cell spans the
    /// columns and the rows it names. [`Landed::Nothing`] when no cell landed.
    pub(super) fn table(
        &mut self,
        frame: &Element,
        tbl: &Element,
        context: &Context<'_>,
        transform: Transform,
        out: &mut SlideOut,
    ) -> Result<Landed, PptxError> {
        let Some(origin) = frame.child(Ns::P, "xfrm").and_then(Frame::of) else {
            return Ok(Landed::Nothing);
        };
        let columns: Vec<PrintLength<Pt>> = tbl
            .child(Ns::A, "tblGrid")
            .map(|grid| {
                grid.elements()
                    .filter(|c| c.is(Ns::A, "gridCol"))
                    .map(|c| length(c, "w").unwrap_or(PrintLength::<Pt>::ZERO))
                    .collect()
            })
            .unwrap_or_default();
        let rows: Vec<&Element> = tbl.elements().filter(|r| r.is(Ns::A, "tr")).collect();
        let heights: Vec<PrintLength<Pt>> = rows
            .iter()
            .map(|r| length(r, "h").unwrap_or(PrintLength::<Pt>::ZERO))
            .collect();
        // The sum of the lengths `lengths` holds from `start`, `count` of
        // them; a span past the grid stops at its edge.
        let span = |lengths: &[PrintLength<Pt>], start: usize, count: usize| -> f32 {
            lengths
                .iter()
                .skip(start)
                .take(count)
                .map(|l| l.get())
                .sum()
        };
        let name = name_of(frame, "nvGraphicFramePr");
        let mut landed = Landed::Nothing;
        for (row, tr) in rows.iter().enumerate() {
            for (column, tc) in tr.elements().filter(|c| c.is(Ns::A, "tc")).enumerate() {
                if tc.attr("hMerge") == Some("1") || tc.attr("vMerge") == Some("1") {
                    continue;
                }
                let spanned = |attr: &str| {
                    tc.attr(attr)
                        .and_then(|v| v.parse::<usize>().ok())
                        .unwrap_or(1)
                        .max(1)
                };
                let cell = Frame {
                    x: PrintLength::<Pt>::new(origin.x.get() + span(&columns, 0, column)),
                    y: PrintLength::<Pt>::new(origin.y.get() + span(&heights, 0, row)),
                    width: PrintLength::<Pt>::new(span(&columns, column, spanned("gridSpan"))),
                    height: PrintLength::<Pt>::new(span(&heights, row, spanned("rowSpan"))),
                    rotation: Radians::ZERO,
                    flip: Flip::default(),
                };
                let props = tc.child(Ns::A, "tcPr");
                let fill = props.and_then(|p| self.fill(p, context.palette)).flatten();
                let line = border(props, context);
                let text = tc
                    .child(Ns::A, "txBody")
                    .and_then(|body| self.text(body, None, &[], None, context))
                    .map(|text| TextBody {
                        inset: props
                            .and_then(|p| length(p, "marL"))
                            .unwrap_or(DEFAULT_INSET),
                        anchor: match props.and_then(|p| p.attr("anchor")) {
                            Some("ctr") => TextAnchor::Middle,
                            _ => TextAnchor::Top,
                        },
                        ..text
                    });
                let _ = out.push(Item::Shape(Shape {
                    name: name.clone(),
                    at: transform.frame(cell).placement(),
                    geometry: Geometry::Rect,
                    fill,
                    line,
                    text,
                }));
                self.count()?;
                landed = Landed::Held;
            }
        }
        Ok(landed)
    }

    /// The chart the graphic data `data` of the frame `frame` cites: a text
    /// box of its title, then one line per series of the values its part
    /// caches. [`Landed::Nothing`] for a chart part that is missing or caches nothing.
    pub(super) fn chart(
        &mut self,
        frame: &Element,
        data: &Element,
        context: &Context<'_>,
        transform: Transform,
        out: &mut SlideOut,
    ) -> Result<Landed, PptxError> {
        let Some(origin) = frame.child(Ns::P, "xfrm").and_then(Frame::of) else {
            return Ok(Landed::Nothing);
        };
        let Some(part) = data
            .child(Ns::C, "chart")
            .and_then(|c| c.attr_in(Ns::R, "id"))
            .and_then(|id| target(&context.part.rels, id))
            .filter(|part| self.package.contains(part))
        else {
            return Ok(Landed::Nothing);
        };
        let root = self.tree(&part)?;
        let Some(chart) = root.child(Ns::C, "chart") else {
            return Ok(Landed::Nothing);
        };
        let mut lines: Vec<String> = Vec::new();
        if let Some(rich) = chart.path(Ns::C, &["title", "tx", "rich"]) {
            let title = paragraphs(rich).join(" ");
            if !title.trim().is_empty() {
                lines.push(title);
            }
        }
        // The cached points under `element`, each with the index it names.
        let points = |element: &Element| -> Vec<(String, String)> {
            element
                .descendants(Ns::C, "pt")
                .into_iter()
                .map(|pt| {
                    let index = pt.attr("idx").unwrap_or_default().to_owned();
                    let value = pt.child(Ns::C, "v").map(Element::text).unwrap_or_default();
                    (index, value)
                })
                .collect()
        };
        let series = chart
            .child(Ns::C, "plotArea")
            .map(|plot| plot.descendants(Ns::C, "ser"))
            .unwrap_or_default();
        for ser in series {
            let label: String = ser
                .child(Ns::C, "tx")
                .map(|tx| {
                    tx.descendants(Ns::C, "v")
                        .into_iter()
                        .map(Element::text)
                        .collect::<Vec<_>>()
                        .join(" ")
                })
                .unwrap_or_default();
            let categories = ser
                .child(Ns::C, "cat")
                .or_else(|| ser.child(Ns::C, "xVal"))
                .map(points)
                .unwrap_or_default();
            let values: Vec<String> = ser
                .child(Ns::C, "val")
                .or_else(|| ser.child(Ns::C, "yVal"))
                .map(points)
                .unwrap_or_default()
                .into_iter()
                .map(
                    |(index, value)| match categories.iter().find(|(held, _)| *held == index) {
                        Some((_, category)) => format!("{category} {value}"),
                        None => value,
                    },
                )
                .collect();
            if values.is_empty() {
                continue;
            }
            let values = values.join(", ");
            lines.push(if label.trim().is_empty() {
                values
            } else {
                format!("{label}: {values}")
            });
        }
        if lines.is_empty() {
            return Ok(Landed::Nothing);
        }
        // A chart names its text size in hundredths of a point.
        let size = root
            .path(Ns::C, &["txPr"])
            .and_then(|body| body.descendants(Ns::A, "defRPr").into_iter().next())
            .and_then(|def| def.attr("sz"))
            .and_then(|sz| sz.parse::<f32>().ok())
            .filter(|sz| sz.is_finite() && *sz > 0.0)
            .map_or(CHART_TEXT_SIZE, |sz| PrintLength::<Pt>::new(sz / 100.0));
        // The face the chart's text names, else the theme's body face, as a
        // table cell's text takes it: an export writes the face it carries,
        // so a face left empty here would not survive a round trip.
        let typeface = root
            .path(Ns::C, &["txPr"])
            .and_then(|body| body.descendants(Ns::A, "latin").into_iter().next())
            .and_then(|latin| latin.attr("typeface"))
            .map_or_else(
                || context.palette.theme.face("+mn-lt"),
                |face| context.palette.theme.face(face),
            );
        let _ = out.push(Item::Shape(Shape {
            name: name_of(frame, "nvGraphicFramePr"),
            at: transform.frame(origin).placement(),
            geometry: Geometry::Rect,
            fill: None,
            line: Some(STYLE_GRID),
            text: Some(TextBody {
                text: lines.join("\n"),
                typeface,
                size,
                colour: BLACK,
                weight: Weight::Regular,
                slant: Slant::Upright,
                underline: Underline::Plain,
                strikethrough: Strikethrough::Plain,
                anchor: TextAnchor::Top,
                align: TextAlign::Left,
                inset: DEFAULT_INSET,
            }),
        }));
        self.count()?;
        Ok(Landed::Held)
    }
}

/// The outline of a cell: its first border that draws, or the style grid
/// when the cell names no border. `None` when each border it names draws
/// nothing.
fn border(props: Option<&Element>, context: &Context<'_>) -> Option<Line> {
    let borders: Vec<&Element> = ["lnL", "lnT", "lnR", "lnB"]
        .iter()
        .filter_map(|edge| props?.child(Ns::A, edge))
        .collect();
    if borders.is_empty() {
        return Some(STYLE_GRID);
    }
    borders.iter().find_map(|ln| {
        if ln.child(Ns::A, "noFill").is_some() {
            return None;
        }
        let colour = ln
            .child(Ns::A, "solidFill")
            .and_then(|fill| context.palette.first(fill))?;
        Some(Line {
            colour,
            width: length(ln, "w").unwrap_or(STYLE_GRID.width),
        })
    })
}
