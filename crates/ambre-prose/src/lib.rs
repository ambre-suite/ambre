//! Word processing application for Ambre (`specs/4-Prose/`).
//!
//! The engine is [`logic::ProseState`], alone in the Logic thread. Every Prose
//! feature reaches the document through it, so the state, the two planes and
//! the seam are one document and not three
//! (`features/ambre-spec-prose-engine.md`).
//!
//! * [`doc`] -- the persisted containers, their keys, and the reads the engine
//!   reaches them through (`ambre-spec-prose-schema.md`).
//! * [`mark`] -- the inline mark registry: the expansion policy and the
//!   canonical value.
//! * [`schema`] -- the persisted version register and the migration registry.
//! * [`logic`] -- the engine: the command entry point, the remote and snapshot
//!   paths, presence, and the picture cache.
//! * [`events`] -- the IPC contract, re-exported from `ambre-bus`, plus the
//!   outcome one command answers with.
//! * [`ui`] -- the UI thread's own state and its send path. It holds no
//!   document data (`PROSE-UI-001`).
//!
//! This crate contacts no network and depends on neither `ambre-crypto` nor
//! `ambre-sync` (`PROSE-025`, `PROSE-043`). What must reach another thread
//! leaves as a return value (`PROSE-024`).

#![forbid(unsafe_code)]

pub mod doc;
pub mod events;
pub mod logic;
pub mod mark;
pub mod schema;
pub mod ui;

pub use events::{ProseCommand, ProseEvent, ProseOutcome};
pub use logic::ProseState;
