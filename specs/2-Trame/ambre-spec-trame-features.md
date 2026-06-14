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

**CRDT Schema:**

```
"filters_{sheet_id}" (LoroMap)
  └── "active" (LoroMap)         → active filter (null = no filter)
      ├── "col_lid"  : String    → column the filter is applied to
      ├── "kind"     : String    → "values" | "condition" | "color"
      ├── "values"   : LoroList  → list of displayed values (for kind="values")
      └── "condition": LoroMap   → {op: String, value: String} (for kind="condition")
          // op : "eq" | "ne" | "gt" | "lt" | "gte" | "lte" | "contains"
          //      | "not_contains" | "starts_with" | "is_empty" | "is_not_empty"
```

**Hidden rows:**

Rows hidden by the filter are stored in a separate `LoroMap` to avoid polluting `rows_{sheet_id}`:

```
"filter_hidden_{sheet_id}" (LoroMap)
  └── "{row_lid}" : bool         → true = hidden by the active filter
```

This `LoroMap` is recomputed by the Logic thread after each change to the filter or data. Only the filter author recomputes it — other clients receive `filter_hidden` via SecSync and apply it directly without recomputing.

**Flow:**

```
TrameCommand::ApplyFilter { sheet_id, col, kind, values/condition }
  │
  ├─ Evaluates the column cells via formula_cache
  ├─ Computes the ListItemId of rows not passing the filter
  ├─ Writes hidden rows into LoroMap("filter_hidden_{sheet}")
  ├─ display : updated viewport (without hidden rows) → TrameEvent::ViewportData
  └─ update  : CRDT delta from the Loro mutation → outcome.update → SyncCommand::SendUpdate (by the Logic dispatcher)

TrameCommand::ClearFilter { sheet_id }
  │
  └─ Removes all entries from "filter_hidden_{sheet}" + "filters_{sheet}/active"
```

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
    pub filter_col:  ListItemId,
    pub kind:        FilterKind,
    pub hidden_rows: HashSet<ListItemId>, // recomputed on activation
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

    // Resolve the ListItemId of the rows in the range
    let rows: Vec<ListItemId> = (cmd.row_start..=cmd.row_end)
        .filter_map(|r| sheet_idx.row_by_pos.get(r as usize).copied())
        .collect();
    let header = if cmd.has_header { rows.first().copied() } else { None };
    let sort_rows: Vec<ListItemId> = rows.iter()
        .skip(if cmd.has_header { 1 } else { 0 })
        .copied()
        .collect();

    let sort_col_id = sheet_idx.col_by_pos[cmd.col_start as usize + cmd.sort_col as usize];

    // Fetch values and sort -- stable sort
    let mut keyed: Vec<(ListItemId, CellValue)> = sort_rows.iter()
        .map(|&row_id| {
            let cell = CellRef { sheet: cmd.sheet_id, row: row_id, col: sort_col_id };
            let val = self.formula_cache.get(&cell).cloned().unwrap_or(CellValue::Empty);
            (row_id, val)
        })
        .collect();

    keyed.sort_by(|(_, a), (_, b)| {
        let ord = compare_cell_values(a, b); // Empty last, Errors after Empty
        if cmd.ascending { ord } else { ord.reverse() }
    });

    // Reorder LoroMovableList("rows_{sheet}") according to the new order
    // Each move = 1 CRDT operation
    let new_order: Vec<ListItemId> = header.into_iter()
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

> **Implemented (Epic `E3`) -- diverges from the pseudocode above.** The single-column
> sort is realized by **rewriting cell contents into fixed row slots**, not by reordering
> `rows_{sheet}` with `MovableList::mov`. Reason: cells are keyed by the row slot's stable
> id (`ListItemId`), but Loro reassigns a moved slot's id, which would orphan every cell
> keyed to it (verified empirically). The slots therefore stay put and each row's
> values/formatting are written back in sorted order. Because the formula engine is
> positional (`reseed_engine` recompiles each cell at its current grid position), formula
> references stay **positional after the sort** -- Excel relative-reference behavior, *not*
> the `ListItemId`-follow ideal claimed in §8 and `trame-logic §12` (which requires id-based
> references the positional engine does not yet model). `compare_cell_values` is implemented
> with an explicit type-rank (`Number < Text < Boolean < Error < Empty`). Emits
> `TrameEvent::SortComplete` + `DataReset`.

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
      ├── "range"    : String           → "{row_lid}:{col_lid}:{row_lid_end}:{col_lid_end}"
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
  └── "{row_lid}:{col_lid}:{row_lid_end}:{col_lid_end}" (LoroMap)
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
  └── "{row_lid}:{col_lid}:{row_lid_end}:{col_lid_end}" (LoroMap)
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

    // Filters
    FilterApplied   { version: u64, hidden_row_count: u32 },
    FilterCleared   { version: u64 },
    FilterViewSaved { view_id: String },

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

**Sort and DAG (as implemented, Epic `E3`):**
- The sort rewrites cell contents into fixed row slots (see §2), so the engine is fully
  reseeded afterwards (`after_structural`): the DAG and `formula_cache` are rebuilt from the
  new layout, not preserved.
- The `SheetIndex` is unchanged by the sort (the row slots do not move); only the cell
  values/formatting they hold are reassigned.
- Formula references are **positional**: a formula's text travels with its row, but it is
  recompiled at the new grid position, so `=A1` reads whatever is now in A1 (Excel
  relative-reference behavior). The `ListItemId`-follow ideal would require id-based
  references, which the positional engine does not model.

**Conditional formatting and dirty:**
- Conditional formatting rules are re-evaluated after each `compute_dirty_batch` for cells in the viewport.
- Cells whose conditional style changes but not their value still generate an updated `DisplayViewport`.

**Undo and CRDT schema:**
- `UndoManager` undoes Loro mutations. If an `InsertRow` is undone, the `SheetIndex` must be rebuilt for the affected sheet.
- Validations and protections are not undoable (they are configuration metadata, not data).

---

## 9. QUALITY & TESTABILITY

```rust
// Filters
#[test] fn shared_filter_hides_rows()                  { /* LoroMap filter_hidden */ }
#[test] fn filter_does_not_affect_sum()                { /* SUM(A:A) includes hidden ones */ }
#[test] fn personal_filter_view_not_synced()           { /* no CRDT delta emitted */ }
#[test] fn concurrent_filters_lww()                    { /* Alice vs Bob filter -> LWW */ }

// Sort (implemented, Epic E3 -- see ambre-trame/src/logic.rs tests)
#[test] fn sort_range_ascending_moves_whole_rows()     { /* row data travels with key col */ }
#[test] fn sort_range_descending_keeps_header()        { /* has_header pin + reverse order */ }
#[test] fn sort_range_formulas_stay_positional()       { /* refs positional, no #REF! */ }
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
- [ ] Shared filter: `TrameCommand::ApplyFilter` / `ClearFilter`, `filter_hidden_{sheet}` Loro
- [x] Simple sort: `TrameCommand::SortRange` (single column) *(Epic `E3`: typed sort keys via `engine.evaluated_value` + `compare_cell_values` precedence `Number < Text < Boolean < Error < Empty`; `has_header` pin, ascending/descending. Implemented by **rewriting cell contents** into fixed row slots, not by `MovableList::mov` -- a moved slot's `ListItemId` is reassigned by Loro and would orphan its cells. Consequence: references stay **positional** (Excel relative-ref behavior), not the `ListItemId`-follow ideal of `trame-logic §12`, which needs id-based references the positional engine does not model. Emits `TrameEvent::SortComplete` + `DataReset`. Tests: ascending whole-row move, descending+header, positional formulas, no-op guards.)*
- [x] Undo/Redo: `TrameCommand::Undo` / `Redo` via `UndoManager` Loro *(delivered under `trame-logic` Phase 1 -- see that section)*
- [ ] All feature variants are in the `TrameCommand` / `TrameEvent` enums (defined in `ambre-bus`, re-exported by `ambre-trame`; transported in `AppCommand::Trame` / `AppEvent::Trame`)
- [ ] **Pure** handlers: return `TrameEvent` (display) and/or a `TrameOutcome` (CRDT delta → `SyncCommand::SendUpdate` by the Logic dispatcher) — no direct inter-thread send from the app model (preserves host testability, cf. `ambre-spec-architecture.md §4`)

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
