//! The files rustc compiles, read from the dep-info cargo writes, and held
//! against the files the walk reads (`ARCH-033`, `ARCH-045`).
//!
//! The walk resolves modules with syn, as rustc does, and every module
//! placement it misreads let a file through unread. rustc writes down, for
//! each crate it compiles, every file it read. A `#[path]` file, an
//! `include!` file and the file of a `mod` a macro writes are all there. So
//! the gate runs each compile of the workspace and holds the walk to that
//! set:
//!
//!   * a file that a shipped build of a crate reads is one the walk reads
//!     from the same crate root. Or a walked file names it as data, with a
//!     string literal. The name of the file decides nothing: rustc compiles
//!     a `.txt` file as a module when a `#[path]` names it;
//!   * a data file that parses as Rust items is refused all the same, since
//!     the dep-info does not tell a module from data;
//!   * a crate root that a build compiles is one the walk starts from;
//!   * a `.rs` file under the workspace root is one that a compile reads,
//!     test builds included. So no file sits in the tree that no build
//!     checks. A nested workspace, such as the fuzz targets, builds apart
//!     and is left out.
//!
//! A failed compile, a missing dep-info file, and a dep-info line the
//! parser cannot read whole all fail the check. A partial set would pass a
//! file no build read. Only a unit of a workspace package is read, so
//! a registry crate and the standard library count for nothing.

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
    /// Every file rustc read, whatever its name, canonical.
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
        let before = errors.len();
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
        if named == 0 && errors.len() == before {
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
    // A unit with no test flag is no unit the check can class, so it fails
    // closed rather than pass as a test build.
    let Some(test) = value
        .pointer("/profile/test")
        .and_then(serde_json::Value::as_bool)
    else {
        return Err(format!(
            "{source}: an artifact names no test flag in its profile, so the check cannot tell \
             a test build from a shipped one, and fails closed"
        ));
    };
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
    // cargo deems a unit fresh from its own fingerprint, and never writes
    // the dep-info again for it. A rebuild of the package writes it anew.
    let package = package(&value);
    let Some(info) = dep_info(output, writer) else {
        return Err(format!(
            "{source}: cargo wrote no dep-info beside {}, so the check cannot read what rustc \
             compiled, and fails closed. Touch a source file of {package}, or run `cargo clean \
             -p {package}`, then run the gate again",
            output.display()
        ));
    };
    let Ok(text) = std::fs::read_to_string(&info) else {
        return Err(format!(
            "{}: the dep-info of {source} cannot be read, so the check fails closed. Touch a \
             source file of {package}, or run `cargo clean -p {package}`, then run the gate again",
            info.display()
        ));
    };
    let reads = dependencies(&text)
        .map_err(|error| format!("{}: {error}", info.display()))?
        .into_iter()
        .map(|path| Disk.canonical(&root.join(path)))
        .collect();
    Ok(Some(Unit {
        root: Disk.canonical(Path::new(source)),
        build,
        reads,
    }))
}

/// The package name of an artifact, as `cargo clean -p` takes it. A
/// package id reads `path+file:///dir#name@version`, or `#version` alone
/// when the name is the last segment of the directory.
fn package(value: &serde_json::Value) -> String {
    let id = value
        .get("package_id")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    let (url, tail) = id.rsplit_once('#').unwrap_or((id, ""));
    let name = tail.split_once('@').map_or(tail, |(name, _)| name);
    if !name.is_empty() && !name.starts_with(|c: char| c.is_ascii_digit()) {
        return name.to_owned();
    }
    url.rsplit('/').next().unwrap_or(url).to_owned()
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

/// Every dependency a dep-info file lists, in the order its first rule
/// lists them. The parser fails closed on any text it cannot read whole:
///
///   * a line is empty, a `#` comment, or a rule `target: dep dep ...`;
///   * a path holds no `:`, and a backslash only before a space, since
///     rustc escapes a space and nothing else. A path that holds a newline
///     or a backslash cannot be read back, so its fragments fail here;
///   * every rule that lists dependencies lists the same ones, and each of
///     them has an empty rule of its own, as rustc writes them. A fragment
///     that reads as a rule breaks one of the two.
pub(crate) fn dependencies(text: &str) -> Result<Vec<PathBuf>, String> {
    let mut listed: Option<Vec<String>> = None;
    let mut own = BTreeSet::new();
    for (number, line) in text.lines().enumerate() {
        let number = number + 1;
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let words = words(line).map_err(|why| format!("line {number}: {why}"))?;
        let mut words = words.into_iter();
        let target = words
            .next()
            .and_then(|word| word.strip_suffix(':').map(str::to_owned))
            .filter(|target| !target.is_empty());
        let Some(target) = target else {
            return Err(format!(
                "line {number} is no rule `target: dependencies`. rustc does not escape a newline, \
                 so a path that holds one splits into fragments the check cannot place, and it \
                 fails closed"
            ));
        };
        let deps: Vec<String> = words.collect();
        if let Some(word) = deps.iter().find(|word| word.contains(':')) {
            return Err(format!(
                "line {number}: the path {word:?} holds a `:`, which reads as a rule, so the \
                 check fails closed"
            ));
        }
        if deps.is_empty() {
            own.insert(target);
            continue;
        }
        match &listed {
            None => listed = Some(deps),
            Some(first) if *first == deps => {}
            Some(_) => {
                return Err(format!(
                    "line {number}: the rule of {target} lists other files than the first rule, \
                     so a path splits, and the check fails closed"
                ));
            }
        }
    }
    let Some(listed) = listed else {
        return Err("no rule lists a dependency, so the check reads nothing, and fails closed"
            .to_owned());
    };
    let named: BTreeSet<String> = listed.iter().cloned().collect();
    if let Some(path) = named.symmetric_difference(&own).next() {
        return Err(format!(
            "the path {path:?} has no rule of its own, or a rule names a path no rule lists, so \
             a path splits, and the check fails closed"
        ));
    }
    Ok(listed.into_iter().map(PathBuf::from).collect())
}

/// The words of one dep-info line: a space separates two, and `\ ` is a
/// space inside one. Any other backslash fails, since rustc escapes nothing
/// else and the path it belongs to cannot be read back.
fn words(line: &str) -> Result<Vec<String>, String> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => match chars.next() {
                Some(' ') => word.push(' '),
                _ => {
                    return Err(
                        "a path holds a backslash that escapes no space. rustc escapes a space \
                         alone, so the path cannot be read back, and the check fails closed"
                            .to_owned(),
                    );
                }
            },
            ' ' => {
                if !word.is_empty() {
                    words.push(std::mem::take(&mut word));
                }
            }
            _ => word.push(c),
        }
    }
    if !word.is_empty() {
        words.push(word);
    }
    Ok(words)
}

/// Every refusal of the dep-info check. `sources` lists every `.rs` file
/// the orphan check covers, canonical. `root` is the workspace root,
/// canonical, and shortens each path a refusal prints. `tree` reads a data
/// file, to refuse one that parses as Rust items.
pub(crate) fn cross_check(
    scan: &Scan,
    compiled: &Compiled,
    sources: &BTreeSet<PathBuf>,
    root: &Path,
    tree: &impl Tree,
) -> Vec<String> {
    let shown = |path: &Path| {
        path.strip_prefix(root)
            .unwrap_or(path)
            .display()
            .to_string()
    };
    let none = BTreeSet::new();
    let mut errors = Vec::new();
    let mut read = BTreeSet::new();
    for unit in &compiled.units {
        read.extend(unit.reads.iter().cloned());
        // A test build is test code whole, which no check of the gate reads.
        // What it reads ships in no build, so it only feeds the orphan check.
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
        let data = scan.data.get(&unit.root).unwrap_or(&none);
        for file in unit.reads.difference(walked) {
            if !data.contains(file) {
                errors.push(format!(
                    "{}: rustc reads it for the crate root {}, and the walk neither reads it as \
                     a module nor finds a string literal that names it as data, so the gate \
                     cannot place it",
                    shown(file),
                    shown(&unit.root)
                ));
            } else if parses_as_items(tree, file) {
                errors.push(format!(
                    "{}: a walked file of the crate root {} names it as data, and it parses as \
                     Rust items. The dep-info does not tell a module from data, so the gate \
                     cannot show that no `mod` compiles it",
                    shown(file),
                    shown(&unit.root)
                ));
            }
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

/// Whether the file is Rust that declares at least one item. rustc compiles
/// a module only from text that parses. A module with no item adds no code,
/// so any other data file cannot hide a module.
fn parses_as_items(tree: &impl Tree, file: &Path) -> bool {
    tree.read(file)
        .and_then(|text| syn::parse_file(&text).ok())
        .is_some_and(|parsed| !parsed.items.is_empty())
}

/// Every `.rs` file under the workspace root, canonical. Build output is
/// left out, and so is each nested workspace. A directory below the root
/// whose manifest declares a `[workspace]` of its own builds apart, as the
/// fuzz targets do under `cargo-fuzz`.
pub(crate) fn sources(tree: &impl Tree, root: &Path) -> BTreeSet<PathBuf> {
    let files = tree.files(root);
    let nested: Vec<PathBuf> = files
        .iter()
        .filter(|path| path.file_name().is_some_and(|name| name == "Cargo.toml"))
        .filter(|path| path.parent() != Some(root))
        .filter(|path| {
            tree.read(path)
                .is_some_and(|text| text.lines().any(|line| line.trim() == "[workspace]"))
        })
        .filter_map(|path| path.parent().map(Path::to_path_buf))
        .collect();
    files
        .into_iter()
        .filter(|path| path.extension().is_some_and(|ext| ext == "rs"))
        .filter(|path| !nested.iter().any(|dir| path.starts_with(dir)))
        .map(|path| tree.canonical(&path))
        .collect()
}

#[cfg(test)]
#[path = "depinfo_tests.rs"]
mod tests;
