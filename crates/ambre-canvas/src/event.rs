// ambre-canvas/src/event.rs
// ============================================================================
// Input events (pointer, keyboard) and EventResponse.
//
// EventResponse is returned by Widget::on_pointer_event / on_key_event.
// The framework applies the effects AFTER dispatch is complete -- zero double
// mutable borrow.
// ============================================================================

use kurbo::{Point, Rect};

use crate::focus::FocusManager;
use crate::layer::{LayerManager, OverlayId, OverlayKind};
use crate::widget::{Widget, WidgetId};

// ---------------------------------------------------------------------------
// Pointer events
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct PointerEvent {
    /// Position in logical pixels (dp), origin top-left.
    pub pos:    Point,
    pub kind:   PointerEventKind,
    pub button: Option<PointerButton>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PointerEventKind {
    Down,
    Up,
    Move,
    Enter,
    Leave,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PointerButton {
    Primary,
    Secondary,
    Middle,
}

impl PointerEvent {
    /// True on primary button release (click end).
    pub fn is_click(&self) -> bool {
        self.kind == PointerEventKind::Up && self.button == Some(PointerButton::Primary)
    }

    /// True if the event should be delivered even outside bounds (e.g. drag).
    pub fn is_capture(&self) -> bool {
        // Phase 0: no capture support yet.
        false
    }
}

// ---------------------------------------------------------------------------
// Keyboard events
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct KeyEvent {
    /// Logical key value (e.g. "a", "Enter", "ArrowUp").
    pub key:       String,
    /// Physical key code (e.g. "KeyA", "Enter", "ArrowUp").
    pub code:      String,
    pub modifiers: KeyModifiers,
    pub kind:      KeyEventKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyEventKind {
    Down,
    Up,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct KeyModifiers {
    pub shift: bool,
    pub ctrl:  bool,
    pub alt:   bool,
    pub meta:  bool,
}

// ---------------------------------------------------------------------------
// Cursor icon
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum CursorIcon {
    #[default]
    Default,
    Pointer,
    Text,
    Crosshair,
    Move,
    ResizeNS,
    ResizeEW,
    ResizeNWSE,
    ResizeNESW,
    NotAllowed,
    Grab,
    Grabbing,
}

// ---------------------------------------------------------------------------
// EventResponse
// ---------------------------------------------------------------------------

/// Result of dispatching an event to a widget.
/// The framework applies these effects AFTER dispatch -- zero double borrow.
pub enum EventResponse {
    /// Event not consumed -- propagation continues to parent.
    Ignored,
    /// Event consumed -- propagation stops.
    Consumed,
    /// Widget requests keyboard focus.
    RequestFocus(WidgetId),
    /// Widget requests opening an overlay (dropdown, modal, tooltip).
    OpenOverlay {
        id:     OverlayId,
        widget: Box<dyn Widget>,
        rect:   Rect,
        kind:   OverlayKind,
    },
    /// Widget requests closing the overlay it opened.
    CloseOverlay(OverlayId),
    /// Widget requests a cursor change.
    SetCursor(CursorIcon),
    /// Multiple simultaneous responses (e.g. consume + change cursor).
    Many(Vec<EventResponse>),
}

// ---------------------------------------------------------------------------
// apply_response
// ---------------------------------------------------------------------------

/// Apply an `EventResponse` to the framework state.
/// Called after event dispatch is complete.
pub fn apply_response(
    response:      EventResponse,
    layer_manager: &mut LayerManager,
    focus_manager: &mut FocusManager,
    cursor:        &mut CursorIcon,
    dirty:         &mut bool,
) {
    match response {
        EventResponse::Ignored | EventResponse::Consumed => {}
        EventResponse::RequestFocus(id) => {
            focus_manager.set_focus(id);
            *dirty = true;
        }
        EventResponse::OpenOverlay { id, widget, rect, kind } => {
            layer_manager.push(id, widget, rect, kind);
            *dirty = true;
        }
        EventResponse::CloseOverlay(id) => {
            layer_manager.pop(id);
            *dirty = true;
        }
        EventResponse::SetCursor(icon) => {
            *cursor = icon;
        }
        EventResponse::Many(responses) => {
            for r in responses {
                apply_response(r, layer_manager, focus_manager, cursor, dirty);
            }
        }
    }
}
