use super::*;
use ambre_bus::{
    ChannelDocument, ChannelDocumentKind, DocumentActivity, MetaId, SealedChannelKey, SegmentId,
    SpaceId,
};
use ambre_crypto::aead::NONCE_LEN;

impl SyncState {
    // -- Channels --------------------------------------------------------------

    /// List the documents of a channel, so the Logic thread can open them.
    ///
    /// A channel travels as its documents (`SRV-102`): this is its only
    /// request of its own. The relay answers the same refusal to an unknown
    /// channel and to a caller who is no member of it (`SRV-047`).
    pub(crate) fn handle_open_channel(&self, channel_id: ChannelId) {
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
                .and_then(|resp| channel_documents(&resp.documents));
            let _ = tx.send(SyncEvent::ChannelListed { channel_id, result });
        });
    }

    /// The answer of [`handle_open_channel`](Self::handle_open_channel), on
    /// the Sync loop.
    pub(crate) fn channel_listed(
        &self,
        channel_id: ChannelId,
        result: Result<Vec<ChannelDocument>, String>,
    ) {
        match result {
            Ok(documents) => {
                self.send_to_logic(LogicEvent::ChannelDocuments {
                    channel_id,
                    documents,
                });
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

    /// The key of an open space that the relay holds, and this user's
    /// exchange key: what a channel creation and a self-join both need.
    fn space_keys(&self, space_id: SpaceId) -> Result<(DocumentKey, StaticSecret), String> {
        let space = self
            .documents
            .get(&space_id.document_id())
            .ok_or("the space is not open")?;
        if !space.server_created {
            return Err("the space is not on the relay yet".into());
        }
        let exchange_key = self.exchange_key.clone().ok_or("session locked")?;
        Ok((space.doc_key.clone(), exchange_key))
    }

    /// Create a public channel of a space on the relay (the Murmure spaces
    /// spec). One fresh key opens both documents, wrapped for this user alone,
    /// and goes back to the Logic thread sealed under the space key. No step
    /// leaves a channel the relay holds half made: each document is created
    /// by its own id, so a retry finds it.
    pub(crate) fn handle_create_public_channel(
        &self,
        channel_id: ChannelId,
        space_id: SpaceId,
        meta: MetaId,
        segment: SegmentId,
    ) {
        let prepared = self.event_tx.clone().ok_or_else(no_relay).and_then(|tx| {
            let (space_key, exchange_key) = self.space_keys(space_id)?;
            let (sealed, doc_key_enc) = new_channel_key(channel_id, &space_key, &exchange_key)?;
            Ok((tx, sealed, doc_key_enc))
        });
        let (tx, sealed, doc_key_enc) = match prepared {
            Ok(prepared) => prepared,
            Err(message) => return self.channel_answered(channel_id, Err(message)),
        };
        let api_base = self.api_base.clone();
        let url_scheme = self.url_scheme;
        let auth = self.auth_context();
        wasm_bindgen_futures::spawn_local(async move {
            let base = format!("{url_scheme}://{api_base}/api");
            let result = async {
                for document_id in [meta.document_id(), segment.document_id()] {
                    let body = http::CreateDocumentRequest {
                        doc_type: "murmure".into(),
                        name_enc: None,
                        doc_key_enc: doc_key_enc.clone(),
                        document_id: Some(document_id.to_string()),
                    };
                    http::post_json_auth::<_, http::CreateDocumentResponse>(
                        &format!("{base}/documents"),
                        &body,
                        auth.clone(),
                    )
                    .await?;
                }
                let body = http::CreateChannelRequest {
                    channel_id: channel_id.to_string(),
                    space_id: space_id.to_string(),
                    meta_id: meta.to_string(),
                    segment_id: segment.to_string(),
                };
                http::post_json_auth_ok(&format!("{base}/channels"), &body, auth).await
            }
            .await
            .map(|()| sealed);
            let _ = tx.send(SyncEvent::ChannelCreated { channel_id, result });
        });
    }

    /// Join a public channel alone. Unseal the catalogue's key with the space
    /// key, wrap it for this user alone, and ask the relay for the member row. The key never leaves this worker.
    pub(crate) fn handle_join_channel(
        &self,
        channel_id: ChannelId,
        space_id: SpaceId,
        sealed: &SealedChannelKey,
    ) {
        let prepared = self.event_tx.clone().ok_or_else(no_relay).and_then(|tx| {
            let (space_key, exchange_key) = self.space_keys(space_id)?;
            let doc_key_enc = join_key(channel_id, sealed, &space_key, &exchange_key)?;
            Ok((tx, doc_key_enc))
        });
        let (tx, doc_key_enc) = match prepared {
            Ok(prepared) => prepared,
            Err(message) => return self.channel_answered(channel_id, Err(message)),
        };
        let api_base = self.api_base.clone();
        let url_scheme = self.url_scheme;
        let auth = self.auth_context();
        wasm_bindgen_futures::spawn_local(async move {
            let url = format!("{url_scheme}://{api_base}/api/channels/{channel_id}/join");
            let body = http::JoinChannelRequest { doc_key_enc };
            let result = http::post_json_auth_ok(&url, &body, auth).await;
            let _ = tx.send(SyncEvent::ChannelJoined { channel_id, result });
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
        let cursors: Vec<(DocumentId, Seq)> = live
            .iter()
            .map(SegmentId::document_id)
            .filter_map(|id| self.read_up_to(id).map(|seq| (id, seq)))
            .collect();
        if self.event_tx.is_none() {
            tracing::warn!("Sync: no relay on this transport, the read cursors stay");
            return;
        }
        let api_base = self.api_base.clone();
        let url_scheme = self.url_scheme;
        let auth = self.auth_context();
        wasm_bindgen_futures::spawn_local(async move {
            for (document_id, seq) in cursors {
                let url =
                    format!("{url_scheme}://{api_base}/api/documents/{document_id}/read-cursor");
                let body = http::ReadCursorRequest { seq: seq.0 };
                if let Err(e) = http::put_json_auth(&url, &body, auth.clone()).await {
                    tracing::warn!("Sync: read cursor of {document_id} not moved: {e}");
                }
            }
            if !finals.is_empty() {
                let url = format!("{url_scheme}://{api_base}/api/read-cursors/mark-read");
                let body = http::MarkReadRequest {
                    documents: Some(finals.iter().map(ToString::to_string).collect()),
                };
                if let Err(e) = http::post_json_auth_ok(&url, &body, auth).await {
                    tracing::warn!("Sync: read cursors of the final segments not moved: {e}");
                }
            }
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
                .and_then(|resp| activity_of(&resp.documents));
            let _ = tx.send(SyncEvent::ActivityFetched { result });
        });
    }

    /// The answer of [`fetch_activity`](Self::fetch_activity), on the Sync
    /// loop. A failure leaves the marks the view holds, and says so in the log.
    pub(crate) fn activity_fetched(&self, result: Result<Vec<DocumentActivity>, String>) {
        match result {
            Ok(documents) => self.send_to_logic(LogicEvent::Activity { documents }),
            Err(message) => tracing::warn!("Sync: activity answer failed: {message}"),
        }
    }
}

fn no_relay() -> String {
    "no relay on this transport".into()
}

/// The aad of a sealed channel key: it binds the key to its channel, so a
/// catalogue entry moved under another channel fails to open.
fn channel_key_aad(channel_id: ChannelId) -> Vec<u8> {
    let mut aad = b"ambre-channel-key:".to_vec();
    aad.extend_from_slice(ambre_core::Uuid::from(channel_id).as_bytes());
    aad
}

/// A fresh channel key: sealed under the space key for the catalogue, and
/// wrapped for this user for the relay.
pub(crate) fn new_channel_key(
    channel_id: ChannelId,
    space_key: &DocumentKey,
    exchange_key: &StaticSecret,
) -> Result<(SealedChannelKey, String), String> {
    let channel_key = DocumentKey::generate();
    let (nonce, ciphertext) = ambre_crypto::aead::seal_document_key(
        &channel_key,
        &channel_key_aad(channel_id),
        space_key,
    )
    .map_err(|e| format!("seal the channel key: {e}"))?;
    let mut sealed = nonce.to_vec();
    sealed.extend_from_slice(&ciphertext);
    let own_pub = x25519_dalek::PublicKey::from(exchange_key);
    let wrapped = crate::sharing::wrap_key_b64(exchange_key, own_pub.as_bytes(), &channel_key)
        .map_err(|e| format!("wrap the channel key: {e}"))?;
    Ok((SealedChannelKey::from_bytes(sealed), wrapped))
}

/// Unseal a channel key the catalogue holds.
pub(crate) fn unseal_channel_key(
    channel_id: ChannelId,
    sealed: &SealedChannelKey,
    space_key: &DocumentKey,
) -> Result<DocumentKey, String> {
    let bytes = sealed.as_bytes();
    let (nonce, ciphertext) = bytes
        .split_first_chunk::<NONCE_LEN>()
        .ok_or("the sealed channel key is too short")?;
    ambre_crypto::aead::open_document_key(
        nonce,
        ciphertext,
        &channel_key_aad(channel_id),
        space_key,
    )
    .map_err(|e| format!("unseal the channel key: {e}"))
}

/// The self-join key: the catalogue's key, unsealed and wrapped for this
/// user alone, as the Murmure spaces spec asks of a self-join.
pub(crate) fn join_key(
    channel_id: ChannelId,
    sealed: &SealedChannelKey,
    space_key: &DocumentKey,
    exchange_key: &StaticSecret,
) -> Result<String, String> {
    let channel_key = unseal_channel_key(channel_id, sealed, space_key)?;
    let own_pub = x25519_dalek::PublicKey::from(exchange_key);
    crate::sharing::wrap_key_b64(exchange_key, own_pub.as_bytes(), &channel_key)
        .map_err(|e| format!("wrap the channel key: {e}"))
}

/// Parse the activity answer. One row the client cannot read fails the whole
/// answer, as a listing does.
pub(crate) fn activity_of(
    entries: &[http::ActivityEntry],
) -> Result<Vec<DocumentActivity>, String> {
    entries
        .iter()
        .map(|e| {
            let channel_id = ChannelId::parse_str(&e.channel_id)
                .map_err(|_| format!("bad channel id {:?}", e.channel_id))?;
            let document_id = DocumentId::parse_str(&e.document_id)
                .map_err(|_| format!("bad document id {:?}", e.document_id))?;
            Ok(DocumentActivity {
                channel_id,
                document_id,
                active: e.active,
            })
        })
        .collect()
}

/// Parse the listing the relay answered. One row the client cannot read
/// fails the whole listing: a channel opened on part of its documents would
/// show a history with holes.
pub(crate) fn channel_documents(
    entries: &[http::ChannelDocumentEntry],
) -> Result<Vec<ChannelDocument>, String> {
    entries
        .iter()
        .map(|e| {
            let document_id = DocumentId::parse_str(&e.document_id)
                .map_err(|_| format!("bad document id {:?}", e.document_id))?;
            let kind = ChannelDocumentKind::parse(&e.kind)
                .ok_or_else(|| format!("unknown document kind {:?}", e.kind))?;
            Ok(ChannelDocument { document_id, kind })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(document_id: &str, kind: &str) -> http::ChannelDocumentEntry {
        http::ChannelDocumentEntry {
            document_id: document_id.to_owned(),
            kind: kind.to_owned(),
        }
    }

    #[test]
    fn a_listing_parses_into_its_documents_in_order() {
        let meta = DocumentId::new();
        let segment = DocumentId::new();
        let parsed = channel_documents(&[
            entry(&meta.to_string(), "meta"),
            entry(&segment.to_string(), "segment"),
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
        assert!(channel_documents(&[entry(&meta, "meta"), entry("nope", "segment")]).is_err());
        assert!(channel_documents(&[entry(&meta, "draft")]).is_err());
    }

    fn activity(channel_id: &str, document_id: &str, active: bool) -> http::ActivityEntry {
        http::ActivityEntry {
            channel_id: channel_id.to_owned(),
            document_id: document_id.to_owned(),
            active,
        }
    }

    #[test]
    fn an_activity_answer_parses_into_its_documents() {
        let (channel, segment) = (ChannelId::new(), DocumentId::new());
        let parsed =
            activity_of(&[activity(&channel.to_string(), &segment.to_string(), true)]).expect("ok");
        assert_eq!(
            parsed,
            [DocumentActivity {
                channel_id: channel,
                document_id: segment,
                active: true
            }]
        );
        let bad = activity(&channel.to_string(), "nope", false);
        assert!(activity_of(&[bad]).is_err(), "one bad row fails the answer");
    }

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
        ambre_crypto::ecdh::unwrap_document_key(exchange_key, own_pub.as_bytes(), &wrapped).ok()
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
        let (sealed, alice_enc) = new_channel_key(channel_id, &space_key, &alice).expect("create");
        let channel_key = unwrap_own(&alice, &alice_enc).expect("Alice opens her own wrap");

        let bob_enc = join_key(channel_id, &sealed, &space_key, &bob).expect("join");
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
        let (sealed, _) =
            new_channel_key(channel_id, &space_key, &exchange_key(1)).expect("create");
        assert_eq!(sealed.as_bytes().len(), NONCE_LEN + 32 + 16);
        assert!(unseal_channel_key(channel_id, &sealed, &space_key).is_ok());
        assert!(unseal_channel_key(channel_id, &sealed, &DocumentKey::generate()).is_err());
        assert!(unseal_channel_key(ChannelId::new(), &sealed, &space_key).is_err());
        let short = SealedChannelKey::from_bytes(vec![0; 8]);
        assert!(unseal_channel_key(channel_id, &short, &space_key).is_err());
    }

    /// With no relay, a creation and a join fail at once, and say so.
    #[test]
    fn with_no_relay_a_creation_and_a_join_fail_at_once() {
        let (sink, rx) = LogicSink::channel();
        let state = SyncState::new(sink, String::new(), None);
        let (channel_id, space_id) = (ChannelId::new(), SpaceId::new());
        state.handle_create_public_channel(channel_id, space_id, MetaId::new(), SegmentId::new());
        state.handle_join_channel(
            channel_id,
            space_id,
            &SealedChannelKey::from_bytes(vec![1; 72]),
        );
        let events: Vec<LogicEvent> = rx.try_iter().collect();
        assert_eq!(events.len(), 2);
        assert!(events.iter().all(|e| matches!(
            e,
            LogicEvent::ChannelRequestFailed { channel_id: id, .. } if *id == channel_id
        )));
    }

    #[test]
    fn with_no_relay_the_open_fails_at_once() {
        let (sink, rx) = LogicSink::channel();
        let state = SyncState::new(sink, String::new(), None);
        let channel_id = ChannelId::new();
        state.handle_open_channel(channel_id);
        let events: Vec<LogicEvent> = rx.try_iter().collect();
        assert!(matches!(
            events.as_slice(),
            [LogicEvent::ChannelOpenFailed { channel_id: id, .. }] if *id == channel_id
        ));
    }
}
