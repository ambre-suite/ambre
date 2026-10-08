# AMBRE -- FEATURE: Z-ORDER, THE ONE PAINT ORDER

**Type:** Vertical (feature)
**Implementation:** partial -- reordering ships; a delete rewrites the whole order.

> What overlaps what. A user brings a shape to the front or sends it to the back, and every
> collaborator sees the same stacking.

One list decides the order for everything on the board, arrows included. That is why an arrow can sit
behind a shape instead of always over it.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TOILE-060` | `INV` | A node MUST appear at most once in the order. | this spec section 3 | none | unheld -- a create pushes once, so a duplicate can only arrive from a concurrent change, nothing on the write path rejects one, and no test drives it; lands with the `Toile board structure held` Story (`ambre-roadmap.md` E9) | untested |
| `TOILE-123` | `CONC` | Two peers that have exchanged their changes MUST hold the same order. | this spec section 3 | `ambre_toile::logic::convergence_tests::two_peers_converge` -- it reads both orders back after random concurrent op sequences; `::caches_match_a_full_rebuild` -- it reads the incremental order against a rebuild of the same document. The second is what `TOILE-059` cited: it compares two walks, so a hole painted by both would pass it | none | verified |
| `TOILE-057` | `OBS` | One order MUST decide the painting of elements and connectors alike, so a connector can be placed behind an element. | this spec section 3 | `ambre_toile::logic::convergence_tests::delete_drops_owned_connectors` -- it reads a three-entry order back over two elements and one connector, the one place the two kinds are asserted to share the list, and the walk reads that list and no other. `::reorder_to_back_moves_node`, cited here before, carries no connector. The shared entry encoding is `ambre-spec-toile-schema.md section 2` | none | verified |
| `TOILE-119` | `OBS` | The order MUST paint back to front: the last entry MUST paint over the first. | this spec section 3 | none | unheld -- it is the sense the four menu entries are named for, and no test reads a derived viewport back to see which of two overlapping nodes lands last. `ambre_toile::logic::convergence_tests::reorder_to_back_moves_node` reads the materialized list, never a paint sequence; lands with the `Toile board structure held` Story (`ambre-roadmap.md` E9) | untested |
| `TOILE-059` | `OBS` | An order entry naming a connector the board can no longer route MUST be skipped, never painted as a hole. | this spec section 3 | `ambre_toile::logic::convergence_tests::deleting_an_endpoint_makes_the_connector_dangle` -- it reads the connector out of the paint set after one endpoint is deleted; `ambre_toile::logic::connector::tests::missing_endpoint_yields_none` -- the routing step under it | none | verified |
| `TOILE-120` | `OBS` | An order entry naming an element the document no longer holds MUST be skipped. | this spec section 3 | none | unheld -- it is the half `TOILE-059` was carrying. The two are different mechanisms: a connector is routed on every walk, while an element is culled against a bounds cache the deletion already emptied, so they fail apart. A local delete strips the entry, so the case arrives only from a concurrent change, and no test drives one; lands with the `Toile board structure held` Story (`ambre-roadmap.md` E9) | untested |
| `TOILE-058` | `OBS` | An element the document holds and the order omits MUST still be painted, last. | this spec section 3 | `ambre_toile::logic::state::derive::tests::an_omitted_element_paints_last_in_id_order` -- it drops four elements' layer entries and reads the derived viewport back: each still paints, after the listed element | none | verified |
| `TOILE-121` | `OBS` | The elements the order omits MUST be painted in an order every peer computes alike, by id. | this spec section 3 | `ambre_toile::logic::state::derive::tests::an_omitted_element_paints_last_in_id_order` -- it reads the omitted tail back and asserts it is in id order, not the element map's iteration order | none | verified |
| `TOILE-122` | `OBS` | A node the order lists twice MUST be painted once, at its first place. | this spec section 3 | none | unheld -- the materialization keeps the first occurrence, which is what makes a duplicate invisible rather than doubled, and it is the half `TOILE-060` was carrying: one rule is what the document may hold, the other is what the screen does with it. No test drives a duplicate through either; lands with the `Toile board structure held` Story (`ambre-roadmap.md` E9) | untested |

### 1.2 Reorder a node

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TOILE-124` | `POST` | A node sent to the back MUST become the first entry of the order. | this spec section 3 | `ambre_toile::logic::convergence_tests::reorder_to_back_moves_node` -- it reads the whole order back after sending the top node of two to the back | none | verified |
| `TOILE-125` | `POST` | The four other targets MUST resolve against the list: front is the last place, forward and backward one step from where the node sits, and an index past the end clamps to the last place. | this spec section 3 | none | unheld -- only the back target is driven, and no surface sends an index at all (section 4), so the clamp is reachable from the command alone; lands with the `Toile board structure held` Story (`ambre-roadmap.md` E9) | untested |
| `TOILE-126` | `FRAME` | A reorder MUST NOT change the relative order of the nodes it does not name. | this spec section 3 | none | unheld -- the handler moves one entry and the neighbours close behind it, and the one test that drives a reorder holds two nodes, where a move and a swap give the same list; lands with the `Toile board structure held` Story (`ambre-roadmap.md` E9) | untested |
| `TOILE-127` | `FRAME` | A reorder whose target is the place the node already holds MUST write nothing and broadcast nothing. | this spec section 3 | none | unheld -- the handler returns before the commit when the resolved index is the current one, and no test sends a reorder with no effect; lands with the `Toile board structure held` Story (`ambre-roadmap.md` E9) | untested |
| `TOILE-128` | `FAIL` | A reorder naming a node the order does not hold MUST do nothing, and MUST NOT report an error. | this spec section 3 | none | unheld -- the handler returns with no event at all, which is what it returns for the no-op above, so a surface cannot tell the refusal from the change that was already made, section 6; lands with the `Toile board structure held` Story (`ambre-roadmap.md` E9) | untested |

### 1.3 Remove a node

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TOILE-061` | `FRAME` | Removing a node MUST NOT rewrite the rest of the order. | `ambre-spec-budgets.md section 1`; this spec section 3 | `ambre_toile::logic::state::elements::inner_tests::a_delete_rewrites_no_other_layer_entry` | none | verified |
| `TOILE-129` | `FRAME` | Removing a node MUST NOT drop an entry it was not asked to remove. | this spec section 3 | `ambre_toile::logic::state::elements::inner_tests::a_delete_preserves_an_undecodable_layer_entry` -- it forges an entry with an unknown tag, deletes an unrelated element, and reads the raw list back to see the entry survive | none | verified |

* **Budget.** a remote change under 2 ms, a row of `ambre-spec-budgets.md section 1`.
  A delete removes only its own entries, so the change it sends is proportional to the deletion
  (`TOILE-061`).
* **Boundary.** none of its own: the order is a list in the Logic thread's document and leaves as
  bytes the Sync worker seals (`ZK-001`).
* **Verification.** 8 `verified`, 8 `untested`. What holds is the order as data: the list, the
  agreed walk, and the fallback painting an omitted element last, by id. The cost of a removal holds
  too. What
  is loose is everything the reorder does: the four targets nobody drives, the neighbours it may not
  disturb. The write it owes nobody and the refusal that reads as a no-op are loose too.

---

## 2. WHAT EACH USER SEES

| Alice does | Bob does | The document holds | Each user sees | Test |
|---|---|---|---|---|
| brings a shape to the front | brings another shape to the front | both moves, in the order they converged | the same stacking on both screens (`TOILE-123`) | `logic::convergence_tests::two_peers_converge` |
| sends a shape to the back | deletes that shape | the deletion | the shape gone, and no gap in the order (`TOILE-124`, `TOILE-057`) | `logic::convergence_tests::reorder_to_back_moves_node`, `::delete_drops_owned_connectors` |
| adds a shape to a frame | sends that shape to the back | the membership, and the order clamped so the child stays above its frame | the shape above its frame (`TOILE-011`) | `logic::convergence_tests::frame_children_paint_after_their_frame` |
| deletes one element on a board of many | watches | the deletion, and the order minus its entry | the same board, at the cost of a change proportional to the deletion (`TOILE-061`) | `logic::state::elements::inner_tests::a_delete_rewrites_no_other_layer_entry` |

---

## 3. MODEL

**Containers.** The `layers` list, one entry per node, declared in
`ambre-spec-toile-schema.md section 3`. An entry joins a tag and an id, so one list addresses both
element kinds and connectors; the encoding is that document's section 2.

**Command** (`ambre-bus`): `Reorder { node, to }`, one undo item. The target is absolute or relative:

```rust
pub enum ZTarget { Front, Back, Forward, Backward, Index(usize) }
```

**Steps, when a node is reordered:**

1. find the node's current place, and stop when it has none, silently (`TOILE-128`);
2. resolve the target to an index, clamped to the list (`TOILE-124`, `TOILE-125`);
3. stop when the target is where the node already is, before any write (`TOILE-127`);
4. move the entry, which leaves the other nodes in their order (`TOILE-126`), commit, and
   re-materialize.

**Steps, when the order is re-materialized:** read the list, and drop an entry whose tag does not
decode. Keep the first occurrence of a repeated node (`TOILE-060`, `TOILE-122`). The result is the
sequence the paint walk and the pick walk both use, front being the end (`TOILE-119`). The walk then
drops the entries it cannot paint: a connector it cannot route (`TOILE-059`) and an element the
document no longer holds (`TOILE-120`).

**How a node is removed.** The delete handler finds the entries it removes and deletes each of them,
from the back. The rest of the list is never written (`TOILE-061`). An entry it cannot decode is
never one it removes, so it survives (`TOILE-129`). What a removal owes the order itself is `TOILE-034`
(`features/ambre-spec-toile-elements.md`).

**The frame clamp.** A child of a frame is kept above it, so a frame paints as a background rather
than over its own contents. The rule and its cost are
`features/ambre-spec-toile-frames.md`.

**Code:**

* `crates/ambre-toile/src/logic/state/elements.rs` -- the reorder handler;
* `logic/state/caches.rs` -- the materialization, the de-duplication and the removal rewrite;
* `logic/state/derive.rs` -- the walk that turns the order into what the UI paints;
* `crates/ambre-bus/src/app/toile.rs` -- the entry encoding and its decoder.

---

## 4. SURFACE

| Trigger | Sends |
|---|---|
| Bring to Front, Bring Forward, Send Backward, Send to Back, in the object menu | `Reorder` with the matching target, one per selected element (`features/ambre-spec-toile-menus.md`) |
| the same four in the context menu | the same commands |
| the same four in the style panel | the same commands (`features/ambre-spec-toile-styles.md`) |

**What the user sees.** The overlap changes and nothing else: the order carries no visible state of
its own, no layer list and no depth read-out. A selection of several elements is reordered one element
at a time. The four entries produce one command per element and one undo item per command.

**What no surface offers.** Reordering a connector, because no connector can be selected
(`TOILE-UI-035`), and placing a node at an absolute index, which the command accepts and nothing
sends.

---

## 5. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| One order for elements and connectors | a connector pass painted after every element | an arrow behind a shape is an ordinary diagram, and two orders make the relative depth of an arrow and a shape unrepresentable |
| The order is a movable list, not a number per element | a depth field on each element | an insertion between two neighbours becomes a renumbering, and two peers inserting at the same depth produce a tie no rule breaks. A list move is one operation and converges by position |
| An entry carries a tag and an id | two parallel lists | it restores the problem the single order exists to remove. One list has to address two id types, and a tag is the cheapest discriminator that keeps the entry a string the format can read back |
| An unlisted element paints last, ordered by id | repairing the order on read | it turns a paint into a write. The order can lose an entry to a concurrent change and an element that exists must be visible; ordering the remainder by id makes every peer agree without any repair write |
| A dead entry is skipped, not repaired | pruning the entry | it destroys data on a merge, and the element may come back with a late-arriving change. Derivation reads |
| The de-duplication keeps the first occurrence | keeping the last | it moves the node on every materialization; the first occurrence is the deterministic half of the rule, and it is where the node's neighbours expect it |

---

## 6. NOT BUILT, AND NOT YET RULED

* **Nothing stops a duplicate reaching the document** (`TOILE-060`, `TOILE-122`). The materialization
  hides one, so a duplicate is invisible until something counts the list, and the paint walk would
  otherwise paint the node twice.
* **A refusal and a no-op are the same event** (`TOILE-128`, `TOILE-127`). A reorder of a node the
  order does not hold and a reorder that changes nothing both return with no event. A surface has
  nothing to tell one from the other, and a menu entry that silently did nothing looks like one that
  worked.
* **Nothing reads the paint order back** (`TOILE-119`). Every test of the order reads the
  materialized list. No test reads a derived viewport to check that the last entry paints over
  the first. Four menu entries are named for a sense the contract now states and nothing verifies.
* **There is no layer panel.** A user reorders by four menu entries and cannot see the order, so a
  stack of five overlapping shapes is arranged by trial.
* **A multi-selection reorder is one command per element.** The four entries fan out over the
  selection. Bringing five shapes to the front takes five undos and interleaves them with whatever
  else sat between them.
