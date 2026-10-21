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

use ambre_core::activity::{ActivitySummary, SummaryError};
use ambre_core::authorship::{SESSION_NONCE_LEN, SessionNonce};
use ambre_core::{KEY_ID_LEN, KeyId, PublicKey};
use ambre_crypto::{CryptoError, DocumentKey, Sealed, envelope};
use ambre_protocol::messages::{Ciphertext, CommitmentTag, Nonce, Signature};
use ambre_protocol::public_data::{EphemeralPublicData, SnapshotPublicData, UpdatePublicData};
use ambre_protocol::ws::WsClientMsg;
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

/// Floor for the per-message decompression ceiling. Tiny legitimate messages (a
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
    #[error("the payload names another key than the one given")]
    WrongKey,
    #[error("unknown sealed object layout: {0:#x}")]
    UnknownLayout(u8),
    #[error("an update body is too short for its parts")]
    ShortBody,
    #[error("an update body carries an activity summary this build cannot read")]
    UnknownSummary,
    #[error("an activity summary past its bounds: {0}")]
    Summary(#[from] SummaryError),
}

// -- Update body ---------------------------------------------------------------

/// The marker byte of an update body with no activity summary.
const NO_SUMMARY: u8 = 0x00;

/// The marker byte of an update body whose activity summary follows it
/// (`SYNC-113`).
const WITH_SUMMARY: u8 = 0x01;

/// What an update carries inside its ciphertext: the session nonce of its
/// author, its activity summary when one was written, then the delta
/// (`SYNC-112`, `SYNC-113`).
#[derive(Debug, PartialEq, Eq)]
pub struct UpdateBody {
    /// The session nonce the author's PeerID derives from.
    pub nonce: SessionNonce,
    /// What the update says of itself, for a client that does not hold the
    /// document (`RESID-014`).
    pub summary: Option<ActivitySummary>,
    /// The Loro update, opaque to the envelope.
    pub delta: Vec<u8>,
}

/// The body of an update: the nonce, the summary marker, the summary when there
/// is one, then the delta. It is the payload the seal compresses and encrypts,
/// so the nonce and the summary stay inside (`SYNC-113`).
///
/// # Errors
///
/// [`PipelineError::Summary`] for a summary past its bounds.
pub fn update_body(
    nonce: &SessionNonce,
    summary: Option<&ActivitySummary>,
    delta: &[u8],
) -> Result<Vec<u8>, PipelineError> {
    let summary = summary.map(ActivitySummary::encode).transpose()?;
    let summary_len = summary.as_ref().map_or(0, Vec::len);
    let mut body = Vec::with_capacity(SESSION_NONCE_LEN + 1 + summary_len + delta.len());
    body.extend_from_slice(&nonce.0);
    match summary {
        Some(summary) => {
            body.push(WITH_SUMMARY);
            body.extend_from_slice(&summary);
        }
        None => body.push(NO_SUMMARY),
    }
    body.extend_from_slice(delta);
    Ok(body)
}

/// Splits an opened update into its nonce, its summary and its delta, the last
/// step of the open. A body too short for its parts, or one whose marker no
/// build writes, is refused.
///
/// # Errors
///
/// [`PipelineError::ShortBody`], [`PipelineError::UnknownSummary`] and
/// [`PipelineError::Summary`].
pub fn split_update_body(body: Vec<u8>) -> Result<UpdateBody, PipelineError> {
    let (nonce, rest) = body
        .split_first_chunk::<SESSION_NONCE_LEN>()
        .ok_or(PipelineError::ShortBody)?;
    let nonce = SessionNonce(*nonce);
    let (&marker, rest) = rest.split_first().ok_or(PipelineError::ShortBody)?;
    let (summary, delta) = match marker {
        NO_SUMMARY => (None, rest),
        WITH_SUMMARY => {
            let (summary, delta) = ActivitySummary::decode(rest)?;
            (Some(summary), delta)
        }
        _ => return Err(PipelineError::UnknownSummary),
    };
    Ok(UpdateBody {
        nonce,
        summary,
        delta: delta.to_vec(),
    })
}

/// Refuses a key other than the one the metadata names (`SYNC-125`). On the
/// seal it keeps a payload from naming a key that did not seal it. On the open
/// it refuses before any cryptography.
fn check_key(named: KeyId, doc_key: &DocumentKey) -> Result<(), PipelineError> {
    if named == doc_key.id() {
        Ok(())
    } else {
        Err(PipelineError::WrongKey)
    }
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
    author_pub: &PublicKey,
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
    check_key(public_data.key_id, doc_key)?;
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
    check_key(update.public_data.key_id, doc_key)?;
    let aad = aad_of(&update.public_data)?;
    let sealed = sealed_from(
        &update.ciphertext,
        &update.nonce,
        &update.commitment_tag,
        &update.signature,
    );
    let plaintext = open_payload(&sealed, &aad, doc_key, &update.public_data.pub_key)?;
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
    check_key(public_data.key_id, doc_key)?;
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

/// Decodes a stored snapshot blob, the serialized [`Snapshot`] object storage
/// holds, refusing another document's before any decryption. The caller
/// reads the key it names before opening it (`SYNC-045`, `SYNC-119`).
///
/// # Errors
///
/// [`PipelineError::Postcard`] for bytes that are not a snapshot, and
/// [`PipelineError::WrongDocument`] for another document's.
pub fn decode_snapshot_blob(
    blob: &[u8],
    document_id: ambre_bus::DocumentId,
) -> Result<Snapshot, PipelineError> {
    let snapshot: Snapshot = postcard::from_bytes(blob).map_err(|_| PipelineError::Postcard)?;
    if snapshot.public_data.document_id != document_id {
        return Err(PipelineError::WrongDocument);
    }
    Ok(snapshot)
}

/// Verifies, decrypts and decompresses a [`Snapshot`] back to its CRDT payload.
pub fn open_snapshot(snapshot: &Snapshot, doc_key: &DocumentKey) -> Result<Vec<u8>, PipelineError> {
    check_key(snapshot.public_data.key_id, doc_key)?;
    let aad = aad_of(&snapshot.public_data)?;
    let sealed = sealed_from(
        &snapshot.ciphertext,
        &snapshot.nonce,
        &snapshot.commitment_tag,
        &snapshot.signature,
    );
    open_payload(&sealed, &aad, doc_key, &snapshot.public_data.pub_key)
}

// -- Ephemeral -----------------------------------------------------------------

/// Compresses, encrypts and signs `payload` into an [`Ephemeral`].
pub fn seal_ephemeral(
    payload: &[u8],
    public_data: EphemeralPublicData,
    doc_key: &DocumentKey,
    signing_key: &SigningKey,
) -> Result<Ephemeral, PipelineError> {
    check_key(public_data.key_id, doc_key)?;
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
    check_key(ephemeral.public_data.key_id, doc_key)?;
    let aad = aad_of(&ephemeral.public_data)?;
    let sealed = sealed_from(
        &ephemeral.ciphertext,
        &ephemeral.nonce,
        &ephemeral.commitment_tag,
        &ephemeral.signature,
    );
    open_payload(&sealed, &aad, doc_key, &ephemeral.public_data.pub_key)
}

// -- What leaves the worker ----------------------------------------------------

/// A socket frame that carries a payload, built by the seal and by nothing
/// else. Its field is private to this module and no conversion fills it. So
/// the socket sends a delta or a presence sample only through
/// [`update_frame`] or [`ephemeral_frame`] (`SYNC-001`, `ZK-001`).
#[derive(Debug)]
pub struct SealedFrame(WsClientMsg);

impl SealedFrame {
    /// The message the frame carries, for the encoder.
    pub(crate) fn message(&self) -> &WsClientMsg {
        &self.0
    }

    /// The signature of a sealed update, which names the delta in the relay's
    /// echo and refusal (`SYNC-147`). `None` for any other sealed message.
    pub(crate) fn update_signature(&self) -> Option<Signature> {
        match &self.0 {
            WsClientMsg::Update(update) => Some(update.signature),
            _ => None,
        }
    }
}

/// Bytes bound for object storage, built by the seal and by nothing else. The
/// presigned upload takes this type, so a snapshot or a media blob leaves the
/// worker only sealed under the document key (`SYNC-049`, `ZK-001`).
#[derive(Debug)]
pub struct SealedBlob(Vec<u8>);

impl SealedBlob {
    /// The sealed bytes, as the upload sends them.
    pub(crate) fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    /// The size the relay is told, which is the sealed size.
    pub(crate) fn len(&self) -> usize {
        self.0.len()
    }
}

/// Seals `payload` into an update frame, through [`seal_update`].
///
/// # Errors
///
/// Every error [`seal_update`] raises.
pub fn update_frame(
    payload: &[u8],
    public_data: UpdatePublicData,
    doc_key: &DocumentKey,
    signing_key: &SigningKey,
) -> Result<SealedFrame, PipelineError> {
    seal_update(payload, public_data, doc_key, signing_key)
        .map(|update| SealedFrame(WsClientMsg::Update(update)))
}

/// Seals `payload` into an ephemeral, through [`seal_ephemeral`].
///
/// # Errors
///
/// Every error [`seal_ephemeral`] raises.
pub fn ephemeral_frame(
    payload: &[u8],
    public_data: EphemeralPublicData,
    doc_key: &DocumentKey,
    signing_key: &SigningKey,
) -> Result<SealedFrame, PipelineError> {
    seal_ephemeral(payload, public_data, doc_key, signing_key)
        .map(|ephemeral| SealedFrame(WsClientMsg::Ephemeral(ephemeral)))
}

/// Seals `payload` into the snapshot blob object storage holds, through
/// [`seal_snapshot`]: the serialized [`Snapshot`] that
/// [`decode_snapshot_blob`] reads back.
///
/// # Errors
///
/// Every error [`seal_snapshot`] raises, and [`PipelineError::Postcard`] when
/// the snapshot does not serialize.
pub fn snapshot_blob(
    payload: &[u8],
    public_data: SnapshotPublicData,
    doc_key: &DocumentKey,
    signing_key: &SigningKey,
) -> Result<SealedBlob, PipelineError> {
    let snapshot = seal_snapshot(payload, public_data, doc_key, signing_key)?;
    postcard::to_allocvec(&snapshot)
        .map(SealedBlob)
        .map_err(|_| PipelineError::Postcard)
}

/// Seals `plaintext` into a standalone object bound for object storage,
/// through [`seal_object`].
///
/// # Errors
///
/// Every error [`seal_object`] raises.
pub fn object_blob(
    plaintext: &[u8],
    aad: &[u8],
    doc_key: &DocumentKey,
    signing_key: &SigningKey,
) -> Result<SealedBlob, PipelineError> {
    seal_object(plaintext, aad, doc_key, signing_key).map(SealedBlob)
}

// -- Standalone objects: a document name, a media blob -------------------------

/// The layout of a standalone object, its first byte (`SYNC-138`). The relay
/// keeps a blob across releases, so a reader must tell an older layout from a
/// forged one. Every object of a release shares it, so it says nothing of the
/// content.
const OBJECT_LAYOUT: u8 = 1;
/// Bytes of the layout byte.
const LAYOUT_LEN: usize = 1;
/// Bytes of the author key that opens a standalone object.
const AUTHOR_KEY_LEN: usize = 32;
/// Bytes before the ciphertext of a standalone object.
const OBJECT_HEADER_LEN: usize = LAYOUT_LEN
    + KEY_ID_LEN
    + AUTHOR_KEY_LEN
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

/// The additional data an object is sealed under: the layout and the key
/// identifier its header names, then the caller's own. The signature covers
/// all three, so a header cannot be made to name another layout or key.
fn object_aad(key_id: KeyId, aad: &[u8]) -> Vec<u8> {
    let mut bound = vec![OBJECT_LAYOUT];
    bound.extend_from_slice(key_id.as_array());
    bound.extend_from_slice(aad);
    bound
}

/// Refuses an object whose first byte names a layout this build does not read,
/// before anything else in it is trusted (`SYNC-138`).
fn check_layout(bytes: &[u8]) -> Result<(), PipelineError> {
    match bytes.first() {
        None => Err(PipelineError::ShortObject),
        Some(&OBJECT_LAYOUT) => Ok(()),
        Some(&other) => Err(PipelineError::UnknownLayout(other)),
    }
}

/// Seals a payload that travels outside a wire message: a document name or a
/// media blob. It takes the full envelope: the cipher, the commitment tag and the
/// author signature (`SYNC-018`). Nothing is compressed: a name is short, and a
/// blob is a format that is already compressed. The header names the key that
/// sealed it (`SYNC-125`), after the layout it was written in (`SYNC-138`).
///
/// Layout: `layout || key id || author key || signature || commitment tag ||
/// nonce || ciphertext`.
pub fn seal_object(
    plaintext: &[u8],
    aad: &[u8],
    doc_key: &DocumentKey,
    signing_key: &SigningKey,
) -> Result<Vec<u8>, PipelineError> {
    let key_id = doc_key.id();
    let sealed = envelope::seal(plaintext, &object_aad(key_id, aad), doc_key, signing_key)?;
    Ok(object_bytes(key_id, signing_key, &sealed))
}

/// Lays out a sealed object: `layout || key id || author key || signature ||
/// commitment tag || nonce || ciphertext`.
fn object_bytes(key_id: KeyId, signing_key: &SigningKey, sealed: &Sealed) -> Vec<u8> {
    let mut out = Vec::with_capacity(OBJECT_HEADER_LEN + sealed.ciphertext.len());
    out.push(OBJECT_LAYOUT);
    out.extend_from_slice(key_id.as_array());
    out.extend_from_slice(signing_key.verifying_key().as_bytes());
    out.extend_from_slice(&sealed.signature);
    out.extend_from_slice(&sealed.commitment_tag);
    out.extend_from_slice(&sealed.nonce);
    out.extend_from_slice(&sealed.ciphertext);
    out
}

/// Splits what [`object_bytes`] laid out, refusing a key other than
/// `doc_key` before any cryptography (`SYNC-125`).
fn object_parts(
    bytes: &[u8],
    doc_key: &DocumentKey,
) -> Result<(KeyId, PublicKey, Sealed), PipelineError> {
    check_layout(bytes)?;
    if bytes.len() < OBJECT_HEADER_LEN {
        return Err(PipelineError::ShortObject);
    }
    let key_id = object_key(bytes)?;
    check_key(key_id, doc_key)?;
    let (author, rest) = bytes
        .get(LAYOUT_LEN + KEY_ID_LEN..)
        .ok_or(PipelineError::ShortObject)?
        .split_at(AUTHOR_KEY_LEN);
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
    let author = PublicKey::from_slice(author).map_err(|_| PipelineError::ShortObject)?;
    Ok((key_id, author, sealed))
}

/// Opens what [`seal_object`] sealed, in the envelope's order: the signature,
/// then the commitment tag, then the cipher.
///
/// # Errors
///
/// * [`PipelineError::UnknownLayout`] when the first byte names a layout this
///   build does not read.
/// * [`PipelineError::ShortObject`] when the bytes cannot hold the header.
/// * [`PipelineError::WrongKey`] when the header names another key.
/// * [`PipelineError::Crypto`] when any of the three checks fails.
pub fn open_object(
    bytes: &[u8],
    aad: &[u8],
    doc_key: &DocumentKey,
) -> Result<Vec<u8>, PipelineError> {
    let (key_id, author, sealed) = object_parts(bytes, doc_key)?;
    Ok(envelope::open(
        &sealed,
        &object_aad(key_id, aad),
        doc_key,
        &author,
    )?)
}

// -- Key links -----------------------------------------------------------------

/// Domain of a key link. The document id and the identifier of the key it
/// seals follow it, so a link opens for its own document and key only.
const LINK_DOMAIN: &[u8] = b"ambre-key-link-v1";

/// The additional data of a link: the domain, the document id, then the
/// identifier of the key it seals.
fn link_aad(document_id: ambre_core::DocumentId, previous: KeyId) -> Vec<u8> {
    let mut aad = LINK_DOMAIN.to_vec();
    aad.extend_from_slice(document_id.as_bytes());
    aad.extend_from_slice(previous.as_array());
    aad
}

/// Seals `previous` under `next` and signs it: the link a rotation leaves
/// (`SYNC-116`). The sealed bytes take the layout of a standalone object, so
/// their header names `next` and the rotator.
///
/// # Errors
///
/// [`PipelineError::Crypto`] from the envelope.
pub fn seal_link(
    document_id: ambre_core::DocumentId,
    previous: &DocumentKey,
    next: &DocumentKey,
    signing_key: &SigningKey,
) -> Result<ambre_protocol::KeyLink, PipelineError> {
    let key = next.id();
    let aad = object_aad(key, &link_aad(document_id, previous.id()));
    let sealed = next.seal_previous(previous, &aad, signing_key)?;
    Ok(ambre_protocol::KeyLink {
        document_id,
        key,
        previous: previous.id(),
        sealed: object_bytes(key, signing_key, &sealed),
    })
}

/// Opens the key `link` seals, with `next`, the key it starts from. The key
/// that opens must hash to the identifier the link names (`SYNC-120`).
///
/// # Errors
///
/// * [`PipelineError::WrongDocument`] for another document's link.
/// * [`PipelineError::WrongKey`] when `next` is not the key the link starts
///   from, or when what opens is not the key it names.
/// * [`PipelineError::Crypto`] when the signature, the tag or the cipher
///   fails.
pub fn open_link(
    link: &ambre_protocol::KeyLink,
    document_id: ambre_core::DocumentId,
    next: &DocumentKey,
) -> Result<DocumentKey, PipelineError> {
    if link.document_id != document_id {
        return Err(PipelineError::WrongDocument);
    }
    check_key(link.key, next)?;
    let (key_id, author, sealed) = object_parts(&link.sealed, next)?;
    let aad = object_aad(key_id, &link_aad(document_id, link.previous));
    let previous = next.open_previous(&sealed, &aad, &author)?;
    if previous.id() == link.previous {
        Ok(previous)
    } else {
        Err(PipelineError::WrongKey)
    }
}

/// The key a sealed object names, read from its header without opening it.
///
/// # Errors
///
/// [`PipelineError::UnknownLayout`] when the layout is not one this build
/// reads, and [`PipelineError::ShortObject`] when the bytes cannot hold an
/// identifier.
pub fn object_key(bytes: &[u8]) -> Result<KeyId, PipelineError> {
    check_layout(bytes)?;
    bytes
        .get(LAYOUT_LEN..LAYOUT_LEN + KEY_ID_LEN)
        .and_then(|id| id.try_into().ok())
        .map(KeyId::from_array)
        .ok_or(PipelineError::ShortObject)
}

/// The key that signed a sealed object, read from its header without opening
/// it. An open checks the signature under this same key, so a filter on it
/// holds (`SYNC-121`).
///
/// # Errors
///
/// [`PipelineError::UnknownLayout`] when the layout is not one this build
/// reads, and [`PipelineError::ShortObject`] when the bytes cannot hold a key.
pub fn object_author(bytes: &[u8]) -> Result<PublicKey, PipelineError> {
    check_layout(bytes)?;
    let start = LAYOUT_LEN + KEY_ID_LEN;
    bytes
        .get(start..start + AUTHOR_KEY_LEN)
        .and_then(|author| PublicKey::from_slice(author).ok())
        .ok_or(PipelineError::ShortObject)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ambre_core::activity::ActivityKind;
    use ambre_core::{DocumentId, PublicKey, SnapshotId, UserId};
    use ambre_crypto::MasterSeed;
    use ambre_protocol::messages::{Clock, Counter};

    fn keys() -> (DocumentKey, SigningKey, PublicKey) {
        let doc_key = DocumentKey::from_array([0x11; 32]);
        let signing = MasterSeed::from_array([0x22; 32]).derive_signing_key();
        let author = PublicKey::from_array(signing.verifying_key().to_bytes());
        (doc_key, signing, author)
    }

    /// The identifier of the key [`keys`] returns.
    fn key_id() -> KeyId {
        DocumentKey::from_array([0x11; 32]).id()
    }

    fn update_pd(author: PublicKey) -> UpdatePublicData {
        UpdatePublicData {
            document_id: DocumentId::new(),
            pub_key: author,
            key_id: key_id(),
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
            key_id: key_id(),
            snapshot_id: SnapshotId::new(),
            parent_snapshot_proof: Vec::new(),
            parent_snapshot_update_clocks: Vec::new(),
        };
        let snapshot = seal_snapshot(&payload, pd, &doc_key, &signing).unwrap();
        // Compressed ciphertext is far smaller than the 4 KiB plaintext.
        assert!(snapshot.ciphertext.0.len() < payload.len());
        assert_eq!(open_snapshot(&snapshot, &doc_key).unwrap(), payload);
    }

    /// The seal runs the wire checks a relay never runs on a snapshot
    /// (`SYNC-022`). A parent clock list naming one author twice is refused
    /// with the check's own error, and nothing is returned to upload.
    #[test]
    fn a_snapshot_that_fails_a_wire_check_is_never_sealed() {
        use ambre_protocol::error::ValidationError;

        let (doc_key, signing, author) = keys();
        let pd = SnapshotPublicData {
            document_id: DocumentId::new(),
            pub_key: author.clone(),
            key_id: key_id(),
            snapshot_id: SnapshotId::new(),
            parent_snapshot_proof: Vec::new(),
            parent_snapshot_update_clocks: vec![(author.clone(), Clock(1)), (author, Clock(2))],
        };
        assert!(matches!(
            seal_snapshot(b"board", pd, &doc_key, &signing),
            Err(PipelineError::Validation(
                ValidationError::DuplicatePublicKey
            ))
        ));
    }

    /// The frame with its four sealed parts blanked: what the relay reads of
    /// it, the public metadata and the shape around it.
    fn readable_part(update: &Update) -> Vec<u8> {
        let mut blank = update.clone();
        blank.ciphertext = Ciphertext(Vec::new());
        blank.nonce = Nonce([0u8; 24]);
        blank.commitment_tag = CommitmentTag([0u8; 32]);
        blank.signature = Signature([0u8; 64]);
        postcard::to_allocvec(&blank).unwrap()
    }

    /// A compressed payload and a raw one leave the same readable bytes. The
    /// version that tells them apart opens as the first plaintext byte
    /// (`SYNC-020`). A version byte moved beside the ciphertext would tell the
    /// relay how compressible a document is.
    #[test]
    fn the_compression_version_is_sealed_inside_the_ciphertext() {
        let (doc_key, signing, author) = keys();
        let pd = update_pd(author);
        let raw = seal_update(b"short", pd.clone(), &doc_key, &signing).unwrap();
        let packed = seal_update(&[0x7Au8; 4096], pd, &doc_key, &signing).unwrap();
        assert_eq!(readable_part(&raw), readable_part(&packed));

        for (update, version) in [
            (&raw, CompressionVersion::Raw as u8),
            (&packed, CompressionVersion::Zstd as u8),
        ] {
            let aad = aad_of(&update.public_data).unwrap();
            let sealed = sealed_from(
                &update.ciphertext,
                &update.nonce,
                &update.commitment_tag,
                &update.signature,
            );
            let plaintext =
                envelope::open(&sealed, &aad, &doc_key, &update.public_data.pub_key).unwrap();
            assert_eq!(plaintext[0], version);
        }
    }

    /// The session nonce rides inside the ciphertext (`SYNC-112`). One delta
    /// sealed under two nonces shows the relay the same readable part. Each
    /// opens back to its own nonce and the delta.
    #[test]
    fn the_session_nonce_is_sealed_inside_the_ciphertext() {
        let (doc_key, signing, author) = keys();
        let pd = update_pd(author);
        let first = SessionNonce([1; SESSION_NONCE_LEN]);
        let second = SessionNonce([2; SESSION_NONCE_LEN]);
        let seal = |nonce: &SessionNonce| {
            seal_update(
                &update_body(nonce, None, b"delta").unwrap(),
                pd.clone(),
                &doc_key,
                &signing,
            )
            .unwrap()
        };
        let (a, b) = (seal(&first), seal(&second));
        assert_eq!(readable_part(&a), readable_part(&b));
        for (update, nonce) in [(&a, first), (&b, second)] {
            let body = split_update_body(open_update(update, &doc_key).unwrap()).unwrap();
            assert_eq!(
                body,
                UpdateBody {
                    nonce,
                    summary: None,
                    delta: b"delta".to_vec()
                }
            );
        }
    }

    /// The activity summary rides inside the ciphertext (`SYNC-113`). The
    /// relay reads the same part with or without one, and no mentioned
    /// member's bytes. The open hands the summary back beside the delta.
    #[test]
    fn the_activity_summary_is_sealed_inside_the_ciphertext() {
        let (doc_key, signing, author) = keys();
        let pd = update_pd(author);
        let nonce = SessionNonce([1; SESSION_NONCE_LEN]);
        let mentioned = UserId::from_bytes([0xA7; 16]);
        let summary = ActivitySummary {
            kind: ActivityKind::Content,
            mentions: vec![mentioned],
            mentions_all: false,
            anchor: None,
        };
        let seal = |summary: Option<&ActivitySummary>| {
            let body = update_body(&nonce, summary, b"delta").unwrap();
            seal_update(&body, pd.clone(), &doc_key, &signing).unwrap()
        };
        let (with, without) = (seal(Some(&summary)), seal(None));
        assert_eq!(readable_part(&with), readable_part(&without));
        let wire = serde_json::to_vec(&with).unwrap();
        assert!(
            !wire.windows(16).any(|w| w == mentioned.as_bytes()),
            "a mentioned member reaches the relay in the clear"
        );
        let body = split_update_body(open_update(&with, &doc_key).unwrap()).unwrap();
        assert_eq!(
            body,
            UpdateBody {
                nonce,
                summary: Some(summary),
                delta: b"delta".to_vec()
            }
        );
    }

    /// A body too short for the nonce and the marker is refused, and so is a
    /// marker no build writes, or a summary cut short.
    #[test]
    fn an_update_body_short_of_its_parts_is_refused() {
        assert!(matches!(
            split_update_body(vec![0; SESSION_NONCE_LEN]),
            Err(PipelineError::ShortBody)
        ));
        let mut body = update_body(&SessionNonce([1; SESSION_NONCE_LEN]), None, b"d").unwrap();
        if let Some(marker) = body.get_mut(SESSION_NONCE_LEN) {
            *marker = 2;
        }
        assert!(matches!(
            split_update_body(body),
            Err(PipelineError::UnknownSummary)
        ));
        let mut cut = vec![1; SESSION_NONCE_LEN];
        cut.extend_from_slice(&[WITH_SUMMARY, 0, 0]);
        assert!(matches!(
            split_update_body(cut),
            Err(PipelineError::Summary(SummaryError::Short))
        ));
        let empty =
            split_update_body(update_body(&SessionNonce([1; 16]), None, b"").unwrap()).unwrap();
        assert!(empty.delta.is_empty());
    }

    /// What the upload sends holds no byte run of the plaintext. It opens
    /// back to it, a snapshot blob and a media blob alike (`SYNC-049`,
    /// `ZK-001`).
    #[test]
    fn a_blob_bound_for_storage_is_sealed() {
        let (doc_key, signing, author) = keys();
        let document_id = DocumentId::new();
        let plaintext = b"a picture's bytes, or a document's whole state, in the clear";
        let contains = |hay: &[u8]| {
            hay.windows(16)
                .any(|w| plaintext.windows(16).any(|p| p == w))
        };

        let pd = SnapshotPublicData {
            document_id,
            pub_key: author,
            key_id: key_id(),
            snapshot_id: SnapshotId::new(),
            parent_snapshot_proof: Vec::new(),
            parent_snapshot_update_clocks: Vec::new(),
        };
        let snapshot = snapshot_blob(plaintext, pd, &doc_key, &signing).unwrap();
        assert!(!contains(snapshot.as_bytes()));
        let decoded = decode_snapshot_blob(snapshot.as_bytes(), document_id).unwrap();
        assert_eq!(open_snapshot(&decoded, &doc_key).unwrap(), plaintext);

        let aad = name_aad(document_id);
        let object = object_blob(plaintext, &aad, &doc_key, &signing).unwrap();
        assert!(!contains(object.as_bytes()));
        assert_eq!(object.len(), object.as_bytes().len());
        assert_eq!(
            open_object(object.as_bytes(), &aad, &doc_key).unwrap(),
            plaintext
        );
    }

    #[test]
    fn roundtrip_ephemeral() {
        let (doc_key, signing, author) = keys();
        let payload = b"cursor@42";
        let pd = EphemeralPublicData {
            document_id: DocumentId::new(),
            pub_key: author,
            key_id: key_id(),
            counter: Counter(7),
        };
        let eph = seal_ephemeral(payload, pd, &doc_key, &signing).unwrap();
        assert_eq!(open_ephemeral(&eph, &doc_key).unwrap(), payload);
    }

    /// Each wire message names the key that sealed it, and a seal refuses
    /// metadata that names another one (`SYNC-125`).
    #[test]
    fn a_frame_names_the_key_that_sealed_it() {
        let (doc_key, signing, author) = keys();
        let update = seal_update(b"delta", update_pd(author.clone()), &doc_key, &signing).unwrap();
        assert_eq!(update.public_data.key_id, doc_key.id());

        let other = DocumentKey::from_array([0x44; 32]);
        assert!(matches!(
            seal_update(b"delta", update_pd(author), &other, &signing),
            Err(PipelineError::WrongKey)
        ));
        assert!(matches!(
            open_update(&update, &other),
            Err(PipelineError::WrongKey)
        ));
    }

    /// A name or a blob names in its header the key that sealed it, and the
    /// header cannot be rewritten to name another (`SYNC-125`).
    #[test]
    fn an_object_names_the_key_that_sealed_it() {
        let (doc_key, signing, _) = keys();
        let sealed = seal_object(b"Budget 2027", b"aad", &doc_key, &signing).unwrap();
        assert_eq!(object_key(&sealed).unwrap(), doc_key.id());

        let other = DocumentKey::from_array([0x44; 32]);
        assert!(matches!(
            open_object(&sealed, b"aad", &other),
            Err(PipelineError::WrongKey)
        ));

        // A header rewritten to name another key no longer opens under it.
        let mut renamed = seal_object(b"Budget 2027", b"aad", &other, &signing).unwrap();
        renamed[LAYOUT_LEN..LAYOUT_LEN + KEY_ID_LEN].copy_from_slice(doc_key.id().as_array());
        assert!(open_object(&renamed, b"aad", &doc_key).is_err());
    }

    /// An object of a layout this build does not read is refused by that
    /// layout (`SYNC-138`). Its length, its key and its signature are never
    /// looked at.
    #[test]
    fn an_unknown_object_layout_is_refused_before_any_check() {
        let (doc_key, signing, _) = keys();
        let mut later = seal_object(b"a picture", b"aad", &doc_key, &signing).unwrap();
        assert_eq!(later[0], OBJECT_LAYOUT);
        later[0] = OBJECT_LAYOUT + 1;
        assert!(matches!(
            open_object(&later, b"aad", &doc_key),
            Err(PipelineError::UnknownLayout(2))
        ));
        assert!(matches!(
            object_key(&later),
            Err(PipelineError::UnknownLayout(2))
        ));
        // One byte alone: the layout is the first thing read.
        assert!(matches!(
            open_object(&[OBJECT_LAYOUT + 1], b"aad", &doc_key),
            Err(PipelineError::UnknownLayout(2))
        ));
        assert!(matches!(
            open_object(&[], b"aad", &doc_key),
            Err(PipelineError::ShortObject)
        ));
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
        let signature_at = LAYOUT_LEN + KEY_ID_LEN + AUTHOR_KEY_LEN;
        let mut forged = sealed.clone();
        forged[signature_at] ^= 1;
        assert!(matches!(
            open_object(&forged, aad, &doc_key),
            Err(PipelineError::Crypto(CryptoError::InvalidSignature))
        ));

        // The tag: a wrong one, re-signed so the signature passes, still fails.
        let tag_at = signature_at + signature::SIGNATURE_LEN;
        let nonce_at = tag_at + commitment::TAG_LEN;
        let mut moved = sealed.clone();
        moved[tag_at] ^= 1;
        let tag: [u8; commitment::TAG_LEN] = moved[tag_at..nonce_at].try_into().unwrap();
        let nonce = &moved[nonce_at..OBJECT_HEADER_LEN];
        let bound = object_aad(doc_key.id(), aad);
        let resigned = signature::sign(&signing, nonce, &moved[OBJECT_HEADER_LEN..], &bound, &tag);
        moved[signature_at..tag_at].copy_from_slice(&resigned);
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
