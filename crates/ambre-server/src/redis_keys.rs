//! The keys the relay writes in the ephemeral store.
//!
//! Each key of a document is its own type. A script that takes the pending
//! list and the tombstone then cannot take one in place of the other.
//! [`PendingKey`] is the one key the relay reads back from the store: the
//! flusher finds its work list there.

use std::fmt;

use ambre_core::id::{Document, Id};

/// Defines one key of a document's state, `doc:{id}:{slot}`.
macro_rules! doc_keys {
    ($($(#[$meta:meta])* $name:ident => $slot:literal;)*) => {$(
        $(#[$meta])*
        #[derive(Debug, Clone, PartialEq, Eq, Hash)]
        pub struct $name(String);

        impl $name {
            /// The key of `document`.
            pub fn of(document: &Id<Document>) -> Self {
                Self(format!(concat!("doc:{}:", $slot), document))
            }

            /// The key text, as a store command takes it.
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(self.as_str())
            }
        }
    )*};
}

doc_keys! {
    /// The counter of the delta numbers the relay gave the document.
    SeqKey => "seq";
    /// The hash of the last clock of each author key.
    ClocksKey => "clocks";
    /// The presence hash: one sealed blob per author key.
    PresenceKey => "presence";
    /// The list of the entries no store could take (`SRV-035`).
    DeadLetterKey => "deadletter";
    /// The tombstone of a document being deleted (`SRV-017`).
    GoneKey => "gone";
    /// The hash that counts the sessions of each author key (`PRESENCE-015`).
    SessionsKey => "sessions";
    /// The mirror of a segment's finalize lock (`SRV-114`).
    LockedKey => "locked";
    /// The mirror of a rotation the document owes (`SRV-122`).
    RotationOwedKey => "rotation-owed";
    /// The one standing snapshot request of the document (`SRV-038`).
    SnapshotAskedKey => "snapshot_asked";
    /// The lock that serializes the flush of the document's pending list.
    FlushLockKey => "flush:lock";
}

/// The list of the deltas a document holds until the flusher stores them,
/// `doc:{id}:pending`. The flusher reads these keys back from the dirty set,
/// so the key keeps the document it names.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PendingKey {
    document: Id<Document>,
    text: String,
}

impl PendingKey {
    /// The pending list of `document`.
    pub fn of(document: &Id<Document>) -> Self {
        Self {
            document: *document,
            text: format!("doc:{document}:pending"),
        }
    }

    /// Reads a key the store hands back. `None` for a text this relay never
    /// writes: another slot, or an identifier in another form.
    pub fn parse(text: &str) -> Option<Self> {
        let id = text.strip_prefix("doc:")?.strip_suffix(":pending")?;
        let key = Self::of(&Id::<Document>::parse_str(id).ok()?);
        (key.text == text).then_some(key)
    }

    /// The document whose list this is.
    pub fn document(&self) -> Id<Document> {
        self.document
    }

    /// The key text, as a store command takes it.
    pub fn as_str(&self) -> &str {
        &self.text
    }
}

impl fmt::Display for PendingKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The set of the pending lists that hold a delta to flush. The flusher reads
/// it instead of a scan of the whole keyspace.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FlushDirtySet;

impl FlushDirtySet {
    /// The key text, as a store command takes it.
    pub const fn as_str(self) -> &'static str {
        "flush:dirty"
    }
}

/// Every key the relay holds for one document, but its tombstone. A delete
/// purges them all in one script (`SRV-017`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocumentStateKeys {
    pub seq: SeqKey,
    pub clocks: ClocksKey,
    pub pending: PendingKey,
    pub presence: PresenceKey,
    pub dead_letter: DeadLetterKey,
    pub rotation_owed: RotationOwedKey,
    pub sessions: SessionsKey,
    pub locked: LockedKey,
}

impl DocumentStateKeys {
    /// The keys of `document`.
    pub fn of(document: &Id<Document>) -> Self {
        Self {
            seq: SeqKey::of(document),
            clocks: ClocksKey::of(document),
            pending: PendingKey::of(document),
            presence: PresenceKey::of(document),
            dead_letter: DeadLetterKey::of(document),
            rotation_owed: RotationOwedKey::of(document),
            sessions: SessionsKey::of(document),
            locked: LockedKey::of(document),
        }
    }

    /// The key texts, in the order the purge script reads them: seq, clocks,
    /// pending, presence, dead-letter, owed rotation, session counts, segment
    /// lock.
    pub fn texts(&self) -> [&str; 8] {
        [
            self.seq.as_str(),
            self.clocks.as_str(),
            self.pending.as_str(),
            self.presence.as_str(),
            self.dead_letter.as_str(),
            self.rotation_owed.as_str(),
            self.sessions.as_str(),
            self.locked.as_str(),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pending_key_reads_back_only_in_the_form_the_relay_writes() {
        let document = Id::<Document>::from_bytes([7u8; 16]);
        let key = PendingKey::of(&document);
        assert_eq!(key.as_str(), format!("doc:{document}:pending"));
        assert_eq!(PendingKey::parse(key.as_str()), Some(key.clone()));
        assert_eq!(key.document(), document);
        let simple = document.uuid().to_string().replace('-', "");
        for text in [
            format!("doc:{document}:gone"),
            format!("doc:{simple}:pending"),
            "doc:not-an-id:pending".to_owned(),
            format!("rate:{document}:pending"),
        ] {
            assert_eq!(PendingKey::parse(&text), None, "{text}");
        }
    }

    #[test]
    fn each_slot_names_its_own_key() {
        let document = Id::<Document>::from_bytes([1u8; 16]);
        let keys = DocumentStateKeys::of(&document);
        let gone = GoneKey::of(&document);
        let lock = FlushLockKey::of(&document);
        let asked = SnapshotAskedKey::of(&document);
        let mut texts = keys.texts().to_vec();
        texts.extend([gone.as_str(), lock.as_str(), asked.as_str()]);
        let count = texts.len();
        texts.sort_unstable();
        texts.dedup();
        assert_eq!(texts.len(), count);
        assert_eq!(
            GoneKey::of(&document).as_str(),
            format!("doc:{document}:gone")
        );
    }
}
