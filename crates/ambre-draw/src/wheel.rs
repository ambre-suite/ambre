// ambre-draw/src/wheel.rs
//! The wheel delta every view reads, with the unit the browser gave it.
//!
//! A DOM `WheelEvent` reports its deltas in pixels, in lines or in pages, and
//! names the unit in `deltaMode`. Firefox sends a mouse notch as three lines,
//! where Chrome sends it as about a hundred pixels. A view that reads the bare
//! number scrolls three pixels for the Firefox notch. [`WheelDelta`] keeps the
//! number and its unit together, and [`WheelDelta::to_px`] is the one way to
//! read it as a distance (`KINETIC-015`).

use ambre_core::Vector;
use ambre_core::{Css, Px};

/// The pixels one wheel line is worth, in a view whose content has no line of
/// its own. Trame steps by a zoomed cell instead.
pub const WHEEL_LINE: Px<Css> = Px::<Css>::new(40.0);

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

    /// Whether a delta in this unit is a discrete step, a line or a page,
    /// rather than a pixel glide. A step never starts a coast (`KINETIC-009`).
    #[must_use]
    pub const fn is_step(self) -> bool {
        !matches!(self, Self::Pixel)
    }
}

/// One axis of a wheel delta, as the browser reported it. It counts the unit
/// that [`WheelDeltaMode`] names: a pixel, a line or a page. The number is
/// not a distance until [`WheelDelta::to_px`] says what one unit is worth.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct WheelAmount(f64);

impl WheelAmount {
    /// An amount of `units`, in the unit of the delta that holds it.
    #[must_use]
    pub const fn new(units: f64) -> Self {
        Self(units)
    }

    /// The amount as CSS pixels, one unit being worth `unit`.
    #[must_use]
    fn scaled(self, unit: f64) -> Px<Css> {
        Px::<Css>::new((self.0 * unit) as f32)
    }
}

/// The two axes of a wheel delta, in the unit the browser gave them.
pub type WheelVector = Vector<WheelAmount>;

/// One wheel event's deltas, in the unit the browser gave them.
///
/// The fields are private: a view reads the distance through
/// [`WheelDelta::to_px`], which asks it what a line and a page are worth.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WheelDelta {
    amount: WheelVector,
    mode: WheelDeltaMode,
}

impl WheelDelta {
    /// A delta of `amount` in `mode` units.
    #[must_use]
    pub const fn new(amount: WheelVector, mode: WheelDeltaMode) -> Self {
        Self { amount, mode }
    }

    /// A delta of `amount` CSS pixels.
    #[must_use]
    pub const fn pixels(amount: WheelVector) -> Self {
        Self::new(amount, WheelDeltaMode::Pixel)
    }

    /// The unit the browser gave.
    #[must_use]
    pub const fn mode(self) -> WheelDeltaMode {
        self.mode
    }

    /// Whether the delta is a discrete step, a line or a page, rather than a
    /// pixel glide. A step never starts a coast (`KINETIC-009`).
    #[must_use]
    pub const fn is_step(self) -> bool {
        self.mode.is_step()
    }

    /// The delta in CSS pixels, given what one line and one page are worth on
    /// each axis.
    #[must_use]
    pub fn to_px(self, line: Vector<Px<Css>>, page: Vector<Px<Css>>) -> Vector<Px<Css>> {
        let (sx, sy) = match self.mode {
            WheelDeltaMode::Pixel => (1.0, 1.0),
            WheelDeltaMode::Line => (f64::from(line.dx.get()), f64::from(line.dy.get())),
            WheelDeltaMode::Page => (f64::from(page.dx.get()), f64::from(page.dy.get())),
        };
        Vector::new(self.amount.dx.scaled(sx), self.amount.dy.scaled(sy))
    }
}

#[cfg(test)]
mod tests {
    use super::{WHEEL_LINE, WheelAmount, WheelDelta, WheelDeltaMode, WheelVector};
    use ambre_core::{Css, Px, Vector};

    const LINE: Vector<Px<Css>> = Vector::new(WHEEL_LINE, WHEEL_LINE);
    const PAGE: Vector<Px<Css>> = Vector::new(Px::<Css>::new(800.0), Px::<Css>::new(600.0));

    fn px(dx: f32, dy: f32) -> Vector<Px<Css>> {
        Vector::new(Px::<Css>::new(dx), Px::<Css>::new(dy))
    }

    fn units(dx: f64, dy: f64) -> WheelVector {
        WheelVector::new(WheelAmount::new(dx), WheelAmount::new(dy))
    }

    /// `KINETIC-015`: each unit reads as its pixels, on both axes.
    #[test]
    fn each_unit_reads_as_its_pixels() {
        assert_eq!(
            WheelDelta::pixels(units(3.0, -5.0)).to_px(LINE, PAGE),
            px(3.0, -5.0)
        );
        let line = WheelDelta::new(units(1.0, 3.0), WheelDeltaMode::Line);
        assert_eq!(line.to_px(LINE, PAGE), px(40.0, 120.0));
        let page = WheelDelta::new(units(0.0, -1.0), WheelDeltaMode::Page);
        assert_eq!(page.to_px(LINE, PAGE), px(0.0, -600.0));
    }

    #[test]
    fn a_line_or_a_page_is_a_step_and_a_pixel_is_not() {
        assert!(!WheelDelta::pixels(units(0.0, 1.0)).is_step());
        assert!(WheelDelta::new(units(0.0, 1.0), WheelDeltaMode::Line).is_step());
        assert!(WheelDelta::new(units(0.0, 1.0), WheelDeltaMode::Page).is_step());
    }

    #[test]
    fn the_dom_modes_name_their_units() {
        assert_eq!(WheelDeltaMode::from_dom(0), WheelDeltaMode::Pixel);
        assert_eq!(WheelDeltaMode::from_dom(1), WheelDeltaMode::Line);
        assert_eq!(WheelDeltaMode::from_dom(2), WheelDeltaMode::Page);
        assert_eq!(WheelDeltaMode::from_dom(7), WheelDeltaMode::Pixel);
    }
}
