# AMBRE -- FEATURE: PRESENTATION

**Type:** Vertical (feature)
**Implementation:** partial -- playback, the panel and the transition ship; no gesture builds a deck.

> A user orders a few frames into a deck and presents them: the board walks the audience from one
> frame to the next, animating what two neighbouring slides share.

A presentation is not a second application. A slide is a frame, and the deck is an order over
frames. What the group agrees on is shared; where each viewer is looking is not.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`. Two halves are cited and never restated:
the frame as an element (`features/ambre-spec-toile-frames.md`), and the ephemeral transport that
carries a presenter's position (`features/ambre-spec-toile-presence.md section 3`).

| ID | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|
| `TOILE-FEAT-001` | Shared intent -- the slide order and the speaker notes -- MUST be CRDT; per-viewer playback MUST stay local and MUST NOT be synced. | this spec section 3 | `ambre_toile::logic::convergence_tests::present_mode_toggle_is_local`, `::playback_is_not_synced`, `::follow_state_is_not_synced`, `::present_broadcast_toggle_is_local`, `::set_slide_order_produces_delta_and_is_undoable` | verified |
| `TOILE-FEAT-002` | The deck MUST deduplicate a repeated frame, and playback MUST be clamped to the deck bounds. | this spec section 3 | `ambre_toile::logic::convergence_tests::slide_order_dedups_repeated_frames`, `::playback_bounds`, `::adopted_slide_is_clamped_to_deck` | verified |
| `TOILE-FEAT-003` | A follower MUST adopt the presenter's slide; a non-follower MUST ignore the broadcast; a manual navigation MUST break the follow. | this spec section 3 | `ambre_toile::logic::convergence_tests::follower_adopts_presenter_slide`, `::non_follower_ignores_broadcast`, `::following_other_peer_ignores_broadcast`, `::manual_nav_breaks_follow`, `::follow_snaps_to_last_known_slide`, `::broadcast_frame_carries_active_slide`, `::no_broadcast_frame_when_not_broadcasting` | verified |
| `TOILE-FEAT-004` | A transition MUST tween the elements two slides share, fade in the new ones and fade out the gone ones, and MUST reproduce each slide exactly at its own end. | this spec section 3 | `ambre_toile::logic::animate::tests::shared_id_tweens_at_midpoint`, `::new_element_fades_in`, `::gone_element_fades_out`, `::endpoints_reproduce_source_and_target`, `::mixed_membership_classifies_each_id`, `::empty_frames` | verified |
| `TOILE-FEAT-008` | Concurrent speaker-notes edits MUST converge character by character, and an out-of-range splice MUST be clamped rather than lost. | this spec section 3 | `ambre_toile::logic::convergence_tests::concurrent_notes_edits_converge`, `::edit_notes_produces_delta_and_is_undoable`, `::edit_notes_clamps_out_of_range_splice`, `::edit_notes_rejects_non_frame` | verified |
| `TOILE-UI-084` | The camera MUST reach the active frame at the end of a transition, and MUST hold the previous one at its start. | this spec section 4 | `ambre_toile::ui::present::tests::start_frame_holds_source_camera_and_content`, `::end_frame_lands_on_target_camera_and_content`, `::midpoint_tweens_between_endpoints`, `::zero_duration_snaps_immediately` | verified |
| `TOILE-UI-086` | The presenter panel MUST show the active slide's rank, the deck's length and that slide's notes. | this spec section 4 | `ambre_toile::ui::presenter::tests::counter_label_is_one_based`, `::counter_label_empty_deck`, `ambre_toile::tests::chrome_ui::presenter_panel_counter_is_one_based`, `::presenter_panel_hidden_when_closed` | verified |
| `TOILE-UI-087` | Every playback control MUST resolve to the local command it names. | this spec section 4 | `ambre_toile::ui::menu_bar::tests::present_entry_emits_enter_present`, `ambre_toile::tests::chrome_ui::presenter_nav_buttons_fire_intents` | verified |
| `TOILE-UI-088` | A note edit MUST reach the document as the one smallest splice that explains it. | this spec section 4 | `ambre_toile::ui::presenter::tests::delta_none_on_no_change`, `::delta_pure_append`, `::delta_pure_prefix_insert`, `::delta_pure_delete`, `::delta_mid_string_replace`, `::delta_from_empty`, `::delta_to_empty`, `::delta_multibyte_indices_are_codepoints`, `::delta_multibyte_mid_replace`, `::delta_repeated_char_prefers_prefix`, `ambre_toile::tests::chrome_ui::presenter_notes_edit_fires_minimal_delta` | verified |
| `TOILE-UI-089` | The follow affordance MUST appear only while a peer broadcasts a slide. | this spec section 4 | `ambre_toile::tests::chrome_ui::presenter_follow_button_hidden_without_presenter`, `::presenter_follow_button_fires_with_peer`, `::presenter_broadcast_button_fires_toggle` | verified |
| `TOILE-FEAT-027` | A slide whose frame is gone MUST be skipped, and MUST NOT be presented as a blank slide. | this spec section 3 | none, and **the rule fails today** on the board: the export skips it (`TOILE-IO-005`) and playback does not, so the deck counts the dead slide and the camera holds still on it -- section 6 | unverified |
| `TOILE-FEAT-028` | A transition MUST last the time the deck's own configuration carries. | this spec section 3 | none, and **the rule fails today**: the render thread uses a fixed 300 ms constant, and nothing reads the three configuration fields the deck writes at birth -- section 6 | unverified |
| `TOILE-UI-083` | Presentation mode MUST hide every editing surface. | this spec section 4 | none, and **the rule fails today**: the mode drops the board grid and nothing else, so the palette, the menu bar, the inspector, the floating bar and the status bar stay on screen and the board stays editable -- section 6 | unverified |
| `TOILE-UI-085` | An element the next slide drops MUST fade out over the whole transition. | this spec section 4 | none, and **the rule fails today**: an override paints only for an element the cull keeps, so an element the gliding camera leaves stops mid-fade -- section 6 | unverified |
| `TOILE-FEAT-026` | A user MUST be able to add a frame to the deck, reorder it and remove it. | this spec section 4 | not built -- lands with the "Order the deck" Story (`ambre-roadmap.md` E4) | unimplemented |

* **Budget.** the transition frame under 4 ms at 500 elements, the slide-transition row of
  `ambre-spec-budgets.md section 1`. A transition frame is a full repaint and a fresh cull, so
  it costs a frame of the board plus one pass over the two slides.
* **Boundary.** one outbound flow, and only while the presenter broadcasts: the slide index rides an
  ephemeral presence frame, which `ZK-005` seals with the `DocumentKey`. The deck and the notes are
  document data like any other, sealed by `ZK-001`.
* **Verification.** 9 `verified`, 5 `unverified`, 1 `unimplemented`. The machinery is the
  best-covered subject of the application: convergence, playback, follow, the solver endpoints, the
  camera and the panel are each held. Every gap is on the way in and the way out -- no gesture builds
  a deck, a dead slide is still presented, the mode hides one surface of six, and a fade stops when
  the cull does.

---

## 2. WHAT EACH USER SEES

| Alice does | Bob does | The document holds | Each user sees | Test |
|---|---|---|---|---|
| orders three frames into a deck | watches | one order of three | the same three slides, in the same order (`TOILE-FEAT-001`) | `logic::convergence_tests::set_slide_order_produces_delta_and_is_undoable` |
| orders A, B, A | watches | one order of two | A then B: a frame is in the deck once (`TOILE-FEAT-002`) | `logic::convergence_tests::slide_order_dedups_repeated_frames` |
| presents and steps to slide 2 | reads the board | the order, unchanged | Alice on slide 2, Bob wherever he was (`TOILE-FEAT-001`) | `logic::convergence_tests::playback_is_not_synced` |
| presents and broadcasts | follows Alice | the order, unchanged | Bob's camera on Alice's slide, and no delta written (`TOILE-FEAT-003`) | `logic::convergence_tests::follower_adopts_presenter_slide` |
| broadcasts slide 4 | follows, on a deck of two | the order, unchanged | Bob on the last slide, never past the end (`TOILE-FEAT-002`) | `logic::convergence_tests::adopted_slide_is_clamped_to_deck` |
| follows Alice, then steps herself | broadcasts | the order, unchanged | Alice takes control and stops following (`TOILE-FEAT-003`) | `logic::convergence_tests::manual_nav_breaks_follow` |
| types a note on the front slide | types on the same note | both sentences, interleaved | one text, no sentence lost (`TOILE-FEAT-008`) | `logic::convergence_tests::concurrent_notes_edits_converge` |
| deletes a frame the deck names | presents | the order, with a dead entry | a blank slide, counted, that no camera moves to (`TOILE-FEAT-027`) | none -- section 6 |

---

## 3. MODEL

**Containers.** The `presentation` map, holding the `order` list of frame ids and the `config` map;
and one `notes` text per frame element (`ambre-spec-toile-schema.md section 3`). The order names
frames by their minted id, never by position, so reordering slides and editing a frame never break
the deck. Two peers ordering the deck at the same moment race for the container itself, which is
`TOILE-029`.

**Playback is a field, not a container.** The active index and the focus flag live on the engine's
own state and reach no peer (`TOILE-FEAT-001`).

**Commands** (`ambre-bus`): `SetSlideOrder { order }` writes the deck; `NextSlide`, `PrevSlide` and
`GotoSlide(index)` move this viewer; `EnterPresent { focus }` and `ExitPresent` toggle the mode;
`SetPresentBroadcast(on)` and `FollowPresenter(who)` set the two roles; `EditNotes { frame, delta }`
splices one frame's notes.

**Events** (`ambre-bus`): `SlideChanged(SlideChange)`, `PresentModeChanged(bool)`,
`PresentBroadcastChanged(bool)` and `FollowingChanged(Option<UserId>)`. The engine owns the board and
the render thread owns none, so the slide event carries everything the camera pass needs:

```rust
pub struct SlideChange {
    pub index:   usize,             // the new slide, 0-based
    pub from:    FrameSnapshot,     // the previous slide's children, empty on entry
    pub to:      FrameSnapshot,     // the new slide's children
    pub to_rect: Option<Rect>,      // the camera fit target, None when the frame is gone
    pub count:   usize,             // the deck length, for the panel counter
    pub frame:   Option<ElementId>, // the note-edit target
    pub notes:   String,            // that frame's notes, seeding the editor
}

pub struct SnapElement {
    pub id:       ElementId,
    pub bounds:   Rect,       // the unrotated element box, world space
    pub rotation: f64,
    pub opacity:  f32,
    pub fill:     Option<u32>,
}
```

**Steps, when the deck is written:**

1. drop a repeated frame, keeping its first place (`TOILE-FEAT-002`);
2. stop when the result equals the current order, so an unchanged deck is not an undo item;
3. rewrite the list whole, commit once, and clamp this viewer's index to the new length.

**Steps, when this viewer moves to a slide:**

1. clear any follow, so a manual navigation takes control (`TOILE-FEAT-003`);
2. clamp the index to the deck, capture the two frame snapshots and the target frame's rect, and
   emit them as one event;
3. attach an ephemeral frame carrying the index when this viewer broadcasts, and nothing otherwise.

**Steps, when a transition frame is drawn:**

1. read the phase from the frame clock, and ease it once for the camera and the content alike;
2. glide the camera from the frame it started on toward the fitted target (`TOILE-UI-084`);
3. tween each element the two slides share by id, fade in what only the new slide holds, fade out
   what only the old one holds (`TOILE-FEAT-004`);
4. stop at the raw phase, so the transition ends exactly on its length.

**Membership is declared, never spatial.** A slide's elements are the frame's own children, not
whatever overlaps its box. An element in no frame never animates.

**Code:**

* `crates/ambre-toile/src/logic/state/presentation.rs` -- the deck, playback, the roles, the notes
  and the export pages;
* `logic/animate.rs` -- the tween, the two fades and the easing curve;
* `logic/state/derive.rs` -- `frame_snapshot`, the capture the solver consumes;
* `ui/present.rs` -- the transition, one camera and one override set per frame;
* `ui/presenter.rs` and `chrome/presenter.rs` -- the counter, the note splice and the panel.

---

## 4. SURFACE

| Trigger | Behaviour |
|---|---|
| the File or Present menu entry | enters presentation mode on the first slide (`TOILE-UI-087`) |
| the panel's previous and next buttons | move this viewer one slide, and break any follow |
| the panel's exit button | leaves presentation mode, and stops broadcasting |
| the panel's broadcast button | starts or stops sharing this viewer's slide with peers |
| the panel's follow button | adopts, or stops adopting, the broadcasting peer's slide (`TOILE-UI-089`) |
| typing in the panel's notes area | splices that slide's notes (`TOILE-UI-088`) |
| a slide change | glides the camera and animates the content until the transition ends |

**The panel** is chrome, not a painted overlay: it is a floating Leptos component, like the find
panel (`features/ambre-spec-toile-find.md section 4`). It opens with presentation mode and shows the
slide counter, the five controls and the notes editor. The follow button is shown only while a peer
broadcasts.

**The notes editor sends a splice, never a body.** A text area only reports its whole value, so the
panel trims the common head and tail and sends what is left as one insert and one delete
(`TOILE-UI-088`). That is what a keystroke and a paste each produce, and it is what makes two authors
converge instead of overwriting each other (`TOILE-FEAT-008`).

**What no surface offers** (`TOILE-FEAT-026`). Nothing adds a frame to the deck, reorders it or
removes it: the write command exists and its only callers are tests. Presentation mode therefore
opens on an empty deck, the counter reads `0 / 0`, and the notes editor has no frame to write to.

**Focus mode drops the grid and nothing else** (`TOILE-UI-083`). The editing chrome stays mounted and
the board stays live under the presentation: a key that deletes still deletes.

---

## 5. DECISIONS

| Decision | Rationale | Rejected alternative |
|---|---|---|
| A slide is a frame of the same board | the presentation is a path through the work, so it needs no second document, no copy and no export step. It is also what makes the transition free: two slides that share an element share its id | a linear deck of its own, which duplicates every element that appears twice and animates nothing |
| The order is shared and the playback is not | the group agrees on the story; each viewer looks where they want, and a reader who scrolls back must not drag everyone with them. Follow-the-presenter is the opt-in that puts them together again | broadcasting the position as document history, which writes a delta per keystroke of navigation and cannot be left |
| The presenter's position rides the presence channel | it is a gesture, not a decision: momentary, per viewer, meaningless an hour later. The channel that carries a cursor carries it for nothing | a shared register naming the active slide, which persists a fact nobody wants back |
| A transition matches elements by their minted id | the match is exact and costs nothing: the author moved the element, so the two snapshots hold the same id and the tween writes itself | matching by position, size or shape, which guesses, and guesses differently on each board |
| The deck names frames, never positions | an id survives a move, a reorder and a regroup, so editing the board never rewrites the deck | an index into the paint order, invalidated by the first reorder |
| The solved transition is derived per frame, never stored | the animation is a view of two states the document already holds, so a peer that joins mid-transition sees the board and not a half-played artefact | writing the interpolated geometry back, which puts a frame of animation in everyone's history |
| Speaker notes are a character-wise text seeded at frame birth | two authors write notes on the same slide, and the container must exist before either of them types. A lazily created one mints two rivals and drops one side | seeding on the first edit, which is the race `TOILE-029` records for the deck itself |

---

## 6. NOT BUILT, AND NOT YET RULED

* **No gesture builds a deck** (`TOILE-FEAT-026`). The write command, the deduplication, the clamp,
  the undo item and the two-peer convergence all exist and are tested; the panel that orders frames
  does not. Every surface downstream inherits the hole: playback walks nothing, the counter reads
  `0 / 0`, the camera never moves, and the notes editor drops each keystroke because it has no frame
  to name. The roadmap's ticked "Presentation and reactions" Story claimed frame-by-frame
  presentation and speaker notes; both are now the "Order the deck" Story.
* **A deleted frame stays a slide** (`TOILE-FEAT-027`). Deleting an element drops its paint-order
  entry, its membership and its connectors (`TOILE-034`); it never touches the deck. The dead entry
  is then counted in the deck length, and the viewer who reaches it sees the board hold still with no
  camera target. The PDF export skips the same entry (`TOILE-IO-005`), so the two readings of one
  deck disagree.
* **The deck's configuration is written once and read by nobody** (`TOILE-FEAT-028`). The three
  fields -- the transition length, autoplay and loop -- are seeded with the deck and no code reads
  them. The transition uses a constant, and neither autoplay nor loop has any code at all.
* **Focus mode is a flag with one value and no reader.** The two menu entries both ask for focus, the
  engine stores the answer, and only a test accessor reads it. The mode the field was meant to
  distinguish -- present without focus -- cannot be asked for.
* **Nothing clips the board to the slide.** The camera fits the frame and the rest of the board keeps
  painting around it, so a neighbouring frame's content shows at the edges. The PDF export clips each
  page to its frame rect, which is the second place the two paths answer the same question
  differently.
* **Presentation mode cannot be left from the keyboard.** `Escape` cancels a gesture, leaves an
  embed, drops the selection and leaves a group scope; it does not leave the presentation. The panel's
  exit button is the only way out.
* **Entering with an empty deck is accepted.** The mode opens, the grid disappears and the panel
  shows `0 / 0`. Nothing refuses the entry and nothing tells the user what is missing.
* **A fade-out is bounded by the cull, not by the transition** (`TOILE-UI-085`). The override set
  carries every element of both slides; the paint pass applies an override only to an element the
  visible set holds. The camera glides away from the old slide, so its elements are culled part-way
  through their own fade.
