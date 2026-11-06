// ambre-trame/src/logic.rs
// ============================================================================
// Trame (spreadsheet) per-document model -- Logic thread side.
//
// `TrameState` owns everything the Logic thread needs for one open Trame
// document: CRDT, formula engine, formatting. The Logic dispatcher
// (`ambre-logic`) holds it inside `DocState::Trame` and calls the handlers
// below with **typed** `TrameCommand`s, getting back typed `TrameEvent`s.
// No trait object, no postcard, no `Vec<u8>` on this path.
//
// The shared IPC types (TrameCommand, TrameEvent, DisplayCell) live in
// `ambre-bus`; the grid dimensions shared with the UI live in the crate root
// (`crate::TOTAL_ROWS` / `crate::TOTAL_COLS`).
// ============================================================================

use ambre_core::Validated;
use ambre_core::id::{Id, Sheet};
use ambre_core::index::Pos;
use ambre_core::{Bounded, Doc, Px};
use std::collections::{HashMap, HashSet};

use loro::{ExportMode, LoroDoc, LoroValue, UndoManager, VersionVector};

use crate::formula::parser::{parse_formula, resolve_refs};
use crate::formula::text::{DisplayForm, SourceForm, StoredForm};
use crate::formula::types::CellValue;
use crate::formula::types::Expr;
use crate::formula::{AxisId, CellId, EngineSheet, FormulaCtx, FormulaEngine, NameTarget, render};
use crate::message::{
    BorderPreset, CellBorder, CellFormat, CellStyle, CellValueKind, ClipboardCell,
    ClipboardPayload, Decimals, FilterKind, FilterOp, Merge, RowFilter, SelectionKind, SheetName,
    SheetNameKey, SheetTab, SortKey, StylePatch, TabStrip,
};
pub(crate) use crate::message::{HeaderRow, SortOrder};
use crate::model::{Line, SheetIndex, Uid, parse_cell_key};
use crate::{DefinedName, DisplayCell, TrameCommand, TrameEvent};
use ambre_bus::{CellPos, CellRect, Col, MergeSpan, Row};
use ambre_core::{Slant, Weight};
pub use find::{CaseMatch, FindOptions, MatchExtent, SearchScope, SheetScope};

// -----------------------------------------------------------------------------
// FormulaCtx bridge
// -----------------------------------------------------------------------------

/// Resolution context handed to the formula engine: maps a sheet *display name*
/// to its local engine index and a defined name to its target. Borrows the
/// derived caches on [`TrameState`], so it can be built at a call site without
/// borrowing the whole state. The engine, a sibling field, is mutated in the
/// same call.
struct TrameCtx<'a> {
    engine_sheet_by_name: &'a HashMap<SheetNameKey, Pos<EngineSheet>>,
    engine_sheet_by_id: &'a HashMap<Id<Sheet>, Pos<EngineSheet>>,
    named: &'a HashMap<Validated<DefinedName>, NameTarget>,
}

impl FormulaCtx for TrameCtx<'_> {
    /// A qualifier resolves whatever its case, as uniqueness ignores case
    /// (`TRAME-236`).
    fn resolve_sheet(&self, name: &SheetName) -> Option<Pos<EngineSheet>> {
        self.engine_sheet_by_name.get(&name.key()).copied()
    }
    fn resolve_sheet_id(&self, id: &Id<Sheet>) -> Option<Pos<EngineSheet>> {
        self.engine_sheet_by_id.get(id).copied()
    }
    fn resolve_name(&self, name: &Validated<DefinedName>) -> Option<NameTarget> {
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
pub struct TrameOutcome {
    /// Display events destined for the UI Thread.
    pub display: Vec<TrameEvent>,
    /// Binary CRDT delta (Loro `ExportMode::Updates`) to encrypt and broadcast.
    pub update: Option<ambre_core::Payload<ambre_core::DeltaBytes>>,
    /// Ephemeral (presence) payload to encrypt and broadcast.
    pub ephemeral: Option<ambre_core::Payload<ambre_core::EphemeralBytes>>,
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

/// The Logic thread's copy of the whole selection (`trame-selection`). `anchor`
/// is the live rectangle's fixed corner; `extra` holds the rectangles a modifier
/// committed, each normalized; `kind` is the band shape. The
/// active cell lives beside it in [`TrameState::active`]. The two corners
/// normalize into the live rectangle on read.
struct TrackedSelection {
    anchor: CellPos,
    extra: Vec<CellRect>,
    kind: SelectionKind,
}

/// Why a cell write was refused. Nothing of it reached the document.
#[derive(Debug, Clone, PartialEq, Eq)]
enum WriteRefusal {
    /// The cell lies past the materialization cap ([`MATERIALIZE_ROWS`]).
    BeyondCap,
    /// The formula qualifies a sheet no tab carries (`TRAME-211`).
    UnknownSheet(SheetName),
    /// The cell needs a new line, and the session has no fresh uid.
    IdsExhausted,
    /// The document refused the cell's new line.
    Unwritten,
    /// A fill series runs past the largest number at the cell, so the fill
    /// stops there (`TRAME-UI-180`).
    PastLargestNumber,
}

/// Per-document state for a Trame (spreadsheet) document.
///
/// One instance per open document, owned by the Logic thread's `DocState`
/// registry keyed by `document_id`.
pub struct TrameState {
    /// Active cell (selection), kept in sync by the UI via
    /// [`TrameCommand::SetActiveCell`]. Formatting toggles target this cell.
    active: CellPos,
    /// The whole selection reported by the UI via [`TrameCommand::SetActiveCell`]:
    /// the live rectangle's anchor corner, every committed disjoint rectangle,
    /// and the band kind. Kept in sync by the UI alongside [`active`](Self::active);
    /// the parameterless structural intents (`MergeSelection`, `DeleteSelectedRows`,
    /// ...) resolve their geometry against it. Like [`active`](Self::active) it
    /// is ephemeral, in-memory only, never in Loro, never synced (a local view
    /// state, `trame-selection`). Defaults to the single active cell.
    selection: TrackedSelection,
    /// Id of the sheet the UI is currently looking at. Step 1 always holds the
    /// implicit [`default_sheet`]; multi-sheet switching arrives with the tabs.
    active_sheet: Id<Sheet>,
    /// Cells carrying a bold run on the active sheet. Keyed by grid position;
    /// absence means regular weight. Mirrors the persisted `format_{sheet}` map
    /// (the CRDT source of truth) for fast lookup; reseeded on import.
    bold: HashSet<CellPos>,
    /// Cells carrying an italic run on the active sheet.
    italic: HashSet<CellPos>,
    /// Rich manual formatting (underline, alignment, colours, font, number
    /// format, borders, wrap) per cell on the active sheet (`trame-cell-styles`).
    /// Keyed by grid position; absence means the default style. Mirrors the
    /// persisted per-sheet style containers (the CRDT source of truth); reseeded
    /// on import. Bold/italic stay in their own sets above.
    styles: HashMap<CellPos, CellStyle>,
    /// The whole workbook CRDT: multi-sheet schema (`trame-schema`). Cells are
    /// keyed by
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
    /// after any import or structural op (`trame-cell-merges`).
    merges: HashMap<CellPos, CellPos>,
    /// Grid positions covered by a merge but not the master (rendered blank,
    /// swallowed by the master cell). Derived alongside [`Self::merges`].
    covered: HashSet<CellPos>,
    /// Active-sheet grid row positions hidden by the shared filter
    /// (`trame-shared-filter`).
    /// Derived cache of `filter_hidden_{sheet}` (a `{row_uid -> true}` map),
    /// rebuilt by [`Self::rebuild_filter`] after any import or structural op.
    /// Cells in these rows are flagged [`DisplayCell::is_filtered`] but still
    /// participate in calculations.
    hidden_rows: HashSet<Pos<Row>>,
    /// Active-sheet column widths (logical px) per grid column position,
    /// derived cache of `col_widths_{sheet}` (a `{col_uid -> width}` map),
    /// rebuilt by [`Self::reseed_metrics`] after any import or structural op.
    /// Absence means the uniform [`COL_WIDTH`]; the viewport response
    /// fills the gap so the UI gets a complete slice (`trame-resize`).
    col_widths: HashMap<Pos<Col>, Px<Doc>>,
    /// Active-sheet row heights (logical px) per grid row position. Row analogue
    /// of [`col_widths`](Self::col_widths); derived from `row_heights_{sheet}`.
    row_heights: HashMap<Pos<Row>, Px<Doc>>,
    /// Grid positions of covered cells that still hold a non-empty value -- a
    /// concurrent-edit conflict (`trame-cell-merges`). Masked: excluded from the
    /// formula engine
    /// and not displayed, but never destroyed; reappear on unmerge.
    masked_cells: HashSet<CellPos>,
    /// Whether the last engine reseed left out a stored cell: a masked one on
    /// any sheet, or one shown under a second key (`TRAME-279`). A structural
    /// edit then reseeds, since no dependency names such a cell (`TRAME-280`).
    engine_skipped: bool,
    /// The path a structural edit takes. Only the test of `TRAME-280` sets the
    /// reference model, which walks every formula and reseeds.
    #[cfg(test)]
    structural_path: structural::StructuralPath,
    /// CRDT sheet id per local engine index (`sheets_by_idx[i]` has index `i`).
    /// The **active sheet is always index 0**, so single-sheet code paths keep
    /// using [`CellRef::new`] (sheet 0); other sheets take 1.. in sorted-id
    /// order. Rebuilt by [`Self::rebuild_sheet_maps`].
    sheets_by_idx: Vec<Id<Sheet>>,
    /// Sheet *display name* -> local engine index, for resolving `Sheet2!A1`
    /// references in formulas. Keyed by the case-folded name, so `sheet2!A1`
    /// resolves too (`TRAME-236`). Derived alongside [`Self::sheets_by_idx`].
    engine_sheet_by_name: HashMap<SheetNameKey, Pos<EngineSheet>>,
    /// Stable `sheet_id` -> local engine index, for resolving the canonical
    /// `#s0!A1` form persisted in the CRDT `v` text (rename-stable,
    /// `trame-sheets`).
    /// Derived alongside [`Self::sheets_by_idx`].
    engine_sheet_by_id: HashMap<Id<Sheet>, Pos<EngineSheet>>,
    /// Stable `sheet_id` -> current display name, for reconstructing formula-bar
    /// text from the canonical id form. Derived alongside [`Self::sheets_by_idx`].
    sheet_id_to_name: HashMap<Id<Sheet>, SheetName>,
    /// Resolved named ranges: defined name -> target cell/range. Rebuilt from the
    /// `named_ranges` CRDT map by [`Self::rebuild_named`] on every reseed.
    named: HashMap<Validated<DefinedName>, NameTarget>,
    /// The cell writes refused since the last [`finish_cell_writes`], which
    /// reports them to the user (`TRAME-211`, `TRAME-UI-164`). Every write path
    /// ends there, so none of them can refuse in silence.
    ///
    /// [`finish_cell_writes`]: Self::finish_cell_writes
    write_refusals: Vec<WriteRefusal>,
    /// The open edit session's cell, held by identity so a peer's structural
    /// change cannot move its commit (`TRAME-UI-160`). `None` with no session.
    edit_anchor: Option<edit_anchor::EditAnchor>,
    /// Local undo/redo history (`trame-undo`). Subscribes to [`Self::doc`] and
    /// records one
    /// item per commit; each `TrameCommand` handler commits exactly once, so one
    /// command maps to one undo step. Scope is the open-document session: it only
    /// knows this peer's commits, not imported remote ops. `undo`/`redo` mutate
    /// it, hence it is created after the constructor's initial commit so that the
    /// implicit-sheet registration is never undoable.
    undo_manager: UndoManager,
    /// Per-session monotonic counter for minting explicit `sheet_id`s. Combined
    /// with the document `PeerID` so two peers that concurrently add a sheet mint
    /// distinct ids that both survive (`trame-sheets`). Not persisted: ids, once
    /// minted,
    /// live in the CRDT; the counter only needs to be unique within this session.
    /// `None` once the session has minted its last counter: a repeated id
    /// would make two sheets one.
    sheet_counter: Option<ambre_core::IdCounter>,
    /// Per-session monotonic counter for minting row/column [`Uid`]s. Same
    /// peer-scoped convergence argument as `sheet_counter`
    /// (`trame-rows-and-columns`): two peers that
    /// concurrently materialize/insert a line mint distinct uids that both
    /// survive. Not persisted -- once minted, a uid lives in the CRDT as the list
    /// element's value. `None` once the session has minted `u32::MAX`: a
    /// repeated uid would make two lines one.
    uid_counter: Option<ambre_core::IdCounter>,
    /// Set once the first [`TrameCommand::ScrollViewport`] has been served. The
    /// UI thread has no dedicated "fetch sheet list" command. The first viewport
    /// request (sent on load) therefore rides a [`TrameEvent::SheetList`] back,
    /// to hydrate the tab strip. Subsequent scrolls do not repeat it.
    sheet_list_sent: bool,
    /// A register was met that this build cannot decode (`SCHEMA-003`). Set by
    /// the style reseed, drained at every import; a `Cell` because the reseed
    /// reads through a shared borrow of the document.
    ///
    /// A flag rather than a count. The same unreadable register is met again on
    /// every later reseed, so a count would measure how often the grid was
    /// rebuilt.
    ///
    /// Drained at an import, never at a repaint. A register first met by a
    /// display-only read is therefore reported at the next import rather than
    /// at once. Late, and never wrong: the register is still unreadable.
    undecodable: std::cell::Cell<bool>,
    /// This PeerID's counter just past its registry entry, while the entry is
    /// all the session wrote. No delta goes out for it alone; the first edit
    /// carries it (`AUTHOR-009`). Set by [`adopt_session`](Self::adopt_session).
    entry_end: Option<ambre_core::OpCounter>,
}

/// The workbook scaffolding, which every peer writes on open, before any edit.
fn write_scaffold(doc: &LoroDoc) {
    // Register the implicit default sheet's name (idempotent map insert, so
    // a peer that also "creates" s0 converges to the same entry).
    let _ = doc
        .get_map(crate::logic::Root::Sheets.as_str())
        .insert(default_sheet().as_str(), SheetName::default().as_str());
    // The schema version is workbook scaffolding too, and it is written in
    // the same commit so a workbook is never persisted without it
    // (`SCHEMA-004`).
    schema::stamp(doc);
}

/// The identity the scaffolding is written under: a hash of the scaffolding
/// this build writes (`TRAME-231`). Two builds that scaffold alike share it,
/// and two that differ never write two changes under one identifier.
#[expect(
    clippy::expect_used,
    reason = "a fresh document with no op takes a peer id and exports"
)]
fn scaffold_identity() -> ambre_core::PeerId {
    static PEER: std::sync::OnceLock<ambre_core::PeerId> = std::sync::OnceLock::new();
    *PEER.get_or_init(|| {
        let scratch = LoroDoc::new();
        scratch
            .set_peer_id(0)
            .expect("a fresh document takes a peer id");
        write_scaffold(&scratch);
        scratch.commit();
        let bytes = scratch
            .export(ExportMode::all_updates())
            .expect("a fresh document exports");
        ambre_core::schema::scaffold_peer(&bytes)
    })
}

impl TrameState {
    pub fn new() -> Self {
        Self::from_doc(LoroDoc::new())
    }

    /// A workbook whose CRDT identity and commit clock are **pinned**, so the
    /// same command sequence produces the same bytes twice (`FIXTURE-001`).
    ///
    /// Two of the three entropy sources of a Trame document are closed here; the
    /// third, `document_id`, belongs to the caller and is supplied to
    /// [`export_ambre_plaintext`](Self::export_ambre_plaintext).
    ///
    /// * **`PeerID`** -- `LoroDoc::new()` draws one at random, and Trame builds
    ///   sheet ids from it (`format!("{:016x}_{:08x}", doc.peer_id(), counter)`
    ///   in `sheets.rs` / `interop.rs`). An unpinned peer therefore changes the
    ///   document's *content*, not merely its encoding. That is also why a
    ///   semantic digest is no escape hatch for it.
    /// * **Commit timestamps** -- Loro records one per commit unless told not to.
    ///
    /// The scaffolding itself is written under the identity every peer shares
    /// (`TRAME-231`), so it is the same bytes whatever `peer` is.
    ///
    /// **Never for a live document.** Two concurrent writers sharing a `PeerID`
    /// mint conflicting `OpID`s and corrupt the history. This exists for
    /// single-owner, offline, host-side reproduction (a fixture generator, a
    /// deterministic test). The client path is [`new`](Self::new).
    ///
    /// # Panics
    ///
    /// When a fresh `LoroDoc` refuses `peer`, which it does only after an op is
    /// recorded. No op precedes this call.
    #[expect(
        clippy::expect_used,
        reason = "a fresh document with no op takes a peer id"
    )]
    pub fn with_peer(peer: ambre_core::PeerId) -> Self {
        let doc = LoroDoc::new();
        // `set_peer_id` before any op: Trame's sheet ids capture it.
        doc.set_peer_id(peer.get())
            .expect("a fresh LoroDoc accepts a peer id");
        // No wall clock in the ops: a recorded timestamp would differ between two
        // runs of the same generator and defeat the byte diff of `fixtures`.
        doc.set_record_timestamp(false);
        Self::from_doc(doc)
    }

    /// Shared body of [`new`](Self::new) and [`with_peer`](Self::with_peer): the
    /// document scaffolding and every derived cache, over a `LoroDoc` the caller
    /// has already configured.
    #[expect(
        clippy::expect_used,
        reason = "a document outside a transaction takes a peer id"
    )]
    fn from_doc(doc: LoroDoc) -> Self {
        let peer = doc.peer_id();
        // Every peer writes the scaffolding as one shared identity, so every
        // copy is one change. Under the workbook's own identity, a reader that
        // has sent nothing would import each change against the whole history
        // (`TRAME-231`).
        doc.set_peer_id(scaffold_identity().get())
            .expect("a document outside a transaction takes a peer id");
        write_scaffold(&doc);
        doc.commit();
        doc.set_peer_id(peer)
            .expect("a document outside a transaction takes a peer id");
        // Create the undo manager AFTER the initial commit. The implicit-sheet
        // registration is document scaffolding, not a user edit, so it must not
        // sit on the undo stack.
        let mut undo_manager = UndoManager::new(&doc);
        // Keep implicit row/column materialization off the undo stack (see
        // `CommitOrigin::Materialize`); only user-intended edits are undoable.
        undo_manager
            .add_exclude_origin_prefix(ambre_core::crdt::CommitOrigin::Materialize.as_str());
        // The registry entry is bookkeeping, not an edit: an undo never
        // retracts it (`AUTHOR-009`).
        undo_manager.add_exclude_origin_prefix(ambre_core::crdt::CommitOrigin::Peers.as_str());
        let mut state = Self {
            active: CellPos::ORIGIN,
            selection: TrackedSelection {
                anchor: CellPos::ORIGIN,
                extra: Vec::new(),
                kind: SelectionKind::Cells,
            },
            active_sheet: default_sheet(),
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
            engine_skipped: false,
            #[cfg(test)]
            structural_path: structural::StructuralPath::Carry,
            sheets_by_idx: Vec::new(),
            engine_sheet_by_name: HashMap::new(),
            engine_sheet_by_id: HashMap::new(),
            sheet_id_to_name: HashMap::new(),
            named: HashMap::new(),
            write_refusals: Vec::new(),
            edit_anchor: None,
            undo_manager,
            sheet_counter: Some(ambre_core::IdCounter::ZERO),
            uid_counter: Some(ambre_core::IdCounter::ZERO),
            sheet_list_sent: false,
            undecodable: std::cell::Cell::new(false),
            entry_end: None,
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

impl TrameState {
    /// The live rectangle (the anchor corner and the active cell), normalized.
    /// The primary range of the tracked selection.
    fn live_selection_rect(&self) -> CellRect {
        CellRect::spanning(self.active, self.selection.anchor)
    }

    /// Every rectangle a range intent acts on, resolved from the tracked
    /// selection (`TRAME-UI-129`). A `Cells` selection returns the live rectangle
    /// plus the committed disjoint ones; a `Cols`/`Rows` band expands to the
    /// materialized used range across its axis (`TRAME-UI-033`).
    fn selection_rects(&self) -> Vec<CellRect> {
        let live = self.live_selection_rect();
        match self.selection.kind {
            SelectionKind::Cells => {
                let mut rects = self.selection.extra.clone();
                rects.push(live);
                rects
            }
            SelectionKind::Cols => {
                let row_end = self
                    .index
                    .row_len()
                    .as_end()
                    .prev()
                    .unwrap_or(Pos::<Row>::ZERO);
                vec![CellRect::spanning(
                    CellPos::new(Pos::<Row>::ZERO, live.top_left.col),
                    CellPos::new(row_end, live.bottom_right.col),
                )]
            }
            SelectionKind::Rows => {
                let col_end = self
                    .index
                    .col_len()
                    .as_end()
                    .prev()
                    .unwrap_or(Pos::<Col>::ZERO);
                vec![CellRect::spanning(
                    CellPos::new(live.top_left.row, Pos::<Col>::ZERO),
                    CellPos::new(live.bottom_right.row, col_end),
                )]
            }
        }
    }

    /// The bounding box of [`selection_rects`](Self::selection_rects): the single
    /// rectangle that covers every selected rectangle. A band resolves to its
    /// used-range rectangle first.
    fn selection_bounds(&self) -> CellRect {
        let mut rects = self.selection_rects().into_iter();
        let first = rects.next().unwrap_or_else(|| self.live_selection_rect());
        rects.fold(first, |a, b| {
            CellRect::spanning(
                CellPos::new(
                    a.top_left.row.min(b.top_left.row),
                    a.top_left.col.min(b.top_left.col),
                ),
                CellPos::new(
                    a.bottom_right.row.max(b.bottom_right.row),
                    a.bottom_right.col.max(b.bottom_right.col),
                ),
            )
        })
    }

    /// Every distinct cell covered by the tracked selection, deduplicated across
    /// rectangles (`TRAME-UI-031`). A whole-axis band resolves to the used range
    /// first. Backs the style, the aggregate and the copy paths.
    fn selected_cells(&self) -> Vec<CellPos> {
        let mut cells: Vec<CellPos> = self
            .selection_rects()
            .into_iter()
            .flat_map(CellRect::cells)
            .collect();
        cells.sort_unstable();
        cells.dedup();
        cells
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
//   - sync       : CRDT delta flush, remote apply, snapshot import/export;
// - docfile: the native `.ambre` container, Trame side (`docfile`).
// -----------------------------------------------------------------------------
mod cells;
use cells::Unmaterialized;
mod clipboard;
mod commands;
pub mod docfile;
mod edit_anchor;
mod fill;
mod filters;
mod find;
/// The generators, and the per-app half of `shape_of` / `canonical_dump`
/// (`fixtures`). Public, because `ambre-logic`'s registry names the generators;
/// fixture code (I4), so the whole module is behind the non-default `fixtures`
/// feature and reaches no client build.
#[cfg(feature = "fixtures")]
pub mod fixtures;
mod helpers;
mod interop;
mod merges;
mod metrics;
mod number_format;
mod packing;
/// Container naming, the packed style layout, and the workbook's persisted
/// schema version (`ambre-spec-schema-version.md`). Public for the version half
/// alone -- the container names stay `pub(crate)`.
pub mod schema;
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
#[cfg(test)]
#[cfg(not(target_arch = "wasm32"))]
mod convergence_tests;

// Host-only, on `proptest` like the convergence tests: the incremental
// structural path against a full rebuild (`TRAME-280`).
#[cfg(test)]
#[cfg(not(target_arch = "wasm32"))]
mod structural_tests;

/// The schema shape belt (`SCHEMA-018`): the key set, the value types and the
/// `postcard` layouts a build writes, frozen per `SCHEMA_VERSION`. Test-only,
/// so no client build carries it.
#[cfg(test)]
mod schema_shape;

/// The schema reading belt (`SCHEMA-017`): what each stored value means, in
/// absolute units, frozen per `SCHEMA_VERSION`. Test-only, like the shape belt.
#[cfg(test)]
mod schema_reading;
