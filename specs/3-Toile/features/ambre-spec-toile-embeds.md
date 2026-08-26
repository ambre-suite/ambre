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

| ID | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|
| `TOILE-021` | A nested Ambre document MUST be stored as a reference; its content MUST NOT be inlined into this board. | this spec section 3 | `ambre_toile::logic::convergence_tests::doc_embed_reference_survives_the_crdt_round_trip`, `::doc_embed_with_an_unreadable_reference_falls_back_to_a_frame` | verified |
| `TOILE-083` | A reference this board cannot read MUST still paint as a placeholder; its element MUST NOT vanish. | this spec section 3 | `ambre_toile::logic::convergence_tests::doc_embed_with_an_unreadable_reference_falls_back_to_a_frame` | verified |
| `TOILE-084` | A nested document MUST carry its own permission: what a user may do on the board MUST NOT decide what they may do inside it. | this spec section 3; `ambre-spec-architecture.md section 2` | `ambre_logic::dispatch::tests::an_embedded_child_is_gated_by_its_own_role_not_the_boards`, `::owning_the_board_grants_nothing_on_a_document_embedded_in_it` | verified |
| `TOILE-085` | Copying an embed MUST produce a second reference to the same document, never a copy of that document. | this spec section 3 | none. The copy captures whatever scalar fields the element carries, so the reference rides along by construction, and no test names an embed among them -- section 6 | unverified |
| `TOILE-086` | A user MUST be able to place another Ambre document on the board. | this spec section 4 | not built -- lands with the "Embeds on the board" Story (`ambre-roadmap.md` E4) | unimplemented |
| `TOILE-087` | A user MUST be able to place a web page on the board. | this spec section 4 | not built -- lands with the "Embeds on the board" Story (`ambre-roadmap.md` E4) | unimplemented |
| `TOILE-088` | A web page on the board MUST reach neither the board, its keys nor its storage. | README pillar "E2EE"; this spec section 4 | not built -- lands with the "Embeds on the board" Story (`ambre-roadmap.md` E4); nothing renders a web page today, so nothing sandboxes one | unimplemented |
| `TOILE-UI-015` | A pointer event handed to a child MUST arrive in the child's own pixels, through zoom and rotation. | this spec section 3 | `ambre_toile::ui::embed::tests::a_child_pixel_is_a_screen_pixel`, `::transform_survives_a_rotated_embed`, `::child_size_scales_with_zoom`, `::child_origin_is_the_embeds_top_left`, `::screen_aabb_contains_a_rotated_quad` | verified |
| `TOILE-UI-016` | At most one embed MUST hold focus, and focus MUST be dropped when its element leaves the frame. | this spec section 3 | `ambre_toile::ui::embed::tests::only_the_focused_embed_comes_back_focused`, `::focus_is_dropped_when_its_element_leaves_the_frame`, `::escape_blurs_only_when_something_is_focused`, `::a_click_outside_returns_focus_to_the_board`, `::a_drag_from_an_embed_does_not_focus_it` | verified |
| `TOILE-UI-017` | A child's session MUST close only after its embed has stayed out of view for the whole debounce. | this spec section 3 | `ambre_toile::ui::embed::tests::close_waits_the_full_debounce`, `::a_scroll_oscillation_never_thrashes_the_session`, `::returning_before_the_deadline_cancels_the_close` | verified |
| `TOILE-UI-090` | One session MUST serve one nested document, however many embeds point at it. | this spec section 3 | `ambre_toile::ui::embed::tests::a_visible_embed_opens_once_and_only_once`, `::two_embeds_of_the_same_document_share_one_session` | verified |
| `TOILE-UI-091` | A click MUST enter an embed and a drag MUST NOT, so moving an embed and editing it stay two gestures. | this spec section 4 | `ambre_toile::ui::embed::tests::a_click_on_an_embed_focuses_it`, `::a_drag_from_an_embed_does_not_focus_it`, `::a_click_on_the_already_focused_embed_is_not_rearmed`, `::a_drag_leaves_an_existing_focus_untouched` | verified |
| `TOILE-UI-092` | A locked embed MUST NOT take focus. | this spec section 4 | `ambre_toile::ui::embed::tests::embed_at_picks_the_topmost_and_skips_locked` | verified |
| `TOILE-UI-093` | `Escape` MUST return focus to the board rather than reach the child. | this spec section 4 | `ambre_toile::ui::embed::tests::escape_blurs_only_when_something_is_focused`, plus type-level: the capture arm names `Escape` as its one exclusion, so no branch can forward it | verified |
| `TOILE-UI-094` | Closing the board MUST close every child at once, with no debounce. | this spec section 3 | `ambre_toile::ui::embed::tests::close_all_takes_every_child_immediately` | verified |
| `TOILE-UI-095` | The board MUST NOT name the child's view type. | `ambre-spec-architecture.md section 4` | type-level: `ambre-toile` does not depend on `ambre-app`, so no signature in the crate can name `AppView` | verified |
| `TOILE-UI-096` | A child MUST be painted in its embed's own frame: a rotated embed MUST paint its child rotated. | this spec section 3 | none, and **the rule fails today**: the paint shifts to the bounding box corner and applies no rotation, while the pointer maps through it -- section 6 | unverified |
| `TOILE-UI-097` | Each embed MUST paint its own child: two embeds of one document MUST NOT share one view. | this spec section 3 | none, and **the rule fails today**: the child views are keyed by document, so two embeds share one and resize it against each other -- section 6 | unverified |
| `TOILE-UI-098` | A child's events MUST NOT reach the parent's view or the board's chrome. | this spec section 3 | none, and **the rule fails today**: the child's route dies with its view while its session outlives it by the debounce -- section 6 | unverified |

* **Budget.** none of its own, and that is the open question rather than the answer: a child is a
  whole document view painting inside the board's own frame, so an embed spends the board's 16 ms
  and no figure bounds how many embeds a board may carry. That figure belongs in
  `ambre-spec-budgets.md` and is not there yet.
* **Boundary.** the board stores a reference and no content, so the relay sees no embedding table
  (`TOILE-021`). A child is opened through the ordinary document path, with its own key, its own
  session and its own permission (`TOILE-084`), so nesting adds no flow the sync layer does not
  already seal. The web page is the exception, and `TOILE-088` is the rule that would hold it.
* **Verification.** 12 `verified`, 4 `unverified`, 3 `unimplemented`. What is held is everything the
  board can decide alone: the reference, the transform, the focus arbitration and the session
  lifecycle, on fifteen host tests. What is loose is everything the launcher does with the answer --
  the paint, the input route and the event fan-out have no test anywhere, because `ambre-app` runs
  on the wasm tier and its suite covers two helpers.

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
| a click on an embed | it takes focus, and the click itself does not reach the child (`TOILE-UI-091`) |
| a drag from an embed | the board moves it; focus is untouched (`TOILE-UI-091`) |
| a click on an embed that is locked | nothing: a locked embed is not enterable (`TOILE-UI-092`) |
| a pointer event inside the focused embed | the child's, remapped into its pixels (`TOILE-UI-015`) |
| a key, a character, a composition step, a wheel, while an embed is focused | the child's, verbatim |
| `Escape` | focus returns to the board (`TOILE-UI-093`) |
| a click outside the focused embed | focus returns to the board |
| leaving the board | every child session closes at once (`TOILE-UI-094`) |

| Trigger, once it exists | What must happen |
|---|---|
| a menu entry offering a document to embed | the board mints a `doc_embed` at the drop point holding that document's reference |
| a menu entry taking a web address | the board mints an `embed` holding the address |
| the web embed painting | the page renders where the board's placeholder is, sealed off from the board (`TOILE-088`) |

---

## 5. DECISIONS

| Decision | Rationale | Rejected alternative |
|---|---|---|
| The board keeps a reference, never the content | the child has its own key and its own membership, so inlining it would hand a reader of the board a document they were never granted, and would give the relay an embedding table to read | copying the child's content into the board, one key for two documents |
| The launcher paints the child, not the board | the closed set of application views lives downstream of every application crate, so a board painting a child would name a type above itself, which is the dependency cycle the launcher exists to break | a view trait the board could hold, which re-introduces the cycle through an object instead of a name |
| The raw event crosses into the child, not a command | building the child's command needs the child view's own state, which the board may not name; the child is the only layer that can translate its own input | an envelope command wrapping the child's, which cannot be built on the board's side and routes around nothing, since the registry is flat |
| The registry is flat and a child is an ordinary entry | one lookup reaches the child, its outcome fans out normally, and its permission is resolved per document rather than inherited | nesting the registry to mirror the board, which makes every lookup recursive and every permission a question about a parent |
| Focus is view-local, never document state | two users looking at one board are in different documents at the same moment, and a shared focus would drag one into the other's embed | a focus field in the document, which converges to one focus for everybody |
| Entering an embed is a click, moving it is a drag | both gestures start with a press on the same box, so only the release can tell them apart; without the split, dragging an embed would type into it | focusing on press, which makes an embed unmovable |
| A session survives a pan for the debounce | visibility flips on every pan, and re-opening costs a membership fetch, a key unwrap and a snapshot, so binding the session to the cull set would storm the socket on a scroll oscillation | closing the instant the embed leaves the screen |
| A rotated embed clips to its bounding box | the clip is rectangular and resolved against the transform at push time, and rotating a nested document is a degenerate gesture | withholding rotation from an embed, which makes it the one element with a missing handle |

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
* **The launcher half has no test.** Fifteen host tests cover what the board decides. The child
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
* **A copied embed is a second reference, and no test says so** (`TOILE-085`). The copy captures the
  element's scalar fields whatever they are, so the reference survives by accident of that
  generality rather than by a rule anything checks.
