// crates/ambre-logic/tests/fixtures_gate.rs
// ============================================================================
// The gate belts of `ambre-spec-fixtures.md` section 1.6: byte drift and the
// two membership failures. They also cover the frozen tree -- its semantic
// comparison, the two ways it retires, the write-once rule and the retention
// cap.
//
// Every case drives the gate over a throwaway corpus root, never the committed
// one. A belt that wrote into `fixtures/` would dirty the working copy and hang
// its own verdict on the last run.
//
// Gated like the rest of the fixture code (I4): a bare `cargo test --workspace`
// sees an empty file.
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

use ambre_bus::{CellPos, Col, Row, TrameCommand};
use ambre_core::{ContainerVersion, DocType, FixtureGen, seeded_document_id};
use ambre_logic::fixtures::{
    self, Drift, FROZEN_KEEP_PER_TYPE, REGISTRY, check_corpus, freeze, generate_into,
    retain_frozen, unfrozen_versions, write_fixture,
};
use ambre_trame::TrameState;

// -----------------------------------------------------------------------------
// A registry the test can mutate
// -----------------------------------------------------------------------------

/// A generator whose output the test steers. `filler` counts the literal cells
/// it writes, so one generator stands for two successive revisions of one name
/// -- which is what a drift is.
struct Gen {
    name: &'static str,
    seed: u64,
    version: u32,
    filler: u32,
}

impl FixtureGen for Gen {
    fn name(&self) -> ambre_core::FixtureName {
        ambre_core::FixtureName::declared(self.name)
    }
    fn doc_type(&self) -> DocType {
        DocType::Trame
    }
    fn seed(&self) -> ambre_core::FixtureSeed {
        ambre_core::FixtureSeed::new(self.seed)
    }
    fn generator_version(&self) -> ambre_core::GeneratorVersion {
        ambre_core::GeneratorVersion::new(self.version)
    }
    fn build(&self) -> ambre_core::PlaintextContainer {
        let mut state = TrameState::with_peer(ambre_core::PeerId(self.seed));
        state.handle_command(TrameCommand::SetCellsBatch {
            cells: (0..self.filler)
                .map(|row| {
                    (
                        CellPos::new(Row::new(row), Col::new(0)),
                        format!("row {row}"),
                    )
                })
                .collect(),
        });
        state
            .export_ambre_plaintext(seeded_document_id(ambre_core::FixtureSeed::new(self.seed)))
            .expect("the document reads")
    }
}

const ALPHA: Gen = Gen {
    name: "trame/gate_alpha",
    seed: 0x6A7E_0001,
    version: 1,
    filler: 3,
};
const BETA: Gen = Gen {
    name: "trame/gate_beta",
    seed: 0x6A7E_0002,
    version: 1,
    filler: 2,
};
/// `ALPHA` one revision later: same name and seed, different content.
const ALPHA_EDITED: Gen = Gen { filler: 5, ..ALPHA };
/// `ALPHA` with the version bumped and nothing else, which is what retires a
/// frozen comparison instead of failing it.
const ALPHA_BUMPED: Gen = Gen {
    version: 2,
    ..ALPHA
};

// -----------------------------------------------------------------------------
// A throwaway corpus root
// -----------------------------------------------------------------------------

/// A corpus root removed when the test drops it, with its `current/` tree
/// already created. Local rather than a `tempfile` dependency, like the sibling
/// belt in `fixtures_corpus.rs`.
struct TempRoot(PathBuf);

impl TempRoot {
    fn new(tag: &str) -> Self {
        let path = std::env::temp_dir().join(format!("ambre-gate-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(path.join(fixtures::CURRENT_DIR)).expect("a writable temp dir");
        Self(path)
    }
    fn path(&self) -> &Path {
        &self.0
    }
    fn current(&self) -> PathBuf {
        self.0.join(fixtures::CURRENT_DIR)
    }
    /// A second directory beside the root, for the gate to regenerate into.
    fn scratch(&self) -> PathBuf {
        let path = self.0.join("scratch");
        let _ = std::fs::remove_dir_all(&path);
        path
    }
}

impl Drop for TempRoot {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Generate `registry` into `root/current/` and assert the gate is happy with
/// what it wrote. Every case starts from a clean corpus, so a failure later in
/// the test is the mutation's doing and not the setup's.
fn seed_corpus(root: &TempRoot, registry: &[&dyn FixtureGen]) {
    generate_into(registry, &root.current()).expect("the generators build and write");
    let report =
        check_corpus(registry, root.path(), &root.scratch()).expect("the gate runs over a corpus");
    assert!(report.is_clean(), "freshly generated corpus: {report:?}");
}

// -----------------------------------------------------------------------------
// Byte drift and membership
// -----------------------------------------------------------------------------

/// `FIXTURE-006`. A committed fixture whose bytes drift fails the gate.
#[test]
fn a_generator_that_changes_its_output_drifts_against_the_committed_bytes() {
    let root = TempRoot::new("bytes");
    seed_corpus(&root, &[&ALPHA]);

    // The same name and seed, one revision later: exactly the diff a careless
    // engine change produces.
    let report = check_corpus(&[&ALPHA_EDITED], root.path(), &root.scratch()).expect("runs");

    assert!(!report.is_clean(), "a changed generator must fail the gate");
    assert!(
        report.drifts.iter().any(|d| matches!(
            d,
            Drift::Bytes { name } if name == ALPHA.name().as_str()
        )),
        "expected a byte drift, got {:?}",
        report.drifts
    );
}

/// `FIXTURE-025`. A committed artifact whose generator is gone fails the gate.
#[test]
fn a_committed_artifact_with_no_generator_left_fails_the_gate() {
    let root = TempRoot::new("orphan");
    seed_corpus(&root, &[&ALPHA, &BETA]);

    let report = check_corpus(&[&ALPHA], root.path(), &root.scratch()).expect("runs");

    assert!(
        report.drifts.iter().any(|d| matches!(
            d,
            Drift::Orphan { name } if name == BETA.name().as_str()
        )),
        "expected an orphan, got {:?}",
        report.drifts
    );
}

/// `FIXTURE-031`. A generator with no committed artifact fails the gate. The
/// mirror of the case above, and the one a byte diff alone cannot see.
#[test]
fn a_generator_with_no_committed_artifact_fails_the_gate() {
    let root = TempRoot::new("missing");
    seed_corpus(&root, &[&ALPHA]);

    let report = check_corpus(&[&ALPHA, &BETA], root.path(), &root.scratch()).expect("runs");

    assert!(
        report.drifts.iter().any(|d| matches!(
            d,
            Drift::Missing { name } if name == BETA.name().as_str()
        )),
        "expected a missing artifact, got {:?}",
        report.drifts
    );
}

/// The gate never writes into the tree it judges. A red run leaves the working
/// copy as it found it, which is what lets CI run it on a checkout.
#[test]
fn a_red_gate_leaves_the_committed_tree_untouched() {
    let root = TempRoot::new("readonly");
    seed_corpus(&root, &[&ALPHA]);
    let before = std::fs::read(fixtures::artifact_path(
        &root.current(),
        ALPHA.name().as_str(),
    ))
    .expect("the committed artifact");

    let report = check_corpus(&[&ALPHA_EDITED], root.path(), &root.scratch()).expect("runs");
    assert!(!report.is_clean());

    let after = std::fs::read(fixtures::artifact_path(
        &root.current(),
        ALPHA.name().as_str(),
    ))
    .expect("the committed artifact");
    assert_eq!(before, after, "the gate rewrote the corpus it was judging");
}

// -----------------------------------------------------------------------------
// The frozen tree
// -----------------------------------------------------------------------------

/// `FIXTURE-019`. A migrated frozen artifact produces the canonical dump of the
/// live artifact its manifest names, and the gate counts the comparison held.
#[test]
fn a_frozen_artifact_dumps_like_the_live_one_it_names() {
    let root = TempRoot::new("frozen-holds");
    seed_corpus(&root, &[&ALPHA]);
    freeze(root.path(), ALPHA.name().as_str()).expect("the artifact freezes");

    let report = check_corpus(&[&ALPHA], root.path(), &root.scratch()).expect("runs");

    assert!(report.is_clean(), "{report:?}");
    assert_eq!(report.frozen_held, 1, "the comparison must be counted");
}

/// The same comparison, failing. The live artifact drifted away from the frozen
/// one under an unchanged `generator_version`. That is the migration break the
/// frozen tree exists to catch.
#[test]
fn a_frozen_artifact_that_no_longer_matches_the_live_one_fails_the_gate() {
    let root = TempRoot::new("frozen-breaks");
    seed_corpus(&root, &[&ALPHA]);
    freeze(root.path(), ALPHA.name().as_str()).expect("the artifact freezes");

    // Regenerate `current/` from the edited generator. That settles the byte
    // drift and leaves the frozen comparison as the only thing left to fail.
    generate_into(&[&ALPHA_EDITED], &root.current()).expect("regenerates");
    let report = check_corpus(&[&ALPHA_EDITED], root.path(), &root.scratch()).expect("runs");

    assert!(
        report
            .drifts
            .iter()
            .any(|d| matches!(d, Drift::Frozen { .. })),
        "expected a frozen drift, got {:?}",
        report.drifts
    );
    assert_eq!(report.frozen_held, 0);
}

/// `FIXTURE-020`. The gate retires the comparison instead of failing it when
/// the frozen manifest and the live generator disagree on `generator_version`.
#[test]
fn a_generator_version_bump_retires_the_frozen_comparison() {
    let root = TempRoot::new("frozen-bumped");
    seed_corpus(&root, &[&ALPHA]);
    freeze(root.path(), ALPHA.name().as_str()).expect("the artifact freezes");

    // A contributor commits a bump like any other change. `just fixtures`
    // rewrites the sidecar, the only part of `current/` a bare bump touches.
    generate_into(&[&ALPHA_BUMPED], &root.current()).expect("regenerates");
    let report = check_corpus(&[&ALPHA_BUMPED], root.path(), &root.scratch()).expect("runs");

    assert!(
        report.is_clean(),
        "a bump must not fail the gate: {report:?}"
    );
    assert_eq!(report.degraded.len(), 1, "{report:?}");
    assert_eq!(report.frozen_held, 0, "the comparison is no longer held");
}

/// `FIXTURE-026`. A frozen artifact whose generator is gone reports as
/// degraded, and the next retention pass drops it.
#[test]
fn a_frozen_artifact_whose_generator_vanished_degrades_then_drops() {
    let root = TempRoot::new("frozen-gone");
    seed_corpus(&root, &[&ALPHA]);
    let frozen = freeze(root.path(), ALPHA.name().as_str()).expect("the artifact freezes");

    // The generator is gone, so the live artifact goes with it.
    std::fs::remove_file(fixtures::artifact_path(
        &root.current(),
        ALPHA.name().as_str(),
    ))
    .expect("removes");
    std::fs::remove_file(fixtures::manifest_path(
        &root.current(),
        ALPHA.name().as_str(),
    ))
    .expect("removes");

    let report = check_corpus(&[], root.path(), &root.scratch()).expect("runs");
    assert_eq!(report.degraded.len(), 1, "{report:?}");
    assert!(frozen.exists(), "the gate must not delete anything");

    let dropped = retain_frozen(&[], root.path()).expect("the retention pass runs");
    assert_eq!(dropped, vec![frozen.clone()]);
    assert!(!frozen.exists(), "retention must drop it");
}

/// `FIXTURE-018`. The freeze recipe writes a frozen artifact once. A second
/// freeze of the same name at that version pair fails instead of overwriting.
/// The bytes stay a record of what the version produced.
#[test]
fn freezing_the_same_version_pair_twice_is_refused() {
    let root = TempRoot::new("write-once");
    seed_corpus(&root, &[&ALPHA]);
    let frozen = freeze(root.path(), ALPHA.name().as_str()).expect("the first freeze writes");
    let bytes = std::fs::read(&frozen).expect("the frozen artifact");

    // A later revision of the same generator at the same version pair: the one
    // case where an overwrite would destroy the record.
    generate_into(&[&ALPHA_EDITED], &root.current()).expect("regenerates");
    let second = freeze(root.path(), ALPHA.name().as_str());

    assert!(second.is_err(), "a second freeze must be refused");
    assert_eq!(
        std::fs::read(&frozen).expect("the frozen artifact"),
        bytes,
        "the frozen bytes were overwritten"
    );
}

/// `FIXTURE-021`. The frozen set keeps the newest three artifacts per document
/// type and drops the rest. That cap holds the gate's load-and-migrate cost
/// inside its budget as version bumps accumulate.
#[test]
fn retention_keeps_the_newest_three_frozen_artifacts_per_doc_type() {
    let root = TempRoot::new("retention");
    seed_corpus(&root, &[&ALPHA]);
    let frozen_dir = root.path().join(fixtures::FROZEN_DIR);

    // Five schema versions of one generator, oldest to newest. Written by hand
    // rather than by five freezes: `freeze` derives the suffix from the
    // manifest, and nothing here can bump a schema version.
    let stems: Vec<String> = (1..=5)
        .map(|s| format!("{}@v1-s{s}", ALPHA.name()))
        .collect();
    for stem in &stems {
        std::fs::create_dir_all(
            fixtures::artifact_path(&frozen_dir, stem)
                .parent()
                .expect("a parent"),
        )
        .expect("creates the frozen tree");
        std::fs::copy(
            fixtures::artifact_path(&root.current(), ALPHA.name().as_str()),
            fixtures::artifact_path(&frozen_dir, stem),
        )
        .expect("copies the artifact");
        std::fs::copy(
            fixtures::manifest_path(&root.current(), ALPHA.name().as_str()),
            fixtures::manifest_path(&frozen_dir, stem),
        )
        .expect("copies the sidecar");
    }

    // Over the cap, the gate says so rather than deleting behind the
    // contributor's back.
    let report = check_corpus(&[&ALPHA], root.path(), &root.scratch()).expect("runs");
    assert!(
        report.drifts.iter().any(|d| matches!(
            d,
            Drift::FrozenOverflow { kept, .. } if *kept == stems.len()
        )),
        "expected an overflow, got {:?}",
        report.drifts
    );

    let dropped = retain_frozen(&[&ALPHA], root.path()).expect("the retention pass runs");

    assert_eq!(dropped.len(), stems.len() - FROZEN_KEEP_PER_TYPE);
    for stem in &stems[..stems.len() - FROZEN_KEEP_PER_TYPE] {
        assert!(
            !fixtures::artifact_path(&frozen_dir, stem).exists(),
            "{stem} is older than the cap and must be gone"
        );
    }
    for stem in &stems[stems.len() - FROZEN_KEEP_PER_TYPE..] {
        assert!(
            fixtures::artifact_path(&frozen_dir, stem).exists(),
            "{stem} is within the cap and must be kept"
        );
    }
}

/// Retention orders by the version pair as two numbers, not as text. Ten schema
/// bumps in, a text order reads `s10` as older than `s9`. It would then drop the
/// newest artifact of the set, the one migration coverage needs most.
#[test]
fn retention_reads_the_version_suffix_as_numbers_not_as_text() {
    let root = TempRoot::new("retention-order");
    seed_corpus(&root, &[&ALPHA]);
    let frozen_dir = root.path().join(fixtures::FROZEN_DIR);

    let stems: Vec<String> = [2u32, 9, 10, 11]
        .iter()
        .map(|s| format!("{}@v1-s{s}", ALPHA.name()))
        .collect();
    for stem in &stems {
        std::fs::create_dir_all(
            fixtures::artifact_path(&frozen_dir, stem)
                .parent()
                .expect("a parent"),
        )
        .expect("creates the frozen tree");
        std::fs::copy(
            fixtures::artifact_path(&root.current(), ALPHA.name().as_str()),
            fixtures::artifact_path(&frozen_dir, stem),
        )
        .expect("copies the artifact");
        std::fs::copy(
            fixtures::manifest_path(&root.current(), ALPHA.name().as_str()),
            fixtures::manifest_path(&frozen_dir, stem),
        )
        .expect("copies the sidecar");
    }

    retain_frozen(&[&ALPHA], root.path()).expect("the retention pass runs");

    assert!(
        !fixtures::artifact_path(&frozen_dir, &stems[0]).exists(),
        "s2 is the oldest of the four and must be the one dropped"
    );
    for stem in &stems[1..] {
        assert!(
            fixtures::artifact_path(&frozen_dir, stem).exists(),
            "{stem} must be kept"
        );
    }
}

/// Copy `ALPHA`'s live artifact into `frozen/` under `stem`, as a freeze at
/// that version pair would. By hand, because nothing here can bump a version.
fn plant_frozen(root: &TempRoot, stem: &str) {
    let frozen_dir = root.path().join(fixtures::FROZEN_DIR);
    let target = fixtures::artifact_path(&frozen_dir, stem);
    std::fs::create_dir_all(target.parent().expect("a parent")).expect("creates the tree");
    std::fs::copy(
        fixtures::artifact_path(&root.current(), ALPHA.name().as_str()),
        &target,
    )
    .expect("copies the artifact");
    std::fs::copy(
        fixtures::manifest_path(&root.current(), ALPHA.name().as_str()),
        fixtures::manifest_path(&frozen_dir, stem),
    )
    .expect("copies the sidecar");
}

/// `DOCFILE-018`. A container bump that skipped its freeze fails the gate, by
/// version and by document type. The freeze clears it, and a document type the
/// registry does not generate owes nothing.
#[test]
fn a_superseded_container_version_with_no_frozen_artifact_fails_the_gate() {
    let root = TempRoot::new("unfrozen");
    seed_corpus(&root, &[&ALPHA]);

    let owed = unfrozen_versions(&[&ALPHA], root.path(), ContainerVersion::from_byte(2))
        .expect("the check runs");
    assert_eq!(
        owed,
        vec![Drift::Unfrozen {
            doc_type: DocType::Trame,
            ambre_version: ContainerVersion::from_byte(1),
        }]
    );

    freeze(root.path(), ALPHA.name().as_str()).expect("the artifact freezes");
    let owed = unfrozen_versions(&[&ALPHA], root.path(), ContainerVersion::from_byte(2))
        .expect("the check runs");
    assert!(owed.is_empty(), "the freeze pays the version: {owed:?}");
}

/// `DOCFILE-018`. A version owes an artifact only while retention could keep
/// it. Three newer artifacts of one document type release the older versions,
/// and a version between them that holds none is still owed.
#[test]
fn a_version_retention_has_moved_past_owes_nothing() {
    let root = TempRoot::new("unfrozen-window");
    seed_corpus(&root, &[&ALPHA]);
    for stem in ["@v2-s1", "@v4-s1", "@v4-s2"] {
        plant_frozen(&root, &format!("{}{stem}", ALPHA.name()));
    }

    let owed = unfrozen_versions(&[&ALPHA], root.path(), ContainerVersion::from_byte(5))
        .expect("the check runs");
    assert_eq!(
        owed,
        vec![Drift::Unfrozen {
            doc_type: DocType::Trame,
            ambre_version: ContainerVersion::from_byte(3),
        }],
        "version 1 sits behind three newer artifacts, and version 3 holds none"
    );
}

// -----------------------------------------------------------------------------
// The shipping registry
// -----------------------------------------------------------------------------

/// Every generator in the shipping registry builds an artifact its own manifest
/// describes. The driver's round trip then loads it back. `just fixtures-check`
/// compares the corpus itself; this belt is what a contributor sees fail first.
#[test]
fn every_shipping_generator_builds_and_round_trips() {
    let root = TempRoot::new("registry");
    for generator in REGISTRY {
        let manifest = write_fixture(*generator, &root.current()).expect("builds and writes");
        assert_eq!(manifest.generator, generator.name());
        assert_eq!(manifest.doc_type, generator.doc_type());
        assert_eq!(manifest.seed, generator.seed());
    }
}
