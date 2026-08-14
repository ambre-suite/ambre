//! The `.ambre` native container, Toile side (`ambre-spec-docfile.md` §2.B).
//!
//! Toile's half of the pair: the payload is this board's `LoroDoc` snapshot. The
//! header is not built here -- layout, version registry and both normative read
//! rules live once in `ambre_core::docfile` and are called from here (I5: one
//! container, every app; no per-app header variant).
//!
//! Unlike the SVG / raster converters next door, this path is **lossless**: it
//! emits no [`ImportReport`](super::ImportReport) and no warning list, because
//! nothing is converted -- the payload *is* the document (I1).

use loro::{ExportMode, LoroDoc};

use ambre_core::{
    DocType, Uuid,
    docfile::{AmbreHeader, DocFileError, read_ambre_plaintext_as, write_ambre_plaintext},
};

/// Plaintext `.ambre` bytes for a Toile document: the current-version header
/// followed by an uncompressed, unencrypted Loro snapshot.
///
/// Pure and crypto-free (I4). Protection, when it applies, is a separate stage
/// owned by `ambre-sync` (`docfile §3`).
pub fn export_ambre_plaintext(doc: &LoroDoc, doc_id: Uuid) -> Vec<u8> {
    let payload = doc.export(ExportMode::Snapshot).unwrap_or_else(|e| {
        // Never silent: the caller is about to write these bytes to a file the
        // user believes holds their board.
        tracing::error!("[Toile] .ambre export failed, writing an empty payload: {e:?}");
        Vec::new()
    });
    write_ambre_plaintext(DocType::Toile, doc_id, &payload)
}

/// Validate a plaintext `.ambre` as a *Toile* document and hand back the header
/// plus the payload.
///
/// The whole read policy is inherited: an older container migrates, a newer one
/// is refused non-destructively, and a valid file belonging to another app comes
/// back as [`DocFileError::WrongDocType`] carrying the type, so the caller
/// re-dispatches those same bytes rather than coercing the document onto a board
/// (dispatch rule, `docfile §2.B`).
pub fn import_ambre_plaintext(data: &[u8]) -> Result<AmbreHeader, DocFileError> {
    read_ambre_plaintext_as(DocType::Toile, data)
}
