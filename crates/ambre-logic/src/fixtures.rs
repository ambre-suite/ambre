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

use ambre_toile::logic::state::ToileState;
use ambre_toile::logic::state::fixtures::TOILE_FIXTURES;
use ambre_trame::logic::TrameState;
use ambre_trame::logic::fixtures::TRAME_FIXTURES;

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

// -----------------------------------------------------------------------------
// The generator registry
// -----------------------------------------------------------------------------

/// Every generator this build knows, in the order the driver walks them.
///
/// The two per-engine registries, concatenated rather than spelled out here.
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

/// Both per-engine registries, concatenated at compile time.
///
/// A `const fn` rather than a hand-written list, for the reason [`REGISTRY`]
/// states. The `while` loops are what a `const fn` has instead of an iterator.
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
    out
}

const REGISTRY_LEN: usize = TRAME_FIXTURES.len() + TOILE_FIXTURES.len();

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

    /// `DocState::new` returned `None` -- a `doc_type` whose engine does not
    /// exist yet (`Prose`, `Murmure`). Unreachable for any fixture this spec
    /// produces (excludes both) and present so the loader degrades cleanly
    /// when they arrive.
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
pub fn load_fixture(name: &str) -> Result<(DocumentId, DocState), FixtureError> {
    load_fixture_from(&corpus_root()?.join(CURRENT_DIR), name)
}

/// [`load_fixture`] against an explicit corpus directory.
///
/// The parameterized form the belt drives with a temporary directory, and what
/// `just fixtures-check` uses to load the tree it just regenerated instead of the
/// committed one. Same body, so testing this tests the shipping path.
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
/// Two hops. First the header, read type-agnostically -- the file names its own
/// app (`docfile` I2), so nothing here has to be told which importer to reach
/// for; that is what lets one `load_fixture` serve every corpus. Then the app's
/// own `import_ambre_plaintext`, which owns the payload half.
///
/// **The payload half is the app's to judge, not this function's**, and that is
/// why the second hop is not `DocState::import_snapshot`: `import_snapshot`
/// returns display events, and the engines disagree about how a refused payload
/// appears in them -- Trame returns none, Toile returns an error *event*. A
/// loader reading rejection off that vector would accept a corrupt Toile
/// artifact and hand back an empty board. Each engine's `import_ambre_plaintext`
/// returns a typed [`DocFileError::PayloadRejected`] instead, carried out
/// verbatim.
pub fn import_fixture(bytes: &[u8]) -> Result<(DocumentId, DocState), FixtureError> {
    let doc_type = read_ambre_plaintext(bytes)?.doc_type;
    // Pinned reader, not `DocState::new`: the receiving state commits its own
    // scaffolding before the merge, so the peer it picks lands in the imported
    // document. A random one would make a restored fixture differ run to run,
    // which `FIXTURE-019` compares, and would leave the manifest's peer count
    // unable to tell the reader from an author.
    let mut state = match doc_type {
        DocType::Trame => DocState::Trame(Box::new(TrameState::with_peer(FIXTURE_IMPORT_PEER))),
        DocType::Toile => DocState::Toile(Box::new(ToileState::with_peer(FIXTURE_IMPORT_PEER))),
        DocType::Prose | DocType::Murmure => return Err(FixtureError::NoEngine { doc_type }),
    };
    // The same bytes, header included: each app re-reads the header through
    // `read_ambre_plaintext_as`, which is also where the dispatch rule fires.
    // That re-reads (and re-copies) the payload, which is the price of keeping
    // one import path instead of two; nothing here is on a hot path.
    let document_id = match &mut state {
        DocState::Trame(trame) => trame.import_ambre_plaintext(bytes).map(|(id, _events)| id),
        DocState::Toile(toile) => toile.import_ambre_plaintext(bytes).map(|(id, _events)| id),
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
/// **Generation is a round trip**, and that is the point rather than a cost:
/// [`FixtureGen::build`] returns bytes while [`shape_of`] needs a decoded state,
/// so the driver re-imports what was just generated -- which proves the artifact
/// loads through the production importer before it is ever committed.
///
/// The driver, not the generator, produces this: a [`FixtureGen`] has no method
/// that could.
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
        // from the generated state rather than assumed: a manifest that names a
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
    // Pretty-printed with a trailing newline: the sidecar exists to be *read* in
    // a diff, and a single-line JSON blob would be as opaque as the binary
    // it describes.
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
/// Takes the registry as a parameter rather than reading [`REGISTRY`]: the belt
/// drives this same body with a synthetic registry while the shipping one is
/// still empty, so the walk is proven before the first real generator lands.
pub fn generate_into(
    registry: &[&dyn FixtureGen],
    dir: &Path,
) -> Result<Vec<FixtureManifest>, FixtureError> {
    registry
        .iter()
        .map(|generator| write_fixture(*generator, dir))
        .collect()
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
    /// Parsed rather than read from the manifest: the manifest of a frozen
    /// artifact is a copy of the one written at freeze time, so the path is the
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
    /// only while every version stays one digit: `s10` sorts before `s9` as text
    /// and after it as a number, so a corpus that survives ten schema bumps
    /// would start dropping its newest artifact instead of its oldest.
    fn age_key(&self) -> (u8, u32) {
        (self.ambre_version, self.schema_version)
    }
}

/// Every artifact under `<root>/frozen/`, newest last.
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
/// Two causes, one pass (`FIXTURE-021`, `FIXTURE-026`): an artifact past the cap
/// for its document type, and an artifact whose generator is gone, which has
/// lost the second term of its comparison for good.
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
/// never a diagnosis: the recipe prints them and the contributor reads the
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

    /// A frozen artifact that fails one of the three semantic checks: its digest
    /// against its bytes, its load through the migration chain, or its migrated
    /// canonical dump against the live artifact its manifest names
    /// (`FIXTURE-019`).
    Frozen { stem: String, reason: String },

    /// More frozen artifacts for one document type than retention keeps
    /// (`FIXTURE-021`). The remedy is `just fixtures`, which runs the pass.
    FrozenOverflow { doc_type: DocType, kept: usize },
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
        }
    }
}

/// A frozen artifact whose semantic comparison is retired rather than failed.
///
/// Two causes (`FIXTURE-020`, `FIXTURE-026`): the live generator declares a
/// different `generator_version`, or it is gone. Nothing can reconstruct what
/// the old generator would have produced at the new schema, so the artifact
/// reports as degraded and the next freeze restores the coverage.
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
/// `scratch` is a directory the caller owns and the gate regenerates into: the
/// committed tree is never written, so a red gate leaves the working copy
/// exactly as it found it.
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
    Ok(report)
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
