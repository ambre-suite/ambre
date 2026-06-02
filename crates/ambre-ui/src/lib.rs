// ambre-ui/src/lib.rs
// ============================================================================
// Widget framework + GPU rendering pipeline for the Ambre UI Thread.
//
// Two sub-systems, one crate:
//
//   render/       -- QuadRenderer, GlyphAtlas, WgpuSurface (wgpu-dependent)
//   text_cache    -- Parley layout cache (parley-dependent)
//   widget system -- Widget trait, WidgetPod, layout (taffy), events,
//                    overlays, focus, theme, display mode (GPU-agnostic)
//
// The widget system depends only on ambre-draw::DrawSurface for painting.
// It never imports wgpu, parley, or swash directly.
// ============================================================================

// -- Rendering pipeline (wgpu + parley + swash) ------------------------------
pub mod render;
pub mod text_cache;

// -- Widget framework (GPU-agnostic) -----------------------------------------
pub mod constraints;
pub mod context;
pub mod display_mode;
pub mod event;
pub mod focus;
pub mod layer;
pub mod theme;
pub mod widget;
pub mod widget_pod;

// -- Convenience re-exports --------------------------------------------------
pub use constraints::BoxConstraints;
pub use context::{LayoutCtx, PaintCtx};
pub use display_mode::DisplayMode;
pub use event::{
    apply_response, CursorIcon, EventResponse, KeyEvent, KeyModifiers, PointerButton,
    PointerEvent, PointerEventKind,
};
pub use focus::FocusManager;
pub use layer::{LayerManager, OverlayId, OverlayKind};
pub use render::{CellsRepaint, Layer, OverlayRepaint, QuadRenderer, WgpuSurface};
pub use text_cache::TextCache;
pub use theme::{Theme, ThemeKey, ThemeValue};
pub use widget::{Widget, WidgetId};
pub use widget_pod::{WidgetPod, WidgetState};
