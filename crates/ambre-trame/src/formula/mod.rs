pub mod cache;
pub mod calendar;
pub mod compile;
pub mod dag;
pub mod engine;
pub mod functions;
pub mod identity;
pub mod parser;
pub mod render;
pub mod shift;
pub mod text;
pub mod types;
pub mod vm;

#[cfg(test)]
pub(crate) mod test_index;

// Host-only: built on `proptest` (a host-target dev-dependency; its
// `wait-timeout` transitive dep does not compile on wasm32). These properties
// cover the formula stack's algebraic contracts (parser/printer, compiler,
// VM, engine, functions).
#[cfg(test)]
#[cfg(not(target_arch = "wasm32"))]
mod prop_tests;

pub use compile::SheetIndices;
pub use engine::FormulaEngine;
pub use types::{
    AxisId, CellId, CellRef, CellValue, EngineSheet, FormulaCtx, FormulaError, FunctionName,
    NameTarget, ParseError, RangeDep, RangeEndpoint, RangeId, SheetQualifier, SingleSheetCtx,
};
