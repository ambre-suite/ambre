# AMBRE -- FEATURE: SELECTION AND NAVIGATION
**Type:** Vertical (feature)
**Implementation:** partial -- a whole-axis band reaches the Logic thread
clamped to the visible rows (section 6).

> A user points at a cell, sweeps a rectangle, adds a second one with a modifier, or takes a whole
> column from its header. Every command that acts on a range -- merge, sort, fill, style, delete --
> acts on what that selection covers, and the keyboard reaches all of it without the mouse.

Selection is local view state: it writes nothing to the document and reaches no collaborator. What
a collaborator changes is the data underneath it, which is what section 2 is about.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`.

| ID | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|
| `TRAME-UI-030` | Extending a selection MUST leave its fixed corner where the user put it. | this spec section 4 | `ambre_trame::ui::grid::tests::group1::move_with_extend_keeps_anchor`, `::selection_extend_disjoint_and_membership`, `::single_cell_selection_tracks_active` | verified |
| `TRAME-UI-031` | A cell covered by two selected rectangles MUST be acted on once. | this spec section 3 | `ambre_trame::ui::grid::tests::group1::selected_cells_enumerates_and_dedups` | verified |
| `TRAME-UI-032` | A range operation MUST act on the rectangle the user selected, never on the active cell alone. | this spec section 3 | type-level: `SetActiveCell` carries the anchor beside the active cell, and every range intent the menus send carries no geometry at all, so the Logic thread can only read the tracked pair | verified |
| `TRAME-UI-033` | A whole-column or whole-row selection MUST NOT require the document height from the UI thread. | this spec section 4 | `ambre_trame::ui::grid::tests::group1::axis_selection_spans_the_band` covers the band shape, which carries no extent across its axis. It does not cover the command built from it, and that command reads the extent out of the UI cache. **The rule fails today** -- section 6 | unverified |
| `TRAME-UI-034` | A lone selected cell MUST show the active border alone, with no range fill behind it. | this spec section 4 | `ambre_trame::ui::renderer::tests::multi_cell_selection_fills_and_outlines_each_range`, `::draw_overlay_draws_headers_and_active_border` | verified |
| `TRAME-UI-035` | A selection MUST NOT survive the document it was made in: it MUST NOT be written to the document nor sent to a peer. | `ambre-spec-ui.md section 2` | structural: `Selection` lives in `ui/grid/`, and `TRAME-UI-001` already forbids that module any document access | verified |
| `TRAME-UI-019` | A remote cursor MUST NOT force a viewport re-derive. | this spec section 3 | not built -- lands with the "Grid surface" Story (`ambre-roadmap.md` E6). The half about leaving no trace in the document is `PRESENCE-001` | unimplemented |

* **Budget.** none: the selection is view state, and `ambre-spec-budgets.md` budgets
  no selection operation. Painting it sits in the frame phases of
  `ambre-spec-budgets.md section 2`.
* **Boundary.** `TRAME-UI-019` is the one exception: another user's selection travels as an
  ephemeral frame, which `ZK-005` seals with the `DocumentKey` and leaves the relay `document_id`,
  `pub_key` and `counter` alone. The local selection never leaves the UI thread.
* **Verification.** 5 `verified`, 1 `unverified`, 1 `unimplemented`. Three rules the selection
  depends on are declared elsewhere and cited, not restated: a merged block selects whole
  (`TRAME-UI-009`), a hidden row is stepped over (`TRAME-UI-008`), and hit-testing inverts the
  drawn geometry (`TRAME-UI-007`). The one gap is the whole-axis band: the rule is right, the
  decision below it is right, and the code answers with the last row it happens to hold. Three of
  the four unbuilt presence rows are **retired, not moved** (the register is `invariants.py`):
  `TRAME-UI-016`, `-017` and `-018` each restated a rule three applications share and carried no
  Trame mechanism, and the rule is
  `PRESENCE-003` to `PRESENCE-006` of `1-Foundations/ambre-spec-presence.md`. What stays Trame's
  own is the `CellAnchor` of section 3, and the re-derive half of `TRAME-UI-019`.

---

## 2. WHAT EACH USER SEES

The selection itself is never shared. Each row below is a peer changing the data under a live
selection, and what the local user then holds.

| Alice does | Bob does | The document holds | Alice sees | Test |
|---|---|---|---|---|
| selects A1:B2 | merges A1:A2 | the merge | her active cell rehomed onto the block's top-left cell, and her rectangle grown to whole blocks | `ambre_trame::ui::grid::tests::group2::snap_active_to_master_rehomes_after_merge`, `group1::drag_selection_expands_to_whole_merge` |
| selects the block A1:A2 | unmerges it | no merge | both freed cells still selected, though the span that defined them is gone | `ambre_trame::ui::grid::tests::group1::select_rect_keeps_unmerged_cells_selected` |
| fills five rows from a 2x1 merged block | -- | one merge per tile | her selection at full extent, with only its active corner rehomed onto a master | `ambre_trame::ui::grid::tests::group2::snap_keeps_multi_block_fill_selection` |
| selects A1:A9 | filters that column | the filter | the same rectangle, and arrows that step over the hidden rows | `ambre_trame::ui::grid::tests::group2::filtered_rows_collapse_geometry_and_nav` |
| selects row 5 | inserts a row above row 1 | the new row | row 5 still selected, now holding what was row 4 | none -- see section 6 |

---

## 3. MODEL

**Containers.** None. Nothing about a selection is persisted.

**View state.** One field of `TrameUiState`:

```rust
pub struct Selection {
    pub anchor: (u32, u32),    // the fixed corner the user set (`TRAME-UI-030`)
    pub active: (u32, u32),    // the moving corner: cursor, editor and format target
    pub extra: Vec<CellRect>,  // rectangles already committed with the modifier
    pub kind: SelKind,         // Cells, or a whole-axis band (`TRAME-UI-033`)
}
```

The live rectangle is `anchor` to `active`, normalized on read. The whole selection is `extra` plus
that rectangle, and a cell in two of them is enumerated once (`TRAME-UI-031`).

**Commands and events** (`ambre-bus`):

| Shape | Direction | Carries |
|---|---|---|
| `TrameCommand::SetActiveCell { row, col, anchor_row, anchor_col }` | UI to Logic | both corners, on every move, so a range intent has a target (`TRAME-UI-032`) |
| `TrameEvent::SelectRange { r1, c1, r2, c2 }` | Logic to UI | a selection the canvas cannot derive: the cells an unmerge just freed |
| `TrameCommand::UpdateCursor`, `TrameEvent::RemoteCursors { cursors }` | both ways | the presence pair, still absent from `ambre-bus`. The event carries the whole live set, rebuilt on every change, so the UI replaces its own and a departed peer needs no removal path |

**Steps, on every move of the active cell:**

1. step from the active cell; on a merge master, step by the whole span, so the move cannot land on
   a covered cell and snap straight back (`TRAME-UI-009`);
2. clamp to the grid, then skip the run of rows the filter hides in the travel direction;
3. collapse to a single cell, or move the active corner alone when the user is extending;
4. snap a collapsing move onto the block master, and leave an extending one on its raw corner, so
   the rectangle can still grow cell by cell;
5. report the new pair to the Logic thread, and scroll the target into view if it left the screen
   (`features/ambre-spec-trame-grid-viewport.md section 2`).

**Code:**

* `crates/ambre-trame/src/ui/grid/mod.rs` -- `Selection`, `SelKind`, the rectangles and the
  deduplicated cell set;
* `ui/grid/state.rs` -- `select`, `select_extend`, `select_disjoint`, `move_cursor`, `move_to`, the
  merge snap, the content scans;
* `ui/view/input.rs` -- the key map, the pointer paths, the axis drag;
* `ui/renderer/overlay.rs` -- the range fill, the active border, the band tint.

**The other users' selections**, not built. The behaviour is one the three applications share and
is `1-Foundations/ambre-spec-presence.md`, `PRESENCE-001` to `PRESENCE-010`: the ephemeral
transport, the identity resolution, the keying by `UserId`, the shared colour, the whole-set event
and the drop of an anchor that resolves to nothing. What is Trame's own is the anchor, and the two
shapes it sits in.

```rust
/// The wire payload, postcard-encoded into the `Ephemeral` bytes. It carries no
/// `UserId` (`PRESENCE-002`): identity is the `from_user_id` the Sync thread
/// resolves out of band, so a peer cannot assert another user's.
pub struct PresencePayload {
    pub anchor:       CellAnchor,
    pub selection:    Option<(CellAnchor, CellAnchor)>,  // top-left, bottom-right
    pub display_name: String,                            // cosmetic (`PRESENCE-004`)
}

/// A cell anchor that survives a structural change: the row and column **uids**
/// of `ambre-spec-trame-schema.md section 2`, never grid positions.
pub struct CellAnchor {
    pub sheet_id: SheetId,
    pub row_uid:  Uid,
    pub col_uid:  Uid,
}

/// Logic to UI, one per live peer. `user_id` is the resolved author and is never
/// read from the payload (`PRESENCE-003`); `color` is `ambre_draw::peer::peer_color`
/// (`PRESENCE-006`); the positions are the anchors resolved against the `SheetIndex`.
pub struct RemoteCursorPayload {
    pub user_id:      UserId,
    pub display_name: String,
    pub color:        u32,
    pub sheet_id:     SheetId,
    pub row:          u32,
    pub col:          u32,
    pub selection:    Option<(u32, u32, u32, u32)>,
}
```

An anchor holds uids because a remote cursor must follow its cell through the inserts, deletes and
sorts a collaborator makes while the frame is in flight; a position would make it jump
(`PRESENCE-007`). An anchor that resolves to no live sheet, row or column is dropped in silence
(`PRESENCE-008`), and the cursor reappears on its own once the delta arrives. The four steps an
inbound frame takes are `1-Foundations/ambre-spec-presence.md section 3` and are not restated; the
fourth is this application's, and paints each peer's cell border and name badge in the peer colour.

Nothing of this exists in Trame today: `TrameState::apply_ephemeral` takes the payload and returns
an empty list, and it does not even receive the resolved author. Toile ships the receiving half
(`features/ambre-spec-toile-presence.md`, `TOILE-016`).

---

## 4. SURFACE

**Pointer.**

| Gesture | Effect |
|---|---|
| press on a cell | collapses the selection onto it and arms a sweep; a press on the already active cell opens the editor instead |
| drag | moves the active corner, never the anchor (`TRAME-UI-030`) |
| `Shift` and press | extends the live rectangle to that cell |
| `Ctrl` or `Cmd` and press | commits the live rectangle and starts a new one, so the selection becomes several rectangles |
| press on a column or row header | takes the whole axis as a band (`TRAME-UI-033`); a drag along the strip sweeps the band, and `Shift` extends it |
| press on a header already inside the band | grabs it for a reorder instead of reselecting (`features/ambre-spec-trame-headers.md section 4`) |
| press on the top-left corner | takes the whole sheet, as a band over every column |

**Keyboard**, when no editor is open. `Ctrl` or `Cmd` is the jump modifier, `Shift` the extend
modifier.

| Key | Effect |
|---|---|
| arrows | move one cell, extending with `Shift` |
| `Tab`, `Enter` | move one column right, one row down; `Shift` reverses the direction and never extends |
| `PageUp`, `PageDown` | move by the number of rows on screen |
| `Home` | first column of the row; with the jump modifier, the top-left cell of the sheet |
| `End` | last cell of the row that carries content; with the jump modifier, the last such cell of the sheet |
| `Delete`, `Backspace` | clear the active cell. `Shift` with `Delete` is left to the host, which serves the cut |

**Rendering** (`ui/renderer/overlay.rs`):

| What | Drawn as |
|---|---|
| each selected rectangle | a translucent fill and a one-pixel outline |
| a lone selected cell | the active border, and no fill (`TRAME-UI-034`) |
| a rectangle touching a merge | grown to whole blocks first (`features/ambre-spec-trame-cell-merges.md section 4`) |
| the covered rows and columns | a tinted header; a whole-axis band tints its whole strip |
| the active cell | announced to the accessibility tree on every move |

**What reads the selection.** Merge, unmerge, sort, filter, fill, clipboard, structural insert and
delete, cell style, and the status bar aggregates all send an intent with no geometry and let the
Logic thread read the tracked pair (`TRAME-UI-032`).

---

## 5. DECISIONS

| Decision | Rationale | Rejected alternative |
|---|---|---|
| The UI sends both corners on every move, and range intents carry no geometry | one source of truth for what a command acts on. A command carrying its own rectangle can disagree with what the user sees, and every menu becomes a second place to get the range right | each intent carrying the rectangle it applies to |
| A whole-axis selection is its own kind, not a rectangle clamped to a guess | the UI thread holds no document, so it cannot know where the data ends; the Logic thread does, and already resolves the used range for a whole-column formula range | a rectangle spanning to the last row of the virtual grid, which would make every axis operation cost 100 000 rows |
| An extending move keeps its raw corner, a collapsing move snaps to the block master | a rectangle that snapped while growing could not be grown cell by cell past a merge; a collapsed active cell must never sit on a cell the merge hides | snapping in both cases, which traps the cursor beside a merge |
| A step off a merge master advances by the whole span | the cell next to a master is covered, so a one-cell step would snap back onto the master and the cursor would not move | leaving the covered cells selectable |
| A lone single cell shows no range fill | in a multi-cell selection the fill already marks the block, so a second active-cell border reads as a spurious selection following the cursor. This is what Excel does | drawing both marks in every case |
| A second rectangle is added with a modifier click, not with a modifier drag | the committed rectangles are a list of finished shapes, and one live rectangle is what a drag can grow | a modifier drag adding a swept rectangle |

---

## 6. NOT BUILT, AND NOT YET RULED

* **A remote structural change moves the data, not the selection.** The selection is a pair of
  absolute indices, so an insert above it, a delete, a reorder or a sort leaves it where it was,
  now covering different cells. No rule says whether it should follow the data, and section 2 row 5
  carries no verification.
* **`End` stops at the edge of what has been fetched.** The scan reads the local cell cache, which
  holds the requested window alone, so on a large sheet the key lands short of the real last cell.
  The Logic thread knows the used range and is not asked.
* **A modifier drag adds nothing.** Only a modifier click commits a rectangle, so a user cannot
  sweep a second range.
* **The band is resolved by the wrong thread, against the wrong data** (`TRAME-UI-033`). The
  decision below section 5 says the UI cannot know where the data ends, and the code has it guess:
  a header click builds a rectangle whose far corner is the last non-empty cell **in the local cell
  cache**, which holds the requested window and its margin. So selecting a column longer than the
  screen selects its first screen, and every command resolving against that rectangle -- the sort,
  the style, the delete, the copy and the status-bar aggregates -- acts on part of the column.
  `features/ambre-spec-trame-status-bar.md section 5` states what a user sees.
* **An empty band has no rule.** Nothing states what an operation does with a whole-axis selection
  on a sheet with no content.
* **A collaborator's cursor is invisible** (`TRAME-UI-019`, `PRESENCE-001`). Everything under the
  feature ships and nothing above it does: the ephemeral transport, the author resolution and the
  shared colour function are built and tested, and Trame's own slice is a stub that drops the
  payload. It is not
  even shaped to receive one -- `apply_ephemeral` takes the bytes without the resolved author, so
  the identity rule the design rests on cannot be applied at that signature.
* **A peer's selection rectangle is carried and never drawn.** `PresencePayload` holds the two
  corners and `RemoteCursorPayload` resolves them, and no rule says what the renderer does with
  them: whether a collaborator's range is tinted like the local one, outlined, or left invisible.
