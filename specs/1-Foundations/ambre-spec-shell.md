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
| `CHROME-148` | `INV` | Each command an application declares MUST name exactly one home region. | this spec section 2; `ambre-spec-commands.md section 2` (`UI-THREAD-019`) | not built | unbuilt -- lands with the `Command home regions` Story (`ambre-roadmap.md` V1). The description of a command carries a category, and no region | unimplemented |
| `CHROME-149` | `INV` | A command that writes to the document MUST NOT have the status bar as its home region. | this spec section 2 | not built | unbuilt -- lands with the `Command home regions` Story (`ambre-roadmap.md` V1) | unimplemented |
| `CHROME-150` | `INV` | The near-selection bar MUST hold only commands whose home region is the context panel. | this spec section 2 | not built | unbuilt -- lands with the `The context panel and the near-selection bar` Story (`ambre-roadmap.md` V1) | unimplemented |
| `CHROME-151` | `INV` | The near-selection bar MUST NOT hold a clipboard command. | this spec section 2; `ambre-spec-clipboard.md` | not built | unbuilt -- lands with the `The context panel and the near-selection bar` Story (`ambre-roadmap.md` V1) | unimplemented |
| `CHROME-152` | `INV` | The creation bar MUST hold every command that inserts a new object, and no other command. | this spec section 2 | not built | unbuilt -- lands with the `The creation bar` Story (`ambre-roadmap.md` V1). The board's tool palette is the one creation bar, and it docks to the left | unimplemented |
| `CHROME-153` | `OBS` | In Prose, Trame and Toile, the creation bar MUST take the same place and the same shape. | this spec section 2, the placement table | not built | unbuilt -- lands with the `The creation bar` Story (`ambre-roadmap.md` V1) | unimplemented |
| `CHROME-154` | `INV` | A command that acts on the document as a file MUST have the Ambre menu as its home region. | this spec section 2 | not built | unbuilt -- lands with the `The Ambre and avatar menus` Story (`ambre-roadmap.md` V1). The board's File menu holds the save to a file, and no other application has a file menu | unimplemented |
| `CHROME-155` | `OBS` | The first entry of the Ambre menu MUST be the way home. | this spec section 2; `CHROME-145` | not built | unbuilt -- lands with the `The Ambre and avatar menus` Story (`ambre-roadmap.md` V1). The way home is a link of the header | unimplemented |
| `CHROME-156` | `INV` | A command that acts on the account of the user MUST have the avatar menu as its home region. | this spec section 2 | not built | unbuilt -- lands with the `The Ambre and avatar menus` Story (`ambre-roadmap.md` V1) | unimplemented |
| `CHROME-157` | `OBS` | The command palette MUST show the home region of each command it lists. | this spec section 2; `CHROME-008` | not built | unbuilt -- lands with the `Command home regions` Story (`ambre-roadmap.md` V1) | unimplemented |
| `CHROME-158` | `OBS` | The command palette MUST show the chord of each command that has one. | this spec section 2; `CHROME-008` | not built | unbuilt -- lands with the `Command home regions` Story (`ambre-roadmap.md` V1). The shortcut chip exists, and no rule holds it in the palette | unimplemented |
| `CHROME-159` | `INV` | In Prose, Trame and Toile, the shell MUST NOT lay out a docked toolbar. | this spec section 2; this spec section 4 | not built | unbuilt -- lands with the `Command home regions` Story (`ambre-roadmap.md` V1). Trame docks its format toolbar in the command bar | unimplemented |
| `CHROME-160` | `INV` | In Prose, Trame and Toile, the shell MUST NOT lay out a bar of drop-down menus. | this spec section 2; this spec section 4 | not built | unbuilt -- lands with the `The Ambre and avatar menus` Story (`ambre-roadmap.md` V1). Toile docks a bar of five menus in the command bar | unimplemented |
| `CHROME-161` | `INV` | In Prose, Trame and Toile, the shell MUST NOT lay out a button whose only role is to open a list of commands. | this spec section 2; this spec section 4 | not built | unbuilt -- lands with the `The Ambre and avatar menus` Story (`ambre-roadmap.md` V1) | unimplemented |
| `CHROME-162` | `OBS` | In Trame, the sheet tabs MUST sit in the status bar, before the zoom. | this spec section 2; `CHROME-147` | not built | unbuilt -- lands with the `Command home regions` Story (`ambre-roadmap.md` V1). The tabs fill a row of their own, and section 5 holds the trial | unimplemented |
| `CHROME-163` | `OBS` | With nothing selected, the context panel MUST show the properties of the document: the page in Prose, the sheet in Trame, the board in Toile. | this spec section 2 | not built | unbuilt -- lands with the `The context panel and the near-selection bar` Story (`ambre-roadmap.md` V1). No application lays out a context panel | unimplemented |
| `CHROME-164` | `OBS` | With a selection, the context panel MUST show the commands that apply to it. | this spec section 2 | not built | unbuilt -- lands with the `The context panel and the near-selection bar` Story (`ambre-roadmap.md` V1) | unimplemented |
| `CHROME-165` | `INV` | A command MUST NOT be reachable by hover alone. | this spec section 2; `ambre-spec-accessibility.md` | not built | unbuilt -- lands with the `Command home regions` Story (`ambre-roadmap.md` V1). No scan reads the controls a hover reveals | unimplemented |
| `CHROME-166` | `OBS` | On a phone, one context panel MUST carry every formatting command. | this spec section 2, the placement table; `ambre-spec-adaptive-layout.md` | not built | unbuilt -- lands with the `The context panel and the near-selection bar` Story (`ambre-roadmap.md` V1) | unimplemented |
| `CHROME-167` | `OBS` | On a phone, the context panel MUST open from the bottom edge, in place of the keyboard. | this spec section 2, the placement table; `CHROME-032` | not built | unbuilt -- lands with the `The context panel and the near-selection bar` Story (`ambre-roadmap.md` V1) | unimplemented |
| `CHROME-168` | `OBS` | On a phone, the near-selection bar MUST NOT cover the menu the system raises over a selection. | this spec section 2, the placement table | not built | unbuilt -- lands with the `The context panel and the near-selection bar` Story (`ambre-roadmap.md` V1) | unimplemented |
| `CHROME-169` | `OBS` | On a phone, the Ambre menu MUST offer the command palette. | this spec section 2, the placement table; `CHROME-008` | not built | unbuilt -- lands with the `The Ambre and avatar menus` Story (`ambre-roadmap.md` V1) | unimplemented |
| `CHROME-170` | `INV` | In Toile, the tool palette and the selection bar MUST each publish the part of the board they cover. | this spec section 2; `ambre-spec-zoom.md section 3` (`ZOOM-011`) | `ambre_toile::tests::chrome_ui::the_palette_publishes_the_edge_it_covers_and_clears_it_when_it_hides` -- wasm tier, the palette measures itself under the shell stylesheet and clears when it hides; `ambre_toile::tests::chrome_ui::the_selection_bar_publishes_its_reach_and_clears_it_when_it_leaves` -- the bar publishes its height and its lift, and clears when it leaves | none | verified |
| `CHROME-171` | `INV` | Each floating surface of the shell MUST publish the part of the canvas it covers. | this spec section 2; `ambre-spec-zoom.md section 3` (`ZOOM-011`) | not built | unbuilt -- lands with the `The floating cover` Story (`ambre-roadmap.md` V1). `CHROME-170` holds the two surfaces of the board; the format bar of Prose publishes nothing | unimplemented |

### 1.2 Run a command of the status bar

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `CHROME-172` | `FRAME` | A command whose home region is the status bar MUST emit no CRDT delta. | this spec section 2; `ZOOM-001` | not built | unbuilt -- lands with the `Command home regions` Story (`ambre-roadmap.md` V1). `ZOOM-001` holds the zoom alone | unimplemented |

### 1.3 Run a command that writes to the document

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `CHROME-173` | `POST` | One undo MUST take back the whole of a command that writes to the document, whichever region ran it. | this spec section 2; `ambre-spec-undo.md` | not built | unbuilt -- lands with the `Command home regions` Story (`ambre-roadmap.md` V1). Each application holds it for some commands, and no rule reads every region | unimplemented |
| `CHROME-174` | `FRAME` | A command that writes to the document MUST NOT change the zoom. | this spec section 2; `ambre-spec-zoom.md` | not built | unbuilt -- lands with the `Command home regions` Story (`ambre-roadmap.md` V1) | unimplemented |

### 1.4 Collapse and open the context panel

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `CHROME-175` | `POST` | `Ctrl+.` MUST collapse an open context panel to a rail. | this spec section 3 | not built | unbuilt -- lands with the `The context panel and the near-selection bar` Story (`ambre-roadmap.md` V1) | unimplemented |
| `CHROME-176` | `POST` | `Ctrl+.` or a click on the rail MUST open a collapsed context panel. | this spec section 3 | not built | unbuilt -- lands with the `The context panel and the near-selection bar` Story (`ambre-roadmap.md` V1) | unimplemented |
| `CHROME-177` | `FRAME` | Collapsing or opening the context panel MUST NOT write to the document, nor drop the open section, the scroll or the edit in progress of the panel. | this spec section 3 | not built | unbuilt -- lands with the `The context panel and the near-selection bar` Story (`ambre-roadmap.md` V1) | unimplemented |

* **Budget.** none: the regions are DOM, off the frame loop, and `ambre-spec-budgets.md` budgets no
  chrome operation. A cover a surface publishes is read by a fit, which is a zoom cost.
* **Boundary.** none: a region holds commands, and a command reaches the Logic thread as every
  command does. Nothing here opens a socket or holds a key.
* **Verification.** 1 `verified`, 29 `unimplemented`. The board's cover is held by a wasm test of
  each surface; the rest waits on the regions, which no application lays out.

---

## 2. MODEL

**CRDT containers.** none. A region is view state. What a command of the context panel writes is
owned by the feature that declares the command.

**The seven regions, and the question each one answers.** A command goes where its subject is. A
command has one home, so a user who looks for it looks in one place.

| Region | Answers | Writes to the document | Examples |
|---|---|---|---|
| status bar | how I see the document | never (`CHROME-149`) | the zoom, the counters, the sheet tabs of Trame |
| context panel | what the document or the selection is | yes | the page format and the margins of Prose, the sheet of Trame, the board of Toile; the style of a selection |
| near-selection bar | the most frequent actions of the panel, at the pointer | yes | bold, italic, colour, a link |
| creation bar | what I can add | yes | the tools of the board; a picture, a table, a chart, a comment |
| Ambre menu | what I do with the file | through the file | the way home first, then new, open, rename, export, print, the history |
| avatar menu | who I am | never | the account, the theme, the interface language, sign out |
| command palette | every command, by name | as the command does | each command with its chord and its home region (`CHROME-008`) |

**A home is not the only way in.** A chord, the context menu and the command palette reach a command
in any region. The share button of the header reaches a command whose home is the Ambre menu. A
reach never makes a second home, so a command keeps one entry in the region map.

**The near-selection bar is a subset of the panel.** It holds the actions a user repeats most over a
selection, so bold costs no pointer travel. It holds no clipboard command (`CHROME-151`): a chord and
the context menu carry those. Every command it holds is also in the panel (`CHROME-150`).

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
| near-selection bar | floating over the selection | a bar above the keyboard, clear of the system menu (`CHROME-168`) |
| creation bar | floating, centred at the bottom of the content | one add button that opens the modes |
| Ambre menu | the logo, top left of the header | the logo, which also offers the command palette (`CHROME-169`) |
| avatar menu | the avatar, top right of the header | the avatar |
| command palette | `Ctrl+K` | an entry of the Ambre menu |

**A floating surface publishes what it covers.** The near-selection bar, the creation bar and the
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
| a selection made | the near-selection bar floats over it, and the panel shows what applies to it |
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
| A near-selection bar beside the panel | the formatting in the side panel alone | bold and italic are the actions a user repeats most. The pointer would cross the screen to the panel for each one |
| One map for three applications | one chrome per application | a user would learn three places for the same command. Each application keeps its commands, and the map of regions is shared |

---

## 5. NOT BUILT, AND NOT YET RULED

* **The sheet tabs in the status bar are a trial.** A workbook of many sheets may not fit beside the
  counters and the zoom. If the trial fails, the tabs keep a strip of their own, and `CHROME-162`
  changes. No rule bounds the number of tabs the band shows.
* **The board's menu bar holds verified rows.** `TOILE-UI-018`, `TOILE-UI-050`, `TOILE-UI-121` to
  `TOILE-UI-125`, `TOILE-UI-355`, `TOILE-UI-364`, `TOILE-UI-375` and `TOILE-UI-416` to
  `TOILE-UI-418` bind its menus. They stay `verified` while the bar ships. The
  `The Ambre and avatar menus` Story (`ambre-roadmap.md` V1) moves each command to its home and
  rewrites them.
* **Trame's toolbar holds verified rows.** `TRAME-097`, `TRAME-186` and `TRAME-188` name the
  toolbar as the control that reports a style. The
  `The context panel and the near-selection bar` Story (`ambre-roadmap.md` V1) rewrites them
  against the panel.
* **No rule gives the clipboard a home.** Copy writes nothing, and cut and paste write. The three
  live in the context menu and under their chords, and no region holds them yet.
* **No rule lists the creation modes of Prose and Trame.** The board has its tools. A picture, a
  table and a chart are the obvious modes of a page and a sheet, and no row names them.

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
