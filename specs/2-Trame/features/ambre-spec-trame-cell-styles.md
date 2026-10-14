# AMBRE -- FEATURE: CELL STYLES AND THE FORMATTING TOOLBAR

**Type:** Vertical (feature)
**Implementation:** partial -- the store, the toolbar and the borders ship; conditional formatting does not.

> A user selects a range and presses Bold. Every cell in the range takes the run, as one undo step,
> and the toolbar reports what the active cell now carries.

A cell's whole appearance is one value: bold, italic, underline, colours, font, alignment, wrap,
number format and four border edges. One value is the point -- two concurrent restyles then resolve
to one writer's cell, never to a blend neither user asked for.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TRAME-092` | `INV` | A style MUST stay on the cell it was applied to, never following the cursor. | this spec section 3 | `ambre_trame::logic::tests::group1::bold_stays_on_the_cell_not_the_selection` | none | verified |
| `TRAME-186` | `INV` | A cell the toolbar calls a number MUST be a cell the grid aligns as one. | this spec section 6 | `ambre_trame::logic::tests::group1::a_formatted_number_aligns_as_a_number_everywhere` | none | verified |
| `TRAME-095` | `IO` | A style MUST survive a reload. | this spec section 3 | `ambre_trame::logic::tests::group1::formatting_survives_snapshot_roundtrip`; `::set_cell_style_round_trips_and_formats_numbers` | none | verified |
| `TRAME-187` | `IO` | A stored style a reload cannot read MUST NOT come back as the default style with no notice. | this spec section 6 | `ambre_trame::logic::schema::tests::a_register_that_cannot_be_decoded_reaches_the_user` | none | verified |
| `TRAME-097` | `OBS` | The toolbar MUST report the style of the active cell. | this spec section 4 | `ambre_trame::logic::tests::group1::bold_stays_on_the_cell_not_the_selection`; `ambre_trame::tests::chrome_ui::style_applied_mirrors_into_style_state` -- the chrome half | none | verified |
| `TRAME-188` | `OBS` | When the cursor sits inside a merged block, the toolbar MUST report the style the user sees, which is the block's. | this spec section 6 | `ambre_trame::logic::tests::group1::toolbar_reports_the_block_style_inside_a_merge` | none | verified |
| `TRAME-100` | `OBS` | A border a user asked for MUST NOT be overpainted by a grid line. | this spec section 4 | `ambre_trame::ui::renderer::tests::a_border_is_drawn_after_the_grid_and_the_merges` -- the first border stroke follows the last grid line and the merge repaint | none | verified |
| `TRAME-101` | `OBS` | A border the user did not colour MUST follow the active theme. | this spec section 6 | `ambre_trame::ui::renderer::tests::uncoloured_border_follows_the_theme` | none | verified |

### 1.2 Format the selected cells

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TRAME-189` | `PRE` | A formatting gesture MUST carry at least one attribute the user changed. | this spec section 6 | `ambre_trame::logic::tests::group13::an_empty_style_patch_writes_nothing` -- no operation, no delta, and the undo after it takes back the edit before it | none | verified |
| `TRAME-093` | `POST` | Styling a selection MUST be one undo item, never one per cell. | this spec section 3 | `ambre_trame::logic::tests::group1::selection_style_is_a_single_undo` | none | verified |
| `TRAME-094` | `POST` | Styling a selection MUST reach a peer as one delta that carries every cell. | this spec section 3 | `ambre_trame::logic::tests::group7::batched_formatting_broadcasts_complete_delta` | none | verified |
| `TRAME-096` | `POST` | A cell returned to the default style MUST leave no style entry behind. | this spec section 6 | `ambre_trame::logic::tests::group13::a_cell_back_to_the_default_style_leaves_no_entry` | none | verified |
| `TRAME-190` | `POST` | A border a user drew MUST reach the document and come back out to the grid. | this spec section 4 | `ambre_trame::logic::convergence_tests::concurrent_whole_cell_style_edits_do_not_blend_across_containers`; `::subset_style_edit_does_not_tear_a_whole_cell_rewrite` -- both carry a border preset through the gesture and compare the four resulting edges after a document exchange and a viewport read | none | verified |
| `TRAME-099` | `POST` | A border preset MUST reach only the edges that preset names. | this spec section 4 | `ambre_trame::logic::tests::group13::a_border_preset_reaches_only_its_edges` -- the ten presets over a 3x3 selection, edge by edge | none | verified |
| `TRAME-098` | `FRAME` | A cell a merge covers MUST NOT take a style of its own. | this spec section 3 | `ambre_trame::logic::tests::group13::a_covered_cell_takes_no_style` | none | verified |
| `TRAME-191` | `FRAME` | Styling a selection MUST NOT change the value of any cell in it. | this spec section 2 | `ambre_trame::logic::tests::group13::styling_keeps_every_value` | none | verified |
| `TRAME-192` | `FRAME` | Styling a selection MUST NOT move the cursor or change what is selected. | this spec section 4 | `ambre_trame::logic::tests::group13::styling_keeps_the_selection` | none | verified |
| `TRAME-193` | `FAIL` | Given a cell the document cannot hold, a formatting gesture MUST tell the user, and MUST NOT draw a style it did not store. | this spec section 6 | `ambre_trame::logic::tests::group1::style_beyond_the_cap_is_refused_and_not_drawn` | none | verified |

### 1.3 Recolour a cell from a rule

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TRAME-FEAT-017` | `POST` | Conditional formatting MUST apply the first matching rule in ascending priority order, and stop there. | this spec section 3 | not built | unbuilt -- lands with the `Data tools` Story (`ambre-roadmap.md` E6) | unimplemented |
| `TRAME-FEAT-018` | `POST` | A conditional format MUST produce a display-time override of the cell's style. | this spec section 3 | not built | unbuilt -- lands with the `Data tools` Story (`ambre-roadmap.md` E6) | unimplemented |
| `TRAME-FEAT-019` | `POST` | A `Formula` conditional rule MUST resolve its relative references from the cell being evaluated, not from the rule's anchor. | this spec section 3 | not built | unbuilt -- lands with the `Data tools` Story (`ambre-roadmap.md` E6) | unimplemented |
| `TRAME-FEAT-028` | `FRAME` | A conditional format MUST NOT change the cell's stored style. | this spec section 3; section 5 | not built | unbuilt -- lands with the `Data tools` Story (`ambre-roadmap.md` E6). It is the half `TRAME-FEAT-018` was carrying, and the one the "a style is stored, a conditional format is not" decision rests on | unimplemented |

* **Budget.** none: a restyle is one LWW register write, and `ambre-spec-budgets.md`
  budgets no style operation. What the styles add to memory sits inside the `LoroDoc` holder of
  `ambre-spec-budgets.md section 6`.
* **Boundary.** none of its own: a style is a document write, and `ZK-001` seals every payload
  before it leaves the client.
* **Verification.** 18 `verified`, 4 `unimplemented`. What holds is the single-register model:
  where a style lives, how it undoes, how it broadcasts, how it reloads and what the toolbar reads.
  The gesture around it is held too. Each preset reaches its own edges over a 3x3 range, a covered
  cell takes no style, and a run keeps every value and the selection.

---

## 2. WHAT EACH USER SEES

| A does | B does | Converged document | What each user sees | Test |
|---|---|---|---|---|
| bolds `B2` | fills `B2` red | one whole style, elected by LWW | one of the two styles on both screens, never bold *and* red (`TRAME-016`) | `ambre_trame::logic::convergence_tests::concurrent_whole_cell_style_edits_do_not_blend_across_containers` |
| restyles `A1:C3` | restyles `B1:B5` | each cell one writer's | the overlap split cell by cell between them; every single cell internally whole | `ambre_trame::logic::convergence_tests::subset_style_edit_does_not_tear_a_whole_cell_rewrite` |
| bolds `B2` | types `9` into `B2` | both: text and style are separate registers | the new value, in bold | the same property, whose op set carries `SetCellText` and `SetCellStyle` |
| bolds `B2` | deletes row 2 | no cell, and no style | the row gone on both screens, its styles purged (`TRAME-082`) | none -- the purge covers styles, the axis metrics it leaves are section 6 there |

A selection is not the unit of convergence: `SetCellStyle` writes one register per cell. Two
overlapping restyles therefore split the overlap rather than elect one selection. Restyling reconciles it.

---

## 3. MODEL

**Containers.** `cellstyle_{sheet}`, declared in `ambre-spec-trame-schema.md section 3`. The key is
the row and column uid pair, the value is one postcard-encoded record, and that record is the LWW
unit (`TRAME-016`).

**Shapes** (`ambre-bus`, plus the private record in `logic/styles.rs`):

```rust
pub struct CellStyle {
    pub underline: bool,
    pub strikethrough: bool,
    pub h_align: HAlign,          // Auto | Left | Center | Right
    pub v_align: VAlign,          // Top | Middle | Bottom
    pub wrap: bool,
    pub fg: Option<u32>,          // 0xRRGGBB; None = the theme default
    pub bg: Option<u32>,          // 0xRRGGBB; None = transparent
    pub font: Option<String>,
    pub size: Option<u16>,
    pub format: CellFormat,       // `features/ambre-spec-trame-number-formats.md`
    pub decimals: Option<u8>,
    pub borders: Borders,         // four independent Option<CellBorder> edges
}

struct StyleRecord {              // what the register actually holds
    bold: bool,                   // bold and italic ride here, not in CellStyle
    italic: bool,
    style: CellStyle,
}

pub struct StylePatch {           // one toolbar control, one sparse edit
    pub bold: Option<bool>,
    pub italic: Option<bool>,
    pub underline: Option<bool>,
    pub strikethrough: Option<bool>,
    pub h_align: Option<HAlign>,
    pub v_align: Option<VAlign>,
    pub wrap: Option<bool>,
    pub fg: Option<Option<u32>>,       // None leaves, Some(None) clears, Some(Some) sets
    pub bg: Option<Option<u32>>,
    pub font: Option<Option<String>>,
    pub size: Option<Option<u16>>,
    pub format: Option<CellFormat>,
    pub decimals: Option<Option<u8>>,
    pub border: Option<BorderEdit>,    // a preset and a pen, resolved per cell
}
```

**Commands and events** (`ambre-bus`):

| Shape | Direction | Carries |
|---|---|---|
| `TrameCommand::SetCellStyle { patch }` | UI to Logic | a sparse `StylePatch`. Every `None` field leaves the cell's value alone |
| `TrameEvent::StyleApplied { style, bold, italic, merged, numeric }` | Logic to UI | the active cell's resolved state, for the toolbar controls |
| `TrameEvent::DataReset` | Logic to UI | emitted beside it, so the viewport re-reads the restyled cells |

A colour, a font, a size and a decimal count each carry three intents: `None` leaves the field,
`Some(None)` clears it, `Some(Some(v))` sets it. No other field needs three states.

**A restyle, in steps:**

1. Every targeted cell is materialized first, so the scaffolding commits land before the run.
2. One undo group opens over the whole selection.
3. Each cell not covered by a merge takes the patch; a border edit resolves to that cell's edges.
4. A cell whose style is now the default is dropped from the map instead of stored (`TRAME-096`).
5. One commit closes the run, so it is one undo item and one broadcast delta.

**Code.** `logic/styles.rs` -- `set_cell_style`, `write_cell_style`, `reseed_styles`,
`style_state_event`; `logic/packing.rs` -- `apply_patch_to_style`; `chrome/bars.rs` -- the toolbar;
`render/content.rs` -- the border pass.

**Conditional formatting, decided and not built.** Its container is `cond_formats_{sheet}`, declared
in `ambre-spec-trame-schema.md section 3`: an ordered list of rules, each carrying a range, a
priority, a kind and the style it applies.

```rust
pub enum CondFormatKind {
    GreaterThan(String), LessThan(String), Between(String, String), Equal(String),
    Contains(String), StartsWith(String),
    Formula(String),                          // relative to the cell tested (`TRAME-FEAT-019`)
    IsEmpty, IsNotEmpty, IsError,
    ColorScale2 { min_color: u32, max_color: u32 },
    ColorScale3 { min_color: u32, mid_color: u32, max_color: u32 },
    DataBar { color: u32 },
    TopN(u32), BottomN(u32), TopNPercent(f32),
}
```

| Kind | What it reads | What a mismatch means |
|---|---|---|
| the four comparisons | the cell's value, and an operand that may itself be a reference | a non-match, never an error |
| `Contains`, `StartsWith` | the displayed text | a non-match |
| `Formula` | the rule compiled in the tested cell's own context; truthy is `true` or non-zero | a non-match |
| `IsEmpty`, `IsNotEmpty`, `IsError` | the cached value directly | -- |
| the scales, the bar, the top-N | the whole range's extent or ranking, computed once per rule | -- |

The result is an override merged over the stored style, field by field, with the first matching rule
in ascending priority order winning (`TRAME-FEAT-017`, `TRAME-FEAT-018`).

**When a rule runs.** Every recompute of the dirty set is followed by a pass over the rules
covering the visible cells. A value change and the colour it triggers therefore reach the user in
one frame. A cell whose override changed while its value did not is still packed into the viewport.
The UI holds no rule and cannot derive the change (`TRAME-UI-001`).

---

## 4. SURFACE

**The toolbar**, a DOM bar above the formula bar. Every control writes the current selection.

| Control | Emits | Reads back from |
|---|---|---|
| Bold, Italic | a `CommandId` on the shared command path, toggled against the reported state | `StyleApplied.bold`, `.italic` |
| Underline, Strikethrough, Wrap | a `StylePatch` with that one field inverted | the matching `CellStyle` field |
| font family, font size | a `StylePatch`; the pickers are the shared controls of `ambre-spec-chrome.md`, which Toile's inspector also uses | `CellStyle.font`, `.size` |
| text colour, fill colour | a `StylePatch` colour, or a reset to the theme default | `CellStyle.fg`, `.bg` |
| horizontal, vertical alignment | a `StylePatch` alignment, from a popover of the three variants | `CellStyle.h_align`, `.v_align` |
| number format, decimals | `features/ambre-spec-trame-number-formats.md section 3` | `CellStyle.format`, `.decimals` |
| borders | a `BorderEdit`: one preset, one pen (style and colour) | nothing; the control is stateless |
| merge, undo, redo | the shared `CommandId` for each | `StyleApplied.merged`, and the undo stack state |

**Border presets, and the edges each takes.** A preset is additive: it sets its own edges and leaves
the others. `None` is the exception and clears all four.

| Preset | Edges it reaches |
|---|---|
| `All` | all four edges of every selected cell |
| `Outline` | only the edges on the perimeter of the selection rectangle |
| `Inner` | only the edges shared with another selected cell |
| `InnerHorizontal`, `InnerVertical` | the shared edges on one axis |
| `Top`, `Bottom`, `Left`, `Right` | that one perimeter edge |
| `None` | all four, cleared |

A merge master resolves its perimeter against its whole span, not its top-left cell. An `Outline`
over a selection containing a merge therefore follows the block (`TRAME-099`).

**Painting.** A border is drawn after the grid lines, per edge, in its own style and colour
(`TRAME-100`). `Double` is two thin parallel lines; `Dashed` and `Dotted` are runs of short
segments, because the shared surface carries no dash parameter.

---

## 5. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| One register per cell holds the whole style | eleven per-sheet registers, one per attribute | it is what the code carried: the fan-out tore a border's presence from its colour, and dropped a colour-only cell on reload. One register makes a concurrent restyle race exactly once. The rule is cross-app and normative in `ambre-spec-architecture.md section 2`; Toile's elements follow it. |
| Concurrency is whole-cell last-writer-wins | faking a per-attribute merge | it would restore the fan-out that was removed. A real per-attribute merge needs a server that can order plaintext content; a blind relay never sees any, so the only ordering available is per-register. |
| A style is stored, a conditional format is not | writing the computed colour into the cell's style | a rule that recolours on a value must recompute when the value changes; storing its result would make the document depend on evaluation order. |
| Bold and italic travel as commands | a fourteenth `StylePatch` field for each | it duplicates the command path they already have: they are the two attributes with a keyboard chord and a shared toolbar toggle, so they already carry a `CommandId`. The record still stores them beside the rest. |
| The toolbar reflects the active cell | resolving a common state over the selection | it needs a tri-state on every control, for a selection of mixed styles that has no single state to show. Excel and Sheets both answer with the anchor, and one cell is also one lookup. |
| A covered cell takes no style | styling every cell in the rectangle, master or not | a covered cell renders blank under its master, so a style on it is invisible -- and an `All` border on it would draw edges *inside* the merged block. |
| A shared style dictionary is refused | a `styles` map with an `"s"` indirection from each cell | deduplication costs a dereference per read and needs a reclamation policy, because Loro never collects an orphaned entry. |

---

## 6. NOT BUILT, AND NOT YET RULED

* **Conditional formatting is decided and unbuilt** (`TRAME-FEAT-017`, `-018`, `-019`). The rules,
  the kinds and the override are section 3; no handler evaluates one. The three gradient kinds --
  colour scales, data bars and top-N -- need a pass over the whole range rather than one cell. They
  wait a Story longer, on "Data tools, second pass".
* **A user fill colour is not a palette colour, and `TRAME-UI-012` says every fill must be.** That
  row is `verified`, by a test whose fixture holds one unstyled cell. The rule needs the exception
  written into it: a colour the *user* chose is not a hardcoded product colour.
* **A cell back to the default keeps its lines.** The run materializes every row and column its
  selection reaches before it writes the style. So `TRAME-096` is narrower than it reads: the style
  entry goes, and the rows and columns the cell needed stay.
* **A style has no clear-formatting command.** A user restores the default by reversing every
  control that was used. `write_cell_style` already drops a defaulted cell, so the store is ready
  for a command that does not exist.
