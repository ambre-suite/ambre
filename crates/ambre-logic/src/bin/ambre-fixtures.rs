// crates/ambre-logic/src/bin/ambre-fixtures.rs
// ============================================================================
// The fixture corpus driver (`ambre-spec-fixtures.md` section 3).
//
// Four subcommands, one per recipe:
//
//   * `generate` rewrites `fixtures/current/`;
//   * `check` compares a regeneration in a temporary tree;
//   * `freeze <name>` copies one artifact into `frozen/`;
//   * `bench-corpus` builds the scale corpora into `fixtures/bench/`.
//
// A binary rather than a test, because `just fixtures` writes the working copy
// and a test harness must not. Four fixed words and one name need no argument
// parser, and a parser would ride into the workspace for them.
//
// Fixture code (I4): the crate surface it calls sits behind the `fixtures`
// feature, and `required-features` keeps this binary out of any other build.
// ============================================================================

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use ambre_logic::fixtures::{
    self, BASELINE_DIR, BENCH_DIR, BENCH_REGISTRY, BenchBaseline, CORPUS_DIR, CURRENT_DIR,
    CorpusReport, FixtureError, REGISTRY,
};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("generate") => generate(),
        Some("check") => check(),
        Some("bench-corpus") => bench_corpus(),
        Some("baseline") => match args.get(1) {
            Some(machine) => baseline(machine),
            None => {
                eprintln!("baseline needs the machine the figures were measured on");
                return ExitCode::FAILURE;
            }
        },
        Some("freeze") => match args.get(1) {
            Some(name) => freeze(name),
            None => {
                eprintln!("freeze needs the name of an artifact in {CORPUS_DIR}/{CURRENT_DIR}");
                return ExitCode::FAILURE;
            }
        },
        _ => {
            eprintln!(
                "usage: ambre-fixtures generate | check | freeze <name> | bench-corpus \
                 | baseline <machine>"
            );
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

/// Build the scale corpora into `bench/`, skipping every artifact already there
/// (`FIXTURE-022`).
///
/// Reports what it built and what it kept, because the two readings differ. A
/// run that builds nothing means the corpus stands. A run that builds
/// everything is the multi-minute first one.
fn bench_corpus() -> Result<ExitCode, FixtureError> {
    let dir = fixtures::corpus_root()?.join(BENCH_DIR);
    std::fs::create_dir_all(&dir)?;
    let built = fixtures::generate_bench_into(BENCH_REGISTRY, &dir)?;
    for name in &built {
        println!("built {name}");
    }
    // The artifact size, per generator. Two of the budgets these corpora serve
    // name megabytes rather than milliseconds. `criterion` measures wall clock
    // alone, so this line is the only reading those two rows have.
    for generator in BENCH_REGISTRY {
        let name = generator.name();
        let bytes = std::fs::metadata(fixtures::artifact_path(&dir, name))?.len();
        println!("{name}: {:.2} MB", bytes as f64 / (1024.0 * 1024.0));
    }
    println!(
        "bench corpus ready in {}: {} built, {} already there",
        display(&dir),
        built.len(),
        BENCH_REGISTRY.len() - built.len()
    );
    Ok(ExitCode::SUCCESS)
}

/// Write the last bench run into a committed baseline named after `machine`
/// (`FIXTURE-023`).
///
/// The machine is an argument rather than something read off the host. A
/// contributor knows their machine, and no portable call answers it. Taking it
/// as a word also puts the rule in front of the author as the file lands.
fn baseline(machine: &str) -> Result<ExitCode, FixtureError> {
    if machine.trim().is_empty() {
        eprintln!("a baseline names the machine it was measured on (FIXTURE-023)");
        return Ok(ExitCode::FAILURE);
    }
    let root = workspace_root()?;
    let measurements = fixtures::criterion_means(&root.join("target").join("criterion"))?;
    if measurements.is_empty() {
        eprintln!("no bench figures under target/criterion. Run `just fixtures-bench` first");
        return Ok(ExitCode::FAILURE);
    }
    let baseline = BenchBaseline {
        machine: machine.to_owned(),
        commit: head_commit(),
        measurements,
    };
    let dir = root.join(BASELINE_DIR);
    std::fs::create_dir_all(&dir)?;
    // Named after the machine, lower-cased with every run of punctuation folded
    // to one dash, so two runs on one machine rewrite one file.
    let stem: String = machine
        .trim()
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .fold(String::new(), |mut acc, c| {
            if c != '-' || !acc.ends_with('-') {
                acc.push(c);
            }
            acc
        });
    let path = dir.join(format!("{}.json", stem.trim_matches('-')));
    let mut json = serde_json::to_string_pretty(&baseline).map_err(|e| FixtureError::Manifest {
        name: path.to_string_lossy().into_owned(),
        reason: e.to_string(),
    })?;
    json.push('\n');
    std::fs::write(&path, json)?;
    println!(
        "wrote {} with {} figures",
        display(&path),
        baseline.measurements.len()
    );
    Ok(ExitCode::SUCCESS)
}

/// The workspace root: the corpus root's parent.
fn workspace_root() -> Result<PathBuf, FixtureError> {
    let corpus = fixtures::corpus_root()?;
    match corpus.parent() {
        Some(root) => Ok(root.to_path_buf()),
        None => Err(FixtureError::Io(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "the corpus root has no parent",
        ))),
    }
}

/// The commit the figures describe, or `unknown` when git does not answer.
///
/// A baseline with no commit is still a baseline. The rule is about the
/// machine, so a checkout without git yields a poorer record, not a refusal.
fn head_commit() -> String {
    std::process::Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .filter(|out| out.status.success())
        .and_then(|out| String::from_utf8(out.stdout).ok())
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".to_owned())
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
