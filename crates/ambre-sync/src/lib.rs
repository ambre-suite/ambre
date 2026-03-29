//! Sync Worker — handles network, crypto, compression, and real‑time collaboration.
//!
//! This crate runs in a dedicated Web Worker (WASM) and is the only consumer of
//! `ambre‑crypto` and `ambre‑protocol` on the client side.
