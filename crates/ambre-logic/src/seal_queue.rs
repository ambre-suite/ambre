//! The order of the local store's writes while their rows are sealed
//! (`ambre-spec-residency.md` section 3).
//!
//! A snapshot or a delta reaches the store sealed, and its seal is a round
//! trip to the Sync worker (`RESID-025`). The writes behind it wait their
//! turn, so the store sees them in the order Logic made them. A delta never
//! lands under a snapshot that holds it, and a read back comes after every
//! write before it (`RESID-010`).

use std::collections::VecDeque;

use ambre_bus::DocumentId;

use crate::PersistWrite;

/// The answer one seal waits for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SealTicket(u64);

/// The writes on their way to the store, in the order Logic made them.
#[derive(Debug, Default)]
pub struct SealQueue {
    next: u64,
    entries: VecDeque<Entry>,
}

#[derive(Debug)]
struct Entry {
    document_id: DocumentId,
    write: PersistWrite,
    /// The seal it waits for. `None` once it may go.
    waiting: Option<SealTicket>,
}

impl SealQueue {
    /// Queue one write. A snapshot or a delta waits for its seal. Its
    /// plaintext comes back with the ticket the seal answers, and the queue
    /// keeps none of it. Any other write holds no content and waits only for
    /// the writes ahead of it.
    pub fn push(
        &mut self,
        document_id: DocumentId,
        mut write: PersistWrite,
    ) -> Option<(SealTicket, Vec<u8>)> {
        let plaintext = match &mut write {
            PersistWrite::Snapshot(bytes) | PersistWrite::Delta(bytes) => {
                Some(std::mem::take(bytes))
            }
            _ => None,
        };
        let seal = plaintext.map(|plaintext| {
            let ticket = SealTicket(self.next);
            self.next += 1;
            (ticket, plaintext)
        });
        self.entries.push_back(Entry {
            document_id,
            write,
            waiting: seal.as_ref().map(|(ticket, _)| *ticket),
        });
        seal
    }

    /// The seal of `ticket` came back. `None` is a seal that did not happen,
    /// and its write is dropped: the store never holds a row in the clear.
    pub fn sealed(&mut self, ticket: SealTicket, sealed: Option<Vec<u8>>) {
        let Some(at) = self
            .entries
            .iter()
            .position(|entry| entry.waiting == Some(ticket))
        else {
            return;
        };
        let Some(sealed) = sealed else {
            self.entries.remove(at);
            return;
        };
        if let Some(entry) = self.entries.get_mut(at) {
            if let PersistWrite::Snapshot(bytes) | PersistWrite::Delta(bytes) = &mut entry.write {
                *bytes = sealed;
            }
            entry.waiting = None;
        }
    }

    /// The writes that may go now, in order: each one up to the first that
    /// still waits for its seal.
    pub fn ready(&mut self) -> Vec<(DocumentId, PersistWrite)> {
        let mut ready = Vec::new();
        while self
            .entries
            .front()
            .is_some_and(|entry| entry.waiting.is_none())
        {
            if let Some(entry) = self.entries.pop_front() {
                ready.push((entry.document_id, entry.write));
            }
        }
        ready
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ambre_bus::DocType;

    /// A write behind a seal waits for it, whatever it is. The store receives
    /// each in the order Logic made them, sealed bytes in place of plaintext.
    #[test]
    fn writes_reach_the_store_in_order_with_their_sealed_bytes() {
        let doc = DocumentId::new();
        let mut queue = SealQueue::default();
        let (first, plain) = queue
            .push(doc, PersistWrite::Snapshot(b"whole".to_vec()))
            .expect("a snapshot is sealed");
        assert_eq!(plain, b"whole");
        let (second, _) = queue
            .push(doc, PersistWrite::Delta(b"edit".to_vec()))
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
        queue.sealed(second, Some(b"sealed edit".to_vec()));
        assert!(queue.ready().is_empty());
        queue.sealed(first, Some(b"sealed whole".to_vec()));
        assert_eq!(
            queue.ready(),
            [
                (doc, PersistWrite::Snapshot(b"sealed whole".to_vec())),
                (doc, PersistWrite::Delta(b"sealed edit".to_vec())),
                (
                    doc,
                    PersistWrite::ReadBack {
                        doc_type: DocType::Prose
                    }
                ),
            ]
        );
    }

    /// A write with no content goes at once when nothing waits ahead of it.
    #[test]
    fn a_write_with_no_content_goes_at_once() {
        let doc = DocumentId::new();
        let mut queue = SealQueue::default();
        assert!(queue.push(doc, PersistWrite::Shown).is_none());
        assert_eq!(queue.ready(), [(doc, PersistWrite::Shown)]);
    }

    /// A seal that did not happen drops its write and nothing else: the store
    /// never receives the plaintext, and the writes behind it go.
    #[test]
    fn a_seal_that_did_not_happen_drops_only_its_write() {
        let doc = DocumentId::new();
        let mut queue = SealQueue::default();
        let (ticket, _) = queue
            .push(doc, PersistWrite::Delta(b"edit".to_vec()))
            .expect("a delta is sealed");
        queue.push(doc, PersistWrite::Hidden);
        queue.sealed(ticket, None);
        assert_eq!(queue.ready(), [(doc, PersistWrite::Hidden)]);
    }
}
