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

use ambre_core::Version;
use std::borrow::Cow;

use loro::{ExportMode, LoroDoc, LoroValue};

use ambre_core::crdt::CrdtName as _;
use ambre_core::schema::{
    SCHEMA_VERSION_ABSENT, Schema, SchemaError, SchemaMigration, SchemaVerdict, migrate_schema,
    migration_peer, verdict,
};
use ambre_core::{Payload, SnapshotBytes};

/// The schema version this build writes into every document it creates
/// (`SCHEMA-004`, `PROSE-028`).
///
/// Raising it is not a decision about this constant. It is a layout change to a
/// persisted record, and that obliges a [`MIGRATIONS`] row which carries an
/// existing document across it (`SCHEMA-018`, `SCHEMA-009`).
pub const SCHEMA_VERSION: Version<Schema> = Version::new(2);

/// Write the current version into a document's `meta` register.
///
/// Called from document construction, inside the scaffolding commit, so the
/// number reaches the document before any user edit. It then travels through
/// every persistence path rather than the one that happens to check it.
pub(crate) fn stamp(doc: &LoroDoc) {
    let _ = doc
        .get_map(ambre_core::crdt::FoundationRoot::Meta.as_str())
        .insert(
            ambre_core::schema::MetaKey::SchemaVersion.as_str(),
            i64::from(SCHEMA_VERSION.get()),
        );
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
pub fn read_version(doc: &LoroDoc) -> Result<Version<Schema>, SchemaError> {
    let Some(value) = doc
        .get_map(ambre_core::crdt::FoundationRoot::Meta.as_str())
        .get(ambre_core::schema::MetaKey::SchemaVersion.as_str())
    else {
        return Ok(SCHEMA_VERSION_ABSENT);
    };
    match value.into_value() {
        Ok(LoroValue::I64(n)) => u32::try_from(n).map(Version::<Schema>::new).map_err(|_| {
            SchemaError::VersionUnreadable {
                found: format!("the integer {n}"),
            }
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
/// Version 1 is the first layout. A document with no version reads as 0
/// (`SCHEMA-005`), and the walk finds no step from 0 and refuses it.
pub(crate) const MIGRATIONS: &[SchemaMigration<LoroDoc>] = &[SchemaMigration {
    from: Version::new(1),
    to: Version::new(2),
    apply: heading_font_v2,
}];

/// Version 1 to 2: `meta/defaults` may carry the heading font
/// (`PROSE-UI-266`).
///
/// The step rewrites nothing but the register. A version 1 document names no
/// heading font, and an absent key sets each heading in the body font, the
/// version 1 rule. So no block moves (`PROSE-149`).
fn heading_font_v2(doc: &LoroDoc) -> Result<(), String> {
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
            // Every peer migrating this history writes as one peer, so their
            // steps write the same operations, not rivals (`SCHEMA-020`). The
            // live document keeps its own identity: only the scratch takes it.
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
                        reason: format!("re-exporting the migrated document failed: {e:?}"),
                    })?;
            Ok(Cow::Owned(Payload::new(bytes)))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::ProseCommand;
    use crate::logic::ProseState;
    use crate::message::DocChar;
    use ambre_core::index::Pos;

    /// A snapshot carrying `version` in its register. A `None` version writes no
    /// register at all.
    fn snapshot_at(version: Option<Version<Schema>>) -> Payload<SnapshotBytes> {
        let doc = LoroDoc::new();
        crate::doc::meta(&doc)
            .insert(crate::doc::MetaKey::Title, "")
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
    fn a_new_document_carries_the_current_version() {
        let state = ProseState::new().expect("a document is built");
        assert_eq!(state.schema_version(), SCHEMA_VERSION);
    }

    #[test]
    fn a_document_at_the_current_version_is_handed_back_untouched() {
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
    fn a_document_written_before_the_register_migrates_to_the_current_version() {
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
    }

    /// No build wrote a document before version 1, so the registry holds no
    /// step and a document with no register finds no path.
    #[test]
    fn a_document_written_before_the_register_finds_no_step() {
        assert_eq!(
            gate(&snapshot_at(None)).expect_err("no step leaves version 0"),
            SchemaError::NoMigrationPath {
                from: SCHEMA_VERSION_ABSENT
            }
        );
    }

    #[test]
    fn a_document_a_newer_client_wrote_is_refused_whole() {
        let bytes = snapshot_at(Some(SCHEMA_VERSION.next()));
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
        let mut state =
            ProseState::with_peer(ambre_core::PeerId::new(1)).expect("a document is built");
        let root = state.root_blocks()[0];
        state.handle_command(ProseCommand::InsertText {
            block_id: root,
            pos: Pos::<DocChar>::ZERO,
            text: "kept".to_owned(),
        });
        let before = state.export_snapshot();

        let err = state
            .import_snapshot(&snapshot_at(Some(SCHEMA_VERSION.next())))
            .expect_err("a newer document is refused");

        assert_eq!(
            err,
            SchemaError::DocumentTooNew {
                document: SCHEMA_VERSION.next(),
                binary: SCHEMA_VERSION,
            }
        );
        assert_eq!(
            state.export_snapshot(),
            before,
            "a refused read must not write the document back"
        );
    }

    /// The styled document as a version 1 build wrote it, frozen before the
    /// heading font raised the version (`PROSE-149`).
    const VERSION_1: &[u8] =
        include_bytes!("../../../fixtures/frozen/prose/styled_document@v1-s1.ambre");

    /// The same generator, run by this build.
    const CURRENT: &[u8] = include_bytes!("../../../fixtures/current/prose/styled_document.ambre");

    /// A document opened from a container file, under a pinned peer.
    fn opened(file: &[u8], peer: u64) -> ProseState {
        let mut state =
            ProseState::with_peer(ambre_core::PeerId::new(peer)).expect("a document is built");
        state
            .import_ambre_plaintext(&Payload::new(file.to_vec()))
            .expect("the file opens");
        state
    }

    /// The runs of every root block, in document order.
    fn every_run(state: &ProseState) -> Vec<Vec<(String, crate::style::ResolvedStyle)>> {
        state
            .root_blocks()
            .iter()
            .map(|id| state.resolved_runs(id))
            .collect()
    }

    /// A version 1 document opens under version 2, and every block resolves
    /// as the version 2 build resolves the same commands: no heading moves
    /// to another font (`PROSE-149`).
    #[test]
    fn a_version_1_document_opens_with_every_block_resolved_as_before() {
        let old = opened(VERSION_1, 1);
        assert_eq!(old.schema_version(), SCHEMA_VERSION);
        assert_eq!(old.defaults().heading_font_family, None);
        let runs = every_run(&old);
        assert_eq!(runs, every_run(&opened(CURRENT, 1)));
        let headings: Vec<_> = old
            .root_blocks()
            .into_iter()
            .filter(|id| old.block_type_of(id) == Some(crate::events::BlockType::Heading))
            .collect();
        assert!(!headings.is_empty(), "the fixture holds a heading");
        for id in headings {
            for (_, style) in old.resolved_runs(&id) {
                assert_eq!(style.font_family, Some(old.defaults().font_family));
            }
        }
    }

    /// Two peers open one version 1 document, each migrates it, each types,
    /// and both edits survive with equal version vectors: the step wrote one
    /// set of operations, not two rivals (`SCHEMA-020`).
    #[test]
    fn two_peers_migrating_a_version_1_document_keep_both_edits() {
        let (mut alice, mut bob) = (opened(VERSION_1, 1), opened(VERSION_1, 2));
        let first = alice.root_blocks()[0];
        assert_eq!(bob.root_blocks()[0], first);
        let write = |state: &mut ProseState, text: &str| {
            state
                .handle_command(ProseCommand::InsertText {
                    block_id: first,
                    pos: Pos::<DocChar>::ZERO,
                    text: text.to_owned(),
                })
                .update
                .expect("a delta")
        };
        let from_alice = write(&mut alice, "A");
        let from_bob = write(&mut bob, "B");
        bob.apply_remote(&from_alice);
        alice.apply_remote(&from_bob);
        assert_eq!(alice.document().oplog_vv(), bob.document().oplog_vv());
        let text = alice.text_of(&first).expect("a text");
        assert_eq!(bob.text_of(&first).as_ref(), Some(&text));
        assert!(text.contains('A') && text.contains('B'), "{text}");
        assert_eq!(alice.schema_version(), SCHEMA_VERSION);
    }

    #[test]
    fn a_register_this_build_cannot_read_is_refused() {
        let doc = LoroDoc::new();
        doc.get_map(ambre_core::crdt::FoundationRoot::Meta.as_str())
            .insert(ambre_core::schema::MetaKey::SchemaVersion.as_str(), "one")
            .expect("a string register");
        doc.commit();
        let bytes = Payload::new(doc.export(ExportMode::Snapshot).expect("snapshot"));
        assert!(matches!(
            gate(&bytes),
            Err(SchemaError::VersionUnreadable { .. })
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
            gate(&Payload::new(vec![0xFF, 0x00, 0xFF])),
            Err(SchemaError::PayloadRejected)
        ));
    }
}
