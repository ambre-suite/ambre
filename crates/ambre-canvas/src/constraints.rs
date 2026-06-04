// ambre-canvas/src/constraints.rs
// ============================================================================
// BoxConstraints -- min/max size passed from parent to child during layout.
// ============================================================================

use kurbo::Size;

/// Layout constraints passed from parent to child.
/// The widget MUST return a Size within [min, max].
#[derive(Clone, Copy, Debug)]
pub struct BoxConstraints {
    pub min: Size,
    pub max: Size,
}

impl BoxConstraints {
    /// Unconstrained -- the widget chooses its own size.
    pub const UNBOUNDED: Self = Self {
        min: Size::ZERO,
        max: Size::new(f64::INFINITY, f64::INFINITY),
    };

    /// Fixed size -- the widget must take exactly this size.
    pub fn tight(size: Size) -> Self {
        Self { min: size, max: size }
    }

    /// Width-constrained, height free.
    pub fn width(w: f64) -> Self {
        Self {
            min: Size::new(w, 0.0),
            max: Size::new(w, f64::INFINITY),
        }
    }

    /// Clamp a size into the constraints.
    pub fn constrain(&self, size: Size) -> Size {
        Size::new(
            size.width.clamp(self.min.width, self.max.width),
            size.height.clamp(self.min.height, self.max.height),
        )
    }

    /// Whether the constraints allow only a single exact size.
    pub fn is_tight(&self) -> bool {
        (self.min.width - self.max.width).abs() < f64::EPSILON
            && (self.min.height - self.max.height).abs() < f64::EPSILON
    }
}
