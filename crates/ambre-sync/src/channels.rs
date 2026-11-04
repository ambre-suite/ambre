use super::*;
use ambre_bus::{
    ChannelDocument, ChannelDocumentKind, ChannelName, DirectMember, DocumentActivity,
    MemberChannel, SealedChannelKey,
};
use ambre_core::{DocType, FixedBytes};

use crate::error::{Part, SyncError};

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
                message: "no relay on this transport".into(),
            });
            return;
        };
        let api_base = self.api_base.clone();
        let url_scheme = self.url_scheme;
        let auth = self.auth_context();
        wasm_bindgen_futures::spawn_local(async move {
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
                // The channel's members live in meta's log. Mark it as one
                // that admits a self-join exactly when a catalogue lists the
                // channel, and forget the mark when it no longer does.
                let metas = documents
                    .iter()
                    .filter(|d| d.kind == ChannelDocumentKind::Meta)
                    .map(|d| d.document_id);
                for meta in metas {
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
                tracing::info!("Sync: channel {channel_id} refused its listing");
                self.send_to_logic(LogicEvent::ChannelRefused { channel_id });
            }
            Err(error) => {
                tracing::warn!("Sync: listing channel {channel_id} failed: {error}");
                self.send_to_logic(LogicEvent::ChannelOpenFailed {
                    channel_id,
                    message: error.to_string(),
                });
            }
        }
    }

    /// The rule a walk of `document_id`'s log applies. A self-join is valid
    /// in the log of a channel a catalogue lists, and nowhere else
    /// (`SYNC-106`). That log also heeds the word of its space, as the walk
    /// of the open space's log last said it.
    pub(crate) fn log_rule(&self, document_id: Id<Document>) -> membership_log::LogRule {
        match self.self_join_logs.get(&document_id) {
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

    /// The answer to a channel creation or a self-join, on the Sync loop.
    pub(crate) fn channel_answered(
        &self,
        channel_id: Id<Channel>,
        result: Result<LogicEvent, http::HttpError>,
    ) {
        match result {
            Ok(event) => self.send_to_logic(event),
            Err(error) => {
                tracing::warn!("Sync: channel {channel_id} request failed: {error}");
                self.send_to_logic(LogicEvent::ChannelRequestFailed {
                    channel_id,
                    message: error.to_string(),
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
        wasm_bindgen_futures::spawn_local(async move {
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
        let held = self.documents.get(&meta.document_id());
        let sealed = held.ok_or(SyncError::NotOpen).and_then(|held| {
            self.seal_channel_name(meta, &held.doc_key, &name.unwrap_or_default())
        });
        let name_enc = match sealed {
            Ok(name_enc) => name_enc,
            Err(message) => {
                tracing::warn!("Sync: the name of {meta} stays unsealed: {message}");
                return;
            }
        };
        let Some(_tx) = self.event_tx.as_ref() else {
            tracing::warn!("Sync: no relay on this transport, the name of {meta} stays local");
            return;
        };
        let url = crate::http::Url::<crate::http::ApiRoot>::of(self.url_scheme, &self.api_base)
            .route(format!("/documents/{}", meta.document_id()));
        let auth = self.auth_context();
        wasm_bindgen_futures::spawn_local(async move {
            let body = http::RenameDocumentRequest { name_enc };
            if let Err(message) = http::patch_json_auth(&url, &body, auth).await {
                tracing::warn!("Sync: the name of {meta} did not reach the relay: {message}");
            }
        });
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
                let (doc_key_enc, key_id) =
                    join_key(channel_id, sealed, &space_keys, &exchange_key)?;
                Ok((tx, doc_key_enc, key_id, author))
            });
        let (tx, doc_key_enc, key_id, author) = match prepared {
            Ok(prepared) => prepared,
            Err(message) => return self.channel_answered(channel_id, Err(message.into())),
        };
        let api_base = self.api_base.clone();
        let url_scheme = self.url_scheme;
        let auth = self.auth_context();
        wasm_bindgen_futures::spawn_local(async move {
            let url = crate::http::Url::<crate::http::ApiRoot>::of(url_scheme, &api_base)
                .route(format!("/channels/{channel_id}/join"));
            let result = async {
                let head =
                    http::get_json_auth::<http::JoinHeadResponse>(&url, auth.clone()).await?;
                let entry = self_join_entry(&author, &head, key_id)?;
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
            Err(message) => {
                return self.send_to_logic(LogicEvent::SegmentCloseFailed {
                    channel_id,
                    closing,
                    message: message.to_string(),
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
        wasm_bindgen_futures::spawn_local(async move {
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
        let meta_key = self
            .documents
            .get(&meta.document_id())
            .map(|doc| doc.doc_key.clone())
            .ok_or(SyncError::NotOpen)?;
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
            Err(message) => {
                return self.send_to_logic(LogicEvent::AmendmentsLinkFailed {
                    channel_id,
                    segment,
                    amendments,
                    message: message.to_string(),
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
        wasm_bindgen_futures::spawn_local(async move {
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
            Err(message) => {
                return self.send_to_logic(LogicEvent::ThreadLinkFailed {
                    channel_id,
                    segment,
                    root,
                    thread,
                    message: message.to_string(),
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
        wasm_bindgen_futures::spawn_local(async move {
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
        wasm_bindgen_futures::spawn_local(async move {
            let url = crate::http::Url::<crate::http::ApiRoot>::of(url_scheme, &api_base)
                .route(format!("/channels/{channel_id}"));
            let result = http::delete_auth(&url, auth).await;
            let _ = tx.send(SyncEvent::ChannelDeleted { channel_id, result });
        });
    }

    /// Ask the relay to remove this user from a channel. The documents stay
    /// open here until the Logic thread hears the answer and closes them. A
    /// refusal leaves the channel whole, as with a delete.
    pub(crate) fn handle_leave_channel(&self, channel_id: Id<Channel>) {
        let Some(tx) = self.event_tx.clone() else {
            return self.channel_answered(channel_id, Err(SyncError::NoRelay.into()));
        };
        let (Some(author), Some(exchange_key)) = (self.author(), self.exchange_key.clone()) else {
            return self.channel_answered(
                channel_id,
                Err(crate::http::HttpError::Local("not signed in".into())),
            );
        };
        let params = crate::sharing::ChannelLeaveParams {
            channel_id,
            author,
            url_scheme: self.url_scheme,
            api_base: self.api_base.clone(),
            exchange_key,
        };
        let http = crate::sharing::GlooHttp {
            auth: self.auth_context(),
        };
        wasm_bindgen_futures::spawn_local(async move {
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
        let api_base = self.api_base.clone();
        let url_scheme = self.url_scheme;
        let auth = self.auth_context();
        wasm_bindgen_futures::spawn_local(async move {
            let base = crate::http::Url::<crate::http::ApiRoot>::of(url_scheme, &api_base);
            let result = async {
                let mut signing_keys = Vec::with_capacity(members.len());
                for member in &members {
                    let url = base.route(format!("/users/{}/pubkeys", member.user_id));
                    let served =
                        http::get_json_auth::<http::PubkeysResponse>(&url, auth.clone()).await?;
                    check_pinned(member, &served.exchange_pubkey)?;
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
        (doc.last_seq.get() > 0 && doc.quarantine_len() == 0).then_some(doc.last_seq)
    }

    /// Move the cursors of one channel to its latest message (the Murmure
    /// unread spec). Each live segment moves to what this worker delivered.
    /// Each final segment no longer changes, so it moves to the relay's last
    /// number.
    pub(crate) fn handle_mark_read(&self, live: &[Id<Segment>], finals: Vec<Id<Segment>>) {
        let cursors: Vec<(Id<Document>, Seq<Relay>)> = live
            .iter()
            .map(Id::<Segment>::document_id)
            .filter_map(|document_id| self.read_up_to(document_id).map(|seq| (document_id, seq)))
            .collect();
        if self.event_tx.is_none() {
            tracing::warn!("Sync: no relay on this transport, the read cursors stay");
            return;
        }
        let api_base = self.api_base.clone();
        let url_scheme = self.url_scheme;
        let auth = self.auth_context();
        wasm_bindgen_futures::spawn_local(async move {
            let base = crate::http::Url::<crate::http::ApiRoot>::of(url_scheme, &api_base);
            put_cursors(&base, cursors, auth.clone()).await;
            if !finals.is_empty() {
                let url = crate::http::Url::<crate::http::ApiRoot>::of(url_scheme, &api_base)
                    .route("/read-cursors/mark-read");
                let body = http::MarkReadRequest {
                    documents: Some(finals.iter().map(Id::<Segment>::document_id).collect()),
                };
                if let Err(e) = http::post_json_auth_ok(&url, &body, auth).await {
                    tracing::warn!("Sync: read cursors of the final segments not moved: {e}");
                }
            }
        });
    }

    /// Move each listed cursor to the number Logic chose, back as well as
    /// forward: marking a message unread (the Murmure unread spec). No other
    /// cursor moves.
    pub(crate) fn handle_move_read_cursors(&self, cursors: Vec<(Id<Segment>, Seq<Relay>)>) {
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
    pub(crate) fn handle_mark_thread_read(&self, thread: Id<Document>) {
        match self.read_up_to(thread) {
            Some(seq) => self.move_cursors(vec![(thread, seq)]),
            None => tracing::warn!("Sync: thread {thread} has nothing delivered, its cursor stays"),
        }
    }

    fn move_cursors(&self, cursors: Vec<(Id<Document>, Seq<Relay>)>) {
        if self.event_tx.is_none() {
            tracing::warn!("Sync: no relay on this transport, the read cursors stay");
            return;
        }
        let base = crate::http::Url::<crate::http::ApiRoot>::of(self.url_scheme, &self.api_base);
        let auth = self.auth_context();
        wasm_bindgen_futures::spawn_local(async move {
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
            tracing::warn!("Sync: no relay on this transport, no activity answer");
            return;
        };
        let api_base = self.api_base.clone();
        let url_scheme = self.url_scheme;
        let auth = self.auth_context();
        wasm_bindgen_futures::spawn_local(async move {
            if fetch == ActivityFetch::MarkAllReadFirst {
                let url = crate::http::Url::<crate::http::ApiRoot>::of(url_scheme, &api_base)
                    .route("/read-cursors/mark-read");
                let body = http::MarkReadRequest { documents: None };
                if let Err(e) = http::post_json_auth_ok(&url, &body, auth.clone()).await {
                    tracing::warn!("Sync: mark all read failed: {e}");
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
            Err(message) => tracing::warn!("Sync: activity answer failed: {message}"),
        }
    }

    /// List the channels the relay holds as public in a space (`SRV-189`),
    /// so a space admin can drop an entry whose channel is gone.
    pub(crate) fn handle_list_space_channels(&self, space_id: Id<Space>) {
        let Some(tx) = self.event_tx.clone() else {
            tracing::warn!("Sync: no relay on this transport, no channels of a space");
            return;
        };
        let api_base = self.api_base.clone();
        let url_scheme = self.url_scheme;
        let auth = self.auth_context();
        wasm_bindgen_futures::spawn_local(async move {
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
                tracing::warn!("Sync: listing the channels of space {space_id} failed: {message}");
            }
        }
    }

    /// List every channel this user is a member of (`SRV-191`), at start and
    /// on each share of a Murmure document. That share is how the relay tells
    /// a member a channel added them (`SRV-192`).
    pub(crate) fn handle_list_member_channels(&mut self) {
        let asked = self.member_channels_asked.issue();
        let Some(tx) = self.event_tx.clone() else {
            tracing::warn!("Sync: no relay on this transport, no channels of the user");
            return;
        };
        let api_base = self.api_base.clone();
        let url_scheme = self.url_scheme;
        let auth = self.auth_context();
        let exchange_key = self.exchange_key.clone();
        wasm_bindgen_futures::spawn_local(async move {
            let url = crate::http::Url::<crate::http::ApiRoot>::of(url_scheme, &api_base)
                .route("/channels");
            let result =
                match http::get_json_auth::<http::MemberChannelsResponse>(&url, auth.clone()).await
                {
                    Ok(answer) => {
                        let names = match exchange_key.as_ref() {
                            Some(key) => {
                                open_channel_names(&answer, key, url_scheme, &api_base, &auth).await
                            }
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
                tracing::warn!("Sync: listing the channels of the user failed: {message}");
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
            )?;
            let listed = ambre_bus::ChannelMeta {
                channel_id: entry.channel_id,
                meta: entry.meta_id,
                last: entry.meta_last,
            };
            Some((listed, key))
        })
        .collect()
}

/// The names sealed beside each meta of `answer`, opened with the user's
/// wrapped key of that meta (`SYNC-171`). A name that does not open is
/// none, and so is an empty one: a rename clears a name that way.
async fn open_channel_names(
    answer: &http::MemberChannelsResponse,
    exchange_key: &StaticSecret,
    url_scheme: crate::http::UrlScheme,
    api_base: &crate::http::Url<crate::http::RelayHost>,
    auth: &http::AuthContext,
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
        };
        match listed.open(exchange_key, url_scheme, api_base, auth).await {
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
            tracing::warn!("Sync: read cursor of {document_id} not moved: {e}");
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
/// meta's log from the word of the member who created the channel.
pub(crate) fn join_key(
    channel_id: Id<Channel>,
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
    let (key, anchor) = unseal_channel_key(channel_id, sealed, space_keys)?;
    let own_pub = crate::sharing::own_exchange_key(exchange_key);
    let wrapped = crate::sharing::wrap_anchored_b64(exchange_key, &own_pub, &key, anchor)?;
    Ok((wrapped, key.id()))
}

/// The `SelfJoin` entry that extends the log the join head names, at the
/// role it grants (`SYNC-106`). `None` for an empty log: it predates the
/// log, and no entry of it could link to a first one.
pub(crate) fn self_join_entry(
    author: &crate::sharing::Author,
    head: &http::JoinHeadResponse,
    key: FixedBytes<KeyId>,
) -> Result<Option<ambre_core::Encoded<ambre_core::EncodedLogEntry>>, SyncError> {
    if head.length.is_first() {
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
    let signing_keys: HashMap<Id<User>, FixedBytes<ambre_core::PublicKey>> = members
        .iter()
        .map(|(m, signing)| (m.user_id, *signing))
        .chain([(author.user_id, own_key)])
        .collect();
    let mut next = membership_log::LogHead::EMPTY;
    let mut root = None;
    let mut entries = Vec::with_capacity(others.len().saturating_add(2));
    for (member, act, role) in ambre_protocol::ownerless_log(author.user_id, &others) {
        let draft = membership_log::Draft {
            document,
            member,
            member_key: signing_keys
                .get(&member)
                .copied()
                .ok_or(SyncError::MemberUnsigned)?,
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
        crate::sharing::wrap_anchored_b64(exchange_key, peer, &key, Some(root))
    };
    let own_pub = crate::sharing::own_exchange_key(exchange_key);
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

/// Refuse a member whose exchange key, as the relay serves it now, is not
/// the one the view pinned for them.
pub(crate) fn check_pinned(
    member: &DirectMember,
    served: &FixedBytes<ambre_core::ExchangePublicKey>,
) -> Result<(), SyncError> {
    if *served == member.exchange_pub_key {
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

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
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

    /// The key a wrapped value opens to, for its holder.
    fn unwrap_own(
        exchange_key: &StaticSecret,
        doc_key_enc: &ambre_core::Encoded<ambre_core::WrappedDocKey>,
    ) -> Option<DocumentKey> {
        let bytes = doc_key_enc.as_bytes();
        let (nonce, ciphertext) = FixedBytes::<ambre_core::Nonce>::split_from(bytes)?;
        let wrapped = ambre_crypto::ecdh::WrappedKey {
            nonce,
            ciphertext: Payload::<ambre_core::Ciphertext>::new(ciphertext.to_vec()),
        };
        let own_pub = crate::sharing::own_exchange_key(exchange_key);
        let (key, anchor) =
            ambre_crypto::ecdh::unwrap_anchored(exchange_key, &own_pub, &wrapped).ok()?;
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
        let own = crate::sharing::wrap_anchored_b64(creator, &own_pub, &channel_key, Some(ANCHOR))
            .expect("wrap");
        (sealed, own)
    }

    /// The keys of a space whose current key is `current`, after rotations
    /// that replaced each of `previous`.
    pub(crate) fn ring(current: &DocumentKey, previous: &[&DocumentKey]) -> crate::chain::Keyring {
        crate::chain::Keyring {
            document_id: Id::<Document>::mint(),
            current: current.clone(),
            previous: previous.iter().map(|&key| key.clone()).collect(),
            chain: crate::chain::Chain::default(),
        }
    }

    /// Whether two keys are one: what one seals, the other opens.
    fn same_key(a: &DocumentKey, b: &DocumentKey) -> bool {
        let (nonce, ct) =
            ambre_crypto::aead::seal(b"probe", &ambre_core::Aad::of(b""), a).expect("seal");
        ambre_crypto::aead::open(&nonce, &ct, &ambre_core::Aad::of(b""), b).is_ok()
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
            join_key(channel_id, &sealed, &ring(&space_key, &[]), &bob).expect("join");
        assert_eq!(key_id, channel_key.id());
        let bob_key = unwrap_own(&bob, &bob_enc).expect("Bob opens his wrap");
        assert!(same_key(&bob_key, &channel_key));
        assert!(
            unwrap_own(&eve, &bob_enc).is_none(),
            "the wrap is Bob's alone"
        );
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
            FixedBytes::<KeyId>::SIZE.as_len()
                + FixedBytes::<ambre_core::Nonce>::SIZE.as_len()
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
            result: Err(crate::http::HttpError::Local("403".into())),
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

        state.handle_leave_channel(channel_id);
        state.handle_sync_event(SyncEvent::ChannelLeft {
            channel_id,
            result: Err(crate::http::HttpError::Local("409".into())),
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
            Err(crate::http::HttpError::Local("HTTP 500".into())),
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
                vec![0; 72]
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
            Err(crate::http::HttpError::Local("HTTP 500".into())),
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
        let wrapped = ambre_crypto::ecdh::wrap_document_key(&own, &own_pub, &key).unwrap();
        let doc_key_enc = ambre_core::Encoded::new(
            [wrapped.nonce.as_slice(), wrapped.ciphertext.as_bytes()].concat(),
        );
        let entry =
            |meta: Id<Meta>, name_enc: Option<ambre_core::Encoded<ambre_core::SealedName>>| {
                http::MemberChannelEntry {
                    channel_id: Id::<Channel>::mint(),
                    space_id: None,
                    visibility: ambre_core::Visibility::Private,
                    meta_id: meta,
                    name_enc,
                    doc_key_enc: doc_key_enc.clone(),
                    wrapper_exchange_pub_key: None,
                    peers: Vec::new(),
                    meta_last: None,
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
        let auth = state.auth_context();
        let names = open_channel_names(
            &answer,
            &own,
            crate::http::UrlScheme::Http,
            &crate::http::Url::<crate::http::RelayHost>::new(String::new()),
            &auth,
        )
        .await;
        assert_eq!(
            names,
            vec![ChannelName {
                channel_id: answer.channels[0].channel_id,
                name: ambre_core::Validated::<ambre_core::Name>::parse("Design").unwrap(),
            }]
        );
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
        let open =
            |document_id| crate::documents::open_name(document_id, &key, &sealed, Vec::new());
        assert_eq!(
            open(meta.document_id())
                .as_ref()
                .map(ambre_core::Validated::as_str),
            Some("Design")
        );
        assert_eq!(open(Id::<Document>::mint()), None);
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
        doc_key_enc: &ambre_core::Encoded<ambre_core::WrappedDocKey>,
    ) -> Option<(DocumentKey, Option<FixedBytes<ambre_protocol::EntryDigest>>)> {
        let bytes = doc_key_enc.as_bytes();
        let (nonce, ciphertext) = FixedBytes::<ambre_core::Nonce>::split_from(bytes)?;
        let wrapped = ambre_crypto::ecdh::WrappedKey {
            nonce,
            ciphertext: Payload::<ambre_core::Ciphertext>::new(ciphertext.to_vec()),
        };
        let sender_pub = crate::sharing::own_exchange_key(sender);
        let (key, root) =
            ambre_crypto::ecdh::unwrap_anchored(holder, &sender_pub, &wrapped).ok()?;
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
        let (own_key, root) = unwrap_from(&alice, &alice, &body.doc_key_enc).expect("own wrap");
        assert_eq!(body.key_id, own_key.id());
        let entries = membership_log::decode(&body.entries);
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
                unwrap_from(secret, &alice, &wrap.doc_key_enc).expect("member wrap");
            assert_eq!(opened_root, root);
            assert!(same_key(&key, &own_key));
        }
    }

    /// A member whose key the relay now serves differently is refused: no
    /// wrap leaves under a key the view did not pin.
    #[test]
    fn a_key_other_than_the_pinned_one_is_refused() {
        let bob = member_of(&exchange_key(2));
        assert!(check_pinned(&bob, &bob.exchange_pub_key).is_ok());
        let other = crate::sharing::own_exchange_key(&exchange_key(3));
        assert!(check_pinned(&bob, &other).is_err());
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
                [LogicEvent::ChannelRequestFailed { message, .. }] => message.clone(),
                other => panic!("one failure expected, got {other:?}"),
            }
        };
        assert_eq!(failure(max - 1), SyncError::NoRelay.to_string());
        assert_eq!(failure(max), SyncError::TooManyMembers.to_string());
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
}
