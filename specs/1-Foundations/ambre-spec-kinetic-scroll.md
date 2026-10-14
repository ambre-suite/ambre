# AMBRE -- FEATURE: THE KINETIC SCROLL

**Type:** Horizontal (framework)
**Applications:** Prose, Trame, Toile
**Implementation:** implemented -- the one model, and Prose, Trame and Toile run on it

> A finger moves the view the same way in every editor. A tap stays a tap, a pan follows the finger,
> a flick coasts to a stop, and a drag held near an edge scrolls. A trackpad glide that stops keeps
> its momentum; a wheel notch never does.

Three editors scroll three spaces: a column of pages, a grid of cells, an endless board. One model
turns timed pointer positions into distances on the screen, and each application moves its own view
by them. This document owns the model and the one policy on where momentum comes from; each
application owns what its view does with a distance.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`. The prefix is `KINETIC`.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `KINETIC-001` | `INV` | An application's pan, flick, wheel momentum and edge scroll MUST come from the one shared model. | this spec section 2 | ci-gate -- `scripts/crate_boundaries.py` refuses a decay, a friction, a damping, an inertia, a half-life or a velocity named outside `crates/ambre-draw/src/kinetic.rs`; human-review -- the reviewer refuses a speed estimate or an edge speed an application computes under other names | partial -- `intent` -- no gate tells a speed estimate under a neutral name from any other arithmetic | human-review |

### 1.2 Pan and flick

The user puts a finger down, moves it or not, and lifts it.

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `KINETIC-003` | `POST` | A pointer that stays inside the tap slop MUST be a tap at its press point. | this spec section 2 | `ambre_draw::kinetic::tests::a_pointer_inside_the_slop_is_a_tap_and_moves_nothing` | none | verified |
| `KINETIC-004` | `POST` | A pan released above the stop speed MUST keep moving at the speed of its last sample window. | this spec section 2 | `ambre_draw::kinetic::tests::a_pan_released_carries_on_at_the_speed_of_its_last_window` | none | verified |
| `KINETIC-005` | `POST` | Each tick of a motion MUST move less than the one before, and the motion MUST stop below the stop speed. | this spec section 2 | `ambre_draw::kinetic::tests::each_tick_moves_less_and_the_motion_stops` | none | verified |
| `KINETIC-006` | `FRAME` | A press, or an edge the application meets, MUST stop the motion, and no tick MUST move the view after. | this spec section 2 | `ambre_draw::kinetic::tests::a_press_or_an_edge_stops_the_motion` | none | verified |
| `KINETIC-010` | `FRAME` | A pointer the browser cancels MUST NOT leave a tap or a motion. | this spec section 2 | `ambre_draw::kinetic::tests::a_cancelled_pan_leaves_no_tap_and_no_motion` | none | verified |
| `KINETIC-011` | `POST` | A cancelled pointer that acted as the mouse MUST end its gesture as a release at its last position. | this spec section 3 | none | unheld -- the host turns a `pointercancel` into a release at the last point it saw. A driven browser dispatches the cancel mid-drag and reads the selection end where the pointer was; lands with the `The browser-driven belt` Story (`ambre-roadmap.md` E9) | untested |

### 1.3 Scroll at an edge

The user drags a selection or an object towards an edge of the view and holds it there.

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `KINETIC-007` | `POST` | A drag held in the edge band, or past it, MUST scroll at a speed that grows with its depth, up to a cap. | this spec section 2 | `ambre_draw::kinetic::tests::the_edge_speed_grows_with_depth_and_is_capped` | none | verified |
| `KINETIC-008` | `FRAME` | A drag away from both edges MUST NOT scroll the view. | this spec section 2 | `ambre_draw::kinetic::tests::the_edge_speed_grows_with_depth_and_is_capped` | none | verified |

### 1.4 The wheel turns

The user glides two fingers on a trackpad and lifts them, or turns a notched wheel.

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `KINETIC-002` | `POST` | A trackpad glide that pauses MUST keep the view moving at the glide's speed. | this spec section 4 | `ambre_draw::kinetic::tests::a_glide_that_pauses_carries_on_and_a_notch_never_does`; Prose `ambre_prose::input_tests::a_paused_glide_keeps_the_column_moving_and_a_notch_never_does`; Trame `ambre_trame::ui::view::scroll::tests::a_paused_glide_keeps_the_grid_moving_and_a_notch_never_does`; Toile `ambre_toile::ui::kinetic::tests::a_paused_glide_keeps_the_board_moving_and_a_notch_never_does` | none | verified |
| `KINETIC-009` | `FRAME` | A wheel notch, a line step or a page step MUST NOT start a motion. | this spec section 4 | `ambre_draw::kinetic::tests::a_glide_that_pauses_carries_on_and_a_notch_never_does`; Prose `ambre_prose::input_tests::a_paused_glide_keeps_the_column_moving_and_a_notch_never_does`; Trame `ambre_trame::ui::view::scroll::tests::a_paused_glide_keeps_the_grid_moving_and_a_notch_never_does`; Toile `ambre_toile::ui::kinetic::tests::a_paused_glide_keeps_the_board_moving_and_a_notch_never_does` | none | verified |

* **Budget.** none of its own: a tick is a multiplication and an exponential. What it costs is the
  scroll the application's viewport already carries in `ambre-spec-budgets.md`.
* **Boundary.** none: the model is view state on the UI thread. It writes no document and sends no
  command, so nothing here reaches the relay.
* **Verification.** 9 `verified`, 1 `untested`, 1 `human-review`. The model's rows are unit tests of
  `crates/ambre-draw`, which knows no application. The glide is also held once per application, each
  driven natively with a clock the test sets. Prose's own rows hold what its column does with the
  distances (`4-Prose/features/ambre-spec-prose-viewport.md`), and Trame's grid rows what its grid
  does (`2-Trame/features/ambre-spec-trame-grid-viewport.md`).

---

## 2. MODEL

**Containers.** none. A press, its samples and the motion are view state on the UI thread, owned by
the application's view state.

**The value.** A pan in progress, or a motion with a speed on each axis, in screen pixels per
millisecond. An application feeds it pointer positions and frame times, and reads back distances.

```rust
/// One pointer's pan, then the motion its release left.
pub struct Kinetic { /* the press and the motion, private */ }

/// What a release leaves: a tap at the press point, a motion, or nothing.
pub enum Release { Tap { x: f32, y: f32 }, Fling, Rest }

/// The speed a drag held at `at` scrolls a view of length `extent` at.
pub fn edge_speed(at: f32, extent: f32) -> f32;
```

**The constants.** One set, so the gesture feels the same in every editor.

| Constant | Value | What it sets |
| :--- | :--- | :--- |
| tap slop | 8 px | how far a pointer moves and stays a tap |
| sample window | 100 ms | how much of the pan the release speed reads |
| slowdown | a time constant of 325 ms | how fast a motion dies |
| stop speed | 0.02 px/ms | below it a release leaves nothing and a motion stops |
| longest step | 50 ms | one stalled frame moves the view no further |
| edge band | 32 px | how near an edge a drag starts to scroll |
| edge depth | 160 px | how far past the band the speed still grows |
| edge speed | 2 px/ms at the cap | the fastest an edge scrolls |

**A flick, in steps.** Only step 4 is the application's own.

1. a press stops any motion and records the point (`KINETIC-006`);
2. each move inside the slop moves nothing, and past it hands back the step the view follows
   (`KINETIC-003`);
3. the release reads the speed over the sample window, and leaves a motion above the stop speed
   (`KINETIC-004`);
4. each frame, the application moves its view by the tick's distance, and reports an edge it met
   (`KINETIC-005`, `KINETIC-006`).

**Code.** `ambre_draw::kinetic`. It sits beside the drawing primitives every application already
depends on, and it depends on nothing.

---

## 3. SURFACE

| Trigger | Behaviour |
|---|---|
| a finger down, lifted without moving | a tap where it went down (`KINETIC-003`) |
| a finger dragged | the view follows it (`KINETIC-003`) |
| a finger lifted while moving | the view coasts and slows to a stop (`KINETIC-004`, `KINETIC-005`) |
| a press during a coast | the coast stops (`KINETIC-006`) |
| the browser cancels a finger that pans | the pan ends, with no tap and no coast (`KINETIC-010`) |
| the browser cancels a pointer that acts as the mouse | the gesture ends where the pointer last was, as a release (`KINETIC-011`) |
| a selection or an object dragged near an edge | the view scrolls, faster the deeper it goes (`KINETIC-007`) |
| a trackpad glide | the view follows each step, and coasts once the glide pauses (`KINETIC-002`) |
| a wheel notch, a line or a page step | the view takes the step once, with no coast, and a coast under way stops (`KINETIC-009`) |

---

## 4. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| A pixel glide that pauses coasts, a notch never does | leaving every wheel momentum to the platform | Firefox on Linux sends no momentum after a two-finger flick, so the sheet stops dead under the fingers. Where the platform does send it, its events keep the glide alive, and the model's coast starts only once they fade below the start speed |
| A cancelled mouse-like gesture ends as a release | reverting the gesture | a system gesture cancels a pointer without the user asking: a notification, an edge swipe. Reverting would throw away a stroke or a move the user meant, and a release keeps what they see |
| A pixel step past the notch size is a notch | telling a trackpad from a mouse by the device | the browser names neither. A notch arrives as one large step or in line mode; a glide stays under the size |
| One model with one set of constants | a model per application | two curves under one gesture make the same flick travel two distances, and the user feels the switch from a text to a sheet |
| The model hands back distances, and the application moves its view | a model that owns a scroll offset | a column scrolls in pixels, a grid by whole cells with a remainder, a board in two dimensions under a zoom. Owning the offset would force one space on three |
| The application reports the edge it met | bounds handed to the model | the bounds move with every layout, and a grid meets an edge as a whole cell. The application already knows, after it moves, whether it moved |
| The model lives in `ambre-draw` | a crate of its own | every application already depends on `ambre-draw`, and the model depends on nothing. A crate for a hundred lines costs a manifest and an entry in the crate map, and buys no boundary the module does not have |

---

## 5. NOT BUILT, AND NOT YET RULED

* **Nothing further is owed here.** Pinch and the zoom it carries belong to the `One zoom model`
  Story (`ambre-roadmap.md` V1).

---

## 6. WHAT THIS DOCUMENT DOES NOT OWN

| Question | Owner |
|---|---|
| What a page column does with a distance, and what stops a flick there | `4-Prose/features/ambre-spec-prose-viewport.md` |
| What a grid does with a distance | `2-Trame/features/ambre-spec-trame-grid-viewport.md` |
| What a board does with a distance | `3-Toile/features/ambre-spec-toile-viewport.md` |
| The pinch, and every zoom | `ambre-spec-zoom.md` |
