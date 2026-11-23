//! The call away from its channel: the bubble over the phone width, and the pill under it (the
//! Murmure calls spec, section 3).
//!
//! The call lives in the page, so it goes on while Murmure shows another channel, a thread or the
//! channel list (`MURMURE-701`). The page then shows the call where the member is. A bubble rests
//! in a corner of the content (`MURMURE-696`, `MURMURE-703`), and a pill sits under the header on a
//! phone (`MURMURE-697`). This module decides when it shows, where the bubble rests and what its clock
//! reads. The page draws it in its DOM.

use ambre_core::id::{Channel, Id};
use ambre_core::{FracMs, Px, Size, TimeSpan, View};

use crate::ui::call_controls::CallPhase;

/// Whether the page shows the call away from its channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Float {
    /// No call, or the channel region of the call is in view.
    Hidden,
    /// The call goes on away from its channel: the bubble, or the pill on a phone.
    Shown,
}

impl Float {
    /// Whether the call floats, from the phase of the call, the channel it lives in and the
    /// channel the view shows (`MURMURE-696`, `MURMURE-697`).
    #[must_use]
    pub fn of(phase: CallPhase, call: Option<Id<Channel>>, shown: Option<Id<Channel>>) -> Self {
        match (phase, call) {
            (CallPhase::InCall(_), Some(call)) if shown != Some(call) => Self::Shown,
            _ => Self::Hidden,
        }
    }
}

/// The corner of the content the bubble rests in (`MURMURE-703`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Corner {
    /// The top left corner.
    TopLeft,
    /// The top right corner.
    TopRight,
    /// The bottom left corner.
    BottomLeft,
    /// The bottom right corner, where the bubble first rests.
    #[default]
    BottomRight,
}

impl Corner {
    /// The corner nearest the centre `at` of a bubble released over a content of `area`.
    #[must_use]
    pub fn nearest(at: (Px<View>, Px<View>), area: Size<Px<View>>) -> Self {
        let left = at.0 < area.width / 2.0;
        let top = at.1 < area.height / 2.0;
        match (top, left) {
            (true, true) => Self::TopLeft,
            (true, false) => Self::TopRight,
            (false, true) => Self::BottomLeft,
            (false, false) => Self::BottomRight,
        }
    }

    /// The word of the corner in the markup, which the stylesheet places.
    #[must_use]
    pub const fn word(self) -> &'static str {
        match self {
            Self::TopLeft => "top-left",
            Self::TopRight => "top-right",
            Self::BottomLeft => "bottom-left",
            Self::BottomRight => "bottom-right",
        }
    }
}

/// The clock of the bubble and of the pill: the minutes and the seconds of `elapsed`, as
/// `mm:ss`, then `h:mm:ss` past an hour.
#[must_use]
pub fn clock(elapsed: TimeSpan<FracMs>) -> String {
    let seconds = (elapsed.get().max(0.0) / 1000.0).floor();
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "a call lasts far less than the seconds a u64 holds, and the floor is not negative"
    )]
    let seconds = seconds as u64;
    let (h, m, s) = (seconds / 3600, (seconds / 60) % 60, seconds % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m:02}:{s:02}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::call_controls::Microphone;

    /// `MURMURE-696`, `MURMURE-697`: the call floats while the page is in a call and shows
    /// another channel, a thread of another channel or the list. Its own channel in view, or no
    /// call, floats nothing.
    #[test]
    fn the_call_floats_away_from_its_channel_alone() {
        let (call, other) = (Id::<Channel>::mint(), Id::<Channel>::mint());
        let live = CallPhase::InCall(Microphone::Live);
        assert_eq!(Float::of(live, Some(call), Some(other)), Float::Shown);
        assert_eq!(Float::of(live, Some(call), None), Float::Shown);
        assert_eq!(Float::of(live, Some(call), Some(call)), Float::Hidden);
        assert_eq!(Float::of(CallPhase::Idle, Some(call), None), Float::Hidden);
        assert_eq!(Float::of(live, None, None), Float::Hidden);
    }

    /// `MURMURE-703`: a released bubble rests in the corner nearest its centre.
    #[test]
    fn a_released_bubble_rests_in_the_nearest_corner() {
        let area = Size::new(Px::<View>::new(1000.0), Px::<View>::new(600.0));
        let at = |x: f32, y: f32| (Px::<View>::new(x), Px::<View>::new(y));
        assert_eq!(Corner::nearest(at(100.0, 80.0), area), Corner::TopLeft);
        assert_eq!(Corner::nearest(at(900.0, 80.0), area), Corner::TopRight);
        assert_eq!(Corner::nearest(at(100.0, 500.0), area), Corner::BottomLeft);
        assert_eq!(Corner::nearest(at(700.0, 400.0), area), Corner::BottomRight);
        assert_eq!(Corner::default(), Corner::BottomRight);
        assert_eq!(Corner::TopLeft.word(), "top-left");
    }

    /// The clock reads minutes and seconds, then hours past an hour.
    #[test]
    fn the_clock_reads_minutes_and_seconds() {
        let ms = |v: f64| TimeSpan::<FracMs>::new(v);
        assert_eq!(clock(ms(0.0)), "00:00");
        assert_eq!(clock(ms(872_400.0)), "14:32");
        assert_eq!(clock(ms(3_725_000.0)), "1:02:05");
        assert_eq!(clock(ms(-5.0)), "00:00");
    }
}
