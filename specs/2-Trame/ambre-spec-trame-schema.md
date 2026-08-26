# AMBRE -- SPEC: TRAME CRDT SCHEMA
**Type:** Schema (persistence format, one document per application)
**Implementation:** partial

> The persisted shape of a Trame workbook: every Loro container, its key, its value, and the
> granularity at which two peers race. It owns identity, keys, merge granularity and the convergence
> properties that follow from that shape. It owns no gesture, no permission and no event.

---

## 0. SCOPE, AND THE MENTION RULE

A schema decision is often justified by an action: a row identity survives a sort, a delete and a
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
not of the engine. Grammar and status values are normative in `ambre-spec-template.md`.

| ID | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|
| `TRAME-003` | A row's or column's identity MUST be a minted `Uid` carried as the `LoroMovableList` element's **value**, never its position. | this spec section 2 | `ambre_trame::model::sheet_index::tests::build_maps_positions_to_stable_ids`, `::apply_move_row_permutes_order_and_reindexes`, `::cell_key_is_fixed_length_and_stable` | verified |
| `TRAME-010` | Computed values MUST NOT enter the CRDT: only the raw `v` is shared and each peer recomputes independently. | this spec section 3 | none. `ambre_trame::logic::tests::group1::sum_over_plain_literals_survives_reseed` covers recompute-after-reseed; **nothing asserts the absence of computed values from the document**, which is the half a remote peer depends on | unverified |
| `TRAME-016` | A cell's whole style MUST be a single LWW register; concurrent restyles MUST NOT blend per attribute. | this spec section 3; architecture section 2.D | `ambre_trame::logic::convergence_tests::concurrent_whole_cell_style_edits_do_not_blend_across_containers`, `::subset_style_edit_does_not_tear_a_whole_cell_rewrite`, `logic::tests::group1::bold_stays_on_the_cell_not_the_selection` | verified |
| `TRAME-025` | A Trame document MUST carry `meta.schema_version`, and a document whose version exceeds the compiled `SCHEMA_VERSION` MUST be refused whole. | architecture section 2.D; this spec section 7 | not built -- lands with `ARCH P8` | unimplemented |
| `TRAME-026` | A change to a container's key shape or value layout MUST raise `SCHEMA_VERSION` in the same commit as the change. | architecture section 2.D; `ambre-spec-fixtures.md section 2` | not built -- lands with `ARCH P8` | unimplemented |

* **Budget.** none: a format has no operation to budget. What loading one costs is legs 3 and 4 of
  `ambre-spec-budgets.md section 4`, and both are recorded there as unbudgeted.
* **Boundary.** none: `TRAME-010` keeps computed values out of the document, and the containers
  this table declares reach the relay as ciphertext alone (`ZK-001`).
* **Verification.** 2 `verified`, 1 `unverified`, 2 `unimplemented`. `TRAME-010` is the gap: a
  regression is invisible locally and corrupts every peer. Reported in `ambre-spec-testing.md
  section 7`.

---

## 2. IDENTITY

One identity type serves rows, columns and sheets.

* `Uid` is `{peer: u64, counter: u32}`, rendered `"{peer:016x}_{counter:08x}"` -- 25 characters.
* `peer` is the document `PeerID`; `counter` is a per-session monotonic counter.
* A cell key joins two halves with `:`: `"{row_uid}:{col_uid}"`, 51 characters, fixed length.
* A `sheet_id` uses the same rendering, minted from the document `PeerID` and a sheet counter. The
  implicit first sheet is the constant `"s0"`.
* `crates/ambre-trame/src/model/sheet_index.rs` holds the type and the parser.

Both halves of the shape are decisions, with their rejected alternatives in section 6.

---

## 3. CONTAINERS

`State` is `built` when the container exists in `crates/ambre-trame/src/`, `design` when this
schema declares it and no code writes it yet. `LWW` names the unit at which two concurrent writers
race: the last writer takes that whole unit.

| Container | Loro type | Key | Value | LWW unit | Owning feature document | State |
|---|---|---|---|---|---|---|
| `meta` | Map | `schema_version` | `u32` | the key | -- (foundation; this spec section 7) | design |
| `workbook` | Map | `name`, `author`, `created` | String | the key | **-- (none; section 5)** | design |
| `sheet_order` | MovableList | position | `sheet_id` | list order, no LWW | `trame-sheets` | built |
| `sheets` | Map | `sheet_id` | sheet name (String) | the sheet entry | `trame-sheets` | built |
| `rows_{sheet}` | MovableList | position | `row_uid` | list order, no LWW | `trame-rows-and-columns` | built |
| `cols_{sheet}` | MovableList | position | `col_uid` | list order, no LWW | `trame-rows-and-columns` | built |
| `cells_{sheet}` | Map | `"{row_uid}:{col_uid}"` | raw text (String) | the cell | `trame-cell-editing` | built |
| `cellstyle_{sheet}` | Map | `"{row_uid}:{col_uid}"` | `Binary(postcard(StyleRecord))` | the whole cell style | `trame-cell-styles` | built |
| `merges_{sheet}` | Map | master `"{row_uid}:{col_uid}"` | bottom-right `"{row_uid}:{col_uid}"` | the merge | `trame-cell-merges` | built |
| `col_widths_{sheet}` | Map | `col_uid` | width in px (`f64`) | the column | `trame-resize` | built |
| `row_heights_{sheet}` | Map | `row_uid` | height in px (`f64`) | the row | `trame-resize` | built |
| `filters_{sheet}` | Map | `col`, `kind`, `op`, `value`; child list `values` | String, and a list of String under `values` | **the key, not the filter** | `trame-shared-filter` | built |
| `filter_hidden_{sheet}` | Map | `row_uid` | `true` | the row | `trame-shared-filter` | built |
| `named_ranges` | Map | range name | A1 reference (String) | the name | `trame-named-ranges` | built |
| `styles` | Map | `style_id` | style attribute map | the style entry | `trame-cell-styles` | design |
| `media` | Map | `blake2b` content hash | `{dims, anchor, mime}` | the media entry | -- (foundation media flow) | design |
| `cond_formats_{sheet}` | List | position | `{range, priority, rule, style}` | list order, no LWW | `trame-cell-styles` | design |
| `validations_{sheet}` | Map | `range` | `{kind, params, on_invalid, message}` | the range entry | `trame-cell-editing` | design |
| `protections_{sheet}` | Map | `range` | `{owner, allowed, description}` | the range entry | `trame-cell-editing` | design |
| `page_setup_{sheet}` | Map | field name | orientation, paper size, margins, scale, flags | the key | `trame-pdf-export` | design |

**Absence is a value.** No key in `cellstyle_{sheet}` means the grid default style; no key in
`col_widths_{sheet}` means `DEFAULT_COL_WIDTH`; an empty `filters_{sheet}` means no filter. No
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
    * `rows_{sheet}`, `cols_{sheet}` -- the axes
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
* **`cells_{sheet}`.** The raw text is stored flat. The `{v, t, s}` sub-map is declared and unbuilt;
  the `s` indirection into `styles` is a deduplication, deferred behind a benchmark.
* **`cellstyle_{sheet}`.** One register per cell, not one per attribute: a fan-out lets a colour
  update racing a border update converge half-applied. `postcard` decodes the blob whole, so a
  layout change is a schema change (`TRAME-026`).
* **`merges_{sheet}`.** Keyed by the master corner, so a merge follows its cells through an insert
  or a delete of a line above it.
* **`filters_{sheet}`.** Fields are separate keys, so two peers filtering at the same moment elect
  each field on its own and can converge on a definition neither wrote. One register would hold the
  definition whole, as `cellstyle_{sheet}` does (`TRAME-FEAT-025`).
* **`filter_hidden_{sheet}`.** Kept out of `rows_{sheet}` so a hidden flag never disturbs the order
  list, and keyed by `row_uid` so it follows the data through a sort.
* **`media`.** The entry holds the content hash and the placement, never the pixels. The bytes travel
  the shared media flow of `architecture section 2.D`, so a peer resolves a hash without the document
  carrying it.
* **`page_setup_{sheet}`.** Persisted state, not an export parameter: it converges across peers and
  survives a session (`TRAME-IO-022`). `orientation` is a field, never derived from paper dimensions.

---

## 6. SCHEMA DECISIONS

| Decision | Rationale | Rejected alternative |
|---|---|---|
| A minted `Uid` carried as the element value | `mov` leaves an element's value untouched, so a sort is one order permutation and no cell key is rewritten | (a) positional `R{row}C{col}` -- inserting a row at position 5 invalidates every key below it, O(n) CRDT operations against one; (b) the Loro slot id (`ListItemId`) -- `LoroMovableList::mov` **reassigns the moved element's op id**, so every cell keyed by that slot is orphaned on the first reorder. This reverses the earlier "Loro already gives stable ids, a uid is redundant" call: it holds for insert and delete, never for `mov`; (c) sort by rewriting cell contents into fixed slots -- the workaround for (b): O(cells) instead of O(rows), every moved cell replicates and lands on the undo stack, and an id-bound reference stays pinned to the overwritten slot |
| `{peer:016x}_{counter:08x}`, fixed width, `:` between the halves | two peers minting concurrently draw a distinct `peer`, so identity needs no coordination; a fixed width makes a cell key parseable by offset | `{client_hex}{counter_hex}` with no separator -- ambiguous; a document-global monotonic counter -- one LWW collision per concurrent insert |
| A dense axis over the used range | list index equals grid position, so resolving `A5` is one lookup, and each line is its own element with no shared state | sparse run-length rows with a shared `gap_before` counter and an order-statistic tree. **Not convergent:** two concurrent inserts splitting the same gap both `set` the shared counter, LWW keeps one, and the absolute numbering of every following row corrupts identically on all replicas. The tree speeds the query, not the hazard; the materialization cap bounds the memory the sparse model aimed at |
| One binary register for a whole cell style | LWW on one blob converges every peer to one writer's whole style | one register per attribute (the old `format_` / `style_` / `fgcolor_` fan-out). Cheap only with a server that orders content; on a blind relay, per-register LWW has no cross-register atomicity, so a whole-cell edit tears (border presence against border colour) and a colour-only or font-only cell was lost on reload. A finer model, if ever justified, is a nested per-cell map, never a flat fan-out |
| Raw text only, never a computed value | a computed value is derived from the raw text, so replicating it costs bandwidth and can disagree with a peer's own recompute | storing the computed value beside the raw text |
| A shared `styles` dictionary, declared and deferred | deduplication is a size optimization and the size is unmeasured | building the `s` indirection now -- a lookup paid before its benchmark |
| The filter definition sits flat in `filters_{sheet}` | one container fewer to merge, for the same LWW granularity -- which is the key either way | a nested `active` sub-map. **Neither shape holds the definition together**: the fields race one by one and converge torn (`TRAME-FEAT-025`), so the open choice is one register against five keys, not flat against nested |
| A container name as the primary key | `merges_{sheet}` is unique, stable, meaningful and verbatim in the code | a numeric container id -- one more indirection, no new information |
| No anchored-comment container, and the anchor already decided | Section 3 declares none, a parity gap with Prose's `annots` subsystem (`4-Prose/features/ambre-spec-prose-comments.md`). The shape is settled ahead of the container: a note keys on the move-invariant `(row_uid, col_uid)` pair, so it follows its cell through a sort and a reorder, and a delete of a bound line orphans it under the Prose model. Deciding it here is what stops the eventual container being keyed positionally by whoever writes it first | (a) a positional `R{row}C{col}` anchor -- every insert above the note re-points it, the same failure the row uid exists to answer; (b) a Trame-private orphan model -- one gesture, two behaviours across the suite, and two rules to keep in step |

---

## 7. FORMAT DUTIES

| Duty | Rule | Where |
|---|---|---|
| Version | root `meta`, key `schema_version` (`u32`); absent means 0 | architecture section 2.D; `TRAME-025` |
| Read | equal proceeds, older migrates, newer is refused whole (`SchemaTooNew`) | architecture section 2.D |
| Migration | the chain lives in `ambre-trame`; the foundation owns the protocol, never the semantics | architecture section 2.D |
| Bump | a key or layout change raises `SCHEMA_VERSION` and freezes one named fixture in the same commit | `TRAME-026`; `ambre-spec-fixtures.md section 2` |
| Unknown field | `postcard` decodes a blob whole and tolerates none, so a stale `StyleRecord` falls back to `Default` -- a silent downgrade, which is what `TRAME-025` exists to stop | architecture section 2.D |
| Size, axes | the used range grows by at most 10 000 lines per write, and holds at most 1 000 columns | `features/ambre-spec-trame-rows-and-columns.md` |
| Size, keys | a cell key is 51 bytes, a uid 25; a 100 000-row axis is 2.5 MB of element values alone | this spec section 2 |
| Deltas | a remote delta carries no version and applies blind; mixed-version live editing is bounded, not solved | architecture section 2.D |

---

## 8. WHAT THIS DOCUMENT DOES NOT OWN

| Question | Owner |
|---|---|
| When a merge is refused, what an unmerge restores | the cell-merges feature |
| That a sort is a permutation and a range stays a positional block | the sort feature; `TRAME-004`, `TRAME-005` |
| When a reference becomes `#REF!` | the formula engine; `TRAME-007`, `TRAME-008` |
| That one command is one undo item | the engine; `TRAME-017` |
| Who may write, and what the server enforces | architecture section 2.D; `ambre-spec-server.md` |
