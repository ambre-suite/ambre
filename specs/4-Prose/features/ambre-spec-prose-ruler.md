# AMBRE -- FEATURE: THE RULER

**Type:** Vertical (feature)
**Implementation:** partial

> A horizontal scale above the page, carrying three handles. Where the first line starts, where the
> rest of the paragraph starts, and where every line ends. Drag one and the paragraph moves with it.

The three values it writes are the paragraph's, and they belong to
`features/ambre-spec-prose-inline-styles.md`. The page width it is drawn against belongs to
`features/ambre-spec-prose-pagination.md`.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-UI-064` | `OBS` | The handles MUST show the indents of the block the caret is in, whichever block that is. | this spec section 3 | `ambre_prose::view::ruler::tests::the_handles_show_the_caret_blocks_laid_out_indents` | none | verified |
| `PROSE-UI-075` | `OBS` | The values the handles show MUST be the ones the page is laid out with, not the block's own stored keys. | this spec section 3; this spec section 5 | `ambre_prose::view::ruler::tests::the_handles_show_the_caret_blocks_laid_out_indents` | none | verified |
| `PROSE-UI-065` | `OBS` | The scale MUST stay aligned with the page's content width, at any zoom and at any horizontal offset of the page column. | this spec section 3 | `ambre_prose::view::ruler::tests::the_scale_follows_the_column_at_any_zoom` -- the canvas coordinates | partial -- the conversion to CSS pixels and the element's placement are browser state, and no tier drives the page; lands with the `The UI-thread belts` Story (`ambre-roadmap.md` E9) | untested |

### 1.2 Drag a handle

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-UI-230` | `POST` | Releasing the first-line handle MUST write the first-line indent alone, counted from the left indent. | this spec section 3 | `ambre_prose::view::ruler::tests::the_first_line_handle_writes_the_first_line_indent_alone` | none | verified |
| `PROSE-UI-063` | `POST` | Releasing a handle MUST write the paragraph once, whatever the length of the drag. | this spec section 3 | `ambre_prose::view::ruler::tests::a_drag_of_any_length_writes_the_paragraph_once` | none | verified |
| `PROSE-UI-076` | `FRAME` | A handle in motion MUST NOT touch the document: the paint follows the pointer and nothing is written until the release. | this spec section 3 | `ambre_prose::view::ruler::tests::a_drag_of_any_length_writes_the_paragraph_once` | none | verified |
| `PROSE-UI-228` | `POST` | A released handle MUST stay inside the content column. | this spec section 3 | `ambre_prose::view::ruler::tests::a_handle_stays_inside_the_column_and_short_of_the_other` | none | verified |
| `PROSE-UI-229` | `POST` | A released handle MUST leave at least 36 points between the two indents. | this spec section 3 | `ambre_prose::view::ruler::tests::a_handle_stays_inside_the_column_and_short_of_the_other` | none | verified |

* **Budget.** none of its own. A drag paints the handle and nothing else until release, the point
  of `PROSE-UI-063`. The frame it sits inside is the row of `ambre-spec-budgets.md section 2`, and
  no document write happens during the gesture.
* **Boundary.** none: the ruler reads view state and emits one command on release. It holds no key
  and opens no socket (`PROSE-UI-002`).
* **Verification.** 7 `verified`, 1 `untested`. Host tests hold every row on the view's state:
  a drag of any length yields one command on release, and the motion writes nothing. The element
  has no command channel, so it cannot write during the gesture. `PROSE-UI-065` stays open in CSS
  pixels, a placement under a live zoom that lands in the wasm tier
  (`ambre-spec-testing.md section 6`).

---

## 2. WHAT EACH USER SEES

The ruler is local; the indents it writes are the document's. No test resolves yet: the last column
names the row each scenario rests on.

| Alice does | Bob does | The document holds | Each user sees | Test |
|---|---|---|---|---|
| drags the left indent across the page | watches | nothing, until she releases | Alice's handle moving; Bob's paragraph unchanged | none -- `PROSE-UI-063` |
| releases it | watches | one left indent | the paragraph indented on both screens | none -- `PROSE-UI-063` |
| sets the left indent | sets the spacing of the same paragraph | both properties, neither lost | Alice's indent and Bob's spacing | none -- `PROSE-FEAT-025` |
| sets the left indent | sets the left indent of the same paragraph | one of the two values | the same paragraph on both screens | none -- `PROSE-FEAT-025` |
| moves the caret into a quote | watches | nothing: a caret is local | Alice's handles at the quote's indents | none -- `PROSE-UI-064` |

---

## 3. MODEL

**Containers.** This feature owns none. It writes three of the nine paragraph properties, through
the command of `features/ambre-spec-prose-inline-styles.md`.

**The state the view publishes.** The ruler is a DOM element fixed to the editor chrome, so it does
not scroll with the page. The canvas view is what knows where the page column sits and how wide it
is. It publishes that (`ambre-spec-ui.md section 2`).

```rust
// ambre-prose/src/view/ruler.rs -- published to the chrome component, never drawn on the canvas

pub struct RulerState {
    pub visible:          bool,
    /// Where the ruler sits, where the content column starts and how wide it is, on screen.
    pub top_px:           f32,
    pub page_left_px:     f32,
    pub content_width_px: f32,
    /// How many screen pixels one point takes, at the zoom in force.
    pub px_per_pt:        f32,
    /// The indents of the block the caret is in, as laid out, in points.
    pub indent_first:     f32,
    pub indent_left:      f32,
    pub indent_right:     f32,
}

pub enum RulerHandle { IndentFirst, IndentLeft, IndentRight }
```

The view publishes canvas pixels and the host converts them to CSS pixels. The element holds the
drag, a `RulerDrag`, and hands back a `RulerRelease`: the handle and the indent in points, so no
pixel crosses back.

**Steps, for one drag:**

1. a press on a handle records which one, and the element paints it at the pointer. The element
   holds no channel, so the document is untouched (`PROSE-UI-076`);
2. the release converts the handle's position into whole points, against the content column, kept
   inside it and short of the other handle (`PROSE-UI-065`, `PROSE-UI-228`, `PROSE-UI-229`);
3. the view sends one `SetParagraphStyle` carrying the one property that moved, or nothing when it
   did not. An absent property is left alone. A concurrent change to another one survives,
   and an inherited value stays inherited (`PROSE-FEAT-025`).

**One command per gesture.** Writing on release makes a drag one operation on the wire and one
undo step (`features/ambre-spec-prose-undo.md`).

**The handles follow the caret** (`PROSE-UI-064`), and read the block's effective settings through
the cascade rather than its stored keys. A paragraph inheriting its indents from a style shows
the values it is laid out with (`PROSE-FEAT-020`, `PROSE-UI-075`).

**The scale is the page's.** Zero is the page's left margin and the length is the content width,
both from the layout. A zoom moves the ruler with the page (`PROSE-UI-065`,
`features/ambre-spec-prose-viewport.md`).

**Code:** `crates/ambre-prose/src/view/ruler.rs`, the element `ProseRuler` in
`crates/ambre-prose/src/chrome/mod.rs`, and the host's `take_ruler` and `publish_ruler`.

---

## 4. SURFACE

| Trigger | What happens |
|---|---|
| a press on a handle | the handle follows the pointer, and nothing is written |
| the release | the paragraph's indent is set, in one operation |
| a caret move to another block | the three handles jump to that block's values |
| a zoom, or a horizontal scroll | the scale and the handles move with the page column |

The element, its keyboard reachability and its pointer capture are the shared chrome toolkit's, and
are not respecified here (`ambre-spec-chrome.md`).

---

## 5. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| The write happens on release | writing on every pointer move | it floods the document and the network for a gesture that lasted a second: hundreds of deltas on the wire, and an undo stack no author can walk back through. A drag is one intent and deserves one operation |
| The ruler is DOM chrome, fixed above the page | drawing the ruler as a `DrawSurface` band inside the scrolled column | the canvas owns content and the DOM owns everything else (`UI-THREAD-001`), and a band on the canvas scrolls with the page it is measuring. Fixed in the chrome, it stays visible while the author reads page four |
| It shows the effective values, not the stored ones | reading the block's own keys | it shows nothing for every paragraph that uses a style. A paragraph inheriting its indents has no stored indent, so zero tells the author the opposite of what the page shows, and the cascade already answers the question |
| It shows one paragraph | showing the selection's common values | it is blank for most real selections and needs a fourth, mixed rendering. The handles are a few numbers and a selection can span two hundred paragraphs with two hundred different triples, so following the caret is the one rule that always has an answer |

---

## 6. NOT BUILT, AND NOT YET RULED

* **The first-line handle stops at the left indent.** A hanging indent is laid out as none
  (`PROSE-FEAT-310`), so the handle shows zero for it and cannot drag past the left indent. It
  lands with the `Hanging indents` Story (`ambre-roadmap.md` E5).
* **A block outside the page column shows no handles.** A table cell, a header, a footer and a
  note are measured against another width than the content column. The ruler hides its handles
  there.
* **The drag over a multi-block selection is unruled.** The handles show the caret's block, and a
  drag writes one `SetParagraphStyle` addressed to one block. Whether a selection spanning ten
  paragraphs indents ten paragraphs or one is undecided, and it is what an author expects to
  differ.
* **The ruler carries no tab stops and no margin handles.** It writes three of the nine paragraph
  properties. The page margins, which the same scale displays in every comparable editor, are
  `meta/page` and reach no gesture at all (`ambre-spec-prose-schema.md section 3`).
* **The scale carries no graduation.** A release rounds to whole points, and the page format is in
  millimetres. No document says what the scale is graduated in, or what a locale changes about it.
  The element draws the column and its handles, and no tick.
* **No gesture shows or hides it.** The caret alone sets `visible`: the application declares no
  view menu, so an author who wants the ruler gone has no way to ask.
