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
    /// Request cell data for a viewport region.
    ///
    /// The UI Thread computes the visible range (plus padding) from its scroll
    /// position and grid dimensions, then sends this command to the Logic
    /// Worker. The Logic Worker responds with [`TrameViewUpdate::ViewportData`]
    /// containing all cells in [row_start..=row_end] x [col_start..=col_end].
    ///
    /// `version` is a monotonically increasing counter. The UI Thread uses it
    /// to discard stale responses: a response whose version is older than the
    /// last accepted one is dropped.
    ScrollViewport {
        version: u64,
        row_start: u32,
        row_end: u32,
        col_start: u32,
        col_end: u32,
    },
}

// -----------------------------------------------------------------------------
// View updates: TrameController -> UI Thread
// -----------------------------------------------------------------------------

/// View updates returned by the [`TrameController`] to the UI Thread.
///
/// Serialized with postcard into `LogicEnvelope::ViewportUpdate::payload`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TrameViewUpdate {
    /// Cell content for a requested viewport region.
    ///
    /// `row_start` and `col_start` are the top-left corner of the batch.
    /// `cells` contains all non-empty cells within the requested range.
    /// The UI Thread merges this batch into its local cache.
    ///
    /// `version` echoes the version from the originating [`TrameCommand::ScrollViewport`]
    /// so the UI Thread can discard out-of-order responses.
    ViewportData {
        version: u64,
        row_start: u32,
        col_start: u32,
        cells: Vec<DisplayCell>,
    },
}

// -----------------------------------------------------------------------------
// Display types
// -----------------------------------------------------------------------------

/// A single cell ready for display.
///
/// This is a flat, data-oriented struct. The UI Thread stores these in a
/// `HashMap<(u32, u32), DisplayCell>` keyed by (row, col).
///
/// Only non-empty cells are transmitted -- missing entries in the cache are
/// treated as blank (white background, no text).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DisplayCell {
    pub row: u32,
    pub col: u32,
    /// Display text (formula result or raw string).
    pub text: String,
}
