use super::*;
use ambre_bus::{ChannelDocument, ChannelDocumentKind, DocumentActivity};

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
    pub(crate) fn handle_mark_read(&self, live: &[DocumentId], finals: Vec<DocumentId>) {
        let cursors: Vec<(DocumentId, Seq)> = live
            .iter()
            .filter_map(|&id| self.read_up_to(id).map(|seq| (id, seq)))
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
