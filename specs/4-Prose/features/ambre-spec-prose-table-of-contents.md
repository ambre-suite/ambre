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

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|---|
| `PROSE-FEAT-003` | `OBS` | The table of contents MUST list the document's headings, in document order, each reading the heading's own text. | this spec section 3 | not built -- lands with the "Tables, table of contents and word count" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-060` | `OBS` | An entry's page number MUST be the page its heading starts on in the layout being drawn. | this spec section 3 | not built -- lands with the "Tables, table of contents and word count" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-113` | `INV` | The document MUST hold no entry: the block carries the deepest level it shows and whether it numbers them, and nothing else. | this spec section 3; this spec section 5 | not built -- lands with the "Tables, table of contents and word count" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-061` | `OBS` | A heading deeper than the configured level MUST NOT appear in the table of contents. | this spec section 3 | not built -- lands with the "Tables, table of contents and word count" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-062` | `OBS` | A section number MUST appear in front of an entry only while the table of contents is configured as numbered. | this spec section 3 | not built -- lands with the "Tables, table of contents and word count" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-114` | `CONC` | Two peers whose settings have converged MUST list the same entries, at the same depth, numbered the same way. | this spec section 2; this spec section 5 | not built -- lands with the "Tables, table of contents and word count" Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.2 Insert a table of contents

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|---|
| `PROSE-FEAT-115` | `POST` | After the command, a table of contents MUST sit at the place in the tree the command named, showing the depth and the numbering it carried. | this spec section 3 | not built -- lands with the "Tables, table of contents and word count" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-116` | `FRAME` | Inserting one MUST NOT touch a heading, and MUST NOT write an entry anywhere. | derived | not built -- lands with the "Tables, table of contents and word count" Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.3 Change what it shows

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|---|
| `PROSE-FEAT-117` | `POST` | After the command, the block MUST carry the depth and the numbering asked for, and the next layout MUST list its entries by them. | this spec section 3 | not built -- lands with the "Tables, table of contents and word count" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-118` | `FRAME` | Changing one block's settings MUST NOT touch a heading, nor the settings of another table of contents. | this spec section 5 | not built -- lands with the "Tables, table of contents and word count" Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.4 Click an entry

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|---|
| `PROSE-UI-082` | `POST` | A click on an entry MUST bring the heading it reads into view. | this spec section 4 | not built -- lands with the "Tables, table of contents and word count" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-UI-083` | `FRAME` | Following an entry MUST NOT write to the document: where this reader looks is not a change a peer receives. | derived | not built -- lands with the "Tables, table of contents and word count" Story (`ambre-roadmap.md` E5) | unimplemented |

* **Budget.** the table-of-contents build over fifty headings, one row of
  `ambre-spec-budgets.md section 1`. What the repeated build costs when the entries move the
  page count is the deferred pass of `features/ambre-spec-prose-pagination.md`, `PROSE-019`.
* **Boundary.** none of its own: the block carries two settings and no text, and `ZK-001` seals
  every payload before it leaves the client.
* **Verification.** 12 `unimplemented`, nothing else: `crates/ambre-prose` is a seven-line
  `lib.rs`, and `ambre-bus` carries no Prose command. Two rows stated two rules each, and both
  second halves were the same ban said twice, so they become one: `PROSE-FEAT-113` is what the
  block may hold, and the schema table already agrees -- the `Toc` node carries `max_level` and
  `numbered` and nothing else (`ambre-spec-prose-schema.md section 3`), which is the closest thing
  to a mechanism any row here has. `PROSE-FEAT-003` keeps document order, which the ban does not
  imply: a list built by walking the headings in any other order stores nothing either.
  `PROSE-FEAT-114` is the agreement the fourth scenario of section 2 rests on and no row carried,
  and it is weaker than it reads -- it holds over the entries, not over the page numbers, which
  section 6 records as an open gap of pagination. The two commands and the one gesture had no rows
  at all: nothing said what an insert leaves behind or may not touch, nothing said that a settings
  change reaches the next layout, and nothing said that following an entry writes nothing, which is
  the whole difference between a link and an edit. `PROSE-FEAT-113`, `PROSE-FEAT-115`,
  `PROSE-FEAT-116`, `PROSE-FEAT-117`, `PROSE-FEAT-118` and `PROSE-UI-083` are what a host test
  holds over a document fixture; the six that read a layout need one and land in the wasm tier of
  `ambre-spec-testing.md section 6`.

---

## 2. WHAT EACH USER SEES

Two peers over one outline. No test resolves yet: the last column names the row each row rests on.

| Alice does | Bob does | The document holds | Each user sees | Test |
|---|---|---|---|---|
| adds a heading in the middle | -- | one more heading, no entry anywhere | one more line in the table of contents, on both screens | none -- `PROSE-FEAT-003` |
| edits a heading's text | -- | the new text, in the heading alone | the entry reading the new text, on both screens | none -- `PROSE-FEAT-003` |
| types a page of text before a heading | -- | the text | the entry's page number one higher, on both screens | none -- `PROSE-FEAT-060` |
| lowers the configured level | raises it at the same moment | one setting, the later of the two | the same depth of entries on both screens | none -- `PROSE-FEAT-114` |
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
* **Nothing bounds the configured depth.** `max_level` is a `u8` and a heading level is one to six
  (`features/ambre-spec-prose-headings.md`), so zero and two hundred are both storable. A depth of
  zero is a table of contents that lists nothing, and no rule says whether a command may write one
  or what the block shows when it holds one.
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
  same one the cross references and the comments rest on. It lands with the `Two peers read the
  same page number` Story (`ambre-roadmap.md` E5).
* **Three more derived blocks are designed and unbuilt, and each is this walk with another
  predicate.** A table of figures and a table of tables list the captioned pictures and tables; an
  index lists the terms an author marked, alphabetically, with the pages they fall on. All three
  reuse the settling pass and the page numbers of section 3, none has a block type in
  `ambre-spec-prose-schema.md section 3`, and the index needs a mark key that is not declared
  either. The Story is "The later writing set" (`ambre-roadmap.md` POST-V1).
