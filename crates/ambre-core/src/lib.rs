//! Fundamental types shared by all Ambre crates.
//!
//! - [`Uuid`]: UUIDv7 wrapper (`[u8; 16]`)
//! - [`DocumentId`], [`UserId`], [`ChannelId`], [`MeetingId`], [`SnapshotId`]:
//!   strongly-typed identifiers (distinct newtypes over `Uuid`)
//! - [`DocType`]: enum of document types (Prose, Trame, Toile, Murmure)
//! - [`PublicKey`]: wrapper `[u8; 32]` for Ed25519 or X25519
//! - [`Version`]: version byte for serialization
//! - [`schema`]: the persisted schema version and its migration walk
//! - [`Error`]: core error types
//! - [`docfile`]: the `.ambre` container header in its two lineages -- plaintext
//!   and passphrase-sealed -- shared by every app
//! - [`fixtures`] (feature `fixtures`, host-only): the document-fixture
//!   generator contract and its sidecar manifest

// No hand-written `unsafe` in this crate (constitution pillar). Any future
// `unsafe` block breaks the build rather than silently landing.
#![forbid(unsafe_code)]

pub mod doc_type;
pub mod docfile;
pub mod error;
/// Fixture machinery (`ambre-spec-fixtures.md`). Behind the non-default
/// `fixtures` feature so no client build can reach it (I4).
#[cfg(feature = "fixtures")]
pub mod fixtures;
pub mod id;
pub mod key_id;
pub mod public_key;
pub mod role;
/// The persisted schema version: the register, the read rule and the migration
/// walk every application shares (`ambre-spec-schema-version.md`).
pub mod schema;
pub mod uuid;
pub mod version;

// Re-export the main types for convenient use.
pub use doc_type::DocType;
pub use docfile::{
    AMBRE_MAGIC, AMBRE_SEALED_VERSION, AMBRE_VERSION, AmbreHeader, ContainerKind, DocFileError,
    HEADER_LEN, SALT_LEN, SEAL_NONCE_LEN, SEALED_AAD_LEN, SEALED_HEADER_LEN, SealedAmbre, classify,
    migrate_ambre, parse_sealed_header, read_ambre_plaintext, read_ambre_plaintext_as, sealed_aad,
    write_ambre_plaintext, write_ambre_sealed,
};
pub use error::{DocTypeError, Error, PublicKeyError, UuidError, VersionError};
#[cfg(feature = "fixtures")]
pub use fixtures::{
    FIXTURE_IMPORT_PEER, FixtureGen, FixtureManifest, FixtureShape, ShapeBody, fixture_peer_count,
    seeded_document_id,
};
pub use id::{ChannelId, DocumentId, MeetingId, SnapshotId, UserId};
pub use key_id::{KEY_ID_LEN, KeyId};
pub use public_key::PublicKey;
pub use role::Role;
pub use schema::{
    SCHEMA_META_MAP, SCHEMA_VERSION_ABSENT, SCHEMA_VERSION_KEY, SchemaError, SchemaMigration,
    SchemaVerdict, migrate_schema, migration_peer, scaffold_peer, verdict,
};
pub use uuid::Uuid;
pub use version::Version;
