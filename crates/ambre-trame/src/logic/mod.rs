// ambre-trame/src/logic.rs
// ============================================================================
// Trame (spreadsheet) per-document model -- Logic thread side.
//
// `TrameState` owns everything the Logic thread needs for one open Trame
// document: CRDT, formula engine, formatting. The Logic dispatcher
// (`ambre-logic`) holds it inside `DocState::Trame` and calls the handlers
// below with **typed** `TrameCommand`s, getting back typed `TrameEvent`s --
// no trait object, no postcard, no `Vec<u8>` on this path.
//
// The shared IPC types (TrameCommand, TrameEvent, DisplayCell) live in
// `ambre-bus`; the grid dimensions shared with the UI live in the crate root
// (`crate::TOTAL_ROWS` / `crate::TOTAL_COLS`).
// ============================================================================

use std::collections::{HashMap, HashSet};

use loro::{ExportMode, LoroDoc, LoroValue, UndoManager, VersionVector};

use crate::formula::parser::{parse_formula, resolve_refs};
use crate::formula::types::CellValue;
use crate::formula::types::Expr;
use crate::formula::{AxisId, CellId, FormulaCtx, FormulaEngine, NameTarget, SheetIndices, render};
use crate::model::{SheetIndex, Uid, cell_key, parse_cell_key};
use crate::{DisplayCell, TOTAL_COLS, TOTAL_ROWS, TrameCommand, TrameEvent};
use ambre_bus::{
    BorderPreset, BorderStyle, Borders, CellBorder, CellFormat, CellStyle, ClipboardCell,
    ClipboardPayload, FilterKind, FilterOp, HAlign, StylePatch, VAlign,
};

// -----------------------------------------------------------------------------
// FormulaCtx bridge
// -----------------------------------------------------------------------------

/// Resolution context handed to the formula engine: maps a sheet *display name*
/// to its local engine index and a defined name to its target. Borrows the
/// derived caches on [`TrameState`] so it can be built at a call site without
/// borrowing the whole state (the engine, a sibling field, is mutated in the
/// same call).
struct TrameCtx<'a> {
    sheet_name_to_u16: &'a HashMap<String, u16>,
    sheet_id_to_u16: &'a HashMap<String, u16>,
    named: &'a HashMap<String, NameTarget>,
}

impl FormulaCtx for TrameCtx<'_> {
    fn resolve_sheet(&self, name: &str) -> Option<u16> {
        self.sheet_name_to_u16.get(name).copied()
    }
    fn resolve_sheet_id(&self, id: &str) -> Option<u16> {
        self.sheet_id_to_u16.get(id).copied()
    }
    fn resolve_name(&self, name: &str) -> Option<NameTarget> {
        self.named.get(name).cloned()
    }
}

// -----------------------------------------------------------------------------
// TrameOutcome -- typed editing result
// -----------------------------------------------------------------------------

/// Outcome of processing a [`TrameCommand`]: display events for the UI Thread,
/// plus optional CRDT and ephemeral payloads to forward to the Sync thread.
///
/// The Logic dispatcher sends `display` to the UI, `update` / `ephemeral` to
/// Sync as [`SyncCommand::SendUpdate`] / [`SyncCommand::SendEphemeral`].  This
/// is the *lossless* path required by the architecture: no app content is
/// silently dropped.
/// A resolvable merge rectangle: master key, inclusive top-left `(row, col)`,
/// and inclusive bottom-right `(row, col)` in grid positions.
type MergeRect = (String, (u32, u32), (u32, u32));

pub struct TrameOutcome {
    /// Display events destined for the UI Thread.
    pub display: Vec<TrameEvent>,
    /// Binary CRDT delta (Loro `ExportMode::Updates`) to encrypt and broadcast.
    pub update: Option<Vec<u8>>,
    /// Ephemeral (presence) payload to encrypt and broadcast.
    pub ephemeral: Option<Vec<u8>>,
}

impl TrameOutcome {
    /// Convenience: display events only, no sync payloads.
    pub fn display_only(events: Vec<TrameEvent>) -> Self {
        Self {
            display: events,
            update: None,
            ephemeral: None,
        }
    }
}

// -----------------------------------------------------------------------------
// TrameState
// -----------------------------------------------------------------------------

/// Per-document state for a Trame (spreadsheet) document.
///
/// One instance per open document, owned by the Logic thread's `DocState`
/// registry keyed by `document_id`.
pub struct TrameState {
    /// Active cell (selection), kept in sync by the UI via
    /// [`TrameCommand::SetActiveCell`]. Formatting toggles target this cell.
    active: (u32, u32),
    /// Current selection rectangle, normalized to `(row_start, col_start,
    /// row_end, col_end)` with `start <= end`. Kept in sync by the UI alongside
    /// [`active`](Self::active) via [`TrameCommand::SetActiveCell`]; the
    /// parameterless structural intents (`MergeSelection`, `DeleteSelectedRows`,
    /// ...) resolve their geometry against it. Like [`active`](Self::active) it
    /// is ephemeral, in-memory only, never in Loro, never synced (a local view
    /// state, `-ui.md` sec.16). Defaults to the single active cell.
    selection: (u32, u32, u32, u32),
    /// Id of the sheet the UI is currently looking at. Step 1 always holds the
    /// implicit [`DEFAULT_SHEET`]; multi-sheet switching arrives with the tabs.
    active_sheet: String,
    /// Cells carrying a bold run on the active sheet. Keyed by grid `(row, col)`;
    /// absence means regular weight. Mirrors the persisted `format_{sheet}` map
    /// (the CRDT source of truth) for fast lookup; reseeded on import.
    bold: HashSet<(u32, u32)>,
    /// Cells carrying an italic run on the active sheet.
    italic: HashSet<(u32, u32)>,
    /// Rich manual formatting (underline, alignment, colours, font, number
    /// format, borders, wrap) per cell on the active sheet (`-ui.md` sec.5bis).
    /// Keyed by grid `(row, col)`; absence means the default style. Mirrors the
    /// persisted per-sheet style containers (the CRDT source of truth); reseeded
    /// on import. Bold/italic stay in their own sets above.
    styles: HashMap<(u32, u32), CellStyle>,
    /// The whole workbook CRDT: multi-sheet schema (§2.B). Cells are keyed by
    /// stable `(row_lid, col_lid)` pairs -- see [`crate::model`].
    doc: LoroDoc,
    /// Position <-> stable-id cache for the **active sheet**'s rows/columns.
    /// Rebuilt by [`Self::rebuild_index`] after any import; mutated in place by
    /// the write path as it materializes rows/columns.
    index: SheetIndex,
    /// Last CRDT version vector exported to Sync. Used to produce incremental
    /// deltas via [`ExportMode::Updates`].
    last_exported_vv: VersionVector,
    /// Formula engine: dependency graph, bytecode cache, dirty recomputation.
    /// Addresses cells positionally; [`Self::index`] bridges to the CRDT ids.
    engine: FormulaEngine,
    /// Active-sheet merges, master grid position -> bottom-right grid position.
    /// Derived cache of `merges_{sheet}`, rebuilt by [`Self::rebuild_merges`]
    /// after any import or structural op (§6).
    merges: HashMap<(u32, u32), (u32, u32)>,
    /// Grid positions covered by a merge but not the master (rendered blank,
    /// swallowed by the master cell). Derived alongside [`Self::merges`].
    covered: HashSet<(u32, u32)>,
    /// Active-sheet grid row positions hidden by the shared filter (§1.A).
    /// Derived cache of `filter_hidden_{sheet}` (a `{row_uid -> true}` map),
    /// rebuilt by [`Self::rebuild_filter`] after any import or structural op.
    /// Cells in these rows are flagged [`DisplayCell::is_filtered`] but still
    /// participate in calculations.
    hidden_rows: HashSet<u32>,
    /// Active-sheet column widths (physical px) per grid column position,
    /// derived cache of `col_widths_{sheet}` (a `{col_uid -> width}` map),
    /// rebuilt by [`Self::reseed_metrics`] after any import or structural op.
    /// Absence means the uniform [`DEFAULT_COL_WIDTH`]; the viewport response
    /// fills the gap so the UI gets a complete slice (sec.4bis.C, §9).
    col_widths: HashMap<u32, f32>,
    /// Active-sheet row heights (physical px) per grid row position. Row analogue
    /// of [`col_widths`](Self::col_widths); derived from `row_heights_{sheet}`.
    row_heights: HashMap<u32, f32>,
    /// Stable cell keys of covered cells that still hold a non-empty value -- a
    /// concurrent-edit conflict (§6). Masked: excluded from the formula engine
    /// and not displayed, but never destroyed; reappear on unmerge.
    masked_cells: HashSet<String>,
    /// CRDT sheet id per local engine index (`sheets_by_idx[i]` has index `i`).
    /// The **active sheet is always index 0**, so single-sheet code paths keep
    /// using [`CellRef::new`] (sheet 0); other sheets take 1.. in sorted-id
    /// order. Rebuilt by [`Self::rebuild_sheet_maps`].
    sheets_by_idx: Vec<String>,
    /// Sheet *display name* -> local engine index, for resolving `Sheet2!A1`
    /// references in formulas. Derived alongside [`Self::sheets_by_idx`].
    sheet_name_to_u16: HashMap<String, u16>,
    /// Stable `sheet_id` -> local engine index, for resolving the canonical
    /// `#s0!A1` form persisted in the CRDT `v` text (rename-stable, §2.D).
    /// Derived alongside [`Self::sheets_by_idx`].
    sheet_id_to_u16: HashMap<String, u16>,
    /// Stable `sheet_id` -> current display name, for reconstructing formula-bar
    /// text from the canonical id form. Derived alongside [`Self::sheets_by_idx`].
    sheet_id_to_name: HashMap<String, String>,
    /// Resolved named ranges: defined name -> target cell/range. Rebuilt from the
    /// `named_ranges` CRDT map by [`Self::rebuild_named`] on every reseed.
    named: HashMap<String, NameTarget>,
    /// Local undo/redo history (§8). Subscribes to [`Self::doc`] and records one
    /// item per commit; each `TrameCommand` handler commits exactly once, so one
    /// command maps to one undo step. Scope is the open-document session: it only
    /// knows this peer's commits, not imported remote ops. `undo`/`redo` mutate
    /// it, hence it is created after the constructor's initial commit so that the
    /// implicit-sheet registration is never undoable.
    undo_manager: UndoManager,
    /// Per-session monotonic counter for minting explicit `sheet_id`s. Combined
    /// with the document `PeerID` so two peers that concurrently add a sheet mint
    /// distinct ids that both survive (§2.D). Not persisted: ids, once minted,
    /// live in the CRDT; the counter only needs to be unique within this session.
    sheet_counter: u32,
    /// Per-session monotonic counter for minting row/column [`Uid`]s. Same
    /// peer-scoped convergence argument as `sheet_counter` (§2.A): two peers that
    /// concurrently materialize/insert a line mint distinct uids that both
    /// survive. Not persisted -- once minted, a uid lives in the CRDT as the list
    /// element's value.
    uid_counter: u32,
    /// Set once the first [`TrameCommand::ScrollViewport`] has been served. The
    /// UI thread has no dedicated "fetch sheet list" command, so the first
    /// viewport request (sent on load) rides a [`TrameEvent::SheetList`] back to
    /// hydrate the tab strip. Subsequent scrolls do not repeat it.
    sheet_list_sent: bool,
}

impl TrameState {
    pub fn new() -> Self {
        let doc = LoroDoc::new();
        // Register the implicit default sheet's name (idempotent map insert, so
        // a peer that also "creates" s0 converges to the same entry).
        let _ = doc.get_map(SHEETS_MAP).insert(DEFAULT_SHEET, "Sheet1");
        doc.commit();
        // Create the undo manager AFTER the initial commit: the implicit-sheet
        // registration is document scaffolding, not a user edit, so it must not
        // sit on the undo stack.
        let mut undo_manager = UndoManager::new(&doc);
        // Keep implicit row/column materialization off the undo stack (see
        // [`MATERIALIZE_ORIGIN`]); only user-intended edits are undoable.
        undo_manager.add_exclude_origin_prefix(MATERIALIZE_ORIGIN);
        let mut state = Self {
            active: (0, 0),
            selection: (0, 0, 0, 0),
            active_sheet: DEFAULT_SHEET.to_owned(),
            bold: HashSet::new(),
            italic: HashSet::new(),
            styles: HashMap::new(),
            doc,
            index: SheetIndex::default(),
            last_exported_vv: VersionVector::default(),
            engine: FormulaEngine::new(),
            merges: HashMap::new(),
            covered: HashSet::new(),
            col_widths: HashMap::new(),
            row_heights: HashMap::new(),
            hidden_rows: HashSet::new(),
            masked_cells: HashSet::new(),
            sheets_by_idx: Vec::new(),
            sheet_name_to_u16: HashMap::new(),
            sheet_id_to_u16: HashMap::new(),
            sheet_id_to_name: HashMap::new(),
            named: HashMap::new(),
            undo_manager,
            sheet_counter: 0,
            uid_counter: 0,
            sheet_list_sent: false,
        };
        // Index the implicit sheet so `=Sheet1!A1` resolves before any reseed.
        state.rebuild_sheet_maps();
        state
    }
}

impl Default for TrameState {
    fn default() -> Self {
        Self::new()
    }
}

// -----------------------------------------------------------------------------
// Submodules. `schema` / `helpers` / `packing` hold the free data and functions
// (re-exported below so every `impl TrameState` block reaches them through its
// own `use super::*`); the remaining modules each carry one slice of the
// `TrameState` impl, grouped by concern:
//   - cells      : cell read/write, materialization, the stable-id index;
//   - commands   : command dispatch, undo/redo history;
//   - merges     : merge / unmerge;
//   - structural : row/column insert/delete;
//   - sheets     : sheet lifecycle, named ranges, engine/format reseed;
//   - sort       : single-column range/sheet sort;
//   - filters    : shared filter apply / clear / recompute;
//   - styles     : cell formatting;
//   - sync       : CRDT delta flush, remote apply, snapshot import/export.
// -----------------------------------------------------------------------------
mod cells;
mod clipboard;
mod commands;
mod fill;
mod filters;
mod find;
mod helpers;
mod interop;
mod merges;
mod metrics;
mod packing;
mod schema;
mod sheets;
mod sort;
mod structural;
mod styles;
mod sync;

pub(crate) use helpers::*;
pub(crate) use packing::*;
pub(crate) use schema::*;

#[cfg(test)]
mod tests;

// Host-only: built on `proptest`, which is a host-target dev-dependency (its
// `wait-timeout` transitive dep does not compile on wasm32). The wasm tier runs
// the chrome UI tests instead (`tests/chrome_ui.rs`).
#[cfg(all(test, not(target_arch = "wasm32")))]
mod convergence_tests;
