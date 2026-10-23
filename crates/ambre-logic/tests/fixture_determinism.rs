// crates/ambre-logic/tests/fixture_determinism.rs
// ============================================================================
// The determinism and round-trip belts of `ambre-spec-fixtures.md` -- the two
// that gate `FIXTURES P0`'s Definition of Done.
//
// **Ungated on purpose.** Everything asserted here is production API. That is
// `TrameState::with_peer` / `ToileState::with_peer` (the I1 seams, documented in
// `ambre-spec-fixtures.md` / `toile-engine`), `export_ambre_plaintext`, and
// `DocState::import_snapshot` / `export_snapshot` (I4's three ungated symbols).
// So this belt runs under a bare `cargo test --workspace`, that is, inside the CI
// `test-native` job, which is what "green in CI" requires of P0. The corpus
// machinery proper -- loader, manifest, driver -- is behind the `fixtures`
// feature and has its own belt in `fixtures_corpus.rs`.
// ============================================================================

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "a test asserts by panicking (ARCH-028 holds outside the tests)"
)]

use ambre_bus::{CellPos, Col, DocType, Row, TrameCommand};
use ambre_core::DocumentId;
use ambre_logic::DocState;
use ambre_prose::{ProseCommand, ProseState};
use ambre_toile::ToileState;
use ambre_toile::logic::command::{InitContent, ToileCommand};
use ambre_toile::logic::model::{ElementKind, ShapeKind};
use ambre_trame::TrameState;
use kurbo::{Point, Size};

/// The seed every single-peer case in this file builds from. A constant, like a
/// real generator's `seed()`: a seed supplied from outside could differ between
/// runs and turn the drift gate into noise.
const SEED: u64 = 0x0BAD_C0FF_EE0D_D00D;

/// The `document_id` the header carries. Pinned rather than minted. A UUID v7 is
/// time-ordered and partly random (I1's third entropy source). `DocumentId::new()`
/// here would defeat the byte comparison before the payload was even reached.
/// The seed-derived form lives in `ambre_core::fixtures::seeded_document_id`,
/// which is fixture code and belted there.
fn doc_id() -> DocumentId {
    DocumentId::from_bytes([
        0x01, 0x8f, 0x2a, 0x3b, 0x7c, 0x4d, 0x70, 0x00, 0x80, 0x00, 0xde, 0xad, 0xbe, 0xef, 0x00,
        0x11,
    ])
}

// -- the two stand-in generators ----------------------------------------------
//
// P0 delivers the mechanism, not the corpus (`FIXTURES P1` writes the real
// generators), so the belt drives its own build bodies. They are deliberately
// shaped like a generator: a pinned peer from the seed, a fixed command
// sequence, an `export_ambre_plaintext` at the end.

/// A workbook with literals, a formula chain over them, a second sheet and a
/// merge. Enough that a lost op shows up in the bytes.
fn build_trame(seed: u64, doc_id: DocumentId) -> Vec<u8> {
    let mut state = TrameState::with_peer(ambre_core::PeerId(seed));
    state.handle_command(TrameCommand::SetCellText {
        row: Row::new(0),
        col: Col::new(0),
        text: "21".to_owned(),
    });
    state.handle_command(TrameCommand::SetCellText {
        row: Row::new(1),
        col: Col::new(0),
        text: "=A1*2".to_owned(),
    });
    state.handle_command(TrameCommand::SetCellText {
        row: Row::new(2),
        col: Col::new(0),
        text: "=A2+A1".to_owned(),
    });
    state.handle_command(TrameCommand::SetCellsBatch {
        cells: (0..8)
            .map(|i| (CellPos::new(Row::new(i), Col::new(3)), format!("row {i}")))
            .collect::<Vec<_>>(),
    });
    state
        .export_ambre_plaintext(doc_id)
        .expect("the document reads")
}

/// A board with several shapes and a frame, so the dump and the shape summary
/// have more than one node kind to report.
fn build_toile(seed: u64, doc_id: DocumentId) -> Vec<u8> {
    let mut state = ToileState::with_peer(ambre_core::PeerId(seed));
    for i in 0..4 {
        state.handle_command(ToileCommand::CreateElement {
            kind: ElementKind::Shape(ShapeKind::Rect),
            at: Point::new(10.0 * f64::from(i), 20.0),
            size: Size::new(100.0, 50.0),
            content: InitContent::None,
            fill: None,
            correlate: None,
        });
    }
    state.handle_command(ToileCommand::CreateElement {
        kind: ElementKind::Frame,
        at: Point::new(-200.0, -200.0),
        size: Size::new(640.0, 480.0),
        content: InitContent::None,
        fill: None,
        correlate: None,
    });
    state
        .export_ambre_plaintext(doc_id)
        .expect("the document reads")
}

// -----------------------------------------------------------------------------
// I1 -- determinism
// -----------------------------------------------------------------------------

#[test]
fn a_generator_rebuilt_in_the_same_process_is_byte_identical() {
    // The plainest half of I1, and the one the seam exists for. Without a pinned
    // `PeerID` these two already differ. `LoroDoc::new()` mints a fresh random
    // one per document, and identical command sequences then produce ops
    // attributed to different peers.
    assert_eq!(build_trame(SEED, doc_id()), build_trame(SEED, doc_id()));
    assert_eq!(build_toile(SEED, doc_id()), build_toile(SEED, doc_id()));
}

#[test]
fn a_generator_that_ignores_its_seed_is_caught() {
    // 's "test-only pair": two builds sharing one body and differing only in
    // their seed must produce different bytes. Without this half, the test above
    // passes vacuously on a generator that hardcodes its content. Such a
    // fixture's `seed` field would be a lie in every manifest.
    assert_ne!(build_trame(SEED, doc_id()), build_trame(SEED ^ 1, doc_id()));
    assert_ne!(build_toile(SEED, doc_id()), build_toile(SEED ^ 1, doc_id()));
}

#[test]
fn the_document_id_is_part_of_what_the_digest_covers() {
    // The header is inside the artifact, so a different `document_id` is a
    // different fixture even at identical payload. C8: the digest covers the
    // whole file, header included.
    let other = DocumentId::from_bytes([0x22; 16]);
    assert_ne!(build_trame(SEED, doc_id()), build_trame(SEED, other));
    assert_eq!(
        build_trame(SEED, doc_id())[22..],
        build_trame(SEED, other)[22..],
        "only the header may differ: the payload is the same document"
    );
}

#[test]
fn an_unpinned_peer_is_not_deterministic() {
    // The negative control that gives the assertions above their meaning. If this
    // ever starts passing, `LoroDoc::new()` has stopped drawing a random peer and
    // the seam's justification needs revisiting -- not a silent free pass.
    let bare = |()| {
        let mut state = TrameState::new();
        state.handle_command(TrameCommand::SetCellText {
            row: Row::new(0),
            col: Col::new(0),
            text: "21".to_owned(),
        });
        state
            .export_ambre_plaintext(doc_id())
            .expect("the document reads")
    };
    assert_ne!(bare(()), bare(()));
}

/// Set in the child process this file re-execs; see the test below.
const CHILD_MARKER: &str = "AMBRE_FIXTURES_DETERMINISM_CHILD";

#[test]
fn a_generator_is_byte_identical_across_processes() {
    // **The half that matters.** Rust reseeds `HashMap` state per process. An
    // encoder iterating a hash map would be stable within a run and unstable
    // between runs, exactly the failure an in-process rebuild cannot see. The
    // spike measured this; the belt keeps it measured.
    //
    // Mechanism: the test re-execs its own binary with `--exact` on this test
    // name and a marker in the environment. The child takes the branch below,
    // prints its digests and exits; the parent compares them against its own.
    if std::env::var_os(CHILD_MARKER).is_some() {
        println!("TRAME {}", hex(&build_trame(SEED, doc_id())));
        println!("TOILE {}", hex(&build_toile(SEED, doc_id())));
        return;
    }

    let exe = std::env::current_exe().expect("the test binary knows its own path");
    let output = std::process::Command::new(exe)
        .args([
            "--exact",
            "a_generator_is_byte_identical_across_processes",
            "--nocapture",
        ])
        .env(CHILD_MARKER, "1")
        .output()
        .expect("re-exec of the test binary");
    assert!(
        output.status.success(),
        "child run failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    let line = |tag: &str| {
        stdout
            .lines()
            .find_map(|l| l.strip_prefix(tag).map(str::trim))
            .unwrap_or_else(|| panic!("child printed no `{tag}` line in:\n{stdout}"))
            .to_owned()
    };
    assert_eq!(line("TRAME "), hex(&build_trame(SEED, doc_id())));
    assert_eq!(line("TOILE "), hex(&build_toile(SEED, doc_id())));
}

/// Lowercase hex of a byte slice. Local rather than `hex::encode`: this belt is
/// ungated and must not pull a dependency the crate would otherwise not have.
fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

#[test]
fn the_commit_clock_is_not_recorded() {
    // The second entropy source. Two builds separated by a real interval must
    // still agree; a recorded timestamp would put wall-clock milliseconds into
    // the ops and make every regeneration drift.
    let first = build_trame(SEED, doc_id());
    std::thread::sleep(std::time::Duration::from_millis(1100));
    assert_eq!(first, build_trame(SEED, doc_id()));
}

#[test]
fn a_two_peer_merge_is_deterministic() {
    // One of the two cases the spike left open. Two pinned peers edit disjoint
    // cells, exchange full snapshots, and the merged document is exported. The
    // result must be reproducible, or no post-convergence fixture (`FIXTURES
    // P1`) can ever be committed.
    let merged = |()| {
        let mut alice = TrameState::with_peer(ambre_core::PeerId(SEED));
        let mut bob = TrameState::with_peer(ambre_core::PeerId(SEED ^ 0xFFFF_FFFF));
        alice.handle_command(TrameCommand::SetCellText {
            row: Row::new(0),
            col: Col::new(0),
            text: "from alice".to_owned(),
        });
        bob.handle_command(TrameCommand::SetCellText {
            row: Row::new(5),
            col: Col::new(2),
            text: "from bob".to_owned(),
        });
        // Both directions, so the merged history carries both peers' ops.
        alice.apply_remote_snapshot(&bob.export_snapshot());
        bob.apply_remote_snapshot(&alice.export_snapshot());
        (
            alice
                .export_ambre_plaintext(doc_id())
                .expect("the document reads"),
            bob.export_ambre_plaintext(doc_id())
                .expect("the document reads"),
        )
    };
    let (alice_1, bob_1) = merged(());
    let (alice_2, bob_2) = merged(());
    assert_eq!(
        alice_1, alice_2,
        "a merged document must rebuild identically"
    );
    assert_eq!(bob_1, bob_2);
    // Convergence itself is `convergence_tests.rs`'s subject and is not restated
    // here. What this asserts is only that a *merged* document is reproducible,
    // which is what a post-convergence fixture needs.
}

#[test]
fn a_large_document_is_deterministic() {
    // The other case the spike left open: scale. An encoder iterating a hash map
    // would stay stable at four cells and drift at a few thousand. The invariant
    // is therefore asserted on a document wide enough for the encoder to take its
    // wide paths. That is 5 000 cells over 250 rows and 20 columns, which also
    // exercises the column axis rather than one very long strip.
    //
    // Not the spec's 100k. This belt runs on every `cargo test --workspace`, and
    // a 100k-cell build is tens of seconds in a debug profile. That is exactly
    // the "corpus developers stop regenerating" failure warns about. The 100k
    // case is `FIXTURES P2`'s benchmark corpus, which builds it under `criterion`
    // in release and can afford it.
    let build = |()| {
        let mut state = TrameState::with_peer(ambre_core::PeerId(SEED));
        state.handle_command(TrameCommand::SetCellsBatch {
            cells: (0..250)
                .flat_map(|row| {
                    (0..20).map(move |col| {
                        let at = CellPos::new(Row::new(row), Col::new(col));
                        (at, format!("v{row}_{col}"))
                    })
                })
                .collect(),
        });
        state
            .export_ambre_plaintext(doc_id())
            .expect("the document reads")
    };
    let first = build(());
    assert!(first.len() > 10_000, "a 5k-cell document, not an empty one");
    assert_eq!(first, build(()));
}

// -----------------------------------------------------------------------------
// The round trip, on every `DocState` variant
// -----------------------------------------------------------------------------

/// `export_snapshot -> import_snapshot -> export_snapshot` is **stable** on one
/// variant.
///
/// Stability, not byte equality with the source. A fresh state commits its own
/// scaffolding under its own peer *before* the import. Both engines materialize
/// their derived containers lazily on the first read of a restored document. The
/// first restore therefore legitimately carries ops and containers the source
/// snapshot never had; neither is a lost round trip. What must hold is the
/// **fixed point**: once a document has been through the cycle, going through it
/// again changes nothing. That is the property a fixture corpus depends on, and
/// the one a regression would break.
fn assert_round_trip(source: &DocState, doc_type: DocType) {
    let first = source.export_snapshot();
    assert!(!first.is_empty(), "{doc_type:?}: a snapshot, not nothing");

    // Into a *fresh* state of the same type, which is what a loader does.
    let mut restored = DocState::new(doc_type).expect("an engine for this doc_type");
    let events = restored
        .import_snapshot(&first)
        .expect("a fixture this build generated reads back at the current schema");
    assert!(
        !events.is_empty(),
        "{doc_type:?}: a restore must tell the UI to repaint"
    );

    let second = restored.export_snapshot();
    assert!(
        !second.is_empty(),
        "{doc_type:?}: a restore produced nothing"
    );

    // The fixed point. Re-importing a document's own bytes into the state that
    // produced them is a no-op merge, so the next export must be identical. This
    // is what fails if an import path starts mutating the document it reads.
    restored
        .import_snapshot(&second)
        .expect("the document's own bytes read back");
    assert_eq!(
        restored.export_snapshot(),
        second,
        "{doc_type:?}: the export/import cycle has no fixed point"
    );

    // And export alone is pure: two exports with nothing in between agree.
    assert_eq!(
        restored.export_snapshot(),
        second,
        "{doc_type:?}: export is not idempotent"
    );
}

#[test]
fn the_round_trip_is_stable_on_every_doc_state_variant() {
    // "Every variant" is the point. `DocState::export_snapshot` had to be lifted
    // to the enum for this belt to be writable at all (B5).
    // `ToileState::import_snapshot` had to exist for the Toile arm.
    let mut trame = TrameState::with_peer(ambre_core::PeerId(SEED));
    trame.handle_command(TrameCommand::SetCellText {
        row: Row::new(0),
        col: Col::new(0),
        text: "21".to_owned(),
    });
    assert_round_trip(&DocState::Trame(Box::new(trame)), DocType::Trame);

    let mut toile = ToileState::with_peer(ambre_core::PeerId(SEED));
    toile.handle_command(ToileCommand::CreateElement {
        kind: ElementKind::Shape(ShapeKind::Rect),
        at: Point::new(1.0, 2.0),
        size: Size::new(3.0, 4.0),
        content: InitContent::None,
        fill: None,
        correlate: None,
    });
    assert_round_trip(&DocState::Toile(Box::new(toile)), DocType::Toile);

    let mut prose = ProseState::with_peer(ambre_core::PeerId(SEED)).expect("a document is built");
    let block = prose.root_blocks()[0];
    prose.handle_command(ProseCommand::InsertText {
        block_id: block,
        pos: ambre_bus::app::prose::DocOffset::ZERO,
        text: "a paragraph".to_owned(),
    });
    assert_round_trip(&DocState::Prose(Box::new(prose)), DocType::Prose);
}

#[test]
fn every_variant_reports_its_own_doc_type() {
    // `DocState::doc_type` is the inverse of `DocState::new`, and what the driver
    // writes into a manifest. A variant that misreported it would produce an
    // artifact no importer could route.
    for doc_type in [DocType::Trame, DocType::Toile, DocType::Prose] {
        let state = DocState::new(doc_type).expect("an engine");
        assert_eq!(state.doc_type(), doc_type);
    }
    assert!(
        DocState::new(DocType::Murmure).is_none(),
        "Murmure has no engine yet, and the loader must see that"
    );
}
