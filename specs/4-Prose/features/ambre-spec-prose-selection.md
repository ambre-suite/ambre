# AMBRE -- FEATURE: CARET AND SELECTION

**Type:** Vertical (feature)
**Implementation:** implemented

> Where the caret is, where it goes, and what a drag selects. A click lands on the character under
> it. The arrow keys walk the text the reader sees rather than the text as it is stored. A
> selection that runs past the window keeps growing while the page scrolls.

The caret and the selection are local view state: they write nothing to the document and reach no
collaborator. There is no two-peer table below. What a peer sees of another's caret is
`1-Foundations/ambre-spec-presence.md`.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-UI-008` | `INV` | Hit-testing MUST be the exact inverse of the drawn layout, under zoom and scroll. | this spec section 2 | `ambre_prose::input_tests::a_point_resolves_to_the_offset_drawn_there_under_zoom_and_scroll`; `::the_hit_test_answers_a_document_offset_while_drafts_wait` | none | verified |
| `PROSE-UI-031` | `INV` | Hit-testing MUST read the shaped runs alone: a block that was never painted MUST resolve like a painted one. | this spec section 2 | `ambre_prose::input_tests::a_point_resolves_to_the_offset_drawn_there_under_zoom_and_scroll` | none | verified |
| `PROSE-UI-039` | `INV` | A selection MUST be valid with its end before its start. | this spec section 2 | `ambre_prose::input_tests::a_backwards_selection_is_read_in_order` | none | verified |
| `PROSE-UI-151` | `INV` | Every reader of a selection MUST order its two ends before it uses them: a backwards selection MUST NOT reach a caller as it is held. | this spec section 2 | `ambre_prose::input_tests::a_backwards_selection_is_read_in_order` | none | verified |
| `PROSE-UI-034` | `INV` | A vertical move MUST keep the column the caret started from until a horizontal move, a click or an edit ends the run of vertical moves. | this spec section 2 | `ambre_prose::input_tests::a_run_of_vertical_moves_keeps_its_column` | none | verified |
| `PROSE-UI-152` | `INV` | A change from a collaborator MUST NOT move the anchor of a drag in flight: the anchor is a gesture, not a document position, for as long as the gesture lasts. | this spec section 2 | `ambre_prose::input_tests::a_remote_change_does_not_move_the_anchor_of_a_drag` | none | verified |
| `PROSE-UI-033` | `INV` | Header and footer text MUST hit-test like body text, so the caret can enter it. | this spec section 2 | `ambre_prose::input_tests::a_press_in_a_header_or_a_footer_lands_in_its_text`; `ambre_prose::input_tests::a_selection_does_not_cross_the_edge_of_a_band` | none | verified |
| `PROSE-UI-187` | `INV` | Footnote text MUST hit-test like body text, so the caret can enter it. | this spec section 2 | `input_tests::a_press_in_a_note_lands_in_its_text`; `::a_selection_does_not_cross_the_edge_of_a_note` -- a note is a block out of the flow, so the press resolves into it and the selection stays inside it | none | verified |
| `PROSE-UI-153` | `OBS` | A pointer resting inside a link MUST show the link cursor, over the marked text alone. | this spec section 2 | `ambre_prose::input_tests::a_link_answers_over_its_marked_text_alone` | none | verified |

### 1.2 Press in the text

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-UI-009` | `POST` | A click that lands in no glyph MUST resolve to the nearest valid caret position, never to no position. | this spec section 2 | `ambre_prose::input_tests::a_press_in_the_margin_resolves_to_the_nearest_caret` | none | verified |
| `PROSE-UI-032` | `POST` | A block that holds no text MUST resolve to one of its two edges, chosen by the side the pointer is on. | this spec section 2 | `ambre_prose::input_tests::a_block_without_text_resolves_to_the_edge_on_the_pointers_side` | none | verified |
| `PROSE-UI-154` | `POST` | A press that does not extend MUST collapse the selection to the caret it places. | this spec section 2 | `ambre_prose::input_tests::a_press_places_the_caret_and_collapses_the_selection` | none | verified |
| `PROSE-UI-155` | `POST` | A second press in place MUST select the word under the pointer. | this spec section 2 | `ambre_prose::input_tests::a_second_press_selects_the_word_and_a_third_the_paragraph` | none | verified |
| `PROSE-UI-171` | `POST` | A third press in place MUST select the paragraph the word sits in. | this spec section 2 | `ambre_prose::input_tests::a_second_press_selects_the_word_and_a_third_the_paragraph`; `::a_third_press_selects_the_text_drafted_at_the_blocks_end` | none | verified |
| `PROSE-UI-072` | `POST` | A press on a document holding no block MUST place the caret in a block created through the command path, never in a locally invented one. | this spec section 2; `PROSE-027` | `ambre_prose::input_tests::a_press_on_a_document_with_no_block_asks_for_one` | none | verified |
| `PROSE-UI-156` | `FAIL` | A press on a layout carrying no page at all MUST resolve to no position, and the caret MUST stay where it was until a block exists to hold it. | this spec section 2 | `ambre_prose::input_tests::a_press_on_a_layout_with_no_page_leaves_the_caret` | none | verified |
| `PROSE-UI-157` | `FRAME` | Placing the caret with a press MUST NOT shape a block and MUST NOT scroll the column: what the press resolved is already on screen. | this spec section 2 | `ambre_prose::input_tests::a_press_places_the_caret_and_collapses_the_selection` | none | verified |

### 1.3 Walk the caret with the keys

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-UI-036` | `POST` | A caret move without extension MUST collapse the selection to the caret. | this spec section 2 | `ambre_prose::input_tests::shift_extends_from_a_fixed_anchor_and_a_plain_move_collapses` | none | verified |
| `PROSE-UI-035` | `POST` | Every caret move MUST leave the caret on screen. | this spec section 2 | `ambre_prose::input_tests::every_move_leaves_the_caret_on_screen` | none | verified |
| `PROSE-UI-158` | `POST` | A move that takes the caret out of the window MUST scroll the least that brings it back. | this spec section 2 | `ambre_prose::input_tests::every_move_leaves_the_caret_on_screen` | none | verified |
| `PROSE-UI-159` | `POST` | A move to the start or the end of a line MUST honour the wrapped line under the caret, and the caret's affinity at a wrap point, never the paragraph. | this spec section 2 | `ambre_prose::input_tests::home_and_end_honour_the_wrapped_line` | none | verified |
| `PROSE-UI-233` | `POST` | A move that lands on the end of a block MUST land after the text drafted there, while no layout carries that text yet. | this spec section 2 | `ambre_prose::input_tests::a_move_to_a_blocks_end_lands_after_its_draft` | none | verified |
| `PROSE-UI-160` | `FRAME` | A move that extends the selection MUST NOT move its anchor. | this spec section 2 | `ambre_prose::input_tests::shift_extends_from_a_fixed_anchor_and_a_plain_move_collapses` | none | verified |

### 1.4 Drag to select

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-UI-037` | `POST` | The anchor of a drag MUST be captured when the drag starts. | this spec section 2 | `ambre_prose::input_tests::a_drag_grows_from_the_anchor_its_press_captured` | none | verified |
| `PROSE-UI-161` | `POST` | Extending a drag MUST resolve the moving end at the position under the pointer, leaving the anchor where the drag started. | this spec section 2 | `ambre_prose::input_tests::a_drag_grows_from_the_anchor_its_press_captured` | none | verified |
| `PROSE-UI-162` | `FRAME` | Extending a drag MUST NOT scroll the column: only an armed autoscroll tick does. | this spec section 2 | `ambre_prose::input_tests::a_drag_grows_from_the_anchor_its_press_captured` | none | verified |

### 1.5 Hold the drag past the edge of the window

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-UI-038` | `POST` | While a drag holds past the edge of the window, each tick MUST scroll and then re-resolve the pointer, so the selection follows the text being revealed. | this spec section 2 | `ambre_prose::input_tests::a_drag_past_the_edge_scrolls_and_the_selection_follows` | none | verified |
| `PROSE-UI-163` | `FRAME` | A tick MUST NOT run once the pointer is released: releasing MUST leave nothing armed. | this spec section 2 | `ambre_prose::input_tests::a_released_drag_leaves_nothing_armed` | none | verified |

### 1.6 Press Escape

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-UI-164` | `POST` | Escape MUST collapse the selection to the caret. | this spec section 2 | `ambre_prose::input_tests::escape_collapses_and_cancels_the_drag` | none | verified |
| `PROSE-UI-165` | `POST` | Escape MUST cancel a drag in flight: the release that follows MUST extend nothing. | this spec section 2 | `ambre_prose::input_tests::escape_collapses_and_cancels_the_drag` | none | verified |
| `PROSE-UI-166` | `FRAME` | Escape MUST NOT move the caret and MUST NOT change the scroll offset. | this spec section 2 | `ambre_prose::input_tests::escape_collapses_and_cancels_the_drag` | none | verified |

### 1.7 Select the whole document

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-UI-167` | `POST` | Selecting the whole document MUST set the anchor at the start of the first block and the moving end at the end of the last. | this spec section 2 | `ambre_prose::input_tests::select_all_spans_the_document_and_keeps_the_window`; `::a_third_press_selects_the_text_drafted_at_the_blocks_end` | none | verified |
| `PROSE-UI-168` | `FRAME` | Selecting the whole document MUST NOT change the scroll offset: the reader stays on the page they were on. | this spec section 2 | `ambre_prose::input_tests::select_all_spans_the_document_and_keeps_the_window` | none | verified |

### 1.8 Press inside a link

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-UI-070` | `POST` | A press inside a link, without the platform's modifier, MUST place the caret. | this spec section 2 | `ambre_prose::input_tests::a_plain_press_inside_a_link_places_the_caret_and_opens_nothing` | none | verified |
| `PROSE-UI-169` | `FRAME` | A press inside a link, without the platform's modifier, MUST NOT open the target. | this spec section 2 | `ambre_prose::input_tests::a_plain_press_inside_a_link_places_the_caret_and_opens_nothing` | none | verified |
| `PROSE-UI-071` | `POST` | Opening a link's target MUST go through the shared open-URL affordance of the host, never a call made from the crate. | this spec section 2; `UI-THREAD-004` | type-level -- the view only fills `ProseUi.followed`, and the one reader of it is the shell's `follow_shell_nav`; `ambre_prose::input_tests::a_press_with_the_modifier_hands_the_target_over_and_moves_nothing` | none | verified |
| `PROSE-UI-170` | `FRAME` | Opening a link's target MUST NOT move the caret and MUST NOT change the selection. | this spec section 2 | `ambre_prose::input_tests::a_press_with_the_modifier_hands_the_target_over_and_moves_nothing` | none | verified |
| `PROSE-UI-186` | `FRAME` | A link's target MUST reach the host only when its scheme is `http`, `https` or `mailto`, read with case folded and with leading control characters, tabs and newlines skipped. | this spec section 2; `UI-THREAD-004` | `ambre_prose::link::tests::only_a_web_or_mail_target_may_be_opened`; `ambre_prose::input_tests::a_target_that_could_run_code_is_held_back` | none | verified |

* **Budget.** the hit-test of a pointer position, a row of `ambre-spec-budgets.md section 1`.
  The autoscroll tick costs one hit-test per frame and no row of its own.
* **Boundary.** none: `PROSE-UI-010` (`features/ambre-spec-prose-engine.md section 1`) keeps the
  selection out of the document, and the UI thread reaches no network, crypto or persistence path
  (`UI-THREAD-004`).
* **Verification.** 38 `verified`. The scroll tick is driven by hand in the tests, as the host
  drives it once a frame. `PROSE-UI-008` is a round trip: the point the layout draws an offset at
  resolves back to that offset, at any zoom and scroll. `PROSE-UI-071` holds by the shape of the
  view: it sets a field and opens nothing, and the shell is the one reader of that field.

---

## 2. MODEL

**Containers.** None. Nothing here is persisted and nothing is shared.

**The caret and the selection** (`ambre-prose`, UI thread):

```rust
/// The caret, and the fixed end of the selection when there is one (`ambre-bus`).
pub struct CursorPosition {
    pub block_id: BlockId,
    pub offset:   DocOffset,
    /// The fixed end, `None` for a collapsed caret. The caret is the moving end, and it
    /// may precede the anchor (`PROSE-UI-039`).
    pub anchor:   Option<(BlockId, DocOffset)>,
}

/// Which side of a soft wrap the caret sits on.
pub enum Affinity { Downstream, Upstream }

/// What the UI state keeps of the caret, beside the rest of the view.
pub struct ProseUiState {
    pub cursor:         Option<CursorPosition>,
    pub caret_affinity: Affinity,
    /// The column a run of vertical moves keeps (`PROSE-UI-034`).
    x_goal:             Option<Px<Doc>>,
    /* the rest of the view */
}
```

**From a point to a position.** The viewport answers which page a screen point is on and where
inside it (`features/ambre-spec-prose-viewport.md section 2`); the hit-test turns that into a
block and an offset. It reads the shaped runs, never a fresh layout pass, which is what makes it
answer for a culled block (`PROSE-UI-031`).

```rust
/// The block and offset under a canvas point. `None` only for a layout with no page.
impl HitTest for ProseUiState {
    type Target = (BlockId, DocOffset);
    fn hit_test(&self, at: Point<Px<View>>) -> Option<(BlockId, DocOffset)>;
}
```

| The point lands | The answer |
|---|---|
| in a glyph | that glyph's offset, from the run's own cluster data (`PROSE-UI-008`) |
| in a page margin, above the first block or below the last | the nearest block edge on that page, then the page's first or last block (`PROSE-UI-009`) |
| in a block with no text -- an image, a rule, a page break | that block's offset zero or its end, by the pointer side (`PROSE-UI-032`) |
| in a header or a footer | that text's own offset, and the caret shows on that page alone (`PROSE-UI-033`) |
| in the footnote band | that note's own offset (`PROSE-UI-187`) |
| on a layout with no page at all | nothing, and the caret stays where it was (`PROSE-UI-156`) |

The block is found by bisection on the page's block positions, then the offset is read off the
run. Both halves are arithmetic over data the engine already produced.

**Moving the caret.**

```rust
pub enum Direction {
    Left, Right, WordLeft, WordRight,
    Up, Down, PageUp, PageDown,
    LineStart, LineEnd, DocStart, DocEnd,
}

/// A place in the text: a block and a code-point offset.
type Spot = (BlockId, DocOffset);

impl ProseUiState {
    /// Move the caret, or grow the selection to it under `SelectionMode::Extend`. Vertical
    /// moves cross wrapped lines, page breaks and block boundaries by walking the line
    /// boxes of the layout.
    pub fn move_cursor(&mut self, direction: Direction, mode: SelectionMode);

    /// The common tail of every move: place the caret, keep or clear the column, extend or
    /// collapse the selection (`PROSE-UI-036`), then reveal it (`PROSE-UI-035`). Never
    /// mutates the document.
    fn apply_caret(&mut self, next: Spot, affinity: Affinity, x_goal: Option<Px<Doc>>,
                   mode: SelectionMode);

    /// After a local insertion, advance the caret in the same block. Clears the column,
    /// because a horizontal edit invalidates it, and reveals the caret through the same rule.
    fn advance_cursor(&mut self, n: DocCount);
}
```

Line start and line end honour the wrapped line under the caret and the caret's affinity at a
wrap point, not the paragraph (`PROSE-UI-159`). Every move funnels through the tail above, so no
path can leave the caret off screen (`PROSE-UI-035`).

**Dragging.**

```rust
/// Extend the drag to a pointer position. The anchor is the one captured on pointer-down
/// (`PROSE-UI-037`); a pointer near the top or bottom edge arms the autoscroll.
pub fn update_drag(&mut self, at: Point<Px<View>>);

/// One tick, once a frame: scroll by the armed speed, clamped (`PROSE-UI-025`), then
/// re-resolve the last pointer so the selection follows the revealed text (`PROSE-UI-038`).
pub fn tick_autoscroll(&mut self, now: PageTime) -> bool;
```

The armed velocity and the anchor live in the UI state and nowhere else
(`features/ambre-spec-prose-engine.md section 3`). A selection built by dragging upward has its
focus before its anchor, and every reader of it normalizes first (`PROSE-UI-151`). That includes
the one that clamps a culled page to the window edge when it paints the bands.

**Selecting a word, a paragraph, everything.** Word and paragraph bounds are read off the shaped
runs at the hit-tested offset (`PROSE-UI-155`). Select-all sets the anchor to the first block's
start and the focus to the last block's end (`PROSE-UI-167`). Escape drops the selection and
cancels a drag in flight (`PROSE-UI-164`, `PROSE-UI-165`); it closes any open menu, and it never
mutates the document.

**A press inside a link.** A link is a mark over text the author is editing, so the press
resolves to a caret and the link is inert (`PROSE-UI-070`). The press carrying the platform's
modifier resolves the target instead. It hands the target to the host's open-URL affordance, which
is where every application's outbound link goes (`PROSE-UI-071`). Hovering paints the link cursor
(`PROSE-UI-153`). Neither path writes to the document, and what the mark holds is
`features/ambre-spec-prose-inline-styles.md`.

**A target from a peer.** Nothing rules on a link's target when it is written, so a peer may
write `javascript:` or `data:`. Handed to the browser, such a target would run in the page's
stead, with the session's keys in reach. The view hands over `http`, `https` and `mailto` alone,
read the way a browser reads a scheme (`PROSE-UI-186`). Any other target stays in the mark, and
the press moves nothing.

**A press on an empty document.** A document with no block has nothing to hit-test. A caret
placed in a block the view invented would be a second model of the document (`PROSE-UI-001`). The
press sends a block insertion and the caret lands at its start once the layout for it arrives
(`PROSE-UI-072`). This is the local guard only: two peers emptying the document at the same moment
converge by the format's own rule, `PROSE-094`.

**Code:** `crates/ambre-prose/src/view/caret.rs` moves the caret and resolves a point, and
`crates/ambre-prose/src/input.rs` counts the presses.

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
(`2-Trame/features/ambre-spec-trame-selection.md section 4`). The fold of `Ctrl` and `Cmd` is
decided for every application in `1-Foundations/ambre-spec-commands.md section 2`. The chord is
declared once and the command key answers it.

---

## 4. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| The hit-test reads the shaped runs, never a live layout | shaping the block under the pointer on demand | it is both slower and a second layout that may disagree with the first. The runs are already in memory for every cached block, painted or not, and re-shaping a paragraph to answer a click costs a layout pass per pointer move; reading the runs also makes the answer identical to what was drawn, which is the whole of `PROSE-UI-008` |
| A click never answers "nowhere" on a page that has content | returning no position outside a glyph | it makes the bottom half of a short page dead to the pointer. A pointer lands in a margin, in a gap between blocks or under the last line constantly, and each of those is a real click a user expects to place a caret; returning nothing pushes the decision into every caller, and each one invents a different fallback |
| The column of a vertical run is remembered | recomputing the column at each step | it turns a straight walk down a paragraph into a staircase. Walking down through a short line and a long one loses the column at the short one, and the caret then drifts left with every further press; keeping the column from the first press is what makes a run of Down presses return to where it started |
| Every move reveals the caret | scrolling at the call sites that need it | it is the version where one path forgets and the caret vanishes. Typing at the bottom of the window, extending a selection with Shift and jumping to the end of the document all put the caret out of view, and one tail covers all of them, so no path can forget |
| The drag anchor is captured once and frozen | re-resolving the anchor against the document | it makes a drag jump under a collaborator's edit. A remote peer editing before the anchor would otherwise move it mid-drag, and the selection would grow from a place the user never pressed; the anchor is a gesture, not a document position, for as long as the gesture lasts |
| Autoscroll re-resolves the pointer each tick | extending on pointer movement alone | it makes the hold-at-the-edge gesture do nothing. The pointer is still and the text under it is not; extending only on pointer moves stops the selection the moment the user holds the mouse past the edge, which is exactly when they are asking for more |
| A selection may run backwards | normalizing the selection when it is stored | it makes the moving end the one the user did not touch. A drag upward is as common as one downward, and normalizing at capture time loses which end the user is moving, which is what Shift with an arrow key extends next |
| The selection is ours, not the browser's | the browser's selection object | it has no node to anchor to on a canvas. The browser's own selection describes DOM nodes, and the page is a canvas with none; every position here comes from the shaped runs and the pointer instead, which is also the only representation that survives a page break |
| A link needs a modifier to be followed | following the link on a plain click | it is a reader's behaviour applied to a writer's surface. The text under a link is text the author edits, and a document editor that navigates on a plain click makes every link a place the caret cannot reach; the modifier is what every editor of this kind uses, so the gesture is already known |
| The selection stays out of the document | storing the selection in the document | every peer then has to merge and undo it. A selection is one person's attention, not content, and writing it would put a keystroke's worth of traffic on the wire per pointer move; what a collaborator needs to see travels as an ephemeral instead |

---

## 5. NOT BUILT, AND NOT YET RULED

* **What a word is, is not defined.** Word-left, word-right and double click all rest on a word
  boundary. No rule says whether it is the Unicode word break, whitespace, or something that
  treats punctuation and hyphens as Trame does.
* **Nothing rules on the autoscroll velocity.** The state holds one. No rule says how it is
  derived from the distance past the edge, whether it accelerates, or what stops it at the
  column's ends.
* **A selection is not repaired when its text is deleted remotely.** The draft has a rule for that
  case (`PROSE-UI-028`), and the selection endpoints have none. A peer deleting the selected
  paragraph leaves two offsets naming a block that is gone.
* **Nothing says which affinity a press produces at a wrap point.** The caret carries one and the
  line-start move honours it (`PROSE-UI-159`). No row says whether a click at the end of a
  wrapped line lands before the wrap or after it. The two answers send Home to two places.
* **Nothing rules on a relayout under a held drag.** The autoscroll re-resolves the pointer each
  tick against a layout the engine may have replaced. No row says what the selection does when
  the page under the pointer is no longer the page that was there.
* **The caret is not restored between sessions.** It is view state and nothing writes it, so a
  reader reopening a document starts at the top of it.
* **Right-to-left and bidirectional text are not ruled on anywhere in this area.** The caret
  affinity carries the wrap case alone, and no rule covers a caret at a direction boundary or a
  selection spanning one.
