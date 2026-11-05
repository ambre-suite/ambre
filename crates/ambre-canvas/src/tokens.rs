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
use std::io;
use std::path::{Path, PathBuf};

use ambre_core::{Px, Rule, Validated, View};
use ambre_draw::Color;
use peniko::color::Rgba8;
use serde::{Deserialize, Deserializer};

/// A file of the generator, by its role. The set is closed: the source and
/// the two files it generates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TokenFile {
    /// The token source, `tokens.ron`.
    Source,
    /// The generated Rust module, `src/theme_gen.rs`.
    Rust,
    /// The generated stylesheet, `src/theme.css`.
    Css,
}

impl TokenFile {
    /// The file on disk, in the directory of the crate.
    pub fn path(self) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join(self.to_string())
    }
}

/// The path of the file, relative to the crate.
impl fmt::Display for TokenFile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Source => "tokens.ron",
            Self::Rust => "src/theme_gen.rs",
            Self::Css => "src/theme.css",
        })
    }
}

/// Why a run of the generator on disk failed.
#[derive(Debug)]
pub enum TokenError {
    /// The source cannot be read.
    Read { file: TokenFile, error: io::Error },
    /// A generated file cannot be written.
    Write { file: TokenFile, error: io::Error },
    /// The source does not parse, or a token is refused. The message
    /// echoes the token.
    Refused(String),
}

impl fmt::Display for TokenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Read { file, error } => write!(f, "read {file}: {error}"),
            Self::Write { file, error } => write!(f, "write {file}: {error}"),
            Self::Refused(message) => f.write_str(message),
        }
    }
}

#[derive(Deserialize)]
struct Tokens {
    chrome: ThemePair<ChromeColors>,
    effects: ThemePair<Effects>,
    photo: Photo,
    canvas: ThemePair<CanvasColors>,
    #[serde(deserialize_with = "whole_px_scale")]
    scale: ScaleSteps,
}

/// The scale of the source: each step, its name and its length, in source
/// order. An alias, because the type written out fails
/// `clippy::type_complexity` in a return (`ARCH-067`).
type ScaleSteps = Vec<(Validated<ScaleName>, Px<View>)>;

#[derive(Deserialize)]
struct ThemePair<T> {
    light: T,
    dark: T,
}

/// A length of the source in CSS pixels, a length on the canvas's view
/// (`CANVAS-032`). The source writes a whole number up to `u16::MAX`, which
/// the `f32` of a `Px<View>` holds exactly. A fractional, a negative or a
/// larger value is refused here. So the length prints back the digits the
/// source wrote. The stylesheet writes it with `px`, the Rust module as a
/// `Px<View>`.
fn whole_px<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Px<View>, D::Error> {
    u16::deserialize(deserializer).map(|pixels| Px::new(f32::from(pixels)))
}

/// The scale of the source, each length read as [`whole_px`] reads one.
fn whole_px_scale<'de, D: Deserializer<'de>>(deserializer: D) -> Result<ScaleSteps, D::Error> {
    Vec::<(Validated<ScaleName>, u16)>::deserialize(deserializer).map(|steps| {
        steps
            .into_iter()
            .map(|(name, pixels)| (name, Px::new(f32::from(pixels))))
            .collect()
    })
}

/// A colour of the source, `#rrggbb` or `#rrggbbaa`. The source text is
/// refused at deserialization when it is neither, so a token typo never
/// reaches the generated files.
#[derive(Deserialize, Clone, Copy)]
#[serde(try_from = "String")]
struct TokenColour(Rgba8);

impl TryFrom<String> for TokenColour {
    type Error = String;

    fn try_from(text: String) -> Result<Self, Self::Error> {
        let refused = || format!("token colour must be #rrggbb or #rrggbbaa: {text}");
        let digits = text
            .strip_prefix('#')
            .filter(|h| (h.len() == 6 || h.len() == 8) && h.bytes().all(|b| b.is_ascii_hexdigit()))
            .ok_or_else(refused)?;
        let byte = |at: usize| {
            digits
                .get(at..at + 2)
                .and_then(|pair| u8::from_str_radix(pair, 16).ok())
        };
        let channel = |at: usize| byte(at).ok_or_else(refused);
        Ok(Self(Rgba8 {
            r: channel(0)?,
            g: channel(2)?,
            b: channel(4)?,
            a: byte(6).unwrap_or(u8::MAX),
        }))
    }
}

/// `#rrggbb` for an opaque colour, `#rrggbbaa` otherwise.
impl fmt::Display for TokenColour {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let Rgba8 { r, g, b, a } = self.0;
        write!(f, "#{r:02x}{g:02x}{b:02x}")?;
        if a != u8::MAX {
            write!(f, "{a:02x}")?;
        }
        Ok(())
    }
}

/// A colour of the source with no transparency: the canvas and the chrome
/// colours, which the Rust module carries. A colour with an alpha below
/// full is refused at deserialization.
#[derive(Deserialize, Clone, Copy)]
#[serde(try_from = "TokenColour")]
struct OpaqueColour(Rgba8);

impl TryFrom<TokenColour> for OpaqueColour {
    type Error = String;

    fn try_from(colour: TokenColour) -> Result<Self, Self::Error> {
        if colour.0.a == u8::MAX {
            Ok(Self(colour.0))
        } else {
            Err(format!(
                "token colour must be opaque, #rrggbb or #rrggbbff: {colour}"
            ))
        }
    }
}

/// `#rrggbb`.
impl fmt::Display for OpaqueColour {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        TokenColour(self.0).fmt(f)
    }
}

impl From<OpaqueColour> for Color {
    fn from(colour: OpaqueColour) -> Self {
        colour.0.into()
    }
}

/// The rule of the name of a scale step: snake_case that starts with a
/// lowercase ASCII letter, refused at deserialization otherwise. A
/// `Validated<ScaleName>` holds one. The Rust module writes it upper case as
/// the name of a constant, which a digit or a `_` cannot start. The
/// stylesheet writes it with hyphens. A name declared twice is refused by
/// [`check_scale`].
enum ScaleName {}

impl Rule for ScaleName {
    type Error = String;

    fn admit(text: String) -> Result<String, Self::Error> {
        let snake = text.bytes().next().is_some_and(|b| b.is_ascii_lowercase())
            && text
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_');
        if snake {
            Ok(text)
        } else {
            Err(format!(
                "scale name must be snake_case and start with a lowercase letter: {text}"
            ))
        }
    }
}

#[derive(Deserialize)]
struct ChromeColors {
    bg: OpaqueColour,
    surface: OpaqueColour,
    surface_2: OpaqueColour,
    surface_hover: OpaqueColour,
    border: OpaqueColour,
    text: OpaqueColour,
    text_dim: OpaqueColour,
    accent: OpaqueColour,
    success: OpaqueColour,
    warning: OpaqueColour,
    error: OpaqueColour,
    info: OpaqueColour,
    /// Text and icons drawn on an accent ground.
    on_accent: OpaqueColour,
}

impl ChromeColors {
    /// The CSS variable name of each colour, in declaration order.
    fn vars(&self) -> [(&'static str, OpaqueColour); 13] {
        [
            ("bg", self.bg),
            ("surface", self.surface),
            ("surface-2", self.surface_2),
            ("surface-hover", self.surface_hover),
            ("border", self.border),
            ("text", self.text),
            ("text-dim", self.text_dim),
            ("accent", self.accent),
            ("success", self.success),
            ("warning", self.warning),
            ("error", self.error),
            ("info", self.info),
            ("on-accent", self.on_accent),
        ]
    }
}

/// A drop shadow: a vertical offset and a blur, and a colour.
#[derive(Deserialize)]
struct Shadow {
    #[serde(deserialize_with = "whole_px")]
    y: Px<View>,
    #[serde(deserialize_with = "whole_px")]
    blur: Px<View>,
    colour: TokenColour,
}

/// The CSS `box-shadow` value.
impl fmt::Display for Shadow {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "0 {}px {}px {}", self.y, self.blur, self.colour)
    }
}

/// The chrome's shadows and the scrim behind a modal, for one theme.
#[derive(Deserialize)]
struct Effects {
    shadow_low: Shadow,
    shadow_mid: Shadow,
    shadow_high: Shadow,
    scrim: TokenColour,
}

impl Effects {
    /// The CSS variable name of each effect, with its CSS value, in
    /// declaration order.
    fn vars(&self) -> [(&'static str, &dyn fmt::Display); 4] {
        [
            ("shadow-low", &self.shadow_low),
            ("shadow-mid", &self.shadow_mid),
            ("shadow-high", &self.shadow_high),
            ("scrim", &self.scrim),
        ]
    }
}

/// Colours drawn over a picture rather than over a theme ground, so one set
/// serves both themes.
#[derive(Deserialize)]
struct Photo {
    text: TokenColour,
    text_shadow: TokenColour,
    line: TokenColour,
    control_line: TokenColour,
    control_ground: TokenColour,
    veil: TokenColour,
    veil_deep: TokenColour,
}

impl Photo {
    /// The CSS variable name of each colour, in declaration order.
    fn vars(&self) -> [(&'static str, TokenColour); 7] {
        [
            ("photo-text", self.text),
            ("photo-text-shadow", self.text_shadow),
            ("photo-line", self.line),
            ("photo-control-line", self.control_line),
            ("photo-control-ground", self.control_ground),
            ("photo-veil", self.veil),
            ("photo-veil-deep", self.veil_deep),
        ]
    }
}

#[derive(Deserialize)]
struct CanvasColors {
    content_bg: OpaqueColour,
    content_pending: OpaqueColour,
    header_bg: OpaqueColour,
    header_active: OpaqueColour,
    header_hover: OpaqueColour,
    grid_line: OpaqueColour,
    header_border: OpaqueColour,
    selection: OpaqueColour,
    cell_text: OpaqueColour,
    header_text: OpaqueColour,
    caret: OpaqueColour,
    cell_error: OpaqueColour,
    /// A cell border the user did not colour.
    cell_border: OpaqueColour,
}

impl CanvasColors {
    /// The `Palette` field name of each colour, in field order.
    fn fields(&self) -> [(&'static str, OpaqueColour); 13] {
        [
            ("content_bg", self.content_bg),
            ("content_pending", self.content_pending),
            ("header_bg", self.header_bg),
            ("header_active", self.header_active),
            ("header_hover", self.header_hover),
            ("grid_line", self.grid_line),
            ("header_border", self.header_border),
            ("selection", self.selection),
            ("cell_text", self.cell_text),
            ("header_text", self.header_text),
            ("caret", self.caret),
            ("cell_error", self.cell_error),
            ("cell_border", self.cell_border),
        ]
    }
}

/// An opaque colour as the Rust module writes it: a `Color::from_rgba8`
/// call, which the compiler evaluates in a constant.
struct RustColour(OpaqueColour);

impl fmt::Display for RustColour {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let Rgba8 { r, g, b, a } = self.0.0;
        write!(f, "Color::from_rgba8({r}, {g}, {b}, {a})")
    }
}

/// Refuse a scale name declared twice. Each name is snake_case by its type.
fn check_scale(scale: &[(Validated<ScaleName>, Px<View>)]) -> Result<(), String> {
    let mut seen = BTreeSet::new();
    scale.iter().try_for_each(|(name, _)| {
        if seen.insert(name) {
            Ok(())
        } else {
            Err(format!("scale name declared twice: {}", name.as_str()))
        }
    })
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
/// The source does not parse, or a colour is not hex. A colour the Rust
/// module carries is not opaque. A scale name is not snake_case, or is
/// declared twice. The message names the token.
pub fn generate(source: &str) -> Result<Generated, String> {
    let tokens: Tokens =
        ron::from_str(source).map_err(|e| format!("parse {}: {e}", TokenFile::Source))?;
    check_scale(&tokens.scale)?;
    Ok(Generated {
        rust: rust_module(&tokens),
        css: stylesheet(&tokens),
    })
}

/// The Rust module: `Palette`, `ChromePalette`, their constants, the scales.
fn rust_module(tokens: &Tokens) -> String {
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
        for (field, colour) in colors.fields() {
            rs.push_str(&format!("    {field}: {},\n", RustColour(colour)));
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
        for (var, colour) in colors.vars() {
            let field = var.replace('-', "_");
            rs.push_str(&format!("    {field}: {},\n", RustColour(colour)));
        }
        rs.push_str("};\n\n");
    }

    for (name, value) in &tokens.scale {
        let rust_name = name.as_str().to_ascii_uppercase();
        // A scale step is a length on the canvas's view (`CANVAS-032`): a
        // spacing, a radius or a font size the canvas draws at. It is
        // whole, so it prints with no fraction, and `.0` makes it a float.
        rs.push_str(&format!(
            "pub const {rust_name}: ambre_core::Px<ambre_core::View> = ambre_core::Px::new({value}.0);\n"
        ));
    }
    rs
}

/// The stylesheet: `:root` carries the light set, `[data-theme="dark"]` the
/// dark one. A scale is declared once, in `:root`.
fn stylesheet(tokens: &Tokens) -> String {
    let mut css = String::new();
    css.push_str(
        "/* @generated by `just tokens-write` from crates/ambre-canvas/tokens.ron -- do not edit. */\n",
    );
    css.push_str(":root {\n");
    for (var, colour) in tokens.chrome.light.vars() {
        css.push_str(&format!("    --{var}: {colour};\n"));
    }
    for (var, value) in tokens.effects.light.vars() {
        css.push_str(&format!("    --{var}: {value};\n"));
    }
    for (var, colour) in tokens.photo.vars() {
        css.push_str(&format!("    --{var}: {colour};\n"));
    }
    for (name, value) in &tokens.scale {
        let var = name.as_str().replace('_', "-");
        css.push_str(&format!("    --{var}: {value}px;\n"));
    }
    css.push_str("}\n\n");
    css.push_str(":root[data-theme=\"dark\"] {\n");
    for (var, colour) in tokens.chrome.dark.vars() {
        css.push_str(&format!("    --{var}: {colour};\n"));
    }
    for (var, value) in tokens.effects.dark.vars() {
        css.push_str(&format!("    --{var}: {value};\n"));
    }
    css.push_str("}\n");
    css
}

/// Both files for the token source on disk.
///
/// # Errors
///
/// The source cannot be read, or [`generate`] refuses it.
pub fn generate_from_disk() -> Result<Generated, TokenError> {
    let file = TokenFile::Source;
    let source =
        fs::read_to_string(file.path()).map_err(|error| TokenError::Read { file, error })?;
    generate(&source).map_err(TokenError::Refused)
}

/// Each generated file, with the whole text it must hold.
fn outputs(generated: &Generated) -> [(TokenFile, &String); 2] {
    [
        (TokenFile::Rust, &generated.rust),
        (TokenFile::Css, &generated.css),
    ]
}

/// Each generated file whose text on disk is not `generated`. A file that
/// cannot be read is stale.
pub fn stale(generated: &Generated) -> Vec<TokenFile> {
    outputs(generated)
        .into_iter()
        .filter(|(file, text)| fs::read_to_string(file.path()).ok().as_ref() != Some(*text))
        .map(|(file, _)| file)
        .collect()
}

/// Rewrite both files with `generated`.
///
/// # Errors
///
/// A file cannot be written. The error names it.
pub fn write(generated: &Generated) -> Result<(), TokenError> {
    outputs(generated).into_iter().try_for_each(|(file, text)| {
        fs::write(file.path(), text).map_err(|error| TokenError::Write { file, error })
    })
}

#[cfg(test)]
mod tests {
    use super::{Color, TokenError, TokenFile, Tokens, generate, stale};

    fn source() -> String {
        std::fs::read_to_string(TokenFile::Source.path()).unwrap()
    }

    /// The deserialization of the source alone, before any generation.
    fn parsed(source: &str) -> Result<Tokens, ron::error::SpannedError> {
        ron::from_str(source)
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
        assert_eq!(
            stale(&generate(&changed).unwrap()),
            [TokenFile::Rust, TokenFile::Css]
        );
    }

    #[test]
    fn a_malformed_colour_is_refused() {
        let broken = source().replace("\"#f2a93d\"", "\"#f2a93\"");
        assert!(generate(&broken).is_err());
    }

    // A colour has a parse rule, so the text is refused where it enters:
    // the deserialization of the source, before any file is generated.
    #[test]
    fn a_malformed_colour_fails_the_deserialization() {
        assert!(parsed(&source()).is_ok());
        for (from, to) in [
            ("\"#f2a93d\"", "\"#f2a93\""),
            ("\"#f2a93d\"", "\"#f2a9zz\""),
            ("\"#f2a93d\"", "\"f2a93d00\""),
            ("\"#0a0a1473\"", "\"#0a0a147\""),
            ("\"#000000bf\"", "\"#000000bfff\""),
        ] {
            let broken = source().replace(from, to);
            assert_ne!(broken, source());
            let refused = parsed(&broken).err().map(|e| e.to_string());
            assert!(
                refused
                    .as_ref()
                    .is_some_and(|e| e.contains("token colour must be")),
                "{to}: {refused:?}"
            );
        }
    }

    // The Rust module carries the canvas and the chrome colours, so a
    // colour with an alpha below full is refused there, and nowhere else.
    #[test]
    fn a_translucent_canvas_or_chrome_colour_fails_the_deserialization() {
        for from in ["\"#f2a93d\"", "\"#141414\""] {
            let broken = source().replace(from, "\"#14141480\"");
            assert_ne!(broken, source());
            let refused = parsed(&broken).err().map(|e| e.to_string());
            assert!(
                refused
                    .as_ref()
                    .is_some_and(|e| e.contains("must be opaque, #rrggbb or #rrggbbff")),
                "{from}: {refused:?}"
            );
        }
        let opaque_photo = source().replace("\"#000000bf\"", "\"#000000ff\"");
        assert!(parsed(&opaque_photo).is_ok());
        let full_alpha = source().replace("\"#f2a93d\"", "\"#f2a93dff\"");
        assert_ne!(full_alpha, source());
        assert!(parsed(&full_alpha).is_ok());
    }

    // A scale name has a rule too, snake_case, held by its type. The
    // duplicate check stays on the whole scale.
    #[test]
    fn a_scale_name_that_is_not_snake_case_fails_the_deserialization() {
        for bad in ["\"Space_1\"", "\"space-1\"", "\"\""] {
            let broken = source().replace("\"space_1\"", bad);
            assert_ne!(broken, source());
            let refused = parsed(&broken).err().map(|e| e.to_string());
            assert!(
                refused.as_ref().is_some_and(|e| e.contains("snake_case")),
                "{bad}: {refused:?}"
            );
        }
        // The Rust module names a constant after it, which a digit or a
        // `_` cannot start.
        for bad in ["\"1space\"", "\"_space\""] {
            let broken = source().replace("\"space_1\"", bad);
            let refused = parsed(&broken).err().map(|e| e.to_string());
            assert!(
                refused
                    .as_ref()
                    .is_some_and(|e| e.contains("start with a lowercase letter")),
                "{bad}: {refused:?}"
            );
        }
        let twice = source().replace("\"space_2\"", "\"space_1\"");
        assert!(parsed(&twice).is_ok());
        assert!(
            generate(&twice)
                .err()
                .is_some_and(|e| e.contains("declared twice: space_1"))
        );
    }

    // A token length is a `Px<View>`, whose `f32` holds a whole pixel up to
    // `u16::MAX` exactly. A fractional, a negative or a larger value is
    // refused where the source is read, so the files print back the digits
    // of the source.
    #[test]
    fn a_length_that_is_not_a_whole_pixel_fails_the_deserialization() {
        let step = "(\"space_1\",         2)";
        let shadow = "blur: 48,";
        for (from, to) in [
            (step, "(\"space_1\",         2.5)"),
            (step, "(\"space_1\",         -2)"),
            (step, "(\"space_1\",         65536)"),
            (shadow, "blur: 48.5,"),
            (shadow, "blur: -1,"),
            (shadow, "blur: 65536,"),
        ] {
            let broken = source().replacen(from, to, 1);
            assert_ne!(broken, source());
            assert!(parsed(&broken).is_err(), "{to}");
        }
        let largest = source().replacen(step, "(\"space_1\",         65535)", 1);
        let generated = generate(&largest).unwrap();
        assert!(generated.css.contains("--space-1: 65535px;"));
        assert!(
            generated.rust.contains(
                "SPACE_1: ambre_core::Px<ambre_core::View> = ambre_core::Px::new(65535.0);"
            )
        );
    }

    // The generated module holds the colours of the source: a constant
    // equals the conversion of the parsed token.
    #[test]
    fn a_generated_constant_is_the_colour_of_its_token() {
        let tokens = parsed(&source()).unwrap();
        assert_eq!(
            Color::from(tokens.canvas.light.caret),
            crate::theme::LIGHT.caret
        );
        assert_eq!(
            Color::from(tokens.chrome.dark.accent),
            crate::theme::CHROME_DARK.accent
        );
    }

    // An I/O failure names the file by its role, never by a bare string.
    #[test]
    fn an_io_error_names_its_file() {
        let error = TokenError::Write {
            file: TokenFile::Css,
            error: std::io::Error::other("disk full"),
        };
        assert_eq!(error.to_string(), "write src/theme.css: disk full");
        assert!(TokenFile::Rust.path().ends_with("src/theme_gen.rs"));
    }
}
