//! Whether the session that hid a part of a document still runs
//! (`ambre-spec-undo.md section 3`).
//!
//! A deletion that an undo must reverse hides what it deletes. Only the
//! session that hid it can undo it, so nothing it hid may leave the document
//! for good while it runs. Prose hides a block and Trame a block of lines,
//! and both ask this one question before a sweep.
//!
//! A session runs when it is this peer's own, or when the user the registry
//! names for it has a session open on the document. The registry entry of a
//! PeerID names its user (`AUTHOR-009`). The caller knows which users have a
//! session open: each one announces itself once the document is ready, and
//! leaves on the relay's word (`PRESENCE-014`).

use ambre_core::PeerId;
use ambre_core::authorship::PeerEntry;
use ambre_core::crdt::CrdtName as _;
use ambre_core::id::{Id, User};
use loro::{LoroDoc, LoroValue, ValueOrContainer};

/// Whether the session behind `peer` runs: `own` is this peer's session, and
/// `live` answers whether a user has a session open on `doc`. A PeerID the
/// registry does not name reads as ended.
pub fn session_runs(
    doc: &LoroDoc,
    own: PeerId,
    peer: PeerId,
    live: impl Fn(&Id<User>) -> bool,
) -> bool {
    if peer == own {
        return true;
    }
    let entry = match doc
        .get_map(ambre_core::crdt::FoundationRoot::Peers.as_str())
        .get(ambre_core::PeerKey::of(peer).as_str())
    {
        Some(ValueOrContainer::Value(LoroValue::Binary(bytes))) => PeerEntry::from_bytes(&bytes),
        _ => None,
    };
    entry.is_some_and(|entry| live(&entry.user))
}
