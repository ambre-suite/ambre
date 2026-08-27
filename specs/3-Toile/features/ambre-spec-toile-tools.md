# AMBRE -- FEATURE: THE TOOLS

**Type:** Vertical (feature)
**Implementation:** partial -- the eight tools, the palette and the cursor are built; the comment tool is not.

> A user picks a tool and the board obeys one rule: the gesture shows what it will make, and the
> release makes it once. `Escape` takes it back and re-arms Select.

One state machine owns the active tool, and it is the only place a gesture turns into a command.
There is no two-peer table below: how each command converges is stated by the feature that owns
it.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`.

| ID | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|
| `TOILE-UI-008` | A gesture MUST emit exactly one command, on release; a click that moves nothing MUST emit none. | this spec section 2 | `ambre_toile::ui::tool::tests::resize_preview_then_release_emits_exactly_one_command`, `::click_without_movement_emits_no_command`, `::click_selects_then_drag_emits_move`, `::drag_corner_handle_emits_resize`, `::drag_rotate_handle_emits_rotate` | verified |
| `TOILE-UI-009` | `Escape` or a tool switch MUST cancel an in-flight gesture without emitting a command. | this spec section 2 | `ambre_toile::ui::tool::tests::switching_tools_cancels_in_flight_gesture`, `::escape_reverts_an_armed_creation_tool_to_select`, `::escape_on_the_select_tool_reports_no_tool_change`, `::connector_release_on_empty_emits_nothing` | verified |
| `TOILE-UI-036` | Exactly one tool MUST be active, and the palette button, the bare-key chord and the canvas MUST resolve to the same one. | this spec section 2; section 3 | `ambre_toile::ui::palette::tests::tool_for_command_maps_every_tool_id`, `::tool_for_command_rejects_non_tool_ids`, `::tool_keys_round_trip_key_to_command_to_tool`, `::tool_chords_are_distinct` | verified |
| `TOILE-UI-037` | A creation tool MUST place what the gesture describes, and a press that does not drag MUST fall back to the tool's default size. | this spec section 2 | `ambre_toile::ui::tool::tests::shape_drag_emits_create_at_drag_rect`, `::shape_click_uses_default_size`, `::sticky_click_creates_sticky_with_empty_text_and_picked_fill`, `::pen_clicks_then_double_click_emits_path`, `::draw_stream_emits_commit_stroke`, `::connector_anchor_to_anchor_emits_create_connector` | verified |
| `TOILE-UI-038` | Placing a note or a text element MUST return the board to Select, so the next click does not place another one. | this spec section 3 | none. The revert runs where the placement command is applied on the view, and the tool tests drive the machine below it, where the revert does not happen | unverified |
| `TOILE-UI-039` | The canvas cursor MUST be a pure function of the active tool, and of the hovered handle while a single element is selected. | this spec section 3 | `ambre_toile::ui::palette::tests::cursor_reflects_the_active_tool`, `::resize_cursor_uses_native_keywords_when_axis_aligned`, `::resize_cursor_rotates_continuously_when_the_element_is_rotated`, `ambre_toile::ui::tool::tests::gesture_cursor_reflects_the_in_flight_drag` | verified |
| `TOILE-UI-040` | A bare-key tool chord MUST NOT be taken while the user is typing into a field. | this spec section 3 | `ambre_toile::ui::palette::tests::editable_tag_guards_text_entry` | verified |
| `TOILE-UI-041` | Holding `Space` MUST pan in any tool, and MUST NOT start that tool's gesture. | this spec section 3 | none. The key sets a flag the pointer handler reads before dispatching to the machine, and no test drives the pair | unverified |
| `TOILE-UI-042` | The comment tool MUST drop a thread where it clicks, on an element or on empty board. | this spec section 5; `features/ambre-spec-toile-comments.md section 4` | not built -- lands with the "Comments" Story (`ambre-roadmap.md` E4) | unimplemented |

* **Budget.** the content hit-test and input dispatch under 0.2 ms, one row of
  `ambre-spec-budgets.md section 2`, and the view-state update under 0.5 ms on the same
  table. Neither has a bench. The rule that keeps the gesture inside them is `TOILE-UI-008`: nothing
  crosses to the Logic thread while the pointer is down.
* **Boundary.** none: the machine is view state on the UI thread, where `UI-THREAD-004` bans a
  network, crypto or persistence path. What it emits is a command; what it streams mid-gesture is
  presence, sealed by the Sync worker like any other frame (`ZK-005`).
* **Verification.** 6 `verified`, 2 `unverified`, 1 `unimplemented`. Every gesture that ends in a
  command is covered; both gaps are on the view above the machine -- the revert after a placement,
  and the space-drag -- and each is a flag a host test can drive.

---

## 2. MODEL

**Containers.** None. The active tool is view state and never leaves the UI thread.

**The tools** (`ambre-toile`):

```rust
pub enum Tool {
    Select,             // move, resize, rotate, marquee
    Pen,                // a Bezier path built across clicks
    Sticky(u32),        // click-drop a note in the armed colour
    Shape(ShapeKind),   // drag a rectangle to size a rect, ellipse, triangle or line
    Connector,          // drag from one element anchor to another
    Draw,               // freehand: sample, stream, commit on release
    Text,               // click to place a text element
    Hand,               // pan only
}
```

The colour rides the sticky tool, so a click places the note in the picked colour with no
create-then-restyle.

**What each gesture emits.**

| Tool | Gesture | Command on release |
|---|---|---|
| Select | drag the body | `MoveElements` |
| Select | drag a handle | `ResizeElement` or `RotateElement` |
| Select | drag empty board | none: a marquee, resolved locally (`features/ambre-spec-toile-selection.md`) |
| Pen | click, then double-click to close | one `CreateElement` carrying the path |
| Sticky, Text | click | `CreateElement`, then the inline editor opens on the new id |
| Shape | drag a rectangle | `CreateElement` at the drag rectangle, or at the default size |
| Connector | anchor to anchor | `CreateConnector`; a release on empty board emits nothing |
| Draw | press, move, release | one `CommitStroke` carrying the raw samples |
| Hand | drag | none: the camera moves (`features/ambre-spec-toile-viewport.md`) |

**Steps, on a gesture:**

1. the press arms a draft on the active tool, or hands the event to the Select machine;
2. each move updates the draft and repaints a local preview, and streams presence;
3. the release turns the draft into exactly one command (`TOILE-UI-008`);
4. a release that describes nothing -- a shape smaller than the drag floor, a connector on empty
   board, a stroke of one sample, a pen path of one node -- emits none.

**The live preview is local.** A move, resize or rotate feeds the in-flight geometry into the same
per-element override a presentation transition uses, so the object itself follows the drag. Nothing
is written while the pointer is down, so the gesture costs no round trip and no history entry, and
the release still commits one undo item.

**Cancelling.** `Escape` and a tool switch both drop the draft. `Escape` also re-arms Select, so an
abandoned creation tool stops placing elements on the next click (`TOILE-UI-009`).

**Code:**

* `crates/ambre-toile/src/ui/tool.rs` -- the machine, the drafts, the handles and the previews;
* `ui/palette.rs` -- the command ids, the chords, the cursors and the typing guard;
* `ui/view.rs` -- the pointer and key handlers, the space-drag and the tool signal.

---

## 3. SURFACE

**The palette.** One button per tool, each a registry command whose active state reflects the
machine (`ambre-spec-chrome.md`). The sticky button opens a swatch popover that arms the tool in the
picked colour and remembers it. A tool sub-option that is not a per-element style -- the shape kind,
the connector routing -- is a palette signal folded into the command the tool emits.

**The chords.** One bare key per tool, the whiteboard letters, bound in the launcher keymap and
refused while the pointer is in a text field (`TOILE-UI-040`).

**The cursor.** A note-shaped cursor arms the sticky tool, a crosshair the other creation tools, a
grab hand the Hand tool, an arrow Select. Over a single selection it also follows the hovered handle:
the double arrow of the axis that side resizes along, tilted to the element's angle, and a rotation
cursor on the rotate handle. An upright element takes the native keyword, a rotated one a drawn
cursor at the exact angle, because a native resize cursor cannot rotate (`TOILE-UI-039`).

**The keyboard, on the canvas.**

| Shortcut | Effect |
|---|---|
| `Escape` | leaves a focused embed, then cancels the gesture and re-arms Select, then leaves the group scope, then clears the selection |
| `Space` held, with a drag | pans, in any tool (`TOILE-UI-041`) |
| `Delete`, `Backspace` | deletes the selection |
| arrow keys, with `Shift` | nudges the selection by one world unit, or ten |
| `Ctrl/Cmd+Z`, `Ctrl/Cmd+Shift+Z`, `Ctrl/Cmd+Y` | undo and redo (`features/ambre-spec-toile-undo.md`) |
| `Ctrl/Cmd+D` | duplicates the selection at a small diagonal offset |
| `Ctrl/Cmd+G`, `Ctrl/Cmd+Shift+G` | groups and ungroups (`features/ambre-spec-toile-groups.md`) |
| `Ctrl/Cmd+A` | selects the board (`features/ambre-spec-toile-selection.md`) |
| a printable character, on a single note or text element | enters the inline editor and replaces the body (`features/ambre-spec-toile-text-editing.md`) |

**After a placement.** A note or a text element opens its editor as soon as its id comes back, and
the board returns to Select, ready to type into (`TOILE-UI-038`).

---

## 4. DECISIONS

| Decision | Rationale | Rejected alternative |
|---|---|---|
| One explicit machine owns every gesture | eight tools times press, move and release is a matrix, and spreading it over the features that own each element would make "one command per gesture" a rule nobody could check. One machine makes it a property of one module | a handler per tool, each free to emit what it wants |
| Nothing is written while the pointer is down | a drag at 60 frames a second would be sixty operations, sixty history entries and sixty deltas for one user action | streaming the transform as it moves, which the presence path already does without touching the document |
| The preview transforms the object, not a box | a user resizing a note wants to see the text reflow; an outline says where it will land, not what it will look like | drawing the selection box alone during the gesture |
| A degenerate gesture emits nothing | a click is how a user cancels, and a zero-area shape or a connector to nowhere would be an element only an undo could remove | committing whatever the gesture described, however small |
| A click with a creation tool falls back to a default size | placing a note is a click, not a drag, and asking the user to size every note would make the fastest gesture on a whiteboard the slowest | requiring a drag for every creation |
| The colour rides the sticky tool | a create-then-restyle is two operations, two undo items and a visible flash of the wrong colour | arming a colour separately from the tool |
| `Escape` re-arms Select | an armed creation tool is invisible once the popover closes, and a user who pressed `Escape` to abort does not expect the next click to place a note | cancelling the gesture and keeping the tool |
| The active tool never reaches the Logic thread | it decides what a pointer event means, and the Logic thread sees only the command that results | a tool register on the document, which two users would fight over |

---

## 5. NOT BUILT, AND NOT YET RULED

* **The comment tool** (`TOILE-UI-042`). The tool set has no comment entry, and the command it would
  emit exists nowhere in the workspace, while three sections describe the behaviour. It is the
  largest unbuilt subject in the application, and it lands whole with its Story.
* **The pen has no node editing.** A path is built across clicks and committed once; dragging the
  nodes of a committed path is described and unbuilt, so a path is editable by its box alone.
* **The connector routing has one shape.** Every connector is created straight, with an arrow at the
  end. The routing modes and the end caps the model carries are not reachable from any gesture.
* **Space-drag and the placement revert have no test.** Both are flags on the view, driven by the
  pointer and key handlers, and both are pure state a host test reaches. The machine under them is
  covered gesture by gesture, which is what makes these two the whole gap.
* **Nothing here states what a gesture does over a focused embed.** The rule that routes a press to
  the child rather than to the board is written on the embed side alone
  (`features/ambre-spec-toile-embeds.md section 4`).
