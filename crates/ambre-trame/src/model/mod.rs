// ambre-trame/src/model/mod.rs
// ============================================================================
// Trame document model layer (Logic thread side).
//
// This module owns the *stable-identity* layer between two address spaces. The
// formula engine addresses cells positionally, by grid `CellPos`. The CRDT
// keys them by our own move-invariant `Uid` pairs. A row insert is then a single
// Loro op, and a reorder never orphans a cell (`trame-rows-and-columns`).
//
// `axis` owns the line order of every sheet. It alone reads or writes the raw
// lists, and it alone builds a `SheetIndex` (`TRAME-275`).
// ============================================================================

mod axis;
mod line;
mod sheet_index;

#[cfg(test)]
pub(crate) use axis::generation_owner;
pub(crate) use axis::{
    Axes, AxisOrder, HiddenEdit, ListAxis, Removed, SweepLeft, Swept, UidLeft, axis_list,
    parse_register,
};
pub use axis::{
    AxisChange, AxisView, CellRank, Element, ElementId, HidingKey, Kept, LayoutChange,
    LineGeneration, Register, SheetIndex, Slot, SlotEdit, slot_edits,
};
pub use line::Line;
pub use sheet_index::{CellKey, LineRead, Uid, parse_cell_key};
