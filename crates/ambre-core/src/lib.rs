//! Fundamental types shared by all Ambre crates.
//!
//! - [`Uuid`]: UUIDv7 wrapper (`[u8; 16]`)
//! - [`DocumentId`], [`UserId`], [`ChannelId`], [`SpaceId`], [`MetaId`], [`SegmentId`],
//!   [`CategoryId`], [`MeetingId`], [`SnapshotId`], and the Prose annotation ids
//!   [`FootnoteId`], [`CommentId`], [`ReplyId`], [`SuggestionId`]:
//!   strongly-typed identifiers (distinct newtypes over `Uuid`)
//! - [`DocType`]: enum of document types (Prose, Trame, Toile, Murmure)
//! - [`PublicKey`]: wrapper `[u8; 32]` for Ed25519 or X25519
//! - [`Seq`]: the relay's sequence number of a document
//! - [`Version`]: version byte for serialization
//! - [`schema`]: the persisted schema version and its migration walk
//! - [`authorship`]: the session nonce and the peer registry entry
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
pub mod channel_kind;
pub mod doc_type;
pub mod docfile;
pub mod email;
pub mod error;
/// Fixture machinery (`ambre-spec-fixtures.md`). Behind the non-default
/// `fixtures` feature so no client build can reach it (I4).
#[cfg(feature = "fixtures")]
pub mod fixtures;
pub mod id;
pub mod key_hex;
pub mod key_id;
pub mod media;
pub mod object_key;
pub mod public_key;
/// Which documents a client holds, and which leave first
/// (`ambre-spec-residency.md`).
pub mod residency;
pub mod role;
/// The persisted schema version: the register, the read rule and the migration
/// walk every application shares (`ambre-spec-schema-version.md`).
pub mod schema;
pub mod seq;
/// Serde adapters for enum serialization as lowercase strings.
pub mod serde_enums;
/// Serde adapters for identifier serialization as hyphenated UUID strings.
pub mod serde_id;
pub mod span;
pub mod status;
pub mod uuid;
pub mod version;

// Re-export the main types for convenient use.
pub use authorship::{PeerEntry, SessionNonce};
pub use channel_kind::ChannelDocumentKind;
pub use doc_type::DocType;
pub use docfile::{
    AMBRE_MAGIC, AMBRE_SEALED_VERSION, AMBRE_VERSION, AmbreHeader, ContainerKind, DocFileError,
    HEADER_LEN, SALT_LEN, SEAL_NONCE_LEN, SEALED_AAD_LEN, SEALED_HEADER_LEN, SealedAmbre, classify,
    migrate_ambre, parse_sealed_header, read_ambre_plaintext, read_ambre_plaintext_as, sealed_aad,
    write_ambre_plaintext, write_ambre_sealed,
};
pub use email::Email;
pub use error::{DocTypeError, Error, PublicKeyError, UuidError, VersionError};
#[cfg(feature = "fixtures")]
pub use fixtures::{
    FIXTURE_IMPORT_PEER, FixtureGen, FixtureManifest, FixtureShape, ShapeBody, fixture_peer_count,
    seeded_document_id,
};
pub use id::{
    CategoryId, ChannelId, CommentId, DocumentId, FootnoteId, LeaseId, MeetingId, MessageId,
    MetaId, OrgId, ReplyId, SegmentId, SnapshotId, SpaceId, SuggestionId, UserId,
};
pub use key_hex::{ExchangeKeyHex, SigningKeyHex};
pub use key_id::{KEY_ID_LEN, KeyId, KeyIdError};
pub use media::{ByteSize, ContentHash, MediaType};
pub use object_key::ObjectKey;
pub use public_key::PublicKey;
pub use role::Role;
pub use schema::{
    SCHEMA_META_MAP, SCHEMA_VERSION_ABSENT, SCHEMA_VERSION_KEY, SchemaError, SchemaMigration,
    SchemaVerdict, migrate_schema, migration_peer, scaffold_peer, verdict,
};
pub use seq::Seq;
pub use span::{Millis, Seconds};
pub use status::{RefusalCode, RotationStatus, SnapshotStatus, StatusError, WriteStatus};
pub use uuid::Uuid;
pub use version::Version;
