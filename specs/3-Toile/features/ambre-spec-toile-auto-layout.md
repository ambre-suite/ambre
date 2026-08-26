# AMBRE -- FEATURE: AUTO-LAYOUT

**Type:** Vertical (feature)
**Implementation:** partial -- the solver is whole and tested; nothing calls it and no surface sets a spec.

> A frame lays its children out by a rule instead of by their coordinates: a direction, a gap, a
> padding, an alignment and a sizing mode.

**Frozen.** Laying out a frame by constraints is the vector-fabrication job, not the whiteboard one
(`features/ambre-spec-toile-engine.md section 5`). The code stays in the tree, earns no stabilization
budget and carries no
promise; it is a candidate for a later version or for removal.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`. The frame this feature constrains is
`features/ambre-spec-toile-frames.md`, and the group that is not a frame is
`features/ambre-spec-toile-groups.md`.

| ID | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|
| `TOILE-FEAT-005` | The solver MUST honour every combination of direction, sizing and alignment, and MUST NOT produce negative spacing on overflow. | this spec section 3 | `ambre_toile::logic::autolayout::tests::overflow_does_not_produce_negative_spacing`, `::wrap_breaks_rows`, `::wrap_align_per_row`, `::sizing_fill_shares_leftover`, `::sizing_hug_equals_fixed`, `::align_center`, `::align_end`, `::align_space_between`, `::space_between_single_child_falls_back_to_start`, `::padding_offsets_origin`, `::horizontal_start_fixed_with_gap`, `::vertical_start_fixed_with_gap`, `::empty_children_is_empty` | verified |
| `TOILE-FEAT-009` | Concurrent edits of one frame's layout MUST converge, last writer winning on the whole rule. | this spec section 3 | `ambre_toile::logic::convergence_tests::concurrent_auto_layout_edits_converge`, `::set_auto_layout_add_change_clear` | verified |
| `TOILE-FEAT-032` | Only a frame MUST carry a layout rule; any other element MUST refuse one. | this spec section 3 | `ambre_toile::logic::convergence_tests::set_auto_layout_rejects_non_frame` | verified |
| `TOILE-FEAT-030` | A solved position MUST NOT reach the document. | this spec section 3 | none. No container holds a solved box and no code writes one, so the rule holds by absence; nothing would go red the day a writer appeared | unverified |
| `TOILE-FEAT-029` | A frame carrying a layout rule MUST place its children by that rule. | this spec section 3 | none, and **the rule fails today**: the solver has no caller outside its own tests, so the rule is stored, converges, and moves nothing on screen -- section 6 | unverified |
| `TOILE-FEAT-031` | A user MUST be able to give a frame a layout rule, change it and clear it. | this spec section 4 | none, and **the rule fails today**: the command has no menu entry, no inspector row and no chord anywhere in the workspace -- section 6 | unverified |

* **Budget.** the solve under 1 ms at 200 children, a row of
  `ambre-spec-budgets.md section 1`. It is a budget for a call nobody makes
  (`TOILE-FEAT-029`).
* **Boundary.** none of its own: a layout rule is written to the document, and `ZK-001` seals every
  payload before it leaves the client.
* **Verification.** 3 `verified`, 3 `unverified`. The solver is held combination by combination and
  the register converges. The three gaps are one shape: the rule is a value the document carries and
  nothing on either side of it -- no writer, no reader -- exists.

---

## 2. WHAT EACH USER SEES

| Alice does | Bob does | The document holds | Each user sees | Test |
|---|---|---|---|---|
| gives a frame a horizontal rule | gives the same frame a vertical one | one rule, the later write | the same rule on both boards (`TOILE-FEAT-009`) | `logic::convergence_tests::concurrent_auto_layout_edits_converge` |
| clears the rule | watches | no rule on the frame | free positioning again (`TOILE-FEAT-009`) | `logic::convergence_tests::set_auto_layout_add_change_clear` |
| gives a group a rule | watches | nothing | the request refused, and no delta (`TOILE-FEAT-032`) | `logic::convergence_tests::set_auto_layout_rejects_non_frame` |
| gives a frame a rule | watches | the rule | the children where they already were (`TOILE-FEAT-029`) | none -- section 6 |

---

## 3. MODEL

**Container.** One `autolayout` key per frame element, a binary value register
(`ambre-spec-toile-schema.md section 3`). An absent key means free positioning. It is a value and not
a container, so two peers writing at once converge by the map's own last-writer-wins and no seeding
is owed (`TOILE-029` is the rule for the containers that do owe one).

**The rule** (`ambre-bus`):

```rust
pub struct AutoLayout {
    pub direction: Direction,   // Horizontal | Vertical | Wrap
    pub gap:       f64,
    pub padding:   [f64; 4],    // top, right, bottom, left
    pub align:     Align,       // Start | Center | End | SpaceBetween
    pub sizing:    Sizing,      // Hug | Fill | Fixed
}
```

**Command** (`ambre-bus`): `SetAutoLayout { frame, layout }`, where an absent layout clears the rule.
One commit, so one undo item.

**Steps, when the rule is set:**

1. refuse an element that is not a frame, so a group keeps free positioning (`TOILE-FEAT-032`);
2. stop when the encoded rule equals the one already stored, so an unchanged rule is not an undo
   item;
3. write or delete the one key, commit once, and ask for a fresh viewport.

**Steps, when a frame is solved:**

1. inset the frame box by its padding: that box is where the children go;
2. size each child on the main axis -- its own size, or an equal share of the leftover under `Fill`;
3. place the run from the alignment: packed at the start, centred, packed at the end, or spread with
   the two ends pinned;
4. keep each child's cross size and start-align it, stretching only under `Fill`;
5. break the run into rows under `Wrap`, and align each row on its own.

Overflow never produces negative spacing: the children simply extend past the box
(`TOILE-FEAT-005`).

**The solve is deterministic**, and it is read-only: identical state gives every peer identical
boxes, and no box is written back (`TOILE-FEAT-030`).

**Code:**

* `crates/ambre-toile/src/logic/autolayout.rs` -- the solver;
* `logic/state/autolayout.rs` -- the register, its guard and its accessor.

---

## 4. SURFACE

**There is none** (`TOILE-FEAT-031`). No menu entry, no inspector row, no floating-bar control and no
chord emits the command; the style inspector that an earlier design placed it in
(`features/ambre-spec-toile-styles.md section 4`) carries no layout control. The rule can be written
by a test and by nothing a user can reach.

A frame with a rule is painted like a frame without one, because the solve never runs
(`TOILE-FEAT-029`). There is nothing on screen that says a frame carries a rule at all.

---

## 5. DECISIONS

| Decision | Rationale | Rejected alternative |
|---|---|---|
| The rule is shared and the solved boxes are not | the rule is design intent, so it must converge; the boxes are a function of it and of the children, so a second copy is a second truth that drifts the first time a write is lost | writing the solved boxes into the document, which desynchronizes from the rule it came from |
| The rule is one value, not a map of fields | the five fields are one decision and are always read together, so one register makes a concurrent edit resolve to one whole rule instead of a mixture neither author asked for | a field per key, which converges to a layout no peer wrote |
| Only a frame carries a rule | a group is a transform binding with no box of its own, so it has nothing to lay children out inside. Promoting it to a frame is the way to get a rule | letting a group carry one, which needs a derived box the group does not have |
| The children order is the frame's own list | it is already the order the author set and the one the paint pass follows, so the layout needs no second ordering | a separate layout order, which is a second list to keep convergent |
| The solver stays in the tree, frozen | it is complete, it passes, and it costs nothing to keep; removing it discards working geometry and stabilizing it spends an MVP budget on another product's job | removing it, or wiring it up and promising it |
| Hug and Fixed are one case for the solver | the frame box is an input here, not something the solver grows, so a rule that hugs its content and one that keeps a fixed size place the children identically | growing the frame from its children, which makes the frame box an output and the layout a fixed point |

---

## 6. NOT BUILT, AND NOT YET RULED

* **The solver has no caller** (`TOILE-FEAT-029`). Thirteen tests cover every combination of
  direction, alignment and sizing; the derivation reads each element's stored coordinates and calls
  none of it. A frame's rule converges between peers and changes nothing anyone can see.
* **Nothing sets a rule** (`TOILE-FEAT-031`). Same shape as the boolean combinators
  (`TOILE-FEAT-018`): a command, a guard, a convergence belt, and no gesture. The freeze explains why
  no surface was built; it does not explain why the corpus described one.
* **The corpus said the solved boxes were written back.** The earlier design stated both that the
  boxes are derived and never persisted, and that dropping an element commits the resolved geometry.
  Neither exists in code. The first is the rule this document keeps (`TOILE-FEAT-030`); the second is
  withdrawn.
* **A frame that hugs its content cannot grow.** The solver takes the frame box as an input, so
  `Hug` and `Fixed` place children the same way. The rule the name promises -- a frame that shrinks
  to what it holds -- needs the frame box to become an output, which no code does.
* **The cross axis has no alignment of its own.** One alignment knob is spent on the main axis and a
  child is start-aligned across it, unless the rule fills. A user who wants a centred row of
  different heights cannot ask for it.
