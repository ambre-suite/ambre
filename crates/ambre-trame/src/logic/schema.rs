// ambre-trame/src/logic/schema.rs
// CRDT container naming, the packed style bit-layout constants, and the
// workbook's persisted schema version. Pure (free) data and functions with no
// `TrameState` dependency; re-exported by the parent so the impl submodules
// reach it through their `use super::*`.
//
// `ambre_core::schema` holds the version protocol, and every other application
// shares it (`ambre-spec-schema-version.md`). The protocol says where the
// register lives, what an absent one means, what each comparison implies, and
// how a reader walks a registry. What lives here is what only Trame can know:
// which version its containers are at today, and what each past layout has to
// become.

use std::borrow::Cow;
use std::collections::HashMap;

use loro::{ExportMode, LoroDoc, LoroValue};
use serde::{Deserialize, Serialize};

use ambre_bus::{CellRect, Col, ColCount, FilterKind, Row, RowCount, SheetId};
use ambre_core::schema::{
    SCHEMA_VERSION_ABSENT, SchemaError, SchemaMigration, SchemaVerdict, SchemaVersion,
    migrate_schema, migration_peer, verdict,
};

use super::op_from_str;
use crate::formula::identity::{AxisEdit, LineEdit};
use crate::formula::parser::parse_formula;
use crate::formula::{SheetQualifier, identity, render};
use crate::model::{Line, SheetIndex, Uid};

/// Stable id of the implicit default sheet created by [`TrameState::new`].
///
/// A single constant, never a per-instance generated id. The document creator
/// and a peer or reload restoring its snapshot are two fresh `TrameState`s.
/// Both must address the *same* per-sheet containers (`cells_s0`, `rows_s0`)
/// and merge cleanly. Explicit additional sheets (a later step) get unique ids
/// and populate `sheet_order`. With one sheet, `sheet_order` stays empty to
/// avoid a duplicate `push` on merge (see `trame-schema`).
#[must_use]
pub(crate) fn default_sheet() -> SheetId {
    SheetId("s0".to_owned())
}

ambre_core::loro_fields!(pub(crate));

ambre_core::crdt_names! {
    /// The root containers a workbook opens by one fixed name.
    pub(crate) enum Root: ContainerName {
        /// The schema version register (`ambre-spec-schema-version.md`).
        Meta = ambre_core::crdt::FoundationRoot::Meta.as_str(),
        /// Sheet metadata: `{sheet_id -> name}`. Step 1 stores the name flat;
        /// the `{name: LoroText, hidden, tab_color}` sub-map of `trame-schema`
        /// lands with the sheet-tabs UI.
        Sheets = "sheets",
        /// The tab-strip order as a `LoroMovableList` of `sheet_id`s
        /// (`trame-schema`, `trame-sheets`). This list *never* carries the
        /// implicit [`default_sheet`] eagerly, because two fresh peers would
        /// each push it and converge to a duplicate.
        /// [`TrameState::sheet_order_ids`] treats it as the implicit first tab
        /// instead. [`TrameCommand::CreateSheet`] appends an explicit sheet.
        SheetOrder = "sheet_order",
        /// Workbook named ranges: `{name -> reference}`, where the reference is
        /// an A1-syntax string, optionally sheet-qualified (`Sheet2!A1:B2`).
        /// LWW: the last writer of a given name wins. Resolved positionally at
        /// reseed time into [`TrameState::named`] (`trame-named-ranges`).
        NamedRanges = "named_ranges",
    }
}

ambre_core::crdt_names! {
    /// The per-sheet root containers, one of each per sheet. A sheet's own
    /// container is named by [`SheetContainer::of`], `cells_{sheet}` for
    /// example, so a multi-sheet document keeps each sheet's cells, rows,
    /// columns and formatting isolated (`trame-schema`).
    pub(crate) enum SheetRoot: ContainerFamily {
        /// The cell map, `{"{row_uid}:{col_uid}" -> raw text}`.
        Cells = "cells",
        /// The cell-style map (`trame-cell-styles`): one atomic LWW register
        /// per styled cell. The stable id pair keys it, so the style follows
        /// its cell across structural ops. Absence = the grid default style.
        ///
        /// * The value is the whole visual style of the cell, postcard-encoded
        ///   into the register's binary value. That is bold/italic plus the
        ///   full [`CellStyle`]: alignments, wrap, colours, borders, font,
        ///   number format, decimals, size.
        /// * One register per cell, never one per attribute, is the
        ///   convergence invariant. Two peers that concurrently restyle the
        ///   same cell race exactly once, so the merged style is always one
        ///   writer's. It is never a per-attribute blend across containers
        ///   (`trame-schema`, `TRAME-016`).
        CellStyle = "cellstyle",
        /// The row order, a `LoroMovableList` of `row_uid`s.
        Rows = "rows",
        /// The column order, a `LoroMovableList` of `col_uid`s.
        Cols = "cols",
        /// The merge map: a LWW `LoroMap` keyed by the master (top-left)
        /// cell's stable id pair, with the bottom-right corner's id pair as
        /// value (`trame-cell-merges`). Keyed by stable ids so a merge follows
        /// its cells across inserts and deletes.
        Merges = "merges",
        /// The shared-filter definition map: the active filter held whole as
        /// one register under [`FilterKey::Active`], so it converges as one
        /// LWW unit (`trame-shared-filter`, `TRAME-FEAT-025`). Empty map = no
        /// filter.
        Filters = "filters",
        /// The hidden-row set: `{row_uid -> true}` for every row the active
        /// filter hides. Kept separate from the row list to avoid polluting
        /// the order list. Keyed by stable `row_uid`, so the hidden set follows
        /// the data across a sort or an insert (`trame-shared-filter`).
        FilterHidden = "filter_hidden",
        /// The column-width map: `{col_uid -> width_px}` as an f64, keyed by
        /// the column's stable `Uid` so the width follows its column across
        /// inserts, deletes and reorders (`trame-resize`). Absence = the
        /// uniform default [`DEFAULT_COL_WIDTH`]. LWW: concurrent resizes of
        /// the same column converge on the last writer, like any cell.
        ColWidths = "col_widths",
        /// The row-height map: `{row_uid -> height_px}`. Row analogue of
        /// [`SheetRoot::ColWidths`]; absence = [`DEFAULT_ROW_HEIGHT`].
        RowHeights = "row_heights",
    }
}

/// The root name of one sheet's container of the family `root`, as
/// `cells_{sheet}`. The one place the two halves are joined.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct SheetContainer(String);

impl SheetContainer {
    /// The container of the family `root` that belongs to `sheet`.
    #[must_use]
    pub(crate) fn of(root: SheetRoot, sheet: &SheetId) -> Self {
        Self(format!("{}_{sheet}", root.as_str()))
    }

    /// The name, for the Loro call that opens the container.
    #[must_use]
    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }

    /// The family and the sheet a stored root name joins, or `None` for a
    /// root outside every family. The schema belts read stored roots back
    /// through it.
    #[cfg(test)]
    #[must_use]
    pub(crate) fn parse(name: &str) -> Option<(SheetRoot, &str)> {
        SheetRoot::ALL
            .iter()
            .filter_map(|root| {
                let sheet = name.strip_prefix(root.as_str())?.strip_prefix('_')?;
                (!sheet.is_empty()).then_some((*root, sheet))
            })
            .max_by_key(|(root, _)| root.as_str().len())
    }
}

ambre_core::crdt_names! {
    /// The key of a sheet's filter map since version 2.
    pub(crate) enum FilterKey: FieldKey {
        /// The single register: the whole filter definition, postcard-encoded
        /// as [`FilterRecord`] (`TRAME-FEAT-025`).
        Active = "active",
    }
}

ambre_core::crdt_names! {
    /// The flat keys a version 1 filter map stored. Only the `1 -> 2` step
    /// reads them, to fold them into [`FilterKey::Active`].
    pub(crate) enum LegacyFilterKey: FieldKey {
        /// The filtered column's stable id, as its key part.
        Col = "col",
        /// `"values"` or `"condition"`.
        Kind = "kind",
        /// The child list of the values a values filter keeps.
        Values = "values",
        /// A condition filter's operator.
        Op = "op",
        /// A condition filter's operand.
        Value = "value",
    }
}

/// Maximum number of rows/columns a single write may materialize past the
/// current used-range high-water mark. Bounds the pathological "lone value far
/// down the grid" case that would otherwise create ~100k placeholder elements
/// (see `trame-rows-and-columns`). The write path drops anything beyond it.
pub(crate) const MATERIALIZE_CAP: usize = 10_000;

/// [`MATERIALIZE_CAP`] as a number of rows.
pub(crate) const MATERIALIZE_ROWS: RowCount = RowCount::new(MATERIALIZE_CAP as u32);

/// [`MATERIALIZE_CAP`] as a number of columns.
pub(crate) const MATERIALIZE_COLS: ColCount = ColCount::new(MATERIALIZE_CAP as u32);

/// Commit origin tagging the implicit row/column scaffolding that a cell edit
/// forces into existence (see [`TrameState::materialize`]). Construction tells
/// the `UndoManager` to exclude this prefix, so scaffolding never lands on the
/// undo stack -- only the user-intended value change does. Explicit structural
/// ops such as `InsertRow` use the default origin and stay undoable.
pub(crate) const MATERIALIZE_ORIGIN: &str = "mat";

/// The whole shared-filter definition as one CRDT record
/// (`trame-shared-filter`). postcard-encoded into the single `filters_{sheet}`
/// register under [`FILTER_KEY`], mirroring `cellstyle_{sheet}` (`TRAME-016`).
///
/// Column and kind travel together, so two concurrent filters race exactly once.
/// The winner is one writer's whole definition, never a blend of one peer's
/// column and the other's operator (`TRAME-FEAT-025`).
#[derive(Serialize, Deserialize)]
pub(crate) struct FilterRecord {
    /// The filtered column's stable id, rendered as its 25-char key part.
    pub col: String,
    pub kind: FilterKind,
}

/// Uniform fallback metrics (logical px) for never-resized axes. Mirror the UI
/// `COL_WIDTH` / `ROW_HEIGHT` defaults (`ui/grid.rs`); the viewport response
/// fills unset columns/rows with these so the UI gets a complete slice.
pub(crate) const DEFAULT_COL_WIDTH: f32 = 80.0;
pub(crate) const DEFAULT_ROW_HEIGHT: f32 = 20.0;
/// Lower clamp on a resize (logical px), matching the UI `MIN_COL_WIDTH` /
/// `MIN_ROW_HEIGHT`. The resize pins a drag that would shrink past this.
pub(crate) const MIN_COL_WIDTH: f32 = 24.0;
pub(crate) const MIN_ROW_HEIGHT: f32 = 12.0;

/// Which axis a structural op targets. Lets [`TrameState::insert_axis`] /
/// [`delete_axis`](TrameState::delete_axis) share one body across rows/columns.
#[derive(Clone, Copy)]
pub(crate) enum Axis {
    Row,
    Col,
}

/// A line of the active sheet, by its position type: where its list lives and
/// how a structural edit of it is named.
pub(crate) trait SheetLine: Line {
    /// The axis the position type names.
    const AXIS: Axis;

    /// The structural edit `edit`, tagged with its axis.
    fn edit(edit: LineEdit<Self>) -> AxisEdit;

    /// The first and the last line `rect` covers on this axis.
    fn bounds(rect: CellRect) -> (Self, Self);
}

impl SheetLine for Row {
    const AXIS: Axis = Axis::Row;

    fn edit(edit: LineEdit<Self>) -> AxisEdit {
        AxisEdit::Rows(edit)
    }
    fn bounds(rect: CellRect) -> (Self, Self) {
        (rect.top_left.row, rect.bottom_right.row)
    }
}

impl SheetLine for Col {
    const AXIS: Axis = Axis::Col;

    fn edit(edit: LineEdit<Self>) -> AxisEdit {
        AxisEdit::Cols(edit)
    }
    fn bounds(rect: CellRect) -> (Self, Self) {
        (rect.top_left.col, rect.bottom_right.col)
    }
}

// -----------------------------------------------------------------------------
// The persisted schema version (`ambre-spec-schema-version.md`)
// -----------------------------------------------------------------------------

/// The schema version this build writes into every workbook it creates
/// (`SCHEMA-004`).
///
/// Raising it is not a decision about this constant. It is a layout change to a
/// persisted record. `1 -> 2` folds the flat filter keys into one register
/// ([`FilterRecord`]), which obliges the [`MIGRATIONS`] row carrying an existing
/// workbook across it (`SCHEMA-009`). `2 -> 3` stores each formula reference
/// by the row and column it names (`TRAME-226`).
pub const SCHEMA_VERSION: SchemaVersion = SchemaVersion::new(3);

/// Write the current version into a workbook's `meta` register.
///
/// Called from workbook construction, inside the scaffolding commit. The number
/// therefore reaches the document before any user edit. It then travels through
/// every persistence path, not through the one that happens to check it.
pub(crate) fn stamp(doc: &LoroDoc) {
    let _ = doc
        .get_map(ambre_core::crdt::FoundationRoot::Meta.as_str())
        .insert(
            ambre_core::schema::MetaKey::SchemaVersion.as_str(),
            i64::from(SCHEMA_VERSION.get()),
        );
}

/// The schema version a workbook carries, or the refusal when the register is
/// present and unreadable.
///
/// An **absent** register is [`SCHEMA_VERSION_ABSENT`] (`SCHEMA-005`): a
/// workbook written before the register existed needs no rewriting. A present
/// register holding something other than a `u32` earns a refusal instead
/// (`SchemaError::VersionUnreadable`). Reading it as 0 would send a document
/// this build cannot understand down the migration path, and open it.
///
/// # Errors
/// [`SchemaError::VersionUnreadable`] for a register holding a non-integer, a
/// container, or an integer outside `u32`.
pub fn read_version(doc: &LoroDoc) -> Result<SchemaVersion, SchemaError> {
    let Some(value) = doc
        .get_map(ambre_core::crdt::FoundationRoot::Meta.as_str())
        .get(ambre_core::schema::MetaKey::SchemaVersion.as_str())
    else {
        return Ok(SCHEMA_VERSION_ABSENT);
    };
    match value.into_value() {
        Ok(LoroValue::I64(n)) => {
            u32::try_from(n)
                .map(SchemaVersion::new)
                .map_err(|_| SchemaError::VersionUnreadable {
                    found: format!("the integer {n}"),
                })
        }
        Ok(other) => Err(SchemaError::VersionUnreadable {
            found: format!("{other:?}"),
        }),
        Err(_) => Err(SchemaError::VersionUnreadable {
            found: "a container".to_owned(),
        }),
    }
}

/// Trame's registry: one row per version bump, appended never rewritten.
///
/// `0 -> 1` is the register's own arrival. Every workbook written before it
/// existed reads as version 0, and its layout is identical to a version 1
/// workbook. The step therefore stamps the number and changes nothing else,
/// which is what makes it a migration rather than a rewrite.
///
/// `1 -> 2` folds `filters_{sheet}` from five flat keys into the one whole
/// register ([`FILTER_KEY`] holding a [`FilterRecord`]).
///
/// `2 -> 3` names each formula reference by its row and column
/// ([`stored_reference_step`]).
pub(crate) const MIGRATIONS: &[SchemaMigration<LoroDoc>] = &[
    SchemaMigration {
        from: SchemaVersion::new(0),
        to: SchemaVersion::new(1),
        apply: stamp_step,
    },
    SchemaMigration {
        from: SchemaVersion::new(1),
        to: SchemaVersion::new(2),
        apply: filter_register_step,
    },
    SchemaMigration {
        from: SchemaVersion::new(2),
        to: SchemaVersion::new(3),
        apply: stored_reference_step,
    },
];

/// The `0 -> 1` step. A free function rather than a closure because a registry
/// row holds a plain `fn`.
fn stamp_step(doc: &LoroDoc) -> Result<(), String> {
    stamp(doc);
    doc.commit();
    Ok(())
}

/// The `1 -> 2` step: fold each sheet's flat filter keys into one register.
///
/// Version 1 stored `filters_{sheet}` as five keys plus a child `values` list.
/// Two concurrent filters elected field by field and could converge torn
/// (`TRAME-FEAT-025`). Version 2 stores the whole
/// definition as one `Binary(postcard(FilterRecord))` under [`FILTER_KEY`].
fn filter_register_step(doc: &LoroDoc) -> Result<(), String> {
    if let LoroValue::Map(sheets) = doc.get_map(Root::Sheets.as_str()).get_value() {
        // Sorted: two peers migrating one history write the same operations
        // only if they write them in the same order (`SCHEMA-020`).
        let mut sheets: Vec<SheetId> = sheets.keys().cloned().map(SheetId).collect();
        sheets.sort();
        for sheet in &sheets {
            fold_filter_keys(doc, sheet)?;
        }
    }
    // Stamp the new version inside the same commit as the shape change
    // (`SCHEMA-018`).
    stamp(doc);
    doc.commit();
    Ok(())
}

/// The `2 -> 3` step: name each reference of each stored formula by the row
/// and column at its position today (`TRAME-226`).
///
/// Version 2 stored `=A2`, so an insert above row 2 moved the formula onto
/// another cell. Version 3 stores `={row|col}` (`TRAME-228`). The positions are
/// read in the layout the workbook was saved with. So each formula reads the
/// same cell before and after the step. A position past a sheet's
/// used range stays a position, and a formula that does not parse is left as
/// typed.
fn stored_reference_step(doc: &LoroDoc) -> Result<(), String> {
    let mut sheets: Vec<SheetId> = match doc.get_map(Root::Sheets.as_str()).get_value() {
        LoroValue::Map(sheets) => sheets.keys().cloned().map(SheetId).collect(),
        _ => Vec::new(),
    };
    if !sheets.contains(&default_sheet()) {
        sheets.push(default_sheet());
    }
    // Sheets and cells are walked sorted, so every peer writes in one order
    // (`SCHEMA-020`).
    sheets.sort();
    let indices: HashMap<&str, SheetIndex> = sheets
        .iter()
        .map(|s| {
            let index = SheetIndex::build(
                &doc.get_movable_list(SheetContainer::of(SheetRoot::Rows, s).as_str()),
                &doc.get_movable_list(SheetContainer::of(SheetRoot::Cols, s).as_str()),
            );
            (s.as_str(), index)
        })
        .collect();
    for sheet in &sheets {
        let map = doc.get_map(SheetContainer::of(SheetRoot::Cells, sheet).as_str());
        let LoroValue::Map(cells) = map.get_value() else {
            continue;
        };
        // A stored qualifier is a sheet id; a name one was never written.
        let index_of = |q: Option<&SheetQualifier>| match q {
            None => indices.get(sheet.as_str()),
            Some(SheetQualifier::Id(id)) => indices.get(id.as_str()),
            Some(SheetQualifier::Name(_)) => None,
        };
        let mut cells: Vec<(&String, &LoroValue)> = cells.iter().collect();
        cells.sort_by_key(|(key, _)| *key);
        for (key, value) in cells {
            let LoroValue::String(text) = value else {
                continue;
            };
            let Some(Ok(ast)) = text.strip_prefix('=').map(parse_formula) else {
                continue;
            };
            let before = render::render(&ast);
            let after = render::render(&identity::bind(ast, &index_of));
            if after != before {
                map.insert(key, format!("={after}").as_str())
                    .map_err(|e| format!("rewriting a formula failed: {e:?}"))?;
            }
        }
    }
    stamp(doc);
    doc.commit();
    Ok(())
}

/// Convert one sheet's flat filter shape into the single register. A sheet with
/// no filter (an empty or absent map) is left untouched.
fn fold_filter_keys(doc: &LoroDoc, sheet: &SheetId) -> Result<(), String> {
    let map = doc.get_map(SheetContainer::of(SheetRoot::Filters, sheet).as_str());
    // `get_deep_value` resolves the `values` child list, which the shallow read
    // in `load_active_filter` could not (`TRAME-FEAT-004`).
    let LoroValue::Map(deep) = map.get_deep_value() else {
        return Ok(());
    };
    let fields = FieldValues::<LegacyFilterKey>::of(&deep);
    // The old shape carries a flat `col` key; a version-2 register does not.
    let Some(LoroValue::String(col)) = fields.get(LegacyFilterKey::Col) else {
        return Ok(());
    };
    let col_uid = Uid::parse(col.as_str())
        .ok_or_else(|| format!("an unreadable filter column {}", col.as_str()))?;
    let kind = match fields.get(LegacyFilterKey::Kind) {
        Some(LoroValue::String(kind)) if kind.as_str() == "values" => {
            let values = match fields.get(LegacyFilterKey::Values) {
                Some(LoroValue::List(items)) => items
                    .iter()
                    .filter_map(|v| match v {
                        LoroValue::String(s) => Some(s.to_string()),
                        _ => None,
                    })
                    .collect(),
                _ => Vec::new(),
            };
            FilterKind::Values(values)
        }
        Some(LoroValue::String(kind)) if kind.as_str() == "condition" => {
            let op = match fields.get(LegacyFilterKey::Op) {
                Some(LoroValue::String(s)) => op_from_str(s.as_str())
                    .ok_or_else(|| format!("an unreadable filter operator {}", s.as_str()))?,
                _ => return Err("a condition filter with no operator".to_owned()),
            };
            let value = match fields.get(LegacyFilterKey::Value) {
                Some(LoroValue::String(s)) => s.to_string(),
                _ => String::new(),
            };
            FilterKind::Condition { op, value }
        }
        _ => return Err("a filter with no readable kind".to_owned()),
    };
    let record = FilterRecord {
        col: col_uid.to_key_part(),
        kind,
    };
    let bytes = postcard::to_allocvec(&record)
        .map_err(|e| format!("encoding the filter register failed: {e}"))?;
    // Replace the flat keys with the one register.
    let mut old_keys: Vec<&String> = deep.keys().collect();
    old_keys.sort();
    for key in old_keys {
        let _ = map.delete(key.as_str());
    }
    let _ = Fields::<FilterKey>::of(map).insert(FilterKey::Active, bytes);
    Ok(())
}

/// The full-state read rule, applied before the caller touches a single
/// container (`SCHEMA-006`).
///
/// Returns the bytes the caller should actually import.
///
/// * The payload untouched, when the versions match.
/// * A migrated snapshot, when the workbook is older (`SCHEMA-007`).
/// * An error, when the workbook is newer (`SCHEMA-008`) or the engine refuses
///   the bytes.
///
/// Nothing here writes to the live document, so a refused read leaves the
/// workbook exactly as it was (`SCHEMA-012`).
///
/// **Cost.** [`gate`] imports the payload into a scratch document before it
/// touches the live state, so a full-state read decodes the snapshot twice.
/// `SCHEMA-012` is the reason: reading the version off the live document would
/// merge the refused workbook into the grid the user has open. The only way
/// back out would be to throw their session away, and a scratch document costs
/// nothing to drop.
///
/// # Errors
/// * [`SchemaError::PayloadRejected`] when the CRDT engine will not read the
///   bytes.
/// * [`SchemaError::DocumentTooNew`] when a newer client wrote the workbook.
/// * A migration variant when the chain cannot carry an older workbook.
pub fn gate(payload: &[u8]) -> Result<Cow<'_, [u8]>, SchemaError> {
    let scratch = LoroDoc::new();
    scratch.import(payload).map_err(|e| {
        tracing::warn!("[Trame] snapshot rejected by the CRDT engine: {e:?}");
        SchemaError::PayloadRejected
    })?;
    let found = read_version(&scratch)?;
    match verdict(found, SCHEMA_VERSION) {
        SchemaVerdict::Current => Ok(Cow::Borrowed(payload)),
        SchemaVerdict::Newer => Err(SchemaError::DocumentTooNew {
            document: found,
            binary: SCHEMA_VERSION,
        }),
        SchemaVerdict::Older => {
            // Every peer migrating this history writes as one peer, so their
            // steps write the same operations, not rivals (`SCHEMA-020`). The
            // live workbook keeps its own identity: only the scratch takes it.
            let heads = scratch.oplog_frontiers();
            let peer = migration_peer(heads.iter().map(|id| (id.peer, id.counter)));
            scratch
                .set_peer_id(peer.0)
                .map_err(|e| SchemaError::MigrationFailed {
                    from: found,
                    to: SCHEMA_VERSION,
                    reason: format!("taking the migration identity failed: {e:?}"),
                })?;
            migrate_schema(MIGRATIONS, found, SCHEMA_VERSION, &scratch)?;
            let bytes =
                scratch
                    .export(ExportMode::Snapshot)
                    .map_err(|e| SchemaError::MigrationFailed {
                        from: found,
                        to: SCHEMA_VERSION,
                        reason: format!("re-exporting the migrated workbook failed: {e:?}"),
                    })?;
            Ok(Cow::Owned(bytes))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TrameState;

    /// A snapshot of a workbook carrying `version` in its register. A `None`
    /// version writes no register at all, which is every workbook written
    /// before `ARCH P8`.
    fn snapshot_at(version: Option<SchemaVersion>) -> Vec<u8> {
        let doc = LoroDoc::new();
        doc.get_map(Root::Sheets.as_str())
            .insert(default_sheet().as_str(), "Sheet1")
            .expect("seed");
        if let Some(v) = version {
            doc.get_map(ambre_core::crdt::FoundationRoot::Meta.as_str())
                .insert(
                    ambre_core::schema::MetaKey::SchemaVersion.as_str(),
                    i64::from(v.get()),
                )
                .expect("stamp");
        }
        doc.commit();
        doc.export(ExportMode::Snapshot).expect("snapshot")
    }

    #[test]
    fn a_new_workbook_carries_the_current_version() {
        assert_eq!(TrameState::new().schema_version(), SCHEMA_VERSION);
    }

    #[test]
    fn a_workbook_at_the_current_version_is_handed_back_untouched() {
        let bytes = snapshot_at(Some(SCHEMA_VERSION));
        let gated = gate(&bytes).expect("current");
        assert!(matches!(gated, Cow::Borrowed(_)));
        assert_eq!(&*gated, &bytes[..]);
    }

    #[test]
    fn a_workbook_written_before_the_register_migrates_to_the_current_version() {
        let bytes = snapshot_at(None);
        let gated = gate(&bytes).expect("version 0 migrates");
        let migrated = LoroDoc::new();
        migrated
            .import(&gated)
            .expect("the migrated snapshot imports");
        assert_eq!(
            read_version(&migrated).expect("the migrated register reads"),
            SCHEMA_VERSION
        );
        // The migration stamps and does nothing else: the workbook it carried is
        // still there.
        assert!(
            migrated
                .get_map(Root::Sheets.as_str())
                .get(default_sheet().as_str())
                .is_some(),
            "the migration must not drop what it migrates"
        );
    }

    #[test]
    fn a_workbook_from_a_newer_client_is_refused_whole() {
        let bytes = snapshot_at(Some(SCHEMA_VERSION.next()));
        assert_eq!(
            gate(&bytes).expect_err("refused"),
            SchemaError::DocumentTooNew {
                document: SCHEMA_VERSION.next(),
                binary: SCHEMA_VERSION,
            }
        );
    }

    /// `SCHEMA-008` + `SCHEMA-012`: the refusal is the whole document, and the
    /// workbook the user was already looking at survives it intact.
    #[test]
    fn a_workbook_from_a_newer_client_is_refused_and_changes_nothing() {
        let mut st = TrameState::new();
        st.handle_command(ambre_bus::TrameCommand::SetCellText {
            row: Row::new(0),
            col: Col::new(0),
            text: "kept".to_owned(),
        });
        let before = st.export_snapshot();

        let err = st
            .import_snapshot(&snapshot_at(Some(SCHEMA_VERSION.next())).into())
            .expect_err("refused");

        assert_eq!(
            err,
            SchemaError::DocumentTooNew {
                document: SCHEMA_VERSION.next(),
                binary: SCHEMA_VERSION,
            }
        );
        assert_eq!(
            st.export_snapshot(),
            before,
            "a refused read must not write the document back"
        );
    }

    /// `SCHEMA-003`: a style register this build cannot decode unavoidably
    /// loses its formatting. The notice reaches the user, who would otherwise
    /// read a workbook that looks intact and is not.
    #[test]
    fn a_register_that_cannot_be_decoded_reaches_the_user() {
        use ambre_bus::{TrameCommand, TrameEvent};

        // A real styled cell first, so the register sits under the cell key the
        // reseed resolves through its own index. The reseed skips an invented
        // key before it attempts any decode, which would prove nothing.
        let mut source = TrameState::new();
        source.handle_command(TrameCommand::SetActiveCell {
            row: Row::new(0),
            col: Col::new(0),
            anchor_row: Row::new(0),
            anchor_col: Col::new(0),
            extra: vec![],
            kind: crate::SelectionKind::Cells,
        });
        source.handle_command(TrameCommand::SetCellStyle {
            patch: crate::StylePatch {
                bold: Some(true),
                ..Default::default()
            },
        });

        // Now overwrite that register with a blob no `StyleRecord` layout can
        // produce: a record written by a client this build does not know.
        let styles = source
            .doc
            .get_map(SheetContainer::of(SheetRoot::CellStyle, &default_sheet()).as_str());
        let LoroValue::Map(entries) = styles.get_value() else {
            panic!("the styled cell wrote a register");
        };
        let key = entries
            .keys()
            .next()
            .expect("one styled cell, one register")
            .clone();
        styles
            .insert(&key, LoroValue::Binary(vec![0xFFu8; 8].into()))
            .expect("overwrite the register");
        source.doc.commit();
        let payload = source.export_snapshot();

        let mut st = TrameState::new();
        let events = st
            .import_snapshot(&payload)
            .expect("the workbook still opens");

        assert!(
            events
                .iter()
                .any(|e| matches!(e, TrameEvent::UndecodableRecords)),
            "an unreadable register must be reported, not skipped in silence"
        );
    }

    /// The same workbook, written by this build, says nothing -- otherwise the
    /// notice is noise and the user learns to ignore it.
    #[test]
    fn a_workbook_this_build_wrote_raises_no_notice() {
        let payload = TrameState::new().export_snapshot();
        let mut st = TrameState::new();
        let events = st.import_snapshot(&payload).expect("opens");
        assert!(
            !events
                .iter()
                .any(|e| matches!(e, ambre_bus::TrameEvent::UndecodableRecords))
        );
    }

    /// `SCHEMA-013`: the gate must not treat a register holding something other
    /// than a version as no register at all, which would read as version 0.
    #[test]
    fn a_register_holding_something_other_than_a_version_is_refused() {
        let doc = LoroDoc::new();
        doc.get_map(ambre_core::crdt::FoundationRoot::Meta.as_str())
            .insert(ambre_core::schema::MetaKey::SchemaVersion.as_str(), "two")
            .expect("a register a newer client might have written");
        doc.commit();
        let bytes = doc.export(ExportMode::Snapshot).expect("snapshot");
        assert!(matches!(
            gate(&bytes).expect_err("refused"),
            SchemaError::VersionUnreadable { .. }
        ));
    }

    /// The engine's refusal comes first, before the register is read. The order
    /// is the rule (`SCHEMA-014`). A payload read after a failed import shows an
    /// absent register, which reads as version 0, which the verdict calls
    /// `Older`. The migration path opens, and bytes the engine just refused are
    /// carried forward and opened. A register that is present and unreadable
    /// opens the same back door, and its own test closes it.
    #[test]
    fn bytes_the_crdt_engine_refuses_never_reach_the_version_check() {
        assert!(matches!(
            gate(&[0xFF, 0x00, 0xFF]).expect_err("refused"),
            SchemaError::PayloadRejected
        ));
    }

    #[test]
    fn bytes_the_engine_will_not_read_are_told_apart_from_a_version_refusal() {
        assert_eq!(
            gate(b"not a loro snapshot").expect_err("rejected"),
            SchemaError::PayloadRejected
        );
    }

    /// `1 -> 2` folds a version-1 flat filter into one register. The five keys
    /// collapse into one binary [`FilterRecord`] (`TRAME-FEAT-004`).
    #[test]
    fn a_flat_filter_migrates_to_one_register() {
        use ambre_bus::FilterOp;

        // A version-1 workbook holding the old flat condition shape.
        let doc = LoroDoc::new();
        doc.get_map(ambre_core::crdt::FoundationRoot::Meta.as_str())
            .insert(
                ambre_core::schema::MetaKey::SchemaVersion.as_str(),
                i64::from(1),
            )
            .expect("stamp");
        doc.get_map(Root::Sheets.as_str())
            .insert(default_sheet().as_str(), "Sheet1")
            .expect("sheet");
        let filters =
            doc.get_map(SheetContainer::of(SheetRoot::Filters, &default_sheet()).as_str());
        let filters = Fields::<LegacyFilterKey>::of(filters);
        filters
            .insert(LegacyFilterKey::Col, "0000000000000001_00000000")
            .unwrap();
        filters.insert(LegacyFilterKey::Kind, "condition").unwrap();
        filters.insert(LegacyFilterKey::Op, "gt").unwrap();
        filters.insert(LegacyFilterKey::Value, "1").unwrap();
        doc.commit();
        let payload = doc.export(ExportMode::Snapshot).expect("snapshot");

        let gated = gate(&payload).expect("version 1 migrates");
        let migrated = LoroDoc::new();
        migrated.import(&gated).expect("migrated snapshot imports");
        assert_eq!(read_version(&migrated).expect("reads"), SCHEMA_VERSION);

        let LoroValue::Map(deep) = migrated
            .get_map(SheetContainer::of(SheetRoot::Filters, &default_sheet()).as_str())
            .get_deep_value()
        else {
            panic!("the filters map survives");
        };
        let old = FieldValues::<LegacyFilterKey>::of(&deep);
        assert!(
            old.get(LegacyFilterKey::Col).is_none(),
            "the flat keys are gone"
        );
        let fields = FieldValues::<FilterKey>::of(&deep);
        let LoroValue::Binary(bytes) = fields.get(FilterKey::Active).expect("one register") else {
            panic!("the register is binary");
        };
        let record: FilterRecord = postcard::from_bytes(bytes).expect("decodes");
        assert_eq!(record.col, "0000000000000001_00000000");
        assert!(
            matches!(record.kind, FilterKind::Condition { op: FilterOp::Gt, value } if value == "1"),
            "the whole condition survived the fold"
        );
    }
}
