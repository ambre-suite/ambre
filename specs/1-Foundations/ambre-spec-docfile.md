# AMBRE -- FEATURE: THE NATIVE FILE (`.ambre`)

**Type:** Horizontal (framework)
**Implementation:** partial -- the container ships, and so do both lineages. A save per
application, one open in the launcher, plaintext or passphrase-sealed.

> The file a user saves a document into, and opens back. One container for every application: a
> spreadsheet, a board and a text document differ by one byte of the header.

Every other export drops something; this file drops nothing, so it is the local backup and the
way out. What travels inside belongs to each application's import and export feature.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `DOCFILE-001` | `IO` | A `.ambre` round trip MUST be the identity on document state, leaving nothing a warning list could report: the payload **is** the document. | this spec section 3 | `ambre_core::docfile::tests::round_trip_is_the_identity_on_the_container`; `ambre_trame::logic::docfile::tests::round_trip_preserves_the_document_state`; `::round_trip_is_byte_identical_on_a_bare_document`; `ambre_toile::logic::state::docfile::tests::round_trip_preserves_the_document_state`; `::round_trip_is_byte_identical_on_a_bare_document`; `fixture_determinism::the_round_trip_is_stable_on_every_doc_state_variant` | none | verified |
| `DOCFILE-006` | `INV` | The container layer MUST NOT compress or encrypt: the two entry points are pure functions over bytes, and protection is a separate stage. | this spec section 5 | crate-graph -- `ambre-core` declares no crypto and no compression dependency, so neither is reachable from the container | none | verified |
| `DOCFILE-007` | `INV` | There MUST be one container for every app: the only app-dependent value in the header is the `doc_type` byte. | this spec section 3 | `ambre_core::docfile::tests::header_layout_is_the_spec_layout` -- it pins the 22-byte layout, and one writer takes the type as a parameter, so a per-app variant has nowhere to be written | none | verified |
| `DOCFILE-011` | `INV` | A snapshot on the wire MUST NOT carry this header: the container is a file format, never a wire format. | this spec section 5 | `ambre_logic::dispatch::tests::a_snapshot_on_the_wire_carries_no_container_header` -- `SnapshotNeeded` is the one path that hands a snapshot to Sync, and the test reads the payload the dispatcher sends and refuses `AMBRE_MAGIC` in its first four bytes; structural -- the sealing entry point answers `RestEvent::DocFileSealed` to the surface that asked, so its bytes reach a download and never a relay message | none | verified |
| `DOCFILE-008` | `INV` | A version bump MUST freeze one named artifact per `doc_type` in the same commit that adds the registry row. | this spec section 3; `ambre-spec-fixtures.md` | human-review -- the reviewer of a bump reads the diff for one frozen artifact per `doc_type`, which is the step section 3 spells out. `just fixtures-freeze <name>` writes the artifact and the gate then holds it for good (`FIXTURE-018`, `FIXTURE-019`, `FIXTURE-021`) | unheld -- what no check can hold is the binding: the bump commit is the last one whose writer still emits the outgoing version, so nothing can reconstruct that artifact afterwards to prove it was owed | human-review |
| `DOCFILE-010` | `INV` | A passphrase-sealed file MUST carry its own container version; it MUST NOT be a flag on an existing one. | this spec section 5 | `ambre_core::docfile::tests::the_sealed_layout_is_the_spec_layout` -- the 45-byte header and its version byte; `::the_plaintext_reader_refuses_a_sealed_file_by_name`; `::the_sealed_parser_refuses_a_plaintext_file_by_name` -- the two refusals that make the version the discriminator; `::an_unknown_sealed_layout_is_refused_without_a_migration_promise`; `::migrating_a_sealed_version_says_it_is_sealed`; `::a_truncated_sealed_header_is_rejected_at_every_length` -- the reader on inputs it must not guess at; `::a_sealed_round_trip_recovers_the_envelope` | none | verified |
| `DOCFILE-009` | `OBS` | The export surface MUST state that a plaintext file carries no encryption of its own. | this spec section 5 | `ambre_toile::ui::commands::tests::the_save_label_says_the_file_is_not_encrypted`; `ambre_trame::ui::commands::tests::the_save_label_says_the_file_is_not_encrypted` -- asserted on the command's label, the one string a user reads before clicking, rather than on a tooltip or a dialog, so a surface that drops the warning cannot compile green | none | verified |
| `DOCFILE-014` | `INV` | Sealing and opening a file MUST run in the Sync worker. | this spec section 5; `ARCH-004` | `ambre_logic::dispatch::tests::a_sealed_export_hands_the_plaintext_to_sync_and_nothing_to_the_ui`; `ambre_sync::integration::a_sealed_file_round_trips_through_the_service_plane` -- the worker's own half; cargo-deny -- `ambre-core`, `ambre-logic`, `ambre-toile` and `ambre-trame` declare no crypto dependency, so neither an application nor the container layer can seal anything | none | verified |
| `DOCFILE-017` | `INV` | A sealed export MUST reach the user through the event its plaintext export already uses. | this spec section 5; `ARCH-004` | `ambre_logic::dispatch::tests::the_sealed_bytes_come_back_as_the_application_s_own_export_event`; `::a_failed_seal_answers_a_failure_and_no_file`; `::a_document_closed_mid_seal_drops_the_answer` -- the one event on all three outcomes | none | verified |

### 1.2 Read a `.ambre` file

The reader is handed bytes and nothing else. It reads the header, decides what wrote the file, and
migrates or refuses before an application sees a payload.

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `DOCFILE-002` | `POST` | A reader MUST identify the file from `doc_type` and `version`, before any app-specific parsing. | this spec section 3 | `ambre_core::docfile::tests::header_layout_is_the_spec_layout`; `::dispatch_reports_the_type_that_wrote_the_file`; `::an_unknown_doc_type_byte_is_not_a_dispatch_target`; `ambre_trame::logic::docfile::tests::a_toile_file_is_re_dispatched_not_coerced` | none | verified |
| `DOCFILE-003` | `POST` | A version older than the binary's MUST migrate on read. | this spec section 3 | `ambre_core::docfile::tests::the_walk_applies_one_step_per_version`; `::the_walk_chains_steps_in_order`; `::a_gap_in_the_registry_stops_the_walk_at_the_missing_row`; `::a_failing_step_fails_the_whole_migration` | none | verified |
| `DOCFILE-012` | `FRAME` | A reader MUST NOT consult the file's name or extension. | this spec section 3 | type-level -- both entry points take a byte slice, so there is no path, no name and no extension in scope to branch on | none | verified |
| `DOCFILE-004` | `FRAME` | No error path MUST mutate the bytes it was handed. | this spec section 3 | `ambre_core::docfile::tests::every_error_path_leaves_the_input_untouched`; `ambre_trame::logic::docfile::tests::a_rejected_file_leaves_both_the_input_and_the_state_alone` | none | verified |
| `DOCFILE-013` | `FAIL` | A version newer than the binary's MUST be refused cleanly. | this spec section 3 | `ambre_core::docfile::tests::a_newer_version_is_refused_and_an_older_one_is_attempted`; `::migrating_from_a_newer_version_is_refused` | none | verified |
| `DOCFILE-005` | `FAIL` | A malformed file MUST produce a typed error, never a panic. | this spec section 3 | `ambre_core::docfile::tests::truncated_header_is_rejected_at_every_length`; `::bad_magic_is_rejected`; `ambre_trame::logic::docfile::tests::a_corrupt_payload_under_a_valid_header_is_typed_not_a_panic`; cargo-fuzz -- `fuzz/fuzz_targets/docfile.rs`, run by the CI `fuzz` job | none | verified |

### 1.3 Seal a `.ambre` file, and open one back

Two operations the plaintext lineage does not have. The surface takes the passphrase; the key and
the cipher never leave the Sync worker (`DOCFILE-014`).

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `DOCFILE-015` | `FRAME` | The seal prompt MUST take the passphrase twice, and MUST NOT seal on a mismatch. | this spec section 4 | `ambre_chrome::passphrase::tests::sealing_needs_the_two_fields_to_agree`; `::nothing_is_submitted_from_an_empty_prompt`; `::opening_asks_once_and_ignores_the_second_field` -- the rule is the prompt's one decision, so it is a pure function of the two fields and tested on host, and the two inputs around it stay DOM glue | none | verified |
| `DOCFILE-016` | `FAIL` | A file that does not open MUST report the same failure whether the passphrase is wrong or the file was modified. | this spec section 5 | `ambre_sync::integration::a_wrong_passphrase_and_a_tampered_file_fail_the_same_way` -- three inputs, a wrong passphrase, an edited ciphertext and an edited salt, against one variant; type-level -- `RestEvent::DocFileOpenFailed` is a unit variant, so a message that distinguished the two causes would have nothing to read | none | verified |

* **Budget.** none, and `ambre-spec-budgets.md` records that: the container adds a 22-byte write,
  a 22-byte parse, and a few dozen bytes in `ambre-core`, shared rather than duplicated.
* **Boundary.** the container holds no key (`DOCFILE-006`), and it keeps the bytes it produces on a
  local disk (`DOCFILE-011`). The seal is the one place a key appears, in the Sync worker for one
  call, and no application links against a cipher (`DOCFILE-014`, `ARCH-004`).
* **Verification.** 16 `verified`, 1 `human-review`. This is pure byte handling on the host tier,
  so a test reaches every rule the container makes about bytes. What the wire must not carry is
  held one crate away, at the single seam that hands a snapshot to Sync. The one gap left is a
  bump obligation no test can time.

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
| mails a sealed file, and the passphrase by another route | opens it with the passphrase | a salt, a nonce and one ciphertext -- no type byte, no document id | the document, after one prompt and one key derivation | `ambre_sync::integration::a_sealed_file_round_trips_through_the_service_plane` |
| mails a sealed file | reads it without Ambre, or with the wrong passphrase | the same bytes | nothing: not the values, not the document's type, not its identity | `ambre_sync::integration::a_wrong_passphrase_and_a_tampered_file_fail_the_same_way` |
| mails a sealed file that was modified in transit | opens it with the right passphrase | a ciphertext whose tag no longer matches | the same refusal a wrong passphrase earns, and no partial document (`DOCFILE-016`) | `ambre_sync::integration::a_wrong_passphrase_and_a_tampered_file_fail_the_same_way` |
| seals a file and forgets the passphrase | -- | a ciphertext under a key nothing else derives | nothing, from either user: there is no recovery path and the prompt says so before the file is written (`DOCFILE-015`) | `ambre_chrome::passphrase::tests::sealing_needs_the_two_fields_to_agree` |

---

## 3. MODEL

**CRDT containers.** none. The file carries a snapshot of the application's own and declares none
of its own; the word *container* below names the file layout.

**The layout.** A fixed 22-byte header, then the payload.

| Offset | Size | Field | Holds |
|---|---|---|---|
| 0 | 4 | `magic` | `0x414D4252`, that is `AMBR` |
| 4 | 1 | `version` | the container version. Current: `0x01` |
| 5 | 1 | `doc_type` | `DocType as u8` -- `Prose` 0, `Trame` 1, `Toile` 2, `Murmure` 3 |
| 6 | 16 | `document_id` | a UUID v7 |
| 22 | rest | `payload` | `LoroDoc::export_snapshot()`, uncompressed and unencrypted (`DOCFILE-006`) |

**The version registry.** One row per container version, appended and never rewritten. A bump is a
container change alone: it does not track the payload's CRDT schema, which each application raises
on its own (`ambre-spec-architecture.md section 2`).

| Version | Status | Change |
|---|---|---|
| `0x01` | current | the initial layout |

**Adding a row freezes an artifact, in the same commit** (`DOCFILE-008`). A writer emits the current
version only, so that commit is the last moment the outgoing version can be written. Three steps:

1. add the registry row and the migration step that reads the outgoing version;
2. run `just fixtures-freeze <name>` for one **named** artifact per `doc_type`. Several generators
   share a `doc_type`, so which lineage keeps migration coverage is a choice, not a default;
3. commit the frozen artifact with the bump (`ambre-spec-fixtures.md`).

Skip step 2 and the outgoing version becomes untestable for good. No generator can produce it
again, and `migrate_ambre` loses the only input that proves it works.

**The three reader and writer rules**, normative.

| Rule | What it says | Why it is this way |
|---|---|---|
| Dispatch | a reader never coerces a document across applications. On `WrongDocType` the caller routes the same bytes to the importer the error names, and surfaces an error only when no application claims that type | it makes "open a `.ambre`" one user action rather than four (`DOCFILE-002`) |
| Version recovery | below the current version, apply `migrate_ambre` in order and retry the import. Above it, refuse, leave the file untouched, and ask the user to update the client | a partial or best-effort read of a format this build does not know produces a document nobody can trust (`DOCFILE-003`) |
| Writer | a writer emits the current version only. There is no backward-compatible write mode and no negotiation: compatibility is a **read-side** duty | one writer and N readers stays testable as versions accumulate; N writers does not (`DOCFILE-007`) |

**The sealed layout.** A second lineage, not a variant of the first: a fixed 45-byte header, then
one ciphertext (`DOCFILE-010`). The version byte tells the two apart, read before anything else in
either reader.

| Offset | Size | Field | Holds |
|---|---|---|---|
| 0 | 4 | `magic` | `0x414D4252`, that is `AMBR` -- the same four bytes: a sealed file is an `.ambre` |
| 4 | 1 | `version` | the sealed layout version. Current: `0x80` |
| 5 | 16 | `salt` | the Argon2id salt, drawn per file |
| 21 | 24 | `nonce` | the XChaCha20-Poly1305 nonce, drawn per file |
| 45 | rest | `ciphertext` | a whole plaintext container, authentication tag included |

**The high bit is the discriminator, and that is why `0x80`.** Plaintext versions count from
`0x01`, sealed ones from `0x80`. Each registry grows 127 rows before a collision is possible, and
one byte sorts a file. Every build that ever shipped refuses `0x80` as a plaintext version. An old
client shows the update prompt rather than parsing a salt as a `doc_type`.

| Version | Status | Change |
|---|---|---|
| `0x80` | current | the initial sealed layout: Argon2id v1 at the login work factor, then XChaCha20-Poly1305 |

**The seal authenticates the header, rather than merely carrying it.** The associated data is the
first 21 bytes: magic, version and salt. An edited salt fails the tag instead of quietly
deriving a second key. The nonce is left out: the AEAD authenticates its own.

**There is no sealed migration walk.** A plaintext container migrates because its bytes are
readable. A sealed one is not, and rewriting its envelope means deriving the key inside it: a
re-seal under a passphrase the user must type. This layer refuses an unknown sealed version. The
document inside *does* migrate one round trip later: a seal opens to a whole plaintext container,
header included. Version recovery and the schema gate run unchanged.

**The parameters, and where they live.** Argon2id v1 at the login work factor -- 64
MiB, 3 passes, 1 lane -- then one domain-separated BLAKE2b pass. The result is a `FileKey` that no
conversion turns into a `DocumentKey` (`ambre-spec-zero-knowledge.md` holds the hierarchy). The cost
is the point: a few hundred milliseconds and 64 MiB per seal and per open, paid in the Sync worker
(`ambre-spec-budgets.md`). The salt makes the file self-describing, so a later version may raise the
work factor; old files keep opening at their own.

**Two stages, and they are not interchangeable.** The container is stage one; protection is stage
two, and where the file is going decides which applies (section 5).

```
   app state
      |  export_ambre_plaintext()          Logic thread, no crypto (DOCFILE-006)
      v
   .ambre plaintext  = header || snapshot
      |
      +---> local file, plaintext: written as-is (DOCFILE-009)
      |
      +---> local file, sealed: Logic -> Sync -> Logic, on opt-in (DOCFILE-014)
      |        Argon2id(passphrase, salt) -> FileKey
      |        XChaCha20-Poly1305(plaintext, aad = header[..21]) -> ciphertext
      |        sealed header || ciphertext
      |
      +---> fixtures corpus: committed as-is (ambre-spec-fixtures.md)
```

An open runs the same steps backwards. The version byte says which lineage the bytes are. The
sealed reader refuses a plaintext file before deriving any key: an unsealed file must not cost a
guessing attempt.

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

**The sealed half of the same crate.** Still pure functions over bytes, and still no key: the
container assembles and parses the envelope, and its caller fills it (`DOCFILE-006`).

```rust
// ambre-core

/// The cheapest question a reader can ask, and the first one it must: which
/// lineage, at which version. Five bytes answer it, so it answers for a file too
/// short to hold either header in full.
pub fn classify(data: &[u8]) -> Result<ContainerKind, DocFileError>;

pub enum ContainerKind { Plaintext(u8), Sealed(u8) }

/// The associated data a seal must be computed over, for a file about to be
/// written under `salt`: the header up to the nonce.
pub fn sealed_aad(salt: &[u8; SALT_LEN]) -> [u8; SEALED_AAD_LEN];

/// The sealed writer rule. No version parameter either: a sealed file is written
/// at `AMBRE_SEALED_VERSION` or not at all.
pub fn write_ambre_sealed(
    salt: &[u8; SALT_LEN],
    nonce: &[u8; SEAL_NONCE_LEN],
    ciphertext: &[u8],
) -> Vec<u8>;

/// The current sealed layout exactly, with no recovery walk behind it.
pub fn parse_sealed_header(data: &[u8]) -> Result<SealedAmbre, DocFileError>;

pub struct SealedAmbre {
    pub version: u8,
    pub salt: [u8; SALT_LEN],
    pub nonce: [u8; SEAL_NONCE_LEN],
    /// Offset 45 onward. Opens to a whole plaintext container.
    pub ciphertext: Vec<u8>,
}

impl SealedAmbre {
    /// The associated data this file was sealed against, assembled from the same
    /// code as `sealed_aad`, so the two cannot drift.
    pub fn aad(&self) -> [u8; SEALED_AAD_LEN];
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
own `DocType`. `PayloadRejected` and `PayloadSchema` are raised there and nowhere else, because the
container layer never touches a `LoroDoc`. The schema gate runs between the two. The header
validates, the payload's own `schema_version` is read in a scratch document, and only then is
anything handed to the live one (`SCHEMA-006`, `SCHEMA-012`). Each engine also carries the
state-level pair, so saving a live document does not reach for its `LoroDoc`.

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
    /// The plaintext reader was handed a sealed container. Its mirror is
    /// `NotSealed`, and a caller that classifies first reaches neither: both
    /// exist so that neither parser has to guess what the other's bytes mean.
    SealedFile { version: u8 },
    NotSealed,
    /// A sealed layout this build does not know. Distinct from
    /// `UnsupportedVersion`, whose older half promises a migration: the sealed
    /// lineage has none to promise.
    SealedVersionUnsupported { found: u8 },
}
```

**No error says whether a passphrase was wrong.** The open failure crosses the bus as one unit
variant and reaches the user as one sentence (`DOCFILE-016`). The authentication tag cannot tell a
wrong key from edited bytes, so a message that chose between them would be a guess.

**Code:** `crates/ambre-core/src/docfile.rs` holds both layouts, the entry points of each lineage
and the migration walk. The gestures live in three places.

* `crates/ambre-app/src/platform.rs` -- `open_ambre_file_picker`, the classification, the header
  read and the store hand-off, with its button in `src/landing.rs`.
* `ambre_logic::LogicState::export_ambre_file` and `DocState::ambre_file_event` for the save. That
  thread serves it because the header's identity comes from the registry that keyed the document.
* `PersistenceStore::save_snapshot_committed` for the durable write.

The per-application bindings are
`crates/ambre-trame/src/logic/docfile.rs`,
`crates/ambre-toile/src/logic/interop/docfile.rs` and its state-level pair in
`crates/ambre-toile/src/logic/state/docfile.rs`. `crates/ambre-logic/tests/docfile_dispatch.rs`
drives the dispatch rule across both.

The seal adds one crate each side and no third path. `crates/ambre-sync/src/docfile.rs` holds the
two handlers -- the only service requests in that crate that never touch the relay -- over
`ambre_crypto::keys::FileKey` and `ambre_crypto::aead::{seal_file, open_file}`.
`ambre_logic::LogicState::export_ambre_file_sealed` intercepts the sealed command beside the
plaintext one. `pending_seals` holds the document and the filename until the worker answers, and
`resolve_pending_seal` turns that answer into the application's own export event (`DOCFILE-014`).
The prompt is `crates/ambre-chrome/src/passphrase.rs`, mounted in `src/shell/app_root.rs` for a
seal and in `src/landing.rs` for an open.

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
| export a sealed file | a passphrase prompt, typed twice, then the same download the plaintext export produces -- one command per application, beside the plaintext save (`DOCFILE-010`, `DOCFILE-014`) |
| type two different passphrases at the prompt | nothing is sealed and nothing is written: the prompt says they differ and keeps both fields (`DOCFILE-015`) |
| open a sealed file | the same file picker, then a passphrase prompt: the version byte decides which of the two the bytes need, never the filename |
| open a sealed file with a wrong passphrase, or one that was modified | one message for both, naming neither cause, and the session untouched (`DOCFILE-016`) |
| a seal that fails outright | one failure message and no file. The reachable causes are a key derivation that ran out of memory and a cipher that refused; neither produces bytes worth writing |
| close the document while its file is being sealed | the answer is dropped: no download lands in a tab whose document is gone |

**Where each gesture lives.** The save is per application, because the user is inside a document
when they want it. Toile's File menu (`toile.file.save_file`) and Trame's toolbar
(`trame.interop.save_file`) carry it, beside the lossy exports and set apart by its own label. The
open is the launcher's alone, because the file owns the choice of application. No document is open
and no engine is booted when the picking happens, and the header names the owner.

**How the open hands the payload over.** The launcher reads the header, migrating an older
container or refusing a newer one before anything sees a payload. The payload is then written to the
**local snapshot store** under the document id the header carries, and the page navigates to it.
That store is where a document's bytes already wait for a boot, so the existing restore leg picks
the file up, schema gate included (`SCHEMA-006`).

The navigation is an unconditional reload, where opening a card reloads only across render kinds.
The restore leg runs at boot, so a pushState would land on an empty document. The write is awaited
on the transaction's commit, not the request's success: a transaction still open at unload is
aborted, and the file with it.

**Where the user types the passphrase, and where they do not.** The launcher asks in both
directions, never an application. A passphrase cannot travel in a `CommandId`, so the sealed save
is a UI-only intent the launcher intercepts. The shape Find and the column filter already do this.
One component serves both (`ambre-chrome`): one operation seals a board and a workbook.

**How the sealed save travels.** The application emits `ExportAmbreFileSealed` and knows nothing
else about it. Logic intercepts that command, writes the plaintext container itself, and hands it to
the Sync worker with the passphrase. The answer comes back as the application's **ordinary** export
event, which leaves the download anchor, the filename rule and both engines untouched
(`DOCFILE-014`). It lives in memory for that trip alone -- never on disk, never on the relay.

**How the sealed open travels.** The picker classifies the bytes before it reads them. A plaintext
file opens as it always did. A sealed one waits in the launcher, the prompt asks for the
passphrase, and the worker answers with a whole plaintext container. That container walks the
plaintext path above. So a sealed file whose *inner* container is older migrates on that second leg.

---

## 5. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| A plaintext file by default | sealing every file | it makes the honest default invisible and still leaves five lossy exports in the clear. An .xlsx, a .csv and a PDF exported from Ambre land on the user's disk in the clear today; the Zero-Knowledge pillar is a statement about the **server**, never about the user's own filesystem, and a partial gesture on one export of six buys the user nothing while suggesting a protection the file lacks (`DOCFILE-009`). |
| A passphrase-sealed variant, on explicit opt-in | no sealed variant at all | it leaves a user carrying a document off the machine with nothing but the filesystem. Argon2id then XChaCha20-Poly1305 over a passphrase typed at export time keeps the key independent of any server-side lifecycle, and the salt and parameters in the header keep the file self-describing and strengthenable later without breaking old files (`DOCFILE-010`). |
| Never the `DocumentKey` | sealing under the key Sync already holds for the document | it is the reading a reader reaches for first, and the reason this row exists. The `DocumentKey` is rotated server-side on every revocation (`ambre-spec-key-rotation.md`), so a file sealed under a since-rotated key is permanently unreadable, silently and late: the user finds out months after the export, when the backup is the only copy left. A local backup whose validity depends on a future membership change is not a backup. |
| The sealed variant is a version, not a flag | a bit in the current header | every existing client reads it as a plaintext file and fails on it. A reader must tell a sealed file from a plaintext one from the header alone, before it attempts to parse a payload (`DOCFILE-010`). |
| The seal runs in the Sync worker | sealing in the canvas, on the UI thread | Argon2id at 64 MiB and a few hundred milliseconds on the thread that paints is a visibly frozen application, and it would put a cipher in `ambre-toile` and `ambre-trame` -- which `cargo deny` refuses, because `ARCH-004` is held by the crate graph rather than by review. The worker already holds an event loop of its own and already holds every key (`DOCFILE-014`). |
| The seal runs in the Sync worker | sealing in the Logic thread, which already writes the container | it is the same crate-graph violation one thread over: Logic holds the documents, and a cipher there makes every engine's dependency tree reach a key. Logic writes the plaintext container and forwards it, which is the division the two rules already draw (`DOCFILE-006`, `ARCH-004`). |
| One command, intercepted where the plaintext save is | a sealed export path of its own, per application | it duplicates the filename rule, the download anchor and the failure surface per application, and it makes "save to a file" two features that drift. The interception point already exists and already serves both applications, because one container serves both (`DOCFILE-007`). |
| The answer comes back as the application's own export event | a sealed-specific event the canvas would learn to handle | the canvas downloads bytes; that a passphrase was involved is not its business. A second event means two code paths for one user-visible outcome, and the second one is the one nobody exercises. |
| The plaintext container never touches the disk | sealing a file already written out | it is the failure mode the feature exists to prevent: the plaintext would land on the user's filesystem, be indexed and backed up, and a crash between the two steps leaves it there for good. The bytes go from Logic to the worker in memory and only the ciphertext is ever offered to the user. |
| One message for a wrong passphrase and a modified file | naming the cause | nothing can name it. Poly1305 fails identically for a wrong key and for edited bytes, so a message that chose would be a guess the user trusts -- and "the file is damaged" sends someone looking for a backup when they mistyped a character (`DOCFILE-016`). |
| The passphrase is typed twice when sealing | one field, like every other password prompt | a login prompt can afford one field because a typo is discovered in seconds and costs nothing. A sealed file's typo is discovered when the file is the only copy left, and nothing in the system can recover it: the confirmation is the only check that exists at all (`DOCFILE-015`). |
| An unknown sealed layout is refused | a sealed migration walk | rewriting an envelope means deriving the key inside it, which is a re-seal under a passphrase the user must type -- not a read-side recovery, and not something a reader may do silently. The document inside migrates through the plaintext walk once the seal comes off, which is where version recovery belongs. |
| The version byte separates the lineages, high bit set | a magic of its own for sealed files | it makes a sealed file stop being an `.ambre` to every tool that recognizes the magic, and it doubles the discriminator a reader must check. One byte already had to be read first; the high bit gives each lineage 127 versions and no possible collision (`DOCFILE-010`). |
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

* **Nothing bounds how long a seal may take on a real device.** The budget row exists and no
  measurement does (`ambre-spec-budgets.md`): the figure is the login work factor, carried over
  rather than re-derived for a file. No criterion benchmark runs the derivation on the target
  tier. A mid-range phone that takes three seconds per export would go unnoticed until a user says
  so.
* **Nothing rules on the strength of what the user types.** The prompt refuses an empty passphrase
  and a mismatched confirmation, and nothing else. No length floor, no estimate shown, no refusal
  of a passphrase a wordlist holds. Argon2id's work factor is the whole defence, and it is a
  constant -- a four-character passphrase seals as readily as a good one.
* **No gesture suspends a document while its file seals.** The user keeps editing during the
  derivation. The sealed file holds the state as of the moment the command left, not the moment
  the download lands. That is the same semantics as the plaintext save, and no rule states
  either.
* **The migration walk has never migrated anything.** The registry holds one row, so
  `migrate_ambre` is exercised by synthetic steps alone (`ambre-spec-testing.md section 6`). The
  first real bump is the first honest test of it, and `DOCFILE-008` is what stops that bump from
  destroying its own input.
* **A sealed file's size is the plaintext's, plus 45 bytes and a tag.** Nothing compresses on this
  path: `DOCFILE-006` bans it from the container, and `DOCFILE P2` added no stage for it. The
  sealed lineage seals what the plaintext lineage wrote, unchanged.
* **Nothing rules on a file larger than memory.** Both entry points take and return a `Vec<u8>`, so
  a document is read whole. No row bounds the size a reader accepts, and no budget names one.

---

## 7. WHAT THIS DOCUMENT DOES NOT OWN

| Question | Owner |
|---|---|
| What goes inside the payload, and what each application drops on every other export | `2-Trame/features/ambre-spec-trame-import-export.md`, `3-Toile/features/ambre-spec-toile-import-export.md`, `4-Prose/features/ambre-spec-prose-import-export.md` |
| How a snapshot is compressed, sealed and relayed | `ambre-spec-sealed-envelope.md` |
| How the payload's own schema is versioned and migrated | `ambre-spec-architecture.md section 2` |
| What a committed artifact is, how it is frozen and what its migration belt asserts | `ambre-spec-fixtures.md` |
| Which crate holds which belt, and what runs in CI | `ambre-spec-testing.md section 6` |
