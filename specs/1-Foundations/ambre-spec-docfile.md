# AMBRE -- FEATURE: THE NATIVE FILE (`.ambre`)

**Type:** Horizontal (framework)
**Implementation:** partial -- the container ships and two gestures reach it: a save in each
application, and one open in the launcher. The sealed variant is a design.

> The file a user saves a document into, and opens back. One container for every application: a
> spreadsheet, a board and a text document differ by one byte of the header.

Every other export drops something. A formula collapses to its value, a comment loses its target,
identity and history leave. This file drops nothing, so it is the local backup and the way out.
What travels inside belongs to each application's import and export feature; the container is here,
and no application respecifies it.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|---|
| `DOCFILE-001` | `IO` | A `.ambre` round trip MUST be the identity on document state, and MUST NOT need a warning list: the payload **is** the document. | this spec section 3 | `ambre_core::docfile::tests::round_trip_is_the_identity_on_the_container`; `ambre_trame::logic::docfile::tests::round_trip_preserves_the_document_state`, `::round_trip_is_byte_identical_on_a_bare_document`; `ambre_toile::logic::state::docfile::tests::round_trip_preserves_the_document_state`, `::round_trip_is_byte_identical_on_a_bare_document`; `fixture_determinism::the_round_trip_is_stable_on_every_doc_state_variant` | verified |
| `DOCFILE-006` | `INV` | The container layer MUST NOT compress or encrypt: the two entry points are pure functions over bytes, and protection is a separate stage. | this spec section 5 | crate graph: `ambre-core` declares no crypto and no compression dependency, so neither is reachable from the container | verified |
| `DOCFILE-007` | `INV` | There MUST be one container for every app: the only app-dependent value in the header is the `doc_type` byte. | this spec section 3 | `ambre_core::docfile::tests::header_layout_is_the_spec_layout` pins the 22-byte layout. One writer takes the type as a parameter, so a per-app variant has nowhere to be written | verified |
| `DOCFILE-011` | `INV` | A snapshot on the wire MUST NOT carry this header: the container is a file format, never a wire format. | this spec section 5 | none. `ambre-sync` names neither entry point and seals a bare `export_snapshot()`, so the rule holds today by construction. Nothing refuses the line that would change it, and no test asserts the magic bytes are absent from a sealed payload | unverified |
| `DOCFILE-008` | `INV` | A version bump MUST freeze one named artifact per `doc_type` in the same commit that adds the registry row. | this spec section 3; `ambre-spec-fixtures.md` | not built -- lands with `FIXTURES P1 -- the corpus and its gate` (`ambre-roadmap.md` EF). The recipe that performs the freeze does not exist, and no check reads the registry against the frozen corpus | unimplemented |
| `DOCFILE-010` | `INV` | A passphrase-sealed file MUST carry its own container version; it MUST NOT be a flag on an existing one. | this spec section 5 | not built -- lands with `DOCFILE P2 -- passphrase-sealed files` (`ambre-roadmap.md` EF) | unimplemented |
| `DOCFILE-009` | `OBS` | The export surface MUST state that a plaintext file carries no encryption of its own. | this spec section 5 | `ambre_toile::ui::commands::tests::the_save_label_says_the_file_is_not_encrypted`, `ambre_trame::ui::commands::tests::the_save_label_says_the_file_is_not_encrypted`. Asserted on the command's **label** -- the one string a user reads before clicking -- rather than on a tooltip or a dialog, so a surface that drops the warning cannot compile green. One test per surface, because the rule is per surface and the suite has two | verified |

### 1.2 Read a `.ambre` file

The reader is handed bytes and nothing else. It reads the header, decides what wrote the file, and
migrates or refuses before an application ever sees a payload.

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|---|
| `DOCFILE-002` | `POST` | A reader MUST identify the file from `doc_type` and `version`, before any app-specific parsing. | this spec section 3 | `ambre_core::docfile::tests::header_layout_is_the_spec_layout`, `::dispatch_reports_the_type_that_wrote_the_file`, `::an_unknown_doc_type_byte_is_not_a_dispatch_target`; `ambre_trame::logic::docfile::tests::a_toile_file_is_re_dispatched_not_coerced` | verified |
| `DOCFILE-003` | `POST` | A version older than the binary's MUST migrate on read. | this spec section 3 | `ambre_core::docfile::tests::the_walk_applies_one_step_per_version`, `::the_walk_chains_steps_in_order`, `::a_gap_in_the_registry_stops_the_walk_at_the_missing_row`, `::a_failing_step_fails_the_whole_migration` | verified |
| `DOCFILE-012` | `FRAME` | A reader MUST NOT consult the file's name or extension. | this spec section 3 | type-level: both entry points take a byte slice, so there is no path, no name and no extension in scope to branch on | verified |
| `DOCFILE-004` | `FRAME` | No error path MUST mutate the bytes it was handed. | this spec section 3 | `ambre_core::docfile::tests::every_error_path_leaves_the_input_untouched`; `ambre_trame::logic::docfile::tests::a_rejected_file_leaves_both_the_input_and_the_state_alone` | verified |
| `DOCFILE-013` | `FAIL` | A version newer than the binary's MUST be refused cleanly. | this spec section 3 | `ambre_core::docfile::tests::a_newer_version_is_refused_and_an_older_one_is_attempted`, `::migrating_from_a_newer_version_is_refused` | verified |
| `DOCFILE-005` | `FAIL` | A malformed file MUST produce a typed error, never a panic. | this spec section 3 | `ambre_core::docfile::tests::truncated_header_is_rejected_at_every_length`, `::bad_magic_is_rejected`; `ambre_trame::logic::docfile::tests::a_corrupt_payload_under_a_valid_header_is_typed_not_a_panic` | verified |

* **Budget.** none, and `ambre-spec-budgets.md` records that as a result: the
  container adds a 22-byte write and a 22-byte parse, and its binary contribution is a few dozen
  bytes in `ambre-core`, shared rather than duplicated per application.
* **Boundary.** none: `DOCFILE-006` bans compression and encryption in this layer, so it holds no
  key, and `DOCFILE-011` keeps the bytes it produces on a local disk rather than on the relay.
* **Verification.** 10 `verified`, 1 `unverified`, 2 `unimplemented`. The container is the one
  Foundations subsystem where the first count is honest: it does pure byte handling on the host
  tier, so a test can reach every rule it makes about bytes. The three gaps that remain are the
  rules that reach outside it -- a bump obligation on a recipe that is unbuilt, the sealed variant,
  and one about what another crate must refrain from putting on the wire. `DOCFILE P1` closed the
  fourth: the export surface now exists, and its warning is held on the command label rather than
  on a reviewer's memory. What the gestures add on top of the container is verified where it can
  be: the save's header identity and its round trip through the launcher's own open path are driven
  by `ambre_logic::dispatch::tests::a_saved_file_carries_the_document_id_the_registry_keyed_it_under`
  and `::a_saved_file_opens_back_as_the_document_that_was_saved`, and the two command mappings by a
  test each. The file picker and the download anchor stay DOM glue, held by using the application
  (`ambre-spec-testing.md section 2`). The fixture
  corpus (`ambre-spec-fixtures.md`) widens the belt further, because it exercises this reader and
  this writer on every run. The pass split `DOCFILE-003`, which cited six tests for a migration and
  a refusal: four drive the walk and two drive the refusal, and one row could not say which. It also
  gave `DOCFILE-002`'s second clause its own row: not consulting the extension is a frame condition,
  and it is held by a stronger mechanism than the rest of the row -- the entry points take a byte
  slice, so no name is in scope to branch on.

---

## 2. WHAT EACH USER SEES

Two users and one file, not one document: nothing here is shared state, and that is `DOCFILE-011`.
Alice writes the file and hands it over; the third column is what the bytes carry.

| Alice does | Bob does | The file holds | Each user sees | Test |
|---|---|---|---|---|
| saves a workbook | opens it | a Trame payload under the Trame type byte | the same cells, formulas and styles | `ambre_trame::logic::docfile::tests::round_trip_preserves_the_document_state` |
| saves a workbook | opens it with a board in front | the same file | the workbook, opened by the spreadsheet, never a coerced board | `ambre_toile::logic::state::docfile::tests::a_trame_file_is_re_dispatched_not_coerced` |
| saves from a newer client | opens it | a version byte this build does not know | a request to update the client, and the file untouched | `ambre_core::docfile::tests::a_newer_version_is_refused_and_an_older_one_is_attempted` |
| saves from an older client | opens it | a superseded version byte | the document, migrated on read | `ambre_core::docfile::tests::the_walk_applies_one_step_per_version` |
| saves from a client shipping a document type this build lacks | opens it | a type byte no application claims | one error, and no dispatch attempt | `ambre_core::docfile::tests::an_unknown_doc_type_byte_is_not_a_dispatch_target` |
| sends a truncated or corrupt file | opens it | bytes that parse to nothing | a typed error, and the document already open unchanged | `ambre_trame::logic::docfile::tests::a_rejected_file_leaves_both_the_input_and_the_state_alone` |
| mails a plaintext file | reads it without Ambre | the snapshot in the clear | every value of the document, to anyone holding the file | none -- `DOCFILE-009` |

---

## 3. MODEL

**CRDT containers.** none. The file carries a snapshot of the application's own containers and
declares none of its own; the word *container* below names the file layout.

**The layout.** A fixed 22-byte header, then the payload.

| Offset | Size | Field | Holds |
|---|---|---|---|
| 0 | 4 | `magic` | `0x414D4252`, that is `AMBR` |
| 4 | 1 | `version` | the container version. Current: `0x01` |
| 5 | 1 | `doc_type` | `DocType as u8` -- `Prose` 0, `Trame` 1, `Toile` 2, `Murmure` 3 |
| 6 | 16 | `document_id` | a UUID v7 |
| 22 | rest | `payload` | `LoroDoc::export_snapshot()`, uncompressed and unencrypted (`DOCFILE-006`) |

**The version registry.** One row per container version, appended and never rewritten. A bump is a
container change alone: it does not track the CRDT schema inside the payload, which each
application raises on its own (`ambre-spec-architecture.md section 2`).

| Version | Status | Change |
|---|---|---|
| `0x01` | current | the initial layout |

**Adding a row freezes an artifact, in the same commit** (`DOCFILE-008`). A writer emits the current
version only, so the commit that adds a row is the last moment the outgoing version can be written
at all. Three steps:

1. add the registry row and the migration step that reads the outgoing version;
2. run `just fixtures-freeze <name>` for one **named** artifact per `doc_type`. Several generators
   share a `doc_type`, so which lineage keeps migration coverage is a choice, not a default;
3. commit the frozen artifact with the bump (`ambre-spec-fixtures.md`).

Skip step 2 and the version that just left becomes untestable for good: no generator can produce it
again, and `migrate_ambre` loses the only input that proves it works.

**The three reader and writer rules**, all normative.

| Rule | What it says | Why it is this way |
|---|---|---|
| Dispatch | a reader never coerces a document across applications. On `WrongDocType` the caller routes the same bytes to the importer the error names, and surfaces an error only when no application claims that type | it makes "open a `.ambre`" one user action rather than four (`DOCFILE-002`) |
| Version recovery | below the current version, apply `migrate_ambre` in order and retry the import. Above it, refuse, leave the file untouched, and ask the user to update the client | a partial or best-effort read of a format this build does not know produces a document nobody can trust (`DOCFILE-003`) |
| Writer | a writer emits the current version only. There is no backward-compatible write mode and no negotiation: compatibility is a **read-side** duty | one writer and N readers stays testable as versions accumulate; N writers does not (`DOCFILE-007`) |

**Two stages, and they are not interchangeable.** The container is stage one; protection is stage
two, and which protection applies depends on where the file is going (section 5).

```
   app state
      |  export_ambre_plaintext()          Logic thread, no crypto (DOCFILE-006)
      v
   .ambre plaintext  = header || snapshot
      |
      +---> local file: written as-is, or passphrase-sealed on opt-in (section 5)
      |
      +---> fixtures corpus: committed as-is (ambre-spec-fixtures.md)
```

**The shared helper.** Each application binds to it in two lines and re-derives neither the layout
nor either read rule (`DOCFILE-007`).

```rust
// ambre-core

/// The writer rule: no version parameter, because there is no write mode to pick.
pub fn write_ambre_plaintext(doc_type: DocType, document_id: Uuid, payload: &[u8]) -> Vec<u8>;

/// Any `DocType`, version-recovery rule applied. What a file-open router reads
/// before it knows which app to hand the bytes to.
pub fn read_ambre_plaintext(data: &[u8]) -> Result<AmbreHeader, DocFileError>;

/// The same, plus the dispatch rule against the caller's own type. The whole body
/// of each app's `import_ambre_plaintext`.
pub fn read_ambre_plaintext_as(expected: DocType, data: &[u8]) -> Result<AmbreHeader, DocFileError>;

/// The strict layout parse: current version only, no recovery. Used by the two
/// above and by anything reading raw bytes -- a header dump, a migration step
/// checking its own output.
pub fn parse_header(data: &[u8]) -> Result<AmbreHeader, DocFileError>;

/// Forward migration, one step per version bump. A step is added with its row.
pub fn migrate_ambre(from: u8, bytes: &[u8]) -> Result<Vec<u8>, DocFileError>;

pub struct AmbreHeader {
    pub version: u8,
    pub doc_type: DocType,
    pub document_id: Uuid,
    /// Offset 22 onward: the raw Loro snapshot.
    pub payload: Vec<u8>,
}
```

**The per-application pair.** The functions are per-application because the payload comes from that
application's `LoroDoc`; the header handling is not reimplemented per application.

```rust
/// Header + uncompressed, unencrypted snapshot. Pure; no crypto (`DOCFILE-006`).
pub fn export_ambre_plaintext(doc: &LoroDoc, doc_id: Uuid) -> Vec<u8>;

/// Validates magic, version and doc_type, and returns the payload. The caller
/// compares the returned `DocType` against its own and re-dispatches on
/// mismatch rather than coercing.
pub fn import_ambre_plaintext(data: &[u8]) -> Result<AmbreHeader, DocFileError>;
```

An application owns the payload half alone: its `export_ambre_plaintext` is `write_ambre_plaintext`
over its own `export_snapshot()`, its `import_ambre_plaintext` is `read_ambre_plaintext_as` for its
own `DocType`. `PayloadRejected` and `PayloadSchema` are raised there and nowhere else, because the container layer
never touches a `LoroDoc`. The schema gate runs between the two: the header validates, then the
payload's own `schema_version` is read in a scratch document, and only then is anything handed to
the live one (`SCHEMA-006`, `SCHEMA-012`). Each engine also carries the state-level pair, so a caller holding a live
document does not reach for its `LoroDoc` to save it.

**The error taxonomy.** Fatal by nature: a header failure means no document can be produced.

```rust
pub enum DocFileError {
    TruncatedHeader,
    InvalidMagic,
    /// `found < AMBRE_VERSION` -> recoverable through `migrate_ambre`.
    /// `found > AMBRE_VERSION` -> unrecoverable; prompt the user to update.
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
    /// The header parsed and the payload is a valid document, but the *schema*
    /// version inside it is one this build will not read, or cannot migrate
    /// (`ambre-spec-schema-version.md`). Raised by the app-side half, like
    /// `PayloadRejected`, and it carries the inner verdict rather than
    /// flattening it: the two versions guarding a stored document are
    /// independent on purpose, and a message that says "unsupported version"
    /// without saying which one sends the user looking in the wrong place.
    PayloadSchema(SchemaError),
}
```

**Code:** `crates/ambre-core/src/docfile.rs` holds the layout, the five entry points and the
migration walk. The gestures: `crates/ambre-app/src/platform.rs` (`open_ambre_file_picker`, the
header read and the store hand-off) with its button in `src/landing.rs`;
`ambre_logic::LogicState::export_ambre_file` and `DocState::ambre_file_event` for the save, which
this thread serves itself because the header's identity comes from the registry that keyed the
document;
`PersistenceStore::save_snapshot_committed` for the durable write the navigation depends on. The per-application bindings are `crates/ambre-trame/src/logic/docfile.rs`,
`crates/ambre-toile/src/logic/interop/docfile.rs` and its state-level pair in
`crates/ambre-toile/src/logic/state/docfile.rs`. `crates/ambre-logic/tests/docfile_dispatch.rs`
drives the dispatch rule across both.

---

## 4. SURFACE

| Trigger | Behaviour |
|---|---|
| save a document to a file | the current state is written at the current version, whatever version it was opened from |
| open a file | the header decides the application, then that application fills an empty state |
| open a file another application wrote | the bytes are routed to that application, with no prompt and no conversion |
| open a file no application claims | one error naming the type byte, and no partial document |
| open a file of a type no engine renders yet | one error naming the kind, and nothing is opened: the launcher offers a file the same predicate its new-document picker uses (`DocType::creatable`), so the two cannot disagree |
| the local store refuses the payload | one error saying so, and no navigation -- a document opened without its bytes is indistinguishable from an empty one |
| open a file a newer client wrote | one error asking the user to update, and the file untouched |
| export a plaintext file | a statement that the file carries no encryption of its own (`DOCFILE-009`) |
| export a sealed file | a passphrase prompt, and a file a reader can tell from a plaintext one by its header alone (`DOCFILE-010`) |

**Where each gesture lives.** The save is per application, because the user is inside a document
when they want it: Toile's File menu (`toile.file.save_file`) and Trame's toolbar
(`trame.interop.save_file`), beside the lossy exports and distinguished from them by its own label.
The open is the launcher's alone, because the file owns the choice of application: at the moment of
picking, no document is open and no engine is booted, and the header is what names the owner. A save
gesture duplicated per application is the cost of one container -- the alternative was an
app-agnostic command and a third download path beside the two the views already own.

**How the open hands the payload over.** The header is read in the launcher, which migrates an older
container or refuses a newer one before anything sees a payload; the payload is then written to the
**local snapshot store** under the document id the header carries, and the page navigates to it. The
store is where a document's bytes already wait for a boot, so the file lands exactly where a refresh
would have left them, and the existing restore leg picks it up -- schema gate included
(`SCHEMA-006`). The navigation is an unconditional reload, where opening a card reloads only across
render kinds: the booted view is fixed per page load, and the restore leg runs at boot rather than on
a router swap, so a pushState would land on an empty document. The write is awaited on the
transaction's commit, not the request's success -- a transaction still open when the page unloads is
aborted, and the file would go with it.

**The two sealed rows are what `DOCFILE P2` builds.** The container beneath every row ships and is
exercised by its own tests and by the fixture corpus.

---

## 5. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| A plaintext file by default | sealing every file | it makes the honest default invisible and still leaves five lossy exports in the clear. An .xlsx, a .csv and a PDF exported from Ambre land on the user's disk in the clear today; the Zero-Knowledge pillar is a statement about the **server**, never about the user's own filesystem, and a partial gesture on one export of six buys the user nothing while suggesting a protection the file lacks (`DOCFILE-009`). |
| A passphrase-sealed variant, on explicit opt-in | no sealed variant at all | it leaves a user carrying a document off the machine with nothing but the filesystem. Argon2id then XChaCha20-Poly1305 over a passphrase typed at export time keeps the key independent of any server-side lifecycle, and the salt and parameters in the header keep the file self-describing and strengthenable later without breaking old files (`DOCFILE-010`). |
| Never the `DocumentKey` | sealing under the key Sync already holds for the document | it is the reading a reader reaches for first, and the reason this row exists. The `DocumentKey` is rotated server-side on every revocation (`ambre-spec-key-rotation.md`), so a file sealed under a since-rotated key is permanently unreadable, silently and late: the user finds out months after the export, when the backup is the only copy left. A local backup whose validity depends on a future membership change is not a backup. |
| The sealed variant is a version, not a flag | a bit in the current header | every existing client reads it as a plaintext file and fails on it. A reader must tell a sealed file from a plaintext one from the header alone, before it attempts to parse a payload (`DOCFILE-010`). |
| A file format, never a wire format | one format for the disk and the socket | it is what put an unnamed key at the centre of the at-rest question. The server already holds `document_id` and `doc_type` as routing metadata, so a header inside the ciphertext duplicates them for nothing, and calling the container a persistence **and sharing** format conflates two destinations (`DOCFILE-011`). |
| No compression and no encryption in this layer | protection inside the container | it puts a crypto dependency in `ambre-core` and a key in every caller. The two entry points stay pure functions over bytes, testable on host with no key and no dependency, and `ambre-sync` stays the sole compression and crypto consumer (`ambre-spec-architecture.md section 2`, `DOCFILE-006`). |
| One container for every application | a per-application container variant | four headers to keep in step, and "open this file" stops being one user action. The type byte plus the dispatch rule cover the whole need (`DOCFILE-007`). |
| A 22-byte binary header | a zip or OPC-style container | it is what OOXML uses, and it buys multi-part packaging Ambre does not need: the payload is one opaque CRDT blob. A fixed layout is trivially inspectable with any hex viewer, and it adds no parser to the Logic thread and no attack surface on untrusted input. |
| A 22-byte binary header | a JSON or RON wrapper around base64 | it inflates every file for a readability that is illusory while the payload stays binary. |
| The document name stays out of the header | carrying the name in the header | it buys a convenience the filename already provides. The name is E2EE metadata (`ambre-spec-zero-knowledge.md`), and in the header it would leak into a plaintext file whose contents the user may have chosen not to expose. |
| Compatibility is a read-side duty | version negotiation | one writer and N readers stays testable as versions accumulate, and every reader path has an input to prove it: the frozen corpus. Negotiation puts the version choice at run time, where no corpus can pin what a build will write. |
| Compatibility is a read-side duty | a backward-compatible write mode | N writers has no input to prove them, because nothing can produce a document at a version the build no longer writes. |

---

## 6. NOT BUILT, AND NOT YET RULED

* **No gesture reaches the container.** No application has a save, an open or a file picker, so
  every row of section 4 is unbuilt (`DOCFILE P1`). What ships is the reader, the writer and the
  migration walk, driven by their own tests and by the fixture corpus.
* **The migration walk has never migrated anything.** The registry holds one row, so
  `migrate_ambre` is exercised by synthetic steps alone (`ambre-spec-testing.md section 6`). The
  first real bump is the first honest test of it, and `DOCFILE-008` is what stops that bump from
  destroying its own input.
* **`DOCFILE-008` binds a recipe that does not exist.** `just fixtures-freeze` lands with
  `FIXTURES P1`, so a bump performed before it has no mechanical way to obey the rule.
* **The sealed variant has no design beyond its two rules.** Nothing says where the passphrase is
  entered, how a wrong one is reported, or what the Argon2id parameters are. `DOCFILE P2` owes all
  three.
* **Nothing rules on a file larger than memory.** Both entry points take and return a `Vec<u8>`, so
  a document is read whole. No row bounds the size a reader accepts, and no budget names one.
* **Nothing fuzzes the header.** It is a parser on untrusted input and it has no fuzz target
  (`ambre-spec-testing.md section 8`); the belt is a corpus of malformed inputs as unit tests.

---

## 7. WHAT THIS DOCUMENT DOES NOT OWN

| Question | Owner |
|---|---|
| What goes inside the payload, and what each application drops on every other export | `2-Trame/features/ambre-spec-trame-import-export.md`, `3-Toile/features/ambre-spec-toile-import-export.md`, `4-Prose/features/ambre-spec-prose-import-export.md` |
| How a snapshot is compressed, sealed and relayed | `ambre-spec-sealed-envelope.md` |
| How the payload's own schema is versioned and migrated | `ambre-spec-architecture.md section 2` |
| What a committed artifact is, how it is frozen and what its migration belt asserts | `ambre-spec-fixtures.md` |
| Which crate holds which belt, and what runs in CI | `ambre-spec-testing.md section 6` |
