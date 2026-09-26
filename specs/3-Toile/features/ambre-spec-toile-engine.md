# AMBRE -- FEATURE: THE TOILE ENGINE
**Type:** Vertical (feature)
**Implementation:** partial -- the engine, the seam and the delta cursor are built. An index
update and a remote change cost the whole board.

> A user drags a note across an endless plane, and every collaborator sees it move. One thread holds
> the board, writes the change, and answers with the elements the screen shows. The other thread
> paints that answer and owns nothing else.

The engine is `ToileState`, alone in the Logic thread. Every Toile feature reaches the board through
it, so the index, the derivation and the seam are one document and not three.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TOILE-001` | `INV` | The board CRDT MUST NOT leave the Logic thread; it exits only as bytes. | this spec section 3; `ambre-spec-architecture.md section 2` | type-level -- `ToileState.doc` is private and **no public signature in `ambre-toile` names `LoroDoc`**, and both exits return `Vec<u8>` | none | verified |
| `TOILE-004` | `INV` | The spatial index MUST be a derived cache: the incrementally maintained index MUST hold the same elements as a from-scratch rebuild. | this spec section 3 | proptest -- `ambre_toile::logic::index::prop::incremental_matches_rebuild` draws an upsert and remove stream and compares the population and the paint set against a rebuild; proptest -- `ambre_toile::logic::convergence_tests::caches_match_a_full_rebuild` draws an operation stream and compares the same (neither compares them at a camera that culls, which is `TOILE-206`) | none | verified |
| `TOILE-206` | `INV` | The visible set the incremental index answers MUST equal the rebuild's at a camera that culls, not only at one that shows everything. | this spec section 3 | none | unheld -- `ambre_toile::logic::index::prop::incremental_matches_rebuild` probes one viewport covering the whole populated area, and both `ToileState` proptests derive under `wide_viewport`, whose comment says it culls nothing. The equivalence is held over the population, never over a cull; lands with the `The Toile belts` Story (`ambre-roadmap.md` E9) | untested |
| `TOILE-005` | `INV` | The visible set MUST be resolved from the spatial index. | this spec section 3 | `ambre_toile::logic::index::tests::remove_then_query_is_empty`; `ambre_toile::logic::convergence_tests::viewport_culls_offscreen_nodes` | none | verified |
| `TOILE-240` | `INV` | The visible set MUST NOT drop an element that meets the viewport. | this spec section 3 | `ambre_toile::logic::index::tests::query_visible_culls_offscreen` -- what the viewport meets and what it does not | none | verified |
| `TOILE-207` | `INV` | The answer the UI receives MUST carry nothing the camera does not meet. | this spec section 3 | none | fails -- the derivation culls elements against the index and walks connectors unconditionally, so every connector of the board is routed and shipped at every camera. `viewport_culls_offscreen_nodes` filters `DisplayNode::Connector` out of what it reads, so no test can see it; lands with the `Engine and render optimization` Story (`ambre-roadmap.md` E6) | failing |
| `TOILE-017` | `CONC` | Two peers exchanging the same operations in any interleaving MUST converge to the same board and the same derived caches. | this spec section 2; `ambre-spec-testing.md` | `ambre_toile::logic::convergence_tests::two_peers_converge` -- the same operations in two interleavings; proptest -- `ambre_toile::logic::convergence_tests::caches_match_a_full_rebuild` draws create, move, resize, rotate, restyle, delete, reorder, group, ungroup, connect, stroke, frame, slide order, auto-layout and react | none | verified |
| `TOILE-208` | `INV` | The answer the UI receives MUST carry no presence sample and no acknowledgement. | this spec section 3 | type-level -- `DisplayViewport` has four fields (the camera, the paint order, the reaction tallies and the comment pins), no display event acknowledges a command, and the absence is the proof | none | verified |
| `TOILE-023` | `INV` | `ambre-toile` MUST contain no `unsafe`. | README pillar "Security" | none | claim -- the crate measures zero `unsafe` today, but its root carries no `#![forbid(unsafe_code)]`, so nothing refuses the next one (`ambre-spec-testing.md`, the mechanism ladder); lands with the `The Toile belts` Story (`ambre-roadmap.md` E9) | untested |
| `TOILE-089` | `INV` | A command handler MUST NOT send on a queue: it returns an outcome, and nothing else. | this spec section 3; `ambre-spec-message-bus.md` | type-level -- `ToileState` holds no sink and no sender, every handler returns `ToileOutcome` by value, and the absence is the proof | none | verified |
| `TOILE-UI-099` | `INV` | Every board mutation MUST leave the UI thread as a command; the UI MUST NOT write board state locally. | `ambre-spec-ui.md section 2` (`UI-THREAD-020`) | type-level -- the three outbound paths of `ui/view.rs` (`send_lossy`, `send_edit`, `send_open`) each hand a command to the Logic queue, there is no other sender, and `TOILE-UI-001` bans the CRDT on this side | none | verified |
| `TOILE-UI-311` | `INV` | The UI thread MUST NOT block on the Logic queue. | `ambre-spec-architecture.md section 2`; `ambre-spec-ui.md section 1` | type-level -- the three senders are `try_send` and no path awaits capacity, and the absence is the proof | none | verified |
| `TOILE-UI-101` | `INV` | The canvas dispatch over the display events MUST be exhaustive -- no wildcard arm -- so a new event is a compile error and never a silent drop. | this spec section 4 | exhaustive-match -- the dispatch of `ui/view.rs` carries **no `_` arm**, so adding an event fails the build | none | verified |

### 1.2 Do one thing to the board

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TOILE-209` | `PRE` | A command MUST cull against the last camera the UI reported, or against the default one while no camera has arrived. | this spec section 3 | none | unheld -- the field is seeded with `Viewport::default()` and no caller checks whether the UI has reported yet. Nothing refuses a command on an unreported camera, and nothing reads what it answers; lands with the `The Toile belts` Story (`ambre-roadmap.md` E9) | untested |
| `TOILE-002` | `POST` | Derivation MUST finish before the command returns. | this spec section 3 | type-level -- `handle_command` returns `ToileOutcome` by value and the crate has no async path; `ambre_toile::logic::state::derive::tests::derive_viewport_at_uses_the_given_camera` | none | verified |
| `TOILE-210` | `POST` | The outcome MUST carry every display event the command produced. | this spec section 3 | `ambre_toile::logic::convergence_tests::viewport_culls_offscreen_nodes` -- it reads the derivation through a test accessor, while thirty-odd tests of the same file read `out.display` directly, over creation, deletion, grouping, find, stats and presence | none | verified |
| `TOILE-211` | `FAIL` | A command naming an element that is gone MUST answer with an error and MUST NOT advance the export cursor. | this spec section 3; `ambre-spec-delta-order.md` | type-level -- the two refusal helpers return `ToileOutcome::display_only`, which carries no delta and never reaches the cursor, and the construction is the proof | none | verified |
| `TOILE-212` | `FRAME` | A camera command MUST NOT write to the board and MUST NOT produce a delta. | this spec section 3 | type-level -- the camera arm stores the viewport and returns `ToileOutcome::display_only`, so it neither commits nor exports, and the construction is the proof | none | verified |

### 1.3 Take in a change from a collaborator

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TOILE-092` | `POST` | A remote change MUST rebuild only the derived caches it touched. | this spec section 3 | none | fails -- the remote path calls `rebuild_caches`, which clears the bounds map, bulk-loads the index, re-reads the paint order and re-unions every group to a fixpoint, whatever the change carried; lands with the `Engine and render optimization` Story (`ambre-roadmap.md` E6) | failing |
| `TOILE-213` | `POST` | A batch of remote deltas MUST repaint once, not once per delta. | this spec section 3 | none | unheld -- `apply_remote_batch` amortizes the rebuild by construction, but no test of `ambre-toile` calls it. Trame holds the same rule with a test (`features/ambre-spec-trame-engine.md`); lands with the `The Toile belts` Story (`ambre-roadmap.md` E9) | untested |
| `TOILE-214` | `FAIL` | A remote change that cannot be imported MUST tell the user their board is behind. | this spec section 3 | `ambre_toile::logic::convergence_tests::a_remote_change_that_cannot_import_reports_an_import_failure` -- a refused delta answers a distinct `RemoteImportFailed`, never an empty-id error | none | verified |
| `TOILE-215` | `FAIL` | One delta of a batch that fails to import MUST NOT leave the board silently behind the rest. | this spec section 3 | `ambre_toile::logic::convergence_tests::a_batch_with_one_failed_delta_reports_the_failure` -- one bad delta of two reports the failure while the survivor still merges | none | verified |
| `TOILE-216` | `FRAME` | A remote change MUST NOT be re-broadcast. | this spec section 3; `ambre-spec-delta-order.md` | type-level -- the remote path returns `Vec<ToileEvent>` and not an outcome, so it has nowhere to put a delta | none | verified |

### 1.4 Hand a change to the Sync worker

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TOILE-090` | `POST` | An export that fails MUST NOT advance the export cursor; the range MUST be offered again. | this spec section 3; `ambre-spec-delta-order.md` | `ambre_toile::logic::state::cursor_tests::a_failed_export_keeps_the_range_for_the_next_flush` -- it hands the cursor a failure and then a success and reads it back after each; `ambre_toile::logic::state::cursor_tests::an_empty_export_is_not_a_failure` -- the other side, where a successful export of nothing still advances | none | verified |
| `TOILE-217` | `POST` | A second export from the same cursor MUST repeat no operation already broadcast. | this spec section 3; `ambre-spec-delta-order.md` | `ambre_toile::logic::convergence_tests::a_flush_with_nothing_new_exports_nothing` -- the idle case | none | verified |
| `TOILE-244` | `POST` | An import MUST NOT be broadcast back. | this spec section 3; `ambre-spec-delta-order.md` | `ambre_toile::logic::convergence_tests::an_import_is_not_broadcast_back` -- a peer holding nothing reads the flush that follows an import and finds none of the imported work in it | none | verified |
| `TOILE-236` | `POST` | Taking the cursor past an import MUST keep this peer's own counter. | this spec section 3; `ambre-spec-delta-order.md` | `ambre_toile::logic::convergence_tests::an_import_does_not_swallow_our_own_unbroadcast_genesis` -- the peer receiving the next delta can resolve it | none | verified |
| `TOILE-245` | `POST` | A restore from local storage MUST NOT move the export cursor at all. | this spec section 3; `ambre-spec-delta-order.md` | `ambre_toile::logic::convergence_tests::a_restore_still_offers_what_this_client_wrote_offline` -- a session written offline still reaches a peer after a reopen | none | verified |
| `TOILE-218` | `FRAME` | Exporting MUST NOT write to the board. | this spec section 3 | type-level -- the export reads the document and the stored cursor and commits nothing, and the construction is the proof | none | verified |

### 1.5 Answer a request for the whole board

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TOILE-093` | `POST` | A snapshot request MUST answer with the board's snapshot. | this spec section 3 | none | fails -- the snapshot command returns an empty outcome, and the dispatcher answers a Sync request from the document state directly instead. No caller sends the command; lands with the `Toile's snapshot command answers` Story (`ambre-roadmap.md` E7) | failing |
| `TOILE-219` | `FRAME` | Answering with a snapshot MUST NOT write to the board and MUST NOT move the export cursor. | this spec section 3; `ambre-spec-delta-order.md` | type-level -- `export_snapshot` takes `&self`, so it can write neither the board nor the cursor | none | verified |

### 1.6 Move one element in the index

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TOILE-091` | `POST` | Dropping an element from the spatial index MUST NOT cost a walk of the index. | this spec section 3; `ambre-spec-budgets.md section 1` | none | fails -- `SpatialIndex::remove` scans every leaf to find the one it removes, and `update` is a remove followed by an insert, so one drag step costs a walk of the whole board; lands with the `Engine and render optimization` Story (`ambre-roadmap.md` E6) | failing |
| `TOILE-220` | `POST` | An element dropped from the index MUST NOT come back in a query. | this spec section 3 | `ambre_toile::logic::index::tests::remove_then_query_is_empty`; proptest -- `ambre_toile::logic::index::prop::incremental_matches_rebuild` draws a removal branch and compares against a rebuild | none | verified |
| `TOILE-221` | `FRAME` | Updating one element's leaf MUST NOT change another element's. | this spec section 3 | proptest -- `ambre_toile::logic::index::prop::incremental_matches_rebuild` draws an interleaved upsert and remove stream over eight keys and reads it against a model map, so a leaf touched in place of another diverges from the rebuild | none | verified |

### 1.7 Send a command from the drawing thread

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TOILE-UI-312` | `POST` | A command sent from the drawing thread MUST reach the Logic thread addressed to the document it was drawn on. | this spec section 4; `ambre-spec-message-bus.md` | type-level -- the two board paths wrap the command in `LogicCommand::App` with the view's own document id, the id is not a parameter of the send, and the construction is the proof | none | verified |
| `TOILE-UI-100` | `FAIL` | A full queue MUST drop an edit rather than block the frame, and MUST say so. | `ambre-spec-architecture.md section 2`; `ambre-spec-ui.md section 1` | type-level -- the edit path and the embed-session path are `try_send` and each handles `TrySendError::Full` with a warning | none | verified |
| `TOILE-UI-313` | `FAIL` | A full queue MUST drop a camera request rather than block the frame, and MUST say so. | `ambre-spec-ui.md section 1` | type-level -- the camera path is a try-send whose `Full` arm warns and returns `false`, and the arm is the only way the call answers a full queue | none | verified |
| `TOILE-UI-314` | `FRAME` | Sending a command MUST NOT change what the canvas draws: the gesture is already on screen and no answer is awaited. | this spec section 4 | type-level -- the three senders take `&self`, so no send can touch the view's render state | none | verified |

### 1.8 Fan one answer out to the three threads

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TOILE-222` | `POST` | One outcome MUST reach the three destinations it names: the display events to the drawing thread, the delta and the presence frame to the Sync worker, the service requests to the service plane. | this spec section 3; `ambre-spec-message-bus.md` | `ambre_logic::dispatch::tests::toile_upload_request_lowers_with_doc_id_and_records_route` -- the service-request leg alone, the others being held by the dispatcher's own belt (`ambre-spec-architecture.md section 2`) | none | verified |
| `TOILE-223` | `FRAME` | Fanning an outcome out MUST NOT let a session or a connection event reach the board state, except the canonical user id the session seeds. | this spec section 3; `ambre-spec-membership.md` | type-level -- the only session-shaped entry point on the state is the user-id seed, no connection type appears in any signature of `ToileState`, and the absence is the proof | none | verified |

* **Budget.** four rows of `ambre-spec-budgets.md section 1`, plus the board holder of section 6. The rows:
  the visible-set query under 1 ms at 50 000 elements, the index update under 50 us. Then a remote change
  under 2 ms, a transform command under 1 ms. None has a benchmark; two are rules the code breaks
  (`TOILE-091`, `TOILE-092`).
* **Boundary.** none of its own: the board lives in this thread's CRDT and leaves as bytes the Sync
  worker seals (`ZK-001`). Its one outbound flow of user content is the media path of
  `features/ambre-spec-toile-media.md section 1`.
* **Verification.** 32 `verified`, 4 `failing`, 4 `untested`. What holds is the pair no other application
  holds: the index equivalence and the convergence of every derived cache. Both under one property test,
  neither at a culling camera (`TOILE-206`). What is loose is four rules nothing exercises and
  `TOILE-023`, a measurement; ten verified rows are structural.

---

## 2. WHAT EACH USER SEES

Two peers, one board. The last column joins the example to the test that holds it.

| Alice does | Bob does | The board holds | Each user sees | Test |
|---|---|---|---|---|
| drags a note to the right | drags the same note down at the same moment | both writes, in either arrival order | the note at one of the two places, the same one on both screens | `logic::convergence_tests::two_peers_converge` (proptest) |
| creates a shape | creates a shape at the same moment | two elements, with distinct minted ids | both shapes, on both screens | same proptest, plus `::create_registers_bounds_and_layer` |
| deletes a shape an arrow joins | restyles that same shape | the deletion, and the arrow left without an endpoint | the shape gone on both screens, and the arrow no longer painted | `logic::convergence_tests::delete_drops_owned_connectors` |
| groups three shapes | reorders the paint order | the group binding and the reorder | one paint order on both screens, with the group below its members | `logic::convergence_tests::group_unions_bounds_and_sits_below_members`, and the proptest |
| moves a note far off Bob's screen | -- | the new coordinates | nothing repainted for Bob, because the note meets no viewport of his | `logic::convergence_tests::viewport_culls_offscreen_nodes` |
| points with the highlighter | -- | nothing: a gesture is never written | Bob's screen shows the trail, and the board's history does not | `logic::convergence_tests::apply_ephemeral_keys_presence_by_user_id_without_touching_history` |

Convergence rests on a rule the format owns: an element is named by a minted id, never by a slot
(`ambre-spec-toile-schema.md section 2`, `TOILE-003`). Two peers holding the same operations hold the
same board.

**What this table cannot see.** Both proptests derive under a camera wide enough to cull nothing, and both end
by exchanging whole snapshots. Three things survive every row above: a cull that answers wrongly
(`TOILE-206`), and a connector shipped to a screen it never meets (`TOILE-207`). Also an export cursor that
skips or repeats a range (`TOILE-090`, `TOILE-217`).

---

## 3. MODEL

**Containers.** Declared in `ambre-spec-toile-schema.md section 3` and cited from there by name. This
document restates no part of the format.

**Dependencies** the engine pulls into `ambre-toile`:

| Crate | Role | Rationale |
|---|---|---|
| `loro` | the single-board CRDT | `LoroMap` for the elements, `LoroMovableList` for the one paint order, `LoroText` for a frame's notes |
| `rstar` | the element R-tree | the visible set in O(log N + k), and the first pass of a pick |
| `kurbo` | the plane geometry | a rectangle, an affine transform and a path, shared with the render side through `ambre-draw` |
| `postcard` | the style register, the clipboard payload and the presence frame | compact, and the encoding the Logic-to-Sync hop already uses |

**There is no dependency graph.** An element carries its own coordinates and nothing derives from
another element's value, so a write invalidates no one else. That is the whole difference from a
spreadsheet, and it is why a board converges on its raw state alone.

### A. The state

`ToileState` is a value in the Logic thread's registry, `enum DocState { Toile(ToileState) }`,
reached by a match on the document id. It holds **no transport handle**: what must reach another
thread leaves as a return value (`TOILE-089`).

| Group | What it holds | Rebuilt |
|---|---|---|
| the board | the CRDT document, private, named by no public signature (`TOILE-001`) | never: it is the source of truth |
| the index | the R-tree over element bounds (section 3.B) | on a geometry write, one leaf at a time; whole on a remote change |
| the derived caches | the world box per element, and the paint order read out of its list | with the index |
| the session identity | the minting counter, the peer that mints, and the canonical user id the session seeds | never: a minted id lives in the board, and the counter only has to be unique in this session |
| the view mirror | the last viewport the UI reported, which is what the cull is run against | on every viewport command. Never persisted, never synchronized |
| the presence map | each remote peer's latest cursor, stroke, laser and selection, keyed by user id (`features/ambre-spec-toile-presence.md`) | from the frames that arrive; never persisted |
| the playback state | the presentation flag, the active slide, the broadcast toggle and the presenter this viewer follows (`features/ambre-spec-toile-presentation.md`) | per viewer, never shared |
| the local history | the undo manager, opened with the board (`features/ambre-spec-toile-undo.md`) | never: it dies with the session |
| the sync cursor | the last version vector exported to Sync | advanced by a successful export, by an import minus our own counter, and by nothing else (`TOILE-090`, `TOILE-236`) |
| the pending imports | the imports waiting for a media hash before they touch the board (`features/ambre-spec-toile-media.md`) | never: it dies with the session |

**Code:** `crates/ambre-toile/src/logic/state/mod.rs` -- the state and the dispatch;
`state/caches.rs` -- the derived caches; `state/derive.rs` -- the read-only derivation;
`logic/index.rs` -- the R-tree.

### B. The spatial index

A board is a plane, so the question every frame asks is "which elements meet this rectangle". An
R-tree answers it in O(log N + k), where k is the visible count, instead of walking fifty thousand
elements.

```rust
/// One leaf: an element's world box, already expanded to cover its rotation.
pub struct ElementBounds {
    pub id:  ElementId,
    pub min: [f64; 2],
    pub max: [f64; 2],
}

impl SpatialIndex {
    /// O(N) bulk build, at load and after a remote change.
    pub fn build(elems: impl Iterator<Item = ElementBounds>) -> Self;

    /// The elements that meet the camera's world rectangle, unordered: the
    /// caller paint-orders them (`features/ambre-spec-toile-z-order.md`).
    pub fn query_visible<'a>(&'a self, vp: &Viewport) -> impl Iterator<Item = ElementId> + 'a;

    /// The first pass of a pick. The exact test -- rotation, path fill, stroke
    /// width -- is `features/ambre-spec-toile-selection.md`.
    pub fn query_point<'a>(&'a self, p: kurbo::Point) -> impl Iterator<Item = ElementId> + 'a;

    pub fn insert(&mut self, b: ElementBounds);
    pub fn remove(&mut self, id: &ElementId);
    pub fn update(&mut self, b: ElementBounds);
}
```

A rotated element is indexed by the box of its rotated corners, so the tree over-approximates. That
never hides a visible element, which is what `TOILE-005` asks of it, and the exact test refines the
answer. The stored coordinates are the truth; the box is derived, which is `TOILE-004`.

**The two paths that maintain it must agree**, and unlike Trame that equivalence is held. One property test
drives an operation stream through the incremental path and compares the result to a rebuild. It is held over
the population and over one probe that covers the whole populated area. So what is compared is which elements
exist, never which ones a camera keeps (`TOILE-206`). What is not held is the cost: `remove` finds its leaf by
walking every leaf, and `update` is a remove followed by an insert. So the index that exists to make a frame
O(log N) makes a drag step O(N) (`TOILE-091`).

### C. What one command costs

`handle_command` is a match, and every mutating arm takes the same four steps:

1. **mutate** the board, and commit -- one command is one undo item
   (`features/ambre-spec-toile-undo.md`, `TOILE-014`);
2. **update** the derived caches for what the write touched (section 3.B);
3. **derive** the visible set against the last reported viewport, before returning (`TOILE-002`);
4. **return** the outcome: the display events, the delta, the presence frame, the service requests.

| Trigger | What runs |
|:---|:---|
| a viewport command | no mutation and no delta: store the camera, derive, answer with one visible set |
| a mutating command | the four steps above. The UI has already drawn the gesture, so there is no acknowledgement |
| a remote change | import, **rebuild every derived cache**, derive, answer. The diff-driven path is `TOILE-092` |
| a presence sample | encode and hand it to Sync. No display event: the UI draws its own cursor |

### D. The seam

A command goes down, an outcome comes up. Each command and each event is declared by the feature
that owns it, in `ambre-bus::app::toile`, where the code is authoritative.

```rust
/// What a handler returns. The `ambre-logic` dispatcher is the only place that
/// turns any of these four into a message on a queue (`TOILE-089`).
pub struct ToileOutcome {
    pub display:   Vec<ToileEvent>,     // to the UI thread
    pub update:    Option<Vec<u8>>,     // the CRDT delta to broadcast
    pub ephemeral: Option<Vec<u8>>,     // presence, broadcast and never persisted
    pub requests:  Vec<ToileRequest>,   // a media upload or download, lowered by the dispatcher
}

impl ToileState {
    /// One command in, one outcome out. Mutation, cache maintenance and
    /// derivation all finish here (`TOILE-002`).
    pub fn handle_command(&mut self, cmd: ToileCommand) -> ToileOutcome;

    /// A remote delta or snapshot, already decrypted by Sync. Imports, rebuilds
    /// the derived caches and answers with display events alone: a remote change
    /// is never re-broadcast.
    pub fn apply_remote(&mut self, update: &[u8]) -> Vec<ToileEvent>;

    /// A batch of remote deltas -- a history replay on open -- as one import
    /// sweep and one rebuild, so the board repaints once instead of once per
    /// delta.
    pub fn apply_remote_batch(&mut self, updates: &[Vec<u8>]) -> Vec<ToileEvent>;

    /// A remote presence frame. The author is the Sync-resolved user id and
    /// never a self-asserted one (`ambre-spec-membership.md`, `SYNC-014`).
    pub fn apply_ephemeral(&mut self, from_user_id: UserId, payload: &[u8]) -> Vec<ToileEvent>;

    /// A full snapshot, for local persistence, for compaction and for a Sync
    /// request. Its named counterpart, `import_snapshot`, is the name every
    /// engine shares, so a loader can address the two without a per-app match.
    pub fn export_snapshot(&self) -> Vec<u8>;
    pub fn import_snapshot(&mut self, payload: &[u8]) -> Vec<ToileEvent>;
}
```

The dispatcher routes by document id, calls one of the five, then explodes the result. The display events go
to the UI, the delta and the presence frame to Sync, the requests to the service plane (`TOILE-222`). Its
routing is normative in `ambre-spec-message-bus.md`. A session or a connection event never reaches the state,
except that the session seeds the canonical user id (`TOILE-223`).

**An import that fails is reported.** A remote delta the document refuses answers with a distinct
`RemoteImportFailed` error, so the board tells the user it is behind (`TOILE-214`). In a batch a failed
delta is skipped and the batch reports the failure alongside the survivors it merged (`TOILE-215`). The
batch path still has no other test in this crate, so the one repaint it exists for is a claim about its
construction (`TOILE-213`).

**A second constructor pins the identity.** A fresh board draws a random peer, and every minted id carries
that peer, so the same command sequence produces two different boards. `ToileState::with_peer` pins the peer
and drops the commit clock before the scaffolding write. That is what makes a board reproducible for a fixture
generator or a deterministic test (`ambre-spec-fixtures.md section 2`). It is never for a live document. Two
writers sharing a peer mint colliding ids, the one hazard the minting argument rests on avoiding.

**The delta is a cursor, not a drain.** The export reads from the last exported version vector, and the cursor
advances **only** when that export succeeded (`TOILE-090`). A failure keeps the range, and the next flush
offers it again. Advancing regardless -- swallowing the error -- dropped those operations for good, because no
later export reads from behind the cursor.

Three movements, and each is a different rule:

| What happened | What the cursor does |
|---|---|
| a local edit exported | advances past everything exported, and only then (`TOILE-090`) |
| a peer's delta imported | advances past what arrived, **keeping this peer's own counter** (`TOILE-236`) |
| a restore from local storage | does not move (`TOILE-236`) |

The second is what stops a peer from receiving its own work back. Its exception is the one Trame learned the
hard way. A board opens by committing its scaffolding under a fresh random peer id, and the first import lands
before any flush. So absorbing our own counter would make the next local delta depend on operations no peer
holds. An unresolvable dependency is not an error a peer reports; it is an edit that never appears.

The third is where Toile parts from Trame deliberately. A restore reads what this client wrote before it
closed, which may have reached nobody, and a reopen mints a fresh peer id. So the exception above cannot save
it, and absorbing the restored history would drop an offline session for good. Toile leaves the cursor alone
there, and re-offers the restored history on the first flush. The relay already holds most of it and drops
what it knows. Trame absorbs it (`features/ambre-spec-trame-engine.md`), and carries the hazard.

**The registry is flat.** A board that carries another Ambre document adds no dispatch step and no recursion.
The child is an ordinary entry keyed by its own document id. Why that is the shape is
`features/ambre-spec-toile-embeds.md section 5`.

### E. What the UI thread receives

The answer is one culled, paint-ordered snapshot of the board, built for the visible set alone. It carries the
camera it was culled against, and a single back-to-front sequence of elements and connectors. It carries the
reaction tallies of the visible targets, and the comment pins whose anchor resolves in it. It carries no board
data beyond that: the render side never links the CRDT (`TOILE-UI-001`). One snapshot per relevant change is
the whole contract between the two threads.

```rust
pub struct DisplayViewport {
    pub viewport:  Viewport,
    /// Elements and connectors interleaved in one paint order
    /// (`features/ambre-spec-toile-z-order.md`).
    pub nodes:     Vec<DisplayNode>,
    /// Derived on read, never stored
    /// (`features/ambre-spec-toile-reactions.md`).
    pub reactions: BTreeMap<TargetId, ReactionTally>,
    /// Where each pin sits, its message count and its resolved state, and no
    /// body (`features/ambre-spec-toile-comments.md`).
    pub pins:      Vec<CommentPin>,
}

pub enum DisplayNode {
    Element(DisplayElement),
    Connector(DisplayConnector),
}
```

**The cull is the elements' alone.** The derivation resolves elements against the index, and walks the
connectors of the whole board unconditionally. So a camera showing one corner still ships every connector on
the plane (`TOILE-207`).

Two shapes are deliberately absent, and each one is a rule (`TOILE-208`):

| Absent | Because |
|---|---|
| a presence field | a cursor arrives outside the cull cycle and would force a re-derive per cursor move, so it rides its own event (`features/ambre-spec-toile-presence.md`) |
| an acknowledgement event | an edit is optimistic: the UI has drawn the gesture before the command left |

The pins the comment feature needs are a third absence, and that one is a gap rather than a rule
(`features/ambre-spec-toile-comments.md`, `TOILE-026`).

### F. How two writers race

Elements are independent, so a race is rare and its resolution is cheap. The unit each row races for
is the format's, declared in `ambre-spec-toile-schema.md section 3`.

| Two peers at once | What the board keeps |
|---|---|
| move the same element | the last write on its coordinate registers, which is what a whiteboard does |
| restyle the same element | the last write on the one style register, so no half-applied look survives (`features/ambre-spec-toile-styles.md`) |
| create an element each | both, under distinct minted ids |
| add to one frame's membership, or to the paint order | both, ordered by the movable list (`features/ambre-spec-toile-z-order.md`) |
| draw a stroke each | both: a stroke is an insert and shares nothing (`features/ambre-spec-toile-freehand.md`) |
| delete an element one of them is editing | the deletion. The connectors it carried go dangling rather than disappear (`features/ambre-spec-toile-connectors.md`) |

There is no recompute cascade, so a convergence test asserts an identical board and identical derived
caches, never an identical computed value. That is what `TOILE-017` and `TOILE-004` check together,
and it is why the two are one property test.

---

## 4. SURFACE

The user touches no part of this document directly. What it exposes is the answer every other Toile feature
draws from, and the rules that keep the drawing thread honest. It holds view state and the current visible set
and nothing else (`TOILE-UI-001`). Every mutation leaves as a command (`TOILE-UI-099`), carrying the document
it was drawn on (`TOILE-UI-312`) and touching nothing the canvas draws (`TOILE-UI-314`). A full queue drops
that command rather than blocking the frame, and both paths say so: the edit path (`TOILE-UI-100`) and the
camera path (`TOILE-UI-313`). The dropped camera is offered again on the next frame (`BUS-024`).

**Where each event lands.** One arm per event and **no wildcard arm**, so a new event is a compile
error rather than a silent drop (`TOILE-UI-101`).

| Event | Canvas effect |
|---|---|
| the visible set | cache the payload, mark the frame dirty |
| an element was created | select it, and arm the tool that made it (`features/ambre-spec-toile-tools.md`) |
| a clipboard payload | build the transport blob and mark it pending for the host (`features/ambre-spec-toile-clipboard.md`) |
| the find hits | set the highlight set and recenter on the first one (`features/ambre-spec-toile-find.md`) |
| the selection aggregates | refresh the status band (`features/ambre-spec-toile-status-bar.md`) |
| a peer's presence | store it under its user id and repaint the overlay (`features/ambre-spec-toile-presence.md`) |
| a slide change, a mode change | drive the camera pass (`features/ambre-spec-toile-presentation.md`) |
| a reaction change | re-request the visible set, which carries the tallies |
| an import report, an import failure | surface the count and the dropped constructs (`features/ambre-spec-toile-import-export.md`) |
| a dangling connector, the undo availability, the two presenter roles | an explicit no-op on the canvas -- the chrome relay owns them |
| an error | log it, and nothing else: the user is never told (`TOILE-214`) |

---

## 5. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| The whiteboard job, not the vector-fabrication job | competing on vector-fabrication depth | it is another product's metier and another audience's need. Toile is a Miro, not a Figma: its job is a team thinking together and then presenting -- brainstorming, workshops, diagramming, notes turned into a deck. That audience is the privacy-conscious European team, not the professional designer, who chooses a competitor for its ecosystem, its developer handoff and its design tokens, none of which Ambre offers. The rule of thumb is one sentence: a capability that serves a team thinking together is core, and a capability that serves one person fabricating a precise artifact is not. Two capabilities sit on the far side of that line and are built anyway, the constraint solver (`features/ambre-spec-toile-auto-layout.md`) and the boolean combinators (`features/ambre-spec-toile-vector-paths.md`): they stay in the tree, earn no stabilization budget, carry no promise, and are candidates for removal |
| A handler returns an outcome and sends nothing | injecting an effect sink into the state | it is the trait object the closed document-state enum exists to avoid. A value returned by a plain method is testable on the host with no queue, no worker and no browser, and it keeps the state free of a transport handle, leaving the dispatcher as the single place where a board change becomes traffic |
| The derivation finishes inside the command | a deferred pass on a timer | it buys a shorter command and pays with a screen showing a stale plane. Finishing inside, what the user reads is never a board from before their gesture, there is no second scheduler to reason about, and the cull bounds the work to the visible set |
| The UI is handed the visible set, never the board | iterating the whole document per frame | it is the shape a plane makes impossible: fifty thousand elements cannot be walked per frame, and the thread that paints must not hold a second model of the document. The index answers in O(log N + k), so the cost follows the screen |
| The spatial index is a cache, never the truth | treating the index as the source of truth, and writing the document from it | a remote change arrives as operations on the document, so an authoritative index desyncs the moment two peers move the same element. A cache is rebuildable, and the rebuild is the oracle the equivalence property compares against |
| No dependency graph over elements | reusing the recompute machinery of Trame, edge for edge | an element carries its own coordinates and derives from nobody, so there is no cascade to propagate and nothing to invalidate. The graph a spreadsheet needs is cost and complexity for an empty answer |
| The engine holds one board, and nesting is flat | routing a child's command through its parent board | it makes every depth a new case. Flat, a document inside a document is an ordinary entry in the registry, so there is no depth bound, no recursion and no dispatch step to add |

---

## 6. NOT BUILT, AND NOT YET RULED

* **A retry has no schedule** (`TOILE-090`). A failed export keeps its range, and the range is
  offered again on the next flush -- which is the next local edit. A board nobody is editing
  therefore holds its unsent operations until someone touches it, and no timer, no backoff and no
  notice cover the gap. The peers are not diverged, only late, and nothing says how late.
* **An index update walks the index** (`TOILE-091`). Removing a leaf is a linear search, and a geometry write
  is a remove followed by an insert. So a drag at 60 frames a second walks the whole board 60 times a second.
  Answered by the "Engine and render optimization" Story (`ambre-roadmap.md` E6).
* **A remote change rebuilds everything** (`TOILE-092`). One moved note from a collaborator clears
  the bounds map, bulk-loads the index and re-reads the paint order. The same rebuild runs on an
  undo and on a redo. Answered by the same Story.
* **A snapshot request answers nothing** (`TOILE-093`). The command is declared and returns an empty
  outcome; no code sends it, and the dispatcher takes the snapshot from the state directly. It is a
  variant with no behaviour on either side.
* **The differential answer is deferred.** A remote change that touches a few visible elements still pushes
  the whole visible set. The engine could instead send the changed elements and the removals, which the UI
  applies onto the set it holds. The whole push stays as the fallback. The decision is the engine's, keyed on
  the size of the change. Nothing of it is built, and it lands with the
  "Engine and render optimization" Story (`ambre-roadmap.md` E6).
* **A connector is never culled** (`TOILE-207`). Elements are resolved against the index and connectors are
  walked whole. So a board of a thousand arrows pays for all of them at every camera. The one test that names
  the cull filters connectors out of what it asserts on, so the belt cannot see it. Answered by the "Engine
  and render optimization" Story (`ambre-roadmap.md` E6).
* **No budget is measured.** Not one of the Toile rows of `ambre-spec-budgets.md` has
  a benchmark, so every cost above is a claim about a number nothing weighs. Two of those rows are
  already known to be false.
* **The engine has no queue seam, and needs none.** Trame drives its Logic-to-Sync boundary through
  a mock sink. Toile asserts on the returned outcome instead, which is stronger: the value is the
  contract, and a test that reads it cannot miss a send. Three sections of the corpus named the mock
  seam for Toile all the same, and the crate has never used it.
