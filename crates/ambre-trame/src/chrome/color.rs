// ambre-trame/src/chrome/color.rs
// ============================================================================
// Themed colour defaults for the formatting controls. These are the "Automatic"
// / "Transparent" fallbacks the shared `ambre_chrome::ColorPopover` shows, so an
// unset colour renders its real on-screen value (theme-aware). The popover itself
// lives in `ambre-chrome`; only these
// canvas-themed defaults stay app-side. Module-private.
// ============================================================================

use ambre_core::Rgb;

/// The themed default text colour (matches the canvas `cell_text` token), so the
/// fg picker, when on "Automatic", shows the colour the glyphs actually render in.
pub(super) fn theme_fg_default(theme: ambre_canvas::Theme) -> Rgb {
    match theme {
        ambre_canvas::Theme::Dark => Rgb::from_bits(0xe8_e8e8),
        ambre_canvas::Theme::Light => Rgb::from_bits(0x20_2020),
    }
}

/// The themed default fill colour (matches the canvas `content_bg` token), so the
/// bg picker, when on "Transparent", shows the sheet background.
pub(super) fn theme_bg_default(theme: ambre_canvas::Theme) -> Rgb {
    match theme {
        ambre_canvas::Theme::Dark => Rgb::from_bits(0x1c_1c1f),
        ambre_canvas::Theme::Light => Rgb::from_bits(0xff_ffff),
    }
}
