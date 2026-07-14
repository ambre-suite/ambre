//! Reactions / votes for [`ToileState`](super::ToileState) (`toile-features`
//! §4.A). Placing an emoji reaction or a vote on an element or frame is shared,
//! persisted state, keyed by target + canonical `UserId`: one user has exactly
//! one entry per target (a re-vote is LWW on their own key) and tallies are
//! convergent.
//!
//! **Physical layout.** A single `reactions` root `LoroMap`, keyed by the
//! composite `"{target_id}/{user_id}"` and holding `Binary(postcard(Reaction))`:
//!
//! ```text
//! reactions (LoroMap)
//! └── "{target_id}/{user_id}" : Binary(postcard(Reaction))
//! ```
//!
//! This is a flat map, not a sub-`LoroMap` per target, on purpose: a per-target
//! nested container would be **created lazily on first reaction**, so two peers
//! reacting on the same fresh target would concurrently create two rival
//! containers and Loro's map register would keep only one -- silently dropping
//! the loser's reactions. The root `reactions` map, by contrast, is a
//! deterministic root container that always exists, so concurrent inserts under
//! distinct composite keys all survive (verified). `ElementId`s are hex + `_`
//! and `UserId`s are hyphenated UUIDs, so neither contains `/`: the composite key
//! is unambiguous and `target` is recovered by splitting on the first `/`.
//!
//! Keyed by `UserId`, never `PeerID` (§4.A / §8): a reaction is a per-user
//! decision that must survive reload and multi-device use. The tally (count per
//! emoji, vote totals) is **derived on read**
//! ([`reaction_tally`](ToileState::reaction_tally)), never stored, so it can
//! never drift from the entry set. A reaction on a concurrently-deleted target is
//! skipped at derivation, never destroyed (data-never-destroyed).

use std::collections::BTreeMap;

use loro::LoroValue;

use ambre_bus::UserId;

use crate::logic::command::{Reaction, ReactionKind, ReactionTally, ToileEvent, ToileOutcome};
use crate::logic::model::TargetId;

use super::{REACTIONS, ToileState};

/// The composite reactions key for a `(target, user)` pair (see module docs).
fn reaction_key(target: &TargetId, user: &UserId) -> String {
    format!("{}/{}", target.as_str(), user)
}

impl ToileState {
    /// Place, change or remove the local user's reaction on `target` (§4.A).
    /// Keyed by the canonical `UserId`, so a re-vote is LWW on this peer's own
    /// key. `reaction: None` deletes the user's own entry. No target-existence
    /// check happens here (a dangling target is skipped only at derivation, like
    /// a dangling connector §5.A). CRDT and undoable: one `doc.commit()` is one
    /// undo item (§8 bis). A pure no-op (removing an absent entry, or re-writing a
    /// byte-identical value) produces neither a commit nor a delta.
    pub(super) fn react(&mut self, target: &TargetId, reaction: Option<Reaction>) -> ToileOutcome {
        // Without an authenticated identity there is no key to write the per-user
        // entry under. The UI cannot reach this pre-`SessionReady`; guard anyway.
        let Some(user_id) = self.my_user_id else {
            return ToileOutcome::display_only(Vec::new());
        };
        let key = reaction_key(target, &user_id);
        let map = self.doc.get_map(REACTIONS);

        match reaction {
            Some(r) => {
                let bytes = match postcard::to_allocvec(&r) {
                    Ok(b) => b,
                    Err(e) => {
                        tracing::warn!("Toile: reaction encode failed: {e}");
                        return ToileOutcome::display_only(Vec::new());
                    }
                };
                // Skip a byte-identical re-write so an unchanged reaction is not a
                // spurious commit / undo item.
                if let Some(cur) = map.get(&key).and_then(|v| v.into_value().ok())
                    && let LoroValue::Binary(cur) = cur
                    && cur.as_slice() == bytes.as_slice()
                {
                    return ToileOutcome::display_only(Vec::new());
                }
                let _ = map.insert(&key, bytes);
            }
            None => {
                // Nothing to remove if the user has no entry on this target.
                if map.get(&key).is_none() {
                    return ToileOutcome::display_only(Vec::new());
                }
                let _ = map.delete(&key);
            }
        }
        self.doc.commit();
        self.emit_with_update(vec![ToileEvent::ReactionsUpdated(target.clone())])
    }

    /// The read-time reaction tally of `target` (§4.A), aggregated from the
    /// per-user entries. A deleted target (no element map) yields an empty tally:
    /// its orphan entries are skipped at derivation, never rendered, never
    /// destroyed. Scans the shared `reactions` map for the target's prefix.
    pub fn reaction_tally(&self, target: &TargetId) -> ReactionTally {
        // Skip a deleted / never-created target (§4.A "reactions on a deleted
        // target"): its entries may still exist but are not rendered.
        if self.element_map(target).is_none() {
            return ReactionTally::default();
        }
        let prefix = format!("{}/", target.as_str());
        let mut emoji: BTreeMap<String, u32> = BTreeMap::new();
        let mut tally = ReactionTally::default();
        if let LoroValue::Map(entries) = self.doc.get_map(REACTIONS).get_deep_value() {
            for (k, value) in entries.iter() {
                if !k.starts_with(&prefix) {
                    continue;
                }
                let LoroValue::Binary(bytes) = value else {
                    continue;
                };
                let Ok(Reaction { kind, .. }) = postcard::from_bytes::<Reaction>(bytes) else {
                    continue;
                };
                match kind {
                    ReactionKind::Emoji(e) => *emoji.entry(e).or_default() += 1,
                    ReactionKind::Vote(w) => {
                        tally.vote_total += i64::from(w);
                        tally.vote_count += 1;
                    }
                }
            }
        }
        tally.emoji = emoji.into_iter().collect();
        tally
    }

    /// The local user's own reaction on `target`, if any (§4.A). UI toggle / test
    /// accessor.
    pub fn my_reaction(&self, target: &TargetId) -> Option<Reaction> {
        let user_id = self.my_user_id?;
        let key = reaction_key(target, &user_id);
        let LoroValue::Binary(bytes) = self.doc.get_map(REACTIONS).get(&key)?.into_value().ok()?
        else {
            return None;
        };
        postcard::from_bytes(&bytes).ok()
    }
}
