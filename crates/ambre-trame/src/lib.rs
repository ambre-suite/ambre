//! Trame IPC types shared between the Logic Worker and the UI Thread.
//!
//! Intentionally thin: only pure serde types, no CRDT, no GPU.
//! - [`TrameCommand`]: postcard-serialized payloads UI Thread -> Logic Worker.
//! - [`TrameViewUpdate`]: postcard-serialized payloads Logic Worker -> UI Thread.
//! - [`DisplayCell`]: flat cell data ready for rendering.
//!
//! Heavy dependencies live in their consumer:
//! - `loro`, `petgraph`, formula engine -> `ambre-logic-worker/src/trame/`
//! - `vello`, `wgpu`, GPU rendering     -> `ambre-app/src/trame/`

pub mod events;

pub use events::{DisplayCell, TrameCommand, TrameViewUpdate};
