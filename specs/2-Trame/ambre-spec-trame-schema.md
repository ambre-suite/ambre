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
what two peers reach, not what one holds. Grammar and status values are normative in `ambre-spec-template.md`.

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TRAME-003` | `INV` | A row's or column's identity MUST be a minted `Uid` carried as the `LoroMovableList` element's **value**, never its position. | this spec section 2 | `ambre_trame::model::sheet_index::tests::build_maps_positions_to_stable_ids`; `::apply_move_row_permutes_order_and_reindexes`; `::cell_key_is_fixed_length_and_stable` -- the index; `ambre_trame::logic::tests::group4::sort_range_formulas_follow_data` -- the one that walks a real move on the `LoroMovableList` and reads the cells back under their unchanged keys, which is the hazard section 6 rejects the slot id for | none | verified |
| `TRAME-010` | `INV` | Computed values MUST NOT enter the CRDT: a container MUST hold the raw text alone. | this spec section 3 | `ambre_trame::logic::tests::group1::sum_over_plain_literals_survives_reseed` -- it reads the raw text back after a snapshot round trip | partial -- a computed value **overwriting** the raw text goes red. A computed value stored **beside** it, the `t` of the declared, unbuilt `{v, t, s}` sub-map of section 5, leaves every test green and costs every peer the bandwidth; lands with the `The Trame belts` Story (`ambre-roadmap.md` E9) | untested |
| `TRAME-221` | `CONC` | Two peers that exchanged their whole history MUST show the same computed value, and a peer rebuilt from the converged snapshot MUST show it too. | this spec section 3 | `ambre_trame::logic::convergence_tests::two_peers_converge_under_interleaved_ops` -- a proptest over 256 interleavings, comparing the rendered text of both peers, then of a third peer imported from the converged snapshot; `logic::tests::group1::sum_over_plain_literals_survives_reseed` -- the worked example | none | verified |
| `TRAME-016` | `INV` | A cell's whole style MUST be one binary register, keyed by that cell's identity. | this spec section 3; architecture section 2.D | `ambre_trame::logic::convergence_tests::subset_style_edit_does_not_tear_a_whole_cell_rewrite` -- it goes red on a split register, because a partial patch racing a whole rewrite is what a second style container tears on; `logic::tests::group1::bold_stays_on_the_cell_not_the_selection` -- the key | none | verified |
| `TRAME-222` | `CONC` | Two concurrent restyles of one cell MUST converge on the whole style one of the two wrote, never a per-attribute blend. | this spec section 3; architecture section 2.D | `ambre_trame::logic::convergence_tests::concurrent_whole_cell_style_edits_do_not_blend_across_containers`; `::subset_style_edit_does_not_tear_a_whole_cell_rewrite` | none | verified |
| `TRAME-025` | `INV` | A Trame document MUST carry `meta.schema_version`. | architecture section 2.D; this spec section 7; `ambre-spec-schema-version.md`, the protocol it obeys | `ambre_trame::logic::schema::tests::a_new_workbook_carries_the_current_version` -- `schema::stamp` writes the key inside the scaffolding commit of `from_doc`, alongside the implicit sheet, so a workbook carries it before any user edit | none | verified |
| `TRAME-026` | `INV` | A change to a container's key set, or to the type a key's value holds, MUST raise `SCHEMA_VERSION` in the same commit as the change. | architecture section 2.D; `ambre-spec-fixtures.md section 2` | none | unheld -- nothing records the key set and the value types a build writes, so nothing notices them move. A shape snapshot frozen under `fixtures/frozen/trame/` and named by `SCHEMA_VERSION` would hold it. The freeze is append-only, so turning the gate green forces a snapshot at a higher version. `TRAME-224` is the half that stays human review; lands with the `The Trame belts` Story (`ambre-roadmap.md` E9) | untested |
| `TRAME-224` | `INV` | A change to what a key's value means, the key and its type unchanged, MUST raise `SCHEMA_VERSION` in the same commit as the change. | architecture section 2.D; `ambre-spec-fixtures.md section 2` | human-review -- the reviewer reads the diff for a unit or a reference point that moved under an unchanged key, which is the one drift no snapshot can show | unheld -- no gate is possible. `col_widths_{sheet}` holds a width in pixels. A build that reads the same `f64` as a count of characters writes the same key and the same type. Every shape snapshot stays green, and every document written before the change is read wrong | human-review |

* **Budget.** none: a format has no operation to budget. What loading one costs is legs 3 and 4 of
  `ambre-spec-budgets.md section 4`, and both are recorded there as unbudgeted.
* **Boundary.** none: `TRAME-010` keeps computed values out of the document, and the containers
  this table declares reach the relay as ciphertext alone (`ZK-001`).
* **Verification.** 5 `verified`, 2 `untested`, 1 `human-review`. What holds is the shape a merge
  reads back. The
  identity a cell keeps, the single register a style lives in, and the `meta.schema_version` every
  document carries. What is loose is `TRAME-010`, narrower than it reads: only a computed value
  stored **beside** the raw text escapes (`ambre-spec-testing.md section 7`). The version bump no
  test can reach is loose too.

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
| `meta` | Map | `schema_version` | `u32` | the key | -- (foundation; this spec section 7) | built |
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
| `filters_{sheet}` | Map | `active` | `Binary(postcard(FilterRecord))` | the whole filter | `trame-shared-filter` | built |
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
| Migration | the registry lives in `ambre-trame` (`logic/schema.rs`), one row per bump; the foundation owns the protocol, never the semantics | architecture section 2.D; `SCHEMA-010` |
| Bump | a key, a value type or the meaning of a value changes: `SCHEMA_VERSION` rises and one named fixture is frozen in the same commit | `TRAME-026`; `TRAME-224`; `ambre-spec-fixtures.md section 2` |
| Unknown field | `postcard` decodes a blob whole and tolerates none, so a `StyleRecord` written by a newer client is skipped -- no longer silently: the reseed raises one warning per import (`SCHEMA-003`) | architecture section 2.D |
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
| That a document newer than the binary is refused whole | `ambre-spec-schema-version.md`; `SCHEMA-008` |
| Who may write, and what the server enforces | architecture section 2.D; `ambre-spec-server.md` |
