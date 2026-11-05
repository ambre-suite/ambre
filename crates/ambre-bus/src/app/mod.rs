//! The value types more than one application, or the Sync worker, shares.
//!
//! Each application declares its own command, event and value types in its
//! crate (`BUS-032`). What stays here is shared by nature:
//! - `grid`, the cell coordinates Trame and Prose both carry;
//! - `find`, the query every editor's search takes;
//! - `murmure`, Murmure's channel types of the service plane, which the Sync
//!   worker reads, with the `Emoji` Toile shares.

pub mod find;
pub mod grid;
pub mod murmure;
