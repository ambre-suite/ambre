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

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TOILE-FEAT-006` | `INV` | Each combinator MUST be area-correct: a union is the two areas minus their overlap, an intersection is the overlap, a subtraction is the subject minus the union of the rest, an exclusion is the symmetric difference. | this spec section 3 | `ambre_toile::logic::boolean::tests::union_area_is_combined_minus_overlap`; `::intersect_area_is_overlap`; `::subtract_removes_the_clip`; `::exclude_is_symmetric_difference`; `::union_of_disjoint_keeps_both` -- a degenerate input; `::intersect_of_disjoint_is_empty` -- another; `::subtract_folds_over_multiple_clips` -- the subject against several clips | none | verified |
| `TOILE-FEAT-033` | `INV` | A combinator MUST return the same outline for the same inputs. | this spec section 3 | `ambre_toile::logic::boolean::tests::deterministic_across_runs` -- it calls the solver twice in one process and compares the SVG, so it holds one build against itself, which is the rule as stated. Nothing compares two builds or two platforms, which is the wider rule the row once carried | none | verified |
| `TOILE-FEAT-018` | `INV` | A boolean operation MUST be reachable from a surface. | this spec section 4 | not built | unbuilt -- lands with the `Boolean operations reach a surface` Story (`ambre-roadmap.md` E6). The command has no menu entry, no button and no chord anywhere in the workspace, section 6 | unimplemented |
| `TOILE-FEAT-112` | `INV` | The flatten tolerance MUST be finite and positive. | this spec section 3 | `ambre_toile::logic::boolean::tests::a_flatten_tolerance_is_finite_and_positive` -- the constructor refuses zero, a negative, NaN and infinity | none | verified |

### 1.2 Draw a path with the pen

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TOILE-UI-057` | `POST` | Finishing a path MUST emit exactly one create. | this spec section 3 | `ambre_toile::ui::tool::tests::pen_clicks_then_double_click_emits_path` -- it reads the one outcome the double-click returns. The two tests this row also cited drive a Shape and a Sticky, never the pen | none | verified |
| `TOILE-UI-058` | `POST` | A node dragged out MUST make a curved segment. | this spec section 3 | `ambre_toile::ui::tool::tests::a_dragged_pen_node_makes_a_curved_segment` | none | verified |
| `TOILE-UI-102` | `POST` | A node clicked without a drag MUST make a straight segment. | this spec section 3 | `ambre_toile::ui::tool::tests::clicked_pen_nodes_make_straight_segments` | none | verified |
| `TOILE-UI-103` | `FRAME` | Placing a node MUST NOT write to the document. | this spec section 3 | `ambre_toile::ui::tool::tests::placing_a_pen_node_writes_nothing` -- what every press, drag and release answered | none | verified |
| `TOILE-UI-104` | `FAIL` | A finish left with fewer than two distinct nodes MUST emit nothing. | this spec section 3 | `ambre_toile::ui::tool::tests::a_pen_finish_on_one_node_emits_nothing` | none | verified |
| `TOILE-UI-105` | `FAIL` | A path abandoned by `Escape` or a tool change MUST emit nothing. | this spec section 3 | `ambre_toile::ui::tool::tests::an_abandoned_pen_path_emits_nothing` -- `Escape` and a tool change | none | verified |

### 1.3 Combine paths

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TOILE-FEAT-034` | `POST` | Combining MUST write one path element carrying the result and the first source's style, and consume the sources in the same commit unless the caller keeps them. | this spec section 3 | `ambre_toile::logic::state::boolean::tests::union_of_two_rects_makes_one_path_element`; `::boolean_is_one_undo_item`; `::keep_sources_leaves_inputs`; `ambre_toile::logic::convergence_tests::a_combined_path_wears_the_first_sources_style` | none | verified |
| `TOILE-FEAT-035` | `FRAME` | Combining MUST NOT delete an element it did not resolve as a source: a locked element and a non-shape are ignored, not consumed. | this spec section 3 | `ambre_toile::logic::convergence_tests::a_combine_consumes_only_its_sources` -- a locked shape and a note among the ids | none | verified |
| `TOILE-FEAT-036` | `FAIL` | Fewer than two eligible outlines MUST be refused rather than write a degenerate element. | this spec section 3 | `ambre_toile::logic::state::boolean::tests::fewer_than_two_eligible_is_rejected` -- the handler; `ambre_toile::logic::boolean::tests::fewer_than_two_paths_is_empty` -- the solver | none | verified |
| `TOILE-FEAT-037` | `FAIL` | An empty result MUST be refused rather than write an element with no outline. | this spec section 3 | `ambre_toile::logic::convergence_tests::an_empty_combine_is_refused` | none | verified |

* **Budget.** the in-flight preview under 0.5 ms, one row of
  `ambre-spec-budgets.md section 2` -- it is a view-state update per click, not per frame. The
  combinators have no budget of their own: they are frozen, so the boolean-combination row of
  `ambre-spec-budgets.md section 1` is a figure this document does not adopt.
* **Boundary.** none: the pen is view state on the UI thread, where `UI-THREAD-004` bans a network,
  crypto or persistence path. The finished path enters the document as one create and leaves as
  bytes the Sync worker seals (`ZK-001`).
* **Verification.** 13 `verified`, 1 `unimplemented`. What holds is the pen and the combinators. The
  tests read the span each pen gesture places and both refusals. They also read the frame conditions
  that keep a path in flight off the board, and the style the result wears. What is owed is a
  gesture that invokes a combinator.

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
kind and whose shape tag is the path tag. It carries its outline as path data
(`ambre-spec-toile-schema.md section 3`).

**View state, while the pen is active.** A list of nodes, each a point plus an optional handle, and a
flag saying whether the current node is being dragged. Both die with the gesture.

**Steps, when a path is drawn:**

1. each click appends a node at the pointer; holding and dragging gives that node a handle;
2. the preview is rebuilt from the nodes on every change and painted locally
   (`features/ambre-spec-toile-tools.md`);
3. a double-click finishes: the nodes become one outline -- a cubic span where either end carries a
   handle (`TOILE-UI-058`), a straight span otherwise (`TOILE-UI-102`). A finish left with fewer than
   two distinct nodes emits nothing (`TOILE-UI-104`);
4. one create is emitted, with the outline as its content (`TOILE-UI-057`);
5. `Escape` or a tool change drops the nodes and emits nothing (`TOILE-UI-105`).

**Command** (`ambre-bus`): `BooleanOp { op, ids, keep_sources }`, one undo item.

**Steps, when paths are combined:**

1. resolve each listed element to its world outline, skipping a locked one and any kind that is not a
   shape;
2. refuse the operation below two eligible outlines (`TOILE-FEAT-036`), consuming none of them
   (`TOILE-FEAT-035`);
3. flatten each outline to polygon contours and combine them under the matching rule. Subtract takes
   the first as the subject and removes the union of the rest, and the others fold left to right;
4. refuse an empty result rather than write a degenerate element (`TOILE-FEAT-037`);
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
| `Escape`, or switching tools | drops the gesture, and nothing reaches the document |

**What no surface offers** (`TOILE-FEAT-018`). Union, subtract, intersect and exclude have no entry in
the object menu, no button in the style panel and no chord. The command exists, is tested nine
ways, and has no sender anywhere in the workspace.

**What is deliberately absent.** Selecting a path and dragging its nodes. The behaviour is
vector-illustration work, which this application rejects at the product line. The earlier design's
claim is withdrawn here rather than turned into an unbuilt row (section 5, section 6).

---

## 5. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| The pen builds the whole path before emitting anything | emitting a create on the first node and editing it per node | it puts an unfinished shape in everyone's history. A node is a view-state append, so a path of twelve nodes is one create rather than twelve and an abandoned path costs nothing -- the same rule the other creation tools follow |
| A finished path is an ordinary element | a path element with its own commands | as an ordinary element it moves, resizes, rotates, restyles, groups and deletes with everything else, so the pen adds a tool and no lifecycle |
| Node editing is not part of the product | a node-editing mode, which three sections of the earlier design described | reshaping a curve node by node is an illustration tool's job, and this application is a whiteboard. Naming it in the spec while nothing built it made the corpus describe a competitor |
| The combinators stay in the tree, frozen | removing them | it would drop working geometry that exists, passes and costs nothing to keep |
| The combinators stay in the tree, frozen | giving them a surface and a promise | stabilizing them would spend MVP time on the wrong product |
| Curves are flattened to combine | an exact boolean | the geometry library has no path booleans, and an exact curve-boolean implementation is a project of its own |
| Curves are flattened to combine | no combinators at all | a whiteboard reads a flattened outline as the same shape, so dropping the operation would cost a tool to avoid a difference nobody on this surface can see |
| Subtract takes the first selection as the subject | the paint order | the user did not choose it; the selection order is the one signal they gave, and it is the only ordering they can predict |

---

## 6. NOT BUILT, AND NOT YET RULED

* **Nothing can invoke a combinator** (`TOILE-FEAT-018`). Four operations, nine passing tests, and no
  menu entry, button or chord in the workspace. The frozen status explains why no surface was built and
  does not explain why the corpus read as though one had been.
* **A path cannot be reshaped.** The design named a node-editing mode: select a path, drag its nodes
  and handles. No command changes a path's outline after its create, the combinators aside. The
  claim is withdrawn above rather than recorded as owed.
* **A path is stored in world coordinates.** The outline is written absolute and read back absolute,
  and the element also carries a rotation. What a rotated path means is decided by the paint
  transform rather than by the stored data.
* **The combinators ignore rotation of the sources.** Each source outline is transformed by the same
  rotation the renderer uses, so the result is correct. The result element is written unrotated, and
  nothing says what a subsequent rotation of it should do to its stored outline.
