//! Word processing application for Ambre.
//!
//! Will provide `ProseState` (the per-document model driven by the Logic
//! thread), a rich‑text CRDT based on Loro, a layout engine with pagination,
//! and import/export filters (DOCX, HTML, Markdown). Its IPC message types
//! (`ProseCommand` / `ProseEvent`) will live in `ambre-bus` alongside the other
//! document types.
