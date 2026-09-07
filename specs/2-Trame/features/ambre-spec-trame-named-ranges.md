# AMBRE -- FEATURE: NAMED RANGES

**Type:** Vertical (feature)
**Implementation:** partial -- defining and using a name are built; five of its rules are not honoured.

> A user gives a range a name -- `TOTAL` for `A1:A3` -- and writes `=SUM(TOTAL)` instead of the
> coordinates. The name is stored with the document, so every collaborator reads the same one.

One container, two commands and one resolution step. It is small, and it is where five rules the
formula engine holds elsewhere stop holding: the same name resolves differently per reader, and a
rename breaks what a formula would have survived.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|---|
| `TRAME-062` | `INV` | A name MUST resolve to the same cells for every reader. | this spec section 2, row 3 | none, and the code does the **opposite**: an unqualified definition binds to engine sheet 0, which is whichever sheet the reader is looking at. The same `=SUM(TOTAL)` therefore totals a different sheet per open tab | unverified |
| `TRAME-066` | `INV` | A name MUST NOT be defined in terms of another name. | this spec section 3 | type-level: the rebuild resolves every stored reference against an empty name table, and a bareword absent from it is a hard error, so the definition is dropped | verified |
| `TRAME-064` | `INV` | A formula using a name MUST read the cells the name covers now, not the values they held when it was defined. | this spec section 3 | `ambre_trame::logic::tests::group1::named_range_defines_resolves_and_deletes`, which edits a covered cell and reads the new total | verified |
| `TRAME-067` | `INV` | A name reaching another sheet MUST survive that sheet being renamed. | this spec section 5 | none, and the code does the **opposite**: the definition is stored as the user typed it, so a display-name qualifier is never rewritten to a stable id the way a formula's is (`TRAME-034`), and a rename leaves the name unresolvable | unverified |
| `TRAME-063` | `IO` | A name MUST survive closing and reopening the document. | this spec section 3 | `ambre_trame::logic::tests::group1::named_range_defines_resolves_and_deletes`, over a snapshot export and import | verified |
| `TRAME-065` | `OBS` | A formula reading a name that no longer exists MUST show an error. | this spec section 5 | none, and the code does the **opposite**: the cell falls back to displaying its own source, so a reader sees `=SUM(TOTAL)` where a number stood. The test asserts that fallback | unverified |
| `TRAME-068` | `CONC` | Two users defining the same name at once MUST converge on one definition. | this spec section 2, row 1 | none. The container elects the whole name by last write, and no property drives a definition: the convergence op set carries no name command | unverified |

### 1.2 Give a range a name

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|---|
| `TRAME-069` | `FAIL` | A definition that is not a reference MUST be refused when it is made. | this spec section 5 | none, and the code does the **opposite**: an unparseable reference is stored, then silently dropped at each rebuild, so the name exists in the document and resolves nowhere | unverified |
| `TRAME-155` | `FAIL` | A definition whose name no formula can read MUST be refused when it is made. | this spec section 4 | none, and the code does the **opposite**: the command writes whatever key it is given, so `A1` or `1TOTAL` becomes a name the grammar of section 4 can never reach, and nothing checks the key against it | unverified |

### 1.3 Remove a name

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|---|
| `TRAME-156` | `FRAME` | Removing a name that was never defined MUST change nothing. | this spec section 3 | none. The command returns before the write when the key is absent, emitting no event and no delta, and no test drives an unknown name | unverified |

* **Budget.** none: a name resolves through one map lookup while a formula compiles, and
  `ambre-spec-budgets.md` budgets no name resolution.
* **Boundary.** none of its own: a name is a document write, and `ZK-001` seals every payload
  before it leaves the client.
* **Verification.** 3 `verified`, 7 `unverified`, 0 `unimplemented`. Five of the seven gaps are
  rules the code contradicts rather than rules nothing exercises, and `TRAME-062` is the one that
  matters: it breaks the engine's promise that a result does not depend on what the reader is
  looking at (`TRAME-021`). The two that are merely untested guard the two commands: a concurrent
  definition, and a removal of a name that was never there.

---

## 2. WHAT EACH USER SEES

| A does | B does | Converged document | What each user sees | Test |
|---|---|---|---|---|
| defines `TOTAL` as `A1:A3` | defines `TOTAL` as `B1:B9` | one definition, elected by last write | one range on both screens, and the other user's definition is gone with no notice | none (`TRAME-068`) |
| writes `=SUM(TOTAL)` | deletes `TOTAL` | the formula, and no name to reach | both read `=SUM(TOTAL)` as text where the total stood (`TRAME-065`) | `ambre_trame::logic::tests::group1::named_range_defines_resolves_and_deletes`, one peer |
| defines `TOTAL` as `A1:A3` while on `Sheet1` | reads `=SUM(TOTAL)` while on `Sheet2` | one definition, one text | different numbers, from the same document (`TRAME-062`) | none |
| edits `A2`, inside `TOTAL` | reads `=SUM(TOTAL)` | the edit | the same new total (`TRAME-064`) | the same test |

---

## 3. MODEL

**Container.** `named_ranges`, a workbook-level map declared in
`ambre-spec-trame-schema.md section 3`: the key is the name, the value is the reference in A1 text,
and the name is the unit two concurrent writers race for.

**Commands** (`ambre-bus`):

| Shape | Direction | Carries |
|---|---|---|
| `TrameCommand::DefineName { name, reference }` | UI to Logic | the name and its reference text. Redefining is the same command |
| `TrameCommand::DeleteName { name }` | UI to Logic | the name alone. An unknown name is a no-op that emits nothing (`TRAME-156`) |

**Resolving, in steps.** The in-memory table is derived, never stored:

1. Every reseed clears the table and reads the container.
2. Each stored reference is parsed as a formula and resolved with an **empty** name table, so a
   nested name fails (`TRAME-066`).
3. A reference that fails to parse or to resolve is dropped, and the name is left absent
   (`TRAME-069`).
4. Compilation expands a bareword in a formula into the target the table holds, before any binding
   happens -- so a name is not a reference kind, it is a rewrite.

**Defining, in steps.** Both commands follow one shape:

1. Write the container, and commit -- one command is one undo item.
2. Reseed the engine, so every formula using the name recomputes.
3. Send the delta to Sync, and a full data reset to the UI.

**Code.** `logic/sheets.rs` -- `rebuild_named`, `define_name`, `delete_name`;
`formula/parser/mod.rs` -- `resolve_refs`, which performs the expansion.

---

## 4. SURFACE

| Question | Answer |
|---|---|
| How a name is written | a bareword inside a formula: `=SUM(TOTAL)` |
| When the grammar tries it | last: after a function call, a qualified reference, an error literal and a plain reference |
| What a name may be called | a letter or underscore, then letters, digits and underscores. The grammar reads names; nothing holds a definition to it (`TRAME-155`) |
| What `TRUEVALUE` is | a name. A bareword merely starting with `TRUE` is not the boolean |
| Where a name may be used | any formula in the workbook. There is no per-sheet scope |
| How a user defines one | no way. No name manager, no dialog, no menu entry sends either command |
| What the formula bar shows | the name, never the coordinates: the expansion happens after the text is stored |

---

## 5. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| A name is stored as reference text | storing the row and column uids the name covered when it was defined | the document must survive a reload into an engine with no memory of the session, and a uid pair means nothing to a file a user may inspect. |
| A name is expanded before compilation | a `PushName` instruction resolved at evaluation | it makes names a third reference shape, needing its own binding rule, its own dirty rule and its own cycle rule. Expanded before compilation, the compiler, the graph and the machine handle exactly two. |
| A name cannot be defined in terms of another | resolving names transitively, with a depth limit | names would then form a graph of their own, needing a second cycle detector before the first one runs (`TRAME-066`). |
| The namespace is the workbook | sheet-scoped names, as the file formats carry | a name is a vocabulary a document agrees on; scoping it per sheet means two definitions of `TOTAL` that look identical in the formula bar. |
| Redefining is the same command as defining | a separate `RedefineName` | a name manager that distinguishes defining from redefining needs a "does it exist" round trip that races another writer. |
| Redefining is the same command as defining | a refusal when the name is taken | it turns an ordinary edit into an error the user has to clear before retyping what they meant. |

---

## 6. NOT BUILT, AND NOT YET RULED

* **The same name reads differently per tab** (`TRAME-062`). An unqualified definition resolves
  against engine sheet 0, and sheet 0 is the sheet being displayed. The cause is engine debt
  (`ambre-spec-trame-engine.md`); the consequence for a name is a wrong number, not a slow tab
  switch.
* **A rename breaks a name, and not a formula** (`TRAME-067`). A formula's sheet qualifier is
  rewritten to a stable id on the way in, which is why `TRAME-034` holds. A definition takes no such
  pass, so `Sheet2!A1:A3` stops resolving the moment `Sheet2` is renamed.
* **A dead name shows the formula, not an error** (`TRAME-065`). There is no `#NAME?` value in the
  error set, so an unresolvable name has nothing to display and the cell shows its own source.
* **Nothing refuses a bad definition** (`TRAME-069`, `TRAME-155`). `DefineName` writes whatever key
  and whatever text it is given. A bad reference is checked at every rebuild afterwards, where the
  only outcome is silence; a bad key is never checked at all, because the grammar that would reject
  it runs on formulas and not on definitions. Both leave a name in the document that resolves
  nowhere, and the second one cannot even be typed to find out.
* **No user can define a name.** The commands exist and the resolution works; the surface does not.
  Whoever builds it also has to decide what a name manager shows for the four rules above.
