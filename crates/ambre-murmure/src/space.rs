//! The state of an open space on the Logic thread
//! (`features/ambre-spec-murmure-spaces.md section 3`).
//!
//! A space is one document (`MURMURE-031`). It holds its name, its categories
//! and its catalogue: one entry per public channel, with the channel key
//! sealed under the space key (`MURMURE-033`). The engine stores the sealed
//! bytes and never opens them: no key material reaches this thread
//! (`MURMURE-157`).

use ambre_bus::{CatalogueEntry, CategoryView, SealedChannelKey, Timestamp, Topic};
use ambre_core::{CategoryId, ChannelId, DocumentId, SpaceId};
use loro::{ExportMode, LoroDoc, LoroMap, LoroValue, ValueOrContainer, VersionVector};

use crate::channel::ChannelState;
use crate::error::MurmureError;
use crate::schema::{
    self, CATALOGUE, CATEGORIES, CATEGORY_NAME, CATEGORY_POSITION, ChannelType, ENTRY_CATEGORY,
    ENTRY_KEY, ENTRY_NAME, ENTRY_POSITION, ENTRY_TOPIC, SPACE, SPACE_NAME,
};

/// A public channel the catalogue is asked to list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listing<'a> {
    /// Absent when the channel has none.
    pub topic: Option<&'a Topic>,
    /// Absent for a channel outside every category.
    pub category: Option<CategoryId>,
    /// The rank in the catalogue: lower first, ties by identifier.
    pub position: u32,
    /// The channel key, sealed under the space key by the Sync worker.
    pub key: SealedChannelKey,
}

/// An open space: its one document.
#[derive(Debug)]
pub struct SpaceState {
    space_id: SpaceId,
    doc: LoroDoc,
}

impl SpaceState {
    /// Create a new space document, named `name`.
    ///
    /// The document holds no member: the relay's rows are the members of a
    /// space (`MURMURE-031`). Creating the document on the relay makes its
    /// creator the owner (`MURMURE-035`).
    ///
    /// # Errors
    /// [`MurmureError::Crdt`] when Loro refuses a write.
    pub fn create(space_id: SpaceId, name: &str, now: Timestamp) -> Result<Self, MurmureError> {
        let doc = LoroDoc::new();
        schema::write_space_header(&doc, space_id, now.0)?;
        if !name.is_empty() {
            doc.get_map(SPACE).insert(SPACE_NAME, name)?;
        }
        doc.commit();
        Ok(Self { space_id, doc })
    }

    /// Open a space from the payloads of its document: a snapshot, then a
    /// replay. The header check runs first (`MURMURE-019`, `MURMURE-001`).
    ///
    /// # Errors
    /// Every refusal of the header check, and
    /// [`MurmureError::ForeignSpace`] for a document of another space.
    pub fn open(space_id: SpaceId, payloads: &[&[u8]]) -> Result<Self, MurmureError> {
        let doc = schema::admit_space(payloads, space_id)?;
        Ok(Self { space_id, doc })
    }

    /// The space this state holds.
    pub fn space_id(&self) -> SpaceId {
        self.space_id
    }

    /// The one document of the space.
    pub fn document_id(&self) -> DocumentId {
        self.space_id.document_id()
    }

    /// The name of the space, absent when it has none.
    pub fn name(&self) -> Option<String> {
        schema::string(&self.doc.get_map(SPACE), SPACE_NAME)
    }

    /// Every operation of the document, as one Loro update: the first
    /// payload the relay receives for a space just created.
    ///
    /// # Errors
    /// [`MurmureError::Crdt`] when Loro refuses the export.
    pub fn export_all(&self) -> Result<Vec<u8>, MurmureError> {
        Ok(self.doc.export(ExportMode::all_updates())?)
    }

    /// The whole document, for the relay's compaction.
    ///
    /// # Errors
    /// [`MurmureError::Crdt`] when Loro refuses the export.
    pub fn snapshot(&self) -> Result<Vec<u8>, MurmureError> {
        Ok(self.doc.export(ExportMode::Snapshot)?)
    }

    /// The version the document stands at, so a caller can tell whether a
    /// command wrote it.
    pub fn version(&self) -> VersionVector {
        self.doc.oplog_vv()
    }

    /// Import a remote delta of the space document.
    ///
    /// # Errors
    /// [`MurmureError::Crdt`] for bytes Loro refuses.
    pub fn apply_remote(&mut self, payload: &[u8]) -> Result<(), MurmureError> {
        self.doc.import(payload)?;
        Ok(())
    }

    /// Add a category, or rename and move one, and return the delta.
    ///
    /// # Errors
    /// [`MurmureError::Crdt`] when Loro refuses a write or the export.
    pub fn put_category(
        &mut self,
        category_id: CategoryId,
        name: &str,
        position: u32,
    ) -> Result<Vec<u8>, MurmureError> {
        let before = self.doc.oplog_vv();
        let entry = self
            .doc
            .get_map(CATEGORIES)
            .ensure_mergeable_map(&category_id.to_string())?;
        entry.insert(CATEGORY_NAME, name)?;
        entry.insert(CATEGORY_POSITION, i64::from(position))?;
        self.commit_since(&before)
    }

    /// List a public channel in the catalogue, and return the delta.
    ///
    /// The name comes from the channel's own `meta` document. Only a public
    /// channel enters (`MURMURE-014`): the catalogue is read by every member
    /// of the space. The listing writes the space document alone, so no
    /// membership changes (`MURMURE-038`).
    ///
    /// # Errors
    /// [`MurmureError::NotPublic`] for a direct or private channel, and
    /// [`MurmureError::Crdt`] when Loro refuses a write or the export.
    pub fn list_channel(
        &mut self,
        channel: &ChannelState,
        listing: &Listing<'_>,
    ) -> Result<Vec<u8>, MurmureError> {
        let channel_id = channel.channel_id();
        if channel.channel_type() != Some(ChannelType::Public) {
            return Err(MurmureError::NotPublic(channel_id));
        }
        let before = self.doc.oplog_vv();
        let entry = self
            .doc
            .get_map(CATALOGUE)
            .ensure_mergeable_map(&channel_id.to_string())?;
        entry.insert(ENTRY_NAME, channel.name().unwrap_or_default())?;
        match listing.topic {
            Some(topic) => entry.insert(ENTRY_TOPIC, topic.as_str())?,
            None => clear(&entry, ENTRY_TOPIC)?,
        }
        match listing.category {
            Some(category) => entry.insert(ENTRY_CATEGORY, category.to_string())?,
            None => clear(&entry, ENTRY_CATEGORY)?,
        }
        entry.insert(ENTRY_POSITION, i64::from(listing.position))?;
        entry.insert(ENTRY_KEY, listing.key.as_bytes().to_vec())?;
        self.commit_since(&before)
    }

    /// The sealed key of a listed channel, which a self-join hands the Sync
    /// worker unopened (`MURMURE-039`).
    ///
    /// # Errors
    /// [`MurmureError::NotListed`] when the catalogue lists no such channel,
    /// or holds no readable key for it.
    pub fn sealed_key(&self, channel_id: ChannelId) -> Result<SealedChannelKey, MurmureError> {
        let entry = child_map(&self.doc.get_map(CATALOGUE), &channel_id.to_string())
            .ok_or(MurmureError::NotListed(channel_id))?;
        match schema::value(&entry, ENTRY_KEY) {
            Some(LoroValue::Binary(bytes)) => Ok(SealedChannelKey::from_bytes(bytes.to_vec())),
            _ => Err(MurmureError::NotListed(channel_id)),
        }
    }

    /// The categories, by position then identifier.
    pub fn categories(&self) -> Vec<CategoryView> {
        let mut found = Vec::new();
        let categories = self.doc.get_map(CATEGORIES);
        categories.for_each(|key, value| {
            let (Ok(id), ValueOrContainer::Container(loro::Container::Map(entry))) =
                (CategoryId::parse_str(key), value)
            else {
                return;
            };
            found.push(CategoryView {
                id,
                name: schema::string(&entry, CATEGORY_NAME).unwrap_or_default(),
                position: position_of(&entry, CATEGORY_POSITION),
            });
        });
        found.sort_by_key(|c| (c.position, *c.id.as_bytes()));
        found
    }

    /// The catalogue, by position then identifier. An entry whose key no
    /// build wrote is skipped rather than refused.
    pub fn catalogue(&self) -> Vec<CatalogueEntry> {
        let mut found = Vec::new();
        self.doc.get_map(CATALOGUE).for_each(|key, value| {
            let (Ok(channel_id), ValueOrContainer::Container(loro::Container::Map(entry))) =
                (ChannelId::parse_str(key), value)
            else {
                return;
            };
            found.push(CatalogueEntry {
                channel_id,
                name: schema::string(&entry, ENTRY_NAME).unwrap_or_default(),
                topic: schema::string(&entry, ENTRY_TOPIC).and_then(Topic::new),
                category: schema::string(&entry, ENTRY_CATEGORY)
                    .and_then(|s| CategoryId::parse_str(&s).ok()),
                position: position_of(&entry, ENTRY_POSITION),
            });
        });
        found.sort_by_key(|c| (c.position, *c.channel_id.as_bytes()));
        found
    }

    /// Commit the pending writes, and export what they added.
    fn commit_since(&self, before: &VersionVector) -> Result<Vec<u8>, MurmureError> {
        self.doc.commit();
        Ok(self.doc.export(ExportMode::updates(before))?)
    }

    /// The document itself, for the shape belt.
    #[cfg(test)]
    pub(crate) fn doc(&self) -> &LoroDoc {
        &self.doc
    }
}

/// The map under `key`, or `None` when it is absent or not a map.
fn child_map(map: &LoroMap, key: &str) -> Option<LoroMap> {
    match map.get(key)? {
        ValueOrContainer::Container(loro::Container::Map(child)) => Some(child),
        _ => None,
    }
}

/// Remove `key` when present: an unset property is an absent key
/// (`MURMURE-018`), and removing one never written would write an operation.
fn clear(map: &LoroMap, key: &str) -> Result<(), MurmureError> {
    if map.get(key).is_some() {
        map.delete(key)?;
    }
    Ok(())
}

/// A stored position, saturating into `u32`. A value no build wrote reads as 0.
fn position_of(map: &LoroMap, key: &str) -> u32 {
    schema::unsigned(map, key)
        .map(|n| u32::try_from(n).unwrap_or(u32::MAX))
        .unwrap_or(0)
}
