//! The compiles of the gate: the lint lines of the `justfile`, run through the
//! rustc driver of `driver/` (`ambre-spec-testing.md` section 2).
//!
//! The gate reads the compile lines of the `lint` and `lint-wasm` recipes. It
//! runs each one as `cargo check` with its flags and its arguments, the driver
//! standing in for rustc. So the driver compiles every unit clippy lints,
//! under the same target, `cfg` and feature set. It reads each file rustc
//! reads in the module rustc gives it (`ARCH-050`).
//!
//! Clippy holds the workspace wrapper of `just lint`. So the gate drops the
//! fingerprints of the workspace units first: each one compiles again, and
//! the dependencies stay fresh in the shared target directory.
//!
//! A line the gate cannot run as `just` runs it fails closed. So does a failed
//! compile. So does a unit cargo builds with no record file. A cache, a
//! wrapper or a fresh fingerprint kept the driver out of it.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;

/// The `justfile`, relative to the workspace root.
pub const JUSTFILE: &str = "justfile";

/// The recipes whose compile lines the gate runs.
const LINT_RECIPES: &[&str] = &["lint", "lint-wasm"];

/// The arguments that change what the gate reads, or how cargo reports it.
const BANNED_ARGS: &[&str] = &[
    "--manifest-path",
    "--target-dir",
    "--config",
    "--message-format",
    "-Z",
];

/// The characters the shell or `just` would read in a word.
const SHELL_CHARS: &[char] = &[
    '"', '\'', '`', '$', '\\', ';', '&', '|', '<', '>', '(', ')', '{', '}', '[', ']', '*', '?',
    '~', '!', '#',
];

/// The variables `cargo run` sets for the binary it runs, beside each
/// `CARGO_PKG_*` one. The gate runs the lint lines without them, as `just`
/// does.
const RUN_VARS: &[&str] = &[
    "CARGO_MANIFEST_DIR",
    "CARGO_MANIFEST_PATH",
    "CARGO_CRATE_NAME",
    "CARGO_BIN_NAME",
    "CARGO_PRIMARY_PACKAGE",
];

/// The variables that put another program between cargo and the driver. A
/// cache such as sccache would answer for the driver and write no record.
const WRAPPERS: &[&str] = &["RUSTC_WRAPPER", "CARGO_BUILD_RUSTC_WRAPPER", "RUSTC"];

/// The directory of the driver package, relative to the workspace root.
pub const DRIVER: &str = "crates/ambre-primitives/driver";

/// The variable through which the driver learns where to write its records.
const OUT: &str = "AMBRE_GATE_OUT";

/// One compile line of a lint recipe, as the `justfile` writes it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Compile {
    /// The value of the `RUSTFLAGS="..."` prefix, if the line sets one.
    pub rustflags: Option<String>,
    /// The arguments of `cargo clippy` before `--`.
    pub args: Vec<String>,
}

impl Compile {
    /// The line as a message prints it.
    fn shown(&self) -> String {
        format!("cargo check {}", self.args.join(" "))
    }
}

/// The compile lines of each recipe of [`LINT_RECIPES`] in the `justfile`
/// text, in order. The parser reads a narrow grammar and fails closed on any
/// other line, since the gate must run each line as `just` runs it:
///
///   * the recipe header is the name and a colon alone, once at the start of
///     a line;
///   * a body line is indented. It is a `#` comment, a `cargo fmt` line,
///     which compiles nothing, or a compile line
///     `[RUSTFLAGS="<flags>"] cargo clippy <args> [-- <tool args>]`;
///   * a word of a compile line holds no shell character, and no argument of
///     [`BANNED_ARGS`];
///   * each recipe holds at least one compile line.
///
/// # Errors
///
/// One message per line the grammar refuses.
pub fn compiles(justfile: &str) -> Result<Vec<Compile>, Vec<String>> {
    let lines: Vec<&str> = justfile.lines().collect();
    let mut found = Vec::new();
    let mut errors = Vec::new();
    for name in LINT_RECIPES {
        let header = format!("{name}:");
        let starts: Vec<usize> = lines
            .iter()
            .enumerate()
            .filter(|(_, line)| {
                line.split_once(':')
                    .is_some_and(|(head, _)| head.split_whitespace().next() == Some(*name))
                    && !line.starts_with(char::is_whitespace)
            })
            .map(|(at, _)| at)
            .collect();
        let [start] = starts.as_slice() else {
            errors.push(format!(
                "{JUSTFILE}: the recipe `{name}` is declared {} times, where the gate reads it \
                 once, so the gate fails closed",
                starts.len()
            ));
            continue;
        };
        if lines.get(*start).map(|line| line.trim_end()) != Some(header.as_str()) {
            errors.push(format!(
                "{JUSTFILE}:{}: the recipe `{name}` takes a parameter or a dependency, which the \
                 gate does not run, so the gate fails closed: write `{header}` alone",
                start + 1
            ));
            continue;
        }
        let before = found.len();
        let body = lines
            .iter()
            .enumerate()
            .skip(start + 1)
            .take_while(|(_, line)| line.is_empty() || line.starts_with(char::is_whitespace));
        for (at, line) in body {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            match compile_line(line) {
                Ok(Some(compile)) => found.push(compile),
                Ok(None) => {}
                Err(why) => errors.push(format!(
                    "{JUSTFILE}:{}: {why}, so the gate cannot show it compiles what the lint \
                     compiles, and fails closed",
                    at + 1
                )),
            }
        }
        if found.len() == before {
            errors.push(format!(
                "{JUSTFILE}: the recipe `{name}` holds no compile line, so the gate reads nothing \
                 of it, and fails closed"
            ));
        }
    }
    if errors.is_empty() {
        Ok(found)
    } else {
        Err(errors)
    }
}

/// One body line of a lint recipe, comments set aside. `None` for a
/// `cargo fmt` line, which compiles nothing.
fn compile_line(line: &str) -> Result<Option<Compile>, String> {
    let (rustflags, rest) = match line.strip_prefix("RUSTFLAGS=\"") {
        Some(quoted) => {
            let Some((flags, rest)) = quoted.split_once('"') else {
                return Err("the RUSTFLAGS value has no closing quote".to_owned());
            };
            if flags.contains(['$', '\\', '`']) {
                return Err("the RUSTFLAGS value asks the shell for an expansion".to_owned());
            }
            if !rest.starts_with(' ') {
                return Err("the RUSTFLAGS value runs into the command".to_owned());
            }
            (Some(flags.to_owned()), rest)
        }
        None => (None, line),
    };
    let words: Vec<&str> = rest.split_whitespace().collect();
    if let Some(word) = words.iter().find(|word| word.contains(SHELL_CHARS)) {
        return Err(format!(
            "the word {word:?} holds a character the shell or just would read"
        ));
    }
    match words.as_slice() {
        ["cargo", "fmt", ..] if rustflags.is_none() => return Ok(None),
        ["cargo", "clippy", ..] => {}
        _ => return Err("the line is no `cargo clippy` or `cargo fmt` line".to_owned()),
    }
    let rest = words.get(2..).unwrap_or_default();
    let args = match rest.iter().position(|word| *word == "--") {
        Some(at) => rest.get(..at).unwrap_or_default(),
        None => rest,
    };
    if let Some(word) = args
        .iter()
        .find(|word| BANNED_ARGS.iter().any(|banned| word.starts_with(banned)))
    {
        return Err(format!(
            "the argument {word:?} changes what the gate reads, or how cargo reports it"
        ));
    }
    Ok(Some(Compile {
        rustflags,
        args: args.iter().map(|word| (*word).to_owned()).collect(),
    }))
}

/// What the compiles gave: the record files of the driver, and the features
/// cargo turned on for a shipped build of each package.
#[derive(Debug, Default)]
pub struct Compiled {
    /// The directory of the record files.
    pub records: PathBuf,
    /// The features of each package, by its canonical manifest.
    pub features: BTreeMap<PathBuf, BTreeSet<String>>,
}

/// A cargo command with no variable of `cargo run` or of a wrapper. It runs
/// at the root of the repository, where rustup picks the pinned toolchain,
/// so no command names one (`ARCH-075`).
fn cargo(root: &Path) -> Command {
    let mut command = Command::new("cargo");
    command.current_dir(root);
    for (name, _) in std::env::vars_os() {
        let drop = name.to_str().is_some_and(|name| {
            RUN_VARS.contains(&name)
                || WRAPPERS.contains(&name)
                || name.starts_with("CARGO_PKG_")
                || name == "CARGO_ENCODED_RUSTFLAGS"
                || name == "RUSTC_WORKSPACE_WRAPPER"
        });
        if drop {
            command.env_remove(name);
        }
    }
    command
}

/// Builds the driver on the pinned nightly and runs every lint line through
/// it. The records land under `target/gate/records`.
///
/// # Errors
///
/// Each failure, in words: a driver that does not build, a line that does not
/// parse or fails, a unit the driver did not read.
pub fn compile(root: &Path) -> Result<Compiled, Vec<String>> {
    let justfile = std::fs::read_to_string(root.join(JUSTFILE))
        .map_err(|e| vec![format!("{JUSTFILE}: {e}")])?;
    let lines = compiles(&justfile)?;
    // Cargo reads its configuration from the root of the disk (`-C /`). The
    // `build-std` key of the repository's config would link a second `std`
    // beside rustc's own. Rustup picks the toolchain where the command starts,
    // the root of the repository.
    let manifest = root.join(DRIVER).join("Cargo.toml");
    let built = cargo(root)
        .args(["-Z", "unstable-options", "-C", "/"])
        .args(["build", "--quiet", "--release", "--manifest-path"])
        .arg(&manifest)
        .status();
    if !built.is_ok_and(|status| status.success()) {
        return Err(vec![format!(
            "the driver {DRIVER} does not build on the pinned toolchain. It needs the \
             rustc-dev component: rustup component add rustc-dev, run from the root of \
             the repository"
        )]);
    }
    let driver = root
        .join(DRIVER)
        .join("target/release/ambre-primitives-driver");
    let sysroot = cargo_sysroot(root)?;
    let target =
        std::env::var_os("CARGO_TARGET_DIR").map_or_else(|| root.join("target"), PathBuf::from);
    let records = target.join("gate/records");
    let _ = std::fs::remove_dir_all(&records);
    forget_workspace_units(&target);
    let mut compiled = Compiled {
        records: records.clone(),
        features: BTreeMap::new(),
    };
    let mut errors = Vec::new();
    let library = std::env::var_os("LD_LIBRARY_PATH").map_or_else(
        || sysroot.join("lib").into_os_string(),
        |old| {
            let mut path = sysroot.join("lib").into_os_string();
            path.push(":");
            path.push(old);
            path
        },
    );
    for line in &lines {
        let mut command = cargo(root);
        command
            .arg("check")
            .args(&line.args)
            .arg("--message-format=json")
            .arg("--quiet")
            .env("RUSTC_WORKSPACE_WRAPPER", &driver)
            .env(OUT, &records)
            .env("LD_LIBRARY_PATH", &library)
            // The incremental cache keys the lint set: a driver rebuilt with
            // another set fails on it with an internal compiler error.
            .env("CARGO_INCREMENTAL", "0");
        if let Some(flags) = &line.rustflags {
            command.env("RUSTFLAGS", flags);
        }
        let started = std::time::Instant::now();
        let output = match command.output() {
            Ok(output) => output,
            Err(e) => {
                errors.push(format!("{}: cannot run cargo: {e}", line.shown()));
                continue;
            }
        };
        // The time of each line, which the budget row of the gate reads
        // (`ambre-spec-budgets.md`).
        println!(
            "primitives: {} in {:.1} s",
            line.shown(),
            started.elapsed().as_secs_f64()
        );
        let stdout = String::from_utf8_lossy(&output.stdout);
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let diagnostics: Vec<String> = stdout.lines().filter_map(error_text).collect();
            let tail: Vec<&str> = stderr.lines().rev().take(20).collect();
            let tail: Vec<&str> = tail.into_iter().rev().collect();
            errors.push(format!(
                "{} fails, so the gate cannot read what rustc compiles:\n{}{}",
                line.shown(),
                diagnostics.join(""),
                tail.join("\n")
            ));
            continue;
        }
        let mut named = 0_usize;
        for message in stdout.lines() {
            match artifact(root, message) {
                Ok(Some(found)) => {
                    named += 1;
                    if !found.test {
                        compiled
                            .features
                            .entry(found.manifest)
                            .or_default()
                            .extend(found.features);
                    }
                    if !found
                        .stems
                        .iter()
                        .any(|stem| records.join(format!("{stem}.tsv")).is_file())
                    {
                        errors.push(format!(
                            "{}: cargo built {} and the driver wrote no record of it, so the \
                             gate cannot read it, and fails closed",
                            line.shown(),
                            found.name
                        ));
                    }
                }
                Ok(None) => {}
                Err(error) => errors.push(format!("{}: {error}", line.shown())),
            }
        }
        if named == 0 {
            errors.push(format!(
                "{} names no unit of the workspace, so the gate reads nothing and fails closed",
                line.shown()
            ));
        }
    }
    if errors.is_empty() {
        Ok(compiled)
    } else {
        Err(errors)
    }
}

/// The sysroot of the pinned toolchain, whose `lib` holds the `rustc_driver`
/// the driver links.
fn cargo_sysroot(root: &Path) -> Result<PathBuf, Vec<String>> {
    let output = Command::new("rustc")
        .args(["--print", "sysroot"])
        .current_dir(root)
        .output()
        .map_err(|e| vec![format!("cannot run rustc: {e}")])?;
    let text = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if !output.status.success() || text.is_empty() {
        return Err(vec!["rustc prints no sysroot".to_owned()]);
    }
    Ok(PathBuf::from(text))
}

/// Drops the fingerprint of every unit of a workspace package, so cargo
/// compiles each one again through the driver. A dependency stays fresh.
fn forget_workspace_units(target: &Path) {
    let Ok(profiles) = std::fs::read_dir(target) else {
        return;
    };
    let mut dirs: Vec<PathBuf> = vec![target.join("debug/.fingerprint")];
    for entry in profiles.flatten() {
        dirs.push(entry.path().join("debug/.fingerprint"));
    }
    for dir in dirs {
        let Ok(units) = std::fs::read_dir(&dir) else {
            continue;
        };
        for unit in units.flatten() {
            if unit.file_name().to_string_lossy().starts_with("ambre-") {
                let _ = std::fs::remove_dir_all(unit.path());
            }
        }
    }
}

/// One unit of the workspace that a message of cargo names.
struct Artifact {
    name: String,
    manifest: PathBuf,
    features: BTreeSet<String>,
    test: bool,
    /// The stem of each file the unit wrote, `lib` set apart: the driver names
    /// its record file after it.
    stems: Vec<String>,
}

/// The unit one message of cargo describes. `None` for another message, and
/// for a unit of a package outside the workspace or vendored into it.
fn artifact(root: &Path, message: &str) -> Result<Option<Artifact>, String> {
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
    let root = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    let manifest = std::fs::canonicalize(manifest).unwrap_or_else(|_| PathBuf::from(manifest));
    if !manifest.starts_with(&root) || manifest.starts_with(root.join("vendor")) {
        return Ok(None);
    }
    let name = value
        .pointer("/target/name")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let kinds: Vec<&str> = value
        .pointer("/target/kind")
        .and_then(serde_json::Value::as_array)
        .map(|kinds| kinds.iter().filter_map(serde_json::Value::as_str).collect())
        .unwrap_or_default();
    let Some(test) = value
        .pointer("/profile/test")
        .and_then(serde_json::Value::as_bool)
    else {
        return Err(format!(
            "{name}: an artifact names no test flag in its profile, so the gate cannot tell a \
             test build from a shipped one, and fails closed"
        ));
    };
    let test = test
        || kinds
            .iter()
            .any(|kind| matches!(*kind, "test" | "bench" | "example"));
    let Some(features) = value
        .get("features")
        .and_then(serde_json::Value::as_array)
        .map(|names| {
            names
                .iter()
                .filter_map(serde_json::Value::as_str)
                .map(str::to_owned)
                .collect()
        })
    else {
        return Err(format!(
            "{name}: an artifact names no feature list, so the gate cannot show which features \
             cargo turned on, and fails closed"
        ));
    };
    let stems = value
        .get("filenames")
        .and_then(serde_json::Value::as_array)
        .map(|names| {
            names
                .iter()
                .filter_map(serde_json::Value::as_str)
                .filter_map(|file| unit_stem(Path::new(file)))
                .collect()
        })
        .unwrap_or_default();
    Ok(Some(Artifact {
        name,
        manifest,
        features,
        test,
        stems,
    }))
}

/// The stem of the record the driver writes for the unit cargo built as
/// `file`.
///
/// A library or a binary is named by its crate and its hash, the record's own
/// stem, with a `lib` prefix dropped. A build script is the exception: cargo
/// names the file `build-script-build` with no hash, in a directory named by
/// the package and the hash. The driver compiled it as the crate
/// `build_script_build` with that hash, so the record takes that name.
fn unit_stem(file: &Path) -> Option<String> {
    let stem = file.file_stem()?.to_string_lossy();
    if stem == "build-script-build" {
        let dir = file.parent()?.file_name()?.to_string_lossy();
        let (_, hash) = dir.rsplit_once('-')?;
        return Some(format!("build_script_build-{hash}"));
    }
    Some(stem.strip_prefix("lib").unwrap_or(&stem).to_owned())
}

/// The rendered text of one compiler error that a message of cargo carries.
fn error_text(message: &str) -> Option<String> {
    let value = serde_json::from_str::<serde_json::Value>(message).ok()?;
    if value.get("reason").and_then(serde_json::Value::as_str) != Some("compiler-message") {
        return None;
    }
    let inner = value.get("message")?;
    if inner.get("level").and_then(serde_json::Value::as_str) != Some("error") {
        return None;
    }
    inner
        .get("rendered")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
}

#[cfg(test)]
#[path = "run_tests.rs"]
mod tests;
