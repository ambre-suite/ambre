# AMBRE -- FEATURE: CONNECTORS

**Type:** Vertical (feature)
**Implementation:** partial -- a straight arrow is drawn, routed and captioned; it cannot be
selected or styled, and two of the three path kinds have no sender.

> A user drags from one shape to another and gets an arrow. Moving either shape keeps the arrow
> attached: it is drawn between the two, not placed on the board.

An arrow stores what it joins, never where it runs. That one choice is why a drag costs one write
instead of one per arrow, and why two users can never make an arrow contradict itself.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|---|
| `TOILE-006` | INV | A connector MUST store what it joins and never where it runs: its geometry is recomputed on read. | this spec section 3 | `ambre_toile::logic::connector::tests::straight_endpoints_are_the_anchors`, `::every_path_type_routes`; the format stores endpoints and a path kind, no geometry (`ambre-spec-toile-schema.md section 3`) | verified |
| `TOILE-164` | INV | Moving an element MUST NOT write a change to the connectors that touch it: a drag costs one register write, whatever hangs off it. | this spec section 3 | structural, and the absence is the proof: no path from a move reaches the connector map, and the routing runs inside the read-only derivation | verified |
| `TOILE-166` | OBS | A connector whose endpoint element is not held MUST NOT be painted. | this spec section 2 | `ambre_toile::logic::connector::tests::missing_endpoint_yields_none`; the derivation drops the connector on the same `None` | verified |
| `TOILE-007` | CONC | A connector whose endpoint element is deleted by another peer MUST keep its own data: dangling is not destruction. | this spec section 2 | none. The one test named for this deletes an endpoint locally, which garbage-collects the connector instead of dangling it (`TOILE-042`), and asserts only that nothing is painted. No test holds two peers, and none reads the document afterwards | unverified |
| `TOILE-044` | OBS | A dangling connector MUST be announced, so a surface can offer a repair. | this spec section 3 | `ambre_toile::logic::convergence_tests::retarget_to_missing_element_reports_dangling` holds the one path that announces. Nothing consumes the announcement: the view drops the event | unverified |
| `TOILE-165` | OBS | A connector that goes dangling because its endpoint left MUST be announced too, not only one re-aimed at a missing element. | this spec section 3 | none, and the rule fails today -- section 6 | unverified |
| `TOILE-043` | INV | A connector MUST sit in the one paint order with the elements, so it can be placed behind one. | this spec section 3; `features/ambre-spec-toile-z-order.md` | `ambre_toile::logic::convergence_tests::reorder_to_back_moves_node` covers the reorder over the shared order. Nothing drives a connector through it, and no surface can select one to try (`TOILE-UI-035`) | unverified |
| `TOILE-045` | INV | A connector MUST be culled with the viewport, like an element. | this spec section 3 | none, and **the rule fails today**: the visible-set walk tests the culled set for an element and emits every connector on the board unconditionally, because no connector is in the spatial index -- section 6 | unverified |
| `TOILE-046` | INV | A connector MUST carry its own style, so an arrow can be coloured and thickened. | this spec section 3 | none, and **the rule fails today**: the field is `design` in the format, no command writes it, and the deriver hands out a default style for every connector -- section 6 | unverified |
| `TOILE-167` | INV | Every path kind and every anchor the format stores MUST be reachable: a shape the document can hold is one a user can make. | this spec section 3 | none, and the rule fails today -- section 6 | unverified |
| `TOILE-047` | CONC | A caption on a connector MUST converge character by character, like a note body. | this spec section 3 | `ambre_toile::logic::convergence_tests::concurrent_captions_converge` holds two peers editing opposite ends of one caption and asserts both survive the merge | verified |
| `TOILE-UI-346` | OBS | A caption MUST be painted on the arrow it belongs to, over a plate, so it stays readable where it crosses the line. | this spec section 4 | none. The paint pass runs after the element bodies and fills a plate sized to the laid-out text, and no test reads a painted frame | unverified |
| `TOILE-232` | INV | A caption MUST store what it says and never where it sits: its place on the arrow is recomputed on read, like the arrow's own geometry (`TOILE-006`). | this spec section 3 | `ambre_toile::ui::label::tests::a_straight_arrow_anchors_at_its_midpoint`, `::an_elbow_anchors_half_way_along_the_run`; the format holds one text and no position (`ambre-spec-toile-schema.md section 3`) | verified |

### 1.2 Drag an arrow from one shape to another

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|---|
| `TOILE-UI-211` | POST | A drag from one element released over another MUST send one connector between the two. | this spec section 4 | `ambre_toile::ui::tool::tests::connector_anchor_to_anchor_emits_create_connector` asserts both endpoints and the caps of the emitted command | verified |
| `TOILE-UI-212` | POST | The anchor at each end MUST be the side of the element nearest the pointer, so an arrow leaves where the user aimed it. | this spec section 4 | `ambre_toile::ui::tool::tests::nearest_anchor_picks_closest_edge` asserts all four sides | verified |
| `TOILE-UI-213` | FAIL | Given a release over empty board, nothing MUST be sent: an arrow needs two elements. | this spec section 4 | `ambre_toile::ui::tool::tests::connector_release_on_empty_emits_nothing` | verified |
| `TOILE-UI-214` | FAIL | Given a release over the element the drag started on, nothing MUST be sent: an arrow to itself is not a diagram. | this spec section 4 | none. The release compares the two ids and drops the draft, and no test releases over the source | unverified |
| `TOILE-UI-215` | FRAME | A connector drag MUST NOT move, resize or select the element it started on: the preview is local and the release is the only write. | this spec section 4 | none. The tool holds a connector draft that emits one command, and no test asserts what the drag leaves alone | unverified |
| `TOILE-168` | PRE | The caller owes two element ids the board holds: a create checks neither. | this spec section 3 | none. The handler mints an id and writes both endpoints as given, so a create against a missing element is accepted and dangles at the next read | unverified |
| `TOILE-169` | POST | A create MUST put the connector in the shared paint order, above what is already there. | this spec section 3 | `ambre_toile::logic::convergence_tests::delete_drops_owned_connectors` asserts the order carries the connector node after a create. Nothing asserts where in it | unverified |
| `TOILE-170` | FRAME | A create MUST NOT touch the two elements it joins: an arrow is drawn between them, not attached to them. | this spec section 3 | structural, and the construction is the proof: the handler writes one connector map and one paint-order entry, and reads the elements only through the bounds cache | verified |

### 1.3 Re-aim an arrow at another shape

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|---|
| `TOILE-171` | POST | Re-aiming an arrow MUST move the endpoint named and MUST leave the other where it was. | this spec section 3 | `ambre_toile::logic::convergence_tests::retarget_rebinds_endpoint_without_dangling` holds that the arrow still routes afterwards, which needs both ends. Nothing reads the untouched end | unverified |
| `TOILE-172` | POST | Re-aiming an arrow at an element nobody holds MUST announce it as dangling. | this spec section 3 | `ambre_toile::logic::convergence_tests::retarget_to_missing_element_reports_dangling` asserts the event and that nothing is painted | verified |
| `TOILE-173` | FRAME | Re-aiming MUST NOT touch the path kind, the end caps, or the connector's place in the paint order. | this spec section 3 | none. The handler writes the two fields of the one endpoint, and no test reads the rest of the map afterwards | unverified |
| `TOILE-174` | FAIL | Given an arrow the board does not hold, re-aiming MUST answer an error and MUST leave the document unchanged (`TOILE-036`). | this spec section 3 | none. The handler returns the error before any write, and no test names a connector nobody minted | unverified |

### 1.4 Caption an arrow

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|---|
| `TOILE-UI-344` | POST | A double-click on an arrow MUST open an editor on its caption, in place on the arrow. | this spec section 4 | none for the gesture: the view is not host-constructible. Its geometry half is held -- `ambre_toile::ui::label::tests::a_click_on_the_line_picks_the_arrow` and `::the_top_most_arrow_wins` hold which arrow a point lands on | unverified |
| `TOILE-UI-345` | POST | Leaving the editor MUST send the caption, and MUST send nothing when it comes back as it opened. | this spec section 4 | `ambre_canvas::editor::tests::an_unchanged_buffer_commits_nothing` holds the half that decides: a buffer equal to its seed answers `None` and the close sends nothing. Nothing drives the close path itself | unverified |
| `TOILE-UI-347` | POST | A click on a caption MUST aim at the arrow that carries it, and an arrow with no caption MUST NOT claim the space one would occupy. | this spec section 4 | `ambre_toile::ui::label::tests::a_click_on_the_caption_picks_the_arrow` asserts both halves on the same point | verified |
| `TOILE-UI-348` | FRAME | Captioning MUST NOT touch what the arrow joins, how it runs, its caps, or its place in the paint order: a caption is a word on an arrow, not a change to it. | this spec section 3 | none. The handler resolves one connector map and writes one text inside it, and no test reads the rest of the map afterwards | unverified |
| `TOILE-233` | FAIL | Given an arrow the board does not hold, captioning MUST answer an error and MUST leave the document unchanged (`TOILE-036`). | this spec section 3 | `ambre_toile::logic::convergence_tests::captioning_a_missing_arrow_is_refused` asserts the error and that no delta is broadcast | verified |

### 1.5 Delete a shape an arrow touches

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|---|
| `TOILE-042` | POST | A local delete of an element MUST take the connectors it owns with it, in the same commit. | this spec section 3 | `ambre_toile::logic::convergence_tests::delete_drops_owned_connectors` asserts both the element count and the paint order after the delete | verified |
| `TOILE-175` | FRAME | A delete MUST NOT touch a connector neither of whose ends it removes. | this spec section 3 | `ambre_toile::logic::convergence_tests::deleting_an_endpoint_makes_the_connector_dangle` deletes an unrelated element first and asserts the connector survives it | verified |
| `TOILE-176` | POST | A delete MUST take the connector out of the paint order as well as out of the map: no entry may name a connector that is gone. | this spec section 3 | `ambre_toile::logic::convergence_tests::delete_drops_owned_connectors` asserts the order holds one element node and nothing else | verified |

* **Budget.** the routing pass is inside the visible-set derivation, budgeted by the visible-set row
  of `ambre-spec-budgets.md section 1`. Because `TOILE-045` fails, the pass runs once per
  connector on the board rather than once per visible connector, and no bench measures either.
* **Boundary.** none of its own: a connector lives in the Logic thread's document and leaves as bytes
  the Sync worker seals (`ZK-001`).
* **Verification.** 15 `verified`, 18 `unverified`, 0 `unimplemented`. The caption added seven rows and
  four held ones, on five new tests: two peers typing on one caption, a caption refused on an arrow
  nobody holds, the anchor on a line and on a bend, and the click that reaches an arrow through its
  words. The one `unimplemented` row is gone -- the caption is built, and the rule that says it
  converges is held by a two-peer run rather than by the note that it will be. What the caption
  could not hold is the gesture: the view is not host-constructible, so the double-click that opens
  the editor and the close that sends the caption rest on their pure halves -- which arrow a point
  lands on, and whether an unchanged buffer commits -- and not on the path between them. What stays
  loose is otherwise unchanged: no arrow can be selected, so re-aiming and deleting one are unheld
  by construction, and four rows describe a shape the format stores and no gesture produces.

---

## 2. WHAT EACH USER SEES

| Alice does | Bob does | The document holds | Each user sees | Test |
|---|---|---|---|---|
| moves a shape an arrow starts from | watches | the shape's new position, and nothing about the arrow | the arrow following the shape, re-routed on both screens | `logic::connector::tests::straight_endpoints_are_the_anchors` |
| deletes the shape an arrow ends at | re-aims that arrow at a third shape | either the deletion with the arrow gone, or the arrow bound to the third shape | one consistent board: no arrow pointing at nothing | none: no test holds two peers over a connector |
| deletes the shape | keeps the arrow, offline | the arrow, referencing an element nobody holds | nothing painted where the arrow was, and the arrow still in the document | `logic::connector::tests::missing_endpoint_yields_none` holds the half that is painted; nothing reads the document afterwards (`TOILE-007`) |
| re-aims an arrow at a shape she just deleted | watches | the arrow bound to a missing element | nothing painted, and an announcement no surface reads (`TOILE-044`) | `logic::convergence_tests::retarget_to_missing_element_reports_dangling` |
| drags a shape an arrow ends at, for a second | watches | the shape's new position, and no connector write at all | the arrow re-routed on every frame (`TOILE-164`) | none |
| types `unit cost` on an arrow's caption | types `cost in EUR` on the same caption, offline | one caption holding both edits, merged character by character | the same caption on both screens, neither user's typing dropped (`TOILE-047`) | `logic::convergence_tests::concurrent_captions_converge` |

---

## 3. MODEL

**Containers.** The `connectors` map, one map per minted connector id, and one entry in the shared
paint order. Both are declared in `ambre-spec-toile-schema.md section 3`, with the field layout.

**The routing shape** (`ambre-bus`). An anchor is a named side, the centre, or a free point on the
element's box; a path kind decides how the two anchors are joined:

```rust
#[derive(Clone, Copy, serde::Serialize, serde::Deserialize)]
pub enum Anchor { Top, Bottom, Left, Right, Center, Free { u: f32, v: f32 } }

#[derive(Clone, Copy, serde::Serialize, serde::Deserialize)]
pub enum PathType { Straight, Elbow, Curved }

/// Directional decoration, persisted as two flags on the connector.
#[derive(Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct EndCaps { pub start: bool, pub end: bool }
```

**Commands** (`ambre-bus`), one undo item each: `CreateConnector` with both endpoints, both anchors,
the path kind and the end caps; `RetargetConnector` moving one endpoint; `DeleteConnector`.

**Steps, when a connector is painted:**

1. read the two endpoint ids from the connector map;
2. resolve each anchor against the endpoint element's current box, taken from the bounds cache;
3. stop and paint nothing when either endpoint is absent (`TOILE-166`);
4. dispatch on the path kind -- a line, an orthogonal path that leaves along the source anchor's
   side and ignores the target's, or a cubic whose control points leave along both sides;
5. hand the outline out with the two end caps.

Nothing in that sequence writes, which is the whole point: an element drag re-routes every connector
that touches it and produces no operation of its own (`TOILE-006`).

**Steps, when an arrow is created:** the handler mints an id, writes both endpoints, both anchors,
the path kind and the caps into one connector map, and appends one entry to the paint order. It
resolves neither endpoint against the board and reads no lock, so a create naming an element nobody
holds is accepted and dangles at the next read (`TOILE-168`).

**Steps, when an endpoint element is deleted locally:** the delete handler collects every connector
referencing a removed id at either end, removes those connector maps, and strips them from the paint
order inside the delete's own commit (`TOILE-042`). Under concurrency the cleanup can lose the race,
which is exactly the dangling case above.

**Steps, when an arrow is captioned:** the handler resolves the connector map, takes its `label`
text and **diffs** the whole caption into it rather than replacing it, so two peers typing on one
arrow merge inside one text instead of one body winning (`TOILE-047`). The caption travels whole on
the wire because the surface holds a plain buffer; the char-CRDT is what the document stores.

The `label` container is seeded **at the connector's birth**, not on the first caption. Seeding at
creation puts the same container on every peer, inside the creation delta, before anyone types; a
lazy seed on each peer would mint rival containers under one key and LWW would drop a caption whole.
A connector minted before the field existed is the one lazy seed left, and the one window where that
can happen.

**Steps, when a caption is placed:** the routed path is measured, and the caption's box is centred on
the point half-way along it by arc length. Nothing about that is stored (`TOILE-232`): moving an
endpoint re-routes the arrow and carries the caption with it, at no write.

**Code:**

* `crates/ambre-toile/src/logic/connector.rs` -- the anchor resolution and the three path kinds;
* `ui/label.rs` -- the caption's anchor, its box, and which arrow a click lands on;
* `logic/state/connectors.rs` -- create, retarget, delete, and the "which connectors touch these
  elements" query the delete uses;
* `logic/state/derive.rs` -- the connector arm of the visible-set walk;
* `logic/state/codec.rs` -- the field encoding of an anchor and a path kind.

---

## 4. SURFACE

| Trigger | Sends |
|---|---|
| the connector tool, dragged from one element to another | `CreateConnector` between the two nearest anchors, always straight (`TOILE-UI-211`, `TOILE-UI-212`) |
| the connector tool, released over empty board | nothing: an arrow needs two elements (`TOILE-UI-213`) |
| the connector tool, released over the element the drag started on | nothing: an arrow to itself is not a diagram (`TOILE-UI-214`) |
| a double-click on an arrow's line or its caption, with no element under the pointer | nothing yet: the caption editor opens in place on the arrow (`TOILE-UI-344`) |
| leaving that editor -- `Escape`, or a click away | `SetConnectorLabel`, and nothing at all when the caption comes back as it opened (`TOILE-UI-345`) |

**What the drawer sees.** A preview line from the source anchor to the pointer while the drag is in
flight, local and uncommitted (`features/ambre-spec-toile-tools.md`). The anchor picked at each end
is the nearest side of the element under the pointer.

**What the captioner sees.** The words centred on the arrow, over a plate in the board colour so
they stay readable where they cross the line (`TOILE-UI-346`). An arrow with no caption paints
nothing there and claims no space: the click target is the line alone (`TOILE-UI-347`). While the
editor is open the selection is empty, so a `Delete` typed mid-caption deletes a character and not
the shapes that were selected before.

**What no surface offers.** Selecting an arrow, deleting one, re-aiming one, or colouring one. The
two commands that would serve the first three exist and nothing sends them, because the pick walk
drops every connector before testing geometry (`TOILE-UI-035`). The caption reaches its arrow
without that walk, through a pick of its own.

---

## 5. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| A connector stores what it joins, never where it runs | persisting the routed path, which the earlier design carried | it would have to be rewritten on every frame of every drag of either endpoint, turning one register write into one per attached arrow, and giving two peers a geometry to disagree about. Recomputing costs a resolution per visible arrow per frame and cannot be wrong |
| A deleted endpoint leaves the connector in place | clearing the reference | it turns one user's delete into another user's data loss. Data is never destroyed by a merge: the local delete cleans up as an intentional batch, a concurrent one cannot, so the connector survives referencing a missing id and is simply skipped -- the same stance a merged-away cell takes in `features/ambre-spec-trame-cell-merges.md` |
| Connectors live in the shared paint order | a connector layer painted after every element | an arrow behind a shape is an ordinary diagram, and a separate always-on-top pass makes it impossible |
| An anchor is a side, not a point | a stored offset from the element origin | a side survives a resize and a rotation of the element, and a stored point does not. The free anchor exists for the case a user places deliberately |
| End caps are two flags on the connector, not a style attribute | folding the caps into the style register | the direction of an arrow is what it means, so it belongs to the connector rather than to its style, and it survives a restyle |

---

## 6. NOT BUILT, AND NOT YET RULED

* **A connector is never culled** (`TOILE-045`). No connector is in the spatial index, and the
  visible-set walk gates the element arm on the culled set and the connector arm on nothing. A board
  of two thousand arrows therefore routes and ships two thousand outlines every frame, whatever the
  camera holds.
* **A connector has no style** (`TOILE-046`). The format declares the field as `design`, no
  command writes it, and the deriver hands the UI thread a default style for every connector, so
  every arrow on every board is the same colour and the same width.
* **A connector still cannot be selected** (`TOILE-UI-035`). The caption is reached by a pick of its
  own -- a double-click resolves the arrow under the pointer without going through the selection --
  so a captioned arrow is not a selected arrow. The retarget and delete commands therefore still
  have no sender, an arrow cannot be reordered by hand, and the style inspector and the floating
  toolbar address elements only. Making a connector a first-class selection means widening every
  container that holds an `ElementId`, which is its own Story, not a side effect of a caption.
* **The announcement goes nowhere** (`TOILE-044`). The dangling event reaches the view, which drops
  it, so the repair affordance the design names does not exist and a dangling arrow is
  indistinguishable from no arrow.
* **A deleted endpoint announces nothing** (`TOILE-165`). The dangling event is emitted on the
  re-aim path only. When the endpoint leaves instead -- deleted by another peer, or restored by an
  undo the local cleanup already ran under -- the arrow simply stops being painted, with no notice
  at all, so the one path that could offer a repair is the one a user never takes.
* **Two path kinds and two anchors have no sender** (`TOILE-167`). The tool writes
  `PathType::Straight` on every create and picks among the four sides, so the elbow, the curve, the
  centre anchor and the free anchor are shapes the format stores, the codec round-trips and the
  router handles that no gesture can produce. The unit test that exercises all three kinds asserts
  only that each returns an outline.
* **A create trusts its endpoints** (`TOILE-168`). The handler validates neither id, so a command
  from a peer running older code, or a replayed undo item, writes an arrow to an element that is not
  there and the board carries it silently.
* **A caption has no style and one size.** It is painted in the theme's text ink at a fixed world
  size in the default family, because a connector carries no style of its own (`TOILE-046`). Until
  that row holds, a caption cannot be coloured, resized or emphasised.
* **A caption's box is fixed, not fitted.** The wrap width and the click box are a constant, chosen
  before the font context exists (the editor opens outside the paint pass). A caption longer than
  the box wraps onto a second line rather than widening, and a very long one runs past its plate.
* **A connector between two elements inside a rotated frame is routed against boxes, not corners.**
  The anchor resolution reads the rotation-expanded box, so an arrow to a rotated shape lands on that
  box's side rather than on the shape's own edge.
