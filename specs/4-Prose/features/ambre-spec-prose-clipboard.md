# AMBRE -- FEATURE: CLIPBOARD

**Type:** Vertical (feature)
**Implementation:** design-only

> Copy, cut and paste. Copying inside Ambre keeps the formatting and the structure of what was
> copied; copying out of it leaves readable plain text; pasting from elsewhere brings the text in.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`.

| ID | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|
| `PROSE-UI-013` | A paste MUST be routed through the Logic thread as a command: the view MUST NOT insert clipboard content into a model of its own. | this spec section 3 | not built -- lands with the "Ruler, clipboard and inertial scroll" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-UI-040` | The copied payload MUST be built by the Logic thread from the whole selection; the view MUST NOT serialize document content and MUST NOT trim the payload to what is on screen. | this spec section 3 | not built -- lands with the "Ruler, clipboard and inertial scroll" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-UI-041` | The view MUST NOT touch the system clipboard: reading and writing it MUST be the host's, inside the window the user's gesture opens. | this spec section 3; `ambre-spec-clipboard.md` | not built -- lands with the "Ruler, clipboard and inertial scroll" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-UI-042` | A paste MUST have exactly two outcomes, the rich payload or the plain-text mirror; no third format MUST be read from the system clipboard. | this spec section 3 | not built -- lands with the "Ruler, clipboard and inertial scroll" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-UI-043` | The kept payload MUST be used only while the system clipboard's text still matches it; otherwise the pasted content MUST come from the system clipboard. | this spec section 3 | not built -- lands with the "Ruler, clipboard and inertial scroll" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-UI-044` | A cut MUST delete the source range only after the copy has been asked for, and MUST NOT delete when no payload was produced. | this spec section 3 | not built -- lands with the "Ruler, clipboard and inertial scroll" Story (`ambre-roadmap.md` E5) | unimplemented |

* **Budget.** none: a copy walks the selection once and a paste is an ordinary insertion, and
  `ambre-spec-budgets.md` budgets neither. What a very large paste costs is named
  in section 6.
* **Boundary.** the system clipboard leaves the application, and it is the one path here that
  does. What is written to it is the user's own selection, already plaintext on their machine;
  nothing is encrypted, nothing is sent, and the host performs the write (`PROSE-UI-041`). What
  the browser then does with the clipboard is outside Ambre's control and is the reason the
  mirror is gated (`ambre-spec-clipboard.md`).
* **Verification.** 6 `unimplemented`, nothing else: `crates/ambre-prose` is a `lib.rs`.
  `PROSE-UI-041` is compiler-held the day the crate exists, because the browser clipboard type
  cannot be named from a crate that does not depend on the web bindings. `PROSE-UI-042` is the row
  a test reaches with two fixtures and no browser: a payload with rich bytes and one without.

---

## 2. WHAT EACH USER SEES

Two peers, one selection. No test resolves yet: the last column names the row each row rests on.

| Alice does | Bob does | The document holds | Each user sees | Test |
|---|---|---|---|---|
| copies three formatted paragraphs, then pastes them lower down | -- | the three paragraphs again, with their formatting | the same text twice, styles kept | none -- `PROSE-UI-040` |
| copies a selection larger than the window | -- | nothing yet | the whole selection on the clipboard, not the visible part | none -- `PROSE-UI-040` |
| cuts a sentence | edits the same sentence at that moment | the deletion and Bob's edit, merged | the sentence gone on both screens, Bob's edit kept where it survived | none -- `PROSE-UI-044` |
| pastes into a text editor outside Ambre | -- | nothing | the plain text of what she copied | none -- `PROSE-UI-042` |
| copies in Ambre, copies again in another application, pastes back | -- | the second copy's text | the other application's text, not the earlier rich payload | none -- `PROSE-UI-043` |
| pastes a paragraph from a web page | -- | its text, inserted at the caret | plain text, without the page's formatting | none -- `PROSE-UI-042` |

---

## 3. MODEL

**Containers.** None. A paste writes through the ordinary editing commands, and a copy reads.

**The shared model.** Prose uses the clipboard model of `ambre-spec-clipboard.md`, which is
host-driven and authoritative in the Logic thread. The transport is the application-agnostic
payload of `ambre-bus`:

```rust
/// What crosses to the host, and back. `text` is the plain-text mirror every other
/// application can read; `ambre` is this application's own bytes, under its private type.
pub struct ClipboardBlob { pub text: String, pub ambre: Option<Vec<u8>> }
```

**The three steps of a copy.**

1. The view asks the Logic thread for the payload (`PROSE-UI-040`); it serializes nothing.
2. The Logic thread walks the selection in the document and answers with the two halves.
3. The host writes both to the system clipboard on the next frame (`PROSE-UI-041`), and the view
   keeps the payload in memory for a paste back into the same tab.

```rust
/// Ask for the payload. `cut` deletes the source range after the copy (`PROSE-UI-044`).
pub fn clipboard_write(&mut self, cut: bool) -> CopyOutcome;
/// The host calls this after each update: the payload to mirror, when one arrived.
pub fn poll_clipboard(&mut self) -> Option<ClipboardBlob>;
/// A paste, with what the host read from the system clipboard.
pub fn paste_clipboard(&mut self, blob: ClipboardBlob);

/// The payload kept for a paste back into the same tab.
struct OwnClipboard { text: String, payload: ProseClipboardPayload }
```

**The two outcomes of a paste** (`PROSE-UI-042`):

| What the host read | What is inserted |
|---|---|
| a payload this application wrote, and the mirror still matches it | the rich content, through `PasteRich` (`PROSE-UI-043`) |
| anything else, including text from another application | the plain text, through `InsertText` |

Both leave as commands, so the document is written by the Logic thread and the view keeps no
model of what was pasted (`PROSE-UI-013`). Opening a Word or Markdown *file* is a document
import, not a paste (`features/ambre-spec-prose-import-export.md`).

**Why the mirror is checked.** The kept payload is the only full-fidelity source, and it stays
valid only while nothing else has taken the clipboard. Comparing it against the plain text the
host read is what tells the two apart: a copy in another application replaces the text, and the
kept payload is then stale (`PROSE-UI-043`).

**Cut.** A cut is a copy and then a deletion of the source range, in that order and never the
other one (`PROSE-UI-044`). The deletion is the ordinary cross-paragraph one
(`features/ambre-spec-prose-text-editing.md section 3`).

**Code:** none. `crates/ambre-prose/src/lib.rs` is seven lines.

---

## 4. SURFACE

| Trigger | Behaviour |
|---|---|
| the copy chord, or the menu entry | asks for the payload and mirrors it |
| the cut chord | the same, then deletes the selection |
| the paste chord | inserts the rich payload, or the plain text |
| a paste with no selection | inserts at the caret |
| a paste over a selection | deletes it first (`PROSE-UI-030`) |
| dropping text into the page | not ruled on: see section 6 |

The legacy chords and the fold of `Ctrl` and `Cmd` are one decision for every application, and it
is the "One keymap" Story.

---

## 5. DECISIONS

| Decision | Rationale | Rejected alternative |
|---|---|---|
| The Logic thread builds the payload | the document lives there, and it is the only place the whole selection can be read. Building it in the view would need a second copy of the content on the UI side, which is the one thing `PROSE-UI-001` forbids | serializing the selection in the view, which requires the view to hold the document |
| The payload covers the selection, never the window | a user selecting forty pages and copying them expects forty pages. Trimming to what was built for the frame is invisible until the paste, and then the loss is silent | building the payload from the display list, which is culled by construction |
| The host owns the system clipboard | browsers grant clipboard access only inside the window a user gesture opens, and that window is the host's event handler. A worker or a view that asks later is refused, and the refusal is not reportable to the user in any useful way | calling the clipboard from the view, which fails outside the gesture and cannot be retried |
| The clipboard carries plain text and this application's own bytes, and nothing else | a third format is a third writer and a third reader, and the rich half already carries everything. Markup on the clipboard also invites a paste path that sanitizes untrusted content on the UI thread, which is where it must never happen | a markup payload beside the two, which doubles the write and adds an untrusted parse |
| The kept payload is gated on the mirror | the payload in memory is not what the clipboard holds -- another application may have replaced it. The plain-text mirror is the one thing both sides can compare, and it is exactly the field the other application also wrote | pasting the kept payload whenever there is one, which pastes what the user copied ten minutes ago |
| A cut copies first | a deletion before a failed copy loses the text with nothing on the clipboard to show for it. Ordering it the other way makes the failure harmless | deleting and then copying, which loses the content if the copy is refused |
| Importing a file is not a paste | a dropped or opened document is untrusted bytes that need a parser, a report of what was lost, and a progress path for a large file. A paste has none of those and must not grow them | routing a file drop through the paste path, which puts an untrusted document parse on a gesture that has no report |

---

## 6. NOT BUILT, AND NOT YET RULED

* **A paste from another Ambre application is not ruled on.** The rich half carries this
  application's own shape, so a paste from the whiteboard or the spreadsheet falls back to plain
  text. Making that structure survive is the "Cross-app rich paste" Story.
* **Nothing rules on the size of a payload.** A copy of a whole document builds it in memory
  twice, once as text and once as bytes, and no rule caps it or says what happens when the system
  clipboard refuses it.
* **Nothing rules on a paste that carries images.** The rich payload is described as runs and
  block structure, and an image is a reference to a blob the receiving document may not have.
* **Dropping text or a file into the page has no design.** It is named as a trigger in this area
  and no command carries it.
* **Copying a selection that spans a comment or a suggestion is not ruled on.** Whether the
  annotation travels, and what its identity becomes in the receiving document, is undecided.
