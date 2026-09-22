# AMBRE -- FEATURE: HEADERS AND FOOTERS

**Type:** Vertical (feature)
**Implementation:** design-only

> A band at the top of every page and a band at the bottom, each holding ordinary text. A page
> number, a page count, a date or the title may be placed in either one. None of them is
> written down: they are resolved when the page is drawn.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-006` | `INV` | A header and a footer MUST be ordinary blocks addressed by a stable identity. | this spec section 3; `ambre-spec-prose-schema.md section 3` | not built | unbuilt -- lands with the "Headers, footers, find and replace" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-160` | `INV` | No command MUST carry a band's edits inside another command: the text of a band MUST be edited by the ordinary text commands, against the block the metadata names. | this spec section 3 | not built | unbuilt -- lands with the "Headers, footers, find and replace" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-161` | `INV` | A band's configuration MUST hold the identity of its block, whether the side is shown, and its alignment. | this spec section 3 | not built | unbuilt -- lands with the "Headers, footers, find and replace" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-281` | `INV` | A band's configuration MUST hold no copy of its block's text. | this spec section 3 | not built | unbuilt -- lands with the "Headers, footers, find and replace" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-063` | `INV` | One document-level setting MUST govern the first page's header and its footer together. | this spec section 3 | not built | unbuilt -- lands with the "Headers, footers, find and replace" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-064` | `INV` | While that setting is off, the first page's own blocks MUST be kept. | this spec section 3 | not built | unbuilt -- lands with the "Headers, footers, find and replace" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-UI-133` | `OBS` | While that setting is off, no page MUST draw the first page's own blocks: the first page MUST draw the same two bands as every other page. | this spec section 3 | not built | unbuilt -- lands with the "Headers, footers, find and replace" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-065` | `INV` | A field MUST store no rendered value in the document: it MUST store what to show, never what it shows today. | this spec section 3 | not built | unbuilt -- lands with the "Headers, footers, find and replace" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-162` | `INV` | The date field MUST store a format, never a locale. | this spec section 3 | not built | unbuilt -- lands with the "Headers, footers, find and replace" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-282` | `INV` | The date field MUST be rendered in the document's language. | this spec section 3 | not built | unbuilt -- lands with the "Headers, footers, find and replace" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-066` | `INV` | The date field MUST be resolved from the document's own last-modified stamp, never from the reader's clock. | this spec section 3 | not built | unbuilt -- lands with the "Headers, footers, find and replace" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-163` | `INV` | A field MUST be one atom to the caret: typing beside it never extends it. | this spec section 3; this spec section 4 | not built | unbuilt -- lands with the "Headers, footers, find and replace" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-283` | `INV` | A field MUST be selected whole and deleted whole. | this spec section 3; this spec section 4 | not built | unbuilt -- lands with the "Headers, footers, find and replace" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-UI-047` | `OBS` | A field MUST be resolved when the page is drawn, from the page it is drawn on. | this spec section 4 | not built | unbuilt -- lands with the "Headers, footers, find and replace" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-164` | `CONC` | Two peers editing one band's block concurrently MUST converge on the text a body paragraph would carry for the same two edits. | this spec section 3 | not built | unbuilt -- lands with the "Headers, footers, find and replace" Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.2 Put a field in a band

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-FEAT-165` | `POST` | The field's mark MUST be written into the block the insertion's target names, and into no other block of that side. | this spec section 3 | not built | unbuilt -- lands with the "Headers, footers, find and replace" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-068` | `POST` | A field position past the end of its target MUST be clamped to that end. | this spec section 3 | not built | unbuilt -- lands with the "Headers, footers, find and replace" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-067` | `FAIL` | Inserting a field into the first page's own block MUST be refused while the setting is off, and the refusal MUST be reported. | this spec section 3 | not built | unbuilt -- lands with the "Headers, footers, find and replace" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-166` | `FRAME` | An insertion MUST NOT change any band's configuration, the document-level setting, or the text on either side of the mark. | this spec section 3 | not built | unbuilt -- lands with the "Headers, footers, find and replace" Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.3 Show or hide a band

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-FEAT-167` | `POST` | Showing or hiding one side MUST change that side's own setting. | this spec section 3 | not built | unbuilt -- lands with the "Headers, footers, find and replace" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-284` | `POST` | Showing or hiding one side MUST leave the other side shown or hidden as it was. | this spec section 3 | not built | unbuilt -- lands with the "Headers, footers, find and replace" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-168` | `FRAME` | Hiding a side MUST NOT touch the block it names: the text MUST be kept, so showing the side again MUST bring back what was written there. | this spec section 3 | not built | unbuilt -- lands with the "Headers, footers, find and replace" Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.4 Turn the first page's own bands on

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-FEAT-169` | `POST` | Turning the setting on MUST make the first page, and only the first page, draw its own header and its own footer. | this spec section 3 | not built | unbuilt -- lands with the "Headers, footers, find and replace" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-170` | `FRAME` | The setting MUST NOT change whether a side is shown, and MUST NOT touch the four blocks the bands name. | this spec section 3 | not built | unbuilt -- lands with the "Headers, footers, find and replace" Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.5 Align a band

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-FEAT-171` | `POST` | An alignment MUST be written into the configuration of the band the caret is in. | this spec section 3; this spec section 4 | not built | unbuilt -- lands with the "Headers, footers, find and replace" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-285` | `POST` | An alignment MUST apply to its band on every page that band is drawn on. | this spec section 3; this spec section 4 | not built | unbuilt -- lands with the "Headers, footers, find and replace" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-172` | `FRAME` | An alignment MUST NOT change the block's text, the paragraph styles inside it, or the alignment of any other band. | this spec section 3 | not built | unbuilt -- lands with the "Headers, footers, find and replace" Story (`ambre-roadmap.md` E5) | unimplemented |

* **Budget.** none of its own: a band is measured once per document and subtracted from every page.
  That is the second step of `features/ambre-spec-prose-pagination.md section 3`, charged to
  the pagination rows of `ambre-spec-budgets.md section 1`.
* **Boundary.** none of its own: a header is a block like any other, and `ZK-001` seals every
  payload before it leaves the client.
* **Verification.** 27 `unimplemented`, nothing else: `crates/ambre-prose` is a `lib.rs`.
  `PROSE-FEAT-160` is compiler-held the day the command enum exists, because a nested content
  command is a variant that is not there. `PROSE-FEAT-065`, `PROSE-FEAT-066` and `PROSE-FEAT-164`
  are the three a two-peer test reaches without a page. Two peers whose clocks differ must render
  one date, and two peers typing in one band must converge. The five `OBS` and `POST` rows about
  what a page draws reach nothing until pagination places a page.

---

## 2. WHAT EACH USER SEES

Two peers over one band. No test resolves yet: the last column names the row each row rests on.

| Alice does | Bob does | The document holds | Each user sees | Test |
|---|---|---|---|---|
| types in the header | types in the header at the same moment | both sentences, merged as in any paragraph | the same header on both screens | none -- `PROSE-FEAT-164` |
| inserts a page number in the footer | -- | the field, and no number | page one on page one, page two on page two | none -- `PROSE-UI-047` |
| inserts a date field | reads the document in another time zone | the field and the format | the same date on both screens | none -- `PROSE-FEAT-066` |
| turns the first-page setting on | -- | the setting | the first page taking its own header, the rest unchanged | none -- `PROSE-FEAT-169` |
| turns it off again | -- | the setting, and the first page's blocks | the ordinary header everywhere, and the first page's text kept for later | none -- `PROSE-FEAT-064`, `PROSE-UI-133` |
| types a page of text | -- | the text | the page count in the footer one higher, on both screens | none -- `PROSE-UI-047` |

---

## 3. MODEL

**Containers.** The document metadata carries the configuration of four bands: the header, the
footer, and the first page's own header and footer. It also carries the document-level setting that
turns the last two on. Each configuration holds whether the side is shown, the identity of the block that
holds its text, and its alignment. All of them are declared in
`ambre-spec-prose-schema.md section 3`.

**The text is a block, not a field.** The metadata holds the block's identity and nothing of its
text (`PROSE-FEAT-161`); the text lives in the block tree like any other text (`PROSE-006`). Every
ordinary editing command therefore works on a header by naming that block: typing, marks, paragraph
styles, the cascade. There is no second editing surface and no command that carries another command
(`PROSE-FEAT-160`). Two peers in one band merge as they would in a body paragraph
(`PROSE-FEAT-164`).

**The four fields.** A field is an inline mark carrying no rendered text
(`ambre-spec-prose-schema.md section 3`): the page number, the page count, the last-modified date
and the document title. The mark's key is the field, and adjacent typing never joins it, so a field
is one atom to the caret (`PROSE-FEAT-163`).

```rust
pub enum HeaderField { PageNumber, PageCount, Date(DateFieldFormat), Title }

/// What the date field stores: a format, never a rendered date (`PROSE-FEAT-065`).
pub enum DateFieldFormat {
    /// The locale's short form.
    Short,
    /// The locale's long form.
    Long,
    /// The calendar date, the same in every locale.
    Iso,
}

/// Which of the two blocks of a side a field insertion targets.
pub enum HfTarget { Regular, FirstPage }
```

The page number and the page count are resolved from the page the band is drawn on
(`PROSE-UI-047`). The date is resolved from the document's own last-modified stamp and its
language. A reader's wall clock is never read: two readers would then render two dates for
one document (`PROSE-FEAT-066`). The short and the long forms follow the document's language, and
the calendar form is the same everywhere. The field stores which of the three, never a locale
(`PROSE-FEAT-162`).

**The first page.** One setting, on the document and not on a side, turns both first-page blocks on
at once (`PROSE-FEAT-063`, `PROSE-FEAT-169`). While it is off, the two blocks are kept
(`PROSE-FEAT-064`) and never drawn (`PROSE-UI-133`), so turning it on again brings back what was
written there. Pagination reads this one setting
to decide, on the first page alone, which of the two blocks a side draws. Whether a side is drawn
at all is that side's own setting.

**The commands** (`ambre-bus`, declared in `features/ambre-spec-prose-engine.md section 3`):

| Shape | Behaviour |
|---|---|
| `SetHeaderEnabled { enabled }` | shows or hides the top band (`PROSE-FEAT-167`) |
| `SetFooterEnabled { enabled }` | shows or hides the bottom band (`PROSE-FEAT-167`) |
| `SetDifferentFirstPage { enabled }` | turns both first-page blocks on or off |
| `InsertHeaderField { field, target, pos }` | a field in the header block the target names |
| `InsertFooterField { field, target, pos }` | a field in the footer block the target names |

Editing the text of a band uses the ordinary text commands against the block the metadata names.

**What a field insertion does, in order:**

1. resolve the target: the regular block, or the first page's own.
2. refuse the first page's block while the setting is off (`PROSE-FEAT-067`). It has no surface to
   appear on, so the insertion would be invisible and unexplained.
3. clamp a position past the end of the target's text to that end (`PROSE-FEAT-068`).
4. write the field's mark at that position, in the block the target names (`PROSE-FEAT-165`).

**Where a band is measured.** The two bands are measured once against the content width, and the
first-page variants beside them, before any page is placed. A band is capped at a quarter of the
page height. Both are `features/ambre-spec-prose-pagination.md`, `PROSE-016`.

**Code:** none. `crates/ambre-prose/src/lib.rs` is seven lines.

---

## 4. SURFACE

| Trigger | Behaviour |
|---|---|
| a double click in the top or bottom margin | puts the caret in that band |
| the header and footer entries of the insert menu | shows the side and puts the caret in it |
| the field entries of the insert menu | a page number, a page count, a date or the title, at the caret |
| the different-first-page switch | turns the first page's own blocks on or off |
| the alignment control while the caret is in a band | left, centre or right for that band (`PROSE-FEAT-171`) |
| Escape, or a click in the body | returns the caret to the document |

A field is drawn as its resolved value and behaves as one character to the caret. It is selected
whole, deleted whole, and never edited in place (`PROSE-FEAT-163`).

---

## 5. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| A band's text is an ordinary block | a command carrying the header's own edits | it must be recursive to carry a mark, and it duplicates the whole editing plane: a second editing surface with its own rules, growing a variant for every command the body already has. As an ordinary block a header inherits typing, marks, styles and the cascade with nothing written for it (`PROSE-006`) |
| The metadata holds the identity, never the text | copying the header's text into the metadata | it is a second copy of a paragraph and it merges as one blob. Holding the identity keeps the text where text lives, leaves the metadata a configuration a reader takes in at a glance, and lets a header be restyled the way any block is |
| A field stores what to show, never what it shows | writing the resolved text into the document | it desynchronizes on the first edit and puts a page number in the merge path. Every field's value changes with the layout, the locale or the clock, so a stored value is stale the moment anything moves and two peers store two of them (`PROSE-FEAT-065`) |
| The date comes from the document's stamp | resolving from the reader's clock | it renders a different date per peer and makes the displayed document diverge. A wall clock is per reader and untrusted, while the last-modified stamp is in the document, so both peers resolve the same value (`PROSE-FEAT-066`) |
| The date field stores a format, not a locale | storing the rendered string per locale | it is the stored-value defect one layer down. The format is the author's intent -- short, long, or the calendar form -- and the locale is the document's, so keeping them apart lets one document re-localize with no rewrite |
| One setting for both first-page blocks | a setting on each side | it doubles the states and makes the common gesture two gestures, producing the two nobody asks for: a first-page header with the ordinary footer, and the reverse. One flag is the checkbox every word processor shows, and one thing to converge (`PROSE-FEAT-063`) |
| The first page's blocks are kept while the setting is off | deleting the two blocks when the setting goes off | it loses the first page's text on a switch that reads as a preference, and the author cannot undo it by turning the setting back on. Kept, the switch is lossless in both directions (`PROSE-FEAT-064`) |
| A field insertion into an inert block is refused | writing it anyway | it is a silent no-op the author repeats until they give up: the field lands where nothing draws it, so they see no result and have no way to learn why. Refusing lets the chrome answer at the one moment they can act (`PROSE-FEAT-067`) |
| The field insertion names its target | a position alone | it is the ambiguity that made the first-page design unbuildable: with the first-page blocks on, a position cannot say which of the two blocks a field lands in, and the two are edited from the same band on screen |
| A position past the end is clamped | refusing the insertion | it turns a normal click into an error, and it would fail a click at the end of a short header. The end is the only correct place left, and the insertion is a gesture rather than a document rule |

---

## 6. NOT BUILT, AND NOT YET RULED

* **A refusal travels, and nothing shows it.** `PROSE-FEAT-067` requires the refusal to be
  reported. The engine names the command and the reason
  (`features/ambre-spec-prose-engine.md section 3`), and no band command exists to be refused yet.
  Nor does a view that would show one. It lands with the
  `Headers, footers, find and replace` Story (`ambre-roadmap.md` E5).
* **A field in a hidden side is not refused.** `PROSE-FEAT-067` refuses a field in the first
  page's own block while the setting is off, because nothing would draw it. A side whose own
  setting is off is invisible for the same reason. `PROSE-FEAT-165` writes the mark there
  without a word. The two inert blocks are told apart by which flag turned them off.
* **Nothing rules on two peers moving one flag.** The three settings are metadata a peer writes
  without reading the others. No row says what the document holds when one peer hides a side
  while the other turns the first page's bands on for it.
* **Nothing rules on a header per section.** The document has one header and one footer, plus the
  first page's pair; a header per section is named as a design nobody has written.
* **Nothing rules on odd and even pages.** A document printed on both sides wants a mirrored band,
  and the four bands above cannot express one.
* **A field of the body is not ruled on.** The four fields are described in a band. Nothing
  says whether a page number may be placed in the body text. Its resolution there would be the
  same, and its behaviour under editing would not.
* **The title field reads a metadata key nothing writes.** The document title is a metadata key
  (`ambre-spec-prose-schema.md section 3`), and no command in the corpus sets it.
* **Nothing rules on a band taller than its cap.** Pagination clips it (`PROSE-016`); no rule says
  what the author sees when their header is clipped.
