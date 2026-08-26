# AMBRE -- FEATURE: CONNECTORS

**Type:** Vertical (feature)
**Implementation:** partial -- an arrow is drawn and routed; it cannot be selected, styled or captioned.

> A user drags from one shape to another and gets an arrow. Moving either shape keeps the arrow
> attached: it is drawn between the two, not placed on the board.

An arrow stores what it joins, never where it runs. That one choice is why a drag costs one write
instead of one per arrow, and why two users can never make an arrow contradict itself.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`.

| ID | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|
| `TOILE-006` | Connector geometry MUST be recomputed on read; moving an element MUST NOT write a change to its connectors. | this spec section 3 | `ambre_toile::logic::connector::tests::straight_endpoints_are_the_anchors`, `::every_path_type_routes`, `::missing_endpoint_yields_none`; the format stores endpoints and a path kind, no geometry (`ambre-spec-toile-schema.md section 3`) | verified |
| `TOILE-007` | A connector whose endpoint element is deleted MUST become dangling; its data MUST NOT be destroyed or silently corrupted. | this spec section 2 | `ambre_toile::logic::convergence_tests::deleting_an_endpoint_makes_the_connector_dangle`, `::retarget_to_missing_element_reports_dangling`, `::retarget_rebinds_endpoint_without_dangling` | verified |
| `TOILE-042` | A local delete of an element MUST take the connectors it owns with it, in the same commit. | this spec section 3 | `ambre_toile::logic::convergence_tests::delete_drops_owned_connectors` | verified |
| `TOILE-043` | A connector MUST sit in the one paint order with the elements, so it can be placed behind one. | this spec section 3; `features/ambre-spec-toile-z-order.md` | `ambre_toile::logic::convergence_tests::reorder_to_back_moves_node` covers the reorder over the shared order. Nothing drives a connector through it, and no surface can select one to try (`TOILE-UI-035`) | unverified |
| `TOILE-044` | A dangling connector MUST NOT be painted, and MUST be announced so a surface can offer a repair. | this spec section 3 | `ambre_toile::logic::connector::tests::missing_endpoint_yields_none` holds the paint half; `::retarget_to_missing_element_reports_dangling` holds the announcement. Nothing consumes the announcement: the view drops the event | unverified |
| `TOILE-045` | A connector MUST be culled with the viewport, like an element. | this spec section 3 | none, and **the rule fails today**: the visible-set walk tests the culled set for an element and emits every connector on the board unconditionally, because no connector is in the spatial index -- section 6 | unverified |
| `TOILE-046` | A connector MUST carry its own style, so an arrow can be coloured and thickened. | this spec section 3 | none, and **the rule fails today**: the field is `design` in the format, no command writes it, and the deriver hands out a default style for every connector -- section 6 | unverified |
| `TOILE-047` | A caption on a connector MUST converge character by character, like a note body. | this spec section 3 | not built -- lands with the "Connector labels" Story (`ambre-roadmap.md` E4) | unimplemented |

* **Budget.** the routing pass is inside the visible-set derivation, budgeted by the visible-set row
  of `ambre-spec-budgets.md section 1`. Because `TOILE-045` fails, the pass runs once per
  connector on the board rather than once per visible connector, and no bench measures either.
* **Boundary.** none of its own: a connector lives in the Logic thread's document and leaves as bytes
  the Sync worker seals (`ZK-001`).
* **Verification.** 3 `verified`, 4 `unverified`, 1 `unimplemented`. The design that makes the
  feature cheap -- geometry on read, endpoints in the document -- is held three ways. Everything
  built on top of it is loose, and two of the four are the code answering a different rule.

---

## 2. WHAT EACH USER SEES

| Alice does | Bob does | The document holds | Each user sees | Test |
|---|---|---|---|---|
| moves a shape an arrow starts from | watches | the shape's new position, and nothing about the arrow | the arrow following the shape, re-routed on both screens | `logic::connector::tests::straight_endpoints_are_the_anchors` |
| deletes the shape an arrow ends at | re-aims that arrow at a third shape | either the deletion with the arrow gone, or the arrow bound to the third shape | one consistent board: no arrow pointing at nothing | `logic::convergence_tests::deleting_an_endpoint_makes_the_connector_dangle` |
| deletes the shape | keeps the arrow, offline | the arrow, referencing an element nobody holds | nothing painted where the arrow was, and the arrow still in the document | `logic::connector::tests::missing_endpoint_yields_none` |
| re-aims an arrow at a shape she just deleted | watches | the arrow bound to a missing element | nothing painted, and an announcement no surface reads (`TOILE-044`) | `logic::convergence_tests::retarget_to_missing_element_reports_dangling` |

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
3. stop and paint nothing when either endpoint is absent (`TOILE-007`, `TOILE-044`);
4. dispatch on the path kind -- a line, an orthogonal path that respects both anchor sides, or a
   cubic whose control points leave along those sides;
5. hand the outline out with the two end caps.

Nothing in that sequence writes, which is the whole point: an element drag re-routes every connector
that touches it and produces no operation of its own (`TOILE-006`).

**Steps, when an endpoint element is deleted locally:** the delete handler collects every connector
referencing a removed id at either end, removes those connector maps, and strips them from the paint
order inside the delete's own commit (`TOILE-042`). Under concurrency the cleanup can lose the race,
which is exactly the dangling case above.

**Code:**

* `crates/ambre-toile/src/logic/connector.rs` -- the anchor resolution and the three path kinds;
* `logic/state/connectors.rs` -- create, retarget, delete, and the "which connectors touch these
  elements" query the delete uses;
* `logic/state/derive.rs` -- the connector arm of the visible-set walk;
* `logic/state/codec.rs` -- the field encoding of an anchor and a path kind.

---

## 4. SURFACE

| Trigger | Sends |
|---|---|
| the connector tool, dragged from one element to another | `CreateConnector` between the two nearest anchors |
| the connector tool, released over empty board | nothing: an arrow needs two elements |

**What the drawer sees.** A preview line from the source anchor to the pointer while the drag is in
flight, local and uncommitted (`features/ambre-spec-toile-tools.md`). The anchor picked at each end
is the nearest side of the element under the pointer.

**What no surface offers.** Selecting an arrow, deleting one, re-aiming one, colouring one, or
captioning one. The two commands that would serve the first three exist and nothing sends them,
because the pick walk drops every connector before testing geometry (`TOILE-UI-035`).

---

## 5. DECISIONS

| Decision | Rationale | Rejected alternative |
|---|---|---|
| A connector stores what it joins, never where it runs | a stored path would have to be rewritten on every frame of every drag of either endpoint, turning one register write into one per attached arrow, and giving two peers a geometry to disagree about. Recomputing costs a resolution per visible arrow per frame and cannot be wrong | persisting the routed path, which the earlier design carried |
| A deleted endpoint leaves the connector in place | data is never destroyed by a merge. The local delete cleans up as an intentional batch; a concurrent one cannot, so the connector survives referencing a missing id and is simply skipped -- the same stance a merged-away cell takes in `features/ambre-spec-trame-cell-merges.md` | clearing the reference, which turns one user's delete into another user's data loss |
| Connectors live in the shared paint order | an arrow behind a shape is an ordinary diagram, and a separate always-on-top pass makes it impossible | a connector layer painted after every element |
| An anchor is a side, not a point | a side survives a resize and a rotation of the element, and a stored point does not. The free anchor exists for the case a user places deliberately | a stored offset from the element origin |
| End caps are two flags on the connector, not a style attribute | the direction of an arrow is what it means, so it belongs to the connector rather than to its style, and it survives a restyle | folding the caps into the style register |

---

## 6. NOT BUILT, AND NOT YET RULED

* **A connector is never culled** (`TOILE-045`). No connector is in the spatial index, and the
  visible-set walk gates the element arm on the culled set and the connector arm on nothing. A board
  of two thousand arrows therefore routes and ships two thousand outlines every frame, whatever the
  camera holds.
* **A connector has no style** (`TOILE-046`). The format declares the field as `design`, no
  command writes it, and the deriver hands the UI thread a default style for every connector, so
  every arrow on every board is the same colour and the same width.
* **A connector cannot be reached** (`TOILE-UI-035`). It cannot be selected, so the retarget and
  delete commands have no sender, and the caption editor three sections describe has nothing to open
  on. The selection half of that is the first thing the "Connector labels" Story has to build.
* **The announcement goes nowhere** (`TOILE-044`). The dangling event reaches the view, which drops
  it, so the repair affordance the design names does not exist and a dangling arrow is
  indistinguishable from no arrow.
* **A connector between two elements inside a rotated frame is routed against boxes, not corners.**
  The anchor resolution reads the rotation-expanded box, so an arrow to a rotated shape lands on that
  box's side rather than on the shape's own edge.
