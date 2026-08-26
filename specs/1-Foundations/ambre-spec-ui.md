# AMBRE -- FEATURE: THE UI THREAD

**Type:** Horizontal (framework)
**Implementation:** partial -- the boundary, the crate graph and the state split are built; the six
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

| ID | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|
| `UI-THREAD-001` | The drawing surface MUST own the content, the caret, the selections and content hit-testing; the page elements MUST own everything else, their own focus and their own accessibility. | this spec section 2 | none. The split holds by construction: the only page element over the surface is the transparent input. Nothing refuses an interface component that draws content | unverified |
| `UI-THREAD-002` | The UI thread MUST NOT block on a channel, inbound or outbound. | this spec section 2; `ambre-spec-architecture.md section 2` | each application holds and types the rule for itself (`TRAME-UI-003`, `TOILE-UI-100`). Nothing holds it for a view added later: the blocking receive compiles on this thread and freezes the page | unverified |
| `UI-THREAD-003` | A piece of interface state MUST NOT be held twice: a value the dispatcher pushes into a signal MUST NOT also live in a view's own state. | this spec section 2 | reviewer. A diff shows a duplicate and no test does: the two copies agree until one frame updates only one of them | human-review |
| `UI-THREAD-004` | The UI thread MUST NOT open a network, cryptography or persistence path. The exceptions are same-origin font loading, the pre-session sign-in configuration read, and the local store. | `ambre-spec-zero-knowledge.md` | none. Neither the drawing crate nor the interface toolkit declares a transport or cryptography dependency, and the launcher holds the two reads plus the browser database (`ambre-spec-launcher.md`). No lint bans the next one | unverified |
| `UI-THREAD-005` | Content rendering MUST stay inside the subset both graphics back ends share: no compute pass, no storage buffer. | `ambre-spec-frame.md` | none as a check. On the older back end a compute pass fails at device creation, so a regression is a blank surface at run time rather than a red job | unverified |
| `UI-THREAD-012` | Every application that zooms MUST zoom through one engine -- one factor, one transform -- behind the shared status-bar control. | this spec section 2 | not built -- lands with `One zoom model` (`ambre-roadmap.md` E8) | unimplemented |
| `UI-THREAD-013` | A two-finger pinch MUST zoom, in every application that zooms. | this spec section 2 | not built -- lands with `One zoom model` (`ambre-roadmap.md` E8) | unimplemented |
| `UI-THREAD-015` | A row or column resize MUST go through one command, whichever table model asks for it. | this spec section 2 | not built -- lands with `One resize command` (`ambre-roadmap.md` E8) | unimplemented |
| `UI-THREAD-016` | Undo depth MUST be one bounded cap, shared by every application. | this spec section 2 | not built -- lands with `One undo bound` (`ambre-roadmap.md` E8). The two applications that ship an undo stack today bound it nowhere | unimplemented |
| `UI-THREAD-017` | Following a link MUST go through one host affordance; an application MUST NOT open an address by a path of its own. | this spec section 2 | not built -- lands with `Links in Trame` (`ambre-roadmap.md` E8) | unimplemented |
| `UI-THREAD-020` | The UI thread MUST hold no document data: only view state and the display payload the Logic thread pushed. Every mutation MUST leave as a command. | this spec section 2 | each application types the rule for itself (`TRAME-UI-001`, `TOILE-UI-099`; the Prose row is unbuilt). Nothing holds it for a view added later: inside an application crate a view may still reach the document | unverified |
| `UI-THREAD-021` | A document snapshot MUST cross this thread as opaque bytes: the thread MUST NOT decode one. | this spec section 2; `SYNC-011` | `cargo deny` -- `deny.toml` allows the engine crate in the four application crates alone, so the launcher that carries a snapshot to the local store has no way to read it | verified |

* **Budget.** the UI-thread rows of `ambre-spec-budgets.md section 1` -- the keystroke to a
  draft, the viewport send, the render of a viewport event, the autocompletion -- plus the memory
  total of `ambre-spec-budgets.md section 6`. Every one of those rows reads that no measurement exists.
* **Boundary.** `UI-THREAD-004` is the boundary on this side, and it names its three exceptions.
  The operating system clipboard is the other outbound surface and `ambre-spec-clipboard.md` fixes
  what it may carry. Everything else leaves through the Sync thread, sealed
  (`ambre-spec-zero-knowledge.md`).
* **Verification.** 1 `verified`, 5 `unverified`, 1 `human-review`, 5 `unimplemented`. The one held
  rule is held by a capability ban rather than a test, which is the strongest form available here:
  it refuses the dependency instead of catching its use. The five loose rules are each held for one
  application by that application's own row, and for a view added tomorrow by nothing.

---

## 2. MODEL

**CRDT containers.** none, and that is `UI-THREAD-020`. The thread holds view state and the display
payload the Logic thread pushed; the document is one mental model and it lives on the other thread.

**The boundary.** Two zones, two technologies, and two channels between them.

| Zone | Drawn by | Specified in |
|---|---|---|
| high-frequency content: a grid, body text, a board | the graphics surface | `ambre-spec-frame.md` |
| everything else: menus, toolbars, dialogs, panels, the palette, toasts | page elements | `ambre-spec-chrome.md` |

The interface is composed **around** the surface, which costs one layout pass; the browser then
recomposes already rasterized layers at no per-frame cost. The one sensitive case is a floating
element anchored to content that pans and zooms, and that element is synchronized to the content's
transform on every frame.

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
* the launcher, downstream of every application: it mounts the shell, creates the channels, spawns
  the Logic and Sync threads, boots the surface and injects the active view
  (`ambre-spec-launcher.md`).

Two of the shared crates are deliberately pure -- one graphics, one page framework -- so the
renderer never depends on the framework the interface is written in.

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

The consequence is `UI-THREAD-020`, and the local store is its one crossing: the launcher carries
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
| any gesture at all | the thread never waits: a full channel drops the message with a log rather than freezing the page |
| a document mutation | leaves as a command and comes back as an event; nothing is written locally first |
| a right-click on content | the surface computes the position and a page element opens there |
| a toolbar control on content that pans | the element follows the content's transform on every frame |
| the network being down | nothing here changes: this thread never reached it |

---

## 4. DECISIONS

| Decision | Rationale | Rejected alternative |
|---|---|---|
| Content on a graphics surface, the rest as page elements | matching a mature widget toolkit means about two years of work and still no accessibility to match what the browser gives away, while the argument against page elements -- their composition cost -- is false for a static interface around a surface, which the browser recomposes at no per-frame cost | one interface drawn entirely on the surface, with a widget framework of our own |
| No windowing library | the target is the browser alone, so a windowing layer is indirection with no portability to buy, and it fights the page for event ownership. Events are read from the browser directly | a windowing library between the page and the surface |
| Both graphics back ends, not the newer one alone | staying inside the shared subset widens the reachable machines, institutional fleets included, and the subset costs nothing: the content pipeline needs no compute pass | the newer back end alone, for the features the content does not use |
| Same-tab fidelity is not decided by comparing text | two tabs holding the same text are the case a fidelity ladder exists for, and a text match reports them as one copy. The reasoning and the rule are `ambre-spec-clipboard.md` | comparing the plain-text mirror to decide whether a paste is our own |
| The browser is the only target | a desktop build, if it comes, is an installable page or a thin native shell. Committing to the browser is what makes an interface out of page elements possible at all | a native build, with an interface drawn entirely on the surface |
| Three undo documents stand, and one shared row | the pattern is one and the proof is not: each application must show it over its own handlers, so collapsing three rows into one would report three implementation states as one. Only the depth cap is a single shared number, and only that is a row here -- the two application rows restating it named no mechanism of their own and were withdrawn | one undo document for the suite, with the three engines as sections of it |
| An arbitrated behaviour with no shared component stays here | a rule belongs to the document that owns the component it constrains, and four of the six behaviours constrain a component that does not exist yet. Filing each in a document of its own would create four design-only stubs whose whole content is one row. The two whose component exists left: the editing substrate and the endpoint names are `ambre-spec-text-editor.md` | a document per arbitrated behaviour, written before the behaviour has any code |
| The exceptions of `UI-THREAD-004` are named rather than removed | the font fetch is same-origin and carries no document data; the sign-in configuration read happens before any session exists; the local store holds bytes this thread cannot read. Each is narrower than the rule it excepts, and moving any of them off this thread buys nothing | tightening the rule to zero exceptions and moving the three paths to another thread |

---

## 5. NOT BUILT, AND NOT YET RULED

* **Four of the six shared behaviours have no code at all.** One zoom engine, one resize command,
  one undo bound and one way to open an address are arbitrated and unbuilt, so each application
  still answers its own way (`UI-THREAD-012` to `UI-THREAD-017`).
* **Nothing refuses the next exception to `UI-THREAD-004`.** The rule is written against what the
  launcher does, and no lint bans a fourth path. The first three were found by reading the crate,
  which is how a fifth will be found.
* **Nothing refuses an interface component that draws content.** `UI-THREAD-001` holds because
  nobody has written one, and the graph does not stop it: the toolkit can reach a canvas element as
  easily as any other.
* **The blocking receive still compiles on this thread.** Each application types the rule for its
  own view and a view added tomorrow inherits nothing.
* **No rule covers what the local store may hold.** The snapshot is opaque to this thread, and
  nothing states how long it lives, what clears it, or what a second document does to it
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
| The channels between the three threads, and what they carry | `ambre-spec-architecture.md section 2` |
| What the sealed path is, and what the relay may read | `ambre-spec-zero-knowledge.md` |
| The home screen, the routes and the sign-in | `ambre-spec-launcher.md` |
| What each application's view holds and paints | each application's engine document |
