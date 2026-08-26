# AMBRE -- FEATURE: THE RULER

**Type:** Vertical (feature)
**Implementation:** design-only

> A horizontal scale above the page, carrying three handles: where the first line starts, where the
> rest of the paragraph starts, and where every line ends. Drag one and the paragraph moves with it.

The three values it writes are the paragraph's, and they belong to
`features/ambre-spec-prose-inline-styles.md`. The page width it is drawn against belongs to
`features/ambre-spec-prose-pagination.md`.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`.

| ID | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|
| `PROSE-UI-063` | A drag MUST write the paragraph once, when the handle is released; it MUST NOT write while the handle moves. | this spec section 3 | not built -- lands with the "Ruler, clipboard and inertial scroll" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-UI-064` | The handles MUST show the indents of the block the caret is in, and MUST follow the caret to another block. | this spec section 3 | not built -- lands with the "Ruler, clipboard and inertial scroll" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-UI-065` | The scale MUST stay aligned with the page's content width, at any zoom and at any horizontal offset of the page column. | this spec section 3 | not built -- lands with the "Ruler, clipboard and inertial scroll" Story (`ambre-roadmap.md` E5) | unimplemented |

* **Budget.** none of its own. A drag paints the handle and nothing else until release, which is
  the point of `PROSE-UI-063`: the frame it sits inside is the row of
  `ambre-spec-budgets.md section 2`, and no document write happens during the gesture.
* **Boundary.** none: the ruler reads view state and emits one command on release. It holds no key
  and opens no socket (`PROSE-UI-002`).
* **Verification.** 3 `unimplemented`, nothing else: `crates/ambre-prose` is a `lib.rs`.
  `PROSE-UI-063` is the one with an oracle a host test can hold: a drag of any length produces one
  command, and counting them needs no browser. The two others are placement under a live zoom and
  land in the wasm tier of `ambre-spec-testing.md section 6`.

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
not scroll with the page; the canvas view is what knows where the page column sits and how wide it
is, and it publishes that (`ambre-spec-ui.md section 2`).

```rust
// ambre-prose/src/ui/ruler.rs -- published to the chrome component, never drawn on the canvas

pub struct RulerState {
    pub visible:          bool,
    /// Where the page's left margin falls on screen, and how wide the content column is.
    pub page_left_px:     f32,
    pub content_width_px: f32,
    /// The three values of the block the caret is in, in points.
    pub indent_first:     f32,
    pub indent_left:      f32,
    pub indent_right:     f32,
    pub dragging:         Option<RulerHandle>,
}

pub enum RulerHandle { IndentFirst, IndentLeft, IndentRight }
```

**Steps, for one drag:**

1. a press on a handle records which one, and the ruler paints the handle at the pointer from then
   on; the document is untouched (`PROSE-UI-063`);
2. the release converts the handle's position into points, against the page's content width
   (`PROSE-UI-065`);
3. the view reads the caret block's current settings, replaces the one value that moved, and sends
   one `SetParagraphStyle` for the whole set;
4. the handler writes only the property that differs (`PROSE-FEAT-025`), so a concurrent change to
   another property of the same paragraph survives.

**One command per gesture.** Writing on release makes a drag one operation on the wire and one
undo step (`features/ambre-spec-prose-undo.md`).

**The handles follow the caret**, and read the block's effective settings through the cascade
rather than its stored keys, so a paragraph inheriting its indents from a style shows the values it
is laid out with (`PROSE-FEAT-020`, `PROSE-UI-064`).

**The scale is the page's.** Zero is the page's left margin and the length is the content width,
both from the layout, so a zoom moves the ruler with the page (`PROSE-UI-065`,
`features/ambre-spec-prose-viewport.md`).

**Code:** none. `crates/ambre-prose/src/lib.rs` is seven lines.

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

## 5. DECISIONS

| Decision | Rationale | Rejected alternative |
|---|---|---|
| The write happens on release | a drag is one intent and deserves one operation. Writing per pointer event puts hundreds of deltas on the wire for one gesture, and leaves an undo stack no author can walk back through | writing on every pointer move, which is the shape that floods the document and the network for a gesture that lasted a second |
| The ruler is DOM chrome, fixed above the page | the canvas owns content and the DOM owns everything else (`UI-THREAD-001`), and a band drawn on the canvas would scroll with the page it is measuring. Fixing it in the chrome is what keeps it visible while the author reads page four | drawing the ruler as a `DrawSurface` band inside the scrolled column |
| It shows the effective values, not the stored ones | a paragraph inheriting its indents from a style has no stored indent, and showing zero there tells the author the opposite of what the page shows. The cascade already answers the question | reading the block's own keys, which shows nothing for every paragraph that uses a style |
| It shows one paragraph | the handles are three numbers and a selection can span two hundred paragraphs with two hundred different triples. Following the caret is the one rule that always has an answer | showing the selection's common values, which is blank for most real selections and needs a fourth, mixed rendering |

---

## 6. NOT BUILT, AND NOT YET RULED

* **Nothing exists.** `crates/ambre-prose/src/lib.rs` is seven lines, so the ruler, its handles and
  its scale are a design, and every row above is `unimplemented`.
* **Nothing bounds a handle.** No rule says that the left indent cannot pass the right one, or that
  either may leave the content column, so a drag can produce a paragraph whose content width is
  zero or negative. Pagination has no rule for that block either
  (`features/ambre-spec-prose-pagination.md`).
* **The drag over a multi-block selection is unruled.** The handles show the caret's block, and a
  drag writes one `SetParagraphStyle` addressed to one block. Whether a selection spanning ten
  paragraphs indents ten paragraphs or one is undecided, and it is what an author expects to
  differ.
* **The ruler carries no tab stops and no margin handles.** It writes three of the nine paragraph
  properties; the page margins, which the same scale displays in every comparable editor, are
  `meta/page` and reach no gesture at all (`ambre-spec-prose-schema.md section 3`).
* **The unit is not chosen.** The stored values are points, the page format is millimetres, and no
  document says what the scale is graduated in or what an author's locale changes about it.
* **No gesture shows or hides it.** `RulerState` carries a `visible` flag and nothing sets it: the
  application declares no view menu.
