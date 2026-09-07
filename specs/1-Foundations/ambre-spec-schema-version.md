# AMBRE -- FEATURE: THE PERSISTED SCHEMA

**Type:** Horizontal (framework)
**Implementation:** partial -- the one-register rule ships in both applications; the version
register, the read rule and the migration chain have no code.

> What a document looks like once it is written down, and how a later binary knows it can still read
> it. A stored document that opens into the wrong meaning is the worst failure available: it is
> silent.

Each application declares its own containers in its own schema document. This one holds the two
rules every application obeys, and the version that makes them safe to change.

---

## 1. CONTRACT

The only mandatory read. Grammar and status values are normative in `ambre-spec-template.md`. The
prefix is `SCHEMA`.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|---|
| `SCHEMA-001` | `INV` | A styled unit's whole style MUST be one binary register keyed by that unit's identity; a register per attribute is forbidden. | this spec section 2; `features/ambre-spec-trame-cell-styles.md` | `ambre_trame::logic::convergence_tests::concurrent_whole_cell_style_edits_do_not_blend_across_containers`, `::subset_style_edit_does_not_tear_a_whole_cell_rewrite`. Toile stores one register per element and no test drives two concurrent restyles of it | unverified |
| `SCHEMA-002` | `INV` | A shared formatting control MUST emit one changed field. | this spec section 2; `ambre-spec-chrome.md` | none. The four shared controls each emit one field, and nothing refuses a fifth that emits a whole style | unverified |
| `SCHEMA-011` | `INV` | A view MUST fold that field into a whole-register write. | this spec section 2; `ambre-spec-chrome.md` | none. It is the half that decides whether `SCHEMA-001` survives contact with the chrome: a view writing the field straight into a register of its own is the per-attribute layout the first rule forbids, arrived at from the other end. Nothing refuses one | unverified |
| `SCHEMA-004` | `INV` | Every document MUST carry its schema version in a root register, written when the document is created. | this spec section 2 | not built -- lands with `ARCH P8` | unimplemented |
| `SCHEMA-009` | `INV` | A change to the layout of a persisted record MUST raise the schema version. | this spec section 2 | not built -- lands with `ARCH P8` | unimplemented |
| `SCHEMA-010` | `INV` | A migration MUST live in the application crate that owns the schema it migrates. | this spec section 2 | not built -- lands with `ARCH P8` | unimplemented |

### 1.2 Read a document's full state

A binary opens a stored document. Everything below happens before a single container is used, which
is the only moment a wrong reading can still be caught.

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|---|
| `SCHEMA-006` | `POST` | Every full-state read MUST compare the document's schema version with the binary's before using the document. | this spec section 2 | not built -- lands with `ARCH P8` | unimplemented |
| `SCHEMA-005` | `POST` | A document with no schema version MUST read as version 0. | this spec section 2 | not built -- lands with `ARCH P8` | unimplemented |
| `SCHEMA-007` | `POST` | A document older than the binary MUST be migrated before use. | this spec section 2 | not built -- lands with `ARCH P8` | unimplemented |
| `SCHEMA-012` | `FRAME` | A read that refuses a document MUST NOT write it back. | this spec section 2 | not built -- lands with `ARCH P8`. It is the half `SCHEMA-008` carried, and it is the one that destroys data: refusing to open a newer document is an inconvenience, and writing an older binary's shape over it is the silent loss the whole register exists to prevent | unimplemented |
| `SCHEMA-008` | `FAIL` | A document newer than the binary MUST be refused whole. | this spec section 2 | not built -- lands with `ARCH P8` | unimplemented |
| `SCHEMA-003` | `FAIL` | A record a binary cannot decode MUST reach the user as an error, never as a default or an absence. | this spec section 2; derived | none, and both applications fail it. Toile answers a failed decode with the default style; Trame skips the cell and leaves it unstyled. Neither tells the user | unverified |

* **Budget.** none of its own. The version read is one map lookup inside leg 4 of
  `ambre-spec-budgets.md section 4`, and a migration chain is charged to the same leg.
* **Boundary.** none. The version register is ordinary document state, so it travels sealed like
  every other container and the relay sees a blob (`ZK-001`).
* **Verification.** 4 `unverified`, 8 `unimplemented`. The one rule with a real mechanism is
  `SCHEMA-001`, and it is held for one application out of two. Everything the version register
  promises is design: the failure it guards against -- a stored document read into the wrong meaning
  -- is today caught by nothing at all. Two conjunctions came apart. `SCHEMA-002` bound what a
  control emits to what a view does with it, and only the second half can undo `SCHEMA-001`: a view
  writing one field into a register of its own reaches the forbidden per-attribute layout from the
  other end. `SCHEMA-008` bound a refusal to a frame condition, and the frame is the half that
  destroys data -- refusing a newer document is an inconvenience, writing an older binary's shape
  over it is the silent loss.

---

## 2. MODEL

**CRDT containers.** one, and it does not exist yet: a root map named `meta`, key `schema_version`,
holding an unsigned integer. Each application declares it in its own schema document once
`ARCH P8` writes it.

**Why the whole style is one register.** A style spread over one register per attribute lets two
concurrent edits **tear**: a fill change racing a stroke change converges to a style neither author
wrote. One register per unit makes every concurrent restyle resolve by last-writer-wins on a single
value, so every peer reaches one whole style. Trame stores it per sheet, keyed by the cell's stable
row and column identity; Toile stores it as the element's own register. Rich text is the stated
exception: a run is a sequence, not a register.

**What that costs, and what pays for it.** The binary encoding of a record tolerates no missing and
no extra field, so a record written by an older binary fails to decode against a newer layout. The
two applications answer that failure differently today, and both answer silently, which is
`SCHEMA-003`.

| Application | On a failed decode | What the user sees |
|---|---|---|
| Trame | the cell is skipped during the rebuild | the cell loses its formatting |
| Toile | the record falls back to the default style | the element is redrawn plain |

**The version register.** A persisted document is still a valid CRDT whatever it holds, so the
engine imports it happily and then reads containers that no longer mean what it expects. A
successful open of a wrong document is silent loss. The register makes that case decidable.

* **Where.** A root map, written when the document is created. It is ordinary document state, so it
  travels through every persistence path -- the container file, the relay snapshot, the local store
  -- rather than through the one path that happens to check it.
* **Absent means zero.** A document written before the register existed is version 0, so no existing
  document needs rewriting.
* **When it is read.** At every full-state entry point: the snapshot import, and the snapshot leg of
  a local restore. A delta carries no version and applies blind.

| Comparison | What happens |
|---|---|
| equal | the document is used |
| the document is older | the application's migration chain runs, then the document is used |
| the document is newer | the open is refused, whole, and reported as a client that needs updating |

**Two versions, and both are checked.**

* the container file carries its own version in its header, and that one guards the twenty-two
  bytes around the payload (`ambre-spec-docfile.md section 3`);
* the schema version guards what is inside;
* they are independent on purpose, and shaped alike so a reader carries one mental model.

**The limitation this does not solve.** Two peers on different client versions, editing one document
at the same time, each write their own schema's containers.

* they converge as CRDTs and diverge in meaning, and no register catches that;
* one deployment serves every client, so version skew lasts as long as an open tab and no longer;
* the relay authenticates sessions already, so a minimum-client-version gate needs nothing from the
  zero-knowledge boundary. That gate is owed to `ambre-spec-sync.md` if live skew ever stops being
  bounded by a tab;
* a file on a user's disk has unbounded age, which is why the asynchronous half gates the container
  Story instead of following it.

**Code.** `crates/ambre-trame/src/logic/styles.rs` and
`crates/ambre-toile/src/logic/state/derive.rs` for the two style registers. The version register has
no code.

---

## 3. SURFACE

The user never sees the version. What they meet is what it protects them from.

| Trigger | Behaviour |
|---|---|
| opening a document written by an older client | it migrates and opens, with nothing to confirm |
| opening a document written by a newer client | it refuses to open and asks the user to update |
| two people restyling one cell at the same time | both converge on one whole style, never on a blend |
| a record the client cannot decode | the formatting disappears with no message, which `SCHEMA-003` forbids |

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

---

## 5. NOT BUILT, AND NOT YET RULED

* **The version register has no code.** Seven of the ten rules here describe it, and the fixture
  manifest records a version of 0 with a comment saying the register lands with `ARCH P8`.
* **A failed decode is silent in both applications, and differently.** One loses the record, the
  other replaces it with a default, and neither reports anything (`SCHEMA-003`).
* **Toile's style register has no convergence test.** The no-tear property is proven over Trame's
  cells alone, and Toile's element register is the same design with no proof.
* **Nothing refuses a per-attribute register.** `SCHEMA-001` is held by the two applications having
  been written that way, and a third would inherit no check.
* **No rule covers what a migration may do to a document it cannot fully read.** The chain is
  designed and unbuilt, and a partial migration has no stated outcome.
* **The relay-side minimum-version gate is named and unowned.** It is the only answer to live
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
