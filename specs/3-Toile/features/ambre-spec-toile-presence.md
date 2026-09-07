# AMBRE -- FEATURE: LIVE PRESENCE

**Type:** Vertical (feature)
**Implementation:** implemented.

> A user sees where the others are: a pointer in their colour, the elements they hold, the stroke
> they are drawing and the trail of their highlighter.

Presence is a gesture, not a decision. It is relayed and forgotten: nothing of it reaches the
document, the history or a file.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`. The behaviour is horizontal and is cited,
never restated: the transport, the identity resolution, the keying by `UserId`, the shared colour,
the whole-set event and the drop of an unresolvable anchor are
`1-Foundations/ambre-spec-presence.md`, `PRESENCE-001` to `PRESENCE-010`. This document holds
Toile's own half: the board anchor, what the overlay paints, and the trail the receiver
accumulates. Trame's slice is `2-Trame/features/ambre-spec-trame-selection.md section 3`.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|---|
| `TOILE-016` | INV | A cell of presence MUST NOT reach the document, the history or a file: it is relayed and forgotten. | this spec section 3; `SYNC-014` | `ambre_toile::logic::convergence_tests::apply_ephemeral_keys_presence_by_user_id_without_touching_history` asserts the snapshot and the element count are the same after an inbound sample; `::presence_command_broadcasts_ephemeral_only` asserts an outbound one carries no delta | verified |
| `TOILE-177` | INV | A sample MUST be keyed by the identity the transport resolved, never by anything the payload carries: no peer can paint under another user's name. | this spec section 3; `SYNC-015` | the payload declares no identity field at all (section 3), and `ambre_toile::logic::convergence_tests::apply_ephemeral_keys_presence_by_user_id_without_touching_history` tracks two senders independently under the ids it is handed. The resolution itself is `PRESENCE-002` | verified |
| `TOILE-178` | INV | A sample MUST replace the peer's presence whole: a field a sender leaves out is a field the receiver clears, not one it keeps. | this spec section 3 | `ambre_toile::logic::convergence_tests::a_sample_replaces_the_peers_presence_whole`, which sends a bare cursor after a sample carrying a selection and a laser, and reads both back gone | verified |
| `TOILE-185` | INV | The client MUST NOT keep a departed peer's presence: the sample, and the in-flight stroke it carries, MUST be dropped. | this spec section 3 | `ambre_toile::logic::convergence_tests::a_peer_who_stops_sending_leaves_the_live_set`, which ages a peer past the TTL and reads the entry gone from the engine and from the set the event carries | verified |
| `TOILE-UI-014` | OBS | Every peer MUST be painted in their own colour, the same colour that peer carries in the other applications. | this spec section 4 | `ambre_toile::ui::presence::tests::two_peers_get_distinct_colours` asserts two peers fill in two colours; the colour is `ambre_draw::peer_color` over the resolved id, held across applications by `PRESENCE-006` | verified |
| `TOILE-UI-216` | OBS | A peer's pointer MUST be painted as an arrow with a label chip beside it. | this spec section 4 | `ambre_toile::ui::presence::tests::cursor_draws_arrow_and_label_chip` asserts the filled arrow, the chip and the one glyph run | verified |
| `TOILE-UI-078` | OBS | A selection ghost MUST outline only the elements the board paints this frame. | this spec section 4 | `ambre_toile::ui::presence::tests::selection_ghosts_outline_visible_elements_only` asserts one outline for a selection of two, one of which is culled out | verified |
| `TOILE-UI-217` | OBS | A peer's stroke MUST be painted while it is still in flight, before it commits. | this spec section 4 | `ambre_toile::ui::presence::tests::live_stroke_draws_a_polyline` asserts the one polyline. Nothing asserts it disappears when the stroke commits, because no tool streams one (`TOILE-UI-073`) | verified |
| `TOILE-UI-077` | OBS | A laser trail MUST fade to nothing once the peer stops moving the highlighter. | this spec section 4 | `ambre_toile::ui::presence::tests::a_still_highlighter_fades_out`, which paints one trail at its own clock and again a fade later, and reads no segment at all the second time | verified |
| `TOILE-UI-218` | OBS | A board with no peer on it MUST paint no overlay at all. | this spec section 4 | `ambre_toile::ui::presence::tests::empty_paints_nothing_and_is_empty` asserts the empty draw log and that the pass leaves no transform behind | verified |
| `TOILE-UI-221` | OBS | Two peers MUST be painted in a stable order, so an overlap does not flicker between frames. | this spec section 4 | structural, and the construction is the proof: the pass sorts the peers by their resolved id before painting, so the order is a function of the set and not of a hash iteration | verified |
| `TOILE-UI-219` | OBS | Presence MUST be painted above every element and below the reaction badges, so a chip is never hidden by a ghost. | this spec section 4; `features/ambre-spec-toile-reactions.md` | none. The overlay pass sits between the two in the frame, and no test reads the order of the three passes | unverified |
| `TOILE-UI-220` | INV | The overlay MUST hold no peer from a board that is no longer open. | this spec section 4 | none. The document swap clears the mirror, and no test opens a second board with a peer painted on the first | unverified |
| `TOILE-UI-075` | OBS | A peer who leaves the board MUST stop being painted. | this spec section 4 | `ambre_toile::ui::presence::tests::a_peer_absent_from_the_set_stops_being_painted`, which adopts a two-peer set, then a one-peer set, then an empty one, and counts what each paints | verified |
| `TOILE-UI-076` | OBS | A peer's label MUST be the display name their session carries, and MUST fall back to a prefix of their identifier only when they send none. | this spec section 4 | `ambre_toile::ui::presence::tests::the_label_is_the_name_the_sample_carried` on the reader, and `ambre_toile::logic::convergence_tests::the_outbound_sample_carries_the_session_display_name` on the writer. That a label decides neither identity nor colour is `PRESENCE-004` | verified |

### 1.2 A peer's sample arrives

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|---|
| `TOILE-179` | POST | An arriving sample MUST become that peer's latest, and MUST be announced once to the surface that paints it. | this spec section 3 | `ambre_toile::logic::convergence_tests::apply_ephemeral_keys_presence_by_user_id_without_touching_history` asserts the one event carries the sender and the sample, and that reading the peer back returns it | verified |
| `TOILE-UI-222` | POST | An arriving laser point MUST extend that peer's trail, and a sample without one MUST clear it: a lifted highlighter leaves nothing behind. | this spec section 3 | `ambre_toile::ui::presence::tests::laser_accumulates_a_fading_trail_and_clears_on_lift` asserts the segment count after three points and after the lift | verified |
| `TOILE-180` | FRAME | An arriving sample MUST NOT move this viewer's camera, selection or slide -- unless the viewer follows the sender, which is the one exception (`features/ambre-spec-toile-presentation.md`). | this spec section 3 | none. The handler writes one map entry and reads the follow target, and no test sends a sample to a viewer who has a selection or a camera to lose | unverified |
| `TOILE-181` | FAIL | Given a sample it cannot read, the client MUST drop it and MUST NOT announce anything: presence is best effort, and the next sample corrects it. | this spec section 3 | `ambre_toile::logic::convergence_tests::apply_ephemeral_drops_a_malformed_frame` asserts no event comes out of a malformed frame | verified |

### 1.3 Broadcast this peer's gesture

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|---|
| `TOILE-182` | POST | Broadcasting a gesture MUST produce one ephemeral frame, and MUST produce it whatever the frame's fate: presence is not acknowledged. | this spec section 3 | `ambre_toile::logic::convergence_tests::presence_command_broadcasts_ephemeral_only` asserts the frame is there | verified |
| `TOILE-183` | FRAME | Broadcasting MUST NOT write a delta, MUST NOT push an undo item, and MUST NOT ask for a repaint: the local pointer is the browser's, not the board's. | this spec section 3 | `ambre_toile::logic::convergence_tests::presence_command_broadcasts_ephemeral_only` asserts both the absent delta and the empty display list. Nothing reads the undo stack afterwards | verified |
| `TOILE-184` | FAIL | Given a sample it cannot encode, the client MUST drop it silently and MUST NOT surface an error: a lost sample is corrected by the next one. | this spec section 3 | none. The encode failure is logged and yields an empty outcome, and no test can reach it: every field of the sample is a plain postcard type | unverified |
| `TOILE-UI-073` | POST | A gesture on the board MUST broadcast the local peer's cursor, selection and in-flight stroke. | this spec section 3 | `ambre_toile::ui::presence::tests::a_sample_is_sent_on_a_change_or_on_the_heartbeat`, which holds the schedule: a changed sample goes at once, an unchanged one waits for the heartbeat. What the sample is built from is `ToileView::presence_sample`, and no test reads it | unverified |
| `TOILE-UI-074` | POST | The highlighter MUST be a tool the user can pick, and its trail MUST NOT become an element. | this spec section 4 | `ambre_toile::ui::tool::tests::the_highlighter_streams_a_point_and_writes_nothing`, which drives a press, two moves and a release and reads a live point on each and no command at all | verified |

* **Budget.** the presence overlay under 1 ms per frame, a row of
  `ambre-spec-budgets.md section 2`. The overlay is skipped whole when no peer is known
  (`TOILE-UI-218`), so an alone board pays nothing. A board with peers pays a scan of the visible
  set per ghost -- section 6 -- and no bench holds the row.
* **Boundary.** every presence frame leaves the client, and `ZK-005` seals it with the `DocumentKey`:
  the relay reads `document_id`, `pub_key` and `counter` and nothing else. The author is resolved to a
  `UserId` in `ambre-sync` before the frame reaches this application, so no peer can paint a cursor
  under another user's name (`SYNC-014`, `SYNC-015`).
* **Verification.** 19 `verified`, 5 `unverified`, 0 `unimplemented`. The four unbuilt rows were one
  hole with four edges -- this board received presence and sent none -- and the hole is closed: a
  sample leaves on every gesture and on a heartbeat, the highlighter is a tool, a peer who stops
  sending leaves the live set, and a label is the name the session carries. Correcting the event
  shape is what closed two of them at once: a departure is the absence of an entry
  (`PRESENCE-009`), so no departure signal was needed, and `TOILE-185` and `TOILE-UI-075` fell to
  the same change. `TOILE-178` and `TOILE-UI-077` were `unverified` and failing, and both are now
  driven. What stays loose is one shape: a rule whose failure only shows on a screen or on a wire
  no test drives -- the three pass orders (`TOILE-UI-219`), the second board (`TOILE-UI-220`), the
  frame condition of an inbound sample (`TOILE-180`), an encode failure nothing can produce
  (`TOILE-184`), and what the sample is built from (`TOILE-UI-073`), whose schedule is held and
  whose construction is view code no host test reaches

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

**The sample** (`ambre-bus`). One sample is a peer's whole presence, not a patch on the last one:
the receiver replaces what it held, so a field a sender leaves out is a field it clears
(`TOILE-178`). The optional fields are how a peer says *not any more* -- pointer off the board,
highlighter lifted -- and not how it omits what has not changed:

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

**Command** (`ambre-bus`): `Presence { payload, now_ms }`, non-mutating, answered by an ephemeral
frame: the local pointer is the browser's, not the board's. `now_ms` is the UI's wall clock, which
the Logic thread does not hold; it never travels, and it is what ages the peers this client knows
about. A sender's own clock decides nothing, so a wrong or hostile one ages nobody.

**Event** (`ambre-bus`): `PeerPresence(Vec<PresenceUpdate>)`, the whole live peer set, which the
view adopts as it stands (`PRESENCE-009`). A departure is an absent entry, so no departure signal
exists anywhere in the workspace.

**Steps, when the local peer gestures:**

1. the view builds the sample from live view state each frame: the pointer in world space, the
   selection, the tail of an in-flight stroke and the highlighter point;
2. it sends it when it differs from the last one sent, or when the heartbeat is due -- which is what
   keeps this client in the others' live set and what advances the Logic thread's only clock
   (`TOILE-UI-073`);
3. the Logic thread stamps the session's display name on it (`TOILE-UI-076`), ages its own peers
   against the clock, and hands the frame to Sync.

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

The render thread keeps the trail the payload does not: the sample carries one highlighter point,
and the overlay accumulates the last 24 into a ring buffer.

**Code:**

* `crates/ambre-toile/src/logic/state/presence.rs` -- the broadcast and the inbound apply;
* `ui/presence.rs` -- the per-peer mirror and the whole overlay pass;
* `crates/ambre-logic/src/data.rs` -- the routing of an inbound ephemeral frame to this application.

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

* **A departure is a timeout, not a signal.** The relay says nothing when a peer closes a board, so
  a peer leaves the live set when their samples stop -- ten seconds of silence, against a two-second
  heartbeat. It is what makes `TOILE-UI-075` hold with nothing added to the transport, and it is
  also its own limit: a peer who closed the board is painted for a few seconds more, and a peer
  behind a stalled network blinks out and back.
* **A pointer never leaves the board.** The host raises no pointer-leave event, so `cursor: None` --
  the way the payload says *the pointer is off the board* -- is produced by nothing: a peer's arrow
  sits where it last was until they leave the set. The field is the sender's to fill and no gesture
  fills it.
* **An in-flight stroke travels as its tail.** A sample rides every gesture frame, and a long stroke
  would resend its whole history each time, so only the last samples travel. A peer joining
  mid-stroke sees the end of it, and the whole stroke when it commits.
* **What the sample is built from is unread** (`TOILE-UI-073`). The schedule is host-tested; the
  construction is `ToileView::presence_sample`, view code no host test reaches, so a field wired to
  the wrong source would break nothing red.
* **A ghost costs a scan of the visible set.** The overlay resolves each selected element by walking
  the culled set and comparing string ids, so one peer selecting a hundred shapes on a board of two
  thousand visible ones costs two hundred thousand string comparisons per frame, against a 1 ms
  budget nothing measures. The board's own passes index what they draw; this one does not.
* **Nothing bounds how often a sample is sent**, and **a viewer's presence is not distinguished
  from an editor's**. Both are the suite's, not this board's, and both are
  `1-Foundations/ambre-spec-presence.md section 6`.
