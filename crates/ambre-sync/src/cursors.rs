//! The read cursors this session owes the relay, sent together once a period
//! (the Murmure unread spec, `SYNC-269`).
//!
//! A member who keeps a channel on screen marks it read at every message it
//! shows. One request per mark spends the REST budget of the relay at a few
//! messages a second. The relay then refuses requests that carry content.
//! So each move waits in this map until the period ends, and a later move of
//! the same document replaces it.

use std::collections::HashMap;

use ambre_core::id::{Document, Id};
use ambre_core::{Ms, Relay, Seq, TimeSpan};

/// How long the moves of the read cursors wait, so one request per document
/// carries them all: the row of `ambre-spec-budgets.md section 8`.
pub(crate) const CURSOR_PERIOD: TimeSpan<Ms> = TimeSpan::<Ms>::new(1_000);

/// Whether a period runs: the moves owed then leave when its timer fires.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Period {
    /// No move is owed, so no timer runs.
    #[default]
    Idle,
    /// A timer runs, and the moves owed leave when it fires.
    Running,
}

/// The cursors owed, by document, each at the number its latest move named.
#[derive(Debug, Default)]
pub(crate) struct OwedCursors {
    owed: HashMap<Id<Document>, Seq<Relay>>,
    period: Period,
}

impl OwedCursors {
    /// Owe each of `cursors`. The latest move of a document replaces the one
    /// it owed, back as well as forward, as the move itself would. Returns
    /// [`Period::Idle`] when no period ran: the caller then starts one.
    pub(crate) fn owe(&mut self, cursors: Vec<(Id<Document>, Seq<Relay>)>) -> Period {
        if cursors.is_empty() {
            return Period::Running;
        }
        self.owed.extend(cursors);
        std::mem::replace(&mut self.period, Period::Running)
    }

    /// The period ended: every move owed, one per document, to send now.
    pub(crate) fn take(&mut self) -> Vec<(Id<Document>, Seq<Relay>)> {
        self.period = Period::Idle;
        self.owed.drain().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `SYNC-269`: the moves of one period leave as one request per
    /// document, at the latest number each named.
    #[test]
    fn the_moves_of_one_period_leave_once_per_document_at_the_latest_number() {
        let (segment, thread) = (Id::<Document>::mint(), Id::<Document>::mint());
        let mut owed = OwedCursors::default();
        assert_eq!(
            owed.owe(vec![(segment, Seq::<Relay>::new(1))]),
            Period::Idle,
            "the first move starts the period"
        );
        for n in 2..=60 {
            let started = owed.owe(vec![(segment, Seq::<Relay>::new(n))]);
            assert_eq!(started, Period::Running, "one timer per period");
        }
        owed.owe(vec![(thread, Seq::<Relay>::new(4))]);
        let mut sent = owed.take();
        sent.sort_by_key(|(_, seq)| *seq);
        assert_eq!(
            sent,
            vec![
                (thread, Seq::<Relay>::new(4)),
                (segment, Seq::<Relay>::new(60))
            ]
        );
        assert!(owed.take().is_empty(), "nothing is sent twice");
        assert_eq!(
            owed.owe(vec![(segment, Seq::<Relay>::new(61))]),
            Period::Idle,
            "the next move starts the next period"
        );
    }

    /// `SYNC-269`: a move back, as marking a message unread asks, replaces
    /// a move forward of the same period.
    #[test]
    fn a_move_back_replaces_a_move_forward_of_the_same_period() {
        let segment = Id::<Document>::mint();
        let mut owed = OwedCursors::default();
        owed.owe(vec![(segment, Seq::<Relay>::new(9))]);
        owed.owe(vec![(segment, Seq::<Relay>::new(3))]);
        assert_eq!(owed.take(), vec![(segment, Seq::<Relay>::new(3))]);
    }

    /// An empty move owes nothing and starts no period.
    #[test]
    fn an_empty_move_starts_no_period() {
        let mut owed = OwedCursors::default();
        owed.owe(Vec::new());
        assert!(owed.take().is_empty());
    }
}
