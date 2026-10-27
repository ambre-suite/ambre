//! The files rustc compiles, read from the dep-info cargo writes, and held
//! against the files the walk reads (`ARCH-033`, `ARCH-045`).
//!
//! The walk resolves modules with syn, as rustc does, and every module
//! placement it misreads let a file through unread. rustc writes down, for
//! each crate it compiles, every source file it read. A `#[path]` file, an
//! `include!` file and the file of a `mod` a macro writes are all there. So the gate runs each compile
//! of the workspace and holds the walk to that set:
//!
//!   * a `.rs` file that a build of a crate reads, outside a test build, is
//!     one the walk reads from the same crate root;
//!   * a crate root that a build compiles is one the walk starts from;
//!   * a `.rs` file under `crates/` is one that a compile reads, test builds
//!     included, so no file sits in the tree that no build checks.
//!
//! A compile that fails, and a dep-info file that is missing, fail the
//! check: a stale set would pass a file no build read. Only a unit of a
//! workspace package is read, so a registry crate and the standard library
//! count for nothing. A file that is not `.rs`, such as the data of an
//! `include_str!`, is no module and is left out.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::scan::{Disk, Scan, Tree};

/// Where one compile builds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Target {
    /// The host, under the flags of `lint`.
    Host,
    /// `wasm32-unknown-unknown`, under the flags of `lint-wasm`.
    Wasm,
}

/// One `cargo check` of the workspace: its target, and the arguments that
/// select its packages and its targets.
pub(crate) struct Compile {
    pub(crate) target: Target,
    pub(crate) args: &'static [&'static str],
}

/// Every compile the workspace makes. The host line is the widest one: every
/// package, every target, every feature. The wasm lines are those of the
/// `lint-wasm` recipe, which compile the code under `cfg(target_arch)` that
/// no host build reads. A test holds the two lists equal.
pub(crate) const COMPILES: &[Compile] = &[
    Compile {
        target: Target::Host,
        args: &["--workspace", "--all-targets", "--all-features"],
    },
    Compile {
        target: Target::Wasm,
        args: &["--workspace", "--exclude", "ambre-server", "--lib"],
    },
    Compile {
        target: Target::Wasm,
        args: &["-p", "ambre-trame", "-p", "ambre-toile", "--tests"],
    },
    Compile {
        target: Target::Wasm,
        args: &["-p", "ambre-prose", "--test", "chrome_panel"],
    },
    Compile {
        target: Target::Wasm,
        args: &["-p", "ambre-murmure", "--test", "chrome_relay"],
    },
    Compile {
        target: Target::Wasm,
        args: &["-p", "ambre-logic", "--tests"],
    },
    Compile {
        target: Target::Wasm,
        args: &["-p", "ambre-app", "--tests"],
    },
];

/// The target triple of the wasm compiles.
const WASM_TRIPLE: &str = "wasm32-unknown-unknown";

/// The flags of every wasm line of `lint-wasm`, so the check shares their
/// build.
const WASM_FLAGS: &str = "-Awarnings -C debuginfo=0";

/// Which build of a crate a unit is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Build {
    /// A library or a binary as it ships, which the walk reads.
    Product,
    /// A test build, an integration test, a bench or an example, which the
    /// walk leaves out.
    Test,
}

/// One crate as rustc compiled it, under one build.
#[derive(Debug)]
pub(crate) struct Unit {
    /// The crate root, canonical.
    pub(crate) root: PathBuf,
    pub(crate) build: Build,
    /// Every `.rs` file rustc read, canonical.
    pub(crate) reads: BTreeSet<PathBuf>,
}

/// What every compile of the workspace read, one unit per crate root and
/// build, each the union over the compiles.
#[derive(Debug, Default)]
pub(crate) struct Compiled {
    pub(crate) units: Vec<Unit>,
}

/// Runs each compile in `root` and reads what rustc read. `configure` sets
/// the command up further, for a test that builds elsewhere. Every failure is
/// an error: the check never runs on a partial set.
pub(crate) fn collect(
    root: &Path,
    compiles: &[Compile],
    configure: &dyn Fn(&mut Command),
) -> Result<Compiled, Vec<String>> {
    let root = Disk.canonical(root);
    let mut units: BTreeMap<(PathBuf, Build), BTreeSet<PathBuf>> = BTreeMap::new();
    let mut errors = Vec::new();
    for compile in compiles {
        let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
        let mut command = Command::new(cargo);
        command
            .current_dir(&root)
            .arg("check")
            .args(compile.args)
            .arg("--message-format=json")
            .arg("--quiet");
        if compile.target == Target::Wasm {
            command
                .arg("--target")
                .arg(WASM_TRIPLE)
                .env("RUSTFLAGS", WASM_FLAGS);
        }
        configure(&mut command);
        let line = compile.args.join(" ");
        let output = match command.output() {
            Ok(output) => output,
            Err(e) => {
                errors.push(format!("cargo check {line}: cannot run cargo: {e}"));
                continue;
            }
        };
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let tail: Vec<&str> = stderr.lines().rev().take(20).collect();
            let tail: Vec<&str> = tail.into_iter().rev().collect();
            errors.push(format!(
                "cargo check {line} fails, so the check cannot read what rustc compiles:\n{}",
                tail.join("\n")
            ));
            continue;
        }
        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut named = 0_usize;
        for message in stdout.lines() {
            match artifact(&root, message) {
                Ok(Some(unit)) => {
                    named += 1;
                    units
                        .entry((unit.root, unit.build))
                        .or_default()
                        .extend(unit.reads);
                }
                Ok(None) => {}
                Err(error) => errors.push(format!("cargo check {line}: {error}")),
            }
        }
        if named == 0 {
            errors.push(format!(
                "cargo check {line} names no unit of the workspace, so the check reads nothing \
                 and fails closed"
            ));
        }
    }
    if errors.is_empty() {
        let units = units
            .into_iter()
            .map(|((root, build), reads)| Unit { root, build, reads })
            .collect();
        Ok(Compiled { units })
    } else {
        Err(errors)
    }
}

/// The unit one message of cargo describes, and the `.rs` files it read.
/// `None` for another message, and for a unit of a package outside the
/// workspace.
fn artifact(root: &Path, message: &str) -> Result<Option<Unit>, String> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(message) else {
        return Ok(None);
    };
    if value.get("reason").and_then(serde_json::Value::as_str) != Some("compiler-artifact") {
        return Ok(None);
    }
    let Some(manifest) = value
        .get("manifest_path")
        .and_then(serde_json::Value::as_str)
    else {
        return Err("an artifact message names no manifest".to_owned());
    };
    if !Disk.canonical(Path::new(manifest)).starts_with(root) {
        return Ok(None);
    }
    let Some(source) = value
        .pointer("/target/src_path")
        .and_then(serde_json::Value::as_str)
    else {
        return Err(format!("{manifest}: an artifact names no crate root"));
    };
    let kinds: Vec<&str> = value
        .pointer("/target/kind")
        .and_then(serde_json::Value::as_array)
        .map(|kinds| kinds.iter().filter_map(serde_json::Value::as_str).collect())
        .unwrap_or_default();
    let test = value
        .pointer("/profile/test")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(true);
    let build = if test
        || kinds
            .iter()
            .any(|kind| matches!(*kind, "test" | "bench" | "example"))
    {
        Build::Test
    } else {
        Build::Product
    };
    let Some(output) = value
        .get("filenames")
        .and_then(serde_json::Value::as_array)
        .and_then(|names| names.first())
        .and_then(serde_json::Value::as_str)
    else {
        return Err(format!("{source}: an artifact names no output file"));
    };
    let output = Path::new(output);
    let writer = if kinds.contains(&"custom-build") {
        Writer::Script
    } else {
        Writer::Crate
    };
    let Some(info) = dep_info(output, writer) else {
        return Err(format!(
            "{source}: cargo wrote no dep-info beside {}, so the check cannot read what rustc \
             compiled, and fails closed",
            output.display()
        ));
    };
    let Ok(text) = std::fs::read_to_string(&info) else {
        return Err(format!(
            "{}: the dep-info of {source} cannot be read, so the check fails closed",
            info.display()
        ));
    };
    let reads = dependencies(&text)
        .into_iter()
        .filter(|path| path.extension().is_some_and(|ext| ext == "rs"))
        .map(|path| Disk.canonical(&root.join(path)))
        .collect();
    Ok(Some(Unit {
        root: Disk.canonical(Path::new(source)),
        build,
        reads,
    }))
}

/// What a unit builds, which decides where its dep-info sits.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Writer {
    /// A library, a binary or a test: `deps/name-hash.d` beside
    /// `deps/libname-hash.rmeta`.
    Crate,
    /// A build script: `build_script_build-hash.d` in the directory of its
    /// binary.
    Script,
}

/// The dep-info file rustc wrote beside an output.
fn dep_info(output: &Path, writer: Writer) -> Option<PathBuf> {
    let dir = output.parent()?;
    if writer == Writer::Script {
        let entries = std::fs::read_dir(dir).ok()?;
        return entries
            .filter_map(Result::ok)
            .map(|e| e.path())
            .find(|path| {
                path.extension().is_some_and(|ext| ext == "d")
                    && path.file_name().is_some_and(|name| {
                        name.to_string_lossy().starts_with("build_script_build-")
                    })
            });
    }
    let stem = output.file_stem()?.to_string_lossy().into_owned();
    let stem = stem.strip_prefix("lib").unwrap_or(&stem);
    let info = dir.join(format!("{stem}.d"));
    info.is_file().then_some(info)
}

/// Every dependency a dep-info file lists, in the order it lists them. A
/// rule reads `target: dep dep ...`, and a space inside a path is `\ `. A
/// line of `# env-dep` and the empty rules rustc adds per file name no
/// further file.
pub(crate) fn dependencies(text: &str) -> Vec<PathBuf> {
    let mut found = Vec::new();
    for line in text.lines().filter(|line| !line.starts_with('#')) {
        let mut words = Vec::new();
        let mut word = String::new();
        let mut chars = line.chars();
        while let Some(c) = chars.next() {
            match c {
                '\\' => match chars.next() {
                    Some(' ') => word.push(' '),
                    Some(next) => {
                        word.push('\\');
                        word.push(next);
                    }
                    None => word.push('\\'),
                },
                ' ' => words.push(std::mem::take(&mut word)),
                _ => word.push(c),
            }
        }
        words.push(word);
        let mut words = words.into_iter().filter(|word| !word.is_empty());
        if words.next().is_some_and(|target| target.ends_with(':')) {
            found.extend(words.map(PathBuf::from));
        }
    }
    found
}

/// Every refusal of the dep-info check. `sources` lists every `.rs` file
/// under `crates/`, canonical. `root` is the workspace root, canonical, and
/// shortens each path a refusal prints.
pub(crate) fn cross_check(
    scan: &Scan,
    compiled: &Compiled,
    sources: &BTreeSet<PathBuf>,
    root: &Path,
) -> Vec<String> {
    let shown = |path: &Path| {
        path.strip_prefix(root)
            .unwrap_or(path)
            .display()
            .to_string()
    };
    let mut errors = Vec::new();
    let mut read = BTreeSet::new();
    for unit in &compiled.units {
        read.extend(unit.reads.iter().cloned());
        if unit.build == Build::Test {
            continue;
        }
        let Some(walked) = scan.walked.get(&unit.root) else {
            errors.push(format!(
                "{}: rustc compiles this crate root, and the walk does not start from it",
                shown(&unit.root)
            ));
            continue;
        };
        for file in unit.reads.difference(walked) {
            errors.push(format!(
                "{}: rustc reads it for the crate root {}, and the walk does not, so the gate \
                 cannot place it",
                shown(file),
                shown(&unit.root)
            ));
        }
    }
    for file in sources.difference(&read) {
        errors.push(format!(
            "{}: no compile of the workspace reads it, so no build checks it",
            shown(file)
        ));
    }
    errors
}

/// Every `.rs` file under `crates`, canonical, build output left out.
pub(crate) fn sources(tree: &impl Tree, crates: &Path) -> BTreeSet<PathBuf> {
    tree.files(crates)
        .into_iter()
        .filter(|path| path.extension().is_some_and(|ext| ext == "rs"))
        .map(|path| tree.canonical(&path))
        .collect()
}

#[cfg(test)]
#[path = "depinfo_tests.rs"]
mod tests;
