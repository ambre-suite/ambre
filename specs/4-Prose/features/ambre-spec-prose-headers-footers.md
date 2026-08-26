# AMBRE -- FEATURE: HEADERS AND FOOTERS

**Type:** Vertical (feature)
**Implementation:** design-only

> A band at the top of every page and a band at the bottom, each holding ordinary text. A page
> number, a page count, a date or the title may be placed in either one, and none of them is
> written down: they are resolved when the page is drawn.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`.

| ID | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|
| `PROSE-006` | A header and a footer MUST be ordinary blocks addressed by a stable identity; no command MUST carry their edits inside another command. | this spec section 3; `ambre-spec-prose-schema.md section 3` | not built -- lands with the "Headers, footers, find and replace" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-063` | One document-level setting MUST govern the first page's header and its footer together. | this spec section 3 | not built -- lands with the "Headers, footers, find and replace" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-064` | While that setting is off, the first page's own blocks MUST be kept and MUST NOT be drawn. | this spec section 3 | not built -- lands with the "Headers, footers, find and replace" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-065` | A field MUST store no rendered value in the document: it MUST store what to show, never what it shows today. | this spec section 3 | not built -- lands with the "Headers, footers, find and replace" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-066` | The date field MUST be resolved from the document's own last-modified stamp, never from the reader's clock. | this spec section 3 | not built -- lands with the "Headers, footers, find and replace" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-067` | Inserting a field into the first page's own block MUST be refused while the setting is off. | this spec section 3 | not built -- lands with the "Headers, footers, find and replace" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-068` | A field position past the end of its target MUST be clamped to that end. | this spec section 3 | not built -- lands with the "Headers, footers, find and replace" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-UI-047` | A field MUST be resolved when the page is drawn, from the page it is drawn on. | this spec section 4 | not built -- lands with the "Headers, footers, find and replace" Story (`ambre-roadmap.md` E5) | unimplemented |

* **Budget.** none of its own: a band is measured once per document and subtracted from every page,
  which is the second step of `features/ambre-spec-prose-pagination.md section 3` and is charged to
  the pagination rows of `ambre-spec-budgets.md section 1`.
* **Boundary.** none of its own: a header is a block like any other, and `ZK-001` seals every
  payload before it leaves the client.
* **Verification.** 8 `unimplemented`, nothing else: `crates/ambre-prose` is a `lib.rs`.
  `PROSE-006` is compiler-held the day the command enum exists, because a nested content command is
  a variant that is not there. `PROSE-FEAT-065` and `PROSE-FEAT-066` are the two a two-peer test
  reaches without a page: two peers whose clocks differ must render one date.

---

## 2. WHAT EACH USER SEES

Two peers over one band. No test resolves yet: the last column names the row each row rests on.

| Alice does | Bob does | The document holds | Each user sees | Test |
|---|---|---|---|---|
| types in the header | types in the header at the same moment | both sentences, merged as in any paragraph | the same header on both screens | none -- `PROSE-006` |
| inserts a page number in the footer | -- | the field, and no number | page one on page one, page two on page two | none -- `PROSE-UI-047` |
| inserts a date field | reads the document in another time zone | the field and the format | the same date on both screens | none -- `PROSE-FEAT-066` |
| turns the first-page setting on | -- | the setting | the first page taking its own header, the rest unchanged | none -- `PROSE-FEAT-063` |
| turns it off again | -- | the setting, and the first page's blocks | the ordinary header everywhere, and the first page's text kept for later | none -- `PROSE-FEAT-064` |
| types a page of text | -- | the text | the page count in the footer one higher, on both screens | none -- `PROSE-UI-047` |

---

## 3. MODEL

**Containers.** The document metadata carries the configuration of four bands -- the header, the
footer, and the first page's own header and footer -- and the document-level setting that turns the
last two on. Each configuration holds whether the side is shown, the identity of the block that
holds its text, and its alignment. All of them are declared in
`ambre-spec-prose-schema.md section 3`.

**The text is a block, not a field.** The metadata holds the block's identity; the text lives in
the block tree like any other text (`PROSE-006`). Every ordinary editing command therefore works on
a header by naming that block: typing, marks, paragraph styles, the cascade. There is no second
editing surface and no command that carries another command.

**The four fields.** A field is an inline mark carrying no rendered text
(`ambre-spec-prose-schema.md section 3`): the page number, the page count, the last-modified date
and the document title. The mark's key is the field, and adjacent typing never joins it, so a field
is one atom to the caret.

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
language; a reader's wall clock is never read, because two readers would then render two dates for
one document (`PROSE-FEAT-066`). The short and the long forms follow the document's language; the
calendar form is the same everywhere.

**The first page.** One setting, on the document and not on a side, turns both first-page blocks on
at once (`PROSE-FEAT-063`). While it is off, the two blocks are kept and never drawn, so turning it
on again brings back what was written there (`PROSE-FEAT-064`). Pagination reads this one setting
to decide, on the first page alone, which of the two blocks a side draws; whether a side is drawn
at all is that side's own setting.

**The commands** (`ambre-bus`, declared in `features/ambre-spec-prose-engine.md section 3`):

| Shape | Behaviour |
|---|---|
| `SetHeaderEnabled { enabled }` | shows or hides the top band |
| `SetFooterEnabled { enabled }` | shows or hides the bottom band |
| `SetDifferentFirstPage { enabled }` | turns both first-page blocks on or off |
| `InsertHeaderField { field, target, pos }` | a field in the header block the target names |
| `InsertFooterField { field, target, pos }` | a field in the footer block the target names |

Editing the text of a band uses the ordinary text commands against the block the metadata names.

**What a field insertion does, in order:**

1. resolve the target: the regular block, or the first page's own.
2. refuse the first page's block while the setting is off (`PROSE-FEAT-067`). It has no surface to
   appear on, so the insertion would be invisible and unexplained.
3. clamp a position past the end of the target's text to that end (`PROSE-FEAT-068`).
4. write the field's mark at that position.

**Where a band is measured.** The two bands are measured once against the content width, and the
first-page variants beside them, before any page is placed; a band is capped at a quarter of the
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
| the alignment control while the caret is in a band | left, centre or right for that band |
| Escape, or a click in the body | returns the caret to the document |

A field is drawn as its resolved value and behaves as one character to the caret: it is selected
whole, deleted whole, and never edited in place.

---

## 5. DECISIONS

| Decision | Rationale | Rejected alternative |
|---|---|---|
| A band's text is an ordinary block | every editing command already addresses a block, so a header inherits typing, marks, styles and the cascade without one line written for it. The alternative is a second editing surface with its own rules, and it has to grow a variant for every command the body already has (`PROSE-006`) | a command carrying the header's own edits, which must be recursive to carry a mark and duplicates the whole editing plane |
| The metadata holds the identity, never the text | the text is then in the one place text lives, and the metadata stays a configuration a reader can take in at a glance. It also means a header survives being restyled the way any block does | copying the header's text into the metadata, which is a second copy of a paragraph and merges as one blob |
| A field stores what to show, never what it shows | the value of every field changes with the layout, the locale or the clock, so a stored value is stale the moment anything moves and two peers would store two of them. Storing the intent makes the field convergent by construction (`PROSE-FEAT-065`) | writing the resolved text into the document, which desynchronizes on the first edit and puts a page number in the merge path |
| The date comes from the document's stamp | the field must read the same on every screen, and a wall clock is per reader and untrusted. The last-modified stamp is in the document, so both peers resolve the same value (`PROSE-FEAT-066`) | resolving from the reader's clock, which renders a different date per peer and makes the displayed document diverge |
| The date field stores a format, not a locale | the format is the author's intent -- short, long, or the calendar form -- and the locale is the document's. Keeping them apart lets one document re-localize with no rewrite | storing the rendered string per locale, which is the stored-value defect one layer down |
| One setting for both first-page blocks | it is the one checkbox every word processor shows, and a per-side setting produces the two states nobody asks for: a first-page header with the ordinary footer, and the reverse. One flag is one thing to explain and one thing to converge (`PROSE-FEAT-063`) | a setting on each side, which doubles the states and makes the common gesture two gestures |
| The first page's blocks are kept while the setting is off | turning the setting off is a display decision, and destroying the text would make it a deletion the author cannot undo by turning it back on. Keeping them makes the switch lossless in both directions (`PROSE-FEAT-064`) | deleting the two blocks when the setting goes off, which loses the first page's text on a switch that reads as a preference |
| A field insertion into an inert block is refused | the field would be written where nothing draws it, so the author would see no result and have no way to learn why. Refusing lets the chrome answer at the one moment they can act (`PROSE-FEAT-067`) | writing it anyway, which is a silent no-op the author repeats until they give up |
| The field insertion names its target | with the first-page blocks on, a position alone cannot say which of the two blocks a field lands in, and the two are edited from the same band on screen | a position alone, which is the ambiguity that made the first-page design unbuildable |
| A position past the end is clamped | the end is the only correct place left, and the insertion is a gesture rather than a document rule. Refusing would fail a click at the end of a short header | refusing the insertion, which turns a normal click into an error |

---

## 6. NOT BUILT, AND NOT YET RULED

* **The refusal has no event.** `PROSE-FEAT-067` requires the refusal to be reported, and the
  engine declares one untyped error carrying a code and a message
  (`features/ambre-spec-prose-engine.md section 3`). The lists, the styles and the tables features report
  through the same gap.
* **Nothing rules on a header per section.** The document has one header and one footer, plus the
  first page's pair; a header per section is named as a design nobody has written.
* **Nothing rules on odd and even pages.** A document printed on both sides wants a mirrored band,
  and the four bands above cannot express one.
* **A field of the body is not ruled on.** The four fields are described in a band, and nothing
  says whether a page number may be placed in the body text, where its resolution would be the
  same and its behaviour under editing would not.
* **The title field reads a metadata key nothing writes.** The document title is a metadata key
  (`ambre-spec-prose-schema.md section 3`), and no command in the corpus sets it.
* **Nothing rules on a band taller than its cap.** Pagination clips it (`PROSE-016`); no rule says
  what the author sees when their header is clipped.
