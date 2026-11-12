//! Prose's mappings in the markup module. The Markdown reader and writer
//! work over the intermediate form. The walk reads the HTML half of a paste
//! into the blocks a copy writes. They run here so the Main instance
//! links none of them (`ambre-spec-budgets.md section 5`). Each reads and
//! answers the types of `ambre_prose_form`, a leaf crate that links no CRDT
//! and no document (`ambre-spec-architecture.md section 2`).

pub mod html_doc;
pub mod html_read;
pub mod html_write;
pub mod md_read;
pub mod md_write;
