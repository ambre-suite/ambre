// ambre-trame/src/logic/schema.rs
// CRDT container naming, the packed style bit-layout constants, and the
// workbook's persisted schema version. Pure (free) data and functions with no
// `TrameState` dependency; re-exported by the parent so the impl submodules
// reach it through their `use super::*`.
//
// The version protocol -- where the register lives, what an absent one means,
// what each comparison implies, how a registry is walked -- is
// `ambre_core::schema` and is shared with every other application
// (`ambre-spec-schema-version.md`). What lives here is what only Trame can
// know: which version its containers are at today, and what each past layout
// has to become.

use std::borrow::Cow;

use loro::{ExportMode, LoroDoc, LoroValue};

use ambre_core::schema::{
    SCHEMA_META_MAP, SCHEMA_VERSION_ABSENT, SCHEMA_VERSION_KEY, SchemaError, SchemaMigration,
    SchemaVerdict, migrate_schema, verdict,
};

/// Stable id of the implicit default sheet created by [`TrameState::new`].
///
/// A single constant (not a per-instance generated id) so that two fresh
/// `TrameState`s -- the document creator and a peer/reload restoring its
/// snapshot -- address the *same* per-sheet containers (`cells_s0`, `rows_s0`,
/// ...) and merge cleanly. Explicit additional sheets (a later step) get unique
/// ids and populate `sheet_order`; with one sheet, `sheet_order` is left empty
/// to avoid a duplicate `push` on merge (see `trame-schema`).
pub(crate) const DEFAULT_SHEET: &str = "s0";

/// Loro map holding sheet metadata: `{sheet_id -> name}`. Step 1 stores the name
/// flat; the `{name: LoroText, hidden, tab_color}` sub-map of `trame-schema`
/// lands with the sheet-tabs UI.
pub(crate) const SHEETS_MAP: &str = "sheets";

/// `LoroMovableList` holding the tab-strip order as a sequence of `sheet_id`s
/// (`trame-schema`, `trame-sheets`). The implicit [`DEFAULT_SHEET`] is *not*
/// eagerly pushed here (two fresh peers would each push it and converge to a
/// duplicate); it is treated as
/// the implicit first tab by [`TrameState::sheet_order_ids`]. Explicit sheets
/// are appended on [`TrameCommand::CreateSheet`].
pub(crate) const SHEET_ORDER: &str = "sheet_order";

/// Loro map holding workbook named ranges: `{name -> reference}` where the
/// reference is an A1-syntax string, optionally sheet-qualified (`Sheet2!A1:B2`).
/// LWW: the last writer of a given name wins. Resolved positionally at reseed
/// time into [`TrameState::named`] (`trame-named-ranges`).
pub(crate) const NAMED_RANGES_MAP: &str = "named_ranges";

/// Maximum number of rows/columns a single write may materialize past the
/// current used-range high-water mark. Bounds the pathological "lone value far
/// down the grid" case that would otherwise create ~100k placeholder elements
/// (see `trame-rows-and-columns`). A write beyond it is dropped.
pub(crate) const MATERIALIZE_CAP: usize = 10_000;

/// Commit origin tagging the implicit row/column scaffolding that a cell edit
/// forces into existence (see [`TrameState::materialize`]). The `UndoManager` is
/// told to exclude this prefix, so that scaffolding never lands on the undo
/// stack -- only the user-intended value change does. Explicit structural ops
/// (`InsertRow`, etc.) use the default origin and stay undoable.
pub(crate) const MATERIALIZE_ORIGIN: &str = "mat";

/// Per-sheet container names (`cells_{sheet}`, etc.). Keyed by sheet id so a
/// multi-sheet document keeps each sheet's cells/rows/columns/formatting
/// isolated (`trame-schema`).
pub(crate) fn cells_map_name(sheet: &str) -> String {
    format!("cells_{sheet}")
}
/// Per-sheet cell-style map (`cellstyle_{sheet}`, `trame-cell-styles`): one atomic
/// LWW register per styled cell, keyed by the stable id pair so the style follows
/// its cell across structural ops. The value is the whole visual style of the
/// cell -- bold/italic plus the full [`CellStyle`] (alignments, wrap, colours,
/// borders, font, number format, decimals, size) -- postcard-encoded into the
/// register's binary value. One register per cell (not one per attribute) is the
/// convergence invariant: two peers that concurrently restyle the same cell race
/// exactly once, so the merged style is always one writer's, never a per-attribute
/// blend across containers (`trame-schema`, `TRAME-016`). Absence = the
/// grid default style.
pub(crate) fn cellstyle_map_name(sheet: &str) -> String {
    format!("cellstyle_{sheet}")
}
pub(crate) fn rows_list_name(sheet: &str) -> String {
    format!("rows_{sheet}")
}
pub(crate) fn cols_list_name(sheet: &str) -> String {
    format!("cols_{sheet}")
}
/// Per-sheet merge map (`merges_{sheet}`): a LWW `LoroMap` keyed by the master
/// (top-left) cell's stable id pair, with the bottom-right corner's id pair as
/// value (`trame-cell-merges`). Keyed by stable ids so a merge
/// follows its cells across inserts/deletes.
pub(crate) fn merges_map_name(sheet: &str) -> String {
    format!("merges_{sheet}")
}

/// Per-sheet shared-filter definition map (`filters_{sheet}`): the active filter
/// (column uid + kind), stored flat so it converges via LWW
/// (`trame-shared-filter`). Empty map = no filter.
pub(crate) fn filters_map_name(sheet: &str) -> String {
    format!("filters_{sheet}")
}

/// Per-sheet hidden-row set (`filter_hidden_{sheet}`): `{row_uid -> true}` for
/// every row the active filter hides. Kept separate from `rows_{sheet}` to avoid
/// polluting the order list, and keyed by stable `row_uid` so the hidden set
/// follows the data across a sort/insert (`trame-shared-filter`).
pub(crate) fn filter_hidden_map_name(sheet: &str) -> String {
    format!("filter_hidden_{sheet}")
}

/// Per-sheet column-width map (`col_widths_{sheet}`): `{col_uid -> width_px}` as
/// an f64, keyed by the column's stable `Uid` so the width follows its column
/// across inserts/deletes/reorders (`trame-resize`). Absence = the uniform
/// default [`DEFAULT_COL_WIDTH`]. LWW: concurrent resizes of the same column
/// converge on the last writer, like any cell.
pub(crate) fn col_widths_map_name(sheet: &str) -> String {
    format!("col_widths_{sheet}")
}
/// Per-sheet row-height map (`row_heights_{sheet}`): `{row_uid -> height_px}`.
/// Row analogue of [`col_widths_map_name`]; absence = [`DEFAULT_ROW_HEIGHT`].
pub(crate) fn row_heights_map_name(sheet: &str) -> String {
    format!("row_heights_{sheet}")
}

/// Uniform fallback metrics (physical px) for never-resized axes. Mirror the UI
/// `COL_WIDTH` / `ROW_HEIGHT` defaults (`ui/grid.rs`); the viewport response
/// fills unset columns/rows with these so the UI gets a complete slice.
pub(crate) const DEFAULT_COL_WIDTH: f32 = 80.0;
pub(crate) const DEFAULT_ROW_HEIGHT: f32 = 20.0;
/// Lower clamp on a resize (physical px), matching the UI `MIN_COL_WIDTH` /
/// `MIN_ROW_HEIGHT`. A drag that would shrink past this is pinned here.
pub(crate) const MIN_COL_WIDTH: f32 = 24.0;
pub(crate) const MIN_ROW_HEIGHT: f32 = 12.0;

/// Which axis a structural op targets. Lets [`TrameState::insert_axis`] /
/// [`delete_axis`](TrameState::delete_axis) share one body across rows/columns.
#[derive(Clone, Copy)]
pub(crate) enum Axis {
    Row,
    Col,
}

// -----------------------------------------------------------------------------
// The persisted schema version (`ambre-spec-schema-version.md`)
// -----------------------------------------------------------------------------

/// The schema version this build writes into every workbook it creates
/// (`SCHEMA-004`).
///
/// Raising it is not a decision about this constant: it is a layout change to a
/// persisted record -- the packed [`StyleRecord`](super::StyleRecord) above is
/// the one most likely to move -- which obliges a [`MIGRATIONS`] row that
/// carries an existing workbook across it (`SCHEMA-009`).
pub const SCHEMA_VERSION: u32 = 1;

/// Write the current version into a workbook's `meta` register.
///
/// Called from workbook construction, inside the scaffolding commit, so the
/// number is present before any user edit and travels with the document through
/// every persistence path rather than through the one that happens to check it.
pub(crate) fn stamp(doc: &LoroDoc) {
    let _ = doc
        .get_map(SCHEMA_META_MAP)
        .insert(SCHEMA_VERSION_KEY, i64::from(SCHEMA_VERSION));
}

/// The schema version a workbook carries, or the refusal when the register is
/// present and unreadable.
///
/// An **absent** register is [`SCHEMA_VERSION_ABSENT`] (`SCHEMA-005`): a
/// workbook written before the register existed needs no rewriting. A register
/// that is present but holds something other than a `u32` is refused instead
/// (`SchemaError::VersionUnreadable`) -- reading it as 0 would send a document
/// this build cannot understand down the migration path and open it.
///
/// # Errors
/// [`SchemaError::VersionUnreadable`] for a register holding a non-integer, a
/// container, or an integer outside `u32`.
pub fn read_version(doc: &LoroDoc) -> Result<u32, SchemaError> {
    let Some(value) = doc.get_map(SCHEMA_META_MAP).get(SCHEMA_VERSION_KEY) else {
        return Ok(SCHEMA_VERSION_ABSENT);
    };
    match value.into_value() {
        Ok(LoroValue::I64(n)) => u32::try_from(n).map_err(|_| SchemaError::VersionUnreadable {
            found: format!("the integer {n}"),
        }),
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
/// existed reads as version 0 and is identical in layout to a version 1
/// workbook, so the step stamps the number and changes nothing else -- which is
/// what makes it a migration and not a rewrite.
const MIGRATIONS: &[SchemaMigration<LoroDoc>] = &[SchemaMigration {
    from: 0,
    to: 1,
    apply: stamp_step,
}];

/// The `0 -> 1` step. A free function rather than a closure because a registry
/// row holds a plain `fn`.
fn stamp_step(doc: &LoroDoc) -> Result<(), String> {
    stamp(doc);
    doc.commit();
    Ok(())
}

/// The full-state read rule, applied before a single container is used
/// (`SCHEMA-006`).
///
/// Returns the bytes the caller should actually import: the payload untouched
/// when the versions match, a migrated snapshot when the workbook is older
/// (`SCHEMA-007`), and an error when it is newer (`SCHEMA-008`) or the engine
/// refuses the bytes. Nothing here writes to the live document, so a refused
/// read leaves the workbook exactly as it was (`SCHEMA-012`).
///
/// **Cost.** The payload is imported into a scratch document before the live
/// state is touched, so a full-state read decodes the snapshot twice. That is
/// the price of `SCHEMA-012`: reading the version off the live document means
/// the refused workbook is already merged into the grid the user is looking at,
/// and the only way back out is to throw their session away. A scratch document
/// is discarded for free.
///
/// # Errors
/// [`SchemaError::PayloadRejected`] when the CRDT engine will not read the
/// bytes, [`SchemaError::DocumentTooNew`] when the workbook was written by a
/// newer client, and the migration variants when the chain cannot carry an
/// older one.
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

    /// A snapshot of a workbook carrying `version` in its register, or none at
    /// all when `version` is `None` -- which is every workbook written before
    /// `ARCH P8`.
    fn snapshot_at(version: Option<u32>) -> Vec<u8> {
        let doc = LoroDoc::new();
        doc.get_map(SHEETS_MAP)
            .insert(DEFAULT_SHEET, "Sheet1")
            .expect("seed");
        if let Some(v) = version {
            doc.get_map(SCHEMA_META_MAP)
                .insert(SCHEMA_VERSION_KEY, i64::from(v))
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
            migrated.get_map(SHEETS_MAP).get(DEFAULT_SHEET).is_some(),
            "the migration must not drop what it migrates"
        );
    }

    #[test]
    fn a_workbook_from_a_newer_client_is_refused_whole() {
        let bytes = snapshot_at(Some(SCHEMA_VERSION + 1));
        assert_eq!(
            gate(&bytes).expect_err("refused"),
            SchemaError::DocumentTooNew {
                document: SCHEMA_VERSION + 1,
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
            row: 0,
            col: 0,
            text: "kept".to_owned(),
        });
        let before = st.export_snapshot();

        let err = st
            .import_snapshot(&snapshot_at(Some(SCHEMA_VERSION + 1)))
            .expect_err("refused");

        assert_eq!(
            err,
            SchemaError::DocumentTooNew {
                document: SCHEMA_VERSION + 1,
                binary: SCHEMA_VERSION,
            }
        );
        assert_eq!(
            st.export_snapshot(),
            before,
            "a refused read must not write the document back"
        );
    }

    /// `SCHEMA-003`: a style register this build cannot decode loses its
    /// formatting -- unavoidably -- but the user is told, instead of reading a
    /// workbook that looks intact and is not.
    #[test]
    fn a_register_that_cannot_be_decoded_reaches_the_user() {
        use ambre_bus::{TrameCommand, TrameEvent};

        // A real styled cell first, so the register sits under the cell key the
        // reseed resolves through its own index -- an invented key is skipped
        // before any decode is attempted, which would prove nothing.
        let mut source = TrameState::new();
        source.handle_command(TrameCommand::SetActiveCell {
            row: 0,
            col: 0,
            anchor_row: 0,
            anchor_col: 0,
        });
        source.handle_command(TrameCommand::SetCellStyle {
            patch: crate::StylePatch {
                bold: Some(true),
                ..Default::default()
            },
        });

        // Now overwrite that register with a blob no `StyleRecord` layout can
        // produce: a record written by a client this build does not know.
        let styles = source.doc.get_map(cellstyle_map_name(DEFAULT_SHEET));
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

    /// `SCHEMA-013`: the back door `SCHEMA-005` opens. A register holding
    /// something that is not a version must not be read as no register at all.
    #[test]
    fn a_register_holding_something_other_than_a_version_is_refused() {
        let doc = LoroDoc::new();
        doc.get_map(SCHEMA_META_MAP)
            .insert(SCHEMA_VERSION_KEY, "two")
            .expect("a register a newer client might have written");
        doc.commit();
        let bytes = doc.export(ExportMode::Snapshot).expect("snapshot");
        assert!(matches!(
            gate(&bytes).expect_err("refused"),
            SchemaError::VersionUnreadable { .. }
        ));
    }

    #[test]
    fn bytes_the_engine_will_not_read_are_told_apart_from_a_version_refusal() {
        assert_eq!(
            gate(b"not a loro snapshot").expect_err("rejected"),
            SchemaError::PayloadRejected
        );
    }
}
