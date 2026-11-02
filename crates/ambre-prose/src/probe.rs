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
use ambre_core::{FracMs, Instant, PageLoad, TimeSpan};

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
    pub start: Instant<PageLoad>,
    pub end: Instant<PageLoad>,
}

impl Measure {
    /// The span from the start to the end.
    #[must_use]
    pub fn duration(&self) -> TimeSpan<FracMs> {
        self.end - self.start
    }
}

/// An input handled and not yet drawn.
#[derive(Clone, Copy, Debug)]
struct Undrawn {
    start: Instant<PageLoad>,
    /// Its effect waits on a layout: a split, a paste, anything the draft
    /// cannot hold. It is drawn by the first frame after the layout.
    needs_layout: Drawing,
}

ambre_core::define_flag! {
    /// Whether an input's effect waits on a layout to be drawn.
    enum Drawing {
        /// The next frame draws it.
        Ready,
        /// A layout must arrive first.
        AwaitsLayout,
    }
}

/// The keystrokes in flight, from their input to their frame.
#[derive(Debug, Default)]
pub struct KeyProbe {
    /// Draft entries awaiting their confirmation, with their input time and
    /// the frame count at that input.
    unconfirmed: Vec<(
        ambre_bus::app::prose::TextAck,
        Instant<PageLoad>,
        FrameNumber,
    )>,
    /// The animation frames begun so far.
    frame: FrameNumber,
    /// The frame counts of the confirmations since the last take.
    counts: Vec<FrameCount>,
    /// The highest acknowledgement number seen in the draft.
    seen: Option<ambre_bus::app::prose::TextAck>,
    undrawn: Vec<Undrawn>,
}

impl KeyProbe {
    /// An animation frame began. Called once per frame, before its answers
    /// are applied, whether or not it paints.
    pub fn frame(&mut self) {
        self.frame = self.frame.next();
    }

    /// The frame counts of the confirmations since the last call.
    pub fn take_counts(&mut self) -> Vec<FrameCount> {
        std::mem::take(&mut self.counts)
    }

    /// An input the view consumed, handled from `start` to `now`. The
    /// entries it added to `draft` are echoed now and wait on their
    /// confirmation. `dispatch` says whether a command left: one the draft
    /// does not hold is drawn only once its layout arrives.
    pub fn handled(
        &mut self,
        start: Instant<PageLoad>,
        now: Instant<PageLoad>,
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
            needs_layout: Drawing::from_bool(dispatch == Dispatch::Sent && fresh.is_empty()),
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
        now: Instant<PageLoad>,
        draft: &DraftBuffer,
        answer: Answer,
    ) -> Vec<Measure> {
        let pending: Vec<ambre_bus::app::prose::TextAck> = draft
            .entries()
            .filter(|&(_, confirmed)| confirmed == crate::ui::Confirmation::Pending)
            .map(|(seq, _)| seq)
            .collect();
        let mut measures = Vec::new();
        let frame = self.frame;
        let counts = &mut self.counts;
        self.unconfirmed.retain(|&(seq, start, at)| {
            if pending.contains(&seq) {
                return true;
            }
            counts.push(frame.since(at));
            measures.push(Measure {
                name: ProbeName::Confirm,
                start,
                end: now,
            });
            false
        });
        if answer == Answer::Layout {
            for input in &mut self.undrawn {
                input.needs_layout = Drawing::Ready;
            }
        }
        measures
    }

    /// A frame finished painting at `now`: every input it shows ends its
    /// caret measure.
    pub fn painted(&mut self, now: Instant<PageLoad>) -> Vec<Measure> {
        let mut measures = Vec::new();
        self.undrawn.retain(|input| {
            if input.needs_layout == Drawing::AwaitsLayout {
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

ambre_core::define_tally! {
    /// The animation frames a confirmation waited for.
    pub struct FrameCount;
}

/// How many animation frames began before a moment, counted by the probe.
/// It only grows, and holds at the top rather than wrap.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
struct FrameNumber(u64);

impl FrameNumber {
    /// The number once one more frame began.
    fn next(self) -> Self {
        Self(self.0.saturating_add(1))
    }

    /// The frames begun from `earlier` to this number, zero when `earlier`
    /// is not before it, held at the top of a tally.
    fn since(self, earlier: Self) -> FrameCount {
        let frames = self.0.saturating_sub(earlier.0);
        FrameCount::new(u32::try_from(frames).unwrap_or(u32::MAX))
    }
}

/// The median and the 95th percentile of a set of samples, in the unit the
/// samples carry.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Percentiles<T> {
    /// The sample at the middle rank.
    pub median: T,
    /// The sample at the 95th percentile rank.
    pub p95: T,
}

/// The median and the 95th percentile of `samples` sorted by `order`, or
/// `None` for none.
#[must_use]
pub fn percentiles<T: Copy + Default>(
    samples: &[T],
    order: fn(&T, &T) -> std::cmp::Ordering,
) -> Option<Percentiles<T>> {
    if samples.is_empty() {
        return None;
    }
    let mut sorted = samples.to_vec();
    sorted.sort_by(order);
    let at = |q: f64| {
        let rank = (q * (sorted.len() - 1) as f64).round() as usize;
        sorted.get(rank).copied().unwrap_or_default()
    };
    Some(Percentiles {
        median: at(0.5),
        p95: at(0.95),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ambre_bus::app::prose::BlockId;
    use ambre_bus::app::prose::DocChar;
    use ambre_core::index::Pos;
    use ambre_core::{FracMs, Instant, PageLoad, TimeSpan};

    fn block() -> BlockId {
        BlockId {
            peer: ambre_core::PeerId::new(1),
            counter: ambre_core::OpCounter::new(0),
        }
    }

    /// A typed key echoes when handled, confirms when acknowledged, and
    /// shows its caret on the next frame.
    #[test]
    fn a_typed_key_echoes_confirms_and_is_drawn() {
        let mut probe = KeyProbe::default();
        let mut draft = DraftBuffer::default();
        draft.push_str(
            ambre_bus::app::prose::TextAck::new(7),
            "a",
            &block(),
            Pos::<DocChar>::ZERO,
        );
        let echo = probe.handled(
            Instant::<PageLoad>::new(10.0),
            Instant::<PageLoad>::new(10.5),
            &draft,
            Dispatch::Sent,
        );
        assert_eq!(echo.len(), 1);
        assert_eq!(echo[0].name, ProbeName::Echo);
        assert!((echo[0].duration().get() - 0.5).abs() < 1e-9);

        let caret = probe.painted(Instant::<PageLoad>::new(14.0));
        assert_eq!(caret.len(), 1, "the draft shows the key on the next frame");
        assert_eq!(caret[0].name, ProbeName::Caret);

        assert!(
            probe
                .answered(Instant::<PageLoad>::new(15.0), &draft, Answer::Other)
                .is_empty(),
            "not yet confirmed"
        );
        draft.confirm(ambre_bus::app::prose::TextAck::new(7));
        let confirm = probe.answered(Instant::<PageLoad>::new(20.0), &draft, Answer::Other);
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
                    Instant::<PageLoad>::new(0.0),
                    Instant::<PageLoad>::new(1.0),
                    &draft,
                    Dispatch::Sent
                )
                .is_empty(),
            "no echo without a draft"
        );
        assert!(
            probe.painted(Instant::<PageLoad>::new(5.0)).is_empty(),
            "the frame before the layout shows nothing"
        );
        probe.answered(Instant::<PageLoad>::new(30.0), &draft, Answer::Layout);
        let caret = probe.painted(Instant::<PageLoad>::new(33.0));
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
            ambre_bus::app::prose::TextAck::new(1),
            "a",
            &block(),
            Pos::<DocChar>::ZERO,
        );
        probe.handled(
            Instant::<PageLoad>::new(0.0),
            Instant::<PageLoad>::new(0.1),
            &draft,
            Dispatch::Sent,
        );
        draft.confirm(ambre_bus::app::prose::TextAck::new(1));
        assert_eq!(
            probe
                .answered(Instant::<PageLoad>::new(0.5), &draft, Answer::Other)
                .len(),
            1
        );
        draft.push_str(
            ambre_bus::app::prose::TextAck::new(2),
            "b",
            &block(),
            Pos::<DocChar>::new(1),
        );
        probe.handled(
            Instant::<PageLoad>::new(1.0),
            Instant::<PageLoad>::new(1.1),
            &draft,
            Dispatch::Sent,
        );
        assert_eq!(
            probe
                .answered(Instant::<PageLoad>::new(1.5), &draft, Answer::Other)
                .len(),
            0,
            "a confirmed entry is not new"
        );
        draft.confirm(ambre_bus::app::prose::TextAck::new(2));
        draft.laid();
        assert_eq!(
            probe
                .answered(Instant::<PageLoad>::new(9.0), &draft, Answer::Layout)
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
            ambre_bus::app::prose::TextAck::new(1),
            "a",
            &block(),
            Pos::<DocChar>::ZERO,
        );
        probe.handled(
            Instant::<PageLoad>::new(0.0),
            Instant::<PageLoad>::new(0.1),
            &draft,
            Dispatch::Sent,
        );
        draft.push_str(
            ambre_bus::app::prose::TextAck::new(2),
            "b",
            &block(),
            Pos::<DocChar>::new(1),
        );
        probe.handled(
            Instant::<PageLoad>::new(1.0),
            Instant::<PageLoad>::new(1.1),
            &draft,
            Dispatch::Sent,
        );
        draft.confirm(ambre_bus::app::prose::TextAck::new(1));
        draft.confirm(ambre_bus::app::prose::TextAck::new(2));
        draft.laid();
        let confirm = probe.answered(Instant::<PageLoad>::new(9.0), &draft, Answer::Layout);
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
                    Instant::<PageLoad>::new(0.0),
                    Instant::<PageLoad>::new(0.2),
                    &draft,
                    Dispatch::Kept
                )
                .is_empty()
        );
        assert_eq!(probe.painted(Instant::<PageLoad>::new(4.0)).len(), 1);
    }

    /// The count is the frames begun between the key and its answer: the
    /// frame that echoes it, then the one that applies the answer.
    #[test]
    fn a_confirmation_counts_the_frames_it_waited() {
        let mut probe = KeyProbe::default();
        let mut draft = DraftBuffer::default();
        probe.frame();
        draft.push_str(
            ambre_bus::app::prose::TextAck::new(1),
            "a",
            &block(),
            Pos::<DocChar>::ZERO,
        );
        probe.handled(
            Instant::<PageLoad>::new(0.0),
            Instant::<PageLoad>::new(0.1),
            &draft,
            Dispatch::Sent,
        );
        probe.frame();
        probe.answered(Instant::<PageLoad>::new(16.0), &draft, Answer::Other);
        assert!(
            probe.take_counts().is_empty(),
            "the echo frame has no answer"
        );
        probe.frame();
        draft.confirm(ambre_bus::app::prose::TextAck::new(1));
        probe.answered(Instant::<PageLoad>::new(33.0), &draft, Answer::Other);
        assert_eq!(probe.take_counts(), [FrameCount::new(2)]);
        assert!(probe.take_counts().is_empty(), "a count is taken once");
    }

    /// The frame number holds at its top. A bare `+= 1` panicked there under
    /// overflow checks, and a panic kills the worker.
    #[test]
    fn a_frame_begun_at_the_top_of_the_count_holds_there() {
        let mut probe = KeyProbe {
            frame: FrameNumber(u64::MAX),
            ..KeyProbe::default()
        };
        probe.frame();
        assert_eq!(probe.frame, FrameNumber(u64::MAX));
        assert_eq!(FrameNumber(1).since(FrameNumber(3)), FrameCount::ZERO);
        assert_eq!(
            FrameNumber(u64::MAX).since(FrameNumber(0)),
            FrameCount::new(u32::MAX)
        );
    }

    #[test]
    fn percentiles_read_the_sorted_samples() {
        assert_eq!(percentiles(&[], TimeSpan::<FracMs>::total_cmp), None);
        let spans: Vec<TimeSpan<FracMs>> = (1..=100)
            .map(|ms| TimeSpan::<FracMs>::new(f64::from(ms)))
            .rev()
            .collect();
        assert_eq!(
            percentiles(&spans, TimeSpan::<FracMs>::total_cmp),
            Some(Percentiles {
                median: TimeSpan::<FracMs>::new(51.0),
                p95: TimeSpan::<FracMs>::new(95.0)
            })
        );
        let frames: Vec<FrameCount> = (1..=100).rev().map(FrameCount::new).collect();
        assert_eq!(
            percentiles(&frames, FrameCount::cmp),
            Some(Percentiles {
                median: FrameCount::new(51),
                p95: FrameCount::new(95)
            })
        );
    }
}
