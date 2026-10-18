use super::*;
use ambre_bus::{
    ChannelDocument, ChannelDocumentKind, DirectMember, DocumentActivity, MemberChannel, MetaId,
    SealedChannelKey, SegmentId, SpaceId,
};
use ambre_core::DocType;
use ambre_crypto::aead::NONCE_LEN;

impl SyncState {
    // -- Channels --------------------------------------------------------------

    /// List the documents of a channel, so the Logic thread can open them.
    ///
    /// A channel travels as its documents (`SRV-102`): this is its only
    /// request of its own. The relay answers the same refusal to an unknown
    /// channel and to a caller who is no member of it (`SRV-047`).
    /// `listed_in` names the space whose catalogue lists the channel; the
    /// listing answer then records which log admits a self-join (`SYNC-106`).
    pub(crate) fn handle_open_channel(&self, channel_id: ChannelId, listed_in: Option<SpaceId>) {
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
            let url = format!("{url_scheme}://{api_base}/api/channels/{channel_id}/documents");
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
        channel_id: ChannelId,
        listed_in: Option<SpaceId>,
        result: Result<Vec<ChannelDocument>, String>,
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
            Err(message) if http::is_forbidden(&message) => {
                // No member of the channel any more: a leave or a delete
                // whose answer was lost, or one from another client.
                tracing::info!("Sync: channel {channel_id} refused its listing");
                self.send_to_logic(LogicEvent::ChannelRefused { channel_id });
            }
            Err(message) => {
                tracing::warn!("Sync: listing channel {channel_id} failed: {message}");
                self.send_to_logic(LogicEvent::ChannelOpenFailed {
                    channel_id,
                    message,
                });
            }
        }
    }

    /// The rule a walk of `document_id`'s log applies. A self-join is valid
    /// in the log of a channel a catalogue lists, and nowhere else
    /// (`SYNC-106`). That log also heeds the word of its space, as the walk
    /// of the open space's log last said it.
    pub(crate) fn log_rule(&self, document_id: DocumentId) -> membership_log::LogRule {
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
        channel_id: ChannelId,
        result: Result<LogicEvent, String>,
    ) {
        match result {
            Ok(event) => self.send_to_logic(event),
            Err(message) => {
                tracing::warn!("Sync: channel {channel_id} request failed: {message}");
                self.send_to_logic(LogicEvent::ChannelRequestFailed {
                    channel_id,
                    message,
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
        space_id: SpaceId,
    ) -> Result<(crate::chain::Keyring, StaticSecret), String> {
        let space = self
            .documents
            .get(&space_id.document_id())
            .ok_or("the space is not open")?;
        if !space.server_created {
            return Err("the space is not on the relay yet".into());
        }
        let exchange_key = self.exchange_key.clone().ok_or("session locked")?;
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
        channel_id: ChannelId,
        space_id: SpaceId,
        (meta, segment): (MetaId, SegmentId),
        public: bool,
    ) {
        let prepared = self.event_tx.clone().ok_or_else(no_relay).and_then(|tx| {
            // A private channel is listed nowhere, so it needs no space key.
            let space_key = if public {
                Some(self.space_keys(space_id)?.0.current)
            } else {
                None
            };
            let exchange_key = self.exchange_key.clone().ok_or("session locked")?;
            let author = self.author().ok_or("session locked")?;
            // Both documents open a log under the one channel key, and the
            // channel's members live in meta's. Each creator copy seals its
            // own document's anchor, and the catalogue seals meta's, so a
            // self-joiner replays from it (`SYNC-104`).
            let channel_key = DocumentKey::generate();
            let documents = [meta.document_id(), segment.document_id()]
                .into_iter()
                .map(|document_id| {
                    let (doc_key_enc, entry) = crate::sharing::creation(
                        document_id,
                        &channel_key,
                        &exchange_key,
                        &author,
                    )?;
                    Ok((document_id, doc_key_enc, entry))
                })
                .collect::<Result<Vec<_>, String>>()?;
            let sealed = match space_key {
                Some(space_key) => {
                    let meta_anchor = documents
                        .first()
                        .and_then(|(_, _, entry)| digest_of(entry))
                        .ok_or("meta's first entry does not decode")?;
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
            Err(message) => return self.channel_answered(channel_id, Err(message)),
        };
        let api_base = self.api_base.clone();
        let url_scheme = self.url_scheme;
        let auth = self.auth_context();
        wasm_bindgen_futures::spawn_local(async move {
            let base = format!("{url_scheme}://{api_base}/api");
            let result = async {
                for (document_id, doc_key_enc, entry) in documents {
                    let body = http::CreateDocumentRequest {
                        doc_type: DocType::Murmure,
                        name_enc: None,
                        doc_key_enc,
                        document_id: Some(document_id),
                        entry: Some(entry),
                    };
                    http::post_json_auth::<_, http::CreateDocumentResponse>(
                        &format!("{base}/documents"),
                        &body,
                        auth.clone(),
                    )
                    .await?;
                }
                let body = http::CreateChannelRequest {
                    public,
                    channel_id,
                    space_id,
                    meta_id: meta,
                    segment_id: segment,
                };
                http::post_json_auth_ok(&format!("{base}/channels"), &body, auth).await
            }
            .await
            .map(|()| sealed);
            let _ = tx.send(SyncEvent::ChannelCreated { channel_id, result });
        });
    }

    /// Join a public channel alone. Unseal the catalogue's key with the space
    /// key, wrap it for this user alone, and ask the relay for the member row.
    /// The key never leaves this worker. The request carries a `SelfJoin`
    /// entry signed at the end of meta's log, which the relay's join head
    /// names with the role the join grants.
    pub(crate) fn handle_join_channel(
        &self,
        channel_id: ChannelId,
        space_id: SpaceId,
        sealed: &SealedChannelKey,
    ) {
        let prepared = self.event_tx.clone().ok_or_else(no_relay).and_then(|tx| {
            let (space_keys, exchange_key) = self.space_keys(space_id)?;
            let author = self.author().ok_or("session locked")?;
            let (doc_key_enc, key_id) = join_key(channel_id, sealed, &space_keys, &exchange_key)?;
            Ok((tx, doc_key_enc, key_id, author))
        });
        let (tx, doc_key_enc, key_id, author) = match prepared {
            Ok(prepared) => prepared,
            Err(message) => return self.channel_answered(channel_id, Err(message)),
        };
        let api_base = self.api_base.clone();
        let url_scheme = self.url_scheme;
        let auth = self.auth_context();
        wasm_bindgen_futures::spawn_local(async move {
            let url = format!("{url_scheme}://{api_base}/api/channels/{channel_id}/join");
            let result = async {
                let head =
                    http::get_json_auth::<http::JoinHeadResponse>(&url, auth.clone()).await?;
                let entry = self_join_entry(&author, &head, key_id)?;
                let body = http::JoinChannelRequest { doc_key_enc, entry };
                http::post_json_auth_ok(&url, &body, auth).await
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
        channel_id: ChannelId,
        meta: MetaId,
        closing: SegmentId,
        successor: SegmentId,
    ) {
        let (tx, meta_key, exchange_key, author) = match self.channel_creation(meta) {
            Ok(prepared) => prepared,
            Err(message) => {
                return self.send_to_logic(LogicEvent::SegmentCloseFailed {
                    channel_id,
                    closing,
                    message,
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
        meta: MetaId,
    ) -> Result<
        (
            ambre_bus::conduit::UnboundedSender<SyncEvent>,
            DocumentKey,
            StaticSecret,
            crate::sharing::Author,
        ),
        String,
    > {
        let tx = self.event_tx.clone().ok_or_else(no_relay)?;
        let meta_key = self
            .documents
            .get(&meta.document_id())
            .map(|doc| doc.doc_key.clone())
            .ok_or("the channel's meta has no open session")?;
        let exchange_key = self.exchange_key.clone().ok_or("session locked")?;
        let author = self.author().ok_or("session locked")?;
        Ok((tx, meta_key, exchange_key, author))
    }

    /// Create the amendments document of the locked `segment`, then link it
    /// (`SRV-201`). It opens under the current key of `meta`, as the
    /// segment's successor did.
    pub(crate) fn handle_create_amendments(
        &self,
        channel_id: ChannelId,
        meta: MetaId,
        segment: SegmentId,
        amendments: DocumentId,
    ) {
        let (tx, meta_key, exchange_key, author) = match self.channel_creation(meta) {
            Ok(prepared) => prepared,
            Err(message) => {
                return self.send_to_logic(LogicEvent::AmendmentsLinkFailed {
                    channel_id,
                    segment,
                    amendments,
                    message,
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
        channel_id: ChannelId,
        meta: MetaId,
        segment: SegmentId,
        root: ambre_bus::MessageId,
        thread: DocumentId,
    ) {
        let (tx, meta_key, exchange_key, author) = match self.channel_creation(meta) {
            Ok(prepared) => prepared,
            Err(message) => {
                return self.send_to_logic(LogicEvent::ThreadLinkFailed {
                    channel_id,
                    segment,
                    root,
                    thread,
                    message,
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
    pub(crate) fn handle_delete_channel(&self, channel_id: ChannelId) {
        let Some(tx) = self.event_tx.clone() else {
            return self.channel_answered(channel_id, Err(no_relay()));
        };
        let api_base = self.api_base.clone();
        let url_scheme = self.url_scheme;
        let auth = self.auth_context();
        wasm_bindgen_futures::spawn_local(async move {
            let url = format!("{url_scheme}://{api_base}/api/channels/{channel_id}");
            let result = http::delete_auth(&url, auth).await;
            let _ = tx.send(SyncEvent::ChannelDeleted { channel_id, result });
        });
    }

    /// Ask the relay to remove this user from a channel. The documents stay
    /// open here until the Logic thread hears the answer and closes them. A
    /// refusal leaves the channel whole, as with a delete.
    pub(crate) fn handle_leave_channel(&self, channel_id: ChannelId) {
        let Some(tx) = self.event_tx.clone() else {
            return self.channel_answered(channel_id, Err(no_relay()));
        };
        let (Some(author), Some(exchange_key)) = (self.author(), self.exchange_key.clone()) else {
            return self.channel_answered(channel_id, Err("not signed in".into()));
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
        channel_id: ChannelId,
        ids: (MetaId, SegmentId),
        members: Vec<DirectMember>,
    ) {
        let prepared = self.event_tx.clone().ok_or_else(no_relay).and_then(|tx| {
            let exchange_key = self.exchange_key.clone().ok_or("session locked")?;
            let author = self.author().ok_or("session locked")?;
            Ok((tx, exchange_key, author))
        });
        let (tx, exchange_key, author) = match prepared {
            Ok(prepared) => prepared,
            Err(message) => return self.channel_answered(channel_id, Err(message)),
        };
        let api_base = self.api_base.clone();
        let url_scheme = self.url_scheme;
        let auth = self.auth_context();
        wasm_bindgen_futures::spawn_local(async move {
            let base = format!("{url_scheme}://{api_base}/api");
            let result = async {
                let mut signing_keys = Vec::with_capacity(members.len());
                for member in &members {
                    let url = format!("{base}/users/{}/pubkeys", member.user_id);
                    let served =
                        http::get_json_auth::<http::PubkeysResponse>(&url, auth.clone()).await?;
                    check_pinned(member, &served.exchange_pubkey)?;
                    signing_keys.push(crate::sharing::decode_signing_pubkey(
                        &served.signing_pubkey,
                    )?);
                }
                let invited: Vec<_> = members.iter().zip(signing_keys).collect();
                let body =
                    direct_message_request(&author, &exchange_key, channel_id, ids, &invited)?;
                let answer = http::post_json_auth::<_, http::DirectMessageResponse>(
                    &format!("{base}/channels/direct"),
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
    pub(crate) fn read_up_to(&self, document_id: DocumentId) -> Option<Seq> {
        let doc = self.documents.get(&document_id)?;
        (doc.last_seq.0 > 0 && doc.quarantine_len() == 0).then_some(doc.last_seq)
    }

    /// Move the cursors of one channel to its latest message (the Murmure
    /// unread spec). Each live segment moves to what this worker delivered.
    /// Each final segment no longer changes, so it moves to the relay's last
    /// number.
    pub(crate) fn handle_mark_read(&self, live: &[SegmentId], finals: Vec<SegmentId>) {
        let cursors: Vec<(SegmentId, Seq)> = live
            .iter()
            .filter_map(|&segment| {
                self.read_up_to(segment.document_id())
                    .map(|seq| (segment, seq))
            })
            .collect();
        if self.event_tx.is_none() {
            tracing::warn!("Sync: no relay on this transport, the read cursors stay");
            return;
        }
        let api_base = self.api_base.clone();
        let url_scheme = self.url_scheme;
        let auth = self.auth_context();
        wasm_bindgen_futures::spawn_local(async move {
            let base = format!("{url_scheme}://{api_base}/api");
            put_cursors(&base, cursors, auth.clone()).await;
            if !finals.is_empty() {
                let url = format!("{url_scheme}://{api_base}/api/read-cursors/mark-read");
                let body = http::MarkReadRequest {
                    documents: Some(finals.iter().map(SegmentId::document_id).collect()),
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
    pub(crate) fn handle_move_read_cursors(&self, cursors: Vec<(SegmentId, Seq)>) {
        if self.event_tx.is_none() {
            tracing::warn!("Sync: no relay on this transport, the read cursors stay");
            return;
        }
        let base = format!("{}://{}/api", self.url_scheme, self.api_base);
        let auth = self.auth_context();
        wasm_bindgen_futures::spawn_local(async move {
            put_cursors(&base, cursors, auth).await;
        });
    }

    /// Move every cursor of the user to its last number (the Murmure unread
    /// spec). Then fetch the activity answer, so the sidebar clears from the
    /// relay's word.
    pub(crate) fn handle_mark_all_read(&self) {
        self.fetch_activity(true);
    }

    /// Ask the relay which channel documents hold something new (`SRV-109`).
    pub(crate) fn handle_fetch_activity(&self) {
        self.fetch_activity(false);
    }

    fn fetch_activity(&self, mark_all_read_first: bool) {
        let Some(tx) = self.event_tx.clone() else {
            tracing::warn!("Sync: no relay on this transport, no activity answer");
            return;
        };
        let api_base = self.api_base.clone();
        let url_scheme = self.url_scheme;
        let auth = self.auth_context();
        wasm_bindgen_futures::spawn_local(async move {
            if mark_all_read_first {
                let url = format!("{url_scheme}://{api_base}/api/read-cursors/mark-read");
                let body = http::MarkReadRequest { documents: None };
                if let Err(e) = http::post_json_auth_ok(&url, &body, auth.clone()).await {
                    tracing::warn!("Sync: mark all read failed: {e}");
                }
            }
            let url = format!("{url_scheme}://{api_base}/api/activity");
            let result = http::get_json_auth::<http::ActivityResponse>(&url, auth)
                .await
                .map(|resp| activity_of(&resp.documents));
            let _ = tx.send(SyncEvent::ActivityFetched { result });
        });
    }

    /// The answer of [`fetch_activity`](Self::fetch_activity), on the Sync
    /// loop. A failure leaves the marks the view holds, and says so in the log.
    pub(crate) fn activity_fetched(&self, result: Result<Vec<DocumentActivity>, String>) {
        match result {
            Ok(documents) => self.send_to_logic(LogicEvent::ChannelActivity { documents }),
            Err(message) => tracing::warn!("Sync: activity answer failed: {message}"),
        }
    }

    /// List the channels the relay holds as public in a space (`SRV-189`),
    /// so a space admin can drop an entry whose channel is gone.
    pub(crate) fn handle_list_space_channels(&self, space_id: SpaceId) {
        let Some(tx) = self.event_tx.clone() else {
            tracing::warn!("Sync: no relay on this transport, no channels of a space");
            return;
        };
        let api_base = self.api_base.clone();
        let url_scheme = self.url_scheme;
        let auth = self.auth_context();
        wasm_bindgen_futures::spawn_local(async move {
            let url = format!("{url_scheme}://{api_base}/api/spaces/{space_id}/channels");
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
        space_id: SpaceId,
        result: Result<Vec<ChannelId>, String>,
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
        self.member_channels_asked = self.member_channels_asked.wrapping_add(1);
        let asked = self.member_channels_asked;
        let Some(tx) = self.event_tx.clone() else {
            tracing::warn!("Sync: no relay on this transport, no channels of the user");
            return;
        };
        let api_base = self.api_base.clone();
        let url_scheme = self.url_scheme;
        let auth = self.auth_context();
        wasm_bindgen_futures::spawn_local(async move {
            let url = format!("{url_scheme}://{api_base}/api/channels");
            let result = http::get_json_auth::<http::MemberChannelsResponse>(&url, auth)
                .await
                .map(|resp| {
                    resp.channels
                        .into_iter()
                        .map(http::MemberChannelEntry::into_member_channel)
                        .collect()
                });
            let _ = tx.send(SyncEvent::MemberChannelsListed { asked, result });
        });
    }

    /// The answer of
    /// [`handle_list_member_channels`](Self::handle_list_member_channels), on
    /// the Sync loop. A failure answers nothing: the next share asks again.
    /// An answer a later listing overtook answers nothing either.
    pub(crate) fn member_channels_listed(
        &self,
        asked: u32,
        result: Result<Vec<MemberChannel>, String>,
    ) {
        if asked != self.member_channels_asked {
            return;
        }
        match result {
            Ok(channels) => self.send_to_logic(LogicEvent::MemberChannels { channels }),
            Err(message) => {
                tracing::warn!("Sync: listing the channels of the user failed: {message}");
            }
        }
    }
}

fn no_relay() -> String {
    "no relay on this transport".into()
}

/// Move each cursor to its number, one request per document. A refused move
/// leaves that cursor where it was and says so in the log.
async fn put_cursors(base: &str, cursors: Vec<(SegmentId, Seq)>, auth: http::AuthContext) {
    for (segment, seq) in cursors {
        let url = format!("{base}/documents/{segment}/read-cursor");
        let body = http::ReadCursorRequest { seq };
        if let Err(e) = http::put_json_auth(&url, &body, auth.clone()).await {
            tracing::warn!("Sync: read cursor of {segment} not moved: {e}");
        }
    }
}

/// The aad of a sealed channel key: it binds the key to its channel, so a
/// catalogue entry moved under another channel fails to open.
fn channel_key_aad(channel_id: ChannelId) -> Vec<u8> {
    let mut aad = b"ambre-channel-key:".to_vec();
    aad.extend_from_slice(ambre_core::Uuid::from(channel_id).as_bytes());
    aad
}

/// The digest of an entry in request form: the anchor of the log a first
/// entry opens, and the link the entry after it carries.
fn digest_of(entry: &str) -> Option<ambre_protocol::EntryDigest> {
    membership_log::decode(&[entry.to_owned()])
        .first()
        .and_then(membership_log::digest)
}

/// The channel key sealed under the space key for the catalogue, with the
/// anchor of meta's log beside it (`SYNC-104`). The id of the space key leads
/// the sealed bytes. A reader then opens it with the one key it names
/// (`SYNC-045`), and a rotation of the space shows which entries it left
/// behind.
pub(crate) fn seal_channel_key(
    channel_id: ChannelId,
    channel_key: &DocumentKey,
    space_key: &DocumentKey,
    meta_anchor: ambre_protocol::EntryDigest,
) -> Result<SealedChannelKey, String> {
    let (nonce, ciphertext) = ambre_crypto::aead::seal_anchored_key(
        channel_key,
        &channel_key_aad(channel_id),
        space_key,
        Some(&meta_anchor.0),
    )
    .map_err(|e| format!("seal the channel key: {e}"))?;
    let mut sealed = space_key.id().as_array().to_vec();
    sealed.extend_from_slice(&nonce);
    sealed.extend_from_slice(&ciphertext);
    Ok(SealedChannelKey::from_bytes(sealed))
}

/// The id of the space key a catalogue entry was sealed under. `None` for
/// bytes too short to name one.
pub(crate) fn sealed_under(sealed: &SealedChannelKey) -> Option<KeyId> {
    sealed
        .as_bytes()
        .first_chunk::<{ ambre_core::KEY_ID_LEN }>()
        .map(|id| KeyId::from_array(*id))
}

/// Unseal a channel key the catalogue holds, with the anchor sealed beside it.
/// The space key that opens it is the one its first bytes name, as `keys`
/// reach it.
pub(crate) fn unseal_channel_key(
    channel_id: ChannelId,
    sealed: &SealedChannelKey,
    keys: &crate::chain::Keyring,
) -> Result<(DocumentKey, Option<ambre_protocol::EntryDigest>), String> {
    let short = "the sealed channel key is too short";
    let (key_id, rest) = sealed
        .as_bytes()
        .split_first_chunk::<{ ambre_core::KEY_ID_LEN }>()
        .ok_or(short)?;
    let space_key = keys
        .find(KeyId::from_array(*key_id))
        .ok_or("no space key this client holds sealed the channel key")?;
    let space_key = &space_key;
    let (nonce, ciphertext) = rest.split_first_chunk::<NONCE_LEN>().ok_or(short)?;
    let (key, anchor) = ambre_crypto::aead::open_anchored_key(
        nonce,
        ciphertext,
        &channel_key_aad(channel_id),
        space_key,
    )
    .map_err(|e| format!("unseal the channel key: {e}"))?;
    Ok((key, anchor.map(ambre_protocol::EntryDigest)))
}

/// The self-join key: the catalogue's key, unsealed and wrapped for this
/// user alone, as the Murmure spaces spec asks of a self-join, and its id.
/// The wrap carries the anchor the catalogue sealed, so the joiner replays
/// meta's log from the word of the member who created the channel.
pub(crate) fn join_key(
    channel_id: ChannelId,
    sealed: &SealedChannelKey,
    space_keys: &crate::chain::Keyring,
    exchange_key: &StaticSecret,
) -> Result<(String, KeyId), String> {
    let (key, anchor) = unseal_channel_key(channel_id, sealed, space_keys)?;
    let own_pub = x25519_dalek::PublicKey::from(exchange_key);
    let wrapped = crate::sharing::wrap_anchored_b64(exchange_key, own_pub.as_bytes(), &key, anchor)
        .map_err(|e| format!("wrap the channel key: {e}"))?;
    Ok((wrapped, key.id()))
}

/// The `SelfJoin` entry that extends the log the join head names, at the
/// role it grants (`SYNC-106`). `None` for an empty log: it predates the
/// log, and no entry of it could link to a first one.
pub(crate) fn self_join_entry(
    author: &crate::sharing::Author,
    head: &http::JoinHeadResponse,
    key: KeyId,
) -> Result<Option<String>, String> {
    if head.length == 0 {
        return Ok(None);
    }
    let meta = head.meta_id.document_id();
    let last = head
        .last
        .as_ref()
        .and_then(|last| digest_of(last))
        .ok_or("the join head names no last entry")?;
    let draft = membership_log::Draft {
        document: meta,
        member: author.user_id,
        member_key: author.signing_key.verifying_key().to_bytes(),
        act: ambre_protocol::MembershipAct::SelfJoin,
        role: head.role,
        key: Some(key),
    };
    let entry = membership_log::sign(
        draft,
        (head.length, Some(last)),
        author.user_id,
        &author.signing_key,
    )?;
    membership_log::encode(&entry).map(Some)
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
    channel_id: ChannelId,
    (meta, segment): (MetaId, SegmentId),
    members: &[(&DirectMember, [u8; 32])],
) -> Result<http::CreateDirectMessageRequest, String> {
    let key = DocumentKey::generate();
    let document = meta.document_id();
    let own_key = author.signing_key.verifying_key().to_bytes();
    let others: Vec<UserId> = members.iter().map(|(m, _)| m.user_id).collect();
    let signing_keys: HashMap<UserId, [u8; 32]> = members
        .iter()
        .map(|(m, signing)| (m.user_id, *signing))
        .chain([(author.user_id, own_key)])
        .collect();
    let mut next = (0, None);
    let mut root = None;
    let mut entries = Vec::with_capacity(others.len() + 2);
    for (member, act, role) in ambre_protocol::ownerless_log(author.user_id, &others) {
        let draft = membership_log::Draft {
            document,
            member,
            member_key: *signing_keys
                .get(&member)
                .ok_or("a member has no signing key")?,
            act,
            role,
            key: (act != ambre_protocol::MembershipAct::ChangeRole).then(|| key.id()),
        };
        let entry = membership_log::sign(draft, next, author.user_id, &author.signing_key)?;
        let digest = membership_log::digest(&entry).ok_or("an entry does not encode")?;
        root.get_or_insert(digest);
        next = (next.0 + 1, Some(digest));
        entries.push(membership_log::encode(&entry)?);
    }
    // The creation goes first, so the first digest is the log's root.
    let root = root.ok_or("the log holds no creation")?;
    let wrap = |peer: &[u8; 32]| {
        crate::sharing::wrap_anchored_b64(exchange_key, peer, &key, Some(root))
            .map_err(|e| format!("wrap the channel key: {e}"))
    };
    let own_pub = x25519_dalek::PublicKey::from(exchange_key);
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
        doc_key_enc: wrap(own_pub.as_bytes())?,
        key_id: key.id(),
        entries,
    })
}

/// Refuse a member whose exchange key, as the relay serves it now, is not
/// the one the view pinned for them.
pub(crate) fn check_pinned(member: &DirectMember, served_hex: &str) -> Result<(), String> {
    let served = crate::sharing::decode_exchange_pubkey(served_hex)?;
    if served == member.exchange_pub_key {
        Ok(())
    } else {
        Err(format!(
            "the exchange key of {} is not the one pinned",
            member.user_id
        ))
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
            active: e.active,
            read: e.read,
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
        let meta = DocumentId::new();
        let segment = DocumentId::new();
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
        let meta = DocumentId::new().to_string();
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
            "active": active,
            "read": 7,
        });
        let body = serde_json::json!({ "documents": [row] });
        serde_json::from_value::<http::ActivityResponse>(body)
            .map(|resp| activity_of(&resp.documents))
    }

    #[test]
    fn an_activity_answer_parses_into_its_documents() {
        let (channel, segment) = (ChannelId::new(), DocumentId::new());
        let parsed = activity(&channel.to_string(), &segment.to_string(), true).expect("ok");
        assert_eq!(
            parsed,
            [DocumentActivity {
                channel_id: channel,
                document_id: segment,
                active: true,
                read: Seq(7),
            }]
        );
        let bad = activity(&channel.to_string(), "nope", false);
        assert!(bad.is_err(), "one bad row fails the answer");
    }

    /// An anchor of meta's log, as the creator's `Create` entry would give it.
    const ANCHOR: ambre_protocol::EntryDigest = ambre_protocol::EntryDigest([5; 32]);

    fn exchange_key(byte: u8) -> StaticSecret {
        StaticSecret::from([byte; 32])
    }

    /// The key a wrapped value opens to, for its holder.
    fn unwrap_own(exchange_key: &StaticSecret, doc_key_enc: &str) -> Option<DocumentKey> {
        let bytes = base64::engine::general_purpose::URL_SAFE
            .decode(doc_key_enc)
            .ok()?;
        let (nonce, ciphertext) = bytes.split_first_chunk::<NONCE_LEN>()?;
        let wrapped = ambre_crypto::ecdh::WrappedKey {
            nonce: *nonce,
            ciphertext: ciphertext.to_vec(),
        };
        let own_pub = x25519_dalek::PublicKey::from(exchange_key);
        let (key, anchor) =
            ambre_crypto::ecdh::unwrap_anchored(exchange_key, own_pub.as_bytes(), &wrapped).ok()?;
        assert_eq!(anchor, Some(ANCHOR.0), "every wrap carries meta's anchor");
        Some(key)
    }

    /// A channel key sealed for the catalogue, and its creator's own copy.
    fn new_channel_key(
        channel_id: ChannelId,
        space_key: &DocumentKey,
        creator: &StaticSecret,
    ) -> (SealedChannelKey, String) {
        let channel_key = DocumentKey::generate();
        let sealed = seal_channel_key(channel_id, &channel_key, space_key, ANCHOR).expect("seal");
        let own_pub = x25519_dalek::PublicKey::from(creator);
        let own = crate::sharing::wrap_anchored_b64(
            creator,
            own_pub.as_bytes(),
            &channel_key,
            Some(ANCHOR),
        )
        .expect("wrap");
        (sealed, own)
    }

    /// The keys of a space whose current key is `current`, after rotations
    /// that replaced each of `previous`.
    pub(crate) fn ring(current: &DocumentKey, previous: &[&DocumentKey]) -> crate::chain::Keyring {
        crate::chain::Keyring {
            document_id: DocumentId::new(),
            current: current.clone(),
            previous: previous.iter().map(|&key| key.clone()).collect(),
            chain: crate::chain::Chain::default(),
        }
    }

    /// Whether two keys are one: what one seals, the other opens.
    fn same_key(a: &DocumentKey, b: &DocumentKey) -> bool {
        let (nonce, ct) = ambre_crypto::aead::seal(b"probe", b"", a).expect("seal");
        ambre_crypto::aead::open(&nonce, &ct, b"", b).is_ok()
    }

    // --- The Murmure spaces spec: the sealed catalogue key and a self-join ---

    /// Alice creates a channel: the catalogue gets its key sealed under the
    /// space key. Bob joins alone: his worker unseals it and wraps it for
    /// himself, and the key he opens is the key Alice's documents are under.
    #[test]
    fn a_self_join_unseals_the_catalogue_key_and_wraps_it_for_the_joiner_alone() {
        let space_key = DocumentKey::generate();
        let (alice, bob, eve) = (exchange_key(1), exchange_key(2), exchange_key(3));
        let channel_id = ChannelId::new();
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
        let channel_id = ChannelId::new();
        let (sealed, _) = new_channel_key(channel_id, &space_key, &exchange_key(1));
        let keys = ring(&space_key, &[]);
        assert_eq!(
            sealed.as_bytes().len(),
            ambre_core::KEY_ID_LEN + NONCE_LEN + 32 + 32 + 16
        );
        let (_, anchor) = unseal_channel_key(channel_id, &sealed, &keys).expect("open");
        assert_eq!(anchor, Some(ANCHOR));
        let stranger = ring(&DocumentKey::generate(), &[]);
        assert!(unseal_channel_key(channel_id, &sealed, &stranger).is_err());
        assert!(unseal_channel_key(ChannelId::new(), &sealed, &keys).is_err());
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
        let channel_id = ChannelId::new();
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
        let state = SyncState::new(sink, String::new(), None);
        let (channel_id, space_id) = (ChannelId::new(), SpaceId::new());
        for public in [true, false] {
            let documents = (MetaId::new(), SegmentId::new());
            state.handle_create_channel(channel_id, space_id, documents, public);
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
        let mut state = SyncState::new(sink, String::new(), None);
        let channel_id = ChannelId::new();
        let meta = DocumentId::new();
        state.documents.insert(
            meta,
            SyncDocumentState::new(meta, DocumentKey::generate(), DocType::Murmure, true),
        );

        state.handle_delete_channel(channel_id);
        state.handle_sync_event(SyncEvent::ChannelDeleted {
            channel_id,
            result: Err("403".into()),
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
        let mut state = SyncState::new(sink, String::new(), None);
        let channel_id = ChannelId::new();
        let meta = DocumentId::new();
        state.documents.insert(
            meta,
            SyncDocumentState::new(meta, DocumentKey::generate(), DocType::Murmure, true),
        );

        state.handle_leave_channel(channel_id);
        state.handle_sync_event(SyncEvent::ChannelLeft {
            channel_id,
            result: Err("409".into()),
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
        let mut state = SyncState::new(sink, String::new(), None);
        let channel_id = ChannelId::new();
        let url = "GET https://relay.example/api/channels/c/documents";
        for message in [
            format!("{url}: {}", ambre_core::RefusalCode::Forbidden),
            format!("{url}: HTTP 403"),
            format!("{url}: HTTP 500"),
        ] {
            state.channel_listed(channel_id, None, Err(message));
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
        let state = SyncState::new(sink, String::new(), None);
        let space_id = SpaceId::new();
        let channel_id = ChannelId::new();
        let body = format!(r#"{{"channels":["{channel_id}"]}}"#);
        let answer: http::SpaceChannelsResponse = serde_json::from_str(&body).expect("decodes");
        state.space_channels_listed(space_id, Ok(answer.channels));
        state.space_channels_listed(space_id, Err("HTTP 500".into()));
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
        let mut state = SyncState::new(sink, String::new(), None);
        for doc_type in [DocType::Prose, DocType::Murmure] {
            state.handle_ws_message(ambre_protocol::ws::WsServerMsg::DocumentShared {
                document_id: DocumentId::new(),
                doc_type,
            });
        }
        assert_eq!(state.member_channels_asked, 1, "one listing");
        let shared = rx
            .try_iter()
            .filter(|e| matches!(e, LogicEvent::DocumentShared { .. }))
            .count();
        assert_eq!(shared, 2, "each share still reaches Logic");
    }

    /// The relay's listing of the user's channels decodes to their places
    /// and reaches the Logic thread. A failed listing answers nothing, nor
    /// does one a later listing overtook.
    #[test]
    fn a_listing_of_the_users_channels_forwards_their_places() {
        let (sink, rx) = LogicSink::channel();
        let mut state = SyncState::new(sink, String::new(), None);
        state.handle_list_member_channels();
        state.handle_list_member_channels();
        let space = SpaceId::new();
        let (public, private, direct) = (ChannelId::new(), ChannelId::new(), ChannelId::new());
        let body = format!(
            r#"{{"channels":[
                {{"channel_id":"{public}","space_id":"{space}","public":true}},
                {{"channel_id":"{private}","space_id":"{space}","public":false}},
                {{"channel_id":"{direct}","space_id":null,"public":false}}]}}"#
        );
        let answer: http::MemberChannelsResponse = serde_json::from_str(&body).expect("decodes");
        let channels = answer
            .channels
            .into_iter()
            .map(http::MemberChannelEntry::into_member_channel)
            .collect();
        state.member_channels_listed(1, Ok(Vec::new()));
        state.member_channels_listed(2, Ok(channels));
        state.member_channels_listed(2, Err("HTTP 500".into()));
        let events: Vec<_> = rx
            .try_iter()
            .filter_map(|e| match e {
                LogicEvent::MemberChannels { channels } => Some(channels),
                _ => None,
            })
            .collect();
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
        assert_eq!(events, vec![expected], "the latest listing alone");
    }

    /// A meta a catalogue lists walks under the word of its open space. Any
    /// other log walks alone (the Murmure spaces spec).
    #[test]
    fn a_listed_meta_walks_under_the_word_of_its_open_space() {
        let (sink, _rx) = LogicSink::channel();
        let mut state = SyncState::new(sink, String::new(), None);
        let (space_id, meta) = (SpaceId::new(), DocumentId::new());
        let document_id = space_id.document_id();
        let mut space =
            SyncDocumentState::new(document_id, DocumentKey::generate(), DocType::Murmure, true);
        space.word.removed.insert(ambre_bus::UserId::new());
        let word = space.word.clone();
        state.documents.insert(document_id, space);
        assert_eq!(state.log_rule(meta).space, None);
        state.self_join_logs.insert(meta, space_id);
        assert_eq!(state.log_rule(meta).space, Some(word));
    }

    #[test]
    fn with_no_relay_the_open_fails_at_once() {
        let (sink, rx) = LogicSink::channel();
        let state = SyncState::new(sink, String::new(), None);
        let channel_id = ChannelId::new();
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
        doc_key_enc: &str,
    ) -> Option<(DocumentKey, Option<ambre_protocol::EntryDigest>)> {
        let bytes = base64::engine::general_purpose::URL_SAFE
            .decode(doc_key_enc)
            .ok()?;
        let (nonce, ciphertext) = bytes.split_first_chunk::<NONCE_LEN>()?;
        let wrapped = ambre_crypto::ecdh::WrappedKey {
            nonce: *nonce,
            ciphertext: ciphertext.to_vec(),
        };
        let sender_pub = x25519_dalek::PublicKey::from(sender);
        let (key, root) =
            ambre_crypto::ecdh::unwrap_anchored(holder, sender_pub.as_bytes(), &wrapped).ok()?;
        Some((key, root.map(ambre_protocol::EntryDigest)))
    }

    fn member_of(exchange_key: &StaticSecret) -> DirectMember {
        DirectMember {
            user_id: UserId::new(),
            exchange_pub_key: x25519_dalek::PublicKey::from(exchange_key).to_bytes(),
        }
    }

    /// A direct message's log replays to every member as an editor, the
    /// creator included, and leaves no owner. Each member opens one key
    /// from their own wrap, with the log's root beside it.
    #[test]
    fn a_direct_message_makes_every_member_an_editor_under_one_key() {
        let author = crate::sharing::Author {
            user_id: UserId::new(),
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
                    SigningKey::from_bytes(&[seed; 32])
                        .verifying_key()
                        .to_bytes(),
                )
            })
            .collect();
        let meta = MetaId::new();
        let body = direct_message_request(
            &author,
            &alice,
            ChannelId::new(),
            (meta, SegmentId::new()),
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
        assert!(check_pinned(&bob, &hex::encode(bob.exchange_pub_key)).is_ok());
        let other = x25519_dalek::PublicKey::from(&exchange_key(3)).to_bytes();
        assert!(check_pinned(&bob, &hex::encode(other)).is_err());
        assert!(check_pinned(&bob, "not hex").is_err());
    }

    #[test]
    fn with_no_relay_a_direct_message_fails_at_once() {
        let (sink, rx) = LogicSink::channel();
        let state = SyncState::new(sink, String::new(), None);
        let channel_id = ChannelId::new();
        let members = vec![member_of(&exchange_key(2))];
        state.handle_create_direct_message(channel_id, (MetaId::new(), SegmentId::new()), members);
        let events: Vec<LogicEvent> = rx.try_iter().collect();
        assert!(matches!(
            events.as_slice(),
            [LogicEvent::ChannelRequestFailed { channel_id: id, .. }] if *id == channel_id
        ));
    }
}
