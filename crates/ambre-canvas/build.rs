// ambre-canvas/build.rs
// ============================================================================
// Token codegen (ambre-spec-design-tokens.md). Reads `tokens.ron` and emits, into
// OUT_DIR:
//   - theme_gen.rs : the canvas `Palette` struct + LIGHT/DARK consts + scales.
//   - theme.css    : CSS custom properties (`:root` light, `[data-theme=dark]`).
// src/theme.rs `include!`s the first and `include_str!`s the second. One source
// keeps the DOM chrome and the wgpu content in lockstep by construction.
// ============================================================================

#![allow(
    clippy::panic,
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::indexing_slicing,
    reason = "a build script reports a failure by panicking"
)]

use std::collections::BTreeSet;
use std::env;
use std::fs;
use std::path::Path;

use serde::Deserialize;

#[derive(Deserialize)]
struct Tokens {
    chrome: ThemePair<ChromeColors>,
    effects: ThemePair<Effects>,
    photo: Photo,
    canvas: ThemePair<CanvasColors>,
    scale: Vec<(String, u32)>,
}

#[derive(Deserialize)]
struct ThemePair<T> {
    light: T,
    dark: T,
}

#[derive(Deserialize)]
struct ChromeColors {
    bg: String,
    surface: String,
    surface_2: String,
    surface_hover: String,
    border: String,
    text: String,
    text_dim: String,
    accent: String,
    success: String,
    warning: String,
    error: String,
    info: String,
    /// Text and icons drawn on an accent ground.
    on_accent: String,
}

impl ChromeColors {
    /// (css-var-name, hex) pairs in declaration order.
    fn vars(&self) -> [(&'static str, &str); 13] {
        [
            ("bg", &self.bg),
            ("surface", &self.surface),
            ("surface-2", &self.surface_2),
            ("surface-hover", &self.surface_hover),
            ("border", &self.border),
            ("text", &self.text),
            ("text-dim", &self.text_dim),
            ("accent", &self.accent),
            ("success", &self.success),
            ("warning", &self.warning),
            ("error", &self.error),
            ("info", &self.info),
            ("on-accent", &self.on_accent),
        ]
    }
}

/// A drop shadow: a vertical offset and a blur in pixels, and a colour.
#[derive(Deserialize)]
struct Shadow {
    y: u32,
    blur: u32,
    colour: String,
}

impl Shadow {
    fn css(&self) -> String {
        format!(
            "0 {}px {}px {}",
            self.y,
            self.blur,
            css_colour(&self.colour)
        )
    }
}

/// The chrome's shadows and the scrim behind a modal, for one theme.
#[derive(Deserialize)]
struct Effects {
    shadow_low: Shadow,
    shadow_mid: Shadow,
    shadow_high: Shadow,
    scrim: String,
}

impl Effects {
    /// (css-var-name, css value) pairs in declaration order.
    fn vars(&self) -> [(&'static str, String); 4] {
        [
            ("shadow-low", self.shadow_low.css()),
            ("shadow-mid", self.shadow_mid.css()),
            ("shadow-high", self.shadow_high.css()),
            ("scrim", css_colour(&self.scrim).to_owned()),
        ]
    }
}

/// Colours drawn over a picture rather than over a theme ground, so one set
/// serves both themes.
#[derive(Deserialize)]
struct Photo {
    text: String,
    text_shadow: String,
    line: String,
    control_line: String,
    control_ground: String,
    veil: String,
    veil_deep: String,
}

impl Photo {
    /// (css-var-name, hex) pairs in declaration order.
    fn vars(&self) -> [(&'static str, &str); 7] {
        [
            ("photo-text", &self.text),
            ("photo-text-shadow", &self.text_shadow),
            ("photo-line", &self.line),
            ("photo-control-line", &self.control_line),
            ("photo-control-ground", &self.control_ground),
            ("photo-veil", &self.veil),
            ("photo-veil-deep", &self.veil_deep),
        ]
    }
}

#[derive(Deserialize)]
struct CanvasColors {
    content_bg: String,
    content_pending: String,
    header_bg: String,
    header_active: String,
    header_hover: String,
    grid_line: String,
    header_border: String,
    selection: String,
    cell_text: String,
    header_text: String,
    caret: String,
    cell_error: String,
    /// A cell border the user did not colour (`TRAME-101`).
    cell_border: String,
}

impl CanvasColors {
    /// (rust-field-name, hex) pairs in `Palette` field order.
    fn fields(&self) -> [(&'static str, &str); 13] {
        [
            ("content_bg", &self.content_bg),
            ("content_pending", &self.content_pending),
            ("header_bg", &self.header_bg),
            ("header_active", &self.header_active),
            ("header_hover", &self.header_hover),
            ("grid_line", &self.grid_line),
            ("header_border", &self.header_border),
            ("selection", &self.selection),
            ("cell_text", &self.cell_text),
            ("header_text", &self.header_text),
            ("caret", &self.caret),
            ("cell_error", &self.cell_error),
            ("cell_border", &self.cell_border),
        ]
    }
}

/// Parse `#rrggbb` into `(r, g, b)`. Panics on malformed input -- a token typo
/// must fail the build, not ship a wrong colour.
fn hex_rgb(hex: &str) -> (u8, u8, u8) {
    let h = hex
        .strip_prefix('#')
        .unwrap_or_else(|| panic!("token colour must start with '#': {hex}"));
    assert_eq!(h.len(), 6, "token colour must be #rrggbb: {hex}");
    let byte = |i: usize| {
        u8::from_str_radix(&h[i..i + 2], 16)
            .unwrap_or_else(|_| panic!("bad hex in token colour: {hex}"))
    };
    (byte(0), byte(2), byte(4))
}

/// Check a CSS-only colour, `#rrggbb` or `#rrggbbaa`, and hand it back.
fn css_colour(hex: &str) -> &str {
    let h = hex
        .strip_prefix('#')
        .unwrap_or_else(|| panic!("token colour must start with '#': {hex}"));
    assert!(
        (h.len() == 6 || h.len() == 8) && h.bytes().all(|b| b.is_ascii_hexdigit()),
        "token colour must be #rrggbb or #rrggbbaa: {hex}"
    );
    hex
}

/// Check the scale names: each one snake_case, none twice.
fn check_scale(scale: &[(String, u32)]) {
    let mut seen = BTreeSet::new();
    for (name, _) in scale {
        assert!(
            !name.is_empty()
                && name
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_'),
            "scale name must be snake_case: {name}"
        );
        assert!(seen.insert(name), "scale name declared twice: {name}");
    }
}

fn main() {
    println!("cargo:rerun-if-changed=tokens.ron");

    let src = fs::read_to_string("tokens.ron").expect("read tokens.ron");
    let tokens: Tokens = ron::from_str(&src).expect("parse tokens.ron");
    check_scale(&tokens.scale);

    let out_dir = env::var("OUT_DIR").expect("OUT_DIR");

    // -- theme_gen.rs : Palette struct + LIGHT/DARK + scale consts -----------
    let mut rs = String::new();
    rs.push_str("// @generated by build.rs from tokens.ron -- do not edit.\n");
    rs.push_str("use ambre_draw::Color;\n\n");
    rs.push_str("/// Canvas content colours for one theme. Generated from `tokens.ron`.\n");
    rs.push_str("pub struct Palette {\n");
    for (name, _) in tokens.canvas.light.fields() {
        rs.push_str(&format!("    pub {name}: Color,\n"));
    }
    rs.push_str("}\n\n");

    let emit_palette = |rs: &mut String, name: &str, colors: &CanvasColors| {
        rs.push_str(&format!("pub const {name}: Palette = Palette {{\n"));
        for (field, hex) in colors.fields() {
            let (r, g, b) = hex_rgb(hex);
            rs.push_str(&format!(
                "    {field}: Color::from_rgba8({r}, {g}, {b}, 255),\n"
            ));
        }
        rs.push_str("};\n\n");
    };
    emit_palette(&mut rs, "LIGHT", &tokens.canvas.light);
    emit_palette(&mut rs, "DARK", &tokens.canvas.dark);

    // The chrome colours too, for the canvas content that shares their
    // meaning. An accent or a status drawn on a page reads the same token as
    // the DOM control beside it.
    rs.push_str("/// Chrome colours for one theme, readable by the canvas. Generated from\n");
    rs.push_str("/// `tokens.ron`.\n");
    rs.push_str("pub struct ChromePalette {\n");
    for (name, _) in tokens.chrome.light.vars() {
        let field = name.replace('-', "_");
        rs.push_str(&format!("    pub {field}: Color,\n"));
    }
    rs.push_str("}\n\n");
    let emit_chrome = |rs: &mut String, name: &str, colors: &ChromeColors| {
        rs.push_str(&format!(
            "pub const {name}: ChromePalette = ChromePalette {{\n"
        ));
        for (var, hex) in colors.vars() {
            let field = var.replace('-', "_");
            let (r, g, b) = hex_rgb(hex);
            rs.push_str(&format!(
                "    {field}: Color::from_rgba8({r}, {g}, {b}, 255),\n"
            ));
        }
        rs.push_str("};\n\n");
    };
    emit_chrome(&mut rs, "CHROME_LIGHT", &tokens.chrome.light);
    emit_chrome(&mut rs, "CHROME_DARK", &tokens.chrome.dark);

    for (name, value) in &tokens.scale {
        let rust_name = name.to_ascii_uppercase();
        rs.push_str(&format!("pub const {rust_name}: f32 = {value}.0;\n"));
    }

    fs::write(Path::new(&out_dir).join("theme_gen.rs"), rs).expect("write theme_gen.rs");

    // -- theme.css : :root (light) + [data-theme=dark] -----------------------
    let mut css = String::new();
    css.push_str("/* @generated by build.rs from tokens.ron -- do not edit. */\n");
    css.push_str(":root {\n");
    for (var, hex) in tokens.chrome.light.vars() {
        css.push_str(&format!("    --{var}: {hex};\n"));
    }
    for (var, value) in tokens.effects.light.vars() {
        css.push_str(&format!("    --{var}: {value};\n"));
    }
    for (var, hex) in tokens.photo.vars() {
        css.push_str(&format!("    --{var}: {};\n", css_colour(hex)));
    }
    for (name, value) in &tokens.scale {
        let var = name.replace('_', "-");
        css.push_str(&format!("    --{var}: {value}px;\n"));
    }
    css.push_str("}\n\n");
    css.push_str(":root[data-theme=\"dark\"] {\n");
    for (var, hex) in tokens.chrome.dark.vars() {
        css.push_str(&format!("    --{var}: {hex};\n"));
    }
    for (var, value) in tokens.effects.dark.vars() {
        css.push_str(&format!("    --{var}: {value};\n"));
    }
    css.push_str("}\n");

    fs::write(Path::new(&out_dir).join("theme.css"), css).expect("write theme.css");
}
