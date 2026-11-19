# AMBRE -- FEATURE: LIVE PRESENCE

**Type:** Horizontal (framework)

> Where the other collaborators are, right now: a position in each peer's colour, their name beside
> it, and what they hold selected. A presence sample is relayed and forgotten -- nothing of it
> reaches the document, the history or a file.

Three applications show the same behaviour over three anchors. They are a cell in Trame, a point on
the board in Toile, and a position in the text in Prose. This document owns the behaviour, and each
application owns its anchor and what it paints. The transport underneath is
`ambre-spec-sealed-envelope.md` and is not restated here. The colour is, because the rule about it
is `PRESENCE-006`.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PRESENCE-002` | `INV` | A presence payload MUST NOT carry a user identifier. | this spec section 3; `SYNC-014` | type-level -- no payload in the suite declares the field, so an impersonated sample has no shape to travel in | none | verified |
| `PRESENCE-003` | `INV` | The author of a presence sample MUST be the `Id<User>` the Sync thread resolves from the signing key. | this spec section 3; `ambre-spec-membership.md`, `SYNC-014` | `ambre_sync::pipeline::tests::roundtrip_ephemeral`; `ambre_server::ws::handler::message::tests::forged_author_key_writes_no_presence` | none | verified |
| `PRESENCE-005` | `INV` | The peer set MUST be keyed by `Id<User>`: one collaborator on two devices MUST NOT render as two. | this spec section 3 | `ambre_toile::logic::convergence_tests::apply_ephemeral_keys_presence_by_user_id_without_touching_history` -- on the assertions that a second peer is tracked under its own identity and neither displaces the other | none | verified |
| `PRESENCE-006` | `INV` | A peer's colour MUST be derived from that `Id<User>` by the shared cross-app helper. | this spec section 3; this spec section 5 | `ambre_draw::peer::tests::stable_and_pure`; `::distinct_seeds_usually_differ`; `::opaque` | none | verified |
| `PRESENCE-004` | `INV` | A peer's display name MUST be cosmetic: it MUST NOT decide identity, colour or permission. | this spec section 3 | `ambre_toile::ui::presence::tests::a_peer_name_decides_neither_identity_nor_colour` -- two peers under one name paint two cursors in their own colours, and a renamed peer keeps its colour | none | verified |
| `PRESENCE-017` | `INV` | A peer's label MUST end on the handle derived from the `Id<User>` the Sync thread resolved, so two members never read one label, whatever names their samples carry. | this spec section 3; `PRESENCE-003` | `ambre_core::name::tests::two_members_under_one_name_read_two_labels`; `ambre_toile::ui::presence::tests::the_label_is_the_name_the_sample_carried` -- the board; `ambre_prose::logic::presence::tests::a_peer_named_like_another_reads_its_own_handle` -- the text | none | verified |
| `PRESENCE-007` | `INV` | A presence anchor MUST name identities that survive a structural edit, never positions. | this spec section 3 | `ambre_toile::ui::presence::tests::a_selection_anchor_survives_a_structural_edit` -- a peer's selection ghost follows its element through a deletion, an insertion before it and a move | none | verified |

### 1.2 Send a presence sample

The local peer moves. What leaves is one ephemeral and nothing else.

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PRESENCE-001` | `POST` | A presence sample MUST travel as an ephemeral. | this spec section 3; `ZK-005` | `ambre_toile::logic::convergence_tests::presence_command_broadcasts_ephemeral_only` -- on the assertion that the command produced an ephemeral | none | verified |
| `PRESENCE-011` | `FRAME` | Sending a presence sample MUST NOT write to the document or its history. | this spec section 3; `ZK-005` | `ambre_toile::logic::convergence_tests::presence_command_broadcasts_ephemeral_only` -- on its two other assertions, that the command produced no CRDT delta and no display event, so a sample that reached the document fails | none | verified |

### 1.3 Receive a presence sample

An inbound sample is opened, its author resolved, and the peer set updated for the view.

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PRESENCE-009` | `POST` | The event delivered to the view MUST carry the whole live peer set, so a departed peer is an absent entry. | this spec section 3 | `ambre_toile::logic::convergence_tests::a_peer_who_stops_sending_leaves_the_live_set` -- it reads the set an event carries after a peer ages out; `ambre_toile::ui::presence::tests::a_peer_absent_from_the_set_stops_being_painted` -- the view that adopts it | none | verified |
| `PRESENCE-010` | `POST` | A remote position MUST snap when its new sample is not adjacent to the previous one. | this spec section 4 | `ambre_toile::ui::presence::tests::a_remote_pointer_that_jumps_snaps_to_its_new_sample` -- a pointer that jumps is painted exactly as one first seen at its new sample | none | verified |
| `PRESENCE-012` | `FRAME` | Applying a presence sample MUST NOT change the document or its history. | this spec section 3; `ZK-005` | `ambre_toile::logic::convergence_tests::apply_ephemeral_keys_presence_by_user_id_without_touching_history` -- on the assertions that the snapshot and the element count are identical either side of the apply | none | verified |
| `PRESENCE-013` | `FAIL` | A presence sample that does not decode MUST be dropped without an error. | this spec section 3; derived | `ambre_toile::logic::convergence_tests::apply_ephemeral_drops_a_malformed_sample` -- it feeds four arbitrary bytes and asserts the apply answers with nothing rather than raising | none | verified |
| `PRESENCE-014` | `POST` | A peer who stops watching a document MUST leave the live set on the relay's word, not on a timeout, on both the close path and the dropped-connection path. | this spec section 3; `ambre-spec-broadcast.md`, `SRV-097` | `ambre_sync::updates::tests::peer_left_resolves_the_key_the_relay_named` -- the attribution; `ambre_toile::logic::convergence_tests::a_departure_message_drops_the_peer_at_once` -- the set the view is handed; `ambre_server::redis_ops::redis_integration_tests::presence_field_is_dropped_on_departure` -- the relay's store drops the sample at once; `ambre_server::routes::tests::handler_tests::the_live_set_crosses_instances_and_a_close_leaves_nothing` -- the relay announces a clean close and a dropped connection, database tier | none | verified |
| `PRESENCE-015` | `POST` | A peer MUST leave the live set only when the last of its sessions on the document stops watching it. | this spec section 3; `PRESENCE-005`, `PRESENCE-014` | `ambre_server::ws::handler::message::ephemeral_binding_tests::a_departure_waits_for_the_last_session_of_its_key` -- two sessions of one key, the first close keeps the entry and says nothing, the last drops it and says it once | none | verified |
| `PRESENCE-008` | `FAIL` | An anchor that resolves to no live target MUST be dropped without an error. | this spec section 3 | `ambre_toile::ui::presence::tests::selection_ghosts_outline_visible_elements_only` -- its peer selects an element the frame does not carry, so one ghost is painted, the other is dropped, and nothing is raised | none | verified |
| `PRESENCE-016` | `POST` | Trame MUST paint each collaborator's cell under the collaborator's colour and name. | this spec section 4; `2-Trame/features/ambre-spec-trame-selection.md section 4` | not built | unbuilt -- lands with the `Grid surface` Story (`ambre-roadmap.md` E6) | unimplemented |

* **Budget.** the presence overlay under 1 ms per frame, a row of
  `ambre-spec-budgets.md section 2`. It is written against Toile's twenty cursors and their trails,
  the heaviest of the three overlays. A Trame border or a Prose caret is one quad and a label, drawn
  in a frame pass already charged.
* **Boundary.** every presence sample leaves the client, and `ZK-005` seals it with the
  `DocumentKey`. The relay reads `document_id`, `pub_key`, `key_id`, `counter`, and the log head
  and entry its sender read (`SYNC-262`). The author is resolved to a `Id<User>` in `ambre-sync`
  first, so no peer paints under another name.
* **Verification.** 16 `verified`, 1 `unimplemented`. A departure travels twice: the relay says it,
  and the client ages a peer out if that word is lost.

---

## 2. WHAT EACH USER SEES

Two peers over one document. The third column never changes, because the absence is the rule.

| Alice does | Bob does | The document holds | Each user sees | Test |
|---|---|---|---|---|
| moves to another cell, point or word | watches | nothing | Alice's position on Bob's screen, in her colour, with her name | `ambre_toile::ui::presence::tests::cursor_draws_arrow_and_label_chip` |
| selects a range | watches | nothing | the range tinted in Alice's colour | `ambre_toile::ui::presence::tests::selection_ghosts_outline_visible_elements_only` |
| opens the same document on a second device | watches | nothing | one collaborator in one colour, not two | `ambre_draw::peer::tests::stable_and_pure` |
| moves onto content Bob has not received yet | inserts that content | Bob's insert | nothing for Alice's position, then her position once the delta lands | none -- `PRESENCE-008` |
| sends a sample naming herself "Bob" | edits | Bob's edit | the position attributed to Alice, the sender Sync resolved, under "Bob" and the handle of Alice | `ambre_core::name::tests::two_members_under_one_name_read_two_labels` -- `PRESENCE-004`, `PRESENCE-017` |
| jumps to the far end of the document | watches | nothing | her position disappear and reappear there, with no slide between | none -- `PRESENCE-010` |
| inserts a row, a shape or a paragraph above Bob's position | watches | Alice's insert | Bob's position still on the same cell, shape or word | none -- `PRESENCE-007` |
| closes the document | watches | nothing | Alice's position off Bob's screen | `ambre_toile::logic::convergence_tests::a_departure_message_drops_the_peer_at_once` |

---

## 3. MODEL

**Containers.** none, in any application, and that is `PRESENCE-001`. A presence sample is the one
thing a collaborator produces that no schema document declares.

**The two shapes, and why there are two.** A peer authors one and reads another. The split is the
identity rule: what a peer may assert about itself, and what the receiver has established.

| Shape | Direction | Carries |
|---|---|---|
| the payload | this peer to the relay, and back from every other | an anchor, and a self-asserted display name |
| the resolved sample | the Logic thread to the view | the resolved `Id<User>`, the name, the colour, the anchor resolved to a position |

The payload is postcard bytes inside an `Ephemeral` wire message, so it is one of the few types in the
suite that keeps `serde`. The resolved sample never leaves the process: it moves by value on
an `ambre_bus::conduit` queue, and carries none.

**The identity rule, stated once.** The payload carries no `user_id` (`PRESENCE-002`). The canonical
one arrives out of band, on `LogicEvent::RemoteEphemeral { from_user_id }`, resolved by `ambre-sync`
from the key that signed the envelope (`PRESENCE-003`). The name inside the payload labels a position
and decides nothing (`PRESENCE-004`). So a peer who writes another member's name into the payload
relabels their own cursor, and nothing else.

The label still ends on a short handle of the resolved
`Id<User>`, which no sender writes. So that cursor never reads as the cursor of the other member
(`PRESENCE-017`).

**The four steps of an inbound sample:**

1. `ambre-sync` decrypts the sample and resolves the author to an `Id<User>`, dropping a sample whose
   key matches no member (`SYNC-014`, `SYNC-015`);
2. the Logic thread records the sample under that `Id<User>`, replacing that peer's previous one, so
   one collaborator on two devices is one entry (`PRESENCE-005`);
3. the anchor is resolved against the current document. An anchor naming no live target is dropped,
   with no error and no record (`PRESENCE-008`);
4. the whole live set travels to the view, which replaces its own (`PRESENCE-009`).

**Why the set travels whole.** Presence is a stream of samples, not a stream of changes. A lost
sample must cost one late position, never a position that stays on the screen for good. A full set
makes the departure of a peer the absence of an entry, which no dropped message can corrupt.

**How a departure travels** (`PRESENCE-014`). The relay is the one party that knows a peer stopped
watching: it sees the unsubscribe, and it sees the socket close. One key signs all of a user's
sessions, so it counts them per document. The last to go drops the entry and says so,
naming the document and the author key and nothing else (`PRESENCE-015`, `SRV-098`).
`ambre-sync` resolves that key as it resolves a sample's author, and drops a key it cannot resolve.
The application then removes the peer and hands the view the set that remains.

**And why the client still ages a peer out.** The message is best effort: a relay restart or a
lost publish loses it. Each application drops a peer silent for a while, the floor under
`PRESENCE-014`.

**The anchor is the variation point.** Each application names the identities its own document mints,
because a position is what a concurrent structural edit invalidates (`PRESENCE-007`).

| Application | Anchor | Resolved against |
|---|---|---|
| Trame | `CellAnchor { sheet_id, row_uid, col_uid }` -- the uids of `ambre-spec-trame-schema.md section 2` | the sheet index, to a row and a column |
| Toile | a world point, and the `Id<Element>` set of the peer's selection | the board, to screen space |
| Prose | `CursorPosition { block_id, offset }` -- a `TreeID` and an offset in that block | the block tree, to a caret rectangle |

Trame's declaration is `2-Trame/features/ambre-spec-trame-selection.md section 3`, Toile's is
`3-Toile/features/ambre-spec-toile-presence.md section 3`, and Prose's is
`4-Prose/features/ambre-spec-prose-engine.md section 3`. None of the three is restated here.

**What each application paints** is its own, and each of the three has a row for it. All three
paint the position in the peer's colour, with their name beside it.

| Application | Painted as | Row |
|---|---|---|
| Trame | the peer's cell border, and a name badge | `2-Trame/features/ambre-spec-trame-selection.md section 4` |
| Toile | a filled arrow, a label chip, ghost outlines, a stroke and a fading trail | `TOILE-UI-014` |
| Prose | a caret bar as tall as the line, a name chip above it, and a tint over the selected range | `features/ambre-spec-prose-page-rendering.md section 4` |

**The colour is one function, and every application calls it** (`PRESENCE-006`). It lives beside
the drawing trait rather than in an application. The same collaborator is therefore the same colour
in a spreadsheet, a board and a text document.

```rust
// ambre-draw/src/peer.rs
use ambre_core::id::{Id, User};
use peniko::Color;

/// Deterministic per-user color, identical across all apps in the suite.
/// Seeded on the **canonical `Id<User>`** (spoof-proof, resolved by `ambre-sync`)
/// -- never on the per-device `pub_key`, which changes on key rotation. The id
/// bytes are folded into a hue with FNV-1a and mapped through HSL at a fixed
/// saturation/lightness, so every peer color is vivid, legible on light and
/// dark backgrounds, and distinct from its neighbours; the result is always
/// fully opaque (callers apply per-element alpha themselves).
pub fn peer_color(user_id: &Id<User>) -> Color;
```

Seeding on the `Id<User>` type rather than on a raw `Uuid` keeps the type end to end through every
presence path. That is why `ambre-draw` takes a types-only dependency on `ambre-core`. It is the
one exception to its dependency rule (`CANVAS-025`), and its manifest records it.

**Code:** `crates/ambre-draw/src/peer.rs` holds the colour.
`crates/ambre-toile/src/logic/state/presence.rs` and `ui/presence.rs` are the only implementation of
the rest. `crates/ambre-logic/src/data.rs` routes an inbound ephemeral to its application, for
all three.

---

## 4. SURFACE

| Trigger | Behaviour |
|---|---|
| a local move, of any origin | one outbound sample carrying the new anchor |
| a sample from a known peer | that peer's position moves |
| a sample from an unknown peer | a position appears, in the colour that peer's identity gives |
| a set the peer no longer appears in | that peer's position leaves the screen |
| a hover over a remote position | the peer's name |

**How a position moves between two samples.** It interpolates over 100 ms, which hides the network
latency and reads as a cursor moving rather than teleporting. A sample that is not adjacent to the
previous one snaps instead (`PRESENCE-010`). A peer jumping from the first page to the fortieth
must not draw a cursor sliding through thirty-nine pages they never visited.

---

## 5. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| One document for three applications | a presence section per application | it is what produced X59, X60, X30 and X15. The transport, the identity, the colour, the keying and the set semantics are one behaviour, and the review that read Prose against Trame found four defects with one root over two documents. Three copies drift in three directions, and the drift is invisible because no reader holds two of them at once. |
| The anchor stays with the application | a shared anchor type parameterized over three models | it buys one type and forces every application through an indirection to reach its own ids. An anchor names the identities of one document model, and a spreadsheet has no text offset. |
| Presence is ephemeral, never a document write | a presence container pruned on a timer | it grows the file and makes undo step over other people's gestures. A position changes on every gesture and matters for as long as it is on the screen; writing it would put a per-gesture operation into the history every peer keeps for good, and make a cursor a thing that merges (`PRESENCE-001`). |
| The payload carries no user identifier | an author field in the sample | it lets any member paint a cursor under any other member's name. Anything a peer writes about itself is something it can write about anyone else; leaving the field out means there is nothing to forge, and the only identity is the one resolved from the key that signed the frame (`PRESENCE-002`). |
| Keyed by `Id<User>`, never by peer or device | keying by the sending peer | it doubles a collaborator every time they open a second tab. A peer identifier is per session and per device, so one collaborator on a laptop and a phone appears as two people in two colours; the `Id<User>` is the person (`PRESENCE-005`). |
| The colour is hashed from the `Id<User>` by the shared helper | a per-application palette assigned in join order | it gives one user a different colour per document and swaps two users' colours on a reconnect. One person with three colours is three people to the reader; the helper is pure, so the colour is the same on every device and in every application (`PRESENCE-006`). |
| The anchor is move-invariant | raw coordinates | it is what Trame carried until the review named it X30. A remote position must follow its target through the inserts, deletes and sorts a collaborator makes while the sample is in flight, and a row number, a screen point or a text offset is exactly what those edits invalidate, so a positional anchor makes remote cursors jump on every structural change (`PRESENCE-007`). |
| An unresolvable anchor is dropped, silently | queueing the sample until the target resolves | it holds a cursor at a position the peer has already left. The sample is one of a stream whose next one is on its way, and the target is missing because a delta is in flight: there is nothing for the user to do and nothing for the log to say (`PRESENCE-008`). |
| The event carries the whole live set | one event per peer with a receiver-side accumulation | it needs a removal path, a timeout or a leave message, and leaves a ghost until whichever one is written. It is what Toile built, and `TOILE-UI-075` is the ghost cursor it produces. The whole live set makes a departure the absence of an entry, so no application needs a departure signal, and none of the three has one (`PRESENCE-009`). |
| A non-adjacent move snaps | interpolating every move | it is one rule fewer and shows a false path. The interpolation exists to hide 100 ms of latency on a cursor that moved one step; applied to a jump, it draws a path through content the peer never visited, which is a lie the reader cannot detect (`PRESENCE-010`). |

---

## 6. NOT BUILT, AND NOT YET RULED

* **Trame broadcasts nothing.** Toile and Prose send, receive, key, route and paint. Trame
  specifies a payload and emits nothing (`PRESENCE-016`), so Toile's tests hold every rule here. Two
  of them, the anchor shape and the absent smoothing, are held by Toile's shape rather than by
  anything that goes red. A third sender inherits neither.
* **A stalled network is not a departure.** The relay names a peer who unsubscribed and one whose
  socket closed. A peer whose network stalls without dropping the socket is named by nobody. They
  leave on the client's own timeout, ten seconds after their last sample, and blink back on the
  next one.
* **Nothing bounds the sample rate.** No document says how often a move broadcasts, and the
  per-frame budget above is the paint cost alone. Toile sends at most one sample per rendered frame, which is
  a coalescing rule and not a rate.
* **Nothing rules on a viewer against an editor.** Every peer paints the same overlay whatever their
  role (`ambre-spec-membership.md`).
* **The label's source is a session, not a directory.** Toile stamps the name its own `SessionReady`
  carried. A peer is named by what they told the relay about themselves, not by what the member
  list holds (`TOILE-UI-076`). `PRESENCE-004` is what makes that safe, because the name decides
  nothing. Prose has never said where the name in its payload comes from.
* **Nothing rules on presence against optimistic local text.** Prose paints text that is not yet in
  the document. A remote anchor inside the same block therefore resolves against a document the
  screen does not show (`features/ambre-spec-prose-text-editing.md`).

---

## 7. WHAT THIS DOCUMENT DOES NOT OWN

| Question | Owner |
|---|---|
| How an ephemeral is sealed and replay-guarded | `ambre-spec-sealed-envelope.md`; `ZK-005`, `SYNC-008` |
| Which member an ephemeral is delivered under | `ambre-spec-membership.md`; `SYNC-014`, `SYNC-015` |
| What the drawing calls that paint an overlay are, and which crate declares them | `ambre-spec-drawing-surface.md` |
| What each application's anchor names, and how it resolves | the three documents named in section 3 |
| What the overlay paints, and in what order | the three rows named in section 3 |
| Where an asynchronous remark is anchored, and what happens when its target dies | the comments feature of each application: the anchor is that feature, and the three anchors share no rule |
