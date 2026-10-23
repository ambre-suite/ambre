// crates/ambre-logic/tests/fixtures_corpus.rs
// ============================================================================
// The corpus belts of `ambre-spec-fixtures.md`: manifest integrity, the
// driver, the loader-rejection belt and the smaller `load_fixture` one beside
// it, plus `shape_of` / `canonical_dump`.
//
// Gated, unlike `fixture_determinism.rs`: everything here is fixture code (I4)
// and compiles only under `--features fixtures`. Run by
// `cargo test -p ambre-logic --features fixtures`, which `just test-native`
// invokes; a bare `cargo test --workspace` sees an empty file.
// ============================================================================

#![cfg(feature = "fixtures")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "a test asserts by panicking (ARCH-028 holds outside the tests)"
)]

use std::path::{Path, PathBuf};

use ambre_bus::{Col, Row, TrameCommand};
use ambre_core::docfile::{AMBRE_VERSION, DocFileError, HEADER_LEN, write_ambre_plaintext};
use ambre_core::{DocType, DocumentId, FixtureGen, ShapeBody, seeded_document_id};
use ambre_logic::DocState;
use ambre_logic::fixtures::{
    self, FixtureError, canonical_dump, digest_of, generate_into, load_fixture_from, manifest_for,
    read_manifest, shape_of, write_fixture,
};
use ambre_toile::ToileState;
use ambre_toile::logic::command::{InitContent, ToileCommand};
use ambre_toile::logic::model::{ElementKind, ShapeKind};
use ambre_trame::TrameState;
use kurbo::{Point, Size};

// -----------------------------------------------------------------------------
// A synthetic registry
// -----------------------------------------------------------------------------
//
// `FIXTURES P0` ships the mechanism, not the corpus, so the shipping `REGISTRY`
// is empty until `P1` writes the correctness generators. These two drive the
// same driver code path meanwhile -- the pattern `ambre_core::docfile` already
// uses for its (equally empty) migration registry.

/// A Trame generator: literals plus a formula over them, so the artifact has
/// both cell representations to dump.
struct TrameGen {
    name: &'static str,
    seed: u64,
    version: u32,
}

impl FixtureGen for TrameGen {
    fn name(&self) -> &'static str {
        self.name
    }
    fn doc_type(&self) -> DocType {
        DocType::Trame
    }
    fn seed(&self) -> u64 {
        self.seed
    }
    fn generator_version(&self) -> u32 {
        self.version
    }
    fn build(&self) -> Vec<u8> {
        let mut state = TrameState::with_peer(ambre_core::PeerId(self.seed));
        for (row, text) in [(0, "21"), (1, "=A1*2"), (2, "plain")] {
            state.handle_command(TrameCommand::SetCellText {
                row: Row::new(row),
                col: Col::new(0),
                text: text.to_owned(),
            });
        }
        state
            .export_ambre_plaintext(seeded_document_id(self.seed))
            .expect("the document reads")
    }
}

/// A Toile generator, so the belt covers both live `DocState` variants.
struct ToileGen {
    name: &'static str,
    seed: u64,
}

impl FixtureGen for ToileGen {
    fn name(&self) -> &'static str {
        self.name
    }
    fn doc_type(&self) -> DocType {
        DocType::Toile
    }
    fn seed(&self) -> u64 {
        self.seed
    }
    fn generator_version(&self) -> u32 {
        1
    }
    fn build(&self) -> Vec<u8> {
        let mut state = ToileState::with_peer(ambre_core::PeerId(self.seed));
        for i in 0..3 {
            state.handle_command(ToileCommand::CreateElement {
                kind: ElementKind::Shape(ShapeKind::Rect),
                at: Point::new(f64::from(i) * 10.0, 5.0),
                size: Size::new(40.0, 30.0),
                content: InitContent::None,
                fill: None,
                correlate: 0,
            });
        }
        state.handle_command(ToileCommand::CreateElement {
            kind: ElementKind::Frame,
            at: Point::new(-50.0, -50.0),
            size: Size::new(300.0, 200.0),
            content: InitContent::None,
            fill: None,
            correlate: 0,
        });
        state
            .export_ambre_plaintext(seeded_document_id(self.seed))
            .expect("the document reads")
    }
}

const TRAME: TrameGen = TrameGen {
    name: "trame/belt_basic",
    seed: 0x5EED_0001,
    version: 4,
};
const TOILE: ToileGen = ToileGen {
    name: "toile/belt_basic",
    seed: 0x5EED_0002,
};

// -----------------------------------------------------------------------------
// A throwaway corpus directory
// -----------------------------------------------------------------------------

/// A directory removed when the test drops it. Local rather than a `tempfile`
/// dependency. That is explicit that this component adds no external crate. A
/// test needing one would be a signal it is reaching outside the engine.
struct TempCorpus(PathBuf);

impl TempCorpus {
    fn new(tag: &str) -> Self {
        // Unique per process *and* per tag, so the parallel test harness cannot
        // have two cases writing the same tree.
        let path =
            std::env::temp_dir().join(format!("ambre-fixtures-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("a writable temp dir");
        Self(path)
    }
    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempCorpus {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

// -----------------------------------------------------------------------------
// Manifest integrity
// -----------------------------------------------------------------------------

#[test]
fn the_manifest_describes_the_artifact_beside_it() {
    let dir = TempCorpus::new("manifest");
    let manifest = write_fixture(&TRAME, dir.path()).expect("the generator builds and writes");

    let bytes = std::fs::read(fixtures::artifact_path(dir.path(), TRAME.name())).expect("artifact");

    // `digest` matches the whole `.ambre` file, header included (C8).
    assert_eq!(manifest.digest, digest_of(&bytes));
    // `ambre_version` matches the version the artifact itself carries.
    assert_eq!(manifest.ambre_version, AMBRE_VERSION);
    assert_eq!(manifest.ambre_version, bytes[4]);
    // The identity fields are the generator's, copied verbatim -- the driver has
    // no other source for them.
    assert_eq!(manifest.generator, TRAME.name());
    assert_eq!(manifest.generator_version, TRAME.generator_version());
    assert_eq!(manifest.seed, TRAME.seed());
    assert_eq!(manifest.doc_type, DocType::Trame);
    // `schema_version` is read back from the document, not declared, so the
    // manifest cannot name a version its artifact does not hold (`SCHEMA-004`).
    assert_eq!(
        manifest.schema_version,
        ambre_trame::logic::schema::SCHEMA_VERSION
    );

    // `shape` matches `shape_of` recomputed from the decoded state, which is what
    // catches a manifest committed out of sync with its artifact.
    let (_id, state) = load_fixture_from(dir.path(), TRAME.name()).expect("loads");
    assert_eq!(manifest.shape, shape_of(&state));
}

#[test]
fn the_sidecar_is_readable_json_and_reads_back_equal() {
    let dir = TempCorpus::new("sidecar");
    let written = write_fixture(&TOILE, dir.path()).expect("writes");
    let path = fixtures::manifest_path(dir.path(), TOILE.name());
    let text = std::fs::read_to_string(&path).expect("a sidecar");

    // The whole reason the sidecar is JSON: a reviewer reads this diff.
    assert!(
        text.contains("\"generator\": \"toile/belt_basic\""),
        "{text}"
    );
    assert!(text.ends_with('\n'), "a text file ends with a newline");
    assert!(
        !text.contains("[\n"),
        "the digest must be a hex string, not a byte array: {text}"
    );

    assert_eq!(read_manifest(dir.path(), TOILE.name()).unwrap(), written);
}

#[test]
fn a_flipped_header_byte_moves_the_digest() {
    // The write-once check of works only if the digest covers the header, so
    // this is the assertion that C8's "header included" is real.
    let bytes = TRAME.build();
    let mut tampered = bytes.clone();
    tampered[10] ^= 0x01; // inside `document_id`, not the payload
    assert_ne!(digest_of(&bytes), digest_of(&tampered));
}

#[test]
fn a_generator_whose_doc_type_contradicts_its_bytes_is_refused() {
    // The driver decodes what it just generated. A generator that declared
    // `Toile` and emitted a Trame container is caught before anything is
    // written. It is not committed and discovered later by a confused importer.
    struct Liar;
    impl FixtureGen for Liar {
        fn name(&self) -> &'static str {
            "liar"
        }
        fn doc_type(&self) -> DocType {
            DocType::Toile
        }
        fn seed(&self) -> u64 {
            1
        }
        fn generator_version(&self) -> u32 {
            1
        }
        fn build(&self) -> Vec<u8> {
            TRAME.build()
        }
    }
    assert!(matches!(
        manifest_for(&Liar, &Liar.build()),
        Err(FixtureError::Container(DocFileError::WrongDocType {
            found: DocType::Trame
        }))
    ));
}

// -----------------------------------------------------------------------------
// The driver
// -----------------------------------------------------------------------------

#[test]
fn the_driver_walks_a_whole_registry_and_creates_its_subdirectories() {
    let dir = TempCorpus::new("driver");
    let registry: &[&dyn FixtureGen] = &[&TRAME, &TOILE];
    let manifests = generate_into(registry, dir.path()).expect("the registry runs");

    assert_eq!(manifests.len(), 2, "one manifest per generator, in order");
    assert_eq!(manifests[0].generator, TRAME.name());
    assert_eq!(manifests[1].generator, TOILE.name());
    for generator in registry {
        // A generator name carries a `/`, which is a subdirectory the driver
        // creates. It is not an escape. It is not a flat filename with a slash
        // in it.
        assert!(fixtures::artifact_path(dir.path(), generator.name()).is_file());
        assert!(fixtures::manifest_path(dir.path(), generator.name()).is_file());
    }
    assert!(dir.path().join("trame").is_dir());
}

#[test]
fn regenerating_rewrites_the_same_bytes() {
    // The drift gate is a byte diff, so this is the property it rests on: a
    // second run over an existing tree reproduces it exactly.
    let dir = TempCorpus::new("regen");
    let first = write_fixture(&TRAME, dir.path()).expect("writes");
    let first_bytes = std::fs::read(fixtures::artifact_path(dir.path(), TRAME.name())).unwrap();
    let second = write_fixture(&TRAME, dir.path()).expect("rewrites");
    let second_bytes = std::fs::read(fixtures::artifact_path(dir.path(), TRAME.name())).unwrap();
    assert_eq!(first, second);
    assert_eq!(first_bytes, second_bytes);
}

#[test]
fn the_shipping_registry_has_no_duplicate_names() {
    // A name is the artifact stem, so two generators sharing one would silently
    // overwrite each other's fixture on every run. The emptiness check is part of
    // the rule here: an empty registry passes the dedup and verifies nothing.
    let mut names: Vec<&str> = fixtures::REGISTRY.iter().map(|g| g.name()).collect();
    assert!(!names.is_empty(), "an empty registry verifies nothing here");
    let count = names.len();
    names.sort_unstable();
    names.dedup();
    assert_eq!(names.len(), count, "two generators share a name");
}

// -----------------------------------------------------------------------------
// The `load_fixture` belt
// -----------------------------------------------------------------------------

#[test]
fn a_written_fixture_loads_back_through_the_production_importer() {
    let dir = TempCorpus::new("load");
    write_fixture(&TRAME, dir.path()).expect("writes");
    let (id, state) = load_fixture_from(dir.path(), TRAME.name()).expect("loads");
    // The `document_id` survives the round trip and is the one the seed derives.
    assert_eq!(id, seeded_document_id(TRAME.seed()));
    assert_eq!(state.doc_type(), DocType::Trame);
}

#[test]
fn a_missing_name_is_not_found_not_an_io_error() {
    let dir = TempCorpus::new("missing");
    match load_fixture_from(dir.path(), "trame/no_such_thing") {
        Err(FixtureError::NotFound { name }) => assert_eq!(name, "trame/no_such_thing"),
        other => panic!("expected NotFound, got {other:?}"),
    }
    match read_manifest(dir.path(), "trame/no_such_thing") {
        Err(FixtureError::NotFound { .. }) => {}
        other => panic!("expected NotFound, got {other:?}"),
    }
}

#[test]
fn an_unreadable_corpus_root_is_an_io_error() {
    // Distinct from `NotFound`: "the corpus has no such fixture" and "the corpus
    // is unreachable" are different problems and lead to different fixes.
    let dir = TempCorpus::new("unreadable");
    // A file where a directory is expected: reading `<file>/x.ambre` fails with
    // NotADirectory, never with NotFound.
    let as_file = dir.path().join("not-a-dir");
    std::fs::write(&as_file, b"x").unwrap();
    match load_fixture_from(&as_file, "anything") {
        Err(FixtureError::Io(_)) => {}
        other => panic!("expected Io, got {other:?}"),
    }
}

#[test]
fn a_container_failure_arrives_intact_never_flattened_into_a_string() {
    // The loader wraps `DocFileError`; it does not reinterpret it. A caller that
    // wants to tell a bad magic from an unsupported version can still do so.
    let dir = TempCorpus::new("container");
    std::fs::create_dir_all(dir.path().join("trame")).unwrap();
    // Long enough to clear `HEADER_LEN`, so the failure is the magic and not a
    // truncation. The two are different conditions, and the belt names one.
    std::fs::write(
        fixtures::artifact_path(dir.path(), "trame/soup"),
        b"XXXX definitely not an ambre container at all",
    )
    .unwrap();
    match load_fixture_from(dir.path(), "trame/soup") {
        Err(FixtureError::Container(DocFileError::InvalidMagic)) => {}
        other => panic!("expected Container(InvalidMagic), got {other:?}"),
    }
}

#[test]
fn a_malformed_sidecar_is_reported_as_a_manifest_failure() {
    let dir = TempCorpus::new("badjson");
    std::fs::create_dir_all(dir.path().join("trame")).unwrap();
    std::fs::write(
        fixtures::manifest_path(dir.path(), "trame/bad"),
        b"{ not json",
    )
    .unwrap();
    match read_manifest(dir.path(), "trame/bad") {
        Err(FixtureError::Manifest { name, .. }) => assert_eq!(name, "trame/bad"),
        other => panic!("expected Manifest, got {other:?}"),
    }
}

#[test]
fn the_corpus_root_is_resolved_from_the_manifest_dir_not_the_cwd() {
    // C4/: `cargo test` runs each test binary with the CWD at its own package
    // root. A bare relative `fixtures/` would resolve differently for every
    // consumer crate. The root must be an absolute path under the workspace.
    let root = fixtures::corpus_root().expect("a workspace root above this crate");
    assert!(root.is_absolute(), "{}", root.display());
    assert!(root.ends_with("fixtures"), "{}", root.display());
    assert!(
        root.parent().unwrap().join("Cargo.toml").is_file(),
        "the corpus root must sit beside the workspace manifest"
    );
}

// -----------------------------------------------------------------------------
// Loader rejection, on the byte entry point
// -----------------------------------------------------------------------------

/// Every `.ambre` header failure, plus byte soup, through the fixture loader.
///
/// Asserted here rather than through `load_fixture`, which takes a corpus *name*
/// and has no way to be handed byte soup. `ambre_core::docfile` has its own belt
/// on the same conditions. This one proves the fixture path reports them without
/// panicking and without inventing a variant of its own.
#[test]
fn every_header_failure_is_typed_and_none_of_them_panics() {
    let good = TRAME.build();
    let doc_id = DocumentId::from_bytes([0x33; 16]);

    let cases: Vec<(&str, Vec<u8>, DocFileError)> = vec![
        (
            "truncated header",
            good[..HEADER_LEN - 1].to_vec(),
            DocFileError::TruncatedHeader,
        ),
        ("empty", Vec::new(), DocFileError::TruncatedHeader),
        (
            "bad magic",
            {
                let mut b = good.clone();
                b[0] = b'X';
                b
            },
            DocFileError::InvalidMagic,
        ),
        (
            "newer version",
            {
                let mut b = good.clone();
                b[4] = AMBRE_VERSION + 1;
                b
            },
            DocFileError::UnsupportedVersion {
                found: AMBRE_VERSION + 1,
            },
        ),
        (
            // Older triggers `migrate_ambre`, which has no step for version 0 --
            // nothing precedes 0x01 -- and says so rather than guessing.
            "older version",
            {
                let mut b = good.clone();
                b[4] = 0;
                b
            },
            DocFileError::UnsupportedVersion { found: 0 },
        ),
        (
            "unknown doc type discriminant",
            {
                let mut b = good.clone();
                b[5] = 9;
                b
            },
            DocFileError::UnknownDocType { found: 9 },
        ),
        (
            "valid Trame header, payload the engine refuses",
            write_ambre_plaintext(DocType::Trame, doc_id, b"not a loro snapshot"),
            DocFileError::PayloadRejected,
        ),
        (
            // Both apps, deliberately. Toile reports a refused payload as a
            // display *event* rather than by returning nothing. A loader that
            // inferred rejection from an empty event vector would accept this one
            // and hand back an empty board.
            "valid Toile header, payload the engine refuses",
            write_ambre_plaintext(DocType::Toile, doc_id, b"not a loro snapshot"),
            DocFileError::PayloadRejected,
        ),
    ];

    for (label, bytes, expected) in cases {
        let before = bytes.clone();
        match fixtures::import_fixture(&bytes) {
            Err(FixtureError::Container(actual)) => {
                assert_eq!(actual, expected, "{label}")
            }
            other => panic!("{label}: expected Container({expected:?}), got {other:?}"),
        }
        assert_eq!(bytes, before, "{label}: a failed load rewrote its input");
    }

    // A *valid* container for another app is not a rejection here. The loader is
    // type-agnostic by design: it reads `doc_type` off the header and routes. A
    // Toile artifact loads as a Toile document.
    let (_id, state) = fixtures::import_fixture(&TOILE.build()).expect("a Toile fixture loads");
    assert_eq!(state.doc_type(), DocType::Toile);
}

#[test]
fn a_doc_type_with_no_engine_degrades_cleanly() {
    // Unreachable for any fixture this spec produces, which excludes Murmure.
    // Present so the loader degrades when it arrives: a panic here would be a
    // corpus that cannot be inspected at all.
    let murmure = write_ambre_plaintext(
        DocType::Murmure,
        DocumentId::from_bytes([0x44; 16]),
        b"payload",
    );
    match fixtures::import_fixture(&murmure) {
        Err(FixtureError::NoEngine { doc_type }) => assert_eq!(doc_type, DocType::Murmure),
        other => panic!("expected NoEngine, got {other:?}"),
    }
    // Prose has an engine now: a Prose header over bytes it refuses is a
    // container failure, typed like any other.
    let prose = write_ambre_plaintext(
        DocType::Prose,
        DocumentId::from_bytes([0x44; 16]),
        b"payload",
    );
    match fixtures::import_fixture(&prose) {
        Err(FixtureError::Container(DocFileError::PayloadRejected)) => {}
        other => panic!("expected PayloadRejected, got {other:?}"),
    }
}

// -----------------------------------------------------------------------------
// `shape_of` and `canonical_dump`
// -----------------------------------------------------------------------------

#[test]
fn the_shape_counts_what_the_document_holds() {
    let (_id, trame) = fixtures::import_fixture(&TRAME.build()).expect("loads");
    match shape_of(&trame).body {
        ShapeBody::Trame {
            cells,
            sheets,
            formulas,
        } => {
            assert_eq!(cells, 3, "three written cells");
            assert_eq!(sheets, 1, "the implicit default sheet");
            assert_eq!(formulas, 1, "one `=`-prefixed source");
        }
        other => panic!("a Trame fixture must carry a Trame shape, got {other:?}"),
    }

    let (_id, toile) = fixtures::import_fixture(&TOILE.build()).expect("loads");
    match shape_of(&toile).body {
        ShapeBody::Toile {
            elements,
            frames,
            connectors,
        } => {
            assert_eq!(elements, 4, "three rects and a frame");
            assert_eq!(frames, 1);
            assert_eq!(connectors, 0);
        }
        other => panic!("a Toile fixture must carry a Toile shape, got {other:?}"),
    }
}

#[test]
fn a_solo_document_reports_one_peer_and_a_merged_one_reports_two() {
    let (_id, solo) = fixtures::import_fixture(&TRAME.build()).expect("loads");
    // The importing state commits its own scaffolding under its own peer before
    // the merge. A restored document always carries one peer that is the reader
    // rather than an author. `FIXTURE_IMPORT_PEER` is what lets the count
    // subtract exactly that one, so a solo document still reports one.
    assert_eq!(shape_of(&solo).peers, 1, "one author");

    let mut alice = TrameState::with_peer(ambre_core::PeerId(1));
    let mut bob = TrameState::with_peer(ambre_core::PeerId(2));
    alice.handle_command(TrameCommand::SetCellText {
        row: Row::new(0),
        col: Col::new(0),
        text: "a".to_owned(),
    });
    bob.handle_command(TrameCommand::SetCellText {
        row: Row::new(4),
        col: Col::new(0),
        text: "b".to_owned(),
    });
    alice.apply_remote_snapshot(&bob.export_snapshot());
    let merged = DocState::Trame(Box::new(alice));
    assert_eq!(shape_of(&merged).peers, 2, "two peers' ops are present");
}

#[test]
fn the_canonical_dump_carries_both_representations_of_a_formula() {
    // dumping only the source would let an evaluation regression pass
    // unnoticed; dumping only the result would hide a change in what is
    // persisted. Both go in.
    let (_id, state) = fixtures::import_fixture(&TRAME.build()).expect("loads");
    let dump = canonical_dump(&state);
    assert!(
        dump.contains("raw==A1*2"),
        "the stored source is missing:\n{dump}"
    );
    assert!(
        dump.contains("text=42"),
        "the evaluated result is missing:\n{dump}"
    );
    // A literal carries one representation, and its A1 label, so the diff reads.
    assert!(dump.contains("A1 21"), "{dump}");
    assert!(dump.contains("A3 plain"), "{dump}");
}

#[test]
fn the_canonical_dump_is_blind_to_the_route_a_document_took() {
    // The property the frozen belt rests on, and the reason it rejects a
    // byte or state comparison. Two documents agreeing on every user-visible
    // value still differ in op history, peer ids and commit timestamps.
    let build = |peer: ambre_core::PeerId, reversed: bool| {
        let mut state = TrameState::with_peer(peer);
        let mut writes = vec![(0u32, "first"), (1u32, "second")];
        if reversed {
            writes.reverse();
        }
        for (row, text) in writes {
            state.handle_command(TrameCommand::SetCellText {
                row: Row::new(row),
                col: Col::new(0),
                text: text.to_owned(),
            });
        }
        DocState::Trame(Box::new(state))
    };
    // Different peers, different write order -- same content.
    let one = build(ambre_core::PeerId(11), false);
    let two = build(ambre_core::PeerId(22), true);
    assert_ne!(
        one.export_snapshot(),
        two.export_snapshot(),
        "the two documents must really differ in their bytes, or this proves nothing"
    );
    assert_eq!(canonical_dump(&one), canonical_dump(&two));

    // And a real content difference still shows.
    let three = {
        let mut state = TrameState::with_peer(ambre_core::PeerId(11));
        state.handle_command(TrameCommand::SetCellText {
            row: Row::new(0),
            col: Col::new(0),
            text: "different".to_owned(),
        });
        DocState::Trame(Box::new(state))
    };
    assert_ne!(canonical_dump(&one), canonical_dump(&three));
}

#[test]
fn a_toile_dump_names_nodes_by_paint_order_not_by_element_id() {
    // An `ElementId` is `(peer, counter)` -- exactly what two documents reached by
    // different routes are allowed to differ in. Z-order is user-visible, so it
    // makes the canonical index.
    let build = |peer: ambre_core::PeerId| {
        let mut state = ToileState::with_peer(peer);
        for i in 0..2 {
            state.handle_command(ToileCommand::CreateElement {
                kind: ElementKind::Shape(ShapeKind::Rect),
                at: Point::new(f64::from(i), 0.0),
                size: Size::new(10.0, 10.0),
                content: InitContent::None,
                fill: None,
                correlate: 0,
            });
        }
        DocState::Toile(Box::new(state))
    };
    let one = build(ambre_core::PeerId(101));
    let two = build(ambre_core::PeerId(202));
    assert_ne!(one.export_snapshot(), two.export_snapshot());
    let dump = canonical_dump(&one);
    assert_eq!(dump, canonical_dump(&two));
    assert!(dump.contains("[0] element"), "{dump}");
    assert!(dump.contains("[1] element"), "{dump}");
    // Geometry is formatted shortest-round-trip, not rounded: a migration is not
    // supposed to move it at all.
    assert!(dump.contains("x=1"), "{dump}");
    assert!(
        !dump.contains("00000000000000"),
        "an element id leaked into the dump:\n{dump}"
    );
}
