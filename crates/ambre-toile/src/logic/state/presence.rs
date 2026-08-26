//! Ephemeral presence for [`ToileState`](super::ToileState).
//!
//! Presence (cursors, in-flight strokes, laser points, selections) is relayed
//! as **ephemeral** Sync payloads: it never enters the CRDT, never persists, and
//! is not an undo item. Outbound, a local `Presence` sample is postcard-encoded
//! into an ephemeral frame; the local cursor is drawn by the UI itself, so there
//! is no display event to emit. Inbound, a frame is keyed by the sender's
//! canonical `UserId` -- resolved from the authenticated `pub_key` in
//! `ambre-sync` before it reaches this thread, so a peer cannot spoof another
//! user's cursor. The render side is `toile-presence`'s overlay pass.

use ambre_bus::UserId;

use crate::logic::command::{PresencePayload, PresenceUpdate, ToileEvent, ToileOutcome};

use super::ToileState;

impl ToileState {
    /// Encode the local peer's presence sample into an ephemeral frame to
    /// broadcast. No display event and no CRDT delta: the local cursor is
    /// painted by the UI. A failed encode yields an empty outcome -- presence is
    /// best-effort and a dropped sample is corrected by the next one.
    pub(super) fn broadcast_presence(&self, payload: &PresencePayload) -> ToileOutcome {
        match postcard::to_allocvec(payload) {
            Ok(bytes) => ToileOutcome::ephemeral_only(bytes),
            Err(e) => {
                tracing::warn!("Toile: presence encode failed: {e}");
                ToileOutcome::display_only(Vec::new())
            }
        }
    }

    /// Apply a decrypted remote ephemeral (presence) frame, keyed by the sender's
    /// canonical `UserId` (resolved in `ambre-sync`,). Records the peer's
    /// latest presence and returns the `PeerPresence` event for the overlay to
    /// render. Never mutates the CRDT or history; a malformed frame is dropped.
    pub fn apply_ephemeral(&mut self, from_user_id: UserId, payload: &[u8]) -> Vec<ToileEvent> {
        let presence: PresencePayload = match postcard::from_bytes(payload) {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!("Toile: presence decode failed from {from_user_id:?}: {e}");
                return Vec::new();
            }
        };
        let present_slide = presence.present_slide;
        self.presence.insert(from_user_id, presence.clone());
        let mut events = vec![ToileEvent::PeerPresence(PresenceUpdate {
            user_id: from_user_id,
            presence,
        })];
        // "Follow presenter": if we follow this sender and the frame carries
        // a broadcast slide, adopt it -- clamp to the deck and glide via
        // `SlideChanged`. A frame from anyone else, or without `present_slide`,
        // never moves us. Purely local: no CRDT / history mutation.
        if self.following == Some(from_user_id)
            && let Some(idx) = present_slide
        {
            let old = self.playback.active;
            let len = self.slide_len();
            let clamped = if len == 0 { 0 } else { idx.min(len - 1) };
            if clamped != old {
                self.playback.active = clamped;
                events.push(ToileEvent::SlideChanged(
                    self.slide_change(Some(old), clamped),
                ));
            }
        }
        events
    }

    /// A remote peer's latest known presence, if any. Overlay / test accessor.
    pub fn peer_presence(&self, user_id: &UserId) -> Option<&PresencePayload> {
        self.presence.get(user_id)
    }
}
