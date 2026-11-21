//! The order of the local store's writes while their rows are sealed
//! (`ambre-spec-residency.md` section 3).
//!
//! A snapshot or a delta reaches the store sealed, and its seal is a round
//! trip to the Sync worker (`RESID-025`). The writes behind it wait their
//! turn, so the store sees them in the order Logic made them. A delta never
//! lands under a snapshot that holds it, and a read back comes after every
//! write before it (`RESID-010`).

use std::collections::VecDeque;

use ambre_bus::{Confidential, Plaintext};
use ambre_core::id::{Document, Id};
use ambre_core::{Issuer, Payload, SealedRowBytes, Seq, SeqKind};

use crate::{PersistWrite, SealedWrite};

/// The answer one seal waits for.
///
/// The family is sealed: only this module builds a [`SealRight`], so only a
/// queue issues a ticket. No other crate can forge one that lands on a write
/// it did not seal.
pub enum SealTicket {}

/// The right to issue seal tickets. Its field is private, so a queue of this
/// module alone holds an issuer.
#[derive(Clone, Copy)]
pub struct SealRight(());

impl SeqKind for SealTicket {
    type IssueRight = SealRight;
    const NAME: ambre_core::StaticKey<ambre_core::DebugNames> =
        ambre_core::StaticKey::new("SealTicket");
}

/// The writes on their way to the store, in the order Logic made them.
#[derive(Debug)]
pub struct SealQueue {
    /// The source of the tickets: each seal has its own.
    tickets: Issuer<SealTicket>,
    entries: VecDeque<Entry>,
}

impl Default for SealQueue {
    fn default() -> Self {
        Self {
            tickets: Issuer::new(SealRight(())),
            entries: VecDeque::new(),
        }
    }
}

#[derive(Debug)]
struct Entry {
    document_id: Id<Document>,
    slot: Slot,
}

/// Where one write stands. A row waiting for its seal holds no bytes: the
/// plaintext left with the ticket, and the sealed row replaces the slot.
#[derive(Debug)]
enum Slot {
    Waiting {
        ticket: Seq<SealTicket>,
        row: RowKind,
    },
    Ready(SealedWrite),
}

/// The kind of row a seal answers for.
#[derive(Debug, Clone, Copy)]
enum RowKind {
    Snapshot,
    Delta,
}

impl SealQueue {
    /// Queue one write. A snapshot or a delta waits for its seal. Its
    /// plaintext comes back with the ticket the seal answers, and the queue
    /// keeps none of it. Any other write holds no content and waits only for
    /// the writes ahead of it.
    pub fn push(
        &mut self,
        document_id: Id<Document>,
        write: PersistWrite,
    ) -> Option<(Seq<SealTicket>, Confidential<Plaintext>)> {
        let (slot, plaintext) = match write {
            PersistWrite::Snapshot(bytes) => {
                let ticket = self.ticket();
                let row = RowKind::Snapshot;
                (
                    Slot::Waiting { ticket, row },
                    Some((ticket, bytes.into_bytes())),
                )
            }
            PersistWrite::Delta(bytes) => {
                let ticket = self.ticket();
                let row = RowKind::Delta;
                (
                    Slot::Waiting { ticket, row },
                    Some((ticket, bytes.into_bytes())),
                )
            }
            PersistWrite::Session { peer, nonce } => {
                (Slot::Ready(SealedWrite::Session { peer, nonce }), None)
            }
            PersistWrite::ForgetSession { peer } => {
                (Slot::Ready(SealedWrite::ForgetSession { peer }), None)
            }
            PersistWrite::ReadBack { doc_type } => {
                (Slot::Ready(SealedWrite::ReadBack { doc_type }), None)
            }
            PersistWrite::Shown => (Slot::Ready(SealedWrite::Shown), None),
            PersistWrite::Hidden => (Slot::Ready(SealedWrite::Hidden), None),
            PersistWrite::Sent => (Slot::Ready(SealedWrite::Sent), None),
            PersistWrite::Erase => (Slot::Ready(SealedWrite::Erase), None),
            PersistWrite::KeepInviter(pin) => (Slot::Ready(SealedWrite::KeepInviter(pin)), None),
            PersistWrite::KeepLog(kept) => (Slot::Ready(SealedWrite::KeepLog(kept)), None),
        };
        self.entries.push_back(Entry { document_id, slot });
        plaintext.map(|(ticket, bytes)| (ticket, Confidential::<Plaintext>::new(bytes)))
    }

    fn ticket(&mut self) -> Seq<SealTicket> {
        self.tickets.issue()
    }

    /// The seal of `ticket` came back. `None` is a seal that did not happen,
    /// and its write is dropped: the store never holds a row in the clear.
    pub fn sealed(&mut self, ticket: Seq<SealTicket>, sealed: Option<Payload<SealedRowBytes>>) {
        let Some(at) = self.entries.iter().position(
            |entry| matches!(entry.slot, Slot::Waiting { ticket: waits, .. } if waits == ticket),
        ) else {
            return;
        };
        let Some(sealed) = sealed else {
            self.entries.remove(at);
            return;
        };
        if let Some(entry) = self.entries.get_mut(at)
            && let Slot::Waiting { row, .. } = entry.slot
        {
            entry.slot = Slot::Ready(match row {
                RowKind::Snapshot => SealedWrite::Snapshot(sealed),
                RowKind::Delta => SealedWrite::Delta(sealed),
            });
        }
    }

    /// The writes that may go now, in order: each one up to the first that
    /// still waits for its seal.
    pub fn ready(&mut self) -> Vec<(Id<Document>, SealedWrite)> {
        let mut ready = Vec::new();
        while self
            .entries
            .front()
            .is_some_and(|entry| matches!(entry.slot, Slot::Ready(_)))
        {
            if let Some(Entry {
                document_id,
                slot: Slot::Ready(write),
            }) = self.entries.pop_front()
            {
                ready.push((document_id, write));
            }
        }
        ready
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ambre_bus::DocType;
    use ambre_core::{DeltaBytes, Payload, SealedRowBytes, SnapshotBytes};

    fn sealed(bytes: &[u8]) -> Payload<SealedRowBytes> {
        Payload::new(bytes.to_vec())
    }

    /// Each seal takes its own ticket, so no answer lands on another write.
    /// A new queue hands out three tickets in order, none the zero one.
    #[test]
    fn each_seal_takes_its_own_ticket() {
        let mut queue = SealQueue::default();
        let tickets = [queue.ticket(), queue.ticket(), queue.ticket()];
        assert!(Seq::ZERO < tickets[0]);
        assert!(tickets.windows(2).all(|pair| pair[0] < pair[1]));
    }

    /// A write behind a seal waits for it, whatever it is. The store receives
    /// each in the order Logic made them, sealed bytes in place of plaintext.
    #[test]
    fn writes_reach_the_store_in_order_with_their_sealed_bytes() {
        let doc = Id::<Document>::mint();
        let mut queue = SealQueue::default();
        let (first, plain) = queue
            .push(
                doc,
                PersistWrite::Snapshot(Payload::<SnapshotBytes>::new(b"whole".to_vec())),
            )
            .expect("a snapshot is sealed");
        assert_eq!(plain.into_plaintext(), b"whole");
        let (second, _) = queue
            .push(
                doc,
                PersistWrite::Delta(Payload::<DeltaBytes>::new(b"edit".to_vec())),
            )
            .expect("a delta is sealed");
        assert!(
            queue
                .push(
                    doc,
                    PersistWrite::ReadBack {
                        doc_type: DocType::Prose
                    }
                )
                .is_none()
        );
        assert!(queue.ready().is_empty(), "nothing goes before its seal");

        // The second seal answers first: still nothing may go.
        queue.sealed(second, Some(sealed(b"sealed edit")));
        assert!(queue.ready().is_empty());
        queue.sealed(first, Some(sealed(b"sealed whole")));
        assert_eq!(
            queue.ready(),
            [
                (doc, SealedWrite::Snapshot(sealed(b"sealed whole"))),
                (doc, SealedWrite::Delta(sealed(b"sealed edit"))),
                (
                    doc,
                    SealedWrite::ReadBack {
                        doc_type: DocType::Prose
                    }
                ),
            ]
        );
    }

    /// A seal answers with the row of the write that asked for it. A delta
    /// sealed comes back a delta, never a snapshot, whichever seal answers
    /// first.
    #[test]
    fn a_sealed_row_keeps_the_kind_of_its_write() {
        let doc = Id::<Document>::mint();
        let mut queue = SealQueue::default();
        let (delta, _) = queue
            .push(
                doc,
                PersistWrite::Delta(Payload::<DeltaBytes>::new(b"edit".to_vec())),
            )
            .expect("a delta is sealed");
        let (snapshot, _) = queue
            .push(
                doc,
                PersistWrite::Snapshot(Payload::<SnapshotBytes>::new(b"whole".to_vec())),
            )
            .expect("a snapshot is sealed");
        queue.sealed(snapshot, Some(sealed(b"s")));
        queue.sealed(delta, Some(sealed(b"d")));
        assert_eq!(
            queue.ready(),
            [
                (doc, SealedWrite::Delta(sealed(b"d"))),
                (doc, SealedWrite::Snapshot(sealed(b"s"))),
            ]
        );
    }

    /// A write with no content goes at once when nothing waits ahead of it.
    #[test]
    fn a_write_with_no_content_goes_at_once() {
        let doc = Id::<Document>::mint();
        let mut queue = SealQueue::default();
        assert!(queue.push(doc, PersistWrite::Shown).is_none());
        assert_eq!(queue.ready(), [(doc, SealedWrite::Shown)]);
    }

    /// A seal that did not happen drops its write and nothing else: the store
    /// never receives the plaintext, and the writes behind it go.
    #[test]
    fn a_seal_that_did_not_happen_drops_only_its_write() {
        let doc = Id::<Document>::mint();
        let mut queue = SealQueue::default();
        let (ticket, _) = queue
            .push(
                doc,
                PersistWrite::Delta(Payload::<DeltaBytes>::new(b"edit".to_vec())),
            )
            .expect("a delta is sealed");
        queue.push(doc, PersistWrite::Hidden);
        queue.sealed(ticket, None);
        assert_eq!(queue.ready(), [(doc, SealedWrite::Hidden)]);
    }
}
