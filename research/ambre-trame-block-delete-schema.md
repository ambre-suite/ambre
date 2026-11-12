# AMBRE -- TRAME: A SCHEMA FOR A CHEAP BLOCK DELETE

**Type:** Working note (study for the `A many-line delete and its undo within the block budget`
Story, `ambre-roadmap.md` E6)
**Spike:** the bench `crates/ambre-trame/benches/block_schema_spike.rs`, and layout D built into
`TrameState` behind the `spike-hidden` feature, on the branch `claude/trame-block-schema`, which
never merges.

> A working note, and not a specification. It declares no contract row and changes no schema. It
> compares four layouts of the cells against the one Trame stores today, for one gesture: the delete
> of a block of rows, its undo and its redo. The user decides from it; the rows follow the decision.

## 1. QUESTION AND METHOD

**Question.** Which layout lets the delete of 500 rows on the sheet of 10k formulas, its undo and its
redo each hold the < 16 ms block row of `ambre-spec-budgets.md section 1`? That row is 12.8 million
instructions at the rate `scripts/instructions.py` assumes. `TRAME-297` and `TRAME-299` hold it.

**Method.**

1. The bench builds the sheet of `crates/ambre-trame/benches/instructions.rs` through `TrameState`.
2. It reads the plain values back out of the snapshot: row elements, cell keys and texts, generation
   keys.
3. It rebuilds one raw `LoroDoc` per layout from those values, under a fixed peer.
4. It replays what each layout writes for the gesture: the list and map writes, the commit, and the
   export of the update.
5. A no-op root subscriber stands for the two subscriptions of Trame, so Loro computes its events.
   The work of the Trame subscribers is out of the count.
6. `callgrind` counts each operation in a function of its own, `spike_<layout>_<op>`.

**Limits.**

* The spike measures the Loro share alone. The Trame share of each layout is an estimate, read from
  the profiles of the product bench (section 6). The second spike measures D whole (section 7).
* One sheet shape: two columns, every row derived by a write past the used range. A wide sheet
  changes the column figures, not the row figures.
* Every figure below: 2026-11-11, `callgrind` (valgrind 3), native release build under the `bench`
  profile (LTO, one codegen unit), `ambre-core/fixed-hash-key`, Loro 1.13.6, a cloud container. The
  commands are in section 9.

## 2. WHAT A BLOCK DELETE WRITES TODAY

The schema is `ambre-spec-trame-schema.md section 3`. On this sheet a row element is a derived
element, 51 bytes, so a cell key is 103 bytes.

| Write | Count for 500 rows | Loro share, delete |
| :--- | :--- | :--- |
| `rows_{sheet}`: one `delete(at, 500)` of the movable list | 1 call, 500 elements | 5 847 144 |
| `cells_{sheet}`: one `delete` per cell key (`TRAME-081`) | 1 000 | 5 827 379, with the next row |
| `line_gen_{sheet}`: one `delete` per generation key (`TRAME-276`) | 500 | in the row above |
| the commit, its events included | 1 | 5 389 677 |
| the export of the update | 1, 121 748 bytes | 4 471 937 |

The movable list deletes element by element inside one call. Its B-tree drain costs about 11 700
instructions per element. No layout that removes the elements at the gesture escapes that floor.

The undo writes the same keys back, and the redo removes them again. The product bench reads
41 917 294, 55 224 884 and 42 612 313 for the three, on 2026-11-11 (`python3 scripts/instructions.py`
reads the same counts).

## 3. MEASURES

The Loro share of each operation, in instructions, and the size of the update it sends.

| Layout | Delete | Undo | Redo | Update of the delete | Update of the undo |
| :--- | ---: | ---: | ---: | ---: | ---: |
| Today, flat cells purged at once | 23 229 460 | 25 731 572 | 23 458 488 | 121 748 B | 187 164 B |
| A. one nested map per row | 16 374 597 | 30 766 293 | 15 873 395 | 43 665 B | 118 366 B |
| B. purge deferred, elements removed | 5 924 907 | 4 461 882 | 6 115 198 | 158 B | 26 601 B |
| C. one tombstone key per row | 5 881 175 | 4 588 823 | 5 370 882 | 26 604 B | 26 103 B |
| D. one hiding register per delete | 476 668 | 53 910 | 473 834 | 26 139 B | 132 B |

The flat layout reproduces the Loro share the product profile gives: about 24, 28 and 26 million.

Other budgets, Loro share:

| Measure | Flat (today, B, C, D) | A |
| :--- | ---: | ---: |
| One cell write, commit and export | 63 966 | 636 996 |
| Import of the whole snapshot | 198 294 849 | 372 866 419 |
| Snapshot size | 3 303 463 B | 3 159 534 B |
| The later purge of B, C or D, off the gesture | 17 556 937 | -- |

## 4. THE FOUR LAYOUTS

### A. One nested map per row

`cells_{sheet}` maps a row element to a child map, which maps a column element to the raw text.

| Question | Answer |
| :--- | :--- |
| Delete | the list delete, and one `delete` of the row key per row. The generation keys stay as today |
| Undo | the list inserts, one `insert_container` per row, and every cell written again |
| Convergence | the map that creates a child races on its key: two creators keep one child, and the writes into the other are lost. Only the peer that pushes an element may create its map, in the same commit |
| Rows | `TRAME-081` holds as written. The history must replay nested maps: `ambre_crdt::history` has the `watch_nested` path, but `TRAME-290` ends the Trame history on a nested write today |
| A column delete | one `delete` per row that holds the column: no cheaper than today |
| Migration | a version 5. The step from 4 rewrites every cell key into a child map. Two peers migrating at once create two children per row: the race above, on every row |
| Other budgets | a cell write costs ten times more (a child lookup), the import 88 % more |

The undo costs more than today, and no operation reaches 12.8 million on its Loro share alone.
**Rejected.**

### B. Purge deferred: the elements go, the cells stay

The delete removes the list elements alone. The cells, the styles and the generation keys stay under
elements that no list holds. An undo inserts the elements back with the same spellings, and the cells
read again. One key per session, `line_hold_{sheet}`, says that the session may still undo.

| Question | Answer |
| :--- | :--- |
| Delete | the list delete, and the hold key once per session |
| Undo, redo | the list inserts, the list delete. No cell key is written |
| Later purge | a client deletes every key under an element no list holds, once no session that holds can still undo. The rule is the sweep of a hidden Prose block (`PROSE-121`) |
| Convergence | a cell a peer writes into a deleted line at the same moment stays, out of view, until the sweep. An undo then shows it, where today the undo writes the old value back over it (`TRAME-287` holds better) |
| Rows | `TRAME-081` changes: no cell of a deleted line reads, and each leaves the document once no session can undo. `TRAME-265`, `TRAME-286` and `TRAME-277` hold unchanged |
| Rejected alternatives reversed | rows-and-columns section 5, "leaving the entries and filtering them at read time"; undo section 5, "cells kept under the deleted line, hidden, and removed by a later pass" |
| Migration | a version 5 for the new `line_hold_{sheet}` key alone, a stamp with no rewrite. A version-4 reader already ignores a cell under a gone element |
| Other budgets | a cell write and the import unchanged. Each formula left under a gone element joins the set the re-anchor walk reads at every structural edit (`TRAME-280`), until the sweep |

The movable list floor stays: about 5.9 million for the delete and the redo.

### C. One tombstone key per row

The elements stay in the list. `dead_{sheet}` maps each deleted element to `true`.

| Question | Answer |
| :--- | :--- |
| Delete, undo, redo | 500 map writes each |
| Convergence | as D, below |
| Against D | the same reads and the same sweep, at 500 writes where D needs one |

**Rejected:** D does the same with one write.

### D. One hiding register per delete

The elements stay in the list. `line_hidden_{sheet}` maps a key `"{session}:{counter}"` to one binary
register: the spellings of every element the delete hides. The axis view reads a hidden element as
gone. The sweep of B later removes the elements, their cells and the register. This is the Prose
model: a deletion hides a block, an undo drops the marker (`PROSE-118`, `PROSE-120`).

| Question | Answer |
| :--- | :--- |
| Delete | one map write: about 26 KB for 500 derived elements |
| Undo | one map delete. The redo writes the register again |
| Convergence | the key is unique to its writer, so two deletes race on nothing. Two peers hiding overlapping blocks hide the union. An undo drops its own register alone, so a line a peer also deleted stays gone |
| Concurrent insert, move | an element inserted inside the block is not in the register, so it shows. A moved element keeps its spelling, so it stays hidden |
| References | a reference bound to a hidden element reads `#REF!`, as one bound to a removed element does (`TRAME-278`). The freeze of `TRAME-277` still rides the delete |
| Derivation | the line before a derived line is the last visible line. A hidden line keeps its generation keys until the sweep, which only adds history (`TRAME-271`) |
| Rows | `TRAME-081` changes as in B. `TRAME-275` keeps the change local: the axis view is the one reader of the list. `TRAME-270` already hides elements in that view |
| Rejected alternatives reversed | the same two as B |
| Migration | a version 5: a version-4 reader shows every hidden line. The step from 4 stamps and rewrites nothing. Mixed versions in one live session show the hidden lines on the old peer |
| Other budgets | a cell write unchanged. The axis view reads the registers at each rebuild: 500 hash inserts for this block. The list keeps the hidden elements until the sweep |

## 5. THE FORMULA PARSE ON UNDO

The undo of the product bench compiles the 500 restored formulas again. `BytecodeCache::recompile_if_stale`
costs 9 724 100, of which `parse_formula` 6 399 312; the engine step `FormulaEngine::apply_remote`
costs 14 679 223 (callgrind, 2026-11-11, the undo alone).

The cache already skips a formula whose text it compiled (`TRAME-212`). A delete drops that entry.
So the delete parks the bytecode and the source of each removed formula in the history step, keyed
by its cell. The undo puts each entry back when the restored text equals its source, and updates the
graph alone.

| Question | Answer |
| :--- | :--- |
| What it saves | the parse and the compile: about 9.1 million of the 9.7, the graph update stays |
| A formula that reads positions | compiles again: its bytecode depends on the layout (`TRAME-280`) |
| Memory | one bytecode per removed formula, for the life of the step. The history cap bounds it |
| Layout | independent: it serves today's layout as well as D |

The values of the parked formulas could be parked too, which skips their evaluation. The figures
below assume the bytecode alone.

## 6. ESTIMATE OF THE WHOLE GESTURE

An estimate, not a measure: the Loro share of section 3, plus the Trame share read from the product
profiles of 2026-11-11 and cut where the layout removes the work. In millions of instructions. The
column "today" splits the product counts; its Loro row reads the product profile, not the spike.

| Part | Today, delete | B, delete | D, delete | Today, undo | B, undo | D, undo |
| :--- | ---: | ---: | ---: | ---: | ---: | ---: |
| Loro writes, commit, export, step replay | 22.5 | 5.9 | 0.5 | 27.5 | 4.5 | 0.1 |
| Trame subscriptions | 5.4 | about 1 | about 0.5 | 6.2 | about 1 | about 0.5 |
| Purge key spelling, record walk | 7.2 | 0 | 0 | -- | -- | -- |
| Engine step | 3.7 | 3.7 | 3.7 | 14.7 | about 5.6 | about 5.6 |
| Restored cells read back | -- | -- | -- | 6.8 | about 2 | about 2 |
| Sheet index copy, walk, layout | 3.1 | 3.1 | 3.4 | -- | -- | -- |
| **Total** | **41.9** | **about 13.7** | **about 8.1** | **55.2** | **about 13.1** | **about 8.2** |

The redo of each layout costs about what its delete costs. B stays near the budget whatever Trame
cuts, because the movable list floor is half of it. D leaves room for the Trame share.

The second spike measured D (section 7). The estimate held for the delete and the redo. It was low
for the undo: it left out the read of the register and the walk of the slots, and it halved the
read back of the restored cells.

## 7. SECOND SPIKE: D THROUGH TRAMESTATE

**What it builds**, behind the `spike-hidden` feature of `ambre-trame`:

* `DeleteSelectedRows` writes one register in `line_hidden_{sheet}` and no cell, style or generation
  key. The key is `"{axis}:{peer}:{first element hidden}"`.
* The axis view keeps each hidden element in its slots and shows no line for it. A reference bound
  to it reads `#REF!`.
* The subscription notes a register written or deleted as a layout change. An undo, a redo and a
  peer's batch then take the carried step of a remote batch (`TRAME-285`).
* The re-anchor walk and the reseed leave a cell under a hidden element alone: it comes back with
  its text.
* `TrameState::spike_sweep` removes the hidden elements, their cells, styles and generations, and
  the registers, in one commit that no history records. The rule of when it may run is not built.

**Measures.** 2026-11-11, `callgrind` on `crates/ambre-trame/benches/instructions.rs`, the whole
command counted as the CI job counts it, subscribers included. The bench profile, the
`ambre-core/fixed-hash-key` feature, a cloud container. "Today" is the same build without
`spike-hidden`.

| Operation | Today | D | D, less the 9.1 million the parked bytecode saves | Budget |
| :--- | ---: | ---: | ---: | ---: |
| Delete of 500 rows | 41 845 748 | 8 671 365 | -- | 12 800 000 |
| Its undo | 55 195 521 | 20 903 316 | about 11 800 000 | 12 800 000 |
| Its redo | 42 719 549 | 6 831 108 | -- | 12 800 000 |
| The sweep of the 500 rows, off the gesture | -- | 47 982 093 | -- | none |
| One row delete after the block, before the sweep | 4 360 982 | 18 623 217 | -- | 8 000 000 |

The delete and the redo hold the budget as measured. The undo holds it only with the bytecode parked
in the step, which another session builds in the product code. The 9.1 million is that session's
estimate: the parse and the compile of 500 formulas, 9 666 127 in this profile.

**Where the undo spends**, in this profile: the engine step 14 547 219, of which the parse 6 399 448;
the read back of the 1 000 restored cells 3 596 416; the register read and the walk of the slots
1 615 736. The Loro share is under 0.5 million.

**Snapshot size**, in bytes, history included:

| State | Today | D |
| :--- | ---: | ---: |
| The sheet of 10k formulas | 2 845 721 | 2 845 721 |
| After the delete of 500 rows | 2 837 777 | 2 877 588 |
| After the sweep | -- | 2 853 871 |

The register adds about 26 KB to the history, and the history keeps it after the sweep.

**Convergence.** Three tests in `crates/ambre-trame/src/logic/tests/spike_hidden.rs`. After each step,
the two peers and a third peer rebuilt from the converged snapshot show one grid:

1. A hides rows 3 to 6. B, at the same moment, types into row 4, inserts a line inside the block and
   types into it. The merged grid shows the inserted line, not the hidden rows, and not B's value.
   A undoes: the block comes back with B's value, and its formula reads it. B undoes, and A redoes.
2. Two peers hide overlapping blocks. One undoes: the rows the other hid stay hidden.
3. The sweep removes what the register hid, and the grid does not change.

Under the feature, the proptests of `TRAME-280` (`a_carried_engine_equals_a_full_rebuild`) and of
`TRAME-286` (`a_recorded_line_delete_undoes_as_the_undo_manager`) pass, 802 of the 804 tests of the
library in all. The two that fail freeze the schema shape, and a new root fails them by design.

**What the spike found.**

1. **A row delete after a hidden block costs four times more.** The view gives up its incremental
   paths while an element is hidden, and reads the lists twice: 14 953 235 of the 18 623 217. The
   product must teach those paths to step over a hidden slot. Until then every line edit pays it,
   until the sweep.
2. **A formula under a hidden line is held, not orphaned.** The walk of `TRAME-280` and the reseed
   must skip it. Otherwise each delete writes `#REF!` into every hidden formula, one cell write per
   formula, and the gain is lost.
3. **The sweep costs 48 million**, not the 17.6 million of its Loro share: the purge, the two
   subscriptions and a read of the lists. It runs off the gesture, and must run in slices.

## 8. RECOMMENDATION

**Layout D: a delete hides its lines in one register, and a sweep removes them once no session can
undo.** It is the only layout whose Loro share leaves the Trame share most of the 12.8 million. It
reuses the Prose model and its sweep rule, so the suite keeps one answer to "a deletion an undo must
reverse". Park the bytecode of the removed formulas in the step in the same Story: the undo needs it
under every layout. Measured on 2026-11-11, D holds the budget for the delete and the redo, and for
the undo once the bytecode is parked (section 7).

**Risks.**

1. The undo holds the budget by about 1 million, once the bytecode is parked. The read back of the
   restored cells, 3.6 million, is the next cut if that margin goes.
2. The incremental paths of the axis view must step over a hidden slot, or every line edit before
   the sweep breaks the line delete row (section 7, finding 1).
3. A document keeps each deleted line until the sweep. A crash of the deleting session leaves the
   sweep to the next editor, after the session reads as ended (`PRESENCE-014`).
4. A version-4 peer in the same live session shows the hidden lines. The schema version protocol
   bounds mixed-version editing and does not solve it (`ambre-spec-trame-schema.md section 7`).
5. Every reader of the axis must treat a hidden element as gone. `TRAME-275` makes the axis view the
   only reader, and its proptest must grow a hiding register.
6. The sweep is a write. A viewer may not write, so only an editor sweeps, as in Prose.
7. The sweep costs about 48 million for 500 rows. It runs off the gesture, in slices, as the import
   does (`TRAME-IO-013`).

## 9. DECISIONS FOR THE USER

| Decision | The study's answer |
| :--- | :--- |
| Reverse the two rejected alternatives, a purge at the delete | yes, for D: the purge moves to the sweep |
| `TRAME-081` | rewritten: no cell of a deleted line shows, and each leaves the document once no session can undo |
| Schema version 5, a new `line_hidden_{sheet}` key | yes, a stamp with no rewrite |
| The sweep rule | the Prose rule of `PROSE-121`, moved to a shared spec or cited from Trame |
| The movable list floor, about 11 700 instructions per removed element | report upstream to Loro. A fork is rejected (`ambre-spec-trame-undo.md section 5`) |
| A second spike through `TrameState` before the rows | done on 2026-11-11 (section 7) |
| The incremental view over a hidden slot, in the same Story | yes: without it a line edit before the sweep breaks its budget |

## 10. HOW TO REPRODUCE

```bash
cargo bench -p ambre-trame --bench block_schema_spike --features ambre-core/fixed-hash-key --no-run
B=target/release/deps/block_schema_spike-<hash>
$B                       # every operation once, with the update sizes
valgrind --tool=callgrind --toggle-collect='*spike_hidden_delete*' $B hidden_delete
```

The product figures: the same build of `instructions`, with
`--toggle-collect='*ambre_budget_trame_block_undo*'` for the undo alone.

The second spike:

```bash
cargo bench -p ambre-trame --bench instructions \
  --features ambre-core/fixed-hash-key,spike-hidden --no-run
B=target/release/deps/instructions-<hash>
valgrind --tool=callgrind --toggle-collect='*ambre_budget_trame_block_remove*' $B many_line_delete
valgrind --tool=callgrind --toggle-collect='*ambre_budget_trame_block_sweep*' $B many_line_sweep
cargo test -p ambre-trame --features spike-hidden --lib -- spike_hidden
```

## 11. THE UPSTREAM REPORT, READY TO FILE

The user decided on 2026-11-11 to report the movable list floor upstream, and to keep a fork
rejected (`ambre-spec-trame-undo.md section 5`). Nothing was sent. The text below is ready for the
Loro issue tracker, and holds no name, no document and no figure of a user. A maintainer of Ambre
files it by hand.

```text
Title: LoroMovableList::delete(pos, len) costs ~11.7k instructions per element

Version: loro 1.13.6, native release build (LTO, one codegen unit), x86-64.

What we see
-----------
One call `list.delete(at, 500)` on a LoroMovableList of 10 000 string
elements (51 bytes each), followed by one commit, costs about 5.85 million
instructions under callgrind: about 11 700 per removed element. The cost is
linear in `len`. The profile points at the drain of the list item B-tree
(`generic_btree::iter::Drain<MovableListTreeTrait>` and its drop), and at
one op per element in the oplog.

Why it matters to us
--------------------
A spreadsheet stores its row order in a LoroMovableList, so that a move keeps
each row's identity. Deleting a block of 500 rows must fit a 16 ms frame on a
mid-range phone, about 12.8 million instructions on our scale. The list
delete alone takes half of that before the application does any work.

Minimal reproduction
--------------------
    use loro::{LoroDoc, ExportMode};

    fn main() {
        let doc = LoroDoc::new();
        doc.set_peer_id(1).unwrap();
        let list = doc.get_movable_list("rows");
        for i in 0..10_000u32 {
            list.push(format!("{i:051}")).unwrap();
        }
        doc.commit();
        // Count from here, for example with
        // valgrind --tool=callgrind --toggle-collect='*delete_block*'
        delete_block(&doc);
    }

    #[inline(never)]
    fn delete_block(doc: &LoroDoc) {
        doc.get_movable_list("rows").delete(5_000, 500).unwrap();
        doc.commit();
        let _ = doc.export(ExportMode::all_updates());
    }

What we ask
-----------
Is a run delete that drains one B-tree range at once, and records one op
for the run, possible within the movable list's semantics? A move of an
element inside the run, concurrent with the delete, is the case we expect
to constrain it. We would gladly test a branch against our benchmark.
```

## 12. WHAT THE STORY BUILT

Layout D shipped with the story `A many-line delete and its undo within the block budget`, on
2026-11-12. The six decisions of section 9 hold in code, and none turned out wrong.

**Measures.** 2026-11-12, `python3 scripts/instructions.py` and `callgrind` on
`crates/ambre-trame/benches/instructions.rs`, under the `fixed-hash-key` feature and a pinned peer,
native release build in a cloud container. "Before" is `origin/main` the same day, with the
bytecode park of `TRAME-300` built.

| Operation | Before | After | Budget |
| :--- | ---: | ---: | ---: |
| Delete of 500 rows | 41 982 315 | 9 113 943 | 12 800 000 |
| Its undo | 47 911 293 | 10 329 819 | 12 800 000 |
| Its redo | 42 844 650 | 6 499 149 | 12 800 000 |
| One row delete while the block stays hidden | 4 572 224, with no block | 4 509 806 | 8 000 000 |
| One slice of the sweep, 32 rows | -- | 5 065 283 | 8 000 000 |
| The whole sweep of the 500 rows, 17 slices | -- | 88 691 941 | none |

**Where each risk of section 8 went.**

1. The undo first read 13 761 414: the read back of the 1 000 restored cells cost 3 577 636 of
   it. The index by line now holds each cell's value, a shared copy of the map's own, so the undo
   spells no key and reads no map.
2. The incremental paths of the axis view step over a hidden slot: a removal, an insert and a list
   diff each keep the hidden elements in place.
3. The next editor sweeps after a crash, once the relay reads the session as ended.
4. A build of version 4 refuses a workbook of version 5 (`TRAME-306`). A version-4 peer already in
   a live session still shows the hidden lines until it reopens.
5. The four-peer proptest of `TRAME-278` grew a block delete, an undo and a sweep (`TRAME-303`).
6. Only an editor starts a sweep (`TRAME-305`).
7. The sweep runs one slice of 32 elements per pass. A slice of 64 read 9 721 630, over the
   8 million of the structural edit row: the walk of the generation record took 2 356 603 of it.
   The generations now go in one last slice.
