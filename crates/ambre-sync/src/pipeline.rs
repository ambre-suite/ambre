//! Compression + cryptographic envelope for the three wire messages.
//!
//! Pure functions (no `SyncState`), host-testable without network or WASM. Each
//! outbound CRDT payload is compressed (a one-byte version prefix records how),
//! the version+body plaintext is sealed with `ambre-crypto`, and the parts are
//! assembled into an `ambre_protocol::{Update,Snapshot,Ephemeral}`. The inbound
//! path is the exact inverse. The version byte lives **inside** the ciphertext,
//! so it stays opaque to the Blind Relay.
//!
//! Pipeline reference: ambre-spec-sync.md §2.B.

use std::io::Read;

use ambre_crypto::{CryptoError, DocumentKey, Sealed, envelope};
use ambre_protocol::messages::{Ciphertext, CommitmentTag, Nonce, Signature};
use ambre_protocol::public_data::{EphemeralPublicData, SnapshotPublicData, UpdatePublicData};
use ambre_protocol::{Ephemeral, Snapshot, Update};
use ed25519_dalek::SigningKey;
use ruzstd::encoding::{CompressionLevel, compress_to_vec};
use thiserror::Error;

/// Payloads at or below this size skip compression: zstd framing would add more
/// bytes than it saves, and the CPU is wasted.
const COMPRESS_THRESHOLD: usize = 256;

/// Compression scheme of a sealed plaintext, encoded as the first plaintext byte.
#[repr(u8)]
enum CompressionVersion {
    Raw = 0x00,
    Zstd = 0x01,
}

/// Failure of the compression/crypto pipeline. Any variant means the caller
/// (Sync) drops the message and raises a `SyncWarning`.
#[derive(Debug, Error)]
pub enum PipelineError {
    #[error("crypto failure: {0}")]
    Crypto(#[from] CryptoError),
    #[error("unknown compression version: {0:#x}")]
    UnknownVersion(u8),
    #[error("decompression failed")]
    Decompress,
    #[error("public data serialization failed")]
    Postcard,
}

// ── Compression layer ─────────────────────────────────────────────────────────

/// Compresses `data` when it exceeds [`COMPRESS_THRESHOLD`], returning the
/// version byte and the (possibly compressed) body.
fn compress(data: &[u8]) -> (u8, Vec<u8>) {
    if data.len() > COMPRESS_THRESHOLD {
        (
            CompressionVersion::Zstd as u8,
            compress_to_vec(data, CompressionLevel::Fastest),
        )
    } else {
        (CompressionVersion::Raw as u8, data.to_vec())
    }
}

/// Reverses [`compress`] given the version byte and body.
fn decompress(version: u8, body: &[u8]) -> Result<Vec<u8>, PipelineError> {
    match version {
        v if v == CompressionVersion::Raw as u8 => Ok(body.to_vec()),
        v if v == CompressionVersion::Zstd as u8 => {
            let mut decoder = ruzstd::decoding::StreamingDecoder::new(body)
                .map_err(|_| PipelineError::Decompress)?;
            let mut out = Vec::new();
            decoder
                .read_to_end(&mut out)
                .map_err(|_| PipelineError::Decompress)?;
            Ok(out)
        }
        other => Err(PipelineError::UnknownVersion(other)),
    }
}

// ── Generic seal / open over the (version || body) plaintext ──────────────────

fn seal_payload(
    payload: &[u8],
    aad: &[u8],
    doc_key: &DocumentKey,
    signing_key: &SigningKey,
) -> Sealed {
    let (version, body) = compress(payload);
    let mut plaintext = Vec::with_capacity(1 + body.len());
    plaintext.push(version);
    plaintext.extend_from_slice(&body);
    envelope::seal(&plaintext, aad, doc_key, signing_key)
}

fn open_payload(
    sealed: &Sealed,
    aad: &[u8],
    doc_key: &DocumentKey,
    author_pub: &[u8; 32],
) -> Result<Vec<u8>, PipelineError> {
    let plaintext = envelope::open(sealed, aad, doc_key, author_pub)?;
    let (version, body) = plaintext.split_first().ok_or(PipelineError::Decompress)?;
    decompress(*version, body)
}

fn aad_of<T: serde::Serialize>(public_data: &T) -> Result<Vec<u8>, PipelineError> {
    postcard::to_allocvec(public_data).map_err(|_| PipelineError::Postcard)
}

/// Rebuilds a [`Sealed`] from a wire message's envelope fields.
fn sealed_from(
    ciphertext: &Ciphertext,
    nonce: &Nonce,
    commitment_tag: &CommitmentTag,
    signature: &Signature,
) -> Sealed {
    Sealed {
        nonce: nonce.0,
        ciphertext: ciphertext.0.clone(),
        commitment_tag: commitment_tag.0,
        signature: signature.0,
    }
}

fn message_parts(sealed: Sealed) -> (Ciphertext, Nonce, CommitmentTag, Signature) {
    (
        Ciphertext(sealed.ciphertext),
        Nonce(sealed.nonce),
        CommitmentTag(sealed.commitment_tag),
        Signature(sealed.signature),
    )
}

// ── Update ────────────────────────────────────────────────────────────────────

/// Compresses, encrypts and signs `payload` into an [`Update`].
pub fn seal_update(
    payload: &[u8],
    public_data: UpdatePublicData,
    doc_key: &DocumentKey,
    signing_key: &SigningKey,
) -> Result<Update, PipelineError> {
    let aad = aad_of(&public_data)?;
    let sealed = seal_payload(payload, &aad, doc_key, signing_key);
    let (ciphertext, nonce, commitment_tag, signature) = message_parts(sealed);
    tracing::debug!(
        "[Pipeline] seal_update: {} bytes plaintext → {} bytes ciphertext",
        payload.len(),
        ciphertext.0.len(),
    );
    Ok(Update {
        ciphertext,
        nonce,
        public_data,
        commitment_tag,
        signature,
    })
}

/// Verifies, decrypts and decompresses an [`Update`] back to its CRDT payload.
#[cfg_attr(not(any(test, target_arch = "wasm32")), allow(dead_code))]
pub fn open_update(update: &Update, doc_key: &DocumentKey) -> Result<Vec<u8>, PipelineError> {
    let aad = aad_of(&update.public_data)?;
    let sealed = sealed_from(
        &update.ciphertext,
        &update.nonce,
        &update.commitment_tag,
        &update.signature,
    );
    let plaintext = open_payload(
        &sealed,
        &aad,
        doc_key,
        update.public_data.pub_key.as_array(),
    )?;
    tracing::debug!(
        "[Pipeline] open_update: {} bytes ciphertext → {} bytes plaintext",
        update.ciphertext.0.len(),
        plaintext.len(),
    );
    Ok(plaintext)
}

// ── Snapshot ──────────────────────────────────────────────────────────────────

/// Compresses, encrypts and signs `payload` into a [`Snapshot`].
pub fn seal_snapshot(
    payload: &[u8],
    public_data: SnapshotPublicData,
    doc_key: &DocumentKey,
    signing_key: &SigningKey,
) -> Result<Snapshot, PipelineError> {
    let aad = aad_of(&public_data)?;
    let sealed = seal_payload(payload, &aad, doc_key, signing_key);
    let (ciphertext, nonce, commitment_tag, signature) = message_parts(sealed);
    Ok(Snapshot {
        ciphertext,
        nonce,
        public_data,
        commitment_tag,
        signature,
    })
}

/// Verifies, decrypts and decompresses a [`Snapshot`] back to its CRDT payload.
// Used by the S3 snapshot-download path (not yet wired); kept symmetric with
// `seal_snapshot` and covered by the round-trip test.
#[allow(dead_code)]
pub fn open_snapshot(snapshot: &Snapshot, doc_key: &DocumentKey) -> Result<Vec<u8>, PipelineError> {
    let aad = aad_of(&snapshot.public_data)?;
    let sealed = sealed_from(
        &snapshot.ciphertext,
        &snapshot.nonce,
        &snapshot.commitment_tag,
        &snapshot.signature,
    );
    open_payload(
        &sealed,
        &aad,
        doc_key,
        snapshot.public_data.pub_key.as_array(),
    )
}

// ── Ephemeral ─────────────────────────────────────────────────────────────────

/// Compresses, encrypts and signs `payload` into an [`Ephemeral`].
pub fn seal_ephemeral(
    payload: &[u8],
    public_data: EphemeralPublicData,
    doc_key: &DocumentKey,
    signing_key: &SigningKey,
) -> Result<Ephemeral, PipelineError> {
    let aad = aad_of(&public_data)?;
    let sealed = seal_payload(payload, &aad, doc_key, signing_key);
    let (ciphertext, nonce, commitment_tag, signature) = message_parts(sealed);
    Ok(Ephemeral {
        ciphertext,
        nonce,
        public_data,
        commitment_tag,
        signature,
    })
}

/// Verifies, decrypts and decompresses an [`Ephemeral`] back to its payload.
#[cfg_attr(not(any(test, target_arch = "wasm32")), allow(dead_code))]
pub fn open_ephemeral(
    ephemeral: &Ephemeral,
    doc_key: &DocumentKey,
) -> Result<Vec<u8>, PipelineError> {
    let aad = aad_of(&ephemeral.public_data)?;
    let sealed = sealed_from(
        &ephemeral.ciphertext,
        &ephemeral.nonce,
        &ephemeral.commitment_tag,
        &ephemeral.signature,
    );
    open_payload(
        &sealed,
        &aad,
        doc_key,
        ephemeral.public_data.pub_key.as_array(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use ambre_core::{DocumentId, PublicKey, SnapshotId};
    use ambre_crypto::MasterSeed;
    use ambre_protocol::messages::{Clock, Counter};

    fn keys() -> (DocumentKey, SigningKey, PublicKey) {
        let doc_key = DocumentKey::from_array([0x11; 32]);
        let signing = MasterSeed::from_array([0x22; 32]).derive_signing_key();
        let author = PublicKey::from_array(signing.verifying_key().to_bytes());
        (doc_key, signing, author)
    }

    fn update_pd(author: PublicKey) -> UpdatePublicData {
        UpdatePublicData {
            document_id: DocumentId::new(),
            pub_key: author,
            snapshot_id: SnapshotId::new(),
            clock: Clock(1),
        }
    }

    #[test]
    fn roundtrip_update_small() {
        let (doc_key, signing, author) = keys();
        let payload = b"small crdt delta";
        let update = seal_update(payload, update_pd(author), &doc_key, &signing).unwrap();
        assert_eq!(open_update(&update, &doc_key).unwrap(), payload);
    }

    #[test]
    fn roundtrip_snapshot_large_compresses() {
        let (doc_key, signing, author) = keys();
        let payload = vec![0x7Au8; 4096]; // highly compressible
        let pd = SnapshotPublicData {
            document_id: DocumentId::new(),
            pub_key: author,
            snapshot_id: SnapshotId::new(),
            parent_snapshot_proof: Vec::new(),
            parent_snapshot_update_clocks: Vec::new(),
        };
        let snapshot = seal_snapshot(&payload, pd, &doc_key, &signing).unwrap();
        // Compressed ciphertext is far smaller than the 4 KiB plaintext.
        assert!(snapshot.ciphertext.0.len() < payload.len());
        assert_eq!(open_snapshot(&snapshot, &doc_key).unwrap(), payload);
    }

    #[test]
    fn roundtrip_ephemeral() {
        let (doc_key, signing, author) = keys();
        let payload = b"cursor@42";
        let pd = EphemeralPublicData {
            document_id: DocumentId::new(),
            pub_key: author,
            counter: Counter(7),
        };
        let eph = seal_ephemeral(payload, pd, &doc_key, &signing).unwrap();
        assert_eq!(open_ephemeral(&eph, &doc_key).unwrap(), payload);
    }

    #[test]
    fn compression_threshold() {
        let small = vec![0u8; COMPRESS_THRESHOLD];
        let (v_small, _) = compress(&small);
        assert_eq!(v_small, CompressionVersion::Raw as u8);

        let large = vec![0u8; COMPRESS_THRESHOLD + 1];
        let (v_large, body) = compress(&large);
        assert_eq!(v_large, CompressionVersion::Zstd as u8);
        assert_eq!(decompress(v_large, &body).unwrap(), large);
    }

    #[test]
    fn decompress_unknown_version_rejected() {
        assert!(matches!(
            decompress(0xFF, b"x"),
            Err(PipelineError::UnknownVersion(0xFF))
        ));
    }

    #[test]
    fn wrong_document_key_rejected() {
        let (doc_key, signing, author) = keys();
        let update = seal_update(b"secret", update_pd(author), &doc_key, &signing).unwrap();
        let other = DocumentKey::from_array([0x44; 32]);
        assert!(open_update(&update, &other).is_err());
    }

    #[test]
    fn tampered_document_id_rejected() {
        let (doc_key, signing, author) = keys();
        let mut update = seal_update(b"secret", update_pd(author), &doc_key, &signing).unwrap();
        // Forge the routing metadata: signature still passes (attacker re-signs is
        // not modeled here), but the aad-bound commitment/AEAD no longer match.
        update.public_data.document_id = DocumentId::new();
        assert!(open_update(&update, &doc_key).is_err());
    }
}
