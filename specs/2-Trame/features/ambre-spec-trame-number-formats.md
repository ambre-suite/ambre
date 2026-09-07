# AMBRE -- FEATURE: NUMBER FORMATS

**Type:** Vertical (feature)
**Implementation:** partial -- nine families exist; two of them render their raw serial.

> A user picks Percent. The cell shows `314.2%` and still holds `3.14159`, so a formula reading it
> reads the number, not the text a reader sees.

A format is a field of the cell style, applied once per viewport pack. It owns no container and no
command: what it owns is the mapping from a value to the text a reader sees, and the rule that the
two never swap places. The register, the toolbar and the convergence of a format change are
`features/ambre-spec-trame-cell-styles.md`.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|---|
| `TRAME-102` | `INV` | A format MUST NOT change what a cell holds. | this spec section 2 | type-level: `format_display` takes `&self` and returns a `String`; its one call site is the viewport packing loop, so no formatted text can reach `cells_{sheet}` | verified |
| `TRAME-157` | `OBS` | A formatted cell MUST show the text its family renders for the value it holds. | this spec section 2 | `ambre_trame::logic::tests::group1::set_cell_style_round_trips_and_formats_numbers`, which reads `314.2%` off the packed viewport for a cell holding `3.14159` under Percent with one decimal. One family of the nine, and the only test that reaches `format_display` at all | verified |
| `TRAME-103` | `OBS` | A value that is not a number MUST be shown unchanged, whatever format the cell carries. | this spec section 2 | none. `format_display` falls back to the text when the parse fails, and no test formats a word | unverified |
| `TRAME-104` | `OBS` | A decimal count MUST be offered only for a family that renders decimal places. | this spec section 3 | `ambre_bus::has_decimals_is_true_only_for_numeric_families`, exhaustive over the ten families; the predicate's one use is the `disabled` attribute of the two buttons. It guards the control, never the write -- section 5 | verified |
| `TRAME-106` | `OBS` | A fraction MUST keep the sign of a value below one, and MUST NOT print a whole as sixteenths. | this spec section 2 | `ambre_trame::logic::packing::tests::fraction_format_keeps_the_sign_and_carries`, which calls the renderer directly, not through a packed cell | verified |
| `TRAME-107` | `OBS` | A date-formatted cell MUST show a calendar date, never the day serial behind it. | this spec section 5 | none, and the code does the **opposite**: `Date` and `Time` print the serial as a number | unverified |
| `TRAME-108` | `OBS` | A formatted value MUST read the same everywhere it is shown as text. | this spec section 5 | none, and the code does the **opposite**: find, replace and CSV export read the unformatted value, so a cell shown as `50.0%` is found by searching `0.5` | unverified |
| `TRAME-109` | `OBS` | A cell carrying a custom pattern MUST show that pattern applied. | this spec section 5 | none, and the code does the **opposite**: `Custom` returns the text untouched, so the pattern is stored and silently ignored | unverified |
| `TRAME-110` | `OBS` | A currency and a date MUST follow the document's locale, not the machine's. | this spec section 5 | not built -- lands with the "One document locale" Story (`ambre-roadmap.md` E8) | unimplemented |

### 1.2 Change the decimal count

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|---|
| `TRAME-105` | `POST` | Increasing the decimals MUST add one place to the count the cell already carries. | this spec section 3 | none, and the code does the **opposite**: the two buttons write the constants `0` and `2`, so the control is a two-value switch, not a step | unverified |
| `TRAME-158` | `FRAME` | Changing the decimal count MUST NOT change a computed value. | this spec section 3 | none. The restyle writes the style containers alone and drives no engine pass; the viewport re-packs from the values it already held, and no test asserts that it re-packs the same ones | unverified |

* **Budget.** none of its own: a format is a pure function of a value and a pattern, applied per
  displayed cell inside the frame phases of `ambre-spec-budgets.md section 2`.
* **Boundary.** none of its own: a format is a document write, and `ZK-001` seals every payload
  before it leaves the client.
* **Verification.** 4 `verified`, 6 `unverified`, 1 `unimplemented`. `TRAME-102` stated two rules
  in one row and is now two: what a cell holds is a type-level argument, and what a cell shows is
  `TRAME-157`, held by the one test that packs a formatted cell. Four of the six gaps are rows where
  the code is known to do something else -- the decimal control, the date family, the custom
  pattern, and the reach of a format past the grid. `TRAME-109` no longer offers a refusal as an
  alternative: nothing lets a user type a pattern, so a rule about refusing one claims a surface
  that does not exist.

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
| `Date`, `Time` | the serial, unconverted (`TRAME-107`) | -- |
| `Text` | the text, verbatim | -- |
| `Custom` | the text, verbatim (`TRAME-109`) | -- |

**Where it happens**, per cell, once:

1. The engine recomputes every dirty cell in the requested window.
2. The packing loop reads the cell's computed text and its resolved `CellStyle`.
3. `format_display` parses the text as an `f64`; a parse that fails returns the text (`TRAME-103`),
   which is what lets `#REF!` and a column header survive a format applied to the whole column.
4. `format_number` renders the value for the family and the decimal count.

**Code.** `logic/styles.rs` -- `format_display`; `logic/packing.rs` -- `format_number`;
`logic/commands.rs` -- the viewport packing loop that calls it.

---

## 3. SURFACE

| Control | Effect |
|---|---|
| the format button | a popover listing the families, a check on the active one |
| decrease decimals | writes a decimal count of zero (`TRAME-105`) |
| increase decimals | writes a decimal count of two (`TRAME-105`) |
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

* **Date and time render their serial** (`TRAME-107`). Both families fall through to a plain number,
  so a cell holding `=TODAY()` under a Date format shows a five-digit integer. The conversion it
  needs already exists one module away: `ymd_from_serial`, in the function library, is what `YEAR`,
  `MONTH` and `DAY` are built on.
* **A format stops at the edge of the grid** (`TRAME-108`). `format_display` has one call site, the
  viewport pack. Find, replace and CSV export each read the computed value instead, so the same cell
  has two texts depending on who asks. `TRAME-UI-078` -- a plain cell is matched on its displayed
  value -- is `verified` by a test whose cells carry no format, and is false as soon as one does.
* **A custom pattern is accepted and ignored** (`TRAME-109`). `CellFormat::Custom` carries the
  string, the toolbar has no field to type one into, and the renderer returns the text unchanged. No
  rule says which pattern language it would be, and none says a bad one is refused: there is no
  surface that would do the refusing.
* **The currency is the euro, in the code** (`TRAME-110`). There is no currency choice and no
  document locale, so a decimal separator, a thousands separator and a date order are all whatever
  the format function was written to emit.
* **Nothing keeps a decimal count off a family that ignores it** (`TRAME-104`). The predicate is
  exhaustive and its one use is greying the two buttons; the command merges whatever `decimals` it
  is given into whatever family the cell carries. A document written by anything but the toolbar --
  an import, a later API -- can hold a count on a `Text` cell, and the row above is a rule about the
  control, not about the document.
* **One test packs a formatted cell** (`TRAME-157`). `format_display` is reached by a single test,
  on Percent. The parse failure, the three pass-through families and the fraction path through the
  pack loop are all unread: the fraction test calls the renderer directly, so nothing proves the
  loop hands it the value the cell holds.
* **A number too wide for its column is not abbreviated.** A spreadsheet shows `#####` rather than a
  truncated number, because a truncated number is a wrong number. Trame spills or clips it like any
  other text (`features/ambre-spec-trame-cell-rendering.md section 3`), so a reader can misread a
  clipped figure.
