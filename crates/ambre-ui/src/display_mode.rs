// ambre-ui/src/display_mode.rs
// ============================================================================
// Adaptive display mode derived from canvas logical width (dp).
// Passed in LayoutCtx and PaintCtx -- widgets adapt their rendering.
// ============================================================================

/// Display mode derived from the logical width of the canvas.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DisplayMode {
    /// < 600 dp -- phones
    Mobile,
    /// 600..1024 dp -- tablets portrait, small laptops
    Tablet,
    /// >= 1024 dp -- desktop, tablets landscape
    Desktop,
}

impl DisplayMode {
    pub fn from_canvas_width(width_dp: f64) -> Self {
        if width_dp < 600.0 {
            Self::Mobile
        } else if width_dp < 1024.0 {
            Self::Tablet
        } else {
            Self::Desktop
        }
    }

    pub fn is_mobile(self) -> bool {
        self == Self::Mobile
    }

    pub fn is_compact(self) -> bool {
        matches!(self, Self::Mobile | Self::Tablet)
    }
}
