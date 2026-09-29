// ambre-logic/src/channels.rs
// ============================================================================
// Murmure channels: route a channel command to its engine, and a remote delta
// to the channel that holds its document. A channel is a set of documents,
// never an entry of `docs` (the Murmure engine spec).
// ============================================================================

use super::*;
use ambre_bus::{ChannelId, MurmureCommand};
#[cfg(test)]
use ambre_murmure::ChannelState;

impl LogicState {
    /// Run one command on an open channel, and emit what it produced
    /// (`BUS-027`).
    ///
    /// The delta leaves for the Sync worker under the document it names, with
    /// its activity summary (the Murmure messages spec). The event goes to the view.
    pub(crate) fn handle_channel_command(
        &mut self,
        channel_id: ChannelId,
        command: MurmureCommand,
    ) {
        let Some(channel) = self.channels.get_mut(&channel_id) else {
            tracing::warn!("Logic: command for channel {channel_id}, which is not open; dropped");
            return;
        };
        match command {
            MurmureCommand::SendMessage { draft, now } => {
                // The author is this session's user. Before `SessionReady`
                // there is none, and a message with no author is refused
                // rather than written under a placeholder (the Murmure schema).
                let Some(author) = self.my_user_id else {
                    tracing::warn!(
                        "Logic: message for channel {channel_id} before the session; dropped"
                    );
                    return;
                };
                match channel.send(&draft, author, now) {
                    Ok(sent) => {
                        self.send_to_sync(SyncCommand::SendUpdate {
                            document_id: sent.segment,
                            payload: sent.delta,
                            summary: Some(sent.summary),
                        });
                        self.send_to_ui(UiEvent::Channel {
                            channel_id,
                            event: sent.event,
                        });
                    }
                    Err(error) => {
                        tracing::warn!("Logic: send on channel {channel_id} refused: {error}");
                    }
                }
            }
        }
    }

    /// Apply remote payloads to the channel that holds `document_id`
    /// (the Murmure engine spec). Returns `false` when no open channel holds it, so the
    /// caller reports the drop as it does for a document.
    pub(crate) fn apply_channel_remote(
        &mut self,
        document_id: DocumentId,
        payloads: &[&[u8]],
    ) -> bool {
        let Some((&channel_id, channel)) = self
            .channels
            .iter_mut()
            .find(|(_, channel)| channel.holds(document_id))
        else {
            return false;
        };
        let mut events = Vec::new();
        for payload in payloads {
            match channel.apply_remote(document_id, payload) {
                Ok(produced) => events.extend(produced),
                Err(error) => {
                    tracing::warn!("Logic: delta for {document_id} refused: {error}");
                }
            }
        }
        // One segment answers with its whole list, so only the last answer of
        // a batch is news to the view.
        if let Some(event) = events.pop() {
            self.send_to_ui(UiEvent::Channel { channel_id, event });
        }
        true
    }

    /// Hold an open channel.
    ///
    /// Nothing calls this in production yet: the documents of a channel reach
    /// the Logic thread with the channel subscription of `SYNC P4`. Tests seed
    /// the registry through it meanwhile.
    #[cfg(test)]
    pub(crate) fn hold_channel(&mut self, channel: ChannelState) {
        self.channels.insert(channel.channel_id(), channel);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ambre_bus::{
        ActivityKind, MentionTarget, MessageDraft, MurmureEvent, RestEvent, Timestamp,
    };
    use ambre_murmure::ChannelType;

    const NOW: Timestamp = Timestamp(1_700_000_000_000);

    fn harness() -> (LogicState, Receiver<UiEvent>, Receiver<SyncCommand>) {
        let (to_ui, ui_rx) = test_link();
        let (to_sync, sync_rx) = test_link();
        let (to_persist, persist_rx) = test_link();
        std::mem::forget(persist_rx);
        (LogicState::new(to_ui, to_sync, to_persist), ui_rx, sync_rx)
    }

    fn signed_in(state: &mut LogicState, user: UserId) {
        state.handle_logic_event(LogicEvent::Rest {
            id: RequestId(0),
            evt: RestEvent::SessionReady {
                user_id: user,
                display_name: "alice".to_owned(),
                email: "alice@example.com".to_owned(),
            },
        });
    }

    fn open_channel(state: &mut LogicState) -> (ChannelId, DocumentId) {
        let channel_id = ChannelId::new();
        let channel =
            ChannelState::create(channel_id, ChannelType::Public, None, NOW).expect("create");
        let active = channel.active_id();
        state.hold_channel(channel);
        (channel_id, active)
    }

    fn send(text: &str, mentions: Vec<MentionTarget>) -> MurmureCommand {
        MurmureCommand::SendMessage {
            draft: MessageDraft {
                text: text.to_owned(),
                mentions,
            },
            now: NOW,
        }
    }

    #[test]
    fn a_channel_command_reaches_its_engine_and_leaves_as_a_delta_and_an_event() {
        let (mut state, ui, sync) = harness();
        let alice = UserId::new();
        signed_in(&mut state, alice);
        let _ = std::iter::from_fn(|| ui.try_recv().ok()).count();
        let (channel_id, active) = open_channel(&mut state);
        let bob = UserId::new();

        state.handle_logic_command(LogicCommand::Channel {
            channel_id,
            command: send("hi @bob", vec![MentionTarget::User(bob)]),
        });

        let sent: Vec<SyncCommand> = std::iter::from_fn(|| sync.try_recv().ok()).collect();
        let [
            SyncCommand::SendUpdate {
                document_id,
                payload,
                summary: Some(summary),
            },
        ] = sent.as_slice()
        else {
            panic!("one update with a summary, got {sent:?}");
        };
        assert_eq!(*document_id, active);
        assert!(!payload.is_empty());
        assert_eq!(summary.kind, ActivityKind::Content);
        assert_eq!(summary.mentions, [bob]);

        let shown: Vec<UiEvent> = std::iter::from_fn(|| ui.try_recv().ok()).collect();
        let [
            UiEvent::Channel {
                channel_id: shown_channel,
                event: MurmureEvent::Messages { segment, messages },
            },
        ] = shown.as_slice()
        else {
            panic!("one channel event, got {shown:?}");
        };
        assert_eq!(*shown_channel, channel_id);
        assert_eq!(*segment, active);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].author, alice);
        assert_eq!(messages[0].text, "hi @bob");
    }

    #[test]
    fn a_message_before_the_session_is_dropped() {
        let (mut state, ui, sync) = harness();
        let (channel_id, _) = open_channel(&mut state);
        state.handle_logic_command(LogicCommand::Channel {
            channel_id,
            command: send("too early", Vec::new()),
        });
        assert!(sync.try_recv().is_err());
        assert!(ui.try_recv().is_err());
    }

    #[test]
    fn a_remote_delta_reaches_the_channel_that_holds_its_document() {
        let (mut alice, _, alice_sync) = harness();
        signed_in(&mut alice, UserId::new());
        let channel_id = ChannelId::new();
        let original =
            ChannelState::create(channel_id, ChannelType::Public, None, NOW).expect("create");
        let active = original.active_id();
        let segments =
            std::collections::HashMap::from([(active, original.snapshot(active).expect("active"))]);
        let meta = original.snapshot(original.meta_id()).expect("meta");
        let copy =
            ChannelState::open(channel_id, original.meta_id(), &meta, &segments).expect("open");
        alice.hold_channel(original);
        let (mut bob, bob_ui, _) = harness();
        bob.hold_channel(copy);

        alice.handle_logic_command(LogicCommand::Channel {
            channel_id,
            command: send("over the wire", Vec::new()),
        });
        let Ok(SyncCommand::SendUpdate {
            document_id,
            payload,
            ..
        }) = alice_sync.try_recv()
        else {
            panic!("alice sends one update");
        };
        // The PeerID Sync would derive from alice's signer.
        let peer = *loro::LoroDoc::decode_import_blob_meta(&payload, false)
            .expect("an update")
            .partial_end_vv
            .keys()
            .next()
            .expect("one peer");
        bob.handle_logic_event(LogicEvent::RemoteUpdate {
            document_id,
            delta: ambre_bus::SignedDelta {
                seq: 1,
                payload,
                nonce: ambre_core::SessionNonce([0; 16]),
                peer,
                user: None,
            },
        });

        let Ok(UiEvent::Channel {
            event: MurmureEvent::Messages { messages, .. },
            ..
        }) = bob_ui.try_recv()
        else {
            panic!("bob's view hears of the message");
        };
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].text, "over the wire");
    }

    #[test]
    fn closing_a_channel_drops_its_documents() {
        let (mut state, _, sync) = harness();
        let (channel_id, active) = open_channel(&mut state);
        state.handle_logic_command(LogicCommand::CloseChannel { channel_id });
        assert!(state.channels.is_empty());
        assert!(!state.apply_channel_remote(active, &[&[]]));
        assert!(matches!(
            sync.try_recv(),
            Ok(SyncCommand::CloseChannel { .. })
        ));
    }
}
