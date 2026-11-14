//! Reactions / votes for [`ToileState`](super::ToileState) (`toile-reactions`).
//! Placing an emoji reaction or a vote on an element or frame is shared,
//! persisted state, keyed by target + canonical `Id<User>`. One user has exactly
//! one entry per target (a re-vote is LWW on their own key), and counts are
//! convergent.
//!
//! **Physical layout.** A single `reactions` root `LoroMap`, keyed by the
//! composite `"{target_id}/{user_id}"` and holding `Binary(postcard(Reaction))`:
//!
//! ```text
//! reactions (LoroMap) +-- "{target_id}/{user_id}" : Binary(postcard(Reaction))
//! ```
//!
//! This is a flat map, not a sub-`LoroMap` per target, on purpose: a per-target
//! nested container would be **created lazily on first reaction**. Two peers
//! reacting on the same fresh target would concurrently create two rival
//! containers. Loro's map register would keep only one, silently dropping the
//! loser's reactions. The root `reactions` map, by contrast, is a deterministic
//! root container that always exists, so concurrent inserts under distinct
//! composite keys all survive (verified). `Id<Element>`s are hex + `_` and
//! `Id<User>`s are hyphenated UUIDs, so neither contains `/`. The composite key is
//! unambiguous, and `target` is recovered by splitting on the first `/`.
//!
//! Keyed by `Id<User>`, never `PeerID`: a reaction is a per-user decision that must
//! survive reload and multi-device use. The count (count per emoji, vote totals)
//! is **derived on read** ([`reaction_counts`](ToileState::reaction_counts)), never
//! stored, so it can never drift from the entry set. A reaction on a
//! concurrently-deleted target is skipped at derivation, never destroyed
//! (data-never-destroyed).

use ambre_core::Count;
use ambre_core::hash::HashSet;
use std::collections::BTreeMap;

use loro::LoroValue;

use crate::ToileError;
use crate::logic::command::{Reaction, ReactionCounts, ReactionKind, ToileEvent, ToileOutcome};
use crate::message::Reactions;
use ambre_core::id::{Element, Id};

use super::ToileState;
use super::names::{ReactionKey, Root};
use ambre_core::crdt::CrdtName as _;

/// Encode a value for storage under a reactions key. A failure is a refusal
/// the user has to see (`TOILE-FEAT-064`). It becomes a typed error, not a
/// line in the log under an outcome that reads like success. The function is
/// generic so the failing arm has an input: no `Reaction` makes `postcard`
/// refuse, and the mapping is what the test holds.
fn encode<T: serde::Serialize>(value: &T) -> Result<Vec<u8>, ToileError> {
    postcard::to_allocvec(value).map_err(|e| {
        tracing::warn!("Toile: a reaction did not encode: {e}");
        ToileError::WriteFailed
    })
}

/// Fold one decoded reaction into a per-target accumulator: emoji counts (a
/// `BTreeMap` for stable, sorted rendering order) and the running vote total /
/// count. Shared by the single-target [`ToileState::reaction_counts`] and the
/// whole-viewport [`ToileState::reaction_counts_by_target`] so both derive identically.
fn accumulate(
    kind: ReactionKind,
    emoji: &mut BTreeMap<ambre_bus::Emoji, Count<Reactions>>,
    counts: &mut ReactionCounts,
) {
    match kind {
        ReactionKind::Emoji(e) => {
            let count = emoji.entry(e).or_default();
            *count = count.incremented();
        }
        ReactionKind::Vote(w) => {
            counts.vote_total = counts.vote_total.plus(w);
            counts.vote_count = counts.vote_count.incremented();
        }
    }
}

impl ToileState {
    /// Place, change or remove the local user's reaction on `target`.
    /// Keyed by the canonical `Id<User>`, so a re-vote is LWW on this peer's own
    /// key. `reaction: None` deletes the user's own entry. No target-existence
    /// check happens here (a dangling target is skipped only at derivation, like
    /// a dangling connector). CRDT and undoable: one `doc.commit()` is one
    /// undo item. A pure no-op (removing an absent entry, or re-writing a
    /// byte-identical value) produces neither a commit nor a delta.
    pub(super) fn react(
        &mut self,
        target: &Id<Element>,
        reaction: Option<Reaction>,
    ) -> ToileOutcome {
        // Without an authenticated identity there is no key to write the per-user
        // entry under. The UI cannot reach this pre-`SessionReady`; guard anyway.
        let Some(user_id) = self.my_user_id else {
            return ToileOutcome::display_only(Vec::new());
        };
        // The caller MUST name a target the board still holds (`TOILE-FEAT-060`).
        if self.element_map(target).is_none() {
            return self.err_no_element(target);
        }
        let key = ReactionKey::of(target, &user_id);
        let map = self.doc.get_map(Root::Reactions.as_str());

        match reaction {
            Some(r) => {
                let bytes = match encode(&r) {
                    Ok(b) => b,
                    Err(e) => return ToileOutcome::display_only(vec![ToileEvent::Error(e)]),
                };
                // Skip a byte-identical re-write so an unchanged reaction is not a
                // spurious commit / undo item.
                if let Some(cur) = map.get(key.as_str()).and_then(|v| v.into_value().ok())
                    && let LoroValue::Binary(cur) = cur
                    && cur.as_slice() == bytes.as_slice()
                {
                    return ToileOutcome::display_only(Vec::new());
                }
                let _ = map.insert(key.as_str(), bytes);
            }
            None => {
                // Nothing to remove if the user has no entry on this target.
                if map.get(key.as_str()).is_none() {
                    return ToileOutcome::display_only(Vec::new());
                }
                let _ = map.delete(key.as_str());
            }
        }
        self.doc.commit();
        self.emit_with_update(vec![ToileEvent::ReactionsUpdated(target.clone())])
    }

    /// The read-time reaction count of `target`, aggregated from the
    /// per-user entries. A deleted target (no element map) yields an empty count:
    /// its orphan entries are skipped at derivation, never rendered, never
    /// destroyed. Scans the shared `reactions` map for the target's prefix.
    pub fn reaction_counts(&self, target: &Id<Element>) -> ReactionCounts {
        // Skip a deleted / never-created target ("reactions on a deleted
        // target"): its entries may still exist but are not rendered.
        if self.element_map(target).is_none() {
            return ReactionCounts::default();
        }
        let mut emoji: BTreeMap<ambre_bus::Emoji, Count<Reactions>> = BTreeMap::new();
        let mut counts = ReactionCounts::default();
        if let LoroValue::Map(entries) = self.doc.get_map(Root::Reactions.as_str()).get_deep_value()
        {
            for (k, value) in entries.iter() {
                if !ReactionKey::names_target(k, target) {
                    continue;
                }
                let LoroValue::Binary(bytes) = value else {
                    continue;
                };
                let Ok(Reaction { kind, .. }) = postcard::from_bytes::<Reaction>(bytes) else {
                    continue;
                };
                accumulate(kind, &mut emoji, &mut counts);
            }
        }
        counts.emoji = emoji.into_iter().collect();
        counts
    }

    /// Derive the reaction counts of every `visible`, still-existing target in
    /// one scan of the `reactions` map, for the display viewport. Keyed only by
    /// targets that carry at least one reaction. A target absent from `visible`
    /// (culled or deleted) is skipped, so its orphan entries never render. This
    /// is O(reactions), not O(elements x reactions). The whole map is walked
    /// once, each entry routed to its target by splitting the composite key on
    /// the first `/`.
    pub(super) fn reaction_counts_by_target(
        &self,
        visible: &HashSet<Id<Element>>,
    ) -> BTreeMap<Id<Element>, ReactionCounts> {
        let mut emoji: BTreeMap<Id<Element>, BTreeMap<ambre_bus::Emoji, Count<Reactions>>> =
            BTreeMap::new();
        let mut by_target: BTreeMap<Id<Element>, ReactionCounts> = BTreeMap::new();
        if let LoroValue::Map(entries) = self.doc.get_map(Root::Reactions.as_str()).get_deep_value()
        {
            for (k, value) in entries.iter() {
                let Some(target) = ReactionKey::target_of(k) else {
                    continue;
                };
                if !visible.contains(&target) {
                    continue;
                }
                let LoroValue::Binary(bytes) = value else {
                    continue;
                };
                let Ok(Reaction { kind, .. }) = postcard::from_bytes::<Reaction>(bytes) else {
                    continue;
                };
                accumulate(
                    kind,
                    emoji.entry(target.clone()).or_default(),
                    by_target.entry(target).or_default(),
                );
            }
        }
        // Flush the per-target sorted emoji maps into their count vectors.
        for (target, e) in emoji {
            if let Some(counts) = by_target.get_mut(&target) {
                counts.emoji = e.into_iter().collect();
            }
        }
        by_target
    }

    /// The local user's own reaction on `target`, if any. UI toggle / test
    /// accessor.
    pub fn my_reaction(&self, target: &Id<Element>) -> Option<Reaction> {
        let user_id = self.my_user_id?;
        let key = ReactionKey::of(target, &user_id);
        let LoroValue::Binary(bytes) = self
            .doc
            .get_map(Root::Reactions.as_str())
            .get(key.as_str())?
            .into_value()
            .ok()?
        else {
            return None;
        };
        postcard::from_bytes(&bytes).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ambre_bus::{Authored, Instant};

    /// A value no encoder can write. Nothing the board holds serializes like
    /// this; it exists so the failing arm of `encode` has an input.
    struct Unwritable;

    impl serde::Serialize for Unwritable {
        fn serialize<S: serde::Serializer>(&self, _serializer: S) -> Result<S::Ok, S::Error> {
            Err(serde::ser::Error::custom(
                "this value refuses to be written",
            ))
        }
    }

    /// An encode that fails answers a typed error, not an empty outcome
    /// (`TOILE-FEAT-064`). The caller can tell it from a reaction that landed.
    #[test]
    fn a_reaction_that_will_not_encode_answers_a_write_failure() {
        let answer = encode(&Unwritable);
        assert!(matches!(answer, Err(ToileError::WriteFailed)));
    }

    /// `TOILE-FEAT-064`: the reaction the board actually stores encodes, so the
    /// failing arm is the only one that carries an error.
    #[test]
    fn a_reaction_encodes() {
        let reaction = Reaction {
            kind: ReactionKind::Emoji(ambre_bus::Emoji::from("+1")),
            ts: Instant::<Authored>::new(0),
        };
        assert!(encode(&reaction).is_ok());
    }
}
