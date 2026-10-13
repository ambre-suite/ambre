// ambre-canvas/src/theme.rs
// ============================================================================
// Design tokens, consumed (ambre-spec-design-tokens.md, ambre-spec-frame.md). The colour
// palettes and scales are generated from `tokens.ron` by `build.rs`; this module
// wraps them in the runtime theme switch shared across the canvas/DOM boundary:
//
//   Palette / LIGHT / DARK / SPACE_* / RADIUS / FONT_*   (generated).
//   ChromePalette / CHROME_LIGHT / CHROME_DARK           (generated).
//   THEME_CSS        the DOM custom-property sheet, injected by the launcher.
//   Theme            Light | Dark, selects a Palette and the `data-theme` value.
//   ThemeControl     Send handle: the chrome toggles it, the canvas reads it.
//
// One source (`tokens.ron`) keeps the wgpu content and the DOM chrome coherent.
// ============================================================================

use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};

// Palette struct, LIGHT/DARK consts, and SPACE_*/RADIUS/FONT_* scales.
include!(concat!(env!("OUT_DIR"), "/theme_gen.rs"));

/// The generated CSS custom-property sheet (`:root` light + `[data-theme=dark]`).
/// The launcher injects this into a `<style>` so the DOM chrome reads the same
/// tokens the canvas draws with.
pub const THEME_CSS: &str = include_str!(concat!(env!("OUT_DIR"), "/theme.css"));

/// The active colour scheme. `Light` is the unmarked CSS `:root` default; the
/// app boots `Dark` (and sets `data-theme="dark"`).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Theme {
    #[default]
    Dark,
    Light,
}

impl Theme {
    /// The canvas colour palette for this theme.
    pub fn palette(self) -> &'static Palette {
        match self {
            Theme::Light => &LIGHT,
            Theme::Dark => &DARK,
        }
    }

    /// The chrome colours for this theme, for canvas content that shares
    /// their meaning: an accent, a status.
    pub fn chrome_palette(self) -> &'static ChromePalette {
        match self {
            Theme::Light => &CHROME_LIGHT,
            Theme::Dark => &CHROME_DARK,
        }
    }

    /// The `data-theme` attribute value for the document element.
    pub fn data_attr(self) -> &'static str {
        match self {
            Theme::Dark => "dark",
            Theme::Light => "light",
        }
    }

    /// The opposite theme.
    pub fn toggled(self) -> Theme {
        match self {
            Theme::Dark => Theme::Light,
            Theme::Light => Theme::Dark,
        }
    }

    fn from_u8(v: u8) -> Theme {
        if v == 1 { Theme::Light } else { Theme::Dark }
    }

    fn as_u8(self) -> u8 {
        match self {
            Theme::Dark => 0,
            Theme::Light => 1,
        }
    }
}

/// Shared, `Send` theme switch. The chrome toggle writes it (from a Leptos
/// callback, hence `Arc<AtomicU8>` like `RafControl`). The canvas view reads it
/// each frame and repaints on change. The single source of truth for the active
/// theme on the UI thread.
#[derive(Clone)]
pub struct ThemeControl {
    v: Arc<AtomicU8>,
}

impl ThemeControl {
    pub fn new(theme: Theme) -> Self {
        Self {
            v: Arc::new(AtomicU8::new(theme.as_u8())),
        }
    }

    pub fn get(&self) -> Theme {
        Theme::from_u8(self.v.load(Ordering::Relaxed))
    }

    pub fn set(&self, theme: Theme) {
        self.v.store(theme.as_u8(), Ordering::Relaxed);
    }

    /// Flip the theme and return the new value.
    pub fn toggle(&self) -> Theme {
        let next = self.get().toggled();
        self.set(next);
        next
    }
}

impl Default for ThemeControl {
    fn default() -> Self {
        Self::new(Theme::default())
    }
}

#[cfg(test)]
mod stylesheet_tests {
    use super::THEME_CSS;
    use std::collections::BTreeSet;

    /// The shell stylesheet the launcher links beside [`THEME_CSS`].
    const SHELL_CSS: &str = include_str!("../../ambre-app/styles.css");

    /// Every custom property `css` declares (`--name:`).
    fn declared(css: &str) -> BTreeSet<&str> {
        css.match_indices("--")
            .filter_map(|(at, _)| {
                let rest = &css[at..];
                let end =
                    rest.find(|c: char| !(c.is_ascii_alphanumeric() || c == '-' || c == '_'))?;
                rest[end..]
                    .trim_start()
                    .starts_with(':')
                    .then(|| &rest[..end])
            })
            .collect()
    }

    /// Every custom property `css` reads (`var(--name`).
    fn read(css: &str) -> BTreeSet<&str> {
        css.match_indices("var(")
            .filter_map(|(at, _)| {
                let rest = css[at + 4..].trim_start();
                let end =
                    rest.find(|c: char| !(c.is_ascii_alphanumeric() || c == '-' || c == '_'))?;
                rest.starts_with("--").then(|| &rest[..end])
            })
            .collect()
    }

    // `UI-THREAD-031`: the shell reads no token that the generated sheet does
    // not emit and the shell does not declare itself.
    #[test]
    fn the_stylesheet_reads_only_declared_tokens() {
        let known: BTreeSet<&str> = declared(THEME_CSS)
            .union(&declared(SHELL_CSS))
            .copied()
            .collect();
        let unknown: Vec<&str> = read(SHELL_CSS).difference(&known).copied().collect();
        assert!(unknown.is_empty(), "read but declared nowhere: {unknown:?}");
        assert!(known.contains("--text") && known.contains("--error"));
    }

    /// The CSS named colours, every one a literal the source never declared.
    const NAMED_COLOURS: &str = "aliceblue antiquewhite aqua aquamarine azure beige bisque black \
        blanchedalmond blue blueviolet brown burlywood cadetblue chartreuse chocolate coral \
        cornflowerblue cornsilk crimson cyan darkblue darkcyan darkgoldenrod darkgray darkgreen \
        darkgrey darkkhaki darkmagenta darkolivegreen darkorange darkorchid darkred darksalmon \
        darkseagreen darkslateblue darkslategray darkslategrey darkturquoise darkviolet deeppink \
        deepskyblue dimgray dimgrey dodgerblue firebrick floralwhite forestgreen fuchsia \
        gainsboro ghostwhite gold goldenrod gray green greenyellow grey honeydew hotpink \
        indianred indigo ivory khaki lavender lavenderblush lawngreen lemonchiffon lightblue \
        lightcoral lightcyan lightgoldenrodyellow lightgray lightgreen lightgrey lightpink \
        lightsalmon lightseagreen lightskyblue lightslategray lightslategrey lightsteelblue \
        lightyellow lime limegreen linen magenta maroon mediumaquamarine mediumblue \
        mediumorchid mediumpurple mediumseagreen mediumslateblue mediumspringgreen \
        mediumturquoise mediumvioletred midnightblue mintcream mistyrose moccasin navajowhite \
        navy oldlace olive olivedrab orange orangered orchid palegoldenrod palegreen \
        paleturquoise palevioletred papayawhip peachpuff peru pink plum powderblue purple \
        rebeccapurple red rosybrown royalblue saddlebrown salmon sandybrown seagreen seashell \
        sienna silver skyblue slateblue slategray slategrey snow springgreen steelblue tan teal \
        thistle tomato turquoise violet wheat white whitesmoke yellow yellowgreen";

    /// The colour functions, each of which spells a colour out in place.
    const COLOUR_FUNCTIONS: [&str; 9] = [
        "rgb(", "rgba(", "hsl(", "hsla(", "hwb(", "lab(", "lch(", "oklab(", "oklch(",
    ];

    /// `css` with its comments blanked and its line breaks kept. A comment
    /// that names a colour is not read as one, and a line number still holds.
    fn uncommented(css: &str) -> String {
        let mut out = String::with_capacity(css.len());
        let mut rest = css;
        while let Some(open) = rest.find("/*") {
            out.push_str(&rest[..open]);
            let close = rest[open..]
                .find("*/")
                .map_or(rest.len(), |at| open + at + 2);
            out.extend(
                rest[open..close]
                    .chars()
                    .map(|c| if c == '\n' { '\n' } else { ' ' }),
            );
            rest = &rest[close..];
        }
        out.push_str(rest);
        out
    }

    /// Every declaration of `css` as (line, property, value). A selector is
    /// the text before a `{`, so a segment ended by `;` or `}` is a
    /// declaration.
    fn declarations(css: &str) -> Vec<(usize, String, String)> {
        let mut found = Vec::new();
        let mut start = 0;
        for (at, c) in css.char_indices() {
            if !matches!(c, '{' | '}' | ';') {
                continue;
            }
            let segment = &css[start..at];
            if c != '{'
                && let Some((property, value)) = segment.split_once(':')
            {
                let line = css[..start + segment.len() - segment.trim_start().len()]
                    .matches('\n')
                    .count()
                    + 1;
                found.push((line, property.trim().to_owned(), value.trim().to_owned()));
            }
            start = at + 1;
        }
        found
    }

    /// Whether `value` spells a colour out rather than reading a token.
    fn names_a_colour(property: &str, value: &str) -> bool {
        let lower = value.to_ascii_lowercase();
        if lower.split('#').skip(1).any(|after| {
            let digits = after.bytes().take_while(u8::is_ascii_hexdigit).count();
            matches!(digits, 3 | 4 | 6 | 8)
        }) {
            return true;
        }
        if COLOUR_FUNCTIONS.iter().any(|f| lower.contains(f)) {
            return true;
        }
        // A named colour is a bare word, so read the words of a property that
        // takes a colour. A font family or an animation name is not one.
        const TAKES_A_COLOUR: [&str; 8] = [
            "color",
            "background",
            "border",
            "outline",
            "shadow",
            "fill",
            "stroke",
            "caret",
        ];
        TAKES_A_COLOUR.iter().any(|p| property.contains(p))
            && lower
                .split(|c: char| !(c.is_ascii_alphanumeric() || c == '-'))
                .any(|word| NAMED_COLOURS.split_whitespace().any(|name| name == word))
    }

    /// Whether `property` sets a spacing, a radius or a type size.
    fn takes_a_scale(property: &str) -> bool {
        [
            "padding",
            "margin",
            "gap",
            "row-gap",
            "column-gap",
            "font-size",
            "font",
        ]
        .iter()
        .any(|p| property == *p || property.starts_with(&format!("{p}-")))
            && property != "font-family"
            && property != "font-weight"
            && property != "font-style"
            && property != "font-variant-numeric"
            || property == "border-radius"
            || property.starts_with("border-") && property.ends_with("-radius")
    }

    /// Every length `value` spells out, other than zero and a hairline. A
    /// hairline of 1px is a border width, so a `-1px` nudge is one too.
    fn lengths(value: &str) -> Vec<&str> {
        value
            .split(|c: char| c.is_whitespace() || matches!(c, ',' | '(' | ')' | '/'))
            .filter(|word| {
                let number = word.trim_start_matches(['-', '+']);
                let digits = number
                    .find(|c: char| !(c.is_ascii_digit() || c == '.'))
                    .unwrap_or(number.len());
                let unit = &number[digits..];
                digits > 0
                    && matches!(unit, "px" | "rem" | "em" | "pt")
                    && !matches!(&number[..digits], "0" | "1")
            })
            .collect()
    }

    // `UI-THREAD-029`: every colour, spacing, radius and type size of the shell
    // is a token the source declares. A literal is reported with its line.
    #[test]
    fn the_shell_stylesheet_spells_out_no_colour_and_no_scale() {
        let mut literals = Vec::new();
        for (line, property, value) in declarations(&uncommented(SHELL_CSS)) {
            if names_a_colour(&property, &value) {
                literals.push(format!("{line}: {property}: {value}"));
            }
            let spelled = lengths(&value);
            if takes_a_scale(&property) && !spelled.is_empty() {
                literals.push(format!("{line}: {property}: {spelled:?}"));
            }
        }
        assert!(
            literals.is_empty(),
            "{} literals the token source never declared:\n{}",
            literals.len(),
            literals.join("\n")
        );
    }

    /// The check reads what it claims to. A literal of each kind is caught,
    /// and a token, a zero, a hairline and a comment are not.
    #[test]
    fn the_literal_check_catches_each_kind_and_spares_a_token() {
        let caught = |css: &str| {
            declarations(&uncommented(css))
                .into_iter()
                .filter(|(_, p, v)| {
                    names_a_colour(p, v) || takes_a_scale(p) && !lengths(v).is_empty()
                })
                .count()
        };
        for literal in [
            ".a { color: #fff; }",
            ".a { box-shadow: 0 1px 2px rgba(0, 0, 0, 0.5) }",
            ".a { border: 1px solid white; }",
            ".a { padding: 4px var(--space-2); }",
            ".a { margin-top: 1.5rem; }",
            ".a { border-top-left-radius: 3px; }",
            ".a { font: 14px Roboto, sans-serif; }",
        ] {
            assert_eq!(caught(literal), 1, "missed {literal}");
        }
        for token in [
            ".a:hover { color: var(--on-accent); }",
            ".a { margin: 0; padding: 0 var(--space-4); }",
            ".a { border: 1px solid var(--border); margin: -1px; }",
            "/* a white #fff */ .a { white-space: nowrap; font-family: Roboto; }",
            ".a { width: 320px; border-radius: 50%; }",
        ] {
            assert_eq!(caught(token), 0, "flagged {token}");
        }
    }

    /// The body of the first `selector { ... }` block of the generated sheet.
    fn block(selector: &str) -> &'static str {
        let open = THEME_CSS
            .find(&format!("{selector} {{"))
            .unwrap_or_else(|| panic!("no `{selector}` block"));
        let body = &THEME_CSS[open..];
        let start = body.find('{').unwrap_or(0) + 1;
        let end = body.find('}').unwrap_or(body.len());
        &body[start..end]
    }

    /// The value `body` declares for `name`, trimmed.
    fn value<'a>(body: &'a str, name: &str) -> Option<&'a str> {
        body.lines()
            .filter_map(|line| line.trim().strip_prefix(name)?.strip_prefix(':'))
            .map(|rest| rest.trim().trim_end_matches(';').trim())
            .next()
    }

    /// A palette colour as the sheet spells it.
    fn hex(color: super::Color) -> String {
        let [r, g, b, _] = color.to_rgba8().to_u8_array();
        format!("#{r:02x}{g:02x}{b:02x}")
    }

    /// `UI-THREAD-033`: the unmarked block carries the light colours, so a page
    /// that never sets the theme attribute renders light, not unstyled.
    ///
    /// The generator writes both blocks; nothing else notices the day the two
    /// are swapped.
    #[test]
    fn the_unmarked_block_carries_the_light_set() {
        let light = super::CHROME_LIGHT;
        let unmarked = block(":root");
        let expected = [
            ("--bg", light.bg),
            ("--surface", light.surface),
            ("--border", light.border),
            ("--text", light.text),
            ("--text-dim", light.text_dim),
            ("--accent", light.accent),
            ("--error", light.error),
        ];
        for (name, colour) in expected {
            assert_eq!(value(unmarked, name), Some(hex(colour).as_str()), "{name}");
        }
        let dark = block(":root[data-theme=\"dark\"]");
        assert_ne!(value(unmarked, "--bg"), value(dark, "--bg"));
    }

    /// `UI-THREAD-034`: a theme switch changes colours and effects, never a
    /// spacing, a radius or a type size.
    ///
    /// The dark block declares no scale, and nothing the unmarked block does
    /// not declare too, so every scale is the same under either theme.
    #[test]
    fn a_theme_switch_changes_no_scale() {
        let unmarked = declared(block(":root"));
        let dark = declared(block(":root[data-theme=\"dark\"]"));
        let scales: Vec<_> = dark
            .iter()
            .filter(|name| {
                ["--space", "--radius", "--font"]
                    .iter()
                    .any(|p| name.starts_with(p))
            })
            .collect();
        assert!(scales.is_empty(), "the dark block redeclares {scales:?}");
        assert!(
            dark.is_subset(&unmarked),
            "the dark block declares a token of its own"
        );
        assert!(
            unmarked.iter().any(|name| name.starts_with("--space")),
            "the unmarked block carries the scales"
        );
    }
}
