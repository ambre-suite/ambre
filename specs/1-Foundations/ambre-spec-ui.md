# AMBRE -- FEATURE: THE UI THREAD

**Type:** Horizontal (framework)
**Implementation:** partial -- the boundary, the crate graph and the state split are built. The six
behaviours every editor should share are arbitrated and unbuilt.

> The page's own thread. It draws the content on a graphics surface, builds the rest of the
> interface out of ordinary page elements, and holds nothing the document owns.

What this document fixes is what the two halves agree on and what the thread may never do. The
content half is `ambre-spec-frame.md`, the interface half is `ambre-spec-chrome.md`, and neither is
restated here.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`. The prefix is `UI-THREAD`, shared with
`ambre-spec-clipboard.md`, `ambre-spec-commands.md` and `ambre-spec-design-tokens.md`.

This thread owns no gesture: it is a place, and every row below binds it at every instant. The
gestures it hosts belong to `ambre-spec-frame.md`, to `ambre-spec-chrome.md` and to each
application.

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `UI-THREAD-049` | `INV` | The UI thread MUST NOT open a cryptography path. | `ambre-spec-zero-knowledge.md` (`ZK-002`); `ambre-spec-architecture.md`, `ARCH-004` | cargo-deny -- `deny.toml` grants `ambre-crypto` to `ambre-sync` and, for one host-only fixture hash, to `ambre-logic`. Every interface crate fails the job on the line that would add it, and the cipher, the passphrase derivation and the key agreement each carry a ban of their own | none | verified |
| `UI-THREAD-021` | `INV` | A document snapshot the launcher carries to the local store MUST stay opaque bytes: the launcher MUST NOT decode one. | this spec section 2; `SYNC-011` | cargo-deny -- `deny.toml` allows `loro` in the four application crates, `ambre-logic` and `ambre-crdt` alone, and `ambre-crdt` in the four application crates alone, so `ambre-app`, which is the crate that carries a snapshot to the browser database, has no way to read it | none | verified |
| `UI-THREAD-058` | `INV` | UI-thread code outside the surface, its host and the shell cell MUST NOT name a canvas, a drawing context or a `<canvas`. | this spec section 2 | ci-gate -- `scripts/crate_boundaries.py` refuses each of them in that code | none | verified |
| `UI-THREAD-001` | `INV` | The drawing surface MUST own the content, the caret, the selections and content hit-testing. | this spec section 2 | type-level -- `ambre_draw::HitTest` is the one way from a canvas point to content, and each application's surface implements it over its own layout; `ambre_prose::input_tests::a_point_resolves_to_the_offset_drawn_there_under_zoom_and_scroll`; `ambre_trame::ui::grid::tests::kinetic::a_canvas_point_reaches_its_cell_through_the_shared_hit_test`; `ambre_toile::ui::view::view_tests::a_canvas_point_reaches_its_element_through_the_shared_hit_test` -- a point reaches what is drawn there through the trait, under zoom and scroll; ci-gate -- `scripts/crate_boundaries.py` refuses a canvas-relative pointer read outside the host and the platform listeners, a content hit-test in an interface component, and an interface file that names the canvas beside a page-relative pointer read. The content, the caret and the selections are drawn, so `UI-THREAD-058` holds them | none | verified |
| `UI-THREAD-048` | `INV` | The page elements MUST own everything the surface does not: their own focus, and their own accessibility. | this spec section 2; `ambre-spec-accessibility.md` | `ambre_canvas::text_input::tests::only_the_spreadsheet_names_itself_a_cell` -- the focus carrier is a page element named by each kind of document; `ambre_toile::ui::a11y::tests::a_board_announces_its_selection`; `ambre_prose::view::a11y::tests::a_page_announces_the_block_at_the_caret`; `ambre_trame::ui::view::tests::the_active_cell_is_announced_by_reference_and_value` -- each application composes what the page element says of its surface. The other drawn states are `CHROME-029` | none | verified |
| `UI-THREAD-002` | `INV` | The UI thread MUST NOT block on a queue, inbound or outbound. | this spec section 2; `ambre-spec-architecture.md section 2` | type-level -- every view sends through its outbox, whose two sends only try, and neither has a blocking form (`BUS-033`); `ambre_prose::ui::tests::a_full_channel_drops_the_command_rather_than_holding_the_frame`; `::a_full_channel_holds_an_edit_rather_than_the_frame`; ci-gate -- `scripts/crate_boundaries.py` refuses a parking handle, a `.wait()`, a parked thread, an atomic wait or a `block_on` in UI-thread code, so a view added later cannot block either | none | verified |
| `UI-THREAD-003` | `INV` | A piece of interface state MUST NOT be held twice: a value the dispatcher pushes into a signal MUST NOT also live in a view's own state. | this spec section 2 | ci-gate -- `scripts/crate_boundaries.py` reads the signals each relay arm writes off an application event, then each view arm on the same event, and refuses a view arm that stores a field while the relay writes a signal off that event. A repaint flag is no state. A pair on its accepted list holds two different parts of the event, with the reason. The view that keeps the active sheet, the filter flag, a find result or the present mode publishes it in the call that stores it, so the dispatcher pushes none of them | none | verified |
| `UI-THREAD-060` | `INV` | A shell signal, and the bus event field that feeds it, MUST carry an identifier, a quantity with a unit or a closed set of values as a type of its own, never as a bare primitive. | this spec section 2; `ai/ambre-rust-style.md` | ci-gate -- `scripts/crate_boundaries.py` reads the payload of each signal the shells hold, peels the wrappers, and refuses a primitive, a tuple or an array. A signal on its accepted list carries free text, with the reason; type-level -- each relay copies the event field into its signal, so the compiler gives the field the signal's type, and a value the relay builds from a reply goes through that type's constructor; `ambre_chrome::status::tests::a_zoom_factor_is_clamped_where_it_is_built`; `ambre_toile::ui::presenter::tests::a_slide_position_stays_inside_its_deck`; `ambre_trame::tests::chrome_ui::a_find_position_counts_from_one_and_never_past_the_matches` -- each constructor holds its invariant | none | verified |
| `UI-THREAD-059` | `INV` | A view MUST NOT hold a field of a type that a shell signal carries, short of a field a reviewer accepted with its reason. | this spec section 2 | ci-gate -- `scripts/crate_boundaries.py` reads the payload types of the signals each shell holds, peels the wrappers, keeps the marker of a family such as `Validated<WebAddress>`, and refuses a view struct field of one of them. A field on its accepted list carries the reason it is not a second copy | none | verified |
| `UI-THREAD-004` | `INV` | The UI thread MUST NOT open a network or persistence path. The exceptions are same-origin font loading, the pre-session sign-in configuration read, the sign-in's own state, the local store and the peer connection of a call. | `ambre-spec-zero-knowledge.md` (`ZK-001`); `ambre-spec-call-media.md section 3` (`CALL-008`) | ci-gate -- the rustc driver of `crates/ambre-primitives`, run by `just primitives` in the CI `lint` job, reads each path once it resolves, so an alias, a re-export and a macro name what they reach. It refuses a network call outside the Sync worker and the two named reads, and the graph `cargo metadata` resolves refuses a transport crate declared outside it (`ARCH-005`); ci-gate -- `scripts/crate_boundaries.py` refuses a browser storage call outside the local store and the two sign-in files | none | verified |
| `UI-THREAD-005` | `INV` | Content rendering MUST stay inside the subset both graphics back ends share: no compute pass, no storage buffer. | `ambre-spec-frame.md` | ci-gate -- `scripts/crate_boundaries.py` refuses a compute pass, a compute pipeline, a storage buffer or a storage texture in any Rust or WGSL source | none | verified |
| `UI-THREAD-020` | `INV` | The UI thread MUST hold no document data: only view state and the display payload the Logic thread pushed. | this spec section 2 | ci-gate -- `scripts/crate_boundaries.py` refuses a CRDT type, an export mode or an engine state in UI-thread code: the page's crates and each application's view, render and chrome modules | none | verified |
| `UI-THREAD-050` | `INV` | The UI thread MUST hold no path by which a mutation reaches the document other than a command. | this spec section 2; `ambre-spec-message-bus.md`, `BUS-002` | ci-gate -- the scan of `UI-THREAD-020`: no CRDT type or engine state is named in UI-thread code, so no writable handle reaches a view and a command is the one path left | none | verified |
| `UI-THREAD-051` | `INV` | An application's view code MUST NOT decode a document snapshot. | this spec section 2; `SYNC-011` | ci-gate -- `scripts/crate_boundaries.py` refuses `loro::`, a `Loro` type or an export mode in each application's view, render and chrome modules, the part of the crate the `loro` grant reaches but must not serve | none | verified |
| `UI-THREAD-012` | `INV` | Every application that zooms MUST zoom through one engine -- one factor, one transform -- behind the shared status-bar control. | this spec section 2 | not built | unbuilt -- lands with the `One zoom model` Story (`ambre-roadmap.md` E8) | unimplemented |
| `UI-THREAD-013` | `INV` | Every application that zooms MUST accept a two-finger pinch as a zoom. | this spec section 2 | not built | unbuilt -- lands with the `One zoom model` Story (`ambre-roadmap.md` E8) | unimplemented |
| `UI-THREAD-015` | `INV` | A row or column resize MUST go through one command, whichever table model asks for it. | this spec section 2 | not built | unbuilt -- lands with the `One resize command` Story (`ambre-roadmap.md` E8) | unimplemented |
| `UI-THREAD-016` | `INV` | Undo depth MUST be one bounded cap, shared by every application. | this spec section 2 | not built | unbuilt -- lands with the `One undo bound` Story (`ambre-roadmap.md` E8). The two applications that ship an undo stack today bound it nowhere | unimplemented |
| `UI-THREAD-017` | `INV` | Following a link MUST go through one host affordance. | this spec section 2 | not built | unbuilt -- lands with the `Links in Trame` Story (`ambre-roadmap.md` E8) | unimplemented |
| `UI-THREAD-052` | `INV` | An application MUST NOT open an address by a path of its own. | this spec section 2; `ambre-spec-zero-knowledge.md` | not built | unbuilt -- lands with the `Links in Trame` Story (`ambre-roadmap.md` E8). The ban outlives the affordance: an address opened outside it leaks the document's content to whoever answers, and no host affordance existing is why nothing refuses one today | unimplemented |

* **Budget.** the UI-thread rows of `ambre-spec-budgets.md section 1`: the keystroke to a draft,
  the viewport send, the render of a viewport event, the autocompletion. The memory total of
  `ambre-spec-budgets.md section 6` is the fifth. Every one reads **none exists**.
* **Boundary.** `UI-THREAD-004` and `UI-THREAD-049` are the boundary on this side, and the first
  names its four exceptions. The operating system clipboard is the other outbound surface and
  `ambre-spec-clipboard.md` fixes what it may carry; everything else leaves through the Sync thread,
  sealed (`ambre-spec-zero-knowledge.md`).
* **Verification.** 14 `verified`, 6 `unimplemented`. Two held rules rest on a capability ban, which
  refuses the dependency instead of catching its use. Where no crate can be refused, a scan of the
  UI-thread source refuses the call. That covers a fetch, a storage call, a wait, a drawing context,
  a content hit-test and the engine in a view.
---

## 2. MODEL

**CRDT containers.** none, and that is `UI-THREAD-020`. The thread holds view state and the display
payload the Logic thread pushed. The document is one mental model, and it lives on the other
thread.

**The boundary.** Two zones, two technologies, and two queues between them.

| Zone | Drawn by | Specified in |
|---|---|---|
| high-frequency content: a grid, body text, a board | the graphics surface | `ambre-spec-frame.md` |
| everything else: menus, toolbars, dialogs, panels, the palette, toasts | page elements | `ambre-spec-chrome.md` |

The interface is composed **around** the surface, which costs one layout pass; the browser then
recomposes already rasterized layers at no per-frame cost. The one sensitive case is a floating
element anchored to content that pans and zooms. That element is synchronized to the content's
transform on every frame.

A pointer crosses the boundary in one direction only. The host reads it relative to the canvas and
asks the surface what is drawn there, through the one trait each application's surface implements.
A page element never resolves a point to content: it receives what the surface resolved.

```rust
pub trait HitTest {
    type Target;
    fn hit_test(&self, at: Point<Px<View>>) -> Option<Self::Target>;
}
```

| Direction | Carries |
|---|---|
| surface to elements | a signal: the surface detects a gesture, computes a position, and an element opens there |
| elements to surface | a command: an identity produces a message, the document changes, an event returns, the surface repaints |

The only page element **above** the surface is the transparent input that carries the keyboard, the
composition and the clipboard events (`ambre-spec-text-input.md`).

**The crate graph.** The boundary is enforced by the graph, in one binary. The rule is one line:
what is specific to an application lives in that application's crate, what is shared lives upstream
of every application.

* shared, upstream of the applications:
  * the command identities, depended on by every application and by the interface toolkit, and
    depending on nothing itself (`ambre-spec-commands.md`);
  * the drawing surface abstraction, with a screen back end and a document back end
    (`ambre-spec-drawing-surface.md`);
  * the content runtime: the render loop, the glyph cache, the generated tokens
    (`ambre-spec-frame.md`);
  * the interface toolkit: the shell, the palette, the dialogs, the menus, the shared formatting
    controls (`ambre-spec-chrome.md`).
* one crate per application, each holding its own Logic state and handlers, its own view, its own
  interface components and its own command table.
* the launcher, downstream of every application: it mounts the shell, creates the queues and
  spawns the Logic and Sync threads. It then boots the surface and injects the active view
  (`ambre-spec-launcher.md`).

Two of the shared crates are deliberately pure: one graphics, one page framework. The renderer
therefore never depends on the framework the interface is written in.

**Where a state lives.** A state can exist in several layers. One layer owns it and every other is
a projection.

| Category | Example | Owner | Leaves the machine |
|---|---|---|---|
| a document mutation | a cell's text, a shape's position, a paragraph | the document, on the Logic thread | yes, as a delta |
| shared metadata | a filter, a conditional format, a named range | the document, on the Logic thread | yes, as a delta |
| ephemeral presence | a remote cursor, a peer's selection, a stroke in flight | the sender's Logic thread | yes, and never persisted |
| local view state | a freeze, a personal filter, a selection, a camera, the active tool | this thread, some of it mirrored to the local store | no |
| derived state | a formula cache, a dirty set, a spatial index, a culled viewport | the Logic thread, rebuilt from the document | no |
| local history | the undo stack | the Logic thread, for this session | no |

The consequence is `UI-THREAD-020`, and the local store is its one crossing. The launcher carries
snapshot bytes to the browser database and back, and cannot read them (`UI-THREAD-021`).

**The six behaviours every editor shares.** Each was arbitrated in the 2026-08 review, where three
applications had given three answers to one question. Each is a Story of `ambre-roadmap.md` E8.

| Behaviour | The rule | What stays each application's own |
|---|---|---|
| zoom | one engine -- the factor and the transform -- behind the already-shared status-bar control (`UI-THREAD-012`, `UI-THREAD-013`) | nothing. Every application zooms the same way, including the one that has no zoom today |
| axis resize | one row and column resize command (`UI-THREAD-015`) | the table models. A spreadsheet grid and a rich-text table differ in scale and in meaning, so there is no shared table object |
| undo depth | one bounded cap, as a shared parameter (`UI-THREAD-016`) | what merges into one step. One application commits on leaving a cell, another merges keystrokes |
| following a link | one host affordance, shared by every application's link model (`UI-THREAD-017`) | nothing. An application-private path to open an address is what this forbids |
| text editing | one substrate: the input pipe, the composition machine, the caret and the selection model (`ambre-spec-text-editor.md`, `CANVAS-037`) | the layout driver. An inline editor shapes one run at paint time; a paginated document shapes on the Logic thread and ships shaped runs |
| selection endpoints | the moving end is the focus, the fixed one the anchor, everywhere (`ambre-spec-text-editor.md`) | nothing. It is a rename |

**Code.** This document describes a boundary, so it names no module of its own. The crates the
graph above lists are `crates/ambre-commands`, `crates/ambre-draw`, `crates/ambre-canvas`,
`crates/ambre-chrome`, the four application crates and `crates/ambre-app`.

---

## 3. SURFACE

The user touches nothing here. What they see is whether the interface stays responsive.

| Trigger | Behaviour |
|---|---|
| any gesture at all | the thread never waits: a full queue drops the message with a log rather than freezing the page |
| a document mutation | leaves as a command and comes back as an event; nothing is written locally first |
| a right-click on content | the surface computes the position and a page element opens there |
| a toolbar control on content that pans | the element follows the content's transform on every frame |
| the network being down | nothing here changes: this thread never reached it |

---

## 4. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| Content on a graphics surface, the rest as page elements | one interface drawn entirely on the surface, with a widget framework of our own | matching a mature widget toolkit means about two years of work and still no accessibility to match what the browser gives away. The argument against page elements -- their composition cost -- is false for a static interface around a surface, which the browser recomposes at no per-frame cost. |
| No windowing library | a windowing library between the page and the surface | the target is the browser alone, so a windowing layer is indirection with no portability to buy, and it fights the page for event ownership. Events are read from the browser directly. |
| Both graphics back ends | the newer back end alone, for the features the content does not use | staying inside the shared subset widens the reachable machines, institutional fleets included, and the subset costs nothing: the content pipeline needs no compute pass. |
| Same-tab fidelity is not decided by comparing text | comparing the plain-text mirror to decide whether a paste is our own | two tabs holding the same text are the case a fidelity ladder exists for, and a text match reports them as one copy. The reasoning and the rule are `ambre-spec-clipboard.md`. |
| The browser is the only target | a native build, with an interface drawn entirely on the surface | a desktop build, if it comes, is an installable page or a thin native shell. Committing to the browser is what makes an interface out of page elements possible at all. |
| Three undo documents stand, and one shared row | one undo document for the suite, with the three engines as sections of it | the pattern is one and the proof is not: each application must show it over its own handlers, so collapsing three rows into one would report three implementation states as one. Only the depth cap is a single shared number, and only that is a row here -- the two application rows restating it named no mechanism of their own and were withdrawn. |
| An arbitrated behaviour with no shared component stays here | a document per arbitrated behaviour, written before the behaviour has any code | a rule belongs to the document that owns the component it constrains, and four of the six behaviours constrain a component that does not exist yet. Filing each in a document of its own would create four design-only stubs whose whole content is one row. The two whose component exists left: the editing substrate and the endpoint names are `ambre-spec-text-editor.md`. |
| The exceptions of `UI-THREAD-004` are named rather than removed | tightening the rule to zero exceptions and moving the four paths to another thread | the font fetch is same-origin and carries no document data; the sign-in configuration read happens before any session exists; the sign-in's own state is a one-tab nonce and the development identity, neither a key; the local store holds bytes this thread cannot read. Each is narrower than the rule it excepts, and moving any of them off this thread buys nothing. |

---

## 5. NOT BUILT, AND NOT YET RULED

* **Four of the six shared behaviours have no code at all.** One zoom engine, one resize command,
  one undo bound and one way to open an address are arbitrated and unbuilt. Each application still
  answers its own way (`UI-THREAD-012` to `UI-THREAD-017`).
* **Nothing refuses the next exception to `UI-THREAD-004`.** The rule is written against what the
  launcher does, and no lint bans a fourth path. The first three were found by reading the crate,
  which is how a fifth will be found.
* **The hit-test scan reads one file at a time.** `UI-THREAD-001` refuses a page point and the
  canvas named in one interface file. A page point carried to a second file, which then subtracts
  the canvas position, passes. So does a canvas found by a selector other than its id.
* **The blocking receive still compiles on this thread.** Each application types the rule for its
  own view and a view added tomorrow inherits nothing.
* **No rule covers what the local store may hold.** The snapshot is opaque to this thread.
  Nothing states how long it lives, what clears it, or what a second document does to it
  (`ambre-spec-launcher.md`).
* **The subset rule has no check.** Nothing in the build refuses a compute pass, and the failure
  mode is a blank surface on the machines least likely to be tested on.
* **Nothing rules on a touch gesture that is not a pinch.** A pinch is a row because it is part of
  the zoom arbitration; a long press, a two-finger pan and a swipe are named nowhere.

---

## 6. WHAT THIS DOCUMENT DOES NOT OWN

| Question | Owner |
|---|---|
| How the content is drawn, and what a frame costs | `ambre-spec-frame.md` |
| The interface components, and how they are placed | `ambre-spec-chrome.md` |
| How a command is named, resolved and described | `ambre-spec-commands.md` |
| Where a colour, a spacing or a type size comes from | `ambre-spec-design-tokens.md` |
| What a copy carries, and how a paste decides fidelity | `ambre-spec-clipboard.md` |
| The keyboard, the composition and the transparent input | `ambre-spec-text-input.md` |
| The shared inline editor, and the names of the selection ends | `ambre-spec-text-editor.md` |
| The queues between the three threads, and what they carry | `ambre-spec-architecture.md section 2` |
| What the sealed path is, and what the relay may read | `ambre-spec-zero-knowledge.md` |
| The home screen, the routes and the sign-in | `ambre-spec-launcher.md` |
| What each application's view holds and paints | each application's engine document |
