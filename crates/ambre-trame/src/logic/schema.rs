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

use ambre_core::{Payload, SnapshotBytes};

use loro::{ExportMode, LoroDoc, LoroValue};
use serde::{Deserialize, Serialize};

use ambre_bus::{CellRect, Col, ColCount, FilterKind, Row, RowCount, SheetId};
use ambre_core::schema::{
    SCHEMA_VERSION_ABSENT, SchemaError, SchemaMigration, SchemaVerdict, SchemaVersion,
    migrate_schema, migration_peer, verdict,
};

use crate::formula::identity::{AxisEdit, LineEdit};
use crate::model::{Line, Uid};

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
    SheetId::new("s0".to_owned())
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
        /// uniform default [`COL_WIDTH`]. LWW: concurrent resizes of
        /// the same column converge on the last writer, like any cell.
        ColWidths = "col_widths",
        /// The row-height map: `{row_uid -> height_px}`. Row analogue of
        /// [`SheetRoot::ColWidths`]; absence = [`ROW_HEIGHT`].
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
    /// The key of a sheet's filter map.
    pub(crate) enum FilterKey: FieldKey {
        /// The single register: the whole filter definition, postcard-encoded
        /// as [`FilterRecord`] (`TRAME-FEAT-025`).
        Active = "active",
    }
}

/// Maximum number of rows a single write may materialize past the current
/// used-range high-water mark. Bounds the pathological "lone value far down
/// the grid" case that would otherwise create ~100k placeholder elements (see
/// `trame-rows-and-columns`). The write path drops anything beyond it.
pub(crate) const MATERIALIZE_ROWS: RowCount = RowCount::new(10_000);

/// The same cap on the columns: one limit holds on both axes.
pub(crate) const MATERIALIZE_COLS: ColCount = ColCount::new(10_000);

// A row count and a column count are two types, so the build compares the
// two caps: a change to one alone fails to compile.
const _: () = assert!(MATERIALIZE_COLS.get() == MATERIALIZE_ROWS.get());

/// The whole shared-filter definition as one CRDT record
/// (`trame-shared-filter`). postcard-encoded into the single `filters_{sheet}`
/// register under [`FILTER_KEY`], mirroring `cellstyle_{sheet}` (`TRAME-016`).
///
/// Column and kind travel together, so two concurrent filters race exactly once.
/// The winner is one writer's whole definition, never a blend of one peer's
/// column and the other's operator (`TRAME-FEAT-025`).
///
/// A uid serialises as its key part string, so the column keeps the bytes
/// it had when it was stored as text (`schema_shape::postcard_layouts`).
#[derive(Serialize, Deserialize)]
pub(crate) struct FilterRecord {
    /// The filtered column's stable id, stored as its 25-char key part.
    pub col: Uid,
    pub kind: FilterKind,
}

pub(crate) use crate::{
    COL_WIDTH, MAX_COL_WIDTH, MAX_ROW_HEIGHT, MIN_COL_WIDTH, MIN_ROW_HEIGHT, ROW_HEIGHT,
};

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
/// persisted record, and that obliges a [`MIGRATIONS`] row which carries an
/// existing workbook across it (`SCHEMA-009`).
pub const SCHEMA_VERSION: SchemaVersion = SchemaVersion::new(1);

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
/// Version 1 is the first layout, and no build before it shipped, so the list
/// is empty. A workbook with no version reads as 0 (`SCHEMA-005`), and the walk
/// finds no step from 0 and refuses it. The first layout change after the
/// release adds its row here.
pub(crate) const MIGRATIONS: &[SchemaMigration<LoroDoc>] = &[];

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
pub fn gate(
    payload: &Payload<SnapshotBytes>,
) -> Result<Cow<'_, Payload<SnapshotBytes>>, SchemaError> {
    gate_through(MIGRATIONS, payload)
}

/// [`gate`] over the registry `steps`. A test hands it a step of its own, so
/// the walk stays proved while [`MIGRATIONS`] holds none.
fn gate_through<'a>(
    steps: &[SchemaMigration<LoroDoc>],
    payload: &'a Payload<SnapshotBytes>,
) -> Result<Cow<'a, Payload<SnapshotBytes>>, SchemaError> {
    let scratch = LoroDoc::new();
    scratch.import(payload.as_bytes()).map_err(|e| {
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
            let peer = migration_peer(heads.iter().map(|id| {
                (
                    ambre_core::PeerId::new(id.peer),
                    ambre_core::OpCounter::new(id.counter),
                )
            }));
            scratch
                .set_peer_id(peer.get())
                .map_err(|e| SchemaError::MigrationFailed {
                    from: found,
                    to: SCHEMA_VERSION,
                    reason: format!("taking the migration identity failed: {e:?}"),
                })?;
            migrate_schema(steps, found, SCHEMA_VERSION, &scratch)?;
            let bytes =
                scratch
                    .export(ExportMode::Snapshot)
                    .map_err(|e| SchemaError::MigrationFailed {
                        from: found,
                        to: SCHEMA_VERSION,
                        reason: format!("re-exporting the migrated workbook failed: {e:?}"),
                    })?;
            Ok(Cow::Owned(Payload::new(bytes)))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TrameState;

    /// A filter record holds its column as a [`Uid`], and keeps the bytes it
    /// had when the column was the key part string. A column that is no key
    /// part refuses the whole record, which the reader reports undecodable.
    #[test]
    fn a_filter_column_is_a_uid_stored_as_its_key_part() {
        let uid = Uid {
            peer: ambre_core::PeerId::new(1),
            counter: ambre_core::IdCounter::new(1),
        };
        let kind = FilterKind::Values(vec!["v".to_owned()]);
        let typed = postcard::to_allocvec(&FilterRecord {
            col: uid,
            kind: kind.clone(),
        })
        .expect("encodes");
        // The record as it was stored when the column was text.
        #[derive(serde::Serialize)]
        struct TextRecord<'a> {
            col: &'a str,
            kind: FilterKind,
        }
        let key_part = uid.to_key_part();
        let text = postcard::to_allocvec(&TextRecord {
            col: &key_part,
            kind: kind.clone(),
        })
        .expect("encodes");
        assert_eq!(typed, text, "the bytes are the key part's");
        let back: FilterRecord = postcard::from_bytes(&typed).expect("decodes");
        assert_eq!(back.col, uid);
        let stray = postcard::to_allocvec(&TextRecord { col: "c", kind }).expect("encodes");
        assert!(postcard::from_bytes::<FilterRecord>(&stray).is_err());
    }

    /// A snapshot of a workbook carrying `version` in its register. A `None`
    /// version writes no register at all, which is every workbook written
    /// before `ARCH P8`.
    fn snapshot_at(version: Option<SchemaVersion>) -> Payload<SnapshotBytes> {
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
        Payload::new(doc.export(ExportMode::Snapshot).expect("snapshot"))
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
        assert_eq!(&*gated, &bytes);
    }

    /// The `0 -> 1` step a test registry holds: it stamps the number and
    /// changes nothing else.
    fn stamp_step(doc: &LoroDoc) -> Result<(), String> {
        stamp(doc);
        doc.commit();
        Ok(())
    }

    #[test]
    fn a_workbook_written_before_the_register_migrates_to_the_current_version() {
        let steps = [SchemaMigration {
            from: SCHEMA_VERSION_ABSENT,
            to: SCHEMA_VERSION,
            apply: stamp_step,
        }];
        let bytes = snapshot_at(None);
        let gated = gate_through(&steps, &bytes).expect("version 0 migrates");
        let migrated = LoroDoc::new();
        migrated
            .import(gated.as_bytes())
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

    /// No build wrote a workbook before version 1, so the registry holds no
    /// step and a workbook with no register finds no path.
    #[test]
    fn a_workbook_written_before_the_register_finds_no_step() {
        assert_eq!(
            gate(&snapshot_at(None)).expect_err("no step leaves version 0"),
            SchemaError::NoMigrationPath {
                from: SCHEMA_VERSION_ABSENT
            }
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
            .import_snapshot(&snapshot_at(Some(SCHEMA_VERSION.next())))
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
                bold: Some(ambre_bus::Weight::Bold),
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
        let bytes = Payload::new(doc.export(ExportMode::Snapshot).expect("snapshot"));
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
            gate(&Payload::new(vec![0xFF, 0x00, 0xFF])).expect_err("refused"),
            SchemaError::PayloadRejected
        ));
    }

    #[test]
    fn bytes_the_engine_will_not_read_are_told_apart_from_a_version_refusal() {
        assert_eq!(
            gate(&Payload::new(b"not a loro snapshot".to_vec())).expect_err("rejected"),
            SchemaError::PayloadRejected
        );
    }
}
