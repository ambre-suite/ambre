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
use crate::session::Session;

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
    /// The PeerID this session writes the document under. `None` until the
    /// Sync worker issues it, and nothing is written before (`AUTHOR-029`).
    session: Option<Session>,
    /// The name and the creation time of a space created here, written again
    /// under its PeerID once the session lands. `None` for a space the relay
    /// served.
    created: Option<(String, Timestamp)>,
}

impl SpaceState {
    /// Create a new space document, named `name`.
    ///
    /// The document holds no member: the relay's rows are the members of a
    /// space (`MURMURE-031`). Creating the document on the relay makes its
    /// creator the owner (`MURMURE-035`).
    ///
    /// The space reads at once. Its document is written again under its
    /// PeerID when its session lands, and only then does it leave
    /// ([`SpaceState::adopt_session`]).
    ///
    /// # Errors
    /// [`MurmureError::NameTooLong`] for a name past its cap.
    /// [`MurmureError::Crdt`] when Loro refuses a write.
    pub fn create(space_id: SpaceId, name: &str, now: Timestamp) -> Result<Self, MurmureError> {
        crate::channel::fitting(Some(name))?;
        let doc = LoroDoc::new();
        write_created(&doc, space_id, name, now)?;
        Ok(Self {
            space_id,
            doc,
            session: None,
            created: Some((name.to_owned(), now)),
        })
    }

    /// Open a space from the payloads of its document: a snapshot, then a
    /// replay. The header check runs first (`MURMURE-019`, `MURMURE-001`).
    ///
    /// # Errors
    /// Every refusal of the header check, and
    /// [`MurmureError::ForeignSpace`] for a document of another space.
    pub fn open(space_id: SpaceId, payloads: &[&[u8]]) -> Result<Self, MurmureError> {
        let doc = schema::admit_space(payloads, space_id)?;
        Ok(Self {
            space_id,
            doc,
            session: None,
            created: None,
        })
    }

    /// Write the document under the PeerID the Sync worker issued for this
    /// session, with its registry entry (`AUTHOR-002`, `AUTHOR-009`).
    ///
    /// A space created here is written again from scratch under it, and its
    /// whole content returns: the first update, which creates it on the
    /// relay. A space the relay served returns nothing: its entry rides with
    /// its first delta. The same identity issued twice changes nothing.
    ///
    /// # Errors
    /// [`MurmureError::Crdt`] when Loro refuses a write or the export.
    pub fn adopt_session(
        &mut self,
        peer: ambre_core::PeerId,
        entry: Vec<u8>,
    ) -> Result<Option<Vec<u8>>, MurmureError> {
        if self.session.map(Session::peer) == Some(peer) {
            return Ok(None);
        }
        let Some((name, now)) = &self.created else {
            self.session = Some(Session::adopt(&self.doc, peer, entry)?);
            return Ok(None);
        };
        let doc = LoroDoc::new();
        let mut session = Session::adopt(&doc, peer, entry)?;
        write_created(&doc, self.space_id, name, *now)?;
        let first = session.export(&doc)?;
        self.doc = doc;
        self.session = Some(session);
        self.created = None;
        Ok(Some(first))
    }

    /// This space under a fresh PeerID, as the Sync worker would issue it.
    #[cfg(test)]
    pub(crate) fn identify(mut self) -> Self {
        let peer = crate::session::next_peer();
        self.adopt_session(peer, peer.0.to_le_bytes().to_vec())
            .expect("adopt");
        self
    }

    /// Whether this session may write the document: its PeerID landed.
    pub fn identified(&self) -> bool {
        self.session.is_some()
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
    /// [`MurmureError::NameTooLong`] for a name past its cap.
    /// [`MurmureError::Crdt`] when Loro refuses a write or the export.
    pub fn put_category(
        &mut self,
        category_id: CategoryId,
        name: &str,
        position: u32,
    ) -> Result<Vec<u8>, MurmureError> {
        crate::channel::fitting(Some(name))?;
        self.writable()?;
        let entry = self
            .doc
            .get_map(CATEGORIES)
            .ensure_mergeable_map(&category_id.to_string())?;
        entry.insert(CATEGORY_NAME, name)?;
        entry.insert(CATEGORY_POSITION, i64::from(position))?;
        self.export()
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
        self.writable()?;
        let entry = self
            .doc
            .get_map(CATALOGUE)
            .ensure_mergeable_map(&channel_id.to_string())?;
        match channel.name() {
            Some(name) => entry.insert(ENTRY_NAME, name)?,
            None => clear(&entry, ENTRY_NAME)?,
        }
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
        self.export()
    }

    /// Remove the entry of a deleted channel from the catalogue, and return
    /// the delta (`MURMURE-247`). `None` when the catalogue does not list it,
    /// so nothing is written.
    ///
    /// # Errors
    /// [`MurmureError::Crdt`] when Loro refuses the delete or the export.
    pub fn unlist_channel(
        &mut self,
        channel_id: ChannelId,
    ) -> Result<Option<Vec<u8>>, MurmureError> {
        let catalogue = self.doc.get_map(CATALOGUE);
        let key = channel_id.to_string();
        if catalogue.get(&key).is_none() {
            return Ok(None);
        }
        self.writable()?;
        catalogue.delete(&key)?;
        self.export().map(Some)
    }

    /// Replace the sealed key of a listed channel after its rotation, and
    /// return the delta (`MURMURE-044`). The key is the one thing written:
    /// the name, the topic, the category and the place stay (`MURMURE-045`).
    /// `None` when the catalogue does not list it, or holds that key already.
    ///
    /// # Errors
    /// [`MurmureError::Crdt`] when Loro refuses the write or the export.
    pub fn reseal_channel(
        &mut self,
        channel_id: ChannelId,
        key: &SealedChannelKey,
    ) -> Result<Option<Vec<u8>>, MurmureError> {
        let Some(entry) = child_map(&self.doc.get_map(CATALOGUE), &channel_id.to_string()) else {
            return Ok(None);
        };
        if self.sealed_key(channel_id).as_ref() == Ok(key) {
            return Ok(None);
        }
        self.writable()?;
        entry.insert(ENTRY_KEY, key.as_bytes().to_vec())?;
        self.export().map(Some)
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

    /// Refuse a write before the PeerID landed, so nothing is written under
    /// another (`AUTHOR-029`).
    ///
    /// # Errors
    /// [`MurmureError::NoSession`] before the Sync worker issued it.
    fn writable(&self) -> Result<(), MurmureError> {
        if self.session.is_none() {
            return Err(MurmureError::NoSession(self.document_id()));
        }
        Ok(())
    }

    /// Commit the pending writes, and export what this session wrote since
    /// its last export.
    fn export(&mut self) -> Result<Vec<u8>, MurmureError> {
        let document_id = self.document_id();
        let session = self
            .session
            .as_mut()
            .ok_or(MurmureError::NoSession(document_id))?;
        session.export(&self.doc)
    }

    /// The document itself, for the shape belt.
    #[cfg(test)]
    pub(crate) fn doc(&self) -> &LoroDoc {
        &self.doc
    }
}

/// Write the content of a space created here into `doc`, and commit it.
///
/// # Errors
/// [`MurmureError::Crdt`] when Loro refuses a write.
fn write_created(
    doc: &LoroDoc,
    space_id: SpaceId,
    name: &str,
    now: Timestamp,
) -> Result<(), MurmureError> {
    schema::write_space_header(doc, space_id, now)?;
    if !name.is_empty() {
        doc.get_map(SPACE).insert(SPACE_NAME, name)?;
    }
    doc.commit();
    Ok(())
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
