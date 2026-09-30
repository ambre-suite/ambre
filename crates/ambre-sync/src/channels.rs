use super::*;

impl SyncState {
    // -- Channels --------------------------------------------------------------

    pub(crate) fn handle_open_channel(&mut self, channel_id: ChannelId) {
        if self.ws_connected() {
            self.ws_send(&WsClientMsg::SubscribeChannel { channel_id });
        }
        tracing::info!("Sync: OpenChannel {channel_id:?}");
    }

    pub(crate) fn handle_close_channel(&mut self, channel_id: ChannelId) {
        if self.ws_connected() {
            self.ws_send(&WsClientMsg::UnsubscribeChannel { channel_id });
        }
        tracing::info!("Sync: CloseChannel {channel_id:?}");
    }

    /// A channel message has no path to the relay yet: it is refused, and the
    /// Logic thread is told, never dropped in silence (`SYNC-069`).
    pub(crate) fn handle_send_channel_update(&self, channel_id: ChannelId, payload: Vec<u8>) {
        self.refuse_channel_send(channel_id, "update", payload.len());
    }

    /// See [`handle_send_channel_update`](Self::handle_send_channel_update).
    pub(crate) fn handle_send_channel_ephemeral(&self, channel_id: ChannelId, payload: Vec<u8>) {
        self.refuse_channel_send(channel_id, "ephemeral", payload.len());
    }

    fn refuse_channel_send(&self, channel_id: ChannelId, kind: &str, len: usize) {
        tracing::warn!(
            "Sync: channel {kind} refused for {channel_id:?} ({len} bytes), no relay path"
        );
        self.send_to_logic(LogicEvent::ApiError {
            id: None,
            request: format!("channel {kind}"),
            code: CHANNEL_SEND_UNAVAILABLE,
            message: "the message was not sent: channels are not available yet".to_owned(),
        });
    }
}

/// The code a refused channel send carries: the relay route does not exist,
/// as an HTTP 501 would say.
const CHANNEL_SEND_UNAVAILABLE: u16 = 501;

#[cfg(test)]
mod tests {
    use super::*;

    /// A channel message is refused and said, never dropped in silence
    /// (`SYNC-069`), on both send paths.
    #[test]
    fn a_channel_message_is_refused_not_dropped() {
        let (sink, rx) = LogicSink::channel();
        let state = SyncState::new(sink, String::new(), None);
        let channel = ChannelId::new();
        state.handle_send_channel_update(channel, b"hello".to_vec());
        state.handle_send_channel_ephemeral(channel, b"typing".to_vec());
        let refusals = rx
            .try_iter()
            .filter(|e| {
                matches!(
                    e,
                    LogicEvent::ApiError {
                        code: CHANNEL_SEND_UNAVAILABLE,
                        ..
                    }
                )
            })
            .count();
        assert_eq!(refusals, 2);
    }
}
