# AMBRE -- FEATURE: AUTO-LAYOUT

**Type:** Vertical (feature)
**Implementation:** partial -- the solver is whole and tested; nothing calls it and no surface sets a spec.

> A frame lays its children out by a rule instead of by their coordinates. The rule is a direction,
> a gap, a padding, an alignment and a sizing mode.

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

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Gap | Status |
|---|---|---|---|---|---|---|
| `TOILE-FEAT-005` | `INV` | Children MUST run along the rule's direction, in the frame's own order, one gap apart. | this spec section 3 | `ambre_toile::logic::autolayout::tests::horizontal_start_fixed_with_gap`; `::vertical_start_fixed_with_gap` -- each reads two boxes back where the second starts one gap past the first. The eleven other tests this row cited hold the eleven rules below, one each | none | verified |
| `TOILE-FEAT-038` | `INV` | A child MUST keep its size across the direction. | this spec section 3 | `ambre_toile::logic::autolayout::tests::horizontal_start_fixed_with_gap`; `::vertical_start_fixed_with_gap` -- what the row above does not read: a ten-tall and a forty-tall child come back at their own heights | none | verified |
| `TOILE-FEAT-102` | `INV` | A child MUST sit at the near edge of the direction it does not lay out along. | this spec section 3 | `ambre_toile::logic::autolayout::tests::horizontal_start_fixed_with_gap`; `::vertical_start_fixed_with_gap` -- both children come back against the near edge. Nothing centres a row of unequal children, section 6 | none | verified |
| `TOILE-FEAT-039` | `INV` | The children MUST go inside the frame's padding: the run MUST start at the padded corner. | this spec section 3 | `ambre_toile::logic::autolayout::tests::padding_offsets_origin` -- one child in a frame padded differently on each of its four sides, read back at the near two | none | verified |
| `TOILE-FEAT-040` | `INV` | The room the children have MUST be the padded room, so a far-packed run MUST stop at the inner edge and a wrapping run MUST break there. | this spec section 3 | none | unheld -- the one padded test packs a single child at the near edge with 140 units to spare, so a solve that subtracted the near padding and ignored the far one would pass it. No other test pads anything; lands with the `The Toile belts` Story (`ambre-roadmap.md` E9) | untested |
| `TOILE-FEAT-041` | `INV` | The alignment MUST spend the room the children leave: packed near, centred, packed far, or spread with the two ends pinned. | this spec section 3 | `ambre_toile::logic::autolayout::tests::align_center`; `::align_end`; `::align_space_between` -- each reads every box back; `::horizontal_start_fixed_with_gap`; `::vertical_start_fixed_with_gap` -- the packed-near case, the two tests of `TOILE-FEAT-005` | none | verified |
| `TOILE-FEAT-042` | `INV` | Under the spreading alignment, a run of fewer than two children MUST pack at the near edge. | this spec section 3 | `ambre_toile::logic::autolayout::tests::space_between_single_child_falls_back_to_start` -- a spread with no interior gap has nothing to divide the room by, and this row is what happens instead of dividing | none | verified |
| `TOILE-FEAT-043` | `INV` | Under the filling mode, the children MUST share the leftover room equally. | this spec section 3 | `ambre_toile::logic::autolayout::tests::sizing_fill_shares_leftover` -- it reads two widths of forty-five back from two children ten wide | none | verified |
| `TOILE-FEAT-103` | `INV` | Under the filling mode, a child MUST stretch across the direction to the padded box. | this spec section 3 | `ambre_toile::logic::autolayout::tests::sizing_fill_shares_leftover` -- it reads a height of fifty back from a child five tall | none | verified |
| `TOILE-FEAT-044` | `INV` | Under the hugging and the fixed modes, each child MUST keep its own size along the direction. | this spec section 3 | `ambre_toile::logic::autolayout::tests::sizing_hug_equals_fixed` -- the two modes agree; `::horizontal_start_fixed_with_gap` -- the size is the child's own, where agreement alone would hold two modes that were both wrong | none | verified |
| `TOILE-FEAT-045` | `INV` | A child MUST NOT be shrunk to fit: children too big for the frame MUST overflow it at their own size. | this spec section 3 | none | unheld -- the filling mode has a floor of its own on its share of the leftover, and no test fills a frame too small for its children. The floor the other modes share is held two rows down (`TOILE-FEAT-048`); lands with the `The Toile belts` Story (`ambre-roadmap.md` E9) | untested |
| `TOILE-FEAT-046` | `INV` | Under the wrapping direction, a child that would not fit the row MUST start a new one, and rows MUST stack one gap apart, each as tall as its tallest child. | this spec section 3 | `ambre_toile::logic::autolayout::tests::wrap_breaks_rows` -- its third child wraps and comes back one gap below a row as tall as its tallest member | none | verified |
| `TOILE-FEAT-047` | `INV` | Under the wrapping direction, the alignment MUST apply to each row on its own. | this spec section 3 | `ambre_toile::logic::autolayout::tests::wrap_align_per_row` -- its two rows hold two children and one and are each packed far against the same edge | none | verified |
| `TOILE-FEAT-048` | `INV` | Spacing MUST never go negative: a run that overflows MUST extend past the frame rather than fold back on itself. | this spec section 3 | `ambre_toile::logic::autolayout::tests::overflow_does_not_produce_negative_spacing` -- the spreading alignment, where one floor guards all four: the room the alignment spends is measured once | none | verified |
| `TOILE-FEAT-049` | `INV` | A frame holding no children MUST place nothing, whatever its rule. | this spec section 3 | `ambre_toile::logic::autolayout::tests::empty_children_is_empty` -- it was cited under the combination row as though it held one of the combinations | none | verified |
| `TOILE-FEAT-030` | `INV` | A solved position MUST NOT reach the document. | this spec section 3 | none | unheld -- no container holds a solved box and no code writes one, so the rule holds by absence, and nothing would go red the day a writer appeared; lands with the `The Toile belts` Story (`ambre-roadmap.md` E9) | untested |
| `TOILE-FEAT-009` | `CONC` | Concurrent edits of one frame's layout MUST converge, last writer winning on the whole rule. | this spec section 3 | `ambre_toile::logic::convergence_tests::concurrent_auto_layout_edits_converge` -- two peers set rival rules and agree afterwards. The second test this row cited drives one peer alone, so it holds the write instead (`TOILE-FEAT-050`) | none | verified |
| `TOILE-FEAT-029` | `OBS` | A frame carrying a layout rule MUST place its children by that rule. | this spec section 3 | none | fails -- the solver has no caller outside its own tests, so the rule is stored, converges, and moves nothing on screen, section 6. It lands with the `Auto-layout reaches a frame` Story (`ambre-roadmap.md` E6) | failing |

### 1.2 Give a frame a layout rule

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Gap | Status |
|---|---|---|---|---|---|---|
| `TOILE-FEAT-050` | `POST` | After a set the frame MUST carry exactly the rule given. | this spec section 3 | `ambre_toile::logic::convergence_tests::set_auto_layout_add_change_clear` -- it reads the rule back after an add and after a change. It is the test `TOILE-FEAT-009` cited for a convergence it never drives | none | verified |
| `TOILE-FEAT-099` | `POST` | After a clear the frame MUST carry no rule. | this spec section 3 | `ambre_toile::logic::convergence_tests::set_auto_layout_add_change_clear` -- its clear step: the frame is read back holding none | none | verified |
| `TOILE-FEAT-051` | `POST` | A set MUST be one undo item: one `Ctrl+Z` MUST bring back the rule the frame carried before it. | this spec section 3 | `ambre_toile::logic::convergence_tests::set_auto_layout_add_change_clear` -- its undo step undoes a change and reads the prior rule back, on the one commit the handler makes for every set. The undo of an add and the undo of a clear are not driven | none | verified |
| `TOILE-FEAT-031` | `POST` | A user MUST be able to give a frame a layout rule, change it and clear it. | this spec section 4 | none | fails -- the command has no menu entry, no inspector row and no chord anywhere in the workspace, section 6. It lands with the `Auto-layout reaches a frame` Story (`ambre-roadmap.md` E6) | failing |
| `TOILE-FEAT-052` | `FRAME` | A rule identical to the one already stored MUST NOT be written, and MUST NOT reach the other users. | this spec section 3 | none | unheld -- the handler compares the encoded rule and returns before its commit, and every set in the tests differs from the rule before it. The clear half of the same guard is held by the row below; lands with the `The Toile belts` Story (`ambre-roadmap.md` E9) | untested |
| `TOILE-FEAT-053` | `FRAME` | Clearing a rule a frame does not carry MUST write nothing, and MUST reach nobody. | this spec section 3 | `ambre_toile::logic::convergence_tests::set_auto_layout_add_change_clear` -- its last step clears an already-cleared frame and asserts that no delta leaves | none | verified |
| `TOILE-FEAT-032` | `FAIL` | Only a frame MUST carry a layout rule: any other element MUST refuse one, and MUST say so. | this spec section 3 | `ambre_toile::logic::convergence_tests::set_auto_layout_rejects_non_frame` -- the kind-mismatch notice and the absent delta | none | verified |
| `TOILE-FEAT-054` | `FAIL` | A rule aimed at an element the board does not hold MUST be refused. | this spec section 3 | `ambre_toile::logic::convergence_tests::set_auto_layout_rejects_non_frame` -- its second half reads the absent delta alone. That refusal sends a notice of its own and no test reads it, which is beyond what the rule claims | none | verified |

* **Budget.** the solve under 1 ms at 200 children, a row of
  `ambre-spec-budgets.md section 1`. It is a budget for a call nobody makes
  (`TOILE-FEAT-029`).
* **Boundary.** none of its own: a layout rule is written to the document, and `ZK-001` seals every
  payload before it leaves the client.
* **Verification.** 20 `verified`, 2 `failing`, 4 `untested`. What holds is the solver: sixteen tests,
  thirteen of them one per combination the solver pins, each falsifiable on its own. What is loose is
  what those thirteen never reach: the far padding no test subtracts, and the frame too small no test
  fills. The unchanged set that must write nothing and the size a child keeps across the direction
  are loose too. The two failures are a write held for a convergence (`TOILE-FEAT-009`) and two
  refusals held to two standards (`TOILE-FEAT-032`).

---

## 2. WHAT EACH USER SEES

| Alice does | Bob does | The document holds | Each user sees | Test |
|---|---|---|---|---|
| gives a frame a horizontal rule | gives the same frame a vertical one | one rule, the later write | the same rule on both boards (`TOILE-FEAT-009`) | `logic::convergence_tests::concurrent_auto_layout_edits_converge` |
| clears the rule | watches | no rule on the frame | free positioning again (`TOILE-FEAT-050`) | `logic::convergence_tests::set_auto_layout_add_change_clear` |
| gives a group a rule | watches | nothing | the request refused, and no delta (`TOILE-FEAT-032`) | `logic::convergence_tests::set_auto_layout_rejects_non_frame` |
| gives a frame a rule | watches | the rule | the children where they already were (`TOILE-FEAT-029`) | none -- section 6 |

---

## 3. MODEL

**Container.** One `autolayout` key per frame element, a binary value register
(`ambre-spec-toile-schema.md section 3`). An absent key means free positioning. It is a value and not
a container. Two peers writing at once converge by the map's own last-writer-wins, and no seeding is
owed. `TOILE-029` is the rule for the containers that do owe one.

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

1. refuse an element that is not a frame, so a group keeps free positioning (`TOILE-FEAT-032`), and
   refuse an element the board does not hold (`TOILE-FEAT-054`);
2. stop when the encoded rule equals the one already stored, so an unchanged rule is not an undo
   item (`TOILE-FEAT-052`). A clear with nothing to clear stops the same way (`TOILE-FEAT-053`);
3. write or delete the one key (`TOILE-FEAT-050`), commit once, so the set is one undo item
   (`TOILE-FEAT-051`), and ask for a fresh viewport.

**Steps, when a frame is solved:**

1. inset the frame box by its padding: that box is where the children go (`TOILE-FEAT-039`). It is
   the room the alignment and the wrap measure themselves against (`TOILE-FEAT-040`);
2. size each child on the main axis -- its own size (`TOILE-FEAT-044`), or an equal share of the
   leftover under `Fill` (`TOILE-FEAT-043`);
3. place the run from the alignment: packed at the start, centred, packed at the end, or spread
   with the two ends pinned (`TOILE-FEAT-041`). A spread of fewer than two children packs at the
   start instead (`TOILE-FEAT-042`);
4. keep each child's cross size and start-align it (`TOILE-FEAT-038`), stretching only under `Fill`;
5. break the run into rows under `Wrap` (`TOILE-FEAT-046`), and align each row on its own
   (`TOILE-FEAT-047`).

The run follows the frame's own child list, along the rule's direction and one gap apart
(`TOILE-FEAT-005`). A frame holding no children solves to nothing (`TOILE-FEAT-049`). Overflow
never produces negative spacing: the children simply extend past the box (`TOILE-FEAT-048`), and no
child is shrunk to fit (`TOILE-FEAT-045`).

**The solve is deterministic**, and it is read-only: identical state gives every peer identical
boxes, and no box is written back (`TOILE-FEAT-030`).

**Code:**

* `crates/ambre-toile/src/logic/autolayout.rs` -- the solver;
* `logic/state/autolayout.rs` -- the register, its guard and its accessor.

---

## 4. SURFACE

**There is none** (`TOILE-FEAT-031`). No menu entry, no inspector row, no floating-bar control and no
chord emits the command. The style inspector that an earlier design placed it in
(`features/ambre-spec-toile-styles.md section 4`) carries no layout control. The rule can be written
by a test and by nothing a user can reach.

A frame with a rule is painted like a frame without one, because the solve never runs
(`TOILE-FEAT-029`). There is nothing on screen that says a frame carries a rule at all.

---

## 5. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| The rule is shared and the solved boxes are not | writing the solved boxes into the document | they desynchronize from the rule they came from: the boxes are a function of the rule and of the children, so a second copy is a second truth that drifts the first time a write is lost. The rule is design intent, so it is the half that must converge |
| The rule is one value, not a map of fields | a field per key | it converges to a layout no peer wrote. The five fields are one decision and are always read together, so one register makes a concurrent edit resolve to one whole rule instead of a mixture neither author asked for |
| Only a frame carries a rule | letting a group carry one | it needs a derived box the group does not have: a group is a transform binding with no box of its own, so it has nothing to lay children out inside. Promoting it to a frame is the way to get a rule |
| The children order is the frame's own list | a separate layout order | it is a second list to keep convergent, and the frame's own list is already the order the author set and the one the paint pass follows |
| The solver stays in the tree, frozen | removing it | it drops working geometry that is complete, passes, and costs nothing to keep |
| The solver stays in the tree, frozen | wiring it up and promising it | stabilizing it spends an MVP budget on another product's job |
| Hug and Fixed are one case for the solver | growing the frame from its children | it makes the frame box an output and the layout a fixed point. The frame box is an input here, so a rule that hugs its content and one that keeps a fixed size place the children identically |

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
* **The far padding is subtracted by nothing** (`TOILE-FEAT-040`). One test pads a frame, and it
  packs a single child at the near corner with room to spare. The right and the bottom insets are
  never read back. They are what a far-packed run stops at and what a wrapping run breaks at, and a
  solve that dropped them passes all thirteen tests.
* **Nothing overflows a filled frame** (`TOILE-FEAT-045`). The share of the leftover has a floor of
  its own, and every filling test has room to give. A frame too small for its children would shrink
  them below their own size without that floor, and the overflow test drives the other one.
* **An unchanged rule is a guard nothing drives** (`TOILE-FEAT-052`). Re-writing a byte-identical
  rule returns before the commit, so it is not an undo item and no delta leaves. The clearing half
  of the same guard is driven and this half is not. It is the shape of the membership change that
  changes nothing (`features/ambre-spec-toile-frames.md`, `TOILE-102`).
* **Two refusals, two standards** (`TOILE-FEAT-032`, `TOILE-FEAT-054`). A rule aimed at a group
  reports a kind mismatch and the test reads the notice. A rule aimed at an element the board does
  not hold reports an unknown element, and the test reads the absent delta alone.
* **A frame that hugs its content cannot grow.** The solver takes the frame box as an input, so
  `Hug` and `Fixed` place children the same way. The rule the name promises -- a frame that shrinks
  to what it holds -- needs the frame box to become an output. No code makes it one.
* **The cross axis has no alignment of its own.** One alignment knob is spent on the main axis and a
  child is start-aligned across it, unless the rule fills. A user who wants a centred row of
  different heights cannot ask for it.
