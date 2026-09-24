//! The `.ambre` native container, Prose side (`ambre-spec-docfile.md`).
//!
//! Prose's half of the pair: the payload is this document's `LoroDoc`
//! snapshot, and nothing else. The header is not built here. The layout, the
//! version registry and both read rules live once in `ambre_core::docfile`
//! (`PROSE-IO-049`).

use ambre_core::docfile::{
    AmbreHeader, DocFileError, read_ambre_plaintext_as, write_ambre_plaintext,
};
use ambre_core::schema::SchemaError;
use ambre_core::{DocType, Uuid};
use loro::{ExportMode, LoroDoc};

use crate::events::ProseEvent;

use super::ProseState;

/// Plaintext `.ambre` bytes for a Prose document: the current-version header
/// over an uncompressed, unencrypted Loro snapshot.
///
/// The payload is the snapshot itself, never a conversion of it
/// (`PROSE-IO-059`). `None` when the document cannot be read: a header over
/// nothing would open as a document the user emptied.
#[must_use]
pub fn export_ambre_plaintext(doc: &LoroDoc, doc_id: Uuid) -> Option<Vec<u8>> {
    let payload = doc
        .export(ExportMode::Snapshot)
        .inspect_err(|e| tracing::error!("Prose: .ambre export failed, writing no file: {e:?}"))
        .ok()?;
    Some(write_ambre_plaintext(DocType::Prose, doc_id, &payload))
}

/// Validate a plaintext `.ambre` as a Prose document and hand back the header
/// and the payload.
///
/// The whole read policy is inherited: an older container migrates, a newer
/// one is refused. A valid file of another application comes back as
/// [`DocFileError::WrongDocType`], so the caller re-dispatches the same bytes
/// rather than reading them as prose (`PROSE-IO-013`).
///
/// # Errors
/// Every error of [`read_ambre_plaintext_as`].
pub fn import_ambre_plaintext(data: &[u8]) -> Result<AmbreHeader, DocFileError> {
    read_ambre_plaintext_as(DocType::Prose, data)
}

impl ProseState {
    /// Export this document as plaintext `.ambre` bytes.
    ///
    /// The caller supplies `doc_id`: the registry holds the document's
    /// identity, not the engine. It reads through `&self`, so an export
    /// changes nothing and emits no delta (`PROSE-IO-017`).
    #[must_use]
    pub fn export_ambre_plaintext(&self, doc_id: Uuid) -> Option<Vec<u8>> {
        export_ambre_plaintext(&self.doc, doc_id)
    }

    /// Load a plaintext `.ambre` into this state, returning the document id
    /// it carried and the display events the view needs.
    ///
    /// Nothing is applied until the header and the schema register have both
    /// validated, so a refused file leaves the state as it was.
    ///
    /// # Errors
    /// Every error of the free [`import_ambre_plaintext`], plus
    /// [`DocFileError::PayloadRejected`] when the CRDT engine refuses the
    /// payload and [`DocFileError::PayloadSchema`] when the schema gate does.
    pub fn import_ambre_plaintext(
        &mut self,
        data: &[u8],
    ) -> Result<(Uuid, Vec<ProseEvent>), DocFileError> {
        let header = import_ambre_plaintext(data)?;
        let events = self.import_snapshot(&header.payload).map_err(|e| match e {
            // Bytes the engine will not read already have a name in the
            // container taxonomy.
            SchemaError::PayloadRejected => DocFileError::PayloadRejected,
            other => DocFileError::PayloadSchema(other),
        })?;
        Ok((header.document_id, events))
    }
}

#[cfg(test)]
mod tests {
    use ambre_core::docfile::{AMBRE_VERSION, HEADER_LEN};

    use super::*;
    use crate::events::ProseCommand;

    fn doc_id() -> Uuid {
        Uuid::from_bytes([
            0x01, 0x8f, 0x2a, 0x3b, 0x7c, 0x4d, 0x70, 0x00, 0x80, 0x00, 0x11, 0x22, 0x33, 0x44,
            0x55, 0x66,
        ])
    }

    /// A document with text and a mark, so the round trip is asserted on more
    /// than an empty paragraph.
    fn populated() -> ProseState {
        let mut state = ProseState::with_peer(3).expect("a document is built");
        let block = state.root_blocks()[0];
        state.handle_command(ProseCommand::InsertText {
            block_id: block,
            pos: 0,
            text: "hello world".to_owned(),
        });
        state.handle_command(ProseCommand::ApplyMark {
            block_id: block,
            range: (0, 5),
            key: "b".to_owned(),
            value: None,
        });
        state
    }

    /// `PROSE-IO-058`, `PROSE-IO-059`: the header names Prose and the
    /// document, and the payload is the snapshot the engine exports.
    #[test]
    fn the_header_names_prose_and_the_payload_is_the_snapshot() {
        let state = populated();
        let bytes = state
            .export_ambre_plaintext(doc_id())
            .expect("the document reads");
        let header = import_ambre_plaintext(&bytes).expect("a Prose container");
        assert_eq!(header.version, AMBRE_VERSION);
        assert_eq!(header.doc_type, DocType::Prose);
        assert_eq!(header.document_id, doc_id());
        assert_eq!(header.payload, bytes[HEADER_LEN..]);
        assert_eq!(header.payload, state.export_snapshot());
    }

    /// `PROSE-IO-012`, `PROSE-IO-066`: the file carries the whole document.
    /// A bare document fed the payload re-exports the same file. A live state
    /// reading it holds the same content, marks included.
    #[test]
    fn the_round_trip_loses_nothing() {
        let state = populated();
        let bytes = state
            .export_ambre_plaintext(doc_id())
            .expect("the document reads");
        let header = import_ambre_plaintext(&bytes).expect("a Prose container");
        let bare = LoroDoc::new();
        bare.import(&header.payload).expect("the payload imports");
        assert_eq!(export_ambre_plaintext(&bare, doc_id()), Some(bytes.clone()));

        let mut restored = ProseState::with_peer(4).expect("a document is built");
        let (id, _events) = restored
            .import_ambre_plaintext(&bytes)
            .expect("a Prose container");
        assert_eq!(id, doc_id());
        let block = state.root_blocks()[0];
        assert_eq!(restored.root_blocks(), vec![block]);
        assert_eq!(restored.text_of(&block), state.text_of(&block));
        let id = crate::events::tree_id(block);
        assert_eq!(
            crate::doc::get_block_text(&restored.doc, &id).to_delta(),
            crate::doc::get_block_text(&state.doc, &id).to_delta(),
        );
    }

    /// `PROSE-IO-017`: an export moves no version, so it has nothing to send.
    #[test]
    fn an_export_changes_nothing() {
        let state = populated();
        let before = state.doc.oplog_vv();
        let _ = state.export_ambre_plaintext(doc_id());
        assert_eq!(state.doc.oplog_vv(), before);
    }

    /// `PROSE-IO-013`: a Trame file is handed back for re-dispatch, carrying
    /// its type, and the document is untouched.
    #[test]
    fn a_file_of_another_application_is_handed_back() {
        let bytes = write_ambre_plaintext(DocType::Trame, doc_id(), b"not prose");
        let mut state = populated();
        let before = state.doc.oplog_vv();
        let refused = state.import_ambre_plaintext(&bytes);
        assert!(matches!(
            refused,
            Err(DocFileError::WrongDocType {
                found: DocType::Trame
            })
        ));
        assert_eq!(state.doc.oplog_vv(), before);
    }

    /// A Prose header over bytes the engine refuses is a typed refusal, and
    /// the document is untouched.
    #[test]
    fn a_payload_the_engine_refuses_is_typed() {
        let bytes = write_ambre_plaintext(DocType::Prose, doc_id(), b"not a snapshot");
        let mut state = populated();
        let before = state.doc.oplog_vv();
        assert!(state.import_ambre_plaintext(&bytes).is_err());
        assert_eq!(state.doc.oplog_vv(), before);
    }
}
