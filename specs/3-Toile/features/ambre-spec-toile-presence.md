# AMBRE -- FEATURE: LIVE PRESENCE

**Type:** Vertical (feature)
**Implementation:** partial -- the overlay paints every peer; nothing broadcasts this peer.

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

| ID | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|
| `TOILE-016` | A cell of presence MUST travel as ephemeral only: broadcast, never appended to history, and keyed by the resolved `UserId`. | this spec section 3; `SYNC-014` | `ambre_toile::logic::convergence_tests::presence_command_broadcasts_ephemeral_only`, `::apply_ephemeral_keys_presence_by_user_id_without_touching_history`, `::apply_ephemeral_drops_a_malformed_frame` | verified |
| `TOILE-UI-014` | Remote presence MUST render per peer with distinct colours, and MUST leave no trace in the document. | this spec section 4 | `ambre_toile::ui::presence::tests::two_peers_get_distinct_colours`, `::cursor_draws_arrow_and_label_chip`, `::selection_ghosts_outline_visible_elements_only`, `::live_stroke_draws_a_polyline`, `::laser_accumulates_a_fading_trail_and_clears_on_lift`; the document half is `TOILE-016` | verified |
| `TOILE-UI-078` | A selection ghost MUST outline only the elements the board paints this frame. | this spec section 4 | `ambre_toile::ui::presence::tests::selection_ghosts_outline_visible_elements_only` | verified |
| `TOILE-UI-077` | A laser trail MUST fade to nothing once the peer stops moving the highlighter. | this spec section 4 | none, and **the rule fails today**: the trail fades by sample rank, not by age, so a highlighter held still keeps its trail until the peer lifts it -- section 6 | unverified |
| `TOILE-UI-073` | A gesture on the board MUST broadcast the local peer's cursor, selection and in-flight stroke. | this spec section 3 | not built -- lands with the "Live presence on the board" Story (`ambre-roadmap.md` E4) | unimplemented |
| `TOILE-UI-074` | The highlighter MUST be a tool the user can pick, and its trail MUST NOT become an element. | this spec section 4 | not built -- lands with the "Live presence on the board" Story (`ambre-roadmap.md` E4) | unimplemented |
| `TOILE-UI-075` | A peer who leaves the board MUST stop being painted. | this spec section 4 | not built -- lands with the "Live presence on the board" Story (`ambre-roadmap.md` E4) | unimplemented |
| `TOILE-UI-076` | A peer's label MUST be the display name the member list carries, never a prefix of their identifier. | this spec section 4 | not built -- lands with the "Live presence on the board" Story (`ambre-roadmap.md` E4). That a label decides neither identity nor colour is `PRESENCE-004` | unimplemented |

* **Budget.** the presence overlay under 1 ms per frame, a row of
  `ambre-spec-budgets.md section 2`. The overlay is skipped whole when no peer is known, so an
  alone board pays nothing.
* **Boundary.** every presence frame leaves the client, and `ZK-005` seals it with the `DocumentKey`:
  the relay reads `document_id`, `pub_key` and `counter` and nothing else. The author is resolved to a
  `UserId` in `ambre-sync` before the frame reaches this application, so no peer can paint a cursor
  under another user's name (`SYNC-014`, `SYNC-015`).
* **Verification.** 3 `verified`, 1 `unverified`, 4 `unimplemented`. The render half is held five ways
  and is complete. The four unbuilt rows are one hole with four edges: this board receives presence
  and sends none, so the overlay has never had anything to paint outside its own tests.
  `TOILE-UI-072` is **retired, not moved** (the register is `invariants.py`): it restated a rule
  three applications share, and its only mechanism was the shared helper's own tests, which now
  hold `PRESENCE-006`. `TOILE-016` stays, because its three convergence tests are Toile's and prove
  `PRESENCE-001` and `PRESENCE-005` on the one implementation that exists.

---

## 2. WHAT EACH USER SEES

Presence never reaches the document, so the third column is the same in every row. It is kept because
the absence is the rule.

| Alice does | Bob does | The document holds | Each user sees | Test |
|---|---|---|---|---|
| moves her pointer | watches | nothing | Alice's arrow and handle on Bob's board, in her colour (`TOILE-UI-014`) | `ui::presence::tests::cursor_draws_arrow_and_label_chip` |
| selects two shapes | watches | nothing | the two shapes outlined in Alice's colour on Bob's board | `ui::presence::tests::selection_ghosts_outline_visible_elements_only` |
| draws a stroke, holding | watches | nothing until the release | the line growing on Bob's board, then one stroke element on both | `ui::presence::tests::live_stroke_draws_a_polyline` |
| sweeps the highlighter | watches | nothing | a trail behind Alice's pointer, fading as she moves (`TOILE-UI-077`) | `ui::presence::tests::laser_accumulates_a_fading_trail_and_clears_on_lift` |
| opens the board from a second device | watches | nothing | one collaborator in one colour, not two (`PRESENCE-005`, `PRESENCE-006`) | `ambre_draw::peer::tests::stable_and_pure` |
| closes the board | watches | nothing | Alice's cursor still on Bob's board (`TOILE-UI-075`) | none -- unbuilt, section 6 |

---

## 3. MODEL

**Containers.** none (`PRESENCE-001`, `ambre-spec-toile-schema.md section 8`).

**The sample** (`ambre-bus`). Every field is optional, so a pointer move need not resend the
selection:

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
`UserId` (`TOILE-016`, `PRESENCE-002`, `PRESENCE-003`). The board anchor is what is Toile's own:
a world point for the pointer and the laser, and the peer's `ElementId` set for the ghosts. Both
survive a structural change, which is `PRESENCE-007`.

**Command** (`ambre-bus`): `Presence(PresencePayload)`, non-mutating, answered by an ephemeral
frame and no display event: the local pointer is the browser's, not the board's.

**Event** (`ambre-bus`): `PeerPresence { user_id, presence }`, one per inbound frame, accumulated
into a per-peer mirror. **This is the shape `PRESENCE-009` refuses**: section 6.

**Steps, when a frame arrives:**

1. decode the sample, and drop the frame on a decode failure -- presence is best effort;
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

## 5. DECISIONS

| Decision | Rationale | Rejected alternative |
|---|---|---|
| The anchor is a world point for the pointer and an `ElementId` set for the ghosts | the two halves answer two questions. A pointer is between elements as often as on one, so it has no identity to name and a world point is already invariant under everything but a pan, which is the viewer's own. A ghost outlines elements, which do have identities, and those survive a move, a restyle and a regroup (`PRESENCE-007`) | one anchor for both, which either drops the pointer to the nearest element or makes a ghost a rectangle that stops following its shape |
| Every sample field is optional | a pointer move is the frequent case and carries no selection, no stroke and no laser point. Making each field optional means the common sample is small, and a peer that stops drawing sends `None` rather than a stale trail | one full sample per frame, which resends the selection on every pointer move |
| The trail is accumulated by the receiver | one point per frame is a fixed sample cost whatever the trail length, and a lost frame shortens the trail instead of breaking it | streaming the whole trail in each sample |
| The highlighter never becomes an element | it is a pointing gesture during a discussion. An element would enter the history, the undo stack and every export | committing the trail as a stroke |

---

## 6. NOT BUILT, AND NOT YET RULED

* **The board sends no presence at all** (`TOILE-UI-073`). The command exists, the Logic thread
  handles it, the transport ships and the overlay paints -- and no gesture in the application emits
  it, so the overlay has never received a frame outside its tests. The receiving half is wired end to
  end, which is what makes the gap invisible in every test that exists.
* **The highlighter has no tool** (`TOILE-UI-074`). The field, the render and the fading trail are
  built; nothing produces a point. The same holds for the live stroke: the tool commits a stroke and
  streams no sample.
* **A peer who leaves stays on the board** (`TOILE-UI-075`). The overlay carries a removal path and a
  reset path, nothing calls either, and no departure signal exists anywhere in the workspace. The
  cause is the event shape: `PRESENCE-009` makes a departure the absence of an entry in a whole set,
  which an accumulating per-peer mirror cannot express. Correcting the shape removes the need for a
  departure signal rather than supplying one.
* **A trail fades by rank, not by age** (`TOILE-UI-077`). The oldest sample of the ring buffer is the
  faintest, so the trail melts while the peer moves and freezes when they stop.
* **The label is a hex prefix** (`TOILE-UI-076`). The member list resolved by `ambre-sync` carries a
  display name; the overlay never receives it. Trame and Prose carry it in the payload, cosmetically
  (`PRESENCE-004`), and this payload has no such field: the three agree on what a label may decide,
  not on where it comes from.
* **Nothing bounds how often a sample is sent**, and **a viewer's presence is not distinguished
  from an editor's**. Both are the suite's, not this board's, and both are
  `1-Foundations/ambre-spec-presence.md section 6`.
