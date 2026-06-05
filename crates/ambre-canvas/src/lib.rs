// ambre-canvas/src/lib.rs
// ============================================================================
// Content rendering runtime for the Ambre UI thread.
//
// Scope is the canvas *content* renderer only -- the widget framework was
// removed in favour of the DOM Leptos chrome (ambre-chrome). The UI chrome
// (menus, toolbars, dialogs, palette) lives in the DOM; this crate owns the
// wgpu surface, the quad/glyph pipeline and the per-app DocView contract.
//
//   render/       -- QuadRenderer, GlyphAtlas, WgpuSurface (wgpu + swash)
//   text_cache    -- Parley layout cache (parley-dependent)
//   view          -- DocView trait: the per-app UI contract (GPU-agnostic)
//   host          -- UI-thread host plumbing: GPU bootstrap + rAF loop (wasm)
// ============================================================================

// -- Rendering pipeline (wgpu + parley + swash) ------------------------------
pub mod render;
pub mod text_cache;

// -- Per-app UI contract (DocView trait, GPU-agnostic) -----------------------
pub mod view;

// -- UI Thread host plumbing (GPU bootstrap + frame driver, wasm32-only) ------
#[cfg(target_arch = "wasm32")]
pub mod host;

// -- Transparent <textarea> keyboard/IME pipe (pure web_sys DOM glue; compiles
//    on every target so app crates can name the handle in host-side tests) ----
pub mod text_input;

// -- Selection anchor: canvas->DOM channel for floating chrome UI ------------
pub mod anchor;

// -- Design tokens (generated from tokens.ron): Palette + Theme + ThemeControl
pub mod theme;

// -- Convenience re-exports --------------------------------------------------
pub use render::{ContentRepaint, Layer, OverlayRepaint, QuadRenderer, WgpuSurface};
pub use text_cache::TextCache;
pub use view::{DocView, FrameStats, InputEvent, RepaintPlan};
#[cfg(target_arch = "wasm32")]
pub use host::{FrameHost, GpuState, RafControl, bootstrap_gpu, run, run_with, start_raf_loop};
pub use text_input::TextInput;
pub use anchor::{Anchor, AnchorPublisher};
pub use theme::{Palette, Theme, ThemeControl};
