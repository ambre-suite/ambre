//! Secure messaging application for Ambre.
//!
//! Will provide `MurmureState` (the per-document model driven by the Logic
//! thread), E2EE channels with segment‑based history, read‑cursor tracking, and
//! real‑time presence. Its IPC message types (`MurmureCommand` /
//! `MurmureEvent`) will live in `ambre-bus` alongside the other document types.
