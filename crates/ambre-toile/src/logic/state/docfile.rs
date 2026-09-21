//! `ToileState` <-> `.ambre` container (`ambre-spec-docfile.md`).
//!
//! The state-side half of the pair. The container functions themselves live in
//! [`crate::logic::interop::docfile`], since they need only a `LoroDoc`. These
//! two methods bind them to a live board: the snapshot on the way out, the cache
//! rebuild on the way in.

use ambre_core::{Uuid, docfile::DocFileError, schema::SchemaError};

use crate::logic::interop::docfile::{export_ambre_plaintext, import_ambre_plaintext};

use super::*;

impl ToileState {
    /// Export this board as plaintext `.ambre` bytes.
    ///
    /// `doc_id` is supplied by the caller because the engine does not hold the
    /// document's identity. The registry does (`ambre-logic`), and a fixture
    /// generator pins it from its seed (`ambre-spec-fixtures.md`).
    /// `None` when the board could not be read: no file is better than a file
    /// that opens empty (`TOILE-IO-069`).
    pub fn export_ambre_plaintext(&self, doc_id: Uuid) -> Option<Vec<u8>> {
        export_ambre_plaintext(&self.doc, doc_id)
    }

    /// Load a plaintext `.ambre` into this state, returning the document id it
    /// carried and the display events the UI needs to repaint.
    ///
    /// Nothing is applied until the header and the schema register have both
    /// validated. A payload the CRDT engine refuses leaves the board as it was.
    /// The two failure modes are distinct, and [`DocFileError::PayloadRejected`]
    /// is the only one this layer adds to the container taxonomy.
    ///
    /// # Errors
    ///
    /// Every error of the free [`import_ambre_plaintext`], plus
    /// [`DocFileError::PayloadRejected`] when the schema register or the CRDT
    /// engine refuses the payload. The board is unchanged in either case.
    pub fn import_ambre_plaintext(
        &mut self,
        data: &[u8],
    ) -> Result<(Uuid, Vec<ToileEvent>), DocFileError> {
        let header = import_ambre_plaintext(data)?;
        // The container has said its piece; the schema register inside the
        // payload is a separate gate and it runs before any container is used
        // (`SCHEMA-006`). A file this build cannot read leaves the board alone.
        let payload = schema::gate(&header.payload).map_err(|e| match e {
            // Bytes the engine will not read already have a name in the
            // container taxonomy; the gate only happens to be where they were
            // discovered.
            SchemaError::PayloadRejected => DocFileError::PayloadRejected,
            other => DocFileError::PayloadSchema(other),
        })?;
        self.doc.import(&payload).map_err(|e| {
            tracing::warn!("[Toile] .ambre payload rejected by the CRDT engine: {e:?}");
            DocFileError::PayloadRejected
        })?;
        // Same tail as `apply_remote`: the derived caches (index, bounds, layers)
        // are rebuilt from the merged document before anything reads them.
        self.rebuild_caches();
        Ok((header.document_id, vec![self.viewport_event()]))
    }
}

#[cfg(test)]
mod tests {
    use ambre_core::DocType;
    use ambre_core::docfile::{
        AMBRE_VERSION, HEADER_LEN, read_ambre_plaintext_as, write_ambre_plaintext,
    };

    use kurbo::{Point, Size};

    use crate::logic::command::{InitContent, ToileCommand};
    use crate::logic::model::{ElementKind, ShapeKind};

    use super::*;

    fn doc_id() -> Uuid {
        Uuid::from_bytes([
            0x01, 0x8f, 0x2a, 0x3b, 0x7c, 0x4d, 0x70, 0x00, 0x80, 0x00, 0x11, 0x22, 0x33, 0x44,
            0x55, 0x66,
        ])
    }

    /// A board carrying two elements, so the round trip is asserted on more than
    /// an empty document.
    fn populated() -> ToileState {
        let mut state = ToileState::new();
        for i in 0..2 {
            state.handle_command(ToileCommand::CreateElement {
                kind: ElementKind::Shape(ShapeKind::Rect),
                at: Point::new(10.0 * f64::from(i), 20.0),
                size: Size::new(100.0, 50.0),
                content: InitContent::None,
                fill: None,
                correlate: 0,
            });
        }
        state
    }

    #[test]
    fn the_header_names_toile_and_the_document() {
        let bytes = populated()
            .export_ambre_plaintext(doc_id())
            .expect("the document reads");
        let header = import_ambre_plaintext(&bytes).expect("a Toile container");
        assert_eq!(header.version, AMBRE_VERSION);
        assert_eq!(header.doc_type, DocType::Toile);
        assert_eq!(header.document_id, doc_id());
        assert_eq!(header.payload, bytes[HEADER_LEN..]);
        assert!(!header.payload.is_empty());
    }

    #[test]
    fn round_trip_is_byte_identical_on_a_bare_document() {
        // I1 at its strictest: the same ops re-exported from a document that
        // received nothing but this payload reproduce the file exactly. Asserted on
        // a bare `LoroDoc` rather than a `ToileState`, because `ToileState::new`
        // commits its own board scaffolding under a fresh random peer id. That op
        // is a second peer's history, not a lost round trip.
        let bytes = populated()
            .export_ambre_plaintext(doc_id())
            .expect("the document reads");
        let header = import_ambre_plaintext(&bytes).expect("a Toile container");
        let reloaded = LoroDoc::new();
        reloaded.import(&header.payload).expect("payload imports");
        assert_eq!(
            export_ambre_plaintext(&reloaded, doc_id()).expect("the document reads"),
            bytes
        );
    }

    #[test]
    fn round_trip_preserves_the_document_state() {
        let source = populated();
        let expected: Vec<NodeId> = source.layer_order().to_vec();
        let bytes = source
            .export_ambre_plaintext(doc_id())
            .expect("the document reads");

        let mut restored = ToileState::new();
        let (id, events) = restored
            .import_ambre_plaintext(&bytes)
            .expect("a Toile container");
        assert_eq!(id, doc_id());
        assert_eq!(events.len(), 1, "one repaint, not one per element");
        // The derived caches are rebuilt, not left empty: paint order and the
        // element count come from `rebuild_caches`, not from the raw CRDT.
        assert_eq!(restored.layer_order(), expected.as_slice());
        assert_eq!(restored.element_count(), source.element_count());
    }

    #[test]
    fn a_trame_file_is_re_dispatched_not_coerced() {
        let trame = write_ambre_plaintext(DocType::Trame, doc_id(), b"whatever");
        assert_eq!(
            import_ambre_plaintext(&trame),
            Err(DocFileError::WrongDocType {
                found: DocType::Trame
            })
        );
        let mut state = ToileState::new();
        // `.err()`: `ToileEvent` (the success half) is not `PartialEq`.
        assert_eq!(
            state.import_ambre_plaintext(&trame).err(),
            Some(DocFileError::WrongDocType {
                found: DocType::Trame
            })
        );
        // The same bytes are accepted, unchanged, by the importer they name.
        assert!(read_ambre_plaintext_as(DocType::Trame, &trame).is_ok());
    }

    #[test]
    fn a_corrupt_payload_under_a_valid_header_is_typed_not_a_panic() {
        let bytes = write_ambre_plaintext(DocType::Toile, doc_id(), b"not a loro snapshot");
        let mut state = ToileState::new();
        assert_eq!(
            state.import_ambre_plaintext(&bytes).err(),
            Some(DocFileError::PayloadRejected)
        );
    }

    #[test]
    fn a_rejected_file_leaves_both_the_input_and_the_state_alone() {
        // I3, on the two things a failed open must not damage: the file's bytes
        // and the board already open.
        let good = populated()
            .export_ambre_plaintext(doc_id())
            .expect("the document reads");
        let mut state = ToileState::new();
        state.import_ambre_plaintext(&good).expect("loads");
        let before_state = state.export_snapshot();
        let before_layers = state.layer_order().to_vec();

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
            write_ambre_plaintext(DocType::Trame, doc_id(), b"whatever"),
            write_ambre_plaintext(DocType::Toile, doc_id(), b"not a loro snapshot"),
        ];
        for bytes in cases {
            let before_bytes = bytes.clone();
            assert!(state.import_ambre_plaintext(&bytes).is_err());
            assert_eq!(bytes, before_bytes, "a failed import rewrote its input");
            assert_eq!(
                state.export_snapshot(),
                before_state,
                "a failed import mutated the open board"
            );
        }
        assert_eq!(state.layer_order(), before_layers.as_slice());
    }
}
