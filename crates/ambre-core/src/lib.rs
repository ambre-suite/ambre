//! Fundamental types shared by all Ambre crates.
//!
//! - [`Uuid`]: UUIDv7 wrapper (`[u8; 16]`)
//! - [`DocType`]: enum of document types (Prose, Trame, Toile, Murmure)
//! - [`PublicKey`]: wrapper `[u8; 32]` for Ed25519 or X25519
//! - [`Version`]: version byte for serialization
//! - [`Error`]: core error types

pub mod doc_type;
pub mod error;
pub mod public_key;
pub mod role;
pub mod uuid;
pub mod version;

// Re‑export the main types for convenient use.
pub use doc_type::DocType;
pub use error::{DocTypeError, Error, PublicKeyError, UuidError, VersionError};
pub use public_key::PublicKey;
pub use role::Role;
pub use uuid::Uuid;
pub use version::Version;
