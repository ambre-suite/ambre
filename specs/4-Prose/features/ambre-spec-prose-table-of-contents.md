# AMBRE -- FEATURE: TABLE OF CONTENTS

**Type:** Vertical (feature)
**Implementation:** implemented

> A list of the document's headings, with the page each one starts on. Nothing about it is written
> down. It is read off the headings and the page layout every time the document is laid out. It
> is never out of date, and two authors never merge two versions of it.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-FEAT-003` | `OBS` | The table of contents MUST list the document's headings, in document order, each reading the heading's own text. | this spec section 3 | `ambre_prose::logic::derived::tests::the_entries_are_the_headings_in_order_reading_their_own_text` | none | verified |
| `PROSE-FEAT-060` | `OBS` | An entry's page number MUST be the page its heading starts on in the layout being drawn. | this spec section 3 | `ambre_prose::logic::derived::tests::an_entry_reads_the_page_its_heading_starts_on` | none | verified |
| `PROSE-FEAT-113` | `INV` | The document MUST hold no entry: the block carries the deepest level it shows and whether it numbers them, and nothing else. | this spec section 3; this spec section 5 | `ambre_prose::logic::derived::tests::the_block_holds_its_two_settings_and_no_entry` | none | verified |
| `PROSE-FEAT-061` | `OBS` | A heading deeper than the configured level MUST NOT appear in the table of contents. | this spec section 3 | `ambre_prose::logic::derived::tests::a_heading_deeper_than_the_depth_is_left_out` | none | verified |
| `PROSE-FEAT-062` | `OBS` | A heading number MUST appear in front of an entry only while the table of contents is configured as numbered. | this spec section 3 | `ambre_prose::logic::derived::tests::a_heading_number_shows_only_while_numbered` | none | verified |
| `PROSE-FEAT-114` | `CONC` | Two peers whose settings have converged MUST list the same entries, at the same depth, numbered the same way. | this spec section 2; this spec section 5 | `ambre_prose::logic::derived::tests::two_converged_peers_list_the_same_entries` | none | verified |

### 1.2 Insert a table of contents

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-FEAT-115` | `POST` | After the command, a table of contents MUST sit at the place in the tree the command named, showing the depth and the numbering it carried. | this spec section 3 | `ambre_prose::logic::derived::tests::an_insert_lands_where_it_was_named_carrying_its_settings` | none | verified |
| `PROSE-FEAT-116` | `FRAME` | Inserting one MUST NOT touch a heading, and MUST NOT write an entry anywhere. | derived | `ambre_prose::logic::derived::tests::inserting_one_touches_no_heading` | none | verified |

### 1.3 Change what it shows

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-FEAT-117` | `POST` | After the command, the block MUST carry the depth and the numbering asked for. | this spec section 3 | `ambre_prose::logic::derived::tests::a_settings_change_lands_on_the_block` | none | verified |
| `PROSE-FEAT-291` | `POST` | The next layout MUST list the block's entries by the depth and the numbering it carries. | this spec section 3 | `ambre_prose::logic::derived::tests::the_next_layout_lists_by_the_settings_just_written` | none | verified |
| `PROSE-FEAT-292` | `POST` | A depth MUST be one to six when it is written, and a stored depth outside that range MUST be read as the nearer of the two. | this spec section 3 | `ambre_prose::logic::derived::tests::a_stored_depth_outside_one_to_six_reads_as_the_nearer_end` -- it stores a zero and a two hundred and reads the settings back; type-level -- `InsertToc` and `UpdateTocConfig` carry a `HeadingLevel`, so no command writes another depth | none | verified |
| `PROSE-FEAT-118` | `FRAME` | Changing one block's settings MUST NOT touch a heading, nor the settings of another table of contents. | this spec section 5 | `ambre_prose::logic::derived::tests::changing_one_block_leaves_another_and_the_headings_alone` | none | verified |

### 1.4 Click an entry

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-UI-082` | `POST` | A click on an entry MUST bring the heading it reads into view. | this spec section 4 | `ambre_prose::view::tests::following_an_entry_brings_its_heading_to_the_top_and_sends_nothing` | none | verified |
| `PROSE-UI-083` | `FRAME` | Following an entry MUST NOT write to the document: where this reader looks is not a change a peer receives. | derived | `ambre_prose::view::tests::following_an_entry_brings_its_heading_to_the_top_and_sends_nothing` | none | verified |

* **Budget.** the table-of-contents build over fifty headings, one row of
  `ambre-spec-budgets.md section 1`. What the repeated build costs when the entries move the
  page count is the deferred pass of `features/ambre-spec-prose-pagination.md`, `PROSE-019`.
* **Boundary.** none of its own: the block carries two settings and no text, and `ZK-001` seals
  every payload before it leaves the client.
* **Verification.** 14 `verified`, nothing else: the build, the settings and the page numbers are
  host tests in `crates/ambre-prose`, over a layout the host measures. `PROSE-UI-082` and `-083` are
  host tests of the view.

---

## 2. WHAT EACH USER SEES

Two peers over one outline. The last column names the test that plays the row, under
`ambre_prose::logic::derived::tests`.

| Alice does | Bob does | The document holds | Each user sees | Test |
|---|---|---|---|---|
| adds a heading in the middle | -- | one more heading, no entry anywhere | one more line in the table of contents, on both screens | `the_entries_are_the_headings_in_order_reading_their_own_text` |
| edits a heading's text | -- | the new text, in the heading alone | the entry reading the new text, on both screens | `the_entries_are_the_headings_in_order_reading_their_own_text` |
| types a page of text before a heading | -- | the text | the entry's page number one higher, on both screens | `an_entry_reads_the_page_its_heading_starts_on` |
| lowers the configured level | raises it at the same moment | one setting, the later of the two | the same depth of entries on both screens | `two_converged_peers_list_the_same_entries` |
| turns numbering on | -- | the setting | a heading number in front of every entry, on both screens | `a_heading_number_shows_only_while_numbered` |
| deletes the last heading of a level | -- | one heading fewer | that entry gone, and every following number shifted | `the_entries_are_the_headings_in_order_reading_their_own_text` |

---

## 3. MODEL

**Containers.** None of its own. The table of contents is one block type of the movable tree. Its
two settings -- the deepest level it shows, and whether it numbers its entries -- are that
node's attributes, declared in `ambre-spec-prose-schema.md section 3`. The entries are not stored,
so no container holds them.

**What it is built from.** The block tree, in document order, and the page layout:

```rust
/// One line of the table of contents. Nothing here is persisted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TocEntry {
    pub block_id:       BlockId,
    pub level:          HeadingLevel,
    pub text:           String,
    /// Present only when the block is configured as numbered (`PROSE-FEAT-062`).
    pub heading_number: Option<HeadingNumber>,
    /// The page number the heading starts on, read from the layout (`PROSE-FEAT-060`).
    /// `None` while the layout places no block that holds the heading.
    pub page:           Option<Ordinal<Page>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TableOfContents {
    pub entries:   Vec<TocEntry>,
    pub max_level: HeadingLevel,
    pub numbered:  TocNumbering,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum TocNumbering { #[default] Plain, Numbered }
```

**The build, in order:**

1. walk the block tree in document order.
2. keep each heading no deeper than the configured level, and skip the rest (`PROSE-FEAT-061`). A
   heading inside a table cell is kept, and takes the page its table starts on.
3. read each kept heading's text from its own body, and its start page from the layout
   (`PROSE-FEAT-060`).
4. when the block is numbered, prefix the heading number the headings feature derives
   (`features/ambre-spec-prose-headings.md`, `PROSE-FEAT-030`); otherwise leave it out
   (`PROSE-FEAT-062`).

The build runs on each layout, and nothing it produces is written back (`PROSE-FEAT-003`). The
engine answers `TocUpdated` for each block, and the view shows the last one it received.

**The depth is one to six.** A heading level is one to six (`features/ambre-spec-prose-headings.md`),
so a command carries a `HeadingLevel`. A stored depth of zero or two hundred reads as the nearer
bound (`PROSE-FEAT-292`).

**Why the build repeats.** The table of contents is itself a block on a page. Its own height
changes where the following blocks fall, which changes the page numbers it shows. Pagination runs
the settling as a bounded pass, never inline on the keystroke path: the rule is `PROSE-019`, and
the algorithm is `features/ambre-spec-prose-pagination.md section 2`. The engine runs it after
`InsertToc` and `UpdateTocConfig`; a run after the author stops typing lands with the painter.

**The commands** (`ambre_prose::message`, declared in `features/ambre-spec-prose-engine.md section 3`):

| Shape | Behaviour |
|---|---|
| `InsertToc { parent_id, after_id, max_level, numbered }` | the block, at a place in the tree |
| `UpdateTocConfig { block_id, max_level, numbered }` | rewrites the two settings |

There is no command that edits an entry, because there is no entry to edit: an entry is changed by
changing the heading it reads.

**Code:** `crates/ambre-prose/src/toc.rs` holds the build; `logic/derived.rs` holds the commands;
`layout/reflow.rs` holds the settling; `view/mod.rs` follows an entry.

---

## 4. SURFACE

| Trigger | Behaviour |
|---|---|
| the slash command, or the insert menu | a table of contents at the caret |
| the level control of its own menu | shows fewer or more levels |
| the numbering switch | shows or hides the heading numbers |
| a click on an entry | moves the view to that heading |

An entry is a link when the document is exported to PDF, and the link resolves to the heading
rather than to a page (`features/ambre-spec-prose-pdf-export.md section 3`).

---

## 5. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| The entries are derived, never stored | a container of entries kept in step with the headings | it is guaranteed to drift and has no correct merge: a stored entry is a second copy of a heading, so two authors editing a heading and its entry at once merge two texts that were meant to be one. Derived, the entries cannot disagree with the headings, because they are the headings (`PROSE-FEAT-003`) |
| The page number comes from the layout | storing the page on the entry | it needs a rewrite of the whole table on any edit and races between two peers laying out differently. A page number is a fact about a layout, and a layout changes on every edit above the heading and on every change to the page format |
| The settling pass is deferred and bounded | iterating to a fixed point inline | a document that never settles turns it into a freeze while the author types. The entries carry page numbers and adding them changes the page count that produced them, so a bounded pass off the keystroke path makes a pathological document cheap rather than fatal, and it is one rule with pagination rather than two (`PROSE-019`) |
| Numbering is a setting of the block, not of the headings | numbering stored on each heading | it makes the second table of contents impossible and turns a display switch into a document-wide rewrite. Two tables in one document may want different depths and different numbering over headings both share |
| The depth is a setting, not a filter the reader applies | a per-reader depth | it makes two peers paginate the same document differently and puts every page number in doubt. The depth changes the height of the block, so it changes pagination and has to be part of the document both peers lay out |

---

## 6. NOT BUILT, AND NOT YET RULED

* **Nothing rules on more than one table of contents.** The block may be inserted anywhere, any
  number of times, and no rule says whether a second one is legal or what it shows.
  `PROSE-FEAT-118` says only that one does not rewrite another's settings.
* **Nothing rules on the entries a table of contents lists of itself.** The block is not a heading,
  so it lists nothing of its own.
* **The entry has no style.** Nothing says how a level-three entry differs from a level-one one.
  The leader dots, the indent and the type size are undecided. They will be a decision of the page
  rendering rather than of this feature.
* **Two peers may lay out the same document differently**, and every page number here rests on
  their agreeing. The gap is `features/ambre-spec-prose-pagination.md section 5`, and it is the
  same one the cross references and the comments rest on. It lands with the `Two peers read the
  same page number` Story (`ambre-roadmap.md` E5).
* **Three more derived blocks are designed and unbuilt, and each is this walk with another
  predicate.** A table of figures and a table of tables list the captioned pictures and tables. An
  index lists the terms an author marked, alphabetically, with the pages they fall on. All three
  reuse the settling pass and the page numbers of section 3, and none has a block type in
  `ambre-spec-prose-schema.md section 3`. The index needs a mark key that is not declared
  either. The Story is "The later writing set" (`ambre-roadmap.md` POST-V1).
