//! Word processing application for Ambre (`specs/4-Prose/`).
//!
//! The engine is [`logic::ProseState`], alone in the Logic thread. Every Prose
//! feature reaches the document through it, so the state, the two planes and
//! the seam are one document and not three
//! (`features/ambre-spec-prose-engine.md`).
//!
//! * [`doc`] -- the persisted containers, their keys, and the reads the engine
//!   reaches them through (`ambre-spec-prose-schema.md`).
//! * [`numbering`] -- the number in front of a heading, read from the document
//!   order and stored nowhere (`features/ambre-spec-prose-headings.md`).
//! * [`toc`] -- the entries of a table of contents, read off the headings and
//!   a layout (`features/ambre-spec-prose-table-of-contents.md`).
//! * [`band`] -- the text a header or a footer shows on one page, its fields
//!   resolved (`features/ambre-spec-prose-headers-footers.md`).
//! * [`note`] -- the footnotes as the view lists them: each one numbered from
//!   the order of its calls, or orphaned (`features/ambre-spec-prose-footnotes.md`).
//! * [`xref`] -- a cross reference's stored record and the value it shows
//!   (`features/ambre-spec-prose-cross-references.md`).
//! * [`suggest`] -- the pending suggestions, read off their marks and entries
//!   (`features/ambre-spec-prose-track-changes.md`).
//! * [`count`] -- the word count's four text tallies
//!   (`features/ambre-spec-prose-word-count.md`).
//! * [`mark`] -- the inline mark registry: the expansion policy and the
//!   canonical value.
//! * [`schema`] -- the persisted version register and the migration registry.
//! * [`layout`] -- pagination: the page box, family resolution, and the passes
//!   that measure and place (`features/ambre-spec-prose-pagination.md`).
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

pub mod band;
pub mod comment;
pub mod count;
pub mod doc;
pub mod events;
pub mod layout;
pub mod logic;
pub mod mark;
pub mod note;
pub mod numbering;
pub mod schema;
pub mod style;
pub mod suggest;
pub mod toc;
pub mod ui;
pub mod view;
pub mod xref;

pub use events::{ProseCommand, ProseEvent, ProseOutcome};
pub use logic::ProseState;
