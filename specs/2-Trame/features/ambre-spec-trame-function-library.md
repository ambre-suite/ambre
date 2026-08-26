# AMBRE -- FEATURE: FUNCTION LIBRARY

**Type:** Vertical (feature)
**Implementation:** partial -- 112 functions ship; the reference-preserving and advanced sets do not.

> The named calls a user may write inside a formula: `SUM`, `VLOOKUP`, `IF`, `TEXT`, `PMT`. The engine
> that evaluates them is a feature of its own; this one is the catalogue and the rules it obeys.

A function is a name, a number of already-evaluated arguments, and one value back. That contract is
the whole seam between this document and `features/ambre-spec-trame-formula-engine.md`: the engine
never looks inside a function, and a function never looks at the document.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`.

| ID | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|
| `TRAME-014` | A total over a range MUST be exact, and MUST NOT depend on the order of the cells. | this spec section 2 | `ambre_trame::formula::prop_tests::sum_is_exact_and_order_independent`, `::engine_sum_matches_naive_sum`, `::product_is_order_independent`, `::average_times_count_equals_sum`, `::min_max_bound_every_element` (proptest) | verified |
| `TRAME-049` | A name the library does not know MUST read as an error, never as a blank. | this spec section 3 | none. The dispatch falls through to `#N/A`, and no test calls a name outside the catalogue -- so the arm that answers a mistyped `=SUMM(A1)` is the one arm nothing drives | unverified |
| `TRAME-050` | A function MUST receive values, never references. | this spec section 2 | type-level: `call_builtin(name, args: &[CellValue], ctx)` takes evaluated values, and the machine pops every argument off the stack before the call | verified |
| `TRAME-051` | A range given where a value is expected MUST be read cell by cell. | this spec section 2 | `ambre_trame::formula::prop_tests::sum_flattens_range_like_scalars`, `::engine_sum_matches_naive_sum`, `ambre_trame::formula::functions::tests::sum_basic` | verified |
| `TRAME-052` | A call MUST NOT lose an argument, whatever their number. | this spec section 4 | `ambre_trame::formula::vm::tests::call_with_more_than_255_arguments_keeps_them_all` | verified |
| `TRAME-053` | Every clock reading inside one recomputation MUST be the same instant. | this spec section 2 | type-level: the clock is sampled once per pass in `formula/engine.rs` and copied into every call context; no function reads the system clock itself | verified |
| `TRAME-054` | A function that asks where it sits MUST answer the cell being computed. | this spec section 3 | `ambre_trame::formula::functions::tests::lookup_reference_functions` | verified |
| `TRAME-055` | A lookup that finds no key MUST say so, and MUST NOT answer a neighbour. | this spec section 3 | `ambre_trame::formula::functions::tests::vlookup_exact_and_approx`, `::index_and_match` | verified |
| `TRAME-056` | A total MUST NOT silently drop a cell it cannot read. | this spec section 5 | none, and the code does the **opposite**: the numeric flattening skips an error value exactly as it skips an empty one, so `=SUM(A1:A3)` answers the sum of the readable cells when one of them holds `#REF!`. Neither behaviour is asserted anywhere | unverified |
| `TRAME-057` | A function given a range MUST be able to answer a reference into it, not only a value. | this spec section 5 | not built -- lands with the "Engine completion" Story (`ambre-roadmap.md` E6) | unimplemented |
| `TRAME-058` | A lookup MUST be expressible without counting columns from the key. | this spec section 5 | not built -- lands with the "Engine completion" Story (`ambre-roadmap.md` E6) | unimplemented |
| `TRAME-059` | A user MUST be able to name a formula of their own and call it. | this spec section 5 | not built -- lands with the "Engine completion" Story (`ambre-roadmap.md` E6) | unimplemented |
| `TRAME-060` | A formula answering many values MUST fill the cells beside it, and MUST refuse to overwrite. | this spec section 5 | not built -- lands with the "Engine completion" Story (`ambre-roadmap.md` E6) | unimplemented |
| `TRAME-061` | A total MUST be able to skip the rows a filter hides. | this spec section 5 | not built -- lands with the "Data tools" Story (`ambre-roadmap.md` E6) | unimplemented |

* **Budget.** none of its own: a call is one node of the graph the engine recomputes, so it lands
  under the 10k-cell DAG recompute of `ambre-spec-budgets.md section 1`, which
  `features/ambre-spec-trame-formula-engine.md` owns.
* **Boundary.** none: a function reads cells and returns a value in the Logic thread. It opens no
  path, and `ZK-001` seals what the document sends.
* **Verification.** 7 `verified`, 2 `unverified`, 5 `unimplemented`. `TRAME-056` is the one the
  code contradicts, and it is the expensive kind: a total that is quietly wrong reads like a total
  that is right. `TRAME-049` is the same shape one step milder -- a typed function name that no
  test exercises.

---

## 2. MODEL

**Containers.** None. A function reads its arguments and nothing else.

**Commands and events.** None. A function is reached only through a formula.

**The call, in steps:**

1. The machine has already evaluated every argument, so the stack holds values (`TRAME-050`).
2. It pops the argument count the call recorded, in order, and builds the slice.
3. It adds the ambient context: the recomputation clock, and the 1-based position of the cell being
   computed (`TRAME-053`, `TRAME-054`).
4. `call_builtin` matches the upper-cased name and returns one value, which the machine pushes back.

```rust
pub struct FnCtx {
    pub now_serial: f64,   // the recomputation clock, one instant per pass
    pub cell_row: u32,     // 1-based position of the cell being computed; 0 means unknown
    pub cell_col: u32,
}

pub fn call_builtin(name: &str, args: &[CellValue], ctx: FnCtx) -> CellValue;
```

**Reading arguments.** Two flattenings, and the choice between them is the semantics:

| Helper | Keeps | Used by |
|---|---|---|
| numeric | numbers, booleans as 1 and 0, and text that parses as a number | every arithmetic and statistical function |
| whole | every value, errors and blanks included | the counting, information and text functions |

**Code.** `formula/functions/mod.rs` holds the dispatch, the context and the criterion parser for the
conditional aggregates; `builtins_a.rs`, `builtins_b.rs` and `builtins_c.rs` hold the bodies.

---

## 3. SURFACE

**The catalogue**, 112 names in seven families:

| Family | Count | Names |
|---|---|---|
| Math | 35 | `SUM` `AVERAGE` `MIN` `MAX` `COUNT` `COUNTA` `ABS` `ROUND` `SQRT` `POWER` `SUMIF` `COUNTIF` `INT` `MOD` `ROUNDUP` `ROUNDDOWN` `CEILING` `FLOOR` `PRODUCT` `SUMPRODUCT` `MEDIAN` `COUNTBLANK` `TRUNC` `SIGN` `SUMSQ` `EXP` `LN` `LOG10` `LOG` `PI` `SIN` `COS` `TAN` `RADIANS` `DEGREES` |
| Statistics | 10 | `AVERAGEIF` `AVERAGEIFS` `SUMIFS` `COUNTIFS` `STDEV` `VAR` `LARGE` `SMALL` `RANK` `MODE` |
| Financial | 7 | `PMT` `FV` `PV` `RATE` `NPER` `NPV` `IRR` |
| Logic | 18 | `IF` `IFERROR` `AND` `OR` `NOT` `XOR` `ISBLANK` `ISNUMBER` `ISTEXT` `ISERROR` `ISNA` `ISEVEN` `ISODD` `IFNA` `IFS` `SWITCH` `TRUE` `FALSE` |
| Lookup | 9 | `VLOOKUP` `HLOOKUP` `INDEX` `MATCH` `CHOOSE` `ROWS` `COLUMNS` `ROW` `COLUMN` |
| Text | 20 | `CONCATENATE` `TEXT` `LEFT` `RIGHT` `MID` `UPPER` `LOWER` `TRIM` `LEN` `PROPER` `SUBSTITUTE` `REPT` `FIND` `SEARCH` `REPLACE` `EXACT` `TEXTJOIN` `CHAR` `CODE` `VALUE` |
| Date and time | 13 | `TODAY` `NOW` `DATE` `YEAR` `MONTH` `DAY` `TIME` `HOUR` `MINUTE` `SECOND` `WEEKDAY` `EDATE` `EOMONTH` |

**How a call behaves:**

| Question | Answer |
|---|---|
| Is the name case-sensitive | no. It is matched upper-cased, so `sum(a1:a3)` and `SUM(A1:A3)` are one call |
| An unknown name | reads `#N/A` (`TRAME-049`) |
| How arguments are separated | by commas. A range is one argument holding many values |
| A criterion, for `SUMIF` and its kin | text carrying an operator and an operand: `">5"`, `"<>done"`, or a bare value meaning equality |
| A criterion against a non-number | `>` and `>=` refuse it; `<>` is the one operator it satisfies |
| How a date is held | a serial number on the 1899-12-30 epoch: integer part the day, fraction the time of day |
| Where `TODAY` and `NOW` read | the recomputation clock, never the system clock (`TRAME-053`) |

---

## 4. DECISIONS

| Decision | Rationale | Rejected alternative |
|---|---|---|
| A function receives values, never references | the machine already evaluates each argument for cycle detection to work through ranges. Handing a function a reference would mean a second evaluation path, with a second set of cycle rules | passing references and letting each function resolve them |
| An empty total is zero, not an error | `=SUM(A1:A10)` over a range a user has not filled yet is the normal state of a sheet under construction, not a mistake | an error on an empty range |
| Text that reads as a number counts in a total | a value pasted from the web or imported from a CSV arrives as text, and a total that ignores it is wrong in a way the user cannot see | counting only values typed as numbers |
| The argument count is not a byte | a `u8` count wrapped at 256, so the 256th argument turned `SUM` into a call with no arguments, which answered the last operand left on the stack instead of the total (`TRAME-052`) | keeping the compact encoding and capping the call |
| One clock per recomputation | two cells reading `NOW()` in the same pass must agree, or a sheet cannot subtract one from the other | reading the system clock in each call |
| The bodies live in three modules, the dispatch in one | 112 arms in one file is a merge conflict per function added; the dispatch is the part that must stay readable as a list | one module per family, which multiplies the imports for no gain |

---

## 5. NOT BUILT, AND NOT YET RULED

* **A total drops what it cannot read** (`TRAME-056`). The numeric flattening treats an error like a
  blank, so a `#REF!` inside a summed range subtracts itself from the answer instead of poisoning it.
  Excel and Sheets return the error. This is the only rule in this document the code contradicts, and
  the failure is silent by construction.
* **Reference-preserving functions** (`TRAME-057`). `OFFSET` and `INDIRECT` are absent, and `ROW`
  with an argument answers `#N/A` on purpose: a function that returns a reference needs the machine to
  accept one, which `TRAME-050` says it does not. The rule and the seam move together.
* **The advanced set** (`TRAME-058`, `TRAME-059`, `TRAME-060`). No `XLOOKUP`, no `LAMBDA`, no spilling
  formula. A dynamic array needs a rule for the cells it fills and for the collision when they are not
  empty, which no document has written.
* **A total that skips filtered rows** (`TRAME-061`). `SUBTOTAL` is absent, and it is the deliberate
  exception to `TRAME-021`: every other result is independent of view state, and this one is not. What
  a peer with a different personal filter reads is the open question.
* **The catalogue has no accuracy oracle.** The property tests cover the aggregates against a naive
  sum; the other hundred functions are covered by example, one assertion per case. Nothing compares
  `PMT` or `WEEKDAY` against a reference implementation, so a wrong formula stays wrong and green.
