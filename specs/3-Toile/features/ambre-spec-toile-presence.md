# AMBRE -- FEATURE: LIVE PRESENCE ON THE BOARD

**Type:** Vertical (feature)
**Implementation:** implemented

> A user sees where the others are: a pointer in their colour, and the elements they hold. Also the
> stroke they are drawing and the trail of their highlighter.

Presence is a gesture, not a decision. It is relayed and forgotten: nothing of it reaches the
document, the history or a file.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`.

The behaviour is horizontal and is cited, never restated. The transport, the identity resolution, the keying
by `UserId` and the shared colour are `1-Foundations/ambre-spec-presence.md`, `PRESENCE-001` to
`PRESENCE-010`. So are the whole-set event and the drop of an unresolvable anchor. This document holds Toile's
own half: the board anchor, what the overlay paints, and the trail the receiver accumulates. Trame's slice is
`2-Trame/features/ambre-spec-trame-selection.md section 3`.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TOILE-016` | `INV` | A cell of presence MUST NOT reach the document, the history or a file: it is relayed and forgotten. | this spec section 3; `SYNC-014` | `ambre_toile::logic::convergence_tests::apply_ephemeral_keys_presence_by_user_id_without_touching_history` -- the snapshot and the element count are the same after an inbound sample; `ambre_toile::logic::convergence_tests::presence_command_broadcasts_ephemeral_only` -- an outbound one carries no delta | none | verified |
| `TOILE-177` | `INV` | A sample MUST be keyed by the identity the transport resolved, never by anything the payload carries: no peer can paint under another user's name. | this spec section 3; `SYNC-015` | structural -- the payload declares no identity field at all (section 3); `ambre_toile::logic::convergence_tests::apply_ephemeral_keys_presence_by_user_id_without_touching_history` -- it tracks two senders independently under the ids it is handed. The resolution itself is `PRESENCE-002` | none | verified |
| `TOILE-178` | `INV` | A sample MUST replace the peer's presence whole: a field a sender leaves out is a field the receiver clears, not one it keeps. | this spec section 3 | `ambre_toile::logic::convergence_tests::a_sample_replaces_the_peers_presence_whole` -- it sends a bare cursor after a sample carrying a selection and a laser, and reads both back gone | none | verified |
| `TOILE-185` | `INV` | The client MUST NOT keep a departed peer's presence: the sample, and the in-flight stroke it carries, MUST be dropped. | this spec section 3; `1-Foundations/ambre-spec-presence.md`, `PRESENCE-014` | `ambre_toile::logic::convergence_tests::a_departure_message_drops_the_peer_at_once` -- the relay's word; `ambre_toile::logic::convergence_tests::a_peer_who_stops_sending_leaves_the_live_set` -- the timeout under it. Both read the entry gone from the engine and from the set the event carries; `ambre_toile::ui::presence::tests::a_peer_absent_from_the_set_stops_being_painted` -- the overlay, trail included | none | verified |
| `TOILE-UI-014` | `OBS` | Every peer MUST be painted in their own colour, the same colour that peer carries in the other applications. | this spec section 4 | `ambre_toile::ui::presence::tests::two_peers_get_distinct_colours` -- two peers fill in two colours, the colour being `ambre_draw::peer_color` over the resolved id, which `PRESENCE-006` holds across applications | none | verified |
| `TOILE-UI-216` | `OBS` | A peer's pointer MUST be painted as an arrow with a label chip beside it. | this spec section 4 | `ambre_toile::ui::presence::tests::cursor_draws_arrow_and_label_chip` -- the filled arrow, the chip and the one glyph run | none | verified |
| `TOILE-UI-078` | `OBS` | A selection ghost MUST outline only the elements the board paints this frame. | this spec section 4 | `ambre_toile::ui::presence::tests::selection_ghosts_outline_visible_elements_only` -- one outline for a selection of two, one of which is culled out | none | verified |
| `TOILE-UI-217` | `OBS` | A peer's stroke MUST be painted while it is still in flight, before it commits. | this spec section 4 | `ambre_toile::ui::presence::tests::live_stroke_draws_a_polyline` -- the one polyline. Nothing asserts it disappears when the stroke commits, which is beyond what the rule claims, and no tool streams one anyway (`TOILE-UI-073`) | none | verified |
| `TOILE-UI-077` | `OBS` | A laser trail MUST fade to nothing once the peer stops moving the highlighter. | this spec section 4 | `ambre_toile::ui::presence::tests::a_still_highlighter_fades_out` -- it paints one trail at its own clock and again a fade later, and reads no segment at all the second time | none | verified |
| `TOILE-UI-218` | `OBS` | A board with no peer on it MUST paint no overlay at all. | this spec section 4 | `ambre_toile::ui::presence::tests::empty_paints_nothing_and_is_empty` -- the empty draw log, and the pass leaves no transform behind | none | verified |
| `TOILE-UI-221` | `OBS` | Two peers MUST be painted in a stable order, so an overlap does not flicker between frames. | this spec section 4 | structural -- the pass sorts the peers by their resolved id before painting, so the order is a function of the set and not of a hash iteration | none | verified |
| `TOILE-UI-219` | `OBS` | Presence MUST be painted above every element and below the reaction badges, so a chip is never hidden by a ghost. | this spec section 4; `features/ambre-spec-toile-reactions.md` | `ambre_toile::ui::view::view_tests::the_overlays_stack_peers_then_badges_then_pins` -- no peer op in the board pass, and every peer op before the first badge | none | verified |
| `TOILE-UI-220` | `INV` | The overlay MUST hold no peer from a board that is no longer open. | this spec section 4 | `ambre_toile::ui::view::view_tests::opening_another_board_drops_its_peers` | none | verified |
| `TOILE-UI-075` | `OBS` | A peer who leaves the board MUST stop being painted. | this spec section 4 | `ambre_toile::ui::presence::tests::a_peer_absent_from_the_set_stops_being_painted` -- it adopts a two-peer set, then a one-peer set, then an empty one, and counts what each paints | none | verified |
| `TOILE-UI-076` | `OBS` | A peer's label MUST be the display name their session carries. | this spec section 4 | `ambre_toile::ui::presence::tests::the_label_is_the_name_the_sample_carried` -- the reader, over a name; `ambre_toile::logic::convergence_tests::the_outbound_sample_carries_the_session_display_name` -- the writer. That a label decides neither identity nor colour is `PRESENCE-004` | none | verified |
| `TOILE-UI-367` | `OBS` | A peer's label MUST fall back to a prefix of their identifier only when they send no display name. | this spec section 4 | `ambre_toile::ui::presence::tests::the_label_is_the_name_the_sample_carried` -- the reader, over an absent name and a blank one | none | verified |

### 1.2 A peer's sample arrives

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TOILE-179` | `POST` | An arriving sample MUST become that peer's latest. | this spec section 3 | `ambre_toile::logic::convergence_tests::apply_ephemeral_keys_presence_by_user_id_without_touching_history` -- reading the peer back returns the sample | none | verified |
| `TOILE-241` | `POST` | An arriving sample MUST be announced once to the surface that paints it. | this spec section 3 | `ambre_toile::logic::convergence_tests::apply_ephemeral_keys_presence_by_user_id_without_touching_history` -- the one event, carrying the sender and the sample | none | verified |
| `TOILE-UI-222` | `POST` | An arriving laser point MUST extend that peer's trail. | this spec section 3 | `ambre_toile::ui::presence::tests::laser_accumulates_a_fading_trail_and_clears_on_lift` -- the segment count after three points | none | verified |
| `TOILE-UI-376` | `POST` | A sample carrying no laser point MUST clear that peer's trail: a lifted highlighter leaves nothing behind. | this spec section 3 | `ambre_toile::ui::presence::tests::laser_accumulates_a_fading_trail_and_clears_on_lift` -- the segment count after the lift | none | verified |
| `TOILE-180` | `FRAME` | An arriving sample MUST NOT move this viewer's camera, selection or slide -- unless the viewer follows the sender, which is the one exception (`features/ambre-spec-toile-presentation.md`). | this spec section 3 | `ambre_toile::ui::view::view_tests::an_arriving_sample_keeps_the_camera_and_the_selection`; `ambre_toile::logic::convergence_tests::non_follower_ignores_broadcast` -- the slide stays; `ambre_toile::logic::convergence_tests::following_other_peer_ignores_broadcast`; `ambre_toile::logic::convergence_tests::follower_adopts_presenter_slide` -- the one exception | none | verified |
| `TOILE-181` | `FAIL` | Given a sample it cannot read, the client MUST drop it and MUST NOT announce anything: presence is best effort, and the next sample corrects it. | this spec section 3 | `ambre_toile::logic::convergence_tests::apply_ephemeral_drops_a_malformed_frame` -- no event comes out of a malformed frame | none | verified |

### 1.3 Broadcast this peer's gesture

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TOILE-182` | `POST` | Broadcasting a gesture MUST produce one ephemeral whatever its fate: presence is not acknowledged. | this spec section 3 | `ambre_toile::logic::convergence_tests::presence_command_broadcasts_ephemeral_only` -- the ephemeral is there | none | verified |
| `TOILE-183` | `FRAME` | Broadcasting MUST NOT write a delta, MUST NOT push an undo item, and MUST NOT ask for a repaint: the local pointer is the browser's, not the board's. | this spec section 3 | `ambre_toile::logic::convergence_tests::presence_command_broadcasts_ephemeral_only` -- the absent delta, the empty display list, and an undo that takes back the edit before the sample | none | verified |
| `TOILE-184` | `FAIL` | Given a sample it cannot encode, the client MUST drop it silently and MUST NOT surface an error: a lost sample is corrected by the next one. | this spec section 3 | `ambre_toile::logic::state::presence::tests::a_sample_that_will_not_encode_is_dropped_silently` -- the encode is generic, so a value that refuses to serialize reaches the failing arm | none | verified |
| `TOILE-UI-073` | `POST` | A gesture on the board MUST broadcast the local peer's cursor, selection and in-flight stroke. | this spec section 3 | `ambre_toile::ui::presence::tests::a_sample_is_sent_on_a_change_or_on_the_heartbeat` -- the schedule alone: a changed sample goes at once, an unchanged one waits for the heartbeat; `ambre_toile::ui::view::view_tests::a_gesture_sample_carries_the_cursor_the_selection_and_the_stroke` -- what one sample carries | none | verified |
| `TOILE-UI-074` | `POST` | The highlighter MUST be a tool the user can pick. | this spec section 4 | `ambre_toile::ui::tool::tests::the_highlighter_streams_a_point_and_writes_nothing` -- it drives a press, two moves and a release through the picked tool and reads a live point on each | none | verified |
| `TOILE-UI-377` | `FRAME` | The highlighter's trail MUST NOT become an element. | this spec section 4 | `ambre_toile::ui::tool::tests::the_highlighter_streams_a_point_and_writes_nothing` -- the same press, moves and release read no command at all | none | verified |

* **Budget.** the presence overlay under 1 ms per frame (`ambre-spec-budgets.md section 2`), held by no bench.
  It is skipped when no peer is known (`TOILE-UI-218`). A board with peers pays a scan of the visible
  set per ghost, section 6, and an alone board pays nothing.
* **Boundary.** every presence sample leaves the client, sealed by `ZK-005`: the relay reads
  `document_id`, `pub_key` and `counter` alone. `ambre-sync` resolves the author first, so no peer
  paints a cursor under another name (`SYNC-014`).
* **Verification.** 28 `verified`. What holds is the round trip: a sample leaves on every gesture
  and on a heartbeat, carrying the cursor, the selection and the stroke. A peer who leaves goes,
  and a ghost carries its session's name. The board view holds the screen side: the pass order,
  the second board, and an arriving sample that moves nothing.

---

## 2. WHAT EACH USER SEES

Presence never reaches the document, so the third column is the same in every row. It is kept because
the absence is the rule.

| Alice does | Bob does | The document holds | Each user sees | Test |
|---|---|---|---|---|
| moves her pointer | watches | nothing | Alice's arrow and handle on Bob's board, in her colour (`TOILE-UI-014`) | `ui::presence::tests::cursor_draws_arrow_and_label_chip` |
| selects two shapes, then moves her pointer | watches | nothing | the two ghosts, then the ghosts gone: a sample replaces the last one whole (`TOILE-178`) | `logic::convergence_tests::a_sample_replaces_the_peers_presence_whole` |
| selects two shapes | watches | nothing | the two shapes outlined in Alice's colour on Bob's board | `ui::presence::tests::selection_ghosts_outline_visible_elements_only` |
| draws a stroke, holding | watches | nothing until the release | the line growing on Bob's board, then one stroke element on both | `ui::presence::tests::live_stroke_draws_a_polyline` |
| sweeps the highlighter | watches | nothing | a trail behind Alice's pointer, fading as she moves and gone when she stops (`TOILE-UI-077`) | `ui::presence::tests::laser_accumulates_a_fading_trail_and_clears_on_lift`, `::a_still_highlighter_fades_out` |
| opens the board from a second device | watches | nothing | one collaborator in one colour, not two (`PRESENCE-005`, `PRESENCE-006`) | `ambre_draw::peer::tests::stable_and_pure` |
| closes the board | watches | nothing | Alice's cursor gone from Bob's board once her samples stop (`TOILE-UI-075`, `TOILE-185`) | `logic::convergence_tests::a_peer_who_stops_sending_leaves_the_live_set`, `ui::presence::tests::a_peer_absent_from_the_set_stops_being_painted` |

---

## 3. MODEL

**Containers.** none (`PRESENCE-001`, `ambre-spec-toile-schema.md section 8`).

**The sample** (`ambre-bus`). One sample is a peer's whole presence, not a patch on the last one. The receiver
replaces what it held, so a field a sender leaves out is a field it clears (`TOILE-178`). The optional fields
are how a peer says *not any more*: pointer off the board, highlighter lifted. They are not how it omits what
has not changed:

```rust
pub struct PresencePayload {
    pub name:          Option<String>,       // the sender's display name, cosmetic (`PRESENCE-004`)
    pub cursor:        Option<Point>,        // world space; None = the pointer left the board
    pub selection:     Vec<ElementId>,       // the peer's selection, drawn as ghosts
    pub live_stroke:   Option<Vec<StrokePoint>>, // an in-flight stroke, before it commits
    pub laser:         Option<Point>,        // one highlighter point; None = lifted
    pub present_slide: Option<usize>,        // the presenter's slide (see the presentation feature)
}
```

**No identity travels in the payload**, and the receiver keys the sample by the sender's resolved
`UserId` (`TOILE-177`, `PRESENCE-002`, `PRESENCE-003`). The board anchor is what is Toile's own:
a world point for the pointer and the laser, and the peer's `ElementId` set for the ghosts. Both
survive a structural change, which is `PRESENCE-007`.

**Command** (`ambre-bus`): `Presence { payload, now_ms }`, non-mutating, answered by an ephemeral: the
local pointer is the browser's, not the board's. `now_ms` is the UI's wall clock, which the Logic thread does
not hold. It never travels, and it is what ages the peers this client knows about. A sender's own clock
decides nothing, so a wrong or hostile one ages nobody.

**Event** (`ambre-bus`): `PeerPresence(Vec<PresenceUpdate>)`, the whole live peer set, which the
view adopts as it stands (`PRESENCE-009`). A departure is an absent entry, so no departure signal
exists anywhere in the workspace.

**Steps, when the local peer gestures:**

1. the view builds the sample from live view state each frame. That is the pointer in world space, the
   selection, the tail of an in-flight stroke and the highlighter point;
2. it sends it when it differs from the last one sent, or when the heartbeat is due. That heartbeat keeps this
   client in the others' live set, and it advances the Logic thread's only clock (`TOILE-UI-073`);
3. the Logic thread stamps the session's display name on it (`TOILE-UI-076`), ages its own peers
   against the clock, and hands the frame to Sync.

**Steps, when a peer leaves** (`PRESENCE-014`):

1. the relay saw them unsubscribe, or their socket close, and says so on the document's topic;
2. `ambre-sync` resolves the key it named through the member directory, exactly as it resolves the author of a
   sample. A key it cannot attribute is dropped;
3. the peer is removed and the set that remains travels to the view.

The timeout stays underneath as the floor. A lost departure costs the few seconds of a stale cursor it cost
before the message existed. No peer is kept for good either way.

**Steps, when a peer's sample arrives:**

1. the frame is decoded and keyed by the `UserId` the transport resolved, never by anything the
   payload carries (`TOILE-177`);
2. it replaces that peer's presence whole and is stamped with the local clock (`TOILE-178`);
3. the whole live set travels to the view, which replaces its mirror with it, trails included
   (`TOILE-185`).

**Steps, when a frame arrives:**

1. decode the sample, and drop the frame on a decode failure -- presence is best effort
   (`TOILE-181`);
2. record it under the sender's `UserId`, replacing that peer's previous sample;
3. emit the event, and fold it into the render thread's mirror -- where the whole live set should
   travel (`PRESENCE-009`);
4. adopt the carried slide when this viewer follows that sender
   (`features/ambre-spec-toile-presentation.md section 3`).

The render thread keeps the trail the payload does not. The sample carries one highlighter point, and the
overlay accumulates the last 24 into a ring buffer.

**Code:**

* `crates/ambre-toile/src/logic/state/presence.rs` -- the broadcast and the inbound apply;
* `ui/presence.rs` -- the per-peer mirror and the whole overlay pass;
* `crates/ambre-logic/src/data.rs` -- the routing of an inbound ephemeral to this application.

---

## 4. SURFACE

The overlay is one pass, above all content, painted in screen space and never culled.

| What | Painted as |
|---|---|
| cursor | a filled arrow at the peer's pointer, plus a label chip in the peer's colour |
| selection | an outline per selected element, in the peer's colour (`TOILE-UI-078`) |
| in-flight stroke | a polyline through the streamed samples, in the peer's colour |
| highlighter | the trail, oldest segment nearly transparent and newest opaque (`TOILE-UI-077`) |

**Order.** Presence paints after the board and before the reaction badges
(`features/ambre-spec-toile-reactions.md`), so a chip is never hidden by a ghost. The pass is skipped
whole when no peer is known.

**The label is a hex handle today.** The overlay prints the first four bytes of the `UserId`, because
nothing hands it the display name the member list already carries (`TOILE-UI-076`).

**What sends nothing.** No gesture emits `Presence`, and no tool produces a highlighter point
(`TOILE-UI-073`, `TOILE-UI-074`). The tool machine that would own the highlighter is
`features/ambre-spec-toile-tools.md`.

---

## 5. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| The anchor is a world point for the pointer and an `ElementId` set for the ghosts | one anchor for both | it either drops the pointer to the nearest element or makes a ghost a rectangle that stops following its shape. The two halves answer two questions: a pointer is between elements as often as on one, so it has no identity to name and a world point is already invariant under everything but a pan, which is the viewer's own; a ghost outlines elements, which do have identities, and those survive a move, a restyle and a regroup (`PRESENCE-007`) |
| A sample is the peer's whole presence, and a missing field means absent | a per-field patch | it needs one presence-or-absence flag per field and makes the receiver's state depend on the order frames arrive in, so two peers can disagree about which of two samples cleared what. Replacing costs the sender the fields that did not change, and buys a state that is exactly the last sample received |
| The trail is accumulated by the receiver | streaming the whole trail in each sample | one point per frame is a fixed sample cost whatever the trail length, and a lost frame shortens the trail instead of breaking it |
| The highlighter never becomes an element | committing the trail as a stroke | it is a pointing gesture during a discussion; an element would enter the history, the undo stack and every export |

---

## 6. NOT BUILT, AND NOT YET RULED

* **A departure is announced, and a stall is not.** The relay names a peer who unsubscribed or whose socket
  closed, so a board someone closes empties at once. What it cannot name is a peer whose network stalled
  without dropping the socket. That one still leaves on the ten-second timeout, and blinks back on the next
  sample.
* **A pointer never leaves the board.** The host raises no pointer-leave event, so `cursor: None` is produced
  by nothing. That is the way the payload says *the pointer is off the board*. A peer's arrow sits where it
  last was until they leave the set. The field is the sender's to fill and no gesture fills it.
* **An in-flight stroke travels as its tail.** A sample rides every gesture frame, and a long stroke
  would resend its whole history each time, so only the last samples travel. A peer joining
  mid-stroke sees the end of it, and the whole stroke when it commits.
* **A ghost costs a scan of the visible set.** The overlay resolves each selected element by walking the
  culled set and comparing string ids. One peer selecting a hundred shapes on a board of two thousand visible
  ones costs two hundred thousand string comparisons per frame. The budget is 1 ms and nothing measures it.
  The board's own passes index what they draw; this one does not.
* **Nothing bounds how often a sample is sent**, and **a viewer's presence is not distinguished
  from an editor's**. Both are the suite's, not this board's, and both are
  `1-Foundations/ambre-spec-presence.md section 6`.
