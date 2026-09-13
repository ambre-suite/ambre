# AMBRE -- SPEC TEMPLATES

**Type:** Template (two document kinds, one contract)

> Sections 0 and 1 are the contract both templates obey. Section 2 orders a **feature document**,
> section 3 a **schema document**: they differ by order, never by contract. Copy the shape from the
> working examples -- `specs/2-Trame/features/ambre-spec-trame-cell-merges.md` and
> `specs/2-Trame/ambre-spec-trame-schema.md`.

---

## 0. CODE IN SPECS (CONVENTION)

A spec states **what** and **why**; the source states **how**. A second, uncompiled copy of the code
drifts from it in silence.

| Block | Verdict |
| :--- | :--- |
| a type or data model -- `struct`, `enum`, a trait or API signature, a message shape, `#[derive(...)]` | **this is the contract.** It belongs here |
| an invariant, a pre or post-condition, an algorithm as numbered steps or pseudo-code | allowed: it declares, it does not execute |
| a short snippet or example test that clarifies intent | allowed as illustration; the authoritative suite is in the repo |
| a function body carrying real logic | **forbidden.** Replace it with the algorithm in steps, the contract it upholds, and a path into `crates/` |

**The test.** A block that compiles as behaviour is implementation. A shape, or an algorithm in
words, is not.

**Fences.** Every fence declares its language, and every fence closes.

| Fence | Holds |
| :--- | :--- |
| `rust` | Rust shapes and signatures, never a body |
| none, or `text` | a wire layout, pseudo-code, numbered steps |
| `mermaid` | a flow or a relationship, which the forge renders as a figure |
| `sql` `toml` `yaml` `css` `json` `html` `bash` | a genuine artefact of that language |

A runnable script (`lua`) is implementation, not a shape: state its algorithm in a `text` fence and
point to the source. A Rust shape in an untagged fence is a tagging bug, not an exemption.

**No ASCII art**, and printable ASCII only (`ai/ambre-writing-style.md`): a box glyph or a Unicode
arrow is a defect.

| To show | Use |
| :--- | :--- |
| a hierarchy | a nested bullet list |
| a sequence of steps | a numbered list |
| a mapping | a table |
| a flow or a relationship | a `mermaid` diagram, or a small graph plus prose when it is too dense |

`spec_code.py` fails on a body inside a `rust` fence, a Rust shape outside one, a tag off the list,
and an unterminated fence.

---

## 1. THE CONTRACT, SHARED BY BOTH TEMPLATES

**Ambre specifies by contract, in Bertrand Meyer's sense.** A component declares what the caller
owes before a call, what the component owes after it, and what holds at every instant in between --
each as a falsifiable rule, checked one at a time. Meyer named three of those, a fourth is what a
caller needs and no classical contract writes down, and Ambre adds four more: a document that
converges across peers, draws itself on a screen and survives a file round trip makes claims a
single-process contract has no word for. The eight are in section 1.B, and the vocabulary is closed.

### A. The header, and the first four lines

```text
# AMBRE -- FEATURE: [NAME]        (a schema: # AMBRE -- SPEC: [APP] CRDT SCHEMA)
**Type:** Vertical (feature) | Horizontal (framework) | Schema (persistence format)
**Implementation:** implemented | partial | design-only

> Two or three lines, in user terms: what this holds, and the problem it solves.
```

`Type` says what kind of document this is. `Implementation` is about **the code it describes**, and
it is what stops a 7-line stub reading like a shipped subsystem.

| Value | Means |
| :--- | :--- |
| `implemented` | the code exists and this document describes it. A divergence is recorded here, never discovered later |
| `partial` | part of the surface is built. The default while a document is under construction |
| `design-only` | no code beyond a stub. Every contract row is `unimplemented` and names the story that will build it |

One clause may follow the value, behind a ` -- `: `partial -- the grid behaviour is built; the file
round trip is not`. Nothing else may follow it, not even a full stop. `implemented.` is a fourth
spelling of a three-value vocabulary, and a gate that forgives it reads half the field.
`spec_shape.py` fails on any other value.

### B. The contract table

**Both templates carry one, and the grammar below is normative.** It is the shape of a spec, so it
lives here. It is a contract table and not an invariant table: an invariant is one of the eight
kinds below, and the other seven bind an operation, a pair of peers, a screen or a file. How a rule
is verified is `ambre-spec-testing.md`; the roll-up of every row's status is derived in
`ambre-contract-ledger.md`.

The state table, then one table per operation. The two kinds never share a table, so the example
carries both:

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Gap | Status |
|---|---|---|---|---|---|---|
| `XXX-001` | `INV` | The feature MUST ... | this spec section 2, row 1 | `ambre_crate::module::test_name`; `::second_test` | none | verified |
| `XXX-003` | `IO` | The feature MUST ... | this spec section 4 | not built | unbuilt -- lands with the `Grid surface` Story (`ambre-roadmap.md` E6) | unimplemented |

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Gap | Status |
|---|---|---|---|---|---|---|
| `XXX-005` | `POST` | The operation MUST ... | this spec section 4 | `ambre_crate::module::test_name` | none | verified |
| `XXX-002` | `FRAME` | The operation MUST NOT ... | README pillar "E2EE" | none | unheld -- which half of the rule is open, and what a mechanism would take | unverified |
| `XXX-004` | `FAIL` | Given [the condition the guard refuses], the operation MUST ... | this spec section 4 | `ambre_crate::module::guard_test` | none | verified |

* **Budget.** none: the three rows above are an example, so they cost nothing and cite no row of
  `ambre-spec-budgets.md`.
* **Boundary.** none: a template reaches no relay, holds no key and opens no socket.
* **Verification.** none: `contracts.py` skips this file, so no row of it is counted anywhere.

**The seven fields.**

| Field | Rule |
| :--- | :--- |
| `ID` | `XXX` is the prefix, declared once and **never renumbered**: it joins document, test name and audit finding. A feature spanning two layers carries two prefixes, and the gaps each leaves are the price |
| `Kind` | one of the eight below, **written in backticks** -- `` `INV` ``, never `INV`. It says what the rule binds: the state, or one step of one operation. The kind is a column and never a part of the id, so a row keeps its number when it moves between tables |
| `Statement` | RFC 2119 -- MUST, MUST NOT, SHOULD, SHOULD NOT, MAY. **A statement carrying none of them binds nothing**: it describes, and a description cannot be falsified, so it cannot carry a status. One rule per row, no conjunction |
| `Sources` | where the rule comes from, in one of five forms: **a section**, of this spec or of a named sibling (`this spec section 3`, `` `1-Foundations/ambre-spec-message-bus.md` (`BUS-018`) ``); a **`README pillar "Zero-Knowledge"`**, the word `README` included, because a bare `pillar "..."` names something a reader cannot grep for; an **``audit `H9` ``** finding, by its number; a **named `document.md`** with the rule id it cites; or **`derived`**, which claims no source but the design. A row that closes a finding cites it, and the finding stops drifting from the code in silence |
| `Verification` | the mechanism, and **nothing else**: one or more tokens of the closed set below, separated by `` ; ``. No argument. The cell answers one question -- what turns red when the rule breaks -- and that answer is a fact about CI, so it is finite and countable |
| `Gap` | what the mechanism does not hold: one of the six openers below, then the argument in prose. The cell answers the other question, which is a judgement and ages with the code. `none` is an answer and the only one that carries no prose |
| `Status` | one of the five values below. An empty cell fails |

**The mechanism tokens.** The closed set `Verification` draws on. Each token names a rung of the
ladder in `1-Foundations/ambre-spec-testing.md section 2`. That document owns the method and wins
where the two disagree; this table invents no rung of its own.

| Token | The rung it names |
| :--- | :--- |
| `` `ambre_crate::module::test_name` `` | the belt: a test that runs in a tier of `ambre-spec-testing.md section 4`. `Trame`, `Toile`, `Prose` or `chrome` may precede it when the same rule is held once per application |
| `` `::second_test` `` | a continuation of the path before it, in that path's module |
| `type-level` | newtypes, enums, typestate, `#[must_use]` -- the invariant holds if it compiles |
| `structural` | the construction is the proof: the state has no shape the violation could take |
| `exhaustive-match` | a closed enum matched with no `_` arm, so the next variant fails the build |
| `crate-graph` | the manifest does not reach the capability, so no call to it can be written |
| `target-build` | the build is the mechanism: the code compiles only under this flag, this manifest entry or this generated file, so the CI build fails without it |
| `lint-wall` | `[workspace.lints]`, `deny(unwrap_used, panic, indexing_slicing)`, `#![forbid(unsafe_code)]` |
| `cargo-deny` | a capability ban in `deny.toml`, held by the CI `deny` job |
| `proptest`, `cargo-fuzz`, `cargo-mutants`, `cargo-llvm-cov`, `criterion`, `insta` | the upper rungs, each written only where the mechanism runs in CI |
| `qa-manual` | the two places the doctrine declares mechanism-free: the wgpu pipeline, and the DOM glue |
| `human-review` | cryptography, line by line, by decision |
| `none` | no mechanism. Explicit, and never the same claim as an empty cell |
| `not built` | the code does not exist yet |

**Any item may carry a locating clause**, written `` type-level -- the id is a newtype ``. The clause
says where the mechanism is, and nothing more: it locates, and it argues nothing. On a path it names
the tier or the feature the test runs under. On a token it names the shape, or what the reviewer
reads.

**Seven tokens must carry one**: `type-level`, `structural`, `exhaustive-match`, `crate-graph`,
`target-build`, `qa-manual` and `human-review`. A path locates itself, and `cargo-deny` names one
file. These seven name a shape or a habit, so the bare token tells a reader nothing to go and read.
**Two tokens must not**: `none` and `not built` name no mechanism for a clause to locate. What
follows either one is an argument, and it belongs in `Gap`.

**The six gap openers.** The closed set `Gap` opens on. What follows an opener is prose, and it is
the one place in the row where an argument belongs.

| Opener | Means |
| :--- | :--- |
| `none` | the mechanism holds the whole rule, and nothing follows |
| `partial -- ` | the mechanism holds part of the rule; the prose says which part is left |
| `fails -- ` | the code breaks the rule today; the prose says how |
| `claim -- ` | what the row cites is an observation and not a mechanism: nothing turns red |
| `unheld -- ` | the code exists, nothing holds it; the prose says what a mechanism would take |
| `unbuilt -- lands with the ` | the rule is unbuilt, and the Story citation follows in the one form |

**Two rules across the columns**, which one merged cell could not express:

* **No mechanism and no gap fails.** `Verification` at `none` or `not built`, with `Gap` at
  `none`, claims a coverage the row does not have.
* **A `verified` row with a gap fails.** A status saying something turns red, beside a cell saying
  which half is loose, is two answers to one question. Move the status or close the gap.

**The five status values.**

| Value | Means |
| :--- | :--- |
| `verified` | something goes red if the rule breaks: a test, the compiler, a lint, `cargo deny` |
| `unverified` | the code exists and nothing would go red. **Legitimate and valuable.** Silence about coverage is the failure mode, not the absence of a test |
| `human-review` | nothing goes red automatically, and a reviewer is expected to catch it. By decision |
| `qa-manual` | caught by using the application |
| `unimplemented` | the code does not exist yet. **Counted apart**, never merged into `unverified`: one number is a risk, the other is planned work |

**The eight kinds.** A rule binds the state, at every instant, or one operation, at one point of
its run. `From` says which half of the grammar is Meyer's and which is this corpus's: the classical
three carry over unchanged, and the corpus had one word, `invariant`, for all eight.

| Kind | Binds | Holds | From |
| :--- | :--- | :--- | :--- |
| `INV` | the state | true before and after every operation, on one peer; a capability the component must not have is one | Meyer, the class invariant |
| `CONC` | the state, across peers | what two peers reach after concurrent operations | Ambre |
| `OBS` | the state, as the user draws it | what the screen shows for a given document | Ambre |
| `IO` | the state, across a file | what a read after a write returns | Ambre |
| `PRE` | one operation, before it runs | what the caller owes, and nothing checks | Meyer |
| `POST` | one operation, after it succeeds | what the operation owes | Meyer |
| `FRAME` | one operation, throughout | what the operation MUST NOT touch | the frame problem |
| `FAIL` | one operation, given input it refuses | the refusal, the notice, the rollback | Ambre |

**Where a kind sits.** A feature document opens its Contract on one table of state rows -- `INV`,
`CONC`, `OBS`, `IO` -- then carries one table per operation, under a `###` heading naming it. An
operation table holds the four operation kinds and no other, so a reader asking what one gesture
guarantees reads one table.

**The headings are numbered, and they name the gesture.** `### 1.1 The state`, then `### 1.2`
onward, one per operation, in the order the document introduces them: `### 1.3 Unmerge the
selection`, never `UnmergeActive` and never `1.3 Unmerge operation`. The number is what a citation
carries and what makes a missing operation visible in a table of contents; the user's words are
what section 1 owes its reader, per the rule above that an operation is named as the gesture and
not as the `fn` that runs it. No script holds this: `spec_shape.py` numbers the `##` sections and
does not read the `###` ones, so a document that drifts is caught in review or not at all.

**A document that owns no gesture carries no `###` heading at all**: its one state table sits
directly under section 1, and a lone `### 1.1` over it names nothing the section heading has not
already said. `ambre-spec-zero-knowledge.md` is that shape whole -- the boundary has no control of
its own, so its fourteen rows are fourteen `INV`, and the column's answer there is the confirmation
that none of them binds an operation.

**One completeness rule**, held by `contracts.py`: **an operation with a `POST` declares a
`FRAME`.** A frame condition is the rule nobody writes, and it is the one that catches an operation
which fixes its target and corrupts a neighbour. It is legitimately `unverified`: naming what an
operation leaves alone costs a row, and proving it costs an isolation assertion.

**`contracts.py` holds the grammar of a row's own cells.** Each of its three rules answers a drift
only a reading could catch. A bare `Kind` cell in 519 rows; six statements with no RFC 2119
keyword; twelve `Sources` cells citing an undeclared form. A vocabulary a gate compares loosely has
two spellings, and the second arrives in silence.

**One rule per row is a ceiling, not an error.** `FAIL` binds the refusal, the notice and the
rollback as one outcome, and `FRAME` names everything one operation must not touch. Both carry a
list by right; the other six kinds do not. A statement of theirs stating a second MUST after an
`and` is two rules under one status: the half nothing tests breaks in green. Telling a licensed
bundle from a hidden second rule is a reading, so `contracts.py` counts them per document in
`scripts/statement_ceilings.json`. The count may only fall, on the same terms as the `unverified`
ceiling: a growth is `just contracts-ceilings-raise`, one reviewed line of diff.

**Three rules a script cannot hold**, and they are what keeps two kinds from becoming one:

* **One guard, one row: `PRE` when nothing checks it, `FAIL` when something does.** The test is
  what happens when the caller breaks the rule. A defined refusal -- a rejection, a notice, an
  unchanged document -- is a `FAIL`, and the operation is total: it accepts any input and answers.
  Silent corruption is a `PRE`, and there is no `FAIL` to write because there is no defined
  failure. `SYNC-003` in `ambre-spec-sealed-envelope.md` is the second case whole: nothing can
  check that a nonce is fresh, and a reused one breaks the sealing without an error. Writing both
  rows for one condition states the guard twice: four of the thirty-five rows of
  `trame-cell-merges` were that duplicate, and each pair doubled one test into two `verified` rows.
  A feature document written in user terms rarely carries a `PRE`, because a defensive command
  handler refuses instead of corrupting; a document describing a crate's API commonly does.
* **A ban on a capability is an `INV`, or the `FRAME` of the operation that would breach it.**
  A rule whose subject is the crate graph, a type or the build binds the component at every
  instant and names no gesture, so it is an `INV`: `ZK-004`, "the server process MUST NOT link a
  crate able to decrypt", is true between two operations as much as during one. It is a `FRAME`
  only where **this document owns the operation** the ban applies to: `SYNC-049` binds what the
  blob upload may emit, and the document that owns that upload can state it as its frame
  condition. The choice is not free: a `FRAME` lives in an operation table, under
  the heading of the operation, and a table mixing it with a state kind fails, so a document owning
  no gesture has one answer. What makes such a row distinctive is its mechanism, not its kind, and
  the `Verification` cell already carries it: a crate graph, a `deny.toml` entry, a type that
  cannot hold the value. These rows are most of the E2EE and Zero-Knowledge pillars, and reading
  them as ordinary invariants is what hides that nothing runs to check them.
* **A `CONC` row states what two peers reach, never what one does.** The kind is otherwise a
  drawer for any rule about an operation the document does not own. A rule holding on a single peer
  is an `INV`, or a `POST` of the operation that produces it.
* **`OBS` is the rendering, never the gesture.** What the screen draws for a given document is
  `OBS`; what a click or a key does is an operation, with a table and a frame condition of its own.
  Merging the two is the collapse the column exists to undo, one level down.

**A `verified` row names the assertion, not only the test.** A test carrying five assertions can
back five rows, and the ledger then reports a coverage that no new assertion bought. When one test
holds several rows, each `Verification` cell says which assertion goes red for that row.
`contracts.py` reports the tests that carry more than one `verified` row; whether each row has its
own assertion stays a review act.

**The rules that keep the grammar honest.**

* **The falsifiability test.** If the way to falsify a statement cannot be stated, it is an intention,
  not a rule. Rejected: "sync is secure". Accepted: "no frame leaves the client with a plaintext
  payload".
* **One row per falsifiable consequence, never one per theme.** Five guarantees are five rows.
* **Every contract table declares the `Kind` column.** The migration that added it is over, so a
  table without it no longer reads as a document the column has not reached. `contracts.py` reads
  the header of every table and fails on a contract row under a header that declares none: the
  column is what holds the row to the vocabulary and to the two completeness rules, and a table
  missing it checks nothing at all.
* **Every contract table declares the `Gap` column.** That migration is over too. `contracts.py`
  fails on a contract row under a six-column header. The mechanism and what it leaves open are two
  cells, and one cell holding both is the shape neither grammar above can read.
* **A rule for code that does not exist yet is a row**, with `Status: unimplemented`, never prose held
  back. A numbered falsifiable statement is cheaper to read, to review and to revise than the
  paragraph it replaces, and it is the only form in which an unbuilt design can be checked against
  itself.
* **An `unimplemented` row names its story** in its `Gap` cell, as `specs/ambre-roadmap.md`
  writes it: `` unbuilt -- lands with the <Story> Story (`ambre-roadmap.md` <epic>) ``. It is the
  citation form every other mention takes. Never a section number, never a phase. **That story is
  not `[x]` until
  the row's status has moved.** Without the binding, `unimplemented` decays into the next `Draft`:
  true everywhere, informative nowhere. `contracts.py` resolves the name against the roadmap, so a
  story that is ticked, renamed or moved fails the build instead of leaving the row pointing nowhere.
* **Every other mention of a story is a citation, in one form**: `` `<Story>` Story
  (`ambre-roadmap.md` <epic>) ``, the whole of it on one line. A `Gap` cell answering a rule that
  fails today, a section 5 bullet, a rejected alternative -- all of them. Writing the name alone
  reads like a citation and resolves to nothing: the corpus carried 41 of them, and one of them
  named a story that had closed into the journal while the sentence still said "the roadmap's". The
  epic is not optional either: ten citations named the file and no epic, so the story could move
  under another epic and nothing failed. `contracts.py` refuses a bare name and a missing epic.
* **A `Verification` cell that says `none` carries a `Gap` that is not `none`.** The mechanism cell
  says what turns red; the gap cell says what is loose. One cell saying both is what let 78 rows
  read `verified` beside a reservation nobody could enumerate.
* **A cited mechanism must run in CI.** A test name resolving to a `fn` behind a Cargo feature no job
  enables is the same claim as a name resolving to nothing.
* **A row lives in the spec of the component it constrains**, next to the design that produced it,
  never in a central registry. It follows the code lifecycle, and it is read by whoever edits the
  component.
* **Extraction from existing code is a comparison, never a source.** A rule read off the code
  describes what the code does, not what it should do, and freezes current defects into
  specification. The spec and the README pillars stay the anchor; the code is inventory. Three
  outcomes, all useful: the code complies (locked against regression), the code violates (a defect
  found, the highest-value outcome), the code constrains beyond the spec (an undocumented decision to
  write down, or an accidental constraint to remove -- the most frequent).

**The three answers under the table.** A table states rules; it asks nothing. The three bullets ask,
and they sit directly under the table in every document that carries one.

| Label | Answers | An accepted answer |
| :--- | :--- | :--- |
| `Budget` | what this costs, and where the figure lives | a row of `ambre-spec-budgets.md`, cited; or `none`, and why |
| `Boundary` | whether anything here reaches the relay, holds a key or opens a socket | the rule that seals it; or `none`, and why |
| `Verification` | how much of the table is held, and by what | the counts per status, and the rows that are the gaps |

* **`none`, and why is the answer for most documents.** It is what the deleted `QUALITY`,
  `PERFORMANCE` and `SECURITY` sections answered badly, at a page each, and often not at all.
* `spec_shape.py` fails on a label that is missing, and on one whose bullet is empty. `contracts.py`
  fails when the counts the `Verification` answer opens on are not the counts of the table above it,
  and a status the answer leaves out claims zero. **Whether a `Budget` or a `Boundary` answer is
  true, no script holds** -- the exact status of the `Implementation` field above. The sentence that
  names the gaps stays a review act too. Forgetting becomes impossible; a false answer becomes
  visible in review.

### C. What no document carries

| Never | Why | Where it lives instead |
| :--- | :--- | :--- |
| a checkbox, a phase, an integration plan, a backlog | a document per schedule is a schedule per document, and "Phase 1" named four different things | `specs/ambre-roadmap.md`, one row per story |
| a milestone word (`MVP`) | a feature lives: it has an MVP form, then others. The increment is what carries the milestone | the roadmap, as a tag on a story |
| production history ("landed", "used to", "was fixed") | the document says what **is**, including for behaviour not yet built | git, and the `Status` column |
| a test list, or a coverage target | a list of intended tests has no join to the suite and no way to fail: 82 names were cited across the corpus and 23 resolved | the `Verification` column, one mechanism per rule |
| a tier declaration, a seam list, a Definition of Done | these belong to a crate, never to a feature, which may span two | `ambre-spec-testing.md section 6`, one row per crate; the tested done condition is a roadmap rule |
| a performance section | a budget is a rule, so it is a row | `ambre-spec-budgets.md`, cited by that row |
| a restatement of a shared contract | the second copy is the one that goes stale | the document that owns it, cited by name |
| an audit of a boundary it does not own | a ZK rule about this document's data is a row like any other | `ambre-spec-zero-knowledge.md`, refined by `ambre-spec-sync.md` and `ambre-spec-server.md` |

### D. The pillars, and what holds each one

A pillar without a script is decoration. The second column names the script that fails.

| Pillar | What holds it |
| :--- | :--- |
| One mandatory read: a reader may stop after the Contract | `spec_shape.py` -- prose is capped at 25 % of content words. An unmigrated spec carries a **count** of prose words instead, and it may only fall |
| No line reads as a wall | `spec_shape.py` -- a line over 110 columns, outside a table and a fence, fails: zero for a migrated document, a **count** that may only fall for an unmigrated one |
| The header says what the reader is holding | `spec_shape.py` -- the two fields present, and `Implementation` one of three values. `feature_inventory.py`, **asymmetric**: `implemented` beside an unbuilt row fails, while claiming less than the rows show is legitimate |
| A rule says what it binds | `contracts.py` -- a `Kind` cell outside the eight fails, and a table mixing a state kind with an operation kind fails: one table per operation is what makes a missing row visible |
| An operation declares its frame | `contracts.py` -- an operation table with a `POST` and no `FRAME` fails. **Whether the frame names the right state, no script holds**: that stays a review act, like the `Budget` and `Boundary` answers |
| One assertion is not two guarantees | `contracts.py` reports every test that backs more than one `verified` row. It is a **report**, not a failure: a test with five assertions legitimately backs five rows, and only a reader can say whether this one does |
| Every claim carries its status | `contracts.py` -- a row with no `Status`, or an unknown one, fails. `feature_inventory.py` -- a feature document that declares no contract table at all fails |
| The table's three questions are answered | `spec_shape.py` -- a document with a contract table carries `Budget`, `Boundary` and `Verification`, each non-empty. `contracts.py` -- the counts the `Verification` answer opens on are the table's own, nine documents having drifted from theirs. Whether the other two answers are true is a review act |
| A cited mechanism exists | `contracts.py` -- every `fn` a built row names must resolve in the workspace, per crate when the crate is named |
| The mechanism cell is a mechanism | `contracts.py` -- a `Verification` cell is read **whole**: every token of it is one of the closed set, or the row fails. Prose in that cell fails even when the prose is true, which is the point: a cell a gate reads half of is a cell nothing holds |
| The gap is stated, never implied | `contracts.py` -- a `Gap` cell opens on one of the six openers, and `none` is the only one carrying no prose. A row with no mechanism and no gap fails, and so does a `verified` row whose gap is not `none`: 78 rows read `verified` beside a reservation while the two answers shared one cell. A six-column header fails on every row under it, because neither grammar can read a cell holding both answers |
| A cited document exists | `spec_refs.py` -- a document is cited by its name, in the corpus and in the code alike, and that name must resolve to a file |
| A cited section exists | `spec_refs.py` -- a citation carrying `section N` must name a section the target document has. The number, never the letter after it. `§N` is the same coordinate |
| A coordinate is a corpus namespace | `spec_refs.py` -- **`crates/` carries no section number.** A name and a contract id are minted once; a section number is the one part a renumbering breaks, and the code held 906 that named no document at all. A comment cites the document, and the id where the rule has one |
| The numbering has no holes | `spec_shape.py` -- sections run from 0 or 1 upward, one step at a time, and `bis` is not a number. A deletion pays for itself the day it happens |
| Derived data is generated | `contracts.py` -- `ambre-contract-ledger.md` must match what the script would write. `feature_inventory.py` -- each application's inventory must match what it would write today |
| Shapes, never bodies | `spec_code.py` -- section 0 |
| Current state, never the steps | `spec_shape.py` -- a checkbox, a milestone word or a past-tense marker in a feature document fails |
| One schedule | `spec_shape.py` -- a checkbox list or a roadmap heading **in any spec** fails |
| An unbuilt row waits on an open story | `contracts.py` -- the story an `unimplemented` row lands with must resolve to one open row of `ambre-roadmap.md`. A name no row carries, a name two rows carry, a story already ticked and a stale epic each fail |
| Every story mention resolves | `contracts.py` -- a mention shaped `<name> Story` that names a row of the roadmap or the journal must carry `` (`ambre-roadmap.md` <epic>) `` on the same line, and the epic must be the one the story lives under. A bare name, a missing epic and a stale epic each fail |
| The coverage debt only falls | `contracts.py` -- a ceiling per document on its `unverified` rows, in `scripts/contract_ceilings.json`, zero for a document the file does not name. An unbuilt row is held by its story; a shipped row nothing would fail on was held by nothing, and adding one cost an author a regeneration. A growth is `just contracts-ceilings-raise`, one reviewed line. `human-review` and `qa-manual` carry no ceiling: they are decisions |
| The schema document stays a schema | `spec_shape.py` -- 600 prose words for the file, 40 for a container note |
| The schema matches the code | `spec_shape.py`, **asymmetric**: a container declared with no code is normal; one built in the code and absent from the table fails |
| One fact, one home | `spec_shape.py` -- a quality, performance or boundary-audit section fails, by title, everywhere but in the document that owns the fact. Otherwise review, plus the container check above |
| History lives in git | the past-tense check above |

* **A cap always names where the overflow goes.** A cap without a destination produces padding:
  sentences get cut in half, the counter improves and the reading gets worse.
* **The table is written before the prose.** Fill the rows, then write only what refused to become
  a row. The other order leaves a wall and a promise to tabulate it later.

---

## 2. TEMPLATE A -- A FEATURE DOCUMENT

One feature, one file, **including its surface**. The line is not the layer: a sentence **about**
this feature belongs here, a sentence that merely **names** it stays where it is.

Vertical or horizontal, same skeleton:

* **horizontal** when two applications or more share the behaviour -- undo, clipboard, presence,
  virtual scrolling, PDF export, convergence. It lives in `specs/1-Foundations/`;
* **vertical** otherwise, in the application's `features/`.

**One tightly coupled engine is one document**, whichever of the two it is: the twelve formula
features are the organs of one engine, so they are one file and not twelve. Coupling decides how
many documents, sharing decides which directory. Reading the two as one test files an engine only
one application has among the framework specs.

| Section | Holds | The rule |
| :--- | :--- | :--- |
| Header | the four lines of section 1.A | in user terms, never in types |
| 1. CONTRACT | the state table, then one table per operation under a `###` heading, then the three answers of section 1.B | **the only mandatory read.** It names no container, no type and no function: "a cell masked by a merge takes part in no computation", never "`masked_cells` is excluded from `reseed_engine`". An operation is named as the user's gesture, not as the `fn` that runs it |
| 2. WHAT EACH USER SEES | the two-peer table: A's operation, B's operation, the converged document, what each user sees, the test | a scenario is a row, never a story. The last column joins the example to its verification. Omit the section only when the feature holds no shared state |
| 3. MODEL | the containers it owns **cited by name**, the command and event shapes, the state fields, the algorithm in steps, then the code pointers | the design. It cites the schema document, never restates it. The shape is declared whether or not the code exists; a path into `crates/` is added once it lands |
| 4. SURFACE | every trigger, and everything the user touches: menus, keys, selection, rendering, the events that reach them | the machinery it moves through is specified elsewhere and cited, never respecified |
| 5. REJECTED ALTERNATIVES | one table: the decision, the alternative it refused, and why | **one row per alternative**, not per decision: a decision that refused two shapes states each on its own line, with the `Decision` cell repeated. `Decision` is a short anchor, never a restatement of the rule; `Why not` is uncapped, and it is the only place the measurement behind a refusal is written |
| 6. NOT BUILT, AND NOT YET RULED | what is broken, what is unbuilt, what no rule covers | a behaviour with no contract row is named here or it is invisible. This section makes a gap findable |

The Contract is the need, the Model is the design. Two blocks, never two files: splitting them
re-creates at ninety degrees the fragmentation this shape exists to remove.

**Where the prose goes when it leaves.** A spec is verbose less because it explains too much than
because its rules sit in the wrong place.

| Prose shape | Destination |
| :--- | :--- |
| "the system must / must never" | a contract row |
| "we chose X rather than Y" | the `Why not` cell of the alternative Y |
| "because / otherwise" for the shape kept | the design section that declares that shape, never a table of refusals |
| "Alice does X, Bob does Y, the result is Z" | a row in the two-peer table |
| "the function walks, then emits" | numbered steps plus a code pointer |
| "this is what Excel does" | a `Rejected alternative` row, with what it costs us |
| "as seen above", "in other words" | deleted |

---

## 3. TEMPLATE B -- A SCHEMA DOCUMENT

One per application, and hand-written: a generated inventory could list `cellstyle_{sheet}` but
never say why it is one register and not eleven, which is the decision that mattered most. A feature
cites a container **by its name**, which is unique, stable and verbatim in the code. A numeric id on
top of the name adds an indirection and buys nothing.

| Section | Holds | The rule |
| :--- | :--- | :--- |
| Header | the four lines of section 1.A | it owns identity, keys and merge granularity; it owns no gesture, no permission, no event |
| 0. SCOPE, AND THE MENTION RULE | the mention table below, and the two budgets with their overflow destinations | stated in the document, so the next writer meets it before adding to the file |
| 1. INVARIANTS | rows about the **format**: identity, key shape, what may never enter the document, the version duty, then the three answers of section 1.B | the prefix is the application's, shared with its other documents: one numbering, no renumbering. Every row is `INV`: a schema owns no gesture, so it owns no operation and declares no `PRE`, `POST`, `FRAME` or `FAIL` |
| 2. IDENTITY | the identity type, its rendering, its width, and the key that joins two halves | both halves of the shape are decisions, and what each refused sits in `6. REJECTED ALTERNATIVES` |
| 3. CONTAINERS | one row per container: name, Loro type, key, value, LWW unit, owning feature, `built` or `design` | the LWW unit is the column a reader comes for: it names what two concurrent writers race for. State what an absent key means |
| 4. HIERARCHY | the container tree as a **nested bullet list** | never a diagram, never ASCII art (section 0) |
| 5. CONTAINER NOTES | one bullet per container that needs a reason, **40 words** | only where a schema decision needs a reason. beyond 40 words the reason is an argument against a shape, and it moves to `6. REJECTED ALTERNATIVES` |
| 6. REJECTED ALTERNATIVES | the decision, the alternative it refused, and why | schema-level only, and one row per alternative. A decision about a gesture belongs to the feature |
| 7. FORMAT DUTIES | version, read, migration, bump, unknown field, size, deltas | one row per duty, each citing the rule or the row that holds it |
| 8. WHAT THIS DOCUMENT DOES NOT OWN | one row per question, and its owner | the cheapest section in the corpus, and the one that stops the schema becoming a dumping ground |

**Mention against specification.** A schema decision is often justified by an action: a row identity
survives a sort, a delete and a reorder. So an action appears as a **cause**, never as a **rule**.

| Position | Actions | Example |
| :--- | :--- | :--- |
| Container table, hierarchy list | forbidden | pure format |
| Container note, `Why not` of a refused alternative | **allowed, as a cause** | "a `mov` reassigns the element's op id, so identity must be the value, not the slot" |
| A normative row (MUST) | forbidden when it binds an action | "a merge MUST be refused while a covered cell is non-empty" belongs to the merges feature |

The mirror rule, on the feature side: **a feature may state a rule whose cause is a format; it may
never restate the format.**

---

## 4. WHAT IS NOT A DOCUMENT TEMPLATE

| File | Shape | Rule |
| :--- | :--- | :--- |
| `specs/ambre-roadmap.md` | epics, then one row per story: a checkbox, a name, its milestone tag, the features it touches | the only place anything is scheduled, and one file for every application: scheduling is cross-cutting, so a roadmap per application writes each cross-application dependency twice. A ticked story stays while its epic is open, which is what lets a reader say "eight of eleven, these three remain". The checkbox is legitimate here and nowhere else |
| `specs/ambre-roadmap-journal.md` | closed epics, cut from the roadmap verbatim the day they close | never planning |
| `specs/<app>/ambre-<app>-inventory.md`, one per application | name, scope, and a state derived from the status of the feature's contract rows | an aggregation with no design content, so it **is** generated, unlike the schema document. Never edited by hand: `feature_inventory.py` writes it, and fails when the file on disk is not what it would write |
| `specs/1-Foundations/ambre-spec-budgets.md` | one section per class of budget, one row per figure: the operation, the target, the thread, and the mechanism that measures it | the one register every budget of the workspace is a row of, so no spec carries a figure of its own. It states no rule, so it declares no contract table and carries no `Implementation` field. Hand-written, because a target is a decision: nothing can derive it. It is held to the prose cap and to the long-line rule like any document, and it is the reason `ambre-spec-architecture.md` needed no template of its own |

---

## 5. REJECTED ALTERNATIVES

The method this file declares is a design, so what it refused lives here, in the shape every
document uses. Each alternative was proposed against this corpus and measured before it was
refused. What was kept is stated in the sections above; this table holds only the roads not taken,
so that none is proposed a second time.

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| A citation names a document, plus a contract id | a code path and a decision id as two further namespaces, with a resolver script over the three | Most citations name a corpus document, so a code-path namespace makes `crates/` the source of truth for behaviour, and `README.md` gives that role to `specs/`. The decision half has nothing to read: no decision table in the corpus is numbered, and an application may carry no code at all -- `crates/ambre-prose/src/lib.rs` is 7 lines. A decision id of the form `TRAME-D-004` also matches the contract row pattern, so one script would count a decision as a rule |
| A feature's state is derived nowhere | a generated checkbox per feature, derived from the status of its contract rows | A story is a unit of production and spans several documents, and one story commonly carries a dozen rows. A state derived from those rows is coarser than the checkbox the roadmap already keeps, and it restates `ambre-contract-ledger.md` |
| A wall is wrapped by hand, one block at a time | a script that reflows prose to a column | Four corruptions pass the word-count proof a reflow script proposed: a lost list indent on a continuation line, an accidental bullet, a `//` comment that swallows a line inside a `rust` fence, and a split table row. A table row and a fenced line resist a wrap entirely, which is why the long-line check excludes both |
| `invariant` names one kind of row, not the table | keeping `invariant` as the word for the table, with the eight kinds as a refinement under it | Five of the eight kinds bind an operation and not the state, so calling every row an invariant said only that a row exists, which is what let a postcondition and a frame condition sit in one cell for years. The tooling follows the table: `contracts.py` writes `ambre-contract-ledger.md`, because a script named after one kind cannot report on eight |
| Eight kinds, and a capability ban is an `INV` | a ninth kind for the bans the build and the type system hold | It buys a label the `Verification` cell already carries -- `crate graph`, `deny.toml`, a type that cannot hold the value -- and it splits the vocabulary for a shape that is a fortieth of the corpus. `INV` is true of these rows in Meyer's own sense: they hold at every instant, between operations included |
| The kind is a column, and the id keeps its number | `TRAME-INV-001`, `TRAME-PRE-001`, one sequence per kind | An id joins a document to a test name and to an audit finding: the corpus carries 3 611 citations of one and `crates/` carries 46 more. Encoding the kind in the id renumbers every one of them, and it freezes the kind: `TRAME-027` moved from a state row to the frame condition of one operation during that migration, which a column makes free and an id makes a rename |
| `PRE` stays, and no rule pairs it with a `FAIL` | a completeness rule requiring a `FAIL` beside every `PRE` | What produced the duplicates was not the kind but the completeness rule behind it: requiring a `FAIL` beside every `PRE` manufactured a second row for a guard the operation already refuses, in all four cases in `trame-cell-merges` |
| `PRE` stays, and no rule pairs it with a `FAIL` | cutting `PRE` from the vocabulary | A precondition is the only place an unchecked obligation is written, and dropping the kind loses `SYNC-003`, which no code can check and whose breach is silent |
| The state kinds keep one table, the operation kinds get one each | a section per kind: `1.3 Concurrent`, `1.4 User-observable`, `1.5 File I/O` | It puts three rows under five headings and separates the two rules that describe one gesture. Grouping by operation is what makes the hole visible: a reader meets `MergeCells` and sees whether it declares what it does not touch |
| `Sources` stays a column | six columns without `Sources`, the audit joined by a hand-kept list | It is the join to `security-audit.md`, read by `contracts.py`, and the check runs both ways: a finding no row cites fails, and a row citing no finding of that number fails. A contract that drops it silently unbinds eight findings from the rules that close them |
| Two templates, one contract | a third template for the schema document | The schema document differs from a feature document by its section order, never by its contract: same header fields, same contract grammar, same ban on implementation. A third copy duplicates all three to change a table of contents. What it needs instead, and has, is its own budgets in `spec_shape.py` |
