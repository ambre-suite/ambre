# AMBRE -- FEATURE: CONTENT ACCESSIBILITY

**Type:** Vertical (feature)
**Implementation:** design-only

> A screen reader must be able to read the document and follow the caret through it. The page is
> painted on a canvas, which tells a screen reader nothing, so the text has a second surface that
> does.

The mandate is Foundations' and applies to every application (`CANVAS-009`,
`ambre-spec-accessibility.md`). What is Prose's own, and what this document holds, is which
text the surface carries and what a caret move announces.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`.

| ID | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|
| `PROSE-UI-066` | The accessible surface MUST carry the text of the block the caret is in, so a screen reader reads and navigates what is being edited. | this spec section 3; `CANVAS-009` | not built -- lands with `CANVAS P2 -- frame quality` | unimplemented |
| `PROSE-UI-067` | A caret move MUST announce the block it lands in: what kind of block it is, and the level when it is a heading. | this spec section 3 | not built -- lands with `CANVAS P2 -- frame quality` | unimplemented |
| `PROSE-UI-068` | Entering or leaving a table cell, a footnote, a comment thread or a tracked change MUST be announced. | this spec section 3 | not built -- lands with `CANVAS P2 -- frame quality` | unimplemented |
| `PROSE-UI-069` | The accessible surface MUST be filled from the state the page is painted from; it MUST NOT be a second model of the document. | this spec section 3; `PROSE-UI-001` | not built -- lands with `CANVAS P2 -- frame quality` | unimplemented |

* **Budget.** the content accessibility diff, a row of `ambre-spec-budgets.md section 2`. It
  is billed per frame in which the announced context changes, and no bench measures it.
* **Boundary.** none: the surface carries text the Logic thread already decrypted and sent, and it
  opens no path of its own (`UI-THREAD-004`).
* **Verification.** 4 `unimplemented`, nothing else: `crates/ambre-prose` is a `lib.rs`.
  `PROSE-UI-067` and `PROSE-UI-068` are the two a host test can hold, because the announcement is a
  string built from the caret's context and needs neither a browser nor a screen reader.
  `PROSE-UI-066` and `PROSE-UI-069` need the live element and land in the wasm tier of
  `ambre-spec-testing.md section 6`. No tier in the corpus drives an actual screen reader.

---

## 2. WHAT EACH USER SEES

Accessibility is local: what one peer's screen reader says is decided by that peer's caret. No test
resolves yet: the last column names the row each scenario rests on.

| Alice does | Bob does | The document holds | Each user sees | Test |
|---|---|---|---|---|
| moves the caret into a heading | watches | nothing: a caret is local | Alice hears the heading and its level; Bob hears nothing | none -- `PROSE-UI-067` |
| moves the caret into a table cell | edits another block | Bob's edit | Alice hears the cell; her surface still holds her own block | none -- `PROSE-UI-068` |
| reads a paragraph with the caret in it | rewrites that paragraph | Bob's text | Alice's surface holds the paragraph as it now reads | none -- `PROSE-UI-069` |
| moves the caret onto a suggested insertion | watches | the suggestion | Alice hears that the text is a pending change | none -- `PROSE-UI-068` |

---

## 3. MODEL

**Containers.** This feature owns none. It reads what the view already holds.

**Two surfaces, both already there.** The shared transparent `<textarea>` is every application's
keyboard and input pipe (`ambre-spec-text-input.md`), and it is what a screen reader
navigates: it carries the caret block's text rather than staying empty (`PROSE-UI-066`). Beside it
sits one polite live region, where a caret move is announced.

**What the surface carries.** The caret block's text, kept in step with the runs the page is
painted from -- not the whole document (`PROSE-UI-066`).

**Steps, on a selection change:**

1. the view resolves the caret's block and its ancestors from the layout it already holds;
2. it replaces the surface's text with that block's text, and places the surface's own caret at the
   matching offset (`PROSE-UI-066`, `PROSE-UI-069`);
3. it builds the context sentence: the block's kind, its heading level when it has one, and the
   structures the caret entered or left since the last move (`PROSE-UI-067`, `PROSE-UI-068`);
4. it writes that sentence into the live region, and writes nothing when the context is unchanged.

**What a context sentence names:**

| The caret is | Announced |
|---|---|
| in a paragraph, a list item, a quote or a code block | the kind of block |
| in a heading | the kind, and the level |
| entering or leaving a table cell | the cell, by its row and column |
| entering or leaving a footnote | the footnote |
| entering or leaving a comment thread | the thread, and whether it is resolved |
| on a suggested insertion or deletion | the kind of change, and that it is pending |

**One source, two renderings.** The surface and the live region are filled from the runs and the
layout the canvas paints from, so a remote edit reaches both at once and neither can hold text the
page does not show (`PROSE-UI-069`). Prose's announcement is the same act as Trame's active-cell
announcement (`features/ambre-spec-trame-selection.md`); the two differ in what they name, not in
where the sentence comes from.

**Chrome is not this document's.** The toolbar, the slash menu, the ruler and the comment panel are
DOM elements and are accessible as DOM elements, through the shared toolkit
(`ambre-spec-accessibility.md`). Only the content the canvas owns needs a surface built for
it.

**Code:** none. `crates/ambre-prose/src/lib.rs` is seven lines.

---

## 4. SURFACE

| Trigger | What happens |
|---|---|
| a caret move | the accessible text becomes the new block's, and the context is announced |
| a remote edit inside the caret's block | the accessible text follows it |
| a caret move inside one block | the accessible caret moves; nothing is announced |
| a screen reader's own navigation | reads and moves inside the caret block's text |

There is no control: nothing here is switched on or off by the author.

---

## 5. DECISIONS

| Decision | Rationale | Rejected alternative |
|---|---|---|
| The accessible surface is the input element already over the canvas | it already holds editing focus, and focus is what a screen reader follows. A second element would compete for that focus and the two would disagree about where the caret is | adding a separate hidden mirror beside the input element, which puts two focusable surfaces over one caret |
| It carries one block, not the document | the caret is what a reader is following, and one block is a string. A hundred pages of text in one element is a DOM the browser rebuilds on every keystroke, and it would be the largest thing on the UI thread | mirroring the whole document, which trades the frame budget for text nobody is reading |
| Context is announced, not read from the text | "Heading 2" is not in the document; it is what the block is. A reader who only hears the characters cannot tell a heading from a paragraph, which is most of what a document's structure is | leaving the structure implicit, which is what a canvas already does and what this feature exists to undo |
| Nothing is announced when the context is unchanged | a live region that speaks on every caret move interrupts the reader on every arrow key, which makes the region worse than silence | announcing on every selection change, which is the shape that makes people switch the feature off |

---

## 6. NOT BUILT, AND NOT YET RULED

* **Nothing exists.** `crates/ambre-prose/src/lib.rs` is seven lines, so the surface, the region and
  the context sentence are a design, and every row above is `unimplemented`.
* **The mechanism is now chosen, and the budget still measures nothing.**
  `ambre-spec-accessibility.md` settles what the corpus long stated two ways: content reaches a
  screen reader through the input element and a live region, not through a platform tree, because
  the web has no platform adapter to push one to. The four rows above are written against that
  choice. What stays open is the cost: the budget row of
  `ambre-spec-budgets.md section 2` measures a diff no design has produced.
* **The other two applications have no content surface at all.** Trame announces its active cell
  (`features/ambre-spec-trame-selection.md`) and Toile announces nothing, so the mandate
  `CANVAS-009` states for every application is designed for one of the three. This document is not
  the horizontal half: it would be a second copy of a rule Foundations already owns.
* **Nothing drives a screen reader.** `ambre-spec-testing.md` has no tier that reads a live region
  or walks an accessibility tree, so the two rows a host test can hold are the announcement's text,
  never that anything says it.
* **The reading order of a page is not the document's order.** A header, a footer and a footnote
  band are laid out around the body, and no rule says where they fall for a reader who is walking
  the document rather than looking at the page.
* **A picture's alternative text reaches no surface.** The image node carries `alt`
  (`ambre-spec-prose-schema.md section 3`) and no announcement names it, so a picture is silent
  even though the document holds its description.
