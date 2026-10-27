// crates/ambre-canvas/src/tokens.rs
// ============================================================================
// The design-token generator (`ambre-spec-design-tokens.md`). It reads
// `tokens.ron` and writes two committed files beside `src/theme.rs`:
//
//   * `src/theme_gen.rs` -- the `Palette` and `ChromePalette` types, their
//     LIGHT and DARK constants, and the scale constants;
//   * `src/theme.css` -- the custom properties, `:root` light and
//     `[data-theme="dark"]`.
//
// The files are committed so that every gate reads them, the primitive gate
// included. A build script must not write into `src/`, so the binary
// `ambre-tokens` writes them. The output is a pure function of the source
// text: a rerun is byte-identical. The module sits behind the `tokens`
// feature, so no client build links the RON parser.
// ============================================================================

use std::collections::BTreeSet;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;

/// The token source, relative to the crate.
pub const SOURCE: &str = "tokens.ron";
/// The generated Rust module, relative to the crate.
pub const RUST_OUT: &str = "src/theme_gen.rs";
/// The generated stylesheet, relative to the crate.
pub const CSS_OUT: &str = "src/theme.css";

#[derive(Deserialize)]
struct Tokens {
    chrome: ThemePair<ChromeColors>,
    effects: ThemePair<Effects>,
    photo: Photo,
    canvas: ThemePair<CanvasColors>,
    scale: Vec<(String, Px)>,
}

#[derive(Deserialize)]
struct ThemePair<T> {
    light: T,
    dark: T,
}

/// A length in CSS pixels, as the source writes it: a whole number. The
/// stylesheet writes it with `px`, and the Rust module as a `ViewPx`.
#[derive(Deserialize, Clone, Copy)]
#[serde(transparent)]
struct Px(u32);

impl fmt::Display for Px {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
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

/// A drop shadow: a vertical offset and a blur, and a colour.
#[derive(Deserialize)]
struct Shadow {
    y: Px,
    blur: Px,
    colour: String,
}

impl Shadow {
    /// The CSS `box-shadow` value, or the refusal of a malformed colour.
    fn css(&self) -> Result<String, String> {
        Ok(format!(
            "0 {}px {}px {}",
            self.y,
            self.blur,
            css_colour(&self.colour)?
        ))
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
    /// (css-var-name, css value) pairs in declaration order, or the refusal
    /// of a malformed colour.
    fn vars(&self) -> Result<[(&'static str, String); 4], String> {
        Ok([
            ("shadow-low", self.shadow_low.css()?),
            ("shadow-mid", self.shadow_mid.css()?),
            ("shadow-high", self.shadow_high.css()?),
            ("scrim", css_colour(&self.scrim)?.to_owned()),
        ])
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
    /// A cell border the user did not colour.
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

/// The Rust expression of the opaque colour `#rrggbb`, a
/// `Color::from_rgba8` call. A token typo fails the generator, so a wrong
/// colour never reaches the committed files.
fn rgba8(hex: &str) -> Result<String, String> {
    let digits = hex
        .strip_prefix('#')
        .filter(|h| h.len() == 6 && h.bytes().all(|b| b.is_ascii_hexdigit()))
        .ok_or_else(|| format!("token colour must be #rrggbb: {hex}"))?;
    let byte = |at: usize| {
        digits
            .get(at..at + 2)
            .and_then(|pair| u8::from_str_radix(pair, 16).ok())
            .ok_or_else(|| format!("bad hex in token colour: {hex}"))
    };
    Ok(format!(
        "Color::from_rgba8({}, {}, {}, 255)",
        byte(0)?,
        byte(2)?,
        byte(4)?
    ))
}

/// Check a CSS-only colour, `#rrggbb` or `#rrggbbaa`, and hand it back.
fn css_colour(hex: &str) -> Result<&str, String> {
    hex.strip_prefix('#')
        .filter(|h| (h.len() == 6 || h.len() == 8) && h.bytes().all(|b| b.is_ascii_hexdigit()))
        .map(|_| hex)
        .ok_or_else(|| format!("token colour must be #rrggbb or #rrggbbaa: {hex}"))
}

/// Check the scale names: each one snake_case, none twice.
fn check_scale(scale: &[(String, Px)]) -> Result<(), String> {
    let mut seen = BTreeSet::new();
    for (name, _) in scale {
        let snake = !name.is_empty()
            && name
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_');
        if !snake {
            return Err(format!("scale name must be snake_case: {name}"));
        }
        if !seen.insert(name) {
            return Err(format!("scale name declared twice: {name}"));
        }
    }
    Ok(())
}

/// The two generated files, as text.
pub struct Generated {
    rust: String,
    css: String,
}

/// Both files for the token source `source`, or the refusal of a malformed
/// token. The output depends on `source` alone.
///
/// # Errors
///
/// The source does not parse, a colour is not hex, or a scale name is not
/// snake_case or is declared twice. The message names the token.
pub fn generate(source: &str) -> Result<Generated, String> {
    let tokens: Tokens = ron::from_str(source).map_err(|e| format!("parse {SOURCE}: {e}"))?;
    check_scale(&tokens.scale)?;
    Ok(Generated {
        rust: rust_module(&tokens)?,
        css: stylesheet(&tokens)?,
    })
}

/// The Rust module: `Palette`, `ChromePalette`, their constants, the scales.
fn rust_module(tokens: &Tokens) -> Result<String, String> {
    let mut rs = String::new();
    rs.push_str(
        "// @generated by `just tokens-write` from `crates/ambre-canvas/tokens.ron`, through\n\
         // `crates/ambre-canvas/src/tokens.rs`. Do not edit: `just tokens-check` fails\n\
         // when this file is not what the generator writes.\n",
    );
    rs.push_str("use ambre_draw::Color;\n\n");
    rs.push_str("/// Canvas content colours for one theme. Generated from `tokens.ron`.\n");
    rs.push_str("pub struct Palette {\n");
    for (name, _) in tokens.canvas.light.fields() {
        rs.push_str(&format!("    pub {name}: Color,\n"));
    }
    rs.push_str("}\n\n");
    for (name, colors) in [
        ("LIGHT", &tokens.canvas.light),
        ("DARK", &tokens.canvas.dark),
    ] {
        rs.push_str(&format!("pub const {name}: Palette = Palette {{\n"));
        for (field, hex) in colors.fields() {
            rs.push_str(&format!("    {field}: {},\n", rgba8(hex)?));
        }
        rs.push_str("};\n\n");
    }

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
    for (name, colors) in [
        ("CHROME_LIGHT", &tokens.chrome.light),
        ("CHROME_DARK", &tokens.chrome.dark),
    ] {
        rs.push_str(&format!(
            "pub const {name}: ChromePalette = ChromePalette {{\n"
        ));
        for (var, hex) in colors.vars() {
            let field = var.replace('-', "_");
            rs.push_str(&format!("    {field}: {},\n", rgba8(hex)?));
        }
        rs.push_str("};\n\n");
    }

    for (name, value) in &tokens.scale {
        let rust_name = name.to_ascii_uppercase();
        // A scale step is a length on the canvas's view (`CANVAS-032`): a
        // spacing, a radius or a font size the canvas draws at.
        rs.push_str(&format!(
            "pub const {rust_name}: ambre_core::ViewPx = ambre_core::ViewPx::new({value}.0);\n"
        ));
    }
    Ok(rs)
}

/// The stylesheet: `:root` carries the light set, `[data-theme="dark"]` the
/// dark one. A scale is declared once, in `:root`.
fn stylesheet(tokens: &Tokens) -> Result<String, String> {
    let mut css = String::new();
    css.push_str(
        "/* @generated by `just tokens-write` from crates/ambre-canvas/tokens.ron -- do not edit. */\n",
    );
    css.push_str(":root {\n");
    for (var, hex) in tokens.chrome.light.vars() {
        css.push_str(&format!("    --{var}: {hex};\n"));
    }
    for (var, value) in tokens.effects.light.vars()? {
        css.push_str(&format!("    --{var}: {value};\n"));
    }
    for (var, hex) in tokens.photo.vars() {
        css.push_str(&format!("    --{var}: {};\n", css_colour(hex)?));
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
    for (var, value) in tokens.effects.dark.vars()? {
        css.push_str(&format!("    --{var}: {value};\n"));
    }
    css.push_str("}\n");
    Ok(css)
}

/// A file of the crate, by its path relative to the crate.
fn crate_file(relative: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(relative)
}

/// Both files for the token source on disk.
///
/// # Errors
///
/// The source cannot be read, or [`generate`] refuses it.
pub fn generate_from_disk() -> Result<Generated, String> {
    fs::read_to_string(crate_file(SOURCE))
        .map_err(|e| format!("read {SOURCE}: {e}"))
        .and_then(|source| generate(&source))
}

/// The pairs of a path relative to the crate and the text it must hold.
fn outputs(generated: &Generated) -> [(&'static str, &String); 2] {
    [(RUST_OUT, &generated.rust), (CSS_OUT, &generated.css)]
}

/// Each generated file whose text on disk is not `generated`, by its path
/// relative to the crate. A file that cannot be read is stale.
pub fn stale(generated: &Generated) -> Vec<&'static str> {
    outputs(generated)
        .into_iter()
        .filter(|(path, text)| fs::read_to_string(crate_file(path)).ok().as_ref() != Some(*text))
        .map(|(path, _)| path)
        .collect()
}

/// Rewrite both files with `generated`.
///
/// # Errors
///
/// A file cannot be written. The message names it.
pub fn write(generated: &Generated) -> Result<(), String> {
    outputs(generated).into_iter().try_for_each(|(path, text)| {
        fs::write(crate_file(path), text).map_err(|e| format!("write {path}: {e}"))
    })
}

#[cfg(test)]
mod tests {
    use super::{CSS_OUT, RUST_OUT, SOURCE, crate_file, generate, stale};

    fn source() -> String {
        std::fs::read_to_string(crate_file(SOURCE)).unwrap()
    }

    // `UI-THREAD-030`: a rerun writes the same bytes, so the check compares
    // the source with the files and never with the time of the run.
    #[test]
    fn a_rerun_writes_the_same_bytes() {
        let first = generate(&source()).unwrap();
        let second = generate(&source()).unwrap();
        assert_eq!(first.rust, second.rust);
        assert_eq!(first.css, second.css);
    }

    // `UI-THREAD-030`: the committed files are what the source generates, and
    // a token changed without a rewrite makes both of them stale.
    #[test]
    fn a_token_changed_without_a_rewrite_makes_both_files_stale() {
        assert!(stale(&generate(&source()).unwrap()).is_empty());
        let changed = source().replace("\"#f2a93d\"", "\"#f2a93e\"");
        assert_ne!(changed, source());
        assert_eq!(stale(&generate(&changed).unwrap()), [RUST_OUT, CSS_OUT]);
    }

    #[test]
    fn a_malformed_colour_is_refused() {
        let broken = source().replace("\"#f2a93d\"", "\"#f2a93\"");
        assert!(generate(&broken).is_err());
    }
}
