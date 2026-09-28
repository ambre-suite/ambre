# AMBRE -- FEATURE: THE CHROME SHELL

**Type:** Horizontal (framework)
**Implementation:** partial -- the layout, the menu, the modal, the toasts, the command palette and
the shared controls ship. The one floating shell does not.

> Everything around the document: the frame the canvas sits in, the menus, the modals and the
> toasts. The status bar and the controls a formatting toolbar is built from are here too. It is
> DOM. It is written once, and every application dresses the same toolkit.

The chrome draws nothing and knows no application. It invokes a command and it shows what the Logic
thread reported. That is what keeps one toolkit serving a spreadsheet, a board and a page.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`. The prefix is `CHROME`, shared with
`ambre-spec-text-input.md`, `ambre-spec-accessibility.md`, `ambre-spec-adaptive-layout.md`,
`ambre-spec-launcher.md` and `ambre-spec-key-verification.md`: one numbering over the six documents
the DOM side produces.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `CHROME-001` | `INV` | The chrome MUST NOT draw content nor hit-test it. | `ambre-spec-ui.md section 2` (`UI-THREAD-001`, `UI-THREAD-050`) | crate-graph -- `ambre-chrome` declares neither `wgpu` nor `ambre-draw`, so no drawing surface is reachable from it. What it does instead (invoke a command, read a signal) is `UI-THREAD-050`, and is not restated here | none | verified |
| `CHROME-002` | `INV` | The shared toolkit MUST NOT depend on an application crate. | this spec section 2 | crate-graph -- the manifest names `ambre-commands`, `leptos`, the two icon crates and the DOM bindings, and no application | none | verified |
| `CHROME-009` | `INV` | The content surface MUST be a cell of the chrome layout, so it is sized by the layout and never by the viewport. | this spec section 2 | `ambre_chrome::chrome_ui::app_shell_owns_the_named_canvas_cell` -- wasm tier, real `document` | none | verified |
| `CHROME-004` | `INV` | Every popover and floating bar MUST take its placement, flip and keyboard model from one shell. | this spec section 2 | not built | unbuilt -- lands with the `One floating-menu shell` Story (`ambre-roadmap.md` E8). Four shells exist: the shared menu, Trame's filter popover, Trame's find panel and Toile's selection toolbar | unimplemented |
| `CHROME-008` | `INV` | The command palette MUST search every command the active application declares, not the subset some menu happens to show. | this spec section 2; `ambre-spec-commands.md` | `ambre_chrome::palette::tests::an_empty_query_lists_every_declared_command`; `::a_query_matches_any_case_and_any_position`; `::a_query_that_answers_nothing_lists_nothing`; `::rows_keep_the_declaration_order`; `ambre_chrome::chrome_ui::palette_lists_every_declared_command` -- the DOM belt, all five over a table the test declares | partial -- the filter is held and the feed is not: `crates/ambre-app/src/shell/app_root.rs` passes the application's whole table today, and passing a menu's subset instead would leave every one of those assertions green; lands with the `The UI-thread belts` Story (`ambre-roadmap.md` E9) | partial |

### 1.2 A floating menu is placed

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `CHROME-003` | `POST` | A floating menu MUST stay inside the viewport: it flips at an edge, and pins to the origin when it is larger than the viewport. | this spec section 2 | `ambre_chrome::context_menu::tests::right_overflow_flips_horizontally`; `::bottom_overflow_flips_vertically`; `::corner_overflow_flips_both_axes` -- each flip; `::menu_larger_than_viewport_pins_to_origin` -- the clamp that would otherwise place it off-screen, over an arithmetic held in a free function, which is why it is testable at all | none | verified |
| `CHROME-085` | `FRAME` | Placing a menu that already fits MUST NOT move it off the point it was raised from. | this spec section 2 | `ambre_chrome::context_menu::tests::no_overflow_keeps_click_point` -- the placement returns the click point unchanged, which is what keeps the flip from becoming an unconditional offset | none | verified |

### 1.3 A key event is resolved

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `CHROME-005` | `FAIL` | A key event MUST resolve to a modelled key or be rejected; an unmodelled key MUST NOT reach a command. | this spec section 2; `ambre-spec-commands.md` | `ambre_chrome::command::tests::named_keys_map`; `::function_keys_in_range`; `::single_printable_char_maps` -- what resolves; `::function_keys_out_of_range_rejected`; `::unmodelled_keys_rejected` -- the refusal, which is the half that keeps an unknown key from binding a command by accident | none | verified |

### 1.4 A control renders, and reports a click

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `CHROME-010` | `POST` | A control MUST show the state the Logic thread reported. | this spec section 2; `ambre-spec-commands.md` | `ambre_chrome::chrome_ui::command_states_defaults`; `::command_states_single_writer_propagates`; `::command_states_unknown_id_is_noop` -- one writer reaches the right signal and leaves its neighbours alone | none | verified |
| `CHROME-013` | `POST` | A shared formatting control MUST report a change. | this spec section 2 | `ambre_chrome::chrome_ui::format_toggle_reflects_pressed_and_reports_clicks` -- a real click is dispatched and the callback fires once; `::font_size_select_lists_presets_plus_auto_entry` -- the same shape for a select | none | verified |
| `CHROME-014` | `POST` | A status-bar chip MUST start collapsed. | this spec section 2 | `ambre_chrome::chrome_ui::zoom_chip_renders_readout_from_signal_and_starts_collapsed`; `::perf_chip_renders_fps_from_signal_and_starts_collapsed` -- the menu element is absent on mount | none | verified |
| `CHROME-086` | `POST` | A control MUST follow its signal when the value changes, not only when it mounts. | this spec section 2 | none | unheld -- both chip tests read the value off the first synchronous render and say so in their own comments: a re-render lands on a later microtask, so a post-change query would race and none is written. What is asserted is the mount, and the live half is the one a stale readout breaks; lands with the `The UI-thread belts` Story (`ambre-roadmap.md` E9) | untested |
| `CHROME-087` | `FRAME` | A control MUST NOT change the state it displays: the click leaves as a report and the value comes back from the Logic thread. | this spec section 2; `ambre-spec-commands.md` | none | unheld -- the toggle test sets the pressed signal itself before dispatching the click, so nothing would notice a control that also wrote it, and the store has one writer by construction with no test driving a second. This is the row that keeps a toolbar from showing a change the document refused; lands with the `The UI-thread belts` Story (`ambre-roadmap.md` E9) | untested |

### 1.5 A modal opens, and closes

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `CHROME-011` | `POST` | A modal MUST suspend the frame loop while it is open. | this spec section 2; `ambre-spec-frame.md`, `CANVAS-012` | none | unheld -- three modals pass a pause callback and the launcher's own confirmation dialog passes none. Nothing joins opening a modal to the state of the loop; lands with the `The UI-thread belts` Story (`ambre-roadmap.md` E9) | untested |
| `CHROME-012` | `POST` | Setting a modal's open signal MUST show the element. | this spec section 2 | none | unheld -- one effect drives the element from the signal, and no test mounts a modal to watch it open; lands with the `The UI-thread belts` Story (`ambre-roadmap.md` E9) | untested |
| `CHROME-091` | `POST` | Clearing a modal's open signal MUST hide the element. | this spec section 2 | none | unheld -- the same effect carries the closing arm, and no test mounts a modal to watch it close; lands with the `The UI-thread belts` Story (`ambre-roadmap.md` E9) | untested |
| `CHROME-088` | `POST` | Escape and a click on the backdrop MUST close a modal. | this spec section 2 | none | unheld -- two handlers drive the dismissal from the element, and no test mounts one; lands with the `The UI-thread belts` Story (`ambre-roadmap.md` E9) | untested |
| `CHROME-092` | `POST` | A dismissed modal MUST clear its signal. | this spec section 2 | none | unheld -- a regression leaves a closed modal whose signal still reads open, which blocks the next open: the failure is not the dismissal, it is the one after; lands with the `The UI-thread belts` Story (`ambre-roadmap.md` E9) | untested |
| `CHROME-089` | `FRAME` | Closing a modal MUST NOT leave the frame loop suspended. | this spec section 2; `ambre-spec-frame.md`, `CANVAS-012` | none | unheld -- and this is the dangerous half: a missing pause costs a few frames of work, and a missing resume leaves the surface frozen with the interface answering normally around it. The resume callback is passed by the same three modals that pass the pause, and by no one else; lands with the `The UI-thread belts` Story (`ambre-roadmap.md` E9) | untested |

### 1.6 A drag drives a control

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `CHROME-015` | `POST` | A control driven by a drag MUST coalesce the burst into one command. | this spec section 2 | none | unheld -- a debouncer reschedules its timer on each call, and each caller decides whether to use it. Nothing refuses a slider wired straight to a command, which is one command per pixel of travel; lands with the `The UI-thread belts` Story (`ambre-roadmap.md` E9) | untested |
| `CHROME-090` | `FRAME` | Coalescing a burst MUST NOT drop its last value. | this spec section 2 | none | unheld -- the debouncer keeps the most recent call and fires it when the timer runs out, so the rule is held by the reschedule being written that way, and nothing drives a burst that ends before the delay. Dropping the tail leaves the control showing one value and the document holding another; lands with the `The UI-thread belts` Story (`ambre-roadmap.md` E9) | untested |
* **Budget.** none: the chrome renders in the DOM, off the frame loop, and
  `ambre-spec-budgets.md` budgets no chrome operation. What it does spend on the frame is the pause
  of `CHROME-011`, which saves rather than costs.
* **Boundary.** none: `UI-THREAD-004` and `UI-THREAD-049` ban a network, crypto and persistence
  path on this thread, and the toolkit adds no exception. Its manifest names no transport and no
  crypto crate, which is the crate-graph fact `CHROME-001` and `CHROME-002` are read off.
* **Verification.** 9 `verified`, 1 `partial`, 10 `untested`, 1 `unimplemented`. What holds is a crate-graph
  fact, placement arithmetic, or a component mounted in a headless document. Each asserts a first
  render. What is loose is everything after that mount. A control writes its own state, a chip
  re-renders live, a resume is missing, and a signal is left open.
---

## 2. MODEL

**CRDT containers.** none. The chrome holds view state and the display payload the Logic thread
pushed; every mutation leaves it as a command (`UI-THREAD-020`).

**The layout.** One CSS grid, five rows, each a slot the application fills. The toolkit lays out
whatever it is handed and names no application.

| Row | Holds | Optional |
|---|---|---|
| header | the title, the sync state, the share action, the account | no |
| command bar | the contextual toolbar the application builds from its commands | no |
| formula bar | a reference box and a content input | yes |
| content | the canvas element, plus the input pipe of `ambre-spec-text-input.md` | no |
| tab strip | a band of document tabs, such as a spreadsheet's sheets | yes |
| status bar | the caption, the counters, the zoom chip and the frame chip | no |

The content row owns the canvas element, so the drawing buffer is sized to a grid cell rather than
to the window (`CHROME-009`). An absent optional row collapses and costs no height.

**What the toolkit ships.**

| Component | Behaviour | State |
|---|---|---|
| the shell | the grid above, and the canvas cell | built |
| the floating menu | placement, flip, roving focus, first-letter typeahead, Escape and click-outside | built |
| the modal | the native dialog element, plus the pause and resume of the frame loop | built |
| the toasts | four severities, a dwell per severity, dismiss on click, one live region | built |
| the status chips | the caption, the zoom read-out with its preset menu, the frame read-out with a panel the application supplies | built |
| the formatting controls | a toggle, a size select, a family picker, a colour popover, and a debouncer for a drag | built |
| the keybinding half | a key event to a modelled chord, for a keymap to resolve | built |
| the shortcut chip | one chord, rendered the same way in a menu, a tooltip and a palette | built |
| the command palette | one overlay listing every command the application declares, filtered by a substring of the label | built |
| the sidebar and the panels | a resizable region beside the content | design |

**The command path, and the one direction it runs.** A trigger names a command, and the command
reaches the Logic thread. The mutation comes back as an event, the dispatcher writes the store, and
the control redraws. The chrome never shortens that loop by writing the store itself
(`CHROME-010`).

1. a click, a chord or a menu entry invokes a command identifier;
2. the launcher maps it to a command for the Logic thread, and sends it;
3. the mutation returns as an event the dispatcher drains;
4. the dispatcher writes the toggle and enabled signals -- it is their single writer;
5. every view of that command redraws, whichever trigger started it.

The identifier, the keymap and the registry that will describe a command once are
`ambre-spec-commands.md`, and are not restated here.

**The floating menu, and the arithmetic under it.** Placement is a pure function of the click point,
the menu size and the viewport size. Each axis that would overflow anchors at `point - size`,
clamped to zero. The DOM read stays in the effect and the decision is a host-tier function, which is
why `CHROME-003` has five tests and no browser.

**The modal.** The native dialog element supplies the focus trap and the inertness of the page
behind it, along with Escape and the backdrop. The component adds two things and no more. It keeps
the open signal and the element agreeing in both directions (`CHROME-012`). It also fires the
callbacks that pause and resume the frame loop (`CHROME-011`). A blur over a frozen canvas then costs nothing.

**The application's own chrome.** An application builds its specific components on this toolkit, in
its own crate. Those are a formula bar, a filter popover, a find panel, a selection toolbar and a
style inspector. The rule is one line, shared upstream and specific in the application crate. The crate
graph holds it (`CHROME-002`).

**Code:** `crates/ambre-chrome/src/app_shell.rs` holds the grid, and
`crates/ambre-chrome/src/context_menu.rs` the menu and its placement function.
`crates/ambre-chrome/src/palette.rs` holds the command palette and its filter,
`crates/ambre-chrome/src/dialog.rs` the modal, `crates/ambre-chrome/src/toast.rs` the toasts,
and `crates/ambre-chrome/src/status.rs` the chips. `crates/ambre-chrome/src/format.rs`,
`font.rs` and `color.rs` hold the formatting controls, `crates/ambre-chrome/src/debounce.rs` the
debouncer, and `crates/ambre-chrome/src/command.rs` the store and the key translation. The wiring
that mounts them is `crates/ambre-app/src/shell/app_root.rs`, and the belt is
`crates/ambre-chrome/tests/chrome_ui.rs`.

---

## 3. SURFACE

| Trigger | Behaviour |
|---|---|
| a right-click on the content | the menu opens under the pointer, flipped at an edge |
| an arrow key in an open menu | the highlight moves and the DOM focus follows it |
| a letter typed in an open menu | the highlight jumps to the next entry starting with it |
| Escape, or a click outside an open menu | the menu closes, and the click still reaches what is under it |
| a menu entry, a toolbar button or a chord | the same command runs, whichever of the three raised it |
| a modal opens | the page behind it goes inert and the canvas freezes on its last frame |
| Escape or a backdrop click on a modal | the modal closes and the canvas resumes |
| an action reports something | a toast appears, and dismisses itself after four seconds, seven for an error |
| a click on a toast | it dismisses immediately |
| a click on the zoom chip | the preset ladder opens, and the current factor is marked |
| a click on the frame chip | the panel the application supplied unfolds under it |
| a formatting control is used | it reports the change, and it redraws once the document confirms it |

**A command in no menu is reachable through the palette, and only there.** Ctrl+K opens the palette
in both applications. It lists the whole table of the booted application, and a substring of the
label narrows the list (`CHROME-008`). The palette is why every command carries a label
(`ambre-spec-commands.md`, `UI-THREAD-019`).

---

## 4. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| The chrome is DOM, and the content is not | a canvas widget framework | it is roughly two years of work to reach parity with an existing toolkit, and it still has to rebuild a hidden DOM tree for accessibility. A menu, a modal and a toolbar are what a browser is good at -- focus, tab order, screen readers, text selection and input methods all come free and correct, and none of them is on a frame path. Recomposing a rasterized DOM layer beside a canvas costs the compositor nothing per frame, so the argument for the canvas was never a performance one |
| A modal is the native dialog element | a component that traps focus and paints its own backdrop | the focus trap, the inertness of the rest of the page, Escape and the backdrop are four behaviours that are hard to get right and already correct in every browser. Reimplementing them buys a styling hook and costs four bugs |
| A menu is placed by arithmetic, not by the browser | CSS anchor positioning | it is too recent to rely on across browsers, for a decision that is two comparisons and a clamp |
| A menu is placed by arithmetic, not by the browser | a positioning library | it takes a dependency for those same two comparisons, and it takes the placement out of a pure function -- which is what makes this the one piece of floating-menu behaviour with a test |
| The toggle store has one writer | optimistic local state in each control, reconciled on the event | a control that flips itself on click is right until the command fails, is refused, or is undone from elsewhere -- and then it lies, silently, until something else repaints it. One writer means the interface can only show what the document actually holds |
| The contextual bar, never a ribbon | an office-style ribbon as the default | a ribbon is dense, adapts badly to a narrow window and is hostile on a phone. One bar showing what the selection can do carries the same commands in less space, and a ribbon remains generable from the registry later as an option |
| The shared toolkit carries only what two applications share | every application's chrome in the shared crate | a component in the shared crate that one application uses is a dependency inversion with no payoff, and it is how a shared crate learns the name of an application. The line is the crate graph, which the compiler holds |
| A drag is debounced by the caller, not by the command | debouncing inside the command dispatch | a slider and a text field want different quiet periods, and a command that debounced itself would have to guess one. The debouncer is a value a component keeps, which is why the rule is about the caller and not about the command |
| The frame chip is shared and its panel is not | a per-application status bar built from scratch | each application would invent its own read-out for a frame rate that reads the same everywhere. The timings behind it are per renderer, so the chip is one component and the panel is a slot |

---

## 5. NOT BUILT, AND NOT YET RULED

* **Four floating shells exist and `CHROME-004` names one.** The shared menu has the placement
  arithmetic and the keyboard model. Trame's filter popover, Trame's find panel and Toile's
  selection toolbar each have their own, and reuse only the value controls. The three keyboard
  models are not the same model.
* **Closing a menu does not restore focus.** The roving tabindex moves the DOM focus into the menu
  and nothing puts it back on Escape. A keyboard user therefore lands on the document body. No row
  covers it, and `ambre-spec-accessibility.md` is where the rule would go.
* **The launcher's confirmation dialog is not the shared modal.** It opens the element directly, so
  it gets the focus trap and misses the pause of `CHROME-011`. It is harmless where it sits, because
  the loop is already paused behind the launcher. It is the counter-example that shows nothing
  enforces the rule.
* **The palette offers a command the document cannot run.** The command store is `Rc` and stays in
  the mount owner, outside the `Send` router subtree. The overlay cannot subscribe to the store, so
  it lists Undo on an empty history. A layout omits what does not apply, which is how a menu hides
  the same gap.
* **Nothing rules on what the palette does beyond the label.** The palette matches a substring of
  the label, in declaration order. It searches neither a command's category nor its identity, and it
  keeps no list of recent commands. No rule asks for any of the three.
* **The sidebar and the resizable panels have no code.** The layout has no row for them, so an
  application that wants one has nowhere to put it.
* **Nothing bounds the toasts.** They stack without limit and each one holds its own timer, so a
  burst of errors covers the interface for seven seconds.
* **A menu entry carries its label, so a command in two menus has two.** That is `UI-THREAD-019`,
  and it is stated there rather than here.

---

## 6. WHAT THIS DOCUMENT DOES NOT OWN

| Question | Owner |
|---|---|
| What the canvas and the DOM each own, and the two queues between them | `ambre-spec-ui.md section 2` |
| The command identifier, the keymap and the registry a command is described in | `ambre-spec-commands.md` |
| Where the design tokens come from, and how a theme switch reaches both sides | `ambre-spec-design-tokens.md` |
| What a copy and a paste carry | `ambre-spec-clipboard.md` |
| The keyboard, the input method and the clipboard events over the canvas | `ambre-spec-text-input.md` |
| The roving focus, the live regions and what a screen reader is told | `ambre-spec-accessibility.md` |
| What a narrow window changes, and where the breakpoints are | `ambre-spec-adaptive-layout.md` |
| The home screen, the sign-in panel and the document list | `ambre-spec-launcher.md` |
| The share dialog's trust step, and the fingerprint it shows | `ambre-spec-key-verification.md` |
| When a frame runs, and what a paused loop holds | `ambre-spec-frame.md` |
| How a command reaches the Logic thread, and what comes back | `ambre-spec-architecture.md section 2` |
