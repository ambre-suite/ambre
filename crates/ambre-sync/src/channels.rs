use super::*;
use ambre_bus::{
    ChannelDocument, ChannelDocumentKind, DocumentActivity, MetaId, SealedChannelKey, SegmentId,
    SpaceId,
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
    /// `self_join` says whether a catalogue lists the channel; the listing
    /// answer then records which log admits a self-join (`SYNC-106`).
    pub(crate) fn handle_open_channel(&self, channel_id: ChannelId, self_join: bool) {
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
            let _ = tx.send(SyncEvent::ChannelListed {
                channel_id,
                self_join,
                result,
            });
        });
    }

    /// The answer of [`handle_open_channel`](Self::handle_open_channel), on
    /// the Sync loop.
    pub(crate) fn channel_listed(
        &mut self,
        channel_id: ChannelId,
        self_join: bool,
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
                    if self_join {
                        self.self_join_logs.insert(meta);
                    } else {
                        self.self_join_logs.remove(&meta);
                    }
                }
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

    /// The rule a walk of `document_id`'s log applies to a self-join: valid
    /// in the log of a channel a catalogue lists, and nowhere else
    /// (`SYNC-106`).
    pub(crate) fn self_join_rule(&self, document_id: DocumentId) -> membership_log::SelfJoinRule {
        if self.self_join_logs.contains(&document_id) {
            membership_log::public_channel_self_join
        } else {
            membership_log::no_self_join
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
                Some(self.space_keys(space_id)?.0)
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
                    let (doc_key_enc, entry) =
                        crate::sharing::creation(document_id, &channel_key, &exchange_key, &author)?;
                    Ok((document_id, doc_key_enc, entry))
                })
                .collect::<Result<Vec<_>, String>>()?;
            let sealed = match space_key {
                Some(space_key) => {
                    let meta_anchor = documents
                        .first()
                        .and_then(|(_, _, entry)| digest_of(entry))
                        .ok_or("meta's first entry does not decode")?;
                    Some(seal_channel_key(channel_id, &channel_key, &space_key, meta_anchor)?)
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
                        document_id: Some(document_id.to_string()),
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
            let (space_key, exchange_key) = self.space_keys(space_id)?;
            let author = self.author().ok_or("session locked")?;
            let (doc_key_enc, key_id) = join_key(channel_id, sealed, &space_key, &exchange_key)?;
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
                    documents: Some(finals.iter().map(ToString::to_string).collect()),
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
/// anchor of meta's log beside it (`SYNC-104`).
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
    let mut sealed = nonce.to_vec();
    sealed.extend_from_slice(&ciphertext);
    Ok(SealedChannelKey::from_bytes(sealed))
}

/// Unseal a channel key the catalogue holds, with the anchor sealed beside it.
pub(crate) fn unseal_channel_key(
    channel_id: ChannelId,
    sealed: &SealedChannelKey,
    space_key: &DocumentKey,
) -> Result<(DocumentKey, Option<ambre_protocol::EntryDigest>), String> {
    let bytes = sealed.as_bytes();
    let (nonce, ciphertext) = bytes
        .split_first_chunk::<NONCE_LEN>()
        .ok_or("the sealed channel key is too short")?;
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
    space_key: &DocumentKey,
    exchange_key: &StaticSecret,
) -> Result<(String, KeyId), String> {
    let (key, anchor) = unseal_channel_key(channel_id, sealed, space_key)?;
    let own_pub = x25519_dalek::PublicKey::from(exchange_key);
    let wrapped =
        crate::sharing::wrap_anchored_b64(exchange_key, own_pub.as_bytes(), &key, anchor)
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
    let meta = DocumentId::parse_str(&head.meta_id).map_err(|e| format!("join head: {e}"))?;
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
                read: e.read,
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
            Ok(ChannelDocument {
                document_id,
                kind: e.kind,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(document_id: &str, kind: ChannelDocumentKind) -> http::ChannelDocumentEntry {
        http::ChannelDocumentEntry {
            document_id: document_id.to_owned(),
            kind,
        }
    }

    #[test]
    fn a_listing_parses_into_its_documents_in_order() {
        let meta = DocumentId::new();
        let segment = DocumentId::new();
        let parsed = channel_documents(&[
            entry(&meta.to_string(), ChannelDocumentKind::Meta),
            entry(&segment.to_string(), ChannelDocumentKind::Segment),
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
        assert!(
            channel_documents(&[
                entry(&meta, ChannelDocumentKind::Meta),
                entry("nope", ChannelDocumentKind::Segment)
            ])
            .is_err()
        );
        // Single valid entry succeeds
        assert!(channel_documents(&[entry(&meta, ChannelDocumentKind::Meta)]).is_ok());
    }

    fn activity(channel_id: &str, document_id: &str, active: bool) -> http::ActivityEntry {
        http::ActivityEntry {
            channel_id: channel_id.to_owned(),
            document_id: document_id.to_owned(),
            active,
            read: Seq(7),
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
                active: true,
                read: Seq(7),
            }]
        );
        let bad = activity(&channel.to_string(), "nope", false);
        assert!(activity_of(&[bad]).is_err(), "one bad row fails the answer");
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
            ambre_crypto::ecdh::unwrap_anchored(exchange_key, own_pub.as_bytes(), &wrapped)
                .ok()?;
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
        let sealed =
            seal_channel_key(channel_id, &channel_key, space_key, ANCHOR).expect("seal");
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

        let (bob_enc, key_id) = join_key(channel_id, &sealed, &space_key, &bob).expect("join");
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
        assert_eq!(sealed.as_bytes().len(), NONCE_LEN + 32 + 32 + 16);
        let (_, anchor) = unseal_channel_key(channel_id, &sealed, &space_key).expect("open");
        assert_eq!(anchor, Some(ANCHOR));
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

    #[test]
    fn with_no_relay_the_open_fails_at_once() {
        let (sink, rx) = LogicSink::channel();
        let state = SyncState::new(sink, String::new(), None);
        let channel_id = ChannelId::new();
        state.handle_open_channel(channel_id, false);
        let events: Vec<LogicEvent> = rx.try_iter().collect();
        assert!(matches!(
            events.as_slice(),
            [LogicEvent::ChannelOpenFailed { channel_id: id, .. }] if *id == channel_id
        ));
    }
}
