use super::*;
use ambre_bus::{ChannelDocument, ChannelDocumentKind};

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
