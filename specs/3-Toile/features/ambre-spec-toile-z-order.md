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

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|---|
| `TOILE-060` | `INV` | A node MUST appear at most once in the order. | this spec section 3 | none. A create pushes once, so a duplicate can only arrive from a concurrent change, and nothing on the write path rejects one; no test drives it | unverified |
| `TOILE-123` | `CONC` | Two peers that have exchanged their changes MUST hold the same order. | this spec section 3 | `ambre_toile::logic::convergence_tests::two_peers_converge` reads both orders back after random concurrent op sequences, and `::caches_match_a_full_rebuild` reads the incremental order against a rebuild of the same document. The second is what `TOILE-059` cited: it compares two walks, so a hole painted by both would pass it | verified |
| `TOILE-057` | `OBS` | One order MUST decide the painting of elements and connectors alike, so a connector can be placed behind an element. | this spec section 3 | `ambre_toile::logic::convergence_tests::delete_drops_owned_connectors` reads a three-entry order back over two elements and one connector, which is the one place the two kinds are asserted to share the list; the walk reads that list and no other. `::reorder_to_back_moves_node`, cited here before, carries no connector. The shared entry encoding is `ambre-spec-toile-schema.md section 2` | verified |
| `TOILE-119` | `OBS` | The order MUST paint back to front: the last entry MUST paint over the first. | this spec section 3 | none. It is the sense the four menu entries are named for, and no test reads a derived viewport back to see which of two overlapping nodes lands last; `::reorder_to_back_moves_node` reads the materialized list, never a paint sequence | unverified |
| `TOILE-059` | `OBS` | An order entry naming a connector the board can no longer route MUST be skipped, never painted as a hole. | this spec section 3 | `ambre_toile::logic::convergence_tests::deleting_an_endpoint_makes_the_connector_dangle` reads the connector out of the paint set after one endpoint is deleted, and `ambre_toile::logic::connector::tests::missing_endpoint_yields_none` holds the routing step under it | verified |
| `TOILE-120` | `OBS` | An order entry naming an element the document no longer holds MUST be skipped. | this spec section 3 | none, and it is the half `TOILE-059` was carrying. The two are different mechanisms: a connector is routed on every walk, while an element is culled against a bounds cache the deletion already emptied, so they fail apart. A local delete strips the entry, so the case arrives only from a concurrent change, and no test drives one | unverified |
| `TOILE-058` | `OBS` | An element the document holds and the order omits MUST still be painted, last. | this spec section 3 | none, and **the rule fails today**: the visible-set walk reads the order and nothing else, so an element missing from it is invisible on every peer. The search carries the fallback the walk does not -- section 6 | unverified |
| `TOILE-121` | `OBS` | The elements the order omits MUST be painted in an order every peer computes alike, by id. | this spec section 3 | none, and it is the half `TOILE-058` was carrying. It fails today with it, and not in the same way: painting the remainder in the element map's iteration order would satisfy the first rule and give two peers two screens | unverified |
| `TOILE-122` | `OBS` | A node the order lists twice MUST be painted once, at its first place. | this spec section 3 | none. The materialization keeps the first occurrence, which is what makes a duplicate invisible rather than doubled, and it is the half `TOILE-060` was carrying: one rule is what the document may hold, the other is what the screen does with it. No test drives a duplicate through either | unverified |

### 1.2 Reorder a node

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|---|
| `TOILE-124` | `POST` | A node sent to the back MUST become the first entry of the order. | this spec section 3 | `ambre_toile::logic::convergence_tests::reorder_to_back_moves_node` reads the whole order back after sending the top node of two to the back | verified |
| `TOILE-125` | `POST` | The four other targets MUST resolve against the list: front is the last place, forward and backward one step from where the node sits, and an index past the end clamps to the last place. | this spec section 3 | none. Only the back target is driven, and no surface sends an index at all (section 4), so the clamp is reachable from the command alone | unverified |
| `TOILE-126` | `FRAME` | A reorder MUST NOT change the relative order of the nodes it does not name. | this spec section 3 | none. The handler moves one entry and the neighbours close behind it, and the one test that drives a reorder holds two nodes, where a move and a swap give the same list | unverified |
| `TOILE-127` | `FRAME` | A reorder whose target is the place the node already holds MUST write nothing and broadcast nothing. | this spec section 3 | none. The handler returns before the commit when the resolved index is the current one, and no test sends a reorder with no effect | unverified |
| `TOILE-128` | `FAIL` | A reorder naming a node the order does not hold MUST do nothing, and MUST NOT report an error. | this spec section 3 | none. The handler returns with no event at all, which is what it returns for the no-op above, so a surface cannot tell the refusal from the change that was already made -- section 6 | unverified |

### 1.3 Remove a node

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|---|
| `TOILE-061` | `FRAME` | Removing a node MUST NOT rewrite the rest of the order. | `ambre-spec-budgets.md section 1`; this spec section 6 | none, and **the rule fails today**: the removal deletes the whole list and re-pushes the survivors, so deleting one element sends an operation per surviving node to every peer -- section 6 | unverified |
| `TOILE-129` | `FRAME` | Removing a node MUST NOT drop an entry it was not asked to remove. | this spec section 3 | none, and **the rule fails today**: the rewrite keeps the entries that decode, so an entry whose tag this version cannot read is destroyed by the next unrelated delete. That is the repair section 5 rejects, run on a write rather than on a read -- section 6 | unverified |

* **Budget.** a remote change under 2 ms, a row of `ambre-spec-budgets.md section 1`.
  `TOILE-061` is what breaks it: a delete on a large board produces a change proportional to the
  board rather than to the deletion, and no bench measures it.
* **Boundary.** none of its own: the order is a list in the Logic thread's document and leaves as
  bytes the Sync worker seals (`ZK-001`).
* **Verification.** 4 `verified`, 12 `unverified`. Five rows become sixteen for two mechanisms more
  than the document cited before, and three of the four verdicts changed hands rather than being
  earned. `TOILE-057` was held by a test of two elements and no connector; the order carrying both
  kinds is read back by the deletion test instead. `TOILE-059` stacked three tests, and the third
  compares two walks of one document, which is convergence (`TOILE-123`) and not the skipping of a
  dead entry. What it skipped is two rules, and only the connector half is driven (`TOILE-120`).
  `TOILE-058` promised a paint and an agreed order, and both fail (`TOILE-121`). `TOILE-060` was what
  the document may hold and what the screen does with a duplicate (`TOILE-122`). The reorder had no
  rows at all: four targets nobody drives (`TOILE-125`), the neighbours it may not disturb
  (`TOILE-126`), the write it owes nobody (`TOILE-127`) and the refusal that reads as a no-op
  (`TOILE-128`). The removal owed one more than its cost (`TOILE-129`), and the sense of the order
  itself was never written (`TOILE-119`).

---

## 2. WHAT EACH USER SEES

| Alice does | Bob does | The document holds | Each user sees | Test |
|---|---|---|---|---|
| brings a shape to the front | brings another shape to the front | both moves, in the order they converged | the same stacking on both screens (`TOILE-123`) | `logic::convergence_tests::two_peers_converge` |
| sends a shape to the back | deletes that shape | the deletion | the shape gone, and no gap in the order (`TOILE-124`, `TOILE-057`) | `logic::convergence_tests::reorder_to_back_moves_node`, `::delete_drops_owned_connectors` |
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

1. find the node's current place, and stop when it has none, silently (`TOILE-128`);
2. resolve the target to an index, clamped to the list (`TOILE-124`, `TOILE-125`);
3. stop when the target is where the node already is, before any write (`TOILE-127`);
4. move the entry, which leaves the other nodes in their order (`TOILE-126`), commit, and
   re-materialize.

**Steps, when the order is re-materialized:** read the list, drop an entry whose tag does not decode,
and keep the first occurrence of a repeated node (`TOILE-060`, `TOILE-122`). The result is the
sequence the paint walk and the pick walk both use, front being the end (`TOILE-119`). The walk then
drops the entries it cannot paint: a connector it cannot route (`TOILE-059`) and an element the
document no longer holds (`TOILE-120`).

**How a node is removed.** The delete handler rewrites the list: it collects the survivors, deletes
every entry, and pushes the survivors back. That is where `TOILE-061` fails, and it is the reason a
delete is the most expensive change on the board. The survivors are the entries that decode, so the
rewrite also drops the ones it cannot read, which is where `TOILE-129` fails. What a removal owes the
order itself is `TOILE-034` (`features/ambre-spec-toile-elements.md`).

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

* **An unlisted element is invisible** (`TOILE-058`). The design says such an element paints last,
  ordered by id, and the visible-set walk iterates the materialized order only, so an element whose
  entry a concurrent change dropped disappears from every screen while staying in the document. The
  search carries exactly the fallback the paint walk lacks, which is where the shape of the fix is
  already written.
* **A delete rewrites the whole order** (`TOILE-061`). The removal deletes every entry and re-pushes
  the survivors, so deleting one element on a board of fifty thousand nodes produces fifty thousand
  list operations, encrypted and broadcast to every peer, and does so again for the next delete
  (`ambre-roadmap.md`, "Engine and render optimization").
* **Nothing stops a duplicate reaching the document** (`TOILE-060`, `TOILE-122`). The materialization
  hides one, so a duplicate is invisible until something counts the list, and the paint walk would
  otherwise paint the node twice.
* **An unreadable entry is destroyed by the next delete** (`TOILE-129`). The removal rewrite keeps
  the entries that decode and pushes those back, so an entry a future version wrote and this one
  cannot read survives every read and dies on the first unrelated deletion. Section 5 rejects
  repairing the order on a read for exactly that reason, and the write does it anyway, to entries no
  gesture named.
* **A refusal and a no-op are the same event** (`TOILE-128`, `TOILE-127`). A reorder of a node the
  order does not hold and a reorder that changes nothing both return with no event, so a surface has
  nothing to tell one from the other, and a menu entry that silently did nothing looks like one that
  worked.
* **Nothing reads the paint order back** (`TOILE-119`). Every test of the order reads the
  materialized list, and no test reads a derived viewport to check that the last entry paints over
  the first. Four menu entries are named for a sense the contract now states and nothing verifies.
* **There is no layer panel.** A user reorders by four menu entries and cannot see the order, so a
  stack of five overlapping shapes is arranged by trial.
* **A multi-selection reorder is one command per element.** The four entries fan out over the
  selection, so bringing five shapes to the front takes five undos and interleaves them with whatever
  else sat between them.
