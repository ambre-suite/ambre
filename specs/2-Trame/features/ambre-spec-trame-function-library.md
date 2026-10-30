# AMBRE -- FEATURE: FUNCTION LIBRARY

**Type:** Vertical (feature)
**Implementation:** partial -- 113 functions ship; the reference-preserving and advanced sets do not.

> The named calls a user may write inside a formula: `SUM`, `VLOOKUP`, `IF`, `TEXT`, `PMT`. The engine
> that evaluates them is a feature of its own; this one is the catalogue and the rules it obeys.

A function is a name, a number of already-evaluated arguments, and one value back. That contract is
the whole seam between this document and `features/ambre-spec-trame-formula-engine.md`. The engine
never looks inside a function, and a function never looks at the document.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`.

### 1.1 The catalogue

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TRAME-050` | `INV` | A function MUST receive values, never references. | this spec section 2 | type-level -- `call_builtin(name, args: &[CellValue], ctx)` takes evaluated values, and the machine pops every argument off the stack before the call | none | verified |
| `TRAME-053` | `INV` | Every clock reading inside one recomputation MUST be the same instant. | this spec section 2 | type-level -- `EvalContext` holds one `now_serial` for the whole pass, and every `FnCtx` the machine builds copies that field | none | verified |
| `TRAME-194` | `INV` | A function MUST NOT read the clock for itself. | this spec section 2 | type-level -- the clock reader `now_serial` is private to the engine module, so a function reads the clock of its context alone | none | verified |
| `TRAME-195` | `INV` | Every name a formula can call MUST appear in the catalogue. | this spec section 3 | `ambre_trame::formula::functions::tests::catalogue_lists_every_dispatched_name` -- reads the `match` arms of `call_builtin` and the table of section 3 | none | verified |
| `TRAME-196` | `INV` | A name MUST be recognised whatever its case. | this spec section 3 | `ambre_trame::logic::tests::group12::a_function_name_is_read_whatever_its_case` | none | verified |

### 1.2 Call a function in a formula

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TRAME-197` | `PRE` | The key column of an approximate lookup MUST already be ascending. | this spec section 3 | `ambre_trame::formula::functions::tests::an_approximate_lookup_over_falling_keys_answers_na` -- `VLOOKUP`, `HLOOKUP` and `MATCH` | none | verified |
| `TRAME-198` | `PRE` | A recomputation MUST be given its clock before it runs. | this spec section 2 | type-level -- `EvalContext::new` takes the clock, so no pass runs without one; `ambre_trame::logic::tests::group12::today_reads_the_clock_of_the_recomputation` | none | verified |
| `TRAME-014` | `POST` | A total over whole numbers MUST equal the naive total. | this spec section 2 | `ambre_trame::formula::prop_tests::sum_is_exact_and_order_independent`; `::engine_sum_matches_naive_sum`; `::average_times_count_equals_sum`; `::min_max_bound_every_element` -- four proptests over integer inputs | none | verified |
| `TRAME-199` | `POST` | A total MUST NOT depend on the order of the cells. | this spec section 2 | `ambre_trame::formula::prop_tests::a_total_of_decimals_is_the_same_in_any_order` -- decimals of mixed magnitudes, three orders compared bit for bit; `ambre_trame::formula::functions::tests::two_orders_of_one_decimal_total_agree`; `::a_total_of_the_largest_numbers_is_exact_in_any_order` -- terms near the largest number that cancel, in two orders; `::a_tiny_term_beside_the_largest_numbers_sums_the_same_in_any_order` -- a tiny term beside them, in three orders compared bit for bit | none | verified |
| `TRAME-051` | `POST` | A range given where a value is expected MUST be read cell by cell. | this spec section 2 | `ambre_trame::formula::prop_tests::sum_flattens_range_like_scalars`; `::engine_sum_matches_naive_sum`; `ambre_trame::formula::functions::tests::sum_basic` | none | verified |
| `TRAME-200` | `POST` | A value stored as text MUST count in a total when it reads as a finite number. | this spec section 4 | `ambre_trame::formula::functions::tests::text_that_reads_as_a_number_counts_in_a_total`; `::text_that_spells_no_finite_number_counts_in_no_total` | none | verified |
| `TRAME-056` | `POST` | A total MUST NOT silently drop a cell it cannot read. | this spec section 2 | `ambre_trame::formula::functions::tests::total_propagates_an_error_it_cannot_read` | none | verified |
| `TRAME-052` | `POST` | A call MUST NOT lose an argument, whatever their number. | this spec section 4 | `ambre_trame::formula::vm::tests::call_with_more_than_255_arguments_keeps_them_all` | none | verified |
| `TRAME-054` | `POST` | A function that asks where it sits MUST answer the cell being computed. | this spec section 3 | `ambre_trame::formula::functions::tests::lookup_reference_functions` | none | verified |
| `TRAME-055` | `POST` | An approximate lookup MUST answer the largest key at or below the target. | this spec section 3 | `ambre_trame::formula::functions::tests::vlookup_exact_and_approx`; `::index_and_match` | none | verified |
| `TRAME-201` | `POST` | A lookup that omits its match flag MUST be read as approximate. | this spec section 3 | `ambre_trame::formula::functions::tests::a_lookup_with_no_flag_is_approximate` | none | verified |
| `TRAME-202` | `POST` | An exact lookup MUST match a text key whatever its case. | this spec section 3 | `ambre_trame::formula::functions::tests::an_exact_lookup_ignores_case`; `::an_exact_lookup_folds_a_final_sigma_as_the_find_does` -- the fold the find and `SEARCH` read, a criterion too | none | verified |
| `TRAME-246` | `POST` | A search that ignores case MUST answer a position in the characters of the text it was given. | this spec section 3 | `ambre_trame::formula::functions::tests::search_counts_the_chars_of_the_text_it_was_given` -- a capital I with a dot, which lowercases to two chars, and a final sigma | none | verified |
| `TRAME-247` | `POST` | An annuity function MUST read a type argument other than zero as a payment at the start of each period. | this spec section 3 | `ambre_trame::formula::functions::tests::an_annuity_type_other_than_zero_reads_as_the_start` -- `PMT`, `FV`, `PV`, `NPER` and `RATE` given a type of 2 and of -1 | none | verified |
| `TRAME-250` | `POST` | `MID` MUST read a blank count as 0, a numeric text as its number. | this spec section 3 | `ambre_trame::formula::functions::tests::mid_reads_a_blank_count_as_zero` -- a blank length and start, a numeric and a non-numeric text start | none | verified |
| `TRAME-057` | `POST` | A function given a range MUST be able to answer a reference into it, not only a value. | this spec section 5 | not built | unbuilt -- lands with the `Functions that answer a reference` Story (`ambre-roadmap.md` E6) | unimplemented |
| `TRAME-058` | `POST` | A lookup MUST be expressible without counting columns from the key. | this spec section 5 | not built | unbuilt -- lands with the `Functions that answer a reference` Story (`ambre-roadmap.md` E6) | unimplemented |
| `TRAME-059` | `POST` | A user MUST be able to name a formula of their own and call it. | this spec section 5 | not built | unbuilt -- lands with the `User-defined functions` Story (`ambre-roadmap.md` E6) | unimplemented |
| `TRAME-061` | `POST` | A total MUST be able to skip the rows a filter hides. | this spec section 5 | not built | unbuilt -- lands with the `Data tools` Story (`ambre-roadmap.md` E6) | unimplemented |
| `TRAME-203` | `FRAME` | A function MUST NOT read or write the document. | this spec section 2 | type-level -- `call_builtin(name, &[CellValue], FnCtx)` receives no document, no cell cache and no index, so a body has nothing to reach for | none | verified |
| `TRAME-204` | `FRAME` | A call MUST NOT consume a value that is not one of its arguments. | this spec section 2 | `ambre_trame::formula::prop_tests::compile_emits_balanced_bytecode` -- its generator emits `FnCall` nodes with 0 to 3 arguments and its stack simulation rejects any `CallFn` reading below its own operands | none | verified |
| `TRAME-049` | `FAIL` | A name the library does not know MUST read as an error, never as a blank. | this spec section 3 | `ambre_trame::logic::tests::group12::an_unknown_function_reads_as_an_error` | none | verified |
| `TRAME-205` | `FAIL` | An exact lookup that finds no key MUST answer an error, and MUST NOT answer a neighbour. | this spec section 3 | `ambre_trame::formula::functions::tests::vlookup_exact_and_approx` -- its exact miss asserts `#N/A` | none | verified |
| `TRAME-244` | `FAIL` | A call or an operator whose result is not a finite number MUST answer `#NUM!`, and so MUST an average whose total is past the largest number. | this spec section 2; section 3 | `ambre_trame::formula::functions::tests::a_result_that_is_not_finite_answers_num` -- a total past the largest number either way, an average of such a total, a sum of squares, a product, a deviation, an exponential, and a maximum given an infinity; `ambre_trame::formula::vm::tests::an_operator_whose_result_is_not_finite_answers_num` -- a quotient and a product past the largest number, an infinity negated or divided, and the `IFERROR` and `ISERROR` that see the error | none | verified |
| `TRAME-249` | `FAIL` | `MID` given a start below 1, a negative length or a NaN MUST answer `#NUM!`, as `LEFT`, `RIGHT` and `REPT` answer a count out of their domain. | this spec section 3; `ambre-spec-trame-formula-engine.md section 4` | `ambre_trame::formula::functions::tests::mid_refuses_a_start_below_one_and_a_negative_length` -- a start of 0 and of -1, a length of -1, and a NaN in either | none | verified |
| `TRAME-251` | `FAIL` | `MID` given an error as a count MUST answer that error unchanged. | this spec section 3; `ambre-spec-trame-formula-engine.md section 4` | `ambre_trame::formula::functions::tests::mid_passes_an_error_count_through` -- `#DIV/0!` as the start and as the length | none | verified |

### 1.3 Spill a result across the cells beside it

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TRAME-060` | `POST` | A formula answering many values MUST fill the cells beside it. | this spec section 5 | not built | unbuilt -- lands with the `Dynamic arrays and spill` Story (`ambre-roadmap.md` E6) | unimplemented |
| `TRAME-206` | `FAIL` | A spill that meets a cell it cannot have MUST refuse, and MUST say which cell. | this spec section 5 | not built | unbuilt -- lands with the `Dynamic arrays and spill` Story (`ambre-roadmap.md` E6) | unimplemented |
| `TRAME-207` | `FRAME` | A spill that shrinks MUST NOT leave a value in a cell it no longer covers. | this spec section 5 | not built | unbuilt -- lands with the `Dynamic arrays and spill` Story (`ambre-roadmap.md` E6) | unimplemented |

* **Budget.** none of its own: a call is one node of the graph the engine recomputes. It lands
  under the 10k-cell DAG recompute of `ambre-spec-budgets.md section 1`, which
  `features/ambre-spec-trame-formula-engine.md` owns.
* **Boundary.** none: a function reads cells and returns a value in the Logic thread. It opens no
  path, and `ZK-001` seals what the document sends.
* **Verification.** 27 `verified`, 7 `unimplemented`. What holds is the arithmetic core and the
  edges of the library. A total is rounded once from an exact sum, so its order cannot change it
  (`TRAME-199`), and one past the largest number reads `#NUM!` (`TRAME-244`). A lookup refuses keys
  out of order, and no function can read the clock for itself. The seven unimplemented rows are
  functions not yet in the catalogue.

---

## 2. MODEL

**Containers.** None. A function reads its arguments and nothing else.

**Commands and events.** None. A function is reached only through a formula.

**The call, in steps:**

1. The machine has already evaluated every argument, so the stack holds values (`TRAME-050`).
2. It pops the argument count the call recorded, in order, and builds the slice.
3. It adds the ambient context: the recomputation clock, and the position of the cell being
   computed, which `ROW()` and `COLUMN()` answer 1-based (`TRAME-053`, `TRAME-054`).
4. `call_builtin` matches the upper-cased name and returns one value, which the machine pushes back.
   A number that is not finite becomes `#NUM!` there, for every function, as it does for every
   operator (`TRAME-244`). An average divides its total, so a total past the largest number
   leaves it `#NUM!`, as in Excel. The true average may still be finite.

```rust
#[derive(Clone, Copy)]
pub struct FnCtx {
    pub now_serial: DateSerial,  // the recomputation clock, one instant per pass
    pub cell_row: Option<Row>,   // the position of the cell being computed; None when unknown
    pub cell_col: Option<Col>,
}

pub fn call_builtin(name: &FunctionName, args: &[CellValue], ctx: FnCtx) -> CellValue;
```

**Reading arguments.** Three flattenings, and the choice between them is the semantics:

| Helper | Keeps | Used by |
|---|---|---|
| numeric | numbers, booleans as 1 and 0, and text that parses as a number | `COUNT`, and every function reading one number from an argument |
| total | the numeric flattening, except that the first error met is the answer (`TRAME-056`) | every function that folds a list: `SUM`, `AVERAGE`, `MIN`, `MAX`, `PRODUCT`, `SUMSQ`, `MEDIAN`, `STDEV`, `VAR`, `MODE`, `LARGE`, `SMALL`, `NPV`, `IRR` |
| whole | every value, errors and blanks included | the counting, information and text functions |

**Code.** `formula/functions/mod.rs` holds the dispatch, the context and the criterion parser for the
conditional aggregates; `builtins_a.rs`, `builtins_b.rs` and `builtins_c.rs` hold the bodies.

---

## 3. SURFACE

**The catalogue**, 113 names in seven families:

| Family | Count | Names |
|---|---|---|
| Math | 35 | `SUM` `AVERAGE` `MIN` `MAX` `COUNT` `COUNTA` `ABS` `ROUND` `SQRT` `POWER` `SUMIF` `COUNTIF` `INT` `MOD` `ROUNDUP` `ROUNDDOWN` `CEILING` `FLOOR` `PRODUCT` `SUMPRODUCT` `MEDIAN` `COUNTBLANK` `TRUNC` `SIGN` `SUMSQ` `EXP` `LN` `LOG10` `LOG` `PI` `SIN` `COS` `TAN` `RADIANS` `DEGREES` |
| Statistics | 10 | `AVERAGEIF` `AVERAGEIFS` `SUMIFS` `COUNTIFS` `STDEV` `VAR` `LARGE` `SMALL` `RANK` `MODE` |
| Financial | 7 | `PMT` `FV` `PV` `RATE` `NPER` `NPV` `IRR` |
| Logic | 18 | `IF` `IFERROR` `AND` `OR` `NOT` `XOR` `ISBLANK` `ISNUMBER` `ISTEXT` `ISERROR` `ISNA` `ISEVEN` `ISODD` `IFNA` `IFS` `SWITCH` `TRUE` `FALSE` |
| Lookup | 9 | `VLOOKUP` `HLOOKUP` `INDEX` `MATCH` `CHOOSE` `ROWS` `COLUMNS` `ROW` `COLUMN` |
| Text | 21 | `CONCAT` `CONCATENATE` `TEXT` `LEFT` `RIGHT` `MID` `UPPER` `LOWER` `TRIM` `LEN` `PROPER` `SUBSTITUTE` `REPT` `FIND` `SEARCH` `REPLACE` `EXACT` `TEXTJOIN` `CHAR` `CODE` `VALUE` |
| Date and time | 13 | `TODAY` `NOW` `DATE` `YEAR` `MONTH` `DAY` `TIME` `HOUR` `MINUTE` `SECOND` `WEEKDAY` `EDATE` `EOMONTH` |

**How a call behaves:**

| Question | Answer |
|---|---|
| Is the function name case-sensitive | no. It is matched upper-cased, so `sum(a1:a3)` and `SUM(A1:A3)` are one call |
| An unknown name | reads `#N/A` (`TRAME-049`) |
| How arguments are separated | by commas. A range is one argument holding many values |
| A criterion, for `SUMIF` and its kin | text carrying an operator and an operand: `">5"`, `"<>done"`, or a bare value meaning equality |
| A criterion against a non-number | `>` and `>=` refuse it; `<>` is the one operator it satisfies |
| A lookup with no match flag | approximate, not exact (`TRAME-201`). `=VLOOKUP(4,A1:B3,2)` answers the row of key 3 |
| A lookup key that is text | matched without regard to case, Unicode letters included (`TRAME-202`) |
| An approximate lookup over keys out of order | reads `#N/A`, never a neighbouring row (`TRAME-197`). Blank keys are skipped |
| A total of decimals | summed exactly, then rounded once, so its order cannot change it (`TRAME-199`) |
| A total past the largest number | reads `#NUM!`, as does any result that is not a finite number (`TRAME-244`). Terms near the largest number that cancel leave the rest, in any order. The terms are then summed at a scale of 2^-64, so a rest below about 4.1e-289 loses bits, the same in any order |
| How a date is held | a date serial (`DateSerial`) on the 1899-12-30 epoch: integer part the day, fraction the time of day |
| How a calendar day is named | a civil date (`CivilDate`): a proleptic-Gregorian year, month and day, counted to and from its serial |
| Where `TODAY` and `NOW` read | the recomputation clock, never the system clock (`TRAME-053`) |

---

## 4. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| A function receives values, never references | passing references and letting each function resolve them | the machine already evaluates each argument for cycle detection to work through ranges, so a function resolving its own would be a second evaluation path, with a second set of cycle rules. |
| An empty total is zero, not an error | an error on an empty range | `=SUM(A1:A10)` over a range a user has not filled yet is the normal state of a sheet under construction, not a mistake. |
| Text that reads as a number counts in a total | counting only values typed as numbers | a value pasted from the web or imported from a CSV arrives as text, and a total that ignores it is wrong in a way the user cannot see. |
| The argument count is not a byte | keeping the compact encoding and capping the call | a `u8` count wrapped at 256, so the 256th argument turned `SUM` into a call with no arguments, which answered the last operand left on the stack instead of the total. A cap makes that a refusal instead of a wrong answer, and still refuses a call the sheet is entitled to make (`TRAME-052`). |
| A result that is not finite reads `#NUM!` | showing the infinity, or guarding each function on its own | an infinity is no value a sheet can hold, and each operator already answers `#NUM!` for one. One guard at the dispatch holds every function, the one added next included. |
| One clock per recomputation | reading the system clock in each call | two cells reading `NOW()` in the same pass must agree, or a sheet cannot subtract one from the other. |
| The bodies live in three modules, the dispatch in one | one module per family | it multiplies the imports for no gain. What the split has to fix is 112 arms in one file, a merge conflict per function added, and the dispatch is the part that must stay readable as a list. |

---

## 5. NOT BUILT, AND NOT YET RULED

* **Reference-preserving functions** (`TRAME-057`). `OFFSET` and `INDIRECT` are absent, and `ROW`
  with an argument answers `#N/A` on purpose. A function that returns a reference needs the machine
  to accept one, which `TRAME-050` says it does not. The rule and the seam move together.
* **The advanced set** (`TRAME-058`, `TRAME-059`, `TRAME-060`). No `XLOOKUP`, no `LAMBDA`, no spilling
  formula. A dynamic array needs a rule for the cells it fills and for the collision when they are not
  empty, which no document has written.
* **A total that skips filtered rows** (`TRAME-061`). `SUBTOTAL` is absent, and it is the deliberate
  exception to `TRAME-021`: every other result is independent of view state, and this one is not. What
  a peer with a different personal filter reads is the open question.
* **The catalogue has no accuracy oracle, and a fifth of it has no test at all.** The property
  tests cover the aggregates against a naive sum; the rest is covered by example, one assertion per
  case. Twenty-one of the 113 names appear in no test in the workspace. Among them are
  `AND`'s partner `OR`, every `IS*` predicate but `ISTEXT`'s neighbours, `ROUNDUP`, `ROUNDDOWN`,
  `FLOOR`, and the whole trigonometric set. Nothing compares `PMT` or `WEEKDAY` against a reference
  implementation either, so a wrong formula stays wrong and green.

