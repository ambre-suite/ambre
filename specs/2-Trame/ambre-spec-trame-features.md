# AMBRE — SPEC: AMBRE-TRAME FEATURES
**Type:** Vertical (Feature / Application)
**Status:** Draft
**Implementation:** partial

> Collaborative features of Trame on top of the calculation engine: sort, filters, conditional formatting, undo/redo, data validation, and range protection. All these features are built on `LoroDoc` and `TrameState` defined in `ambre-spec-trame-logic.md`.
>
> **Scope:** Logic thread only. Rendering of resulting states (hidden rows, conditional styles, protection indicators) is delegated to `ambre-spec-trame-ui.md`.

> **Architecture note.** The client runs a 3-thread shared-memory architecture (`flume` channels, single WASM binary) — see `ambre-spec-architecture.md`. Implications for this spec:
> - **State registry.** A Trame document's state is `TrameState`, a concrete value stored in the Logic thread's typed registry (`enum DocState { Trame(TrameState), … }`) and dispatched via `match` (no trait object, no `Box<dyn Any>`).
> - **Typed data plane, zero inter-thread postcard.** `TrameCommand`/`TrameEvent`/`DisplayCell` live in **`ambre-bus`** (re-exported by `ambre-trame`), travel **by move** on `flume` inside `AppCommand::Trame(..)` / `AppEvent::Trame(..)` — **no `serde` derive**. `Vec<u8>` only remain for real network bytes (Loro deltas/snapshots).
> - **Typed outcome for editing.** A handler that mutates the CRDT does not "self-synchronize": it **returns** domain bytes that the Logic dispatcher fans out to the Sync thread. See §0 below and `ambre-spec-architecture.md §7 ARCH P2` ("one-way content handler").

---

## 0. EXECUTION MODEL — COMMANDS, EVENTS, OUTCOME

All features below are driven by `TrameCommand` (UI → Logic thread) and produce `TrameEvent` (Logic thread → UI) — both types live in `ambre-bus` and are transported typed inside `AppCommand::Trame` / `AppEvent::Trame` (the Logic dispatcher routes by `DocumentId` to the live `TrameState`).

A feature handler does **not** do networking or direct `send_to_ui`. Depending on what it produces:
- **Display only** (filter, visual sort, conditional formatting): returns `TrameEvent` (display) → the dispatcher wraps them in `UiEvent::Viewport`.
- **CRDT mutation to broadcast** (cell edit, row insertion, shared filter application, …): the Loro mutation produces a **delta**; the handler passes it back via a **typed outcome** (domain bytes), and the Logic dispatcher emits `SyncCommand::SendUpdate { document_id, payload }` (Logic adds the `document_id` it holds). Target schema:

```rust
// App level (ambre-trame). The Logic dispatcher fans out the outcome; the app remains
// unaware of SyncCommand (domain bytes, no transport envelope).
struct TrameOutcome {
    display:   Vec<TrameEvent>,   // -> UI   (UiEvent::Viewport / Presence)
    update:    Option<Vec<u8>>,   // Loro CRDT delta to broadcast -> SyncCommand::SendUpdate
    ephemeral: Option<Vec<u8>>,   // presence/cursor to broadcast -> SyncCommand::SendEphemeral
}
```

> The `fn handle_*(…) -> Vec<TrameEvent>` signatures shown in the following sections are the **display-only** form; for operations that must also broadcast a delta, read the return as the `display` branch of a `TrameOutcome` (the `update` branch carrying the Loro delta).

### Invariants

Prefix `TRAME-FEAT`. Grammar, falsifiability test and status values are normative in `ambre-spec-architecture.md §4`; the cross-spec roll-up is `ambre-spec-testing.md §7`. Rows `001..013` cover the built features -- shared filters (§1.A), sort (§2) and the execution model above. Rows `014..024` cover Phase 2/3 features whose command variants exist in `ambre-bus` but whose Logic handlers are explicit no-ops: they carry `Status: unimplemented` and cite the Story that will build them (architecture §4). Undo/redo is specified here (§4) but owned by the engine: its invariants are `TRAME-017` and `TRAME-018` in `trame-logic §1`, not restated here.

| ID | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|
| `TRAME-FEAT-001` | A feature handler MUST NOT send on any channel: it returns a `TrameOutcome` and the Logic dispatcher fans it out. | §0; architecture §4 (host testability) | type-level: `TrameState` holds no sink or sender and `handle_command` returns `TrameOutcome` by value; the dispatcher seam is exercised by `ambre_trame::logic::tests::group3::edit_emits_send_update_through_sink` | verified |
| `TRAME-FEAT-002` | A shared filter MUST hide rows by their `row_uid`, so the hidden set follows the data through a sort or an insert. | §1.A | `ambre_trame::logic::tests::group5::shared_filter_hidden_follows_sort` | verified |
| `TRAME-FEAT-003` | A filtered-out row MUST still be counted by formulas: filtering is a view, not a mutation of the data. | §1.A | `ambre_trame::logic::tests::group5::shared_filter_does_not_affect_sum` | verified |
| `TRAME-FEAT-004` | The filter definition MUST persist in the document and be re-evaluated when a cell it covers is edited. | §1.A | `ambre_trame::logic::tests::group5::shared_filter_recomputes_on_edit`, `group4::shared_filter_by_values_hides_nonmatching`, `::shared_filter_condition_greater_than` | verified |
| `TRAME-FEAT-005` | A peer receiving the broadcast MUST apply the hidden set directly, without re-evaluating the predicate. | §1.A | `ambre_trame::logic::tests::group5::shared_filter_applies_on_remote_import` | verified |
| `TRAME-FEAT-006` | Two concurrent shared filters MUST converge by last-writer-wins. | §1.A; §12 of `trame-logic` | `ambre_trame::logic::tests::group5::concurrent_filters_converge_lww`; the 2-peer convergence proptest also drives `ApplyFilter`/`ClearFilter` and asserts the elected `is_filtered` set (`logic::convergence_tests::two_peers_converge_under_interleaved_ops`) | verified |
| `TRAME-FEAT-007` | A shared filter MUST be undoable as an ordinary CRDT mutation. | §1.A; §4 | `ambre_trame::logic::tests::group5::shared_filter_is_undoable`, `group2::filtered_column_edit_is_one_undo_item` | verified |
| `TRAME-FEAT-008` | A filter command that cannot apply (wrong sheet, virtual column, absent filter) MUST leave the document untouched. | §1.A | `ambre_trame::logic::tests::group5::shared_filter_noop_guards`, `::clear_filter_unhides_and_noops_when_absent` | verified |
| `TRAME-FEAT-009` | Every filter operator MUST be exercised, and a filter definition MUST round-trip through its stored form. | §1.A | `ambre_trame::logic::tests::group5::condition_pass_covers_every_operator`, `::filter_pass_values_and_op_string_roundtrip` | verified |
| `TRAME-FEAT-010` | A sort MUST order rows by their **evaluated** value, and MUST pin the header row when one is declared. | §2 | `ambre_trame::logic::tests::group3::sort_range_ascending_moves_whole_rows`, `::sort_range_descending_keeps_header`, `group4::sort_range_multi_pins_header_row` | verified |
| `TRAME-FEAT-011` | A multi-column sort MUST be lexicographic over its keys, each key applying its own direction. | §2 "Multi-Column Sort" | `ambre_trame::logic::tests::group4::sort_range_multi_lexicographic_with_per_key_direction`, `::sort_range_multi_no_keys_is_noop` | verified |
| `TRAME-FEAT-012` | A sort that changes no row order MUST emit no CRDT delta. | §2 | `ambre_trame::logic::tests::group4::sort_range_multi_already_sorted_emits_no_delta`, `::sort_range_noops` | verified |
| `TRAME-FEAT-013` | A sort invoked without an explicit range MUST resolve it from the tracked selection, in the Logic thread. | §2 | `ambre_trame::logic::tests::group4::sort_selection_resolves_against_tracked_selection`, `::sort_selection_multi_resolves_against_tracked_selection`, `::sort_sheet_sorts_used_range_by_active_column` | verified |
| `TRAME-FEAT-014` | A personal filter view MUST NOT emit any CRDT delta: it is local state, persisted to IndexedDB alone. | §1.B | not built -- lands with §10 Phase 2 (Epic `E6`) | unimplemented |
| `TRAME-FEAT-015` | Activating a personal filter view MUST recompute its hidden rows against the current document, never trust the stored set. | §1.B | not built -- lands with §10 Phase 2 (Epic `E6`) | unimplemented |
| `TRAME-FEAT-016` | Activating a personal filter view MUST NOT be an undo item. | §1.B; §4 | not built -- lands with §10 Phase 2 (Epic `E6`) | unimplemented |
| `TRAME-FEAT-017` | Conditional formatting MUST apply the first matching rule in ascending priority order, and stop there. | §3 Evaluation | not built -- lands with §10 Phase 2 (Epic `E6`) | unimplemented |
| `TRAME-FEAT-018` | A conditional format MUST produce a display-time `style_override` and MUST NOT mutate the stored cell style. | §3 Evaluation | not built -- lands with §10 Phase 2 (Epic `E6`) | unimplemented |
| `TRAME-FEAT-019` | A `Formula` conditional rule MUST resolve its relative references from the cell being evaluated, not from the rule's anchor. | §3 | not built -- lands with §10 Phase 2 (Epic `E6`) | unimplemented |
| `TRAME-FEAT-020` | A `Reject` validation MUST block the local mutation and emit no CRDT delta. | §5 Evaluation on Input | not built -- lands with §10 Phase 2 (Epic `E6`) | unimplemented |
| `TRAME-FEAT-021` | A `Warn` validation MUST allow the mutation and surface a warning alongside it. | §5 Evaluation on Input | not built -- lands with §10 Phase 2 (Epic `E6`) | unimplemented |
| `TRAME-FEAT-022` | A remote delta violating a local validation MUST be applied and MUST surface a warning; it MUST NOT be rejected. | §5 "Fundamental limitation" | not built -- lands with §10 Phase 2 (Epic `E6`) | unimplemented |
| `TRAME-FEAT-023` | Range protection MUST block a local edit by a non-owner, and MUST warn rather than reject on a remote violation. | §6 Enforcement | not built -- lands with §10 Phase 3 (Epic `E6`) | unimplemented |
| `TRAME-FEAT-024` | Protection MUST identify its owner by `UserId`, never by `pub_key`. | §6 Security Model; §10 Phase 3 | not built -- lands with §10 Phase 3 (Epic `E6`) | unimplemented |

> **Invariant coverage.** 13 `verified`, 0 `unverified`, 11 `unimplemented`. The 13 are the Phase-1 surface, the area the workspace tested most heavily before the grammar existed. The 11 are the rules of §1.B, §3, §5 and §6, whose handlers are explicit no-ops today: each cites the Story that will build it, and that Story is not done until its rows have moved off `unimplemented`.

---

## 1. FILTERS

Google Sheets distinguishes two filtering modes. Both are implemented.

### A. Shared Filter

Visible to all collaborators. Stored in the `LoroDoc` — the view is synchronized via SecSync.

**CRDT Schema (implemented):** the active filter is stored *flat* in
`filters_{sheet_id}` (empty map = no filter) rather than under a nested `active`
sub-map -- one fewer container to merge, same LWW semantics:

```
"filters_{sheet_id}" (LoroMap)         → empty = no filter
  ├── "col"    : String                → column uid the filter is applied to
  ├── "kind"   : String                → "values" | "condition"
  ├── "values" : LoroList<String>      → displayed (checked) values (kind="values")
  ├── "op"     : String                → operator (kind="condition")
  │       // "eq" | "ne" | "gt" | "lt" | "gte" | "lte" | "contains"
  │       // | "not_contains" | "starts_with" | "is_empty" | "is_not_empty"
  └── "value"  : String                → operand (kind="condition")
```

The `FilterKind` enum (`Values(Vec<String>)` / `Condition { op, value }`, with
`FilterOp`) lives in **`ambre-bus`** (re-exported by `ambre-trame`) and travels
typed in `TrameCommand::ApplyFilter`. Colour filters are Phase 2.

**Hidden rows:**

Rows hidden by the filter are stored in a separate `LoroMap` to avoid polluting `rows_{sheet_id}`:

```
"filter_hidden_{sheet_id}" (LoroMap)
  └── "{row_uid}" : bool         → true = hidden by the active filter
```

This `LoroMap` is recomputed by the Logic thread after each change to the filter or data. Only the filter author recomputes it — other clients receive `filter_hidden` via SecSync and apply it directly without recomputing (`apply_remote` → `rebuild_filter`, no recompute).

**Flow (implemented):**

```
TrameCommand::ApplyFilter { sheet_id, col, kind: FilterKind }
  │  (no-op for a non-active sheet or a virtual/never-materialized column)
  ├─ compute_all_dirty (so the formula_cache is current)
  ├─ write_filter_hidden: evaluate each materialized row's cell in `col` via
  │     engine.evaluated_value; row_uids failing `filter_pass(kind, value)` are
  │     written into LoroMap("filter_hidden_{sheet}") as {row_uid -> true}
  ├─ persist_filter_def: store col/kind/op/value(+values list) in "filters_{sheet}"
  ├─ rebuild_filter: derive `hidden_rows` (grid positions) from the uid map
  ├─ display : [TrameEvent::FilterApplied { hidden_row_count }, DataReset]
  └─ update  : CRDT delta → outcome.update → SyncCommand::SendUpdate (dispatcher)

TrameCommand::ClearFilter { sheet_id }
  │  (no-op -- no delta -- when no filter is active)
  └─ clears "filter_hidden_{sheet}" + "filters_{sheet}", emits
     [TrameEvent::FilterCleared, DataReset]
```

A `SetCellText` on the active sheet calls `recompute_active_filter`: it reloads
the stored definition and rewrites `filter_hidden_{sheet}` so a row's hidden
status tracks the edited value. Structural ops (insert/delete/sort) do **not**
recompute -- the hidden set is keyed by `row_uid`, so `rebuild_filter` simply
remaps it to the new positions and the hidden flag follows its row.

**Rendering:** hidden rows are not dropped from the viewport; each cell in a
hidden row is returned with `DisplayCell::is_filtered = true` (§8) and the UI
Thread collapses those rows.

**DAG interaction:** hidden rows are excluded from `DisplayViewport` but **included in calculations**. `=SUM(A:A)` on a filtered column returns the sum of all rows. To exclude hidden rows, the user uses `SUBTOTAL` (planned Phase 2).

**Concurrent conflict:** if Alice and Bob apply different filters simultaneously on the same sheet, Loro LWW determines the active filter. `filter_hidden` is recomputed by the client receiving the winning filter's delta. Deterministic behavior.

### B. Personal Filter Views

Each user can create filtered views without affecting other collaborators. Entirely local state, never synchronized.

**Storage:** `IndexedDB` only, key = `filter_view:{user_id_hex}:{sheet_id}:{view_id}`.

```rust
pub struct FilterView {
    pub id:          String,
    pub name:        String,
    pub sheet_id:    SheetId,
    pub filter_col:  Uid,
    pub kind:        FilterKind,
    pub hidden_rows: HashSet<Uid>, // recomputed on activation
}
```

**Flow:**

```
TrameCommand::ActivateFilterView { view_id }
  │
  ├─ Loads from IndexedDB
  ├─ Recomputes hidden_rows on current doc state
  └─ Sends DisplayViewport without hidden rows
     (without touching the LoroDoc — zero CRDT delta emitted)

TrameCommand::SaveFilterView { name, filter_def }
  │
  └─ Persists in IndexedDB only
```

**UI indicator:** when a personal view is active, the UI Thread displays a distinctive badge to signal that the view is not shared.

---

## 2. SORT

### Simple Sort (single column)

```
TrameCommand::SortRange {
    sheet_id:   SheetId,
    row_start:  u32, row_end:  u32,     // range to sort
    col_start:  u32, col_end:  u32,
    sort_col:   u32,                    // reference column (index within range)
    ascending:  bool,
    has_header: bool,                   // ignore first row
}
```

**Implementation:**

```rust
// As-built: `TrameState::sort_range` (`logic/sort.rs`). Sorting only ever targets
// the ACTIVE sheet (the UI activates it first): the formula cache the keys read
// from is pinned to engine index 0. The header (if any) is pinned by starting the
// sortable region one row down; rows after `row_end` are simply never moved -- there
// is no "rows after the range" chain to append (that was a bug: `rows` only spans
// the range, so `skip(row_end + 1)` was always empty).
fn sort_range(
    &mut self, sheet_id: &str, row_start: u32, row_end: u32, col_start: u32,
    col_end: u32, sort_col: u32, ascending: bool, has_header: bool,
) -> TrameOutcome {
    if sheet_id != self.active_sheet { return TrameOutcome::display_only(vec![]); }
    // Clamp to materialized rows; virtual rows carry no data to reorder.
    let last = self.index.row_len().saturating_sub(1) as u32;
    let row_end = row_end.min(last);
    if col_start + sort_col > col_end || row_start > row_end {
        return TrameOutcome::display_only(vec![]);
    }
    let sort_col_abs = col_start + sort_col;
    let sort_start = if has_header { row_start + 1 } else { row_start };
    if sort_start >= row_end { return TrameOutcome::display_only(vec![]); } // <= 1 sortable row

    // Key each sortable row by its uid and the evaluated value of its sort column
    // (`active_cell_id` builds the CellId with the engine u16 index -- not the
    // String sheet_id). Whole rows travel by uid; no cell content is read/rewritten.
    let mut keyed: Vec<(Uid, CellValue)> = (sort_start..=row_end)
        .filter_map(|pos| {
            let row_uid = self.index.row_id(pos as usize)?;
            let value = self.engine
                .evaluated_value(self.active_cell_id(pos, sort_col_abs))
                .unwrap_or(CellValue::Empty);
            Some((row_uid, value))
        })
        .collect();

    let before: Vec<Uid> = keyed.iter().map(|(uid, _)| *uid).collect();
    keyed.sort_by(|(_, a), (_, b)| {
        let ord = compare_cell_values(a, b); // Number < Text < Boolean < Error < Empty
        if ascending { ord } else { ord.reverse() }
    });
    let target: Vec<Uid> = keyed.into_iter().map(|(uid, _)| uid).collect();
    // realize_sort replays the permutation as `mov` ops on rows_{sheet} (a no-op +
    // SortComplete when already ordered); the delta rides outcome.update.
    self.realize_sort(sort_start, &before, target)
}

fn compare_cell_values(a: &CellValue, b: &CellValue) -> std::cmp::Ordering {
    use CellValue::*;
    use std::cmp::Ordering::*;
    match (a, b) {
        (Number(x), Number(y))   => x.partial_cmp(y).unwrap_or(Equal),
        (Text(x),   Text(y))     => x.cmp(y),
        (Boolean(x), Boolean(y)) => x.cmp(y),
        (Empty, Empty)           => Equal,
        // Type order: Number < Text < Boolean < Error < Empty
        (Number(_), _)  => Less,
        (_, Number(_))  => Greater,
        (Text(_), _)    => Less,
        (_, Text(_))    => Greater,
        (Boolean(_), _) => Less,
        (_, Boolean(_)) => Greater,
        (Error(_), _)   => Less,
        _               => Equal,
    }
}
```

> **Sort is a pure row-order permutation.** Because each row carries an own move-invariant
> `Uid` stored as the `rows_{sheet}` element *value* (`trame-logic §2.A`), reordering the list
> with `MovableList::mov` leaves every `"{row_uid}:{col_uid}"` cell key intact -- the data
> follows the move with **no cell rewrite**. The sort touches only the order list (one `mov`
> per displaced row), then rebuilds the `SheetIndex`. Crucially it **preserves** the engine's
> uid-anchored bytecode (no reseed/re-parse) and merely recomputes; single-cell references
> (bound to a row/column `Uid`, `trame-logic §3`) resolve to the new positions, so they
> **follow the data** -- the §8 / `trame-logic §12` guarantee. A *range* is a positional block
> (its endpoints stay positional), so an internal sort leaves the block and its total intact.
> `compare_cell_values` uses an explicit type-rank (`Number < Text < Boolean < Error < Empty`);
> `Error` ranks **before** `Empty` (errors are not last -- blanks are).
> Emits `TrameEvent::SortComplete` + `DataReset`.
>
> **Assumed difference from Excel (descending).** Descending reverses the *whole*
> comparator, blank cells included, so blanks sort to the **top** on a descending
> sort. Excel keeps blanks last regardless of direction. This falls out of the
> single `ord.reverse()` and is accepted as-is (a blank has no value to anchor last).

### Multi-Column Sort (MVP)

> **MVP.** Promoted from Phase 2: multi-column sort ships with the single-column sort. It is the
> same pure row-order permutation (uid-anchored bytecode preserved, references follow), only the
> comparator changes. UI entry point = the `SortDialog` (`-ui` §5.C).

```rust
TrameCommand::SortRangeMulti {
    sheet_id:   SheetId,
    row_start:  u32, row_end: u32,
    col_start:  u32, col_end: u32,
    sort_keys:  Vec<SortKey>,          // decreasing priority: [primary, secondary, ...]
    has_header: bool,
}

pub struct SortKey {
    pub col:       u32,
    pub ascending: bool,
}
```

Implementation: `sort_by` with lexicographic comparison on `sort_keys` in order.

### Concurrent Sort Conflict

Two users sorting simultaneously: LWW on the row `LoroMovableList` — the last commit wins. Deterministic result. Behavior identical to Google Sheets.

---

## 3. CONDITIONAL FORMATTING

### CRDT Storage

```
"cond_formats_{sheet_id}" (LoroList)
  └── item (LoroMap)
      ├── "id"       : String           → stable rule identifier
      ├── "range"    : String           → "{row_uid}:{col_uid}:{row_uid_end}:{col_uid_end}"
      ├── "priority" : i64              → evaluation order (smaller = higher priority)
      ├── "rule"     : LoroMap          → rule definition
      │   ├── "kind" : String           → see CondFormatKind
      │   └── ...kind-specific fields
      └── "style"    : LoroMap          → style applied if rule is true
          ├── "fg"   : u32 (optional)
          ├── "bg"   : u32
          └── "bold" : bool
```

```rust
pub enum CondFormatKind {
    // Value comparison
    GreaterThan(String),    // String = value or cell reference
    LessThan(String),
    Between(String, String),
    Equal(String),
    // Text
    Contains(String),
    StartsWith(String),
    // Custom formula
    Formula(String),        // e.g. "=A1>AVERAGE(A:A)"
    // Presets
    IsEmpty,
    IsNotEmpty,
    IsError,
    // Color scales (computed over the range)
    ColorScale2 { min_color: u32, max_color: u32 },
    ColorScale3 { min_color: u32, mid_color: u32, max_color: u32 },
    // Data bars
    DataBar { color: u32 },
    // Top/Bottom N
    TopN(u32),
    BottomN(u32),
    TopNPercent(f32),
}
```

### Evaluation

Rules are evaluated in the Logic thread after each `compute_dirty_batch` for cells in the viewport. Evaluation produces a per-cell `style_override` injected into `DisplayCell.style` — it does not modify `LoroMap("styles")`.

```rust
fn apply_cond_formats(&self, cell: CellRef, base_style: &CellStyle) -> CellStyle {
    let sheet = cell.sheet;
    let rules = self.load_cond_formats(sheet); // from LoroDoc, sorted by priority ASC

    // First matching rule wins (Google Sheets behavior)
    for rule in &rules {
        if !self.cell_in_range(cell, &rule.range) { continue; }
        if self.eval_cond_rule(cell, &rule.rule) {
            return self.merge_styles(base_style, &rule.style);
        }
    }
    base_style.clone()
}

fn eval_cond_rule(&self, cell: CellRef, rule: &CondFormatRule) -> bool {
    match &rule.kind {
        CondFormatKind::GreaterThan(val) => {
            let cell_val = self.formula_cache.get(&cell).cloned().unwrap_or(CellValue::Empty);
            let cmp_val  = self.resolve_value_or_ref(cell.sheet, val);
            matches!((cell_val, cmp_val), (CellValue::Number(a), CellValue::Number(b)) if a > b)
        }
        CondFormatKind::Formula(formula) => {
            // Compile and evaluate the formula in the context of the current cell
            // Resolves relative references (A1 in the rule = current cell)
            let result = self.eval_formula_in_context(cell, formula);
            matches!(result, CellValue::Boolean(true) | CellValue::Number(n) if n != 0.0)
        }
        CondFormatKind::ColorScale2 { min_color, max_color } => {
            // Gradient computation: requires range min/max -> computed once per rule
            false // implemented in Phase 2
        }
        // ... other kinds
    }
}
```

**Interaction with normal styles:** the conditional formatting `style_override` is applied on top of the base style. `None` fields in `style_override` leave the base style intact.

---

## 4. UNDO / REDO

### Model: `loro::UndoManager` — Local-Only Undo

Undo only undoes **local peer** operations (current user). Remote operations are never undone. This is the behavior of Google Sheets and all modern collaborative editors.

**Scope:** per open document. The `UndoManager` is created on document open and destroyed on close. No undo stack persistence between sessions.

```rust
pub struct TrameState {
    // ...existing fields...
    undo_manager: loro::UndoManager,
}

impl TrameState {
    pub fn new(doc: LoroDoc, /* ... */) -> Self {
        let undo_manager = loro::UndoManager::new(&doc);
        // Exclude non-undoable metadata fields if needed
        Self { doc, undo_manager, /* ... */ }
    }
}
```

**Undo commit granularity:**

Each atomic user operation constitutes an undo item. Granularity follows Google Sheets:
- `SetCellText` + confirmation (Enter or click elsewhere) → 1 undo item
- Rapid succession of keystrokes in the same cell → 1 undo item (commit on cell exit)
- `InsertRow` → 1 undo item
- `SortRange` → 1 undo item (undoes the entire reordering)
- `ApplyFilter` (shared filter) → **undoable** — the filter is a `LoroDoc` mutation like any other. `UndoManager` tracks it automatically. Ctrl+Z removes the filter.
- `ActivateFilterView` (personal view) → **non-undoable** — it's a local view state, not a doc mutation. Not in `UndoManager`.
- `AddCondFormat`, `RemoveCondFormat` → undoable (LoroDoc mutations)
- `AddValidation`, `RemoveValidation` → undoable (LoroDoc mutations)
- Formatting (`SetStyle`) → 1 undo item

> **Note:** Google Sheets makes filter non-undoable. Trame makes the opposite choice — it's more consistent with the model "everything in LoroDoc is undoable". The user experience is better (no mental exception to remember).

**Implementation:**

A cell edit (`SetCellText`) writes the raw string into `cells_{sheet}`; Loro's
`UndoManager` observes the mutation through its internal subscription (nothing to
register manually), and the undo item is sealed on cell exit by `CommitEdit` (1
edit = 1 undo item).

As-built, undo and redo share one path, `apply_history(undo: bool)`
(`logic/commands.rs`): call `undo_manager.undo()`/`redo()`, then rebuild **all**
derived caches through the same `after_structural()` used by structural ops and
broadcast the inverse ops as a fresh delta. There is **no** `doc.diff` against a
stored previous version (the state keeps none) and **no** partial "touched cells"
rebuild -- the reseed is total, which is why the filter hidden-set is never stale
after an undo.

```rust
fn apply_history(&mut self, undo: bool) -> TrameOutcome {
    let changed = if undo { self.undo_manager.undo() } else { self.undo_manager.redo() };
    match changed {
        Ok(true) => {}                                   // something was un/redone
        _ => return TrameOutcome::display_only(vec![self.undo_state_event()]),
    }
    // Full derived-cache reseed + delta broadcast, exactly like after_structural:
    // rebuild_index -> reseed_engine -> rebuild_merges -> reseed_styles ->
    // rebuild_filter (so filter_hidden is recomputed, never stale) -> flush_delta.
    let mut outcome = self.after_structural();
    outcome.display.push(self.undo_state_event());       // { can_undo, can_redo }
    outcome
}
```

**Keyboard shortcut:** `Ctrl+Z` / `Ctrl+Y` (or `Cmd` on macOS) → `TrameCommand::Undo` / `TrameCommand::Redo` from the UI Thread.

---

## 5. DATA VALIDATION

### CRDT Storage

```
"validations_{sheet_id}" (LoroMap)
  └── "{row_uid}:{col_uid}:{row_uid_end}:{col_uid_end}" (LoroMap)
      ├── "kind"       : String    → see ValidationKind
      ├── "params"     : LoroMap   → parameters based on kind
      ├── "on_invalid" : String    → "warn" | "reject" | "info"
      └── "message"    : String    → displayed message (encrypted in name_enc style)
```

```rust
pub enum ValidationKind {
    List(Vec<String>),          // dropdown -- fixed values
    ListRange(RangeDep),        // dropdown -- values from a range
    NumberRange { min: f64, max: f64, inclusive: bool },
    NumberInteger,
    DateRange { min: Option<String>, max: Option<String> },
    TextLength { min: Option<u32>, max: Option<u32> },
    Regex(String),
    Formula(String),            // custom formula -> must return TRUE
}

pub enum ValidationOnInvalid {
    Reject, // P2 blocks the mutation -> TrameEvent::ValidationError
    Warn,   // P2 allows through + TrameEvent::ValidationWarning
    Info,   // visual indicator only
}
```

### Evaluation on Input

```rust
fn handle_set_cell_text(&mut self, row: u32, col: u32, text: String) -> Vec<TrameEvent> {
    // Positional edit on the active sheet (SetCellText carries no sheet_id).
    let cell_ref = self.resolve_cell(self.active_sheet, row, col);

    // Check validation before Loro mutation
    if let Some(validation) = self.find_validation(self.active_sheet, cell_ref) {
        let result = self.eval_validation(&validation, &text, self.active_sheet);
        // ValidationError/Warning still carry a `version` field; SetCellText is
        // optimistic and version-less, so it is 0 here (Post-MVP: thread an edit
        // sequence if echo-matching is ever needed).
        match (result, &validation.on_invalid) {
            (false, ValidationOnInvalid::Reject) => {
                // Mutation blocked -- no CRDT delta emitted
                return vec![TrameEvent::ValidationError {
                    version: 0, row, col,
                    message: validation.message.clone(),
                }];
            }
            (false, ValidationOnInvalid::Warn) => {
                // Mutation allowed but warning added to returned events
                let events = vec![TrameEvent::ValidationWarning {
                    version: 0, row, col,
                    message: validation.message.clone(),
                }];
                // Continue to mutation -- additional events added below
                self.apply_edit_cell(cell_ref, &text);
                // ... compute_all_dirty, build_viewport, etc.
                return events;
            }
            _ => {} // Info or validation passed -> continue
        }
    }

    // Loro mutation
    self.apply_edit_cell(cell_ref, &text);
    // ... compute_all_dirty, build_viewport, etc.
    vec![] // events added by the caller (dispatch)
}
```

> **Fundamental limitation — CRDT convergence:** `Reject` validation blocks **local** input only. A remote delta sent by a collaborator (who doesn't have the same validation, or whose client doesn't respect it) will be applied via SecSync even if it violates the local rule. This delta triggers a `ValidationWarning` to the UI but **is not rejected** — rejecting it would break CRDT convergence.
>
> **Documented consequence:** validations are an input aid, not an integrity barrier. They prevent accidental errors by official AMBRE client users. They do not guarantee that the document will never contain out-of-validation values. This point must be clearly communicated in the validation configuration UI.

---

## 6. RANGE PROTECTION (Post-MVP)

> **Status: Post-MVP.** Implemented after V1. The fundamental limitation is documented explicitly in the UI.

### Security Model

The server is a Blind Relay. It cannot verify that a CRDT delta respects a protection — content is encrypted. **Protection is soft-enforcement on the client side.** An official AMBRE client respects it. A modified client can bypass it.

This model is documented in the protection invitation UI: *"Protection prevents accidental modifications. It does not constitute a cryptographic guarantee."*

### CRDT Storage

```
"protections_{sheet_id}" (LoroMap)
  └── "{row_uid}:{col_uid}:{row_uid_end}:{col_uid_end}" (LoroMap)
      ├── "owner_user_id"    : String     → creator's UserId as Uuid hex (stable, per-user)
      ├── "allowed_user_ids" : LoroList   → [UserId as uuid_hex, ...] — empty = owner only
      └── "description"      : String     → optional
```

> **Note: `UserId` and not `pub_key`.** The Ed25519 public key is a per-device credential
> that changes on key rotation (member revocation). Using `pub_key` would break
> protections after each rotation. `UserId: Uuid` is the canonical per-user identifier,
> stable. The `pub_key → UserId` resolution is done in `ambre-sync` — the Logic thread
> only sees `UserId` (see `ambre-spec-architecture.md` §2.D and the `RemoteCursor` pattern
> in `ambre-spec-prose-logic.md` §C).

### Enforcement on the Logic Thread Side

```rust
fn check_protection(&self, sheet: SheetId, cell: CellRef) -> ProtectionResult {
    // my_user_id is a UserId received via LogicEvent::SessionReady.
    // No crypto types in the Logic thread -- UserId stable per-user.
    let my_user_id = self.my_user_id;

    for (range_key, protection) in self.load_protections(sheet) {
        if !self.cell_in_protection_range(cell, &range_key, sheet) { continue; }

        let allowed: Vec<UserId> = self.load_allowed_user_ids(&protection);
        let owner = self.load_owner_user_id(&protection);

        if my_user_id == owner || allowed.contains(&my_user_id) {
            return ProtectionResult::Allowed;
        } else {
            return ProtectionResult::Denied { description: self.load_description(&protection) };
        }
    }
    ProtectionResult::Allowed
}

fn handle_set_cell_text(&mut self, row: u32, col: u32, text: String) -> Vec<TrameEvent> {
    let cell_ref = self.resolve_cell(self.active_sheet, row, col);

    match self.check_protection(self.active_sheet, cell_ref) {
        ProtectionResult::Denied { description } => {
            return vec![TrameEvent::ProtectionDenied {
                version: 0, row, col,
                description,
            }];
            // No Loro mutation, no CRDT delta
        }
        ProtectionResult::Allowed => {}
    }
    // ...normal flow (validation, mutation, compute_all_dirty)
    vec![]
}
```

**On receiving a remote update:**

Post-hoc protection enforcement needs the delta's **author**, which the MVP
`apply_remote(&mut self, payload: &[u8]) -> Vec<TrameEvent>` (`-logic.md` §8) does
**not** receive. Enforcing it (Post-MVP) therefore requires threading the
Sync-resolved `from_user_id` into `apply_remote` -- the same trusted,
spoof-proof identity `apply_ephemeral` already gets (`-logic.md` §8, `-sync.md §3`),
never an id read from the payload. The signature below shows that Post-MVP
extension:

```rust
/// Applies a remote delta (forwarded by the Logic dispatcher from
/// LogicEvent::RemoteUpdate) and returns display TrameEvent + violation
/// TrameEvent. `from_user_id` is the Sync-resolved author (Post-MVP param, only
/// needed for range-protection enforcement). Import errors are surfaced, not
/// unwrapped.
fn apply_remote(&mut self, payload: &[u8], from_user_id: UserId) -> Vec<TrameEvent> {
    if self.doc.import(payload).is_err() { return vec![]; }
    let diff = /* ... */;
    let mut events = Vec::new();

    for changed_cell in self.extract_changed_cells(diff) {
        // Post-hoc protection check
        if let ProtectionResult::Denied { description } =
            self.check_protection_for_user(changed_cell.sheet, changed_cell, &from_user_id)
        {
            // The delta IS APPLIED (mandatory CRDT convergence)
            // but the UI is warned via a returned event
            events.push(TrameEvent::ProtectionViolationWarning {
                row: /* ... */, col: /* ... */,
                from_user_id,
                description,
            });
        }
        // Normal flow: dirty, recompute, etc.
    }
    events
}
```

---

## 7. ADDITIONAL MESSAGES (TrameEvent)

> **Source of truth:** the `TrameEvent` enum lives in **`ambre-bus`** (`ambre-bus/src/app/trame/event.rs`, re-exported flat at the crate root and by `ambre-trame`) and constitutes the payload of `AppEvent::Trame`. `ambre-spec-trame-logic.md §9` documents the base; the variants below are **additions** for the features of this spec.
>
> **No `serde`.** `TrameEvent` travel **by move** on `flume` (shared memory) inside `AppEvent::Trame` — no (de)serialization. Derives: `#[derive(Debug, Clone)]`.

```rust
// Additions to TrameEvent for the features of this spec
// (base definition in ambre-spec-trame-logic.md sec.9; type in ambre-bus)
#[derive(Debug, Clone)]
pub enum TrameEvent {
    // ... existing variants from the logic spec ...

    // Filters -- shared filter implemented field-light (the accompanying
    // DataReset prompts the viewport re-fetch; hidden rows arrive flagged via
    // DisplayCell::is_filtered). Matches the implemented ambre_bus::TrameEvent.
    FilterApplied   { hidden_row_count: u32, col: u32 }, // `col` -> funnel badge on the header
    FilterCleared,
    FilterViewSaved { view_id: String }, // Phase 2 (personal views)

    // Sort -- implemented field-less (the accompanying DataReset prompts the
    // viewport re-fetch; SortComplete is the explicit completion signal).
    SortComplete,

    // Conditional formatting
    CondFormatsUpdated { version: u64 },

    // Undo/Redo -- single event for undo, redo, and empty-stack states.
    // Matches the implemented `ambre_bus::TrameEvent::UndoState`.
    UndoState       { can_undo: bool, can_redo: bool },

    // Validation
    ValidationError   { version: u64, row: u32, col: u32, message: String },
    ValidationWarning { version: u64, row: u32, col: u32, message: String },

    // Protection (Post-MVP)
    ProtectionDenied  { version: u64, row: u32, col: u32, description: String },
    ProtectionViolationWarning { row: u32, col: u32, from_user_id: UserId, description: String },
}
```

---

## 8. INTERACTIONS WITH THE ENGINE

**Filters and calculations:**
- Filtered rows are excluded from `DisplayViewport` but **not** from `SUM`, `COUNT`, etc. calculations. To exclude hidden rows, use `SUBTOTAL` (Phase 2).
- The `is_filtered` flag is added to `DisplayCell` for the UI Thread to manage rendering.

**Sort and DAG:**
- The sort permutes the `rows_{sheet}` order list (one `mov` per displaced row, see §2); cells
  are keyed by their move-invariant `Uid`, so no cell is rewritten. The engine's uid-anchored
  bytecode is **preserved** (no reseed/re-parse); the sort just rebuilds the `SheetIndex` and
  recomputes, refreshing the `formula_cache` from the new positions.
- The `SheetIndex` row order is permuted to mirror the moves; each row's `Uid` -- and the cells
  keyed by it -- travel together.
- Single-cell references **follow the data**: a `CellId` binds each axis to its row/column `Uid`
  (`trame-logic §3`) resolved to a position at eval, so a reference to a sorted row still points
  at the same physical cell after the sort (`trame-logic §12`). A *range* is a positional block
  (`Virtual` endpoints): sorting the rows inside `SUM(A1:A3)` reorders the same values, so the
  block total is unchanged -- uid-bound endpoints would scatter and invert (`trame-logic §13`).

**Conditional formatting and dirty:**
- Conditional formatting rules are re-evaluated after each `compute_dirty_batch` for cells in the viewport.
- Cells whose conditional style changes but not their value still generate an updated `DisplayViewport`.

**Undo and CRDT schema:**
- `UndoManager` undoes Loro mutations. If an `InsertRow` is undone, the `SheetIndex` must be rebuilt for the affected sheet.
- Validations and protections are not undoable (they are configuration metadata, not data).

---

## 9. QUALITY & TESTABILITY

> What the built features guarantee, and what verifies each guarantee, is the table in §0. This section used to be a list of 30 `#[test]` names; 18 named real tests and 12 named tests for handlers that are still no-ops, with nothing distinguishing the two. The 18 are now `Verification` cells joined to a rule; the 12 are below, as what they always were -- intent.

* **Tier.** host, entirely (`just test-native`). Every feature here runs in the Logic thread on pure Rust.
* **Mandatory belts per built feature.** A view feature (filter) lands with: a positive case, a no-op guard, a convergence case in the 2-peer proptest, and an "it does not change the data" case (`TRAME-FEAT-003` is the model). A mutation feature (sort) lands with: the permutation assertion, a header-pin case, a no-delta case, and a "references follow" case owned by `trame-logic §1`.
* **Unbuilt handlers.** Their rules are `TRAME-FEAT-014..024` in §0, not prose here. When one of those handlers lands, its rows move from `unimplemented` to a `Verification` cell naming the test -- which is the whole point of writing them now rather than later.
* **Definition of Done.** An increment is `[x]` only when its belts are green in CI **and** no invariant in §0 is left without a `Status`.

---

## 10. ROADMAP

> Tracked as Stories of Epic **E3** (Trame MVP) in `specs/ambre-roadmap.md`.

**Phase 1 — MVP (with the engine)**
- [x] Shared filter: `TrameCommand::ApplyFilter` / `ClearFilter`, `filter_hidden_{sheet}` Loro *(Epic `E3`: `FilterKind::Values` / `Condition { op, value }` typed in `ambre-bus`; the failing rows' `row_uid`s are written into `filter_hidden_{sheet}` and broadcast, the definition persisted flat in `filters_{sheet}` (LWW). Hidden rows ride the viewport flagged `DisplayCell::is_filtered` (still counted by `=SUM`); the uid-keyed hidden set follows the data across a sort/insert, and `SetCellText` recomputes the active filter. Undoable (plain CRDT mutation). Emits `TrameEvent::FilterApplied { hidden_row_count, col }` / `FilterCleared` + `DataReset`. Tests: by-values, condition, sum-unaffected, clear, follows-sort, recompute-on-edit, remote-import, LWW convergence, undoable, no-op guards, full predicate coverage.)*
- [x] Simple sort: `TrameCommand::SortRange` (single column) *(Epic `E3`: typed sort keys via `engine.evaluated_value` + `compare_cell_values` precedence `Number < Text < Boolean < Error < Empty`; `has_header` pin, ascending/descending. Realized as a **pure `rows_{sheet}` order permutation** (`MovableList::mov`): cells keyed by their move-invariant `Uid` follow for free, references follow the data (`trame-logic §2.A`, §12). Emits `TrameEvent::SortComplete` + `DataReset`. Tests: ascending whole-row move, descending+header, formulas follow data, no-op guards.)* **Depends on the own-uid cell model (`trame-logic §14`).**
- [x] **Multi-column sort: `TrameCommand::SortRangeMulti`** *(promoted from Phase 2 to MVP; same pure permutation as single-column sort, lexicographic comparator over `sort_keys`. UI: `SortDialog`, `-ui` §5.C.)* *(Epic `E3`: `sort_range_multi` keys each sortable row by the evaluated value in every sort column and `sort_by`s lexicographically over the keys -- first non-equal column decides, each key applying its own ascending/descending direction via `compare_cell_values`. Out-of-range keys are dropped; the permutation is realized through the shared `realize_sort` helper (`mov` ops on `rows_{sheet}`, uid-anchored bytecode preserved). Both the typed ranged `SortRangeMulti` and the parameterless `SortSelectionMulti` (Logic resolves the range from the tracked selection, driven by the `SortDialog`) are wired. Emits `SortComplete` + `DataReset`.)*
- [x] **Filter feedback on the first row** (UI story, `-ui` §5.D): Excel-style autofilter dropdown buttons on the filter range's header row, fed by `FilterApplied { col }`. *(Logic side unchanged -- the existing shared filter; this is the UX surface the funnel-only badge lacked. Delivered: always-on buttons on the active cell's current region (resolved UI-side from the viewport cache), chevron/funnel keyed off `filtered_col`, a click selects the column and opens the existing `FilterPopover` via a one-shot `FilterOpenPublisher` canvas->launcher intent that re-runs the context-menu `FILTER_COLUMN` path -- one filter code path, two entry points. See `-ui` §5.D.)*
- [x] Undo/Redo: `TrameCommand::Undo` / `Redo` via `UndoManager` Loro *(delivered under `trame-logic` Phase 1 -- see that section)*
- [x] All feature variants are in the `TrameCommand` / `TrameEvent` enums (defined in `ambre-bus`, re-exported by `ambre-trame`; transported in `AppCommand::Trame` / `AppEvent::Trame`) *(Epic `E3`: commands `SaveFilterView` / `ActivateFilterView` / `SortRangeMulti` (+ `SortKey`) / `AddCondFormat` / `UpdateCondFormat` / `RemoveCondFormat` / `AddValidation` / `RemoveValidation` / `SetProtection` / `RemoveProtection` with payload types `GridRange`, `CondFormatKind`, `CondStyle`, `ValidationKind`, `ValidationOnInvalid`; events `FilterViewSaved` / `CondFormatsUpdated` / `ValidationError` / `ValidationWarning` / `ProtectionDenied` / `ProtectionViolationWarning` (`from_user_id: UserId`). Ranges carried in grid coordinates at the bus level (`RangeDep` is engine-private). Logic-thread handlers are explicit no-ops until their phase lands.)*
- [x] **Pure** handlers: return `TrameEvent` (display) and/or a `TrameOutcome` (CRDT delta → `SyncCommand::SendUpdate` by the Logic dispatcher) — no direct inter-thread send from the app model (preserves host testability, cf. `ambre-spec-architecture.md §4`) *(Epic `E3`: `TrameState::handle_command` returns `TrameOutcome { display, update, ephemeral }`; `TrameState` holds no sink/sender. The Logic dispatcher (`ambre-logic::LogicState`) fans the outcome out -- `display → UiEvent::Viewport`, `update → SyncCommand::SendUpdate`, `ephemeral → SendEphemeral` -- adding the `DocumentId` it holds. `CommandSink` remains the test seam (`MockSyncHandle`).)*
- [x] **Tested DoD (test debt, Epic `ET`).** All MVP `#[test]`s listed in §9 (filters, simple sort, undo/redo) green on the **host** tier in CI -- including the three Phase-1 undo belts (`undo_local_only_not_remote`, `undo_insert_row_rebuilds_sheet_index`, `undo_stack_cleared_on_doc_close`); the concurrent-filter LWW case (`concurrent_filters_converge_lww`) folds into the Trame 2-peer convergence property-test, which now drives `ApplyFilter`/`ClearFilter` ops and asserts the LWW-elected `is_filtered` set in its converged snapshot (`logic/convergence_tests.rs`, `trame-logic §14`, `ambre-spec-architecture.md §4`). *(The conditional-formatting / validation / protection `#[test]`s in §9 follow their Phase 2 / Post-MVP implementation.)*

**Phase 2 — V1**
- [ ] Personal filter views (`TrameCommand::ActivateFilterView` / `SaveFilterView`, IndexedDB)
- [ ] Conditional formatting: `TrameCommand::AddCondFormat` / `RemoveCondFormat` / `UpdateCondFormat`, basic kinds (value comparison, `IsEmpty`, `Formula`)
- [ ] Data validation: `TrameCommand::AddValidation` / `RemoveValidation`, kinds `List`, `NumberRange`, `TextLength`
- [ ] `SUBTOTAL` function in the engine (excludes filtered rows)

**Phase 3 — Post-MVP**
- [ ] Conditional formatting: `ColorScale2`, `ColorScale3`, `DataBar`, `TopN`
- [ ] Range protection: `TrameCommand::SetProtection` / `RemoveProtection` (soft-enforcement, identified by `UserId: Uuid` — not `pub_key`)
- [ ] Validation: `ListRange`, `DateRange`, `Regex`
- [ ] Optimization: cache of pre-compiled conditional formatting rules
