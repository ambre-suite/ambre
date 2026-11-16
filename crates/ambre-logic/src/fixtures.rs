// ambre-logic/src/fixtures.rs
// ============================================================================
// The document-fixture loader, registry and driver (`ambre-spec-fixtures.md`).
//
// Three homes, one dependency grain: the *types* in `ambre-core`, where the
// orphan rule needs `FixtureGen` upstream of both app crates. The *generators*
// sit next to the engine whose commands they drive; the *loader and driver* live
// here, over every `DocState` variant.
//
// Fixture code (I4) behind the non-default `fixtures` feature, since
// `ambre-logic` ships to wasm; `DocState::import_snapshot` and
// `DocState::export_snapshot` stay ungated production code in `lib.rs`.
//
// Not `#[cfg(test)]`: such a module is reachable only from its own crate's test
// harness, whereas `just fixtures` invokes the driver from a binary.
// ============================================================================

use ambre_core::Version;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use ambre_client_core::docfile::{AMBRE_VERSION, Container, DocFileError, read_ambre_plaintext};
use ambre_core::id::{Document, Id};
use ambre_core::{
    DocType, FIXTURE_IMPORT_PEER, FixtureGen, FixtureManifest, FixtureName, FixtureShape, Schema,
};

use ambre_prose::ProseState;
use ambre_prose::logic::fixtures::PROSE_FIXTURES;
use ambre_toile::logic::state::ToileState;
use ambre_toile::logic::state::fixtures::{TOILE_BENCH_FIXTURES, TOILE_FIXTURES};
use ambre_trame::logic::TrameState;
use ambre_trame::logic::fixtures::{TRAME_BENCH_FIXTURES, TRAME_FIXTURES};

use crate::DocState;
use ambre_core::{Count, NarrowCount, Ns, Refused, Refuses, Rule, TimeSpan, Validated};

// -----------------------------------------------------------------------------
// On-disk layout
// -----------------------------------------------------------------------------

/// The corpus root, relative to the workspace root.
pub const CORPUS_DIR: &str = "fixtures";
/// Artifacts a live generator produces; rewritten by every `just fixtures` run.
pub const CURRENT_DIR: &str = "current";
/// Artifacts copied out of `current/` at a version bump; **never** regenerated,
/// because no current generator can produce them.
pub const FROZEN_DIR: &str = "frozen";
/// Scale corpora: produced on demand, git-ignored.
pub const BENCH_DIR: &str = "bench";

/// Extension of the document artifact itself -- an ordinary plaintext `.ambre`.
pub const ARTIFACT_EXT: &str = "ambre";
/// Extension of the JSON sidecar carrying the artifact's provenance.
pub const MANIFEST_EXT: &str = "manifest";

/// Committed benchmark baselines, relative to the workspace root
/// (`FIXTURE-023`). One file per machine, named after it.
pub const BASELINE_DIR: &str = "benches/baselines";

// -----------------------------------------------------------------------------
// Benchmark baselines (`FIXTURE-023`)
// -----------------------------------------------------------------------------

/// One committed set of benchmark figures, and the machine that produced them.
///
/// **The machine is not optional.** `criterion` measures wall clock, so a figure
/// from one machine means nothing on another. A baseline read as an absolute
/// target then fails on hardware rather than on code (`FIXTURE-023`). The type
/// makes the field unskippable, and [`BenchBaseline::names_its_machine`] refuses
/// an empty one, which the type cannot.
#[derive(serde::Serialize, serde::Deserialize, PartialEq, Eq, Clone, Debug)]
pub struct BenchBaseline {
    /// Free text describing the hardware: the CPU model, the core count, the
    /// operating system. No field set survives contact with the machines
    /// contributors use. A reader recognizes a machine rather than parsing it.
    pub machine: String,

    /// The commit the run covered, so a later reader knows which tree the
    /// figures describe. `None` is a checkout git did not answer for, written
    /// `unknown` in the file.
    #[serde(with = "unknown_commit")]
    pub commit: Option<Validated<CommitHash>>,

    /// One entry per benchmark: its `criterion` id, and the mean in nanoseconds.
    /// Nanoseconds because that is what `criterion` reports, and a converted
    /// figure is one more thing to get wrong.
    pub measurements: Vec<BenchMeasurement>,
}

/// The rule of a git commit id, abbreviated or full: 4 to 40 lowercase
/// hexadecimal digits. A `Validated<CommitHash>` holds one.
pub enum CommitHash {}

/// A text that is no commit id: too short, too long, or not lowercase hex.
/// The error, `Refused<CommitHash>`, echoes the refused text.
impl Refuses for CommitHash {
    type Raw = String;
    const NAME: ambre_core::StaticKey<ambre_core::DebugNames> =
        ambre_core::StaticKey::new("InvalidCommitHash");

    fn message(raw: &Self::Raw, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{raw:?} is not a commit id")
    }
}

impl Rule for CommitHash {
    type Error = Refused<CommitHash>;

    const LOGGED_AS: Option<&'static str> = Some("CommitHash");

    fn admit(text: String) -> Result<String, Refused<CommitHash>> {
        let hex = text.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'));
        if hex && (4..=40).contains(&text.len()) {
            Ok(text)
        } else {
            Err(Refused::new(text))
        }
    }
}

/// The file form of a baseline's commit: the id, or `unknown` for none.
mod unknown_commit {
    use super::CommitHash;
    use ambre_core::Validated;
    use serde::{Deserialize, Deserializer, Serializer};

    const UNKNOWN: &str = "unknown";

    pub(super) fn serialize<S: Serializer>(
        commit: &Option<Validated<CommitHash>>,
        out: S,
    ) -> Result<S::Ok, S::Error> {
        out.serialize_str(commit.as_ref().map_or(UNKNOWN, Validated::as_str))
    }

    pub(super) fn deserialize<'de, D: Deserializer<'de>>(
        input: D,
    ) -> Result<Option<Validated<CommitHash>>, D::Error> {
        let text = String::deserialize(input)?;
        if text == UNKNOWN {
            return Ok(None);
        }
        Validated::parse(text)
            .map(Some)
            .map_err(serde::de::Error::custom)
    }
}

/// One benchmark's mean, as `criterion` estimated it.
#[derive(serde::Serialize, serde::Deserialize, PartialEq, Eq, Clone, Debug)]
pub struct BenchMeasurement {
    pub id: String,
    pub mean_ns: TimeSpan<Ns>,
}

impl BenchBaseline {
    /// Whether this baseline names its machine (`FIXTURE-023`).
    ///
    /// A present but blank field is the failure the type cannot express, and the
    /// one a hurried contributor produces.
    pub fn names_its_machine(&self) -> bool {
        !self.machine.trim().is_empty()
    }
}

/// Collect the mean of every benchmark `criterion` last wrote under `dir`.
///
/// Reads `<dir>/**/new/estimates.json`, which is where `criterion` puts the
/// estimates of the most recent run. The `criterion` id is the directory path
/// below `dir`, which is how a grouped benchmark keeps its group in its name.
///
/// An empty result means no bench has run, which the caller reports rather than
/// committing an empty baseline.
///
/// # Errors
///
/// [`FixtureError::Io`] when a directory or an estimate file cannot be read.
pub fn criterion_means(dir: &Path) -> Result<Vec<BenchMeasurement>, FixtureError> {
    let mut out = Vec::new();
    // A tree that is not there is a run that has not happened. The caller says
    // so, with the recipe to fix it; an IO error here would bury that message.
    if !dir.is_dir() {
        return Ok(out);
    }
    collect_means(dir, dir, &mut out)?;
    // Sorted by id, so two runs on one machine produce a reviewable diff rather
    // than a reordering.
    out.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(out)
}

/// Walk `dir` for `new/estimates.json` files, naming each by its path below
/// `root`.
fn collect_means(
    root: &Path,
    dir: &Path,
    out: &mut Vec<BenchMeasurement>,
) -> Result<(), FixtureError> {
    for entry in fs::read_dir(dir)?.filter_map(Result::ok) {
        let path = entry.path();
        if path.is_dir() {
            collect_means(root, &path, out)?;
            continue;
        }
        // `base/estimates.json` is the previous run, which a fresh baseline has
        // no use for.
        let is_new_estimate = path.file_name().is_some_and(|f| f == "estimates.json")
            && path.parent().is_some_and(|p| p.ends_with("new"));
        if !is_new_estimate {
            continue;
        }
        // The id is the path below `root`, minus the trailing `new`.
        let Some(id) = path
            .parent()
            .and_then(|p| p.parent())
            .and_then(|p| p.strip_prefix(root).ok())
            .map(|p| p.to_string_lossy().replace('\\', "/"))
        else {
            continue;
        };
        // `criterion` reports a float; a mean in nanoseconds is an integer to
        // every reader, and the fraction is far below the measurement noise.
        let text = fs::read_to_string(&path)?;
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
            continue;
        };
        let Some(mean) = value
            .get("mean")
            .and_then(|m| m.get("point_estimate"))
            .and_then(serde_json::Value::as_f64)
        else {
            continue;
        };
        out.push(BenchMeasurement {
            id,
            mean_ns: TimeSpan::<Ns>::from_mean(mean),
        });
    }
    Ok(())
}

/// Every committed baseline under `root`, with the file each came from.
///
/// The belt walks them and refuses one that names no machine. A file that is
/// neither JSON nor a baseline raises an error rather than a skip. A baseline
/// nobody can read is not a baseline.
///
/// # Errors
///
/// [`FixtureError::Io`] on an unreadable file, and [`FixtureError::Manifest`]
/// on a file that is not a baseline.
pub fn baselines_in(dir: &Path) -> Result<Vec<(PathBuf, BenchBaseline)>, FixtureError> {
    if !dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    let mut paths: Vec<PathBuf> = fs::read_dir(dir)?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .collect();
    // Sorted, so a report reads the same way twice.
    paths.sort();
    for path in paths {
        let text = fs::read_to_string(&path)?;
        let baseline =
            serde_json::from_str::<BenchBaseline>(&text).map_err(|e| FixtureError::Manifest {
                path: path.clone(),
                reason: e.to_string(),
            })?;
        out.push((path, baseline));
    }
    Ok(out)
}

// -----------------------------------------------------------------------------
// The generator registry
// -----------------------------------------------------------------------------

/// Every generator this build knows, in the order the driver walks them.
///
/// The per-engine registries, concatenated rather than spelled out here.
/// Each generator sits next to the engine whose commands it drives, so adding
/// one takes a single edit in that crate. A second list here is the edit a
/// contributor forgets, and the gate would then report a generator-less
/// artifact rather than the truth.
///
/// The driver takes its registry as a parameter ([`generate_into`]) rather than
/// reading this constant directly. The belt then drives the same code path with
/// a synthetic registry, the pattern `ambre_client_core::docfile`'s migration walk
/// already uses.
pub const REGISTRY: &[&dyn FixtureGen] = &merged_registry();

/// The per-engine registries, concatenated at compile time.
///
/// A `const fn` rather than a hand-written list, for the reason [`REGISTRY`]
/// states. The `while` loops are what a `const fn` has instead of an iterator.
#[expect(
    clippy::indexing_slicing,
    reason = "const evaluation turns an index out of bounds into a build error"
)]
const fn merged_registry() -> [&'static dyn FixtureGen; REGISTRY_LEN] {
    // Every slot is overwritten below; the repeated element only gives the array
    // a value to start from, which a `const fn` cannot obtain any other way.
    let mut out: [&dyn FixtureGen; REGISTRY_LEN] = [TRAME_FIXTURES[0]; REGISTRY_LEN];
    let mut i = 0;
    while i < TRAME_FIXTURES.len() {
        out[i] = TRAME_FIXTURES[i];
        i += 1;
    }
    let mut i = 0;
    while i < TOILE_FIXTURES.len() {
        out[TRAME_FIXTURES.len() + i] = TOILE_FIXTURES[i];
        i += 1;
    }
    let mut i = 0;
    while i < PROSE_FIXTURES.len() {
        out[TRAME_FIXTURES.len() + TOILE_FIXTURES.len() + i] = PROSE_FIXTURES[i];
        i += 1;
    }
    out
}

const REGISTRY_LEN: usize = TRAME_FIXTURES.len() + TOILE_FIXTURES.len() + PROSE_FIXTURES.len();

/// Every scale generator this build knows (`FIXTURE-022`).
///
/// A registry of its own rather than more entries in [`REGISTRY`], because the
/// two go to two directories. `just fixtures` writes [`REGISTRY`] into
/// `current/`, which git carries. This one writes into `bench/`, which the
/// ignore file covers. One registry carrying both would drop a multi-megabyte
/// artifact into the working copy on every regeneration.
pub const BENCH_REGISTRY: &[&dyn FixtureGen] = &merged_bench_registry();

/// Both per-engine scale registries, concatenated at compile time. The reasoning
/// is [`merged_registry`]'s.
#[expect(
    clippy::indexing_slicing,
    reason = "const evaluation turns an index out of bounds into a build error"
)]
const fn merged_bench_registry() -> [&'static dyn FixtureGen; BENCH_REGISTRY_LEN] {
    let mut out: [&dyn FixtureGen; BENCH_REGISTRY_LEN] =
        [TRAME_BENCH_FIXTURES[0]; BENCH_REGISTRY_LEN];
    let mut i = 0;
    while i < TRAME_BENCH_FIXTURES.len() {
        out[i] = TRAME_BENCH_FIXTURES[i];
        i += 1;
    }
    let mut i = 0;
    while i < TOILE_BENCH_FIXTURES.len() {
        out[TRAME_BENCH_FIXTURES.len() + i] = TOILE_BENCH_FIXTURES[i];
        i += 1;
    }
    out
}

const BENCH_REGISTRY_LEN: usize = TRAME_BENCH_FIXTURES.len() + TOILE_BENCH_FIXTURES.len();

/// One generator of a registry, under the name it parsed once for the run.
struct Named<'r> {
    name: Validated<FixtureName>,
    generator: &'r dyn FixtureGen,
}

/// Every generator of `registry` under its name, in registry order.
///
/// The one parse of each name a walk makes. A refused name stops the walk.
/// The run then fails with the refused text, before it touches a file.
fn named<'r>(registry: &[&'r dyn FixtureGen]) -> Result<Vec<Named<'r>>, FixtureError> {
    registry
        .iter()
        .map(|generator| {
            Ok(Named {
                name: generator.name()?,
                generator: *generator,
            })
        })
        .collect()
}

// -----------------------------------------------------------------------------
// Errors
// -----------------------------------------------------------------------------

/// Failures of the *corpus*, distinct from failures of the *container*.
///
/// [`DocFileError`] (`docfile`) stays the authority on malformed bytes; this
/// wraps it and adds the conditions only a corpus has.
#[derive(Debug, thiserror::Error)]
pub enum FixtureError {
    /// No such artifact under the corpus root.
    #[error("no fixture at `{}` in the corpus", path.display())]
    NotFound { path: PathBuf },

    /// A generator or a command line names a fixture with a text the rule of
    /// [`FixtureName`] refuses. The error echoes the text.
    #[error("{0}")]
    Name(#[from] Refused<FixtureName>),

    /// A file of the corpus tree carries a stem that no fixture name, or no
    /// frozen stem, reads as. No generator writes such a file.
    #[error("`{}` is no artifact stem: {reason}", path.display())]
    Stem { path: PathBuf, reason: String },

    /// Unreadable corpus root, permissions, IO.
    #[error("fixture corpus IO error: {0}")]
    Io(#[from] std::io::Error),

    /// The bytes are not a valid `.ambre`. Carried verbatim, never
    /// reinterpreted. A caller that separates a bad magic from an unsupported
    /// version matches on the inner error, not on a flattened string.
    #[error("fixture is not a valid .ambre container: {0}")]
    Container(#[from] DocFileError),

    /// No engine could be built for `doc_type`: `Murmure` has none yet, and
    /// Prose's constructor answers `None` when it cannot seed a document.
    /// Unreachable for any fixture this spec produces, and present so the
    /// loader degrades cleanly.
    #[error("no engine for document type {doc_type:?}")]
    NoEngine { doc_type: DocType },

    /// The sidecar next to an artifact is missing or is not the JSON the
    /// manifest schema describes.
    #[error("unreadable fixture manifest `{}`: {reason}", path.display())]
    Manifest { path: PathBuf, reason: String },
}

// -----------------------------------------------------------------------------
// Decoding a state (the manifest's independent witness)
// -----------------------------------------------------------------------------

/// Decode a state into the coarse summary the manifest records.
///
/// On the loader side rather than on [`FixtureGen`]. The manifest-integrity
/// check then has something independent to compare against. A generator
/// declaring its own shape would assert only that it agrees with itself.
pub fn shape_of(state: &DocState) -> FixtureShape {
    match state {
        DocState::Trame(s) => s.fixture_shape(),
        DocState::Toile(s) => s.fixture_shape(),
        DocState::Prose(s) => s.fixture_shape(),
    }
}

/// The canonical semantic form of a document: every user-visible value, in a
/// deterministic order, as text.
///
/// **Not** the CRDT. Two documents agreeing on content still differ in op
/// history, peer ids and commit timestamps. No byte or state comparison can
/// express "same document by a different route". The frozen belt of
/// `FIXTURE-019` compares this text, so a failure prints a readable diff rather
/// than a length mismatch.
///
/// **Read-only in meaning, not in bytes.** Reaching a Loro container by name
/// materializes it. Dumping a document can therefore register containers a
/// sparse one never created, and change what its next `export_snapshot` emits.
/// Every engine reseed performs the same lazy materialization, which is why
/// `FIXTURE-013` states the round trip as a fixed point, not byte equality. It harms no consumer here: the drift gate diffs bytes read off
/// disk, never bytes re-exported from a dumped state. It stays a reason not to
/// dump a document and then persist it.
pub fn canonical_dump(state: &DocState) -> String {
    match state {
        DocState::Trame(s) => s.canonical_dump(),
        DocState::Toile(s) => s.canonical_dump(),
        DocState::Prose(s) => s.canonical_dump(),
    }
}

// -----------------------------------------------------------------------------
// Loading
// -----------------------------------------------------------------------------

/// The corpus root: `<workspace root>/fixtures`.
///
/// Resolved from `CARGO_MANIFEST_DIR` walked up to the workspace root, **never**
/// from the process working directory. `cargo test` runs a test binary with the
/// CWD at its own package root. A bare relative `fixtures/` would then resolve
/// differently for every consumer crate. The `env!` expands when *this* crate
/// compiles, so the anchor stays `ambre-logic`'s own directory whoever calls.
///
/// # Errors
///
/// [`FixtureError::Io`] when a manifest cannot be read or no ancestor holds a
/// workspace.
pub fn corpus_root() -> Result<PathBuf, FixtureError> {
    let start = Path::new(env!("CARGO_MANIFEST_DIR"));
    for dir in start.ancestors() {
        let manifest = dir.join("Cargo.toml");
        if !manifest.is_file() {
            continue;
        }
        // The workspace root is the first ancestor whose manifest declares one.
        // Reading the file rather than trusting the directory depth keeps this
        // correct if the crate ever moves under `crates/`.
        if fs::read_to_string(&manifest)?.contains("[workspace]") {
            return Ok(dir.join(CORPUS_DIR));
        }
    }
    Err(FixtureError::Io(std::io::Error::new(
        std::io::ErrorKind::NotFound,
        format!("no workspace root above {}", start.display()),
    )))
}

/// Load a committed fixture by name -- the entry point every consumer uses.
///
/// Resolves `<corpus root>/current/<name>.ambre` and hands the bytes to
/// [`import_fixture`], which is the production import path.
///
/// # Errors
///
/// As [`load_fixture_from`].
pub fn load_fixture(
    name: &Validated<FixtureName>,
) -> Result<(Id<Document>, DocState), FixtureError> {
    load_fixture_from(&corpus_root()?.join(CURRENT_DIR), name)
}

/// Load a scale fixture by name, out of the git-ignored `bench/` tree
/// (`FIXTURE-022`).
///
/// The benches' entry point, and the reason a bench never spells the directory:
/// a corpus that moved would then move in one place. The error a missing
/// artifact yields is [`FixtureError::NotFound`], which tells the author to run
/// `just fixtures-bench-corpus`.
///
/// # Errors
///
/// As [`load_fixture_from`].
pub fn load_bench_fixture(
    name: &Validated<FixtureName>,
) -> Result<(Id<Document>, DocState), FixtureError> {
    load_fixture_from(&corpus_root()?.join(BENCH_DIR), name)
}

/// [`load_fixture`] against an explicit corpus directory.
///
/// The parameterized form the belt drives with a temporary directory. `just
/// fixtures-check` uses it to load the tree it just regenerated instead of the
/// committed one. Same body, so testing this tests the shipping path.
///
/// # Errors
///
/// [`FixtureError::NotFound`] when no artifact has that name, and
/// [`FixtureError::Io`] when it cannot be read. Otherwise as [`import_fixture`].
pub fn load_fixture_from(
    dir: &Path,
    name: &Validated<FixtureName>,
) -> Result<(Id<Document>, DocState), FixtureError> {
    let path = artifact_path(dir, name);
    let bytes = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(FixtureError::NotFound { path });
        }
        Err(e) => return Err(FixtureError::Io(e)),
    };
    import_fixture(&ambre_core::Payload::<ambre_core::PlaintextContainer>::new(
        bytes,
    ))
}

/// Parse plaintext `.ambre` bytes into a live state through the production import
/// path (`FIXTURE-008`: the fixture loader **is** the production importer).
///
/// Two hops. First the header, read type-agnostically. The file names its own app
/// (`docfile` I2), so nothing here has to be told which importer to reach for.
/// That is what lets one `load_fixture` serve every corpus. Then the app's own
/// `import_ambre_plaintext`, which owns the payload half.
///
/// **The payload half is the app's to judge, not this function's.** That is why
/// the second hop is not `DocState::import_snapshot`. `import_snapshot` returns
/// display events, and the engines disagree about how a refused payload appears
/// in them: Trame returns none, Toile returns an error *event*. A loader reading
/// rejection off that vector would accept a corrupt Toile artifact and hand back
/// an empty board. Each engine's `import_ambre_plaintext` returns a typed
/// [`DocFileError::PayloadRejected`] instead, carried out verbatim.
///
/// # Errors
///
/// [`FixtureError::Container`] on bytes the container or the engine refuses,
/// and [`FixtureError::NoEngine`] for a type with no engine.
pub fn import_fixture(
    container: &ambre_core::Payload<ambre_core::PlaintextContainer>,
) -> Result<(Id<Document>, DocState), FixtureError> {
    let doc_type = read_ambre_plaintext(container)?.doc_type;
    // Pinned reader, not `DocState::new`: the receiving state commits its own
    // scaffolding before the merge, so the peer it picks lands in the imported
    // document. A random one would make a restored fixture differ run to run,
    // which `FIXTURE-019` compares. It would also leave the manifest's peer
    // count unable to tell the reader from an author.
    let mut state = match doc_type {
        DocType::Trame => DocState::Trame(Box::new(TrameState::with_peer(FIXTURE_IMPORT_PEER))),
        DocType::Toile => DocState::Toile(Box::new(ToileState::with_peer(FIXTURE_IMPORT_PEER))),
        DocType::Prose => match ProseState::with_peer(FIXTURE_IMPORT_PEER) {
            Some(prose) => DocState::Prose(Box::new(prose)),
            None => return Err(FixtureError::NoEngine { doc_type }),
        },
        DocType::Murmure => return Err(FixtureError::NoEngine { doc_type }),
    };
    // The same bytes, header included: each app re-reads the header through
    // `read_ambre_plaintext_as`, which is also where the dispatch rule fires.
    // That re-reads (and re-copies) the payload, which is the price of keeping
    // one import path instead of two; nothing here is on a hot path.
    let document_id = match &mut state {
        DocState::Trame(trame) => trame.import_ambre_plaintext(container).map(|(id, _)| id),
        DocState::Toile(toile) => toile.import_ambre_plaintext(container).map(|(id, _)| id),
        DocState::Prose(prose) => prose.import_ambre_plaintext(container).map(|(id, _)| id),
    }?;
    Ok((document_id, state))
}

/// The stem of an artifact: its path relative to its tree, extension
/// excluded. A [`FixtureName`] in `current/` and `bench/`, a [`FrozenEntry`]
/// in `frozen/`.
///
/// The one way a stem reaches a file path, so no unchecked text forms one.
pub trait Stem: Sized {
    /// The stem the artifact at `relative` carries. `relative` is the path
    /// below the tree, extension dropped.
    ///
    /// # Errors
    ///
    /// [`FixtureError::Stem`] when the rule of the stem refuses the path.
    fn read(relative: &Path) -> Result<Self, FixtureError>;

    /// The path of the artifact below its tree, extension excluded.
    fn relative(&self) -> PathBuf;
}

/// A live name is its own stem: `trame/grid_literals` is the subdirectory
/// `trame` and the file stem `grid_literals`.
impl Stem for Validated<FixtureName> {
    fn read(relative: &Path) -> Result<Self, FixtureError> {
        let refused = |reason: String| FixtureError::Stem {
            path: relative.to_path_buf(),
            reason,
        };
        let text = relative
            .to_str()
            .ok_or_else(|| refused("the path is not UTF-8".to_owned()))?;
        Self::parse(text.replace('\\', "/")).map_err(|e| refused(e.to_string()))
    }

    fn relative(&self) -> PathBuf {
        PathBuf::from(self.as_str())
    }
}

/// `<dir>/<stem>.ambre`. A stem may carry `/` (`"trame/formula_dag_deep"`).
/// That `/` is a subdirectory the driver creates, not an escape. The rule of
/// [`FixtureName`] admits no `..` and no leading `/`. No stem
/// holds a `.`, so the extension is the only one the file name carries.
pub fn artifact_path(dir: &Path, stem: &impl Stem) -> PathBuf {
    dir.join(stem.relative().with_extension(ARTIFACT_EXT))
}

/// `<dir>/<stem>.manifest` -- the JSON sidecar beside the artifact.
pub fn manifest_path(dir: &Path, stem: &impl Stem) -> PathBuf {
    dir.join(stem.relative().with_extension(MANIFEST_EXT))
}

/// Read a sidecar back off disk. Used by `fixtures-check` and by the frozen belt,
/// which both compare a committed manifest against a recomputed one.
///
/// # Errors
///
/// [`FixtureError::NotFound`] when there is no sidecar, and [`FixtureError::Io`]
/// when it cannot be read. [`FixtureError::Manifest`] when it does not parse,
/// or names a generator the rule refuses.
pub fn read_manifest(dir: &Path, stem: &impl Stem) -> Result<FixtureManifest, FixtureError> {
    let path = manifest_path(dir, stem);
    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(FixtureError::NotFound { path });
        }
        Err(e) => return Err(FixtureError::Io(e)),
    };
    serde_json::from_str(&text).map_err(|e| FixtureError::Manifest {
        path,
        reason: e.to_string(),
    })
}

// -----------------------------------------------------------------------------
// The driver
// -----------------------------------------------------------------------------

/// Assemble the manifest for freshly generated bytes.
///
/// **Seq is a round trip**, and that is the point rather than a cost.
/// [`FixtureGen::build`] returns bytes while [`shape_of`] needs a decoded state,
/// so the driver re-imports what was just generated. That proves the artifact
/// loads through the production importer before it is ever committed.
///
/// The driver, not the generator, produces this: a [`FixtureGen`] has no method
/// that could.
///
/// # Errors
///
/// As [`import_fixture`], and [`FixtureError::Container`] when the bytes hold
/// another type than the generator declares. [`FixtureError::Name`] when the
/// rule refuses the name of the generator.
pub fn manifest_for(
    generator: &dyn FixtureGen,
    container: &ambre_core::Payload<ambre_core::PlaintextContainer>,
) -> Result<FixtureManifest, FixtureError> {
    let name = generator.name()?;
    let (_document_id, state) = import_fixture(container)?;
    if state.doc_type() != generator.doc_type() {
        return Err(FixtureError::Container(DocFileError::WrongDocType {
            found: state.doc_type(),
        }));
    }
    Ok(FixtureManifest {
        generator: name,
        generator_version: generator.generator_version(),
        seed: generator.seed(),
        doc_type: generator.doc_type(),
        ambre_version: AMBRE_VERSION,
        // The register the document actually carries (`SCHEMA-004`), read back
        // from the generated state rather than assumed. A manifest that names a
        // version the artefact does not hold is worse than no manifest.
        schema_version: state.schema_version(),
        digest: digest_of(container),
        shape: shape_of(&state),
    })
}

/// BLAKE2b over the **whole `.ambre` file, header included**, so a flipped
/// `document_id` byte is caught by the write-once check.
///
/// The keyless digest is [`ambre_crypto::hash::file_digest`], never a link
/// of the snapshot chain.
pub fn digest_of(
    container: &ambre_core::Payload<ambre_core::PlaintextContainer>,
) -> ambre_core::FixedBytes<ambre_core::FileDigest> {
    ambre_crypto::hash::file_digest(container)
}

/// Run one generator and write its artifact plus sidecar under `dir`.
///
/// Returns the manifest it wrote, so a caller can diff it against a committed one
/// without re-reading the disk.
///
/// # Errors
///
/// As [`manifest_for`], and [`FixtureError::Io`] when a file cannot be written.
pub fn write_fixture(
    generator: &dyn FixtureGen,
    dir: &Path,
) -> Result<FixtureManifest, FixtureError> {
    let container = generator.build();
    let manifest = manifest_for(generator, &container)?;
    let bytes = container.as_bytes();
    let artifact = artifact_path(dir, &manifest.generator);
    if let Some(parent) = artifact.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&artifact, bytes)?;
    // Pretty-printed with a trailing newline. The sidecar exists to be *read* in
    // a diff, and a single-line JSON blob would be as opaque as the binary it
    // describes.
    let sidecar = manifest_path(dir, &manifest.generator);
    let mut json = serde_json::to_string_pretty(&manifest).map_err(|e| FixtureError::Manifest {
        path: sidecar.clone(),
        reason: e.to_string(),
    })?;
    json.push('\n');
    fs::write(sidecar, json)?;
    Ok(manifest)
}

/// Run a whole registry into `dir`, in registry order.
///
/// Takes the registry as a parameter rather than reading [`REGISTRY`]. The belt
/// drives this same body with a synthetic registry while the shipping one is
/// still empty. The walk is therefore proven before the first real generator
/// lands.
///
/// # Errors
///
/// The first error [`write_fixture`] returns.
pub fn generate_into(
    registry: &[&dyn FixtureGen],
    dir: &Path,
) -> Result<Vec<FixtureManifest>, FixtureError> {
    registry
        .iter()
        .map(|generator| write_fixture(*generator, dir))
        .collect()
}

/// Write every scale generator that has no artifact yet under `dir`
/// (`FIXTURE-022`), and report the names it built.
///
/// Skips a generator whose artifact and sidecar are both already there. The
/// corpus takes minutes to build and the benches that read it run far more
/// often, so a rerun costs nothing when the corpus stands. A generator whose
/// output changed on purpose is rebuilt by deleting its artifact, the same
/// gesture as for any cache.
///
/// No drift gate rides on this tree. The committed corpus has one
/// (`check_corpus`), and it exists because git keeps a committed artifact
/// forever; a generated-on-demand tree is rebuilt from the seed instead.
///
/// # Errors
///
/// The first error [`write_fixture`] returns.
pub fn generate_bench_into(
    registry: &[&dyn FixtureGen],
    dir: &Path,
) -> Result<Vec<Validated<FixtureName>>, FixtureError> {
    let mut built = Vec::new();
    for generator in registry {
        let name = generator.name()?;
        if artifact_path(dir, &name).is_file() && manifest_path(dir, &name).is_file() {
            continue;
        }
        write_fixture(*generator, dir)?;
        built.push(name);
    }
    Ok(built)
}

// -----------------------------------------------------------------------------
// The frozen tree
// -----------------------------------------------------------------------------

/// One artifact in `fixtures/frozen/`, as its filename describes it. Its stem
/// is `<name>@v<ambre>-s<schema>`, relative to `frozen/`: the filename minus
/// the extension, and what a report names. `Display` writes that stem.
///
/// Membership is positional: a frozen artifact is one that sits under
/// `frozen/`, and no flag in the manifest can contradict the path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FrozenEntry {
    generator: Validated<FixtureName>,
    ambre_version: Version<Container>,
    schema_version: Version<Schema>,
}

impl FrozenEntry {
    /// The entry of the artifact `generator` wrote at the two versions, that
    /// is, the outgoing ones.
    #[must_use]
    pub const fn new(
        generator: Validated<FixtureName>,
        ambre_version: Version<Container>,
        schema_version: Version<Schema>,
    ) -> Self {
        Self {
            generator,
            ambre_version,
            schema_version,
        }
    }

    /// The generator the artifact came from -- the part before the `@`.
    #[must_use]
    pub const fn generator(&self) -> &Validated<FixtureName> {
        &self.generator
    }

    /// The container version the artifact was written at.
    #[must_use]
    pub const fn ambre_version(&self) -> Version<Container> {
        self.ambre_version
    }

    /// The schema version the artifact was written at.
    #[must_use]
    pub const fn schema_version(&self) -> Version<Schema> {
        self.schema_version
    }

    /// The ordering key of `FIXTURE-021`: the version pair, compared as two
    /// numbers.
    ///
    /// **Numeric, not the lexicographic order of the filename.** The two agree
    /// only while every version stays one digit. `s10` sorts before `s9` as text
    /// and after it as a number. A corpus that survives ten schema bumps would
    /// start dropping its newest artifact instead of its oldest.
    fn age_key(&self) -> (Version<Container>, Version<Schema>) {
        (self.ambre_version, self.schema_version)
    }
}

/// The stem, `<name>@v<ambre>-s<schema>`.
impl std::fmt::Display for FrozenEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}@v{}-s{}",
            self.generator, self.ambre_version, self.schema_version
        )
    }
}

/// Read off the path rather than the manifest. The manifest of a frozen
/// artifact is a copy of the one written at freeze time. The path is the only
/// statement about the artifact that freezing itself produced.
impl Stem for FrozenEntry {
    fn read(relative: &Path) -> Result<Self, FixtureError> {
        let refused = |reason: &str| FixtureError::Stem {
            path: relative.to_path_buf(),
            reason: reason.to_owned(),
        };
        let file = relative
            .file_name()
            .and_then(|file| file.to_str())
            .ok_or_else(|| refused("no UTF-8 file name"))?;
        let (stem, versions) = file
            .rsplit_once('@')
            .ok_or_else(|| refused("no `@` before the version pair"))?;
        let (ambre, schema) = versions
            .strip_prefix('v')
            .and_then(|pair| pair.split_once("-s"))
            .ok_or_else(|| refused("no `v<ambre>-s<schema>` version pair"))?;
        let ambre = ambre.parse().map_err(|_| refused("no container version"))?;
        let schema = schema.parse().map_err(|_| refused("no schema version"))?;
        Ok(Self::new(
            Validated::<FixtureName>::read(&relative.with_file_name(stem))?,
            Version::<Container>::new(ambre),
            Version::<Schema>::new(schema),
        ))
    }

    fn relative(&self) -> PathBuf {
        let mut path = self.generator.relative().into_os_string();
        path.push(format!("@v{}-s{}", self.ambre_version, self.schema_version));
        PathBuf::from(path)
    }
}

/// Every artifact under `<root>/frozen/`, newest last.
///
/// # Errors
///
/// [`FixtureError::Io`] when the frozen tree cannot be listed, and
/// [`FixtureError::Stem`] when a file of it carries no frozen stem.
pub fn frozen_entries(root: &Path) -> Result<Vec<FrozenEntry>, FixtureError> {
    let dir = root.join(FROZEN_DIR);
    let mut entries: Vec<FrozenEntry> = artifact_stems(&dir)?;
    entries.sort_by(|a, b| {
        a.age_key()
            .cmp(&b.age_key())
            .then_with(|| a.generator.cmp(&b.generator))
    });
    Ok(entries)
}

/// How far back the frozen tree exercises migration (`FIXTURE-021`).
///
/// The gate loads and migrates every frozen artifact, so an unbounded set walks
/// into the `fixtures-check` budget one version bump at a time.
pub const FROZEN_KEEP_PER_TYPE: Count<FrozenArtifacts> = Count::new(3);

/// The artifacts of `frozen/` one document type holds.
pub enum FrozenArtifacts {}

impl NarrowCount for FrozenArtifacts {
    const NAME: ambre_core::StaticKey<ambre_core::DebugNames> =
        ambre_core::StaticKey::new("FrozenArtifacts");
}

/// Copy one named artifact out of `current/` into `frozen/`, and report where it
/// landed.
///
/// The version suffix comes from the artifact's own manifest rather than from a
/// person typing it, so the path cannot disagree with the bytes. The call refuses
/// an existing target rather than overwriting it: a frozen artifact takes its
/// bytes once (`FIXTURE-018`).
///
/// # Errors
///
/// As [`read_manifest`], [`FixtureError::Manifest`] when the target is already
/// frozen, and [`FixtureError::Io`] when a copy fails.
pub fn freeze(root: &Path, name: &Validated<FixtureName>) -> Result<PathBuf, FixtureError> {
    let current = root.join(CURRENT_DIR);
    let manifest = read_manifest(&current, name)?;
    let stem = FrozenEntry::new(
        name.clone(),
        manifest.ambre_version,
        manifest.schema_version,
    );
    let frozen = root.join(FROZEN_DIR);
    let target = artifact_path(&frozen, &stem);
    if target.exists() {
        return Err(FixtureError::Manifest {
            path: target,
            reason: "already frozen at this version pair; a frozen artifact is written once"
                .to_owned(),
        });
    }
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::copy(artifact_path(&current, name), &target)?;
    fs::copy(manifest_path(&current, name), manifest_path(&frozen, &stem))?;
    Ok(target)
}

/// Drop the frozen artifacts the retention rule no longer keeps, and report
/// them.
///
/// Two causes, one pass (`FIXTURE-021`, `FIXTURE-026`). One is an artifact past
/// the cap for its document type. The other is an artifact whose generator is
/// gone, which has lost the second term of its comparison for good.
///
/// # Errors
///
/// [`FixtureError::Io`] when the tree cannot be listed or a file cannot be
/// removed.
pub fn retain_frozen(
    registry: &[&dyn FixtureGen],
    root: &Path,
) -> Result<Vec<PathBuf>, FixtureError> {
    let frozen = root.join(FROZEN_DIR);
    let named = named(registry)?;
    let entries = frozen_entries(root)?;
    let mut kept_per_type: HashMap<DocType, Count<FrozenArtifacts>> = HashMap::new();
    let mut dropped = Vec::new();
    // Newest first, so the cap keeps the head of each document type's run.
    for entry in entries.iter().rev() {
        let live = named.iter().find(|g| g.name == entry.generator);
        let doc_type = match read_manifest(&frozen, entry) {
            Ok(manifest) => manifest.doc_type,
            // An unreadable sidecar leaves the artifact unattributable, and the
            // gate reports it. Retention keeps it rather than deleting what it
            // cannot describe.
            Err(_) => continue,
        };
        let kept = kept_per_type.entry(doc_type).or_insert(Count::ZERO);
        if live.is_none() || *kept >= FROZEN_KEEP_PER_TYPE {
            dropped.push(artifact_path(&frozen, entry));
            fs::remove_file(artifact_path(&frozen, entry))?;
            fs::remove_file(manifest_path(&frozen, entry))?;
            continue;
        }
        *kept = kept.incremented();
    }
    Ok(dropped)
}

// -----------------------------------------------------------------------------
// The gate
// -----------------------------------------------------------------------------

/// One way the committed corpus and the live registry disagree.
///
/// Every variant fails `just fixtures-check`. A drift is a fact about the tree,
/// never a diagnosis. The recipe prints them and the contributor reads the
/// manifest diff, which is what the sidecar exists for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Drift {
    /// A generator with no committed artifact (`FIXTURE-031`). Silent on the run
    /// that introduces it unless the gate says so: a new generator is a diff
    /// with nothing to diff.
    Missing { name: Validated<FixtureName> },

    /// A committed artifact whose generator is gone (`FIXTURE-025`). A byte diff
    /// reports nothing here, because the term it would compare against is the
    /// one that vanished.
    Orphan { name: Validated<FixtureName> },

    /// A committed artifact whose bytes no longer match a regeneration
    /// (`FIXTURE-006`).
    Bytes { name: Validated<FixtureName> },

    /// A committed sidecar that is unreadable, or that describes a different
    /// document from the one a regeneration produces.
    Manifest {
        name: Validated<FixtureName>,
        reason: String,
    },

    /// A frozen artifact that fails one of the three semantic checks
    /// (`FIXTURE-019`). Those are its digest against its bytes and its load
    /// through the migration chain. The third is its migrated canonical dump
    /// against the live artifact its manifest names.
    Frozen { stem: FrozenEntry, reason: String },

    /// More frozen artifacts for one document type than retention keeps
    /// (`FIXTURE-021`). The remedy is `just fixtures`, which runs the pass.
    FrozenOverflow {
        doc_type: DocType,
        kept: Count<FrozenArtifacts>,
    },

    /// A superseded container version with no frozen artifact for a document
    /// type the registry generates (`DOCFILE-018`). A bump that skipped its
    /// freeze reads this way, and so does the drop of the last artifact a
    /// version kept.
    Unfrozen {
        doc_type: DocType,
        ambre_version: Version<Container>,
    },
}

impl std::fmt::Display for Drift {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Missing { name } => {
                write!(f, "{name}: generator has no committed artifact")
            }
            Self::Orphan { name } => write!(f, "{name}: committed artifact has no generator"),
            Self::Bytes { name } => write!(f, "{name}: committed bytes differ from a regeneration"),
            Self::Manifest { name, reason } => write!(f, "{name}: manifest {reason}"),
            Self::Frozen { stem, reason } => write!(f, "frozen/{stem}: {reason}"),
            Self::FrozenOverflow { doc_type, kept } => write!(
                f,
                "frozen/: {kept} artifacts for {doc_type:?}, retention keeps {FROZEN_KEEP_PER_TYPE}"
            ),
            Self::Unfrozen {
                doc_type,
                ambre_version,
            } => write!(
                f,
                "frozen/: no {doc_type:?} artifact at superseded container version \
                 {ambre_version}; no writer can produce one now, so restore it from history"
            ),
        }
    }
}

/// A frozen artifact whose semantic comparison is retired rather than failed.
///
/// Two causes (`FIXTURE-020`, `FIXTURE-026`): the live generator declares a
/// different `generator_version`, or it is gone. Nothing can reconstruct what
/// the old generator would have produced at the new schema. The artifact
/// reports as degraded, and the next freeze restores the coverage.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Degraded {
    pub stem: FrozenEntry,
    pub reason: String,
}

/// What one gate run found.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CorpusReport {
    pub drifts: Vec<Drift>,
    pub degraded: Vec<Degraded>,
    /// Frozen artifacts whose dump still matches the live one. Reported so a
    /// corpus that degrades to zero held comparisons is visible before the next
    /// version bump rather than after it.
    pub frozen_held: Count<FrozenArtifacts>,
}

impl CorpusReport {
    pub fn is_clean(&self) -> bool {
        self.drifts.is_empty()
    }
}

/// Compare the committed corpus under `root` against `registry`.
///
/// `scratch` is a directory the caller owns and the gate regenerates into. The
/// committed tree is never written, so a red gate leaves the working copy
/// exactly as it found it.
///
/// # Errors
///
/// An error that stops the comparison itself, from [`write_fixture`] or from
/// reading the tree. A disagreement is a [`Drift`] in the report instead.
pub fn check_corpus(
    registry: &[&dyn FixtureGen],
    root: &Path,
    scratch: &Path,
) -> Result<CorpusReport, FixtureError> {
    let current = root.join(CURRENT_DIR);
    let named = named(registry)?;
    let mut report = CorpusReport::default();

    for generator in &named {
        let name = &generator.name;
        let fresh = write_fixture(generator.generator, scratch)?;
        let committed = match fs::read(artifact_path(&current, name)) {
            Ok(bytes) => ambre_core::Payload::<ambre_core::PlaintextContainer>::new(bytes),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                report.drifts.push(Drift::Missing { name: name.clone() });
                continue;
            }
            Err(e) => return Err(FixtureError::Io(e)),
        };
        if digest_of(&committed) != fresh.digest {
            report.drifts.push(Drift::Bytes { name: name.clone() });
        }
        match read_manifest(&current, name) {
            Ok(manifest) if manifest == fresh => {}
            Ok(_) => report.drifts.push(Drift::Manifest {
                name: name.clone(),
                reason: "describes a different document from a regeneration".to_owned(),
            }),
            Err(e) => report.drifts.push(Drift::Manifest {
                name: name.clone(),
                reason: e.to_string(),
            }),
        }
    }

    for name in artifact_stems::<Validated<FixtureName>>(&current)? {
        if named.iter().any(|g| g.name == name) {
            continue;
        }
        report.drifts.push(Drift::Orphan { name });
    }

    check_frozen(&named, root, &mut report)?;
    report
        .drifts
        .extend(unfrozen_versions(registry, root, AMBRE_VERSION)?);
    Ok(report)
}

/// Every superseded container version below `current` that owes a frozen
/// artifact for a document type of `registry`, and holds none (`DOCFILE-018`).
///
/// Container versions count from 1, so each one below `current` is superseded.
/// A version is owed only while retention could still keep its artifact. Once
/// a document type holds [`FROZEN_KEEP_PER_TYPE`] newer ones, the cap of
/// `FIXTURE-021` has dropped it by design.
///
/// # Errors
///
/// [`FixtureError::Io`] when the frozen tree cannot be listed.
pub fn unfrozen_versions(
    registry: &[&dyn FixtureGen],
    root: &Path,
    current: Version<Container>,
) -> Result<Vec<Drift>, FixtureError> {
    let frozen = root.join(FROZEN_DIR);
    // An unreadable sidecar is reported by `check_frozen`; it attributes
    // nothing here.
    let typed: Vec<(DocType, Version<Container>)> = frozen_entries(root)?
        .into_iter()
        .filter_map(|entry| {
            let manifest = read_manifest(&frozen, &entry).ok()?;
            Some((manifest.doc_type, entry.ambre_version))
        })
        .collect();
    let mut doc_types: Vec<DocType> = Vec::new();
    for generator in registry {
        if !doc_types.contains(&generator.doc_type()) {
            doc_types.push(generator.doc_type());
        }
    }
    let mut owed = Vec::new();
    for doc_type in doc_types {
        let of_type = || typed.iter().filter(move |(t, _)| *t == doc_type);
        for ambre_version in (1..current.get()).map(Version::<Container>::new) {
            let newer = Count::of(of_type().filter(|(_, v)| *v > ambre_version).count());
            let held = of_type().any(|(_, v)| *v == ambre_version);
            if newer < FROZEN_KEEP_PER_TYPE && !held {
                owed.push(Drift::Unfrozen {
                    doc_type,
                    ambre_version,
                });
            }
        }
    }
    Ok(owed)
}

/// The frozen half of [`check_corpus`], in strengthening order: the digest, the
/// load through the migration chain, then the canonical dump.
fn check_frozen(
    named: &[Named<'_>],
    root: &Path,
    report: &mut CorpusReport,
) -> Result<(), FixtureError> {
    let frozen = root.join(FROZEN_DIR);
    let current = root.join(CURRENT_DIR);
    let mut per_type: Vec<(DocType, Count<FrozenArtifacts>)> = Vec::new();

    for entry in frozen_entries(root)? {
        let manifest = match read_manifest(&frozen, &entry) {
            Ok(manifest) => manifest,
            Err(e) => {
                report.drifts.push(Drift::Frozen {
                    stem: entry.clone(),
                    reason: e.to_string(),
                });
                continue;
            }
        };
        match per_type.iter_mut().find(|(t, _)| *t == manifest.doc_type) {
            Some(slot) => slot.1 = slot.1.incremented(),
            None => per_type.push((manifest.doc_type, Count::new(1))),
        }
        let bytes = ambre_core::Payload::<ambre_core::PlaintextContainer>::new(fs::read(
            artifact_path(&frozen, &entry),
        )?);
        if digest_of(&bytes) != manifest.digest {
            report.drifts.push(Drift::Frozen {
                stem: entry.clone(),
                reason: "bytes no longer match the digest its manifest carries".to_owned(),
            });
            continue;
        }
        // The load is the migration: `read_ambre_plaintext` walks the container
        // chain before the application sees the payload.
        let migrated = match import_fixture(&bytes) {
            Ok((_id, state)) => state,
            Err(e) => {
                report.drifts.push(Drift::Frozen {
                    stem: entry.clone(),
                    reason: format!("does not load through the migration chain: {e}"),
                });
                continue;
            }
        };
        let Some(live) = named.iter().find(|g| g.name == manifest.generator) else {
            report.degraded.push(Degraded {
                stem: entry.clone(),
                reason: format!("generator `{}` no longer exists", manifest.generator),
            });
            continue;
        };
        let live_version = live.generator.generator_version();
        if live_version != manifest.generator_version {
            report.degraded.push(Degraded {
                stem: entry.clone(),
                reason: format!(
                    "generator is at version {live_version}, the artifact at {}",
                    manifest.generator_version
                ),
            });
            continue;
        }
        let (_id, live_state) = match load_fixture_from(&current, &live.name) {
            Ok(loaded) => loaded,
            Err(e) => {
                report.drifts.push(Drift::Frozen {
                    stem: entry.clone(),
                    reason: format!("the live artifact it names does not load: {e}"),
                });
                continue;
            }
        };
        if canonical_dump(&migrated) == canonical_dump(&live_state) {
            report.frozen_held = report.frozen_held.incremented();
        } else {
            report.drifts.push(Drift::Frozen {
                stem: entry,
                reason: "migrated dump differs from the live artifact it names".to_owned(),
            });
        }
    }

    report.drifts.extend(
        per_type
            .into_iter()
            .filter(|(_, kept)| *kept > FROZEN_KEEP_PER_TYPE)
            .map(|(doc_type, kept)| Drift::FrozenOverflow { doc_type, kept }),
    );
    Ok(())
}

/// Delete every artifact under `dir` that no generator in `registry` produces,
/// and report what went.
///
/// Detecting an orphan is the gate's job; removing one is this function's. The
/// fix for a red gate is therefore `just fixtures`, never a manual delete.
///
/// # Errors
///
/// [`FixtureError::Io`] when the tree cannot be listed or an artifact cannot be
/// removed. [`FixtureError::Name`] when the rule refuses the name of a
/// generator, and [`FixtureError::Stem`] when a file carries no name.
pub fn prune_orphans(
    registry: &[&dyn FixtureGen],
    dir: &Path,
) -> Result<Vec<PathBuf>, FixtureError> {
    let named = named(registry)?;
    let mut removed = Vec::new();
    for name in artifact_stems::<Validated<FixtureName>>(dir)? {
        if named.iter().any(|g| g.name == name) {
            continue;
        }
        fs::remove_file(artifact_path(dir, &name))?;
        // A sidecar may already be gone; its absence is not a second failure.
        let _ = fs::remove_file(manifest_path(dir, &name));
        removed.push(artifact_path(dir, &name));
    }
    Ok(removed)
}

/// Every artifact stem under `dir`, relative to it and sorted by path.
///
/// A stem carries its subdirectory (`"trame/grid_literals"`), because that is
/// what a generator name is. A missing directory is an empty corpus rather than
/// an error: the first run of `just fixtures` creates the tree. A file whose
/// stem the rule of `S` refuses fails the walk: no generator wrote it, and
/// its name forms no checked path.
fn artifact_stems<S: Stem>(dir: &Path) -> Result<Vec<S>, FixtureError> {
    let mut relatives = Vec::new();
    collect_stems(dir, dir, &mut relatives)?;
    relatives.sort();
    relatives.iter().map(|relative| S::read(relative)).collect()
}

/// Walk `dir` for artifacts, pushing each path below `root`, extension
/// dropped.
fn collect_stems(root: &Path, dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), FixtureError> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(FixtureError::Io(e)),
    };
    for entry in entries {
        let path = entry?.path();
        if path.is_dir() {
            collect_stems(root, &path, out)?;
            continue;
        }
        if path.extension().and_then(|e| e.to_str()) != Some(ARTIFACT_EXT) {
            continue;
        }
        // The stem reads the path through its own rule, so a non-UTF-8 name
        // fails the walk rather than pass in silence.
        if let Ok(relative) = path.with_extension("").strip_prefix(root) {
            out.push(relative.to_path_buf());
        }
    }
    Ok(())
}
