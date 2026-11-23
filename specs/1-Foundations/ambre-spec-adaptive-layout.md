# AMBRE -- FEATURE: THE ADAPTIVE LAYOUT

**Type:** Horizontal (framework)

> How the interface changes with the width of the window. Three widths, one stylesheet, and a rule
> that the narrow layout is the one everything starts from.

The content is unaffected. A view lays out from the box it is handed, so this document is about the
chrome around it.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`. The prefix is `CHROME`, shared with the
other six documents of the DOM side.

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `CHROME-030` | `INV` | The chrome MUST adapt at three declared widths. | this spec section 2 | ci-gate -- `scripts/crate_boundaries.py` refuses a width query in the shell stylesheet other than the three it declares | none | verified |
| `CHROME-056` | `INV` | Each threshold MUST be written once. | this spec section 2 | ci-gate -- `scripts/crate_boundaries.py` refuses a width query spelled in any Rust, CSS or HTML file outside the shell stylesheet and the density module, and a `matchMedia` a page or a script spells; ci-gate -- the rustc driver of `crates/ambre-primitives`, run by `just primitives` in the CI `lint` job, reads each path once it resolves, so an alias, a re-export and a macro name what they reach. It refuses a media query that names a dimension, read outside the density module, test code included: width, height, aspect ratio or resolution, with a `min-`, `max-` or `device-` form, on either side of a comparison. A query it cannot read as a literal is refused too (`ambre_primitives::driver::chrome::a_media_read_that_names_a_dimension_is_refused_in_test_code_too`) | none | verified |
| `CHROME-031` | `INV` | The narrowest layout MUST be the default: a wider window MUST only add to it. | this spec section 2 | ci-gate -- `scripts/crate_boundaries.py` refuses a `display: none` or `visibility: hidden` inside a width query, so a query only adds | none | verified |
| `CHROME-032` | `OBS` | On a phone, a floating menu MUST become a sheet pinned to the bottom edge. | this spec section 2 | `ambre_app::shell::narrow::a_phone_menu_is_a_sheet_on_the_bottom_edge` -- the context menu opened at a click point in a frame of 360, 599 and 600 pixels, its edges read off the layout | none | verified |
| `CHROME-057` | `OBS` | A keyboard chord MUST NOT be shown below the desktop width. | this spec section 2, the table of what moves today | `ambre_app::shell::sync_badge::tests::the_sync_label_and_the_chord_appear_with_the_width` -- a chord chip mounted in a frame of 400, 599, 600, 1024 and 1025 pixels, read off the computed style | none | verified |
| `CHROME-058` | `OBS` | On a phone, the sync state MUST show its icon without its label. | this spec section 2, the table of what moves today | `ambre_app::shell::sync_badge::tests::the_sync_label_and_the_chord_appear_with_the_width` -- the sync badge mounted with the shell stylesheet at each side of 600, the label read off the computed style | none | verified |
| `CHROME-033` | `OBS` | Navigation and panels MUST stay reachable at every width: a region MUST collapse, never disappear. | this spec section 2 | not built | unbuilt -- lands with the `The adaptive chrome` Story (`ambre-roadmap.md` E6). No sidebar, no drawer and no panel region exists at any width, so there is nothing to collapse yet | unimplemented |
| `CHROME-117` | `OBS` | A bar of the shell MUST move the controls its width cannot hold into an overflow menu, never clip them. | this spec section 2 | not built | unbuilt -- lands with the `The adaptive chrome` Story (`ambre-roadmap.md` E6). Each bar keeps one row of controls at every width | unimplemented |
| `CHROME-034` | `INV` | A content view MUST lay out from the size it is given, never from a device class. | this spec section 2; `ambre-spec-frame.md` | ci-gate -- the rustc driver of `crates/ambre-primitives`, run by `just primitives` in the CI `lint` job, reads each path once it resolves, so an alias, a re-export and a macro name what they reach. It refuses a size of the window or of the screen, and a media query, read in a content view (`ambre_primitives::driver::chrome::a_window_read_under_a_re_export_is_refused_in_a_content_view`) | none | verified |

* **Budget.** none: adaptation is style, resolved by the browser, and
  `ambre-spec-budgets.md` budgets no chrome operation. A resize does cost a frame,
  and that is charged to `ambre-spec-frame.md`.
* **Boundary.** none: the window width is read from the browser and never leaves the client. It is a
  signal the page already carries, and nothing sends it anywhere.
* **Verification.** 7 `verified`, 2 `unimplemented`. A width query reads the viewport of its own
  frame. The browser tier sets a width by mounting a component in a sized frame. The rules of
  absence are held by a scan of the source.
---

## 2. MODEL

**CRDT containers.** none. The width of a window is a property of the machine, never of the
document.

**The three widths.** Density-independent pixels, so a phone and a scaled desktop are compared on
what the user can actually see rather than on a raw pixel count.

| Width | From | To |
|---|---|---|
| phone | -- | under 600 |
| tablet | 600 | 1024 |
| desktop | over 1024 | -- |

**What moves today.** Four entries. The three that appear and disappear are each written as a hide
at the base and a restore upward (`CHROME-031`); the fourth overrides a length.

| What | Phone | Tablet | Desktop |
|---|---|---|---|
| a shortcut chip in a menu | hidden | hidden | shown |
| the sync state's label beside its icon | hidden | shown | shown |
| the floating menu | a full-width sheet at the bottom edge, with taller rows | a popover under the pointer | a popover under the pointer |
| the header and command bar spacing | tightened | standard | standard |

**What is designed and absent.** Each row is a region that does not exist at any width yet, so it
adapts at none (`CHROME-033`).

| What | Phone | Tablet | Desktop |
|---|---|---|---|
| navigation | a drawer plus a bottom bar | a collapsible sidebar | a persistent sidebar |
| a bar of the shell | icons alone, overflowing early | icons and short labels | icons, labels and chords |
| the side panels | hidden, opened by a gesture or a control | collapsible | persistent |
| the command palette | full screen | most of the window | half the window |
| a wide data view | the priority columns, and horizontal scrolling | every column | every column, with a frozen edge |

**The sheet, and the arithmetic it overrides.** On a phone the menu's placement is still computed: the
click point, the flip and the clamp of `CHROME-003`. Every edge is then overridden by the style, so
the sheet wins. The arithmetic is not wrong there; it is simply not what a thumb wants.

**The content does not adapt, and that is the design.** A view is handed the size of its cell and
lays out from it. Nothing in the content path reads a threshold, so a narrow window gives a smaller
grid rather than a different one (`CHROME-034`). The two halves of the interface therefore reach a
narrow screen by two routes: the chrome by declared widths, the content by a continuous size.

**Code:** `crates/ambre-app/styles.css` holds the three width queries and every rule under them.
There is no Rust: no type names a display mode, and no view reads the window's width.

---

## 3. SURFACE

The user touches nothing here. What they see is whether the interface fits.

| Trigger | Behaviour |
|---|---|
| the application opens on a phone | the chrome is tight, chords are hidden, and the menu opens as a sheet at the bottom |
| the application opens on a tablet | the sync state gains its label, and the menu is a popover again |
| the application opens on a desktop | the chords appear beside the menu entries |
| the window is resized across a threshold | the chrome reflows at once, and the content resizes without changing shape |
| the window is narrowed with a menu open | the menu keeps its placement until it is reopened |

---

## 4. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| Three widths, declared once, in the stylesheet | a display mode computed on resize and pushed through the reactive layer | it buys a resize listener, a signal and a re-render for something the style engine does for free: a threshold is a design constant, and the browser already resolves it without a frame |
| The narrow layout is the default | a desktop default, narrowed downward | it makes the smallest screen depend on the rule nobody remembers to write, so a missing query degrades to broken rather than to less. With the narrow default, a component written with no query at all is correct on a phone |
| A phone gets a bottom sheet, not a smaller popover | the same popover, scaled down | a popover under the finger is under the finger: the hand covers what it opened, and the entries are too small to hit. A sheet at the bottom edge is where the thumb already is, and it is the platform idiom on both mobile operating systems |
| The content reads no threshold | a content layout per width band | a view that branches on a device class has two layouts to keep correct, two sets of pixel arithmetic and two things to test, for a document whose content did not change. A continuous size has one |
| Density-independent pixels, not raw pixels | a threshold in physical pixels | a raw pixel count calls a modern phone a desktop, because the screen has three physical pixels per logical one. What the layout is written against is what the user can see, which is the logical unit `ambre-spec-high-density.md` names |

---

## 5. NOT BUILT, AND NOT YET RULED

* **Most of the adaptation table has no code.** The navigation, the panels, the toolbar overflow and
  the data-view column priority are designed and absent. The interface today is one layout with
  three small differences (`CHROME-033`).
* **No tier resizes a viewport.** The wasm tier mounts components in a document of fixed size, so
  every row here is held by reading the stylesheet.
* **The phone block has no scale of its own.** It tightens the header and the command bar and
  grows the sheet's rows by picking other steps of the one spacing ladder
  (`ambre-spec-design-tokens.md`, `UI-THREAD-029`). Nothing states which steps a phone uses.
* **An open menu does not re-place itself on a resize.** The placement is computed when the menu
  opens, and a window narrowed under it leaves it where it was.
* **Nothing rules on orientation, or on a soft keyboard.** A phone rotated, or a keyboard covering
  half the viewport, changes the usable height and no rule reads it.
* **Nothing rules on a pointer that is not a mouse.** The widths say how much room there is, not
  whether the user has a finger or a cursor. The two questions are not the same one.
* **The thresholds are unmeasured.** 600 and 1024 are the conventional pair and no measurement of
  this interface produced them.

---

## 6. WHAT THIS DOCUMENT DOES NOT OWN

| Question | Owner |
|---|---|
| The components that adapt: the shell, the menu, the modal, the status bar | `ambre-spec-chrome.md` |
| The placement arithmetic a phone overrides | `ambre-spec-chrome.md`, `CHROME-003` |
| The logical pixel, and what a dense display multiplies | `ambre-spec-high-density.md` |
| The palette, the spacings and the type sizes a layout is built from | `ambre-spec-design-tokens.md` |
| What a resize makes the content rebuild | `ambre-spec-frame.md` |
| Whether a control is reachable at all, at any width | `ambre-spec-accessibility.md` |
| The home screen's own layout | `ambre-spec-launcher.md` |
