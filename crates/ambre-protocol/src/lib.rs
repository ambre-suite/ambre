//! Wire protocol types for Ambre client‑server communication.
//!
//! Defines all network messages: updates, snapshots, ephemerals, WebSocket frames,
//! and signaling messages for WebRTC meetings.
//!
//! This crate is the single source of truth for wire types; neither `ambre-sync`
//! nor `ambre-server` redefine these structures.

