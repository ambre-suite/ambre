// ambre-canvas/src/lib.rs
// ============================================================================
// Content rendering runtime for the Ambre UI thread.
//
// Scope is the canvas *content* renderer only -- the widget framework was
// removed in favour of the DOM Leptos chrome (ambre-chrome). The UI chrome
// (menus, toolbars, dialogs, palette) lives in the DOM. This crate owns the
// wgpu surface, the quad/glyph pipeline, the GPU bootstrap + rAF primitives,
// and the per-frame value types. The frame DRIVER (FrameHost) and the closed
// AppView registry live in the launcher (ambre-app), downstream of the apps.
//
//   render/       -- QuadRenderer, GlyphAtlas, WgpuSurface (wgpu + swash).
//   text_cache    -- Parley layout cache (parley-dependent).
//   view          -- per-frame value types (FrameStats/RepaintPlan/InputEvent).
//   host          -- GPU bootstrap + rAF primitives (wasm); the frame DRIVER
//                    (FrameHost) + the AppView registry live in ambre-app.
//   request_registry -- P3 request/response correlator (ARCH P1).
// ============================================================================

// -- Rendering pipeline (wgpu + parley + swash) ------------------------------
pub mod render;
pub mod text_cache;

// -- Shared inline text-editor core (parley PlainEditor + font-free intent
//    queue), wrapped by Toile's element editor and Trame's cell editor --------
pub mod editor;

// -- Per-frame view value types (GPU-agnostic) -------------------------------
pub mod view;

// -- The display density, read once, and the two sizes of the buffer --------
pub mod density;

// -- Process-global fatal latch (canvas terminal screen on unrecoverable error)
pub mod fatal;

// -- UI Thread host plumbing (GPU bootstrap + rAF primitives, wasm32-only) ----
#[cfg(target_arch = "wasm32")]
pub mod host;

// -- Transparent <textarea> keyboard/IME pipe (pure web_sys DOM glue; compiles
//    on every target so app crates can name the handle in host-side tests) ----
pub mod text_input;

// -- Selection anchor: canvas->DOM signal for floating chrome UI ------------
pub mod anchor;

// -- One-shot UI intents: canvas->DOM "open a floating surface now" pulses ----
pub mod intent;

// -- Busy level: canvas->DOM signal for a view that refuses input -----------
pub mod busy;

// -- Perf snapshot: canvas->DOM signal for the performance HUD --------------
pub mod perf;

// -- Design tokens (generated from tokens.ron): Palette + Theme + ThemeControl
pub mod theme;

// -- Lazy font loading (CJK/Arabic/Devanagari fetched on missing codepoint) --
pub mod fonts;

// -- P3 request/response correlator for the UI thread -----------------------
pub mod request_registry;

// -- Convenience re-exports --------------------------------------------------
pub use anchor::{Anchor, AnchorPublisher};
pub use busy::BusyPublisher;
pub use density::SurfaceSize;
#[cfg(target_arch = "wasm32")]
pub use density::{density_query, size_backing_store};
pub use editor::{CARET_WIDTH, EditIntent, TextEditor, TextEditorStyle};
pub use fatal::{fatal_message, signal_fatal};
#[cfg(target_arch = "wasm32")]
pub use host::{GpuState, RafControl, bootstrap_gpu, start_raf_loop};
pub use intent::{FilterOpenPublisher, NoticePublisher};
pub use perf::PerfPublisher;
pub use render::{ContentRepaint, Layer, OverlayRepaint, QuadRenderer, WgpuSurface};
pub use request_registry::{RequestRegistry, RequestTag};
pub use text_cache::{TextCache, registered_family_name};
pub use text_input::{ContentKind, TextInput};
pub use theme::{Palette, Theme, ThemeControl};
pub use view::{FrameStats, InputEvent, PerfSnapshot, RepaintPlan};
