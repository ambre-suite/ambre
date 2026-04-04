//! Trame IPC types shared between the Logic Worker and the UI Thread.
//!
//! This crate is intentionally thin: only pure serde types, no CRDT, no GPU.
//! - [`TrameCommand`] / [`TrameViewUpdate`]: postcard-serialized payloads
//!   transported as opaque `Vec<u8>` inside [`ambre_bus`] envelopes.
//!
//! Heavy dependencies live in their consumer:
//! - `loro`, `petgraph`, `FormulaEngine` -> `ambre-logic-worker/src/trame/`
//! - `masonry`, `vello`, widgets          -> `ambre-app/src/trame/`

pub mod events;

pub use events::{TrameCommand, TrameViewUpdate};
