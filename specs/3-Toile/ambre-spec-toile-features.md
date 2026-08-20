# AMBRE -- SPEC: TOILE FEATURES (Presentation, Auto-Layout, Collaboration)
**Type:** Vertical (Feature / Application -- Features)
**Status:** Draft
**Implementation:** partial

> Higher-level Toile capabilities layered on the engine (`toile-logic`) and renderer (`toile-ui`):
> presentation mode (frame sequencing, Smart Animate transitions, focus mode, presenter overlay), the
> auto-layout constraint solver, vector boolean operations, and the real-time collaboration primitives that
> *do* touch the CRDT (votes and reactions) versus those that stay ephemeral (highlighters/laser, live
> cursors -- their render is in `toile-ui section 8`).
>
> **Scope of this spec:** the feature logic and its CRDT/ephemeral placement. It reuses `ToileState`'s command
> plane (`toile-logic section 8`) and `DisplayViewport` (`section 9`); it does **not** redefine the element model, spatial
> index, rendering pipeline, or import/export (that is `toile-interop`).

---

## 0. EXECUTION MODEL -- COMMANDS, EVENTS, OUTCOME

Every feature here is expressed as additional `ToileCommand` variants handled synchronously in
`ToileState::handle_command`, returning a `ToileOutcome` (`toile-logic section 8`). Presentation *playback* state and
selection are **local/ephemeral**; the presentation *sequence* (which frames, in what order) and votes/reactions
are **CRDT** (shared, persisted). This split is the through-line of the spec: persist shared intent, keep
per-viewer playback local.

### Invariants

Prefix `TOILE-FEAT`. Grammar, falsifiability test and status values are normative in `ambre-spec-architecture.md section 4`; the cross-spec roll-up is `ambre-spec-testing.md section 7`. Rows `001..010` cover the built features: presentation (section 1), auto-layout (section 2), boolean ops (section 3), reactions and speaker notes (section 4.A-C). Rows `011..017` cover comments (section 4.D), whose CRDT does not exist -- `CreateComment` appears nowhere in the workspace -- so they carry `Status: unimplemented` and cite the Story that will build them (architecture section 4). Undo granularity is the engine's (`TOILE-014`).

| ID | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|
| `TOILE-FEAT-001` | Shared intent (slide sequence, notes, auto-layout spec, reactions) MUST be CRDT; per-viewer playback MUST stay local and MUST NOT be synced. | section 0 | `ambre_toile::logic::convergence_tests::present_mode_toggle_is_local`, `::playback_is_not_synced`, `::follow_state_is_not_synced`, `::present_broadcast_toggle_is_local`, `::set_slide_order_produces_delta_and_is_undoable` | verified |
| `TOILE-FEAT-002` | The slide deck MUST deduplicate repeated frames, and playback MUST be clamped to the deck bounds. | section 1.A | `ambre_toile::logic::convergence_tests::slide_order_dedups_repeated_frames`, `::playback_bounds`, `::adopted_slide_is_clamped_to_deck` | verified |
| `TOILE-FEAT-003` | A follower MUST adopt the presenter's active slide; a non-follower MUST ignore the broadcast; manual navigation MUST break the follow. | section 1.B | `ambre_toile::logic::convergence_tests::follower_adopts_presenter_slide`, `::non_follower_ignores_broadcast`, `::following_other_peer_ignores_broadcast`, `::manual_nav_breaks_follow`, `::follow_snaps_to_last_known_slide`, `::broadcast_frame_carries_active_slide`, `::no_broadcast_frame_when_not_broadcasting` | verified |
| `TOILE-FEAT-004` | `smart_animate` MUST tween elements sharing an id, fade in the new ones and fade out the gone ones, and MUST reproduce the source and target frames exactly at `t=0` and `t=1`. | section 1.C | `ambre_toile::logic::animate::tests::shared_id_tweens_at_midpoint`, `::new_element_fades_in`, `::gone_element_fades_out`, `::endpoints_reproduce_source_and_target`, `::mixed_membership_classifies_each_id`, `::empty_frames` | verified |
| `TOILE-FEAT-005` | `solve_layout` MUST honour every direction x sizing x alignment combination, and MUST NOT produce negative spacing on overflow. | section 2 | `ambre_toile::logic::autolayout::tests::overflow_does_not_produce_negative_spacing`, `::wrap_breaks_rows`, `::wrap_align_per_row`, `::sizing_fill_shares_leftover`, `::sizing_hug_equals_fixed`, `::align_space_between`, `::space_between_single_child_falls_back_to_start`, `::padding_offsets_origin`, `::empty_children_is_empty` | verified |
| `TOILE-FEAT-006` | A boolean operation MUST be deterministic across runs and area-correct; fewer than two eligible paths MUST be rejected, not silently produce a degenerate element. | section 3 | `ambre_toile::logic::boolean::tests::deterministic_across_runs`, `::union_area_is_combined_minus_overlap`, `::intersect_area_is_overlap`, `::subtract_removes_the_clip`, `::exclude_is_symmetric_difference`, `::fewer_than_two_paths_is_empty`, `ambre_toile::logic::state::boolean::tests::fewer_than_two_eligible_is_rejected`, `::keep_sources_leaves_inputs`, `::boolean_is_one_undo_item` | verified |
| `TOILE-FEAT-007` | A reaction MUST be keyed by `UserId`: the same user reacting from two devices MUST count once. | section 4.A | `ambre_toile::logic::convergence_tests::same_user_two_devices_is_one_entry`, `::concurrent_reactions_distinct_users_converge`, `::react_add_change_remove_updates_tally`, `::tally_skips_a_deleted_target`, `::react_broadcasts_delta_and_is_undoable`, `::react_before_session_ready_is_a_noop` | verified |
| `TOILE-FEAT-008` | Concurrent speaker-notes edits MUST converge character by character, and an out-of-range splice MUST be clamped rather than rejected or lost. | section 1.D | `ambre_toile::logic::convergence_tests::concurrent_notes_edits_converge`, `::edit_notes_produces_delta_and_is_undoable`, `::edit_notes_clamps_out_of_range_splice`, `::edit_notes_rejects_non_frame` | verified |
| `TOILE-FEAT-009` | Concurrent auto-layout spec edits MUST converge by last-writer-wins on the whole spec. | section 2 | `ambre_toile::logic::convergence_tests::concurrent_auto_layout_edits_converge`, `::set_auto_layout_add_change_clear`, `::set_auto_layout_rejects_non_frame` | verified |
| `TOILE-FEAT-010` | A reaction tally MUST skip a deleted target rather than surface a dangling badge. | section 4.A | `ambre_toile::logic::convergence_tests::derive_viewport_carries_tally_and_skips_deleted_target`, `::tally_skips_a_deleted_target` | verified |
| `TOILE-FEAT-011` | Two peers dropping a comment on the same spot MUST mint distinct threads; neither MUST overwrite the other. | section 4.D | not built -- lands with section 9 Phase 2 (Epic `E4`) | unimplemented |
| `TOILE-FEAT-012` | Message order in a thread MUST be the `LoroMovableList` order, never the `ts` register: clocks are untrusted. | section 4.D | not built -- lands with section 9 Phase 2 (Epic `E4`) | unimplemented |
| `TOILE-FEAT-013` | A message body MUST be a `LoroText`, so a self-edit converges character-wise rather than by whole-string LWW. | section 4.D | not built -- lands with section 9 Phase 2 (Epic `E4`) | unimplemented |
| `TOILE-FEAT-014` | A message author MUST be the canonical `UserId` resolved by `ambre-sync`, never a `PeerID`. | section 4.D; section 4.A | not built -- lands with section 9 Phase 2 (Epic `E4`), and needs the `UserId` directory of `SYNC P4` (Epic `E5`) | unimplemented |
| `TOILE-FEAT-015` | Resolving and reopening a thread MUST be idempotent and convergent, and MUST NOT destroy the thread or its messages. | section 4.D | not built -- lands with section 9 Phase 2 (Epic `E4`) | unimplemented |
| `TOILE-FEAT-016` | A thread anchored to a deleted element MUST stay valid and simply not render; it MUST NOT be auto-deleted. | section 4.D; `toile-logic section 2.J` | not built -- lands with section 9 Phase 2 (Epic `E4`) | unimplemented |
| `TOILE-FEAT-017` | A comment command MUST NOT enter the undo stack: `Ctrl+Z` on the board MUST NOT retract a teammate-visible message. | section 4.D; `toile-logic section 8 bis` | not built -- lands with section 9 Phase 2 (Epic `E4`) | unimplemented |

> **Invariant coverage.** 10 `verified`, 0 `unverified`, 7 `unimplemented`. Writing section 4.D's rules as rows is what makes the contradiction it used to hide impossible: a section cannot both specify a *mandatory* convergence belt and describe code nobody has written, once each of its rules carries a status.

---

## 1. PRESENTATION MODE

Presentation is **not a separate application** -- it is an orchestrated view of the same board. A frame becomes a
"slide"; a side panel orders frames into a narrative thread.

### A. Sequence (CRDT)

```
LoroDoc
└── "presentation" (LoroMap)
    ├── "order"   (LoroMovableList<FrameId>)  → the slide sequence (reorderable, convergent)
    └── "config"  (LoroMap)                   → {transition_ms, autoplay, loop}
```

> **`FrameId` is an `ElementId`.** A frame is an ordinary element (`toile-logic section 2.F`); `FrameId` is just the
> `ElementId` of an element whose `type = "frame"`. `presentation.order` is a top-level `LoroDoc` list (it is
> board-global sequencing, not per-frame content, so it stays outside the frame elements).

The sequence references `FrameId`s (stable ids from `toile-logic section 2`), never positions, so reordering slides or
editing a frame's content never breaks the deck. Concurrent reorders converge through `LoroMovableList`;
concurrent inserts of the same frame are idempotent-deduped at derivation.

### B. Playback (local / ephemeral)

```rust
pub struct PlaybackState {
    pub active: usize,          // index into presentation.order (this viewer only)
    pub focus:  bool,           // Focus Mode: chrome hidden, camera locked to the active frame
}
```

`NextSlide`/`PrevSlide`/`GotoSlide` mutate `PlaybackState` locally and drive the `toile-ui` camera; they emit
**no** CRDT delta. Optionally a presenter can broadcast their `active` index as **ephemeral presence**
("follow the presenter"), never as history.

### C. Smart Animate (transitions)

Moving between slide A and slide B, Toile interpolates properties of elements that appear in **both** frames and
share the same stable `ElementId` -- position, size, rotation, opacity, fill -- with an eased `lerp`; elements only
in A fade out, only in B fade in.

```rust
/// Pure, deterministic; runs each frame of the transition in toile-ui's camera pass.
/// No CRDT mutation -- it animates the *presentation* of existing elements.
pub fn smart_animate(
    from: &FrameSnapshot, to: &FrameSnapshot, t: f64 /* 0..=1, eased */,
) -> Vec<AnimatedElement> {
    to.elements.iter().map(|e| {
        match from.by_id(e.id) {
            Some(a) => AnimatedElement::lerp(a, e, t),      // shared id -> tween
            None    => AnimatedElement::fade_in(e, t),      // new in B
        }
    }).chain(
        from.elements.iter()
            .filter(|a| to.by_id(a.id).is_none())
            .map(|a| AnimatedElement::fade_out(a, t))       // gone in B
    ).collect()
}
```

Because matching is by **stable id**, an element the author moved/resized between two frames animates smoothly
"for free" -- the Figma Smart Animate model. The interpolation is a `toile-ui` camera-pass concern fed by two
`FrameSnapshot`s the engine derives.

> **Frame membership is explicit, not spatial.** A `FrameSnapshot`'s elements are the frame's declared
> `children` (`toile-logic section 2.C`), **not** whatever happens to overlap the frame's rect. This keeps matching
> deterministic and lets an element belong to a slide even if it visually spills outside the frame bounds. An
> element in no frame never animates.

### D. Focus Mode & Presenter Overlay

- **Focus Mode:** hides the editing chrome and auto-centers/zooms the camera on the active frame's rect
  (`toile-ui` transform); purely local.
- **Presenter Overlay:** a floating panel showing the active frame's **speaker notes**. Notes are per-frame
  content, stored in the frame **element's** payload (`toile-logic section 2.C`), edited via `EditNotes`:

```
"elements" / "{frame_id}" / payload / "notes" : LoroText   // collaborative speaker notes
```

The `notes` LoroText is seeded empty **at frame creation** (`toile-logic section 2.C`), not on the first `EditNotes`:
a shared container broadcast in the creation delta lets concurrent first edits from two peers target the same
LoroText and converge (section 6). A lazy per-peer seed would mint two rival containers under the same map key -- an
LWW collision that drops one side's notes. `EditNotes { frame, delta: TextDelta }` applies a single char-CRDT
splice `{ at, delete, insert }` (loro codepoint indices, clamped to length); one splice = one undo item.

---

## 2. AUTO-LAYOUT (CONSTRAINT SYSTEM)

> **EXPERIMENTAL -- out of MVP (frozen).** This is the Figma vector-fabrication metier, not the whiteboard job
> (`toile-logic section 0`). It is implemented (`logic/autolayout.rs`) but receives **no MVP stabilization budget** and
> carries no UI/quality promise; it is a V2-or-removal candidate. Kept in the tree; not in the definition of done.

A **frame** can carry an auto-layout spec so its children are positioned by a solver rather than by absolute
coordinates (the Figma model). Auto-layout is a frame-only feature: a plain **group** (`toile-logic section 2.F bis`)
is a transform-binding container with no solver -- promote it to a frame to get auto-layout. The **spec** is CRDT (shared design intent); the **solved positions** are
derived, never persisted (like connector geometry in `toile-logic section 5`).

```
"elements" / "{frame_id}" / payload / "autolayout" : Binary(postcard(AutoLayout))   // absent = free positioning
```

```rust
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct AutoLayout {
    pub direction: Direction,   // Horizontal | Vertical | Wrap
    pub gap:       f64,
    pub padding:   [f64; 4],    // top, right, bottom, left
    pub align:     Align,       // Start | Center | End | SpaceBetween
    pub sizing:    Sizing,      // Hug | Fill | Fixed
}

/// Pure solver: children order = the frame's `children` LoroMovableList.
/// Runs at derivation; output feeds x/y/w/h into the DisplayViewport, and a
/// commit-on-edit writes the resolved geometry back only when the user drops an
/// element (so the CRDT keeps absolute coords as the durable truth).
pub fn solve_layout(frame: kurbo::Rect, layout: &AutoLayout, children: &[kurbo::Size]) -> Vec<kurbo::Rect> { /* ... */ }
```

Reordering `children` (`LoroMovableList`) reflows the layout convergently; adding/removing a child reflows the
rest. The solver is deterministic so all peers derive identical positions from identical CRDT state.

---

## 3. VECTOR BOOLEAN OPERATIONS

> **EXPERIMENTAL -- out of MVP (frozen).** Vector-fabrication metier, not the whiteboard job (`toile-logic section 0`).
> Implemented (`logic/boolean.rs`) but receives **no MVP stabilization budget** and carries no UI/quality
> promise; V2-or-removal candidate. Kept in the tree; not in the definition of done.

Union / Subtract / Intersect / Exclude over selected `shape:path` (and shape) elements, computed client-side,
producing a **new** `shape:path` `BezPath` element (one `BooleanOp` command). The source elements are deleted or
kept per the user's choice (`keep_sources`); when deleted, the whole op is one commit / one undo item. This is a
pure geometry transform -- no special CRDT state; the result is an ordinary element (`toile-logic section 2.C`).

`kurbo` (0.13) has **no** path-vs-path boolean ops, so the combinators run on `i_overlay` (pure Rust,
WASM-friendly): each source outline is flattened to polygon contours, combined with the matching `OverlayRule`
under the non-zero fill rule, and the result is rebuilt as a line-segment `BezPath`. Trade-off: curves are
flattened to polylines (acceptable for a whiteboard). `Subtract` treats the first selected element as the subject
and removes the union of the rest; the other rules fold left to right over the selection.

> Node-level Bezier editing (add/remove/convert nodes, handle drag) is the `toile-ui` Pen tool (`ui section 5bis`);
> this section is only the boolean combinators.

---

## 4. COLLABORATION PRIMITIVES

### A. Votes & reactions (CRDT -- shared, persisted)

Placing an emoji reaction or a vote on an element or frame is shared state that must survive reload, so it is a
CRDT map keyed by target + **canonical `UserId`** (one entry per user per target -> a user's re-vote is LWW on
their own key, and tallies are convergent):

```
LoroDoc
└── "reactions" (LoroMap)
    └── "{target_id}/{user_id}" : Binary(postcard(Reaction))   // {emoji | vote_weight, ts}
```

> **Flat composite key, not a nested sub-map per target.** The entries live directly in the single `reactions`
> root map under a composite `"{target_id}/{user_id}"` key, **not** in a per-target nested `LoroMap`. A per-target
> sub-map would be **created lazily on the first reaction to that target**, so two peers reacting on the same
> fresh target would concurrently create two rival containers; Loro's map register keeps only one, silently
> dropping the loser's reactions. The `reactions` root is a deterministic root container that always exists, so
> concurrent inserts under distinct composite keys all converge. `ElementId`s are hex + `_` and `UserId`s are
> hyphenated UUIDs, so neither contains `/`: the composite key is unambiguous and `target_id` is recovered by
> splitting on the first `/`. (Speaker notes avoid this race differently -- their `LoroText` is seeded at frame
> birth section 1.D, so it is a shared container, never a concurrent lazy creation.)

> **Keyed by `UserId`, never `PeerID`.** A reaction/vote is a per-**user** decision that must survive reload, key
> rotation and multi-device use -- the same identity argument as presence (`toile-logic section 7`) and Trame's range
> protection (`trame-features section 6`, "UserId and not pub_key... stable per-user"), applied here to *persisted* CRDT
> state. `PeerID` is per-session/per-device (`toile-logic section 2.A/section 7`), so keying by it would let one user on two
> devices **vote twice** (double-counted tally), would strand a vote when the user reopens the document in a new
> session (a new `PeerID` cannot remove or change the old key), and would break on rotation. The Logic thread
> writes the local user's own key from `my_user_id` (the `Uuid` received via `LogicEvent::SessionReady`, threaded
> into `ToileState` exactly as in `trame-logic section 7`); remote entries are already `UserId`-resolved in `ambre-sync`.

A tally (count per emoji, vote totals) is **derived** at read time by scanning the `reactions` map for the
target's `"{target_id}/"` key prefix -- never stored -- so it is always consistent with the set of per-user
entries. Removing a reaction deletes the user's own composite key.

**Reactions on a deleted target.** If the `target_id` element/frame is gone (deleted concurrently), its
reaction entries are **skipped at tally derivation** (the deriver first checks the target still exists) -- never
rendered, never destroyed (data-never-destroyed, like a dangling connector `toile-logic section 5.A`). The orphan
entries are reclaimed by ordinary document compaction; no active GC is required.

### B. Highlighter / laser pointer (ephemeral)

The laser/highlighter brainstorming tool is **not** persisted: it is streamed as presence and rendered by
`toile-ui section 8`, decaying on a timer. It never becomes an element and never enters history. (Placed here for
completeness of the collaboration surface; the render and transport are already specified in `toile-ui` /
`toile-logic section 7`.)

### C. Why votes persist but laser does not

Votes/reactions are **decisions** -- they must survive reload and reconcile across peers -> CRDT. Laser/cursors are
**gestures** -- momentary, per-viewer, meaningless after the moment -> ephemeral. This is the same durable-intent
vs momentary-gesture split as section 1 (sequence vs playback).

### D. Comments (CRDT -- anchored threads)

Comments are Toile's **asynchronous** collaboration surface (the synchronous ones are cursors/laser and
reactions): a pinned discussion a teammate can leave on the board and another can answer later. They must survive
reload and converge, so they are CRDT. The *anchoring* rule is fixed in `toile-logic section 2.J`
(`CommentAnchor = Element(ElementId) | Point{x,y}`); the thread schema and semantics live here, next to
reactions.

```
LoroDoc
└── "comments" (LoroMap)                        → key = CommentThreadId (minted stable id, toile-logic section 2.A)
    └── "{thread_id}" (LoroMap)
        ├── "anchor"   : Binary                 → postcard(CommentAnchor) -- element id or free world point
        ├── "resolved" : bool                   → LWW flag; a resolved thread renders collapsed/muted, never deleted
        └── "messages" (LoroMovableList<LoroMap>)→ ordered messages, append-mostly. Each message LoroMap:
            ├── "author" : String               → canonical UserId (hyphenated UUID)
            ├── "ts"     : i64                   → author-local wall clock, DISPLAY ONLY
            └── "body"   (LoroText)              → nested char-CRDT container (self-edit converges char-wise)
```

Each message is a `LoroMap` inside the `messages` `LoroMovableList`, holding an `author` register, a `ts`
register, and a **nested `LoroText`** body container -- so a user editing *their own* message converges
character-wise (the sticky-body pattern, `toile-logic section 2.D`) rather than LWW-on-the-whole-string. `author` is the
canonical `UserId` (resolved in `ambre-sync`, never `PeerID` -- the same identity argument as reactions section 4.A and
presence `toile-logic section 7`); `ts` is display-only -- **message order is the `LoroMovableList` order, never `ts`**
(clocks are untrusted, matching the spoof-proof-presence stance). The `messages` list and each message's `body`
`LoroText` are seeded when the message is appended, so there is no lazy-container race (each thread is already its
own top-level entry under the deterministic `comments` root).

- **Thread identity is a minted `CommentThreadId`** (`toile-logic section 2.A` id scheme), so two peers dropping a
  comment on the same spot mint **distinct** threads -- both survive, neither overwrites the other (the reactions
  lazy-creation race section 4.A does not apply: each thread is its own top-level entry under a deterministic root
  `comments` map).
- **Message order** is the `LoroMovableList` order, convergent under concurrent replies; message bodies are
  `LoroText` so a self-edit converges char-wise.
- **Resolution** is an LWW `bool` -- resolving/reopening is idempotent and convergent; the thread and its messages
  are **never destroyed** by resolving (data-never-destroyed, like a dangling connector `toile-logic section 5.A`).
- **Anchored to a deleted element:** the thread stays valid; its pin is simply not rendered while the anchor
  element is missing (skipped at derivation, exactly like a reaction on a deleted target section 4.A), reclaimed by
  ordinary compaction. It is never auto-deleted.
- **Not in the undo stack.** Posting, replying to, editing or resolving a comment is a **collaboration act**, not
  a canvas edit -- `Ctrl+Z` on the board must not retract a teammate-visible message. Comment commands therefore
  commit to the CRDT but are excluded from the `loro::UndoManager` scope (`toile-logic section 8 bis`), the same stance
  as ephemeral presence being non-undoable.
- **@mentions & notifications are deferred (post-MVP).** A mention would need a notification channel; that is
  **Murmure**'s domain (`README.md` "Applications"), so mentions land only once Toile<->Murmure notification routing
  exists. MVP comments are plain-text threads with resolution. See the roadmap.

Rendering (anchored pins with an unread/resolved state, the thread panel) is `toile-ui section 5sexies` / `section 8ter`; the
Logic thread surfaces pins in `DisplayViewport.comments` (`toile-logic section 9`).

### E. Facilitation tools (DEFERRED -- post-MVP)

Miro's workshop layer -- a **shared countdown timer** and a **structured voting session** -- layers on top of the
primitives above but is **out of MVP** (the core is a whiteboard that presents; facilitation is the next tier).
Specified here so its shape is fixed:

- **Shared timer.** A board-global countdown every participant sees synchronized, for timeboxing an exercise. It
  is **shared intent**, so a small CRDT map `"timer" -> {ends_at: i64, running: bool}` (LWW) -- not ephemeral
  presence, because a late joiner must see the running timer. The countdown itself is derived locally from
  `ends_at`; only start/stop/reset mutate the CRDT. (Clocks are untrusted for ordering, but a timer is a display
  convenience, not a security-sensitive value, so a wall-clock `ends_at` is acceptable here.)
- **Voting session.** A facilitated round over the existing `reactions` map (section 4.A): the facilitator opens a
  session with a **per-user dot budget**, participants spend votes on targets, and the session closes to a
  ranked tally. Session state is CRDT (`"vote_session" -> {open: bool, budget: u8}`); the votes reuse the
  per-`UserId` reaction keys (no new convergence model), with the budget enforced **locally/soft** (blind relay,
  advisory like `locked` -- a modified client could overspend, matching section toile-logic section 6). Closing derives the
  ranked result from the same prefix scan as the tally.

Both are collaboration CRDT state (durable intent, not gesture -- the section 4.C split), so they sit here, not in
presence. Commands/events land under section 5 when built. See the roadmap.

---

## 5. NEW COMMANDS & EVENTS

These variants extend the base `ToileCommand`/`ToileEvent` of `toile-logic section 8`; the frame-content commands
(`EditNotes`, `SetAutoLayout`) are **owned here**, not in logic section 8 (they were removed there to avoid a duplicate
`enum` variant).

```rust
/// Both are plain aliases of ElementId, not new types: a frame is an ordinary
/// element (toile-logic section 2.F), and a reaction target is any element or frame
/// (never a connector). Declared once, here, and reused across the features spec.
pub type FrameId  = ElementId;
pub type TargetId = ElementId;

// added to ToileCommand
SetSlideOrder { order: Vec<FrameId> },
NextSlide, PrevSlide, GotoSlide(usize),   // local playback, no CRDT
EnterPresent { focus: bool }, ExitPresent,
EditNotes { frame: FrameId, delta: TextDelta },
SetAutoLayout { frame: FrameId, layout: Option<AutoLayout> },
BooleanOp { op: BoolOp, ids: Vec<ElementId>, keep_sources: bool },
React { target: TargetId, reaction: Option<Reaction> },  // None = remove

// Comments (section 4.D). Not undoable (a collaboration act, not a canvas edit -- section 4.D).
CreateComment { anchor: CommentAnchor, body: String },      // mints a thread + first message -> ToileEvent::CommentThreadOpened
ReplyComment  { thread: CommentThreadId, body: String },    // appends a CommentMsg
EditComment   { thread: CommentThreadId, msg: usize, delta: TextDelta }, // edit own message body (LoroText)
ResolveThread { thread: CommentThreadId, resolved: bool },  // LWW resolve/reopen
DeleteComment { thread: CommentThreadId, msg: Option<usize> }, // None = delete whole thread (author/moderation)

// added to ToileEvent
SlideChanged(usize),
PresentModeChanged(bool),
ReactionsUpdated(TargetId),               // UI re-reads the derived tally
CommentsUpdated(CommentThreadId),         // a thread changed; UI re-reads the panel + pin badge
CommentThreadOpened(CommentThreadId),     // echo of CreateComment so the UI can open the composer on the new pin
```

---

## 6. QUALITY & TESTABILITY

> What the built features guarantee, and what verifies each guarantee, is the table in section 0. This section used to enumerate in prose the cases that "must" be covered -- including a comments convergence belt for a feature that does not exist. The enumeration was right about the built half and unfalsifiable about the rest; the table is neither.

* **Tier.** host, entirely. `smart_animate`, `solve_layout`, the boolean geometry, the tally derivation and every sequence/playback handler are pure Rust. **wasm** for the presentation camera pass and inline notes editing on the GPU surface, specified in `toile-ui`.
* **Seams.** `CommandSink` / `MockSyncHandle`; fake `FrameSnapshot`s to drive `smart_animate`.
* **Mandatory belts.** A shared feature lands with a convergence case inside the 2-peer proptest (`TOILE-017`) **and** a local/shared split assertion -- what is CRDT converges, what is playback stays local (`TOILE-FEAT-001`). A pure solver lands with its own branch coverage, on host (`TOILE-FEAT-005`, `TOILE-FEAT-006`, `TOILE-FEAT-007` are the models).
* **Comments (section 4.D) are not built.** Their rules are `TOILE-FEAT-011..017` in section 0, with a convergence belt that becomes mandatory the day `CreateComment` exists -- not before.
* **Benchmarks.** None today. section 7 budgets `smart_animate` over a 500-element frame pair and `solve_layout` over a 200-child frame; neither has a `criterion` bench, so neither budget can fail a build.
* **Definition of Done.** An increment is `[x]` only when its belts are green in CI **and** no invariant in section 0 is left without a `Status`.

---

## 7. PERFORMANCE & RESOURCES

| Operation | Budget | Rationale |
|---|---|---|
| `smart_animate` per transition frame (500 elements) | < 4 ms | Leaves room for paint within 16 ms during the transition. |
| `solve_layout` (200 children) | < 1 ms | Runs at derivation on every child edit. |
| Reaction tally derivation | O(board reactions), < 0.1 ms typical | Prefix-scan of the flat `reactions` map on read, never stored (section 4.A). Reaction counts are small, so a full-map scan is well within budget. |
| Boolean op (typical two paths) | < 5 ms, off the render frame | User-initiated, one-shot; may show a brief spinner for huge paths. |

**WASM impact:** feature logic is small; the only new dependency is `i_overlay` (section 3, a light pure-Rust polygon
boolean crate), on top of `kurbo`, `loro`, `postcard`. The presentation camera reuses the `toile-ui` pipeline.

---

## 8. REJECTED ARCHITECTURAL DECISIONS

| Rejected | Reason |
|---|---|
| **Linear presentation engine (PPT-style)** | Rejected in favor of Spatial Frames (Toile Todo section 7); slides are frames of the same board, enabling Smart Animate by shared id. |
| **Persisting solved auto-layout positions in the CRDT** | Redundant derived state that could desync from the spec; the spec is CRDT, positions are derived (section 2, mirrors connector routing). |
| **Persisting laser/highlighter as elements** | Momentary gestures must not enter history; ephemeral presence only (section 4.B/C). |
| **Storing reaction tallies** | Would drift from the per-user entry set; tallies are derived on read (section 4.A). |
| **Reactions/votes keyed by `PeerID`** | `PeerID` is per-session/per-device, so one user on two devices would double-vote and a re-opened session could not remove its own reaction. Keyed by canonical `UserId` (section 4.A), the same decision as presence (`toile-logic section 7`) and Trame protection (`trame-features section 6`). |
| **A nested `LoroMap` per reaction target** | The sub-map would be created lazily on the first reaction, so two peers reacting on the same fresh target concurrently create rival containers and Loro's register keeps only one, silently dropping reactions. A flat `reactions` root map with a composite `"{target_id}/{user_id}"` key (section 4.A) has no lazy container creation, so all concurrent entries converge. |
| **Broadcasting playback position as CRDT** | Per-viewer navigation is local; "follow presenter" is ephemeral presence, not history (section 1.B). |
| **Matching Smart Animate by geometry/heuristics** | Fragile; stable-`ElementId` matching is exact and free (section 1.C). |

---

## 9. INTEGRATION PLAN (ROADMAP)

> Tracked as Stories of Epic **E4** (Toile MVP) in `specs/ambre-roadmap.md`.

**Phase 0 - Core standalone**
- [x] Presentation sequence CRDT + playback state machine
- [x] Speaker notes
- [x] `smart_animate` pure solver (stable-`ElementId` matching, section 1.C) with host tests
- [x] `solve_layout` pure auto-layout solver with host tests
- [x] Reactions / votes CRDT keyed by `UserId` (section 4.A) + derived read-time tally
- [x] Boolean ops (union / subtract / intersect)
- [x] Convergence belt green (concurrent reactions -> one vote per `UserId`, convergent tally, section 6)

**Phase 1 - Integration**
- [x] Wire playback to the `toile-ui` camera pass
- [x] Wire the top menu bar Present entry point (`toile-ui section 5quinquies.E`, `chrome::menu_bar`)
  to `EnterPresent`. `menu_bar_edit(PRESENT)` resolves to `EnterPresent { focus: true }` and
  `ToileView::apply_menu_command` dispatches it; the resulting `PresentModeChanged` opens the
  presenter overlay panel.
- [x] Presenter overlay panel (section 1.D): a Leptos chrome panel (`chrome::PresenterPanel`) showing
  the active slide's speaker notes (editable via `EditNotes`), a `k / n` counter and Prev / Next /
  Exit controls. Fed by the enriched `SlideChanged` payload; the pure model lives in
  `ui::presenter` (host-tested).
- [x] Reactions render (section 4.A): a screen-space badge row painted under each element carrying a
  derived tally, always visible (edit + present). The tally rides in `DisplayViewport.reactions`
  (derived on the Logic thread in one scan, keyed by visible target; deleted targets skipped),
  repainted on `ReactionsUpdated` (local) or fresh `ViewportData` (remote). The pass
  (`ui::reactions::paint_reactions`, host-tested) draws one rounded chip per emoji -- the color
  emoji glyph + count via the shared color-glyph atlas (`ambre-canvas`) -- plus a vote badge.
  Minimal input: a fixed emoji picker in the context menu (`REACT_*` `CommandId`s resolving to
  one `React` per selected target; `REACT_CLEAR` removes the user's own reaction).
- [x] "Follow presenter" ephemeral broadcast (section 1.B): a presenter broadcasts its active slide as an
  ephemeral presence frame (`PresencePayload::present_slide`, never CRDT); followers
  (`FollowPresenter`) adopt it and glide via the existing camera pass. Manual navigation takes
  control (clears the follow). Local roles `SetPresentBroadcast` / `FollowPresenter`; the presenter
  panel gains Broadcast / Follow toggles.

**Phase 1b - Comments (MVP collaboration gap, section 4.D)**
- [ ] `comments` CRDT (anchored threads, replies, resolve) + commands (`CreateComment`/`ReplyComment`/
      `EditComment`/`ResolveThread`/`DeleteComment`), excluded from undo
- [ ] Comment pins in `DisplayViewport.comments` (derive + skip-on-deleted-anchor) and the `toile-ui` panel
      (`section 5sexies`) + pin render/hit-test
- [ ] Convergence belt: distinct threads survive a same-spot drop, concurrent replies converge, resolve is LWW

**Phase 2 - Optimization**
- [ ] Auto-layout incremental reflow (only affected children)
- [ ] Smart Animate snapshot caching
- [ ] Large-deck sequence-panel virtualization

**Deferred - Facilitation layer (section 4.E, post-MVP)**
- [ ] Shared countdown timer (`timer` CRDT, LWW start/stop/reset; countdown derived locally)
- [ ] Voting session over `reactions` (per-user dot budget, open/close, ranked tally)
