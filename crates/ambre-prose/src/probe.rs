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

use ambre_bus::app::prose::ProseEvent;

use crate::ui::DraftBuffer;

/// Whether `event` lays the document out again, which is when a moved caret
/// can first be drawn. The host passes the answer to [`KeyProbe::answered`].
pub fn lays_out(event: &ProseEvent) -> bool {
    matches!(event, ProseEvent::LayoutShaped { .. })
}

/// Whether a handled input sent a command to the Logic thread.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dispatch {
    /// Nothing left: the view acted alone.
    Kept,
    /// A command left.
    Sent,
}

/// Whether an event from the Logic thread laid the document out again.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Answer {
    Other,
    Layout,
}

/// A name the probes stamp on the browser's timeline.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ProbeName {
    /// The echo measure: input to draft updated.
    Echo,
    /// The confirmation measure: input to the Logic thread's acknowledgement
    /// reaching the view.
    Confirm,
    /// The confirmation count: the animation frames that began between the
    /// input and its acknowledgement reaching the view. A count, not a
    /// duration, so it holds at any refresh rate.
    ConfirmFrames,
    /// The caret measure: input to the first frame that shows it.
    Caret,
    /// The relay mark: the main thread took Logic's answers off their
    /// channel. A mark, not a measure: it splits a confirmation in two, the
    /// wait for the relay and the wait for the view.
    Relay,
    /// The draft shaping measure: the paragraphs an edit in flight changes,
    /// shaped on the UI thread for one frame (`PROSE-UI-215`).
    Shape,
}

impl ProbeName {
    /// The name as the profiler shows it.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Echo => "ambre:echo",
            Self::Confirm => "ambre:confirm",
            Self::ConfirmFrames => "ambre:confirm-frames",
            Self::Caret => "ambre:caret",
            Self::Relay => "ambre:relay",
            Self::Shape => "ambre:shape-draft",
        }
    }
}

/// One span on the timeline, in milliseconds of the host's clock.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Measure {
    pub name: ProbeName,
    pub start: ambre_core::PageTime,
    pub end: ambre_core::PageTime,
}

impl Measure {
    /// The span from the start to the end.
    #[must_use]
    pub fn duration(&self) -> ambre_core::PageMillis {
        self.end - self.start
    }
}

/// An input handled and not yet drawn.
#[derive(Clone, Copy, Debug)]
struct Undrawn {
    start: ambre_core::PageTime,
    /// Its effect waits on a layout: a split, a paste, anything the draft
    /// cannot hold. It is drawn by the first frame after the layout.
    needs_layout: bool,
}

/// The keystrokes in flight, from their input to their frame.
#[derive(Debug, Default)]
pub struct KeyProbe {
    /// Draft entries awaiting their confirmation, with their input time and
    /// the frame count at that input.
    unconfirmed: Vec<(ambre_bus::app::prose::TextAck, ambre_core::PageTime, u64)>,
    /// The animation frames begun so far.
    frame: u64,
    /// The frame counts of the confirmations since the last take.
    counts: Vec<u32>,
    /// The highest acknowledgement number seen in the draft.
    seen: Option<ambre_bus::app::prose::TextAck>,
    undrawn: Vec<Undrawn>,
}

impl KeyProbe {
    /// An animation frame began. Called once per frame, before its answers
    /// are applied, whether or not it paints.
    pub fn frame(&mut self) {
        self.frame += 1;
    }

    /// The frame counts of the confirmations since the last call.
    pub fn take_counts(&mut self) -> Vec<u32> {
        std::mem::take(&mut self.counts)
    }

    /// An input the view consumed, handled from `start` to `now`. The
    /// entries it added to `draft` are echoed now and wait on their
    /// confirmation. `dispatch` says whether a command left: one the draft
    /// does not hold is drawn only once its layout arrives.
    pub fn handled(
        &mut self,
        start: ambre_core::PageTime,
        now: ambre_core::PageTime,
        draft: &DraftBuffer,
        dispatch: Dispatch,
    ) -> Vec<Measure> {
        // Acknowledgement numbers only grow, so an entry above the last one
        // seen is one this input added.
        let fresh: Vec<ambre_bus::app::prose::TextAck> = draft
            .entries()
            .map(|(seq, _)| seq)
            .filter(|&seq| self.seen.is_none_or(|seen| seq > seen))
            .collect();
        if let Some(&last) = fresh.iter().max() {
            self.seen = Some(last);
        }
        self.unconfirmed
            .extend(fresh.iter().map(|&seq| (seq, start, self.frame)));
        self.undrawn.push(Undrawn {
            start,
            needs_layout: dispatch == Dispatch::Sent && fresh.is_empty(),
        });
        if fresh.is_empty() {
            return Vec::new();
        }
        vec![Measure {
            name: ProbeName::Echo,
            start,
            end: now,
        }]
    }

    /// An event from the Logic thread was applied at `now`. An entry that
    /// is confirmed, or already gone from the draft, ends its measure.
    pub fn answered(
        &mut self,
        now: ambre_core::PageTime,
        draft: &DraftBuffer,
        answer: Answer,
    ) -> Vec<Measure> {
        let pending: Vec<ambre_bus::app::prose::TextAck> = draft
            .entries()
            .filter(|&(_, confirmed)| !confirmed)
            .map(|(seq, _)| seq)
            .collect();
        let mut measures = Vec::new();
        let frame = self.frame;
        let counts = &mut self.counts;
        self.unconfirmed.retain(|&(seq, start, at)| {
            if pending.contains(&seq) {
                return true;
            }
            counts.push(u32::try_from(frame - at).unwrap_or(u32::MAX));
            measures.push(Measure {
                name: ProbeName::Confirm,
                start,
                end: now,
            });
            false
        });
        if answer == Answer::Layout {
            for input in &mut self.undrawn {
                input.needs_layout = false;
            }
        }
        measures
    }

    /// A frame finished painting at `now`: every input it shows ends its
    /// caret measure.
    pub fn painted(&mut self, now: ambre_core::PageTime) -> Vec<Measure> {
        let mut measures = Vec::new();
        self.undrawn.retain(|input| {
            if input.needs_layout {
                return true;
            }
            measures.push(Measure {
                name: ProbeName::Caret,
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
        sorted.get(rank).copied().unwrap_or_default()
    };
    Some((at(0.5), at(0.95)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ambre_bus::app::prose::BlockId;
    use ambre_bus::app::prose::DocOffset;

    fn block() -> BlockId {
        BlockId {
            peer: ambre_core::PeerId(1),
            counter: 0,
        }
    }

    /// A typed key echoes when handled, confirms when acknowledged, and
    /// shows its caret on the next frame.
    #[test]
    fn a_typed_key_echoes_confirms_and_is_drawn() {
        let mut probe = KeyProbe::default();
        let mut draft = DraftBuffer::default();
        draft.push_str(
            ambre_bus::app::prose::TextAck(7),
            "a",
            &block(),
            DocOffset::ZERO,
        );
        let echo = probe.handled(
            ambre_core::PageTime(10.0),
            ambre_core::PageTime(10.5),
            &draft,
            Dispatch::Sent,
        );
        assert_eq!(echo.len(), 1);
        assert_eq!(echo[0].name, ProbeName::Echo);
        assert!((echo[0].duration().get() - 0.5).abs() < 1e-9);

        let caret = probe.painted(ambre_core::PageTime(14.0));
        assert_eq!(caret.len(), 1, "the draft shows the key on the next frame");
        assert_eq!(caret[0].name, ProbeName::Caret);

        assert!(
            probe
                .answered(ambre_core::PageTime(15.0), &draft, Answer::Other)
                .is_empty(),
            "not yet confirmed"
        );
        draft.confirm(ambre_bus::app::prose::TextAck(7));
        let confirm = probe.answered(ambre_core::PageTime(20.0), &draft, Answer::Other);
        assert_eq!(confirm.len(), 1);
        assert_eq!(confirm[0].name, ProbeName::Confirm);
        assert!((confirm[0].duration().get() - 10.0).abs() < 1e-9);
    }

    /// An input the draft cannot hold waits for the layout before its caret
    /// counts as drawn, and has no echo of its own.
    #[test]
    fn a_structural_input_is_drawn_after_its_layout() {
        let mut probe = KeyProbe::default();
        let draft = DraftBuffer::default();
        assert!(
            probe
                .handled(
                    ambre_core::PageTime(0.0),
                    ambre_core::PageTime(1.0),
                    &draft,
                    Dispatch::Sent
                )
                .is_empty(),
            "no echo without a draft"
        );
        assert!(
            probe.painted(ambre_core::PageTime(5.0)).is_empty(),
            "the frame before the layout shows nothing"
        );
        probe.answered(ambre_core::PageTime(30.0), &draft, Answer::Layout);
        let caret = probe.painted(ambre_core::PageTime(33.0));
        assert_eq!(caret.len(), 1);
        assert!((caret[0].duration().get() - 33.0).abs() < 1e-9);
    }

    /// An entry confirmed and still in the draft is not counted again when
    /// the next key lands.
    #[test]
    fn a_confirmed_entry_is_counted_once() {
        let mut probe = KeyProbe::default();
        let mut draft = DraftBuffer::default();
        draft.push_str(
            ambre_bus::app::prose::TextAck(1),
            "a",
            &block(),
            DocOffset::ZERO,
        );
        probe.handled(
            ambre_core::PageTime(0.0),
            ambre_core::PageTime(0.1),
            &draft,
            Dispatch::Sent,
        );
        draft.confirm(ambre_bus::app::prose::TextAck(1));
        assert_eq!(
            probe
                .answered(ambre_core::PageTime(0.5), &draft, Answer::Other)
                .len(),
            1
        );
        draft.push_str(
            ambre_bus::app::prose::TextAck(2),
            "b",
            &block(),
            DocOffset::new(1),
        );
        probe.handled(
            ambre_core::PageTime(1.0),
            ambre_core::PageTime(1.1),
            &draft,
            Dispatch::Sent,
        );
        assert_eq!(
            probe
                .answered(ambre_core::PageTime(1.5), &draft, Answer::Other)
                .len(),
            0,
            "a confirmed entry is not new"
        );
        draft.confirm(ambre_bus::app::prose::TextAck(2));
        draft.laid();
        assert_eq!(
            probe
                .answered(ambre_core::PageTime(9.0), &draft, Answer::Layout)
                .len(),
            1
        );
    }

    /// A burst laid in one pass ends every measure, even when the probe
    /// never saw an entry confirmed before it left the draft.
    #[test]
    fn a_burst_laid_at_once_ends_every_measure() {
        let mut probe = KeyProbe::default();
        let mut draft = DraftBuffer::default();
        draft.push_str(
            ambre_bus::app::prose::TextAck(1),
            "a",
            &block(),
            DocOffset::ZERO,
        );
        probe.handled(
            ambre_core::PageTime(0.0),
            ambre_core::PageTime(0.1),
            &draft,
            Dispatch::Sent,
        );
        draft.push_str(
            ambre_bus::app::prose::TextAck(2),
            "b",
            &block(),
            DocOffset::new(1),
        );
        probe.handled(
            ambre_core::PageTime(1.0),
            ambre_core::PageTime(1.1),
            &draft,
            Dispatch::Sent,
        );
        draft.confirm(ambre_bus::app::prose::TextAck(1));
        draft.confirm(ambre_bus::app::prose::TextAck(2));
        draft.laid();
        let confirm = probe.answered(ambre_core::PageTime(9.0), &draft, Answer::Layout);
        assert_eq!(confirm.len(), 2);
    }

    /// A caret move sends nothing, so the next frame draws it.
    #[test]
    fn a_caret_move_is_drawn_by_the_next_frame() {
        let mut probe = KeyProbe::default();
        let draft = DraftBuffer::default();
        assert!(
            probe
                .handled(
                    ambre_core::PageTime(0.0),
                    ambre_core::PageTime(0.2),
                    &draft,
                    Dispatch::Kept
                )
                .is_empty()
        );
        assert_eq!(probe.painted(ambre_core::PageTime(4.0)).len(), 1);
    }

    /// The count is the frames begun between the key and its answer: the
    /// frame that echoes it, then the one that applies the answer.
    #[test]
    fn a_confirmation_counts_the_frames_it_waited() {
        let mut probe = KeyProbe::default();
        let mut draft = DraftBuffer::default();
        probe.frame();
        draft.push_str(
            ambre_bus::app::prose::TextAck(1),
            "a",
            &block(),
            DocOffset::ZERO,
        );
        probe.handled(
            ambre_core::PageTime(0.0),
            ambre_core::PageTime(0.1),
            &draft,
            Dispatch::Sent,
        );
        probe.frame();
        probe.answered(ambre_core::PageTime(16.0), &draft, Answer::Other);
        assert!(
            probe.take_counts().is_empty(),
            "the echo frame has no answer"
        );
        probe.frame();
        draft.confirm(ambre_bus::app::prose::TextAck(1));
        probe.answered(ambre_core::PageTime(33.0), &draft, Answer::Other);
        assert_eq!(probe.take_counts(), [2]);
        assert!(probe.take_counts().is_empty(), "a count is taken once");
    }

    #[test]
    fn percentiles_read_the_sorted_samples() {
        assert_eq!(percentiles(&[]), None);
        let samples: Vec<f64> = (1..=100).map(f64::from).rev().collect();
        assert_eq!(percentiles(&samples), Some((51.0, 95.0)));
    }
}
