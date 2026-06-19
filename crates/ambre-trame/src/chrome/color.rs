// ambre-trame/src/chrome/color.rs
// ============================================================================
// Shared colour helpers for the formatting controls: conversions between the
// packed `0xRRGGBB` model and the native `<input type="color">` `"#rrggbb"`
// string, and the themed "Automatic" / "Transparent" defaults the pickers fall
// back to so an unset colour shows its real on-screen value. Module-private.
// ============================================================================

/// Parse an `<input type="color">` value (`"#rrggbb"`) into a packed `0xRRGGBB`.
pub(super) fn parse_hex_color(s: &str) -> Option<u32> {
    let hex = s.strip_prefix('#')?;
    u32::from_str_radix(hex, 16).ok()
}

/// Format a packed `0xRRGGBB` colour into an `<input type="color">` value, using
/// `default` when the cell has no explicit colour (`<input type=color>` cannot be
/// empty). The default is the themed text / fill colour, so an "Automatic" /
/// "Transparent" cell shows its real on-screen colour rather than a misleading
/// black/white -- the picker tracks the active light/dark theme.
pub(super) fn color_hex(c: Option<u32>, default: u32) -> String {
    format!("#{:06x}", c.unwrap_or(default))
}

/// The themed default text colour (matches the canvas `cell_text` token), so the
/// fg picker, when on "Automatic", shows the colour the glyphs actually render in.
pub(super) fn theme_fg_default(theme: ambre_canvas::Theme) -> u32 {
    match theme {
        ambre_canvas::Theme::Dark => 0xe8e8e8,
        ambre_canvas::Theme::Light => 0x202020,
    }
}

/// The themed default fill colour (matches the canvas `content_bg` token), so the
/// bg picker, when on "Transparent", shows the sheet background.
pub(super) fn theme_bg_default(theme: ambre_canvas::Theme) -> u32 {
    match theme {
        ambre_canvas::Theme::Dark => 0x1c1c1f,
        ambre_canvas::Theme::Light => 0xffffff,
    }
}
