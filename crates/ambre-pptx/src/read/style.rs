//! The theme a slide inherits and the colours it names (ECMA-376 Part 1,
//! sections 19.3.1.51 and 20.1.6). Also the text style a shape inherits
//! from its layout and its master.
//!
//! A slide names a colour by its role, `tx1` or `accent2`. The master maps
//! each role onto a slot of the theme, and the theme gives the slot its
//! value. A text takes each property from the first level of its chain
//! that sets it. The chain is its run, its own list style, its layout, its
//! master and the defaults of the presentation.

use ambre_core::{PrintLength, Pt, Rgba, Slant, Strikethrough, Underline, Weight};

use super::xml::{Element, Ns};

/// One slot of a theme's colour scheme.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Slot {
    Dark1,
    Light1,
    Dark2,
    Light2,
    Accent1,
    Accent2,
    Accent3,
    Accent4,
    Accent5,
    Accent6,
    Link,
    FollowedLink,
}

impl Slot {
    /// The slot a scheme element or a colour map value names.
    fn of(name: &str) -> Option<Self> {
        Some(match name {
            "dk1" => Self::Dark1,
            "lt1" => Self::Light1,
            "dk2" => Self::Dark2,
            "lt2" => Self::Light2,
            "accent1" => Self::Accent1,
            "accent2" => Self::Accent2,
            "accent3" => Self::Accent3,
            "accent4" => Self::Accent4,
            "accent5" => Self::Accent5,
            "accent6" => Self::Accent6,
            "hlink" => Self::Link,
            "folHlink" => Self::FollowedLink,
            _ => return None,
        })
    }

    /// The colour a theme without this slot gives it: the light and dark
    /// defaults of Office.
    const fn fallback(self) -> Rgba {
        match self {
            Self::Light1 | Self::Light2 => Rgba::from_bits(0xFFFF_FFFF),
            _ => Rgba::from_bits(0x0000_00FF),
        }
    }
}

/// The colours and the two faces of one theme.
#[derive(Clone, Debug, Default)]
pub struct Theme {
    scheme: Vec<(Slot, Rgba)>,
    /// The face of a heading, `+mj-lt`.
    major: String,
    /// The face of a body, `+mn-lt`.
    minor: String,
}

impl Theme {
    /// The theme `root`, a theme part, holds.
    pub fn read(root: &Element) -> Self {
        let elements = root.child(Ns::A, "themeElements");
        let scheme = elements
            .and_then(|e| e.child(Ns::A, "clrScheme"))
            .map(|scheme| {
                scheme
                    .elements()
                    .filter_map(|slot| {
                        let at = Slot::of(slot.name())?;
                        let colour = plain_colour(slot.elements().next()?)?;
                        Some((at, colour))
                    })
                    .collect()
            })
            .unwrap_or_default();
        let face = |which: &str| {
            elements
                .and_then(|e| e.path(Ns::A, &["fontScheme", which, "latin"]))
                .and_then(|latin| latin.attr("typeface"))
                .unwrap_or_default()
                .to_owned()
        };
        Self {
            scheme,
            major: face("majorFont"),
            minor: face("minorFont"),
        }
    }

    fn slot(&self, slot: Slot) -> Rgba {
        self.scheme
            .iter()
            .find(|(at, _)| *at == slot)
            .map_or(slot.fallback(), |(_, colour)| *colour)
    }

    /// The face a run names: a theme face for `+mj-lt` and `+mn-lt`, the
    /// name itself otherwise.
    pub fn face(&self, typeface: &str) -> String {
        match typeface {
            "+mj-lt" | "+mj-ea" | "+mj-cs" => self.major.clone(),
            "+mn-lt" | "+mn-ea" | "+mn-cs" => self.minor.clone(),
            other => other.to_owned(),
        }
    }
}

/// The map from the role a slide names to the slot of the theme: a
/// master's `p:clrMap`, which a layout or a slide may override.
#[derive(Clone, Debug)]
pub struct ColourMap {
    roles: Vec<(Role, Slot)>,
}

/// A role a slide names a colour by, which the map sends to a slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Role {
    Background1,
    Text1,
    Background2,
    Text2,
    /// A role that names a slot of its own name: `accent1` to `folHlink`.
    Slot(Slot),
}

impl Role {
    fn of(name: &str) -> Option<Self> {
        Some(match name {
            "bg1" => Self::Background1,
            "tx1" => Self::Text1,
            "bg2" => Self::Background2,
            "tx2" => Self::Text2,
            other => Self::Slot(Slot::of(other)?),
        })
    }
}

impl Default for ColourMap {
    /// The map of the default Office master: a dark text on a light
    /// background.
    fn default() -> Self {
        Self {
            roles: vec![
                (Role::Background1, Slot::Light1),
                (Role::Text1, Slot::Dark1),
                (Role::Background2, Slot::Light2),
                (Role::Text2, Slot::Dark2),
            ],
        }
    }
}

impl ColourMap {
    /// The map `element`, a `p:clrMap` or an `a:overrideClrMapping`, holds.
    /// A role it leaves out keeps the default.
    pub fn read(element: &Element) -> Self {
        let mut map = Self::default();
        for (role, name) in [
            (Role::Background1, "bg1"),
            (Role::Text1, "tx1"),
            (Role::Background2, "bg2"),
            (Role::Text2, "tx2"),
        ] {
            if let Some(slot) = element.attr(name).and_then(Slot::of) {
                map.roles.retain(|(at, _)| *at != role);
                map.roles.push((role, slot));
            }
        }
        map
    }

    fn slot(&self, role: Role) -> Option<Slot> {
        match role {
            Role::Slot(slot) => Some(slot),
            other => self
                .roles
                .iter()
                .find(|(at, _)| *at == other)
                .map(|(_, slot)| *slot),
        }
    }
}

/// The theme and the colour map one slide resolves its colours through.
#[derive(Clone, Copy)]
pub struct Palette<'a> {
    pub theme: &'a Theme,
    pub map: &'a ColourMap,
}

impl Palette<'_> {
    /// The colour a colour element names: `a:srgbClr`, `a:schemeClr`,
    /// `a:sysClr` or `a:prstClr`, with its modifiers applied. `None` for a
    /// placeholder colour and for an element no reader knows.
    pub fn colour(&self, element: &Element) -> Option<Rgba> {
        let base = if element.is(Ns::A, "schemeClr") {
            let role = Role::of(element.attr("val")?)?;
            self.theme.slot(self.map.slot(role)?)
        } else {
            plain_colour(element)?
        };
        Some(modified(base, element))
    }

    /// The colour of the first colour element under `parent`: the content
    /// of an `a:solidFill`, or of a style reference.
    pub fn first(&self, parent: &Element) -> Option<Rgba> {
        parent.elements().find_map(|child| self.colour(child))
    }
}

/// A colour that needs no theme. It is an RGB value, a system colour by
/// its last value, or a preset name a writer uses most.
fn plain_colour(element: &Element) -> Option<Rgba> {
    let hex = match element.name() {
        "srgbClr" => element.attr("val")?,
        "sysClr" => element.attr("lastClr")?,
        "prstClr" => match element.attr("val")? {
            "black" => "000000",
            "white" => "FFFFFF",
            "red" => "FF0000",
            "green" => "008000",
            "blue" => "0000FF",
            "yellow" => "FFFF00",
            "gray" | "grey" => "808080",
            _ => return None,
        },
        _ => return None,
    };
    let bits = u32::from_str_radix(hex, 16)
        .ok()
        .filter(|_| hex.len() == 6)?;
    Some(modified(Rgba::from_bits((bits << 8) | 0xFF), element))
}

/// `base` under the modifiers `element` holds, in document order: an alpha,
/// a luminance scale and offset, a tint toward white and a shade toward
/// black. Each value is in thousandths of a percent.
fn modified(base: Rgba, element: &Element) -> Rgba {
    let [r, g, b, a] = base.channels();
    let mut rgb = [f64::from(r), f64::from(g), f64::from(b)].map(|c| c / 255.0);
    let mut alpha = f64::from(a) / 255.0;
    for modifier in element.elements() {
        let Some(value) = modifier
            .attr("val")
            .and_then(|v| v.parse::<f64>().ok())
            .map(|v| v / 100_000.0)
            .filter(|v| v.is_finite())
        else {
            continue;
        };
        match modifier.name() {
            "alpha" => alpha = value,
            "lumMod" | "lumOff" => {
                let (h, s, l) = hsl(rgb);
                let l = if modifier.name() == "lumMod" {
                    l * value
                } else {
                    l + value
                };
                rgb = from_hsl(h, s, l.clamp(0.0, 1.0));
            }
            "tint" => rgb = rgb.map(|c| 1.0 - (1.0 - c) * value),
            "shade" => rgb = rgb.map(|c| c * value),
            _ => {}
        }
    }
    let byte = |c: f64| (c.clamp(0.0, 1.0) * 255.0).round() as u8;
    let [r, g, b] = rgb.map(byte);
    Rgba::from_channels(r, g, b, byte(alpha))
}

/// The hue, saturation and lightness of a colour, each in `0..=1`.
fn hsl([r, g, b]: [f64; 3]) -> (f64, f64, f64) {
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let l = (max + min) / 2.0;
    let d = max - min;
    if d <= f64::EPSILON {
        return (0.0, 0.0, l);
    }
    let s = if l > 0.5 {
        d / (2.0 - max - min)
    } else {
        d / (max + min)
    };
    let h = if (max - r).abs() <= f64::EPSILON {
        (g - b) / d + if g < b { 6.0 } else { 0.0 }
    } else if (max - g).abs() <= f64::EPSILON {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    };
    (h / 6.0, s, l)
}

/// The colour of a hue, a saturation and a lightness.
fn from_hsl(h: f64, s: f64, l: f64) -> [f64; 3] {
    if s <= f64::EPSILON {
        return [l, l, l];
    }
    let q = if l < 0.5 {
        l * (1.0 + s)
    } else {
        l + s - l * s
    };
    let p = 2.0 * l - q;
    let channel = |t: f64| {
        let t = t.rem_euclid(1.0);
        if t < 1.0 / 6.0 {
            p + (q - p) * 6.0 * t
        } else if t < 0.5 {
            q
        } else if t < 2.0 / 3.0 {
            p + (q - p) * (2.0 / 3.0 - t) * 6.0
        } else {
            p
        }
    };
    [channel(h + 1.0 / 3.0), channel(h), channel(h - 1.0 / 3.0)]
}

/// The properties of a run a level of the chain may set. `None` leaves a
/// property to the next level.
#[derive(Clone, Debug, Default)]
pub struct RunStyle {
    pub size: Option<PrintLength<Pt>>,
    pub weight: Option<Weight>,
    pub slant: Option<Slant>,
    pub underline: Option<Underline>,
    pub strikethrough: Option<Strikethrough>,
    pub colour: Option<Rgba>,
    /// The face, a theme face already resolved to its name.
    pub typeface: Option<String>,
}

impl RunStyle {
    /// The properties `props`, an `a:rPr`, `a:defRPr` or `a:endParaRPr`,
    /// sets.
    pub fn read(props: &Element, palette: Palette<'_>) -> Self {
        let on = |name: &str| props.attr(name).map(|v| v == "1" || v == "true");
        Self {
            size: props
                .attr("sz")
                .and_then(|v| v.parse::<f32>().ok())
                .filter(|v| v.is_finite() && *v > 0.0)
                .map(|v| PrintLength::<Pt>::new(v / 100.0)),
            weight: on("b").map(|b| if b { Weight::Bold } else { Weight::Regular }),
            slant: on("i").map(|i| if i { Slant::Italic } else { Slant::Upright }),
            underline: props.attr("u").map(|u| {
                if u == "none" {
                    Underline::Plain
                } else {
                    Underline::Underlined
                }
            }),
            strikethrough: props.attr("strike").map(|s| {
                if s == "noStrike" {
                    Strikethrough::Plain
                } else {
                    Strikethrough::Struck
                }
            }),
            colour: props
                .child(Ns::A, "solidFill")
                .and_then(|fill| palette.first(fill)),
            typeface: props
                .child(Ns::A, "latin")
                .and_then(|latin| latin.attr("typeface"))
                .map(|face| palette.theme.face(face)),
        }
    }

    /// This style, with each property it leaves unset taken from `below`.
    #[must_use]
    pub fn over(self, below: Self) -> Self {
        Self {
            size: self.size.or(below.size),
            weight: self.weight.or(below.weight),
            slant: self.slant.or(below.slant),
            underline: self.underline.or(below.underline),
            strikethrough: self.strikethrough.or(below.strikethrough),
            colour: self.colour.or(below.colour),
            typeface: self.typeface.or(below.typeface),
        }
    }
}

/// The name of the level element of a list style for a paragraph level:
/// `a:lvl1pPr` for the first.
pub fn level_name(level: usize) -> &'static str {
    const NAMES: [&str; 9] = [
        "lvl1pPr", "lvl2pPr", "lvl3pPr", "lvl4pPr", "lvl5pPr", "lvl6pPr", "lvl7pPr", "lvl8pPr",
        "lvl9pPr",
    ];
    NAMES.get(level).copied().unwrap_or("lvl1pPr")
}
