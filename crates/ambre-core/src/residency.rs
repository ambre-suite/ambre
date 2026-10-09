//! Which documents a client holds, and which leave first
//! (`ambre-spec-residency.md`).
//!
//! The policy reads no document. It knows each held document by its key and
//! its size in bytes. It also knows whether a view pins it and whether its
//! application marks it immutable. It knows whether it carries a change the
//! relay has not acknowledged, and when it was last viewed. The local store
//! of the Main thread runs one.

use std::collections::HashMap;
use std::hash::Hash;

/// What the policy knows of one held document.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Held {
    /// The bytes the document costs in this tier.
    pub bytes: u64,
    /// A view shows it, so it never leaves (`RESID-003`).
    pub pinned: bool,
    /// Its application marks it immutable, so it leaves first (`RESID-007`).
    pub immutable: bool,
    /// It carries a change the relay has not acknowledged, so it stays
    /// (`RESID-009`, `RESID-019`).
    pub unsent: bool,
    /// The tick of its last view. A lower tick leaves first (`RESID-008`).
    viewed: u64,
}

impl Held {
    /// The tick of its last view, kept so a reload restores the order.
    #[must_use]
    pub fn viewed(&self) -> u64 {
        self.viewed
    }
}

/// The documents one tier holds, and the order they leave in.
#[derive(Debug, Clone)]
pub struct Residency<K> {
    held: HashMap<K, Held>,
    /// A counter, not a clock: it orders the views and reads no time.
    tick: u64,
}

impl<K> Default for Residency<K> {
    fn default() -> Self {
        Self {
            held: HashMap::new(),
            tick: 0,
        }
    }
}

impl<K: Copy + Eq + Hash> Residency<K> {
    /// An empty tier.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The next tick, so the latest view always sorts last.
    fn next_tick(&mut self) -> u64 {
        self.tick = self.tick.saturating_add(1);
        self.tick
    }

    /// Hold a document of `bytes`, or record its new size. A document held for
    /// the first time counts as viewed now.
    pub fn hold(&mut self, key: K, bytes: u64) {
        let tick = self.next_tick();
        self.held
            .entry(key)
            .and_modify(|held| held.bytes = bytes)
            .or_insert(Held {
                bytes,
                viewed: tick,
                ..Held::default()
            });
    }

    /// Pin a document a view shows (`RESID-003`). A document not held is left
    /// alone: the caller holds it first.
    pub fn pin(&mut self, key: K) {
        let tick = self.next_tick();
        if let Some(held) = self.held.get_mut(&key) {
            held.pinned = true;
            held.viewed = tick;
        }
    }

    /// Unpin a document that left the screen. It was viewed until now, so it
    /// is the last of its rank to leave (`RESID-008`).
    pub fn unpin(&mut self, key: K) {
        let tick = self.next_tick();
        if let Some(held) = self.held.get_mut(&key) {
            held.pinned = false;
            held.viewed = tick;
        }
    }

    /// Record whether the application marks a document immutable.
    pub fn set_immutable(&mut self, key: K, immutable: bool) {
        if let Some(held) = self.held.get_mut(&key) {
            held.immutable = immutable;
        }
    }

    /// Record whether a document carries a change the relay has not
    /// acknowledged.
    pub fn set_unsent(&mut self, key: K, unsent: bool) {
        if let Some(held) = self.held.get_mut(&key) {
            held.unsent = unsent;
        }
    }

    /// Hold a document as an earlier run of the tier knew it, its view tick
    /// included, so the order survives a reload (`RESID-008`). Later ticks
    /// are drawn past it.
    pub fn restore(&mut self, key: K, held: Held, viewed: u64) {
        self.tick = self.tick.max(viewed);
        self.held.insert(key, Held { viewed, ..held });
    }

    /// Stop holding a document, returning what was known of it.
    pub fn release(&mut self, key: K) -> Option<Held> {
        self.held.remove(&key)
    }

    /// What is known of one held document.
    #[must_use]
    pub fn get(&self, key: K) -> Option<&Held> {
        self.held.get(&key)
    }

    /// Whether the tier holds a document.
    #[must_use]
    pub fn holds(&self, key: K) -> bool {
        self.held.contains_key(&key)
    }

    /// The bytes every held document costs together.
    #[must_use]
    pub fn bytes(&self) -> u64 {
        self.held.values().map(|held| held.bytes).sum()
    }

    /// The documents to evict, in order, so the rest fits in `budget`.
    ///
    /// A pinned document and one with an unsent change are never named
    /// (`RESID-003`, `RESID-009`, `RESID-019`). The immutable ones come first,
    /// then the least recently viewed (`RESID-007`, `RESID-008`). When the
    /// documents that cannot leave exceed the budget alone, every other one is
    /// named and the tier stays over it.
    #[must_use]
    pub fn evictions(&self, budget: u64) -> Vec<K> {
        let mut over = self.bytes().saturating_sub(budget);
        if over == 0 {
            return Vec::new();
        }
        let mut candidates: Vec<(&K, &Held)> = self
            .held
            .iter()
            .filter(|(_, held)| !held.pinned && !held.unsent)
            .collect();
        // Every tick is drawn once, so two documents never tie and the order
        // never depends on the map's.
        candidates.sort_by_key(|(_, held)| (!held.immutable, held.viewed));
        candidates
            .into_iter()
            .take_while(|(_, held)| {
                let take = over > 0;
                over = over.saturating_sub(held.bytes);
                take
            })
            .map(|(key, _)| *key)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MB: u64 = 1 << 20;

    /// Three documents of 10 MB each, held in the order 1, 2, 3.
    fn three() -> Residency<u8> {
        let mut tier = Residency::new();
        for key in 1..=3 {
            tier.hold(key, 10 * MB);
        }
        tier
    }

    /// Hold every document the evictions name gone, as the caller does.
    fn evict(tier: &mut Residency<u8>, budget: u64) -> Vec<u8> {
        let gone = tier.evictions(budget);
        for key in &gone {
            tier.release(*key);
        }
        gone
    }

    #[test]
    fn a_tier_past_its_budget_comes_back_within_it() {
        let mut tier = three();
        tier.hold(4, 25 * MB);
        evict(&mut tier, 32 * MB);
        assert!(tier.bytes() <= 32 * MB);
        assert!(
            tier.holds(4),
            "the last held stays while the rest makes room"
        );
    }

    #[test]
    fn a_tier_within_its_budget_evicts_nothing() {
        assert!(three().evictions(30 * MB).is_empty());
    }

    #[test]
    fn a_pinned_document_never_leaves() {
        let mut tier = three();
        tier.pin(1);
        assert_eq!(evict(&mut tier, 0), vec![2, 3]);
        assert!(tier.holds(1));
    }

    /// Holds `RESID-019`: a copy with a change the relay has not acknowledged
    /// is never named, and leaves once the change is acknowledged.
    #[test]
    fn an_unsent_change_keeps_its_document() {
        let mut tier = three();
        tier.set_unsent(1, true);
        assert_eq!(evict(&mut tier, 0), vec![2, 3]);
        tier.set_unsent(1, false);
        assert_eq!(evict(&mut tier, 0), vec![1]);
    }

    /// Holds `RESID-007`: an immutable document leaves first, whatever its
    /// last view.
    #[test]
    fn an_immutable_document_leaves_before_any_other() {
        let mut tier = three();
        tier.set_immutable(3, true);
        assert_eq!(tier.evictions(20 * MB), vec![3]);
    }

    /// Holds `RESID-008`: within a rank, the least recently viewed leaves
    /// first, and a reload keeps the order.
    #[test]
    fn the_least_recently_viewed_leaves_first() {
        let mut tier = three();
        tier.pin(1);
        tier.unpin(1);
        assert_eq!(tier.evictions(10 * MB), vec![2, 3]);
        tier.pin(2);
        tier.unpin(2);
        assert_eq!(tier.evictions(20 * MB), vec![3]);
        let mut reloaded = Residency::new();
        for key in [3, 1, 2] {
            if let Some(held) = tier.get(key) {
                reloaded.restore(key, *held, held.viewed());
            }
        }
        reloaded.hold(4, 10 * MB);
        assert_eq!(reloaded.evictions(20 * MB), vec![3, 1]);
    }

    #[test]
    fn what_cannot_leave_may_exceed_the_budget_alone() {
        let mut tier = three();
        tier.pin(1);
        tier.set_unsent(2, true);
        assert_eq!(evict(&mut tier, MB), vec![3]);
        assert_eq!(tier.bytes(), 20 * MB);
    }
}
