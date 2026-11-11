# AMBRE -- FEATURE: THE TRAME CLIPBOARD

**Type:** Vertical (feature)

> A user selects a thousand rows, scrolls past them and copies. What lands on the clipboard is the
> whole selection, not the part that was on screen. Pasted formulas point at their new neighbours.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TRAME-UI-124` | `CONC` | Two peers MUST converge when one pastes a block over cells the other is editing. | this spec section 2 | `ambre_trame::logic::convergence_tests::two_peers_converge_under_interleaved_ops` -- its operation set holds a paste and the batch clear a cut sends, over the cells the other peer edits | none | verified |

### 1.2 Copy the selection

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TRAME-UI-059` | `POST` | A copy MUST cover the whole selection, never only the cells the viewport holds. | this spec section 3 | `ambre_trame::logic::tests::group7::copy_range_covers_the_full_selection_not_a_viewport` -- it spans five hundred rows and asserts the far cell is in the payload | none | verified |
| `TRAME-UI-186` | `POST` | A copy MUST carry the range as a table that another spreadsheet or a word processor reads as cells. | this spec section 3; `UI-THREAD-061` | `ambre_trame::ui::view::view_tests::a_copy_hands_the_host_an_html_table` -- the copy reply hands the host the mirror and the table; `ambre_trame::tests::message::to_html_lays_out_the_tsv_grid_escaped` -- the table lays out the mirror's grid, each cell escaped | none | verified |

**Frame:** `UI-THREAD-044`

### 1.3 Cut the selection

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TRAME-UI-064` | `POST` | A cut MUST clear its source only once the paste lands. | this spec section 4 | `ambre_trame::ui::view::view_tests::a_cut_clears_its_source_behind_the_paste` -- the copy writes nothing, and the paste goes out before the clear | none | verified |
| `TRAME-UI-125` | `FRAME` | A cut MUST NOT clear its source when the paste did not reach the document. | this spec section 5; derived | `ambre_trame::ui::view::view_tests::a_refused_paste_leaves_the_cut_source`; `ambre_trame::ui::view::view_tests::a_refused_source_clear_keeps_the_cut_range` | none | verified |

### 1.4 Paste

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TRAME-UI-061` | `POST` | A pasted formula MUST have its relative references shifted by the paste offset. | this spec section 3 | `ambre_trame::logic::tests::group5::paste_reindexes_relative_formula`; `ambre_belt::trame_grid::tests::a_copied_range_pastes_with_its_formula_shifted` -- in the browser, `Ctrl+C` then `Ctrl+V` | none | verified |
| `TRAME-UI-063` | `POST` | A paste MUST be one undo item, whatever the size of the range. | this spec section 3 | `ambre_trame::logic::tests::group10::a_paste_is_one_undo_item` | none | verified |
| `TRAME-UI-065` | `POST` | A paste MUST prefer this tab's own copy while the clipboard still carries its ownership token. | this spec section 3 | `ambre_trame::ui::view::view_tests::a_paste_takes_the_highest_rung_it_can_serve` -- the kept copy answers on the token; `ambre_app::host::pipe::tests::a_paste_event_and_the_paste_button_end_in_one_call` -- the token read back decides it, in the browser tier | none | verified |
| `TRAME-UI-174` | `POST` | A plain paste MUST land the plain-text mirror as literal cells, whatever typed payload the clipboard or this tab's own copy holds. | this spec section 4; `PROSE-UI-227` | not built | unbuilt -- lands with the `Paste as plain text in Trame` Story (`ambre-roadmap.md` E6). The host already strips the typed payload on the chord, but the view ignores the flag and Trame offers no menu entry | unimplemented |
| `TRAME-UI-062` | `FRAME` | A blank source cell MUST leave its destination untouched. | this spec section 3 | `ambre_trame::logic::tests::group7::copy_range_covers_the_full_selection_not_a_viewport` -- the copy omits the blanks; `ambre_trame::logic::tests::group10::a_blank_source_cell_leaves_its_destination` -- a paste over occupied cells keeps what the blanks cover | none | verified |

* **Budget.** none: a copy serialises the selection once, and
  `ambre-spec-budgets.md` budgets no clipboard operation.
* **Boundary.** the OS clipboard carries the selection out of the application, in plaintext, by the
  user's own gesture. `ambre-spec-clipboard.md` fixes what it may hold and the window it may be
  written in. Nothing here reaches the relay or holds a key.
* **Verification.** 9 `verified`, 1 `unimplemented`. What holds is the rules that live in the Logic
  thread, and two peers that paste and cut over each other's edits converge. Above the seam, a
  native test drives the view. The kept copy answers on the token, and a cut clears its source only
  behind its paste. The token itself is browser state, held in the browser tier.

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

**The transport** is the app-agnostic `ClipboardBlob { text, html, ambre }` of `ambre-bus`. It is
specified in `ambre-spec-clipboard.md`. `text` is the plain-text mirror, `html` the same grid as one
escaped table, and `ambre` the postcard of the payload above. The table also carries the ownership
token where the browser refuses the `web ` types (`UI-THREAD-061`). This document states what Trame
puts in the transport, never how the seam works.

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
| `Ctrl+Shift+V`, or its menu entry | `TRAME-UI-174`, on the `Paste as plain text in Trame` Story (`ambre-roadmap.md` E6) |
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
* **A cut across two peers.** The clear is a separate command from the paste. A cut is two undo
  items, and a concurrent edit inside the source range races with the clear. No rule covers either.
* **A paste that lands past the grid.** The destination is clamped for the selection but nothing
  refuses a payload whose extent runs past the last row or column.
* **No paste is ever refused.** `TrameEvent::PasteRejected` is declared in `ambre-bus` and matched
  twice in the chrome, and no site in the workspace constructs it. The toast section 4 draws has no
  producer, so a paste that cannot land says nothing.
* **A cut leaves the formatting of its source behind.** The clear writes empty text through
  `SetCellsBatch` and touches neither the style register nor the bold and italic sets. The source
  keeps its formatting, while a copy of a blank-but-styled cell carries none of it. No rule says
  which of the two is right.
