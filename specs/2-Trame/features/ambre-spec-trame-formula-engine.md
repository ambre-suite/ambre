# AMBRE -- FEATURE: FORMULA ENGINE

**Type:** Vertical (feature)
**Implementation:** partial -- the language, the graph and the evaluator are built; whole-axis ranges are not.

> A user types `=SUM(A1:A3)*2`. The cell shows a number. It keeps showing the right number after
> someone else edits `A2`, sorts the rows or renames the sheet the formula reads from.

Twelve behaviours here name organs, not features. A range needs a binding, a binding needs a graph,
a graph needs a cycle rule, and none of the four ships alone. They are one document. The function
catalogue and the named ranges are not organs of it, and have their own.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TRAME-006` | `INV` | A reference past the used range MUST read empty, never an error. | this spec section 3 | `ambre_trame::formula::compile::tests::ref_beyond_used_range_is_virtual`; `ambre_trame::formula::vm::tests::push_cell_virtual_beyond_used_range_reads_empty` | none | verified |
| `TRAME-007` | `INV` | A reference to a deleted row or column MUST read `#REF!`. | this spec section 4 | `ambre_trame::formula::vm::tests::push_cell_deleted_bound_yields_ref_error` | none | verified |
| `TRAME-208` | `INV` | Every operator MUST propagate an operand error rather than coerce it. | this spec section 4 | `ambre_trame::formula::vm::tests::every_operator_propagates_an_operand_error` -- it covers eleven operator shapes; `::unary_not_propagates_an_operand_error` -- the prefix form | none | verified |
| `TRAME-013` | `INV` | No text typed into a cell MUST make the engine panic. | this spec section 3 | `ambre_trame::formula::prop_tests::parser_never_panics_on_arbitrary_text`; `::parser_never_panics_on_formula_noise`; `::compile_emits_balanced_bytecode`; `::vm_is_total_and_deterministic` -- four proptests; `ambre_trame::formula::parser::tests::a_formula_past_a_bound_is_refused_before_it_recurses`; `::the_worst_admitted_formula_fits_a_small_stack`; cargo-fuzz -- `fuzz/fuzz_targets/formula.rs`, run by the CI `fuzz` job | none | verified |
| `TRAME-021` | `INV` | A formula result MUST NOT depend on what the reader is looking at. | this spec section 5 | `ambre_trame::logic::tests::group5::shared_filter_does_not_affect_sum`; `ambre_trame::logic::tests::group3::masked_cell_excluded_across_sheets_regardless_of_active_tab` | none | verified |
| `TRAME-034` | `INV` | A reference to another sheet MUST survive that sheet being renamed. | this spec section 3 | `ambre_trame::formula::render::tests::canonicalize_name_to_id`; `::display_id_to_name`; `ambre_trame::formula::parser::tests::id_qualified_cell_resolves_index`; `::sheet_qualified_resolves_index` | none | verified |
| `TRAME-228` | `INV` | A stored formula MUST name each reference by the row and column identities it was typed against, never by a position. | this spec section 5 | not built | unbuilt -- lands with the `References stored by identity` Story (`ambre-roadmap.md` E6) | unimplemented |
| `TRAME-229` | `CONC` | A reference typed while a peer inserts or deletes a line MUST read, after the merge, the cell it was typed against. | this spec section 5 | not built | unbuilt -- lands with the `References stored by identity` Story (`ambre-roadmap.md` E6) | unimplemented |
| `TRAME-226` | `IO` | A workbook storing positional references MUST open with each one reading the cell it read when saved. | this spec section 5; `TRAME-025` | not built | unbuilt -- lands with the `References stored by identity` Story (`ambre-roadmap.md` E6) | unimplemented |
| `TRAME-036` | `INV` | Operators MUST bind in the order a spreadsheet user expects. | this spec section 4 | `ambre_trame::formula::parser::tests::arithmetic_precedence`; `::parens_override_precedence`; `::comparison_chain`; `ambre_trame::formula::vm::tests::precedence` | none | verified |
| `TRAME-037` | `INV` | The power operator MUST group to the right. | this spec section 4 | `ambre_trame::formula::parser::tests::power_right_associative`; `ambre_trame::formula::render::tests::pow_right_associative` | none | verified |
| `TRAME-038` | `INV` | A cell holding plain text MUST hold no compiled formula and no dependency. | audit `H7`; this spec section 3 | `ambre_trame::formula::cache::tests::non_formula_clears_cache`; `ambre_trame::formula::dag::tests::clear_deps_drops_the_formula_and_its_now_orphan_node` | none | verified |
| `TRAME-046` | `INV` | An empty cell MUST compare as the zero of whatever it is compared with. | this spec section 4 | `ambre_trame::formula::vm::tests::an_empty_operand_compares_at_its_peer_zero`; `::booleans_order_false_before_true` | none | verified |
| `TRAME-047` | `INV` | Two numbers MUST compare on a relative tolerance, never an absolute one. | this spec section 5 | `ambre_trame::formula::vm::tests::equality_is_relative_not_absolute` | none | verified |
| `TRAME-044` | `INV` | A range too large to hold in memory MUST read `#REF!`. | this spec section 3 | `ambre_trame::formula::vm::tests::eval_range_over_cell_cap_yields_ref_error` | none | verified |
| `TRAME-209` | `INV` | A range over that cap MUST NOT be materialized. | this spec section 3 | none | unheld -- the test above reads the error value back, which a run that allocated the million cells and then returned the error would return too. Nothing weighs the allocation; lands with the `The Trame belts` Story (`ambre-roadmap.md` E9) | untested |
| `TRAME-210` | `INV` | The cells a single recompute materializes for ranges MUST be bounded across the whole pass, not one range at a time. | this spec section 6 | `ambre_trame::formula::vm::tests::ranges_of_one_pass_share_one_budget` | none | verified |
| `TRAME-042` | `INV` | A loop closed through a range, a name or another sheet MUST be caught while evaluating. | this spec section 3 | none | unheld -- the guard is the evaluation stack in `formula/vm.rs`, and every cycle test drives the static graph instead, so no test closes a loop the graph cannot see; lands with the `The Trame belts` Story (`ambre-roadmap.md` E9) | untested |
| `TRAME-048` | `INV` | A whole-column or whole-row range MUST cover the used part of its axis. | this spec section 6 | not built | unbuilt -- lands with the `Engine completion` Story (`ambre-roadmap.md` E6) | unimplemented |
| `TRAME-033` | `OBS` | A formula shown back to its author MUST carry only the parentheses its meaning needs. | this spec section 4 | `ambre_trame::formula::render::tests::precedence_minimal_parens`; `::pow_right_associative` | none | verified |
| `TRAME-045` | `OBS` | Every evaluation failure MUST show a named error, never a blank and never a number. | this spec section 4 | `ambre_trame::formula::vm::tests::division_by_zero`; `ambre_trame::formula::engine::tests::division_by_zero_displayed`; `ambre_trame::formula::types::tests::cell_value_display` | none | verified |
| `TRAME-032` | `IO` | A formula the engine writes back MUST read as the same formula. | this spec section 3 | `ambre_trame::formula::prop_tests::render_parse_render_is_a_fixpoint` -- a proptest over every node but `SheetRef` and `Name`; `ambre_trame::formula::parser::tests::error_literal_reparses`; `ambre_trame::formula::shift::tests::ref_error_text_is_reparseable` | none | verified |

### 1.2 Type into a cell

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TRAME-211` | `PRE` | A sheet a formula qualifies MUST already exist when the formula is typed. | this spec section 3 | `ambre_trame::logic::tests::group3::formula_naming_a_missing_sheet_is_refused` | none | verified |
| `TRAME-011` | `POST` | Typing into a cell MUST compile that cell again. | this spec section 3 | `ambre_trame::formula::cache::tests::stale_cell_recompiled`; `::non_formula_clears_cache` | none | verified |
| `TRAME-040` | `POST` | An edit MUST update every cell that reads it, directly or through a range. | this spec section 3 | `ambre_trame::formula::dag::tests::simple_dependency_propagation`; `::range_dirty_collection`; `::collect_dirty_reaches_a_fixpoint_over_chained_ranges`; `ambre_trame::formula::engine::tests::cascade_after_edit`; `::range_dependency_propagates_through_a_chain` | none | verified |
| `TRAME-041` | `FRAME` | An edit MUST NOT recompute a cell that reads nothing it changed. | this spec section 3 | `ambre_trame::formula::dag::tests::range_outside_no_dirty`; `ambre_trame::formula::cache::tests::not_stale_no_recompile` | none | verified |
| `TRAME-212` | `FRAME` | Retyping the text a cell already holds MUST NOT recompile it. | this spec section 6 | `ambre_trame::formula::engine::tests::retyping_the_same_text_does_not_recompile` | none | verified |
| `TRAME-012` | `FAIL` | A formula that reads itself through other cells MUST be refused with its culprits named. | this spec section 3 | `ambre_trame::formula::dag::tests::static_cycle_detected`; `::cycle_detection_kosaraju_returns_culprits`; `::rejected_cycle_leaves_no_stale_range_deps` | none | verified |
| `TRAME-213` | `FAIL` | A formula that reads its own cell directly MUST be refused. | this spec section 3 | `ambre_trame::formula::dag::tests::self_reference_rejected`; `ambre_trame::formula::engine::tests::self_reference_rejected` | none | verified |
| `TRAME-035` | `FAIL` | A reference to a sheet that never existed MUST be refused, not evaluated. | this spec section 4 | `ambre_trame::formula::parser::tests::unknown_sheet_name_is_parse_error`; `::unknown_sheet_is_error` | none | verified |
| `TRAME-039` | `FAIL` | A refused formula MUST replace the cell's previous result, never leave it on screen. | this spec section 3 | `ambre_trame::formula::engine::tests::parse_error_replaces_the_stale_value`; `::cycle_replaces_the_stale_value_with_an_error` | none | verified |

### 1.3 Delete a row, a column or a sheet

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TRAME-008` | `POST` | A row or column delete MUST propagate `#REF!` to every formula that read the deleted line. | this spec section 6 | none | fails -- the delete path rebinds every formula from its raw text, so the reference lands positionally on the shifted grid. `ambre_trame::logic::tests::group2::formula_reference_is_positional_after_delete` asserts `=A2` reads empty after deleting row 1; lands with the `References stored by identity` Story (`ambre-roadmap.md` E6) | failing |
| `TRAME-009` | `POST` | Deleting a sheet MUST break the references that read it. | this spec section 3 | `ambre_trame::logic::tests::group1::cross_sheet_reference_survives_rename_and_breaks_on_delete`; `ambre_trame::formula::parser::tests::deleted_sheet_id_yields_broken_ref` | none | verified |
| `TRAME-214` | `FRAME` | Renaming or reordering a sheet MUST NOT break a reference that reads it. | this spec section 3 | `ambre_trame::logic::tests::group1::cross_sheet_reference_survives_rename_and_breaks_on_delete`; `ambre_trame::logic::tests::group3::cross_sheet_reference_converges_across_peers_on_rename`; `::rename_sheet_keeps_id_and_cells`; `::move_sheet_reorders_tabs` | none | verified |
| `TRAME-215` | `FRAME` | A row or column delete MUST NOT rewrite the raw text of a formula. | this spec section 3 | `ambre_trame::logic::tests::group2::formula_reference_is_positional_after_delete` -- it reads the shifted grid through unchanged text, and `shift_formula` is reachable only from the paste and fill paths | none | verified |

### 1.4 Sort a range

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TRAME-005` | `POST` | A single-cell reference MUST follow its row through a sort or a move. | this spec section 3 | `ambre_trame::logic::tests::group4::sort_range_formulas_follow_data` | none | verified |
| `TRAME-216` | `FRAME` | A sort MUST NOT move the endpoints of a range: the range stays the positional block it was. | this spec section 3 | `ambre_trame::logic::tests::group4::sort_range_sum_block_is_stable_under_internal_sort` | none | verified |
| `TRAME-227` | `FRAME` | A sort or a line move MUST re-anchor each range endpoint it moves to the identity now at the endpoint's position, so the range keeps covering the same positions. | this spec section 5 | not built | unbuilt -- lands with the `References stored by identity` Story (`ambre-roadmap.md` E6) | unimplemented |

### 1.5 Paste or fill a formula

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TRAME-217` | `POST` | A paste or a fill MUST shift every cell and range reference by the offset the formula moved. | this spec section 4 | `ambre_trame::formula::shift::tests::shifts_cell_and_range_down_right`; `::keeps_operators_and_functions` | none | verified |
| `TRAME-043` | `POST` | A reference a paste or a fill pushes off the grid MUST read `#REF!`. | this spec section 4 | `ambre_trame::formula::shift::tests::off_grid_reference_becomes_ref_error`; `::ref_error_text_is_reparseable` | none | verified |
| `TRAME-218` | `FRAME` | A paste or a fill MUST NOT rewrite an operator, a literal or a function name. | this spec section 4 | `ambre_trame::formula::shift::tests::keeps_operators_and_functions`; `::no_shift_is_verbatim` | none | verified |
| `TRAME-219` | `FRAME` | A paste or a fill MUST NOT shift a defined name. | this spec section 4 | none | unheld -- `shift_expr` returns a `Name` node untouched, which is the whole of the rule and is stated nowhere else, and no test pastes a formula that carries one; lands with the `The Trame belts` Story (`ambre-roadmap.md` E9) | untested |
| `TRAME-220` | `FAIL` | A formula the parser refuses MUST be pasted verbatim, never mangled. | this spec section 4 | `ambre_trame::formula::shift::tests::unparseable_left_verbatim` | none | verified |

* **Budget.** three rows of `ambre-spec-budgets.md section 1`: the 10k-cell DAG recompute, the
  `range_index` scan and `SetCellText` to the viewport, plus the DAG and cache holder of section 6.
  **None of the four has a mechanism**: no recipe runs a Trame benchmark.
* **Boundary.** none of its own. `TRAME-010` keeps computed values out of the document, so only
  the raw source travels, and `ZK-001` seals it before it leaves the client.
* **Verification.** 34 `verified`, 1 `failing`, 3 `untested`, 5 `unimplemented`. What holds is the
  graph and its recompute, the best-covered engine of the workspace. What is loose is weight rather
  than behaviour: nothing measures what a range allocates. The failure is a rule the code
  contradicts, how a reference follows a delete (`TRAME-008`); its insert twin is `TRAME-171`.
  Beside them is the dynamic half of cycle detection, which nothing drives.

---

## 2. WHAT EACH USER SEES

| A does | B does | Converged document | What each user sees | Test |
|---|---|---|---|---|
| types `=SUM(A1:A3)` into `C1` | types `5` into `A2` | both writes, on their own cells | the same total on both screens | `ambre_trame::logic::convergence_tests::two_peers_converge_under_interleaved_ops` |
| types `=A1+1` into `B2` | deletes row 1 | the formula, rebound to the shifted grid | both read the same value, and neither reads `#REF!` (`TRAME-008`) | `ambre_trame::logic::tests::group2::formula_reference_is_positional_after_delete` |
| types `=Sheet2!A1` | renames `Sheet2` | the reference, stored by sheet id | both keep reading the value (`TRAME-034`) | `ambre_trame::logic::tests::group3::cross_sheet_reference_converges_across_peers_on_rename` |
| types `=Sheet2!A1` | deletes `Sheet2` | the reference, with no sheet to reach | both read `#REF!` (`TRAME-009`) | `ambre_trame::logic::tests::group1::cross_sheet_reference_survives_rename_and_breaks_on_delete` |
| sorts the range under a total | edits a cell inside it | the rows permuted, the edit on its own row | both read the same total (`TRAME-005`) | the same property, whose op set carries `Sort` and `SetText` |
| filters rows out of a total | reads the total | the filter, which is view state | both read the same total, filtered or not (`TRAME-021`) | `ambre_trame::logic::tests::group5::shared_filter_does_not_affect_sum` |

---

## 3. MODEL

**Containers.** None. The engine owns no CRDT state. It reads the raw text of `cells_{sheet}`
(`ambre-spec-trame-schema.md section 3`) and holds every derived structure in memory, rebuilt from
the document on load. Named ranges are the exception, and are a feature of their own.

**The pipeline**, from typed text to displayed value:

1. `parse_formula` reads the text into an AST. It knows nothing of the document.
2. `resolve_refs` stamps the formula's own sheet onto unqualified references, resolves a qualifier
   to a sheet index, and expands a name into its target.
3. `compile` walks the AST and emits bytecode, binding each reference against the sheet index.
4. `DependencyGraph::update` records what the formula reads, and refuses a static cycle.
5. `collect_dirty` gathers every cell downstream of the edit (`TRAME-040`).
6. `compute_all_dirty` walks the dirty set in dependency order and evaluates each cell on the stack
   machine.

**Reference identity.** A position is what the user types; a uid is what the engine keeps.

```rust
pub enum AxisId {
    Bound(Uid),     // a materialized row or column. #REF! once that uid is deleted
    Virtual(u32),   // a position past the used range: stays positional, reads empty
}

pub struct CellId { pub sheet: u16, pub row: AxisId, pub col: AxisId }

pub struct RangeId {
    pub sheet: u16,
    pub row_start: RangeEndpoint, pub row_end: RangeEndpoint,
    pub col_start: RangeEndpoint, pub col_end: RangeEndpoint,
}

pub enum RangeEndpoint {
    Specific(AxisId),   // stable through the axis id
    EntireAxis,         // A:A or 1:1. Reserved: the parser never emits it (`TRAME-048`)
}
```

**The two levels.** Cell-to-cell edges live in a `petgraph` DAG, where a cycle is a static error. A
range is not expanded into edges: it is stored whole against the formula that reads it, and a write
is tested for containment. One entire column would otherwise cost a hundred thousand edges.

```rust
pub struct DependencyGraph {
    cell_dag: StableGraph<CellId, ()>,          // level 1: cycles are static
    cell_to_node: HashMap<CellId, NodeIndex>,
    range_deps: HashMap<CellId, Vec<RangeId>>,  // level 2: cycles are dynamic
    range_index: HashMap<u16, Vec<(CellId, RangeId)>>,
}
```

**Bytecode.** `CellBytecode` carries the ops, the cells read and the ranges read. The instruction set
is a stack machine: `PushNum`, `PushStr`, `PushBool`, `PushEmpty`, `PushCell`, `EvalRange`, the
binary and unary operators and `CallFn { name, n_args }`. `PushError` carries a failure the
compiler already knows about.

**Evaluating one cell**, in order. A cell already on the evaluation stack returns a
circular-reference error (`TRAME-042`). A cell already resolved returns its memo. Otherwise it is
pushed, executed, popped and memoised. This is the only guard against a loop the cell graph cannot see.

**The value domain.** What the machine pushes, what a builtin receives, and what the cache holds.

```rust
pub enum CellValue {
    Number(f64),
    Text(String),
    Boolean(bool),
    Error(FormulaError),
    Empty,
    Range(Vec<CellValue>),   // machine-internal: never in the value cache
}

pub enum FormulaError {
    CircularReference,
    BrokenRef,
    ParseError(String),
    DivisionByZero,
    TypeError,
    InvalidRef(String),
    NameNotFound(String),
    NumError,
    NaError,
}
```

An error is a value, not an exception: it is pushed like any other. It reaches the screen as the
cell's displayed text (`features/ambre-spec-trame-engine.md`, `TRAME-UI-014`).

**Code.**

* `formula/parser/` -- the grammar and `resolve_refs`;
* `formula/render.rs` -- rendering back, and the two qualifier rewrites (`TRAME-032`, `TRAME-034`);
* `formula/compile.rs`, `formula/cache.rs` -- binding and incremental compilation;
* `formula/dag.rs` -- the two levels and the cycle search;
* `formula/vm.rs` -- the stack machine and range evaluation;
* `formula/engine.rs` -- the dirty set and the recompute pass;
* `formula/shift.rs` -- the relative shift a paste or a fill applies (`TRAME-043`).

---

## 4. SURFACE

**What a user may type**, after the leading `=`:

| Form | Example | Note |
|---|---|---|
| number, text, boolean | `12.5`, `"ok"`, `TRUE` | a bareword starting with `TRUE` stays a name |
| cell, range | `A1`, `A1:B10` | row numbers are 1-based; `A0` is not a reference |
| another sheet | `Sheet2!A1`, `'My Sheet'!A1:B2` | stored by id, shown by name (`TRAME-034`) |
| a defined name | `TOTAL` | `features/ambre-spec-trame-named-ranges.md` |
| a function call | `SUM(A1:A3, 2)` | `features/ambre-spec-trame-function-library.md` |
| an error | `#REF!`, `#DIV/0!` | the engine writes these back, so it must read them |

**Operators**, tightest first: `^`, then `*` `/`, then `+` `-`, then `&`, then the comparisons
`=` `<>` `<=` `>=` `<` `>`. Unary minus binds tighter than `^`. Only `^` groups to the right
(`TRAME-036`, `TRAME-037`).

**Bounds.** The parser, the compiler and the drop of the tree all recurse. So a formula is refused
before parsing past 8 192 characters, past 64 levels of parentheses or calls, or past 512
operators (`TRAME-013`). The first two are Excel's own limits.

A wasm thread gets 1 MiB of stack. A native release build overflows it past about 256 levels and
2 048 operators, so both bounds keep a margin of four. That was measured on x86, not yet on wasm.

**Errors a cell can show:**

| Value | Cause |
|---|---|
| `#REF!` | a deleted row, column or sheet, or a reference shifted off the grid |
| `#DIV/0!` | a division whose divisor is zero |
| `#CIRCULAR!` | a formula that reads itself, directly or through others |
| `#TYPE!` | an operand of a type the operator refuses |
| `#NUM!` | a numeric argument out of the function's domain |
| `#N/A` | a lookup that found nothing, or a name that is not a function |
| `#PARSE!(msg)` | text that is not a formula. Evaluation produces it; no formula holds it |

**Where the formula appears.** The grid paints the value, the formula bar the source; opening the
cell shows the source again. Both are specified in `features/ambre-spec-trame-cell-editing.md`.

---

## 5. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| A range is a dependency of its own, not a set of edges | expanding each range into cell-to-cell edges in `petgraph` | `=SUM(A:A)` would be a hundred thousand edges, rebuilt on every row insert. Storing the rectangle and testing containment costs one entry per formula, at the price of a linear scan the interval tree will replace |
| Cycles are caught in two places | one detector | it would have to be the dynamic one, and would then run on every evaluation. The cell graph sees only what compilation could resolve to a cell: a loop closed through a range, a name or another sheet is invisible to it, and a spreadsheet that hangs is worse than one that says `#CIRCULAR!` |
| A reference is stored by the identities it names (`TRAME-228`) | storing positions and rewriting each formula's text on an insert or a delete, as the file formats do | the rewrite misses a formula a peer types during the insert, which then reads the wrong line after the merge (`TRAME-229`). A positional text is also re-bound by every reseed, so what a sort kept is lost on the next tab switch |
| A reorder re-anchors the range endpoints it moves (`TRAME-227`) | binding both endpoints to uids and never re-anchoring | it inverts `SUM(A1:A3)` the first time a sort moves `A3` above `A1`. A single reference follows its row; a range must stay a rectangle, and an insert inside it must widen it (`TRAME-171`) |
| A position past the used range is virtual, not an error | materializing rows on reference | reading `=A100` on a ten-row sheet would create ninety lines nobody wrote, on every peer, for one read |
| A position past the used range is virtual, not an error | refusing the formula | `=A100` on a ten-row sheet is a forward reference a user writes on purpose; refusing it forbids writing a formula before the data it will read |
| A formula compiles once and runs many times | walking the AST on every evaluation | recomputation is the hot path and parsing is not. Only a text change makes a cell stale (`TRAME-011`) |
| A sheet qualifier is stored as a stable id and shown as a name | storing the display name, as the file formats do | a rename must not rewrite every formula in the workbook, and a rewrite that fails halfway is a corrupted document |
| Numbers compare on a relative tolerance | an absolute epsilon | it calls `1e-16` and `2e-16` equal while they differ by a factor of two |
| Numbers compare on a relative tolerance | a plain `==` | it calls `0.1+0.2` different from `0.3` |
| An empty cell takes the zero of its peer | a type error on any comparison with a blank | a formula dragged one row past its data must keep working. Refusing the comparison made `=IF(A1>5,...)` fail on the blank cell instead of reading it as zero |
| Evaluation refuses a range past a million cells | clamping to the used range | it silently answers something else. A co-editor can type `A1:ZZ100000` by accident, and the cost lands on every peer, not only the author |
| A line deleted then reinserted is a different line | finding the cell by its position after a delete and a reinsert, which Excel can do | it is a heuristic that fails hardest in the case it is wanted for, where the position moved between the two operations on another peer. A delete is an explicit intent and the reinsertion mints a new identity, so `#REF!` states exactly one thing: the target no longer exists. The user retypes or copies the formula onto the new line |
| A result never depends on view state | excluding hidden rows from an aggregate | `SUBTOTAL` will do that explicitly instead. Two peers scrolled elsewhere, filtered differently or on another tab must read the same number, or the document has no single value (`TRAME-021`) |

---

## 6. NOT BUILT, AND NOT YET RULED

* **The delete rule and the code disagree** (`TRAME-008`). A row delete rebuilds the sheet index and
  rebinds every formula from its raw text, so nothing survives holding the deleted uid. `=A2` after
  deleting row 1 reads the shifted cell, not `#REF!`. The broken-reference machinery is real and
  tested at the instruction level, and unreachable from this path. The decision is made
  (section 5): references are stored by identity (`TRAME-228`), and a deleted line reads `#REF!`.
* **Whole-column and whole-row ranges** (`TRAME-048`). `RangeEndpoint::EntireAxis` exists in the type
  and every resolver handles it; the grammar never emits it, so `A:A` parses as a name and fails.
  The interactive cousin, a whole-column selection, ships.
* **No absolute reference.** There is no `$A$1` syntax, so every reference shifts on a paste or a
  fill. A user who wants a fixed lookup table has no way to say so, and no rule covers it.
* **The dynamic cycle guard is driven by nothing** (`TRAME-042`). It is one line in the evaluator.
  The case it exists for is the one no test builds: a loop through a range, a name or another
  sheet. A direct self-reference never reaches it: `update` refuses it as a cycle of one cell
  (`TRAME-213`).
* **The range caps are code constants, not budget rows** (`TRAME-209`, `TRAME-210`).
  `MAX_RANGE_CELLS` bounds the memory of one range, and `MAX_PASS_RANGE_CELLS` bounds the reads of
  one pass, sized to clear a running total over ten thousand rows. Neither figure is a row of
  `1-Foundations/ambre-spec-budgets.md`, and nothing weighs the allocation `TRAME-209` forbids.
* **A tab switch recompiles the workbook.** `rebuild_sheet_maps` pins the active sheet at index 0
  and every `CellId` carries that index. `activate_sheet` therefore resets the engine and re-parses
  every cell of every sheet. The numbers stay right (`TRAME-021` holds), but the engine's state is
  a function of what the reader is looking at. No budget covers the gesture.
* **The second level of the graph is tested on a shape the compiler never emits.** `resolve_range`
  binds range endpoints to `AxisId::Virtual` only; the range tests in `dag.rs` build
  `AxisId::Bound` endpoints, where a deleted line makes the endpoint unresolvable. Only
  `engine::tests::range_dependency_propagates_through_a_chain` walks the real path.
* **The index the engine rests on has no equivalence property.** `TRAME-023`, in
  `features/ambre-spec-trame-engine.md`, states that the incrementally maintained sheet index must
  equal a rebuild. Every reference resolves through that index, so a divergence in it is a wrong
  number in a cell, not a slow redraw.
* **The graph pays for an interior mutability it no longer needs.** The memo of the evaluation
  pass is held behind a `RefCell` that results staged before write-back make redundant. It
  produces no wrong number; it is a cost nothing weighs. The walk and the cycle check are the
  other half, in `features/ambre-spec-trame-engine.md section 6`. Answered by the
  `A recompute bounded by what changed` Story (`ambre-roadmap.md` E6).
