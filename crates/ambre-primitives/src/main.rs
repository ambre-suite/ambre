//! The primitive gate (`ambre-spec-testing.md` section 2).
//!
//! Every site of the workspace whose type holds a bare primitive is typed. The
//! exception is a site that one line of the allowlist names, with the reason
//! it may keep the primitive (`ai/ambre-rust-style.md`). A model writing code
//! copies the primitive it sees in the next signature, so a review rule alone
//! loses ground on every change. The gate makes the count only go down.
//!
//! The gate reads the code through rustc. It runs the compile lines of the
//! `lint` and `lint-wasm` recipes with the driver of `driver/` in place of
//! rustc (`run.rs`). The driver writes the sites and the facts of each one;
//! this binary judges them against the allowlists. Three modes:
//!
//!   * `check`, the default, fails on each defect of these checks, then prints
//!     the lines per category:
//!     * a site no line names, and a line that names no site (`ARCH-033`,
//!       `ARCH-034`);
//!     * a claimed shape the site lacks, and the three privacy refusals
//!       (`ARCH-036` to `ARCH-040`);
//!     * a `math` line on a name that ends on a unit (`ARCH-084`);
//!     * each refusal the code alone settles (`ARCH-037`, `ARCH-045` to
//!       `ARCH-049`, `NEWTYPE-001`). A `Deref` to a primitive is one, and a
//!       secret read or wiped outside its module is another. So are a sink
//!       outside its file and a public field of a tuple struct;
//!     * the newtype census (`ARCH-062`) and the bare-alias check
//!       (`ARCH-066`);
//!     * a feature no compile turns on, a `.rs` file no compile reads, and a
//!       stray workspace (`features.rs`, `files.rs`);
//!     * the crate boundaries (`boundaries.rs`). The driver refuses a
//!       resolved path, the judge reads the manifests from `cargo metadata`,
//!       and refuses a wire type the worker or the relay redeclares. It also
//!       counts the facts of the launcher's wiring over every unit;
//!   * `check --unwrap-strict` also runs the strict unwrap report on the same
//!     records, and prints both verdicts. It fails when either fails;
//!   * `--prune` removes the lines that name no site, and the exception lines
//!     that name no newtype, and nothing else;
//!   * `unwrap` reports each workspace newtype unwrapped before a workspace
//!     callee (`ARCH-073`, `unwraps.rs`). `--strict` also fails on a refusal
//!     no line names, and on a line whose count differs from its sites.
//!
//! `--records <dir>` judges the records of an earlier run and compiles
//! nothing, for a run by hand.

mod aliases;
mod allowlist;
mod boundaries;
mod features;
mod files;
mod newtypes;
mod records;
mod run;
mod units;
mod unwraps;
mod views;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use allowlist::{Allowlist, Category, Defect, HEADER};
use records::Records;
use unwraps::{Strictness, UNWRAP_ALLOWLIST};

/// The allowlist, relative to the workspace root. It sits beside the other
/// files the gates of the corpus read.
const ALLOWLIST: &str = "specs/1-Foundations/ambre-primitives.tsv";

/// What one run does.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Check(Report),
    Prune,
    Unwrap(Strictness),
}

/// What `check` reports.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Report {
    /// The primitive gate alone.
    Primitives,
    /// The primitive gate, then the strict unwrap report on the same records.
    WithStrictUnwrap,
}

/// The verdict of one report.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Verdict {
    Pass,
    Fail,
}

impl Verdict {
    /// Fails when either verdict fails.
    fn and(self, other: Self) -> Self {
        match (self, other) {
            (Self::Pass, Self::Pass) => Self::Pass,
            _ => Self::Fail,
        }
    }

    /// The word the summary line prints.
    fn word(self) -> &'static str {
        match self {
            Self::Pass => "passes",
            Self::Fail => "fails",
        }
    }

    fn exit(self) -> ExitCode {
        match self {
            Self::Pass => ExitCode::SUCCESS,
            Self::Fail => ExitCode::FAILURE,
        }
    }
}

const USAGE: &str = "usage: ambre-primitives [check [--unwrap-strict] | --prune | unwrap \
                     [--strict]] [--records <dir>]";

/// The mode and the records directory the arguments name.
fn arguments(args: &[String]) -> Result<(Mode, Option<PathBuf>), String> {
    let mut records = None;
    let mut words = Vec::new();
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        if arg == "--records" {
            let dir = iter.next().ok_or("--records takes a directory")?;
            records = Some(PathBuf::from(dir));
        } else {
            words.push(arg.as_str());
        }
    }
    let mode = match words.as_slice() {
        [] | ["check"] => Mode::Check(Report::Primitives),
        ["check", "--unwrap-strict"] => Mode::Check(Report::WithStrictUnwrap),
        ["--prune"] => Mode::Prune,
        ["unwrap"] => Mode::Unwrap(Strictness::Report),
        ["unwrap", "--strict"] => Mode::Unwrap(Strictness::Strict),
        _ => return Err(format!("unknown arguments {args:?}")),
    };
    Ok((mode, records))
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (mode, given) = match arguments(&args) {
        Ok(parsed) => parsed,
        Err(why) => {
            eprintln!("primitives: {why} -- {USAGE}");
            return ExitCode::FAILURE;
        }
    };
    let root = workspace_root();
    let (dir, compiled) = match given {
        Some(dir) => (dir, None),
        None => match run::compile(&root) {
            Ok(compiled) => (compiled.records.clone(), Some(compiled)),
            Err(errors) => {
                for error in errors {
                    eprintln!("primitives: {error}");
                }
                return ExitCode::FAILURE;
            }
        },
    };
    let records = Records::read(&dir);
    if !records.errors.is_empty() {
        for error in &records.errors {
            eprintln!("primitives: {error}");
        }
        return ExitCode::FAILURE;
    }
    let verdict = match mode {
        Mode::Unwrap(strictness) => unwraps(&records, &root, strictness),
        Mode::Prune => prune(&records, &root),
        Mode::Check(report) => {
            let primitives = gate(&records, &root, compiled.as_ref());
            match report {
                Report::Primitives => primitives,
                Report::WithStrictUnwrap => {
                    // Both reports read the one set of records, and the second
                    // runs whatever the first says.
                    let unwrap = unwraps(&records, &root, Strictness::Strict);
                    println!(
                        "primitives: the primitive gate {}, the strict unwrap report {}",
                        primitives.word(),
                        unwrap.word()
                    );
                    primitives.and(unwrap)
                }
            }
        }
    };
    verdict.exit()
}

/// The workspace root, two levels above this crate's manifest.
fn workspace_root() -> PathBuf {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    manifest
        .parent()
        .and_then(Path::parent)
        .unwrap_or(manifest)
        .to_path_buf()
}

/// The allowlist, parsed, or the errors that keep it from parsing.
fn allowlist(root: &Path) -> Result<(Allowlist, PathBuf), Vec<String>> {
    let path = root.join(ALLOWLIST);
    let text =
        std::fs::read_to_string(&path).map_err(|_| vec![format!("cannot read {ALLOWLIST}")])?;
    let list = Allowlist::parse(&text).map_err(|errors| {
        errors
            .into_iter()
            .map(|e| format!("{ALLOWLIST}: {e}"))
            .collect::<Vec<_>>()
    })?;
    Ok((list, path))
}

/// The unwrap report: each refusal, then the counts.
fn unwraps(records: &Records, root: &Path, strictness: Strictness) -> Verdict {
    for (key, at) in &records.unwraps {
        println!("{at}: {key}");
    }
    println!("unwrap: {} refusals", records.unwraps.len());
    let text = std::fs::read_to_string(root.join(UNWRAP_ALLOWLIST)).ok();
    let keys = records.unwraps.iter().map(|(key, _)| key.as_str());
    let judgement = match unwraps::judge(keys, text.as_deref(), strictness) {
        Ok(judgement) => judgement,
        Err(errors) => {
            for error in errors {
                eprintln!("primitives: {UNWRAP_ALLOWLIST}: {error}");
            }
            return Verdict::Fail;
        }
    };
    for problem in &judgement.problems {
        eprintln!("primitives: {UNWRAP_ALLOWLIST}: {problem}");
    }
    println!(
        "unwrap: {} refusal keys no line of {UNWRAP_ALLOWLIST} names, {} lines over their count, \
         {} stale lines",
        judgement.unlisted, judgement.over, judgement.stale
    );
    match judgement.outcome {
        unwraps::Outcome::Pass => Verdict::Pass,
        unwraps::Outcome::Fail => Verdict::Fail,
    }
}

/// Removes the lines that name no site, and the census lines that name no
/// newtype.
fn prune(records: &Records, root: &Path) -> Verdict {
    let (list, path) = match allowlist(root) {
        Ok(found) => found,
        Err(errors) => {
            for error in errors {
                eprintln!("primitives: {error}");
            }
            return Verdict::Fail;
        }
    };
    let census = root.join(newtypes::NEWTYPES);
    let found = records.newtypes.keys().cloned().collect();
    if let Ok(text) = std::fs::read_to_string(&census)
        && let Err(e) = std::fs::write(&census, newtypes::pruned(&text, &found))
    {
        eprintln!("primitives: cannot write {}: {e}", newtypes::NEWTYPES);
        return Verdict::Fail;
    }
    let keys = records.keys();
    let stale = list.compare(&keys).stale.len();
    if let Err(e) = std::fs::write(&path, list.pruned(&keys)) {
        eprintln!("primitives: cannot write {ALLOWLIST}: {e}");
        return Verdict::Fail;
    }
    println!("primitives: removed {stale} lines that named no site");
    Verdict::Pass
}

/// The primitive gate on one set of records. `compiled` carries the features
/// of the compiles, absent when the records come from an earlier run.
fn gate(records: &Records, root: &Path, compiled: Option<&run::Compiled>) -> Verdict {
    let mut failed = false;
    let mut report = |what: &str, refusals: Vec<String>| {
        for refusal in &refusals {
            eprintln!("primitives: {refusal}");
        }
        if !refusals.is_empty() {
            eprintln!("primitives: {} refusals of {what}.", refusals.len());
            failed = true;
        }
    };
    if let Some(compiled) = compiled {
        report(
            "the feature check (ARCH-033)",
            features::check(root, &compiled.features),
        );
    }
    report(
        "the file check (ARCH-033, ARCH-046, UI-THREAD-062)",
        files::check(root, &records.files),
    );
    let census = std::fs::read_to_string(root.join(newtypes::NEWTYPES)).ok();
    let found = records.newtypes.keys().cloned().collect();
    report(
        "the newtype census (ARCH-062)",
        newtypes::judge(census.as_deref(), &found),
    );
    let spec = std::fs::read_to_string(root.join(aliases::SPEC)).ok();
    report(
        "the bare-alias check (ambre-spec-newtypes.md section 3)",
        aliases::judge(spec.as_deref(), records),
    );
    let manifests = boundaries::metadata(root)
        .and_then(|text| boundaries::Graph::parse(&text))
        .map_or_else(
            |error| vec![error],
            |graph| {
                let mut refusals =
                    boundaries::manifests(&graph, &|p| std::fs::read_to_string(p).ok());
                refusals.extend(boundaries::chrome::manifests(&graph));
                refusals
            },
        );
    report(
        "the crate boundaries of the manifests (ambre-spec-architecture.md section 2)",
        manifests,
    );
    report(
        "the one wire definition (ARCH-006)",
        boundaries::wire(records),
    );
    report(
        "the launcher's wiring (ambre-spec-architecture.md section 2)",
        boundaries::wiring(records),
    );
    report(
        "the open-time create (CHROME-037)",
        boundaries::chrome::facts(records),
    );
    report(
        "the views, the shell signals and the client build (ambre-spec-ui.md section 2)",
        views::views(records),
    );
    for (check, message) in &records.refusals {
        eprintln!("primitives: {check}: {message}");
    }
    if !records.refusals.is_empty() {
        eprintln!(
            "primitives: {} refusals the code settles: a Deref to a primitive, a secret read \
             or wiped outside its module, a sink outside its file, a public field of a tuple \
             struct, a crate boundary crossed. No line of {ALLOWLIST} admits one.",
            records.refusals.len()
        );
        failed = true;
    }
    let list = match allowlist(root) {
        Ok((list, _)) => list,
        Err(errors) => {
            for error in errors {
                eprintln!("primitives: {error}");
            }
            return Verdict::Fail;
        }
    };
    let keys = records.keys();
    let verdict = list.compare(&keys);
    for key in &verdict.missing {
        let (at, origin) = records
            .sites
            .get(key)
            .map_or(("", ""), |site| (site.at.as_str(), site.origin.as_str()));
        eprintln!("primitives: no line names the site at {at}, written by {origin}\n  {key}");
    }
    if !verdict.missing.is_empty() {
        eprintln!(
            "primitives: {} sites hold a primitive and no line of {ALLOWLIST} names them. Type \
             the value with a newtype or an enum. If the primitive is right, add the line by \
             hand, as {HEADER:?} separated by tabs, with one category of: {}.",
            verdict.missing.len(),
            Category::list()
        );
    }
    for (line, key) in &verdict.stale {
        eprintln!("primitives: {ALLOWLIST}:{line} names no site\n  {key}");
    }
    if !verdict.stale.is_empty() {
        eprintln!(
            "primitives: {} lines name no site. Remove them with `just primitives-prune`: the \
             file only shrinks on its own.",
            verdict.stale.len()
        );
    }
    let refuted = list.refuted(records);
    for refuted in &refuted {
        match refuted.defect {
            Defect::Shape => eprintln!(
                "primitives: {ALLOWLIST}:{} is not {}: {}\n  {}",
                refuted.line, refuted.category, refuted.reason, refuted.key
            ),
            defect => eprintln!(
                "primitives: {ALLOWLIST}:{} ({}) is refused, {defect}: {}\n  {}",
                refuted.line, refuted.category, refuted.reason, refuted.key
            ),
        }
    }
    if !refuted.is_empty() {
        eprintln!(
            "primitives: {} lines are refused from their site. Type the value, or give the line \
             the category true of its site.",
            refuted.len()
        );
    }
    let counts: Vec<String> = Category::ALL
        .iter()
        .map(|category| {
            format!(
                "{category} {}",
                verdict.counts.get(category).copied().unwrap_or(0)
            )
        })
        .collect();
    println!("primitives: {} sites; {}", keys.len(), counts.join(", "));
    if failed || !verdict.is_clean() || !refuted.is_empty() {
        Verdict::Fail
    } else {
        Verdict::Pass
    }
}
