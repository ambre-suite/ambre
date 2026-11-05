# AMBRE -- FEATURE: CROSS REFERENCES

**Type:** Vertical (feature)
**Implementation:** partial -- an undone deletion leaves its references broken

> A pointer from one place in the document to another: "see page 4", "see section 2.1", or a label
> the author chose. The pointer names the block it refers to, so moving that block anywhere keeps
> the reference working.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-FEAT-073` | `INV` | A reference MUST name its target by the target's own identity, never by the text the target reads. | this spec section 3; `ambre-spec-prose-schema.md section 2` | `ambre_prose::logic::xrefs::tests::a_reference_names_its_target_by_identity_and_writes_nothing_there` | none | verified |
| `PROSE-FEAT-072` | `INV` | A reference's stored value MUST be a record whose parts are delimited by their length; it MUST NOT be a string split on a separator. | this spec section 3; `ambre-spec-prose-schema.md section 3` | `ambre_prose::xref::tests::a_value_is_a_length_prefixed_record_and_never_a_joined_string` | none | verified |
| `PROSE-FEAT-128` | `IO` | A reference read back MUST carry the label the author typed, whatever characters it holds. | this spec section 3; this spec section 5 | `ambre_prose::xref::tests::a_label_holding_any_character_reads_back_whole` | none | verified |
| `PROSE-FEAT-129` | `INV` | The value a reference is drawn as MUST NOT be stored in the document. | this spec section 3; this spec section 5 | `ambre_prose::logic::xrefs::tests::a_page_reference_follows_its_target_to_a_later_page` | none | verified |
| `PROSE-FEAT-130` | `OBS` | A reference to a page MUST show the page its target starts on in the layout being drawn. | this spec section 3 | `ambre_prose::logic::xrefs::tests::a_page_reference_follows_its_target_to_a_later_page`; `::a_reference_shows_the_heading_number_and_the_page_of_its_target` | none | verified |
| `PROSE-FEAT-131` | `OBS` | A reference to a heading MUST show the heading number the headings feature derives for its target. | this spec section 3; `PROSE-FEAT-030` | `ambre_prose::logic::xrefs::tests::a_reference_shows_the_heading_number_and_the_page_of_its_target` | none | verified |
| `PROSE-FEAT-014` | `OBS` | A reference whose target is gone MUST be drawn as a broken mark. | this spec section 3 | `ambre_prose::logic::xrefs::tests::a_gone_target_breaks_the_reference_and_the_reference_stays` | none | verified |
| `PROSE-FEAT-132` | `INV` | A reference MUST NOT resolve to any block but the one it names. | this spec section 5 | `ambre_prose::logic::xrefs::tests::a_gone_target_breaks_the_reference_and_the_reference_stays` | none | verified |
| `PROSE-FEAT-074` | `INV` | A reference whose target is gone MUST stay in the document. | this spec section 3; this spec section 5 | `ambre_prose::logic::xrefs::tests::a_gone_target_breaks_the_reference_and_the_reference_stays` | none | verified |
| `PROSE-FEAT-133` | `CONC` | A target restored by an undo MUST make the reference resolve again on every peer. | this spec section 2; this spec section 3 | `ambre_prose::logic::xrefs::tests::a_target_an_undo_restores_resolves_again_on_the_other_peer` | none | verified |
| `PROSE-UI-048` | `OBS` | A reference carrying its own label MUST keep showing that label when its target is gone. | this spec section 4 | `ambre_prose::logic::xrefs::tests::a_gone_target_breaks_the_reference_and_the_reference_stays` | none | verified |
| `PROSE-UI-084` | `INV` | A caret position MUST NOT fall inside the value a reference is drawn as. | this spec section 4; this spec section 5 | `ambre_prose::logic::xrefs::tests::a_reference_is_one_character_to_the_caret` | none | verified |

### 1.2 Insert a cross reference

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-FEAT-134` | `POST` | After the command, a reference naming the chosen target MUST sit at the position it was given. | this spec section 3; this spec section 4 | `ambre_prose::logic::xrefs::tests::a_reference_names_its_target_by_identity_and_writes_nothing_there`; `::the_resolved_list_names_where_each_reference_sits` | none | verified |
| `PROSE-FEAT-135` | `FRAME` | Inserting a reference MUST NOT touch the target block: no anchor, no mark and no text is written there. | derived | `ambre_prose::logic::xrefs::tests::a_reference_names_its_target_by_identity_and_writes_nothing_there` | none | verified |

### 1.3 Click a reference

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-UI-085` | `POST` | A click on a reference MUST bring its target into view. | this spec section 4 | `ambre_prose::logic::xrefs::tests::following_a_reference_reveals_its_target_and_writes_nothing` | none | verified |
| `PROSE-UI-086` | `FRAME` | A click on a reference MUST NOT write to the document. | derived | `ambre_prose::logic::xrefs::tests::following_a_reference_reveals_its_target_and_writes_nothing` | none | verified |

* **Budget.** none of its own: a reference is resolved with the page it is drawn on, from the
  layout and the heading numbers the headings derive. Both figures are rows of
  `ambre-spec-budgets.md section 1`, owned by pagination and by the headings.
* **Boundary.** none of its own: a reference is a mark on the text, and `ZK-001` seals every
  payload before it leaves the client.
* **Verification.** 16 `verified`. The record's round trip is a unit test with no document
  (`PROSE-FEAT-072`, `PROSE-FEAT-128`); the rest drive the engine. `PROSE-FEAT-133` runs two peers:
  one deletes the target and undoes it, and the other sees the reference resolve again.

---

## 2. WHAT EACH USER SEES

Two peers over one reference. The last column names the test that plays the row, or the row it
rests on where none does.

| Alice does | Bob does | The document holds | Each user sees | Test |
|---|---|---|---|---|
| refers to a heading further down | -- | the reference and the heading's identity | the heading's page number, on both screens | `a_reference_shows_the_heading_number_and_the_page_of_its_target` |
| types a page of text above the target | -- | the text | the reference's page number one higher, on both screens | `a_page_reference_follows_its_target_to_a_later_page` |
| moves the target heading to another chapter | -- | the same identity, in a new place | the reference showing the new page, on both screens | `a_reference_shows_the_heading_number_and_the_page_of_its_target` |
| deletes the target | -- | the reference, with nothing to resolve | a broken reference on both screens, not a neighbour's number | `a_gone_target_breaks_the_reference_and_the_reference_stays` |
| undoes that deletion | -- | the target again | the reference resolving again, on both screens | `a_target_an_undo_restores_resolves_again_on_the_other_peer` |
| refers to a heading with a label of her own containing a colon | -- | the label, whole | the label as she typed it, on both screens | `a_label_holding_any_character_reads_back_whole` |

---

## 3. MODEL

**Containers.** None of its own. A reference is an inline mark on the text, declared in
`ambre-spec-prose-schema.md section 3`. Its value is the whole of what it stores.

**The value is a record, not a joined string:**

```rust
// ambre-prose/src/xref.rs
/// What the reference mark stores: postcard prefixes a label with its length, and the bytes
/// are written in hex in the mark's string value (`PROSE-FEAT-072`).
#[derive(Serialize, Deserialize)]
struct Record {
    /// The target's own identity, never its text (`PROSE-FEAT-073`): the peer and the
    /// counter of its `TreeID`.
    peer:    PeerId,
    counter: OpCounter,
    kind:    CrossRefKind,
}

/// The hex text of a `Record`, as the mark stores it. It spells a label of the document, so it
/// has no `Debug`.
#[derive(Clone, PartialEq, Eq)]
pub struct CrossRefMark(String);

pub fn encode(target: TreeID, kind: &CrossRefKind) -> CrossRefMark;
pub fn decode(value: &str) -> Option<(TreeID, CrossRefKind)>;

// ambre_prose::message. The variant's position is its stored code: a new kind is appended,
// never inserted.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CrossRefKind {
    /// The page the target starts on.
    PageNumber,
    /// The derived number of the target heading.
    HeadingNumber,
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
`PROSE-FEAT-030`, `PROSE-FEAT-130`, `PROSE-FEAT-131`). Both are resolved when the page is drawn, so
both follow every edit above the target with no write (`PROSE-FEAT-129`).

**The broken reference.** When the target no longer resolves to a live block, the reference is
neither an error nor a deletion (`PROSE-FEAT-014`). The view draws a neutral broken mark, and the
mark stays in the document (`PROSE-FEAT-074`). An undo of the deletion therefore brings the
reference back on its own (`PROSE-FEAT-133`). A peer who retypes the heading does not, because a
new block is a new identity (`PROSE-FEAT-073`). A reference carrying its own label keeps
showing that label, because the label is in the reference and needs no target to be read
(`PROSE-UI-048`).

**The command** (`ambre_prose::message`, declared in `features/ambre-spec-prose-engine.md section 3`):

| Shape | Behaviour |
|---|---|
| `InsertCrossRef { block_id, pos, target_block_id, kind }` | a reference at a position, naming a target (`PROSE-FEAT-134`, `PROSE-FEAT-135`) |

There is no command that edits a reference. A reference is changed by removing it and inserting
another, and it is deleted as one character with the text it sits in.

**The reference is a carrier character.** The insert writes one U+FFFC and marks it with the
record, so the reference is one character to the caret and is deleted whole (`PROSE-UI-084`). A
note's call is written the same way.

**Code:** `crates/ambre-prose/src/xref.rs` encodes the record and resolves what it shows.
`crates/ambre-prose/src/logic/xrefs.rs` holds the insert and the follow. The resolved list reaches
the view as `CrossRefsUpdated`, after an insert and at each pause in typing.

---

## 4. SURFACE

| Trigger | Behaviour |
|---|---|
| the cross-reference entry of the insert menu | a picker of the document's headings, then a reference at the caret |
| the kind control of the picker | a page number, a heading number, or a label to type |
| a click on a reference | moves the view to its target (`PROSE-UI-085`), and writes nothing (`PROSE-UI-086`) |
| a broken reference | drawn as a neutral broken mark, or as its own label |
| deleting the character | removes the reference |

A reference is drawn as its resolved value and behaves as one character to the caret. It is
selected whole and deleted whole, and no caret position falls inside it (`PROSE-UI-084`).

---

## 5. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| The target is named by its identity | naming the target by its text | it breaks the moment the heading is reworded, and matches the wrong heading when two read alike. A block's identity survives a move, an indent and a regroup, and nothing else in the document has that property (`PROSE-FEAT-073`) |
| The value is a length-prefixed record | joining the target and the kind with a separator | a label is free text and may contain that separator, so the parse splits in the wrong place. Length-prefixing makes the parse independent of the content, so no label can be misread (`PROSE-FEAT-072`) |
| The displayed value is resolved when the page is drawn | storing the resolved number in the mark | it is wrong one keystroke later and races between two peers. A page number and a heading number are facts about a layout and an outline, both of which change on edits far from the reference, so resolving late keeps it out of the merge path |
| A broken reference is kept, not removed | deleting the mark when the target dies | it loses the author's pointer permanently on a remote peer's edit. The deletion may be a mistake, a concurrent edit or an undo away from being reversed, and keeping the reference makes the repair automatic (`PROSE-FEAT-074`) |
| A broken reference never resolves to a neighbour | resolving to the nearest surviving block | it produces a reference that is confidently wrong. A neighbour is a different block, and a reference silently pointing somewhere else is worse than one visibly broken, because the reader has no way to know it moved (`PROSE-FEAT-014`) |
| A label survives its target | showing the broken mark for every kind | it throws away a label the reference is carrying. The label is the reference's own text, so nothing about it depends on the target existing (`PROSE-UI-048`) |
| A reference is one atom to the caret | letting the caret enter the resolved text | it offers an edit that cannot be written anywhere: the displayed text is derived, so a caret inside it is a position in a string that is not in the document |

---

## 6. NOT BUILT, AND NOT YET RULED

* **Nothing rules on what a reference may target.** The picker above offers the headings. The
  stored value names any block, so a reference to a table, a picture or a paragraph is
  representable and undecided.
* **A reference to a numbered item shows no label of its own.** A page number and a heading number
  resolve to a bare figure. Nothing says whether "see" or "page" is drawn with it or typed by
  the author.
* **The import path drops what this document keeps.** `features/ambre-spec-prose-engine.md section 3`
  states that a reference whose target does not resolve is dropped at import validation, while
  `PROSE-FEAT-074` keeps a broken reference in the document. Which rule applies to a file being
  opened is undecided.
* **Two peers may lay out the same document differently**, and every page number here rests on
  their agreeing. The gap is `features/ambre-spec-prose-pagination.md section 5`, and the table of
  contents and the comments rest on it too. It lands with the `Two peers read the same page number`
  Story (`ambre-roadmap.md` E5).
* **Nothing lists the references to a block.** An author deleting a heading is told nothing about
  the references that will break, and no command answers the question.
* **A bookmark is a second kind of target, designed and unbuilt.** The deferred design marks a
  range with an identity and keeps the author's name for it in `annots`. Two bookmarks may share
  a display name and still resolve to one place. The annotation entry already carries the three
  fields (`ambre-spec-prose-schema.md section 3`), and no mark key does. Nothing says what a
  reference to a bookmark renders, which is the question this feature would inherit.
* **A citation is a third, and it needs a store this feature does not own.** The deferred design
  puts the sources in a `bibliography` map and marks each citation with the source's key. It
  formats both the citation and the generated list on demand from the chosen style. The container
  is declared and unwritten (`ambre-spec-prose-schema.md section 3`), and the mark key is not
  declared at all. The generated list is a derived block like the table of contents rather than a
  reference.
  Both designs are the `The later writing set` Story (`ambre-roadmap.md` POST-V1).
