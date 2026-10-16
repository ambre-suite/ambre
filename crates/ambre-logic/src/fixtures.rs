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

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use ambre_core::docfile::{AMBRE_VERSION, DocFileError, read_ambre_plaintext};
use ambre_core::{
    DocType, DocumentId, FIXTURE_IMPORT_PEER, FixtureGen, FixtureManifest, FixtureShape,
};

use ambre_prose::ProseState;
use ambre_prose::logic::fixtures::PROSE_FIXTURES;
use ambre_toile::logic::state::ToileState;
use ambre_toile::logic::state::fixtures::{TOILE_BENCH_FIXTURES, TOILE_FIXTURES};
use ambre_trame::logic::TrameState;
use ambre_trame::logic::fixtures::{TRAME_BENCH_FIXTURES, TRAME_FIXTURES};

use crate::DocState;

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
    /// figures describe.
    pub commit: String,

    /// One entry per benchmark: its `criterion` id, and the mean in nanoseconds.
    /// Nanoseconds because that is what `criterion` reports, and a converted
    /// figure is one more thing to get wrong.
    pub measurements: Vec<BenchMeasurement>,
}

/// One benchmark's mean, as `criterion` estimated it.
#[derive(serde::Serialize, serde::Deserialize, PartialEq, Eq, Clone, Debug)]
pub struct BenchMeasurement {
    pub id: String,
    pub mean_ns: u64,
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
            mean_ns: mean.max(0.0) as u64,
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
                name: path.to_string_lossy().into_owned(),
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
/// a synthetic registry, the pattern `ambre_core::docfile`'s migration walk
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
    #[error("no fixture named `{name}` in the corpus")]
    NotFound { name: String },

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
    #[error("unreadable fixture manifest for `{name}`: {reason}")]
    Manifest { name: String, reason: String },
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
pub fn load_fixture(name: &str) -> Result<(DocumentId, DocState), FixtureError> {
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
pub fn load_bench_fixture(name: &str) -> Result<(DocumentId, DocState), FixtureError> {
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
pub fn load_fixture_from(dir: &Path, name: &str) -> Result<(DocumentId, DocState), FixtureError> {
    let path = artifact_path(dir, name);
    let bytes = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(FixtureError::NotFound {
                name: name.to_owned(),
            });
        }
        Err(e) => return Err(FixtureError::Io(e)),
    };
    import_fixture(&bytes)
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
pub fn import_fixture(bytes: &[u8]) -> Result<(DocumentId, DocState), FixtureError> {
    let doc_type = read_ambre_plaintext(bytes)?.doc_type;
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
        DocState::Trame(trame) => trame.import_ambre_plaintext(bytes).map(|(id, _events)| id),
        DocState::Toile(toile) => toile.import_ambre_plaintext(bytes).map(|(id, _events)| id),
        DocState::Prose(prose) => prose.import_ambre_plaintext(bytes).map(|(id, _events)| id),
    }?;
    Ok((DocumentId::from_uuid(document_id), state))
}

/// `<dir>/<name>.ambre`. `name` may carry `/` (`"trame/formula_dag_deep"`), which
/// is a subdirectory, not an escape: the driver creates it.
pub fn artifact_path(dir: &Path, name: &str) -> PathBuf {
    dir.join(format!("{name}.{ARTIFACT_EXT}"))
}

/// `<dir>/<name>.manifest` -- the JSON sidecar beside the artifact.
pub fn manifest_path(dir: &Path, name: &str) -> PathBuf {
    dir.join(format!("{name}.{MANIFEST_EXT}"))
}

/// Read a sidecar back off disk. Used by `fixtures-check` and by the frozen belt,
/// which both compare a committed manifest against a recomputed one.
///
/// # Errors
///
/// [`FixtureError::NotFound`] when there is no sidecar, [`FixtureError::Io`]
/// when it cannot be read, and [`FixtureError::Manifest`] when it does not parse.
pub fn read_manifest(dir: &Path, name: &str) -> Result<FixtureManifest, FixtureError> {
    let path = manifest_path(dir, name);
    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(FixtureError::NotFound {
                name: name.to_owned(),
            });
        }
        Err(e) => return Err(FixtureError::Io(e)),
    };
    serde_json::from_str(&text).map_err(|e| FixtureError::Manifest {
        name: name.to_owned(),
        reason: e.to_string(),
    })
}

// -----------------------------------------------------------------------------
// The driver
// -----------------------------------------------------------------------------

/// Assemble the manifest for freshly generated bytes.
///
/// **Generation is a round trip**, and that is the point rather than a cost.
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
/// another type than the generator declares.
pub fn manifest_for(
    generator: &dyn FixtureGen,
    bytes: &[u8],
) -> Result<FixtureManifest, FixtureError> {
    let (_document_id, state) = import_fixture(bytes)?;
    if state.doc_type() != generator.doc_type() {
        return Err(FixtureError::Container(DocFileError::WrongDocType {
            found: state.doc_type(),
        }));
    }
    Ok(FixtureManifest {
        generator: generator.name().to_owned(),
        generator_version: generator.generator_version(),
        seed: generator.seed(),
        doc_type: generator.doc_type(),
        ambre_version: AMBRE_VERSION,
        // The register the document actually carries (`SCHEMA-004`), read back
        // from the generated state rather than assumed. A manifest that names a
        // version the artefact does not hold is worse than no manifest.
        schema_version: state.schema_version(),
        digest: digest_of(bytes),
        shape: shape_of(&state),
    })
}

/// BLAKE2b over the **whole `.ambre` file, header included**, so a flipped
/// `document_id` byte is caught by the write-once check.
///
/// `ambre-crypto` has no plain `hash()`; `chain_hash` with an empty parent proof
/// degenerates to a keyless 32-byte digest, which is exactly what this needs.
pub fn digest_of(bytes: &[u8]) -> [u8; 32] {
    ambre_crypto::hash::chain_hash(&[], bytes)
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
    let bytes = generator.build();
    let manifest = manifest_for(generator, &bytes)?;
    let artifact = artifact_path(dir, generator.name());
    if let Some(parent) = artifact.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&artifact, &bytes)?;
    // Pretty-printed with a trailing newline. The sidecar exists to be *read* in
    // a diff, and a single-line JSON blob would be as opaque as the binary it
    // describes.
    let mut json = serde_json::to_string_pretty(&manifest).map_err(|e| FixtureError::Manifest {
        name: generator.name().to_owned(),
        reason: e.to_string(),
    })?;
    json.push('\n');
    fs::write(manifest_path(dir, generator.name()), json)?;
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
) -> Result<Vec<&'static str>, FixtureError> {
    let mut built = Vec::new();
    for generator in registry {
        let name = generator.name();
        if artifact_path(dir, name).is_file() && manifest_path(dir, name).is_file() {
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

/// One artifact in `fixtures/frozen/`, as its filename describes it.
///
/// Membership is positional: a frozen artifact is one that sits under
/// `frozen/`, and no flag in the manifest can contradict the path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FrozenEntry {
    /// The stem, `<name>@v<ambre>-s<schema>`, relative to `frozen/`. The
    /// filename minus the extension, and what a report names.
    pub stem: String,
    /// The generator the artifact came from -- the part before the `@`.
    pub generator: String,
    /// The two versions the artifact was written at, that is, the outgoing ones.
    pub ambre_version: u8,
    pub schema_version: u32,
}

impl FrozenEntry {
    /// Read one entry off its stem, or `None` when the stem carries no version
    /// pair.
    ///
    /// Parsed rather than read from the manifest. The manifest of a frozen
    /// artifact is a copy of the one written at freeze time. The path is the
    /// only statement about the artifact that freezing itself produced.
    fn parse(stem: &str) -> Option<Self> {
        let (generator, versions) = stem.rsplit_once('@')?;
        let (ambre, schema) = versions.strip_prefix('v')?.split_once("-s")?;
        Some(Self {
            stem: stem.to_owned(),
            generator: generator.to_owned(),
            ambre_version: ambre.parse().ok()?,
            schema_version: schema.parse().ok()?,
        })
    }

    /// The ordering key of `FIXTURE-021`: the version pair, compared as two
    /// numbers.
    ///
    /// **Numeric, not the lexicographic order of the filename.** The two agree
    /// only while every version stays one digit. `s10` sorts before `s9` as text
    /// and after it as a number. A corpus that survives ten schema bumps would
    /// start dropping its newest artifact instead of its oldest.
    fn age_key(&self) -> (u8, u32) {
        (self.ambre_version, self.schema_version)
    }
}

/// Every artifact under `<root>/frozen/`, newest last.
///
/// # Errors
///
/// [`FixtureError::Io`] when the frozen tree cannot be listed.
pub fn frozen_entries(root: &Path) -> Result<Vec<FrozenEntry>, FixtureError> {
    let dir = root.join(FROZEN_DIR);
    let mut entries: Vec<FrozenEntry> = artifact_stems(&dir)?
        .iter()
        .filter_map(|stem| FrozenEntry::parse(stem))
        .collect();
    entries.sort_by(|a, b| a.age_key().cmp(&b.age_key()).then(a.stem.cmp(&b.stem)));
    Ok(entries)
}

/// How far back the frozen tree exercises migration (`FIXTURE-021`).
///
/// The gate loads and migrates every frozen artifact, so an unbounded set walks
/// into the `fixtures-check` budget one version bump at a time.
pub const FROZEN_KEEP_PER_TYPE: usize = 3;

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
pub fn freeze(root: &Path, name: &str) -> Result<PathBuf, FixtureError> {
    let current = root.join(CURRENT_DIR);
    let manifest = read_manifest(&current, name)?;
    let stem = format!(
        "{name}@v{}-s{}",
        manifest.ambre_version, manifest.schema_version
    );
    let frozen = root.join(FROZEN_DIR);
    let target = artifact_path(&frozen, &stem);
    if target.exists() {
        return Err(FixtureError::Manifest {
            name: stem,
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
    let entries = frozen_entries(root)?;
    let mut kept_per_type: HashMap<DocType, usize> = HashMap::new();
    let mut dropped = Vec::new();
    // Newest first, so the cap keeps the head of each document type's run.
    for entry in entries.iter().rev() {
        let live = registry.iter().find(|g| g.name() == entry.generator);
        let doc_type = match read_manifest(&frozen, &entry.stem) {
            Ok(manifest) => manifest.doc_type,
            // An unreadable sidecar leaves the artifact unattributable, and the
            // gate reports it. Retention keeps it rather than deleting what it
            // cannot describe.
            Err(_) => continue,
        };
        let kept = kept_per_type.entry(doc_type).or_insert(0);
        if live.is_none() || *kept >= FROZEN_KEEP_PER_TYPE {
            dropped.push(artifact_path(&frozen, &entry.stem));
            fs::remove_file(artifact_path(&frozen, &entry.stem))?;
            fs::remove_file(manifest_path(&frozen, &entry.stem))?;
            continue;
        }
        *kept += 1;
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
    Missing { name: String },

    /// A committed artifact whose generator is gone (`FIXTURE-025`). A byte diff
    /// reports nothing here, because the term it would compare against is the
    /// one that vanished.
    Orphan { name: String },

    /// A committed artifact whose bytes no longer match a regeneration
    /// (`FIXTURE-006`).
    Bytes { name: String },

    /// A committed sidecar that is unreadable, or that describes a different
    /// document from the one a regeneration produces.
    Manifest { name: String, reason: String },

    /// A frozen artifact that fails one of the three semantic checks
    /// (`FIXTURE-019`). Those are its digest against its bytes and its load
    /// through the migration chain. The third is its migrated canonical dump
    /// against the live artifact its manifest names.
    Frozen { stem: String, reason: String },

    /// More frozen artifacts for one document type than retention keeps
    /// (`FIXTURE-021`). The remedy is `just fixtures`, which runs the pass.
    FrozenOverflow { doc_type: DocType, kept: usize },

    /// A superseded container version with no frozen artifact for a document
    /// type the registry generates (`DOCFILE-018`). A bump that skipped its
    /// freeze reads this way, and so does the drop of the last artifact a
    /// version kept.
    Unfrozen {
        doc_type: DocType,
        ambre_version: u8,
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
    pub stem: String,
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
    pub frozen_held: usize,
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
    let mut report = CorpusReport::default();

    for generator in registry {
        let name = generator.name();
        let fresh = write_fixture(*generator, scratch)?;
        let committed = match fs::read(artifact_path(&current, name)) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                report.drifts.push(Drift::Missing {
                    name: name.to_owned(),
                });
                continue;
            }
            Err(e) => return Err(FixtureError::Io(e)),
        };
        if digest_of(&committed) != fresh.digest {
            report.drifts.push(Drift::Bytes {
                name: name.to_owned(),
            });
        }
        match read_manifest(&current, name) {
            Ok(manifest) if manifest == fresh => {}
            Ok(_) => report.drifts.push(Drift::Manifest {
                name: name.to_owned(),
                reason: "describes a different document from a regeneration".to_owned(),
            }),
            Err(e) => report.drifts.push(Drift::Manifest {
                name: name.to_owned(),
                reason: e.to_string(),
            }),
        }
    }

    for name in artifact_stems(&current)? {
        if registry.iter().any(|g| g.name() == name) {
            continue;
        }
        report.drifts.push(Drift::Orphan { name });
    }

    check_frozen(registry, root, &mut report)?;
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
    current: u8,
) -> Result<Vec<Drift>, FixtureError> {
    let frozen = root.join(FROZEN_DIR);
    // An unreadable sidecar is reported by `check_frozen`; it attributes
    // nothing here.
    let typed: Vec<(DocType, u8)> = frozen_entries(root)?
        .into_iter()
        .filter_map(|entry| {
            let manifest = read_manifest(&frozen, &entry.stem).ok()?;
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
        for ambre_version in 1..current {
            let newer = of_type().filter(|(_, v)| *v > ambre_version).count();
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
    registry: &[&dyn FixtureGen],
    root: &Path,
    report: &mut CorpusReport,
) -> Result<(), FixtureError> {
    let frozen = root.join(FROZEN_DIR);
    let current = root.join(CURRENT_DIR);
    let mut per_type: Vec<(DocType, usize)> = Vec::new();

    for entry in frozen_entries(root)? {
        let manifest = match read_manifest(&frozen, &entry.stem) {
            Ok(manifest) => manifest,
            Err(e) => {
                report.drifts.push(Drift::Frozen {
                    stem: entry.stem.clone(),
                    reason: e.to_string(),
                });
                continue;
            }
        };
        match per_type.iter_mut().find(|(t, _)| *t == manifest.doc_type) {
            Some(slot) => slot.1 += 1,
            None => per_type.push((manifest.doc_type, 1)),
        }
        let bytes = fs::read(artifact_path(&frozen, &entry.stem))?;
        if digest_of(&bytes) != manifest.digest {
            report.drifts.push(Drift::Frozen {
                stem: entry.stem.clone(),
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
                    stem: entry.stem.clone(),
                    reason: format!("does not load through the migration chain: {e}"),
                });
                continue;
            }
        };
        let Some(live) = registry.iter().find(|g| g.name() == manifest.generator) else {
            report.degraded.push(Degraded {
                stem: entry.stem.clone(),
                reason: format!("generator `{}` no longer exists", manifest.generator),
            });
            continue;
        };
        if live.generator_version() != manifest.generator_version {
            report.degraded.push(Degraded {
                stem: entry.stem.clone(),
                reason: format!(
                    "generator is at version {}, the artifact at {}",
                    live.generator_version(),
                    manifest.generator_version
                ),
            });
            continue;
        }
        let (_id, live_state) = match load_fixture_from(&current, live.name()) {
            Ok(loaded) => loaded,
            Err(e) => {
                report.drifts.push(Drift::Frozen {
                    stem: entry.stem.clone(),
                    reason: format!("the live artifact it names does not load: {e}"),
                });
                continue;
            }
        };
        if canonical_dump(&migrated) == canonical_dump(&live_state) {
            report.frozen_held += 1;
        } else {
            report.drifts.push(Drift::Frozen {
                stem: entry.stem.clone(),
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
/// removed.
pub fn prune_orphans(
    registry: &[&dyn FixtureGen],
    dir: &Path,
) -> Result<Vec<PathBuf>, FixtureError> {
    let mut removed = Vec::new();
    for name in artifact_stems(dir)? {
        if registry.iter().any(|g| g.name() == name) {
            continue;
        }
        fs::remove_file(artifact_path(dir, &name))?;
        // A sidecar may already be gone; its absence is not a second failure.
        let _ = fs::remove_file(manifest_path(dir, &name));
        removed.push(artifact_path(dir, &name));
    }
    Ok(removed)
}

/// Every artifact stem under `dir`, relative to it and sorted.
///
/// A stem carries its subdirectory (`"trame/grid_literals"`), because that is
/// what a generator name is. A missing directory is an empty corpus rather than
/// an error: the first run of `just fixtures` creates the tree.
fn artifact_stems(dir: &Path) -> Result<Vec<String>, FixtureError> {
    let mut out = Vec::new();
    collect_stems(dir, dir, &mut out)?;
    out.sort();
    Ok(out)
}

fn collect_stems(root: &Path, dir: &Path, out: &mut Vec<String>) -> Result<(), FixtureError> {
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
        // Lossy is unreachable for a tree this code wrote. The gate would flag
        // a non-UTF-8 name that reached it as an orphan, not skip it in
        // silence.
        if let Ok(relative) = path.with_extension("").strip_prefix(root) {
            out.push(relative.to_string_lossy().replace('\\', "/"));
        }
    }
    Ok(())
}
