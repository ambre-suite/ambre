//! The styles part: fonts, fills, borders, number formats and the cell
//! formats that combine them, read into Trame's cell style and written back.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use ambre_core::{
    Bounded, FontFamily, FontSize, Rgb, Slant, Strikethrough, Underline, Validated, Weight,
    WordBreak,
};

use crate::model::{
    BorderStyle, Borders, CellBorder, CellStyle, Decimals, FormatCode, HAlign, NumberFormat, VAlign,
};
use crate::report::{Outcome, Report, XlsxConstruct};
use crate::xml::{self, Element};

/// The font a workbook names first, which every cell without a font of its
/// own draws with. Trame reads it as its own default.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct DefaultFont {
    name: String,
    size: Option<FontSize>,
    color: Option<Rgb>,
}

/// The default font of a written workbook: Calibri at 11 points, the font a
/// new Excel workbook names.
fn written_default_font() -> DefaultFont {
    DefaultFont {
        name: "Calibri".to_owned(),
        size: FontSize::new(11.0),
        color: None,
    }
}

/// The colours a theme names, in the order a `theme` attribute counts them.
/// The order is light 1, dark 1, light 2, dark 2, the six accents, and the
/// two hyperlink colours.
#[derive(Debug, Clone, Default)]
pub(crate) struct Theme {
    colors: Vec<Rgb>,
}

impl Theme {
    /// The colours of the theme part `root`. A theme that names none leaves
    /// every theme colour to the default.
    pub(crate) fn read(root: &Element) -> Self {
        let Some(scheme) = root
            .child("themeElements")
            .and_then(|e| e.child("clrScheme"))
        else {
            return Self::default();
        };
        let order = [
            "dk1", "lt1", "dk2", "lt2", "accent1", "accent2", "accent3", "accent4", "accent5",
            "accent6", "hlink", "folHlink",
        ];
        let read = |name: &str| -> Rgb {
            scheme
                .child(name)
                .and_then(|slot| {
                    slot.all_children().first().and_then(|c| match c.name() {
                        "srgbClr" => c.attr("val").and_then(parse_rgb),
                        "sysClr" => c.attr("lastClr").and_then(parse_rgb),
                        _ => None,
                    })
                })
                .unwrap_or_default()
        };
        let mut colors: Vec<Rgb> = order.iter().map(|n| read(n)).collect();
        // The `theme` attribute counts light before dark: index 0 is lt1.
        colors.swap(0, 1);
        colors.swap(2, 3);
        Self { colors }
    }
}

/// The 64 colours an `indexed` attribute names when the workbook overrides
/// none (ECMA-376 part 1, 18.8.27).
const INDEXED: [u32; 64] = [
    0x000000, 0xFFFFFF, 0xFF0000, 0x00FF00, 0x0000FF, 0xFFFF00, 0xFF00FF, 0x00FFFF, 0x000000,
    0xFFFFFF, 0xFF0000, 0x00FF00, 0x0000FF, 0xFFFF00, 0xFF00FF, 0x00FFFF, 0x800000, 0x008000,
    0x000080, 0x808000, 0x800080, 0x008080, 0xC0C0C0, 0x808080, 0x9999FF, 0x993366, 0xFFFFCC,
    0xCCFFFF, 0x660066, 0xFF8080, 0x0066CC, 0xCCCCFF, 0x000080, 0xFF00FF, 0xFFFF00, 0x00FFFF,
    0x800080, 0x800000, 0x008080, 0x0000FF, 0x00CCFF, 0xCCFFFF, 0xCCFFCC, 0xFFFF99, 0x99CCFF,
    0xFF99CC, 0xCC99FF, 0xFFCC99, 0x3366FF, 0x33CCCC, 0x99CC00, 0xFFCC00, 0xFF9900, 0xFF6600,
    0x666699, 0x969696, 0x003366, 0x339966, 0x003300, 0x333300, 0x993300, 0x993366, 0x333399,
    0x333333,
];

/// The colour an `RRGGBB` or `AARRGGBB` text names.
fn parse_rgb(text: &str) -> Option<Rgb> {
    let hex = match text.len() {
        8 => text.get(2..)?,
        6 => text,
        _ => return None,
    };
    u32::from_str_radix(hex, 16).ok().map(Rgb::from_bits)
}

/// What a read needs beside the styles part: the theme and the palette.
pub(crate) struct Palette {
    theme: Theme,
    indexed: Vec<Rgb>,
}

impl Palette {
    pub(crate) fn new(theme: Theme, root: Option<&Element>) -> Self {
        let custom: Vec<Rgb> = root
            .and_then(|r| r.child("colors"))
            .and_then(|c| c.child("indexedColors"))
            .map(|ic| {
                ic.children("rgbColor")
                    .filter_map(|c| c.attr("rgb").and_then(parse_rgb))
                    .collect()
            })
            .unwrap_or_default();
        let indexed = if custom.is_empty() {
            INDEXED.iter().map(|&b| Rgb::from_bits(b)).collect()
        } else {
            custom
        };
        Self { theme, indexed }
    }

    /// The colour a colour element names, or `None` for the automatic
    /// colour. A theme colour and a tint resolve to the colour they draw.
    fn color(&self, element: &Element) -> Option<Rgb> {
        if element.attr("auto") == Some("1") || element.attr("auto") == Some("true") {
            return None;
        }
        let base = if let Some(rgb) = element.attr("rgb") {
            parse_rgb(rgb)?
        } else if let Some(index) = element.attr("indexed") {
            let index: usize = index.parse().ok()?;
            // 64 and 65 are the system foreground and background.
            *self.indexed.get(index)?
        } else {
            let index: usize = element.attr("theme")?.parse().ok()?;
            *self.theme.colors.get(index)?
        };
        match element.attr("tint").and_then(|t| t.parse::<f64>().ok()) {
            Some(tint) if tint != 0.0 => Some(apply_tint(base, tint)),
            _ => Some(base),
        }
    }
}

/// `color` lightened or darkened by `tint`, through its HLS lightness, as
/// Excel draws a tinted colour (ECMA-376 part 1, 18.8.19).
fn apply_tint(color: Rgb, tint: f64) -> Rgb {
    let [r, g, b] = color.channels().map(|c| f64::from(c) / 255.0);
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let mut l = f64::midpoint(max, min);
    let d = max - min;
    let (h, s) = if d == 0.0 {
        (0.0, 0.0)
    } else {
        let s = if l > 0.5 {
            d / (2.0 - max - min)
        } else {
            d / (max + min)
        };
        let h = if max == r {
            ((g - b) / d).rem_euclid(6.0)
        } else if max == g {
            (b - r) / d + 2.0
        } else {
            (r - g) / d + 4.0
        };
        (h / 6.0, s)
    };
    l = if tint < 0.0 {
        l * (1.0 + tint)
    } else {
        l * (1.0 - tint) + tint
    };
    let q = if l < 0.5 {
        l * (1.0 + s)
    } else {
        l + s - l * s
    };
    let p = 2.0 * l - q;
    let channel = |t: f64| -> u8 {
        let t = t.rem_euclid(1.0);
        let v = if t < 1.0 / 6.0 {
            p + (q - p) * 6.0 * t
        } else if t < 0.5 {
            q
        } else if t < 2.0 / 3.0 {
            p + (q - p) * (2.0 / 3.0 - t) * 6.0
        } else {
            p
        };
        // The value lies in 0..=1, so the rounded product fits a byte.
        u8::try_from((v.clamp(0.0, 1.0) * 255.0).round() as u32).unwrap_or(u8::MAX)
    };
    Rgb::from_channels(channel(h + 1.0 / 3.0), channel(h), channel(h - 1.0 / 3.0))
}

/// The number format codes Excel implies for the built-in ids
/// (ECMA-376 part 1, 18.8.30). An id missing here is locale-specific.
fn builtin_code(id: u32) -> Option<&'static str> {
    Some(match id {
        0 => "General",
        1 => "0",
        2 => "0.00",
        3 => "#,##0",
        4 => "#,##0.00",
        9 => "0%",
        10 => "0.00%",
        11 => "0.00E+00",
        12 => "# ?/?",
        13 => "# ??/??",
        14 => "mm-dd-yy",
        15 => "d-mmm-yy",
        16 => "d-mmm",
        17 => "mmm-yy",
        18 => "h:mm AM/PM",
        19 => "h:mm:ss AM/PM",
        20 => "h:mm",
        21 => "h:mm:ss",
        22 => "m/d/yy h:mm",
        37 => "#,##0 ;(#,##0)",
        38 => "#,##0 ;[Red](#,##0)",
        39 => "#,##0.00;(#,##0.00)",
        40 => "#,##0.00;[Red](#,##0.00)",
        45 => "mm:ss",
        46 => "[h]:mm:ss",
        47 => "mmss.0",
        48 => "##0.0E+0",
        49 => "@",
        _ => return None,
    })
}

/// The number format a code reads as: a family where the code is one of a
/// family's own, a pattern otherwise.
pub(crate) fn number_format_of(code: &str) -> NumberFormat {
    let decimals_after = |prefix: &str, suffix: &str| -> Option<Bounded<Decimals>> {
        let rest = code.strip_prefix(prefix)?.strip_suffix(suffix)?;
        if rest.is_empty() {
            return Bounded::new(0);
        }
        let zeros = rest.strip_prefix('.')?;
        if zeros.is_empty() || !zeros.bytes().all(|b| b == b'0') {
            return None;
        }
        Bounded::new(u8::try_from(zeros.len()).ok()?)
    };
    match code {
        "General" | "general" => return NumberFormat::General,
        "@" => return NumberFormat::Text,
        "# ?/?" | "# ??/??" => return NumberFormat::Fraction,
        "mm-dd-yy" | "yyyy-mm-dd" | "yyyy\\-mm\\-dd" => return NumberFormat::Date,
        "h:mm:ss" | "hh:mm:ss" => return NumberFormat::Time,
        _ => {}
    }
    if let Some(d) = decimals_after("0", "%") {
        return NumberFormat::Percent(d);
    }
    if let Some(d) = decimals_after("0", "E+00") {
        return NumberFormat::Scientific(d);
    }
    if let Some(d) = decimals_after("0", "") {
        return NumberFormat::Number(d);
    }
    Validated::<FormatCode>::parse(code.to_owned())
        .map_or(NumberFormat::General, NumberFormat::Custom)
}

/// The code a number format writes as, and whether the code only
/// approximates the family.
pub(crate) fn code_of(format: &NumberFormat) -> (String, Outcome) {
    let zeros = |d: Bounded<Decimals>| -> String {
        if d.get() == 0 {
            String::new()
        } else {
            format!(".{}", "0".repeat(usize::from(d.get())))
        }
    };
    match format {
        NumberFormat::General => ("General".to_owned(), Outcome::Carried),
        NumberFormat::Number(d) => (format!("0{}", zeros(*d)), Outcome::Carried),
        // Trame draws the currency of the reader's locale; the file names
        // the generic currency sign, which Excel draws in its own locale.
        NumberFormat::Currency(d) => (
            format!("#,##0{} [$\u{a4}]", zeros(*d)),
            Outcome::Approximated,
        ),
        NumberFormat::Percent(d) => (format!("0{}%", zeros(*d)), Outcome::Carried),
        NumberFormat::Scientific(d) => (format!("0{}E+00", zeros(*d)), Outcome::Carried),
        NumberFormat::Fraction => ("# ?/?".to_owned(), Outcome::Carried),
        NumberFormat::Date => ("yyyy-mm-dd".to_owned(), Outcome::Carried),
        NumberFormat::Time => ("hh:mm:ss".to_owned(), Outcome::Carried),
        NumberFormat::Text => ("@".to_owned(), Outcome::Carried),
        NumberFormat::Custom(code) => (code.as_str().to_owned(), Outcome::Carried),
    }
}

/// Whether a number format shows its value as a calendar date. A 1904
/// workbook moves the serial of such a cell to the 1900 date system. A time
/// of day and an elapsed time read the same in both systems, and stay.
pub(crate) fn is_calendar_date(format: &NumberFormat) -> bool {
    match format {
        NumberFormat::Date => true,
        NumberFormat::Custom(code) => {
            let mut in_quote = false;
            let mut in_bracket = false;
            code.as_str().chars().any(|c| {
                match c {
                    '"' => in_quote = !in_quote,
                    '[' => in_bracket = true,
                    ']' => in_bracket = false,
                    _ => {}
                }
                !in_quote && !in_bracket && matches!(c.to_ascii_lowercase(), 'd' | 'y')
            })
        }
        _ => false,
    }
}

/// The styles a workbook's cell formats resolve to, by cell format index, and
/// what the read approximated or dropped on the way.
pub(crate) struct ReadStyles {
    pub(crate) by_xf: Vec<CellStyle>,
}

/// Read the styles part `root` against `palette`.
pub(crate) fn read(root: Option<&Element>, palette: &Palette, report: &mut Report) -> ReadStyles {
    let Some(root) = root else {
        return ReadStyles {
            by_xf: vec![CellStyle::default()],
        };
    };
    let custom_codes: BTreeMap<u32, String> = root
        .child("numFmts")
        .map(|n| {
            n.children("numFmt")
                .filter_map(|f| {
                    Some((
                        f.attr("numFmtId")?.parse().ok()?,
                        f.attr("formatCode")?.to_owned(),
                    ))
                })
                .collect()
        })
        .unwrap_or_default();
    let fonts: Vec<&Element> = root
        .child("fonts")
        .map(|f| f.children("font").collect())
        .unwrap_or_default();
    let fills: Vec<&Element> = root
        .child("fills")
        .map(|f| f.children("fill").collect())
        .unwrap_or_default();
    let borders: Vec<&Element> = root
        .child("borders")
        .map(|b| b.children("border").collect())
        .unwrap_or_default();
    let default_font = fonts
        .first()
        .map_or_else(written_default_font, |f| DefaultFont {
            name: f
                .child("name")
                .and_then(|n| n.attr("val"))
                .unwrap_or("Calibri")
                .to_owned(),
            size: f
                .child("sz")
                .and_then(|s| s.attr("val"))
                .and_then(|v| v.parse().ok())
                .and_then(FontSize::new),
            color: f.child("color").and_then(|c| palette.color(c)),
        });
    let by_xf = root
        .child("cellXfs")
        .map(|x| x.children("xf").collect::<Vec<_>>())
        .unwrap_or_default()
        .into_iter()
        .map(|xf| {
            let index = |name: &str| {
                xf.attr(name)
                    .and_then(|v| v.parse::<usize>().ok())
                    .unwrap_or(0)
            };
            let mut style = CellStyle::default();
            if let Some(font) = fonts.get(index("fontId")) {
                read_font(font, &default_font, palette, &mut style, report);
            }
            if let Some(fill) = fills.get(index("fillId")) {
                style.bg = read_fill(fill, palette, report);
            }
            if let Some(border) = borders.get(index("borderId")) {
                style.borders = read_border(border, palette, report);
            }
            let id = xf
                .attr("numFmtId")
                .and_then(|v| v.parse::<u32>().ok())
                .unwrap_or(0);
            let code = custom_codes
                .get(&id)
                .map(String::as_str)
                .or_else(|| builtin_code(id));
            style.format = code.map_or(NumberFormat::General, number_format_of);
            read_alignment(xf.child("alignment"), &mut style, report);
            style
        })
        .collect::<Vec<_>>();
    ReadStyles {
        by_xf: if by_xf.is_empty() {
            vec![CellStyle::default()]
        } else {
            by_xf
        },
    }
}

/// Whether an element that toggles a font attribute turns it on: present,
/// and not `val="0"` or `val="false"`.
fn is_on(element: Option<&Element>) -> bool {
    element.is_some_and(|e| !matches!(e.attr("val"), Some("0" | "false")))
}

fn read_font(
    font: &Element,
    default: &DefaultFont,
    palette: &Palette,
    style: &mut CellStyle,
    report: &mut Report,
) {
    style.weight = if is_on(font.child("b")) {
        Weight::Bold
    } else {
        Weight::Regular
    };
    style.slant = if is_on(font.child("i")) {
        Slant::Italic
    } else {
        Slant::Upright
    };
    style.strikethrough = if is_on(font.child("strike")) {
        Strikethrough::Struck
    } else {
        Strikethrough::Plain
    };
    style.underline = match font.child("u").map(|u| u.attr("val").unwrap_or("single")) {
        None | Some("none") => Underline::Plain,
        Some("single") => Underline::Underlined,
        Some(_) => {
            report.record(XlsxConstruct::FontEffect, Outcome::Approximated);
            Underline::Underlined
        }
    };
    let effect = font
        .child("vertAlign")
        .is_some_and(|v| v.attr("val") != Some("baseline"))
        || is_on(font.child("outline"))
        || is_on(font.child("shadow"));
    if effect {
        report.record(XlsxConstruct::FontEffect, Outcome::Dropped);
    }
    // The default font's colour is the theme's text colour, which Trame
    // draws in the reader's light or dark theme.
    style.fg = font
        .child("color")
        .and_then(|c| palette.color(c))
        .filter(|fg| Some(*fg) != default.color);
    style.font = font
        .child("name")
        .and_then(|n| n.attr("val"))
        .filter(|name| *name != default.name)
        .map(FontFamily::from_label);
    style.size = font
        .child("sz")
        .and_then(|s| s.attr("val"))
        .and_then(|v| v.parse::<f32>().ok())
        .and_then(FontSize::new)
        .filter(|size| Some(*size) != default.size);
}

fn read_fill(fill: &Element, palette: &Palette, report: &mut Report) -> Option<Rgb> {
    if fill.child("gradientFill").is_some() {
        report.record(XlsxConstruct::GradientFill, Outcome::Dropped);
        return None;
    }
    let pattern = fill.child("patternFill")?;
    match pattern.attr("patternType").unwrap_or("none") {
        "none" => None,
        kind => {
            if kind != "solid" {
                report.record(XlsxConstruct::PatternFill, Outcome::Approximated);
            }
            pattern.child("fgColor").and_then(|c| palette.color(c))
        }
    }
}

fn read_border(border: &Element, palette: &Palette, report: &mut Report) -> Borders {
    let mut edge = |name: &str| -> Option<CellBorder> {
        let e = border.child(name)?;
        let style = match e.attr("style")? {
            "none" => return None,
            "thin" => BorderStyle::Solid,
            "medium" => BorderStyle::Medium,
            "thick" => BorderStyle::Thick,
            "double" => BorderStyle::Double,
            "dashed" => BorderStyle::Dashed,
            "dotted" => BorderStyle::Dotted,
            "hair" => {
                report.record(XlsxConstruct::BorderStroke, Outcome::Approximated);
                BorderStyle::Dotted
            }
            "mediumDashed" | "dashDot" | "mediumDashDot" | "dashDotDot" | "mediumDashDotDot"
            | "slantDashDot" => {
                report.record(XlsxConstruct::BorderStroke, Outcome::Approximated);
                BorderStyle::Dashed
            }
            _ => {
                report.record(XlsxConstruct::BorderStroke, Outcome::Approximated);
                BorderStyle::Solid
            }
        };
        let color = e.child("color").and_then(|c| palette.color(c));
        Some(CellBorder { style, color })
    };
    let borders = Borders {
        top: edge("top"),
        right: edge("right").or_else(|| edge("end")),
        bottom: edge("bottom"),
        left: edge("left").or_else(|| edge("start")),
    };
    let diagonal = border
        .child("diagonal")
        .and_then(|d| d.attr("style"))
        .is_some_and(|s| s != "none");
    if diagonal {
        report.record(XlsxConstruct::DiagonalBorder, Outcome::Dropped);
    }
    borders
}

fn read_alignment(alignment: Option<&Element>, style: &mut CellStyle, report: &mut Report) {
    // An Excel cell without alignment sits at the bottom.
    style.v_align = VAlign::Bottom;
    let Some(a) = alignment else {
        return;
    };
    style.h_align = match a.attr("horizontal").unwrap_or("general") {
        "general" => HAlign::Auto,
        "left" => HAlign::Left,
        "center" => HAlign::Center,
        "right" => HAlign::Right,
        "centerContinuous" => {
            report.record(XlsxConstruct::Alignment, Outcome::Approximated);
            HAlign::Center
        }
        _ => {
            report.record(XlsxConstruct::Alignment, Outcome::Approximated);
            HAlign::Left
        }
    };
    style.v_align = match a.attr("vertical").unwrap_or("bottom") {
        "top" => VAlign::Top,
        "center" => VAlign::Middle,
        "bottom" => VAlign::Bottom,
        _ => {
            report.record(XlsxConstruct::Alignment, Outcome::Approximated);
            VAlign::Middle
        }
    };
    style.wrap = if matches!(a.attr("wrapText"), Some("1" | "true")) {
        WordBreak::Anywhere
    } else {
        WordBreak::Whole
    };
    let turned = a.attr("textRotation").is_some_and(|r| r != "0")
        || a.attr("indent").is_some_and(|i| i != "0");
    if turned {
        report.record(XlsxConstruct::TextOrientation, Outcome::Dropped);
    }
}

/// The styles part for `styles`, whose order gives the cell format indexes:
/// the cell format of `styles[i]` is index `i`. Equal fonts, fills, borders
/// and codes are written once.
pub(crate) fn write(styles: &[CellStyle], report: &mut Report) -> String {
    let default_font = written_default_font();
    let mut fonts: Vec<String> = vec![font_xml(&CellStyle::default(), &default_font)];
    // The format requires the first two fills: none, and the grey pattern.
    let mut fills: Vec<String> = vec![
        "<fill><patternFill patternType=\"none\"/></fill>".to_owned(),
        "<fill><patternFill patternType=\"gray125\"/></fill>".to_owned(),
    ];
    let mut borders: Vec<String> = vec![border_xml(&Borders::default())];
    let mut codes: Vec<String> = Vec::new();
    let mut xfs = String::new();
    let intern = |list: &mut Vec<String>, item: String| -> usize {
        list.iter().position(|x| *x == item).unwrap_or_else(|| {
            list.push(item);
            list.len() - 1
        })
    };
    for style in styles {
        let font = intern(&mut fonts, font_xml(style, &default_font));
        let fill = match style.bg {
            None => 0,
            Some(bg) => intern(
                &mut fills,
                format!(
                    "<fill><patternFill patternType=\"solid\"><fgColor rgb=\"FF{:06X}\"/><bgColor indexed=\"64\"/></patternFill></fill>",
                    bg.bits()
                ),
            ),
        };
        let border = intern(&mut borders, border_xml(&style.borders));
        let (code, outcome) = code_of(&style.format);
        if outcome != Outcome::Carried {
            report.record(XlsxConstruct::NumberFormat, outcome);
        }
        let num_fmt = builtin_id(&code).unwrap_or_else(|| {
            let at = intern(&mut codes, code.clone());
            164 + u32::try_from(at).unwrap_or(0)
        });
        let _ = write!(
            xfs,
            "<xf numFmtId=\"{num_fmt}\" fontId=\"{font}\" fillId=\"{fill}\" borderId=\"{border}\" xfId=\"0\"{}{}{}{}>{}</xf>",
            if num_fmt == 0 {
                ""
            } else {
                " applyNumberFormat=\"1\""
            },
            if font == 0 { "" } else { " applyFont=\"1\"" },
            if fill == 0 { "" } else { " applyFill=\"1\"" },
            if border == 0 {
                ""
            } else {
                " applyBorder=\"1\""
            },
            alignment_xml(style),
        );
    }
    let mut out = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n\
         <styleSheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\">",
    );
    if !codes.is_empty() {
        let _ = write!(out, "<numFmts count=\"{}\">", codes.len());
        for (i, code) in codes.iter().enumerate() {
            let _ = write!(
                out,
                "<numFmt numFmtId=\"{}\" formatCode=\"{}\"/>",
                164 + i,
                xml::escape(code)
            );
        }
        out.push_str("</numFmts>");
    }
    let _ = write!(
        out,
        "<fonts count=\"{}\">{}</fonts>",
        fonts.len(),
        fonts.concat()
    );
    let _ = write!(
        out,
        "<fills count=\"{}\">{}</fills>",
        fills.len(),
        fills.concat()
    );
    let _ = write!(
        out,
        "<borders count=\"{}\">{}</borders>",
        borders.len(),
        borders.concat()
    );
    out.push_str(
        "<cellStyleXfs count=\"1\"><xf numFmtId=\"0\" fontId=\"0\" fillId=\"0\" borderId=\"0\"/></cellStyleXfs>",
    );
    let _ = write!(
        out,
        "<cellXfs count=\"{}\">{xfs}</cellXfs>",
        styles.len().max(1)
    );
    out.push_str(
        "<cellStyles count=\"1\"><cellStyle name=\"Normal\" xfId=\"0\" builtinId=\"0\"/></cellStyles>\
         </styleSheet>",
    );
    out
}

/// The built-in id of a code, when one implies it.
fn builtin_id(code: &str) -> Option<u32> {
    (0..=49).find(|&id| builtin_code(id) == Some(code))
}

fn font_xml(style: &CellStyle, default: &DefaultFont) -> String {
    let mut out = String::from("<font>");
    if style.weight == Weight::Bold {
        out.push_str("<b/>");
    }
    if style.slant == Slant::Italic {
        out.push_str("<i/>");
    }
    if style.strikethrough == Strikethrough::Struck {
        out.push_str("<strike/>");
    }
    if style.underline == Underline::Underlined {
        out.push_str("<u/>");
    }
    let size = style.size.or(default.size).map_or(11.0, FontSize::points);
    let _ = write!(out, "<sz val=\"{size}\"/>");
    match style.fg {
        Some(fg) => {
            let _ = write!(out, "<color rgb=\"FF{:06X}\"/>", fg.bits());
        }
        None => out.push_str("<color theme=\"1\"/>"),
    }
    let name = style
        .font
        .as_ref()
        .map_or(default.name.as_str(), FontFamily::label);
    let _ = write!(
        out,
        "<name val=\"{}\"/><family val=\"2\"/></font>",
        xml::escape(name)
    );
    out
}

fn border_xml(borders: &Borders) -> String {
    let edge = |name: &str, edge: Option<CellBorder>| -> String {
        match edge {
            None => format!("<{name}/>"),
            Some(b) => {
                let style = match b.style {
                    BorderStyle::Solid => "thin",
                    BorderStyle::Dashed => "dashed",
                    BorderStyle::Dotted => "dotted",
                    BorderStyle::Medium => "medium",
                    BorderStyle::Thick => "thick",
                    BorderStyle::Double => "double",
                };
                let color = b.color.map_or_else(
                    || "<color indexed=\"64\"/>".to_owned(),
                    |c| format!("<color rgb=\"FF{:06X}\"/>", c.bits()),
                );
                format!("<{name} style=\"{style}\">{color}</{name}>")
            }
        }
    };
    format!(
        "<border>{}{}{}{}<diagonal/></border>",
        edge("left", borders.left),
        edge("right", borders.right),
        edge("top", borders.top),
        edge("bottom", borders.bottom)
    )
}

fn alignment_xml(style: &CellStyle) -> String {
    let horizontal = match style.h_align {
        HAlign::Auto => None,
        HAlign::Left => Some("left"),
        HAlign::Center => Some("center"),
        HAlign::Right => Some("right"),
    };
    let vertical = match style.v_align {
        VAlign::Top => Some("top"),
        VAlign::Middle => Some("center"),
        VAlign::Bottom => None,
    };
    let wrap = style.wrap == WordBreak::Anywhere;
    if horizontal.is_none() && vertical.is_none() && !wrap {
        return String::new();
    }
    let mut out = String::from("<alignment");
    if let Some(h) = horizontal {
        let _ = write!(out, " horizontal=\"{h}\"");
    }
    if let Some(v) = vertical {
        let _ = write!(out, " vertical=\"{v}\"");
    }
    if wrap {
        out.push_str(" wrapText=\"1\"");
    }
    out.push_str("/>");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_family_code_reads_as_its_family_and_any_other_as_a_pattern() {
        let d = |n: u8| Bounded::<Decimals>::new(n).unwrap_or(Bounded::MIN);
        assert_eq!(number_format_of("0.00"), NumberFormat::Number(d(2)));
        assert_eq!(number_format_of("0%"), NumberFormat::Percent(d(0)));
        assert_eq!(
            number_format_of("0.000E+00"),
            NumberFormat::Scientific(d(3))
        );
        assert_eq!(number_format_of("@"), NumberFormat::Text);
        assert!(matches!(
            number_format_of("#,##0.00"),
            NumberFormat::Custom(_)
        ));
        for format in [
            NumberFormat::Number(d(3)),
            NumberFormat::Percent(d(1)),
            NumberFormat::Scientific(d(2)),
            NumberFormat::Date,
            NumberFormat::Time,
            NumberFormat::Text,
            NumberFormat::Fraction,
            NumberFormat::General,
        ] {
            assert_eq!(number_format_of(&code_of(&format).0), format);
        }
    }

    #[test]
    fn a_date_pattern_is_a_calendar_date_and_a_duration_is_not() {
        let custom = |c: &str| {
            NumberFormat::Custom(
                Validated::<FormatCode>::parse(c.to_owned()).unwrap_or_else(|_| panic!("{c}")),
            )
        };
        assert!(is_calendar_date(&custom("d-mmm-yy")));
        assert!(!is_calendar_date(&custom("0.00 \"days\"")));
        assert!(!is_calendar_date(&custom("[Red]0.00")));
        assert!(!is_calendar_date(&custom("[h]:mm:ss")));
        assert!(!is_calendar_date(&NumberFormat::Time));
    }

    #[test]
    fn a_tint_lightens_or_darkens_the_colour() {
        let blue = Rgb::from_bits(0x4472C4);
        assert_eq!(apply_tint(blue, 0.0), blue);
        let lighter = apply_tint(blue, 0.8).channels();
        let darker = apply_tint(blue, -0.5).channels();
        assert!(lighter.iter().zip(blue.channels()).all(|(l, b)| *l >= b));
        assert!(darker.iter().zip(blue.channels()).all(|(d, b)| *d <= b));
        assert_eq!(
            apply_tint(Rgb::from_bits(0x000000), 1.0),
            Rgb::from_bits(0xFFFFFF)
        );
    }
}
