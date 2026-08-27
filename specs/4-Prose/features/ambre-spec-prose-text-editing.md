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

| ID | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|
| `PROSE-UI-004` | Keystrokes MUST be displayed optimistically from a local draft, never by mutating shared state on the UI side. | this spec section 3 | not built -- lands with the "Input and cursor" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-UI-005` | The draft MUST be discarded when the confirming event arrives, and MUST NOT survive a divergence between what was typed and what the document accepted. | this spec section 3 | not built -- lands with the "Input and cursor" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-UI-027` | An event carrying no change MUST leave the draft untouched: a download or a presence update MUST NOT erase what is being typed. | this spec section 3 | not built -- lands with the "Input and cursor" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-UI-028` | A remote change MUST shift the pending draft by what it applied before the draft position, and MUST drop the draft only when the paragraph it sits in is deleted. | this spec section 3 | not built -- lands with the "Input and cursor" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-UI-029` | While a composition is open the view MUST send no text to the document; the composed text MUST reach it once, when the composition ends, at the position the draft holds then. | this spec section 3 | not built -- lands with the "Toolbar, slash menu and composition" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-UI-030` | An edit that inserts or splits MUST delete the active selection first, through the one primitive both paths share. | this spec section 3 | not built -- lands with the "Ruler, clipboard and inertial scroll" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-037` | A deletion spanning two paragraphs MUST keep the first one's identity and paragraph style, append what survives of the last to it, and remove every paragraph it emptied. | this spec section 3 | not built -- lands with `ARCH P2` (Epic `E5`) | unimplemented |
| `PROSE-038` | Splitting a paragraph MUST give the new one the source's type and paragraph style, and MUST leave the caret at its start. | this spec section 3 | not built -- lands with `ARCH P2` (Epic `E5`) | unimplemented |
| `PROSE-039` | Joining two paragraphs MUST be expressed as the deletion above with an empty range; a second join operation MUST NOT exist. | this spec section 3 | not built -- lands with `ARCH P2` (Epic `E5`) | unimplemented |

* **Budget.** the keystroke to a local echo and the keystroke to a confirmed one, two rows of
  `ambre-spec-budgets.md section 1`. The first is what the draft exists to hold.
* **Boundary.** none: the draft is view state on the UI thread, where `UI-THREAD-004` bans a
  network, crypto or persistence path, and every mutation leaves as a command (`PROSE-UI-002`,
  `features/ambre-spec-prose-engine.md section 1`).
* **Verification.** 9 `unimplemented`, nothing else: `crates/ambre-prose` is a `lib.rs`.
  `PROSE-039` is the row a reviewer can hold before a test exists, because a second join primitive
  is a second command variant. The three draft rows are pure state machine over one struct, which
  is the shape a property test reaches directly.

---

## 2. WHAT EACH USER SEES

Two peers in one paragraph. No test resolves yet: the last column names the row each row rests on.

| Alice does | Bob does | The document holds | Each user sees | Test |
|---|---|---|---|---|
| types `hello` fast | -- | the five characters, one command each | the text under the caret at once, before the round trip | none -- `PROSE-UI-004` |
| is mid-word when Bob's change arrives | inserts a word earlier in the same paragraph | both edits, in arrival order | Alice's pending letters, shifted past Bob's word, still under her caret | none -- `PROSE-UI-028` |
| is mid-word | deletes the paragraph she is typing in | Bob's deletion | Alice's draft dropped, her caret at the nearest surviving position | none -- `PROSE-UI-028` |
| is composing with an input method | inserts a word earlier | Bob's word alone, until the composition ends | Alice's preview unbroken; the committed text lands where her caret now is | none -- `PROSE-UI-029` |
| selects a sentence and types over it | -- | the deletion, then the insertion, in that order | the sentence replaced | none -- `PROSE-UI-030` |
| presses Backspace at the start of a paragraph | -- | one deletion spanning the boundary | the two paragraphs joined, keeping the first one's style | none -- `PROSE-037`, `PROSE-039` |
| presses Enter mid-paragraph | -- | a new sibling paragraph after the source | the tail on a new line, the caret at its start | none -- `PROSE-038` |

---

## 3. MODEL

**Containers.** None of its own. The text of a paragraph is the `body` text of its block, declared
in `ambre-spec-prose-schema.md section 3`.

**The draft** (`ambre-prose`, UI thread). What was typed and not yet confirmed, held apart from the
document rather than written into a second copy of it:

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
    /// consecutive keystrokes in one block accumulate.
    pub fn push_str(&mut self, text: &str, block_id: &TreeID, pos: u32);
    pub fn begin_composition(&mut self, block_id: TreeID, pos: u32);
    /// Replace the preview text. Canvas only: no command leaves (`PROSE-UI-029`).
    pub fn set_composition(&mut self, text: &str);
    /// End the composition and answer the anchor the insertion must use -- the current one,
    /// which a remote change may have moved since it opened (`PROSE-UI-028`).
    pub fn end_composition(&mut self) -> (TreeID, u32);
    pub fn clear(&mut self);
    /// Carry the draft over a remote change: `shift` is the net offset change applied at or
    /// before `insert_pos`. Answers `false` when the anchored block is gone (`PROSE-UI-028`).
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
| deleted the anchored block | dropped, the caret collapses to the confirmed position |

**The input path.** Printable text is never matched key by key. The shared transparent text field
owns editing focus and normalizes the DOM events (`ambre-spec-text-input.md`); the view
receives them as input events and answers whether the host must swallow the key.

1. The host normalizes a key or a text input and dispatches it to the view.
2. A text input pushes to the draft (`PROSE-UI-004`) and emits the insertion command.
3. A composition draws in the draft alone until it ends (`PROSE-UI-029`).
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

**The join, in one primitive.** A deletion across paragraphs keeps the head block, appends the
tail's surviving suffix to it and removes what it emptied (`PROSE-037`). Backspace at offset zero
is that deletion with both endpoints at the boundary, so a merge and a cross-paragraph delete are
one code path and one convergence case (`PROSE-039`). Splitting is its inverse (`PROSE-038`).

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

## 5. DECISIONS

| Decision | Rationale | Rejected alternative |
|---|---|---|
| A local draft absorbs the round trip | the confirmation costs one to three milliseconds, which is invisible on one keystroke and visible on a fast typist. Drawing the letters immediately keeps the echo under the budget without a second model of the document: the draft holds text, not structure, and it is thrown away the moment the document answers | waiting for the confirmation before drawing, which puts the round trip inside every keystroke |
| Only a confirming change clears the draft | the draft is cleared by whatever arrives next, and what arrives next is often unrelated -- an image finishing its download, a peer's cursor. Each of those would wipe in-progress keystrokes, and the user has no way to tell it happened | clearing on any document event, which is the version that loses text |
| A remote change shifts the draft rather than dropping it | the common case of collaboration is two authors in different paragraphs, and the second most common is two in the same one. Both are survivable by moving the anchor over the applied shift; dropping is right only when the paragraph itself is gone | dropping the draft on any remote change, which makes typing feel broken whenever somebody else is present |
| No command leaves during a composition | an input method emits a preview per keystroke and only the last one is the word. Sending each preview would put a dozen operations in the document and in every peer's history for one word | emitting on each composition update, which turns one word into a dozen convergence cases |
| One primitive deletes the selection | typing over a selection and pressing Delete on it are the same operation with a different follow-up. Written twice they drift, and the second copy is the one that forgets a corner case | a delete inside each of the four editing paths |
| A join is a deletion with an empty range | a join is what a cross-paragraph deletion already does at its boundary, so a separate command would be a second implementation of the same merge, tested apart and converging apart | a dedicated merge command, which needs its own convergence proof for a case the deletion already covers |
| The page is not an editable DOM element | letting the browser edit the text gives away every decision this application has to make: what a paragraph is, where it breaks, which page it is on. The behaviour also differs per browser and per input method, so the same document edits differently in two of them | a content-editable page, whose layout and composition behaviour cannot be controlled and cannot be paginated |
| The transparent text field owns focus and the input method | the browser gives composition, dictation and mobile keyboards to a focused editable element and to nothing else. A canvas cannot receive them at all, so the field is not an extra layer but the only way in | reading key events off the canvas, which loses every input method and every mobile keyboard |

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
  no rule says what the draft does when the confirmation that arrives contradicts it, which is
  the divergence half of `PROSE-UI-005`.
* **A protected range is designed and unbuilt, and its limit is the design.** The deferred design
  marks a range with the peers allowed to write it and has the dispatcher refuse a mutation from
  anyone else. The mark key is in no registry (`ambre-spec-prose-schema.md section 3`), and the
  rule it would carry is application-level only: a peer writing to the document directly is not
  refused by anything, because a CRDT has no writer to check. That is a limitation to state to the
  author, not a security boundary, and `ZK-001` is untouched by it. The Story is "The later
  writing set" (`ambre-roadmap.md`).
* **A drop of text into the page is named as a trigger and specified nowhere.** No command carries
  it, and the import path it would otherwise take is a document import, not an insertion.
