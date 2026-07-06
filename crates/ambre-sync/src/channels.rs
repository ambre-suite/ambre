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

    pub(crate) fn handle_send_channel_update(&self, channel_id: ChannelId, payload: Vec<u8>) {
        tracing::debug!(
            "Sync: SendChannelUpdate {channel_id:?} len={}",
            payload.len()
        );
    }

    pub(crate) fn handle_send_channel_ephemeral(&self, channel_id: ChannelId, payload: Vec<u8>) {
        tracing::debug!(
            "Sync: SendChannelEphemeral {channel_id:?} len={}",
            payload.len()
        );
    }
}
