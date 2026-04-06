// ambre-ui/src/theme.rs
// ============================================================================
// Theme system -- resolution of colors and metrics via ThemeKey.
// Zero hardcoding in widgets. The theme is a flat HashMap passed in PaintCtx.
// ============================================================================

use kurbo::Size;
use peniko::Color;
use std::collections::HashMap;
use std::sync::Arc;

/// Typed theme key. The associated value lives in ThemeValue.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ThemeKey(pub &'static str);

/// Typed theme value.
#[derive(Clone, Debug)]
pub enum ThemeValue {
    Color(Color),
    Float(f64),
    Str(Arc<str>),
}

/// Flat map of theme values. Passed via `PaintCtx` -- no global Env.
pub struct Theme {
    pub values:   HashMap<ThemeKey, ThemeValue>,
    /// Cached viewport size for overlay repositioning.
    pub viewport: Size,
}

impl Theme {
    /// Resolve a color key. Returns magenta on missing key (visible bug).
    pub fn resolve_color(&self, key: ThemeKey) -> Color {
        match self.values.get(&key) {
            Some(ThemeValue::Color(c)) => *c,
            _ => Color::from_rgba8(255, 0, 255, 255),
        }
    }

    /// Resolve a float key. Returns 0.0 on missing key.
    pub fn resolve_f64(&self, key: ThemeKey) -> f64 {
        match self.values.get(&key) {
            Some(ThemeValue::Float(f)) => *f,
            _ => 0.0,
        }
    }

    /// Resolve a string key. Returns empty on missing key.
    pub fn resolve_str(&self, key: ThemeKey) -> Arc<str> {
        match self.values.get(&key) {
            Some(ThemeValue::Str(s)) => Arc::clone(s),
            _ => Arc::from(""),
        }
    }
}

// ---------------------------------------------------------------------------
// Theme constructors
// ---------------------------------------------------------------------------

/// Light theme (default).
pub fn theme_light() -> Theme {
    let mut v = HashMap::new();

    macro_rules! c {
        ($key:expr, $r:expr, $g:expr, $b:expr) => {
            v.insert($key, ThemeValue::Color(Color::from_rgba8($r, $g, $b, 255)));
        };
        ($key:expr, $r:expr, $g:expr, $b:expr, $a:expr) => {
            v.insert($key, ThemeValue::Color(Color::from_rgba8($r, $g, $b, $a)));
        };
    }
    macro_rules! f {
        ($key:expr, $val:expr) => {
            v.insert($key, ThemeValue::Float($val));
        };
    }

    // -- Surfaces --
    c!(keys::SURFACE,             255, 255, 255);
    c!(keys::SURFACE_VARIANT,     245, 245, 247);
    c!(keys::ON_SURFACE,           28,  28,  30);
    c!(keys::ON_SURFACE_VARIANT,  100, 100, 110);
    c!(keys::OVERLAY_SCRIM,         0,   0,   0, 128);

    // -- Accent --
    c!(keys::ACCENT,               10, 132, 255);
    c!(keys::ACCENT_FOREGROUND,   255, 255, 255);

    // -- Menus --
    c!(keys::MENU_BACKGROUND,     255, 255, 255);
    c!(keys::MENU_ITEM_HOVER,     242, 242, 247);
    c!(keys::MENU_SEPARATOR,      220, 220, 225);

    // -- Inputs --
    c!(keys::INPUT_BORDER,        200, 200, 210);
    c!(keys::INPUT_BORDER_FOCUS,   10, 132, 255);
    c!(keys::INPUT_BACKGROUND,    255, 255, 255);

    // -- Status --
    c!(keys::COLOR_SUCCESS,        52, 199,  89);
    c!(keys::COLOR_WARNING,       255, 204,   0);
    c!(keys::COLOR_ERROR,         255,  59,  48);
    c!(keys::COLOR_INFO,           10, 132, 255);

    // -- Embed --
    c!(keys::EMBED_HANDLE_FILL,    10, 132, 255);
    c!(keys::EMBED_HANDLE_STROKE, 255, 255, 255);
    c!(keys::EMBED_SELECTION,      10, 132, 255);

    // -- Metrics --
    f!(keys::TOOLBAR_HEIGHT,       48.0);
    f!(keys::SPACING_XS,            4.0);
    f!(keys::SPACING_SM,            8.0);
    f!(keys::SPACING_MD,           16.0);
    f!(keys::SPACING_LG,           24.0);
    f!(keys::BORDER_RADIUS,         6.0);
    f!(keys::FONT_SIZE_BODY,       14.0);
    f!(keys::FONT_SIZE_SMALL,      12.0);
    f!(keys::FONT_SIZE_HEADING,    20.0);

    // -- Rendering-layer keys (ambre-spec-rendering.md D) --
    f!(keys::SCALE_FACTOR,          1.0);

    Theme { values: v, viewport: Size::ZERO }
}

/// Dark theme (placeholder -- to be fleshed out).
pub fn theme_dark() -> Theme {
    let mut t = theme_light();

    t.values.insert(keys::SURFACE,           ThemeValue::Color(Color::from_rgba8( 28,  28,  30, 255)));
    t.values.insert(keys::SURFACE_VARIANT,   ThemeValue::Color(Color::from_rgba8( 44,  44,  46, 255)));
    t.values.insert(keys::ON_SURFACE,        ThemeValue::Color(Color::from_rgba8(242, 242, 247, 255)));
    t.values.insert(keys::ON_SURFACE_VARIANT,ThemeValue::Color(Color::from_rgba8(174, 174, 178, 255)));
    t.values.insert(keys::MENU_BACKGROUND,   ThemeValue::Color(Color::from_rgba8( 44,  44,  46, 255)));
    t.values.insert(keys::MENU_ITEM_HOVER,   ThemeValue::Color(Color::from_rgba8( 58,  58,  60, 255)));
    t.values.insert(keys::INPUT_BACKGROUND,  ThemeValue::Color(Color::from_rgba8( 44,  44,  46, 255)));

    t
}

// ---------------------------------------------------------------------------
// Theme key constants
// ---------------------------------------------------------------------------

pub mod keys {
    use super::ThemeKey;

    // Surfaces
    pub const SURFACE:             ThemeKey = ThemeKey("ambre.surface");
    pub const SURFACE_VARIANT:     ThemeKey = ThemeKey("ambre.surface.variant");
    pub const ON_SURFACE:          ThemeKey = ThemeKey("ambre.on_surface");
    pub const ON_SURFACE_VARIANT:  ThemeKey = ThemeKey("ambre.on_surface.variant");
    pub const OVERLAY_SCRIM:       ThemeKey = ThemeKey("ambre.overlay.scrim");

    // Accent
    pub const ACCENT:              ThemeKey = ThemeKey("ambre.accent");
    pub const ACCENT_FOREGROUND:   ThemeKey = ThemeKey("ambre.accent.foreground");

    // Menus
    pub const MENU_BACKGROUND:     ThemeKey = ThemeKey("ambre.menu.background");
    pub const MENU_ITEM_HOVER:     ThemeKey = ThemeKey("ambre.menu.item.hover");
    pub const MENU_SEPARATOR:      ThemeKey = ThemeKey("ambre.menu.separator");

    // Inputs
    pub const INPUT_BORDER:        ThemeKey = ThemeKey("ambre.input.border");
    pub const INPUT_BORDER_FOCUS:  ThemeKey = ThemeKey("ambre.input.border.focus");
    pub const INPUT_BACKGROUND:    ThemeKey = ThemeKey("ambre.input.background");

    // Status
    pub const COLOR_SUCCESS:       ThemeKey = ThemeKey("ambre.color.success");
    pub const COLOR_WARNING:       ThemeKey = ThemeKey("ambre.color.warning");
    pub const COLOR_ERROR:         ThemeKey = ThemeKey("ambre.color.error");
    pub const COLOR_INFO:          ThemeKey = ThemeKey("ambre.color.info");

    // Embed
    pub const EMBED_HANDLE_FILL:   ThemeKey = ThemeKey("ambre.embed.handle.fill");
    pub const EMBED_HANDLE_STROKE: ThemeKey = ThemeKey("ambre.embed.handle.stroke");
    pub const EMBED_SELECTION:     ThemeKey = ThemeKey("ambre.embed.selection");

    // Metrics
    pub const TOOLBAR_HEIGHT:      ThemeKey = ThemeKey("ambre.toolbar.height");
    pub const SPACING_XS:          ThemeKey = ThemeKey("ambre.spacing.xs");
    pub const SPACING_SM:          ThemeKey = ThemeKey("ambre.spacing.sm");
    pub const SPACING_MD:          ThemeKey = ThemeKey("ambre.spacing.md");
    pub const SPACING_LG:          ThemeKey = ThemeKey("ambre.spacing.lg");
    pub const BORDER_RADIUS:       ThemeKey = ThemeKey("ambre.border.radius");
    pub const FONT_SIZE_BODY:      ThemeKey = ThemeKey("ambre.font.size.body");
    pub const FONT_SIZE_SMALL:     ThemeKey = ThemeKey("ambre.font.size.small");
    pub const FONT_SIZE_HEADING:   ThemeKey = ThemeKey("ambre.font.size.heading");

    // Rendering-layer (ambre-spec-rendering.md D)
    pub const SCALE_FACTOR:        ThemeKey = ThemeKey("ambre.scale_factor");
    pub const FONT_FAMILY_SANS:    ThemeKey = ThemeKey("ambre.font.family.sans");
    pub const FONT_FAMILY_MONO:    ThemeKey = ThemeKey("ambre.font.family.mono");
    pub const TOOLBAR_ITEM_ACTIVE: ThemeKey = ThemeKey("ambre.toolbar.item.active");
}
