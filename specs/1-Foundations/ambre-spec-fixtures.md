# AMBRE -- FEATURE: THE FIXTURE CORPUS

**Type:** Horizontal (framework)
**Implementation:** partial -- generation and the loader ship; no artifact is committed, no gate reads one.

> Ambre documents in known states, built from a seed instead of by hand. A fixture is what a test or
> a benchmark starts from, and it is an ordinary `.ambre` file.

Without one, an Ambre document comes into existence only when a person clicks in the application.
Two consequences follow: the budgets of `ambre-spec-budgets.md` are unmeasurable,
because no large document exists to measure; and every state-dependent defect is reproduced by
hand-written setup inside one test, shared with nothing.

**Not the interop fixtures.** `crates/ambre-toile/src/logic/interop/fixtures/` holds **input** files,
third-party formats fed to a converter. These are document-state fixtures, the output of our own
engine. Import consumes theirs, export consumes ours; neither substitutes for the other.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`.

| ID | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|
| `FIXTURE-001` | For a generator and a seed, `build` MUST produce byte-identical output, across runs and across processes. | this spec section 2 | `fixture_determinism::a_generator_rebuilt_in_the_same_process_is_byte_identical`, `::a_generator_is_byte_identical_across_processes`, `::a_generator_that_ignores_its_seed_is_caught`; `fixtures_corpus::regenerating_rewrites_the_same_bytes` | verified |
| `FIXTURE-002` | A generator MUST pin all three sources of entropy -- the `PeerID`, the commit clock and the `document_id`. None is optional. | this spec section 2 | `fixture_determinism::an_unpinned_peer_is_not_deterministic`, `::the_commit_clock_is_not_recorded`, `::the_document_id_is_part_of_what_the_digest_covers`; `ambre_core::fixtures::tests::a_seeded_document_id_is_stable_distinct_and_well_formed` | verified |
| `FIXTURE-003` | Determinism MUST hold at the scale a fixture is built at, and on a document that has been through a merge. | this spec section 2 | `fixture_determinism::a_large_document_is_deterministic` (5 000 cells, wide enough for the encoder's wide paths), `::a_two_peer_merge_is_deterministic`. Two edges stay open by choice. The 100k-cell case belongs to the `FIXTURES P2` benchmark corpus, because it is too slow for a per-commit belt. No test exercises an architecture other than x86_64 | verified |
| `FIXTURE-004` | A generator MUST build through `handle_command` on the app state; it MUST NOT write CRDT containers directly. | this spec section 2 | none. Every generator obeys the rule, and nothing stops the next one from writing to the `LoroDoc` directly. Such a fixture holds a state that no user action can reach, so it proves nothing | unverified |
| `FIXTURE-005` | Every committed artifact MUST trace to a generator run from a seed, and MUST NOT come from a real user document. | this spec section 2 | none is possible: a real document and a generated one are both Loro bytes, so no gate can tell them apart. It is a **review rule**, and it belongs in the contribution checklist. A committed real document would place plaintext user content in a public AGPL repository, permanently, in git history; a fixture that is interesting *because* it came from real data is exactly the one that must not be committed -- write a generator that reproduces its shape instead | human-review |
| `FIXTURE-006` | A committed fixture whose bytes drift MUST fail CI. | this spec section 3 | not built -- lands with `FIXTURES P1 -- the corpus and its gate` (`ambre-roadmap.md` EF). Nothing is committed and no recipe compares a corpus against a regeneration | unimplemented |
| `FIXTURE-007` | Fixture machinery MUST NOT reach a client build. The three production symbols named in section 2 are the whole exception. | this spec section 2; section 3 | crate graph: `ambre-app` names no `fixtures` feature, and Cargo never enables a dependency feature implicitly. The surface is therefore unreachable from the wasm artifact today. Nothing refuses the manifest line that would change that | unverified |
| `FIXTURE-008` | Loading a fixture MUST go through the production importer; no test-only reader MUST read a `.ambre`. | this spec section 2 | `fixtures_corpus::a_written_fixture_loads_back_through_the_production_importer` | verified |
| `FIXTURE-009` | A generator whose declared document type contradicts the bytes it built MUST be refused. | this spec section 2 | `fixtures_corpus::a_generator_whose_doc_type_contradicts_its_bytes_is_refused` | verified |
| `FIXTURE-010` | Two generators MUST NOT share a name: the name is the artifact's identity on disk. | this spec section 2 | `fixtures_corpus::the_shipping_registry_has_no_duplicate_names`, `::the_driver_walks_a_whole_registry_and_creates_its_subdirectories` | verified |
| `FIXTURE-011` | A manifest MUST describe the artifact beside it, and its digest MUST cover the whole file, header included. | this spec section 2 | `fixtures_corpus::the_manifest_describes_the_artifact_beside_it`, `::a_flipped_header_byte_moves_the_digest`, `::the_shape_counts_what_the_document_holds`, `::a_solo_document_reports_one_peer_and_a_merged_one_reports_two` | verified |
| `FIXTURE-012` | A manifest MUST be readable as text and MUST read back equal to what was written. | this spec section 2 | `fixtures_corpus::the_sidecar_is_readable_json_and_reads_back_equal` | verified |
| `FIXTURE-013` | A document that has been through the export and import cycle MUST be a fixed point of it, on every document type with an engine. | this spec section 2 | `fixture_determinism::the_round_trip_is_stable_on_every_doc_state_variant`, `::every_variant_reports_its_own_doc_type` | verified |
| `FIXTURE-014` | Every corpus and container failure MUST reach the caller typed, and MUST NOT panic. | this spec section 2 | `fixtures_corpus::every_header_failure_is_typed_and_none_of_them_panics`, `::a_container_failure_arrives_intact_never_flattened_into_a_string`, `::a_missing_name_is_not_found_not_an_io_error`, `::an_unreadable_corpus_root_is_an_io_error`, `::a_malformed_sidecar_is_reported_as_a_manifest_failure`, `::a_doc_type_with_no_engine_degrades_cleanly` | verified |
| `FIXTURE-015` | The corpus root MUST be resolved from the workspace, never from the process working directory. | this spec section 2 | `fixtures_corpus::the_corpus_root_is_resolved_from_the_manifest_dir_not_the_cwd` | verified |
| `FIXTURE-016` | The canonical dump MUST be blind to the route a document took to its content. | this spec section 2 | `fixtures_corpus::the_canonical_dump_is_blind_to_the_route_a_document_took`, `::a_toile_dump_names_nodes_by_paint_order_not_by_element_id` | verified |
| `FIXTURE-017` | The canonical dump of a formula cell MUST carry both the stored source and the evaluated result. | this spec section 2 | `fixtures_corpus::the_canonical_dump_carries_both_representations_of_a_formula` | verified |
| `FIXTURE-018` | A frozen artifact MUST NOT be regenerated. | this spec section 2; `ambre-spec-docfile.md section 3` | not built -- lands with `FIXTURES P1 -- the corpus and its gate` (`ambre-roadmap.md` EF) | unimplemented |
| `FIXTURE-019` | A migrated frozen artifact MUST produce the canonical dump of the live artifact its manifest names. | this spec section 2 | not built -- lands with `FIXTURES P1 -- the corpus and its gate` (`ambre-roadmap.md` EF) | unimplemented |
| `FIXTURE-020` | That comparison MUST be retired, not failed, when the frozen manifest and the live generator disagree on `generator_version`. | this spec section 2 | not built -- lands with `FIXTURES P1 -- the corpus and its gate` (`ambre-roadmap.md` EF) | unimplemented |
| `FIXTURE-021` | The frozen set MUST keep the newest three artifacts per document type, and drop the rest. | this spec section 2 | not built -- lands with `FIXTURES P1 -- the corpus and its gate` (`ambre-roadmap.md` EF) | unimplemented |
| `FIXTURE-022` | A benchmark corpus MUST NOT be committed. | this spec section 3 | not built -- lands with `FIXTURES P2 -- scale corpora and benches` (`ambre-roadmap.md` EF). The `.gitignore` entry lands with the recipe that writes the corpus | unimplemented |
| `FIXTURE-023` | A committed benchmark baseline MUST name the machine it was measured on. | this spec section 3 | not built -- lands with `FIXTURES P2 -- scale corpora and benches` (`ambre-roadmap.md` EF) | unimplemented |
| `FIXTURE-024` | A benchmark built on a fixture MUST NOT be reported as a frame-rate validation. | this spec section 3; `ambre-spec-testing.md section 5` | none, and none is possible: the claim is made in a pull request, not in a test. The reviewer refuses it, and section 3 states what the corpus does measure so the honest claim is at hand | human-review |
| `FIXTURE-025` | The committed corpus and the generator registry MUST match one for one: an artifact whose generator is gone, and a generator with no committed artifact, MUST each fail the gate. | this spec section 3 | not built -- lands with `FIXTURES P1 -- the corpus and its gate` (`ambre-roadmap.md` EF). A byte diff alone reports neither, because each case is missing the term the diff would compare, so both pass silently on the very run that introduces them | unimplemented |
| `FIXTURE-026` | A frozen artifact whose manifest names a generator that no longer exists MUST report as degraded, and MUST be dropped at the next retention pass. | this spec section 2 | not built -- lands with `FIXTURES P1 -- the corpus and its gate` (`ambre-roadmap.md` EF). The outcome of `FIXTURE-020` for a different cause: a renamed or deleted generator leaves the comparison without a second term rather than with a stale one | unimplemented |

* **Budget.** the tooling targets of `ambre-spec-budgets.md`: `just fixtures` in
  ~30 s, `just fixtures-check` in ~10 s, and the committed artifact cap. `FIXTURE-021` is what keeps
  the second inside its figure as bumps accumulate. The binary cost is zero, and `FIXTURE-007` is
  the rule that keeps it there.
* **Boundary.** none: the machinery is host-only and gated behind a non-default `fixtures` feature
  that `ambre-app` never names (`FIXTURE-007`), so it links into no shipped binary and reaches no
  relay. The one key it touches is derived at test setup, guards nothing, and is never written to
  disk (section 2).
* **Verification.** 13 `verified`, 2 `unverified`, 2 `human-review`, 9 `unimplemented`. Determinism
  is held on four axes including the cross-process one, and the whole loader is held: the round
  trip, every typed failure, the root resolution and both canonical-dump rules. The two
  `unverified` are the same shape -- a rule nothing refuses breaking, one on how a generator builds
  and one on what a manifest may declare. Every `unimplemented` row is the corpus itself, which does
  not exist: `fixtures/` is not in the tree, so freezing, retention, drift and the match against the
  registry are specified and unbuilt.

---

## 2. MODEL

**CRDT containers.** none. This feature holds no collaborative state: it serializes and
deserializes the applications' own, and every container belongs to their schema documents.

**Three homes, one dependency grain.** The trait must sit upstream of its implementors, and Rust's
orphan rule decides which crate that is.

| Home | Holds | Why there |
|---|---|---|
| `ambre-core` | `FixtureGen`, `FixtureManifest`, `FixtureShape`, `ShapeBody`, the seeded `document_id` | upstream of everything: `ambre-bus` depends on it and both applications depend on `ambre-bus`. It already owns `DocType`, `serde` and `hex` |
| `ambre-trame`, `ambre-toile` | the generators, and the pinned-peer constructor | a generator drives the engine's own `handle_command`, so it moves in the same commit as the schema it exercises |
| `ambre-logic` | the loader, the driver, `shape_of`, `canonical_dump` | the lowest crate that sees every document type |

**The three sources of entropy.** For a generator and a seed, `build` must produce the same bytes on
any machine. Three sources must be pinned, none of them optional (`FIXTURE-002`).

| Source | Why it is not deterministic | Pinned by |
|---|---|---|
| `PeerID` | `LoroDoc::new()` draws one at random, and it reaches the **content**: Trame builds sheet ids from the peer id, and `ToileState` caches the peer at construction | `TrameState::with_peer`, `ToileState::with_peer` |
| the commit clock | Loro records a timestamp per commit | `set_record_timestamp(false)`, applied by the same constructor |
| `document_id` | the exporter takes a UUID v7, which is time-ordered and partly random | derived from the seed rather than minted |

**A semantic digest is not an escape hatch for the first of those.** Peer-derived ids are part of the
decoded state, so an unpinned `PeerID` defeats a semantic digest exactly as it defeats a byte one.

**The two constructors are production code, and deliberately.** `TrameState::with_peer` and
`ToileState::with_peer` exist for this document alone, and what asserts on them is the determinism
belt, which runs under a bare `cargo test --workspace`. Gating them would put the belt out of CI's
reach. They cost a client nothing: `ambre-app` calls neither, and neither pulls in a fixture type.
**Never for a live document:** two concurrent writers sharing a `PeerID` mint colliding op ids and
corrupt the history.

**The artifact is a `.ambre` file, and this document invents no container**
(`ambre-spec-docfile.md section 3`). A committed fixture is the plaintext form, taken before the
compression and encryption `ambre-sync` applies downstream: same format, same parser, two ends of
one pipe. Committing the pre-Sync form is what `FIXTURE-001` requires, because zstd output is not
guaranteed stable across a dependency bump or a level change, which would turn that bump into drift
unrelated to the document.

**The sidecar manifest.** The header carries the document type and id but no provenance, which a
drift gate needs. Each artifact keeps a JSON sidecar beside it: an opaque binary has no reviewable
diff, so a binary encoding would defeat the sidecar's only purpose.

```rust
// ambre-core

#[derive(Serialize, Deserialize, PartialEq, Eq, Clone, Debug)]
pub struct FixtureManifest {
    /// Generator identity, for example "trame/formula_dag_deep". Also the
    /// artifact **stem**: the whole filename in `current/`, the part before the
    /// `@` in `frozen/`. Owned, because a manifest is read back off disk.
    pub generator: String,
    /// Bumped by the generator author when the produced document changes on
    /// purpose. Frozen into the artifact too: that is how a frozen fixture and
    /// the current one are found to be no longer comparable (`FIXTURE-020`).
    pub generator_version: u32,
    /// Declared by the generator, and pins the `PeerID` and the `document_id`.
    pub seed: u64,
    pub doc_type: DocType,
    /// The document's two independent versions at write time -- current for a
    /// generated artifact, superseded for a frozen one. Generators write
    /// `schema_version: 0` until the payload register exists, 0 being what an
    /// absent one reads as (`ambre-spec-architecture.md section 2`).
    pub ambre_version: u8,
    pub schema_version: u32,
    /// BLAKE2b over the whole file, header included (`FIXTURE-011`). Hex, not
    /// 32 raw integers, which would defeat the readable diff this sidecar
    /// exists for.
    #[serde(with = "hex")]
    pub digest: [u8; 32],
    pub shape: FixtureShape,
}

/// Coarse size summary: when the bytes move, this says whether the document
/// grew, lost a sheet, or is unchanged in shape. Not a selection index --
/// lookup is by name.
#[derive(Serialize, Deserialize, PartialEq, Eq, Clone, Debug)]
pub struct FixtureShape {
    /// Distinct peers whose ops are present; 1 for a solo document.
    pub peers: u8,
    pub body: ShapeBody,
}

/// Per-application rather than a union of nullable counters: an application
/// that joins later adds a variant instead of widening every existing row with
/// a field that means nothing to it.
#[derive(Serialize, Deserialize, PartialEq, Eq, Clone, Debug)]
pub enum ShapeBody {
    Trame { cells: u32, sheets: u16, formulas: u32 },
    Toile { elements: u32, frames: u16, connectors: u32 },
}
```

**The generator.** Method-based rather than all-associated-item: the driver iterates a registry of
trait objects, and a trait of bare constants is not object-safe.

```rust
// ambre-core

pub trait FixtureGen {
    /// Stable identity: the manifest's `generator` and the artifact stem. Unique
    /// across the registry (`FIXTURE-010`).
    fn name(&self) -> &'static str;
    fn doc_type(&self) -> DocType;

    /// A constant the generator author picks once. It belongs to the generator
    /// rather than the driver because a seed supplied from outside could differ
    /// between runs, turning the drift gate into noise. Changing it is a content
    /// change and takes a `generator_version` bump with it.
    fn seed(&self) -> u64;

    /// Bumped by the author when the produced document changes on purpose; the
    /// driver copies it into the manifest, having no other source for it.
    fn generator_version(&self) -> u32;

    /// Build the document and return the plaintext `.ambre` bytes. Not a raw
    /// snapshot: the header makes the artifact self-describing and loadable
    /// through the production importer (`FIXTURE-008`).
    ///
    /// Implemented by driving `handle_command` on the app state (`FIXTURE-004`);
    /// `seed` pins the three entropy sources (`FIXTURE-002`).
    ///
    /// Infallible: a generator drives commands it chose itself, so a rejected
    /// one is a bug, not a runtime condition. It **panics** rather than emit a
    /// half-built document -- a corpus that silently lost a merge is worse than
    /// a failed run.
    fn build(&self) -> Vec<u8>;
}
```

**The driver, and why generation is a round trip.** `build` returns bytes and `shape_of` needs a
decoded state, so writing a manifest means importing what was just generated. The extra import is
the point rather than a cost: it proves the artifact loads through the production importer before it
is ever committed. The driver writes the sidecar; a generator has no method that could.

**The loader.** Two hops: the container yields a header, then the payload fills an empty state.

```rust
// ambre-logic -- production, ungated (FIXTURE-007)

impl DocState {
    /// Load a full Loro snapshot into a state, returning the display events the
    /// application emits.
    pub fn import_snapshot(&mut self, payload: &[u8]) -> Vec<AppEvent>;

    /// Lift the per-application `export_snapshot` to the enum. `LogicState`'s
    /// takes a `DocumentId` and needs a live registry entry, so it cannot serve
    /// the bare `DocState` the round trip quantifies over (`FIXTURE-013`).
    pub fn export_snapshot(&self) -> Vec<u8>;
}

// ambre-logic -- fixture code, behind the `fixtures` feature

/// Decode a state into the summary the manifest records. On the loader side
/// rather than on `FixtureGen`, so manifest integrity has something independent
/// to compare against: a generator declaring its own shape would assert only
/// that it agrees with itself (`FIXTURE-011`).
pub fn shape_of(state: &DocState) -> FixtureShape;

/// The canonical semantic form: every user-visible value, in a deterministic
/// order, as text. **Not** the CRDT -- two documents agreeing on content still
/// differ in op history, peer ids and commit timestamps, so no byte or state
/// comparison can express "the same document by a different route"
/// (`FIXTURE-016`). Text, so a failure prints a readable diff.
pub fn canonical_dump(state: &DocState) -> String;

/// `<workspace root>/fixtures`, resolved from the compiled crate's own
/// directory (`FIXTURE-015`).
pub fn corpus_root() -> Result<PathBuf, FixtureError>;

/// Load a committed fixture by name -- the entry point every consumer uses.
pub fn load_fixture(name: &str) -> Result<(DocumentId, DocState), FixtureError>;

/// The same, against a caller-supplied directory: what a test drives.
pub fn load_fixture_from(dir: &Path, name: &str) -> Result<(DocumentId, DocState), FixtureError>;

/// The production import path over bytes already in hand: `read_ambre_plaintext`
/// for the header, `DocState::new(doc_type)` for the empty state, then the
/// **application's own** `import_ambre_plaintext` to fill it.
///
/// That last hop is the application importer, not `DocState::import_snapshot`,
/// and the difference is load-bearing: `import_snapshot` returns display events,
/// and the two engines disagree about how a refused payload appears in them --
/// one returns none, the other returns an error *event*. A loader reading
/// rejection off that vector would accept a corrupt artifact and hand back an
/// empty document. The application importer returns a typed error instead,
/// carried out verbatim (`FIXTURE-014`).
pub fn import_fixture(bytes: &[u8]) -> Result<(DocumentId, DocState), FixtureError>;

pub fn artifact_path(dir: &Path, name: &str) -> PathBuf;
pub fn manifest_path(dir: &Path, name: &str) -> PathBuf;
pub fn read_manifest(dir: &Path, name: &str) -> Result<FixtureManifest, FixtureError>;
pub fn manifest_for(gen: &dyn FixtureGen, bytes: &[u8], state: &DocState) -> FixtureManifest;
pub fn digest_of(bytes: &[u8]) -> [u8; 32];
pub fn write_fixture(dir: &Path, gen: &dyn FixtureGen) -> Result<FixtureManifest, FixtureError>;
pub fn generate_into(dir: &Path, registry: &[&dyn FixtureGen]) -> Result<(), FixtureError>;

/// Failures of the corpus, distinct from failures of the container.
/// `DocFileError` stays the authority on malformed bytes; this wraps it and
/// adds the conditions only a corpus has (`ambre-spec-docfile.md section 3`).
pub enum FixtureError {
    /// No such artifact under the corpus root.
    NotFound { name: String },
    /// Unreadable corpus root, permissions, IO.
    Io(std::io::Error),
    /// The bytes are not a valid `.ambre`. Carried verbatim, never reinterpreted.
    Container(DocFileError),
    /// `DocState::new` returned `None` -- a document type whose engine does not
    /// exist yet. Unreachable for any fixture this document produces, and
    /// present so the loader degrades cleanly when they arrive.
    NoEngine { doc_type: DocType },
    /// The sidecar is missing, or is not the JSON the manifest describes.
    Manifest { name: String, reason: String },
}
```

**The boundary is by symbol, not by crate** (`FIXTURE-007`). `ambre-logic` ships to wasm, so "no
fixture code in the client binary" has to say which symbols count.

| Symbol | Gated | Why |
|---|---|---|
| `DocState::import_snapshot` | no | the enum-level restore path, which dispatches to the Toile one |
| `DocState::export_snapshot` | no | the other half of the round trip, and a save path in its own right |
| `ToileState::import_snapshot` | no | gating it makes an ordinary client build fail to compile, and takes Toile's local restore with it |
| everything else here | yes | behind the non-default `fixtures` feature, which `ambre-app` never names |

**Two trees, and their lifecycles differ.**

| Path | Written by | Regenerated | Committed |
|---|---|---|---|
| `fixtures/current/<name>.ambre` and `.manifest` | the driver, from a live generator | every run | yes |
| `fixtures/frozen/<name>@v<ambre>-s<schema>.ambre` and `.manifest` | the freeze recipe, once | never (`FIXTURE-018`) | yes |
| `fixtures/bench/<name>.ambre` and `.manifest` | the bench recipe | on demand | no (`FIXTURE-022`) |

The `@v...-s...` suffix records the **outgoing** versions, the ones the artifact was written at
before the bump that froze it. Membership is positional: a frozen artifact is one that sits in
`fixtures/frozen/`. No flag in the manifest, which could only ever contradict the path.

**Why a frozen tree exists at all.** A writer emits the current container version only, and
`handle_command` builds at the current schema version, so once a version is superseded no generator
can produce an artifact at the old one. A frozen fixture is outside the generator set, not a golden
we declined to refresh, which is what reconciles it with the drift gate. Freezing is therefore a
step of the version bump itself (`ambre-spec-docfile.md section 3`).

**The belt on a frozen artifact is semantic, in strengthening order:** its digest still matches its
bytes; it still loads through the migration chain; and its migrated canonical dump equals that of
the live artifact its manifest names (`FIXTURE-019`). The third is the one with teeth, and it is
retired rather than repaired on a `generator_version` mismatch (`FIXTURE-020`): nothing can
reconstruct what the old generator would have produced at the new schema, so the artifact reports as
degraded and re-freezing at the next bump restores the coverage. A generator that has been renamed
or deleted degrades the same way and stays degraded, since the manifest then names nothing at all
(`FIXTURE-026`).

**"Newest" is the lexicographic order of the version pair** the filename already carries
(`FIXTURE-021`). The two axes are independent, but a freeze only ever happens at a bump of one of
them and neither ever decreases, so the pair is strictly increasing across freezes of a given
document type and orders them totally. No timestamp is stored, and none is needed.

**The db tier reuses the same fixture, never a second artifact.** Where a server-side test needs
stored bytes, the fixture takes the ordinary client path: the importer strips the header, because
the network carries a bare snapshot, then `seal_snapshot` compresses and seals it into what a real
client would upload. The setup derives two secrets from the fixture seed, a document key and a
signing key, and synthesizes the public data the AAD is built from. Both keys are **test keys over
synthetic content**: they guard nothing, the plaintext they seal is committed in the clear beside
them, and neither is written to disk. Hand-rolled bytes would prove that the server serves
something, not that it serves what a client produces.

**Data flow.** No worker, no channel, no transport.

```
  FixtureGen::build()               seed() is the generator's own constant
        |
        |  handle_command(..) x N          (the same entry point the UI drives)
        v
   TrameState / ToileState
        |
        |  export_ambre_plaintext()        (production exporter, not a test path)
        v
   plaintext .ambre bytes
        |
        |  the driver re-imports them to compute the shape,
        |  which also proves they load before anything is committed
        v
   .ambre file + manifest  ->  disk
        |
        |  load_fixture(name) -> import_fixture(bytes)
        |    read_ambre_plaintext            (the header, type-agnostic)
        |    DocState::new(doc_type)
        |    <app>::import_ambre_plaintext   (production importer, typed refusal)
        v
   a test or a benchmark, starting from a known state

   ...and, for the db tier only, that same payload -- the bare snapshot, no
   header, as the network carries it -- continues through the ordinary client
   pipeline (seal_snapshot: zstd + AEAD) to produce the bytes a real upload
   would carry. Derived at setup, never committed.
```

**Code:** `crates/ambre-core/src/fixtures.rs` holds the trait and the manifest types;
`crates/ambre-logic/src/fixtures.rs` the loader and the driver; `crates/ambre-trame/src/logic/fixtures.rs`
and `crates/ambre-toile/src/logic/state/fixtures.rs` the generators and the shape decoders. The belt
is `crates/ambre-logic/tests/fixture_determinism.rs` and `tests/fixtures_corpus.rs`.

---

## 3. SURFACE

The surface is a contributor's, not a user's: five recipes and one feature flag.

| Trigger | Behaviour |
|---|---|
| `just fixtures` | regenerate `fixtures/current/` in place, and delete any artifact whose generator no longer exists. Detecting an orphan is the gate's job; removing one is this recipe's, so the fix for a red gate is always to run this, never a manual delete |
| `just fixtures-check` | regenerate into a temporary directory and diff against the committed goldens, refuse a golden whose generator is gone and a generator with no golden (`FIXTURE-025`), then load every frozen artifact and assert its migration (`FIXTURE-006`). Runs in the CI host job |
| `just fixtures-freeze <name>` | copy the **named** artifact out of `current/` into `frozen/`, deriving the version suffix from its manifest rather than from a person typing it. A name, not a document type: several generators share one, and which lineage keeps migration coverage is a deliberate choice |
| `just fixtures-bench-corpus` | produce the large, git-ignored scale corpora into `fixtures/bench/` (`FIXTURE-022`) |
| `just fixtures-bench` | run the `criterion` benches, building the corpus first when it is absent. Split from the recipe above because generating is minutes and re-running benches is not |

All five pass `--features fixtures`, and the two bench recipes add `pdf`, because Toile's PDF writer
is behind that non-default feature and the presentation bench cannot otherwise be compiled in. A
bare `cargo test` sees none of this, which is why the gate cannot ride inside the existing
`cargo test --workspace` and the host job carries an explicit line for it. None of the five touches
the network, a database or a browser.

**What the benches read.** Each corpus is sized to a figure its owning budget already states; no
target is set or restated here.

| Budget, in `ambre-spec-budgets.md` | The corpus the bench needs |
|---|---|
| 10k-cell DAG recomputation | a deep formula DAG, 10k formula cells |
| document size and cache size at 100k cells | a 100k-cell sheet, dense and formula-heavy variants |
| snapshot import, delta batch, sheet index rebuild | the same corpora, plus recorded delta batches |
| visible-set query and index update at 50k elements | a 50k-element board |
| board paint at 2 000 visible elements | a dense board with 2 000 elements in one viewport |
| xlsx and CSV export at 10k cells | a 10k-cell sheet |
| PDF export of 50 slides | a 50-frame presentation sequence |

**No CI job runs a bench.** The corpora are git-ignored and minutes to build, so benches are local
and on demand. The author of a performance-relevant change produces the numbers and puts them in the
pull request, with the machine they came from (`FIXTURE-023`): `criterion` measures wall clock, so a
number from one machine means nothing on another, and a committed baseline read as an absolute
target fails on hardware rather than on code.

**What a bench on fixtures does not measure.** The frame-rate budget is GPU-bound and stays
validated visually (`ambre-spec-testing.md section 5`), so no fixture bench may be reported as
holding it (`FIXTURE-024`). What fixtures do unlock is the **CPU half** of the frame -- the half the
pillars make a correctness claim about, that the render thread is never blocked by computation:
content walk, culling, hit-testing, tessellation, text layout and batch build, all measurable on
host with no GPU through the recording surface.

---

## 4. DECISIONS

| Decision | Rationale | Rejected alternative |
|---|---|---|
| A fixture is an `.ambre` file | every test run then exercises the production import path, and the corpus becomes the regression belt for the format itself. An earlier draft of this document invented a container: it would carry the same bytes through a second, test-only reader and writer, unexercised by users, free to drift from the shipping importer, and a duplicated migration surface (`FIXTURE-008`) | a bespoke fixture container; and a textual format, RON or JSON, which is readable in a diff and is a second, uncompiled copy of each schema, drifting exactly as `ambre-spec-template.md section 0` warns |
| Generators are code, seeded and versioned | a fixture must be deterministic, replayable, diffable and runnable in CI. Generated prose output is none of those: the same prompt yields a different document each run, so no golden can be committed and no gate can exist | a document produced by a language model |
| A generator drives `handle_command` | writing CRDT containers directly is faster and produces documents no user action can produce, including states the engine's own invariants forbid. Such a fixture tests the test (`FIXTURE-004`) | writing the containers directly, bypassing the command surface |
| The seed is the generator's own constant | a seed supplied from outside could differ between runs, which turns the drift gate into noise. Changing it is a content change and takes a `generator_version` bump with it | a seed passed to `build` by the driver |
| The trait is defined in `ambre-core` | the orphan rule requires an implementing crate to depend on the trait's home, and both applications sit *upstream* of `ambre-logic`. Types go to `ambre-core`, behaviour stays in `ambre-logic` | defining `FixtureGen` in `ambre-logic` next to the loader, which is the natural reading of "the machinery lives in `ambre-logic`" and is impossible |
| The generators sit next to their engine | a generator depends on the engine's command surface and must change in the same commit as the schema it exercises. Proximity enforces that coupling; a separate crate lets it drift silently, which is the failure this document exists to prevent elsewhere | a separate `ambre-fixtures` crate |
| Two trees, membership positional | a flag in the manifest is free to disagree with reality, and nothing could adjudicate. A directory cannot (`FIXTURE-018`) | one corpus tree with a `frozen` flag in each manifest |
| A frozen artifact is compared by canonical dump | two documents agreeing on every user-visible value still differ in op history, peer ids and commit timestamps, which is the same fact `FIXTURE-001` pins around. The dump compares content and prints a readable diff when it fails (`FIXTURE-019`) | comparing the migrated frozen document against the current one by state or by bytes |
| The retention cap is three per document type | the gate loads and migrates every frozen artifact, so an unbounded set walks into its 10 s budget one bump at a time. The cap states how far back migration is exercised, rather than leaving it to be discovered when CI slows down (`FIXTURE-021`) | keeping every frozen artifact, and discovering the cost as a CI regression |
| Only the small corpora are committed | git keeps a committed artifact forever, and the scale corpora are multi-megabyte and fully reproducible from a seed. Generated on demand and git-ignored instead (`FIXTURE-022`) | committing the benchmark corpora, for a convenience a seed already provides |
| "No silent golden refresh" is a review rule | the gate compares the corpus against the code, never against the corpus's own history, so any regenerate-and-commit passes. The manifest makes a refresh **visible** instead, moving a digest and usually a shape in a JSON file read as plain text, and `generator_version` is the author's declaration that the change was intended | a gate that claims to enforce it, which would put a machine check's name on a review rule |
| A real document is never committed | plaintext user content in a public AGPL repository, irreversibly, and a fixture that is interesting *because* it came from real data is exactly the one that must not be committed. No check can hold the rule, and `FIXTURE-005` says so rather than implying otherwise | capturing a fixture from a real document, and trusting the gate to notice |

---

## 5. NOT BUILT, AND NOT YET RULED

* **The corpus does not exist.** There is no `fixtures/` directory in the tree, so ten rows of
  section 1 describe a corpus, a gate, a freeze and a retention cap that nothing performs. The
  machinery underneath them is built and held by its own belt.
* **None of the five recipes exists.** The host job carries a package-scoped `cargo test` line in
  place of `just fixtures-check`, so the loader and the driver are exercised while the corpus rules
  are not.
* **No generator is registered outside a test.** Both engines carry a fixture module and the driver
  walks a registry, and the only registries that exist are built inside the belt.
* **Determinism has been measured on one architecture.** Every figure and every determinism test
  runs on x86_64. A failure elsewhere reopens `FIXTURE-001`, and the fallback is a semantic digest,
  which is weaker because it cannot see a peer-derived id.
* **Nothing refuses a generator that writes containers directly**, and nothing refuses a manifest
  line that would pull the fixture feature into a client build. Both are `unverified` rows rather
  than gaps, because the rule is stated and the mechanism is missing.
* **The 2-peer generator is Trame-only.** It reuses the mock sync seam that replays the relay step
  without a network, and `ambre-toile` has no counterpart, so a post-convergence fixture is a Trame
  document until one exists.
* **A fixture proves no convergence.** One planned generator emits a post-convergence state; that
  is a fixture **produced by** the converged engine, never a proof of it. The 2-peer property tests
  remain the sole authority.
* **One link of the load path stays out of reach.** A db-tier test exercises the real read path over
  opaque blobs, and the host legs are pure functions of a snapshot. What neither covers is a real
  client speaking WebSocket to a running server, which needs a native transport for `ambre-sync`.
  Named so it is not mistaken for coverage.

---

## 6. WHAT THIS DOCUMENT DOES NOT OWN

| Question | Owner |
|---|---|
| The container a fixture is written in, and its two version rules | `ambre-spec-docfile.md section 3` |
| What a document means: containers, commands, convergence | the applications' schema and engine documents |
| Which crate holds which belt, what runs in CI, and where the mechanism gaps are | `ambre-spec-testing.md section 6` |
| What each operation may cost | `ambre-spec-budgets.md` |
| How a snapshot is compressed, sealed and uploaded | `ambre-spec-sealed-envelope.md` |
| What a test asserts about a document loaded from a fixture | the feature that owns the behaviour; this document produces the corpus, never the expectations |
