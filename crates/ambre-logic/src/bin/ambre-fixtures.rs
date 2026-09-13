// crates/ambre-logic/src/bin/ambre-fixtures.rs
// ============================================================================
// The fixture corpus driver (`ambre-spec-fixtures.md` section 3).
//
// Three subcommands, one per recipe: `generate` rewrites `fixtures/current/`;
// `check` compares a regeneration in a temporary tree; `freeze <name>` copies
// one artifact into `frozen/`.
// A binary rather than a test, because `just fixtures` writes the working copy
// and a test harness must not. Three fixed words and one name need no argument
// parser, and a parser would ride into the workspace for them.
//
// Fixture code (I4): the crate surface it calls sits behind the `fixtures`
// feature, and `required-features` keeps this binary out of any other build.
// ============================================================================

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use ambre_logic::fixtures::{self, CORPUS_DIR, CURRENT_DIR, CorpusReport, FixtureError, REGISTRY};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("generate") => generate(),
        Some("check") => check(),
        Some("freeze") => match args.get(1) {
            Some(name) => freeze(name),
            None => {
                eprintln!("freeze needs the name of an artifact in {CORPUS_DIR}/{CURRENT_DIR}");
                return ExitCode::FAILURE;
            }
        },
        _ => {
            eprintln!("usage: ambre-fixtures generate | check | freeze <name>");
            return ExitCode::FAILURE;
        }
    };
    match result {
        Ok(code) => code,
        Err(e) => {
            eprintln!("fixtures: {e}");
            ExitCode::FAILURE
        }
    }
}

/// Rewrite `current/` from the live registry. Then restore the two rules a
/// regeneration can: no orphan artifact, and no frozen artifact past
/// retention.
fn generate() -> Result<ExitCode, FixtureError> {
    let root = fixtures::corpus_root()?;
    let current = root.join(CURRENT_DIR);
    std::fs::create_dir_all(&current)?;
    let manifests = fixtures::generate_into(REGISTRY, &current)?;
    for manifest in &manifests {
        println!("wrote {}", manifest.generator);
    }
    for path in fixtures::prune_orphans(REGISTRY, &current)? {
        println!("dropped orphan {}", display(&path));
    }
    for path in fixtures::retain_frozen(REGISTRY, &root)? {
        println!("dropped frozen {}", display(&path));
    }
    Ok(ExitCode::SUCCESS)
}

/// Regenerate into a temporary tree and compare. The working copy stays as it
/// stands, even when the gate goes red.
fn check() -> Result<ExitCode, FixtureError> {
    let root = fixtures::corpus_root()?;
    let scratch = std::env::temp_dir().join(format!("ambre-fixtures-check-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&scratch);
    std::fs::create_dir_all(&scratch)?;
    let report = fixtures::check_corpus(REGISTRY, &root, &scratch);
    let _ = std::fs::remove_dir_all(&scratch);
    Ok(report_outcome(report?))
}

fn report_outcome(report: CorpusReport) -> ExitCode {
    for degraded in &report.degraded {
        println!("degraded frozen/{}: {}", degraded.stem, degraded.reason);
    }
    if report.is_clean() {
        println!(
            "corpus clean: {} generators, {} frozen comparisons held",
            REGISTRY.len(),
            report.frozen_held
        );
        return ExitCode::SUCCESS;
    }
    for drift in &report.drifts {
        eprintln!("drift {drift}");
    }
    eprintln!(
        "{} drifts. Run `just fixtures` and review the manifest diff.",
        report.drifts.len()
    );
    ExitCode::FAILURE
}

/// Copy one artifact into `frozen/`, then run the retention pass. The cap holds
/// at the moment the set grows, not at the next regeneration.
fn freeze(name: &str) -> Result<ExitCode, FixtureError> {
    let root = fixtures::corpus_root()?;
    let path = fixtures::freeze(&root, name)?;
    println!("froze {}", display(&path));
    for dropped in fixtures::retain_frozen(REGISTRY, &root)? {
        println!("dropped frozen {}", display(&dropped));
    }
    Ok(ExitCode::SUCCESS)
}

/// A path as text. Lossy is unreachable for a tree this binary wrote, and a
/// report is not the place to refuse a name.
fn display(path: &Path) -> String {
    PathBuf::from(path).to_string_lossy().into_owned()
}
