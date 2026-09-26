# AMBRE -- FEATURE: CLIPBOARD

**Type:** Vertical (feature)
**Implementation:** implemented

> Copy, cut and paste. Copying inside Ambre keeps the formatting and the structure of what was
> copied. Copying out of it leaves readable plain text. Pasting from elsewhere brings the text in.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-UI-013` | `INV` | A paste MUST be routed through the Logic thread as a command: the view MUST NOT insert clipboard content into a model of its own. | this spec section 3 | `ambre_prose::input_tests::a_paste_leaves_as_a_command` | none | verified |
| `PROSE-UI-105` | `INV` | The view MUST NOT serialize document content: the payload MUST be built where the document lives. | this spec section 5 | structural -- the payload type is private to `crates/ambre-prose/src/logic/clipboard.rs`, and the view sends the selection's two ends; `ambre_prose::logic::clipboard::tests::a_copy_covers_the_whole_selection_and_writes_nothing` | none | verified |
| `PROSE-UI-041` | `INV` | The view MUST NOT touch the system clipboard: reading and writing it MUST be the host's, inside the window the user's gesture opens. | this spec section 3; `ambre-spec-clipboard.md` | structural -- `web-sys` is a wasm-only dependency of `crates/ambre-prose` given to the reply field, and no Prose source names the clipboard API, which the host in `crates/ambre-app/src/host/mod.rs` alone reads and writes | none | verified |
| `PROSE-UI-106` | `INV` | No format other than the plain-text mirror and this application's own bytes MUST be read from or written to the system clipboard. | this spec section 5 | structural -- `ambre_bus::ClipboardBlob` holds the plain text and this application's bytes and nothing else, and the host adds only the token that names its copy | none | verified |
| `PROSE-UI-107` | `IO` | Content copied in Ambre and pasted back into Ambre MUST return with the marks and the block structure it was copied with. | this spec section 3 | `ambre_prose::logic::clipboard::tests::a_rich_paste_keeps_types_and_marks` | none | verified |

### 1.2 Copy

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-UI-040` | `POST` | The copied payload MUST be built from the whole selection, never trimmed to what is on screen. | this spec section 3 | `ambre_prose::logic::clipboard::tests::a_copy_covers_the_whole_selection_and_writes_nothing` | none | verified |
| `PROSE-UI-108` | `POST` | The payload MUST carry a plain-text mirror of the selection, readable by an application that knows nothing of Ambre. | this spec section 3 | `ambre_prose::logic::clipboard::tests::a_copy_covers_the_whole_selection_and_writes_nothing` | none | verified |
| `PROSE-UI-109` | `FRAME` | A copy MUST NOT write to the document, and MUST NOT move or shrink the selection it read. | derived | `ambre_prose::logic::clipboard::tests::a_copy_covers_the_whole_selection_and_writes_nothing` | none | verified |

### 1.3 Cut

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-UI-044` | `POST` | A cut MUST delete the source range only after the copy has been asked for. | this spec section 3 | `ambre_prose::input_tests::a_cut_copies_before_it_deletes_the_selection` | none | verified |
| `PROSE-UI-110` | `FAIL` | A cut that produced no payload MUST delete nothing, and MUST leave the document as it was. | this spec section 3 | `ambre_prose::input_tests::a_refused_copy_deletes_nothing` | none | verified |
| `PROSE-UI-111` | `FRAME` | A cut MUST NOT delete outside the source range, and MUST NOT change the formatting of the text that remains around it. | derived | `ambre_prose::input_tests::a_cut_copies_before_it_deletes_the_selection` | none | verified |

### 1.4 Paste

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-UI-042` | `POST` | A paste MUST have exactly two outcomes: the rich payload, or the plain-text mirror. | this spec section 3 | `ambre_prose::logic::clipboard::tests::foreign_bytes_paste_as_plain_text` | none | verified |
| `PROSE-UI-043` | `POST` | The kept payload MUST be used only while the system clipboard's text still matches it; otherwise the pasted content MUST come from the system clipboard. | this spec section 3 | `ambre_prose::input_tests::the_kept_payload_serves_only_its_own_copy` | none | verified |
| `PROSE-UI-112` | `FRAME` | A paste MUST NOT change a block outside the range it replaces, and MUST NOT reformat the text that surrounds it. | derived | `ambre_prose::logic::clipboard::tests::a_paste_mid_paragraph_leaves_the_neighbours_alone` | none | verified |

* **Budget.** none: a copy walks the selection once, a paste is an ordinary insertion, and
  `ambre-spec-budgets.md` budgets neither. What a very large paste costs is in section 6.
* **Boundary.** the system clipboard leaves the application, and it is the one path here that does.
  It carries the user's own selection, already plaintext on their machine; nothing is encrypted,
  nothing is sent, and the host performs the write (`PROSE-UI-041`). The browser's own handling is
  outside Ambre's control, and gates the mirror (`ambre-spec-clipboard.md`).
* **Verification.** 14 `verified`. The Logic rows are host tests over a real document, a round
  trip included; the view rows drive the input queue with a recording sender. `PROSE-UI-041`,
  `PROSE-UI-105` and `PROSE-UI-106` are held by where the types live.

---

## 2. WHAT EACH USER SEES

Two peers, one selection. The last column names the row each line rests on.

| Alice does | Bob does | The document holds | Each user sees | Test |
|---|---|---|---|---|
| copies three formatted paragraphs, then pastes them lower down | -- | the three paragraphs again, with their formatting | the same text twice, styles kept | none -- `PROSE-UI-107` |
| copies a selection larger than the window | -- | nothing yet | the whole selection on the clipboard, not the visible part | none -- `PROSE-UI-040` |
| cuts a sentence | edits the same sentence at that moment | the deletion and Bob's edit, merged | the sentence gone on both screens, Bob's edit kept where it survived | none -- `PROSE-UI-044` |
| pastes into a text editor outside Ambre | -- | nothing | the plain text of what she copied | none -- `PROSE-UI-108` |
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

1. The view asks the Logic thread for the payload (`PROSE-UI-040`); it serializes nothing
   (`PROSE-UI-105`).
2. The Logic thread walks the selection in the document and answers with the two halves.
3. The host writes both to the system clipboard on the next frame (`PROSE-UI-041`). The view
   keeps the payload in memory for a paste back into the same tab.

```rust
/// Ask for the payload. `cut` deletes the source range after the copy (`PROSE-UI-044`).
pub fn clipboard_write(&mut self, cut: bool) -> CopyOutcome;
/// The host calls this after each update: the payload to mirror, when one arrived.
pub fn poll_clipboard(&mut self) -> Option<ClipboardBlob>;
/// A paste, with what the host read. `own_copy` says the host's token names this tab's copy.
pub fn paste_clipboard(&mut self, blob: ClipboardBlob, own_copy: bool);
```

**The two outcomes of a paste** (`PROSE-UI-042`):

| What the host read | What is inserted |
|---|---|
| a payload this application wrote, and the mirror still matches it | the rich content, through `PasteRich` (`PROSE-UI-043`) |
| anything else, including text from another application | the plain text, through `PasteText`: one paragraph per line, control characters dropped |

Both leave as commands, so the document is written by the Logic thread and the view keeps no
model of what was pasted (`PROSE-UI-013`). Opening a Word or Markdown *file* is a document
import, not a paste (`features/ambre-spec-prose-import-export.md`).

**Why the mirror is checked.** The kept payload is the only full-fidelity source, and it stays
valid only while nothing else has taken the clipboard. Comparing it against the plain text the
host read is what tells the two apart. A copy in another application replaces the text, and the
kept payload is then stale (`PROSE-UI-043`).

**Cut.** A cut is a copy and then a deletion of the source range, in that order and never the
other one (`PROSE-UI-044`). A copy that produced nothing deletes nothing (`PROSE-UI-110`). The
deletion is the ordinary cross-paragraph one
(`features/ambre-spec-prose-text-editing.md section 3`).

A payload opens with four bytes of its own, so bytes another Ambre application wrote under the
same clipboard type paste as their plain text. Only the formatting marks travel: a comment or a
suggestion anchor stays in the document it was copied from. A paste of several blocks into a band
or a note is refused, as a split there is. A paste of one line with no payload is typing, and
joins the draft.

**Code:** `crates/ambre-prose/src/logic/clipboard.rs` (the payload, the copy and the paste),
`crates/ambre-prose/src/input.rs` (the copy, cut and paste inputs), and
`crates/ambre-app/src/host/prose.rs` (the host entries).

---

## 4. SURFACE

| Trigger | Behaviour |
|---|---|
| the copy chord, or the menu entry | asks for the payload and mirrors it |
| the cut chord | the same, then deletes the selection |
| the paste chord | inserts the rich payload, or the plain text |
| a paste with no selection | inserts at the caret |
| a paste over a selection | deletes it first (`PROSE-UI-030`) |
| dropping text into the page | `PROSE-UI-204`, on the "A drag that scrolls, and text dropped into the page" Story (`ambre-roadmap.md` E5) |

The legacy chords and the fold of `Ctrl` and `Cmd` are decided for every application in
`1-Foundations/ambre-spec-commands.md section 2`. Prose takes the shared clipboard identities and
their chord lists, and decodes none of the three itself.

---

## 5. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| The Logic thread builds the payload | serializing the selection in the view | it requires the view to hold the document, which is the one thing `PROSE-UI-001` forbids. The document lives on the Logic thread, and that is the only place the whole selection can be read |
| The payload covers the selection, never the window | building the payload from the display list | the display list is culled by construction, so a user selecting forty pages gets what was built for the frame. The loss is invisible until the paste, and then it is silent |
| The host owns the system clipboard | calling the clipboard from the view | browsers grant clipboard access only inside the window a user gesture opens, and that window is the host's event handler. A worker or a view that asks later is refused, and the refusal cannot be retried or reported usefully |
| The clipboard carries plain text and this application's own bytes, and nothing else | a markup payload beside the two | it doubles the write and adds an untrusted parse. A third format is a third writer and a third reader while the rich half already carries everything, and markup invites a paste path that sanitizes untrusted content on the UI thread, which is where it must never happen |
| The kept payload is gated on the mirror | pasting the kept payload whenever there is one | it pastes what the user copied ten minutes ago. The payload in memory is not what the clipboard holds, since another application may have replaced it, and the plain-text mirror is the one field both sides wrote and can compare |
| A cut copies first | deleting and then copying | it loses the content if the copy is refused, with nothing on the clipboard to show for it. The other order makes the failure harmless |
| Importing a file is not a paste | routing a file drop through the paste path | it puts an untrusted document parse on a gesture that has no report. A dropped or opened document needs a parser, a report of what was lost, and a progress path for a large file; a paste has none of those and must not grow them |

---

## 6. NOT BUILT, AND NOT YET RULED

* **A paste from another Ambre application is not ruled on.** The rich half carries this
  application's own shape, so a paste from the whiteboard or the spreadsheet falls back to plain
  text. Making that structure survive is the
  "Cross-app rich paste" Story (`ambre-roadmap.md` POST-V1).
* **Nothing rules on the size of a payload.** A copy of a whole document builds it in memory
  twice, once as text and once as bytes. No rule caps it or says what happens when the system
  clipboard refuses it.
* **Nothing rules on a paste that carries images.** The rich payload is described as runs and
  block structure, and an image is a reference to a blob the receiving document may not have.
* **A dropped file has no design.** Dropped text is `PROSE-UI-204`; a dropped document is an
  import (`features/ambre-spec-prose-import-export.md`).
