# AMBRE -- FEATURE: THE SHELL ANATOMY

**Type:** Horizontal (framework)
**Applications:** Prose, Trame, Toile

> Where each command of a document page lives. Prose, Trame and Toile share one map of regions.
> A user who learns one application finds each command of the other two in the same place.

The map has one criterion per region: what the command acts on. The chrome toolkit draws the
regions (`ambre-spec-chrome.md`); this document says which command goes in which region, and which
bars no application lays out.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`. The prefix is `CHROME`, shared with the
other six documents of the DOM side.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `CHROME-148` | `INV` | Each command an application declares MUST name exactly one home region. | this spec section 2; `ambre-spec-commands.md section 2` (`UI-THREAD-019`) | type-level -- `Command::home` is a required field that holds one `Home`, so a description that names no region, or two, does not compile. The `compile_fail` doctest of `ambre_commands::Home` shows a description with no home refused | none | verified |
| `CHROME-149` | `INV` | A command that writes to the document MUST NOT have the status bar as its home region. | this spec section 2 | `ambre_toile::ui::view::view_tests::each_command_keeps_the_rules_of_its_home` -- every command of the board's table, run through the sink every region uses against a real Logic state, on a selection of one and of three: a command homed in the status bar writes nothing; `ambre_trame::ui::view::view_tests::each_command_keeps_the_rules_of_its_home` -- the same over the Trame table, and a command routed through a dialog, a file or the clipboard is never homed in the status bar; `ambre_prose::input_tests::each_command_the_view_runs_keeps_the_zoom_and_is_one_undo_item` -- Prose homes no command in the status bar | none | verified |
| `CHROME-150` | `INV` | The context bar MUST hold only commands whose home region is the context panel. | this spec section 2 | Toile `ambre_toile::ui::panel::tests::each_bar_reaches_the_panel_and_holds_no_clipboard`; Prose `ambre_prose::view::panel::tests::each_bar_reaches_the_panel_and_holds_no_clipboard`; Trame `ambre_trame::ui::panel::tests::every_command_the_panel_shows_is_homed_in_the_panel` -- the sheet lays out no bar, so its list is empty; Toile `ambre_toile::tests::chrome_ui::each_entry_of_the_board_bar_is_a_control_of_the_panel` -- wasm tier, the floating bar shows its declared list alone; Prose `ambre_prose::view::panel::tests::the_clipboard_homes_in_the_menu_and_the_panel_holds_every_formatting_command` -- the format bar shows its declared list | none | verified |
| `CHROME-151` | `INV` | The context bar MUST NOT hold a clipboard command. | this spec section 2; `ambre-spec-clipboard.md` | Toile `ambre_toile::ui::panel::tests::each_bar_reaches_the_panel_and_holds_no_clipboard`; Prose `ambre_prose::view::panel::tests::each_bar_reaches_the_panel_and_holds_no_clipboard`; Trame `ambre_trame::ui::panel::tests::the_clipboard_homes_in_the_menu_and_the_panel_holds_every_formatting_command` -- the sheet's empty bar; `ambre_commands::tests::a_bar_refuses_the_clipboard_and_a_command_of_another_home` | none | verified |
| `CHROME-152` | `INV` | The creation bar MUST hold every command that inserts a new object, and no other command. | this spec section 2 | not built | unbuilt -- lands with the `The creation bar` Story (`ambre-roadmap.md` MVP). The board's tool palette is the one creation bar, and it docks to the left | unimplemented |
| `CHROME-153` | `OBS` | In Prose, Trame and Toile, the creation bar MUST take the same place and the same shape. | this spec section 2, the placement table | not built | unbuilt -- lands with the `The creation bar` Story (`ambre-roadmap.md` MVP) | unimplemented |
| `CHROME-154` | `INV` | A command that acts on the document as a file MUST have the Ambre menu as its home region. | this spec section 2 | not built | unbuilt -- lands with the `The Ambre and avatar menus` Story (`ambre-roadmap.md` MVP). The board's File menu holds the save to a file, and no other application has a file menu | unimplemented |
| `CHROME-155` | `OBS` | The first entry of the Ambre menu MUST be the way home. | this spec section 2; `CHROME-204` | not built | unbuilt -- lands with the `The Ambre and avatar menus` Story (`ambre-roadmap.md` MVP). The way home is a link of the header | unimplemented |
| `CHROME-156` | `INV` | A command that acts on the account of the user MUST have the avatar menu as its home region. | this spec section 2 | not built | unbuilt -- lands with the `The Ambre and avatar menus` Story (`ambre-roadmap.md` MVP) | unimplemented |
| `CHROME-157` | `OBS` | The command palette MUST show the home region of each command it lists. | this spec section 2; `CHROME-008` | `ambre_chrome::chrome_ui::palette_row_shows_home_region_and_chord` -- wasm tier, each row of the palette shows the name of its command's home | none | verified |
| `CHROME-158` | `OBS` | The command palette MUST show the chord of each command that has one. | this spec section 2; `CHROME-008` | `ambre_chrome::chrome_ui::palette_row_shows_home_region_and_chord` -- wasm tier, a row shows a chord chip exactly when its command has a chord, and the chip names the key | none | verified |
| `CHROME-159` | `INV` | In Prose, Trame and Toile, the shell MUST NOT lay out a docked toolbar. | this spec section 2; this spec section 4 | `ambre_app::shell::app_root::tests::the_formula_bar_is_trames_alone_and_prose_has_no_command_bar` -- wasm tier, Trame and Prose lay out no command bar; `ambre_trame::tests::chrome_ui::the_panel_announces_the_registry_labels` -- the controls the toolbar held answer in the sheet's context panel | none | verified |
| `CHROME-160` | `INV` | In Prose, Trame and Toile, the shell MUST NOT lay out a bar of drop-down menus. | this spec section 2; this spec section 4 | not built | unbuilt -- lands with the `The Ambre and avatar menus` Story (`ambre-roadmap.md` MVP). Toile docks a bar of five menus in the command bar | unimplemented |
| `CHROME-161` | `INV` | In Prose, Trame and Toile, the shell MUST NOT lay out a button whose only role is to open a list of commands. | this spec section 2; this spec section 4 | not built | unbuilt -- lands with the `The Ambre and avatar menus` Story (`ambre-roadmap.md` MVP) | unimplemented |
| `CHROME-162` | `OBS` | In Trame, the sheet tabs MUST sit in the status bar, before the zoom. | this spec section 2; `CHROME-206` | `ambre_trame::tests::chrome_ui::the_sheet_tabs_sit_in_the_status_bar_before_the_zoom` -- wasm tier, the tabs and the zoom share one band, the zoom after the tabs, and a tab and the add button still act; `ambre_app::shell::app_root::tests::the_formula_bar_is_trames_alone_and_prose_has_no_command_bar` -- Trame lays out no tab strip | none | verified |
| `CHROME-163` | `OBS` | With nothing selected, the context panel MUST show the properties of the document: the page in Prose, the sheet in Trame, the board in Toile. | this spec section 2 | Toile `ambre_toile::tests::chrome_ui::the_board_panel_shows_the_board_then_what_applies_to_the_selection` -- wasm tier, the board section with nothing selected; Prose `ambre_prose::tests::chrome_panel::with_no_selection_the_panel_shows_the_page_the_layout_holds`; Trame `ambre_trame::tests::chrome_ui::the_panel_announces_the_registry_labels` -- the sheet section, page setup and frozen panes | none | verified |
| `CHROME-164` | `OBS` | With a selection, the context panel MUST show the commands that apply to it. | this spec section 2 | Toile `ambre_toile::tests::chrome_ui::the_board_panel_shows_the_board_then_what_applies_to_the_selection` -- wasm tier, a shape's and a connector's sections; Prose `ambre_prose::tests::chrome_panel::superscript_subscript_and_clearing_reach_a_control_of_the_panel` -- the text section over a selection; Trame `ambre_trame::tests::chrome_ui::the_panel_announces_the_registry_labels` -- the cells, the rows and columns, the sort and filter | none | verified |
| `CHROME-165` | `INV` | A command an application declares MUST NOT be reachable by hover alone. | this spec section 2; `ambre-spec-accessibility.md` | `ambre_app::shell::app_root::tests::the_palette_is_fed_the_whole_declared_table` -- wasm tier, the palette lists every command of each table; `ambre_chrome::chrome_ui::the_palette_is_operable_by_keyboard_alone` -- wasm tier, the palette moves and runs a command from the keyboard alone | none | verified |
| `CHROME-183` | `INV` | A control that writes to the document MUST NOT be reachable by hover alone. | this spec section 2; `CHROME-165`; `ambre-spec-accessibility.md` | none | unheld -- in Prose, the accept and the reject of one suggestion show only while the pointer is over it (`PROSE-UI-148`), and no scan reads the controls a hover reveals; lands with the `No control by hover alone` Story (`ambre-roadmap.md` MVP) | failing |
| `CHROME-166` | `OBS` | On a phone, one context panel MUST carry every formatting command. | this spec section 2, the placement table; `ambre-spec-adaptive-layout.md` | Prose `ambre_prose::view::panel::tests::the_clipboard_homes_in_the_menu_and_the_panel_holds_every_formatting_command` -- every command of the format group is a control of the text section; Trame `ambre_trame::ui::panel::tests::the_clipboard_homes_in_the_menu_and_the_panel_holds_every_formatting_command` -- bold and italic, the rest are properties of the cells section; Toile `ambre_toile::ui::panel::tests::the_board_formats_through_the_text_section` -- the board's text style is properties of the text section; `ambre_app::shell::narrow::a_phone_panel_is_a_sheet_on_the_bottom_edge_whatever_its_side` -- the same panel is the phone's sheet | none | verified |
| `CHROME-167` | `OBS` | On a phone, the context panel MUST open as a sheet on the bottom edge, whatever its side. | this spec section 2, the placement table; `CHROME-032` | `ambre_app::shell::narrow::a_phone_panel_is_a_sheet_on_the_bottom_edge_whatever_its_side` -- wasm tier, frames of 360 and 599 px under the shell stylesheet: on either side the panel spans the width and meets the bottom edge | none | verified |
| `CHROME-212` | `OBS` | On a phone, the context panel MUST take the place of the keyboard while it is open. | this spec section 2, the placement table; `CHROME-167` | not built | unheld -- the panel opens over the content, and nothing closes the system keyboard while it shows; no wasm page raises a keyboard to measure, so the rule waits for the browser tier on a phone profile; lands with the `The context panel, bar and menu` Story (`ambre-roadmap.md` MVP) | unimplemented |
| `CHROME-168` | `OBS` | On a phone, the context bar MUST NOT cover the menu the system raises over a selection. | this spec section 2, the placement table | not built | unheld -- a phone shows the Prose format bar and the board's bar where a desktop does, over the selection, and no page measures the menu the system raises; lands with the `The context panel, bar and menu` Story (`ambre-roadmap.md` MVP) | unimplemented |
| `CHROME-169` | `OBS` | On a phone, the Ambre menu MUST offer the command palette. | this spec section 2, the placement table; `CHROME-008` | not built | unbuilt -- lands with the `The Ambre and avatar menus` Story (`ambre-roadmap.md` MVP) | unimplemented |
| `CHROME-170` | `INV` | In Toile, the tool palette and the selection bar MUST each publish the part of the board they cover. | this spec section 2; `ambre-spec-zoom.md section 3` (`ZOOM-011`) | `ambre_toile::tests::chrome_ui::the_palette_publishes_the_edge_it_covers_and_clears_it_when_it_hides` -- wasm tier, the palette measures itself under the shell stylesheet and clears when it hides; `ambre_toile::tests::chrome_ui::the_selection_bar_publishes_its_reach_and_clears_it_when_it_leaves` -- the bar publishes its height and its lift, and clears when it leaves | none | verified |
| `CHROME-178` | `INV` | Cut, copy and paste MUST have the context menu as their home region. | this spec section 2; `ambre-spec-clipboard.md` | Toile `ambre_toile::ui::panel::tests::the_clipboard_homes_in_the_menu`; Prose `ambre_prose::view::panel::tests::the_clipboard_homes_in_the_menu_and_the_panel_holds_every_formatting_command`; Trame `ambre_trame::ui::panel::tests::the_clipboard_homes_in_the_menu_and_the_panel_holds_every_formatting_command`; type-level -- `Command::home` holds one region | none | verified |
| `CHROME-182` | `INV` | Besides its clipboard entries, the context menu MUST hold only commands whose home is the context panel. | this spec section 2 | Toile `ambre_toile::ui::panel::menu_tests::the_board_menu_opens_on_the_clipboard_then_reaches_the_panel`; Prose `ambre_prose::view::panel::tests::the_page_menu_opens_on_the_clipboard_then_reaches_the_panel`; Trame `ambre_trame::ui::panel::tests::the_sheet_menu_opens_on_the_clipboard_then_reaches_the_panel` | none | verified |
| `CHROME-179` | `OBS` | In Prose, Trame and Toile, the context menu MUST list first the clipboard entries that apply, cut, copy and paste in that order. | this spec section 2 | Toile `ambre_toile::ui::panel::menu_tests::the_board_menu_opens_on_the_clipboard_then_reaches_the_panel` -- every scope, a locked selection keeping copy and paste; Prose `ambre_prose::view::panel::tests::the_page_menu_opens_on_the_clipboard_then_reaches_the_panel`; Trame `ambre_trame::ui::panel::tests::the_sheet_menu_opens_on_the_clipboard_then_reaches_the_panel` -- every scope and filter state; `ambre_commands::tests::a_menu_opens_on_cut_copy_paste_then_reaches_the_panel_alone` | none | verified |
| `CHROME-180` | `OBS` | In Prose, the creation bar MUST offer text, a picture, a table, a separator and a comment. | this spec section 2 | not built | unbuilt -- lands with the `The creation bar` Story (`ambre-roadmap.md` MVP). Prose inserts each from a chord or the format bar | unimplemented |
| `CHROME-181` | `OBS` | In Trame, the creation bar MUST offer a function, a chart, a picture and a comment. | this spec section 2 | not built | unbuilt -- lands with the `The creation bar` Story (`ambre-roadmap.md` MVP). A filter acts on a range and creates nothing, so its home is the context panel | unimplemented |
| `CHROME-214` | `OBS` | In Prose, the context menu MUST reach after the clipboard bold, italic, underline, strikethrough and clearing the formatting. | this spec section 2; `CHROME-182` | Prose `ambre_prose::view::panel::tests::the_page_menu_opens_on_the_clipboard_then_reaches_the_panel` -- the declared list, each entry a control of the text section | none | verified |
| `CHROME-215` | `OBS` | In Trame, the context menu MUST reach after the clipboard the rows and columns, the sort, the filter, the merge and the freeze. | this spec section 2; `CHROME-182` | Trame `ambre_trame::ui::panel::tests::the_sheet_menu_opens_on_the_clipboard_then_reaches_the_panel` -- every scope, each entry a command of the panel | none | verified |
| `CHROME-216` | `OBS` | In Toile, the context menu MUST reach after the clipboard duplicate, delete, the reactions, the grouping, the order, the lock, the alignment and the frame entries that apply. | this spec section 2; `CHROME-182`; `TOILE-UI-047` | Toile `ambre_toile::tests::chrome_ui::each_entry_of_the_board_menu_is_a_control_of_the_panel` -- every scope, each entry a control the panel shows | none | verified |
| `CHROME-171` | `INV` | Each floating surface of the shell MUST publish the part of the canvas it covers. | this spec section 2; `ambre-spec-zoom.md section 3` (`ZOOM-011`) | not built | unbuilt -- lands with the `The floating cover` Story (`ambre-roadmap.md` MVP). `CHROME-170` holds the two surfaces of the board; the format bar of Prose publishes nothing | unimplemented |
| `CHROME-190` | `OBS` | In Prose, Trame and Toile, the context panel MUST take one frame: a header naming the selection, then sections that fold. | this spec section 2 | `ambre_chrome::context_panel_ui::the_panel_takes_one_frame_a_header_then_sections_that_fold` -- wasm tier, the frame; Toile `ambre_toile::tests::chrome_ui::the_board_panel_shows_the_board_then_what_applies_to_the_selection`; Prose `ambre_prose::tests::chrome_panel::with_no_selection_the_panel_shows_the_page_the_layout_holds`; Trame `ambre_trame::tests::chrome_ui::the_panel_announces_the_registry_labels` -- each application's content in that one frame | none | verified |
| `CHROME-191` | `OBS` | The rail of a collapsed panel MUST show one icon for each section of the panel. | this spec section 2 | `ambre_chrome::context_panel_ui::the_rail_shows_one_icon_for_each_section` -- wasm tier, one labelled icon per section, in order | none | verified |
| `CHROME-193` | `OBS` | On a desktop, the context panel MUST dock to the side the user chose, the right by default. | this spec section 2; `ambre-spec-adaptive-layout.md` | `ambre_app::shell::narrow::a_desktop_panel_docks_beside_the_canvas_on_its_side` -- wasm tier, a 1280 px frame under the shell stylesheet: the panel meets the right edge by default and the left edge when the side is left; `ambre_app::shell::panel::tests::the_side_is_one_setting_for_the_three_applications` -- the right by default | none | verified |
| `CHROME-194` | `INV` | The side of the context panel MUST be one setting for Prose, Trame and Toile. | this spec section 2 | `ambre_app::shell::panel::tests::the_side_is_one_setting_for_the_three_applications` -- wasm tier, a side set in Prose reads back in Trame and Toile | none | verified |
| `CHROME-198` | `OBS` | A docked context panel MUST NOT cover the canvas, on either side. | this spec section 2; `ZOOM-011` | `ambre_app::shell::narrow::a_desktop_panel_docks_beside_the_canvas_on_its_side` -- wasm tier, on either side the content cell ends where the panel starts, and the canvas sizes to its cell | none | verified |
| `CHROME-207` | `INV` | Each control of the context panel MUST carry the id of the command or of the property it reaches. | this spec section 2, the property model | Toile `ambre_toile::tests::chrome_ui::the_board_panel_shows_the_board_then_what_applies_to_the_selection`; Prose `ambre_prose::tests::chrome_panel::superscript_subscript_and_clearing_reach_a_control_of_the_panel`; `ambre_prose::tests::chrome_panel::with_no_selection_the_panel_shows_the_page_the_layout_holds`; Trame `ambre_trame::tests::chrome_ui::the_panel_announces_the_registry_labels` -- wasm tier, no button, input or select of a section stands outside a `data-reach`, and each one names a declared command or property; `ambre_chrome::context_panel_ui::each_kit_control_carries_what_it_reaches_and_shows_a_mixed_value` | none | verified |
| `CHROME-208` | `INV` | For each kind of selection, each entry of the context bar MUST be a control the context panel shows for that kind. | this spec section 2; `CHROME-150`; `CHROME-207` | Toile `ambre_toile::tests::chrome_ui::each_entry_of_the_board_bar_is_a_control_of_the_panel` -- wasm tier; Prose `ambre_prose::tests::chrome_panel::superscript_subscript_and_clearing_reach_a_control_of_the_panel` -- each entry of the text bar in the panel; Trame `ambre_trame::ui::panel::tests::the_clipboard_homes_in_the_menu_and_the_panel_holds_every_formatting_command` -- the sheet's bar is empty | none | verified |
| `CHROME-209` | `INV` | For each kind of selection, each entry of the context menu besides cut, copy and paste MUST be a control the context panel shows for that kind. | this spec section 2; `CHROME-182`; `CHROME-207` | Toile `ambre_toile::tests::chrome_ui::each_entry_of_the_board_menu_is_a_control_of_the_panel` -- wasm tier, every scope of the board's menu; Prose `ambre_prose::view::panel::tests::the_page_menu_opens_on_the_clipboard_then_reaches_the_panel`; Trame `ambre_trame::ui::panel::tests::the_sheet_menu_opens_on_the_clipboard_then_reaches_the_panel` -- each entry a command of the panel's lists | none | verified |
| `CHROME-210` | `OBS` | A property control MUST show the value the selection holds, or a mixed state when the selected elements disagree. | this spec section 2, the property model | `ambre_chrome::context_panel_ui::each_kit_control_carries_what_it_reaches_and_shows_a_mixed_value` -- wasm tier, a mixed colour says so and a uniform choice is checked; Toile `ambre_toile::ui::style::tests::differing_fill_flags_only_fill_mixed` -- the readings the board's controls take; Prose `ambre_prose::tests::chrome_panel::with_no_selection_the_panel_shows_the_page_the_layout_holds` -- the page's values; Trame `ambre_trame::tests::chrome_ui::style_applied_mirrors_into_style_state` -- the style of the active cell the controls read | none | verified |
| `CHROME-185` | `OBS` | A control of the shell that shows an icon and no text MUST show a tooltip with the name of its command, and the chord when the command has one. | this spec section 2; `ambre-spec-chrome.md section 2`, the shortcut chip | not built | unbuilt -- lands with the `Command tooltips` Story (`ambre-roadmap.md` MVP). Icon buttons carry a native title, without the chord | unimplemented |
| `CHROME-186` | `INV` | A tooltip MUST take its name, its chord and its description from the description of the command. | this spec section 2; `UI-THREAD-019` | not built | unbuilt -- lands with the `Command tooltips` Story (`ambre-roadmap.md` MVP). The description of a command carries no sentence that describes it | unimplemented |
| `CHROME-187` | `OBS` | A tooltip MUST show when its control takes the keyboard focus, as it does under the pointer. | this spec section 2; `CHROME-165` | not built | unbuilt -- lands with the `Command tooltips` Story (`ambre-roadmap.md` MVP) | unimplemented |
| `CHROME-188` | `OBS` | On a phone, a long press on an icon MUST show its tooltip in place of running its command. | this spec section 2, the placement table | not built | unbuilt -- lands with the `Command tooltips` Story (`ambre-roadmap.md` MVP) | unimplemented |
| `CHROME-189` | `OBS` | A screen reader MUST announce the name and the chord of a control that shows an icon and no text. | this spec section 2; `ambre-spec-accessibility.md` | not built | unbuilt -- lands with the `Command tooltips` Story (`ambre-roadmap.md` MVP) | unimplemented |

### 1.2 Run a command of the status bar

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `CHROME-172` | `FRAME` | A command whose home region is the status bar MUST emit no CRDT delta. | this spec section 2; `ZOOM-001` | `ambre_toile::ui::view::view_tests::each_command_keeps_the_rules_of_its_home` -- a command homed in the status bar, run on a selection of one and of three, leaves Logic with no update; `ambre_trame::ui::view::view_tests::each_command_keeps_the_rules_of_its_home` -- the same over the Trame table, and the zoom to the selection moves the zoom | none | verified |

### 1.3 Run a command that writes to the document

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `CHROME-173` | `POST` | Besides a paste over a text selection, one undo MUST take back the whole of a command that writes to the document, whichever region ran it. | this spec section 2; `ambre-spec-undo.md` | `ambre_toile::ui::view::view_tests::each_command_keeps_the_rules_of_its_home` -- every writer of the board's table, through the one sink, then one undo, compares the whole document; `ambre_toile::ui::view::view_tests::a_placed_embed_keeps_the_zoom_and_is_one_undo_item`; `::a_set_link_keeps_the_zoom_and_is_one_undo_item`; `ambre_toile::logic::state::import::tests::import_is_one_undo_group`; `ambre_trame::ui::view::view_tests::each_command_keeps_the_rules_of_its_home` -- every writer of the Trame table; `ambre_trame::ui::view::view_tests::a_paste_keeps_the_zoom_and_is_one_undo_item`; `::a_resolved_dialog_keeps_the_zoom_and_is_one_undo_item`; `ambre_trame::logic::tests::group12::an_import_is_one_undo_item`; `ambre_trame::logic::xlsx_tests::an_edit_lands_the_import_first_and_undo_removes_it_whole` -- the workbook path, which an ODS file takes too; `ambre_prose::input_tests::each_command_the_view_runs_keeps_the_zoom_and_is_one_undo_item` -- the marks, the cut and the pastes at a caret; `ambre_prose::logic::pictures::tests::a_picture_from_a_file_is_one_undo_item`; `ambre_prose::logic::interop::tests::one_undo_takes_the_import_back` | none | verified |
| `CHROME-184` | `POST` | One undo MUST take back the whole of a paste over a text selection, the text it replaced included. | this spec section 2; `CHROME-173`; `ambre-spec-undo.md` | none | unheld -- in Prose, the view takes the selection out and then pastes, as two commands, so the first undo puts the paste back and leaves the selection deleted; lands with the `A paste over a selection is one undo item` Story (`ambre-roadmap.md` MVP) | failing |
| `CHROME-174` | `FRAME` | A command that writes to the document MUST NOT change the zoom. | this spec section 2; `ambre-spec-zoom.md` | `ambre_toile::ui::view::view_tests::each_command_keeps_the_rules_of_its_home` -- every writer of the board's table, from a zoom off one hundred per cent; `ambre_toile::ui::view::view_tests::a_placed_embed_keeps_the_zoom_and_is_one_undo_item`; `::a_set_link_keeps_the_zoom_and_is_one_undo_item`; `::an_import_keeps_the_zoom`; `ambre_trame::ui::view::view_tests::each_command_keeps_the_rules_of_its_home` -- every writer of the Trame table; `ambre_trame::ui::view::view_tests::a_paste_keeps_the_zoom_and_is_one_undo_item`; `::a_resolved_dialog_keeps_the_zoom_and_is_one_undo_item`; `::an_import_keeps_the_zoom`; `ambre_prose::input_tests::each_command_the_view_runs_keeps_the_zoom_and_is_one_undo_item`; `::an_import_keeps_the_zoom` | none | verified |

### 1.4 Collapse, open, dock and size the context panel

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `CHROME-175` | `POST` | `Ctrl+.` MUST collapse an open context panel to a rail. | this spec section 3 | `ambre_chrome::context_panel_ui::ctrl_period_collapses_an_open_panel_and_opens_a_collapsed_one` -- wasm tier, the chord on the window folds the open frame to its rail | none | verified |
| `CHROME-176` | `POST` | `Ctrl+.` or a click on the rail MUST open a collapsed context panel. | this spec section 3 | `ambre_chrome::context_panel_ui::ctrl_period_collapses_an_open_panel_and_opens_a_collapsed_one` -- wasm tier, the chord and a click on the rail each open the folded frame | none | verified |
| `CHROME-177` | `FRAME` | Collapsing or opening the context panel MUST NOT write to the document, nor drop the open section, the scroll or the edit in progress of the panel. | this spec section 3 | `ambre_chrome::context_panel_ui::a_collapse_keeps_the_open_sections_the_scroll_and_the_edit` -- wasm tier under the shell stylesheet: the same field holds its typed text, the list its scroll, a folded section its fold; structural -- the frame takes no command sink, so a collapse has no path to the document | none | verified |
| `CHROME-192` | `POST` | A click on an icon of the rail MUST open the panel at the section of that icon. | this spec section 3 | `ambre_chrome::context_panel_ui::a_rail_icon_opens_the_panel_at_its_section` -- wasm tier, the icon of a folded section opens the panel with that section unfolded | none | verified |
| `CHROME-195` | `FRAME` | Collapsing the context panel in one application MUST NOT collapse it in another. | this spec section 2 | `ambre_app::shell::panel::tests::a_fold_in_one_application_leaves_the_others_open` -- wasm tier, a fold kept in Trame leaves Prose and Toile open | none | verified |
| `CHROME-196` | `POST` | The context panel MUST open in an application as the user last left it there, collapsed or open. | this spec section 2 | `ambre_app::shell::panel::tests::the_panel_opens_as_it_was_last_left_in_that_application` -- wasm tier, the fold and the width read back as they were kept, then open again once opened | none | verified |
| `CHROME-197` | `POST` | A drag of the edge of the context panel MUST set its width, held between the narrowest and the widest width of section 2. | this spec section 2, the placement table | `ambre_chrome::context_panel_ui::a_drag_of_the_edge_sets_the_width_between_the_two_bounds` -- wasm tier, pointer events on the edge set 360 px, then clamp to 480 and 240, and a move after the release sets nothing; `ambre_chrome::context_panel::tests::a_width_is_held_between_the_two_bounds` | none | verified |

### 1.5 Edit a property of the selection

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `CHROME-211` | `POST` | A write from a property control MUST change that property alone, on each element of the selection. | this spec section 2, the property model; `CHROME-173` | `ambre_chrome::context_panel_ui::each_kit_control_carries_what_it_reaches_and_shows_a_mixed_value` -- a stepper writes its length alone; Toile `ambre_toile::ui::floatbar::tests::a_panel_edit_keeps_every_field_it_did_not_edit`; Prose `ambre_prose::tests::chrome_panel::a_page_control_hands_the_whole_format_to_the_view` -- one field of the format changed; Trame `ambre_trame::logic::tests::group13::a_border_preset_reaches_only_its_edges`; `ambre_trame::logic::tests::group13::an_empty_style_patch_writes_nothing` -- a patch writes the fields it names alone | none | verified |
| `CHROME-213` | `FRAME` | A write from a property control MUST NOT change what is selected, nor the zoom. | this spec section 2, the property model; `CHROME-174` | not built | unheld -- the board holds it (`TOILE-UI-176`, `TOILE-UI-181`), but no test runs a page write of Prose or a style patch of Trame from a zoom off one hundred per cent; lands with the `The context panel, bar and menu` Story (`ambre-roadmap.md` MVP) | unimplemented |

* **Budget.** none: the regions are DOM, off the frame loop, and `ambre-spec-budgets.md` budgets no
  chrome operation. A cover a surface publishes is read by a fit, which is a zoom cost.
* **Boundary.** none: a region holds commands, and a command reaches the Logic thread as every
  command does. Nothing here opens a socket or holds a key.
* **Verification.** 40 `verified`, 2 `failing`, 19 `unimplemented`, and the type holds the home of
  each command. Host tests run each table through the one sink of every region, on a real Logic
  state. Wasm tests hold the palette and the sheet tabs. A hover-only control of Prose and a paste
  over a text selection fail. The rest waits on the regions, which no application lays out.

---

## 2. MODEL

**CRDT containers.** none. A region is view state. What a command of the context panel writes is
owned by the feature that declares the command.

**The eight regions, and the question each one answers.** A command goes where its subject is. A
command has one home, so a user who looks for it looks in one place. Six regions are homes; the
context bar and the command palette reach commands whose home is elsewhere.

| Region | Answers | Home of commands | Writes to the document | Examples |
|---|---|---|---|---|
| status bar | how I see the document | yes | never (`CHROME-149`) | the zoom, the counters, the sheet tabs of Trame |
| context panel | what the document or the selection is | yes | yes | the page format and the margins of Prose, the sheet and the filter of Trame, the board of Toile; the style of a selection |
| creation bar | what I can add | yes | yes | the tools of the board; text, a picture, a table, a chart, a comment (`CHROME-180`, `CHROME-181`) |
| context menu | what I do with the selection under the pointer | the clipboard alone (`CHROME-178`) | yes | the clipboard first (`CHROME-179`), then commands of the panel (`CHROME-182`, `CHROME-214` to `CHROME-216`) |
| Ambre menu | what I do with the file | yes | through the file | the way home first, then new, open, rename, export, print, the history |
| avatar menu | who I am | yes | never | the account, the theme, the interface language, sign out |
| context bar | the most frequent actions of the panel, at the pointer | no (`CHROME-150`) | yes | bold, italic, colour, a link |
| command palette | every command, by name | no | as the command does | each command with its chord and its home region (`CHROME-008`) |

**A home is not the only way in.** A chord and the command palette reach a command in any
region. The share button of the header reaches a command whose home is the Ambre menu. A reach
never makes a second home, so a command keeps one entry in the region map.

**What is contextual repeats the panel.** The context panel is the home of every command of the
selection. Two surfaces show part of it again, at the pointer, and are the exceptions to one place.
The context bar gathers the actions a user repeats most, so bold costs no pointer travel
(`CHROME-150`). After the clipboard, the context menu lists the panel actions that apply under
the pointer (`CHROME-182`). Each surface shows a command twice, yet files it once.

The shared word is the rule: a surface named contextual holds nothing the panel lacks, except the
clipboard.

**The clipboard lives in the context menu.** Copy writes nothing, and cut and paste write, so the
three fit neither the status bar nor the panel. The context bar holds none of them (`CHROME-151`).
The context menu opens on the selection, which is what the three act on, and lists them first in
every application (`CHROME-179`).

**One frame, three contents.** The shell owns the frame of the context panel, the same in the
three applications (`CHROME-190`). A header names the selection, sections fold, and the panel
collapses to a rail. Each section declares an icon and a label, and the rail shows those icons
(`CHROME-191`): a click opens the panel at that section (`CHROME-192`). An application writes the
content of its sections and nothing else, from the widgets the chrome toolkit shares.

**A value is a property, not a command.** A command runs with no argument: bold, delete, bring
to front. A colour, a font, a size, a number format or a margin carries a value. The panel edits it
through a property: a value read from the selection, and a write of a new value. The selection
holds one value, or a mixed state when its elements disagree (`CHROME-210`).

A write changes that
property alone, on each element of the selection (`CHROME-211`). A property has no home field: the
context panel is its home by construction. The context bar and the context menu reach a command or
a property, and nothing else.

**The panel names what it reaches.** Each control of the panel carries the id of its command or of
its property (`CHROME-207`). The context bar and the context menu are declared lists of those ids,
one list per kind of selection. Each entry of either list is a control the panel shows for that
kind (`CHROME-208`, `CHROME-209`). The clipboard of the menu is the one exception (`CHROME-178`).

**The user chooses the side.** The panel docks to the right by default, or to the left, and the
choice holds in the three applications (`CHROME-193`, `CHROME-194`). Whether it is collapsed is
remembered for each application (`CHROME-195`, `CHROME-196`): a writer folds it in Prose and keeps
it open in Trame. A drag of its inner edge sets its width, from 240 to 480 CSS pixels, 300 by
default (`CHROME-197`). The bounds are CSS pixels, which keep their size from a 1080p screen to a
4K one at its scale. The panel docks beside the canvas and never over it
(`CHROME-198`), so a fit frames inside what is left, on either side.

**The panel shows the document when nothing is selected.** The page format and the margins of
Prose, the sheet of Trame and the board of Toile are document state. They live in the panel, never
in the status bar (`CHROME-163`).

**What the shell does not lay out.** No docked toolbar, no bar of drop-down menus and no button that
opens a list of every command (`CHROME-159` to `CHROME-161`). The formula bar of Trame stays: it
edits the content of the active cell, and holds no command.

**Where each region sits.** The roles above are fixed; the place follows the width
(`ambre-spec-adaptive-layout.md`).

| Region | Desktop | Phone |
|---|---|---|
| status bar | a band under the content, the zoom at its end | a short band; a pinch zooms |
| sheet tabs of Trame | in the status bar, before the zoom (`CHROME-162`) | a strip above the status bar |
| context panel | a column on the side the user chose, the right by default, collapsible to a rail (`CHROME-193`) | a sheet from the bottom edge, in place of the keyboard (`CHROME-167`, `CHROME-212`) |
| context bar | floating over the selection | a bar above the keyboard, clear of the system menu (`CHROME-168`) |
| creation bar | floating, centred at the bottom of the content | one add button that opens the modes |
| Ambre menu | the logo, top left of the header | the logo, which also offers the command palette (`CHROME-169`) |
| avatar menu | the avatar, top right of the header | the avatar |
| command palette | `Ctrl+K` | an entry of the Ambre menu |

**An icon names itself.** A bar of icons alone teaches nothing, so each icon shows a tooltip. It
holds the name of its command, its chord as the shortcut chip, and one line of description where
the name is not enough (`CHROME-185`). The tooltip reads the description of the command, as the
palette and the context menu do, so no tooltip is written by hand (`CHROME-186`). It shows under
the keyboard focus as under the pointer (`CHROME-187`), and a screen reader hears the same name and
chord (`CHROME-189`). A tooltip waits about half a second, then shows at once while the pointer
moves to the next icon.

**A floating surface publishes what it covers.** The context bar, the creation bar and the
context panel float over the canvas, so a fit must frame the selection clear of them (`ZOOM-011`).
Each surface measures the part of the canvas it covers, and a fit reads the latest measure
(`CHROME-171`). The board's tool palette and selection bar do it today (`CHROME-170`). No size of
a surface is written twice.

**Code:** the board's cover is `crates/ambre-toile/src/ui/cover.rs`, and the measure the chrome
takes is `crates/ambre-toile/src/chrome/cover.rs`. No region has code beyond it.

---

## 3. SURFACE

| Trigger | Behaviour |
|---|---|
| `Ctrl+K` | the command palette opens, each row with its chord and its home region |
| `Ctrl+.` on an open context panel | the panel collapses to a rail, and keeps its state |
| `Ctrl+.`, or a click on the rail | the panel opens again as it was |
| a click on the Ambre logo | the Ambre menu opens, the way home first |
| a click on the avatar | the avatar menu opens |
| the pointer or the focus held on an icon | its tooltip shows the name, the chord and, if useful, a description |
| a long press on an icon, on a phone | its tooltip shows, and the command does not run |
| a selection made | the context bar floats over it, and the panel shows what applies to it |
| the selection cleared | the bar leaves, and the panel shows the document |
| a mode of the creation bar | the next press on the content inserts that object |
| a tap on a selection, on a phone | the system menu offers copy and paste, and the bar sits above the keyboard |

---

## 4. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| One region per subject | a docked toolbar | it holds every frequent command at once, whatever is selected. It costs a row of height on every page, and a narrow window clips it or overflows it into a menu |
| One region per subject | a bar of drop-down menus | it files commands under words a user must guess, and hides each one behind two clicks. The command palette finds a command by name in one |
| One region per subject | a hamburger button | it is a drawer of everything, so it answers no question. The Ambre menu holds the file and the avatar menu holds the account, each with a subject |
| The margins live in the panel | the margins in the status bar | margins are document state and every peer sees them. The zoom is view state and only this user sees it. One band holding both mixes what writes the document with what does not |
| A context bar beside the panel | the formatting in the side panel alone | bold and italic are the actions a user repeats most. The pointer would cross the screen to the panel for each one |
| One map for three applications | one chrome per application | a user would learn three places for the same command. Each application keeps its commands, and the map of regions is shared |

---

## 5. NOT BUILT, AND NOT YET RULED

* **The side, the width and the fold of the panel stay on the device.** No setting of the account
  travels between devices yet, and the theme itself is read from the system at each boot
  (`UI-THREAD-066`). A setting that follows the account needs a sealed store of its own, which no
  spec owns yet. That store is the work of a branch of its own, which brings its spec and its story.
* **The board's tool palette docks to the left today.** A panel docked to the left would cover it
  until `The creation bar` Story (`ambre-roadmap.md` MVP) moves it to the bottom.
* **The sheet tabs in the status bar are a trial.** The owner chose on 2026-11-21 to try them there.
  A workbook of many sheets may not fit beside the
  counters and the zoom. If the trial fails, the tabs keep a strip of their own, and `CHROME-162`
  changes. No rule bounds the number of tabs the band shows.
* **The board's menu bar holds verified rows.** `TOILE-UI-018`, `TOILE-UI-050`, `TOILE-UI-121` to
  `TOILE-UI-125`, `TOILE-UI-355`, `TOILE-UI-364`, `TOILE-UI-375` and `TOILE-UI-416` to
  `TOILE-UI-418` bind its menus. They stay `verified` while the bar ships. The
  `The Ambre and avatar menus` Story (`ambre-roadmap.md` MVP) moves each command to its home and
  rewrites them.
* **Each application lays out its own context menu.** Trame's is in
  `ambre-spec-trame-context-menu.md`, the board's in `ambre-spec-toile-menus.md`, and the page's in
  Prose. Each lists the clipboard first, then commands its panel shows (`CHROME-214` to
  `CHROME-216`). No rule says what a long press shows on a phone, where the system menu covers text
  alone.
* **Trame lays out no context bar yet.** Its panel holds bold, the fill, the number format, the
  merge and the sort, which a floating bar over a range would repeat.

---

## 6. WHAT THIS DOCUMENT DOES NOT OWN

| Question | Owner |
|---|---|
| The components that draw a region: the menu, the chips, the format bar, the palette | `ambre-spec-chrome.md` |
| The description of a command, and its chord | `ambre-spec-commands.md` |
| Where the three widths fall, and what a narrow window changes | `ambre-spec-adaptive-layout.md` |
| The zoom, and how a fit reads the cover | `ambre-spec-zoom.md` |
| What one undo takes back | `ambre-spec-undo.md` |
| The context menu of each application | `3-Toile/features/ambre-spec-toile-menus.md`, `2-Trame/features/ambre-spec-trame-context-menu.md` |
| The chrome of Murmure: its regions, its channel header in the command bar | `5-Murmure/features/ambre-spec-murmure-layout.md` |
