//! Fundamental types shared by all Ambre crates.
//!
//! - [`Uuid`]: UUIDv7 wrapper (`[u8; 16]`)
//! - [`DocumentId`], [`UserId`], [`ChannelId`], [`MeetingId`], [`SnapshotId`]:
//!   strongly-typed identifiers (distinct newtypes over `Uuid`)
//! - [`DocType`]: enum of document types (Prose, Trame, Toile, Murmure)
//! - [`PublicKey`]: wrapper `[u8; 32]` for Ed25519 or X25519
//! - [`Version`]: version byte for serialization
//! - [`Error`]: core error types

pub mod doc_type;
pub mod error;
pub mod id;
pub mod public_key;
pub mod role;
pub mod uuid;
pub mod version;

// Re‑export the main types for convenient use.
pub use doc_type::DocType;
pub use error::{DocTypeError, Error, PublicKeyError, UuidError, VersionError};
pub use id::{ChannelId, DocumentId, MeetingId, SnapshotId, UserId};
pub use public_key::PublicKey;
pub use role::Role;
pub use uuid::Uuid;
pub use version::Version;
