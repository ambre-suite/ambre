//! Ephemeral presence for [`ToileState`](super::ToileState).
//!
//! Presence (cursors, in-flight strokes, laser points, selections) is relayed as
//! **ephemeral** Sync payloads. It never enters the CRDT, never persists, and is
//! not an undo item. Outbound, a local `Presence` sample is postcard-encoded
//! into an ephemeral. Inbound, a sample is keyed by the sender's canonical
//! `Id<User>`. That id is resolved from the authenticated `pub_key` in
//! `ambre-sync` before it reaches this thread, so a peer cannot spoof another
//! user's cursor. The render side is `toile-presence`'s overlay pass.
//!
//! **Why the event carries the whole set.** A departure is the absence of an
//! entry (`PRESENCE-009`), so no departure signal is needed anywhere. This
//! thread ages its peers and hands the view the set that is still live, which
//! the view adopts whole (`TOILE-185`, `TOILE-UI-075`). One event per inbound
//! sample with a receiver-side accumulation is what this used to do, and it
//! painted a peer who left for ever.
//!
//! **Why the clock rides the command.** This thread has none. Every local sample
//! carries the UI's wall clock. The UI sends one on each gesture and a heartbeat
//! while idle, so the clock advances even on a still board. The clock is local:
//! a sender's own clock never reaches this decision, so a wrong or hostile one
//! ages nobody.

use ambre_core::id::{Id, User};

use crate::logic::command::{PresencePayload, PresenceUpdate, ToileEvent, ToileOutcome};

use super::ToileState;
use ambre_core::{FracMs, Instant, PageLoad, TimeSpan};

/// How long a peer stays live after their last sample. Chosen against the UI's
/// heartbeat (`ambre_toile::ui::view::PRESENCE_HEARTBEAT`). A peer survives
/// several missed samples, so a stalled frame or a slow relay does not blink a
/// cursor out. A peer who closed the board still leaves within a few seconds.
pub const PRESENCE_TTL: TimeSpan<FracMs> = TimeSpan::<FracMs>::new(10_000.0);

impl ToileState {
    /// Encode the local peer's presence sample into an ephemeral to
    /// broadcast, and age the peers this client knows about against `now`.
    ///
    /// The local display name is stamped on the outbound payload here rather
    /// than by the UI. The name arrives with the session (`SessionReady`) and
    /// this thread already holds it. It is cosmetic (`PRESENCE-004`).
    ///
    /// A failed encode still ages the set. Presence is best-effort and a
    /// dropped sample is corrected by the next one. But a peer must not outlive
    /// its TTL because our own encode failed.
    pub(super) fn broadcast_presence(
        &mut self,
        payload: &PresencePayload,
        now: Instant<PageLoad>,
    ) -> ToileOutcome {
        let expired = self.expire_peers(now);
        let display = if expired {
            vec![self.live_presence_event()]
        } else {
            Vec::new()
        };
        let mut sample = payload.clone();
        sample.name = self.my_name.clone();
        sample_outcome(display, &sample)
    }

    /// Apply a decrypted remote presence sample, keyed by the sender's
    /// canonical `Id<User>` (resolved in `ambre-sync`). Records the peer's latest
    /// presence and returns the live set for the overlay to adopt whole. Never
    /// mutates the CRDT or history; a malformed sample is dropped.
    ///
    /// The sample is stamped with the last clock a local command carried, which
    /// is what later ages it. A client that has never sent one stamps zero, and
    /// the first heartbeat corrects it.
    pub fn apply_ephemeral(
        &mut self,
        from_user_id: Id<User>,
        payload: &ambre_core::Payload<ambre_core::EphemeralBytes>,
    ) -> Vec<ToileEvent> {
        let presence: PresencePayload = match postcard::from_bytes(payload.as_bytes()) {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!("Toile: presence decode failed from {from_user_id:?}: {e}");
                return Vec::new();
            }
        };
        let present_slide = presence.present_slide;
        self.presence.insert(from_user_id, (presence, self.now));
        let mut events = vec![self.live_presence_event()];
        // "Follow presenter": if we follow this sender and the sample carries
        // a broadcast slide, adopt it -- clamp to the deck and glide via
        // `SlideChanged`. A sample from anyone else, or without `present_slide`,
        // never moves us. Purely local: no CRDT / history mutation.
        if self.following == Some(from_user_id)
            && let Some(idx) = present_slide
        {
            let old = self.playback.active;
            let clamped = idx.within(self.slide_len());
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
    /// A peer nobody was painting yields nothing. A departure the client never
    /// saw arrive is not a change, and an empty event would repaint the overlay
    /// for nothing. The TTL still covers the case where this message is lost.
    pub fn peer_left(&mut self, user_id: Id<User>) -> Vec<ToileEvent> {
        if self.presence.remove(&user_id).is_none() {
            return Vec::new();
        }
        vec![self.live_presence_event()]
    }

    /// Advance the local clock and drop every peer whose last sample is older
    /// than [`PRESENCE_TTL`]. `true` when the set changed, which is what
    /// makes a departure observable without a departure signal (`TOILE-185`).
    ///
    /// The clock only moves forward. A machine whose wall clock steps backwards
    /// (an NTP correction, a suspended tab) must not age every peer out at once.
    fn expire_peers(&mut self, now: Instant<PageLoad>) -> bool {
        self.now = self.now.max(now);
        let cutoff = self.now.before(PRESENCE_TTL);
        let before = self.presence.len();
        self.presence.retain(|_, (_, seen)| *seen > cutoff);
        self.presence.len() != before
    }

    /// The whole live peer set, in a stable order, as the event the view adopts
    /// (`PRESENCE-009`). Sorted by id so two passes over an unchanged set are the
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
        ambre_core::order::sort_by_id(&mut peers, |peer| peer.user_id);
        ToileEvent::PeerPresence(peers)
    }

    /// A remote peer's latest known presence, if any. Overlay / test accessor.
    pub fn peer_presence(&self, user_id: &Id<User>) -> Option<&PresencePayload> {
        self.presence.get(user_id).map(|(p, _)| p)
    }
}

/// The outcome of one outbound sample: the frame to broadcast beside
/// `display`, or `display` alone when the sample does not encode. A lost
/// sample is corrected by the next one, so the failure is logged and never
/// surfaced (`TOILE-184`). The function is generic so the failing arm has an
/// input: no `PresencePayload` makes `postcard` refuse.
fn sample_outcome<T: serde::Serialize>(display: Vec<ToileEvent>, sample: &T) -> ToileOutcome {
    match postcard::to_allocvec(sample) {
        Ok(bytes) => ToileOutcome::display_and_ephemeral(display, Some(bytes.into())),
        Err(e) => {
            tracing::warn!("Toile: presence encode failed: {e}");
            ToileOutcome::display_only(display)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A value no encoder can write. Nothing the board sends serializes like
    /// this; it exists so the failing arm of `sample_outcome` has an input.
    struct Unwritable;

    impl serde::Serialize for Unwritable {
        fn serialize<S: serde::Serializer>(&self, _serializer: S) -> Result<S::Ok, S::Error> {
            Err(serde::ser::Error::custom(
                "this value refuses to be written",
            ))
        }
    }

    /// `TOILE-184`: a sample that does not encode is dropped. No frame
    /// leaves, no error event reaches the user, and no delta is written. The
    /// events the aging produced still go out.
    #[test]
    fn a_sample_that_will_not_encode_is_dropped_silently() {
        let aged = ToileEvent::PeerPresence(Vec::new());
        let out = sample_outcome(vec![aged], &Unwritable);
        assert!(out.ephemeral.is_none(), "no frame leaves");
        assert!(out.update.is_none(), "no delta is written");
        assert!(
            matches!(out.display.as_slice(), [ToileEvent::PeerPresence(_)]),
            "the aged set goes out, and no error does: {:?}",
            out.display
        );
    }
}
