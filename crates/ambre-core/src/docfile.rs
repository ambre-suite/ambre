//! The `.ambre` container -- the shared header of the native document file
//! (`ambre-spec-docfile.md`).
//!
//! This module owns the *container* half of the format and nothing else: the
//! fixed 22-byte header, the version registry and its forward migrations, the
//! error taxonomy, and the two normative rules that make a file readable by a
//! client that did not write it (dispatch, version recovery). The *payload* half
//! -- producing and consuming a `LoroDoc` snapshot -- belongs to each app's
//! interop module, which calls in here rather than re-deriving the layout (I5:
//! one container, every app).
//!
//! Layout, offsets fixed:
//!
//! ```text
//!   0   4   magic       0x414D4252 ("AMBR")
//!   4   1   version     container version, current 0x01
//!   5   1   doc_type    DocType as u8
//!   6  16   document_id UUID v7
//!  22   ..  payload     LoroDoc::export_snapshot(), plaintext
//! ```
//!
//! Four invariants shape the API:
//! * **I2 self-describing** -- `version` and `doc_type` precede any app-specific
//!   parsing, so a reader identifies a file without guessing from its extension.
//! * **I3 non-destructive** -- every entry point takes `&[u8]` and returns fresh
//!   bytes. No path here mutates its input, under any error.
//! * **I4 no cryptography** -- pure byte handling. Protection (the future
//!   passphrase-sealed variant) is a separate stage and a separate version.
//! * **Writer rule** -- writers only ever emit [`AMBRE_VERSION`]. Compatibility
//!   is a read-side obligation: one writer path, N reader paths.

use thiserror::Error;

use crate::{DocType, Uuid};

/// Magic bytes `"AMBR"` (`0x414D4252`), offset 0.
pub const AMBRE_MAGIC: [u8; 4] = *b"AMBR";

/// The container version this build writes, and the only one it parses without
/// migrating first. Bumping it obliges a [`MIGRATIONS`] row and a fixture freeze
/// (`ambre-spec-fixtures.md`).
pub const AMBRE_VERSION: u8 = 0x01;

/// Fixed header size; the payload starts at this offset.
pub const HEADER_LEN: usize = 22;

// Field offsets inside the header. Named rather than inlined so a version bump
// that moves a field moves it in one place.
const VERSION_AT: usize = 4;
const DOC_TYPE_AT: usize = 5;
const DOCUMENT_ID_AT: usize = 6;

/// A parsed `.ambre` container: the header fields plus the payload that followed
/// them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AmbreHeader {
    /// Container version the bytes carried. Always [`AMBRE_VERSION`] once the
    /// reader has returned: an older file is migrated before it gets here, a
    /// newer one is refused.
    pub version: u8,
    /// Which app wrote the file. The caller compares it against its own and
    /// re-dispatches on mismatch (dispatch rule) rather than coercing.
    pub doc_type: DocType,
    /// Document identity, UUID v7.
    pub document_id: Uuid,
    /// Offset 22 onward: the raw Loro snapshot, uncompressed and unencrypted.
    pub payload: Vec<u8>,
}

/// Container-level failures. Fatal by nature: a header failure means no document
/// can be produced.
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
    UnsupportedVersion { found: u8 },

    /// The `doc_type` byte names no app this build knows. Distinct from
    /// [`Self::WrongDocType`], which is a *valid* type belonging to another app:
    /// that one is re-dispatchable, this one is not, so it cannot carry a
    /// [`DocType`]. Reachable from a file written by a client that shipped a new
    /// document type, or from corruption.
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
    /// version inside it is one this build will not read (`SCHEMA-008`) or
    /// cannot migrate. Raised by the app-side half of the pair, like
    /// [`Self::PayloadRejected`].
    ///
    /// Carries the inner verdict rather than flattening it: the two versions
    /// guarding a stored document are independent on purpose -- this one guards
    /// the twenty-two bytes around the payload, the other guards what is inside
    /// -- and a caller that reports "unsupported version" without saying which
    /// one sends the user looking in the wrong place.
    #[error("{0}")]
    PayloadSchema(#[from] crate::schema::SchemaError),
}

// -----------------------------------------------------------------------------
// Writing
// -----------------------------------------------------------------------------

/// Assemble a plaintext `.ambre`: the current-version header, then `payload`.
///
/// The writer rule lives here and nowhere else -- there is no parameter for the
/// version, because there is no backward-compatible write mode.
pub fn write_ambre_plaintext(doc_type: DocType, document_id: Uuid, payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(HEADER_LEN + payload.len());
    out.extend_from_slice(&AMBRE_MAGIC);
    out.push(AMBRE_VERSION);
    out.push(doc_type.as_u8());
    out.extend_from_slice(document_id.as_bytes());
    debug_assert_eq!(out.len(), HEADER_LEN);
    out.extend_from_slice(payload);
    out
}

// -----------------------------------------------------------------------------
// Reading
// -----------------------------------------------------------------------------

/// Parse the header at the current version exactly, with no recovery: an older
/// file is reported as [`DocFileError::UnsupportedVersion`] rather than
/// migrated.
///
/// The container half of the read path, exposed for callers that want to inspect
/// raw bytes (a header dump, a migration step validating its own output). The
/// ordinary entry points are [`read_ambre_plaintext`] and
/// [`read_ambre_plaintext_as`].
pub fn parse_header(data: &[u8]) -> Result<AmbreHeader, DocFileError> {
    if data.len() < HEADER_LEN {
        return Err(DocFileError::TruncatedHeader);
    }
    if data[..AMBRE_MAGIC.len()] != AMBRE_MAGIC {
        return Err(DocFileError::InvalidMagic);
    }
    // Version before doc_type: a future version is free to redefine the byte at
    // offset 5, so validating it against *this* build's DocType set only makes
    // sense once the version matches.
    let version = data[VERSION_AT];
    if version != AMBRE_VERSION {
        return Err(DocFileError::UnsupportedVersion { found: version });
    }
    let raw_type = data[DOC_TYPE_AT];
    let doc_type =
        DocType::from_u8(raw_type).ok_or(DocFileError::UnknownDocType { found: raw_type })?;
    // Infallible: the slice is exactly 16 bytes wide by construction.
    let document_id = Uuid::from_slice(&data[DOCUMENT_ID_AT..HEADER_LEN])
        .map_err(|_| DocFileError::TruncatedHeader)?;
    Ok(AmbreHeader {
        version,
        doc_type,
        document_id,
        payload: data[HEADER_LEN..].to_vec(),
    })
}

/// Read a plaintext `.ambre` of any [`DocType`], applying the version-recovery
/// rule: an older container is migrated then re-parsed, a newer one is
/// refused with the file left untouched (I3).
///
/// Type-agnostic on purpose -- this is what an "open this file" caller wants:
/// it reads `doc_type` off the returned header and routes to the app that claims
/// it. An importer that already knows which app it is wants
/// [`read_ambre_plaintext_as`] instead.
pub fn read_ambre_plaintext(data: &[u8]) -> Result<AmbreHeader, DocFileError> {
    match parse_header(data) {
        // Older container: migrate forward, then parse the result. The migrated
        // buffer is a fresh allocation, so `data` is untouched either way.
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
// Version registry and migration
// -----------------------------------------------------------------------------

/// One forward step of the version registry: `from` -> `to`, whole file in,
/// whole file out.
///
/// A step owns the *entire* transformation, header byte included: it returns
/// bytes that parse at `to`. Payload contents are not its business -- the CRDT
/// schema inside the snapshot is versioned separately by the `meta`
/// / `schema_version` register (`ambre-spec-schema-version.md`, `ARCH P8`).
struct Migration {
    from: u8,
    to: u8,
    apply: fn(&[u8]) -> Result<Vec<u8>, DocFileError>,
}

/// The version registry: one row per container version bump, appended
/// never rewritten.
///
/// Empty at `0x01` -- nothing precedes the initial layout. The walk that
/// consumes it ([`migrate_steps`]) is exercised by the synthetic-step test below
/// so the path is not dead code on the day the first bump lands.
const MIGRATIONS: &[Migration] = &[];

/// Migrate a whole `.ambre` file forward from `from` to [`AMBRE_VERSION`],
/// one registry step at a time.
///
/// Returns fresh bytes; the input is never mutated (I3). `from` at the current
/// version is the identity, and a `from` newer than this build is refused rather
/// than guessed at.
pub fn migrate_ambre(from: u8, bytes: &[u8]) -> Result<Vec<u8>, DocFileError> {
    migrate_steps(MIGRATIONS, from, AMBRE_VERSION, bytes)
}

/// The registry walk, parameterized by the step table so the harness can be
/// driven by a synthetic registry in tests while the shipping one is still
/// empty.
fn migrate_steps(
    steps: &[Migration],
    from: u8,
    target: u8,
    bytes: &[u8],
) -> Result<Vec<u8>, DocFileError> {
    if from > target {
        return Err(DocFileError::UnsupportedVersion { found: from });
    }
    let mut current = from;
    let mut buf = bytes.to_vec();
    while current < target {
        // `to > current` is required, not merely expected: it is what makes the
        // walk terminate, whatever a future registry row looks like.
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

    fn doc_id() -> Uuid {
        Uuid::from_bytes(DOC_ID)
    }

    fn sample(doc_type: DocType) -> Vec<u8> {
        write_ambre_plaintext(doc_type, doc_id(), b"snapshot-bytes")
    }

    #[test]
    fn header_layout_is_the_spec_layout() {
        let bytes = sample(DocType::Toile);
        assert_eq!(&bytes[..4], b"AMBR");
        assert_eq!(bytes[VERSION_AT], AMBRE_VERSION);
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
            write_ambre_plaintext(header.doc_type, header.document_id, &header.payload),
            bytes
        );
    }

    #[test]
    fn an_empty_payload_is_a_valid_container() {
        // A header failure and an empty document are different conditions: the
        // container reports the second faithfully and lets the engine judge it.
        let bytes = write_ambre_plaintext(DocType::Trame, doc_id(), &[]);
        let header = read_ambre_plaintext(&bytes).expect("valid container");
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
        newer[VERSION_AT] = AMBRE_VERSION + 1;
        assert_eq!(
            read_ambre_plaintext(&newer),
            Err(DocFileError::UnsupportedVersion {
                found: AMBRE_VERSION + 1
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
            Err(DocFileError::UnsupportedVersion { found: 0 })
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
        // The same bytes, unchanged, are accepted by the importer they name.
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
                b[VERSION_AT] = AMBRE_VERSION + 1;
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
        ];
        for bytes in cases {
            let before = bytes.clone();
            let _ = read_ambre_plaintext(&bytes);
            let _ = read_ambre_plaintext_as(DocType::Trame, &bytes);
            let _ = migrate_ambre(0, &bytes);
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
            migrate_ambre(AMBRE_VERSION + 1, &bytes),
            Err(DocFileError::UnsupportedVersion {
                found: AMBRE_VERSION + 1
            })
        );
    }

    // -- the synthetic registry ------------------------------------------
    //
    // The shipping registry is empty at 0x01, so the walk above has no real step
    // to run. These tests drive it with a two-row synthetic registry: the day a
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
    /// is seen to chain outputs rather than re-reading the original.
    fn bump_to_3(bytes: &[u8]) -> Result<Vec<u8>, DocFileError> {
        let mut out = bytes.to_vec();
        *out.get_mut(VERSION_AT)
            .ok_or(DocFileError::TruncatedHeader)? = 3;
        out.extend_from_slice(b"+v3");
        Ok(out)
    }

    const SYNTHETIC: &[Migration] = &[
        Migration {
            from: 1,
            to: 2,
            apply: bump_to_2,
        },
        Migration {
            from: 2,
            to: 3,
            apply: bump_to_3,
        },
    ];

    #[test]
    fn the_walk_applies_one_step_per_version() {
        let bytes = sample(DocType::Trame);
        let out = migrate_steps(SYNTHETIC, 1, 2, &bytes).expect("one step");
        assert_eq!(out[VERSION_AT], 2);
        assert_eq!(&out[HEADER_LEN..], b"snapshot-bytes");
    }

    #[test]
    fn the_walk_chains_steps_in_order() {
        let bytes = sample(DocType::Trame);
        let out = migrate_steps(SYNTHETIC, 1, 3, &bytes).expect("two steps");
        assert_eq!(out[VERSION_AT], 3);
        assert_eq!(&out[HEADER_LEN..], b"snapshot-bytes+v3");
    }

    #[test]
    fn a_gap_in_the_registry_stops_the_walk_at_the_missing_row() {
        let bytes = sample(DocType::Trame);
        // No row leaves 3, so a 3 -> 4 target reports the version it got stuck on
        // rather than silently returning a half-migrated file.
        assert_eq!(
            migrate_steps(SYNTHETIC, 3, 4, &bytes),
            Err(DocFileError::UnsupportedVersion { found: 3 })
        );
    }

    #[test]
    fn a_failing_step_fails_the_whole_migration() {
        let truncated = sample(DocType::Trame)[..2].to_vec();
        assert_eq!(
            migrate_steps(SYNTHETIC, 1, 2, &truncated),
            Err(DocFileError::TruncatedHeader)
        );
    }
}
