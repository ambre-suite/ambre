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

| ID | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|
| `TOILE-008` | Stroke smoothing MUST NOT be persisted: the document stores the raw sampled points only. | this spec section 3 | `ambre_toile::logic::stroke::tests::passes_through_samples_and_is_denser`, `::empty_and_single_point`; `ambre_toile::logic::convergence_tests::commit_stroke_creates_drawing` writes the raw sample vector | verified |
| `TOILE-038` | A stroke MUST NOT be mutated after it is committed, so two strokes MUST NOT conflict. | this spec section 3 | structural, and the construction is the proof: no command names a stroke's sample vector, so a stroke is an insert and nothing else. `ambre_toile::logic::convergence_tests::two_peers_converge` drives concurrent stroke commits | verified |
| `TOILE-039` | The smoothed outline MUST pass through every sample, and MUST be denser than the samples. | this spec section 3 | `ambre_toile::logic::stroke::tests::passes_through_samples_and_is_denser`, `::empty_and_single_point` | verified |
| `TOILE-040` | An in-flight stroke MUST travel as presence, and MUST NOT enter the document before release. | this spec section 4; `TOILE-016` | `ambre_toile::ui::tool::tests::draw_stream_emits_commit_stroke`, `ambre_toile::logic::convergence_tests::presence_command_broadcasts_ephemeral_only`, `ambre_toile::ui::presence::tests::live_stroke_draws_a_polyline` | verified |
| `TOILE-041` | A stroke's box MUST span its samples and MUST have area, so a single dot stays cullable and pickable. | this spec section 3 | partial: `ambre_toile::logic::convergence_tests::commit_stroke_creates_drawing` asserts the box exists for a two-sample stroke. Nothing drives the one-sample case through the engine, which is where the minimum applies | unverified |

* **Budget.** the stroke commit under 1 ms, the transform-command row of
  `ambre-spec-budgets.md section 1`. Smoothing runs
  again at every derivation, so its real budget is the frame: it is inside the visible-set walk of
  `features/ambre-spec-toile-board-rendering.md`, and no bench measures either.
* **Boundary.** none of its own: the samples enter the document and leave as bytes the Sync worker
  seals (`ZK-001`). What streams during the gesture is an ephemeral frame, sealed the same way
  (`ZK-005`).
* **Verification.** 4 `verified`, 1 `unverified`. The whole rule set of the feature holds, including
  the one that matters most -- nothing derived is stored. The gap is the degenerate input.

---

## 2. WHAT EACH USER SEES

| Alice does | Bob does | The document holds | Each user sees | Test |
|---|---|---|---|---|
| draws a stroke | draws a stroke at the same moment | both strokes, as two independent objects | both lines, in the order the two inserts converged | `logic::convergence_tests::two_peers_converge` |
| is drawing, pointer down | watches | nothing yet | Alice's line growing under her pointer, as presence on Bob's screen | `ui::presence::tests::live_stroke_draws_a_polyline` |
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

1. mint an id and compute the box spanning the samples, widened to at least one unit each way so a
   dot has area (`TOILE-041`);
2. write the geometry block, the style register and the samples as one binary field;
3. push the stroke onto the top of the paint order;
4. commit once, then record the box in the bounds cache and the spatial index.

**Steps, when a stroke is painted:** the deriver reads the samples back and runs a Catmull-Rom pass
through them, duplicating the endpoints so the first and last spans have a tangent, and hands the
result out as a flattened outline (`TOILE-039`). The density comes from the zoom, so the same samples
give a coarse curve far out and a fine one close in.

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
| pointer move while down | appends a sample, draws the line locally, and broadcasts the in-flight stroke as presence (`TOILE-040`) |
| pointer up | sends `CommitStroke` once, with every sample of the gesture |
| `Escape` mid-gesture | drops the stroke: nothing is committed (`features/ambre-spec-toile-tools.md`) |
| switching tools mid-gesture | the same cancellation |

**What the drawer sees.** The line under the pointer is painted locally from the samples already
taken, so it never waits for a round trip. On release the same line is repainted from the document,
smoothed the same way, so there is no visible change at the seam.

**What the others see.** A polyline in the drawer's presence colour, rebuilt from every frame that
arrives, replaced by the real stroke when the commit lands
(`features/ambre-spec-toile-presence.md`).

---

## 5. DECISIONS

| Decision | Rationale | Rejected alternative |
|---|---|---|
| The samples are stored and the curve is not | the curve is a pure function of the samples and the zoom, so storing it would cost several times the samples, freeze the density at the zoom it was drawn at, and lose nothing recoverable | persisting the smoothed curve, which the earlier design carried |
| A stroke is an insert and never an update | one author draws one stroke in one gesture, so there is no second writer to converge with. That is why the whole feature needs no conflict rule | a mutable stroke, editable point by point, which would need a sequence CRDT per stroke |
| The gesture streams as presence, not as deltas | a stroke at 60 samples a second would put a thousand operations per second into the history for a line the user may abandon | streaming the samples as document updates |
| Smoothing runs at derivation, not at commit | the density depends on the zoom, which the writer does not know and the reader does | smoothing once at commit, at a fixed density |
| A single sample keeps a minimum area | a zero-area box is invisible to the index, so a dot would be uncullable and unpickable -- present in the document and unreachable on screen | the exact box, which for one point is empty |

---

## 6. NOT BUILT, AND NOT YET RULED

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
