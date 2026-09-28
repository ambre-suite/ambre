# AMBRE -- SPEC TEMPLATES

**Type:** Template (two document kinds, one contract)

> Sections 0 and 1 are the contract both templates obey. Section 2 orders a **feature document**,
> section 3 a **schema document**: they differ by order, never by contract. Copy the shape from the
> working examples -- `specs/2-Trame/features/ambre-spec-trame-cell-merges.md` and
> `specs/2-Trame/ambre-spec-trame-schema.md`.

**Changing this file is changing the corpus, and it starts with four questions.** Does this line let
a writer do something, or does it only explain? What breaks if it is deleted? Is it the history of
how the corpus got here, which section 1.F sends to git? And what is missing, the one question a
reader of the diff cannot ask. Section 5 holds the roads already refused.

**No gate declares a vocabulary of its own.** `scripts/template_vocabulary.py` parses this file at
import and hands out every closed set a gate reads. They are the vocabularies of a contract row: its
columns, its fields, the kinds, the statuses, the gap openers and the mechanism tokens. The three
answer labels, the `Implementation` values and the fence tags belong to the document around it. The
parser is anchored on the bold phrase opening each table, and it fails at import on a phrase
reworded, duplicated or counting wrong. A vocabulary table is therefore edited whole, heading
included: the heading counts its rows, and the count is checked.

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
| `sql` `toml` `yaml` `css` `json` `html` `ron` `bash` `sh` | a genuine artefact of that language |

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
owes before a call, what it owes after, and what holds at every instant in between. A document that
converges across peers, draws itself and survives a file round trip needs more. The vocabulary is
the eight kinds of section 1.C, and it is closed.

### A. The header, and the first four lines

```text
# AMBRE -- FEATURE: [NAME]        (a schema: # AMBRE -- SPEC: [APP] CRDT SCHEMA)
**Type:** Vertical (feature) | Horizontal (framework) | Schema (persistence format)
**Applications:** Prose, Trame, Toile      (a horizontal document whose rules each app holds)
**Implementation:** implemented | partial | design-only

> Two or three lines, in user terms: what this holds, and the problem it solves.
```

`Type` says what kind of document this is. `Implementation` is about **the code it describes**, and
it is what stops a stub reading like a shipped subsystem. `Applications` is optional: a horizontal
document declares it when some of its rules bind what each application does with the shared code.

| Value | Means |
| :--- | :--- |
| `implemented` | the code exists and this document describes it. A divergence is recorded here, never discovered later |
| `partial` | part of the surface is built. The default while a document is under construction |
| `design-only` | no code beyond a stub. Every contract row is `unimplemented` and names the story that will build it |

One clause may follow the value, behind a ` -- `: `partial -- the grid behaviour is built; the file
round trip is not`. Nothing else may follow it, not even a full stop. `implemented.` is a fourth
spelling of a three-value vocabulary, and a gate that forgives it reads half the field.
`spec_shape.py` fails on any other value.

**Each field opens its own line, and the title is unique in the corpus.** Two fields on one line
read as one field to a gate anchored on the line. A title is what a search result and a diff hang
the document on. The obvious names are the ones several documents reach for: `UNDO AND REDO`,
`PDF EXPORT`, `IMPORT AND EXPORT`.

**What disambiguates is free, and the corpus already does it two ways.** A domain word, where one
exists: `SELECTION ON THE BOARD` against `CARET AND SELECTION`. The application name otherwise,
which is what `THE TRAME ENGINE` and `TRAME CRDT SCHEMA` already use. A shared behaviour needs it
too: `TRAME PDF EXPORT` against the horizontal `PDF EXPORT` it is a half of. The horizontal document
keeps the bare name, because it is the one the others cite. `spec_shape.py` fails on a shared title
and on a field that shares a line.

### B. The contract table

What a row is made of: the columns a table declares, and what each cell may hold.

**The table is written before the prose.** Fill the rows, then write only what refused to become a
row. The other order leaves a wall and a promise to tabulate it later.

**Both templates carry one, and the grammar below is normative.** It is the shape of a spec, so it
lives here. How a rule is verified is `ambre-spec-testing.md`; the roll-up of every row's status is
derived in `ambre-contract-ledger.md`.

**Every contract table declares the `Kind` and the `Gap` columns.** `contracts.py` reads the header
of every table, and fails on a contract row under a header missing either. The kind holds the row to
the vocabulary and to the two completeness rules. The mechanism and what it leaves open are two
cells, and one cell holding both is a shape this grammar cannot read.

The state table, then one table per operation. The two kinds never share a table, so the example
carries both:

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `XXX-001` | `INV` | The feature MUST ... | this spec section 2, row 1 | `ambre_crate::module::test_name`; `::second_test` | none | verified |
| `XXX-003` | `IO` | The feature MUST ... | this spec section 4 | not built | unbuilt -- lands with the `Grid surface` Story (`ambre-roadmap.md` E6) | unimplemented |

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `XXX-005` | `POST` | The operation MUST ... | this spec section 4 | `ambre_crate::module::test_name` | none | verified |
| `XXX-002` | `FRAME` | The operation MUST NOT ... | README pillar "E2EE" | none | unheld -- what a mechanism would take; lands with the `...` Story (`ambre-roadmap.md` E0) | untested |
| `XXX-004` | `FAIL` | Given [the condition the guard refuses], the operation MUST ... | this spec section 4 | `ambre_crate::module::guard_test` | none | verified |

* **Budget.** none: an example table costs nothing and cites no row of `ambre-spec-budgets.md`.
* **Boundary.** none: a template reaches no relay, holds no key and opens no socket.
* **Verification.** none: `contracts.py` skips this file.

**The seven fields.**

| Field | Rule |
| :--- | :--- |
| `ID` | `XXX` is the prefix, declared once and **never renumbered**: it joins document, test name and audit finding. A new row takes the next free number of its prefix. **An id is never reused, and a prefix has no hole in its numbering**: `contracts.py` fails on both, so a row is narrowed or split, never deleted. A rule leaving the application for a shared document is the one exception, and its id is entered in the `RETIRED` register of `contracts.py` with the row that carries the rule today. A feature spanning two layers carries two prefixes, and the gaps each leaves are the price |
| `Kind` | one of the eight below, **written in backticks** -- `` `INV` ``, never `INV`. It says what the rule binds: the state, or one step of one operation. The kind is a column and never a part of the id, so a row keeps its number when it moves between tables |
| `Statement` | RFC 2119 -- MUST, MUST NOT, SHOULD, SHOULD NOT, MAY. **A statement carrying none of them binds nothing**: it describes, and a description cannot be falsified, so it cannot carry a status. One rule per row, no conjunction |
| `Sources` | where the rule comes from, `;` between one source and the next. Each source names its kind, its target and, where it has one, its section, in one of six forms: **a section** of this spec (`this spec section 3`); a **named `document.md`**, with the section it lands in and the rule ids it cites (`` `1-Foundations/ambre-spec-message-bus.md section 4` (`BUS-018`) ``); a **bare rule id**, which resolves through the ledger without naming the file that declares it (`` `ZK-001` ``); a **`README pillar "Zero-Knowledge"`**, the word `README` included, because a bare `pillar "..."` names something a reader cannot grep for; an **``audit `H9` ``** finding, by its number; or **`derived`**, which claims no source but the design. A source outside the six is refused on its own, whatever the sources beside it read. A row that closes a finding cites it, and the finding stops drifting from the code in silence |
| `Mechanism` | the mechanism, and **nothing else**: one or more tokens of the closed set below, separated by `` ; ``. No argument. The cell answers one question -- what turns red when the rule breaks -- and that answer is a fact about CI, so it is finite and countable. The `Verification` answer under the table asks a different question, and section 5 says why the column is the cell that carries the mechanism's name |
| `Gap` | what the mechanism does not hold: one of the six openers below, then the argument in prose. The cell answers the other question, which is a judgement and ages with the code. `none` is an answer and the only one that carries no prose |
| `Status` | one of the seven values below. An empty cell fails |

**The mechanism tokens.** The closed set `Mechanism` draws on. Each token names a rung of the
ladder in `1-Foundations/ambre-spec-testing.md section 2`. That document owns the method and wins
where the two disagree; this table invents no rung of its own.

| Token | The rung it names |
| :--- | :--- |
| `` `ambre_crate::module::test_name` `` | the belt: a test that runs in a tier of `ambre-spec-testing.md section 4`. `Trame`, `Toile`, `Prose` or `chrome` may precede it when the same rule is held once per application, below |
| `` `::second_test` `` | a continuation of the path before it, in that path's module |
| `type-level` | newtypes, enums, typestate, `#[must_use]` -- the invariant holds if it compiles |
| `structural` | the construction is the proof: the state has no shape the violation could take |
| `exhaustive-match` | a closed enum matched with no `_` arm, so the next variant fails the build |
| `crate-graph` | the manifest does not reach the capability, so no call to it can be written |
| `target-build` | the build is the mechanism: the code compiles only under this flag, this manifest entry or this generated file, so the CI build fails without it |
| `lint-wall` | `[workspace.lints]`, `deny(unwrap_used, panic, indexing_slicing)`, `#![forbid(unsafe_code)]` |
| `cargo-deny` | a capability ban in `deny.toml`, held by the CI `deny` job |
| `ci-gate` | a gate in `scripts/`, run by a `just` recipe in the CI `lint` job. The clause names the script |
| `proptest`, `cargo-fuzz`, `cargo-mutants`, `cargo-llvm-cov`, `criterion`, `insta` | the upper rungs, each written only where the mechanism runs in CI |
| `qa-manual` | the two places the doctrine declares mechanism-free: the wgpu pipeline, and the DOM glue |
| `human-review` | cryptography, line by line, by decision |
| `none` | no mechanism. Explicit, and never the same claim as an empty cell |
| `not built` | the code does not exist yet |

**Any item may carry a locating clause**, written `` type-level -- the id is a newtype ``. The clause
says where the mechanism is, and nothing more: it locates, and it argues nothing. On a path it names
the tier or the feature the test runs under. On a token it names the shape, or what the reviewer
reads.

**Eight tokens must carry one**: `type-level`, `structural`, `exhaustive-match`, `crate-graph`,
`target-build`, `ci-gate`, `qa-manual` and `human-review`. A path locates itself, and `cargo-deny`
names one file. These eight name a shape, a habit or one script of fifteen, so the bare token
tells a reader nothing to go and read.
**Two tokens must not**: `none` and `not built` name no mechanism for a clause to locate. What
follows either one is an argument, and it belongs in `Gap`.

**A rule each application holds names a test in each.** A test of the shared crate proves the
shared code, and never that an application calls it: `KINETIC-002` read `verified` while Toile broke
it. So a row binding what each application does labels its tests `Prose`, `Trame` or `Toile`, and
its document declares the `Applications` field. Such a row is `verified` only once every declared
application has its test; until then it is `untested`, and `Gap` names the ones missing.
`contracts.py` fails a labelled row whose document declares no field, a label the field leaves out,
and a `verified` row missing an application. A row with no label is the shared code's own rule, and
the reviewer holds which rows need one.

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

* **No mechanism and no gap fails.** `Mechanism` at `none` or `not built`, with `Gap` at
  `none`, claims a coverage the row does not have. The mechanism cell says what turns red, the gap
  cell says what is loose, and one cell saying both says neither.
* **A `verified` row with a gap fails.** A status saying something turns red, beside a cell saying
  which half is loose, is two answers to one question. Move the status or close the gap.
* **A `partial` row names its mechanism and opens its gap on `partial -- `.** An `untested` row
  opening on it fails. The rule is half held, and `untested` reads as a row nothing tests at all.
  A gap that holds nothing opens on `unheld -- `.

**The seven status values.** `Owes` is what the row must carry beside the status, and
`contracts.py` holds it.

| Value | Means | Owes |
| :--- | :--- | :--- |
| `verified` | something goes red if the rule breaks: a test, the compiler, a lint, `cargo deny` | the mechanism |
| `partial` | a mechanism holds part of the rule, and `Gap` says which part is left | the mechanism and a Story |
| `failing` | **the code breaks the rule today.** A live defect, and the only status that reports one | a Story |
| `untested` | the code exists, nothing holds it, and a mechanism could | a Story |
| `human-review` | nothing goes red automatically, and a reviewer is expected to catch it. By decision | nothing |
| `qa-manual` | caught by using the application | nothing |
| `unimplemented` | the code does not exist yet | a Story |

**Every gap that can close names its Story.** A row with no mechanism answers one question: can
this gap close? If it can, it is debt, and debt has an owner, so the row lands with a Story of
`ambre-roadmap.md`. If it cannot, it is a permanent property, and `qa-manual` and `human-review`
are the two words for that. The roadmap is then the whole debt register, which is where `AGENTS.md`
already puts every schedule.

### C. The eight kinds, and the shape of a table

Which table a row goes in, and which kind it takes.

**The eight kinds.** A rule binds the state, at every instant, or one operation, at one point of
its run. `From` says which half of the grammar is Meyer's and which is this corpus's.

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

**Where a kind sits.** A feature document opens its Contract on one table of state rows: `INV`,
`CONC`, `OBS`, `IO`. It then carries one table per operation, under a `###` heading naming it. An
operation table holds the four operation kinds and no other, so a reader asking what one gesture
guarantees reads one table.

**The headings are numbered, and they name the gesture.** `### 1.1 The state`, then `### 1.2`
onward, one per operation, in the order the document introduces them: `### 1.3 Unmerge the
selection`, never `UnmergeActive` and never `1.3 Unmerge operation`. The number is what a citation
carries, and what makes a missing operation visible in a table of contents. An operation is named
as the user's gesture, never as the `fn` that runs it.

**A document that owns no gesture carries no `###` heading at all.** Its one state table sits
directly under section 1, and a lone `### 1.1` over it names nothing the section heading has not
already said. `ambre-spec-zero-knowledge.md` is that shape whole: the boundary owns no control, so
every row of it is an `INV`.

**One completeness rule**, held by `contracts.py`: **an operation with a `POST` declares a
`FRAME`.** A frame condition is the rule nobody writes, and it is the one that catches an operation
which fixes its target and corrupts a neighbour. It is legitimately `untested`: naming what an
operation leaves alone costs a row, and proving it costs an isolation assertion.

**Four rules a script cannot hold**, and they are what keeps two kinds from becoming one:

* **One guard, one row: `PRE` when nothing checks it, `FAIL` when something does.** The test is
  what happens when the caller breaks the rule. A defined refusal (a rejection, a notice, an
  unchanged document) is a `FAIL`, and the operation is total: it accepts any input and answers.
  Silent corruption is a `PRE`, and there is no `FAIL` to write because there is no defined
  failure. `CANVAS-039` asks for a pointer position in the editor's own frame, and one in screen
  space moves the caret with no error. Writing both rows for one condition states the guard twice,
  and each pair doubles one test into two `verified` rows.
* **A ban on a capability is an `INV`, or the `FRAME` of the operation that would breach it.**
  A rule whose subject is the crate graph, a type or the build names no gesture and binds every
  instant, so it is an `INV`. It is a `FRAME` only where **this document owns the operation** the
  ban applies to. These rows are most of the E2EE and Zero-Knowledge pillars, and reading
  them as ordinary invariants is what hides that nothing runs to check them.
  * `ZK-004` says "the server process MUST NOT link a crate able to decrypt". It is true between
    two operations as much as during one, so it is an `INV`.
  * `SYNC-049` binds what the blob upload may emit, and the document that owns that upload states
    it as its frame condition.
  * The choice is not free: a `FRAME` lives in an operation table, so a document owning no gesture
    has one answer.
* **A `CONC` row states what two peers reach, never what one does.** The kind is otherwise a
  drawer for any rule about an operation the document does not own. A rule holding on a single peer
  is an `INV`, or a `POST` of the operation that produces it.
* **`OBS` is the rendering, never the gesture.** What the screen draws for a given document is
  `OBS`. What a click or a key does is an operation, with a table and a frame condition of its own.
  Merging the two is the collapse the column exists to undo, one level down.

### D. The rules that keep the grammar honest

What makes a row a rule at all, and where it lives.

* **The falsifiability test.** If the way to falsify a statement cannot be stated, it is an intention,
  not a rule. Rejected: "sync is secure". Accepted: "no frame leaves the client with a plaintext
  payload".
* **One row per falsifiable consequence, never one per theme.** Five guarantees are five rows.
* **One rule per row is an error, and the kind draws the line.** `FAIL` binds the refusal, the
  notice and the rollback as one outcome, and `FRAME` names everything one operation must not
  touch. Both carry a list by right; the other six kinds do not. A statement of theirs stating a
  second MUST after an `and` is two rules under one status: the half nothing tests breaks in green.
  `contracts.py` refuses one, so a row of the six other kinds carries one rule.
* **The refusal reads the keyword after the `and`, and a subject hides the row from it.** "MUST be
  dropped, and the others MUST be told" is the same defect with four words in it. `contracts.py`
  reports those rows rather than refusing them: a convergence rule states what both peers reach,
  from both ends, and it is one rule.
* **A rule for code that does not exist yet is a row**, with `Status: unimplemented`, never prose held
  back. A numbered falsifiable statement is cheaper to read, to review and to revise than the
  paragraph it replaces. It is the only form in which an unbuilt design can be checked against
  itself.
* **An `unimplemented` row names its story** in its `Gap` cell, as `specs/ambre-roadmap.md`
  writes it: `` unbuilt -- lands with the <Story> Story (`ambre-roadmap.md` <epic>) ``. It is the
  citation form every other mention takes, never a section number and never a phase. **That story
  is not `[x]` until the row's status has moved.** Without the binding, `unimplemented` decays into
  the next `Draft`: true everywhere, informative nowhere. `contracts.py` resolves the name against
  the roadmap. A story that is ticked, renamed or moved fails the build instead of leaving the row
  pointing nowhere.
* **Every other mention of a story is a citation, in one form**: `` `<Story>` Story
  (`ambre-roadmap.md` <epic>) ``, the whole of it on one line. A `Gap` cell answering a rule that
  fails today, a section 5 bullet, a rejected alternative -- all of them. Writing the name alone
  reads like a citation and resolves to nothing, and a story closes into the journal without the
  sentence noticing. The epic is not optional either: without it the story moves under another and
  nothing fails. `contracts.py` refuses a bare name and a missing epic.
* **A cited mechanism must run in CI.** A test name resolving to a `fn` behind a Cargo feature no job
  enables is the same claim as a name resolving to nothing.
* **A `verified` row names the assertion, not only the test.** A test carrying five assertions can
  back five rows, and the ledger then reports a coverage that no new assertion bought. When one
  test holds several rows, each `Mechanism` cell says which assertion goes red for that row.
* **A row lives in the spec of the component it constrains**, next to the design that produced it,
  never in a central registry. It follows the code lifecycle, and it is read by whoever edits the
  component.
* **Extraction from existing code is a comparison, never a source.** A rule read off the code
  describes what the code does, not what it should do, and freezes current defects into
  specification. The spec and the README pillars stay the anchor; the code is inventory. Three
  outcomes, all useful. The code complies, which locks it against regression, or the code violates,
  which is a defect found and the highest-value outcome. The code constrains beyond the spec, the
  most frequent: an undocumented decision to write down, or an accidental constraint to remove.

### E. The three answers under the table

**The three answers.** A table states rules; it asks nothing. The three bullets ask, and they sit
directly under the table in every document that carries one.

| Label | Answers | An accepted answer |
| :--- | :--- | :--- |
| `Budget` | what this costs, and where the figure lives | a row of `ambre-spec-budgets.md`, cited; or `none`, and why |
| `Boundary` | whether anything here reaches the relay, holds a key or opens a socket | the rule that seals it; or `none`, and why |
| `Verification` | how much of the table is held, and by what | the counts per status, and the rows that are the gaps |

**The three answers together are one paragraph, and one paragraph is six sentences.** That is
`prose.py`'s cap, and a reader sees the three bullets as one block. Six sentences under the 25-word
cap is **150 words for the three**, which is the budget. Two sentences each is the shape that fits.
A `Budget` that cites rows, a `Boundary` that names the sealing rule, a `Verification` that states
the counts and then the gap.

A `Verification` answer spends no sentence on the document's own history, which section 1.F sends
to git. Its counts are the table's own, and a status it leaves out claims zero.

**`none`, and why is the answer for most documents.** A label with nothing to say still answers,
because a missing answer and a `none` are not the same claim. What holds the three, and what no
script holds of them, is section 1.G.

### F. What no document carries

| Never | Why | Where it lives instead |
| :--- | :--- | :--- |
| a checkbox, a phase, an integration plan, a backlog | a document per schedule is a schedule per document, and "Phase 1" named a different thing in each | `specs/ambre-roadmap.md`, one row per story |
| a milestone word (`MVP`) | a feature lives: it has an MVP form, then others. The increment is what carries the milestone | the roadmap, as a tag on a story |
| history of any kind: the product's ("landed", "used to", "was fixed") and the document's own ("came apart", "the pass added", "was carrying", "is new") | the document says what **is**, including for behaviour not yet built. A sentence about how the document got here ages with no reader and no gate can tell whether it is still true | git, and the `Status` column. `spec_answers.py` counts the lines that do it |
| a test list, or a coverage target | a list of intended tests has no join to the suite and no way to fail: most such names resolve to nothing | the `Mechanism` column, one mechanism per rule |
| a tier declaration, a seam list, a Definition of Done | these belong to a crate, never to a feature, which may span two | `ambre-spec-testing.md section 6`, one row per crate; the tested done condition is a roadmap rule |
| a performance section | a budget is a rule, so it is a row | `ambre-spec-budgets.md`, cited by that row |
| a restatement of a shared contract | the second copy is the one that goes stale | the document that owns it, cited by name |
| an audit of a boundary it does not own | a ZK rule about this document's data is a row like any other | `ambre-spec-zero-knowledge.md`, refined by `ambre-spec-sync.md` and `ambre-spec-server.md` |

### G. The pillars, and what holds each one

A pillar without a script is decoration. The second column names the script and what fails it. The
third names what no script holds, so a reviewer reads one column and not thirty cells. The rule
itself is stated in the section that owns it, never here.

| Pillar | The script, and what fails | What no script holds |
| :--- | :--- | :--- |
| One mandatory read: a reader may stop after the Contract | `spec_shape.py` -- prose over 25 % of content words, this file included | -- |
| No line reads as a wall | `spec_shape.py` -- a line over 110 columns, outside a table and a fence | -- |
| The header says what the reader is holding | `spec_shape.py` -- a missing field, or `Implementation` outside the three values. `feature_inventory.py`, **asymmetric** -- `implemented` beside an unbuilt row; claiming less than the rows show passes | whether the clause after the value is true |
| A vocabulary is declared once, here | `template_vocabulary.py` -- a table of this file reworded, duplicated, or carrying a row count its heading does not state. It raises at import, so every gate that reads a vocabulary goes red | a count opening a bullet list, which no table check reads: moving one bullet between sections leaves it stating a number nothing holds |
| A rule says what it binds | `contracts.py` -- a `Kind` cell outside the eight, and a table mixing a state kind with an operation kind | whether the kind is the right one of the eight, per section 1.C |
| An operation declares its frame | `contracts.py` -- an operation table with a `POST` and no `FRAME` | whether the frame names the right state |
| One row, one rule | `contracts.py` -- a second MUST after an `and`, outside `FAIL` and `FRAME`. It **reports** the ones carrying a subject between the two, and fails on the rest | whether a reported row is a convergence rule, which states one thing from both ends |
| One assertion is not two guarantees | `contracts.py` **reports** a test backing two `verified` rows, and fails on none | whether each row has its own assertion |
| Every claim carries its status | `contracts.py` -- a missing or unknown `Status`. `feature_inventory.py` -- a feature document declaring no contract table | -- |
| An id is a join key | `contracts.py` -- a reused id, and a hole in a prefix's numbering outside the `RETIRED` register | -- |
| The table's three questions are answered | `spec_shape.py` -- a missing or empty answer. `contracts.py` -- counts that are not the table's own. `spec_answers.py` -- the three over the word budget | whether the `Budget` and `Boundary` answers are true |
| A cited mechanism exists | `contracts.py` -- a `fn` a built row names and the workspace does not hold, per crate when the crate is named | whether CI reaches it, and whether it asserts this rule |
| The mechanism cell is a mechanism | `contracts.py` -- a `Mechanism` cell is read **whole**, and any token outside the closed set fails, prose included | -- |
| The gap is stated, never implied | `contracts.py` -- a `Gap` outside the six openers, a row with no mechanism and no gap, a `verified` row whose gap is not `none`, a six-column header | whether the prose after the opener says what is actually loose |
| The source cell is a grammar | `contracts.py` -- a `Sources` cell is read **source by source**, and a source outside the six forms fails whatever sits beside it | -- |
| A cited document exists | `spec_refs.py` -- a document name, in the corpus or in the code, resolving to no file | -- |
| A cited rule exists | `contracts.py` -- an id no row declares, and a document named beside an id it does not declare | -- |
| A cited section exists | `spec_refs.py` -- a `section N` the target document does not have. The number, never the letter after it, and the section sign spells the same coordinate | whether the letter names the right subsection, and a coordinate spelling `5bis` reads as no coordinate at all |
| A cited code path exists | `spec_refs.py` -- a `crates/` path naming no file, and a `*` matching none | -- |
| A coordinate is a corpus namespace | `spec_refs.py` -- **`crates/` carries no section number and no line range**, a dated record excepted, being a quotation | whether a citation is narrow enough: the file, plus the symbol where it must be |
| The numbering has no holes | `spec_shape.py` -- a section number skipped, and `bis` is not a number. It reads the `##` sections and never the `###` ones | whether an operation heading names the gesture and not the `fn` |
| Every citation is an edge, and the edge resolves | `contract_index.py` -- an edge resolving to nothing, the README pillar included | -- |
| Derived data is generated | `contracts.py`, `contract_index.py` and `feature_inventory.py` -- a generated file that is not what the script would write today | -- |
| Shapes, never bodies | `spec_code.py` -- a body inside a `rust` fence, a Rust shape outside one, a tag off the list, an unterminated fence (section 0) | whether a `mermaid` block renders, which no gate reads |
| Current state, never the steps | `spec_shape.py` -- a checkbox, a milestone word or a past-tense marker in a feature document | -- |
| One schedule | `spec_shape.py` -- a checkbox list or a roadmap heading **in any spec** | -- |
| A gap that can close waits on an open story | `contracts.py` -- a story a `failing`, `untested` or `unimplemented` row names and the roadmap does not carry open: unknown, ambiguous, ticked or under a stale epic. `human-review` and `qa-manual` owe none | -- |
| Every story mention resolves | `contracts.py` -- a `<name> Story` mention carrying no `` (`ambre-roadmap.md` <epic>) `` on the same line, and a stale epic | -- |
| The schema document stays a schema | `spec_shape.py` -- 600 prose words for the file, 40 for a container note | -- |
| The schema matches the code | `spec_shape.py`, **asymmetric** -- a container built in the code and absent from the table; declared with no code is normal | -- |
| One fact, one home | `spec_shape.py` -- a quality, performance or boundary-audit section, by title, outside the document that owns the fact | whether a rule restates one another document owns |
| History lives in git | `spec_answers.py` -- a line writing the document's own history, capped at zero. `spec_shape.py` -- the past-tense check above | -- |

### H. The reading order of one row

**Building one rule is a read of seven things, in this order.** A row that cannot be read this way
is a row missing a citation. `spec_context.py` compiles the order, and `just spec-context <ID>`
prints it.
`just spec-context-story "<NAME>"` prints the same order for every row one story of the roadmap
lands, and quotes a shared block once.

| Read | What answers it |
| ---: | :--- |
| 1. What must be built | the row: its kind, its statement, its status |
| 2. What it must not change | the `FRAME` rows of its table |
| 3. The operation it belongs to | the other rows of that table, whole |
| 4. What design governs it | each section its `Sources` cite, verbatim |
| 5. What other rules it depends on | each row, pillar and finding its `Sources` cite |
| 6. How it is verified | the `Mechanism` cell |
| 7. What is missing | the `Gap` cell, and the story it lands with |

**The closure carries the sections the row cites, and never guesses one.** A design section reached
by the document's shape rather than by a citation is a section the next reader will not find. A row
whose `Sources` name only a rule, a pillar, a finding or `derived` has an empty closure. That is
where a builder needs it most, so it is a defect with an address.

---

## 2. TEMPLATE A -- A FEATURE DOCUMENT

One feature, one file, **including its surface**. The line is not the layer: a sentence **about**
this feature belongs here, a sentence that merely **names** it stays where it is.

Vertical or horizontal, same skeleton:

* **horizontal** when two applications or more share the behaviour -- undo, clipboard, presence,
  virtual scrolling, PDF export, convergence. It lives in `specs/1-Foundations/`;
* **vertical** otherwise, in the application's `features/`.

**One tightly coupled engine is one document**, whichever of the two it is. The twelve formula
features are the organs of one engine, so they are one file and not twelve. Coupling decides how
many documents, sharing decides which directory. Reading the two as one test files an engine only
one application has among the framework specs.

| Section | Holds | The rule |
| :--- | :--- | :--- |
| Header | the four lines of section 1.A | in user terms, never in types |
| 1. CONTRACT | the state table, then one table per operation under a `###` heading, then the three answers of section 1.E | **the only mandatory read.** It names no container, no type and no function: "a cell masked by a merge takes part in no computation", never "`masked_cells` is excluded from `reseed_engine`" |
| 2. WHAT EACH USER SEES | the two-peer table: A's operation, B's operation, the converged document, what each user sees, the test | a scenario is a row, never a story. The last column joins the example to its verification. Omit the section only when the feature holds no shared state |
| 3. MODEL | the containers it owns **cited by name**, the command and event shapes, the state fields, the algorithm in steps, then the code pointers | the design. It cites the schema document, never restates it. The shape is declared whether or not the code exists; a path into `crates/` is added once it lands |
| 4. SURFACE | every trigger, and everything the user touches: menus, keys, selection, rendering, the events that reach them | the machinery it moves through is specified elsewhere and cited, never respecified |
| 5. REJECTED ALTERNATIVES | one table: the decision, the alternative it refused, and why | **one row per alternative**, not per decision: a decision that refused two shapes states each on its own line, with the `Decision` cell repeated. `Decision` is a short anchor, never a restatement of the rule; `Why not` is uncapped, and it is the only place the measurement behind a refusal is written |
| 6. NOT BUILT, AND NOT YET RULED | what is broken, what is unbuilt, what no rule covers | a behaviour with no contract row is named here or it is invisible. This section makes a gap findable |
| 7. WHAT THIS DOCUMENT DOES NOT OWN | one row per question, and the document that owns it | **mandatory in a horizontal document, optional in a vertical one.** A horizontal document is what the corpus routes through, so the question it refuses is what sends a reader to the right file. A vertical document is reached by its feature name and rarely needs one. The rejected table refuses a shape; this refuses a subject |

The Contract is the need, the Model is the design. Two blocks, never two files.

**The last section is the cheapest in the corpus.** The reason is the same as in a schema document:
a question with no owner lands in whichever file mentions it first. Its number moves down when a
document omits the two-peer table, exactly like the four before it.

**Where the prose goes when it leaves.**

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

One per application, and hand-written. A generated inventory could list `cellstyle_{sheet}` but
never say why it is one register and not eleven, which is the decision that mattered most. A feature
cites a container **by its name**, which is unique, stable and verbatim in the code. A numeric id on
top of the name adds an indirection and buys nothing.

| Section | Holds | The rule |
| :--- | :--- | :--- |
| Header | the four lines of section 1.A | it owns identity, keys and merge granularity; it owns no gesture, no permission, no event |
| 0. SCOPE, AND THE MENTION RULE | the mention table below, and the two budgets with their overflow destinations | stated in the document, so the next writer meets it before adding to the file |
| 1. INVARIANTS | rows about the **format**: identity, key shape, what may never enter the document, the version duty, then the three answers of section 1.E | the prefix is the application's, shared with its other documents: one numbering, no renumbering. Every row is `INV` or `CONC`: a schema owns no gesture, so it owns no operation and declares no `PRE`, `POST`, `FRAME` or `FAIL`. A `CONC` row states what two concurrent writers converge on, and a schema carries no two-peer table: the scenario belongs to the feature document that owns the gesture |
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
| `specs/<N-Layer>/ambre-<layer>-inventory.md`, one per layer, the directory carrying the number and the file not | name, scope, and a state derived from the status of the feature's contract rows | an aggregation with no design content, so it **is** generated, unlike the schema document. Never edited by hand: `feature_inventory.py` writes it, and fails when the file on disk is not what it would write. The three applications carry one, and so does `1-Foundations`: it is the layer every other document routes through, so it is the one a reader asks "which file owns this subject" of |
| `specs/1-Foundations/ambre-spec-budgets.md` | one section per class of budget, one row per figure: the operation, the target, the thread, and the mechanism that measures it | the one register every budget of the workspace is a row of, so no spec carries a figure of its own. It states no rule, so it declares no contract table and carries no `Implementation` field. Hand-written, because a target is a decision: nothing can derive it. It is held to the prose cap and to the long-line rule like any document, and it is the reason `ambre-spec-architecture.md` needed no template of its own |
| `specs/ambre-glossary.md` | one row per term: the gloss, the synonyms it forbids, the document that defines it | a register, not a specification. It states no rule of its own, so it declares no contract table and carries no `Implementation` field. The gloss names an idea and never restates the rule, because a gloss carrying a condition is a second specification. `prose.py` reads the `Forbidden` column and fails on any word it names |
| `specs/1-Foundations/ambre-contract-ledger.md` | the roll-up of every contract row: the totals per status, then one row per document, then the index that joins an id to the file declaring it, one row per document and the ids folded into runs | generated by `contracts.py`, never edited. It reports state and carries no fact of its own, so no document cites a number from it. The index is the half a reader needs and a roll-up cannot give: an id in a code comment or a citation resolves to one file without a search. One line per id would be a ledger of thousands of lines, which is a file nobody opens |
| `specs/1-Foundations/ambre-contract-index.json`, and `ambre-contract-graph.json` beside it | one record per contract row -- every column as written, with the `Sources` cell parsed into its kinds, targets and sections -- then one edge per citation those records make | generated by `contract_index.py`, never edited. The ledger answers how much is verified and nothing else, so every other question about a row re-parsed the corpus; these two answer it from one file. Neither carries a line number: a document and an id locate a row, and a line is the part an edit above it breaks. Neither is authoritative, exactly as the ledger is not: the Markdown is, and a stale artefact fails rather than being read |
| `specs/1-Foundations/ambre-spec-testing.md` | the mechanism ladder, the regime per component shape, the three tiers, the two seams, the belt per crate | **the method, and the only place it lives.** It governs the `Mechanism` column of every document, so it declares no contract table of its own: a rule about how a rule is held would be held by itself. It is normative, and it wins where this file disagrees with it |
| `specs/audits/<name>.md` | numbered findings, each dated, each carrying what it costs and what closes it | a dated record, so it is out of the coordinate namespace: `spec_refs.py` reads no section number in it. It is a join target, and `contracts.py` checks the join both ways -- a finding no row cites fails, and a row citing no finding of that number fails. It is never rewritten to match the code; a finding closes, it does not disappear |
| `specs/Todo/ambre-<app>.md` | a sketch: the vision, the flow, the shape the application would take | **an application that has not started, and a sketch is not a specification.** It carries no contract table, no `Implementation` field and no section of either template, because the grammar costs more than it buys before a single row is falsifiable. Nothing cites it, and no gate reads it. The day the application starts, the sketch is replaced by a schema document and one feature document per feature; it is never migrated in place |

---

## 5. REJECTED ALTERNATIVES

The method this file declares is a design, so what it refused lives here, in the shape every
document uses. **This is the table to read before changing anything above.** What was kept is
stated in the sections above; this one holds only the roads not taken, so that none is proposed a
second time.

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| A citation names a document, plus a contract id | a code path and a decision id as two further namespaces, with a resolver script over the three | Most citations name a corpus document, so a code-path namespace makes `crates/` the source of truth for behaviour, and `README.md` gives that role to `specs/`. The decision half has nothing to read: no decision table in the corpus is numbered, and an application may carry no code at all -- `crates/ambre-prose/src/lib.rs` is a stub. A decision id of the form `TRAME-D-004` also matches the contract row pattern, so one script would count a decision as a rule |
| A feature's state is derived nowhere | a generated checkbox per feature, derived from the status of its contract rows | A story is a unit of production and spans several documents, and one story commonly carries a dozen rows. A state derived from those rows is coarser than the checkbox the roadmap already keeps, and it restates `ambre-contract-ledger.md` |
| A wall is wrapped by hand, one block at a time | a script that reflows prose to a column | Four corruptions pass the word-count proof a reflow script proposed: a lost list indent on a continuation line, an accidental bullet, a `//` comment that swallows a line inside a `rust` fence, and a split table row. A table row and a fenced line resist a wrap entirely, which is why the long-line check excludes both |
| `invariant` names one kind of row, not the table | keeping `invariant` as the word for the table, with the eight kinds as a refinement under it | Four of the eight kinds bind an operation and not the state, so calling every row an invariant says only that a row exists, and it lets a postcondition and a frame condition sit in one cell. The tooling follows the table: `contracts.py` writes `ambre-contract-ledger.md`, because a script named after one kind cannot report on eight |
| Eight kinds, and a capability ban is an `INV` | a ninth kind for the bans the build and the type system hold | It buys a label the `Mechanism` cell already carries -- `crate-graph`, `cargo-deny`, a type that cannot hold the value -- and it splits the vocabulary for a shape a handful of rows take. `INV` is true of these rows in Meyer's own sense: they hold at every instant, between operations included |
| The kind is a column, and the id keeps its number | `TRAME-INV-001`, `TRAME-PRE-001`, one sequence per kind | An id joins a document to a test name and to an audit finding, and the corpus and `crates/` cite ids in thousands of places. Encoding the kind in the id renumbers every one of them, and it freezes the kind: a row moving from the state table to the frame condition of an operation is free in a column and a rename in an id |
| The fifth column is `Mechanism`, and the third answer keeps `Verification` | renaming the answer instead, and leaving the column its old name | One word named three things: a column, an answer under the table and the CI gate. The column is the one that was wrong, because it holds a closed set of mechanism tokens and nothing else, and every script that reads it already says the mechanism in prose. The cheap rename was the other one, and cheapness is the whole of the argument for it: it would leave the corpus with a `Verification` column that holds no verification, and a `Coverage` answer nothing in the corpus calls coverage |
| `PRE` stays, and no rule pairs it with a `FAIL` | a completeness rule requiring a `FAIL` beside every `PRE` | What produced the duplicates was not the kind but the completeness rule behind it: requiring a `FAIL` beside every `PRE` manufactured a second row for a guard the operation already refuses |
| `PRE` stays, and no rule pairs it with a `FAIL` | cutting `PRE` from the vocabulary | A precondition is the only place an unchecked obligation is written, and dropping the kind loses `CANVAS-039`, which no code can check and whose breach is silent |
| The state kinds keep one table, the operation kinds get one each | a section per kind: `1.3 Concurrent`, `1.4 User-observable`, `1.5 File I/O` | It puts three rows under five headings and separates the two rules that describe one gesture. Grouping by operation is what makes the hole visible: a reader meets `MergeCells` and sees whether it declares what it does not touch |
| `Sources` stays a column | six columns without `Sources`, the audit joined by a hand-kept list | It is the join to `security-audit.md`, read by `contracts.py`, and the check runs both ways: a finding no row cites fails, and a row citing no finding of that number fails. A contract that drops it silently unbinds every audit finding from the rule that closes it |
| A source keeps the form the corpus writes | rewriting every source to `kind:target`, one machine-readable prefix per citation | The parser gains nothing: a full parse of the six prose forms yields the same kind, target and locator, and it yields them over every source in the corpus with no edit at all. The rewrite costs one edit per source, each unreviewable on its own, to make a reader decode a prefix before reading a citation they already read as English |
| A pointer keeps the form the corpus writes | a `code:` and a `test:` prefix, one declared kind per pointer | The kind is carried twice already. A `crates/` path is a path and `ambre_sync::pipeline::merge` is a Rust path, so the shapes do not collide; and the column says which promise is made, a `Mechanism` token naming a test that runs and prose naming a file to open. The prefix adds an edit per pointer, and a second thing to read before the citation |
| The `fn` check holds the mechanism cell alone | resolving every Rust path of the corpus, prose included | Prose legitimately names what no `fn` declares: a module (`ambre_toile::logic::state::comments`), a type (`FontConfig::system`), a method of a dependency (`LoroMovableList::mov`). Widened over the Rust paths outside a `Mechanism` cell, every failure the check reports is a true citation. What the cell promises is a test that runs, and that promise is what the check holds |
| The graph joins a row to what the row cites | an edge from a row to the container it constrains | Section 2 refuses it at the source: a Contract row names no container, no type and no function. A container is named by a `3. MODEL` section and belongs to a document, so such an edge has no tail to leave from, and `spec_shape.py` holds that table against the code already |
| A record locates its row by document and id | a line number per record, so a reader opens the file at the row | It is the coordinate this file refuses everywhere else, and it rots in the same way: an edit anywhere above a row rewrites that row's record while the row itself changes no word, and every such rewrite lands in a diff as a corpus change. An id is unique corpus-wide, and `ambre-contract-ledger.md` joins it to its file |
| Two templates, one contract | a third template for the schema document | The schema document differs from a feature document by its section order, never by its contract: same header fields, same contract grammar, same ban on implementation. A third copy duplicates all three to change a table of contents. What it needs instead, and has, is its own budgets in `spec_shape.py` |
