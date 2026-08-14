// ambre-trame/src/logic/docfile.rs
// The `.ambre` native container, Trame side (`ambre-spec-docfile.md` §2.B).
//
// Trame's half of the pair: the payload is this workbook's `LoroDoc` snapshot.
// The header is not built here -- layout, version registry and both normative
// read rules live once in `ambre_core::docfile` and are called from here (I5:
// one container, every app; no per-app header variant).
//
// Part of the `TrameState` impl, split out by concern like `sync` and `interop`.
// Submodule of `logic`, so it reaches the private fields of the parent module
// through the `use super::*` glob.

use ambre_core::{
    DocType, Uuid,
    docfile::{AmbreHeader, DocFileError, read_ambre_plaintext_as, write_ambre_plaintext},
};

use super::*;

/// Plaintext `.ambre` bytes for a Trame document: the current-version header
/// followed by an uncompressed, unencrypted Loro snapshot.
///
/// Pure and crypto-free (I4). Protection, when it applies, is a separate stage
/// owned by `ambre-sync` (`docfile §3`).
pub fn export_ambre_plaintext(doc: &LoroDoc, doc_id: Uuid) -> Vec<u8> {
    let payload = doc.export(ExportMode::Snapshot).unwrap_or_else(|e| {
        // Same treatment as `TrameState::export_snapshot`: never silently, since
        // the caller is about to write these bytes to a file the user believes
        // holds their document.
        tracing::error!("[Trame] .ambre export failed, writing an empty payload: {e:?}");
        Vec::new()
    });
    write_ambre_plaintext(DocType::Trame, doc_id, &payload)
}

/// Validate a plaintext `.ambre` as a *Trame* document and hand back the header
/// plus the payload.
///
/// The whole read policy is inherited: an older container migrates, a newer one
/// is refused non-destructively, and a valid file belonging to another app comes
/// back as [`DocFileError::WrongDocType`] carrying the type, so the caller
/// re-dispatches those same bytes rather than coercing the document into a
/// spreadsheet (dispatch rule, `docfile §2.B`).
pub fn import_ambre_plaintext(data: &[u8]) -> Result<AmbreHeader, DocFileError> {
    read_ambre_plaintext_as(DocType::Trame, data)
}

impl TrameState {
    /// Export this workbook as plaintext `.ambre` bytes.
    ///
    /// `doc_id` is supplied by the caller because the engine does not hold the
    /// document's identity -- the registry does (`ambre-logic`), and a fixture
    /// generator pins it from its seed (`ambre-spec-fixtures.md` §2.A).
    pub fn export_ambre_plaintext(&self, doc_id: Uuid) -> Vec<u8> {
        export_ambre_plaintext(&self.doc, doc_id)
    }

    /// Load a plaintext `.ambre` into this state, returning the document id it
    /// carried and the display events the UI needs to repaint.
    ///
    /// Nothing is applied until the header has validated, and a payload the CRDT
    /// engine refuses leaves the state as it was: the two failure modes are
    /// distinct, and [`DocFileError::PayloadRejected`] is the only one this layer
    /// adds to the container taxonomy.
    pub fn import_ambre_plaintext(
        &mut self,
        data: &[u8],
    ) -> Result<(Uuid, Vec<TrameEvent>), DocFileError> {
        let header = import_ambre_plaintext(data)?;
        self.doc.import(&header.payload).map_err(|e| {
            tracing::warn!("[Trame] .ambre payload rejected by the CRDT engine: {e:?}");
            DocFileError::PayloadRejected
        })?;
        // Same tail as a snapshot restore: rebuild the derived caches and reseed
        // the formula engine so formula cells evaluate instead of showing their
        // source (`sync::import_snapshot`).
        let events = self.reseed_after_merge("import_ambre_plaintext");
        Ok((header.document_id, events))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ambre_core::docfile::{AMBRE_VERSION, HEADER_LEN};

    fn doc_id() -> Uuid {
        Uuid::from_bytes([
            0x01, 0x8f, 0x2a, 0x3b, 0x7c, 0x4d, 0x70, 0x00, 0x80, 0x00, 0x11, 0x22, 0x33, 0x44,
            0x55, 0x66,
        ])
    }

    /// A workbook with a literal, a formula depending on it, and a style, so the
    /// round trip is asserted on more than an empty document.
    fn populated() -> TrameState {
        let mut state = TrameState::new();
        state.handle_command(TrameCommand::SetCellText {
            row: 0,
            col: 0,
            text: "21".to_owned(),
        });
        state.handle_command(TrameCommand::SetCellText {
            row: 1,
            col: 0,
            text: "=A1*2".to_owned(),
        });
        state
    }

    /// Read one cell's displayed text through a viewport request (the only
    /// public read path into the grid).
    fn read_cell(state: &mut TrameState, row: u32, col: u32) -> String {
        let outcome = state.handle_command(TrameCommand::ScrollViewport {
            version: 1,
            row_start: row,
            row_end: row,
            col_start: col,
            col_end: col,
        });
        let TrameEvent::ViewportData { cells, .. } = &outcome.display[0] else {
            panic!("expected ViewportData");
        };
        cells[0].text.clone()
    }

    #[test]
    fn the_header_names_trame_and_the_document() {
        let state = populated();
        let bytes = state.export_ambre_plaintext(doc_id());
        let header = import_ambre_plaintext(&bytes).expect("a Trame container");
        assert_eq!(header.version, AMBRE_VERSION);
        assert_eq!(header.doc_type, DocType::Trame);
        assert_eq!(header.document_id, doc_id());
        assert_eq!(header.payload, bytes[HEADER_LEN..]);
        assert!(!header.payload.is_empty());
    }

    #[test]
    fn round_trip_is_byte_identical_on_a_bare_document() {
        // I1 at its strictest: the same ops re-exported from a document that
        // received nothing but this payload reproduce the file exactly. Asserted
        // on a bare `LoroDoc` rather than a `TrameState`, because
        // `TrameState::new` commits its own default-sheet op under a fresh random
        // peer id -- that op is a second peer's history, not a lost round trip.
        let bytes = populated().export_ambre_plaintext(doc_id());
        let header = import_ambre_plaintext(&bytes).expect("a Trame container");
        let reloaded = LoroDoc::new();
        reloaded.import(&header.payload).expect("payload imports");
        assert_eq!(export_ambre_plaintext(&reloaded, doc_id()), bytes);
    }

    #[test]
    fn round_trip_preserves_the_document_state() {
        let bytes = populated().export_ambre_plaintext(doc_id());
        let mut restored = TrameState::new();
        let (id, _events) = restored
            .import_ambre_plaintext(&bytes)
            .expect("a Trame container");
        assert_eq!(id, doc_id());
        assert_eq!(read_cell(&mut restored, 0, 0), "21");
        // The formula is reseeded, not left as its source text.
        assert_eq!(read_cell(&mut restored, 1, 0), "42");
    }

    #[test]
    fn a_toile_file_is_re_dispatched_not_coerced() {
        let toile = write_ambre_plaintext(DocType::Toile, doc_id(), b"whatever");
        assert_eq!(
            import_ambre_plaintext(&toile),
            Err(DocFileError::WrongDocType {
                found: DocType::Toile
            })
        );
        let mut state = TrameState::new();
        // `.err()`: `TrameEvent` (the success half) is not `PartialEq`.
        assert_eq!(
            state.import_ambre_plaintext(&toile).err(),
            Some(DocFileError::WrongDocType {
                found: DocType::Toile
            })
        );
    }

    #[test]
    fn a_corrupt_payload_under_a_valid_header_is_typed_not_a_panic() {
        let bytes = write_ambre_plaintext(DocType::Trame, doc_id(), b"not a loro snapshot");
        let mut state = TrameState::new();
        assert_eq!(
            state.import_ambre_plaintext(&bytes).err(),
            Some(DocFileError::PayloadRejected)
        );
    }

    #[test]
    fn a_rejected_file_leaves_both_the_input_and_the_state_alone() {
        // I3, on the two things a failed open must not damage: the file's bytes
        // and the document already open.
        let good = populated().export_ambre_plaintext(doc_id());
        let mut state = TrameState::new();
        state.import_ambre_plaintext(&good).expect("loads");
        let before_state = state.export_snapshot();

        let cases: Vec<Vec<u8>> = vec![
            good[..8].to_vec(),
            {
                let mut b = good.clone();
                b[0] = b'X';
                b
            },
            {
                let mut b = good.clone();
                b[4] = AMBRE_VERSION + 1;
                b
            },
            write_ambre_plaintext(DocType::Toile, doc_id(), b"whatever"),
            write_ambre_plaintext(DocType::Trame, doc_id(), b"not a loro snapshot"),
        ];
        for bytes in cases {
            let before_bytes = bytes.clone();
            assert!(state.import_ambre_plaintext(&bytes).is_err());
            assert_eq!(bytes, before_bytes, "a failed import rewrote its input");
            assert_eq!(
                state.export_snapshot(),
                before_state,
                "a failed import mutated the open document"
            );
        }
        assert_eq!(read_cell(&mut state, 1, 0), "42");
    }
}
