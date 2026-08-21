# AMBRE -- SPEC: DOCUMENT FILE FORMAT (`.ambre`)
**Type:** Horizontal (Foundation -- Interop / Persistence)
**Status:** Draft
**Implementation:** partial

> The native, lossless, cross-app on-disk representation of an Ambre document. One container, every app.
>
> **Problem solved.** Every other export Ambre offers -- .xlsx, .csv, .docx, .svg, PDF -- is lossy by
> construction: formulas collapse to values, CRDT identity and history are dropped, presentation sequence
> and comments have no target. A user who wants a local copy of their own document, an offline backup, or a
> clean way to leave, currently has no lossless option. For a product whose pitch is that the server cannot
> read your data, "and you can only get it out through a lossy converter" is a weak ending.
>
> **Provenance.** The container was first defined in `ambre-spec-prose-interop.md section B`. It is cross-app -- it
> carries a `DocType` byte and its own `WrongDocType` path re-dispatches to another app's importer -- so
> defining it inside a vertical spec, for an app that is still a stub, put a foundation-level contract in the
> wrong place. This spec is now normative for the container; `prose-interop section B` references it and keeps only
> Prose's own concerns.

---

## 1. OBJECTIVES & CONSTRAINTS

### A. Scope

**Handles:** the container layout; the version registry and migration policy; `DocType` dispatch on read;
the at-rest protection policy (section 3); the error taxonomy; the round-trip invariants.

**Delegates:**
* Producing and consuming the payload -- each app's interop module owns `export_ambre_plaintext()` /
  `import_ambre_plaintext()` against its own `LoroDoc`.
* Compression and encryption -- `ambre-sync`, the sole compression/crypto consumer
  (`ambre-spec-architecture.md section 1`). The container layer carries none (I4).
* The CRDT schema inside the payload -- the app logic specs.

**Out of scope:** third-party format conversion (the per-app interop specs); the wire protocol
(`ambre-protocol`, `ambre-spec-sync.md`); server-side storage (`ambre-spec-server.md`).

### B. Dependencies

**No new dependency.** `ambre-core` for `DocType` and `Uuid`; each app's interop module for the payload;
`ambre-sync` for the downstream protection stage when one applies.

### C. Invariants

* **I1 -- Lossless.** A `.ambre` round trip is the identity on document state. Unlike every other export
  path there is no `ConversionReport` and no warning list, because there is nothing to lose: the payload
  **is** the document. A `.ambre` export that needed to warn about anything would be a bug in the container,
  not a fidelity trade-off.
* **I2 -- Self-describing.** `doc_type` and `version` precede any app-specific parsing. A reader identifies a
  file without guessing from its extension or content.
* **I3 -- Forward-compatible reads, non-destructive failures.** An older version migrates on read; a newer
  version is refused cleanly. The original bytes are never mutated in place, under any error.
* **I4 -- No cryptography in the container layer.** `export_ambre_plaintext` / `import_ambre_plaintext` are
  pure functions over bytes. Protection is a separate, explicit stage (section 3).
* **I5 -- One container, every app.** No per-app header variant. The only app-dependent value is the
  `doc_type` byte.

---

## 2. TECHNICAL DESIGN & STATE

### A. Data Modeling

**Container layout.** Fixed 22-byte header, then the payload.

| Offset | Size | Field | Notes |
|---|---|---|---|
| 0 | 4 | `magic` | `0x414D4252` (`"AMBR"`) |
| 4 | 1 | `version` | Container version. Current: `0x01`. |
| 5 | 1 | `doc_type` | `DocType as u8` -- `Prose` 0, `Trame` 1, `Toile` 2, `Murmure` 3 (`ambre-core::DocType`). |
| 6 | 16 | `document_id` | UUID v7. |
| 22 | rest | `payload` | `LoroDoc::export_snapshot()`, uncompressed and unencrypted at this layer. |

**Version registry.** One row per container version, appended never rewritten. A bump is a container change
only -- it does **not** track the CRDT schema inside the payload (see section 7 for why that distinction matters).

| Version | Status | Change |
|---|---|---|
| `0x01` | current | Initial layout. |

> **Adding a row obliges a freeze (normative).** Writers only ever emit the current version, so the moment a
> row is added is the **last** moment the outgoing version can be written at all. The same commit therefore
> runs `just fixtures-freeze <name>` for one **named** artifact per `doc_type` -- several generators share a
> `doc_type`, so which lineage keeps migration coverage is a choice, not a default -- moving it from
> `fixtures/current/` into `fixtures/frozen/`
> (`ambre-spec-fixtures.md section 4`). Skip it and the version that just left becomes untestable forever: no
> generator can produce it again, and `migrate_ambre` loses the only input that proves it works.

**Two stages, and they are not interchangeable.** The container is stage one; protection is stage two, and
which protection applies depends on **where the file is going** (section 3):

```
   app state
      |  export_ambre_plaintext()          Logic thread, no crypto (I4)
      v
   .ambre plaintext  = header || snapshot
      |
      +---> local file: written as-is, or passphrase-sealed on explicit opt-in (section 3)
      |
      +---> fixtures corpus: committed as-is (ambre-spec-fixtures.md section 2.A)
```

> **The network path does not use this container.** As built, `SyncCommand::SendSnapshot` carries a **bare**
> `export_snapshot()` payload with no header, which `ambre-sync` compresses and seals under the
> `DocumentKey`. That is correct and stays: the server already holds `document_id` and `doc_type` as routing
> metadata, so a header would duplicate them inside the ciphertext for nothing. `prose-interop section B` described
> the container as "the reference persistence **and sharing** format", conflating the two destinations; that
> conflation is the root of the at-rest question resolved in section 3. **`.ambre` is a file format, not a wire
> format.**

### B. Interface Contract (API)

Each app's interop module exposes the pair. The functions are per-app because the payload comes from that
app's `LoroDoc`, but the header handling is identical and must not be reimplemented per app -- it lives in
one shared helper in `ambre-core`, called by each app.

```rust
/// Header + uncompressed, unencrypted snapshot. Pure; no crypto (I4).
pub fn export_ambre_plaintext(doc: &LoroDoc, doc_id: Uuid) -> Vec<u8>;

/// Validates magic / version / doc_type, returns the payload.
/// The caller compares the returned `DocType` against its own and re-dispatches
/// on mismatch rather than coercing (see the dispatch rule below).
pub fn import_ambre_plaintext(data: &[u8]) -> Result<AmbreHeader, DocFileError>;

pub struct AmbreHeader {
    pub version: u8,
    pub doc_type: DocType,
    pub document_id: Uuid,
    /// Offset 22 onward: the raw Loro snapshot.
    pub payload: Vec<u8>,
}

/// Forward migration, one step per version bump. Added when a version is added.
pub fn migrate_ambre(from: u8, bytes: &[u8]) -> Result<Vec<u8>, DocFileError>;
```

**The shared helper (`ambre_core::docfile`, as built).** Each app's pair above is a two-line binding over
these; no app re-derives the layout or either read rule (I5).

```rust
/// The writer rule: no version parameter, because there is no write mode to pick.
pub fn write_ambre_plaintext(doc_type: DocType, document_id: Uuid, payload: &[u8]) -> Vec<u8>;

/// Any `DocType`, version-recovery rule applied. What a file-open router reads
/// before it knows which app to hand the bytes to.
pub fn read_ambre_plaintext(data: &[u8]) -> Result<AmbreHeader, DocFileError>;

/// The same, plus the dispatch rule against the caller's own type. The whole body
/// of each app's `import_ambre_plaintext`.
pub fn read_ambre_plaintext_as(expected: DocType, data: &[u8]) -> Result<AmbreHeader, DocFileError>;

/// The strict layout parse: current version only, no recovery. Used by the two
/// above and by anything reading raw bytes (a header dump, a migration step
/// checking its own output).
pub fn parse_header(data: &[u8]) -> Result<AmbreHeader, DocFileError>;
```

An app owns the payload half only: its `export_ambre_plaintext` is `write_ambre_plaintext` over its own
`export_snapshot()`, its `import_ambre_plaintext` is `read_ambre_plaintext_as` for its own `DocType`.
`PayloadRejected` is raised there and nowhere else -- the container layer never touches a `LoroDoc` (I4).
Each engine also carries the state-level pair (`TrameState` / `ToileState`: `export_ambre_plaintext(doc_id)`,
and `import_ambre_plaintext(bytes)` returning the file's `document_id` with the display events of the
reload), so a caller holding a live document does not reach for its `LoroDoc` to save it.

**Error taxonomy.** Fatal by nature: a header failure means no document can be produced.

```rust
pub enum DocFileError {
    TruncatedHeader,
    InvalidMagic,
    /// `found < AMBRE_VERSION` -> recoverable via `migrate_ambre`.
    /// `found > AMBRE_VERSION` -> unrecoverable, prompt the user to update.
    UnsupportedVersion { found: u8 },
    /// The `doc_type` byte names no app this build knows -- a file from a client
    /// that shipped a new document type, or corruption. Distinct from
    /// `WrongDocType`, which is a *valid* type belonging to another app and is
    /// therefore re-dispatchable; this one cannot carry a `DocType` at all.
    UnknownDocType { found: u8 },
    /// A valid `.ambre` for a different app. Carries the type so the caller can
    /// re-dispatch instead of failing the user.
    WrongDocType { found: DocType },
    /// The header parsed but the Loro payload did not import.
    PayloadRejected,
}
```

**Dispatch rule (normative).** A reader never coerces a document across apps. On `WrongDocType`, the caller
routes the same bytes to the importer named by `found`; only if no app claims that `DocType` does it surface
an error. This is what makes "open a `.ambre`" a single user action rather than four.

**Version-recovery rule (normative).** `found < AMBRE_VERSION`: apply `migrate_ambre` steps in order, then
retry the import. `found > AMBRE_VERSION`: refuse, leave the file untouched, and tell the user to update the
client -- never a partial or best-effort read.

**Writer rule (normative).** Writers always emit `AMBRE_VERSION`, the current one. There is no
backward-compatible write mode and no version negotiation: compatibility is a **read-side** obligation. One
writer path, N reader paths, is the only combination that stays testable as versions accumulate.

---

## 3. SECURITY & CONFIDENTIALITY (ZERO-KNOWLEDGE)

**Boundary audit.** The container layer opens no socket and touches no key (I4). Nothing in this spec can
reach the Blind Relay; the network path is a different payload entirely (section 2.A).

### At-rest protection of a local file -- the decision

`prose-interop section B` said the native format is "encrypted XChaCha20-Poly1305 by `ambre-sync`" **without
naming the key**. Read the obvious way -- the `DocumentKey`, the key Sync already holds for that document --
that is a trap, and it is the reason this section exists.

| Option | Verdict |
|---|---|
| **Sealed under the `DocumentKey`** | **Rejected.** The `DocumentKey` is rotated server-side on every revocation (`RevokeFromDocument`, `ambre-spec-sync.md` sharing flow). A file sealed under a since-rotated key is permanently unreadable -- and it fails *silently and late*: the user discovers it months after the export, when the backup is the only copy left. A local backup whose validity depends on future membership changes to a server-side document is not a backup. |
| **Plaintext** | **Accepted as the default.** It is what every other export already does: an .xlsx, a .csv, a PDF exported from Ambre lands on the user's disk in the clear today. The Zero-Knowledge pillar is a statement about the **server**, not about the user's own filesystem. Consistency matters more than a partial gesture here. |
| **Passphrase-sealed** (Argon2id -> XChaCha20-Poly1305) | **Accepted as an explicit opt-in.** The right answer for a backup that leaves the machine. Its key is derived from a passphrase the user types at export time, so it is independent of the `DocumentKey` lifecycle and immune to the rotation hazard. |

**Decision: plaintext by default, passphrase-sealed on explicit opt-in, never the `DocumentKey`.**

Two obligations follow, and neither is optional:
* The export UI states plainly that a plaintext `.ambre` is readable by anything with access to the file. It
  must not imply that a file inherits the document's end-to-end encryption, because it does not.
* The passphrase-sealed variant is a **container version bump**, not a flag on `0x01`: a reader must be able
  to tell a sealed file from a plaintext one from the header alone, before attempting to parse a payload.
  Its salt and Argon2id parameters live in the header, so the file stays self-describing (I2) and the
  parameters can be strengthened later without breaking old files.

**Cryptography used.** None at version `0x01`. The passphrase-sealed variant reuses `ambre-crypto`
(Argon2id, XChaCha20-Poly1305) and introduces no primitive.

---

## 4. QUALITY & TESTABILITY

> Tiering, CI shape and the tested Definition of Done are normative in `ambre-spec-architecture.md section 4`.
> Referenced, not redefined.

* **Test tiers.** **host** only. Pure byte handling; no `cfg(target_arch = "wasm32")` surface, no database.

* **Unit Tests.**
  * **Round-trip identity (I1):** export -> import -> export is byte-identical, and the reloaded state is
    equal to the original, on every `DocType` with an engine.
  * **Header rejection:** truncated header (at every length below 22), bad magic, `UnsupportedVersion` in
    both directions, an unknown `doc_type` byte, `WrongDocType`, and a valid header over a corrupt payload
    -- each producing the right typed error and **never** a panic.
  * **Dispatch:** a Toile file handed to the Trame importer yields `WrongDocType { found: Toile }` and is
    accepted by the Toile importer unchanged.
  * **Non-destructive failure (I3):** after every error path, the input buffer is unmodified.
  * **Migration:** once `0x02` exists, a `0x01` file still loads through `migrate_ambre`. Until then, the
    harness exists and is exercised with a synthetic step so the path is not dead code on the day it is
    needed.

* **The regression belt for this format is the fixture corpus.** `ambre-spec-fixtures.md` commits `.ambre`
  files and gates drift in CI, so every fixture run exercises this reader and writer, and a set of
  deliberately stale fixtures keeps the migration path honest. That is a deliberate dependency in both
  directions: this spec owns the container, the fixtures spec owns the corpus that proves it stable.

* **CRDT convergence.** N/A -- the container holds no CRDT state, it transports one.

* **Benchmarks.** Export and import cost is dominated by the Loro snapshot, not by the 22-byte header;
  measured through the load-path benches of `ambre-spec-fixtures.md section 4` rather than duplicated here.

* **Definition of Done (tested).** Per architecture section 4, no phase in section 7 ships `[x]` without its belt green in
  CI.

---

## 5. PERFORMANCE & RESOURCES

* **Performance Budget.** The container adds a 22-byte write and a 22-byte parse. It carries **no** budget
  of its own: export is `export_snapshot()` plus a header, import is a header parse plus `import_snapshot`,
  and both of those legs are budgeted where they belong (`ambre-spec-fixtures.md section 4`, load path legs 3 and
  4). Adding a target here would duplicate one.

* **WASM Impact.** Negligible: header handling is a few dozen bytes of code in `ambre-core`, shared by every
  app rather than duplicated per app. The compression and crypto of the passphrase-sealed variant are
  already linked in `ambre-sync` / `ambre-crypto` and counted there.

---

## 6. REJECTED ARCHITECTURAL DECISIONS

| Rejected | Justification |
|---|---|
| **Sealing a local file under the `DocumentKey`** | A revocation-triggered key rotation makes the file permanently unreadable, silently and long after export. Detailed in section 3. |
| **Using `.ambre` as the wire/snapshot format too** | The server already holds `document_id` and `doc_type` as routing metadata; putting them inside the ciphertext duplicates them and buys nothing. Conflating the two destinations is what produced the at-rest ambiguity in the first place (section 2.A). |
| **A per-app container variant** | Four headers to keep in step, and "open this file" stops being one user action. The `doc_type` byte plus the dispatch rule cover the whole need. |
| **A zip / OPC-style container** (OOXML's approach) | Buys multi-part packaging Ambre does not need -- the payload is one opaque CRDT blob -- at the cost of a zip dependency in the Logic thread and a much larger attack surface on untrusted input. |
| **A text-based container (JSON/RON wrapper around base64)** | Inflates every file, and readability is illusory: the payload stays opaque binary. The header is the only part worth reading, and a 22-byte layout is trivially inspectable with any hex viewer. |
| **Carrying the document name in the header** | The name is E2EE metadata (`name_enc`, `architecture section 3`), and the filename already serves the purpose. Putting it in the header would leak it into a plaintext file whose contents the user may have chosen not to expose. |
| **Version negotiation / backward-compatible writes** | Writers emit the current version only; compatibility is a read-side obligation. One writer and N readers stays testable as versions accumulate; N writers does not. |

---

## 7. INTEGRATION PLAN (ROADMAP)

> **Epic:** these phases are Stories of **`EF` -- Document corpora & native file format**
> (`ambre-roadmap.md`), shared with `ambre-spec-fixtures.md`. `EF` is transverse and explicitly does not
> block `E4`.

> **Sequencing constraint (binding).** The *user-facing* export must not ship before the Trame per-cell
> style consolidation (`ambre-roadmap.md` E6 / `trame-logic section 11 bis.F`) -- **discharged**: that
> Story has landed. It changed the schema while **no third-party document existed**; a file on a
> user's disk is a stronger and more permanent commitment than a server-side document, because it is
> beyond our reach to migrate. The same argument that defers other third-party-document work applies
> here with more force, and it applies to `P1`, not to `P0`.
>
> **Two independent versions, both checked.** `AMBRE_VERSION` versions the **container** -- the 22-byte
> header. It does **not** version the CRDT schema inside the payload, and `migrate_ambre` does not address
> it: a schema change like the E6 style consolidation leaves the header at `0x01`, so a file written before
> it passes every check in this spec and then imports into an engine reading containers that no longer mean
> what it expects. That is a *successful open of a wrong document*, which is worse than a rejection.
>
> The payload half is solved separately, by the `meta` / `schema_version` register specified in
> `ambre-spec-architecture.md section 2.D` and built under `ARCH P8`. Both checks run, in order: the header first
> (is this an `.ambre` this client can parse?), then the schema (is this a document this client understands?).
> They are deliberately the same shape -- equal proceeds, older migrates, newer refuses non-destructively --
> so there is one rule to learn.
>
> **`ARCH P8` is a hard prerequisite of `DOCFILE P1`.** Container versioning alone would ship files whose
> payloads have no migration story, and a file on a user's disk has unbounded age.

### `DOCFILE P0` -- Container  `[x]`
* `[x]` Shared header helper in `ambre-core`: layout, `AmbreHeader`, `DocFileError`, writer rule
  (`ambre-core/src/docfile.rs`).
* `[x]` `migrate_ambre` harness with the version registry, exercised via a synthetic step. The registry is
  empty at `0x01` -- nothing precedes the initial layout -- so the walk it drives is proven by a synthetic
  two-row registry: adding a real row is then the whole change.
* `[x]` `export_ambre_plaintext` / `import_ambre_plaintext` wired for Trame and Toile (Prose and Murmure
  join with their engines).
* `[x]` **Belt:** round-trip identity, full header rejection set, dispatch, non-destructive failure. The
  cross-app dispatch case lives in `ambre-logic` (`tests/docfile_dispatch.rs`), the first crate that links
  both engines, with a real file from each app rather than a hand-assembled header.

### `DOCFILE P1` -- User-facing export/import  `[ ]`
* `[ ]` Chrome: export to file (download) and import from file (picker), app-agnostic, routed by `doc_type`.
* `[ ]` Plaintext-at-rest statement in the export UI (section 3).
* `[ ]` **Belt:** the fixtures corpus drift gate (`ambre-spec-fixtures.md`) covering this reader/writer.
* **Gated on** `ARCH P8` (persisted schema versioning) and on the E6 style consolidation, per the constraints
  above. The two gates are distinct: `ARCH P8` gives payload migration a mechanism, E6 makes the one schema
  change we already know about land while it is still free.

### `DOCFILE P2` -- Passphrase-sealed variant  `[ ]`
* `[ ]` Container version bump carrying salt + Argon2id parameters in the header.
* `[ ]` Export-time passphrase prompt; import-time unlock with a typed wrong-passphrase error.
* `[ ]` **Belt:** seal/open round trip, wrong passphrase, parameter upgrade on an old file.
