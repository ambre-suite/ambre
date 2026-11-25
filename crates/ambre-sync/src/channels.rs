use super::*;
use ambre_bus::{
    ChannelDocument, ChannelDocumentKind, ChannelName, DirectMember, DocumentActivity,
    MemberChannel, SealedChannelKey,
};
use ambre_core::{DocType, FixedBytes};

use crate::error::{Part, SyncError};
use crate::http::Reported as _;

/// What an activity fetch does before it asks the relay.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ActivityFetch {
    /// Ask for the activity alone.
    Plain,
    /// Move every read cursor to its last number first.
    MarkAllReadFirst,
}

impl SyncState {
    // -- Channels --------------------------------------------------------------

    /// List the documents of a channel, so the Logic thread can open them.
    ///
    /// A channel travels as its documents (`SRV-102`): this is its only
    /// request of its own. The relay answers the same refusal to an unknown
    /// channel and to a caller who is no member of it (`SRV-047`).
    /// `listed_in` names the space whose catalogue lists the channel; the
    /// listing answer then records which log admits a self-join (`SYNC-106`).
    pub(crate) fn handle_open_channel(
        &self,
        channel_id: Id<Channel>,
        listed_in: Option<Id<Space>>,
    ) {
        let Some(tx) = self.event_tx.clone() else {
            // No event bus, no HTTP: say so instead of leaving the view waiting.
            self.send_to_logic(LogicEvent::ChannelOpenFailed {
                channel_id,
                failure: ApiFailure::Local(ambre_core::LocalFailure::NoRelay),
            });
            return;
        };
        let api_base = self.api_base.clone();
        let url_scheme = self.url_scheme;
        let auth = self.auth_context();
        crate::spawn(async move {
            let url = crate::http::Url::<crate::http::ApiRoot>::of(url_scheme, &api_base)
                .route(format!("/channels/{channel_id}/documents"));
            let result = http::get_json_auth::<http::ChannelDocumentsResponse>(&url, auth)
                .await
                .map(|resp| channel_documents(&resp.documents));
            let _ = tx.send(SyncEvent::ChannelListed {
                channel_id,
                listed_in,
                result,
            });
        });
    }

    /// The answer of [`handle_open_channel`](Self::handle_open_channel), on
    /// the Sync loop.
    pub(crate) fn channel_listed(
        &mut self,
        channel_id: Id<Channel>,
        listed_in: Option<Id<Space>>,
        result: Result<Vec<ChannelDocument>, http::HttpError>,
    ) {
        match result {
            Ok(documents) => {
                // A mark the Logic thread sent while the listing was in
                // flight outlives the open that asked it (`SYNC-230`).
                let listed_in = self.listed_channels.get(&channel_id).copied().or(listed_in);
                // The channel's members live in meta's log. Mark it as one
                // that admits a self-join exactly when a catalogue lists the
                // channel, and forget the mark when it no longer does.
                let metas: Vec<Id<Document>> = documents
                    .iter()
                    .filter(|d| d.kind == ChannelDocumentKind::Meta)
                    .map(|d| d.document_id)
                    .collect();
                for meta in metas {
                    self.listing_metas.insert(channel_id, meta);
                    match listed_in {
                        Some(space_id) => self.self_join_logs.insert(meta, space_id),
                        None => self.self_join_logs.remove(&meta),
                    };
                }
                self.send_to_logic(LogicEvent::ChannelDocuments {
                    channel_id,
                    documents,
                });
            }
            Err(error) if error.is(ambre_core::RefusalCode::Forbidden) => {
                // No member of the channel any more: a leave or a delete
                // whose answer was lost, or one from another client.
                crate::log::info!("Sync: channel {channel_id} refused its listing");
                self.send_to_logic(LogicEvent::ChannelRefused { channel_id });
            }
            Err(error) => {
                crate::log::warning!("Sync: listing channel {channel_id} failed: {error}");
                self.send_to_logic(LogicEvent::ChannelOpenFailed {
                    channel_id,
                    failure: error.failure(),
                });
            }
        }
    }

    /// The catalogue of `space_id` lists `channel_id`, which opened with no
    /// space or under another. The log of its `meta` admits a self-join from
    /// now on, and each open document of the channel walks it again
    /// (`SYNC-230`). A listing still in flight marks the log at its answer.
    pub(crate) fn mark_channel_listed(&mut self, channel_id: Id<Channel>, space_id: Id<Space>) {
        if self.listed_channels.insert(channel_id, space_id) == Some(space_id) {
            return;
        }
        let Some(meta) = self.listing_metas.get(&channel_id).copied() else {
            return;
        };
        self.self_join_logs.insert(meta, space_id);
        self.walk_log_again(meta);
    }

    /// The walk of the log of `document_id` set a new word. When that
    /// document is a space, each channel log it lists heeds the word, so
    /// each walks again (`SYNC-230`).
    pub(crate) fn space_word_changed(&mut self, document_id: Id<Document>) {
        let metas: Vec<Id<Document>> = self
            .self_join_logs
            .iter()
            .filter(|(_, space_id)| space_id.document_id() == document_id)
            .map(|(meta, _)| *meta)
            .collect();
        for meta in metas {
            self.walk_log_again(meta);
        }
    }

    /// A walk found the log of `document_id` rooted outside the space that
    /// lists it (audit `N16`). An earlier walk, run before this client held
    /// the space's word, accepted its key and its deltas. The document now
    /// takes no delta, and the deltas it held sealed are dropped. The Logic
    /// thread learns the open failed, so it ends the channel (`SYNC-231`).
    pub(crate) fn fail_rooted_outside(&mut self, document_id: Id<Document>) {
        let Some(doc_state) = self.documents.get_mut(&document_id) else {
            return;
        };
        doc_state.writers = crate::membership_log::Writers::granted(Default::default());
        doc_state.unvouched.clear();
        doc_state.refused.clear();
        doc_state.gate.end();
        crate::log::warning!("Sync: the log of {document_id:?} starts outside its space");
        self.send_to_logic(LogicEvent::DocumentOpenFailed {
            document_id,
            cause: ambre_bus::OpenFailure::RootOutsideSpace,
        });
    }

    /// Lists the members of each open document whose log is that of `meta`,
    /// so the walk runs again under the rule `log_rule` now gives. Each delta
    /// a walk refused as no writer's waits for that answer, which decides it
    /// again (`SYNC-211`, `SYNC-230`).
    fn walk_log_again(&mut self, meta: Id<Document>) {
        let documents: Vec<Id<Document>> = self
            .documents
            .keys()
            .copied()
            .filter(|document_id| self.log_document(*document_id) == meta)
            .collect();
        for document_id in documents {
            let Some(doc_state) = self.documents.get_mut(&document_id) else {
                continue;
            };
            let floor = doc_state.directory_asked.latest();
            let mut refused = std::mem::take(&mut doc_state.refused);
            for held in &mut refused {
                held.seen = floor;
            }
            doc_state.unvouched.extend(refused);
            crate::trim_held(&mut doc_state.unvouched);
            self.handle_list_document_members(None, document_id);
            // Until the answer, the document lands no delta and seals no
            // frame: the walk may find the log rooted outside (`SYNC-232`).
            if let Some(doc_state) = self.documents.get_mut(&document_id) {
                let asked = doc_state.directory_asked.latest();
                doc_state.gate.walk(floor, asked);
            }
        }
    }

    /// The answer to the listing `asked` walked the log of `document_id`
    /// under the rule of its space, and found it rooted in that space. A walk
    /// it answers lets the deltas it held settle. It seals the frames it
    /// held, in the order they were written. It fetches the snapshots it
    /// deferred again (`SYNC-232`, `SYNC-234`).
    pub(crate) fn release_walk(
        &mut self,
        document_id: Id<Document>,
        asked: ambre_core::Seq<crate::generation::Listing>,
    ) {
        let Some(released) = self
            .documents
            .get_mut(&document_id)
            .and_then(|doc_state| doc_state.gate.release(asked))
        else {
            return;
        };
        self.settle_unvouched(document_id, asked);
        // The refused frames leave first, at their own clocks, then the edits
        // the walk held, at the next clocks (`SYNC-129`, `SYNC-234`).
        self.resend_refused(document_id, released.resend, released.outbound);
        for (snapshot_id, covered_seq) in released.snapshots {
            self.fetch_announced_snapshot(document_id, snapshot_id, covered_seq);
        }
    }

    /// The document whose log and links vouch for the key of `document_id`:
    /// the `meta` of the channel that groups it, or the document itself
    /// (`SYNC-229`). The relay serves a document of a channel the rows and
    /// the links of its `meta`, so no other log names its key.
    pub(crate) fn log_document(&self, document_id: Id<Document>) -> Id<Document> {
        self.channel_metas
            .get(&document_id)
            .map_or(document_id, |meta| meta.document_id())
    }

    /// The documents open here whose log is the log of `meta`, `meta` aside:
    /// the open documents of the channel whose `meta` it is. A revoke on that
    /// log caps its member in each of them (`SYNC-311`).
    pub(crate) fn open_channel_documents(&self, meta: Id<Document>) -> Vec<Id<Document>> {
        self.channel_metas
            .iter()
            .filter(|(document_id, of)| {
                of.document_id() == meta
                    && **document_id != meta
                    && self.documents.contains_key(document_id)
            })
            .map(|(document_id, _)| *document_id)
            .collect()
    }

    /// The rule a walk of `document_id`'s log applies, the log of its `meta`
    /// for a document of a channel. A self-join is valid in the log of a
    /// channel a catalogue lists, and nowhere else (`SYNC-106`). That log
    /// also heeds the word of its space, as the walk of the open space's log
    /// last said it.
    pub(crate) fn log_rule(&self, document_id: Id<Document>) -> membership_log::LogRule {
        match self.self_join_logs.get(&self.log_document(document_id)) {
            Some(space_id) => membership_log::LogRule {
                self_join: membership_log::public_channel_self_join,
                space: self
                    .documents
                    .get(&space_id.document_id())
                    .map(|space| space.word.clone()),
            },
            None => membership_log::LogRule::document(),
        }
    }

    /// What this client holds out of band to root the log of an open of
    /// `document_id`, a document of `doc_type`, on. The roots of the session
    /// count for a Murmure open alone (`SYNC-235`): a Prose, Trame or Toile
    /// open stands on its own link (`SYNC-205`, `SYNC-210`).
    pub(crate) fn roots(
        &self,
        document_id: Id<Document>,
        doc_type: DocType,
    ) -> crate::key_vouch::Roots {
        crate::key_vouch::Roots {
            pin: self.pinned_inviters.get(&document_id).copied(),
            session: match doc_type {
                DocType::Murmure => self.murmure_roots.iter().copied().collect(),
                // The settings log is rooted on the user's own key, which `signer`
                // names (`SETTINGS-001`).
                DocType::Trame | DocType::Toile | DocType::Prose | DocType::Settings => Vec::new(),
            },
            signer: self.author_pub_key(),
            catalogue: self.catalogue_pin(document_id),
        }
    }

    /// The channel key the catalogue pins for the channel whose `meta` logs
    /// `document_id`, if a catalogue lists it (`SYNC-299`).
    fn catalogue_pin(&self, document_id: Id<Document>) -> Option<crate::key_vouch::CataloguePin> {
        let meta = self.log_document(document_id);
        self.listing_metas
            .iter()
            .find(|(_, listed)| **listed == meta)
            .and_then(|(channel_id, _)| self.catalogue_pins.get(channel_id))
            .copied()
    }

    /// Pins the channel key that `listed`, the catalogue entry of `channel_id`
    /// in its space, holds (`SYNC-299`). The entry unseals under a key of the
    /// space. One that does not unseal pins no key, so every open of the
    /// channel fails closed. A channel no catalogue lists drops its pin.
    pub(crate) fn pin_catalogue(
        &mut self,
        channel_id: Id<Channel>,
        listed: Option<(Id<Space>, &SealedChannelKey)>,
    ) {
        let Some((space_id, sealed)) = listed else {
            self.catalogue_pins.remove(&channel_id);
            return;
        };
        let pin = self
            .space_keys(space_id)
            .and_then(|(keys, _)| unseal_channel_key(channel_id, sealed, &keys))
            .map_or_else(
                |error| {
                    crate::log::warning!(
                        "Sync: the catalogue entry of channel {channel_id} does not unseal: {error}"
                    );
                    crate::key_vouch::CataloguePin::Unreadable
                },
                |(key, _)| crate::key_vouch::CataloguePin::Key(key.id()),
            );
        self.catalogue_pins.insert(channel_id, pin);
    }

    /// The answer to a channel creation or a self-join, on the Sync loop.
    pub(crate) fn channel_answered(
        &self,
        channel_id: Id<Channel>,
        result: Result<LogicEvent, http::HttpError>,
    ) {
        match result {
            Ok(event) => self.send_to_logic(event),
            Err(error) => {
                crate::log::warning!("Sync: channel {channel_id} request failed: {error}");
                self.send_to_logic(LogicEvent::ChannelRequestFailed {
                    channel_id,
                    failure: error.failure(),
                });
            }
        }
    }

    /// Whether the document of `space_id` is open here and on its way to the
    /// relay, which does not hold it yet.
    pub(crate) fn space_on_its_way(&self, space_id: Id<Space>) -> bool {
        self.documents
            .get(&space_id.document_id())
            .is_some_and(|space| space.existence == crate::Existence::LocalOnly)
    }

    /// Run the channel creations that waited on `document_id`, now on the
    /// relay (`SYNC-189`).
    pub(crate) fn release_creations(&mut self, document_id: Id<Document>) {
        for command in self.held_creations.remove(&document_id).unwrap_or_default() {
            self.handle_command(command);
        }
    }

    /// Fail the channel creations that waited on `document_id`, whose own
    /// creation failed: the view learns each one (`SYNC-189`).
    pub(crate) fn refuse_creations(&mut self, document_id: Id<Document>) {
        for command in self.held_creations.remove(&document_id).unwrap_or_default() {
            if let SyncCommand::CreateChannel { channel_id, .. } = command {
                self.send_to_logic(LogicEvent::ChannelRequestFailed {
                    channel_id,
                    failure: ApiFailure::Local(ambre_core::LocalFailure::NotOnRelay),
                });
            }
        }
    }

    /// The keys of an open space that the relay holds, and this user's
    /// exchange key: what a channel creation and a self-join both need. A
    /// catalogue entry sealed before a rotation of the space opens under the
    /// older key it names.
    pub(crate) fn space_keys(
        &self,
        space_id: Id<Space>,
    ) -> Result<(crate::chain::Keyring, StaticSecret), SyncError> {
        let space = self
            .documents
            .get(&space_id.document_id())
            .ok_or(SyncError::NotOpen)?;
        if space.existence == crate::Existence::LocalOnly {
            return Err(SyncError::NotOnRelay);
        }
        let exchange_key = self.exchange_key.clone().ok_or(SyncError::SessionLocked)?;
        Ok((space.keyring(), exchange_key))
    }

    /// Create a channel of a space on the relay (the Murmure spaces and
    /// channels specs). One fresh key opens both documents, wrapped for this
    /// user alone. A public channel's key also goes back to the Logic thread
    /// sealed under the space key, for the catalogue. A private one is listed
    /// nowhere, so its key goes nowhere else. No step leaves a channel the
    /// relay holds half made: each document is created by its own id, so a
    /// retry finds it.
    pub(crate) fn handle_create_channel(
        &self,
        channel_id: Id<Channel>,
        space_id: Id<Space>,
        (meta, segment): (Id<Meta>, Id<Segment>),
        visibility: ambre_core::Visibility,
        name: Option<ambre_core::Validated<ambre_core::Name>>,
    ) {
        let prepared = self
            .event_tx
            .clone()
            .ok_or(SyncError::NoRelay)
            .and_then(|tx| {
                // A private channel is listed nowhere, so it needs no space key.
                let space_key = if visibility == ambre_core::Visibility::Public {
                    Some(self.space_keys(space_id)?.0.current)
                } else {
                    None
                };
                let exchange_key = self.exchange_key.clone().ok_or(SyncError::SessionLocked)?;
                let author = self.author().ok_or(SyncError::SessionLocked)?;
                // Both documents open a log under the one channel key, and the
                // channel's members live in meta's. Each creator copy seals its
                // own document's anchor, and the catalogue seals meta's, so a
                // self-joiner replays from it (`SYNC-104`).
                let channel_key = DocumentKey::generate();
                // The name rides beside meta alone, so it shows before the
                // channel opens (`SYNC-171`).
                let name_enc = name
                    .as_ref()
                    .map(|name| self.seal_channel_name(meta, &channel_key, name))
                    .transpose()?;
                let documents = [
                    (meta.document_id(), name_enc),
                    (segment.document_id(), None),
                ]
                .into_iter()
                .map(|(document_id, name_enc)| {
                    let (doc_key_enc, entry) = crate::sharing::creation(
                        document_id,
                        &channel_key,
                        &exchange_key,
                        &author,
                    )?;
                    Ok((document_id, doc_key_enc, entry, name_enc))
                })
                .collect::<Result<Vec<_>, SyncError>>()?;
                let sealed = match space_key {
                    Some(space_key) => {
                        let meta_anchor = documents
                            .first()
                            .and_then(|(_, _, entry, _)| digest_of(entry))
                            .ok_or(SyncError::NoDigest)?;
                        Some(seal_channel_key(
                            channel_id,
                            &channel_key,
                            &space_key,
                            meta_anchor,
                        )?)
                    }
                    None => None,
                };
                Ok((tx, sealed, documents))
            });
        let (tx, sealed, documents) = match prepared {
            Ok(prepared) => prepared,
            Err(message) => return self.channel_answered(channel_id, Err(message.into())),
        };
        let api_base = self.api_base.clone();
        let url_scheme = self.url_scheme;
        let auth = self.auth_context();
        crate::spawn(async move {
            let base = crate::http::Url::<crate::http::ApiRoot>::of(url_scheme, &api_base);
            let result = async {
                for (document_id, doc_key_enc, entry, name_enc) in documents {
                    let body = http::CreateDocumentRequest {
                        doc_type: DocType::Murmure,
                        name_enc,
                        doc_key_enc,
                        document_id: Some(document_id),
                        entry: Some(entry),
                        space: ambre_core::DocumentScope::Personal,
                    };
                    http::post_json_auth::<_, http::CreateDocumentResponse>(
                        &base.route("/documents"),
                        &body,
                        auth.clone(),
                    )
                    .await?;
                }
                let body = http::CreateChannelRequest {
                    visibility,
                    channel_id,
                    space_id,
                    meta_id: meta,
                    segment_id: segment,
                };
                http::post_json_auth_ok(&base.route("/channels"), &body, auth).await
            }
            .await
            .map(|()| sealed);
            let _ = tx.send(SyncEvent::ChannelCreated { channel_id, result });
        });
    }

    /// Seal the name just written into `meta` beside it on the relay, under
    /// the key meta holds now (`SYNC-171`). No name seals an empty one. A
    /// failure is logged: the hint stays stale until the next name.
    pub(crate) fn handle_name_channel(
        &self,
        meta: Id<Meta>,
        name: Option<ambre_core::Validated<ambre_core::Name>>,
    ) {
        let name_enc = match self.sealed_channel_name(meta, &name.unwrap_or_default()) {
            Ok(name_enc) => name_enc,
            Err(message) => {
                crate::log::warning!("Sync: the name of {meta} stays unsealed: {message}");
                return;
            }
        };
        let Some(_tx) = self.event_tx.as_ref() else {
            crate::log::warning!(
                "Sync: no relay on this transport, the name of {meta} stays local"
            );
            return;
        };
        let url = crate::http::Url::<crate::http::ApiRoot>::of(self.url_scheme, &self.api_base)
            .route(format!("/documents/{}", meta.document_id()));
        let auth = self.auth_context();
        crate::spawn(async move {
            let body = http::RenameDocumentRequest { name_enc };
            if let Err(message) = http::patch_json_auth(&url, &body, auth).await {
                crate::log::warning!("Sync: the name of {meta} did not reach the relay: {message}");
            }
        });
    }

    /// `name` sealed for the relay beside `meta`, under the key meta holds
    /// now, once the walk of its log settled that key (`SYNC-234`). A client
    /// that may not write meta seals nothing: the relay refuses its write,
    /// and a stale hint waits for a writer's open (`SYNC-318`).
    fn sealed_channel_name(
        &self,
        meta: Id<Meta>,
        name: &ambre_core::Validated<ambre_core::Name>,
    ) -> Result<ambre_core::Encoded<ambre_core::SealedName>, SyncError> {
        let held = self
            .documents
            .get(&meta.document_id())
            .ok_or(SyncError::NotOpen)?;
        if !held.role.can_write_document() {
            return Err(SyncError::NameRefused);
        }
        self.seal_channel_name(meta, held.sealing_key()?, name)
    }

    /// `name` sealed for the relay beside `meta`, under `key`.
    fn seal_channel_name(
        &self,
        meta: Id<Meta>,
        key: &DocumentKey,
        name: &ambre_core::Validated<ambre_core::Name>,
    ) -> Result<ambre_core::Encoded<ambre_core::SealedName>, SyncError> {
        let signing_key = self.signing_key.as_ref().ok_or(SyncError::SessionLocked)?;
        Ok(Self::seal_document_name(
            meta.document_id(),
            key,
            signing_key,
            name,
        )?)
    }

    /// Join a public channel alone. Unseal the catalogue's key with the space
    /// key, wrap it for this user alone, and ask the relay for the member row.
    /// The key never leaves this worker. The request carries a `SelfJoin`
    /// entry signed at the end of meta's log, which the relay's join head
    /// names with the role the join grants.
    pub(crate) fn handle_join_channel(
        &self,
        channel_id: Id<Channel>,
        space_id: Id<Space>,
        sealed: &SealedChannelKey,
    ) {
        let prepared = self
            .event_tx
            .clone()
            .ok_or(SyncError::NoRelay)
            .and_then(|tx| {
                let (space_keys, exchange_key) = self.space_keys(space_id)?;
                let author = self.author().ok_or(SyncError::SessionLocked)?;
                let unsealed = unseal_channel_key(channel_id, sealed, &space_keys)?;
                Ok((tx, unsealed, exchange_key, author))
            });
        let (tx, unsealed, exchange_key, author) = match prepared {
            Ok(prepared) => prepared,
            Err(message) => return self.channel_answered(channel_id, Err(message.into())),
        };
        let api_base = self.api_base.clone();
        let url_scheme = self.url_scheme;
        let auth = self.auth_context();
        crate::spawn(async move {
            let url = crate::http::Url::<crate::http::ApiRoot>::of(url_scheme, &api_base)
                .route(format!("/channels/{channel_id}/join"));
            let result = async {
                let head =
                    http::get_json_auth::<http::JoinHeadResponse>(&url, auth.clone()).await?;
                // The copy lands on the rows of the meta the join head names,
                // and binds it (audit `P59`). The key is this user's own, so a
                // meta the relay names wrongly opens nothing for anyone else.
                let doc_key_enc =
                    wrap_join_key(head.meta_id.document_id(), &unsealed, &exchange_key)?;
                let exchange = crate::sharing::own_exchange_key(&exchange_key);
                let entry = self_join_entry(&author, exchange, &head, unsealed.0.id())?;
                let body = http::JoinChannelRequest { doc_key_enc, entry };
                http::post_json_auth_ok(&url, &body, auth).await?;
                Ok::<_, http::HttpError>(())
            }
            .await;
            let _ = tx.send(SyncEvent::ChannelJoined { channel_id, result });
        });
    }

    /// Close the active segment `closing` of an open channel into
    /// `successor` (the Murmure segments spec). The successor opens under
    /// the current key of `meta`, whose members are the channel's.
    pub(crate) fn handle_close_segment(
        &self,
        channel_id: Id<Channel>,
        meta: Id<Meta>,
        closing: Id<Segment>,
        successor: Id<Segment>,
    ) {
        let (tx, meta_key, exchange_key, author) = match self.channel_creation(meta) {
            Ok(prepared) => prepared,
            Err(error) => {
                crate::log::warning!("Sync: a channel step of {channel_id} failed: {error}");
                return self.send_to_logic(LogicEvent::SegmentCloseFailed {
                    channel_id,
                    closing,
                    failure: error.failure(),
                });
            }
        };
        let params = crate::sharing::SegmentCloseParams {
            channel_id,
            closing,
            successor,
            meta_key,
            author,
            url_scheme: self.url_scheme,
            api_base: self.api_base.clone(),
            exchange_key,
        };
        let http = crate::sharing::GlooHttp {
            auth: self.auth_context(),
        };
        crate::spawn(async move {
            let result = crate::sharing::close_segment_task(&http, params).await;
            let _ = tx.send(SyncEvent::SegmentClosed {
                channel_id,
                closing,
                result,
            });
        });
    }

    /// What a new document of the channel of `meta` is created with: the
    /// current key of `meta`, whose members are the channel's, and this
    /// user.
    fn channel_creation(
        &self,
        meta: Id<Meta>,
    ) -> Result<
        (
            ambre_bus::conduit::UnboundedSender<SyncEvent>,
            DocumentKey,
            StaticSecret,
            crate::sharing::Author,
        ),
        SyncError,
    > {
        let tx = self.event_tx.clone().ok_or(SyncError::NoRelay)?;
        // A document of the channel opens under the key the walk of the log
        // of `meta` settled (`SYNC-234`).
        let meta_key = self
            .documents
            .get(&meta.document_id())
            .ok_or(SyncError::NotOpen)?
            .sealing_key()?
            .clone();
        let exchange_key = self.exchange_key.clone().ok_or(SyncError::SessionLocked)?;
        let author = self.author().ok_or(SyncError::SessionLocked)?;
        Ok((tx, meta_key, exchange_key, author))
    }

    /// Create the amendments document of the locked `segment`, then link it
    /// (`SRV-201`). It opens under the current key of `meta`, as the
    /// segment's successor did.
    pub(crate) fn handle_create_amendments(
        &self,
        channel_id: Id<Channel>,
        meta: Id<Meta>,
        segment: Id<Segment>,
        amendments: Id<Document>,
    ) {
        let (tx, meta_key, exchange_key, author) = match self.channel_creation(meta) {
            Ok(prepared) => prepared,
            Err(error) => {
                crate::log::warning!("Sync: a channel step of {channel_id} failed: {error}");
                return self.send_to_logic(LogicEvent::AmendmentsLinkFailed {
                    channel_id,
                    segment,
                    amendments,
                    failure: error.failure(),
                });
            }
        };
        let params = crate::sharing::AmendmentsParams {
            channel_id,
            segment,
            amendments,
            meta_key,
            author,
            url_scheme: self.url_scheme,
            api_base: self.api_base.clone(),
            exchange_key,
        };
        let http = crate::sharing::GlooHttp {
            auth: self.auth_context(),
        };
        crate::spawn(async move {
            let result = crate::sharing::amendments_task(&http, params).await;
            let _ = tx.send(SyncEvent::AmendmentsLinked {
                channel_id,
                segment,
                amendments,
                result,
            });
        });
    }

    /// Create the thread document of `root`, then register it (`SRV-213`).
    /// It opens under the current key of `meta`, as amendments do.
    pub(crate) fn handle_create_thread(
        &self,
        channel_id: Id<Channel>,
        meta: Id<Meta>,
        segment: Id<Segment>,
        root: Id<Message>,
        thread: Id<Document>,
    ) {
        let (tx, meta_key, exchange_key, author) = match self.channel_creation(meta) {
            Ok(prepared) => prepared,
            Err(error) => {
                crate::log::warning!("Sync: a channel step of {channel_id} failed: {error}");
                return self.send_to_logic(LogicEvent::ThreadLinkFailed {
                    channel_id,
                    segment,
                    root,
                    thread,
                    failure: error.failure(),
                });
            }
        };
        let params = crate::sharing::ThreadParams {
            channel_id,
            root,
            thread,
            meta_key,
            author,
            url_scheme: self.url_scheme,
            api_base: self.api_base.clone(),
            exchange_key,
        };
        let http = crate::sharing::GlooHttp {
            auth: self.auth_context(),
        };
        crate::spawn(async move {
            let result = crate::sharing::thread_task(&http, params).await;
            let _ = tx.send(SyncEvent::ThreadLinked {
                channel_id,
                segment,
                root,
                thread,
                result,
            });
        });
    }

    /// Ask the relay to delete a channel with every document it groups
    /// (`SRV-142`). The documents stay open here until the Logic thread hears
    /// the answer and closes them, so a refusal leaves the channel whole.
    pub(crate) fn handle_delete_channel(&self, channel_id: Id<Channel>) {
        let Some(tx) = self.event_tx.clone() else {
            return self.channel_answered(channel_id, Err(SyncError::NoRelay.into()));
        };
        let api_base = self.api_base.clone();
        let url_scheme = self.url_scheme;
        let auth = self.auth_context();
        crate::spawn(async move {
            let url = crate::http::Url::<crate::http::ApiRoot>::of(url_scheme, &api_base)
                .route(format!("/channels/{channel_id}"));
            let result = http::delete_auth(&url, auth).await;
            let _ = tx.send(SyncEvent::ChannelDeleted { channel_id, result });
        });
    }

    /// Ask the relay to remove this user from a channel. The documents stay
    /// open here until the Logic thread hears the answer and closes them. A
    /// refusal leaves the channel whole, as with a delete.
    ///
    /// The census opens each document of the channel not open here first,
    /// so the departure caps this user in each one (`SYNC-312`). `named`
    /// lists the documents the sealed `meta` names (`SYNC-320`), and
    /// `threads` the threads they name (`SYNC-321`).
    pub(crate) fn handle_leave_channel(
        &mut self,
        channel_id: Id<Channel>,
        named: Vec<Id<Document>>,
        threads: Vec<Id<Document>>,
    ) {
        self.begin_removal(
            crate::census::Removal::LeaveChannel { channel_id },
            Some(crate::census::Named {
                documents: named,
                threads,
            }),
        );
    }

    /// The departure of [`handle_leave_channel`](Self::handle_leave_channel),
    /// from the documents open here.
    pub(crate) fn leave_channel_now(&self, channel_id: Id<Channel>) {
        let Some(tx) = self.event_tx.clone() else {
            return self.channel_answered(channel_id, Err(SyncError::NoRelay.into()));
        };
        let (Some(author), Some(exchange_key)) = (self.author(), self.exchange_key.clone()) else {
            return self.channel_answered(
                channel_id,
                Err(crate::http::HttpError::Local(
                    ambre_core::LocalFailure::NotSignedIn,
                )),
            );
        };
        // The departure caps this user in each document of the channel open
        // here, at the last clock they wrote in it (`SYNC-311`).
        let key = author.public();
        let written = self
            .documents
            .iter()
            .map(|(document_id, doc)| {
                (
                    *document_id,
                    self.log_document(*document_id),
                    doc.own_last_clock(&key),
                )
            })
            .collect();
        let params = crate::sharing::ChannelLeaveParams {
            channel_id,
            author,
            url_scheme: self.url_scheme,
            api_base: self.api_base.clone(),
            exchange_key,
            written,
        };
        let http = crate::sharing::GlooHttp {
            auth: self.auth_context(),
        };
        crate::spawn(async move {
            let result = crate::sharing::leave_channel_task(&http, params).await;
            let _ = tx.send(SyncEvent::ChannelLeft { channel_id, result });
        });
    }

    /// Ask the relay for the direct message of this user and `members`
    /// (the Murmure channels spec). The relay creates it, or answers the one
    /// that already holds exactly them. One fresh key opens it, wrapped for
    /// every member under the exchange key the view pinned. The relay's
    /// copy of each key is fetched again first, and a key that changed
    /// fails the request. A relay that swaps a key gets no wrap under it.
    /// The same answer gives each member's signing key, which their invite
    /// in the log names.
    pub(crate) fn handle_create_direct_message(
        &self,
        channel_id: Id<Channel>,
        ids: (Id<Meta>, Id<Segment>),
        members: Vec<DirectMember>,
    ) {
        // The cap counts the creator, whom `members` leaves out. It holds
        // before any request leaves (`SYNC-180`).
        let within_budget = members.len() < ambre_protocol::membership::DIRECT_MESSAGE_MAX_MEMBERS;
        let prepared = within_budget
            .then_some(())
            .ok_or(SyncError::TooManyMembers)
            .and_then(|()| self.event_tx.clone().ok_or(SyncError::NoRelay))
            .and_then(|tx| {
                let exchange_key = self.exchange_key.clone().ok_or(SyncError::SessionLocked)?;
                let author = self.author().ok_or(SyncError::SessionLocked)?;
                Ok((tx, exchange_key, author))
            });
        let (tx, exchange_key, author) = match prepared {
            Ok(prepared) => prepared,
            Err(message) => return self.channel_answered(channel_id, Err(message.into())),
        };
        // The keys this session showed for each member, whose fingerprint
        // the user compared before the request (audit `P44`).
        let shown: Vec<_> = members
            .iter()
            .map(|member| self.shown_identities.get(&member.user_id).copied())
            .collect();
        let api_base = self.api_base.clone();
        let url_scheme = self.url_scheme;
        let auth = self.auth_context();
        crate::spawn(async move {
            let base = crate::http::Url::<crate::http::ApiRoot>::of(url_scheme, &api_base);
            let result = async {
                let mut signing_keys = Vec::with_capacity(members.len());
                for (member, shown) in members.iter().zip(&shown) {
                    let url = base.route(format!("/users/{}/pubkeys", member.user_id));
                    let served =
                        http::get_json_auth::<http::PubkeysResponse>(&url, auth.clone()).await?;
                    check_pinned(member, &served, shown.as_ref())?;
                    signing_keys.push(served.signing_pubkey);
                }
                let invited: Vec<_> = members.iter().zip(signing_keys).collect();
                let body =
                    direct_message_request(&author, &exchange_key, channel_id, ids, &invited)?;
                let answer = http::post_json_auth::<_, http::DirectMessageResponse>(
                    &base.route("/channels/direct"),
                    &body,
                    auth,
                )
                .await?;
                Ok(answer.channel_id)
            }
            .await;
            let _ = tx.send(SyncEvent::DirectMessageOpened {
                requested: channel_id,
                result,
            });
        });
    }

    // -- Read cursors ------------------------------------------------------------

    /// The sequence number a read of `document_id` reaches: the last one this
    /// worker delivered to the Logic thread.
    ///
    /// `None` when the document is not open, holds nothing, or holds updates
    /// in quarantine. Those are sequenced and not shown, and a cursor moved
    /// past them would skip them. The cursor then stays where it was, which
    /// leaves the channel unread rather than read too far.
    pub(crate) fn read_up_to(&self, document_id: Id<Document>) -> Option<Seq<Relay>> {
        let doc = self.documents.get(&document_id)?;
        (doc.last_seq.get() > 0 && doc.quarantine_len().is_zero()).then_some(doc.last_seq)
    }

    /// Move the cursors of one channel to its latest message (the Murmure
    /// unread spec). Each live segment moves to what this worker delivered.
    /// Each final segment no longer changes, so it moves to the relay's last
    /// number.
    pub(crate) fn handle_mark_read(&mut self, live: &[Id<Segment>], finals: Vec<Id<Segment>>) {
        let cursors: Vec<(Id<Document>, Seq<Relay>)> = live
            .iter()
            .map(Id::<Segment>::document_id)
            .filter_map(|document_id| self.read_up_to(document_id).map(|seq| (document_id, seq)))
            .collect();
        if self.event_tx.is_none() {
            crate::log::warning!("Sync: no relay on this transport, the read cursors stay");
            return;
        }
        self.owe_cursors(cursors);
        if finals.is_empty() {
            return;
        }
        let url = crate::http::Url::<crate::http::ApiRoot>::of(self.url_scheme, &self.api_base)
            .route("/read-cursors/mark-read");
        let auth = self.auth_context();
        crate::spawn(async move {
            let body = http::MarkReadRequest {
                documents: Some(finals.iter().map(Id::<Segment>::document_id).collect()),
            };
            if let Err(e) = http::post_json_auth_ok(&url, &body, auth).await {
                crate::log::warning!("Sync: read cursors of the final segments not moved: {e}");
            }
        });
    }

    /// Move each listed cursor to the number Logic chose, back as well as
    /// forward: marking a message unread (the Murmure unread spec). No other
    /// cursor moves.
    pub(crate) fn handle_move_read_cursors(&mut self, cursors: Vec<(Id<Segment>, Seq<Relay>)>) {
        self.move_cursors(
            cursors
                .into_iter()
                .map(|(segment, seq)| (segment.document_id(), seq))
                .collect(),
        );
    }

    /// Move the cursor of one thread to what this worker delivered on it
    /// (the Murmure threads spec). A thread not open, empty or holding a
    /// quarantine keeps its cursor, so a reply is never read too far.
    pub(crate) fn handle_mark_thread_read(&mut self, thread: Id<Document>) {
        match self.read_up_to(thread) {
            Some(seq) => self.move_cursors(vec![(thread, seq)]),
            None => crate::log::warning!(
                "Sync: thread {thread} has nothing delivered, its cursor stays"
            ),
        }
    }

    fn move_cursors(&mut self, cursors: Vec<(Id<Document>, Seq<Relay>)>) {
        if self.event_tx.is_none() {
            crate::log::warning!("Sync: no relay on this transport, the read cursors stay");
            return;
        }
        self.owe_cursors(cursors);
    }

    /// Owe the relay `cursors`, and start the period that sends them unless
    /// one runs (`SYNC-269`).
    fn owe_cursors(&mut self, cursors: Vec<(Id<Document>, Seq<Relay>)>) {
        if self.owed_cursors.owe(cursors) == crate::cursors::Period::Idle {
            self.arm_event_timer(SyncEvent::CursorsDue, crate::cursors::CURSOR_PERIOD);
        }
    }

    /// The period of the cursors owed ended: move each, one request per
    /// document (`SYNC-269`).
    pub(crate) fn send_owed_cursors(&mut self) {
        let cursors = self.owed_cursors.take();
        if cursors.is_empty() {
            return;
        }
        let base = crate::http::Url::<crate::http::ApiRoot>::of(self.url_scheme, &self.api_base);
        let auth = self.auth_context();
        crate::spawn(async move {
            put_cursors(&base, cursors, auth).await;
        });
    }

    /// Move every cursor of the user to its last number (the Murmure unread
    /// spec). Then fetch the activity answer, so the sidebar clears from the
    /// relay's word.
    pub(crate) fn handle_mark_all_read(&self) {
        self.fetch_activity(ActivityFetch::MarkAllReadFirst);
    }

    /// Ask the relay which channel documents hold something new (`SRV-109`).
    pub(crate) fn handle_fetch_activity(&self) {
        self.fetch_activity(ActivityFetch::Plain);
    }

    fn fetch_activity(&self, fetch: ActivityFetch) {
        let Some(tx) = self.event_tx.clone() else {
            crate::log::warning!("Sync: no relay on this transport, no activity answer");
            return;
        };
        let api_base = self.api_base.clone();
        let url_scheme = self.url_scheme;
        let auth = self.auth_context();
        crate::spawn(async move {
            if fetch == ActivityFetch::MarkAllReadFirst {
                let url = crate::http::Url::<crate::http::ApiRoot>::of(url_scheme, &api_base)
                    .route("/read-cursors/mark-read");
                let body = http::MarkReadRequest { documents: None };
                if let Err(e) = http::post_json_auth_ok(&url, &body, auth.clone()).await {
                    crate::log::warning!("Sync: mark all read failed: {e}");
                }
            }
            let url = crate::http::Url::<crate::http::ApiRoot>::of(url_scheme, &api_base)
                .route("/activity");
            let result = http::get_json_auth::<http::ActivityResponse>(&url, auth)
                .await
                .map(|resp| activity_of(&resp.documents));
            let _ = tx.send(SyncEvent::ActivityFetched { result });
        });
    }

    /// The answer of [`fetch_activity`](Self::fetch_activity), on the Sync
    /// loop. A failure leaves the marks the view holds, and says so in the log.
    pub(crate) fn activity_fetched(&self, result: Result<Vec<DocumentActivity>, http::HttpError>) {
        match result {
            Ok(documents) => self.send_to_logic(LogicEvent::ChannelActivity { documents }),
            Err(message) => crate::log::warning!("Sync: activity answer failed: {message}"),
        }
    }

    /// List the channels the relay holds as public in a space (`SRV-189`),
    /// so a space admin can drop an entry whose channel is gone.
    pub(crate) fn handle_list_space_channels(&self, space_id: Id<Space>) {
        let Some(tx) = self.event_tx.clone() else {
            crate::log::warning!("Sync: no relay on this transport, no channels of a space");
            return;
        };
        let api_base = self.api_base.clone();
        let url_scheme = self.url_scheme;
        let auth = self.auth_context();
        crate::spawn(async move {
            let url = crate::http::Url::<crate::http::ApiRoot>::of(url_scheme, &api_base)
                .route(format!("/spaces/{space_id}/channels"));
            let result = http::get_json_auth::<http::SpaceChannelsResponse>(&url, auth)
                .await
                .map(|resp| resp.channels);
            let _ = tx.send(SyncEvent::SpaceChannelsListed { space_id, result });
        });
    }

    /// The answer of
    /// [`handle_list_space_channels`](Self::handle_list_space_channels), on
    /// the Sync loop. A failure answers nothing: the catalogue stays as it is.
    pub(crate) fn space_channels_listed(
        &self,
        space_id: Id<Space>,
        result: Result<Vec<Id<Channel>>, http::HttpError>,
    ) {
        match result {
            Ok(channels) => self.send_to_logic(LogicEvent::SpaceChannels { space_id, channels }),
            Err(message) => {
                crate::log::warning!(
                    "Sync: listing the channels of space {space_id} failed: {message}"
                );
            }
        }
    }

    /// List every channel this user is a member of (`SRV-191`), at start and
    /// on each share of a Murmure document. That share is how the relay tells
    /// a member a channel added them (`SRV-192`).
    pub(crate) fn handle_list_member_channels(&mut self) {
        let asked = self.member_channels_asked.issue();
        let Some(tx) = self.event_tx.clone() else {
            crate::log::warning!("Sync: no relay on this transport, no channels of the user");
            return;
        };
        let api_base = self.api_base.clone();
        let url_scheme = self.url_scheme;
        let auth = self.auth_context();
        let exchange_key = self.exchange_key.clone();
        // A listed name opens under the key the walked or kept log of its
        // meta accepted alone (`SYNC-291`), from an author it admits
        // (`SYNC-135`).
        // A name sealed before the last rotation opens through the chain the
        // log of its meta kept, with no request (`SYNC-302`).
        let kept = self.kept_names();
        let authors = self.name_authors();
        crate::spawn(async move {
            let url = crate::http::Url::<crate::http::ApiRoot>::of(url_scheme, &api_base)
                .route("/channels");
            let result =
                match http::get_json_auth::<http::MemberChannelsResponse>(&url, auth.clone()).await
                {
                    Ok(answer) => {
                        let names = match exchange_key.as_ref() {
                            Some(key) => open_channel_names(&answer, &authors, key, &kept),
                            None => Vec::new(),
                        };
                        let keys = exchange_key
                            .as_ref()
                            .map(|key| listed_keys(&answer, key))
                            .unwrap_or_default();
                        let (channels, spaces, peers) = answer.listing();
                        Ok(MemberListing {
                            channels,
                            spaces,
                            names,
                            peers,
                            keys,
                        })
                    }
                    Err(message) => Err(message),
                };
            let _ = tx.send(SyncEvent::MemberChannelsListed { asked, result });
        });
    }

    /// The answer of
    /// [`handle_list_member_channels`](Self::handle_list_member_channels), on
    /// the Sync loop. A failure answers nothing: the next share asks again.
    /// An answer a later listing overtook answers nothing either.
    /// The key of each listed `meta` is kept, and the Logic thread learns
    /// which `meta` it may watch.
    pub(crate) fn member_channels_listed(
        &mut self,
        asked: ambre_core::Seq<crate::generation::Listing>,
        result: Result<MemberListing, http::HttpError>,
    ) {
        if asked != self.member_channels_asked.latest() {
            return;
        }
        match result {
            Ok(MemberListing {
                channels,
                spaces,
                names,
                peers,
                keys,
            }) => {
                let metas = keys.iter().map(|(listed, _)| *listed).collect::<Vec<_>>();
                self.listed_keys = keys
                    .into_iter()
                    .map(|(listed, key)| (listed.channel_id, (listed.meta, key)))
                    .collect();
                self.send_to_logic(LogicEvent::MemberChannels {
                    channels,
                    spaces,
                    names,
                    peers,
                    metas,
                });
            }
            Err(message) => {
                crate::log::warning!("Sync: listing the channels of the user failed: {message}");
            }
        }
    }
}

/// What a listing of the user's channels answers: the channels, the spaces,
/// and the name each named channel shows before it opens. Each direct
/// message also carries its other members.
pub(crate) struct MemberListing {
    pub(crate) channels: Vec<MemberChannel>,
    pub(crate) spaces: Vec<Id<Space>>,
    pub(crate) names: Vec<ChannelName>,
    pub(crate) peers: Vec<ambre_bus::DirectPeers>,
    /// The key of each channel's `meta` that opened, with its channel.
    pub(crate) keys: Vec<(ambre_bus::ChannelMeta, DocumentKey)>,
}

/// The key of each `meta` of `answer`, unwrapped with the user's exchange
/// key. A key that does not open is left out: its channel is not watched.
fn listed_keys(
    answer: &http::MemberChannelsResponse,
    exchange_key: &StaticSecret,
) -> Vec<(ambre_bus::ChannelMeta, DocumentKey)> {
    answer
        .channels
        .iter()
        .filter_map(|entry| {
            let key = crate::documents::unwrap_listed_key(
                &entry.doc_key_enc,
                entry.wrapper_exchange_pub_key.as_ref(),
                exchange_key,
                entry.meta_id.document_id(),
            )?;
            let listed = ambre_bus::ChannelMeta {
                channel_id: entry.channel_id,
                meta: entry.meta_id,
                last: entry.meta_last,
                call: entry.call,
            };
            Some((listed, key))
        })
        .collect()
}

/// The names sealed beside each meta of `answer`, opened with the user's
/// wrapped key of that meta (`SYNC-171`). A name opens only from an author
/// `authors` admits (`SYNC-135`). It opens under the key `kept` holds for it
/// (`SYNC-291`), or an older key the kept chain of the meta reaches from it
/// (`SYNC-302`). A name that does not open is none, and so is an empty one:
/// a rename clears a name that way.
fn open_channel_names(
    answer: &http::MemberChannelsResponse,
    authors: &crate::documents::NameAuthors,
    exchange_key: &StaticSecret,
    kept: &crate::documents::KeptNames,
) -> Vec<ChannelName> {
    let mut names = Vec::new();
    for entry in &answer.channels {
        let Some(name_enc) = entry.name_enc.as_ref() else {
            continue;
        };
        let listed = crate::documents::ListedName {
            document_id: entry.meta_id.document_id(),
            doc_key_enc: &entry.doc_key_enc,
            wrapper_exchange_pub_key: entry.wrapper_exchange_pub_key.as_ref(),
            name_enc,
            kept: kept.keys.get(&entry.meta_id.document_id()).copied(),
            chain: kept.chains.get(&entry.meta_id.document_id()),
            authors,
        };
        match listed.open(exchange_key) {
            Some(name) if !ambre_core::name_is_empty(&name) => names.push(ChannelName {
                channel_id: entry.channel_id,
                name,
            }),
            _ => {}
        }
    }
    names
}

/// Move each cursor to its number, one request per document. A refused move
/// leaves that cursor where it was and says so in the log.
async fn put_cursors(
    base: &crate::http::Url<crate::http::ApiRoot>,
    cursors: Vec<(Id<Document>, Seq<Relay>)>,
    auth: http::AuthContext,
) {
    for (document_id, seq) in cursors {
        let url = base.route(format!("/documents/{document_id}/read-cursor"));
        let body = http::ReadCursorRequest { seq };
        if let Err(e) = http::put_json_auth(&url, &body, auth.clone()).await {
            crate::log::warning!("Sync: read cursor of {document_id} not moved: {e}");
        }
    }
}

/// The aad of a sealed channel key: it binds the key to its channel, so a
/// catalogue entry moved under another channel fails to open.
fn channel_key_aad(channel_id: Id<Channel>) -> ambre_core::Aad {
    let mut aad = b"ambre-channel-key:".to_vec();
    aad.extend_from_slice(channel_id.as_bytes());
    ambre_core::Aad::new(aad)
}

/// The digest of an entry in request form: the anchor of the log a first
/// entry opens, and the link the entry after it carries.
fn digest_of(
    entry: &ambre_core::Encoded<ambre_core::EncodedLogEntry>,
) -> Option<FixedBytes<ambre_protocol::EntryDigest>> {
    membership_log::decode(std::slice::from_ref(entry))
        .first()
        .and_then(membership_log::digest)
}

/// The channel key sealed under the space key for the catalogue, with the
/// anchor of meta's log beside it (`SYNC-104`). The id of the space key leads
/// the sealed bytes. A reader then opens it with the one key it names
/// (`SYNC-045`), and a rotation of the space shows which entries it left
/// behind.
pub(crate) fn seal_channel_key(
    channel_id: Id<Channel>,
    channel_key: &DocumentKey,
    space_key: &DocumentKey,
    meta_anchor: FixedBytes<ambre_protocol::EntryDigest>,
) -> Result<SealedChannelKey, SyncError> {
    let (nonce, ciphertext) = ambre_crypto::aead::seal_anchored_key(
        channel_key,
        &channel_key_aad(channel_id),
        space_key,
        Some(&meta_anchor),
    )?;
    Ok(SealedChannelKey::from_parts(
        space_key.id(),
        &nonce,
        &ciphertext,
    ))
}

/// The id of the space key a catalogue entry was sealed under. `None` for
/// bytes too short to name one.
pub(crate) fn sealed_under(sealed: &SealedChannelKey) -> Option<FixedBytes<KeyId>> {
    sealed.key_id()
}

/// Unseal a channel key the catalogue holds, with the anchor sealed beside it.
/// The space key that opens it is the one its first bytes name, as `keys`
/// reach it.
pub(crate) fn unseal_channel_key(
    channel_id: Id<Channel>,
    sealed: &SealedChannelKey,
    keys: &crate::chain::Keyring,
) -> Result<(DocumentKey, Option<FixedBytes<ambre_protocol::EntryDigest>>), SyncError> {
    let (key_id, nonce, ciphertext) = sealed
        .split()
        .ok_or(SyncError::Malformed(Part::SealedChannelKey))?;
    let space_key = keys.find(key_id).ok_or(SyncError::KeyOutOfReach)?;
    let space_key = &space_key;
    let (key, anchor) = ambre_crypto::aead::open_anchored_key(
        &nonce,
        &ciphertext,
        &channel_key_aad(channel_id),
        space_key,
    )?;
    Ok((key, anchor))
}

/// The self-join key: the catalogue's key, unsealed and wrapped for this
/// user alone, as the Murmure spaces spec asks of a self-join, and its id.
/// The wrap carries the anchor the catalogue sealed, so the joiner replays
/// meta's log from the word of the member who created the channel. It binds
/// `meta`, whose rows it lands on (audit `P59`). The join task wraps once the
/// join head names the meta, so only the tests take the two steps at once.
#[cfg(test)]
pub(crate) fn join_key(
    channel_id: Id<Channel>,
    meta: Id<Document>,
    sealed: &SealedChannelKey,
    space_keys: &crate::chain::Keyring,
    exchange_key: &StaticSecret,
) -> Result<
    (
        ambre_core::Encoded<ambre_core::WrappedDocKey>,
        FixedBytes<KeyId>,
    ),
    SyncError,
> {
    let unsealed = unseal_channel_key(channel_id, sealed, space_keys)?;
    let wrapped = wrap_join_key(meta, &unsealed, exchange_key)?;
    Ok((wrapped, unsealed.0.id()))
}

/// The copy of an unsealed catalogue key and its anchor this user wraps for
/// itself, bound to `meta`.
fn wrap_join_key(
    meta: Id<Document>,
    (key, anchor): &(DocumentKey, Option<FixedBytes<ambre_protocol::EntryDigest>>),
    exchange_key: &StaticSecret,
) -> Result<ambre_core::Encoded<ambre_core::WrappedDocKey>, SyncError> {
    let own_pub = crate::sharing::own_exchange_key(exchange_key);
    Ok(crate::sharing::wrap_anchored_b64(
        exchange_key,
        &own_pub,
        meta,
        key,
        *anchor,
    )?)
}

/// The `SelfJoin` entry that extends the log the join head names, at the
/// role it grants (`SYNC-106`). `None` for an empty log: it predates the
/// log, and no entry of it could link to a first one.
pub(crate) fn self_join_entry(
    author: &crate::sharing::Author,
    exchange: FixedBytes<ambre_core::ExchangePublicKey>,
    head: &http::JoinHeadResponse,
    key: FixedBytes<KeyId>,
) -> Result<Option<ambre_core::Encoded<ambre_core::EncodedLogEntry>>, SyncError> {
    if head.length == ambre_core::Seq::ZERO {
        return Ok(None);
    }
    let meta = head.meta_id.document_id();
    let last = head
        .last
        .as_ref()
        .and_then(digest_of)
        .ok_or(SyncError::Malformed(Part::JoinHead))?;
    let draft = membership_log::Draft {
        document: meta,
        member: author.user_id,
        member_key: author.public(),
        member_exchange_key: exchange,
        act: ambre_protocol::MembershipAct::SelfJoin,
        role: head.role,
        key: Some(key),
    };
    let entry = membership_log::sign(
        draft,
        membership_log::LogHead {
            position: head.length,
            previous: Some(last),
        },
        author.user_id,
        &author.signing_key,
    )?;
    Ok(Some(membership_log::encode(&entry)?))
}

/// The request that creates a direct message of the signer and `members`
/// under a fresh key.
///
/// Meta's log makes every member an editor, and leaves no owner. The signer
/// creates it, invites each member as an editor, then lowers its own role to
/// editor. Each member comes with the signing key their invite names. Every
/// wrap carries the log's root, so each member replays it. The segment opens
/// no log of its own: the relay resolves its access to meta's members.
pub(crate) fn direct_message_request(
    author: &crate::sharing::Author,
    exchange_key: &StaticSecret,
    channel_id: Id<Channel>,
    (meta, segment): (Id<Meta>, Id<Segment>),
    members: &[(&DirectMember, FixedBytes<ambre_core::PublicKey>)],
) -> Result<http::CreateDirectMessageRequest, SyncError> {
    let key = DocumentKey::generate();
    let document = meta.document_id();
    let own_key = author.public();
    let others: Vec<Id<User>> = members.iter().map(|(m, _)| m.user_id).collect();
    let own_pub = crate::sharing::own_exchange_key(exchange_key);
    // Each entry signs the member's exchange key beside the signing key, so
    // a later rotation wraps for the key the log names (`SYNC-197`).
    let member_keys: HashMap<
        Id<User>,
        (
            FixedBytes<ambre_core::PublicKey>,
            FixedBytes<ambre_core::ExchangePublicKey>,
        ),
    > = members
        .iter()
        .map(|(m, signing)| (m.user_id, (*signing, m.exchange_pub_key)))
        .chain([(author.user_id, (own_key, own_pub))])
        .collect();
    let mut next = membership_log::LogHead::EMPTY;
    let mut root = None;
    let mut entries = Vec::with_capacity(others.len().saturating_add(2));
    for (member, act, role) in ambre_protocol::ownerless_log(author.user_id, &others) {
        let (member_key, member_exchange_key) = member_keys
            .get(&member)
            .copied()
            .ok_or(SyncError::MemberUnsigned)?;
        let draft = membership_log::Draft {
            document,
            member,
            member_key,
            member_exchange_key,
            act,
            role,
            key: (act != ambre_protocol::MembershipAct::ChangeRole).then(|| key.id()),
        };
        let entry = membership_log::sign(draft, next, author.user_id, &author.signing_key)?;
        let digest = membership_log::digest(&entry).ok_or(SyncError::NoDigest)?;
        root.get_or_insert(digest);
        next = next.after(digest);
        entries.push(membership_log::encode(&entry)?);
    }
    // The creation goes first, so the first digest is the log's root.
    let root = root.ok_or(SyncError::NoDigest)?;
    let wrap = |peer: &FixedBytes<ambre_core::ExchangePublicKey>| {
        crate::sharing::wrap_anchored_b64(exchange_key, peer, document, &key, Some(root))
    };
    let members = members
        .iter()
        .map(|(m, _)| {
            wrap(&m.exchange_pub_key).map(|doc_key_enc| http::DirectMemberJson {
                user_id: m.user_id,
                doc_key_enc,
            })
        })
        .collect::<Result<_, _>>()?;
    Ok(http::CreateDirectMessageRequest {
        channel_id,
        meta_id: meta,
        segment_id: segment,
        members,
        doc_key_enc: wrap(&own_pub)?,
        key_id: key.id(),
        entries,
    })
}

/// Refuse a member whose keys, as the relay serves them now, differ from
/// the keys this session showed. Those are the exchange key the view pinned
/// and the signing key the fingerprint covered beside it (`CHROME-140`).
///
/// A member this session showed no keys for is refused too. The signing key
/// the entry binds would then be one no person compared.
pub(crate) fn check_pinned(
    member: &DirectMember,
    served: &http::PubkeysResponse,
    shown: Option<&ambre_crypto::IdentityKeys>,
) -> Result<(), SyncError> {
    let served_keys = ambre_crypto::IdentityKeys {
        exchange: served.exchange_pubkey,
        signing: served.signing_pubkey,
    };
    let pinned = served.exchange_pubkey == member.exchange_pub_key;
    if pinned && shown == Some(&served_keys) {
        Ok(())
    } else {
        Err(SyncError::PinMismatch(member.user_id))
    }
}

/// The activity answer, as the bus carries it. One row the client cannot
/// read refused the whole answer when it was read, as a listing does.
pub(crate) fn activity_of(entries: &[http::ActivityEntry]) -> Vec<DocumentActivity> {
    entries
        .iter()
        .map(|e| DocumentActivity {
            channel_id: e.channel_id,
            document_id: e.document_id,
            kind: e.kind,
            active: e.active,
            read: e.read,
            last: e.last,
        })
        .collect()
}

/// The listing the relay answered, as the bus carries it. One row the client
/// cannot read refused the whole listing when it was read. A channel opened
/// on part of its documents would show a history with holes.
pub(crate) fn channel_documents(entries: &[http::ChannelDocumentEntry]) -> Vec<ChannelDocument> {
    entries
        .iter()
        .map(|e| ChannelDocument {
            document_id: e.document_id,
            kind: e.kind,
        })
        .collect()
}

/// Whether a channel document lands deltas and seals frames, against the walk
/// of its log under the rule of a space (`SYNC-231`, `SYNC-232`, `SYNC-234`).
#[derive(Default)]
pub(crate) enum LogGate {
    /// No walk is owed: the writers the latest walk granted decide.
    #[default]
    Open,
    /// The log walks again. A writer of the earlier walk may be the relay's,
    /// and the key it vouched for too.
    Walking(Walk),
    /// A walk found the log rooted outside its space. Nothing is sealed under
    /// its key again.
    Ended,
}

/// What a channel document holds while its log walks again, until the answer
/// of that walk (`SYNC-232`, `SYNC-234`).
pub(crate) struct Walk {
    /// The listing each delta held counts as seen at.
    floor: ambre_core::Seq<crate::generation::Listing>,
    /// The listing whose answer ends the walk.
    asked: ambre_core::Seq<crate::generation::Listing>,
    /// Each edit this client wrote, unsealed, in order. Bounded by
    /// [`QUARANTINE_LIMIT`].
    outbound: Vec<pipeline::UpdateFrame>,
    /// Each frame the relay refused, to seal again at its own clock. Bounded
    /// by the sent log it comes from (`rekey::UNCONFIRMED_LIMIT`).
    resend: Vec<(
        ambre_protocol::messages::Seq<ambre_protocol::messages::DeltaAuthor>,
        pipeline::UpdateFrame,
    )>,
    /// Each snapshot a peer announced, fetched again once the walk answers.
    /// Bounded by [`QUARANTINE_LIMIT`].
    snapshots: HashMap<Id<Snapshot>, Seq<Relay>>,
}

/// What the gate did with an edit handed to it.
pub(crate) enum Hold {
    /// The gate is open: seal the edit now.
    Seal(pipeline::UpdateFrame),
    /// The walk holds the edit unsealed.
    Held,
    /// The walk already held its bound of edits. It wiped each of them and
    /// this one, so no later edit builds on one the relay never gets.
    Overflow,
    /// The log is rooted outside its space. The edit is wiped.
    Ended,
}

/// What the gate did with a snapshot it refused to adopt.
pub(crate) enum Deferral {
    /// The walk fetches the snapshot again once it answers.
    Held,
    /// The walk already held its bound of snapshots. This one is dropped.
    Overflow,
    /// The log is rooted outside its space, or no walk is owed.
    Dropped,
}

/// What a walk held, handed back by its answer.
pub(crate) struct Released {
    pub outbound: Vec<pipeline::UpdateFrame>,
    pub resend: Vec<(
        ambre_protocol::messages::Seq<ambre_protocol::messages::DeltaAuthor>,
        pipeline::UpdateFrame,
    )>,
    pub snapshots: HashMap<Id<Snapshot>, Seq<Relay>>,
}

impl LogGate {
    /// The listing each delta held during a walk counts as seen at, so the
    /// answer of that walk decides it. `None` outside a walk.
    pub(crate) fn floor(&self) -> Option<ambre_core::Seq<crate::generation::Listing>> {
        match self {
            Self::Walking(walk) => Some(walk.floor),
            Self::Open | Self::Ended => None,
        }
    }

    /// The log walks again as the listing `asked`, the latest one asked
    /// before it being `floor`. A walk already owed keeps
    /// its floor and what it holds, and waits for the newer answer. An ended
    /// gate stays ended.
    fn walk(
        &mut self,
        floor: ambre_core::Seq<crate::generation::Listing>,
        asked: ambre_core::Seq<crate::generation::Listing>,
    ) {
        match self {
            Self::Open => {
                *self = Self::Walking(Walk {
                    floor,
                    asked,
                    outbound: Vec::new(),
                    resend: Vec::new(),
                    snapshots: HashMap::new(),
                });
            }
            Self::Walking(walk) => walk.asked = asked,
            Self::Ended => {}
        }
    }

    /// Holds `body` unsealed while a walk is owed, up to
    /// [`QUARANTINE_LIMIT`] edits. Hands it back when the gate is open.
    pub(crate) fn hold(&mut self, body: pipeline::UpdateFrame) -> Hold {
        match self {
            Self::Open => Hold::Seal(body),
            Self::Walking(walk)
                if ambre_core::Count::of(walk.outbound.len()) >= crate::QUARANTINE_LIMIT =>
            {
                body.wipe();
                walk.outbound
                    .drain(..)
                    .for_each(pipeline::UpdateFrame::wipe);
                Hold::Overflow
            }
            Self::Walking(walk) => {
                walk.outbound.push(body);
                Hold::Held
            }
            Self::Ended => {
                body.wipe();
                Hold::Ended
            }
        }
    }

    /// Holds the frames the relay refused while a walk is owed, each at its
    /// own clock. A clock held already keeps its later copy. Hands them back
    /// when the gate is open, and wipes them when it ended.
    pub(crate) fn hold_resend(
        &mut self,
        refused: Vec<(
            ambre_protocol::messages::Seq<ambre_protocol::messages::DeltaAuthor>,
            pipeline::UpdateFrame,
        )>,
    ) -> Vec<(
        ambre_protocol::messages::Seq<ambre_protocol::messages::DeltaAuthor>,
        pipeline::UpdateFrame,
    )> {
        match self {
            Self::Open => refused,
            Self::Walking(walk) => {
                for (clock, body) in refused {
                    if let Some(held) = walk.resend.iter_mut().find(|(held, _)| *held == clock) {
                        std::mem::replace(&mut held.1, body).wipe();
                    } else {
                        walk.resend.push((clock, body));
                    }
                }
                ambre_core::order::sort_by_seq(&mut walk.resend, |(clock, _)| *clock);
                Vec::new()
            }
            Self::Ended => {
                refused.into_iter().for_each(|(_, body)| body.wipe());
                Vec::new()
            }
        }
    }

    /// Defers the adoption of `snapshot_id` to the answer of the walk, up to
    /// [`QUARANTINE_LIMIT`] snapshots.
    pub(crate) fn defer_snapshot(
        &mut self,
        snapshot_id: Id<Snapshot>,
        covered_seq: Seq<Relay>,
    ) -> Deferral {
        match self {
            Self::Walking(walk)
                if ambre_core::Count::of(walk.snapshots.len()) >= crate::QUARANTINE_LIMIT
                    && !walk.snapshots.contains_key(&snapshot_id) =>
            {
                Deferral::Overflow
            }
            Self::Walking(walk) => {
                walk.snapshots.insert(snapshot_id, covered_seq);
                Deferral::Held
            }
            Self::Open | Self::Ended => Deferral::Dropped,
        }
    }

    /// Opens the gate if `asked` answers the walk owed, and hands back what
    /// the walk held. `None` when no walk is owed, or a newer one is.
    fn release(&mut self, asked: ambre_core::Seq<crate::generation::Listing>) -> Option<Released> {
        if !matches!(self, Self::Walking(walk) if asked >= walk.asked) {
            return None;
        }
        match std::mem::replace(self, Self::Open) {
            Self::Walking(walk) => Some(Released {
                outbound: walk.outbound,
                resend: walk.resend,
                snapshots: walk.snapshots,
            }),
            Self::Open | Self::Ended => None,
        }
    }

    /// Ends the gate for good, and wipes each frame it held.
    fn end(&mut self) {
        if let Self::Walking(walk) = std::mem::replace(self, Self::Ended) {
            walk.outbound
                .into_iter()
                .for_each(pipeline::UpdateFrame::wipe);
            walk.resend.into_iter().for_each(|(_, body)| body.wipe());
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// [`QUARANTINE_LIMIT`] as a length, the bound of what a walk holds.
    fn walk_limit() -> usize {
        usize::try_from(QUARANTINE_LIMIT.get()).unwrap_or(usize::MAX)
    }
    use ambre_bus::Role;

    /// The listing as the relay writes it: each row's id as its text.
    fn listing(rows: &[(&str, &str)]) -> Result<Vec<ChannelDocument>, serde_json::Error> {
        let documents: Vec<_> = rows
            .iter()
            .map(|(id, kind)| serde_json::json!({ "document_id": id, "kind": kind }))
            .collect();
        let body = serde_json::json!({ "documents": documents });
        serde_json::from_value::<http::ChannelDocumentsResponse>(body)
            .map(|resp| channel_documents(&resp.documents))
    }

    #[test]
    fn a_listing_parses_into_its_documents_in_order() {
        let meta = Id::<Document>::mint();
        let segment = Id::<Document>::mint();
        let parsed = listing(&[
            (&meta.to_string(), "meta"),
            (&segment.to_string(), "segment"),
        ])
        .expect("parse");
        assert_eq!(
            parsed,
            [
                ChannelDocument {
                    document_id: meta,
                    kind: ChannelDocumentKind::Meta
                },
                ChannelDocument {
                    document_id: segment,
                    kind: ChannelDocumentKind::Segment
                },
            ]
        );
    }

    #[test]
    fn one_unreadable_row_fails_the_whole_listing() {
        let meta = Id::<Document>::mint().to_string();
        // Bad document_id causes an error
        assert!(listing(&[(&meta, "meta"), ("nope", "segment")]).is_err());
        // Single valid entry succeeds
        assert!(listing(&[(&meta, "meta")]).is_ok());
    }

    /// The activity answer as the relay writes it: one row.
    fn activity(
        channel_id: &str,
        document_id: &str,
        active: bool,
    ) -> Result<Vec<DocumentActivity>, serde_json::Error> {
        let row = serde_json::json!({
            "channel_id": channel_id,
            "document_id": document_id,
            "kind": "thread",
            "active": active,
            "read": 7,
            "last": 9,
        });
        let body = serde_json::json!({ "documents": [row] });
        serde_json::from_value::<http::ActivityResponse>(body)
            .map(|resp| activity_of(&resp.documents))
    }

    #[test]
    fn an_activity_answer_parses_into_its_documents() {
        let (channel, segment) = (Id::<Channel>::mint(), Id::<Document>::mint());
        let parsed = activity(&channel.to_string(), &segment.to_string(), true).expect("ok");
        assert_eq!(
            parsed,
            [DocumentActivity {
                channel_id: channel,
                document_id: segment,
                kind: ChannelDocumentKind::Thread,
                active: ambre_bus::Activity::Active,
                read: Seq::<Relay>::new(7),
                last: Some(Seq::<Relay>::new(9)),
            }]
        );
        let bad = activity(&channel.to_string(), "nope", false);
        assert!(bad.is_err(), "one bad row fails the answer");
        // A relay that predates the last number answers none.
        let row = serde_json::json!({
            "channel_id": channel.to_string(),
            "document_id": segment.to_string(),
            "kind": "segment",
            "active": false,
        });
        let older = serde_json::from_value::<http::ActivityResponse>(
            serde_json::json!({ "documents": [row] }),
        )
        .map(|resp| activity_of(&resp.documents))
        .expect("ok");
        assert_eq!(
            older.first().map(|d| (d.read, d.last)),
            Some((Seq::<Relay>::ZERO, None))
        );
    }

    /// An anchor of meta's log, as the creator's `Create` entry would give it.
    const ANCHOR: FixedBytes<ambre_protocol::EntryDigest> = FixedBytes::from_array([5; 32]);

    fn exchange_key(byte: u8) -> StaticSecret {
        StaticSecret::from([byte; 32])
    }

    /// The meta every self-join test wraps a copy for.
    const META: Id<Document> = Id::<Document>::from_bytes([0x3e; 16]);

    /// The key a wrapped value of [`META`] opens to, for its holder.
    fn unwrap_own(
        exchange_key: &StaticSecret,
        doc_key_enc: &ambre_core::Encoded<ambre_core::WrappedDocKey>,
    ) -> Option<DocumentKey> {
        let own_pub = crate::sharing::own_exchange_key(exchange_key);
        let (key, anchor) =
            ambre_crypto::ecdh::unwrap_anchored(exchange_key, &own_pub, META, doc_key_enc).ok()?;
        assert_eq!(anchor, Some(ANCHOR), "every wrap carries meta's anchor");
        Some(key)
    }

    /// A channel key sealed for the catalogue, and its creator's own copy.
    fn new_channel_key(
        channel_id: Id<Channel>,
        space_key: &DocumentKey,
        creator: &StaticSecret,
    ) -> (
        SealedChannelKey,
        ambre_core::Encoded<ambre_core::WrappedDocKey>,
    ) {
        let channel_key = DocumentKey::generate();
        let sealed = seal_channel_key(channel_id, &channel_key, space_key, ANCHOR).expect("seal");
        let own_pub = crate::sharing::own_exchange_key(creator);
        let own =
            crate::sharing::wrap_anchored_b64(creator, &own_pub, META, &channel_key, Some(ANCHOR))
                .expect("wrap");
        (sealed, own)
    }

    /// The keys of a space whose current key is `current`, after rotations
    /// that replaced each of `previous`.
    pub(crate) fn ring(current: &DocumentKey, previous: &[&DocumentKey]) -> crate::chain::Keyring {
        let document_id = Id::<Document>::mint();
        crate::chain::Keyring {
            document_id,
            current: current.clone(),
            previous: previous.iter().map(|&key| key.clone()).collect(),
            chain: crate::chain::Chain::new(document_id, Vec::new()),
        }
    }

    /// Whether two keys are one: what one seals, the other opens. The probe
    /// seals `a` under itself through the typed key sealer, so no test hands
    /// bare bytes to the cipher.
    fn same_key(a: &DocumentKey, b: &DocumentKey) -> bool {
        let aad = ambre_core::Aad::of(b"");
        let (nonce, ct) = ambre_crypto::aead::seal_document_key(a, &aad, a).expect("seal");
        ambre_crypto::aead::open_document_key(&nonce, &ct, &aad, b).is_ok()
    }

    // --- The Murmure spaces spec: the sealed catalogue key and a self-join ---

    /// Alice creates a channel: the catalogue gets its key sealed under the
    /// space key. Bob joins alone: his worker unseals it and wraps it for
    /// himself, and the key he opens is the key Alice's documents are under.
    #[test]
    fn a_self_join_unseals_the_catalogue_key_and_wraps_it_for_the_joiner_alone() {
        let space_key = DocumentKey::generate();
        let (alice, bob, eve) = (exchange_key(1), exchange_key(2), exchange_key(3));
        let channel_id = Id::<Channel>::mint();
        let (sealed, alice_enc) = new_channel_key(channel_id, &space_key, &alice);
        let channel_key = unwrap_own(&alice, &alice_enc).expect("Alice opens her own wrap");

        let (bob_enc, key_id) =
            join_key(channel_id, META, &sealed, &ring(&space_key, &[]), &bob).expect("join");
        assert_eq!(key_id, channel_key.id());
        let bob_key = unwrap_own(&bob, &bob_enc).expect("Bob opens his wrap");
        assert!(same_key(&bob_key, &channel_key));
        assert!(
            unwrap_own(&eve, &bob_enc).is_none(),
            "the wrap is Bob's alone"
        );
    }

    /// Holds `SYNC-299` at the worker: an open of a listed channel pins the
    /// channel key its catalogue entry holds. The roots of each open of its
    /// `meta` carry the pin. An entry that does not unseal pins no key,
    /// and a channel no catalogue lists pins nothing.
    #[test]
    fn a_listed_channel_open_pins_the_key_its_catalogue_holds() {
        let mut state = SyncState::new(
            LogicSink::channel().0,
            crate::http::Url::<crate::RelayHost>::new(String::new()),
        );
        state.exchange_key = Some(exchange_key(1));
        let (space_id, space_key) = (Id::<Space>::mint(), DocumentKey::generate());
        state.documents.insert(
            space_id.document_id(),
            crate::SyncDocumentState::new(
                space_id.document_id(),
                space_key.clone(),
                DocType::Murmure,
                crate::Existence::OnRelay,
            ),
        );
        let (channel_id, meta) = (Id::<Channel>::mint(), Id::<Meta>::mint());
        let (sealed, own) = new_channel_key(channel_id, &space_key, &exchange_key(1));
        let channel_key = unwrap_own(&exchange_key(1), &own).expect("own wrap");
        let open = |state: &mut SyncState, listed: Option<SealedChannelKey>| {
            state.handle_command(SyncCommand::OpenChannel {
                channel_id,
                listed_in: Some(space_id),
                catalogue_key: listed,
            });
            let documents = vec![ambre_bus::ChannelDocument {
                document_id: meta.document_id(),
                kind: ChannelDocumentKind::Meta,
            }];
            state.channel_listed(channel_id, Some(space_id), Ok(documents));
            state.roots(meta.document_id(), DocType::Murmure).catalogue
        };

        assert_eq!(
            open(&mut state, Some(sealed)),
            Some(crate::key_vouch::CataloguePin::Key(channel_key.id()))
        );
        let foreign = new_channel_key(channel_id, &DocumentKey::generate(), &exchange_key(1)).0;
        assert_eq!(
            open(&mut state, Some(foreign)),
            Some(crate::key_vouch::CataloguePin::Unreadable)
        );
        assert_eq!(open(&mut state, None), None);
    }

    /// The sealed key opens under the space key of its own channel only: a
    /// wrong space key, or an entry moved under another channel, fails.
    #[test]
    fn a_sealed_channel_key_opens_for_its_own_channel_and_space_only() {
        let space_key = DocumentKey::generate();
        let channel_id = Id::<Channel>::mint();
        let (sealed, _) = new_channel_key(channel_id, &space_key, &exchange_key(1));
        let keys = ring(&space_key, &[]);
        assert_eq!(
            sealed.as_bytes().len(),
            FixedBytes::<KeyId>::SIZE.saturating_len().to_usize()
                + FixedBytes::<ambre_core::Nonce>::SIZE
                    .saturating_len()
                    .to_usize()
                + 32
                + 32
                + 16
        );
        let (_, anchor) = unseal_channel_key(channel_id, &sealed, &keys).expect("open");
        assert_eq!(anchor, Some(ANCHOR));
        let stranger = ring(&DocumentKey::generate(), &[]);
        assert!(unseal_channel_key(channel_id, &sealed, &stranger).is_err());
        assert!(unseal_channel_key(Id::<Channel>::mint(), &sealed, &keys).is_err());
        let short = SealedChannelKey::from_bytes(vec![0; 8]);
        assert!(unseal_channel_key(channel_id, &short, &keys).is_err());
        assert_eq!(sealed_under(&short), None);
    }

    /// The sealed key names the space key that sealed it. After a rotation of
    /// the space, the entry names the older key. The space's keys still reach
    /// it, so the entry opens, and reads as one to reseal.
    #[test]
    fn a_sealed_channel_key_names_its_space_key_and_opens_after_a_rotation() {
        let (old, new) = (DocumentKey::generate(), DocumentKey::generate());
        let channel_id = Id::<Channel>::mint();
        let (sealed, _) = new_channel_key(channel_id, &old, &exchange_key(1));
        assert_eq!(sealed_under(&sealed), Some(old.id()));
        let rotated = ring(&new, &[&old]);
        assert_ne!(sealed_under(&sealed), Some(rotated.current.id()));
        let (_, anchor) = unseal_channel_key(channel_id, &sealed, &rotated).expect("open");
        assert_eq!(anchor, Some(ANCHOR));
    }

    /// With no relay, a creation and a join fail at once, and say so.
    #[test]
    fn with_no_relay_a_creation_and_a_join_fail_at_once() {
        let (sink, rx) = LogicSink::channel();
        let state = SyncState::new(
            sink,
            crate::http::Url::<crate::RelayHost>::new(String::new()),
        );
        let (channel_id, space_id) = (Id::<Channel>::mint(), Id::<Space>::mint());
        for visibility in [
            ambre_core::Visibility::Public,
            ambre_core::Visibility::Private,
        ] {
            let documents = (Id::<Meta>::mint(), Id::<Segment>::mint());
            state.handle_create_channel(channel_id, space_id, documents, visibility, None);
        }
        state.handle_join_channel(
            channel_id,
            space_id,
            &SealedChannelKey::from_bytes(vec![1; 72]),
        );
        let events: Vec<LogicEvent> = rx.try_iter().collect();
        assert_eq!(events.len(), 3);
        assert!(events.iter().all(|e| matches!(
            e,
            LogicEvent::ChannelRequestFailed { channel_id: id, .. } if *id == channel_id
        )));
    }

    /// A delete answers the Logic thread either way, and touches no document
    /// session. The Logic thread closes them once it hears the relay deleted
    /// the channel. With no relay, the delete fails at once.
    #[test]
    fn a_delete_answers_logic_and_leaves_the_sessions_to_it() {
        let (sink, rx) = LogicSink::channel();
        let mut state = SyncState::new(
            sink,
            crate::http::Url::<crate::RelayHost>::new(String::new()),
        );
        let channel_id = Id::<Channel>::mint();
        let meta = Id::<Document>::mint();
        state.documents.insert(
            meta,
            SyncDocumentState::new(
                meta,
                DocumentKey::generate(),
                DocType::Murmure,
                crate::Existence::OnRelay,
            ),
        );

        state.handle_delete_channel(channel_id);
        state.handle_sync_event(SyncEvent::ChannelDeleted {
            channel_id,
            result: Err(crate::http::HttpError::Local(
                ambre_core::LocalFailure::NoRelay,
            )),
        });
        state.handle_sync_event(SyncEvent::ChannelDeleted {
            channel_id,
            result: Ok(()),
        });
        let events: Vec<LogicEvent> = rx.try_iter().collect();
        assert!(
            matches!(
                events.as_slice(),
                [
                    LogicEvent::ChannelRequestFailed { channel_id: a, .. },
                    LogicEvent::ChannelRequestFailed { channel_id: b, .. },
                    LogicEvent::ChannelDeleted { channel_id: c },
                ] if [*a, *b, *c] == [channel_id; 3]
            ),
            "no relay, a refusal, then the relay's yes: {events:?}"
        );
        assert!(
            state.documents.contains_key(&meta),
            "no session closed here"
        );
    }

    /// A leave answers the Logic thread either way, and touches no document
    /// session, as a delete does. With no relay, the leave fails at once.
    #[test]
    fn a_leave_answers_logic_and_leaves_the_sessions_to_it() {
        let (sink, rx) = LogicSink::channel();
        let mut state = SyncState::new(
            sink,
            crate::http::Url::<crate::RelayHost>::new(String::new()),
        );
        let channel_id = Id::<Channel>::mint();
        let meta = Id::<Document>::mint();
        state.documents.insert(
            meta,
            SyncDocumentState::new(
                meta,
                DocumentKey::generate(),
                DocType::Murmure,
                crate::Existence::OnRelay,
            ),
        );

        state.handle_leave_channel(channel_id, vec![meta], Vec::new());
        state.handle_sync_event(SyncEvent::ChannelLeft {
            channel_id,
            result: Err(crate::http::HttpError::Local(
                ambre_core::LocalFailure::NoRelay,
            )),
        });
        state.handle_sync_event(SyncEvent::ChannelLeft {
            channel_id,
            result: Ok(()),
        });
        let events: Vec<LogicEvent> = rx.try_iter().collect();
        assert!(
            matches!(
                events.as_slice(),
                [
                    LogicEvent::ChannelRequestFailed { channel_id: a, .. },
                    LogicEvent::ChannelRequestFailed { channel_id: b, .. },
                    LogicEvent::ChannelLeft { channel_id: c },
                ] if [*a, *b, *c] == [channel_id; 3]
            ),
            "no relay, a refusal, then the relay's yes: {events:?}"
        );
        assert!(
            state.documents.contains_key(&meta),
            "no session closed here"
        );
    }

    /// A listing the relay refuses for want of membership tells the Logic
    /// thread the channel is refused. Any other failure stays a failure to
    /// open.
    #[test]
    fn a_listing_refused_for_want_of_membership_reads_as_refused() {
        let (sink, rx) = LogicSink::channel();
        let mut state = SyncState::new(
            sink,
            crate::http::Url::<crate::RelayHost>::new(String::new()),
        );
        let channel_id = Id::<Channel>::mint();
        let url = "https://relay.example/api/channels/c/documents";
        for (status, code) in [
            (403, Some(ambre_core::RefusalCode::Forbidden)),
            (403, None),
            (500, None),
        ] {
            let error = crate::http::HttpError::refused(
                crate::http::Method::Get,
                &crate::http::Target::relay(url),
                ambre_core::HttpStatus::new(status),
                code,
            );
            state.channel_listed(channel_id, None, Err(error));
        }
        let events: Vec<LogicEvent> = rx.try_iter().collect();
        assert!(
            matches!(
                events.as_slice(),
                [
                    LogicEvent::ChannelRefused { channel_id: a },
                    LogicEvent::ChannelOpenFailed { channel_id: b, .. },
                    LogicEvent::ChannelOpenFailed { channel_id: c, .. },
                ] if [*a, *b, *c] == [channel_id; 3]
            ),
            "{events:?}"
        );
    }

    /// The relay's listing of a space's public channels decodes to their ids
    /// and reaches the Logic thread. A failed listing answers nothing.
    #[test]
    fn a_listing_of_a_space_forwards_its_channels_and_a_failure_nothing() {
        let (sink, rx) = LogicSink::channel();
        let state = SyncState::new(
            sink,
            crate::http::Url::<crate::RelayHost>::new(String::new()),
        );
        let space_id = Id::<Space>::mint();
        let channel_id = Id::<Channel>::mint();
        let body = format!(r#"{{"channels":["{channel_id}"]}}"#);
        let answer: http::SpaceChannelsResponse = serde_json::from_str(&body).expect("decodes");
        state.space_channels_listed(space_id, Ok(answer.channels));
        state.space_channels_listed(
            space_id,
            Err(crate::http::HttpError::Local(
                ambre_core::LocalFailure::NoRelay,
            )),
        );
        let events: Vec<LogicEvent> = rx.try_iter().collect();
        assert!(
            matches!(
                events.as_slice(),
                [LogicEvent::SpaceChannels { space_id: s, channels }]
                    if *s == space_id && *channels == [channel_id]
            ),
            "{events:?}"
        );
    }

    /// The share of a Murmure document asks for the user's channels again,
    /// since a channel reaches a new member as the share of its meta. The
    /// share of any other document asks nothing.
    #[test]
    fn the_share_of_a_murmure_document_lists_the_users_channels() {
        let (sink, rx) = LogicSink::channel();
        let mut state = SyncState::new(
            sink,
            crate::http::Url::<crate::RelayHost>::new(String::new()),
        );
        for doc_type in [DocType::Prose, DocType::Murmure] {
            state.handle_ws_message(ambre_protocol::ws::WsServerMsg::DocumentShared {
                document_id: Id::<Document>::mint(),
                doc_type,
            });
        }
        assert_eq!(
            state.member_channels_asked.latest(),
            ambre_core::Seq::nth(1),
            "one listing"
        );
        let shared = rx
            .try_iter()
            .filter(|e| matches!(e, LogicEvent::DocumentShared { .. }))
            .count();
        assert_eq!(shared, 2, "each share still reaches Logic");
    }

    /// A lost document can be the meta of a channel never opened. Its loss
    /// lists the user's channels again, unless it is held as another kind.
    /// The loss of a held Prose document asks nothing.
    #[test]
    fn the_loss_of_a_document_not_held_as_another_kind_lists_the_users_channels() {
        let (sink, rx) = LogicSink::channel();
        let mut state = SyncState::new(
            sink,
            crate::http::Url::<crate::RelayHost>::new(String::new()),
        );
        state.user_id = Some(Id::<User>::mint());
        let (prose, murmure, unheld) = (
            Id::<Document>::mint(),
            Id::<Document>::mint(),
            Id::<Document>::mint(),
        );
        for (id, doc_type) in [(prose, DocType::Prose), (murmure, DocType::Murmure)] {
            let doc = crate::SyncDocumentState::new(
                id,
                DocumentKey::generate(),
                doc_type,
                crate::Existence::OnRelay,
            );
            state.documents.insert(id, doc);
        }
        for document_id in [prose, murmure, unheld] {
            state.handle_ws_message(ambre_protocol::ws::WsServerMsg::DocumentRevoked {
                document_id,
            });
        }
        assert_eq!(
            state.member_channels_asked.latest(),
            ambre_core::Seq::nth(2),
            "one listing per channel loss"
        );
        let revoked = rx
            .try_iter()
            .filter(|e| matches!(e, LogicEvent::MemberRevoked { .. }))
            .count();
        assert_eq!(revoked, 3, "each loss still reaches Logic");
    }

    /// The relay's listing of the user's channels decodes to their places
    /// and reaches the Logic thread, with every space of the user. A failed
    /// listing answers nothing, nor does one a later listing overtook.
    #[test]
    fn a_listing_of_the_users_channels_forwards_their_places() {
        let (sink, rx) = LogicSink::channel();
        let mut state = SyncState::new(
            sink,
            crate::http::Url::<crate::RelayHost>::new(String::new()),
        );
        state.handle_list_member_channels();
        state.handle_list_member_channels();
        let (space, bare) = (Id::<Space>::mint(), Id::<Space>::mint());
        let (public, private, direct) = (
            Id::<Channel>::mint(),
            Id::<Channel>::mint(),
            Id::<Channel>::mint(),
        );
        let meta = Id::<Meta>::mint();
        // A wrap of the right length that opens under no key of this user.
        let keys = format!(
            r#""doc_key_enc":{},"wrapper_exchange_pub_key":null"#,
            serde_json::to_string(&ambre_core::Encoded::<ambre_core::WrappedDocKey>::new(
                vec![0; 73]
            ))
            .unwrap()
        );
        let body = format!(
            r#"{{"channels":[
                {{"channel_id":"{public}","space_id":"{space}","public":true,"meta_id":"{meta}",{keys},"peers":["Eve"]}},
                {{"channel_id":"{private}","space_id":"{space}","public":false,"meta_id":"{meta}",{keys},"meta_last":6}},
                {{"channel_id":"{direct}","space_id":null,"public":false,"meta_id":"{meta}",{keys},"peers":["Bob","Carol"]}}],
                "spaces":["{space}","{bare}"]}}"#
        );
        let answer: http::MemberChannelsResponse = serde_json::from_str(&body).expect("decodes");
        let lasts: Vec<_> = answer.channels.iter().map(|c| c.meta_last).collect();
        assert_eq!(
            lasts,
            [
                None,
                Some(ambre_core::Seq::<ambre_core::Relay>::new(6)),
                None
            ]
        );
        let named = vec![ChannelName {
            channel_id: private,
            name: ambre_core::Validated::<ambre_core::Name>::parse("Design").unwrap(),
        }];
        let listed = ambre_bus::ChannelMeta {
            channel_id: private,
            meta,
            last: Some(ambre_core::Seq::<ambre_core::Relay>::new(6)),
            call: None,
        };
        let key = DocumentKey::generate();
        let listing = |(channels, spaces, peers), names, keys| MemberListing {
            channels,
            spaces,
            names,
            peers,
            keys,
        };
        let empty = (Vec::new(), Vec::new(), Vec::new());
        state.member_channels_listed(
            ambre_core::Seq::nth(1),
            Ok(listing(empty, Vec::new(), Vec::new())),
        );
        let keys = vec![(listed, key.clone())];
        state.member_channels_listed(
            ambre_core::Seq::nth(2),
            Ok(listing(answer.listing(), named.clone(), keys)),
        );
        state.member_channels_listed(
            ambre_core::Seq::nth(2),
            Err(crate::http::HttpError::Local(
                ambre_core::LocalFailure::NoRelay,
            )),
        );
        let events: Vec<_> = rx
            .try_iter()
            .filter_map(|e| match e {
                LogicEvent::MemberChannels {
                    channels,
                    spaces,
                    names,
                    peers,
                    metas,
                } => {
                    assert_eq!(metas, vec![listed], "the watchable meta of each channel");
                    Some((channels, spaces, names, peers))
                }
                _ => None,
            })
            .collect();
        let kept = state.listed_keys.get(&private).map(|(m, k)| (*m, k.id()));
        assert_eq!(kept, Some((meta, key.id())), "the listed key is kept");
        // A channel of a space composes no name, whatever the relay lists.
        let peers = vec![ambre_bus::DirectPeers {
            channel_id: direct,
            names: vec![
                ambre_core::Validated::<ambre_core::DisplayName>::parse("Bob").unwrap(),
                ambre_core::Validated::<ambre_core::DisplayName>::parse("Carol").unwrap(),
            ],
        }];
        let expected = vec![
            MemberChannel {
                channel_id: public,
                place: ambre_bus::ChannelPlace::Public(space),
            },
            MemberChannel {
                channel_id: private,
                place: ambre_bus::ChannelPlace::Private(space),
            },
            MemberChannel {
                channel_id: direct,
                place: ambre_bus::ChannelPlace::Direct,
            },
        ];
        assert_eq!(
            events,
            vec![(expected, vec![space, bare], named, peers)],
            "the latest listing alone"
        );
    }

    /// The live call the listing names for a channel reaches the Logic thread with its `meta`
    /// (`CALL-162`). So a channel never opened shows a call that started before its watch.
    #[test]
    fn the_live_call_of_a_listed_channel_rides_its_meta() {
        let own = exchange_key(1);
        let own_pub = crate::sharing::own_exchange_key(&own);
        let key = DocumentKey::generate();
        let (meta, call) = (Id::<Meta>::mint(), Id::<ambre_core::id::Call>::mint());
        let json = serde_json::json!({
            "channels": [{
                "channel_id": Id::<Channel>::mint().to_string(),
                "space_id": null,
                "public": false,
                "meta_id": meta.to_string(),
                "doc_key_enc": ambre_crypto::ecdh::wrap_document_key(
                    &own, &own_pub, meta.document_id(), &key,
                ).unwrap(),
                "wrapper_exchange_pub_key": null,
                "call": call.to_string(),
            }],
            "spaces": [],
        });
        let answer: http::MemberChannelsResponse = serde_json::from_value(json).unwrap();
        let listed = listed_keys(&answer, &own);
        assert_eq!(
            listed.iter().map(|(m, _)| m.call).collect::<Vec<_>>(),
            [Some(call)]
        );
    }

    /// `SYNC-171`: each name the listing carries opens under the user's
    /// wrapped key of its meta. A name sealed for another document opens to
    /// none, and so does an empty one. A meta never named shows none.
    #[tokio::test]
    async fn a_listed_name_opens_under_the_users_key_of_its_meta() {
        let mut state = SyncState::new(
            LogicSink::channel().0,
            crate::http::Url::<crate::RelayHost>::new(String::new()),
        );
        state.signing_key =
            Some(ambre_crypto::MasterSeed::from_array([5; 32]).derive_signing_key());
        let own = exchange_key(1);
        let own_pub = crate::sharing::own_exchange_key(&own);
        let key = DocumentKey::generate();
        // Each copy binds the meta whose rows carry it (audit `P59`).
        let doc_key_enc = |meta: Id<Meta>| {
            ambre_crypto::ecdh::wrap_document_key(&own, &own_pub, meta.document_id(), &key).unwrap()
        };
        let entry =
            |meta: Id<Meta>, name_enc: Option<ambre_core::Encoded<ambre_core::SealedName>>| {
                http::MemberChannelEntry {
                    channel_id: Id::<Channel>::mint(),
                    space_id: None,
                    visibility: ambre_core::Visibility::Private,
                    meta_id: meta,
                    name_enc,
                    doc_key_enc: doc_key_enc(meta),
                    wrapper_exchange_pub_key: None,
                    peers: Vec::new(),
                    meta_last: None,
                    call: None,
                }
            };
        let (named, elsewhere, cleared, bare) = (
            Id::<Meta>::mint(),
            Id::<Meta>::mint(),
            Id::<Meta>::mint(),
            Id::<Meta>::mint(),
        );
        let seal = |meta, name: &str| {
            Some(
                state
                    .seal_channel_name(
                        meta,
                        &key,
                        &ambre_core::Validated::<ambre_core::Name>::parse(name).unwrap(),
                    )
                    .unwrap(),
            )
        };
        let answer = http::MemberChannelsResponse {
            channels: vec![
                entry(named, seal(named, "Design")),
                entry(elsewhere, seal(Id::<Meta>::mint(), "Forged")),
                entry(cleared, seal(cleared, "")),
                entry(bare, None),
            ],
            spaces: Vec::new(),
        };
        // Every meta's log accepted `key` at an open (`SYNC-291`).
        let kept: HashMap<Id<Document>, FixedBytes<KeyId>> = [named, elsewhere, cleared, bare]
            .into_iter()
            .map(|meta| (meta.document_id(), key.id()))
            .collect();
        let kept = crate::documents::KeptNames {
            keys: kept,
            chains: HashMap::new(),
        };
        // No log of a listed meta was walked: no name shows (`SYNC-135`).
        let unwalked = open_channel_names(&answer, &state.name_authors(), &own, &kept);
        assert!(unwalked.is_empty(), "{unwalked:?}");
        for meta in [named, elsewhere, cleared] {
            state.walked_writers.insert(
                meta.document_id(),
                crate::membership_log::Writers::Unchecked,
            );
        }
        let authors = state.name_authors();
        let names = open_channel_names(&answer, &authors, &own, &kept);
        assert_eq!(
            names,
            vec![ChannelName {
                channel_id: answer.channels[0].channel_id,
                name: ambre_core::Validated::<ambre_core::Name>::parse("Design").unwrap(),
            }]
        );
        // `SYNC-291`: a meta this client never walked shows no name, nor
        // one whose log accepted another key. The relay can wrap a key of
        // its own, so the copy alone names nothing.
        assert!(
            open_channel_names(
                &answer,
                &authors,
                &own,
                &crate::documents::KeptNames::default()
            )
            .is_empty()
        );
        let moved = crate::documents::KeptNames {
            keys: [(named.document_id(), DocumentKey::generate().id())].into(),
            chains: HashMap::new(),
        };
        assert!(open_channel_names(&answer, &authors, &own, &moved).is_empty());
        // Each key opens, and one that does not is left out.
        // Each keeps the last number of its meta, none where none is listed.
        let mut answer = answer;
        answer.channels[3].doc_key_enc = ambre_core::Encoded::new(vec![0]);
        answer.channels[0].meta_last = Some(ambre_core::Seq::<ambre_core::Relay>::new(6));
        let keys: Vec<_> = listed_keys(&answer, &own)
            .into_iter()
            .map(|(listed, opened)| (listed.meta, listed.last, opened.id()))
            .collect();
        let held = [
            (named, Some(ambre_core::Seq::<ambre_core::Relay>::new(6))),
            (elsewhere, None),
            (cleared, None),
        ]
        .map(|(meta, last)| (meta, last, key.id()));
        assert_eq!(keys, held);
    }

    /// `SYNC-171`: the name a new private channel carries rides sealed beside
    /// its meta, under the channel key, and only meta opens it.
    #[test]
    fn a_channel_name_is_sealed_under_the_channel_key_for_its_meta() {
        let mut state = SyncState::new(
            LogicSink::channel().0,
            crate::http::Url::<crate::RelayHost>::new(String::new()),
        );
        let (meta, key) = (Id::<Meta>::mint(), DocumentKey::generate());
        let design = ambre_core::Validated::<ambre_core::Name>::parse("Design").unwrap();
        assert!(
            state.seal_channel_name(meta, &key, &design).is_err(),
            "locked"
        );
        state.signing_key =
            Some(ambre_crypto::MasterSeed::from_array([5; 32]).derive_signing_key());
        let sealed = state.seal_channel_name(meta, &key, &design).unwrap();
        let bytes = sealed.as_bytes().to_vec();
        assert!(!bytes.windows(6).any(|w| w == b"Design"));
        let open = |document_id| {
            crate::documents::open_name(
                document_id,
                &key,
                &sealed,
                Vec::new(),
                &crate::documents::NameAuthors::unchecked(document_id),
            )
        };
        assert_eq!(
            open(meta.document_id())
                .as_ref()
                .map(ambre_core::Validated::as_str),
            Some("Design")
        );
        assert_eq!(open(Id::<Document>::mint()), None);
    }

    /// `SYNC-318`: a member that only reads meta seals no name beside it,
    /// whatever the name, and a writer of the same meta seals it.
    #[test]
    fn a_reader_of_meta_seals_no_channel_name() {
        let (mut state, _rx, _events, meta) = gated_meta(false);
        let held = state.documents.get_mut(&meta.document_id()).unwrap();
        held.gate = LogGate::Open;
        held.role = Role::Viewer;
        let design = ambre_core::Validated::<ambre_core::Name>::parse("Design").unwrap();
        assert!(matches!(
            state.sealed_channel_name(meta, &design),
            Err(SyncError::NameRefused)
        ));
        state.documents.get_mut(&meta.document_id()).unwrap().role = Role::Editor;
        assert!(state.sealed_channel_name(meta, &design).is_ok());
    }

    /// `SYNC-170`: a space this client creates is marked until the relay
    /// holds it, and its lazy creation tells the relay it is a space. Any
    /// other document says nothing of it, and a closed space drops its mark.
    #[test]
    fn a_new_space_is_created_as_a_space() {
        let (sink, _rx) = LogicSink::channel();
        let mut state = SyncState::new(
            sink,
            crate::http::Url::<crate::RelayHost>::new(String::new()),
        );
        let (space, gone) = (Id::<Space>::mint(), Id::<Space>::mint());
        let other = Id::<Document>::mint();
        state.handle_command(SyncCommand::CreateSpace { space_id: space });
        state.handle_command(SyncCommand::CreateSpace { space_id: gone });
        state.handle_command(SyncCommand::OpenDocument {
            document_id: other,
            doc_type: DocType::Murmure,
            meta: None,
        });
        state.handle_command(SyncCommand::CloseDocument {
            document_id: gone.document_id(),
        });
        let body = |state: &SyncState, document_id: Id<Document>| {
            let request = http::CreateDocumentRequest {
                doc_type: DocType::Murmure,
                name_enc: None,
                doc_key_enc: ambre_core::Encoded::new(Vec::new()),
                document_id: Some(document_id),
                entry: Some(ambre_core::Encoded::new(Vec::new())),
                space: if state.new_spaces.contains(&document_id) {
                    ambre_core::DocumentScope::Space
                } else {
                    ambre_core::DocumentScope::Personal
                },
            };
            serde_json::to_value(request).expect("encodes")
        };
        assert_eq!(body(&state, space.document_id())["space"], true);
        assert!(body(&state, other).get("space").is_none(), "not a space");
        assert!(
            body(&state, gone.document_id()).get("space").is_none(),
            "closed"
        );
        state.handle_sync_event(SyncEvent::DocumentMaterialized {
            document_id: space.document_id(),
        });
        assert!(state.new_spaces.is_empty(), "the relay holds it");
    }

    /// A meta a catalogue lists walks under the word of its open space. Any
    /// other log walks alone (the Murmure spaces spec).
    #[test]
    fn a_listed_meta_walks_under_the_word_of_its_open_space() {
        let (sink, _rx) = LogicSink::channel();
        let mut state = SyncState::new(
            sink,
            crate::http::Url::<crate::RelayHost>::new(String::new()),
        );
        let (space_id, meta) = (Id::<Space>::mint(), Id::<Document>::mint());
        let document_id = space_id.document_id();
        let mut space = SyncDocumentState::new(
            document_id,
            DocumentKey::generate(),
            DocType::Murmure,
            crate::Existence::OnRelay,
        );
        space.word.removed.insert(Id::<User>::mint());
        let word = space.word.clone();
        state.documents.insert(document_id, space);
        assert_eq!(state.log_rule(meta).space, None);
        state.self_join_logs.insert(meta, space_id);
        assert_eq!(state.log_rule(meta).space, Some(word));
    }

    fn bare_state() -> (SyncState, ambre_bus::conduit::Receiver<LogicEvent>) {
        let (sink, rx) = LogicSink::channel();
        let state = SyncState::new(
            sink,
            crate::http::Url::<crate::RelayHost>::new(String::new()),
        );
        (state, rx)
    }

    /// `SYNC-269`: with a relay, a move of the read cursors waits in the
    /// cursors owed until its period ends. Each document waits at the latest
    /// number it asked.
    #[test]
    fn a_move_of_the_read_cursors_waits_for_its_period() {
        let (mut state, _rx) = bare_state();
        let (tx, _events) = ambre_bus::conduit::unbounded::<SyncEvent>();
        state.set_event_tx(tx);
        let (one, two) = (Id::<Segment>::mint(), Id::<Segment>::mint());
        state.handle_move_read_cursors(vec![(one, Seq::<Relay>::new(3))]);
        state.handle_move_read_cursors(vec![
            (one, Seq::<Relay>::new(5)),
            (two, Seq::<Relay>::new(2)),
        ]);
        let mut owed = state.owed_cursors.take();
        owed.sort_by_key(|(_, seq)| *seq);
        assert_eq!(
            owed,
            vec![
                (two.document_id(), Seq::<Relay>::new(2)),
                (one.document_id(), Seq::<Relay>::new(5)),
            ]
        );
    }

    /// `SYNC-186`: a request for a new document of a channel answers its
    /// failure at once when no relay is there to take it.
    #[test]
    fn a_channel_document_request_with_no_relay_answers_its_failure_at_once() {
        let (state, rx) = bare_state();
        let (channel_id, meta) = (Id::<Channel>::mint(), Id::<Meta>::mint());
        let (segment, root) = (Id::<Segment>::mint(), Id::<Message>::mint());
        let document = Id::<Document>::mint();
        let no_relay = ApiFailure::Local(ambre_core::LocalFailure::NoRelay);
        let answered = || rx.try_iter().collect::<Vec<LogicEvent>>();
        state.handle_close_segment(channel_id, meta, segment, Id::<Segment>::mint());
        assert!(matches!(
            answered().as_slice(),
            [LogicEvent::SegmentCloseFailed { closing, failure, .. }]
                if *closing == segment && *failure == no_relay
        ));
        state.handle_create_amendments(channel_id, meta, segment, document);
        assert!(matches!(
            answered().as_slice(),
            [LogicEvent::AmendmentsLinkFailed { amendments, failure, .. }]
                if *amendments == document && *failure == no_relay
        ));
        state.handle_create_thread(channel_id, meta, segment, root, document);
        assert!(matches!(
            answered().as_slice(),
            [LogicEvent::ThreadLinkFailed { root: answered_root, failure, .. }]
                if *answered_root == root && *failure == no_relay
        ));
    }

    /// `SYNC-189`: a public channel created while its new space is on
    /// its way to the relay waits for it, then runs. One whose space fails
    /// to reach the relay fails, and the view learns it.
    #[test]
    fn a_public_creation_waits_for_its_space_to_reach_the_relay() {
        let (mut state, rx) = bare_state();
        let (tx, _events) = ambre_bus::conduit::unbounded::<SyncEvent>();
        state.set_event_tx(tx);
        let (space_id, failed_space) = (Id::<Space>::mint(), Id::<Space>::mint());
        for space in [space_id, failed_space] {
            state.documents.insert(
                space.document_id(),
                SyncDocumentState::new(
                    space.document_id(),
                    DocumentKey::generate(),
                    DocType::Murmure,
                    crate::Existence::LocalOnly,
                ),
            );
        }
        let create = |state: &mut SyncState, space_id| {
            let channel_id = Id::<Channel>::mint();
            state.handle_command(SyncCommand::CreateChannel {
                channel_id,
                space_id,
                meta: Id::<Meta>::mint(),
                segment: Id::<Segment>::mint(),
                visibility: ambre_core::Visibility::Public,
                name: None,
            });
            channel_id
        };
        let (waits, refused) = (
            create(&mut state, space_id),
            create(&mut state, failed_space),
        );
        assert_eq!(rx.try_iter().count(), 0, "both wait, and neither fails");

        state.handle_sync_event(SyncEvent::DocumentMaterialized {
            document_id: space_id.document_id(),
        });
        // The creation ran past the space: the next missing piece answers.
        assert!(matches!(
            rx.try_iter().collect::<Vec<_>>().as_slice(),
            [LogicEvent::ChannelRequestFailed { channel_id, failure }]
                if *channel_id == waits
                    && *failure == ApiFailure::Local(ambre_core::LocalFailure::SessionLocked)
        ));

        state.handle_sync_event(SyncEvent::DocumentCreateFailed {
            document_id: failed_space.document_id(),
            failure: http::Failed::local(ambre_core::LocalFailure::NoRelay),
        });
        assert!(matches!(
            rx.try_iter().collect::<Vec<_>>().as_slice(),
            [LogicEvent::ChannelRequestFailed { channel_id, failure }]
                if *channel_id == refused
                    && *failure == ApiFailure::Local(ambre_core::LocalFailure::NotOnRelay)
        ));
        assert!(state.held_creations.is_empty());
    }

    /// `SYNC-190`: the log of a public channel this client created
    /// admits a self-join from the moment the relay holds the channel. A
    /// refused creation, and a private channel, mark no log.
    #[test]
    fn a_created_public_channel_admits_a_self_join_at_once() {
        let (mut state, _rx) = bare_state();
        let (tx, _events) = ambre_bus::conduit::unbounded::<SyncEvent>();
        state.set_event_tx(tx);
        let space_id = Id::<Space>::mint();
        let mut create = |visibility| {
            let (channel_id, meta) = (Id::<Channel>::mint(), Id::<Meta>::mint());
            state.handle_command(SyncCommand::CreateChannel {
                channel_id,
                space_id,
                meta,
                segment: Id::<Segment>::mint(),
                visibility,
                name: None,
            });
            (channel_id, meta.document_id())
        };
        let (public, public_meta) = create(ambre_core::Visibility::Public);
        let (refused, refused_meta) = create(ambre_core::Visibility::Public);
        let (private, private_meta) = create(ambre_core::Visibility::Private);
        for (channel_id, result) in [
            (public, Ok(None)),
            (
                refused,
                Err(crate::http::HttpError::Local(
                    ambre_core::LocalFailure::NoRelay,
                )),
            ),
            (private, Ok(None)),
        ] {
            state.handle_sync_event(SyncEvent::ChannelCreated { channel_id, result });
        }
        assert_eq!(state.self_join_logs.get(&public_meta), Some(&space_id));
        assert!(!state.self_join_logs.contains_key(&refused_meta));
        assert!(!state.self_join_logs.contains_key(&private_meta));
        assert!(state.creating_public.is_empty());
    }

    /// `SYNC-187`: a public channel needs the key of a space the relay
    /// holds, and a private one needs no space at all.
    #[test]
    fn a_public_channel_needs_its_space_on_the_relay_and_a_private_one_does_not() {
        let (mut state, rx) = bare_state();
        let (tx, _events) = ambre_bus::conduit::unbounded::<SyncEvent>();
        state.set_event_tx(tx);
        let space_id = Id::<Space>::mint();
        let create = |state: &SyncState, visibility| {
            let channel_id = Id::<Channel>::mint();
            let ids = (Id::<Meta>::mint(), Id::<Segment>::mint());
            state.handle_create_channel(channel_id, space_id, ids, visibility, None);
            match rx.try_iter().collect::<Vec<_>>().as_slice() {
                [LogicEvent::ChannelRequestFailed { failure, .. }] => *failure,
                other => panic!("one failure expected, got {other:?}"),
            }
        };
        let public = ambre_core::Visibility::Public;
        let private = ambre_core::Visibility::Private;
        assert_eq!(
            create(&state, public),
            ApiFailure::Local(ambre_core::LocalFailure::DocumentNotOpen)
        );
        assert_eq!(
            create(&state, private),
            ApiFailure::Local(ambre_core::LocalFailure::SessionLocked)
        );
        let space = |existence| {
            SyncDocumentState::new(
                space_id.document_id(),
                DocumentKey::generate(),
                DocType::Murmure,
                existence,
            )
        };
        state
            .documents
            .insert(space_id.document_id(), space(crate::Existence::LocalOnly));
        assert_eq!(
            create(&state, public),
            ApiFailure::Local(ambre_core::LocalFailure::NotOnRelay)
        );
        state
            .documents
            .insert(space_id.document_id(), space(crate::Existence::OnRelay));
        assert_eq!(
            create(&state, public),
            ApiFailure::Local(ambre_core::LocalFailure::SessionLocked)
        );
    }

    /// `SYNC-106`: a listing marks the log of its channel's meta as one that
    /// admits a self-join, never a segment's. The mark goes once no
    /// catalogue lists the channel.
    #[test]
    fn a_listing_marks_the_meta_of_a_listed_channel_alone() {
        let (mut state, _rx) = bare_state();
        let (meta, segment) = (Id::<Document>::mint(), Id::<Document>::mint());
        let documents = vec![
            ChannelDocument {
                document_id: meta,
                kind: ChannelDocumentKind::Meta,
            },
            ChannelDocument {
                document_id: segment,
                kind: ChannelDocumentKind::Segment,
            },
        ];
        let (channel_id, space_id) = (Id::<Channel>::mint(), Id::<Space>::mint());
        state.channel_listed(channel_id, Some(space_id), Ok(documents.clone()));
        assert_eq!(state.self_join_logs.get(&meta), Some(&space_id));
        assert!(!state.self_join_logs.contains_key(&segment));
        state.channel_listed(channel_id, None, Ok(documents));
        assert!(state.self_join_logs.is_empty());
    }

    #[test]
    fn with_no_relay_the_open_fails_at_once() {
        let (sink, rx) = LogicSink::channel();
        let state = SyncState::new(
            sink,
            crate::http::Url::<crate::RelayHost>::new(String::new()),
        );
        let channel_id = Id::<Channel>::mint();
        state.handle_open_channel(channel_id, None);
        let events: Vec<LogicEvent> = rx.try_iter().collect();
        assert!(matches!(
            events.as_slice(),
            [LogicEvent::ChannelOpenFailed { channel_id: id, .. }] if *id == channel_id
        ));
    }

    /// The key a member opens from a wrap `sender` made, with the root it
    /// carries.
    fn unwrap_from(
        holder: &StaticSecret,
        sender: &StaticSecret,
        meta: Id<Meta>,
        doc_key_enc: &ambre_core::Encoded<ambre_core::WrappedDocKey>,
    ) -> Option<(DocumentKey, Option<FixedBytes<ambre_protocol::EntryDigest>>)> {
        let sender_pub = crate::sharing::own_exchange_key(sender);
        let (key, root) = ambre_crypto::ecdh::unwrap_anchored(
            holder,
            &sender_pub,
            meta.document_id(),
            doc_key_enc,
        )
        .ok()?;
        Some((key, root))
    }

    fn member_of(exchange_key: &StaticSecret) -> DirectMember {
        DirectMember {
            user_id: Id::<User>::mint(),
            exchange_pub_key: crate::sharing::own_exchange_key(exchange_key),
        }
    }

    /// A direct message's log replays to every member as an editor, the
    /// creator included, and leaves no owner. Each member opens one key
    /// from their own wrap, with the log's root beside it.
    #[test]
    fn a_direct_message_makes_every_member_an_editor_under_one_key() {
        let author = crate::sharing::Author {
            user_id: Id::<User>::mint(),
            signing_key: SigningKey::from_bytes(&[4; 32]),
        };
        let alice = exchange_key(1);
        let (bob, carol) = (exchange_key(2), exchange_key(3));
        let members = [member_of(&bob), member_of(&carol)];
        let invited: Vec<_> = members
            .iter()
            .zip(5u8..)
            .map(|(m, seed)| {
                (
                    m,
                    FixedBytes::from_array(
                        SigningKey::from_bytes(&[seed; 32])
                            .verifying_key()
                            .to_bytes(),
                    ),
                )
            })
            .collect();
        let meta = Id::<Meta>::mint();
        let body = direct_message_request(
            &author,
            &alice,
            Id::<Channel>::mint(),
            (meta, Id::<Segment>::mint()),
            &invited,
        )
        .expect("request");

        assert_eq!(body.entries.len(), members.len() + 2);
        let (own_key, root) =
            unwrap_from(&alice, &alice, meta, &body.doc_key_enc).expect("own wrap");
        assert_eq!(body.key_id, own_key.id());
        let entries = membership_log::decode(&body.entries);
        // `SYNC-123`: the creation and each invite name the key; the role
        // change of the creator installs none.
        for entry in &entries {
            let named =
                (entry.act != ambre_protocol::MembershipAct::ChangeRole).then_some(body.key_id);
            assert_eq!(entry.key, named, "{:?}", entry.act);
        }
        let log = membership_log::Log::walk(
            meta.document_id(),
            root,
            &entries,
            membership_log::no_self_join,
        );
        let everyone = [author.user_id, members[0].user_id, members[1].user_id];
        for user in everyone {
            assert_eq!(
                log.standing(user),
                (ambre_bus::MemberStanding::Verified, Some(Role::Editor)),
                "{user}"
            );
        }

        for (secret, wrap) in [(&bob, &body.members[0]), (&carol, &body.members[1])] {
            let (key, opened_root) =
                unwrap_from(secret, &alice, meta, &wrap.doc_key_enc).expect("member wrap");
            assert_eq!(opened_root, root);
            assert!(same_key(&key, &own_key));
        }
    }

    /// `SYNC-106`: a self-join extends the log the join head names, at its
    /// place and after its last entry, with the role the relay grants. An
    /// empty log takes none.
    #[test]
    fn a_self_join_extends_the_log_the_join_head_names() {
        let author = crate::sharing::Author {
            user_id: Id::<User>::mint(),
            signing_key: SigningKey::from_bytes(&[4; 32]),
        };
        let meta_id = Id::<Meta>::mint();
        let key = DocumentKey::generate().id();
        let mut head = http::JoinHeadResponse {
            meta_id,
            role: Role::Editor,
            length: ambre_core::Seq::ZERO,
            last: None,
        };
        let exchange = FixedBytes::from_array([5; 32]);
        assert_eq!(
            self_join_entry(&author, exchange, &head, key).expect("empty"),
            None
        );

        let owner = SigningKey::from_bytes(&[6; 32]);
        let first = membership_log::sign(
            membership_log::Draft {
                document: meta_id.document_id(),
                member: Id::<User>::mint(),
                member_key: FixedBytes::from_array(owner.verifying_key().to_bytes()),
                member_exchange_key: FixedBytes::from_array([6; 32]),
                act: ambre_protocol::MembershipAct::Create,
                role: Role::Owner,
                key: Some(key),
            },
            membership_log::LogHead::EMPTY,
            Id::<User>::mint(),
            &owner,
        )
        .expect("sign");
        head.length = ambre_core::Seq::new(1);
        head.last = Some(membership_log::encode(&first).expect("encode"));
        let entry = self_join_entry(&author, exchange, &head, key)
            .expect("joined")
            .expect("an entry");
        let joined = membership_log::decode(std::slice::from_ref(&entry));
        let [joined] = joined.as_slice() else {
            panic!("one entry");
        };
        assert_eq!(joined.act, ambre_protocol::MembershipAct::SelfJoin);
        assert_eq!(
            (joined.document, joined.member, joined.role, joined.key),
            (
                meta_id.document_id(),
                author.user_id,
                Role::Editor,
                Some(key)
            )
        );
        assert_eq!(joined.position, head.length);
        assert_eq!(joined.previous, membership_log::digest(&first));
    }

    /// The activity answer reaches the Logic thread as it came; a failed one
    /// sends nothing.
    #[test]
    fn the_activity_answer_reaches_logic() {
        let (state, rx) = bare_state();
        state.activity_fetched(Ok(Vec::new()));
        assert!(matches!(
            rx.try_iter().collect::<Vec<_>>().as_slice(),
            [LogicEvent::ChannelActivity { documents }] if documents.is_empty()
        ));
        state.activity_fetched(Err(crate::http::HttpError::Local(
            ambre_core::LocalFailure::NoRelay,
        )));
        assert_eq!(rx.try_iter().count(), 0);
    }

    /// A member whose key the relay now serves differently is refused: no
    /// wrap leaves under a key the view did not pin. `CHROME-140`, audit
    /// `P44`: a signing key other than the one this session showed, or a
    /// member this session showed nothing for, is refused too.
    #[test]
    fn a_key_other_than_the_pinned_one_is_refused() {
        let bob = member_of(&exchange_key(2));
        let signing = FixedBytes::<ambre_core::PublicKey>::from_array([0x5B; 32]);
        let served = http::PubkeysResponse {
            signing_pubkey: signing,
            exchange_pubkey: bob.exchange_pub_key,
        };
        let shown = ambre_crypto::IdentityKeys {
            exchange: bob.exchange_pub_key,
            signing,
        };
        assert!(check_pinned(&bob, &served, Some(&shown)).is_ok());
        let other = http::PubkeysResponse {
            exchange_pubkey: crate::sharing::own_exchange_key(&exchange_key(3)),
            ..served
        };
        assert!(
            check_pinned(&bob, &other, Some(&shown)).is_err(),
            "another exchange key"
        );
        let swapped = http::PubkeysResponse {
            signing_pubkey: FixedBytes::from_array([0xEE; 32]),
            ..served
        };
        assert!(
            check_pinned(&bob, &swapped, Some(&shown)).is_err(),
            "another signing key"
        );
        assert!(check_pinned(&bob, &served, None).is_err(), "no keys shown");
        // A key that is not hex never reaches the pin: the answer that
        // carries it is refused when it is read.
        let served = serde_json::json!({
            "signing_pubkey": "aa".repeat(32),
            "exchange_pubkey": "not hex",
        });
        assert!(serde_json::from_value::<http::PubkeysResponse>(served).is_err());
    }

    /// `SYNC-180`: a direct message past its budget fails before any request
    /// leaves. One member fewer passes the cap and fails only for the relay
    /// this state lacks.
    #[test]
    fn a_direct_message_past_its_budget_is_refused_before_any_request() {
        let max = ambre_protocol::membership::DIRECT_MESSAGE_MAX_MEMBERS;
        let failure = |others: usize| {
            let (sink, rx) = LogicSink::channel();
            let state = SyncState::new(
                sink,
                crate::http::Url::<crate::RelayHost>::new(String::new()),
            );
            let members = (0..others).map(|_| member_of(&exchange_key(2))).collect();
            state.handle_create_direct_message(
                Id::<Channel>::mint(),
                (Id::<Meta>::mint(), Id::<Segment>::mint()),
                members,
            );
            match rx.try_iter().collect::<Vec<_>>().as_slice() {
                [LogicEvent::ChannelRequestFailed { failure, .. }] => *failure,
                other => panic!("one failure expected, got {other:?}"),
            }
        };
        assert_eq!(
            failure(max - 1),
            ApiFailure::Local(ambre_core::LocalFailure::NoRelay)
        );
        assert_eq!(
            failure(max),
            ApiFailure::Local(ambre_core::LocalFailure::TooManyMembers)
        );
    }

    #[test]
    fn with_no_relay_a_direct_message_fails_at_once() {
        let (sink, rx) = LogicSink::channel();
        let state = SyncState::new(
            sink,
            crate::http::Url::<crate::RelayHost>::new(String::new()),
        );
        let channel_id = Id::<Channel>::mint();
        let members = vec![member_of(&exchange_key(2))];
        state.handle_create_direct_message(
            channel_id,
            (Id::<Meta>::mint(), Id::<Segment>::mint()),
            members,
        );
        let events: Vec<LogicEvent> = rx.try_iter().collect();
        assert!(matches!(
            events.as_slice(),
            [LogicEvent::ChannelRequestFailed { channel_id: id, .. }] if *id == channel_id
        ));
    }

    /// A catalogue entry the build before the typed key id and nonce splits
    /// sealed: `space key id || nonce || ciphertext`. A later build still
    /// names its space key and opens it to the same channel key and anchor.
    #[test]
    fn a_channel_key_the_old_code_sealed_still_names_its_space_key_and_opens() {
        let space_key = DocumentKey::from_array([0x22; 32]);
        let channel_key = DocumentKey::from_array([0x33; 32]);
        let channel_id = Id::<Channel>::from_bytes([0x44; 16]);
        let sealed = SealedChannelKey::from_bytes(
            hex::decode(
                "9479381e0c27f84d6d91c3eecc99fba280ff12c6dbadd55baceaf1a971b717dd\
                 bb8e06ccc1472565a445c65bce964f3b821d5e5186794395e27463cff52a757f\
                 12be02fa8ce015225ff9540c0b1c2c2056f65168431c8105a7133b643b528149\
                 d07d0299074b87ebc5a0c46a35094821d47111cc8215ab7e",
            )
            .unwrap(),
        );
        assert_eq!(sealed_under(&sealed), Some(space_key.id()));
        let (opened, anchor) =
            unseal_channel_key(channel_id, &sealed, &ring(&space_key, &[])).expect("open");
        assert_eq!(opened.id(), channel_key.id());
        assert_eq!(anchor, Some(ANCHOR));
    }

    /// A channel `meta` open here whose log walks again, or ended outside its
    /// space. The state holds a relay to reach and session keys.
    fn gated_meta(
        ended: bool,
    ) -> (
        SyncState,
        ambre_bus::conduit::Receiver<LogicEvent>,
        ambre_bus::conduit::Receiver<SyncEvent>,
        Id<Meta>,
    ) {
        let (mut state, rx) = bare_state();
        let (tx, events) = ambre_bus::conduit::unbounded::<SyncEvent>();
        state.set_event_tx(tx);
        let seed = ambre_crypto::MasterSeed::generate();
        state.signing_key = Some(seed.derive_signing_key());
        state.exchange_key = Some(exchange_key(3));
        let meta = Id::<Meta>::mint();
        let mut doc_state = SyncDocumentState::new(
            meta.document_id(),
            DocumentKey::generate(),
            DocType::Murmure,
            crate::Existence::OnRelay,
        );
        doc_state
            .gate
            .walk(ambre_core::Seq::ZERO, ambre_core::Seq::nth(1));
        if ended {
            doc_state.gate.end();
        }
        state.documents.insert(meta.document_id(), doc_state);
        (state, rx, events, meta)
    }

    /// Holds `SYNC-234` on every request that seals under the key of a
    /// channel document, fail closed. The log walks again, or a walk found it
    /// rooted outside. Each request is then refused by name, and none
    /// leaves. That holds for a media upload, a rename, an invitation, a
    /// revoke, a segment close and a channel name. An owed rotation is not paid.
    #[test]
    fn every_request_sealing_under_an_unsettled_key_is_refused() {
        let refused = ApiFailure::Local(ambre_core::LocalFailure::LogUnsettled);
        for ended in [false, true] {
            let (mut state, rx, events, meta) = gated_meta(ended);
            let doc = meta.document_id();
            let id = Seq::<Request>::new(4);
            state.handle_upload_media(id, doc, b"a picture".to_vec().into());
            state.handle_invite_to_document(
                id,
                doc,
                None,
                Id::<User>::mint(),
                Role::Editor,
                FixedBytes::from_array([7; 32]),
            );
            state.handle_revoke_from_document(id, doc, Id::<User>::mint());
            let (channel_id, closing) = (Id::<Channel>::mint(), Id::<Segment>::mint());
            state.handle_close_segment(channel_id, meta, closing, Id::<Segment>::mint());
            let failures: Vec<_> = rx
                .try_iter()
                .map(|event| match event {
                    LogicEvent::ApiError {
                        request, failure, ..
                    } => (Some(request), failure),
                    LogicEvent::SegmentCloseFailed { failure, .. } => (None, failure),
                    other => panic!("unexpected {other:?}"),
                })
                .collect();
            assert_eq!(
                failures,
                [
                    (Some(ApiRequest::UploadMedia), refused),
                    (Some(ApiRequest::InviteToDocument), refused),
                    (Some(ApiRequest::RevokeFromDocument), refused),
                    (None, refused),
                ]
            );
            state.handle_rename_document(
                id,
                doc,
                ambre_core::Validated::<ambre_core::Name>::parse("x").unwrap(),
            );
            assert!(matches!(
                events.try_iter().collect::<Vec<_>>().as_slice(),
                [SyncEvent::HttpError { failure, .. }] if *failure == refused
            ));
            assert!(matches!(
                state.sealed_channel_name(meta, &ambre_core::Validated::default()),
                Err(SyncError::LogUnsettled)
            ));

            let doc_state = state.documents.get_mut(&doc).unwrap();
            doc_state.rekey.found_debt();
            assert!(doc_state.rekey.pay_now());
            state.start_settle(doc);
            assert!(
                state.documents.get_mut(&doc).unwrap().rekey.pay_now(),
                "the payment failed at once, so the debt stands"
            );
        }
    }

    /// Holds the bounds of `SYNC-234`. A walk holds a snapshot once and up to
    /// [`QUARANTINE_LIMIT`] of them, and its answer hands each one back. No
    /// walk owed defers nothing.
    #[test]
    fn a_walk_defers_its_snapshots_within_its_bound_and_hands_them_back() {
        let mut gate = LogGate::default();
        assert!(matches!(
            gate.defer_snapshot(Id::<Snapshot>::mint(), Seq::<Relay>::new(1)),
            Deferral::Dropped
        ));
        gate.walk(ambre_core::Seq::ZERO, ambre_core::Seq::nth(2));
        let first = Id::<Snapshot>::mint();
        for _ in 0..2 {
            assert!(matches!(
                gate.defer_snapshot(first, Seq::<Relay>::new(1)),
                Deferral::Held
            ));
        }
        for _ in 1..walk_limit() {
            gate.defer_snapshot(Id::<Snapshot>::mint(), Seq::<Relay>::new(1));
        }
        assert!(matches!(
            gate.defer_snapshot(Id::<Snapshot>::mint(), Seq::<Relay>::new(1)),
            Deferral::Overflow
        ));
        assert!(
            gate.release(ambre_core::Seq::nth(1)).is_none(),
            "an older answer"
        );
        let released = gate.release(ambre_core::Seq::nth(2)).unwrap();
        assert_eq!(released.snapshots.len(), walk_limit());
        assert!(released.snapshots.contains_key(&first));
        assert!(matches!(gate, LogGate::Open));
    }

    /// A gate whose log walks again, as the answer to the listing 2.
    fn walking_gate() -> LogGate {
        let mut gate = LogGate::Open;
        gate.walk(ambre_core::Seq::nth(1), ambre_core::Seq::nth(2));
        gate
    }

    fn one_byte_frame(byte: u8) -> pipeline::UpdateFrame {
        pipeline::UpdateFrame::from_bytes(vec![byte])
    }

    /// A walk holds one refused frame per clock, its latest copy, and hands
    /// them back in the order of their clocks.
    #[test]
    fn a_walk_holds_the_latest_refused_frame_of_each_clock() {
        type Clock = ambre_protocol::messages::Seq<ambre_protocol::messages::DeltaAuthor>;
        let mut gate = walking_gate();
        let refused = vec![
            (Clock::new(2), one_byte_frame(1)),
            (Clock::new(1), one_byte_frame(2)),
        ];
        assert!(gate.hold_resend(refused).is_empty());
        assert!(
            gate.hold_resend(vec![(Clock::new(2), one_byte_frame(3))])
                .is_empty()
        );
        let released = gate
            .release(ambre_core::Seq::nth(2))
            .expect("the answer of the walk");
        let held: Vec<(Clock, Vec<u8>)> = released
            .resend
            .iter()
            .map(|(clock, frame)| (*clock, frame.as_bytes().to_vec()))
            .collect();
        assert_eq!(
            held,
            vec![(Clock::new(1), vec![2]), (Clock::new(2), vec![3])]
        );
    }

    /// An ended gate holds no edit again, and no answer opens it.
    #[test]
    fn an_ended_gate_seals_nothing_again() {
        let mut gate = walking_gate();
        gate.end();
        assert!(matches!(gate, LogGate::Ended));
        assert!(matches!(gate.hold(one_byte_frame(1)), Hold::Ended));
        assert!(gate.release(ambre_core::Seq::nth(2)).is_none());
        let mut open = LogGate::Open;
        open.end();
        assert!(matches!(open, LogGate::Ended));
    }

    /// A listing of an open space whose walk sets a new word walks the log of
    /// each channel the space lists again. A listing that carries the word
    /// held walks nothing.
    #[test]
    fn a_space_listing_with_a_new_word_walks_its_channel_logs_again() {
        let (mut state, _rx) = bare_state();
        let (space_id, meta) = (Id::<Space>::mint(), Id::<Document>::mint());
        let space = space_id.document_id();
        for document_id in [space, meta] {
            state.documents.insert(
                document_id,
                SyncDocumentState::new(
                    document_id,
                    DocumentKey::generate(),
                    DocType::Murmure,
                    crate::Existence::OnRelay,
                ),
            );
        }
        state.self_join_logs.insert(meta, space_id);
        let writers = state.documents[&space].writers.clone();
        let listing =
            |asked: u64, word: membership_log::SpaceWord| SyncEvent::DocumentMembersReady {
                id: None,
                asked: ambre_core::Seq::nth(asked),
                document_id: space,
                word,
                rotation_owed: ambre_core::RotationDebt::Settled,
                members: Vec::new(),
                signing_keys: Vec::new(),
                heads: Default::default(),
                writers: writers.clone(),
                departed: None,
                revoked: None,
                root: membership_log::Root::Free,
            };
        let walks = |state: &SyncState| matches!(state.documents[&meta].gate, LogGate::Walking(_));

        state.handle_sync_event(listing(1, membership_log::SpaceWord::default()));
        assert!(!walks(&state), "the word held walks nothing");

        let mut word = membership_log::SpaceWord::default();
        word.removed.insert(Id::<User>::mint());
        state.handle_sync_event(listing(2, word));
        assert!(walks(&state), "a new word walks the channel log again");
    }
}
