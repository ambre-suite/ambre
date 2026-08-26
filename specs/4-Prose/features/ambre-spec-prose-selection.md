# AMBRE -- FEATURE: CARET AND SELECTION

**Type:** Vertical (feature)
**Implementation:** design-only

> Where the caret is, where it goes, and what a drag selects. A click lands on the character under
> it, the arrow keys walk the text the reader sees rather than the text as it is stored, and a
> selection that runs past the window keeps growing while the page scrolls.

The caret and the selection are local view state: they write nothing to the document and reach no
collaborator, so there is no two-peer table below. What a peer sees of another's caret is
`1-Foundations/ambre-spec-presence.md`.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`.

| ID | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|
| `PROSE-UI-008` | Hit-testing MUST be the exact inverse of the drawn layout, under zoom and scroll. | this spec section 2 | not built -- lands with the "Input and cursor" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-UI-009` | A click that lands in no glyph MUST resolve to the nearest valid caret position, never to no position. | this spec section 2 | not built -- lands with the "Input and cursor" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-UI-031` | Hit-testing MUST read the shaped runs alone: a block that was never painted MUST resolve like a painted one. | this spec section 2 | not built -- lands with the "Input and cursor" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-UI-032` | A block that holds no text MUST resolve to one of its two edges, chosen by the side the pointer is on. | this spec section 2 | not built -- lands with the "Input and cursor" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-UI-033` | Header, footer and footnote text MUST hit-test like body text, so the caret can enter it. | this spec section 2 | not built -- lands with the "Headers, footers, find and replace" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-UI-034` | A vertical move MUST keep the column the caret started from until a horizontal move, a click or an edit ends the run of vertical moves. | this spec section 2 | not built -- lands with the "Input and cursor" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-UI-035` | Every caret move MUST leave the caret on screen: a move that takes it out of the window MUST scroll the least that brings it back. | this spec section 2 | not built -- lands with the "Input and cursor" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-UI-036` | A caret move without extension MUST collapse the selection to the caret. | this spec section 2 | not built -- lands with the "Input and cursor" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-UI-037` | The anchor of a drag MUST be captured when the drag starts and MUST NOT move under a remote change for as long as the drag lasts. | this spec section 2 | not built -- lands with the "Input and cursor" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-UI-038` | While a drag holds past the edge of the window, each tick MUST scroll and then re-resolve the pointer, so the selection follows the text being revealed. | this spec section 2 | not built -- lands with the "Ruler, clipboard and inertial scroll" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-UI-039` | A selection MUST be valid with its end before its start, and MUST be read in document order wherever it is used. | this spec section 2 | not built -- lands with the "Input and cursor" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-UI-070` | A press inside a link, without the platform's modifier, MUST place the caret and MUST NOT open the target. | this spec section 2 | not built -- lands with the "Input and cursor" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-UI-071` | Opening a link's target MUST go through the shared open-URL affordance of the host, never a call made from the crate. | this spec section 2; `UI-THREAD-004` | not built -- lands with the "Input and cursor" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-UI-072` | A press on a document holding no block MUST place the caret in a block created through the command path, never in a locally invented one. | this spec section 2; `PROSE-027` | not built -- lands with the "Input and cursor" Story (`ambre-roadmap.md` E5) | unimplemented |

* **Budget.** the hit-test of a pointer position, a row of `ambre-spec-budgets.md section 1`.
  The autoscroll tick costs one hit-test per frame and no row of its own.
* **Boundary.** none: `PROSE-UI-010` (`features/ambre-spec-prose-engine.md section 1`) keeps the
  selection out of the document, and the UI thread reaches no network, crypto or persistence path
  (`UI-THREAD-004`).
* **Verification.** 14 `unimplemented`, nothing else: `crates/ambre-prose` is a `lib.rs`.
  `PROSE-UI-008` is the one row with an oracle already available: the layout draws a glyph at a
  position and the hit-test answers a position for a point, so the round trip through both is a
  property over the shaped runs, at every zoom and every scroll offset. `PROSE-UI-071` is
  compiler-held the day the crate exists, for the reason the clipboard's write is: the browser
  binding cannot be named from a crate that does not depend on it.

---

## 2. MODEL

**Containers.** None. Nothing here is persisted and nothing is shared.

**The caret and the selection** (`ambre-prose`, UI thread):

```rust
pub struct TextCursor {
    pub block_id: TreeID,
    pub offset:   u32,
    /// Which side of a soft wrap the caret sits on.
    pub affinity: CursorAffinity,
    /// The column a run of vertical moves keeps (`PROSE-UI-034`).
    pub x_goal:   Option<f32>,
}

pub struct TextSelection {
    /// The fixed end.
    pub anchor: (TreeID, u32),
    /// The moving end; it may precede the anchor (`PROSE-UI-039`).
    pub focus:  (TreeID, u32),
}

pub enum CursorAffinity { Before, After }
```

**From a point to a position.** The viewport answers which page a screen point is on and where
inside it (`features/ambre-spec-prose-viewport.md section 2`); the hit-test turns that into a
block and an offset. It reads the shaped runs, never a fresh layout pass, which is what makes it
answer for a culled block (`PROSE-UI-031`).

```rust
/// The block and offset under a document position. `None` only for a layout with no page.
pub fn hit_test(pos: DocPos, state: &ProseUiState) -> Option<(TreeID, u32)>;
```

| The point lands | The answer |
|---|---|
| in a glyph | that glyph's offset, from the run's own cluster data (`PROSE-UI-008`) |
| in a page margin, above the first block or below the last | the nearest block edge on that page, then the page's first or last block (`PROSE-UI-009`) |
| in a block with no text -- an image, a rule, a page break | that block's offset zero or its end, by the pointer side (`PROSE-UI-032`) |
| in a header, a footer or a footnote band | that text's own offset (`PROSE-UI-033`) |
| on a layout with no page at all | nothing, and the caller asks the engine for a first paragraph |

The block is found by bisection on the page's block positions, then the offset is read off the
run. Both halves are arithmetic over data the engine already produced.

**Moving the caret.**

```rust
pub enum Direction {
    Left, Right, WordLeft, WordRight,
    Up, Down, PageUp, PageDown,
    LineStart, LineEnd, DocStart, DocEnd,
}

/// Move the caret, or grow the selection to it when `extend`. Vertical moves cross wrapped
/// lines, page breaks and block boundaries by walking the line boxes of the layout.
pub fn move_cursor(state: &mut ProseUiState, dir: Direction, extend: bool);

/// The common tail of every move: place the caret, keep or clear the column, extend or
/// collapse the selection (`PROSE-UI-036`), then reveal it (`PROSE-UI-035`). Never mutates
/// the document.
fn apply_caret(state: &mut ProseUiState, next: TextCursor, x_goal: Option<f32>, extend: bool);

/// After a local insertion, advance the caret in the same block. Clears the column, because a
/// horizontal edit invalidates it, and reveals the caret through the same rule.
pub fn advance_cursor(state: &mut ProseUiState, n: u32);
```

Line start and line end honour the wrapped line under the caret and the caret's affinity at a
wrap point, not the paragraph. Every move funnels through the tail above, so no path can leave
the caret off screen (`PROSE-UI-035`).

**Dragging.**

```rust
/// Extend the drag to a pointer position. The anchor is the one captured on pointer-down
/// (`PROSE-UI-037`); a pointer past the top or bottom edge arms the autoscroll.
pub fn update_drag_selection(state: &mut ProseUiState, x: f32, y: f32);

/// One armed tick: scroll by the armed velocity, clamp it (`PROSE-UI-025`), then re-resolve
/// the last pointer position so the selection follows the revealed text (`PROSE-UI-038`).
pub fn tick_autoscroll(state: &mut ProseUiState);
```

The armed velocity and the anchor live in the UI state and nowhere else
(`features/ambre-spec-prose-engine.md section 3`). A selection built by dragging upward has its
focus before its anchor, and every reader of it normalizes first (`PROSE-UI-039`), including the
one that clamps a culled page to the window edge when it paints the bands.

**Selecting a word, a paragraph, everything.** Word and paragraph bounds are read off the shaped
runs at the hit-tested offset. Select-all sets the anchor to the first block's start and the focus
to the last block's end. Escape drops the selection, closes any open menu and cancels a drag in
flight; it never mutates the document.

**A press inside a link.** A link is a mark over text the author is still editing, so the press
resolves to a caret like any other and the link is inert (`PROSE-UI-070`). The press carrying the
platform's modifier resolves the target instead and hands it to the host's open-URL affordance,
which is where every application's outbound link goes (`PROSE-UI-071`); hovering paints the link
cursor. Neither path writes to the document. What the mark holds is
`features/ambre-spec-prose-inline-styles.md`.

**A press on an empty document.** A document with no block has nothing to hit-test, and a caret
placed in a block the view invented would be a second model of the document (`PROSE-UI-001`). The
press sends a block insertion and the caret lands at its start once the layout for it arrives
(`PROSE-UI-072`). This is the local guard only: two peers emptying the document at the same moment
converge by the format's own rule, `PROSE-027`.

**Code:** none. `crates/ambre-prose/src/lib.rs` is seven lines.

---

## 3. SURFACE

| Trigger | Behaviour |
|---|---|
| a click | places the caret, collapses the selection |
| a click inside a link | places the caret; the link is not followed |
| the same click with `Ctrl`, or `Cmd` | asks the host to open the link's target |
| a click on a document with no block | asks for one, then places the caret at its start |
| a double click, a triple click | selects the word, then the paragraph under the pointer |
| a drag | extends the selection from the anchor; past the window edge it scrolls while it extends |
| the arrow keys, with or without Shift | moves or extends by one grapheme or one line |
| `Ctrl` with an arrow | moves or extends by one word |
| Home, End | the start and the end of the wrapped line under the caret |
| `Ctrl` with Home or End | the start and the end of the document |
| Page Up, Page Down | one window of lines, keeping the column |
| `Ctrl` with `A` | selects the whole document and shows the format bar |
| Escape | drops the selection and any transient chrome |
| a local insertion | advances the caret past what was inserted and reveals it |
| an undo or a redo | places the caret at the change (`features/ambre-spec-prose-undo.md`) |

`Ctrl` with `A` is the chord Trame binds for the same purpose
(`2-Trame/features/ambre-spec-trame-selection.md section 4`); the fold of `Ctrl` and `Cmd` is one
decision for every application, and it is the "One keymap" Story.

---

## 4. DECISIONS

| Decision | Rationale | Rejected alternative |
|---|---|---|
| The hit-test reads the shaped runs, never a live layout | the runs are already in memory for every cached block, painted or not, and re-shaping a paragraph to answer a click costs a layout pass per pointer move. It also makes the answer identical to what was drawn, which is the whole of `PROSE-UI-008` | shaping the block under the pointer on demand, which is both slower and a second layout that may disagree with the first |
| A click never answers "nowhere" on a page that has content | a pointer lands in a margin, in a gap between blocks or under the last line constantly, and each of those is a real click a user expects to place a caret. Returning nothing pushes the decision into every caller, and each one invents a different fallback | returning no position outside a glyph, which makes the bottom half of a short page dead to the pointer |
| The column of a vertical run is remembered | walking down through a short line and a long one loses the column at the short one, and the caret then drifts left with every further press. Keeping the column from the first press is what makes a run of Down presses return to where it started | recomputing the column at each step, which turns a straight walk down a paragraph into a staircase |
| Every move reveals the caret | typing at the bottom of the window, extending a selection with Shift and jumping to the end of the document all put the caret out of view, and each would otherwise need its own scroll. One tail covers all of them, so no path can forget | scrolling at the call sites that need it, which is the version where one path forgets and the caret vanishes |
| The drag anchor is captured once and frozen | a remote peer editing before the anchor would otherwise move it mid-drag, and the selection would grow from a place the user never pressed. The anchor is a gesture, not a document position, for as long as the gesture lasts | re-resolving the anchor against the document, which makes a drag jump under a collaborator's edit |
| Autoscroll re-resolves the pointer each tick | the pointer is still, the text under it is not. Extending only on pointer moves stops the selection the moment the user holds the mouse past the edge, which is exactly when they are asking for more | extending on pointer movement alone, which makes the hold-at-the-edge gesture do nothing |
| A selection may run backwards | a drag upward is as common as one downward, and normalizing at capture time loses which end the user is moving, which is what Shift with an arrow key extends next | normalizing the selection when it is stored, which makes the moving end the one the user did not touch |
| The selection is ours, not the browser's | the browser's own selection describes DOM nodes, and the page is a canvas with none: there is nothing for it to select. Every position here comes from the shaped runs and the pointer instead, which is also the only representation that survives a page break | the browser's selection object, which has no node to anchor to on a canvas |
| A link needs a modifier to be followed | the text under a link is text the author edits, and a document editor that navigates on a plain click makes every link a place the caret cannot reach. The modifier is what every editor of this kind uses, so the gesture is already known | following the link on a plain click, which is a reader's behaviour applied to a writer's surface |
| The selection stays out of the document | a selection is one person's attention, not content, and writing it would put a keystroke's worth of traffic on the wire per pointer move. What a collaborator needs to see travels as an ephemeral instead | storing the selection in the document, which every peer then has to merge and undo |

---

## 5. NOT BUILT, AND NOT YET RULED

* **What a word is, is not defined.** Word-left, word-right and double click all rest on a word
  boundary, and no rule says whether it is the Unicode word break, whitespace, or something that
  treats punctuation and hyphens as Trame does.
* **Nothing rules on the autoscroll velocity.** The state holds one and no rule says how it is
  derived from the distance past the edge, whether it accelerates, or what stops it at the ends of
  the column.
* **A selection is not repaired when its text is deleted remotely.** The draft has a rule for that
  case (`PROSE-UI-028`); the selection endpoints have none, so a peer deleting the selected
  paragraph leaves two offsets naming a block that is gone.
* **The caret is not restored between sessions.** It is view state and nothing writes it, so a
  reader reopening a document starts at the top of it.
* **Right-to-left and bidirectional text are not ruled on anywhere in this area.** The caret
  affinity carries the wrap case alone, and no rule covers a caret at a direction boundary or a
  selection spanning one.
