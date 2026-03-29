//! IPC message types for inter‑worker communication in Ambre.
//!
//! Defines `SyncCommand` (Logic → Sync), `LogicEvent` (Sync → Logic),
//! `UiEnvelope` (UI → Logic), `LogicEnvelope` (Logic → UI), and the
//! `AppController` / `CommandSink` traits.
