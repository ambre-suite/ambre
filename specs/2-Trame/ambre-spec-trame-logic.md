# AMBRE — SPEC: TRAME LOGIC (Spreadsheet Engine)
**Type:** Vertical (Feature / Application — Logic)
**Status:** Draft

> Trame collaborative spreadsheet engine: multi-sheet CRDT state management (`LoroDoc`), incremental formula compilation to bytecode, evaluation via stack VM, two-level dependency graph, synchronous recomputation inside `handle_command`, and production of data-oriented structures for rendering.
>
> **Scope of this spec:** The engine (`TrameState`) that runs in the **Logic thread** (one of the three `wasm_thread` threads of the single WASM binary). Trame is a complete Logic + UI application — rendering (QuadRenderer/DrawSurface), user interactions, and the UI thread are covered in a separate spec (`spec-ambre-trame-ui.md`). Features (sort, filters, conditional formatting, undo/redo, validation) are covered in `spec-ambre-trame-features.md`. xlsx/PDF/CSV interoperability is covered in `spec-ambre-trame-interop.md`.
>
> **State registry.** Trame state is a concrete value, not a trait object. The Logic thread holds a typed registry `HashMap<DocumentId, DocState>` where `enum DocState { Trame(TrameState), … }`, and dispatch is a `match` on `(DocState, AppCommand)` — no trait object, no `Box<dyn Any>`, no `downcast`. `TrameState::handle_command` returns a typed [`TrameOutcome`]; the `ambre-logic` dispatcher reads it and forwards display events to the UI and CRDT/ephemeral payloads to Sync over bare `flume` channels. See `spec-ambre-architecture.md §6`.
>
> `LoroDoc` is instantiated in the Logic thread and never leaves it.

---

## 1. OBJECTIVES & CONSTRAINTS

**Scope of this module:**
- Owns the multi-sheet `LoroDoc`
- Incremental compilation of formulas to bytecode (compile once, execute N times)
- Evaluation via stack VM with dynamic cycle detection
- Two-level dependency graph: cell-to-cell + ranges
- Synchronous recomputation of dirty cells inside `handle_command` / `apply_remote` (aligned with the Prose pattern)
- Broken reference detection (`#REF!`)
- Production of `DisplayViewport` for the UI thread
- Export of CRDT deltas to the Sync thread via `SyncCommand::SendUpdate`

**Dependencies:**

| Crate | Role | Rationale |
|---|---|---|
| `ambre-core` | `Uuid`, `DocType` | Shared foundation |
| `ambre-bus` | `AppCommand`/`AppEvent` (incl. `TrameCommand`/`TrameEvent`), `SyncCommand`, `LogicEvent`, `CommandSink` (test seam) | Typed inter-thread data plane + IPC types |
| `ambre-draw` | `DrawSurface` trait, re-exports kurbo/peniko/parley | Shared rendering screen (WgpuSurface) / PDF (KrillaSurface) — required by architectural convention |
| `loro` | Multi-sheet CRDT | `LoroMovableList` (stable rows/columns), `LoroMap`, `LoroDoc` |
| `petgraph` | Cell-to-cell dependency DAG | Kosaraju SCC, topological sort |
| `nom` | Formula parser | Pure Rust, `no_std`, Excel-compatible syntax |
| `flume` | Logic<->Sync channels (prod) + native test bridges | The Logic dispatcher forwards `SyncCommand`s on a bare `flume::Sender`; tests use the `CommandSink` seam (`MockSyncHandle`) |
| `postcard` | Serialization of `TrameCommand` / `TrameEvent` / `DisplayViewport` | Compact, consistent with inter-thread IPC protocol |
| `rstar` | Floating objects R-tree | O(log n) viewport culling |

> **`ruzstd` absent** — compression delegated to the Sync thread.
> **Rayon absent (MVP)** — incremental recomputation O(dirty) is sufficient. Planned Phase 3+.

**Invariants:**
- Dirty recomputation is **synchronous** in each `handle_command` / `apply_remote` — the state returns all events (including `ViewportData`) in the `TrameOutcome.display` / returned `Vec<TrameEvent>`.
- `LoroDoc` confined to the Logic thread: only exits via `export_updates()` → `Vec<u8>`.
- Computed values kept out of Loro: local `formula_cache`, each client recomputes independently.
- Bytecode compiled once: only cells whose `raw` changed are recompiled.
- No `unsafe`.

---

## 2. MULTI-SHEET CRDT MODEL

### A. Own stable uids carried as `LoroMovableList` values

Positional addressing (`"R{row}C{col}"`) is **rejected**: inserting a row at position 5 would invalidate all keys `R5C*..RnC*` → O(n) CRDT operations.

**Decision: each row and column carries an own stable `Uid` that *we* mint, stored as the
*value* of an element in a `LoroMovableList` (`rows_{sheet}` / `cols_{sheet}`).** A `Uid` is
`{peer: u64, counter: u32}`, rendered `"{peer:016x}_{counter:08x}"`, where `peer` is the document
`PeerID` and `counter` a per-session monotonic counter. Inserting a row = 1 CRDT operation
(push/insert one uid value). No cell key is invalidated.

> **Why an owned value, not the Loro element id.** `LoroMovableList` does expose a per-element op
> id via `get_cursor(pos, Side::Middle).id` (a `loro::ID { peer, counter }`), but
> `LoroMovableList::mov(from, to)` **reassigns the moved element's op id** -- it becomes the move
> op's id. A cell keyed by that slot id would be orphaned by every reorder. Owning the uid as the
> element's *value* makes it **move-invariant**: a reorder (`mov`) shuffles the values, the uid each
> carries is untouched, and every `"{row_uid}:{col_uid}"` cell key keeps resolving. This is what
> lets a sort be a **pure order permutation** (§12, `trame-features §2`) instead of a content
> rewrite, and what lets id-based references follow the data. See the rejected "Loro slot id as
> cell key" and "sort by content rewrite" entries in §13.

**Convergence of minting.** Two peers that concurrently insert a row each mint a **distinct** uid
(their `PeerID` differs), so both rows survive and `LoroMovableList` orders the two values
canonically -- no shared counter, no LWW collision. This is the §2.D sheet-id argument applied to
rows/columns (see the rejected shared-counter hazard in §13). The counter need only be unique
within a peer's session; once minted, a uid lives in the CRDT and never changes.

**Key format in `LoroMap("cells_{sheet}")` — fixed-length 51 chars** (16 + `_` + 8 per side, joined by `:`):
```
"{row_peer:016x}_{row_ctr:08x}:{col_peer:016x}_{col_ctr:08x}"
```

### A bis. Dense over used range — reconciliation with the virtual 100k grid

The grid is virtual at `TOTAL_ROWS = 100_000`. We do **not** materialize 100k list elements. The
model that keeps O(1) insert *and* CRDT convergence is:

> **Own stable `Uid`s carried as element values, `rows_/cols_` `LoroMovableList` dense over the
> *used range* `[0..hwm]` (high-water mark), rows/columns beyond the used range are virtual** (no
> element, no storage; a read there returns the generated label, a write there materializes up to
> that index).

Crucially, **list index == grid position**: resolving `A5` is a direct `row_by_pos[5]` lookup, and a
concurrent insert of two distinct rows is two independent element insertions that Loro orders
canonically — convergent *and* intent-preserving, with no shared mutable counter.

**Materialization cap.** To bound the pathological "single value far down" case (e.g. an edit at row
99 999 in an otherwise empty sheet, which would materialize ~100k placeholders), a write that would
extend the used range by more than `ROW_MATERIALIZE_CAP` (= 10 000) rows past the current high-water
mark is **rejected** (the cell write is dropped and a warning is surfaced). Normal top-down editing
is unaffected. The symmetric cap applies to columns (bounded by `TOTAL_COLS = 1000` anyway).

The **truly-sparse run-length** alternative (store only non-empty rows + a per-row `gap_before`
counter + an order-statistic tree) is **rejected** — see §13: splitting a shared gap on concurrent
inserts is not convergent under LWW and corrupts absolute numbering. The order-statistic tree only
speeds the position query; it does not remove the shared-counter hazard.

### B. CRDT Schema

```
LoroDoc
├── "workbook" (LoroMap)              → {name, author, created}
├── "sheet_order" (LoroMovableList)   → [sheet_id, ...] reorderable
├── "sheets" (LoroMap)
│   └── "{sheet_id}" (LoroMap)        → {name: LoroText, hidden: bool, tab_color: u32}
│
├── "rows_{sheet_id}" (LoroMovableList)   → each element VALUE = a row's own uid (move-invariant)
├── "cols_{sheet_id}" (LoroMovableList)   → same for columns
│
├── "cells_{sheet_id}" (LoroMap)      → sparse, key = "{row_uid}:{col_uid}"
│   └── "{uid}:{uid}" (LoroMap)
│       ├── "v" : LoroText            → raw value ("42", "=SUM(A1:A10)", "text")
│       ├── "t" : String              → "n" | "s" | "b" | ""
│       └── "s" : String              → style_id
│
├── "col_widths_{sheet_id}" (LoroMap) → {col_uid: f64}
├── "row_heights_{sheet_id}" (LoroMap)
│
├── "merges_{sheet_id}" (LoroMap)     → {"{row_uid}:{col_uid}": "{row_uid_end}:{col_uid_end}"}
│
├── "styles" (LoroMap)                → shared styles
│   └── "style_007" (LoroMap)         → {bold, italic, underline, font, size, fg, bg, fmt, halign, valign, wrap, border}
│
├── "named_ranges" (LoroMap)          → {"TOTAL": {sheet_id, range}}
├── "media" (LoroMap)                 → {hash_blake2b: {dims, anchor, mime}}
│
│   ── Features (defined in spec-ambre-trame-features.md) ──
├── "filters_{sheet_id}" (LoroMap)    → active shared filter {col_uid, kind, values/condition}
├── "filter_hidden_{sheet_id}" (LoroMap) → {row_uid: bool} — rows hidden by filter
├── "cond_formats_{sheet_id}" (LoroList) → conditional formatting rules [{range, priority, rule, style}]
├── "validations_{sheet_id}" (LoroMap) → data validations [{kind, params, on_invalid, message}]
└── "protections_{sheet_id}" (LoroMap) → range protection (Post-MVP) [{owner, allowed, description}]
```

### C. SheetIndex — Bidirectional O(1) Cache

The `Vec`s are dense over the **used range** (`len == hwm`, see §2.A bis), never over the 100k
virtual grid, so `resolve` is a direct O(1) index lookup and memory is proportional to real sheet
size. `row_by_pos[i]` is the `Uid` carried by the element at list position `i` (read from the
element's *value*, parsed with [`parse_uid`], not from a Loro cursor -- see §2.A). Because the uid
is move-invariant, a `rows_{sheet}.mov(from, to)` simply **permutes `row_by_pos`** and every cell
key still resolves; this is the single primitive behind sort (§12) and any future drag-to-reorder.

```rust
pub struct SheetIndex {
    row_by_pos: Vec<Uid>,
    col_by_pos: Vec<Uid>,
    row_to_pos: HashMap<Uid, usize>,
    col_to_pos: HashMap<Uid, usize>,
}

impl SheetIndex {
    /// O(N_used) reconstruction at load (N_used = materialized rows + cols, not 100k).
    /// Reads each element's uid VALUE in list order.
    pub fn build(rows: &LoroMovableList, cols: &LoroMovableList) -> Self { /* ... */ }

    /// Incremental O(N) update for shifts
    /// Phase 2: order-statistic tree for O(log N)
    pub fn apply_insert_row(&mut self, pos: usize, id: Uid) { /* ... */ }
    pub fn apply_delete_row(&mut self, pos: usize) -> Uid   { /* ... */ }

    /// Permute the row order to mirror a `mov(from, to)` (sort, reorder).
    pub fn apply_move_row(&mut self, from: usize, to: usize) { /* ... */ }

    pub fn resolve(&self, row: usize, col: usize) -> Option<(Uid, Uid)> {
        Some((*self.row_by_pos.get(row)?, *self.col_by_pos.get(col)?))
    }

    /// Used for #REF! detection
    pub fn contains_row(&self, id: &Uid) -> bool { self.row_to_pos.contains_key(id) }
    pub fn contains_col(&self, id: &Uid) -> bool { self.col_to_pos.contains_key(id) }

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

### D. Sheet Lifecycle (create / rename / reorder / delete)

A workbook is a set of sheets ordered by `sheet_order` (a `LoroMovableList` of
stable `sheet_id`s). Each operation below is a small, convergent CRDT mutation;
none rewrites another sheet's content. The four commands are
[`CreateSheet`](#8-interface-contract-api) / `RenameSheet` / `MoveSheet` /
`DeleteSheet` (§9).

> **Sheet identity is a stable `sheet_id`, never the display name.** Formulas
> depend on `sheet_id`; the display name is a mutable label resolved to its id
> at parse time. This is the §2.A principle applied to sheets: rename and
> reorder never invalidate a reference, exactly as a row insert never
> invalidates a cell key. Only **delete** removes an id, and that is the only
> operation that can produce a cross-sheet `#REF!`.

**`sheet_id` generation.** The implicit first sheet keeps the constant
[`DEFAULT_SHEET`](../../crates/ambre-trame/src/logic.rs) `"s0"` so two fresh
states (creator and reloading peer) address the same containers and merge
cleanly. Every **explicit** sheet gets a peer-scoped id
`"{peer:016x}_{counter:08x}"` (the document `PeerID` plus a per-session
counter), so two peers that concurrently "add a sheet" mint two **distinct**
ids -- both survive, neither overwrites the other (LWW on a single shared id is
thereby avoided, mirroring the rejected shared-counter hazard of §13).

#### Create

```
sheets.insert(sheet_id, {name, hidden: false, tab_color})   // metadata
sheet_order.push(sheet_id)                                  // 1 LoroMovableList op
// cells_{id} / rows_{id} / cols_{id} are created lazily on first write.
```

Convergence: concurrent creates append two distinct ids; `LoroMovableList`
orders them canonically. The local engine assigns the new sheet a `u16` index at
the next reseed (its empty cell set needs no recompute). Reply:
`TrameEvent::SheetList` so the tab strip refreshes.

#### Rename

```
sheets.get(sheet_id).set("name", new_name)   // LWW on the name field only
```

The `sheet_id` is untouched, so **every cross-sheet reference keeps resolving and
keeps computing** -- no formula is recompiled for a rename. The formula *text*
shown in the formula bar is reconstructed from `sheet_id -> current name`, so a
renamed sheet immediately displays the new qualifier (`Data!A1` where the user
once typed `Sheet2!A1`). Concurrent renames resolve by LWW on the `name` field;
all peers converge to one name and reconstruct identical formula text.

> **Implementation note.** This requires the compiled cross-sheet dependency to
> carry the `sheet_id` (the engine's local `u16` is a per-session interning of
> it), and the stored raw `v` text to be re-rendered through the current
> `sheet_id -> name` map rather than replayed verbatim. The MVP engine re-parses
> raw text by display name on reseed, so until id-persistence lands a rename
> breaks an existing cross-sheet reference -- tracked in §14.

#### Reorder (move)

```
sheet_order.mov(from_index, to_index)   // 1 LoroMovableList move op
```

Pure presentation: `sheet_order` drives only the tab strip and the local `u16`
index assignment, never resolution (references are by `sheet_id`). No recompute.
`MoveSheet { sheet_id, to_index }` looks up the current index and issues one
`mov`; concurrent moves converge through `LoroMovableList`'s canonical ordering.

#### Delete

```
sheet_order.delete(index, 1)
sheets.delete(sheet_id)
// drop per-sheet containers: cells_{id}, rows_{id}, cols_{id},
// format_{id}, merges_{id}, and the feature maps of trame-features.
```

- **Cross-sheet `#REF!`.** A formula `=Sheet2!A1` whose `sheet_id` is gone
  resolves to `BrokenRef` at evaluation -- the §5 mechanism, extended to the
  sheet axis: absence in the `sheet_id -> u16` map is the trigger, no bytecode
  patching. Deleting a sheet marks every dependent formula dirty (full reseed,
  since the `u16` remap shifts indices) and recomputes them to `#REF!`.
- **A workbook always keeps >= 1 sheet.** Deleting the last sheet is **rejected
  locally** (`TrameEvent::Error`). Under concurrency, if two peers delete
  different sheets and the deletes converge to an empty `sheet_order`, the sheet
  whose `sheet_id` sorts first is **re-inserted** (deterministic on all peers) so
  the workbook is never sheet-less.
- **Active sheet deleted** (locally or by a remote delete): the active sheet
  switches to the previous tab, or the first remaining sheet if it was first.

**Engine impact.** Create/move/rename need at most a `SheetList` refresh; only
**delete** forces a reseed + recompute (sheet set changed -> `u16` remap +
`#REF!` propagation), aligned with the structural-op reseed path of §8.

---

## 3. TWO-LEVEL DEPENDENCY GRAPH

### Problem of Ranges in `petgraph`

`=SUM(A1:A100)` represented by 100 edges in `petgraph`:
- Memory explosion on entire columns
- O(n) updates on each row insert

**Solution: two separate structures.**

### A. Range Types

**Two reference representations.** The `nom` parser produces *positional* refs
(`CellRef { sheet: u16, row: u32, col: u32 }`, `RangeDep { .. : u32 }`) -- a typed
`A1` / `A1:B10`. Compilation resolves those against the current per-sheet
`SheetIndex` into the *uid-anchored* engine forms below (`CellId` / `RangeId`),
which are what the bytecode, dependency graph, bytecode cache and VM are keyed
by. Binding `A1` to the row/column `Uid` is what makes a reference follow the data
(§4.B / §12).

**Bound vs virtual axes (`AxisId`).** Within the used range a single-cell
reference binds to the row/column `Uid`. Beyond the used range -- a *virtual*
row/column with no list element yet (§2.A bis), so no `Uid` -- there is nothing to
bind to, so the reference keeps the grid position it was typed at and reads
`Empty` until that line is materialized (a later re-edit then re-binds it to the
new `Uid`). This is the only correct way to model `=A100` on a 10-row sheet.

**Cells follow, ranges are blocks.** Only *single-cell* references bind to a `Uid`
and follow their row/column through a sort (§12). A **range** is compiled with
`Virtual` (positional) endpoints, so it denotes a rectangular *region* -- a block
of grid positions -- not a set of specific rows. Sorting the rows inside
`SUM(A1:A3)` reorders the same three values, leaving the block (and its total)
intact; this matches Excel/Sheets and is the only well-defined behavior, since
uid-bound endpoints could scatter and invert the rectangle (see §13). A range
therefore never `#REF!`s on a row delete -- it shifts positionally, like Excel.

```rust
pub enum AxisId {
    /// Bound to a materialized row/column `Uid`. #REF! once that id is deleted.
    Bound(Uid),
    /// A virtual position beyond the used range; stays positional, reads Empty.
    Virtual(u32),
}

pub struct CellId {           // engine-side (uid-anchored) cell identity
    pub sheet: u16,           // local engine sheet index
    pub row:   AxisId,
    pub col:   AxisId,
}

pub struct RangeId {          // engine-side (uid-anchored) range identity
    pub sheet:     u16,
    pub row_start: RangeEndpoint,
    pub row_end:   RangeEndpoint,
    pub col_start: RangeEndpoint,
    pub col_end:   RangeEndpoint,
}

pub enum RangeEndpoint {
    /// Bounded endpoint -- stable via the axis `AxisId`. #REF! if a bound id is deleted.
    Specific(AxisId),
    /// Unbounded range: entire column (A:A) or entire row (1:1). Reserved; the
    /// MVP parser never emits it.
    EntireAxis,
}

impl RangeId {
    pub fn contains(&self, row: Uid, col: Uid, index: &SheetIndex) -> bool {
        self.axis_contains(&self.row_start, &self.row_end, row, &index.row_to_pos)
            && self.axis_contains(&self.col_start, &self.col_end, col, &index.col_to_pos)
    }

    /// Resolves endpoints to positions for iteration. `entire` is the position an
    /// `EntireAxis` bound takes (0 for a start, axis_len-1 for an end). Returns
    /// None only when a `Specific(Bound(_))` endpoint is no longer in the index
    /// (-> #REF!); a `Specific(Virtual(p))` keeps position `p` even past the
    /// used range (the caller clamps iteration to the materialized cells).
    pub fn row_range(&self, index: &SheetIndex) -> Option<(usize, usize)> { /* axis resolve */ }
    pub fn col_range(&self, index: &SheetIndex) -> Option<(usize, usize)> { /* axis resolve */ }

    fn endpoint_pos(ep: &RangeEndpoint, entire: usize, pos: impl Fn(&Uid) -> Option<usize>)
        -> Option<usize>
    {
        match ep {
            RangeEndpoint::EntireAxis                   => Some(entire),
            RangeEndpoint::Specific(AxisId::Bound(u))   => pos(u),         // None -> #REF!
            RangeEndpoint::Specific(AxisId::Virtual(p)) => Some(*p as usize),
        }
    }
}
```

### B. DependencyGraph

```rust
pub struct DependencyGraph {
    /// Level 1: cell-to-cell dependencies (cycles detected statically)
    cell_dag:    petgraph::stable_graph::StableGraph<CellId, ()>,
    cell_to_node: HashMap<CellId, petgraph::graph::NodeIndex>,

    /// Level 2: range dependencies (cycles detected dynamically)
    range_deps:  HashMap<CellId, Vec<RangeId>>,

    /// Reverse index: local sheet index -> [(formula, RangeId)]
    /// MVP: linear scan O(n). Phase 2: 2D interval tree.
    range_index: HashMap<u16, Vec<(CellId, RangeId)>>,
}

impl DependencyGraph {
    pub fn update(
        &mut self,
        formula_cell: CellId,
        new_cell_deps:  Vec<CellId>,
        new_range_deps: Vec<RangeId>,
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
        changed: CellId,
        index: &SheetIndex,   // the changed cell's sheet index
    ) -> HashSet<CellId> {
        let mut dirty = HashSet::new();

        // Level 1: DFS descendants in cell_dag
        if let Some(&node) = self.cell_to_node.get(&changed) {
            let mut dfs = petgraph::visit::Dfs::new(&self.cell_dag, node);
            while let Some(n) = dfs.next(&self.cell_dag) {
                dirty.insert(self.cell_dag[n]);
            }
        }

        // Level 2: formulas whose range contains changed. Only a fully bound
        // cell can sit inside a uid-keyed range; a virtual cell is skipped.
        let (Some(row), Some(col)) = (changed.row_uid(), changed.col_uid()) else { return dirty };
        if let Some(entries) = self.range_index.get(&changed.sheet) {
            for (formula_cell, dep) in entries {
                if dep.contains(row, col, index) {
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

    fn find_cycle_cells(&self) -> Vec<CellId> {
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
    PushCell(CellId),
    EvalRange(RangeId),     // iterates range -> pushes Vec<CellValue> for functions

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
    pub cell_deps:  Vec<CellId>,    // extracted at compilation -> DependencyGraph update
    pub range_deps: Vec<RangeId>,
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
        Expr::CellRef(r)                     => vec![Op::PushCell(bind_cell(r, index))],
        Expr::Range(dep)                     => vec![Op::EvalRange(bind_range(dep, index))],
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

**A1 → CellId Resolution:** compilation receives the current `SheetIndex` (per sheet). `A1` (col=A→0, row=1→index 0) is resolved against the index to the `Uid`s currently at those positions, producing a stable `CellId { sheet, row: AxisId::Bound(uid), col: AxisId::Bound(uid) }`. The reference then **binds to those rows/columns**, surviving inserts, deletes, and reorders -- after a sort it still points at the same physical cells (now elsewhere), which is the §12 "references follow the data" guarantee. A position *beyond* the used range (e.g. `A100` on a 10-row sheet) has no `Uid`, so it compiles to `AxisId::Virtual(pos)` and reads `Empty` until that line is materialized (§3.A).

### C. Bytecode Cache and Incremental Compilation

```rust
pub struct BytecodeCache {
    entries: HashMap<CellId, CellBytecode>,
    stale:   HashSet<CellId>,   // raw changed -> recompilation needed
}

impl BytecodeCache {
    pub fn invalidate(&mut self, cell: CellId) { self.stale.insert(cell); }

    /// Recompiles if stale. Called just before compute_cell(). `indices` lets
    /// compilation bind every positional `A1` to its row/column `AxisId`.
    pub fn recompile_if_stale(
        &mut self,
        cell:        CellId,
        raw:         &str,
        indices:     &HashMap<u16, SheetIndex>,
        deps:        &mut DependencyGraph,
        ctx:         &dyn FormulaCtx,
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
    eval_stack: HashSet<CellId>,

    /// Results already computed in this cycle -- avoids redundant recomputation
    resolved:   HashMap<CellId, CellValue>,

    indices: &'a HashMap<u16, SheetIndex>,
    bc:      &'a BytecodeCache,
    cache:   &'a HashMap<CellId, CellValue>,
}

impl<'a> EvalContext<'a> {
    pub fn eval_cell(&mut self, cell: CellId) -> CellValue {
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
                    // Resolve each axis to a position: a deleted Bound id -> #REF!;
                    // a Virtual position past the used range -> Empty; otherwise
                    // normalize to the canonical bound cell at that position and
                    // recurse (detecting cycles).
                    stack.push(self.read_cell(*r));
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

    fn eval_range(&mut self, dep: &RangeId) -> Vec<CellValue> {
        let idx = &self.indices[&dep.sheet];

        let (row_start, row_end) = match dep.row_range(idx) {
            Some(r) => r,
            None    => return vec![CellValue::Error(FormulaError::BrokenRef)],
        };
        let (col_start, col_end) = match dep.col_range(idx) {
            Some(r) => r,
            None    => return vec![CellValue::Error(FormulaError::BrokenRef)],
        };

        // Iterate the logical span (preserving shape for VLOOKUP/INDEX); a
        // position past the materialized range reads Empty, a materialized one
        // normalizes to its (row_uid, col_uid) and recurses (cycle detection).
        let mut values = Vec::new();
        for rp in row_start..=row_end {
            for cp in col_start..=col_end {
                values.push(match (idx.row_id(rp), idx.col_id(cp)) {
                    (Some(row), Some(col)) => self.eval_cell(CellId::bound(dep.sheet, row, col)),
                    _                      => CellValue::Empty,
                });
            }
        }
        values
    }
}
```

---

## 5. BROKEN REFERENCES (`#REF!`)

A reference is broken when a `Bound` `Uid` is no longer in the `SheetIndex` — the
row or column has been deleted. (A `Virtual` axis is never broken: it reads
`Empty`, since it never referred to materialized data.)

**Detection at evaluation:**
- `Op::PushCell(r)` → resolving `AxisId::Bound(uid)` finds no position → `BrokenRef`
- `Op::EvalRange(dep)` → `dep.row_range(idx)` returns `None` when a `Specific(Bound(id))` endpoint is missing → `BrokenRef`

**Propagation after delete_row/delete_col:**

```rust
fn on_delete_row(&mut self, sheet: SheetId, deleted_id: Uid) {
    // All formulas depending on this Uid will be detected as broken
    // on the next evaluation via SheetIndex.contains_*
    // Mark them simply dirty to force recomputation
    let affected = self.deps.collect_dirty_for_deleted(sheet, deleted_id, &self.sheet_indices);
    self.dirty.extend(affected);
}
```

No bytecode patching — the `AxisId::Bound(uid)` values in `CellId` and `RangeId` remain, absence in `SheetIndex` triggers `BrokenRef` at evaluation.

---

## 6. CELL MERGES

### CRDT Storage

```
"merges_{sheet_id}" (LoroMap) — LWW
  key   = "{row_uid}:{col_uid}"       (master cell = top-left corner)
  value = "{row_uid_end}:{col_uid_end}"  (bottom-right corner)
```

### Client-side rule: merge blocked if covered cell is non-empty

Before applying a merge locally (`TrameCommand::MergeCells`), the Logic thread checks that all covered cells (excluding the master) are empty in `LoroMap`. If a covered cell is non-empty or contains a formula, the merge is **rejected locally** with `TrameEvent::MergeRejected`. This is Excel and Google Sheets behavior. This rule only applies to local actions — it cannot prevent a concurrent conflict.

### Overlap Validation

LWW Loro resolves overlapping merge conflicts deterministically. Validation after each remote import: detection of cells covered by multiple merges → the merge with the most recent `ListItemID` (Loro order) wins, the other is removed.

### Concurrent Conflict Strategy: Data is Never Destroyed

**Scenario:** Alice sends `MergeCells(R3C2:R4C2)` when `R3C2` is empty on her side. Bob simultaneously sends `EditCell(R3C2, "=SUM(...)")`. Both deltas converge — result: `R3C2` is both a merge master AND non-empty in `LoroDoc`.

This conflict is unavoidable in collaborative CRDT. Rejecting Bob's delta would break convergence. **Strategy: the merge masks covered values without destroying them.**

```rust
/// Field of TrameState
masked_cells: HashSet<String>, // lid_keys of covered non-empty cells

/// Returns detected conflict events -- added to TrameOutcome.display
/// (handle_command) or the Vec<TrameEvent> returned by apply_remote().
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

Called after each `doc.import()` — returned events are added to the `Vec<TrameEvent>` of `apply_remote()`.

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

> **No Sync handle inside the state.** `TrameState` holds **no** `sync_tx` / `Box<dyn CommandSink>`. CRDT and ephemeral payloads leave the state as fields of the [`TrameOutcome`] returned by `handle_command`; the `ambre-logic` dispatcher is the single place that forwards them to the Sync thread (on a bare `flume::Sender<SyncCommand>`). The `document_id` is owned by the registry key, not by the state, so it is not a field here either.

```rust
pub struct TrameState {
    doc:           LoroDoc,
    active_sheet:  SheetId,
    sheet_indices: HashMap<SheetId, SheetIndex>,
    deps:          DependencyGraph,
    bytecode:      BytecodeCache,
    formula_cache: HashMap<CellId, CellValue>,
    dirty:         HashSet<CellId>,
    float_objects: rstar::RTree<FloatObject>,
    /// Cache name -> SheetId for resolving cross-sheet references
    /// in formulas (e.g. Sheet2!A1). Updated on each CreateSheet/RenameSheet/DeleteSheet.
    sheet_name_to_id: HashMap<String, SheetId>,
    viewport:      Viewport,
    tx_version:    u64,
    /// Last CRDT version vector exported to Sync -- produces incremental deltas
    /// via `ExportMode::Updates` in `handle_command`.
    last_exported_vv: VersionVector,
    /// Cells covered by a merge but non-empty (masked, not destroyed)
    masked_cells:  HashSet<String>,
    /// Undo/Redo local -- scope = session document ouverte
    undo_manager:  loro::UndoManager,
    /// Canonical identifier for the current user (received via LogicEvent::SessionReady).
    /// Uuid stable per-user -- not pub_key which changes on key rotation.
    my_user_id:    Uuid,
}

// Engine-side, uid-anchored cell identity (the compiled form of a positional
// `CellRef`). `AxisId::Bound(uid)` inside the used range follows the data;
// `AxisId::Virtual(pos)` past it stays positional and reads Empty (§3.A).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CellId { pub sheet: u16, pub row: AxisId, pub col: AxisId }

/// Our own, move-invariant row/column identity (§2.A). Minted as
/// `{peer = document PeerID, counter = per-session monotonic}`, stored as the
/// value of an element in the `rows_/cols_` `LoroMovableList`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Uid { pub peer: u64, pub counter: u32 }

impl Uid {
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

### `TrameState` — typed inherent methods (dispatched by `ambre-logic`)

`TrameState` exposes plain inherent methods and holds no transport handle of its own. The `ambre-logic` dispatcher owns the registry, routes by `document_id`, calls these methods, and forwards the results over the typed channels. Dirty recomputation (P4) runs synchronously at the end of each `handle_command` / `apply_remote`. No custom loop, no `setTimeout(0)`.

The state's surface (see `ambre-trame/src/logic.rs`):

```rust
// ambre-trame/src/logic.rs
use ambre_bus::{TrameCommand, TrameEvent};

impl TrameState {
    /// Process a UI command. Returns a typed `TrameOutcome`:
    ///   - `display`   : Vec<TrameEvent>  -> UI Thread (UiEvent::Viewport)
    ///   - `update`    : Option<Vec<u8>>  -> Sync (SyncCommand::SendUpdate)
    ///   - `ephemeral` : Option<Vec<u8>>  -> Sync (SyncCommand::SendEphemeral)
    /// A single command can yield several display events (e.g. EditAck +
    /// ViewportData). Loro mutation + dirty propagation + compute_all_dirty()
    /// all run synchronously here before the outcome is returned.
    pub fn handle_command(&mut self, cmd: TrameCommand) -> TrameOutcome { /* §8 dispatch */ }

    /// Apply a remote CRDT delta (decrypted by Sync). Imports into LoroDoc,
    /// rebuilds the derived caches, runs compute_all_dirty(), and returns the
    /// viewport/conflict events for the UI. No sync payload leaves here -- a
    /// remote import is never re-broadcast.
    pub fn apply_remote(&mut self, payload: &[u8]) -> Vec<TrameEvent> { /* import + recompute */ }

    /// Apply a remote ephemeral (presence) payload -> RemoteCursors events.
    pub fn apply_ephemeral(&mut self, payload: &[u8]) -> Vec<TrameEvent> { /* cursors */ }

    /// Export a full CRDT snapshot (compaction / local persistence / SnapshotNeeded).
    pub fn export_snapshot(&self) -> Vec<u8> { self.doc.export(ExportMode::Snapshot) }
}
```

**Production dispatch (`ambre-logic::LogicState`).** The single place that touches the transport — the state never does. `document_id` filtering is the registry lookup, not an `if document_id != self.doc_id` inside the state:

```rust
// ambre-logic/src/lib.rs (condensed)
match (self.docs.get_mut(&document_id)?, command) {
    (DocState::Trame(state), AppCommand::Trame(cmd)) => {
        let outcome = state.handle_command(cmd);
        // Snapshot for local persistence whenever the doc changed.
        let snapshot = outcome.update.is_some().then(|| state.export_snapshot());
        for event in outcome.display {
            self.send_to_ui(UiEvent::Viewport { document_id, event: AppEvent::Trame(event) });
        }
        if let Some(payload) = outcome.update {
            self.send_to_sync(SyncCommand::SendUpdate { document_id, payload });
        }
        if let Some(snapshot) = snapshot { self.send_to_persist(document_id, snapshot); }
        if let Some(payload) = outcome.ephemeral {
            self.send_to_sync(SyncCommand::SendEphemeral { document_id, payload });
        }
    }
}

// Remote inbound (LogicEvent::RemoteUpdate / RemoteSnapshot):
let events = state.apply_remote(&payload);           // import + recompute
let snapshot = (!events.is_empty()).then(|| state.export_snapshot());
for event in events { self.send_to_ui(UiEvent::Viewport { document_id, event: AppEvent::Trame(event) }); }
// SnapshotNeeded -> SyncCommand::SendSnapshot { document_id, payload: state.export_snapshot() }
// SessionReady / ConnectionStatus / SyncWarning are handled by LogicState, not TrameState.
```

### Command Dispatch and Synchronous Recalculation

```rust
impl TrameState {
    pub fn handle_command(&mut self, cmd: TrameCommand) -> TrameOutcome {
        match cmd {
            TrameCommand::Noop => TrameOutcome::display_only(vec![]),

            TrameCommand::ScrollViewport { version, sheet_id, .. } => {
                // P1 -- immediate viewport response, no sync payload
                self.viewport = /* ... */;
                self.compute_all_dirty(); // recompute dirty in viewport
                TrameOutcome::display_only(vec![TrameEvent::ViewportData(self.build_viewport(version))])
            }

            TrameCommand::EditCell { version, sheet_id, row, col, raw } => {
                // P2 -- Loro mutation + dirty propagation + synchronous recomputation
                let cell_ref = self.resolve_cell(sheet_id, row, col);
                self.apply_edit_cell(cell_ref, &raw);
                self.bytecode.invalidate(cell_ref);
                let dirty = self.deps.collect_dirty(cell_ref, &self.sheet_indices);
                self.dirty.extend(dirty);
                self.compute_all_dirty();
                // The incremental delta becomes TrameOutcome.update; the
                // ambre-logic dispatcher turns it into SyncCommand::SendUpdate.
                let update = Some(self.export_incremental_delta());
                let mut display = vec![TrameEvent::EditAck { version }];
                if self.viewport_intersects_dirty() {
                    display.push(TrameEvent::ViewportData(self.build_viewport(version)));
                }
                TrameOutcome { display, update, ephemeral: None }
            }

            // ... other variants: same pattern (mutation -> dirty -> compute_all_dirty -> TrameOutcome)

            TrameCommand::Undo => { /* undo_manager.undo() + rebuild */ }
            TrameCommand::Redo => { /* undo_manager.redo() + rebuild */ }

            _ => self.handle_features_and_interop(cmd),
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

If Phase 2 benchmarks show a real problem on mobile hardware, a yield mechanism can be added without touching the typed data plane or the `DocState` registry:

- `compute_all_dirty()` is replaced by `compute_dirty_batch(N)` which processes N cells and returns a boolean `more_dirty`.
- If `more_dirty`, `handle_command` adds a `TrameEvent::ComputePending` to `TrameOutcome.display`.
- The `LogicState` dispatcher in `ambre-logic`, when it sees this signal, re-enqueues a `TrameCommand::ContinueDirtyCompute` (e.g. via a self-send on the Logic command channel) so pending P1/P2 commands are drained first.
- The `DisplayViewport` is sent with `is_dirty=true` on cells not yet recomputed.

This mechanism is **additive** — it does not change `TrameState`'s method signatures or the `DocState` dispatch. It adds two variants (`TrameCommand::ContinueDirtyCompute`, `TrameEvent::ComputePending`) specific to Trame.

---

## 9. DATA-ORIENTED STRUCTURES (UI Thread)

> **Typed data plane (no postcard on the UI<->Logic hop):** `TrameCommand` (UI→Logic) and `TrameEvent` (Logic→UI) travel as **Rust values** inside `AppCommand::Trame` / `AppEvent::Trame` over the `flume` channels between the UI and Logic threads of the single WASM binary — they are **not** serialized (shared memory; see `spec-ambre-architecture.md §2/§6`). postcard is reserved for the Logic<->Sync `postMessage` hop (`SyncCommand`/`LogicEvent`) and for real network bytes. The UI thread sends a `LogicCommand::App { document_id, command: AppCommand::Trame(cmd) }`; the Logic thread `match`es it to the right `DocState` and dispatches.

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

/// Resolved cell style sent to the UI thread (projection of the "styles" LoroMap).
/// The UI renders it; it never edits it directly -- every change goes through
/// `TrameCommand::SetCellStyle`. The `fmt` choice drives `display_text` formatting,
/// computed 100% in the Logic thread (the UI computes nothing).
#[derive(Serialize, Deserialize, Clone, PartialEq)]
pub struct CellStyle {
    pub bold:      bool,
    pub italic:    bool,
    pub underline: bool,
    pub font:      Option<String>,    // None = default family
    pub size:      Option<f32>,       // None = default 11.0
    pub fg_color:  Option<Color>,
    pub bg_color:  Option<Color>,
    pub fmt:       NumberFormat,
    pub h_align:   HAlign,            // Auto | Left | Center | Right
    pub v_align:   VAlign,            // Top | Middle | Bottom
    pub wrap:      bool,             // automatic line wrapping
    pub border:    Borders,
}

/// Number/value display format. The value -> `display_text` conversion is done in the
/// Logic thread. The UI only *selects* the format (toolbar) via `SetCellStyle`.
#[derive(Serialize, Deserialize, Clone, PartialEq)]
pub enum NumberFormat {
    General,
    Number     { decimals: u8, thousands: bool },
    Currency   { decimals: u8, code: CurrencyCode },   // EUR, USD, GBP, ...
    Percent    { decimals: u8 },
    Scientific { decimals: u8 },
    Fraction   { denom: FractionDenom },   // Halves | Quarters | Eighths | upto N significant digits
    Date(DatePattern),    // Iso | LocaleShort | LocaleLong
    Time(TimePattern),    // Hm | Hms
    Text,                 // force raw text, no numeric/date auto-cast
    Custom(String),       // Excel-style format code, e.g. "#,##0.00;[Red](#,##0.00)"
}
// "Special" (zip code, phone, SSN) is locale-specific and is NOT a distinct primitive:
// it is expressed via `Custom(..)` patterns rather than a hardcoded, US-centric variant.

/// Partial style update -- only `Some(..)` fields are applied. A toolbar control
/// sets a single field at a time (e.g. toggle bold -> patch { bold: Some(true), .. }).
#[derive(Serialize, Deserialize, Clone, Default)]
pub struct StylePatch {
    pub bold:      Option<bool>,
    pub italic:    Option<bool>,
    pub underline: Option<bool>,
    pub font:      Option<Option<String>>,
    pub size:      Option<Option<f32>>,
    pub fg_color:  Option<Option<Color>>,
    pub bg_color:  Option<Option<Color>>,
    pub fmt:       Option<NumberFormat>,
    pub h_align:   Option<HAlign>,
    pub v_align:   Option<VAlign>,
    pub wrap:      Option<bool>,
    pub border:    Option<Borders>,
}

// `Color`, `HAlign`, `VAlign`, `Borders`, `CurrencyCode`, `DatePattern`, `TimePattern`, `FractionDenom`
// are defined in `ambre-trame/src/style.rs` and implement Serialize/Deserialize so
// they travel in `DisplayCell` / `TrameCommand` via postcard.

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

    // -- Format & Clipboard (trame-ui sec.5bis/6bis) ----------------------
    // Manual cell styling. Partial patch: only Some(..) fields applied.
    SetCellStyle { version: u64, sheet_id: SheetId, range: CellRange, patch: StylePatch },
    // Paste a range: relative-formula reindexing + styles done in the Logic thread
    // (the UI cache is viewport-only, sec.16).
    PasteRange   { version: u64, sheet_id: SheetId, target_row: u32, target_col: u32,
                   payload: ClipboardPayload },
    // Fill handle (autofill). mode = Copy (replicate) | Series (increment detected pattern).
    FillRange    { version: u64, sheet_id: SheetId, src: CellRange, dest: CellRange,
                   mode: FillMode },

    // -- Find & Replace (trame-ui sec.5ter) -------------------------------
    // Whole-document scope: the UI sees only the viewport, so the Logic thread scans.
    ReplaceAll   { version: u64, scope: Scope, find: String, replace: String,
                   match_case: bool, whole_cell: bool, in_formulas: bool },
    FindNext     { sheet_id: SheetId, from: CellRef, find: String,
                   match_case: bool, whole_cell: bool },

    // -- Selection stats (status bar) -------------------------------------
    // Selection may exceed the viewport -> aggregates computed in the Logic thread.
    RequestSelectionStats { sheet_id: SheetId, range: CellRange },

    // -- Sheets (lifecycle, see §2.D) -------------------------------------
    CreateSheet  { name: String },
    DeleteSheet  { sheet_id: SheetId },
    RenameSheet  { sheet_id: SheetId, name: String },
    MoveSheet    { sheet_id: SheetId, to_index: u32 },

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
/// `ambre-trame/src/features/sort.rs`. `StylePatch`, `NumberFormat`, `CellRange`, `ClipboardPayload`,
/// `FillMode`, `Scope` are defined in `ambre-trame/src/style.rs` / `ambre-trame/src/clipboard.rs`.
/// All these types implement `Serialize`/`Deserialize` to travel in `TrameCommand` via postcard.

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

    // -- Format manuel / clipboard / autofill (trame-ui) -----------------
    // The viewport is refreshed alongside; these just unblock UI busy flags / toasts.
    StyleApplied     { version: u64 },
    PasteRejected    { version: u64, reason: String },   // e.g. paste over a protected range

    // -- Find & Replace (trame-ui) ---------------------------------------
    FindResult       { matches: Vec<CellRef>, active: Option<CellRef> },
    ReplaceDone      { version: u64, replaced: u32 },

    // -- Selection stats (status bar) ------------------------------------
    SelectionStats   { sum: f64, avg: f64, count: u32, count_num: u32,
                       min: f64, max: f64 },

    // -- Undo / Redo (trame-features) ------------------------------------
    UndoState        { can_undo: bool, can_redo: bool },

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
  │  flume send: LogicCommand::App { document_id, AppCommand::Trame(cmd) }   (typed value)
  ▼
ambre-logic — LogicState::handle_app_command()
  │  match (DocState::Trame(state), AppCommand::Trame(cmd))
  │  → state.handle_command(cmd) → TrameOutcome
  │       (LoroDoc mutation, compute_all_dirty(), build_viewport — all synchronous)
  │  → for ev in outcome.display: send_to_ui(UiEvent::Viewport { AppEvent::Trame(ev) })
  │  → outcome.update    → send_to_sync(SyncCommand::SendUpdate    { document_id, payload })
  │  → outcome.ephemeral → send_to_sync(SyncCommand::SendEphemeral { document_id, payload })
  ▼
Sync thread → encryption + signature → WsClientMsg::Update → Blind Relay

Sync thread → Logic thread (remote update)
  │  postMessage (postcard): LogicEvent::RemoteUpdate { document_id, payload }
  ▼
ambre-logic — LogicState::apply_remote()
  │  match DocState::Trame(state) → state.apply_remote(&payload) → Vec<TrameEvent>
  │       (doc.import, compute_all_dirty(), build_viewport — all synchronous)
  │  → for ev in events: send_to_ui(UiEvent::Viewport { AppEvent::Trame(ev) })
  ▼
UI Thread — re-render viewport
```

---

## 10. QUALITY & TESTABILITY

```rust
#[test] fn insert_row_stable_cell_keys()            { /* row uid not invalidated */ }
#[test] fn sheetindex_o1_lookup_after_inserts()      { }
#[test] fn sort_is_uid_permutation_refs_follow()     { /* mov reorders uids; =A1 ref tracks its physical cell */ }
#[test] fn move_row_preserves_cell_keys()            { /* rows_.mov -> every cell key still resolves */ }
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

**Trame:** references are stable `Uid` values. Inserting a row within the range `A1:A10` naturally includes the new row in the computation (it lands positionally between the endpoints), without any rewriting. The formula references the same physical rows + the new row inserted within the range.

**Visible difference:** if the user inserts a row **outside** the range (before A1 or after A10), Excel does not modify the formula. Trame does not either — identical behavior. The difference is only visible for insertions **within** the range, where Trame naturally includes the new row while Excel must rewrite.

**Why this is correct for a CRDT:** Excel's implicit rewriting is a non-idempotent operation. If Alice and Bob each simultaneously insert a row in the same range, two concurrent rewrites produce an inconsistent state. The `Uid` model avoids this problem.

### References After Sort

**Excel:** after a sort, absolute references (`$A$1`) do not move, relative references (`A1`) move with the data.

**Trame:** a sort is a **pure permutation of the row order** (`rows_{sheet}.mov`, §2.A) -- it never rewrites a cell. Cells are keyed by their move-invariant row/column `Uid`, so the data follows the reorder for free, and every reference, being a `Uid` too, **follows the same physical rows**. `=SUM(A1:A10)` after sort still references the same 10 physical rows, now in a different order. Behavior equivalent to Excel absolute references (`$A$1`).

**Visible difference:** if the user expected a formula referencing "row 3" to point to "what is now in third position after the sort", Trame does not do this. This is a choice consistent with the CRDT model.

### References After Delete + Reinsert

**Excel:** if a row is deleted then reinserted at the same position, Excel can find the cell "by position" in some cases.

**Trame:** reinsertion mints a new `Uid`. The old `Uid` no longer exists in the `SheetIndex`. Any formula that referenced the old id produces `#REF!` — even if a new row is inserted at the same visual position. The user must retype or copy the formula to point it to the new `Uid`.

**Why this is correct:** deletion is an explicit intent. Finding "by position" after delete+reinsert is a fragile heuristic, especially in collaborative mode where the position may have changed between the two operations on different clients. `#REF!` is the correct information: the referenced cell no longer exists.

### Cross-Sheet References After Sheet Operations

**Excel:** renaming a sheet rewrites the name token in every dependent formula; deleting a sheet leaves `#REF!`.

**Trame:** cross-sheet references depend on the stable `sheet_id` (§2.D), not the display name. **Rename** therefore changes nothing in the dependency graph -- the formula keeps computing and only its *displayed* qualifier follows the new name. **Reorder** is pure presentation and never touches a reference. **Delete** is the sole sheet operation that removes a `sheet_id`, so it is the only one that yields a cross-sheet `#REF!`, by the same absence-triggers-broken-ref rule as a deleted row (§5).

**Why this is correct for a CRDT:** Excel's rename-rewrite is the same non-idempotent rewrite hazard as its row-insert rewrite (§12, "References After Structural Operations"). Binding to `sheet_id` makes rename and reorder convergent and reference-preserving, leaving `#REF!` to mean exactly one thing: the target no longer exists.

---

## 13. REJECTED ARCHITECTURAL DECISIONS

| Alternative | Rejected because |
|---|---|
| **Positional addressing `R{row}C{col}`** | Row insert = O(n) CRDT ops. `LoroMovableList` = 1 op. |
| **Sparse run-length rows + order-statistic tree** | Stores only non-empty rows with a shared `gap_before` counter to avoid materializing empty rows. **Not convergent:** two concurrent inserts that split the same gap both `set` the shared counter -> LWW keeps one -> absolute numbering of every following row corrupts identically on all replicas. Dense-over-used-range (each row = its own element, no shared counter) is convergent and intent-preserving; the tree would only speed the query, not remove the hazard. A `ROW_MATERIALIZE_CAP` bounds the memory cost the sparse model aimed to avoid. |
| **Loro slot/cursor id (`ListItemId`) as the row/column identity** | The element's op id (`get_cursor(pos).id`) looks stable, but `LoroMovableList::mov(from, to)` **reassigns the moved element's op id** to the move op. Every cell keyed by that slot id is orphaned on the first reorder, so a sort could only be done by rewriting cell contents (see next row). We instead mint our **own** `Uid` and store it as the element's *value*, which `mov` leaves untouched (§2.A). (This reverses the earlier "Loro already gives stable ids, a UUID is redundant" call: it holds for insert/delete but **not** for `mov`.) |
| **Sort by rewriting cell contents into fixed row slots** | A workaround for the slot-id hazard above: keep the row slots fixed and copy values/formatting into sorted order. It is O(cells) not O(rows), churns the CRDT (every moved cell is a write that replicates and lands on the undo stack), and **breaks id-based references** (a reference to "the row that moved" stays pinned to the now-overwritten slot -> positional, not the §12 follow-the-data ideal). The owned-uid model makes sort a single `mov` permutation instead. |
| **Uid-bound *range* endpoints (ranges follow the data like cells)** | A range bound to two row `Uid`s breaks under a sort: the endpoints reorder independently and can cross, inverting the rectangle (`SUM(A1:A3)` over a sorted block collapses to `0`). A rectangle cannot follow a non-contiguous set of rows. Ranges are compiled with **positional** (`Virtual`) endpoints -- a region/block -- so an internal sort leaves the block and its total intact (Excel/Sheets behavior). Only single-cell references bind to a `Uid` and follow the data (§3.A, §12). |
| **`{client_hex}{counter_hex}` format without fixed width** | Ambiguous without separator. Fixed encoding `{peer:016x}_{ctr:08x}` chosen (shared by `Uid` and `sheet_id`). |
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
- [~] Multi-sheet `LoroDoc`, per-sheet CRDT schema (§2.B: `sheets`, `cells_/format_/rows_/cols_{sheet}`, dense stable-id model) *(structural foundation landed: implicit default sheet, lazy materialization, `MATERIALIZE_CAP`; the `{v,t,s}` cell sub-map, shared `styles`, and the feature maps `filter_hidden`/`cond_formats`/`validations`/`protections` arrive with their features; explicit add-sheet + `sheet_order` arrive with the tabs UI)*
- [x] Bidirectional `SheetIndex` (`model::SheetIndex`, dense over used range) + `sheet_name_to_id`
- [x] `nom` parser: arithmetic, `CellRef`, `Range` (bounded + `EntireAxis`), basic functions, cross-sheet references (`Sheet2!A1` via `sheet_name_to_id`)
- [x] Bytecode VM: full `Op` set, `compile()`, `execute()`, `EvalContext`
- [x] Two-level `DependencyGraph` + `collect_dirty`
- [x] Cycles: static Kosaraju + dynamic `eval_stack` (cell-to-cell, ranges, cross-sheet)
- [x] `#REF!`: detection via `SheetIndex.contains_*`
- [x] `TrameCommand` and `TrameEvent`: consolidated enums (§9, single source of truth) *(MVP subset; extended per feature)*
- [x] Unit tests + property tests + postcard round-trip `TrameCommand`/`TrameEvent`

**Phase 1 — Logic Thread Integration**
- [x] Logic-thread command dispatch (`handle_command` returning `TrameOutcome`, §8) *(typed `DocState` + `match`; see `spec-ambre-architecture.md §6`)*
- [x] Synchronous recomputation `compute_all_dirty()` in each handler
- [x] `DisplayViewport` + anti-flickering (`tx_version`)
- [x] Insert/delete rows/columns (`TrameCommand::Insert/DeleteRow/Column { at }`): one `LoroMovableList` op, `rebuild_index` + positional engine reseed + formatting reseed; delete purges the removed line's cells/formatting; no-op past the used range *(Epic `E3`)*
- [x] Merges: `merges_{sheet}` LWW map (master/extent stable-id pairs), client-side non-empty rejection (`MergeRejected`), `DisplayCell.is_merged`/`merge_span`, masked covered cells (`masked_cells`, excluded from the engine, restored on unmerge), deterministic overlap precedence, conflict detection (`MergeConflictDetected`) on import. Merges follow their cells across structural ops *(Epic `E3`)*
- [x] Shared rendering module `ambre-trame/src/render.rs`: `GridLayout` + `draw_cells_on<S>()`, `draw_grid_lines_on<S>()`, `draw_col_headers_on<S>()`, `draw_row_headers_on<S>()`, all `DrawSurface`-generic. `ui::renderer` composes them (cells layer + header overlay); uniform-metrics `GridLayout` (variable widths deferred); `DisplayViewport`-fed PDF path lands with `export_pdf.rs` (Phase 3). *(Epic `E3`)*
- [x] `MockSyncHandle` for controller tests (via `CommandSink`) *(`ambre-bus::CommandSink` seam + `ambre-trame::test_support::MockSyncHandle`: captures emitted `SyncCommand`s and relays CRDT deltas peer-to-peer. Production forwards `TrameOutcome` payloads from `ambre-logic`; the test seam mirrors that exact relay via `MockSyncHandle::absorb`, so tests drive the same `SyncCommand`s the dispatcher emits) (Epic `E3`)*
- [x] Undo/Redo via Loro `UndoManager` (`TrameCommand::Undo/Redo` -> `undo_manager.undo()/redo()` + full derived-cache reseed via `after_structural` + `flush_delta` broadcast of the inverse ops; `TrameEvent::UndoState { can_undo, can_redo }`; chrome `UNDO`/`REDO` commands bound to Ctrl+Z / Ctrl+Shift+Z / Ctrl+Y). Implicit row/column materialization is committed under an excluded origin (`add_exclude_origin_prefix`) so undo reverts only the value and the cell key still resolves to its stable list-element ids on redo *(Epic `E3`)*
- [x] **Own-uid cell model (§2.A) + id-based engine (§3, §12).** Each `rows_/cols_` element carries a minted `Uid` as its *value* (replacing the Loro cursor id); `SheetIndex` reads values and gained `apply_move_row`. The compiled engine is uid-anchored: positional AST `CellRef`/`RangeDep` are resolved at compile time into `CellId`/`RangeId` over `AxisId { Bound(Uid), Virtual(pos) }` (a reference past the used range stays virtual and reads `Empty`). `SortRange` is a single `mov` permutation that **preserves** the uid-anchored bytecode (no reseed/re-parse) and recomputes, so single-cell references follow the moved data -- the §12 guarantee, proven by `sort_range_formulas_follow_data`. Ranges are positional blocks (`Virtual` endpoints), so `SUM(A1:A3)` over a sorted block stays intact (`sort_range_sum_block_is_stable_under_internal_sort`; uid-bound endpoints would invert -- see §13). Other structural ops still reset+reseed (re-bind against the converged order). Landed in stages, 164 tests green. *(Epic `E3`)*
- [ ] Manual cell formatting (`TrameCommand::SetCellStyle` + `StylePatch`): write the extended `CellStyle` (bold/italic/underline, font/size, fg/bg, `NumberFormat`, h/v align, wrap, borders) into the `styles` LoroMap; value -> `display_text` formatting per `NumberFormat` done in the Logic thread. Emits `StyleApplied`. *(Epic `E3`)*
- [ ] Clipboard paste (`TrameCommand::PasteRange` + `ClipboardPayload`): relative-formula reindexing + style application from the anchor; `PasteRejected` on protected range. (Copy/cut stay UI-local.) *(Epic `E3`)*
- [ ] Autofill (`TrameCommand::FillRange` + `FillMode`): Copy replication or Series increment (numeric/date pattern detection). *(Epic `E3`)*
- [ ] Find & replace (`ReplaceAll` / `FindNext`): full-document scan (UI cache is viewport-only); `FindResult` / `ReplaceDone`. *(Epic `E3`)*
- [ ] Selection aggregates (`RequestSelectionStats` -> `SelectionStats`: sum/avg/count/count_num/min/max) for the status bar; computed in the Logic thread since the selection can exceed the viewport. *(Epic `E3`)*

**Phase 2 — Core features (~50 functions)**
- [x] SUM, AVERAGE, MIN, MAX, COUNT, IF, AND, OR *(plus ABS, COUNTA, CONCATENATE, LEFT, RIGHT, MID, LEN, LOWER, UPPER, NOT, POWER, ROUND, SQRT, TRIM -- 22 implemented)*
- [x] VLOOKUP, INDEX, MATCH, IFERROR, SUMIF, COUNTIF *(2D `RangeValue` added to `CellValue::Range` so lookups can address cells by row/col; criteria parser for SUMIF/COUNTIF) (Epic `E3`)*
- [x] TEXT, CONCAT *(`CONCAT`/`CONCATENATE` already present; `TEXT` minimal numeric formatting: decimals + thousands grouping) (Epic `E3`)*
- [x] TODAY, NOW, DATE, YEAR, MONTH, DAY *(Excel serial numbers, epoch 1899-12-30; evaluation clock threaded through `EvalContext::now_serial` -> `call_builtin`, sourced from `functions::now_serial()` (js-sys on wasm, std::time native)) (Epic `E3`)*
- [x] Named ranges, cross-sheet references *(Epic `E3`: `CellRef`/`RangeDep` carry a local `sheet: u16`; the parser emits `Expr::SheetRef`/`Expr::Name`, resolved by a context-aware `resolve_refs` pass against a `FormulaCtx` (sheet display name -> index, defined name -> target). Named ranges fully wired e2e: `named_ranges` LWW CRDT map, `TrameCommand::DefineName`/`DeleteName`, positional resolution on reseed. `reseed_engine` is multi-sheet (active sheet pinned to index 0; cross-sheet `Sheet2!A1` evaluates in the engine, unit-tested). **Gap:** the end-to-end cross-sheet path needs a sheet-creation command (`CreateSheet`/`ActivateSheet`), which lands with the sheet-tabs UI (see Phase 0) -- the engine is ready, only the entry point is missing)*
- [x] Sheet lifecycle commands `CreateSheet` / `RenameSheet` / `MoveSheet` / `DeleteSheet` + `ActivateSheet` (§2.D): peer-scoped `sheet_id` minting (`{peer:016x}_{counter:08x}`), `sheet_order` `LoroMovableList` ops (default sheet implicit-first, never eagerly pushed), last-sheet-deletion guard (`TrameEvent::Error`), active-sheet fallback on delete, delete-driven reseed (drops per-sheet containers) + full engine remap. `TrameEvent::SheetList { sheets, active }` refreshes the tab strip (also on load). Unlocks the end-to-end cross-sheet path (unit-tested `cross_sheet_formula_evaluates_end_to_end`). *(Epic `E3`; convergent last-sheet re-insert under concurrency and the sheet-tabs UI still pending)*
- [x] Cross-sheet references bound to `sheet_id` (not display name): the CRDT `v` text is canonicalized to an id-qualified form (`#s0!A1` via the `#` sigil) at edit time, and the formula-bar text is reconstructed through `sheet_id -> name`, so rename/reorder preserve references and delete yields cross-sheet `#REF!` (§2.D rename note, §12). *(Epic `E3`: `formula::SheetQualifier` on `Expr::SheetRef`, parser `#id` token + `FormulaCtx::resolve_sheet_id`, `FormulaError::BrokenRef`/`Op::PushError`, `formula::render` AST printer with `canonicalize`/`to_display_form`, `TrameState::canonicalize_input`/`formula_display` + `sheet_id_to_u16`/`sheet_id_to_name` maps. Tests: parser `#id`/`#REF!`, render round-trips, `cross_sheet_reference_survives_rename_and_breaks_on_delete`, `cross_sheet_reference_converges_across_peers_on_rename`)* **Interop note (deferred):** xlsx/CSV export must map `#id -> name` and import `name -> #id` (tracked in the interop spec). Pre-existing display-name formulas are canonicalized on next edit (no bulk rewrite).

**Phase 3 — Optimization & Completion**
- [ ] `SheetRangeIndex` interval tree 2D if benchmarks justify it
- [ ] Floating objects R-tree
- [ ] `compute_all_dirty` benchmarks on mobile hardware — if > 50ms P95, implement the asynchronous P4 yield (`ContinueDirtyCompute` / `ComputePending`, see §8 optional Phase 2)
- [ ] Advanced functions: XLOOKUP, LAMBDA, dynamic arrays
- [ ] Rayon for independent DAG branches
- [ ] `SheetIndex` O(log N) for shifts
- [ ] PDF export via `KrillaSurface` in `ambre-trame/src/export_pdf.rs` (lazy-loaded)
- [ ] 1M-cell benchmarks
