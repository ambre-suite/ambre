# AMBRE — SPEC: TOILE FEATURES (Presentation, Auto-Layout, Collaboration)
**Type:** Vertical (Feature / Application — Features)
**Status:** Draft

> Higher-level Toile capabilities layered on the engine (`toile-logic`) and renderer (`toile-ui`):
> presentation mode (frame sequencing, Smart Animate transitions, focus mode, presenter overlay), the
> auto-layout constraint solver, vector boolean operations, and the real-time collaboration primitives that
> *do* touch the CRDT (votes and reactions) versus those that stay ephemeral (highlighters/laser, live
> cursors — their render is in `toile-ui §8`).
>
> **Scope of this spec:** the feature logic and its CRDT/ephemeral placement. It reuses `ToileState`'s command
> plane (`toile-logic §8`) and `DisplayViewport` (`§9`); it does **not** redefine the element model, spatial
> index, rendering pipeline, or import/export (that is `toile-interop`).

---

## 0. EXECUTION MODEL — COMMANDS, EVENTS, OUTCOME

Every feature here is expressed as additional `ToileCommand` variants handled synchronously in
`ToileState::handle_command`, returning a `ToileOutcome` (`toile-logic §8`). Presentation *playback* state and
selection are **local/ephemeral**; the presentation *sequence* (which frames, in what order) and votes/reactions
are **CRDT** (shared, persisted). This split is the through-line of the spec: persist shared intent, keep
per-viewer playback local.

---

## 1. PRESENTATION MODE

Presentation is **not a separate application** — it is an orchestrated view of the same board. A frame becomes a
"slide"; a side panel orders frames into a narrative thread.

### A. Sequence (CRDT)

```
LoroDoc
└── "presentation" (LoroMap)
    ├── "order"   (LoroMovableList<FrameId>)  → the slide sequence (reorderable, convergent)
    └── "config"  (LoroMap)                   → {transition_ms, autoplay, loop}
```

> **`FrameId` is an `ElementId`.** A frame is an ordinary element (`toile-logic §2.F`); `FrameId` is just the
> `ElementId` of an element whose `type = "frame"`. `presentation.order` is a top-level `LoroDoc` list (it is
> board-global sequencing, not per-frame content, so it stays outside the frame elements).

The sequence references `FrameId`s (stable ids from `toile-logic §2`), never positions, so reordering slides or
editing a frame's content never breaks the deck. Concurrent reorders converge through `LoroMovableList`;
concurrent inserts of the same frame are idempotent-deduped at derivation.

### B. Playback (local / ephemeral)

```rust
pub struct PlaybackState {
    pub active: usize,          // index into presentation.order (this viewer only)
    pub focus:  bool,           // Focus Mode: chrome hidden, camera locked to the active frame
}
```

`NextSlide`/`PrevSlide`/`GotoSlide` mutate `PlaybackState` locally and drive the `toile-ui` camera; they emit
**no** CRDT delta. Optionally a presenter can broadcast their `active` index as **ephemeral presence**
("follow the presenter"), never as history.

### C. Smart Animate (transitions)

Moving between slide A and slide B, Toile interpolates properties of elements that appear in **both** frames and
share the same stable `ElementId` — position, size, rotation, opacity, fill — with an eased `lerp`; elements only
in A fade out, only in B fade in.

```rust
/// Pure, deterministic; runs each frame of the transition in toile-ui's camera pass.
/// No CRDT mutation -- it animates the *presentation* of existing elements.
pub fn smart_animate(
    from: &FrameSnapshot, to: &FrameSnapshot, t: f64 /* 0..=1, eased */,
) -> Vec<AnimatedElement> {
    to.elements.iter().map(|e| {
        match from.by_id(e.id) {
            Some(a) => AnimatedElement::lerp(a, e, t),      // shared id -> tween
            None    => AnimatedElement::fade_in(e, t),      // new in B
        }
    }).chain(
        from.elements.iter()
            .filter(|a| to.by_id(a.id).is_none())
            .map(|a| AnimatedElement::fade_out(a, t))       // gone in B
    ).collect()
}
```

Because matching is by **stable id**, an element the author moved/resized between two frames animates smoothly
"for free" — the Figma Smart Animate model. The interpolation is a `toile-ui` camera-pass concern fed by two
`FrameSnapshot`s the engine derives.

> **Frame membership is explicit, not spatial.** A `FrameSnapshot`'s elements are the frame's declared
> `children` (`toile-logic §2.C`), **not** whatever happens to overlap the frame's rect. This keeps matching
> deterministic and lets an element belong to a slide even if it visually spills outside the frame bounds. An
> element in no frame never animates.

### D. Focus Mode & Presenter Overlay

- **Focus Mode:** hides the editing chrome and auto-centers/zooms the camera on the active frame's rect
  (`toile-ui` transform); purely local.
- **Presenter Overlay:** a floating panel showing the active frame's **speaker notes**. Notes are per-frame
  content, stored in the frame **element's** payload (`toile-logic §2.C`), edited via `EditNotes`:

```
"elements" / "{frame_id}" / payload / "notes" : LoroText   // collaborative speaker notes
```

---

## 2. AUTO-LAYOUT (CONSTRAINT SYSTEM)

A **frame** can carry an auto-layout spec so its children are positioned by a solver rather than by absolute
coordinates (the Figma model). Auto-layout is a frame-only feature: a plain **group** (`toile-logic §2.F bis`)
is a transform-binding container with no solver — promote it to a frame to get auto-layout. The **spec** is CRDT (shared design intent); the **solved positions** are
derived, never persisted (like connector geometry in `toile-logic §5`).

```
"elements" / "{frame_id}" / payload / "autolayout" : Binary(postcard(AutoLayout))   // absent = free positioning
```

```rust
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct AutoLayout {
    pub direction: Direction,   // Horizontal | Vertical | Wrap
    pub gap:       f64,
    pub padding:   [f64; 4],    // top, right, bottom, left
    pub align:     Align,       // Start | Center | End | SpaceBetween
    pub sizing:    Sizing,      // Hug | Fill | Fixed
}

/// Pure solver: children order = the frame's `children` LoroMovableList.
/// Runs at derivation; output feeds x/y/w/h into the DisplayViewport, and a
/// commit-on-edit writes the resolved geometry back only when the user drops an
/// element (so the CRDT keeps absolute coords as the durable truth).
pub fn solve_layout(frame: kurbo::Rect, layout: &AutoLayout, children: &[kurbo::Size]) -> Vec<kurbo::Rect> { /* ... */ }
```

Reordering `children` (`LoroMovableList`) reflows the layout convergently; adding/removing a child reflows the
rest. The solver is deterministic so all peers derive identical positions from identical CRDT state.

---

## 3. VECTOR BOOLEAN OPERATIONS

Union / Subtract / Intersect / Exclude over selected `shape:path` (and shape) elements, computed client-side with
`kurbo` path booleans, producing a **new/edited** `BezPath` element (one `RestyleElements`/geometry command). The
source elements are deleted or kept per the user's choice. This is a pure geometry transform — no special CRDT
state; the result is an ordinary element (`toile-logic §2.C`).

> Node-level Bezier editing (add/remove/convert nodes, handle drag) is the `toile-ui` Pen tool (`ui §5bis`);
> this section is only the boolean combinators.

---

## 4. COLLABORATION PRIMITIVES

### A. Votes & reactions (CRDT — shared, persisted)

Placing an emoji reaction or a vote on an element or frame is shared state that must survive reload, so it is a
CRDT sub-map keyed by target + **canonical `UserId`** (one entry per user per target → a user's re-vote is LWW on
their own key, and tallies are convergent):

```
LoroDoc
└── "reactions" (LoroMap)
    └── "{target_id}" (LoroMap)
        └── "{user_id}" : Binary(postcard(Reaction))   // {emoji | vote_weight, ts}
```

> **Keyed by `UserId`, never `PeerID`.** A reaction/vote is a per-**user** decision that must survive reload, key
> rotation and multi-device use — the same identity argument as presence (`toile-logic §7`) and Trame's range
> protection (`trame-features §6`, "UserId and not pub_key… stable per-user"), applied here to *persisted* CRDT
> state. `PeerID` is per-session/per-device (`toile-logic §2.A/§7`), so keying by it would let one user on two
> devices **vote twice** (double-counted tally), would strand a vote when the user reopens the document in a new
> session (a new `PeerID` cannot remove or change the old key), and would break on rotation. The Logic thread
> writes the local user's own key from `my_user_id` (the `Uuid` received via `LogicEvent::SessionReady`, threaded
> into `ToileState` exactly as in `trame-logic §7`); remote entries are already `UserId`-resolved in `ambre-sync`.

A tally (count per emoji, vote totals) is **derived** at read time from the sub-map — never stored — so it is
always consistent with the set of per-user entries. Removing a reaction deletes the user's own key.

**Reactions on a deleted target.** If the `target_id` element/frame is gone (deleted concurrently), its
reaction sub-map is **skipped at tally derivation** — never rendered, never destroyed (data-never-destroyed,
like a dangling connector `toile-logic §5.A`). The orphan sub-map is reclaimed by ordinary document
compaction; no active GC is required.

### B. Highlighter / laser pointer (ephemeral)

The laser/highlighter brainstorming tool is **not** persisted: it is streamed as presence and rendered by
`toile-ui §8`, decaying on a timer. It never becomes an element and never enters history. (Placed here for
completeness of the collaboration surface; the render and transport are already specified in `toile-ui` /
`toile-logic §7`.)

### C. Why votes persist but laser does not

Votes/reactions are **decisions** — they must survive reload and reconcile across peers → CRDT. Laser/cursors are
**gestures** — momentary, per-viewer, meaningless after the moment → ephemeral. This is the same durable-intent
vs momentary-gesture split as §1 (sequence vs playback).

---

## 5. NEW COMMANDS & EVENTS

These variants extend the base `ToileCommand`/`ToileEvent` of `toile-logic §8`; the frame-content commands
(`EditNotes`, `SetAutoLayout`) are **owned here**, not in logic §8 (they were removed there to avoid a duplicate
`enum` variant).

```rust
/// Both are plain aliases of ElementId, not new types: a frame is an ordinary
/// element (toile-logic §2.F), and a reaction target is any element or frame
/// (never a connector). Declared once, here, and reused across the features spec.
pub type FrameId  = ElementId;
pub type TargetId = ElementId;

// added to ToileCommand
SetSlideOrder { order: Vec<FrameId> },
NextSlide, PrevSlide, GotoSlide(usize),   // local playback, no CRDT
EnterPresent { focus: bool }, ExitPresent,
EditNotes { frame: FrameId, delta: TextDelta },
SetAutoLayout { frame: FrameId, layout: Option<AutoLayout> },
BooleanOp { op: BoolOp, ids: Vec<ElementId>, keep_sources: bool },
React { target: TargetId, reaction: Option<Reaction> },  // None = remove

// added to ToileEvent
SlideChanged(usize),
PresentModeChanged(bool),
ReactionsUpdated(TargetId),               // UI re-reads the derived tally
```

---

## 6. QUALITY & TESTABILITY
> See the transverse test architecture in `ambre-spec-architecture.md §4`.

- **Test tiers:** **host** for the pure logic — `smart_animate` matching/tween, `solve_layout`, boolean-op
  geometry, reaction tally derivation, and the sequence/playback command handlers. **wasm** only for the
  presentation camera pass and inline notes editing on the GPU surface. No **db** tier.
- **Unit tests:** `smart_animate` covers all three cases (shared-id tween, fade-in, fade-out) and endpoint
  `t=0/1`; `solve_layout` covers Horizontal/Vertical/Wrap × Hug/Fill/Fixed and padding/gap/align; boolean ops on
  overlapping/disjoint/nested paths; tally derivation with add/change/remove; playback bounds (`Next` past last,
  `Prev` before first). Target: every predicate branch of the solver and animate matcher.
- **CRDT convergence (mandatory):** the collaborative state here is the **presentation sequence**, **speaker
  notes**, **auto-layout spec**, and **reactions** — a 2-peer convergence property test via `MockSyncHandle` +
  `apply_remote` must show identical final `LoroDoc` for: concurrent slide reorders (`LoroMovableList`),
  concurrent notes edits (`LoroText` char-CRDT), concurrent auto-layout spec edits (LWW register), and
  concurrent reactions (per-**user** keys — one vote per `UserId`, no double-count across devices — convergent
  tally). Derived playback state is excluded (local). Assert a two-device same-user re-vote is one entry (§4.A).
- **Mocks & Environment:** `CommandSink`/`MockSyncHandle`; fake `FrameSnapshot`s to drive `smart_animate`;
  headless wasm for the camera pass. No live network.
- **Benchmarks:** `criterion` on `smart_animate` for a 500-element frame pair (must fit the transition frame
  budget) and `solve_layout` for a 200-child frame.
- **Definition of Done (tested):** unit + convergence (sequence/notes/autolayout/reactions) + one wasm
  presentation test green in CI.

---

## 7. PERFORMANCE & RESOURCES

| Operation | Budget | Rationale |
|---|---|---|
| `smart_animate` per transition frame (500 elements) | < 4 ms | Leaves room for paint within 16 ms during the transition. |
| `solve_layout` (200 children) | < 1 ms | Runs at derivation on every child edit. |
| Reaction tally derivation | O(entries), < 0.1 ms typical | Recomputed on read, never stored. |
| Boolean op (typical two paths) | < 5 ms, off the render frame | User-initiated, one-shot; may show a brief spinner for huge paths. |

**WASM impact:** feature logic is small (no new heavy crates — reuses `kurbo` booleans, `loro`, `postcard`); the
presentation camera reuses the `toile-ui` pipeline.

---

## 8. REJECTED ARCHITECTURAL DECISIONS

| Rejected | Reason |
|---|---|
| **Linear presentation engine (PPT-style)** | Rejected in favor of Spatial Frames (Toile Todo §7); slides are frames of the same board, enabling Smart Animate by shared id. |
| **Persisting solved auto-layout positions in the CRDT** | Redundant derived state that could desync from the spec; the spec is CRDT, positions are derived (§2, mirrors connector routing). |
| **Persisting laser/highlighter as elements** | Momentary gestures must not enter history; ephemeral presence only (§4.B/C). |
| **Storing reaction tallies** | Would drift from the per-user entry set; tallies are derived on read (§4.A). |
| **Reactions/votes keyed by `PeerID`** | `PeerID` is per-session/per-device, so one user on two devices would double-vote and a re-opened session could not remove its own reaction. Keyed by canonical `UserId` (§4.A), the same decision as presence (`toile-logic §7`) and Trame protection (`trame-features §6`). |
| **Broadcasting playback position as CRDT** | Per-viewer navigation is local; "follow presenter" is ephemeral presence, not history (§1.B). |
| **Matching Smart Animate by geometry/heuristics** | Fragile; stable-`ElementId` matching is exact and free (§1.C). |

---

## 9. INTEGRATION PLAN (ROADMAP)

> Tracked as Stories of Epic **E4** (Toile MVP) in `specs/ambre-roadmap.md`.

**Phase 0 - Core standalone**
- [ ] Presentation sequence CRDT + playback state machine
- [ ] Speaker notes
- [ ] `smart_animate` pure solver (stable-`ElementId` matching, §1.C) with host tests
- [ ] `solve_layout` pure auto-layout solver with host tests
- [ ] Reactions / votes CRDT keyed by `UserId` (§4.A) + derived read-time tally
- [ ] Boolean ops (union / subtract / intersect)
- [ ] Convergence belt green (concurrent reactions -> one vote per `UserId`, convergent tally, §6)

**Phase 1 - Integration**
- [ ] Wire playback to the `toile-ui` camera pass
- [ ] Presenter overlay panel
- [ ] Reactions render
- [ ] "Follow presenter" ephemeral broadcast

**Phase 2 - Optimization**

auto-layout incremental reflow (only affected children), Smart Animate snapshot caching, large-deck sequence-panel virtualization.
