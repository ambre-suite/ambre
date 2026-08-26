# AMBRE -- FEATURE: VECTOR PATHS

**Type:** Vertical (feature)
**Implementation:** partial -- the pen draws paths; the combinators have no surface at all.

> A user clicks a sequence of points and gets a shape of their own making, straight or curved. It is
> then an ordinary element of the board.

Drawing a path is a whiteboard gesture. Reshaping one afterwards is a vector-illustration gesture,
and this application is deliberately not that (`features/ambre-spec-toile-engine.md section 5`).

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`.

| ID | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|
| `TOILE-FEAT-006` | A boolean operation MUST be deterministic across runs and area-correct; fewer than two eligible paths MUST be rejected rather than silently produce a degenerate element. | this spec section 3 | `ambre_toile::logic::boolean::tests::deterministic_across_runs`, `::union_area_is_combined_minus_overlap`, `::intersect_area_is_overlap`, `::subtract_removes_the_clip`, `::exclude_is_symmetric_difference`, `::fewer_than_two_paths_is_empty`, `ambre_toile::logic::state::boolean::tests::fewer_than_two_eligible_is_rejected`, `::keep_sources_leaves_inputs`, `::boolean_is_one_undo_item` | verified |
| `TOILE-FEAT-018` | A boolean operation MUST be reachable from a surface. | this spec section 4 | none, and **the rule fails today**: the command has no menu entry, no button and no chord anywhere in the workspace, so nine tests hold behaviour nothing can invoke -- section 6 | unverified |
| `TOILE-UI-057` | The pen MUST build its path on the UI thread and MUST emit exactly one create on finish. | this spec section 3 | `ambre_toile::ui::tool::tests::pen_clicks_then_double_click_emits_path`, `::switching_tools_cancels_in_flight_gesture`, `::escape_reverts_an_armed_creation_tool_to_select` | verified |
| `TOILE-UI-058` | A node dragged out MUST make a curved segment, and a node clicked MUST make a straight one. | this spec section 3 | none. The builder emits a cubic when either end of a span carries a handle and a line otherwise, and the one pen test asserts the kind of the created element rather than the shape of its path | unverified |

* **Budget.** the in-flight preview under 0.5 ms, one row of
  `ambre-spec-budgets.md section 2` -- it is a view-state update per click, not per frame. The
  combinators have no budget of their own: they are frozen, so the boolean-combination row of
  `ambre-spec-budgets.md section 1` is a figure this document does not adopt.
* **Boundary.** none: the pen is view state on the UI thread, where `UI-THREAD-004` bans a network,
  crypto or persistence path. The finished path enters the document as one create and leaves as bytes
  the Sync worker seals (`ZK-001`).
* **Verification.** 2 `verified`, 2 `unverified`. The combinator geometry is the most thoroughly tested
  pure function in the application, and nothing can call it. The pen's one-command rule holds; the
  shape of what it builds does not.

---

## 2. WHAT EACH USER SEES

| Alice does | Bob does | The document holds | Each user sees | Test |
|---|---|---|---|---|
| draws a path | draws a path at the same moment | both paths, as two independent elements | both shapes | `logic::convergence_tests::two_peers_converge` |
| combines two paths, consuming them | watches | one path element, and the two sources gone, from one change | the combined shape alone, on both screens | `logic::state::boolean::tests::boolean_is_one_undo_item` |
| combines two paths, keeping them | deletes one source | the result and one surviving source | the result, unaffected by the deletion: it is a plain element | `logic::state::boolean::tests::keep_sources_leaves_inputs` |
| is drawing, three nodes placed | watches | nothing yet | Alice's outline under her pointer, and nothing on Bob's screen | `ui::tool::tests::pen_clicks_then_double_click_emits_path` |

An in-flight path is not presence: unlike a freehand stroke, nothing streams it
(`features/ambre-spec-toile-freehand.md`).

---

## 3. MODEL

**Containers.** None of its own. A path is an element of the `elements` map whose kind is the shape
kind and whose shape tag is the path tag, carrying its outline as path data
(`ambre-spec-toile-schema.md section 3`).

**View state, while the pen is active.** A list of nodes, each a point plus an optional handle, and a
flag saying whether the current node is being dragged. Both die with the gesture.

**Steps, when a path is drawn:**

1. each click appends a node at the pointer; holding and dragging gives that node a handle;
2. the preview is rebuilt from the nodes on every change and painted locally
   (`features/ambre-spec-toile-tools.md`);
3. a double-click finishes: the nodes become one outline -- a cubic span where either end carries a
   handle, a straight span otherwise (`TOILE-UI-058`);
4. one create is emitted, with the outline as its content (`TOILE-UI-057`);
5. `Escape` or a tool change drops the nodes and emits nothing.

**Command** (`ambre-bus`): `BooleanOp { op, ids, keep_sources }`, one undo item.

**Steps, when paths are combined:**

1. resolve each listed element to its world outline, skipping a locked one and any kind that is not a
   shape;
2. refuse the operation below two eligible outlines (`TOILE-FEAT-006`);
3. flatten each outline to polygon contours and combine them under the matching rule -- subtract takes
   the first as the subject and removes the union of the rest, the others fold left to right;
4. refuse an empty result rather than write a degenerate element;
5. write one new path element carrying the result and the first source's style;
6. delete the sources in the same commit unless they are kept, then commit once.

**Why the contours are flattened.** The geometry library carries no path-versus-path booleans, so the
combinators run on a polygon overlay library instead. Curves become polylines, which is acceptable on a
whiteboard and is the trade-off the frozen status accepts.

**Code:**

* `crates/ambre-toile/src/ui/tool.rs` -- the pen nodes, the preview and the outline builder;
* `logic/boolean.rs` -- the pure combinators;
* `logic/state/boolean.rs` -- the handler, the eligibility filter and the single commit;
* `logic/state/derive.rs` -- the path read back for painting.

---

## 4. SURFACE

| Trigger | Behaviour |
|---|---|
| the pen tool, then a click | places a node |
| the pen tool, then a click and drag | places a node with a handle, giving the span its curve |
| a double-click | finishes the path and emits the create |
| `Escape`, or switching tools | drops the gesture |

**What no surface offers** (`TOILE-FEAT-018`). Union, subtract, intersect and exclude have no entry in
the object menu, no button in the style panel and no chord. The command exists, is tested nine
ways, and has no sender anywhere in the workspace.

**What is deliberately absent.** Selecting a path and dragging its nodes. The behaviour is
vector-illustration work, which this application rejects at the product line, so the earlier design's
claim is withdrawn here rather than turned into an unbuilt row (section 5, section 6).

---

## 5. DECISIONS

| Decision | Rationale | Rejected alternative |
|---|---|---|
| The pen builds the whole path before emitting anything | a node is a view-state append, so a path of twelve nodes is one create rather than twelve, and an abandoned path costs nothing. It is the same rule the other creation tools follow | emitting a create on the first node and editing it per node, which puts an unfinished shape in everyone's history |
| A finished path is an ordinary element | it moves, resizes, rotates, restyles, groups and deletes with everything else, so the pen adds a tool and no lifecycle | a path element with its own commands |
| Node editing is not part of the product | reshaping a curve node by node is an illustration tool's job, and this application is a whiteboard. Naming it in the spec while nothing built it made the corpus describe a competitor | a node-editing mode, which three sections of the earlier design described |
| The combinators stay in the tree, frozen | the code exists, passes and costs nothing to keep; removing it would discard working geometry, and stabilizing it would spend an MVP budget on the wrong product | removing them, or giving them a surface and a promise |
| Curves are flattened to combine | the geometry library has no path booleans, and an exact curve-boolean implementation is a project of its own. A whiteboard reads a flattened outline as the same shape | an exact boolean, or no combinators at all |
| Subtract takes the first selection as the subject | it is the only ordering a user can predict, and the selection order is the one signal they gave | the paint order, which the user did not choose |

---

## 6. NOT BUILT, AND NOT YET RULED

* **Nothing can invoke a combinator** (`TOILE-FEAT-018`). Four operations, nine passing tests, and no
  menu entry, button or chord in the workspace. The frozen status explains why no surface was built and
  does not explain why the corpus read as though one had been.
* **A path cannot be reshaped.** The design named a node-editing mode -- select a path, drag its nodes
  and handles -- and no command changes a path's outline after its create, the combinators aside. The
  claim is withdrawn above rather than recorded as owed.
* **The pen has no live shape test** (`TOILE-UI-058`). The one test asserts that a create with a path
  arrives; nothing asserts that a dragged node produced a curve, so the builder could emit straight
  spans throughout and fail nothing.
* **A path is stored in world coordinates.** The outline is written absolute and read back absolute,
  and the element also carries a rotation, so what a rotated path means is decided by the paint
  transform rather than by the stored data.
* **The combinators ignore rotation of the sources.** Each source outline is transformed by the same
  rotation the renderer uses, so the result is correct; the result element is written unrotated, and
  nothing says what a subsequent rotation of it should do to its stored outline.
