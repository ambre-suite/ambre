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

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|---|
| `TOILE-FEAT-001` | INV | The slide order and the speaker notes MUST be document data: shared by the CRDT, and undoable. | this spec section 3 | `ambre_toile::logic::convergence_tests::set_slide_order_produces_delta_and_is_undoable`, `::edit_notes_produces_delta_and_is_undoable` | verified |
| `TOILE-FEAT-075` | INV | Playback -- the active slide, the mode, the broadcast role and the follow role -- MUST stay local to one viewer and MUST NOT be synced. | this spec section 3 | `ambre_toile::logic::convergence_tests::present_mode_toggle_is_local`, `::playback_is_not_synced`, `::follow_state_is_not_synced`, `::present_broadcast_toggle_is_local` | verified |
| `TOILE-FEAT-002` | INV | The deck MUST hold a frame at most once. | this spec section 3 | `ambre_toile::logic::convergence_tests::slide_order_dedups_repeated_frames` | verified |
| `TOILE-FEAT-076` | INV | The active index MUST name a slide of the deck, whatever asked for it. | this spec section 3 | `ambre_toile::logic::convergence_tests::playback_bounds`, `::adopted_slide_is_clamped_to_deck` | verified |
| `TOILE-FEAT-008` | CONC | Concurrent speaker-notes edits MUST converge character by character, with no sentence lost. | this spec section 3 | `ambre_toile::logic::convergence_tests::concurrent_notes_edits_converge` | verified |
| `TOILE-FEAT-027` | INV | The deck MUST hold only slides whose frame still exists. | this spec section 3 | none, and **the rule fails today**: deleting a frame never touches the deck, so the dead entry is counted in the deck length and stops the camera when a viewer reaches it. The PDF export skips the same entry (`TOILE-IO-041`), so the two readings of one deck disagree -- section 6 | unverified |
| `TOILE-UI-086` | OBS | The presenter panel MUST show the active slide's rank, the deck's length and that slide's notes. | this spec section 4 | `ambre_toile::ui::presenter::tests::counter_label_is_one_based`, `::counter_label_empty_deck`, `ambre_toile::tests::chrome_ui::presenter_panel_counter_is_one_based`, `::presenter_panel_hidden_when_closed` | verified |
| `TOILE-UI-089` | OBS | The panel MUST show the follow affordance when the presenter signal names a peer, and MUST NOT show it when it names none. | this spec section 4 | `ambre_toile::tests::chrome_ui::presenter_follow_button_hidden_without_presenter`, `::presenter_follow_button_fires_with_peer`. Both mount the panel with that signal set by hand, so they hold the component and never the relay that feeds it, which is `TOILE-UI-315` | verified |
| `TOILE-UI-315` | OBS | The presenter signal MUST name a peer only while that peer is broadcasting a slide. | this spec section 4 | none, and **the rule fails today**: the shell arms the signal on the first presence sample carrying a slide index and never clears it, so the follow affordance survives the presenter stopping, leaving and disconnecting. No test drives the relay -- section 6 | unverified |
| `TOILE-UI-316` | OBS | The notes area MUST show a peer's edit to the note on screen. | this spec section 4 | none, and **the rule fails today**: the editor is seeded from the slide event and no event reports a remote note edit, so a co-author's writing stays invisible until the slide changes -- section 6 | unverified |
| `TOILE-UI-083` | OBS | Presentation mode MUST hide every editing surface. | this spec section 4 | none, and **the rule fails today**: the content pass swaps the grid for a plain background and does nothing else, and the overlay pass is never told the mode at all, so the selection box, the handles, the find hits and the presence cursors keep painting over the slide. The chrome stays mounted and the board stays live -- section 6 | unverified |
| `TOILE-UI-085` | OBS | An element the next slide drops MUST fade out over the whole transition. | this spec section 4 | none, and **the rule fails today**: the paint pass reads an override only for an element the visible set holds, so an element the gliding camera leaves stops mid-fade -- section 6 | unverified |

### 1.2 Order the deck

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|---|
| `TOILE-FEAT-077` | POST | Writing the deck MUST leave the order the caller named, each frame at its first place, as one undo item. | this spec section 3 | `ambre_toile::logic::convergence_tests::set_slide_order_produces_delta_and_is_undoable`, `::slide_order_dedups_repeated_frames` | verified |
| `TOILE-FEAT-078` | POST | Writing the order already held MUST produce neither a commit nor a delta. | this spec section 3 | none: `::set_slide_order` returns early on an unchanged order, and no test asks for the same order twice | unverified |
| `TOILE-FEAT-079` | POST | Writing a shorter deck MUST clamp this viewer's active index to the new length. | this spec section 3 | none: `::set_slide_order` clamps after the commit, and no test shortens a deck under a viewer standing past its new end | unverified |
| `TOILE-FEAT-080` | FRAME | Writing the deck MUST NOT touch an element, a membership or a note. | this spec section 3 | structural, and the construction is the proof: `::set_slide_order` clears and re-pushes the order list alone, and reaches no element map, no children list and no notes container | verified |
| `TOILE-FEAT-026` | POST | A user MUST be able to add a frame to the deck, reorder it and remove it. | this spec section 4 | not built -- lands with the "Order the deck" Story (`ambre-roadmap.md` E4) | unimplemented |

### 1.3 Move to a slide

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|---|
| `TOILE-FEAT-081` | POST | Moving to a slide MUST emit one event carrying the new index, both slides' snapshots, the camera target, the deck length, and the target frame with its notes. | this spec section 3 | structural, and the type is the proof: `SlideChange` carries exactly those seven fields, and `::slide_change` fills every one of them on each move | verified |
| `TOILE-FEAT-082` | POST | Moving to a slide by hand MUST clear any follow, so the viewer takes control. | this spec section 3 | `ambre_toile::logic::convergence_tests::manual_nav_breaks_follow` | verified |
| `TOILE-FEAT-083` | FRAME | Moving to a slide MUST NOT write to the document. | this spec section 3 | `ambre_toile::logic::convergence_tests::playback_is_not_synced` | verified |

### 1.4 Follow a presenter

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|---|
| `TOILE-FEAT-003` | POST | A follower MUST adopt the presenter's broadcast slide. | this spec section 3 | `ambre_toile::logic::convergence_tests::follower_adopts_presenter_slide`, `::adopted_slide_is_clamped_to_deck` | verified |
| `TOILE-FEAT-084` | POST | Starting to follow MUST snap this viewer to the presenter's last known slide, without waiting for the next broadcast. | this spec section 3 | `ambre_toile::logic::convergence_tests::follow_snaps_to_last_known_slide` | verified |
| `TOILE-FEAT-085` | FRAME | A broadcast from a peer this viewer does not follow MUST change nothing. | this spec section 3 | `ambre_toile::logic::convergence_tests::non_follower_ignores_broadcast`, `::following_other_peer_ignores_broadcast` | verified |

### 1.5 Broadcast this viewer's position

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|---|
| `TOILE-FEAT-086` | POST | While this viewer broadcasts, each slide change MUST attach an ephemeral presence frame carrying the active index. | this spec section 3 | `ambre_toile::logic::convergence_tests::broadcast_frame_carries_active_slide` | verified |
| `TOILE-FEAT-087` | FRAME | While this viewer does not broadcast, a slide change MUST attach no frame. | this spec section 3 | `ambre_toile::logic::convergence_tests::no_broadcast_frame_when_not_broadcasting` | verified |

### 1.6 Write a speaker note

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|---|
| `TOILE-UI-088` | POST | A note edit MUST reach the document as the one smallest splice that explains it. | this spec section 4 | `ambre_toile::ui::presenter::tests::delta_none_on_no_change`, `::delta_pure_append`, `::delta_pure_prefix_insert`, `::delta_pure_delete`, `::delta_mid_string_replace`, `::delta_from_empty`, `::delta_to_empty`, `::delta_multibyte_indices_are_codepoints`, `::delta_multibyte_mid_replace`, `::delta_repeated_char_prefers_prefix`, `ambre_toile::tests::chrome_ui::presenter_notes_edit_fires_minimal_delta` | verified |
| `TOILE-FEAT-088` | PRE | The caller MUST offer a splice measured against the note the document holds now. | this spec section 4 | nothing checks it, and the panel does not honour it: `ui::presenter::notes_delta` diffs the typed value against a mirror the slide event alone refreshes, so a peer's edit between two keystrokes puts the offsets in the wrong place. `TOILE-FEAT-008` holds the merge, never the offsets -- section 6 | unverified |
| `TOILE-FEAT-089` | FAIL | A splice reaching past the end of the note MUST be clamped, not dropped. | this spec section 3 | `ambre_toile::logic::convergence_tests::edit_notes_clamps_out_of_range_splice` | verified |
| `TOILE-FEAT-090` | FAIL | A note edit naming an element that is not a frame MUST be refused. | this spec section 3 | `ambre_toile::logic::convergence_tests::edit_notes_rejects_non_frame` | verified |
| `TOILE-FEAT-091` | FAIL | A note edit made with no frame on screen MUST be refused before it reaches the document. | this spec section 4 | none: the shell drops the intent when the panel names no frame, which is every keystroke on an empty deck, and nothing drives that path -- section 6 | unverified |
| `TOILE-FEAT-092` | FRAME | A note edit MUST touch that frame's notes and nothing else. | this spec section 3 | structural, and the construction is the proof: `::edit_notes` resolves the named frame's notes container and splices it; it reaches no other element and no other container | verified |

### 1.7 Draw one frame of a transition

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|---|
| `TOILE-FEAT-004` | POST | A transition MUST tween the elements two slides share, fade in the ones only the new slide holds, and fade out the ones only the old one holds. | this spec section 3 | `ambre_toile::logic::animate::tests::shared_id_tweens_at_midpoint`, `::new_element_fades_in`, `::gone_element_fades_out`, `::mixed_membership_classifies_each_id`, `::empty_frames` | verified |
| `TOILE-FEAT-093` | POST | A transition MUST reproduce each slide exactly at its own end. | this spec section 3 | `ambre_toile::logic::animate::tests::endpoints_reproduce_source_and_target` | verified |
| `TOILE-UI-084` | POST | The camera MUST reach the active frame at the end of a transition. | this spec section 4 | `ambre_toile::ui::present::tests::end_frame_lands_on_target_camera_and_content`, `::zero_duration_snaps_immediately` | verified |
| `TOILE-UI-317` | POST | The camera MUST hold the previous slide at the start of a transition, and move between the two ends only. | this spec section 4 | `ambre_toile::ui::present::tests::start_frame_holds_source_camera_and_content`, `::midpoint_tweens_between_endpoints` | verified |
| `TOILE-FEAT-028` | POST | A transition MUST last the time the deck's own configuration carries. | this spec section 3 | none, and **the rule fails today**: the render thread eases over a fixed 300 ms constant, and nothing reads the three configuration fields the deck writes at birth -- section 6 | unverified |
| `TOILE-UI-318` | FRAME | Drawing a transition frame MUST NOT write to the document. | this spec section 4 | structural, and the absence is the proof: `ui::present::Transition` holds two snapshots and a clock, and `::advance` has no document handle and no command sink, so a played frame cannot be written back | verified |

### 1.8 Enter and leave the presentation

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|---|
| `TOILE-UI-087` | POST | The Present menu entry MUST enter presentation mode on the first slide. | this spec section 4 | `ambre_toile::ui::menu_bar::tests::present_entry_emits_enter_present` | verified |
| `TOILE-UI-319` | POST | Every panel control MUST resolve to the local command it names. | this spec section 4 | `ambre_toile::tests::chrome_ui::presenter_nav_buttons_fire_intents`, `::presenter_broadcast_button_fires_toggle`, `::presenter_follow_button_fires_with_peer` | verified |
| `TOILE-FEAT-094` | POST | Leaving presentation mode MUST stop this viewer's broadcast. | this spec section 3 | none: `::exit_present` clears the broadcast flag, and `::present_mode_toggle_is_local` asserts the mode is local, never that the role is dropped with it | unverified |
| `TOILE-FEAT-095` | FRAME | Entering or leaving presentation mode MUST NOT write to the document. | this spec section 3 | `ambre_toile::logic::convergence_tests::present_mode_toggle_is_local`, `::present_broadcast_toggle_is_local` | verified |

* **Budget.** the transition frame under 4 ms at 500 elements, the slide-transition row of
  `ambre-spec-budgets.md section 1`. A transition frame is a full repaint and a fresh cull, so
  it costs a frame of the board plus one pass over the two slides.
* **Boundary.** one outbound flow, and only while the presenter broadcasts: the slide index rides an
  ephemeral presence frame, which `ZK-005` seals with the `DocumentKey`. The deck and the notes are
  document data like any other, sealed by `ZK-001`.
* **Verification.** 29 `verified`, 11 `unverified`, 1 `unimplemented`. Fifteen rows became
  forty-one and the tests they cite did not move: of the nineteen new `verified` rows, four are
  structural -- one construction, one type, one absence and one more construction -- and fifteen
  cite a test already cited here, for a rule that test asserts literally. No row below is new
  coverage. The machinery remains the best-covered subject of the application: convergence,
  playback, follow, the solver endpoints, the camera and the panel are each held. The eleven gaps
  are three kinds: six rules the board answers with something else, four the code holds and no test
  drives, and one the panel owes its own splice and does not honour.

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

**What this table cannot see.** Every row is one peer's document beside another's, and three of the
subject's rules are neither: the panel tests mount the component with the presenter signal set by
hand, so nothing here drives the relay that arms it (`TOILE-UI-315`); no event tells the editor that
a peer wrote in the note on screen (`TOILE-UI-316`); and the splice a keystroke sends is measured
against a mirror only a slide change refreshes (`TOILE-FEAT-088`), which the convergence row above
cannot show because both writers there splice against the text they hold.

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
slide counter, the five controls and the notes editor. The follow button is shown while the
presenter signal names a peer -- and the shell arms that signal on the first slide any peer
broadcasts, then never clears it (`TOILE-UI-315`).

**The notes editor sends a splice, never a body.** A text area only reports its whole value, so the
panel trims the common head and tail and sends what is left as one insert and one delete
(`TOILE-UI-088`). That is what a keystroke and a paste each produce, and it is what makes two authors
converge instead of overwriting each other (`TOILE-FEAT-008`).

**What no surface offers** (`TOILE-FEAT-026`). Nothing adds a frame to the deck, reorders it or
removes it: the write command exists and its only callers are tests. Presentation mode therefore
opens on an empty deck, the counter reads `0 / 0`, and the notes editor has no frame to write to.

**Focus mode drops the grid and nothing else** (`TOILE-UI-083`). The content pass swaps the grid for
a plain background; the overlay pass is never told the mode at all, so the selection box, the
handles, the find hits and the presence cursors keep painting over the slide. The editing chrome
stays mounted and the board stays live under the presentation: a key that deletes still deletes.

---

## 5. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| A slide is a frame of the same board | a linear deck of its own | it duplicates every element that appears twice and animates nothing. A path through the work needs no second document, no copy and no export step, and it is what makes the transition free: two slides that share an element share its id |
| The order is shared and the playback is not | broadcasting the position as document history | it writes a delta per keystroke of navigation and cannot be left. The group agrees on the story, but each viewer looks where they want, and a reader who scrolls back must not drag everyone with them; follow-the-presenter is the opt-in that puts them together again |
| The presenter's position rides the presence channel | a shared register naming the active slide | it persists a fact nobody wants back. The position is a gesture, not a decision: momentary, per viewer, meaningless an hour later, and the channel that carries a cursor carries it for nothing |
| A transition matches elements by their minted id | matching by position, size or shape | it guesses, and guesses differently on each board. The id match is exact and costs nothing: the author moved the element, so the two snapshots hold the same id and the tween writes itself |
| The deck names frames, never positions | an index into the paint order | the first reorder invalidates it, while an id survives a move, a reorder and a regroup, so editing the board never rewrites the deck |
| The solved transition is derived per frame, never stored | writing the interpolated geometry back | it puts a frame of animation in everyone's history. Derived, the animation is a view of two states the document already holds, so a peer that joins mid-transition sees the board and not a half-played artefact |
| Speaker notes are a character-wise text seeded at frame birth | seeding on the first edit | it is the race `TOILE-029` records for the deck itself: two authors write notes on the same slide, a lazily created container mints two rivals and one side is dropped |

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
  camera target. The PDF export skips the same entry (`TOILE-IO-041`), so the two readings of one
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
* **The follow affordance never goes away** (`TOILE-UI-315`). The shell names a presenter the first
  time a presence sample carries a slide index, and no path writes that name back to none: not the
  presenter stopping the broadcast, not leaving the presentation, not leaving the document. The
  button therefore offers to follow a peer who is no longer presenting, and following one sends this
  viewer to whatever slide that peer was last seen on. The two panel tests set the signal by hand,
  so neither the arming nor the clearing is exercised anywhere.
* **A peer's note edit is invisible until the slide changes** (`TOILE-UI-316`). The editor is seeded
  from the slide event and refreshed by nothing else; no display event reports a remote splice. Two
  authors on one slide converge in the document (`TOILE-FEAT-008`) and each keeps reading their own
  last keystroke.
* **The splice is measured against a mirror, not against the note** (`TOILE-FEAT-088`). The panel
  diffs the value the text area reports against the string it last received, and that string is
  refreshed only by a slide change. A peer writing at the same moment moves the text under the diff,
  so the insert and the delete land at offsets that no longer mean what they meant. Convergence
  survives -- the CRDT merges whatever it is given -- and the sentence does not.
* **A keystroke with no frame is dropped in silence** (`TOILE-FEAT-091`). On an empty deck the panel
  names no frame, so the shell discards the edit intent before it reaches the engine. Nothing tells
  the user that what they typed went nowhere, which is the same hole as entering an empty deck at
  all.
* **Three behaviours the code holds and nothing drives** (`TOILE-FEAT-078`, `TOILE-FEAT-079`,
  `TOILE-FEAT-094`). Writing the order already held returns before the commit, writing a shorter
  deck clamps the viewer's index afterwards, and leaving the presentation clears the broadcast role.
  All three are one line of the engine and none of them is asserted: the deck tests write one order
  once, and the mode test asks only that the toggle stay local.
