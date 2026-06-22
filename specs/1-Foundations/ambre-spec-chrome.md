# AMBRE — SPEC: Chrome (Leptos DOM) + canvas cohabitation (`ambre-chrome`)
**Type:** Horizontal (Foundation — UI Thread)
**Status:** Draft

> The application **chrome** (menus, toolbars, dialogs, panels, command palette) rendered in **DOM via Leptos** (`ambre-chrome` crate), and its cohabitation with the wgpu `<canvas>` that renders the **content** (`ambre-canvas` crate). This spec covers the chrome stack, its components, text/IME input, and adaptive design. **Cross-cutting** UI thread concerns — canvas/DOM boundary, crate map, command model, single source of tokens — are in the umbrella `ambre-spec-ui.md`. The GPU pipeline is in `ambre-spec-canvas.md`; inter-thread transport in `ambre-spec-architecture.md`.

## 1. OBJECTIVES & CONSTRAINTS

* **Scope:** DOM chrome (Leptos), its cohabitation with the canvas, text/IME input, theming, and adaptive design. **Out of scope:** the canvas/DOM boundary, crate map, command registry, and token generation (`ambre-spec-ui.md`); the GPU pipeline (`ambre-spec-canvas.md`); business logic (Logic thread); IPC transport (`ambre-spec-architecture.md`).

* **Web-only.** Ambre targets the browser. Native is not a target; a possible desktop goes through **PWA** (installable, offline) or Tauri. This constraint enables a DOM chrome (which does not run in pure native) and the removal of winit (§6).

* **Dependencies:**

  | Crate | Role | Rationale |
  |:---|:---|:---|
  | `leptos` | Chrome DOM framework | Fine-grained reactivity (signals), no VDOM diffing, compact WASM binary. Chrome reflects selection state in real time without massive re-renders. |
  | `nucleo-matcher` | Fuzzy matching | Filters the command palette over the registry. Pure Rust. |
  | `ambre-commands` | Shared command registry | `CommandId`/`Command`/`CommandSource`. Leaf crate, zero UI dependency. Model in `ambre-spec-ui.md §2.C`. |
  | `wgpu`, `parley`, `swash` | Content rendering | Detail in `ambre-spec-canvas.md`. |
  | `accesskit` | Canvas **content** a11y | Content a11y tree exposed to screen readers. DOM chrome is natively accessible (§2.F). |

* **Invariants:**
  - **Thread-level** invariants (boundary rule, single source of truth, non-blocking, WebGL2-compatible) are in `ambre-spec-ui.md §1`.
  - **Chrome does not own the content:** it never writes into the `<canvas>` nor hit-tests content; it invokes `CommandId` and receives state via signals (boundary `ambre-spec-ui.md §2.A`).
  - **`ambre-chrome` does not depend on apps**: it displays `Command` and their state (dependency inversion, symmetrical to the `AppView` enum on the rendering side).

---

## 2. TECHNICAL DESIGN & STATE

### A. Chrome Technical Stack

| Element | Choice | Rationale |
|:---|:---|:---|
| Modals | Native `<dialog>` + Leptos content | `showModal()` provides focus trap, inertness, Escape, backdrop for free. |
| Theming | Generated design tokens | Single source `tokens.ron` → `theme.css` (DOM) + `theme.rs` (canvas), consistency by construction (`ambre-spec-ui.md §2.D`). |
| Menu/popover positioning | Manual calculation (position + flip if overflow) | CSS Anchor Positioning too recent cross-browser; ~80 lines of Rust. |
| Fuzzy palette | `nucleo-matcher` | Pure Rust. |

### B. Chrome Components

- **App shell**: CSS grid layout — header (title, sync status, avatar), command bar, optional sidebar, **canvas in the center**, status bar. The `<canvas>` resizes via `ResizeObserver` → wgpu swap chain reconfiguration. (Provided by `ambre-chrome`, parameterized by the app.)
- **Command bar — contextual toolbar**: a single bar showing commands relevant to the selection (text → formatting; image → image tools; cell → formulas); secondary commands in an overflow menu (`⋯`); on mobile, bottom sheet. A ribbon can still be generated from the registry (one `category` = one tab) as a user option.
- **Menus & context menus**: `role="menu"`/`menuitem"`, keyboard navigation (arrows, Home/End, first letter), Escape/click-outside close, flip-positioned. The canvas context menu is triggered by the wgpu handler (right-click → signal → open at coordinates).
- **Dialogs**: native `<dialog>` + Leptos content. When a fullscreen modal is open, the canvas `requestAnimationFrame` is **suspended** (`cancelAnimationFrame`) — battery saving, and `backdrop-filter: blur()` becomes free (canvas frozen). rAF restarted on close.
- **Sidebar / panels**: pure DOM, native `overflow: auto`, resizable via splitter drag.
- **Status bar**: cursor position, zoom, word count, sync status — reactive DOM on signals.
- **Command palette**: centered overlay, input + list filtered by `nucleo-matcher` over the registry, keyboard navigation.
- **Toasts**: `position: fixed` container, CSS `transform`/`opacity` animations run by the compositor **outside the canvas rAF** — zero ms stolen from the frame budget.

### C. Canvas Cohabitation

> The canvas/DOM **boundary** and the two communication channels (signals / commands) are defined in `ambre-spec-ui.md §2.A`. This section covers the DOM-side wiring for the chrome.

**Winit-free bootstrap.** The canvas is obtained via `web-sys` (`HtmlCanvasElement`) and `wgpu::Surface` configured on it. Events are captured by DOM listeners (`pointerdown/move/up`, `keydown`, `wheel`, `ResizeObserver`); `window.devicePixelRatio` + `matchMedia("(resolution: …)")` for DPI. Clean boundary: DOM captures chrome, the canvas listener captures the content area — no event ownership conflict. (Code in `ambre-canvas::host`.)

**Event routing.** Events on chrome → Leptos/DOM. Events on canvas → listener → content hit-tests. Editing keyboard focus belongs to the `<textarea>` overlay; a click in the canvas calls `textarea.focus()` from the wgpu handler.

**The transparent textarea (input + IME).** A technically invisible `<textarea>` serves as the keyboard + IME + clipboard pipe; it **displays nothing** (the canvas renders caret and selections).

```css
textarea.ambre-input {
  color: transparent; caret-color: transparent;  /* caret drawn by wgpu */
  background: transparent; outline: none; resize: none;
}
textarea.ambre-input::selection { background: transparent; color: transparent; }
```

In Latin input the textarea is **empty**: `keydown`/`beforeinput` → apply to Loro model → Parley re-layout on the dirty portion → re-render → clear the textarea. During **IME composition** (CJK, Korean, Vietnamese: `compositionstart → compositionupdate* → compositionend`) the text is rendered in the canvas with a "composition" style (dotted underline) while the textarea stays invisible; the IME popup positions relative to the textarea, placed under the caret via `transform: translate(x, y)` (never `top/left`), `will-change: transform`, minimal dimensions. On `compositionend`, the final text is transferred to the model and the textarea cleared.

Constraints to uphold: hidden native caret (`caret-color: transparent`); `user-select: none` on the canvas; textarea positioned at its **real** position (never `left: -9999px`, otherwise the IME popup displays there); interception of `ArrowUp/Down`, `Home/End`, `Ctrl+A/C/V/X` in `keydown` (clipboard via `copy`/`paste` + `event.clipboardData` from the model); the textarea follows canvas scroll.

**WYSIWYG (Prose).** The textarea carries only **plaintext**; style lives in the Loro model (`LoroText` + attributes) and the canvas renders via Parley. Cross-device fidelity rests on fonts served by Ambre (Noto Sans/Mono) and deterministic Skrifa metrics — shared with krilla PDF export via `DrawSurface`.

### D. Content Rendering (reference — detail in `ambre-spec-canvas.md`)

Graphics pipeline wgpu: instanced quads, vertex/fragment shaders, standard textures, **no compute** on the hot path. Text: Parley (layout/shaping) → Swash/Skrifa (CPU rasterization) → glyph atlas texture (R8, LRU) → quads. WebGL2 remains a functional GPU rendering backend (atlas ≤ 4096^2).

### E. Adaptive Design

> **Design tokens** (single source `tokens.ron` → `theme.css` + `theme.rs`) are defined in `ambre-spec-ui.md §2.D`. This section covers chrome `DisplayMode` and adaptive breakpoints.

**Breakpoints** (achieved via CSS/media queries for chrome; read on the canvas side via logical width for content layout):

```
Mobile: < 600 dp   |   Tablet: 600–1024 dp   |   Desktop: > 1024 dp
```

| Component | Mobile | Tablet | Desktop |
|:---|:---|:---|:---|
| Navigation | Drawer + bottom bar | Collapsible sidebar | Persistent sidebar |
| Toolbar | Icons only, aggressive overflow | Icons + short labels | Icons + labels + shortcuts |
| Side panels | Hidden, swipe/button | Collapsible | Persistent |
| Context menus | Full-width bottom sheet | Popover | Popover |
| CommandPalette | Fullscreen | 70% modal | 50% modal |
| Data views | Priority columns + H scroll | All columns | All columns + freeze |

### F. Accessibility

The DOM provides a11y primitives natively (ARIA, focus, screen readers). **Interaction patterns** are implemented in `ambre-chrome`: roving tabindex of menus, focus trap (except `<dialog>`), `aria-expanded`/`aria-controls`, live regions for toasts. Territory mapped by WAI-ARIA Authoring Practices, written **once** for shared chrome components. Canvas **content** a11y goes through **AccessKit**, on a perimeter reduced to content.

### G. Launcher Screen (the `/` route)

The launcher is the **app-agnostic** home shown on the `/` route: a full-bleed Leptos DOM overlay rendered **on top of** the persistent canvas (the canvas is never unmounted, only covered and its rAF paused — `ambre-spec-architecture.md §6`). It hosts no document content, only chrome, so it lives here and **not** in any app crate (`ambre-trame`, ...): every `DocType` is listed through the same view. Implemented in `ambre-app` (the launcher crate) on the shared `ambre-chrome` toolkit, **driven entirely by design tokens** (`ambre-spec-ui.md §2.D`) — no ad-hoc colours.

It has two faces, switched on the session state (`username: Option<String>` filled by `RestEvent::SessionReady`):

**Signed out — sign-in panel.** A centered panel carrying the Ambre wordmark and a one-line product tagline, then:
- **OIDC** (production): a "Sign in with Google" button → `window.location = /api/auth/google/login`.
- **Dev login** (server reports `dev_mode`): a username field dispatching `RestCommand::DevLogin` to the live Logic thread (no page reload — the Sync worker keeps its keys, `ambre-spec-architecture.md §6`).
- A neutral "Connecting..." placeholder until `/api/auth/config` answers.

**Signed in — document list.** A header (Ambre wordmark, current user, "new document" action) over a list of **document cards**. Each card is one `DocumentSummary` and shows:

| Field | Source | Notes |
|:---|:---|:---|
| **Type icon** | `DocType` | One glyph per type (Prose / Trame / Toile / Murmure), from the icon set. |
| **Name** | `DocumentSummary.name` | The **decrypted** title (see ZK note below). `None` → a dimmed/italic **"Untitled"** placeholder — **never** the raw `document_id` / a short hash, and **never** the document type word (the icon already carries the type). |
| **Last modified** | `DocumentSummary.updated_at` | Relative time ("2 h ago"); routing metadata, plaintext, no crypto cost. |
| **Shared badge** | member count > 1 | Discreet chip; member count is routing metadata (plaintext). |

- **Open**: client `pushState` to `/{doc_type}/{id}`; the persistent canvas swaps document in place via `AppView::set_document` (no full-page nav, no worker respawn — `ambre-spec-architecture.md §6`).
- **New document**: generate the id client-side (UUID v7) and **open it immediately** — **no name prompt** (friction on every create; the clean default is unnamed). The document opens **untitled** (`name = None`); the server row is created lazily on first edit. `CreateDocument { doc_type, name }` carries a name up to Sync (which encrypts it) only once the user actually sets one.
- **Rename / Delete**: per-card actions → `RestCommand::RenameDocument` / `DeleteDocument`. Renaming is also available inline from the open document's header, at any time.
- **Empty state**: a soigné placeholder ("No documents yet — create one above"), not a bare list.

**Naming strategy (normative).** The default of a new document is the **absence of a name**, not a generated string:
- Stored: `name = None` until the user renames. A placeholder is **never** persisted into `name_enc` (a stored "Untitled" / "Trame 1" is indistinguishable from a user who typed it, breaks "is this named yet?", and pollutes the ciphertext).
- **No counters** ("Trame 1"): counter state collides across offline devices (each device has its own client-decrypted list, no server authority over names — `ambre-spec-architecture.md §3`) and rots after deletes.
- **No type word in the name** ("Trame - ..."): the type is carried by the **icon** (and a small type chip) — repeating it is the redundant pattern this redesign removes.
- **Icon-less contexts** (browser tab title, share invitations, future global search) compose the placeholder with the **generic, localized** type label (e.g. tab = "Untitled — Spreadsheet"), **at the point of use, never stored**. This generic label is distinct from the brand `DocType::name()` ("Trame") used as the type chip.
- *(Deferred, see §6)* auto-suggesting a title from first content (A1 for Trame, first heading for Prose) — type-specific, out of MVP.

**MVP scope (normative).** Type icon + name (or "Untitled" placeholder) + last-modified + shared badge; create-untitled, inline rename, delete; sign-in; empty state; full token theming. **Deferred (see §6):** document thumbnails/previews, content stats (cell/word/page counts), folders / favorites / custom ordering, auto-suggested titles.

**ZK note.** Document names are **end-to-end encrypted**: the launcher only ever renders `DocumentSummary.name` already decrypted by the Sync thread (`name_enc` + `doc_key_enc` → Document Key → plaintext, `ambre-spec-architecture.md §3`). The chrome itself performs no crypto and no `fetch` (§3). Names being absent (`None`) is a normal state (never named, never opened, or key not yet available) and renders as the "Untitled" placeholder.

---

## 3. SECURITY & CONFIDENTIALITY (ZERO-KNOWLEDGE)

Pure presentation layer:
- **No network outbound flows**: chrome invokes `CommandId` → `LogicCommand` (`flume` channels); content comes from the Logic thread. No direct `fetch`/WebSocket (exception: same-origin fonts, rendering side).
- **No direct persistence**: UI states (panel ratios, accordions) are sent up to the Logic thread, which decides.
- **Tokens/theme**: colors and metrics only, no user data.
- **Invariant**: zero calls outside the WASM process in this layer.

---

## 4. QUALITY & TESTABILITY

- **`ambre-commands`**: registry lookup, keybinding resolution, `CommandState` derivation. Pure functions, testable on host.
- **`ambre-chrome`**: Leptos components via `wasm-bindgen-test` (DOM) — a11y patterns (roving tabindex, focus trap, `aria-*`), menu positioning (flip on viewport overflow).
- **Canvas cohabitation**: an end-to-end command (click → `CommandId` → `LogicCommand` → `UiEvent` → toggle chrome + canvas repaint); an IME composition cycle; rAF suspension/resumption behind a modal.
- **Mocks & environment**: `RecordingSurface` (the mock `DrawSurface` in `ambre-draw`, feature `testing`) for headless content rendering; native `ambre-commands` tests outside WASM.

---

## 5. PERFORMANCE & RESOURCES

### A. Nominal Prose Frame (target 16.6 ms @ 60 FPS)

| Phase | Budget | Notes |
|:---|:---|:---|
| Keypress → beforeinput → dispatch | ~0.1 ms | Negligible |
| Round-trip to logic (`flume` channels) | low | Typed inter-thread transport |
| `LoroText::insert` | < 0.1 ms | Sequential insert |
| Parley layout on **dirty paragraph only** | 0.5–2 ms | Hot spot |
| Glyph rasterization (Swash) if new glyph | ~0 ms nominal | Atlas hit |
| Vertex/instance buffer build | 0.2–0.5 ms | |
| wgpu submit + present | 0.5–1 ms | Textured quads |
| **Total typical** | **~2–5 ms** | Large margin under 16.6 ms |

### B. Threats to 60 FPS (and countermeasures)
1. **Parley without incremental layout** → atomicity at the **paragraph** (one `Layout` per paragraph); monster paragraph (> N KB) → internal split, layout in `requestIdleCallback`.
2. **Glyph atlas overflow** → LRU per 256^2/512^2 slot, separate cache per size.
3. **Long document scroll** → **per-paragraph virtualization** (render visible + buffer, `Layout` cached).
4. **Multi-paragraph selection** → reuse cached `Layout` to extract rectangles.
5. **Mid-range mobile** → wgpu quads holds 60 FPS; the risk is CPU (Parley + Loro + dispatch, ~6–8 usable ms).

**Regularity target:** prioritize **p99** (a stable 45 FPS is perceived as smoother than 60 FPS with stutters) over max FPS. Guaranteed 60 FPS mobile everywhere is not a goal.

---

## 6. REJECTED ARCHITECTURAL DECISIONS

> **UI model** rejections (100% canvas UI, Xilem/Masonry, winit, WebGPU-only, pure native): `ambre-spec-ui.md §6`. **Content rendering** rejections (Vello, `vello_hybrid`, Canvas 2D): `ambre-spec-canvas.md §6`. Below, **chrome**-specific rejections.

- **egui for the entire app**: a uniform look does not suit a premium product (egui remains a reference for the text pipeline: atlas + quads).
- **Office-style ribbon as default**: dense, poorly responsive, hostile to mobile. Adaptive contextual toolbar instead (still generable from the registry).
- **Reimplementing focus trap / backdrop for modals**: native `<dialog>` provides them.
- **CSS Anchor Positioning for menus**: cross-browser support too recent; manual positioning.
- **All apps' chrome in `ambre-chrome`**: `ambre-chrome` only carries **shared** chrome; app-specific lives in each app crate (`ambre-spec-ui.md §2.B`).
- **Launcher document previews / thumbnails (deferred, not rejected)**: an encrypted thumbnail is a full mini-pipeline — offscreen render on save (hot path), cross-peer invalidation policy, an extra encrypted blob per doc (S3, wrapped under the Document Key), and decrypt-N-images on list load (mobile memory/latency, against the < 3 s on 4G target). Purely cosmetic gain over icon + name + date. Reconsider only with Toile (the one type where a preview beats an icon). MVP shows **icon + name + last-modified + shared badge** (§2.G).
- **Launcher content stats (cell/word/page counts) (deferred)**: unlike `updated_at` / role / member count (routing metadata, plaintext, free), content stats must be computed client-side and stored **encrypted** like the name — another coherent blob to maintain for low value. Out of MVP.

---

## 7. INTEGRATION PLAN (ROADMAP)

> Detailed backlog of this spec; cross-cutting scheduling (milestones) lives in `specs/ambre-roadmap.md`.
> Content rendering (canvas, `CANVAS P0`) is a prerequisite **done** — see `ambre-spec-canvas.md §7`.

### CHROME P1 — Chrome Foundation
- [x] **Lift the integration risk**: validate Leptos on the main thread under the `+atomics`/`shared-memory` build + nightly `build-std` of `wasm_thread`
- [x] `ambre-commands` (registry, keybindings) and `ambre-chrome` (shared Leptos toolkit) crates
- [x] Restrict `ambre-canvas` to render runtime (remove widget framework: `widget`/`widget_pod`/`constraints`/`focus`/`layer`/`context`/`display_mode`/`theme` widgets; deps taffy/slotmap/smallvec)
- [x] Leptos app shell (header + contextual toolbar + status bar) with central `<canvas>` cell
- [x] Canvas bootstrap + wgpu **without winit**: surface, `ResizeObserver`, event routing via `web-sys`
- [x] **One end-to-end command**: Bold button → `CommandId` → `LogicCommand` → `UiEvent` → button toggle + canvas reaction
- [x] `<dialog>` modal suspending rAF; context menu triggered by canvas right-click

### CHROME P2 — Input & Editing
- [x] Transparent `<textarea>`: Latin input + IME composition cycle
- [x] Trame WYSIWYG on a cell (plaintext textarea → Loro attributes → Parley rendering)
- [x] Floating UI anchored to pan/zoom (transform-synced selection toolbar)

### CHROME P3 — Adaptive, Tokens & A11y
- [x] Design tokens: `tokens.ron` → `theme.css` + `theme.rs` (build.rs), dark/light switch
- [x] CSS Mobile/Tablet/Desktop breakpoints on the chrome
- [x] ARIA a11y patterns in `ambre-chrome` (roving tabindex, focus trap, live regions); content a11y = DOM mirror (accessible textarea + live region announcing active cell). *AccessKit ruled out on the web: the a11y tree is the DOM, no web adapter — the DOM mirror replaces it.*
- [x] `performance.mark`/`measure` profiling desktop **and** mobile (keydown → caret < 50 ms target / < 100 ms acceptable; sustained FPS while typing; paste 10 KB)
