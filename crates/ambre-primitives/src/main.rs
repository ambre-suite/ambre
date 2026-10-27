//! The primitive gate (`ambre-spec-testing.md` section 2).
//!
//! Every site of the workspace whose type holds a bare primitive is typed. The
//! exception is a site that one line of the allowlist names, with the reason
//! it may keep the primitive (`ai/ambre-rust-style.md`). A model writing code
//! copies the primitive it sees in the next signature, so a review rule alone
//! loses ground on every change. The gate makes the count only go down.
//!
//! Three modes, one per recipe:
//!
//!   * `check`, the default, fails on seven defects, then prints the lines
//!     per category. Two defects compare the file with the sites: a site no
//!     line names, and a line that names no site. Three read the site: a
//!     claimed shape the site lacks, a privacy refusal, and a `Deref` of a
//!     workspace type to a primitive. The sixth reads names: a secret reader outside `ambre-crypto`, or
//!     the `zeroize` crate outside the Secret module (`ARCH-045`,
//!     `ARCH-046`).
//!
//!     The seventh holds the walk to rustc. It refuses a file a compile
//!     reads that the walk neither reads nor names as data, and a `.rs` file
//!     no compile reads. It refuses a build lever that enables an
//!     unstable feature too, in the build configuration or in the gate's
//!     own environment (`ARCH-033`);
//!   * `files` runs the seventh check alone, so a red allowlist hides no
//!     file the walk misses;
//!   * `--prune` removes the lines that name no site, and nothing else.

mod allowlist;
mod depinfo;
mod scan;
mod secrets;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use allowlist::{Allowlist, Category, Defect, HEADER};
use scan::Scan;

/// The allowlist, relative to the workspace root. It sits beside the other
/// files the gates of the corpus read.
const ALLOWLIST: &str = "specs/1-Foundations/ambre-primitives.tsv";

/// What one run does.
enum Mode {
    Check,
    Files,
    Prune,
}

fn main() -> ExitCode {
    let mode = match std::env::args().nth(1).as_deref() {
        None | Some("check") => Mode::Check,
        Some("files") => Mode::Files,
        Some("--prune") => Mode::Prune,
        Some(other) => {
            eprintln!(
                "primitives: unknown argument {other:?} -- usage: ambre-primitives [check | files \
                 | --prune]"
            );
            return ExitCode::FAILURE;
        }
    };
    let root = workspace_root();
    let crates = root.join("crates");
    let disk = scan::Disk::at(&root);
    let scan = scan::scan(&disk, &crates);
    if !scan.errors.is_empty() {
        for error in &scan.errors {
            eprintln!("primitives: {error}");
        }
        return ExitCode::FAILURE;
    }
    let unread = match mode {
        Mode::Prune => Vec::new(),
        Mode::Check | Mode::Files => files(&scan, &disk, &root),
    };
    if let Mode::Files = mode {
        return if unread.is_empty() {
            ExitCode::SUCCESS
        } else {
            ExitCode::FAILURE
        };
    }
    let path = root.join(ALLOWLIST);
    let Ok(text) = std::fs::read_to_string(&path) else {
        eprintln!("primitives: cannot read {ALLOWLIST}");
        return ExitCode::FAILURE;
    };
    let list = match Allowlist::parse(&text) {
        Ok(list) => list,
        Err(errors) => {
            for error in errors {
                eprintln!("primitives: {ALLOWLIST}: {error}");
            }
            return ExitCode::FAILURE;
        }
    };
    match mode {
        Mode::Check | Mode::Files => check(&list, &scan, &unread),
        Mode::Prune => prune(&list, &scan.sites, &path),
    }
}

/// Every refusal of the dep-info check, printed with its count. Each compile
/// runs in the workspace, so the check reads the files the recipes build.
/// The build configuration is read first, and refused whatever the
/// compiles say: a lever can make a compile fail, or pass. A lever in the
/// gate's own environment stops the check before any compile, since a
/// wrapper it names would run.
fn files(scan: &Scan, disk: &scan::Disk, root: &Path) -> Vec<String> {
    let environment = depinfo::env_levers(std::env::vars_os());
    let compile = environment.is_empty();
    let mut refusals = environment;
    refusals.append(&mut depinfo::levers(disk, root));
    refusals.append(&mut depinfo::outer_levers(
        disk,
        root,
        cargo_home().as_deref(),
    ));
    refusals.append(&mut depinfo::stray_workspaces(disk, root));
    if compile {
        match depinfo::collect(root, depinfo::COMPILES, &|_| {}) {
            Ok(compiled) => {
                let sources = depinfo::sources(disk, root);
                let root = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
                refusals.append(&mut depinfo::cross_check(
                    scan, &compiled, &sources, &root, disk,
                ));
            }
            Err(mut errors) => refusals.append(&mut errors),
        }
    }
    for refusal in &refusals {
        eprintln!("primitives: {refusal}");
    }
    println!(
        "primitives: dep-info: {} refusals of the file set rustc compiles",
        refusals.len()
    );
    refusals
}

/// The cargo home whose config cargo reads: `CARGO_HOME`, or `.cargo` in
/// the home directory, as cargo finds it.
fn cargo_home() -> Option<PathBuf> {
    std::env::var_os("CARGO_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| Path::new(&home).join(".cargo")))
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

fn check(list: &Allowlist, scan: &Scan, unread: &[String]) -> ExitCode {
    let sites = &scan.sites;
    let verdict = list.compare(sites);
    for key in &verdict.missing {
        eprintln!("primitives: no line names the site\n  {key}");
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
    let refuted = list.refuted(scan);
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
    let shapes = refuted
        .iter()
        .filter(|refuted| refuted.defect == Defect::Shape)
        .count();
    if shapes > 0 {
        eprintln!(
            "primitives: {shapes} lines claim a shape their site does not have. Type the value, or \
             give the line the category true of its site."
        );
    }
    let private = refuted.len() - shapes;
    if private > 0 {
        eprintln!(
            "primitives: {private} refusals name a site that is not private, where the rule wants \
             a private one. Type the value: only a bool field that mirrors a foreign schema may \
             move to foreign-api."
        );
    }
    for deref in &scan.derefs {
        eprintln!("primitives: {deref}");
    }
    if !scan.derefs.is_empty() {
        eprintln!(
            "primitives: {} impls dereference a workspace type to a primitive. No line of \
             {ALLOWLIST} admits one: remove the impl.",
            scan.derefs.len()
        );
    }
    for secret in &scan.secrets {
        eprintln!("primitives: {secret}");
    }
    if !scan.secrets.is_empty() {
        eprintln!(
            "primitives: {} names read a secret, or wipe one, outside the code that may. Move \
             the read into ambre-crypto, and hold the value in an ambre_core::Secret.",
            scan.secrets.len()
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
    println!("primitives: {} sites; {}", sites.len(), counts.join(", "));
    if verdict.is_clean()
        && refuted.is_empty()
        && scan.derefs.is_empty()
        && scan.secrets.is_empty()
        && unread.is_empty()
    {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

fn prune(
    list: &Allowlist,
    sites: &std::collections::BTreeSet<scan::SiteKey>,
    path: &Path,
) -> ExitCode {
    let stale = list.compare(sites).stale.len();
    if let Err(e) = std::fs::write(path, list.pruned(sites)) {
        eprintln!("primitives: cannot write {ALLOWLIST}: {e}");
        return ExitCode::FAILURE;
    }
    println!("primitives: removed {stale} lines that named no site");
    ExitCode::SUCCESS
}
