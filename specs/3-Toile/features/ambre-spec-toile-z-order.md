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

| ID | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|
| `TOILE-057` | One order MUST decide the painting of elements and connectors alike, so a connector can be placed behind an element. | this spec section 3 | `ambre_toile::logic::convergence_tests::reorder_to_back_moves_node`; the shared entry encoding is `ambre-spec-toile-schema.md section 2` | verified |
| `TOILE-058` | An element the document holds and the order omits MUST still be painted, last, in an order every peer computes alike. | this spec section 3 | none, and **the rule fails today**: the visible-set walk reads the order and nothing else, so an element missing from it is invisible on every peer. The search carries the fallback the walk does not -- section 6 | unverified |
| `TOILE-059` | An order entry whose target is gone MUST be skipped, never painted as a hole. | this spec section 3 | `ambre_toile::logic::connector::tests::missing_endpoint_yields_none`, `ambre_toile::logic::convergence_tests::deleting_an_endpoint_makes_the_connector_dangle`, `::caches_match_a_full_rebuild` | verified |
| `TOILE-060` | A node MUST appear at most once in the order. | this spec section 3 | none. A create pushes once and the materialization keeps the first occurrence of a repeat, so a duplicate cannot reach the paint walk; nothing stops one reaching the document, and no test drives either | unverified |
| `TOILE-061` | Removing a node MUST NOT rewrite the rest of the order. | `ambre-spec-budgets.md section 1`; this spec section 6 | none, and **the rule fails today**: the removal deletes the whole list and re-pushes the survivors, so deleting one element sends an operation per surviving node to every peer -- section 6 | unverified |

* **Budget.** a remote change under 2 ms, a row of `ambre-spec-budgets.md section 1`.
  `TOILE-061` is what breaks it: a delete on a large board produces a change proportional to the
  board rather than to the deletion, and no bench measures it.
* **Boundary.** none of its own: the order is a list in the Logic thread's document and leaves as
  bytes the Sync worker seals (`ZK-001`).
* **Verification.** 2 `verified`, 4 `unverified`. The order itself and the skipping of a dead entry
  are held. Everything about its edges is loose, and two of the four are rules the code answers with
  the opposite.

---

## 2. WHAT EACH USER SEES

| Alice does | Bob does | The document holds | Each user sees | Test |
|---|---|---|---|---|
| brings a shape to the front | brings another shape to the front | both moves, in the order they converged | the same stacking on both screens | `logic::convergence_tests::two_peers_converge` |
| sends a shape to the back | deletes that shape | the deletion | the shape gone, and no gap in the order | `logic::convergence_tests::reorder_to_back_moves_node` |
| adds a shape to a frame | sends that shape to the back | the membership, and the order clamped so the child stays above its frame | the shape above its frame (`TOILE-011`) | `logic::convergence_tests::frame_children_paint_after_their_frame` |
| deletes one element on a board of many | watches | the deletion, and a rewritten order | the same board, at the cost of a change proportional to the board (`TOILE-061`) | none -- section 6 |

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

1. find the node's current place, and stop when it has none;
2. resolve the target to an index, clamped to the list -- front is the last place, back the first,
   forward and backward one step;
3. stop when the target is where the node already is;
4. move the entry, commit, and re-materialize the order.

**Steps, when the order is re-materialized:** read the list, drop an entry whose tag does not decode,
and keep the first occurrence of a repeated node (`TOILE-060`). The result is the sequence the paint
walk and the pick walk both use, front being the end.

**How a node is removed.** The delete handler rewrites the list: it collects the survivors, deletes
every entry, and pushes the survivors back. That is where `TOILE-061` fails, and it is the reason a
delete is the most expensive change on the board.

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
at a time, so the four entries produce one command per element and one undo item per command.

**What no surface offers.** Reordering a connector, because no connector can be selected
(`TOILE-UI-035`), and placing a node at an absolute index, which the command accepts and nothing
sends.

---

## 5. DECISIONS

| Decision | Rationale | Rejected alternative |
|---|---|---|
| One order for elements and connectors | an arrow behind a shape is an ordinary diagram, and two orders make the relative depth of an arrow and a shape unrepresentable | a connector pass painted after every element |
| The order is a movable list, not a number per element | a number per element makes an insertion between two neighbours a renumbering, and two peers inserting at the same depth produce a tie no rule breaks. A list move is one operation and converges by position | a depth field on each element |
| An entry carries a tag and an id | one list has to address two id types, and a tag is the cheapest discriminator that keeps the entry a string the format can read back | two parallel lists, which restores the problem the single order exists to remove |
| An unlisted element paints last, ordered by id | the order can lose an entry to a concurrent change, and an element that exists must be visible. Ordering the remainder by id makes every peer agree without any repair write | repairing the order on read, which turns a paint into a write |
| A dead entry is skipped, not repaired | the same reason: derivation reads, and the element may come back with a late-arriving change | pruning the entry, which destroys data on a merge |
| The de-duplication keeps the first occurrence | it is the deterministic half of the rule, and it is where the node's neighbours expect it | keeping the last, which moves the node on every materialization |

---

## 6. NOT BUILT, AND NOT YET RULED

* **An unlisted element is invisible** (`TOILE-058`). The design says such an element paints last,
  ordered by id, and the visible-set walk iterates the materialized order only, so an element whose
  entry a concurrent change dropped disappears from every screen while staying in the document. The
  search carries exactly the fallback the paint walk lacks, which is where the shape of the fix is
  already written.
* **A delete rewrites the whole order** (`TOILE-061`). The removal deletes every entry and re-pushes
  the survivors, so deleting one element on a board of fifty thousand nodes produces fifty thousand
  list operations, encrypted and broadcast to every peer, and does so again for the next delete
  (`ambre-roadmap.md`, "Engine and render optimization").
* **Nothing stops a duplicate reaching the document** (`TOILE-060`). The materialization hides one, so
  a duplicate is invisible until something counts the list, and the paint walk would otherwise paint
  the node twice.
* **There is no layer panel.** A user reorders by four menu entries and cannot see the order, so a
  stack of five overlapping shapes is arranged by trial.
* **A multi-selection reorder is one command per element.** The four entries fan out over the
  selection, so bringing five shapes to the front takes five undos and interleaves them with whatever
  else sat between them.
