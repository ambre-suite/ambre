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
    SCHEMA_META_MAP, SCHEMA_VERSION_ABSENT, SCHEMA_VERSION_KEY, SchemaError, SchemaMigration,
    SchemaVerdict, migrate_schema, verdict,
};

use crate::logic::model::ElementStyle;
use crate::logic::state::{CONNECTORS, ELEMENTS, PRESENTATION_CONFIG, PRESENTATION_ORDER};

/// The schema version this build writes into every board it creates
/// (`SCHEMA-004`).
///
/// Raising it is not a decision about this constant. It is a layout change to a
/// persisted record, and that obliges a [`MIGRATIONS`] row which carries an
/// existing document across it (`SCHEMA-009`). `1 -> 2` puts the layout version
/// in front of every `"style"` register, which obliges the [`MIGRATIONS`] row
/// that rewrites the registers an older board holds. `2 -> 3` moves the deck
/// out of the `presentation` map and into two root containers.
pub const SCHEMA_VERSION: u32 = 3;

/// Write the current version into a board's `meta` register.
///
/// Called from board construction, inside the scaffolding commit. The number
/// therefore reaches the document before any user edit. It then travels through
/// every persistence path, not through the one that happens to check it.
pub(crate) fn stamp(doc: &LoroDoc) {
    let _ = doc
        .get_map(SCHEMA_META_MAP)
        .insert(SCHEMA_VERSION_KEY, i64::from(SCHEMA_VERSION));
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
/// reads as version 0, and its layout is identical to a version 1 board. The
/// step therefore stamps the number and changes nothing else, which is what
/// makes it a migration rather than a rewrite.
///
/// `1 -> 2` puts [`STYLE_VERSION`] in front of every `"style"` register, the
/// element registers and the connector registers alike.
///
/// `2 -> 3` copies the slide order and the presentation config out of the
/// `presentation` map, whose two sub-containers each peer created for itself.
/// They land in the two root containers every peer addresses alike
/// (`TOILE-029`).
const MIGRATIONS: &[SchemaMigration<LoroDoc>] = &[
    SchemaMigration {
        from: 0,
        to: 1,
        apply: stamp_step,
    },
    SchemaMigration {
        from: 1,
        to: 2,
        apply: style_version_step,
    },
    SchemaMigration {
        from: 2,
        to: 3,
        apply: presentation_root_step,
    },
];

/// The `0 -> 1` step. A free function rather than a closure because a registry
/// row holds a plain `fn`.
fn stamp_step(doc: &LoroDoc) -> Result<(), String> {
    stamp(doc);
    doc.commit();
    Ok(())
}

/// The `1 -> 2` step: put the layout version in front of every `"style"`
/// register.
///
/// A version 1 board stores the bare postcard value. This build reads the first
/// byte as the layout version (`TOILE-UI-071`). Every such register therefore
/// decodes as a layout it does not know, and the element loses its style. The
/// step rewrites each one through
/// [`ElementStyle::upgrade_unversioned`](ambre_bus::app::toile::ElementStyle::upgrade_unversioned).
///
/// A register that call refuses is left verbatim (`SCHEMA-003`): the layout the
/// step cannot name is not one it may overwrite.
fn style_version_step(doc: &LoroDoc) -> Result<(), String> {
    for container in [ELEMENTS, CONNECTORS] {
        let root = doc.get_map(container);
        let LoroValue::Map(entries) = root.get_value() else {
            continue;
        };
        for id in entries.keys() {
            let Some(map) = root
                .get(id.as_str())
                .and_then(|v| v.into_container().ok())
                .and_then(|c| c.into_map().ok())
            else {
                continue;
            };
            let Some(Ok(LoroValue::Binary(bytes))) = map.get("style").map(|v| v.into_value())
            else {
                continue;
            };
            if let Some(upgraded) = ElementStyle::upgrade_unversioned(&bytes) {
                map.insert("style", LoroValue::Binary(upgraded.into()))
                    .map_err(|e| format!("rewriting the style register of {id} failed: {e}"))?;
            }
        }
    }
    stamp(doc);
    doc.commit();
    Ok(())
}

/// The `2 -> 3` step: carry the deck into its two root containers
/// (`TOILE-029`).
///
/// A version 2 board holds `presentation.order` and `presentation.config`, two
/// sub-containers created by the first peer to write a deck. The step copies
/// the slide order into [`PRESENTATION_ORDER`] and the config registers into
/// [`PRESENTATION_CONFIG`]. A root container's identity is its name, so every
/// peer addresses the same one.
///
/// The old map is left verbatim (`SCHEMA-003`): this build names neither key
/// again, and rewriting a container it no longer reads buys nothing.
fn presentation_root_step(doc: &LoroDoc) -> Result<(), String> {
    let pres = doc.get_map("presentation");
    if let Some(old) = pres
        .get("order")
        .and_then(|v| v.into_container().ok())
        .and_then(|c| c.into_movable_list().ok())
        && let LoroValue::List(items) = old.get_deep_value()
    {
        let root = doc.get_movable_list(PRESENTATION_ORDER);
        for value in items.iter() {
            if let LoroValue::String(id) = value {
                root.push(id.as_str())
                    .map_err(|e| format!("carrying a slide to the root list failed: {e}"))?;
            }
        }
    }
    let root_cfg = doc.get_map(PRESENTATION_CONFIG);
    if let Some(old) = pres
        .get("config")
        .and_then(|v| v.into_container().ok())
        .and_then(|c| c.into_map().ok())
        && let LoroValue::Map(entries) = old.get_deep_value()
    {
        for (key, value) in entries.iter() {
            root_cfg
                .insert(key.as_str(), value.clone())
                .map_err(|e| format!("carrying the config key {key} failed: {e}"))?;
        }
    }
    // A board that carried no deck still gets the defaults a fresh one has.
    crate::logic::state::presentation::seed_presentation_config(doc);
    stamp(doc);
    doc.commit();
    Ok(())
}

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

    /// A snapshot of a board carrying `version` in its register. A `None`
    /// version writes no register at all, which is every board written before
    /// `ARCH P8`.
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

    /// A version 1 board: one element and one connector, each carrying the bare
    /// postcard style register that layout stored.
    fn unversioned_board(style: &ElementStyle) -> Vec<u8> {
        let doc = LoroDoc::new();
        let bare = postcard::to_allocvec(style).expect("the layout before the version byte");
        for (container, id) in [(ELEMENTS, "e1"), (CONNECTORS, "c1")] {
            let map = doc
                .get_map(container)
                .insert_container(id, loro::LoroMap::new())
                .expect("seed");
            map.insert("style", LoroValue::Binary(bare.clone().into()))
                .expect("seed the style register");
        }
        doc.get_map(SCHEMA_META_MAP)
            .insert(SCHEMA_VERSION_KEY, 1i64)
            .expect("stamp");
        doc.commit();
        doc.export(ExportMode::Snapshot).expect("snapshot")
    }

    /// Read every `"style"` register of a migrated snapshot back.
    fn styles_of(bytes: &[u8]) -> Vec<Option<ElementStyle>> {
        let doc = LoroDoc::new();
        doc.import(bytes).expect("the migrated snapshot imports");
        [(ELEMENTS, "e1"), (CONNECTORS, "c1")]
            .into_iter()
            .map(|(container, id)| {
                let map = doc
                    .get_map(container)
                    .get(id)
                    .and_then(|v| v.into_container().ok())
                    .and_then(|c| c.into_map().ok())
                    .expect("the migration must not drop the map");
                let Some(Ok(LoroValue::Binary(b))) = map.get("style").map(|v| v.into_value())
                else {
                    panic!("the migration must not drop the register");
                };
                ElementStyle::try_from_bytes(&b)
            })
            .collect()
    }

    /// `SCHEMA-009`: a board written before the style register carried its
    /// layout version keeps its styles. Without the `1 -> 2` row, this build
    /// reads the first postcard byte as a layout it does not know. Every element
    /// then falls back to the default look.
    #[test]
    fn a_board_whose_styles_carry_no_version_keeps_them() {
        let style = ElementStyle {
            fill: Some(0xffaa_bbcc),
            corner: 8.0,
            ..ElementStyle::default()
        };
        let board = unversioned_board(&style);
        let gated = gate(&board).expect("version 1 migrates");
        assert_eq!(
            styles_of(&gated),
            vec![Some(style.clone()), Some(style)],
            "the element and the connector both keep the style they carried"
        );
    }

    /// `SCHEMA-009`, the `2 -> 3` row: a version 2 board keeps its deck. The
    /// slides live under `presentation.order`, which this build never reads
    /// again, so without the step the deck opens empty.
    #[test]
    fn a_board_whose_deck_sits_in_the_presentation_map_keeps_it() {
        let doc = LoroDoc::new();
        let pres = doc.get_map("presentation");
        let old = pres
            .insert_container("order", loro::LoroMovableList::new())
            .expect("the layout before the root list");
        old.push("f1").expect("seed");
        old.push("f2").expect("seed");
        let cfg = pres
            .insert_container("config", loro::LoroMap::new())
            .expect("seed");
        cfg.insert("transition_ms", 800.0).expect("seed");
        doc.get_map(SCHEMA_META_MAP)
            .insert(SCHEMA_VERSION_KEY, 2i64)
            .expect("stamp");
        doc.commit();
        let bytes = doc.export(ExportMode::Snapshot).expect("snapshot");

        let gated = gate(&bytes).expect("version 2 migrates");
        let mut state = ToileState::new();
        state.apply_remote(&gated);
        let order: Vec<String> = state
            .slide_order()
            .into_iter()
            .map(|id| id.0.clone())
            .collect();
        assert_eq!(order, vec!["f1".to_owned(), "f2".to_owned()]);

        let migrated = LoroDoc::new();
        migrated.import(&gated).expect("imports");
        assert_eq!(
            migrated
                .get_map(PRESENTATION_CONFIG)
                .get("transition_ms")
                .and_then(|v| v.into_value().ok()),
            Some(LoroValue::Double(800.0)),
            "the config the board carried must survive the move"
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

    /// `SCHEMA-013`: the back door `SCHEMA-005` opens. The gate must not treat a
    /// register holding something other than a version as no register at all.
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
