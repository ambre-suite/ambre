# AMBRE — SPEC: AMBRE-TRAME FEATURES
**Type:** Vertical (Feature / Application)
**Status:** Draft

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
fn sort_range(&mut self, cmd: SortRangeCmd) {
    let sheet_idx = &self.sheet_indices[&cmd.sheet_id];

    // Resolve the Uid of the rows in the range
    let rows: Vec<Uid> = (cmd.row_start..=cmd.row_end)
        .filter_map(|r| sheet_idx.row_by_pos.get(r as usize).copied())
        .collect();
    let header = if cmd.has_header { rows.first().copied() } else { None };
    let sort_rows: Vec<Uid> = rows.iter()
        .skip(if cmd.has_header { 1 } else { 0 })
        .copied()
        .collect();

    let sort_col_id = sheet_idx.col_by_pos[cmd.col_start as usize + cmd.sort_col as usize];

    // Fetch values and sort -- stable sort
    let mut keyed: Vec<(Uid, CellValue)> = sort_rows.iter()
        .map(|&row_id| {
            let cell = CellId::bound(cmd.sheet_idx, row_id, sort_col_id);
            let val = self.formula_cache.get(&cell).cloned().unwrap_or(CellValue::Empty);
            (row_id, val)
        })
        .collect();

    keyed.sort_by(|(_, a), (_, b)| {
        let ord = compare_cell_values(a, b); // Empty last, Errors after Empty
        if cmd.ascending { ord } else { ord.reverse() }
    });

    // Reorder LoroMovableList("rows_{sheet}") according to the new order
    // Each move = 1 CRDT operation; cells keyed by uid follow for free.
    let new_order: Vec<Uid> = header.into_iter()
        .chain(keyed.into_iter().map(|(id, _)| id))
        .chain(rows.iter().skip(cmd.row_end as usize + 1).copied())
        .collect();

    self.apply_row_order(cmd.sheet_id, &new_order);
    // The Loro mutation produces a delta: passed back via outcome.update, the
    // Logic dispatcher emits SyncCommand::SendUpdate { document_id, payload }.
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
> `compare_cell_values` uses an explicit type-rank (`Number < Text < Boolean < Error < Empty`).
> Emits `TrameEvent::SortComplete` + `DataReset`.

### Multi-Column Sort

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
- `EditCell` + confirmation (Enter or click elsewhere) → 1 undo item
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

```rust
// TrameCommand::EditCell flow
fn handle_edit_cell(&mut self, cmd: EditCellCmd) {
    // Loro mutations are automatically tracked by UndoManager
    let cells = self.doc.get_map("cells_{}", &cmd.sheet_id);
    cells.insert(&cell_key, "v", cmd.raw);
    // UndoManager observes via internal subscription -- nothing to do manually here
    // The undo commit is triggered by TrameCommand::CommitEdit or timeout
    self.bytecode.invalidate(cell_ref);
    // ...
}

// TrameCommand::Undo
fn handle_undo(&mut self) {
    match self.undo_manager.undo() {
        Ok(_) => {
            // Loro reapplied the previous state in doc
            // Must rebuild: SheetIndex if structure changed,
            // bytecode.invalidate() for all touched cells,
            // dirty for dependents
            self.rebuild_after_undo_redo();
        }
        Err(loro::UndoError::NothingToUndo) => { /* no item in stack */ }
        Err(e) => { /* log */ }
    }
}

// TrameCommand::Redo
fn handle_redo(&mut self) {
    match self.undo_manager.redo() {
        Ok(_)  => self.rebuild_after_undo_redo(),
        Err(_) => {}
    }
}

fn rebuild_after_undo_redo(&mut self) {
    // Diff between previous state and current doc state
    // -> touched cells -> invalidate bytecode -> mark dirty -> send viewport
    let diff = self.doc.diff(/* prev_version */, /* cur_version */);
    for changed_cell in self.extract_changed_cells(diff) {
        self.bytecode.invalidate(changed_cell);
        let dirty = self.deps.collect_dirty(changed_cell, &self.sheet_indices);
        self.dirty.extend(dirty);
    }
    // If structure (rows/columns) changed -> rebuild SheetIndex
    self.rebuild_sheet_indices_if_needed(diff);
    // Send viewport immediately
    self.send_viewport_response();
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
fn handle_edit_cell(&mut self, cmd: EditCellCmd) -> Vec<TrameEvent> {
    let cell_ref = self.resolve_cell(cmd.sheet_id, cmd.row, cmd.col);

    // Check validation before Loro mutation
    if let Some(validation) = self.find_validation(cmd.sheet_id, cell_ref) {
        let result = self.eval_validation(&validation, &cmd.raw, cmd.sheet_id);
        match (result, &validation.on_invalid) {
            (false, ValidationOnInvalid::Reject) => {
                // Mutation blocked -- no CRDT delta emitted
                return vec![TrameEvent::ValidationError {
                    version: cmd.version,
                    row: cmd.row, col: cmd.col,
                    message: validation.message.clone(),
                }];
            }
            (false, ValidationOnInvalid::Warn) => {
                // Mutation allowed but warning added to returned events
                let mut events = vec![TrameEvent::ValidationWarning {
                    version: cmd.version,
                    row: cmd.row, col: cmd.col,
                    message: validation.message.clone(),
                }];
                // Continue to mutation -- additional events added below
                self.apply_edit_cell(cell_ref, &cmd.raw);
                // ... compute_all_dirty, build_viewport, etc.
                return events;
            }
            _ => {} // Info or validation passed -> continue
        }
    }

    // Loro mutation
    self.apply_edit_cell(cell_ref, &cmd.raw);
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
      ├── "owner_user_id"    : String     → creator's Uuid hex (stable, per-user)
      ├── "allowed_user_ids" : LoroList   → [uuid_hex, ...] — empty = owner only
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
    // my_user_id is a Uuid received via LogicEvent::SessionReady.
    // No crypto types in the Logic thread -- UserId stable per-user.
    let my_user_id = self.my_user_id;

    for (range_key, protection) in self.load_protections(sheet) {
        if !self.cell_in_protection_range(cell, &range_key, sheet) { continue; }

        let allowed: Vec<Uuid> = self.load_allowed_user_ids(&protection);
        let owner = self.load_owner_user_id(&protection);

        if my_user_id == owner || allowed.contains(&my_user_id) {
            return ProtectionResult::Allowed;
        } else {
            return ProtectionResult::Denied { description: self.load_description(&protection) };
        }
    }
    ProtectionResult::Allowed
}

fn handle_edit_cell(&mut self, cmd: EditCellCmd) -> Vec<TrameEvent> {
    let cell_ref = self.resolve_cell(cmd.sheet_id, cmd.row, cmd.col);

    match self.check_protection(cmd.sheet_id, cell_ref) {
        ProtectionResult::Denied { description } => {
            return vec![TrameEvent::ProtectionDenied {
                version: cmd.version,
                row: cmd.row, col: cmd.col,
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

```rust
/// Applies a remote delta (forwarded by the Logic dispatcher from
/// LogicEvent::RemoteUpdate) and returns display TrameEvent + violation
/// TrameEvent. The Logic dispatcher wraps them in AppEvent::Trame -> UiEvent::Viewport.
fn apply_remote(&mut self, payload: &[u8]) -> Vec<TrameEvent> {
    self.doc.import(payload).unwrap();
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

> **Source of truth:** the `TrameEvent` enum lives in **`ambre-bus`** (`ambre-bus/src/ui.rs`, re-exported by `ambre-trame`) and constitutes the payload of `AppEvent::Trame`. `ambre-spec-trame-logic.md §9` documents the base; the variants below are **additions** for the features of this spec.
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
    FilterApplied   { hidden_row_count: u32 },
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
    ProtectionViolationWarning { row: u32, col: u32, from_user_id: Uuid, description: String },
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

```rust
// Filters (shared filter implemented, Epic E3 -- see ambre-trame/src/logic.rs tests)
#[test] fn shared_filter_by_values_hides_nonmatching() { /* LoroMap filter_hidden, is_filtered */ }
#[test] fn shared_filter_condition_greater_than()      { /* numeric condition */ }
#[test] fn shared_filter_does_not_affect_sum()         { /* SUM(A:A) includes hidden ones */ }
#[test] fn clear_filter_unhides_and_noops_when_absent(){ }
#[test] fn shared_filter_hidden_follows_sort()         { /* row_uid-keyed hidden set */ }
#[test] fn shared_filter_recomputes_on_edit()          { /* stored def drives recompute */ }
#[test] fn shared_filter_applies_on_remote_import()    { /* peer applies filter_hidden directly */ }
#[test] fn concurrent_filters_converge_lww()           { /* Alice vs Bob filter -> LWW */ }
#[test] fn shared_filter_is_undoable()                 { /* Ctrl+Z removes shared filter */ }
#[test] fn shared_filter_noop_guards()                 { /* wrong sheet / virtual column */ }
#[test] fn condition_pass_covers_every_operator()      { /* predicate branch coverage */ }
// Phase 2:
#[test] fn personal_filter_view_not_synced()           { /* no CRDT delta emitted */ }

// Sort (implemented, Epic E3 -- see ambre-trame/src/logic.rs tests)
#[test] fn sort_range_ascending_moves_whole_rows()     { /* row data travels with key col */ }
#[test] fn sort_range_descending_keeps_header()        { /* has_header pin + reverse order */ }
#[test] fn sort_range_formulas_follow_data()           { /* refs track their physical cell, no #REF! */ }
#[test] fn sort_range_noops()                          { /* wrong sheet / already sorted */ }
// Phase 2:
#[test] fn sort_multi_column_primary_secondary()        { }

// Conditional formatting
#[test] fn first_matching_rule_wins()                  { /* priority */ }
#[test] fn formula_rule_relative_ref()                 { /* =A1>... resolved from target cell */ }
#[test] fn cond_format_does_not_mutate_styles_map()    { /* style_override != LoroMap styles */ }

// Undo/Redo
#[test] fn undo_local_only_not_remote()                { /* Alice undo does not affect Bob */ }
#[test] fn undo_insert_row_rebuilds_sheet_index()      { }
#[test] fn undo_stack_cleared_on_doc_close()           { }
#[test] fn filter_shared_is_undoable()                 { /* Ctrl+Z removes shared filter */ }
#[test] fn filter_view_personal_not_undoable()         { /* personal view activation = not in undo stack */ }

// Validation
#[test] fn reject_blocks_mutation()                    { /* no CRDT delta on Reject */ }
#[test] fn warn_allows_mutation_with_warning()         { }
#[test] fn list_validation_dropdown_values()           { }

// Protection (Post-MVP)
#[test] fn protection_blocks_edit_for_non_owner()      { }
#[test] fn remote_violation_applied_but_warned()       { /* CRDT not rejected */ }
```

---

## 10. ROADMAP

> Tracked as Stories of Epic **E3** (Trame MVP) in `specs/ambre-roadmap.md`.

**Phase 1 — MVP (with the engine)**
- [x] Shared filter: `TrameCommand::ApplyFilter` / `ClearFilter`, `filter_hidden_{sheet}` Loro *(Epic `E3`: `FilterKind::Values` / `Condition { op, value }` typed in `ambre-bus`; the failing rows' `row_uid`s are written into `filter_hidden_{sheet}` and broadcast, the definition persisted flat in `filters_{sheet}` (LWW). Hidden rows ride the viewport flagged `DisplayCell::is_filtered` (still counted by `=SUM`); the uid-keyed hidden set follows the data across a sort/insert, and `SetCellText` recomputes the active filter. Undoable (plain CRDT mutation). Emits `TrameEvent::FilterApplied { hidden_row_count }` / `FilterCleared` + `DataReset`. Tests: by-values, condition, sum-unaffected, clear, follows-sort, recompute-on-edit, remote-import, LWW convergence, undoable, no-op guards, full predicate coverage.)*
- [x] Simple sort: `TrameCommand::SortRange` (single column) *(Epic `E3`: typed sort keys via `engine.evaluated_value` + `compare_cell_values` precedence `Number < Text < Boolean < Error < Empty`; `has_header` pin, ascending/descending. Realized as a **pure `rows_{sheet}` order permutation** (`MovableList::mov`): cells keyed by their move-invariant `Uid` follow for free, references follow the data (`trame-logic §2.A`, §12). Emits `TrameEvent::SortComplete` + `DataReset`. Tests: ascending whole-row move, descending+header, formulas follow data, no-op guards.)* **Depends on the own-uid cell model (`trame-logic §14`).**
- [x] Undo/Redo: `TrameCommand::Undo` / `Redo` via `UndoManager` Loro *(delivered under `trame-logic` Phase 1 -- see that section)*
- [x] All feature variants are in the `TrameCommand` / `TrameEvent` enums (defined in `ambre-bus`, re-exported by `ambre-trame`; transported in `AppCommand::Trame` / `AppEvent::Trame`) *(Epic `E3`: commands `SaveFilterView` / `ActivateFilterView` / `SortRangeMulti` (+ `SortKey`) / `AddCondFormat` / `UpdateCondFormat` / `RemoveCondFormat` / `AddValidation` / `RemoveValidation` / `SetProtection` / `RemoveProtection` with payload types `GridRange`, `CondFormatKind`, `CondStyle`, `ValidationKind`, `ValidationOnInvalid`; events `FilterViewSaved` / `CondFormatsUpdated` / `ValidationError` / `ValidationWarning` / `ProtectionDenied` / `ProtectionViolationWarning` (`from_user_id: UserId`). Ranges carried in grid coordinates at the bus level (`RangeDep` is engine-private). Logic-thread handlers are explicit no-ops until their phase lands.)*
- [x] **Pure** handlers: return `TrameEvent` (display) and/or a `TrameOutcome` (CRDT delta → `SyncCommand::SendUpdate` by the Logic dispatcher) — no direct inter-thread send from the app model (preserves host testability, cf. `ambre-spec-architecture.md §4`) *(Epic `E3`: `TrameState::handle_command` returns `TrameOutcome { display, update, ephemeral }`; `TrameState` holds no sink/sender. The Logic dispatcher (`ambre-logic::LogicState`) fans the outcome out -- `display → UiEvent::Viewport`, `update → SyncCommand::SendUpdate`, `ephemeral → SendEphemeral` -- adding the `DocumentId` it holds. `CommandSink` remains the test seam (`MockSyncHandle`).)*

**Phase 2 — V1**
- [ ] Personal filter views (`TrameCommand::ActivateFilterView` / `SaveFilterView`, IndexedDB)
- [ ] Multi-column sort: `TrameCommand::SortRangeMulti`
- [ ] Conditional formatting: `TrameCommand::AddCondFormat` / `RemoveCondFormat` / `UpdateCondFormat`, basic kinds (value comparison, `IsEmpty`, `Formula`)
- [ ] Data validation: `TrameCommand::AddValidation` / `RemoveValidation`, kinds `List`, `NumberRange`, `TextLength`
- [ ] `SUBTOTAL` function in the engine (excludes filtered rows)

**Phase 3 — Post-MVP**
- [ ] Conditional formatting: `ColorScale2`, `ColorScale3`, `DataBar`, `TopN`
- [ ] Range protection: `TrameCommand::SetProtection` / `RemoveProtection` (soft-enforcement, identified by `UserId: Uuid` — not `pub_key`)
- [ ] Validation: `ListRange`, `DateRange`, `Regex`
- [ ] Optimization: cache of pre-compiled conditional formatting rules
