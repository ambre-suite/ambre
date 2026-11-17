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
| Log head in the public data of one update, wire bytes (`SYNC-206`) | 34 B, for a log under 128 entries | 43 B | Sync -> relay | `ambre_protocol::tests::a_log_head_costs_an_update_at_most_43_bytes` -- 2 B on an empty log, 34 B below 128 entries, 43 B at a place of `u64::MAX`, by postcard, read on 2026-11-14. The relay stores each update whole, so each delta stored costs the same bytes more. A peer checks a head against its log from the end, so a current head costs one comparison of 32 bytes. Only a head the log does not hold costs one more signature check |
| Log entry in the public data of one update, wire bytes (`SYNC-218`) | 230 B, for a log under 128 entries | 239 B | Sync -> relay | `ambre_protocol::tests::a_log_entry_costs_an_update_at_most_239_bytes` -- 1 B on an empty log, 230 B below 128 entries, 239 B at a place of `u64::MAX`, by postcard, read on 2026-11-16. With the head, an update carries 264 B of log metadata below 128 entries. The relay stores each update whole, so each delta stored costs the same bytes more. A peer checks the entry only for a head its log does not hold: one encode, one hash and one signature check |
| Log head and log entry in the public data of one presence sample, wire bytes (`SYNC-262`) | 264 B, for a log under 128 entries | 282 B | Sync -> relay | `ambre_protocol::tests::a_log_head_and_entry_cost_an_ephemeral_at_most_282_bytes` -- 3 B on an empty log, 264 B below 128 entries, 282 B at a place of `u64::MAX`, by postcard, read on 2026-11-17. The relay keeps no sample past its expiry, so the cost is per send, at the pace of presence. A current head costs a receiver one comparison of 32 bytes; only a head the log does not hold costs the check of its entry and of the sample signature |
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
| Trame CSV import (10k lines) | < 200 ms | -- | the converter thread, then Logic | criterion -- **none exists** |
| Trame CSV export (10k lines) | < 100 ms | -- | Logic, then the converter thread | criterion -- **none exists** |
| Trame xlsx import (1k cells / 10k cells) | < 500 ms / < 3 s | -- | the converter thread, then Logic | criterion -- **none exists**. The import lands one slice of 256 cells per pass (`TRAME-IO-013`): 3 to 6 ms a slice on 10 250 cells, 2026-11-10, host release build in a cloud container. The codec alone reads 100k cells in about 100 ms: 2026-11-09, `cargo test --release -p ambre-xlsx --test large_sheet -- --nocapture`, host build in a cloud container, three runs |
| Trame xlsx export (10k cells) | < 1 s | -- | Logic, then the converter thread | criterion -- **none exists**, and no gesture reaches the codec. The codec alone writes 100k cells in 107 to 124 ms: 2026-11-09, the same command, the same three runs |
| Trame `SetCellText` to viewport (P2) | < 5 ms | -- | Logic | recompiles the bytecode and updates the DAG -- **none exists** for the write itself. The CI `instructions` job holds the whole undo and the whole redo of one cell write on the sheet of 10k formulas to this row, through `cell_write_undo` and `cell_write_redo` in `crates/ambre-trame/benches/instructions.rs`, counted in the functions named after them, against a ceiling of 4 000 000 each. Measured by callgrind on 2026-11-08, native release build in a cloud container: through the Loro undo manager, 247 030 884 for a first undo, 299 192 706 for a second and 199 521 206 for a redo, 99.9 % of each in its checkout. With the edit recording its own inverse (`TRAME-288`): 231 789 and 227 997 under `just instructions` On 2026-11-10, read by `python3 scripts/instructions.py` under the `fixed-hash-key` feature and a pinned peer, native release build in a cloud container, the same count in each of three runs: the undo of one cell write 173 915 and its redo 173 403. |
| Trame batch of 50 cells (P4) | < 1 ms | -- | Logic | **none exists** |
| Trame `SheetIndex` rebuild, 10k rows | < 10 ms | -- | Logic | at load time -- **none exists** |
| Trame structural edit on the Logic thread, 10k formula cells: the re-anchor walk and the engine step of one row or column insert, delete or move | < 10 ms | -- | Logic | the 15 ms of a content update, the Scroll viewport response row above, less the < 5 ms frame total of section 2. A recompute the user asks for is the Trame formula DAG recompute row, not this one. The CI `instructions` job holds it through `structural_edit` in `crates/ambre-trame/benches/instructions.rs`: a delete of the middle row, counted in `reanchor_formulas` and `carry_engine`, against a ceiling of 8 000 000. Measured by callgrind, native release build in an Intel Xeon 2.30 GHz cloud container. On 2026-11-05, before the carried engine: the walk 249 845 678 instructions; the reseed 11 003 622 904, of which the whole-graph cycle check 8 499 150 000, the compile and graph build about 1 841 000 000, the recompute 168 075 977; the whole delete 11 452 904 754. On 2026-11-06, after it: the walk and the engine step 4 408 783 under `just instructions`, about 5.5 ms on the target phone at the rate `scripts/instructions.py` assumes; the whole delete 162 433 725. The purge and the commit fall outside the count. The whole delete is the row below. On 2026-11-07: 87 204 under `just instructions`. The delete now derives the layout after it as it removes the line, outside the walk, and the engine step borrows that layout instead of copying it On 2026-11-10, read by `python3 scripts/instructions.py` under the `fixed-hash-key` feature and a pinned peer, native release build in a cloud container, the same count in each of three runs: 56 537. The same job holds one slice of the sweep of 500 hidden rows to this row through `many_line_sweep_slice`, counted in the whole `ambre_budget_trame_sweep_slice` function, against a ceiling of 8 000 000 (`TRAME-307`). On 2026-11-12, read the same way: 5 065 283 for a slice of 32 rows, and 88 691 941 for the 17 slices of the whole sweep. |
| Trame whole line delete on the Logic thread, 10k formula cells: one `DeleteRow` command from its start to its outcome, the purge, the commit and every cache included | < 10 ms | -- | Logic | the 15 ms of a content update, the Scroll viewport response row above, less the < 5 ms frame total of section 2, as for the structural edit row above, which is its walk and its engine step alone. The CI `instructions` job holds it through `line_delete` in `crates/ambre-trame/benches/instructions.rs`, counted in the whole `ambre_budget_trame_structural_edit` function, against a ceiling of 8 000 000. It holds the whole undo and the whole redo of that delete to the same budget each, through `line_delete_undo` and `line_delete_redo`, counted in the whole `ambre_budget_trame_line_delete_undo` and `ambre_budget_trame_line_delete_redo` functions. Those of a row insert in the middle of the sheet hold the same budget, through `line_insert_undo` and `line_insert_redo`. Each operation counts its step in a function of its own, named after it. Measured by callgrind, native release build in an Intel Xeon 2.30 GHz cloud container. On 2026-11-06: 162 433 725 instructions, about 203 ms on the target phone at the rate `scripts/instructions.py` assumes, of which the purge, a walk of the whole cell map, about 116 000 000. On 2026-11-07, with the purge read from an index by line, the slots read from the position cache and the layout lent to the engine: 4 858 050 under `just instructions`, about 6.1 ms. The tail of the undo 2 395 320 and of the redo 2 401 417, about 3.0 ms each. The step of the Loro undo manager fell outside the tail: about 336 000 000 for the undo and 288 000 000 for the redo, measured the same day. It rebuilt the state of the whole cell map at a version. Later on 2026-11-07, with the delete recording its own inverse (`TRAME-286`), the whole undo and the whole redo each, under `just instructions`: 6 045 659 and 5 354 932, about 7.6 ms and 6.7 ms. The whole delete reads 4 906 798 the same day. On 2026-11-08, an undo of a row insert went through the checkout of the Loro undo manager: 49 962 823 for the undo and 2 650 288 for the redo, under the same count. With the insert recording its own inverse: 5 158 926 and 5 552 100 under `just instructions`, about 6.4 ms and 6.9 ms. That path was withdrawn the same day: its redo put back a new list element, and the manager's redo of a later move revived the old one (`2-Trame/features/ambre-spec-trame-undo.md` section 6). The undo and the redo of the delete read 5 756 689 and 5 409 913 the same day. The delete of a block of rows is the row below. The undo and the redo of one line delete then read 6 187 243 and 2 517 595 under `just instructions`. Later on 2026-11-08, with every edit recording its own inverse into one history and no undo manager (`TRAME-288`): under `just instructions`, the undo and the redo of one line delete 3 721 120 and 3 351 188, those of a row insert 3 129 512 and 3 440 342. The whole delete reads 4 933 212. The tail of the undo is about 73 000 000 of it, the formula engine recompiling each restored formula and the change observers parsing each restored key. The delete itself cost 1 658 407 733 the same day, about 2.1 s On 2026-11-10, read by `python3 scripts/instructions.py` under the `fixed-hash-key` feature and a pinned peer, native release build in a cloud container, the same count in each of three runs: the delete 4 528 564, its undo 2 347 127 and its redo 2 355 539, the undo of a row insert 2 216 194 and its redo 2 209 545. The same job holds a line delete while 500 rows stay hidden to this row through `line_delete_after_block`, counted in the whole `ambre_budget_trame_line_delete_after_block` function, against a ceiling of 8 000 000 (`TRAME-308`). On 2026-11-12, read the same way: 4 509 806, where the line delete alone reads 4 572 224. |
| Trame delete of a block of five hundred rows on the Logic thread, 10k formula cells: one `DeleteSelectedRows` command, its whole undo and its whole redo, each | < 16 ms | -- | Logic | the frame at 60 FPS, the target set on 2026-11-08: a block delete is rarer than a keystroke, so it may take a frame where a content update takes 10 ms. `many_line_delete`, `many_line_delete_undo` and `many_line_delete_redo` in `crates/ambre-trame/benches/instructions.rs` count each in a function of its own. The CI `instructions` job holds each under the count reached plus 0.5 %, below the target since 2026-11-12: 9 160 000, 10 390 000 and 6 540 000 (`TRAME-296`, `TRAME-297`, `TRAME-298`, `TRAME-299`). A ceiling moves down only, except by a story that names the new figure and its cause. Measured by callgrind on 2026-11-08, native release build in a cloud container. At the start of the day: the delete 1 666 842 974 instructions, its undo 1 550 269 610 and its redo 1 454 367 305, about 2.1 s, 1.9 s and 1.8 s on the target phone at the rate `scripts/instructions.py` assumes. The delete walked the generation record and renumbered the layout once per line, and the undo read the row list once per element. With one history, one pass of the list and of the record for the whole block, and each key spelled by hand: 85 146 535, 101 044 252 and 73 363 855, about 106 ms, 126 ms and 92 ms. Of the undo, about 17 % hashes keys through SipHash, and the formula engine recompiles each of the 500 restored formulas. Later on 2026-11-08, the same base read 85 437 547, 101 414 089 and 74 238 101 under callgrind. With every Trame table on foldhash under a key drawn per process (`TRAME-292`): 58 398 348, 74 592 577 and 54 318 734. With the dirty closure walked without a `Dfs` per cell and the purge of the record by hash: 50 392 801, 70 865 335 and 50 280 585, about 63 ms, 89 ms and 63 ms. What stays is Loro: the list and map writes, the commit with its observers and the export are about half of each. The formula parser takes about 8 000 000 of the undo On 2026-11-09, under a random key, a run read 73 597 735 for the undo and 52 555 839 for the redo, each over its ceiling of then, with no change to the code. On 2026-11-10, read by `python3 scripts/instructions.py` under the `fixed-hash-key` feature and a pinned peer, native release build in a cloud container, the same count in each of three runs: 50 688 963, 70 227 650 and 51 738 504. On 2026-11-11, read the same way: 41 920 358, 55 227 907 and 42 612 313. The purge reaches each map once for the block, one subscription keeps both notes of Trame and parses each key once, and the engine walks the dirty closure once per batch. The Loro writes, the commit without its observers and the export are about 24 000 000, 28 000 000 and 26 000 000 of them, read from the same profiles: the row is out of reach of any pass on the Trame side (`TRAME-297`, `TRAME-299`). Later on 2026-11-11, read the same way: 41 921 379, 55 224 896 and 42 609 298 on the base. With the delete parking each removed formula and the undo reusing its bytecode (`TRAME-300`): 41 625 876, 47 150 108 and 42 379 032. The undo no longer parses or compiles the 500 restored formulas. Later on 2026-11-11, on `nightly-2026-11-11`, two runs read the same counts: 41 982 315, 47 912 075 and 42 844 674, up 0.86 %, 1.62 % and 1.10 %. The source is the same on both pins, so the move is the codegen of the new compiler. Each ceiling rose to the new count plus 0.5 %, rounded up to 10 000, by the story `The pinned nightly moved to nightly-2026-11-11`. On 2026-11-12, read by `python3 scripts/instructions.py` the same way, a block delete hides its lines under one register and writes no cell, and the undo reads the cells it brings back from the index by line (`TRAME-302`, `TRAME-299`): 9 113 943, 10 329 819 and 6 499 149, about 11.4 ms, 12.9 ms and 8.1 ms. Met. Each ceiling fell to the new count plus 0.5 %, rounded up to 10 000. The sweep that removes the hidden lines later runs off the gesture, one slice per pass of the Logic thread: the structural edit row holds a slice (`TRAME-307`). |
| Trame remote batch on the Logic thread, 10k formula cells: one `apply_remote_batch` of a peer's cell write and row delete, from the import to the events, every cache included | < 10 ms | -- | Logic | the 15 ms of a content update, the Scroll viewport response row above, less the < 5 ms frame total of section 2, as for the structural edit row above: a peer's change is a content update on this screen. The CI `instructions` job holds it through `remote_batch` in `crates/ambre-trame/benches/instructions.rs`, counted in the whole `ambre_budget_trame_remote_batch` function, against a ceiling of 8 000 000. Measured by callgrind on 2026-11-06, native release build in an Intel Xeon 2.10 GHz cloud container, command `valgrind --tool=callgrind --toggle-collect='*ambre_budget_trame_remote_batch*'` on the bench binary with the argument `remote_batch`. Before the carried apply: 1 218 213 117 instructions, about 1 523 ms on the target phone at the rate `scripts/instructions.py` assumes, of which the engine reseed 1 159 983 829. After it: 2 595 061, about 3.2 ms. Later the same day, with the walk of formulas that place on no cell and the ranged repaint: 2 652 475 under `just instructions`, about 3.3 ms. The layout derived from the list diffs replaced a read of the 10k-row list, which alone cost about 41 600 000 On 2026-11-10, under the fixed key and a pinned peer: 1 746 629, 1 745 580 and 1 745 500 in three runs. The import of the snapshot fills a block cache of Loro, `quick_cache` under a random `ahash` key, and the heap it leaves moves the count by about 0.1 %. The ceiling is the budget. |
| Trame `range_index` scan, 1k range formulas | < 1 ms | -- | Logic | linear scan at the MVP, an interval tree later -- **none exists** |
| Toile visible-set query, 50 000 elements | < 1 ms | -- | Logic | must fit inside a frame with margin for tessellation -- **none exists** |
| Toile index update, one element | < 50 us | -- | Logic | a drag mutates one element per input event -- criterion `ambre-toile` `index_update_one_of_50000`: 1.3 us on a desktop in native code (`TOILE-091`) |
| Toile remote change, delta < 32 KB | < 2 ms | -- | Logic | keeps the client share of the < 300 ms inter-user budget -- criterion `ambre-toile` `remote_change_5000`: one move costs 0.8 ms on a desktop in native code, against 48 ms before the scaffolding was shared (`TOILE-265`) |
| Toile transform command (move, restyle) | < 1 ms | -- | Logic | one register write, one index update, one derivation -- **none exists** for the command itself. Its whole undo and its whole redo, on a board of 2 000 stickies, are `move_undo` and `move_redo` in `crates/ambre-toile/benches/instructions.rs`, each counted in a function named after it. Measured by callgrind on 2026-11-08, native release build in a cloud container: 59 930 493 and 59 594 465 instructions, about 75 ms on the target phone, 98 % of it the whole reseed of the caches. On 2026-11-09, once an undo refreshes only what it wrote (`3-Toile/features/ambre-spec-toile-undo.md`, `TOILE-292`): 769 315 and 768 985, the median of three runs of `valgrind --tool=callgrind --toggle-collect` on the bench binary. Met: the CI `instructions` job holds both at 800 000 (`TOILE-291`). The derivation of the viewport is 86 % of it, and SipHash over the paint order is 61 %: the margin is thin On 2026-11-10, with every table of Toile and of the history on `ambre_core::hash`, read by `python3 scripts/instructions.py` under the `fixed-hash-key` feature and a pinned peer, native release build in a cloud container, the same count in each of three runs: 419 225 and 415 173. Under a random SipHash key the day before, three runs read 770 226, 770 849 and 769 734 for the undo. |
| Toile body commit, a 4 KB body merged from its base | < 1 ms | -- | Logic | two character diffs capped at 256 edit steps, then the merge's edits written into the body, per close (`TOILE-UI-381`). The UI echo runs the same two diffs inside its frame (`TOILE-UI-382`) -- `callgrind` `ambre-toile` `body_commit`: about 690 000 instructions against a ceiling of 800 000 (`TOILE-UI-383`). Both diffs run to their cap cost about 6 million, 7 ms on the phone: the bound is held at 6.5 million, and it is not the budget On 2026-11-10, read by `python3 scripts/instructions.py` under the `fixed-hash-key` feature and a pinned peer, native release build in a cloud container, the same count in each of three runs: 714 603, and 5 844 281 with both diffs at their cap. |
| Toile slide transition, 500 elements per frame | < 4 ms | -- | Logic | leaves room to paint inside the frame during a transition -- **none exists** |
| Toile layout solve, 200 children | < 1 ms | -- | Logic | runs at derivation on every child edit -- **none exists**, and nothing calls the solver (`TOILE-FEAT-029`) |
| Toile reaction count | < 0.1 ms typical | -- | Logic | one scan of the reactions map on read, never stored -- **none exists** |
| Toile boolean combination, two typical paths | < 5 ms | -- | Logic | user-initiated and one-shot, off the render frame -- **none exists** |
| Toile SVG import, 5 000 nodes | < 500 ms | -- | the converter thread, then Logic | one-shot user action, off the render thread, applied in slices between the thread's messages (`TOILE-IO-028`) -- `crates/ambre-toile/benches/import.rs` measures the whole import and one slice, desktop only: about 280 ms whole and 1.7 ms a slice. The slice is what a camera move waits behind; an edit waits for the whole |
| Toile PPTX export, 50 slides | < 500 ms | -- | Logic, then the converter thread | the board walk on Logic, then the writer on the converter thread (`ARCH-129`). The writer alone took 18 ms for 50 slides of 26 items and one 256 px picture, on 2026-11-09, on the x86_64 host of the session, release build, through a timing test kept out of the tree -- **none exists** in CI (`3-Toile/features/ambre-spec-toile-pptx-export.md`) |
| Toile PDF export, 50 slides | < 3 s | -- | Logic, then the converter thread | acceptable for an explicit export, off the render frame. The pages are recorded one per pass of the Logic thread, so an edit waits behind one page, the row below, never behind the export (`3-Toile/features/ambre-spec-toile-pdf-export.md`, `TOILE-IO-099`) -- criterion `ambre-toile` `pdf_export_50_slides`, a deck of a title, a sticky and four shapes per slide, no picture, on the shipped faces. On 2026-11-14, native release build on the x86_64 host of a cloud session: 10.7 ms whole, of which 0.93 ms is the pass of the request that derives every page, 0.12 ms the first page, and 7.8 ms the PDF module writing the file in the pass of the last page. Since `ARCH-124` that write runs on the converter thread, so no pass of the Logic thread holds it. No CI job runs it. On 2026-11-15, `callgrind` on `deck_request` of `crates/ambre-toile/benches/instructions.rs`, native release build in a cloud container: 9 248 839 instructions for the whole pass of the request, which then derived every page. The derivation of the 50 pages took 7 880 736 of them, and the first page 1 367 560. Each pass now derives its own page alone, the row below |
| Toile PDF export, one slice: the recording of one slide | < 4 ms | -- | Logic | the Logic pass that answers a keystroke row: an edit of the board waits behind one page (`TOILE-IO-099`). The CI `instructions` job holds `deck_page` of `crates/ambre-toile/benches/instructions.rs` under 3 200 000: each page of the deck of the row above, counted apart, the dearest held. It holds `deck_request` under the same ceiling: the whole pass of the request. That pass reads the frame of every page, the pictures and the version of the board, then derives and records the first page. On 2026-11-14, `scripts/instructions.py` on `deck_page`, native release build in a cloud container: 1 384 090, the first page, which shapes its runs cold. On 2026-11-17, the same script, with each page derived in its own pass: 1 472 684 for `deck_page` and 2 971 844 for `deck_request`. The step after an edit copies the board at the version of the request: 58 430 104 for `deck_freeze`, the same day, through `callgrind` on its function, and no ceiling holds it (`3-Toile/features/ambre-spec-toile-pdf-export.md section 6`). The pass of the last page is dearer, the row above |
| Coverage levels across a 30-degree vector edge | >= 8 of 256 | -- | UI | the antialiasing quality figure, measured rather than eyeballed: rendered headless, the alpha histogram along the edge is the mechanism, and a pipeline with no coverage term yields exactly 2 (`CANVAS-061`, `ambre-spec-drawing-surface.md`) -- **none exists** |
| Antialiasing band width, at any zoom | 1.0 physical px +/- 0.25 | -- | UI | measured at both ends of the zoom range on one shape; a band tessellated in model units and not divided by the transform scale fails it as the board zooms in (`CANVAS-063`) -- **none exists** |
| Toile first paint of a picture | < 15 ms | -- | UI | the display-latency budget: the placeholder paints and the network never blocks it -- **none exists** |
| Toile media cache hit on a revisit | 0 refetch | -- | UI | the bounded local cache (`features/ambre-spec-toile-media.md section 3`) -- **none exists** |
| Prose local text insert, transaction plus exported delta | < 2 ms | -- | Logic | **none exists** for the insert itself. Its whole undo and its whole redo, one key on a document of 300 paragraphs of 500 characters, are `key_undo` and `key_redo` in `crates/ambre-prose/benches/instructions.rs`, each counted in a function named after it, against a ceiling of 1 600 000. Measured by callgrind on 2026-11-08, native release build in a cloud container, through the one history: 4 015 618 and 3 991 777 instructions while the replay walked every root block, 95 % of it. With that walk skipped for a replay of text alone: 204 638 and 216 502, about 0.26 ms. Under `just instructions` on 2026-11-09: 196 090 and 209 364 (`4-Prose/features/ambre-spec-prose-undo.md`, `PROSE-135`) On 2026-11-10, read by `python3 scripts/instructions.py` under the `fixed-hash-key` feature and a pinned peer, native release build in a cloud container, the same count in each of three runs: the undo of a key 197 324 and its redo 209 410. |
| Prose pagination, full recompute of 100 pages | < 20 ms | -- | Logic | placement over already shaped runs, which is why the shaping rows below are charged apart (`features/ambre-spec-prose-pagination.md`) -- **none exists**, and no code either |
| Prose pagination, full recompute of 300 pages | < 60 ms | -- | Logic | a novel, the document Prose must hold, not an extreme of it. The 100-page row times three, held apart so that a cost that grows faster than the page count shows -- **none exists** |
| Prose pagination, partial reflow of 10 pages or fewer | < 3 ms | -- | Logic | the early exit converges inside the frame; an edit whose reflow does not converge is charged to the row above -- **none exists**, and no code either |
| Prose cold shape, 10 dirty blocks | < 5 ms | -- | Logic | a glyph-level pass, an order of magnitude over placement -- **none exists**, and no code either |
| Prose runs shaped again on a jump, a window of twelve pages | < 20 ms | -- | Logic | a jump to a page far from the pages held shapes the runs of the whole window again before its layout is handed over (`PROSE-137`). A scroll shapes the four pages it asks ahead of the ones it builds. Measured on 2026-11-10 by the probe of the `Prose shaped runs` row, on the x86_64 host of the session: 72 blocks in 14.6 ms in the middle of 300 pages, 70 in 13.6 ms at the end -- **none exists** in CI |
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
| Prose spell check, 200 words, its lookups | < 10 ms | -- | Converter | the converter thread, never the Logic thread (`PROSE-FEAT-041`). On 2026-11-16, a scratch binary over `spellbook` 0.4.2, host release build in a cloud container, read 6 000 lookups in 0.6 ms in English and 30 ms in French: about 0.02 ms and 1 ms for 200 words. No bench of the workspace holds it, and no phone was measured |
| Prose spell check, the first load of one dictionary | < 300 ms | -- | Converter | the module parses its dictionary on its first request and keeps it. The same binary read 11 ms for English and 33 ms for French on the host, and 2.3 MB and 5.7 MB of resident memory, under the 64 MiB reset of a converter instance (`ARCH-099`) |
| Prose spelling suggestions, one word | < 1 s | -- | Converter | one job per press on an underlined word, never one per error (`features/ambre-spec-prose-spell-check.md section 4`). The same binary read 6 ms to 13 ms for an English word and 40 ms to 250 ms for a French one |
| Prose spell check, the Logic thread's half: the cut of the stale blocks into words | < 4 ms | -- | Logic | the words of one check are capped at 4 096, so a pasted book is cut in several passes -- **none exists**: no bench counts it |
| Prose DOCX import / export, 100 pages | < 500 ms / < 300 ms | -- | Logic, then the converter thread | sequential ZIP and XML work, one-shot and off the render frame. `crates/ambre-docx/benches/codec.rs` measures the codec alone, desktop only: on 2026-11-09, `cargo bench -p ambre-docx --bench codec` in a cloud container (Xeon at 2.1 GHz, host release build) reads 100 pages of 500 words, each with a table, a comment and a tracked change, in 7.9 ms and writes them in 4.5 ms. A novel of 300 pages reads in 3.7 ms, and a real 12 MB Word body in 270 to 550 ms. The module boundary and the mapping into the document are built and not measured, and **no mobile figure exists** |
| Prose DOCX import / export, 300 pages | < 1.5 s / < 1 s | -- | Logic, then the converter thread | a novel, the whole conversion. The Logic thread holds it a slice at a time, as the row below states (`PROSE-IO-048`). The converter thread holds the module call whole, so a key typed in another document never waits behind it (`ARCH-122`) -- **none exists** |
| Prose import or export, one slice: the write of 12 operations into the document, or the walk of 48 top-level blocks | < 4 ms | -- | Logic | the Logic pass that answers a keystroke row: a key typed in another document waits behind one slice (`PROSE-IO-099`). The CI `instructions` job holds `import_slice` and `export_slice` of `crates/ambre-prose/benches/instructions.rs` under 3 200 000 each, a slice past the middle of a file of 2 000 paragraphs. On 2026-11-12, native build in a cloud container: 2 169 924 and 2 195 180. On 2026-11-13, once an import wrote its list once and no longer rehearsed it, same build: 2 158 076 and 2 202 852. The module call stays whole, a few percent of the conversion: on 2026-11-12, host release build, a Markdown file of 11 MB read in 84 ms and wrote into the document in 3.8 s, and its walk took 1.3 s and its module write 92 ms. Through the Logic passes, by a probe kept out of the tree, 6 000 operations landed in 0.69 s over 1 000 passes, p95 0.56 ms a pass, against 0.7 s whole. The slowest pass, 191 ms, holds the module read or the layout at the end |
| Prose Markdown export, 100 pages | < 50 ms | -- | Logic | no layout required -- **none exists**, and no code either |
| Prose PDF export, 100 pages, without / with images | < 2 s / < 5 s | -- | Logic, then the converter thread | the walk draws each page over one pass or more (row below), shaping again the runs the paginator dropped, and the PDF module writes the file whole on the converter thread (`ARCH-124`). The second figure adds the pictures, each drawn as its stored file (`features/ambre-spec-prose-images.md section 3`) -- **none exists** |
| Prose PDF export, one slice: the shaping of runs for one page, or the drawing of one page | < 4 ms | -- | Logic | the Logic pass that answers a keystroke row: a key typed in another document waits behind one slice (`PROSE-IO-103`). A slice shapes again at most 500 characters of the runs the paginator dropped, or one longer block alone, and the slice after the last one draws the page. The CI `instructions` job holds `print_slice` of `crates/ambre-prose/benches/instructions.rs` under 3 200 000: each slice of page 1, near the view, and of a page past the middle of a file of 2 000 paragraphs, far from it, counted apart, the dearest held. On 2026-11-12, one slice a page, `scripts/instructions.py` on that operation: 11 724 885 far and 3 497 920 near, both above the ceiling. On 2026-11-13, native build in a cloud container, the same script: 2 511 589, the drawing of a page. Each slice apart, through `valgrind --dump-before` on the same functions: 2 490 403 for the near page, 2 501 557 for the drawing of the far page, and 847 376 to 919 288 for each of its ten shaping slices. A block longer than about 1 200 characters exceeds the row alone: a paragraph shapes whole. With the pictures drawn (`features/ambre-spec-prose-images.md section 3`), the same day and script: 2 498 757 for `print_slice`, and 2 728 514 for `print_slice_picture`, the same slices with pictures on the first pages, each drawn as its stored file and decoded nowhere. On 2026-11-14, the same container and script, once a page walks the items of each line once and the recorder sizes its lists: 2 181 903 for `print_slice`, 2 374 083 for `print_slice_picture`, and 2 831 143 for `print_slice_dense`, a page of short paragraphs that change style every few words and a picture after every third block. Before that change the dense page read 4 621 542: Parley walks a line from its start to reach each of its items. CI holds all three, `print_slice` under the row and the two others under their count plus 0.5 % |
| Prose picture decode, one picture: the Logic thread's half, the header check and the decode handed on | < 4 ms | -- | Logic | a slice of the Logic thread, as the export slice row above: a key typed in another document waits behind it (`features/ambre-spec-prose-images.md section 3`). On 2026-11-13, before the converter thread, x86_64 cloud container of 4 vCPU, Intel Xeon, native release build, by a timing test kept out of the tree, best of three, `ambre_prose::picture::decode` whole on the Logic thread: a JPEG of 8000 by 6000 pixels and 4.7 MB, 642 ms; a photograph of 4032 by 3024 pixels and 0.9 MB, 235 ms. About 4.5 s and 1.6 s on the target phone at the rate `scripts/instructions.py` assumes. Later on 2026-11-13, a JPEG decodes on the converter thread (`PROSE-FEAT-330`, `ARCH-097`). The CI `instructions` job holds `picture_arrive` of `crates/ambre-prose/benches/instructions.rs` under 3 200 000: a photograph of 4032 by 3024 pixels arrives, and the count covers the whole media answer on the Logic thread. Read by `scripts/instructions.py` on that operation alone: 2 981. Met for a JPEG. The decode itself moved to the converter thread, and takes as long there: the same container, the images engine through `ambre_convert::Converter` at the cache edge and upright, by a timing test kept out of the tree, best of three, 235 ms for 4032 by 3024 and 646 ms for 8000 by 6000. On 2026-11-15, a PNG and a GIF decode on the converter thread too (`PROSE-FEAT-332`, `ARCH-116`), and the job holds `picture_arrive_png` and `picture_arrive_gif` under the same 3 200 000: the photograph as a PNG, and a GIF of ten frames of 640 by 480. Read by `scripts/instructions.py` on those operations alone, the same container: 3 242 for the JPEG, 2 847 for the PNG, 2 665 for the GIF. Met for every format. The decode itself, on the same container, by `crates/ambre-convert/benches/decode.rs` (`cargo bench -p ambre-convert --features images --bench decode`), native release build, the median of ten samples: the JPEG photograph at the cache edge, upright, 284 ms; the same photograph as a PNG, 443 ms; the GIF at the preview size, every frame, 101 ms; the photograph as a lossless WebP, whole, 186 ms |
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
| Call audio bitrate, Opus, asked constant of each sender | 48 kbit/s | 64 kbit/s | Call worker | the ceiling is the bitrate whose 20 ms frame fills the 160 clear bytes of a chunk, and the target leaves a quarter of it free. `ambre_belt::murmure::tests::three_clients_decode_each_others_sealed_audio_through_the_sfu` counts no frame dropped for its size. On 2026-11-10, an instrumented run of that test read each 20 ms frame. Chrome 153 held 120 bytes on all 1 757. Firefox 160 ignores `cbr` and read 121 bytes at the median, 130 at P99 and 152 at most, over 1 749 frames of its fake microphone |
| Call chunk, one seal or one open in the transform | < 5 ms (P99) | -- | Call worker | **none exists**. The call worker seals and opens each chunk in its transform (`crates/ambre-call`), and no run measures one. `just call-proof-android` and `just call-proof-ios` measure it on a phone, its worker phase (`ambre-spec-call-proofs.md`). |
| Call worker load: three VP8 sizes and Opus sent, nine videos and the loudest audio received, about 1 500 chunks per second | < 25 % of one core | -- | Call worker | measured while the Sync worker seals a file -- **none exists**. No run measures the call worker with video. `just call-proof-android` and `just call-proof-ios` measure it on a phone, its worker phase (`ambre-spec-call-proofs.md`). |
| Call video sent, the three VP8 encodings of one camera | 320x180 at 150 kbit/s, 640x360 at 500 kbit/s, 1280x720 at 1 500 kbit/s | the same | UI | the sizes the encodings scale the camera to, and the bitrate each one is capped at (`CALL-096`). The bitrates are those of the synthetic participants of the SFU simulation, so its measures hold for a browser. A headless engine scales the camera down when its encoder lacks the CPU, and every size shrinks with it. `ambre_belt::murmure::tests::three_members_decode_each_others_video_at_the_size_the_sfu_chooses` reads the sizes and caps a sender asks, and the ratio of the sizes it encodes |
| Call screen sent, the one VP8 encoding of a share | 1920x1080 at most, 30 fps at most, 1 500 kbit/s | the same | UI | the capture asks for the size and the frame rate, and the encoding is capped at the frame rate and the bitrate (`CALL-115`). Text needs the full size. Under the hint `detail`, the browser spends spare bitrate on frame rate only once the text is sharp: a still document stays sharp, and a scroll or a demonstration reaches 30 fps. The bitrate cap, not the frame rate, sets the egress of the SFU, 49 times 1 500 kbit/s at worst, so the network budget does not move. The cost is encoder time on the sharer while the screen moves. No run measures that time, nor the decode of a screen on a phone |
| Call sender key switch, from a leave or a removal to the fresh key in use | 5 s | 12 s | Sync -> Call worker | the delay lets late wraps arrive, and a member who left opens chunks until it ends (`CALL-027`). The key model switches at the target, in simulated time: `ambre_sync::call::tests::a_leave_switches_each_remaining_sender_within_the_delay`. The handoff reaches the call worker, and `ambre_belt::murmure::tests::three_clients_decode_each_others_sealed_audio_through_the_sfu` finds a removed member decoding nothing 9 s after her leave, a bound and not a measure of the switch |
| Call state in the relay store, its time to live after the last heartbeat of a seat | 60 s | 60 s | Relay | nothing of a call outlives its last socket by more, and a seat no socket beats for, or a lost call, lasts one time to live at most (`SRV-248`, `SRV-251`, `ZK-022`), held by `ambre_server::ws::handler::call::tests::a_seat_that_stops_its_heartbeat_leaves_the_roster_and_frees_its_place` and by `ambre_server::ws::handler::call::tests::nothing_of_a_call_outlives_its_time_to_live_without_a_heartbeat` at a time to live of 1 s. The target trades the room a seat holds after its relay stops against the load of the heartbeat |
| Call heartbeat of an open call socket | 20 s | 20 s | Relay | three heartbeats per time to live, so one late beat loses no seat (`SRV-248`), held by `ambre_server::ws::handler::call::tests::a_call_whose_state_is_lost_ends` at a beat of 1 s and a time to live of 2 s |
| Call roster ask, the interval between two asks of one live call by a reader whose view shows its channel | 5 s | 5 s | Logic | the reader learns who joined or left within it, and a call whose last participant crashed shows ended one time to live and one interval after its last heartbeat at most (`MURMURE-592`, `MURMURE-595`). Each ask costs the relay one read of the state of the call. The interval is held by `ambre_logic::channels::tests::a_tick_asks_who_is_in_each_live_call_once_per_interval`. No run measures the load of the asks on the relay |
| Call ring, the time a call of a direct message rings with no answer, from the instant its call message was written | 30 s | 30 s | UI | the member has the time to reach the page, and the caller waits no longer than a telephone rings (`MURMURE-643`, `MURMURE-644`), held by `ambre_murmure::ui::call_ring::tests::a_ring_cue_of_a_direct_message_rings_a_free_page_until_its_timeout` and by `ambre_murmure::ui::call_ring::tests::a_decline_or_the_timeout_ends_the_ring_of_the_caller`. The page of the member reads the instant on its own clock, so a drift between two clocks moves its end. A hidden tab may fire its timer late. No run measures either |
| Call participants | 50 | 50 | -- | a cap: a join beyond it is refused with a notice (`CALL-021`), held by `ambre_server::ws::handler::call::tests::a_join_at_the_participant_cap_is_refused_with_a_notice`. The SFU refuses one too. A broadcast to hundreds is another product |
| Call videos forwarded to one receiver | 9: one at the top layer its estimate allows, eight at the lowest | 9 | SFU | a cap: the SFU forwards the video of the most recent speakers alone (`CALL-041`). `ambre_sfu::sim::tests::a_receiver_gets_the_video_of_the_most_recent_speakers_alone` holds it. Each receiver holds 9 video slots and one screen slot (`CALL-045`), which carries one screen apart from the 9 (`CALL-104`) |
| Call audio forwarded to one receiver | 3: the most recent speakers | 3 | SFU | a cap: the SFU forwards the audio of the most recent speakers alone, over 3 audio slots (`CALL-047`). `ambre_sfu::sim::tests::a_receiver_gets_the_audio_of_the_three_most_recent_speakers_alone` holds it |
| Call microphone gain set by hand, its highest value | 2 times the captured level | 2 | UI | a cap: past it a voice near the microphone clips in the encoder. The gain runs from silence, 0, and the automatic gain stays the default (`CALL-089`). `ambre_murmure::ui::call_volumes::tests::a_gain_past_its_cap_is_refused` holds the bounds |
| Call microphone meter, the interval between two readings of the level | 100 ms | 100 ms | UI | ten readings a second follow a voice. Each reads a window of 8 192 samples, about 170 ms at 48 kHz, longer than the interval, so no short sound falls between two readings. No run measures its load
| Call speaker, the outline of a participant who speaks: the interval between two readings of the level of each received audio slot, the level of speech, and the hold after the last speech | 100 ms, a level of 0.02, 600 ms | 100 ms, 0.02, 600 ms | UI | the interval follows a voice as the meter does. The level is about -34 dBov on the linear scale the browser reads from the header extension of each packet, above the noise the echo canceller and the noise filter leave. The hold keeps the outline through a pause between two words (`MURMURE-653`). `ambre_murmure::ui::call_videos::tests::a_member_who_speaks_is_outlined_for_the_hold` holds the level and the hold. No run measures the level of a real voice through a browser |
| Call video paint, the UI-thread time of one frame that draws the received videos of a call on the canvas, nine at 30 fps | < 4 ms | -- | UI | the `ambre:video-paint` measure on the page's timeline, one per frame that draws a video, from the start of its paint to its present (`CALL-015`). `ambre_belt::murmure::tests::a_member_draws_the_video_of_another_on_its_canvas` prints its median, its 95th percentile and its maximum over one received video at the top size. On 2026-11-12, by `trunk build --release` then that test under `just test-browser` and `just test-browser-firefox`, on a cloud container of 4 vCPU, Intel Xeon 2.30 GHz, with no GPU: Chrome for Testing 157 over SwiftShader, median 47 ms to 52 ms and p95 66 ms; Firefox ESR 153 under Xvfb, median 0.7 ms and p95 3.0 ms to 3.6 ms. Chrome copies the picture in a call that waits for its software renderer, and Firefox queues the copy for its GPU process. **none exists** on a desktop with a GPU, or for nine videos. `Call client proofs on Android` Story (`ambre-roadmap.md` E5) holds the phone figure. `just call-proof-desktop` measures nine videos on a desktop with a GPU, and `just call-proof-android` on a phone (`ambre-spec-call-proofs.md`). |
| Call video stall, per 10 minutes of one received video: 50 participants at 150 ms of round trip and 2 % loss | < 1 s | -- | Browser | a stall is a gap between two pictures longer than three times the mean gap and longer than the mean plus 150 ms, as the WebRTC statistics count a freeze; a gap of 5 s or more is a pause. Measured on 2026-11-08 by `LOAD_SFU_CPUS=0,1 LOAD_SECONDS=600 taskset -c 2,3 cargo test -p ambre-sfu --release -- --ignored load_of_one_call`, on a cloud container of 4 vCPU, Intel Xeon 2.10 GHz, the SFU pinned to 2 of them and the 50 synthetic participants and their impairment relay to the other 2: 47.2 s per 10 minutes over 441 received videos and 600 s, the worst video 82.0 s. **It fails the target.** The synthetic receiver models the playout of a browser: a picture shows at its capture time plus the least transit plus the larger of one round trip and the 99th percentile of its lateness over 10 s. No browser ran in the load: the 2 vCPU beside the SFU held the participants at 1.36 vCPU. Later on 2026-11-08, with the probe of the next layer step and the 1 ms clock of the simulation, by the same command on the same machine: 11.2 s per 10 minutes, the worst video 20.4 s. **It still fails the target**. `just call-stall-proof`, with `scripts/call_stall_receivers.sh` on a second machine, measures it with real browsers (`ambre-spec-call-proofs.md`). |
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
| Text layout (Parley, dirty paragraph) | < 2 ms | UI or Logic | UI-thread shaping for the in-cell editor case (Trame cells, multi-line), typical < 0.5 ms. **Prose shapes on the UI thread the paragraphs an edit in flight changes, and no more**: a window of lines around the edit, until the layout that answers carries it, and the input method's preview (`features/ambre-spec-prose-text-editing.md section 3`). Pagination shapes in the Logic thread and ships pre-shaped `ParleyRunData`, so this row is a paragraph or two per edit on the UI thread for Prose. A key whose line breaks never fall back on the old ones shapes the rest of its paragraph, and the budget holds that worst case up to 500 characters, a paragraph of prose. A longer one costs what it costs, and nothing is degraded to save it (`PROSE-UI-215`). The CI `instructions` job holds it through `paragraph_shape` in `crates/ambre-prose/benches/instructions.rs` and 0.5-2 ms on the Logic thread (`features/ambre-spec-prose-pagination.md`, `features/ambre-spec-prose-page-rendering.md`) On 2026-11-10, under the fixed key: 1 219 438, 1 219 522 and 1 216 274 in three runs. parley and fontique seed their hashbrown tables from the clock, and the heap those tables leave moves the count by about 0.3 %. On 2026-11-14, with the emoji sequence scan of `CANVAS-090`: 1 220 150 and 1 219 819. The parent commit counted 1 222 450 and 1 220 121 in the same container, through `scripts/instructions.py`. The ceiling is the budget. |
| Content accessibility diff (AccessKit) | < 0.2 ms | UI | OS notifications on the delta only |
| Glyph rasterization (Swash) on a new glyph | ~0 ms nominal | UI | Atlas hit |
| Build the vertex batch (dirty only) | < 1 ms | UI | `QuadRenderer::begin_frame` plus the scene draw. Zero allocation in steady state |
| GPU upload and render pass | < 1 ms | UI | `write_buffer` plus up to 3 draw calls per layer (quads, mask glyphs, color glyphs). Typical 0.3-0.5 ms |
| Antialiasing band, share of a tessellated path | < 15 % of its vertices, < 5 % of the batch build | UI | The silhouette band of `ambre-spec-drawing-surface.md section 2` (`CANVAS-061`). It adds vertices to a buffer and a draw call that already exist, so it is charged against the two rows above and against nothing else: no pass, no dispatch, no attachment (`CANVAS-062`) -- **none exists** |
| Toile board paint, 2 000 visible elements | < 16 ms | UI | The whole frame for a busy board, and the reason the cull is the engine's job (`features/ambre-spec-toile-board-rendering.md`). The CI `instructions` job holds its content walk through `board_paint` in `crates/ambre-toile/benches/instructions.rs`, at most 5 ms of the 16: about 3 million instructions against a ceiling of 4 million (`CANVAS-072`). The tessellation, the upload and the GPU fall outside the count (`CANVAS-005`) On 2026-11-10, read by `python3 scripts/instructions.py` under the `fixed-hash-key` feature and a pinned peer, native release build in a cloud container, the same count in each of three runs: 3 061 069. |
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
| WS `Update` receive and broadcast | < 5 ms p99 | Redis Lua plus PubSub | the load run of the next row reads it as the p99 of `handle_update`. On 2026-11-15, on a cloud container of 4 vCPU: 5.6 ms at 25 typists, 8.8 ms at 100. **It fails the target at every load measured** |
| Write path load of N typists, each at five keys a second. Per key: the signature check, the ordering script, the index row, its share of the hot-tail moves, and the broadcast | 200 active typists per host of 4 vCPU, at a p99 of `handle_update` under 50 ms. The owner set it on 2026-11-15 | the inline insert, which reads the count and the bytes of the inline tail of its document before it writes. At 200 typists it took 87 % of the statement time of Postgres, at 0.58 ms a key. The hot-tail moves took 9 % | measured on 2026-11-15, on `main` at 6cb1ba6, by `LOAD_TYPISTS=<N> cargo test -p ambre-server --release --features db,test-helpers -- --ignored --nocapture typist_load`. The command runs `ambre_server::ws::handler::typist_load::typist_load`, and no CI job runs it. Each typist is one session that calls `handle_update` every 200 ms, at a random phase. Four typists share a document, and one reader per document listens on the PubSub hub. A key is 160 B to 320 B of ciphertext. The run warms up for 10 s, then counts 60 s. The relay runs on a runtime of four workers, with its pool of 20 connections. The machine is a cloud container of 4 vCPU, Intel Xeon 2.10 GHz, 15 GB of memory. It holds the relay, Postgres 16.15 and Redis together, with the defaults of the database tier. By N, the p99 of `handle_update`, then the processor time of Postgres and of the relay: 25 typists, 125 keys/s: 5.6 ms, 0.13 vCPU, 0.07 vCPU. 100 typists, 500 keys/s: 8.8 ms, 0.59 vCPU, 0.25 vCPU. 200 typists, 1 000 keys/s: 41 ms, 1.18 vCPU, 0.41 vCPU. 300 typists: 240 ms, and 1 443 keys left late. From 400 typists, the tier passes 1 880 to 1 940 keys/s, and the queue of each session grows. Every key costs 8.0 Redis commands, 1.09 commits and about 2 090 B of WAL, with no trim in the run. Up to 100 typists, the p50 of a key stays near 2.6 ms, most of it the commit. A pool of 40 or 80 connections at 400 typists passed 1 834 and 1 943 keys/s, against 1 770 for 20, on a runtime of one thread. A `commit_delay` of 1 ms passed 1 689 keys/s. Alone after the run, the read of a tail of 120 rows took 0.15 ms to 0.20 ms. Past the ceiling, the readers heard 97 % of the keys at 400 typists and 53 % at 800, for a cause not known yet. **It holds the target: 41 ms at 200 typists. The tier stops near 380.** The `The inline insert reads no tail` Story (`ambre-roadmap.md`) names the cut |
| Redis Lua clock validation | < 1 ms | Redis round trip | **none exists** |
| `GET` deltas, 100 entries | < 20 ms | Postgres read | **none exists** |
| S3 presigned URL generation | < 50 ms | S3 SDK network I/O | **none exists** |
| Document cold start (snapshot plus N deltas) | < 500 ms | S3 plus Postgres read | **none exists** |
| OIDC auth callback | < 200 ms | Provider token exchange | **none exists** |
| Global S3 flush batch, 15 s cycle | < 2 s for 1000 deltas | Parallel S3 upload | **none exists** |
| Delta index write cost, per accepted delta: the insert of its row, the move to a batch object, and the delete by the trim | 1 680 B of WAL in the short run, a ceiling: the dearest measure plus 2 %. Each story on `updates` lowers it to its new measure plus 2 %. A raise needs a story row that names the new figure and its cause | the write-ahead log (WAL) of each row version and of each index of `updates`, and the full-page images after a checkpoint | The CI `test-db` job holds the ceiling through `just write-cost`, after the db tier, alone on its services (`SRV-259`). The recipe runs `ambre_server::hot_tail::write_cost::write_cost_short_run`, and `scripts/write_cost.py` holds the ceiling. The short run is 4 documents warmed up past the snapshots compaction keeps, then 4 000 deltas and a compaction pass each 500 deltas. It is unpaced, on one connection, with autovacuum off, and opens on a checkpoint. It counts the WAL inside the windows of the three steps and of the snapshot, which no disk changes. On 2026-11-14, ten runs of the debug build of the db tier, on the cloud container below and the same Postgres: 2 632 B to 2 643 B per delta, a spread of 0.41 %, of which 503 B to 512 B of full-page images. Four of the runs shared the machine with a full compile and read inside the same spread. By step: the insert 1 489 B to 1 507 B, the move 818 B to 835 B, the delete 307 B to 311 B, the snapshot 4 B. Each run took about 85 s, 65 s of it the warm-up. The margin of 2 % is about five times the spread. On 2026-11-14, the CI `test-db` job of run 43650328641 read 2 676 B per delta, of which 539 B of full-page images, in 27 s: 1.2 % over the dearest local run, with 1 254 distinct blocks modified against 1 181. The cause is not known yet, so the ceiling was that CI reading plus 2 %: 2 730 B. Each row of `updates` then carried a random key that no query read, and the primary key on it wrote a random page per insert. On 2026-11-15, `updates` is keyed by its document and its number, and the index on the same two columns is gone (`SRV-263`). By `just write-cost 5` on the same container, before: 2 638 B, 2 648 B and 2 639 B per delta in the three runs that finished, of which 507 B to 514 B of full-page images. After: 2 316 B to 2 327 B, a spread of 0.45 %, of which 348 B to 352 B of full-page images. By step: the insert 1 283 B to 1 292 B, the move 727 B to 746 B, the delete 288 B to 293 B, the snapshot 4 B. That is 12 % less, and the spread did not narrow, so the random key did not cause it. On 2026-11-15, with two snapshots kept in place of ten (`SRV-264`), five runs on the same container: 2 383 B to 2 397 B per delta, a spread of 0.57 %, of which 278 B to 284 B of full-page images. By step: the insert 1 392 B to 1 411 B, the move 769 B to 776 B, the delete 212 B to 213 B, the snapshot 3 B. Three runs of the branch point, the same day, read 2 638 B to 2 644 B: the gain is 9 %. The distinct blocks modified per delta fell from 0.30 to 0.23. Each run took about 23 s, 11 s of it the warm-up. On 2026-11-15, with both changes, by `just write-cost 5` on the same container: 2 197 B to 2 228 B per delta, a spread of 1.44 %, of which 244 B to 271 B of full-page images. By step: the insert 1 292 B to 1 328 B, the move 695 B to 702 B, the delete 201 B to 202 B, the snapshot 3 B. The two dearest runs also wrote 7 B to 11 B between the steps. The CI `test-db` job of the branch that keeps two snapshots read 2 383 B, inside the local spread of that branch. On main, the CI `test-db` job of run 43726544175 merged both changes and read 2 191 B per delta, of which 244 B of full-page images, with 0.210 distinct blocks modified per delta. By step: the insert 1 289 B, the move 697 B, the delete 202 B, the snapshot 3 B. The next two runs of main, 43768828218 and 43771488156 on 2026-11-15, read 2 194 B and 2 199 B. CI now reads under the dearest local run, by 1.7 %, where it read 1.2 % over it on 2026-11-14. The ceiling is the CI reading of run 43726544175 plus 2 %, rounded up to ten bytes: 2 240 B. The dearest local run of 2026-11-15 sits 0.5 % under it. On 2026-11-16, the cause of the CI gap is named. The run reads the WAL of the whole cluster, and autovacuum still visits the catalogs of every database. An autoanalyze that lands in a window charged its writes to `pg_statistic` and `pg_class` to a step: 7 B to 50 B per delta. The catalogs of the test database, written by the migrations, draw the worker in any run. In CI, the db tier also churns the register of `#[sqlx::test]` in `ambre_dev` just before the step. The CI reading of run 43650328641 holds at least 34 B per delta in blocks of another database, which covers its gap of 33 B. The 7 B to 11 B between the steps of the two dearest runs of 2026-11-15 are the same writes. The run now charges to no step a transaction that writes none of its relations, nor a record of no transaction that writes only blocks of other relations. By `just write-cost 5` on the same container, five runs on a quiet cluster and five after `just test-db`: 2 190 B to 2 200 B per delta, a spread of 0.44 %, of which 243 B to 246 B of full-page images. Six of the ten runs held 7 B to 50 B of foreign WAL, left out. The CI `test-db` job of run 43771488156, on `main` the day before, read 2 194 B, inside that spread. On 2026-11-16, with no foreign key from `updates` to `documents` (`SRV-274`), by `just write-cost 5` on the same container: 2 135 B to 2 177 B per delta, of which 242 B to 273 B of full-page images. By step: the insert 1 228 B to 1 266 B, the move 702 B to 712 B, the delete 200 B to 203 B, the snapshot 3 B. Five runs of the branch point, the same day, read 2 188 B to 2 242 B, the insert 1 289 B to 1 335 B: the gain is 2.7 %, about 60 B per insert. The ceiling stays as it is in this change. On 2026-11-16, with a trigger that removes the delta rows of a deleted document (`SRV-275`), by `just write-cost 5` on the same container: 2 133 B to 2 185 B per delta, the insert 1 230 B to 1 278 B. Five runs of its branch point, the same day, read 2 137 B to 2 186 B, the insert 1 234 B to 1 279 B: the trigger fires on a delete alone, and the insert is unchanged. On 2026-11-16, measured on its own branch point, before the two changes above: the partial index on the inline rows is gone, and the inline tail count reads the primary key above the newest number in a batch object (`SRV-150`). By `just write-cost 5` on the same container, with the index: 2 194 B to 2 199 B per delta, a spread of 0.24 %, of which 244 B to 247 B of full-page images, and the insert 1 287 B to 1 301 B. Without it: 2 089 B to 2 133 B, of which 239 B to 268 B of full-page images, and the insert 1 187 B to 1 228 B. The move and the delete did not change. The index cost about 100 B per delta, 4.6 %, all of it in the insert. The dearest run without it, 2 133 B, was the first after the build and wrote 7 B between the steps, as the dearest runs of 2026-11-15 did. A new index on `updates` costs more than the margin, so it fails the ceiling. `just write-cost 5` repeats the run and holds the dearest, for a story that measures its gain. `ambre_server::hot_tail::write_cost::write_cost_per_delta`, the by-hand run, drives the same functions at a fixed rate. It reads the WAL through `pg_walinspect` and the block writes through `pg_stat_io`, over one cycle between two checkpoints. No CI job runs it. Measured on 2026-11-13 by `cargo test -p ambre-server --release --features db -- --ignored --nocapture write_cost_per_delta`, with `LOAD_SEED` 1, 2 and 3. The run is 32 documents at 200 deltas a second for 240 s, after a warm-up of 5 750 deltas per document, and a compaction pass every 30 s. A delta is 120 B to 900 B, and one in 200 is 4 KB to 32 KB. The machine is a cloud container of 4 vCPU, Intel Xeon 2.70 GHz, 15 GB of memory, on a virtio disk of unknown type. Postgres is the 16.15 of the database tier, with its defaults: `full_page_writes` on, `wal_compression` off, `checkpoint_timeout` 300 s, `shared_buffers` 128 MB, `synchronous_commit` on. Over the cycle: 2 901 B to 2 966 B of WAL per delta, of which 690 B to 761 B are full-page images, and 0.27 to 0.28 block writes per delta. By step, per delta: the insert 1 729 B to 1 818 B, the move 795 B to 820 B, the delete 288 B to 332 B, the snapshot 1 B. The first 30 s after the checkpoint cost 3 717 B to 3 964 B per delta. A 30 s with no compaction pass and few new pages costs 2 220 B to 2 269 B. The heap of `updates` takes 0.15 of the 0.25 distinct blocks per delta that the checkpoint writes. A container predicts the ratios between steps and the WAL per delta, which the disk does not change. It predicts no throughput: no per-user delta rate is in the specs. On 2026-11-16, measured on its own branch point, with the pass period of section 6 (`SRV-277`) and the trim split into three deletes, by `just write-cost` then `just write-cost 3` on the same container: 2 217 B, then 2 193 B to 2 198 B per delta, of which 243 B to 264 B of full-page images. By step: the insert 1 285 B to 1 318 B, the move 698 B to 706 B, the delete 199 B to 201 B, the snapshot 3 B. The short run passes each 500 deltas whatever the period, so the figure stays inside the spread of 2026-11-15, and the ceiling stays. On 2026-11-17, with every change of 2026-11-16 above on one branch: no foreign key on `updates` (`SRV-274`), the trigger (`SRV-275`), the move written in place with no partial index (`SRV-276`), the trim split into three deletes (`SRV-277`) and the autovacuum WAL left out, by `just write-cost 5` on a cloud container of 4 vCPU and the same Postgres: 1 639 B to 1 643 B per delta, a spread of 0.22 %, of which 240 B to 246 B of full-page images. By step: the insert 1 133 B to 1 139 B, the move 298 B to 299 B, the delete 202 B to 206 B, the snapshot 3 B. Two of the runs left out 55 B of foreign WAL. The move fell from about 700 B to 298 B, and the insert by about 150 B. The ceiling falls to the dearest run plus 2 %, rounded up to ten bytes: 1 680 B. No CI reading of this branch exists yet |
| Presence write, per heartbeat | < 2 ms | Two Redis round trips: the entry, then its own expiry | **none exists** |
| SFU load, one call of 50 participants | < 2 vCPU | packet forwarding and the layer choice per receiver | measured on 2026-11-08 by `LOAD_SFU_CPUS=0,1 LOAD_SECONDS=600 taskset -c 2,3 cargo test -p ambre-sfu --release -- --ignored load_of_one_call`, on a cloud container of 4 vCPU, Intel Xeon 2.10 GHz, the SFU pinned to 2 of them and the 50 synthetic participants and their impairment relay to the other 2, at 150 ms of round trip and 2 % loss: 1.51 vCPU over 600 s. Earlier the same day, before the SFU spread its connections over threads, one thread saturated at 0.99 vCPU and the transit reached 7.5 s at p99. The pacer of str0m costs the most: it reads every send stream of a connection at each packet it sends. Later on 2026-11-08, with the congestion control of Ambre's own and 12 slots per receiver, on a cloud container of 4 vCPU, Intel Xeon 2.30 GHz, by the same command: 0.87 vCPU over 600 s, and a speech transit of 161 ms at p95. Later the same day, with the layer choice run at once on a lower target: 0.89 vCPU, and 161 ms at p95. Later on 2026-11-08, with the probe of the next layer step, by the same command on a cloud container of 4 vCPU, Intel Xeon 2.10 GHz: 1.04 vCPU over 600 s, and 162 ms at p95 |
| SFU layer drop, from a cap on the downlink of a receiver to a lower layer at that receiver | < 3 s | the estimate of the downlink, from the transport-wide feedback of the receiver | `ambre_sfu::sim::tests::a_capped_receiver_drops_a_layer_within_the_budget` measures it in simulated time, at 150 ms of round trip and 2 % loss, and no CI job runs it (`CALL-038`). On 2026-11-08, alone on the machine: 20 runs from 0.87 s to 2.33 s. Beside three other runs: 4 of 21 above 3 s, the slowest 4.0 s. **It fails the target under load.** In the load run of 50, no receiver held a layer above the lowest under 2 % loss, so the cap found no layer to drop there. Later on 2026-11-08, with the congestion control of Ambre's own and a congested drop that stops the current layer, the run is seeded and runs in the CI native tier over eight seeds. By `cargo test -p ambre-sfu --release a_capped_receiver -- --nocapture`, on a cloud container of 4 vCPU, Intel Xeon 2.30 GHz: from 0.82 s to 1.13 s. The same seeds on the estimate of str0m: from 0.83 s to 3.19 s. In the load run of 50 above, the capped receiver dropped after 1.58 s. Later on 2026-11-08, the layer choice runs at once on a lower target, and the SFU pads nothing under overuse. On the same machine, by the same two commands: from 0.68 s to 0.85 s over the eight seeds, and 0.87 s in the load run of 50. Later on 2026-11-08, with the probe of the next layer step and a simulated clock of 1 ms, on a cloud container of 4 vCPU, Intel Xeon 2.10 GHz, by the same two commands: from 0.72 s to 0.78 s over the eight seeds, and 0.88 s in the load run of 50 |
| SFU layer recovery, from a delay step with no lasting congestion to the top layer at that receiver | < 2 s | the end of the overuse, one probe of the next layer step, then the keyframe of the top layer | `ambre_sfu::sim::tests::a_receiver_climbs_back_to_the_top_layer_after_a_delay_step` measures it in simulated time, at 150 ms of round trip and 2 % loss, with a step of 40 ms on the one-way delay of the downlink, over eight seeds in the CI native tier (`CALL-050`). On 2026-11-08, by `cargo test -p ambre-sfu --release a_receiver_climbs_back -- --nocapture`, on a cloud container of 4 vCPU, Intel Xeon 2.10 GHz: from 1.12 s to 1.29 s. Of that time, the overuse of the step lasts 0.45 s to 0.53 s, the probe takes 0.35 s to 0.40 s, and the keyframe of the top layer arrives 0.25 s to 0.49 s later. Before the probe, the same seeds held the middle layer for over 7 s |
| SFU switch to a chosen sender, from the choice the SFU reads to the first packet of that sender at the receiver | < 500 ms at 150 ms of round trip | one keyframe request to the sender and its keyframe back, one round trip | `ambre_sfu::sim::tests::a_chosen_sender_is_asked_a_keyframe_and_shows_within_the_budget` measures it in simulated time, with no loss, for two senders outside the nine a receiver of a call of twelve gets (`CALL-129`). On 2026-11-16, by `cargo test -p ambre-sfu --release a_chosen_sender_is -- --nocapture`, on a cloud container: 245 ms and 246 ms. The page and the Sync worker add the trip of the choice over the socket of the SFU, which no run measures |

---

## 4. OPENING A LARGE DOCUMENT -- A COMPOSITE OPERATION WITH NO END-TO-END TARGET

* **Every leg is budgeted, the whole is not.** Each row of section 1 budgets one leg; none budgets
  the chain, and two legs are unbudgeted entirely.

| Leg of an open | Stated budget |
|---|---|
| 1. Server read (snapshot plus N deltas from S3 and Postgres) | < 500 ms, section 3 |
| 2. Client AEAD open plus zstd decompress of a full snapshot | **none** -- section 1 budgets compression of 100 KB and inbound decryption per 1 KB |
| 3. `LoroDoc` import of a full snapshot | **none** -- `doc.import` in section 1 is < 2 ms for a **1 KB delta**, which says nothing about a 10 MB snapshot. A full-state read imports it **twice**: once into a scratch document to read the schema register, once into the live one (`ambre-spec-schema-version.md section 2`). Deliberate, and it is what makes a refused document cost the user nothing |
| 4a. Engine rebuild after import: the Trame `SheetIndex` | < 10 ms for 10k rows, section 1 |
| 4b. Engine rebuild after import: the Trame reseed and the Toile `SpatialIndex` bulk load | **none** -- Trame's `import_snapshot` triggers the **full reseed**, which re-parses and recompiles every formula of every sheet (`ambre-spec-trame-engine.md`), and is likely dominant at 100k cells. The `SpatialIndex` bulk load of Toile has no budget either |
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
  compile and boot. The boot spawns no converter thread: the first job spawns it (`ARCH-112`).
* **The budgets count brotli, as served.** The release build writes a brotli copy at quality 11
  beside each compressible file (`scripts/trunk_post_build.py`). A static server sends it as it is,
  with no compression per request. On the wasm it weighs 27% less than gzip at level 9.
* **The ceilings count raw wasm bytes since 2026-11-11 (`ARCH-096`).** Two builds of one source
  differ by about 850 bytes of brotli on Main, and by about 63 raw bytes: see the noise band
  below. So a brotli ceiling passed or failed on noise. `scripts/wasm_size.py` still prints the
  brotli figure of each artefact, for information. The figures before that day stay in brotli,
  as written.
* **The noise band closed on 2026-11-14.** The `view!` macro of leptos emitted slots in the order
  of a map seeded per process, so each compile of Main wrote other bytes. The vendored copy fixes
  that order (`vendor/leptos_macro/PATCH.md`), and `just reproducible` read one client digest
  from two clean builds (`ZK-038`). The ceilings keep their floor of 1 000 bytes of room.
* **Cached for good after the first visit.** Every built file is named by a hash of its content
  (`filehash` in `Trunk.toml`), so a deployment caches it as immutable. Only the page is asked
  again, and a release downloads only the instance it changed.
* **The first download holds the budget.** On 2026-10-20 a release build weighed 3.07 MB brotli
  for Main and 0.34 MB for Sync. On 2026-11-07, `just size` read 3 128 174 bytes for Main and
  352 134 for Sync. The `wasm-release` profile builds every crate for size and strips the name
  section, which took Main from 6.9 MB to 4.27 MB gzip.
* **The build fails on the product budgets alone since 2026-11-15 (`ARCH-096`).** Until that day,
  each artefact held a raw ceiling a few hundred bytes above its measure. A feature that grew a
  module or a worker failed `just size`, and needed a raise story. So the ceilings rose one after
  another, on growth no user sees. The owner set the rule that day. `just size` fails on three
  figures alone:
  1. the first download, Main plus Sync, under 4 MB brotli;
  2. each lazy artefact, a converter module or the call worker, under its own brotli budget;
  3. the raw wasm of Main under a ceiling set at least 2 % above its measure.
* **The brotli budgets are wide, so the brotli noise cannot move them.** Brotli moves Main by
  about 850 bytes between two builds of one source, and the first download holds about 856 KB of
  room. A tight brotli figure would pass or fail on that noise, which is why `ARCH-096` once moved
  the ceilings to raw bytes.
* **The Main ceiling is the net against growth on the first download.** It holds raw bytes, which
  move by about 63 bytes between two builds. It sits 2 % above the measure of `main`, rounded up to
  10 000. That is room for about 200 KB of raw growth, about 58 KB of brotli. A story that takes
  Main past it raises it through a row, which names the new figure and the cut tried first. The
  new ceiling sits 2 % above the new measure again (`ARCH-095`).
* **Every push prints the change of each artefact against `main` (`ARCH-115`).** The `size` CI job
  runs in the fast lane. A run of `main` saves its measure in the CI cache. A branch reads the
  measure of its merge base, or else the newest one, and the report names it by its commit. The
  report prints the raw and brotli bytes of each artefact and their change. The log and the run
  summary carry it, and no figure of it fails.
* **The figures of 2026-11-15.** `trunk build --release` then `scripts/wasm_size.py`, on the
  session container, over `main` at `6fe7e50`. The download time is the figure at 20 Mbit/s, the
  4G rate of the bullet above.

| Artefact | Raw | Brotli | Fails on | Room | 4G at 20 Mbit/s |
|---|---|---|---|---|---|
| Main | 10 442 998 | 2 773 165 | raw ceiling 10 920 000 | 477 002 raw, 4.6 % | -- |
| Sync | 1 269 817 | 370 575 | -- | -- | -- |
| First download | -- | 3 143 740 | brotli budget 4 000 000 | 856 260, 21 % | 1.26 s |
| Tables module | 122 422 | 43 595 | brotli budget 100 000 | 56 405, 56 % | 0.02 s |
| PDF module | 1 822 699 | 425 324 | brotli budget 600 000 | 174 676, 29 % | 0.17 s |
| Markup module | 868 158 | 246 446 | brotli budget 300 000 | 53 554, 18 % | 0.10 s |
| Images module | 813 828 | 253 750 | brotli budget 300 000 | 46 250, 15 % | 0.10 s |
| Office module | 1 457 724 | 394 037 | brotli budget 900 000 | 505 963, 56 % | 0.16 s |
| Call worker | 107 452 | 37 181 | brotli budget 100 000 | 62 819, 63 % | 0.01 s |

* **The two spelling modules joined on 2026-11-16.** Each holds `spellbook` and one Hunspell
  dictionary, measured as `scripts/converter_build.py` builds them. The English module reads
  830 626 raw and 243 424 brotli, the French one 1 704 676 raw and 363 308 brotli. The dictionary
  is most of each: 166 KB and 287 KB of brotli for the files alone. Their budgets are 300 000 and
  450 000 bytes of brotli, by the The spell checker Story (`ambre-roadmap.md`).

* **A budget or the Main ceiling rises only through a story of its own (`ARCH-095`).** The ceiling
  of Main rose twice in a row, to 2.73 MB and then 2.74 MB, each time inside the merge of a feature
  story. A raise needs a story row of the roadmap or of the journal that names the new figure and
  the cut tried first. The `lint` CI job runs `scripts/ceiling_raise.py`, which compares the Main
  ceiling and each budget with its value on `origin/main`. Lowering stays free, and a figure the
  report alone prints needs no row.
* **A rename and a new artefact are read too (`ARCH-095`).** A renamed artefact meets the figure
  of its old name, and a new artefact needs a story row that names it and its figure. The bullets
  below that name a raise still state the crate that grew and why.
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

* **The Office module joined on 2026-11-10, and Main kept its 2.72 MB ceiling.** The Word codec,
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
| Main | 2 716 043 | 2 713 117 | -2 926 | the Markdown and HTML-paste mappings left; the Word entries, the review builder and the form's serde code came in |
| Sync | 355 413 | 355 668 | +255 | one digest and one module name more |
| First download | 3 071 456 | 3 068 785 | -2 671 | Main plus Sync |
| Tables module | 43 565 | 43 993 | +428 | the new variants of the request and answer types |
| PDF module | 413 216 | 413 306 | +90 | the same |
| Markup module | 204 510 | 239 785 | +35 275 | the Markdown mapping, the HTML-paste walk, the form's serde code and the float parser the walk reads a style with |
| Images module | 220 949 | 221 239 | +290 | the new variants of the request and answer types |
| Office module | -- | 253 527 | new | `ambre-docx`, `ambre-opc`, `zip`, `quick-xml`, the Word mapping, and `jiff` with the IANA database a Word date is read under |

* **The xlsx mapping joined the Office module on 2026-11-10, and Main kept its 2.72 MB ceiling.**
  The Excel codec reads and writes in the module, and maps the model to and from Trame's form, the
  `Book` of `ambre-xlsx`. A style crosses as the bytes Trame stores, so Main decodes no Excel style.
  Main keeps the sliced import, the sort of each formula and the walk of the export.
* **Main made room by sharing two image paths.** Toile's picture thumbnail dropped `thumbnail`, 18 KB
  of code, for the RGBA resize Murmure's preview runs and the PNG encoder the WebP path runs. Eleven
  `sort` calls over values whose equal elements are identical became `sort_unstable`. `trunk build
  --release` then `scripts/wasm_size.py`, on the session container, read the figures below. The
  figure before is the same build of `origin/main`.

| Artefact | Before | After | Change | Why |
|---|---|---|---|---|
| Main | 2 716 508 | 2 718 808 | +2 300 | the xlsx import and export entries, the form's serde code and the hidden sheets came in; the thumbnail resize and eleven stable sorts left |
| Office module | 253 731 | 321 908 | +68 177 | `ambre-xlsx` with its codec, and the map between the model and Trame's form |
| Main, after the merge of the call client | 2 725 968 | 2 727 284 | +1 316 | the same change over `origin/main` at `ba6d544`, against its 2.73 MB ceiling |

* **The PowerPoint writer joined the Office module on 2026-11-11, and Main shrank.** The module
  writes the deck Toile maps, and the deck crosses as postcard bytes, so no other module links its
  serde code. The board walk, the map and the deck's serde code came into Main.
* **Main paid for them with its log stack.** A dedicated subscriber, `ambre-app/src/console_trace.rs`,
  replaced the `tracing-subscriber` registry and the `tracing-wasm` layer: the same filter and
  the same console line. Every call site stays: the cut of the call sites is still out, as the
  table of compile-time cuts below states.
* **The method.** `trunk` was absent from the session container. `cargo build --profile
  wasm-release` of each artefact, `wasm-bindgen` 0.2.122 on Main, then brotli at quality 11 read
  the figures below. The figure before is the same build of `origin/main`. This build of
  `origin/main` at `3dc915c` reads Main at 2 728 222, where `just size` read 2 727 621. At
  `fb8ac9e`, it reads 2 733 129: past the ceiling before this change.

| Artefact | Before | After | Change | Why |
|---|---|---|---|---|
| Main, PPTX export wired, over `3dc915c` | 2 728 222 | 2 733 399 | +5 177 | the board walk, the map, the deck's serde code, the menu entry and the command |
| Main, with the dedicated subscriber, over `fb8ac9e` | 2 733 129 | 2 730 874 | -2 255 | the PPTX export came in; `tracing-subscriber`, `tracing-wasm`, `sharded-slab` and `thread_local` left |
| Tables module | 44 155 | 44 184 | +29 | the new variant of the request |
| PDF module | 413 257 | 413 029 | -228 | the same |
| Markup module | 239 949 | 240 360 | +411 | the same |
| Images module | 221 529 | 221 655 | +126 | the same |
| Office module | 321 822 | 332 922 | +11 100 | the PowerPoint writer of `ambre-pptx` and the deck's serde code |

* **The OpenDocument text codec joined the Office module on 2026-11-11, and Main kept its
  ceiling.** `ambre-odt` reads and writes an `.odt` onto the Word model. So it adds no mapping, and
  shares the zip, the XML parser and the Word mapping of the module. Main gains three palette
  entries and the request variants.
* **The method.** `trunk build --release` then `scripts/wasm_size.py`, on the
  session container, read the figures below. The figure before is the same build of `8211e1c`,
  the OpenDocument spreadsheet merged and the text codec not wired. The Office ceiling rises to
  400 KB, the next 10 KB above the figure, the spreadsheet codec included.

| Artefact | Before | After | Change | Why |
|---|---|---|---|---|
| Main, raw wasm, over `8211e1c` | 10 457 687 | 10 458 140 | +453 | the import, the export and the reviewed export entries, and the two request variants |
| Main, brotli, over `8211e1c` | 2 734 704 | 2 735 398 | +694 | the same |
| Office module, brotli, over `8211e1c` | 363 319 | 392 417 | +29 098 | `ambre-odt`: its reader, its writer and its XML tree over the shared package |

* **The subscriber took contract rows on 2026-11-11, and lost its timing code in a release.** The
  variant of a debug build exists in a debug binary alone, so a release links no timing mark
  (`ARCH-092`). A span prints a line that refuses it (`ARCH-093`). `trunk build --release` then
  `scripts/wasm_size.py`, on the session container, read the figures below.
* **Brotli at quality 11 is not monotonic on Main.** A smaller raw wasm can compress larger, by
  several hundred bytes. The raw figure is the one a change controls.

| Artefact | Before | After | Change | Why |
|---|---|---|---|---|
| Main, raw wasm, over `2fc5618` | 10 441 439 | 10 441 090 | -349 | the timing code left the release; the line of a refused span came in |
| Main, brotli, over `2fc5618` | 2 730 026 | 2 730 455 | +429 | the same change; the figure before is already 26 bytes past the ceiling |

* **The OpenDocument spreadsheet codec joined the Office module on 2026-11-11, and Main kept its
  ceiling.** The codec, its formula translation and its number styles run in the module alone, over
  the zip and XML readers it already linked. Main gained the two commands, the two menu entries and
  the format names. The method is `trunk build --release` and `scripts/wasm_size.py`, on the branch
  and on `origin/main` at `f498f34`.

| Artefact | Before | After | Change | Why |
|---|---|---|---|---|
| Main | 2 734 840 | 2 735 238 | +398 | the OpenDocument import and export commands, the menu entries and the format names |
| Office module | 332 922 | 363 195 | +30 273 | the OpenDocument codec of `ambre-xlsx`, its formula translation and its number styles, and the two conversions |

* **The PowerPoint reader joined the Office module on 2026-11-14, past its raw ceiling.** The reader
  of `ambre-pptx` flattens a deck in the module. `zune-jpeg` decodes the thumbnail of each JPEG
  picture there, since Main links no JPEG decoder. The brotli figure stays far under the 900 KB
  budget. The raw figure passes the 1 480 000 ceiling, which rises to 1 660 000 through its own story row.
* **The method.** `cargo build --profile wasm-release` of the office family, as
  `scripts/converter_build.py` runs it, then brotli at quality 11, on the session container. The
  figure before is the same build of `4691a55`. The cut tried first left the JPEG decode out.

| Artefact | Before | After | Change | Why |
|---|---|---|---|---|
| Office module, raw, over `4691a55` | 1 456 192 | 1 654 287 | +198 095 | the reader, its XML tree and its style chain; `zune-jpeg` and the thumbnail decode |
| Office module, brotli, over `4691a55` | 393 808 | 442 630 | +48 822 | the same |
| Office module, raw, without the JPEG decode | 1 456 192 | 1 528 559 | +72 367 | the reader alone: the cut tried first, still past the ceiling |

* **One budget per converter module.** A row below is the budget of one module: what its family
  may weigh with the formats the roadmap announces in it, never today's figure. `wasm_size.py`
  holds each module to its brotli budget alone, and adds none to the first download. A budget
  rises through a story row (`ARCH-095`), with the crate named here. On 4G at 20 Mbit/s, the
  largest budget, 900 KB, takes 0.36 s once.

| Module | Budget, brotli | Budget on 4G at 20 Mbit/s | Measured 2026-11-15 | Formats today | Formats the roadmap announces | Method |
|---|---|---|---|---|---|---|
| Tables | < 100 KB | 0.04 s | 43 595 | CSV, read and write | TSV; xlsx and ODS live in the Office family | `scripts/wasm_size.py`, run by `just size` |
| PDF | < 600 KB | 0.24 s | 425 324 | PDF, write | a selectable text layer, the Prose and Trame exports through the same writer | `scripts/wasm_size.py`, run by `just size` |
| Markup | < 300 KB | 0.12 s | 246 446 | Markdown and HTML, read and write; HTML, read for a paste | -- | `scripts/wasm_size.py`, run by `just size` |
| Images | < 300 KB | 0.12 s | 286 364 | SVG, WebP, JPEG, PNG and GIF, read | -- | `scripts/wasm_size.py`, run by `just size` |
| Office | < 900 KB | 0.36 s | 394 037 | DOCX, xlsx, ODS and ODT, read and write; PPTX, read and write | ODP, which joins this module: one ZIP reader and one XML parser for every office codec | `scripts/wasm_size.py`, run by `just size` |

* **The call worker joined on 2026-11-10, as a third artefact outside the first download.** The
  page spawns it on a join, and the browser fetches it then, once. It seals and opens the chunks of
  a call, so it links the chunk seal of `ambre-crypto` and no socket. That day, `trunk build
  --release` then `scripts/wasm_size.py`, on the session container, read the figures below. The
  figure before is the same build on the parent commit `fc030fb`. Sync already read 8 455 bytes over
  its ceiling there: the key flow of a call, merged without a new ceiling.
* **The ceilings rose with it.** Main rose to 2.73 MB and Sync to 0.38 MB. The growth of each is
  the feature itself, so each ceiling keeps under 10 KB of room above its figure.

| Artefact | Before | After | Change | Why |
|---|---|---|---|---|
| Main | 2 718 471 | 2 726 278 | +7 807 | the call client of the UI thread, the WebRTC bindings of `web-sys`, and the envelopes of a call in `ambre-bus` |
| Sync | 368 455 | 370 383 | +1 928 | the join command, the `meta` read of the channel and the port of the call worker |
| Call worker | -- | 35 300 | new | the chunk seal and open of `ambre-crypto`, ChaCha20-Poly1305 and HKDF-SHA256 |

| Artefact | Budget, brotli | Budget on 4G at 20 Mbit/s | Measured 2026-11-15 | Carries today | Method |
|---|---|---|---|---|---|
| Call worker | < 100 KB | 0.04 s | 37 181 | the audio of a call | `scripts/wasm_size.py`, run by `just size` |

* **The call controls joined Main on 2026-11-11, and its ceiling rose to 2.74 MB.** The call
  button, mute and hang up live in the header (`ambre-app/src/shell/call_bar.rs`), with three
  `icondata` icons and the refusal of a microphone. On the session container, `trunk build
  --release` then `scripts/wasm_size.py` read the figures below. The figure before is the same
  build on `8f4b5ec`, which already read past the ceiling of 2.73 MB on that container. The growth
  is the feature itself, so the ceiling keeps under 10 KB of room above it.

| Artefact | Before | After | Change | Why |
|---|---|---|---|---|
| Main | 2 732 285 | 2 736 131 | +3 846 | three inlined SVG icons, the call bar view, the call controls of `ambre-murmure`, the refusal and mute paths |

* **Main took three cuts on 2026-11-11, and kept its 2.74 MB ceiling.** No behaviour changed, and
  each cut removes code that another path already links:
  * A Murmure draft row decodes through the slice reader of `serde_json`. The JSON update of a
    delta already links that reader, so the `str` reader left.
  * The `wasm-release` profile sets `trim-paths`. A panic location keeps its file and its line,
    and its path starts at the crate, without the registry path of the build machine. The
    `location-detail` cut, left out below, drops the location itself.
  * The callers of one sort order share one comparator, in `ambre_core::order` when two crates
    share it. The standard sort links one copy per comparator, and a copy weighs about 3.5 KB.
* **The method.** `trunk build --release` on the session container read the figures below. The
  first two rows build over `fb2533c`, the third over the merge of `origin/main` at `e5d3f19`.
  That commit alone reads 10 456 635 raw and 2 733 984 brotli.

| Artefact | Before | After | Change, raw | Change, brotli | Why |
|---|---|---|---|---|---|
| Main, the draft row | 10 441 141 raw, 2 730 732 brotli | 10 432 619 raw, 2 729 883 brotli | -8 522 | -849 | the `str` reader of `serde_json` left |
| Main, `trim-paths` | 10 432 619 raw, 2 729 883 brotli | 10 406 238 raw, 2 728 772 brotli | -26 381 | -1 111 | 651 paths of the registry and the toolchain left the panic locations |
| Main, the shared sorts | 10 421 819 raw, 2 733 612 brotli | 10 403 493 raw, 2 732 720 brotli | -18 326 | -892 | the duplicate copies of the standard sort left |
| Sync, `trim-paths` | 374 785 | 374 593 | -- | -192 | the same paths |

* **Brotli on Main moves by 850 bytes when the code does not.** On 2026-11-11, the session
  container built the head of the three cuts four more times with `trunk build --release`. One
  build followed `cargo clean -p ambre-app`. Three builds each changed one letter of one log text,
  at the same length. The raw wasm read from 10 403 458 to 10 403 521 bytes. The brotli read from
  2 732 378 to 2 733 228.
* **The same source does not give the same bytes twice.** The clean rebuild read 28 raw bytes and
  405 brotli bytes more than the first build of the head.
* **So a cut under about 1 KB of brotli cannot be read on one build.** The raw figure is the one a
  change controls. The whole wasm compresses to 26% of its raw size. The three cuts above removed
  code that repeats other code, and their brotli gain was 4% to 10% of their raw gain.

* **The device choice of a call joined Main on 2026-11-11, and its ceiling rose to 2.755 MB.** The
  device panel of the call bar, the change of device in `ambre-app/src/call.rs` and the
  `MediaDeviceInfo` bindings of `web-sys` came in. On the session container, `trunk build
  --release` then `scripts/wasm_size.py` read the figures below. The figure before is the same
  build on `54ee84e`. The growth is the feature itself, so the ceiling keeps under 10 KB of room
  above it.

| Artefact | Before | After | Change | Why |
|---|---|---|---|---|
| Main | 2 734 988 | 2 746 974 | +11 986 | the device panel, its two views and its selects; `enumerateDevices`, `replaceTrack`, `setSinkId` and the `devicechange` and `ended` listeners; the `MediaDeviceInfo`, `MediaDeviceKind` and `MediaStreamTrackState` bindings |

* **The ceilings moved to raw bytes on 2026-11-11.** Each ceiling is the raw figure rounded up to
  the next multiple of 10 000. When that leaves under 1 000 bytes of room, it takes the next one.
  That is the rule of the brotli ceilings, with a floor above the 63-byte noise band. The cut is
  not applicable: the change is one of unit, and no code moved. `trunk build --release` then
  `scripts/wasm_size.py`, on the session container, read the figures below on `origin/main` at
  `e3d6557`.

| Artefact | Raw | Raw ceiling | Brotli | Brotli ceiling before |
|---|---|---|---|---|
| Main | 10 463 059 | 10 470 000 | 2 745 742 | 2 755 000 |
| Sync | 1 300 477 | 1 310 000 | 374 853 | 380 000 |
| Tables module | 121 651 | 130 000 | 44 171 | 50 000 |
| PDF module | 1 785 317 | 1 790 000 | 413 058 | 420 000 |
| Markup module | 842 826 | 850 000 | 240 184 | 250 000 |
| Images module | 685 735 | 690 000 | 221 650 | 230 000 |
| Office module | 1 471 463 | 1 480 000 | 392 700 | 400 000 |
| Call worker | 100 164 | 110 000 | 35 285 | 40 000 |

* **The volumes of a call joined Main on 2026-11-11, and its ceiling rose to 10 510 000 raw
  bytes.** The `ambre-app` crate grew by the volumes panel of the call bar and the slot and gain
  paths of `ambre-app/src/call.rs`. The Web Audio graph of the gain and its `web-sys` bindings
  came with them. The
  `ambre-murmure` crate added the playout model and the codec of the stored volumes. Five cuts
  came first, each read by `trunk build --release` then `scripts/wasm_size.py` on the session
  container. The figure before is `origin/main` at `9fb2de9` on that container.

| Build | Main raw | Change | What |
|---|---|---|---|
| `origin/main` at `9fb2de9` | 10 463 056 | -- | before the story |
| the story, before any cut | 10 530 694 | +67 638 | the panel, the slots, the graph |
| cut 1: one slider component for the volumes and the gain, and one boxed call of the debouncer | 10 527 808 | -2 886 | kept |
| cut 2: vectors in place of the maps of the playout and the stored volumes | 10 516 864 | -10 944 | kept |
| cut 3: a text codec of its own for the stored volumes and gain, in place of JSON | 10 506 175 | -10 689 | kept |
| cut 4: the panel reads the signals of the call client, with no conversion to a `Signal` | 10 498 817 | -7 358 | kept |
| cut 5: plain functions in place of the two components | 10 498 585 | -232 | dropped, within the noise |
| cut 6: no read of the state of the audio context | 10 501 561 | +2 744 | dropped, it grew |
| kept, with the props of the slider typed | 10 498 785 | +35 729 | the ceiling rounds it up, with 1 215 bytes of room |
| merged on `main` after the undo bytecode park | 10 501 646 | +2 861 | the two stories together pass 10 500 000, so the ceiling rounds up to 10 510 000, with 8 354 bytes of room |

* **The pin moved to `nightly-2026-11-11` on 2026-11-11, and no ceiling moved.** The code is the
  same on both sides: the compiler alone changed, with one linker flag the threads transform needs
  (`__heap_base`). On the session container, `trunk build --release` then `scripts/wasm_size.py`
  read the figures below. The figure before is the same build on `origin/main` at `92b145e`, on
  `nightly-2026-03-29`.

| Artefact | Raw before | Raw after | Raw change | Brotli before | Brotli after |
|---|---|---|---|---|---|
| Main | 10 463 133 | 10 301 007 | -162 126 | 2 746 229 | 2 725 270 |
| Sync | 1 300 477 | 1 281 819 | -18 658 | 374 853 | 371 740 |
| Tables module | 121 651 | 122 795 | +1 144 | 44 171 | 44 690 |
| PDF module | 1 785 317 | 1 772 785 | -12 532 | 413 058 | 412 148 |
| Markup module | 842 826 | 836 108 | -6 718 | 240 184 | 238 734 |
| Images module | 685 735 | 684 474 | -1 261 | 221 650 | 221 888 |
| Office module | 1 471 463 | 1 448 409 | -23 054 | 392 700 | 392 154 |
| Call worker | 100 164 | 101 244 | +1 080 | 35 285 | 35 442 |

  The first download reads 3 097 010 brotli after, against 3 121 082 before.

* **The HTML import and export joined the markup module on 2026-11-12, and its ceiling rose to
  860 000 raw bytes.** The module now reads a whole document through the walk a paste runs, and
  writes the intermediate form as a standalone HTML document
  (`4-Prose/features/ambre-spec-prose-import-export.md section 3`). Both live in `ambre-convert`,
  with the look the form carries in `ambre-prose-form`. One cut came first: the writer spells a
  colour and a size with the walk's own helpers, so it adds no formatter of its own. The build
  is `scripts/converter_build.py`'s command for the markup family, on the session container. The
  figure before is the same build of `origin/main` at `b856771`.

| Build | Raw | Change | Brotli | What |
|---|---|---|---|---|
| `origin/main` at `b856771` | 836 108 | -- | 238 734 | before the story |
| the story, before the cut | 856 989 | +20 881 | -- | the document walk and its report, the list nesting, the writer, the look's serde code |
| the cut: the writer spells a colour and a size with the walk's helpers | 856 687 | -302 | 244 354 | kept; the ceiling rounds it up to 860 000, with 3 313 bytes of room |

* **Prose pictures raised three module ceilings on 2026-11-13.** The PDF module embeds a picture
  as its stored file, and krilla's `raster-images` decodes a PNG and reads a JPEG header there
  (`4-Prose/features/ambre-spec-prose-images.md section 3`). Each module also decodes the request
  types it shares with the others, and the recorded document gained an encoded picture. So the
  markup and images modules grew with its serde code, and with that of the picture of the form.

  The markup module also writes a picture as a `data:` address. Main took a cut first: the decode
  scales and turns RGB and RGBA samples alone, rather than every kind `DynamicImage` holds. The
  builds are `just size` on the session container.

| Artefact | Ceiling before | Raw after | Ceiling after | What |
|---|---|---|---|---|
| Main, before the cut | 10 510 000 | 10 531 471 | -- | the picture engine, its decode, scale and turn over every sample kind |
| Main, after the cut | 10 510 000 | 10 496 125 | 10 510 000 | kept: under its ceiling |
| PDF module | 1 790 000 | 1 823 919 | 1 830 000 | the PNG decode and the JPEG header reader of krilla, and the encoded picture |
| Markup module | 860 000 | 868 514 | 870 000 | the picture of the form, its `data:` address and the encoded picture's serde code |
| Images module | 690 000 | 691 281 | 700 000 | the serde code of the shared request types |

  A cut of the PDF growth was weighed and refused: a PNG decoded on the Logic thread. Printing its
  samples holds every picture of the file decoded until the file is written.

* **The JPEG decoder left Main on 2026-11-13.** Main linked `zune-jpeg` and the JPEG glue of
  `image` for three decodes: a Prose picture, a Toile drop and a Murmure preview. The images module
  now runs `zune-jpeg` on the converter thread, with a scale and a turn of its own
  (`ambre-spec-architecture.md section 2`, the converter thread). Main keeps the PNG decoder, which
  the encoder of the picture paths shares, and reads a JPEG header alone. The request type joined
  every module's serde code, so the Tables module grew too. The builds are `just size` on the
  session container; the figure before is the Main after the cut of the Images story above.

| Artefact | Ceiling before | Raw after | Ceiling after | What |
|---|---|---|---|---|
| Main | 10 510 000 | 10 388 763 | 10 390 000 | lowered by 120 000: `zune-jpeg` and the JPEG glue of `image` left, 107 362 bytes below 10 496 125 |
| Images module | 700 000 | 823 444 | 830 000 | `zune-jpeg`, the scale and the turn, and the request type |
| Tables module | 130 000 | 130 390 | 140 000 | the serde code of the new request type, which every module decodes |

  The cut tried first for the images module was the decoder without the `image` crate around it.
  That crate adds about 98 KB of brotli to a module, measured for WebP on 2026-11-09. It adds no
  pixel, so the module calls `zune-jpeg` directly.

  A cut of the Tables growth was weighed and refused. The request could cross as bytes that the
  images module alone decodes. That saves under 400 bytes, and costs an encode on the Logic thread
  for each picture. The Tables brotli, 46 277, sits under its budget.


* **The PNG and GIF decoders joined the images module on 2026-11-15.** A Prose picture and a
  Murmure preview decode on the converter thread in every format (`ARCH-116`). The module runs
  `png` and `gif` directly, with the scale and the turn of the JPEG decode. Main dropped the GIF
  decoder and the decode paths of `image`, to 10 401 873 raw bytes from 10 433 278. The images
  module grew to 934 412 raw bytes, with no raw ceiling to hold it. Its brotli, 286 364 bytes,
  stays under the budget of 300 000.
* **Screen sharing joined Main on 2026-11-14, and its ceiling rose to 10 410 000 raw bytes.**
  The `ambre-app` crate grew by the share of `ambre-app/src/call.rs`. It holds the capture, the
  screen sender and its single encoding, the offer that adds it, and the take-over. The
  `ambre-murmure` crate added the share button and its refusals. The `getDisplayMedia` and
  `RTCRtpSender` bindings of `web-sys` came with them. The builds are `just size` on the session
  container; the figure before is Main after the JPEG decoder left it, above.

| Build | Main raw | Change | What |
|---|---|---|---|
| `origin/main` at `3c03d56d` | 10 388 763 | -- | before the story |
| the story, merged | 10 401 951 | +13 188 | the capture, the screen sender, the offer and the take-over |
| the cut: one helper calls each JavaScript method in place of the typed bindings | -- | +2 015 | dropped, it grew; read on the story's branch before the merge |

  The growth is the feature itself, so the ceiling rounds the merged figure up to 10 410 000,
  with 8 049 bytes of room. The first download, 3 127 784 bytes of brotli, sits under its budget.

* **Two PDF exports took Main over its ceiling on 2026-11-14, and the ceiling rose to 10 470 000
  raw bytes on 2026-11-15.** The `size` CI job read 10 406 422 bytes on `fc5d798e`, the last
  green run. The crates that grew come from `twiggy top` on builds that keep the name section,
  one per step, on the session container. Each figure is the code of the crate, the name
  section left out.
  * Four stories, `fc5d798e` to `2ca71ef1`: the font, Prose import, converter trap and signed
    exchange keys. `ambre-logic` +4 833 bytes, `core` +1 855, `ambre-prose` +1 039, `hashbrown`
    +689. The one copy of each font face and the send window live on the Logic thread.
  * The sheet PDF export, `a95515c3`: `ambre-logic` +13 298, where the print walk of
    `ambre-trame` inlines into `run_trame_print`. `core` +7 151, of which a stable sort of the
    printed cells weighs 4 551. `ambre-trame` +5 479 and `alloc` +1 235.
  * The deck export off the board's pass, `51bd2384`: `ambre-logic` +2 427 and `hashbrown`
    +359. The page loop moved out of `run_conversion` into `resume_deck_export`.

  Four cuts came first. The builds are `just size` on the session container; the figure before
  is `origin/main` at `df6196a3` on that container.

| Build | Main raw | Change | What |
|---|---|---|---|
| `origin/main` at `df6196a3` | 10 446 744 | -- | before the story |
| cuts 1 and 2: the printed cells of a sheet page and the marks of a Prose run fill their map one insert at a time, in place of a `collect` that sorts the pairs first | 10 437 193 | -9 551 | kept |
| cuts 3 and 4: `ambre_core::order` sorts by an identifier, or by a key then an identifier, over the keys and their places. Seven callers in Prose, Toile and Murmure share one copy of the sort in place of one copy each | 10 423 590 | -13 603 | kept |
| merged on `main` at `7acf751d` after the launcher list, the script policy, the passphrase floor and the lazy converter spawn | 10 433 009 | +9 419 | under the ceiling, with 36 991 bytes of room |

  The growth left is the two exports themselves. The ceiling holds 40 000 bytes of room above
  10 423 590 for the stories that wait to merge. It rounds up to 10 470 000. The first
  download, 3 146 335 bytes of brotli before the cuts, sits under its budget.

* **The boards module joined on 2026-11-15, under a ceiling of 160 000 raw bytes.** It reads a
  Figma or a Miro export into a board that Toile maps
  (`3-Toile/features/ambre-spec-toile-import-export.md section 3`). The reader first ran in the
  images module, which it took past its ceiling of 830 000. The cut tried first was a JSON reader
  of the module's own in place of `serde_json`. It left the images module at 851 186, so the reader
  moved to a family of its own. The builds are `scripts/converter_build.py` on the session
  container; the figure before is `origin/main` at `6cb1ba6` on that container.

| Build | Images raw | Boards raw | What |
|---|---|---|---|
| `origin/main` at `6cb1ba6` | 813 828 | -- | before the story |
| the reader in the images module, over `serde_json` | 870 421 | -- | the reader over `serde_json::Value`, and the board in every module's answer type |
| the cut: a JSON reader of the module's own, and the board as bytes | 851 186 | -- | still past the ceiling of the images module |
| the reader in the boards module | 813 966 | 149 883 | kept; the boards ceiling rounds up to 160 000, as 150 000 would leave under 1 000 bytes of room |

  The boards module holds a brotli budget of 100 000, as the Tables module does. The images module
  keeps its ceiling, and grows by the 138 bytes of the new request variant.

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

* **The strength floor joined Sync on 2026-11-14, and Sync kept its ceiling of 1.31 MB raw.** The
  estimator of `ZK-035` runs in the Sync worker alone, on ASCII classes, so it links no Unicode
  table of the standard library. On the session container, `just size` read the figures below. The
  figure before is the same recipe on `origin/main` at `367b010`. Two cuts made the room: the
  estimator walks its runs in place, and it dropped the search for a unit the whole text repeats.
  Sync now sits 109 raw bytes under its ceiling.

| Artefact | Before | After | Change | Why |
|---|---|---|---|---|
| Sync, raw wasm | 1 303 731 | 1 309 891 | +6 160 | the estimator, its list of common passwords and its rows, and the refusal of a setup |

* **Two cuts took Sync to 1 257 356 raw bytes on 2026-11-15, and its ceiling fell to 1 270 000.**
  Sync read 1 315 088 bytes on `origin/main` at `6cb1ba6`, over its ceiling of 1 310 000. The
  ceiling had risen to 1 325 000 with no cut. `twiggy top` on a build that keeps the name section
  found two crates that a small call pulled in whole. The builds are `just size` on the session
  container.
  * The key fingerprint called the generic encoder of `data-encoding`: 18 611 bytes of code for
    20 bytes of base32. A hand-written encoder of RFC 4648 replaces it, and a property test holds
    it to the reference encoder.
  * The bounded zstd read went through the `std::io::Read` adapter of ruzstd. The adapter boxes
    each decoder error into an `io::Error`, which links the `Display` and `Debug` of every ruzstd
    error and the `io::Error` machinery. The read drops the error anyway. It now drives the frame
    decoder one block at a time, under the same ceiling.

| Build | Sync raw | Change | What |
|---|---|---|---|
| `origin/main` at `6cb1ba6` | 1 315 088 | -- | before the story |
| cut 1: the fingerprint spells its base32 by hand | 1 294 551 | -20 537 | kept |
| cut 2: the zstd read runs the frame decoder with no `Read` adapter | 1 257 356 | -37 195 | kept |
| cut 3: an empty struct in place of `json!({})` for the lease bodies | 1 295 082 | +531 | dropped, it grew; read after cut 1 |

  The inviter pin adds 3 331 bytes and waits to merge, so the figure to round is 1 260 687. The
  ceiling rounds it up to 1 270 000, with 9 313 bytes of room once the pin lands. Main and the
  converter modules hold their figures, and the call worker reads 77 bytes less. The first
  download reads 3 138 616 bytes of brotli.
* **The PNG raster raised the PDF module ceiling on 2026-11-15.** The PNG export rasterizes a
  recorded page in the PDF module, on the converter thread
  (`3-Toile/features/ambre-spec-toile-import-export.md section 3`). Three cuts came before the
  raise. Each figure is the module built alone by the command of `scripts/converter_build.py`, on
  the session container. The figure before is the same command on `origin/main` at `1d26e12`.

| Build of the PDF module | Raw | Change | Brotli | Verdict |
|---|---|---|---|---|
| `origin/main` | 1 822 699 | -- | 425 324 | before the story |
| `tiny-skia` rasterizes and encodes | 2 204 715 | +382 016 | 485 000 | refused: its fill pipeline is about 250 KB |
| the cut: `tiny-skia` without `png-format`, the `png` encoder krilla links | 2 203 910 | +381 211 | 484 553 | refused: the encoder was not the weight |
| the cut: `zeno` rasterizes a coverage mask | 2 077 341 | +254 642 | 458 617 | refused: its stroker, dasher and mask paths are about 110 KB |
| the cut: a coverage rasterizer of the module's own, with the `png` encoder | 1 980 011 | +157 312 | 448 036 | refused: the encoder adds about 20 KB |
| the cut: the module writes the PNG chunks over the zlib stream krilla links | 1 953 691 | +130 992 | 440 771 | kept |
| the samples typed as peniko's premultiplied colour, not bare bytes | 1 953 835 | +131 136 | 440 804 | kept; the ceiling rounds it up to 1 960 000, with 6 165 bytes of room |

  The glyph outlines hold 91 655 bytes of the growth kept. skrifa draws them through a pen type of
  its own, and krilla's pen is private, so the module instantiates skrifa's outline code twice. A
  third cut, dead code elsewhere in the module, was not found: `twiggy diff` against
  `origin/main` names no other crate past 7 KB.

**The Main instance, per contribution.** Estimates, and why the lazy-loading rule above exists.

| Contribution | Estimated raw | Estimated gzip | Notes |
|---|---|---|---|
| `ambre-prose`, engine plus features plus interop plus UI | ~230 KB | ~72 KB | Of which the UI module is ~50 KB raw |
| `unicode-segmentation` | ~40 KB | ~15 KB | Word and grapheme boundaries: the word count, caret movement |
| `spellbook` plus one dictionary | 830 626 raw for English, 1 704 676 for French | 243 424 and 363 308 brotli | Measured on 2026-11-16, `cargo build -p ambre-convert --features spell_en` then `spell_fr`, `wasm-release`, as `scripts/converter_build.py` builds them. A module of its own per language, fetched on the first check of that language. No instance links it (`PROSE-FEAT-104`) |
| `ambre-docx`, with `zip` and `quick-xml` | ~427 KB | ~147 KB brotli | The Word codec, read and write, in a test `cdylib` at `opt-level = "z"` with LTO, measured on 2026-11-09. `docx-rs` measured 505 KB of brotli in the same frame. No instance links it: the Office module does since 2026-11-10, over the package layer `ambre-opc`, and Main links its model alone |
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
| deltas held for a writer, per document | ~2 MB max | Capped at 1000 updates, as the quarantine (`SYNC-211`) |
| edits held while a channel log walks again, per document | ~2 MB max | Capped at 1000 edits, as the quarantine. Past it the walk drops them all (`SYNC-234`) |
| snapshots deferred while a channel log walks again, per document | ~64 KB max | Capped at 1000 identifiers, as the quarantine. The blob is fetched again after the walk (`SYNC-234`) |
| `old_keys`, per document | negligible | Capped at 10 `DocumentKey` (32 bytes plus overhead) |
| Unconfirmed updates, per document | 1024 updates | `UNCONFIRMED_LIMIT`: the sent deltas the relay has not echoed yet, kept to resend one it refuses for its key (`SYNC-129`) or its clock (`SYNC-147`). Past the cap the oldest is forgotten, and a refusal for it resends nothing |
| Sync events held on Main past a full channel | 4096 events, released at 1024 | `SYNC_INBOX_MARKS`, high and low at four times and once the channel capacity. At the high mark the Sync worker closes its socket, and the relay holds the rest (`SYNC-059`). The bound is a count, not bytes: a replay batch is one event |
| View events held on Main past a full view queue | 2048 events per view: 1024 in its queue, 1024 in its backlog | `CHANNEL_CAPACITY` and `VIEW_BACKLOG_LIMIT` in `ambre-app`. A view reads both once a frame. A hidden page relays on its timer, and its views read nothing until it shows (`CANVAS-075`). Past the bound the oldest event drops, and the document is resent once the view reads again (`ambre-spec-message-bus.md`, `BUS-039`, `BUS-041`). **Reachable.** Measured on 2026-11-07 by `cargo test -p ambre-logic view_load -- --nocapture`, host debug build in a cloud container. A remote delta sends its view 3 events in Trame, 2 in Toile and 3 in Prose, alone in its pass or in a burst. So a hidden page overflows after about 683 remote cell writes, 1024 remote elements or 683 remote words. A replay batch sends 3 events, a paste of 10 000 cells 2, a CSV import 5, and an SVG import of 5 000 nodes about 3 per slice. A shown page reads each of those within a frame |
| Edits held before the session identity, per document | 256 edits | `HELD_EDITS_LIMIT` in `ambre-logic`. The wait is one message from the Sync worker, so the cap is reached only by a session opened before its unlock. Past the cap a new edit is dropped and logged (`AUTHOR-026`). A cached document restored in that wait is held too, one copy of its bytes until the identity lands (`AUTHOR-027`) |
| Sync overhead, 1 host document plus 3 embedded | ~4-8 MB | The typical scenario |
| `LoroDoc` | < 10 MB target, per 100k cells | |
| `LoroDoc` Prose | < 10 MB, per 100 pages and 50 000 words | |
| Prose page layout, 100 pages | < 200 KB | About 300 blocks, 20 footnotes and the header and footer runs. Derived, so it is re-allocated whole on a full recompute |
| Prose shaped runs, Logic thread | < 4 MB, a ceiling | The runs of the pages the view asked for and of four pages on each side, every other run dropped (`PROSE-136`, `PROSE-139`): about twenty pages. A run costs about 26 KB, a paragraph of eighty words, six to a page. Measured on 2026-11-10 in a cloud container, native release build, by a counting allocator in a probe kept out of the tree, on a document of 300 pages and a view asking for twelve: 46.5 MB with every run held, as every run was before `PROSE-136`. Then 2.56 MB at the start, 1.86 MB in the middle and 1.81 MB at the end |
| Prose document of 300 pages, Logic thread | < 32 MB | A novel: its `LoroDoc`, its page layout and the shaped runs it holds, within the documents-held-in-memory row below. Measured on 2026-11-10 by the probe of the row above, the whole Prose state of the Logic thread after a snapshot import, the 2.84 MB of the shipped faces included: 20.44 MB at the start, 19.79 MB in the middle and 19.75 MB at the end. Every run held, it took about 64 MB |
| Prose import, transient, Logic thread | < 100 MB, by a Markdown or HTML file of at most 3.5 MiB | The file, its form, its operation list and the blocks written. The import writes the document once, and each operation gives up its text once written. Measured on 2026-11-13 in a cloud container, native release build, by the peak resident size of a test process that imports a Markdown file of 400-character paragraphs and drops each slice's delta, as the Logic thread hands it on: 61.8 MB for 3.67 MB of file and 90.8 MB for 5.5 MB, about 17 times the file. A counting allocator in a probe kept out of the tree reads 63.0 MB and 97.3 MB. The document written holds about 16 times the file, and keeps it once the import ends. On 2026-11-14 the undo step of the import stopped copying the text, by the same probe: the peak fell to 56.5 MB for 3.67 MB, 15.4 times, and the document to 14.8 times (`Prose document, held after an import` row below). By the same probe, the rehearsal on a scratch document that this replaced held 130.4 MB for 5.5 MB, 23.7 times. The figure of 2026-11-12, 45 times, also kept every slice's delta. The markup module reads the file in its own memory, about 6.6 times the file, and the browser keeps that memory for the session. The bound of the file follows from the two: 100 MB over about 24 is about 4.2 MB, so 3.5 MiB, about 63 MB on the Logic thread and 87 MB with the module. Derived again on 2026-11-14 from the peak of 15.4 times: 100 MB over about 22 is about 4.5 MB. The bound stays 3.5 MiB, about 57 MB on the Logic thread and 81 MB with the module, and the 19 MB left is headroom for the browser's allocator, which the native figures do not measure. Derived again on 2026-11-17, from the document of 13.9 times by a heap profile (`Prose document, held after an import` row below). The document no longer sets the bound: the peak of the import does. By `import_memory` on the same day, the peak reads 15.0 times above the resident size before the import, about 55 MB. A wasm memory never shrinks, so the bound holds the peak plus the module: 100 MB over about 21.6 is about 4.6 MB. The bound stays 3.5 MiB, about 79 MB with the module, and the 21 MB left keeps the headroom of the browser's allocator. It is `ambre_prose::interop::markdown::MAX_SIZE`, which HTML shares, and a file past it fails as `TooLarge` (`PROSE-IO-102`). `PROSE-IO-111` holds a ceiling of 20 times the file in CI. A Word or an OpenDocument package keeps the OOXML package bound below |
| Prose document, held after an import, Logic thread | < 15 times its Markdown file, by a file of 400-character paragraphs | What the document keeps once the import ends, part by part. Measured on 2026-11-17 in a cloud container, native release build, by the bytes the heap holds at exit under `valgrind --tool=dhat`. A probe kept out of the tree imports the Markdown file at the bound, 3.67 MB and 9 129 paragraphs, then drops one part at a time. In all 50.9 MB, 13.9 times. The `LoroDoc` holds 40.8 MB, 11.1 times: about 4.5 KB a paragraph, of which 400 bytes are its text. Of that, the B-tree of each body text allocates room for four internal nodes on its first character, 13.4 MB in all, 3.7 times. The string arena of its oplog holds 8.4 MB in 9 chunks for 3.7 MB of text, 2.3 times: each chunk doubles the one before, and a slice keeps its chunk. The same `LoroDoc` read again from its snapshot holds 30.3 MB once its state is decoded. The undo history holds 9.3 MB, 2.5 times. Its mirror is about 8.9 MB: a copy of every text, 3.6 MB, the table of the containers, 2.7 MB, and the node maps, 1.6 MB. The step of the import holds about 0.4 MB: the tree write of each commit, with the nodes it created. The rest of the state, the catch-up stamps and the block watch of the paginator, holds 0.8 MB. The same day, before the change, the same probe read 58.1 MB, 15.8 times, of which 16.5 MB in the undo history. The step of the import then kept a map write and a text write per paragraph, 5.7 MB, which no replay read (`UNDO-022`, `UNDO-023`). Each text of the mirror also kept room for four runs, 1.5 MB. On 2026-11-14 a counting allocator read 54.4 MB, 14.8 times, by a probe that measured each part another way. `PROSE-IO-119` holds a ceiling of 15 times in CI, by the resident size of a child process under a fixed trim threshold of the C allocator. The test reads about 14.4 times, and 16.4 times on the branch point of the change. Ten times needs two changes outside this crate: the B-tree's first allocation, and a mirror that copies no text |
| Prose run cache, 500 runs | < 1 MB | The drawing state of the runs painted lately, about 50 pages: an index into runs the Logic thread already holds (`features/ambre-spec-prose-page-rendering.md`) |
| Prose view state, excluding the run cache | < 1 MB | |
| OOXML package read, per conversion | 4096 parts; 32 MiB inflated per part; 64 MiB inflated in all | One set for DOCX, xlsx and PPTX, checked while the bytes inflate, never read off a header (`ambre_opc::MAX_PARTS`, `MAX_PART_SIZE`, `MAX_INFLATED`). The sum is about 67 MB, so the model a codec builds beside the parts stays inside the < 100 MB total. A Word body of 12 MB and a sheet of 100 000 cells read under the bound of one part. Held by one test per bound in `ambre-opc` |
| Trame CSV import file | 8 MiB | `CSV_MAX_SIZE` in `ambre-convert`. The Logic thread refuses a file past it before any work, as too large (`TRAME-IO-090`). At ten columns of short text, the bound holds about 840 000 cells, the cells of the largest Excel sheet under the part bound above. It also holds the 100 000 rows of the grid at that width. The bound holds the time of the module call (section 8, `ARCH-128`), and **not the memory**. On 2026-11-17, the file of one-byte lines at the bound peaked at 729 MB of resident memory in the host bench process, x86_64 release build. Ten short fields a line peaked at 128 MB. Each record costs two allocations for two bytes of input, and the Logic thread decodes the records again. No bound holds the records yet |
| Prose decoded pictures, per document | < 64 MiB | `CACHE_CAP` in `ambre-prose`, a cache in the Logic thread counted in decoded bytes, evicted least recently painted, never a transfer in flight (`PROSE-FEAT-054`). One picture is at most 2048 px a side, 16 MiB, so it holds about three such pictures, or the pictures of a few pages, inside the < 100 MB total (`features/ambre-spec-prose-images.md`) |
| Prose picture source edge | 8192 px, each side | `MAX_SOURCE_EDGE` in `ambre-prose`, read off the header before any decode (`PROSE-FEAT-314`). A JPEG at the bound decodes to about 192 MiB of RGB samples before its scale, in the memory of the converter thread's images module: a transient peak past the < 100 MB total on the device that opens such a file. The row below gives that memory back. A PNG at the bound decodes to 256 MiB of RGBA samples in the same module, before its scale |
| Converter thread module memory, after a job | 64 MiB per instance | `RESET_FOOTPRINT` in `ambre-logic`. A WebAssembly memory never shrinks, so an instance past it after a job is dropped and loaded again from its bytes, and the peak of one decode does not stay for the session (`ARCH-099`). A decode under it keeps its instance, and the next decode reuses that memory |
| Toile PNG export raster edge | 4096 px, each side | `RASTER_EDGE` in `ambre-convert`. The Logic thread scales a page down until its longest edge meets it, and the PDF module refuses a request past it (`TOILE-IO-185`, `TOILE-IO-187`). At the bound the pixmap holds 64 MiB in the converter thread's instance of the module, so the instance is loaded again after the job (`ARCH-099`) |
| Converter module memory on the Logic thread, after a pass | 16 MiB per instance | `LOGIC_RESET_FOOTPRINT` in `ambre-logic`. The Logic thread holds one instance per family for the session, and a memory never shrinks. So at the end of each pass an instance past it is instantiated again from its compiled module, at its first span (`ARCH-125`). A module starts at 1.06 MiB to 1.38 MiB: the initial memory of the tables, PDF and markup modules, read from their `wasm-release` builds on 2026-11-16. Five families thus keep at most 80 MiB, where the 64 MiB of the converter thread would let them keep 320 MiB. A Markdown or HTML file reads on the converter thread (`ARCH-122`), so the instance this renews is the one a large HTML paste or a Toile board file grew |
| Converter thread jobs in flight | 8 | `IN_FLIGHT` in `ambre-logic`. The rest wait in the Logic thread, oldest first, so each of the two queues of the thread, 16 slots, never fills (`ARCH-103`). Each job holds its file until its answer, a JPEG of a few megabytes |
| Converter threads spawned again, per session | 3 | `RESPAWN_LIMIT` in `ambre-logic`. A thread that missed its answer deadline is never ended: its worker and its stack stay in the shared heap for the session. A spawn that failed and is tried again by the next job counts too (`ARCH-118`). Past the third, no thread is spawned, and every job fails as a module that did not load (`ARCH-108`). Each answer deadline holds three times the largest input its family admits (`ARCH-127`, section 8), so a missed deadline means a thread that stopped, not a slow job. The limit bounds the workers a session leaves to stopped threads, and no export spends it |
| Converter threads spawned at boot | 0 | The first job spawns the first thread, while its module downloads, and that spawn counts against no respawn limit (`ARCH-112`). A session that converts nothing holds no converter worker and no stack for one |
| Prose picture cache edge | 2048 px, the longer side | `CACHE_EDGE` in `ambre-prose`: a decoded picture is scaled down to it once (`PROSE-FEAT-315`). A full-width picture of an A4 page prints at about 300 dots an inch at it |
| Prose picture display edge | 4096 px, each side | `MAX_DISPLAY_EDGE` in `ambre-prose`: the document stores a display size clamped to it (`PROSE-FEAT-059`). Past it no page shows the picture, and a layout pays for nothing it draws |
| Prose export picture bytes, per export | < 32 MiB | `EXPORT_BYTES_CAP` in `ambre-prose`: the encoded bytes an export fetched and holds until its file is written (`PROSE-IO-106`). A picture past it goes out as a placeholder and a line in the report. The writer copies them into the file it builds, so the peak is about twice the figure, 64 MiB for the length of one export, beside the document of the row of 300 pages above inside the < 100 MB total |
| Toile export picture bytes, per export | < 32 MiB | `EXPORT_BYTES_CAP` in `ambre-toile`: the stored files a deck export fetched and holds until its file is written (`TOILE-IO-100`). A picture past it goes out from its thumbnail, and the report counts it. The writer copies them into the file, so the peak is about twice the figure for the length of one export |
| Trame hidden line elements, per sheet | < 1 000, about 51 KB | Only two peers writing at once into one new line create one: each such race leaves one hidden element per line it adds (`2-Trame/features/ambre-spec-trame-rows-and-columns.md section 3`). None is removed, since a reference may name it. **None exists** to count them |
| Prose vertex buffers and atlases, 3 pages | < 50 MB VRAM | Stated by the Prose design and larger than the shared wgpu row above, which is the arithmetic the two rows now make visible. Reconciled by the `The memory budgets add up` Story (`ambre-roadmap.md` E7) |
| Trame DAG plus cache | < 50 MB, per 100k formula cells | |
| Toile board plus index, 50 000 elements | proportional to the element count, inside the < 100 MB total | The R-tree adds a small multiple of its leaf count over the bounds map it mirrors |
| Server, per open document | ~50 KB | No in-process blob cache: `LIST doc:{id}:pending` buffers in Redis, ~50 deltas of ~1 KB over the 15 s window |
| Server, inline delta size | < 64 KB | A delta past it keeps an object of its own (`SRV-149`) |
| Server, inline deltas per document | 256 deltas or 256 KB, whichever comes first | Past it, the oldest move into one batch object (`SRV-150`, `SRV-151`). Bounds the register at the documents in use times this figure |
| Server, deltas past a document's served snapshot before the relay asks for one | 500 | `SNAPSHOT_THRESHOLD` in `ambre_server::compaction` (`SRV-038`). Bounds what a cold open replays. A first estimate, to measure against open times |
| Server, delta rows kept per document after a compaction pass | < 2 x the snapshot threshold above: 1 000 rows, while each snapshot comes at the threshold and is confirmed | `SNAPSHOTS_KEPT` in `ambre_server::compaction`, 2 (`SRV-264`). The trim stops at the older of the two snapshots kept, so a document keeps one threshold of deltas past it, plus the deltas past its latest snapshot. A client that answers no ask leaves the second part unbounded (`SRV-038`). A pass runs each pass period below, so a document at the design delta rate adds one threshold of deltas between two passes: about 1 500 rows at most before a pass. Once an hour, before 2026-11-16, let it add 18 000 rows at that rate. Ten snapshots kept, before 2026-11-15, left about 4 500 rows per active document in the write-cost run of 2026-11-13 |
| Server, design delta rate of one active document | 5 deltas a second | `DESIGN_DELTA_RATE` in `ambre_server::compaction`. A typist at 60 words a minute types 5 characters a second, and each key outside the send window leaves as one delta (`SYNC-192`). The send window caps a document at 40 deltas a second, a burst no person holds for minutes. A first estimate: no delta rate per user is measured yet |
| Server, compaction pass period | 100 s: the snapshot threshold over the design delta rate, 500 / 5 | `PASS_PERIOD` in `ambre_server::compaction` (`SRV-277`). A document at the design rate adds one threshold of deltas between two passes, so one pass at most falls between two of its snapshots. A document at the cap of the send window adds 4 000 deltas between two passes. Each instance runs the pass, and the trim reads the snapshots of every document: on 2026-11-16, over 2 000 documents of 1 000 rows each and two confirmed snapshots, the trim took 0.42 s and deleted nothing, by `EXPLAIN ANALYZE` on the Postgres of the database tier, on the cloud container of the write-cost row. The trim scans `updates` once, so its time grows with the rows kept over all documents |
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
| Interop input fixture | 64 KiB each, a ceiling | Each file under a `fixtures` directory of a crate, the `.expected` files included (`INTEROP-018`). A bigger file is a real document more often than a test case, and git carries it forever. On 2026-11-10, `just fixture-provenance` read 49 input files. The largest weighs 56 544 octets, `calamine/picture.xlsx`, and the 91 files of the fixture directories weigh 488 081 octets. A format that needs more raises this row in a reviewed edit |
| `just primitives`, the dependencies fresh, every workspace unit compiled again through the driver | 120 s, a ceiling | Measured on 2026-11-08 in a cloud container of 4 vCPU. The nine lint lines took 106 s after an earlier run of the gate, and 110 s right after `just lint` and `just lint-wasm`, 116 s for the whole recipe. The host line takes 29.5 s, the line of `ambre-belt` 2.2 s and the seven wasm lines 79 s, each printed by the gate. The spike timed the host library line alone at 9.3 s. A first run after a cold lint rebuilds the dependencies whose feature set differs, 7 min 50 s on the same day |
| CI fast lane: a push of a branch other than `main`, from the start of the run to the end of its last job | 15 min | The lanes are `ambre-spec-testing.md section 7`. Before them, every push ran every job. Run 43574018216 of 2026-11-14, a branch push, took 60 min 29 s, set by `instructions` at 59 min 51 s. The jobs of today's fast lane took 18 min 59 s (`lint`), 21 min 58 s (`test-native`), 16 min 13 s (`test-wasm`), 5 min 13 s (`test-db`) and 26 s (`deny`). Each compiled every dependency again: `test-wasm` spent 9 min 40 s compiling and 30 s testing. Read from the job and step timestamps of the GitHub API. On 2026-11-14, in a cloud container of 4 vCPU, the build of `just test-native` took 15 min 42 s cold and 2 min 29 s from a `target/` that `just ci-cache prune` left, the `std` sources dated by `just ci-cache pin-std` after a fresh install: 31 workspace crates compiled, no dependency. With the lanes and the cache, run 43588651946 of 2026-11-14, a branch push on caches restored with a full match, took 12 min 19 s, set by `test-native` (10 min 9 s in `just test-native`). `lint` took 5 min 34 s, `test-wasm` 4 min 52 s, `test-db` 2 min 36 s and `deny` 27 s. No dependency compiled. Run 43585929042 before it, which filled the caches, took 28 min 55 s: a branch reads the caches of `main`, so a branch runs cold until `main` has run once under the `v1-rust` caches. On 2026-11-15, the branch pushes of the day took 9 min 13 s to 26 min 5 s. `test-native` set each one, from 8 min 53 s to 16 min 21 s, and a run that waited for a runner lost up to 10 min before its first job. The unit tests of `ambre-sfu` took 517 s of `test-native` in job 132759869196: the call simulation spends CPU alone, and the guard of `CALL-053` runs it four times. The `test` profile now builds `ambre-sfu` at `opt-level` 3, and the job no longer deletes the unused toolchains of the image, 99 s. Run 43719065570 of 2026-11-15, a branch push, took 8 min 14 s, set by `test-native` at 7 min 56 s, of which 195 s of `ambre-sfu` tests. The runner kept 79 GB free, and `target/` weighed 7.3 GB |
| CI full lane: a push of `main` or a pull request, the same span | 30 min | The same run set the full lane too: 60 min 29 s. Its longest jobs were `instructions` (59 min 51 s for 26 counts on 4 cores), `test-browser` on Chrome (36 min 35 s, of which 23 min 17 s of belt tests one at a time) and on Firefox (26 min 39 s). The lane now counts the instructions in four shards and runs the belt of each engine in two parts. Run 43590023650 of 2026-11-14, a manual dispatch on a branch, built every full-lane job cold: no cache of `main` existed under `v1-rust`, and a branch writes none. Its shards of `instructions` took 12 min 41 s to 16 min 2 s. The Chrome part of Murmure took 35 min 44 s, of which 13 min 16 s of belt tests, and the Firefox part of the rest 22 min 58 s. `fuzz` took 15 min 20 s and `size` 10 min 48 s. With 26 jobs queued at once, one shard started 21 min after the run: the lane holds its row only when the nightly jobs do not share its runners. Run 43701241023 of 2026-11-15, a push of `main` on warm caches, took 20 min 58 s. It was set by `instructions` (1) at 19 min 43 s, then `instructions` (0) at 19 min 36 s and `test-browser` on Chrome, the rest, at 18 min 51 s |
| CI workflow `devcontainer`: the image, the stack and the probe | 60 min, a ceiling | The workflow runs outside the lanes, on a push that changes the dev container and nightly (`ambre-spec-testing.md section 7`). Measured on 2026-11-14 in a cloud container of 4 vCPU, by hand: the image built in about 10 min with no layer cache, 9 min of it compiling Trunk and `just`. In the container, the relay built cold in 5 min 10 s and the client in about 11 min, then `scripts/dev_stack.py probe` passed. CI run 43651629322 of 2026-11-14 took 20 min: the image in 7 min 26 s with no layer cache, the stack and the probe in 12 min 4 s |

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
| Join clock skew: how far the clock of a later joiner may lag behind the clock of a participant | 60 s | the clocks of two devices synchronised by their operating systems differ by seconds. A minute admits that, and refuses a replayed join of a session that ended before the participant joined (`CALL-124`) | `JOIN_CLOCK_SKEW` in `ambre_sync::call` |
| Delay before a Murmure client fetches again a successor that failed to open | 1 s, doubled at each failure, five failures at most | a successor fetched before its first update lands within a second, and a relay down for minutes is not asked each tick (`MURMURE-482`) | `SUCCESSOR_RETRY` in `ambre_murmure::channel` |
| Refresh period of a typing indicator | 3 s | one sample per few seconds of typing, against one per keystroke (`MURMURE-124`) | `TYPING_REFRESH` in `ambre_murmure::watchers` |
| Typing indicator expiry | 6 s | two refresh periods: one for the tick that dates a sample, one for a lost sample (`MURMURE-124`) | `TYPING_EXPIRY` in `ambre_murmure::watchers` |
| Idle delay before a member shows as away | 10 min | a short break keeps a member online, and a meeting marks them away | **none exists** |
| Finalize lease | 30 s | long enough to encode, seal and upload one segment of about 2 MB | `FINALIZE_LEASE` in `ambre_server::routes::channels` (`SRV-119`), which the fold lease shares (`SRV-204`) |
| Answer deadline of the converter thread, images family | 45 s | three times the dearest images call on the target phone, rounded up to the next 5 s with 5 % to spare: a JPEG of 8192 by 8192 pixels, the Prose source bound, decoded upright down to the cache edge of 2048 px. On 2026-11-16, in an x86_64 cloud container of 4 vCPU, Intel Xeon at 2.30 GHz, host release build, by the commands of the next column: 10 642 561 413 instructions, 13.3 s at the phone rate of `scripts/instructions.py`. The native call took 0.79 s, best of three, about 5.5 s at the time ratio of 7 that `scripts/instructions.py` names. The deadline runs from the latest answer, under the family of the oldest job in flight (`ARCH-108`, `ARCH-127`) | `answer_deadline` in `ambre_logic::converter_thread`. The CI `instructions` job holds `images_largest` of `crates/ambre-convert/benches/instructions.rs` under 12 000 000 000, the deadline divided by 3. A photograph of 4032 by 3024 pixels, `images_dense`, counts 3 054 408 010 |
| Answer deadline of the converter thread, Office family | 40 s | three times the dearest Office call on the target phone, rounded the same way: the write of a sheet of 840 000 cells, whose part inflates to 32 772 106 bytes under the 32 MiB bound of `ambre_opc::MAX_PART_SIZE`. Half the cells are numbers, a quarter texts, a quarter formulas. On 2026-11-16, the same container: 10 035 483 151 instructions, 12.5 s at the phone rate; native, 1.45 s to 1.57 s. The read of the same file counts 8 273 978 111, native 0.69 s to 0.75 s. A Word body of 32 283 569 bytes counts 2 634 951 724 to read and 2 371 336 634 to write, native 0.37 s to 0.43 s and 0.21 s to 0.24 s | `answer_deadline` in `ambre_logic::converter_thread`. The CI `instructions` job holds `sheet_write_largest` of `crates/ambre-convert/benches/instructions.rs` under 10 666 666 666. A sheet of 100 000 cells, `sheet_write_dense`, counts 1 166 169 623 |
| Answer deadline of the converter thread, PDF family | 35 s | three times the dearest PDF call on the target phone, rounded the same way: the write of the Prose novel of the slice rows, 193 pages, with seven PNG photographs of 1182 by 1182 pixels, 28 MiB under the 32 MiB an export fetches. The module decodes and compresses each PNG again. On 2026-11-16, the same container, counted inside `ambre_pdf::document::write_document`: 7 979 577 555 instructions, 10.0 s at the phone rate; native, 0.86 s to 0.95 s. The copy of the request into the module falls outside the count. No bound holds the pages: each page of text adds about 6.6 ms at the phone rate, so a document past about 250 more pages with its pictures at the cap holds less than the margin of 3 | `answer_deadline` in `ambre_logic::converter_thread`. The CI `instructions` job holds `pdf_write_largest` of `crates/ambre-prose/benches/instructions.rs` under 9 333 333 333. The same novel with twenty small pictures, `pdf_write`, 198 pages, counts 1 045 410 730, native 0.11 s |
| Answer deadline of the converter thread, markup family | 30 s | the floor of every deadline: a deadline catches a thread that stopped, which is rare, so a shorter one catches it little sooner and fails more jobs that still work. The dearest markup call measured is the read of an HTML file at the 3.5 MiB bound of a Prose import. On 2026-11-16, the same container: 1 127 287 017 instructions, 1.4 s at the phone rate. A Markdown file at the bound counts 236 120 417. A write of Markdown or HTML has no bound and is not measured | `answer_deadline` in `ambre_logic::converter_thread`. The CI `instructions` job holds `html_largest` of `crates/ambre-convert/benches/instructions.rs` under 8 000 000 000 |
| Answer deadline of the converter thread, tables family | 45 s | three times the dearest tables call on the target phone, rounded as the images row: the read of a CSV file at the 8 MiB bound of an import, one field of one byte a line, so 4 194 304 records. On 2026-11-17, in an x86_64 cloud container, host release build, by `valgrind --tool=callgrind` over `csv_read_largest`: 10 522 858 129 instructions, 13.2 s at the phone rate of `scripts/instructions.py`. The native process took 1.5 s to 1.8 s, file built in. Ten short fields a line count a sixth of the dearest shape, twenty one-byte fields two fifths, and twenty empty fields a quarter. A write of CSV has no bound and is not measured | `answer_deadline` in `ambre_logic::converter_thread`. The CI `instructions` job holds `csv_read_largest` of `crates/ambre-convert/benches/instructions.rs` under 12 000 000 000, the deadline divided by 3 (`ARCH-128`). A file of 10 000 common lines, `csv_read_dense`, counts 114 762 644 |
| Delay before the amendments of a segment fold into it | 7 days | a week batches the edits of one segment into one fold, and leaves three weeks of the erasure budget below for the confirmation | `FOLD_DELAY` in `ambre_server::routes::channels`, counted from the last fold or from the creation of the amendments (`SRV-206`); `FOLD_DELAY` in `ambre_murmure::channel`, which a client waits out after its own fold |
| Idle delay before a document's inline deltas move to a batch object | 24 h | a document untouched for a day is rarely opened within the next, and its tail then costs one object read | `IDLE_AFTER` in `ambre_server::hot_tail`, the delay the compaction pass reads (`SRV-152`) |
| Shortest interval between two replays of one document on one connection | 1 s | a reconnect asks once per document, so a second ask within the second is a client looping or an amplification attempt (`SRV-086`) | `REPLAY_MIN_INTERVAL` in `ambre_protocol::ws`, which the relay reads |
| Delay before Sync asks again for a stopped replay | 1 s, doubled at each stop, five stops at most | the first ask must outlast the replay interval above, or the relay drops it. Five stops wait 31 s in all, which outlasts a store that restarts, and a delta lost for good stops every ask (`SYNC-153`, `SYNC-154`) | `REPLAY_RETRY_BASE` and `REPLAY_RETRY_LIMIT` in `ambre_sync::websocket` |
| Shortest interval between two replay asks of one document Sync sends on one socket | 1.25 s | the asks must outlast the replay interval above, with a margin for the frames in flight. A watch and an open of one channel ask within the second (`SYNC-188`). A spacing at or under that interval fails the build (`SYNC-191`) | `REPLAY_ASK_SPACING` in `ambre_sync::websocket` |
| Delay before Sync sends again a delta the relay failed to sequence | 1 s, doubled at each failure since the last echo, five failures at most | five failures wait 31 s in all, which outlasts a clock store that restarts. A relay that keeps failing is not hammered, and past the budget the document holds the delta (`SYNC-159`, `SYNC-156`) | `RESEND_RETRY_BASE` and `RESEND_RETRY_LIMIT` in `ambre_sync::updates` |
| Send window of a document's deltas | 25 ms | the client share of the inter-user budget is 50 ms (README). Outside the window, one key spends 24.55 ms of it on the targets of section 1: 2 ms for the Prose insert and its export, two hops between the instances at 1 ms each, 0.2 ms for the seal, 0.35 ms for the open, 2 ms for the import, 3 ms for a partial reflow and 15 ms for the display. The window takes the 25 ms left. A key typed alone waits for nothing, and a document sends at most 40 deltas a second (`SYNC-192`, `SYNC-193`) | `SEND_WINDOW` in `ambre_logic::send_window`; `ambre_logic::send_window::tests::no_key_waits_past_the_window` holds the wait on a simulated clock, and `ambre_logic::window_tests::a_held_delta_leaves_with_no_message` holds the wake of the thread |
| Margin before the announced expiry of an access token at which Sync renews it | 60 s | twice the request deadline of Sync: the renewal may take one, and a request sent under the old token just before it may take the other. Both land while the old token lives (`SYNC-183`) | `RENEWAL_MARGIN` in `ambre_sync::renewal` |
| Delay between a Murmure delete after the finalization and its erasure | < 30 days | the re-finalization runs in batches, and an erasure request expects an answer within a month | **none exists**: `FOLD_DELAY` bounds the wait for a fold (`SRV-206`), and nothing yet measures the confirmation after it (`SRV-208`) |
