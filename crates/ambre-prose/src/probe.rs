//! The keystroke latency probes (`1-Foundations/ambre-spec-budgets.md`
//! section 1).
//!
//! Three budgets govern typing: the local echo, the confirmation and the
//! moved caret. This module turns the times the host reads into those three
//! measures, and owns no clock. The host stamps each one on the browser's
//! timeline, where the profiler shows it.
//!
//! Every time is read on the UI thread, the confirmation included: it ends
//! when the answer reaches the view. No clock crosses a thread.

use crate::ui::DraftBuffer;

/// The name of the echo measure: input to draft updated.
pub const ECHO: &str = "ambre:echo";
/// The name of the confirmation measure: input to the Logic thread's
/// acknowledgement reaching the view.
pub const CONFIRM: &str = "ambre:confirm";
/// The name of the caret measure: input to the first frame that shows it.
pub const CARET: &str = "ambre:caret";

/// The name of the relay mark: the main thread took Logic's answers off
/// their channel. A mark, not a measure: it splits a confirmation in two,
/// the wait for the relay and the wait for the view.
pub const RELAY: &str = "ambre:relay";

/// The name of the draft shaping measure: the paragraphs an edit in flight
/// changes, shaped on the UI thread for one frame (`PROSE-UI-215`).
pub const SHAPE: &str = "ambre:shape-draft";

/// One span on the timeline, in milliseconds of the host's clock.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Measure {
    pub name: &'static str,
    pub start: f64,
    pub end: f64,
}

impl Measure {
    #[must_use]
    pub fn duration(&self) -> f64 {
        self.end - self.start
    }
}

/// An input handled and not yet drawn.
#[derive(Clone, Copy, Debug)]
struct Undrawn {
    start: f64,
    /// Its effect waits on a layout: a split, a paste, anything the draft
    /// cannot hold. It is drawn by the first frame after the layout.
    needs_layout: bool,
}

/// The keystrokes in flight, from their input to their frame.
#[derive(Debug, Default)]
pub struct KeyProbe {
    /// Draft entries awaiting their confirmation, with their input time.
    unconfirmed: Vec<(u64, f64)>,
    /// The highest acknowledgement number seen in the draft.
    seen: Option<u64>,
    undrawn: Vec<Undrawn>,
}

impl KeyProbe {
    /// An input the view consumed, handled from `start` to `now`. The
    /// entries it added to `draft` are echoed now and wait on their
    /// confirmation. `sent` says whether a command left: one the draft does
    /// not hold is drawn only once its layout arrives.
    pub fn handled(
        &mut self,
        start: f64,
        now: f64,
        draft: &DraftBuffer,
        sent: bool,
    ) -> Vec<Measure> {
        // Acknowledgement numbers only grow, so an entry above the last one
        // seen is one this input added.
        let fresh: Vec<u64> = draft
            .entries()
            .map(|(seq, _)| seq)
            .filter(|&seq| self.seen.is_none_or(|seen| seq > seen))
            .collect();
        if let Some(&last) = fresh.iter().max() {
            self.seen = Some(last);
        }
        self.unconfirmed
            .extend(fresh.iter().map(|&seq| (seq, start)));
        self.undrawn.push(Undrawn {
            start,
            needs_layout: sent && fresh.is_empty(),
        });
        if fresh.is_empty() {
            return Vec::new();
        }
        vec![Measure {
            name: ECHO,
            start,
            end: now,
        }]
    }

    /// An event from the Logic thread was applied at `now`. An entry that
    /// is confirmed, or already gone from the draft, ends its measure.
    pub fn answered(&mut self, now: f64, draft: &DraftBuffer, laid: bool) -> Vec<Measure> {
        let pending: Vec<u64> = draft
            .entries()
            .filter(|&(_, confirmed)| !confirmed)
            .map(|(seq, _)| seq)
            .collect();
        let mut measures = Vec::new();
        self.unconfirmed.retain(|&(seq, start)| {
            if pending.contains(&seq) {
                return true;
            }
            measures.push(Measure {
                name: CONFIRM,
                start,
                end: now,
            });
            false
        });
        if laid {
            for input in &mut self.undrawn {
                input.needs_layout = false;
            }
        }
        measures
    }

    /// A frame finished painting at `now`: every input it shows ends its
    /// caret measure.
    pub fn painted(&mut self, now: f64) -> Vec<Measure> {
        let mut measures = Vec::new();
        self.undrawn.retain(|input| {
            if input.needs_layout {
                return true;
            }
            measures.push(Measure {
                name: CARET,
                start: input.start,
                end: now,
            });
            false
        });
        measures
    }
}

/// The median and the 95th percentile of `samples`, or `None` for none.
#[must_use]
pub fn percentiles(samples: &[f64]) -> Option<(f64, f64)> {
    if samples.is_empty() {
        return None;
    }
    let mut sorted = samples.to_vec();
    sorted.sort_by(f64::total_cmp);
    let at = |q: f64| {
        let rank = (q * (sorted.len() - 1) as f64).round() as usize;
        sorted[rank]
    };
    Some((at(0.5), at(0.95)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ambre_bus::app::prose::BlockId;

    fn block() -> BlockId {
        BlockId {
            peer: 1,
            counter: 0,
        }
    }

    /// A typed key echoes when handled, confirms when acknowledged, and
    /// shows its caret on the next frame.
    #[test]
    fn a_typed_key_echoes_confirms_and_is_drawn() {
        let mut probe = KeyProbe::default();
        let mut draft = DraftBuffer::default();
        draft.push_str(7, "a", &block(), 0);
        let echo = probe.handled(10.0, 10.5, &draft, true);
        assert_eq!(echo.len(), 1);
        assert_eq!(echo[0].name, ECHO);
        assert!((echo[0].duration() - 0.5).abs() < 1e-9);

        let caret = probe.painted(14.0);
        assert_eq!(caret.len(), 1, "the draft shows the key on the next frame");
        assert_eq!(caret[0].name, CARET);

        assert!(
            probe.answered(15.0, &draft, false).is_empty(),
            "not yet confirmed"
        );
        draft.confirm(7);
        let confirm = probe.answered(20.0, &draft, false);
        assert_eq!(confirm.len(), 1);
        assert_eq!(confirm[0].name, CONFIRM);
        assert!((confirm[0].duration() - 10.0).abs() < 1e-9);
    }

    /// An input the draft cannot hold waits for the layout before its caret
    /// counts as drawn, and has no echo of its own.
    #[test]
    fn a_structural_input_is_drawn_after_its_layout() {
        let mut probe = KeyProbe::default();
        let draft = DraftBuffer::default();
        assert!(
            probe.handled(0.0, 1.0, &draft, true).is_empty(),
            "no echo without a draft"
        );
        assert!(
            probe.painted(5.0).is_empty(),
            "the frame before the layout shows nothing"
        );
        probe.answered(30.0, &draft, true);
        let caret = probe.painted(33.0);
        assert_eq!(caret.len(), 1);
        assert!((caret[0].duration() - 33.0).abs() < 1e-9);
    }

    /// An entry confirmed and still in the draft is not counted again when
    /// the next key lands.
    #[test]
    fn a_confirmed_entry_is_counted_once() {
        let mut probe = KeyProbe::default();
        let mut draft = DraftBuffer::default();
        draft.push_str(1, "a", &block(), 0);
        probe.handled(0.0, 0.1, &draft, true);
        draft.confirm(1);
        assert_eq!(probe.answered(0.5, &draft, false).len(), 1);
        draft.push_str(2, "b", &block(), 1);
        probe.handled(1.0, 1.1, &draft, true);
        assert_eq!(
            probe.answered(1.5, &draft, false).len(),
            0,
            "a confirmed entry is not new"
        );
        draft.confirm(2);
        draft.laid();
        assert_eq!(probe.answered(9.0, &draft, true).len(), 1);
    }

    /// A burst laid in one pass ends every measure, even when the probe
    /// never saw an entry confirmed before it left the draft.
    #[test]
    fn a_burst_laid_at_once_ends_every_measure() {
        let mut probe = KeyProbe::default();
        let mut draft = DraftBuffer::default();
        draft.push_str(1, "a", &block(), 0);
        probe.handled(0.0, 0.1, &draft, true);
        draft.push_str(2, "b", &block(), 1);
        probe.handled(1.0, 1.1, &draft, true);
        draft.confirm(1);
        draft.confirm(2);
        draft.laid();
        let confirm = probe.answered(9.0, &draft, true);
        assert_eq!(confirm.len(), 2);
    }

    /// A caret move sends nothing, so the next frame draws it.
    #[test]
    fn a_caret_move_is_drawn_by_the_next_frame() {
        let mut probe = KeyProbe::default();
        let draft = DraftBuffer::default();
        assert!(probe.handled(0.0, 0.2, &draft, false).is_empty());
        assert_eq!(probe.painted(4.0).len(), 1);
    }

    #[test]
    fn percentiles_read_the_sorted_samples() {
        assert_eq!(percentiles(&[]), None);
        let samples: Vec<f64> = (1..=100).map(f64::from).rev().collect();
        assert_eq!(percentiles(&samples), Some((51.0, 95.0)));
    }
}
