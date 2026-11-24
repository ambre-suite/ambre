// ambre-canvas/src/theme.rs
// ============================================================================
// Design tokens, consumed (ambre-spec-design-tokens.md, ambre-spec-frame.md).
// `just tokens-write` generates the colour palettes and scales from
// `tokens.ron` into the committed `theme_gen.rs` and `theme.css`. This module
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
pub use crate::theme_gen::*;

/// The generated CSS custom-property sheet (`:root` light + `[data-theme=dark]`).
/// The launcher injects this into a `<style>` so the DOM chrome reads the same
/// tokens the canvas draws with.
pub const THEME_CSS: &str = include_str!("theme.css");

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
    fn hex(color: ambre_draw::Color) -> String {
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

    /// Every rule of `css` as (selector list, body), comments blanked. A rule
    /// nested in an at-rule keeps its own selector.
    fn rules(css: &str) -> Vec<(String, String)> {
        let css = uncommented(css);
        let mut found = Vec::new();
        let mut rest = css.as_str();
        while let Some(open) = rest.find('{') {
            let head = &rest[..open];
            let selector = head.rsplit(['}', '{', ';']).next().unwrap_or(head).trim();
            let Some(close) = rest[open + 1..].find(['{', '}']) else {
                break;
            };
            let body = &rest[open + 1..open + 1 + close];
            if !rest[open + 1 + close..].starts_with('{') {
                found.push((selector.to_owned(), body.to_owned()));
            }
            rest = &rest[open + 1..];
        }
        found
    }

    /// The value `body` gives `property`, if it sets it.
    fn value_of<'a>(body: &'a str, property: &str) -> Option<&'a str> {
        body.split(';').find_map(|declaration| {
            let (name, value) = declaration.split_once(':')?;
            (name.trim() == property).then(|| value.trim())
        })
    }

    /// `CHROME-201`: every family the chrome names is the chrome family, or a
    /// monospace one, and the body carries the chrome family. A toast or a
    /// dialog mounted outside the shell then never falls back to the serif
    /// face of the browser.
    #[test]
    fn every_family_the_chrome_names_is_the_chrome_family_or_a_monospace_one() {
        let body = rules(SHELL_CSS)
            .into_iter()
            .find(|(selector, _)| selector == "body")
            .map(|(_, body)| body)
            .expect("a body rule");
        assert_eq!(value_of(&body, "font-family"), Some("var(--font-sans)"));
        for (line, property, value) in declarations(&uncommented(SHELL_CSS)) {
            if property != "font-family" && property != "font" {
                continue;
            }
            assert!(
                ["var(--font-sans)", "inherit", "monospace"]
                    .iter()
                    .any(|allowed| value.contains(allowed)),
                "line {line}: `{property}: {value}` names a family the page may not load"
            );
        }
    }

    /// `CHROME-200`: a fixed surface that sets its top sits under the header,
    /// or is centred on the viewport. A call ring is an alert that covers the
    /// header on purpose, for the seconds it rings.
    #[test]
    fn no_fixed_surface_covers_the_header() {
        const ALERTS: [&str; 1] = [".murmure-ring"];
        for (selector, body) in rules(SHELL_CSS) {
            if value_of(&body, "position") != Some("fixed") || ALERTS.contains(&selector.as_str()) {
                continue;
            }
            if let Some(top) = value_of(&body, "top") {
                assert!(
                    top.contains("var(--header-h)") || top.ends_with('%') || top == "0",
                    "`{selector}` is fixed at `top: {top}`, over the header"
                );
            }
        }
        let toile = rules(SHELL_CSS)
            .into_iter()
            .filter(|(selector, _)| selector.contains("ambre-toile-menubar"))
            .collect::<Vec<_>>();
        // The command bar hosts a floating menu: a rule that positions what
        // carries a name must leave that menu fixed.
        for (selector, body) in rules(SHELL_CSS) {
            if selector.contains(".ambre-commandbar")
                && selector.contains("[aria-label]")
                && value_of(&body, "position").is_some()
            {
                // `:not()` may list other exclusions after the menu.
                assert!(
                    selector.contains(":not(.ambre-ctx-menu *)")
                        || selector.contains(":not(.ambre-ctx-menu *,"),
                    "`{selector}` positions the floating menu of the bar"
                );
            }
        }
        assert!(!toile.is_empty(), "the Toile menu bar keeps a rule");
        assert!(
            toile
                .iter()
                .all(|(_, body)| value_of(body, "position").is_none()),
            "the Toile menu bar leaves the command bar"
        );
    }

    /// The rules of the control primitives, each with the box it sizes.
    const CONTROL_BOXES: [&str; 6] = [
        ".ambre-icon-button",
        ".ambre-select-trigger",
        ".ambre-select-option",
        ".ambre-stepper",
        ".ambre-kit-command",
        ".ambre-segments > button",
    ];

    /// `UI-THREAD-068`: every control primitive takes its height from the
    /// control scale, and every glyph inside one takes the icon scale. A
    /// height spelled out in pixels, or a missing one, fails with its rule.
    #[test]
    fn every_control_reads_the_control_scale_and_every_glyph_the_icon_scale() {
        let all = rules(SHELL_CSS);
        for wanted in CONTROL_BOXES {
            let body = all
                .iter()
                .find(|(selector, _)| selector == wanted)
                .map(|(_, body)| body.as_str())
                .unwrap_or_else(|| panic!("a `{wanted}` rule"));
            let height = value_of(body, "height").or_else(|| value_of(body, "min-height"));
            assert!(
                height.is_some_and(|h| h.starts_with("var(--control")),
                "`{wanted}` sets its height from the control scale, not `{height:?}`"
            );
        }
        for (selector, body) in &all {
            let sizes_a_glyph = selector.ends_with("svg")
                && CONTROL_BOXES
                    .iter()
                    .chain(&[".ambre-ctx-section-body"])
                    .any(|c| selector.contains(c));
            if !sizes_a_glyph {
                continue;
            }
            for property in ["width", "height"] {
                if let Some(v) = value_of(body, property) {
                    assert!(
                        v.starts_with("var(--icon"),
                        "`{selector}` sizes a glyph at `{property}: {v}`"
                    );
                }
            }
        }
        let tokens: BTreeSet<&str> = declared(THEME_CSS);
        for name in [
            "--control-sm",
            "--control",
            "--control-lg",
            "--icon",
            "--focus-width",
        ] {
            assert!(tokens.contains(name), "the source emits {name}");
        }
    }

    /// The ruler of a Prose page draws its markers from the tokens. A marker
    /// reads in the dimmed text, and in the accent under the pointer and in a
    /// drag. Its triangle is painted in that colour, on a track in the strong
    /// border. No glyph of a font draws it.
    #[test]
    fn the_ruler_markers_read_the_tokens_with_a_hover_and_a_drag_state() {
        let all = rules(SHELL_CSS);
        let body_of = |wanted: &str| {
            all.iter()
                .find(|(selector, _)| selector == wanted)
                .map(|(_, body)| body.clone())
                .unwrap_or_else(|| panic!("a `{wanted}` rule"))
        };
        let handle = ".ambre-prose-ruler-handle";
        assert_eq!(value_of(&body_of(handle), "color"), Some("var(--text-dim)"));
        for state in [":hover", "[data-dragging=\"true\"]"] {
            assert_eq!(
                value_of(&body_of(&format!("{handle}{state}")), "color"),
                Some("var(--accent)"),
                "{state}"
            );
        }
        let marker = body_of(&format!("{handle}::before"));
        assert_eq!(value_of(&marker, "background"), Some("currentColor"));
        assert!(value_of(&marker, "clip-path").is_some(), "a drawn triangle");
        let track = body_of(".ambre-prose-ruler");
        assert!(
            value_of(&track, "background").is_some_and(|v| v.contains("var(--border-strong)")),
            "the track reads the strong border"
        );
    }

    /// `CHROME-202`: the format bar carries the frame of a floating bar. Each
    /// control carries the look of a chrome button, not the raw button of the
    /// browser.
    #[test]
    fn the_format_bar_has_the_frame_of_a_floating_bar() {
        let all = rules(SHELL_CSS);
        let body_of = |wanted: &str| {
            all.iter()
                .find(|(selector, _)| selector == wanted)
                .map(|(_, body)| body.clone())
                .unwrap_or_else(|| panic!("a `{wanted}` rule"))
        };
        let bar = body_of(".ambre-format-bar");
        for property in ["background", "border", "border-radius", "box-shadow"] {
            assert!(
                value_of(&bar, property).is_some(),
                "the bar sets {property}"
            );
        }
        let control = body_of(".ambre-format-control");
        assert_eq!(value_of(&control, "appearance"), Some("none"));
        assert!(
            value_of(
                &body_of(".ambre-format-control[aria-pressed=\"true\"]"),
                "background"
            )
            .is_some()
        );
    }
}

#[cfg(test)]
mod canvas_palette_tests {
    use super::Theme;
    use ambre_draw::Color;

    /// The relative luminance of an opaque colour (WCAG 2.2).
    fn luminance(colour: Color) -> f64 {
        let [r, g, b, _] = colour.to_rgba8().to_u8_array();
        let linear = |c: u8| {
            let c = f64::from(c) / 255.0;
            if c <= 0.040_45 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * linear(r) + 0.7152 * linear(g) + 0.0722 * linear(b)
    }

    /// The contrast ratio of two opaque colours (WCAG 2.2).
    fn contrast(a: Color, b: Color) -> f64 {
        let (la, lb) = (luminance(a), luminance(b));
        (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
    }

    /// `UI-THREAD-069`: the grounds the canvas draws are the warm surfaces of
    /// the chrome, in both themes. A grid lies on the surface. Its headers,
    /// and the ground around a page or under a board, lie on the sunken one.
    /// A grid line is the chrome's border. Every text drawn on one of those
    /// grounds passes the AA ratio for body text.
    #[test]
    fn the_canvas_grounds_are_the_warm_chrome_surfaces_and_their_text_passes_aa() {
        for theme in [Theme::Light, Theme::Dark] {
            let canvas = theme.palette();
            let chrome = theme.chrome_palette();
            assert_eq!(canvas.content_bg, chrome.surface, "{theme:?}: grid ground");
            assert_eq!(canvas.header_bg, chrome.surface_2, "{theme:?}: header");
            assert_eq!(canvas.ground, chrome.surface_2, "{theme:?}: ground");
            assert_eq!(
                canvas.header_hover, chrome.surface_hover,
                "{theme:?}: hover"
            );
            assert_eq!(
                canvas.header_active, chrome.accent_soft,
                "{theme:?}: active"
            );
            assert_eq!(canvas.grid_line, chrome.border, "{theme:?}: grid line");
            let pairs = [
                ("cell text", canvas.cell_text, canvas.content_bg),
                ("header text", canvas.header_text, canvas.header_bg),
                ("hovered header", canvas.header_text, canvas.header_hover),
                ("active header", canvas.header_text, canvas.header_active),
            ];
            for (what, ink, ground) in pairs {
                let ratio = contrast(ink, ground);
                assert!(ratio >= 4.5, "{theme:?}: {what} at {ratio:.2}:1");
            }
        }
    }
}
