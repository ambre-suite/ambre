//! BLAKE2b hashing for the snapshot integrity chain.
//!
//! Each snapshot carries a `parent_snapshot_proof` linking it to its parent
//! without decryption. The chain hash folds the parent proof with the new
//! snapshot bytes, so any reordering or substitution breaks the chain.

use blake2::{Blake2b512, Digest};
use data_encoding::BASE32_NOPAD;

/// Length of a chain hash, in bytes.
pub const HASH_LEN: usize = 32;

/// Domain-separation tag for the TOFU public-key fingerprint. Distinct from the
/// key-derivation tags in `keys.rs` so a fingerprint can never collide with a
/// derived key digest over the same input.
const FINGERPRINT_DOMAIN: &[u8] = b"ambre-fingerprint-v1";

/// Bytes of the fingerprint retained after truncation: 20 bytes = 160 bits, far
/// beyond collision reach yet short enough to read aloud over the phone.
const FINGERPRINT_LEN: usize = 20;

/// Human-verifiable fingerprint of a public key for the TOFU trust check
/// (`ambre-spec-server.md` "Public Key Trust Model (TOFU)",
/// `ambre-spec-chrome.md §2.H`).
///
/// Computes `BLAKE2b-256(domain || pub_key)`, keeps the first 20 bytes, and
/// base32-encodes them (RFC 4648, no padding), grouping the 32 output characters
/// into 4-char blocks separated by spaces so the string reads cleanly aloud for
/// out-of-band verification. The output is stable for a given key: the same key
/// always yields the same string.
pub fn fingerprint(pub_key: &[u8; 32]) -> String {
    let mut hasher = Blake2b512::new();
    hasher.update(FINGERPRINT_DOMAIN);
    hasher.update(pub_key);
    let digest = hasher.finalize();
    let encoded = BASE32_NOPAD.encode(&digest[..FINGERPRINT_LEN]);
    // Group into 4-char blocks: "XXXX XXXX ..." (8 groups for 32 chars).
    encoded
        .as_bytes()
        .chunks(4)
        .map(|c| std::str::from_utf8(c).expect("base32 output is ASCII"))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Computes the chain hash `BLAKE2b(len(parent_proof) || parent_proof ||
/// snapshot_bytes)`, truncated to 32 bytes. The length prefix makes the field
/// boundary unambiguous, so distinct `(parent, snapshot)` splits never collide.
pub fn chain_hash(parent_proof: &[u8], snapshot_bytes: &[u8]) -> [u8; HASH_LEN] {
    let mut hasher = Blake2b512::new();
    hasher.update((parent_proof.len() as u64).to_le_bytes());
    hasher.update(parent_proof);
    hasher.update(snapshot_bytes);
    let digest = hasher.finalize();
    let mut out = [0u8; HASH_LEN];
    out.copy_from_slice(&digest[..HASH_LEN]);
    out
}
