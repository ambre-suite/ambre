//! The keys the relay writes in the ephemeral store.
//!
//! Every key of one document or one call carries the identifier of that
//! document or call between braces, as its hash tag (`SRV-252`). A cluster
//! hashes the braces alone, so every key of one document sits in one slot,
//! and a script may name them together (`SRV-253`).
//!
//! Each key of a document is its own type. A script that takes the pending
//! list and the tombstone then cannot take one in place of the other.
//! [`PendingKey`] is the one key the relay reads back from the store: the
//! flusher finds its work list there.

use std::fmt;
use std::hash::{Hash, Hasher};
use std::marker::PhantomData;

use ambre_core::id::{Call, Document, Id};

mod sealed {
    /// Closes [`super::Slot`]: every slot of a document lives in this module.
    pub trait Sealed {}
}

/// The marker of one slot of a document's state. Sealed: the relay writes
/// these slots and no other.
pub trait Slot: sealed::Sealed + 'static {
    /// The last segment of the key, after `doc:{id}:`.
    const SLOT: &'static str;
}

/// The key of the slot `S` of one document, `doc:{id}:{slot}`.
///
/// Each slot is its own type. A script that takes the dead-letter list and
/// the tombstone then cannot take one in place of the other.
pub struct DocKey<S: Slot> {
    text: String,
    slot: PhantomData<fn() -> S>,
}

impl<S: Slot> DocKey<S> {
    /// The key of `document`.
    pub fn of(document: &Id<Document>) -> Self {
        Self {
            text: format!("doc:{{{document}}}:{}", S::SLOT),
            slot: PhantomData,
        }
    }

    /// The key text, as a store command takes it.
    pub fn as_str(&self) -> &str {
        &self.text
    }
}

// By hand, every impl below: a derive would bound the marker, which has no
// value to clone, compare or hash.

impl<S: Slot> Clone for DocKey<S> {
    fn clone(&self) -> Self {
        Self {
            text: self.text.clone(),
            slot: PhantomData,
        }
    }
}

impl<S: Slot> PartialEq for DocKey<S> {
    fn eq(&self, other: &Self) -> bool {
        self.text == other.text
    }
}

impl<S: Slot> Eq for DocKey<S> {}

impl<S: Slot> Hash for DocKey<S> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.text.hash(state);
    }
}

/// `DocKey("doc:{id}:{slot}")`: the text names the slot.
impl<S: Slot> fmt::Debug for DocKey<S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("DocKey").field(&self.text).finish()
    }
}

impl<S: Slot> fmt::Display for DocKey<S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The counter of the delta numbers the relay gave the document.
pub enum SeqCounter {}
/// The hash of the last clock of each author key.
pub enum Clocks {}
/// The presence hash: one sealed blob per author key.
pub enum Presence {}
/// The list of the entries no store could take (`SRV-035`).
pub enum DeadLetter {}
/// The tombstone of a document being deleted (`SRV-017`).
pub enum Gone {}
/// The hash that counts the sessions of each author key (`PRESENCE-015`).
pub enum Sessions {}
/// The mirror of a segment's finalize lock (`SRV-114`).
pub enum Locked {}
/// The mirror of a rotation the document owes (`SRV-122`).
pub enum RotationOwed {}
/// The one standing snapshot request of the document (`SRV-038`).
pub enum SnapshotAsked {}
/// The lock that serializes the flush of the document's pending list.
pub enum FlushLock {}

impl sealed::Sealed for SeqCounter {}
impl Slot for SeqCounter {
    const SLOT: &'static str = "seq";
}
impl sealed::Sealed for Clocks {}
impl Slot for Clocks {
    const SLOT: &'static str = "clocks";
}
impl sealed::Sealed for Presence {}
impl Slot for Presence {
    const SLOT: &'static str = "presence";
}
impl sealed::Sealed for DeadLetter {}
impl Slot for DeadLetter {
    const SLOT: &'static str = "deadletter";
}
impl sealed::Sealed for Gone {}
impl Slot for Gone {
    const SLOT: &'static str = "gone";
}
impl sealed::Sealed for Sessions {}
impl Slot for Sessions {
    const SLOT: &'static str = "sessions";
}
impl sealed::Sealed for Locked {}
impl Slot for Locked {
    const SLOT: &'static str = "locked";
}
impl sealed::Sealed for RotationOwed {}
impl Slot for RotationOwed {
    const SLOT: &'static str = "rotation-owed";
}
impl sealed::Sealed for SnapshotAsked {}
impl Slot for SnapshotAsked {
    const SLOT: &'static str = "snapshot_asked";
}
impl sealed::Sealed for FlushLock {}
impl Slot for FlushLock {
    const SLOT: &'static str = "flush:lock";
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
            text: format!("doc:{{{document}}}:pending"),
        }
    }

    /// Reads a key the store hands back. `None` for a text this relay never
    /// writes: another slot, or an identifier in another form.
    pub fn parse(text: &str) -> Option<Self> {
        let id = text.strip_prefix("doc:{")?.strip_suffix("}:pending")?;
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

/// The sorted set of one sliding window of the rate limiter,
/// `rate:{tier}:{bucket}`. The bucket names a user or a client address. So
/// the key is a type of this module, and the gate of `ZK-030` refuses it in
/// a log line above `debug`.
pub struct RateKey(String);

impl RateKey {
    /// The window of `bucket` in the tier whose word is `tier`.
    pub fn of(tier: &str, bucket: impl fmt::Display) -> Self {
        Self(format!("rate:{tier}:{bucket}"))
    }

    /// The key text, as a store command takes it.
    pub fn as_str(&self) -> &str {
        &self.0
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
    pub seq: DocKey<SeqCounter>,
    pub clocks: DocKey<Clocks>,
    pub pending: PendingKey,
    pub presence: DocKey<Presence>,
    pub dead_letter: DocKey<DeadLetter>,
    pub rotation_owed: DocKey<RotationOwed>,
    pub sessions: DocKey<Sessions>,
    pub locked: DocKey<Locked>,
}

impl DocumentStateKeys {
    /// The keys of `document`.
    pub fn of(document: &Id<Document>) -> Self {
        Self {
            seq: DocKey::<SeqCounter>::of(document),
            clocks: DocKey::<Clocks>::of(document),
            pending: PendingKey::of(document),
            presence: DocKey::<Presence>::of(document),
            dead_letter: DocKey::<DeadLetter>::of(document),
            rotation_owed: DocKey::<RotationOwed>::of(document),
            sessions: DocKey::<Sessions>::of(document),
            locked: DocKey::<Locked>::of(document),
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

/// The marker of one slot of a call's state. Sealed: the relay writes these slots and no
/// other (`ambre-spec-call-media.md section 3`, the state of a call).
pub trait CallSlot: sealed::Sealed + 'static {
    /// The last segment of the key, after `call:{id}:`.
    const SLOT: &'static str;
}

/// The key of the slot `S` of one call, `call:{id}:{slot}`.
pub struct CallKey<S: CallSlot> {
    text: String,
    slot: PhantomData<fn() -> S>,
}

impl<S: CallSlot> CallKey<S> {
    /// The key of `call`.
    pub fn of(call: &Id<Call>) -> Self {
        Self {
            text: format!("call:{{{call}}}:{}", S::SLOT),
            slot: PhantomData,
        }
    }

    /// The key text, as a store command takes it.
    pub fn as_str(&self) -> &str {
        &self.text
    }
}

/// The hash of a call: its channel, its room on the SFU, the next sender index, and whether
/// the call ended.
pub enum CallState {}
/// The hash of the seats of a call: the public key of each sender index.
pub enum CallSeats {}
/// The hash of the signed joins of a call: the announced join of each sender index.
pub enum CallJoins {}
/// The set of the sender indexes seated and not yet announced as gone. A seat field that
/// expired is still in it, so a heartbeat finds the seat and announces its leave.
pub enum CallKnown {}

impl sealed::Sealed for CallState {}
impl CallSlot for CallState {
    const SLOT: &'static str = "state";
}
impl sealed::Sealed for CallSeats {}
impl CallSlot for CallSeats {
    const SLOT: &'static str = "seats";
}
impl sealed::Sealed for CallJoins {}
impl CallSlot for CallJoins {
    const SLOT: &'static str = "joins";
}
impl sealed::Sealed for CallKnown {}
impl CallSlot for CallKnown {
    const SLOT: &'static str = "known";
}

/// Every key of one call. The scripts of a call take all four, in this order.
pub struct CallStateKeys {
    pub state: CallKey<CallState>,
    pub seats: CallKey<CallSeats>,
    pub joins: CallKey<CallJoins>,
    pub known: CallKey<CallKnown>,
}

impl CallStateKeys {
    /// The keys of `call`.
    pub fn of(call: &Id<Call>) -> Self {
        Self {
            state: CallKey::of(call),
            seats: CallKey::of(call),
            joins: CallKey::of(call),
            known: CallKey::of(call),
        }
    }

    /// The key texts, in the order the scripts of a call read them: state, seats, joins,
    /// known.
    pub fn texts(&self) -> [&str; 4] {
        [
            self.state.as_str(),
            self.seats.as_str(),
            self.joins.as_str(),
            self.known.as_str(),
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
        assert_eq!(key.as_str(), format!("doc:{{{document}}}:pending"));
        assert_eq!(PendingKey::parse(key.as_str()), Some(key.clone()));
        assert_eq!(key.document(), document);
        let simple = document.uuid().to_string().replace('-', "");
        for text in [
            format!("doc:{{{document}}}:gone"),
            format!("doc:{{{simple}}}:pending"),
            format!("doc:{document}:pending"),
            "doc:not-an-id:pending".to_owned(),
            format!("rate:{document}:pending"),
        ] {
            assert_eq!(PendingKey::parse(&text), None, "{text}");
        }
    }

    /// The hash slot a Redis cluster gives `key`, by its own rule. It is the
    /// CRC16 (XMODEM) of the hash tag modulo 16384. A key with no tag hashes
    /// whole.
    fn cluster_slot(key: &str) -> u16 {
        let bytes = key.as_bytes();
        let hashed = bytes
            .iter()
            .position(|b| *b == b'{')
            .and_then(|open| {
                let rest = &bytes[open + 1..];
                let close = rest.iter().position(|b| *b == b'}')?;
                (close > 0).then(|| &rest[..close])
            })
            .unwrap_or(bytes);
        let crc = hashed.iter().fold(0u16, |crc, byte| {
            (0..8).fold(crc ^ (u16::from(*byte) << 8), |crc, _| {
                if crc & 0x8000 == 0 {
                    crc << 1
                } else {
                    (crc << 1) ^ 0x1021
                }
            })
        });
        crc % 16384
    }

    /// The slot function agrees with the answers of `CLUSTER KEYSLOT` on a
    /// Redis 8.10 cluster, read on 2026-11-13. The tests below thus read the
    /// slots a cluster gives.
    #[test]
    fn the_slot_function_reads_the_slots_a_cluster_gives() {
        for (key, slot) in [
            ("123456789", 12739),
            ("foo", 12182),
            ("bar", 5061),
            ("foo{}{bar}", 8363),
            ("foo{{bar}}zap", 4015),
            ("{bar", 4015),
        ] {
            assert_eq!(cluster_slot(key), slot, "{key}");
        }
        assert_eq!(cluster_slot("{bar}:x"), cluster_slot("bar"));
    }

    /// `SRV-252`: every key of one document sits in the slot of the document,
    /// and so does every key of one call. Ten documents spread over more than
    /// one slot, so the tag is the identifier and not a constant.
    #[test]
    fn every_key_of_a_document_or_a_call_sits_in_its_slot() {
        let documents: Vec<_> = (0u8..10)
            .map(|n| Id::<Document>::from_bytes([n; 16]))
            .collect();
        for document in &documents {
            let keys = DocumentStateKeys::of(document);
            let mut texts = keys.texts().to_vec();
            let gone = DocKey::<Gone>::of(document);
            let lock = DocKey::<FlushLock>::of(document);
            let asked = DocKey::<SnapshotAsked>::of(document);
            texts.extend([gone.as_str(), lock.as_str(), asked.as_str()]);
            let slot = cluster_slot(&document.to_string());
            for text in texts {
                assert_eq!(cluster_slot(text), slot, "{text}");
            }
        }
        let slots: std::collections::BTreeSet<_> = documents
            .iter()
            .map(|d| cluster_slot(PendingKey::of(d).as_str()))
            .collect();
        assert!(slots.len() > 1, "one slot for every document: {slots:?}");

        let call = Id::<Call>::from_bytes([3u8; 16]);
        let slot = cluster_slot(&call.to_string());
        for text in CallStateKeys::of(&call).texts() {
            assert_eq!(cluster_slot(text), slot, "{text}");
        }
    }

    #[test]
    fn each_slot_names_its_own_key() {
        let document = Id::<Document>::from_bytes([1u8; 16]);
        let keys = DocumentStateKeys::of(&document);
        let gone = DocKey::<Gone>::of(&document);
        let lock = DocKey::<FlushLock>::of(&document);
        let asked = DocKey::<SnapshotAsked>::of(&document);
        let mut texts = keys.texts().to_vec();
        texts.extend([gone.as_str(), lock.as_str(), asked.as_str()]);
        let count = texts.len();
        texts.sort_unstable();
        texts.dedup();
        assert_eq!(texts.len(), count);
        assert_eq!(
            DocKey::<Gone>::of(&document).as_str(),
            format!("doc:{{{document}}}:gone")
        );
    }

    /// Each key keeps one text, byte for byte, with the document as its hash
    /// tag: every relay of one build reads the keys of another.
    #[test]
    fn each_key_keeps_the_text_the_store_holds() {
        let document = Id::<Document>::from_bytes([2u8; 16]);
        let id = "02020202-0202-0202-0202-020202020202";
        assert_eq!(document.to_string(), id);
        for (key, slot) in [
            (DocKey::<SeqCounter>::of(&document).to_string(), "seq"),
            (DocKey::<Clocks>::of(&document).to_string(), "clocks"),
            (DocKey::<Presence>::of(&document).to_string(), "presence"),
            (
                DocKey::<DeadLetter>::of(&document).to_string(),
                "deadletter",
            ),
            (DocKey::<Gone>::of(&document).to_string(), "gone"),
            (DocKey::<Sessions>::of(&document).to_string(), "sessions"),
            (DocKey::<Locked>::of(&document).to_string(), "locked"),
            (
                DocKey::<RotationOwed>::of(&document).to_string(),
                "rotation-owed",
            ),
            (
                DocKey::<SnapshotAsked>::of(&document).to_string(),
                "snapshot_asked",
            ),
            (DocKey::<FlushLock>::of(&document).to_string(), "flush:lock"),
        ] {
            assert_eq!(key, format!("doc:{{{id}}}:{slot}"));
        }
        assert_eq!(
            format!("{:?}", DocKey::<Gone>::of(&document)),
            format!("DocKey(\"doc:{{{id}}}:gone\")")
        );
    }
}
