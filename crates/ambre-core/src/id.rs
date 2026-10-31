//! Strongly-typed identifiers.
//!
//! Every identifier is one generic type, [`Id<E>`], over an entity marker `E`.
//! The compiler refuses an `Id<User>` where an `Id<Document>` is expected. An
//! untyped `Uuid` everywhere cannot catch that class of bug.
//!
//! The marker carries what differs between two identifiers:
//! * [`Entity::Repr`], the value the identifier holds: a [`Uuid`], or the text a
//!   peer mints for a board node (the Toile schema);
//! * [`Entity::NAME`], the name `Debug` prints, so a log still reads
//!   `DocumentId(...)`.
//!
//! The wire form is the one of the representation. `Id<E>` is
//! `#[serde(transparent)]`, so an `Id<Document>` serializes byte-for-byte like
//! its `Uuid`, and an `Id<Element>` like its `String`.
//!
//! A `Uuid` identifier has no `Default`: only [`Id::mint`] creates a fresh one.
//! It converts from a `Uuid`, and goes back through [`Id::uuid`] alone, so the
//! conversion runs one way.
//!
//! The compiler refuses an identifier of one kind where another kind belongs:
//!
//! ```compile_fail
//! use ambre_core::id::{Document, Id, User};
//! let user: Id<User> = Id::mint();
//! let document: Id<Document> = user;
//! ```
//!
//! It refuses a random identifier from `Default`, and a silent way back to the
//! `Uuid`:
//!
//! ```compile_fail
//! use ambre_core::id::{Document, Id};
//! let document: Id<Document> = Default::default();
//! ```
//!
//! ```compile_fail
//! use ambre_core::id::{Document, Id};
//! let uuid: ambre_core::Uuid = Id::<Document>::mint().into();
//! ```
//!
//! A frame becomes its element for free, but no conversion turns an element
//! into a frame. The board checks that way, and a decode of stored text is
//! checked again where the id lands:
//!
//! ```compile_fail
//! use ambre_core::id::{Element, Frame, Id};
//! let frame: Id<Frame> = Id::<Element>::from_text("e1".to_owned()).into();
//! ```
//!
//! The allowed forms compile, so each refusal above comes from the kind alone:
//!
//! ```
//! use ambre_core::id::{Document, Element, Frame, Id};
//! let document: Id<Document> = Id::mint();
//! let same: Id<Document> = document;
//! let _uuid: ambre_core::Uuid = same.uuid();
//! let _element: Id<Element> = Id::<Frame>::from_text("f1".to_owned()).into();
//! ```

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::marker::PhantomData;

use crate::authorship::{IdCounter, PeerId};
use crate::error::UuidError;
use crate::uuid::Uuid;

mod sealed {
    /// Closes [`super::Entity`] and [`super::IdRepr`]: every identifier and
    /// every representation is declared in this module.
    pub trait Sealed {}
}

/// The value an identifier holds. Sealed: a [`Uuid`], or the minted text of a
/// board node.
pub trait IdRepr:
    sealed::Sealed + Clone + Eq + Hash + fmt::Display + Serialize + DeserializeOwned + 'static
{
    /// Writes the value inside the `Debug` form of its identifier.
    ///
    /// # Errors
    ///
    /// Whatever `f` refuses.
    fn fmt_debug(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result;
}

impl sealed::Sealed for Uuid {}
impl IdRepr for Uuid {
    fn fmt_debug(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl sealed::Sealed for String {}
impl IdRepr for String {
    fn fmt_debug(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self, f)
    }
}

/// A kind of entity an [`Id`] names. Sealed: every entity kind lives in this
/// module, as every identifier did before.
pub trait Entity: sealed::Sealed + 'static {
    /// The name `Debug` prints, for example `"DocumentId"`.
    const NAME: &'static str;
    /// The value the identifier holds.
    type Repr: IdRepr;
}

/// An entity whose identifier is the identifier of its document: a space, a
/// channel `meta` and a segment are each one document.
pub trait DocumentScoped: Entity<Repr = Uuid> {}

/// The identifier of one entity of kind `E`.
#[derive(Serialize, Deserialize)]
#[serde(transparent, bound = "")]
pub struct Id<E: Entity> {
    raw: E::Repr,
    #[serde(skip)]
    kind: PhantomData<fn() -> E>,
}

impl<E: Entity> Id<E> {
    const fn from_raw(raw: E::Repr) -> Self {
        Self {
            raw,
            kind: PhantomData,
        }
    }
}

impl<E: Entity> Clone for Id<E> {
    fn clone(&self) -> Self {
        Self::from_raw(self.raw.clone())
    }
}

impl<E: Entity> Copy for Id<E> where E::Repr: Copy {}

impl<E: Entity> PartialEq for Id<E> {
    fn eq(&self, other: &Self) -> bool {
        self.raw == other.raw
    }
}

impl<E: Entity> Eq for Id<E> {}

impl<E: Entity> Hash for Id<E> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.raw.hash(state);
    }
}

/// An identifier is ordered when its representation is. A board keeps its
/// nodes in a sorted map, while a `Uuid` has no order to offer.
impl<E: Entity> PartialOrd for Id<E>
where
    E::Repr: Ord,
{
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl<E: Entity> Ord for Id<E>
where
    E::Repr: Ord,
{
    fn cmp(&self, other: &Self) -> Ordering {
        self.raw.cmp(&other.raw)
    }
}

impl<E: Entity> fmt::Debug for Id<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}(", E::NAME)?;
        self.raw.fmt_debug(f)?;
        f.write_str(")")
    }
}

impl<E: Entity> fmt::Display for Id<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.raw, f)
    }
}

impl<E: Entity<Repr = Uuid>> Id<E> {
    /// Mints a new identifier backed by a fresh UUID v7. The one way to create
    /// an identifier that names nothing yet.
    pub fn mint() -> Self {
        Self::from_raw(Uuid::new())
    }

    /// Wraps an existing [`Uuid`].
    pub const fn from_uuid(uuid: Uuid) -> Self {
        Self::from_raw(uuid)
    }

    /// Builds an identifier from a raw 16-byte array.
    pub const fn from_bytes(bytes: [u8; 16]) -> Self {
        Self::from_raw(Uuid::from_bytes(bytes))
    }

    /// The [`Uuid`] the identifier holds. The one way back to the `Uuid`.
    pub const fn uuid(&self) -> Uuid {
        self.raw
    }

    /// The 16 bytes of the identifier.
    pub const fn as_bytes(&self) -> &[u8; 16] {
        self.raw.as_bytes()
    }

    /// The hyphenated lowercase form, as a key in a CRDT map names it.
    pub fn to_hyphenated(&self) -> String {
        self.raw.to_hyphenated()
    }

    /// Parses an identifier from its text, hyphenated or not.
    ///
    /// # Errors
    ///
    /// [`UuidError::InvalidString`] when `s` is not a UUID in either form.
    pub fn parse_str(s: &str) -> Result<Self, UuidError> {
        Ok(Self::from_raw(Uuid::parse_str(s)?))
    }
}

impl<E: Entity<Repr = Uuid>> From<Uuid> for Id<E> {
    fn from(uuid: Uuid) -> Self {
        Self::from_raw(uuid)
    }
}

impl<E: DocumentScoped> Id<E> {
    /// The document this entity is.
    pub const fn document_id(&self) -> Id<Document> {
        Id::from_raw(self.raw)
    }

    /// The entity a document is, for a caller that knows its kind.
    pub const fn from_document(document_id: Id<Document>) -> Self {
        Self::from_raw(document_id.raw)
    }
}

impl<E: Entity<Repr = String>> Id<E> {
    /// Mints the identifier `"{peer:016x}_{counter:08x}"` from the document
    /// `PeerID` and a per-session monotonic counter. Two peers never mint the
    /// same text, so two concurrent creations both survive.
    pub fn mint_from(peer: PeerId, counter: IdCounter) -> Self {
        Self::from_raw(format!("{peer:016x}_{:08x}", counter.get()))
    }

    /// Wraps a text read back from the document, which minted it earlier.
    /// The kind is not checked here: the engine checks it where the id lands.
    pub fn from_text(text: String) -> Self {
        Self::from_raw(text)
    }

    /// The minted text, as the document stores it.
    pub fn as_str(&self) -> &str {
        &self.raw
    }
}

// The markers. Each one is an uninhabited `enum` that states its `Debug` name
// and its representation, and nothing else.

/// A document (Prose, Trame, Toile, ...), across the whole system.
pub enum Document {}
impl sealed::Sealed for Document {}
impl Entity for Document {
    const NAME: &'static str = "DocumentId";
    type Repr = Uuid;
}

/// A user account.
pub enum User {}
impl sealed::Sealed for User {}
impl Entity for User {
    const NAME: &'static str = "UserId";
    type Repr = Uuid;
}

/// A Murmure channel.
pub enum Channel {}
impl sealed::Sealed for Channel {}
impl Entity for Channel {
    const NAME: &'static str = "ChannelId";
    type Repr = Uuid;
}

/// A Murmure space. A space is a document, so its identifier is the identifier
/// of that document (the Murmure spaces spec).
pub enum Space {}
impl sealed::Sealed for Space {}
impl Entity for Space {
    const NAME: &'static str = "SpaceId";
    type Repr = Uuid;
}
impl DocumentScoped for Space {}

/// The `meta` document of a Murmure channel. A distinct kind, so a segment never
/// takes its place: access to a channel resolves through the member rows of
/// this one document.
pub enum Meta {}
impl sealed::Sealed for Meta {}
impl Entity for Meta {
    const NAME: &'static str = "MetaId";
    type Repr = Uuid;
}
impl DocumentScoped for Meta {}

/// A segment of a Murmure channel: one document of its message log. A distinct
/// kind, so a segment never takes the place of `meta`.
pub enum Segment {}
impl sealed::Sealed for Segment {}
impl Entity for Segment {
    const NAME: &'static str = "SegmentId";
    type Repr = Uuid;
}
impl DocumentScoped for Segment {}

/// A category of a Murmure space, minted by the admin's client.
pub enum Category {}
impl sealed::Sealed for Category {}
impl Entity for Category {
    const NAME: &'static str = "CategoryId";
    type Repr = Uuid;
}

/// A Murmure message, minted by the peer that creates it (the Murmure schema).
/// A position in a list moves with every concurrent append, so a pin or a reply
/// names the message by its identifier instead.
pub enum Message {}
impl sealed::Sealed for Message {}
impl Entity for Message {
    const NAME: &'static str = "MessageId";
    type Repr = Uuid;
}

/// A Prose footnote, minted by the peer that inserts it. A distinct kind, so a
/// comment or a suggestion never takes its place.
pub enum Footnote {}
impl sealed::Sealed for Footnote {}
impl Entity for Footnote {
    const NAME: &'static str = "FootnoteId";
    type Repr = Uuid;
}

/// A Prose comment thread, minted by the peer that opens it.
pub enum Comment {}
impl sealed::Sealed for Comment {}
impl Entity for Comment {
    const NAME: &'static str = "CommentId";
    type Repr = Uuid;
}

/// One reply inside a Prose comment thread.
pub enum Reply {}
impl sealed::Sealed for Reply {}
impl Entity for Reply {
    const NAME: &'static str = "ReplyId";
    type Repr = Uuid;
}

/// A Prose suggestion, minted by the peer that proposes it.
pub enum Suggestion {}
impl sealed::Sealed for Suggestion {}
impl Entity for Suggestion {
    const NAME: &'static str = "SuggestionId";
    type Repr = Uuid;
}

/// A Rendez-Vous meeting room.
pub enum Meeting {}
impl sealed::Sealed for Meeting {}
impl Entity for Meeting {
    const NAME: &'static str = "MeetingId";
    type Repr = Uuid;
}

/// A CRDT snapshot of a document.
pub enum Snapshot {}
impl sealed::Sealed for Snapshot {}
impl Entity for Snapshot {
    const NAME: &'static str = "SnapshotId";
    type Repr = Uuid;
}

/// An organization, minted by the relay when one is created.
pub enum Org {}
impl sealed::Sealed for Org {}
impl Entity for Org {
    const NAME: &'static str = "OrgId";
    type Repr = Uuid;
}

/// A finalize lease of a segment, minted by the relay when it grants one. A
/// commit names it.
pub enum Lease {}
impl sealed::Sealed for Lease {}
impl Entity for Lease {
    const NAME: &'static str = "LeaseId";
    type Repr = Uuid;
}

/// A node of a Toile board: an element, a frame or a group. [`Id::mint_from`]
/// mints it once, and no node reuses it. A Loro op id changes on a reorder, so
/// a reference names the node by this value.
pub enum Element {}
impl sealed::Sealed for Element {}
impl Entity for Element {
    const NAME: &'static str = "ElementId";
    type Repr = String;
}

/// A Toile connector. Minted as an element is, and a distinct kind, so the
/// compiler refuses one wherever an element belongs.
pub enum Connector {}
impl sealed::Sealed for Connector {}
impl Entity for Connector {
    const NAME: &'static str = "ConnectorId";
    type Repr = String;
}

/// A Toile frame: an element of type `frame`, as a presentation sequence names
/// it. An `Id<Frame>` becomes an `Id<Element>` for free. No conversion goes
/// back: the board answers the frame of an element only when it is one.
pub enum Frame {}
impl sealed::Sealed for Frame {}
impl Entity for Frame {
    const NAME: &'static str = "FrameId";
    type Repr = String;
}

/// A Toile comment thread. Minted as an element is, and a distinct kind: a
/// thread is its own entry, never a node of the board.
pub enum Thread {}
impl sealed::Sealed for Thread {}
impl Entity for Thread {
    const NAME: &'static str = "ThreadId";
    type Repr = String;
}

impl From<Id<Frame>> for Id<Element> {
    fn from(frame: Id<Frame>) -> Self {
        Self::from_raw(frame.raw)
    }
}

impl Id<Snapshot> {
    /// The genesis snapshot of a document: a deterministic id every client agrees
    /// on without a server round-trip, reusing the document's own bytes.
    ///
    /// A brand-new document has no uploaded snapshot, yet its first deltas must
    /// reference SOME parent. If a client invented a random base id, a peer (or
    /// the same client after a refresh) opening the document would never
    /// recognise that parent. It would quarantine those deltas forever. Anchoring
    /// the pre-first-snapshot base to `genesis(document_id)` makes the parent
    /// universally known, so the earliest deltas are always releasable. Once a
    /// real snapshot is uploaded, that snapshot's id supersedes the genesis.
    pub const fn genesis(document_id: Id<Document>) -> Self {
        Self::from_raw(document_id.raw)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_transparent_over_inner_uuid() {
        // A typed id must serialize exactly like its inner Uuid (wire compat).
        let uuid = Uuid::new();
        let id = Id::<Document>::from_uuid(uuid);
        assert_eq!(
            serde_json::to_string(&id).unwrap(),
            serde_json::to_string(&uuid).unwrap(),
        );
        assert_eq!(
            postcard::to_allocvec(&id).unwrap(),
            postcard::to_allocvec(&uuid).unwrap(),
        );
    }

    #[test]
    fn roundtrip() {
        let id = Id::<Document>::mint();
        let bytes = serde_json::to_vec(&id).unwrap();
        let back: Id<Document> = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(id, back);
        let frame = postcard::to_allocvec(&id).unwrap();
        assert_eq!(postcard::from_bytes::<Id<Document>>(&frame).unwrap(), id);
    }

    /// A minted text identifier travels as its plain string, both ways.
    #[test]
    fn a_minted_id_travels_as_its_text() {
        let id = Id::<Element>::mint_from(PeerId::new(0xab), IdCounter::new(7));
        assert_eq!(id.as_str(), "00000000000000ab_00000007");
        let json = serde_json::to_string(&id).unwrap();
        assert_eq!(json, "\"00000000000000ab_00000007\"");
        assert_eq!(serde_json::from_str::<Id<Element>>(&json).unwrap(), id);
        let frame = postcard::to_allocvec(&id).unwrap();
        assert_eq!(frame, postcard::to_allocvec(id.as_str()).unwrap());
        assert_eq!(postcard::from_bytes::<Id<Element>>(&frame).unwrap(), id);
    }

    /// A frame becomes the element it is, with the same text.
    #[test]
    fn a_frame_is_the_element_of_the_same_text() {
        let frame = Id::<Frame>::from_text("f".to_owned());
        let element: Id<Element> = frame.into();
        assert_eq!(element, Id::<Element>::from_text("f".to_owned()));
    }

    /// A document-scoped entity names its document by the same bytes.
    #[test]
    fn a_document_scoped_id_is_its_document() {
        let doc = Id::<Document>::from_bytes([0x3c; 16]);
        let space = Id::<Space>::from_document(doc);
        assert_eq!(space.document_id(), doc);
        assert_eq!(Id::<Segment>::from_document(doc).as_bytes(), doc.as_bytes());
    }

    /// Holds `SYNC-026`: the parent of a document never snapshotted is the
    /// document's own bytes. A peer derives it from the id alone, so the
    /// derivation is pinned: changing it would strand every peer's first deltas.
    #[test]
    fn every_client_derives_the_same_genesis_parent() {
        let bytes = [0x5a; 16];
        let doc = Id::<Document>::from_bytes(bytes);
        assert_eq!(
            Id::<Snapshot>::genesis(doc),
            Id::<Snapshot>::from_bytes(bytes)
        );
        // A second client, holding the id only through its text, agrees.
        let parsed = Id::<Document>::parse_str(&doc.to_string()).unwrap();
        assert_eq!(
            Id::<Snapshot>::genesis(parsed),
            Id::<Snapshot>::genesis(doc)
        );
        assert_ne!(
            Id::<Snapshot>::genesis(Id::<Document>::from_bytes([0x5b; 16])),
            Id::<Snapshot>::genesis(doc)
        );
    }

    #[test]
    fn debug_carries_type_name() {
        let id = Id::<User>::from_bytes([0; 16]);
        assert_eq!(
            format!("{id:?}"),
            "UserId(00000000-0000-0000-0000-000000000000)"
        );
        let element = Id::<Element>::from_text("e1".to_owned());
        assert_eq!(format!("{element:?}"), "ElementId(\"e1\")");
    }
}
