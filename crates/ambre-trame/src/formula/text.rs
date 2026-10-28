//! The three forms a cell's text takes (`TRAME-228`, `trame-sheets`). They
//! live in `ambre-bus`, because a deferred rule payload carries the source
//! form; this module keeps their engine path.

pub use ambre_bus::app::trame::{DisplayFormula, FormulaSource, StoredFormula};
