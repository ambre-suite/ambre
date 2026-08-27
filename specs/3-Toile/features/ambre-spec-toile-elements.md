# AMBRE -- FEATURE: ELEMENTS ON THE BOARD

**Type:** Vertical (feature)
**Implementation:** partial -- the lifecycle ships; the lock misses the delete, and align has no code.

> Everything on a board is an element: a note, a shape, a picture, a frame, a stroke. One set of
> operations moves them, resizes them, rotates them, restyles them, locks them and deletes them,
> whatever they are.

The kinds differ only by the content they carry. That is why a frame needs no lifecycle of its own,
and why one table below covers the whole board.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`.

| ID | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|
| `TOILE-012` | A locked element MUST NOT be moved by a command. | this spec section 3 | `ambre_toile::logic::convergence_tests::locked_element_ignores_move` | verified |
| `TOILE-022` | A derived kind (`group`, `drawing`) MUST NOT be creatable through the create command, and a body overwrite MUST reject a non-text element. | this spec section 3 | `ambre_toile::logic::convergence_tests::group_and_drawing_are_not_creatable`, `::set_text_rejects_a_non_text_element`, `::set_text_overwrites_sticky_body` | verified |
| `TOILE-031` | A newly created element MUST enter the paint order, the bounds cache and the spatial index in the same commit that mints it. | this spec section 3 | `ambre_toile::logic::convergence_tests::create_registers_bounds_and_layer`, `::caches_match_a_full_rebuild` | verified |
| `TOILE-032` | A move, a resize and a rotate MUST update the element's rotation-expanded box, so culling and connector routing stay exact. | this spec section 3; `TOILE-005` | `ambre_toile::logic::convergence_tests::move_translates_bounds`, `::resize_updates_bounds`, `::rotate_expands_the_aabb` | verified |
| `TOILE-033` | A lock MUST refuse every geometry and style change to the element, not the move alone. | this spec section 3 | none. One test covers the move (`TOILE-012`); the resize, the rotate, the restyle and the body overwrite each carry the same refusal in code and no test, and **the delete and the reorder carry none at all** -- section 6 | unverified |
| `TOILE-034` | Deleting an element MUST drop its paint-order entry, its container membership and every connector that touched it, in one commit. | this spec section 3 | `ambre_toile::logic::convergence_tests::delete_drops_owned_connectors`, `::deleting_a_group_cascades_to_members` | verified |
| `TOILE-035` | Element birth MUST seed a per-kind default style, and a colour picked at placement MUST override the fill alone. | this spec section 3 | partial: `ambre_toile::ui::tool::tests::sticky_click_creates_sticky_with_empty_text_and_picked_fill` covers the picked colour reaching the command, and `ambre_toile::ui::render::tests::sticky_paints_a_colored_card` covers the paint. Nothing asserts the seeded default per kind | unverified |
| `TOILE-036` | A command naming an element the board does not hold MUST answer an error and MUST leave the document unchanged. | this spec section 3 | none. Every handler resolves the element map first and returns early, and no test drives a missing id through the lifecycle commands. The connector half has one (`TOILE-007`) | unverified |
| `TOILE-037` | Aligning or distributing a selection MUST be one convergent batch and one undo item. | this spec section 4 | not built -- lands with the "Align and distribute" Story (`ambre-roadmap.md` E4) | unimplemented |

* **Budget.** a command that writes one register and re-derives the visible set under 1 ms, the
  transform-command row of `ambre-spec-budgets.md section 1`. No bench measures it. A move over a
  selection is one register pair per leaf, so the figure is per element and the drag rate is the
  multiplier (`features/ambre-spec-toile-tools.md`).
* **Boundary.** none of its own: an element lives in the Logic thread's document and leaves as bytes
  the Sync worker seals (`ZK-001`). No lifecycle command opens a socket or holds a key.
* **Verification.** 5 `verified`, 3 `unverified`, 1 `unimplemented`. The happy path of every
  operation is held, including the cache equivalence the whole engine rests on. What is loose is the
  refusals: the lock is proven for one operation out of six, and a missing id is proven for none.

---

## 2. WHAT EACH USER SEES

| Alice does | Bob does | The document holds | Each user sees | Test |
|---|---|---|---|---|
| moves a note right | moves the same note down | both translations, applied to the two coordinates independently | the note at the diagonal of the two moves | `logic::convergence_tests::two_peers_converge` |
| restyles a shape red | restyles the same shape blue | one of the two whole styles, never a blend | the same colour on both screens (`TOILE-018`) | `logic::convergence_tests::two_peers_converge` |
| deletes a shape | resizes the same shape | the deletion: a resize of an absent element writes nothing | the shape gone on both screens | `logic::convergence_tests::caches_match_a_full_rebuild` |
| locks a shape | drags the same shape | the shape where it was | nothing moves, and Bob is told nothing | `logic::convergence_tests::locked_element_ignores_move` |

The lock is advisory and local to the operation, not a permission: it refuses the write on the peer
that attempts it. Who may write at all is `ambre-spec-architecture.md section 2`.

---

## 3. MODEL

**Containers.** The `elements` map and the `layers` list, both declared in
`ambre-spec-toile-schema.md section 3`. An element is one map under a minted id; the field layout per
kind is that document's field table.

**Commands** (`ambre-bus`), one per operation and one undo item each:

| Shape | Carries |
|---|---|
| `CreateElement { kind, at, size, content, fill }` | the kind, the birth rectangle, the type-specific seed, and an optional fill override |
| `MoveElements { ids, delta }` | a translation over a whole selection, groups expanded to their leaves |
| `ResizeElement { id, new }` | the new rectangle, normalized so a negative drag is still a rectangle |
| `RotateElement { id, radians }` | an absolute angle about the box centre |
| `RestyleElements { ids, style }` | one whole style register per element (`TOILE-018`) |
| `SetText { id, text }` | the whole body of a note or a text element (`TOILE-030`) |
| `LockElements { ids, locked }` | the advisory lock, set or cleared |
| `DeleteElements { ids }` | the removal, cascading through a group's members |

**Steps, when an element is created:**

1. refuse a derived kind, and refuse a seed that does not match the kind (`TOILE-022`);
2. mint an id (`ambre-spec-toile-schema.md section 2`) and open the element map under it;
3. write the geometry block, the lock flag and the per-kind style, with the picked fill applied
   over it (`TOILE-035`);
4. seed the type-specific fields, and for a frame its membership list and its notes container
   (`features/ambre-spec-toile-frames.md`);
5. push the element onto the top of the paint order;
6. record the rotation-expanded box in the bounds cache and the spatial index (`TOILE-031`);
7. commit once, then answer with the minted id and a fresh visible set.

**Steps, when an element is deleted:**

1. expand every listed id to its whole subtree, so deleting a group takes its members
   (`features/ambre-spec-toile-groups.md`);
2. remove each element map, and stop when nothing was there;
3. drop each removed id from its container's membership list;
4. remove every connector that referenced a removed id at either endpoint;
5. strip both the elements and those connectors from the paint order;
6. drop the bounds and index entries, then prune a container the deletion emptied;
7. commit once (`TOILE-034`).

**The rotation-expanded box.** The bounds cache holds the axis-aligned box of the *rotated* element,
which is larger than its own rectangle. The index therefore over-approximates on purpose: the exact
geometry is tested on the UI thread (`features/ambre-spec-toile-selection.md`).

**Code:**

* `crates/ambre-toile/src/logic/state/elements.rs` -- every handler above, plus the commit-free
  variants an import batches into one undo item;
* `state/codec.rs` -- the rotation-expanded box, the per-kind seed and the field readers;
* `state/derive.rs` -- how each kind becomes something to paint.

---

## 4. SURFACE

| Trigger | Sends |
|---|---|
| a creation tool released on the board | `CreateElement` with the tool's kind (`features/ambre-spec-toile-tools.md`) |
| a drag on a selection | `MoveElements` once, on release |
| a drag on a corner or an edge handle | `ResizeElement` once, on release |
| a drag on the rotate handle | `RotateElement` once, on release |
| `Delete` or `Backspace` | `DeleteElements` over the selection |
| an arrow key, alone or with `Shift` | `MoveElements` by one world unit, or ten |
| the style controls | `RestyleElements` (`features/ambre-spec-toile-styles.md`) |
| Lock and Unlock, in the object menu or the context menu | `LockElements` (`features/ambre-spec-toile-menus.md`) |

**What a lock looks like.** A locked element shows no handles and cannot be picked, so it stops
absorbing clicks over a background someone drew on
(`features/ambre-spec-toile-selection.md`).

**Align and distribute** (`TOILE-037`) is designed and unbuilt: no command, no menu entry, no
button. What it will do is align a selection to a shared edge or centre of its union box, and space
three or more elements evenly along one axis, as one undo item.

---

## 5. DECISIONS

| Decision | Rationale | Rejected alternative |
|---|---|---|
| Every kind shares one lifecycle | a frame, a group and a note differ only by the content they carry, so a move is a move. A per-kind lifecycle would duplicate move, resize, rotate, delete and reorder once per kind, and split the paint order across as many structures | frame-specific and group-specific commands, which the earlier design carried |
| Align and distribute is a batch of coordinates, not an operation of its own | the result is a set of positions, and the positions are what converges. Modelling it as a stored alignment would make two peers fight over a constraint neither can see | a persisted alignment rule on the container |
| A create is one commit, and an import is one commit for the whole batch | a user placing a note wants one undo; a user importing a file wants one undo. Both need the same handler, so the write and the commit are separate steps | a commit inside every write, which makes an import of 400 elements 400 undo items |
| The lock refuses the write rather than hiding the element | locking exists so a user can draw over a background and keep clicking through it. Greying it out would make a board built of colour unreadable | a visual lock state, with the refusal on the transform |
| The bounds cache holds the rotated box, not the element's own | the index answers "what might be visible", and a rotated element sticks out of its own rectangle. Over-approximating there costs one exact test per candidate and never drops a hit | indexing the unrotated rectangle, which loses a rotated element at the edge of the screen |
| A rejected command answers an error and writes nothing | a partial write on a bad id is the one failure a convergent document cannot recover from, because the other peers receive it | a best-effort write over the ids that resolved |

---

## 6. NOT BUILT, AND NOT YET RULED

* **A lock stops a move and not a delete** (`TOILE-033`). The geometry and style handlers all
  read the lock flag and return early; the delete handler and the reorder handler do not read it at
  all. Select-all takes locked elements too, because it walks what is displayed rather than what is
  pickable (`TOILE-UI-033`), so `Ctrl+A` then `Delete` removes a locked background.
* **Align and distribute has no code** (`TOILE-037`). The command, its operation enumeration and the
  inspector row that would emit it are all specified and absent from the workspace.
* **Nothing asserts the per-kind default** (`TOILE-035`). A note is born yellow and square and
  everything else white; the only tests near it drive the picked colour and the paint, so a change to
  the seeded default would fail nothing.
* **A resize does not scale what is inside.** A resized frame keeps its children where they were, and
  a resized group is refused outright, because a group's box is derived
  (`features/ambre-spec-toile-groups.md`). Scaling a subtree is not designed.
* **No kind carries a name.** An element has no user-facing label, so a board of two hundred shapes
  offers nothing to search but the bodies of its notes (`features/ambre-spec-toile-find.md`).
