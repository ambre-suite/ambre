# AMBRE -- FEATURE: TEXT EDITING

**Type:** Vertical (feature)
**Implementation:** design-only

> Typing, deleting, splitting and joining paragraphs. A keystroke shows on the glass at once and
> reaches the document a moment later, so two authors typing in the same paragraph never wait for
> each other and never lose a character.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|---|
| `PROSE-UI-004` | `INV` | The view MUST NOT mutate shared state: what was typed and not yet confirmed MUST live in a draft held apart from the document, and every mutation MUST leave as a command. | this spec section 3 | not built -- lands with the "Input and cursor" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-UI-134` | `OBS` | The text a draft holds MUST be drawn under the caret over the confirmed runs, before the document has answered. | this spec section 3 | not built -- lands with the "Input and cursor" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-UI-135` | `INV` | A draft MUST hold text and an anchor and no structure: it MUST NOT be able to express a split, a deletion or a mark. | this spec section 3 | not built -- lands with the "Input and cursor" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-UI-137` | `INV` | One primitive MUST delete the active selection, and a second one MUST NOT exist per editing path. | this spec section 3 | not built -- lands with the "Ruler, clipboard and inertial scroll" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-039` | `INV` | Joining two paragraphs MUST be expressed as the deletion below with an empty range; a second join operation MUST NOT exist. | this spec section 3 | not built -- lands with the `ARCH P2` Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.2 Type a run of text

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|---|
| `PROSE-UI-136` | `POST` | The text MUST be appended to the draft at its anchor, and one insertion command MUST leave carrying the same text at the same position. | this spec section 3 | not built -- lands with the "Input and cursor" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-UI-138` | `POST` | The draft MUST re-anchor only when it moves to another block, so consecutive keystrokes in one block MUST accumulate in one draft. | this spec section 3 | not built -- lands with the "Input and cursor" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-UI-139` | `FRAME` | Typing MUST NOT change the structure of the document, the paragraph's settings, or the marks the run lands beside. | this spec section 3 | not built -- lands with the "Input and cursor" Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.3 Take the active selection

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|---|
| `PROSE-UI-030` | `POST` | An edit that inserts or splits MUST delete the active selection first, and the caret MUST collapse to the start of the range it deleted. | this spec section 3 | not built -- lands with the "Ruler, clipboard and inertial scroll" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-UI-140` | `FRAME` | Taking the selection MUST emit the deletion alone: the insertion or the split that follows MUST be the caller's, and MUST reach the document after it. | this spec section 3 | not built -- lands with the "Ruler, clipboard and inertial scroll" Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.4 Delete across a paragraph boundary

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|---|
| `PROSE-037` | `POST` | What survives of the last paragraph MUST be appended to the first, and every paragraph the deletion emptied MUST be removed. | this spec section 3 | not built -- lands with the `ARCH P2` Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-041` | `FRAME` | The deletion MUST NOT change the identity of the first paragraph or its paragraph style, whichever of the two the caret started in. | this spec section 3 | not built -- lands with the `ARCH P2` Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.5 Split a paragraph

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|---|
| `PROSE-038` | `POST` | The new paragraph MUST carry the source's type and paragraph style. | this spec section 3 | not built -- lands with the `ARCH P2` Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-UI-141` | `POST` | The caret MUST be left at the start of the new paragraph. | this spec section 3 | not built -- lands with the `ARCH P2` Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-042` | `FRAME` | The split MUST NOT change the text the two paragraphs carry between them, or the identity of the source paragraph. | this spec section 3 | not built -- lands with the `ARCH P2` Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.6 Compose with an input method

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|---|
| `PROSE-UI-029` | `POST` | The composed text MUST reach the document once, when the composition ends, at the position the draft holds then. | this spec section 3 | not built -- lands with the "Toolbar, slash menu and composition" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-UI-142` | `FRAME` | While a composition is open the view MUST send no text to the document: a preview MUST reach the canvas and nothing else. | this spec section 3 | not built -- lands with the "Toolbar, slash menu and composition" Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.7 Take in a document event

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|---|
| `PROSE-UI-005` | `POST` | An event confirming the anchored block MUST discard the draft: its text is then in the confirmed runs. | this spec section 3 | not built -- lands with the "Input and cursor" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-UI-028` | `POST` | A remote change MUST shift the pending draft by what it applied at or before the draft position. | this spec section 3 | not built -- lands with the "Input and cursor" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-UI-143` | `POST` | The draft MUST be dropped only when the block it is anchored in is deleted, and the caret MUST then collapse to the nearest confirmed position. | this spec section 3 | not built -- lands with the "Input and cursor" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-UI-027` | `FRAME` | An event carrying no change MUST leave the draft untouched: a download or a presence update MUST NOT erase what is being typed. | this spec section 3 | not built -- lands with the "Input and cursor" Story (`ambre-roadmap.md` E5) | unimplemented |

* **Budget.** the keystroke to a local echo and the keystroke to a confirmed one, two rows of
  `ambre-spec-budgets.md section 1`. The first is what the draft exists to hold.
* **Boundary.** none: the draft is view state on the UI thread, where `UI-THREAD-004` bans a
  network, crypto or persistence path, and every mutation leaves as a command (`PROSE-UI-002`,
  `features/ambre-spec-prose-engine.md section 1`).
* **Verification.** 21 `unimplemented`, nothing else: `crates/ambre-prose` is a `lib.rs`.
  `PROSE-039` and `PROSE-UI-137` are the two a reviewer holds before a test exists, because a
  second primitive is a second declaration. The four rows of section 1.7 are a pure state machine
  over one struct, which is the shape a property test reaches directly; `PROSE-UI-134` is the one
  row of this document that needs a page drawn, so it waits on the renderer rather than on the
  engine.

---

## 2. WHAT EACH USER SEES

Two peers in one paragraph. No test resolves yet: the last column names the row each row rests on.

| Alice does | Bob does | The document holds | Each user sees | Test |
|---|---|---|---|---|
| types `hello` fast | -- | the five characters, one command each | the text under the caret at once, before the round trip | none -- `PROSE-UI-134` |
| is mid-word when Bob's change arrives | inserts a word earlier in the same paragraph | both edits, in arrival order | Alice's pending letters, shifted past Bob's word, still under her caret | none -- `PROSE-UI-028` |
| is mid-word | deletes the paragraph she is typing in | Bob's deletion | Alice's draft dropped, her caret at the nearest surviving position | none -- `PROSE-UI-143` |
| is composing with an input method | inserts a word earlier | Bob's word alone, until the composition ends | Alice's preview unbroken; the committed text lands where her caret now is | none -- `PROSE-UI-029`, `PROSE-UI-142` |
| selects a sentence and types over it | -- | the deletion, then the insertion, in that order | the sentence replaced | none -- `PROSE-UI-030` |
| presses Backspace at the start of a paragraph | -- | one deletion spanning the boundary | the two paragraphs joined, keeping the first one's style | none -- `PROSE-039`, `PROSE-041` |
| presses Enter mid-paragraph | -- | a new sibling paragraph after the source | the tail on a new line, the caret at its start | none -- `PROSE-038`, `PROSE-UI-141` |

---

## 3. MODEL

**Containers.** None of its own. The text of a paragraph is the `body` text of its block, declared
in `ambre-spec-prose-schema.md section 3`.

**The draft** (`ambre-prose`, UI thread). What was typed and not yet confirmed, held apart from the
document rather than written into a second copy of it. It carries text and an anchor and no
structure (`PROSE-UI-135`):

```rust
pub struct DraftBuffer {
    /// Typed since the last confirming event, drawn over the confirmed runs.
    pub pending_text: String,
    pub block_id:     Option<TreeID>,
    pub insert_pos:   u32,
    /// True while an input-method composition is open; `pending_text` is then the preview.
    pub composing:    bool,
}

impl DraftBuffer {
    pub fn is_empty(&self) -> bool;
    /// Append committed text. Re-anchors only when the draft moves to another block, so
    /// consecutive keystrokes in one block accumulate (`PROSE-UI-138`).
    pub fn push_str(&mut self, text: &str, block_id: &TreeID, pos: u32);
    pub fn begin_composition(&mut self, block_id: TreeID, pos: u32);
    /// Replace the preview text. Canvas only: no command leaves (`PROSE-UI-029`).
    pub fn set_composition(&mut self, text: &str);
    /// End the composition and answer the anchor the insertion must use -- the current one,
    /// which a remote change may have moved since it opened (`PROSE-UI-028`).
    pub fn end_composition(&mut self) -> (TreeID, u32);
    pub fn clear(&mut self);
    /// Carry the draft over a remote change: `shift` is the net offset change applied at or
    /// before `insert_pos` (`PROSE-UI-028`). Answers `false` when the anchored block is gone
    /// (`PROSE-UI-143`).
    pub fn reanchor(&mut self, block_still_alive: bool, shift: i64) -> bool;
}
```

**What clears the draft, and what does not.** One branch, on the delta the confirming event
carries. Anything else erases keystrokes the user is still typing.

| The change that arrived | The draft |
|---|---|
| carries no delta -- a media download, presence, any content-free event | untouched (`PROSE-UI-027`) |
| confirms the anchored block: the local edit came back | cleared, the text is in the confirmed runs (`PROSE-UI-005`) |
| touches other blocks, or this one from another peer | reanchored over the applied shift (`PROSE-UI-028`) |
| deleted the anchored block | dropped, the caret collapses to the confirmed position (`PROSE-UI-143`) |

**The input path.** Printable text is never matched key by key. The shared transparent text field
owns editing focus and normalizes the DOM events (`ambre-spec-text-input.md`); the view
receives them as input events and answers whether the host must swallow the key.

1. The host normalizes a key or a text input and dispatches it to the view.
2. A text input pushes to the draft and emits the insertion command (`PROSE-UI-136`).
3. A composition draws in the draft alone until it ends (`PROSE-UI-142`).
4. Any of the four mutating paths -- typing, Backspace, Delete, Enter -- takes the selection
   first (`PROSE-UI-030`).
5. The caret advances locally, and is revealed (`features/ambre-spec-prose-selection.md`).

```rust
/// Dispatched by the frame host for the key variants. True when the host must swallow the key.
pub fn handle_key(ev: &InputEvent, state: &mut ProseUiState,
                  to_logic: &flume::Sender<LogicCommand>) -> bool;
/// Committed, non-composition text.
pub fn handle_text_input(text: &str, state: &mut ProseUiState,
                         to_logic: &flume::Sender<LogicCommand>);
/// Composition start, update and end. No `web_sys` here: the shared text field captures the
/// DOM events and the host delivers them typed.
pub fn handle_composition(phase: CompositionPhase, text: &str, state: &mut ProseUiState,
                          to_logic: &flume::Sender<LogicCommand>);
/// Delete the active selection and collapse the caret to its start. True when one was
/// consumed; callers that then insert or split act at the collapsed caret (`PROSE-UI-030`).
/// The one primitive of the four editing paths (`PROSE-UI-137`), and it emits the deletion
/// alone (`PROSE-UI-140`).
fn take_selection(state: &mut ProseUiState, to_logic: &flume::Sender<LogicCommand>) -> bool;
```

**The four commands** (`ambre-bus`, declared in `features/ambre-spec-prose-engine.md section 3`):

| Shape | Carries | Rule it serves |
|---|---|---|
| `InsertText { block_id, pos, text }` | one committed run of text | `PROSE-UI-004`, `PROSE-UI-029` |
| `DeleteText { block_id, range }` | a range inside one block | -- |
| `DeleteRange { anchor, focus }` | two endpoints, in either order | `PROSE-037`, `PROSE-039` |
| `SplitBlock { block_id, pos }` | where the paragraph parts | `PROSE-038` |

The channel keeps the order it was sent in, so the deletion of a selection always applies before
the insertion that replaced it. Whether the two are one undo item is undecided
(`features/ambre-spec-prose-undo.md section 4`).

**The join, in one primitive.** A deletion across paragraphs keeps the head block
(`PROSE-041`), appends the tail's surviving suffix to it and removes what it emptied
(`PROSE-037`). Backspace at offset zero
is that deletion with both endpoints at the boundary, so a merge and a cross-paragraph delete are
one code path and one convergence case (`PROSE-039`). Splitting is its inverse (`PROSE-038`,
`PROSE-042`), and it leaves the caret on the new paragraph (`PROSE-UI-141`).

**Code:** none. `crates/ambre-prose/src/lib.rs` is seven lines.

---

## 4. SURFACE

| Trigger | Behaviour |
|---|---|
| a printable key, a dictation, a drag of text into the page | inserts at the caret, drawn from the draft first |
| Backspace | deletes the selection, else the grapheme before the caret, else joins with the previous paragraph |
| Delete | deletes the selection, else the grapheme after the caret, else pulls the next paragraph up |
| Enter | deletes the selection, then splits the paragraph at the caret |
| an input method: start, update, end | preview in the draft, one insertion at the end |
| a key the view does not consume | left to the host, which does not swallow it |

The chords that apply a mark or open a menu are chrome, not text input, and live with the
toolbar and the slash command.

---

## 5. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| A local draft absorbs the round trip | waiting for the confirmation before drawing | it puts the round trip inside every keystroke. The confirmation costs one to three milliseconds, invisible on one keystroke and visible on a fast typist, and the draft holds text rather than structure, so it buys the echo budget without a second model of the document |
| Only a confirming change clears the draft | clearing on any document event | it is the version that loses text: what arrives next is often unrelated -- an image finishing its download, a peer's cursor -- and each would wipe in-progress keystrokes with no way for the user to tell it happened |
| A remote change shifts the draft rather than dropping it | dropping the draft on any remote change | it makes typing feel broken whenever somebody else is present. Two authors in different paragraphs, and two in the same one, are the two common cases and both survive a shift of the anchor; dropping is right only when the paragraph itself is gone |
| No command leaves during a composition | emitting on each composition update | it turns one word into a dozen convergence cases. An input method emits a preview per keystroke and only the last one is the word, so each preview would sit in the document and in every peer's history |
| One primitive deletes the selection | a delete inside each of the four editing paths | typing over a selection and pressing Delete on it are the same operation with a different follow-up. Written four times they drift, and the copies are what forget a corner case |
| A join is a deletion with an empty range | a dedicated merge command | it needs its own convergence proof for a case the deletion already covers. A join is what a cross-paragraph deletion does at its boundary, so the command is a second implementation of the same merge, tested apart and converging apart |
| The page is not an editable DOM element | a content-editable page | its layout and composition behaviour cannot be controlled and cannot be paginated. Letting the browser edit the text gives away what a paragraph is, where it breaks and which page it is on, and the behaviour differs per browser and per input method, so the same document edits differently in two of them |
| The transparent text field owns focus and the input method | reading key events off the canvas | it loses every input method and every mobile keyboard. The browser gives composition, dictation and mobile keyboards to a focused editable element and to nothing else, so the field is not an extra layer but the only way in |

---

## 6. NOT BUILT, AND NOT YET RULED

* **The corpus states the position unit four ways, and they are not the same.** The command plane
  declares a Unicode code point (`features/ambre-spec-prose-engine.md section 3`), the interaction model
  declares a UTF-8 byte offset and cites that same section for it, the shaped run joins glyphs to
  byte ranges (`features/ambre-spec-prose-page-rendering.md section 2`), and `PROSE-FEAT-006`
  requires graphemes wherever a Loro position is addressed. Every document of this area converts
  between the UI representation and the command one, and no document says what the conversion is.
  Deciding it is authorship, not migration: it is recorded here and as a finding.
* **Nothing rules on how many pending characters a draft may hold.** A slow or blocked Logic
  thread grows the string without bound, and no rule caps it or says what happens when the
  confirmation never comes.
* **Nothing rules on an insertion refused by the document.** The engine can reject a command;
  no rule says what the draft does when the confirmation that arrives contradicts what was typed.
  `PROSE-UI-005` rules on the confirming event alone, and section 1.7 has no `FAIL` row because
  no event reports a refusal.
* **Nothing rules on what a split does to a mark it cuts.** `PROSE-038` carries the type and the
  paragraph style to the new paragraph, and says nothing of an inline mark, a comment or a
  footnote anchored across the point the paragraph parts. `PROSE-042` holds the text and leaves
  the marks over it unruled.
* **A protected range is designed and unbuilt, and its limit is the design.** The deferred design
  marks a range with the peers allowed to write it and has the dispatcher refuse a mutation from
  anyone else. The mark key is in no registry (`ambre-spec-prose-schema.md section 3`), and the
  rule it would carry is application-level only: a peer writing to the document directly is not
  refused by anything, because a CRDT has no writer to check. That is a limitation to state to the
  author, not a security boundary, and `ZK-001` is untouched by it. The Story is
  "The later writing set" (`ambre-roadmap.md` POST-V1).
* **A drop of text into the page is named as a trigger and specified nowhere.** No command carries
  it, and the import path it would otherwise take is a document import, not an insertion.
