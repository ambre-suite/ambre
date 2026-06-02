//! Whiteboard & presentations application for Ambre.
//!
//! Will provide `ToileState` (the per-document model driven by the Logic
//! thread), an infinite canvas with R‑Tree spatial indexing, support for
//! embedded documents (Prose, Trame), and drawing primitives. Embedding is
//! expressed by recursion on the application command/event enums in `ambre-bus`
//! (`ToileCommand` may carry a `Box<AppCommand>` for an embedded child).
