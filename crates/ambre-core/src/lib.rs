//! Fundamental types shared by all Ambre crates.
//!
//! - [`Uuid`]: UUIDv7 wrapper (`[u8; 16]`)
//! - [`DocumentId`], [`UserId`], [`ChannelId`], [`SpaceId`], [`MetaId`], [`SegmentId`],
//!   [`CategoryId`], [`MeetingId`], [`SnapshotId`], and the Prose annotation ids
//!   [`FootnoteId`], [`CommentId`], [`ReplyId`], [`SuggestionId`]:
//!   strongly-typed identifiers (distinct newtypes over `Uuid`)
//! - [`DocType`]: enum of document types (Prose, Trame, Toile, Murmure)
//! - [`PublicKey`], [`ExchangePublicKey`]: `[u8; 32]` wrappers for Ed25519 and X25519
//! - [`Seq`]: the relay's sequence number of a document
//! - [`Version`]: version byte for serialization
//! - [`schema`]: the persisted schema version and its migration walk
//! - [`authorship`]: the session nonce and the peer registry entry
//! - [`crdt`]: the typed root names and map keys of a CRDT document
//! - [`Error`]: core error types
//! - [`docfile`]: the `.ambre` container header in its two lineages -- plaintext
//!   and passphrase-sealed -- shared by every app
//! - [`fixtures`] (feature `fixtures`, host-only): the document-fixture
//!   generator contract and its sidecar manifest

// No hand-written `unsafe` in this crate (constitution pillar). Any future
// `unsafe` block breaks the build rather than silently landing.
#![forbid(unsafe_code)]

/// The activity summary an update may carry, and the counters it moves
/// (`ambre-spec-residency.md`).
pub mod activity;
pub mod authorship;
pub mod canvas_id;
pub mod channel_kind;
/// Colours typed by their packing: [`Rgba`], [`Rgb`].
pub mod color;
pub mod crdt;
pub mod credential;
pub mod doc_type;
pub mod docfile;
pub mod email;
pub mod encoded;
pub mod envelope;
pub mod error;
pub mod fingerprint;
/// Fixture machinery (`ambre-spec-fixtures.md`). Behind the non-default
/// `fixtures` feature so no client build can reach it (I4).
#[cfg(feature = "fixtures")]
pub mod fixtures;
/// Font attributes every application shares: [`FontSize`].
pub mod font;
pub mod generation;
pub mod id;
/// Index families: a position, a count, a difference and a span, written by
/// [`define_index!`].
pub mod index;
pub mod key_hex;
pub mod key_id;
pub mod lang;
pub mod layout;
/// The target of a link: [`LinkUrl`].
pub mod link;
pub mod media;
pub mod member_count;
/// Two-way choices shared across crates: [`StepDirection`], [`HistoryStep`].
pub mod mode;
pub mod name;
pub mod object_key;
pub mod ordinal;
pub mod passphrase;
pub mod payload;
pub mod provider;
pub mod public_key;
/// Which documents a client holds, and which leave first
/// (`ambre-spec-residency.md`).
pub mod residency;
pub mod role;
/// The persisted schema version: the register, the read rule and the migration
/// walk every application shares (`ambre-spec-schema-version.md`).
pub mod schema;
pub mod secret;
pub mod seq;
/// Serde adapters for enum serialization as lowercase strings.
pub mod serde_enums;
/// Serde adapters for identifier serialization as hyphenated UUID strings.
pub mod serde_id;
pub mod span;
pub mod status;
pub mod tally;
pub mod text_offset;
/// Lengths and angles, one type per unit and per pixel space.
pub mod unit;
pub mod uuid;
pub mod version;
pub mod zoom;

// Re-export the main types for convenient use.
pub use authorship::{OpCounter, PeerEntry, PeerEntryBytes, PeerId, PeerKey, SessionNonce};
pub use canvas_id::CanvasId;
pub use channel_kind::{ChannelDocumentKind, SegmentState};
pub use color::{InvalidOpacity, Opacity, Rgb, Rgba};
pub use credential::{OidcCode, OidcState, PresignedUrl, ProviderUrl};
pub use doc_type::DocType;
pub use docfile::{
    AMBRE_MAGIC, AMBRE_SEALED_VERSION, AMBRE_VERSION, AmbreHeader, ContainerKind, ContainerVersion,
    DocFileError, FileSalt, HEADER_LEN, SALT_LEN, SEAL_NONCE_LEN, SEALED_AAD_LEN,
    SEALED_HEADER_LEN, SealedAmbre, classify, migrate_ambre, parse_sealed_header,
    read_ambre_plaintext, read_ambre_plaintext_as, sealed_aad, write_ambre_plaintext,
    write_ambre_sealed,
};
pub use email::Email;
pub use encoded::{
    EncodedKeyLink, EncodedLogEntry, EscrowedKey, InvalidMasterKeySalt, MasterKeySalt, SealedName,
    SealedShare, WrappedDocKey, WrappedMasterKey,
};
pub use envelope::{Aad, ChainHash, Ciphertext, CommitmentTag, EntryDigest, Nonce, Signature};
pub use error::{DocTypeError, Error, PublicKeyError, UuidError, VersionError};
pub use fingerprint::KeyFingerprint;
#[cfg(feature = "fixtures")]
pub use fixtures::{
    FIXTURE_IMPORT_PEER, FixtureGen, FixtureManifest, FixtureShape, ShapeBody, fixture_peer_count,
    seeded_document_id,
};
pub use font::{FontFamily, FontSize, FontWeight, InvalidFontSize, InvalidFontWeight};
pub use id::{
    CategoryId, ChannelId, CommentId, DocumentId, FootnoteId, LeaseId, MeetingId, MessageId,
    MetaId, OrgId, ReplyId, SegmentId, SnapshotId, SpaceId, SuggestionId, UserId,
};
pub use key_hex::{ExchangeKeyHex, SignatureHex, SigningKeyHex};
pub use key_id::{KEY_ID_LEN, KeyId, KeyIdError};
pub use lang::LangTag;
pub use link::LinkUrl;
pub use media::{
    ByteSize, ContentHash, KnownMediaType, MEDIA_ADDRESS_LEN, MediaAddress, MediaType,
};
pub use member_count::{MemberCount, MemberCountError};
pub use mode::{AfterDrop, DocumentScope, FinalizeEnd, HistoryStep, StepDirection, Visibility};
pub use name::{
    DisplayName, InvalidDisplayName, NAME_MAX_CHARS, NAME_TOO_LONG, Name, NameTooLong, name_fits,
};
pub use object_key::ObjectKey;
pub use passphrase::Passphrase;
pub use payload::{
    DeltaBytes, EphemeralBytes, FontBytes, LoroPayload, LoroPayloadRef, MediaBytes,
    PlaintextContainer, SealedContainer, SealedPreviousKey, SealedRowBytes, SnapshotBytes,
};
pub use provider::OidcProvider;
pub use public_key::{ExchangePublicKey, PublicKey};
pub use role::{DocumentRank, OrganizationRank, Role};
pub use schema::{
    SCHEMA_VERSION_ABSENT, SchemaError, SchemaMigration, SchemaVerdict, SchemaVersion,
    migrate_schema, migration_peer, scaffold_peer, verdict,
};
pub use secret::SecretString;
pub use seq::Seq;
pub use span::{
    LamportStamp, Millis, Nanos, PageMillis, PageTime, Rfc3339, Seconds, Timestamp, UnixMillis,
    UnixSeconds,
};
pub use status::{
    ApiFailure, HttpStatus, RefusalCode, RotationStatus, SnapshotStatus, StatusError,
    UnreadableCause, WriteStatus,
};
pub use text_offset::{
    ByteCount, ByteDelta, ByteOffset, ByteSpan, ByteVec, CharCount, CharDelta, CharOffset,
    CharSpan, CharVec, Utf16Count, Utf16Delta, Utf16Offset, Utf16Span, Utf16Vec,
    byte_span_to_chars, byte_to_char, char_len, char_span_to_bytes, char_to_byte, char_to_utf16,
    utf16_to_char,
};
pub use unit::{
    CssPoint, CssPx, CssVector, DocPoint, DocPx, ImagePx, ImageSize, Mm, PX_PER_MM, PixelSize,
    Point, Pt, Radians, SurfacePx, Vector, ViewPoint, ViewPx, ViewVector,
};
pub use uuid::Uuid;
pub use version::Version;
pub use zoom::ZoomFactor;
