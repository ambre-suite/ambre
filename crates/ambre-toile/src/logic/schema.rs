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

use loro::{ExportMode, LoroDoc, LoroText, LoroValue};

use ambre_core::schema::{
    SCHEMA_VERSION_ABSENT, SchemaError, SchemaMigration, SchemaVerdict, SchemaVersion,
    migrate_schema, migration_peer, verdict,
};

use crate::logic::model::ElementStyle;
use crate::logic::state::{
    ConnectorKey, ElementKey, Fields, LegacyDeckKey, LegacyRoot, Root, entry, root_list, root_map,
};

/// The schema version this build writes into every board it creates
/// (`SCHEMA-004`).
///
/// Raising it is not a decision about this constant. It is a layout change to a
/// persisted record, and that obliges a [`MIGRATIONS`] row which carries an
/// existing document across it (`SCHEMA-009`). `1 -> 2` puts the layout version
/// in front of every `"style"` register, which obliges the [`MIGRATIONS`] row
/// that rewrites the registers an older board holds. `2 -> 3` moves the deck
/// out of the `presentation` map and into two root containers. `3 -> 4` turns
/// each note and text body from a string register into a text container. `4 ->
/// 5` stores a style's font size in points rather than in world px.
pub const SCHEMA_VERSION: SchemaVersion = SchemaVersion::new(5);

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
///
/// `3 -> 4` moves each string body into a text container (`TOILE-030`).
///
/// `4 -> 5` stores the font size of a style in points, the unit the size
/// picker and every other view use, in place of world px.
pub(crate) const MIGRATIONS: &[SchemaMigration<LoroDoc>] = &[
    SchemaMigration {
        from: SchemaVersion::new(0),
        to: SchemaVersion::new(1),
        apply: stamp_step,
    },
    SchemaMigration {
        from: SchemaVersion::new(1),
        to: SchemaVersion::new(2),
        apply: style_version_step,
    },
    SchemaMigration {
        from: SchemaVersion::new(2),
        to: SchemaVersion::new(3),
        apply: presentation_root_step,
    },
    SchemaMigration {
        from: SchemaVersion::new(3),
        to: SchemaVersion::new(4),
        apply: body_text_step,
    },
    SchemaMigration {
        from: SchemaVersion::new(4),
        to: SchemaVersion::new(5),
        apply: font_points_step,
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
    rewrite_styles(doc, ElementStyle::upgrade_unversioned)?;
    stamp(doc);
    doc.commit();
    Ok(())
}

/// Rewrite every `"style"` register of the elements and the connectors that
/// `rewrite` has new bytes for. A register `rewrite` refuses is left verbatim
/// (`SCHEMA-003`).
fn rewrite_styles(doc: &LoroDoc, rewrite: impl Fn(&[u8]) -> Option<Vec<u8>>) -> Result<(), String> {
    rewrite_style_registers(doc, Root::Elements, ElementKey::Style, &rewrite)?;
    rewrite_style_registers(doc, Root::Connectors, ConnectorKey::Style, &rewrite)
}

/// Rewrite the register `key` of every entry of the dictionary `root`. A
/// register `rewrite` refuses is left verbatim (`SCHEMA-003`).
fn rewrite_style_registers<K: ambre_core::crdt::FieldKey>(
    doc: &LoroDoc,
    root: Root,
    key: K,
    rewrite: &impl Fn(&[u8]) -> Option<Vec<u8>>,
) -> Result<(), String> {
    let dict = root_map(doc, root);
    let LoroValue::Map(entries) = dict.get_value() else {
        return Ok(());
    };
    // Sorted: two peers migrating one history write the same operations only
    // if they write them in the same order (`SCHEMA-020`).
    let mut ids: Vec<&String> = entries.keys().collect();
    ids.sort();
    for id in ids {
        let Some(map) = entry::<K>(&dict, id) else {
            continue;
        };
        let Some(LoroValue::Binary(bytes)) = map.value(key) else {
            continue;
        };
        if let Some(upgraded) = rewrite(&bytes) {
            map.insert(key, LoroValue::Binary(upgraded.into()))
                .map_err(|e| format!("rewriting the style register of {id} failed: {e}"))?;
        }
    }
    Ok(())
}

/// The `4 -> 5` step: carry each font size from world px to points.
///
/// A version 4 board stores the size the renderer lays out, in world px, which
/// is CSS px at zoom 1. This build stores points and lays them out through
/// [`FontSize::to_px`](ambre_core::FontSize::to_px). The step carries each
/// size back to points through [`DocPx::to_pt`](ambre_core::DocPx::to_pt), so
/// a body keeps the height it had. Zero, an automatic size, stays automatic.
fn font_points_step(doc: &LoroDoc) -> Result<(), String> {
    rewrite_styles(doc, |bytes| {
        let mut style = ElementStyle::try_from_bytes(bytes)?;
        let px = f32::from(style.font.size);
        style.font.size = ambre_bus::app::toile::FontSizeChoice::try_from(
            ambre_core::DocPx::new(px).to_pt().get(),
        )
        .ok()?;
        Some(style.to_bytes())
    })?;
    stamp(doc);
    doc.commit();
    Ok(())
}

/// The `2 -> 3` step: carry the deck into its two root containers
/// (`TOILE-029`).
///
/// A version 2 board holds `presentation.order` and `presentation.config`, two
/// sub-containers created by the first peer to write a deck. The step copies
/// the slide order into [`Root::PresentationOrder`] and the config registers into
/// [`Root::PresentationConfig`]. A root container's identity is its name, so every
/// peer addresses the same one.
///
/// The old map is left verbatim (`SCHEMA-003`): this build names neither key
/// again, and rewriting a container it no longer reads buys nothing.
fn presentation_root_step(doc: &LoroDoc) -> Result<(), String> {
    let pres = Fields::<LegacyDeckKey>::of(doc.get_map(LegacyRoot::Presentation.as_str()));
    if let Some(old) = pres
        .get(LegacyDeckKey::Order)
        .and_then(|v| v.into_container().ok())
        .and_then(|c| c.into_movable_list().ok())
        && let LoroValue::List(items) = old.get_deep_value()
    {
        let root = root_list(doc, Root::PresentationOrder);
        for value in items.iter() {
            if let LoroValue::String(id) = value {
                root.push(id.as_str())
                    .map_err(|e| format!("carrying a slide to the root list failed: {e}"))?;
            }
        }
    }
    let root_cfg = root_map(doc, Root::PresentationConfig);
    if let Some(old) = pres
        .get(LegacyDeckKey::Config)
        .and_then(|v| v.into_container().ok())
        .and_then(|c| c.into_map().ok())
        && let LoroValue::Map(entries) = old.get_deep_value()
    {
        let mut pairs: Vec<(&String, &LoroValue)> = entries.iter().collect();
        pairs.sort_by_key(|(key, _)| *key);
        for (key, value) in pairs {
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

/// The `3 -> 4` step: move each string body into a text container
/// (`TOILE-030`).
///
/// A version 3 board holds the body of a note or a text element as one string
/// register, which two authors race for whole. The step replaces each such
/// register with a text holding the same characters. A body already a text is
/// left alone.
///
/// Two peers migrating one board write as one derived peer, so both mint the
/// same text and an edit typed into either survives (`SCHEMA-020`).
fn body_text_step(doc: &LoroDoc) -> Result<(), String> {
    let root = root_map(doc, Root::Elements);
    let LoroValue::Map(entries) = root.get_value() else {
        return Ok(());
    };
    // Sorted, so every peer mints each text at the same identifier.
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
            .map_err(|e| format!("seeding the body text of {id} failed: {e}"))?;
        if !body.is_empty() {
            text.insert(0, &body)
                .map_err(|e| format!("writing the body of {id} failed: {e}"))?;
        }
    }
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
pub fn gate(
    payload: &ambre_core::SnapshotBytes,
) -> Result<Cow<'_, ambre_core::SnapshotBytes>, SchemaError> {
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
            migrate_schema(MIGRATIONS, found, SCHEMA_VERSION, &scratch)?;
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
    use crate::logic::state::{BoardKey, ConfigKey, ToileState};
    use ambre_core::Rgba;

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

    #[test]
    fn a_board_written_before_the_register_migrates_to_the_current_version() {
        let bytes = snapshot_at(None);
        let snapshot = ambre_core::SnapshotBytes::new(bytes.clone());
        let gated = gate(&snapshot).expect("version 0 migrates");
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

    /// A version 1 board: one element and one connector, each carrying the bare
    /// postcard style register that layout stored.
    fn unversioned_board(style: &ElementStyle) -> Vec<u8> {
        let doc = LoroDoc::new();
        let bare = postcard::to_allocvec(style).expect("the layout before the version byte");
        let style = LoroValue::Binary(bare.into());
        Fields::<ElementKey>::of(
            root_map(&doc, Root::Elements)
                .insert_container("e1", loro::LoroMap::new())
                .expect("seed"),
        )
        .insert(ElementKey::Style, style.clone())
        .expect("seed the style register");
        Fields::<ConnectorKey>::of(
            root_map(&doc, Root::Connectors)
                .insert_container("c1", loro::LoroMap::new())
                .expect("seed"),
        )
        .insert(ConnectorKey::Style, style)
        .expect("seed the style register");
        doc.get_map(ambre_core::crdt::FoundationRoot::Meta.as_str())
            .insert(ambre_core::schema::MetaKey::SchemaVersion.as_str(), 1i64)
            .expect("stamp");
        doc.commit();
        doc.export(ExportMode::Snapshot).expect("snapshot")
    }

    /// Read every `"style"` register of a migrated snapshot back.
    fn styles_of(bytes: &[u8]) -> Vec<Option<ElementStyle>> {
        let doc = LoroDoc::new();
        doc.import(bytes).expect("the migrated snapshot imports");
        let element = entry::<ElementKey>(&root_map(&doc, Root::Elements), "e1")
            .expect("the migration must not drop the map")
            .value(ElementKey::Style);
        let connector = entry::<ConnectorKey>(&root_map(&doc, Root::Connectors), "c1")
            .expect("the migration must not drop the map")
            .value(ConnectorKey::Style);
        [element, connector]
            .into_iter()
            .map(|register| {
                let Some(LoroValue::Binary(b)) = register else {
                    panic!("the migration must not drop the register");
                };
                ElementStyle::try_from_bytes(&b)
            })
            .collect()
    }

    /// `SCHEMA-007`: a board written before the style register carried its
    /// layout version keeps its styles. Without the `1 -> 2` row, this build
    /// reads the first postcard byte as a layout it does not know. Every element
    /// then falls back to the default look.
    #[test]
    fn a_board_whose_styles_carry_no_version_keeps_them() {
        let style = ElementStyle {
            fill: Some(Rgba::from_bits(0xaabb_ccff)),
            corner: 8.0,
            ..ElementStyle::default()
        };
        let board = unversioned_board(&style);
        let snapshot = ambre_core::SnapshotBytes::new(board.clone());
        let gated = gate(&snapshot).expect("version 1 migrates");
        // The `4 -> 5` row then reads the stored size as px, and carries it to
        // points.
        let mut kept = style;
        kept.font.size = ambre_bus::app::toile::FontSizeChoice::try_from(
            ambre_core::DocPx::new(f32::from(kept.font.size))
                .to_pt()
                .get(),
        )
        .expect("a size");
        assert_eq!(
            styles_of(gated.as_bytes()),
            vec![Some(kept.clone()), Some(kept)],
            "the element and the connector both keep the style they carried"
        );
    }

    /// `SCHEMA-007`, the `2 -> 3` row: a version 2 board keeps its deck. The
    /// slides live under `presentation.order`, which this build never reads
    /// again, so without the step the deck opens empty.
    #[test]
    fn a_board_whose_deck_sits_in_the_presentation_map_keeps_it() {
        let doc = LoroDoc::new();
        let pres = Fields::<LegacyDeckKey>::of(doc.get_map(LegacyRoot::Presentation.as_str()));
        let old = pres
            .insert_container(LegacyDeckKey::Order, loro::LoroMovableList::new())
            .expect("the layout before the root list");
        old.push("f1").expect("seed");
        old.push("f2").expect("seed");
        let cfg = pres
            .insert_container(LegacyDeckKey::Config, loro::LoroMap::new())
            .expect("seed");
        cfg.insert(ConfigKey::TransitionMs.as_str(), 800.0)
            .expect("seed");
        doc.get_map(ambre_core::crdt::FoundationRoot::Meta.as_str())
            .insert(ambre_core::schema::MetaKey::SchemaVersion.as_str(), 2i64)
            .expect("stamp");
        doc.commit();
        let bytes = doc.export(ExportMode::Snapshot).expect("snapshot");

        let snapshot = ambre_core::SnapshotBytes::new(bytes.clone());
        let gated = gate(&snapshot).expect("version 2 migrates");
        let mut state = ToileState::new();
        state.apply_remote_snapshot(&gated);
        let order: Vec<String> = state
            .slide_order()
            .into_iter()
            .map(|id| id.0.clone())
            .collect();
        assert_eq!(order, vec!["f1".to_owned(), "f2".to_owned()]);

        let migrated = LoroDoc::new();
        migrated.import(gated.as_bytes()).expect("imports");
        assert_eq!(
            Fields::<ConfigKey>::of(root_map(&migrated, Root::PresentationConfig))
                .value(ConfigKey::TransitionMs),
            Some(LoroValue::Double(800.0)),
            "the config the board carried must survive the move"
        );
    }

    /// `TOILE-030`, the `3 -> 4` row: a version 3 board keeps every body, now
    /// as a text two authors can share. Without the step the body
    /// would still read, and still race whole.
    #[test]
    fn a_board_whose_bodies_are_strings_keeps_them_as_text() {
        let doc = LoroDoc::new();
        let elements = doc.get_map(Root::Elements.as_str());
        for (id, body) in [("e1", "buy milk"), ("e2", "")] {
            let em = Fields::<ElementKey>::of(
                elements
                    .insert_container(id, loro::LoroMap::new())
                    .expect("seed"),
            );
            em.insert(ElementKey::Type, ElementKind::Sticky.type_tag())
                .expect("seed");
            em.insert(ElementKey::Body, body)
                .expect("the layout before the text");
        }
        doc.get_map(ambre_core::crdt::FoundationRoot::Meta.as_str())
            .insert(ambre_core::schema::MetaKey::SchemaVersion.as_str(), 3i64)
            .expect("stamp");
        doc.commit();
        let bytes = doc.export(ExportMode::Snapshot).expect("snapshot");

        let snapshot = ambre_core::SnapshotBytes::new(bytes.clone());
        let gated = gate(&snapshot).expect("version 3 migrates");
        let migrated = LoroDoc::new();
        migrated.import(gated.as_bytes()).expect("imports");
        for (id, body) in [("e1", "buy milk"), ("e2", "")] {
            let text = migrated
                .get_map(Root::Elements.as_str())
                .get(id)
                .and_then(|v| v.into_container().ok())
                .and_then(|c| c.into_map().ok())
                .and_then(|em| em.get(ElementKey::Body.as_str()))
                .and_then(|v| v.into_container().ok())
                .and_then(|c| c.into_text().ok())
                .expect("the body is a text");
            assert_eq!(text.to_string(), body, "{id} lost its body");
        }
    }

    /// `SCHEMA-007`, the `4 -> 5` row: a version 4 board keeps the height of
    /// its text. That board stores world px, and this build reads points and
    /// lays them out at 4/3 of their value.
    #[test]
    fn a_board_whose_font_sizes_are_px_keeps_their_height() {
        let doc = LoroDoc::new();
        let elements = doc.get_map(Root::Elements.as_str());
        for (id, px) in [("e1", Some(16)), ("e2", None)] {
            let mut style = ElementStyle::default();
            style.font.size = ambre_bus::app::toile::FontSizeChoice::from_whole_points(px);
            elements
                .insert_container(id, loro::LoroMap::new())
                .expect("seed")
                .insert(
                    ElementKey::Style.as_str(),
                    LoroValue::Binary(style.to_bytes().into()),
                )
                .expect("a version 4 style");
        }
        doc.get_map(ambre_core::crdt::FoundationRoot::Meta.as_str())
            .insert(ambre_core::schema::MetaKey::SchemaVersion.as_str(), 4i64)
            .expect("stamp");
        doc.commit();
        let bytes = doc.export(ExportMode::Snapshot).expect("snapshot");

        let snapshot = ambre_core::SnapshotBytes::new(bytes.clone());
        let gated = gate(&snapshot).expect("version 4 migrates");
        let migrated = LoroDoc::new();
        migrated.import(gated.as_bytes()).expect("imports");
        let size_of = |id: &str| {
            let Some(LoroValue::Binary(raw)) =
                entry::<ElementKey>(&root_map(&migrated, Root::Elements), id)
                    .and_then(|em| em.value(ElementKey::Style))
            else {
                panic!("{id} lost its style");
            };
            ElementStyle::try_from_bytes(&raw)
                .expect("decodes")
                .font
                .size
        };
        let e1 = size_of("e1").as_fixed();
        assert_eq!(
            e1.map(ambre_core::FontSize::to_px),
            Some(ambre_core::DocPx::new(16.0)),
            "16 px read back as 16 px"
        );
        assert_eq!(e1, Some(ambre_core::FontSize::from_whole_points(12)));
        assert_eq!(size_of("e2"), ambre_bus::app::toile::FontSizeChoice::AUTO);
    }

    /// `SCHEMA-020`: two peers open one version 3 board, and each migrates its
    /// own copy. Each then types into the same body before the two meet. Both
    /// edits survive, because both peers minted the same text, not two rivals.
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
        doc.get_map(ambre_core::crdt::FoundationRoot::Meta.as_str())
            .insert(ambre_core::schema::MetaKey::SchemaVersion.as_str(), 3i64)
            .expect("stamp");
        doc.commit();
        let bytes = doc.export(ExportMode::Snapshot).expect("snapshot");

        let open = |peer: ambre_core::PeerId| {
            let live = LoroDoc::new();
            live.set_peer_id(peer.0).expect("a peer of its own");
            live.import(
                gate(&bytes.clone().into())
                    .expect("version 3 migrates")
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
