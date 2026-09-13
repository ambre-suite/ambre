# AMBRE -- FEATURE: THE TRAME ENGINE
**Type:** Vertical (feature)
**Implementation:** partial -- the engine and the seam are built; the remote apply diverges, and
so does the local one.

> A user types into a cell, and every value that depended on it changes before the next frame. One
> thread holds the workbook, writes the change, recomputes what the change made stale, and answers
> with the window of cells the screen shows. The other thread draws that answer and owns nothing
> else.

The engine is `TrameState`, alone in the Logic thread. Every Trame feature reaches the document
through it, so the index, the recompute and the seam are one document and not three.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Gap | Status |
|---|---|---|---|---|---|---|
| `TRAME-001` | `INV` | The workbook CRDT MUST NOT leave the Logic thread; it exits only as bytes. | this spec section 3; architecture section 2.B | cargo-deny -- `deny.toml` bans `loro` outside the four engine crates (`SYNC-011`), so no crate on the other side of the seam can even name the type. `ambre_trame::logic::docfile::export_ambre_plaintext` is a public signature naming `LoroDoc`, but it takes the document and returns bytes, and no accessor hands one out | none | verified |
| `TRAME-015` | `CONC` | Two peers exchanging the same operations in any interleaving MUST converge to the same document. | audit `H6`; this spec section 2; `ambre-spec-testing.md` | `ambre_trame::logic::convergence_tests::two_peers_converge_under_interleaved_ops` -- a proptest over the interleavings; `ambre_trame::logic::tests::group3::two_peers_converge_via_mock_sync` | none | verified |
| `TRAME-023` | `INV` | The incrementally maintained sheet index MUST equal a from-scratch rebuild after any sequence of structural operations. | this spec section 3 | `ambre_trame::model::sheet_index::tests::apply_insert_and_delete_row_shift_positions`; `::apply_move_row_permutes_order_and_reindexes` -- per-operation unit tests | partial -- the equivalence property exists on the Toile index and not here, though this is the cache invariant the whole engine rests on | unverified |
| `TRAME-024` | `INV` | `ambre-trame` MUST contain no `unsafe`. | README pillar "Security" | none | claim -- the crate measures zero `unsafe`, and its root carries no `#![forbid(unsafe_code)]`, so the count is a measurement and not a mechanism (`ambre-spec-testing.md`, the mechanism ladder) | unverified |
| `TRAME-FEAT-001` | `INV` | A feature handler MUST NOT send on any channel: it returns a `TrameOutcome` and the Logic dispatcher fans it out. | this spec section 3; architecture section 4 | type-level -- `TrameState` holds no sink or sender, so a handler has no channel in reach; `ambre_trame::logic::tests::group3::edit_emits_send_update_through_sink` -- the dispatcher seam | none | verified |
| `TRAME-UI-001` | `INV` | The UI thread MUST hold no document data -- only view state and the viewport the Logic thread sends. | `ambre-spec-ui.md section 2` (`UI-THREAD-020`) | none | claim -- no occurrence of `LoroDoc` or `loro::` sits under `ambre-trame/src/ui/` today, but nothing runs that search: `deny.toml` bans the crate, not the directory, and `ambre-trame` is one of the four the ban allows. It is a measurement, like `TRAME-024` | unverified |
| `TRAME-UI-002` | `INV` | Every document mutation MUST leave the UI thread as a `TrameCommand`; the UI MUST NOT write document state locally. | `ambre-spec-ui.md section 2` (`UI-THREAD-020`) | type-level -- the outbound path is `TrameView::send_command_lossless` and its lossy twin (`ui/view/lifecycle.rs`), both of which take a `TrameCommand`, and the document itself is out of reach on this side (`TRAME-UI-001`) | none | verified |
| `TRAME-UI-003` | `INV` | The UI thread MUST NOT block on the Logic channel. | architecture section 2.B; `ambre-spec-ui.md section 1` | type-level -- both senders are `try_send`, and neither has a blocking form in scope | none | verified |
| `TRAME-UI-142` | `INV` | A command the UI thread failed to send MUST NOT be dropped in silence. | this spec section 4; `1-Foundations/ambre-spec-message-bus.md` (`BUS-018`) | none | fails -- two call sites drop in silence: the `SetCellsBatch` of a paste fallback (`ui/view/interact.rs`) and the `CommitEdit` on cell exit (`ui/view/lifecycle.rs`) discard the result outright. The other fifteen match `SendError::Full` and log it. Answered by the `A lost edit is never shown as applied` Story (`ambre-roadmap.md` E7) | unverified |
| `TRAME-UI-006` | `INV` | The canvas dispatch over `TrameEvent` MUST be exhaustive -- no wildcard arm -- so a new variant is a compile error, not a silent drop. | this spec section 4 | exhaustive-match -- `ui/view/frame.rs::dispatch_trame` names all 26 variants of the enum and carries no `_` arm, so adding one fails the build | none | verified |
| `TRAME-UI-014` | `INV` | There MUST NOT be an `is_error` field on `DisplayCell`. | this spec section 3 | type-level -- the struct declares no such field, and the renderer has no other input | none | verified |
| `TRAME-UI-141` | `OBS` | A cell in error MUST be marked visually from its displayed text. | this spec section 3 | `ambre_trame::ui::renderer::tests::errored_cell_gets_a_corner_marker`; `::dirty_cell_gets_a_translucent_veil` | none | verified |

### 1.2 A command reaches the engine

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Gap | Status |
|---|---|---|---|---|---|---|
| `TRAME-002` | `POST` | Recomputing what a change made stale MUST finish before the command returns. | this spec section 3 | type-level -- `handle_command` returns `TrameOutcome` by value and the crate has no `async fn`, no `.await` and no spawn, so there is nowhere to defer to. No test separates the two recomputes: the one that reads a formula's new value (`ambre_trame::logic::tests::group1::sum_over_plain_literals_and_formula_raw_in_viewport`) reads it from the viewport that follows, which recomputes again | none | verified |
| `TRAME-166` | `FRAME` | A command that mutates nothing MUST NOT produce a delta. | this spec section 3 | `ambre_trame::logic::tests::group4::active_column_values_are_distinct_sorted_and_visible` -- a read-only query carries no update; `ambre_trame::logic::tests::group3::merge_selection_single_cell_is_noop` -- the same for a refused mutation | none | verified |

### 1.3 A remote delta arrives

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Gap | Status |
|---|---|---|---|---|---|---|
| `TRAME-167` | `POST` | A batch of remote operations MUST repaint once, carrying the whole new state. | this spec section 3 | `ambre_trame::logic::tests::group7::apply_remote_batch_emits_single_datareset_with_full_state` -- it counts the resets and reads both cells back | none | verified |
| `TRAME-168` | `FRAME` | Applying a remote delta MUST NOT re-broadcast it. | this spec section 3 | type-level -- `apply_remote` returns `Vec<TrameEvent>` and no outcome, so it has no shape to broadcast through | none | verified |
| `TRAME-169` | `FRAME` | An import MUST NOT swallow the local peer's own operations before they have been broadcast. | this spec section 2, row 4 | `ambre_trame::logic::tests::group7::edit_after_reopen_broadcasts_causally_complete_delta` -- it drives two reopens and asserts the second replay carries both cells, the merge preserving this peer's entry in the export cursor and adopting every other | none | verified |

* **Budget.** the Trame rows of `ambre-spec-budgets.md section 1` -- the 10k-cell recompute
  in < 50 ms, the sheet-index rebuild of 10k rows in < 10 ms, the 50-cell batch in < 1 ms, the
  `ScrollViewport` send in < 1 ms and the `ViewportData` render in < 5 ms -- plus the workbook and
  graph holders of `section 6`. Only the recompute has a benchmark behind it.
* **Boundary.** none of its own: `TRAME-001` keeps the workbook inside this thread and lets it out
  as bytes alone, which the Sync worker seals before anything reaches the relay (`ZK-001`).
  `UI-THREAD-004` bans a network, crypto or persistence path on the other side of the seam.
* **Verification.** 13 `verified`, 4 `unverified`, 0 `unimplemented`. Eleven rows became
  seventeen, and six mechanisms came with them, so the split manufactured no coverage. What holds is
  what a type or a compiler holds: a handler with no channel in reach, an apply that returns display
  events and cannot broadcast, a dispatch the compiler checks against all 26 variants of the event
  enum. `TRAME-UI-001` is withdrawn: nothing runs the search it cites, `deny.toml` bans the crate
  rather than the directory, and `ambre-trame` is one of the four crates the ban allows -- it is a
  measurement, exactly like `TRAME-024`. Two conjunctions came apart. `TRAME-UI-014` claimed a
  drawing and an absent field, and each half has its own mechanism now. `TRAME-UI-003` claimed the
  send never waits and that a refused one reaches the log: the first is `try_send`, the second is
  `TRAME-UI-142`, which two call sites break. The three new frames were the rules the seam relied on
  and never wrote down. Four divergences between this design and the code are named in section 6.

---

## 2. WHAT EACH USER SEES

Two peers, one workbook. The last column joins the example to the test that holds it.

| Alice does | Bob does | The document holds | Each user sees | Test |
|---|---|---|---|---|
| types `=B1+1` into A1 | types `7` into B1 at the same moment | both edits, in either arrival order | `8` in A1, recomputed by each peer from the raw text alone | `logic::convergence_tests::two_peers_converge_under_interleaved_ops` (proptest) |
| edits, deletes a row and merges a block, in any order | does the same on the same sheet | every operation of both peers | the same grid on both screens, whatever order the deltas arrived in | same proptest |
| writes one cell | -- | one delta, sent once | Bob's grid one repaint later, with no request of his own | `logic::tests::group3::edit_emits_send_update_through_sink`, `::two_peers_converge_via_mock_sync` |
| closes the document and reopens it, then edits | -- | the whole causal history, not the tail | Bob applies the edit rather than rejecting it as an orphan | `logic::tests::group3::edit_after_reopen_broadcasts_causally_complete_delta` |
| sends a batch of remote operations | -- | every operation of the batch | one repaint carrying the whole new state, not one per operation | `logic::tests::group7::apply_remote_batch_emits_single_datareset_with_full_state` |
| renames a sheet a formula of Bob's refers to | writes that cross-sheet formula | the rename and the formula, joined by the sheet id | the formula still reads Alice's cell, under the new tab name | `logic::tests::group3::cross_sheet_reference_converges_across_peers_on_rename` |

Convergence rests on a rule the schema owns: a computed value never enters the document
(`ambre-spec-trame-schema.md section 1`, `TRAME-010`). Two peers holding the same raw text hold the
same grid.

---

## 3. MODEL

**Containers.** Declared in `ambre-spec-trame-schema.md section 3` and cited from there by name.
This document restates no part of the format.

**Dependencies** the engine pulls into `ambre-trame`:

| Crate | Role | Rationale |
|---|---|---|
| `loro` | the multi-sheet CRDT | `LoroMovableList` for stable rows and columns, `LoroMap` for the cells |
| `petgraph` | the cell-to-cell dependency graph | a stable graph whose removals never reindex, with a topological walk |
| `nom` | the formula parser | pure Rust, `no_std`, and a grammar close enough to a spreadsheet's to read |
| `postcard` | the style register and the clipboard blob | compact, and the encoding the Logic-to-Sync hop already uses |

### A. The state

`TrameState` is a plain value in the Logic thread's registry, `enum DocState { Trame(TrameState) }`,
reached by a `match` on the document id. It holds **no transport handle**: what must reach another
thread leaves as a return value (`TRAME-FEAT-001`).

| Group | What it holds | Rebuilt |
|---|---|---|
| the workbook | the `LoroDoc`, private, named by no public signature (`TRAME-001`) | never: it is the source of truth |
| the index | position to uid and back, for one sheet's rows and columns (section 3.B) | incrementally on a structural write, whole on an import |
| the formula engine | the dependency graph, the bytecode, the value cache and the dirty set (`features/ambre-spec-trame-formula-engine.md`) | per dirty cell; whole on an import |
| the derived caches | merges, masked cells, hidden rows, styles, axis metrics, named ranges, sheet maps | from their containers, after any import or structural operation |
| the view mirror | the active cell, the selection and the active sheet, sent up by the UI so a parameterless command resolves its own geometry | on every `SetActiveCell`. Never persisted, never synchronized |
| the session counters | the uid and sheet-id counters, and the undo history | never: a minted id lives in the document, the counter only has to be unique in this session |
| the sync cursor | the last version vector exported to Sync | advanced once the delta it produced has left |

**Code:** `crates/ambre-trame/src/logic/mod.rs` -- the state; `logic/commands.rs` -- the dispatch;
`logic/cells.rs` -- the write path; `logic/sync.rs` -- the export cursor.

### B. The sheet index

A cell key is a uid pair, so this is the bridge between what the user points at and what the
document stores. Both directions resolve in constant time.

```rust
pub struct SheetIndex {
    row_by_pos: Vec<Uid>,
    col_by_pos: Vec<Uid>,
    row_to_pos: HashMap<Uid, usize>,
    col_to_pos: HashMap<Uid, usize>,
}

impl SheetIndex {
    /// Read each list element's uid VALUE in order. O(used range), not O(100k).
    pub fn build(rows: &LoroMovableList, cols: &LoroMovableList) -> Self;

    /// Shift the positions after `pos`, and reindex.
    pub fn apply_insert_row(&mut self, pos: usize, id: Uid);
    pub fn apply_delete_row(&mut self, pos: usize) -> Uid;

    /// Permute the order to mirror a `mov(from, to)`: the primitive behind a sort.
    pub fn apply_move_row(&mut self, from: usize, to: usize);

    /// A position pair to a uid pair. `None` past the used range.
    pub fn resolve(&self, row: usize, col: usize) -> Option<(Uid, Uid)>;

    /// Whether a uid still names a line: what turns a reference into `#REF!`.
    pub fn contains_row(&self, id: &Uid) -> bool;
    pub fn contains_col(&self, id: &Uid) -> bool;

    /// Bijective base 26, the spreadsheet convention: 0 -> "A", 26 -> "AA".
    pub fn col_letter(index: usize) -> String;
    pub fn letter_to_index(s: &str) -> usize;
}
```

The vectors are dense over the used range, never over the virtual 100 000 rows
(`features/ambre-spec-trame-rows-and-columns.md`). A uid is move-invariant, so a `mov` permutes
`row_by_pos` and every cell key still resolves. The two paths that maintain the index -- the
incremental update and the rebuild -- must agree, which is `TRAME-023`, and nothing holds it.

### C. What one command costs

`handle_command` is a `match`, and every mutating arm takes the same four steps:

1. **mutate** the workbook, and commit -- one command is one undo item
   (`features/ambre-spec-trame-undo.md`, `TRAME-017`);
2. **invalidate** the bytecode of the cells the write touched;
3. **collect** the dirty set through the dependency graph and evaluate it, before returning
   (`TRAME-002`);
4. **return** the outcome: the display events, the delta, the presence payload.

Step 3 walks the graph in topological order and evaluates each dirty cell on the stack machine
(`features/ambre-spec-trame-formula-engine.md`). What each priority may cost is a row of
`ambre-spec-budgets.md section 1`.

| Priority | Trigger | What runs |
|:---|:---|:---|
| **P1** | `ScrollViewport` | no mutation and no delta: evaluate the dirty cells inside the new window, answer with one viewport |
| **P2** | `SetCellText`, `SetCellsBatch`, a structural write | the four steps above, and a fresh viewport only when the recompute touched the visible window. The UI has already drawn the edit, so there is no acknowledgement |
| **P3** | a remote delta | import, diff, dirty, evaluate, answer. **As built** the code reseeds the whole engine instead (section 6) |

### D. The seam

A `TrameCommand` goes down, a `TrameOutcome` comes up. Each variant of the two enums is declared by
the feature that owns it, in `ambre-bus::app::trame`, where the code is authoritative.

```rust
/// What a handler returns. The `ambre-logic` dispatcher is the only place that
/// turns any of these three into a message on a channel (`TRAME-FEAT-001`).
pub struct TrameOutcome {
    display:   Vec<TrameEvent>,   // to the UI thread
    update:    Option<Vec<u8>>,   // the CRDT delta to broadcast
    ephemeral: Option<Vec<u8>>,   // presence, broadcast and never persisted
}

impl TrameState {
    /// One command in, one outcome out. Mutation, invalidation, dirty
    /// propagation and recompute all finish here (`TRAME-002`).
    pub fn handle_command(&mut self, cmd: TrameCommand) -> TrameOutcome;

    /// A remote delta, already decrypted by Sync. Imports, rebuilds the derived
    /// caches, recomputes, and answers with display events alone: an import is
    /// never re-broadcast.
    pub fn apply_remote(&mut self, payload: &[u8]) -> Vec<TrameEvent>;

    /// A remote presence payload. The author is the Sync-resolved `from_user_id`
    /// and never a self-asserted one (`ambre-spec-membership.md`,
    /// `SYNC-014`); the cursor set that comes back is whole, so a departed peer
    /// disappears on its own (`features/ambre-spec-trame-selection.md`).
    pub fn apply_ephemeral(&mut self, from_user_id: UserId, payload: &[u8]) -> Vec<TrameEvent>;

    /// A full snapshot, for compaction and for a `SnapshotNeeded` from Sync.
    pub fn export_snapshot(&self) -> Vec<u8>;
}
```

The dispatcher routes by document id, calls one of the four, then explodes the result: `display` to
the UI, `update` and `ephemeral` to Sync. Its routing is normative in
`ambre-spec-message-bus.md`. A session or a connection event never reaches the state.

**The delta is a cursor, not a drain.** The export reads from the last exported version vector and
advances it once the bytes have left, so a second export from the same cursor repeats the
operations rather than answering empty.

### E. What the UI thread receives

`TrameCommand` and `TrameEvent` travel as Rust values by move, inside `AppCommand::Trame` and
`AppEvent::Trame`, over shared-memory channels: no serialization on this hop, and no `postMessage`.
The viewport answer is the `TrameEvent::ViewportData` variant, whose fields the Logic thread fills
directly -- there is no `DisplayViewport` type on the bus. One cell of it is flat:

```rust
#[derive(Debug, Clone, Default)]
pub struct DisplayCell {
    pub row: u32,
    pub col: u32,
    pub text: String,                    // the value the user reads
    pub raw: String,                     // the text they typed, for the formula bar
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strikethrough: bool,
    pub h_align: HAlign,
    pub v_align: VAlign,
    pub wrap: bool,
    pub fg: Option<u32>,                 // 0xRRGGBB, None = the theme's colour
    pub bg: Option<u32>,                 // 0xRRGGBB, None = transparent
    pub font: Option<String>,
    pub size: Option<u16>,
    pub borders: Borders,
    pub is_merged: bool,                 // a master, or a cell it covers
    pub merge_span: Option<(u32, u32)>,  // Some((col_span, row_span)) on the master
    pub is_filtered: bool,               // hidden by the active shared filter
}
```

Three shapes are deliberately absent, and each one is a rule:

| Absent | Because |
|---|---|
| an `is_error` field | a cell in error carries its error string as its displayed `text`, and the renderer marks the corner from that (`TRAME-UI-014`) |
| an `EditAck` event | an edit is optimistic: the UI has drawn it before the command left |
| an import-progress event | an import answers once, when it is done |

---

## 4. SURFACE

The user touches no part of this document directly. What it exposes is the answer every other
Trame feature draws from, and the rules that keep the drawing thread honest: it holds view state
and the current window and nothing else (`TRAME-UI-001`), every mutation leaves as a command
(`TRAME-UI-002`), and a full channel drops that command rather than blocking the frame
(`TRAME-UI-003`).

**Where each event lands.** One arm per variant and **no `_` arm**, so a new variant is a compile
error rather than a silent drop (`TRAME-UI-006`).

| `TrameEvent` | Canvas effect |
|---|---|
| `ViewportData` | cache the payload, mark the frame dirty |
| `DataReset`, `CondFormatsUpdated` | re-request the viewport |
| `SheetList` | refresh the tab strip |
| `SelectRange` | set the selection, park the input, announce it |
| `FilterApplied`, `FilterCleared` | set or clear the funnel badge on the column |
| `FindResult` | set the highlight set and navigate to the first match |
| `Error` | log it and announce it |
| `ExportComplete` | trigger the browser download |
| `Clipboard` | build the transport blob and mark it pending for the host |
| every chrome-only variant: `StyleApplied`, `UndoState`, `MergeRejected`, `MergeConflictDetected`, `SelectionStats`, `ColumnValues`, `ReplaceDone`, `PasteRejected`, `SortComplete`, `ImportComplete`, validation and protection | an explicit no-op on the canvas -- the chrome relay owns them |

Each viewport answer carries a version, and the UI drops one older than the last it took
(`features/ambre-spec-trame-grid-viewport.md`, `TRAME-UI-005`).

---

## 5. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| A handler returns an outcome and sends nothing | injecting an effect sink into the state | it is the trait object the closed `DocState` enum exists to avoid, and it puts a transport handle in the state. A value returned by a pure method is testable on the host with no channel, no worker and no browser, and the dispatcher is then the single place where a document change becomes traffic |
| The recompute finishes inside the command | a deferred pass on a timer | it buys a shorter command and pays with a grid showing stale numbers, plus a second scheduler to reason about. The dirty set is what bounds the cost, so an incremental recompute stays inside the frame budget |
| A cell reference binds to a uid; a range binds to positions | binding both endpoints of a range to uids, so a range follows the data the way a cell does | the endpoints then reorder independently under a sort and can cross, so `SUM(A1:A3)` over a sorted block collapses to zero. A rectangle cannot follow a non-contiguous set of rows, and a user expects an internal sort to leave the block total intact |
| A range is one dependency, not an edge per cell | expanding each range into cell-to-cell edges | a whole column is a hundred thousand edges, rebuilt on every row insert. Storing the rectangle and testing containment costs one entry per formula |
| A formula compiles to bytecode before it is evaluated | walking the syntax tree on each recompute | it re-parses the text every time. A formula is compiled once and executed on every recompute, and the compiled form is where a reference is bound to a uid |
| Formula cycles are detected outside the CRDT graph | reusing the document's own dependency graph for the formula cycle search | the CRDT graph orders operations causally, which says nothing about which cell reads which |
| The recompute is single-threaded | a parallel recompute over the dirty set | a fourth worker context costs a thread, a copy of the state and a synchronization rule, and the dirty set is small in every case the budget covers |
| The index is a dense vector over the used range | a sparse map over the virtual grid | it pays a hash on every resolve. The sheet the user sees is a hundred thousand rows and the sheet the document holds is the few thousand carrying data; a dense vector over the second is one lookup and a small allocation |

---

## 6. NOT BUILT, AND NOT YET RULED

* **The index equivalence has no test** (`TRAME-023`). Two paths maintain it -- the incremental
  update and the rebuild -- and only the first is driven, one operation at a time. Toile holds the
  same rule with a property test over an operation stream, and that property is what belongs here.
* **`unsafe` is measured, not banned** (`TRAME-024`). The crate root carries no
  `#![forbid(unsafe_code)]`, so nothing goes red the day a line of it appears.
* **A remote change costs the whole document.** Every remote batch reseeds the engine rather than
  taking the diff-driven path P3 prescribes, and the same reseed runs on an undo, a sheet rename, a
  name definition and a tab switch. So one peer keystroke costs O(total cells) on the receiver,
  against the client-processing budget of `ambre-spec-budgets.md section 1`. **The target
  drives the apply from the document's own diff:** a changed `cells_{sheet}` key recomputes that
  cell, a `rows_{sheet}` or `cols_{sheet}` list diff calls the incremental methods of section 3.B,
  and a derived cache is rebuilt for the container that changed. The full reseed stays the fallback
  for a snapshot import and for a diff the incremental path cannot classify. One consequence rides
  with it: a per-batch `DataReset` makes the UI re-fetch the whole viewport, where an
  invalidated-range event would name what changed. Answered by the
  "Incremental remote apply" Story (`ambre-roadmap.md` E6).
* **The sheet indexes are state here and a rebuild in the code.** The active sheet's index is cloned
  and every other sheet's is reconstructed from its full lists, once per write and once per scroll.
  A scroll on one sheet therefore rebuilds every other sheet's index, against the P1 budget on any
  multi-sheet document. **The target materializes the map as state**, invalidated per sheet by a
  structural write or an import, with the active entry shared by borrow rather than by clone.
  Answered by the "Cached sheet indexes" Story (`ambre-roadmap.md` E6).
* **A tab switch resets the engine.** The active sheet is pinned to engine index 0, so which sheet
  is displayed -- a view state -- decides engine identity. It is the direct cause of `TRAME-062`,
  the unqualified name that follows the reader from tab to tab
  (`ambre-spec-trame-named-ranges.md`). **The target interns a sheet index at first sight**, stable
  for the document lifetime, so a tab switch rebuilds the active sheet's display caches alone and
  resets no engine. Answered by the "Stable engine sheet indexes" Story (`ambre-roadmap.md` E6).
* **The recompute walks the whole graph.** The cycle check and the topological walk both cover every
  formula cell rather than the dirty closure. **The target bounds each one:** a new cycle exists
  only where the edited cell is reachable from a dependency it has just gained, which a bounded
  search answers, and a Kahn walk over the dirty closure's induced subgraph costs the dirty set
  rather than the DAG. Two further defects sit in the same path, and they belong to the graph:
  `features/ambre-spec-trame-formula-engine.md section 6`. Answered by the "A recompute bounded by
  what changed" story.
* **A local edit costs the whole viewport too** (`TRAME-002`). Section 3.C says P2 answers with a
  fresh viewport only when the recompute touched the visible window. `finish_cell_writes` answers
  with a bare `DataReset` every time, so each keystroke that commits makes the UI re-fetch the whole
  window, whether or not anything visible changed. It is the same defect as the remote one above,
  on the path taken far more often, and the invalidated-range event that answers one answers both.
* **A dropped command is silent twice** (`TRAME-UI-142`). Fifteen call sites match `SendError::Full`
  and log it; the `SetCellsBatch` of a paste fallback and the `CommitEdit` on cell exit discard the
  result. A `CommitEdit` lost this way leaves the undo group open, so the next command's fallback
  seal is what closes it -- and a lost `SetCellsBatch` is a paste the user watched land.
  `1-Foundations/ambre-spec-message-bus.md` (`BUS-018`) states the general rule, unbuilt.
* **Presence is a stub, and section 3.D describes what it will be.** `apply_ephemeral` takes a
  payload it ignores and returns an empty vector; it takes no `from_user_id` at all. The signature
  in section 3.D, and the rule about a Sync-resolved author it carries, describe the target.
  `features/ambre-spec-trame-selection.md` owns the collaborator cursors that will use it.
* **No budget is measured.** One benchmark exists, for the recompute, and no CI job runs it. Every
  divergence above is a claim about a cost that nothing weighs.
