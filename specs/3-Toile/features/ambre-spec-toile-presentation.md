# AMBRE -- FEATURE: PRESENTATION

**Type:** Vertical (feature)
**Implementation:** implemented -- the deck panel, playback, the transition and the notes all ship. The
deck's configuration is written and read by nobody.

> A user orders a few frames into a deck and presents them. The board walks the audience from one
> frame to the next, animating what two neighbouring slides share.

A presentation is not a second application. A slide is a frame, and the deck is an order over
frames. What the group agrees on is shared; where each viewer is looking is not.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`. Two halves are cited and never restated:
the frame as an element (`features/ambre-spec-toile-frames.md`), and the ephemeral transport that
carries a presenter's position (`features/ambre-spec-toile-presence.md section 3`).

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TOILE-FEAT-001` | `INV` | The slide order and the speaker notes MUST be document data: shared by the CRDT, and undoable. | this spec section 3 | `ambre_toile::logic::convergence_tests::set_slide_order_produces_delta_and_is_undoable`; `ambre_toile::logic::convergence_tests::edit_notes_produces_delta_and_is_undoable` | none | verified |
| `TOILE-FEAT-075` | `INV` | Playback -- the active slide, the mode, the broadcast role and the follow role -- MUST stay local to one viewer, never synced. | this spec section 3 | `ambre_toile::logic::convergence_tests::present_mode_toggle_is_local`; `ambre_toile::logic::convergence_tests::playback_is_not_synced`; `ambre_toile::logic::convergence_tests::follow_state_is_not_synced`; `ambre_toile::logic::convergence_tests::present_broadcast_toggle_is_local` | none | verified |
| `TOILE-FEAT-002` | `INV` | The deck MUST hold a frame at most once. | this spec section 3 | `ambre_toile::logic::convergence_tests::slide_order_dedups_repeated_frames` | none | verified |
| `TOILE-FEAT-076` | `INV` | The active index MUST name a slide of the deck, whatever asked for it. | this spec section 3 | `ambre_toile::logic::convergence_tests::playback_bounds`; `ambre_toile::logic::convergence_tests::adopted_slide_is_clamped_to_deck` | none | verified |
| `TOILE-FEAT-008` | `CONC` | Concurrent speaker-notes edits MUST converge character by character, with no sentence lost. | this spec section 3 | `ambre_toile::logic::convergence_tests::concurrent_notes_edits_converge` | none | verified |
| `TOILE-FEAT-027` | `INV` | The deck MUST hold only slides whose frame still exists. | this spec section 3 | `ambre_toile::logic::convergence_tests::deleting_a_frame_drops_its_slide`; `ambre_toile::logic::convergence_tests::deleting_an_element_seeds_no_deck` | none | verified |
| `TOILE-UI-342` | `OBS` | The deck panel MUST show the deck in playback order. | this spec section 4 | `ambre_toile::tests::chrome_effects::deck_panel_lists_the_slides_in_order`; `ambre_toile::tests::chrome_ui::deck_panel_empty_deck_invites_a_first_slide` | none | verified |
| `TOILE-UI-368` | `OBS` | The deck panel MUST name a frame the user has not titled by its rank. | this spec section 4 | `ambre_toile::ui::deck::tests::an_unnamed_frame_reads_by_rank`; `ambre_toile::ui::deck::tests::a_named_frame_reads_by_name` | none | verified |
| `TOILE-FEAT-098` | `OBS` | A deck that changed under this viewer -- by a peer, a history step or a deleted frame -- MUST reach the open panel. | this spec section 3 | `ambre_toile::logic::convergence_tests::every_path_that_moves_the_deck_says_so` -- `DeckChanged` on a local reorder, a peer's delta, an undo and a deleted frame; `ambre_toile::ui::view::view_tests::a_changed_deck_is_read_again_while_the_panel_is_open` -- the view asks for the deck again while the panel is open, and not while it is closed | none | verified |
| `TOILE-UI-086` | `OBS` | The presenter panel MUST show the active slide's rank, the deck's length and that slide's notes. | this spec section 4 | `ambre_toile::ui::presenter::tests::counter_label_is_one_based`; `ambre_toile::ui::presenter::tests::counter_label_empty_deck`; `ambre_toile::tests::chrome_ui::presenter_panel_counter_is_one_based`; `ambre_toile::tests::chrome_ui::presenter_panel_hidden_when_closed` | none | verified |
| `TOILE-UI-089` | `OBS` | The panel MUST show the follow affordance exactly when the presenter signal names a peer. | this spec section 4 | `ambre_toile::tests::chrome_ui::presenter_follow_button_hidden_without_presenter` -- the signal naming no peer; `ambre_toile::tests::chrome_ui::presenter_follow_button_fires_with_peer` -- the signal naming one (both mount the panel with that signal set by hand, so they hold the component and never the relay that feeds it, which is `TOILE-UI-315`) | none | verified |
| `TOILE-UI-315` | `OBS` | The presenter signal MUST name a peer only while that peer is broadcasting a slide. | this spec section 4 | `ambre_toile::logic::convergence_tests::stopping_a_broadcast_sends_a_frame_naming_no_slide` -- the frame a presenter sends when it stops, and the silence of a viewer that was not broadcasting; `ambre_toile::tests::chrome_ui::the_presenter_is_named_only_while_it_broadcasts` -- the relay names the peer on a frame carrying a slide, and nobody once its frame carries none or it leaves the live set | none | verified |
| `TOILE-UI-316` | `OBS` | The notes area MUST show a peer's edit to the note on screen. | this spec section 4 | `ambre_toile::logic::convergence_tests::a_peers_note_edit_reaches_the_slide_on_screen` -- the event a remote splice raises, and the silence of a remote change that leaves the note alone; `ambre_toile::tests::chrome_effects::a_peer_note_write_keeps_the_caret` -- the area shows the write, the caret shifted past the peer's prefix | none | verified |
| `TOILE-UI-083` | `OBS` | Presentation mode MUST hide every editing surface. | this spec section 4 | `ambre_toile::ui::render::tests::overlay_paints_no_editing_surface_while_presenting` -- a selection and a find hit that draw nothing while the mode holds; `ambre_toile::ui::view::view_tests::a_presentation_takes_no_edit` -- a click, a delete, an undo chord and a double click send nothing; `ambre_toile::ui::view::view_tests::a_presentation_closes_the_open_editor` -- the open editor closes and keeps its text; `ambre_toile::tests::chrome_ui::the_editing_chrome_leaves_while_the_board_presents` -- the editing chrome leaves the DOM and comes back | none | verified |
| `TOILE-UI-085` | `OBS` | An element the next slide drops MUST fade out over the whole transition. | this spec section 4 | `ambre_toile::ui::present::tests::the_covering_cull_holds_both_ends_of_the_glide` -- the cull a transition asks for covers the camera it starts on and the one it lands on, so neither slide's elements leave the visible set while the fade runs | none | verified |

### 1.2 Order the deck

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TOILE-FEAT-077` | `POST` | Writing the deck MUST leave the order the caller named, each frame at its first place, as one undo item. | this spec section 3 | `ambre_toile::logic::convergence_tests::set_slide_order_produces_delta_and_is_undoable` -- the order and the undo item; `ambre_toile::logic::convergence_tests::slide_order_dedups_repeated_frames` -- each frame at its first place | none | verified |
| `TOILE-FEAT-078` | `POST` | Writing the order already held MUST produce neither a commit nor a delta. | this spec section 3 | `ambre_toile::logic::convergence_tests::set_slide_order_unchanged_writes_nothing` | none | verified |
| `TOILE-FEAT-079` | `POST` | Writing a shorter deck MUST clamp this viewer's active index to the new length. | this spec section 3 | `ambre_toile::logic::convergence_tests::shorter_deck_clamps_the_active_slide` | none | verified |
| `TOILE-FEAT-080` | `FRAME` | Writing the deck MUST NOT touch an element, a membership or a note. | this spec section 3 | structural -- `set_slide_order` clears and re-pushes the order list alone, and reaches no element map, no children list and no notes container | none | verified |
| `TOILE-FEAT-026` | `POST` | A user MUST be able to add a frame to the deck, reorder it and remove it. | this spec section 4 | `ambre_toile::ui::deck::tests::adding_appends_and_a_frame_already_on_the_deck_writes_nothing`; `ambre_toile::ui::deck::tests::removing_drops_that_rank_alone`; `ambre_toile::ui::deck::tests::shifting_swaps_with_the_neighbour`; `ambre_toile::ui::deck::tests::shifting_past_either_end_writes_nothing`; `ambre_toile::tests::chrome_ui::deck_panel_edits_fire_intents` | none | verified |
| `TOILE-UI-343` | `FRAME` | The deck panel MUST NOT hold an order of its own: every edit it shows MUST be one the board answered with. | this spec section 4 | structural -- the panel renders the `DeckView` signal alone and emits a `DeckIntent`, the view turns each intent into one `SetSlideOrder` over the order it last read back, and it writes the signal only from a `DeckData` event | none | verified |

### 1.3 Move to a slide

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TOILE-FEAT-081` | `POST` | Moving to a slide MUST emit one event carrying the new index, both slides' snapshots, the camera target, the deck length, and the target frame with its notes. | this spec section 3 | type-level -- `SlideChange` carries exactly those seven fields, and `slide_change` fills every one of them on each move | none | verified |
| `TOILE-FEAT-082` | `POST` | Moving to a slide by hand MUST clear any follow, so the viewer takes control. | this spec section 3 | `ambre_toile::logic::convergence_tests::manual_nav_breaks_follow` | none | verified |
| `TOILE-FEAT-083` | `FRAME` | Moving to a slide MUST NOT write to the document. | this spec section 3 | `ambre_toile::logic::convergence_tests::playback_is_not_synced` | none | verified |

### 1.4 Follow a presenter

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TOILE-FEAT-003` | `POST` | A follower MUST adopt the presenter's broadcast slide. | this spec section 3 | `ambre_toile::logic::convergence_tests::follower_adopts_presenter_slide`; `ambre_toile::logic::convergence_tests::adopted_slide_is_clamped_to_deck` | none | verified |
| `TOILE-FEAT-084` | `POST` | Starting to follow MUST snap this viewer to the presenter's last known slide, without waiting for the next broadcast. | this spec section 3 | `ambre_toile::logic::convergence_tests::follow_snaps_to_last_known_slide` | none | verified |
| `TOILE-FEAT-085` | `FRAME` | A broadcast from a peer this viewer does not follow MUST change nothing. | this spec section 3 | `ambre_toile::logic::convergence_tests::non_follower_ignores_broadcast`; `ambre_toile::logic::convergence_tests::following_other_peer_ignores_broadcast` | none | verified |

### 1.5 Broadcast this viewer's position

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TOILE-FEAT-086` | `POST` | While this viewer broadcasts, each slide change MUST attach an ephemeral presence frame carrying the active index. | this spec section 3 | `ambre_toile::logic::convergence_tests::broadcast_frame_carries_active_slide` | none | verified |
| `TOILE-FEAT-087` | `FRAME` | While this viewer does not broadcast, a slide change MUST attach no frame. | this spec section 3 | `ambre_toile::logic::convergence_tests::no_broadcast_frame_when_not_broadcasting` | none | verified |

### 1.6 Write a speaker note

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TOILE-UI-088` | `POST` | A note edit MUST reach the document as the one smallest splice that explains it. | this spec section 4 | `ambre_toile::ui::presenter::tests::delta_none_on_no_change`; `ambre_toile::ui::presenter::tests::delta_pure_append`; `ambre_toile::ui::presenter::tests::delta_pure_prefix_insert`; `ambre_toile::ui::presenter::tests::delta_pure_delete`; `ambre_toile::ui::presenter::tests::delta_mid_string_replace`; `ambre_toile::ui::presenter::tests::delta_from_empty`; `ambre_toile::ui::presenter::tests::delta_to_empty`; `ambre_toile::ui::presenter::tests::delta_multibyte_indices_are_codepoints`; `ambre_toile::ui::presenter::tests::delta_multibyte_mid_replace`; `ambre_toile::ui::presenter::tests::delta_repeated_char_prefers_prefix`; `ambre_toile::tests::chrome_ui::presenter_notes_edit_fires_minimal_delta` | none | verified |
| `TOILE-FEAT-088` | `POST` | A splice measured against a note a peer has since moved MUST land where it was typed, and the panel MUST be told the merged note. | this spec section 4 | `ambre_toile::logic::convergence_tests::a_splice_measured_against_a_moved_note_lands_where_it_was_typed` -- a keystroke after a peer's prefix lands at the end, the merged note is announced, and a splice on the stored note announces nothing | none | verified |
| `TOILE-FEAT-089` | `FAIL` | A splice reaching past the end of the note MUST be clamped, not dropped. | this spec section 3 | `ambre_toile::logic::convergence_tests::edit_notes_clamps_out_of_range_splice` | none | verified |
| `TOILE-FEAT-090` | `FAIL` | A note edit naming an element that is not a frame MUST be refused. | this spec section 3 | `ambre_toile::logic::convergence_tests::edit_notes_rejects_non_frame` | none | verified |
| `TOILE-FEAT-091` | `FAIL` | A note edit made with no frame on screen MUST be refused before it reaches the document. | this spec section 4 | `ambre_toile::tests::chrome_ui::a_note_edit_with_no_frame_on_screen_is_refused` -- the intent with no frame becomes no command, and the same intent with one becomes a note edit | none | verified |
| `TOILE-FEAT-092` | `FRAME` | A note edit MUST touch that frame's notes and nothing else. | this spec section 3 | structural -- `edit_notes` resolves the named frame's notes container and splices it, and it reaches no other element and no other container | none | verified |

### 1.7 Draw one frame of a transition

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TOILE-FEAT-004` | `POST` | A transition MUST tween the elements two slides share, fade in the ones only the new slide holds, and fade out the ones only the old one holds. | this spec section 3 | `ambre_toile::logic::animate::tests::shared_id_tweens_at_midpoint`; `ambre_toile::logic::animate::tests::new_element_fades_in`; `ambre_toile::logic::animate::tests::gone_element_fades_out`; `ambre_toile::logic::animate::tests::mixed_membership_classifies_each_id`; `ambre_toile::logic::animate::tests::empty_frames` | none | verified |
| `TOILE-FEAT-093` | `POST` | A transition MUST reproduce each slide exactly at its own end. | this spec section 3 | `ambre_toile::logic::animate::tests::endpoints_reproduce_source_and_target` | none | verified |
| `TOILE-UI-084` | `POST` | The camera MUST reach the active frame at the end of a transition. | this spec section 4 | `ambre_toile::ui::present::tests::end_frame_lands_on_target_camera_and_content`; `ambre_toile::ui::present::tests::zero_duration_snaps_immediately` | none | verified |
| `TOILE-UI-317` | `POST` | The camera MUST hold the previous slide at the start of a transition, and move between the two ends only. | this spec section 4 | `ambre_toile::ui::present::tests::start_frame_holds_source_camera_and_content`; `ambre_toile::ui::present::tests::midpoint_tweens_between_endpoints` | none | verified |
| `TOILE-FEAT-028` | `POST` | A transition MUST last the time the deck's own configuration carries. | this spec section 3 | `ambre_toile::logic::convergence_tests::a_transition_lasts_what_the_deck_configures` -- the configured length riding the slide event, and the default a register holding anything else reads as | none | verified |
| `TOILE-UI-318` | `FRAME` | Drawing a transition frame MUST NOT write to the document. | this spec section 4 | structural -- `ui::present::Transition` holds two snapshots and a clock, and its `advance` has no document handle and no command sink, so a played frame cannot be written back | none | verified |

### 1.8 Enter and leave the presentation

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TOILE-UI-087` | `POST` | The Present menu entry MUST enter presentation mode on the first slide. | this spec section 4 | `ambre_toile::ui::menu_bar::tests::present_entry_emits_enter_present` | none | verified |
| `TOILE-UI-319` | `POST` | Every panel control MUST resolve to the local command it names. | this spec section 4 | `ambre_toile::tests::chrome_ui::presenter_nav_buttons_fire_intents`; `ambre_toile::tests::chrome_ui::presenter_broadcast_button_fires_toggle`; `ambre_toile::tests::chrome_ui::presenter_follow_button_fires_with_peer` | none | verified |
| `TOILE-FEAT-094` | `POST` | Leaving presentation mode MUST stop this viewer's broadcast. | this spec section 3 | `ambre_toile::logic::convergence_tests::leaving_the_presentation_stops_the_broadcast` -- after the exit the viewer is not broadcasting, says so, and its presence frame names no slide | none | verified |
| `TOILE-FEAT-095` | `FRAME` | Entering or leaving presentation mode MUST NOT write to the document. | this spec section 3 | `ambre_toile::logic::convergence_tests::present_mode_toggle_is_local`; `ambre_toile::logic::convergence_tests::present_broadcast_toggle_is_local` | none | verified |

* **Budget.** the transition frame under 4 ms at 500 elements, the slide-transition row of
  `ambre-spec-budgets.md section 1`. A transition frame is a full repaint and a fresh cull. It costs a frame
  of the board plus one pass over the two slides.
* **Boundary.** one outbound flow, and only while the presenter broadcasts: the slide index rides an
  ephemeral presence frame, which `ZK-005` seals with the `DocumentKey`. The deck and the notes are
  document data like any other, sealed by `ZK-001`.
* **Verification.** 45 `verified`, 0 `failing`, 0 `untested`. What holds is the
  machinery, the best-covered subject of the application: convergence, playback, follow, the solver
  endpoints, the camera and both panels. The presenter relay and the board locked while it presents
  hold too. What is loose is behaviour no row rules on yet, listed in section 6.

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
| deletes a frame the deck names | presents | the order, without that frame | one slide fewer, and one undo brings the frame and its slide back together (`TOILE-FEAT-027`) | `logic::convergence_tests::deleting_a_frame_drops_its_slide` |
| appends to a note her panel shows | prefixed the note a moment before | both edits, each where it was typed | Alice's keystroke at the end, and her panel told the merged note (`TOILE-FEAT-088`) | `logic::convergence_tests::a_splice_measured_against_a_moved_note_lands_where_it_was_typed` |

---

## 3. MODEL

**Containers.** The `presentation_order` list of frame ids and the `presentation_config` map, both
root containers; and one `notes` text per frame element (`ambre-spec-toile-schema.md section 3`).
The order names frames by their minted id, never by position, so reordering slides and editing a
frame never break the deck. A root container is created by no peer, so two peers ordering the deck
at the same moment write into one list (`TOILE-029`).

**Playback is a field, not a container.** The active index and the focus flag live on the engine's
own state and reach no peer (`TOILE-FEAT-001`).

**Commands** (`ambre-bus`): `SetSlideOrder { order }` writes the deck. `NextSlide`, `PrevSlide` and
`GotoSlide(index)` move this viewer; `EnterPresent { focus }` and `ExitPresent` toggle the mode.
`SetPresentBroadcast(on)` and `FollowPresenter(who)` set the two roles; `EditNotes { frame, base, delta }`
splices one frame's notes. `base` is the note the splice was measured against. Logic merges it into
a note a peer moved meanwhile, and answers `NotesChanged` when the result differs from the panel's.

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

**Steps, when a frame is deleted:** the deck is pruned inside the same write. The deletion and the slides it
drops are one undo item, and can never be seen apart (`TOILE-FEAT-027`). A board with no deck writes nothing:
deleting a note must not seed a presentation. A deck the panel is showing is then re-read, because the panel
holds no order of its own (`TOILE-FEAT-098`, `TOILE-UI-343`).

**Steps, when this viewer moves to a slide:**

1. clear any follow, so a manual navigation takes control (`TOILE-FEAT-003`);
2. clamp the index to the deck, capture the two frame snapshots and the target frame's rect, and
   emit them as one event;
3. attach an ephemeral frame carrying the index when this viewer broadcasts, and nothing otherwise.

**Steps, when a transition frame is drawn:**

1. read the phase from the frame clock, and ease it once for the camera and the content alike;
2. glide the camera from the frame it started on toward the fitted target (`TOILE-UI-084`);
3. tween each element the two slides share by id. Fade in what only the new slide holds, and fade out what
   only the old one holds (`TOILE-FEAT-004`);
4. stop at the raw phase, so the transition ends exactly on its length.

**Membership is declared, never spatial.** A slide's elements are the frame's own children, not
whatever overlaps its box. An element in no frame never animates.

**Code:**

* `crates/ambre-toile/src/logic/state/presentation.rs` -- the deck, playback, the roles, the notes
  and the export pages;
* `logic/animate.rs` -- the tween, the two fades and the easing curve;
* `logic/state/derive.rs` -- `frame_snapshot`, the capture the solver consumes;
* `ui/present.rs` -- the transition, one camera and one override set per frame;
* `ui/presenter.rs` and `chrome/presenter.rs` -- the counter, the note splice and the panel;
* `ui/deck.rs` and `chrome/deck.rs` -- the deck label, the three order edits and the ordering panel.

---

## 4. SURFACE

| Trigger | Behaviour |
|---|---|
| the File or Present menu entry | enters presentation mode on the first slide (`TOILE-UI-087`) |
| the Present menu's deck entry | opens the deck panel on the board's frames (`TOILE-FEAT-026`) |
| a click on a frame under "Frames" | appends it to the deck, once (`TOILE-FEAT-002`) |
| a deck row's up / down button | swaps that slide with its neighbour, as one undo item (`TOILE-FEAT-077`) |
| a deck row's remove button | drops the slide, and leaves the frame on the board |
| a click on a deck row | moves this viewer to that slide (`TOILE-FEAT-081`) |
| typing in a row's title field | names that frame, deck member or not (`features/ambre-spec-toile-frames.md`, `TOILE-247`) |
| the panel's previous and next buttons | move this viewer one slide, and break any follow |
| the panel's exit button | leaves presentation mode, and stops broadcasting |
| the panel's broadcast button | starts or stops sharing this viewer's slide with peers |
| the panel's follow button | adopts, or stops adopting, the broadcasting peer's slide (`TOILE-UI-089`) |
| typing in the panel's notes area | splices that slide's notes (`TOILE-UI-088`) |
| a slide change | glides the camera and animates the content until the transition ends |

**The panel** is chrome, not a painted overlay: it is a floating Leptos component, like the find panel
(`features/ambre-spec-toile-find.md section 4`). It opens with presentation mode and shows the slide counter,
the five controls and the notes editor. The follow button is shown while the presenter signal names a peer.
The shell reads it off the whole live presence set, so a peer that stops broadcasting, leaves or ages out
stops being offered (`TOILE-UI-315`).

**The notes editor sends a splice, never a body.** A text area only reports its whole value. So the panel
trims the common head and tail, and sends what is left as one insert and one delete (`TOILE-UI-088`). That is
what a keystroke and a paste each produce, and it is what makes two authors converge instead of overwriting
each other (`TOILE-FEAT-008`).

**The deck panel is where a frame is named.** Each row's title is an editable field, on both lists, so
a frame off the deck is named as easily as a slide. The field carries the frame's own name and shows
its rank as a placeholder. One edit leaves as one command, never one per keystroke (`TOILE-247`,
`TOILE-248`).

**The deck panel** is the second floating component of the subject, opened from the Present menu. It is
independent of presentation mode: a user builds the deck before presenting it. It lists the deck in playback
order, each row carrying its rank, its two reorder steps and its removal. Then the board's other frames, each
addable at the end. A frame the user never titled reads by its rank, so a deck of unnamed frames is still
navigable (`TOILE-UI-342`).

**The panel answers with an order, never with a patch.** Each edit is computed over the order the view last
read back and sent whole, which is the only shape `SetSlideOrder` takes. An edit the model refuses -- a frame
the deck names, a rank past its end -- sends nothing and costs no undo item (`TOILE-UI-343`).

**Focus mode drops every editing surface** (`TOILE-UI-083`). The content pass swaps the grid for a plain
background. The overlay pass is told the mode, and skips the gesture preview, the selection box, the handles,
the find hits and the presence cursors. The link badges stay, because following a link is navigation. The
editing chrome stays mounted and the board stays live: a key that deletes still deletes.

---

## 5. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| A slide is a frame of the same board | a linear deck of its own | it duplicates every element that appears twice and animates nothing. A path through the work needs no second document, no copy and no export step, and it is what makes the transition free: two slides that share an element share its id |
| The order is shared and the playback is not | broadcasting the position as document history | it writes a delta per keystroke of navigation and cannot be left. The group agrees on the story, but each viewer looks where they want, and a reader who scrolls back must not drag everyone with them; follow-the-presenter is the opt-in that puts them together again |
| The presenter's position rides the presence frames | a shared register naming the active slide | it persists a fact nobody wants back. The position is a gesture, not a decision: momentary, per viewer, meaningless an hour later, and the frame that carries a cursor carries it for nothing |
| A transition matches elements by their minted id | matching by position, size or shape | it guesses, and guesses differently on each board. The id match is exact and costs nothing: the author moved the element, so the two snapshots hold the same id and the tween writes itself |
| The deck names frames, never positions | an index into the paint order | the first reorder invalidates it, while an id survives a move, a reorder and a regroup, so editing the board never rewrites the deck |
| The solved transition is derived per frame, never stored | writing the interpolated geometry back | it puts a frame of animation in everyone's history. Derived, the animation is a view of two states the document already holds, so a peer that joins mid-transition sees the board and not a half-played artefact |
| Speaker notes are a character-wise text seeded at frame birth | seeding on the first edit | it is the race `TOILE-029` records for the deck itself: two authors write notes on the same slide, a lazily created container mints two rivals and one side is dropped |

---

## 6. NOT BUILT, AND NOT YET RULED

* **Two of the deck's three configuration fields are read by nobody.** The transition length rides
  the slide event now (`TOILE-FEAT-028`); neither autoplay nor loop has any code at all.
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
* **A keystroke with no frame is dropped in silence** (`TOILE-FEAT-091`). On an empty deck the panel
  names no frame, so the shell refuses the edit intent before it reaches the engine. Nothing tells
  the user that what they typed went nowhere, which is the same hole as entering an empty deck at
  all.
