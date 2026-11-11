# AMBRE -- TRAME: A SCHEMA FOR A CHEAP BLOCK DELETE

**Type:** Working note (study for the `A many-line delete and its undo within the block budget`
Story, `ambre-roadmap.md` E6)
**Spike:** the bench `crates/ambre-trame/benches/block_schema_spike.rs` lives on
the branch `claude/trame-block-schema`, which never merges.

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
  the profiles of the product bench (section 6). No Trame code was changed.
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

## 7. RECOMMENDATION

**Layout D: a delete hides its lines in one register, and a sweep removes them once no session can
undo.** It is the only layout whose Loro share leaves the Trame share most of the 12.8 million. It
reuses the Prose model and its sweep rule, so the suite keeps one answer to "a deletion an undo must
reverse". Park the bytecode of the removed formulas in the step in the same Story: the undo needs it
under every layout.

**Risks.**

1. The estimate of section 6 is not a measure. A second spike must drive D through `TrameState`
   before any row is written, against the product bench.
2. A document keeps each deleted line until the sweep. A crash of the deleting session leaves the
   sweep to the next editor, after the session reads as ended (`PRESENCE-014`).
3. A version-4 peer in the same live session shows the hidden lines. The schema version protocol
   bounds mixed-version editing and does not solve it (`ambre-spec-trame-schema.md section 7`).
4. Every reader of the axis must treat a hidden element as gone. `TRAME-275` makes the axis view the
   only reader, and its proptest must grow a hiding register.
5. The sweep is a write. A viewer may not write, so only an editor sweeps, as in Prose.
6. The sweep costs about 17.6 million on its Loro share alone for 500 rows. It runs off the gesture,
   in slices, as the import does (`TRAME-IO-013`).

## 8. DECISIONS FOR THE USER

| Decision | The study's answer |
| :--- | :--- |
| Reverse the two rejected alternatives, a purge at the delete | yes, for D: the purge moves to the sweep |
| `TRAME-081` | rewritten: no cell of a deleted line shows, and each leaves the document once no session can undo |
| Schema version 5, a new `line_hidden_{sheet}` key | yes, a stamp with no rewrite |
| The sweep rule | the Prose rule of `PROSE-121`, moved to a shared spec or cited from Trame |
| The movable list floor, about 11 700 instructions per removed element | report upstream to Loro. A fork is rejected (`ambre-spec-trame-undo.md section 5`) |
| A second spike through `TrameState` before the rows | yes, with a time box of one day |

## 9. HOW TO REPRODUCE

```bash
cargo bench -p ambre-trame --bench block_schema_spike --features ambre-core/fixed-hash-key --no-run
B=target/release/deps/block_schema_spike-<hash>
$B                       # every operation once, with the update sizes
valgrind --tool=callgrind --toggle-collect='*spike_hidden_delete*' $B hidden_delete
```

The product figures: the same build of `instructions`, with
`--toggle-collect='*ambre_budget_trame_block_undo*'` for the undo alone.
