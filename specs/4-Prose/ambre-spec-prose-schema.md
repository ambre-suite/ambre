# AMBRE -- SPEC: PROSE CRDT SCHEMA
**Type:** Schema (persistence format, one document per application)
**Implementation:** partial -- a footnote is the one annotation entry written

> The persisted shape of a Prose document: every Loro container, its key, its value, and the
> granularity at which two peers race. It owns identity, keys, merge granularity and the convergence
> properties that follow from that shape. It owns no gesture, no permission and no event.

---

## 0. SCOPE, AND THE MENTION RULE

A schema decision is often justified by an action. A block survives a move, an indent and a split,
and Loro's own `TreeID` answers concurrent minting. So an action appears here as a **cause**, never
as a **rule**.

| Position | Actions | Why |
|---|---|---|
| Container table (section 3), hierarchy (section 4) | forbidden | pure format |
| Container note (section 5), `Rationale` of a decision (section 6) | allowed, as a cause | the reason is the action |
| An invariant statement (section 1) | forbidden when it binds an action | "a footnote MUST NOT be inserted in a header" belongs to the footnotes feature |

**Budgets**, each with the destination its overflow takes:

| Cap | Overflow goes to |
|---|---|
| 600 words of prose for the whole document, outside tables and lists | a table, or nothing |
| 40 words for a container note (section 5) | section 6, whose `Rationale` column is not capped |

---

## 1. INVARIANTS

Prefix `PROSE`, shared with the feature documents: one prefix, one numbering, no renumbering.
`PROSE-002`, `-007`, `-008` and `-027` sit here because each states a property of the format, not
of the engine. `PROSE-089`, `-090`, `-091` and `-094` are the halves those four hid. Three of
them state what two peers reach rather than what one peer holds. `PROSE-085` to `-088`, `-093`,
`-095` and `-096` state format rules sections 2 to 5 already lean on and no row carried. Grammar and
status values are normative in `ambre-spec-template.md`.

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-085` | `INV` | A block's identity MUST be the `TreeID` the tree mints; a position among siblings MUST NOT be used as a key. | this spec section 2 | structural -- `ambre_prose::doc::body_key` addresses a block by the `TreeID` alone, and no key of the node map holds a sibling position; `ambre_prose::logic::convergence_tests::a_block_a_peer_moved_keeps_every_reference_to_it` | none | verified |
| `PROSE-086` | `INV` | A reference to a block MUST still resolve after a move, an indent and a move into a table cell. | this spec section 2 | `ambre_prose::logic::blocks::tests::a_reference_survives_an_indent_and_a_move_into_a_table_cell`; `::a_reference_to_a_block_survives_a_move`; `ambre_prose::logic::convergence_tests::a_block_a_peer_moved_keeps_every_reference_to_it` | none | verified |
| `PROSE-087` | `INV` | An annotation's identity MUST be a `Uuid` minted by the peer that creates it, carried as the value of the mark that anchors it. | this spec section 2 | `ambre_prose::logic::notes::tests::a_note_lands_as_one_call_and_an_entry_holding_its_text`; `::a_note_keeps_its_identity_through_a_move_an_edit_and_a_remote_change` | none | verified |
| `PROSE-088` | `INV` | A body key MUST re-read as the pair it joins -- the tag and the tree id -- split on a separator neither half can carry. | this spec section 2 | `ambre_prose::doc::tests::a_body_key_re_reads_as_the_pair_it_joins`; `::neither_half_of_a_body_key_carries_the_separator`; `::a_name_that_is_not_a_body_key_reads_as_none` | none | verified |
| `PROSE-118` | `INV` | A user's deletion of a block MUST keep the node and its `TreeID`. | this spec section 2 | `ambre_prose::logic::blocks::tests::a_deleted_block_keeps_its_node_and_names_the_session_that_hid_it` | none | verified |
| `PROSE-122` | `INV` | A hidden block MUST carry the peer id of the session that deleted it. | this spec section 2 | `ambre_prose::logic::blocks::tests::a_deleted_block_keeps_its_node_and_names_the_session_that_hid_it` | none | verified |
| `PROSE-119` | `INV` | A hidden block, and every block under it, MUST read as gone to every reader of the document. | this spec section 2 | `ambre_prose::logic::blocks::tests::a_hidden_table_hides_every_block_under_it` | none | verified |
| `PROSE-120` | `INV` | An undo of a deletion MUST bring the block back under its own `TreeID`, with its body. | this spec section 2 | `ambre_prose::logic::blocks::tests::an_undone_deletion_brings_the_block_back_whole` | none | verified |
| `PROSE-121` | `INV` | A hidden block MUST NOT be removed for good while the session named in its marker still runs. | this spec section 2 | `ambre_prose::logic::sweep::tests::a_block_hidden_by_a_running_session_stays` -- the session's own pass, and a peer holding its caret, keep the block, and its undo still restores it; `::a_block_hidden_by_a_session_that_ended_is_removed_for_good` -- the converse, carried to the other peer; `::a_silent_session_runs_once_it_has_announced_itself` -- an announce, or a caret this peer cannot place, keeps the block; `::a_newcomer_is_answered_once`; `::a_session_whose_user_left_has_ended`; `::the_removal_is_no_step_of_the_undo_stack` | none | verified |
| `PROSE-002` | `INV` | All editable text MUST live in `LoroText`; a `LoroMap` MUST NOT hold body text. | this spec section 3 | structural -- `ambre_prose::doc::get_block_text` opens one root `LoroText` per leaf block, and `ambre_prose::doc::create_block` writes no text into the node map; `ambre_prose::logic::convergence_tests::two_peers_typing_in_one_paragraph_keep_both_sentences` | none | verified |
| `PROSE-089` | `CONC` | Two peers typing in the same paragraph MUST both keep their text: one author's sentence MUST NOT be lost to the other's write. | this spec section 3; this spec section 6 | `ambre_prose::logic::convergence_tests::two_peers_typing_in_one_paragraph_keep_both_sentences`; `::an_offline_edit_and_a_concurrent_one_both_land` | none | verified |
| `PROSE-007` | `INV` | A mark's expansion behaviour MUST come from `expand_type_for` alone: no call site MUST decide it. | this spec section 3 | structural -- `ambre_prose::mark::configure` builds the document's whole style configuration from `expand_type_for`, and no write path names an expansion; `ambre_prose::mark::tests::a_formatting_mark_swallows_an_insertion_at_its_edge`; `::an_anchor_mark_never_swallows_an_insertion` | none | verified |
| `PROSE-090` | `INV` | `expand_type_for` MUST be total: an unknown key MUST return a default, never a panic. | this spec section 3 | `ambre_prose::mark::tests::an_unknown_key_answers_the_default_and_never_panics`; `::a_header_field_is_read_under_its_own_root` | none | verified |
| `PROSE-008` | `INV` | A mark value MUST be encoded as a canonical string: a value under a numeric key of section 3 in `f32::to_string()` form, and every other key's value as the user wrote it. | this spec section 3 | `ambre_prose::mark::tests::two_peers_spell_one_size_the_same_way`; `ambre_prose::mark::tests::a_textual_value_that_reads_as_a_number_is_left_alone`; `ambre_prose::logic::marks::tests::a_size_reaches_the_document_in_its_canonical_spelling` | none | verified |
| `PROSE-091` | `CONC` | Two peers writing the same mark value MUST produce the same bytes, so the merge holds one value rather than a choice between two spellings. | this spec section 3; this spec section 6 | `ambre_prose::logic::convergence_tests::two_peers_writing_one_size_merge_to_one_value` | none | verified |
| `PROSE-030` | `INV` | A block's paragraph style MUST be stored one key per property. | this spec section 3; architecture section 2.D | `ambre_prose::doc::tests::an_unset_property_is_the_absence_of_its_key` -- it reads one written key back and the eight others absent | none | verified |
| `PROSE-092` | `CONC` | Two concurrent edits to two different paragraph-style properties MUST both survive: neither MUST clobber the other. | this spec section 3; architecture section 2.D | `ambre_prose::logic::convergence_tests::two_peers_setting_two_paragraph_properties_lose_neither` | none | verified |
| `PROSE-093` | `INV` | `styles` MUST hold the user's own styles alone: the nine built-in names MUST NOT be stored. | this spec section 5 | structural -- `ambre_prose` opens `styles` and writes no entry into it: the nine names are the compiled list `ambre_prose::doc::BUILT_IN_STYLES`; `ambre_prose::doc::tests::the_nine_built_in_style_names_are_not_user_styles` | none | verified |
| `PROSE-031` | `INV` | An `annots` entry MUST declare its kind in one field of one type; a reader MUST NOT infer the kind from the presence of another field. | this spec section 3 | `ambre_prose::logic::notes::tests::an_entry_is_a_note_by_its_declared_kind_and_by_nothing_else` | none | verified |
| `PROSE-096` | `INV` | An unset property MUST be the absence of its key; no container MUST carry a "cleared" marker. | this spec section 3 | structural -- `ambre_prose::doc::write_paragraph_style` writes the properties the caller named and no other key, and `ambre_prose::doc::read_paragraph_style` reads an absent key as an unset property rather than a cleared one; `ambre_prose::doc::tests::an_unset_property_is_the_absence_of_its_key` | none | verified |
| `PROSE-032` | `INV` | `meta.page.orientation` MUST agree with `width_mm` and `height_mm`; a document whose orientation contradicts its two dimensions MUST NOT be written. | this spec section 3 | `ambre_prose::doc::tests::a_portrait_page_wider_than_it_is_tall_is_refused`; structural -- `ambre_prose::doc::write_page_format` is the one path that writes the three keys, and it writes them together | none | verified |
| `PROSE-027` | `INV` | A document MUST hold at least one root body block: an empty `blocks` tree is invalid. | this spec section 4 | `ambre_prose::logic::text::tests::a_deletion_that_empties_the_tree_keeps_one_block`; `ambre_prose::logic::tests::a_fresh_document_holds_one_paragraph_and_a_version` | none | verified |
| `PROSE-094` | `CONC` | Two peers concurrently emptying the tree MUST converge on one block: each grows one, and the merge MUST keep the grown block whose `TreeID` sorts first. | this spec section 4; this spec section 6 | `ambre_prose::logic::convergence_tests::two_peers_emptying_the_tree_converge_on_one_block` | none | verified |
| `PROSE-095` | `INV` | A converged document MUST hold only the parentages section 4 declares. | this spec section 4 | `ambre_prose::logic::convergence_tests::a_converged_document_holds_only_the_parentages_the_format_declares`; `ambre_prose::logic::blocks::tests::a_table_row_is_refused_at_the_root`; `::a_block_moved_under_its_own_descendant_is_refused`; `::an_insert_after_a_table_row_does_not_land_a_paragraph_under_the_table`; `::a_move_after_a_table_row_does_not_land_a_paragraph_under_the_table` | none | verified |
| `PROSE-028` | `INV` | A Prose document MUST carry `meta.schema_version`. | architecture section 2.D; this spec section 7 | `ambre_prose::schema::tests::a_new_document_carries_the_current_version`; `ambre_prose::logic::tests::a_fresh_document_holds_one_paragraph_and_a_version` | none | verified |
| `PROSE-097` | `INV` | A document whose `schema_version` exceeds the compiled `SCHEMA_VERSION` MUST be refused whole. | architecture section 2.D; this spec section 7 | `ambre_prose::schema::tests::a_document_a_newer_client_wrote_is_refused_whole`; `ambre_prose::logic::tests::a_document_a_newer_client_wrote_leaves_this_one_untouched` | none | verified |

* **Budget.** none: a format has no operation to budget. What opening, shaping and paginating a
  document costs is a row of `ambre-spec-budgets.md`, cited by the document that owns it.
* **Boundary.** none: the containers this table declares reach the relay as ciphertext alone
  (`ZK-001`), and an image node holds a content hash, never the pixels
  (`features/ambre-spec-prose-images.md`).
* **Verification.** 26 `verified`. The bump obligation splits in two. A key or a
  type moving is red against the frozen shape, which a CI gate holds append-only. A meaning moving
  under an unchanged key stays a human read (`SCHEMA-017`). The four convergence rows each run a
  two-peer test.
  * The two annotation rows are held by the footnote, the first entry a command writes.

---

## 2. IDENTITY

Four identities and one composite key. Prose mints one of the four: Loro mints the block's, the user
names the style's, and the blob's own bytes decide the image's.

* A block is identified by the `TreeID` Loro mints for the tree node (`PROSE-085`). It is stable
  through `move_to`, so an indent, a reorder and a move into a table cell all keep it
  (`PROSE-086`). The command plane carries the same pair under `BlockId`, so `ambre-bus` names no
  CRDT type (`features/ambre-spec-prose-engine.md section 3`).
* A user's deletion hides a block rather than removing it (`PROSE-118`). Loro would recreate a
  removed node under a new `TreeID` on undo, and the body and every reference would keep naming
  the old one. The hidden block and its subtree read as gone (`PROSE-119`). An undo drops the
  marker, and the block comes back with its identity and its body (`PROSE-120`).
* A hidden block is never removed while the session that hid it runs, since only that session's
  undo can restore it (`PROSE-121`). A compaction's snapshot triggers the pass. The client
  removes for good each hidden block whose session reads as ended, with its subtree and bodies. The
  removal is a delta of the client's session, sent after the snapshot, and never a step of its undo.
* A session runs when it is the client's own, or when the user the registry names for it has a
  session open here. Each session sends a presence frame once the document is ready, and answers
  each newcomer once, so a silent one is known. A user leaves on the relay's word alone
  (`PRESENCE-014`), which waits for the last of that user's sessions (`PRESENCE-015`).
* A block's body is a **root** Text container whose name joins the tag `t` and the tree id with `:`
  -- `"t:{tree_id}"`. A `TreeID` renders as two decimal numbers joined by `@`, so it cannot contain
  the separator (`PROSE-088`).
* An annotation -- a comment, a suggestion, a footnote -- is identified by a `Uuid` minted by the
  peer that creates it. The mark that anchors it carries that id as its String value
  (`PROSE-087`).
* A style is identified by its `style_id`, a name. The nine built-in names are not stored.
* An image is identified by the content hash of its plaintext blob, which is also its key into the
  media flow of `architecture section 2.D`.

The tree id and the body-key convention are decisions, with their rejected alternatives in
section 6. `crates/ambre-prose` holds the first two.

---

## 3. CONTAINERS

`State` is `built` when the container exists in `crates/ambre-prose/src/`, `design` when this schema
declares it and no code writes it yet. `LWW` names the unit at which two
concurrent writers race: the last writer takes that whole unit. A nested container carries its path;
the leaf is the name the code will use. `Owning feature` names the feature that states the rules of
the container, `--` where no feature does.

| Container | Loro type | Key | Value | LWW unit | Owning feature | State |
|---|---|---|---|---|---|---|
| `meta` | Map | `schema_version`, `title`, `created_at`, `updated_at`, `lang`, `pageless`, `different_first_page` | `u32`, String, `u64` and `u64` as Unix milliseconds, a BCP 47 tag, `bool`, `bool` | the key | -- (foundation; this spec section 7) | built |
| `meta/page` | Map | `orientation`, `width_mm`, `height_mm`, `margin_top_mm`, `margin_bottom_mm`, `margin_left_mm`, `margin_right_mm` | `"portrait"` or `"landscape"` (`PROSE-032`), and `f32` for the six | the key | pagination | built |
| `meta/defaults` | Map | `font_family`, `font_size_pt`, `line_height` | the picker's label, resolved through `registered_family_name`; `f32` points; `f32` multiple | the key | inline-styles | built |
| `meta/header`, `meta/footer` | Map | `enabled`, `block_id`, `align` | `bool`, String, String | the key | headers-footers | built |
| `meta/header_first`, `meta/footer_first` | Map | `block_id` | String | the key | headers-footers | built |
| `meta/line_numbers` | Map | `enabled`, `start`, `step`, `restart_each_page` | `bool`, `u32`, `u32`, `bool` | the key | -- (deferred; section 5) | design |
| `meta/version_history` | List | position | one `{name, created_at, snapshot_id}` map | list order, no LWW | -- (deferred; section 5) | design |
| `blocks` | MovableTree | a `TreeID`, minted by the tree | one block node map | tree order and parentage, no LWW | -- (structure; section 4) | built |
| `blocks/{id}` | Map | a field name, per the table below | per field | the field | per block type, below | built |
| `blocks/{id}/a` | Map | an attribute name, per block type | per attribute | the attribute | per block type, below | built |
| `t:{tree_id}` | Text | -- | one leaf block's body | none: char-CRDT | text-editing | built |
| `styles` | Map | `style_id` | one style map | the style entry | inline-styles | built |
| `styles/{id}` | Map | a paragraph-style field name, per the table below | per field | the field | inline-styles | design |
| `annots` | Map | a `Uuid` | one annotation map | the entry | comments, track-changes, footnotes | built |
| `annots/{id}` | Map | a field name, per the table below | per field | the field | comments, track-changes, footnotes | built |
| `annots/{id}/replies` | List | position | one reply map | list order, no LWW | comments | built |
| `annots/{id}/replies/{n}` | Map | `id`, `author`, `user_id`, `created_at` | String, String, String, `i64` | the key | comments | built |
| `annots/{id}/replies/{n}/text` | Text | -- | a reply's body | none: char-CRDT | comments | built |
| `bibliography` | Map | `source_id` | one map of CSL fields | the source entry | -- (deferred; section 5) | design |

**The block node map, field by field.** `Kinds` names the block types that carry the field. The nine
paragraph-style fields sit flat in the node map, one key each (`PROSE-030`), beside the three the
block always carries.

| Field | Type | Holds | Kinds | State |
|---|---|---|---|---|
| `t` | `u8` | the block type discriminant, per the table below | every | built |
| `s` | String | the `style_id`: a key in `styles`, or one of the nine built-in names -- `body`, `heading-1` to `heading-6`, `code`, `quote`; absent means the type's own built-in | every | built |
| `a` | Map | the type-specific attributes; absent means the type has none | the types below | built |
| `hb` | `bool` | that the block holds a header or a footer, named by `meta`; kept out of the body flow (`PROSE-006`) | Paragraph, at the root | built |
| `nb` | `bool` | that the block holds a footnote's text, named by the note's entry; kept out of the body flow (`PROSE-FEAT-305`) | Paragraph, at the root | built |
| `gn` | String | that a user deleted the block: the Loro peer id of the session that did, in decimal; absent on every live block (`PROSE-118`, `PROSE-122`) | every | built |
| `rg` | `bool` | that a peer grew this block for want of any root block, and not the author; absent on every block the author asked for (`PROSE-094`) | Paragraph, at the root | built |
| `align` | `u8` | left, center, right or justify | every text block | built |
| `indent_first`, `indent_left`, `indent_right` | `f32` | the three indents, in points | every text block | built |
| `space_before`, `space_after` | `f32` | the vertical margins, in points, before collapse | every text block | built |
| `line_height` | Map | the mode (multiple, exact, at least) and its value | every text block | built |
| `keep_together`, `keep_with_next` | `bool` | the two pagination constraints | every text block | built |

**The block types, and the attributes each one carries.** `t` is the discriminant; a type with no
attribute carries no `a` map.

| Block type | `t` | `a` keys |
|---|---|---|
| Paragraph | 0 | -- |
| Heading | 1 | `level` (`u8`, `1..=6`) |
| ListItem | 2 | `ls` (`u8`: 0 bullet, 1 ordered, 2 todo), `lo` (`u8` when `ls` is 1: 0 decimal, 1 lower alpha, 2 upper alpha, 3 lower roman, 4 upper roman), `tc` (`bool` when `ls` is 2) |
| Table | 3 | `col_widths` (a list of `f32`, one per column), `merged` (a map keyed on the anchor `TreeID`, each entry a map of `e`, the extent `TreeID`, and `n`, the Lamport stamp of the merge) |
| TableRow | 4 | -- |
| TableCell | 5 | `bg` (`"#rrggbb"`), `bt`, `bb`, `bl`, `br` (`f32`, the four border widths), `bc` (`"#rrggbb"`, the border colour), `pad` (`f32`), `va` (`u8`: 0 top, 1 middle, 2 bottom) |
| CodeBlock | 6 | `lang` (String) |
| Image | 7 | `content_hash` (String), `alt` (String), `w`, `h` (`u32`, the display size, never the native one), `align` (`u8`: 0 block, 1 left, 2 right, 3 centre), `caption` (String) |
| PageBreak | 8 | -- |
| HRule | 9 | -- |
| Toc | 10 | `max_level` (`u8`, `1..=6`), `numbered` (`bool`) |

**The inline mark registry.** A mark's key, the grammar of its String value, and the expansion an
adjacent insertion gets. `ExpandType` is read from the key alone, and the last row is why the
function is total (`PROSE-007`).

| Mark key | Value | `ExpandType` | Owning feature |
|---|---|---|---|
| `b`, `i`, `u`, `s`, `sup`, `sub`, `c` | none: the key's presence is the value | `After` | inline-styles |
| `a` | the URL | `None` | inline-styles |
| `fg`, `bg` | `"#rrggbb"`, or `"#rrggbbaa"` when the alpha is not opaque | `After` | inline-styles |
| `fs` | the size in points: base 10, `.` separator, no exponent, no trailing zero, no unit -- `"11"` and `"11.5"`, never `"11.0pt"` (`PROSE-008`) | `After` | inline-styles |
| `ff` | the family name, as the picker labels it | `After` | inline-styles |
| `cm:{id}` | the thread's `Uuid` into `annots`, one key per thread so two threads over one character never share a value | `None` | comments |
| `si:{id}`, `sd:{id}` | the suggestion's `Uuid` into `annots`, a suggested insertion and a suggested deletion, one key per suggestion so a span can carry both | `None` | track-changes |
| `fn` | a `Uuid` into `annots`, laid on one U+FFFC the insert writes | `None` | footnotes |
| `ref` | the hex of a postcard record: the target `TreeID` and the reference kind, laid on one U+FFFC the insert writes | `None` | cross-references |
| `f:pn`, `f:pc`, `f:dt`, `f:ti` | the key is the field -- the page number, the page count, `meta.updated_at`, `meta.title`; `f:dt` holds its format, `short`, `long` or `iso` | `None` | headers-footers |
| any other key | -- | `After` | -- |

**The annotation entry, field by field.** `Kinds` names the annotation kinds that carry the field.

| Field | Type | Holds | Kinds | State |
|---|---|---|---|---|
| `kind` | String | the kind, which every entry names: `"footnote"`, `"comment"` or `"suggestion"` today. No kind is read from an absent field (`PROSE-031`) | every kind | built |
| `anchor_block` | String | the `TreeID` of the annotated block, a cache of where the live mark sits | comment | built |
| `anchor_start`, `anchor_end` | `i64` | the cached span, in the block's own offsets | comment | built |
| `resolved` | `bool` | whether the thread is closed | comment | built |
| `author` | String | the self-asserted display name, stamped by the local session | suggestion | built |
| `user_id` | String | the canonical `UserId` of the author, from the session identity. Ownership is read from it | suggestion | built |
| `replies` | List | the thread, the opening comment first. A comment's author is the one its opening reply names | comment | built |
| `block_id` | String | the `TreeID` of the block holding the note's text (`PROSE-FEAT-305`) | footnote | built |
| `name`, `block_id`, `range` | -- | the deferred bookmark fields | bookmark | design |

**Absence is a value** (`PROSE-096`). No `s` key means the block type's built-in style. No `a` map
means the type carries no attribute. No mark on a range means the cascade decides. An empty
`annots` map means no comment, no suggestion and no footnote. No container carries a "cleared" marker.

---

## 4. HIERARCHY

* `LoroDoc` root
  * `meta`
    * `page`, `defaults`, `header`, `header_first`, `footer`, `footer_first`, `line_numbers`,
      `version_history`
  * `blocks`
    * per block, keyed by the `TreeID`: the type, the style id, the nine paragraph-style keys, and
      `a` where the type carries attributes
  * one `t:{tree_id}` per leaf block, at the root
  * `styles`
    * per custom style, keyed by `style_id`: the paragraph-style keys it overrides
  * `annots`
    * per annotation, keyed by `Uuid`: the kind, the anchor cache, the authorship, and the
      `block_id` of a footnote's text
      * per reply: the author, the stamp, and `text`
  * `bibliography`

**What may nest under what.** The tree is not free. A reader of a converged document meets only
these parentages, and the feature that inserts a block is what refuses the rest (section 8).

| Parent | Allowed children |
|---|---|
| the root | Paragraph, Heading, ListItem, Table, CodeBlock, Image, PageBreak, HRule, Toc |
| Table | TableRow |
| TableRow | TableCell |
| TableCell | Paragraph, Heading, ListItem |
| ListItem | ListItem, and Paragraph as the degenerate case |
| every other type | -- : a leaf |

The root body tree is never empty (`PROSE-027`), and only these parentages survive a merge
(`PROSE-095`). A `TableCell` or a `ListItem` emptying to zero
children follows its own type's rule, which is the tables and lists features'.

---

## 5. CONTAINER NOTES

Only where a schema decision needs a reason. 40 words each.

* **`meta`.** `schema_version` is declared here and by nothing else in this application: no key, no
  command and no rule required it before `PROSE-028`. `pageless` and `line_numbers` are deferred
  designs whose register is named so the version rule already covers them.
* **`blocks`.** One tree holds paragraphs, list items, table rows and cells alike. One lifecycle
  serves every block, and the nesting table is the only structural rule. Depth is read from the
  ancestor chain, never stored.
* **`blocks/{id}.rg`.** Loro refuses to raise a tombstone, so a peer meeting an empty tree grows a
  paragraph. The marker tells a grown paragraph from an authored one, and every peer drops the
  surplus alike (`PROSE-094`).
* **`t:{tree_id}`.** A root container per leaf block, addressed by name rather than nested in the
  node map. Loro's text is opened from the document, not from a map entry. Deleting a block leaves
  its text container behind, and nothing reclaims it.
* **`styles`.** Only the user's custom styles (`PROSE-093`). The nine built-ins, `quote` among
  them, are compiled in. A document written by an older client renders them from the client
  rather than from stale copies of its own.
* **`annots`.** One map for comments, suggestions and footnotes, because all three are reached the
  same way: a mark carrying an id. The cost is the discriminant of `PROSE-031`.
* **`annots/{id}/replies`.** A list, so order is the list's and no reply carries an index two peers
  could race for. `created_at` is display only.
* **`bibliography`, `meta/version_history`, `meta/line_numbers`.** Declared and unwritten: three
  deferred designs whose containers are named here so a later one cannot mint a rival shape.

---

## 6. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| Body text in a `LoroText` per leaf block | a `LoroMap` key | LWW on the whole body, so one of two authors typing in the same paragraph loses their sentence, the defect `ambre-spec-toile-schema.md section 1` records as `TOILE-030`. Fugue converges two insertions at the same position without interleaving, and keeps a cursor stable across a remote change, which is the whole of collaborative editing (`PROSE-002`) |
| Body text in a `LoroText` per leaf block | a `LoroList` of characters | it is convergent, but marks are native to the text container and would have to be rebuilt on top. Fugue converges two insertions at the same position without interleaving, and keeps a cursor stable across a remote change, which is the whole of collaborative editing (`PROSE-002`) |
| Loro's own `TreeID` as the block identity | a minted `"{peer}_{counter}"` string, as Trame and Toile carry | it is a second identity to keep in step with the one the tree mints anyway, and two ids to migrate the day either shape changes. The tree already mints an id that survives `move_to`, so a move, an indent and a regroup keep every reference: a comment anchor, a cross reference target, a header block id |
| The block hierarchy in a `LoroMovableTree` | a flat `LoroList` of blocks | a move is a delete plus an insert, which mints a new identity and breaks every reference to the block. A move as one operation keeps the node's id, so nesting a list item costs no recreation and orphans no reference |
| A block's body keyed `"t:{tree_id}"` at the root | nesting the text inside the node map | it is one more container per block on the merge path, for an address the code never needs. The body is opened from the document by name, which is how Loro addresses a text container, and the tag keeps the block namespace out of every other root's |
| The block that re-grows an emptied tree is grown per peer and marked | raising the tombstoned block whose `TreeID` sorts first | Loro refuses it: a deleted node is moved under the deleted root and `mov` will not take it back out, so no peer can raise what the deletion tombstoned. Each peer grows a paragraph instead, marks it, and every peer then drops all but the marked paragraph whose `TreeID` sorts first. The choice reads the document alone, so two peers reach one tree (`PROSE-094`) |
| Depth derived from the ancestor chain, never stored | a `depth` field on the list item | it is correct until two peers indent the same item, and silently wrong afterwards. A stored depth is a second truth that a concurrent move puts out of step with the tree, and the tree is the one both peers converge on (`PROSE-005`) |
| Inline styles as marks on the text | a `LoroMovableTree` of styled spans | overlapping ranges are not a tree, so bold across a link would need a node split the document never asked for. A mark spans a range and may overlap another, which is what a formatted document is |
| Every mark value a String, in a canonical form | a typed value per mark key | it is a second grammar per key, and `11` against `11.0` race as two distinct values. One value type across every key keeps the mark API single, and a canonical rendering makes two peers writing the same size produce the same bytes rather than two values LWW must choose between (`PROSE-008`) |
| A paragraph style stored one key per property | one binary register per block | it is the cross-app pattern of `architecture section 2.D` -- correct for a fill racing a stroke, and here it turns two unrelated edits into one loss. The nine properties are independent scalars with no atomicity between them, so per-key LWW lets two peers change the alignment and the spacing at once and lose neither (`PROSE-030`); a whole-blob register would also have no field to fall back to but the type default |
| One `annots` map for every annotation kind | one container per kind | it is three roots to seed, and a mark key that must agree with the root it points into. A comment, a suggestion and a footnote are all reached by a mark carrying a `Uuid`, so one map is one lookup and the marks are what separate the kinds |
| No media register: the image node carries the hash | Toile's `media` map | it stores the native size and an inline thumbnail, and a Prose page paints the blob or a placeholder, so the thumbnail has no surface and the native size no reader. A Prose image has one display size and one alt text, both the block's own, and the blob is content-addressed out of band |
| The header and the footer as ordinary blocks, addressed by a `TreeID` in `meta` | a nested content command carrying the header's own edits | it is a second editing surface, and a command that must be recursive to carry a mark. Every text command already targets a block id, so a header is edited by the machinery the body uses and inherits its marks and its cascade (`PROSE-006`) |
| The list style as a discriminant, never a flag | `ordered` as a boolean | it can encode neither a todo item nor lower-roman numbering, so a second key would follow within one release. Four ordered renderings and a checkbox are five values, and the ordinal a reader sees comes from that one key |
| No quote block type | `BlockType::Quote` | it is a twelfth discriminant carrying no structure of its own. A quote is an indent and a sidebar, which is a paragraph style; adding a type would fork every rule that walks paragraphs |

---

## 7. FORMAT DUTIES

| Duty | Rule | Where |
|---|---|---|
| Version | root `meta`, key `schema_version` (`u32`); absent means 0 | architecture section 2.D; `PROSE-028` |
| Read | equal proceeds, older migrates, newer is refused whole (`SchemaTooNew`) | architecture section 2.D |
| Migration | the chain lives in `ambre-prose`; the foundation owns the protocol, never the semantics | architecture section 2.D |
| Bump | a key, a value type or the meaning of a value changes: `SCHEMA_VERSION` rises and one named fixture is frozen in the same commit | `SCHEMA-018`; `ambre-spec-schema-version.md` `SCHEMA-017`; `ambre-spec-fixtures.md section 2` |
| Unknown field | a map key no build knows is ignored on read, so a forward-written key is dropped rather than refused; the `ref` mark is the exception, a postcard record that tolerates none, and `PROSE-028` is what stops a stale build reading it | architecture section 2.D |
| Size, ids | a body key is the tag, a separator and a `TreeID`; a document of 300 blocks carries 300 root containers keyed that way | this spec section 2 |
| Size, blobs | an image node holds a hash and a display size; the document never holds pixels | `features/ambre-spec-prose-images.md` |
| Deltas | a remote delta carries no version and applies blind; mixed-version live editing is bounded, not solved | architecture section 2.D |
| Container, file | the `.ambre` envelope has a version of its own, and the two are read the same way on purpose | `ambre-spec-docfile.md section 3`; `features/ambre-spec-prose-import-export.md section 3` |

---

## 8. WHAT THIS DOCUMENT DOES NOT OWN

| Question | Owner |
|---|---|
| What refuses a Table inside a TableCell, and what a rejected insert reports | `features/ambre-spec-prose-tables.md`; the nesting table of section 4 is the shape, not the guard |
| How a list item's depth and its ordinal are computed | the lists feature; `PROSE-005` |
| Which page a block lands on, and what may not be split | `features/ambre-spec-prose-pagination.md`: `PROSE-010` and the constraint order |
| How four cascade levels resolve into one style | the inline-styles feature; `PROSE-FEAT-020` |
| That the mark, not the cached anchor, says where a comment sits | `features/ambre-spec-prose-comments.md`; `PROSE-FEAT-001` |
| What an orphaned comment, suggestion or footnote renders | the comments, track-changes and footnotes features |
| How a content hash becomes pixels, and what a failed download shows | `features/ambre-spec-prose-images.md` |
| That a suggestion accepts and rejects as one unit, whatever its span count | the track-changes feature |
| That undo reverts local operations only | the shared undo model; `ambre-spec-undo.md`, `UNDO-001` |
| That the `LoroDoc` leaves the Logic thread as bytes alone | the engine; `PROSE-001`, `PROSE-003` |
| Who may write, and what the server enforces | architecture section 2.D; `ambre-spec-server.md` |
