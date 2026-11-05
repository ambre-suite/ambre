use ambre_core::id::{Id, Sheet};
use ambre_core::index::{Axis, Len, Numbered, Pos, Span};
use ambre_core::{Displayed, Ordinal, OrdinalKind, Rule, Validated};
use std::fmt;

use crate::message::{DefinedName, SheetName};
use ambre_bus::{CellPos, Col, Row};

use crate::model::{Element, Line, SheetIndex, Uid};

// -----------------------------------------------------------------------------
// EngineSheet -- the engine's local sheet index
// -----------------------------------------------------------------------------

/// The engine's local sheet indexes: a [`Pos<EngineSheet>`] names a sheet
/// inside the formula engine.
///
/// The engine interns each sheet of the workbook when it is fed: the active
/// sheet is [`ACTIVE_SHEET`], the others follow by id. The index is **not**
/// the CRDT sheet id, and it holds only until the next reseed. A new sheet
/// whose id sorts among the others moves their indices, so a compiled
/// reference is valid only within the epoch that compiled it.
#[derive(PartialEq, Eq)]
pub enum EngineSheet {}

impl Axis for EngineSheet {
    const NAME: &'static str = "EngineSheet";
}

/// A reader sees an engine sheet counted from one, as a defined name's
/// target prints it.
impl OrdinalKind for EngineSheet {
    const NAME: &'static str = "EngineSheetNumber";
}

impl Numbered for EngineSheet {
    type Number = Ordinal<EngineSheet>;

    fn number_of(pos: Pos<EngineSheet>) -> Ordinal<EngineSheet> {
        Ordinal::<EngineSheet>::from_position(pos.to_usize())
    }
}

/// The active sheet, always index 0.
pub const ACTIVE_SHEET: Pos<EngineSheet> = Pos::ZERO;

// -----------------------------------------------------------------------------
// CellRef -- positional cell identifier (AST / parser / renderer)
// -----------------------------------------------------------------------------

/// Identifies a cell by its sheet plus zero-based row and column index.
///
/// `sheet` is a local sheet index ([`Pos<EngineSheet>`]) interned by the engine for the
/// duration of a recomputation; it is **not** the CRDT sheet id. A formula's
/// own sheet is the default for unqualified references (`A1`); a qualified
/// reference (`Sheet2!A1`) carries the resolved index. Single-sheet code uses
/// [`CellRef::new`], which defaults `sheet` to 0.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CellRef {
    pub sheet: Pos<EngineSheet>,
    pub row: Pos<Row>,
    pub col: Pos<Col>,
}

impl CellRef {
    /// Cell on the default sheet (index 0). Used by single-sheet call sites.
    pub const fn new(row: Pos<Row>, col: Pos<Col>) -> Self {
        Self {
            sheet: ACTIVE_SHEET,
            row,
            col,
        }
    }

    /// The cell at `at` on the default sheet.
    pub const fn at(at: CellPos) -> Self {
        Self::new(at.row, at.col)
    }

    /// Cell on an explicit sheet index.
    pub const fn with_sheet(sheet: Pos<EngineSheet>, row: Pos<Row>, col: Pos<Col>) -> Self {
        Self { sheet, row, col }
    }

    /// The grid position of the cell, its sheet left out.
    pub const fn pos(self) -> CellPos {
        CellPos::new(self.row, self.col)
    }
}

impl fmt::Display for CellRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Sheet is intentionally omitted: Display is used for in-cell error and
        // debug text, where the local sheet index carries no user meaning.
        write!(
            f,
            "{}{}",
            col_to_alpha(self.col),
            u64::from(self.row.get()) + 1
        )
    }
}

// -----------------------------------------------------------------------------
// RangeDep -- bounded cell range
// -----------------------------------------------------------------------------

/// Describes a rectangular range of cells.
///
/// Bounds are inclusive (`row_start..=row_end`, `col_start..=col_end`).
/// A range of whole lines is an [`AxisSpan`] instead.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RangeDep {
    /// Local sheet index (see [`CellRef::sheet`]). Defaults to 0 via
    /// [`RangeDep::new`]; a cross-sheet range carries the resolved index.
    pub sheet: Pos<EngineSheet>,
    pub row_start: Pos<Row>,
    pub row_end: Pos<Row>,
    pub col_start: Pos<Col>,
    pub col_end: Pos<Col>,
}

impl RangeDep {
    /// Range on the default sheet (index 0). Used by single-sheet call sites.
    pub fn new(
        row_start: Pos<Row>,
        row_end: Pos<Row>,
        col_start: Pos<Col>,
        col_end: Pos<Col>,
    ) -> Self {
        Self {
            sheet: ACTIVE_SHEET,
            row_start,
            row_end,
            col_start,
            col_end,
        }
    }

    /// Range on an explicit sheet index.
    pub fn with_sheet(
        sheet: Pos<EngineSheet>,
        row_start: Pos<Row>,
        row_end: Pos<Row>,
        col_start: Pos<Col>,
        col_end: Pos<Col>,
    ) -> Self {
        Self {
            sheet,
            row_start,
            row_end,
            col_start,
            col_end,
        }
    }

    /// Whether the given cell falls inside this range.
    pub fn contains(&self, cell: CellRef) -> bool {
        cell.row >= self.row_start
            && cell.row <= self.row_end
            && cell.col >= self.col_start
            && cell.col <= self.col_end
    }
}

/// The lines a range takes whole, on one axis. Both bounds are inclusive, and
/// the first never comes after the last.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SpanLines {
    Rows { first: Pos<Row>, last: Pos<Row> },
    Cols { first: Pos<Col>, last: Pos<Col> },
}

/// A range of whole lines: `A:C` takes three whole columns, `2:5` four whole
/// rows (`TRAME-048`). On the other axis the range covers the used part of the
/// sheet when it is read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AxisSpan {
    /// Local sheet index (see [`CellRef::sheet`]).
    pub sheet: Pos<EngineSheet>,
    /// The lines the range takes whole.
    pub lines: SpanLines,
}

impl AxisSpan {
    /// The rows between `a` and `b` on the default sheet, in either order.
    pub fn rows(a: Pos<Row>, b: Pos<Row>) -> Self {
        Self {
            sheet: ACTIVE_SHEET,
            lines: SpanLines::Rows {
                first: a.min(b),
                last: a.max(b),
            },
        }
    }

    /// The columns between `a` and `b` on the default sheet, in either order.
    pub fn cols(a: Pos<Col>, b: Pos<Col>) -> Self {
        Self {
            sheet: ACTIVE_SHEET,
            lines: SpanLines::Cols {
                first: a.min(b),
                last: a.max(b),
            },
        }
    }
}

// -----------------------------------------------------------------------------
// Engine-side uid identities (compiled references)
// -----------------------------------------------------------------------------
//
// The parser produces positional `CellRef` / `RangeDep` (a typed `A1`). The
// compiler resolves those against the current `SheetIndex` into the uid-anchored
// forms below. Those forms are what the bytecode, dependency graph, bytecode
// cache and VM are keyed by. Binding to the row/column `Uid` is what makes a
// reference follow the data through inserts, deletes and sorts
// (`trame-formula-engine`).

/// Engine-side identity of one axis (a row or a column) inside a compiled
/// reference.
///
/// Within the used range a reference binds to the row/column `Uid`, so it tracks
/// the physical line through any reorder. Beyond the used range lies a
/// *virtual* row/column that has no list element yet
/// (`trame-rows-and-columns`), and no `Uid` to bind. The reference therefore
/// keeps the grid position it was typed at, and evaluates `Empty` until that
/// line is materialized. A re-edit then re-binds it to the new `Uid`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AxisId<P> {
    /// Bound to a materialized row/column `Uid`. `#REF!` once that id is deleted.
    Bound(Uid),
    /// A virtual position beyond the used range; stays positional, reads `Empty`.
    Virtual(P),
}

impl<L: Line> AxisId<Pos<L>> {
    /// The bound `Uid`, or `None` when the axis is virtual.
    pub fn uid(self) -> Option<Uid> {
        match self {
            Self::Bound(u) => Some(u),
            Self::Virtual(_) => None,
        }
    }

    /// The axis's position under `index`, or `None` for a deleted line.
    pub fn pos(self, index: &SheetIndex) -> Option<Pos<L>> {
        match self {
            Self::Bound(u) => L::pos(index, &u),
            Self::Virtual(p) => Some(p),
        }
    }
}

/// Engine-side cell identity: a sheet plus a row and column [`AxisId`]. This is
/// the compiled, uid-anchored counterpart of the positional AST [`CellRef`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CellId {
    pub sheet: Pos<EngineSheet>,
    pub row: AxisId<Pos<Row>>,
    pub col: AxisId<Pos<Col>>,
}

impl CellId {
    pub const fn new(
        sheet: Pos<EngineSheet>,
        row: AxisId<Pos<Row>>,
        col: AxisId<Pos<Col>>,
    ) -> Self {
        Self { sheet, row, col }
    }

    /// Convenience constructor for a fully bound cell `(row_uid, col_uid)`.
    pub const fn bound(sheet: Pos<EngineSheet>, row: Uid, col: Uid) -> Self {
        Self {
            sheet,
            row: AxisId::Bound(row),
            col: AxisId::Bound(col),
        }
    }

    /// The bound row `Uid`, or `None` when the row axis is virtual.
    pub fn row_uid(&self) -> Option<Uid> {
        self.row.uid()
    }
    /// The bound column `Uid`, or `None` when the column axis is virtual.
    pub fn col_uid(&self) -> Option<Uid> {
        self.col.uid()
    }
}

/// One bound of a compiled range, on a single axis.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RangeEndpoint<P> {
    /// A bounded endpoint, stable via the row/column [`AxisId`]. `#REF!` if a
    /// bound `Uid` is deleted.
    Specific(AxisId<P>),
    /// An unbounded endpoint: entire column (`A:A`) or entire row (`1:1`). It
    /// reads the used part of the axis (`TRAME-048`).
    EntireAxis,
}

/// Engine-side range identity: the uid-anchored counterpart of the positional
/// AST [`RangeDep`]. Endpoints resolve to grid positions through the current
/// [`SheetIndex`] at evaluation time, so the spanned cells follow the data.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RangeId {
    pub sheet: Pos<EngineSheet>,
    pub row_start: RangeEndpoint<Pos<Row>>,
    pub row_end: RangeEndpoint<Pos<Row>>,
    pub col_start: RangeEndpoint<Pos<Col>>,
    pub col_end: RangeEndpoint<Pos<Col>>,
}

impl RangeId {
    /// Resolve the row endpoints to an inclusive `(start, end)` position pair, or
    /// `None` when a `Specific(Bound(_))` endpoint refers to a deleted row
    /// (`#REF!`). A `Virtual` endpoint keeps its typed position even beyond the
    /// used range (the caller clamps iteration to the materialized rows).
    pub fn row_range(&self, index: &SheetIndex) -> Option<(Pos<Row>, Pos<Row>)> {
        axis_range(&self.row_start, &self.row_end, index)
    }
    pub fn col_range(&self, index: &SheetIndex) -> Option<(Pos<Col>, Pos<Col>)> {
        axis_range(&self.col_start, &self.col_end, index)
    }

    /// Whether the bound cell `(row, col)` falls inside this range under the
    /// current `index`. Used by the dependency graph to dirty range formulas
    /// when one of their member cells changes.
    pub fn contains(&self, row: Uid, col: Uid, index: &SheetIndex) -> bool {
        let Some(at) = index.pos_of(&row, &col) else {
            return false;
        };
        matches!(self.row_range(index), Some((s, e)) if at.row >= s && at.row <= e)
            && matches!(self.col_range(index), Some((s, e)) if at.col >= s && at.col <= e)
    }
}

/// Resolve the two endpoints of one axis to an inclusive pair of positions. An
/// unbounded start reads the first line, an unbounded end the last one.
fn axis_range<L: Line>(
    start: &RangeEndpoint<Pos<L>>,
    end: &RangeEndpoint<Pos<L>>,
    index: &SheetIndex,
) -> Option<(Pos<L>, Pos<L>)> {
    let last = L::len(index).last().unwrap_or(Pos::ZERO);
    Some((
        endpoint_pos(start, Pos::ZERO, index)?,
        endpoint_pos(end, last, index)?,
    ))
}

/// Resolve a single range endpoint to a grid position. `entire` is the position
/// used for an unbounded `EntireAxis` endpoint. Returns `None` only when a
/// bound `Uid` is missing from the index.
fn endpoint_pos<L: Line>(
    ep: &RangeEndpoint<Pos<L>>,
    entire: Pos<L>,
    index: &SheetIndex,
) -> Option<Pos<L>> {
    match ep {
        RangeEndpoint::EntireAxis => Some(entire),
        RangeEndpoint::Specific(axis) => axis.pos(index),
    }
}

// -----------------------------------------------------------------------------
// CellValue -- runtime value in the stack VM
// -----------------------------------------------------------------------------

/// Whether two floats are equal to within a few units in the last place.
///
/// A fixed absolute tolerance such as `f64::EPSILON` (~2.2e-16) is meaningless
/// once the operands leave the neighbourhood of 1.0. Below it, `1e-16` and
/// `2e-16` are two distinct numbers it declares equal. Scaling the tolerance by
/// the operands' magnitude keeps the test at "equal to a few ulps" everywhere.
/// That is also what Excel's 15-significant-digit comparison amounts to.
pub fn nearly_equal(x: f64, y: f64) -> bool {
    // Exact hit first: covers both zeros and same-sign infinities, where the
    // relative test below would compute `inf - inf`.
    if x == y {
        return true;
    }
    let scale = x.abs().max(y.abs());
    (x - y).abs() <= scale * 1e-15
}

/// A rectangular block of values produced by evaluating a range reference.
///
/// Values are stored row-major (`values[row * cols + col]`). The 2D shape is
/// kept (unlike a flat `Vec`) so functions such as `VLOOKUP`, `INDEX` and
/// `MATCH` can address cells by row/column. Aggregation functions that only
/// need a flat stream iterate `values` directly.
#[derive(Debug, Clone)]
pub struct RangeValue {
    pub rows: Len<Row>,
    pub cols: Len<Col>,
    /// Row-major, `len == rows * cols`.
    pub values: Vec<CellValue>,
}

impl RangeValue {
    pub fn new(rows: Len<Row>, cols: Len<Col>, values: Vec<CellValue>) -> Self {
        debug_assert_eq!(values.len(), rows.to_usize() * cols.to_usize());
        Self { rows, cols, values }
    }

    /// The single `#REF!` a range collapses to when it cannot be read.
    pub fn broken_ref() -> Self {
        Self::new(
            Len::<Row>::ONE,
            Len::<Col>::ONE,
            vec![CellValue::Error(FormulaError::BrokenRef)],
        )
    }

    /// The rows of the block, counted from its own top row.
    pub fn row_offsets(&self) -> Span<Row> {
        Span::<Row>::from_len(Pos::<Row>::ZERO, self.rows)
    }

    /// The columns of the block, counted from its own left column.
    pub fn col_offsets(&self) -> Span<Col> {
        Span::<Col>::from_len(Pos::<Col>::ZERO, self.cols)
    }

    /// Value at `(row, col)` from the block's top-left cell, or `None` if out
    /// of bounds. The offsets are relative to the block, not to the sheet.
    pub fn get(&self, row: Pos<Row>, col: Pos<Col>) -> Option<&CellValue> {
        if row >= self.rows.as_end() || col >= self.cols.as_end() {
            return None;
        }
        self.values
            .get(row.to_index()? * self.cols.to_usize() + col.to_index()?)
    }
}

/// The result of evaluating a cell: a literal value, an error, or an internal
/// range (used only on the VM stack, never persisted in `formula_cache`).
#[derive(Clone)]
pub enum CellValue {
    Number(f64),
    Text(String),
    Boolean(bool),
    Empty,
    /// Only on the VM stack during evaluation; never stored in `formula_cache`.
    Range(RangeValue),
    Error(FormulaError),
}

/// The length of a text alone: a cell's text is document content.
impl fmt::Debug for CellValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Number(n) => f.debug_tuple("Number").field(n).finish(),
            Self::Text(text) => write!(f, "Text({} chars)", text.chars().count()),
            Self::Boolean(b) => f.debug_tuple("Boolean").field(b).finish(),
            Self::Empty => write!(f, "Empty"),
            Self::Range(range) => f.debug_tuple("Range").field(range).finish(),
            Self::Error(error) => f.debug_tuple("Error").field(error).finish(),
        }
    }
}

/// The number `text` spells, when it is a finite one (`TRAME-245`).
///
/// Rust's parse also reads `NaN`, `inf` and `infinity` in any case, and an
/// overflowing literal such as `1e999` as an infinity. None of them is a
/// number a cell holds, so each stays text, as in Excel.
pub(crate) fn finite_number(text: &str) -> Option<f64> {
    text.parse::<f64>().ok().filter(|n| n.is_finite())
}

impl CellValue {
    /// Coerce this value to a numeric `f64`, or return an error.
    ///
    /// # Errors
    ///
    /// [`FormulaError::TypeError`] on text that does not parse as a number, and
    /// on a range. An error value returns itself.
    pub fn as_number(&self) -> Result<f64, FormulaError> {
        match self {
            CellValue::Number(n) => Ok(*n),
            CellValue::Text(s) => finite_number(s).ok_or(FormulaError::TypeError),
            CellValue::Boolean(b) => Ok(if *b { 1.0 } else { 0.0 }),
            CellValue::Empty => Ok(0.0),
            CellValue::Range(_) => Err(FormulaError::TypeError),
            CellValue::Error(e) => Err(e.clone()),
        }
    }

    /// String representation suitable for display in a spreadsheet cell.
    pub fn to_display(&self) -> String {
        match self {
            CellValue::Number(n) => {
                // Strip trailing zeros for cleaner display.
                if *n == n.trunc() && n.is_finite() {
                    format!("{:.0}", n)
                } else {
                    format!("{n}")
                }
            }
            CellValue::Text(s) => s.clone(),
            CellValue::Boolean(b) => format!("{b}"),
            CellValue::Empty => String::new(),
            CellValue::Range(r) => {
                // A range on the stack is only consumed by functions;
                // if it surfaces as a display value, show the count.
                format!("[{} values]", r.values.len())
            }
            CellValue::Error(e) => format!("{e}"),
        }
    }
}

// -----------------------------------------------------------------------------
// ParseError
// -----------------------------------------------------------------------------

/// Why a formula's text did not parse, or named a sheet no tab carries. Its
/// display is the message `#PARSE!(...)` shows.
#[derive(Clone, PartialEq, Eq)]
pub enum ParseError {
    /// Nothing but blanks after the `=`.
    Empty,
    /// More characters than the parser reads (`TRAME-013`).
    TooLong,
    /// Nested deeper than the parser reads (`TRAME-013`).
    TooDeep,
    /// More operators than the parser reads (`TRAME-013`).
    TooManyOperators,
    /// Text the grammar does not take after a whole expression.
    TrailingInput(String),
    /// The grammar stopped here: the first characters of what it left.
    Near(String),
    /// The text ended inside an expression.
    Incomplete,
    /// A sheet display name no tab carries, as the formula spells it.
    UnknownSheet(SheetName),
}

/// The length alone of the formula text a variant quotes: that text is
/// document content. The sheet name prints through its own type.
impl fmt::Debug for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => write!(f, "Empty"),
            Self::TooLong => write!(f, "TooLong"),
            Self::TooDeep => write!(f, "TooDeep"),
            Self::TooManyOperators => write!(f, "TooManyOperators"),
            Self::TrailingInput(text) => {
                write!(f, "TrailingInput({} chars)", text.chars().count())
            }
            Self::Near(text) => write!(f, "Near({} chars)", text.chars().count()),
            Self::Incomplete => write!(f, "Incomplete"),
            Self::UnknownSheet(name) => f.debug_tuple("UnknownSheet").field(name).finish(),
        }
    }
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        use super::parser::{MAX_FORMULA_CHARS, MAX_FORMULA_NESTING, MAX_FORMULA_OPERATORS};
        match self {
            Self::Empty => write!(f, "empty formula"),
            Self::TooLong => write!(
                f,
                "formula longer than {} characters",
                MAX_FORMULA_CHARS.to_usize()
            ),
            Self::TooDeep => write!(
                f,
                "formula nested deeper than {} levels",
                MAX_FORMULA_NESTING
            ),
            Self::TooManyOperators => {
                write!(
                    f,
                    "formula with more than {} operators",
                    MAX_FORMULA_OPERATORS
                )
            }
            Self::TrailingInput(rest) => write!(f, "unexpected trailing input: '{rest}'"),
            Self::Near(near) => write!(f, "parse error near '{near}'"),
            Self::Incomplete => write!(f, "incomplete input"),
            Self::UnknownSheet(name) => write!(f, "unknown sheet: {name}"),
        }
    }
}

impl std::error::Error for ParseError {}

// -----------------------------------------------------------------------------
// FormulaError
// -----------------------------------------------------------------------------

/// Reasons a formula can fail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FormulaError {
    /// Two or more cells depend on each other (detected statically or
    /// dynamically).
    CircularReference,
    /// A formula failed to parse.
    ParseError(ParseError),
    /// A reference points at something that no longer exists (a deleted sheet,
    /// row, or column). Rendered as `#REF!`.
    BrokenRef,
    /// Division by zero.
    DivisionByZero,
    /// Operands have incompatible types.
    TypeError,
    /// A formula produced a numeric overflow or invalid math operation.
    NumError,
    /// No applicable value (for example VLOOKUP did not find a match).
    NaError,
    /// A name the workbook does not define (`TRAME-065`). Rendered as `#NAME?`.
    NameError,
}

impl fmt::Display for FormulaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FormulaError::CircularReference => write!(f, "#CIRCULAR!"),
            FormulaError::ParseError(msg) => write!(f, "#PARSE!({msg})"),
            FormulaError::BrokenRef => write!(f, "#REF!"),
            FormulaError::DivisionByZero => write!(f, "#DIV/0!"),
            FormulaError::TypeError => write!(f, "#TYPE!"),
            FormulaError::NumError => write!(f, "#NUM!"),
            FormulaError::NaError => write!(f, "#N/A"),
            FormulaError::NameError => write!(f, "#NAME?"),
        }
    }
}

// -----------------------------------------------------------------------------
// Op -- bytecode instruction
// -----------------------------------------------------------------------------

/// A single instruction in the stack VM bytecode.
///
/// Compiled once per formula, executed on every recomputation
/// (`trame-formula-engine`).
#[derive(Debug, Clone)]
pub enum Op {
    // -- Stack literals --------------------------------------------------------
    PushNum(f64),
    PushStr(super::text::LiteralText),
    PushBool(bool),
    PushEmpty,
    /// Push a constant error onto the stack. Emitted for an error literal,
    /// for example a cross-sheet reference whose sheet was deleted (`#REF!`).
    PushError(FormulaError),

    // -- Cell / range references (resolved at eval time) -----------------------
    /// Push the evaluated value of another cell onto the stack.
    PushCell(CellId),
    /// Evaluate every cell in a range and push them as `CellValue::Range`.
    EvalRange(RangeId),

    // -- Binary arithmetic -----------------------------------------------------
    Add,
    Sub,
    Mul,
    Div,
    Pow,

    // -- Comparisons (push `CellValue::Boolean`) -------------------------------
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,

    // -- String ----------------------------------------------------------------
    /// Pop two values, push their string concatenation.
    Concat,

    // -- Unary -----------------------------------------------------------------
    Neg,
    Not,

    // -- Function call ---------------------------------------------------------
    /// `n_args` values are popped from the stack and passed to the named
    /// built-in function; the result is pushed back.
    CallFn {
        name: Validated<FunctionName>,
        n_args: Len<Arg>,
    },
}

/// The place of an argument in a formula call, from zero. Its own type,
/// so the place of a value inside a range argument cannot take it.
#[derive(PartialEq, Eq)]
pub enum Arg {}

impl Axis for Arg {
    const NAME: &'static str = "Arg";
}

/// The place of a value among the values of a range argument, from zero.
/// Its own type, so the place of an argument in the call cannot take it.
#[derive(PartialEq, Eq)]
pub enum Value {}

impl Axis for Value {
    const NAME: &'static str = "Value";
}

// -----------------------------------------------------------------------------
// CellBytecode -- compiled formula for one cell
// -----------------------------------------------------------------------------

/// The compiled bytecode for a formula cell, plus its extracted dependencies
/// (used to update the [`DependencyGraph`](super::dag::DependencyGraph)).
#[derive(Debug, Clone)]
pub struct CellBytecode {
    pub ops: Vec<Op>,
    /// Cell references extracted at compile time (uid-anchored).
    pub cell_deps: Vec<CellId>,
    /// Range dependencies extracted at compile time (uid-anchored).
    pub range_deps: Vec<RangeId>,
}

// -----------------------------------------------------------------------------
// Stored references -- the identity form a formula is kept in
// -----------------------------------------------------------------------------

/// One axis of a stored reference (`TRAME-228`): the row or column it was typed
/// against, or a position when that line did not exist yet.
///
/// A position past the used range has no line to name, so it stays positional
/// and reads empty (`TRAME-006`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StoredAxis<P> {
    /// The line of this element, wherever it now sits. `#REF!` once the
    /// element is gone, even when another element carries the uid again
    /// (`TRAME-278`).
    Line(Element),
    /// A zero-based position with no line behind it when the formula was stored.
    At(P),
    /// Every line of the axis, written `*`: the far axis of a whole-column or
    /// whole-row range (`TRAME-048`).
    Whole,
}

/// A cell reference in stored form: one [`StoredAxis`] per axis. Written
/// `{row|col}` in the stored text, each axis either a uid or a position.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct StoredCell {
    pub row: StoredAxis<Pos<Row>>,
    pub col: StoredAxis<Pos<Col>>,
}

impl StoredCell {
    /// The stored form of a positional cell, with no line behind either axis.
    pub const fn at(cell: CellRef) -> Self {
        Self {
            row: StoredAxis::At(cell.row),
            col: StoredAxis::At(cell.col),
        }
    }

    /// The positional cell, when neither axis names a line.
    pub fn as_position(&self) -> Option<CellRef> {
        match (self.row, self.col) {
            (StoredAxis::At(row), StoredAxis::At(col)) => Some(CellRef::new(row, col)),
            _ => None,
        }
    }
}

/// The rule of the name of a function a formula calls, upper-cased. A
/// `Validated<FunctionName>` holds one. It reads as the parser reads it: a
/// letter or `_`, then letters, digits and `_`, all ASCII. The rule keeps the
/// name upper-cased.
///
/// A name no built-in answers is still a name: the call evaluates to `#N/A`,
/// and the formula renders it back as written.
pub enum FunctionName {}

/// A text that is no identifier, so it names no function.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NotAFunctionName;

impl fmt::Display for NotAFunctionName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a function name is an ASCII identifier")
    }
}

impl Rule for FunctionName {
    type Error = NotAFunctionName;

    fn admit(text: String) -> Result<String, NotAFunctionName> {
        let mut chars = text.chars();
        let head = chars.next().ok_or(NotAFunctionName)?;
        let ok = (head.is_ascii_alphabetic() || head == '_')
            && chars.all(|c| c.is_ascii_alphanumeric() || c == '_');
        if ok {
            Ok(text.to_ascii_uppercase())
        } else {
            Err(NotAFunctionName)
        }
    }
}

/// The renderer writes the call back into the formula text through `Display`.
/// `Debug` masks the name: an unknown one is text the user typed.
impl Displayed for FunctionName {}

// -----------------------------------------------------------------------------
// Expr -- AST produced by the parser
// -----------------------------------------------------------------------------

/// The abstract syntax tree of a parsed formula, fed into the compiler.
///
/// [`Expr::SheetRef`] and [`Expr::Name`] are produced by the (context-free)
/// parser and removed by [`resolve_refs`](super::parser::resolve_refs) before
/// compilation. The resolver rewrites them into [`Expr::CellRef`] /
/// [`Expr::Range`] carrying the resolved sheet index. The compiler therefore
/// never sees them. [`Expr::StoredCell`] and [`Expr::StoredRange`] are the
/// stored text's references; [`unbind`](super::identity::unbind) turns them
/// back into positions before resolution.
#[derive(Debug, Clone)]
pub enum Expr {
    Literal(CellValue),
    CellRef(CellRef),
    Range(RangeDep),
    /// A cell named by the lines it was typed against (`TRAME-228`).
    StoredCell(StoredCell),
    /// A range whose two corners are named by their lines. A corner may sit
    /// above or left of the other: the range is the rectangle between them.
    StoredRange(StoredCell, StoredCell),
    /// A range of whole columns or whole rows, `A:C` or `2:5` (`TRAME-048`).
    AxisSpan(AxisSpan),
    /// A sheet-qualified reference `Sheet2!A1` / `'My Sheet'!A1:B2` /
    /// `#s0_a1b2!A1`. `inner` is always a [`Expr::CellRef`] or [`Expr::Range`];
    /// the resolver stamps the resolved sheet index onto it.
    SheetRef {
        sheet: SheetQualifier,
        inner: Box<Expr>,
    },
    /// A bareword reference to a named range, resolved by the context.
    Name(Validated<DefinedName>),
    BinOp {
        kind: BinOpKind,
        lhs: Box<Expr>,
        rhs: Box<Expr>,
    },
    UnaryOp {
        kind: UnaryOpKind,
        operand: Box<Expr>,
    },
    FnCall {
        name: Validated<FunctionName>,
        args: Vec<Expr>,
    },
}

// -----------------------------------------------------------------------------
// Name resolution context
// -----------------------------------------------------------------------------

/// What a named range resolves to: a single cell or a rectangular range. Both
/// carry their own sheet index (a name is defined against a specific sheet).
#[derive(Debug, Clone)]
pub enum NameTarget {
    Cell(CellRef),
    Range(RangeDep),
}

/// How a sheet-qualified reference names its sheet.
///
/// User input and named-range definitions carry a [`SheetQualifier::Name`]
/// (display name). The canonical form persisted in the CRDT `v` text carries a
/// [`SheetQualifier::Id`] (the stable `sheet_id`), so rename/reorder never
/// invalidate the reference (`trame-sheets`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SheetQualifier {
    /// A display name (`Sheet2`, `'My Sheet'`).
    Name(SheetName),
    /// A stable CRDT `sheet_id` (`s0`, `00..0_0000002a`), prefixed `#` in text.
    Id(Id<Sheet>),
}

/// Resolution services the formula crate needs from the host (`TrameState`).
/// It maps a sheet *display name* or stable *id* to its local index, and a
/// defined name to its target. Implemented by the host; [`SingleSheetCtx`] is
/// the no-op default for single-sheet code and unit tests.
pub trait FormulaCtx {
    /// Resolve a sheet display name (`Sheet2`) to its local index, or `None`
    /// if no such sheet exists (the reference becomes a parse error).
    fn resolve_sheet(&self, name: &SheetName) -> Option<Pos<EngineSheet>>;
    /// Resolve a stable sheet id (`#s0_a1b2` canonical form) to its local index,
    /// or `None` if the sheet was deleted (the reference becomes `#REF!`).
    fn resolve_sheet_id(&self, id: &Id<Sheet>) -> Option<Pos<EngineSheet>>;
    /// Resolve a defined name (`TOTAL`) to its target, or `None` if undefined.
    fn resolve_name(&self, name: &Validated<DefinedName>) -> Option<NameTarget>;
}

/// Default context: a single sheet (index 0), no cross-sheet references, no
/// defined names. Any `Sheet!ref` or bareword name fails to resolve.
pub struct SingleSheetCtx;

impl FormulaCtx for SingleSheetCtx {
    fn resolve_sheet(&self, _name: &SheetName) -> Option<Pos<EngineSheet>> {
        None
    }
    fn resolve_sheet_id(&self, _id: &Id<Sheet>) -> Option<Pos<EngineSheet>> {
        None
    }
    fn resolve_name(&self, _name: &Validated<DefinedName>) -> Option<NameTarget> {
        None
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOpKind {
    Add,
    Sub,
    Mul,
    Div,
    Pow,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    Concat,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOpKind {
    Neg,
    Not,
}

impl BinOpKind {
    /// Maps a `BinOpKind` to its stack VM instruction.
    pub fn to_op(self) -> Op {
        match self {
            BinOpKind::Add => Op::Add,
            BinOpKind::Sub => Op::Sub,
            BinOpKind::Mul => Op::Mul,
            BinOpKind::Div => Op::Div,
            BinOpKind::Pow => Op::Pow,
            BinOpKind::Eq => Op::Eq,
            BinOpKind::Ne => Op::Ne,
            BinOpKind::Lt => Op::Lt,
            BinOpKind::Le => Op::Le,
            BinOpKind::Gt => Op::Gt,
            BinOpKind::Ge => Op::Ge,
            BinOpKind::Concat => Op::Concat,
        }
    }
}

// -----------------------------------------------------------------------------
// Helpers
// -----------------------------------------------------------------------------

// Column-label conversions moved to the crate-level `columns` module (single
// source of truth); re-exported here so existing `types::col_to_alpha` /
// `types::alpha_to_col` call sites keep compiling.
pub(crate) use crate::columns::{alpha_to_col, col_to_alpha};

#[cfg(test)]
mod tests {
    use super::*;

    /// A cell's text, a string literal and the formula text a parse error
    /// quotes are document content: each Debug form holds a length alone.
    #[test]
    fn debug_forms_print_the_length_of_document_text_alone() {
        let secret = || "salary".to_owned();
        let printed = [
            format!("{:?}", CellValue::Text(secret())),
            format!(
                "{:?}",
                Op::PushStr(crate::formula::text::LiteralText::new(secret()))
            ),
            format!("{:?}", Expr::Literal(CellValue::Text(secret()))),
            format!("{:?}", ParseError::TrailingInput(secret())),
            format!("{:?}", ParseError::Near(secret())),
            format!("{:?}", FormulaError::ParseError(ParseError::Near(secret()))),
        ];
        for line in &printed {
            assert!(!line.contains("salary"), "{line}");
            assert!(line.contains("6 chars"), "{line}");
        }
    }

    #[test]
    fn col_to_alpha_basics() {
        assert_eq!(col_to_alpha(Pos::<Col>::new(0)), "A");
        assert_eq!(col_to_alpha(Pos::<Col>::new(25)), "Z");
        assert_eq!(col_to_alpha(Pos::<Col>::new(26)), "AA");
        assert_eq!(col_to_alpha(Pos::<Col>::new(51)), "AZ");
        assert_eq!(col_to_alpha(Pos::<Col>::new(52)), "BA");
        assert_eq!(col_to_alpha(Pos::<Col>::new(701)), "ZZ");
    }

    #[test]
    fn alpha_to_col_roundtrip() {
        for col in [0, 1, 25, 26, 51, 52, 100, 701] {
            let alpha = col_to_alpha(Pos::<Col>::new(col));
            assert_eq!(alpha_to_col(&alpha).unwrap(), Pos::<Col>::new(col));
        }
    }

    #[test]
    fn alpha_to_col_invalid() {
        assert!(alpha_to_col("").is_err());
        assert!(alpha_to_col("a1").is_err());
        assert!(alpha_to_col("1").is_err());
    }

    #[test]
    fn range_contains() {
        let r = RangeDep::new(
            Pos::<Row>::new(0),
            Pos::<Row>::new(9),
            Pos::<Col>::new(0),
            Pos::<Col>::new(4),
        );
        assert!(r.contains(CellRef::new(Pos::<Row>::new(0), Pos::<Col>::new(0))));
        assert!(r.contains(CellRef::new(Pos::<Row>::new(9), Pos::<Col>::new(4))));
        assert!(!r.contains(CellRef::new(Pos::<Row>::new(10), Pos::<Col>::new(0))));
        assert!(!r.contains(CellRef::new(Pos::<Row>::new(0), Pos::<Col>::new(5))));
    }

    #[test]
    fn cell_value_display() {
        assert_eq!(CellValue::Number(42.0).to_display(), "42");
        assert_eq!(CellValue::Number(1.23).to_display(), "1.23");
        assert_eq!(CellValue::Text("hello".into()).to_display(), "hello");
        assert_eq!(CellValue::Boolean(true).to_display(), "true");
        assert_eq!(CellValue::Empty.to_display(), "");
        assert_eq!(
            CellValue::Error(FormulaError::DivisionByZero).to_display(),
            "#DIV/0!"
        );
        assert_eq!(
            CellValue::Error(FormulaError::ParseError(ParseError::Incomplete)).to_display(),
            "#PARSE!(incomplete input)"
        );
    }

    /// A function name reads upper-cased, and only an identifier is one.
    #[test]
    fn a_function_name_is_an_upper_cased_identifier() {
        assert_eq!(
            Validated::<FunctionName>::parse("sum").map(|n| n.to_string()),
            Ok("SUM".into())
        );
        assert_eq!(
            Validated::<FunctionName>::parse("_x1").map(|n| n.to_string()),
            Ok("_X1".into())
        );
        assert_eq!(
            Validated::<FunctionName>::parse("sum").map(|n| format!("{n:?}")),
            Ok("3 chars".into())
        );
        for bad in ["", "1A", "A-B", "\u{e9}t\u{e9}"] {
            assert_eq!(
                Validated::<FunctionName>::parse(bad),
                Err(NotAFunctionName),
                "{bad}"
            );
        }
    }

    /// Each parse failure reads the message the parser wrote as text before
    /// it had a type of its own.
    #[test]
    fn a_parse_error_reads_its_message() {
        let cases = [
            (ParseError::Empty, "empty formula"),
            (ParseError::TooLong, "formula longer than 8192 characters"),
            (ParseError::TooDeep, "formula nested deeper than 64 levels"),
            (
                ParseError::TooManyOperators,
                "formula with more than 512 operators",
            ),
            (
                ParseError::TrailingInput(")".to_owned()),
                "unexpected trailing input: ')'",
            ),
            (ParseError::Near("+*".to_owned()), "parse error near '+*'"),
            (ParseError::Incomplete, "incomplete input"),
            (
                ParseError::UnknownSheet(SheetName::stored("Nope".to_owned())),
                "unknown sheet: Nope",
            ),
        ];
        for (error, message) in cases {
            assert_eq!(error.to_string(), message);
        }
    }

    // -- Engine uid range resolution -----------------------------------------

    use crate::formula::test_index::index_with;
    use crate::model::Uid;

    fn row_uid(p: u32) -> Uid {
        Uid {
            peer: ambre_core::PeerId::new(1),
            counter: ambre_core::IdCounter::new(p),
        }
    }
    fn col_uid(p: u32) -> Uid {
        Uid {
            peer: ambre_core::PeerId::new(2),
            counter: ambre_core::IdCounter::new(p),
        }
    }
    fn bound_range(r0: u32, r1: u32, c0: u32, c1: u32) -> RangeId {
        RangeId {
            sheet: ACTIVE_SHEET,
            row_start: RangeEndpoint::Specific(AxisId::Bound(row_uid(r0))),
            row_end: RangeEndpoint::Specific(AxisId::Bound(row_uid(r1))),
            col_start: RangeEndpoint::Specific(AxisId::Bound(col_uid(c0))),
            col_end: RangeEndpoint::Specific(AxisId::Bound(col_uid(c1))),
        }
    }

    #[test]
    fn range_id_resolves_bound_endpoints_to_positions() {
        let idx = index_with(10, 10);
        let r = bound_range(2, 5, 1, 3);
        assert_eq!(
            r.row_range(&idx),
            Some((Pos::<Row>::new(2), Pos::<Row>::new(5)))
        );
        assert_eq!(
            r.col_range(&idx),
            Some((Pos::<Col>::new(1), Pos::<Col>::new(3)))
        );
    }

    #[test]
    fn range_id_virtual_endpoint_keeps_typed_position() {
        // A1:A1000 on a 10-row sheet: the far endpoint sits past the used range,
        // so it stays a virtual position rather than resolving to a uid.
        let idx = index_with(10, 1);
        let r = RangeId {
            sheet: ACTIVE_SHEET,
            row_start: RangeEndpoint::Specific(AxisId::Bound(row_uid(0))),
            row_end: RangeEndpoint::Specific(AxisId::Virtual(Pos::<Row>::new(999))),
            col_start: RangeEndpoint::Specific(AxisId::Bound(col_uid(0))),
            col_end: RangeEndpoint::Specific(AxisId::Bound(col_uid(0))),
        };
        assert_eq!(
            r.row_range(&idx),
            Some((Pos::<Row>::ZERO, Pos::<Row>::new(999)))
        );
    }

    #[test]
    fn range_id_deleted_bound_endpoint_is_broken() {
        // An endpoint bound to a uid absent from the index (a deleted row) makes
        // the whole range resolve to None -> the VM yields #REF!.
        let idx = index_with(10, 10);
        let r = RangeId {
            sheet: ACTIVE_SHEET,
            row_start: RangeEndpoint::Specific(AxisId::Bound(row_uid(0))),
            row_end: RangeEndpoint::Specific(AxisId::Bound(row_uid(999))), // not materialized
            col_start: RangeEndpoint::Specific(AxisId::Bound(col_uid(0))),
            col_end: RangeEndpoint::Specific(AxisId::Bound(col_uid(0))),
        };
        assert_eq!(r.row_range(&idx), None);
    }

    #[test]
    fn range_id_contains_only_bound_cells_inside_span() {
        let idx = index_with(10, 10);
        let r = bound_range(1, 3, 1, 2); // rows 1..=3, cols 1..=2
        assert!(r.contains(row_uid(2), col_uid(1), &idx));
        assert!(!r.contains(row_uid(0), col_uid(1), &idx), "row above span");
        assert!(
            !r.contains(row_uid(2), col_uid(3), &idx),
            "col right of span"
        );
        assert!(
            !r.contains(row_uid(999), col_uid(1), &idx),
            "deleted row not contained"
        );
    }

    #[test]
    fn cell_value_as_number() {
        assert_eq!(CellValue::Number(3.0).as_number().unwrap(), 3.0);
        assert_eq!(CellValue::Text("5".into()).as_number().unwrap(), 5.0);
        assert_eq!(CellValue::Boolean(true).as_number().unwrap(), 1.0);
        assert_eq!(CellValue::Boolean(false).as_number().unwrap(), 0.0);
        assert_eq!(CellValue::Empty.as_number().unwrap(), 0.0);
        assert!(matches!(
            CellValue::Text("abc".into()).as_number(),
            Err(FormulaError::TypeError)
        ));
    }
}
