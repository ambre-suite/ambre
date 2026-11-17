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

use ambre_core::activity::{ActivitySummary, SummaryError};
use ambre_core::authorship::SessionNonce;
use ambre_core::id::{Document, Id};
use ambre_core::{
    Aad, Confidential, Count, EphemeralBytes, FixedBytes, KeyId, MediaBytes, Octets, Payload,
    Plaintext, PublicKey, Relay, Seq, SnapshotBytes,
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
const COMPRESS_THRESHOLD: ambre_core::Count<ambre_core::Octets> = ambre_core::Count::new(256);

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
const MIN_DECOMPRESS_CEILING: Count<Octets> = Count::<Octets>::new(1 << 20); // 1 MiB

/// Hard upper bound on any single decompressed message, regardless of the
/// ratio-derived ceiling. Matches the client-memory pillar (< 100 MB per doc).
const ABSOLUTE_DECOMPRESS_CAP: Count<Octets> = Count::<Octets>::new(64 << 20); // 64 MiB

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
    #[error("the snapshot seals another identifier than the one asked for")]
    WrongSnapshot,
    #[error("the snapshot seals another horizon than the one the relay served")]
    WrongHorizon,
    #[error("the snapshot was signed by another key than the author its trust names")]
    WrongAuthor,
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
    #[error("the signer holds no writing role in the verified log")]
    NotAWriter,
    #[error("the log of the document walks again, or roots outside its space")]
    LogUnsettled,
}

impl PipelineError {
    /// The warning a frame refused for this error raises. A missed re-key
    /// and a forged signature each read apart from a corrupt frame
    /// (`SYNC-091`).
    pub(crate) const fn warning(&self) -> ambre_bus::sync_warning::SyncWarningCode {
        use ambre_bus::sync_warning::SyncWarningCode;
        match self {
            Self::Crypto(CryptoError::InvalidSignature) => SyncWarningCode::SignatureRefused,
            Self::WrongKey => SyncWarningCode::KeyNotHeld,
            Self::NotAWriter => SyncWarningCode::WriterRefused,
            _ => SyncWarningCode::DecryptFailed,
        }
    }
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
    pub nonce: FixedBytes<SessionNonce>,
    /// What the update says of itself, for a client that does not hold the
    /// document (`RESID-014`).
    pub summary: Option<ActivitySummary>,
    /// The Loro update, opaque to the envelope.
    pub delta: Payload<ambre_core::DeltaBytes>,
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
    pub fn len(&self) -> ambre_core::Count<ambre_core::Octets> {
        ambre_core::Count::of(self.0.len())
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
    nonce: &FixedBytes<SessionNonce>,
    summary: Option<&ActivitySummary>,
    delta: &Payload<ambre_core::DeltaBytes>,
) -> Result<UpdateFrame, PipelineError> {
    let summary = summary.map(ActivitySummary::encode).transpose()?;
    let summary_len = summary.as_ref().map_or(0, Vec::len);
    // A capacity hint over buffers already in memory: the sum stays below the
    // bound, and saturating states it.
    let mut body = Vec::with_capacity(
        FixedBytes::<SessionNonce>::SIZE
            .saturating_len()
            .to_usize()
            .saturating_add(1)
            .saturating_add(summary_len)
            .saturating_add(delta.as_bytes().len()),
    );
    body.extend_from_slice(nonce.as_slice());
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
    let (nonce, rest) =
        FixedBytes::<SessionNonce>::split_from(body).ok_or(PipelineError::ShortBody)?;
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
        delta: Payload::new(delta.to_vec()),
    })
}

/// Refuses a key other than the one the metadata names (`SYNC-125`). On the
/// seal it keeps a payload from naming a key that did not seal it. On the open
/// it refuses before any cryptography.
fn check_key(named: FixedBytes<KeyId>, doc_key: &DocumentKey) -> Result<(), PipelineError> {
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
    if ambre_core::Count::of(data.len()) > COMPRESS_THRESHOLD {
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
    let ceiling = Count::<Octets>::of(body.len().saturating_mul(MAX_DECOMPRESS_RATIO))
        .clamp(MIN_DECOMPRESS_CEILING, ABSOLUTE_DECOMPRESS_CAP);

    // The frame decoder runs block by block, with no `std::io::Read` adapter.
    // The adapter boxes each decoder error into an `io::Error`. That links
    // the `Display` and `Debug` of every ruzstd error into the Sync worker,
    // and this function drops the error anyway.
    let mut source = body;
    let mut decoder = ruzstd::decoding::FrameDecoder::new();
    decoder
        .init(&mut source)
        .map_err(|_| PipelineError::Decompress)?;

    // Never pre-allocate the whole ceiling. Grow the buffer one block at a
    // time, and stop once the output passes the ceiling. The decoder retains
    // its window until the frame ends, so `out` holds the bytes past it.
    //
    // A refused body leaves a partial plaintext in `out`, and `Secret::wipe`
    // wipes it, and each block's copy too. The decoder's own block and window
    // buffers, and each buffer `out` outgrows, drop unwiped, out of the
    // wrapper's reach.
    let mut out = Vec::new();
    let refusal = loop {
        if let Some(block) = decoder.collect() {
            out.extend_from_slice(&block);
            ambre_core::Secret::wipe(block);
        }
        if Count::<Octets>::of(out.len()) > ceiling {
            break PipelineError::DecompressTooLarge;
        }
        if decoder.is_finished() {
            return Ok(out);
        }
        if decoder
            .decode_blocks(
                &mut source,
                ruzstd::decoding::BlockDecodingStrategy::UptoBlocks(1),
            )
            .is_err()
        {
            break PipelineError::Decompress;
        }
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
    let plaintext = ambre_core::Confidential::<ambre_core::Plaintext>::new(plaintext);
    Ok(envelope::seal(&plaintext, aad, doc_key, signing_key)?)
}

fn open_payload(
    sealed: &Sealed,
    aad: &Aad,
    doc_key: &DocumentKey,
    author_pub: &FixedBytes<PublicKey>,
) -> Result<OpenedPayload, PipelineError> {
    // The opened bytes are lent, with no copy, through the door `ARCH-047`
    // admits. They are still compressed, and the wrapper wipes them as it
    // drops, on the error paths too. The zstd decoder copies them on the
    // way, into its block and window buffers and into each buffer the
    // output outgrows. Those copies are out of the wrapper's reach.
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
    fn into_snapshot(self) -> Payload<SnapshotBytes> {
        Payload::new(self.0)
    }

    /// The payload as one presence sample.
    fn into_ephemeral(self) -> Payload<EphemeralBytes> {
        Payload::new(self.0)
    }
}

fn aad_of<T: serde::Serialize>(public_data: &T) -> Result<Aad, PipelineError> {
    postcard::to_allocvec(public_data)
        .map(Aad::new)
        .map_err(|_| PipelineError::Postcard)
}

/// Rebuilds a [`Sealed`] from a wire message's envelope fields.
fn sealed_from(
    ciphertext: &Payload<Ciphertext>,
    nonce: &FixedBytes<Nonce>,
    commitment_tag: &FixedBytes<CommitmentTag>,
    signature: &FixedBytes<Signature>,
) -> Sealed {
    Sealed {
        nonce: *nonce,
        ciphertext: ciphertext.clone(),
        commitment_tag: *commitment_tag,
        signature: *signature,
    }
}

fn message_parts(
    sealed: Sealed,
) -> (
    Payload<Ciphertext>,
    FixedBytes<Nonce>,
    FixedBytes<CommitmentTag>,
    FixedBytes<Signature>,
) {
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
    crate::log::debug!(
        "[Pipeline] seal_update: {} bytes plaintext -> {} bytes ciphertext",
        payload.len(),
        ciphertext.as_bytes().len(),
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
    crate::log::debug!(
        "[Pipeline] open_update: {} bytes ciphertext -> {} bytes plaintext",
        update.ciphertext.as_bytes().len(),
        frame.len(),
    );
    Ok(frame)
}

/// Whether the author `update` names signed it, checked alone. A frame
/// sealed under a key this client does not hold proves its author this way,
/// so the log head it carries still counts (`SYNC-207`).
#[must_use]
pub fn update_signed(update: &Update) -> bool {
    aad_of(&update.public_data).is_ok_and(|aad| {
        ambre_crypto::signature::verify(
            &update.public_data.pub_key,
            &update.nonce,
            &update.ciphertext,
            &aad,
            &update.commitment_tag,
            &update.signature,
        )
        .is_ok()
    })
}

// -- Snapshot ------------------------------------------------------------------

/// Compresses, encrypts and signs `payload` into a [`Snapshot`].
pub fn seal_snapshot(
    payload: &Payload<SnapshotBytes>,
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

/// What the relay names for a snapshot it serves: its identifier, and the
/// horizon its row records. The open holds the blob to both, because the
/// replay starts after that horizon (`SYNC-257`, `SYNC-259`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ServedSnapshot {
    /// The identifier the client asked for.
    pub snapshot_id: Id<ambre_core::id::Snapshot>,
    /// The last relay number the relay says the snapshot folds in.
    pub covered_seq: Seq<Relay>,
}

/// Decodes a stored snapshot blob, the serialized [`Snapshot`] object storage
/// holds. It refuses another document's, another snapshot than the one
/// `served` names (`SYNC-257`), and a horizon other than the served one
/// (`SYNC-259`), before any decryption. The caller reads the key it names
/// before opening it (`SYNC-045`, `SYNC-119`). The signature covers the
/// identifier and the horizon, so a refusal here is final once the open
/// verifies it.
///
/// # Errors
///
/// * [`PipelineError::Postcard`] for bytes that are not a snapshot.
/// * [`PipelineError::WrongDocument`] for another document's.
/// * [`PipelineError::WrongSnapshot`] for another snapshot of the document.
/// * [`PipelineError::WrongHorizon`] for a sealed horizon the relay did not
///   serve.
pub fn decode_snapshot_blob(
    blob: &Payload<FetchedBlob>,
    document_id: Id<Document>,
    served: ServedSnapshot,
) -> Result<Snapshot, PipelineError> {
    let snapshot: Snapshot =
        postcard::from_bytes(blob.as_bytes()).map_err(|_| PipelineError::Postcard)?;
    if snapshot.public_data.document_id != document_id {
        return Err(PipelineError::WrongDocument);
    }
    if snapshot.public_data.snapshot_id != served.snapshot_id {
        return Err(PipelineError::WrongSnapshot);
    }
    if snapshot.public_data.covered_seq != served.covered_seq {
        return Err(PipelineError::WrongHorizon);
    }
    Ok(snapshot)
}

/// Verifies, decrypts and decompresses a [`Snapshot`] back to its CRDT payload.
pub fn open_snapshot(
    snapshot: &Snapshot,
    doc_key: &DocumentKey,
) -> Result<Payload<SnapshotBytes>, PipelineError> {
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
    payload: &Payload<EphemeralBytes>,
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

/// Whether the sender `ephemeral` names signed it, checked alone. A sample
/// under a key this client lacks proves its sender this way. Its log head
/// then still counts (`SYNC-263`).
#[must_use]
pub fn ephemeral_signed(ephemeral: &Ephemeral) -> bool {
    aad_of(&ephemeral.public_data).is_ok_and(|aad| {
        ambre_crypto::signature::verify(
            &ephemeral.public_data.pub_key,
            &ephemeral.nonce,
            &ephemeral.ciphertext,
            &aad,
            &ephemeral.commitment_tag,
            &ephemeral.signature,
        )
        .is_ok()
    })
}

/// Verifies, decrypts and decompresses an [`Ephemeral`] back to its payload.
pub fn open_ephemeral(
    ephemeral: &Ephemeral,
    doc_key: &DocumentKey,
) -> Result<Payload<EphemeralBytes>, PipelineError> {
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
    pub(crate) fn update_signature(&self) -> Option<FixedBytes<Signature>> {
        match &self.0 {
            WsClientMsg::Update(update) => Some(update.signature),
            _ => None,
        }
    }
}

/// The role of a blob as object storage returned it: sealed by some writer,
/// and neither verified nor opened yet. `Payload<FetchedBlob>`. Distinct from
/// [`SealedBlob`], which only the local seal builds, so a download never
/// stands in for an upload. [`decode_snapshot_blob`] reads a snapshot from
/// it, and [`open_object`] a media blob. `Debug` prints the length and none
/// of the bytes.
pub enum FetchedBlob {}

impl ambre_core::PayloadKind for FetchedBlob {
    const NAME: ambre_core::StaticKey<ambre_core::DebugNames> =
        ambre_core::StaticKey::new("FetchedBlob");
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
    pub(crate) fn to_body(&self) -> Payload<crate::http::HttpBody> {
        Payload::new(self.0.clone())
    }

    /// The sealed bytes, moved out, for a test that tampers with them or
    /// serves them back as a download.
    #[cfg(test)]
    pub(crate) fn into_bytes(self) -> Vec<u8> {
        self.0
    }

    /// The sealed bytes, moved out, as the body of a key link.
    pub(crate) fn into_previous_key(self) -> Payload<ambre_core::SealedPreviousKey> {
        Payload::new(self.0)
    }

    /// The sealed bytes as the name of a document, which the relay stores
    /// and never opens.
    pub(crate) fn into_name(self) -> ambre_core::Encoded<ambre_core::SealedName> {
        ambre_core::Encoded::new(self.0)
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
    impl Sealed for ambre_core::Payload<super::FetchedBlob> {}
    impl Sealed for ambre_core::Encoded<ambre_core::SealedName> {}
    impl Sealed for ambre_core::Payload<ambre_core::SealedPreviousKey> {}
}

impl StoredObject for SealedBlob {
    fn object(&self) -> &[u8] {
        &self.0
    }
}

impl StoredObject for Payload<FetchedBlob> {
    fn object(&self) -> &[u8] {
        self.as_bytes()
    }
}

impl StoredObject for ambre_core::Encoded<ambre_core::SealedName> {
    fn object(&self) -> &[u8] {
        self.as_bytes()
    }
}

impl StoredObject for Payload<ambre_core::SealedPreviousKey> {
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
    payload: &Payload<EphemeralBytes>,
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
    payload: &Payload<SnapshotBytes>,
    public_data: SnapshotPublicData,
    doc_key: &DocumentKey,
    signing_key: &SigningKey,
) -> Result<SealedBlob, PipelineError> {
    let snapshot = seal_snapshot(payload, public_data, doc_key, signing_key)?;
    postcard::to_allocvec(&snapshot)
        .map(SealedBlob)
        .map_err(|_| PipelineError::Postcard)
}

/// A snapshot sealed once to measure it, and kept to seal again under the
/// identifier the relay mints for its upload link (`SYNC-258`). The upload
/// link binds the size, and the identifier is part of the signed public data,
/// so the size comes first. An identifier has one encoded length, so the
/// second seal has the size of the first. `Debug` prints the size alone.
pub struct SnapshotDraft {
    payload: Payload<SnapshotBytes>,
    public_data: SnapshotPublicData,
    doc_key: DocumentKey,
    signing_key: SigningKey,
    size: Count<Octets>,
}

impl std::fmt::Debug for SnapshotDraft {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "SnapshotDraft({} bytes)", self.size.get())
    }
}

impl SnapshotDraft {
    /// Seals `payload` once under `public_data`, which runs the wire checks
    /// before anything leaves (`SYNC-022`), and keeps what the final seal
    /// needs.
    ///
    /// # Errors
    ///
    /// Every error [`snapshot_blob`] raises.
    pub fn new(
        payload: Payload<SnapshotBytes>,
        public_data: SnapshotPublicData,
        doc_key: DocumentKey,
        signing_key: SigningKey,
    ) -> Result<Self, PipelineError> {
        let size = Count::<Octets>::of(
            snapshot_blob(&payload, public_data.clone(), &doc_key, &signing_key)?.len(),
        );
        Ok(Self {
            payload,
            public_data,
            doc_key,
            signing_key,
            size,
        })
    }

    /// The size the upload link binds: the size of every seal of this draft.
    pub const fn size(&self) -> Count<Octets> {
        self.size
    }

    /// The horizon every seal of this draft carries, which the upload
    /// metadata names (`SYNC-259`).
    pub const fn covered_seq(&self) -> Seq<Relay> {
        self.public_data.covered_seq
    }

    /// The author's confirmation of the seal under `snapshot_id`: the
    /// signature over the document, that identifier, the horizon and the
    /// author's key (`AUTHOR-031`).
    pub fn confirm(&self, snapshot_id: Id<ambre_core::id::Snapshot>) -> FixedBytes<Signature> {
        let transcript = ambre_protocol::snapshot_confirm::confirm_transcript(
            self.public_data.document_id,
            snapshot_id,
            self.public_data.covered_seq,
            &self.public_data.pub_key,
        );
        ambre_crypto::signature::sign_message(&self.signing_key, &transcript)
    }

    /// Seals the snapshot under `snapshot_id`, the identifier the relay
    /// minted for its upload link.
    ///
    /// # Errors
    ///
    /// Every error [`snapshot_blob`] raises.
    pub fn seal(
        &self,
        snapshot_id: Id<ambre_core::id::Snapshot>,
    ) -> Result<SealedBlob, PipelineError> {
        let public_data = SnapshotPublicData {
            snapshot_id,
            ..self.public_data.clone()
        };
        snapshot_blob(&self.payload, public_data, &self.doc_key, &self.signing_key)
    }
}

/// Seals a media blob into a standalone object bound for object storage, in
/// the layout and under the AAD of [`seal_object`]. The cipher reads the
/// blob where it lies, with no copy into a `Confidential<Plaintext>` first.
///
/// # Errors
///
/// Every error [`seal_object`] raises.
pub fn object_blob(
    plaintext: &Payload<MediaBytes>,
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
const OBJECT_HEADER: Count<Octets> = ObjectLayout::SIZE
    .plus(FixedBytes::<KeyId>::SIZE)
    .plus(FixedBytes::<PublicKey>::SIZE)
    .plus(FixedBytes::<Signature>::SIZE)
    .plus(FixedBytes::<CommitmentTag>::SIZE)
    .plus(FixedBytes::<Nonce>::SIZE);

/// Domain of a sealed document name. The document id follows it in the
/// additional data, so a name opens under its own document only (`SYNC-068`).
const NAME_DOMAIN: &[u8] = b"ambre-doc-name-v2";

/// The additional data of a document name: the domain, then the document id.
pub fn name_aad(document_id: Id<Document>) -> Aad {
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
pub fn media_aad(document_id: Id<Document>, address: &FixedBytes<ambre_core::MediaAddress>) -> Aad {
    let mut aad = MEDIA_DOMAIN.to_vec();
    aad.extend_from_slice(document_id.as_bytes());
    aad.extend_from_slice(address.as_array());
    Aad::new(aad)
}

/// The additional data an object is sealed under: the layout and the key
/// identifier its header names, then the caller's own. The signature covers
/// all three, so a header cannot be made to name another layout or key.
fn object_aad(key_id: FixedBytes<KeyId>, aad: &Aad) -> Aad {
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
    plaintext: &Confidential<Plaintext>,
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
fn object_bytes(
    key_id: FixedBytes<KeyId>,
    signing_key: &SigningKey,
    sealed: &Sealed,
) -> SealedBlob {
    let mut out = Vec::with_capacity(
        OBJECT_HEADER
            .plus(sealed.ciphertext.len())
            .saturating_len()
            .to_usize(),
    );
    out.push(ObjectLayout::CURRENT.byte());
    out.extend_from_slice(key_id.as_array());
    out.extend_from_slice(signing_key.verifying_key().as_bytes());
    out.extend_from_slice(sealed.signature.as_slice());
    out.extend_from_slice(sealed.commitment_tag.as_slice());
    out.extend_from_slice(sealed.nonce.as_slice());
    out.extend_from_slice(sealed.ciphertext.as_bytes());
    SealedBlob(out)
}

/// Splits what [`object_bytes`] laid out, refusing a key other than
/// `doc_key` before any cryptography (`SYNC-125`).
fn object_parts(
    object: &impl StoredObject,
    doc_key: &DocumentKey,
) -> Result<(FixedBytes<KeyId>, FixedBytes<PublicKey>, Sealed), PipelineError> {
    let bytes = object.object();
    check_layout(bytes)?;
    let (_, rest) = bytes.split_first().ok_or(PipelineError::ShortObject)?;
    let (key_id, rest) = FixedBytes::<KeyId>::split_from(rest).ok_or(PipelineError::ShortObject)?;
    let (author, rest) =
        FixedBytes::<PublicKey>::split_from(rest).ok_or(PipelineError::ShortObject)?;
    let (signature, rest) =
        FixedBytes::<Signature>::split_from(rest).ok_or(PipelineError::ShortObject)?;
    let (commitment_tag, rest) =
        FixedBytes::<CommitmentTag>::split_from(rest).ok_or(PipelineError::ShortObject)?;
    let (nonce, ciphertext) =
        FixedBytes::<Nonce>::split_from(rest).ok_or(PipelineError::ShortObject)?;
    check_key(key_id, doc_key)?;
    let sealed = Sealed {
        nonce,
        ciphertext: Payload::<Ciphertext>::new(ciphertext.to_vec()),
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
) -> Result<Confidential<Plaintext>, PipelineError> {
    let (key_id, author, sealed) = object_parts(object, doc_key)?;
    Ok(envelope::open(
        &sealed,
        &object_aad(key_id, aad),
        doc_key,
        &author,
    )?)
}

// -- Key links -----------------------------------------------------------------

/// Domain of a key link. Three fields follow it: the document id, the key it
/// seals and the log head its rotator read. So a link opens for its own
/// document, key and head only. Version 2 added the head
/// (`SYNC-204`).
const LINK_DOMAIN: &[u8] = b"ambre-key-link-v2";

/// The additional data of a link: the domain, the document id, the
/// identifier of the key it seals, then the head (`SYNC-204`). The head
/// takes its place as 8 big-endian bytes, then a presence byte and the
/// digest of the last entry when there is one.
fn link_aad(
    document_id: Id<Document>,
    previous: FixedBytes<KeyId>,
    head: ambre_protocol::LogHead,
) -> Aad {
    let mut aad = LINK_DOMAIN.to_vec();
    aad.extend_from_slice(document_id.as_bytes());
    aad.extend_from_slice(previous.as_array());
    aad.extend_from_slice(&head.position.get().to_be_bytes());
    match head.previous {
        Some(digest) => {
            aad.push(1);
            aad.extend_from_slice(digest.as_array());
        }
        None => aad.push(0),
    }
    Aad::new(aad)
}

/// Seals `previous` under `next` and signs it: the link a rotation leaves
/// (`SYNC-116`). The sealed bytes take the layout of a standalone object, so
/// their header names `next` and the rotator. `head` is the head of the
/// membership log the rotator read, which the signature covers (`SYNC-204`).
///
/// # Errors
///
/// [`PipelineError::Crypto`] from the envelope.
pub fn seal_link(
    document_id: Id<Document>,
    previous: &DocumentKey,
    next: &DocumentKey,
    head: ambre_protocol::LogHead,
    signing_key: &SigningKey,
) -> Result<ambre_protocol::KeyLink, PipelineError> {
    let key = next.id();
    let aad = object_aad(key, &link_aad(document_id, previous.id(), head));
    let sealed = next.seal_previous(previous, &aad, signing_key)?;
    Ok(ambre_protocol::KeyLink {
        document_id,
        key,
        previous: previous.id(),
        head,
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
    document_id: Id<Document>,
    next: &DocumentKey,
) -> Result<DocumentKey, PipelineError> {
    if link.document_id != document_id {
        return Err(PipelineError::WrongDocument);
    }
    check_key(link.key, next)?;
    let (key_id, author, sealed) = object_parts(&link.sealed, next)?;
    let aad = object_aad(key_id, &link_aad(document_id, link.previous, link.head));
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
pub fn object_key(object: &impl StoredObject) -> Result<FixedBytes<KeyId>, PipelineError> {
    let bytes = object.object();
    check_layout(bytes)?;
    bytes
        .split_first()
        .and_then(|(_, rest)| FixedBytes::<KeyId>::split_from(rest))
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
pub fn object_author(object: &impl StoredObject) -> Result<FixedBytes<PublicKey>, PipelineError> {
    let bytes = object.object();
    check_layout(bytes)?;
    bytes
        .split_first()
        .and_then(|(_, rest)| FixedBytes::<KeyId>::split_from(rest))
        .and_then(|(_, rest)| FixedBytes::<PublicKey>::split_from(rest))
        .map(|(author, _)| author)
        .ok_or(PipelineError::ShortObject)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A forged signature, a key not held and a signer with no writing role
    /// each raise a warning of their own. Any other refusal reads as a
    /// frame that does not decrypt.
    #[test]
    fn each_refusal_of_a_frame_raises_its_own_warning() {
        use ambre_bus::sync_warning::SyncWarningCode;
        let cases = [
            (
                PipelineError::Crypto(CryptoError::InvalidSignature),
                SyncWarningCode::SignatureRefused,
            ),
            (PipelineError::WrongKey, SyncWarningCode::KeyNotHeld),
            (PipelineError::NotAWriter, SyncWarningCode::WriterRefused),
            (PipelineError::ShortBody, SyncWarningCode::DecryptFailed),
        ];
        for (error, warning) in cases {
            assert_eq!(error.warning(), warning, "{error}");
        }
    }

    /// A frame and a sealed blob count their bytes, and their `Debug`
    /// prints the count alone, never a byte of what they hold.
    #[test]
    fn a_frame_and_a_blob_print_their_size_alone() {
        let frame = UpdateFrame::from_bytes(b"secret".to_vec());
        assert_eq!(frame.len().get(), 6);
        assert_eq!(format!("{frame:?}"), "UpdateFrame(6 bytes)");
        let blob = object_blob(
            &Payload::new(b"secret".to_vec()),
            &ambre_core::Aad::of(b"aad"),
            &DocumentKey::generate(),
            &SigningKey::from_bytes(&[7; 32]),
        )
        .expect("seal");
        assert_eq!(
            format!("{blob:?}"),
            format!("SealedBlob({} bytes)", blob.len())
        );
        assert_eq!(blob.to_body().as_bytes(), blob.as_bytes());
    }
    use ambre_core::PublicKey;
    use ambre_core::activity::ActivityKind;
    use ambre_core::authorship::SESSION_NONCE_LEN;
    use ambre_core::id::{self, User};
    use ambre_crypto::MasterSeed;
    use ambre_protocol::messages::{DeltaAuthor, Ephemerals, Seq};

    /// Plaintext as the object seal takes it.
    fn secret(bytes: &[u8]) -> Confidential<Plaintext> {
        Confidential::<Plaintext>::new(bytes.to_vec())
    }

    /// Object bytes as a download returns them.
    fn fetched(bytes: &[u8]) -> Payload<FetchedBlob> {
        Payload::<FetchedBlob>::new(bytes.to_vec())
    }

    /// Bytes sealed as a frame as they stand, framed or not.
    fn frame(bytes: &[u8]) -> UpdateFrame {
        UpdateFrame::from_bytes(bytes.to_vec())
    }

    fn keys() -> (DocumentKey, SigningKey, FixedBytes<PublicKey>) {
        let doc_key = DocumentKey::from_array([0x11; 32]);
        let signing = MasterSeed::from_array([0x22; 32]).derive_signing_key();
        let author = FixedBytes::from_array(signing.verifying_key().to_bytes());
        (doc_key, signing, author)
    }

    /// The identifier of the key [`keys`] returns.
    fn key_id() -> FixedBytes<KeyId> {
        DocumentKey::from_array([0x11; 32]).id()
    }

    fn update_pd(author: FixedBytes<PublicKey>) -> UpdatePublicData {
        UpdatePublicData {
            document_id: Id::<Document>::mint(),
            pub_key: author,
            key_id: key_id(),
            snapshot_id: Id::<id::Snapshot>::mint(),
            clock: Seq::<DeltaAuthor>::new(1),
            log_head: ambre_protocol::LogHead::EMPTY,
            log_entry: None,
        }
    }

    /// `SYNC-206`: the log head rides under the signature. A head rewritten
    /// in transit fails the signature checked alone, and fails the open.
    #[test]
    fn a_rewritten_log_head_breaks_the_signature() {
        let (doc_key, signing, author) = keys();
        let mut public_data = update_pd(author);
        public_data.log_head =
            ambre_protocol::LogHead::EMPTY.after(FixedBytes::from_array([3; 32]));
        let mut update = seal_update(&frame(b"d"), public_data, &doc_key, &signing).unwrap();
        assert!(update_signed(&update));
        update.public_data.log_head = ambre_protocol::LogHead::EMPTY;
        assert!(!update_signed(&update));
        assert!(matches!(
            open_update(&update, &doc_key),
            Err(PipelineError::Crypto(CryptoError::InvalidSignature))
        ));
    }

    /// `SYNC-218`: the entry that proves the head rides under the signature
    /// too. A relay that strips it, or swaps it, breaks the signature.
    #[test]
    fn a_stripped_log_entry_breaks_the_signature() {
        use crate::membership_log::tests::{Person, created};
        let (doc_key, signing, author) = keys();
        let entries = created(&Person::new(1), &Person::new(2));
        let mut public_data = update_pd(author);
        public_data.log_entry = entries.last().cloned().map(Box::new);
        let mut update = seal_update(&frame(b"d"), public_data, &doc_key, &signing).unwrap();
        assert!(update_signed(&update));
        update.public_data.log_entry = entries.first().cloned().map(Box::new);
        assert!(!update_signed(&update));
        update.public_data.log_entry = None;
        assert!(!update_signed(&update));
    }

    /// `SYNC-262`: the head and the entry a presence sample carries ride
    /// under its signature. A relay that rewrites either one, or strips
    /// the entry, breaks the signature, alone and at the open.
    #[test]
    fn a_rewritten_presence_head_breaks_the_signature() {
        use crate::membership_log::tests::{Person, created};
        let (doc_key, signing, author) = keys();
        let entries = created(&Person::new(1), &Person::new(2));
        let public_data = EphemeralPublicData {
            document_id: Id::<Document>::mint(),
            pub_key: author,
            key_id: key_id(),
            counter: Seq::<Ephemerals>::new(1),
            log_head: crate::membership_log::ServedHeads::of(&entries).last(),
            log_entry: entries.last().cloned().map(Box::new),
        };
        let sealed = seal_ephemeral(
            &Payload::new(b"c".to_vec()),
            public_data,
            &doc_key,
            &signing,
        )
        .unwrap();
        assert!(ephemeral_signed(&sealed));
        let mut head = sealed.clone();
        head.public_data.log_head = ambre_protocol::LogHead::EMPTY;
        let mut swapped = sealed.clone();
        swapped.public_data.log_entry = entries.first().cloned().map(Box::new);
        let mut stripped = sealed;
        stripped.public_data.log_entry = None;
        for forged in [head, swapped, stripped] {
            assert!(!ephemeral_signed(&forged));
            assert!(matches!(
                open_ephemeral(&forged, &doc_key),
                Err(PipelineError::Crypto(CryptoError::InvalidSignature))
            ));
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
            document_id: Id::<Document>::mint(),
            pub_key: author,
            key_id: key_id(),
            snapshot_id: Id::<id::Snapshot>::mint(),
            covered_seq: Seq::<Relay>::new(7),
            parent_snapshot_proof: None,
            parent_snapshot_update_clocks: Vec::new(),
        };
        let snapshot =
            seal_snapshot(&Payload::new(payload.to_vec()), pd, &doc_key, &signing).unwrap();
        // Compressed ciphertext is far smaller than the 4 KiB plaintext.
        assert!(snapshot.ciphertext.as_bytes().len() < payload.len());
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
            document_id: Id::<Document>::mint(),
            pub_key: author,
            key_id: key_id(),
            snapshot_id: Id::<id::Snapshot>::mint(),
            covered_seq: Seq::<Relay>::new(7),
            parent_snapshot_proof: None,
            parent_snapshot_update_clocks: vec![
                (author, Seq::<DeltaAuthor>::new(1)),
                (author, Seq::<DeltaAuthor>::new(2)),
            ],
        };
        assert!(matches!(
            seal_snapshot(&Payload::new(b"board".to_vec()), pd, &doc_key, &signing),
            Err(PipelineError::Validation(
                ValidationError::DuplicatePublicKey
            ))
        ));
    }

    /// The frame with its four sealed parts blanked: what the relay reads of
    /// it, the public metadata and the shape around it.
    fn readable_part(update: &Update) -> Vec<u8> {
        let mut blank = update.clone();
        blank.ciphertext = Payload::<Ciphertext>::new(Vec::new());
        blank.nonce = FixedBytes::from_array([0u8; 24]);
        blank.commitment_tag = FixedBytes::from_array([0u8; 32]);
        blank.signature = FixedBytes::from_array([0u8; 64]);
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
        let first = FixedBytes::<SessionNonce>::from_array([1; SESSION_NONCE_LEN]);
        let second = FixedBytes::<SessionNonce>::from_array([2; SESSION_NONCE_LEN]);
        let seal = |nonce: &FixedBytes<SessionNonce>| {
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
                    delta: Payload::new(b"delta".to_vec())
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
        let nonce = FixedBytes::<SessionNonce>::from_array([1; SESSION_NONCE_LEN]);
        let mentioned = Id::<User>::from_bytes([0xA7; 16]);
        let summary = ActivitySummary {
            kind: ActivityKind::Content,
            mentions: vec![mentioned],
            mentions_all: ambre_bus::MentionScope::Named,
            anchor: None,
            call: None,
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
                delta: Payload::new(b"delta".to_vec())
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
            &FixedBytes::<SessionNonce>::from_array([1; SESSION_NONCE_LEN]),
            None,
            &b"d".to_vec().into(),
        )
        .unwrap()
        .into_bytes();
        if let Some(marker) =
            body.get_mut(FixedBytes::<SessionNonce>::SIZE.saturating_len().to_usize())
        {
            *marker = 2;
        }
        assert!(matches!(
            split_update_body(UpdateFrame::from_bytes(body)),
            Err(PipelineError::UnknownSummary)
        ));
        let mut cut = vec![1; FixedBytes::<SessionNonce>::SIZE.saturating_len().to_usize()];
        cut.extend_from_slice(&[SummaryMarker::Present.byte(), 0, 0]);
        assert!(matches!(
            split_update_body(UpdateFrame::from_bytes(cut)),
            Err(PipelineError::Summary(SummaryError::Short))
        ));
        let empty = split_update_body(
            update_body(
                &FixedBytes::<SessionNonce>::from_array([1; 16]),
                None,
                &b"".to_vec().into(),
            )
            .unwrap(),
        )
        .unwrap();
        assert!(empty.delta.is_empty());
    }

    /// `SYNC-258`: a draft seals the identifier the relay minted, at the size
    /// it measured, whatever the identifier and whether the payload
    /// compresses or not. `SYNC-257`: the blob opens under that identifier
    /// alone.
    #[test]
    fn a_draft_seals_the_minted_identifier_at_its_measured_size() {
        let (doc_key, signing, author) = keys();
        let document_id = Id::<Document>::mint();
        for len in [1_usize, 127, 128, 4_096, 70_000] {
            let payload: Vec<u8> = (0..len).map(|i| (i % 251) as u8).collect();
            let pd = SnapshotPublicData {
                document_id,
                pub_key: author,
                key_id: key_id(),
                snapshot_id: Id::<id::Snapshot>::genesis(document_id),
                covered_seq: Seq::<Relay>::new(7),
                parent_snapshot_proof: None,
                parent_snapshot_update_clocks: Vec::new(),
            };
            let draft = SnapshotDraft::new(
                Payload::new(payload.clone()),
                pd,
                doc_key.clone(),
                signing.clone(),
            )
            .unwrap();
            let minted = Id::<id::Snapshot>::mint();
            let blob = draft.seal(minted).unwrap();
            assert_eq!(Count::<Octets>::of(blob.len()), draft.size(), "{len} bytes");
            let fetched = Payload::<FetchedBlob>::new(blob.into_bytes());
            let served = |covered| ServedSnapshot {
                snapshot_id: minted,
                covered_seq: Seq::<Relay>::new(covered),
            };
            let opened = decode_snapshot_blob(&fetched, document_id, served(7)).unwrap();
            assert_eq!(
                open_snapshot(&opened, &doc_key).unwrap().as_bytes(),
                payload.as_slice()
            );
            assert!(matches!(
                decode_snapshot_blob(
                    &fetched,
                    document_id,
                    ServedSnapshot {
                        snapshot_id: Id::<id::Snapshot>::mint(),
                        ..served(7)
                    }
                ),
                Err(PipelineError::WrongSnapshot)
            ));
            // `SYNC-259`: a horizon short of the sealed one, or past it.
            for covered in [6, 8] {
                assert!(matches!(
                    decode_snapshot_blob(&fetched, document_id, served(covered)),
                    Err(PipelineError::WrongHorizon)
                ));
            }
        }
    }

    /// What the upload sends holds no byte run of the plaintext. It opens
    /// back to it, a snapshot blob and a media blob alike (`SYNC-049`,
    /// `ZK-001`).
    #[test]
    fn a_blob_bound_for_storage_is_sealed() {
        let (doc_key, signing, author) = keys();
        let document_id = Id::<Document>::mint();
        let plaintext = b"a picture's bytes, or a document's whole state, in the clear";
        let contains = |hay: &[u8]| {
            hay.windows(16)
                .any(|w| plaintext.windows(16).any(|p| p == w))
        };

        let snapshot_id = Id::<id::Snapshot>::mint();
        let pd = SnapshotPublicData {
            document_id,
            pub_key: author,
            key_id: key_id(),
            snapshot_id,
            covered_seq: Seq::<Relay>::new(7),
            parent_snapshot_proof: None,
            parent_snapshot_update_clocks: Vec::new(),
        };
        let snapshot =
            snapshot_blob(&Payload::new(plaintext.to_vec()), pd, &doc_key, &signing).unwrap();
        assert!(!contains(snapshot.as_bytes()));
        let decoded = decode_snapshot_blob(
            &Payload::<FetchedBlob>::new(snapshot.as_bytes().to_vec()),
            document_id,
            ServedSnapshot {
                snapshot_id,
                covered_seq: Seq::<Relay>::new(7),
            },
        )
        .unwrap();
        assert_eq!(open_snapshot(&decoded, &doc_key).unwrap(), plaintext);

        let aad = name_aad(document_id);
        let object =
            object_blob(&Payload::new(plaintext.to_vec()), &aad, &doc_key, &signing).unwrap();
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
            document_id: Id::<Document>::mint(),
            pub_key: author,
            key_id: key_id(),
            counter: Seq::<Ephemerals>::new(7),
            log_head: ambre_protocol::LogHead::EMPTY,
            log_entry: None,
        };
        let eph = seal_ephemeral(&Payload::new(payload.to_vec()), pd, &doc_key, &signing).unwrap();
        assert_eq!(open_ephemeral(&eph, &doc_key).unwrap(), payload);
    }

    /// Each wire message names the key that sealed it, and a seal refuses
    /// metadata that names another one (`SYNC-125`).
    #[test]
    fn a_frame_names_the_key_that_sealed_it() {
        let (doc_key, signing, author) = keys();
        let update = seal_update(&frame(b"delta"), update_pd(author), &doc_key, &signing).unwrap();
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
        let key_at = ObjectLayout::SIZE.saturating_len().to_usize();
        renamed[key_at..key_at + FixedBytes::<KeyId>::SIZE.saturating_len().to_usize()]
            .copy_from_slice(doc_key.id().as_array());
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
        let small = vec![0u8; COMPRESS_THRESHOLD.get() as usize];
        let (v_small, _) = compress(&small);
        assert_eq!(v_small, CompressionVersion::Raw);

        let large = vec![0u8; COMPRESS_THRESHOLD.get() as usize + 1];
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

    /// The ceiling is inclusive: an output of exactly the floor opens, and
    /// one byte more is refused.
    #[test]
    fn an_output_at_the_ceiling_opens_and_one_byte_past_is_refused() {
        let floor = usize::try_from(MIN_DECOMPRESS_CEILING.get()).expect("fits");
        let at = vec![0u8; floor];
        let compressed = compress_to_vec(at.as_slice(), CompressionLevel::Fastest);
        assert!(
            compressed.len() * MAX_DECOMPRESS_RATIO < floor,
            "the floor binds"
        );
        assert_eq!(
            decompress(CompressionVersion::Zstd, &compressed).unwrap(),
            at
        );
        let past = vec![0u8; floor + 1];
        let compressed = compress_to_vec(past.as_slice(), CompressionLevel::Fastest);
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

    /// A payload of many blocks opens whole: the decoder runs one block at a
    /// time and the output gathers each block past the window.
    #[test]
    fn a_payload_of_many_blocks_round_trips() {
        let payload: Vec<u8> = (0u32..600_000)
            .map(|i| u8::try_from(i.wrapping_mul(2_654_435_761) >> 24).expect("one byte"))
            .collect();
        let compressed = compress_to_vec(payload.as_slice(), CompressionLevel::Fastest);
        assert_eq!(
            decompress(CompressionVersion::Zstd, &compressed).unwrap(),
            payload
        );
    }

    /// A frame cut short, or a body that is no frame, is refused as a failed
    /// decompression, never as an output past the ceiling.
    #[test]
    fn a_truncated_frame_is_refused() {
        let payload = vec![0x5Au8; 4096];
        let compressed = compress_to_vec(payload.as_slice(), CompressionLevel::Fastest);
        let cut = compressed
            .get(..compressed.len() - 3)
            .expect("a longer frame");
        assert!(matches!(
            decompress(CompressionVersion::Zstd, cut),
            Err(PipelineError::Decompress)
        ));
        assert!(matches!(
            decompress(CompressionVersion::Zstd, b"not a zstd frame"),
            Err(PipelineError::Decompress)
        ));
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
        update.public_data.document_id = Id::<Document>::mint();
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
            .plus(FixedBytes::<KeyId>::SIZE)
            .plus(FixedBytes::<PublicKey>::SIZE)
            .saturating_len()
            .to_usize();
        let mut forged = sealed.clone();
        forged[signature_at] ^= 1;
        assert!(matches!(
            open_object(&fetched(&forged), aad, &doc_key),
            Err(PipelineError::Crypto(CryptoError::InvalidSignature))
        ));

        // The tag: a wrong one, re-signed so the signature passes, still fails.
        let tag_at = signature_at + FixedBytes::<Signature>::SIZE.saturating_len().to_usize();
        let nonce_at = tag_at
            + FixedBytes::<CommitmentTag>::SIZE
                .saturating_len()
                .to_usize();
        let header_len = OBJECT_HEADER.saturating_len().to_usize();
        let mut moved = sealed.clone();
        moved[tag_at] ^= 1;
        let tag = FixedBytes::<CommitmentTag>::from_slice(&moved[tag_at..nonce_at]).unwrap();
        let nonce = FixedBytes::<Nonce>::from_slice(&moved[nonce_at..header_len]).unwrap();
        let body = Payload::<ambre_core::Ciphertext>::new(moved[header_len..].to_vec());
        let bound = object_aad(doc_key.id(), aad);
        let resigned = signature::sign(&signing, &nonce, &body, &bound, &tag);
        moved[signature_at..tag_at].copy_from_slice(resigned.as_slice());
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
        let own = Id::<Document>::mint();
        let sealed =
            seal_object(&secret(b"Budget 2027"), &name_aad(own), &doc_key, &signing).unwrap();
        assert_eq!(
            open_object(&sealed, &name_aad(own), &doc_key)
                .unwrap()
                .into_plaintext(),
            b"Budget 2027"
        );
        assert!(open_object(&sealed, &name_aad(Id::<Document>::mint()), &doc_key).is_err());
    }

    /// A name the build before the typed layout sealed: `layout || key id ||
    /// author key || signature || commitment tag || nonce || ciphertext`. The
    /// relay keeps such objects across releases, so each later build reads its
    /// header and opens it.
    #[test]
    fn an_object_the_old_code_sealed_still_reads_and_opens() {
        let (doc_key, _, author) = keys();
        let id = Id::<Document>::from_bytes([7; 16]);
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
        let sealed = Payload::<FetchedBlob>::new(sealed);
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
        let id = Id::<Document>::from_bytes([7; 16]);
        let address = FixedBytes::<ambre_core::MediaAddress>::from_array([0xab; 32]);
        let sealed = Payload::<FetchedBlob>::new(
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
        let nonce = FixedBytes::<SessionNonce>::from_array([0x5a; SESSION_NONCE_LEN]);
        let delta = Payload::<ambre_core::DeltaBytes>::new(b"delta".to_vec());
        let summary = ActivitySummary {
            kind: ActivityKind::Content,
            mentions: vec![Id::<User>::from_bytes([0xA7; 16])],
            mentions_all: ambre_bus::MentionScope::Named,
            anchor: None,
            call: None,
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
