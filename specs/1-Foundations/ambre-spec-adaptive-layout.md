# AMBRE -- FEATURE: THE ADAPTIVE LAYOUT

**Type:** Horizontal (framework)
**Implementation:** partial -- three widths are declared and three things move between them; the
navigation, the panels and the toolbars do not.

> How the interface changes with the width of the window. Three widths, one stylesheet, and a rule
> that the narrow layout is the one everything starts from.

The content is unaffected: a view lays out from the box it is handed, so the whole of this document
is about the chrome around it.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`. The prefix is `CHROME`, shared with the
other five documents of the DOM side.

| ID | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|
| `CHROME-030` | The chrome MUST adapt at three declared widths, and each threshold MUST be written once. | this spec section 2 | none. Three width queries sit in one stylesheet and no other file carries a threshold. Nothing refuses a fourth query added beside them, which is how two components come to disagree about where a phone stops | unverified |
| `CHROME-031` | The narrowest layout MUST be the default: a wider window MUST only add to it. | this spec section 2 | none. Each adapting rule hides at the base and restores upward, so a component with no query at all is correct on a phone. Nothing refuses the reverse, which fails on the width nobody tests on | unverified |
| `CHROME-032` | On a phone, a floating menu MUST become a sheet pinned to the bottom edge. | this spec section 2 | none. The style pins all four edges and overrides the placement the arithmetic of `CHROME-003` computed, so the two agree by one of them winning. No test drives a narrow viewport | unverified |
| `CHROME-033` | Navigation and panels MUST stay reachable at every width: a region MUST collapse, never disappear. | this spec section 2 | not built -- lands with `The adaptive chrome` (`ambre-roadmap.md` E6). No sidebar, no drawer and no panel region exists at any width, so there is nothing to collapse yet | unimplemented |
| `CHROME-034` | A content view MUST lay out from the size it is given, never from a device class. | this spec section 2; `ambre-spec-frame.md` | none. No code outside the stylesheet reads a width threshold, so the rule holds by absence. Nothing refuses the first view that reads the window and branches on it | unverified |

* **Budget.** none: adaptation is style, resolved by the browser, and
  `ambre-spec-budgets.md` budgets no chrome operation. A resize does cost a frame,
  and that is charged to `ambre-spec-frame.md`.
* **Boundary.** none: the window width is read from the browser and never leaves the client. It is a
  signal the page already carries, and nothing sends it anywhere.
* **Verification.** 0 `verified`, 4 `unverified`, 1 `unimplemented`. Nothing holds a single row, and
  the reason is one reason: every rule is a property of a rendered page at a given width, and no
  tier resizes a viewport. What the stylesheet declares is checked by looking at it.

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

**What moves today.** Three rules, each written as a hide at the base and a restore upward
(`CHROME-031`).

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
| the toolbar | icons alone, overflowing early | icons and short labels | icons, labels and chords |
| the side panels | hidden, opened by a gesture or a control | collapsible | persistent |
| the command palette | full screen | most of the window | half the window |
| a wide data view | the priority columns, and horizontal scrolling | every column | every column, with a frozen edge |

**The sheet, and the arithmetic it overrides.** On a phone the menu's placement is still computed --
the click point, the flip and the clamp of `CHROME-003` -- and then every edge is overridden by the
style, so the sheet wins. The arithmetic is not wrong there; it is simply not what a thumb wants.

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

## 4. DECISIONS

| Decision | Rationale | Rejected alternative |
|---|---|---|
| Three widths, declared once, in the stylesheet | a threshold is a design constant, and the browser already resolves it without a frame, a listener or a signal. Putting it in Rust means a resize listener, a signal and a re-render for something the style engine does for free | a display mode computed on resize and pushed through the reactive layer |
| The narrow layout is the default | a component written with no query at all is then correct on a phone, and a missing query degrades to less rather than to broken. The reverse makes the smallest screen depend on the rule nobody remembers to write | a desktop default, narrowed downward |
| A phone gets a bottom sheet, not a smaller popover | a popover under the finger is under the finger: the hand covers what it opened, and the entries are too small to hit. A sheet at the bottom edge is where the thumb already is, and it is the platform idiom on both mobile operating systems | the same popover, scaled down |
| The content reads no threshold | a view that branches on a device class has two layouts to keep correct, two sets of pixel arithmetic and two things to test, for a document whose content did not change. A continuous size has one | a content layout per width band |
| Density-independent pixels, not raw pixels | a raw pixel count calls a modern phone a desktop, because the screen has three physical pixels per logical one. What the layout is written against is what the user can see, which is the logical unit `ambre-spec-high-density.md` names | a threshold in physical pixels |

---

## 5. NOT BUILT, AND NOT YET RULED

* **Most of the adaptation table has no code.** The navigation, the panels, the toolbar overflow and
  the data-view column priority are designed and absent, so the interface today is one layout with
  three small differences (`CHROME-033`).
* **No tier resizes a viewport.** The wasm tier mounts components in a document of fixed size, so
  every row here is held by reading the stylesheet.
* **An open menu does not re-place itself on a resize.** The placement is computed when the menu
  opens, and a window narrowed under it leaves it where it was.
* **Nothing rules on orientation, or on a soft keyboard.** A phone rotated, or a keyboard covering
  half the viewport, changes the usable height and no rule reads it.
* **Nothing rules on a pointer that is not a mouse.** The widths say how much room there is, not
  whether the user has a finger or a cursor, and the two questions are not the same one.
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
