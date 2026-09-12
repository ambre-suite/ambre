// ambre-logic/src/fixtures.rs
// ============================================================================
// The document-fixture loader, registry and driver (`ambre-spec-fixtures.md`).
//
// Three homes, one dependency grain: the *types* are in `ambre-core` (the orphan
// rule puts `FixtureGen` upstream of both app crates); the *generators* sit next
// to the engine whose commands they drive; the *loader and driver* are here, in
// the lowest crate that sees every `DocState` variant.
//
// Fixture code (I4). `ambre-logic` ships to wasm, so the gate matters most in
// this crate: everything below is behind the non-default `fixtures` feature,
// while `DocState::import_snapshot` and `DocState::export_snapshot` are ungated
// production code and live in `lib.rs`.
//
// Not `#[cfg(test)]`: such a module is reachable only from its own crate's test
// harness, whereas `just fixtures` invokes the driver from a binary.
// ============================================================================

use std::fs;
use std::path::{Path, PathBuf};

use ambre_core::docfile::{AMBRE_VERSION, DocFileError, read_ambre_plaintext};
use ambre_core::{DocType, DocumentId, FixtureGen, FixtureManifest, FixtureShape};

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
/// Empty until `FIXTURES P1` writes the correctness generators: `P0` delivers the
/// mechanism, not the corpus. The driver takes its registry as a parameter
/// ([`generate_into`]) rather than reading this constant directly, so the belt
/// can drive the same code path with a synthetic registry while the shipping one
/// is still empty -- the pattern `ambre_core::docfile`'s migration walk already
/// uses.
pub const REGISTRY: &[&dyn FixtureGen] = &[];

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

    /// The bytes are not a valid `.ambre`. Carried verbatim, never reinterpreted:
    /// a caller that wants to distinguish a bad magic from an unsupported version
    /// matches on the inner error rather than on a flattened string.
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
/// On the loader side rather than on [`FixtureGen`], so the manifest-integrity
/// check has something independent to compare against: a generator
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
/// **Not** the CRDT -- two documents agreeing on content still differ in op
/// history, peer ids and commit timestamps, so no byte or state comparison can
/// express "same document by a different route". 's frozen belt compares
/// this, and text means a failure prints a readable diff rather than a length
/// mismatch.
///
/// **Read-only in meaning, not in bytes.** Reaching a Loro container by name
/// materializes it, so dumping a document can register containers a sparse one
/// had never created and change what its next `export_snapshot` emits. That is
/// the same lazy materialization every engine reseed already performs (which is
/// why defines the round trip as a fixed point rather than as byte equality),
/// and it is harmless to every consumer here -- the drift gate diffs bytes read
/// off disk, never bytes re-exported from a state that has been dumped. It is
/// nonetheless a reason not to dump a document and then persist it.
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
/// from the process working directory: `cargo test` runs a test binary with the
/// CWD at its own package root, so a bare relative `fixtures/` would resolve
/// differently for every consumer crate. The `env!` is expanded when *this*
/// crate compiles, so the anchor is `ambre-logic`'s own directory whoever calls.
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
/// path (: the fixture loader **is** the production importer).
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
    let mut state = DocState::new(doc_type).ok_or(FixtureError::NoEngine { doc_type })?;
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
/// degenerates to a keyless 32-byte digest, which is exactly what is wanted here.
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
