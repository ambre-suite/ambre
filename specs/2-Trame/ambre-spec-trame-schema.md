# AMBRE -- SPEC: TRAME CRDT SCHEMA
**Type:** Schema (persistence format, one document per application)
**Implementation:** partial

> The persisted shape of a Trame workbook: every Loro container, its key, its value, and the
> granularity at which two peers race. It owns identity, keys, merge granularity and the convergence
> properties that follow from that shape. It owns no gesture, no permission and no event.

---

## 0. SCOPE, AND THE MENTION RULE

A schema decision is often justified by an action. A row identity survives a sort, a delete and a
reorder, and its `{peer}_{counter}` shape answers concurrent minting. So an action appears here as a
**cause**, never as a **rule**.

| Position | Actions | Why |
|---|---|---|
| Container table (section 3), hierarchy (section 4) | forbidden | pure format |
| Container note (section 5), `Rationale` of a decision (section 6) | allowed, as a cause | the reason is the action |
| An invariant statement (section 1) | forbidden when it binds an action | "a merge MUST be refused while a covered cell is non-empty" belongs to the cell-merges feature |

**Budgets**, each with the destination its overflow takes:

| Cap | Overflow goes to |
|---|---|
| 600 words of prose for the whole document, outside tables and lists | a table, or nothing |
| 40 words for a container note (section 5) | section 6, whose `Rationale` column is not capped |

---

## 1. INVARIANTS

Prefix `TRAME`, shared with `trame-engine`: one prefix, one numbering, no renumbering.
`TRAME-003`, `TRAME-010` and `TRAME-016` sit here because each states a property of the format,
not of the engine. `TRAME-221` and `TRAME-222` are the halves those last two hid, and each states
what two peers reach, not what one holds. `TRAME-242` states the rule section 2 leans on. Grammar
and status values are normative in `ambre-spec-template.md`.

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TRAME-003` | `INV` | A row's or column's identity MUST be a `Uid` carried as the `LoroMovableList` element's **value**, never its position. | this spec section 2 | `ambre_trame::model::sheet_index::tests::build_maps_positions_to_stable_ids`; `::apply_move_row_permutes_order_and_reindexes`; `::cell_key_is_fixed_length_and_stable` -- the index; `ambre_trame::logic::tests::group4::sort_range_formulas_follow_data` -- the one that walks a real move on the `LoroMovableList` and reads the cells back under their unchanged keys, which is the hazard section 6 rejects the slot id for | none | verified |
| `TRAME-242` | `INV` | One session MUST NOT mint two equal row, column or sheet ids. | this spec section 2 | `ambre_core::authorship::tests::an_id_counter_ends_at_the_top_and_crosses_the_wire_bare` -- the step past `u32::MAX` answers no counter; `ambre_trame::logic::tests::group2::a_spent_uid_counter_refuses_new_lines_and_overwrites_nothing` -- two rows, two uids; `ambre_trame::logic::tests::group3::a_spent_sheet_counter_refuses_the_next_sheet_and_overwrites_nothing` -- the sheet that holds the last id keeps its name | none | verified |
| `TRAME-010` | `INV` | Computed values MUST NOT enter the CRDT: a container MUST hold the raw text alone. | this spec section 3 | `ambre_trame::logic::tests::group12::a_computed_value_stays_out_of_the_document` -- the cells map holds each raw text alone, and no result appears anywhere in the document | none | verified |
| `TRAME-221` | `CONC` | Two peers that exchanged their whole history MUST show the same computed value, and a peer rebuilt from the converged snapshot MUST show it too. | this spec section 3 | `ambre_trame::logic::convergence_tests::two_peers_converge_under_interleaved_ops` -- a proptest over 256 interleavings, comparing the rendered text of both peers, then of a third peer imported from the converged snapshot; `logic::tests::group1::sum_over_plain_literals_survives_reseed` -- the worked example; `ambre_belt::trame_grid::tests::two_peers_typing_at_once_converge_on_both_screens` -- two users in two browsers, two edits crossing, the text and the status bar total read on both screens | none | verified |
| `TRAME-016` | `INV` | A cell's whole style MUST be one binary register, keyed by that cell's identity. | this spec section 3; architecture section 2.D | `ambre_trame::logic::convergence_tests::subset_style_edit_does_not_tear_a_whole_cell_rewrite` -- it goes red on a split register, because a partial patch racing a whole rewrite is what a second style container tears on; `logic::tests::group1::bold_stays_on_the_cell_not_the_selection` -- the key | none | verified |
| `TRAME-222` | `CONC` | Two concurrent restyles of one cell MUST converge on the whole style one of the two wrote, never a per-attribute blend. | this spec section 3; architecture section 2.D | `ambre_trame::logic::convergence_tests::concurrent_whole_cell_style_edits_do_not_blend_across_containers`; `::subset_style_edit_does_not_tear_a_whole_cell_rewrite` | none | verified |
| `TRAME-270` | `INV` | A uid that one axis list holds twice MUST read as one line, at the position of its element with the smallest element id. | this spec section 2; `features/ambre-spec-trame-rows-and-columns.md section 3` (`TRAME-261`) | `ambre_trame::logic::tests::group14::a_uid_pushed_twice_reads_as_one_line_at_its_smallest_element`; `ambre_trame::model::axis::tests::a_repeated_uid_reads_as_one_line_at_its_smallest_element`; `ambre_trame::logic::tests::group14::four_peers::every_reference_reads_its_live_element_or_ref` -- the four-peer proptest, 512 cases, driven through `TrameState` and the formula engine | none | verified |
| `TRAME-271` | `INV` | A derived uid MUST NOT repeat a uid derived after the same line in the history the deriving peer holds. | this spec section 2; `features/ambre-spec-trame-rows-and-columns.md section 3` (`TRAME-267`) | `ambre_trame::logic::tests::group14::no_uid_comes_back::a_derived_uid_never_repeats_one_derived_after_the_same_line` -- a proptest, 256 cases: first writes, deletes and moves on one peer, and no derived uid seen twice | none | verified |
| `TRAME-272` | `IO` | A document of schema version 1 MUST read under version 2 with every line at its position and every cell on its line. | this spec section 7; `1-Foundations/ambre-spec-schema-version.md` (`SCHEMA-018`) | `ambre_trame::logic::tests::group14::a_version_1_document_reads_with_every_line_and_cell_in_place`; ci-gate -- `just fixtures-check`, run in the CI host job, migrates `fixtures/frozen/trame/formula_dag@v1-s1.ambre` and compares its dump with the live artifact (`FIXTURE-019`) | none | verified |
| `TRAME-275` | `INV` | Trame code outside the axis module MUST NOT reach an axis list except through the deduplicated axis view. | this spec section 5; `TRAME-270`; `features/ambre-spec-trame-rows-and-columns.md section 3` | type-level -- `ambre_trame::model::AxisView` keeps its fields and its reader private to `model/axis.rs`, and two `compile_fail` doctests of that module show a read of a list and a struct literal refused outside it; ci-gate -- `scripts/crate_boundaries.py`, run by `just crate-boundaries` in the CI `lint` job, refuses an axis family path, a raw list call and an axis container name as a literal in any Trame file outside `model/axis.rs`, above its tests; `ambre_trame::model::axis::tests::a_delete_drops_every_element_and_a_move_moves_the_visible_one` | none | verified |
| `TRAME-025` | `INV` | A Trame document MUST carry `meta.schema_version`. | architecture section 2.D; this spec section 7; `ambre-spec-schema-version.md`, the protocol it obeys | `ambre_trame::logic::schema::tests::a_new_workbook_carries_the_current_version` -- `schema::stamp` writes the key inside the scaffolding commit of `from_doc`, alongside the implicit sheet, so a workbook carries it before any user edit | none | verified |

* **Budget.** none: a format has no operation to budget. What loading one costs is legs 3 and 4 of
  `ambre-spec-budgets.md section 4`, and both are recorded there as unbudgeted.
* **Boundary.** none: `TRAME-010` keeps computed values out of the document, and the containers
  this table declares reach the relay as ciphertext alone (`ZK-001`).
* **Verification.** 11 `verified`. What holds is the shape a merge reads back. The identity a cell
  keeps, and the single register a style lives in. The key set a build writes, which a CI gate holds
  append-only. A computed value is looked for in the whole document, beside the raw text too.

---

## 2. IDENTITY

One identity type serves rows, columns and sheets.

* `Uid` is `{peer: PeerId, counter: IdCounter}`, rendered `"{peer:016x}_{counter:08x}"` -- 25
  characters.
* A uid is minted or derived. An insert, and a sheet, take a minted uid. A line that a write past
  the used range adds takes a derived uid (`features/ambre-spec-trame-rows-and-columns.md section 3`).
* In a minted uid, `peer` is the document `PeerID`; `counter` is a per-session monotonic counter.
* A derived uid is 96 bits of a hash, spelled the same way. A reader cannot tell the two forms
  apart, and needs not to. The generation its line before carries keeps it from repeating
  (`TRAME-271`).
* A derived line's list element spells `"{uid}#{element}"`, each half in the canonical spelling a
  reader holds to (`TRAME-248`). A minted line's element is the bare uid.
* A session mints each counter once, and past `u32::MAX` it mints no uid, so a uid never repeats
  (`TRAME-242`). What a write answers then is `TRAME-243`, in the engine feature.
* A cell key joins two halves with `:`: `"{row_element}:{col_element}"`, each half the spelling of
  the line's element. Two minted lines make the 51-character key of version 1. A derived half
  adds 26 characters (`features/ambre-spec-trame-rows-and-columns.md section 3`).
* A key or a line element reads back from that spelling alone. Another spelling of one uid
  would give it a second key, so a reader refuses it and says so (`TRAME-248`, in the engine
  feature). A stored formula writes its line uids in compact hex, a separate spelling its own
  parser reads.
* A `sheet_id` uses the same rendering, minted from the document `PeerID` and a sheet counter, an
  `IdCounter` too. The implicit first sheet is the constant `"s0"`.
* `crates/ambre-trame/src/model/sheet_index.rs` holds the type and the parser.

Both halves of the shape are decisions, with their rejected alternatives in section 6.

---

## 3. CONTAINERS

`State` is `built` when the container exists in `crates/ambre-trame/src/`, `design` when this
schema declares it and no code writes it yet. `LWW` names the unit at which two concurrent writers
race: the last writer takes that whole unit.

| Container | Loro type | Key | Value | LWW unit | Owning feature document | State |
|---|---|---|---|---|---|---|
| `meta` | Map | `schema_version`, `locale` | `u32`, a BCP 47 tag | the key | -- (foundation; this spec section 7); `locale`: `1-Foundations/ambre-spec-locale.md` | built |
| `workbook` | Map | `name`, `author`, `created` | String | the key | **-- (none; section 5)** | design |
| `sheet_order` | MovableList | position | `sheet_id` | list order, no LWW | `trame-sheets` | built |
| `sheets` | Map | `sheet_id` | sheet name (String) | the sheet entry | `trame-sheets` | built |
| `rows_{sheet}` | MovableList | position | `row_uid` | list order, no LWW | `trame-rows-and-columns` | built |
| `cols_{sheet}` | MovableList | position | `col_uid` | list order, no LWW | `trame-rows-and-columns` | built |
| `cells_{sheet}` | Map | `"{row_element}:{col_element}"` | raw text (String) | the cell | `trame-cell-editing` | built |
| `cellstyle_{sheet}` | Map | `"{row_element}:{col_element}"` | `Binary(postcard(StyleRecord))` | the whole cell style | `trame-cell-styles` | built |
| `merges_{sheet}` | Map | master `"{row_element}:{col_element}"` | bottom-right `"{row_element}:{col_element}"` | the merge | `trame-cell-merges` | built |
| `col_widths_{sheet}` | Map | `col_uid` | width in px (`f64`) | the column | `trame-resize` | built |
| `row_heights_{sheet}` | Map | `row_uid` | height in px (`f64`) | the row | `trame-resize` | built |
| `line_gen_{sheet}` | Map | `"{uid}:{generation:08x}"`; the axis root of an empty axis spells `rows` or `cols` in place of the uid | `true` | the key | `trame-rows-and-columns` | built |
| `filters_{sheet}` | Map | `active` | `Binary(postcard(FilterRecord))` | the whole filter | `trame-shared-filter` | built |
| `filter_hidden_{sheet}` | Map | `row_uid` | `true` | the row | `trame-shared-filter` | built |
| `named_ranges` | Map | range name | A1 reference (String) | the name | `trame-named-ranges` | built |
| `styles` | Map | `style_id` | style attribute map | the style entry | `trame-cell-styles` | design |
| `media` | Map | media address, keyed by the document key | `{dims, anchor, mime}` | the media entry | -- (foundation media flow) | design |
| `cond_formats_{sheet}` | List | position | `{range, priority, rule, style}` | list order, no LWW | `trame-cell-styles` | design |
| `validations_{sheet}` | Map | `range` | `{kind, params, on_invalid, message}` | the range entry | `trame-cell-editing` | design |
| `protections_{sheet}` | Map | `range` | `{owner, allowed, description}` | the range entry | `trame-cell-editing` | design |
| `page_setup_{sheet}` | Map | field name | orientation, paper size, margins, scale, flags | the key | `trame-pdf-export` | design |

**Absence is a value.** No key in `cellstyle_{sheet}` means the grid default style; no key in
`col_widths_{sheet}` means `COL_WIDTH`; an empty `filters_{sheet}` means no filter. No
container carries a "cleared" marker.

---

## 4. HIERARCHY

* `LoroDoc` root
  * `meta`
  * `workbook`
  * `sheet_order`
  * `sheets`
  * `named_ranges`
  * `styles`
  * `media`
  * per sheet, named by `sheet_id`:
    * `rows_{sheet}`, `cols_{sheet}`, `line_gen_{sheet}` -- the axes
    * `cells_{sheet}`, `cellstyle_{sheet}`, `merges_{sheet}` -- the grid content
    * `col_widths_{sheet}`, `row_heights_{sheet}` -- the metrics
    * `filters_{sheet}`, `filter_hidden_{sheet}`, `cond_formats_{sheet}`, `validations_{sheet}`,
      `protections_{sheet}`, `page_setup_{sheet}` -- the data tools

---

## 5. CONTAINER NOTES

Only where a schema decision needs a reason. 40 words each.

* **`workbook`.** Declared, unwritten, owned by no feature document. A workbook name is not a sheet,
  so `trame-sheets` does not claim it. No rule says what a peer reads when the three keys are
  absent.
* **`sheet_order`.** The implicit sheet `"s0"` is never pushed: two fresh peers would each push it
  and converge to a duplicate tab. It is the implicit first entry, and explicit sheets append after
  it.
* **`sheets`.** The name is stored flat. The `{name: LoroText, hidden, tab_color}` sub-map is
  declared and unbuilt; `LoroText` on the name makes two concurrent renames merge per character
  rather than race whole.
* **`rows_{sheet}`, `cols_{sheet}`.** A derived line's element is `"{uid}#{element}"`. Two peers
  deriving one line push two elements. The smallest element id is the line. No write removes the
  other, since a reference may name it (`TRAME-270`). One module owns both lists (`TRAME-275`).
* **`line_gen_{sheet}`.** A grow-only set: two peers adding one key merge into one. A derivation
  drops the key below the one it adds, and a delete purges its line's keys (`TRAME-276`).
* **`cells_{sheet}`.** The raw text is stored flat. The `{v, t, s}` sub-map is declared and unbuilt;
  the `s` indirection into `styles` is a deduplication, deferred behind a benchmark.
* **`cellstyle_{sheet}`.** One register per cell, not one per attribute: a fan-out lets a colour
  update racing a border update converge half-applied. `postcard` decodes the blob whole, so a
  layout change is a schema change (`SCHEMA-018`).
* **`merges_{sheet}`.** Keyed by the master corner, so a merge follows its cells through an insert
  or a delete of a line above it.
* **`filters_{sheet}`.** One register holds the definition whole, as `cellstyle_{sheet}` does. Two
  peers filtering at the same moment race exactly once, so the winner is one writer's, never a field
  blend (`TRAME-FEAT-025`).
* **`filter_hidden_{sheet}`.** Kept out of `rows_{sheet}` so a hidden flag never disturbs the order
  list, and keyed by `row_uid` so it follows the data through a sort.
* **`media`.** The entry holds the content hash and the placement, never the pixels. The bytes travel
  the shared media flow of `architecture section 2.D`, so a peer resolves a hash without the document
  carrying it.
* **`page_setup_{sheet}`.** Persisted state, not an export parameter: it converges across peers and
  survives a session (`TRAME-IO-022`). `orientation` is a field, never derived from paper dimensions.

---

## 6. REJECTED ALTERNATIVES

Section 3 declares no anchored-comment container, a parity gap with Prose's `annots` subsystem
(`4-Prose/features/ambre-spec-prose-comments.md`). The shape is settled ahead of the container. A
note keys on the move-invariant `(row_uid, col_uid)` pair, so it follows its cell through a sort and
a reorder. A delete of a bound line orphans it under the Prose model. Deciding it here is what
stops the eventual container being keyed positionally by whoever writes it first.

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| A minted `Uid` carried as the element value | a positional `R{row}C{col}` key | inserting a row at position 5 invalidates every key below it, O(n) CRDT operations against one |
| A minted `Uid` carried as the element value | the Loro slot id (`ListItemId`) | `LoroMovableList::mov` **reassigns the moved element's op id**, so every cell keyed by that slot is orphaned on the first reorder. This reverses the earlier "Loro already gives stable ids, a uid is redundant" call: it holds for insert and delete, never for `mov` |
| A minted `Uid` carried as the element value | sorting by rewriting cell contents into fixed slots | the workaround for the slot id: O(cells) instead of O(rows), every moved cell replicates and lands on the undo stack, and an id-bound reference stays pinned to the overwritten slot |
| A derived uid for a line a write adds, a minted one for an insert | a minted uid for every line | two peers writing at once in one new line add two lines, and one value moves a row down on both screens (`TRAME-261`) |
| A derived uid for a line a write adds, a minted one for an insert | a derived uid for every line, an insert included | two peers inserting at one position mean two lines (`TRAME-091`); one derived uid would make them one |
| `{peer:016x}_{counter:08x}`, fixed width, `:` between the halves | `{client_hex}{counter_hex}` with no separator | ambiguous: without a separator the two fixed-width halves cannot be told apart once either widens |
| `{peer:016x}_{counter:08x}`, fixed width, `:` between the halves | a document-global monotonic counter | one LWW collision per concurrent insert. Two peers minting concurrently draw a distinct `peer` instead, so identity needs no coordination |
| A dense axis over the used range | sparse run-length rows with a shared `gap_before` counter and an order-statistic tree | **not convergent:** two concurrent inserts splitting the same gap both `set` the shared counter, LWW keeps one, and the absolute numbering of every following row corrupts identically on all replicas. The tree speeds the query, not the hazard; the materialization cap bounds the memory the sparse model aimed at |
| One binary register for a whole cell style | one register per attribute (the old `format_` / `style_` / `fgcolor_` fan-out) | cheap only with a server that orders content; on a blind relay, per-register LWW has no cross-register atomicity, so a whole-cell edit tears (border presence against border colour) and a colour-only or font-only cell was lost on reload. A finer model, if ever justified, is a nested per-cell map, never a flat fan-out |
| Raw text only, never a computed value | storing the computed value beside the raw text | it is derived from the raw text, so replicating it costs bandwidth and can disagree with a peer's own recompute |
| A shared `styles` dictionary, declared and deferred | building the `s` indirection now | a lookup paid before its benchmark: deduplication is a size optimization and the size is unmeasured |
| One binary register for the whole filter definition | five flat keys, or a nested `active` sub-map | the fields race one by one and converge torn (`TRAME-FEAT-025`); one register holds column and kind together, so the winner is one writer's whole filter |
| A container name as the primary key | a numeric container id | one more indirection, no new information: `merges_{sheet}` is already unique, stable, meaningful and verbatim in the code |
| No anchored-comment container, and the anchor already decided | a positional `R{row}C{col}` anchor | every insert above the note re-points it, the same failure the row uid exists to answer |
| No anchored-comment container, and the anchor already decided | a Trame-private orphan model | one gesture, two behaviours across the suite, and two rules to keep in step |

---

## 7. FORMAT DUTIES

| Duty | Rule | Where |
|---|---|---|
| Version | root `meta`, key `schema_version` (`u32`); absent means 0 | architecture section 2.D; `TRAME-025` |
| Read | equal proceeds, older migrates, newer is refused whole (`SchemaError::DocumentTooNew`), read in a scratch document so a refusal writes nothing | architecture section 2.D; `ambre-spec-schema-version.md section 2` |
| Migration | the registry lives in `ambre-trame` (`logic/schema.rs`), one row per bump; the foundation owns the protocol, never the semantics. Version 1 is the first layout. The rows from 1 to 2 and from 2 to 3 stamp the register and rewrite nothing | architecture section 2.D; `SCHEMA-010` |
| Bump | a key, a value type or the meaning of a value changes: `SCHEMA_VERSION` rises and one named fixture is frozen in the same commit | `SCHEMA-018`; `ambre-spec-schema-version.md` `SCHEMA-017`; `ambre-spec-fixtures.md section 2` |
| Bump to version 2 | four changes. A cell, style or merge key spells each line's element, so a cell follows its element through a delete (`TRAME-265`). `line_gen_{sheet}` is a new key. A uid listed twice changes its meaning: a version-1 reader would show the line twice. A derived element spells `"{uid}#{element}"`, which a version-1 reader refuses (`TRAME-248`). The migration from 1 rewrites nothing: a version-1 element is a bare minted uid, and its element id is that uid. A version-1 cell key therefore names its elements already. A version-1 reference carries no element id and binds to that element, which no derived uid can equal. A version-1 fixture is frozen in the commit that bumps |
| Bump to version 3 | `meta.locale` is a new key (`1-Foundations/ambre-spec-locale.md`). The migration from 2 rewrites nothing: an absent key is no language. A version-2 fixture is frozen in the commit that bumps |
| Unknown field | `postcard` decodes a blob whole and tolerates none, so a `StyleRecord` written by a newer client is skipped -- no longer silently: the reseed raises one warning per import (`SCHEMA-003`) | architecture section 2.D |
| Size, axes | the used range grows by at most 10 000 lines per write, and holds at most 1 000 columns | `features/ambre-spec-trame-rows-and-columns.md` |
| Size, keys | a cell key is 51 bytes on two minted lines, and up to 103 on two derived ones; a uid is 25; a 100 000-row axis is 2.5 MB of element values alone. A generation key is 34 bytes, and the record keeps about one per line that has derived a line after it. A derived element is 51 bytes, and a reference to it carries up to 26 more | this spec section 2 |
| Deltas | a remote delta carries no version and applies blind; mixed-version live editing is bounded, not solved | architecture section 2.D |

---

## 8. WHAT THIS DOCUMENT DOES NOT OWN

| Question | Owner |
|---|---|
| When a merge is refused, what an unmerge restores | the cell-merges feature |
| That a sort is a permutation and a range stays a positional block | the sort feature; `TRAME-004`, `TRAME-005` |
| When a reference becomes `#REF!` | the formula engine; `TRAME-007`, `TRAME-008` |
| That one command is one undo item | the engine; `TRAME-017` |
| That a document newer than the binary is refused whole | `ambre-spec-schema-version.md`; `SCHEMA-008` |
| Who may write, and what the server enforces | architecture section 2.D; `ambre-spec-server.md` |
