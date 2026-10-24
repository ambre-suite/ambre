//! Toile's persisted schema version: the number, the register, and the
//! migration registry (`ambre-spec-schema-version.md`, `ambre-spec-toile-schema.md`).
//!
//! `ambre_core::schema` holds the protocol, and every other application shares
//! it. The protocol says where the register lives, what an absent one means,
//! what each comparison implies, and how a reader walks a registry.
//!
//! What lives here is what only Toile can know: which version its containers are
//! at today, and what each past layout has to become.
//!
//! **Cost.** [`gate`] imports the payload into a scratch document before it
//! touches the live state, so a full-state read decodes the snapshot twice.
//! That is deliberate, and it is the price of `SCHEMA-012`. Reading the version
//! off the live document would merge the refused document into the board the
//! user has open. The only way back out would be to throw their session away,
//! and a scratch document costs nothing to drop.

use std::borrow::Cow;

use loro::{ExportMode, LoroDoc, LoroValue};

use ambre_core::schema::{
    SCHEMA_VERSION_ABSENT, SchemaError, SchemaMigration, SchemaVerdict, SchemaVersion,
    migrate_schema, migration_peer, verdict,
};

/// The schema version this build writes into every board it creates
/// (`SCHEMA-004`).
///
/// Raising it is not a decision about this constant. It is a layout change to a
/// persisted record, and that obliges a [`MIGRATIONS`] row which carries an
/// existing document across it (`SCHEMA-009`).
pub const SCHEMA_VERSION: SchemaVersion = SchemaVersion::new(1);

/// Write the current version into a board's `meta` register.
///
/// Called from board construction, inside the scaffolding commit. The number
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

/// The schema version a board carries, or the refusal when the register is
/// present and unreadable.
///
/// An **absent** register is [`SCHEMA_VERSION_ABSENT`] (`SCHEMA-005`): a
/// board written before the register existed needs no rewriting. A present
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

/// Toile's registry: one row per version bump, appended never rewritten.
///
/// Version 1 is the first layout, and no build before it shipped, so the list
/// is empty. A board with no version reads as 0 (`SCHEMA-005`), and the walk
/// finds no step from 0 and refuses it. The first layout change after the
/// release adds its row here.
pub(crate) const MIGRATIONS: &[SchemaMigration<LoroDoc>] = &[];

/// The full-state read rule, applied before the caller touches a single
/// container (`SCHEMA-006`).
///
/// Returns the bytes the caller should actually import.
///
/// * The payload untouched, when the versions match.
/// * A migrated snapshot, when the document is older (`SCHEMA-007`).
/// * An error, when the document is newer (`SCHEMA-008`) or the engine refuses
///   the bytes.
///
/// Nothing here writes to the live document, so a refused read leaves the board
/// exactly as it was (`SCHEMA-012`).
///
/// # Errors
/// * [`SchemaError::PayloadRejected`] when the CRDT engine will not read the
///   bytes.
/// * [`SchemaError::DocumentTooNew`] when a newer client wrote the board.
/// * A migration variant when the chain cannot carry an older board.
pub fn gate(
    payload: &ambre_core::SnapshotBytes,
) -> Result<Cow<'_, ambre_core::SnapshotBytes>, SchemaError> {
    gate_through(MIGRATIONS, payload)
}

/// [`gate`] over the registry `steps`. A test hands it a step of its own, so
/// the walk stays proved while [`MIGRATIONS`] holds none.
fn gate_through<'a>(
    steps: &[SchemaMigration<LoroDoc>],
    payload: &'a ambre_core::SnapshotBytes,
) -> Result<Cow<'a, ambre_core::SnapshotBytes>, SchemaError> {
    let scratch = LoroDoc::new();
    scratch.import(payload.as_bytes()).map_err(|e| {
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
            // Every peer migrating this history writes as one peer, so their
            // steps mint one container, not rivals (`SCHEMA-020`). The live
            // board keeps its own identity: only the scratch takes this one.
            let heads = scratch.oplog_frontiers();
            let peer = migration_peer(heads.iter().map(|id| {
                (
                    ambre_core::PeerId(id.peer),
                    ambre_core::OpCounter(id.counter),
                )
            }));
            scratch
                .set_peer_id(peer.0)
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
                        reason: format!("re-exporting the migrated board failed: {e:?}"),
                    })?;
            Ok(Cow::Owned(ambre_core::SnapshotBytes::new(bytes)))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logic::model::ElementKind;
    use crate::logic::state::{BoardKey, ElementKey, Fields, Root, ToileState, entry, root_map};
    use loro::LoroText;

    /// A snapshot of a board carrying `version` in its register. A `None`
    /// version writes no register at all, which is every board written before
    /// `ARCH P8`.
    fn snapshot_at(version: Option<SchemaVersion>) -> Vec<u8> {
        let doc = LoroDoc::new();
        crate::logic::state::board(&doc)
            .insert(BoardKey::BgColor, 0i64)
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
    fn a_new_board_carries_the_current_version() {
        let state = ToileState::new();
        assert_eq!(state.schema_version(), SCHEMA_VERSION);
    }

    #[test]
    fn a_board_at_the_current_version_is_handed_back_untouched() {
        let bytes = snapshot_at(Some(SCHEMA_VERSION));
        let snapshot = ambre_core::SnapshotBytes::new(bytes.clone());
        let gated = gate(&snapshot).expect("current");
        assert!(matches!(gated, Cow::Borrowed(_)));
        assert_eq!(&*gated, &bytes[..]);
    }

    /// The `0 -> 1` step a test registry holds: it stamps the number and
    /// changes nothing else.
    fn stamp_step(doc: &LoroDoc) -> Result<(), String> {
        stamp(doc);
        doc.commit();
        Ok(())
    }

    #[test]
    fn a_board_written_before_the_register_migrates_to_the_current_version() {
        let steps = [SchemaMigration {
            from: SCHEMA_VERSION_ABSENT,
            to: SCHEMA_VERSION,
            apply: stamp_step,
        }];
        let bytes = snapshot_at(None);
        let snapshot = ambre_core::SnapshotBytes::new(bytes.clone());
        let gated = gate_through(&steps, &snapshot).expect("version 0 migrates");
        let migrated = LoroDoc::new();
        migrated
            .import(gated.as_bytes())
            .expect("the migrated snapshot imports");
        assert_eq!(
            read_version(&migrated).expect("the migrated register reads"),
            SCHEMA_VERSION
        );
        // The migration stamps and does nothing else: the board it carried is
        // still there.
        assert!(
            crate::logic::state::board(&migrated).contains(BoardKey::BgColor),
            "the migration must not drop what it migrates"
        );
    }

    /// No build wrote a board before version 1, so the registry holds no step
    /// and a board with no register finds no path.
    #[test]
    fn a_board_written_before_the_register_finds_no_step() {
        let snapshot = ambre_core::SnapshotBytes::new(snapshot_at(None));
        assert_eq!(
            gate(&snapshot).expect_err("no step leaves version 0"),
            SchemaError::NoMigrationPath {
                from: SCHEMA_VERSION_ABSENT
            }
        );
    }

    /// The `0 -> 1` step of the `SCHEMA-020` test: each string body becomes a
    /// text container, which every peer mints under one map key. Sorted, so
    /// every peer mints each text at the same identifier.
    fn body_text_step(doc: &LoroDoc) -> Result<(), String> {
        let root = root_map(doc, Root::Elements);
        let LoroValue::Map(entries) = root.get_value() else {
            return Ok(());
        };
        let mut ids: Vec<&String> = entries.keys().collect();
        ids.sort();
        for id in ids {
            let Some(map) = entry::<ElementKey>(&root, id) else {
                continue;
            };
            let Some(LoroValue::String(body)) = map.value(ElementKey::Body) else {
                continue;
            };
            let text = map
                .insert_container(ElementKey::Body, LoroText::new())
                .map_err(|e| e.to_string())?;
            text.insert(0, &body).map_err(|e| e.to_string())?;
        }
        stamp(doc);
        doc.commit();
        Ok(())
    }

    /// `SCHEMA-020`: two peers open one version 0 board, and each migrates its
    /// own copy through a step that mints a text per body. Each then types into
    /// the same body before the two meet. Both edits survive, because both
    /// peers minted the same text, not two rivals.
    #[test]
    fn two_peers_migrating_one_board_keep_both_edits() {
        let doc = LoroDoc::new();
        let elements = doc.get_map(Root::Elements.as_str());
        for i in 0..24 {
            let em = Fields::<ElementKey>::of(
                elements
                    .insert_container(&format!("e{i}"), loro::LoroMap::new())
                    .expect("seed"),
            );
            em.insert(ElementKey::Type, ElementKind::Sticky.type_tag())
                .expect("seed");
            em.insert(ElementKey::Body, "buy milk")
                .expect("the layout before the text");
        }
        doc.commit();
        let bytes = doc.export(ExportMode::Snapshot).expect("snapshot");

        let steps = [SchemaMigration {
            from: SCHEMA_VERSION_ABSENT,
            to: SCHEMA_VERSION,
            apply: body_text_step,
        }];
        let open = |peer: ambre_core::PeerId| {
            let live = LoroDoc::new();
            live.set_peer_id(peer.0).expect("a peer of its own");
            live.import(
                gate_through(&steps, &bytes.clone().into())
                    .expect("version 0 migrates")
                    .as_bytes(),
            )
            .expect("imports");
            live
        };
        let body = |live: &LoroDoc| {
            live.get_map(Root::Elements.as_str())
                .get("e7")
                .and_then(|v| v.into_container().ok())
                .and_then(|c| c.into_map().ok())
                .and_then(|em| em.get(ElementKey::Body.as_str()))
                .and_then(|v| v.into_container().ok())
                .and_then(|c| c.into_text().ok())
                .expect("the body is a text")
        };
        let (a, b) = (open(ambre_core::PeerId(1)), open(ambre_core::PeerId(2)));
        body(&a).insert(0, "now ").expect("a types");
        a.commit();
        body(&b).insert(8, " and eggs").expect("b types");
        b.commit();
        a.import(&b.export(ExportMode::all_updates()).expect("b's history"))
            .expect("a merges");
        b.import(&a.export(ExportMode::all_updates()).expect("a's history"))
            .expect("b merges");

        for (peer, live) in [("a", &a), ("b", &b)] {
            assert_eq!(
                body(live).to_string(),
                "now buy milk and eggs",
                "{peer} kept both edits"
            );
        }
        assert_eq!(a.oplog_vv(), b.oplog_vv());
    }

    #[test]
    fn a_board_from_a_newer_client_is_refused_whole() {
        let bytes = snapshot_at(Some(SCHEMA_VERSION.next()));
        let err = gate(&bytes.clone().into()).expect_err("refused");
        assert_eq!(
            err,
            SchemaError::DocumentTooNew {
                document: SCHEMA_VERSION.next(),
                binary: SCHEMA_VERSION,
            }
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
            gate(&bytes.clone().into()).expect_err("refused"),
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
            gate(&vec![0xFF, 0x00, 0xFF].into()).expect_err("refused"),
            SchemaError::PayloadRejected
        ));
    }

    #[test]
    fn bytes_the_engine_will_not_read_are_told_apart_from_a_version_refusal() {
        assert_eq!(
            gate(&b"not a loro snapshot".to_vec().into()).expect_err("rejected"),
            SchemaError::PayloadRejected
        );
    }
}
