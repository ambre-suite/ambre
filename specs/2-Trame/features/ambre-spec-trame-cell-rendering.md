# AMBRE -- FEATURE: CELL RENDERING (ALIGNMENT, WRAP AND OVERFLOW)

**Type:** Vertical (feature)
**Implementation:** partial -- sparklines and charts are not drawn, and the automatic alignment
misreads a formatted number (section 5).

> A label wider than its column runs over the empty cells beside it. Put a value in the next cell
> and the label stops at its own edge, because something is now there to hide.

Where a cell's text lands inside its box. Which way it is aligned, whether it breaks into lines,
how far it may run past its column, and where it is cut. All of it is UI-thread work on the
`DisplayViewport` alone -- no document, no round trip (`TRAME-UI-001`).

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`.

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TRAME-UI-093` | `OBS` | A cell the user did not align MUST put a number right and anything else left. | this spec section 3 | `ambre_trame::logic::tests::group1::a_formatted_number_aligns_as_a_number_everywhere` -- the `numeric` flag the pack hands `cell_alignment` | none | verified |
| `TRAME-UI-094` | `OBS` | The alignment control MUST resolve an automatic alignment the way the renderer does. | this spec section 3 | `ambre_trame::logic::tests::group1::a_formatted_number_aligns_as_a_number_everywhere` -- the toolbar event against the pack | none | verified |
| `TRAME-UI-095` | `OBS` | A cell that does not wrap MUST spill only over neighbours that are empty. | this spec section 3 | `ambre_trame::render::tests::spill_width_stops_at_first_non_empty_neighbour` -- over an occupied neighbour, an immediate one and the grid edge | none | verified |
| `TRAME-UI-096` | `OBS` | A spill MUST run in the direction the text is aligned in. | this spec section 3 | none | unheld -- the three branches, left spills right, right spills left and centred both ways, are in `draw_cells_on`, and no test enters any of them; lands with the `The Trame grid on screen held` Story (`ambre-roadmap.md` E9) | untested |
| `TRAME-UI-097` | `OBS` | A cell that wraps MUST stay inside its own box. | this spec section 3 | none | unheld -- wrap excludes the cell from the spill branch and clips it to its own rect. The one test that draws a cell draws one that fits, so it reaches neither half; lands with the `The Trame grid on screen held` Story (`ambre-roadmap.md` E9) | untested |
| `TRAME-UI-098` | `OBS` | Text taller than its row MUST be cut, never painted over the row below. | this spec section 3 | none | unheld -- the test this row once cited asserts that the clip stack unwinds, and its cell fits its box, so no clip is pushed and the assertion holds over an empty stack; lands with the `The Trame grid on screen held` Story (`ambre-roadmap.md` E9) | untested |
| `TRAME-UI-099` | `OBS` | A vertical alignment MUST NOT lift text above the top of its cell. | this spec section 3 | none | unheld -- `cell_v_offset` clamps the offset at zero, and no test passes it text taller than the box, though it is a free function the render tier already reaches, like `spill_width`; lands with the `The Trame grid on screen held` Story (`ambre-roadmap.md` E9) | untested |
| `TRAME-UI-100` | `OBS` | Glyphs MUST scale with the zoom, like the row heights and column widths. | this spec section 3 | none | unheld -- the font size is multiplied by the layout zoom in `draw_cells_on`, and every render fixture builds a layout at zoom one; lands with the `The Trame grid on screen held` Story (`ambre-roadmap.md` E9) | untested |
| `TRAME-UI-139` | `OBS` | A fill and a border MUST be cut at the cell's own edge, whatever its text does. | this spec section 3 | none | unheld -- the fill takes the cell rect and is emitted before the spill clip is pushed, so only glyphs cross a column edge, and the one fill test counts the fills and reads neither rect; lands with the `The Trame grid on screen held` Story (`ambre-roadmap.md` E9) | untested |
| `TRAME-UI-020` | `INV` | A sparkline MUST be derived on the UI thread from the viewport data alone, with no round trip to the Logic thread. | this spec section 5 | not built | unbuilt -- lands with the `Post-V1 surface` Story (`ambre-roadmap.md` post-V1) | unimplemented |
| `TRAME-UI-021` | `OBS` | A chart MUST be clipped to the cell range that hosts it. | this spec section 5 | not built | unbuilt -- lands with the `Post-V1 surface` Story (`ambre-roadmap.md` post-V1) | unimplemented |
| `TRAME-UI-140` | `OBS` | A chart MUST NOT paint outside the grid. | this spec section 5 | not built | unbuilt -- lands with the `Post-V1 surface` Story (`ambre-roadmap.md` post-V1) | unimplemented |

* **Budget.** the frame phases of `ambre-spec-budgets.md section 2` -- the text layout, the
  vertex batch build and the GPU pass. `CANVAS-005` is the rule that holds them, and nothing
  measures it.
* **Boundary.** none: the renderer paints view state on the UI thread, where `UI-THREAD-004` bans a
  network, crypto or persistence path.
* **Verification.** 3 `verified`, 6 `untested`, 3 `unimplemented`. What holds is the one rule
  extracted into a pure function of a layout and an emptiness predicate. What is loose is everything
  else, inside `draw_cells_on`, where the render tests reach the drawing and read none of its
  decisions back. The clip a too-wide text pushes is among them. The two alignment rows hold: the
  pack decides once, on the **computed** value, and the renderer and the toolbar read that answer.

---

## 2. MODEL

**Containers.** None. Every field arrives in the `DisplayViewport` the Logic thread packs.

**Shapes** (`ambre-trame/src/render`):

```rust
pub struct CellContent<'a> {      // one cell, as the renderer sees it
    pub text: &'a str,            // already number-formatted by the Logic thread
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strikethrough: bool,
    pub h_align: HAlign,          // Auto resolves against `text`
    pub v_align: VAlign,
    pub fg: Option<u32>,          // None = the palette's cell text colour
    pub bg: Option<u32>,          // None = the palette's content background
    pub font: Option<&'a str>,
    pub size: Option<u16>,        // points, before zoom
    pub wrap: bool,
    pub borders: Borders,
}
```

Both the screen cache and a paginated surface map onto this type, so the drawing primitives depend
on neither. The same code path therefore serves the canvas and, in time, a printed page.

**The drawing module** is `crates/ambre-trame/src/render/`, inside the application crate rather than
a UI crate of its own. The Logic thread therefore reaches the same walk for a PDF page. Every entry
point is generic over the `DrawSurface` of `ambre-spec-drawing-surface.md section 2`, and the caller
chooses the implementation. The UI thread hands it a `WgpuSurface` in its frame, the PDF path a
`KrillaSurface` (`ambre-spec-pdf-export.md`, `PDF-001`).

| Entry point | Draws |
|---|---|
| `draw_cells_on` | the fills, the glyphs and the spill of a viewport's cells |
| `draw_merges_on` | the same layout over a merge master's whole span |
| `draw_grid_lines_on` | the column and row rules of the layout |
| `draw_col_headers_on`, `draw_row_headers_on` | the two header strips |

`draw_cells_on` skips a cell a merge covers, resolves each rectangle through the layout, and emits
the same calls whichever surface it holds.

**Code.** `render/cells.rs` -- `draw_cells_on` and `spill_width`; `render/content.rs` --
`cell_alignment`, `cell_v_offset`, `draw_cell_borders_on`; `render/merges.rs`, `render/grid.rs`,
`render/headers.rs` -- the three entry points above; `render/text.rs` -- the shared glyph runs.

---

## 3. SURFACE

**Laying out one cell**, in steps:

1. The cell is filled: its own colour if it carries one, the palette's content background otherwise.
2. Wrap decides the layout width: the cell's content box when it wraps, unbounded when it does not.
3. The glyph size is the cell's point size, or the grid default, multiplied by the zoom
   (`TRAME-UI-100`).
4. The text is always shaped left-aligned; the renderer then positions the run itself.
5. The vertical offset follows the vertical alignment, clamped so it never goes negative
   (`TRAME-UI-099`).
6. A line wider than its box either spills or is cut, per the table below.

**Alignment.**

| Setting | Where the text sits |
|---|---|
| automatic | right for a value that parses as a number, left for anything else (`TRAME-UI-093`) |
| left, centre, right | there, inside the cell's content box |
| top, middle, bottom | there, inside the cell's height, never above its top (`TRAME-UI-099`) |

**Overflow.** Only a single unwrapped line may spill. The walk stops at the first neighbour that
holds text, at the grid edge, or once the overflow is covered.

| The cell | What happens |
|---|---|
| fits its column | drawn in place, no clip |
| does not wrap, one line, too wide | spills over the empty neighbours in its alignment's direction (`TRAME-UI-095`, `TRAME-UI-096`) |
| does not wrap, and the next neighbour holds text | cut at its own edge, with no ellipsis |
| wraps | broken into lines inside its own box, and cut there (`TRAME-UI-097`) |
| holds newlines, or wraps taller than its row | cut at the row's bottom edge (`TRAME-UI-098`) |

A fill and a border are always cut to the owning cell: only glyphs ever cross a column edge
(`TRAME-UI-139`).

**Zoom** scales the glyph size the same way the per-axis metrics scale the column widths and row
heights. A zoomed grid is the same layout, not a different one.

---

## 4. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| Text spills over empty neighbours | cutting at the column edge and marking the cut | an ellipsis would hide text that has room to be read. A spreadsheet's labels are routinely wider than their column, and every spreadsheet a user has met spills them. |
| The renderer shapes left-aligned and positions the run itself | letting the text engine align inside a bounded layout | the text engine's own alignment needs a finite advance, and the unbounded layout an overflowing cell needs has none -- centred text would be pushed off the surface. |
| A row's height is measured on the Logic thread | remeasuring wrapped text on the UI thread each frame | the height belongs to the axis, which is document state shared with the header strip and the resize drag; a height measured on the UI thread would be a second answer. |
| Only a single line may spill | spilling every line of a wrapped cell | multi-line text already claims vertical room its neighbours do not expect; letting it claim horizontal room too makes the overlap unbounded in both axes. |
| Colour is resolved late, from the palette | resolving the theme colour into the cell when it is stored | a cell carries a colour or it carries nothing, and nothing must mean the current theme's colour. Stored, it freezes one theme into the document, so a theme switch would need a document write over every cell. |

---

## 5. NOT BUILT, AND NOT YET RULED

* **A wrapped cell gets no more height.** Wrap is honoured, the lines are laid out, and the row
  stays one line tall, so everything past the first line is cut. The user has to drag the row taller
  by hand. Auto-height is named as a Logic-thread concern and no Story schedules it.
* **Sparklines** (`TRAME-UI-020`). A chart inside a cell, derived from the viewport's own numbers.
  Neither the cell syntax nor the style field that would mark one exists, and nothing draws one.
* **Charts** (`TRAME-UI-021`). A float object over the grid, its definition stored in the document
  and its geometry bounded before it is written. Trame has no float-object plane in the code, so the
  bound and the drawing are both open.
* **A number too wide for its column is spilled or cut like a label.** A spreadsheet shows a row of
  hashes instead, because a cut number reads as a smaller number. Trame has no such marker
  (`features/ambre-spec-trame-number-formats.md section 5`).

  One parse read by both sides, on one text, is the fix. Which text is the open question, and
  `features/ambre-spec-trame-number-formats.md section 5` states the same split from the format's
  side.
* **A merged block never spills.** `draw_merges_on` clips every master to its span rect. A label
  wider than its merge is cut where a label wider than its column would run on. Nothing here or in
  `features/ambre-spec-trame-cell-merges.md` says whether that is the intent.
* **The style surface has no test.** Nine rows above say `none`, and all nine are reachable from the
  render tests that already exist. Those tests build a font context, a recording surface and a grid
  layout. They only ever draw one plain cell that fits its box at zoom one.
