// crates/ambre-logic/tests/fixtures_bench.rs
// ============================================================================
// The belts of the scale corpora (`ambre-spec-fixtures.md` section 3). They
// hold the two rules that matter once a corpus or a figure leaves a developer's
// machine.
//
//   * Neither belt reads the corpus itself. The ignore file covers it, so no CI
//     job has one, and a belt that needed it would never run. These hold what a
//     commit can carry: the ignore entry, and the shape of a baseline.
//   * Gated like the rest of the fixture code (I4). A bare
//     `cargo test --workspace` sees an empty file.
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

use ambre_core::{Ns, Refused, TimeSpan, Validated};
use ambre_logic::fixtures::{
    BASELINE_DIR, BENCH_DIR, BENCH_REGISTRY, BenchBaseline, BenchMeasurement, CORPUS_DIR,
    CommitHash, REGISTRY, baselines_in, corpus_root,
};

/// The workspace root, derived from the corpus root the loader already resolves.
fn workspace_root() -> PathBuf {
    corpus_root()
        .expect("the corpus root resolves from the crate manifest")
        .parent()
        .expect("the corpus root has a parent")
        .to_path_buf()
}

/// The scale corpus is git-ignored, so no commit can carry it (`FIXTURE-022`).
///
/// Reads the ignore file rather than asking git. The rule covers what the
/// repository refuses, and the file is that configuration. Asking git would
/// also demand a git checkout, which a source tarball is not.
#[test]
fn the_scale_corpus_is_ignored_by_git() {
    let ignore = std::fs::read_to_string(workspace_root().join(".gitignore"))
        .expect("the workspace has a .gitignore");
    let entry = format!("{CORPUS_DIR}/{BENCH_DIR}/");
    assert!(
        ignore.lines().any(|line| line.trim() == entry),
        "`.gitignore` must carry `{entry}` so no commit can carry a scale corpus \
         (FIXTURE-022). The corpus is multi-megabyte and rebuilt from a seed by \
         `just fixtures-bench-corpus`"
    );
}

/// Every scale generator writes under the ignored directory, and none writes
/// under the committed one (`FIXTURE-022`).
///
/// The ignore entry covers one directory. A generator whose name escaped it
/// would drop a multi-megabyte artifact into the working copy, and the belt
/// above would stay green.
#[test]
fn every_scale_generator_writes_under_the_ignored_directory() {
    for generator in BENCH_REGISTRY {
        let name = generator.name();
        let name = name.as_str();
        let relative = Path::new(name);
        assert!(
            relative.is_relative() && !name.contains(".."),
            "the scale generator `{name}` must name a path inside the corpus, \
             so the ignore entry covers what it writes (FIXTURE-022)"
        );
    }
}

/// No two generators share a name or a seed, across both registries
/// (`FIXTURE-010`).
///
/// The name is the artifact's identity on disk, and the seed pins the
/// `document_id`. A collision therefore overwrites an artifact, or makes a
/// copied file look plausible. `fixtures_corpus` covers the committed registry.
/// The scale registry doubled the population, and the cross-registry pair
/// reaches neither belt.
#[test]
fn no_generator_shares_a_name_or_a_seed_with_another() {
    let all: Vec<&&dyn ambre_core::FixtureGen> =
        REGISTRY.iter().chain(BENCH_REGISTRY.iter()).collect();
    assert!(
        !BENCH_REGISTRY.is_empty(),
        "an empty scale registry passes every check below on nothing at all"
    );
    for (i, one) in all.iter().enumerate() {
        for other in all.iter().skip(i + 1) {
            assert_ne!(
                one.name().as_str(),
                other.name().as_str(),
                "two generators share the name `{}`, so one overwrites the other",
                one.name().as_str()
            );
            assert_ne!(
                one.seed(),
                other.seed(),
                "`{}` and `{}` share a seed, so their documents share an id",
                one.name().as_str(),
                other.name().as_str()
            );
        }
    }
}

/// A committed baseline names the machine behind its figures (`FIXTURE-023`).
///
/// `criterion` measures wall clock, so a figure from one machine means nothing
/// on another. A baseline read as an absolute target then fails on hardware
/// rather than on code.
///
/// An empty directory passes. No baseline has landed yet, and the rule covers
/// the ones that do.
#[test]
fn every_committed_baseline_names_its_machine() {
    let dir = workspace_root().join(BASELINE_DIR);
    let baselines = baselines_in(&dir).expect("a baseline directory reads back");
    for (path, baseline) in baselines {
        assert!(
            baseline.names_its_machine(),
            "{} names no machine. A `criterion` figure without the hardware it \
             came from is not comparable (FIXTURE-023)",
            path.display()
        );
    }
}

/// A blank machine fails the predicate, which the type alone cannot express.
///
/// The belt above walks the committed set, and that set stays empty until
/// someone lands a baseline. This drives the same predicate over both shapes,
/// so the rule holds before the first one arrives.
#[test]
fn a_blank_machine_is_not_a_named_machine() {
    let measurements = vec![BenchMeasurement {
        id: "import_snapshot/example".to_owned(),
        mean_ns: TimeSpan::<Ns>::new(1_234),
    }];
    let named = BenchBaseline {
        machine: "Ryzen 7 5800X, 32 GB, Linux 6.18".to_owned(),
        commit: Validated::<CommitHash>::parse("abc1234").ok(),
        measurements: measurements.clone(),
    };
    assert!(named.names_its_machine());
    for blank in ["", "   ", "\t"] {
        let unnamed = BenchBaseline {
            machine: blank.to_owned(),
            commit: Validated::<CommitHash>::parse("abc1234").ok(),
            measurements: measurements.clone(),
        };
        assert!(
            !unnamed.names_its_machine(),
            "a blank machine is not a named machine (FIXTURE-023)"
        );
    }
}

/// A baseline round-trips through its committed encoding.
///
/// A reviewer reads the file in a pull request, and so does the belt above. A
/// field that did not survive the encoding would leave the rule standing over a
/// value nobody wrote.
#[test]
fn a_baseline_round_trips_through_its_file() {
    let baseline = BenchBaseline {
        machine: "Ryzen 7 5800X, 32 GB, Linux 6.18".to_owned(),
        commit: Validated::<CommitHash>::parse("abc1234").ok(),
        measurements: vec![BenchMeasurement {
            id: "toile_viewport/toile/bench_board_50k".to_owned(),
            mean_ns: TimeSpan::<Ns>::new(5_062_900_000),
        }],
    };
    let dir = std::env::temp_dir().join(format!("ambre-baseline-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a scratch directory");
    let mut json = serde_json::to_string_pretty(&baseline).expect("a baseline encodes");
    json.push('\n');
    std::fs::write(dir.join("ryzen-7-5800x.json"), json).expect("the scratch file writes");

    let read = baselines_in(&dir).expect("the scratch directory reads back");
    assert_eq!(read.len(), 1, "one baseline in, one out");
    assert_eq!(read[0].1, baseline, "every field survives the encoding");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A commit id is 4 to 40 lowercase hex digits. The refusal echoes the text,
/// and `Debug` prints the id under its old name.
#[test]
fn a_commit_id_is_four_to_forty_lowercase_hex_digits() {
    let id = Validated::<CommitHash>::parse("abc1234").expect("a short id");
    assert_eq!(format!("{id:?}"), "CommitHash(\"abc1234\")");
    assert!(Validated::<CommitHash>::parse("a".repeat(40)).is_ok());
    for bad in ["abc", "ABC1234", "abc123g", &"a".repeat(41)] {
        assert_eq!(
            Validated::<CommitHash>::parse(bad),
            Err(Refused::<CommitHash>::new(bad.to_owned())),
            "{bad}"
        );
    }
    let refused = Validated::<CommitHash>::parse("abc").expect_err("too short");
    assert_eq!(refused.to_string(), "\"abc\" is not a commit id");
    assert_eq!(format!("{refused:?}"), "InvalidCommitHash(\"abc\")");
}
