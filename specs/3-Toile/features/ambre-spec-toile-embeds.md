# AMBRE -- FEATURE: EMBEDS

**Type:** Vertical (feature)
**Implementation:** partial -- the nesting plane is built end to end; nothing puts an embed on the
board, and a web page has no renderer.

> A board holds more than shapes: a web page, and another Ambre document a user edits in place
> without leaving the board.

This is the only Toile feature that reaches out of the application crate. The board may not name the
child's view type, so it publishes its embeds and the launcher paints them. That inversion is the
whole design, and everything below follows from it.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|---|
| `TOILE-021` | INV | A nested Ambre document MUST be stored as a reference; its content MUST NOT be inlined into this board. | this spec section 3 | `ambre_toile::logic::convergence_tests::doc_embed_reference_survives_the_crdt_round_trip` | verified |
| `TOILE-084` | INV | A nested document MUST carry its own permission: what a user may do on the board MUST NOT decide what they may do inside it. | this spec section 3; `ambre-spec-architecture.md section 2` | `ambre_logic::dispatch::tests::an_embedded_child_is_gated_by_its_own_role_not_the_boards`, `::owning_the_board_grants_nothing_on_a_document_embedded_in_it` | verified |
| `TOILE-088` | INV | A web page on the board MUST reach neither the board, its keys nor its storage. | README pillar "E2EE"; this spec section 4 | not built -- lands with the "Embeds on the board" Story (`ambre-roadmap.md` E4); nothing renders a web page today, so nothing sandboxes one | unimplemented |
| `TOILE-224` | INV | Which embed is entered MUST be this user's own view state: a peer entering an embed MUST NOT change what this user has focused. | this spec section 5 | `ambre_toile::logic::convergence_tests::doc_embed_reference_survives_the_crdt_round_trip` asserts the derived focus is false whatever the document holds, so no field carries it | verified |
| `TOILE-UI-016` | INV | At most one embed MUST hold focus. | this spec section 3 | `ambre_toile::ui::embed::tests::only_the_focused_embed_comes_back_focused`; the focus is one `Option<ElementId>`, so a second is unrepresentable | verified |
| `TOILE-UI-090` | INV | One session MUST serve one nested document, however many embeds point at it. | this spec section 3 | `ambre_toile::ui::embed::tests::a_visible_embed_opens_once_and_only_once`, `::two_embeds_of_the_same_document_share_one_session` | verified |
| `TOILE-UI-095` | INV | The board MUST NOT name the child's view type. | `ambre-spec-architecture.md section 4` | type-level: `ambre-toile` does not depend on `ambre-app`, so no signature in the crate can name `AppView` | verified |
| `TOILE-UI-098` | INV | A child's events MUST NOT reach the parent's view or the board's chrome. | this spec section 3 | none, and **the rule fails today**: the child's route dies with its view while its session outlives it by the debounce -- section 6 | unverified |
| `TOILE-083` | OBS | A reference this board cannot read MUST still paint as a placeholder rather than vanish from the board. | this spec section 3 | `ambre_toile::logic::convergence_tests::doc_embed_with_an_unreadable_reference_falls_back_to_a_frame` | verified |
| `TOILE-UI-096` | OBS | A rotated embed MUST paint its child rotated. | this spec section 3 | none, and **the rule fails today**: the paint shifts to the bounding box corner and applies no rotation, while the pointer maps through it -- section 6 | unverified |
| `TOILE-UI-097` | OBS | Each embed MUST paint its own child: two embeds of one document MUST NOT share one view. | this spec section 3 | none, and **the rule fails today**: the child views are keyed by document, so two embeds share one and resize it against each other -- section 6 | unverified |
| `TOILE-UI-320` | OBS | A child MUST paint its own overlay: its selection, its caret and its handles MUST be visible inside the embed. | this spec section 3 | none, and **the rule fails today**: the launcher paints each child's content and never its overlay, so an embedded document shows no selection at all -- section 6 | unverified |

### 1.2 Put a document or a page on the board

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|---|
| `TOILE-086` | POST | A user MUST be able to place another Ambre document on the board. | this spec section 4 | not built -- lands with the "Embeds on the board" Story (`ambre-roadmap.md` E4) | unimplemented |
| `TOILE-087` | POST | A user MUST be able to place a web page on the board. | this spec section 4 | not built -- lands with the "Embeds on the board" Story (`ambre-roadmap.md` E4) | unimplemented |
| `TOILE-225` | FRAME | Placing an embed MUST NOT touch the document it points at. | this spec section 3 | not built -- lands with the "Embeds on the board" Story (`ambre-roadmap.md` E4) | unimplemented |

### 1.3 Click into an embed

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|---|
| `TOILE-UI-091` | POST | A click on an embed MUST enter it, so that moving an embed and editing it stay two gestures. | this spec section 4 | `ambre_toile::ui::embed::tests::a_click_on_an_embed_focuses_it` | verified |
| `TOILE-UI-321` | POST | The click MUST enter the topmost embed under the pointer. | this spec section 4 | `ambre_toile::ui::embed::tests::embed_at_picks_the_topmost_and_skips_locked` | verified |
| `TOILE-UI-322` | FRAME | The click that enters MUST NOT reach the child. | this spec section 4 | structural, and the construction is the proof: focus is granted by the release, so both events of the entering click are dispatched while the board still holds focus and the capture arm is not reached | verified |
| `TOILE-UI-323` | FRAME | A click on the embed that already holds focus MUST NOT return focus to the board. | this spec section 4 | `ambre_toile::ui::embed::tests::a_click_on_the_already_focused_embed_is_not_rearmed` | verified |
| `TOILE-UI-092` | FAIL | A locked embed MUST refuse focus, and MUST NOT shadow a live embed beneath it. | this spec section 4 | `ambre_toile::ui::embed::tests::embed_at_picks_the_topmost_and_skips_locked` | verified |

### 1.4 Drag an embed across the board

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|---|
| `TOILE-UI-324` | POST | A press released past the click slop MUST move the embed on the board and MUST NOT enter it. | this spec section 4 | `ambre_toile::ui::embed::tests::a_drag_from_an_embed_does_not_focus_it` | verified |
| `TOILE-UI-325` | FRAME | A drag starting on another embed MUST leave the current focus where it is. | this spec section 4 | `ambre_toile::ui::embed::tests::a_drag_leaves_an_existing_focus_untouched` | verified |

### 1.5 Point and type inside an entered embed

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|---|
| `TOILE-UI-015` | POST | A pointer event handed to a child MUST arrive in the child's own pixels, through zoom and rotation. | this spec section 3 | `ambre_toile::ui::embed::tests::a_child_pixel_is_a_screen_pixel`, `::transform_survives_a_rotated_embed`, `::child_size_scales_with_zoom`, `::child_origin_is_the_embeds_top_left`, `::screen_aabb_contains_a_rotated_quad` | verified |
| `TOILE-UI-326` | POST | A key, a character, a composition step and a wheel MUST reach the child verbatim. | this spec section 4 | none. The board's capture arm forwards each kind, but what the launcher does with it before the child sees it has no test anywhere -- section 6 | unverified |
| `TOILE-UI-327` | FRAME | An event handed to the child MUST NOT also run the board's own handlers. | this spec section 3 | structural, and the construction is the proof: the capture arm returns before the dispatch match, so no board handler can be reached on a captured event | verified |
| `TOILE-UI-328` | FAIL | A pointer event whose position cannot be mapped into the child MUST be dropped, not handed over unmapped. | this spec section 3 | none, and **the rule fails today**: the remap falls back to the screen position, so the child receives a coordinate from the board's space -- section 6 | unverified |
| `TOILE-UI-329` | FAIL | A wheel outside the entered embed MUST pan the board rather than scroll the child. | this spec section 4 | none, and **the rule fails today**: the capture arm bounds pointer events to the embed's quad and captures every wheel unconditionally -- section 6 | unverified |

### 1.6 Leave an embed

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|---|
| `TOILE-UI-093` | POST | `Escape` MUST return focus to the board. | this spec section 4 | `ambre_toile::ui::embed::tests::escape_blurs_only_when_something_is_focused` | verified |
| `TOILE-UI-330` | POST | A click outside the entered embed MUST return focus to the board. | this spec section 4 | `ambre_toile::ui::embed::tests::a_click_outside_returns_focus_to_the_board` | verified |
| `TOILE-UI-331` | FRAME | `Escape` MUST NOT reach the child. | this spec section 4 | type-level: the capture arm names `Escape` as its one exclusion, so no branch can forward it | verified |
| `TOILE-UI-332` | FRAME | Returning focus to the board MUST NOT close the child's session. | this spec section 3 | structural, and the construction is the proof: focus and sessions are two registries, and the blur path names neither the open set nor the frame clock | verified |

### 1.7 Draw one frame of a board carrying embeds

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|---|
| `TOILE-UI-333` | POST | An embed entering the culled set MUST open its document's session. | this spec section 3 | `ambre_toile::ui::embed::tests::a_visible_embed_opens_once_and_only_once` | verified |
| `TOILE-UI-017` | POST | A child's session MUST close only after its embed has stayed out of view for the whole debounce. | this spec section 3 | `ambre_toile::ui::embed::tests::close_waits_the_full_debounce`, `::a_scroll_oscillation_never_thrashes_the_session`, `::returning_before_the_deadline_cancels_the_close` | verified |
| `TOILE-UI-334` | POST | Focus MUST be dropped when the element that held it leaves the frame. | this spec section 3 | `ambre_toile::ui::embed::tests::focus_is_dropped_when_its_element_leaves_the_frame` | verified |
| `TOILE-UI-335` | POST | A child whose own content changed MUST repaint the board it sits on. | this spec section 3 | none. The launcher pumps each child and raises the board's repaint when a child plans one, and `ambre-app` has no test for any of it -- section 6 | unverified |
| `TOILE-UI-336` | FRAME | A frame where no embed entered and none left MUST send no session command. | this spec section 3 | `ambre_toile::ui::embed::tests::a_visible_embed_opens_once_and_only_once`, `::a_scroll_oscillation_never_thrashes_the_session` | verified |
| `TOILE-UI-337` | FRAME | The published embeds MUST be those of the current culled set and no others. | this spec section 3 | `ambre_toile::ui::embed::tests::a_board_with_no_embeds_publishes_nothing`, `::only_the_focused_embed_comes_back_focused` | verified |
| `TOILE-UI-338` | FAIL | An open the board could not send MUST be sent on a later frame. | this spec section 3 | none, and **the rule fails today**: the session is recorded open before the command is queued, so a full channel loses it for good and the embed stays a placeholder -- section 6 | unverified |
| `TOILE-UI-339` | FAIL | A close the board could not send MUST be sent on a later frame. | this spec section 3 | none, and **the rule fails today**: the session is forgotten before the command is queued, so a full channel leaks the child until the page is left -- section 6 | unverified |
| `TOILE-UI-340` | FAIL | An event for a child that its channel cannot take MUST NOT be dropped in silence. | `ambre-spec-message-bus.md section 5`; this spec section 3 | none, and **the rule fails today**: the relay diverts the event and ignores a full child channel, so the child misses a delta the board already applied -- section 6 | unverified |

### 1.8 Leave the board

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|---|
| `TOILE-UI-094` | POST | Closing the board MUST close every child at once, with no debounce. | this spec section 3 | `ambre_toile::ui::embed::tests::close_all_takes_every_child_immediately` | verified |
| `TOILE-UI-341` | FRAME | A second close MUST send nothing. | this spec section 3 | `ambre_toile::ui::embed::tests::close_all_takes_every_child_immediately` | verified |

### 1.9 Copy an embed

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|---|
| `TOILE-085` | POST | Copying an embed MUST produce a second reference to the same document. | this spec section 3 | none. The copy captures whatever scalar fields the element carries, so the reference rides along by construction, and no test names an embed among them -- section 6 | unverified |
| `TOILE-226` | FRAME | Copying an embed MUST NOT copy the document it points at. | this spec section 3 | none, for the same reason as `TOILE-085`: nothing exercises a copy of an embed at all -- section 6 | unverified |

* **Budget.** none of its own, and that is the open question rather than the answer: a child is a
  whole document view painting inside the board's own frame, so an embed spends the board's 16 ms
  and no figure bounds how many embeds a board may carry. That figure belongs in
  `ambre-spec-budgets.md` and is not there yet.
* **Boundary.** the board stores a reference and no content, so the relay sees no embedding table
  (`TOILE-021`). A child is opened through the ordinary document path, with its own key, its own
  session and its own permission (`TOILE-084`), so nesting adds no flow the sync layer does not
  already seal. The web page is the exception, and `TOILE-088` is the rule that would hold it.
* **Verification.** 27 `verified`, 13 `unverified`, 4 `unimplemented`. What is held is everything
  the board can decide alone: the reference, the transform, the focus arbitration and the session
  lifecycle, on twenty-one host tests, every one of them cited. What is loose is everything the
  launcher does with the answer -- the paint, the input route and the event fan-out have no test
  anywhere, because `ambre-app` runs on the wasm tier and its suite covers two helpers. Five of the
  thirteen loose rules are known to fail today, and three of those five were found by writing the
  frame conditions down: a session command the board drops is never re-sent (`TOILE-UI-338`,
  `TOILE-UI-339`), and an event a child's channel refuses is dropped in silence (`TOILE-UI-340`).

---

## 2. WHAT EACH USER SEES

Two documents, two convergences. The board converges on where the embed sits; the child converges on
its own content, with the peers it has rather than the ones the board has.

| Alice does | Bob does | The document holds | Each user sees | Test |
|---|---|---|---|---|
| moves the embed | types inside the embedded document | the board holds the reference and the geometry; the words are in the child (`TOILE-021`) | the box moves for both, the words appear for both | `logic::convergence_tests::doc_embed_reference_survives_the_crdt_round_trip` |
| deletes the embed | has the same document open on its own | the board loses the reference; the child document is untouched | the box leaves the board, and Bob keeps editing | none -- the deletion is the ordinary element delete (`TOILE-034`) |
| copies the embed and pastes it | watches | two references to one document (`TOILE-085`) | two boxes showing one document: an edit in either appears in both | none (`TOILE-085`) |
| may read the board and not the child | may edit the child and not the board | each document's own membership (`TOILE-084`) | Alice cannot type in the embed; Bob cannot move it | `ambre_logic::dispatch::tests::an_embedded_child_is_gated_by_its_own_role_not_the_boards` |
| pans until the embed leaves the screen | edits the child meanwhile | the child's own edits, converged as always | Alice's board keeps the session alive for the debounce, then closes it (`TOILE-UI-017`) | `ui::embed::tests::close_waits_the_full_debounce` |

**What this table cannot see.** Every row above is one user's board and one user's session. The
rules that break are the ones between the two: a child that misses a delta its board already
applied (`TOILE-UI-340`) shows Bob's edit to Bob and not to Alice, and no row of a convergence
table reaches it, because the divergence is between two views of one client rather than between two
peers.

---

## 3. MODEL

**Containers.** This feature owns none. The reference is two scalar fields of the element map --
`doc_id` and `doc_type` for a nested document, `url` for a web page -- declared in
`ambre-spec-toile-schema.md section 3`. Geometry, lock and paint order are the element's own
(`features/ambre-spec-toile-elements.md`, `features/ambre-spec-toile-z-order.md`), so move, resize,
rotate, delete and reorder cost this feature nothing.

**What the board publishes** (`ambre-toile`):

```rust
/// One `doc_embed` of the current culled frame, projected for the launcher.
pub struct VisibleEmbed {
    /// The board element carrying the reference: the focus and hit-test key.
    pub element: ElementId,
    /// The embedded document: the key into the child views, and the id the
    /// child's own commands are addressed to.
    pub document_id: DocumentId,
    pub doc_type: DocType,
    /// Where to paint, in physical screen pixels: the axis-aligned box of the
    /// element's quad.
    pub screen_rect: Rect,
    /// The child's own canvas size in physical pixels.
    pub child_size: Size,
    /// View-local focus. Never CRDT: a peer's focus is not ours.
    pub focused: bool,
}

/// How long an embed stays out of the culled set before its child's session is
/// closed. A re-entry cancels the pending close.
pub const EMBED_CLOSE_DEBOUNCE_MS: f64 = 5_000.0;

/// How far the pointer may travel between press and release and still count as
/// a click that enters the embed rather than a drag that moves it.
pub const CLICK_SLOP: f64 = 4.0;
```

**What the launcher supplies** (`ambre-app`):

```rust
/// Builds a child view for a nested document. Only the launcher can: it alone
/// may name every application's view type. Given the document, its type and the
/// child canvas size, it wires a private event channel and returns the view.
pub type MakeChild = Rc<dyn Fn(DocumentId, DocType, Size) -> AppView>;
```

**Steps, once per frame:**

1. the board reconciles the culled set against its open sessions: an embed that just appeared asks
   for its document to be opened, one that has been gone past the debounce asks for it to be closed,
   and one document answers however many embeds point at it (`TOILE-UI-090`, `TOILE-UI-017`);
2. focus is dropped if the element that held it is no longer in the frame (`TOILE-UI-016`);
3. the board publishes its visible embeds and paints each one's placeholder box, and nothing else
   (`TOILE-UI-095`);
4. the launcher builds a child view for each new one, resizes the ones whose box changed, drops the
   ones that left, and paints each child clipped to its rect over the board's own pass;
5. an input event aimed at the focused embed is remapped into the child's pixels and handed to the
   child's own view, which builds its own commands against its own document (`TOILE-UI-015`).

**Why the raw event crosses, and not a command.** The board cannot build the child's command: turning
a pointer press into a spreadsheet edit needs the child view's own selection and edit state, and the
board may not name that view. So what crosses is the event; the child translates it and addresses
the result itself.

**Why the events come back on a private channel.** A view ignores the document id on the events it
drains: it assumes every event on its channel is its own. That holds while one view reads one
channel, so a child cannot share the board's. Each child registers its own channel, and the relay
routes every document-tagged event whose id is a live child there instead (`TOILE-UI-098`).

**Code:**

* `crates/ambre-toile/src/ui/embed.rs` -- the projection, the pointer transform, the focus
  arbitration and the session debounce;
* `crates/ambre-toile/src/ui/view.rs` -- the capture arm that hands an event to the child, and the
  per-frame lifecycle;
* `crates/ambre-app/src/host/frame.rs` -- the child views, their paint and the input remap;
* `crates/ambre-app/src/lib.rs` -- the child factory and the event fan-out;
* `crates/ambre-logic/src/dispatch.rs` -- the flat registry a child is an ordinary entry of.

---

## 4. SURFACE

**Nothing places an embed** (`TOILE-086`, `TOILE-087`). No tool, no menu entry and no chord builds
either kind: the two constructors exist and their only callers are tests. The surface below is what
an embed already on the board answers to.

| Trigger | What happens |
|---|---|
| a click on an embed | the topmost one under the pointer takes focus, and the click itself does not reach the child (`TOILE-UI-091`, `TOILE-UI-321`, `TOILE-UI-322`) |
| a drag from an embed | the board moves it; focus is untouched (`TOILE-UI-324`, `TOILE-UI-325`) |
| a click on an embed that is locked | nothing: a locked embed is not enterable, and it does not shadow a live embed beneath it (`TOILE-UI-092`) |
| a pointer event inside the focused embed | the child's, remapped into its pixels (`TOILE-UI-015`) |
| a key, a character, a composition step, a wheel, while an embed is focused | the child's, verbatim (`TOILE-UI-326`) -- and the wheel goes to the child wherever the pointer is (`TOILE-UI-329`) |
| `Escape` | focus returns to the board (`TOILE-UI-093`) |
| a click outside the focused embed | focus returns to the board (`TOILE-UI-330`) |
| leaving the board | every child session closes at once (`TOILE-UI-094`) |

| Trigger, once it exists | What must happen |
|---|---|
| a menu entry offering a document to embed | the board mints a `doc_embed` at the drop point holding that document's reference |
| a menu entry taking a web address | the board mints an `embed` holding the address |
| the web embed painting | the page renders where the board's placeholder is, sealed off from the board (`TOILE-088`) |

---

## 5. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| The board keeps a reference, never the content | copying the child's content into the board, one key for two documents | the child has its own key and its own membership, so inlining it hands a reader of the board a document they were never granted, and gives the relay an embedding table to read |
| The launcher paints the child, not the board | a view trait the board could hold | the closed set of application views lives downstream of every application crate, so a board painting a child names a type above itself. A trait re-introduces that cycle through an object instead of a name, and the cycle is what the launcher exists to break |
| The raw event crosses into the child, not a command | an envelope command wrapping the child's | building the child's command needs the child view's own state, which the board may not name, so the envelope cannot be built on the board's side. It routes around nothing either, since the registry is flat |
| The registry is flat and a child is an ordinary entry | nesting the registry to mirror the board | it makes every lookup recursive and every permission a question about a parent. Flat, one lookup reaches the child, its outcome fans out normally, and its permission is resolved per document rather than inherited |
| Focus is view-local, never document state | a focus field in the document | it converges to one focus for everybody. Two users looking at one board are in different documents at the same moment, so a shared focus drags one into the other's embed |
| Entering an embed is a click, moving it is a drag | focusing on the press | both gestures start with a press on the same box, so only the release can tell them apart. Focusing on the press makes an embed unmovable, and without the split a drag types into it |
| A session survives a pan for the debounce | closing the instant the embed leaves the screen | visibility flips on every pan, and re-opening costs a membership fetch, a key unwrap and a snapshot, so binding the session to the cull set storms the socket on a scroll oscillation |
| A rotated embed clips to its bounding box | withholding rotation from an embed | it makes an embed the one element with a missing handle. The clip is rectangular and resolved against the transform at push time, and rotating a nested document is a degenerate gesture |

---

## 6. NOT BUILT, AND NOT YET RULED

* **Nothing places an embed** (`TOILE-086`, `TOILE-087`). The element kind, the constructor, the
  round trip, the focus, the session and the paint all exist; the gesture that starts the whole
  chain does not. Fourth area in a row where the engine ships whole and the entry point is missing,
  after the deck (`TOILE-FEAT-026`) and the import (`TOILE-IO-020`).
* **A web page has no renderer, so it has no sandbox either** (`TOILE-088`). The corpus calls it a
  sandboxed web embed. There is no frame, no sandbox attribute and no fetch anywhere in the
  workspace: the element paints the same placeholder box as a nested document, and its address is
  stored exactly as it was given, unchecked. The rule the word "sandboxed" stands for has never been
  written down, and it is the one flow of this feature that leaves the client.
* **A rotated embed paints its child upright** (`TOILE-UI-096`). The pointer transform undoes the
  rotation and is held by five tests. The paint shifts to the corner of the bounding box and applies
  no rotation at all, so a rotated embed shows its child straight, in the wrong corner, while the
  pointer arrives as though it were turned. One half of the mapping is proved and the other
  contradicts it.
* **Two embeds of one document share one view** (`TOILE-UI-097`). The child views are keyed by the
  document, which is right for the session and wrong for the paint: two boxes of different sizes
  resize the one view against each other on every frame, and only one of the two boxes is ever the
  right size for it.
* **The child's events reach the board for the length of the debounce** (`TOILE-UI-098`). The child
  view is dropped the frame its embed leaves the culled set, and its private channel goes with it,
  while its session stays open for five more seconds. An event arriving in that window finds no
  route and falls through to the board's own stream and to the board's chrome mirrors -- and a Toile
  nested in a Toile emits exactly the event the parent board applies to itself.
* **A session command the board cannot send is lost** (`TOILE-UI-338`, `TOILE-UI-339`). The open
  set is written before the command is queued: an open marks the document open and then pushes
  `OpenDocument`, a close forgets it and then pushes `CloseDocument`. When the channel to Logic is
  full the command is warned about and dropped, and the set already says the work is done -- so no
  later frame retries. A lost open leaves that embed a blank placeholder until the board is left; a
  lost close leaves the child's session open with nothing left that names it, which `close_all`
  cannot reclaim either. The comment at the drop site claims the next frame retries the open and
  that the board's close reclaims the leak; neither is true of the code beneath it.
* **An event a child's channel refuses is dropped in silence** (`TOILE-UI-340`). The relay diverts
  every event tagged with a live child's document to that child's own channel, and handles only a
  disconnected one. A full channel falls through the `if` and then past the `continue`: the event
  reaches neither the child nor the board. It is a CRDT delta, so the child's view is behind its own
  document until something else redraws it from scratch.
* **A wheel is captured wherever the pointer is** (`TOILE-UI-329`). Pointer events are routed to the
  child only while the cursor is inside its quad, which is what lets a click outside return focus.
  The wheel arm has no such test: while any embed holds focus, every wheel on the board scrolls that
  child, so the board cannot be panned or zoomed until the user clicks out. The asymmetry is not a
  decision anybody wrote down.
* **A child never paints its overlay** (`TOILE-UI-320`). The launcher paints each child's content
  into the embed's rect and stops there; `paint_overlay` is called for the top-level view only. An
  embedded spreadsheet shows its cells and never its selection, its caret or its handles, so a user
  typing into it cannot see where they are typing.
* **The launcher half has no test.** Twenty-one host tests cover what the board decides. The child
  reconciliation, the paint, the clip, the input remap and the event fan-out have none: `ambre-app`
  compiles for wasm only and its suite covers a base64 codec and a frame counter. The half that was
  hard is the half that is unverified.
* **A focused embed cannot be moved.** While an embed holds focus, every pointer event inside it
  belongs to the child, so the board's own move and resize are unreachable until the user clicks
  outside. No rule says whether that is right; the transform handles stay painted, and they do not
  answer.
* **A child writes into chrome nobody reads.** The formula bar, the zoom chip, the status band and
  the floating bar are surfaces of the top-level document, so a child is built against a throwaway
  set of them. A user editing an embedded spreadsheet gets no formula bar, and nothing says what
  they should get instead.
* **A copied embed is a second reference, and no test says so** (`TOILE-085`, `TOILE-226`). The copy
  captures the element's scalar fields whatever they are, so the reference survives by accident of
  that generality rather than by a rule anything checks -- and nothing says the child document
  itself is left alone either.
