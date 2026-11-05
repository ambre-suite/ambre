//! The files rustc compiles, read from the dep-info cargo writes, and held
//! against the files the walk reads (`ARCH-033`, `ARCH-045`).
//!
//! The walk resolves modules with syn, as rustc does, and every module
//! placement it misreads let a file through unread. rustc writes down, for
//! each crate it compiles, every file it read. A `#[path]` file, an
//! `include!` file and the file of a `mod` a macro writes are all there. So
//! the gate runs each compile of the workspace and holds the walk to that
//! set.
//!
//! The compiles are the lines of the `lint` and `lint-wasm` recipes,
//! read from the `justfile` and run as written. cargo then finds fresh every
//! unit the lint built, so the gate reads the dep-info the lint wrote and
//! compiles nothing again. The clippy configuration and the workspace
//! manifest are dependencies of each unit clippy lints, read as data.
//!
//! The check holds that:
//!
//!   * a file that a shipped build of a crate reads is one the walk reads
//!     from the same crate root. Or a walked file names it as data, with a
//!     string literal. The name of the file decides nothing: rustc compiles
//!     a `.txt` file as a module when a `#[path]` names it;
//!   * a data file that parses as Rust items is refused all the same, since
//!     the dep-info does not tell a module from data. This parse guard is
//!     defence in depth, and best effort: syn and rustc parse apart, so a
//!     file syn refuses passes it. The scan refuses the `mod` a macro
//!     declares, the one way a data file compiles as a module the walk
//!     does not see;
//!   * no build configuration cargo reads enables an unstable feature or a
//!     crate attribute, or names a compiler wrapper. Neither does a variable
//!     of the gate's own environment. So no flag the check reads enables
//!     syntax the guard misreads. A flag set some other way is a
//!     reviewer's (`ARCH-050`);
//!   * a crate root that a build compiles is one the walk starts from;
//!   * a `.rs` file under the workspace root is one that a compile reads,
//!     test builds included. So no file sits in the tree that no build
//!     checks. The nested workspace of the fuzz targets builds apart and is
//!     left out, by its name. A manifest elsewhere that declares a workspace
//!     of its own is refused. Each manifest and each config is parsed as
//!     TOML, as cargo reads it.
//!
//! A failed compile, a missing dep-info file, and a dep-info line the
//! parser cannot read whole all fail the check. A partial set would pass a
//! file no build read. Only a unit of a workspace package is read, so
//! a registry crate and the standard library count for nothing.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::scan::{NESTED_WORKSPACES, SQLX, Scan, Tree, canonical};

/// The cargo subcommand of one compile line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Subcommand {
    Clippy,
    Check,
}

impl Subcommand {
    /// The word cargo takes.
    fn word(self) -> &'static str {
        match self {
            Self::Clippy => "clippy",
            Self::Check => "check",
        }
    }
}

/// One compile line of a lint recipe, as the `justfile` writes it. The gate
/// runs it with its flags, its arguments and its tool arguments. cargo then
/// deems fresh each unit the lint built, and rebuilds none. cargo fingerprints `RUSTFLAGS` and the clippy arguments: a line
/// that differs in either compiles the workspace a second time.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Compile {
    /// The value of the `RUSTFLAGS="..."` prefix, if the line sets one.
    rustflags: Option<String>,
    subcommand: Subcommand,
    /// The arguments before `--`: packages, targets, features, the triple.
    args: Vec<String>,
    /// The arguments after `--`, which cargo hands to clippy.
    tool_args: Vec<String>,
}

impl Compile {
    pub(crate) fn new(
        rustflags: Option<String>,
        subcommand: Subcommand,
        args: Vec<String>,
        tool_args: Vec<String>,
    ) -> Self {
        Self {
            rustflags,
            subcommand,
            args,
            tool_args,
        }
    }

    /// The line as a refusal prints it, without its flags.
    fn shown(&self) -> String {
        let mut words = vec!["cargo", self.subcommand.word()];
        words.extend(self.args.iter().map(String::as_str));
        if !self.tool_args.is_empty() {
            words.push("--");
            words.extend(self.tool_args.iter().map(String::as_str));
        }
        words.join(" ")
    }
}

/// The `justfile`, relative to the workspace root.
pub(crate) const JUSTFILE: &str = "justfile";

/// The recipes whose compile lines the gate runs. `lint` holds the host
/// line, the widest one: every package and every target. `lint-wasm` holds
/// the wasm lines, which compile the code under `cfg(target_arch)` that no
/// host build reads. The gate runs their lines as written, so it reads the
/// dep-info the lint wrote and compiles nothing the lint compiled.
pub(crate) const LINT_RECIPES: &[&str] = &["lint", "lint-wasm"];

/// The arguments a compile line may not carry, since each one changes what
/// the gate reads or how cargo reports it. They name another workspace,
/// another output directory, a config override that can carry flags, an
/// unstable option, or a message format the gate sets itself.
const BANNED_ARGS: &[&str] = &[
    "--manifest-path",
    "--target-dir",
    "--config",
    "--message-format",
    "-Z",
];

/// The characters a word of a compile line may not hold. Each one asks the
/// shell or `just` for an expansion, a quote or a redirect. The gate
/// performs none, so it cannot show it runs the line the lint runs.
const SHELL_CHARS: &[char] = &[
    '"', '\'', '`', '$', '\\', ';', '&', '|', '<', '>', '(', ')', '{', '}', '[', ']', '*', '?',
    '~', '!', '#',
];

/// The compile lines of the lint recipes, read from the `justfile` under
/// `root`. The `justfile` is the one source of the lines: the gate runs
/// what the lint runs, and no second copy can drift from it.
pub(crate) fn lint_compiles(tree: &impl Tree, root: &Path) -> Result<Vec<Compile>, Vec<String>> {
    let Some(text) = tree.read(&root.join(JUSTFILE)) else {
        return Err(vec![format!(
            "{JUSTFILE}: the check cannot read it, so it cannot run the compiles of the lint, \
             and fails closed"
        )]);
    };
    compiles(&text, LINT_RECIPES)
}

/// The compile lines of each recipe of `names` in the `justfile` text, in
/// order. The parser reads a narrow grammar and fails closed on any other
/// line, since the gate must run each line as `just` runs it:
///
///   * the recipe header is the name and a colon alone, once at the start of
///     a line. A parameter or a dependency would run something the gate
///     does not;
///   * a body line is indented. It is a `#` comment, a `cargo fmt` line,
///     which compiles nothing, or a compile line
///     `[RUSTFLAGS="<flags>"] cargo <clippy|check> <args> [-- <tool args>]`;
///   * a word of a compile line holds no shell character, and no argument of
///     `BANNED_ARGS`;
///   * each recipe holds at least one compile line.
pub(crate) fn compiles(justfile: &str, names: &[&str]) -> Result<Vec<Compile>, Vec<String>> {
    let lines: Vec<&str> = justfile.lines().collect();
    let mut found = Vec::new();
    let mut errors = Vec::new();
    for name in names {
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
                "{JUSTFILE}: the recipe `{name}` is declared {} times, where the gate reads \
                 it once, so the check fails closed",
                starts.len()
            ));
            continue;
        };
        if lines.get(*start).map(|line| line.trim_end()) != Some(header.as_str()) {
            errors.push(format!(
                "{JUSTFILE}:{}: the recipe `{name}` takes a parameter or a dependency, which \
                 the gate does not run, so the check fails closed: write `{header}` alone",
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
                    "{JUSTFILE}:{}: {why}, so the gate cannot show it runs the line the lint \
                     runs, and fails closed",
                    at + 1
                )),
            }
        }
        if found.len() == before {
            errors.push(format!(
                "{JUSTFILE}: the recipe `{name}` holds no compile line, so the check reads \
                 nothing of it, and fails closed"
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
    let subcommand = match words.as_slice() {
        ["cargo", "fmt", ..] if rustflags.is_none() => return Ok(None),
        ["cargo", "clippy", ..] => Subcommand::Clippy,
        ["cargo", "check", ..] => Subcommand::Check,
        _ => {
            return Err(
                "the line is no `cargo clippy`, `cargo check` or `cargo fmt` line".to_owned(),
            );
        }
    };
    let rest = words.get(2..).unwrap_or_default();
    let (args, tool_args) = match rest.iter().position(|word| *word == "--") {
        Some(at) => (
            rest.get(..at).unwrap_or_default(),
            rest.get(at + 1..).unwrap_or_default(),
        ),
        None => (rest, &[][..]),
    };
    if let Some(word) = args
        .iter()
        .find(|word| BANNED_ARGS.iter().any(|banned| word.starts_with(banned)))
    {
        return Err(format!(
            "the argument {word:?} changes what the gate reads, or how cargo reports it"
        ));
    }
    let owned = |words: &[&str]| words.iter().map(|word| (*word).to_owned()).collect();
    Ok(Some(Compile::new(
        rustflags,
        subcommand,
        owned(args),
        owned(tool_args),
    )))
}

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

/// The names of the files clippy reads beside the sources: its
/// configuration, and the manifest of the workspace. Clippy lists them in
/// the dep-info of each unit it lints.
const CLIPPY_READS: &[&str] = &["clippy.toml", ".clippy.toml", "Cargo.toml"];

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
    let root = canonical(root);
    let mut units: BTreeMap<(PathBuf, Build), BTreeSet<PathBuf>> = BTreeMap::new();
    let mut errors = Vec::new();
    for compile in compiles {
        let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
        let mut command = Command::new(cargo);
        command
            .current_dir(&root)
            .arg(compile.subcommand.word())
            .args(&compile.args)
            // Neither flag enters a fingerprint, so the units stay fresh.
            .arg("--message-format=json")
            .arg("--quiet")
            // It overrides `RUSTFLAGS`, so a line would drop its own flags.
            .env_remove("CARGO_ENCODED_RUSTFLAGS");
        // `cargo run` sets these for the gate, and `just lint` runs with none
        // of them. An inherited `CARGO_MANIFEST_DIR` makes cargo rebuild
        // units the lint left fresh.
        for (name, _) in std::env::vars_os() {
            if name
                .to_str()
                .is_some_and(|name| RUN_VARS.contains(&name) || name.starts_with("CARGO_PKG_"))
            {
                command.env_remove(name);
            }
        }
        if !compile.tool_args.is_empty() {
            command.arg("--").args(&compile.tool_args);
        }
        if let Some(flags) = &compile.rustflags {
            command.env("RUSTFLAGS", flags);
        }
        configure(&mut command);
        let line = compile.shown();
        let output = match command.output() {
            Ok(output) => output,
            Err(e) => {
                errors.push(format!("{line}: cannot run cargo: {e}"));
                continue;
            }
        };
        let stdout = String::from_utf8_lossy(&output.stdout);
        // A lint the line denies fails it as a compile error does: either
        // way no dep-info is whole, and the gate never passes on it.
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let diagnostics: Vec<String> = stdout.lines().filter_map(error_text).collect();
            let tail: Vec<&str> = stderr.lines().rev().take(20).collect();
            let tail: Vec<&str> = tail.into_iter().rev().collect();
            errors.push(format!(
                "{line} fails, so the check cannot read what rustc compiles:\n{}{}",
                diagnostics.join(""),
                tail.join("\n")
            ));
            continue;
        }
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
                Err(error) => errors.push(format!("{line}: {error}")),
            }
        }
        if named == 0 && errors.len() == before {
            errors.push(format!(
                "{line} names no unit of the workspace, so the check reads nothing and fails \
                 closed"
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

/// The rendered text of one compiler error that a message of cargo carries,
/// so a failed line names what failed. `None` for any other message.
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
    if !canonical(Path::new(manifest)).starts_with(root) {
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
        .map(|path| canonical(&root.join(path)))
        .collect();
    Ok(Some(Unit {
        root: canonical(Path::new(source)),
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
///   * a line is empty, a `#` comment, or a rule `target: dep dep ...`. A
///     comment names no file: rustc writes `# env-dep:NAME=value` for each
///     variable a crate reads, and clippy adds its own, `CLIPPY_ARGS` and
///     `CLIPPY_CONF_DIR`. The files clippy reads, its configuration and the
///     workspace manifest, are dependencies, which `cross_check` reads as
///     data;
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
        return Err(
            "no rule lists a dependency, so the check reads nothing, and fails closed".to_owned(),
        );
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
            if clippy_config(file, root) {
                if parses_as_items(tree, file) {
                    errors.push(format!(
                        "{}: clippy reads it for the crate root {}, and \
                         it parses as Rust items. The dep-info does not tell a module from \
                         data, so the gate cannot show that no `mod` compiles it",
                        shown(file),
                        shown(&unit.root)
                    ));
                }
            } else if !data.contains(file) {
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

/// Whether a file a unit read is one clippy reads: a file of
/// `CLIPPY_READS` under the workspace root. Clippy lists it in the dep-info
/// of each unit it lints, so the gate reads it as data, under the parse
/// guard.
fn clippy_config(file: &Path, root: &Path) -> bool {
    file.starts_with(root)
        && file
            .file_name()
            .is_some_and(|name| CLIPPY_READS.iter().any(|read| name == *read))
}

/// Whether the file is Rust that declares at least one item: the parse
/// guard, defence in depth. A module with no item adds no code. A file that
/// rustc compiles and syn refuses passes the guard, so the guard holds no
/// rule alone. The scan refuses each `mod` a macro declares, and the walk
/// follows each other `mod`, so no `mod` compiles a data file unseen. A
/// shader or a style sheet often lexes as Rust tokens and does not parse.
/// So a refusal of what syn cannot parse would refuse real data.
fn parses_as_items(tree: &impl Tree, file: &Path) -> bool {
    tree.read(file)
        .and_then(|text| syn::parse_file(&text).ok())
        .is_some_and(|parsed| !parsed.items.is_empty())
}

/// Every `.rs` file under the workspace root, canonical. Build output is
/// left out, and so is each nested workspace that `NESTED_WORKSPACES`
/// names. A manifest decides nothing here: one that declares a workspace
/// elsewhere is refused by `stray_workspaces`, and its files stay in.
pub(crate) fn sources(tree: &impl Tree, root: &Path) -> BTreeSet<PathBuf> {
    let nested: Vec<PathBuf> = NESTED_WORKSPACES.iter().map(|dir| root.join(dir)).collect();
    tree.files(root)
        .into_iter()
        .filter(|path| path.extension().is_some_and(|ext| ext == "rs"))
        .filter(|path| !nested.iter().any(|dir| path.starts_with(dir)))
        .map(|path| tree.canonical(&path))
        .collect()
}

/// Each manifest under the workspace root, the root's own and those of the
/// nested workspaces apart, that declares a workspace of its own. Such a
/// directory builds apart from the compiles the check runs, so no shipped
/// build of it is held. The manifest is parsed as TOML, as cargo reads it,
/// and any top-level `workspace` key counts, in any spelling. A manifest
/// that does not parse is `levers`'s to refuse, so this check skips one.
pub(crate) fn stray_workspaces(tree: &impl Tree, root: &Path) -> Vec<String> {
    let kept: Vec<PathBuf> = std::iter::once(root.join("Cargo.toml"))
        .chain(
            NESTED_WORKSPACES
                .iter()
                .map(|dir| root.join(dir).join("Cargo.toml")),
        )
        .collect();
    tree.files(root)
        .into_iter()
        .filter(|path| path.file_name().is_some_and(|name| name == "Cargo.toml"))
        .filter(|path| !kept.contains(path))
        .filter_map(|path| {
            let shown = path.strip_prefix(root).unwrap_or(&path).display();
            tree.read(&path)
                .is_some_and(|text| declares_workspace(&text) == Ok(true))
                .then(|| {
                    format!(
                        "{shown}: the manifest declares a workspace of its own, which builds \
                         apart from the compiles the check runs. Make it a member of the root \
                         workspace, or name its directory in NESTED_WORKSPACES"
                    )
                })
        })
        .collect()
}

/// Whether a manifest declares a workspace: a top-level `workspace` key. A
/// `workspace` key inside `[package]` names the workspace of a member, and
/// declares none.
pub(crate) fn declares_workspace(manifest: &str) -> Result<bool, String> {
    toml_table(manifest).map(|table| table.contains_key("workspace"))
}

/// The text parsed as one TOML table, or the first line of the parser's
/// error.
fn toml_table(text: &str) -> Result<toml::Table, String> {
    text.parse::<toml::Table>().map_err(|error| {
        error
            .to_string()
            .lines()
            .next()
            .unwrap_or_default()
            .to_owned()
    })
}

/// The refusal of a TOML file the check cannot parse.
fn unparsed(shown: &str, error: &str) -> String {
    format!(
        "{shown}: the file does not parse as TOML, so the check cannot show what it sets, and \
         fails closed: {error}"
    )
}

/// One key of an `[unstable]` table of a cargo config that the build needs,
/// admitted by its file, its key and its exact value. No pattern admits one.
struct Unstable {
    /// The config file, relative to the workspace root.
    config: &'static str,
    key: &'static str,
    value: &'static [&'static str],
}

/// The unstable cargo settings the build needs. `build-std` rebuilds the
/// standard library for the threaded wasm build, against `+atomics`. It
/// enables no feature and sets no crate attribute on a workspace crate.
const UNSTABLE: &[Unstable] = &[Unstable {
    config: ".cargo/config.toml",
    key: "build-std",
    value: &["std", "panic_abort"],
}];

/// The recipe files whose command lines set the flags of a build: the
/// `justfile` and the CI workflow, relative to the workspace root.
const RECIPES: &[&str] = &[JUSTFILE, ".github/workflows/ci.yml"];

/// The keys whose value cargo passes to rustc or rustdoc as flags.
const FLAG_KEYS: &[&str] = &["rustflags", "rustdocflags"];

/// The keys of a cargo config that name the compiler, or a wrapper that
/// runs it. Either one can pass rustc any flag the check never reads, so a
/// config fails closed on each, at any depth: `build.rustc` and the like.
const COMPILER_KEYS: &[&str] = &["rustc", "rustc-wrapper", "rustc-workspace-wrapper"];

/// One entry of an `[env]` table of a cargo config that the build needs,
/// admitted by its file, its key and its exact value. No pattern admits one.
struct ConfigEnv {
    /// The config file, relative to the workspace root.
    config: &'static str,
    key: &'static str,
    value: &'static str,
}

/// The `[env]` entries the build needs. None does today. An entry sets the
/// environment of rustc and of each build script, so it can carry a flag
/// variable such as `RUSTFLAGS` past the check.
const CONFIG_ENV: &[ConfigEnv] = &[];

/// Each lever of the build configuration that enables an unstable feature
/// or a crate attribute, as one refusal (`ARCH-033`). Such a lever enables
/// syntax the pinned syn may not parse, as a `#![feature]` does. A data
/// file could then compile as a module the parse guard misreads. The check
/// reads four sources:
///
///   * each `.cargo/config.toml` and `.cargo/config` under the root. A `-Z`
///     flag, `crate-attr` or `feature(` in a `rustflags` or `rustdocflags`
///     value fails, at any depth. So does each key of an `[unstable]` table
///     that `UNSTABLE` does not name with its exact value. A key of
///     `COMPILER_KEYS`, an `include` key and each `[env]` entry that
///     `CONFIG_ENV` does not name fail too;
///   * each `Cargo.toml` under the root: a `cargo-features` key, and a
///     `rustflags` or `rustdocflags` key at any depth;
///   * the `justfile`;
///   * the CI workflow: a non-comment line of either that holds a `-Z` flag,
///     `crate-attr` or `feature(`.
///
/// A config file or a manifest that does not parse fails closed. A manifest
/// that puts another crate under the name `sqlx` fails too, since the scan
/// reads `sqlx::migrate!` as data by its name.
///
/// A config in a crate directory is read too, though cargo run from the
/// root reads none. Its levers are refused all the same: that fails closed,
/// and costs nothing.
pub(crate) fn levers(tree: &impl Tree, root: &Path) -> Vec<String> {
    let mut found = Vec::new();
    for path in tree.files(root) {
        let shown = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .display()
            .to_string();
        let name = path.file_name().unwrap_or_default();
        let in_cargo = path
            .parent()
            .and_then(Path::file_name)
            .is_some_and(|dir| dir == ".cargo");
        if in_cargo && (name == "config" || name == "config.toml") {
            match tree.read(&path).as_deref().map(toml_table) {
                Some(Ok(table)) => config_levers(&shown, &table, &mut found),
                Some(Err(error)) => found.push(unparsed(&shown, &error)),
                None => found.push(unparsed(&shown, "it is not UTF-8 text")),
            }
        } else if name == "Cargo.toml" {
            match tree.read(&path).as_deref().map(toml_table) {
                Some(Ok(table)) => manifest_levers(&shown, &table, &mut found),
                Some(Err(error)) => found.push(unparsed(&shown, &error)),
                None => found.push(unparsed(&shown, "it is not UTF-8 text")),
            }
        }
    }
    for recipe in RECIPES {
        let Some(text) = tree.read(&root.join(recipe)) else {
            found.push(format!(
                "{recipe}: the check cannot read it, so it cannot show that its flags enable \
                 no unstable feature, and fails closed"
            ));
            continue;
        };
        for (at, line) in text.lines().enumerate() {
            if !line.trim_start().starts_with('#') && lever(line) {
                found.push(lever_refusal(&format!("{recipe}:{}", at + 1), "the line"));
            }
        }
    }
    found
}

/// The environment variables whose value cargo passes to rustc or rustdoc
/// as flags. A `CARGO_TARGET_<triple>_RUSTFLAGS` counts too, for any triple.
const FLAG_VARS: &[&str] = &[
    "RUSTFLAGS",
    "CARGO_ENCODED_RUSTFLAGS",
    "CARGO_BUILD_RUSTFLAGS",
    "RUSTDOCFLAGS",
    "CARGO_ENCODED_RUSTDOCFLAGS",
    "CARGO_BUILD_RUSTDOCFLAGS",
];

/// The environment variables that name the compiler or a wrapper that runs
/// it. A `CARGO_BUILD_RUSTC` prefix counts too, for each of its forms.
const COMPILER_VARS: &[&str] = &["RUSTC", "RUSTC_WRAPPER", "RUSTC_WORKSPACE_WRAPPER"];

/// Each lever the gate's own environment sets, as one refusal (`ARCH-033`).
/// The compiles inherit the environment, so a flag variable or a wrapper
/// there reaches rustc as a config does:
///
///   * a variable of `FLAG_VARS`, or a `CARGO_TARGET_*_RUSTFLAGS`, that
///     holds a `-Z` flag, `crate-attr` or `feature(`;
///   * any `CARGO_UNSTABLE_*` variable, as an `[unstable]` key;
///   * any variable of `COMPILER_VARS`, or a `CARGO_BUILD_RUSTC*`, set at
///     all.
///
/// A value that is not UTF-8 fails closed. The `justfile` sets `RUSTFLAGS`
/// on its wasm lines, but no lever in it. Neither the `justfile` nor the CI
/// workflow names a wrapper, so no allowlist admits a value. `vars` is
/// the environment, passed in, so a test sets none of the process.
pub(crate) fn env_levers(vars: impl IntoIterator<Item = (OsString, OsString)>) -> Vec<String> {
    let mut vars: Vec<(OsString, OsString)> = vars.into_iter().collect();
    vars.sort();
    let mut found = Vec::new();
    for (name, value) in vars {
        let name = name.to_string_lossy();
        let flags = FLAG_VARS.contains(&name.as_ref())
            || (name.starts_with("CARGO_TARGET_") && name.ends_with("_RUSTFLAGS"));
        if name.starts_with("CARGO_UNSTABLE_") {
            found.push(format!(
                "environment: `{name}` is an unstable cargo setting, so the check fails closed \
                 (ARCH-033): unset it"
            ));
        } else if COMPILER_VARS.contains(&name.as_ref()) || name.starts_with("CARGO_BUILD_RUSTC") {
            found.push(format!(
                "environment: `{name}` names the compiler or a wrapper that runs it, which can \
                 pass rustc a flag the check does not read, so the check fails closed \
                 (ARCH-033): unset it"
            ));
        } else if flags {
            match value.to_str() {
                Some(text) if lever(text) => {
                    found.push(lever_refusal("environment", &format!("`{name}`")));
                }
                Some(_) => {}
                None => found.push(format!(
                    "environment: `{name}` is not UTF-8, so the check cannot read its flags, \
                     and fails closed (ARCH-033): unset it"
                )),
            }
        }
    }
    found
}

/// The levers of each cargo config cargo reads outside the root. Those are
/// the `.cargo/config.toml` and `.cargo/config` of each directory above it,
/// and the `config.toml` and `config` of `cargo_home`. A file that does not
/// exist counts for nothing. The check reads them as it reads one under the
/// root, and no `UNSTABLE` entry admits a key of one.
pub(crate) fn outer_levers(
    tree: &impl Tree,
    root: &Path,
    cargo_home: Option<&Path>,
) -> Vec<String> {
    let above = root
        .ancestors()
        .skip(1)
        .flat_map(|dir| ["config.toml", "config"].map(|name| dir.join(".cargo").join(name)));
    let home = cargo_home
        .into_iter()
        .flat_map(|dir| ["config.toml", "config"].map(|name| dir.join(name)));
    let configs: BTreeSet<PathBuf> = above.chain(home).collect();
    let mut found = Vec::new();
    for path in configs {
        let shown = path.display().to_string();
        match tree.read(&path).as_deref().map(toml_table) {
            Some(Ok(table)) => config_levers(&shown, &table, &mut found),
            Some(Err(error)) => found.push(unparsed(&shown, &error)),
            None => {}
        }
    }
    found
}

/// The levers of one cargo config, parsed.
fn config_levers(shown: &str, table: &toml::Table, found: &mut Vec<String>) {
    if table.contains_key("include") {
        found.push(format!(
            "{shown}: `include` loads another config file, which the check does not read, so \
             the check fails closed (ARCH-033): remove it"
        ));
    }
    if let Some(env) = table.get("env") {
        let entries: Vec<(String, Option<&toml::Value>)> = match env.as_table() {
            Some(entries) => entries
                .iter()
                .map(|(key, value)| (key.clone(), Some(value)))
                .collect(),
            None => vec![(String::new(), None)],
        };
        for (key, value) in entries {
            let admitted = CONFIG_ENV.iter().any(|entry| {
                entry.config == shown
                    && entry.key == key
                    && value.and_then(toml::Value::as_str) == Some(entry.value)
            });
            if !admitted {
                let dotted = if key.is_empty() {
                    "env".to_owned()
                } else {
                    format!("env.{key}")
                };
                found.push(format!(
                    "{shown}: `{dotted}` sets the environment of rustc, which can carry a flag \
                     the check does not read, and CONFIG_ENV does not name it with this value, \
                     so the check fails closed (ARCH-033): remove it, or name it in CONFIG_ENV \
                     with the reason"
                ));
            }
        }
    }
    if let Some(unstable) = table.get("unstable") {
        let keys: Vec<(String, Option<&toml::Value>)> = match unstable.as_table() {
            Some(keys) => keys
                .iter()
                .map(|(key, value)| (format!("unstable.{key}"), Some(value)))
                .collect(),
            None => vec![("unstable".to_owned(), None)],
        };
        for (key, value) in keys {
            let admitted = UNSTABLE.iter().any(|entry| {
                entry.config == shown
                    && key.strip_prefix("unstable.") == Some(entry.key)
                    && value.and_then(toml::Value::as_array).is_some_and(|items| {
                        items.len() == entry.value.len()
                            && items
                                .iter()
                                .zip(entry.value)
                                .all(|(item, want)| item.as_str() == Some(*want))
                    })
            });
            if !admitted {
                found.push(format!(
                    "{shown}: `{key}` is an unstable cargo setting that UNSTABLE does \
                     not name with this value, so the check fails closed (ARCH-033): remove \
                     it, or name it in UNSTABLE with the reason"
                ));
            }
        }
    }
    flag_values(shown, "", table, found);
}

/// Each `rustflags` or `rustdocflags` value of `table`, at any depth, that
/// holds a lever, and each key of `COMPILER_KEYS`. `prefix` is the dotted
/// key of `table`.
fn flag_values(shown: &str, prefix: &str, table: &toml::Table, found: &mut Vec<String>) {
    for (key, value) in table {
        let dotted = if prefix.is_empty() {
            key.clone()
        } else {
            format!("{prefix}.{key}")
        };
        if COMPILER_KEYS.contains(&key.as_str()) {
            found.push(format!(
                "{shown}: `{dotted}` names the compiler or a wrapper that runs it, which can \
                 pass rustc a flag the check does not read, so the check fails closed \
                 (ARCH-033): remove it"
            ));
        } else if FLAG_KEYS.contains(&key.as_str()) {
            let words: Vec<String> = match value {
                toml::Value::Array(items) => items.iter().map(flag_text).collect(),
                other => vec![flag_text(other)],
            };
            if words.iter().any(|word| lever(word)) {
                found.push(lever_refusal(shown, &format!("`{dotted}`")));
            }
        } else if let Some(inner) = value.as_table() {
            flag_values(shown, &dotted, inner, found);
        }
    }
}

/// The text of one flag value: a string as written, any other value in its
/// debug form. A value of another type still meets the check.
fn flag_text(value: &toml::Value) -> String {
    value
        .as_str()
        .map_or_else(|| format!("{value:?}"), str::to_owned)
}

/// The levers of one manifest, parsed.
fn manifest_levers(shown: &str, table: &toml::Table, found: &mut Vec<String>) {
    if table.contains_key("cargo-features") {
        found.push(format!(
            "{shown}: `cargo-features` enables an unstable cargo feature, so the check fails \
             closed (ARCH-033): remove it"
        ));
    }
    // A patch or a replace of `sqlx` puts another crate under its name.
    let patched = table
        .get("patch")
        .and_then(toml::Value::as_table)
        .into_iter()
        .flat_map(|sources| sources.iter())
        .filter(|(_, source)| source.get(SQLX).is_some())
        .map(|(source, _)| format!("patch.{source}.{SQLX}"));
    let replaced = table
        .get("replace")
        .and_then(toml::Value::as_table)
        .into_iter()
        .flat_map(|specs| specs.keys())
        .filter(|spec| spec.contains(SQLX))
        .map(|spec| format!("replace.{spec}"));
    for dotted in patched.chain(replaced) {
        found.push(substituted(shown, &dotted));
    }
    manifest_keys(shown, "", table, found);
}

/// The refusal of a manifest key that puts another crate under the name
/// `sqlx`.
fn substituted(shown: &str, dotted: &str) -> String {
    format!(
        "{shown}: `{dotted}` puts another crate under the name sqlx. The scan reads \
         `sqlx::migrate!` as a data read by its name, so the check fails closed (ARCH-033): \
         keep the name `sqlx` for the sqlx crate of the registry alone"
    )
}

/// Each flag key of a manifest, at any depth, and each `sqlx` dependency
/// renamed to another package. `prefix` is the dotted key of `table`.
fn manifest_keys(shown: &str, prefix: &str, table: &toml::Table, found: &mut Vec<String>) {
    for (key, value) in table {
        let dotted = if prefix.is_empty() {
            key.clone()
        } else {
            format!("{prefix}.{key}")
        };
        if FLAG_KEYS.contains(&key.as_str()) {
            found.push(format!(
                "{shown}: `{dotted}` passes flags to rustc from a manifest, so the check fails \
                 closed (ARCH-033): remove it"
            ));
            continue;
        }
        let Some(inner) = value.as_table() else {
            continue;
        };
        let sqlx = inner.get(SQLX).filter(|_| key.ends_with("dependencies"));
        let renamed = sqlx
            .and_then(|dependency| dependency.get("package"))
            .is_some_and(|package| package.as_str() != Some(SQLX));
        if renamed {
            found.push(format!(
                "{shown}: `{dotted}.sqlx` names another package. The scan reads `sqlx::migrate!` \
                 as a data read by its name, so the check fails closed (ARCH-033): keep the \
                 name `sqlx` for the sqlx crate alone"
            ));
        }
        if sqlx.is_some_and(|dependency| {
            dependency.get("path").is_some() || dependency.get("git").is_some()
        }) {
            found.push(substituted(shown, &format!("{dotted}.{SQLX}")));
        }
        manifest_keys(shown, &dotted, inner, found);
    }
}

/// Whether `text` holds a lever: a `-Z` flag, `crate-attr`, or `feature(`
/// with any spaces. A `-Z` counts where no letter or digit comes before it,
/// so `[A-Z]` is no flag.
fn lever(text: &str) -> bool {
    let joined: String = text.split_whitespace().collect();
    joined.contains("crate-attr")
        || joined.contains("feature(")
        || text.match_indices("-Z").any(|(at, _)| {
            at.checked_sub(1)
                .and_then(|before| text.as_bytes().get(before))
                .is_none_or(|byte| !byte.is_ascii_alphanumeric())
        })
}

/// The refusal of a lever that `what` of `shown` holds.
fn lever_refusal(shown: &str, what: &str) -> String {
    format!(
        "{shown}: {what} holds a -Z flag, a crate-attr or a feature( lever, which enables syntax \
         the gate may not parse, so the check fails closed (ARCH-033): remove it"
    )
}

#[cfg(test)]
#[path = "depinfo_tests.rs"]
mod tests;
