use std::fmt;

use ambre_bus::{CellPos, Col, Row, SheetId};

use crate::model::{Line, SheetIndex, Uid};

// -----------------------------------------------------------------------------
// EngineSheet -- the engine's local sheet index
// -----------------------------------------------------------------------------

/// A sheet's local index inside the formula engine.
///
/// The engine interns each sheet of the workbook when it is fed: the active
/// sheet is [`EngineSheet::ACTIVE`], the others follow by id. The index is
/// **not** the CRDT sheet id, and it holds only until the next reseed. A new
/// sheet whose id sorts among the others moves their indices, so a compiled
/// reference is valid only within the epoch that compiled it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct EngineSheet(u16);

impl EngineSheet {
    /// The active sheet, always index 0.
    pub const ACTIVE: Self = Self(0);

    /// The sheet at local index `index`.
    pub const fn new(index: u16) -> Self {
        Self(index)
    }

    /// The local index, as the engine's tables are laid out.
    pub const fn index(self) -> u16 {
        self.0
    }

    /// The local index as a position in a table laid out by sheet.
    pub const fn position(self) -> usize {
        self.0 as usize
    }
}

// -----------------------------------------------------------------------------
// CellRef -- positional cell identifier (AST / parser / renderer)
// -----------------------------------------------------------------------------

/// Identifies a cell by its sheet plus zero-based row and column index.
///
/// `sheet` is a local sheet index ([`EngineSheet`]) interned by the engine for the
/// duration of a recomputation; it is **not** the CRDT sheet id. A formula's
/// own sheet is the default for unqualified references (`A1`); a qualified
/// reference (`Sheet2!A1`) carries the resolved index. Single-sheet code uses
/// [`CellRef::new`], which defaults `sheet` to 0.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CellRef {
    pub sheet: EngineSheet,
    pub row: Row,
    pub col: Col,
}

impl CellRef {
    /// Cell on the default sheet (index 0). Used by single-sheet call sites.
    pub const fn new(row: Row, col: Col) -> Self {
        Self {
            sheet: EngineSheet::ACTIVE,
            row,
            col,
        }
    }

    /// The cell at `at` on the default sheet.
    pub const fn at(at: CellPos) -> Self {
        Self::new(at.row, at.col)
    }

    /// Cell on an explicit sheet index.
    pub const fn with_sheet(sheet: EngineSheet, row: Row, col: Col) -> Self {
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
    pub sheet: EngineSheet,
    pub row_start: Row,
    pub row_end: Row,
    pub col_start: Col,
    pub col_end: Col,
}

impl RangeDep {
    /// Range on the default sheet (index 0). Used by single-sheet call sites.
    pub fn new(row_start: Row, row_end: Row, col_start: Col, col_end: Col) -> Self {
        Self {
            sheet: EngineSheet::ACTIVE,
            row_start,
            row_end,
            col_start,
            col_end,
        }
    }

    /// Range on an explicit sheet index.
    pub fn with_sheet(
        sheet: EngineSheet,
        row_start: Row,
        row_end: Row,
        col_start: Col,
        col_end: Col,
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

    /// Number of cells spanned by this range.
    pub fn len(&self) -> usize {
        let rows = self.row_end.count_from(self.row_start).get() as usize + 1;
        let cols = self.col_end.count_from(self.col_start).get() as usize + 1;
        rows * cols
    }

    /// Returns `true` when the range spans zero cells.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// The lines a range takes whole, on one axis. Both bounds are inclusive, and
/// the first never comes after the last.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SpanLines {
    Rows { first: Row, last: Row },
    Cols { first: Col, last: Col },
}

/// A range of whole lines: `A:C` takes three whole columns, `2:5` four whole
/// rows (`TRAME-048`). On the other axis the range covers the used part of the
/// sheet when it is read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AxisSpan {
    /// Local sheet index (see [`CellRef::sheet`]).
    pub sheet: EngineSheet,
    /// The lines the range takes whole.
    pub lines: SpanLines,
}

impl AxisSpan {
    /// The rows between `a` and `b` on the default sheet, in either order.
    pub fn rows(a: Row, b: Row) -> Self {
        Self {
            sheet: EngineSheet::ACTIVE,
            lines: SpanLines::Rows {
                first: a.min(b),
                last: a.max(b),
            },
        }
    }

    /// The columns between `a` and `b` on the default sheet, in either order.
    pub fn cols(a: Col, b: Col) -> Self {
        Self {
            sheet: EngineSheet::ACTIVE,
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

impl<P: Line> AxisId<P> {
    /// The bound `Uid`, or `None` when the axis is virtual.
    pub fn uid(self) -> Option<Uid> {
        match self {
            Self::Bound(u) => Some(u),
            Self::Virtual(_) => None,
        }
    }

    /// The axis's position under `index`, or `None` for a deleted line.
    pub fn pos(self, index: &SheetIndex) -> Option<P> {
        match self {
            Self::Bound(u) => P::pos(index, &u),
            Self::Virtual(p) => Some(p),
        }
    }
}

/// Engine-side cell identity: a sheet plus a row and column [`AxisId`]. This is
/// the compiled, uid-anchored counterpart of the positional AST [`CellRef`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CellId {
    pub sheet: EngineSheet,
    pub row: AxisId<Row>,
    pub col: AxisId<Col>,
}

impl CellId {
    pub const fn new(sheet: EngineSheet, row: AxisId<Row>, col: AxisId<Col>) -> Self {
        Self { sheet, row, col }
    }

    /// Convenience constructor for a fully bound cell `(row_uid, col_uid)`.
    pub const fn bound(sheet: EngineSheet, row: Uid, col: Uid) -> Self {
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
    pub sheet: EngineSheet,
    pub row_start: RangeEndpoint<Row>,
    pub row_end: RangeEndpoint<Row>,
    pub col_start: RangeEndpoint<Col>,
    pub col_end: RangeEndpoint<Col>,
}

impl RangeId {
    /// Resolve the row endpoints to an inclusive `(start, end)` position pair, or
    /// `None` when a `Specific(Bound(_))` endpoint refers to a deleted row
    /// (`#REF!`). A `Virtual` endpoint keeps its typed position even beyond the
    /// used range (the caller clamps iteration to the materialized rows).
    pub fn row_range(&self, index: &SheetIndex) -> Option<(Row, Row)> {
        axis_range(&self.row_start, &self.row_end, index)
    }
    pub fn col_range(&self, index: &SheetIndex) -> Option<(Col, Col)> {
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
fn axis_range<P: Line>(
    start: &RangeEndpoint<P>,
    end: &RangeEndpoint<P>,
    index: &SheetIndex,
) -> Option<(P, P)> {
    let last = P::last(index).unwrap_or(P::FIRST);
    Some((
        endpoint_pos(start, P::FIRST, index)?,
        endpoint_pos(end, last, index)?,
    ))
}

/// Resolve a single range endpoint to a grid position. `entire` is the position
/// used for an unbounded `EntireAxis` endpoint. Returns `None` only when a
/// bound `Uid` is missing from the index.
fn endpoint_pos<P: Line>(ep: &RangeEndpoint<P>, entire: P, index: &SheetIndex) -> Option<P> {
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
    pub rows: usize,
    pub cols: usize,
    /// Row-major, `len == rows * cols`.
    pub values: Vec<CellValue>,
}

impl RangeValue {
    pub fn new(rows: usize, cols: usize, values: Vec<CellValue>) -> Self {
        debug_assert_eq!(values.len(), rows * cols);
        Self { rows, cols, values }
    }

    /// Value at zero-based `(row, col)`, or `None` if out of bounds.
    pub fn get(&self, row: usize, col: usize) -> Option<&CellValue> {
        if row >= self.rows || col >= self.cols {
            return None;
        }
        self.values.get(row * self.cols + col)
    }
}

/// The result of evaluating a cell: a literal value, an error, or an internal
/// range (used only on the VM stack, never persisted in `formula_cache`).
#[derive(Debug, Clone)]
pub enum CellValue {
    Number(f64),
    Text(String),
    Boolean(bool),
    Empty,
    /// Only on the VM stack during evaluation; never stored in `formula_cache`.
    Range(RangeValue),
    Error(FormulaError),
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
            CellValue::Text(s) => s.parse().map_err(|_| FormulaError::TypeError),
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
// FormulaError
// -----------------------------------------------------------------------------

/// Reasons a formula can fail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FormulaError {
    /// Two or more cells depend on each other (detected statically or
    /// dynamically).
    CircularReference,
    /// A formula failed to parse.
    ParseError(String),
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
    PushStr(String),
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
        name: String,
        n_args: usize,
    },
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
    /// The line with this identity, wherever it now sits. `#REF!` once deleted.
    Line(Uid),
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
    pub row: StoredAxis<Row>,
    pub col: StoredAxis<Col>,
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
    Name(String),
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
        name: String,
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
    Name(String),
    /// A stable CRDT `sheet_id` (`s0`, `00..0_0000002a`), prefixed `#` in text.
    Id(SheetId),
}

/// Resolution services the formula crate needs from the host (`TrameState`).
/// It maps a sheet *display name* or stable *id* to its local index, and a
/// defined name to its target. Implemented by the host; [`SingleSheetCtx`] is
/// the no-op default for single-sheet code and unit tests.
pub trait FormulaCtx {
    /// Resolve a sheet display name (`Sheet2`) to its local index, or `None`
    /// if no such sheet exists (the reference becomes a parse error).
    fn resolve_sheet(&self, name: &str) -> Option<EngineSheet>;
    /// Resolve a stable sheet id (`#s0_a1b2` canonical form) to its local index,
    /// or `None` if the sheet was deleted (the reference becomes `#REF!`).
    fn resolve_sheet_id(&self, id: &SheetId) -> Option<EngineSheet>;
    /// Resolve a defined name (`TOTAL`) to its target, or `None` if undefined.
    fn resolve_name(&self, name: &str) -> Option<NameTarget>;
}

/// Default context: a single sheet (index 0), no cross-sheet references, no
/// defined names. Any `Sheet!ref` or bareword name fails to resolve.
pub struct SingleSheetCtx;

impl FormulaCtx for SingleSheetCtx {
    fn resolve_sheet(&self, _name: &str) -> Option<EngineSheet> {
        None
    }
    fn resolve_sheet_id(&self, _id: &SheetId) -> Option<EngineSheet> {
        None
    }
    fn resolve_name(&self, _name: &str) -> Option<NameTarget> {
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

    #[test]
    fn col_to_alpha_basics() {
        assert_eq!(col_to_alpha(Col::new(0)), "A");
        assert_eq!(col_to_alpha(Col::new(25)), "Z");
        assert_eq!(col_to_alpha(Col::new(26)), "AA");
        assert_eq!(col_to_alpha(Col::new(51)), "AZ");
        assert_eq!(col_to_alpha(Col::new(52)), "BA");
        assert_eq!(col_to_alpha(Col::new(701)), "ZZ");
    }

    #[test]
    fn alpha_to_col_roundtrip() {
        for col in [0, 1, 25, 26, 51, 52, 100, 701] {
            let alpha = col_to_alpha(Col::new(col));
            assert_eq!(alpha_to_col(&alpha).unwrap(), Col::new(col));
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
        let r = RangeDep::new(Row::new(0), Row::new(9), Col::new(0), Col::new(4));
        assert!(r.contains(CellRef::new(Row::new(0), Col::new(0))));
        assert!(r.contains(CellRef::new(Row::new(9), Col::new(4))));
        assert!(!r.contains(CellRef::new(Row::new(10), Col::new(0))));
        assert!(!r.contains(CellRef::new(Row::new(0), Col::new(5))));
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
            CellValue::Error(FormulaError::ParseError("oops".into())).to_display(),
            "#PARSE!(oops)"
        );
    }

    // -- Engine uid range resolution -----------------------------------------

    use crate::formula::test_index::index_with;
    use crate::model::Uid;

    fn row_uid(p: u32) -> Uid {
        Uid {
            peer: ambre_core::PeerId(1),
            counter: p,
        }
    }
    fn col_uid(p: u32) -> Uid {
        Uid {
            peer: ambre_core::PeerId(2),
            counter: p,
        }
    }
    fn bound_range(r0: u32, r1: u32, c0: u32, c1: u32) -> RangeId {
        RangeId {
            sheet: EngineSheet::ACTIVE,
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
        assert_eq!(r.row_range(&idx), Some((Row::new(2), Row::new(5))));
        assert_eq!(r.col_range(&idx), Some((Col::new(1), Col::new(3))));
    }

    #[test]
    fn range_id_virtual_endpoint_keeps_typed_position() {
        // A1:A1000 on a 10-row sheet: the far endpoint sits past the used range,
        // so it stays a virtual position rather than resolving to a uid.
        let idx = index_with(10, 1);
        let r = RangeId {
            sheet: EngineSheet::ACTIVE,
            row_start: RangeEndpoint::Specific(AxisId::Bound(row_uid(0))),
            row_end: RangeEndpoint::Specific(AxisId::Virtual(Row::new(999))),
            col_start: RangeEndpoint::Specific(AxisId::Bound(col_uid(0))),
            col_end: RangeEndpoint::Specific(AxisId::Bound(col_uid(0))),
        };
        assert_eq!(r.row_range(&idx), Some((Row::ZERO, Row::new(999))));
    }

    #[test]
    fn range_id_deleted_bound_endpoint_is_broken() {
        // An endpoint bound to a uid absent from the index (a deleted row) makes
        // the whole range resolve to None -> the VM yields #REF!.
        let idx = index_with(10, 10);
        let r = RangeId {
            sheet: EngineSheet::ACTIVE,
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
