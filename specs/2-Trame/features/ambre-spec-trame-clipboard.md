# AMBRE -- FEATURE: THE CLIPBOARD

**Type:** Vertical (feature)
**Implementation:** implemented

> A user selects a thousand rows, scrolls past them and copies. What lands on the clipboard is the
> whole selection, not the part that was on screen. Pasted formulas point at their new neighbours.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`.

| ID | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|
| `TRAME-UI-059` | A copy MUST cover the whole selection, never only the cells the viewport holds. | this spec section 3 | `ambre_trame::logic::tests::group7::copy_range_covers_the_full_selection_not_a_viewport` | verified |
| `TRAME-UI-060` | A copy MUST NOT change the document. | this spec section 3 | type-level: `copy_range` takes `&self` and returns `TrameOutcome::display_only`, which carries no delta (`logic/clipboard.rs`) | verified |
| `TRAME-UI-061` | A pasted formula MUST have its relative references shifted by the paste offset. | this spec section 3 | `ambre_trame::logic::tests::group5::paste_reindexes_relative_formula` | verified |
| `TRAME-UI-062` | A blank source cell MUST leave its destination untouched. | this spec section 3 | none. Copy omits blank cells from the payload and paste writes only what the payload lists, so the rule holds by construction on both sides -- and neither side is asserted | unverified |
| `TRAME-UI-063` | A paste MUST be one undo item, whatever the size of the range. | this spec section 3 | none. `paste_range` opens one undo group and pre-materializes every cell so the writes collapse into one commit; the paste test asserts the values, never the undo | unverified |
| `TRAME-UI-064` | A cut MUST clear its source only once the paste lands. | this spec section 4 | none. `finish_paste` is the only site that clears, and it runs after the paste command is sent; nothing drives a cut end to end | unverified |
| `TRAME-UI-065` | A paste MUST prefer this tab's own copy while the clipboard still carries its ownership token. | this spec section 3 | none. The token is written synchronously by the host and read back into `own_copy`; the ladder below it has no test | unverified |

* **Budget.** none: a copy serialises the selection once, and
  `ambre-spec-budgets.md` budgets no clipboard operation.
* **Boundary.** the OS clipboard carries the selection out of the application, in plaintext, by the
  user's own gesture. `ambre-spec-clipboard.md` fixes what it may hold and the window it may be
  written in. Nothing here reaches the relay or holds a key.
* **Verification.** 3 `verified`, 4 `unverified`. The two rules that are held are the two that live
  in the Logic thread, where a test can reach them. Everything above the seam -- the ownership
  token and the cut -- is browser state, and the gap is the same one the edit session has.

---

## 2. WHAT EACH USER SEES

| A does | B does | Converged document | What each user sees | Test |
|---|---|---|---|---|
| pastes a block over `B2:C3` | types into `B2` | one value per cell, elected by LWW | the same grid; the later write wins per cell | none. The paste writes cells like any edit, so it converges for the same reason, but `Paste` is not in the convergence property's op set |
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
| a refused paste | a toast, raised from `PasteRejected` |

---

## 5. DECISIONS

| Decision | Rationale | Rejected alternative |
|---|---|---|
| Copy is built in the Logic thread | the UI holds one viewport, so a copy built there would silently truncate at the edge of what was scrolled. The Logic thread holds the document, and `Ctrl+A` then copy has to mean the whole selection | building the payload from the UI cache, which is what a viewport-limited copy does |
| The OS write happens one frame later | the payload does not exist yet when the key fires, and the async clipboard API needs the user activation the key opened. One frame is inside that window | preparing a payload ahead of every copy, or holding a promise in the clipboard item, both rejected in `ambre-spec-clipboard.md` |
| A cut clears its source on the paste, not on the cut | a cut that clears at once and is never pasted destroys data with no gesture to blame | clearing at cut time, which is what a delete already does |
| The typed payload is opaque bytes under a private MIME type | each app owns its own shape, and the plain-text mirror is what crosses to other applications | one interchange format shared by every app, which is the cross-app follow-up, not this |
| A pasted formula shifts, a pasted literal does not | Ambre has no absolute reference syntax, so every reference is relative and a paste that did not shift would break every formula it moved | keeping the source references, matching an absolute-only model |

---

## 6. NOT BUILT, AND NOT YET RULED

* **The module header contradicts its own file.** `logic/clipboard.rs` opens with "copy and cut are
  pure UI-thread operations", and `copy_range` sits forty lines below it. `TRAME-UI-059` is the rule
  that says which one is true.
* **A cut across two peers.** The clear is a separate command from the paste, so a cut is two undo
  items and a concurrent edit inside the source range races with the clear. No rule covers either.
* **A paste that lands past the grid.** The destination is clamped for the selection but nothing
  refuses a payload whose extent runs past the last row or column.
