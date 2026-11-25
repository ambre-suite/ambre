//! Murmure's channel types of the service plane. They are what the relay lists
//! of a channel, its members and its activity, which the Sync worker reads and
//! answers with (`features/ambre-spec-murmure-engine.md section 3`).
//!
//! Murmure's content vocabulary, the command a view sends to a channel and the
//! event the engine answers with, lives in `ambre-murmure` (`BUS-032`). What
//! stays here travels in `RestCommand` and `RestEvent`, so the worker builds
//! against it, and `Emoji`, which Toile shares.

use ambre_core::{Count, Recounts, Rule};
use serde::{Deserialize, Serialize};

use ambre_core::id::{Channel, Document, Id, Meta, Space, User};

use crate::{ChannelDocumentKind, Relay, Seq};
use ambre_core::NarrowCount;

/// One document a channel groups, as the relay lists it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChannelDocument {
    pub document_id: Id<Document>,
    pub kind: ChannelDocumentKind,
}

/// The relay's receipt of a thread link, as a root or an amendments document
/// of the channel holds it (`MURMURE-732`). A census refuses to erase a
/// thread whose receipt verifies (`SYNC-330`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinkReceipt {
    pub thread: Id<Document>,
    pub signature: ambre_core::FixedBytes<ambre_core::Signature>,
}

/// Whether one document of a channel holds something past the caller's
/// read cursor. The relay answers a yes or no, never a count (`SRV-110`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocumentActivity {
    pub channel_id: Id<Channel>,
    pub document_id: Id<Document>,
    /// A segment or a thread. A thread counts its mentions alone.
    pub kind: ChannelDocumentKind,
    pub active: crate::Activity,
    /// The caller's own read cursor on the document: the sequence number it
    /// has read up to. The client counts against it once it holds the
    /// document.
    pub read: Seq<Relay>,
    /// The document's last number when the relay answered. A watch of a
    /// document the client never held resumes after it; nothing counts with
    /// it. `None` from a relay that predates it.
    pub last: Option<Seq<Relay>>,
}

/// Whether a channel holds something new, and how much once the client
/// knows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChannelActivity {
    pub channel_id: Id<Channel>,
    /// The mark the sidebar shows before any count is known (`MURMURE-119`).
    pub active: crate::Activity,
    /// The new messages of the channel, once the loaded document gives the
    /// number (`MURMURE-120`). `None` while only the mark is known.
    pub unread: Option<Count<Unread>>,
    /// The new messages that name this user or the channel. A loaded
    /// document counts its marks, so a false summary does not stay
    /// (`MURMURE-104`). `None` while only the mark is known.
    pub mentions: Option<Count<Mentions>>,
}

/// How many messages a segment or a list holds, or how many a segment
/// holds before it asks to close.
pub enum Messages {}

impl NarrowCount for Messages {
    const NAME: ambre_core::StaticKey<ambre_core::DebugNames> =
        ambre_core::StaticKey::new("MessageCount");
}

/// How many messages past the reader's cursor mention them, or the channel
/// (`MURMURE-233`). A marker of its own beside [`Unread`].
pub enum Mentions {}

impl NarrowCount for Mentions {
    const NAME: ambre_core::StaticKey<ambre_core::DebugNames> =
        ambre_core::StaticKey::new("MentionCount");
}

/// Each message a count found that mentions a reader is one mention.
impl Recounts<Messages> for Mentions {}

/// Each update of a document's activity that named the reader, or every
/// member, is one mention.
impl Recounts<ambre_core::activity::MentionUpdates> for Mentions {}

/// Where a channel the user belongs to lives, as the relay lists it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChannelPlace {
    /// A public channel of the space: its catalogue lists it.
    Public(Id<Space>),
    /// A private channel of the space: only its members know it.
    Private(Id<Space>),
    /// A direct message, in no space.
    Direct,
}

/// One channel the user is a member of, answering
/// [`LogicCommand::ListMemberChannels`](crate::LogicCommand::ListMemberChannels).
/// Learning of it opens nothing (`MURMURE-475`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemberChannel {
    pub channel_id: Id<Channel>,
    pub place: ChannelPlace,
}

/// The `meta` of a channel the user is a member of, as the relay's listing
/// names it. The Logic thread watches a channel it never opened through it
/// (the Murmure notifications spec).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChannelMeta {
    pub channel_id: Id<Channel>,
    pub meta: Id<Meta>,
    /// The last number the relay gave in `meta`, none from a relay that
    /// lists none. A watch resumes after it.
    pub last: Option<Seq<Relay>>,
    /// The live call of the channel as the listing named it, none for none (`CALL-162`).
    pub call: Option<Id<ambre_core::id::Call>>,
}

/// The name a channel not yet open shows, read from the name sealed beside
/// its `meta`. The name its `meta` holds replaces it once the channel opens.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChannelName {
    pub channel_id: Id<Channel>,
    pub name: ambre_core::Validated<ambre_core::Name>,
}

/// The display names of a direct message's other members, as the relay
/// lists them. The view composes a name from them for a direct message
/// nobody named, and never stores it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DirectPeers {
    pub channel_id: Id<Channel>,
    pub names: Vec<ambre_core::Validated<ambre_core::DisplayName>>,
}

/// How many messages of a channel are past the reader's cursor. A marker of
/// its own, so a count of mentions never takes its place.
pub enum Unread {}

impl NarrowCount for Unread {
    const NAME: ambre_core::StaticKey<ambre_core::DebugNames> =
        ambre_core::StaticKey::new("UnreadCount");
}

/// Each message a count found past a reader's cursor is one unread message.
impl Recounts<Messages> for Unread {}

/// Each content update of a document's activity is one unread message.
impl Recounts<ambre_core::activity::ContentUpdates> for Unread {}

pub use ambre_core::{Authored, Instant};

/// One emoji a member reacts with, as the view picked it. The engine
/// refuses a value that is not one emoji, and this type keeps the value
/// apart from free text. A table compiled into the client borrows its text,
/// so the table needs no allocation.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Emoji(std::borrow::Cow<'static, str>);

impl Emoji {
    /// The emoji `glyph` spells, for a table compiled into the client.
    pub const fn from_static(glyph: &'static str) -> Self {
        Self(std::borrow::Cow::Borrowed(glyph))
    }

    /// The emoji, as its characters spell it.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for Emoji {
    fn from(text: &str) -> Self {
        Self(std::borrow::Cow::Owned(text.to_owned()))
    }
}

impl From<String> for Emoji {
    fn from(text: String) -> Self {
        Self(std::borrow::Cow::Owned(text))
    }
}

// A file's media type is shown and used to open the file, and never sent to
// the relay (`SYNC-053`). The three types live in `ambre-core`, which the
// relay's media routes share. A message names its file by the address the
// Sync worker gave it, read back through `Validated::<ContentHash>::parse`.
pub use ambre_core::{ContentHash, MediaType, Octets};

/// A channel key sealed under the key of its space (`MURMURE-157`).
///
/// The Logic thread holds it and never opens it: the space document lives
/// there, and no key material may reach it (`SYNC-039`). Only the Sync
/// worker, which holds the space key, seals and unseals it.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SealedChannelKey(Vec<u8>);

impl SealedChannelKey {
    /// Wrap the sealed bytes the Sync worker produced.
    pub fn from_bytes(bytes: Vec<u8>) -> Self {
        Self(bytes)
    }

    /// The catalogue form of one seal: the id of the space key that sealed
    /// it, the nonce, then the ciphertext and its tag.
    pub fn from_parts(
        key_id: ambre_core::FixedBytes<ambre_core::KeyId>,
        nonce: &ambre_core::FixedBytes<ambre_core::Nonce>,
        ciphertext: &ambre_core::Payload<ambre_core::Ciphertext>,
    ) -> Self {
        let mut bytes = Vec::new();
        key_id.push_to(&mut bytes);
        nonce.push_to(&mut bytes);
        bytes.extend_from_slice(ciphertext.as_bytes());
        Self(bytes)
    }

    /// The id of the space key that sealed it. `None` for bytes too short to
    /// name a key.
    pub fn key_id(&self) -> Option<ambre_core::FixedBytes<ambre_core::KeyId>> {
        ambre_core::FixedBytes::<ambre_core::KeyId>::split_from(&self.0).map(|(key_id, _)| key_id)
    }

    /// The parts [`Self::from_parts`] joined. `None` for bytes too short to
    /// hold a key id and a nonce.
    pub fn split(
        &self,
    ) -> Option<(
        ambre_core::FixedBytes<ambre_core::KeyId>,
        ambre_core::FixedBytes<ambre_core::Nonce>,
        ambre_core::Payload<ambre_core::Ciphertext>,
    )> {
        let (key_id, rest) = ambre_core::FixedBytes::<ambre_core::KeyId>::split_from(&self.0)?;
        let (nonce, ciphertext) = ambre_core::FixedBytes::<ambre_core::Nonce>::split_from(rest)?;
        Some((
            key_id,
            nonce,
            ambre_core::Payload::<ambre_core::Ciphertext>::new(ciphertext.to_vec()),
        ))
    }

    /// The sealed bytes, as the catalogue stores them.
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

impl std::fmt::Debug for SealedChannelKey {
    // The length alone: a log line needs no ciphertext.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "SealedChannelKey({} bytes)", self.0.len())
    }
}

/// The rule of the topic of a channel: a line of text under its name. The
/// name keeps it apart from the relay topic, a publish-subscribe address. A
/// `Validated<ChannelTopic>` holds one.
///
/// Never empty: a channel with no topic holds no topic at all, so the
/// catalogue stores an absent key rather than an empty string. A member typed
/// it, so `Debug` prints its length alone.
pub enum ChannelTopic {}

impl Rule for ChannelTopic {
    type Error = &'static str;

    fn admit(text: String) -> Result<String, Self::Error> {
        if text.is_empty() {
            Err("a topic is never empty")
        } else {
            Ok(text)
        }
    }
}

/// A member of a direct message, other than this user, with the exchange key
/// the user confirmed for them (`ambre-spec-key-verification.md`).
///
/// The key is public. The Sync worker fetches the member's key again and
/// wraps the channel key only when the two match. So a key the relay swapped
/// in never receives a wrap.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DirectMember {
    pub user_id: Id<User>,
    pub exchange_pub_key: ambre_core::FixedBytes<ambre_core::ExchangePublicKey>,
}

#[cfg(test)]
mod tests {
    use super::{ChannelTopic, SealedChannelKey};
    use ambre_core::Validated;
    use ambre_core::{Ciphertext, FixedBytes, KeyId, Nonce, Payload};

    /// A split gives back the parts a join wrote. Bytes too short to hold a
    /// key id and a nonce give nothing.
    #[test]
    fn a_sealed_channel_key_splits_into_the_parts_it_joined() {
        let (key_id, nonce) = (
            FixedBytes::<KeyId>::from_array([3; 16]),
            FixedBytes::<Nonce>::from_array([7; 24]),
        );
        let ciphertext = Payload::<Ciphertext>::new(b"sealed".to_vec());
        let sealed = SealedChannelKey::from_parts(key_id, &nonce, &ciphertext);
        assert_eq!(sealed.key_id(), Some(key_id));
        assert_eq!(sealed.split(), Some((key_id, nonce, ciphertext)));
        let empty =
            SealedChannelKey::from_parts(key_id, &nonce, &Payload::<Ciphertext>::new(Vec::new()));
        assert_eq!(
            empty.split(),
            Some((key_id, nonce, Payload::<Ciphertext>::new(Vec::new())))
        );
        let short = SealedChannelKey::from_bytes(vec![3; 39]);
        assert_eq!(short.key_id(), Some(key_id));
        assert_eq!(short.split(), None);
        assert_eq!(SealedChannelKey::from_bytes(vec![3; 15]).key_id(), None);
    }

    /// An empty topic is no topic: it cannot be built, nor decoded from the
    /// wire, so the catalogue never stores an empty string.
    #[test]
    fn a_topic_is_never_empty() {
        assert_eq!(
            Validated::<ChannelTopic>::parse(""),
            Err("a topic is never empty")
        );
        let figures = Validated::<ChannelTopic>::parse("the weekly figures").expect("a topic");
        assert_eq!(figures.as_str(), "the weekly figures");

        let bytes = postcard::to_allocvec(&figures).expect("encode");
        assert_eq!(
            bytes,
            postcard::to_allocvec("the weekly figures").expect("encode")
        );
        assert_eq!(
            serde_json::to_string(&figures).expect("encode"),
            "\"the weekly figures\""
        );
        assert_eq!(
            postcard::from_bytes::<Validated<ChannelTopic>>(&bytes),
            Ok(figures)
        );
        let empty = postcard::to_allocvec("").expect("encode");
        assert!(postcard::from_bytes::<Validated<ChannelTopic>>(&empty).is_err());
    }
}
