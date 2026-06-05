// ambre-canvas/src/theme.rs
// ============================================================================
// Design tokens, consumed (ambre-spec-ui 2.D, ambre-spec-canvas F). The colour
// palettes and scales are generated from `tokens.ron` by `build.rs`; this module
// wraps them in the runtime theme switch shared across the canvas/DOM boundary:
//
//   Palette / LIGHT / DARK / SPACE_* / RADIUS / FONT_*   (generated)
//   THEME_CSS        the DOM custom-property sheet, injected by the launcher
//   Theme            Light | Dark, selects a Palette and the `data-theme` value
//   ThemeControl     Send handle: the chrome toggles it, the canvas reads it
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

/// Shared, `Send` theme switch: the chrome toggle writes it (from a Leptos
/// callback, hence `Arc<AtomicU8>` like `RafControl`), the canvas view reads it
/// each frame and repaints on change. The single source of truth for the active
/// theme on the UI thread.
#[derive(Clone)]
pub struct ThemeControl {
    v: Arc<AtomicU8>,
}

impl ThemeControl {
    pub fn new(theme: Theme) -> Self {
        Self { v: Arc::new(AtomicU8::new(theme.as_u8())) }
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
