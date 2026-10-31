//! What the local store holds, and which copies leave it
//! (`ambre-spec-residency.md` section 3).
//!
//! The Main thread owns the store and runs one ledger beside it. The ledger
//! reads no bytes from the store: it counts what each write costs, and knows
//! what Logic tells it about each document. It names the copies to delete so
//! the store stays within its budget, and decides what a refused write does.

use std::collections::HashMap;

use ambre_bus::DocumentId;
use ambre_core::ByteSize;
use ambre_core::Generation;
use ambre_core::residency::{Delivery, Held, Mutability, Residency, ViewTick};

use crate::SealedWrite;

/// The bytes the local store may hold on one client: the row "Local store,
/// per client" of `ambre-spec-budgets.md` section 6.
pub const LOCAL_STORE_BUDGET: ByteSize = ByteSize(500 << 20);

/// What one document costs in the store, by kind of row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct Cost {
    /// The bytes of its snapshot.
    snapshot: ByteSize,
    /// The bytes of the deltas logged since that snapshot.
    deltas: ByteSize,
}

impl Cost {
    /// The two together.
    fn total(self) -> ByteSize {
        self.snapshot.saturating_add(self.deltas)
    }
}

/// Which try of one write the quota refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuotaAttempt {
    /// The write as Logic sent it: evicting a copy may make room.
    First,
    /// The one retry after an eviction: nothing more is evicted.
    Retried,
}

/// What the store does about a write the browser refused for its quota.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// Delete these copies, which the relay holds, then write again once
    /// (`RESID-017`).
    Retry {
        /// The copies to delete, in the order they leave.
        evict: Vec<DocumentId>,
    },
    /// The write carries a change the relay has not acknowledged and nothing
    /// is left to free: editing stops until it is sent (`RESID-018`).
    Full,
    /// The relay holds every change of the document, so the write is a copy
    /// it can serve again: the store goes without it.
    Drop,
}

/// What the ledger knows of one document, as the store keeps it across a
/// reload. Its encoding is [`Meta::LEN`] bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Meta {
    cost: Cost,
    viewed: Generation<ViewTick>,
    immutable: Mutability,
    unsent: Delivery,
}

impl Meta {
    /// The length of an encoded row: three little-endian `u64` and a flag byte.
    pub const LEN: usize = 25;

    /// The row the store keeps.
    #[must_use]
    pub fn encode(&self) -> [u8; Self::LEN] {
        let mut row = [0u8; Self::LEN];
        let fields = [
            self.cost.snapshot.0,
            self.cost.deltas.0,
            self.viewed.stored(),
        ];
        for (chunk, field) in row.chunks_exact_mut(8).zip(fields) {
            chunk.copy_from_slice(&field.to_le_bytes());
        }
        if let Some(flags) = row.get_mut(24) {
            *flags = u8::from(self.immutable == Mutability::Immutable)
                | (u8::from(self.unsent == Delivery::Unsent) << 1);
        }
        row
    }

    /// The row without its delta bytes, which change at every keystroke.
    fn settled(self) -> Self {
        Self {
            cost: Cost {
                deltas: ByteSize(0),
                ..self.cost
            },
            ..self
        }
    }

    /// Read a row back. `None` for a row of another length.
    #[must_use]
    pub fn decode(row: &[u8]) -> Option<Self> {
        let row: &[u8; Self::LEN] = row.try_into().ok()?;
        let (fields, flags) = row.split_at(24);
        let mut words = fields
            .chunks_exact(8)
            .filter_map(|chunk| <[u8; 8]>::try_from(chunk).ok())
            .map(u64::from_le_bytes);
        let (snapshot, deltas, viewed) = (words.next()?, words.next()?, words.next()?);
        let flags = *flags.first()?;
        Some(Self {
            cost: Cost {
                snapshot: ByteSize(snapshot),
                deltas: ByteSize(deltas),
            },
            viewed: Generation::from_stored(viewed),
            immutable: if flags & 1 == 0 {
                Mutability::Mutable
            } else {
                Mutability::Immutable
            },
            unsent: if flags & 2 == 0 {
                Delivery::Sent
            } else {
                Delivery::Unsent
            },
        })
    }
}

/// What the store does after it observes one write.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Observed {
    /// The copies to delete so the store fits its budget (`RESID-002`).
    pub evict: Vec<DocumentId>,
    /// Whether the document's [`Meta`] changed, so its row is written again.
    pub record: RowChange,
}

ambre_core::define_flag! {
    /// Whether one write changed the [`Meta`] row the store keeps of a
    /// document.
    pub enum RowChange {
        /// The row keeps what it held, so it is not written.
        Unchanged,
        /// The row changed, so it is written again.
        Changed,
    }
}

/// The documents the local store holds, what each costs, and which leave
/// first (`RESID-002`, `RESID-007`, `RESID-008`).
#[derive(Debug, Clone)]
pub struct StoreLedger {
    residency: Residency<DocumentId>,
    costs: HashMap<DocumentId, Cost>,
    budget: ByteSize,
}

impl Default for StoreLedger {
    fn default() -> Self {
        Self::new(LOCAL_STORE_BUDGET)
    }
}

impl StoreLedger {
    /// An empty ledger that keeps the store within `budget` bytes.
    #[must_use]
    pub fn new(budget: ByteSize) -> Self {
        Self {
            residency: Residency::new(),
            costs: HashMap::new(),
            budget,
        }
    }

    /// Count one write or signal from Logic, and name the copies that leave.
    ///
    /// Any write marks the document unsent: only the relay's acknowledgement,
    /// [`SealedWrite::Sent`], clears it (`RESID-019`).
    pub fn observe(&mut self, id: DocumentId, write: &SealedWrite) -> Observed {
        let before = self.meta(id);
        let mut cost = self.costs.get(&id).copied().unwrap_or_default();
        match write {
            SealedWrite::Snapshot(bytes) => {
                cost = Cost {
                    snapshot: ByteSize::of_len(bytes.len()),
                    deltas: ByteSize(0),
                };
                self.charge(id, cost, Delivery::Unsent);
            }
            SealedWrite::Delta(bytes) => {
                cost.deltas = cost.deltas.saturating_add(ByteSize::of_len(bytes.len()));
                self.charge(id, cost, Delivery::Unsent);
            }
            SealedWrite::Session { .. } => self.charge(id, cost, Delivery::Unsent),
            SealedWrite::Shown => {
                if !self.residency.holds(id) {
                    self.charge(id, cost, Delivery::Sent);
                }
                self.residency.pin(id);
            }
            SealedWrite::Hidden => self.residency.unpin(id),
            SealedWrite::Sent => self.residency.set_unsent(id, Delivery::Sent),
            SealedWrite::ForgetSession { .. } | SealedWrite::ReadBack { .. } => {}
        }
        // The delta bytes alone change at every keystroke: a reload counts
        // them from the last row, which the next snapshot corrects.
        let record =
            RowChange::from_bool(self.meta(id).map(Meta::settled) != before.map(Meta::settled));
        let evict = self.residency.evictions(self.budget);
        for gone in &evict {
            self.forget(*gone);
        }
        Observed { evict, record }
    }

    /// Record what `id` costs, and mark it unsent when a write changed it.
    fn charge(&mut self, id: DocumentId, cost: Cost, unsent: Delivery) {
        self.costs.insert(id, cost);
        self.residency.hold(id, cost.total());
        if unsent == Delivery::Unsent {
            self.residency.set_unsent(id, Delivery::Unsent);
        }
    }

    /// Decide what a write of `need` bytes on `id`, refused for the quota,
    /// does. A write met on its `Retried` attempt was already retried once
    /// (`RESID-017`).
    ///
    /// A copy named to leave is never `id`, and never a document with a
    /// change the relay has not acknowledged (`RESID-019`).
    pub fn refused(&mut self, id: DocumentId, need: ByteSize, attempt: QuotaAttempt) -> Refusal {
        if attempt == QuotaAttempt::First {
            let room = self.residency.bytes().saturating_sub(need);
            let evict: Vec<DocumentId> = self
                .residency
                .evictions(room)
                .into_iter()
                .filter(|gone| *gone != id)
                .collect();
            if !evict.is_empty() {
                for gone in &evict {
                    self.forget(*gone);
                }
                return Refusal::Retry { evict };
            }
        }
        if self.unsent(id) {
            Refusal::Full
        } else {
            Refusal::Drop
        }
    }

    /// Record whether the application marks a document immutable
    /// (`RESID-007`).
    pub fn set_immutable(&mut self, id: DocumentId, immutable: Mutability) {
        self.residency.set_immutable(id, immutable);
    }

    /// Stop counting a document whose copy left the store.
    pub fn forget(&mut self, id: DocumentId) {
        self.residency.release(id);
        self.costs.remove(&id);
    }

    /// Whether the document carries a change the relay has not acknowledged.
    #[must_use]
    pub fn unsent(&self, id: DocumentId) -> bool {
        self.residency
            .get(id)
            .is_some_and(|held| held.unsent == Delivery::Unsent)
    }

    /// The bytes every copy costs together.
    #[must_use]
    pub fn bytes(&self) -> ByteSize {
        self.residency.bytes()
    }

    /// The row the store keeps for `id`, if it holds it.
    #[must_use]
    pub fn meta(&self, id: DocumentId) -> Option<Meta> {
        let held = self.residency.get(id)?;
        Some(Meta {
            cost: self.costs.get(&id).copied().unwrap_or_default(),
            viewed: held.viewed(),
            immutable: held.immutable,
            unsent: held.unsent,
        })
    }

    /// Count a document as an earlier run left it. No view shows it yet.
    pub fn restore(&mut self, id: DocumentId, meta: Meta) {
        let mut held = Held::default();
        held.bytes = meta.cost.total();
        held.immutable = meta.immutable;
        held.unsent = meta.unsent;
        self.costs.insert(id, meta.cost);
        self.residency.restore(id, held, meta.viewed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ambre_core::{Payload, SealedRowBytes};

    fn doc(n: u8) -> DocumentId {
        DocumentId::from_bytes([n; 16])
    }

    /// A document written once, then acknowledged by the relay and closed.
    /// Returns every copy that left on the way.
    fn settled(ledger: &mut StoreLedger, id: DocumentId, bytes: usize) -> Vec<DocumentId> {
        [
            SealedWrite::Shown,
            SealedWrite::Snapshot(Payload::<SealedRowBytes>::new(vec![0; bytes])),
            SealedWrite::Hidden,
            SealedWrite::Sent,
        ]
        .iter()
        .flat_map(|write| ledger.observe(id, write).evict)
        .collect()
    }

    /// `RESID-002`: past its budget, the store names the copies to delete
    /// until the rest fits, and stops counting them.
    #[test]
    fn the_store_stays_within_its_budget() {
        let mut ledger = StoreLedger::new(ByteSize(100));
        assert!(settled(&mut ledger, doc(1), 60).is_empty());
        assert_eq!(settled(&mut ledger, doc(2), 60), vec![doc(1)]);
        assert_eq!(ledger.bytes(), ByteSize(60));
        assert!(ledger.meta(doc(1)).is_none());
    }

    /// `RESID-019`: a copy shown, or holding a change the relay has not
    /// acknowledged, never leaves, even past the budget.
    #[test]
    fn a_shown_or_unsent_copy_never_leaves() {
        let mut ledger = StoreLedger::new(ByteSize(100));
        ledger.observe(doc(1), &SealedWrite::Shown);
        ledger.observe(
            doc(1),
            &SealedWrite::Snapshot(Payload::<SealedRowBytes>::new(vec![0; 80])),
        );
        ledger.observe(doc(1), &SealedWrite::Hidden);
        assert!(ledger.unsent(doc(1)));
        ledger.observe(doc(2), &SealedWrite::Shown);
        let observed = ledger.observe(
            doc(2),
            &SealedWrite::Delta(Payload::<SealedRowBytes>::new(vec![0; 80])),
        );
        assert!(observed.evict.is_empty(), "both stay");
        assert_eq!(
            ledger.refused(doc(2), ByteSize(10), QuotaAttempt::First),
            Refusal::Full
        );
        let observed = ledger.observe(doc(1), &SealedWrite::Sent);
        assert_eq!(observed.evict, vec![doc(1)], "once acknowledged, it may go");
    }

    /// `RESID-017`: a refused write frees what the relay holds, then retries
    /// once. A second refusal stops editing only for an unsent change.
    #[test]
    fn a_refused_write_frees_what_the_relay_holds_then_retries_once() {
        let mut ledger = StoreLedger::new(ByteSize(1_000));
        settled(&mut ledger, doc(1), 30);
        settled(&mut ledger, doc(2), 30);
        settled(&mut ledger, doc(3), 30);
        ledger.observe(doc(4), &SealedWrite::Shown);
        ledger.observe(
            doc(4),
            &SealedWrite::Delta(Payload::<SealedRowBytes>::new(vec![0; 10])),
        );
        assert_eq!(
            ledger.refused(doc(4), ByteSize(50), QuotaAttempt::First),
            Refusal::Retry {
                evict: vec![doc(1), doc(2)]
            },
            "the least recently viewed leave until the write fits"
        );
        assert_eq!(ledger.bytes(), ByteSize(40));
        assert_eq!(
            ledger.refused(doc(4), ByteSize(50), QuotaAttempt::Retried),
            Refusal::Full
        );
        assert_eq!(
            ledger.refused(doc(3), ByteSize(50), QuotaAttempt::Retried),
            Refusal::Drop
        );
    }

    /// `RESID-008`: what a reload restores keeps the order of the views.
    #[test]
    fn a_reload_keeps_the_order_of_the_views() {
        let mut ledger = StoreLedger::new(ByteSize(100));
        settled(&mut ledger, doc(1), 40);
        settled(&mut ledger, doc(2), 40);
        ledger.observe(doc(1), &SealedWrite::Shown);
        ledger.observe(doc(1), &SealedWrite::Hidden);
        let mut reloaded = StoreLedger::new(ByteSize(100));
        for id in [doc(1), doc(2)] {
            let row = ledger.meta(id).map(|meta| meta.encode());
            let meta = row.and_then(|row| Meta::decode(&row));
            assert_eq!(meta, ledger.meta(id), "the row round-trips");
            if let Some(meta) = meta {
                reloaded.restore(id, meta);
            }
        }
        assert_eq!(
            settled(&mut reloaded, doc(3), 40),
            vec![doc(2)],
            "the one viewed longest ago"
        );
    }

    /// A write that changes nothing the row keeps is not recorded again.
    #[test]
    fn only_a_change_of_the_row_is_recorded() {
        let mut ledger = StoreLedger::default();
        assert_eq!(
            ledger.observe(doc(1), &SealedWrite::Shown).record,
            RowChange::Changed
        );
        assert_eq!(
            ledger
                .observe(
                    doc(1),
                    &SealedWrite::Delta(Payload::<SealedRowBytes>::new(vec![1]))
                )
                .record,
            RowChange::Changed
        );
        assert_eq!(
            ledger
                .observe(
                    doc(1),
                    &SealedWrite::ForgetSession {
                        peer: ambre_core::PeerId(1)
                    }
                )
                .record,
            RowChange::Unchanged
        );
        assert_eq!(Meta::decode(&[0; 3]), None);
    }
}
