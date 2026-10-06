# AMBRE -- FEATURE: THE PERSISTED SCHEMA

**Type:** Horizontal (framework)
**Applications:** Prose, Trame, Toile
**Implementation:** partial -- the version register ships. Every document carries it from creation,
every full-state entry point reads it, and each of the three applications carries a migration
registry. The one-register rule ships in the two applications that store a style register, and
nothing refuses a third shape.

> What a document looks like once it reaches the disk, and how a later binary knows it can still
> read it. A stored document that opens into the wrong meaning is the worst failure available: it
> is silent.

Each application declares its own containers in its own schema document. This one holds the two
rules every application obeys, and the version that makes them safe to change. The version says
where the number lives, what an absent one means, what each comparison implies, and how a reader
walks the forward steps. The semantics of a step are the application's, never the foundation's.

---

## 1. CONTRACT

The only mandatory read. Grammar and status values are normative in `ambre-spec-template.md`. The
prefix is `SCHEMA`.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `SCHEMA-001` | `INV` | A styled unit's whole style MUST be one binary register keyed by that unit's identity; a register per attribute is forbidden. | this spec section 2; `features/ambre-spec-trame-cell-styles.md` | `ambre_trame::logic::convergence_tests::concurrent_whole_cell_style_edits_do_not_blend_across_containers`; `::subset_style_edit_does_not_tear_a_whole_cell_rewrite`; `ambre_toile::logic::convergence_tests::concurrent_whole_element_restyles_do_not_blend_across_registers` -- two peers restyle one element concurrently and exchange whole histories, and the converged style must be one peer's, so a field arriving from the other peer is red | none | verified |
| `SCHEMA-002` | `INV` | A shared formatting control MUST emit one changed field. | this spec section 2; `ambre-spec-chrome.md` | crate-graph -- `ambre-chrome` depends on no application crate, so a shared control has no whole-style type to name and can only report the scalar it edits; ci-gate -- `scripts/crate_boundaries.py`, run by `just crate-boundaries` in the CI `lint` job, which refuses the dependency line that would hand it one | none | verified |
| `SCHEMA-011` | `INV` | A view MUST fold that field into a whole-register write. | this spec section 2; `ambre-spec-chrome.md` | `ambre_trame::logic::schema_shape::the_shape_a_build_writes_is_the_shape_frozen_for_its_version`; `ambre_toile::logic::state::schema_shape::the_shape_a_build_writes_is_the_shape_frozen_for_its_version` -- a field written into a register of its own is a key the shape did not carry, so the frozen text gains a line and the belt turns red; ci-gate -- `scripts/schema_freeze.py`, run by `just schema-freeze` in the CI `lint` job, which holds the published section append-only, so the red cannot be cleared by rewriting it | none | verified |
| `SCHEMA-004` | `INV` | Every document MUST carry its schema version in a root register, written when the document is created. | this spec section 2 | `ambre_prose::schema::tests::a_new_document_carries_the_current_version`; `ambre_toile::logic::schema::tests::a_new_board_carries_the_current_version`; `ambre_trame::logic::schema::tests::a_new_workbook_carries_the_current_version` -- written inside the scaffolding commit of each engine's `from_doc`, so no document reaches persistence without it | none | verified |
| `SCHEMA-009` | `INV` | A change to the layout of a persisted record MUST raise the schema version. | this spec section 2 | `ambre_prose::logic::schema_shape::the_shape_a_build_writes_is_the_shape_frozen_for_its_version`; `ambre_trame::logic::schema_shape::the_shape_a_build_writes_is_the_shape_frozen_for_its_version`; `ambre_toile::logic::state::schema_shape::the_shape_a_build_writes_is_the_shape_frozen_for_its_version` -- each recomputes the shape from this build and compares it against the section frozen for the compiled `SCHEMA_VERSION`, so a moved key, a moved value type or a moved `postcard` layout is red until the number rises; ci-gate -- `scripts/schema_freeze.py`, run by `just schema-freeze` in the CI `lint` job, which holds a published section append-only, so the red cannot be cleared by rewriting the section it broke | none | verified |
| `SCHEMA-017` | `INV` | A change to what a key's value means, the key and its type unchanged, MUST raise the schema version in the same commit as the change. | this spec section 2; `ambre-spec-fixtures.md section 2` | human-review -- in each application that stamps a schema version, the reviewer reads the diff for a unit or a reference point that moved under an unchanged key, which is the one drift no frozen shape can show | unheld -- no gate is possible. Prose's `meta/defaults/line_height` holds a multiple of the font size; a build that reads the same `f32` as a point measure writes the same key and the same type, every shape snapshot stays green, and every document written before the change is read wrong | human-review |
| `SCHEMA-015` | `INV` | An application that stamps a schema version MUST freeze the shape its build writes. | this spec section 2 | ci-gate -- `scripts/schema_freeze.py`, run by `just schema-freeze` in the CI `lint` job, which reads every crate declaring a `SCHEMA_VERSION` and refuses one carrying no frozen shape | none | verified |
| `SCHEMA-016` | `INV` | Each build MUST be compared against the section frozen for its version. | this spec section 2 | ci-gate -- `scripts/schema_freeze.py`, run by `just schema-freeze` in the CI `lint` job, which refuses a frozen shape whose crate names no belt, and a compiled version with no section for the belt to read | none | verified |
| `SCHEMA-018` | `INV` | The key set a build writes, the type each key's value holds, and the `postcard` layout of each binary register MUST match the shape frozen under the compiled `SCHEMA_VERSION`. | this spec section 2; architecture section 2.D; `ambre-spec-fixtures.md section 2` | Trame `ambre_trame::logic::schema_shape::the_shape_a_build_writes_is_the_shape_frozen_for_its_version`; `ambre_trame::logic::schema_shape::every_declared_key_appears_in_the_shape`; `ambre_trame::logic::schema_shape::a_key_appearing_moves_the_shape`; `ambre_trame::logic::schema_shape::a_value_type_changing_moves_the_shape`; Toile `ambre_toile::logic::state::schema_shape::the_shape_a_build_writes_is_the_shape_frozen_for_its_version`; `ambre_toile::logic::state::schema_shape::every_declared_key_appears_in_the_shape`; `ambre_toile::logic::state::schema_shape::a_key_appearing_moves_the_shape`; `ambre_toile::logic::state::schema_shape::a_value_type_changing_moves_the_shape`; Prose `ambre_prose::logic::schema_shape::the_shape_a_build_writes_is_the_shape_frozen_for_its_version`; `ambre_prose::logic::schema_shape::every_declared_key_appears_in_the_shape`; `ambre_prose::logic::schema_shape::a_key_appearing_moves_the_shape`; `ambre_prose::logic::schema_shape::a_value_type_changing_moves_the_shape` | none | verified |
| `SCHEMA-019` | `INV` | A frozen shape section MUST NOT change once the default branch carries it. | this spec section 2; architecture section 2.D | ci-gate -- `scripts/schema_freeze.py`, run by `just schema-freeze` in the CI `lint` job over the shape files of every application, its own reads held by `scripts/test_schema_freeze.py` under `just test-scripts` | none | verified |
| `SCHEMA-020` | `CONC` | Two peers migrating one document at the same time MUST NOT lose an edit made to what either migrated. | this spec section 2 | `ambre_toile::logic::schema::tests::two_peers_migrating_one_board_keep_both_edits` -- two peers migrate one version 3 board, each types into one body, and both edits survive with the two version vectors equal, so neither peer minted an operation the other lacks; `ambre_core::schema::tests::one_history_derives_one_migration_peer` -- the identity is pinned to its value, so a peer on another build derives the same one | none | verified |
| `SCHEMA-010` | `INV` | A migration MUST live in the application crate that owns the schema it migrates. | this spec section 2 | crate-graph -- `ambre_core::schema::SchemaMigration<D>` is generic over the document, and `ambre-core` depends on no CRDT: it is linked by `ambre-server`, which may never decode a document (`ZK-001`), so a migration written in the foundation has no type to write against and does not compile | none | verified |

### 1.2 Read a document's full state

A binary opens a stored document. Everything below happens before the caller touches a single
container, which is the only moment a wrong reading can still be caught.

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `SCHEMA-006` | `POST` | Every full-state read MUST compare the document's schema version with the binary's before using the document. | this spec section 2 | `ambre_prose::schema::tests::a_document_at_the_current_version_is_handed_back_untouched`; `ambre_toile::logic::schema::tests::a_board_at_the_current_version_is_handed_back_untouched`; `ambre_trame::logic::schema::tests::a_workbook_at_the_current_version_is_handed_back_untouched`; structural -- the gate is the first statement of every full-state entry point, the three engines' `import_snapshot` and the two `import_ambre_plaintext`, and the refusal and migration rows below each drive the comparison through a different verdict | none | verified |
| `SCHEMA-005` | `POST` | A document with no schema version MUST read as version 0. | this spec section 2 | `ambre_core::schema::tests::an_absent_register_reads_as_zero_and_zero_is_older_than_any_release`; `ambre_prose::schema::tests::a_document_written_before_the_register_migrates_to_the_current_version`; `ambre_toile::logic::schema::tests::a_board_written_before_the_register_migrates_to_the_current_version`; `ambre_trame::logic::schema::tests::a_workbook_written_before_the_register_migrates_to_the_current_version` -- end to end, on an input that carries no register at all | none | verified |
| `SCHEMA-007` | `POST` | A document older than the binary MUST be migrated before use. | this spec section 2 | `ambre_core::schema::tests::a_chain_runs_every_hop_in_order`; `::a_document_at_the_current_version_runs_nothing` -- over a three-hop registry, and the three application tests named in `SCHEMA-005` assert the register the migrated snapshot comes back carrying | none | verified |
| `SCHEMA-012` | `FRAME` | A read that refuses a document MUST NOT write it back. | this spec section 2 | `ambre_prose::schema::tests::a_document_from_a_newer_client_is_refused_and_changes_nothing`; `ambre_toile::logic::state::schema_tests::a_board_from_a_newer_client_is_refused_and_changes_nothing`; `ambre_trame::logic::schema::tests::a_workbook_from_a_newer_client_is_refused_and_changes_nothing` -- each compares the full snapshot before and after the refusal, not merely an element count; structural -- the gate reads the payload in a scratch document, so the live one is never written on the refusing path | none | verified |
| `SCHEMA-008` | `FAIL` | A document newer than the binary MUST be refused whole. | this spec section 2 | `ambre_prose::schema::tests::a_document_a_newer_client_wrote_is_refused_whole`; `ambre_toile::logic::schema::tests::a_board_from_a_newer_client_is_refused_whole`; `ambre_trame::logic::schema::tests::a_workbook_from_a_newer_client_is_refused_whole`; `ambre_core::schema::tests::the_walk_refuses_to_run_backwards` -- the back door: a chain asked to run downhill refuses rather than guessing | none | verified |
| `SCHEMA-013` | `FAIL` | A schema version register this build cannot read MUST be refused, never read as absent. | this spec section 2 | `ambre_prose::schema::tests::a_register_this_build_cannot_read_is_refused`; `ambre_toile::logic::schema::tests::a_register_holding_something_other_than_a_version_is_refused`; `ambre_trame::logic::schema::tests::a_register_holding_something_other_than_a_version_is_refused` -- the back door `SCHEMA-005` opens, where absent means 0 means migrate means open, so a register holding a string, or an integer outside `u32`, read as absent would open a document this build does not understand | none | verified |
| `SCHEMA-014` | `FAIL` | A payload the CRDT engine refuses MUST be refused before its schema version is read. | this spec section 2 | `ambre_prose::schema::tests::bytes_the_crdt_engine_refuses_never_reach_the_version_check`; `ambre_trame::logic::schema::tests::bytes_the_crdt_engine_refuses_never_reach_the_version_check`; `ambre_toile::logic::schema::tests::bytes_the_crdt_engine_refuses_never_reach_the_version_check` -- each hands the gate bytes no engine reads and requires the engine's own refusal, not a version verdict; structural -- the import into the scratch document is the first statement of `gate`, and the version is read off that same scratch, so the order is carried by the data flow rather than by a convention | none | verified |
| `SCHEMA-003` | `FAIL` | A record a binary cannot decode MUST reach the user as an error, never as a default or an absence. | this spec section 2; derived | `ambre_toile::logic::state::schema_tests::a_register_that_cannot_be_decoded_reaches_the_user`; `ambre_trame::logic::schema::tests::a_register_that_cannot_be_decoded_reaches_the_user`; `::a_board_this_build_wrote_raises_no_notice`; `::a_workbook_this_build_wrote_raises_no_notice` -- the last pair holds the notice back on every ordinary open | none | verified |

* **Budget.** none of its own, and one cost worth naming. A full-state read imports the payload
  twice: once into a scratch document to read the register, once into the live one. Leg 3 of
  `ambre-spec-budgets.md section 4` carries that cost, and carries no figure.
* **Boundary.** none. The version register is ordinary document state, so it travels sealed like
  every other container and the relay sees a blob (`ZK-001`). The protocol half lives in
  `ambre-core`, which the relay links, and nothing in it knows what a CRDT is.
* **Verification.** 19 `verified`, 1 `human-review`. A test holds the register: the
  number carried from creation, the check at each full-state entry, a case per verdict. A layout
  moved under an unchanged number turns a shape belt red. A reviewer reads `SCHEMA-017`. Two peers
  migrating at once keep both edits (`SCHEMA-020`).

---

## 2. MODEL

**CRDT containers.** one: a root map named `meta`, key `schema_version`, holding an unsigned
integer. The names are `ambre_core::schema::SCHEMA_META_MAP` / `SCHEMA_VERSION_KEY` so the three
applications cannot drift apart on spelling, and each declares the container in its own schema
document.

**Why the whole style is one register.** A style spread over one register per attribute lets two
concurrent edits **tear**. A fill change racing a stroke change converges to a style neither author
wrote. One register per unit makes every concurrent restyle resolve by last-writer-wins on a single
value, so every peer reaches one whole style.

**Where each application keeps it.** Trame stores it per sheet, keyed by the cell's stable
row and column identity; Toile stores it as the element's own register. Rich text is the stated
exception: a run is a sequence, not a register. Prose therefore carries no style register, and no
rule below speaks of one.

**What that costs, and what pays for it.** The binary encoding of a record tolerates no missing and
no extra field. A record written by a *newer* binary therefore fails to decode against this layout.
The fallback is unavoidable, because nothing else can stand in for an unreadable register. So
`SCHEMA-003` is about the word that goes with it, and the two applications that hold such a record
now say it.

| Application | On a failed decode | What the user sees |
|---|---|---|
| Trame | the cell is skipped during the reseed, and the reseed sets the flag | the cell loses its formatting, and one warning toast says formatting could not be read |
| Toile | the register reads as the default style, and the read sets the flag | the element is drawn plain, and one warning toast says the same |

The flag is a **fact, not a tally**. Every later read meets the same unreadable register again, so
a count would measure how often the document was redrawn. The import drains it, which is what keeps
the notice to one per open rather than one per frame. An **empty** register is not a failure. An
element with no style and a cell with no formatting carry no bytes. Reading those as the default is
the design, not a decode error.

**The version register.** A persisted document is still a valid CRDT whatever it holds. The engine
therefore imports it happily, then reads containers that no longer mean what it expects. A
successful open of a wrong document is silent loss. The register makes that case decidable.

* **Where.** A root map, written when the engine creates the document. It is ordinary document
  state, so it travels through every persistence path: the container file, the relay snapshot, the
  local store. It does not travel through the one path that happens to check it.
* **Absent means zero.** A document written before the register existed is version 0, so no existing
  document needs rewriting.
* **Present and unreadable is not absent.** The gate refuses a register holding something other
  than a `u32`: a string, a container, an integer that does not fit (`SCHEMA-013`). It is the back
  door the rule above opens: absent means 0 means migrate means open. Reading an unreadable
  register as absent would open a document this build does not understand, which is the one outcome
  the register exists to prevent.
* **When the gate reads it.** At every full-state entry point. That is `import_snapshot` in all three
  engines, for the local restore and a fixture load. It is also `import_ambre_plaintext` in the two
  engines that carry a container file. A
  delta carries no version and applies blind -- there is nothing in it to compare.
* **How the gate reads it.** It imports the payload into a **scratch** document and reads the
  register there. The import comes first, so bytes the engine refuses never reach the comparison
  (`SCHEMA-014`). Read the other way around, a failed import would show an absent register, which
  is version 0, which is a migration, which is an open. Only then does anything reach the live
  document: the payload itself when the
  versions match, a re-exported snapshot when a migration ran. That is what makes a refusal free
  (`SCHEMA-012`) and what the second import in the budget note above pays for.

| Comparison | What happens |
|---|---|
| equal | the document is used |
| the document is older | the application's migration chain runs, then the document is used |
| the document is newer | the open is refused, whole, and reported as a client that needs updating |

**The number is bound to a layout, by a frozen shape.** A version raised by hand is a promise,
and a layout that moves under an unchanged number is silent loss on every peer. So each
application writes the key set and the value types of its build to a `schema/shapes.txt` of its
own, one section per version. A belt in the crate compares the build against the section frozen
for its compiled number (`SCHEMA-016`). A moved key, a moved value type or a moved `postcard`
layout is then red until the number rises.

* **The section is append-only.** A developer whose build moved the shape could regenerate the
  section in place and watch the belt go green, with the number never raised. `just schema-freeze`
  reads the base revision and refuses a published section that changed.
* **The pairing is read, not trusted.** A crate that stamps a version and freezes nothing, and a
  frozen shape no belt compares, are each a half that holds nothing (`SCHEMA-015`). The same gate
  pairs them both ways, so a fourth application inherits the obligation rather than the list.
* **What a layout is.** The frozen text records shape, not meaning. A value read differently under
  the same key and type leaves it identical, so that half of the bump obligation stays with the
  reviewer.

**Two versions, and the reader checks both.**

* the container file carries its own version in its header, and that one guards the twenty-two
  bytes around the payload (`ambre-spec-docfile.md section 3`);
* the schema version guards what is inside;
* they are independent on purpose, and shaped alike so a reader carries one mental model.

**The limitation this does not solve.** Two peers on different client versions, editing one document
at the same time, each write their own schema's containers.

* they converge as CRDTs and diverge in meaning, and no register catches that;
* one deployment serves every client, so version skew lasts as long as an open tab and no longer;
* the relay authenticates sessions already, so a minimum-client-version gate needs nothing from the
  zero-knowledge boundary. That gate belongs to `ambre-spec-sync.md`, if live skew ever outlasts a
  tab;
* a file on a user's disk has unbounded age, which is why the asynchronous half gates the container
  Story instead of following it.

**Two peers migrating at once.** Each peer runs the step on its own copy. A step that creates a
container under a map key would mint a rival on each peer, and the map would keep one. So a migration
runs under a peer identity derived from the history it starts from, then the document takes its own
identity back (`SCHEMA-020`).

* two peers that start from one history write the same operations under the same identifiers;
* the CRDT engine keeps one copy of an operation it already holds, so there is no rival;
* two different histories derive two identities, so no identifier ever carries two contents;
* the step walks its containers in a fixed order, and writes nothing that depends on the clock;
* the identity is a hash of the history's heads, computed in `ambre-core` (`migration_peer`). It
  is written out there, not borrowed from a library, so a later build derives the same one.

Only the scratch document takes that identity. The live one imports the migrated snapshot under its
own, so the next edit a user types is theirs.

**Code.** `crates/ambre-core/src/schema.rs` for the protocol: the two container names, the three
verdicts, the error taxonomy and the registry walk. The walk is generic over the document, so the
foundation needs no CRDT.

`crates/ambre-prose/src/schema.rs`, `crates/ambre-trame/src/logic/schema.rs` and
`crates/ambre-toile/src/logic/schema.rs` for each application's current version, its registry and
its gate. The three registries each carry one row, `0 -> 1`, the register's own arrival. A document
written before it existed is identical in layout. The step therefore stamps the number and changes
nothing else, which is what makes it a migration and not a rewrite.
`crates/ambre-trame/src/logic/styles.rs` and
`crates/ambre-toile/src/logic/state/derive.rs` for the two style registers and for the decode notice
of `SCHEMA-003`. Prose appears in neither pair: it stores no style register, so it holds no record
a decode can fail on.

---

## 3. SURFACE

The user never sees the version. What they meet is what it protects them from.

| Trigger | Behaviour |
|---|---|
| opening a document written by an older client | it migrates and opens, with nothing to confirm |
| opening a document written by a newer client | it refuses to open and asks the user to update |
| two people restyling one cell at the same time | both converge on one whole style, never on a blend |
| a record the client cannot decode | the formatting is gone -- unavoidably -- and one warning toast per open says so (`SCHEMA-003`) |

---

## 4. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| One register for the whole style of a unit | one register per attribute, so that concurrent edits to different attributes both survive | the fan-out lets two concurrent edits converge to a style neither author wrote, and the tear is invisible until someone reads the result. One value per unit makes the race resolve as last writer wins over a whole style, which is a result someone did write |
| The version lives in the document, not in the carrier | a version field on each persistence carrier | a version held by the container file protects the file alone, and the same document also arrives through a relay snapshot and a local store. State that travels with the document is checked on every path by construction |
| Absent means version 0 | refusing a document that carries no version | it means rewriting every existing document to adopt the register, which is a migration whose only purpose is to enable migrations |
| A newer document is refused whole | reading a newer document on a best-effort basis and dropping what is unknown | a best-effort read of a schema the binary does not know is the silent-loss case the register exists to remove. Refusing costs the user an update; reading costs them the document |
| Migrations live in the application crate | a shared migration registry in the foundation | what a container meant in version 2 and means in version 3 is the application's own semantics, and the foundation owns the protocol instead -- where the number is, when it is read, what each comparison means |
| Mixed-version live collaboration is stated, not solved | a schema version on every delta, checked before it is applied | it needs a relay-side gate, and paying for that buys nothing while the skew is still bounded: one deployment serves every client, so the window is an open tab |
| The register is read in a scratch document, at the cost of a second import | importing into the live document and reading the register there, which is one map lookup and no second import | it reads the version off a document that is, by then, already merged into the one the user is looking at. The only way back out of a refusal is to drop their session, so the cheap read buys a refusal that destroys exactly what `SCHEMA-012` exists to protect. The cost lands on leg 3 of `ambre-spec-budgets.md section 4`, which carries no figure |
| A register holding something other than a `u32` is refused | reading it as absent, which is version 0, which migrates and opens | it is indistinguishable from a *newer* document whose register this build cannot parse, and opening one of those is the silent loss the register exists to remove. Refusing costs a reader of a genuinely corrupt document an error message; reading costs a reader of a newer one their document |
| An unreadable register raises one notice per import | a count of unreadable records, reported per document | the same register is met again on every later read, so the number measures repaints, not records. A count that drifts with the frame rate is worse than the plain fact, because a reader believes it |
| A migration writes under an identity its history derives | a container per body named at the root by its element | each peer inserts the same characters into the one shared text, and the body converges doubled |
| A migration writes under an identity its history derives | one elected peer migrates, and the others wait | a peer working offline would wait for a peer it cannot reach, and a document opened from a file has nobody to elect |
| The walk is generic over the document type | `ambre-core` takes a `loro` dependency and owns whole migrations | `ambre-server` links `ambre-core`, and a relay that can decode a document is the one thing the zero-knowledge posture forbids (`ZK-001`). Generic over `D` costs one type parameter and keeps the CRDT out of the foundation -- which is also what holds `SCHEMA-010` with no test |

---

## 5. NOT BUILT, AND NOT YET RULED

* **Nothing refuses a per-attribute register in a third application.** The two that store a style
  register are held by their shape belts, and `SCHEMA-015` now obliges a new crate to carry one.
  What the belt reads is a layout, so it would still take a reader to see a register per attribute
  in the lines it gained.
* **No rule covers what a migration may do to a document it cannot fully read.** The three
  registries hold one step each and none can fail, so nothing has forced the question yet. A step
  that *can* fail reports the hop it broke on (`SchemaError::MigrationFailed`), and the caller
  drops the scratch document. The reader therefore refuses the document whole rather than
  half-migrating it. That is the behaviour, and no rule states it.
* **A second unreadable register says nothing new.** The import drains the notice, so a document
  with one broken record and a document with a thousand read identically. Deliberate -- see the
  rejected alternative -- but it does mean the surface cannot express severity.
* **A meaning that moves under an unchanged layout still raises nothing.** The shape belts hold
  `SCHEMA-009` because a layout change moves a line of the frozen text. A value read differently
  under the same key and type leaves that text identical. The bump obligation on meaning
  therefore stays with the reviewer. Each application's schema document names that half.
* **Two peers who each edited the old version before migrating still race.** Their histories
  differ, so they derive two identities and mint rival containers. An edit typed into the losing
  one before they meet is lost. The common case, two peers opening one stored version, shares a
  history and loses nothing (`SCHEMA-020`).
* **The relay-side minimum-version gate has a name and no owner.** It is the only answer to live
  version skew, and no document holds a rule for it.

---

## 6. WHAT THIS DOCUMENT DOES NOT OWN

| Question | Owner |
|---|---|
| Which containers an application declares, and what each holds | that application's schema document |
| The container file, its header and its own version | `ambre-spec-docfile.md` |
| How a style reaches a register from a toolbar control | `ambre-spec-chrome.md` |
| What a snapshot is, and when one is written | `ambre-spec-delta-order.md` |
| Where a document is stored locally between sessions | `ambre-spec-launcher.md` |
| How a document is sealed before it is stored anywhere | `ambre-spec-sealed-envelope.md` |
| What the whole open of a document is budgeted at | `ambre-spec-budgets.md section 4` |
| What each application's current version is, and what its steps do | that application's schema document, and `logic/schema.rs` in its crate |
| How a refusal or a decode notice is worded and surfaced | that application's chrome (`notify_event`, in each crate's `chrome`) |
