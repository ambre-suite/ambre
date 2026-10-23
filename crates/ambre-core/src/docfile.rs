//! The `.ambre` container -- the shared header of the native document file
//! (`ambre-spec-docfile.md`).
//!
//! This module owns the *container* half alone: the 22-byte header, the version
//! registry with its migrations, and the error taxonomy. The *payload* half --
//! a `LoroDoc` snapshot -- belongs to each app's interop module, which calls in
//! here rather than re-deriving it (I5: one container).
//!
//! Two lineages share one version byte, and its top bit is the partition: clear
//! for a plaintext container, set for a passphrase-sealed one (`DOCFILE-010`).
//! A sealed file therefore tells itself apart from the header alone, and an
//! older client refuses it rather than parsing a salt as a `doc_type`.
//!
//! Plaintext layout, offsets fixed:
//!
//! ```text
//!   0   4   magic       0x414D4252 ("AMBR")
//!   4   1   version     container version, current 0x01
//!   5   1   doc_type    DocType as u8
//!   6  16   document_id UUID v7
//!  22   ..  payload     LoroDoc::export_snapshot(), plaintext
//! ```
//!
//! Sealed layout, offsets fixed:
//!
//! ```text
//!   0   4   magic       0x414D4252 ("AMBR")
//!   4   1   version     sealed container version, current 0x80
//!   5  16   salt        Argon2id salt, fresh per file
//!  21  24   nonce       XChaCha20 nonce, fresh per file
//!  45   ..  ciphertext  a whole plaintext container, sealed
//! ```
//!
//! The sealed header carries no `doc_type` and no `document_id`: what it seals
//! is an entire plaintext container, header included, so both travel inside the
//! ciphertext. A sealed file therefore names neither the application that wrote
//! it nor the document it holds. Opening one yields bytes the plaintext reader
//! above accepts unchanged, so dispatch, version recovery and the schema gate
//! come along for free.
//!
//! Four invariants shape the API:
//! * **I2 self-describing** -- `version` and `doc_type` precede any app-specific
//!   parsing, so a reader identifies a file without guessing from its extension.
//! * **I3 non-destructive** -- every entry point takes `&[u8]` and returns fresh
//!   bytes. No path here mutates its input, under any error.
//! * **I4 no cryptography** -- pure byte handling, both lineages. This module
//!   lays a sealed file out and takes it apart. It derives no key and runs no
//!   cipher, and `ambre-core` declares no crypto dependency that would let it
//!   (`DOCFILE-006`). The seal itself runs in the Sync worker, the one place
//!   that may hold a primitive (`ARCH-004`).
//! * **Writer rule** -- writers only ever emit [`AMBRE_VERSION`]. Compatibility
//!   is a read-side obligation: one writer path, N reader paths.

use std::fmt;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{Ciphertext, DocType, DocumentId, Nonce, PlaintextContainer, SealedContainer, Uuid};

/// Magic bytes `"AMBR"` (`0x414D4252`), offset 0.
pub const AMBRE_MAGIC: [u8; 4] = *b"AMBR";

/// The container version this build writes, and the only one it parses without
/// migrating first. Bumping it obliges a [`MIGRATIONS`] row and a fixture freeze
/// (`ambre-spec-fixtures.md`).
pub const AMBRE_VERSION: ContainerVersion = ContainerVersion(0x01);

/// Fixed header size; the payload starts at this offset.
pub const HEADER_LEN: usize = 22;

/// The bit of the version byte that partitions the two lineages: clear for a
/// plaintext container, set for a passphrase-sealed one.
///
/// The version byte changes, not a field beside it. A build that predates the
/// sealed lineage reads `0x80` as a container version it does not know, and
/// refuses the file. That is the whole of `DOCFILE-010`.
pub const SEALED_BIT: u8 = 0x80;

/// The sealed container version this build writes, and the only sealed layout it
/// parses. There is no sealed migration registry: see [`parse_sealed_header`].
pub const AMBRE_SEALED_VERSION: ContainerVersion = ContainerVersion(0x80);

/// Argon2id salt width in a sealed header.
pub const SALT_LEN: usize = 16;

/// The Argon2id salt of one sealed `.ambre`, fresh per file. Public in the
/// file, so not a secret; distinct from the account's
/// [`MasterKeySalt`](crate::MasterKeySalt), which no file ever carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileSalt([u8; SALT_LEN]);

impl FileSalt {
    /// Wraps the salt bytes a header carries or a CSPRNG drew.
    pub const fn from_array(bytes: [u8; SALT_LEN]) -> Self {
        Self(bytes)
    }

    /// The salt bytes, for the KDF and for the header.
    pub const fn as_array(&self) -> &[u8; SALT_LEN] {
        &self.0
    }
}

/// XChaCha20 nonce width in a sealed header. Mirrors `ambre_crypto::aead::NONCE_LEN`,
/// which this crate cannot name (`DOCFILE-006`). One crate up, where both sides
/// are in scope at once, `ambre_crypto::tests::the_sealed_header_widths_match_the_primitives`
/// pins the two together.
pub const SEAL_NONCE_LEN: usize = 24;

/// Fixed sealed-header size; the ciphertext starts at this offset.
pub const SEALED_HEADER_LEN: usize = 5 + SALT_LEN + SEAL_NONCE_LEN;

/// Width of the associated data the tag covers: the sealed header **up to but
/// excluding the nonce**.
///
/// The nonce is left out because the tag already covers it. An AEAD tag binds
/// its key *and* its nonce, so a flipped bit there fails the open either way.
/// It adds the rest of the context -- the magic and the lineage byte -- so no
/// other sealed layout reads this ciphertext.
pub const SEALED_AAD_LEN: usize = 5 + SALT_LEN;

// Field offsets inside the header. Named rather than inlined so a version bump
// that moves a field moves it in one place.
const MAGIC_LEN: usize = AMBRE_MAGIC.len();
const VERSION_AT: usize = 4;
const DOC_TYPE_AT: usize = 5;
const DOCUMENT_ID_AT: usize = 6;

// Sealed-header offsets. `SALT_AT` collides with the plaintext `DOC_TYPE_AT` by
// construction: the lineages diverge at offset 5, and the version byte before
// it names what follows.
const SALT_AT: usize = 5;
const SEAL_NONCE_AT: usize = SALT_AT + SALT_LEN;

/// The version byte of an `.ambre` container, offset 4.
///
/// Any byte reads as one: what the file carries is a fact about the file, and
/// the readers decide what to do with it. Its [`SEALED_BIT`] names the
/// lineage, so one value says both which lineage and which version
/// (`DOCFILE-010`). The order compares two versions of one lineage; the walk
/// of [`migrate_ambre`] reaches it only for a plaintext one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ContainerVersion(u8);

impl ContainerVersion {
    /// The version a file's byte at offset 4 names.
    #[must_use]
    pub const fn from_byte(byte: u8) -> Self {
        Self(byte)
    }

    /// The byte written at offset 4.
    #[must_use]
    pub const fn as_u8(self) -> u8 {
        self.0
    }

    /// Whether it names the sealed lineage: [`SEALED_BIT`] is set.
    #[must_use]
    pub const fn is_sealed(self) -> bool {
        self.0 & SEALED_BIT != 0
    }
}

impl fmt::Display for ContainerVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, f)
    }
}

impl fmt::LowerHex for ContainerVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::LowerHex::fmt(&self.0, f)
    }
}

/// A parsed `.ambre` container: the header fields plus the payload that followed
/// them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AmbreHeader {
    /// Container version the bytes carried. Always [`AMBRE_VERSION`] once the
    /// reader has returned: the reader migrates an older file before it gets
    /// here, and refuses a newer one.
    pub version: ContainerVersion,
    /// Which app wrote the file. The caller compares it against its own and
    /// re-dispatches on mismatch (dispatch rule) rather than coercing.
    pub doc_type: DocType,
    /// Document identity, UUID v7.
    pub document_id: DocumentId,
    /// Offset 22 onward: the raw Loro snapshot, uncompressed and unencrypted.
    pub payload: crate::SnapshotBytes,
}

/// Container-level failures. Fatal by nature: a header failure leaves no
/// document to produce.
#[derive(Error, Debug, Clone, PartialEq, Eq)]
pub enum DocFileError {
    /// Fewer than [`HEADER_LEN`] bytes -- not a container at all.
    #[error("truncated .ambre header: expected at least {HEADER_LEN} bytes")]
    TruncatedHeader,

    /// The first four bytes are not [`AMBRE_MAGIC`].
    #[error("not an .ambre file: bad magic")]
    InvalidMagic,

    /// `found < AMBRE_VERSION` -> recoverable via [`migrate_ambre`].
    /// `found > AMBRE_VERSION` -> unrecoverable, prompt the user to update.
    #[error("unsupported .ambre container version: {found}")]
    UnsupportedVersion { found: ContainerVersion },

    /// The version byte has [`SEALED_BIT`] set, so the file is a sealed
    /// container and there is no payload to read without a passphrase. The
    /// plaintext entry points raise it, because they cannot open one.
    ///
    /// Not a version failure, and not an error to report as one. This build
    /// reads a sealed file perfectly well once it holds the passphrase, and the
    /// caller prompts for it.
    #[error("this .ambre file is sealed with a passphrase")]
    SealedFile,

    /// The sealed parser met a plaintext container. The mirror of
    /// [`Self::SealedFile`], and out of reach for a caller that classifies
    /// first. Both exist so neither parser guesses at the other's bytes.
    #[error("this .ambre file is not sealed")]
    NotSealed,

    /// A sealed container whose sealed-layout version this build does not know.
    /// Distinct from [`Self::UnsupportedVersion`], which promises a migration
    /// for the older half. The sealed lineage has none: rewriting the envelope
    /// means deriving the key it wraps.
    #[error("unsupported sealed .ambre layout: {found:#04x}")]
    SealedVersionUnsupported { found: ContainerVersion },

    /// The `doc_type` byte names no app this build knows. Distinct from
    /// [`Self::WrongDocType`], a *valid* type belonging to another app: that one
    /// re-dispatches, this one cannot, so it carries no [`DocType`]. A client
    /// shipping a new document type reaches it, and so does corruption.
    #[error("unknown .ambre document type discriminant: {found}")]
    UnknownDocType { found: u8 },

    /// A valid `.ambre` for a different app. Carries the type so the caller can
    /// re-dispatch instead of failing the user.
    #[error("wrong document type for this importer: {found}")]
    WrongDocType { found: DocType },

    /// The header parsed but the Loro payload did not import. Raised by the
    /// app-side half of the pair, never by this module (I4: pure bytes here).
    #[error("the .ambre payload was rejected by the CRDT engine")]
    PayloadRejected,

    /// The header parsed and the payload is a valid document, but the *schema*
    /// version inside it is one this build refuses (`SCHEMA-008`) or cannot
    /// migrate. The app-side half of the pair raises it, like
    /// [`Self::PayloadRejected`].
    ///
    /// It carries the inner verdict rather than flattening it. The two versions
    /// guarding a stored document are independent on purpose: this one guards
    /// the twenty-two bytes around the payload, the other what is inside. A
    /// caller that reports "unsupported version" without saying which one sends
    /// the user looking in the wrong place.
    #[error("{0}")]
    PayloadSchema(#[from] crate::schema::SchemaError),
}

// -----------------------------------------------------------------------------
// The two lineages
// -----------------------------------------------------------------------------

/// Which lineage a file's version byte names, and at which version.
///
/// What a caller holding raw bytes reads *first*. It decides whether the next
/// step is a parse or a passphrase prompt, with no key, no filename and no
/// guess.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContainerKind {
    /// A plaintext container at this version. [`AMBRE_VERSION`] parses as it
    /// stands; an older one migrates first, and a newer one fails.
    Plaintext(ContainerVersion),
    /// A sealed container at this sealed-layout version. Needs a passphrase
    /// before any of the above applies.
    Sealed(ContainerVersion),
}

/// Read the magic and the version byte, and say which lineage the file belongs
/// to -- nothing more.
///
/// The cheapest question a reader can ask, and the first one it must. The two
/// lineages diverge at offset 5, so a caller that parses before it classifies
/// reads a salt as a document type. Five bytes are enough, so this answers for
/// a file too short to hold either header in full.
///
/// # Errors
///
/// [`DocFileError::TruncatedHeader`] when `data` is shorter than 5 bytes, and
/// [`DocFileError::InvalidMagic`] when the first four bytes are not
/// [`AMBRE_MAGIC`].
pub fn classify(data: &[u8]) -> Result<ContainerKind, DocFileError> {
    let Some(&byte) = data.get(VERSION_AT) else {
        return Err(DocFileError::TruncatedHeader);
    };
    if data.get(..AMBRE_MAGIC.len()) != Some(&AMBRE_MAGIC[..]) {
        return Err(DocFileError::InvalidMagic);
    }
    let version = ContainerVersion::from_byte(byte);
    if !version.is_sealed() {
        Ok(ContainerKind::Plaintext(version))
    } else {
        Ok(ContainerKind::Sealed(version))
    }
}

// -----------------------------------------------------------------------------
// Writing
// -----------------------------------------------------------------------------

/// Assemble a plaintext `.ambre`: the current-version header, then `payload`.
///
/// The writer rule lives here and nowhere else -- there is no parameter for the
/// version, because there is no backward-compatible write mode.
pub fn write_ambre_plaintext(
    doc_type: DocType,
    document_id: DocumentId,
    payload: &[u8],
) -> PlaintextContainer {
    let mut out = Vec::with_capacity(HEADER_LEN + payload.len());
    out.extend_from_slice(&AMBRE_MAGIC);
    out.push(AMBRE_VERSION.as_u8());
    out.push(doc_type.as_u8());
    out.extend_from_slice(document_id.as_bytes());
    debug_assert_eq!(out.len(), HEADER_LEN);
    out.extend_from_slice(payload);
    PlaintextContainer::new(out)
}

// -----------------------------------------------------------------------------
// Reading
// -----------------------------------------------------------------------------

/// Parse the header at the current version exactly, with no recovery: an older
/// file earns [`DocFileError::UnsupportedVersion`] rather than a migration.
///
/// The container half of the read path, for callers that inspect raw bytes -- a
/// header dump, a migration step validating its own output. The ordinary entry
/// points are [`read_ambre_plaintext`] and [`read_ambre_plaintext_as`].
///
/// # Errors
///
/// * [`DocFileError::TruncatedHeader`] below [`HEADER_LEN`] bytes.
/// * [`DocFileError::InvalidMagic`] on foreign bytes.
/// * [`DocFileError::SealedFile`] when [`SEALED_BIT`] is set.
/// * [`DocFileError::UnsupportedVersion`] on any other version.
/// * [`DocFileError::UnknownDocType`] when the type byte names no app this
///   build knows.
pub fn parse_header(data: &[u8]) -> Result<AmbreHeader, DocFileError> {
    let Some((head, payload)) = data.split_first_chunk::<HEADER_LEN>() else {
        return Err(DocFileError::TruncatedHeader);
    };
    if !head.starts_with(&AMBRE_MAGIC) {
        return Err(DocFileError::InvalidMagic);
    }
    // Version before doc_type, and the lineage before the version. A sealed
    // file holds a salt at offset 5, so reading it as a `DocType` is the
    // confusion the partition prevents. A future plaintext version may redefine
    // that byte too, so it is worth checking against *this* build's DocType set
    // only once the version matches.
    let version = ContainerVersion::from_byte(head[VERSION_AT]);
    if version.is_sealed() {
        return Err(DocFileError::SealedFile);
    }
    if version != AMBRE_VERSION {
        return Err(DocFileError::UnsupportedVersion { found: version });
    }
    let raw_type = head[DOC_TYPE_AT];
    let doc_type =
        DocType::from_u8(raw_type).ok_or(DocFileError::UnknownDocType { found: raw_type })?;
    // Infallible: the slice is exactly 16 bytes wide by construction.
    let document_id = Uuid::from_slice(&head[DOCUMENT_ID_AT..])
        .map(DocumentId::from_uuid)
        .map_err(|_| DocFileError::TruncatedHeader)?;
    Ok(AmbreHeader {
        version,
        doc_type,
        document_id,
        payload: crate::SnapshotBytes::new(payload.to_vec()),
    })
}

/// Read a plaintext `.ambre` of any [`DocType`], under the version-recovery
/// rule: this migrates an older container then re-parses it, and refuses a
/// newer one (I3).
///
/// Type-agnostic on purpose, for an "open this file" caller: it reads
/// `doc_type` off the header and routes to the app that claims it. An importer
/// that already knows which app it is wants [`read_ambre_plaintext_as`]
/// instead.
///
/// # Errors
///
/// Every error of [`parse_header`], and every error of [`migrate_ambre`] when
/// the container needs the migration walk. A version above [`AMBRE_VERSION`]
/// keeps its [`DocFileError::UnsupportedVersion`]: this build cannot read
/// forward.
pub fn read_ambre_plaintext(data: &[u8]) -> Result<AmbreHeader, DocFileError> {
    match parse_header(data) {
        // Older container: migrate forward, then parse the result. The migrated
        // buffer is a fresh allocation, so nothing here touches `data`.
        Err(DocFileError::UnsupportedVersion { found }) if found < AMBRE_VERSION => {
            parse_header(&migrate_ambre(found, data)?)
        }
        other => other,
    }
}

/// Read a plaintext `.ambre` that must belong to `expected`, applying both
/// normative rules: version recovery, then the dispatch rule.
///
/// The whole body of each app's `import_ambre_plaintext`. On
/// [`DocFileError::WrongDocType`] the caller routes the same bytes to the
/// importer named by `found` instead of coercing the document across apps.
///
/// # Errors
///
/// Every error of [`read_ambre_plaintext`], plus
/// [`DocFileError::WrongDocType`] when the header names an app other than
/// `expected`. That one carries the type it found, so the caller re-dispatches
/// the same bytes.
pub fn read_ambre_plaintext_as(
    expected: DocType,
    data: &[u8],
) -> Result<AmbreHeader, DocFileError> {
    let header = read_ambre_plaintext(data)?;
    if header.doc_type != expected {
        return Err(DocFileError::WrongDocType {
            found: header.doc_type,
        });
    }
    Ok(header)
}

// -----------------------------------------------------------------------------
// The sealed lineage
// -----------------------------------------------------------------------------

/// A parsed sealed `.ambre`: the envelope's public parameters, and the
/// ciphertext they belong to.
///
/// Every field here is public in the file and public in this type. A salt and a
/// nonce are not secrets. What *is* secret -- the document, its type, its
/// identity -- sits inside [`Self::ciphertext`] until something holding the
/// passphrase opens it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SealedAmbre {
    /// Sealed-layout version the bytes carried. Always [`AMBRE_SEALED_VERSION`]
    /// once the parser has returned.
    pub version: ContainerVersion,
    /// Argon2id salt, fresh per file.
    pub salt: FileSalt,
    /// AEAD nonce, fresh per file.
    pub nonce: Nonce,
    /// Offset [`SEALED_HEADER_LEN`] onward: the sealed bytes, authentication tag
    /// included. Opens to a whole plaintext container.
    pub ciphertext: Ciphertext,
}

impl SealedAmbre {
    /// The associated data the seal covered: this file's own header, up to the
    /// nonce.
    ///
    /// A prefix of the header by construction, not by convention: the writer
    /// and this method assemble from the same code, and cannot drift apart.
    pub fn aad(&self) -> crate::Aad {
        sealed_aad_at(self.version, &self.salt)
    }
}

/// Assemble the authenticated prefix of a sealed header. The one place that
/// writes the sealed layout down, called by [`sealed_aad`] and by
/// [`SealedAmbre::aad`].
fn sealed_aad_at(version: ContainerVersion, salt: &FileSalt) -> crate::Aad {
    let mut out = [0u8; SEALED_AAD_LEN];
    out[..MAGIC_LEN].copy_from_slice(&AMBRE_MAGIC);
    out[VERSION_AT] = version.as_u8();
    out[SALT_AT..].copy_from_slice(salt.as_array());
    crate::Aad::of(&out)
}

/// The associated data a seal covers, for a file about to go out under `salt`.
///
/// The write-side counterpart of [`SealedAmbre::aad`]. The sealer needs the
/// associated data *before* it has a ciphertext to put in a [`SealedAmbre`], so
/// it asks here. It hands the same salt to [`write_ambre_sealed`] afterwards,
/// along with the nonce the cipher drew.
pub fn sealed_aad(salt: &FileSalt) -> crate::Aad {
    sealed_aad_at(AMBRE_SEALED_VERSION, salt)
}

/// Assemble a sealed `.ambre`: the current sealed header, then `ciphertext`.
///
/// The writer rule of the sealed lineage, and it takes no version either: this
/// emits [`AMBRE_SEALED_VERSION`] or nothing at all.
pub fn write_ambre_sealed(
    salt: &FileSalt,
    nonce: &Nonce,
    ciphertext: &Ciphertext,
) -> SealedContainer {
    let mut out = Vec::with_capacity(SEALED_HEADER_LEN + ciphertext.len());
    out.extend_from_slice(sealed_aad(salt).as_bytes());
    out.extend_from_slice(&nonce.0);
    debug_assert_eq!(out.len(), SEALED_HEADER_LEN);
    out.extend_from_slice(&ciphertext.0);
    SealedContainer::new(out)
}

/// Parse a sealed `.ambre` header, at the current sealed layout exactly.
///
/// **There is no sealed migration walk, and this is where that decision shows.**
/// A plaintext container migrates because its bytes are readable; a sealed one
/// is not, so rewriting its envelope means deriving the key inside it. That is
/// a re-seal, under a passphrase the user must type, not a read-side recovery.
/// The document inside migrates once opened, through the ordinary reader. So
/// this refuses an unknown sealed layout, and the refusal names itself
/// ([`DocFileError::SealedVersionUnsupported`]) rather than promising a
/// migration that cannot exist.
///
/// A ciphertext too short for an authentication tag still parses: this module
/// owns no tag width (`DOCFILE-006`), and the AEAD refuses it one layer up.
///
/// # Errors
///
/// Every error of [`classify`], plus [`DocFileError::NotSealed`] on a plaintext
/// container, [`DocFileError::SealedVersionUnsupported`] on an unknown layout,
/// and [`DocFileError::TruncatedHeader`] below [`SEALED_HEADER_LEN`].
pub fn parse_sealed_header(data: &[u8]) -> Result<SealedAmbre, DocFileError> {
    let version = match classify(data)? {
        ContainerKind::Plaintext(_) => return Err(DocFileError::NotSealed),
        ContainerKind::Sealed(version) => version,
    };
    if version != AMBRE_SEALED_VERSION {
        return Err(DocFileError::SealedVersionUnsupported { found: version });
    }
    let Some((head, ciphertext)) = data.split_first_chunk::<SEALED_HEADER_LEN>() else {
        return Err(DocFileError::TruncatedHeader);
    };
    let salt = head
        .get(SALT_AT..SEAL_NONCE_AT)
        .and_then(|s| <[u8; SALT_LEN]>::try_from(s).ok())
        .map(FileSalt)
        .ok_or(DocFileError::TruncatedHeader)?;
    let nonce = head
        .get(SEAL_NONCE_AT..)
        .and_then(|s| <[u8; SEAL_NONCE_LEN]>::try_from(s).ok())
        .map(Nonce)
        .ok_or(DocFileError::TruncatedHeader)?;
    Ok(SealedAmbre {
        version,
        salt,
        nonce,
        ciphertext: Ciphertext(ciphertext.to_vec()),
    })
}

// -----------------------------------------------------------------------------
// Version registry and migration
// -----------------------------------------------------------------------------

/// One forward step of the version registry: `from` -> `to`, whole file in,
/// whole file out.
///
/// A step owns the *entire* transformation, header byte included: it returns
/// bytes that parse at `to`. Payload contents are not its business -- the
/// `meta` / `schema_version` register versions the CRDT schema inside the
/// snapshot (`ambre-spec-schema-version.md`, `ARCH P8`).
struct Migration {
    from: ContainerVersion,
    to: ContainerVersion,
    apply: fn(&[u8]) -> Result<Vec<u8>, DocFileError>,
}

/// The version registry: one row per container version bump, appended
/// never rewritten.
///
/// Empty at `0x01` -- nothing precedes the initial layout. The synthetic-step
/// test below drives the walk that consumes it ([`migrate_steps`]), so the path
/// is not dead code when the first bump lands.
const MIGRATIONS: &[Migration] = &[];

/// Migrate a whole `.ambre` file forward from `from` to [`AMBRE_VERSION`],
/// one registry step at a time.
///
/// Returns fresh bytes; the input is never mutated (I3). `from` at the current
/// version is the identity, and anything newer than this build draws a refusal
/// rather than a guess.
///
/// # Errors
///
/// [`DocFileError::SealedFile`] when `from` carries [`SEALED_BIT`], and
/// [`DocFileError::UnsupportedVersion`] when the registry holds no step out of
/// a version the walk reaches. A step of the registry raises its own error
/// unchanged.
pub fn migrate_ambre(from: ContainerVersion, bytes: &[u8]) -> Result<Vec<u8>, DocFileError> {
    // The walk would refuse a sealed version anyway, since every one of them
    // sits above `AMBRE_VERSION`. But it would call it an unknown *plaintext*
    // version, the one message that sends the user to the changelog instead of
    // to their passphrase.
    if from.is_sealed() {
        return Err(DocFileError::SealedFile);
    }
    migrate_steps(MIGRATIONS, from, AMBRE_VERSION, bytes)
}

/// The registry walk, parameterized by the step table: the tests drive it with
/// a synthetic registry while the shipping one is still empty.
fn migrate_steps(
    steps: &[Migration],
    from: ContainerVersion,
    target: ContainerVersion,
    bytes: &[u8],
) -> Result<Vec<u8>, DocFileError> {
    if from > target {
        return Err(DocFileError::UnsupportedVersion { found: from });
    }
    let mut current = from;
    let mut buf = bytes.to_vec();
    while current < target {
        // `to > current` is a requirement, not an expectation: it is what makes
        // the walk terminate, whatever a future registry row looks like.
        let step = steps
            .iter()
            .find(|step| step.from == current && step.to > current)
            .ok_or(DocFileError::UnsupportedVersion { found: current })?;
        buf = (step.apply)(&buf)?;
        current = step.to;
    }
    Ok(buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    const DOC_ID: [u8; 16] = [
        0x01, 0x8f, 0x2a, 0x3b, 0x7c, 0x4d, 0x70, 0x00, 0x80, 0x00, 0xde, 0xad, 0xbe, 0xef, 0x00,
        0x01,
    ];

    fn doc_id() -> DocumentId {
        DocumentId::from_bytes(DOC_ID)
    }

    fn sample(doc_type: DocType) -> Vec<u8> {
        write_ambre_plaintext(doc_type, doc_id(), b"snapshot-bytes").into_bytes()
    }

    #[test]
    fn header_layout_is_the_spec_layout() {
        let bytes = sample(DocType::Toile);
        assert_eq!(&bytes[..4], b"AMBR");
        assert_eq!(bytes[VERSION_AT], AMBRE_VERSION.as_u8());
        assert_eq!(bytes[DOC_TYPE_AT], DocType::Toile.as_u8());
        assert_eq!(&bytes[DOCUMENT_ID_AT..HEADER_LEN], &DOC_ID);
        assert_eq!(&bytes[HEADER_LEN..], b"snapshot-bytes");
        assert_eq!(bytes.len(), HEADER_LEN + b"snapshot-bytes".len());
    }

    #[test]
    fn round_trip_is_the_identity_on_the_container() {
        let bytes = sample(DocType::Trame);
        let header = read_ambre_plaintext(&bytes).expect("valid container");
        assert_eq!(header.version, AMBRE_VERSION);
        assert_eq!(header.doc_type, DocType::Trame);
        assert_eq!(header.document_id, doc_id());
        assert_eq!(header.payload, b"snapshot-bytes");
        // Re-writing the parsed parts reproduces the file byte for byte (I1 at
        // the container layer).
        assert_eq!(
            write_ambre_plaintext(
                header.doc_type,
                header.document_id,
                header.payload.as_bytes()
            ),
            bytes
        );
    }

    #[test]
    fn an_empty_payload_is_a_valid_container() {
        // A header failure and an empty document are different conditions: the
        // container reports the second faithfully and lets the engine judge it.
        let bytes = write_ambre_plaintext(DocType::Trame, doc_id(), &[]);
        let header = read_ambre_plaintext(bytes.as_bytes()).expect("valid container");
        assert!(header.payload.is_empty());
    }

    #[test]
    fn truncated_header_is_rejected_at_every_length() {
        let bytes = sample(DocType::Trame);
        for len in 0..HEADER_LEN {
            assert_eq!(
                parse_header(&bytes[..len]),
                Err(DocFileError::TruncatedHeader),
                "a {len}-byte file must not parse"
            );
        }
        // Exactly HEADER_LEN parses: an empty payload is not a truncation.
        assert!(parse_header(&bytes[..HEADER_LEN]).is_ok());
    }

    #[test]
    fn bad_magic_is_rejected() {
        let mut bytes = sample(DocType::Trame);
        bytes[0] = b'X';
        assert_eq!(
            read_ambre_plaintext(&bytes),
            Err(DocFileError::InvalidMagic)
        );
    }

    #[test]
    fn a_newer_version_is_refused_and_an_older_one_is_attempted() {
        let mut newer = sample(DocType::Trame);
        newer[VERSION_AT] = AMBRE_VERSION.as_u8() + 1;
        assert_eq!(
            read_ambre_plaintext(&newer),
            Err(DocFileError::UnsupportedVersion {
                found: ContainerVersion(AMBRE_VERSION.as_u8() + 1)
            }),
            "a newer container is never read best-effort"
        );

        // Older: the recovery rule sends it through `migrate_ambre`, which has no
        // step for version 0 (nothing precedes 0x01) and says so rather than
        // guessing.
        let mut older = sample(DocType::Trame);
        older[VERSION_AT] = 0;
        assert_eq!(
            read_ambre_plaintext(&older),
            Err(DocFileError::UnsupportedVersion {
                found: ContainerVersion(0)
            })
        );
    }

    #[test]
    fn an_unknown_doc_type_byte_is_not_a_dispatch_target() {
        let mut bytes = sample(DocType::Trame);
        bytes[DOC_TYPE_AT] = 9;
        assert_eq!(
            read_ambre_plaintext(&bytes),
            Err(DocFileError::UnknownDocType { found: 9 })
        );
    }

    #[test]
    fn dispatch_reports_the_type_that_wrote_the_file() {
        let toile = sample(DocType::Toile);
        assert_eq!(
            read_ambre_plaintext_as(DocType::Trame, &toile),
            Err(DocFileError::WrongDocType {
                found: DocType::Toile
            }),
            "the caller must be able to re-dispatch, so the type is carried"
        );
        // The importer they name accepts the same bytes, unchanged.
        let header = read_ambre_plaintext_as(DocType::Toile, &toile).expect("right importer");
        assert_eq!(header.payload, b"snapshot-bytes");
    }

    #[test]
    fn every_error_path_leaves_the_input_untouched() {
        // I3: the reader borrows; nothing it does can reach the caller's buffer.
        let cases: Vec<Vec<u8>> = vec![
            Vec::new(),
            sample(DocType::Trame)[..10].to_vec(),
            {
                let mut b = sample(DocType::Trame);
                b[0] = b'X';
                b
            },
            {
                let mut b = sample(DocType::Trame);
                b[VERSION_AT] = AMBRE_VERSION.as_u8() + 1;
                b
            },
            {
                let mut b = sample(DocType::Trame);
                b[VERSION_AT] = 0;
                b
            },
            {
                let mut b = sample(DocType::Trame);
                b[DOC_TYPE_AT] = 9;
                b
            },
            sample(DocType::Toile),
            sealed_sample(),
            {
                let mut b = sealed_sample();
                b[VERSION_AT] = 0x81;
                b
            },
            sealed_sample()[..20].to_vec(),
        ];
        for bytes in cases {
            let before = bytes.clone();
            let _ = read_ambre_plaintext(&bytes);
            let _ = read_ambre_plaintext_as(DocType::Trame, &bytes);
            let _ = migrate_ambre(ContainerVersion(0), &bytes);
            let _ = classify(&bytes);
            let _ = parse_sealed_header(&bytes);
            assert_eq!(bytes, before, "an error path rewrote its input");
        }
    }

    #[test]
    fn migrating_from_the_current_version_is_the_identity() {
        let bytes = sample(DocType::Trame);
        assert_eq!(migrate_ambre(AMBRE_VERSION, &bytes).unwrap(), bytes);
    }

    #[test]
    fn migrating_from_a_newer_version_is_refused() {
        let bytes = sample(DocType::Trame);
        assert_eq!(
            migrate_ambre(ContainerVersion(AMBRE_VERSION.as_u8() + 1), &bytes),
            Err(DocFileError::UnsupportedVersion {
                found: ContainerVersion(AMBRE_VERSION.as_u8() + 1)
            })
        );
    }

    // -- the synthetic registry ------------------------------------------
    //
    // The shipping registry is empty at 0x01, so the walk above has no real step
    // to run. These tests drive it with a two-row synthetic registry. The day a
    // real bump lands, the harness it plugs into is already proven, and adding
    // the row is the whole change.

    /// `0x01` -> `0x02`: rewrite the version byte, leave the payload alone.
    fn bump_to_2(bytes: &[u8]) -> Result<Vec<u8>, DocFileError> {
        let mut out = bytes.to_vec();
        *out.get_mut(VERSION_AT)
            .ok_or(DocFileError::TruncatedHeader)? = 2;
        Ok(out)
    }

    /// `0x02` -> `0x03`: same shape, plus a payload edit, so the multi-step walk
    /// must chain outputs rather than re-read the original.
    fn bump_to_3(bytes: &[u8]) -> Result<Vec<u8>, DocFileError> {
        let mut out = bytes.to_vec();
        *out.get_mut(VERSION_AT)
            .ok_or(DocFileError::TruncatedHeader)? = 3;
        out.extend_from_slice(b"+v3");
        Ok(out)
    }

    const SYNTHETIC: &[Migration] = &[
        Migration {
            from: ContainerVersion(1),
            to: ContainerVersion(2),
            apply: bump_to_2,
        },
        Migration {
            from: ContainerVersion(2),
            to: ContainerVersion(3),
            apply: bump_to_3,
        },
    ];

    #[test]
    fn the_walk_applies_one_step_per_version() {
        let bytes = sample(DocType::Trame);
        let out = migrate_steps(SYNTHETIC, ContainerVersion(1), ContainerVersion(2), &bytes)
            .expect("one step");
        assert_eq!(out[VERSION_AT], 2);
        assert_eq!(&out[HEADER_LEN..], b"snapshot-bytes");
    }

    #[test]
    fn the_walk_chains_steps_in_order() {
        let bytes = sample(DocType::Trame);
        let out = migrate_steps(SYNTHETIC, ContainerVersion(1), ContainerVersion(3), &bytes)
            .expect("two steps");
        assert_eq!(out[VERSION_AT], 3);
        assert_eq!(&out[HEADER_LEN..], b"snapshot-bytes+v3");
    }

    #[test]
    fn a_gap_in_the_registry_stops_the_walk_at_the_missing_row() {
        let bytes = sample(DocType::Trame);
        // No row leaves 3, so a 3 -> 4 target reports the version it got stuck on
        // rather than silently returning a half-migrated file.
        assert_eq!(
            migrate_steps(SYNTHETIC, ContainerVersion(3), ContainerVersion(4), &bytes),
            Err(DocFileError::UnsupportedVersion {
                found: ContainerVersion(3)
            })
        );
    }

    #[test]
    fn a_failing_step_fails_the_whole_migration() {
        let truncated = sample(DocType::Trame)[..2].to_vec();
        assert_eq!(
            migrate_steps(
                SYNTHETIC,
                ContainerVersion(1),
                ContainerVersion(2),
                &truncated
            ),
            Err(DocFileError::TruncatedHeader)
        );
    }

    // -- the sealed lineage ----------------------------------------------

    const SALT: FileSalt = FileSalt::from_array([0xA5; SALT_LEN]);
    const NONCE: Nonce = Nonce([0x5A; SEAL_NONCE_LEN]);

    fn sealed_sample() -> Vec<u8> {
        write_ambre_sealed(&SALT, &NONCE, &Ciphertext(b"ciphertext-bytes".to_vec())).into_bytes()
    }

    #[test]
    fn the_sealed_layout_is_the_spec_layout() {
        let bytes = sealed_sample();
        assert_eq!(&bytes[..4], b"AMBR");
        assert_eq!(bytes[VERSION_AT], AMBRE_SEALED_VERSION.as_u8());
        assert_eq!(&bytes[SALT_AT..SEAL_NONCE_AT], SALT.as_array());
        assert_eq!(&bytes[SEAL_NONCE_AT..SEALED_HEADER_LEN], &NONCE.0);
        assert_eq!(&bytes[SEALED_HEADER_LEN..], b"ciphertext-bytes");
        assert_eq!(SEALED_HEADER_LEN, 45);
        assert_eq!(bytes.len(), SEALED_HEADER_LEN + b"ciphertext-bytes".len());
    }

    #[test]
    fn the_associated_data_is_the_header_up_to_the_nonce() {
        // The sealer computes the associated data before it has a ciphertext;
        // the opener recovers it from the parsed envelope. Both must be the
        // bytes on disk, or a file seals against one and opens against another.
        let bytes = sealed_sample();
        let envelope = parse_sealed_header(&bytes).expect("valid sealed container");
        assert_eq!(sealed_aad(&SALT).as_bytes(), &bytes[..SEALED_AAD_LEN]);
        assert_eq!(envelope.aad(), sealed_aad(&SALT));
        // The nonce is deliberately outside it: the AEAD authenticates the
        // nonce on its own, and naming it here would add nothing.
        assert_eq!(SEALED_AAD_LEN, SEALED_HEADER_LEN - SEAL_NONCE_LEN);
    }

    #[test]
    fn a_sealed_round_trip_recovers_the_envelope() {
        let envelope = parse_sealed_header(&sealed_sample()).expect("valid sealed container");
        assert_eq!(envelope.version, AMBRE_SEALED_VERSION);
        assert_eq!(envelope.salt, SALT);
        assert_eq!(envelope.nonce, NONCE);
        assert_eq!(envelope.ciphertext.0, b"ciphertext-bytes");
        assert_eq!(
            write_ambre_sealed(&envelope.salt, &envelope.nonce, &envelope.ciphertext),
            sealed_sample()
        );
    }

    #[test]
    fn the_version_byte_alone_tells_the_lineages_apart() {
        // `DOCFILE-010`: the header, and only the header, says which lineage a
        // file belongs to -- five bytes of it, before any field that moved.
        assert_eq!(
            classify(&sample(DocType::Trame)),
            Ok(ContainerKind::Plaintext(AMBRE_VERSION))
        );
        assert_eq!(
            classify(&sealed_sample()),
            Ok(ContainerKind::Sealed(AMBRE_SEALED_VERSION))
        );
        // Five bytes is the whole input it needs: a file too short to hold
        // either header still classifies.
        assert_eq!(
            classify(&sealed_sample()[..5]),
            Ok(ContainerKind::Sealed(AMBRE_SEALED_VERSION))
        );
        assert_eq!(
            classify(&sealed_sample()[..4]),
            Err(DocFileError::TruncatedHeader)
        );
        let mut bad = sealed_sample();
        bad[0] = b'X';
        assert_eq!(classify(&bad), Err(DocFileError::InvalidMagic));
    }

    #[test]
    fn the_plaintext_reader_refuses_a_sealed_file_by_name() {
        // Not `UnsupportedVersion`: this build reads a sealed file perfectly
        // well once it has the passphrase. Asking the user to update their
        // client is the wrong message.
        let bytes = sealed_sample();
        assert_eq!(parse_header(&bytes), Err(DocFileError::SealedFile));
        assert_eq!(read_ambre_plaintext(&bytes), Err(DocFileError::SealedFile));
        assert_eq!(
            read_ambre_plaintext_as(DocType::Trame, &bytes),
            Err(DocFileError::SealedFile)
        );
        // Every sealed version, not just the one this build writes: the
        // partition is the bit, not the value.
        let mut future = sealed_sample();
        future[VERSION_AT] = 0xFF;
        assert_eq!(read_ambre_plaintext(&future), Err(DocFileError::SealedFile));
    }

    #[test]
    fn the_sealed_parser_refuses_a_plaintext_file_by_name() {
        assert_eq!(
            parse_sealed_header(&sample(DocType::Trame)),
            Err(DocFileError::NotSealed)
        );
    }

    #[test]
    fn an_unknown_sealed_layout_is_refused_without_a_migration_promise() {
        let mut bytes = sealed_sample();
        bytes[VERSION_AT] = 0x81;
        assert_eq!(
            parse_sealed_header(&bytes),
            Err(DocFileError::SealedVersionUnsupported {
                found: ContainerVersion(0x81)
            }),
            "a sealed layout has no migration, so the refusal must not promise one"
        );
    }

    #[test]
    fn migrating_a_sealed_version_says_it_is_sealed() {
        assert_eq!(
            migrate_ambre(AMBRE_SEALED_VERSION, &sealed_sample()),
            Err(DocFileError::SealedFile)
        );
    }

    #[test]
    fn a_truncated_sealed_header_is_rejected_at_every_length() {
        let bytes = sealed_sample();
        for len in 0..SEALED_HEADER_LEN {
            assert!(
                matches!(
                    parse_sealed_header(&bytes[..len]),
                    Err(DocFileError::TruncatedHeader)
                ),
                "a {len}-byte sealed file must not parse"
            );
        }
        assert!(parse_sealed_header(&bytes[..SEALED_HEADER_LEN]).is_ok());
    }

    #[test]
    fn an_empty_ciphertext_parses_and_leaves_the_verdict_to_the_aead() {
        // This module owns no tag width (`DOCFILE-006`), so it cannot tell a
        // ciphertext too short to be one from a valid short document. It reports
        // the envelope faithfully; the open fails one layer up.
        let envelope = parse_sealed_header(
            write_ambre_sealed(&SALT, &NONCE, &Ciphertext(Vec::new())).as_bytes(),
        )
        .expect("a header with no ciphertext is still a header");
        assert!(envelope.ciphertext.is_empty());
    }
}
