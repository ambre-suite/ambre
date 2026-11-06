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

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TRAME-001` | `INV` | The workbook CRDT MUST NOT leave the Logic thread; it exits only as bytes. | this spec section 3; architecture section 2.B | cargo-deny -- `deny.toml` bans `loro` outside the four engine crates (`SYNC-011`), so no crate on the other side of the seam can even name the type. `ambre_trame::logic::docfile::export_ambre_plaintext` is a public signature naming `LoroDoc`, but it takes the document and returns bytes, and no accessor hands one out | none | verified |
| `TRAME-015` | `CONC` | Two peers exchanging the same operations in any interleaving MUST converge to the same document. | audit `H6`; this spec section 2; `ambre-spec-testing.md` | `ambre_trame::logic::convergence_tests::two_peers_converge_under_interleaved_ops` -- a proptest over the interleavings; `ambre_trame::logic::tests::group3::two_peers_converge_via_mock_sync` | none | verified |
| `TRAME-023` | `INV` | The incrementally maintained sheet index MUST equal a from-scratch rebuild after any sequence of structural operations. | this spec section 3 | `ambre_trame::model::sheet_index::tests::incremental_equals_rebuild::the_incremental_index_equals_a_rebuild` -- proptest: pushes, inserts, deletes and moves on both axes, compared with a rebuild after each step | none | verified |
| `TRAME-024` | `INV` | `ambre-trame` MUST contain no `unsafe`. | README pillar "Security" | lint-wall -- `unsafe_code = "forbid"` in the root `[workspace.lints.rust]`, which every crate inherits (`ARCH-032`) | none | verified |
| `TRAME-FEAT-001` | `INV` | A feature handler MUST NOT send on any queue: it returns a `TrameOutcome` and the Logic dispatcher fans it out. | this spec section 3; architecture section 4 | type-level -- `TrameState` holds no sink or sender, so a handler has no queue in reach; `ambre_trame::logic::tests::group3::edit_emits_send_update_through_sink` -- the dispatcher seam | none | verified |
| `TRAME-UI-001` | `INV` | The UI thread MUST hold no document data -- only view state and the viewport the Logic thread sends. | `ambre-spec-ui.md section 2` (`UI-THREAD-020`) | ci-gate -- `scripts/crate_boundaries.py` refuses a CRDT type, an export mode or an engine state in `ambre-trame/src/ui`, `render` and `chrome`, and its fixture covers the Trame view | none | verified |
| `TRAME-UI-002` | `INV` | Every document mutation MUST leave the UI thread as a `TrameCommand`; the UI MUST NOT write document state locally. | `ambre-spec-ui.md section 2` (`UI-THREAD-020`) | type-level -- the outbound path is `TrameView::send_command_lossless` and its lossy twin (`ui/view/lifecycle.rs`), both of which take a `TrameCommand`, and the document itself is out of reach on this side (`TRAME-UI-001`) | none | verified |
| `TRAME-UI-142` | `INV` | A command the UI thread failed to send MUST NOT be dropped in silence. | this spec section 4; `1-Foundations/ambre-spec-message-bus.md` (`BUS-018`) | `ambre_trame::ui::view::view_tests::a_refused_send_is_never_silent` -- each lossless path logs an error over a closed queue; `ambre_trame::ui::view::view_tests::a_refused_source_clear_keeps_the_cut_range` | none | verified |
| `TRAME-UI-006` | `INV` | The canvas dispatch over `TrameEvent` MUST be exhaustive -- no wildcard arm -- so a new variant is a compile error, not a silent drop. | this spec section 4 | exhaustive-match -- `ui/view/frame.rs::dispatch_trame` names all 26 variants of the enum and carries no `_` arm, so adding one fails the build | none | verified |
| `TRAME-UI-014` | `INV` | There MUST NOT be an `is_error` field on `DisplayCell`. | this spec section 3 | type-level -- the struct declares no such field, and the renderer has no other input | none | verified |
| `TRAME-UI-141` | `OBS` | A cell in error MUST be marked visually from its displayed text. | this spec section 3 | `ambre_trame::ui::renderer::tests::errored_cell_gets_a_corner_marker`; `::dirty_cell_gets_a_translucent_veil` | none | verified |

### 1.2 A command reaches the engine

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TRAME-002` | `POST` | Recomputing what a change made stale MUST finish before the command returns. | this spec section 3 | type-level -- `handle_command` returns `TrameOutcome` by value and the crate has no `async fn`, no `.await` and no spawn, so there is nowhere to defer to. No test separates the two recomputes: the one that reads a formula's new value (`ambre_trame::logic::tests::group1::sum_over_plain_literals_and_formula_raw_in_viewport`) reads it from the viewport that follows, which recomputes again | none | verified |
| `TRAME-243` | `FAIL` | A write that needs a row, column or sheet id the session can no longer mint MUST answer with a notice saying so. Each cell, line or sheet short of an id MUST stay as it was. | `ambre-spec-trame-schema.md section 2` (`TRAME-242`) | `ambre_trame::logic::tests::group2::a_cell_short_of_uids_pushes_no_line_and_keeps_the_undo_stack` -- a cell one uid short pushes neither line and leaves the undo stack alone; `::a_spent_uid_counter_answers_each_line_write_with_its_notice` -- a resize, a merge and a restyle; `::a_spent_uid_counter_refuses_new_lines_and_overwrites_nothing` -- a cell write and an insert; `::a_batch_names_the_cap_after_the_counter_is_spent` -- each refusal keeps its own cause; `ambre_trame::logic::tests::group3::a_spent_sheet_counter_refuses_the_next_sheet_and_overwrites_nothing` -- a sheet; `ambre_trame::logic::tests::group12::an_import_with_a_spent_sheet_counter_writes_nothing` -- a CSV import into a new sheet; `ambre_trame::logic::tests::group10::a_merged_tile_short_of_a_line_says_so`; `::a_carried_style_short_of_a_line_says_so` -- a fill; `::an_empty_fill_onto_a_cell_short_of_a_line_says_nothing` -- a cell that writes nothing raises no notice; `::a_pasted_style_short_of_a_line_says_so` -- a paste | none | verified |
| `TRAME-166` | `FRAME` | A command that mutates nothing MUST NOT produce a delta. | this spec section 3 | `ambre_trame::logic::tests::group4::active_column_values_are_distinct_sorted_and_visible` -- a read-only query carries no update; `ambre_trame::logic::tests::group3::merge_selection_single_cell_is_noop` -- the same for a refused mutation | none | verified |

### 1.3 A remote delta arrives

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TRAME-167` | `POST` | A batch of remote operations MUST repaint once, carrying the whole new state. | this spec section 3 | `ambre_trame::logic::tests::group7::apply_remote_batch_emits_single_datareset_with_full_state` -- it counts the resets and reads both cells back | none | verified |
| `TRAME-168` | `FRAME` | Applying a remote delta MUST NOT re-broadcast it. | this spec section 3 | type-level -- `apply_remote` returns `Vec<TrameEvent>` and no outcome, so it has no shape to broadcast through | none | verified |
| `TRAME-169` | `FRAME` | An import MUST NOT swallow the local peer's own operations before they have been broadcast. | this spec section 2, row 4 | `ambre_trame::logic::tests::group7::edit_after_reopen_broadcasts_causally_complete_delta` -- it drives two reopens and asserts the second replay carries both cells, the merge preserving this peer's entry in the export cursor and adopting every other | none | verified |
| `TRAME-248` | `FAIL` | A key or a line element that spells a uid other than as `{peer:016x}_{counter:08x}` MUST be refused, and the refusal MUST reach the user. | `ambre-spec-trame-schema.md section 2`; `1-Foundations/ambre-spec-schema-version.md` (`SCHEMA-003`) | `ambre_trame::model::sheet_index::tests::a_uid_reads_back_from_its_canonical_spelling_alone` -- a short, a signed, an upper-case and a wide spelling; `ambre_trame::logic::tests::group12::a_cell_key_in_a_non_canonical_spelling_is_refused_and_said`; `::a_non_canonical_key_in_every_keyed_map_is_said` -- the style, merge, width, height and filter maps; `::a_line_element_in_a_non_canonical_spelling_is_refused_and_said` | none | verified |
| `TRAME-231` | `POST` | Opening a workbook MUST add no operation its peers lack, so each change a silent reader receives imports at the cost of the change. | this spec section 2; `ambre-spec-budgets.md section 1` | `ambre_trame::logic::tests::group7::a_reader_that_sent_nothing_holds_no_history_of_its_own` -- the reader's version vector equals the author's after the snapshot and after a delta | none | verified |
| `TRAME-282` | `POST` | After a remote batch, each formula MUST hold the value, the dependencies, the error and the cycle refusal that a full reseed of the merged workbook gives it. | this spec section 3.F; `TRAME-042` | `ambre_trame::logic::remote_tests::a_carried_remote_apply_equals_a_full_rebuild` -- a proptest, 256 cases: two authors write cells and formulas on two sheets, write past the used range at once, insert, delete and move lines, sort, merge, redefine a name, undo and sync with each other. A receiver makes the same edits and switches sheets. One receiver carries its engine across each batch, the other reseeds after it, and they match on the stored text, each value, each dependency, each cycle refusal, the mark that sends the next structural edit to a reseed, and the stored formulas that place on no cell. Each derived layout is checked against a read of the lists; `ambre_trame::logic::remote_tests::a_remote_cell_write_recomputes_its_readers_without_a_reseed`; `::a_remote_row_delete_carries_the_engine_without_a_reseed`; `::a_remote_sheet_rename_reseeds_the_engine`; `::a_line_left_on_another_element_reseeds_the_engine`; `::a_second_key_on_a_cell_reseeds_the_engine`; `::an_undone_remote_delete_brings_back_a_value_written_on_the_gone_row`; `::a_formula_written_with_its_own_text_over_a_removed_row_compiles_again`; `::a_local_write_under_a_second_key_marks_the_engine_as_a_reseed_does`; `::a_purge_drops_a_formula_left_under_a_removed_column` -- each counts the reseeds, or replays a shrunk seed | none | verified |
| `TRAME-283` | `POST` | A remote batch that writes one cell and deletes one row of a sheet of 10k formula cells MUST hold the remote batch budget. | this spec section 3.F; `ambre-spec-budgets.md section 1` | `callgrind` -- the `remote_batch` operation of `crates/ambre-trame/benches/instructions.rs`, the whole apply of the batch, under its ceiling in the CI `instructions` job | none | verified |
| `TRAME-284` | `POST` | A remote batch MUST name the cells whose shown value it changed. | this spec section 6 | not built | unbuilt -- lands with the `Incremental remote apply` Story (`ambre-roadmap.md` E6) | unimplemented |
| `TRAME-UI-184` | `POST` | On a remote batch, the UI MUST re-fetch only the cells of its window that the batch names. | this spec section 6; `TRAME-284` | not built | unbuilt -- lands with the `Incremental remote apply` Story (`ambre-roadmap.md` E6) | unimplemented |

* **Budget.** six rows of `ambre-spec-budgets.md section 1`: the 10k-cell recompute, the
  sheet-index rebuild, the 50-cell batch, the remote batch, the viewport send and its
  render. The workbook and graph
  holders of section 6 come on top. Only the recompute has a benchmark.
* **Boundary.** none of its own: `TRAME-001` keeps the workbook inside this thread and lets it out
  as bytes alone, which the Sync worker seals (`ZK-001`). `UI-THREAD-004` bans a network, crypto or
  persistence path on the other side.
* **Verification.** 21 `verified`, 2 `unimplemented`. What holds is what a compiler holds: a handler
  with no queue in reach, an apply that returns display events and cannot broadcast. The sheet index
  equals a rebuild after every step of a random stream, and so does an engine carried across each
  remote batch. A native view reads each refused send in the log.

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
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct SheetIndex {
    row_by_pos: IdxVec<Row, Uid>,
    col_by_pos: IdxVec<Col, Uid>,
    row_to_pos: HashMap<Uid, Pos<Row>>,
    col_to_pos: HashMap<Uid, Pos<Col>>,
}

impl SheetIndex {
    /// Read each list element's uid VALUE in order. O(used range), not O(100k).
    pub fn build(rows: &LoroMovableList, cols: &LoroMovableList) -> Self;

    /// Shift the positions after `pos`, and reindex. A position past the end
    /// inserts nothing, and deletes nothing, answering `None`.
    pub fn apply_insert_row(&mut self, pos: Pos<Row>, id: Uid);
    pub fn apply_delete_row(&mut self, pos: Pos<Row>) -> Option<Uid>;

    /// Permute the order to mirror a `mov(from, to)`: the primitive behind a sort.
    pub fn apply_move_row(&mut self, from: Pos<Row>, to: Pos<Row>);

    /// A position pair to a uid pair. `None` past the used range.
    pub fn resolve(&self, at: CellPos) -> Option<(Uid, Uid)>;

    /// Whether a uid still names a line: what turns a reference into `#REF!`.
    pub fn contains_row(&self, id: &Uid) -> bool;
    pub fn contains_col(&self, id: &Uid) -> bool;
}

// ambre-trame/src/columns.rs
/// Bijective base 26, the spreadsheet convention: 0 -> "A", 26 -> "AA".
pub fn col_to_alpha(col: Pos<Col>) -> String;
pub fn alpha_to_col(s: &str) -> Result<Pos<Col>, ColumnParseError>;
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
| **P3** | a remote delta | import, read what the import wrote, carry the engine, evaluate the dirty closure, answer (section 3.F). The full reseed stays the fallback |

### D. The seam

A `TrameCommand` goes down, a `TrameOutcome` comes up. Each variant of the two enums is declared by
the feature that owns it, in `ambre-bus::app::trame`, where the code is authoritative.

```rust
/// What a handler returns. The `ambre-logic` dispatcher is the only place that
/// turns any of these three into a message on a queue (`TRAME-FEAT-001`).
pub struct TrameOutcome {
    pub display:   Vec<TrameEvent>,         // to the UI thread
    pub update:    Option<DeltaBytes>,      // the CRDT delta to broadcast
    pub ephemeral: Option<EphemeralBytes>,  // presence, broadcast and never persisted
}

impl TrameState {
    /// One command in, one outcome out. Mutation, invalidation, dirty
    /// propagation and recompute all finish here (`TRAME-002`).
    pub fn handle_command(&mut self, cmd: TrameCommand) -> TrameOutcome;

    /// A remote delta, already decrypted by Sync. Imports, rebuilds the derived
    /// caches, recomputes, and answers with display events alone: an import is
    /// never re-broadcast.
    pub fn apply_remote(&mut self, payload: &DeltaBytes) -> Vec<TrameEvent>;

    /// A remote presence payload. Answers no event until collaborator presence
    /// lands (`features/ambre-spec-trame-selection.md`); its author is the one
    /// Sync resolved, never a self-asserted one (`ambre-spec-membership.md`,
    /// `SYNC-014`).
    pub fn apply_ephemeral(&mut self, payload: &EphemeralBytes) -> Vec<TrameEvent>;

    /// A full snapshot, for compaction and for a `SnapshotNeeded` from Sync.
    pub fn export_snapshot(&self) -> SnapshotBytes;
}
```

The dispatcher routes by document id, calls one of the four, then explodes the result: `display` to
the UI, `update` and `ephemeral` to Sync. Its routing is normative in
`ambre-spec-message-bus.md`. A session or a connection event never reaches the state.

**The delta is a cursor, not a drain.** The export reads from the last exported version vector and
advances it once the bytes have left. A second export from the same cursor therefore repeats the
operations rather than answering empty.

### E. What the UI thread receives

`TrameCommand` and `TrameEvent` travel as Rust values by move, inside `AppCommand::Trame` and
`AppEvent::Trame`, over shared-memory queues: no serialization on this hop, and no `postMessage`.
The viewport answer is the `TrameEvent::ViewportData` variant, whose fields the Logic thread fills
directly -- there is no `DisplayViewport` type on the bus. One cell of it is flat:

```rust
#[derive(Debug, Clone, Default)]
pub struct DisplayCell {
    pub row: Pos<Row>,
    pub col: Pos<Col>,
    pub text: String,                    // the value the user reads
    pub raw: String,                     // the text they typed, for the formula bar
    pub bold: Weight,
    pub italic: Slant,
    pub underline: Underline,
    pub strikethrough: Strikethrough,
    pub h_align: HAlign,
    pub numeric: CellValueKind,          // Text | Number: what `Auto` resolves on
    pub v_align: VAlign,
    pub wrap: WordBreak,
    pub fg: Option<Rgb>,                 // None = the theme's colour
    pub bg: Option<Rgb>,                 // None = transparent
    pub font: Option<FontFamily>,
    pub size: Option<FontSize>,         // points
    pub borders: Borders,
    pub is_merged: Merge,                // Merged: a master, or a cell it covers
    pub merge_span: Option<MergeSpan>,   // the rows and columns it covers, on the master
    pub is_filtered: RowFilter,          // Filtered: hidden by the active shared filter
}
```

Three shapes are deliberately absent, and each one is a rule:

| Absent | Because |
|---|---|
| an `is_error` field | a cell in error carries its error string as its displayed `text`, and the renderer marks the corner from that (`TRAME-UI-014`) |
| an `EditAck` event | an edit is optimistic: the UI has drawn it before the command left |
| an import-progress event | an import answers once, when it is done |

### F. A remote batch

A remote batch costs what it wrote, not the workbook (`TRAME-282`). A subscription to the document
notes, container by container, what each import changes. A local commit, an undo and a checkout
are not noted: their own paths keep the caches. Once the batch merges, the tail reads the note.

| The batch changed | What runs |
|---|---|
| a key of a `cells_{sheet}` map | that cell compiles again from the text it now shows, and its dependents recompute |
| the rows or the columns of the sheet on screen | the position cache takes the list diffs the import reported, and reads no list: the positions after the first edit are recorded again. The engine takes the carried step of `features/ambre-spec-trame-rows-and-columns.md section 3`: it clears the cells of each gone line, and compiles again each formula that read one or reads positions. It also reads each cell that a line the batch added crosses. An undo of a delete brings a line back on its own element, so a key written under it while it was gone reads again, and no operation of the batch names that key |
| a style, size or filter map of the sheet on screen | that cache rebuilds from its container |
| the generation record of a sheet, or the peer registry | nothing: no cache holds either |

The merges rebuild after every batch, since a covered cell a peer writes is masked. The dirty
closure then recomputes in the order it induces. The events are those of a reseed.

**Where a remote batch still reseeds.** The full reseed answers a batch in five cases:

1. the batch changed the sheet list, the tab order, a defined name, the schema register, a nested
   container or a container no family names. Only a reseed rebuilds the sheet maps and the names;
2. the batch changed the line order or the merge map of a sheet off the screen. Only the active
   sheet keeps its layout from before the batch, and the carried step reads it. The layout of
   every sheet is the `Cached sheet indexes` Story (`ambre-roadmap.md` E6);
3. the engine cannot carry a local structural edit either (`TRAME-280`). It holds a formula
   refused for a cycle, a masked cell, or a cell under a second key;
4. the position cache cannot take the list diffs, and is read from the lists. That happens in
   three cases:
   * a line sits on two elements before or after the batch;
   * a value is no element this build reads;
   * a line the batch removed and added again sits on another element.

   A batch that leaves a cell under a second key, or masks a cell, reseeds too. A reseed leaves
   such a cell out of the engine;
5. the carried step meets a cycle. The feed order then decides which cell of it is refused.

A snapshot, an `.ambre` file and the restore from storage reseed too: each reads the whole state.

---

## 4. SURFACE

The user touches no part of this document directly. What it exposes is the answer every other
Trame feature draws from, and the rules that keep the drawing thread honest. The UI thread holds
view state and the current window and nothing else (`TRAME-UI-001`). Every mutation leaves as a
command (`TRAME-UI-002`), and a full queue drops that command rather than blocking the frame
(`UI-THREAD-002`).

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
| A handler returns an outcome and sends nothing | injecting an effect sink into the state | it is the trait object the closed `DocState` enum exists to avoid, and it puts a transport handle in the state. A value returned by a pure method is testable on the host with no queue, no worker and no browser, and the dispatcher is then the single place where a document change becomes traffic |
| The recompute finishes inside the command | a deferred pass on a timer | it buys a shorter command and pays with a grid showing stale numbers, plus a second scheduler to reason about. The dirty set is what bounds the cost, so an incremental recompute stays inside the frame budget |
| A cell reference binds to a uid; a range binds to positions | binding both endpoints of a range to uids, so a range follows the data the way a cell does | the endpoints then reorder independently under a sort and can cross, so `SUM(A1:A3)` over a sorted block collapses to zero. A rectangle cannot follow a non-contiguous set of rows, and a user expects an internal sort to leave the block total intact |
| A range is one dependency, not an edge per cell | expanding each range into cell-to-cell edges | a whole column is a hundred thousand edges, rebuilt on every row insert. Storing the rectangle and testing containment costs one entry per formula |
| A formula compiles to bytecode before it is evaluated | walking the syntax tree on each recompute | it re-parses the text every time. A formula is compiled once and executed on every recompute, and the compiled form is where a reference is bound to a uid |
| Formula cycles are detected outside the CRDT graph | reusing the document's own dependency graph for the formula cycle search | the CRDT graph orders operations causally, which says nothing about which cell reads which |
| The recompute is single-threaded | a parallel recompute over the dirty set | a fourth worker context costs a thread, a copy of the state and a synchronization rule, and the dirty set is small in every case the budget covers |
| The index is a dense vector over the used range | a sparse map over the virtual grid | it pays a hash on every resolve. The sheet the user sees is a hundred thousand rows and the sheet the document holds is the few thousand carrying data; a dense vector over the second is one lookup and a small allocation |

---

## 6. NOT BUILT, AND NOT YET RULED

* **A remote change of a sheet off the screen, or of the workbook, costs the whole document.** A
  remote batch carries the engine by what it wrote (section 3.F). It reseeds on the five cases
  that section 3.F lists. A new sheet, a rename, a name definition and a structural edit of
  another sheet are among them. The same reseed runs on a local undo, a sheet rename, a name
  definition and a tab switch. The `Cached sheet indexes` Story (`ambre-roadmap.md` E6) removes
  one cause, and the `Stable engine sheet indexes` Story (`ambre-roadmap.md` E6) another.
* **The UI re-fetches the whole viewport on every batch.** A per-batch `DataReset` tells it to,
  where an event naming what changed would let it re-fetch less (`TRAME-284`, `TRAME-UI-184`). Not
  yet ruled:

  * what the event names. It may name the cells whose shown value changed, or a rectangle
    around them. After a structural edit every cell below the edit moves, so it may name the
    whole window;
  * how it meets `TRAME-167`, whose test counts the `DataReset` of a batch, and the viewport
    version of `TRAME-UI-005`;
  * whether a local edit answers with the same event, as the bullet on `TRAME-002` below asks.

  Answered by the `Incremental remote apply` Story (`ambre-roadmap.md` E6).
* **The sheet indexes are state here and a rebuild in the code.** The active sheet's index is cloned
  and every other sheet's is reconstructed from its full lists, once per write and once per scroll.
  A scroll on one sheet therefore rebuilds every other sheet's index, against the P1 budget on any
  multi-sheet document. **The target materializes the map as state**, invalidated per sheet by a
  structural write or an import. The active entry is shared by borrow rather than by clone.
  Answered by the `Cached sheet indexes` Story (`ambre-roadmap.md` E6).
* **A tab switch resets the engine.** The active sheet is pinned to engine index 0, so which sheet
  is displayed -- a view state -- decides engine identity. Every bound reference therefore has to
  carry a sheet id rather than an index, as a name's definition now does
  (`ambre-spec-trame-named-ranges.md`). **The target interns a sheet index at first sight**, stable
  for the document lifetime. A tab switch then rebuilds the active sheet's display caches alone and
  resets no engine. Answered by the `Stable engine sheet indexes` Story (`ambre-roadmap.md` E6).
* **The recompute of a cell edit walks the whole graph.** The topological walk after a cell write
  covers every formula cell rather than the dirty closure. The static cycle check is already bounded
  to the descendants of the edited formula (`TRAME-012`). The engine step of a structural edit
  already walks the dirty closure alone (`ambre-spec-trame-rows-and-columns.md`, `TRAME-280`).

  **The target bounds the walk of a cell edit too.** A Kahn walk over the dirty closure's induced
  subgraph costs the dirty set rather than the DAG. Two further defects sit in the same path, and
  they belong to the graph:
  `features/ambre-spec-trame-formula-engine.md section 6`. Answered by the "A recompute bounded by
  what changed" story.
* **A local edit costs the whole viewport too** (`TRAME-002`). Section 3.C says P2 answers with a
  fresh viewport only when the recompute touched the visible window. `finish_cell_writes` answers
  with a bare `DataReset` every time. Each keystroke that commits therefore makes the UI re-fetch
  the whole window, whether or not anything visible changed. It is the same defect as the remote one
  above, on the path taken far more often. The invalidated-range event that answers one answers
  both.
* **A dropped command is silent twice** (`TRAME-UI-142`). Fifteen call sites match `SendError::Full`
  and log it; the `SetCellsBatch` of a paste fallback and the `CommitEdit` on cell exit drop the
  result. A `CommitEdit` lost this way leaves the undo group open, so the next command's fallback
  seal is what closes it. A lost `SetCellsBatch` is a paste the user watched land.
  `1-Foundations/ambre-spec-message-bus.md` (`BUS-018`) states the general rule, unbuilt.
* **Presence is a stub, and section 3.D describes what it will be.** `apply_ephemeral` takes a
  payload it ignores and returns an empty vector; it takes no author at all. Section 3.D shows that
  signature, and the rule about a Sync-resolved author it carries describes the target.
  `features/ambre-spec-trame-selection.md` owns the collaborator cursors that will use it.
* **No budget is measured.** One benchmark exists, for the recompute, and no CI job runs it. Every
  divergence above is a claim about a cost that nothing weighs.
