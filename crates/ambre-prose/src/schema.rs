//! Prose's persisted schema version: the number, the register, and the
//! migration registry (`ambre-spec-schema-version.md`,
//! `ambre-spec-prose-schema.md section 7`).
//!
//! `ambre_core::schema` holds the protocol, and every application shares it. The
//! protocol says where the register lives, what an absent one means, what each
//! comparison implies, and how a reader walks a registry.
//!
//! What lives here is what only Prose can know: which version its containers
//! are at today, and what each past layout has to become.
//!
//! **Cost.** [`gate`] imports the payload into a scratch document before it
//! touches the live state, so a full-state read decodes the snapshot twice.
//! That is the price of `SCHEMA-012`: reading the version off the live document
//! would merge a refused document into the one the user has open.

use std::borrow::Cow;

use loro::{ExportMode, LoroDoc, LoroValue};

use ambre_core::schema::{
    SCHEMA_META_MAP, SCHEMA_VERSION_ABSENT, SCHEMA_VERSION_KEY, SchemaError, SchemaMigration,
    SchemaVerdict, migrate_schema, verdict,
};

/// The schema version this build writes into every document it creates
/// (`SCHEMA-004`, `PROSE-028`).
///
/// Raising it is not a decision about this constant. It is a layout change to a
/// persisted record, and that obliges a [`MIGRATIONS`] row which carries an
/// existing document across it (`PROSE-029`, `SCHEMA-009`).
pub const SCHEMA_VERSION: u32 = 3;

/// Write the current version into a document's `meta` register.
///
/// Called from document construction, inside the scaffolding commit, so the
/// number reaches the document before any user edit. It then travels through
/// every persistence path rather than the one that happens to check it.
pub(crate) fn stamp(doc: &LoroDoc) {
    let _ = doc
        .get_map(SCHEMA_META_MAP)
        .insert(SCHEMA_VERSION_KEY, i64::from(SCHEMA_VERSION));
}

/// The schema version a document carries, or the refusal when the register is
/// present and unreadable.
///
/// An **absent** register is [`SCHEMA_VERSION_ABSENT`] (`SCHEMA-005`). A present
/// register holding something other than a `u32` earns a refusal instead.
/// Reading it as 0 would send a document this build cannot understand down the
/// migration path, and open it.
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

/// Prose's registry: one row per version bump, appended never rewritten.
///
/// `0 -> 1` is the register's own arrival. Prose stamps the number from its
/// first document, so a version 0 document is one written by no released build.
/// The step exists because the protocol walks a chain rather than a special
/// case, and it stamps the number and changes nothing else.
///
/// `1 -> 2` is the list marker. A list item gained three attribute keys, and a
/// version 1 document holds no list item at all, so there is nothing to
/// rewrite. The step stamps the number and leaves the containers alone.
///
/// `2 -> 3` is the table grid, the cell styles and the table of contents
/// settings. A version 2 document holds no table and no table of contents,
/// since no command could write either, so this step stamps the number too.
const MIGRATIONS: &[SchemaMigration<LoroDoc>] = &[
    SchemaMigration {
        from: 0,
        to: 1,
        apply: stamp_step,
    },
    SchemaMigration {
        from: 1,
        to: 2,
        apply: stamp_step,
    },
    SchemaMigration {
        from: 2,
        to: 3,
        apply: stamp_step,
    },
];

/// The `0 -> 1` step. A free function rather than a closure because a registry
/// row holds a plain `fn`.
fn stamp_step(doc: &LoroDoc) -> Result<(), String> {
    stamp(doc);
    doc.commit();
    Ok(())
}

/// The full-state read rule, applied before the caller touches a single
/// container (`SCHEMA-006`, `PROSE-097`).
///
/// Returns the bytes the caller should actually import.
///
/// * The payload untouched, when the versions match.
/// * A migrated snapshot, when the document is older (`SCHEMA-007`).
/// * An error, when the document is newer (`SCHEMA-008`) or the engine refuses
///   the bytes.
///
/// Nothing here writes to the live document, so a refused read leaves it
/// exactly as it was (`SCHEMA-012`).
///
/// # Errors
/// * [`SchemaError::PayloadRejected`] when the CRDT engine will not read the
///   bytes.
/// * [`SchemaError::DocumentTooNew`] when a newer client wrote the document.
/// * A migration variant when the chain cannot carry an older document.
pub fn gate(payload: &[u8]) -> Result<Cow<'_, [u8]>, SchemaError> {
    let scratch = LoroDoc::new();
    scratch.import(payload).map_err(|e| {
        tracing::warn!("Prose: the snapshot was rejected by the CRDT engine: {e:?}");
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
                        reason: format!("re-exporting the migrated document failed: {e:?}"),
                    })?;
            Ok(Cow::Owned(bytes))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::ProseCommand;
    use crate::logic::ProseState;

    /// A snapshot carrying `version` in its register. A `None` version writes no
    /// register at all.
    fn snapshot_at(version: Option<u32>) -> Vec<u8> {
        let doc = LoroDoc::new();
        doc.get_map("meta").insert("title", "").expect("seed");
        if let Some(v) = version {
            doc.get_map(SCHEMA_META_MAP)
                .insert(SCHEMA_VERSION_KEY, i64::from(v))
                .expect("stamp");
        }
        doc.commit();
        doc.export(ExportMode::Snapshot).expect("snapshot")
    }

    #[test]
    fn a_new_document_carries_the_current_version() {
        let state = ProseState::new().expect("a document is built");
        assert_eq!(state.schema_version(), SCHEMA_VERSION);
    }

    #[test]
    fn a_document_at_the_current_version_is_handed_back_untouched() {
        let bytes = snapshot_at(Some(SCHEMA_VERSION));
        let gated = gate(&bytes).expect("current");
        assert!(matches!(gated, Cow::Borrowed(_)));
        assert_eq!(&*gated, &bytes[..]);
    }

    #[test]
    fn a_document_written_before_the_register_migrates_to_the_current_version() {
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
    }

    #[test]
    fn a_document_a_newer_client_wrote_is_refused_whole() {
        let bytes = snapshot_at(Some(SCHEMA_VERSION + 1));
        assert!(matches!(
            gate(&bytes),
            Err(SchemaError::DocumentTooNew { .. })
        ));
    }

    /// The refusal is free: the gate reads the payload in a scratch document,
    /// so the live one is never written on the refusing path (`SCHEMA-012`).
    /// The whole snapshot is compared, not a block count, because a write that
    /// left the block set alone would still be a write.
    #[test]
    fn a_document_from_a_newer_client_is_refused_and_changes_nothing() {
        let mut state = ProseState::with_peer(1).expect("a document is built");
        let root = state.root_blocks()[0];
        state.handle_command(ProseCommand::InsertText {
            block_id: root,
            pos: 0,
            text: "kept".to_owned(),
        });
        let before = state.export_snapshot();

        let err = state
            .import_snapshot(&snapshot_at(Some(SCHEMA_VERSION + 1)))
            .expect_err("a newer document is refused");

        assert_eq!(
            err,
            SchemaError::DocumentTooNew {
                document: SCHEMA_VERSION + 1,
                binary: SCHEMA_VERSION,
            }
        );
        assert_eq!(
            state.export_snapshot(),
            before,
            "a refused read must not write the document back"
        );
    }

    #[test]
    fn a_register_this_build_cannot_read_is_refused() {
        let doc = LoroDoc::new();
        doc.get_map(SCHEMA_META_MAP)
            .insert(SCHEMA_VERSION_KEY, "one")
            .expect("a string register");
        doc.commit();
        let bytes = doc.export(ExportMode::Snapshot).expect("snapshot");
        assert!(matches!(
            gate(&bytes),
            Err(SchemaError::VersionUnreadable { .. })
        ));
    }

    /// The engine's refusal comes first, before the register is read. The order
    /// is the rule (`SCHEMA-014`). A payload read after a failed import shows an
    /// absent register, which `SCHEMA-005` values at 0, which the verdict calls
    /// `Older`. The migration path opens, and bytes the engine just refused are
    /// carried forward and opened. That is the back door `SCHEMA-013` closes
    /// for a register that is present and unreadable.
    #[test]
    fn bytes_the_crdt_engine_refuses_never_reach_the_version_check() {
        assert!(matches!(
            gate(&[0xFF, 0x00, 0xFF]),
            Err(SchemaError::PayloadRejected)
        ));
    }
}
