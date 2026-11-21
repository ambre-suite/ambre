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
| `CHROME-150` | `INV` | The context bar MUST hold only commands whose home region is the context panel. | this spec section 2 | not built | unbuilt -- lands with the `The context panel and the context bar` Story (`ambre-roadmap.md` MVP) | unimplemented |
| `CHROME-151` | `INV` | The context bar MUST NOT hold a clipboard command. | this spec section 2; `ambre-spec-clipboard.md` | not built | unbuilt -- lands with the `The context panel and the context bar` Story (`ambre-roadmap.md` MVP) | unimplemented |
| `CHROME-152` | `INV` | The creation bar MUST hold every command that inserts a new object, and no other command. | this spec section 2 | not built | unbuilt -- lands with the `The creation bar` Story (`ambre-roadmap.md` MVP). The board's tool palette is the one creation bar, and it docks to the left | unimplemented |
| `CHROME-153` | `OBS` | In Prose, Trame and Toile, the creation bar MUST take the same place and the same shape. | this spec section 2, the placement table | not built | unbuilt -- lands with the `The creation bar` Story (`ambre-roadmap.md` MVP) | unimplemented |
| `CHROME-154` | `INV` | A command that acts on the document as a file MUST have the Ambre menu as its home region. | this spec section 2 | not built | unbuilt -- lands with the `The Ambre and avatar menus` Story (`ambre-roadmap.md` MVP). The board's File menu holds the save to a file, and no other application has a file menu | unimplemented |
| `CHROME-155` | `OBS` | The first entry of the Ambre menu MUST be the way home. | this spec section 2; `CHROME-145` | not built | unbuilt -- lands with the `The Ambre and avatar menus` Story (`ambre-roadmap.md` MVP). The way home is a link of the header | unimplemented |
| `CHROME-156` | `INV` | A command that acts on the account of the user MUST have the avatar menu as its home region. | this spec section 2 | not built | unbuilt -- lands with the `The Ambre and avatar menus` Story (`ambre-roadmap.md` MVP) | unimplemented |
| `CHROME-157` | `OBS` | The command palette MUST show the home region of each command it lists. | this spec section 2; `CHROME-008` | `ambre_chrome::chrome_ui::palette_row_shows_home_region_and_chord` -- wasm tier, each row of the palette shows the name of its command's home | none | verified |
| `CHROME-158` | `OBS` | The command palette MUST show the chord of each command that has one. | this spec section 2; `CHROME-008` | `ambre_chrome::chrome_ui::palette_row_shows_home_region_and_chord` -- wasm tier, a row shows a chord chip exactly when its command has a chord, and the chip names the key | none | verified |
| `CHROME-159` | `INV` | In Prose, Trame and Toile, the shell MUST NOT lay out a docked toolbar. | this spec section 2; this spec section 4 | not built | unbuilt -- lands with the `The context panel and the context bar` Story (`ambre-roadmap.md` MVP). Trame docks its format toolbar in the command bar, and it leaves once the panel carries its commands | unimplemented |
| `CHROME-160` | `INV` | In Prose, Trame and Toile, the shell MUST NOT lay out a bar of drop-down menus. | this spec section 2; this spec section 4 | not built | unbuilt -- lands with the `The Ambre and avatar menus` Story (`ambre-roadmap.md` MVP). Toile docks a bar of five menus in the command bar | unimplemented |
| `CHROME-161` | `INV` | In Prose, Trame and Toile, the shell MUST NOT lay out a button whose only role is to open a list of commands. | this spec section 2; this spec section 4 | not built | unbuilt -- lands with the `The Ambre and avatar menus` Story (`ambre-roadmap.md` MVP) | unimplemented |
| `CHROME-162` | `OBS` | In Trame, the sheet tabs MUST sit in the status bar, before the zoom. | this spec section 2; `CHROME-147` | `ambre_trame::tests::chrome_ui::the_sheet_tabs_sit_in_the_status_bar_before_the_zoom` -- wasm tier, the tabs and the zoom share one band, the zoom after the tabs, and a tab and the add button still act; `ambre_app::shell::app_root::tests::the_formula_bar_is_trames_alone_and_prose_has_no_command_bar` -- Trame lays out no tab strip | none | verified |
| `CHROME-163` | `OBS` | With nothing selected, the context panel MUST show the properties of the document: the page in Prose, the sheet in Trame, the board in Toile. | this spec section 2 | not built | unbuilt -- lands with the `The context panel and the context bar` Story (`ambre-roadmap.md` MVP). No application lays out a context panel | unimplemented |
| `CHROME-164` | `OBS` | With a selection, the context panel MUST show the commands that apply to it. | this spec section 2 | not built | unbuilt -- lands with the `The context panel and the context bar` Story (`ambre-roadmap.md` MVP) | unimplemented |
| `CHROME-165` | `INV` | A command an application declares MUST NOT be reachable by hover alone. | this spec section 2; `ambre-spec-accessibility.md` | `ambre_app::shell::app_root::tests::the_palette_is_fed_the_whole_declared_table` -- wasm tier, the palette lists every command of each table; `ambre_chrome::chrome_ui::the_palette_is_operable_by_keyboard_alone` -- wasm tier, the palette moves and runs a command from the keyboard alone | none | verified |
| `CHROME-183` | `INV` | A control that writes to the document MUST NOT be reachable by hover alone. | this spec section 2; `CHROME-165`; `ambre-spec-accessibility.md` | none | unheld -- in Prose, the accept and the reject of one suggestion show only while the pointer is over it (`PROSE-UI-148`), and no scan reads the controls a hover reveals; lands with the `No control by hover alone` Story (`ambre-roadmap.md` MVP) | failing |
| `CHROME-166` | `OBS` | On a phone, one context panel MUST carry every formatting command. | this spec section 2, the placement table; `ambre-spec-adaptive-layout.md` | not built | unbuilt -- lands with the `The context panel and the context bar` Story (`ambre-roadmap.md` MVP) | unimplemented |
| `CHROME-167` | `OBS` | On a phone, the context panel MUST open from the bottom edge, in place of the keyboard. | this spec section 2, the placement table; `CHROME-032` | not built | unbuilt -- lands with the `The context panel and the context bar` Story (`ambre-roadmap.md` MVP) | unimplemented |
| `CHROME-168` | `OBS` | On a phone, the context bar MUST NOT cover the menu the system raises over a selection. | this spec section 2, the placement table | not built | unbuilt -- lands with the `The context panel and the context bar` Story (`ambre-roadmap.md` MVP) | unimplemented |
| `CHROME-169` | `OBS` | On a phone, the Ambre menu MUST offer the command palette. | this spec section 2, the placement table; `CHROME-008` | not built | unbuilt -- lands with the `The Ambre and avatar menus` Story (`ambre-roadmap.md` MVP) | unimplemented |
| `CHROME-170` | `INV` | In Toile, the tool palette and the selection bar MUST each publish the part of the board they cover. | this spec section 2; `ambre-spec-zoom.md section 3` (`ZOOM-011`) | `ambre_toile::tests::chrome_ui::the_palette_publishes_the_edge_it_covers_and_clears_it_when_it_hides` -- wasm tier, the palette measures itself under the shell stylesheet and clears when it hides; `ambre_toile::tests::chrome_ui::the_selection_bar_publishes_its_reach_and_clears_it_when_it_leaves` -- the bar publishes its height and its lift, and clears when it leaves | none | verified |
| `CHROME-178` | `INV` | Cut, copy and paste MUST have the context menu as their home region. | this spec section 2; `ambre-spec-clipboard.md` | not built | unbuilt -- lands with the `One context menu` Story (`ambre-roadmap.md` MVP). Each application lays out a context menu of its own | unimplemented |
| `CHROME-182` | `INV` | Besides cut, copy and paste, the context menu MUST hold only commands whose home region is the context panel. | this spec section 2 | not built | unbuilt -- lands with the `One context menu` Story (`ambre-roadmap.md` MVP). Each menu holds commands of its own, such as the sort and the merge of Trame | unimplemented |
| `CHROME-179` | `OBS` | In Prose, Trame and Toile, the context menu MUST list cut, copy and paste first, in that order. | this spec section 2 | not built | unbuilt -- lands with the `One context menu` Story (`ambre-roadmap.md` MVP) | unimplemented |
| `CHROME-180` | `OBS` | In Prose, the creation bar MUST offer text, a picture, a table, a separator and a comment. | this spec section 2 | not built | unbuilt -- lands with the `The creation bar` Story (`ambre-roadmap.md` MVP). Prose inserts each from a chord or the format bar | unimplemented |
| `CHROME-181` | `OBS` | In Trame, the creation bar MUST offer a function, a chart, a picture and a comment. | this spec section 2 | not built | unbuilt -- lands with the `The creation bar` Story (`ambre-roadmap.md` MVP). A filter acts on a range and creates nothing, so its home is the context panel | unimplemented |
| `CHROME-171` | `INV` | Each floating surface of the shell MUST publish the part of the canvas it covers. | this spec section 2; `ambre-spec-zoom.md section 3` (`ZOOM-011`) | not built | unbuilt -- lands with the `The floating cover` Story (`ambre-roadmap.md` MVP). `CHROME-170` holds the two surfaces of the board; the format bar of Prose publishes nothing | unimplemented |

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

### 1.4 Collapse and open the context panel

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `CHROME-175` | `POST` | `Ctrl+.` MUST collapse an open context panel to a rail. | this spec section 3 | not built | unbuilt -- lands with the `The context panel and the context bar` Story (`ambre-roadmap.md` MVP) | unimplemented |
| `CHROME-176` | `POST` | `Ctrl+.` or a click on the rail MUST open a collapsed context panel. | this spec section 3 | not built | unbuilt -- lands with the `The context panel and the context bar` Story (`ambre-roadmap.md` MVP) | unimplemented |
| `CHROME-177` | `FRAME` | Collapsing or opening the context panel MUST NOT write to the document, nor drop the open section, the scroll or the edit in progress of the panel. | this spec section 3 | not built | unbuilt -- lands with the `The context panel and the context bar` Story (`ambre-roadmap.md` MVP) | unimplemented |

* **Budget.** none: the regions are DOM, off the frame loop, and `ambre-spec-budgets.md` budgets no
  chrome operation. A cover a surface publishes is read by a fit, which is a zoom cost.
* **Boundary.** none: a region holds commands, and a command reaches the Logic thread as every
  command does. Nothing here opens a socket or holds a key.
* **Verification.** 10 `verified`, 2 `failing`, 25 `unimplemented`, and the type holds the home
  of each command. Host tests run each table through the one sink of every region, on a real Logic
  state. Wasm tests hold the palette and the sheet tabs. A hover-only control of Prose and a
  paste over a text selection fail. The rest waits on the regions, which no application lays out.

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
| context menu | what I do with the selection under the pointer | the clipboard alone (`CHROME-178`) | yes | cut, copy and paste first (`CHROME-179`), then commands of the panel (`CHROME-182`) |
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
| context panel | a column on the right, collapsible to a rail | a sheet from the bottom edge, in place of the keyboard (`CHROME-167`) |
| context bar | floating over the selection | a bar above the keyboard, clear of the system menu (`CHROME-168`) |
| creation bar | floating, centred at the bottom of the content | one add button that opens the modes |
| Ambre menu | the logo, top left of the header | the logo, which also offers the command palette (`CHROME-169`) |
| avatar menu | the avatar, top right of the header | the avatar |
| command palette | `Ctrl+K` | an entry of the Ambre menu |

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

* **The sheet tabs in the status bar are a trial.** The owner chose on 2026-11-21 to try them there.
  A workbook of many sheets may not fit beside the
  counters and the zoom. If the trial fails, the tabs keep a strip of their own, and `CHROME-162`
  changes. No rule bounds the number of tabs the band shows.
* **The board's menu bar holds verified rows.** `TOILE-UI-018`, `TOILE-UI-050`, `TOILE-UI-121` to
  `TOILE-UI-125`, `TOILE-UI-355`, `TOILE-UI-364`, `TOILE-UI-375` and `TOILE-UI-416` to
  `TOILE-UI-418` bind its menus. They stay `verified` while the bar ships. The
  `The Ambre and avatar menus` Story (`ambre-roadmap.md` MVP) moves each command to its home and
  rewrites them.
* **Trame's toolbar holds verified rows.** `TRAME-097`, `TRAME-186` and `TRAME-188` name the
  toolbar as the control that reports a style. The
  `The context panel and the context bar` Story (`ambre-roadmap.md` MVP) rewrites them
  against the panel.
* **The context menu has no shared design.** Each application lays out its own: Trame's in
  `ambre-spec-trame-context-menu.md`, the board's in `ambre-spec-toile-menus.md`, and a page menu in
  Prose. Beyond the clipboard first, no rule says which selection commands it reaches, nor in
  which order. No rule says what a long press shows on a phone either, where the system menu
  covers text alone. The
  `One context menu` Story (`ambre-roadmap.md` MVP) designs it before it lays it out.

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
