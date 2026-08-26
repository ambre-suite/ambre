# AMBRE -- FEATURE: LIVE PRESENCE

**Type:** Horizontal (framework)
**Implementation:** partial -- Toile receives and paints; nothing in the suite broadcasts.

> Where the other collaborators are, right now: a position in each peer's colour, their name beside
> it, and what they hold selected. A presence sample is relayed and forgotten -- nothing of it
> reaches the document, the history or a file.

Three applications show the same behaviour over three anchors: a cell in Trame, a point on the
board in Toile, a position in the text in Prose. This document owns the behaviour; each application
owns its anchor and what it paints. The transport underneath is `ambre-spec-sealed-envelope.md` and
is not restated here; the colour is, because the rule about it is `PRESENCE-006`.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`.

| ID | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|
| `PRESENCE-001` | A presence sample MUST travel as an ephemeral frame only: broadcast, and never written to the document or its history. | this spec section 3; `ZK-005` | `ambre_toile::logic::convergence_tests::presence_command_broadcasts_ephemeral_only`, `::apply_ephemeral_keys_presence_by_user_id_without_touching_history` | verified |
| `PRESENCE-002` | A presence payload MUST NOT carry a user identifier. | this spec section 3; `SYNC-014` | type-level: no payload in the suite declares the field, so an impersonated sample has no shape to travel in | verified |
| `PRESENCE-003` | The author of a presence frame MUST be the `UserId` the Sync thread resolves from the signing key. | this spec section 3; `ambre-spec-membership.md`, `SYNC-014` | `ambre_sync::pipeline::tests::roundtrip_ephemeral`; `ambre_server::ws::handler::message::tests::forged_author_key_writes_no_presence` | verified |
| `PRESENCE-004` | A peer's display name MUST be cosmetic: it MUST NOT decide identity, colour or permission. | this spec section 3 | none. `PRESENCE-002` and `PRESENCE-003` leave the name nothing to decide, and no test drives a sample whose name contradicts its resolved author | unverified |
| `PRESENCE-005` | The peer set MUST be keyed by `UserId`: one collaborator on two devices MUST NOT render as two. | this spec section 3 | `ambre_toile::logic::convergence_tests::apply_ephemeral_keys_presence_by_user_id_without_touching_history` | verified |
| `PRESENCE-006` | A peer's colour MUST be derived from that `UserId` by the shared cross-app helper. | this spec section 3; this spec section 5 | `ambre_draw::peer::tests::stable_and_pure`, `::distinct_seeds_usually_differ`, `::opaque` | verified |
| `PRESENCE-007` | A presence anchor MUST name identities that survive a structural edit, never positions. | this spec section 3 | not built -- lands with the "Live presence on the board" Story (`ambre-roadmap.md` E4). Toile's anchor is an `ElementId` and satisfies it; the two anchors that must be resolved, Trame's and Prose's, have no crate | unimplemented |
| `PRESENCE-008` | An anchor that resolves to no live target MUST be dropped without an error. | this spec section 3 | not built -- lands with the "Live presence on the board" Story (`ambre-roadmap.md` E4) | unimplemented |
| `PRESENCE-009` | The event delivered to the view MUST carry the whole live peer set, so a departed peer is an absent entry. | this spec section 3 | none, and **the rule fails today**: Toile emits one event per inbound frame and the view accumulates, so a peer who leaves is painted for good -- section 6, and `TOILE-UI-075` | unverified |
| `PRESENCE-010` | A remote position MUST snap when its new sample is not adjacent to the previous one. | this spec section 4 | not built -- lands with the "Live presence on the board" Story (`ambre-roadmap.md` E4) | unimplemented |

* **Budget.** the presence overlay under 1 ms per frame, a row of
  `ambre-spec-budgets.md section 2`, written against Toile's twenty cursors and their trails
  because that is the heaviest of the three overlays. A Trame cell border or a Prose caret is one
  quad and a label, drawn in the frame pass each application already charges.
* **Boundary.** every presence frame leaves the client, and `ZK-005` seals it with the
  `DocumentKey`: the relay reads `document_id`, `pub_key` and `counter` and nothing else. The author
  is resolved to a `UserId` in `ambre-sync` before the frame reaches an application, so no peer can
  paint a position under another user's name (`SYNC-014`, `SYNC-015`).
* **Verification.** 5 `verified`, 2 `unverified`, 3 `unimplemented`. The identity half is held end
  to end, by the seal, the author resolution and Toile's two convergence tests. The two gaps are
  where the three applications disagree, and both are named in section 6: nothing sends a sample in
  any application, and Toile's event shape is the one that keeps a departed peer on the screen.

---

## 2. WHAT EACH USER SEES

Two peers over one document. The third column is the same in every row, and it is kept because the
absence is the rule.

| Alice does | Bob does | The document holds | Each user sees | Test |
|---|---|---|---|---|
| moves to another cell, point or word | watches | nothing | Alice's position on Bob's screen, in her colour, with her name | `ambre_toile::ui::presence::tests::cursor_draws_arrow_and_label_chip` |
| selects a range | watches | nothing | the range tinted in Alice's colour | `ambre_toile::ui::presence::tests::selection_ghosts_outline_visible_elements_only` |
| opens the same document on a second device | watches | nothing | one collaborator in one colour, not two | `ambre_draw::peer::tests::stable_and_pure` |
| moves onto content Bob has not received yet | inserts that content | Bob's insert | nothing for Alice's position, then her position once the delta lands | none -- `PRESENCE-008` |
| sends a sample naming herself "Bob" | edits | Bob's edit | the position attributed to Alice, the sender Sync resolved | none -- `PRESENCE-004` |
| jumps to the far end of the document | watches | nothing | her position disappear and reappear there, with no slide between | none -- `PRESENCE-010` |
| inserts a row, a shape or a paragraph above Bob's position | watches | Alice's insert | Bob's position still on the same cell, shape or word | none -- `PRESENCE-007` |
| closes the document | watches | nothing | Alice's position off Bob's screen | none, and it fails -- `PRESENCE-009` |

---

## 3. MODEL

**Containers.** none, in any application, and that is `PRESENCE-001`. A presence sample is the one
thing a collaborator produces that no schema document declares.

**The two shapes, and why there are two.** A peer authors one and reads another. The split is the
identity rule: what a peer may assert about itself, and what the receiver has established.

| Shape | Direction | Carries |
|---|---|---|
| the payload | this peer to the relay, and back from every other | an anchor, and a self-asserted display name |
| the resolved sample | the Logic thread to the view | the resolved `UserId`, the name, the colour, the anchor resolved to a position |

The payload is postcard bytes inside an `Ephemeral` frame, so it is one of the few types in the
suite that keeps `serde`. The resolved sample never leaves the process: it moves by value on
`flume`, and carries none.

**The identity rule, stated once.** The payload carries no `user_id` (`PRESENCE-002`). The canonical
one arrives out of band, on `LogicEvent::RemoteEphemeral { from_user_id }`, resolved by `ambre-sync`
from the key that signed the frame (`PRESENCE-003`). The name inside the payload labels a position
and decides nothing (`PRESENCE-004`). So a peer who writes another member's name into the payload
relabels their own cursor, and nothing else.

**The four steps of an inbound frame:**

1. `ambre-sync` decrypts the frame and resolves the author to a `UserId`, dropping a frame whose key
   matches no member (`SYNC-014`, `SYNC-015`);
2. the Logic thread records the sample under that `UserId`, replacing that peer's previous one, so
   one collaborator on two devices is one entry (`PRESENCE-005`);
3. the anchor is resolved against the current document. An anchor naming no live target is dropped,
   with no error and no record (`PRESENCE-008`);
4. the whole live set travels to the view, which replaces its own (`PRESENCE-009`).

**Why the set travels whole.** Presence is a stream of samples, not a stream of changes. A lost
frame must cost one late position, never a position that stays on the screen for good. A full set
makes the departure of a peer the absence of an entry, which no dropped message can corrupt, and it
is what removes the need for a departure signal that no application has.

**The anchor is the variation point.** Each application names the identities its own document mints,
because a position is what a concurrent structural edit invalidates (`PRESENCE-007`).

| Application | Anchor | Resolved against |
|---|---|---|
| Trame | `CellAnchor { sheet_id, row_uid, col_uid }` -- the uids of `ambre-spec-trame-schema.md section 2` | the sheet index, to a row and a column |
| Toile | a world point, and the `ElementId` set of the peer's selection | the board, to screen space |
| Prose | `CursorPosition { block_id, offset }` -- a `TreeID` and an offset in that block | the block tree, to a caret rectangle |

Trame's declaration is `2-Trame/features/ambre-spec-trame-selection.md section 3`, Toile's is
`3-Toile/features/ambre-spec-toile-presence.md section 3`, and Prose's is
`4-Prose/features/ambre-spec-prose-engine.md section 3`. None of the three is restated here.

**What each application paints** is its own, and each of the three has a row for it: the position in
the peer's colour, and their name beside it.

| Application | Painted as | Row |
|---|---|---|
| Trame | the peer's cell border, and a name badge | `2-Trame/features/ambre-spec-trame-selection.md section 4` |
| Toile | a filled arrow, a label chip, ghost outlines, a stroke and a fading trail | `TOILE-UI-014` |
| Prose | a caret bar as tall as the line, a name chip above it, and a tint over the selected range | `features/ambre-spec-prose-page-rendering.md section 4` |

**The colour is one function, and every application calls it** (`PRESENCE-006`). It lives beside
the drawing trait rather than in an application, so the same collaborator is the same colour in a
spreadsheet, a board and a text document.

```rust
// ambre-draw/src/peer.rs
use ambre_core::UserId;
use peniko::Color;

/// Deterministic per-user color, identical across all apps in the suite.
/// Seeded on the **canonical `UserId`** (spoof-proof, resolved by `ambre-sync`)
/// -- never on the per-device `pub_key`, which changes on key rotation. The id
/// bytes are folded into a hue with FNV-1a and mapped through HSL at a fixed
/// saturation/lightness, so every peer color is vivid, legible on light and
/// dark backgrounds, and distinct from its neighbours; the result is always
/// fully opaque (callers apply per-element alpha themselves).
pub fn peer_color(user_id: &UserId) -> Color { /* ... */ }
```

Seeding on the `UserId` newtype rather than on a raw `Uuid` keeps the type end to end through every
presence path, which is why `ambre-draw` takes a types-only dependency on `ambre-core`. That is the
one exception to its dependency rule (`CANVAS-025`), and its manifest records it.

**Code:** `crates/ambre-draw/src/peer.rs` holds the colour.
`crates/ambre-toile/src/logic/state/presence.rs` and `ui/presence.rs` are the only implementation of
the rest. `crates/ambre-logic/src/data.rs` routes an inbound ephemeral frame to its application, for
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
previous one snaps instead (`PRESENCE-010`): a peer jumping from the first page to the fortieth must
not draw a cursor sliding through thirty-nine pages they never visited.

---

## 5. DECISIONS

| Decision | Rationale | Rejected alternative |
|---|---|---|
| One document for three applications | the transport, the identity, the colour, the keying and the set semantics are one behaviour, and the review that read Prose against Trame found four defects with one root over two documents. Three copies drift in three directions, and the drift is invisible because no reader holds two of them at once | a presence section per application, which is what produced X59, X60, X30 and X15 |
| The anchor stays with the application | an anchor names the identities of one document model, and a spreadsheet has no text offset. The behaviour above is identical for all three; the thing it points at is the one part that cannot be | a shared anchor type parameterized over three models, which buys one type and forces every application through an indirection to reach its own ids |
| Presence is ephemeral, never a document write | a position changes on every gesture and matters for as long as it is on the screen. Writing it would put a per-gesture operation into the history every peer keeps for good, and make a cursor a thing that merges (`PRESENCE-001`) | a presence container pruned on a timer, which grows the file and makes undo step over other people's gestures |
| The payload carries no user identifier | anything a peer writes about itself is something it can write about anyone else. Leaving the field out means there is nothing to forge: the only identity is the one resolved from the key that signed the frame (`PRESENCE-002`) | an author field in the sample, which lets any member paint a cursor under any other member's name |
| Keyed by `UserId`, never by peer or device | a peer identifier is per session and per device, so one collaborator on a laptop and a phone appears as two people in two colours. The `UserId` is the person (`PRESENCE-005`) | keying by the sending peer, which doubles a collaborator every time they open a second tab |
| The colour is hashed from the `UserId` by the shared helper | a collaborator working in a sheet, a board and a document is one person, and one person with three colours is three people to the reader. The helper is pure, so the colour is the same on every device and in every application (`PRESENCE-006`) | a per-application palette assigned in join order, which gives one user a different colour per document and swaps two users' colours on a reconnect |
| The anchor is move-invariant | a remote position must follow its target through the inserts, deletes and sorts a collaborator makes while the frame is in flight. A row number, a screen point or a text offset is exactly what those edits invalidate, so a positional anchor makes remote cursors jump on every structural change (`PRESENCE-007`) | raw coordinates, which is what Trame carried until the review named it X30 |
| An unresolvable anchor is dropped, silently | the sample is one of a stream whose next one is on its way, and the target is missing because a delta is in flight. There is nothing for the user to do and nothing for the log to say (`PRESENCE-008`) | queueing the sample until the target resolves, which holds a cursor at a position the peer has already left |
| The event carries the whole live set | it makes a departure the absence of an entry, so no application needs a departure signal, and none of the three has one. The alternative is what Toile built, and `TOILE-UI-075` is the ghost cursor it produces (`PRESENCE-009`) | one event per peer with a receiver-side accumulation, which needs a removal path, a timeout or a leave message, and leaves a ghost until whichever one is written |
| A non-adjacent move snaps | the interpolation exists to hide 100 ms of latency on a cursor that moved one step. Applied to a jump, it draws a path through content the peer never visited, which is a lie the reader cannot detect (`PRESENCE-010`) | interpolating every move, which is one rule fewer and shows a false path |

---

## 6. NOT BUILT, AND NOT YET RULED

* **Nothing in the suite broadcasts a sample.** Toile receives, keys, routes and paints, and no
  gesture in any of the three applications emits one (`TOILE-UI-073`). The receiving half being
  wired end to end is what makes the gap invisible in every test that exists.
* **Toile's event shape contradicts `PRESENCE-009`.** It emits `PeerPresence { user_id, presence }`
  once per inbound frame and the view accumulates a per-peer mirror, with a removal path nothing
  calls. Trame and Prose both specify the whole set and neither is built, so the decision above is
  taken from two designs against one implementation. The correction is Toile's to make, and
  `TOILE-UI-075` is the row that carries it.
* **Nothing bounds the sample rate.** No document says how often a move broadcasts, and the frame
  budget above is the paint cost alone.
* **Nothing rules on a viewer against an editor.** Every peer paints the same overlay whatever their
  role (`ambre-spec-membership.md`).
* **The label has no source in two of the three.** Toile prints a hex prefix of the `UserId` because
  nothing hands the overlay the display name the member list already carries (`TOILE-UI-076`), and
  Prose has never said where the name in its payload comes from.
* **Nothing rules on presence against optimistic local text.** Prose paints text that is not yet in
  the document, so a remote anchor inside the same block resolves against a document the screen does
  not show (`features/ambre-spec-prose-text-editing.md`).

---

## 7. WHAT THIS DOCUMENT DOES NOT OWN

| Question | Owner |
|---|---|
| How an ephemeral frame is sealed and replay-guarded | `ambre-spec-sealed-envelope.md`; `ZK-005`, `SYNC-008` |
| Which member an ephemeral frame is delivered under | `ambre-spec-membership.md`; `SYNC-014`, `SYNC-015` |
| What the drawing calls that paint an overlay are, and which crate declares them | `ambre-spec-drawing-surface.md` |
| What each application's anchor names, and how it resolves | the three documents named in section 3 |
| What the overlay paints, and in what order | the three rows named in section 3 |
| Where an asynchronous remark is anchored, and what happens when its target dies | the comments feature of each application: the anchor is that feature, and the three anchors share no rule |
