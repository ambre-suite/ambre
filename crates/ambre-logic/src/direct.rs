// ambre-logic/src/direct.rs
// ============================================================================
// Murmure direct messages (the Murmure channels spec). A request names its
// members, and the relay answers with the direct message that holds exactly
// them, created or found. A direct message belongs to no space, so nothing
// here touches a catalogue. No key reaches this thread: the Sync worker wraps
// the channel key for each member.
// ============================================================================

use super::*;
use ambre_bus::{Authored, DirectMember, Instant, Role};
use ambre_murmure::message::{DirectMessageDraft, DirectMessageEvent, MurmureEvent};
use ambre_murmure::{ChannelState, ChannelType};

use crate::channels::HeldCommand;

impl LogicState {
    /// Ask the relay for the direct message of this user and the draft's
    /// members. The channel it would create is built now, unnamed, and held
    /// until the relay answers.
    pub(crate) fn open_direct_message(
        &mut self,
        draft: DirectMessageDraft,
        now: Instant<Authored>,
    ) {
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
    pub(crate) fn direct_message_opened(
        &mut self,
        requested: Id<Channel>,
        channel_id: Id<Channel>,
    ) {
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
    pub(crate) fn direct_failed(&self, requested: Id<Channel>, message: String) {
        tracing::warn!("Logic: direct message {requested}: {message}");
        self.send_to_ui(UiEvent::DirectMessage {
            requested,
            event: DirectMessageEvent::Failed { message },
        });
    }

    /// An invite into a direct message of two announces the member who joins
    /// first (the Murmure channels spec). The notice leaves before the invite,
    /// on the same queue to Sync, so it is written before the invite applies.
    ///
    /// A direct message whose members are not listed yet announces too: a
    /// notice too many is harmless, a missing one is not. Returns `true` when
    /// the invite does not leave now. It waits for the active segment to be
    /// writable, or it is refused because the notice could not be written.
    pub(crate) fn announce_before_invite(&mut self, id: Seq<Request>, cmd: &RestCommand) -> bool {
        let RestCommand::InviteToDocument {
            document_id,
            invitee,
            now,
            ..
        } = cmd
        else {
            return false;
        };
        let Some(channel_id) = self.channel_of_meta(*document_id) else {
            return false;
        };
        let Some(channel) = self.channels.get(&channel_id) else {
            return false;
        };
        let of_two = self
            .channel_members
            .get(&channel_id)
            .is_none_or(|members| members.len() == 2);
        if channel.channel_type() != Some(ChannelType::Direct) || !of_two {
            return false;
        }
        let active = channel.active_id().document_id();
        if !self.murmure_writable(active, channel.identified(active)) {
            self.wait_for_session(active, HeldCommand::Invite(id, cmd.clone()));
            return true;
        }
        let announced = match (self.my_user_id, self.channels.get_mut(&channel_id)) {
            (Some(author), Some(channel)) => channel
                .announce(*invitee, author, *now)
                .map_err(|error| error.to_string()),
            _ => Err("no session".to_owned()),
        };
        match announced {
            Ok(sent) => {
                self.send_to_sync(SyncCommand::SendUpdate {
                    document_id: sent.segment.document_id(),
                    payload: sent.delta,
                    summary: Some(sent.summary),
                });
                self.send_to_ui(UiEvent::Channel {
                    channel_id,
                    event: sent.event,
                });
                false
            }
            Err(message) => {
                self.refuse_invite(
                    id,
                    format!("the notice of the new member failed: {message}"),
                );
                true
            }
        }
    }

    /// An invite asked from an open channel goes on its `meta`, as an editor,
    /// and takes the path of every invite, notice included. A member the
    /// channel already lists is refused: a second invite would wrap the key
    /// again, and announce a member who never left.
    pub(crate) fn invite_to_channel(
        &mut self,
        id: Seq<Request>,
        channel_id: Id<Channel>,
        invitee: DirectMember,
        now: Instant<Authored>,
    ) {
        let Some(meta) = self.channels.get(&channel_id).map(|c| c.meta_id()) else {
            return self.refuse_invite(id, "the channel is not open".to_owned());
        };
        let listed = self
            .channel_members
            .get(&channel_id)
            .is_some_and(|members| members.iter().any(|m| m.user_id == invitee.user_id));
        if listed {
            return self.refuse_invite(id, "already a member of the channel".to_owned());
        }
        let cmd = RestCommand::InviteToDocument {
            document_id: meta.document_id(),
            invitee_email: None,
            invitee: invitee.user_id,
            role: Role::Editor,
            expect_exchange_pub_key: invitee.exchange_pub_key,
            now,
        };
        self.handle_logic_command(LogicCommand::Rest { id, cmd });
    }

    /// Refuse an invite this thread did not let leave.
    pub(crate) fn refuse_invite(&self, id: Seq<Request>, message: String) {
        tracing::warn!("Logic: invite refused: {message}");
        self.send_to_ui(UiEvent::ApiError {
            id: Some(id),
            request: ambre_bus::ApiRequest::InviteToDocument,
            failure: ambre_bus::ApiFailure::Local,
            message,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ambre_bus::DirectMember;
    use ambre_murmure::message::{MurmureCommand, Notice};

    const NOW: Instant<Authored> = Instant::<Authored>::new(1_700_000_000_000);

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
            user_id: Id::<User>::mint(),
            exchange_pub_key: FixedBytes::from_array([9; 32]),
        }
    }

    /// Ask for a direct message with `members`, and answer the ids the Sync
    /// worker was asked to create under.
    fn request(
        state: &mut LogicState,
        sync_rx: &Receiver<SyncCommand>,
        members: Vec<DirectMember>,
    ) -> (Id<Channel>, Id<Meta>, Id<Segment>) {
        let channel_id = Id::<Channel>::mint();
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
        let opened: Vec<Id<Document>> = drain(&sync_rx)
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
            assert!(
                crate::sent_by(payload.as_bytes(), peer),
                "only {peer:#x} wrote it"
            );
        }
    }

    /// The relay held a direct message of these members already: it opens,
    /// and the channel built at the request is dropped with nothing sent.
    #[test]
    fn an_existing_direct_message_opens_in_place_of_the_new_one() {
        let (mut state, ui_rx, sync_rx) = harness();
        let (requested, meta, _) = request(&mut state, &sync_rx, vec![bob()]);
        let existing = Id::<Channel>::mint();

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

    /// A direct message the relay created, with `listed` members, and this
    /// user signed in. Its documents have no identity yet.
    fn created_with(
        state: &mut LogicState,
        sync_rx: &Receiver<SyncCommand>,
        listed: usize,
    ) -> (Id<Channel>, Id<Meta>, Id<Segment>) {
        let (channel_id, meta, segment) = request(state, sync_rx, vec![bob()]);
        state.handle_logic_event(LogicEvent::DirectMessageOpened {
            requested: channel_id,
            channel_id,
        });
        state.my_user_id = Some(Id::<User>::mint());
        let member = |_| ambre_bus::MemberInfo {
            user_id: Id::<User>::mint(),
            display_name: ambre_core::Validated::<ambre_core::DisplayName>::parse("M").unwrap(),
            email: ambre_core::Email::from("m@test.local"),
            role: ambre_bus::Role::Editor,
            standing: ambre_bus::MemberStanding::Verified,
        };
        state
            .channel_members
            .insert(channel_id, (0..listed).map(member).collect());
        drain(sync_rx);
        (channel_id, meta, segment)
    }

    fn invite(document_id: Id<Document>, invitee: Id<User>) -> RestCommand {
        RestCommand::InviteToDocument {
            document_id,
            invitee_email: Some(ambre_core::Email::from("carol@test.local")),
            invitee,
            role: ambre_bus::Role::Editor,
            expect_exchange_pub_key: FixedBytes::from_array([3; 32]),
            now: NOW,
        }
    }

    /// Ready `document_id` under a fresh identity, as Sync would.
    fn ready(state: &mut LogicState, document_id: Id<Document>) {
        state.grant_identity(document_id);
        state.handle_logic_event(LogicEvent::DocumentReady {
            document_id,
            role: ambre_bus::Role::Editor,
        });
    }

    /// An invite into a direct message of two waits for its active segment,
    /// then leaves behind the notice that names the member who joins.
    #[test]
    fn an_invite_into_a_direct_message_of_two_leaves_behind_its_notice() {
        let (mut state, ui_rx, sync_rx) = harness();
        let (channel_id, meta, segment) = created_with(&mut state, &sync_rx, 2);
        let (id, carol) = (ambre_bus::next_request(), Id::<User>::mint());

        state.handle_logic_command(LogicCommand::Rest {
            id,
            cmd: invite(meta.document_id(), carol),
        });
        assert!(
            drain(&sync_rx).is_empty(),
            "the invite waits for its notice"
        );

        ready(&mut state, segment.document_id());
        let sent = drain(&sync_rx);
        let notice = sent.iter().position(|c| {
            matches!(c, SyncCommand::SendUpdate { document_id, summary: Some(_), .. }
                if *document_id == segment.document_id())
        });
        let invited = sent.iter().position(|c| {
            matches!(c, SyncCommand::Rest { id: sent, cmd: RestCommand::InviteToDocument { .. } }
                if *sent == id)
        });
        assert!(
            matches!((notice, invited), (Some(n), Some(i)) if n < i),
            "the notice, then the invite: {sent:?}"
        );
        let shown = drain(&ui_rx).into_iter().any(|e| match e {
            UiEvent::Channel {
                channel_id: on,
                event: MurmureEvent::Messages { messages, .. },
            } => {
                on == channel_id
                    && messages[0].notice == Some(Notice::MemberAdded { member: carol })
            }
            _ => false,
        });
        assert!(shown, "the view draws the notice");
    }

    /// A direct message of three, or a channel of no direct message, takes
    /// the invite with no notice.
    #[test]
    fn an_invite_into_a_larger_direct_message_writes_no_notice() {
        let (mut state, _ui_rx, sync_rx) = harness();
        let (_, meta, segment) = created_with(&mut state, &sync_rx, 3);
        ready(&mut state, segment.document_id());
        drain(&sync_rx);

        let id = ambre_bus::next_request();
        state.handle_logic_command(LogicCommand::Rest {
            id,
            cmd: invite(meta.document_id(), Id::<User>::mint()),
        });
        assert!(matches!(
            drain(&sync_rx).as_slice(),
            [SyncCommand::Rest { id: sent, cmd: RestCommand::InviteToDocument { .. } }] if *sent == id
        ));
    }

    /// An invite held for its notice is refused when the session of the
    /// segment ends, and never leaves.
    #[test]
    fn an_invite_whose_notice_cannot_be_written_is_refused() {
        let (mut state, ui_rx, sync_rx) = harness();
        let (_, meta, segment) = created_with(&mut state, &sync_rx, 2);
        let id = ambre_bus::next_request();
        state.handle_logic_command(LogicCommand::Rest {
            id,
            cmd: invite(meta.document_id(), Id::<User>::mint()),
        });
        state.forget_murmure_session(segment.document_id());

        assert!(drain(&sync_rx).is_empty(), "no invite leaves");
        assert!(drain(&ui_rx).iter().any(|e| matches!(
            e,
            UiEvent::ApiError { id: Some(refused), failure: ambre_bus::ApiFailure::Local, .. } if *refused == id
        )));
    }

    /// An invite asked from the open channel leaves on its `meta`, by id, as
    /// an editor, with the key the user confirmed. A direct message of two
    /// writes its notice first.
    #[test]
    fn an_invite_from_the_channel_leaves_on_its_meta_after_its_notice() {
        let (mut state, _ui_rx, sync_rx) = harness();
        let (channel_id, meta, segment) = created_with(&mut state, &sync_rx, 2);
        ready(&mut state, segment.document_id());
        drain(&sync_rx);
        let (id, carol) = (ambre_bus::next_request(), bob());

        state.handle_logic_command(LogicCommand::InviteToChannel {
            id,
            channel_id,
            invitee: carol.clone(),
            now: NOW,
        });
        let sent = drain(&sync_rx);
        let notice = sent.iter().position(|c| {
            matches!(
                c,
                SyncCommand::SendUpdate {
                    summary: Some(_),
                    ..
                }
            )
        });
        let invited = sent.iter().position(|c| {
            matches!(c, SyncCommand::Rest {
                id: sent,
                cmd: RestCommand::InviteToDocument {
                    document_id,
                    invitee_email: None,
                    invitee,
                    role: ambre_bus::Role::Editor,
                    expect_exchange_pub_key,
                    now,
                },
            } if *sent == id
                && *now == NOW
                && *document_id == meta.document_id()
                && *invitee == carol.user_id
                && *expect_exchange_pub_key == carol.exchange_pub_key)
        });
        assert!(
            matches!((notice, invited), (Some(n), Some(i)) if n < i),
            "the notice, then the invite on meta: {sent:?}"
        );
    }

    /// A member the channel lists, or a channel not open, is refused, and
    /// nothing leaves.
    #[test]
    fn an_invite_of_a_listed_member_is_refused_and_nothing_leaves() {
        let (mut state, ui_rx, sync_rx) = harness();
        let (channel_id, _, segment) = created_with(&mut state, &sync_rx, 2);
        ready(&mut state, segment.document_id());
        drain(&sync_rx);
        drain(&ui_rx);
        let listed = state.channel_members[&channel_id][0].user_id;
        let (first, second) = (ambre_bus::next_request(), ambre_bus::next_request());

        let member = DirectMember {
            user_id: listed,
            exchange_pub_key: FixedBytes::from_array([9; 32]),
        };
        state.handle_logic_command(LogicCommand::InviteToChannel {
            id: first,
            channel_id,
            invitee: member,
            now: NOW,
        });
        state.handle_logic_command(LogicCommand::InviteToChannel {
            id: second,
            channel_id: Id::<Channel>::mint(),
            invitee: bob(),
            now: NOW,
        });

        assert!(drain(&sync_rx).is_empty(), "no notice and no invite leave");
        let refused: Vec<_> = drain(&ui_rx)
            .into_iter()
            .filter_map(|e| match e {
                UiEvent::ApiError {
                    id: Some(id),
                    failure: ambre_bus::ApiFailure::Local,
                    ..
                } => Some(id),
                _ => None,
            })
            .collect();
        assert_eq!(refused, [first, second]);
    }

    /// A name leaves as a delta on `meta`, and the view gets it. Clearing it
    /// leaves the view with no name. Each name is also sealed beside meta, so
    /// it shows before the channel opens (the Murmure layout spec).
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

        // The name is trimmed once: the sealed one is the one `meta` holds.
        for (name, shown) in [
            (Some("trip"), Some("trip")),
            (Some("  trip "), Some("trip")),
            (None, None),
        ] {
            state.handle_logic_command(LogicCommand::Channel {
                channel_id,
                command: MurmureCommand::Name {
                    name: name
                        .map(|n| ambre_core::Validated::<ambre_core::Name>::parse(n).unwrap()),
                },
            });
            assert!(matches!(
                drain(&sync_rx).as_slice(),
                [
                    SyncCommand::SendUpdate { document_id, summary: None, .. },
                    SyncCommand::NameChannel { meta: sealed, name: sealed_name },
                ] if *document_id == meta.document_id() && *sealed == meta
                    && sealed_name.as_ref().map(ambre_core::Validated::as_str) == shown
            ));
            assert!(matches!(
                drain(&ui_rx).as_slice(),
                [UiEvent::Channel {
                    channel_id: id,
                    event: MurmureEvent::Named { name },
                }] if *id == channel_id && name.as_ref().map(ambre_core::Validated::as_str) == shown
            ));
        }
    }
}
