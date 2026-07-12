//! Deferred feature payloads (Phase 2+): typed value types carried by the
//! deferred `TrameCommand` variants (grid ranges, fill/sort keys, conditional
//! formatting and data-validation predicates). Travel by move, no `serde`.

// ─────────────────────────────────────────────────────────────────────────────
// Deferred feature payloads (Phase 2+)
//
// Typed value types carried by the deferred `TrameCommand` variants above. They
// live in `ambre-bus` (re-exported by `ambre-trame`) and travel by move inside
// `AppCommand::Trame` -- no `serde`. Ranges are expressed in **grid
// coordinates** here because the engine-level `RangeDep` (`ambre-trame`) is not
// visible from the bus; the Logic thread resolves these positions to stable ids
// when each feature's handler lands.
// ─────────────────────────────────────────────────────────────────────────────

/// A rectangular grid range `[row..=row_end] x [col..=col_end]` (0-based,
/// inclusive). Corners may be given in any order; the handler normalizes them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GridRange {
    pub row: u32,
    pub col: u32,
    pub row_end: u32,
    pub col_end: u32,
}

/// How [`TrameCommand::FillRange`] fills `dest \ src` (`-ui.md` sec.5.F).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FillMode {
    /// The Logic thread picks `Series` when a fill line's source values form an
    /// arithmetic numeric progression, `Copy` otherwise. The fill handle default.
    #[default]
    Auto,
    /// Tile the source values verbatim across the fill region (relative formula
    /// references reindex by the tile offset).
    Copy,
    /// Extrapolate a numeric arithmetic progression from the source values; a
    /// line whose source is not numeric falls back to `Copy`.
    Series,
}

/// One key of a multi-column sort ([`TrameCommand::SortRangeMulti`],
/// `ambre-spec-trame-features.md §2`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SortKey {
    /// Column to sort on, as an index within the range (`0` = `col_start`).
    pub col: u32,
    pub ascending: bool,
}

/// A conditional-formatting rule predicate (`ambre-spec-trame-features.md §3`).
/// `String` operands are either a literal value or a cell reference, resolved at
/// evaluation. MVP+ kinds (value/text/formula/presets) land in Phase 2; the
/// gradient / data-bar / top-bottom kinds are Phase 3.
#[derive(Debug, Clone)]
pub enum CondFormatKind {
    GreaterThan(String),
    LessThan(String),
    Between(String, String),
    Equal(String),
    Contains(String),
    StartsWith(String),
    /// Custom formula, e.g. `"=A1>AVERAGE(A:A)"` (relative refs resolved from
    /// the target cell).
    Formula(String),
    IsEmpty,
    IsNotEmpty,
    IsError,
    ColorScale2 {
        min_color: u32,
        max_color: u32,
    },
    ColorScale3 {
        min_color: u32,
        mid_color: u32,
        max_color: u32,
    },
    DataBar {
        color: u32,
    },
    TopN(u32),
    BottomN(u32),
    TopNPercent(f32),
}

/// Style applied when a conditional-formatting rule matches (`§3`). Layered on
/// top of the base style; absent fields leave the base intact. Colours are
/// packed `0xRRGGBB`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CondStyle {
    pub fg: Option<u32>,
    pub bg: u32,
    pub bold: bool,
}

/// A data-validation predicate (`ambre-spec-trame-features.md §5`). `List` /
/// `NumberRange` / `TextLength` are Phase 2; `ListRange` / `DateRange` / `Regex`
/// are Phase 3.
#[derive(Debug, Clone)]
pub enum ValidationKind {
    /// Dropdown over a fixed value set.
    List(Vec<String>),
    /// Dropdown over the values of a range, given as an A1 reference (the
    /// engine-level `RangeDep` is not visible from the bus).
    ListRange(String),
    NumberRange {
        min: f64,
        max: f64,
        inclusive: bool,
    },
    NumberInteger,
    DateRange {
        min: Option<String>,
        max: Option<String>,
    },
    TextLength {
        min: Option<u32>,
        max: Option<u32>,
    },
    Regex(String),
    /// Custom formula that must return `TRUE`.
    Formula(String),
}

/// What happens when a [`ValidationKind`] rejects an input (`§5`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValidationOnInvalid {
    /// Block the local mutation -> [`TrameEvent::ValidationError`](super::TrameEvent::ValidationError).
    Reject,
    /// Allow the mutation but warn -> [`TrameEvent::ValidationWarning`](super::TrameEvent::ValidationWarning).
    Warn,
    /// Visual indicator only.
    Info,
}
