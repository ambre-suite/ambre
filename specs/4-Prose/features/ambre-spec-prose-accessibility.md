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

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-UI-066` | `OBS` | The accessible surface MUST carry the text of the block the caret is in, so a screen reader reads what is being edited. | this spec section 3; `CANVAS-009` | not built | unbuilt -- lands with the `CANVAS P2 -- frame quality` Story (`ambre-roadmap.md` E6) | unimplemented |
| `PROSE-UI-077` | `OBS` | The caret of the accessible surface MUST sit at the offset the document caret sits at, so a screen reader resumes where the author is rather than at the head of the block. | this spec section 3 | not built | unbuilt -- lands with the `CANVAS P2 -- frame quality` Story (`ambre-roadmap.md` E6) | unimplemented |
| `PROSE-UI-078` | `INV` | The accessible surface MUST NOT carry the text of any block but the one the caret is in. | this spec section 3; this spec section 5 | not built | unbuilt -- lands with the `CANVAS P2 -- frame quality` Story (`ambre-roadmap.md` E6) | unimplemented |
| `PROSE-UI-069` | `INV` | The accessible surface MUST be filled from the runs and the layout the page is painted from. | this spec section 3; `PROSE-UI-001` | not built | unbuilt -- lands with the `CANVAS P2 -- frame quality` Story (`ambre-roadmap.md` E6) | unimplemented |
| `PROSE-UI-079` | `OBS` | An edit to the block the caret is in MUST reach the accessible surface, whether or not the caret moved. | this spec section 2; this spec section 4 | not built | unbuilt -- lands with the `CANVAS P2 -- frame quality` Story (`ambre-roadmap.md` E6) | unimplemented |

### 1.2 Move the caret

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-UI-067` | `POST` | After a caret move, the block it landed in MUST be announced: what kind of block it is, and the level when it is a heading. | this spec section 3 | not built | unbuilt -- lands with the `CANVAS P2 -- frame quality` Story (`ambre-roadmap.md` E6) | unimplemented |
| `PROSE-UI-068` | `POST` | A move that enters or leaves a table cell, a footnote, a comment thread or a tracked change MUST announce it. | this spec section 3 | not built | unbuilt -- lands with the `CANVAS P2 -- frame quality` Story (`ambre-roadmap.md` E6) | unimplemented |
| `PROSE-UI-080` | `FRAME` | A move that lands in the same context MUST NOT announce anything. | this spec section 3; this spec section 5 | not built | unbuilt -- lands with the `CANVAS P2 -- frame quality` Story (`ambre-roadmap.md` E6) | unimplemented |
| `PROSE-UI-081` | `FRAME` | Filling the accessible surface MUST NOT reach the document: the text written into it is not a keystroke. | derived | not built | unbuilt -- lands with the `CANVAS P2 -- frame quality` Story (`ambre-roadmap.md` E6) | unimplemented |

* **Budget.** the content accessibility diff, a row of `ambre-spec-budgets.md section 2`. It
  is billed per frame in which the announced context changes, and no bench measures it.
* **Boundary.** none: the surface carries text the Logic thread already decrypted and sent, and it
  opens no path of its own (`UI-THREAD-004`).
* **Verification.** 9 `unimplemented`, nothing else: `crates/ambre-prose` is a seven-line
  `lib.rs`. `PROSE-UI-069` carried a second half `PROSE-UI-001` already owns -- the UI thread holds
  no document data -- so the row keeps the source rule and the ban stays where it was written.
  Four rules the document turns on carried no row: the surface's own caret sits where the author's
  does (`PROSE-UI-077`), it holds one block and not the document (`PROSE-UI-078`, the alternative
  section 5 rejects), an edit reaches it with no caret move (`PROSE-UI-079`, which scenario three
  of section 2 rests on while the steps of section 3 run on a selection change alone), and a move
  inside one context announces nothing (`PROSE-UI-080`). That last one is the difference between a
  live region and one an author switches off, and an implementation speaking on every arrow key
  satisfies `PROSE-UI-067` and `PROSE-UI-068` whole. `PROSE-UI-081` is the frame condition the
  section 5 decision to reuse the input element needs, and section 6 records that
  `ambre-spec-text-input.md` contradicts it as both are written today. `PROSE-UI-067`,
  `PROSE-UI-068` and `PROSE-UI-080` are the three a host test can hold, because the announcement is
  a string built from the caret's context and needs neither a browser nor a screen reader. The
  other six need the live element and land in the wasm tier of `ambre-spec-testing.md section 6`,
  and no tier in the corpus drives an actual screen reader.

---

## 2. WHAT EACH USER SEES

Accessibility is local: what one peer's screen reader says is decided by that peer's caret. No test
resolves yet: the last column names the row each scenario rests on.

| Alice does | Bob does | The document holds | Each user sees | Test |
|---|---|---|---|---|
| moves the caret into a heading | watches | nothing: a caret is local | Alice hears the heading and its level; Bob hears nothing | none -- `PROSE-UI-067` |
| moves the caret into a table cell | edits another block | Bob's edit | Alice hears the cell; her surface still holds her own block | none -- `PROSE-UI-068` |
| reads a paragraph with the caret in it | rewrites that paragraph | Bob's text | Alice's surface holds the paragraph as it now reads | none -- `PROSE-UI-079` |
| moves the caret onto a suggested insertion | watches | the suggestion | Alice hears that the text is a pending change | none -- `PROSE-UI-068` |

---

## 3. MODEL

**Containers.** This feature owns none. It reads what the view already holds.

**Two surfaces, both already there.** The shared transparent `<textarea>` is every application's
keyboard and input pipe (`ambre-spec-text-input.md`), and it is what a screen reader
navigates: it carries the caret block's text rather than staying empty (`PROSE-UI-066`). Beside it
sits one polite live region, where a caret move is announced.

**What the surface carries.** The caret block's text, kept in step with the runs the page is
painted from (`PROSE-UI-066`) -- not the whole document (`PROSE-UI-078`).

**Steps, on a selection change:**

1. the view resolves the caret's block and its ancestors from the layout it already holds;
2. it replaces the surface's text with that block's text, which is a write into the pipe and not a
   keystroke (`PROSE-UI-081`), and places the surface's own caret at the matching offset
   (`PROSE-UI-066`, `PROSE-UI-077`);
3. it builds the context sentence: the block's kind, its heading level when it has one, and the
   structures the caret entered or left since the last move (`PROSE-UI-067`, `PROSE-UI-068`);
4. it writes that sentence into the live region, and writes nothing when the context is unchanged
   (`PROSE-UI-080`).

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
layout the canvas paints from (`PROSE-UI-069`), so neither can hold text the page does not show. An
edit to the caret's block therefore reaches the surface on the frame that paints it, with no caret
move behind it (`PROSE-UI-079`); the steps above run on a selection change and are not that path.
Prose's announcement is the same act as Trame's active-cell announcement
(`features/ambre-spec-trame-selection.md`); the two differ in what they name, not in where the
sentence comes from.

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
| a remote edit inside the caret's block | the accessible text follows it (`PROSE-UI-079`) |
| a caret move inside one block | the accessible caret moves (`PROSE-UI-077`); nothing is announced (`PROSE-UI-080`) |
| a screen reader's own navigation | reads and moves inside the caret block's text |

There is no control: nothing here is switched on or off by the author.

---

## 5. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| The accessible surface is the input element already over the canvas | a separate hidden mirror beside the input element | it puts two focusable surfaces over one caret. The input element already holds editing focus, focus is what a screen reader follows, and the two surfaces would disagree about where the caret is |
| It carries one block, not the document | mirroring the whole document | it trades the frame budget for text nobody is reading: a hundred pages in one element is a DOM the browser rebuilds on every keystroke, and the largest thing on the UI thread. The caret is what a reader follows, and one block is a string |
| Context is announced, not read from the text | leaving the structure implicit | it is what a canvas already does, and what this feature exists to undo. "Heading 2" is not in the document, it is what the block is, and a reader who only hears the characters cannot tell a heading from a paragraph |
| Nothing is announced when the context is unchanged | announcing on every selection change | a live region that speaks on every caret move interrupts the reader on every arrow key, which makes the region worse than silence and is the shape that makes people switch the feature off |

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
* **The pipe cannot hold the block's text as `ambre-spec-text-input.md` is written.** The decision
  above reuses the shared input element, and that document empties the pipe after every commit
  (`CHROME-017`) and forwards whatever it holds as one committed run (`CHROME-061`). Text put there
  for a screen reader is then the block repeated into the document on the next keystroke, which is
  the defect the clear exists to prevent. `PROSE-UI-081` states what must hold; which of the two
  mechanisms gives way is undecided, and the two documents were written apart.
* **The other two applications have no content surface at all.** Trame announces its active cell
  (`features/ambre-spec-trame-selection.md`) and Toile announces nothing, so the mandate
  `CANVAS-009` states for every application is designed for one of the three. This document is not
  the horizontal half: it would be a second copy of a rule Foundations already owns.
* **Nothing drives a screen reader.** `ambre-spec-testing.md` has no tier that reads a live region
  or walks an accessibility tree, so the three rows a host test can hold are the announcement's
  text and its silence, never that anything says either.
* **The reading order of a page is not the document's order.** A header, a footer and a footnote
  band are laid out around the body, and no rule says where they fall for a reader who is walking
  the document rather than looking at the page.
* **A picture's alternative text reaches no surface.** The image node carries `alt`
  (`ambre-spec-prose-schema.md section 3`) and no announcement names it, so a picture is silent
  even though the document holds its description.
