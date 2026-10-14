# AMBRE -- FEATURE: NUMBER FORMATS

**Type:** Vertical (feature)
**Implementation:** partial -- nine families exist; two of them render their raw serial.

> A user picks Percent. The cell shows `314.2%` and still holds `3.14159`, so a formula reading it
> reads the number, not the text a reader sees.

A format is a field of the cell style, applied once per viewport pack. It owns no container and no
command. What it owns is the mapping from a value to the text a reader sees, and the rule that the
two never swap places. The register, the toolbar and the convergence of a format change are
`features/ambre-spec-trame-cell-styles.md`.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TRAME-102` | `INV` | A format MUST NOT change what a cell holds. | this spec section 2 | type-level -- `format_display` takes `&self` and returns a `String`, and its one call site is the viewport packing loop, so no formatted text can reach `cells_{sheet}` | none | verified |
| `TRAME-157` | `OBS` | A formatted cell MUST show the text its family renders for the value it holds. | this spec section 2 | `ambre_trame::logic::tests::group1::set_cell_style_round_trips_and_formats_numbers` -- it reads `314.2%` off the packed viewport for a cell holding `3.14159` under Percent with one decimal, one family of the nine and the only test that reaches `format_display` at all | none | verified |
| `TRAME-103` | `OBS` | A value that is not a number MUST be shown unchanged, whatever format the cell carries. | this spec section 2 | `ambre_trame::logic::tests::group13::a_word_ignores_its_number_format` | none | verified |
| `TRAME-104` | `OBS` | A decimal count MUST be offered only for a family that renders decimal places. | this spec section 3 | `ambre_bus::has_decimals_is_true_only_for_numeric_families` -- exhaustive over the ten families, the predicate's one use being the `disabled` attribute of the two buttons, so it guards the control and never the write (section 5) | none | verified |
| `TRAME-106` | `OBS` | A fraction MUST keep the sign of a value below one. | this spec section 2 | `ambre_trame::logic::packing::tests::fraction_format_keeps_the_sign_and_carries` -- the sign half. It calls the renderer directly, not through a packed cell | none | verified |
| `TRAME-223` | `OBS` | A fraction MUST NOT print a whole as sixteenths. | this spec section 2 | `ambre_trame::logic::packing::tests::fraction_format_keeps_the_sign_and_carries` -- the carry half, on the same direct call | none | verified |
| `TRAME-107` | `OBS` | A date-formatted cell MUST show a calendar date, never the day serial behind it. | this spec section 5 | `ambre_trame::logic::tests::group1::date_and_time_cells_show_the_calendar`; `ambre_trame::logic::number_format::tests::date_and_time_render_the_calendar` | none | verified |
| `TRAME-108` | `OBS` | A formatted value MUST read the same everywhere it is shown as text. | this spec section 5 | `ambre_trame::logic::tests::group7::find_matches_the_formatted_value`; `ambre_trame::logic::tests::group7::csv_export_writes_the_formatted_value` | none | verified |
| `TRAME-109` | `OBS` | A cell carrying a custom pattern MUST show that pattern applied. | this spec section 5 | `ambre_trame::logic::tests::group1::custom_pattern_is_applied`; `ambre_trame::logic::number_format::tests::custom_numeric_patterns`; `::custom_literals_and_interleaved_digits`; `::custom_sections_choose_by_sign`; `::custom_date_and_time_tokens` | none | verified |
| `TRAME-110` | `OBS` | A currency and a date MUST follow the document's locale, not the machine's. | this spec section 5 | not built | unbuilt -- lands with the `One document locale` Story (`ambre-roadmap.md` E8) | unimplemented |

### 1.2 Change the decimal count

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TRAME-105` | `POST` | Increasing the decimals MUST add one place to the count the cell already carries. | this spec section 3 | `ambre_trame::logic::tests::group1::decimal_step_adds_one_place_to_the_count` | none | verified |
| `TRAME-158` | `FRAME` | Changing the decimal count MUST NOT change a computed value. | this spec section 3 | `ambre_trame::logic::tests::group13::a_decimal_step_keeps_the_computed_value` -- the engine result and the cells map compared across the step | none | verified |

* **Budget.** none of its own. A format is a pure function of a value and a pattern, applied per
  displayed cell inside the frame phases of `ambre-spec-budgets.md section 2`.
* **Boundary.** none of its own: a format is a document write, and `ZK-001` seals every payload
  before it leaves the client.
* **Verification.** 11 `verified`, 1 `unimplemented`. What holds is the pair at the
  root of the area. A type-level argument gives what a cell holds (`TRAME-102`), and one test that
  packs a formatted cell gives what it shows (`TRAME-157`). The decimal step, the calendar, the
  custom pattern and the reach past the grid each have their test. What is loose is the surface: no
  control lets a user type a pattern, so `TRAME-109` holds for patterns only a document carries.

---

## 2. MODEL

**Containers.** None of its own. The family and the decimal count are two fields of `CellStyle`, in
`cellstyle_{sheet}` (`features/ambre-spec-trame-cell-styles.md section 3`).

**Shapes** (`ambre-bus`):

```rust
pub enum CellFormat {
    General,                 // no reformatting: the value is shown as-is
    Number, Currency, Percent, Scientific, Fraction,
    Date, Time,              // the value is a day or second serial
    Text,                    // no numeric coercion
    Custom(String),          // the pattern the user typed
}
```

**The families, and what each renders.** `decimals` is the cell's count when it carries one.

| Family | A numeric value renders as | Default decimals |
|---|---|---|
| `General` | the computed text, verbatim | -- |
| `Number` | a fixed-point decimal | 2 |
| `Currency` | a fixed-point decimal, then a space and the euro sign | 2 |
| `Percent` | the value times a hundred, then `%` | 0 |
| `Scientific` | exponent notation | 2 |
| `Fraction` | a whole part and sixteenths, both signed together (`TRAME-106`) | -- |
| `Date`, `Time` | the calendar date `2026-09-29`, the time of day `14:05:09` (`TRAME-107`) | -- |
| `Text` | the text, verbatim | -- |
| `Custom` | the value through the pattern (`TRAME-109`), in the language below | -- |

**Where it happens**, per cell, once:

1. The engine recomputes every dirty cell in the requested window.
2. The packing loop reads the cell's computed text and its resolved `CellStyle`.
3. `format_display` parses the text as an `f64`. A parse that fails returns the text (`TRAME-103`),
   which lets `#REF!` and a column header survive a format applied to the whole column.
4. `format_number` renders the value for the family and the decimal count.

Find, replace and CSV export read the same rendered text, never the computed one (`TRAME-108`).

**The pattern language** of `Custom`, the common subset of the spreadsheet formats:

| Piece | Renders |
|---|---|
| `;` | up to three sections: positive, negative, zero. A sign-chosen section prints the magnitude |
| `0` / `#` | a digit; `0` prints a zero where the number has none, `#` prints nothing |
| `.` / `,` / `%` | the decimal point, the thousands grouping, and the value times a hundred |
| `"text"` / `\c` | a literal |
| `y m d h s`, `AM/PM` | the date and time parts; `m` after `h` or before `s` is the minutes |
| anything else | itself: no pattern is refused |

**Code.** `logic/styles.rs` -- `format_display`, `displayed_text`; `logic/packing.rs` --
`format_number`; `logic/number_format.rs` -- the calendar and the pattern language;
`logic/commands.rs` -- the viewport packing loop that calls it.

---

## 3. SURFACE

| Control | Effect |
|---|---|
| the format button | a popover listing the families, a check on the active one |
| decrease decimals | removes one place from each selected cell's count, down to zero (`TRAME-105`) |
| increase decimals | adds one place to each selected cell's count, from its family default (`TRAME-105`) |
| both decimal buttons | disabled unless the active cell's family renders decimals (`TRAME-104`) |

The controls read the active cell, like the rest of the toolbar, and write the whole selection. A
format edit is a restyle -- one undo item, one delta, whole-cell last-writer-wins -- and answers with
`DataReset`, so the viewport re-packs. Nothing recomputes: the engine's values did not change.

---

## 4. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| The format is applied at pack time | storing the formatted text beside the raw text | the document must hold one value for two readers on different locales; a formatted string in the CRDT would be a second source of truth, and would break every formula reading the cell. |
| A non-numeric value ignores its format | rendering a type mismatch as an error value | a column formatted as Currency also holds its header and, after a broken reference, `#REF!`. Blanking those, or erroring on them, loses more than it explains. |
| The family and the decimal count are two fields | folding the count into each numeric variant | the count is meaningful for four families and meaningless for six, and folding it in loses the precision a user chose the moment they change family. |
| `General` is the default and does nothing | defaulting to a fixed decimal count | a spreadsheet must show what was typed until asked otherwise; a default that rounds would make an untouched document lie about its own contents. |
| A country-specific family is expressed as a pattern | a `Special` family, as Excel carries | a zip code, a phone number and a national identifier are one country's shapes, and one family per country is a list with no end. `Custom` carries an arbitrary pattern, so the shape is expressible without a family the rest of the world never selects. |

---

## 5. NOT BUILT, AND NOT YET RULED

* **The currency is the euro, in the code** (`TRAME-110`). There is no currency choice and no
  document locale. The decimal separator, the thousands separator and the date order are whatever
  the format function was written to emit.
* **Nothing keeps a decimal count off a family that ignores it** (`TRAME-104`). The predicate is
  exhaustive, and its one use is greying the two buttons. The command merges whatever `decimals` it
  is given into whatever family the cell carries. A document written by anything but the toolbar --
  an import, a later API -- can hold a count on a `Text` cell. The row above is a rule about the
  control, not about the document.
* **Few tests pack a formatted cell** (`TRAME-157`). `format_display` is reached through the pack on
  Percent, and on a word under five families (`TRAME-103`). The three pass-through families and the
  fraction path through the pack loop are unread. The fraction test calls the renderer directly,
  so nothing proves the loop hands it the value the cell holds.
* **A number too wide for its column is not abbreviated.** A spreadsheet shows `#####` rather than a
  truncated number, because a truncated number is a wrong number. Trame spills or clips it like any
  other text (`features/ambre-spec-trame-cell-rendering.md section 3`), so a reader can misread a
  clipped figure.
