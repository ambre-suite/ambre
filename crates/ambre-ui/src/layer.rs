// ambre-ui/src/layer.rs
// ============================================================================
// LayerManager -- overlay Z-layers (dropdowns, modals, tooltips).
//
// Overlays are rendered ABOVE the main widget tree. They capture pointer
// events with priority (top-down). A modal blocks all interaction below it.
// ============================================================================

use ambre_draw::DrawSurface;
use kurbo::{Point, Rect, Size};
use slotmap::Key;

use crate::context::PaintCtx;
use crate::event::{EventResponse, KeyEvent, PointerEvent};
use crate::theme;
use crate::widget::{Widget, WidgetId};

// ---------------------------------------------------------------------------
// OverlayId / OverlayKind
// ---------------------------------------------------------------------------

/// Unique identifier for an overlay, derived from the triggering widget.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct OverlayId(u64);

impl OverlayId {
    /// Derive an overlay ID from the widget that triggered it.
    pub fn from_widget(id: WidgetId) -> Self {
        Self(id.data().as_ffi())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OverlayKind {
    /// Dropdown / popover -- closes on outside click.
    Dropdown,
    /// Modal -- closes only by explicit action. Scrim over everything else.
    Modal,
    /// Tooltip -- closes on pointer move.
    Tooltip,
}

// ---------------------------------------------------------------------------
// Overlay (private)
// ---------------------------------------------------------------------------

struct Overlay {
    id: OverlayId,
    widget: Box<dyn Widget>,
    /// Absolute rect in logical coordinates (dp).
    rect: Rect,
    kind: OverlayKind,
}

// ---------------------------------------------------------------------------
// LayerManager
// ---------------------------------------------------------------------------

pub struct LayerManager {
    overlays: Vec<Overlay>,
    viewport: Size,
}

impl LayerManager {
    pub fn new(viewport: Size) -> Self {
        Self {
            overlays: Vec::new(),
            viewport,
        }
    }

    /// Push an overlay. Automatically repositioned if it overflows the viewport.
    pub fn push(
        &mut self,
        id: OverlayId,
        widget: Box<dyn Widget>,
        mut rect: Rect,
        kind: OverlayKind,
    ) {
        // Clamp to viewport horizontally.
        if rect.x1 > self.viewport.width {
            let w = rect.width();
            rect.x0 = (self.viewport.width - w).max(0.0);
            rect.x1 = rect.x0 + w;
        }
        // Clamp to viewport vertically.
        if rect.y1 > self.viewport.height {
            let h = rect.height();
            rect.y0 = (self.viewport.height - h).max(0.0);
            rect.y1 = rect.y0 + h;
        }
        self.overlays.push(Overlay {
            id,
            widget,
            rect,
            kind,
        });
    }

    /// Remove an overlay by ID.
    pub fn pop(&mut self, id: OverlayId) {
        self.overlays.retain(|o| o.id != id);
    }

    /// Pop the top Dropdown if the click is outside its bounds.
    /// Called by the main dispatcher before forwarding the click to the tree.
    /// Returns `true` if an overlay was dismissed.
    pub fn dismiss_on_outside_click(&mut self, pos: Point) -> bool {
        if let Some(top) = self.overlays.last()
            && top.kind == OverlayKind::Dropdown
            && !top.rect.contains(pos)
        {
            let id = top.id;
            self.pop(id);
            return true;
        }
        false
    }

    /// Hit-test top-down. The highest layer (last) has priority.
    pub fn hit_test(&self, pos: Point) -> Option<OverlayId> {
        self.overlays
            .iter()
            .rev()
            .find(|o| o.rect.contains(pos))
            .map(|o| o.id)
    }

    /// Dispatch a pointer event to the relevant overlay.
    /// Called BEFORE the main widget tree. Returns `Some` if an overlay
    /// consumed the event.
    pub fn dispatch_pointer(&mut self, event: &PointerEvent) -> Option<EventResponse> {
        // Modals block everything: even clicks outside bounds are consumed.
        let has_modal = self.overlays.iter().any(|o| o.kind == OverlayKind::Modal);
        if has_modal
            && let Some(top) = self.overlays.last_mut()
            && top.kind == OverlayKind::Modal
        {
            let r = top.widget.on_pointer_event(event, top.rect);
            return Some(if matches!(r, EventResponse::Ignored) {
                EventResponse::Consumed // modal absorbs everything
            } else {
                r
            });
        }
        // Dropdown / Tooltip: dispatch only if within bounds.
        for overlay in self.overlays.iter_mut().rev() {
            if overlay.rect.contains(event.pos) {
                return Some(overlay.widget.on_pointer_event(event, overlay.rect));
            }
        }
        None
    }

    /// Dispatch a keyboard event to the topmost overlay (focus trap for modals).
    pub fn dispatch_key(&mut self, event: &KeyEvent) -> Option<EventResponse> {
        self.overlays
            .last_mut()
            .map(|o| o.widget.on_key_event(event))
    }

    pub fn has_modal(&self) -> bool {
        self.overlays.iter().any(|o| o.kind == OverlayKind::Modal)
    }

    pub fn is_empty(&self) -> bool {
        self.overlays.is_empty()
    }

    /// Paint all overlays above the main widget tree.
    pub fn paint(&self, ctx: &PaintCtx, surface: &mut dyn DrawSurface) {
        for overlay in &self.overlays {
            if overlay.kind == OverlayKind::Modal {
                // Semi-transparent scrim over the whole surface.
                surface.fill_rect(
                    Rect::new(
                        0.0,
                        0.0,
                        ctx.theme.viewport.width,
                        ctx.theme.viewport.height,
                    ),
                    ctx.theme.resolve_color(theme::keys::OVERLAY_SCRIM),
                );
            }
            surface.push_clip(overlay.rect);
            overlay.widget.paint(ctx, surface, overlay.rect);
            surface.pop_clip();
        }
    }

    /// Update the cached viewport size (called on resize).
    pub fn update_viewport(&mut self, viewport: Size) {
        self.viewport = viewport;
    }
}
