// ambre-logic/tests/docfile_dispatch.rs The cross-app half of the `.ambre`
// belt (`ambre-spec-docfile.md`, `DOCFILE-002`).
//
// The dispatch rule is a statement about two apps at once, so it cannot be
// asserted from inside either app crate. It reads: a Toile file handed to the
// Trame importer yields `WrongDocType { found: Toile }` and is accepted by
// the Toile importer unchanged. `ambre-logic` is the first crate that links
// both. It is also where the file-open router will live when `DOCFILE P1`
// wires the chrome. The belt therefore sits here rather than in a test-only
// crate.
//
// Real files on both sides. Each is produced by its own app's exporter, never
// hand-assembled. The test would catch a header written by one app and
// unreadable by the other.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "a test asserts by panicking (ARCH-028 holds outside the tests)"
)]

use ambre_core::{
    DocType, Uuid,
    docfile::{DocFileError, read_ambre_plaintext},
};
use ambre_toile::ToileState;
use ambre_trame::TrameState;

fn doc_id(tag: u8) -> Uuid {
    let mut bytes = [0u8; 16];
    bytes[..8].copy_from_slice(&[0x01, 0x8f, 0x2a, 0x3b, 0x7c, 0x4d, 0x70, 0x00]);
    bytes[15] = tag;
    Uuid::from_bytes(bytes)
}

fn trame_file() -> Vec<u8> {
    let mut state = TrameState::new();
    state.handle_command(ambre_trame::TrameCommand::SetCellText {
        row: 0,
        col: 0,
        text: "hello".to_owned(),
    });
    state
        .export_ambre_plaintext(doc_id(1))
        .expect("the document reads")
}

fn toile_file() -> Vec<u8> {
    let mut state = ToileState::new();
    state.handle_command(ambre_toile::ToileCommand::CreateElement {
        kind: ambre_toile::ElementKind::Sticky,
        at: kurbo::Point::new(0.0, 0.0),
        size: kurbo::Size::new(120.0, 80.0),
        content: ambre_toile::InitContent::Text("hello".to_owned()),
        fill: None,
        correlate: 0,
    });
    state
        .export_ambre_plaintext(doc_id(2))
        .expect("the document reads")
}

#[test]
fn a_toile_file_offered_to_trame_names_the_app_that_should_have_it() {
    let bytes = toile_file();
    assert_eq!(
        ambre_trame::import_ambre_plaintext(&bytes),
        Err(DocFileError::WrongDocType {
            found: DocType::Toile
        }),
        "the type must be carried, or the caller cannot re-dispatch"
    );

    // Re-dispatch: the *same* bytes, unmodified, load in the app they name.
    let before = bytes.clone();
    let mut state = ToileState::new();
    let (id, _events) = state
        .import_ambre_plaintext(&bytes)
        .expect("Toile claims its own file");
    assert_eq!(id, doc_id(2));
    assert_eq!(state.element_count(), 1);
    assert_eq!(bytes, before, "the rejected read mutated the buffer");
}

#[test]
fn a_trame_file_offered_to_toile_names_the_app_that_should_have_it() {
    let bytes = trame_file();
    assert_eq!(
        ambre_toile::import_ambre_plaintext(&bytes),
        Err(DocFileError::WrongDocType {
            found: DocType::Trame
        })
    );

    let before = bytes.clone();
    let mut state = TrameState::new();
    let (id, _events) = state
        .import_ambre_plaintext(&bytes)
        .expect("Trame claims its own file");
    assert_eq!(id, doc_id(1));
    assert_eq!(bytes, before, "the rejected read mutated the buffer");
}

#[test]
fn routing_reads_the_type_off_the_header_without_asking_an_app_first() {
    // What "open a `.ambre`" is: one user action, one header read, then the app
    // is chosen. Not four importers tried in turn (I2 / I5).
    for (bytes, expected) in [
        (trame_file(), DocType::Trame),
        (toile_file(), DocType::Toile),
    ] {
        let header = read_ambre_plaintext(&bytes).expect("a valid container");
        assert_eq!(header.doc_type, expected);
        // The container is app-agnostic: the two files differ in one header byte
        // and their payloads, in nothing else.
        assert_eq!(&bytes[..5], b"AMBR\x01");
    }
}
