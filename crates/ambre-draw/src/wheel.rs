// ambre-draw/src/wheel.rs
//! The wheel delta every view reads, with the unit the browser gave it.
//!
//! A DOM `WheelEvent` reports its deltas in pixels, in lines or in pages, and
//! names the unit in `deltaMode`. Firefox sends a mouse notch as three lines,
//! where Chrome sends it as about a hundred pixels. A view that reads the bare
//! number scrolls three pixels for the Firefox notch. [`WheelDelta`] keeps the
//! number and its unit together, and [`WheelDelta::to_px`] is the one way to
//! read it as a distance (`KINETIC-015`).

/// The pixels one wheel line is worth, in a view whose content has no line of
/// its own. Trame steps by a zoomed cell instead.
pub const WHEEL_LINE_PX: f64 = 40.0;

/// The unit of a wheel delta, as the DOM's `WheelEvent.deltaMode` names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WheelDeltaMode {
    /// CSS pixels: a trackpad, or a high-resolution mouse.
    Pixel,
    /// Lines: a classic notched wheel, as Firefox reports it.
    Line,
    /// Pages: a wheel the system set to scroll a page per notch.
    Page,
}

impl WheelDeltaMode {
    /// The unit a DOM `deltaMode` names. A value the DOM does not define reads
    /// as pixels, the unit a modern browser sends.
    #[must_use]
    pub const fn from_dom(mode: u32) -> Self {
        match mode {
            1 => Self::Line,
            2 => Self::Page,
            _ => Self::Pixel,
        }
    }
}

/// One wheel event's deltas, in the unit the browser gave them.
///
/// The fields are private: a view reads the distance through
/// [`WheelDelta::to_px`], which asks it what a line and a page are worth.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WheelDelta {
    dx: f64,
    dy: f64,
    mode: WheelDeltaMode,
}

impl WheelDelta {
    /// A delta of `(dx, dy)` in `mode` units.
    #[must_use]
    pub const fn new(dx: f64, dy: f64, mode: WheelDeltaMode) -> Self {
        Self { dx, dy, mode }
    }

    /// A delta of `(dx, dy)` CSS pixels.
    #[must_use]
    pub const fn pixels(dx: f64, dy: f64) -> Self {
        Self::new(dx, dy, WheelDeltaMode::Pixel)
    }

    /// The unit the browser gave.
    #[must_use]
    pub const fn mode(self) -> WheelDeltaMode {
        self.mode
    }

    /// Whether the delta is a discrete step, a line or a page, rather than a
    /// pixel glide. A step never starts a coast (`KINETIC-009`).
    #[must_use]
    pub fn is_step(self) -> bool {
        self.mode != WheelDeltaMode::Pixel
    }

    /// The delta in CSS pixels, given what one line and one page are worth on
    /// each axis, as `(x, y)`.
    #[must_use]
    pub fn to_px(self, line: (f64, f64), page: (f64, f64)) -> (f64, f64) {
        let (sx, sy) = match self.mode {
            WheelDeltaMode::Pixel => (1.0, 1.0),
            WheelDeltaMode::Line => line,
            WheelDeltaMode::Page => page,
        };
        (self.dx * sx, self.dy * sy)
    }
}

#[cfg(test)]
mod tests {
    use super::{WHEEL_LINE_PX, WheelDelta, WheelDeltaMode};

    const LINE: (f64, f64) = (WHEEL_LINE_PX, WHEEL_LINE_PX);
    const PAGE: (f64, f64) = (800.0, 600.0);

    /// `KINETIC-015`: each unit reads as its pixels, on both axes.
    #[test]
    fn each_unit_reads_as_its_pixels() {
        assert_eq!(WheelDelta::pixels(3.0, -5.0).to_px(LINE, PAGE), (3.0, -5.0));
        let line = WheelDelta::new(1.0, 3.0, WheelDeltaMode::Line);
        assert_eq!(line.to_px(LINE, PAGE), (40.0, 120.0));
        let page = WheelDelta::new(0.0, -1.0, WheelDeltaMode::Page);
        assert_eq!(page.to_px(LINE, PAGE), (0.0, -600.0));
    }

    #[test]
    fn a_line_or_a_page_is_a_step_and_a_pixel_is_not() {
        assert!(!WheelDelta::pixels(0.0, 1.0).is_step());
        assert!(WheelDelta::new(0.0, 1.0, WheelDeltaMode::Line).is_step());
        assert!(WheelDelta::new(0.0, 1.0, WheelDeltaMode::Page).is_step());
    }

    #[test]
    fn the_dom_modes_name_their_units() {
        assert_eq!(WheelDeltaMode::from_dom(0), WheelDeltaMode::Pixel);
        assert_eq!(WheelDeltaMode::from_dom(1), WheelDeltaMode::Line);
        assert_eq!(WheelDeltaMode::from_dom(2), WheelDeltaMode::Page);
        assert_eq!(WheelDeltaMode::from_dom(7), WheelDeltaMode::Pixel);
    }
}
