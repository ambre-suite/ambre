# AMBRE -- FEATURE: FORMULA ENGINE

**Type:** Vertical (feature)
**Implementation:** partial -- the language, the graph and the evaluator are built; whole-axis ranges are not.

> A user types `=SUM(A1:A3)*2`. The cell shows a number, and keeps showing the right number after
> someone else edits `A2`, sorts the rows or renames the sheet the formula reads from.

Twelve behaviours here name organs, not features: a range needs a binding, a binding needs a graph,
a graph needs a cycle rule, and none of the four ships alone. They are one document. The function
catalogue and the named ranges are not organs of it, and have their own.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`.

| ID | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|
| `TRAME-005` | A single-cell reference MUST follow its row through a sort or a move; a range MUST remain a positional block. | this spec section 3 | `ambre_trame::logic::tests::group4::sort_range_formulas_follow_data`, `::sort_range_sum_block_is_stable_under_internal_sort` | verified |
| `TRAME-006` | A reference past the used range MUST read empty, never an error. | this spec section 3 | `ambre_trame::formula::compile::tests::ref_beyond_used_range_is_virtual`, `ambre_trame::formula::vm::tests::push_cell_virtual_beyond_used_range_reads_empty` | verified |
| `TRAME-007` | A reference to a deleted row or column MUST read `#REF!`, and every operator MUST propagate that error. | this spec section 4 | `ambre_trame::formula::vm::tests::push_cell_deleted_bound_yields_ref_error`, `::every_operator_propagates_an_operand_error`, `::unary_not_propagates_an_operand_error`, `::eval_range_over_cell_cap_yields_ref_error` | verified |
| `TRAME-008` | A row or column delete MUST propagate `#REF!` to every formula that read the deleted line. | this spec section 6 | none, and the code does the **opposite**: the delete path rebinds every formula from its raw text, so the reference lands positionally on the shifted grid. `ambre_trame::logic::tests::group2::formula_reference_is_positional_after_delete` asserts `=A2` reads empty after deleting row 1 | unverified |
| `TRAME-009` | Deleting a sheet MUST break the references that read it; a rename or a reorder MUST NOT. | this spec section 3 | `ambre_trame::logic::tests::group1::cross_sheet_reference_survives_rename_and_breaks_on_delete`, `group3::cross_sheet_reference_converges_across_peers_on_rename`, `::rename_sheet_keeps_id_and_cells`, `ambre_trame::formula::parser::tests::deleted_sheet_id_yields_broken_ref` | verified |
| `TRAME-011` | Only a cell whose text changed MUST be compiled again. | this spec section 3 | `ambre_trame::formula::cache::tests::stale_cell_recompiled`, `::not_stale_no_recompile`, `::non_formula_clears_cache` | verified |
| `TRAME-012` | A formula that reads itself through other cells MUST be refused with its culprits named. | this spec section 3 | `ambre_trame::formula::dag::tests::static_cycle_detected`, `::cycle_detection_kosaraju_returns_culprits`, `::self_reference_rejected`, `::rejected_cycle_leaves_no_stale_range_deps` | verified |
| `TRAME-013` | No text typed into a cell MUST make the engine panic; every failure MUST be an error value. | this spec section 3 | `ambre_trame::formula::prop_tests::parser_never_panics_on_arbitrary_text`, `::parser_never_panics_on_formula_noise`, `::compile_emits_balanced_bytecode`, `::vm_is_total_and_deterministic` (proptest) | verified |
| `TRAME-021` | A formula result MUST NOT depend on what the reader is looking at. | this spec section 5 | `ambre_trame::logic::tests::group5::shared_filter_does_not_affect_sum`, `group3::masked_cell_excluded_across_sheets_regardless_of_active_tab` | verified |
| `TRAME-032` | A formula the engine writes back MUST read as the same formula. | this spec section 3 | `ambre_trame::formula::prop_tests::render_parse_render_is_a_fixpoint` (proptest), `ambre_trame::formula::parser::tests::error_literal_reparses`, `ambre_trame::formula::shift::tests::ref_error_text_is_reparseable` | verified |
| `TRAME-033` | A formula shown back to its author MUST carry only the parentheses its meaning needs. | this spec section 4 | `ambre_trame::formula::render::tests::precedence_minimal_parens`, `::pow_right_associative` | verified |
| `TRAME-034` | A reference to another sheet MUST survive that sheet being renamed. | this spec section 3 | `ambre_trame::formula::render::tests::canonicalize_name_to_id`, `::display_id_to_name`, `ambre_trame::formula::parser::tests::id_qualified_cell_resolves_index`, `::sheet_qualified_resolves_index` | verified |
| `TRAME-035` | A reference to a sheet that never existed MUST be refused, not evaluated. | this spec section 4 | `ambre_trame::formula::parser::tests::unknown_sheet_name_is_parse_error`, `::unknown_sheet_is_error` | verified |
| `TRAME-036` | Operators MUST bind in the order a spreadsheet user expects. | this spec section 4 | `ambre_trame::formula::parser::tests::arithmetic_precedence`, `::parens_override_precedence`, `::comparison_chain`, `ambre_trame::formula::vm::tests::precedence` | verified |
| `TRAME-037` | The power operator MUST group to the right. | this spec section 4 | `ambre_trame::formula::parser::tests::power_right_associative`, `ambre_trame::formula::render::tests::pow_right_associative` | verified |
| `TRAME-038` | A cell holding plain text MUST hold no compiled formula and no dependency. | this spec section 3 | `ambre_trame::formula::cache::tests::non_formula_clears_cache`, `ambre_trame::formula::dag::tests::clear_deps_drops_the_formula_and_its_now_orphan_node` | verified |
| `TRAME-039` | A refused formula MUST replace the cell's previous result, never leave it on screen. | this spec section 3 | `ambre_trame::formula::engine::tests::parse_error_replaces_the_stale_value`, `::cycle_replaces_the_stale_value_with_an_error` | verified |
| `TRAME-040` | An edit MUST update every cell that reads it, directly or through a range. | this spec section 3 | `ambre_trame::formula::dag::tests::simple_dependency_propagation`, `::range_dirty_collection`, `::collect_dirty_reaches_a_fixpoint_over_chained_ranges`, `ambre_trame::formula::engine::tests::cascade_after_edit`, `::range_dependency_propagates_through_a_chain` | verified |
| `TRAME-041` | An edit MUST NOT recompute a cell that reads nothing it changed. | this spec section 3 | `ambre_trame::formula::dag::tests::range_outside_no_dirty`, `ambre_trame::formula::cache::tests::not_stale_no_recompile` | verified |
| `TRAME-042` | A loop closed through a range, a name or another sheet MUST be caught while evaluating. | this spec section 3 | none. The guard is the evaluation stack in `formula/vm.rs`; every cycle test drives the static graph instead, so no test closes a loop the graph cannot see | unverified |
| `TRAME-043` | A reference a paste or a fill pushes off the grid MUST read `#REF!`. | this spec section 4 | `ambre_trame::formula::shift::tests::off_grid_reference_becomes_ref_error`, `::ref_error_text_is_reparseable` | verified |
| `TRAME-044` | A range too large to hold in memory MUST read `#REF!`, and MUST NOT be built. | this spec section 3 | `ambre_trame::formula::vm::tests::eval_range_over_cell_cap_yields_ref_error` | verified |
| `TRAME-045` | Every evaluation failure MUST show a named error, never a blank and never a number. | this spec section 4 | `ambre_trame::formula::vm::tests::division_by_zero`, `ambre_trame::formula::engine::tests::division_by_zero_displayed`, `ambre_trame::formula::types::tests::cell_value_display` | verified |
| `TRAME-046` | An empty cell MUST compare as the zero of whatever it is compared with. | this spec section 4 | `ambre_trame::formula::vm::tests::an_empty_operand_compares_at_its_peer_zero`, `::booleans_order_false_before_true` | verified |
| `TRAME-047` | Two numbers MUST compare on a relative tolerance, never an absolute one. | this spec section 5 | `ambre_trame::formula::vm::tests::equality_is_relative_not_absolute` | verified |
| `TRAME-048` | A whole-column or whole-row range MUST cover the used part of its axis. | this spec section 6 | not built -- lands with the "Engine completion" Story (`ambre-roadmap.md` E6) | unimplemented |

* **Budget.** three rows of `ambre-spec-budgets.md section 1` -- the 10k-cell DAG recompute
  in < 50 ms, the `range_index` scan of 1k range formulas in < 1 ms, `SetCellText` to the viewport
  in < 5 ms -- plus the DAG and cache holder of `ambre-spec-budgets.md section 6`, < 50 MB
  per 100k formula cells. Only the recompute has a benchmark.
* **Boundary.** none of its own: `TRAME-010` keeps computed values out of the document, so only the
  raw source travels, and `ZK-001` seals it before it leaves the client.
* **Verification.** 23 `verified`, 2 `unverified`, 1 `unimplemented`. Both gaps are named.
  `TRAME-008` is the row the code contradicts: the spec claims a `#REF!` on a row delete and the
  suite asserts the opposite, green. `TRAME-042` is the dynamic half of cycle detection, which
  exists precisely for the loops the static graph cannot see, and is driven by nothing.

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

**Containers.** None. The engine owns no CRDT state: it reads the raw text of `cells_{sheet}`
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
binary and unary operators, `CallFn { name, n_args }`, and `PushError` for a failure the compiler
already knows about.

**Evaluating one cell**, in order: a cell already on the evaluation stack returns a circular-reference
error (`TRAME-042`); a cell already resolved returns its memo; otherwise it is pushed, executed,
popped and memoised. This is the only guard against a loop the cell graph cannot see.

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

An error is a value, not an exception: it is pushed like any other, and it reaches the screen as the
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

## 5. DECISIONS

| Decision | Rationale | Rejected alternative |
|---|---|---|
| A range is a dependency of its own, not a set of edges | `=SUM(A:A)` would be a hundred thousand edges, rebuilt on every row insert. Storing the rectangle and testing containment costs one entry per formula, at the price of a linear scan the interval tree will replace | expanding each range into cell-to-cell edges in `petgraph` |
| Cycles are caught in two places | the cell graph sees only what compilation could resolve to a cell. A loop closed through a range, a name or another sheet is invisible to it, and a spreadsheet that hangs is worse than one that says `#CIRCULAR!` | one detector, which would have to be the dynamic one, and would then run on every evaluation |
| A single-cell reference binds to a uid, a range endpoint stays positional | a reference must follow its row through a sort, which needs an identity. A range must stay a rectangle: uid-bound endpoints could be sorted past each other and invert it | binding both, which inverts `SUM(A1:A3)` the first time a sort moves `A3` above `A1` |
| A position past the used range is virtual, not an error | `=A100` on a ten-row sheet is a forward reference a user writes on purpose. It reads empty, and rebinds when the row is created | materializing rows on reference, or refusing the formula |
| A formula compiles once and runs many times | recomputation is the hot path and parsing is not. Only a text change makes a cell stale (`TRAME-011`) | walking the AST on every evaluation |
| A sheet qualifier is stored as a stable id and shown as a name | a rename must not rewrite every formula in the workbook, and a rewrite that fails halfway is a corrupted document | storing the display name, as the file formats do |
| Numbers compare on a relative tolerance | an absolute epsilon calls `1e-16` and `2e-16` equal while they differ by a factor of two; a plain `==` calls `0.1+0.2` different from `0.3` | either of the two extremes |
| An empty cell takes the zero of its peer | a formula dragged one row past its data must keep working. Refusing the comparison made `=IF(A1>5,...)` fail on the blank cell instead of reading it as zero | a type error on any comparison with a blank |
| Evaluation refuses a range past a million cells | a co-editor can type `A1:ZZ100000` by accident, and the cost lands on every peer, not only the author | clamping to the used range, which silently answers something else |
| A line deleted then reinserted is a different line, and the old reference stays broken | a delete is an explicit intent, and the reinsertion mints a new identity, so `#REF!` states exactly one thing: the target no longer exists. Recovering the cell by its position is a heuristic that fails hardest in the case it is wanted for, where the position moved between the two operations on another peer. The user retypes or copies the formula onto the new line | finding the cell by its position after a delete and a reinsert, which Excel can do |
| A result never depends on view state | two peers scrolled elsewhere, filtered differently or on another tab must read the same number, or the document has no single value (`TRAME-021`) | excluding hidden rows from an aggregate, which `SUBTOTAL` will do explicitly instead |

---

## 6. NOT BUILT, AND NOT YET RULED

* **The delete rule and the code disagree** (`TRAME-008`). A row delete rebuilds the sheet index and
  rebinds every formula from its raw text, so nothing survives holding the deleted uid: `=A2` after
  deleting row 1 reads the shifted cell, not `#REF!`. The broken-reference machinery is real and
  tested at the instruction level, and unreachable from this path. Positional like Excel, or broken
  like a deleted sheet, is a product decision no document has made.
* **Whole-column and whole-row ranges** (`TRAME-048`). `RangeEndpoint::EntireAxis` exists in the type
  and every resolver handles it; the grammar never emits it, so `A:A` parses as a name and fails.
  The interactive cousin, a whole-column selection, ships.
* **No absolute reference.** There is no `$A$1` syntax, so every reference shifts on a paste or a
  fill. A user who wants a fixed lookup table has no way to say so, and no rule covers it.
* **The dynamic cycle guard is driven by nothing** (`TRAME-042`). It is one line in the evaluator,
  and the case it exists for -- a loop through a range, a name or another sheet -- is the case no
  test builds.
* **The index the engine rests on has no equivalence property.** `TRAME-023`, in
  `features/ambre-spec-trame-engine.md`, states that the incrementally maintained sheet index must
  equal a rebuild. Every reference resolves through that index, so a divergence in it is a wrong
  number in a cell, not a slow redraw.
* **The graph pays twice for every edit.** Removing a cell rebuilds the whole `cell_to_node` map,
  on the premise that a `StableGraph` may reindex on removal -- which is the one thing that graph
  guarantees it never does, so the entry alone has to go. And the memo of the evaluation pass is
  held behind an interior mutability that results staged before write-back no longer need. Neither
  produces a wrong number; both are a cost nothing weighs. The walk and the cycle check are the
  other half, in `features/ambre-spec-trame-engine.md section 6`. Answered by the "A recompute
  bounded by what changed" story (`ambre-roadmap.md`).
