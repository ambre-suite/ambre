# AMBRE — SPEC: TRAME LOGIC (Spreadsheet Engine)
**Type:** Vertical (Feature / Application — Logic)
**Status:** Draft

> Trame collaborative spreadsheet engine: multi-sheet CRDT state management (`LoroDoc`), incremental formula compilation to bytecode, evaluation via stack VM, two-level dependency graph, synchronous recomputation in the `AppController` contract, and production of data-oriented structures for rendering.
>
> **Scope of this spec:** The engine (`TrameController`) that runs in the **Logic thread** (WASM Worker 1). Trame is a complete Logic + UI application — rendering (QuadRenderer/DrawSurface), user interactions, and the UI thread are covered in a separate spec (`spec-ambre-trame-ui.md`). Features (sort, filters, conditional formatting, undo/redo, validation) are covered in `spec-ambre-trame-features.md`. xlsx/PDF/CSV interoperability is covered in `spec-ambre-trame-interop.md`.
>
> `LoroDoc` is instantiated in the Logic thread and never leaves it.

---

## 1. OBJECTIVES & CONSTRAINTS

**Scope of this module:**
- Owns the multi-sheet `LoroDoc`
- Incremental compilation of formulas to bytecode (compile once, execute N times)
- Evaluation via stack VM with dynamic cycle detection
- Two-level dependency graph: cell-to-cell + ranges
- Synchronous recomputation of dirty cells in `handle_ui_command` / `on_logic_event` (aligned with the `AppController` contract, Prose pattern)
- Broken reference detection (`#REF!`)
- Production of `DisplayViewport` for the UI thread
- Export of CRDT deltas to the Sync thread via `SyncCommand::SendUpdate`

**Dependencies:**

| Crate | Role | Rationale |
|---|---|---|
| `ambre-core` | `Uuid`, `DocType` | Shared foundation |
| `ambre-bus` | `SyncCommand`, `LogicEvent`, `AppController`, `CommandSink` | Inter-thread interface (IPC types + traits) |
| `ambre-draw` | `DrawSurface` trait, re-exports kurbo/peniko/parley | Shared rendering screen (WgpuSurface) / PDF (KrillaSurface) — required by architectural convention |
| `loro` | Multi-sheet CRDT | `LoroMovableList` (stable rows/columns), `LoroMap`, `LoroDoc` |
| `petgraph` | Cell-to-cell dependency DAG | Kosaraju SCC, topological sort |
| `nom` | Formula parser | Pure Rust, `no_std`, Excel-compatible syntax |
| `flume` | Channels for native tests only (`cfg(not(wasm32))`) | Simulates `postMessage` bridges natively behind `CommandSink` / `AppController` |
| `postcard` | Serialization of `TrameCommand` / `TrameEvent` / `DisplayViewport` | Compact, consistent with inter-thread IPC protocol |
| `rstar` | Floating objects R-tree | O(log n) viewport culling |

> **`ruzstd` absent** — compression delegated to the Sync thread.
> **Rayon absent (MVP)** — incremental recomputation O(dirty) is sufficient. Planned Phase 3+.

**Invariants:**
- Dirty recomputation is **synchronous** in each `handle_ui_command` / `on_logic_event` — the controller returns all events (including `ViewportData`) in the return `Vec<Vec<u8>>`.
- `LoroDoc` confined to the Logic thread: only exits via `export_updates()` → `Vec<u8>`.
- Computed values kept out of Loro: local `formula_cache`, each client recomputes independently.
- Bytecode compiled once: only cells whose `raw` changed are recompiled.
- No `unsafe`.

---

## 2. MULTI-SHEET CRDT MODEL

### A. Stable IDs via `LoroMovableList`

Positional addressing (`"R{row}C{col}"`) is **rejected**: inserting a row at position 5 would invalidate all keys `R5C*..RnC*` → O(n) CRDT operations.

**Decision: `LoroMovableList` for rows and columns.** Each element has an internal `ListItemID = (PeerID: u64, Counter: i32)`, stable and unique. Inserting a row = 1 CRDT operation. No cell key is invalidated.

**Key format in `LoroMap("cells_{sheet}")` — fixed-length 42 chars:**
```
"{row_peer:016x}_{row_ctr:08x}:{col_peer:016x}_{col_ctr:08x}"
```

### B. CRDT Schema

```
LoroDoc
├── "workbook" (LoroMap)              → {name, author, created}
├── "sheet_order" (LoroMovableList)   → [sheet_id, ...] reorderable
├── "sheets" (LoroMap)
│   └── "{sheet_id}" (LoroMap)        → {name: LoroText, hidden: bool, tab_color: u32}
│
├── "rows_{sheet_id}" (LoroMovableList)   → each element = a row (stable ListItemId)
├── "cols_{sheet_id}" (LoroMovableList)   → same for columns
│
├── "cells_{sheet_id}" (LoroMap)      → sparse, key = "{row_lid}:{col_lid}"
│   └── "{lid}:{lid}" (LoroMap)
│       ├── "v" : LoroText            → raw value ("42", "=SUM(A1:A10)", "text")
│       ├── "t" : String              → "n" | "s" | "b" | ""
│       └── "s" : String              → style_id
│
├── "col_widths_{sheet_id}" (LoroMap) → {col_lid: f64}
├── "row_heights_{sheet_id}" (LoroMap)
│
├── "merges_{sheet_id}" (LoroMap)     → {"{row_lid}:{col_lid}": "{row_lid_end}:{col_lid_end}"}
│
├── "styles" (LoroMap)                → shared styles
│   └── "style_007" (LoroMap)         → {bold, italic, fg, bg, fmt, halign, border}
│
├── "named_ranges" (LoroMap)          → {"TOTAL": {sheet_id, range}}
├── "media" (LoroMap)                 → {hash_blake2b: {dims, anchor, mime}}
│
│   ── Features (defined in spec-ambre-trame-features.md) ──
├── "filters_{sheet_id}" (LoroMap)    → active shared filter {col_lid, kind, values/condition}
├── "filter_hidden_{sheet_id}" (LoroMap) → {row_lid: bool} — rows hidden by filter
├── "cond_formats_{sheet_id}" (LoroList) → conditional formatting rules [{range, priority, rule, style}]
├── "validations_{sheet_id}" (LoroMap) → data validations [{kind, params, on_invalid, message}]
└── "protections_{sheet_id}" (LoroMap) → range protection (Post-MVP) [{owner, allowed, description}]
```

### C. SheetIndex — Bidirectional O(1) Cache

```rust
pub struct SheetIndex {
    row_by_pos: Vec<ListItemId>,
    col_by_pos: Vec<ListItemId>,
    row_to_pos: HashMap<ListItemId, usize>,
    col_to_pos: HashMap<ListItemId, usize>,
}

impl SheetIndex {
    /// Reconstruction O(N) au chargement
    pub fn build(rows: &LoroMovableList, cols: &LoroMovableList) -> Self { /* ... */ }

    /// Incremental O(N) update for shifts
    /// Phase 2: order-statistic tree for O(log N)
    pub fn apply_insert_row(&mut self, pos: usize, id: ListItemId) { /* ... */ }
    pub fn apply_delete_row(&mut self, pos: usize) -> ListItemId   { /* ... */ }

    pub fn resolve(&self, row: usize, col: usize) -> Option<(ListItemId, ListItemId)> {
        Some((*self.row_by_pos.get(row)?, *self.col_by_pos.get(col)?))
    }

    /// Used for #REF! detection
    pub fn contains_row(&self, id: &ListItemId) -> bool { self.row_to_pos.contains_key(id) }
    pub fn contains_col(&self, id: &ListItemId) -> bool { self.col_to_pos.contains_key(id) }

    pub fn col_letter(index: usize) -> String {
        let mut chars = Vec::new();
        let mut n = index;
        loop {
            chars.push((b'A' + (n % 26) as u8) as char);
            if n < 26 { break; }
            n = n / 26 - 1;
        }
        chars.into_iter().rev().collect()
    }

    pub fn letter_to_index(s: &str) -> usize {
        s.bytes().fold(0usize, |acc, b| acc * 26 + (b - b'A') as usize + 1) - 1
    }
}
```

---

## 3. TWO-LEVEL DEPENDENCY GRAPH

### Problem of Ranges in `petgraph`

`=SUM(A1:A100)` represented by 100 edges in `petgraph`:
- Memory explosion on entire columns
- O(n) updates on each row insert

**Solution: two separate structures.**

### A. Range Types

```rust
pub struct RangeDep {
    pub sheet:     SheetId,
    pub row_start: RangeEndpoint,
    pub row_end:   RangeEndpoint,
    pub col_start: RangeEndpoint,
    pub col_end:   RangeEndpoint,
}

pub enum RangeEndpoint {
    /// Bounded endpoint -- stable via ListItemId. #REF! if deleted.
    Specific(ListItemId),
    /// Unbounded range: entire column (A:A) or entire row (1:1)
    EntireAxis,
}

impl RangeDep {
    pub fn contains(&self, row: ListItemId, col: ListItemId, index: &SheetIndex) -> bool {
        self.axis_contains(&self.row_start, &self.row_end, row, &index.row_to_pos)
            && self.axis_contains(&self.col_start, &self.col_end, col, &index.col_to_pos)
    }

    fn axis_contains(
        &self, start: &RangeEndpoint, end: &RangeEndpoint,
        id: ListItemId, pos_map: &HashMap<ListItemId, usize>,
    ) -> bool {
        let pos = match pos_map.get(&id) { Some(p) => p, None => return false };
        let s = match start {
            RangeEndpoint::EntireAxis    => 0,
            RangeEndpoint::Specific(sid) => *pos_map.get(sid).unwrap_or(&usize::MAX),
        };
        let e = match end {
            RangeEndpoint::EntireAxis    => usize::MAX,
            RangeEndpoint::Specific(eid) => *pos_map.get(eid).unwrap_or(&0),
        };
        *pos >= s && *pos <= e
    }

    /// Resolves endpoints to positions for iteration.
    /// Returns None if a Specific endpoint is no longer in SheetIndex -> #REF!
    pub fn row_range(&self, index: &SheetIndex) -> Option<(usize, usize)> {
        self.resolve_range(&self.row_start, &self.row_end, &index.row_to_pos, index.row_by_pos.len())
    }
    pub fn col_range(&self, index: &SheetIndex) -> Option<(usize, usize)> {
        self.resolve_range(&self.col_start, &self.col_end, &index.col_to_pos, index.col_by_pos.len())
    }

    fn resolve_range(
        &self, start: &RangeEndpoint, end: &RangeEndpoint,
        pos_map: &HashMap<ListItemId, usize>, axis_len: usize,
    ) -> Option<(usize, usize)> {
        let s = match start {
            RangeEndpoint::EntireAxis    => 0,
            RangeEndpoint::Specific(id)  => *pos_map.get(id)?,
        };
        let e = match end {
            RangeEndpoint::EntireAxis    => axis_len.saturating_sub(1),
            RangeEndpoint::Specific(id)  => *pos_map.get(id)?,
        };
        Some((s, e))
    }
}
```

### B. DependencyGraph

```rust
pub struct DependencyGraph {
    /// Level 1: cell-to-cell dependencies (cycles detected statically)
    cell_dag:    petgraph::stable_graph::StableGraph<CellRef, ()>,
    cell_to_node: HashMap<CellRef, petgraph::graph::NodeIndex>,

    /// Level 2: range dependencies (cycles detected dynamically)
    range_deps:  HashMap<CellRef, Vec<RangeDep>>,

    /// Reverse index: sheet -> [(formula, RangeDep)]
    /// MVP: linear scan O(n). Phase 2: 2D interval tree.
    range_index: HashMap<SheetId, Vec<(CellRef, RangeDep)>>,
}

impl DependencyGraph {
    pub fn update(
        &mut self,
        formula_cell: CellRef,
        new_cell_deps:  Vec<CellRef>,
        new_range_deps: Vec<RangeDep>,
    ) -> Result<(), CycleError> {
        // Update cell_dag
        let node = self.get_or_create_node(formula_cell);
        let old: Vec<_> = self.cell_dag
            .edges_directed(node, petgraph::Direction::Outgoing)
            .map(|e| e.id()).collect();
        for e in old { self.cell_dag.remove_edge(e); }
        for dep in &new_cell_deps {
            self.cell_dag.add_edge(node, self.get_or_create_node(*dep), ());
        }

        // Static cycle detection on cell_dag
        if petgraph::algo::is_cyclic_directed(&self.cell_dag) {
            // Rollback
            let added: Vec<_> = self.cell_dag
                .edges_directed(node, petgraph::Direction::Outgoing)
                .map(|e| e.id()).collect();
            for e in added { self.cell_dag.remove_edge(e); }
            return Err(CycleError(self.find_cycle_cells()));
        }

        // Update range_deps + range_index
        for (_, entries) in self.range_index.iter_mut() {
            entries.retain(|(c, _)| *c != formula_cell);
        }
        self.range_deps.remove(&formula_cell);
        if !new_range_deps.is_empty() {
            for dep in &new_range_deps {
                self.range_index.entry(dep.sheet).or_default()
                    .push((formula_cell, dep.clone()));
            }
            self.range_deps.insert(formula_cell, new_range_deps);
        }
        Ok(())
    }

    /// Collects all cells to mark dirty after `changed` is modified.
    pub fn collect_dirty(
        &self,
        changed: CellRef,
        indices: &HashMap<SheetId, SheetIndex>,
    ) -> HashSet<CellRef> {
        let mut dirty = HashSet::new();

        // Level 1: DFS descendants in cell_dag
        if let Some(&node) = self.cell_to_node.get(&changed) {
            let mut dfs = petgraph::visit::Dfs::new(&self.cell_dag, node);
            while let Some(n) = dfs.next(&self.cell_dag) {
                dirty.insert(self.cell_dag[n]);
            }
        }

        // Level 2: formulas whose range contains changed
        let sheet_idx = match indices.get(&changed.sheet) { Some(i) => i, None => return dirty };
        if let Some(entries) = self.range_index.get(&changed.sheet) {
            for (formula_cell, dep) in entries {
                if dep.contains(changed.row, changed.col, sheet_idx) {
                    dirty.insert(*formula_cell);
                    // Propagate in cell_dag from formula_cell
                    if let Some(&node) = self.cell_to_node.get(formula_cell) {
                        let mut dfs = petgraph::visit::Dfs::new(&self.cell_dag, node);
                        while let Some(n) = dfs.next(&self.cell_dag) {
                            dirty.insert(self.cell_dag[n]);
                        }
                    }
                }
            }
        }
        dirty
    }

    fn find_cycle_cells(&self) -> Vec<CellRef> {
        petgraph::algo::kosaraju_scc(&self.cell_dag)
            .into_iter()
            .find(|scc| scc.len() > 1)
            .unwrap_or_default()
            .into_iter()
            .map(|n| self.cell_dag[n])
            .collect()
    }
}
```

---

## 4. BYTECODE VM & INCREMENTAL COMPILATION

### A. Instruction Set

```rust
#[derive(Clone)]
pub enum Op {
    // Pile
    PushNum(f64),
    PushStr(String),
    PushBool(bool),
    PushEmpty,

    // References -- verify SheetIndex (broken ref) and eval_stack (dynamic cycle)
    PushCell(CellRef),
    EvalRange(RangeDep),    // iterates range -> pushes Vec<CellValue> for functions

    // Binary operations
    Add, Sub, Mul, Div, Pow,
    Eq, Ne, Lt, Le, Gt, Ge,
    Concat,                 // &

    // Unary operations
    Neg, Not,

    // Appel de fonction : n_args valeurs sur la pile
    CallFn { name: String, n_args: u8 },

    // Short-circuit: if top = Error -> immediate return
    PropagateError,
}

pub struct CellBytecode {
    pub ops:        Vec<Op>,
    pub cell_deps:  Vec<CellRef>,   // extracted at compilation -> DependencyGraph update
    pub range_deps: Vec<RangeDep>,
}
```

### B. AST → Bytecode Compilation

```rust
fn compile(expr: &Expr) -> Vec<Op> {
    match expr {
        Expr::Literal(CellValue::Number(n))  => vec![Op::PushNum(*n)],
        Expr::Literal(CellValue::Text(s))    => vec![Op::PushStr(s.clone())],
        Expr::Literal(CellValue::Boolean(b)) => vec![Op::PushBool(*b)],
        Expr::Literal(_)                     => vec![Op::PushEmpty],
        Expr::CellRef(r)                     => vec![Op::PushCell(*r)],
        Expr::Range(dep)                     => vec![Op::EvalRange(dep.clone())],
        Expr::BinOp(kind, lhs, rhs) => {
            let mut ops = compile(lhs);
            ops.extend(compile(rhs));
            ops.push(kind.to_op());
            ops
        }
        Expr::FnCall(name, args) => {
            let mut ops = Vec::new();
            for arg in args { ops.extend(compile(arg)); }
            ops.push(Op::CallFn { name: name.clone(), n_args: args.len() as u8 });
            ops
        }
        Expr::SheetRef(_, inner) => compile(inner), // resolved at parse time
        Expr::NamedRange(_)      => vec![],          // resolved at parse time -> CellRef or Range
    }
}
```

**A1 → CellRef Resolution:** the `nom` parser receives the current `SheetIndex`. `A1` (col=A→0, row=1→index 0) is converted to a stable `CellRef { sheet, row: ListItemId, col: ListItemId }`, which survives row/column insertions.

### C. Bytecode Cache and Incremental Compilation

```rust
pub struct BytecodeCache {
    entries: HashMap<CellRef, CellBytecode>,
    stale:   HashSet<CellRef>,   // raw changed -> recompilation needed
}

impl BytecodeCache {
    pub fn invalidate(&mut self, cell: CellRef) { self.stale.insert(cell); }

    /// Recompiles if stale. Called just before compute_cell().
    pub fn recompile_if_stale(
        &mut self,
        cell:        CellRef,
        raw:         &str,
        sheet_index: &SheetIndex,
        sheet_name_to_id: &HashMap<String, SheetId>,
        deps:        &mut DependencyGraph,
    ) {
        if !self.stale.remove(&cell) { return; }

        if !raw.starts_with('=') {
            self.entries.remove(&cell);
            deps.update(cell, vec![], vec![]).ok();
            return;
        }

        match formula_parser::parse(&raw[1..], sheet_index, sheet_name_to_id) {
            Ok(ast) => {
                let bc = compile_with_deps(&ast);
                match deps.update(cell, bc.cell_deps.clone(), bc.range_deps.clone()) {
                    Ok(()) => { self.entries.insert(cell, bc); }
                    Err(CycleError(cycle)) => {
                        self.entries.remove(&cell);
                        // compute_cell traitera le CycleError
                    }
                }
            }
            Err(e) => {
                self.entries.remove(&cell);
                // formula_cache will receive ParseError directly in compute_cell
            }
        }
    }
}
```

### D. Stack VM with Dynamic Cycle Detection

```rust
/// Shared context for an entire recomputation cycle.
pub struct EvalContext<'a> {
    /// Cells currently being evaluated -- dynamic cycle detection
    /// (ranges, named ranges, cross-sheet, any cycle not visible in cell_dag)
    eval_stack: HashSet<CellRef>,

    /// Results already computed in this cycle -- avoids redundant recomputation
    resolved:   HashMap<CellRef, CellValue>,

    doc:     &'a LoroDoc,
    indices: &'a HashMap<SheetId, SheetIndex>,
    bc:      &'a BytecodeCache,
    cache:   &'a HashMap<CellRef, CellValue>,
}

impl<'a> EvalContext<'a> {
    pub fn eval_cell(&mut self, cell: CellRef) -> CellValue {
        // Dynamic cycle: cell being evaluated in the call stack
        if self.eval_stack.contains(&cell) {
            return CellValue::Error(FormulaError::CircularReference);
        }
        if let Some(v) = self.resolved.get(&cell) {
            return v.clone();
        }
        self.eval_stack.insert(cell);
        let result = match self.bc.entries.get(&cell) {
            None     => self.read_literal(cell),
            Some(bc) => self.execute(bc),
        };
        self.eval_stack.remove(&cell);
        self.resolved.insert(cell, result.clone());
        result
    }

    fn execute(&mut self, bc: &CellBytecode) -> CellValue {
        let mut stack: Vec<CellValue> = Vec::with_capacity(8);

        for op in &bc.ops {
            match op {
                Op::PushNum(n)  => stack.push(CellValue::Number(*n)),
                Op::PushStr(s)  => stack.push(CellValue::Text(s.clone())),
                Op::PushBool(b) => stack.push(CellValue::Boolean(*b)),
                Op::PushEmpty   => stack.push(CellValue::Empty),

                Op::PushCell(r) => {
                    let idx = &self.indices[&r.sheet];
                    if !idx.contains_row(&r.row) || !idx.contains_col(&r.col) {
                        stack.push(CellValue::Error(FormulaError::BrokenRef));
                    } else {
                        stack.push(self.eval_cell(*r)); // recursive -- detects cycles
                    }
                }

                Op::EvalRange(dep) => {
                    stack.push(CellValue::Range(self.eval_range(dep)));
                }

                Op::Add  => self.binop_num(&mut stack, |a, b| a + b),
                Op::Sub  => self.binop_num(&mut stack, |a, b| a - b),
                Op::Mul  => self.binop_num(&mut stack, |a, b| a * b),
                Op::Div  => {
                    let b = stack.pop().unwrap_or(CellValue::Empty);
                    let a = stack.pop().unwrap_or(CellValue::Empty);
                    stack.push(match (&a, &b) {
                        (CellValue::Number(_), CellValue::Number(d)) if *d == 0.0
                            => CellValue::Error(FormulaError::DivisionByZero),
                        (CellValue::Number(x), CellValue::Number(y))
                            => CellValue::Number(x / y),
                        _ => CellValue::Error(FormulaError::TypeError),
                    });
                }
                Op::Concat => {
                    let b = stack.pop().unwrap_or(CellValue::Empty).to_string_val();
                    let a = stack.pop().unwrap_or(CellValue::Empty).to_string_val();
                    stack.push(CellValue::Text(a + &b));
                }
                // Eq, Ne, Lt, Le, Gt, Ge, Neg, Not, Pow: similar implementations

                Op::CallFn { name, n_args } => {
                    let args: Vec<CellValue> = stack
                        .drain(stack.len() - *n_args as usize ..)
                        .collect();
                    stack.push(call_builtin(name, args));
                }

                Op::PropagateError => {
                    if matches!(stack.last(), Some(CellValue::Error(_))) {
                        return stack.pop().unwrap();
                    }
                }
            }
        }

        stack.pop().unwrap_or(CellValue::Empty)
    }

    fn eval_range(&mut self, dep: &RangeDep) -> Vec<CellValue> {
        let idx = &self.indices[&dep.sheet];

        let (row_start, row_end) = match dep.row_range(idx) {
            Some(r) => r,
            None    => return vec![CellValue::Error(FormulaError::BrokenRef)],
        };
        let (col_start, col_end) = match dep.col_range(idx) {
            Some(r) => r,
            None    => return vec![CellValue::Error(FormulaError::BrokenRef)],
        };

        let mut values = Vec::new();
        for rp in row_start..=row_end {
            for cp in col_start..=col_end {
                let row_id = idx.row_by_pos[rp];
                let col_id = idx.col_by_pos[cp];
                // Dynamic cycle detection on each cell in the range
                values.push(self.eval_cell(CellRef { sheet: dep.sheet, row: row_id, col: col_id }));
            }
        }
        values
    }
}
```

---

## 5. BROKEN REFERENCES (`#REF!`)

A reference is broken when its `ListItemId` is no longer in the `SheetIndex` — the row or column has been deleted.

**Detection at evaluation:**
- `Op::PushCell(r)` → `SheetIndex.contains_row(r.row) && contains_col(r.col)` → otherwise `BrokenRef`
- `Op::EvalRange(dep)` → `dep.row_range(idx)` returns `None` if `Specific(id)` endpoint is missing → `BrokenRef`

**Propagation after delete_row/delete_col:**

```rust
fn on_delete_row(&mut self, sheet: SheetId, deleted_id: ListItemId) {
    // All formulas depending on this ListItemId will be detected as broken
    // on the next evaluation via SheetIndex.contains_*
    // Mark them simply dirty to force recomputation
    let affected = self.deps.collect_dirty_for_deleted(sheet, deleted_id, &self.sheet_indices);
    self.dirty.extend(affected);
}
```

No bytecode patching — the `ListItemId` values in `CellRef` and `RangeDep` remain, absence in `SheetIndex` triggers `BrokenRef` at evaluation.

---

## 6. CELL MERGES

### CRDT Storage

```
"merges_{sheet_id}" (LoroMap) — LWW
  key   = "{row_lid}:{col_lid}"       (master cell = top-left corner)
  value = "{row_lid_end}:{col_lid_end}"  (bottom-right corner)
```

### Client-side rule: merge blocked if covered cell is non-empty

Before applying a merge locally (`TrameCommand::MergeCells`), the Logic thread checks that all covered cells (excluding the master) are empty in `LoroMap`. If a covered cell is non-empty or contains a formula, the merge is **rejected locally** with `TrameEvent::MergeRejected`. This is Excel and Google Sheets behavior. This rule only applies to local actions — it cannot prevent a concurrent conflict.

### Overlap Validation

LWW Loro resolves overlapping merge conflicts deterministically. Validation after each remote import: detection of cells covered by multiple merges → the merge with the most recent `ListItemID` (Loro order) wins, the other is removed.

### Concurrent Conflict Strategy: Data is Never Destroyed

**Scenario:** Alice sends `MergeCells(R3C2:R4C2)` when `R3C2` is empty on her side. Bob simultaneously sends `EditCell(R3C2, "=SUM(...)")`. Both deltas converge — result: `R3C2` is both a merge master AND non-empty in `LoroDoc`.

This conflict is unavoidable in collaborative CRDT. Rejecting Bob's delta would break convergence. **Strategy: the merge masks covered values without destroying them.**

```rust
/// Addition in TrameController
masked_cells: HashSet<String>, // lid_keys of covered non-empty cells

/// Returns detected conflict events -- added to the Vec<TrameEvent>
/// returned by dispatch() or on_logic_event().
fn detect_and_handle_merge_conflicts(&mut self, sheet: SheetId) -> Vec<TrameEvent> {
    let mut events = Vec::new();
    for (master_key, extent_key) in self.load_merges(sheet) {
        for covered_key in self.covered_cells(sheet, &master_key, &extent_key) {
            if self.cell_has_raw(sheet, &covered_key) {
                // Masked value: excluded from DAG, not displayed, not computed
                // tant que la fusion est active
                self.masked_cells.insert(covered_key.clone());
                events.push(TrameEvent::MergeConflictDetected {
                    master_cell:  self.parse_cell_key(&master_key, sheet),
                    covered_cell: self.parse_cell_key(&covered_key, sheet),
                });
            }
        }
    }
    events
}
```

Called after each `doc.import()` — returned events are added to the `Vec<TrameEvent>` of `on_logic_event()`.

**Guaranteed behavior:**
- Bob's value exists in `LoroDoc` — not erased, not corrupted.
- It is invisible as long as Alice's merge is active (`masked_cells`).
- If the merge is removed → removal from `masked_cells`, `bytecode.invalidate()`, dirty propagation → the value immediately reappears.
- All clients converge to the same state: masking is deterministic (based solely on `merges_{sheet}` and the presence of `raw` in `LoroMap`).
- No corrective mutation generated: no risk of a second round of conflicts.

### DAG and Masked Cells

Cells in `masked_cells` are excluded from the `DependencyGraph` and `formula_cache`. They do not participate in any computation while masked. When the mask is lifted (merge split), `bytecode.recompile_if_stale()` and `deps.update()` are called normally.

### DisplayCell

```rust
// Master cell
DisplayCell { is_merged: true, merge_span: Some((col_span, row_span)), .. }
// Covered cells (empty or masked) -- UI thread does not render them
DisplayCell { is_merged: true, merge_span: None, display_text: String::new(), .. }
```

---

## 7. INTERNAL STATE

```rust
pub struct TrameController {
    doc:           LoroDoc,
    active_sheet:  SheetId,
    sheet_indices: HashMap<SheetId, SheetIndex>,
    deps:          DependencyGraph,
    bytecode:      BytecodeCache,
    formula_cache: HashMap<CellRef, CellValue>,
    dirty:         HashSet<CellRef>,
    float_objects: rstar::RTree<FloatObject>,
    /// Cache name -> SheetId for resolving cross-sheet references
    /// in formulas (e.g. Sheet2!A1). Updated on each CreateSheet/RenameSheet/DeleteSheet.
    sheet_name_to_id: HashMap<String, SheetId>,
    viewport:      Viewport,
    tx_version:    u64,
    /// Bridge to the Sync thread -- `Option<Box<dyn CommandSink>>` injected by `ControllerRegistry`
    /// in `ambre-app` via `AppController::init()`. None before init.
    /// En WASM : wrapper `postMessage`. En tests natifs : wrapper `flume`.
    sync_tx:       Option<Box<dyn CommandSink>>,
    /// Cells covered by a merge but non-empty (masked, not destroyed)
    masked_cells:  HashSet<String>,
    /// Undo/Redo local -- scope = session document ouverte
    undo_manager:  loro::UndoManager,
    /// Canonical identifier for the current user (received via LogicEvent::SessionReady).
    /// Uuid stable per-user -- not pub_key which changes on key rotation.
    my_user_id:    Uuid,
    /// Document identifier (received on open).
    doc_id:        Uuid,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CellRef { pub sheet: SheetId, pub row: ListItemId, pub col: ListItemId }

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ListItemId { pub peer: u64, pub counter: i32 }

impl ListItemId {
    pub fn to_key_part(&self) -> String { format!("{:016x}_{:08x}", self.peer, self.counter) }
}

#[derive(Debug, Clone)]
pub enum CellValue {
    Number(f64),
    Text(String),
    Boolean(bool),
    Error(FormulaError),
    Empty,
    Range(Vec<CellValue>), // VM internal type only -- never in formula_cache
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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

---

## 8. INTERFACE CONTRACT (API)

### `TrameController` — impl `AppController`

Aligned with the Prose pattern: `TrameController` implements `AppController` directly. Dirty recomputation (P4) runs synchronously at the end of each `handle_ui_command` / `on_logic_event`. No custom loop, no `setTimeout(0)`.

```rust
// ambre-trame/src/controller.rs
use ambre_bus::{AppController, CommandSink, SyncCommand, LogicEvent};
use ambre_core::{DocType, Uuid};

impl AppController for TrameController {
    fn doc_type(&self) -> DocType { DocType::Trame }

    fn init(&mut self, sync_tx: Box<dyn CommandSink>) {
        self.sync_tx = Some(sync_tx);
    }

    /// Receives a serialized command (postcard) from the UI thread.
    /// Returns a list of serialized events -- ControllerRegistry dispatches them
    /// all via postMessage. A single TrameCommand can produce multiple events
    /// (e.g. EditAck + ViewportData after input).
    fn handle_ui_command(&mut self, cmd: Vec<u8>) -> Vec<Vec<u8>> {
        let command: TrameCommand = postcard::from_bytes(&cmd)
            .unwrap_or(TrameCommand::Noop);
        let events = self.dispatch(command);
        events.iter()
            .filter_map(|e| postcard::to_allocvec(e).ok())
            .collect()
    }

    /// Receives an event from the Sync thread (via ControllerRegistry).
    /// Returns a list of serialized events to dispatch to the UI thread --
    /// symmetrical with handle_ui_command.
    fn on_logic_event(&mut self, event: LogicEvent) -> Vec<Vec<u8>> {
        let trame_events: Vec<TrameEvent> = match event {
            // -- Real-time sync ------------------------------------------
            LogicEvent::RemoteUpdate { document_id, payload } => {
                if document_id != self.doc_id { return vec![]; }
                self.doc.import(&payload).ok();
                self.apply_remote_changes(&payload);
                self.compute_all_dirty();
                self.produce_viewport_events()
            }
            LogicEvent::RemoteSnapshot { document_id, payload } => {
                if document_id != self.doc_id { return vec![]; }
                self.doc.import(&payload).ok();
                self.rebuild_all();
                self.produce_viewport_events()
            }
            LogicEvent::RemoteEphemeral { document_id, payload } => {
                if document_id != self.doc_id { return vec![]; }
                // The pub_key -> UserId resolution is done by ambre-sync
                // BEFORE the payload propagates to the Logic thread.
                match postcard::from_bytes::<RemoteCursorPayload>(&payload) {
                    Ok(cursor) => vec![TrameEvent::RemoteCursors { cursors: vec![cursor] }],
                    Err(_) => vec![],
                }
            }
            LogicEvent::SnapshotNeeded { document_id } => {
                if document_id != self.doc_id { return vec![]; }
                let snapshot = self.doc.export_snapshot();
                if let Some(tx) = &self.sync_tx {
                    tx.send(SyncCommand::SendSnapshot { document_id, payload: snapshot });
                }
                vec![]
            }
            LogicEvent::SessionReady { user_id, .. } => {
                self.my_user_id = user_id;
                vec![]
            }
            LogicEvent::ConnectionStatus { document_id, connected } => {
                if document_id != self.doc_id { return vec![]; }
                vec![] // handled by ControllerRegistry (online/offline indicator)
            }
            LogicEvent::SyncWarning { document_id, code, message } => {
                if document_id != self.doc_id { return vec![]; }
                vec![TrameEvent::Error { code, message }]
            }
            // Events ignored by Trame (handled by ControllerRegistry)
            _ => vec![],
        };

        trame_events.iter()
            .filter_map(|e| postcard::to_allocvec(e).ok())
            .collect()
    }
}
```

### Command Dispatch and Synchronous Recalculation

```rust
impl TrameController {
    fn dispatch(&mut self, cmd: TrameCommand) -> Vec<TrameEvent> {
        match cmd {
            TrameCommand::Noop => vec![],

            TrameCommand::ScrollViewport { version, sheet_id, .. } => {
                // P1 -- immediate viewport response
                self.viewport = /* ... */;
                self.compute_all_dirty(); // recompute dirty in viewport
                vec![TrameEvent::ViewportData(self.build_viewport(version))]
            }

            TrameCommand::EditCell { version, sheet_id, row, col, raw } => {
                // P2 -- Loro mutation + dirty propagation + synchronous recomputation
                let cell_ref = self.resolve_cell(sheet_id, row, col);
                self.apply_edit_cell(cell_ref, &raw);
                self.bytecode.invalidate(cell_ref);
                let dirty = self.deps.collect_dirty(cell_ref, &self.sheet_indices);
                self.dirty.extend(dirty);
                self.compute_all_dirty();
                let delta = self.doc.export_updates();
                if let Some(tx) = &self.sync_tx {
                    tx.send(SyncCommand::SendUpdate {
                        document_id: self.doc_id, payload: delta,
                    });
                }
                let mut events = vec![TrameEvent::EditAck { version }];
                if self.viewport_intersects_dirty() {
                    events.push(TrameEvent::ViewportData(self.build_viewport(version)));
                }
                events
            }

            // ... other variants: same pattern (mutation -> dirty -> compute_all_dirty -> events)

            TrameCommand::Undo => { /* undo_manager.undo() + rebuild */ }
            TrameCommand::Redo => { /* undo_manager.redo() + rebuild */ }

            _ => self.dispatch_features_and_interop(cmd),
        }
    }

    /// Recomputes ALL dirty cells -- synchronous, no batching.
    /// MVP: full recomputation of 10k cells takes < 50 ms (P95 mobile target).
    /// This occasional lag is acceptable because it only occurs on large
    /// RemoteUpdate (rare case) -- identical to the Prose pattern where
    /// produce_layout_events() includes full pagination recomputation.
    fn compute_all_dirty(&mut self) {
        let mut topo = petgraph::visit::Topo::new(&self.deps.cell_dag);
        let mut ctx = EvalContext::new(&self.doc, &self.sheet_indices, &self.bytecode, &self.formula_cache);

        while let Some(node) = topo.next(&self.deps.cell_dag) {
            let cell = self.deps.cell_dag[node];
            if !self.dirty.remove(&cell) { continue; }

            let raw = self.get_raw(cell);
            self.bytecode.recompile_if_stale(
                cell, &raw, &self.sheet_indices[&cell.sheet],
                &self.sheet_name_to_id, &mut self.deps
            );
            let value = ctx.eval_cell(cell);
            self.formula_cache.insert(cell, value);
        }
    }
}
```

> **Invariant `doc.commit()` per `TrameCommand`:** each `TrameCommand` handler that mutates `LoroDoc` must end with `self.doc.commit()`. Loro groups all mutations since the last commit into a single `UndoManager` item. Without an explicit commit, multiple successive `TrameCommand` values could be grouped into a single undo item — which would break the expected granularity (1 command = 1 undo item). `EditCell` commits on cell exit (via `TrameCommand::CommitEdit` or a delay), not on each keystroke, so that an entire cell edit forms 1 undo item.

### Priorities and Budgets

| Priority | Task | Trigger | Behavior |
|:---|:---|:---|:---|
| **P1** | Scroll | `ScrollViewport` | `compute_all_dirty()` synchronous. Padding 20–100 rows depending on velocity. `DisplayViewport` immediate with `is_dirty=true` on pending cells if recomputation is not finished (theoretical case Phase 2). |
| **P2** | Input / Structure | `EditCell`, `InsertRow`, etc. | Loro mutation → dirty propagation → `compute_all_dirty()` synchronous → `EditAck` + `ViewportData`. |
| **P3** | Remote import | `LogicEvent::RemoteUpdate` | `doc.import()` → diff → dirty → `compute_all_dirty()` synchronous → `ViewportData`. |

**Anti-flickering:** `tx_version` incremented on each `DisplayViewport`. UI thread drops `version < last_processed`.

### Optional Phase 2 — Asynchronous P4 Yield

Full recomputation of 10k formula cells takes < 50 ms (target). During these 50 ms, the Logic thread is blocked — P1 (scroll) messages accumulate. This lag is **occasional** and **rare** (large `RemoteUpdate` during active scroll). It is accepted for the MVP, aligned with the Prose pattern.

If Phase 2 benchmarks show a real problem on mobile hardware, a yield mechanism can be added without modifying the `AppController` trait:

- `compute_all_dirty()` is replaced by `compute_dirty_batch(N)` which processes N cells and returns a boolean `more_dirty`.
- If `more_dirty`, the controller adds a `TrameEvent::ComputePending` to its returned events.
- The `ControllerRegistry` in `ambre-app`, when it sees this signal, injects a `TrameCommand::ContinueDirtyCompute` via `setTimeout(0)`.
- Pending P1/P2 `postMessage` messages in the JS queue are processed **before** the `setTimeout(0)`, guaranteeing the interruption.
- The `DisplayViewport` is sent with `is_dirty=true` on cells not yet recomputed.

This mechanism is **additive** — it does not modify the `AppController` trait or the existing `ControllerRegistry`. It adds two variants (`TrameCommand::ContinueDirtyCompute`, `TrameEvent::ComputePending`) specific to Trame.

---

## 9. DATA-ORIENTED STRUCTURES (UI Thread)

> **postcard serialization:** `TrameCommand` (UI→Logic) and `TrameEvent` (Logic→UI) are serialized via `postcard` in the `Vec<u8>` values exchanged through `AppController::handle_ui_command()` and `AppController::on_logic_event()` (see `spec-ambre-architecture.md` §2.D). The UI thread serializes a `TrameCommand` into a `UiEnvelope`, the Logic thread deserializes and dispatches. `TrameEvent` responses are serialized into the `Vec<Vec<u8>>` returned by `handle_ui_command` / `on_logic_event`.

```rust
#[derive(Serialize, Deserialize)]
pub struct DisplayViewport {
    pub version:       u64,
    pub sheet_id:      SheetId,
    pub row_start:     u32,
    pub col_start:     u32,
    pub cells:         Vec<DisplayCell>,
    pub col_widths:    Vec<f32>,
    pub row_heights:   Vec<f32>,
    pub float_objects: Vec<DisplayFloatObject>,
}

#[derive(Serialize, Deserialize)]
pub struct DisplayCell {
    pub row_index:    u32,
    pub col_index:    u32,
    pub display_text: String,
    pub raw:          String,           // barre de formule si cellule active
    pub is_dirty:     bool,             // UI displays as grayed out
    pub is_merged:    bool,
    pub merge_span:   Option<(u32, u32)>,
    pub style:        CellStyle,
}

/// Single source of truth -- UI thread -> Logic thread.
/// All feature and interop variants are consolidated here.
/// The `trame-features` and `trame-interop` specs document the behavior
/// of each variant but do not redefine the enum.
#[derive(Serialize, Deserialize)]
pub enum TrameCommand {
    Noop,

    // -- Grille & Navigation ---------------------------------------------
    ScrollViewport { version: u64, sheet_id: SheetId,
                     row_start: u32, row_end: u32, col_start: u32, col_end: u32,
                     velocity_px_ms: f32 },
    ActivateSheet { sheet_id: SheetId },

    // -- Saisie & Structure ----------------------------------------------
    EditCell    { version: u64, sheet_id: SheetId, row: u32, col: u32, raw: String },
    CommitEdit,
    ClearCell   { version: u64, sheet_id: SheetId, row: u32, col: u32 },
    InsertRow   { version: u64, sheet_id: SheetId, before_row: u32 },
    InsertCol   { version: u64, sheet_id: SheetId, before_col: u32 },
    DeleteRow   { version: u64, sheet_id: SheetId, row: u32 },
    DeleteCol   { version: u64, sheet_id: SheetId, col: u32 },
    ResizeCol   { version: u64, sheet_id: SheetId, col: u32, width_px: f32 },
    ResizeRow   { version: u64, sheet_id: SheetId, row: u32, height_px: f32 },

    // -- Fusions ---------------------------------------------------------
    MergeCells   { version: u64, sheet_id: SheetId,
                   row: u32, col: u32, row_span: u32, col_span: u32 },
    UnmergeCells { version: u64, sheet_id: SheetId, row: u32, col: u32 },

    // -- Sheets --------------------------------------------------------
    CreateSheet  { name: String },
    DeleteSheet  { sheet_id: SheetId },
    RenameSheet  { sheet_id: SheetId, name: String },

    // -- Filtres (trame-features sec.1) -------------------------------------
    ApplyFilter         { sheet_id: SheetId, col: u32, kind: FilterKind,
                          values: Option<Vec<String>>, condition: Option<FilterCondition> },
    ClearFilter         { sheet_id: SheetId },
    ActivateFilterView  { view_id: String },
    DeactivateFilterView,
    SaveFilterView      { name: String, filter_def: FilterDef },

    // -- Tri (trame-features sec.2) -----------------------------------------
    SortRange      { sheet_id: SheetId, row_start: u32, row_end: u32,
                     col_start: u32, col_end: u32, sort_col: u32,
                     ascending: bool, has_header: bool },
    SortRangeMulti { sheet_id: SheetId, row_start: u32, row_end: u32,
                     col_start: u32, col_end: u32,
                     sort_keys: Vec<SortKey>, has_header: bool },

    // -- Formatage conditionnel (trame-features sec.3) ----------------------
    AddCondFormat    { sheet_id: SheetId, rule: CondFormatRule },
    RemoveCondFormat { sheet_id: SheetId, rule_id: String },
    UpdateCondFormat { sheet_id: SheetId, rule_id: String, rule: CondFormatRule },

    // -- Validation (trame-features sec.5) ----------------------------------
    AddValidation    { sheet_id: SheetId, range: String, validation: ValidationRule },
    RemoveValidation { sheet_id: SheetId, range: String },

    // -- Protection (trame-features sec.6, Post-MVP) ------------------------
    SetProtection    { sheet_id: SheetId, range: String, owner_user_id: Uuid,
                       allowed_user_ids: Vec<Uuid>, description: String },
    RemoveProtection { sheet_id: SheetId, range: String },

    // -- Undo / Redo (trame-features sec.4) ---------------------------------
    Undo,
    Redo,

    // -- Collaborative cursor (ephemeral) ---------------------------------
    UpdateCursor { sheet_id: SheetId, row: u32, col: u32,
                   selection: Option<(u32, u32, u32, u32)> },

    // -- Import / Export (trame-interop) ----------------------------------
    ImportXlsx   { data: Vec<u8> },
    ImportCsv    { data: Vec<u8>, delimiter: Option<u8>, has_header: bool,
                   target_sheet_id: Option<SheetId>, start_row: u32, start_col: u32 },
    ExportXlsx   { sheet_ids: Option<Vec<SheetId>> },
    ExportCsv    { sheet_id: SheetId, delimiter: Option<u8>,
                   range: Option<CsvRange> },
    ExportPdf    { sheet_id: SheetId, page_setup: PdfPageSetup },

    // -- Snapshot (sync) -------------------------------------------------
    RequestSnapshot,
}

/// **Note:** `FilterKind`, `FilterCondition`, `FilterDef`, `CondFormatRule`,
/// `ValidationRule` types are defined in the corresponding `ambre-trame/src/features/` modules
/// (filters, conditional formatting, data validation). `CsvRange`, `PdfPageSetup`,
/// `ImportWarning` types are defined in `ambre-trame/src/interop/`. `SortKey` is defined in
/// `ambre-trame/src/features/sort.rs`. All these types implement `Serialize`/`Deserialize`
/// to travel in `TrameCommand` via postcard.

/// Single source of truth -- Logic thread -> UI thread.
/// All feature and interop variants are consolidated here.
#[derive(Serialize, Deserialize)]
pub enum TrameEvent {
    // -- Engine ----------------------------------------------------------
    ViewportData(DisplayViewport),
    EditAck      { version: u64 },
    FormulaError { row: u32, col: u32, error: FormulaError, version: u64 },
    SheetList    { sheets: Vec<(SheetId, String)> },
    MergeRejected { version: u64, reason: String },
    MergeConflictDetected { master_cell: CellRef, covered_cell: CellRef },

    // -- Curseurs distants -----------------------------------------------
    RemoteCursors { cursors: Vec<RemoteCursorPayload> },

    // -- Filtres (trame-features) ----------------------------------------
    FilterApplied    { version: u64, hidden_row_count: u32 },
    FilterCleared    { version: u64 },
    FilterViewSaved  { view_id: String },

    // -- Tri (trame-features) --------------------------------------------
    SortComplete     { version: u64 },

    // -- Formatage conditionnel (trame-features) -------------------------
    CondFormatsUpdated { version: u64 },

    // -- Undo / Redo (trame-features) ------------------------------------
    UndoApplied      { version: u64, can_undo: bool, can_redo: bool },
    RedoApplied      { version: u64, can_undo: bool, can_redo: bool },
    UndoStackEmpty,

    // -- Validation (trame-features) -------------------------------------
    ValidationError   { version: u64, row: u32, col: u32, message: String },
    ValidationWarning { version: u64, row: u32, col: u32, message: String },

    // -- Protection (trame-features, Post-MVP) ---------------------------
    ProtectionDenied  { version: u64, row: u32, col: u32, description: String },
    ProtectionViolationWarning { row: u32, col: u32, from_user_id: Uuid, description: String },

    // -- Import / Export (trame-interop) ----------------------------------
    ImportComplete   { version: u64, sheets_count: u32, warnings: Vec<ImportWarning> },
    ImportProgress   { percent: u8, current_sheet: String },
    ExportComplete   { version: u64, filename: String, data: Vec<u8> },

    // -- Erreur ----------------------------------------------------------
    Error { code: u16, message: String },
}

/// Remote cursor -- identified by UserId (stable), not pub_key.
/// The pub_key -> UserId resolution is done in ambre-sync via trusted_keys.
#[derive(Serialize, Deserialize)]
pub struct RemoteCursorPayload {
    pub user_id:      Uuid,
    pub display_name: String,
    pub color:        u32,
    pub sheet_id:     SheetId,
    pub row:          u32,
    pub col:          u32,
    pub selection:    Option<(u32, u32, u32, u32)>,
}
```

### Shared Rendering Module — `ambre-trame/src/render.rs`

Rendering functions are parameterized on the `DrawSurface` trait (defined in `ambre-draw`). The **same code** serves both screen rendering (`WgpuSurface` in the UI thread) and PDF export (`KrillaSurface` in the Logic thread, lazy-loaded). These functions live in `ambre-trame`, not in a separate UI crate.

```rust
// ambre-trame/src/render.rs
use ambre_draw::DrawSurface;

/// Rendu des cellules d'un viewport sur n'importe quelle surface.
pub fn draw_cells_on<S: DrawSurface>(
    surface: &mut S,
    vp: &DisplayViewport,
    layout: &GridLayout,
) {
    for cell in &vp.cells {
        if cell.is_merged && cell.merge_span.is_none() { continue; }
        let (x, y, w, h) = layout.cell_rect(cell.row_index, cell.col_index);
        // fill, text, borders -- identical screen and PDF
        surface.fill_path(/* ... */);
        surface.draw_glyphs(/* ... */);
    }
}

pub fn draw_grid_lines_on<S: DrawSurface>(surface: &mut S, layout: &GridLayout) { /* ... */ }
pub fn draw_col_headers_on<S: DrawSurface>(surface: &mut S, layout: &GridLayout) { /* ... */ }
pub fn draw_row_headers_on<S: DrawSurface>(surface: &mut S, layout: &GridLayout) { /* ... */ }
```

The UI thread instantiates `WgpuSurface` and calls these functions in its `requestAnimationFrame` loop. The Logic thread instantiates `KrillaSurface` for PDF export (triggered by `TrameCommand::ExportPdf`).

### Data Flow

```
UI Thread
  │  postMessage (postcard: UiEnvelope { cmd: Vec<u8> })
  ▼
Logic thread — TrameController::handle_ui_command()
  │  dispatch(TrameCommand) → Vec<TrameEvent>
  │  → doc.with_txn(|txn| ...)      — LoroDoc mutation
  │  → doc.export_updates()          — delta Vec<u8>
  │  → sync_tx.as_ref().map(|tx| tx.send(SyncCommand::SendUpdate { payload: delta }))
  │  → compute_all_dirty()           — synchronous recomputation
  │  → build_viewport()              — ViewportData
  │  → postMessage × N (one per event)
  ▼
Sync thread → encryption + signature → WsClientMsg::Update → Blind Relay

Sync thread → Logic thread (remote update)
  │  LogicEvent::RemoteUpdate { payload }
  ▼
Logic thread — TrameController::on_logic_event() → Vec<Vec<u8>>
  │  doc.import(&payload)
  │  → compute_all_dirty()           — synchronous recomputation
  │  → build_viewport()              — ViewportData
  ▼
UI Thread — re-render viewport
```

---

## 10. QUALITY & TESTABILITY

```rust
#[test] fn insert_row_stable_cell_keys()            { /* ListItemId not invalidated */ }
#[test] fn sheetindex_o1_lookup_after_inserts()      { }
#[test] fn static_cycle_cell_to_cell()               { /* A1=B1+1, B1=A1 -> CycleError */ }
#[test] fn dynamic_cycle_via_range()                 { /* A5=SUM(A1:A10) -> CircularRef */ }
#[test] fn dynamic_cycle_cross_sheet()               { /* Sheet1!A1=Sheet2!B1, Sheet2!B1=Sheet1!A1 */ }
#[test] fn broken_ref_cell_after_delete_row()        { /* =A1 -> delete row -> #REF! */ }
#[test] fn broken_ref_range_endpoint_after_delete()  { /* =SUM(A1:A10) -> delete row 10 -> #REF! */ }
#[test] fn range_includes_inserted_row()             { /* insert dans A1:A100 -> inclus dans SUM */ }
#[test] fn entire_axis_range()                       { /* SUM(A:A) -> inclut toute la colonne */ }
#[test] fn bytecode_only_stale_cells_recompiled()    { }
#[test] fn merge_overlap_resolved_by_lww()           { }
#[test] fn split_merge_restores_empty_cells()        { }
#[test] fn crdt_convergence_concurrent_edits()       { }
```

```rust
proptest! {
    #[test]
    fn dag_acyclic_after_valid_formulas(
        formulas in prop::collection::vec(any_valid_formula(), 1..100)
    ) { prop_assert!(!petgraph::algo::is_cyclic_directed(&ctrl.deps.cell_dag)); }

    #[test]
    fn sheetindex_incremental_eq_rebuild(
        ops in prop::collection::vec(any_row_col_op(), 1..200)
    ) { prop_assert_eq!(incremental_index, rebuilt_index); }

    #[test]
    fn crdt_convergence(
        edits_a in prop::collection::vec(any_cell_edit(), 1..50),
        edits_b in prop::collection::vec(any_cell_edit(), 1..50),
    ) { prop_assert_eq!(doc_a.get_deep_value(), doc_b.get_deep_value()); }
}
```

---

## 11. PERFORMANCE & RESOURCES

| Operation | Target | Note |
|---|---|---|
| P1 response (scroll → `DisplayViewport`) | < 15 ms | |
| P2 response (EditCell → ack) | < 5 ms | Recompile bytecode + update DAG |
| P3 import (`doc.import` 1 KB) | < 2 ms | |
| P4 batch (50 cells) | < 1 ms | |
| 10k-cell DAG recomputation | < 50 ms | |
| SheetIndex rebuild 10k rows | < 10 ms | At load time |
| `range_index` scan (MVP, 1k range formulas) | < 1 ms | Phase 2: interval tree |

**Memory:** `LoroDoc` < 10 MB / 100k cells ; DAG + cache < 50 MB / 100k formula cells.

---

## 12. ASSUMED SEMANTIC DIFFERENCES WITH EXCEL

Trame is not a semantic clone of Excel. These differences are **intentional**, documented, and motivated by the collaborative CRDT model. They must be clearly communicated to users coming from Excel.

### References After Structural Operations

**Excel:** inserting a row at position 3 implicitly rewrites `=SUM(A1:A10)` to `=SUM(A1:A11)`. This rewriting is performed by the Excel engine at the time of the operation.

**Trame:** references are stable `ListItemId` values. Inserting a row within the range `A1:A10` naturally includes the new row in the computation (it lands positionally between the endpoints), without any rewriting. The formula references the same physical rows + the new row inserted within the range.

**Visible difference:** if the user inserts a row **outside** the range (before A1 or after A10), Excel does not modify the formula. Trame does not either — identical behavior. The difference is only visible for insertions **within** the range, where Trame naturally includes the new row while Excel must rewrite.

**Why this is correct for a CRDT:** Excel's implicit rewriting is a non-idempotent operation. If Alice and Bob each simultaneously insert a row in the same range, two concurrent rewrites produce an inconsistent state. The `ListItemId` model avoids this problem.

### References After Sort

**Excel:** after a sort, absolute references (`$A$1`) do not move, relative references (`A1`) move with the data.

**Trame:** all references are stable `ListItemId` values — they follow the data during the sort. `=SUM(A1:A10)` after sort still references the same 10 physical rows, which are now in a different order. Behavior equivalent to Excel absolute references (`$A$1`).

**Visible difference:** if the user expected a formula referencing "row 3" to point to "what is now in third position after the sort", Trame does not do this. This is a choice consistent with the CRDT model.

### References After Delete + Reinsert

**Excel:** if a row is deleted then reinserted at the same position, Excel can find the cell "by position" in some cases.

**Trame:** reinsertion creates a new `ListItemId`. The old `ListItemId` no longer exists in the `SheetIndex`. Any formula that referenced the old ID produces `#REF!` — even if a new row is inserted at the same visual position. The user must retype or copy the formula to point it to the new `ListItemId`.

**Why this is correct:** deletion is an explicit intent. Finding "by position" after delete+reinsert is a fragile heuristic, especially in collaborative mode where the position may have changed between the two operations on different clients. `#REF!` is the correct information: the referenced cell no longer exists.

---

## 13. REJECTED ARCHITECTURAL DECISIONS

| Alternative | Rejected because |
|---|---|
| **Positional addressing `R{row}C{col}`** | Row insert = O(n) CRDT ops. `LoroMovableList` = 1 op. |
| **UUID v7 per row/column** | Redundant — Loro already generates stable `ListItemID` values. |
| **`{client_hex}{counter_hex}` format without fixed width** | Ambiguous without separator. Fixed encoding `{peer:016x}_{ctr:08x}` chosen. |
| **100 petgraph edges for a range** | Memory explosion + O(n) updates on insert. Separate `range_deps`. |
| **Interval tree 2D (MVP)** | Premature. Linear scan sufficient for < 1k formulas with ranges. Phase 2. |
| **Direct AST evaluation** | Re-parse on each recomputation. Bytecode = compile once, eval N times. |
| **Computed values in Loro** | Pointlessly replicated. Each client recomputes from `raw`. |
| **Loro for formula cycle detection** | Loro DAG manages CRDT causal order, orthogonal to formula dependencies. |
| **Rayon (MVP)** | 4th Web Worker context unjustified before 100k cells. Phase 3+. |

---

## 14. ROADMAP

> Tracked as Stories of Epic **E3** (and the engine core, already delivered under **E1**) in
> `specs/ambre-roadmap.md`.

**Phase 0 — Core standalone**
- [ ] Multi-sheet `LoroDoc`, full CRDT schema (§2.B including features: `filter_hidden`, `cond_formats`, `validations`, `protections`) *(single-sheet today; multi-sheet is Epic `E3`)*
- [ ] Bidirectional `SheetIndex` + `sheet_name_to_id` *(Epic `E3`)*
- [x] `nom` parser: arithmetic, `CellRef`, `Range` (bounded + `EntireAxis`), basic functions, cross-sheet references (`Sheet2!A1` via `sheet_name_to_id`)
- [x] Bytecode VM: full `Op` set, `compile()`, `execute()`, `EvalContext`
- [x] Two-level `DependencyGraph` + `collect_dirty`
- [x] Cycles: static Kosaraju + dynamic `eval_stack` (cell-to-cell, ranges, cross-sheet)
- [x] `#REF!`: detection via `SheetIndex.contains_*`
- [x] `TrameCommand` and `TrameEvent`: consolidated enums (§9, single source of truth) *(MVP subset; extended per feature)*
- [x] Unit tests + property tests + postcard round-trip `TrameCommand`/`TrameEvent`

**Phase 1 — Logic Thread Integration**
- [x] Logic-thread command dispatch (`handle_command` returning `TrameOutcome`, §8) *(current pattern; the `TrameController`/`AppController` trait refactor is deferred)*
- [x] Synchronous recomputation `compute_all_dirty()` in each handler
- [x] `DisplayViewport` + anti-flickering (`tx_version`)
- [ ] Insert/delete rows/columns + `SheetIndex` update + dirty propagation *(Epic `E3`)*
- [ ] Merges: Loro storage, validation, `DisplayCell.merge_span`, conflict detection *(Epic `E3`)*
- [ ] Shared rendering module `ambre-trame/src/render.rs`: `draw_cells_on<S: DrawSurface>()`, `draw_grid_lines_on<S>()`, etc. *(Epic `E3`)*
- [ ] `MockSyncHandle` for controller tests (via `CommandSink`)
- [ ] Undo/Redo via Loro `UndoManager` *(Epic `E3`)*

**Phase 2 — Core features (~50 functions)**
- [x] SUM, AVERAGE, MIN, MAX, COUNT, IF, AND, OR *(plus ABS, COUNTA, CONCATENATE, LEFT, RIGHT, MID, LEN, LOWER, UPPER, NOT, POWER, ROUND, SQRT, TRIM -- 22 implemented)*
- [ ] VLOOKUP, INDEX, MATCH, IFERROR, SUMIF, COUNTIF *(Epic `E3`: complete to ~50)*
- [ ] TEXT, CONCAT *(remaining string functions, Epic `E3`)*
- [ ] TODAY, NOW, DATE, YEAR, MONTH, DAY *(Epic `E3`)*
- [ ] Named ranges, cross-sheet references *(Epic `E3`)*
- [ ] `SheetRangeIndex` interval tree 2D if benchmarks justify it *(deferred -- Phase 3)*
- [ ] Floating objects R-tree *(deferred -- Phase 3)*
- [ ] `compute_all_dirty` benchmarks on mobile hardware — if > 50ms P95, implement the asynchronous P4 yield (`ContinueDirtyCompute` / `ComputePending`, see §8 optional Phase 2) *(deferred -- Epic `E5`)*

**Phase 3 — Optimization & Completion**
- [ ] Advanced functions: XLOOKUP, LAMBDA, dynamic arrays
- [ ] Rayon for independent DAG branches
- [ ] `SheetIndex` O(log N) for shifts
- [ ] PDF export via `KrillaSurface` in `ambre-trame/src/export_pdf.rs` (lazy-loaded)
- [ ] 1M-cell benchmarks
