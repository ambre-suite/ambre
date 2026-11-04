# AMBRE -- FEATURE: NAMED RANGES

**Type:** Vertical (feature)
**Implementation:** partial -- defining and using a name are built; five of its rules are not honoured.

> A user gives a range a name -- `TOTAL` for `A1:A3` -- and writes `=SUM(TOTAL)` instead of the
> coordinates. The name is stored with the document, so every collaborator reads the same one.

One container, two commands and one resolution step. It is small, and it is where five rules the
formula engine holds elsewhere stop holding. The same name resolves differently per reader, and a
rename breaks what a formula would have survived.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TRAME-062` | `INV` | A name MUST resolve to the same cells for every reader. | this spec section 2, row 3 | `ambre_trame::logic::tests::group1::unqualified_name_resolves_the_same_for_every_reader` | none | verified |
| `TRAME-066` | `INV` | A name MUST NOT be defined in terms of another name. | this spec section 3 | type-level -- the rebuild resolves every stored reference against an empty name table, and a bareword absent from it is a hard error, so the definition is dropped | none | verified |
| `TRAME-064` | `INV` | A formula using a name MUST read the cells the name covers now, not the values they held when it was defined. | this spec section 3 | `ambre_trame::logic::tests::group1::named_range_defines_resolves_and_deletes` -- it edits a covered cell and reads the new total | none | verified |
| `TRAME-067` | `INV` | A name reaching another sheet MUST survive that sheet being renamed. | this spec section 5 | `ambre_trame::logic::tests::group1::name_on_another_sheet_survives_its_rename` | none | verified |
| `TRAME-063` | `IO` | A name MUST survive closing and reopening the document. | this spec section 3 | `ambre_trame::logic::tests::group1::named_range_defines_resolves_and_deletes` -- it runs over a snapshot export and import | none | verified |
| `TRAME-065` | `OBS` | A formula reading a name that no longer exists MUST show an error. | this spec section 5 | `ambre_trame::logic::tests::group1::named_range_defines_resolves_and_deletes`; `ambre_trame::formula::parser::tests::undefined_name_is_error` | none | verified |
| `TRAME-068` | `CONC` | Two users defining the same name at once MUST converge on one definition. | this spec section 2, row 1 | `ambre_trame::logic::convergence_tests::two_peers_converge_under_interleaved_ops` -- proptest: concurrent defines and removals of one name, the named-range map compared on both peers | none | verified |

### 1.2 Give a range a name

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TRAME-069` | `FAIL` | A definition that is not a reference MUST be refused when it is made. | this spec section 5 | `ambre_trame::logic::tests::group1::definition_that_is_not_a_reference_is_refused` | none | verified |
| `TRAME-155` | `FAIL` | A definition whose name no formula can read MUST be refused when it is made. | this spec section 4 | `ambre_trame::logic::tests::group1::name_a_formula_cannot_read_is_refused` | none | verified |

### 1.3 Remove a name

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TRAME-156` | `FRAME` | Removing a name that was never defined MUST change nothing. | this spec section 3 | `ambre_trame::logic::tests::group12::removing_an_unknown_name_changes_nothing` | none | verified |

* **Budget.** none: a name resolves through one map lookup while a formula compiles, and
  `ambre-spec-budgets.md` budgets no name resolution.
* **Boundary.** none of its own: a name is a document write, and `ZK-001` seals every payload
  before it leaves the client.
* **Verification.** 10 `verified`. What holds is the definition. It is refused when it could not
  resolve. It is stored by sheet id, so every reader resolves one target whatever tab it looks at
  (`TRAME-021`). The convergence property drives concurrent definitions and removals of one name.

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
`ambre-spec-trame-schema.md section 3`. The key is the name, the value is the reference in A1 text,
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
3. A reference that fails to parse or to resolve is dropped, and the name is left absent. Only a
   document written elsewhere holds one: `DefineName` refuses it (`TRAME-069`).
4. A definition an older build stored unqualified binds to the first sheet of the strip, the one
   every reader agrees on (`TRAME-062`).
4. Compilation expands a bareword in a formula into the target the table holds, before any binding
   happens. A name is not a reference kind, it is a rewrite.

**Defining, in steps.** Both commands follow one shape:

1. For a definition, refuse a key the grammar would not read as a name (`TRAME-155`). The
   command carries a `Validated<DefinedName>`, which only a text with a name's shape makes. The grammar reads
   some such words as something else first, `A1` or `TRUE`, and those are refused here. Refuse a
   text that is not a reference to a live sheet (`TRAME-069`). A refusal answers with an error and
   writes nothing.
2. Store the reference qualified by a sheet id: the active sheet's for an unqualified one, the
   named sheet's id for a display name (`TRAME-062`, `TRAME-067`).
3. Write the container, and commit -- one command is one undo item.
4. Reseed the engine, so every formula using the name recomputes.
5. Send the delta to Sync, and a full data reset to the UI.

**Code.** `logic/sheets.rs` -- `rebuild_named`, `define_name`, `delete_name`;
`formula/parser/mod.rs` -- `resolve_refs`, which performs the expansion.

---

## 4. SURFACE

| Question | Answer |
|---|---|
| How a name is written | a bareword inside a formula: `=SUM(TOTAL)` |
| When the grammar tries it | last: after a function call, a qualified reference, an error literal and a plain reference |
| What a name may be called | an ASCII letter or underscore, then ASCII letters, digits and underscores: a `Validated<DefinedName>`. `DefineName` also refuses a word the grammar reads first (`TRAME-155`) |
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

* **No user can define a name.** The commands exist and the resolution works; the surface does not.
  Whoever builds it also has to decide what a name manager shows for the four rules above.
* **A name's reference is positional.** A formula reference names its row and column
  (`TRAME-228`), but a name stores `A1` text, read again on each reseed. An insert above its target
  moves the name off the cells it named, and a delete does not break it. Whether a name follows
  its cells is not ruled; storing it in the formula's form would make it so.
