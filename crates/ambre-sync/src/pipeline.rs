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
use ambre_core::authorship::SessionNonce;
use ambre_core::{
    Aad, ByteSize, EphemeralBytes, KeyId, MediaBytes, PublicKey, SecretBytes, SnapshotBytes,
};
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

/// Maximum decompression ratio accepted on the inbound path, output bytes per
/// compressed byte. zstd routinely
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
const MIN_DECOMPRESS_CEILING: ByteSize = ByteSize(1 << 20); // 1 MiB

/// Hard upper bound on any single decompressed message, regardless of the
/// ratio-derived ceiling. Matches the client-memory pillar (< 100 MB per doc).
const ABSOLUTE_DECOMPRESS_CAP: ByteSize = ByteSize(64 << 20); // 64 MiB

/// Compression scheme of a sealed plaintext, encoded as the first plaintext byte.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
enum CompressionVersion {
    Raw = 0x00,
    Zstd = 0x01,
}

impl CompressionVersion {
    /// The byte that opens the plaintext.
    const fn byte(self) -> u8 {
        self as u8
    }

    /// The scheme `byte` names, refused when no build writes it.
    fn from_byte(byte: u8) -> Result<Self, PipelineError> {
        match byte {
            0x00 => Ok(Self::Raw),
            0x01 => Ok(Self::Zstd),
            other => Err(PipelineError::UnknownVersion(other)),
        }
    }
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

/// The marker byte of an update body: whether an activity summary follows it
/// (`SYNC-113`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
enum SummaryMarker {
    /// No activity summary: the delta follows the marker.
    Absent = 0x00,
    /// The activity summary follows the marker, then the delta.
    Present = 0x01,
}

impl SummaryMarker {
    /// The byte the body carries.
    const fn byte(self) -> u8 {
        self as u8
    }

    /// The marker `byte` names, refused when no build writes it.
    fn from_byte(byte: u8) -> Result<Self, PipelineError> {
        match byte {
            0x00 => Ok(Self::Absent),
            0x01 => Ok(Self::Present),
            _ => Err(PipelineError::UnknownSummary),
        }
    }
}

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
    pub delta: ambre_core::DeltaBytes,
}

/// The plaintext an update seals: the session nonce of its author, the summary
/// marker, the summary when there is one, then the delta (`SYNC-113`). A delta
/// alone is a [`ambre_core::DeltaBytes`] and never one of these, so a delta
/// cannot leave unframed. Only [`update_body`] frames one, and only
/// [`open_update`] yields one from the relay, for [`split_update_body`] to
/// check. It holds document plaintext, so `Debug` prints the length alone.
///
/// A frame is a transient on the way to the seal or out of the open: the
/// delta it carries lives on as a [`ambre_core::DeltaBytes`]. The Sync worker
/// runs no destructor (`ARCH-015`), so an owner that drops a frame wipes it
/// through [`UpdateFrame::wipe`].
#[derive(Clone, PartialEq, Eq)]
pub struct UpdateFrame(Vec<u8>);

impl UpdateFrame {
    /// The framed bytes, as the seal compresses them.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    /// The framed bytes, owned, for a fuzz target that reads them back.
    #[cfg(any(test, feature = "fuzz"))]
    #[must_use]
    pub fn into_bytes(self) -> Vec<u8> {
        self.0
    }

    /// Wipes the frame as its owner drops it, through `Secret::wipe`.
    pub(crate) fn wipe(self) {
        ambre_core::Secret::wipe(self.0);
    }

    /// How many bytes it holds.
    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Bytes taken as a frame without the framing, for a test that seals a
    /// malformed body.
    #[cfg(test)]
    pub(crate) fn from_bytes(bytes: Vec<u8>) -> Self {
        Self(bytes)
    }
}

impl std::fmt::Debug for UpdateFrame {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "UpdateFrame({} bytes)", self.0.len())
    }
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
    delta: &ambre_core::DeltaBytes,
) -> Result<UpdateFrame, PipelineError> {
    let summary = summary.map(ActivitySummary::encode).transpose()?;
    let summary_len = summary.as_ref().map_or(0, Vec::len);
    // A capacity hint over buffers already in memory: the sum stays below the
    // bound, and saturating states it.
    let mut body = Vec::with_capacity(
        SessionNonce::SIZE
            .as_len()
            .saturating_add(1)
            .saturating_add(summary_len)
            .saturating_add(delta.len()),
    );
    body.extend_from_slice(&nonce.0);
    match summary {
        Some(summary) => {
            body.push(SummaryMarker::Present.byte());
            body.extend_from_slice(&summary);
        }
        None => body.push(SummaryMarker::Absent.byte()),
    }
    body.extend_from_slice(delta.as_bytes());
    Ok(UpdateFrame(body))
}

/// Splits an opened update into its nonce, its summary and its delta, the last
/// step of the open. A body too short for its parts, or one whose marker no
/// build writes, is refused.
///
/// # Errors
///
/// [`PipelineError::ShortBody`], [`PipelineError::UnknownSummary`] and
/// [`PipelineError::Summary`].
pub fn split_update_body(body: UpdateFrame) -> Result<UpdateBody, PipelineError> {
    // The frame is read in place, then wiped on success and on each refusal.
    let split = split_frame(&body.0);
    body.wipe();
    split
}

fn split_frame(body: &[u8]) -> Result<UpdateBody, PipelineError> {
    let (nonce, rest) = SessionNonce::split_from(body).ok_or(PipelineError::ShortBody)?;
    let (&marker, rest) = rest.split_first().ok_or(PipelineError::ShortBody)?;
    let (summary, delta) = match SummaryMarker::from_byte(marker)? {
        SummaryMarker::Absent => (None, rest),
        SummaryMarker::Present => {
            let (summary, delta) = ActivitySummary::decode(rest)?;
            (Some(summary), delta)
        }
    };
    Ok(UpdateBody {
        nonce,
        summary,
        delta: ambre_core::DeltaBytes::new(delta.to_vec()),
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
/// scheme and the (possibly compressed) body.
fn compress(data: &[u8]) -> (CompressionVersion, Vec<u8>) {
    if data.len() > COMPRESS_THRESHOLD {
        (
            CompressionVersion::Zstd,
            compress_to_vec(data, CompressionLevel::Fastest),
        )
    } else {
        (CompressionVersion::Raw, data.to_vec())
    }
}

/// Reverses [`compress`] given the scheme and body.
fn decompress(version: CompressionVersion, body: &[u8]) -> Result<Vec<u8>, PipelineError> {
    match version {
        CompressionVersion::Raw => Ok(body.to_vec()),
        CompressionVersion::Zstd => decompress_zstd_bounded(body),
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
    let ceiling = ByteSize::of_len(body.len().saturating_mul(MAX_DECOMPRESS_RATIO))
        .clamp(MIN_DECOMPRESS_CEILING, ABSOLUTE_DECOMPRESS_CAP);

    let mut decoder =
        ruzstd::decoding::StreamingDecoder::new(body).map_err(|_| PipelineError::Decompress)?;

    // Never pre-allocate the whole ceiling. Grow the buffer as we go, reading
    // one byte past the ceiling to distinguish "exactly at the limit" from
    // "over the limit". `take` caps the reader at `ceiling + 1` bytes.
    //
    // A refused body leaves a partial plaintext in `out`, and `Secret::wipe`
    // wipes it. That wipe reaches `out` alone. The decoder's own block and
    // window buffers, and each buffer `read_to_end` outgrows, drop unwiped,
    // out of the wrapper's reach.
    let mut out = Vec::new();
    let refusal = match decoder
        .by_ref()
        .take(ceiling.0.saturating_add(1))
        .read_to_end(&mut out)
    {
        Err(_) => PipelineError::Decompress,
        Ok(read) if ByteSize::of_len(read) > ceiling => PipelineError::DecompressTooLarge,
        Ok(_) => return Ok(out),
    };
    ambre_core::Secret::wipe(out);
    Err(refusal)
}

// -- Generic seal / open over the (version || body) plaintext ------------------

fn seal_payload(
    payload: &[u8],
    aad: &Aad,
    doc_key: &DocumentKey,
    signing_key: &SigningKey,
) -> Result<Sealed, PipelineError> {
    // The body is a plaintext too, compressed or not. It is wiped once it is
    // copied into the sealed plaintext, before the seal can fail. The sealed
    // plaintext drops under its wrapper on every path. The zstd encoder's own
    // buffers drop unwiped, out of reach of both.
    let (version, body) = compress(payload);
    let mut plaintext = Vec::with_capacity(body.len().saturating_add(1));
    plaintext.push(version.byte());
    plaintext.extend_from_slice(&body);
    ambre_core::Secret::wipe(body);
    let plaintext = ambre_core::SecretBytes::new(plaintext);
    Ok(envelope::seal(&plaintext, aad, doc_key, signing_key)?)
}

fn open_payload(
    sealed: &Sealed,
    aad: &Aad,
    doc_key: &DocumentKey,
    author_pub: &PublicKey,
) -> Result<OpenedPayload, PipelineError> {
    // The opened bytes are lent, with no copy, through the door `ARCH-047`
    // admits. They are still compressed, and the wrapper wipes them as it
    // drops, on the error paths too. The zstd decoder copies them on the
    // way, into its block and window buffers and into each buffer
    // `read_to_end` outgrows. Those copies are out of the wrapper's reach.
    let opened = envelope::open(sealed, aad, doc_key, author_pub)?;
    opened.with_plaintext(|plaintext| {
        let (version, body) = plaintext.split_first().ok_or(PipelineError::Decompress)?;
        decompress(CompressionVersion::from_byte(*version)?, body).map(OpenedPayload)
    })
}

/// A payload opened and decompressed, before its caller names its role. Each
/// open moves it into the one role it reads, with no copy on the way. The
/// role keeps the bytes in the clear for the document model. A wipe here
/// would find an emptied buffer, so it is left out (`ARCH-047`).
struct OpenedPayload(Vec<u8>);

impl OpenedPayload {
    /// The payload as the frame of an update, for [`split_update_body`].
    fn into_frame(self) -> UpdateFrame {
        UpdateFrame(self.0)
    }

    /// The payload as a full CRDT state.
    fn into_snapshot(self) -> SnapshotBytes {
        SnapshotBytes::new(self.0)
    }

    /// The payload as one presence sample.
    fn into_ephemeral(self) -> EphemeralBytes {
        EphemeralBytes::new(self.0)
    }
}

fn aad_of<T: serde::Serialize>(public_data: &T) -> Result<Aad, PipelineError> {
    postcard::to_allocvec(public_data)
        .map(Aad::new)
        .map_err(|_| PipelineError::Postcard)
}

/// Rebuilds a [`Sealed`] from a wire message's envelope fields.
fn sealed_from(
    ciphertext: &Ciphertext,
    nonce: &Nonce,
    commitment_tag: &CommitmentTag,
    signature: &Signature,
) -> Sealed {
    Sealed {
        nonce: *nonce,
        ciphertext: ciphertext.clone(),
        commitment_tag: *commitment_tag,
        signature: *signature,
    }
}

fn message_parts(sealed: Sealed) -> (Ciphertext, Nonce, CommitmentTag, Signature) {
    (
        sealed.ciphertext,
        sealed.nonce,
        sealed.commitment_tag,
        sealed.signature,
    )
}

// -- Update --------------------------------------------------------------------

/// Compresses, encrypts and signs `payload` into an [`Update`].
pub fn seal_update(
    payload: &UpdateFrame,
    public_data: UpdatePublicData,
    doc_key: &DocumentKey,
    signing_key: &SigningKey,
) -> Result<Update, PipelineError> {
    check_key(public_data.key_id, doc_key)?;
    let aad = aad_of(&public_data)?;
    let sealed = seal_payload(payload.as_bytes(), &aad, doc_key, signing_key)?;
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
pub fn open_update(update: &Update, doc_key: &DocumentKey) -> Result<UpdateFrame, PipelineError> {
    check_key(update.public_data.key_id, doc_key)?;
    let aad = aad_of(&update.public_data)?;
    let sealed = sealed_from(
        &update.ciphertext,
        &update.nonce,
        &update.commitment_tag,
        &update.signature,
    );
    let frame = open_payload(&sealed, &aad, doc_key, &update.public_data.pub_key)?.into_frame();
    tracing::debug!(
        "[Pipeline] open_update: {} bytes ciphertext -> {} bytes plaintext",
        update.ciphertext.0.len(),
        frame.len(),
    );
    Ok(frame)
}

// -- Snapshot ------------------------------------------------------------------

/// Compresses, encrypts and signs `payload` into a [`Snapshot`].
pub fn seal_snapshot(
    payload: &SnapshotBytes,
    public_data: SnapshotPublicData,
    doc_key: &DocumentKey,
    signing_key: &SigningKey,
) -> Result<Snapshot, PipelineError> {
    check_key(public_data.key_id, doc_key)?;
    let aad = aad_of(&public_data)?;
    let sealed = seal_payload(payload.as_bytes(), &aad, doc_key, signing_key)?;
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
    blob: &FetchedBlob,
    document_id: ambre_bus::DocumentId,
) -> Result<Snapshot, PipelineError> {
    let snapshot: Snapshot =
        postcard::from_bytes(blob.as_bytes()).map_err(|_| PipelineError::Postcard)?;
    if snapshot.public_data.document_id != document_id {
        return Err(PipelineError::WrongDocument);
    }
    Ok(snapshot)
}

/// Verifies, decrypts and decompresses a [`Snapshot`] back to its CRDT payload.
pub fn open_snapshot(
    snapshot: &Snapshot,
    doc_key: &DocumentKey,
) -> Result<SnapshotBytes, PipelineError> {
    check_key(snapshot.public_data.key_id, doc_key)?;
    let aad = aad_of(&snapshot.public_data)?;
    let sealed = sealed_from(
        &snapshot.ciphertext,
        &snapshot.nonce,
        &snapshot.commitment_tag,
        &snapshot.signature,
    );
    open_payload(&sealed, &aad, doc_key, &snapshot.public_data.pub_key)
        .map(OpenedPayload::into_snapshot)
}

// -- Ephemeral -----------------------------------------------------------------

/// Compresses, encrypts and signs `payload` into an [`Ephemeral`].
pub fn seal_ephemeral(
    payload: &EphemeralBytes,
    public_data: EphemeralPublicData,
    doc_key: &DocumentKey,
    signing_key: &SigningKey,
) -> Result<Ephemeral, PipelineError> {
    check_key(public_data.key_id, doc_key)?;
    let aad = aad_of(&public_data)?;
    let sealed = seal_payload(payload.as_bytes(), &aad, doc_key, signing_key)?;
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
) -> Result<EphemeralBytes, PipelineError> {
    check_key(ephemeral.public_data.key_id, doc_key)?;
    let aad = aad_of(&ephemeral.public_data)?;
    let sealed = sealed_from(
        &ephemeral.ciphertext,
        &ephemeral.nonce,
        &ephemeral.commitment_tag,
        &ephemeral.signature,
    );
    open_payload(&sealed, &aad, doc_key, &ephemeral.public_data.pub_key)
        .map(OpenedPayload::into_ephemeral)
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

/// A blob as object storage returned it: sealed by some writer, and neither
/// verified nor opened yet. Distinct from [`SealedBlob`], which only the
/// local seal builds, so a download never stands in for an upload.
/// [`decode_snapshot_blob`] reads a snapshot from it, and [`open_object`] a
/// media blob. `Debug` prints the length and none of the bytes.
#[derive(Clone, PartialEq, Eq)]
pub struct FetchedBlob(Vec<u8>);

impl FetchedBlob {
    /// The bytes as the download returned them.
    pub fn new(bytes: Vec<u8>) -> Self {
        Self(bytes)
    }

    /// The bytes, for the decoder.
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

impl std::fmt::Debug for FetchedBlob {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "FetchedBlob({} bytes)", self.0.len())
    }
}

/// Bytes bound for object storage, built by the seal and by nothing else. The
/// presigned upload takes this type, so a snapshot or a media blob leaves the
/// worker only sealed under the document key (`SYNC-049`, `ZK-001`).
/// `Debug` prints the length and none of the bytes.
pub struct SealedBlob(Vec<u8>);

impl SealedBlob {
    /// The sealed bytes, for a test that reads them back.
    #[cfg(test)]
    pub(crate) fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    /// The body of the upload that sends it.
    pub(crate) fn to_body(&self) -> crate::http::HttpBody {
        crate::http::HttpBody::new(self.0.clone())
    }

    /// The sealed bytes, moved out, for the role that stores them: a
    /// document name.
    pub(crate) fn into_bytes(self) -> Vec<u8> {
        self.0
    }

    /// The sealed bytes, moved out, as the body of a key link.
    pub(crate) fn into_previous_key(self) -> ambre_core::SealedPreviousKey {
        ambre_core::SealedPreviousKey::new(self.0)
    }

    /// The size the relay is told, which is the sealed size.
    pub(crate) fn len(&self) -> usize {
        self.0.len()
    }
}

impl std::fmt::Debug for SealedBlob {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "SealedBlob({} bytes)", self.0.len())
    }
}

/// The bytes of a standalone object, in each role that holds one: a blob
/// sealed here or downloaded, a sealed document name, a key link. The header
/// readers take only these, so no other bytes are parsed as an object
/// (`SYNC-138`).
pub trait StoredObject: stored_object::Sealed {
    /// The object as it was laid out, header first.
    fn object(&self) -> &[u8];
}

mod stored_object {
    pub trait Sealed {}
    impl Sealed for super::SealedBlob {}
    impl Sealed for super::FetchedBlob {}
    impl Sealed for ambre_core::SealedName {}
    impl Sealed for ambre_core::SealedPreviousKey {}
}

impl StoredObject for SealedBlob {
    fn object(&self) -> &[u8] {
        &self.0
    }
}

impl StoredObject for FetchedBlob {
    fn object(&self) -> &[u8] {
        &self.0
    }
}

impl StoredObject for ambre_core::SealedName {
    fn object(&self) -> &[u8] {
        self.as_bytes()
    }
}

impl StoredObject for ambre_core::SealedPreviousKey {
    fn object(&self) -> &[u8] {
        self.as_bytes()
    }
}

/// Seals `payload` into an update frame, through [`seal_update`].
///
/// # Errors
///
/// Every error [`seal_update`] raises.
pub fn update_frame(
    payload: &UpdateFrame,
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
    payload: &EphemeralBytes,
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
    payload: &SnapshotBytes,
    public_data: SnapshotPublicData,
    doc_key: &DocumentKey,
    signing_key: &SigningKey,
) -> Result<SealedBlob, PipelineError> {
    let snapshot = seal_snapshot(payload, public_data, doc_key, signing_key)?;
    postcard::to_allocvec(&snapshot)
        .map(SealedBlob)
        .map_err(|_| PipelineError::Postcard)
}

/// Seals a media blob into a standalone object bound for object storage, in
/// the layout and under the AAD of [`seal_object`]. The cipher reads the
/// blob where it lies, with no copy into a `SecretBytes` first.
///
/// # Errors
///
/// Every error [`seal_object`] raises.
pub fn object_blob(
    plaintext: &MediaBytes,
    aad: &Aad,
    doc_key: &DocumentKey,
    signing_key: &SigningKey,
) -> Result<SealedBlob, PipelineError> {
    let key_id = doc_key.id();
    let sealed = envelope::seal_media(plaintext, &object_aad(key_id, aad), doc_key, signing_key)?;
    Ok(object_bytes(key_id, signing_key, &sealed))
}

// -- Standalone objects: a document name, a media blob -------------------------

ambre_core::define_layout_version! {
    /// The layout of a standalone object, its first byte (`SYNC-138`). The
    /// relay keeps a blob across releases, so a reader must tell an older
    /// layout from a forged one. Every object of a release shares it, so it
    /// says nothing of the content.
    enum ObjectLayout {
        /// The key id, the author key, the signature, the commitment tag and
        /// the nonce, then the ciphertext.
        V1 = 1,
    }
    current = V1;
}

/// Bytes before the ciphertext of a standalone object, each part at the size
/// its own type states.
const OBJECT_HEADER: ByteSize = ObjectLayout::SIZE
    .saturating_add(KeyId::SIZE)
    .saturating_add(PublicKey::SIZE)
    .saturating_add(Signature::SIZE)
    .saturating_add(CommitmentTag::SIZE)
    .saturating_add(Nonce::SIZE);

/// Domain of a sealed document name. The document id follows it in the
/// additional data, so a name opens under its own document only (`SYNC-068`).
const NAME_DOMAIN: &[u8] = b"ambre-doc-name-v2";

/// The additional data of a document name: the domain, then the document id.
pub fn name_aad(document_id: ambre_core::DocumentId) -> Aad {
    let mut aad = NAME_DOMAIN.to_vec();
    aad.extend_from_slice(document_id.as_bytes());
    Aad::new(aad)
}

/// Domain of a sealed media blob. The document id and the address follow it,
/// so a blob opens under its own document and address only (`SYNC-051`,
/// `SYNC-074`).
const MEDIA_DOMAIN: &[u8] = b"ambre-media-v2";

/// The additional data of a media blob: the domain, the document id, then the
/// address the document stores.
pub fn media_aad(document_id: ambre_core::DocumentId, address: &ambre_core::MediaAddress) -> Aad {
    let mut aad = MEDIA_DOMAIN.to_vec();
    aad.extend_from_slice(document_id.as_bytes());
    aad.extend_from_slice(address.as_bytes());
    Aad::new(aad)
}

/// The additional data an object is sealed under: the layout and the key
/// identifier its header names, then the caller's own. The signature covers
/// all three, so a header cannot be made to name another layout or key.
fn object_aad(key_id: KeyId, aad: &Aad) -> Aad {
    let mut bound = vec![ObjectLayout::CURRENT.byte()];
    bound.extend_from_slice(key_id.as_array());
    bound.extend_from_slice(aad.as_bytes());
    Aad::new(bound)
}

/// Refuses an object whose first byte names a layout this build does not read,
/// before anything else in it is trusted (`SYNC-138`).
fn check_layout(bytes: &[u8]) -> Result<(), PipelineError> {
    let byte = bytes.first().copied().ok_or(PipelineError::ShortObject)?;
    match ObjectLayout::from_byte(byte) {
        Some(ObjectLayout::V1) => Ok(()),
        None => Err(PipelineError::UnknownLayout(byte)),
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
    plaintext: &SecretBytes,
    aad: &Aad,
    doc_key: &DocumentKey,
    signing_key: &SigningKey,
) -> Result<SealedBlob, PipelineError> {
    let key_id = doc_key.id();
    let sealed = envelope::seal(plaintext, &object_aad(key_id, aad), doc_key, signing_key)?;
    Ok(object_bytes(key_id, signing_key, &sealed))
}

/// Lays out a sealed object: `layout || key id || author key || signature ||
/// commitment tag || nonce || ciphertext`.
fn object_bytes(key_id: KeyId, signing_key: &SigningKey, sealed: &Sealed) -> SealedBlob {
    let mut out = Vec::with_capacity(
        OBJECT_HEADER
            .saturating_add(ByteSize::of_len(sealed.ciphertext.len()))
            .as_len(),
    );
    out.push(ObjectLayout::CURRENT.byte());
    out.extend_from_slice(key_id.as_array());
    out.extend_from_slice(signing_key.verifying_key().as_bytes());
    out.extend_from_slice(&sealed.signature.0);
    out.extend_from_slice(&sealed.commitment_tag.0);
    out.extend_from_slice(&sealed.nonce.0);
    out.extend_from_slice(&sealed.ciphertext.0);
    SealedBlob(out)
}

/// Splits what [`object_bytes`] laid out, refusing a key other than
/// `doc_key` before any cryptography (`SYNC-125`).
fn object_parts(
    object: &impl StoredObject,
    doc_key: &DocumentKey,
) -> Result<(KeyId, PublicKey, Sealed), PipelineError> {
    let bytes = object.object();
    check_layout(bytes)?;
    let (_, rest) = bytes.split_first().ok_or(PipelineError::ShortObject)?;
    let (key_id, rest) = KeyId::split_from(rest).ok_or(PipelineError::ShortObject)?;
    let (author, rest) = PublicKey::split_from(rest).ok_or(PipelineError::ShortObject)?;
    let (signature, rest) = Signature::split_from(rest).ok_or(PipelineError::ShortObject)?;
    let (commitment_tag, rest) =
        CommitmentTag::split_from(rest).ok_or(PipelineError::ShortObject)?;
    let (nonce, ciphertext) = Nonce::split_from(rest).ok_or(PipelineError::ShortObject)?;
    check_key(key_id, doc_key)?;
    let sealed = Sealed {
        nonce,
        ciphertext: Ciphertext(ciphertext.to_vec()),
        commitment_tag,
        signature,
    };
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
    object: &impl StoredObject,
    aad: &Aad,
    doc_key: &DocumentKey,
) -> Result<SecretBytes, PipelineError> {
    let (key_id, author, sealed) = object_parts(object, doc_key)?;
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
fn link_aad(document_id: ambre_core::DocumentId, previous: KeyId) -> Aad {
    let mut aad = LINK_DOMAIN.to_vec();
    aad.extend_from_slice(document_id.as_bytes());
    aad.extend_from_slice(previous.as_array());
    Aad::new(aad)
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
        sealed: object_bytes(key, signing_key, &sealed).into_previous_key(),
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
pub fn object_key(object: &impl StoredObject) -> Result<KeyId, PipelineError> {
    let bytes = object.object();
    check_layout(bytes)?;
    bytes
        .split_first()
        .and_then(|(_, rest)| KeyId::split_from(rest))
        .map(|(key_id, _)| key_id)
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
pub fn object_author(object: &impl StoredObject) -> Result<PublicKey, PipelineError> {
    let bytes = object.object();
    check_layout(bytes)?;
    bytes
        .split_first()
        .and_then(|(_, rest)| KeyId::split_from(rest))
        .and_then(|(_, rest)| PublicKey::split_from(rest))
        .map(|(author, _)| author)
        .ok_or(PipelineError::ShortObject)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ambre_core::activity::ActivityKind;
    use ambre_core::authorship::SESSION_NONCE_LEN;
    use ambre_core::{DocumentId, PublicKey, SnapshotId, UserId};
    use ambre_crypto::MasterSeed;
    use ambre_protocol::messages::{Clock, Counter};

    /// Plaintext as the object seal takes it.
    fn secret(bytes: &[u8]) -> SecretBytes {
        SecretBytes::new(bytes.to_vec())
    }

    /// Object bytes as a download returns them.
    fn fetched(bytes: &[u8]) -> FetchedBlob {
        FetchedBlob::new(bytes.to_vec())
    }

    /// Bytes sealed as a frame as they stand, framed or not.
    fn frame(bytes: &[u8]) -> UpdateFrame {
        UpdateFrame::from_bytes(bytes.to_vec())
    }

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
        let update = seal_update(&frame(payload), update_pd(author), &doc_key, &signing).unwrap();
        assert_eq!(open_update(&update, &doc_key).unwrap().as_bytes(), payload);
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
            parent_snapshot_proof: None,
            parent_snapshot_update_clocks: Vec::new(),
        };
        let snapshot = seal_snapshot(
            &ambre_core::SnapshotBytes::new(payload.to_vec()),
            pd,
            &doc_key,
            &signing,
        )
        .unwrap();
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
            parent_snapshot_proof: None,
            parent_snapshot_update_clocks: vec![(author.clone(), Clock(1)), (author, Clock(2))],
        };
        assert!(matches!(
            seal_snapshot(
                &ambre_core::SnapshotBytes::new(b"board".to_vec()),
                pd,
                &doc_key,
                &signing
            ),
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
        let raw = seal_update(&frame(b"short"), pd.clone(), &doc_key, &signing).unwrap();
        let packed = seal_update(&frame(&[0x7Au8; 4096]), pd, &doc_key, &signing).unwrap();
        assert_eq!(readable_part(&raw), readable_part(&packed));

        for (update, version) in [
            (&raw, CompressionVersion::Raw.byte()),
            (&packed, CompressionVersion::Zstd.byte()),
        ] {
            let aad = aad_of(&update.public_data).unwrap();
            let sealed = sealed_from(
                &update.ciphertext,
                &update.nonce,
                &update.commitment_tag,
                &update.signature,
            );
            let plaintext = envelope::open(&sealed, &aad, &doc_key, &update.public_data.pub_key)
                .unwrap()
                .into_plaintext();
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
                &update_body(nonce, None, &b"delta".to_vec().into()).unwrap(),
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
                    delta: ambre_core::DeltaBytes::new(b"delta".to_vec())
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
            mentions_all: ambre_bus::MentionScope::Named,
            anchor: None,
        };
        let seal = |summary: Option<&ActivitySummary>| {
            let body = update_body(&nonce, summary, &b"delta".to_vec().into()).unwrap();
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
                delta: ambre_core::DeltaBytes::new(b"delta".to_vec())
            }
        );
    }

    /// A body too short for the nonce and the marker is refused, and so is a
    /// marker no build writes, or a summary cut short.
    #[test]
    fn an_update_body_short_of_its_parts_is_refused() {
        assert!(matches!(
            split_update_body(frame(&[0; SESSION_NONCE_LEN])),
            Err(PipelineError::ShortBody)
        ));
        let mut body = update_body(
            &SessionNonce([1; SESSION_NONCE_LEN]),
            None,
            &b"d".to_vec().into(),
        )
        .unwrap()
        .into_bytes();
        if let Some(marker) = body.get_mut(SessionNonce::SIZE.as_len()) {
            *marker = 2;
        }
        assert!(matches!(
            split_update_body(UpdateFrame::from_bytes(body)),
            Err(PipelineError::UnknownSummary)
        ));
        let mut cut = vec![1; SessionNonce::SIZE.as_len()];
        cut.extend_from_slice(&[SummaryMarker::Present.byte(), 0, 0]);
        assert!(matches!(
            split_update_body(UpdateFrame::from_bytes(cut)),
            Err(PipelineError::Summary(SummaryError::Short))
        ));
        let empty = split_update_body(
            update_body(&SessionNonce([1; 16]), None, &b"".to_vec().into()).unwrap(),
        )
        .unwrap();
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
            parent_snapshot_proof: None,
            parent_snapshot_update_clocks: Vec::new(),
        };
        let snapshot = snapshot_blob(
            &SnapshotBytes::new(plaintext.to_vec()),
            pd,
            &doc_key,
            &signing,
        )
        .unwrap();
        assert!(!contains(snapshot.as_bytes()));
        let decoded =
            decode_snapshot_blob(&FetchedBlob::new(snapshot.as_bytes().to_vec()), document_id)
                .unwrap();
        assert_eq!(open_snapshot(&decoded, &doc_key).unwrap(), plaintext);

        let aad = name_aad(document_id);
        let object = object_blob(
            &MediaBytes::new(plaintext.to_vec()),
            &aad,
            &doc_key,
            &signing,
        )
        .unwrap();
        assert!(!contains(object.as_bytes()));
        assert_eq!(object.len(), object.as_bytes().len());
        assert_eq!(
            open_object(&object, &aad, &doc_key)
                .unwrap()
                .into_plaintext(),
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
        let eph = seal_ephemeral(
            &EphemeralBytes::new(payload.to_vec()),
            pd,
            &doc_key,
            &signing,
        )
        .unwrap();
        assert_eq!(open_ephemeral(&eph, &doc_key).unwrap(), payload);
    }

    /// Each wire message names the key that sealed it, and a seal refuses
    /// metadata that names another one (`SYNC-125`).
    #[test]
    fn a_frame_names_the_key_that_sealed_it() {
        let (doc_key, signing, author) = keys();
        let update = seal_update(
            &frame(b"delta"),
            update_pd(author.clone()),
            &doc_key,
            &signing,
        )
        .unwrap();
        assert_eq!(update.public_data.key_id, doc_key.id());

        let other = DocumentKey::from_array([0x44; 32]);
        assert!(matches!(
            seal_update(&frame(b"delta"), update_pd(author), &other, &signing),
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
        let sealed = seal_object(
            &secret(b"Budget 2027"),
            &ambre_core::Aad::of(b"aad"),
            &doc_key,
            &signing,
        )
        .unwrap();
        assert_eq!(object_key(&sealed).unwrap(), doc_key.id());

        let other = DocumentKey::from_array([0x44; 32]);
        assert!(matches!(
            open_object(&sealed, &ambre_core::Aad::of(b"aad"), &other),
            Err(PipelineError::WrongKey)
        ));

        // A header rewritten to name another key no longer opens under it.
        let mut renamed = seal_object(
            &secret(b"Budget 2027"),
            &ambre_core::Aad::of(b"aad"),
            &other,
            &signing,
        )
        .unwrap()
        .into_bytes();
        let key_at = ObjectLayout::SIZE.as_len();
        renamed[key_at..key_at + KeyId::SIZE.as_len()].copy_from_slice(doc_key.id().as_array());
        assert!(open_object(&fetched(&renamed), &ambre_core::Aad::of(b"aad"), &doc_key).is_err());
    }

    /// An object of a layout this build does not read is refused by that
    /// layout (`SYNC-138`). Its length, its key and its signature are never
    /// looked at.
    #[test]
    fn an_unknown_object_layout_is_refused_before_any_check() {
        let (doc_key, signing, _) = keys();
        let mut later = seal_object(
            &secret(b"a picture"),
            &ambre_core::Aad::of(b"aad"),
            &doc_key,
            &signing,
        )
        .unwrap()
        .into_bytes();
        assert_eq!(later[0], ObjectLayout::V1.byte());
        later[0] = ObjectLayout::V1.byte() + 1;
        let later = fetched(&later);
        assert!(matches!(
            open_object(&later, &ambre_core::Aad::of(b"aad"), &doc_key),
            Err(PipelineError::UnknownLayout(2))
        ));
        assert!(matches!(
            object_key(&later),
            Err(PipelineError::UnknownLayout(2))
        ));
        // One byte alone: the layout is the first thing read.
        assert!(matches!(
            open_object(
                &fetched(&[ObjectLayout::V1.byte() + 1]),
                &ambre_core::Aad::of(b"aad"),
                &doc_key
            ),
            Err(PipelineError::UnknownLayout(2))
        ));
        assert!(matches!(
            open_object(&fetched(&[]), &ambre_core::Aad::of(b"aad"), &doc_key),
            Err(PipelineError::ShortObject)
        ));
    }

    #[test]
    fn compression_threshold() {
        let small = vec![0u8; COMPRESS_THRESHOLD];
        let (v_small, _) = compress(&small);
        assert_eq!(v_small, CompressionVersion::Raw);

        let large = vec![0u8; COMPRESS_THRESHOLD + 1];
        let (v_large, body) = compress(&large);
        assert_eq!(v_large, CompressionVersion::Zstd);
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
            decompress(CompressionVersion::Zstd, &compressed),
            Err(PipelineError::DecompressTooLarge)
        ));
    }

    #[test]
    fn decompress_within_ceiling_ok() {
        // A legitimate ~4 KiB payload round-trips unaffected by the bound.
        let payload = vec![0x5Au8; 4096];
        let compressed = compress_to_vec(payload.as_slice(), CompressionLevel::Fastest);
        assert_eq!(
            decompress(CompressionVersion::Zstd, &compressed).unwrap(),
            payload
        );
    }

    #[test]
    fn decompress_unknown_version_rejected() {
        assert!(matches!(
            CompressionVersion::from_byte(0xFF),
            Err(PipelineError::UnknownVersion(0xFF))
        ));
    }

    #[test]
    fn wrong_document_key_rejected() {
        let (doc_key, signing, author) = keys();
        let update = seal_update(&frame(b"secret"), update_pd(author), &doc_key, &signing).unwrap();
        let other = DocumentKey::from_array([0x44; 32]);
        assert!(open_update(&update, &other).is_err());
    }

    #[test]
    fn tampered_document_id_rejected() {
        let (doc_key, signing, author) = keys();
        let mut update =
            seal_update(&frame(b"secret"), update_pd(author), &doc_key, &signing).unwrap();
        // Forge the routing metadata: signature still passes (attacker re-signs is
        // not modeled here), but the aad-bound commitment/AEAD no longer match.
        update.public_data.document_id = DocumentId::new();
        assert!(open_update(&update, &doc_key).is_err());
    }

    /// A standalone object carries the author signature and the commitment
    /// tag, and each is checked on the open (`SYNC-018`).
    #[test]
    fn a_sealed_object_carries_the_signature_and_the_tag() {
        use ambre_crypto::signature;

        let (doc_key, signing, _) = keys();
        let aad = &ambre_core::Aad::of(b"ambre-media");
        let sealed = seal_object(&secret(b"a picture"), aad, &doc_key, &signing).unwrap();
        assert_eq!(
            open_object(&sealed, aad, &doc_key)
                .unwrap()
                .into_plaintext(),
            b"a picture"
        );
        let sealed = sealed.into_bytes();

        // The signature: one flipped byte, and the author no longer signed it.
        let signature_at = ObjectLayout::SIZE
            .saturating_add(KeyId::SIZE)
            .saturating_add(PublicKey::SIZE)
            .as_len();
        let mut forged = sealed.clone();
        forged[signature_at] ^= 1;
        assert!(matches!(
            open_object(&fetched(&forged), aad, &doc_key),
            Err(PipelineError::Crypto(CryptoError::InvalidSignature))
        ));

        // The tag: a wrong one, re-signed so the signature passes, still fails.
        let tag_at = signature_at + Signature::SIZE.as_len();
        let nonce_at = tag_at + CommitmentTag::SIZE.as_len();
        let header_len = OBJECT_HEADER.as_len();
        let mut moved = sealed.clone();
        moved[tag_at] ^= 1;
        let tag = ambre_core::CommitmentTag(moved[tag_at..nonce_at].try_into().unwrap());
        let nonce = ambre_core::Nonce(moved[nonce_at..header_len].try_into().unwrap());
        let body = ambre_core::Ciphertext(moved[header_len..].to_vec());
        let bound = object_aad(doc_key.id(), aad);
        let resigned = signature::sign(&signing, &nonce, &body, &bound, &tag);
        moved[signature_at..tag_at].copy_from_slice(&resigned.0);
        assert!(matches!(
            open_object(&fetched(&moved), aad, &doc_key),
            Err(PipelineError::Crypto(CryptoError::CommitmentMismatch))
        ));

        // Too short to hold the header.
        assert!(matches!(
            open_object(&fetched(&sealed[..header_len - 1]), aad, &doc_key),
            Err(PipelineError::ShortObject)
        ));
    }

    /// A name sealed for one document does not open under another, even one
    /// that holds the same key (`SYNC-068`).
    #[test]
    fn a_name_opens_under_its_own_document_only() {
        let (doc_key, signing, _) = keys();
        let own = DocumentId::new();
        let sealed =
            seal_object(&secret(b"Budget 2027"), &name_aad(own), &doc_key, &signing).unwrap();
        assert_eq!(
            open_object(&sealed, &name_aad(own), &doc_key)
                .unwrap()
                .into_plaintext(),
            b"Budget 2027"
        );
        assert!(open_object(&sealed, &name_aad(DocumentId::new()), &doc_key).is_err());
    }

    /// A name the build before the typed layout sealed: `layout || key id ||
    /// author key || signature || commitment tag || nonce || ciphertext`. The
    /// relay keeps such objects across releases, so each later build reads its
    /// header and opens it.
    #[test]
    fn an_object_the_old_code_sealed_still_reads_and_opens() {
        let (doc_key, _, author) = keys();
        let id = DocumentId::from_bytes([7; 16]);
        let sealed = hex::decode(
            "01b6e14f706ce523ebafb9bf6eb3cbeb9a9baf2dea5ae773139d2499db2dd2ac\
             92bbc3240f24e943973cea139eed5162b6778951fb570716916c42c0cc88465a\
             9fb25cc10cb41b75948d220a81172bfa6d9eca4481d7f078f020364a7ddc484e\
             50398b47023371369177cfd328459f5c04cba8ec92312677c2ca9f706c4e2d52\
             f91e2d47354e6d833e7b12136c92d10e4f4d3856b7f787bb2da2d508f26c3c4d\
             8876ffa08bc6b42302ca3d36b85c61954f077ce20d9469edff91c361d3b358d5\
             720a7c78",
        )
        .unwrap();
        assert_eq!(sealed.first(), Some(&1));
        let sealed = FetchedBlob::new(sealed);
        assert_eq!(object_key(&sealed).unwrap(), doc_key.id());
        assert_eq!(object_author(&sealed).unwrap(), author);
        assert_eq!(
            open_object(&sealed, &name_aad(id), &doc_key)
                .unwrap()
                .into_plaintext(),
            b"Budget 2027"
        );
    }

    /// A media blob the build before the typed header splits sealed. Its
    /// header still reads, and it still opens under its own document and
    /// address.
    #[test]
    fn a_media_blob_the_old_code_sealed_still_reads_and_opens() {
        let (doc_key, _, author) = keys();
        let id = DocumentId::from_bytes([7; 16]);
        let address = ambre_core::MediaAddress::from_array([0xab; 32]);
        let sealed = FetchedBlob::new(
            hex::decode(
                "01b6e14f706ce523ebafb9bf6eb3cbeb9a9baf2dea5ae773139d2499db2dd2ac\
                 92bbc3240f24e943973cea139eed5162b6955be778488e42b9ef1c4243d0d692\
                 d12fb0ef81fe774113caf4de8eb443efc6e6176caeaa5e5ad6726969c619cd90\
                 fb58db3615fa0f646a3b99200fe6c61f08d8010365a843a2f20863acfee9326c\
                 8f9fcc3f08d46abefb510db0b7ddc66146292306f736cb20066be817d78d3722\
                 3eb83fe56b4825ab50d04c9f9f33538dc7627042df7ac1d9e6a4bdf237baa85b\
                 2ed451",
            )
            .unwrap(),
        );
        assert_eq!(object_key(&sealed).unwrap(), doc_key.id());
        assert_eq!(object_author(&sealed).unwrap(), author);
        assert_eq!(
            open_object(&sealed, &media_aad(id, &address), &doc_key)
                .unwrap()
                .into_plaintext(),
            b"\x89PNG media"
        );
    }

    /// The body of an update as the build before the typed nonce split laid
    /// it out. It holds the session nonce, the summary marker, the summary
    /// when there is one, then the delta. Each later build writes and reads
    /// these bytes.
    #[test]
    fn an_update_body_keeps_the_bytes_of_the_old_layout() {
        let nonce = SessionNonce([0x5a; SESSION_NONCE_LEN]);
        let delta = ambre_core::DeltaBytes::new(b"delta".to_vec());
        let summary = ActivitySummary {
            kind: ActivityKind::Content,
            mentions: vec![UserId::from_bytes([0xA7; 16])],
            mentions_all: ambre_bus::MentionScope::Named,
            anchor: None,
        };
        for (summary, golden) in [
            (None, "5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a0064656c7461"),
            (
                Some(summary),
                "5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a01000001a7a7a7a7a7a7a7a7a7a7a7a7a7a7a7a70064656c7461",
            ),
        ] {
            let body = update_body(&nonce, summary.as_ref(), &delta).unwrap();
            assert_eq!(hex::encode(body.as_bytes()), golden);
            let read = split_update_body(frame(&hex::decode(golden).unwrap())).unwrap();
            assert_eq!(
                read,
                UpdateBody {
                    nonce,
                    summary,
                    delta: delta.clone()
                }
            );
        }
    }
}
