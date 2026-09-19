# AMBRE -- FEATURE: THE CONTEXTUAL TOOLBAR

**Type:** Vertical (feature)
**Implementation:** design-only

> Select a few words and a small bar appears above them. It shows what the selection already is --
> bold, italic, a heading -- and lets the author change it.

The bar shows a state and sends a command; it owns neither. What a mark means is
`features/ambre-spec-prose-inline-styles.md`, what a heading level is
`features/ambre-spec-prose-headings.md`, and where the selection is
`features/ambre-spec-prose-selection.md`.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-UI-054` | `OBS` | Each control MUST report the state of the whole selection. | this spec section 3 | not built | unbuilt -- lands with the "Toolbar, slash menu and composition" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-UI-096` | `OBS` | A selection that is formatted in part MUST be reported as mixed, never as the state of either end. | this spec section 3 | not built | unbuilt -- lands with the "Toolbar, slash menu and composition" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-UI-055` | `OBS` | The bar MUST stay over the text it was raised on while the page scrolls or zooms. | this spec section 3 | not built | unbuilt -- lands with the "Toolbar, slash menu and composition" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-UI-057` | `INV` | The bar MUST NOT hold document content: the state it renders MUST arrive from the Logic thread with the selection it describes. | this spec section 3; `PROSE-UI-001` | not built | unbuilt -- lands with the "Toolbar, slash menu and composition" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-UI-097` | `INV` | The state a control shows MUST be rebuilt from the current selection; a mark MUST NOT be carried over from the selection before it. | this spec section 5 | not built | unbuilt -- lands with the "Toolbar, slash menu and composition" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-UI-056` | `INV` | A control and the keyboard chord for the same formatting MUST resolve to one command id. | this spec section 3; `ambre-spec-chrome.md` | not built | unbuilt -- lands with the "Toolbar, slash menu and composition" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-UI-098` | `INV` | The bar MUST NOT have a path into the document of its own: every control MUST reach it through a command the plane already declares. | this spec section 5 | not built | unbuilt -- lands with the "Toolbar, slash menu and composition" Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.2 Raise the bar

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-UI-053` | `POST` | The bar MUST appear only over a non-empty selection. | this spec section 3 | not built | unbuilt -- lands with the "Toolbar, slash menu and composition" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-UI-099` | `FRAME` | Raising the bar MUST NOT write to the document, and MUST NOT move or shrink the selection it was raised on. | derived | not built | unbuilt -- lands with the "Toolbar, slash menu and composition" Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.3 Dismiss the bar

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-UI-100` | `POST` | The bar MUST disappear when the selection is emptied or cancelled. | this spec section 3 | not built | unbuilt -- lands with the "Toolbar, slash menu and composition" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-UI-101` | `FRAME` | Dismissing the bar MUST NOT write to the document: a formatting applied before it closed MUST stay applied. | derived | not built | unbuilt -- lands with the "Toolbar, slash menu and composition" Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.4 Format from a control

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-UI-102` | `POST` | A control MUST apply or remove its formatting over the whole selection. | this spec section 4 | not built | unbuilt -- lands with the "Toolbar, slash menu and composition" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-UI-103` | `POST` | A control reporting mixed MUST apply its formatting to the whole selection rather than remove it. | this spec section 3 | not built | unbuilt -- lands with the "Toolbar, slash menu and composition" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-UI-104` | `FRAME` | A control MUST NOT change the text of the selection, its extent, or any formatting the control does not name. | derived | not built | unbuilt -- lands with the "Toolbar, slash menu and composition" Story (`ambre-roadmap.md` E5) | unimplemented |

* **Budget.** none of its own. The bar is repositioned inside the frame that paints the page, the
  row of `ambre-spec-budgets.md section 2`. A static element over the canvas costs a compositor
  recomposition and no layout pass (`ambre-spec-ui.md section 4`).
* **Boundary.** none: the bar reads view state and emits commands. It holds no key, opens no
  socket, and every mutation it starts is a `ProseCommand` (`PROSE-UI-002`).
* **Verification.** 14 `unimplemented`, nothing else: `crates/ambre-prose` is a `lib.rs`.
  `PROSE-UI-054` and `PROSE-UI-096` are a fold over the marks of a range, so a host test reaches
  them without a browser. So are the three rows a control writes, which a delta answers.
  `PROSE-UI-055` and `PROSE-UI-053` are browser behaviour
  and land in the wasm tier of `ambre-spec-testing.md section 6`.

---

## 2. WHAT EACH USER SEES

The bar is local: it is raised by a selection, and a selection stays on the peer that made it
(`PROSE-UI-010`). What converges is what its controls write. No test resolves yet: the last column
names the row each scenario rests on.

| Alice does | Bob does | The document holds | Each user sees | Test |
|---|---|---|---|---|
| selects a sentence | selects nothing | nothing: a selection is local | a bar over Alice's sentence, and no bar for Bob | none -- `PROSE-UI-053` |
| makes the sentence bold | reads it | the bold mark over the range | the sentence bold on both screens, the control lit for Alice | none -- `PROSE-UI-102` |
| selects a range half of which is bold | -- | nothing | the bold control mixed, neither on nor off | none -- `PROSE-UI-096` |
| makes it bold | makes the same range italic at the same moment | both marks, neither lost | bold and italic text for both | none -- `PROSE-FEAT-008` |
| scrolls while the bar is open | -- | nothing | the bar still over the same words | none -- `PROSE-UI-055` |

---

## 3. MODEL

**Containers.** This feature owns none. It reads the marks of the selected range and writes them
through the commands of `features/ambre-spec-prose-inline-styles.md`.

**The state the view publishes.** The bar is a DOM element, and the canvas view is what knows where
the selection is. The view publishes a state and the element renders it
(`ambre-spec-ui.md section 2`, Canvas to DOM by signal).

```rust
// ambre-prose/src/ui/toolbar.rs -- published to the chrome component, never drawn on the canvas

pub struct ToolbarState {
    pub visible: bool,
    /// The selection's bottom-centre, in canvas space. The element is transform-synced to it.
    pub anchor:  Vec2,
    pub active:  ActiveMarks,
}

pub struct ActiveMarks {
    pub bold:          TriState,
    pub italic:        TriState,
    pub underline:     TriState,
    pub strikethrough: TriState,
    pub code:          TriState,
    /// The address, when the whole selection carries one link and the same one.
    pub link:          Option<String>,
}

pub enum TriState { Off, On, Mixed }
```

**Three states, not two.** A selection covering formatted and unformatted text is neither on nor
off. Reporting it as either makes the next click do the opposite of what the author expects
(`PROSE-UI-096`). `Mixed` is what a control shows, and a click on a mixed control applies the mark
to the whole range rather than removing it (`PROSE-UI-103`).

**Steps, on every selection change:**

1. the view reads the new selection; an empty one hides the bar and stops (`PROSE-UI-053`);
2. it folds the marks carried by the range into `ActiveMarks`, one `TriState` per control
   (`PROSE-UI-054`);
3. it computes the anchor rectangle from the selection's geometry and raises the signal;
4. the element positions itself against the anchor, and follows it each frame while it is visible
   (`PROSE-UI-055`).

**What a control sends.** A control invokes a command id of `ambre-commands`, the same id the
keyboard chord invokes. That id resolves to one command of the plane
(`features/ambre-spec-prose-engine.md section 3`). There is no toolbar-only path into the document
(`PROSE-UI-098`).

| Control | Command |
|---|---|
| bold, italic, underline, strikethrough, inline code | `ApplyMark` or `RemoveMark`, on the selected range |
| the link control | `ApplyMark` with the `a` key, or `RemoveMark` with it |
| the heading controls | `SetHeadingLevel`, on the block the selection starts in |
| the quote control | `SetBlockStyle`, with the built-in `quote` style |
| the overflow control | opens the colour and size controls, which write through `ApplyMark` |

**The bar holds nothing.** `ToolbarState` is view state, rebuilt from the selection and from what
the Logic thread reports about it. The bar never remembers a mark across a selection change
(`PROSE-UI-097`).

**Code:** none. `crates/ambre-prose/src/lib.rs` is seven lines.

---

## 4. SURFACE

| Trigger | What happens |
|---|---|
| a pointer release leaving a non-empty selection | the bar appears above it |
| a keyboard selection reaching a non-empty range | the same |
| the selection is emptied, or a click lands outside it | the bar disappears |
| a control | applies or removes the formatting on the whole selection |
| the overflow control | reveals the controls the bar has no room for |
| a narrow window | the bar becomes a bottom sheet (`ambre-spec-adaptive-layout.md`) |

The element, its keyboard order and its dismissal are the shared chrome toolkit's, and are not
respecified here (`ambre-spec-chrome.md`).

---

## 5. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| The bar is a DOM element over the canvas | drawing the bar on the `DrawSurface`, beside the text it formats | the canvas owns content and the DOM owns everything else (`UI-THREAD-001`), and a DOM element inherits keyboard order, focus and a screen-reader name from the browser. A canvas bar reimplements all three and is still not reachable by a screen reader |
| The view computes the anchor, the element follows it | letting the element hit-test the document | it puts a second geometry model in the chrome. Only the canvas knows where a selection is on screen and only the DOM can place an element, which is the boundary the framework already draws |
| A mixed selection is a third state | reporting the state of the selection's first character | it is what a two-state control does, and why it toggles the wrong way: the control lies about half the selections a user makes, and the lie decides what the next click does. Three states cost one enum variant |
| A control invokes a command id, never a command | wiring each control straight to a `ProseCommand` | it duplicates the mapping once per surface. The chord, the menu and the bar are three surfaces for one operation, and one id keeps them one operation and the keymap remappable |
| The bar carries no document state | caching the marks of the last selection to avoid a fold | it makes the bar wrong after any concurrent change. The UI thread holds no document (`PROSE-UI-001`), and a remembered mark is a second truth a remote edit puts out of step |

---

## 6. NOT BUILT, AND NOT YET RULED

* **Nothing exists.** `crates/ambre-prose/src/lib.rs` is seven lines, so the bar, its state and its
  anchor are a design, and every row above is `unimplemented`.
* **The fold has no declared source.** `ActiveMarks` is a fold over the marks of a range, and no
  event of `ProseEvent` carries them. The view is told the layout and the runs, not the formatting
  of a selection (`features/ambre-spec-prose-engine.md section 3`). Either an event is
  missing or the fold is meant to run over the run cache, and no document says which.
* **The controls are fewer than the marks.** The registry declares eleven presentation marks plus
  the link (`ambre-spec-prose-schema.md section 3`). The bar names five, a link, three heading
  levels and a quote, and defers colour and size to an overflow nothing specifies. Superscript,
  subscript and clearing formatting reach no surface at all.
* **No control opens a comment or a suggestion.** Both features record the same absence, and the
  bar is where a selection-scoped gesture would live
  (`features/ambre-spec-prose-comments.md section 6`,
  `features/ambre-spec-prose-track-changes.md section 6`).
* **A refused mark value has nowhere to go.** `PROSE-FEAT-022` requires a malformed value to be
  reported to the author, and no event carries the refusal. The colour and size controls therefore
  have no failure path (`features/ambre-spec-prose-inline-styles.md section 6`). It lands with the
  `A refused command names what it refused` Story (`ambre-roadmap.md` E5).
* **What a control applies to is stated twice, and differently.** Section 4 says a control
  applies its formatting to the whole selection, which is what `PROSE-UI-102` carries. The control
  table of section 3 sends the heading and quote controls to the block the selection starts in.
  A selection spanning three paragraphs therefore has two answers, and nothing rules between them.
* **The anchor has no rule when the selection leaves the window.** A selection scrolled off screen
  keeps an anchor with no pixels under it. Nothing says whether the bar hides, sticks to the
  edge or follows the anchor out of view.
