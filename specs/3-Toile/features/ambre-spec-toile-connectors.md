# AMBRE -- FEATURE: CONNECTORS

**Type:** Vertical (feature)
**Implementation:** partial -- the board draws, routes, captions, selects, re-aims, restyles,
reorders and deletes a straight arrow. It says when one goes dangling. Two path kinds and two
anchors have no sender.

> A user drags from one shape to another and gets an arrow. Moving either shape keeps the arrow
> attached: the board draws it between the two, rather than placing it on the board.

An arrow stores what it joins, never where it runs. That one choice is why a drag costs one write
instead of one per arrow. It is also why two users can never make an arrow contradict itself.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TOILE-006` | `INV` | A connector MUST store what it joins and never where it runs: its geometry is recomputed on read. | this spec section 3 | `ambre_toile::logic::connector::tests::straight_endpoints_are_the_anchors` -- the straight case; `ambre_toile::logic::connector::tests::every_path_type_routes` -- every path kind; structural -- the format stores endpoints and a path kind, no geometry (`ambre-spec-toile-schema.md section 3`) | none | verified |
| `TOILE-164` | `INV` | Moving an element MUST NOT write a change to the connectors that touch it: a drag costs one register write, whatever hangs off it. | this spec section 3 | structural -- no path from a move reaches the connector map, and the routing runs inside the read-only derivation | none | verified |
| `TOILE-166` | `OBS` | A connector whose endpoint element is not held MUST NOT be painted. | this spec section 2 | `ambre_toile::logic::connector::tests::missing_endpoint_yields_none` -- the routing answers nothing; structural -- the derivation drops the connector on that same `None` | none | verified |
| `TOILE-007` | `CONC` | A connector whose endpoint element is deleted by another peer MUST keep its own data: dangling is not destruction. | this spec section 2 | none | unheld -- the one test named for this deletes an endpoint locally, which garbage-collects the connector instead of dangling it (`TOILE-042`), and reads only that nothing is painted. No test holds two peers, and none reads the document afterwards; lands with the `The Toile belts` Story (`ambre-roadmap.md` E9) | untested |
| `TOILE-044` | `OBS` | A dangling connector MUST be announced, so a surface can offer a repair. | this spec section 3 | `ambre_toile::logic::convergence_tests::retarget_to_missing_element_reports_dangling` -- the re-aim path; `ambre_toile::logic::convergence_tests::an_arrow_stranded_by_a_peer_is_announced` -- the endpoint that left; qa-manual -- DOM glue held by using the application (`1-Foundations/ambre-spec-testing.md section 2`), the shell raising its notice off this event, reached by deleting a shape a collaborator is joining to another | none | qa-manual |
| `TOILE-165` | `OBS` | A connector that goes dangling because its endpoint left MUST be announced too, not only one re-aimed at a missing element. | this spec section 3 | `ambre_toile::logic::convergence_tests::an_arrow_stranded_by_a_peer_is_announced` -- a concurrent delete announcing the arrow on both peers, and the later delta that strands nothing announcing nothing | none | verified |
| `TOILE-043` | `INV` | A connector MUST sit in the one paint order with the elements, so it can be placed behind one. | this spec section 3; `features/ambre-spec-toile-z-order.md` | `ambre_toile::logic::convergence_tests::reorder_to_back_moves_node` -- the reorder over the shared order; `ambre_toile::ui::arrow::tests::an_arrow_answers_delete_and_the_z_order` -- the four entries that drive a connector through it | partial -- no test runs the two ends together; lands with the `The Toile belts` Story (`ambre-roadmap.md` E9) | partial |
| `TOILE-045` | `INV` | A connector MUST be culled with the viewport, like an element. | this spec section 3 | none | fails -- the visible-set walk tests the culled set for an element and emits every connector on the board unconditionally, because no connector is in the spatial index, section 6; lands with the `A board costs what it shows and what changed` Story (`ambre-roadmap.md` E7) | failing |
| `TOILE-046` | `INV` | A connector MUST carry its own style, so an arrow can be coloured and thickened. | this spec section 3 | `ambre_toile::logic::convergence_tests::an_arrow_carries_its_own_style` -- it restyles one arrow of two and reads the second keeping the default: the register is per connector, not shared | none | verified |
| `TOILE-167` | `INV` | Every path kind and every anchor the format stores MUST be reachable: a shape the document can hold is one a user can make. | this spec section 3 | not built | unbuilt -- lands with the `Every connector shape a user can make` Story (`ambre-roadmap.md` E6). Section 6 lists the shapes no gesture makes | unimplemented |
| `TOILE-047` | `CONC` | A caption on a connector MUST converge character by character, like a note body. | this spec section 3 | `ambre_toile::logic::convergence_tests::concurrent_captions_converge` -- it holds two peers editing opposite ends of one caption and reads both surviving the merge | none | verified |
| `TOILE-263` | `CONC` | Leaving a caption editor MUST keep a peer's edit to the same caption that arrived while it was open. | this spec section 3; `TOILE-047` | `ambre_toile::logic::convergence_tests::an_open_caption_editor_keeps_a_peers_edit` -- a peer's edit lands between the open and the close, and both writers' words survive on both replicas; structural -- the close sends the caption the editor opened on as the base (`ambre_toile::ui::edit::InlineEditor::opened_text`) | none | verified |
| `TOILE-UI-346` | `OBS` | A caption MUST be painted on the arrow it belongs to, over a plate, so it stays readable where it crosses the line. | this spec section 4 | none | unheld -- the paint pass runs after the element bodies and fills a plate sized to the laid-out text, and no test reads a painted frame; lands with the `The Toile belts` Story (`ambre-roadmap.md` E9) | untested |
| `TOILE-UI-351` | `OBS` | A selected arrow MUST be shown by tracing the arrow itself and putting one handle on each end, rather than by a selection box: a rectangle around a diagonal says nothing about which end a drag would move. | this spec section 4 | `ambre_toile::ui::render::tests::a_selected_arrow_gets_two_handles_and_no_box` -- the two handles and the absent box; `ambre_toile::ui::render::tests::a_vanished_arrow_paints_nothing` -- the stale case | none | verified |
| `TOILE-232` | `INV` | A caption MUST store what it says and never where it sits: its place on the arrow is recomputed on read, like the arrow's own geometry (`TOILE-006`). | this spec section 3 | `ambre_toile::ui::label::tests::a_straight_arrow_anchors_at_its_midpoint` -- the straight case; `ambre_toile::ui::label::tests::an_elbow_anchors_half_way_along_the_run` -- the elbow; structural -- the format holds one text and no position (`ambre-spec-toile-schema.md section 3`) | none | verified |

### 1.2 Drag an arrow from one shape to another

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TOILE-UI-211` | `POST` | A drag from one element released over another MUST send one connector between the two. | this spec section 4 | `ambre_toile::ui::tool::tests::connector_anchor_to_anchor_emits_create_connector` -- it reads both endpoints and the caps of the emitted command | none | verified |
| `TOILE-UI-212` | `POST` | The anchor at each end MUST be the side of the element nearest the pointer, so an arrow leaves where the user aimed it. | this spec section 4 | `ambre_toile::ui::tool::tests::nearest_anchor_picks_closest_edge` -- it reads all four sides | none | verified |
| `TOILE-UI-213` | `FAIL` | Given a release over empty board, nothing MUST be sent: an arrow needs two elements. | this spec section 4 | `ambre_toile::ui::tool::tests::connector_release_on_empty_emits_nothing` | none | verified |
| `TOILE-UI-214` | `FAIL` | Given a release over the element the drag started on, nothing MUST be sent: an arrow to itself is not a diagram. | this spec section 4 | none | unheld -- the release compares the two ids and drops the draft, and no test releases over the source; lands with the `The Toile belts` Story (`ambre-roadmap.md` E9) | untested |
| `TOILE-UI-215` | `FRAME` | A connector drag MUST NOT move, resize or select the element it started on: the preview is local and the release is the only write. | this spec section 4 | none | unheld -- the tool holds a connector draft that emits one command, and no test reads what the drag leaves alone; lands with the `The Toile belts` Story (`ambre-roadmap.md` E9) | untested |
| `TOILE-168` | `PRE` | The caller MUST hand over two element ids the board holds: a create checks neither. | this spec section 3 | none | unheld -- the handler mints an id and writes both endpoints as given, so a create against a missing element is accepted and dangles at the next read; lands with the `The Toile belts` Story (`ambre-roadmap.md` E9) | untested |
| `TOILE-169` | `POST` | A create MUST put the connector in the shared paint order, above what is already there. | this spec section 3 | `ambre_toile::logic::convergence_tests::delete_drops_owned_connectors` -- it reads the order carrying the connector node after a create | partial -- nothing reads where in the order it landed; lands with the `The Toile belts` Story (`ambre-roadmap.md` E9) | partial |
| `TOILE-170` | `FRAME` | A create MUST NOT touch the two elements it joins: an arrow is drawn between them, not attached to them. | this spec section 3 | structural -- the handler writes one connector map and one paint-order entry, and reads the elements only through the bounds cache | none | verified |

### 1.3 Re-aim an arrow at another shape

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TOILE-171` | `POST` | Re-aiming an arrow MUST move the endpoint named. | this spec section 3 | `ambre_toile::logic::convergence_tests::retarget_rebinds_endpoint_without_dangling` -- it holds that the arrow still routes afterwards | partial -- nothing reads the new binding back; lands with the `The Toile belts` Story (`ambre-roadmap.md` E9) | partial |
| `TOILE-238` | `POST` | Re-aiming an arrow MUST leave the endpoint it does not name where it was. | this spec section 3 | none | unheld -- nothing reads the untouched end back; lands with the `The Toile belts` Story (`ambre-roadmap.md` E9) | untested |
| `TOILE-172` | `POST` | Re-aiming an arrow at an element nobody holds MUST announce it as dangling. | this spec section 3 | `ambre_toile::logic::convergence_tests::retarget_to_missing_element_reports_dangling` -- it reads the event and the empty paint | none | verified |
| `TOILE-173` | `FRAME` | Re-aiming MUST NOT touch the path kind, the end caps, or the connector's place in the paint order. | this spec section 3 | none | unheld -- the handler writes the two fields of the one endpoint, and no test reads the rest of the map afterwards; lands with the `The Toile belts` Story (`ambre-roadmap.md` E9) | untested |
| `TOILE-174` | `FAIL` | Given an arrow the board does not hold, re-aiming MUST answer an error and MUST leave the document unchanged (`TOILE-036`). | this spec section 3 | none | unheld -- the handler returns the error before any write, and no test names a connector nobody minted; lands with the `The Toile belts` Story (`ambre-roadmap.md` E9) | untested |

### 1.4 Caption an arrow

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TOILE-UI-344` | `POST` | A double-click on an arrow MUST open an editor on its caption, in place on the arrow. | this spec section 4 | `ambre_toile::ui::label::tests::a_click_on_the_line_picks_the_arrow` -- which arrow a point lands on; `ambre_toile::ui::label::tests::the_top_most_arrow_wins` -- which one wins where two answer | partial -- the geometry half only: nothing drives the gesture, because the view is not host-constructible; lands with the `The Toile belts` Story (`ambre-roadmap.md` E9) | partial |
| `TOILE-UI-345` | `POST` | Leaving the editor MUST send the caption. | this spec section 4 | none | unheld -- nothing drives the close path itself; lands with the `The Toile belts` Story (`ambre-roadmap.md` E9) | untested |
| `TOILE-UI-363` | `POST` | Leaving the editor MUST send nothing when the caption comes back as it opened. | this spec section 4 | `ambre_canvas::editor::tests::an_unchanged_buffer_commits_nothing` -- a buffer equal to its seed answers `None` | partial -- nothing drives the close path itself; lands with the `The Toile belts` Story (`ambre-roadmap.md` E9) | partial |
| `TOILE-UI-347` | `POST` | A click on a caption MUST aim at the arrow that carries it. | this spec section 4 | `ambre_toile::ui::label::tests::a_click_on_the_caption_picks_the_arrow` -- the arrow the point resolves to | none | verified |
| `TOILE-UI-371` | `POST` | An arrow with no caption MUST NOT claim the space one would occupy. | this spec section 4 | `ambre_toile::ui::label::tests::a_click_on_the_caption_picks_the_arrow` -- the same point over an uncaptioned arrow, which answers none | none | verified |
| `TOILE-UI-348` | `FRAME` | Captioning MUST NOT touch what the arrow joins, how it runs, its caps, or its place in the paint order: a caption is a word on an arrow, not a change to it. | this spec section 3 | none | unheld -- the handler resolves one connector map and writes one text inside it, and no test reads the rest of the map afterwards; lands with the `The Toile belts` Story (`ambre-roadmap.md` E9) | untested |
| `TOILE-233` | `FAIL` | Given an arrow the board does not hold, captioning MUST answer an error and MUST leave the document unchanged (`TOILE-036`). | this spec section 3 | `ambre_toile::logic::convergence_tests::captioning_a_missing_arrow_is_refused` -- it reads the error and the absent delta | none | verified |

### 1.5 Select an arrow

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TOILE-UI-350` | `POST` | A click on an arrow with nothing else under the pointer MUST select it. | this spec section 4 | `ambre_toile::ui::label::tests::a_click_on_the_line_picks_the_arrow` -- which arrow a point lands on; `ambre_toile::ui::label::tests::the_top_most_arrow_wins` -- which one wins where two answer | partial -- the geometry half only: nothing drives the gesture, because the view is not host-constructible; lands with the `The Toile belts` Story (`ambre-roadmap.md` E9) | partial |
| `TOILE-UI-359` | `POST` | A click that lands on an element MUST NOT take the arrow under it: what the user sees on top is what the click takes. | this spec section 4 | none | unheld -- the hit test runs the elements before the arrows, and no test puts an arrow under an element and reads back which one the click took; lands with the `The Toile belts` Story (`ambre-roadmap.md` E9) | untested |
| `TOILE-UI-352` | `POST` | Dragging an endpoint handle onto a shape MUST re-aim that end at it, on the side the pointer aimed at. | this spec section 4 | `ambre_toile::ui::arrow::tests::a_press_grabs_the_nearer_end` -- the end the press takes; `ambre_toile::ui::arrow::tests::a_release_over_a_shape_re_aims_that_end` -- the re-aim | partial -- the two run the pure halves, and nothing drives the drag itself; lands with the `The Toile belts` Story (`ambre-roadmap.md` E9) | partial |
| `TOILE-UI-372` | `POST` | Releasing an endpoint handle over empty board MUST leave the arrow as it was. | this spec section 4 | `ambre_toile::ui::arrow::tests::a_release_over_empty_board_re_aims_nothing` -- the release over nothing | partial -- the pure half only, and nothing drives the drag itself; lands with the `The Toile belts` Story (`ambre-roadmap.md` E9) | partial |
| `TOILE-UI-353` | `POST` | With an arrow selected, delete MUST delete the arrow. | this spec section 4 | `ambre_toile::ui::arrow::tests::an_arrow_answers_delete_and_the_z_order` -- the delete entry the arrow answers | none | verified |
| `TOILE-UI-373` | `POST` | With an arrow selected, each of the four z-order entries MUST reorder it. | this spec section 4 | `ambre_toile::ui::arrow::tests::an_arrow_answers_delete_and_the_z_order` -- the four entries it reads back | none | verified |
| `TOILE-UI-374` | `FRAME` | Every board entry other than delete and the four z-order entries MUST do nothing to a selected arrow. | this spec section 4 | `ambre_toile::ui::arrow::tests::an_arrow_answers_delete_and_the_z_order` -- the silence on grouping and copying | none | verified |
| `TOILE-UI-354` | `FRAME` | Selecting an arrow MUST empty the element selection and MUST NOT put the arrow in one: an arrow has no box, so it never joins a marquee, a group, a transform or the clipboard. | this spec section 4 | none | unheld -- the two are separate fields written to exclude one another, and no test drives a selection through both; lands with the `The Toile belts` Story (`ambre-roadmap.md` E9) | untested |

### 1.6 Delete a shape an arrow touches

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TOILE-042` | `POST` | A local delete of an element MUST take the connectors it owns with it, in the same commit. | this spec section 3 | `ambre_toile::logic::convergence_tests::delete_drops_owned_connectors` -- it reads both the element count and the paint order after the delete | none | verified |
| `TOILE-175` | `FRAME` | A delete MUST NOT touch a connector neither of whose ends it removes. | this spec section 3 | `ambre_toile::logic::convergence_tests::deleting_an_endpoint_makes_the_connector_dangle` -- it deletes an unrelated element first and reads the connector surviving it | none | verified |
| `TOILE-176` | `POST` | A delete MUST take the connector out of the paint order as well as out of the map: an entry MUST NOT name a connector that is gone. | this spec section 3 | `ambre_toile::logic::convergence_tests::delete_drops_owned_connectors` -- it reads an order holding one element node and nothing else | none | verified |

* **Budget.** the routing pass is inside the visible-set derivation, budgeted by the visible-set row
  of `ambre-spec-budgets.md section 1`. Because `TOILE-045` fails, the pass runs once per
  connector on the board rather than once per visible connector, and no bench measures either.
* **Boundary.** none of its own: a connector lives in the Logic thread's document and leaves as bytes
  the Sync worker seals (`ZK-001`).
* **Verification.** 23 `verified`, 8 `partial`, 1 `failing`, 12 `untested`, 1 `qa-manual`, 1
  `unimplemented`. What holds is the pure geometry and what the surface paints back: which arrow a
  point lands on, and which end a press grabs. Held too are both ways an arrow goes dangling. What
  is loose is the paths between them, because the view is not host-constructible. Loose too are a
  create that trusts its endpoints, and four rows describing a shape no gesture produces.

---

## 2. WHAT EACH USER SEES

| Alice does | Bob does | The document holds | Each user sees | Test |
|---|---|---|---|---|
| moves a shape an arrow starts from | watches | the shape's new position, and nothing about the arrow | the arrow following the shape, re-routed on both screens | `logic::connector::tests::straight_endpoints_are_the_anchors` |
| deletes the shape an arrow ends at | re-aims that arrow at a third shape | either the deletion with the arrow gone, or the arrow bound to the third shape | one consistent board: no arrow pointing at nothing | none: no test holds two peers over a connector |
| deletes the shape | keeps the arrow, offline | the arrow, referencing an element nobody holds | nothing painted where the arrow was, and the arrow still in the document | `logic::connector::tests::missing_endpoint_yields_none` holds the half that is painted; nothing reads the document afterwards (`TOILE-007`) |
| re-aims an arrow at a shape she just deleted | watches | the arrow bound to a missing element | nothing painted, and a notice naming the loss (`TOILE-044`) | `logic::convergence_tests::retarget_to_missing_element_reports_dangling` |
| deletes a shape | joins that shape to another, at the same moment | the arrow, bound to a shape nobody holds | nothing painted on either board, and both are told (`TOILE-165`) | `logic::convergence_tests::an_arrow_stranded_by_a_peer_is_announced` |
| drags a shape an arrow ends at, for a second | watches | the shape's new position, and no connector write at all | the arrow re-routed on every frame (`TOILE-164`) | none |
| types `unit cost` on an arrow's caption | types `cost in EUR` on the same caption, offline | one caption holding both edits, merged character by character | the same caption on both screens, neither user's typing dropped (`TOILE-047`) | `logic::convergence_tests::concurrent_captions_converge` |
| types `unit cost` on an arrow's caption | has that caption open, and adds `in EUR` | both changes, merged from the caption Bob opened on | Bob's typed caption for a moment, then both changes (`TOILE-263`) | `logic::convergence_tests::an_open_caption_editor_keeps_a_peers_edit` |

---

## 3. MODEL

**Containers.** The `connectors` map, one map per minted connector id, and one entry in the shared
paint order. `ambre-spec-toile-schema.md section 3` declares both, with the field layout.

**The routing shape** (`ambre-bus`). An anchor names a side, the centre, or a free point on the
element's box. A path kind decides how the arrow joins the two anchors:

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

**Steps, when the board paints a connector:**

1. read the two endpoint ids from the connector map;
2. resolve each anchor against the endpoint element's current box, taken from the bounds cache;
3. stop and paint nothing when either endpoint is absent (`TOILE-166`);
4. dispatch on the path kind:
   * a line;
   * an orthogonal path that leaves along the source anchor's side and ignores the target's;
   * a cubic whose control points leave along both sides;
5. hand the outline out with the two end caps.

Nothing in that sequence writes, which is the whole point. An element drag re-routes every connector
that touches it and produces no operation of its own (`TOILE-006`).

**Steps, when a user creates an arrow:** the handler mints an id. It writes both endpoints, both
anchors, the path kind and the caps into one connector map, then appends one entry to the paint
order. It resolves neither endpoint against the board and reads no lock. A create naming an element
nobody holds therefore lands, and dangles at the next read (`TOILE-168`).

**Steps, when a user deletes an endpoint element locally:** the delete handler collects every
connector referencing a removed id at either end. It removes those connector maps and strips them
from the paint order, inside the delete's own commit (`TOILE-042`). Under concurrency the cleanup
can lose the race, which is exactly the dangling case above.

**Steps, when a write this viewer did not make lands:** the set of connectors missing an endpoint is
read before and after. Each one that was whole and is not any more is announced (`TOILE-165`). A
remote delta, a history step and a peer's delete all reach this. The announcement is once per arrow:
one already dangling before the write is not announced again.

**Steps, when a user captions an arrow:** the handler resolves the connector map and takes its
`label` text. It **diffs** the whole caption into that text, rather than replacing it. Two peers
typing on one arrow therefore merge inside one text, instead of one body winning (`TOILE-047`).
The caption travels whole on the wire because the surface holds a plain buffer; the document stores
the char-CRDT.

The command also names the caption the editor opened on. The handler merges the change from that
base into the stored caption, as a note body does (`TOILE-UI-381`). A peer's edit that landed while
the editor was open therefore survives the close (`TOILE-263`).

The handler seeds the `label` container **at the connector's birth**, not on the first caption.
Seeding at creation puts the same container on every peer, inside the creation delta, before anyone
types. A lazy seed on each peer would mint rival containers under one key, and LWW would drop a
caption whole. A connector minted before the field existed is the one lazy seed left, and the one
window where that can happen.

**Steps, when the board places a caption:** it measures the routed path. It then centres the
caption's box on the point half-way along it by arc length. The document stores none of that
(`TOILE-232`):
moving an endpoint re-routes the arrow and carries the caption with it, at no write.

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
| a click on an arrow, with no element under the pointer | nothing: the arrow is selected, and the element selection is emptied (`TOILE-UI-350`, `TOILE-UI-354`) |
| an endpoint handle of the selected arrow, dragged onto a shape | `RetargetConnector` at that shape's nearest side; nothing on a release over empty board (`TOILE-UI-352`) |
| `Delete` with an arrow selected | `DeleteConnector` |
| a z-order entry with an arrow selected | `Reorder` over the shared paint order (`TOILE-043`) |
| a colour or a width from the style inspector, with an arrow selected | `RestyleConnector`, the register written whole (`TOILE-046`) |

**What the drawer sees.** A preview line from the source anchor to the pointer while the drag is in
flight, local and uncommitted (`features/ambre-spec-toile-tools.md`). The anchor picked at each end
is the nearest side of the element under the pointer.

**What the captioner sees.** The words centred on the arrow, over a plate in the board colour so
they stay readable where they cross the line (`TOILE-UI-346`). An arrow with no caption paints
nothing there and claims no space: the click target is the line alone (`TOILE-UI-347`). While the
editor is open the selection is empty. A `Delete` typed mid-caption therefore deletes a character,
rather than the shapes the user had selected before.

**What the selector sees.** The arrow traced in the selection colour with a handle on each end, and
no box (`TOILE-UI-351`). The view grabs the handle at a screen distance taken back into world units,
so it is equally easy to hit at any zoom. The drag paints the same rubber band the connector tool
draws. An arrow loses its endpoint, leaves the frame, and takes the selection with it.

**What no surface offers.** A context menu on an arrow. A right-click drops the arrow selection,
rather than offering entries that would do nothing to it. An arrow's selection also never reaches
the peers, because presence carries element ids -- section 6.

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

* **No pass culls a connector** (`TOILE-045`). The spatial index holds no connector, and the
  visible-set walk gates the element arm on the culled set and the connector arm on nothing. A board
  of two thousand arrows therefore routes and ships two thousand outlines every frame, whatever the
  camera holds.
* **An arrow's selection is invisible to the others.** A presence sample carries element ids
  (`features/ambre-spec-toile-presence.md`), so a peer who selects an arrow shows a peer who
  selected nothing. The ghost the others paint is an element box, and an arrow has none.
* **A user selects an arrow alone, and never with an element** (`TOILE-UI-354`). The two selections
  are separate fields that exclude one another, rather than one list of nodes.
  * That is a decision, not a shortcut. A marquee, a group, a move, a resize and the clipboard all
    need a box, and an arrow has none. It does mean that one gesture cannot remove an arrow and the
    shapes it joins, and that a select-all takes the shapes only.
* **No context menu addresses an arrow.** The menu takes its scope from the element selection. A
  right-click with an arrow selected would therefore offer group, lock, react and frame entries
  that do nothing to it. Rather than offer them, the right-click drops the arrow. Delete, the
  z-order and the style inspector reach it from the keyboard and the menu bar instead.
* **A selected arrow that leaves the frame reads as deleted.** The view drops the selection when
  the arrow leaves the display, which is right only because no pass culls a connector
  (`TOILE-045`). The day that row holds, an arrow panned off screen would deselect itself, and this
  has to ask the board instead of reading absence.
* **The notice names no arrow** (`TOILE-044`). The shell says an arrow lost an end, and the board
  paints nothing the user can point at (`TOILE-166`). The repair is the re-aim, and reaching it
  means finding an arrow that is not drawn. No rule says how a surface offers that.
* **Two path kinds and two anchors have no sender** (`TOILE-167`). The tool writes
  `PathType::Straight` on every create and picks among the four sides.
  * The elbow, the curve, the centre anchor and the free anchor are therefore shapes no gesture
    produces. The format stores them, the codec round-trips them, and the router handles them. The
    unit test that exercises all three kinds asserts only that each returns an outline.
* **A create trusts its endpoints** (`TOILE-168`). The handler validates neither id. A command from
  a peer running older code, or a replayed undo item, therefore writes an arrow to an element that
  is not there. The board carries it silently.
* **A caption has no style and one size.** The view paints it in the theme's text ink at a fixed
  world size in the default family. The arrow's style register now exists (`TOILE-046`), and the
  caption does not read it: colouring an arrow colours the line, never its words.
* **A caption's box takes a constant, not a fit.** The wrap width and the click box are a constant,
  chosen before the font context exists (the editor opens outside the paint pass). A caption longer
  than the box wraps onto a second line rather than widening, and a very long one runs past its
  plate.
* **The router aims at boxes, not at corners, inside a rotated frame.** The anchor resolution reads
  the rotation-expanded box. An arrow to a rotated shape therefore lands on that box's side, rather
  than on the shape's own edge.
