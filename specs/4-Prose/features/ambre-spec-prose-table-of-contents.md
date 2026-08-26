# AMBRE -- FEATURE: TABLE OF CONTENTS

**Type:** Vertical (feature)
**Implementation:** design-only

> A list of the document's headings, with the page each one starts on. Nothing about it is written
> down: it is read off the headings and the page layout every time the document is laid out, so it
> is never out of date and two authors never merge two versions of it.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`.

| ID | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|
| `PROSE-FEAT-003` | The table of contents MUST be derived from the headings; no copy of them MUST be stored. | this spec section 3 | not built -- lands with the "Tables, table of contents and word count" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-060` | An entry's page number MUST be read from the current page layout; it MUST NOT be stored in the document. | this spec section 3 | not built -- lands with the "Tables, table of contents and word count" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-061` | A heading deeper than the configured level MUST NOT appear in the table of contents. | this spec section 3 | not built -- lands with the "Tables, table of contents and word count" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-062` | A section number MUST appear in front of an entry only while the table of contents is configured as numbered. | this spec section 3 | not built -- lands with the "Tables, table of contents and word count" Story (`ambre-roadmap.md` E5) | unimplemented |

* **Budget.** the table-of-contents build over fifty headings, one row of
  `ambre-spec-budgets.md section 1`. What the repeated build costs when the entries move the
  page count is the deferred pass of `features/ambre-spec-prose-pagination.md`, `PROSE-019`.
* **Boundary.** none of its own: the block carries two settings and no text, and `ZK-001` seals
  every payload before it leaves the client.
* **Verification.** 4 `unimplemented`, nothing else: `crates/ambre-prose` is a `lib.rs`.
  `PROSE-FEAT-003` and `PROSE-FEAT-060` are the two a reviewer can hold without a test, because a
  stored heading and a stored page number would each be a key in the schema table and the schema
  table has neither.

---

## 2. WHAT EACH USER SEES

Two peers over one outline. No test resolves yet: the last column names the row each row rests on.

| Alice does | Bob does | The document holds | Each user sees | Test |
|---|---|---|---|---|
| adds a heading in the middle | -- | one more heading, no entry anywhere | one more line in the table of contents, on both screens | none -- `PROSE-FEAT-003` |
| edits a heading's text | -- | the new text, in the heading alone | the entry reading the new text, on both screens | none -- `PROSE-FEAT-003` |
| types a page of text before a heading | -- | the text | the entry's page number one higher, on both screens | none -- `PROSE-FEAT-060` |
| lowers the configured level | raises it at the same moment | one setting, the later of the two | the same depth of entries on both screens | none -- `PROSE-FEAT-061` |
| turns numbering on | -- | the setting | a section number in front of every entry, on both screens | none -- `PROSE-FEAT-062` |
| deletes the last heading of a level | -- | one heading fewer | that entry gone, and every following number shifted | none -- `PROSE-FEAT-003` |

---

## 3. MODEL

**Containers.** None of its own. The table of contents is one block type of the movable tree, and
its two settings -- the deepest level it shows, and whether it numbers its entries -- are that
node's attributes, declared in `ambre-spec-prose-schema.md section 3`. The entries are not stored,
so no container holds them.

**What it is built from.** The block tree, in document order, and the page layout:

```rust
/// One line of the table of contents. Nothing here is persisted.
pub struct TocEntry {
    pub block_id:       TreeID,
    pub level:          u8,
    pub text:           String,
    /// Present only when the block is configured as numbered (`PROSE-FEAT-062`).
    pub section_number: Option<Vec<u32>>,
    /// The page the heading starts on, read from the layout (`PROSE-FEAT-060`).
    pub page:           u32,
}

pub struct TableOfContents {
    pub entries:   Vec<TocEntry>,
    pub max_level: u8,
}
```

**The build, in order:**

1. walk the block tree in document order.
2. keep each heading no deeper than the configured level, and skip the rest (`PROSE-FEAT-061`).
3. read each kept heading's text from its own body, and its start page from the layout
   (`PROSE-FEAT-060`).
4. when the block is numbered, prefix the section number the headings feature derives
   (`features/ambre-spec-prose-headings.md`, `PROSE-FEAT-030`); otherwise leave it out
   (`PROSE-FEAT-062`).

The build runs when the layout is published, and nothing it produces is written back
(`PROSE-FEAT-003`).

**Why the build repeats.** The table of contents is itself a block on a page, so its own height
changes where the following blocks fall, which changes the page numbers it shows. Pagination runs
the settling as a bounded, deferred pass, never inline on the keystroke path: the rule is
`PROSE-019`, and the algorithm is `features/ambre-spec-prose-pagination.md section 3`.

**The commands** (`ambre-bus`, declared in `features/ambre-spec-prose-engine.md section 3`):

| Shape | Behaviour |
|---|---|
| `InsertToc { parent_id, after_id, max_level, numbered }` | the block, at a place in the tree |
| `UpdateTocConfig { block_id, max_level, numbered }` | rewrites the two settings |

There is no command that edits an entry, because there is no entry to edit: an entry is changed by
changing the heading it reads.

**Code:** none. `crates/ambre-prose/src/lib.rs` is seven lines.

---

## 4. SURFACE

| Trigger | Behaviour |
|---|---|
| the slash command, or the insert menu | a table of contents at the caret |
| the level control of its own menu | shows fewer or more levels |
| the numbering switch | shows or hides the section numbers |
| a click on an entry | moves the view to that heading |

An entry is a link when the document is exported to PDF, and the link resolves to the heading
rather than to a page (`features/ambre-spec-prose-pdf-export.md section 3`).

---

## 5. DECISIONS

| Decision | Rationale | Rejected alternative |
|---|---|---|
| The entries are derived, never stored | a stored entry is a second copy of a heading, and two authors editing a heading and its entry at once merge two texts that were meant to be one. Deriving them cannot disagree with the headings, because it is the headings (`PROSE-FEAT-003`) | a container of entries kept in step with the headings, which is guaranteed to drift and has no correct merge |
| The page number comes from the layout | a page number is a fact about a layout, and a layout changes on every edit above the heading and on every change to the page format. Stored, it would be wrong one keystroke after it was written | storing the page on the entry, which needs a rewrite of the whole table on any edit and races between two peers laying out differently |
| The settling pass is deferred and bounded | the entries carry page numbers, and adding them changes the page count that produced them. A bounded pass off the keystroke path makes a pathological document cheap rather than fatal, and it is one rule with pagination rather than two (`PROSE-019`) | iterating to a fixed point inline, which a document that never settles turns into a freeze while the author types |
| Numbering is a setting of the block, not of the headings | two tables of contents in one document may want different depths and different numbering, and the headings are shared by both. Keeping it on the block keeps the headings free of a decision that belongs to the view of them | numbering stored on each heading, which makes the second table of contents impossible and turns a display switch into a document-wide rewrite |
| The depth is a setting, not a filter the reader applies | the depth changes the height of the block, so it changes pagination: it has to be part of the document both peers lay out, not a per-reader preference | a per-reader depth, which makes two peers paginate the same document differently and puts every page number in doubt |

---

## 6. NOT BUILT, AND NOT YET RULED

* **Nothing rules on more than one table of contents.** The block may be inserted anywhere, any
  number of times, and no rule says whether a second one is legal or what it shows.
* **Nothing rules on the entries a table of contents lists of itself.** The block is not a heading,
  so it lists nothing of its own; whether a heading *inside* a table cell is an entry is undecided,
  and the walk of section 3 would keep it.
* **The entry has no style.** Nothing says how a level-three entry differs from a level-one one, so
  the leader dots, the indent and the type size are undecided and will be a decision of the page
  rendering rather than of this feature.
* **What the reader sees while the settling pass runs is not ruled on.** Pagination states the pass
  is deferred and bounded; nothing says whether the entries show the previous page numbers or
  nothing at all in the meantime.
* **Two peers may lay out the same document differently**, and every page number here rests on
  their agreeing. The gap is `features/ambre-spec-prose-pagination.md section 5`, and it is the
  same one the cross references and the comments rest on.
* **Three more derived blocks are designed and unbuilt, and each is this walk with another
  predicate.** A table of figures and a table of tables list the captioned pictures and tables; an
  index lists the terms an author marked, alphabetically, with the pages they fall on. All three
  reuse the settling pass and the page numbers of section 3, none has a block type in
  `ambre-spec-prose-schema.md section 3`, and the index needs a mark key that is not declared
  either. The Story is "The later writing set" (`ambre-roadmap.md`).
