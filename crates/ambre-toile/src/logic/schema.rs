//! Toile's persisted schema version: the number, the register, and the
//! migration registry (`ambre-spec-schema-version.md`, `ambre-spec-toile-schema.md`).
//!
//! The protocol -- where the register lives, what an absent one means, what each
//! comparison implies, how a registry is walked -- is `ambre_core::schema` and is
//! shared with every other application. What lives here is what only Toile can
//! know: which version its containers are at today, and what each past layout
//! has to become.
//!
//! **Cost.** [`gate`] imports the payload into a scratch document before the
//! live state is touched, so a full-state read decodes the snapshot twice. That
//! is deliberate and it is the price of `SCHEMA-012`: reading the version off
//! the live document means the refused document is already merged into the board
//! the user is looking at, and the only way back out is to throw their session
//! away. A scratch document is discarded for free.

use std::borrow::Cow;

use loro::{ExportMode, LoroDoc, LoroValue};

use ambre_core::schema::{
    SCHEMA_META_MAP, SCHEMA_VERSION_ABSENT, SCHEMA_VERSION_KEY, SchemaError, SchemaMigration,
    SchemaVerdict, migrate_schema, verdict,
};

/// The schema version this build writes into every board it creates
/// (`SCHEMA-004`).
///
/// Raising it is not a decision about this constant: it is a layout change to a
/// persisted record, which obliges a [`MIGRATIONS`] row that carries an existing
/// document across it (`SCHEMA-009`).
pub const SCHEMA_VERSION: u32 = 1;

/// Write the current version into a board's `meta` register.
///
/// Called from board construction, inside the scaffolding commit, so the number
/// is present before any user edit and travels with the document through every
/// persistence path rather than through the one that happens to check it.
pub(crate) fn stamp(doc: &LoroDoc) {
    let _ = doc
        .get_map(SCHEMA_META_MAP)
        .insert(SCHEMA_VERSION_KEY, i64::from(SCHEMA_VERSION));
}

/// The schema version a board carries, or the refusal when the register is
/// present and unreadable.
///
/// An **absent** register is [`SCHEMA_VERSION_ABSENT`] (`SCHEMA-005`): a
/// board written before the register existed needs no rewriting. A register
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

/// Toile's registry: one row per version bump, appended never rewritten.
///
/// `0 -> 1` is the register's own arrival. Every board written before it existed
/// reads as version 0 and is identical in layout to a version 1 board, so the
/// step stamps the number and changes nothing else -- which is what makes it a
/// migration and not a rewrite.
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
/// when the versions match, a migrated snapshot when the document is older
/// (`SCHEMA-007`), and an error when it is newer (`SCHEMA-008`) or the engine
/// refuses the bytes. Nothing here writes to the live document, so a refused
/// read leaves the board exactly as it was (`SCHEMA-012`).
///
/// # Errors
/// [`SchemaError::PayloadRejected`] when the CRDT engine will not read the
/// bytes, [`SchemaError::DocumentTooNew`] when the board was written by a newer
/// client, and the migration variants when the chain cannot carry an older one.
pub fn gate(payload: &[u8]) -> Result<Cow<'_, [u8]>, SchemaError> {
    let scratch = LoroDoc::new();
    scratch.import(payload).map_err(|e| {
        tracing::warn!("[Toile] snapshot rejected by the CRDT engine: {e:?}");
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
                        reason: format!("re-exporting the migrated board failed: {e:?}"),
                    })?;
            Ok(Cow::Owned(bytes))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logic::state::ToileState;

    /// A snapshot of a board carrying `version` in its register, or none at all
    /// when `version` is `None` -- which is every board written before `ARCH P8`.
    fn snapshot_at(version: Option<u32>) -> Vec<u8> {
        let doc = LoroDoc::new();
        doc.get_map("board").insert("bg_color", 0i64).expect("seed");
        if let Some(v) = version {
            doc.get_map(SCHEMA_META_MAP)
                .insert(SCHEMA_VERSION_KEY, i64::from(v))
                .expect("stamp");
        }
        doc.commit();
        doc.export(ExportMode::Snapshot).expect("snapshot")
    }

    #[test]
    fn a_new_board_carries_the_current_version() {
        let state = ToileState::new();
        assert_eq!(state.schema_version(), SCHEMA_VERSION);
    }

    #[test]
    fn a_board_at_the_current_version_is_handed_back_untouched() {
        let bytes = snapshot_at(Some(SCHEMA_VERSION));
        let gated = gate(&bytes).expect("current");
        assert!(matches!(gated, Cow::Borrowed(_)));
        assert_eq!(&*gated, &bytes[..]);
    }

    #[test]
    fn a_board_written_before_the_register_migrates_to_the_current_version() {
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
        // The migration stamps and does nothing else: the board it carried is
        // still there.
        assert!(
            migrated.get_map("board").get("bg_color").is_some(),
            "the migration must not drop what it migrates"
        );
    }

    #[test]
    fn a_board_from_a_newer_client_is_refused_whole() {
        let bytes = snapshot_at(Some(SCHEMA_VERSION + 1));
        let err = gate(&bytes).expect_err("refused");
        assert_eq!(
            err,
            SchemaError::DocumentTooNew {
                document: SCHEMA_VERSION + 1,
                binary: SCHEMA_VERSION,
            }
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
