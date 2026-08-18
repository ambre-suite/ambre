# AMBRE — SPEC: FIXTURES (deterministic document corpora)
**Type:** Horizontal (Foundation — Quality / Tooling)
**Status:** Draft
**Implementation:** partial

> Producing Ambre documents in known, representative states -- deterministically, on the host target, from
> code. A fixture is a document the test suite and the benchmarks start from instead of building by hand.
>
> **Problem solved.** Today an Ambre document only comes into existence by a human clicking in the WASM SPA.
> Two consequences: the performance budgets of `ambre-spec-trame-logic.md` and `ambre-spec-toile-logic.md`
> are **unmeasurable**, because no large document exists to measure against; and every state-dependent
> defect (the view-dependent merge masking of `trame-logic §11 bis.E` is the reference case) is reproduced
> by hand-written setup inside individual tests, repeated and unshared.
>
> **Not the interop fixtures.** `ambre-toile/src/logic/interop/fixtures/*.svg` are **input** files --
> third-party formats fed to a converter. These are **document-state** fixtures: serialized Ambre CRDT
> states, output of our own engine. Import consumes theirs, export consumes ours (§4); neither substitutes
> for the other.

---

## 1. OBJECTIVES & CONSTRAINTS

### A. Scope

**Handles:** building document states by driving the app engines; serializing them through the existing
`export_ambre_plaintext`; a manifest and digest per artifact; a CI drift gate; the loader side of the round
trip, including the `load_fixture` entry point consumers call.
The corpora it produces serve three consumers: **correctness** tests that need a hard-to-reach state,
**interop export** tests that need a rich source document, and **benchmarks** that need scale (§4).

**Delegates:**
* Document semantics -- owned by the app specs (`2-Trame/`, `3-Toile/`).
* Test assertions -- owned by each crate's own belt. This spec produces the corpus, not the expectations.
* Test tiering, CI shape and the tested Definition of Done -- normative in `ambre-spec-architecture.md §4`,
  realization tracked in `ambre-spec-testing.md`. Referenced, never redefined.

**Out of scope, explicitly:**
* **Any network.** Generators run in-process on host and write to disk. Nothing here opens a socket.
* **Any cryptography inside generation.** `FixtureGen::build` is crypto-free; a document fixture is
  plaintext (§2.A). The one crypto touch is the *derived* sealed blob seeding the db tier (§2.A), which
  calls the existing `ambre-sync` seal pipeline -- test setup, not generation.
* **Interop fixtures** (third-party input files) -- see the note above.
* **Fixtures for apps that have no engine.** `ambre-prose` and `ambre-murmure` are stubs
  (`ambre-spec-testing.md §2`); they join when `DocState::new` supports them.

### B. Dependencies

**No new external dependency.** Only workspace crates that already build and test on host:

| Crate | Used for | Scope |
|---|---|---|
| `ambre-core` | Home of `FixtureGen`, `FixtureManifest`, `FixtureShape` (§2.B); already holds `DocType` and already depends on `serde` + `hex` | Definition |
| `ambre-trame`, `ambre-toile` | Driving the engines; the `FixtureGen` impls | Generation |
| `ambre-logic`, `ambre-bus` | The loader, the driver and the artifact types they carry | Loading |
| each app's `interop` module | `export_ambre_plaintext` / `import_ambre_plaintext` -- the `.ambre` container (§2.A) | Generation + loading |
| `ambre-crypto` | `hash::chain_hash` -- the manifest digest | Generation |
| `ambre-sync` | `pipeline::seal_snapshot` -- the derived db-tier blob (§2.A) | Test setup only, not generation |

**One new dependency edge, no new external crate.** `serde_json` is a workspace dependency but not yet one
of `ambre-logic`'s, where the driver lives; that `Cargo.toml` edit rides with `FIXTURES P0`. `ambre-core`
already has `serde` and `hex`.

`TrameState` alone already carries a few hundred host tests, so a fixture generator is ordinary host code
calling the same public API those tests call. If an implementation needs a **third-party** crate, that is a
signal it is reaching outside the engine instead of driving it.

### C. Invariants

* **I1 — Determinism.** For a given generator and seed, `build` produces **byte-identical** output across
  runs and machines. Three sources of entropy must be pinned, none of them optional.
  * **`PeerID`.** `LoroDoc::new()` draws one at random, and neither `TrameState::new()` nor
    `ToileState::new()` exposes a seam to fix it -- adding one is a **public API change in both engines**
    (§7). Not merely an encoding concern: Trame builds sheet ids from the peer id
    (`format!("{:016x}_{:08x}", doc.peer_id(), counter)`, `ambre-trame/src/logic/sheets.rs`) and `ToileState`
    caches `peer` at construction, so an unpinned peer changes the document's **content**, not only its bytes.
  * **Commit timestamps.** Loro records one per commit; the generator pins the clock source.
  * **`document_id`.** `export_ambre_plaintext` takes a UUID v7 -- time-ordered and partly random. A
    generator derives it from the seed rather than minting one.

  A **semantic** digest over the decoded state is *not* an escape hatch for the first of those: peer-derived
  ids are part of the decoded state, so an unpinned `PeerID` defeats it exactly as it defeats a byte digest.
  Never silently ship a non-deterministic golden.

  **Measured, not assumed** (Loro 1.13.6, host x86_64). With `set_peer_id` and `set_record_timestamp(false)`
  -- both present in the API -- a document of maps, lists and text exports **byte-identically across separate
  processes**, and so do `TrameState` and `ToileState` driven through `handle_command`. The cross-process
  half is the one that matters: Rust reseeds `HashMap` state per process, so an encoder iterating a hash map
  would be stable within a run and unstable between runs. It is not.

  **Why the seam opens the phase.** Without it, two builds in the *same process* already differ, for the
  plainest of reasons: `LoroDoc::new()` mints a fresh random `PeerID` per document, so identical command
  sequences produce ops attributed to different peers. Nothing subtler is involved -- no map ordering, no
  clock. The divergence then shows up three ways: every op id is `(peer, counter)`; Trame's sheet ids carry
  the peer into the **content**, which is what rules out a semantic digest as an escape hatch; and Loro
  varint-encodes the id, so even the snapshot *length* tracks its magnitude (measured: 12 286 bytes at
  `PeerID` 1, 12 320 at `0xFFFF_FFFF`, same document).

  Untested and therefore still open: 100k-cell scale, the 2-peer merge path, and any architecture other than
  x86_64. A failure there reopens this invariant and makes a semantic digest the fallback contract.
* **I2 — One document model.** Generators drive `handle_command` on the app state, exactly as the UI does.
  Writing CRDT containers directly is forbidden: it would produce documents no user action can produce, and
  fixtures would stop being representative of anything.
* **I3 — Synthetic only.** Every committed artifact traces to a generator run from a seed -- directly for
  `fixtures/current/`, by copy for `fixtures/frozen/` (§4), which no current generator can reproduce. Never
  from a real user document -- see §3.
* **I4 — Host only, and the boundary is by symbol, not by crate.** `ambre-logic` ships to wasm, so "no
  fixture code in the client binary" has to say what counts. Three symbols this spec introduces are
  **production** code and ship ungated: `DocState::import_snapshot`, `DocState::export_snapshot`, and
  `ToileState::import_snapshot`. The last is not optional -- `DocState::import_snapshot` dispatches to it for
  the Toile variant, so gating it would make an ordinary client build fail to compile, taking Toile's local
  restore with it. Everything else is **fixture code** and sits behind the `fixtures` feature (§5).

  **The two determinism seams are on the production side of that line**, and deliberately:
  `TrameState::with_peer` / `ToileState::with_peer` are engine constructors, documented in
  `trame-logic §2` / `toile-logic §2` rather than here, and the belt that asserts on them is the one
  §4 makes P0's Definition of Done. That belt has to run under a bare `cargo test --workspace` -- the
  CI `test-native` job passes no `--features` -- so gating the seams would put P0's own gate out of
  CI's reach. They cost the client nothing: `ambre-app` never calls them, and neither pulls in a
  fixture type.
* **I5 — No `unsafe`** in our code, per the Security pillar.

---

## 2. TECHNICAL DESIGN & STATE

### A. Data Modeling

**CRDT structure: N/A.** This component holds no CRDT state of its own. It serializes and deserializes the
existing app states; every collaborative structure belongs to `trame-logic §2` / `toile-logic §2`.

**Artifact: a `.ambre` file. This spec invents no container.** The native format -- header, payload,
`export_ambre_plaintext()` / `import_ambre_plaintext()` and the `migrate_ambre` path -- is normative in
**`ambre-spec-docfile.md`** and consumed here without restatement. A fixture is one of those files, and the
fixture loader **is** the production importer (§6 rejects the alternative).

**Sidecar manifest.** The `.ambre` header carries `doc_type` and `document_id` but no **provenance**, which
a drift gate needs. Each committed fixture therefore keeps a sidecar next to it, **JSON-encoded**
(`serde_json`, already a workspace dependency): the manifest exists because an opaque binary has no
reviewable diff, so a binary encoding would defeat its only purpose. It lets CI and a human reviewer tell
"regenerated, identical meaning" apart from "the schema drifted".

```rust
#[derive(Serialize, Deserialize, PartialEq)]
struct FixtureManifest {
    /// Generator identity, e.g. "trame/formula_dag_deep". Also the artifact
    /// **stem**: the whole filename in `current/`, the part before the `@` in
    /// `frozen/` (§2.B). `String` because `fixtures-check` reads manifests back
    /// off disk, and owned deserialization cannot borrow `'static`.
    generator: String,
    /// Bumped by the generator author when the produced document changes on
    /// purpose. Frozen into the artifact too: that is how §4 detects a frozen
    /// fixture and the current one are no longer comparable.
    generator_version: u32,
    /// Declared by the generator (`FixtureGen::seed`), and pins the `PeerID`
    /// and `document_id` (I1).
    seed: u64,
    doc_type: DocType,
    /// The document's two independent versions at write time -- current for a
    /// generated artifact, superseded for a frozen one (§4). Distinct from
    /// `generator_version`, which tracks authorial intent rather than format.
    /// Generators write `schema_version: 0` until `ARCH P8` creates the register,
    /// 0 being what an absent one reads as (`architecture §2.D`).
    ambre_version: u8,   // container, `docfile §2.A`
    schema_version: u32, // payload, `architecture §2.D`
    /// BLAKE2b over the **whole `.ambre` file, header included**, so a flipped
    /// `document_id` byte is caught by the write-once check of §4. `ambre-crypto`
    /// has no plain `hash()`; the call is `hash::chain_hash(&[], bytes)`, an empty
    /// parent proof degenerating to a keyless 32-byte digest. Hex, not 32 raw
    /// integers, which would defeat the readable diff this sidecar exists for.
    #[serde(with = "hex")]
    digest: [u8; 32],
    shape: FixtureShape,
}

/// Coarse size/complexity summary: when the bytes move, this says whether the
/// document grew, lost a sheet, or is unchanged in shape, and §4 checks it
/// against `shape_of` to catch a manifest committed out of sync with its
/// artifact. Not a selection index -- lookup is by name (§2.B).
#[derive(Serialize, Deserialize, PartialEq)]
struct FixtureShape {
    /// Number of distinct peers whose ops are present (1 for a solo document).
    peers: u8,
    body: ShapeBody,
}

/// Per-app rather than a union of nullable counters: the apps that join later
/// (`ambre-prose`, `ambre-murmure` -- `FIXTURES P2`) add a variant instead of
/// widening every existing row with a field that means nothing to them.
#[derive(Serialize, Deserialize, PartialEq)]
enum ShapeBody {
    Trame { cells: u32, sheets: u16, formulas: u32 },
    Toile { elements: u32, frames: u16, connectors: u32 },
}
```

**A committed fixture is the plaintext `.ambre`, not the user-facing one.** `export_ambre_plaintext()` emits
header + raw snapshot, and **`ambre-sync` applies zstd and XChaCha20-Poly1305 downstream**
(`prose-interop §2.B`). A user's exported `.ambre` therefore carries compression and encryption; a
committed fixture is the same file taken before that stage. Same format, same parser, two ends of one pipe.

Committing the pre-Sync form is also what `I1` requires: zstd output is not guaranteed stable across
`ruzstd` versions or a compression-level change, so committing compressed bytes would turn a dependency bump
into golden drift unrelated to the document. (Git also compresses blobs in the packfile, so the disk saving
would be near zero anyway. This is a *repository* argument; it does not apply to a user's local copy, where
compression is plainly right.)

**Feeding the db tier is the same fixture, not a second artifact.** Where a server-side test needs stored
bytes, the fixture takes the ordinary client path: `import_ambre_plaintext` strips the header -- the network
carries a **bare** snapshot, the server already holding `document_id` and `doc_type` as routing metadata
(`docfile §2.A`) -- then `seal_snapshot(payload, public_data, doc_key, signing_key)` compresses and seals it
into what a real client would upload. The setup therefore derives **two** secrets from the fixture seed, a
`DocumentKey` and a `SigningKey`, and synthesizes the `SnapshotPublicData` the AAD is built from -- copying
what the real client does today, `parent_snapshot_proof` and `parent_snapshot_update_clocks` both empty
(`ambre-sync/src/updates.rs`), the BLAKE2b parent chain being post-MVP. There is no parent to pick. Neither
`seal_snapshot` nor `open_snapshot` is wasm-gated (`ambre-sync/src/pipeline.rs`), so this runs on host
today. Those bytes are **derived at setup and never committed** (§3). Hand-rolled bytes would prove only
that the server serves *something*, not that it serves what a client produces.

### B. Interface Contract (API)

**The trait lives in `ambre-core`, and it has to.** Implementors sit in the app crates
(`ambre-trame/src/fixtures/`), so a schema change and its generator move in one commit (§6). Rust's orphan
rule then forbids defining the trait anywhere downstream of them: `ambre-logic` depends on the apps, never
the reverse. `ambre-core` is upstream of everything -- `ambre-bus` depends on it, both apps depend on
`ambre-bus` -- it already owns `DocType`, and it already carries `serde` and `hex`. The artifact types go
with it, so `ambre-logic` can read a manifest without either app in scope.

```rust
// ambre-core

/// One parametric document generator.
///
/// Method-based rather than all-associated-item: the driver iterates a
/// `&[&dyn FixtureGen]` registry, and a trait of bare consts and free functions
/// is not object-safe -- it would force a hand-maintained table of function
/// pointers, one more place to forget a generator.
trait FixtureGen {
    /// Stable identity: the manifest's `generator` and the artifact stem (§2.A).
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

    /// Build the document and return the plaintext `.ambre` bytes, i.e. the
    /// output of the app's `export_ambre_plaintext()`. Not a raw snapshot: the
    /// header makes the artifact self-describing and loadable through the
    /// production importer.
    ///
    /// Implemented by driving `handle_command` on the app state (I2). `seed()`
    /// pins the `PeerID`, the commit clock and the `document_id` (I1).
    ///
    /// Infallible: a generator drives commands it chose itself, so a rejected
    /// one is a bug, not a runtime condition. It **panics** rather than emit a
    /// half-built document -- a corpus that silently lost a merge is worse than
    /// a failed `just fixtures`.
    fn build(&self) -> Vec<u8>;
}
```

**Loading a fixture is the production import path**, in two hops:
`import_ambre_plaintext(bytes)` validates magic / version / `doc_type` and yields an `AmbreHeader`
-- `version`, `doc_type`, `document_id`, `payload` -- or a typed `DocFileError` (`docfile §2.B`); the payload
then goes into the state. That second hop is where a gap sits.

**Required normalization — the loader side.** `TrameState::import_snapshot` exists
(`ambre-trame/src/logic/sync.rs`); `ToileState` has **no** counterpart -- restore goes through
`apply_remote` -> `LoroDoc::import` (`ambre-toile/src/logic/state/mod.rs`). Both funnel into the same Loro
call, but the entry point is spelled differently per app, so a loader cannot address them uniformly. This
spec adds `ToileState::import_snapshot` and exposes one entry point per variant:

```rust
// ambre-logic -- production, ungated (I4)

impl DocState {
    /// Load a full Loro snapshot into a state, returning the display events the
    /// app emits.
    fn import_snapshot(&mut self, payload: &[u8]) -> Vec<AppEvent>;

    /// Lift the per-app `export_snapshot` to the enum. `LogicState`'s takes a
    /// `DocumentId` and needs a live registry entry, so it cannot serve the bare
    /// `DocState` the round-trip belt of §4 quantifies over.
    fn export_snapshot(&self) -> Vec<u8>;
}

// ambre-logic -- fixture code, behind the `fixtures` feature (I4, §5)

/// Decode a state into the coarse summary the manifest records. On the loader
/// side rather than on `FixtureGen`, so the manifest-integrity check of §4 has
/// something independent to compare against: a generator declaring its own
/// shape would assert only that it agrees with itself. `peers` comes from
/// `LoroDoc::oplog_vv()`, whose `VersionVector` derefs to a map keyed by
/// `PeerID` -- the codebase already calls it in both engines' sync paths.
fn shape_of(state: &DocState) -> FixtureShape;

/// Load a committed fixture by name -- the entry point every consumer uses.
/// Resolves `<corpus root>/current/<name>.ambre`, then: `read_ambre_plaintext`
/// for the header, `DocState::new(doc_type)` for the empty state, and the
/// **app's own** `import_ambre_plaintext` to fill it.
///
/// That last hop is the app importer, not `DocState::import_snapshot`, and the
/// difference is load-bearing: `import_snapshot` returns display events, and the
/// two engines disagree about how a refused payload appears in them -- Trame
/// returns none, Toile returns an error *event*. A loader reading rejection off
/// that vector would accept a corrupt Toile artifact and hand back an empty
/// board. The app importer returns a typed `DocFileError::PayloadRejected`
/// instead, which is carried out verbatim. `DocState::import_snapshot` stays
/// what it was introduced for: the enum-level restore path, and the half of the
/// round trip §4 quantifies over.
///
/// The corpus root is resolved from `CARGO_MANIFEST_DIR` walked up to the
/// workspace root, never from the process working directory: `cargo test` runs
/// a test binary with the CWD at its own package root, so a bare relative
/// `fixtures/` would resolve differently for every consumer crate.
fn load_fixture(name: &str) -> Result<(DocumentId, DocState), FixtureError>;

/// Failures of the corpus, distinct from failures of the container. `DocFileError`
/// (`docfile §2.B`) stays the authority on malformed bytes; this wraps it and adds
/// the two conditions only a corpus has.
enum FixtureError {
    /// No such artifact under the corpus root.
    NotFound { name: String },
    /// Unreadable corpus root, permissions, IO.
    Io(std::io::Error),
    /// The bytes are not a valid `.ambre`. Carried verbatim, never reinterpreted.
    Container(DocFileError),
    /// `DocState::new` returned `None` -- a `doc_type` whose engine does not exist
    /// yet (`Prose`, `Murmure`). Unreachable for any fixture this spec produces
    /// (§1.A excludes both) and present so the loader degrades cleanly when they
    /// arrive.
    NoEngine { doc_type: DocType },
}

/// The canonical semantic form of a document: every user-visible value, in a
/// deterministic order, as text. **Not** the CRDT -- two documents agreeing on
/// content still differ in op history, peer ids and commit timestamps, so no
/// byte or state comparison can express "same document by a different route".
/// §4's frozen belt compares this, and text means a failure prints a readable
/// diff rather than a length mismatch.
///
/// Two rules the comparison does not work without:
/// * **Formula cells carry both representations** -- the stored source and the
///   evaluated result (`trame-logic`'s `raw` and `text`). Dumping only the source
///   would let an evaluation regression pass unnoticed; dumping only the result
///   would hide a change in what is persisted. The failure mode §4 warns about is
///   exactly the silent one, so both go in.
/// * **Floats are formatted shortest-round-trip** (Rust's `{}` for `f64`), which
///   is deterministic for a given value. It is *not* a tolerance: two paths that
///   compute Toile geometry differently to the last bit will differ here, and
///   that is intended -- a migration is not supposed to move geometry at all.
fn canonical_dump(state: &DocState) -> String;
```

`DocState::new` returns `Option` -- `None` for `Prose` and `Murmure` -- which surfaces as
`FixtureError::NoEngine`. Unreachable for any fixture this spec produces (§1.A excludes both apps), and
there so the loader degrades cleanly when they arrive. `import_snapshot` is independently valuable: it makes
the export/import round trip assertable on **every** app, not just Trame.

**Three homes, one dependency grain.** Types in `ambre-core`; **generators** next to the app whose commands
they drive, needing only its `handle_command` and `export_ambre_plaintext`; **loader and driver** in
`ambre-logic`, the lowest crate that sees every variant. `DocState` becomes `pub` for this, which the
round-trip belt of §4 needs regardless.

**The driver, and why generation is a round trip.** `build` returns bytes; `shape_of` needs a decoded
state. So writing a manifest means importing what was just generated -- the driver does exactly that, and
the extra import is the point rather than a cost: it proves the artifact loads through the production
importer before it is ever committed. The driver, not the generator, writes the `.manifest`; a `FixtureGen`
has no method that could.

**Data flow.** No worker, no channel, no transport:

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
        |  the driver re-imports them to compute `shape_of`,
        |  which also proves they load before anything is committed
        v
   .ambre file + FixtureManifest  ->  disk
        |
        |  load_fixture(name):
        |    read_ambre_plaintext()        (the header, type-agnostic)
        |    DocState::new(doc_type)
        |    <app>::import_ambre_plaintext (production importer, typed payload
        |                                   rejection -- see load_fixture above)
        v
   a test or a benchmark, starting from a known state

   ...and, for the db tier only, that same payload -- the bare snapshot, no
   header, as the network carries it -- continues through the ordinary client
   pipeline (seal_snapshot: zstd + AEAD) to produce the bytes a real upload
   would carry. Derived at setup, never committed.
```

**On-disk layout.** One corpus root, `fixtures/`, holding two trees whose lifecycles differ (§4):

| Path | Written by | Regenerated | Committed |
|---|---|---|---|
| `fixtures/current/<name>.ambre` + `.manifest` | the driver, from a live generator | every run | yes, under the §5 cap |
| `fixtures/frozen/<name>@v<ambre>-s<schema>.ambre` + `.manifest` | `just fixtures-freeze`, once | **never** -- no generator can produce it (§4) | yes |
| `fixtures/bench/<name>.ambre` + `.manifest` | `just fixtures-bench-corpus` | on demand | **no**, git-ignored (§5) |

The `@v…-s…` suffix records the **outgoing** versions -- the ones the artifact was written at, before the
bump that froze it. Bench corpora carry manifests like everything else: they are not committed, but a bench
still needs to know the shape and seed of what it just generated.

Membership is positional: a frozen artifact is one that sits in `fixtures/frozen/`. No `frozen` flag in the
manifest, which could only ever contradict the path.

**Invocation.** Five `just` recipes, matching the existing local/CI parity convention (`justfile` header):
* `just fixtures` -- regenerate `fixtures/current/` in place, and **delete any artifact whose generator no
  longer exists**. Detecting an orphan is `fixtures-check`'s job; removing one is this recipe's, so the fix
  for a red gate is always "run `just fixtures`", never a manual `rm`. Committed artifacts only, which keeps
  it inside the ~30 s budget of §5.
* `just fixtures-check` -- regenerate `fixtures/current/` into a temporary directory and diff against the
  committed goldens; then, separately, load every `fixtures/frozen/` artifact and assert its migration (§4).
  Runs in the CI `test-native` job and fails on drift.
* `just fixtures-freeze <name>` -- copy **the named artifact** out of `fixtures/current/` into
  `fixtures/frozen/`, deriving the `@v…-s…` suffix from its manifest rather than from a human typing it.
  Takes a generator name, not a `doc_type`: several generators share a `doc_type` (`FIXTURES P1` alone plans
  four Trame ones), and which lineage gets migration coverage is a deliberate choice, not a lucky pick. Run
  during a version bump (§4); a recipe rather than a documented `cp` because the filename is load-bearing and
  the versions in it are what a tired committer gets wrong.
* `just fixtures-bench-corpus` -- produce the large, git-ignored scale corpora into `fixtures/bench/`.
* `just fixtures-bench` -- run the `criterion` benches, invoking `fixtures-bench-corpus` first if the corpus
  is absent. The two are split because generating is minutes and re-running benches against an existing
  corpus is not.

**Feature flags, explicitly.** All five pass `--features fixtures`; the two bench recipes pass
`--features fixtures,pdf`, because Toile's PDF writer is behind `ambre-toile`'s existing non-default `pdf`
feature (`#[cfg(feature = "pdf")]` on the export path) and the 50-slide bench of §4 cannot otherwise be
compiled in. A bare `cargo test` sees none of this, which is why `fixtures-check` cannot ride along inside
`just test-native`'s existing `cargo test --workspace` -- `test-native` gains an explicit
`just fixtures-check` line.

None of them touches the network, a database, or a browser.

---

## 3. SECURITY & CONFIDENTIALITY (ZERO-KNOWLEDGE)

**Boundary audit: no outbound flow exists.** Generators run in-process on host and write to the repository.
No socket is opened; generation derives no key and links `ambre-crypto` only for `hash::chain_hash` (the
manifest digest). The Blind Relay contract (`architecture §3`) is untouched because nothing here can reach
it.

**The one crypto path, and why it is not a weakening.** The db-tier sealed blob derives two secrets from the
fixture seed (§2.A): a `DocumentKey` to seal under and a `SigningKey` to sign with. Both are **test keys
protecting synthetic content** -- they guard nothing, the plaintext they seal is already committed in the
clear next to them, and the signing identity authenticates a document nobody owns. Neither is written to
disk or committed, and both live only for the duration of a test setup -- derived rather than stored
precisely because committing either would be a real weakening.

**The rule is absolute: a fixture is synthetic.** Committing a snapshot derived from a real document --
imported, exported, captured from a session, or copied out of a running client -- is **forbidden**: it would
place plaintext user content in a public AGPL repository, permanently, in git history. A fixture that is
interesting because it came from real data is exactly the fixture that must not be committed; write a
generator that reproduces its shape instead.

`fixtures-check` cannot detect a violation -- a real document and a generated one are both just Loro bytes.
It is therefore a **review rule**, belonging in the contribution checklist rather than in CI.

**Cryptography used:** BLAKE2b (`ambre_crypto::hash::chain_hash`) for the manifest digest. Nothing else;
this component introduces no primitive.

---

## 4. QUALITY & TESTABILITY

> Tiering, CI shape and the tested Definition of Done are normative in `ambre-spec-architecture.md §4`;
> realization state is tracked in `ambre-spec-testing.md`. Referenced, not redefined.

* **Test tiers.** **host** for everything this component produces and for every consumer of it.
  The derived sealed blobs (§2.A) additionally feed the **db** tier, where `ambre-server` tests seed S3 +
  Postgres and exercise the read path; those tests belong to `ambre-server` and follow the existing
  `cfg_attr(not(feature = "db"), ignore = ...)` convention. No **wasm** tier: the `fixtures` feature is off
  in every client build, so none of this code is in a wasm artifact to test (I4).

* **Unit Tests.**
  * **Determinism (I1):** every generator, built twice, yields byte-identical output. The other direction --
    that a generator actually *uses* its seed rather than hardcoding its content -- can no longer be written
    by rebuilding one generator with a different seed, since `seed()` is now a constant the generator owns
    (§2.B). It is written instead with a **test-only pair**: two `FixtureGen` impls sharing one build body
    and differing only in `seed()`, asserted to produce different bytes. Without that second half the first
    passes vacuously on a generator that ignores its seed entirely.
  * **Golden drift:** `fixtures-check` regenerates and diffs; drift fails CI. So do the two asymmetric
    cases -- a generator with no committed golden (added but never committed) and a golden with no generator
    (left behind by a deletion).
  * **Manifest integrity:** `digest` matches the whole `.ambre` file; `shape` matches `shape_of` recomputed
    from the decoded state, which catches a manifest committed out of sync with its artifact;
    `ambre_version` and `schema_version` match the versions the artifact itself carries.

    **What the gate cannot enforce.** `just fixtures` rewrites the goldens with no ceremony, so a developer
    can regenerate, commit, and see a green `fixtures-check`: the gate compares the corpus against the code,
    never against the corpus's own history. Nothing makes a refresh *impossible*; the manifest makes it
    **visible**, moving `digest` and usually `shape` in a committed JSON file read as plain text. Same
    posture as I3 (§3) -- a review rule with a legible artifact, not a machine check pretending to be one.
    `generator_version` is the author's declaration that the change was intended: no recipe reads it, and it
    exists for the reviewer looking at that diff.
  * **Round trip:** `export_snapshot` -> `import_snapshot` -> `export_snapshot` is stable, on **every**
    `DocState` variant -- which needs both halves lifted to the enum (§2.B), not just the new
    `ToileState::import_snapshot`.

    **"Stable" is a fixed point, not byte equality with the source**, and the distinction is not a
    weakening -- byte equality is false here for two reasons that are both correct behaviour.
    `DocState::new` commits its own scaffolding (Trame's implicit sheet, Toile's board metadata) under
    its own fresh peer *before* the import, so a restored document legitimately carries an op the
    source never had; and both engines materialize their derived Loro containers lazily, on the first
    read of a restored document, so the first re-export names containers the source snapshot did not.
    Neither is a lost round trip. What must hold -- and what a regression in an import path would
    break -- is that **once a document has been through the cycle, going through it again changes
    nothing**: import a document's own exported bytes back into the state that produced them, and the
    next export is identical.
  * **Loader rejection**, asserted on `import_ambre_plaintext` directly, not through `load_fixture`: the
    latter takes a corpus name and has no way to be handed byte soup. `load_fixture` gets its own, smaller
    belt -- missing name, unreadable root, and a `DocFileError` arriving intact as `FixtureError::Container`
    rather than flattened into a string. Every `.ambre` header failure is exercised -- truncated header, bad magic,
    `UnsupportedVersion` in both directions (older triggers `migrate_ambre`, newer is a clean unrecoverable
    error), `WrongDocType` re-dispatch -- plus byte soup, all without panicking. A fixture loader is a
    deserializer and gets a deserializer's robustness belt, in the spirit of the CSV fuzz cases already
    green under `trame-interop` -- and since it is the production importer, that belt covers shipping code.
  * **Migration, on both axes.** A document carries two independent versions and the corpus must exercise
    both:
    * **Container** -- an artifact written at `AMBRE_VERSION = N` still loads after the bump to `N+1`,
      through `migrate_ambre` (`docfile §2.B`).
    * **Payload schema** -- an artifact written at `schema_version = N` still loads after the app's
      `SCHEMA_VERSION` reaches `N+1`, through that app's migration chain, and the migrated state matches a
      document built natively at `N+1` (`architecture §2.D`, `ARCH P8`).

    **Neither artifact can be generated, which is what reconciles them with the drift gate.** Writers only
    ever emit `AMBRE_VERSION` (`docfile §2.B`) and `handle_command` builds at the current `SCHEMA_VERSION`,
    so once a version is superseded no generator can produce an artifact at the old one -- a frozen fixture
    is outside the generator set, not a golden we declined to refresh. Hence `fixtures/frozen/`, which
    `just fixtures` does not walk. Freezing is consequently a step in the **version-bump** checklist -- one
    small artifact per affected `doc_type`, copied out of `fixtures/current/` in the bump's own commit, the
    last moment the outgoing version can be written at all. The obligation is recorded against the version
    registry (`docfile §2.A`) and `ARCH P8`, where whoever performs a bump will meet it.

    **The belt on a frozen artifact is semantic, not a diff.** Three assertions, in strengthening order:
    its digest still matches its bytes (write-once, so any change is a bug); it still loads through the
    migration chain; and `canonical_dump` of the migrated state equals `canonical_dump` of the live artifact
    its manifest names in `fixtures/current/`. The third is the one with teeth, and §2.B says why it cannot
    be a state or byte comparison.

    **When a generator's content changes, that third assertion is retired, not repaired.** A
    `generator_version` bump means the live artifact deliberately no longer matches what was frozen, so the
    comparison would fail for a reason unrelated to migration. When the frozen manifest's
    `generator_version` differs from the current generator's, `fixtures-check` **keeps the first two
    assertions and skips the third**, reporting the artifact as degraded. That is a real loss of coverage
    and the honest one -- nothing can reconstruct what the old generator would have produced at the new
    schema. Re-freezing at the next bump restores it.

    This is why an always-current corpus proves nothing: the payload axis fails *silently* -- an unmigrated
    document imports cleanly and reads wrong -- and the frozen set is the only place that surfaces before a
    user hits it.

    **Retention: keep the newest three per `doc_type`**, drop the rest. `fixtures-check` loads and migrates
    every one of them, so an unbounded set walks into §5's 10 s budget one bump at a time. What the cap costs
    is coverage of documents older than three bumps -- an explicit statement of how far back migration is
    exercised, rather than a limit discovered when CI slows down. A frozen artifact whose generator was
    renamed or deleted has no counterpart to compare against; it degrades exactly as a `generator_version`
    mismatch does and is dropped at the next pass.

    "Newest" is the lexicographic order of `(ambre_version, schema_version)`, the pair the filename already
    carries. The two axes are independent, but a freeze only ever happens *at* a bump of one of them, and
    neither ever decreases -- so the pair is strictly increasing across freezes of a given `doc_type` and
    orders them totally. No timestamp is stored, and none is needed.
  * Indicative target: every predicate branch and every error path in the fixture code itself. The component
    is thin -- there is no excuse for an uncovered branch.

* **Interop export corpora.** The export half of interop is untestable at any realistic size for the same
  reason the benchmarks are: no rich document exists to export. A fixture is that source, and it unlocks
  three things owned elsewhere -- **fidelity round trips** (export to .xlsx / .csv / .svg, re-import, assert
  the surviving subset; what is *expected* to be lost belongs in the interop specs), the **export budgets**
  in the bench table below, and **golden-testable exporters**, the same document exported twice having to
  produce the same bytes. This spec guarantees only that the corpus is there and stable.

* **CRDT convergence.** **N/A -- this component holds no CRDT state**, and it neither replaces nor weakens
  the 2-peer property-tests in `convergence_tests.rs`, which remain the sole authority. One planned generator
  *emits* a post-convergence state (two simulated peers, interleaved ops); that is a fixture **produced by**
  the converged engine, not a proof of convergence, and must never be cited as one.

* **Mocks & Environment.** Generators are pure and need no double. The 2-peer generator reuses the existing
  `MockSyncHandle` seam (`ambre-trame/src/test_support.rs`, a plain `pub mod` so a feature-gated generator
  can reach it), which already replays the Sync relay step without a network. That seam is **Trame-only
  today**: `ambre-toile` has no counterpart, so the post-convergence fixture is a Trame document until one
  exists. Isolated, headless, no live network.

* **Benchmarks.** `criterion`. A **primary motivation**, not a follow-up: nearly every budget in the app
  specs is stated against a document size no document reaches, leaving them **aspirational rather than
  measured**. Each fixture below is sized to the number its owning spec already states -- no new target
  here, none restated:

  | Budget (owner) | Fixture the bench needs |
  |---|---|
  | 10k-cell DAG recomputation < 50 ms (`trame-logic §11`) | Deep formula DAG, 10k formula cells |
  | `LoroDoc` < 10 MB / 100k cells, DAG + cache < 50 MB / 100k formula cells (`trame-logic §11`) | 100k-cell sheet, dense and formula-heavy variants |
  | `doc.import` < 2 ms, 50-cell batch < 1 ms, `SheetIndex` rebuild 10k rows < 10 ms (`trame-logic §11`) | The same corpora, plus recorded delta batches |
  | `query_visible` @ N=50 000 < 1 ms, incremental `SpatialIndex::update` < 50 us (`toile-logic §11`) | 50k-element board |
  | Frame time at 2 000 visible elements < 16 ms (`toile-ui §11`) | Dense board with 2 000 elements inside one viewport |
  | Export .xlsx 10k cells < 1 s, CSV 10k lines < 100 ms (`trame-interop §7`) | 10k-cell sheet |
  | PDF export 50 slides < 3 s (`toile-interop §8`) | 50-frame presentation sequence |

  **Who runs them.** No CI job: the corpora are git-ignored and minutes to build (§5). Benches are **local
  and on demand**, behind `just fixtures-bench`, which generates the corpus if absent and runs `criterion`
  over it. The author of a performance-relevant change produces the numbers and puts them in the PR.

  **A baseline is machine-relative, so the criterion is too.** `criterion` measures wall clock, so a number
  committed from one machine means nothing on another. What is committed is a baseline **plus the machine it
  came from**, and the 20% rule compares runs **on that same machine** -- in practice the author's own
  before/after. Treating a committed baseline as an absolute target would fail on hardware rather than code.

* **Document load path.** Opening a large document is the one user-visible operation with no end-to-end
  target; `architecture §5` now tabulates its five legs and owns the missing figure. This spec contributes
  the ability to measure it: legs 2 to 5 are pure functions of a snapshot, so a fixture makes them benchable
  on host with no network and no browser (`FIXTURES P2`), and leg 1 becomes a db-tier bench once a sealed
  blob (§2.A) seeds the server's storage.

  **How far that reaches, precisely.** A db-tier test seeds storage and exercises the real read path --
  `GET /snapshot`, `POST /snapshot/download-url`, the `RequestMissing` replay with its `covered_seq` gating
  and `join_all` fan-out (`server §2.B`) -- asserting the bytes return byte-identical and open through
  `open_snapshot`. The server never parses a document; it is a blind relay over opaque blobs
  (`architecture §3`). Combined with the host legs that covers the chain **except one link**: a real client
  speaking WebSocket to a running server, which needs a native transport for `ambre-sync` (browser-bound
  today: `gloo-net` + `web_sys::WebSocket`, no trait seam) and is **out of scope here**. Named so it is not
  mistaken for coverage.

* **What fixtures do NOT measure -- the GPU half of the frame.** The 60+ FPS / < 16 ms budget
  (`canvas §5.A`) is GPU-bound and stays validated visually, per `ambre-spec-testing.md §5.3`. A bench built
  on fixtures must never be reported as "60 FPS validated". What they **do** unlock is the **CPU half** --
  the half the pillars make a correctness claim about ("the render thread must never be blocked by
  computation"): content walk, culling, hit-testing, tessellation, text layout, batch build, all measurable
  on host, in CI with no GPU, through `RecordingSurface` (`ambre-draw/src/testing.rs`), the mock
  `DrawSurface` already backing `ambre-trame/src/render/tests.rs` and `ambre-toile/src/ui/render.rs`.

* **Definition of Done (tested).** Per architecture §4, no phase below ships `[x]` without its belt green in
  CI. Concretely: `FIXTURES P0` is not done until the determinism and round-trip tests are green, and
  `FIXTURES P1` is not done until `fixtures-check` runs in the CI `test-native` job.

  **`FIXTURES P2` is carved out, with a substitute.** Its benches run on git-ignored corpora no CI job
  produces, so "green in CI" is unreachable by construction rather than by neglect: P2 ships `[x]` when
  `just fixtures-bench` runs clean on a developer machine and its baselines are committed with their machine
  identity. Naming the substitute is the point -- a phase silently exempt is how a DoD stops meaning anything.

---

## 5. PERFORMANCE & RESOURCES

* **Performance Budget.** No hot path: nothing here runs during a frame or on a network round trip. The
  binding budgets are ergonomic and scoped to the **committed** corpus -- `just fixtures` under ~30 s,
  `just fixtures-check` under ~10 s in CI. A slow corpus is a corpus developers stop regenerating. The scale
  corpora are minutes rather than seconds and sit outside this budget, hence their own recipe (§2.B). The
  10 s figure holds only because §4 caps the frozen set at three per `doc_type`, every one of which that
  check loads and migrates.

* **Repository size.** Goldens are committed artifacts and git keeps them forever. Rule: **generators are
  the deliverable, goldens are a sample.** Commit only the small ones (indicative cap: 256 KB per artifact,
  ~5 MB total across `fixtures/current/` and `fixtures/frozen/`, covering every correctness fixture, the
  **small** interop export sources of `FIXTURES P1`, and the frozen migration set). The large corpora --
  the 100k-cell sheet, the 50k-element board, and the budget-sized export sources of `FIXTURES P2` -- exist
  to feed benchmarks, are produced on demand into `fixtures/bench/`, and are **git-ignored**; that path
  needs a `.gitignore` entry, added with the recipe.

* **WASM Impact: zero.** Host-only. Everything I4 calls fixture code sits behind a non-default `fixtures`
  feature, declared in `ambre-core`, `ambre-trame`, `ambre-toile` and `ambre-logic` **and wired to
  propagate** -- each downstream crate declares `fixtures = ["ambre-core/fixtures"]` (and `ambre-logic`
  additionally `"ambre-trame/fixtures", "ambre-toile/fixtures"`), because a Cargo feature never enables its
  dependency's feature implicitly and the gated code references `ambre_core::FixtureGen`. `ambre-app` never
  names it, so the only way it reaches a client build is a workspace-wide `--features fixtures`; the recipes
  of §2.B are therefore package-scoped rather than workspace-wide. None of it can be
  pulled into a client build by accident and none of it touches the 2 MB gzipped budget
  (`ambre-context.md §4`). This matters most in `ambre-logic`, which **does** ship to wasm: `shape_of`,
  `load_fixture`, `canonical_dump` and the driver are gated there, while `DocState::import_snapshot` and
  `export_snapshot` are ungated production code. **Not `#[cfg(test)]`:** such a module is reachable only
  from its own crate's test harness, whereas `just fixtures` invokes the generators from a binary.

---

## 6. REJECTED ARCHITECTURAL DECISIONS

| Rejected | Justification |
|---|---|
| **LLM-generated fixtures** | Fixtures must be deterministic, replayable, diffable and runnable in CI. Generated output is none of those: the same prompt yields a different document each run, so no golden can be committed and no drift gate can exist. Generators are code, seeded and versioned. |
| **Writing CRDT containers directly** (bypassing `handle_command`) | Faster to write, and it produces documents no user action can produce -- including states the engine's own invariants forbid. A fixture built that way tests the test, not the product. Violates I2. |
| **A textual fixture format (RON / JSON) instead of Loro snapshots** | Readable in a diff, and it would be a second, uncompiled copy of the Trame/Toile schema, drifting from the CRDT exactly as `ambre-spec-template.md §0` warns. The snapshot is the real persisted format; the `FixtureManifest` covers the reviewability need without a parallel schema. |
| **A bespoke fixture container instead of `.ambre`** | An earlier draft of this spec invented one. It would carry the same bytes as the native format through a second, test-only reader and writer -- unexercised by users, free to drift from the shipping importer, and a duplicated migration surface. Reusing `.ambre` instead means every test run exercises the production import path, and the fixture corpus becomes the regression belt for the format itself (§4). |
| **Committing the large benchmark corpora** | Multi-megabyte binaries in git history, forever, for artifacts fully reproducible from a seed. Generated on demand and git-ignored instead (§5). |
| **Capturing a fixture from a real document** | Plaintext user content in a public AGPL repository, irreversibly. See §3. |
| **One corpus tree, with a `frozen` flag in the manifest** | Would fix the mechanics of §4 while leaving the manifest free to disagree with reality. Two trees make membership positional and therefore unfalsifiable. |
| **A separate `ambre-fixtures` crate** | A generator depends on the engine's command surface and must change in the same commit as the schema it exercises. Housed next to each app (`ambre-trame/src/fixtures/`), that coupling is enforced by proximity; in a separate crate it drifts silently, which is the failure this spec exists to prevent elsewhere. |
| **Defining `FixtureGen` in `ambre-logic`, next to the loader** | The natural reading of "the machinery lives in `ambre-logic`", and impossible: the orphan rule requires an implementing crate to depend on the trait's home, and `ambre-trame` / `ambre-toile` sit *upstream* of `ambre-logic`. Types go to `ambre-core`, behaviour stays in `ambre-logic` (§2.B). |
| **Comparing a migrated frozen document against the current one by state or bytes** | Cannot work: two documents agreeing on every user-visible value still differ in op history, peer ids and commit timestamps -- the same fact I1 has to pin around. `canonical_dump` compares the content, and prints a readable diff when it fails (§4). |
| **Letting the drift gate enforce "no silent golden refresh"** | It cannot: the gate compares the corpus against the code, never against the corpus's own history, so any regenerate-and-commit passes. Pretending otherwise would put a machine check's name on a review rule. The manifest makes a refresh *visible* in a text diff instead (§4). |

---

## 7. INTEGRATION PLAN (ROADMAP)

> **Epic:** these phases are Stories of **`EF` -- Document corpora & native file format**
> (`ambre-roadmap.md`), shared with `ambre-spec-docfile.md`. `EF` is transverse and explicitly does not
> block `E4`.

> **Scheduling note.** This component is **unaffected** by the Trame per-cell style consolidation
> (`ambre-roadmap.md` E6 / `trame-logic §11 bis.F`), which gates work that would create third-party
> documents. Fixtures are regenerated from source rather than migrated: when the schema changes, the bump
> freezes one artifact per `doc_type` (§4) and `just fixtures` rewrites the rest, with the drift gate making
> the change visible. That argues for building them **before** the consolidation: the drift gate is
> precisely the tool that makes a schema migration reviewable.

### `FIXTURES P0` — Core  `[x]`
> **Gated on `DOCFILE P0`.** `FixtureGen::build` returns plaintext `.ambre` bytes (§2.B), so it cannot be
> written before `export_ambre_plaintext` exists; the loader-rejection belt below asserts on `DocFileError`
> variants that `DOCFILE P0` defines. The gate is real and one-directional -- `DOCFILE P0` depends on nothing
> here -- so within `EF` the container Story is built first.
>
> **Soft-gated on `ARCH P8`, not blocked by it.** The manifest records `schema_version`, whose register
> arrives with `ARCH P8`; until then generators write 0, which is what an absent register reads as
> (`architecture §2.D`). So P0 ships first and the field starts telling the truth when `ARCH P8` lands. What
> genuinely waits for it is §4's **payload** migration axis, not the container one.
>
> **`ARCH P8` writing its first register is itself a schema bump**, so it carries the freeze step and a full
> `just fixtures` in its own commit: every artifact starts carrying a real `schema_version` that day, and
> every committed manifest still says 0. Regenerating is the whole fix -- but it has to be *in* that commit,
> or the drift gate goes red across the entire corpus for a reason no one will connect to `ARCH P8`.

* `[x]` **Determinism seams in the engines (I1):** a constructor taking a pinned `PeerID` on `TrameState`
  and `ToileState`, plus a pinned commit clock. Leads the phase -- nothing else here is reproducible without
  it -- and carries the matching update to `trame-logic §2` / `toile-logic §2`, which own those constructors.
  *Done: `TrameState::with_peer` / `ToileState::with_peer`, both `set_peer_id` + `set_record_timestamp(false)`
  before the scaffolding commit, both documented in the engine specs, both ungated (I4).*
* `[x]` The `fixtures` feature in `ambre-core`, `ambre-trame`, `ambre-toile` and `ambre-logic`, plus
  `serde_json` on `ambre-logic` (§1.B). Everything below sits behind it except the two `DocState` methods.
  *Done, with two additions §1.B did not anticipate: `ambre-crypto` on `ambre-logic` (the manifest digest,
  which §1.B's own table already assigned to the driver) and `thiserror` (`FixtureError` wraps `DocFileError`
  rather than flattening it). Both optional, both pulled in by the feature only. `hex` gains its `serde`
  feature in `ambre-core`, without which `#[serde(with = "hex")]` does not resolve.*
* `[x]` `FixtureGen` (with `seed()` and `generator_version()`), `FixtureManifest` / `FixtureShape` and the
  hex-encoded JSON sidecar, **in `ambre-core`** (§2.B); the generator registry and the driver in
  `ambre-logic`. *Done. The registry is empty until `P1` writes the generators; the driver takes it as a
  parameter (`generate_into`) so the belt drives the same walk with a synthetic registry meanwhile -- the
  pattern `docfile §2.A`'s equally-empty migration registry already uses. `ambre-core` also gains
  `seeded_document_id(seed)`, which I1's third entropy source needs and §2.B named without siting.*
* `[x]` `ToileState::import_snapshot`; `DocState::import_snapshot` **and `export_snapshot`** lifted to the
  enum -- all three ungated (I4); `shape_of`, `canonical_dump`, `load_fixture` and `FixtureError` behind the
  feature; `DocState` made public (§2.B). *Done. `shape_of` / `canonical_dump` dispatch in `ambre-logic` over
  per-app bodies in the app crates, for §6's reason: the code that decodes a schema moves in the same commit
  as the schema. `DocState` also gains `doc_type()` (the inverse of `new`, which the driver writes into a
  manifest) and a `Debug` printing the variant only -- neither engine is `Debug`, and dumping a `LoroDoc`
  into a log line is not something the zero-knowledge posture allows.*
* `[x]` Determinism harness (I1) over the three entropy sources -- `PeerID`, commit clock, `document_id` --
  extended to the cases the spike left open: 100k-cell scale and the 2-peer merge path. Should either fail,
  the fallback to a semantic digest **also redefines what `fixtures-check` compares** (§2.B leaves that
  byte-diff as written until then). *Done, and **neither failed**: the byte diff stands and the semantic
  fallback stays unadopted. The harness also asserts the **cross-process** half, which is the one that
  matters (a hash-map-iterating encoder is stable within a run and unstable between runs), by re-execing its
  own test binary. Two deliberate narrowings: **scale is asserted at 5 000 cells, not 100 000** -- this belt
  runs on every `cargo test --workspace` and a 100k debug build is tens of seconds, exactly the "corpus
  developers stop regenerating" failure §5 warns about; the 100k case is `FIXTURES P2`'s benchmark corpus,
  built in release under `criterion`, and the invariant is reopened there if it breaks. And the 2-peer case
  exchanges snapshots directly rather than through `MockSyncHandle`, which `P1`'s post-convergence generator
  brings in.*
* `[x]` **Belt:** determinism -- rebuild-identical, plus the test-only generator pair that proves a seed is
  actually used (§4); export/import round trip on every `DocState` variant; manifest integrity; loader
  rejection on `import_ambre_plaintext`, and the smaller `load_fixture` belt beside it. *Done, split by gate:
  `ambre-logic/tests/fixture_determinism.rs` is **ungated** and runs inside `cargo test --workspace`, which
  is what makes P0's Definition of Done reachable in CI; `ambre-logic/tests/fixtures_corpus.rs` is gated and
  runs from the one `--features ambre-logic/fixtures` line `test-native` gains (replaced by
  `just fixtures-check` at `P1`). "Stable" above is now pinned to a **fixed point** rather than left to the
  reader; §4 carries the definition and the two reasons byte equality with the source is the wrong reading.
  That second `test-native` line makes the job build the workspace under two feature sets before the native
  server stack, which overflowed a hosted runner's disk on the first push; the job now frees the image's
  unused toolchains first, and prints `df` so the next squeeze is diagnosable rather than guessed at.*

### `FIXTURES P1` — Integration  `[ ]`
* `[ ]` **Correctness generators** (small, committed): deep formula DAG; cross-sheet references with merges
  and active filters (the `trame-logic §11 bis.E` shape); Toile board with connectors and nested groups;
  post-convergence 2-peer state via `MockSyncHandle` (Trame only -- Toile has no such seam, §4).
* `[ ]` **Interop export sources** (small, committed): a styled multi-sheet Trame exercising the .xlsx and
  .csv writers, and a multi-frame Toile presentation exercising the .svg and PDF writers. Consumed by the
  export fidelity round trips of §4.
* `[ ]` `just fixtures`, `just fixtures-check` and `just fixtures-freeze` over the `fixtures/current/` +
  `fixtures/frozen/` layout of §2.B, all passing `--features fixtures`; `test-native` gains an explicit
  `just fixtures-check` line, since `cargo test --workspace` cannot reach gated code.
* `[ ]` **`CONTRIBUTING.md`, which does not exist yet** (`README.md` links it as "to be written"). The
  synthetic-only rule of §3 and the freeze step of §4 both need a checklist to live in; creating that file
  is part of this phase rather than an assumed precondition.
* `[x]` Freeze step recorded against the `docfile §2.A` version registry and `ARCH P8` rather than here
  (§4). *Done: both carry it in normative language.*
* `[ ]` **Belt:** golden drift gate green in CI, including the new-generator and orphan-golden cases.

### `FIXTURES P2` — Extension  `[ ]`
* `[ ]` **Scale generators** (git-ignored, benchmark-only) behind `just fixtures-bench-corpus`, each sized to
  the budget it serves (§4): 10k-cell formula DAG; 100k-cell sheet (memory); 50k-element board;
  2 000-visible-element viewport; 10k-cell export source; 50-frame presentation.
* `[ ]` `just fixtures-bench` and `criterion` benches on those corpora, with baselines committed **alongside
  the machine that produced them** and the 20% same-machine regression criterion (§4). No CI job: P2's
  Definition of Done is the local substitute stated in §4.
* `[ ]` **CPU-frame bench** through `RecordingSurface`: per-frame content walk / culling / tessellation /
  layout cost on a dense fixture, on host, no GPU. Explicitly **not** a 60 FPS validation (§4).
* `[ ]` **Load-path bench** on legs 2 to 5 of `architecture §5`: AEAD open + decompress, full-snapshot
  `LoroDoc` import, engine rebuild (Trame reseed, Toile `SpatialIndex` bulk load), first-frame CPU cost --
  on the 100k-cell and 50k-element corpora. Feeds the end-to-end open budget that `architecture §5` is to
  set from those numbers.
* `[ ]` Generators for further apps as their engines land (`ambre-prose`, `ambre-murmure`), gated on
  `DocState::new` supporting them.
