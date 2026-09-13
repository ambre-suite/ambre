# AMBRE -- FEATURE: THE CLIPBOARD

**Type:** Vertical (feature)
**Implementation:** implemented

> A user selects a thousand rows, scrolls past them and copies. What lands on the clipboard is the
> whole selection, not the part that was on screen. Pasted formulas point at their new neighbours.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Gap | Status |
|---|---|---|---|---|---|---|
| `TRAME-UI-124` | `CONC` | Two peers MUST converge when one pastes a block over cells the other is editing. | this spec section 2 | none | unheld -- `ambre_trame::logic::convergence_tests::two_peers_converge_under_interleaved_ops` interleaves fourteen operations over two peers, and its operation set holds neither the paste nor the batch clear a cut sends: the gestures this document owns are the ones the property never generates | unverified |

### 1.2 Copy the selection

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Gap | Status |
|---|---|---|---|---|---|---|
| `TRAME-UI-059` | `POST` | A copy MUST cover the whole selection, never only the cells the viewport holds. | this spec section 3 | `ambre_trame::logic::tests::group7::copy_range_covers_the_full_selection_not_a_viewport` -- it spans five hundred rows and asserts the far cell is in the payload | none | verified |
| `TRAME-UI-060` | `FRAME` | A copy MUST NOT change the document. | this spec section 3 | `ambre_trame::logic::tests::group7::copy_range_covers_the_full_selection_not_a_viewport` -- it asserts the outcome carries no delta; type-level -- `copy_range` takes `&self` and returns `TrameOutcome::display_only` (`logic/clipboard.rs`) | none | verified |

### 1.3 Cut the selection

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Gap | Status |
|---|---|---|---|---|---|---|
| `TRAME-UI-064` | `POST` | A cut MUST clear its source only once the paste lands. | this spec section 4 | none | unheld -- `finish_paste` is the only site that clears, and it runs after the paste command is sent, and nothing drives a cut end to end | unverified |
| `TRAME-UI-125` | `FRAME` | A cut MUST NOT clear its source when the paste did not reach the document. | this spec section 5; derived | none | fails -- `grid_paste_payload` logs a full channel, drops the paste, and calls `finish_paste` anyway, which sends the source clear as a lossless batch. The gesture the decision was built to make safe destroys the range in exactly that case, and it is the shape `BUS-018` bans. Answered by the `A lost edit is never shown as applied` Story (`ambre-roadmap.md` E7) | unverified |

### 1.4 Paste

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Gap | Status |
|---|---|---|---|---|---|---|
| `TRAME-UI-061` | `POST` | A pasted formula MUST have its relative references shifted by the paste offset. | this spec section 3 | `ambre_trame::logic::tests::group5::paste_reindexes_relative_formula` | none | verified |
| `TRAME-UI-063` | `POST` | A paste MUST be one undo item, whatever the size of the range. | this spec section 3 | none | unheld -- `paste_range` opens one undo group and pre-materializes every cell so the writes collapse into one commit, and the paste test asserts the values, never the undo | unverified |
| `TRAME-UI-065` | `POST` | A paste MUST prefer this tab's own copy while the clipboard still carries its ownership token. | this spec section 3 | none | unheld -- the token is written synchronously by the host and read back into `own_copy`, and the ladder below it has no test | unverified |
| `TRAME-UI-062` | `FRAME` | A blank source cell MUST leave its destination untouched. | this spec section 3 | `ambre_trame::logic::tests::group7::copy_range_covers_the_full_selection_not_a_viewport` -- it counts exactly two cells in a five-hundred-row payload, so the copy side omits the blanks | partial -- the paste side is not asserted: no test pastes over occupied cells and reads back what the blanks did not overwrite | unverified |

* **Budget.** none: a copy serialises the selection once, and
  `ambre-spec-budgets.md` budgets no clipboard operation.
* **Boundary.** the OS clipboard carries the selection out of the application, in plaintext, by the
  user's own gesture. `ambre-spec-clipboard.md` fixes what it may hold and the window it may be
  written in. Nothing here reaches the relay or holds a key.
* **Verification.** 3 `verified`, 6 `unverified`. The three that hold are the three that live in
  the Logic thread, where a test can reach them, and one of them is stronger than it was written:
  the copy test asserts the empty delta, so `TRAME-UI-060` no longer rests on the signature alone.
  Everything above the seam -- the ownership token and the cut -- is browser state, and the gap is
  the same one the edit session has. Reading the cut end to end found the decision of section 5
  undone by the code: the source clear is sent whether or not the paste was, which is the one path
  in this document that loses data outright.

---

## 2. WHAT EACH USER SEES

| A does | B does | Converged document | What each user sees | Test |
|---|---|---|---|---|
| pastes a block over `B2:C3` | types into `B2` | one value per cell, elected by LWW | the same grid; the later write wins per cell | none. The paste writes cells like any edit, so it converges for the same reason, but `Paste` is not in the convergence property's op set (`TRAME-UI-124`) |
| cuts `A1:A9` and pastes at `C1` | types into `A5` | A's clear and B's text race on `A5` | the value may survive the cut or vanish, by LWW | none, and no rule covers it -- section 6 |
| copies `A1:A9` | deletes row 3 | unchanged by the copy | A's clipboard still holds the row B deleted; pasting it back recreates the values | none |

---

## 3. MODEL

**Containers.** None of its own. A paste writes `cells_{sheet}` and `cellstyle_{sheet}`, both
declared in `ambre-spec-trame-schema.md section 3`.

**Commands and events** (`ambre-bus`):

| Shape | Direction | Carries |
|---|---|---|
| `TrameCommand::CopyRange` | UI to Logic | nothing. The Logic thread resolves the selection it already tracks |
| `TrameEvent::Clipboard(ClipboardPayload)` | Logic to UI | the source origin, the extent, and one entry per non-blank cell: raw text, bold, italic, style |
| `TrameCommand::PasteRange { target_row, target_col, payload }` | UI to Logic | the landing cell and the typed payload |
| `TrameCommand::SetCellsBatch { cells }` | UI to Logic | the source clear of a cut, as one undo item |

**The transport** is the app-agnostic `ClipboardBlob { text, ambre }` of `ambre-bus`, specified in
`ambre-spec-clipboard.md`: `text` is the plain-text mirror, `ambre` the postcard of the payload
above. This document states what Trame puts in it, never how the seam works.

**The paste fidelity ladder**, first match wins:

1. an open edit session takes the plain text to the caret;
2. this tab's in-memory copy, while the clipboard still carries its ownership token;
3. the decoded `ambre` payload of the blob;
4. the plain-text mirror, split on tabs and newlines into literal cells.

**Code.**

* `logic/clipboard.rs` -- the authoritative copy, the paste, the reference shift, the style carry;
* `ui/view/interact.rs` -- `clipboard_write`, `poll_clipboard`, `paste_clipboard`, `finish_paste`;
* `crates/ambre-app/src/host/mod.rs` -- the OS clipboard, the MIME types and the ownership token.

---

## 4. SURFACE

| Gesture | Effect |
|---|---|
| `Ctrl+C`, `Ctrl+X`, `Ctrl+V` | ride the native clipboard events; they are not decoded as key chords |
| `Ctrl+Insert`, `Shift+Insert`, `Shift+Delete` | the legacy set, mapped to the same three intents (`ambre-spec-commands.md`, `UI-THREAD-011`) |
| the context menu | `Cut`, `Copy` and `Paste`, cell scope only (`features/ambre-spec-trame-context-menu.md section 3`) |
| `Escape` | drops the marching ants and cancels a pending cut |

**Inside an edit session** the selection of the text wins: a copy returns the selected substring and
a cut deletes it. The grid range is only consulted in navigation mode.

**Rendering.**

| What | Drawn as |
|---|---|
| the copied range | an animated dashed outline, until Escape or the next copy |
| the pasted range | selected as a whole, anchor at the landing cell |
| a refused paste | a toast, raised from `PasteRejected`, which nothing constructs (section 6) |

---

## 5. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| Copy is built in the Logic thread | building the payload from the UI cache | it is what a viewport-limited copy does: the UI holds one viewport, so a copy built there would silently truncate at the edge of what was scrolled. The Logic thread holds the document, and `Ctrl+A` then copy has to mean the whole selection. |
| The OS write happens one frame later | preparing a payload ahead of every copy | the payload does not exist yet when the key fires, so preparing one ahead means preparing it for every selection change on the chance a copy follows. Rejected in `ambre-spec-clipboard.md`. |
| The OS write happens one frame later | holding a promise in the clipboard item | the async clipboard API needs the user activation the key opened, and a promise outlives it. One frame is inside that window. Rejected in `ambre-spec-clipboard.md`. |
| A cut clears its source on the paste | clearing at cut time | it is what a delete already does: a cut that clears at once and is never pasted destroys data with no gesture to blame. |
| The typed payload is opaque bytes under a private MIME type | one interchange format shared by every app | it is the cross-app follow-up, not this. Each app owns its own shape, and the plain-text mirror is what crosses to other applications. |
| A pasted formula shifts, a pasted literal does not | keeping the source references, matching an absolute-only model | Ambre has no absolute reference syntax, so every reference is relative and a paste that did not shift would break every formula it moved. |

---

## 6. NOT BUILT, AND NOT YET RULED

* **The module header contradicts its own file.** `logic/clipboard.rs` opens with "copy and cut are
  pure UI-thread operations", and `copy_range` sits forty lines below it. `TRAME-UI-059` is the rule
  that says which one is true.
* **A cut across two peers.** The clear is a separate command from the paste, so a cut is two undo
  items and a concurrent edit inside the source range races with the clear. No rule covers either.
* **A paste that lands past the grid.** The destination is clamped for the selection but nothing
  refuses a payload whose extent runs past the last row or column.
* **No paste is ever refused.** `TrameEvent::PasteRejected` is declared in `ambre-bus` and matched
  twice in the chrome, and no site in the workspace constructs it: the toast section 4 draws has no
  producer, so a paste that cannot land says nothing.
* **A dropped paste still clears the cut** (`TRAME-UI-125`). The paste and the source clear are two
  commands, and only the second one is guaranteed to be sent. `BUS-018` is the shared rule and it is
  `unimplemented`, so nothing here holds the range.
* **A cut leaves the formatting of its source behind.** The clear writes empty text through
  `SetCellsBatch` and touches neither the style register nor the bold and italic sets, so the source
  keeps its formatting while a copy of a blank-but-styled cell carries none of it. No rule says
  which of the two is right.
