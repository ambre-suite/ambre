use std::fmt;

// -----------------------------------------------------------------------------
// CellRef — positional cell identifier
// -----------------------------------------------------------------------------

/// Identifies a cell by its sheet plus zero-based row and column index.
///
/// `sheet` is a local sheet index (`u16`) interned by the engine for the
/// duration of a recomputation; it is **not** the CRDT sheet id. A formula's
/// own sheet is the default for unqualified references (`A1`); a qualified
/// reference (`Sheet2!A1`) carries the resolved index. Single-sheet code uses
/// [`CellRef::new`], which defaults `sheet` to 0.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CellRef {
    pub sheet: u16,
    pub row: u32,
    pub col: u32,
}

impl CellRef {
    /// Cell on the default sheet (index 0). Used by single-sheet call sites.
    pub const fn new(row: u32, col: u32) -> Self {
        Self { sheet: 0, row, col }
    }

    /// Cell on an explicit sheet index.
    pub const fn with_sheet(sheet: u16, row: u32, col: u32) -> Self {
        Self { sheet, row, col }
    }
}

impl fmt::Display for CellRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Sheet is intentionally omitted: Display is used for in-cell error and
        // debug text, where the local sheet index carries no user meaning.
        write!(f, "{}{}", col_to_alpha(self.col), self.row + 1)
    }
}

// -----------------------------------------------------------------------------
// RangeDep — bounded cell range
// -----------------------------------------------------------------------------

/// Describes a rectangular range of cells.
///
/// Bounds are inclusive (`row_start..=row_end`, `col_start..=col_end`).
/// MVP: positional only. Phase 1+ adds `RangeEndpoint::Specific(ListItemId)`
/// and `EntireAxis` for whole-column/whole-row ranges (see spec §3.A).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RangeDep {
    /// Local sheet index (see [`CellRef::sheet`]). Defaults to 0 via
    /// [`RangeDep::new`]; a cross-sheet range carries the resolved index.
    pub sheet: u16,
    pub row_start: u32,
    pub row_end: u32,
    pub col_start: u32,
    pub col_end: u32,
}

impl RangeDep {
    /// Range on the default sheet (index 0). Used by single-sheet call sites.
    pub fn new(
        row_start: u32,
        row_end: u32,
        col_start: u32,
        col_end: u32,
    ) -> Self {
        Self { sheet: 0, row_start, row_end, col_start, col_end }
    }

    /// Range on an explicit sheet index.
    pub fn with_sheet(
        sheet: u16,
        row_start: u32,
        row_end: u32,
        col_start: u32,
        col_end: u32,
    ) -> Self {
        Self { sheet, row_start, row_end, col_start, col_end }
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
        let rows = self.row_end.saturating_sub(self.row_start).saturating_add(1) as usize;
        let cols = self.col_end.saturating_sub(self.col_start).saturating_add(1) as usize;
        rows * cols
    }

    /// Returns `true` when the range spans zero cells.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

// -----------------------------------------------------------------------------
// CellValue — runtime value in the stack VM
// -----------------------------------------------------------------------------

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
    /// Division by zero.
    DivisionByZero,
    /// Operands have incompatible types.
    TypeError,
    /// A formula produced a numeric overflow or invalid math operation.
    NumError,
    /// No applicable value (e.g. VLOOKUP did not find a match).
    NaError,
}

impl fmt::Display for FormulaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FormulaError::CircularReference => write!(f, "#CIRCULAR!"),
            FormulaError::ParseError(msg) => write!(f, "#PARSE!({msg})"),
            FormulaError::DivisionByZero => write!(f, "#DIV/0!"),
            FormulaError::TypeError => write!(f, "#TYPE!"),
            FormulaError::NumError => write!(f, "#NUM!"),
            FormulaError::NaError => write!(f, "#N/A"),
        }
    }
}

// -----------------------------------------------------------------------------
// Op — bytecode instruction
// -----------------------------------------------------------------------------

/// A single instruction in the stack VM bytecode.
///
/// Compiled once per formula, executed on every recomputation (spec §4).
#[derive(Debug, Clone)]
pub enum Op {
    // -- Stack literals --------------------------------------------------------
    PushNum(f64),
    PushStr(String),
    PushBool(bool),
    PushEmpty,

    // -- Cell / range references (resolved at eval time) -----------------------
    /// Push the evaluated value of another cell onto the stack.
    PushCell(CellRef),
    /// Evaluate every cell in a range and push them as `CellValue::Range`.
    EvalRange(RangeDep),

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
    CallFn { name: String, n_args: u8 },
}

// -----------------------------------------------------------------------------
// CellBytecode — compiled formula for one cell
// -----------------------------------------------------------------------------

/// The compiled bytecode for a formula cell, plus its extracted dependencies
/// (used to update the [`DependencyGraph`](super::dag::DependencyGraph)).
#[derive(Debug, Clone)]
pub struct CellBytecode {
    pub ops: Vec<Op>,
    /// Cell references extracted at compile time.
    pub cell_deps: Vec<CellRef>,
    /// Range dependencies extracted at compile time.
    pub range_deps: Vec<RangeDep>,
}

// -----------------------------------------------------------------------------
// Expr — AST produced by the parser
// -----------------------------------------------------------------------------

/// The abstract syntax tree of a parsed formula, fed into the compiler.
///
/// [`Expr::SheetRef`] and [`Expr::Name`] are produced by the (context-free)
/// parser and removed by [`resolve_refs`](super::parser::resolve_refs) before
/// compilation: the resolver rewrites them into [`Expr::CellRef`] /
/// [`Expr::Range`] carrying the resolved sheet index. The compiler therefore
/// never sees them.
#[derive(Debug, Clone)]
pub enum Expr {
    Literal(CellValue),
    CellRef(CellRef),
    Range(RangeDep),
    /// A sheet-qualified reference `Sheet2!A1` / `'My Sheet'!A1:B2`. `inner` is
    /// always a [`Expr::CellRef`] or [`Expr::Range`]; the resolver stamps the
    /// resolved sheet index onto it.
    SheetRef {
        sheet_name: String,
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

/// Resolution services the formula crate needs from the host (`TrameState`):
/// mapping a sheet *display name* to its local index, and a defined name to its
/// target. Implemented by the host; [`SingleSheetCtx`] is the no-op default for
/// single-sheet code and unit tests.
pub trait FormulaCtx {
    /// Resolve a sheet display name (`Sheet2`) to its local index, or `None`
    /// if no such sheet exists (the reference becomes a parse error).
    fn resolve_sheet(&self, name: &str) -> Option<u16>;
    /// Resolve a defined name (`TOTAL`) to its target, or `None` if undefined.
    fn resolve_name(&self, name: &str) -> Option<NameTarget>;
}

/// Default context: a single sheet (index 0), no cross-sheet references, no
/// defined names. Any `Sheet!ref` or bareword name fails to resolve.
pub struct SingleSheetCtx;

impl FormulaCtx for SingleSheetCtx {
    fn resolve_sheet(&self, _name: &str) -> Option<u16> {
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

/// Converts a zero-based column index to spreadsheet letters (A, B, ..., Z,
/// AA, AB, ...).
pub(crate) fn col_to_alpha(mut col: u32) -> String {
    let mut chars = Vec::new();
    loop {
        chars.push((b'A' + (col % 26) as u8) as char);
        if col < 26 {
            break;
        }
        col = col / 26 - 1;
    }
    chars.into_iter().rev().collect()
}

/// Converts spreadsheet column letters to a zero-based index.
pub(crate) fn alpha_to_col(s: &str) -> Result<u32, String> {
    if s.is_empty() || !s.bytes().all(|b| b.is_ascii_uppercase()) {
        return Err(format!("invalid column reference: {s}"));
    }
    let mut col = 0u32;
    for b in s.bytes() {
        col = col
            .checked_mul(26)
            .and_then(|c| c.checked_add((b - b'A') as u32 + 1))
            .ok_or_else(|| format!("column overflow: {s}"))?;
    }
    Ok(col.saturating_sub(1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn col_to_alpha_basics() {
        assert_eq!(col_to_alpha(0), "A");
        assert_eq!(col_to_alpha(25), "Z");
        assert_eq!(col_to_alpha(26), "AA");
        assert_eq!(col_to_alpha(51), "AZ");
        assert_eq!(col_to_alpha(52), "BA");
        assert_eq!(col_to_alpha(701), "ZZ");
    }

    #[test]
    fn alpha_to_col_roundtrip() {
        for col in [0, 1, 25, 26, 51, 52, 100, 701] {
            let alpha = col_to_alpha(col);
            assert_eq!(alpha_to_col(&alpha).unwrap(), col);
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
        let r = RangeDep::new(0, 9, 0, 4);
        assert!(r.contains(CellRef::new(0, 0)));
        assert!(r.contains(CellRef::new(9, 4)));
        assert!(!r.contains(CellRef::new(10, 0)));
        assert!(!r.contains(CellRef::new(0, 5)));
    }

    #[test]
    fn cell_value_display() {
        assert_eq!(CellValue::Number(42.0).to_display(), "42");
        assert_eq!(CellValue::Number(1.23).to_display(), "1.23");
        assert_eq!(CellValue::Text("hello".into()).to_display(), "hello");
        assert_eq!(CellValue::Boolean(true).to_display(), "true");
        assert_eq!(CellValue::Empty.to_display(), "");
        assert_eq!(CellValue::Error(FormulaError::DivisionByZero).to_display(), "#DIV/0!");
        assert_eq!(
            CellValue::Error(FormulaError::ParseError("oops".into())).to_display(),
            "#PARSE!(oops)"
        );
    }

    #[test]
    fn cell_value_as_number() {
        assert_eq!(CellValue::Number(3.0).as_number().unwrap(), 3.0);
        assert_eq!(CellValue::Text("5".into()).as_number().unwrap(), 5.0);
        assert_eq!(CellValue::Boolean(true).as_number().unwrap(), 1.0);
        assert_eq!(CellValue::Boolean(false).as_number().unwrap(), 0.0);
        assert_eq!(CellValue::Empty.as_number().unwrap(), 0.0);
        assert!(matches!(CellValue::Text("abc".into()).as_number(), Err(FormulaError::TypeError)));
    }
}
