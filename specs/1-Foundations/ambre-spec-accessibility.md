# AMBRE -- FEATURE: ACCESSIBILITY

**Type:** Horizontal (framework)

> What a screen reader and a keyboard reach. The chrome is DOM, so it inherits the platform's
> accessibility for free. The content is pixels we drew, so it reaches assistive technology only
> through what we put in the page beside it.

The interface is two halves that arrive at a screen reader by two entirely different routes. This
document is where they are made to agree.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`. The prefix is `CHROME`, shared with the
other five documents of the DOM side.

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `CHROME-006` | `INV` | A toast MUST be announced to a screen reader. | this spec section 2 | `ambre_chrome::chrome_ui::toast_host_is_an_aria_live_region` -- it reads the host's role and its politeness back off a real `document`, so a host that stopped being a live region fails | none | verified |
| `CHROME-065` | `INV` | Announcing a toast MUST NOT move focus. | this spec section 2, the pattern table; this spec section 3, row 4 | `ambre_chrome::chrome_ui::a_toast_leaves_the_focus_where_it_was` -- it focuses a control, raises a toast, and reads the focused element back once the toast is in the page | none | verified |
| `CHROME-022` | `INV` | A control carrying no text MUST carry an accessible name. | this spec section 2 | `ambre_chrome::chrome_ui::format_toggle_reflects_pressed_and_reports_clicks` -- its first assertion reads the accessible name off an icon-only toggle | none | verified |
| `CHROME-066` | `INV` | A control with two states MUST report which one it is in. | this spec section 2 | `ambre_chrome::chrome_ui::format_toggle_reflects_pressed_and_reports_clicks` -- it reads the pressed state at false, after the signal rises, and after it falls; ci-gate -- `scripts/crate_boundaries.py` refuses a pressed, expanded, checked or selected state written so that false drops the attribute | none | verified |
| `CHROME-069` | `INV` | A menu trigger MUST declare that it opens a menu. | this spec section 2, the pattern table | `ambre_chrome::chrome_ui::zoom_chip_renders_readout_from_signal_and_starts_collapsed` -- it reads the trigger's popup declaration off the mounted button | none | verified |
| `CHROME-070` | `INV` | A menu trigger MUST report whether it is open. | this spec section 2, the pattern table | `ambre_chrome::chrome_ui::a_menu_trigger_reports_open_and_closed` -- it reads the expanded state at false, after a click opens the menu, and after a second click closes it; ci-gate -- the same refusal of a state whose false drops the attribute | none | verified |
| `CHROME-023` | `OBS` | The focused unit of drawn content MUST be announced, in every application that draws content. | this spec section 2; `ambre-spec-frame.md`, `CANVAS-009` | `ambre_trame::ui::view::tests::the_active_cell_is_announced_by_reference_and_value` -- the grid says the cell and its value, or that it is empty; `ambre_toile::ui::a11y::tests::a_board_announces_its_selection` -- one element by its kind and text, several by their count; `ambre_toile::ui::a11y::tests::an_element_is_announced_once_its_data_arrives_and_never_whole` -- a lone element waits for its data, and a long body is cut; `ambre_prose::view::a11y::tests::a_page_announces_the_block_at_the_caret` -- the block at the caret by its kind and text; `ambre_murmure::ui::view::tests::up_and_down_move_the_focus_and_say_each_message` -- the focused message by its author, its time and its text; `ambre_belt::prose::tests::the_keyboard_walks_the_caret_through_the_text` -- browser tier: the arrow keys and `Ctrl+Home` move the caret, and the live region names each paragraph it enters | none | verified |
| `CHROME-024` | `OBS` | The name the content announces MUST describe the kind of document the view holds. | this spec section 2 | `ambre_canvas::text_input::tests::only_the_spreadsheet_names_itself_a_cell` -- four kinds, four names, and only the grid says it is a cell | none | verified |
| `CHROME-025` | `INV` | The element that carries content focus MUST stay in the page's tab order. | this spec section 2; `ambre-spec-text-input.md` | ci-gate -- `scripts/crate_boundaries.py` refuses a pipe that no longer sets a zero tab index, or sets a negative one or `inert` | none | verified |
| `CHROME-067` | `INV` | The element that carries content focus MUST NOT be hidden from assistive technology. | this spec section 2; `ambre-spec-text-input.md` | ci-gate -- `scripts/crate_boundaries.py` refuses `aria-hidden` and `inert` in the pipe's code, comments aside | none | verified |
| `CHROME-026` | `INV` | A menu MUST be fully operable from the keyboard: move, jump to an end, jump by first letter, choose, and dismiss. | this spec section 2 | `ambre_chrome::chrome_ui::a_menu_is_operable_by_keyboard_alone` -- the zoom menu takes the focus, the arrows move and wrap, Enter chooses and Escape closes; `ambre_app::landing::tests::the_type_menu_is_operable_by_keyboard_alone` -- End, a first letter read from the item's name, Enter and Escape; `ambre_chrome::menu::tests::a_letter_jumps_to_the_next_item_it_starts` -- the shared key map, Home and End beside it; `ambre_chrome::chrome_ui::the_floating_menu_is_operable_by_keyboard_alone` -- the floating menu: the arrows wrap and skip a separator, Home and End, a letter to the next row it starts, and Enter runs the focused row; ci-gate -- `scripts/crate_boundaries.py` refuses a file that renders a menu and names no key handler | none | verified |
| `CHROME-027` | `INV` | Closing a floating menu MUST return focus to what opened it. | this spec section 2; `ambre-spec-chrome.md`, `CHROME-004` | `ambre_chrome::chrome_ui::closing_the_floating_menu_returns_the_focus_to_its_opener` -- the shared floating menu gives the focus back on Escape, and leaves a focus the user moved; `ambre_chrome::chrome_ui::a_key_that_closes_a_trigger_menu_returns_the_focus_to_its_trigger` -- a menu opened from a trigger gives it back on Escape and on Enter, through the key map every such menu shares | none | verified |
| `CHROME-028` | `INV` | A modal MUST trap focus. | this spec section 2 | `ambre_chrome::chrome_ui::an_open_modal_traps_the_focus_and_makes_the_page_inert` -- the focus moves inside on open and a control behind cannot take it; ci-gate -- `scripts/crate_boundaries.py` refuses a dialog role or `aria-modal` outside the shared dialog, so no panel poses as a modal | none | verified |
| `CHROME-068` | `INV` | A modal MUST make the rest of the page inert. | this spec section 2 | `ambre_chrome::chrome_ui::an_open_modal_traps_the_focus_and_makes_the_page_inert` -- the control behind refuses the focus while the modal is open and takes it once it closes; ci-gate -- the same refusal of a panel posing as a modal | none | verified |
| `CHROME-029` | `INV` | Drawn content MUST NOT be the only carrier of a state: a state the user needs MUST also reach the page. | this spec section 2 | not built | unbuilt -- lands with the `Drawn content reaches a screen reader` Story (`ambre-roadmap.md` E6). The selection, the active tool, the sync state and the presence of collaborators are drawn, and only the last two also exist as chrome | unimplemented |

* **Budget.** none: an announcement is a text write into a hidden element, off the frame path, and
  `ambre-spec-budgets.md` budgets no chrome operation.
* **Boundary.** none: an announcement carries what the user already sees on their own screen, into
  their own page. It reaches no relay and crosses no thread.
* **Verification.** 14 `verified`, 1 `unimplemented`. Every row is an `INV` or an `OBS`. This
  document owns no gesture, the components belong to `ambre-spec-chrome.md`, and the announcement
  belongs to the view that draws.

  The chrome rules are read off a mounted component: its states at both values, its focus, and a
  menu driven by keys alone. Three absences hold by gate: a state that drops false, a panel posing
  as a modal, and a pipe hidden or out of the order. The two content rules are held by the text
  each view composes. What a screen reader says needs one attached, and no tier drives one
  (`ambre-spec-testing.md section 8`).
---

## 2. MODEL

**CRDT containers.** none. Accessibility carries no document state; it re-presents what a view
already holds.

**The two halves, and why they differ.**

| Half | Reaches a screen reader by | What we owe it |
|---|---|---|
| the chrome | being DOM: roles, names, focus and tab order are the platform's | using the platform's own patterns rather than inventing them |
| the content | nothing, by default: it is pixels in a canvas element | a parallel presence in the page, kept in step with what is drawn |

**What the chrome uses.** Each is a documented interaction pattern, implemented once in the shared
toolkit and inherited by every application.

| Component | Pattern |
|---|---|
| the modal | the platform's modal element: the focus trap, the inertness and Escape come with it |
| the floating menu | a menu with a roving focus, arrow keys, Home and End, first-letter jump, and Escape |
| the toasts | one polite live region for the whole application, so a message is read and focus stays put |
| a menu trigger | it declares that it opens a menu, and reports whether it is open |
| an icon-only control | an accessible name, and a pressed state when it has two states |
| a value menu | each entry reports whether it is the chosen one |

**How the content reaches the page.** The web has no platform accessibility adapter to push a tree
to: on the web the accessibility tree **is** the DOM. So the content's route is a mirror, not a
tree, and it is two elements beside the canvas.

| Element | Role | Carries |
|---|---|---|
| the input pipe | a focusable text box, named after the unit being edited | the keyboard focus, and the content's place in the tab order |
| the live region | a visually hidden polite status region | the focused unit, re-written each time the selection moves |

Writing new text into the live region is what produces the announcement. The scope is deliberately
the focused unit and not the document. A hundred-thousand-row mirror is a second rendering of the
document, in DOM, and that cost is what the whole content design exists to avoid.

**What each application says.** The mechanism is shared, and the words are each view's own.

| Application | The pipe is named | Announces |
|---|---|---|
| the spreadsheet | a spreadsheet cell | the active cell reference and its value on every selection change, plus a refused operation |
| the board | a whiteboard | the selection: one element by its kind, its text and its lock, several by their count, and an emptied one as such |
| the page | a document | the block the caret enters, by its kind and the start of its text; a move inside the block says nothing |
| the conversation | a conversation | the focused message, by its author, its time and its text, and a new one while the list rests at the bottom |

One pipe serves the whole page, so the top-level view names it. A document embedded in a board
leaves the name alone, since the board still owns the pipe (`CHROME-024`).

**Code:** `crates/ambre-canvas/src/text_input.rs` creates the pipe and the live region, holds the
announcement call and the kinds. The words are `crates/ambre-trame/src/ui/view/mod.rs`,
`crates/ambre-toile/src/ui/a11y.rs`, `crates/ambre-prose/src/view/a11y.rs` and
`crates/ambre-murmure/src/ui/list.rs`, and each view
names the pipe at its creation. The chrome patterns are
`crates/ambre-chrome/src/context_menu.rs`, `dialog.rs`, `toast.rs`, `status.rs` and `format.rs`, and
the belt is `crates/ambre-chrome/tests/chrome_ui.rs`. The hidden-region style is
`crates/ambre-app/styles.css`.

---

## 3. SURFACE

| Trigger | Behaviour |
|---|---|
| Tab from the chrome | focus enters the content through the input pipe, in page order |
| the selection moves in the content | the focused unit is read out, and the view keeps drawing |
| an operation is refused | the message is read out and also raised as a toast |
| a toast appears | it is read out politely, and focus does not move |
| a menu opens | focus enters it, and arrows, Home, End and a letter move the highlight |
| Escape in a menu | the menu closes, and focus is left on the page body |
| a modal opens | focus is trapped inside it and the rest of the page is inert |
| an icon-only button is focused | its name is read out, and its pressed state with it |
| the zoom chip is focused | it is read out as a menu trigger, with its current factor |

**A board's tools are silent.** A board announces its selection. Switching a tool or opening an
editor is drawn and says nothing, which is `CHROME-029`.

---

## 4. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| The chrome is DOM, so its accessibility is the platform's | drawing the chrome, and rebuilding an accessibility layer under it | roles, names, focus order, screen-reader support and high-contrast handling are decades of work a canvas widget set has to rebuild and a browser already ships. Choosing DOM buys all of it and costs nothing on the frame path |
| Content accessibility is a DOM mirror, not an accessibility tree | pushing a tree to a platform adapter | on this target there is nothing to push to: on the web the accessibility tree is the DOM, and the library the corpus used to name for this has no web adapter at all. A "tree" would be reconstructed as hidden DOM anyway -- the mirror is the same destination reached directly, and it is the destination the code chose |
| The mirror is the focused unit alone | a complete hidden mirror of the document | it is the document rendered twice, once in pixels and once in DOM, with the second copy to keep in step on every edit and every scroll. What a screen-reader user needs as they navigate is what they just moved onto, which is one element |
| The focus carrier is the input pipe, not a second element | a second focusable element for accessibility | it would be focusable, silent and confusing. The element that receives the keyboard is the one a browser reports as focused, so making it also the accessible name of the content keeps one focus and one announcement |
| The live region is polite, and the modal is not | one severity for everything | a toast and a selection change are information, and interrupting a reader mid-sentence for them is worse than waiting. A trust decision or a destructive confirmation is different: it takes a modal, which stops everything, and `ambre-spec-key-verification.md` is where that rule is stated |
| The announcement is composed by the view, not by the shared crate | a shared formatter | it would have to enumerate every application's content model: the shared crate cannot know that a unit is a cell reference plus a value, a shape plus a label, or a paragraph plus a style. It owns the region and the write; the words are the view's |

---

## 5. NOT BUILT, AND NOT YET RULED

* **Nothing observes the write into the page.** The words and the names are unit-tested. The call
  that writes them is one line per view, and the browser tier that would read it back is the one
  `ambre-spec-testing.md section 8` describes. Reading it back is the `Each surface's name read back
  from the page` Story (`ambre-roadmap.md` E9).
* **Nothing states what the content owes beyond the focused unit.** A screen-reader user cannot ask
  "what is in this column". Only the focused unit exists in the page.
* **No test in any tier drives an assistive technology.** The wasm tier mounts a component and reads
  its attributes and its focus, which is what holds the chrome rows. Nothing observes what a screen
  reader actually says, and nothing walks the page for an unnamed control.
* **There is no keyboard route into the content on some paths.** Reaching the pipe requires it to
  be in the tab order, which it is. Nothing states what happens for a view whose content is not
  editable. A board with a shape selected has no text box to focus.
* **Reduced motion, contrast and text scaling are unruled.** The tokens carry one palette pair and
  the animations are unconditional, and no row reads the user's own preferences for either.

---

## 6. WHAT THIS DOCUMENT DOES NOT OWN

| Question | Owner |
|---|---|
| The components themselves: the shell, the menu, the modal, the toasts, the chips | `ambre-spec-chrome.md` |
| The element that carries content focus, and what it does with a keystroke | `ambre-spec-text-input.md` |
| What a narrow window changes about a component's shape | `ambre-spec-adaptive-layout.md` |
| The palette, the contrast values and the type sizes | `ambre-spec-design-tokens.md` |
| Why a key change is a modal and not a toast | `ambre-spec-key-verification.md` |
| What the canvas draws, and when | `ambre-spec-frame.md` |
| Which unit is focused, and what its value is | each application's feature documents |
| Prose's own reading order and its document structure | `4-Prose/features/ambre-spec-prose-accessibility.md` |
