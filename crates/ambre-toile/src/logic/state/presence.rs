//! Ephemeral presence for [`ToileState`](super::ToileState).
//!
//! Presence (cursors, in-flight strokes, laser points, selections) is relayed
//! as **ephemeral** Sync payloads: it never enters the CRDT, never persists, and
//! is not an undo item. Outbound, a local `Presence` sample is postcard-encoded
//! into an ephemeral frame. Inbound, a frame is keyed by the sender's canonical
//! `UserId` -- resolved from the authenticated `pub_key` in `ambre-sync` before
//! it reaches this thread, so a peer cannot spoof another user's cursor. The
//! render side is `toile-presence`'s overlay pass.
//!
//! **Why the event carries the whole set.** A departure is the absence of an
//! entry (`PRESENCE-009`), so no departure signal is needed anywhere: this
//! thread ages its peers and hands the view the set that is still live, which
//! the view adopts whole (`TOILE-185`, `TOILE-UI-075`). One event per inbound
//! frame with a receiver-side accumulation is what this used to do, and it
//! painted a peer who left for ever.
//!
//! **Why the clock rides the command.** This thread has none. Every local
//! sample carries the UI's wall clock, and the UI sends one on each gesture and
//! a heartbeat while idle, so the clock advances even on a still board. The
//! clock is local: a sender's own clock never reaches this decision, so a wrong
//! or hostile one ages nobody.

use ambre_bus::UserId;

use crate::logic::command::{PresencePayload, PresenceUpdate, ToileEvent, ToileOutcome};

use super::ToileState;

/// How long a peer stays live after their last sample. Chosen against the UI's
/// heartbeat (`ambre_toile::ui::view::PRESENCE_HEARTBEAT_MS`): a peer survives
/// several missed samples, so a stalled frame or a slow relay does not blink a
/// cursor out, and a peer who closed the board leaves within a few seconds.
pub const PRESENCE_TTL_MS: i64 = 10_000;

impl ToileState {
    /// Encode the local peer's presence sample into an ephemeral frame to
    /// broadcast, and age the peers this client knows about against `now_ms`.
    ///
    /// The local display name is stamped on the outbound payload here rather
    /// than by the UI: the name arrives with the session (`SessionReady`) and
    /// this thread already holds it. It is cosmetic (`PRESENCE-004`).
    ///
    /// A failed encode still ages the set -- presence is best-effort and a
    /// dropped sample is corrected by the next one, but a peer must not outlive
    /// its TTL because our own encode failed.
    pub(super) fn broadcast_presence(
        &mut self,
        payload: &PresencePayload,
        now_ms: i64,
    ) -> ToileOutcome {
        let expired = self.expire_peers(now_ms);
        let display = if expired {
            vec![self.live_presence_event()]
        } else {
            Vec::new()
        };
        let mut sample = payload.clone();
        sample.name = self.my_name.clone();
        match postcard::to_allocvec(&sample) {
            Ok(bytes) => ToileOutcome::display_and_ephemeral(display, Some(bytes)),
            Err(e) => {
                tracing::warn!("Toile: presence encode failed: {e}");
                ToileOutcome::display_only(display)
            }
        }
    }

    /// Apply a decrypted remote ephemeral (presence) frame, keyed by the sender's
    /// canonical `UserId` (resolved in `ambre-sync`). Records the peer's latest
    /// presence and returns the live set for the overlay to adopt whole. Never
    /// mutates the CRDT or history; a malformed frame is dropped.
    ///
    /// The sample is stamped with the last clock a local command carried, which
    /// is what later ages it. A client that has never sent one stamps zero, and
    /// the first heartbeat corrects it.
    pub fn apply_ephemeral(&mut self, from_user_id: UserId, payload: &[u8]) -> Vec<ToileEvent> {
        let presence: PresencePayload = match postcard::from_bytes(payload) {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!("Toile: presence decode failed from {from_user_id:?}: {e}");
                return Vec::new();
            }
        };
        let present_slide = presence.present_slide;
        self.presence.insert(from_user_id, (presence, self.now_ms));
        let mut events = vec![self.live_presence_event()];
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

    /// A peer said they were leaving (`PRESENCE-014`): the relay saw them
    /// unsubscribe or their connection close, and `ambre-sync` resolved the key
    /// it named. Drop them and hand the view the set that remains.
    ///
    /// A peer nobody was painting yields nothing: a departure the client never
    /// saw arrive is not a change, and an empty event would repaint the overlay
    /// for nothing. The TTL still covers the case where this message is lost.
    pub fn peer_left(&mut self, user_id: UserId) -> Vec<ToileEvent> {
        if self.presence.remove(&user_id).is_none() {
            return Vec::new();
        }
        vec![self.live_presence_event()]
    }

    /// Advance the local clock and drop every peer whose last sample is older
    /// than [`PRESENCE_TTL_MS`]. `true` when the set changed, which is what
    /// makes a departure observable without a departure signal (`TOILE-185`).
    ///
    /// The clock only moves forward: a machine whose wall clock steps backwards
    /// (an NTP correction, a suspended tab) must not age every peer out at once.
    fn expire_peers(&mut self, now_ms: i64) -> bool {
        self.now_ms = self.now_ms.max(now_ms);
        let cutoff = self.now_ms - PRESENCE_TTL_MS;
        let before = self.presence.len();
        self.presence.retain(|_, (_, seen)| *seen > cutoff);
        self.presence.len() != before
    }

    /// The whole live peer set, in a stable order, as the event the view adopts
    /// (`PRESENCE-009`). Sorted by id so two frames of an unchanged set are the
    /// same event, and the overlay's paint order never depends on a hash walk.
    fn live_presence_event(&self) -> ToileEvent {
        let mut peers: Vec<PresenceUpdate> = self
            .presence
            .iter()
            .map(|(user_id, (presence, _))| PresenceUpdate {
                user_id: *user_id,
                presence: presence.clone(),
            })
            .collect();
        peers.sort_by(|a, b| a.user_id.as_bytes().cmp(b.user_id.as_bytes()));
        ToileEvent::PeerPresence(peers)
    }

    /// A remote peer's latest known presence, if any. Overlay / test accessor.
    pub fn peer_presence(&self, user_id: &UserId) -> Option<&PresencePayload> {
        self.presence.get(user_id).map(|(p, _)| p)
    }
}
