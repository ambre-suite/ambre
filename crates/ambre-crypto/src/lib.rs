//! Cryptographic primitives for Ambre.
//!
//! Provides AEAD (XChaCha20-Poly1305), signatures (Ed25519), hashing (BLAKE2b),
//! key derivation (Argon2id), Diffie-Hellman (X25519), and key storage with
//! zeroization.
//!
//! The crate is deliberately standalone: it does **not** depend on
//! `ambre-protocol`. Envelope functions operate on raw byte slices (`aad` =
//! serialized public metadata supplied by the caller). The three wire messages
//! (Update, Snapshot, Ephemeral) are covered by a single seal/open pair.
//! Assembly into protocol types and the compression version byte live in the
//! Sync thread.

pub mod aead;
pub mod call;
pub mod commitment;
pub mod ecdh;
pub mod envelope;
pub mod error;
pub mod hash;
pub mod keys;
pub mod passphrase;
pub mod peer;
mod random;
pub mod signature;
pub mod strength;

pub use ecdh::WrappedKey;
pub use envelope::{Sealed, open, seal};
pub use error::CryptoError;
pub use hash::fingerprint;
pub use keys::{DocumentKey, FileKey, MasterSeed, StoreKey};

#[cfg(test)]
mod tests;
