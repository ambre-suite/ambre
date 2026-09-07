# AMBRE -- FEATURE: LIVE PRESENCE

**Type:** Vertical (feature)
**Implementation:** partial -- the overlay paints every peer; nothing broadcasts this peer, and
nobody ever leaves.

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
| `TOILE-178` | INV | A sample MUST replace the peer's presence whole: a field a sender leaves out is a field the receiver clears, not one it keeps. | this spec section 3 | none. The one test that sends a second sample to the same peer sends it for the laser, whose clearing is its own branch; nothing sends a pointer move after a selection to see the ghosts go | unverified |
| `TOILE-185` | INV | The client MUST NOT keep a departed peer's presence: the sample, and the in-flight stroke it carries, MUST be dropped. | this spec section 3 | none, and **the rule fails today**: no departure signal exists, the Logic thread's map is only ever inserted into, and the overlay's own removal path has no caller -- section 6 | unverified |
| `TOILE-UI-014` | OBS | Every peer MUST be painted in their own colour, the same colour that peer carries in the other applications. | this spec section 4 | `ambre_toile::ui::presence::tests::two_peers_get_distinct_colours` asserts two peers fill in two colours; the colour is `ambre_draw::peer_color` over the resolved id, held across applications by `PRESENCE-006` | verified |
| `TOILE-UI-216` | OBS | A peer's pointer MUST be painted as an arrow with a label chip beside it. | this spec section 4 | `ambre_toile::ui::presence::tests::cursor_draws_arrow_and_label_chip` asserts the filled arrow, the chip and the one glyph run | verified |
| `TOILE-UI-078` | OBS | A selection ghost MUST outline only the elements the board paints this frame. | this spec section 4 | `ambre_toile::ui::presence::tests::selection_ghosts_outline_visible_elements_only` asserts one outline for a selection of two, one of which is culled out | verified |
| `TOILE-UI-217` | OBS | A peer's stroke MUST be painted while it is still in flight, before it commits. | this spec section 4 | `ambre_toile::ui::presence::tests::live_stroke_draws_a_polyline` asserts the one polyline. Nothing asserts it disappears when the stroke commits, because no tool streams one (`TOILE-UI-073`) | verified |
| `TOILE-UI-077` | OBS | A laser trail MUST fade to nothing once the peer stops moving the highlighter. | this spec section 4 | none, and **the rule fails today**: the trail fades by sample rank, not by age, so a highlighter held still keeps its trail until the peer lifts it -- section 6 | unverified |
| `TOILE-UI-218` | OBS | A board with no peer on it MUST paint no overlay at all. | this spec section 4 | `ambre_toile::ui::presence::tests::empty_paints_nothing_and_is_empty` asserts the empty draw log and that the pass leaves no transform behind | verified |
| `TOILE-UI-221` | OBS | Two peers MUST be painted in a stable order, so an overlap does not flicker between frames. | this spec section 4 | structural, and the construction is the proof: the pass sorts the peers by their resolved id before painting, so the order is a function of the set and not of a hash iteration | verified |
| `TOILE-UI-219` | OBS | Presence MUST be painted above every element and below the reaction badges, so a chip is never hidden by a ghost. | this spec section 4; `features/ambre-spec-toile-reactions.md` | none. The overlay pass sits between the two in the frame, and no test reads the order of the three passes | unverified |
| `TOILE-UI-220` | INV | The overlay MUST hold no peer from a board that is no longer open. | this spec section 4 | none. The document swap clears the mirror, and no test opens a second board with a peer painted on the first | unverified |
| `TOILE-UI-075` | OBS | A peer who leaves the board MUST stop being painted. | this spec section 4 | not built -- lands with the "Live presence on the board" Story (`ambre-roadmap.md` E4) | unimplemented |
| `TOILE-UI-076` | OBS | A peer's label MUST be the display name the member list carries, never a prefix of their identifier. | this spec section 4 | not built -- lands with the "Live presence on the board" Story (`ambre-roadmap.md` E4). That a label decides neither identity nor colour is `PRESENCE-004` | unimplemented |

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
| `TOILE-UI-073` | POST | A gesture on the board MUST broadcast the local peer's cursor, selection and in-flight stroke. | this spec section 3 | not built -- lands with the "Live presence on the board" Story (`ambre-roadmap.md` E4) | unimplemented |
| `TOILE-UI-074` | POST | The highlighter MUST be a tool the user can pick, and its trail MUST NOT become an element. | this spec section 4 | not built -- lands with the "Live presence on the board" Story (`ambre-roadmap.md` E4) | unimplemented |

* **Budget.** the presence overlay under 1 ms per frame, a row of
  `ambre-spec-budgets.md section 2`. The overlay is skipped whole when no peer is known
  (`TOILE-UI-218`), so an alone board pays nothing. A board with peers pays a scan of the visible
  set per ghost -- section 6 -- and no bench holds the row.
* **Boundary.** every presence frame leaves the client, and `ZK-005` seals it with the `DocumentKey`:
  the relay reads `document_id`, `pub_key` and `counter` and nothing else. The author is resolved to a
  `UserId` in `ambre-sync` before the frame reaches this application, so no peer can paint a cursor
  under another user's name (`SYNC-014`, `SYNC-015`).
* **Verification.** 13 `verified`, 7 `unverified`, 4 `unimplemented`. The rows went 8 to 24 and the
  distinct mechanisms 8 to 9: the split manufactured no coverage, it stopped hiding what the eight
  tests already asserted. Two rows carried the render half between them and named five tests; the
  five hold five different rules, one per thing the overlay draws, and only one of them was about
  colour. The ninth name is a test of the empty board that nothing pointed at. What the split found
  loose is the receiver's own rule -- a sample replaces the peer's presence whole -- which the model
  and the decisions both described as a per-field merge, and which nothing tests. The four unbuilt
  rows are one hole with four edges: this board receives presence and sends none, so the overlay has
  never had anything to paint outside its own tests. `TOILE-UI-072` is **retired, not moved** (the
  register is `contracts.py`): it restated a rule three applications share, and its only mechanism
  was the shared helper's own tests, which now hold `PRESENCE-006`.

---

## 2. WHAT EACH USER SEES

Presence never reaches the document, so the third column is the same in every row. It is kept because
the absence is the rule.

| Alice does | Bob does | The document holds | Each user sees | Test |
|---|---|---|---|---|
| moves her pointer | watches | nothing | Alice's arrow and handle on Bob's board, in her colour (`TOILE-UI-014`) | `ui::presence::tests::cursor_draws_arrow_and_label_chip` |
| selects two shapes, then moves her pointer | watches | nothing | the two ghosts, then the ghosts gone: a sample replaces the last one whole (`TOILE-178`) | none |
| selects two shapes | watches | nothing | the two shapes outlined in Alice's colour on Bob's board | `ui::presence::tests::selection_ghosts_outline_visible_elements_only` |
| draws a stroke, holding | watches | nothing until the release | the line growing on Bob's board, then one stroke element on both | `ui::presence::tests::live_stroke_draws_a_polyline` |
| sweeps the highlighter | watches | nothing | a trail behind Alice's pointer, fading as she moves (`TOILE-UI-077`) | `ui::presence::tests::laser_accumulates_a_fading_trail_and_clears_on_lift` |
| opens the board from a second device | watches | nothing | one collaborator in one colour, not two (`PRESENCE-005`, `PRESENCE-006`) | `ambre_draw::peer::tests::stable_and_pure` |
| closes the board | watches | nothing | Alice's cursor still on Bob's board (`TOILE-UI-075`) | none -- unbuilt, section 6 |

---

## 3. MODEL

**Containers.** none (`PRESENCE-001`, `ambre-spec-toile-schema.md section 8`).

**The sample** (`ambre-bus`). One sample is a peer's whole presence, not a patch on the last one:
the receiver replaces what it held, so a field a sender leaves out is a field it clears
(`TOILE-178`). The optional fields are how a peer says *not any more* -- pointer off the board,
highlighter lifted -- and not how it omits what has not changed:

```rust
pub struct PresencePayload {
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

**Command** (`ambre-bus`): `Presence(PresencePayload)`, non-mutating, answered by an ephemeral
frame and no display event: the local pointer is the browser's, not the board's.

**Event** (`ambre-bus`): `PeerPresence { user_id, presence }`, one per inbound frame, accumulated
into a per-peer mirror. **This is the shape `PRESENCE-009` refuses**: section 6.

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

* **The board sends no presence at all** (`TOILE-UI-073`). The command exists, the Logic thread
  handles it, the transport ships and the overlay paints -- and no gesture in the application emits
  it, so the overlay has never received a frame outside its tests. The receiving half is wired end to
  end, which is what makes the gap invisible in every test that exists.
* **The highlighter has no tool** (`TOILE-UI-074`). The field, the render and the fading trail are
  built; nothing produces a point. The same holds for the live stroke: the tool commits a stroke and
  streams no sample.
* **A peer who leaves stays on the board** (`TOILE-UI-075`, `TOILE-185`). No departure signal exists
  anywhere in the workspace. The overlay's removal path has no caller at all; its reset path is
  called, but only when the whole document is swapped (`TOILE-UI-220`), which is not a peer leaving.
  The Logic thread's map is worse: it is only ever inserted into, so a peer who has been gone for an
  hour still costs whatever their last sample carried, in-flight stroke included. The
  cause is the event shape: `PRESENCE-009` makes a departure the absence of an entry in a whole set,
  which an accumulating per-peer mirror cannot express. Correcting the shape removes the need for a
  departure signal rather than supplying one.
* **A trail fades by rank, not by age** (`TOILE-UI-077`). The oldest sample of the ring buffer is the
  faintest, so the trail melts while the peer moves and freezes when they stop.
* **The label is a hex prefix** (`TOILE-UI-076`). The member list resolved by `ambre-sync` carries a
  display name; the overlay never receives it. Trame and Prose carry it in the payload, cosmetically
  (`PRESENCE-004`), and this payload has no such field: the three agree on what a label may decide,
  not on where it comes from.
* **A ghost costs a scan of the visible set.** The overlay resolves each selected element by walking
  the culled set and comparing string ids, so one peer selecting a hundred shapes on a board of two
  thousand visible ones costs two hundred thousand string comparisons per frame, against a 1 ms
  budget nothing measures. The board's own passes index what they draw; this one does not.
* **Nothing bounds how often a sample is sent**, and **a viewer's presence is not distinguished
  from an editor's**. Both are the suite's, not this board's, and both are
  `1-Foundations/ambre-spec-presence.md section 6`.
