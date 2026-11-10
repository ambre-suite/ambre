# AMBRE -- THE BUDGET REGISTER

**Type:** Register (every budget of the workspace)

> Every performance and resource target Ambre commits to, one row each. A rule that must hold a
> budget cites the row here instead of restating the figure.

* **Every budget of the workspace is a row here.** A budget a reader cannot compare to its
  neighbour is a number, not a target. No component spec carries one of its own, and a rule that
  must hold a budget cites the row instead (`CANVAS-005`, `FIXTURE-007`). `Method` names the mechanism
  that measures the row, and "**none exists**" is the honest value.
* **This file is a register, not a specification.** It states no rule of its own, so it declares no
  contract table: `ambre-spec-template.md section 4` holds what a file of this class owes.

---

## 1. THE CLIENT, PER OPERATION (TARGET: MOBILE HARDWARE 2024)

| Operation | Target (P95 mobile) | Ceiling | Thread | Method |
|-----------|-------------------|---------|--------|---------|
| queue send/receive UI <-> Logic (`ambre_bus::conduit`, shared heap, by move) | < 1 us | -- | UI <-> Logic | benchmark |
| Logic <-> Sync `postMessage` (postcard encode + structured clone / transfer) | < 1 ms | -- | Main <-> Sync | benchmark |
| Scroll round-trip UI -> Logic -> UI (P1) | < 2 ms | -- | UI <-> Logic | benchmark e2e |
| Outbound encryption 1 KB (compress+AEAD+HMAC+Ed25519) | < 200 us | < 500 us | Sync | criterion |
| Inbound decryption 1 KB (verify+HMAC+AEAD+decompress) | < 350 us | < 1 ms | Sync | criterion |
| Full crypto round-trip (excluding network) | < 600 us | < 1.5 ms | Sync | criterion |
| Snapshot compression 100 KB (ruzstd) | < 5 ms | -- | Sync | criterion |
| Passphrase-sealed file, one seal or one open (Argon2id, then one AEAD pass over the whole container) | < 1 s | < 2 s | Sync | the login work factor, deliberately: 64 MiB, 3 passes, 1 lane, paid per file rather than per session, because it is also what an offline guessing attack pays (`ambre-spec-docfile.md`, `DOCFILE-014`). The 64 MiB is the largest transient the Sync worker allocates, and it is released with the call -- **none exists** |
| Quarantine release, 100 updates | < 1 ms | -- | Sync | criterion -- **none exists** |
| `doc.import()` Loro update (1 KB delta) | < 2 ms | -- | Logic | criterion |
| Murmure authorship check, one received delta read as a Loro JSON update | < 50 us | -- | Logic | criterion `ambre-murmure` `segment_json_check` -- 2.3 us on a desktop in native code, whatever the size of the segment. The fork it replaced measured 3.5 ms at 500 messages and 51.6 ms at 5000 (`segment_fork_check`). The phone figure assumes a phone about 7 times slower than the desktop: the ratio of the inbound open budget (350 us) to its desktop measure (50 us). The `Murmure measured on a phone` Story (`ambre-roadmap.md` E5) replaces that ratio with a measure |
| Authorship check, one received delta of Trame, Toile or Prose read from its Loro blob metadata | < 1 ms | -- | Logic | criterion -- **none exists**: `decode_import_blob_meta` decodes the whole blob without importing it, so its cost grows with the delta |
| Murmure space leave, the rotation of the space key and of every public channel key | < 2 s, for 50 public channels and 200 members, to measure on a phone | -- | Sync | **none exists** -- one wrap per remaining member for each key, so about 10 000 key exchanges at that size. A measure past the target brings back one key for every public channel (`features/ambre-spec-murmure-spaces.md`) |
| Murmure segment, Loro import of its snapshot, 5000 messages | < 10 ms | -- | Logic | criterion `ambre-murmure` `segment_snapshot` -- 3.0 ms on a desktop in native code |
| Murmure segment, Loro replay of one delta past its snapshot | < 250 us | -- | Logic | criterion `ambre-murmure` `segment_replay_one_by_one` -- about 35 us on a desktop in native code: 220 ms for the 6250 deltas of a segment at its close |
| Trame formula DAG recompute (10k cells) | < 50 ms | -- | Logic | custom benchmark -- **none exists** |
| Scroll viewport response (P1, end-to-end) | < 15 ms | -- | Logic -> UI | benchmark e2e |
| Trame CSV import (10k lines) | < 200 ms | -- | Logic | criterion -- **none exists** |
| Trame CSV export (10k lines) | < 100 ms | -- | Logic | criterion -- **none exists** |
| Trame xlsx import (1k cells / 10k cells) | < 500 ms / < 3 s | -- | Logic | criterion -- **none exists**, and no gesture reaches the codec. The codec alone reads 100k cells in about 100 ms: 2026-11-09, `cargo test --release -p ambre-xlsx --test large_sheet -- --nocapture`, host build in a cloud container, three runs |
| Trame xlsx export (10k cells) | < 1 s | -- | Logic | criterion -- **none exists**, and no gesture reaches the codec. The codec alone writes 100k cells in 107 to 124 ms: 2026-11-09, the same command, the same three runs |
| Trame `SetCellText` to viewport (P2) | < 5 ms | -- | Logic | recompiles the bytecode and updates the DAG -- **none exists** for the write itself. The CI `instructions` job holds the whole undo and the whole redo of one cell write on the sheet of 10k formulas to this row, through `cell_write_undo` and `cell_write_redo` in `crates/ambre-trame/benches/instructions.rs`, counted in the functions named after them, against a ceiling of 4 000 000 each. Measured by callgrind on 2026-11-08, native release build in a cloud container: through the Loro undo manager, 247 030 884 for a first undo, 299 192 706 for a second and 199 521 206 for a redo, 99.9 % of each in its checkout. With the edit recording its own inverse (`TRAME-288`): 231 789 and 227 997 under `just instructions` |
| Trame batch of 50 cells (P4) | < 1 ms | -- | Logic | **none exists** |
| Trame `SheetIndex` rebuild, 10k rows | < 10 ms | -- | Logic | at load time -- **none exists** |
| Trame structural edit on the Logic thread, 10k formula cells: the re-anchor walk and the engine step of one row or column insert, delete or move | < 10 ms | -- | Logic | the 15 ms of a content update, the Scroll viewport response row above, less the < 5 ms frame total of section 2. A recompute the user asks for is the Trame formula DAG recompute row, not this one. The CI `instructions` job holds it through `structural_edit` in `crates/ambre-trame/benches/instructions.rs`: a delete of the middle row, counted in `reanchor_formulas` and `carry_engine`, against a ceiling of 8 000 000. Measured by callgrind, native release build in an Intel Xeon 2.30 GHz cloud container. On 2026-11-05, before the carried engine: the walk 249 845 678 instructions; the reseed 11 003 622 904, of which the whole-graph cycle check 8 499 150 000, the compile and graph build about 1 841 000 000, the recompute 168 075 977; the whole delete 11 452 904 754. On 2026-11-06, after it: the walk and the engine step 4 408 783 under `just instructions`, about 5.5 ms on the target phone at the rate `scripts/instructions.py` assumes; the whole delete 162 433 725. The purge and the commit fall outside the count. The whole delete is the row below. On 2026-11-07: 87 204 under `just instructions`. The delete now derives the layout after it as it removes the line, outside the walk, and the engine step borrows that layout instead of copying it |
| Trame whole line delete on the Logic thread, 10k formula cells: one `DeleteRow` command from its start to its outcome, the purge, the commit and every cache included | < 10 ms | -- | Logic | the 15 ms of a content update, the Scroll viewport response row above, less the < 5 ms frame total of section 2, as for the structural edit row above, which is its walk and its engine step alone. The CI `instructions` job holds it through `line_delete` in `crates/ambre-trame/benches/instructions.rs`, counted in the whole `ambre_budget_trame_structural_edit` function, against a ceiling of 8 000 000. It holds the whole undo and the whole redo of that delete to the same budget each, through `line_delete_undo` and `line_delete_redo`, counted in the whole `ambre_budget_trame_line_delete_undo` and `ambre_budget_trame_line_delete_redo` functions. Those of a row insert in the middle of the sheet hold the same budget, through `line_insert_undo` and `line_insert_redo`. Each operation counts its step in a function of its own, named after it. Measured by callgrind, native release build in an Intel Xeon 2.30 GHz cloud container. On 2026-11-06: 162 433 725 instructions, about 203 ms on the target phone at the rate `scripts/instructions.py` assumes, of which the purge, a walk of the whole cell map, about 116 000 000. On 2026-11-07, with the purge read from an index by line, the slots read from the position cache and the layout lent to the engine: 4 858 050 under `just instructions`, about 6.1 ms. The tail of the undo 2 395 320 and of the redo 2 401 417, about 3.0 ms each. The step of the Loro undo manager fell outside the tail: about 336 000 000 for the undo and 288 000 000 for the redo, measured the same day. It rebuilt the state of the whole cell map at a version. Later on 2026-11-07, with the delete recording its own inverse (`TRAME-286`), the whole undo and the whole redo each, under `just instructions`: 6 045 659 and 5 354 932, about 7.6 ms and 6.7 ms. The whole delete reads 4 906 798 the same day. On 2026-11-08, an undo of a row insert went through the checkout of the Loro undo manager: 49 962 823 for the undo and 2 650 288 for the redo, under the same count. With the insert recording its own inverse: 5 158 926 and 5 552 100 under `just instructions`, about 6.4 ms and 6.9 ms. That path was withdrawn the same day: its redo put back a new list element, and the manager's redo of a later move revived the old one (`2-Trame/features/ambre-spec-trame-undo.md` section 6). The undo and the redo of the delete read 5 756 689 and 5 409 913 the same day. The delete of a block of rows is the row below. The undo and the redo of one line delete then read 6 187 243 and 2 517 595 under `just instructions`. Later on 2026-11-08, with every edit recording its own inverse into one history and no undo manager (`TRAME-288`): under `just instructions`, the undo and the redo of one line delete 3 721 120 and 3 351 188, those of a row insert 3 129 512 and 3 440 342. The whole delete reads 4 933 212. The tail of the undo is about 73 000 000 of it, the formula engine recompiling each restored formula and the change observers parsing each restored key. The delete itself cost 1 658 407 733 the same day, about 2.1 s |
| Trame delete of a block of five hundred rows on the Logic thread, 10k formula cells: one `DeleteSelectedRows` command, its whole undo and its whole redo, each | < 16 ms | -- | Logic | the frame at 60 FPS, the target set on 2026-11-08: a block delete is rarer than a keystroke, so it may take a frame where a content update takes 10 ms. `many_line_delete`, `many_line_delete_undo` and `many_line_delete_redo` in `crates/ambre-trame/benches/instructions.rs` count each in a function of its own. The CI `instructions` job holds each under the count reached, not under the target: 52 500 000, 73 500 000 and 52 500 000. A ceiling moves down only. Measured by callgrind on 2026-11-08, native release build in a cloud container. At the start of the day: the delete 1 666 842 974 instructions, its undo 1 550 269 610 and its redo 1 454 367 305, about 2.1 s, 1.9 s and 1.8 s on the target phone at the rate `scripts/instructions.py` assumes. The delete walked the generation record and renumbered the layout once per line, and the undo read the row list once per element. With one history, one pass of the list and of the record for the whole block, and each key spelled by hand: 85 146 535, 101 044 252 and 73 363 855, about 106 ms, 126 ms and 92 ms. Of the undo, about 17 % hashes keys through SipHash, and the formula engine recompiles each of the 500 restored formulas. Later on 2026-11-08, the same base read 85 437 547, 101 414 089 and 74 238 101 under callgrind. With every Trame table on foldhash under a key drawn per process (`TRAME-292`): 58 398 348, 74 592 577 and 54 318 734. With the dirty closure walked without a `Dfs` per cell and the purge of the record by hash: 50 392 801, 70 865 335 and 50 280 585, about 63 ms, 89 ms and 63 ms. What stays is Loro: the list and map writes, the commit with its observers and the export are about half of each. The formula parser takes about 8 000 000 of the undo |
| Trame remote batch on the Logic thread, 10k formula cells: one `apply_remote_batch` of a peer's cell write and row delete, from the import to the events, every cache included | < 10 ms | -- | Logic | the 15 ms of a content update, the Scroll viewport response row above, less the < 5 ms frame total of section 2, as for the structural edit row above: a peer's change is a content update on this screen. The CI `instructions` job holds it through `remote_batch` in `crates/ambre-trame/benches/instructions.rs`, counted in the whole `ambre_budget_trame_remote_batch` function, against a ceiling of 8 000 000. Measured by callgrind on 2026-11-06, native release build in an Intel Xeon 2.10 GHz cloud container, command `valgrind --tool=callgrind --toggle-collect='*ambre_budget_trame_remote_batch*'` on the bench binary with the argument `remote_batch`. Before the carried apply: 1 218 213 117 instructions, about 1 523 ms on the target phone at the rate `scripts/instructions.py` assumes, of which the engine reseed 1 159 983 829. After it: 2 595 061, about 3.2 ms. Later the same day, with the walk of formulas that place on no cell and the ranged repaint: 2 652 475 under `just instructions`, about 3.3 ms. The layout derived from the list diffs replaced a read of the 10k-row list, which alone cost about 41 600 000 |
| Trame `range_index` scan, 1k range formulas | < 1 ms | -- | Logic | linear scan at the MVP, an interval tree later -- **none exists** |
| Toile visible-set query, 50 000 elements | < 1 ms | -- | Logic | must fit inside a frame with margin for tessellation -- **none exists** |
| Toile index update, one element | < 50 us | -- | Logic | a drag mutates one element per input event -- criterion `ambre-toile` `index_update_one_of_50000`: 1.3 us on a desktop in native code (`TOILE-091`) |
| Toile remote change, delta < 32 KB | < 2 ms | -- | Logic | keeps the client share of the < 300 ms inter-user budget -- criterion `ambre-toile` `remote_change_5000`: one move costs 0.8 ms on a desktop in native code, against 48 ms before the scaffolding was shared (`TOILE-265`) |
| Toile transform command (move, restyle) | < 1 ms | -- | Logic | one register write, one index update, one derivation -- **none exists** for the command itself. Its whole undo and its whole redo, on a board of 2 000 stickies, are `move_undo` and `move_redo` in `crates/ambre-toile/benches/instructions.rs`, each counted in a function named after it. Measured by callgrind on 2026-11-08, native release build in a cloud container: 59 930 493 and 59 594 465 instructions, about 75 ms on the target phone, 98 % of it the whole reseed of the caches. On 2026-11-09, once an undo refreshes only what it wrote (`3-Toile/features/ambre-spec-toile-undo.md`, `TOILE-292`): 769 315 and 768 985, the median of three runs of `valgrind --tool=callgrind --toggle-collect` on the bench binary. Met: the CI `instructions` job holds both at 800 000 (`TOILE-291`). The derivation of the viewport is 86 % of it, and SipHash over the paint order is 61 %: the margin is thin |
| Toile body commit, a 4 KB body merged from its base | < 1 ms | -- | Logic | two character diffs capped at 256 edit steps, then the merge's edits written into the body, per close (`TOILE-UI-381`). The UI echo runs the same two diffs inside its frame (`TOILE-UI-382`) -- `callgrind` `ambre-toile` `body_commit`: about 690 000 instructions against a ceiling of 800 000 (`TOILE-UI-383`). Both diffs run to their cap cost about 6 million, 7 ms on the phone: the bound is held at 6.5 million, and it is not the budget |
| Toile slide transition, 500 elements per frame | < 4 ms | -- | Logic | leaves room to paint inside the frame during a transition -- **none exists** |
| Toile layout solve, 200 children | < 1 ms | -- | Logic | runs at derivation on every child edit -- **none exists**, and nothing calls the solver (`TOILE-FEAT-029`) |
| Toile reaction count | < 0.1 ms typical | -- | Logic | one scan of the reactions map on read, never stored -- **none exists** |
| Toile boolean combination, two typical paths | < 5 ms | -- | Logic | user-initiated and one-shot, off the render frame -- **none exists** |
| Toile SVG import, 5 000 nodes | < 500 ms | -- | Logic | one-shot user action, off the render thread, applied in slices between the thread's messages (`TOILE-IO-028`) -- `crates/ambre-toile/benches/import.rs` measures the whole import and one slice, desktop only: about 280 ms whole and 1.7 ms a slice. The slice is what a camera move waits behind; an edit waits for the whole |
| Toile PPTX export, 50 slides | < 500 ms | -- | Logic, then the converter module | the board walk, then the writer. The writer alone took 18 ms for 50 slides of 26 items and one 256 px picture, on 2026-11-09, on the x86_64 host of the session, release build, through a timing test kept out of the tree -- **none exists** in CI (`3-Toile/features/ambre-spec-toile-pptx-export.md`) |
| Toile PDF export, 50 slides | < 3 s | -- | Logic | acceptable for an explicit export, off the render frame. It is not off the Logic command loop, so it is also how long the document stops accepting edits (`3-Toile/features/ambre-spec-toile-pdf-export.md`, `TOILE-IO-099`) -- **none exists** |
| Coverage levels across a 30-degree vector edge | >= 8 of 256 | -- | UI | the antialiasing quality figure, measured rather than eyeballed: rendered headless, the alpha histogram along the edge is the mechanism, and a pipeline with no coverage term yields exactly 2 (`CANVAS-061`, `ambre-spec-drawing-surface.md`) -- **none exists** |
| Antialiasing band width, at any zoom | 1.0 physical px +/- 0.25 | -- | UI | measured at both ends of the zoom range on one shape; a band tessellated in model units and not divided by the transform scale fails it as the board zooms in (`CANVAS-063`) -- **none exists** |
| Toile first paint of a picture | < 15 ms | -- | UI | the display-latency budget: the placeholder paints and the network never blocks it -- **none exists** |
| Toile media cache hit on a revisit | 0 refetch | -- | UI | the bounded local cache (`features/ambre-spec-toile-media.md section 3`) -- **none exists** |
| Prose local text insert, transaction plus exported delta | < 2 ms | -- | Logic | **none exists** for the insert itself. Its whole undo and its whole redo, one key on a document of 300 paragraphs of 500 characters, are `key_undo` and `key_redo` in `crates/ambre-prose/benches/instructions.rs`, each counted in a function named after it, against a ceiling of 1 600 000. Measured by callgrind on 2026-11-08, native release build in a cloud container, through the one history: 4 015 618 and 3 991 777 instructions while the replay walked every root block, 95 % of it. With that walk skipped for a replay of text alone: 204 638 and 216 502, about 0.26 ms. Under `just instructions` on 2026-11-09: 196 090 and 209 364 (`4-Prose/features/ambre-spec-prose-undo.md`, `PROSE-135`) |
| Prose pagination, full recompute of 100 pages | < 20 ms | -- | Logic | placement over already shaped runs, which is why the shaping rows below are charged apart (`features/ambre-spec-prose-pagination.md`) -- **none exists**, and no code either |
| Prose pagination, full recompute of 300 pages | < 60 ms | -- | Logic | a novel, the document Prose must hold, not an extreme of it. The 100-page row times three, held apart so that a cost that grows faster than the page count shows -- **none exists** |
| Prose pagination, partial reflow of 10 pages or fewer | < 3 ms | -- | Logic | the early exit converges inside the frame; an edit whose reflow does not converge is charged to the row above -- **none exists**, and no code either |
| Prose cold shape, 10 dirty blocks | < 5 ms | -- | Logic | a glyph-level pass, an order of magnitude over placement -- **none exists**, and no code either |
| Prose cold shape at document open, about 300 blocks | < 150 ms | -- | Logic | one-shot, off the interactive path -- **none exists**, and no code either |
| Prose page-layout handoff, 100 pages | < 1 ms | -- | Logic -> UI | a typed value by move on an `ambre_bus::conduit` queue, no serialization -- **none exists**, and no code either |
| Prose `export_snapshot()`, 100 pages | < 10 ms | -- | Logic | **none exists**, and no code either |
| Prose table-of-contents build, 50 headings | < 1 ms | -- | Logic | **none exists**, and no code either |
| Prose list ordinals, 100 items | < 500 us | -- | Logic | derived at read time, never stored (`PROSE-FEAT-002`) -- **none exists**, and no code either |
| Prose heading numbers, 50 headings | < 500 us | -- | Logic | **none exists**, and no code either |
| Prose track-changes command intercept | < 100 us | -- | Logic | on the path of every mutation while suggesting is on -- **none exists**, and no code either |
| Prose accept all, 100 suggestions | < 5 ms | -- | Logic | **none exists**, and no code either |
| Prose find, 100 pages, 10-character query | < 20 ms | -- | Logic | **none exists**, and no code either |
| Prose replace all, 50 occurrences | < 30 ms | -- | Logic | **none exists**, and no code either |
| Prose word count, 100 pages | < 5 ms | -- | Logic | debounced off the keystroke path -- **none exists**, and no code either |
| Prose spell check, 200 words | < 10 ms | -- | Logic | post-V1, and the dictionary is lazily loaded -- **none exists**, and no code either |
| Prose DOCX import / export, 100 pages | < 500 ms / < 300 ms | -- | Logic | sequential ZIP and XML work, one-shot and off the render frame. `crates/ambre-docx/benches/codec.rs` measures the codec alone, desktop only: on 2026-11-09, `cargo bench -p ambre-docx --bench codec` in a cloud container (Xeon at 2.1 GHz, host release build) reads 100 pages of 500 words, each with a table, a comment and a tracked change, in 7.9 ms and writes them in 4.5 ms. A novel of 300 pages reads in 3.7 ms, and a real 12 MB Word body in 270 to 550 ms. The module boundary and the mapping into the document are built and not measured, and **no mobile figure exists** |
| Prose DOCX import / export, 300 pages | < 1.5 s / < 1 s | -- | Logic | a novel. Whole, so it is also how long the Logic thread stops serving edits until the conversion runs in slices (`PROSE-IO-048`; the `Large-file import` Story (`ambre-roadmap.md` E6)) -- **none exists** |
| Prose Markdown export, 100 pages | < 50 ms | -- | Logic | no layout required -- **none exists**, and no code either |
| Prose PDF export, 100 pages, without / with images | < 2 s / < 5 s | -- | Logic | shaping per page through the shared surface; the second figure adds the blob fetches -- **none exists**, and no code either |
| Prose HTML clipboard import, 500 KB | < 100 ms | -- | Logic | on the paste path, so it is the one interop leg a user waits on -- `crates/ambre-prose/benches/html_paste.rs` measures the whole paste, desktop only; Google Docs, Word and the deepest nesting hold it, **a dense web page does not** |
| Prose plaintext `.ambre` export, 100 pages | < 10 ms | -- | Logic | **none exists**, and no code either |
| Prose document to intermediate form, 100 pages | < 50 ms | -- | Logic | one tree walk, shared by all four export formats -- **none exists**, and no code either |
| Prose scroll of one page | < 16 ms | -- | UI | culling plus the rebuild of the pages revealed (`features/ambre-spec-prose-viewport.md`) -- **none exists**, and no code either |
| Prose slash menu opened | < 8 ms | -- | UI | **none exists**, and no code either |
| Prose contextual toolbar shown | < 4 ms | -- | UI | **none exists**, and no code either |
| Keystroke to a local draft echo | < 2 ms | -- | UI | the editor echoes locally with no round trip: Trame's inline cell editor, Prose's draft buffer -- the `ambre:echo` probe on the page's timeline, read by `scripts/keystroke_probes.mjs` over single keys, a burst and a 10 KB paste; `?probes` prints its median and 95th percentile. Read by hand on a desktop only, and never on the slow devices the target names; Trame's editor is not measured |
| Keystroke to CRDT confirmation | 2 frames | -- | UI <-> Logic | counted in animation frames, not milliseconds: the first frame after the key paints the echo, and the answer is applied by the next one. The relay reads answers once a frame, so a time would measure the refresh rate, and reading sooner would move no pixel. The `ambre:confirm-frames` mark on the page's timeline, read by `scripts/keystroke_probes.mjs` over single keys, a burst and a 10 KB paste; `?probes` prints its median and 95th percentile. Read by hand on a desktop only, and never on the slow devices the target names |
| Logic pass that answers a keystroke | < 4 ms | -- | Logic | from the pass taking the command to its answer leaving, the part of the confirmation the Logic thread holds: under a frame, with room for the relay. The `ambre:logic-pass` measure on the worker's timeline, read from a Chrome trace by `scripts/keystroke_probes.mjs` over single keys. Read by hand on a desktop only, and never on the slow devices the target names |
| Keydown to a moved caret, end to end | < 50 ms | < 100 ms | UI <-> Logic | from the input handler to the end of the first frame that shows it -- the `ambre:caret` probe on the page's timeline, read by `scripts/keystroke_probes.mjs` over single keys, a burst and a 10 KB paste; `?probes` prints its median and 95th percentile. Read by hand on a desktop only, and never on the slow devices the target names |
| `ScrollViewport` send | < 1 ms | -- | UI | non-blocking -- **none exists** |
| Receive and render `TrameEvent::ViewportData` | < 5 ms | -- | UI | typed value by move, plus the vertex batch build -- **none exists** |
| Autocompletion, computation plus dropdown | < 2 ms | -- | UI | static lookup, no round trip -- **none exists** |
| Macro run started by a gesture (wall clock, network waits included) | -- | 30 s | Macro sandbox | the worker is terminated at the ceiling -- **none exists** |
| Macro run fired by an edit trigger (wall clock) | -- | 2 s | Macro sandbox | the worker is terminated at the ceiling -- **none exists** |
| Macro run fork, 10k-cell workbook | < 50 ms | -- | Logic | criterion -- **none exists** |
| Call chunk, one seal or one open in the transform | < 5 ms (P99) | -- | Call worker | **none exists**. The seal and the open run on the host alone, in `crates/ambre-crypto/src/call.rs`, and no call worker exists |
| Call worker load: three VP8 sizes and Opus sent, nine videos and the loudest audio received, about 1 500 chunks per second | < 25 % of one core | -- | Call worker | measured while the Sync worker seals a file -- **none exists**, and no code either |
| Call sender key switch, from a leave or a removal to the fresh key in use | 5 s | 12 s | Sync -> Call worker | the delay lets late wraps arrive, and a member who left opens chunks until it ends (`CALL-027`). The key model switches at the target, in simulated time: `ambre_sync::call::tests::a_leave_switches_each_remaining_sender_within_the_delay`. **None exists** for the handoff to a call worker, which does not exist |
| Call participants | 50 | 50 | -- | a cap: a join beyond it is refused with a notice (`CALL-021`), held by `ambre_server::ws::handler::call::tests::a_join_at_the_participant_cap_is_refused_with_a_notice`. The SFU refuses one too. A broadcast to hundreds is another product |
| Call videos forwarded to one receiver | 9: one at the top layer its estimate allows, eight at the lowest | 9 | SFU | a cap: the SFU forwards the video of the most recent speakers alone (`CALL-041`). `ambre_sfu::sim::tests::a_receiver_gets_the_video_of_the_most_recent_speakers_alone` holds it. Each receiver holds 9 video slots (`CALL-045`) |
| Call audio forwarded to one receiver | 3: the most recent speakers | 3 | SFU | a cap: the SFU forwards the audio of the most recent speakers alone, over 3 audio slots (`CALL-047`). `ambre_sfu::sim::tests::a_receiver_gets_the_audio_of_the_three_most_recent_speakers_alone` holds it |
| Call video stall, per 10 minutes of one received video: 50 participants at 150 ms of round trip and 2 % loss | < 1 s | -- | Browser | a stall is a gap between two pictures longer than three times the mean gap and longer than the mean plus 150 ms, as the WebRTC statistics count a freeze; a gap of 5 s or more is a pause. Measured on 2026-11-08 by `LOAD_SFU_CPUS=0,1 LOAD_SECONDS=600 taskset -c 2,3 cargo test -p ambre-sfu --release -- --ignored load_of_one_call`, on a cloud container of 4 vCPU, Intel Xeon 2.10 GHz, the SFU pinned to 2 of them and the 50 synthetic participants and their impairment relay to the other 2: 47.2 s per 10 minutes over 441 received videos and 600 s, the worst video 82.0 s. **It fails the target.** The synthetic receiver models the playout of a browser: a picture shows at its capture time plus the least transit plus the larger of one round trip and the 99th percentile of its lateness over 10 s. No browser ran in the load: the 2 vCPU beside the SFU held the participants at 1.36 vCPU. Later on 2026-11-08, with the probe of the next layer step and the 1 ms clock of the simulation, by the same command on the same machine: 11.2 s per 10 minutes, the worst video 20.4 s. **It still fails the target** |
| Call first media through TURN on TCP 443, from the join to the first decoded picture | < 5 s | -- | Browser | a network that opens TCP on port 443 alone (`CALL-042`), held by `ambre_belt::call::tests::a_browser_on_tcp_443_alone_decodes_the_call_through_turn`. Measured on 2026-11-08 by `just test-browser`, coturn over TCP behind a front at 150 ms of round trip, loopback, no loss: Chrome for Testing 157, first audio 2.1 s and first picture 2.4 s; Firefox ESR 153, 0.9 s and 1.3 s |
| Call mouth to ear, two phones on 4G | < 300 ms | < 400 ms through TURN | Browser | the inter-user budget of the README, applied to sound. Measured on 2026-11-08. Through TURN, by the TURN test above under Chrome for Testing 157: a tone onset sent to its onset heard, p95 166 ms to the relayed page and 182 ms from it. Under load, by `LOAD_SFU_CPUS=0,1 LOAD_SECONDS=600 taskset -c 2,3 cargo test -p ambre-sfu --release -- --ignored load_of_one_call`: the transit of each speech packet, p95 170 ms and p99 190 ms at 150 ms of round trip and 2 % loss. That transit leaves out capture, encode, jitter buffer and decode, which the TURN test measures at about 80 ms. No phone and no 4G network was measured |

* **The transport is not the cost.** UI <-> Logic stays zero-cost (`ambre_bus::conduit` by move,
  shared heap). The Logic <-> Sync `postMessage` hop adds a postcard encode plus a structured clone,
  or a zero-copy `ArrayBuffer` transfer for a large payload. The switchboard's cadence bounds
  it, well under the < 300 ms inter-user budget, because network latency dominates that path. The
  crypto bottleneck is Ed25519, 80 to 250 us.
* **p99 over peak.** A stable 45 FPS reads as smoother than 60 FPS with stutters, so the target is
  regularity. A guaranteed 60 FPS on every mobile device is not a goal.
* **The `.ambre` container has no budget, and that is a result.** It adds a 22-byte write and a
  22-byte parse. An export is `export_snapshot()` plus a header; an import is a header parse plus
  `import_snapshot`. Both are legs 2 to 4 of section 4. Its binary contribution is
  negligible for the same reason -- a few dozen bytes in `ambre-core`, shared rather than duplicated
  per app (`ambre-spec-docfile.md section 3`).

---

## 2. THE FRAME (16.6 MS AT 60 FPS)

| Phase | Budget | Thread | Notes |
|:---|:---|:---|:---|
| Keypress -> `beforeinput` -> dispatch | ~0.1 ms | UI | Negligible |
| `AppView::update` (drain the `UiEvent` queue) | < 0.1 ms | UI | Inbound IPC, dirty flag mutations |
| Content hit-test and input dispatch (in view) | < 0.2 ms | UI | `AppView::handle_input`, pointer and keyboard on the canvas area |
| View state update | < 0.5 ms | UI | Internal state mutations, transition logic |
| `LoroText::insert` | < 0.1 ms | Logic | Sequential insert, off the render frame |
| Text layout (Parley, dirty paragraph) | < 2 ms | UI or Logic | UI-thread shaping for the in-cell editor case (Trame cells, multi-line), typical < 0.5 ms. **Prose shapes on the UI thread the paragraphs an edit in flight changes, and no more**: a window of lines around the edit, until the layout that answers carries it, and the input method's preview (`features/ambre-spec-prose-text-editing.md section 3`). Pagination shapes in the Logic thread and ships pre-shaped `ParleyRunData`, so this row is a paragraph or two per edit on the UI thread for Prose. A key whose line breaks never fall back on the old ones shapes the rest of its paragraph, and the budget holds that worst case up to 500 characters, a paragraph of prose. A longer one costs what it costs, and nothing is degraded to save it (`PROSE-UI-215`). The CI `instructions` job holds it through `paragraph_shape` in `crates/ambre-prose/benches/instructions.rs` and 0.5-2 ms on the Logic thread (`features/ambre-spec-prose-pagination.md`, `features/ambre-spec-prose-page-rendering.md`) |
| Content accessibility diff (AccessKit) | < 0.2 ms | UI | OS notifications on the delta only |
| Glyph rasterization (Swash) on a new glyph | ~0 ms nominal | UI | Atlas hit |
| Build the vertex batch (dirty only) | < 1 ms | UI | `QuadRenderer::begin_frame` plus the scene draw. Zero allocation in steady state |
| GPU upload and render pass | < 1 ms | UI | `write_buffer` plus up to 3 draw calls per layer (quads, mask glyphs, color glyphs). Typical 0.3-0.5 ms |
| Antialiasing band, share of a tessellated path | < 15 % of its vertices, < 5 % of the batch build | UI | The silhouette band of `ambre-spec-drawing-surface.md section 2` (`CANVAS-061`). It adds vertices to a buffer and a draw call that already exist, so it is charged against the two rows above and against nothing else: no pass, no dispatch, no attachment (`CANVAS-062`) -- **none exists** |
| Toile board paint, 2 000 visible elements | < 16 ms | UI | The whole frame for a busy board, and the reason the cull is the engine's job (`features/ambre-spec-toile-board-rendering.md`). The CI `instructions` job holds its content walk through `board_paint` in `crates/ambre-toile/benches/instructions.rs`, at most 5 ms of the 16: about 3 million instructions against a ceiling of 4 million (`CANVAS-072`). The tessellation, the upload and the GPU fall outside the count (`CANVAS-005`) |
| Toile pan and zoom, input to paint | < 15 ms | UI | The camera is applied locally, so the gesture never waits on the Logic thread (`features/ambre-spec-toile-viewport.md`) |
| Toile presence overlay, 20 cursors and their trails | < 1 ms | UI | Instanced quads and small ring buffers (`features/ambre-spec-toile-presence.md`) |
| Prose display list, 3 visible pages | < 4 ms | UI | Walking the placed blocks and assembling already shaped runs, with the highlights costing what is visible (`features/ambre-spec-prose-page-rendering.md`) |
| Prose page paint, 3 visible pages | < 14 ms | UI | The display list above, the vertex batch and upload (< 4 ms) and the submit (< 6 ms). A full page repaint is the busy case, as the board paint is for Toile |
| **Total target** | **< 5 ms typical** | UI | 11 ms of margin for a complex case |
| `TextCache`, 100 distinct layouts, cold cache | < 1 ms | UI | Not a frame phase: the shape budget of the cache itself, off the critical path once warm |

* **The scroll round-trip holds because the frame is a fifth of it.** Section 1 budgets the P1 scroll
  round-trip at < 15 ms. Rendering consumes < 5 ms. Queue transport is negligible: < 1 us per
  hop, no serialization, no boundary memcpy. Most of the budget is left to the Logic thread.
* **Threats to the frame, and the countermeasure each takes.**
  1. **Parley has no incremental layout** -> atomicity at the **paragraph**, one `Layout` per
     paragraph. For Prose this shaping runs in the Logic thread, and a monster paragraph is split
     internally and deferred through the P1-P4 priority scheduler
     (`4-Prose/features/ambre-spec-prose-pagination.md`). That scheduler is **not**
     `requestIdleCallback`, which does not exist in the `wasm_thread` Logic worker. It stays
     available for opportunistic
     UI-thread work.
  2. **Glyph atlas overflow** -> an LRU per 256^2 / 512^2 slot, one cache per size. UI thread.
  3. **Scrolling a long document** -> per-paragraph virtualization. Render the visible range plus a
     buffer, cache the pre-shaped runs in the UI `RunCache`, and cull by page for Prose
     (`features/ambre-spec-prose-viewport.md`).
  4. **A multi-paragraph selection** -> reuse the cached `Layout` to extract the rectangles.
  5. **A mid-range mobile device** -> wgpu quads hold 60 FPS; the risk is CPU (Parley, Loro and
     dispatch share 6 to 8 usable ms).

---

## 3. THE SERVER, PER OPERATION

| Operation | Target | Potential bottleneck | Method |
|---|---|---|---|
| WS `Update` receive and broadcast | < 5 ms p99 | Redis Lua plus PubSub | **none exists** |
| Redis Lua clock validation | < 1 ms | Redis round trip | **none exists** |
| `GET` deltas, 100 entries | < 20 ms | Postgres read | **none exists** |
| S3 presigned URL generation | < 50 ms | S3 SDK network I/O | **none exists** |
| Document cold start (snapshot plus N deltas) | < 500 ms | S3 plus Postgres read | **none exists** |
| OIDC auth callback | < 200 ms | Provider token exchange | **none exists** |
| Global S3 flush batch, 15 s cycle | < 2 s for 1000 deltas | Parallel S3 upload | **none exists** |
| Presence write, per heartbeat | < 2 ms | Two Redis round trips: the entry, then its own expiry | **none exists** |
| SFU load, one call of 50 participants | < 2 vCPU | packet forwarding and the layer choice per receiver | measured on 2026-11-08 by `LOAD_SFU_CPUS=0,1 LOAD_SECONDS=600 taskset -c 2,3 cargo test -p ambre-sfu --release -- --ignored load_of_one_call`, on a cloud container of 4 vCPU, Intel Xeon 2.10 GHz, the SFU pinned to 2 of them and the 50 synthetic participants and their impairment relay to the other 2, at 150 ms of round trip and 2 % loss: 1.51 vCPU over 600 s. Earlier the same day, before the SFU spread its connections over threads, one thread saturated at 0.99 vCPU and the transit reached 7.5 s at p99. The pacer of str0m costs the most: it reads every send stream of a connection at each packet it sends. Later on 2026-11-08, with the congestion control of Ambre's own and 12 slots per receiver, on a cloud container of 4 vCPU, Intel Xeon 2.30 GHz, by the same command: 0.87 vCPU over 600 s, and a speech transit of 161 ms at p95. Later the same day, with the layer choice run at once on a lower target: 0.89 vCPU, and 161 ms at p95. Later on 2026-11-08, with the probe of the next layer step, by the same command on a cloud container of 4 vCPU, Intel Xeon 2.10 GHz: 1.04 vCPU over 600 s, and 162 ms at p95 |
| SFU layer drop, from a cap on the downlink of a receiver to a lower layer at that receiver | < 3 s | the estimate of the downlink, from the transport-wide feedback of the receiver | `ambre_sfu::sim::tests::a_capped_receiver_drops_a_layer_within_the_budget` measures it in simulated time, at 150 ms of round trip and 2 % loss, and no CI job runs it (`CALL-038`). On 2026-11-08, alone on the machine: 20 runs from 0.87 s to 2.33 s. Beside three other runs: 4 of 21 above 3 s, the slowest 4.0 s. **It fails the target under load.** In the load run of 50, no receiver held a layer above the lowest under 2 % loss, so the cap found no layer to drop there. Later on 2026-11-08, with the congestion control of Ambre's own and a congested drop that stops the current layer, the run is seeded and runs in the CI native tier over eight seeds. By `cargo test -p ambre-sfu --release a_capped_receiver -- --nocapture`, on a cloud container of 4 vCPU, Intel Xeon 2.30 GHz: from 0.82 s to 1.13 s. The same seeds on the estimate of str0m: from 0.83 s to 3.19 s. In the load run of 50 above, the capped receiver dropped after 1.58 s. Later on 2026-11-08, the layer choice runs at once on a lower target, and the SFU pads nothing under overuse. On the same machine, by the same two commands: from 0.68 s to 0.85 s over the eight seeds, and 0.87 s in the load run of 50. Later on 2026-11-08, with the probe of the next layer step and a simulated clock of 1 ms, on a cloud container of 4 vCPU, Intel Xeon 2.10 GHz, by the same two commands: from 0.72 s to 0.78 s over the eight seeds, and 0.88 s in the load run of 50 |
| SFU layer recovery, from a delay step with no lasting congestion to the top layer at that receiver | < 2 s | the end of the overuse, one probe of the next layer step, then the keyframe of the top layer | `ambre_sfu::sim::tests::a_receiver_climbs_back_to_the_top_layer_after_a_delay_step` measures it in simulated time, at 150 ms of round trip and 2 % loss, with a step of 40 ms on the one-way delay of the downlink, over eight seeds in the CI native tier (`CALL-050`). On 2026-11-08, by `cargo test -p ambre-sfu --release a_receiver_climbs_back -- --nocapture`, on a cloud container of 4 vCPU, Intel Xeon 2.10 GHz: from 1.12 s to 1.29 s. Of that time, the overuse of the step lasts 0.45 s to 0.53 s, the probe takes 0.35 s to 0.40 s, and the keyframe of the top layer arrives 0.25 s to 0.49 s later. Before the probe, the same seeds held the middle layer for over 7 s |

---

## 4. OPENING A LARGE DOCUMENT -- A COMPOSITE OPERATION WITH NO END-TO-END TARGET

* **Every leg is budgeted, the whole is not.** Each row of section 1 budgets one leg; none budgets
  the chain, and two legs are unbudgeted entirely.

| Leg of an open | Stated budget |
|---|---|
| 1. Server read (snapshot plus N deltas from S3 and Postgres) | < 500 ms, section 3 |
| 2. Client AEAD open plus zstd decompress of a full snapshot | **none** -- section 1 budgets compression of 100 KB and inbound decryption per 1 KB |
| 3. `LoroDoc` import of a full snapshot | **none** -- `doc.import` in section 1 is < 2 ms for a **1 KB delta**, which says nothing about a 10 MB snapshot. A full-state read imports it **twice**: once into a scratch document to read the schema register, once into the live one (`ambre-spec-schema-version.md section 2`). Deliberate, and it is what makes a refused document cost the user nothing |
| 4. Engine rebuild after import | partial -- `SheetIndex` rebuild of 10k rows is < 10 ms in section 1. Trame's `import_snapshot` triggers the **full reseed**, which re-parses and recompiles every formula of every sheet (`ambre-spec-trame-engine.md`): unbudgeted, and likely dominant at 100k cells. Toile's `SpatialIndex` **bulk load** is unbudgeted too |
| 5. First painted frame | the frame budget of section 2, GPU-bound |

> **This section sets the missing figure, once it can be measured.** Legs 2 to 5 are pure functions
> of a snapshot, so a large document makes them measurable on host, with no network and no browser.
> Leg 1 needs the db tier. `ambre-spec-fixtures.md` (`FIXTURES P2`) builds the
> corpus and runs those benches. Inventing an "open in < X ms" target before then would produce a
> number with no basis. This is recorded as a **known gap with an owner**, not as a budget.

---

## 5. BINARY SIZE

* **Two binaries.** `ambre-app` carries UI plus Logic: wgpu, loro, leptos, the heavy one.
  `ambre-sync` carries crypto plus protocol plus gloo-net, small at ~6.8 MB unoptimized, with no
  wgpu, no loro and no leptos. Only crypto, protocol and serde are duplicated across the two; the heavy
  dependencies live once, in `ambre-app`. The two wasm fetches are parallelizable. The converter
  modules, one per format family, are each fetched on the first conversion of their family alone.
* **The first download, Main plus Sync, < 4 MB brotli**, and every format converter stays
  **lazy-loaded**, in the converter module of its family. One wasm carries the whole suite: an application is
  never a download of its own, so opening a second one costs nothing. 4G commonly runs at
  20 Mbit/s, where 4 MB takes 1.6 s. That leaves the rest of the README's 3 s initial load to
  compile and boot.
* **Counted in brotli, as served.** The release build writes a brotli copy at quality 11 beside
  each compressible file (`scripts/trunk_post_build.py`). A static server sends it as it is, with
  no compression per request. On the wasm it weighs 27% less than gzip at level 9.
* **Cached for good after the first visit.** Every built file is named by a hash of its content
  (`filehash` in `Trunk.toml`), so a deployment caches it as immutable. Only the page is asked
  again, and a release downloads only the instance it changed.
* **The first download holds the budget, and the ceilings stay tight.** On 2026-10-20 a release
  build weighed 3.07 MB brotli for Main and 0.34 MB for Sync. On 2026-11-07, `just size` read
  3 128 174 bytes for Main and 352 134 for Sync. The `wasm-release` profile builds every crate for
  size and strips the name section, which took Main from 6.9 MB to 4.27 MB gzip. The `size` CI job
  holds each figure as a ceiling (`scripts/wasm_size.py`) and fails past the budget. A ceiling
  sits under the budget: it catches any growth long before the budget does.
* **Both ceilings went up on 2026-11-07**, to 3.13 MB for Main and 0.36 MB for Sync. No dead code
  grew, so no cut was found. The table below is `twiggy top` on a build that keeps the name
  section, from the 2026-10-20 ceiling commit `a70872374` to that day. A crate's raw growth is
  net of the code that moved out of `ambre-bus`.

| Instance | Crate that grew | Raw growth | Why |
|---|---|---|---|
| Main | `ambre-trame`, `ambre-toile`, `ambre-murmure`, `ambre-app`, `ambre-logic`, `ambre-chrome`, `ambre-prose` | +148 KB | The message vocabulary moved in from `ambre-bus`, and new features: the switchboard, the outbox, the line uids of Trame schema 2 |
| Main | `reactive_graph`, `tachys`, `leptos` | +59 KB | The new chrome views of those features |
| Main | `ambre-core` and `hashbrown` | +51 KB | The generic newtype families (`ambre-spec-newtypes.md`) build one copy of each function per marker, a hash table included |
| Main | `blake2` | +7 KB | The line uid of Trame schema 2 is a BLAKE2b hash, so the hash now links into Main |
| Sync | `ambre-core` | +19 KB | The generic newtype families, one copy per marker |
| Sync | `ambre-sync`, `serde`, `percent-encoding` | +11 KB | The percent-encoded query (`SYNC-179`) and a flattened token pair |
* **Main sat over its ceiling on 2026-11-08.** That day, `just size` read 3 136 988 bytes for
  Main and 354 035 for Sync. The features merged since 2026-11-07 grew Main by 8 814 bytes. One
  cut took 4 461 back: `ambre-toile` now names `roxmltree` 0.21, the version usvg 0.47 links,
  so the instance carries one copy of the parser. Main then read 3 132 527, still 2 527 over.
* **The ceiling of Main went up to 3.14 MB on 2026-11-08.** The growth is the features
  themselves: the HTML half of the clipboard write (`UI-THREAD-055`) and the formula bind of
  Trame (`TRAME-289`).
* **The ceiling of Main came down to 3.10 MB on 2026-11-08.** That day, `just size` read
  3 134 561 bytes for Main before the cut and 3 098 641 after it: 35 920 bytes less. Sync did not
  move, at 354 020.
* **The cut aligns parley on the font readers of krilla.** krilla 0.8.2 is the newest release, and
  it and subsetter 0.2.6 pin skrifa 0.42 and read-fonts 0.39. Parley went back from 0.12 to 0.10,
  the last release on the same pair, so Main links one copy of each reader. The 0.10 line breaker
  already sets each line's x offset and width, which `MURMURE-570` needs. Two bans of `deny.toml`
  keep a newer pair out of the lock file.
* **The cut costs one change of layout.** Parley 0.10 breaks a CR LF twice, so a CR LF that
  reaches the canvas draws one empty line more (`CANVAS-084`). Prose, Trame and the browser
  input turn a CR LF into an LF first, so only a peer or an imported file can bring one.
* **The ceiling of Main went up to 3.11 MB later on 2026-11-08.** Trame's own undo history took
  Main to 3 102 694 bytes, 2 694 over the ceiling of the cut. Each edit now records its inverse
  (`TRAME-286`). The growth is the feature itself, so the ceiling keeps about 7 KB of
  room above it instead of a few hundred bytes.
* **The converter module left Main on 2026-11-08.** The PDF writer and the CSV reader and writer
  moved into a third artefact. The Sync worker fetches it on the first import or export
  (`ambre-spec-architecture.md section 2`, the converter module). That day, `trunk build --release`
  then `scripts/wasm_size.py`, on the session container, read the figures below. The ceiling of
  Main came down to 2.96 MB.

| Artefact | Before | After | Change | Why |
|---|---|---|---|---|
| Main | 3 102 694 | 2 951 673 | -151 021 | `krilla`, its subsetter and the CSV crate left; the draw list recorder, the request types and the loader came in |
| Sync | 354 020 | 355 125 | +1 105 | the fetch of the module and its digest check |
| Converter module | -- | 419 304 | new | `krilla`, `csv`, `postcard`, `std` and the workspace types they read, built single-threaded |

* **The converter module, < 420 KB brotli, outside the first download.** A user fetches it on the
  first import or export, then the browser caches it for good under its digest. `wasm_size.py`
  holds its ceiling apart and never adds it to the first download. On 4G at 20 Mbit/s, 420 KB
  takes 0.17 s: the first export waits that long once. The split below replaced this row on
  2026-11-09.
* **The ceiling of Main came down to 2.72 MB on 2026-11-09**, with the split below.
* **The converters split by format family on 2026-11-09.** Each family is its own module, fetched
  on the first conversion of that family and checked against its own digest
  (`ambre-spec-architecture.md section 2`, the converter modules). Prose's Markdown and HTML
  readers and Toile's SVG reader left Main for their family. That day, `trunk build --release`
  then `scripts/wasm_size.py`, on the session container, read the figures below. The Main figure
  before is the same build on the parent commit.

| Artefact | Before | After | Change | Why |
|---|---|---|---|---|
| Main | 2 949 129 | 2 712 212 | -236 917 | `pulldown-cmark`, `html5ever`, `usvg` and `roxmltree` left; the trees the modules answer came in |
| Sync | 355 038 | 355 660 | +622 | one digest and one module name per family |
| First download | 3 304 167 | 3 067 872 | -236 295 | Main plus Sync |
| Converter module | 419 468 | -- | gone | split into the four modules below |
| Tables module | -- | 43 318 | new | `csv` |
| PDF module | -- | 412 507 | new | `krilla` and the draw list it replays |
| Markup module | -- | 204 276 | new | `pulldown-cmark` and `html5ever` |
| Images module | -- | 180 880 | new | `usvg` and `roxmltree` |

* **WebP joined the images module on 2026-11-09.** Toile reads a WebP picture through the module,
  dropped or inside an SVG file. It writes the pixels as PNG with the encoder Main already links.
  Main's own `image` line keeps PNG and JPEG alone. `scripts/converter_build.py` then the brotli of
  each module, on the session container, read the figures below.

| Images module | Brotli | Change | Why |
|---|---|---|---|
| Without WebP | 180 880 | -- | `usvg` and `roxmltree` |
| With `image` and its `webp` feature, answering PNG | 318 205 | +137 325 | the `image` decoding frame, its colour management and the PNG encoder came with the decoder: past the 300 KB budget |
| With `image-webp` alone, answering pixels | 220 913 | +40 033 | the same pure-Rust decoder the `webp` feature wraps, called directly. Kept |

* **The OOXML module joined on 2026-11-10, and Main kept its 2.72 MB ceiling.** The Word codec,
  the package layer `ambre-opc`, `zip` and `quick-xml` link into the module alone. The Word
  mapping runs there too, over Prose's intermediate form in the leaf crate `ambre-prose-form`.
  The form crosses as postcard bytes, so Main links its serde code and none of the mapping.
* **Main made room by sending Prose's other mappings to the markup module.** The Markdown reader
  and writer, with their mapping onto the form, left Main. So did the walk that reads the HTML
  half of a paste into the blocks a copy writes, with its style parser. Main keeps the import and
  export entries, the builder of the review a Word file carries, and the serde code of the form.
  `trunk build --release` then `scripts/wasm_size.py`, on the session container, read the figures
  below. The figure before is the same build of `origin/main`.

| Artefact | Before | After | Change | Why |
|---|---|---|---|---|
| Main | 2 716 043 | 2 714 723 | -1 320 | the Markdown and HTML-paste mappings left; the Word entries, the review builder and the form's serde code came in |
| Sync | 355 413 | 355 606 | +193 | one digest and one module name more |
| First download | 3 071 456 | 3 070 329 | -1 127 | Main plus Sync |
| Tables module | 43 565 | 44 066 | +501 | the new variants of the request and answer types |
| PDF module | 413 216 | 413 213 | -3 | the same |
| Markup module | 204 510 | 239 827 | +35 317 | the Markdown mapping, the HTML-paste walk, the form's serde code and the float parser the walk reads a style with |
| Images module | 220 949 | 221 511 | +562 | the new variants of the request and answer types |
| OOXML module | -- | 185 483 | new | `ambre-docx`, `ambre-opc`, `zip`, `quick-xml` and the Word mapping |

* **One budget per converter module.** A row below is the budget of one module: what its family
  may weigh with the formats the roadmap announces in it, never today's figure. `wasm_size.py`
  holds each module to its budget and to a tight ceiling at today's figure, and adds none to the
  first download. A ceiling rises up to its budget in the commit that grows the module, with the
  crate named here. On 4G at 20 Mbit/s, the largest budget, 900 KB, takes 0.36 s once.

| Module | Budget, brotli | Ceiling | Measured 2026-11-09 | Formats today | Formats the roadmap announces | Method |
|---|---|---|---|---|---|---|
| Tables | < 100 KB | 50 KB | 43 318 | CSV, read and write | TSV; xlsx and ODS have families of their own | `scripts/wasm_size.py`, run by `just size` |
| PDF | < 600 KB | 420 KB | 412 507 | PDF, write | a selectable text layer, the Prose and Trame exports through the same writer | `scripts/wasm_size.py`, run by `just size` |
| Markup | < 300 KB | 240 KB | 239 827, on 2026-11-10 | Markdown, read and write; HTML, read for a paste | the HTML import and export | `scripts/wasm_size.py`, run by `just size` |
| Images | < 300 KB | 230 KB | 220 913 | SVG and WebP, read | the picture codecs, which link into Main today (`image`, about 67 KB) | `scripts/wasm_size.py`, run by `just size` |
| OOXML | < 900 KB | 190 KB | 185 483, on 2026-11-10 | DOCX, read and write | xlsx and PPTX, read and write: one ZIP reader and one XML parser for three codecs | `scripts/wasm_size.py`, run by `just size` |
| OpenDocument | < 600 KB | -- | not built | -- | ODS read, then ODT and ODP | **none exists**: the OpenDocument story builds the module, and `wasm_size.py` holds it from that commit |

* **Where the weight sits.** The table below splits the Main instance by
  subsystem, from `twiggy top` on a build that keeps the name section, on 2026-11-08 before the
  cut. The brotli column is an estimate: each part takes the ratio of the whole, 0.228. The
  parts that only an application needs weigh about 0.84 MB of brotli. Splitting them out would
  not have reached the old 2 MB budget: the shared part alone weighs about 2.3 MB.

| Part | Raw | Brotli, estimated | What it is |
|---|---|---|---|
| Data segments, glue, `core`, `alloc` and the generic code no crate names | 3 068 KB | ~700 KB | Shared, and not split by the measure |
| GPU | 1 635 KB | ~373 KB | `naga`, `wgpu-core` and `wgpu-hal`, needed by the WebGL2 backend |
| CRDT | 1 585 KB | ~361 KB | `loro` and its crates |
| Shared Ambre crates | 1 471 KB | ~335 KB | `ambre-app`, `ambre-logic`, `ambre-core`, `ambre-bus`, `ambre-canvas`, `ambre-chrome` |
| UI | 1 170 KB | ~267 KB | `leptos`, `tachys`, `reactive_graph` |
| Text | 1 126 KB | ~257 KB | `parley`, `swash`, `skrifa`, `read-fonts`, `harfrust`, `zeno` |
| PDF writer | 968 KB | ~221 KB | `krilla` and the second copy of `skrifa` and `read-fonts` it links, older than `parley`'s. The 2026-11-08 cut removed that copy |
| Toile | 849 KB | ~194 KB | `ambre-toile`, `usvg`, `roxmltree`, `i_overlay` |
| Prose | 664 KB | ~151 KB | `ambre-prose`, `html5ever`, `pulldown-cmark` |
| Trame | 541 KB | ~123 KB | `ambre-trame`, `petgraph` |
| Murmure | 352 KB | ~80 KB | `ambre-murmure` |
| Picture codecs | 296 KB | ~67 KB | `image`, `png`, `zune-jpeg`, `gif` |

* **Four compile-time cuts were measured on 2026-11-08 and left out.** Each figure is the brotli
  size of Main against the same build without the cut.

| Cut | Main | Why it stays out |
|---|---|---|
| `-Z location-detail=none` in the `wasm-release` profile | -75 697, and -8 709 on Sync | The primitive gate refuses a `-Z` flag in a rustflags list (`ARCH-033`) |
| `-Z fmt-debug=shallow` | -51 777 | Every `Debug` output loses its fields, the logs included, and `ARCH-033` refuses it too |
| `panic_immediate_abort` in `build-std-features` | not built | The panic loses the message that the fatal screen shows |
| `log` and `tracing` call sites removed in a release | -1 590 | No logger reads them, so they cost nothing at run time, and the gain is too small for a new dependency |

* **wasm-opt stays off.** Binaryen 124 takes 7% off the raw Main wasm, and the result compresses
  0.8% worse, in gzip and in brotli alike. The download is the budget, so the pass costs.

| Contribution | Estimated raw | Notes |
|---|---|---|
| `usvg` plus `lyon` (build-time icons) | ~80 KB | Icon pipeline, precompiled vertex buffers |
| `swash` | ~120 KB | Glyph rasterization |
| Fonts (Noto Sans plus Mono) | 0 KB binary | ~600 KB fetched and HTTP-cached: Regular plus Bold, Latin, Cyrillic, Greek |
| Icon atlas blob (vertex buffers) | < 200 KB | ~500 tessellated icons |
| `image` (PNG, JPEG, WebP, GIF) | ~60 KB | Feature-flagged, only the activated decoders |
| `ambre-sync` logic | ~50 KB | Excluding its dependencies |
| RustCrypto (chacha20, ed25519, blake2, hmac, argon2) | ~140 KB | Shared with `ambre-crypto`; Argon2 is there for login, and for a passphrase-sealed file, which reuses the same work factor rather than adding a second one (`ambre-spec-docfile.md`) |
| `ruzstd` (compression and decompression) | ~80 KB | |
| `gloo-net` (websocket and http) | ~40 KB | |
| `postcard` | ~15 KB | Shared |
| **`ambre-sync` total** | **~325 KB raw** | Shared dependencies deduplicate in the final binary |

**The Main instance, per contribution.** Estimates, and why the lazy-loading rule above exists.

| Contribution | Estimated raw | Estimated gzip | Notes |
|---|---|---|---|
| `ambre-prose`, engine plus features plus interop plus UI | ~230 KB | ~72 KB | Of which the UI module is ~50 KB raw |
| `unicode-segmentation` | ~40 KB | ~15 KB | Word and grapheme boundaries: the word count, caret movement |
| `zspell` plus one dictionary | ~550 KB | ~210 KB | Post-V1, and lazily loaded on the first spell check |
| `ambre-docx`, with `zip` and `quick-xml` | ~427 KB | ~147 KB brotli | The Word codec, read and write, in a test `cdylib` at `opt-level = "z"` with LTO, measured on 2026-11-09. `docx-rs` measured 505 KB of brotli in the same frame. No instance links it: the OOXML module does since 2026-11-10, over the package layer `ambre-opc`, and Main links its model alone |
| `html5ever` plus `markup5ever_rcdom` | ~200 KB | ~60 KB | In the markup module since 2026-11-09, fetched on the first paste of HTML or the first Markdown import. `markup5ever_rcdom` never linked: the module builds its own arena |
| `pulldown-cmark` and its writer | ~80 KB | ~25 KB | The parser is in the markup module since 2026-11-09, and Prose's own writer since 2026-11-10 |
| `krilla`, billed to `ambre-pdf` | ~400 KB | ~120 KB | Counted once, in the converter module since 2026-11-08 and in the PDF module since 2026-11-09: the Main instance links none of it (`ARCH-078`) |
| `loro` | ~400 KB | ~120 KB | Shared by every application |
| `parley` | ~200 KB | ~60 KB | Shared |
| `wgpu` plus `lyon` plus `swash` | ~800 KB | ~250 KB | Shared, and the `swash` row above is its glyph half alone |

* **The heavy render crates live in the Main instance.** wgpu, lyon and swash link once there,
  deduplicated with the rest; the separate Sync instance links none of them.
* **Fixture machinery contributes zero** (`FIXTURE-007`): it is host-only and gated behind a
  non-default `fixtures` feature that `ambre-app` never names.

---

## 6. MEMORY

* **Two independent** `WebAssembly.Memory` heaps -- Main shared across UI and Logic, Sync its own --
  each capped at 4 GiB by the platform.

| Holder | Budget | Notes |
|:---|:---|:---|
| CPU vertex buffers (`QuadRenderer`) | < 5 MB | Proportional to the visible draw-call count |
| wgpu textures and buffers | < 20 MB | Swap chain, staging buffers, `GlyphAtlas` (2048x2048 R8 = 4 MB). The RGBA color atlas (2048x2048 = 16 MB) is **allocated lazily** on the first color glyph -- a 1x1 placeholder otherwise -- so an emoji-free document pays nothing |
| Uploaded pictures, mip chains included | < 24 MB | `IMAGE_VRAM_BUDGET`, charged apart from the row above rather than sharing an unstated remainder of it. At four bytes a texel that is roughly six 1024x1024 photographs, each carrying a chain a third its own size. Held by the eviction (`CANVAS-066`) and measured by nothing: the frame where the budget starts to thrash -- evicting a picture the next frame asks for again -- is the figure no bench reads (`ambre-spec-drawing-surface.md section 5`) |
| View state (content) | < 5 MB | Proportional to the visible content: cells, text runs |
| Fonts in memory | < 10 MB | Base fonts plus lazily loaded ones |
| Icon atlas | < 1 MB | Compiled vector blob |
| Murmure decoded pictures, per page | < 16 MiB | `PICTURES_MAX_SIZE` in `ambre-murmure`, shared by the channel's region and the thread's. Evicted least recently painted, never a picture on screen, which may hold it past the cap. One picture holds at most 8 MiB of frames (`PICTURE_MAX_SIZE` in `ambre-logic`), decoded at most 480x320: an animation past it plays the frames that fit (`features/ambre-spec-murmure-attachments.md`) |
| **Total UI thread** | **< 57 MB** | Within the global < 100 MB |
| `SyncState` base, per session | < 1 MB | `MasterSeed`, `SigningKey`, `ExchangeKey`, plus `HashMap` overhead |
| `SyncDocumentState`, per open document | < 1 MB | `DocumentKey`, counters, sync state, quarantine |
| `quarantine`, per document | ~2 MB max | Capped at 1000 updates of ~2 KB average |
| `old_keys`, per document | negligible | Capped at 10 `DocumentKey` (32 bytes plus overhead) |
| Unconfirmed updates, per document | 1024 updates | `UNCONFIRMED_LIMIT`: the sent deltas the relay has not echoed yet, kept to resend one it refuses for its key (`SYNC-129`) or its clock (`SYNC-147`). Past the cap the oldest is forgotten, and a refusal for it resends nothing |
| Sync events held on Main past a full channel | 4096 events, released at 1024 | `SYNC_INBOX_MARKS`, high and low at four times and once the channel capacity. At the high mark the Sync worker closes its socket, and the relay holds the rest (`SYNC-059`). The bound is a count, not bytes: a replay batch is one event |
| View events held on Main past a full view queue | 2048 events per view: 1024 in its queue, 1024 in its backlog | `CHANNEL_CAPACITY` and `VIEW_BACKLOG_LIMIT` in `ambre-app`. A view reads both once a frame. A hidden page relays on its timer, and its views read nothing until it shows (`CANVAS-075`). Past the bound the oldest event drops, and the document is resent once the view reads again (`ambre-spec-message-bus.md`, `BUS-039`, `BUS-041`). **Reachable.** Measured on 2026-11-07 by `cargo test -p ambre-logic view_load -- --nocapture`, host debug build in a cloud container. A remote delta sends its view 3 events in Trame, 2 in Toile and 3 in Prose, alone in its pass or in a burst. So a hidden page overflows after about 683 remote cell writes, 1024 remote elements or 683 remote words. A replay batch sends 3 events, a paste of 10 000 cells 2, a CSV import 5, and an SVG import of 5 000 nodes about 3 per slice. A shown page reads each of those within a frame |
| Edits held before the session identity, per document | 256 edits | `HELD_EDITS_LIMIT` in `ambre-logic`. The wait is one message from the Sync worker, so the cap is reached only by a session opened before its unlock. Past the cap a new edit is dropped and logged (`AUTHOR-026`). A cached document restored in that wait is held too, one copy of its bytes until the identity lands (`AUTHOR-027`) |
| Sync overhead, 1 host document plus 3 embedded | ~4-8 MB | The typical scenario |
| `LoroDoc` | < 10 MB target, per 100k cells | |
| `LoroDoc` Prose | < 10 MB, per 100 pages and 50 000 words | |
| Prose page layout, 100 pages | < 200 KB | About 300 blocks, 20 footnotes and the header and footer runs. Derived, so it is re-allocated whole on a full recompute |
| Prose shaped runs, Logic thread | ~150 KB per page, unbounded | One run per block, kept past measurement and shared with the view (`PROSE-036`). Measured at about 18 KB a paragraph of eighty words, eight to a page: 43 MB for 300 pages. Bounding it is the `Shaped runs bounded to the view` Story (`ambre-roadmap.md` E7) |
| Prose document of 300 pages, Logic thread | < 32 MB, to measure | A novel of 100 000 words: its `LoroDoc`, its page layout and the shaped runs it holds, within the documents-held-in-memory row below. Unbounded runs alone pass it. Bounding them is the `Shaped runs bounded to the view` Story (`ambre-roadmap.md` E7) |
| Prose run cache, 500 runs | < 1 MB | The drawing state of the runs painted lately, about 50 pages: an index into runs the Logic thread already holds (`features/ambre-spec-prose-page-rendering.md`) |
| Prose view state, excluding the run cache | < 1 MB | |
| OOXML package read, per conversion | 4096 parts; 32 MiB inflated per part; 64 MiB inflated in all | One set for DOCX, xlsx and PPTX, checked while the bytes inflate, never read off a header (`ambre_opc::MAX_PARTS`, `MAX_PART_SIZE`, `MAX_INFLATED`). The sum is about 67 MB, so the model a codec builds beside the parts stays inside the < 100 MB total. A Word body of 12 MB and a sheet of 100 000 cells read under the bound of one part. Held by one test per bound in `ambre-opc` |
| Prose decoded pictures, per session | < 64 MiB | A bounded cache in the Logic thread, evicted least recently painted; it holds the pictures of a few pages and stays well inside the < 100 MB total (`features/ambre-spec-prose-images.md`) |
| Trame hidden line elements, per sheet | < 1 000, about 51 KB | Only two peers writing at once into one new line create one: each such race leaves one hidden element per line it adds (`2-Trame/features/ambre-spec-trame-rows-and-columns.md section 3`). None is removed, since a reference may name it. **None exists** to count them |
| Prose vertex buffers and atlases, 3 pages | < 50 MB VRAM | Stated by the Prose design and larger than the shared wgpu row above, which is the arithmetic the two rows now make visible. Reconciled by the `The memory budgets add up` Story (`ambre-roadmap.md` E7) |
| Trame DAG plus cache | < 50 MB, per 100k formula cells | |
| Toile board plus index, 50 000 elements | proportional to the element count, inside the < 100 MB total | The R-tree adds a small multiple of its leaf count over the bounds map it mirrors |
| Server, per open document | ~50 KB | No in-process blob cache: `LIST doc:{id}:pending` buffers in Redis, ~50 deltas of ~1 KB over the 15 s window |
| Server, inline delta size | < 64 KB | A delta past it keeps an object of its own (`SRV-149`) |
| Server, inline deltas per document | 256 deltas or 256 KB, whichever comes first | Past it, the oldest move into one batch object (`SRV-150`, `SRV-151`). Bounds the register at the documents in use times this figure |
| Server, deltas past a document's served snapshot before the relay asks for one | 500 | `SNAPSHOT_THRESHOLD` in `ambre_server::compaction` (`SRV-038`). Bounds what a cold open replays. A first estimate, to measure against open times |
| Documents held in memory, per client | < 32 MB, to measure | Every document the Logic thread holds: the ones a view shows. A closed view frees its document (`RESID-001`) |
| Local store, per client | < 500 MB, to measure | Inside the browser quota. Held by eviction (`RESID-002`) |
| Murmure segment, at its close | 5000 messages or 2 MB of snapshot, whichever comes first | Bounds the first display of a channel and every final snapshot. Measured with chat-length bodies: 5000 messages make a snapshot of 879 KB, 581 KB shallow, so the message count binds first |
| Murmure members before an `@channel` asks for confirmation | 10 | Past it, one `@channel` notifies enough people to warrant a second thought. A first estimate, to measure against use (`MURMURE-231`) |
| Murmure members past which a channel defaults to the mentions level | 30 | A busy room would otherwise wake every member for every line. A first estimate (`MURMURE-267`) |
| Murmure members of a direct message, its creator included | 50 | Each member costs the relay one lookup and one signature check, so the cap bounds one request. `DIRECT_MESSAGE_MAX_MEMBERS` in `ambre_protocol::membership` (`SYNC-180`, `SRV-240`) |
| Murmure pins per channel | 50 | A pinned list longer than a screen stops being a list of what matters (`MURMURE-339`) |
| Murmure custom emoji, per space | 500, of 128 KB each at most | Every member loads the images the channel shows, so the cap bounds the space document and the images drawn |
| Murmure active segment, deltas past its latest snapshot | < 250, to measure on a phone | A cold open pays one inbound open (section 1, < 350 us) and one replay (section 1, < 250 us) for each. 250 of them cost about 150 ms of a phone cold open. Without a snapshot, a segment at its close replays 6250 deltas, fetched as 6250 objects. The relay asks for the snapshot (`SRV-038`). The phone figure assumes a phone about 7 times slower than the desktop: the ratio of the inbound open budget (350 us) to its desktop measure (50 us). The `Murmure measured on a phone` Story (`ambre-roadmap.md` E5) replaces that ratio with a measure |
| Light subscription, per document not held | < 1 KB | The key, the counters and the cursor (`RESID-004`) |
| Avatar picture | 256 x 256 pixels, < 48 KB | A member list draws one per member, so the cap bounds the list of a large space (`PROFILE-005`) |
| Document or channel name | 128 characters, sealed in at most 697 bytes | Every listing of documents and of channels serves each sealed name, so the cap bounds a listing. The same cap as a display name. `SealedName::MAX_CHARS` in `ambre_core` (`SRV-230`, `SYNC-178`) |
| User settings document | < 256 KB, to measure | Pinned while the user is signed in (`SETTINGS-005`), and counted in the documents held in memory |
| Color emoji font | loaded on first use, size to measure | Charged to the fonts row above. A full color emoji font weighs about 10 MB, so its size decides whether the fonts row still holds |

---

## 7. TOOLING

| Recipe | Target | Notes |
|---|---|---|
| `just fixtures` | ~30 s | A slow corpus is a corpus developers stop regenerating |
| `just fixtures-check` in CI | ~10 s | Holds only because the frozen set is capped at three per `doc_type`, and the check loads and migrates every one |
| Committed fixture artifact | 256 KB each, ~5 MB total | Indicative cap over `fixtures/current/` and `fixtures/frozen/`. The scale corpora are git-ignored and produced on demand (`ambre-spec-fixtures.md section 2`) |
| `just primitives`, the dependencies fresh, every workspace unit compiled again through the driver | 120 s, a ceiling | Measured on 2026-11-08 in a cloud container of 4 vCPU. The nine lint lines took 106 s after an earlier run of the gate, and 110 s right after `just lint` and `just lint-wasm`, 116 s for the whole recipe. The host line takes 29.5 s, the line of `ambre-belt` 2.2 s and the seven wasm lines 79 s, each printed by the gate. The spike timed the host library line alone at 9.3 s. A first run after a cold lint rebuilds the dependencies whose feature set differs, 7 min 50 s on the same day |

---

## 8. POLICY DURATIONS

A duration a rule waits on before it acts. Each figure is a first estimate, to measure.

| Duration | Target | Why this order of size | Method |
|---|---|---|---|
| Grace period before a closing Murmure segment is finalized | 24 h | a typo fix lands within minutes, so a day keeps every recent edit out of a sealed segment | `FINALIZE_GRACE` in `ambre_server::routes::channels`, counted from the close and from the latest delta (`SRV-199`) |
| Murmure message grouping span | 5 min | consecutive messages of one author inside the span share one header, as every chat product groups them | `GROUP_SPAN` in `ambre_murmure::list` (`MURMURE-183`) |
| Refresh period of the presence in a space or a channel | 60 s | a member list needs a state per minute, and the relay then writes one entry per member per minute (`MURMURE-445`, `MURMURE-126`) | `REFRESH_PERIOD` in `ambre_murmure::members` |
| Presence expiry in a space or a channel | 180 s | three refresh periods: one for the tick that dates a sample, one for a lost sample, one for two timers drifting apart (`MURMURE-445`, `MURMURE-126`) | `PRESENCE_EXPIRY` in `ambre_murmure::members` |
| Tick of an open channel | 1 s | a typing indicator lasts seconds, so its expiry needs a clock finer than it (`MURMURE-124`) | `TICK_PERIOD` in `ambre_murmure::watchers` |
| Delay before a Murmure client fetches again a successor that failed to open | 1 s, doubled at each failure, five failures at most | a successor fetched before its first update lands within a second, and a relay down for minutes is not asked each tick (`MURMURE-482`) | `SUCCESSOR_RETRY` in `ambre_murmure::channel` |
| Refresh period of a typing indicator | 3 s | one sample per few seconds of typing, against one per keystroke (`MURMURE-124`) | `TYPING_REFRESH` in `ambre_murmure::watchers` |
| Typing indicator expiry | 6 s | two refresh periods: one for the tick that dates a sample, one for a lost sample (`MURMURE-124`) | `TYPING_EXPIRY` in `ambre_murmure::watchers` |
| Idle delay before a member shows as away | 10 min | a short break keeps a member online, and a meeting marks them away | **none exists** |
| Finalize lease | 30 s | long enough to encode, seal and upload one segment of about 2 MB | `FINALIZE_LEASE` in `ambre_server::routes::channels` (`SRV-119`), which the fold lease shares (`SRV-204`) |
| Delay before the amendments of a segment fold into it | 7 days | a week batches the edits of one segment into one fold, and leaves three weeks of the erasure budget below for the confirmation | `FOLD_DELAY` in `ambre_server::routes::channels`, counted from the last fold or from the creation of the amendments (`SRV-206`); `FOLD_DELAY` in `ambre_murmure::channel`, which a client waits out after its own fold |
| Idle delay before a document's inline deltas move to a batch object | 24 h | a document untouched for a day is rarely opened within the next, and its tail then costs one object read | `IDLE_AFTER` in `ambre_server::hot_tail`, the delay the compaction pass reads (`SRV-152`) |
| Shortest interval between two replays of one document on one connection | 1 s | a reconnect asks once per document, so a second ask within the second is a client looping or an amplification attempt (`SRV-086`) | `REPLAY_MIN_INTERVAL` in `ambre_protocol::ws`, which the relay reads |
| Delay before Sync asks again for a stopped replay | 1 s, doubled at each stop, five stops at most | the first ask must outlast the replay interval above, or the relay drops it. Five stops wait 31 s in all, which outlasts a store that restarts, and a delta lost for good stops every ask (`SYNC-153`, `SYNC-154`) | `REPLAY_RETRY_BASE` and `REPLAY_RETRY_LIMIT` in `ambre_sync::websocket` |
| Shortest interval between two replay asks of one document Sync sends on one socket | 1.25 s | the asks must outlast the replay interval above, with a margin for the frames in flight. A watch and an open of one channel ask within the second (`SYNC-188`). A spacing at or under that interval fails the build (`SYNC-191`) | `REPLAY_ASK_SPACING` in `ambre_sync::websocket` |
| Delay before Sync sends again a delta the relay failed to sequence | 1 s, doubled at each failure since the last echo, five failures at most | five failures wait 31 s in all, which outlasts a clock store that restarts. A relay that keeps failing is not hammered, and past the budget the document holds the delta (`SYNC-159`, `SYNC-156`) | `RESEND_RETRY_BASE` and `RESEND_RETRY_LIMIT` in `ambre_sync::updates` |
| Margin before the announced expiry of an access token at which Sync renews it | 60 s | twice the request deadline of Sync: the renewal may take one, and a request sent under the old token just before it may take the other. Both land while the old token lives (`SYNC-183`) | `RENEWAL_MARGIN` in `ambre_sync::renewal` |
| Delay between a Murmure delete after the finalization and its erasure | < 30 days | the re-finalization runs in batches, and an erasure request expects an answer within a month | **none exists**: `FOLD_DELAY` bounds the wait for a fold (`SRV-206`), and nothing yet measures the confirmation after it (`SRV-208`) |
