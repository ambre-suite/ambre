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

| ID | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|
| `TRAME-092` | A style MUST stay on the cell it was applied to, and MUST NOT follow the cursor. | this spec section 3 | `ambre_trame::logic::tests::group1::bold_stays_on_the_cell_not_the_selection` | verified |
| `TRAME-093` | Styling a selection MUST be one undo item, never one per cell. | this spec section 3 | `ambre_trame::logic::tests::group1::selection_style_is_a_single_undo` | verified |
| `TRAME-094` | Styling a selection MUST reach a peer as one delta that carries every cell. | this spec section 3 | `ambre_trame::logic::tests::group7::batched_formatting_broadcasts_complete_delta` | verified |
| `TRAME-095` | A style MUST survive a reload. | this spec section 3 | `ambre_trame::logic::tests::group1::formatting_survives_snapshot_roundtrip`, `::set_cell_style_round_trips_and_formats_numbers` | verified |
| `TRAME-096` | A cell returned to the default style MUST leave no entry behind. | this spec section 6 | none. `write_cell_style` deletes the key when the record equals the default, and no test reads the container back to see it gone | unverified |
| `TRAME-097` | The toolbar MUST report the style of the active cell. | this spec section 4 | `ambre_trame::logic::tests::group1::bold_stays_on_the_cell_not_the_selection`; the chrome half is `ambre_trame::tests::chrome_ui::style_applied_mirrors_into_style_state` | verified |
| `TRAME-098` | A cell a merge covers MUST NOT take a style of its own. | this spec section 3 | none. `set_cell_style` skips every cell in `covered`, and nothing drives a restyle across a merge | unverified |
| `TRAME-099` | A border preset MUST reach only the edges that preset names. | this spec section 4 | none. The ten presets resolve to per-cell edges in `logic/styles.rs`, and no test applies one | unverified |
| `TRAME-100` | A border a user asked for MUST NOT be overpainted by a grid line. | this spec section 4 | none. The border pass runs after the grid lines in `draw_grid`, which is a call order nothing asserts | unverified |
| `TRAME-101` | A border the user did not colour MUST follow the active theme. | this spec section 6 | none, and the code does the opposite: `BORDER_DEFAULT` in `render/content.rs` is one literal grey for both themes | unverified |
| `TRAME-FEAT-017` | Conditional formatting MUST apply the first matching rule in ascending priority order, and stop there. | this spec section 3 | not built -- lands with the "Data tools" Story (`ambre-roadmap.md` E6) | unimplemented |
| `TRAME-FEAT-018` | A conditional format MUST produce a display-time `style_override` and MUST NOT mutate the stored cell style. | this spec section 3 | not built -- lands with the "Data tools" Story (`ambre-roadmap.md` E6) | unimplemented |
| `TRAME-FEAT-019` | A `Formula` conditional rule MUST resolve its relative references from the cell being evaluated, not from the rule's anchor. | this spec section 3 | not built -- lands with the "Data tools" Story (`ambre-roadmap.md` E6) | unimplemented |

* **Budget.** none: a restyle is one LWW register write, and `ambre-spec-budgets.md`
  budgets no style operation. What the styles add to memory sits inside the `LoroDoc` holder of
  `ambre-spec-budgets.md section 6`.
* **Boundary.** none of its own: a style is a document write, and `ZK-001` seals every payload
  before it leaves the client.
* **Verification.** 5 `verified`, 5 `unverified`, 3 `unimplemented`. The five that hold are the
  five the single-register model was built to hold: where a style lives, how it undoes, how it
  broadcasts, how it reloads, what the toolbar reads. Every rule about **borders** is loose, and
  one of them -- the default border colour -- is a rule the code is known to break. The convergence
  rule itself is `TRAME-016`, in `ambre-spec-trame-schema.md section 1`.

---

## 2. WHAT EACH USER SEES

| A does | B does | Converged document | What each user sees | Test |
|---|---|---|---|---|
| bolds `B2` | fills `B2` red | one whole style, elected by LWW | one of the two styles on both screens, never bold *and* red (`TRAME-016`) | `ambre_trame::logic::convergence_tests::concurrent_whole_cell_style_edits_do_not_blend_across_containers` |
| restyles `A1:C3` | restyles `B1:B5` | each cell one writer's | the overlap split cell by cell between them; every single cell internally whole | `ambre_trame::logic::convergence_tests::subset_style_edit_does_not_tear_a_whole_cell_rewrite` |
| bolds `B2` | types `9` into `B2` | both: text and style are separate registers | the new value, in bold | the same property, whose op set carries `SetCellText` and `SetCellStyle` |
| bolds `B2` | deletes row 2 | no cell, and no style | the row gone on both screens, its styles purged (`TRAME-082`) | none -- the purge covers styles, the axis metrics it leaves are section 6 there |

A selection is not the unit of convergence: `SetCellStyle` writes one register per cell, so two
overlapping restyles split the overlap rather than elect one selection. Restyling reconciles it.

---

## 3. MODEL

**Containers.** `cellstyle_{sheet}`, declared in `ambre-spec-trame-schema.md section 3`: the key is
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

**When a rule runs.** Every recompute of the dirty set is followed by a pass over the rules covering
the visible cells, so a value change and the colour it triggers reach the user in one frame. A cell
whose override changed while its value did not is still packed into the viewport, because the UI
holds no rule and cannot derive the change (`TRAME-UI-001`).

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

A merge master resolves its perimeter against its whole span, not its top-left cell, so an `Outline`
over a selection containing a merge follows the block (`TRAME-099`).

**Painting.** A border is drawn after the grid lines, per edge, in its own style and colour
(`TRAME-100`). `Double` is two thin parallel lines; `Dashed` and `Dotted` are runs of short
segments, because the shared surface carries no dash parameter.

---

## 5. DECISIONS

| Decision | Rationale | Rejected alternative |
|---|---|---|
| One register per cell holds the whole style | a fan-out over per-attribute registers tore a border's presence from its colour, and dropped a colour-only cell on reload. One register makes a concurrent restyle race exactly once. The rule is cross-app and normative in `ambre-spec-architecture.md section 2`; Toile's elements follow it | eleven per-sheet registers, one per attribute, which is what the code carried |
| Concurrency is whole-cell last-writer-wins | per-attribute merge needs a server that can order plaintext content; a blind relay never sees any, so the only ordering available is per-register | faking a per-attribute merge, which would restore the fan-out that was removed |
| A style is stored, a conditional format is not | a rule that recolours on a value must recompute when the value changes; storing its result would make the document depend on evaluation order | writing the computed colour into the cell's style |
| Bold and italic travel as commands, not as patch fields | they are the two attributes with a keyboard chord and a shared toolbar toggle, so they already have a `CommandId`; the record still stores them beside the rest | a fourteenth `StylePatch` field for each, duplicating the command path |
| The toolbar reflects the active cell, not the selection | a selection of mixed styles has no single state to show, and Excel and Sheets both answer with the anchor. One cell is also one lookup | resolving a common state over the selection, which needs a tri-state on every control |
| A covered cell takes no style | it renders blank under its master, so a style on it is invisible -- and an `All` border on it would draw edges *inside* the merged block | styling every cell in the rectangle, master or not |
| A shared style dictionary is refused | deduplication costs a dereference per read and needs a reclamation policy, because Loro never collects an orphaned entry | a `styles` map with an `"s"` indirection from each cell |

---

## 6. NOT BUILT, AND NOT YET RULED

* **Conditional formatting is decided and unbuilt** (`TRAME-FEAT-017`, `-018`, `-019`). The rules,
  the kinds and the override are section 3; no handler evaluates one. The three gradient kinds --
  colour scales, data bars and top-N -- need a pass over the whole range rather than one cell, and
  wait a Story longer, on "Data tools, second pass".
* **A border has one colour in both themes.** `BORDER_DEFAULT` is a literal mid grey, chosen to sit
  darker than the light-theme grid line. On a dark theme it is darker than the background
  (`TRAME-101`). It is also the one colour in the cell renderer that does not come from the palette.
* **A user fill colour is not a palette colour, and `TRAME-UI-012` says every fill must be.** That
  row is `verified`, by a test whose fixture holds one unstyled cell. The rule needs the exception
  written into it: a colour the *user* chose is not a hardcoded product colour.
* **Nothing tests a border.** Three of the five `unverified` rows are the border model -- the preset
  resolution, the paint order, the default colour. The preset resolution is a pure function of a
  rectangle and a merge map, so it is the cheapest of the three to close.
* **A style has no clear-formatting command.** A user restores the default by reversing every
  control that was used. `write_cell_style` already drops a defaulted cell, so the store is ready
  for a command that does not exist.
