//! The cell styles of OpenDocument: a chain of named and automatic styles,
//! each overriding its parent, read into Trame's cell style and written back.
//!
//! A cell names a style, or takes the default style of its row or column.
//! The reader resolves the chain from the default style down, and turns the
//! properties into a [`CellStyle`]. A font, a size or a colour equal to the
//! document's default reads as none of its own, as the xlsx reader reads it.

use std::collections::HashMap;
use std::fmt::Write as _;

use ambre_core::{FontFamily, FontSize, Rgb, Slant, Strikethrough, Underline, Weight, WordBreak};

use super::number::{self, Approximation};
use super::{Node, length_px, push_escaped};
use crate::model::{
    BorderStyle, Borders, CellBorder, CellStyle, HAlign, NumberFormat, StyleId, VAlign,
};
use crate::report::{Outcome, Report, XlsxConstruct};

/// The name of the style a cell without one takes.
pub(crate) const DEFAULT: &str = "Default";

/// The font a written document draws a cell with when its style names none:
/// Calibri at 11 points, the font the xlsx writer names.
const WRITTEN_FONT: &str = "Calibri";
const WRITTEN_SIZE: FontSize = FontSize::from_whole_points(11);

/// The properties one style sets, each `None` where it inherits.
#[derive(Debug, Clone, Default)]
struct Props {
    weight: Option<Weight>,
    slant: Option<Slant>,
    underline: Option<Underline>,
    strikethrough: Option<Strikethrough>,
    h_align: Option<HAlign>,
    v_align: Option<VAlign>,
    wrap: Option<WordBreak>,
    fg: Option<Option<Rgb>>,
    bg: Option<Option<Rgb>>,
    font: Option<String>,
    size: Option<FontSize>,
    data_style: Option<String>,
    top: Option<Option<CellBorder>>,
    right: Option<Option<CellBorder>>,
    bottom: Option<Option<CellBorder>>,
    left: Option<Option<CellBorder>>,
    /// The constructs this style approximates or drops, recorded once per
    /// style a cell uses.
    losses: Vec<(XlsxConstruct, Outcome)>,
}

impl Props {
    /// `self` over `parent`: each property this style sets wins.
    fn over(&self, parent: &Self) -> Self {
        Self {
            weight: self.weight.or(parent.weight),
            slant: self.slant.or(parent.slant),
            underline: self.underline.or(parent.underline),
            strikethrough: self.strikethrough.or(parent.strikethrough),
            h_align: self.h_align.or(parent.h_align),
            v_align: self.v_align.or(parent.v_align),
            wrap: self.wrap.or(parent.wrap),
            fg: self.fg.or(parent.fg),
            bg: self.bg.or(parent.bg),
            font: self.font.clone().or_else(|| parent.font.clone()),
            size: self.size.or(parent.size),
            data_style: self
                .data_style
                .clone()
                .or_else(|| parent.data_style.clone()),
            top: self.top.or(parent.top),
            right: self.right.or(parent.right),
            bottom: self.bottom.or(parent.bottom),
            left: self.left.or(parent.left),
            losses: parent.losses.iter().chain(&self.losses).copied().collect(),
        }
    }
}

/// One style definition: its parent and its own properties.
struct Definition {
    parent: Option<String>,
    props: Props,
}

/// Every cell style and number style of a document, and the styles the
/// cells resolved to so far.
pub(crate) struct Styles {
    cells: HashMap<String, Definition>,
    numbers: HashMap<String, String>,
    fonts: HashMap<String, String>,
    base: Props,
    resolved: HashMap<String, StyleId>,
    /// The distinct styles, the default first.
    pub(crate) list: Vec<CellStyle>,
}

impl Styles {
    /// The styles `roots` declare: the styles part, then the automatic
    /// styles and the font declarations of the content part.
    pub(crate) fn new(roots: &[&Node]) -> Self {
        let mut styles = Self {
            cells: HashMap::new(),
            numbers: HashMap::new(),
            fonts: HashMap::new(),
            base: Props::default(),
            resolved: HashMap::new(),
            list: Vec::new(),
        };
        for root in roots {
            for node in root.walk() {
                styles.declare(node);
            }
        }
        styles
    }

    fn declare(&mut self, node: &Node) {
        match node.name() {
            "font-face" => {
                if let Some(name) = node.attr("name") {
                    let family = node
                        .attr("font-family")
                        .map_or(name, |f| f.trim_matches(['\'', '"']));
                    self.fonts.insert(name.to_owned(), family.to_owned());
                }
            }
            "default-style" if node.attr("family") == Some("table-cell") => {
                self.base = props_of(node);
            }
            "style" if node.attr("family") == Some("table-cell") => {
                if let Some(name) = node.attr("name") {
                    self.cells.insert(
                        name.to_owned(),
                        Definition {
                            parent: node.attr("parent-style-name").map(str::to_owned),
                            props: props_of(node),
                        },
                    );
                }
            }
            _ => {
                if let (Some(name), Some(code)) = (node.attr("name"), number::code_of(node)) {
                    self.numbers.insert(name.to_owned(), code);
                }
            }
        }
    }

    /// The style a cell of the style `name` takes, deduplicated into
    /// [`Styles::list`]. A name no style declares reads as the default.
    pub(crate) fn id(&mut self, name: &str, report: &mut Report) -> StyleId {
        if self.list.is_empty() {
            let default = self.resolve(DEFAULT);
            self.list.push(default.0);
            self.resolved.insert(DEFAULT.to_owned(), StyleId::DEFAULT);
            for (construct, outcome) in default.1 {
                report.record(construct, outcome);
            }
        }
        if let Some(&id) = self.resolved.get(name) {
            return id;
        }
        let (style, losses) = self.resolve(name);
        for (construct, outcome) in losses {
            report.record(construct, outcome);
        }
        let at = self
            .list
            .iter()
            .position(|s| *s == style)
            .unwrap_or_else(|| {
                self.list.push(style);
                self.list.len() - 1
            });
        let id = StyleId::at(ambre_core::index::Pos::from_usize(at));
        self.resolved.insert(name.to_owned(), id);
        id
    }

    /// The style `name` resolves to, and what it lost.
    fn resolve(&self, name: &str) -> (CellStyle, Vec<(XlsxConstruct, Outcome)>) {
        let mut chain = Vec::new();
        let mut at = Some(name.to_owned());
        while let Some(current) = at {
            if chain.len() > 16 || chain.contains(&current) {
                break;
            }
            at = self.cells.get(&current).and_then(|d| d.parent.clone());
            chain.push(current);
        }
        if !chain.iter().any(|n| n == DEFAULT) {
            chain.push(DEFAULT.to_owned());
        }
        let props = chain.iter().rev().fold(self.base.clone(), |acc, name| {
            self.cells
                .get(name)
                .map_or_else(|| acc.clone(), |d| d.props.over(&acc))
        });
        let family = |name: &str| self.fonts.get(name).map_or(name, String::as_str).to_owned();
        let default_font = self.base.font.as_deref().map(family);
        let mut losses = props.losses.clone();
        let format = match props.data_style.as_ref() {
            None => NumberFormat::General,
            Some(name) => match self.numbers.get(name) {
                Some(code) => crate::styles::number_format_of(code),
                None => NumberFormat::General,
            },
        };
        let style = CellStyle {
            weight: props.weight.unwrap_or(Weight::Regular),
            slant: props.slant.unwrap_or(Slant::Upright),
            underline: props.underline.unwrap_or(Underline::Plain),
            strikethrough: props.strikethrough.unwrap_or(Strikethrough::Plain),
            h_align: props.h_align.unwrap_or(HAlign::Auto),
            // A cell that names no vertical alignment sits at the bottom, as
            // an Excel cell does.
            v_align: props.v_align.unwrap_or(VAlign::Bottom),
            wrap: props.wrap.unwrap_or(WordBreak::Whole),
            fg: props
                .fg
                .flatten()
                .filter(|fg| Some(*fg) != self.base.fg.flatten()),
            bg: props.bg.flatten(),
            font: props
                .font
                .as_deref()
                .map(family)
                .filter(|f| Some(f) != default_font.as_ref())
                .map(|f| FontFamily::from_label(&f)),
            size: props.size.filter(|s| Some(*s) != self.base.size),
            format,
            borders: Borders {
                top: props.top.flatten(),
                right: props.right.flatten(),
                bottom: props.bottom.flatten(),
                left: props.left.flatten(),
            },
        };
        losses.dedup();
        (style, losses)
    }
}

/// The properties the style element `node` sets.
fn props_of(node: &Node) -> Props {
    let mut props = Props {
        data_style: node.attr("data-style-name").map(str::to_owned),
        ..Props::default()
    };
    for part in node.children() {
        match part.name() {
            "text-properties" => text_props(part, &mut props),
            "table-cell-properties" => cell_props(part, &mut props),
            "paragraph-properties" => paragraph_props(part, &mut props),
            _ => {}
        }
    }
    // A cell that aligns by its value's type aligns as Trame's automatic
    // alignment does, whatever paragraph alignment it names.
    let by_value = node.children().iter().any(|p| {
        p.name() == "table-cell-properties" && p.attr("text-align-source") == Some("value-type")
    });
    if by_value {
        props.h_align = Some(HAlign::Auto);
    }
    props
}

fn text_props(part: &Node, props: &mut Props) {
    if let Some(w) = part.attr("font-weight") {
        props.weight = Some(if w == "bold" || w.parse::<u16>().is_ok_and(|n| n >= 600) {
            Weight::Bold
        } else {
            Weight::Regular
        });
    }
    if let Some(s) = part.attr("font-style") {
        props.slant = Some(if s == "normal" {
            Slant::Upright
        } else {
            Slant::Italic
        });
    }
    if let Some(u) = part.attr("text-underline-style") {
        props.underline = Some(if u == "none" {
            Underline::Plain
        } else {
            let plain = u == "solid" && part.attr("text-underline-type") != Some("double");
            if !plain {
                props
                    .losses
                    .push((XlsxConstruct::FontEffect, Outcome::Approximated));
            }
            Underline::Underlined
        });
    }
    if let Some(s) = part.attr("text-line-through-style") {
        props.strikethrough = Some(if s == "none" {
            Strikethrough::Plain
        } else {
            Strikethrough::Struck
        });
    }
    if part.attr("use-window-font-color") == Some("true") {
        props.fg = Some(None);
    }
    if let Some(c) = part.attr("color") {
        props.fg = Some(parse_color(c));
    }
    if let Some(f) = part.attr("font-name").or_else(|| part.attr("font-family")) {
        props.font = Some(f.trim_matches(['\'', '"']).to_owned());
    }
    if let Some(points) = part
        .attr("font-size")
        .and_then(|s| s.strip_suffix("pt"))
        .and_then(|p| p.trim().parse::<f32>().ok())
    {
        props.size = FontSize::new(points);
    }
    let effect = part
        .attr("text-position")
        .is_some_and(|p| !p.starts_with("0%") && p != "0")
        || part.attr("text-shadow").is_some_and(|s| s != "none")
        || part.attr("text-outline") == Some("true");
    if effect {
        props
            .losses
            .push((XlsxConstruct::FontEffect, Outcome::Dropped));
    }
}

fn cell_props(part: &Node, props: &mut Props) {
    if let Some(c) = part.attr("background-color") {
        props.bg = Some(parse_color(c));
    }
    if let Some(v) = part.attr("vertical-align") {
        props.v_align = Some(match v {
            "top" => VAlign::Top,
            "middle" => VAlign::Middle,
            _ => VAlign::Bottom,
        });
    }
    if let Some(w) = part.attr("wrap-option") {
        props.wrap = Some(if w == "wrap" {
            WordBreak::Anywhere
        } else {
            WordBreak::Whole
        });
    }
    if let Some(b) = part.attr("border") {
        let edge = border_of(b, &mut props.losses);
        props.top = Some(edge);
        props.right = Some(edge);
        props.bottom = Some(edge);
        props.left = Some(edge);
    }
    for (attr, slot) in [
        ("border-top", &mut props.top),
        ("border-right", &mut props.right),
        ("border-bottom", &mut props.bottom),
        ("border-left", &mut props.left),
    ] {
        if let Some(b) = part.attr(attr) {
            *slot = Some(border_of(b, &mut props.losses));
        }
    }
    let diagonal = ["diagonal-bl-tr", "diagonal-tl-br"]
        .iter()
        .any(|d| part.attr(d).is_some_and(|v| v != "none"));
    if diagonal {
        props
            .losses
            .push((XlsxConstruct::DiagonalBorder, Outcome::Dropped));
    }
    let turned = part.attr("rotation-angle").is_some_and(|r| {
        r.trim_end_matches("deg")
            .parse::<f32>()
            .is_ok_and(|a| a % 360.0 != 0.0)
    });
    if turned {
        props
            .losses
            .push((XlsxConstruct::TextOrientation, Outcome::Dropped));
    }
}

fn paragraph_props(part: &Node, props: &mut Props) {
    if let Some(a) = part.attr("text-align") {
        props.h_align = Some(match a {
            "start" | "left" => HAlign::Left,
            "center" => HAlign::Center,
            "end" | "right" => HAlign::Right,
            _ => {
                props
                    .losses
                    .push((XlsxConstruct::Alignment, Outcome::Approximated));
                HAlign::Left
            }
        });
    }
    let indented = part
        .attr("margin-left")
        .and_then(length_px)
        .is_some_and(|px| px.get().abs() > 0.5);
    if indented {
        props
            .losses
            .push((XlsxConstruct::TextOrientation, Outcome::Dropped));
    }
}

/// The colour `#rrggbb` names, or `None` for `transparent` and any other
/// text.
fn parse_color(text: &str) -> Option<Rgb> {
    let hex = text.strip_prefix('#')?;
    (hex.len() == 6)
        .then(|| u32::from_str_radix(hex, 16).ok())
        .flatten()
        .map(Rgb::from_bits)
}

/// The edge a border property such as `0.74pt solid #000000` draws.
fn border_of(text: &str, losses: &mut Vec<(XlsxConstruct, Outcome)>) -> Option<CellBorder> {
    let mut width = None;
    let mut kind = None;
    let mut color = None;
    for word in text.split_whitespace() {
        if word.starts_with('#') {
            color = parse_color(word);
        } else if let Some(px) = length_px(word) {
            width = Some(f64::from(px.get()) * 0.75);
        } else {
            kind = Some(word);
        }
    }
    let points = width.unwrap_or(0.75);
    let style = match kind.unwrap_or("solid") {
        "none" | "hidden" => return None,
        "solid" if points > 2.0 => BorderStyle::Thick,
        "solid" if points > 1.0 => BorderStyle::Medium,
        "solid" => BorderStyle::Solid,
        "dashed" => BorderStyle::Dashed,
        "dotted" => BorderStyle::Dotted,
        "double" => BorderStyle::Double,
        "dash-dot" | "dash-dot-dot" | "fine-dashed" => {
            losses.push((XlsxConstruct::BorderStroke, Outcome::Approximated));
            BorderStyle::Dashed
        }
        _ => {
            losses.push((XlsxConstruct::BorderStroke, Outcome::Approximated));
            BorderStyle::Solid
        }
    };
    Some(CellBorder { style, color })
}

/// The parts a written document's styles take. They are the font
/// declarations, the named and number styles of the styles part, and the
/// automatic cell styles of the content part. The style at place `i` of `styles` is
/// the default style for `i == 0`, and the cell style `ce{i}` otherwise.
pub(crate) struct Written {
    pub(crate) fonts: String,
    pub(crate) named: String,
    pub(crate) automatic: String,
}

/// Write `styles`, recording each approximation in `report`.
pub(crate) fn write(styles: &[CellStyle], report: &mut Report) -> Written {
    let mut fonts: Vec<String> = vec![WRITTEN_FONT.to_owned()];
    for style in styles {
        if let Some(font) = &style.font {
            let label = font.label().to_owned();
            if !fonts.contains(&label) {
                fonts.push(label);
            }
        }
    }
    let mut font_decls = String::new();
    for font in &fonts {
        let mut name = String::new();
        push_escaped(&mut name, font);
        let _ = write!(
            font_decls,
            "<style:font-face style:name=\"{name}\" svg:font-family=\"{name}\"/>"
        );
    }
    let mut named = format!(
        "<number:number-style style:name=\"N0\"><number:number number:min-integer-digits=\"1\"/></number:number-style>\
         <style:default-style style:family=\"table-cell\">\
         <style:text-properties style:font-name=\"{WRITTEN_FONT}\" fo:font-size=\"{}pt\"/>\
         </style:default-style>",
        WRITTEN_SIZE.points()
    );
    let mut automatic = String::new();
    for (at, style) in styles.iter().enumerate() {
        let data = data_style(at, &style.format, &mut named, report);
        let body = style_body(style);
        if at == 0 {
            let _ = write!(
                named,
                "<style:style style:name=\"{DEFAULT}\" style:family=\"table-cell\" style:data-style-name=\"{data}\">{body}</style:style>"
            );
        } else {
            let _ = write!(
                automatic,
                "<style:style style:name=\"ce{at}\" style:family=\"table-cell\" style:parent-style-name=\"{DEFAULT}\" style:data-style-name=\"{data}\">{body}</style:style>"
            );
        }
    }
    if styles.is_empty() {
        let _ = write!(
            named,
            "<style:style style:name=\"{DEFAULT}\" style:family=\"table-cell\"/>"
        );
    }
    Written {
        fonts: font_decls,
        named,
        automatic,
    }
}

/// The name of the number style of `format`, written into `named` when it
/// is not the general one.
fn data_style(at: usize, format: &NumberFormat, named: &mut String, report: &mut Report) -> String {
    if *format == NumberFormat::General {
        return "N0".to_owned();
    }
    let (code, outcome) = crate::styles::code_of(format);
    if outcome != Outcome::Carried {
        report.record(XlsxConstruct::NumberFormat, outcome);
    }
    let name = format!("N{}", at + 1);
    if code == number::BOOLEAN_CODE {
        let _ = write!(
            named,
            "<number:boolean-style style:name=\"{name}\"><number:boolean/></number:boolean-style>"
        );
        return name;
    }
    let (xml, approximation) = number::style_of(&name, &code);
    if approximation == Approximation::Approximated {
        report.record(XlsxConstruct::NumberFormat, Outcome::Approximated);
    }
    named.push_str(&xml);
    name
}

/// The properties of a written style. Every property is written, so a cell
/// style never inherits one from the default style it overrides.
fn style_body(style: &CellStyle) -> String {
    let mut out = String::from("<style:table-cell-properties");
    let _ = write!(
        out,
        " fo:background-color=\"{}\"",
        style.bg.map_or_else(|| "transparent".to_owned(), hex)
    );
    for (attr, edge) in [
        ("fo:border-top", style.borders.top),
        ("fo:border-right", style.borders.right),
        ("fo:border-bottom", style.borders.bottom),
        ("fo:border-left", style.borders.left),
    ] {
        let _ = write!(out, " {attr}=\"{}\"", border_text(edge));
    }
    let _ = write!(
        out,
        " fo:wrap-option=\"{}\" style:vertical-align=\"{}\"",
        match style.wrap {
            WordBreak::Anywhere => "wrap",
            _ => "no-wrap",
        },
        match style.v_align {
            VAlign::Top => "top",
            VAlign::Middle => "middle",
            VAlign::Bottom => "bottom",
        }
    );
    match style.h_align {
        HAlign::Auto => out.push_str(" style:text-align-source=\"value-type\"/>"),
        _ => out.push_str(" style:text-align-source=\"fix\"/>"),
    }
    let align = match style.h_align {
        HAlign::Auto => None,
        HAlign::Left => Some("start"),
        HAlign::Center => Some("center"),
        HAlign::Right => Some("end"),
    };
    if let Some(align) = align {
        let _ = write!(
            out,
            "<style:paragraph-properties fo:text-align=\"{align}\"/>"
        );
    }
    let font = style.font.as_ref().map_or(WRITTEN_FONT, FontFamily::label);
    let size = style.size.unwrap_or(WRITTEN_SIZE).points();
    out.push_str("<style:text-properties style:font-name=\"");
    push_escaped(&mut out, font);
    let _ = write!(
        out,
        "\" fo:font-size=\"{size}pt\" fo:font-weight=\"{}\" fo:font-style=\"{}\" \
         style:text-underline-style=\"{}\" style:text-line-through-style=\"{}\"",
        match style.weight {
            Weight::Bold => "bold",
            _ => "normal",
        },
        match style.slant {
            Slant::Italic => "italic",
            _ => "normal",
        },
        match style.underline {
            Underline::Underlined => "solid",
            _ => "none",
        },
        match style.strikethrough {
            Strikethrough::Struck => "solid",
            _ => "none",
        },
    );
    match style.fg {
        Some(fg) => {
            let _ = write!(out, " fo:color=\"{}\"", hex(fg));
        }
        None => out.push_str(" style:use-window-font-color=\"true\""),
    }
    out.push_str("/>");
    out
}

fn hex(color: Rgb) -> String {
    format!("#{:06x}", color.bits())
}

/// The border property of one edge. An edge without a colour names none,
/// and reads back with none.
fn border_text(edge: Option<CellBorder>) -> String {
    let Some(edge) = edge else {
        return "none".to_owned();
    };
    let stroke = match edge.style {
        BorderStyle::Solid => "0.74pt solid",
        BorderStyle::Dashed => "0.74pt dashed",
        BorderStyle::Dotted => "0.74pt dotted",
        BorderStyle::Medium => "1.76pt solid",
        BorderStyle::Thick => "2.49pt solid",
        BorderStyle::Double => "2.01pt double",
    };
    match edge.color {
        Some(color) => format!("{stroke} {}", hex(color)),
        None => stroke.to_owned(),
    }
}
