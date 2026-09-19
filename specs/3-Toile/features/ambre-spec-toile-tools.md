# AMBRE -- FEATURE: THE TOOLS

**Type:** Vertical (feature)
**Implementation:** implemented

> A user picks a tool and the board obeys one rule: the gesture shows what it will make, and the
> release makes it once. `Escape` takes it back and re-arms Select.

One state machine owns the active tool, and it is the only place a gesture turns into a command.
There is no two-peer table below: how each command converges is stated by the feature that owns
it.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TOILE-UI-036` | `INV` | Exactly one tool MUST be active on the board. | this spec section 2 | type-level -- the machine holds one `ambre_toile::ui::tool::Tool` and every arm replaces it, so a second active tool has nowhere to live | none | verified |
| `TOILE-UI-243` | `INV` | The board MUST NOT let the active tool reach the Logic thread: it decides what a pointer event means, and nothing more. | this spec section 4; `UI-THREAD-004` | type-level -- no `ToileCommand` carries a tool, and a tool `CommandId` is absent from the command-to-application map, so the only thing that crosses is the command a gesture produced | none | verified |
| `TOILE-UI-244` | `INV` | A palette button and its bare key MUST arm the same tool. | this spec section 2; section 3 | `ambre_toile::ui::palette::tests::tool_keys_round_trip_key_to_command_to_tool` -- it walks key to command to tool for every one of them, the chord arming the default note colour where a swatch button arms its own, which is the same tool and not the same colour; `ambre_toile::ui::palette::tests::tool_for_command_maps_every_tool_id` -- the map; `ambre_toile::ui::palette::tests::tool_chords_are_distinct` -- the collisions | none | verified |
| `TOILE-UI-039` | `OBS` | The canvas cursor MUST say what the armed tool will do. | this spec section 3 | `ambre_toile::ui::palette::tests::cursor_reflects_the_active_tool` -- it reads the note cursor, the crosshair, the grab hand and the arrow | none | verified |
| `TOILE-UI-245` | `OBS` | Over a single selection, the cursor MUST follow the hovered handle and tilt with the element. | this spec section 3 | `ambre_toile::ui::palette::tests::resize_cursor_uses_native_keywords_when_axis_aligned` -- the upright case; `ambre_toile::ui::palette::tests::resize_cursor_rotates_continuously_when_the_element_is_rotated` -- the exact angle; `ambre_toile::ui::tool::tests::hover_handle_follows_the_element_rotation` -- the handle the hover picks | none | verified |
| `TOILE-UI-246` | `OBS` | An in-flight drag MUST keep one cursor for the whole gesture, wherever the pointer wanders. | this spec section 3 | `ambre_toile::ui::tool::tests::gesture_cursor_reflects_the_in_flight_drag` -- it reads the cursor each drag opens with; structural -- that it then holds is the construction, since the handle is captured at press and read back unchanged | none | verified |

### 1.2 Pick a tool

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TOILE-UI-247` | `POST` | Arming a tool MUST abandon any gesture in flight on the previous one. | this spec section 2 | `ambre_toile::ui::tool::tests::switching_tools_cancels_in_flight_gesture` -- it arms a shape, presses, switches, and reads the machine idle | none | verified |
| `TOILE-UI-369` | `POST` | Arming a tool MUST emit nothing of the gesture it abandons. | this spec section 2 | `ambre_toile::ui::tool::tests::switching_tools_cancels_in_flight_gesture` -- it reads no preview left after the switch | none | verified |
| `TOILE-UI-248` | `FRAME` | Arming a tool MUST NOT touch the document or the selection. | this spec section 2 | type-level -- the switch returns nothing and holds no document handle, so the only state it can reach is the draft it drops | none | verified |
| `TOILE-UI-040` | `FAIL` | A bare-key tool chord MUST NOT be taken while the user is typing into a field. | this spec section 3 | `ambre_toile::ui::palette::tests::editable_tag_guards_text_entry` -- it holds the decision for a text field and a contenteditable host | partial -- nothing drives `ambre_app::platform::target_is_editable`, which reads the tag off the event and is the half that has to run; lands with the `The Toile belts` Story (`ambre-roadmap.md` E9) | untested |

### 1.3 Drag on the board

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TOILE-UI-008` | `POST` | A gesture MUST emit exactly one command, on release. | this spec section 2 | `ambre_toile::ui::tool::tests::resize_preview_then_release_emits_exactly_one_command` -- the one command a move-then-release makes; `ambre_toile::ui::tool::tests::click_selects_then_drag_emits_move` -- the move; `ambre_toile::ui::tool::tests::drag_corner_handle_emits_resize` -- the resize; `ambre_toile::ui::tool::tests::drag_rotate_handle_emits_rotate` -- the rotate | none | verified |
| `TOILE-UI-037` | `POST` | A creation tool MUST place what the gesture described. | this spec section 2 | `ambre_toile::ui::tool::tests::shape_drag_emits_create_at_drag_rect` -- the drag rectangle; `ambre_toile::ui::tool::tests::pen_clicks_then_double_click_emits_path` -- the path; `ambre_toile::ui::tool::tests::draw_stream_emits_commit_stroke` -- the stroke; `ambre_toile::ui::tool::tests::connector_anchor_to_anchor_emits_create_connector` -- the two anchors | none | verified |
| `TOILE-UI-249` | `POST` | A press that does not drag MUST fall back to the tool's default size. | this spec section 2 | `ambre_toile::ui::tool::tests::shape_click_uses_default_size` -- the shape; `ambre_toile::ui::tool::tests::sticky_click_creates_sticky_with_empty_text_and_picked_fill` -- the note, and the picked colour riding the tool with it | none | verified |
| `TOILE-UI-250` | `FRAME` | A gesture MUST NOT write while the pointer is down: no command, no history entry, no round trip. | this spec section 2; section 4 | `ambre_toile::ui::tool::tests::resize_preview_then_release_emits_exactly_one_command` -- it reads the intermediate move producing nothing while the live override is already there | none | verified |
| `TOILE-UI-251` | `FAIL` | A press on an element that moves nothing MUST emit no command. | this spec section 2 | `ambre_toile::ui::tool::tests::click_without_movement_emits_no_command` | none | verified |
| `TOILE-UI-252` | `FAIL` | A press on a transform handle that moves nothing MUST emit no command. | this spec section 2 | none | fails -- the release emits a resize or a rotate whatever the pointer did, so a stray click on a handle costs a delta and an undo item for a geometry nobody changed, section 5; lands with the `The board tools let go` Story (`ambre-roadmap.md` E7) | failing |
| `TOILE-UI-253` | `FAIL` | A connector released anywhere but on a second element MUST emit nothing. | this spec section 2 | `ambre_toile::ui::tool::tests::connector_release_on_empty_emits_nothing` | none | verified |
| `TOILE-UI-254` | `FAIL` | A pen path of fewer than two nodes MUST emit nothing. | this spec section 2 | none | unheld -- the guard is there and no test drives it, and the sibling guard on the freehand stroke is unreachable, because press and release each add a sample, section 5; lands with the `The Toile belts` Story (`ambre-roadmap.md` E9) | untested |

### 1.4 Hold `Space` to pan

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TOILE-UI-041` | `POST` | Holding `Space` MUST pan, in any tool. | this spec section 3; `features/ambre-spec-toile-viewport.md section 1` | none | unheld -- the key sets a flag the pointer handler reads before it dispatches to the machine, and no test drives the pair; lands with the `The Toile belts` Story (`ambre-roadmap.md` E9) | untested |
| `TOILE-UI-255` | `FRAME` | A space-drag MUST NOT start the active tool's gesture. | this spec section 3 | none | unheld -- the pan branch returns before the machine is reached, so the rule holds by the early return and nothing would go red the day the branch moved; lands with the `The Toile belts` Story (`ambre-roadmap.md` E9) | untested |
| `TOILE-UI-256` | `POST` | Releasing `Space` MUST give the board back to its tool. | this spec section 3 | none | fails -- the flag is raised on the key and lowered by nothing, so one press turns every later drag into a pan for the rest of the session, section 5; lands with the `The board tools let go` Story (`ambre-roadmap.md` E7) | failing |

### 1.5 Cancel a gesture

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TOILE-UI-009` | `POST` | `Escape` MUST cancel an in-flight gesture without emitting a command. | this spec section 2 | `ambre_toile::ui::tool::tests::escape_reverts_an_armed_creation_tool_to_select` -- it presses with a note armed, escapes, and reads the machine idle and the next click placing nothing | none | verified |
| `TOILE-UI-257` | `POST` | `Escape` on a creation tool MUST re-arm Select. | this spec section 2 | `ambre_toile::ui::tool::tests::escape_reverts_an_armed_creation_tool_to_select` -- the revert | none | verified |
| `TOILE-UI-370` | `POST` | `Escape` MUST report exactly the tool change it made, so the palette follows. | this spec section 2 | `ambre_toile::ui::tool::tests::escape_reverts_an_armed_creation_tool_to_select` -- the report; `ambre_toile::ui::tool::tests::escape_on_the_select_tool_reports_no_tool_change` -- the tool that was already Select | none | verified |
| `TOILE-UI-258` | `FRAME` | `Escape` MUST NOT touch the document. | this spec section 2 | type-level -- the cancel returns whether the tool changed and holds no document handle | none | verified |

### 1.6 Place a note or a text element

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TOILE-UI-038` | `POST` | Placing a note or a text element MUST return the board to Select, so the next click does not place another one. | this spec section 3 | none | unheld -- the revert runs on the view where the placement command is sent, and the tool tests drive the machine below it, where the revert does not happen; lands with the `The Toile belts` Story (`ambre-roadmap.md` E9) | untested |
| `TOILE-UI-259` | `POST` | Placing a note or a text element MUST open the inline editor on the new element once its id comes back. | this spec section 3; `features/ambre-spec-toile-text-editing.md section 1` | none | unheld -- the view arms a flag and waits for the creation event, and no test drives the pair: the same view seam as the revert of `TOILE-UI-038`; lands with the `The Toile belts` Story (`ambre-roadmap.md` E9) | untested |
| `TOILE-UI-260` | `FRAME` | The revert MUST NOT clear the selection or move the camera: the new element stays selected and ready to type into. | this spec section 3 | structural -- the switch clears the draft, the preview and the overrides, and touches neither the selection nor the camera | none | verified |

### 1.7 Drop a comment thread

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TOILE-UI-042` | `POST` | The comment tool MUST drop a thread where it clicks, on an element or on empty board. | this spec section 3; `features/ambre-spec-toile-comments.md section 4` | `ambre_toile::ui::tool::tests::the_comment_tool_drops_a_thread_where_it_clicks` -- it reads an element anchor off a click on a note and a point anchor off a click on empty board | none | verified |
| `TOILE-UI-261` | `FRAME` | Dropping a thread MUST NOT change the board: no element is created, moved or selected. | this spec section 3; `features/ambre-spec-toile-comments.md section 4` | `ambre_toile::ui::tool::tests::the_comment_tool_drops_a_thread_where_it_clicks` -- on the outcome it reads back: the drop yields an anchor and never a command, so nothing reaches the document | none | verified |

* **Budget.** the content hit-test and input dispatch under 0.2 ms, one row of
  `ambre-spec-budgets.md section 2`, and the view-state update under 0.5 ms on the same table.
  Neither has a bench. `TOILE-UI-250` keeps the gesture inside them: nothing crosses to the Logic
  thread while the pointer is down.
* **Boundary.** none: the machine is view state on the UI thread, where `UI-THREAD-004` bans a
  network, crypto or persistence path. What it emits is a command; what it streams mid-gesture is
  presence, sealed by the Sync worker like any other frame (`ZK-005`).
* **Verification.** 22 `verified`, 2 `failing`, 6 `untested`. What holds is the machine itself: the
  modes, the transitions and the commands each gesture ends in. What is loose is the refusals: a
  click on a transform handle, the release of `Space`, a degenerate pen path. The first two fail
  today (`TOILE-UI-252`, `TOILE-UI-256`).

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
    Comment,            // drop a comment thread (`features/ambre-spec-toile-comments.md`)
    Laser,              // the highlighter: a trail the peers see and the board never keeps
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
| Comment | click | none: the composer opens, and the thread is born with its first message (`features/ambre-spec-toile-comments.md`) |
| Laser | press, move, release | none: the point rides the presence sample and is never board state (`features/ambre-spec-toile-presence.md`) |
| Hand | drag | none: the camera moves (`features/ambre-spec-toile-viewport.md`) |

**Steps, on a gesture:**

1. the press arms a draft on the active tool, or hands the event to the Select machine;
2. each move updates the draft and repaints a local preview, and streams presence;
3. the release turns the draft into exactly one command (`TOILE-UI-008`);
4. a release that describes nothing -- a connector on empty board, a pen path of one node -- emits
   none. A shape under the drag floor is not one of them: it falls back to the tool's default size
   (`TOILE-UI-249`). Neither is a handle: the release emits its transform whether the pointer moved
   or not (`TOILE-UI-252`).

**The live preview is local.** A move, resize or rotate feeds the in-flight geometry into the same
per-element override a presentation transition uses, so the object itself follows the drag. Nothing
is written while the pointer is down. The gesture costs no round trip and no history entry, and the
release still commits one undo item.

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
picked colour and remembers it. A tool sub-option that is not a per-element style is a palette
signal, folded into the command the tool emits. The shape kind and the connector routing are the
two.

**The chords.** One bare key per tool, the whiteboard letters, bound in the launcher keymap and
refused while the pointer is in a text field (`TOILE-UI-040`).

**The cursor.** A note-shaped cursor arms the sticky tool, a crosshair the other creation tools, a
grab hand the Hand tool, an arrow Select. Over a single selection it also follows the hovered handle.
It shows the double arrow of the axis that side resizes along, tilted to the element's angle, and a
rotation cursor on the rotate handle. An upright element takes the native keyword, a rotated one a drawn
cursor at the exact angle, because a native resize cursor cannot rotate (`TOILE-UI-039`).

**The keyboard, on the canvas.**

| Shortcut | Effect |
|---|---|
| `Escape` | leaves a focused embed; otherwise one press does the rest at once: it cancels the gesture, re-arms Select, clears the selection and leaves the group scope |
| `Space` held, with a drag | pans, in any tool, and the release gives the board back to it (`TOILE-UI-041`, `TOILE-UI-256`) |
| `Delete`, `Backspace` | deletes the selection |
| arrow keys, with `Shift` | nudges the selection by one world unit, or ten |
| `Ctrl/Cmd+Z`, `Ctrl/Cmd+Shift+Z`, `Ctrl/Cmd+Y` | undo and redo (`features/ambre-spec-toile-undo.md`) |
| `Ctrl/Cmd+D` | duplicates the selection at a small diagonal offset |
| `Ctrl/Cmd+G`, `Ctrl/Cmd+Shift+G` | groups and ungroups (`features/ambre-spec-toile-groups.md`) |
| `Ctrl/Cmd+A` | selects the board (`features/ambre-spec-toile-selection.md`) |
| a printable character, on a single note or text element | enters the inline editor and replaces the body (`features/ambre-spec-toile-text-editing.md`) |

**After a placement.** A note or a text element opens its editor as soon as its id comes back. The
board returns to Select, ready to type into (`TOILE-UI-038`).

---

## 4. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| One explicit machine owns every gesture | a handler per tool, each free to emit what it wants | every tool times press, move and release is a matrix, and spreading it over the features that own each element makes "one command per gesture" a rule nobody can check. One machine makes it a property of one module |
| Nothing is written while the pointer is down | streaming the transform as it moves | a drag at 60 frames a second is sixty operations, sixty history entries and sixty deltas for one user action. The presence path already streams it without touching the document |
| The preview transforms the object, not a box | drawing the selection box alone during the gesture | a user resizing a note wants to see the text reflow, and an outline says where it will land, not what it will look like |
| A degenerate gesture emits nothing | committing whatever the gesture described, however small | a click is how a user cancels, and a zero-area shape or a connector to nowhere is an element only an undo can remove. The transform handles are the corner the code has not reached: they commit whatever they were, moved or not (`TOILE-UI-252`) |
| A click with a creation tool falls back to a default size | requiring a drag for every creation | placing a note is a click, not a drag, and sizing every note makes the fastest gesture on a whiteboard the slowest |
| The colour rides the sticky tool | arming a colour separately from the tool | a create-then-restyle is two operations, two undo items and a visible flash of the wrong colour |
| `Escape` re-arms Select | cancelling the gesture and keeping the tool | an armed creation tool is invisible once the popover closes, and a user who pressed `Escape` to abort does not expect the next click to place a note |
| The active tool never reaches the Logic thread | a tool register on the document | two users would fight over it. The tool decides what a pointer event means, and the Logic thread sees only the command that results |

---

## 5. NOT BUILT, AND NOT YET RULED

* **The pen has no node editing.** A path is built across clicks and committed once. Dragging the
  nodes of a committed path is described and unbuilt, so a path is editable by its box alone.
* **The connector routing has one shape.** Every connector is created straight, with an arrow at the
  end. The routing modes and the end caps the model carries are not reachable from any gesture.
* **`Space` is raised and nothing lowers it.** The key handler sets the pan flag and no handler in
  the workspace clears it: there is no `keyup` listener anywhere. One press turns every later drag
  into a pan for the rest of the session, whatever tool the palette shows (`TOILE-UI-256`).
* **A click on a transform handle commits a null transform.** A body drag that moves nothing emits
  nothing. A handle release emits its resize or its rotate unconditionally. A stray click on a
  handle costs a delta and an undo item for a geometry nobody changed (`TOILE-UI-252`).
* **The freehand refusal is unreachable.** Press and release each push a sample, so the guard that
  would drop a stroke of one point can never fire. A click with Draw armed commits a two-point
  stroke standing still. The pen guard beside it is reachable and undriven (`TOILE-UI-254`).
* **A resize has no floor.** The rectangle is normalised and nothing bounds it. Dragging a handle
  past the opposite edge lands an element at zero width or height. The decision above says a
  degenerate gesture never reaches that case.
* **The tool signal is compiled out on the host.** The call that tells the palette which tool is
  armed exists under wasm alone. That is why the revert after a placement and the space-drag sit
  below any host test that would read them back (`TOILE-UI-038`, `TOILE-UI-041`).
* **Half of the typing guard has no test.** The pure predicate is held for a field and for a
  contenteditable host. The reader that pulls the tag off the event, the half that runs on every
  keystroke, has none (`TOILE-UI-040`).
* **Nothing here states what a gesture does over a focused embed.** The rule that routes a press to
  the child rather than to the board is written on the embed side alone
  (`features/ambre-spec-toile-embeds.md section 4`).
