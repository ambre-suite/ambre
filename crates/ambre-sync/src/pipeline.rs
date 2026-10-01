//! Compression + cryptographic envelope for the three wire messages.
//!
//! Pure functions (no `SyncState`), host-testable without network or WASM. Each
//! outbound CRDT payload is compressed (a one-byte version prefix records how),
//! and the version+body plaintext is sealed with `ambre-crypto`. The parts are
//! then assembled into an `ambre_protocol::{Update,Snapshot,Ephemeral}`. The
//! inbound path is the exact inverse. The version byte lives **inside** the
//! ciphertext, so it stays opaque to the Blind Relay.
//!
//! Pipeline reference: `ambre-spec-sealed-envelope.md`.

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

/// Maximum decompression ratio accepted on the inbound path. zstd routinely
/// reaches 1000:1+ on adversarial input, so a bomb-shaped plaintext could OOM
/// the WASM Sync worker. The DocumentKey is shared with every collaborator, so a
/// malicious-but-authorized peer can craft such a plaintext *after* passing
/// AEAD/commitment/signature verification (H9). We bound the output at
/// `max(body.len() * MAX_DECOMPRESS_RATIO, MIN_DECOMPRESS_CEILING)`, capped by
/// [`ABSOLUTE_DECOMPRESS_CAP`], and reject anything larger before it fully
/// allocates.
const MAX_DECOMPRESS_RATIO: usize = 200;

/// Floor for the per-message decompression ceiling. Tiny legitimate frames (a
/// few compressed bytes expanding to a normal delta) are never rejected by
/// the ratio bound alone.
const MIN_DECOMPRESS_CEILING: usize = 1 << 20; // 1 MiB

/// Hard upper bound on any single decompressed message, regardless of the
/// ratio-derived ceiling. Matches the client-memory pillar (< 100 MB per doc).
const ABSOLUTE_DECOMPRESS_CAP: usize = 64 << 20; // 64 MiB

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
    #[error("decompressed size exceeds the per-message ceiling")]
    DecompressTooLarge,
    #[error("public data serialization failed")]
    Postcard,
    #[error("snapshot validation failed: {0}")]
    Validation(#[from] ambre_protocol::error::ValidationError),
    #[error("the snapshot names another document")]
    WrongDocument,
    #[error("a sealed object is shorter than its header")]
    ShortObject,
}

// -- Compression layer ---------------------------------------------------------

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
        v if v == CompressionVersion::Zstd as u8 => decompress_zstd_bounded(body),
        other => Err(PipelineError::UnknownVersion(other)),
    }
}

/// Streams a zstd body into memory with a hard output ceiling (H9). The ceiling
/// is derived from the compressed length and [`MAX_DECOMPRESS_RATIO`], floored
/// at [`MIN_DECOMPRESS_CEILING`] and capped at [`ABSOLUTE_DECOMPRESS_CAP`]. We
/// read in bounded chunks and abort as soon as the accumulated output would
/// exceed the ceiling. A decompression bomb never allocates the full output.
fn decompress_zstd_bounded(body: &[u8]) -> Result<Vec<u8>, PipelineError> {
    // MIN_DECOMPRESS_CEILING < ABSOLUTE_DECOMPRESS_CAP by construction, so clamp
    // never panics.
    let ceiling = body
        .len()
        .saturating_mul(MAX_DECOMPRESS_RATIO)
        .clamp(MIN_DECOMPRESS_CEILING, ABSOLUTE_DECOMPRESS_CAP);

    let mut decoder =
        ruzstd::decoding::StreamingDecoder::new(body).map_err(|_| PipelineError::Decompress)?;

    // Never pre-allocate the whole ceiling. Grow the buffer as we go, reading
    // one byte past the ceiling to distinguish "exactly at the limit" from
    // "over the limit". `take` caps the reader at `ceiling + 1` bytes.
    let mut out = Vec::new();
    let read = decoder
        .by_ref()
        .take(ceiling as u64 + 1)
        .read_to_end(&mut out)
        .map_err(|_| PipelineError::Decompress)?;

    if read > ceiling {
        return Err(PipelineError::DecompressTooLarge);
    }
    Ok(out)
}

// -- Generic seal / open over the (version || body) plaintext ------------------

fn seal_payload(
    payload: &[u8],
    aad: &[u8],
    doc_key: &DocumentKey,
    signing_key: &SigningKey,
) -> Result<Sealed, PipelineError> {
    let (version, body) = compress(payload);
    let mut plaintext = Vec::with_capacity(1 + body.len());
    plaintext.push(version);
    plaintext.extend_from_slice(&body);
    Ok(envelope::seal(&plaintext, aad, doc_key, signing_key)?)
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

// -- Update --------------------------------------------------------------------

/// Compresses, encrypts and signs `payload` into an [`Update`].
pub fn seal_update(
    payload: &[u8],
    public_data: UpdatePublicData,
    doc_key: &DocumentKey,
    signing_key: &SigningKey,
) -> Result<Update, PipelineError> {
    let aad = aad_of(&public_data)?;
    let sealed = seal_payload(payload, &aad, doc_key, signing_key)?;
    let (ciphertext, nonce, commitment_tag, signature) = message_parts(sealed);
    tracing::debug!(
        "[Pipeline] seal_update: {} bytes plaintext -> {} bytes ciphertext",
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
        "[Pipeline] open_update: {} bytes ciphertext -> {} bytes plaintext",
        update.ciphertext.0.len(),
        plaintext.len(),
    );
    Ok(plaintext)
}

// -- Snapshot ------------------------------------------------------------------

/// Compresses, encrypts and signs `payload` into a [`Snapshot`].
pub fn seal_snapshot(
    payload: &[u8],
    public_data: SnapshotPublicData,
    doc_key: &DocumentKey,
    signing_key: &SigningKey,
) -> Result<Snapshot, PipelineError> {
    let aad = aad_of(&public_data)?;
    let sealed = seal_payload(payload, &aad, doc_key, signing_key)?;
    let (ciphertext, nonce, commitment_tag, signature) = message_parts(sealed);
    let snapshot = Snapshot {
        ciphertext,
        nonce,
        public_data,
        commitment_tag,
        signature,
    };
    // Enforce the wire invariants before upload: the Blind Relay never sees this
    // struct (snapshots go client-to-S3), so this is the only enforcement point.
    snapshot.validate()?;
    Ok(snapshot)
}

/// Decode a stored snapshot blob, the serialized [`Snapshot`] object storage
/// holds, and open it for `document_id`. A snapshot sealed for another document
/// is refused before any decryption.
///
/// # Errors
///
/// [`PipelineError::Postcard`] for bytes that are not a snapshot,
/// [`PipelineError::WrongDocument`] for another document's, and every error of
/// [`open_snapshot`].
pub fn open_snapshot_blob(
    blob: &[u8],
    document_id: ambre_bus::DocumentId,
    doc_key: &DocumentKey,
) -> Result<Vec<u8>, PipelineError> {
    let snapshot: Snapshot = postcard::from_bytes(blob).map_err(|_| PipelineError::Postcard)?;
    if snapshot.public_data.document_id != document_id {
        return Err(PipelineError::WrongDocument);
    }
    open_snapshot(&snapshot, doc_key)
}

/// Verifies, decrypts and decompresses a [`Snapshot`] back to its CRDT payload.
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

// -- Ephemeral -----------------------------------------------------------------

/// Compresses, encrypts and signs `payload` into an [`Ephemeral`].
pub fn seal_ephemeral(
    payload: &[u8],
    public_data: EphemeralPublicData,
    doc_key: &DocumentKey,
    signing_key: &SigningKey,
) -> Result<Ephemeral, PipelineError> {
    let aad = aad_of(&public_data)?;
    let sealed = seal_payload(payload, &aad, doc_key, signing_key)?;
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

// -- Standalone objects: a document name, a media blob -------------------------

/// Bytes of the author key that opens a standalone object.
const AUTHOR_KEY_LEN: usize = 32;
/// Bytes before the ciphertext of a standalone object.
const OBJECT_HEADER_LEN: usize = AUTHOR_KEY_LEN
    + ambre_crypto::signature::SIGNATURE_LEN
    + ambre_crypto::commitment::TAG_LEN
    + ambre_crypto::aead::NONCE_LEN;

/// Domain of a sealed document name. The document id follows it in the
/// additional data, so a name opens under its own document only (`SYNC-068`).
const NAME_DOMAIN: &[u8] = b"ambre-doc-name-v2";

/// The additional data of a document name: the domain, then the document id.
pub fn name_aad(document_id: ambre_core::DocumentId) -> Vec<u8> {
    let mut aad = NAME_DOMAIN.to_vec();
    aad.extend_from_slice(document_id.as_bytes());
    aad
}

/// Domain of a sealed media blob. The document id and the address follow it,
/// so a blob opens under its own document and address only (`SYNC-051`,
/// `SYNC-074`).
const MEDIA_DOMAIN: &[u8] = b"ambre-media-v2";

/// The additional data of a media blob: the domain, the document id, then the
/// address the document stores.
pub fn media_aad(
    document_id: ambre_core::DocumentId,
    address: &[u8; ambre_crypto::commitment::TAG_LEN],
) -> Vec<u8> {
    let mut aad = MEDIA_DOMAIN.to_vec();
    aad.extend_from_slice(document_id.as_bytes());
    aad.extend_from_slice(address);
    aad
}

/// Seals a payload that travels outside a wire message: a document name or a
/// media blob. It takes the full envelope: the cipher, the commitment tag and the
/// author signature (`SYNC-018`). Nothing is compressed: a name is short, and a
/// blob is a format that is already compressed.
///
/// Layout: `author key || signature || commitment tag || nonce || ciphertext`.
pub fn seal_object(
    plaintext: &[u8],
    aad: &[u8],
    doc_key: &DocumentKey,
    signing_key: &SigningKey,
) -> Result<Vec<u8>, PipelineError> {
    let sealed = envelope::seal(plaintext, aad, doc_key, signing_key)?;
    let mut out = Vec::with_capacity(OBJECT_HEADER_LEN + sealed.ciphertext.len());
    out.extend_from_slice(signing_key.verifying_key().as_bytes());
    out.extend_from_slice(&sealed.signature);
    out.extend_from_slice(&sealed.commitment_tag);
    out.extend_from_slice(&sealed.nonce);
    out.extend_from_slice(&sealed.ciphertext);
    Ok(out)
}

/// Opens what [`seal_object`] sealed, in the envelope's order: the signature,
/// then the commitment tag, then the cipher.
///
/// # Errors
///
/// [`PipelineError::ShortObject`] when the bytes cannot hold the header, and
/// [`PipelineError::Crypto`] when any of the three checks fails.
pub fn open_object(
    bytes: &[u8],
    aad: &[u8],
    doc_key: &DocumentKey,
) -> Result<Vec<u8>, PipelineError> {
    if bytes.len() < OBJECT_HEADER_LEN {
        return Err(PipelineError::ShortObject);
    }
    let (author, rest) = bytes.split_at(AUTHOR_KEY_LEN);
    let (signature, rest) = rest.split_at(ambre_crypto::signature::SIGNATURE_LEN);
    let (tag, rest) = rest.split_at(ambre_crypto::commitment::TAG_LEN);
    let (nonce, ciphertext) = rest.split_at(ambre_crypto::aead::NONCE_LEN);
    let sealed = Sealed {
        nonce: nonce.try_into().map_err(|_| PipelineError::ShortObject)?,
        ciphertext: ciphertext.to_vec(),
        commitment_tag: tag.try_into().map_err(|_| PipelineError::ShortObject)?,
        signature: signature
            .try_into()
            .map_err(|_| PipelineError::ShortObject)?,
    };
    let author: [u8; AUTHOR_KEY_LEN] = author.try_into().map_err(|_| PipelineError::ShortObject)?;
    Ok(envelope::open(&sealed, aad, doc_key, &author)?)
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
    fn decompress_bomb_rejected() {
        // A few hundred compressed bytes expanding to 8 MiB is well past the 1
        // MiB floor with a small body. The ratio-derived ceiling rejects it.
        let bomb = vec![0u8; 8 << 20];
        let compressed = compress_to_vec(bomb.as_slice(), CompressionLevel::Fastest);
        assert!(compressed.len() * MAX_DECOMPRESS_RATIO < bomb.len());
        assert!(matches!(
            decompress(CompressionVersion::Zstd as u8, &compressed),
            Err(PipelineError::DecompressTooLarge)
        ));
    }

    #[test]
    fn decompress_within_ceiling_ok() {
        // A legitimate ~4 KiB payload round-trips unaffected by the bound.
        let payload = vec![0x5Au8; 4096];
        let compressed = compress_to_vec(payload.as_slice(), CompressionLevel::Fastest);
        assert_eq!(
            decompress(CompressionVersion::Zstd as u8, &compressed).unwrap(),
            payload
        );
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

    /// A standalone object carries the author signature and the commitment
    /// tag, and each is checked on the open (`SYNC-018`).
    #[test]
    fn a_sealed_object_carries_the_signature_and_the_tag() {
        use ambre_crypto::{commitment, signature};

        let (doc_key, signing, _) = keys();
        let aad = b"ambre-media";
        let sealed = seal_object(b"a picture", aad, &doc_key, &signing).unwrap();
        assert_eq!(open_object(&sealed, aad, &doc_key).unwrap(), b"a picture");

        // The signature: one flipped byte, and the author no longer signed it.
        let mut forged = sealed.clone();
        forged[AUTHOR_KEY_LEN] ^= 1;
        assert!(matches!(
            open_object(&forged, aad, &doc_key),
            Err(PipelineError::Crypto(CryptoError::InvalidSignature))
        ));

        // The tag: a wrong one, re-signed so the signature passes, still fails.
        let tag_at = AUTHOR_KEY_LEN + signature::SIGNATURE_LEN;
        let nonce_at = tag_at + commitment::TAG_LEN;
        let mut moved = sealed.clone();
        moved[tag_at] ^= 1;
        let tag: [u8; commitment::TAG_LEN] = moved[tag_at..nonce_at].try_into().unwrap();
        let nonce = &moved[nonce_at..OBJECT_HEADER_LEN];
        let resigned = signature::sign(&signing, nonce, &moved[OBJECT_HEADER_LEN..], aad, &tag);
        moved[AUTHOR_KEY_LEN..tag_at].copy_from_slice(&resigned);
        assert!(matches!(
            open_object(&moved, aad, &doc_key),
            Err(PipelineError::Crypto(CryptoError::CommitmentMismatch))
        ));

        // Too short to hold the header.
        assert!(matches!(
            open_object(&sealed[..OBJECT_HEADER_LEN - 1], aad, &doc_key),
            Err(PipelineError::ShortObject)
        ));
    }

    /// A name sealed for one document does not open under another, even one
    /// that holds the same key (`SYNC-068`).
    #[test]
    fn a_name_opens_under_its_own_document_only() {
        let (doc_key, signing, _) = keys();
        let own = DocumentId::new();
        let sealed = seal_object(b"Budget 2027", &name_aad(own), &doc_key, &signing).unwrap();
        assert_eq!(
            open_object(&sealed, &name_aad(own), &doc_key).unwrap(),
            b"Budget 2027"
        );
        assert!(open_object(&sealed, &name_aad(DocumentId::new()), &doc_key).is_err());
    }
}
