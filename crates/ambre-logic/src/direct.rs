// ambre-logic/src/direct.rs
// ============================================================================
// Murmure direct messages (the Murmure channels spec). A request names its
// members, and the relay answers with the direct message that holds exactly
// them, created or found. A direct message belongs to no space, so nothing
// here touches a catalogue. No key reaches this thread: the Sync worker wraps
// the channel key for each member.
// ============================================================================

use super::*;
use ambre_bus::{ChannelId, DirectMessageDraft, DirectMessageEvent, MurmureEvent, Timestamp};
use ambre_murmure::{ChannelState, ChannelType};

impl LogicState {
    /// Ask the relay for the direct message of this user and the draft's
    /// members. The channel it would create is built now, unnamed, and held
    /// until the relay answers.
    pub(crate) fn open_direct_message(&mut self, draft: DirectMessageDraft, now: Timestamp) {
        let DirectMessageDraft {
            channel_id,
            members,
        } = draft;
        if self.direct.contains_key(&channel_id) {
            return;
        }
        if members.is_empty() {
            return self.direct_failed(channel_id, "a direct message needs a member".into());
        }
        match ChannelState::create(channel_id, ChannelType::Direct, None, now) {
            Ok(channel) => {
                self.send_to_sync(SyncCommand::CreateDirectMessage {
                    channel_id,
                    meta: channel.meta_id(),
                    segment: channel.active_id(),
                    members,
                });
                self.direct.insert(channel_id, channel);
            }
            Err(error) => self.direct_failed(channel_id, error.to_string()),
        }
    }

    /// The relay answered a request with the direct message of its members.
    ///
    /// A new one is the channel built at the request: its content leaves as
    /// its first update. An existing one opens as any channel does, and the
    /// channel built at the request is dropped unsent. The request adds no
    /// member to it.
    pub(crate) fn direct_message_opened(&mut self, requested: ChannelId, channel_id: ChannelId) {
        let Some(channel) = self.direct.remove(&requested) else {
            return;
        };
        if channel_id == requested {
            self.hold_created(channel);
            self.send_to_ui(UiEvent::Channel {
                channel_id,
                event: MurmureEvent::Named { name: None },
            });
        } else {
            self.open_channel(channel_id);
        }
        self.send_to_ui(UiEvent::DirectMessage {
            requested,
            event: DirectMessageEvent::Opened { channel_id },
        });
    }

    /// A request failed, and the relay changed nothing.
    pub(crate) fn direct_failed(&self, requested: ChannelId, message: String) {
        tracing::warn!("Logic: direct message {requested}: {message}");
        self.send_to_ui(UiEvent::DirectMessage {
            requested,
            event: DirectMessageEvent::Failed { message },
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ambre_bus::{DirectMember, MetaId, MurmureCommand, SegmentId, UserId};

    const NOW: Timestamp = Timestamp(1_700_000_000_000);

    fn harness() -> (LogicState, Receiver<UiEvent>, Receiver<SyncCommand>) {
        let (to_ui, ui_rx) = test_link();
        let (to_sync, sync_rx) = test_link();
        let (to_persist, persist_rx) = test_link();
        std::mem::forget(persist_rx);
        (LogicState::new(to_ui, to_sync, to_persist), ui_rx, sync_rx)
    }

    fn drain<T>(rx: &Receiver<T>) -> Vec<T> {
        std::iter::from_fn(|| rx.try_recv().ok()).collect()
    }

    fn bob() -> DirectMember {
        DirectMember {
            user_id: UserId::new(),
            exchange_pub_key: [9; 32],
        }
    }

    /// Ask for a direct message with `members`, and answer the ids the Sync
    /// worker was asked to create under.
    fn request(
        state: &mut LogicState,
        sync_rx: &Receiver<SyncCommand>,
        members: Vec<DirectMember>,
    ) -> (ChannelId, MetaId, SegmentId) {
        let channel_id = ChannelId::new();
        state.handle_logic_command(LogicCommand::OpenDirectMessage {
            draft: DirectMessageDraft {
                channel_id,
                members: members.clone(),
            },
            now: NOW,
        });
        let sent = drain(sync_rx);
        let [
            SyncCommand::CreateDirectMessage {
                channel_id: asked,
                meta,
                segment,
                members: carried,
            },
        ] = sent.as_slice()
        else {
            panic!("one creation request: {sent:?}");
        };
        assert_eq!((*asked, carried), (channel_id, &members));
        (channel_id, *meta, *segment)
    }

    /// The relay created the direct message: the channel built at the
    /// request is held, unnamed, and its two documents leave whole.
    #[test]
    fn a_created_direct_message_is_held_and_sent_whole() {
        let (mut state, ui_rx, sync_rx) = harness();
        let (channel_id, meta, segment) = request(&mut state, &sync_rx, vec![bob()]);

        state.handle_logic_event(LogicEvent::DirectMessageOpened {
            requested: channel_id,
            channel_id,
        });
        let opened: Vec<DocumentId> = drain(&sync_rx)
            .into_iter()
            .filter_map(|c| match c {
                SyncCommand::OpenDocument { document_id, .. } => Some(document_id),
                _ => None,
            })
            .collect();
        assert_eq!(opened, [meta.document_id(), segment.document_id()]);
        let channel = &state.channels[&channel_id];
        assert_eq!(channel.channel_type(), Some(ChannelType::Direct));
        assert_eq!(channel.name(), None, "nothing composed is stored");
        assert!(drain(&ui_rx).iter().any(|e| matches!(
            e,
            UiEvent::DirectMessage {
                requested,
                event: DirectMessageEvent::Opened { channel_id: id },
            } if *requested == channel_id && *id == channel_id
        )));

        // Each document leaves once its identity landed and the Sync worker
        // holds it: whole, and written under its own PeerID alone.
        for document_id in [meta.document_id(), segment.document_id()] {
            let peer = state.grant_identity(document_id);
            assert!(drain(&sync_rx).is_empty(), "nothing leaves before ready");
            state.handle_logic_event(LogicEvent::DocumentReady {
                document_id,
                role: ambre_bus::Role::Owner,
            });
            let sent = drain(&sync_rx);
            let [
                SyncCommand::SendUpdate {
                    document_id: up,
                    payload,
                    summary: None,
                },
            ] = sent.as_slice()
            else {
                panic!("one whole upload, got {sent:?}");
            };
            assert_eq!(*up, document_id);
            assert!(crate::sent_by(payload, peer), "only {peer:#x} wrote it");
        }
    }

    /// The relay held a direct message of these members already: it opens,
    /// and the channel built at the request is dropped with nothing sent.
    #[test]
    fn an_existing_direct_message_opens_in_place_of_the_new_one() {
        let (mut state, ui_rx, sync_rx) = harness();
        let (requested, meta, _) = request(&mut state, &sync_rx, vec![bob()]);
        let existing = ChannelId::new();

        state.handle_logic_event(LogicEvent::DirectMessageOpened {
            requested,
            channel_id: existing,
        });
        let sent = drain(&sync_rx);
        assert!(matches!(
            sent.as_slice(),
            [SyncCommand::OpenChannel { channel_id, listed_in: None }] if *channel_id == existing
        ));
        assert!(!state.channels.contains_key(&requested));
        assert!(!state.unsent.contains_key(&meta.document_id()));
        assert!(drain(&ui_rx).iter().any(|e| matches!(
            e,
            UiEvent::DirectMessage {
                event: DirectMessageEvent::Opened { channel_id },
                ..
            } if *channel_id == existing
        )));
    }

    /// A refused request tells the view, and holds nothing.
    #[test]
    fn a_refused_request_holds_nothing() {
        let (mut state, ui_rx, sync_rx) = harness();
        let (requested, _, _) = request(&mut state, &sync_rx, vec![bob()]);

        state.handle_logic_event(LogicEvent::ChannelRequestFailed {
            channel_id: requested,
            message: "key changed".into(),
        });
        assert!(state.direct.is_empty());
        assert!(!state.channels.contains_key(&requested));
        assert!(drain(&ui_rx).iter().any(|e| matches!(
            e,
            UiEvent::DirectMessage {
                event: DirectMessageEvent::Failed { .. },
                ..
            }
        )));
    }

    /// A name leaves as a delta on `meta`, and the view gets it. Clearing it
    /// leaves the view with no name.
    #[test]
    fn a_name_leaves_on_meta_and_reaches_the_view() {
        let (mut state, ui_rx, sync_rx) = harness();
        let (channel_id, meta, _) = request(&mut state, &sync_rx, vec![bob()]);
        state.handle_logic_event(LogicEvent::DirectMessageOpened {
            requested: channel_id,
            channel_id,
        });
        state.grant_identity(meta.document_id());
        state.handle_logic_event(LogicEvent::DocumentReady {
            document_id: meta.document_id(),
            role: ambre_bus::Role::Owner,
        });
        drain(&sync_rx);
        drain(&ui_rx);

        for (name, shown) in [(Some("trip"), Some("trip")), (None, None)] {
            state.handle_logic_command(LogicCommand::Channel {
                channel_id,
                command: MurmureCommand::Name {
                    name: name.map(str::to_owned),
                },
            });
            assert!(matches!(
                drain(&sync_rx).as_slice(),
                [SyncCommand::SendUpdate { document_id, summary: None, .. }]
                    if *document_id == meta.document_id()
            ));
            let shown = shown.map(str::to_owned);
            assert!(matches!(
                drain(&ui_rx).as_slice(),
                [UiEvent::Channel {
                    channel_id: id,
                    event: MurmureEvent::Named { name },
                }] if *id == channel_id && *name == shown
            ));
        }
    }
}
