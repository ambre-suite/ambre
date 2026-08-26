# AMBRE -- FEATURE: CROSS REFERENCES

**Type:** Vertical (feature)
**Implementation:** design-only

> A pointer from one place in the document to another: "see page 4", "see section 2.1", or a label
> the author chose. The pointer names the block it refers to, so moving that block anywhere keeps
> the reference working.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`.

| ID | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|
| `PROSE-FEAT-014` | A reference whose target is gone MUST show that it is broken; it MUST NOT resolve to a neighbour. | this spec section 3 | not built -- lands with the "Undo, footnotes and cross references" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-072` | A reference's stored value MUST be a record whose parts are delimited by their length; it MUST NOT be a string split on a separator. | this spec section 3 | not built -- lands with the "Undo, footnotes and cross references" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-073` | A reference MUST name its target by the target's own identity, never by the text the target reads. | this spec section 3 | not built -- lands with the "Undo, footnotes and cross references" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-074` | A broken reference MUST stay in the document, so restoring its target MUST restore the reference. | this spec section 3 | not built -- lands with the "Undo, footnotes and cross references" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-UI-048` | A reference carrying its own label MUST keep showing that label when its target is gone. | this spec section 4 | not built -- lands with the "Undo, footnotes and cross references" Story (`ambre-roadmap.md` E5) | unimplemented |

* **Budget.** none of its own: a reference is resolved with the page it is drawn on, from the page
  layout and the section numbers the headings feature derives, and both figures are rows of
  `ambre-spec-budgets.md section 1` owned by pagination and by the headings.
* **Boundary.** none of its own: a reference is a mark on the text, and `ZK-001` seals every
  payload before it leaves the client.
* **Verification.** 5 `unimplemented`, nothing else: `crates/ambre-prose` is a `lib.rs`.
  `PROSE-FEAT-072` is the row a test reaches with one fixture and no document: a label holding the
  separator the old form used must round-trip unchanged. `PROSE-FEAT-074` is the one a two-peer
  test reaches, because restoring a target is what an undo of the deletion does.

---

## 2. WHAT EACH USER SEES

Two peers over one reference. No test resolves yet: the last column names the row each row rests
on.

| Alice does | Bob does | The document holds | Each user sees | Test |
|---|---|---|---|---|
| refers to a heading further down | -- | the reference and the heading's identity | the heading's page number, on both screens | none -- `PROSE-FEAT-073` |
| types a page of text above the target | -- | the text | the reference's page number one higher, on both screens | none -- `PROSE-FEAT-073` |
| moves the target heading to another chapter | -- | the same identity, in a new place | the reference showing the new page, on both screens | none -- `PROSE-FEAT-073` |
| deletes the target | -- | the reference, with nothing to resolve | a broken reference on both screens, not a neighbour's number | none -- `PROSE-FEAT-014` |
| undoes that deletion | -- | the target again | the reference resolving again, on both screens | none -- `PROSE-FEAT-074` |
| refers to a heading with a label of her own containing a colon | -- | the label, whole | the label as she typed it, on both screens | none -- `PROSE-FEAT-072` |

---

## 3. MODEL

**Containers.** None of its own. A reference is an inline mark on the text, declared in
`ambre-spec-prose-schema.md section 3`. Its value is the whole of what it stores.

**The value is a record, not a joined string:**

```rust
/// What the reference mark stores. Encoded as a length-prefixed record, hex in the
/// mark's string value (`PROSE-FEAT-072`).
pub struct CrossRefValue {
    /// The target's own identity, never its text (`PROSE-FEAT-073`).
    pub target_block_id: TreeID,
    pub kind:            CrossRefKind,
}

pub enum CrossRefKind {
    /// The page the target starts on.
    PageNumber,
    /// The target heading's derived section number.
    SectionNumber,
    /// A label the author wrote. It may hold any character, because the record
    /// carries its length rather than a separator.
    CustomText(String),
}
```

The earlier form joined the target and the kind with a colon, which a label containing a colon
splits in the wrong place. Length-prefixing removes the class of defect rather than escaping it
(`PROSE-FEAT-072`).

**What is resolved, and when.** A page number is read from the current page layout; a section
number is the one the headings feature derives (`features/ambre-spec-prose-headings.md`,
`PROSE-FEAT-030`). Both are resolved when the page is drawn, so both follow every edit above the
target with no write.

**The broken reference.** When the target no longer resolves to a live block, the reference is
neither an error nor a deletion (`PROSE-FEAT-014`): the view draws a neutral broken mark, and the
mark stays in the document (`PROSE-FEAT-074`). A concurrent undo, or a remote peer re-inserting the
target, therefore brings the reference back on its own. A reference carrying its own label keeps
showing that label, because the label is in the reference and needs no target to be read
(`PROSE-UI-048`).

**The command** (`ambre-bus`, declared in `features/ambre-spec-prose-engine.md section 3`):

| Shape | Behaviour |
|---|---|
| `InsertCrossRef { block_id, pos, target_block_id, kind }` | a reference at a position, naming a target |

There is no command that edits a reference: a reference is changed by removing it and inserting
another, and it is deleted as one character with the text it sits in.

**Code:** none. `crates/ambre-prose/src/lib.rs` is seven lines.

---

## 4. SURFACE

| Trigger | Behaviour |
|---|---|
| the cross-reference entry of the insert menu | a picker of the document's headings, then a reference at the caret |
| the kind control of the picker | a page number, a section number, or a label to type |
| a click on a reference | moves the view to its target |
| a broken reference | drawn as a neutral broken mark, or as its own label |
| deleting the character | removes the reference |

A reference is drawn as its resolved value and behaves as one character to the caret: it is
selected whole and deleted whole.

---

## 5. DECISIONS

| Decision | Rationale | Rejected alternative |
|---|---|---|
| The target is named by its identity | a block's identity survives a move, an indent and a regroup, so a reference keeps meaning the same block whatever happens around it. Nothing else in the document has that property (`PROSE-FEAT-073`) | naming the target by its text, which breaks the moment the heading is reworded, and matches the wrong heading when two read alike |
| The value is a length-prefixed record | a label is free text and may contain any character, including whatever separator a joined form picks. Length-prefixing makes the parse independent of the content, so no label can be misread (`PROSE-FEAT-072`) | joining the target and the kind with a separator, which splits a label containing that separator in the wrong place |
| The displayed value is resolved when the page is drawn | a page number and a section number are facts about a layout and an outline, both of which change on edits far from the reference. Resolving late means the reference is never stale, and nothing about it enters the merge path | storing the resolved number in the mark, which is wrong one keystroke later and races between two peers |
| A broken reference is kept, not removed | the deletion of the target may be a mistake, a concurrent edit or an undo away from being reversed, and a reference removed cannot come back. Keeping it makes the repair automatic (`PROSE-FEAT-074`) | deleting the mark when the target dies, which loses the author's pointer permanently on a remote peer's edit |
| A broken reference never resolves to a neighbour | a neighbour is a different block, and a reference silently pointing somewhere else is worse than one visibly broken: the reader has no way to know it moved (`PROSE-FEAT-014`) | resolving to the nearest surviving block, which produces a reference that is confidently wrong |
| A label survives its target | the label is the reference's own text, so nothing about it depends on the target existing. Blanking it would lose content the author typed here (`PROSE-UI-048`) | showing the broken mark for every kind, which throws away a label the reference is carrying |
| A reference is one atom to the caret | its displayed text is derived, so a caret inside it would be a position in a string that is not in the document. Selecting and deleting it whole is the only editing model that matches what is stored | letting the caret enter the resolved text, which offers an edit that cannot be written anywhere |

---

## 6. NOT BUILT, AND NOT YET RULED

* **Nothing rules on what a reference may target.** The picker above offers the headings; the
  stored value names any block, so a reference to a table, a picture or a paragraph is
  representable and undecided.
* **A reference to a numbered item shows no label of its own.** A page number and a section number
  resolve to a bare figure, and nothing says whether "see" or "page" is drawn with it or typed by
  the author.
* **The import path drops what this document keeps.** `features/ambre-spec-prose-engine.md section 3`
  states that a reference whose target does not resolve is dropped at import validation, while
  `PROSE-FEAT-074` keeps a broken reference in the document. Which rule applies to a file being
  opened is undecided.
* **Two peers may lay out the same document differently**, and every page number here rests on
  their agreeing. The gap is `features/ambre-spec-prose-pagination.md section 5`, and the table of
  contents and the comments rest on it too.
* **Nothing lists the references to a block.** An author deleting a heading is told nothing about
  the references that will break, and no command answers the question.
* **A bookmark is a second kind of target, designed and unbuilt.** The deferred design marks a
  range with an identity and keeps the author's name for it in `annots`, so two bookmarks may share
  a display name and still resolve to one place; the annotation entry already carries the three
  fields (`ambre-spec-prose-schema.md section 3`) and no mark key does. Nothing says what a
  reference to a bookmark renders, which is the question this feature would inherit.
* **A citation is a third, and it needs a store this feature does not own.** The deferred design
  puts the sources in a `bibliography` map, marks each citation with the source's key, and formats
  both the citation and the generated list on demand from the chosen style. The container is
  declared and unwritten (`ambre-spec-prose-schema.md section 3`), the mark key is not declared at
  all, and the generated list is a derived block like the table of contents rather than a reference.
  Both designs are the "The later writing set" Story (`ambre-roadmap.md`).
