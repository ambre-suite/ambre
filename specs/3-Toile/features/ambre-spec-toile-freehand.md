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

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TOILE-008` | `INV` | Stroke smoothing MUST NOT be persisted: the document stores the raw sampled points only. | this spec section 3 | structural -- the commit serializes the sample vector and writes no outline field, and the smoothing pass runs only at derivation, so a smoothed vector has no field to land in. Nothing executable would catch one written in the samples' place: `ambre_toile::logic::convergence_tests::commit_stroke_creates_drawing` reads the box back and never the samples | none | verified |
| `TOILE-038` | `INV` | A stroke MUST NOT be mutated after it is committed, so two strokes MUST NOT conflict. | this spec section 3 | structural -- no command names a stroke's sample vector, so a stroke is an insert and nothing else; `ambre_toile::logic::convergence_tests::two_peers_converge` -- it drives concurrent stroke commits | none | verified |
| `TOILE-039` | `INV` | The smoothed outline MUST pass through every sample. | this spec section 3 | `ambre_toile::logic::stroke::tests::the_outline_passes_through_every_sample` -- each span ends on its sample | none | verified |
| `TOILE-103` | `INV` | The smoothed outline MUST be denser than the samples. | this spec section 3 | `ambre_toile::logic::stroke::tests::passes_through_samples_and_is_denser` -- it reads the output length, and this is the half the row above was carrying | none | verified |
| `TOILE-104` | `INV` | A stroke of no sample MUST smooth to nothing. | this spec section 3 | `ambre_toile::logic::stroke::tests::empty_and_single_point` -- its first half smooths an empty slice and requires an empty result, so a smoother that seeded a point fails it | none | verified |
| `TOILE-259` | `INV` | A stroke of one sample MUST smooth to exactly that point. | this spec section 3 | `ambre_toile::logic::stroke::tests::empty_and_single_point` -- its second half smooths one sample and compares the output against that point, so neither a dropped nor a moved point passes | none | verified |
| `TOILE-268` | `INV` | A stored pressure outside 0 to 1 MUST read as the nearest bound, and an unreadable one as full pressure, so the stroke still draws. | this spec section 3 | `ambre_toile::logic::state::derive::tests::an_out_of_range_pressure_reads_clamped` -- it writes three samples with the pressures 2.0, -0.5 and NaN past the commands, then reads a drawn stroke and no undecodable notice | none | verified |

### 1.2 Draw a stroke

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TOILE-040` | `POST` | Releasing the pointer MUST send one commit carrying every sample of the gesture. | this spec section 4 | `ambre_toile::ui::tool::tests::draw_stream_emits_commit_stroke` -- it walks a press, two moves and a release and reads the one command back, with a sample per event | none | verified |
| `TOILE-105` | `FRAME` | An in-flight stroke MUST NOT enter the document before release. | this spec section 4 | `ambre_toile::ui::tool::tests::a_stroke_in_flight_writes_nothing` | none | verified |
| `TOILE-106` | `POST` | While a user draws, each new sample MUST reach the other users as presence. | this spec section 4; `TOILE-016` | `ambre_toile::ui::view::view_tests::each_frame_offers_the_stroke_being_drawn_to_the_others` -- the sampler; `ambre_toile::ui::tool::tests::a_stroke_being_drawn_is_offered_to_the_presence_sampler` -- the samples the machine offers; `ambre_toile::logic::convergence_tests::a_stroke_in_flight_reaches_the_other_users` -- the frame and the watcher; `ambre_toile::ui::presence::tests::live_stroke_draws_a_polyline` -- the trail | none | verified |
| `TOILE-041` | `POST` | A stroke's box MUST span its samples. | this spec section 3 | `ambre_toile::logic::convergence_tests::a_stroke_box_spans_its_samples` | none | verified |
| `TOILE-107` | `POST` | A stroke's box MUST have area, so a single dot stays cullable and pickable. | this spec section 3 | `ambre_toile::logic::convergence_tests::a_dot_has_a_box_with_area` | none | verified |
| `TOILE-108` | `FAIL` | A stroke abandoned by `Escape` or a tool change MUST commit nothing. | this spec section 4 | `ambre_toile::ui::tool::tests::an_abandoned_stroke_commits_nothing` -- `Escape` and a tool change | none | verified |

* **Budget.** the stroke commit under 1 ms, the transform-command row of
  `ambre-spec-budgets.md section 1`. Smoothing runs
  again at every derivation, so its real budget is the frame. It is inside the visible-set walk of
  `features/ambre-spec-toile-board-rendering.md`, and no bench measures either.
* **Boundary.** none of its own: the samples enter the document and leave as bytes the Sync worker
  seals (`ZK-001`). What streams during the gesture is an ephemeral, sealed the same way
  (`ZK-005`).
* **Verification.** 13 `verified`. What holds is the committed stroke and the stroke in flight. The
  tests read the samples, the box a stroke spans, the minimum area of a dot, and the abandoned
  gesture that commits nothing. One test reads a stored pressure out of range that still draws
  (`TOILE-268`). The view test reads the live stroke each frame offers to the other peers
  (`TOILE-106`).

---

## 2. WHAT EACH USER SEES

| Alice does | Bob does | The document holds | Each user sees | Test |
|---|---|---|---|---|
| draws a stroke | draws a stroke at the same moment | both strokes, as two independent objects | both lines, in the order the two inserts converged | `logic::convergence_tests::two_peers_converge` |
| is drawing, pointer down | watches | nothing yet | Alice's line growing under her pointer, and the same line growing on Bob's screen in her colour (`TOILE-106`) | `logic::convergence_tests::a_stroke_in_flight_reaches_the_other_users`, and `ui::presence::tests::live_stroke_draws_a_polyline` for the painting |
| releases | watches | one stroke object | the same smoothed line on both screens, recomputed from the same samples | `logic::stroke::tests::passes_through_samples_and_is_denser` |
| deletes her stroke | draws over where it was | the deletion and the new stroke, unrelated | Bob's line alone | `logic::convergence_tests::caches_match_a_full_rebuild` |

---

## 3. MODEL

**Containers.** None of its own. A stroke is an element of the `elements` map whose kind is the
drawing kind, carrying its samples in one binary field
(`ambre-spec-toile-schema.md section 3`).

**One sample** is a world point plus a pressure reading, twelve bytes with a fixed layout:

```rust
/// One sampled point of a freehand stroke. 12 bytes, fixed layout: three
/// `f32`s, the coordinates in board units then the pressure.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct StrokePoint {
    pub x:        Px<StoredBoard>,
    pub y:        Px<StoredBoard>,
    /// `Bounded::<UnitFraction>::ONE` when the input device reports none. A writer
    /// refuses a value outside `0 ..= 1`; the reader clamps it (`TOILE-268`).
    pub pressure: Bounded<UnitFraction>,
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
through them. It duplicates the endpoints so the first and last spans have a tangent. The result is
handed out as a flattened outline (`TOILE-039`, `TOILE-103`, `TOILE-104`). The density comes from the
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
| pointer move while down | appends a sample and draws the line locally (`TOILE-105`); the frame hook broadcasts the tail as presence (`TOILE-106`) |
| pointer up | sends `CommitStroke` once, with every sample of the gesture (`TOILE-040`) |
| `Escape` mid-gesture | drops the stroke: nothing is committed (`TOILE-108`, `features/ambre-spec-toile-tools.md`) |
| switching tools mid-gesture | the same cancellation |

**What the drawer sees.** The line under the pointer is painted locally from the samples already
taken, so it never waits for a round trip. On release the same line is repainted from the document,
smoothed the same way, so there is no visible change at the seam.

**What the others see.** A polyline in the drawer's presence colour, rebuilt from every sample that
arrives and replaced by the real stroke when the commit lands. The sample carries the tail of the
gesture, never its whole history: what a peer needs is where the pen is now (`TOILE-106`,
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

* **Pressure is recorded and never read.** Every sample is written at full pressure by the tool,
  which never reads a pointer pressure value. No paint path varies the width by it. The field is
  four bytes per sample carrying nothing, and a pressure-sensitive pen would change nothing on
  screen.
* **An eraser does not exist.** A stroke is removed by selecting it and deleting it, whole. There is
  no partial erase, and none is designed. A partial erase would have to split one stroke into two,
  which contradicts the insert-only rule above.
* **The width does not follow the zoom.** The stroke is drawn at the width its style register
  holds, in world units. Zooming out makes a thick stroke thin like any other outline. Whether a
  stroke should keep a screen-space width is not ruled.
* **Nothing bounds the sample count.** A long slow drag records a sample per input event for as long
  as the user holds the pointer. No rule caps the vector or thins it. What that costs a board of
  a thousand strokes is unmeasured (`ambre-roadmap.md`, "Engine and render optimization").
