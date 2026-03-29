//! Cryptographic primitives for Ambre.
//!
//! Provides AEAD (XChaCha20‑Poly1305), signatures (Ed25519), hashing (BLAKE2b),
//! key derivation (Argon2id), Diffie‑Hellman (X25519), and key storage with zeroization.
//!
//! This crate is used exclusively by the Sync Worker; no other crate imports it.
