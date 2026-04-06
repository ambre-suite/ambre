// ambre-ui/src/context.rs
// ============================================================================
// LayoutCtx and PaintCtx -- passed to Widget::layout and Widget::paint.
// Both are immutable from the widget's perspective (no framework mutation
// during layout or paint passes).
// ============================================================================

use kurbo::{Rect, Size};
use taffy::{AvailableSpace, NodeId, Style};

use crate::constraints::BoxConstraints;
use crate::display_mode::DisplayMode;
use crate::theme::Theme;

// ---------------------------------------------------------------------------
// LayoutCtx
// ---------------------------------------------------------------------------

/// Context available during `Widget::layout`.
/// Gives access to the taffy Flexbox engine and viewport metrics.
pub struct LayoutCtx<'a> {
    pub taffy:        &'a mut taffy::TaffyTree<()>,
    pub scale:        f64,
    pub viewport:     Size,
    pub display_mode: DisplayMode,
}

impl LayoutCtx<'_> {
    /// Create a taffy leaf node with the given style.
    pub fn new_leaf(&mut self, style: Style) -> NodeId {
        self.taffy.new_leaf(style).expect("taffy new_leaf")
    }

    /// Create a taffy container node with children.
    pub fn new_container(&mut self, style: Style, children: &[NodeId]) -> NodeId {
        self.taffy
            .new_with_children(style, children)
            .expect("taffy new_with_children")
    }

    /// Run taffy layout on `node` with the given box constraints.
    pub fn compute_layout(&mut self, node: NodeId, bc: BoxConstraints) {
        let available = taffy::Size {
            width:  to_available(bc.max.width),
            height: to_available(bc.max.height),
        };
        self.taffy
            .compute_layout(node, available)
            .expect("taffy compute_layout");
    }

    /// Read the computed rect of a child node (position relative to parent).
    pub fn child_rect(&self, node: NodeId) -> Rect {
        let l = self.taffy.layout(node).expect("taffy layout");
        Rect::new(
            l.location.x as f64,
            l.location.y as f64,
            (l.location.x + l.size.width) as f64,
            (l.location.y + l.size.height) as f64,
        )
    }

    /// Read the computed size of a node.
    pub fn node_size(&self, node: NodeId) -> Size {
        let l = self.taffy.layout(node).expect("taffy layout");
        Size::new(l.size.width as f64, l.size.height as f64)
    }

    /// Remove a previously created taffy node (and its children).
    pub fn remove(&mut self, node: NodeId) {
        let _ = self.taffy.remove(node);
    }
}

/// Convert a f64 constraint bound to taffy AvailableSpace.
fn to_available(v: f64) -> AvailableSpace {
    if v.is_finite() {
        AvailableSpace::Definite(v as f32)
    } else {
        AvailableSpace::MaxContent
    }
}

// ---------------------------------------------------------------------------
// PaintCtx
// ---------------------------------------------------------------------------

/// Context available during `Widget::paint`.
/// Gives access to the theme and scale factor. Read-only -- widgets never
/// mutate framework state during paint.
pub struct PaintCtx<'a> {
    pub theme:        &'a Theme,
    pub scale:        f64,
    pub display_mode: DisplayMode,
}
