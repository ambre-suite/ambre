# AMBRE -- FEATURE: THE CHROME SHELL

**Type:** Horizontal (framework)

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
`ambre-spec-launcher.md`, `ambre-spec-key-verification.md` and `ambre-spec-shell.md`: one
numbering over the seven documents the DOM side produces.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `CHROME-001` | `INV` | The chrome MUST NOT draw content nor hit-test it. | `ambre-spec-ui.md section 2` (`UI-THREAD-001`, `UI-THREAD-050`) | crate-graph -- `ambre-chrome` declares neither `wgpu` nor `ambre-draw`, so no drawing surface is reachable from it. What it does instead (invoke a command, read a signal) is `UI-THREAD-050`, and is not restated here | none | verified |
| `CHROME-002` | `INV` | The shared toolkit MUST NOT depend on an application crate. | this spec section 2 | crate-graph -- the manifest names `ambre-commands`, `leptos`, the two icon crates and the DOM bindings, and no application | none | verified |
| `CHROME-009` | `INV` | The content surface MUST be a cell of the chrome layout, so it is sized by the layout and never by the viewport. | this spec section 2 | `ambre_chrome::chrome_ui::app_shell_owns_the_named_canvas_cell` -- wasm tier, real `document` | none | verified |
| `CHROME-004` | `INV` | Every popover and floating bar MUST take its placement, flip and keyboard model from one shell. | this spec section 2 | not built | unbuilt -- lands with the `One floating-menu shell` Story (`ambre-roadmap.md` E8). Four shells exist: the shared menu, Trame's filter popover, Trame's find panel and Toile's selection toolbar | unimplemented |
| `CHROME-008` | `INV` | The command palette MUST search every command the active application declares, not the subset some menu happens to show. | this spec section 2; `ambre-spec-commands.md` | `ambre_chrome::palette::tests::an_empty_query_lists_every_declared_command`; `::a_query_matches_any_case_and_any_position`; `::a_query_that_answers_nothing_lists_nothing`; `::rows_keep_the_declaration_order`; `ambre_chrome::chrome_ui::palette_lists_every_declared_command` -- the DOM belt, all five over a table the test declares; `ambre_app::shell::app_root::tests::the_palette_is_fed_the_whole_declared_table` -- the feed, per kind | none | verified |
| `CHROME-093` | `INV` | The palette MUST list the entries an application supplies before the commands it declares. | this spec section 2 | `ambre_chrome::palette::tests::the_entries_of_the_application_come_before_the_commands` -- the pure rows, one query over both sides; `ambre_chrome::chrome_ui::palette_entry_comes_first_and_hands_back_its_key` -- the DOM belt: the entry is the first row, and choosing it hands its key back | none | verified |
| `CHROME-094` | `INV` | A palette query that starts with `>` MUST list the commands alone. | this spec section 2; `CHROME-008` | `ambre_chrome::palette::tests::a_query_that_starts_with_a_chevron_lists_the_commands_alone` -- the bare chevron and one followed by a query, over entries that would match it | none | verified |
| `CHROME-114` | `INV` | The command palette MUST be operable from the keyboard: the arrows move the highlight and wrap, Enter runs the highlighted row, and Escape closes it. | this spec section 2; `CHROME-008` | `ambre_chrome::chrome_ui::the_palette_is_operable_by_keyboard_alone` -- keys raised on the query: each arrow wraps at its end, Enter runs the highlighted command and closes the palette, and Escape closes it with nothing run | none | verified |
| `CHROME-115` | `INV` | Each opening of the command palette MUST start from a blank query, on the first row, with the caret in the query. | this spec section 2; `CHROME-008` | `ambre_chrome::chrome_ui::a_reopened_palette_starts_blank_on_the_first_row` -- a query typed and a row highlighted, then a close and an open: the query reads empty, every row is back, the first is highlighted and the query holds the focus | none | verified |
| `CHROME-140` | `INV` | Each application MUST lay out only the optional rows it declares, and an absent row MUST render no element. | this spec section 2 | `ambre_chrome::chrome_ui::an_absent_row_renders_no_element_and_a_shown_row_keeps_its_content` -- wasm tier, a formula bar, a tab strip and a command bar marked absent leave no element; `ambre_app::shell::app_root::tests::the_formula_bar_is_trames_alone_and_prose_has_no_command_bar` -- the rows each application declares | none | verified |
| `CHROME-141` | `INV` | A bar of the chrome MUST NOT cover the header. | this spec section 2 | `ambre_canvas::theme::stylesheet_tests::no_fixed_surface_covers_the_header` -- every fixed surface of the shell sheet that sets its top sits under the header or is centred, the Toile groups set no position, and the tooltip anchor of the command bar leaves its floating menu fixed | none | verified |
| `CHROME-142` | `INV` | Every surface of the chrome MUST render in the chrome family, a surface mounted outside the shell included. | this spec section 2 | `ambre_canvas::theme::stylesheet_tests::every_family_the_chrome_names_is_the_chrome_family_or_a_monospace_one` -- the body carries the chrome family, and no declaration names a family the page does not load | none | verified |
| `CHROME-143` | `OBS` | The format bar MUST show the frame of a floating bar, and its controls MUST NOT show the native button of the browser. | this spec section 2; `CHROME-105` | `ambre_canvas::theme::stylesheet_tests::the_format_bar_has_the_frame_of_a_floating_bar` -- the bar sets a ground, a border, a radius and a shadow, and a control drops the native look | none | verified |
| `CHROME-144` | `OBS` | The header of a document page MUST name the open document. | this spec section 2; `CHROME-080` | `ambre_app::shell::app_root::tests::the_header_links_home_and_names_the_open_document` -- wasm tier, the name the list holds shows once a document is open, and no name shows without one | none | verified |
| `CHROME-147` | `OBS` | An application that zooms MUST show its zoom in the status bar, at the end of the band, and in no other bar. | this spec section 2; `ambre-spec-zoom.md` | `ambre_toile::tests::chrome_ui::the_zoom_chip_closes_the_status_bar` -- wasm tier, the board band ends on the zoom chip; `ambre_trame::tests::chrome_ui::the_zoom_chip_closes_the_status_bar` -- the same read over the grid band; `ambre_app::host::prose::tests::the_zoom_chip_closes_the_band_and_zooms_the_page` -- the same read over the page band; `ambre_toile::tests::chrome_ui::the_view_menu_reports_zoom_in_and_the_bar_holds_no_zoom_control` -- the menu bar holds no zoom control | none | verified |

### 1.2 A floating menu is placed

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `CHROME-003` | `POST` | A floating menu MUST stay inside the viewport: it flips at an edge, and pins to the origin when it is larger than the viewport. | this spec section 2 | `ambre_chrome::context_menu::tests::right_overflow_flips_horizontally`; `::bottom_overflow_flips_vertically`; `::corner_overflow_flips_both_axes` -- each flip; `::menu_larger_than_viewport_pins_to_origin` -- the clamp that would otherwise place it off-screen, over an arithmetic held in a free function, which is why it is testable at all | none | verified |
| `CHROME-085` | `FRAME` | Placing a menu that already fits MUST NOT move it off the point it was raised from. | this spec section 2 | `ambre_chrome::context_menu::tests::no_overflow_keeps_click_point` -- the placement returns the click point unchanged, which is what keeps the flip from becoming an unconditional offset | none | verified |
| `CHROME-111` | `POST` | A press outside an open floating menu MUST close it and run nothing. A press inside it MUST NOT close it. | this spec section 2 | `ambre_chrome::chrome_ui::a_press_outside_closes_the_floating_menu_and_one_inside_does_not` -- a press raised on a row keeps the menu, and one raised on a control beside it closes the menu with no command run | none | verified |
| `CHROME-112` | `FRAME` | A disabled menu row MUST NOT run its command, from the keyboard or from the pointer. | this spec section 2 | `ambre_chrome::chrome_ui::a_disabled_row_runs_nothing_from_the_keyboard_or_the_pointer` -- Enter, Space and a click on the disabled row run nothing, and the menu stays open | none | verified |
| `CHROME-113` | `FAIL` | A press after a floating menu is unmounted MUST NOT throw. | this spec section 2 | `ambre_chrome::chrome_ui::a_press_after_the_floating_menu_is_unmounted_throws_nothing` -- regression: the menu is unmounted and its owner disposed, as a `Show` hides its children, then a press on the page raises no error | none | verified |

### 1.3 A key event is resolved

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `CHROME-005` | `FAIL` | A key event MUST resolve to a modelled key or be rejected; an unmodelled key MUST NOT reach a command. | this spec section 2; `ambre-spec-commands.md` | `ambre_commands::keybinding::tests::named_keys_map`; `::function_keys_in_range`; `::single_printable_char_maps` -- what resolves; `::function_keys_out_of_range_rejected`; `::unmodelled_keys_rejected` -- the refusal, which is the half that keeps an unknown key from binding a command by accident | none | verified |

### 1.4 A control renders, and reports a click

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `CHROME-010` | `POST` | A control MUST show the state the Logic thread reported. | this spec section 2; `ambre-spec-commands.md` | `ambre_chrome::chrome_ui::command_states_defaults`; `::command_states_single_writer_propagates`; `::command_states_unknown_id_is_noop` -- one writer reaches the right signal and leaves its neighbours alone | none | verified |
| `CHROME-013` | `POST` | A shared formatting control MUST report a change. | this spec section 2 | `ambre_chrome::chrome_ui::format_toggle_reflects_pressed_and_reports_clicks` -- a real click is dispatched and the callback fires once; `::font_size_select_lists_presets_plus_auto_entry` -- the same shape for a select | none | verified |
| `CHROME-109` | `POST` | A colour picked with an alpha, as `#rrggbbaa` spells it, MUST carry that alpha into a colour that holds one. | this spec section 2 | `ambre_chrome::color::tests::a_picked_alpha_reaches_the_colour_that_holds_one` -- the alpha reaches a colour with an alpha, set or unset, and a colour with none drops it; `::parse_hex_reads_an_eight_digit_spelling_with_its_alpha` | none | verified |
| `CHROME-110` | `POST` | A colour picked without an alpha, as a swatch or `#rrggbb` spells it, MUST keep the alpha of the colour it replaces. | this spec section 2 | `ambre_chrome::color::tests::a_repaint_keeps_the_alpha_of_the_current_colour` -- a translucent colour keeps its alpha, an unset one reads as opaque | none | verified |
| `CHROME-105` | `POST` | The format bar MUST hang above the anchor it is handed, show each control with its pressed state, and report a press of that control alone. | this spec section 2; `CHROME-013` | `ambre_chrome::chrome_ui::the_format_bar_hangs_its_controls_and_reports_a_press` -- the placement, the three pressed states, and one press reported | none | verified |
| `CHROME-014` | `POST` | A status-bar chip MUST start collapsed. | this spec section 2 | `ambre_chrome::chrome_ui::zoom_chip_renders_readout_from_signal_and_starts_collapsed`; `::perf_chip_renders_fps_from_signal_and_starts_collapsed` -- the menu element is absent on mount | none | verified |
| `CHROME-086` | `POST` | A control MUST follow its signal when the value changes, not only when it mounts. | this spec section 2 | `ambre_chrome::chrome_ui::the_zoom_readout_follows_its_signal`; `::the_fps_readout_follows_its_signal` -- each changes the signal after the mount and reads the DOM after a reactive tick | none | verified |
| `CHROME-087` | `FRAME` | A control MUST NOT change the state it displays: the click leaves as a report and the value comes back from the Logic thread. | this spec section 2; `ambre-spec-commands.md` | `ambre_chrome::chrome_ui::a_toggle_reports_its_click_and_waits_for_the_value` -- the click is reported and the pressed state stays put until the test writes the answer | none | verified |

### 1.5 A modal opens, and closes

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `CHROME-011` | `POST` | A modal MUST suspend the frame loop while it is open. | this spec section 2; `ambre-spec-frame.md`, `CANVAS-012` | `ambre_chrome::chrome_ui::a_modal_follows_its_signal_and_holds_the_loop_while_open` -- the shared modal takes the shell's hold on open; ci-gate -- the rustc driver of `crates/ambre-primitives`, run by `just primitives` in the CI `lint` job, reads each path once it resolves, so an alias, a re-export and a macro name what they reach. It refuses the `<dialog>` element of the toolkit, the browser's dialog element and a `create_element("dialog")` outside `crates/ambre-chrome/src/dialog.rs`, so no modal opens without it (`ambre_primitives::driver::chrome::a_modal_a_macro_writes_or_concat_spells_is_refused_outside_the_shared_dialog`, `ambre_primitives::driver::chrome::a_static_subtree_the_toolkit_writes_as_html_is_read_where_the_inert_element_takes_it`); `ambre_belt::frame::tests::a_modal_stops_the_frames_and_escape_gives_them_back` -- the browser tier: the palette opened by its chord, and the page issues no draw call for a second. A page script installed before boot counts the WebGL draw calls | none | verified |
| `CHROME-012` | `POST` | Setting a modal's open signal MUST show the element. | this spec section 2 | `ambre_chrome::chrome_ui::a_modal_follows_its_signal_and_holds_the_loop_while_open` | none | verified |
| `CHROME-091` | `POST` | Clearing a modal's open signal MUST hide the element. | this spec section 2 | `ambre_chrome::chrome_ui::a_modal_follows_its_signal_and_holds_the_loop_while_open` | none | verified |
| `CHROME-088` | `POST` | Escape and a click on the backdrop MUST close a modal. | this spec section 2 | `ambre_chrome::chrome_ui::a_backdrop_click_dismisses_the_modal_and_clears_its_signal`; `::a_native_close_clears_the_signal_and_the_modal_reopens` -- Escape is the browser's own close, which the second test drives; `ambre_belt::frame::tests::a_modal_stops_the_frames_and_escape_gives_them_back` -- the browser tier: a trusted Escape from the driver closes the palette modal in the whole application | none | verified |
| `CHROME-092` | `POST` | A dismissed modal MUST clear its signal. | this spec section 2 | `ambre_chrome::chrome_ui::a_backdrop_click_dismisses_the_modal_and_clears_its_signal`; `::a_native_close_clears_the_signal_and_the_modal_reopens` -- the second opens the modal again after the dismissal | none | verified |
| `CHROME-106` | `FRAME` | A close that lands after its modal opened again MUST NOT close it, clear its signal, give the frame loop back or notify its caller. | this spec section 2; `CHROME-092` | `ambre_chrome::chrome_ui::a_close_landing_after_a_reopen_leaves_the_modal_open` -- the signal is cleared and set again before the queued event lands | none | verified |
| `CHROME-089` | `FRAME` | Closing a modal MUST NOT leave the frame loop suspended. | this spec section 2; `ambre-spec-frame.md`, `CANVAS-012` | `ambre_chrome::chrome_ui::a_modal_follows_its_signal_and_holds_the_loop_while_open`; `::a_backdrop_click_dismisses_the_modal_and_clears_its_signal`; `::an_open_modal_unmounted_releases_the_loop` -- each close path returns the hold; `ambre_canvas::frame::tests::a_closed_modal_resumes_only_what_it_suspended` -- a release keeps the loop paused while the route is away; `ambre_belt::frame::tests::a_modal_stops_the_frames_and_escape_gives_them_back` -- the browser tier: the draw calls resume once a real Escape closes the palette | none | verified |

### 1.6 A drag drives a control

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `CHROME-015` | `POST` | A control driven by a drag MUST coalesce the burst into one command. | this spec section 2 | `ambre_chrome::chrome_ui::a_burst_runs_once_with_its_last_value`; ci-gate -- the rustc driver of `crates/ambre-primitives`, run by `just primitives` in the CI `lint` job, reads each path once it resolves, so an alias, a re-export and a macro name what they reach. It refuses a range or colour input type, read where the attribute call takes it, in a UI-thread file that names no `Debouncer` (`ambre_primitives::driver::chrome::a_drag_control_a_macro_writes_is_refused_in_a_file_with_no_debouncer`, `ambre_primitives::driver::chrome::a_static_subtree_the_toolkit_writes_as_html_is_read_where_the_inert_element_takes_it`) | none | verified |
| `CHROME-090` | `FRAME` | Coalescing a burst MUST NOT drop its last value. | this spec section 2 | `ambre_chrome::chrome_ui::a_burst_runs_once_with_its_last_value` | none | verified |

### 1.7 Go back home

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `CHROME-145` | `POST` | A click on the way home in the header MUST move the address bar to the home screen, from any page. | this spec section 2; `ambre-spec-launcher.md`, `CHROME-042` | `ambre_app::shell::app_root::tests::the_header_links_home_and_names_the_open_document` -- wasm tier, a click from a document address leaves the address bar on `/`, which the frame loop reads | none | verified |
| `CHROME-146` | `FRAME` | Going home from the header MUST NOT reload the page. | this spec section 2; `ambre-spec-launcher.md`, `CHROME-042` | `ambre_app::shell::app_root::tests::the_header_links_home_and_names_the_open_document` -- wasm tier, the header mounted before the click is still in the page after it, which a reload would remove | none | verified |

* **Budget.** none: the chrome renders in the DOM, off the frame loop, and
  `ambre-spec-budgets.md` budgets no chrome operation. What it does spend on the frame is the pause
  of `CHROME-011`, which saves rather than costs.
* **Boundary.** none: `UI-THREAD-004` and `UI-THREAD-049` ban a network, crypto and persistence
  path on this thread, and the toolkit adds no exception. Its manifest names no transport and no
  crypto crate, which is the crate-graph fact `CHROME-001` and `CHROME-002` are read off.
* **Verification.** 39 `verified`, 1 `unimplemented`. What holds is a crate-graph fact, placement
  arithmetic, or a component mounted in a headless document and driven past its mount. The modal
  rows rest on one shared element, which a gate keeps the only one, and on a hold the loop counts.
  The drag rows rest on one debouncer, which a gate keeps every drag control behind.
---

## 2. MODEL

**CRDT containers.** none. The chrome holds view state and the display payload the Logic thread
pushed; every mutation leaves it as a command (`UI-THREAD-020`).

**The layout.** One CSS grid, five rows, each a slot the application fills. The toolkit lays out
whatever it is handed and names no application.

| Row | Holds | Optional |
|---|---|---|
| header | the Ambre menu with the way home, the name of the open document, the sync state, the share action, the avatar menu | no |
| command bar | the channel header of Murmure; Prose, Trame and Toile lay out none (`CHROME-159`) | yes |
| formula bar | a reference box and a content input | yes |
| content | one canvas element, or several side by side, plus the input pipe of `ambre-spec-text-input.md` | no |
| tab strip | a band of document tabs, such as a spreadsheet's sheets | yes |
| status bar | the caption, the counters, the frame chip and, at its end, the zoom chip | yes |

The content row owns the canvas element, so the drawing buffer is sized to a grid cell rather than
to the window (`CHROME-009`). A view that shows several lists, such as a channel and a thread in
Murmure, puts one canvas in each cell of the row. Each canvas is sized by its own cell.

Each application declares the optional rows it lays out, and an absent row renders no element
(`CHROME-140`). The formula bar is Trame's alone. No application lays out the tab strip: Trame's
sheet tabs sit in the status bar (`CHROME-162`). Which region holds each
command, and which bars no application lays out, is `ambre-spec-shell.md`. Trame, Toile and
Prose show their zoom at the end of the status bar alone (`CHROME-147`). A Prose page formats
from the bar over its selection, so it lays out no command bar.

**What the toolkit ships.**

| Component | Behaviour | State |
|---|---|---|
| the shell | the grid above, and the canvas cell | built |
| the floating menu | placement, flip, roving focus, first-letter typeahead, Escape and click-outside | built |
| the modal | the native dialog element, plus the pause and resume of the frame loop | built |
| the toasts | four severities, a dwell per severity, dismiss on click, one live region | built |
| the status chips | the caption, the zoom read-out with its preset menu, the frame read-out with a panel the application supplies | built |
| the formatting controls | a toggle, a size select, a family picker, a colour popover, and a debouncer for a drag | built |
| the format bar | the bar over a selection, which Prose and Murmure hand their controls | built |
| the keybinding half | a key event to a modelled chord, for a keymap to resolve | built |
| the shortcut chip | one chord, rendered the same way in a menu, a tooltip and a palette | built |
| the command palette | one overlay listing every command the application declares, filtered by a substring of the label | built; the entries of an application and the `>` prefix are design |
| the sidebar and the panels | a resizable region beside the content, such as the context panel of `ambre-spec-shell.md` | design |

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
callbacks that pause and resume the frame loop (`CHROME-011`). A blur over a frozen canvas then
costs nothing.

**A late close.** The element's close event is queued, so a modal can open again before the event of
its last close lands. That event then describes a state already gone, and the component ignores it
(`CHROME-106`).

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

**An application may add entries to the palette.** Murmure lists its channels and its direct
messages there. The entries of the application come first, and the commands follow
(`CHROME-093`). A query that starts with `>` drops the entries and lists the commands alone
(`CHROME-094`). An application that adds no entry sees no change.

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
