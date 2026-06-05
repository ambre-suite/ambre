//! BLAKE2b hashing for the snapshot integrity chain.
//!
//! Each snapshot carries a `parent_snapshot_proof` linking it to its parent
//! without decryption. The chain hash folds the parent proof with the new
//! snapshot bytes, so any reordering or substitution breaks the chain.

use blake2::{Blake2b512, Digest};

/// Length of a chain hash, in bytes.
pub const HASH_LEN: usize = 32;

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
