// ambre-canvas/src/widget_pod.rs
// ============================================================================
// WidgetPod<W> -- composition primitive wrapping a concrete widget with
// framework state (layout cache, hot/focus flags).
//
// A Select contains WidgetPod<SelectTrigger> and Option<WidgetPod<DropdownList>>
// in its fields -- fully composable, no arena indirection needed.
// ============================================================================

use ambre_draw::DrawSurface;
use kurbo::{Point, Rect, Size};

use crate::constraints::BoxConstraints;
use crate::context::{LayoutCtx, PaintCtx};
use crate::event::{EventResponse, PointerEvent};
use crate::widget::{Widget, WidgetId};

/// Framework state associated with a widget instance.
pub struct WidgetState {
    pub id:           WidgetId,
    /// Absolute position + size -- filled by the layout pass.
    pub bounds:       Rect,
    /// True if the pointer is currently over this widget.
    pub is_hot:       bool,
    /// True if this widget has keyboard focus.
    pub has_focus:    bool,
    /// True if a re-layout is needed (size or position changed).
    pub needs_layout: bool,
    /// True if a repaint is needed.
    pub needs_paint:  bool,
}

/// Composition wrapper. `W` can be a concrete type or `Box<dyn Widget>`.
pub struct WidgetPod<W: Widget> {
    pub widget: W,
    pub state:  WidgetState,
}

impl<W: Widget> WidgetPod<W> {
    pub fn new(id: WidgetId, widget: W) -> Self {
        Self {
            widget,
            state: WidgetState {
                id,
                bounds:       Rect::new(0.0, 0.0, 0.0, 0.0),
                is_hot:       false,
                has_focus:    false,
                needs_layout: true,
                needs_paint:  true,
            },
        }
    }

    /// Dispatch a pointer event. Pre-filter: if `pos` is outside bounds,
    /// returns `Ignored` (unless the event is a capture).
    pub fn dispatch_pointer(&mut self, event: &PointerEvent) -> EventResponse {
        let was_hot = self.state.is_hot;
        self.state.is_hot = self.state.bounds.contains(event.pos);

        if self.state.is_hot != was_hot {
            self.state.needs_paint = true;
        }

        if self.state.is_hot || event.is_capture() {
            self.widget.on_pointer_event(event, self.state.bounds)
        } else {
            EventResponse::Ignored
        }
    }

    /// Run the layout pass on this widget. Skipped if `needs_layout` is false.
    pub fn layout(&mut self, ctx: &mut LayoutCtx, bc: BoxConstraints) -> Size {
        if !self.state.needs_layout {
            return self.state.bounds.size();
        }
        let size = self.widget.layout(ctx, bc);
        self.state.bounds = Rect::from_origin_size(self.state.bounds.origin(), size);
        self.state.needs_layout = false;
        self.state.needs_paint = true;
        size
    }

    /// Update the widget origin (absolute position). Marks needs_paint if changed.
    pub fn set_origin(&mut self, origin: Point) {
        if self.state.bounds.origin() != origin {
            self.state.bounds = Rect::from_origin_size(origin, self.state.bounds.size());
            self.state.needs_paint = true;
        }
    }

    /// Paint this widget via the DrawSurface.
    pub fn paint(&self, ctx: &PaintCtx, surface: &mut dyn DrawSurface) {
        self.widget.paint(ctx, surface, self.state.bounds);
    }

    /// Mark this widget (and implicitly its subtree) for re-layout.
    pub fn invalidate_layout(&mut self) {
        self.state.needs_layout = true;
    }

    /// Mark this widget for repaint.
    pub fn invalidate_paint(&mut self) {
        self.state.needs_paint = true;
    }

    pub fn id(&self) -> WidgetId {
        self.state.id
    }

    pub fn bounds(&self) -> Rect {
        self.state.bounds
    }
}
