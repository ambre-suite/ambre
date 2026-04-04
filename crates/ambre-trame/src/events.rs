// ambre-trame/src/events.rs
// ============================================================================
// IPC command and view-update types for the Trame spreadsheet.
//
// These enums are postcard-serialized as the opaque `payload: Vec<u8>` inside
// `UiEnvelope::AppCommand` (UI Thread -> Logic Worker) and
// `LogicEnvelope::ViewportUpdate` (Logic Worker -> UI Thread).
//
// Both ends import this module directly -- the UI Thread via ambre-app's
// dependency on ambre-trame, the Logic Worker via TrameController.
// ============================================================================

use serde::{Deserialize, Serialize};

// -----------------------------------------------------------------------------
// Commands: UI Thread -> TrameController
// -----------------------------------------------------------------------------

/// Commands sent from the UI Thread to the [`super::TrameController`].
///
/// Serialized with postcard into `UiEnvelope::AppCommand::payload`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TrameCommand {
    /// A key was pressed while a Trame document had focus.
    ///
    /// `seq` is a monotonically increasing counter maintained by the UI Thread.
    /// `t0_ms` is `performance.now()` captured immediately before the
    /// `UiEnvelope` was serialized, used to measure end-to-end IPC latency.
    KeyPress { seq: u64, t0_ms: f64 },
}

// -----------------------------------------------------------------------------
// View updates: TrameController -> UI Thread
// -----------------------------------------------------------------------------

/// View updates returned by [`super::TrameController`] to the UI Thread.
///
/// Serialized with postcard into `LogicEnvelope::ViewportUpdate::payload`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TrameViewUpdate {
    /// Response to [`TrameCommand::KeyPress`].
    ///
    /// `count` is the total number of key presses seen by this controller.
    /// `seq` and `t0_ms` are echoed from the originating command so the UI
    /// Thread can compute the round-trip latency without extra bookkeeping.
    KeyCount { count: u64, seq: u64, t0_ms: f64 },
}
