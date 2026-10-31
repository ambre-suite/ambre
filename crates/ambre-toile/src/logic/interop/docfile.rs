//! The `.ambre` native container, Toile side (`ambre-spec-docfile.md`).
//!
//! Toile's half of the pair: the payload is this board's `LoroDoc` snapshot. The
//! header is not built here. Layout, version registry and both normative read
//! rules live once in `ambre_core::docfile`, and are called from here. That is
//! I5: one container, every app, with no per-app header variant.
//!
//! Unlike the SVG / raster converters next door, this path is **lossless**. It
//! emits no [`ImportReport`](super::ImportReport) and no warning list, because
//! nothing is converted: the payload *is* the document (I1).

use loro::{ExportMode, LoroDoc};

use ambre_core::id::{Document, Id};
use ambre_core::{
    DocType,
    docfile::{AmbreHeader, DocFileError, read_ambre_plaintext_as, write_ambre_plaintext},
};

/// Plaintext `.ambre` bytes for a Toile document: the current-version header
/// followed by an uncompressed, unencrypted Loro snapshot.
///
/// Pure and crypto-free (I4). Protection, when it applies, is a separate stage
/// owned by `ambre-sync` (`docfile`).
pub fn export_ambre_plaintext(
    doc: &LoroDoc,
    doc_id: Id<Document>,
) -> Option<ambre_core::Payload<ambre_core::PlaintextContainer>> {
    // `None` rather than a header over nothing (`TOILE-IO-069`). A well-formed
    // file carrying the right document id and an empty board is what the user
    // cannot tell from a board they emptied themselves.
    let payload = doc
        .export(ExportMode::Snapshot)
        .inspect_err(|e| tracing::error!("[Toile] .ambre export failed, writing no file: {e:?}"))
        .ok()?;
    Some(write_ambre_plaintext(
        DocType::Toile,
        doc_id,
        &ambre_core::Payload::<ambre_core::SnapshotBytes>::new(payload),
    ))
}

/// Validate a plaintext `.ambre` as a *Toile* document and hand back the header
/// plus the payload.
///
/// The whole read policy is inherited. An older container migrates, and a newer
/// one is refused non-destructively. A valid file belonging to another app comes
/// back as [`DocFileError::WrongDocType`] carrying the type. The caller then
/// re-dispatches those same bytes rather than coercing the document onto a board
/// (dispatch rule, `docfile`).
///
/// # Errors
///
/// Every error of [`read_ambre_plaintext_as`], including
/// [`DocFileError::WrongDocType`] for a valid file of another app.
pub fn import_ambre_plaintext(
    data: &ambre_core::Payload<ambre_core::PlaintextContainer>,
) -> Result<AmbreHeader, DocFileError> {
    read_ambre_plaintext_as(DocType::Toile, data)
}
