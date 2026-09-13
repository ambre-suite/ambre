# AMBRE -- FEATURE: FREEHAND DRAWING

**Type:** Vertical (feature)
**Implementation:** partial -- drawing ships; pressure is recorded and never used.

> A user drags with the pen and a line follows the pointer. Other users watch it appear as it is
> drawn. On release it becomes one object on the board, smooth rather than angular.

A stroke has one author and is never edited afterwards, which is what makes it the cheapest thing on
the board to synchronize.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|---|
| `TOILE-008` | `INV` | Stroke smoothing MUST NOT be persisted: the document stores the raw sampled points only. | this spec section 3 | structural, and the construction is the proof: the commit serializes the sample vector and writes no outline field, and the smoothing pass runs only at derivation. `ambre_toile::logic::convergence_tests::commit_stroke_creates_drawing` reads the box back and never the samples, so nothing executable would catch a smoothed vector written in their place | verified |
| `TOILE-038` | `INV` | A stroke MUST NOT be mutated after it is committed, so two strokes MUST NOT conflict. | this spec section 3 | structural, and the construction is the proof: no command names a stroke's sample vector, so a stroke is an insert and nothing else. `ambre_toile::logic::convergence_tests::two_peers_converge` drives concurrent stroke commits | verified |
| `TOILE-039` | `INV` | The smoothed outline MUST pass through every sample. | this spec section 3 | none. `ambre_toile::logic::stroke::tests::passes_through_samples_and_is_denser` reads back the first and the last output point only, so a pass that cut the corner at every interior sample would satisfy it | unverified |
| `TOILE-103` | `INV` | The smoothed outline MUST be denser than the samples. | this spec section 3 | `ambre_toile::logic::stroke::tests::passes_through_samples_and_is_denser` on the output length, and it is the half the row above was carrying | verified |
| `TOILE-104` | `INV` | Smoothing MUST survive a stroke too short to curve: no sample MUST give nothing, and one sample MUST give exactly that point. | this spec section 3 | `ambre_toile::logic::stroke::tests::empty_and_single_point`, which is the only thing that drives either degenerate input and was cited as though it held the density rule | verified |

### 1.2 Draw a stroke

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|---|
| `TOILE-040` | `POST` | Releasing the pointer MUST send one commit carrying every sample of the gesture. | this spec section 4 | `ambre_toile::ui::tool::tests::draw_stream_emits_commit_stroke` walks a press, two moves and a release and reads the one command back, with a sample per event | verified |
| `TOILE-105` | `FRAME` | An in-flight stroke MUST NOT enter the document before release. | this spec section 4 | none. Sampling appends to view state and the move returns no command, so a create per sample is unreachable by construction; the pen test discards what each move returned, so nothing executable holds it | unverified |
| `TOILE-106` | `POST` | While a user draws, each new sample MUST reach the other users as presence. | this spec section 4; `TOILE-016` | none, and **the rule fails today**: nothing in the workspace sends a presence payload for a board -- the in-flight sample field is written by tests only, and the presence command has no sender at all. `ambre_toile::ui::presence::tests::live_stroke_draws_a_polyline` paints a frame that arrives and `ambre_toile::logic::convergence_tests::presence_command_broadcasts_ephemeral_only` drives the command with no stroke in it, so both hold the receiving half -- section 6. Answered by the `A stroke in flight reaches the other users` Story (`ambre-roadmap.md` E7) | unverified |
| `TOILE-041` | `POST` | A stroke's box MUST span its samples. | this spec section 3 | none. `ambre_toile::logic::convergence_tests::commit_stroke_creates_drawing` asserts a box exists for a two-sample stroke and never compares it to the samples, so a box of the wrong extent would pass | unverified |
| `TOILE-107` | `POST` | A stroke's box MUST have area, so a single dot stays cullable and pickable. | this spec section 3 | none, and it is the half `TOILE-041` was carrying. The widening is a one-unit floor on each side; nothing drives a one-sample stroke through the engine, which is where the floor applies | unverified |
| `TOILE-108` | `FAIL` | A stroke abandoned by `Escape` or a tool change MUST commit nothing. | this spec section 4 | none for the pen. `ambre_toile::ui::tool::tests::switching_tools_cancels_in_flight_gesture` cancels a Shape draft and `::escape_reverts_an_armed_creation_tool_to_select` an armed Sticky; a stroke is the gesture that carries the most work to lose | unverified |

* **Budget.** the stroke commit under 1 ms, the transform-command row of
  `ambre-spec-budgets.md section 1`. Smoothing runs
  again at every derivation, so its real budget is the frame: it is inside the visible-set walk of
  `features/ambre-spec-toile-board-rendering.md`, and no bench measures either.
* **Boundary.** none of its own: the samples enter the document and leave as bytes the Sync worker
  seals (`ZK-001`). What streams during the gesture is an ephemeral frame, sealed the same way
  (`ZK-005`).
* **Verification.** 5 `verified`, 6 `unverified`. Five rows become eleven, and the feature does not
  hold the way the old bullet said. `TOILE-040` was two rules, and the one it hid is false: nothing
  in the workspace sends a board presence payload, so an in-flight stroke reaches nobody
  (`TOILE-106`). `TOILE-039` claimed the outline passes through every sample and its test reads the
  two endpoints (`TOILE-039`, `TOILE-103`). `TOILE-041` was the span and the minimum area under one
  verdict, and neither half is driven (`TOILE-041`, `TOILE-107`). `TOILE-008` cited two smoothing
  tests that never touch the document; what holds it is the construction, and the row now says so.
  The abandoned gesture (`TOILE-108`) and the degenerate smoothing input (`TOILE-104`) were
  unwritten, the second by a test cited for a rule it does not hold.

---

## 2. WHAT EACH USER SEES

| Alice does | Bob does | The document holds | Each user sees | Test |
|---|---|---|---|---|
| draws a stroke | draws a stroke at the same moment | both strokes, as two independent objects | both lines, in the order the two inserts converged | `logic::convergence_tests::two_peers_converge` |
| is drawing, pointer down | watches | nothing yet | Alice's line growing under her pointer, and nothing on Bob's screen until she releases: the presence frame is painted and never sent (`TOILE-106`) | `ui::presence::tests::live_stroke_draws_a_polyline`, on the painting alone |
| releases | watches | one stroke object | the same smoothed line on both screens, recomputed from the same samples | `logic::stroke::tests::passes_through_samples_and_is_denser` |
| deletes her stroke | draws over where it was | the deletion and the new stroke, unrelated | Bob's line alone | `logic::convergence_tests::caches_match_a_full_rebuild` |

---

## 3. MODEL

**Containers.** None of its own. A stroke is an element of the `elements` map whose kind is the
drawing kind, carrying its samples in one binary field
(`ambre-spec-toile-schema.md section 3`).

**One sample** is a world point plus a pressure reading, twelve bytes with a fixed layout:

```rust
/// One sampled point of a freehand stroke. 12 bytes, fixed layout.
#[derive(Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct StrokePoint {
    pub x: f32,
    pub y: f32,
    pub pressure: f32, // 0.0 ..= 1.0, 1.0 when the input device reports none
}
```

**Command** (`ambre-bus`): `CommitStroke { points, style }`, one undo item. There is no create
command for a stroke and no edit command: the commit is the whole lifecycle
(`TOILE-022`, `TOILE-038`).

**Steps, when a stroke is committed:**

1. mint an id and compute the box spanning the samples (`TOILE-041`), widened to at least one unit
   each way so a dot has area (`TOILE-107`);
2. write the geometry block, the style register and the samples as one binary field;
3. push the stroke onto the top of the paint order;
4. commit once, then record the box in the bounds cache and the spatial index.

**Steps, when a stroke is painted:** the deriver reads the samples back and runs a Catmull-Rom pass
through them, duplicating the endpoints so the first and last spans have a tangent, and hands the
result out as a flattened outline (`TOILE-039`, `TOILE-103`, `TOILE-104`). The density comes from the
zoom, so the same samples give a coarse curve far out and a fine one close in.

**Code:**

* `crates/ambre-toile/src/logic/stroke.rs` -- the smoothing pass;
* `logic/state/elements.rs` -- the commit;
* `logic/state/codec.rs` -- the box over the samples;
* `logic/state/derive.rs` -- the outline handed to the UI thread;
* `ui/tool.rs` -- the sampling and the streaming.

---

## 4. SURFACE

| Trigger | Behaviour |
|---|---|
| the pen tool, pointer down | starts a stroke and samples the first point |
| pointer move while down | appends a sample and draws the line locally (`TOILE-105`); the broadcast the row promises has no sender (`TOILE-106`) |
| pointer up | sends `CommitStroke` once, with every sample of the gesture (`TOILE-040`) |
| `Escape` mid-gesture | drops the stroke: nothing is committed (`TOILE-108`, `features/ambre-spec-toile-tools.md`) |
| switching tools mid-gesture | the same cancellation |

**What the drawer sees.** The line under the pointer is painted locally from the samples already
taken, so it never waits for a round trip. On release the same line is repainted from the document,
smoothed the same way, so there is no visible change at the seam.

**What the others see.** Nothing, today. The receiving half is built -- a polyline in the drawer's
presence colour, rebuilt from every frame that arrives and replaced by the real stroke when the
commit lands -- and no surface sends the frame (`TOILE-106`,
`features/ambre-spec-toile-presence.md`).

---

## 5. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| The samples are stored and the curve is not | persisting the smoothed curve, which the earlier design carried | the curve is a pure function of the samples and the zoom, so storing it would cost several times the samples, freeze the density at the zoom it was drawn at, and lose nothing recoverable |
| A stroke is an insert and never an update | a mutable stroke, editable point by point | it would need a sequence CRDT per stroke. One author draws one stroke in one gesture, so there is no second writer to converge with -- which is why the whole feature needs no conflict rule |
| The gesture streams as presence, not as deltas | streaming the samples as document updates | a stroke at 60 samples a second would put a thousand operations per second into the history for a line the user may abandon |
| Smoothing runs at derivation, not at commit | smoothing once at commit, at a fixed density | the density depends on the zoom, which the writer does not know and the reader does |
| A single sample keeps a minimum area | the exact box, which for one point is empty | a zero-area box is invisible to the index, so a dot would be uncullable and unpickable -- present in the document and unreachable on screen |

---

## 6. NOT BUILT, AND NOT YET RULED

* **Nothing sends a stroke while it is drawn** (`TOILE-106`). The payload carries an in-flight
  sample field, the presence painter draws it, and no code in the workspace ever fills it: the
  presence command has no sender at all, for a stroke or for a cursor. The watching half of the
  feature is written, tested and unreachable, and the spec read as though it shipped.
* **Pressure is recorded and never read.** Every sample is written at full pressure by the tool,
  which never reads a pointer pressure value, and no paint path varies the width by it. The field is
  four bytes per sample carrying nothing, and a pressure-sensitive pen would change nothing on
  screen.
* **An eraser does not exist.** A stroke is removed by selecting it and deleting it, whole. There is
  no partial erase, and none is designed: a partial erase would have to split one stroke into two,
  which contradicts the insert-only rule above.
* **The width does not follow the zoom.** The stroke is drawn at the width its style register
  holds, in world units, so zooming out makes a thick stroke thin like any other outline. Whether a
  stroke should keep a screen-space width is not ruled.
* **Nothing bounds the sample count.** A long slow drag records a sample per input event for as long
  as the user holds the pointer, and no rule caps the vector or thins it. What that costs a board of
  a thousand strokes is unmeasured (`ambre-roadmap.md`, "Engine and render optimization").
