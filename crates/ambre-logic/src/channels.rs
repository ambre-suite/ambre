// Host tests of the Murmure runtime, driven through the Logic dispatcher.

use super::*;
use ambre_bus::{ChannelActivity, ChannelDocument, ChannelDocumentKind, DocumentActivity};
use ambre_bus::{MemberInfo, Mentions, OpenFailure, Relay, Seq, SignedDelta, Unread};
use ambre_murmure::message::Unreadable;
use ambre_murmure::message::{HeldReply, HeldSend, MessageAt, MurmureCommand};
use ambre_murmure::{ChannelState, Position, Sample, ThreadTarget};

#[cfg(test)]
mod tests {
    use super::*;
    use ambre_bus::{
        ActivityKind, Authored, ContentHash, FoldAnswer, Instant, LeaseAnswer, MediaType, Octets,
        RestCommand, RestEvent,
    };
    use ambre_core::Char;
    use ambre_core::Count;
    // The preview tests that size a picture run on the host alone, where the
    // images engine is linked.
    #[cfg(not(target_arch = "wasm32"))]
    use ambre_core::Px;
    use ambre_core::Validated;
    use ambre_core::id::Lease;
    use ambre_core::index::Span;
    use ambre_core::{Ms, TimeSpan};
    use ambre_murmure::ChannelType;
    use ambre_murmure::message::{Attachment, Mention, MentionTarget, MessageDraft, MurmureEvent};

    const NOW: Instant<Authored> = Instant::<Authored>::new(1_700_000_000_000);

    fn harness() -> (LogicState, Receiver<UiEvent>, Receiver<SyncCommand>) {
        let (state, ui, sync, store) = harness_with_store();
        std::mem::forget(store);
        (state, ui, sync)
    }

    /// What Logic hands the local store.
    ///
    /// Written out at its uses, the type fails `clippy::type_complexity`.
    type StoreRx = Receiver<(Id<Document>, PersistWrite)>;

    /// The harness, with what Logic hands the local store.
    fn harness_with_store() -> (
        LogicState,
        Receiver<UiEvent>,
        Receiver<SyncCommand>,
        StoreRx,
    ) {
        let (to_ui, ui_rx) = test_link();
        let (to_sync, sync_rx) = test_link();
        let (to_persist, persist_rx) = test_link();
        (
            LogicState::new(to_ui, to_sync, to_persist),
            ui_rx,
            sync_rx,
            persist_rx,
        )
    }

    fn signed_in(state: &mut LogicState, user: Id<User>) {
        state.handle_logic_event(LogicEvent::Rest {
            id: None,
            evt: RestEvent::SessionReady {
                user_id: user,
                display_name: ambre_core::Validated::<ambre_core::DisplayName>::parse("alice")
                    .unwrap(),
                email: ambre_core::Email::from("alice@example.com"),
            },
        });
    }

    fn open_channel(state: &mut LogicState) -> (Id<Channel>, Id<Segment>) {
        let channel_id = Id::<Channel>::mint();
        let channel =
            ChannelState::create(channel_id, ChannelType::Public, None, NOW).expect("create");
        let active = channel.active_id();
        state.hold_channel(channel);
        (channel_id, active)
    }

    fn send(text: &str, mentions: Vec<Mention>) -> MurmureCommand {
        MurmureCommand::SendMessage {
            draft: MessageDraft {
                text: text.to_owned(),
                mentions,
                styles: Vec::new(),
            },
            attachments: Vec::new(),
            now: NOW,
            confirmed: ambre_murmure::message::Confirmation::Unconfirmed,
        }
    }

    #[test]
    fn a_channel_command_reaches_its_engine_and_leaves_as_a_delta_and_an_event() {
        let (mut state, ui, sync) = harness();
        let alice = Id::<User>::mint();
        signed_in(&mut state, alice);
        let _ = std::iter::from_fn(|| ui.try_recv().ok()).count();
        let (channel_id, active) = open_channel(&mut state);
        let bob = Id::<User>::mint();

        state.handle_logic_command(LogicCommand::Channel {
            channel_id,
            command: send(
                "hi @bob",
                vec![Mention {
                    target: MentionTarget::User(bob),
                    range: Span::<Char>::from_usize_range(3..7),
                }],
            ),
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
        assert_eq!(*document_id, active.document_id());
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

    /// A message sent before its segment's identity lands waits. It leaves
    /// once the segment went up whole, after that first update, and both are
    /// written under the session's PeerID alone (`AUTHOR-026`, `AUTHOR-029`).
    #[test]
    fn a_message_before_the_identity_waits_then_leaves_under_it() {
        let (mut state, ui, sync) = harness();
        signed_in(&mut state, Id::<User>::mint());
        let _ = drain(&ui);
        let channel_id = Id::<Channel>::mint();
        let channel =
            ChannelState::create(channel_id, ChannelType::Public, None, NOW).expect("create");
        let active = channel.active_id().document_id();
        state.route(|m| m.hold_created(channel));
        let _ = drain(&sync);

        state.handle_logic_command(LogicCommand::Channel {
            channel_id,
            command: send("early", Vec::new()),
        });
        assert!(
            drain(&sync).is_empty(),
            "nothing leaves before the identity"
        );
        assert!(drain(&ui).is_empty(), "nothing is written either");

        let peer = state.grant_identity(active);
        assert!(
            drain(&sync).is_empty(),
            "nothing leaves before the Sync worker holds the document"
        );
        state.handle_logic_event(LogicEvent::DocumentReady {
            document_id: active,
            role: ambre_bus::Role::Owner,
        });
        let sent = drain(&sync);
        let [
            SyncCommand::SendUpdate {
                document_id: first_to,
                payload: first,
                summary: None,
            },
            SyncCommand::SendUpdate {
                document_id: message_to,
                payload: message,
                summary: Some(_),
            },
        ] = sent.as_slice()
        else {
            panic!("the whole segment, then the message, got {sent:?}");
        };
        assert_eq!((*first_to, *message_to), (active, active));
        assert!(crate::sent_by(first.as_bytes(), peer));
        assert!(crate::sent_by(message.as_bytes(), peer));
        assert!(drain(&ui).iter().any(|e| matches!(
            e,
            UiEvent::Channel { event: MurmureEvent::Messages { messages, .. }, .. }
                if messages.len() == 1 && messages[0].text == "early"
        )));
    }

    #[test]
    fn a_remote_delta_reaches_the_channel_that_holds_its_document() {
        let (mut alice, _, alice_sync) = harness();
        let author = Id::<User>::mint();
        signed_in(&mut alice, author);
        let channel_id = Id::<Channel>::mint();
        alice.hold_channel(
            ChannelState::create(channel_id, ChannelType::Public, None, NOW).expect("create"),
        );
        // Bob copies the documents as they leave alice: under her PeerIDs.
        let original = &alice.rt().channels()[&channel_id];
        let active = original.active_id();
        let segments = std::collections::HashMap::from([(
            active,
            original.snapshot(active.document_id()).expect("active"),
        )]);
        let meta = original
            .snapshot(original.meta_id().document_id())
            .expect("meta");
        let copy =
            ChannelState::open(channel_id, original.meta_id(), &meta, &segments).expect("open");
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
        let peer = alice.rt().channels()[&channel_id]
            .peer(document_id)
            .expect("a session");
        bob.handle_logic_event(LogicEvent::RemoteUpdate {
            document_id,
            delta: ambre_bus::SignedDelta {
                seq: ambre_core::Seq::<ambre_core::Relay>::new(1),
                payload,
                nonce: FixedBytes::from_array([0; 16]),
                peer,
                user: Some(author),
                summary: None,
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

    /// An open channel reads the news from the summary of each accepted
    /// delta (the Murmure notifications spec). A message of another member
    /// that names the user is a mention. The user's own message, from another device, is
    /// none, and neither is a delta with no summary. The news names the
    /// segment of its message, so a click opens it there.
    #[test]
    fn a_message_of_another_member_is_news_and_names_a_mention() {
        let (mut alice, _, alice_sync) = harness();
        let author = Id::<User>::mint();
        signed_in(&mut alice, author);
        let channel_id = Id::<Channel>::mint();
        alice.hold_channel(
            ChannelState::create(channel_id, ChannelType::Public, None, NOW).expect("create"),
        );
        let original = &alice.rt().channels()[&channel_id];
        let active = original.active_id();
        let segments = std::collections::HashMap::from([(
            active,
            original.snapshot(active.document_id()).expect("active"),
        )]);
        let meta = original
            .snapshot(original.meta_id().document_id())
            .expect("meta");
        let copy =
            ChannelState::open(channel_id, original.meta_id(), &meta, &segments).expect("open");
        let (mut bob, bob_ui, _) = harness();
        let bob_id = Id::<User>::mint();
        signed_in(&mut bob, bob_id);
        bob.hold_channel(copy);
        drain(&bob_ui);

        let mut deltas = Vec::new();
        for (seq, text, mention) in [(1, "plain", false), (2, "hi @bob", true)] {
            let mentions = if mention {
                vec![Mention {
                    target: MentionTarget::User(bob_id),
                    range: Span::<Char>::from_usize_range(3..7),
                }]
            } else {
                Vec::new()
            };
            alice.handle_logic_command(LogicCommand::Channel {
                channel_id,
                command: send(text, mentions),
            });
            let Ok(SyncCommand::SendUpdate {
                document_id,
                payload,
                summary,
            }) = alice_sync.try_recv()
            else {
                panic!("alice sends one update");
            };
            let peer = alice.rt().channels()[&channel_id]
                .peer(document_id)
                .expect("a session");
            deltas.push((
                document_id,
                ambre_bus::SignedDelta {
                    seq: ambre_core::Seq::<ambre_core::Relay>::new(seq),
                    payload,
                    nonce: FixedBytes::from_array([0; 16]),
                    peer,
                    user: Some(author),
                    summary,
                },
            ));
        }
        let news = |bob: &mut LogicState, (document_id, delta): (Id<Document>, SignedDelta)| {
            bob.handle_logic_event(LogicEvent::RemoteUpdate { document_id, delta });
            drain(&bob_ui)
                .into_iter()
                .filter_map(|event| match event {
                    UiEvent::NewMessage {
                        channel_id: c,
                        mention,
                        message,
                    } if c == channel_id => {
                        let segment = message.map(|(segment, _)| segment.document_id());
                        Some((mention, segment == Some(document_id)))
                    }
                    _ => None,
                })
                .collect::<Vec<_>>()
        };
        use ambre_bus::MentionState::{Mentioned, Unmentioned};
        let [plain, named] = <[_; 2]>::try_from(deltas).expect("two deltas");
        // The same message as bob's own, which the engine would refuse
        // under alice's peer: the filter is read on its own.
        let (doc, mut own) = plain.clone();
        own.user = Some(bob_id);
        assert_eq!(bob.route(|m| m.news_of(&own)), None);
        assert_eq!(bob.route(|m| m.news_of(&plain.1)), Some(Unmentioned));
        let (_, mut bare) = plain.clone();
        bare.summary = None;
        assert_eq!(
            news(&mut bob, (doc, bare)),
            Vec::<(ambre_bus::MentionState, bool)>::new()
        );
        assert_eq!(news(&mut bob, plain), [(Unmentioned, true)]);
        assert_eq!(news(&mut bob, named), [(Mentioned, true)]);
    }

    /// An open channel keeps its documents while its view shows it (the
    /// Murmure engine spec). Closing one channel releases its documents alone, as
    /// light subscriptions, and the other stays whole.
    #[test]
    fn closing_a_channel_releases_its_documents_and_no_others() {
        let (mut state, _, sync) = harness();
        let (channel_id, active) = open_channel(&mut state);
        let (shown, shown_active) = open_channel(&mut state);
        let released = state.rt().channels()[&channel_id].document_ids();
        drain(&sync);
        state.handle_logic_command(LogicCommand::CloseChannel { channel_id });
        assert!(!state.rt().channels().contains_key(&channel_id));
        let empty = ambre_core::Payload::<ambre_core::DeltaBytes>::default();
        assert!(!state.route(|m| m.apply_channel_remote(
            active.document_id(),
            &[(None, ambre_core::LoroPayloadRef::Delta(&empty))]
        )));
        let sent = sent(&sync);
        assert_eq!(
            sent,
            released
                .iter()
                .map(|id| Sent::Release(*id))
                .collect::<Vec<_>>()
        );
        assert!(state.rt().channels()[&shown].holds(shown_active.document_id()));
        assert!(!sent.contains(&Sent::Release(shown_active.document_id())));
    }

    /// A closed channel with a cursor on each held segment: its count at
    /// close was zero.
    fn closed_counted(state: &mut LogicState) -> (Id<Channel>, Id<Segment>, Id<Meta>) {
        signed_in(state, Id::<User>::mint());
        let (channel_id, active) = open_channel(state);
        let meta = state.rt().channels()[&channel_id].meta_id();
        state.rt_mut().read_cursors_mut().insert(
            active.document_id(),
            ambre_murmure::logic::runtime::ReadCursor {
                read: Seq::<Relay>::new(0),
                active: ambre_bus::Activity::Quiet,
            },
        );
        state.handle_logic_command(LogicCommand::CloseChannel { channel_id });
        (channel_id, active, meta)
    }

    fn activity(content: u32) -> ambre_core::activity::ActivityCounters {
        ambre_core::activity::ActivityCounters {
            active: ambre_bus::Activity::Active,
            content: ambre_core::Count::<ambre_core::activity::ContentUpdates>::new(content),
            ..Default::default()
        }
    }

    /// The summaries of a closed channel's documents give it a count, over
    /// the count it had at close (the Murmure unread spec). A document's own
    /// badge is not shown, and an update with no message leaves the count.
    #[test]
    fn a_closed_channel_counts_its_unread_from_the_summaries() {
        let (mut state, ui, _sync) = harness();
        let (channel_id, active, meta) = closed_counted(&mut state);
        drain(&ui);
        for (document_id, content) in [
            (active.document_id(), 2),
            (active.document_id(), 3),
            (meta.document_id(), 0),
        ] {
            state.handle_logic_event(LogicEvent::Activity {
                document_id,
                counters: activity(content),
            });
        }
        let mut news = 0;
        let marks: Vec<_> = drain(&ui)
            .into_iter()
            .filter_map(|event| match event {
                UiEvent::ChannelActivity { channels } => Some(channels),
                UiEvent::NewMessage {
                    channel_id: c,
                    mention: ambre_bus::MentionState::Unmentioned,
                    message: None,
                } if c == channel_id => {
                    news += 1;
                    None
                }
                other => panic!("a document badge leaked: {other:?}"),
            })
            .collect();
        let mark = |unread| {
            vec![ChannelActivity {
                channel_id,
                active: ambre_bus::Activity::Active,
                unread: Some(ambre_core::Count::<Unread>::new(unread)),
                mentions: Some(ambre_core::Count::<Mentions>::ZERO),
            }]
        };
        assert_eq!(marks, vec![mark(2), mark(3), mark(3)]);
        // A count that rose is news; the update with no message is not.
        assert_eq!(news, 2);
    }

    /// A closed channel counts its mentions from the summaries beside its
    /// unread, over the count it had at close (the Murmure mentions spec).
    #[test]
    fn a_closed_channel_counts_its_mentions_from_the_summaries() {
        let (mut state, ui, _sync) = harness();
        let (channel_id, active, _meta) = closed_counted(&mut state);
        drain(&ui);
        state.handle_logic_event(LogicEvent::Activity {
            document_id: active.document_id(),
            counters: ambre_core::activity::ActivityCounters {
                mentions: ambre_core::Count::<ambre_core::activity::MentionUpdates>::new(1),
                ..activity(2)
            },
        });
        let shown = drain(&ui);
        let [
            UiEvent::ChannelActivity { channels },
            UiEvent::NewMessage {
                channel_id: news,
                mention: ambre_bus::MentionState::Mentioned,
                message: None,
            },
        ] = shown.as_slice()
        else {
            panic!("expected the channel's mark, then a mention, got {shown:?}");
        };
        assert_eq!(*news, channel_id);
        assert_eq!(
            channels,
            &[ChannelActivity {
                channel_id,
                active: ambre_bus::Activity::Active,
                unread: Some(ambre_core::Count::<Unread>::new(2)),
                mentions: Some(ambre_core::Count::<Mentions>::new(1)),
            }]
        );
    }

    /// A channel whose count was unknown at close keeps its mark until a
    /// summary counts a message, then is marked with no count.
    #[test]
    fn a_closed_channel_with_no_count_is_marked_by_a_counted_message() {
        let (mut state, ui, _sync) = harness();
        signed_in(&mut state, Id::<User>::mint());
        let (channel_id, active) = open_channel(&mut state);
        state.handle_logic_command(LogicCommand::CloseChannel { channel_id });
        drain(&ui);
        state.handle_logic_event(LogicEvent::Activity {
            document_id: active.document_id(),
            counters: activity(0),
        });
        assert!(
            drain(&ui).is_empty(),
            "an update with no message moved the mark"
        );
        state.handle_logic_event(LogicEvent::Activity {
            document_id: active.document_id(),
            counters: activity(1),
        });
        let shown = drain(&ui);
        let [
            UiEvent::ChannelActivity { channels },
            UiEvent::NewMessage {
                mention: ambre_bus::MentionState::Unmentioned,
                ..
            },
        ] = shown.as_slice()
        else {
            panic!("expected the channel's mark, then the news, got {shown:?}");
        };
        assert_eq!(
            channels,
            &[ChannelActivity {
                channel_id,
                active: ambre_bus::Activity::Active,
                unread: None,
                mentions: None,
            }]
        );
    }

    /// Holds `RESID-013`: a reopen asks the relay for the channel and moves
    /// no read cursor. Until it opens, its summaries still count it.
    #[test]
    fn reopening_a_channel_leaves_its_read_cursor_where_it_was() {
        let (mut state, ui, sync) = harness();
        let (channel_id, active, _) = closed_counted(&mut state);
        let cursors = state.rt().read_cursors().clone();
        drain(&sync);
        state.handle_logic_command(LogicCommand::OpenChannel { channel_id });
        assert_eq!(sent(&sync), [Sent::List(channel_id)]);
        assert_eq!(*state.rt().read_cursors(), cursors);
        drain(&ui);
        state.handle_logic_event(LogicEvent::Activity {
            document_id: active.document_id(),
            counters: activity(1),
        });
        assert!(
            matches!(
                drain(&ui).as_slice(),
                [UiEvent::ChannelActivity { .. }, UiEvent::NewMessage { .. }]
            ),
            "a reopening channel stopped counting from its summaries"
        );
    }

    /// A closed channel reopens from the copy of its documents the local
    /// store holds (the Murmure engine spec). Each copy reaches the store at close,
    /// and the light subscription keeps the horizon it covers. The reopen
    /// reads the copy back before the Sync worker opens the document, so
    /// the relay replays only what follows. A document with no copy is
    /// fetched whole.
    #[test]
    fn a_closed_channel_reopens_from_its_local_copy() {
        let (mut state, ui, sync, store) = harness_with_store();
        let (channel_id, stored) = stored_channel();
        let meta = stored.meta_id().document_id();
        let active = stored.active_id().document_id();
        let listed = || LogicEvent::ChannelDocuments {
            channel_id,
            documents: listing(&stored),
        };
        state.handle_logic_command(LogicCommand::OpenChannel { channel_id });
        state.handle_logic_event(listed());
        serve(&mut state, &stored, meta);
        serve(&mut state, &stored, active);
        drain(&sync);
        drain(&store);

        state.handle_logic_command(LogicCommand::CloseChannel { channel_id });
        let copies: HashMap<Id<Document>, ambre_core::Payload<ambre_core::SnapshotBytes>> =
            drain(&store)
                .into_iter()
                .filter_map(|(id, write)| match write {
                    PersistWrite::Snapshot(copy) => Some((id, copy)),
                    _ => None,
                })
                .collect();
        assert_eq!(copies.len(), 2, "each document reaches the store");
        let horizons: HashMap<Id<Document>, Seq<Relay>> = drain(&sync)
            .into_iter()
            .filter_map(|command| match command {
                SyncCommand::ReleaseDocument {
                    document_id,
                    applied_seq,
                } => Some((document_id, applied_seq)),
                _ => None,
            })
            .collect();
        assert_eq!(
            horizons,
            HashMap::from([
                (meta, ambre_core::Seq::<ambre_core::Relay>::new(1)),
                (active, ambre_core::Seq::<ambre_core::Relay>::new(1))
            ]),
            "each light subscription keeps its horizon"
        );

        state.handle_logic_command(LogicCommand::OpenChannel { channel_id });
        state.handle_logic_event(listed());
        assert_eq!(
            sent(&sync),
            [Sent::List(channel_id)],
            "meta waits on its copy"
        );
        let read_back = |rx: &Receiver<(Id<Document>, PersistWrite)>, id: Id<Document>| {
            matches!(
                drain(rx).as_slice(),
                [(asked, PersistWrite::ReadBack { doc_type: DocType::Murmure })] if *asked == id
            )
        };
        assert!(read_back(&store, meta));
        state.handle_logic_command(LogicCommand::RestoreLocalSnapshot {
            document_id: meta,
            doc_type: DocType::Murmure,
            snapshot: Some(copies[&meta].clone()),
            deltas: Vec::new(),
            sessions: Vec::new(),
        });
        assert_eq!(sent(&sync), [Sent::Open(meta)]);
        // The relay holds nothing past the horizon: only its ready comes.
        state.handle_logic_event(LogicEvent::DocumentReady {
            document_id: meta,
            role: ambre_bus::Role::Editor,
        });
        assert!(read_back(&store, active));
        state.handle_logic_command(LogicCommand::RestoreLocalSnapshot {
            document_id: active,
            doc_type: DocType::Murmure,
            snapshot: None,
            deltas: Vec::new(),
            sessions: Vec::new(),
        });
        assert_eq!(
            sent(&sync),
            [Sent::Close(active), Sent::Open(active)],
            "a document with no copy is fetched whole"
        );
        drain(&ui);
        serve(&mut state, &stored, active);
        assert!(drain(&ui).iter().any(|event| matches!(
            event,
            UiEvent::Channel { event: MurmureEvent::Messages { messages, .. }, .. }
                if messages.first().is_some_and(|m| m.text == "stored")
        )));
        assert!(state.rt().channels().contains_key(&channel_id));
        assert!(!state.rt().closed_channels().any(|c| c == channel_id));
    }

    /// The relay removing the user from a closed channel ends its light
    /// subscriptions and forgets its read cursors.
    #[test]
    fn a_closed_channel_the_user_left_ends_its_light_subscriptions() {
        let (mut state, _ui, sync) = harness();
        let (channel_id, active, meta) = closed_counted(&mut state);
        drain(&sync);
        state.route(|m| m.channel_revoked(meta.document_id()));
        assert!(!state.rt().closed_channels().any(|c| c == channel_id));
        assert!(
            !state
                .rt()
                .read_cursors()
                .contains_key(&active.document_id())
        );
        assert!(sent(&sync).contains(&Sent::Close(active.document_id())));
    }

    // --- The staged open over the relay ---

    fn drain<T>(rx: &Receiver<T>) -> Vec<T> {
        std::iter::from_fn(|| rx.try_recv().ok()).collect()
    }

    /// A channel as the relay stores it: one message in its active segment.
    fn stored_channel() -> (Id<Channel>, ChannelState) {
        let channel_id = Id::<Channel>::mint();
        let mut channel = crate::identified(
            ChannelState::create(channel_id, ChannelType::Public, None, NOW).expect("create"),
        );
        channel
            .send(
                &MessageDraft {
                    text: "stored".to_owned(),
                    mentions: Vec::new(),
                    styles: Vec::new(),
                },
                Id::<User>::mint(),
                NOW,
            )
            .expect("send");
        (channel_id, channel)
    }

    fn listing(channel: &ChannelState) -> Vec<ChannelDocument> {
        vec![
            ChannelDocument {
                document_id: channel.meta_id().document_id(),
                kind: ChannelDocumentKind::Meta,
            },
            ChannelDocument {
                document_id: channel.active_id().document_id(),
                kind: ChannelDocumentKind::Segment,
            },
        ]
    }

    /// The relay serves a document: its snapshot, then its ready signal.
    fn serve(state: &mut LogicState, stored: &ChannelState, document_id: Id<Document>) {
        state.handle_logic_event(LogicEvent::RemoteSnapshot {
            document_id,
            covered_seq: ambre_core::Seq::<ambre_core::Relay>::new(1),
            payload: (stored.snapshot(document_id).expect("snapshot")),
        });
        state.handle_logic_event(LogicEvent::DocumentReady {
            document_id,
            role: ambre_bus::Role::Editor,
        });
    }

    /// What Logic asked the Sync worker, reduced to the calls these tests
    /// follow: `SyncCommand` carries secrets, so it has no `PartialEq`.
    #[derive(Debug, PartialEq)]
    enum Sent {
        List(Id<Channel>),
        Open(Id<Document>),
        Close(Id<Document>),
        Release(Id<Document>),
        Other,
    }

    fn sent(rx: &Receiver<SyncCommand>) -> Vec<Sent> {
        drain(rx)
            .into_iter()
            .map(|c| match c {
                SyncCommand::OpenChannel { channel_id, .. } => Sent::List(channel_id),
                // A document of a channel opens naming its `meta`
                // (`SYNC-229`).
                SyncCommand::OpenDocument {
                    document_id,
                    doc_type: DocType::Murmure,
                    meta: Some(_),
                } => Sent::Open(document_id),
                SyncCommand::CloseDocument { document_id } => Sent::Close(document_id),
                SyncCommand::ReleaseDocument { document_id, .. } => Sent::Release(document_id),
                _ => Sent::Other,
            })
            .collect()
    }

    #[test]
    fn a_channel_opens_its_meta_then_its_live_segments_then_shows_them() {
        let (mut state, ui, sync) = harness();
        let (channel_id, stored) = stored_channel();

        state.handle_logic_command(LogicCommand::OpenChannel { channel_id });
        assert_eq!(sent(&sync), [Sent::List(channel_id)]);

        state.handle_logic_event(LogicEvent::ChannelDocuments {
            channel_id,
            documents: listing(&stored),
        });
        assert_eq!(
            sent(&sync),
            [Sent::Open(stored.meta_id().document_id())],
            "meta first"
        );

        serve(&mut state, &stored, stored.meta_id().document_id());
        assert_eq!(sent(&sync), [Sent::Open(stored.active_id().document_id())]);
        assert!(drain(&ui).is_empty(), "nothing of the channel shows yet");

        serve(&mut state, &stored, stored.active_id().document_id());
        let shown = drain(&ui);
        let [
            UiEvent::Channel {
                event: MurmureEvent::Named { name },
                ..
            },
            UiEvent::Channel {
                channel_id: shown_channel,
                event: MurmureEvent::Messages { segment, messages },
            },
        ] = shown.as_slice()
        else {
            panic!("the channel's name, then its active segment's messages, got {shown:?}");
        };
        assert_eq!(*name, stored.name());
        assert_eq!(*shown_channel, channel_id);
        assert_eq!(*segment, stored.active_id());
        assert_eq!(messages[0].text, "stored");
        assert!(state.rt().channels().contains_key(&channel_id));
        assert!(state.rt().opening().is_empty());
    }

    /// Holds `SYNC-229` on the Logic side. The meta opens naming itself, and
    /// each live segment it names opens naming that meta, read from the
    /// sealed state of the channel.
    #[test]
    fn each_document_of_an_opening_channel_names_its_meta() {
        let (mut state, _ui, sync) = harness();
        let (channel_id, stored) = stored_channel();
        let meta = stored.meta_id();
        let opened = |sync: &Receiver<SyncCommand>| -> Vec<(Id<Document>, Option<Id<Meta>>)> {
            drain(sync)
                .into_iter()
                .filter_map(|c| match c {
                    SyncCommand::OpenDocument {
                        document_id, meta, ..
                    } => Some((document_id, meta)),
                    _ => None,
                })
                .collect()
        };
        state.handle_logic_command(LogicCommand::OpenChannel { channel_id });
        drain(&sync);
        state.handle_logic_event(LogicEvent::ChannelDocuments {
            channel_id,
            documents: listing(&stored),
        });
        assert_eq!(opened(&sync), [(meta.document_id(), Some(meta))]);
        serve(&mut state, &stored, meta.document_id());
        assert_eq!(
            opened(&sync),
            [(stored.active_id().document_id(), Some(meta))]
        );
    }

    /// A channel no catalogue of this client lists admits invites alone, so
    /// its member list counts no self-join.
    #[test]
    fn a_channel_no_catalogue_lists_admits_no_self_join() {
        let (mut state, _, sync) = harness();
        let channel_id = Id::<Channel>::mint();
        state.handle_logic_command(LogicCommand::OpenChannel { channel_id });
        assert!(matches!(
            drain(&sync).as_slice(),
            [SyncCommand::OpenChannel {
                listed_in: None,
                ..
            }]
        ));
    }

    #[test]
    fn a_refused_listing_tells_the_view_and_holds_nothing() {
        let (mut state, ui, _) = harness();
        let channel_id = Id::<Channel>::mint();
        state.handle_logic_command(LogicCommand::OpenChannel { channel_id });
        state.handle_logic_event(LogicEvent::ChannelOpenFailed {
            channel_id,
            failure: ambre_bus::ApiFailure::NoAnswer,
        });
        assert!(matches!(
            drain(&ui).as_slice(),
            [UiEvent::Channel {
                event: MurmureEvent::OpenFailed,
                ..
            }]
        ));
        assert!(state.rt().opening().is_empty());
    }

    /// A listing refused for want of membership drops the channel: the view
    /// learns it is gone, not that it failed. A second refusal, or one for a
    /// channel never opened, tells nothing.
    #[test]
    fn a_channel_the_relay_refuses_to_open_is_dropped() {
        let (mut state, ui, sync) = harness();
        let channel_id = Id::<Channel>::mint();
        state.handle_logic_command(LogicCommand::OpenChannel { channel_id });
        let _ = drain(&sync);
        state.handle_logic_event(LogicEvent::ChannelRefused { channel_id });
        assert!(matches!(
            drain(&ui).as_slice(),
            [UiEvent::Channel {
                channel_id: id,
                event: MurmureEvent::Left,
            }] if *id == channel_id
        ));
        assert!(state.rt().opening().is_empty());
        assert!(drain(&sync).is_empty(), "no session was open to end");

        state.handle_logic_event(LogicEvent::ChannelRefused { channel_id });
        state.handle_logic_event(LogicEvent::ChannelRefused {
            channel_id: Id::<Channel>::mint(),
        });
        assert!(drain(&ui).is_empty());
    }

    #[test]
    fn a_document_that_fails_to_open_ends_the_opening() {
        let (mut state, ui, sync) = harness();
        let (channel_id, stored) = stored_channel();
        state.handle_logic_command(LogicCommand::OpenChannel { channel_id });
        state.handle_logic_event(LogicEvent::ChannelDocuments {
            channel_id,
            documents: listing(&stored),
        });
        let _ = drain(&sync);

        state.handle_logic_event(LogicEvent::DocumentOpenFailed {
            document_id: stored.meta_id().document_id(),
            cause: OpenFailure::KeyNotReceived,
        });
        assert_eq!(sent(&sync), [Sent::Close(stored.meta_id().document_id())]);
        assert!(
            matches!(
                drain(&ui).as_slice(),
                [UiEvent::Channel {
                    event: MurmureEvent::OpenFailed,
                    ..
                }]
            ),
            "the channel fails, and no bare auth error"
        );
        assert!(state.rt().opening().is_empty());
    }

    #[test]
    fn a_meta_of_another_channel_ends_the_opening() {
        let (mut state, ui, sync) = harness();
        let (_, stored) = stored_channel();
        let channel_id = Id::<Channel>::mint();
        state.handle_logic_command(LogicCommand::OpenChannel { channel_id });
        state.handle_logic_event(LogicEvent::ChannelDocuments {
            channel_id,
            documents: listing(&stored),
        });
        let _ = drain(&sync);
        serve(&mut state, &stored, stored.meta_id().document_id());
        assert_eq!(sent(&sync), [Sent::Close(stored.meta_id().document_id())]);
        assert!(matches!(
            drain(&ui).as_slice(),
            [UiEvent::Channel {
                event: MurmureEvent::OpenFailed,
                ..
            }]
        ));
        assert!(state.rt().channels().is_empty());
    }

    #[test]
    fn closing_a_channel_still_opening_ends_its_documents() {
        let (mut state, _, sync) = harness();
        let (channel_id, stored) = stored_channel();
        state.handle_logic_command(LogicCommand::OpenChannel { channel_id });
        state.handle_logic_event(LogicEvent::ChannelDocuments {
            channel_id,
            documents: listing(&stored),
        });
        let _ = drain(&sync);
        state.handle_logic_command(LogicCommand::CloseChannel { channel_id });
        assert_eq!(sent(&sync), [Sent::Close(stored.meta_id().document_id())]);
        // A listing that lands after the close is stale.
        state.handle_logic_event(LogicEvent::ChannelDocuments {
            channel_id,
            documents: listing(&stored),
        });
        assert!(drain(&sync).is_empty());
    }

    #[test]
    fn asking_for_the_segment_before_the_oldest_asks_nothing() {
        let (mut state, _, sync) = harness();
        let (channel_id, active) = open_channel(&mut state);
        state.handle_logic_command(LogicCommand::Channel {
            channel_id,
            command: MurmureCommand::LoadOlder { segment: active },
        });
        assert!(drain(&sync).is_empty());
    }

    #[test]
    fn marking_a_channel_read_moves_its_live_segments_and_clears_its_mark() {
        let (mut state, ui, sync) = harness();
        signed_in(&mut state, Id::<User>::mint());
        let _ = drain(&ui);
        let (channel_id, active) = open_channel(&mut state);

        state.handle_logic_command(LogicCommand::Channel {
            channel_id,
            command: MurmureCommand::MarkRead,
        });

        let sent = drain(&sync);
        let [SyncCommand::MarkRead { live, finals }] = sent.as_slice() else {
            panic!("one mark read, got {sent:?}");
        };
        assert_eq!(*live, [active]);
        assert!(finals.is_empty());
        let shown: Vec<UiEvent> = std::iter::from_fn(|| ui.try_recv().ok()).collect();
        let [
            UiEvent::ChannelActivity { channels },
            UiEvent::Channel {
                event: MurmureEvent::FirstUnread { message: None },
                ..
            },
        ] = shown.as_slice()
        else {
            panic!("one activity event, then no first unread, got {shown:?}");
        };
        assert_eq!(
            *channels,
            [ChannelActivity {
                channel_id,
                active: ambre_bus::Activity::Quiet,
                unread: Some(ambre_core::Count::<Unread>::ZERO),
                mentions: Some(ambre_core::Count::<Mentions>::ZERO),
            }]
        );
    }

    #[test]
    fn an_activity_answer_marks_each_channel_once_and_opens_nothing() {
        let (mut state, ui, sync) = harness();
        let (quiet, busy) = (Id::<Channel>::mint(), Id::<Channel>::mint());
        let entry = |channel_id, active| DocumentActivity {
            channel_id,
            document_id: Id::<Document>::mint(),
            kind: ChannelDocumentKind::Segment,
            active,
            read: Seq::<Relay>::ZERO,
            last: None,
        };

        state.handle_logic_event(LogicEvent::ChannelActivity {
            documents: vec![
                entry(busy, ambre_bus::Activity::Quiet),
                entry(quiet, ambre_bus::Activity::Quiet),
                entry(busy, ambre_bus::Activity::Active),
                entry(quiet, ambre_bus::Activity::Quiet),
            ],
        });

        let shown: Vec<UiEvent> = std::iter::from_fn(|| ui.try_recv().ok()).collect();
        let [UiEvent::ChannelActivity { channels }] = shown.as_slice() else {
            panic!("one activity event, got {shown:?}");
        };
        assert_eq!(
            *channels,
            [
                ChannelActivity {
                    channel_id: busy,
                    active: ambre_bus::Activity::Active,
                    unread: None,
                    mentions: None,
                },
                ChannelActivity {
                    channel_id: quiet,
                    active: ambre_bus::Activity::Quiet,
                    unread: None,
                    mentions: None,
                },
            ]
        );
        assert!(
            drain(&sync).is_empty(),
            "an activity answer opens no document"
        );
    }

    /// The view's ask for the user's channels reaches Sync, and the answer
    /// reaches the view whole. It opens no channel and asks for no document.
    #[test]
    fn the_users_channels_reach_the_view_and_open_nothing() {
        let (mut state, ui, sync) = harness();
        state.handle_logic_command(LogicCommand::ListMemberChannels);
        let sent = drain(&sync);
        assert!(
            matches!(sent.as_slice(), [SyncCommand::ListMemberChannels]),
            "got {sent:?}"
        );

        let channels = vec![
            ambre_bus::MemberChannel {
                channel_id: Id::<Channel>::mint(),
                place: ambre_bus::ChannelPlace::Private(Id::<Space>::mint()),
            },
            ambre_bus::MemberChannel {
                channel_id: Id::<Channel>::mint(),
                place: ambre_bus::ChannelPlace::Direct,
            },
        ];
        let spaces = vec![Id::<Space>::mint()];
        let names = vec![ambre_bus::ChannelName {
            channel_id: channels[0].channel_id,
            name: ambre_core::Validated::<ambre_core::Name>::parse("board").unwrap(),
        }];
        let peers = vec![ambre_bus::DirectPeers {
            channel_id: channels[1].channel_id,
            names: vec![ambre_core::Validated::<ambre_core::DisplayName>::parse("Bob").unwrap()],
        }];
        state.handle_logic_event(LogicEvent::MemberChannels {
            channels: channels.clone(),
            spaces: spaces.clone(),
            names: names.clone(),
            peers: peers.clone(),
            metas: vec![ambre_bus::ChannelMeta {
                channel_id: channels[0].channel_id,
                meta: Id::<Meta>::mint(),
                last: None,
            }],
        });
        let shown: Vec<UiEvent> = std::iter::from_fn(|| ui.try_recv().ok()).collect();
        assert!(
            matches!(
                shown.as_slice(),
                [UiEvent::MemberChannels { channels: c, spaces: s, names: n, peers: p }]
                    if *c == channels && *s == spaces && *n == names && *p == peers
            ),
            "got {shown:?}"
        );
        assert!(
            drain(&sync).is_empty(),
            "learning of a channel or a space opens nothing"
        );
        assert!(state.rt().channels().is_empty(), "no channel state is held");
        assert!(
            state.rt().spaces().next().is_none(),
            "no space state is held"
        );
    }

    #[test]
    fn the_activity_commands_reach_the_sync_worker() {
        let (mut state, _, sync) = harness();
        state.handle_logic_command(LogicCommand::FetchActivity);
        state.handle_logic_command(LogicCommand::MarkAllRead);
        let sent = drain(&sync);
        assert!(
            matches!(
                sent.as_slice(),
                [SyncCommand::FetchActivity, SyncCommand::MarkAllRead]
            ),
            "got {sent:?}"
        );
    }

    // --- Unread counts and mark unread ---

    /// Alice writes on the engine; Bob is a signed-in client holding a copy
    /// of the same channel.
    struct Members {
        alice: ChannelState,
        /// The user Alice writes as, and signs as.
        author: Id<User>,
        bob: LogicState,
        bob_ui: Receiver<UiEvent>,
        bob_sync: Receiver<SyncCommand>,
        channel_id: Id<Channel>,
        active: Id<Segment>,
    }

    fn members() -> Members {
        let channel_id = Id::<Channel>::mint();
        let alice = crate::identified(
            ChannelState::create(channel_id, ChannelType::Public, None, NOW).expect("create"),
        );
        let active = alice.active_id();
        let segments = std::collections::HashMap::from([(
            active,
            alice.snapshot(active.document_id()).expect("active"),
        )]);
        let meta = alice.snapshot(alice.meta_id().document_id()).expect("meta");
        let copy = ChannelState::open(channel_id, alice.meta_id(), &meta, &segments).expect("open");
        let (mut bob, bob_ui, bob_sync) = harness();
        signed_in(&mut bob, Id::<User>::mint());
        bob.hold_channel(copy);
        let _ = drain(&bob_ui);
        Members {
            alice,
            author: Id::<User>::mint(),
            bob,
            bob_ui,
            bob_sync,
            channel_id,
            active,
        }
    }

    impl Members {
        /// Alice sends `text`; the relay delivers it to Bob as update `seq`.
        fn deliver(&mut self, text: &str, seq: u64) {
            let sent = self
                .alice
                .send(
                    &MessageDraft {
                        text: text.to_owned(),
                        mentions: Vec::new(),
                        styles: Vec::new(),
                    },
                    self.author,
                    NOW,
                )
                .expect("send");
            let document_id = sent.segment.document_id();
            self.bob.handle_logic_event(LogicEvent::RemoteUpdate {
                document_id,
                delta: ambre_bus::SignedDelta {
                    peer: self.alice.peer(document_id).expect("a session"),
                    user: Some(self.author),
                    ..crate::remote_delta(seq, sent.delta)
                },
            });
        }

        /// The relay's activity answer: Bob's cursor on the active segment.
        fn answer(&mut self, read: u64) {
            self.bob.handle_logic_event(LogicEvent::ChannelActivity {
                documents: vec![DocumentActivity {
                    channel_id: self.channel_id,
                    document_id: self.active.document_id(),
                    kind: ChannelDocumentKind::Segment,
                    active: ambre_bus::Activity::Active,
                    read: Seq::<Relay>::new(read),
                    last: None,
                }],
            });
        }

        /// Every count the view was handed for the channel since the last
        /// call.
        fn counts(&self) -> Vec<Option<u32>> {
            drain(&self.bob_ui)
                .into_iter()
                .filter_map(|e| match e {
                    UiEvent::ChannelActivity { channels } => Some(channels),
                    _ => None,
                })
                .flatten()
                .filter(|c| c.channel_id == self.channel_id)
                .map(|c| c.unread.map(|count| count.get()))
                .collect()
        }

        /// Every first unread message the view was handed for the channel
        /// since the last call.
        fn firsts(&self) -> Vec<Option<Id<Message>>> {
            drain(&self.bob_ui)
                .into_iter()
                .filter_map(|e| match e {
                    UiEvent::Channel {
                        channel_id,
                        event: MurmureEvent::FirstUnread { message },
                    } if channel_id == self.channel_id => Some(message),
                    _ => None,
                })
                .collect()
        }

        fn message_ids(&self) -> Vec<Id<Message>> {
            self.bob.rt().channels()[&self.channel_id]
                .messages(self.active)
                .expect("messages")
                .iter()
                .map(|m| m.id)
                .collect()
        }

        fn mark_unread(&mut self, message: Id<Message>) {
            self.bob.handle_logic_command(LogicCommand::Channel {
                channel_id: self.channel_id,
                command: MurmureCommand::MarkUnread {
                    segment: self.active,
                    message,
                },
            });
        }

        /// The cursors Bob asked the relay to move since the last call.
        fn moved(&self) -> Vec<Vec<(Id<Segment>, Seq<Relay>)>> {
            drain(&self.bob_sync)
                .into_iter()
                .filter_map(|c| match c {
                    SyncCommand::MoveReadCursors { cursors } => Some(cursors),
                    _ => None,
                })
                .collect()
        }
    }

    /// Each delivered message carries the relay's number, and the count
    /// reads it against the cursor.
    #[test]
    fn a_delivered_message_carries_the_relay_number_its_count_reads() {
        let mut m = members();
        m.answer(0);
        m.deliver("one", 5);
        m.deliver("two", 6);
        let ids = m.message_ids();
        let channel = &m.bob.rt().channels()[&m.channel_id];
        assert_eq!(
            ids.iter()
                .map(|id| channel.position(m.active, *id))
                .collect::<Vec<_>>(),
            [
                Some(Position::Exact(Seq::<Relay>::new(5))),
                Some(Position::Exact(Seq::<Relay>::new(6)))
            ]
        );
    }

    /// An open channel's count comes from its messages and the cursor, and
    /// none shows while the cursor is unknown.
    #[test]
    fn an_open_channel_counts_the_messages_past_its_cursor() {
        let mut m = members();
        m.deliver("before any answer", 1);
        assert_eq!(m.counts(), [], "no cursor yet, so no count");

        m.answer(0);
        assert_eq!(m.counts(), [Some(1)]);
        m.deliver("two", 2);
        assert_eq!(m.counts(), [Some(2)]);
        m.answer(1);
        assert_eq!(m.counts(), [Some(1)]);
    }

    /// The view is handed the first unread message each time the count is
    /// known anew, and none once the channel is read.
    #[test]
    fn the_view_is_handed_the_first_unread_message_with_each_count() {
        let mut m = members();
        m.deliver("before any answer", 1);
        assert_eq!(m.firsts(), [], "no cursor yet, so no first unread");

        m.answer(0);
        let ids = m.message_ids();
        assert_eq!(m.firsts(), [ids.first().copied()]);
        m.deliver("two", 2);
        assert_eq!(m.firsts(), [ids.first().copied()], "the first one stays");
        m.bob.handle_logic_command(LogicCommand::Channel {
            channel_id: m.channel_id,
            command: MurmureCommand::MarkRead,
        });
        assert_eq!(m.firsts(), [None]);
    }

    /// Marking a message unread moves the cursor to just before it, so it
    /// and every later message count again.
    #[test]
    fn marking_a_message_unread_moves_the_cursor_to_just_before_it() {
        let mut m = members();
        m.deliver("one", 5);
        m.deliver("two", 6);
        m.answer(6);
        assert_eq!(m.counts(), [Some(0)]);
        let ids = m.message_ids();

        m.mark_unread(ids[1]);
        assert_eq!(m.moved(), [vec![(m.active, Seq::<Relay>::new(5))]]);
        assert_eq!(m.counts(), [Some(1)]);

        m.mark_unread(ids[0]);
        assert_eq!(m.moved(), [vec![(m.active, Seq::<Relay>::new(4))]]);
        assert_eq!(m.counts(), [Some(2)]);
    }

    /// A delta passes the engine's authorship check before it is imported,
    /// and a refused one changes nothing the view shows (the Murmure engine
    /// spec).
    #[test]
    fn a_delta_the_authorship_check_refuses_reaches_no_view() {
        let mut m = members();
        let forged = m
            .alice
            .send(
                &MessageDraft {
                    text: "as somebody else".to_owned(),
                    mentions: Vec::new(),
                    styles: Vec::new(),
                },
                Id::<User>::mint(),
                NOW,
            )
            .expect("send");
        let document_id = forged.segment.document_id();
        m.bob.handle_logic_event(LogicEvent::RemoteUpdate {
            document_id,
            delta: ambre_bus::SignedDelta {
                peer: m.alice.peer(document_id).expect("a session"),
                user: Some(m.author),
                ..crate::remote_delta(1, forged.delta)
            },
        });
        assert!(m.message_ids().is_empty());
        // Refused, and still delivered: the count reads past it.
        assert_eq!(
            m.bob.rt().channels()[&m.channel_id].delivered(document_id),
            Seq::<Relay>::new(1)
        );
        assert!(
            !drain(&m.bob_ui)
                .iter()
                .any(|e| matches!(e, UiEvent::Channel { .. } | UiEvent::UpdateRequired)),
            "nothing reaches the view"
        );
    }

    /// A delta from a newer build is refused, and the user is asked to
    /// update (the Murmure messages spec).
    #[test]
    fn a_delta_from_a_newer_build_asks_the_user_to_update() {
        let mut m = members();
        let sent = m
            .alice
            .send(
                &MessageDraft {
                    text: "from the future".to_owned(),
                    mentions: Vec::new(),
                    styles: Vec::new(),
                },
                m.author,
                NOW,
            )
            .expect("send");
        let mut update: serde_json::Value =
            serde_json::from_slice(sent.delta.as_bytes()).expect("json");
        update["schema_version"] =
            serde_json::Value::from(ambre_murmure::JSON_SCHEMA_VERSION.get() + 1);
        let document_id = sent.segment.document_id();
        m.bob.handle_logic_event(LogicEvent::RemoteUpdate {
            document_id,
            delta: ambre_bus::SignedDelta {
                peer: m.alice.peer(document_id).expect("a session"),
                user: Some(m.author),
                ..crate::remote_delta(
                    1,
                    ambre_core::Payload::<ambre_core::DeltaBytes>::new(
                        serde_json::to_vec(&update).expect("bytes"),
                    ),
                )
            },
        });
        assert!(m.message_ids().is_empty());
        assert!(
            drain(&m.bob_ui)
                .iter()
                .any(|e| matches!(e, UiEvent::UpdateRequired))
        );
    }

    /// A message the relay has not numbered exactly cannot place a cursor.
    #[test]
    fn a_message_with_no_exact_number_is_not_marked_unread() {
        let mut m = members();
        m.answer(0);
        m.mark_unread(Id::<Message>::mint());
        assert_eq!(m.moved(), Vec::<Vec<(Id<Segment>, Seq<Relay>)>>::new());
    }

    /// Marking a message unread moves no cursor of another channel.
    #[test]
    fn marking_a_message_unread_moves_no_cursor_of_another_channel() {
        let mut m = members();
        let (other, other_active) = open_channel(&mut m.bob);
        m.deliver("one", 5);
        let ids = m.message_ids();
        let other_documents = m.bob.rt().channels()[&other].document_ids();

        m.mark_unread(ids[0]);
        let moved = m.moved();
        assert_eq!(moved, [vec![(m.active, Seq::<Relay>::new(4))]]);
        assert!(
            moved
                .iter()
                .flatten()
                .all(|(segment, _)| !other_documents.contains(&segment.document_id())),
            "no document of {other} (active {other_active}) moves"
        );
    }

    /// A search answers from every open channel, each with the segments it
    /// read, and asks the Sync worker nothing (the Murmure search spec).
    #[test]
    fn a_search_covers_every_open_channel_and_leaves_nothing() {
        let (mut state, ui, sync) = harness();
        signed_in(&mut state, Id::<User>::mint());
        let first = open_channel(&mut state);
        let second = open_channel(&mut state);
        for ((channel_id, _), text) in [(first, "the budget"), (second, "lunch")] {
            state.handle_logic_command(LogicCommand::Channel {
                channel_id,
                command: send(text, Vec::new()),
            });
        }
        drain(&sync);
        drain(&ui);

        state.handle_logic_command(LogicCommand::SearchMessages {
            query: ambre_core::Validated::<ambre_core::SearchQuery>::parse("Budget".to_owned())
                .expect("a word"),
        });
        assert!(drain(&sync).is_empty(), "the query never leaves the client");
        let shown = drain(&ui);
        let [UiEvent::MessageSearch { query, channels }] = shown.as_slice() else {
            panic!("one answer, got {shown:?}");
        };
        assert_eq!(query.as_str(), "Budget");
        #[expect(
            clippy::type_complexity,
            reason = "each `Id<_>` names its kind, and no alias may hide it"
        )]
        let mut answered: Vec<(Id<Channel>, Vec<Id<Segment>>, u32, Vec<String>)> = channels
            .iter()
            .map(|c| {
                let texts = c.matches.iter().map(|m| m.message.text.clone()).collect();
                (c.channel_id, c.covered.clone(), c.listed.get(), texts)
            })
            .collect();
        answered.sort_by_key(|(id, ..)| *id != first.0);
        assert_eq!(
            answered,
            [
                (first.0, vec![first.1], 1, vec!["the budget".to_owned()]),
                (second.0, vec![second.1], 1, Vec::new()),
            ]
        );

        state.handle_logic_command(LogicCommand::SearchMessages {
            query: ambre_core::Validated::<ambre_core::SearchQuery>::parse(" ? ".to_owned())
                .expect("not blank"),
        });
        assert!(matches!(
            drain(&ui).as_slice(),
            [UiEvent::MessageSearch { channels, .. }] if channels.is_empty()
        ));
    }

    /// A reaction leaves for the relay with a summary of kind reaction, and
    /// its removal too. A second removal writes nothing, so nothing leaves.
    #[test]
    fn a_reaction_leaves_with_its_summary_and_a_removal_with_it() {
        let (mut state, ui, sync) = harness();
        let me = Id::<User>::mint();
        signed_in(&mut state, me);
        let (channel_id, segment) = open_channel(&mut state);
        state.handle_logic_command(LogicCommand::Channel {
            channel_id,
            command: send("lunch?", Vec::new()),
        });
        drain(&sync);
        drain(&ui);
        let message = state.rt().channels()[&channel_id]
            .messages(segment)
            .expect("held")[0]
            .id;
        let react = MurmureCommand::React {
            at: segment.into(),
            message,
            emoji: ambre_bus::Emoji::from("\u{1F44D}"),
            now: NOW,
        };
        let unreact = MurmureCommand::Unreact {
            at: segment.into(),
            message,
            emoji: ambre_bus::Emoji::from("\u{1F44D}"),
        };

        for (command, users) in [(react, vec![me]), (unreact.clone(), Vec::new())] {
            state.handle_logic_command(LogicCommand::Channel {
                channel_id,
                command,
            });
            let sent = drain(&sync);
            assert!(
                matches!(
                    sent.as_slice(),
                    [SyncCommand::SendUpdate { document_id, summary: Some(summary), .. }]
                        if *document_id == segment.document_id()
                            && summary.kind == ActivityKind::Reaction
                ),
                "one reaction delta, got {sent:?}"
            );
            let shown = drain(&ui);
            let [
                UiEvent::Channel {
                    event: MurmureEvent::Messages { messages, .. },
                    ..
                },
            ] = shown.as_slice()
            else {
                panic!("the messages, got {shown:?}");
            };
            let reacted: Vec<Id<User>> = messages[0]
                .reactions
                .iter()
                .flat_map(|r| r.users.clone())
                .collect();
            assert_eq!(reacted, users);
        }
        state.handle_logic_command(LogicCommand::Channel {
            channel_id,
            command: unreact,
        });
        assert!(drain(&sync).is_empty(), "no entry left, nothing leaves");
        assert!(drain(&ui).is_empty());
    }

    /// An edit leaves with a summary of kind edit and shows the new body,
    /// marked edited, with no mark for a user outside the channel. A delete
    /// leaves the same way and shows the message deleted. A second delete
    /// is refused, so nothing leaves.
    #[test]
    fn an_edit_and_a_delete_leave_as_edits_and_show_the_message() {
        let (mut state, ui, sync) = harness();
        let me = Id::<User>::mint();
        let bob = Id::<User>::mint();
        signed_in(&mut state, me);
        let (channel_id, segment) = open_channel(&mut state);
        let meta = state.rt().channels()[&channel_id].meta_id().document_id();
        assert!(
            state
                .route(|m| m.channel_members_changed(meta, vec![member(me), member(bob)]))
                .is_none()
        );
        state.handle_logic_command(LogicCommand::Channel {
            channel_id,
            command: send("lunch?", Vec::new()),
        });
        drain(&sync);
        drain(&ui);
        let message = state.rt().channels()[&channel_id]
            .messages(segment)
            .expect("held")[0]
            .id;
        let mention = |target, range| Mention { target, range };
        let edit = MurmureCommand::EditMessage {
            at: segment.into(),
            message,
            draft: MessageDraft {
                text: "lunch @bob @eve?".to_owned(),
                styles: Vec::new(),
                mentions: vec![
                    mention(
                        MentionTarget::User(bob),
                        Span::<Char>::from_usize_range(6..10),
                    ),
                    mention(
                        MentionTarget::User(Id::<User>::mint()),
                        Span::<Char>::from_usize_range(11..15),
                    ),
                ],
            },
            now: Instant::<Authored>::new(1),
        };
        let delete = MurmureCommand::DeleteMessage {
            at: segment.into(),
            message,
            now: Instant::<Authored>::new(2),
        };

        for command in [edit, delete.clone()] {
            state.handle_logic_command(LogicCommand::Channel {
                channel_id,
                command,
            });
            let sent = drain(&sync);
            assert!(
                matches!(
                    sent.as_slice(),
                    [SyncCommand::SendUpdate { document_id, summary: Some(summary), .. }]
                        if *document_id == segment.document_id()
                            && summary.kind == ActivityKind::Edit
                ),
                "one edit delta, got {sent:?}"
            );
        }
        let shown = drain(&ui);
        let [
            UiEvent::Channel {
                event:
                    MurmureEvent::Messages {
                        messages: edited, ..
                    },
                ..
            },
            UiEvent::Channel {
                event:
                    MurmureEvent::Messages {
                        messages: deleted, ..
                    },
                ..
            },
        ] = shown.as_slice()
        else {
            panic!("the messages twice, got {shown:?}");
        };
        assert_eq!(
            (edited[0].text.as_str(), edited[0].edited, edited[0].deleted),
            (
                "lunch @bob @eve?",
                ambre_murmure::message::Revision::Edited,
                ambre_murmure::message::Deletion::Live
            )
        );
        assert_eq!(
            edited[0].mentions,
            [mention(
                MentionTarget::User(bob),
                Span::<Char>::from_usize_range(6..10)
            )],
            "no mark for a user outside the channel"
        );
        assert_eq!(
            (deleted[0].text.as_str(), deleted[0].deleted),
            ("", ambre_murmure::message::Deletion::Deleted)
        );

        state.handle_logic_command(LogicCommand::Channel {
            channel_id,
            command: delete,
        });
        assert!(drain(&sync).is_empty(), "a deleted message takes no delete");
        assert!(drain(&ui).is_empty());
    }

    /// The erase a delete asks for, as `(request, document, file)`.
    fn erases(
        sent: &[SyncCommand],
    ) -> Vec<(
        Seq<Request>,
        Id<Document>,
        ambre_core::Validated<ambre_core::ContentHash>,
    )> {
        sent.iter()
            .filter_map(|command| match command {
                SyncCommand::Rest {
                    id,
                    cmd:
                        RestCommand::EraseMedia {
                            document_id,
                            content_hash,
                        },
                } => Some((*id, *document_id, content_hash.clone())),
                _ => None,
            })
            .collect()
    }

    /// A file the view uploads or fetches goes through the shared media
    /// flow, under the channel's meta and the view's request.
    #[test]
    fn a_file_uploads_and_downloads_under_the_channels_meta() {
        let (mut state, _ui, sync) = harness();
        signed_in(&mut state, Id::<User>::mint());
        let (channel_id, _) = open_channel(&mut state);
        let meta = state.rt().channels()[&channel_id].meta_id().document_id();
        let file = Validated::<ContentHash>::parse("cd".repeat(32)).expect("a content hash");
        let (up, down) = (Seq::nth(41), Seq::nth(42));
        drain(&sync);

        for asked in [
            MurmureCommand::UploadFile {
                id: up,
                data: vec![1, 2, 3].into(),
            },
            MurmureCommand::FetchFile {
                id: down,
                media_ref: file.clone(),
            },
        ] {
            command(&mut state, channel_id, asked);
        }
        let sent = drain(&sync);
        assert!(
            matches!(
                sent.as_slice(),
                [
                    SyncCommand::Rest {
                        id: a,
                        cmd: RestCommand::UploadMedia { document_id: d1, data },
                    },
                    SyncCommand::Rest {
                        id: b,
                        cmd: RestCommand::DownloadMedia { document_id: d2, content_hash },
                    },
                ] if *a == up && *b == down && *d1 == meta && *d2 == meta
                    && data == &[1, 2, 3] && content_hash.as_str() == file.as_str()
            ),
            "got {sent:?}"
        );
    }

    /// A delete erases the file its message carried, under the channel's meta document. A failed erase waits for
    /// the next connection, and no view sees either answer.
    #[test]
    fn a_delete_erases_its_file_and_retries_on_the_next_connection() {
        let (mut state, ui, sync) = harness();
        let me = Id::<User>::mint();
        signed_in(&mut state, me);
        let (channel_id, segment) = open_channel(&mut state);
        let meta = state.rt().channels()[&channel_id].meta_id().document_id();
        let file = "ab".repeat(32);
        let MurmureCommand::SendMessage {
            draft,
            now,
            confirmed,
            ..
        } = send("", Vec::new())
        else {
            panic!("a send is a SendMessage");
        };
        command(
            &mut state,
            channel_id,
            MurmureCommand::SendMessage {
                draft,
                attachments: vec![Attachment {
                    media_ref: Validated::<ContentHash>::parse(&file).expect("a content hash"),
                    name: "plan.pdf".to_owned(),
                    media_type: MediaType::new("application/pdf".to_owned()),
                    size: Count::<Octets>::new(2048),
                }],
                now,
                confirmed,
            },
        );
        assert!(erases(&drain(&sync)).is_empty(), "a send erases nothing");
        let message = state.rt().channels()[&channel_id]
            .messages(segment)
            .expect("held")[0]
            .id;
        drain(&ui);

        command(
            &mut state,
            channel_id,
            MurmureCommand::DeleteMessage {
                at: segment.into(),
                message,
                now: Instant::<Authored>::new(2),
            },
        );
        let asked = erases(&drain(&sync));
        let [(first, document_id, hash)] = asked.as_slice() else {
            panic!("one erase, got {asked:?}");
        };
        assert_eq!((*document_id, hash.as_str()), (meta, file.as_str()));
        let first = *first;

        drain(&ui);
        state.handle_logic_event(LogicEvent::ApiError {
            id: Some(first),
            request: ambre_bus::ApiRequest::EraseMedia,
            failure: ambre_bus::ApiFailure::NoAnswer,
        });
        assert!(drain(&ui).is_empty(), "a failed erase reaches no view");
        assert!(drain(&sync).is_empty(), "the erase waits");

        state.handle_logic_event(LogicEvent::ConnectionStatus {
            document_id: meta,
            connected: ambre_core::LinkState::Connected,
        });
        let asked = erases(&drain(&sync));
        let [(again, document_id, hash)] = asked.as_slice() else {
            panic!("the erase asked again, got {asked:?}");
        };
        assert_eq!((*document_id, hash.as_str()), (meta, file.as_str()));
        assert_ne!(*again, first);
        let again = *again;

        drain(&ui);
        state.handle_logic_event(LogicEvent::Rest {
            id: Some(again),
            evt: RestEvent::MediaErased {
                document_id: meta,
                content_hash: Validated::<ContentHash>::parse(&file).expect("a content hash"),
            },
        });
        assert!(drain(&ui).is_empty(), "no view awaits the erase");
        state.handle_logic_event(LogicEvent::ConnectionStatus {
            document_id: meta,
            connected: ambre_core::LinkState::Connected,
        });
        assert!(
            erases(&drain(&sync)).is_empty(),
            "an answered erase is done"
        );
    }

    fn log_out(state: &mut LogicState) {
        state.handle_logic_command(LogicCommand::Rest {
            id: ambre_bus::next_request(),
            cmd: RestCommand::Logout,
        });
    }

    /// An erase on its way and an erase owed at the logout
    /// belong to the user who deleted the message. The next session, its
    /// connection and the late answer of the first ask none of them again.
    #[test]
    fn a_logout_forgets_the_erases_the_relay_has_not_answered() {
        let (mut state, _ui, sync) = harness();
        signed_in(&mut state, Id::<User>::mint());
        let (channel_id, segment) = open_channel(&mut state);
        let meta = state.rt().channels()[&channel_id].meta_id().document_id();
        let attachments = ["ab", "cd"]
            .into_iter()
            .map(|byte| Attachment {
                media_ref: Validated::<ContentHash>::parse(byte.repeat(32)).expect("a hash"),
                name: "plan.pdf".to_owned(),
                media_type: MediaType::new("application/pdf".to_owned()),
                size: Count::<Octets>::new(2048),
            })
            .collect();
        let MurmureCommand::SendMessage {
            draft,
            now,
            confirmed,
            ..
        } = send("", Vec::new())
        else {
            panic!("a send is a SendMessage");
        };
        command(
            &mut state,
            channel_id,
            MurmureCommand::SendMessage {
                draft,
                attachments,
                now,
                confirmed,
            },
        );
        let message = state.rt().channels()[&channel_id]
            .messages(segment)
            .expect("held")[0]
            .id;
        drain(&sync);
        command(
            &mut state,
            channel_id,
            MurmureCommand::DeleteMessage {
                at: segment.into(),
                message,
                now: Instant::<Authored>::new(2),
            },
        );
        let asked = erases(&drain(&sync));
        let [(owed, ..), (on_its_way, ..)] = asked.as_slice() else {
            panic!("two erases, got {asked:?}");
        };
        let on_its_way = *on_its_way;
        state.handle_logic_event(LogicEvent::ApiError {
            id: Some(*owed),
            request: ambre_bus::ApiRequest::EraseMedia,
            failure: ambre_bus::ApiFailure::NoAnswer,
        });
        assert!(drain(&sync).is_empty(), "the failed erase waits");

        log_out(&mut state);
        signed_in(&mut state, Id::<User>::mint());
        state.handle_logic_event(LogicEvent::ConnectionStatus {
            document_id: meta,
            connected: ambre_core::LinkState::Connected,
        });
        state.handle_logic_event(LogicEvent::ApiError {
            id: Some(on_its_way),
            request: ambre_bus::ApiRequest::EraseMedia,
            failure: ambre_bus::ApiFailure::NoAnswer,
        });
        state.handle_logic_event(LogicEvent::ConnectionStatus {
            document_id: meta,
            connected: ambre_core::LinkState::Connected,
        });
        let sent = erases(&drain(&sync));
        assert!(sent.is_empty(), "no later session erases: {sent:?}");
    }

    /// A preview fetch on its way at the logout is forgotten.
    /// The next session that paints the same image fetches it under its own
    /// session, and the late answer of the first reaches no view.
    #[test]
    #[cfg(not(target_arch = "wasm32"))]
    fn a_logout_forgets_the_preview_fetches_the_relay_has_not_answered() {
        let (mut state, ui, sync) = harness();
        signed_in(&mut state, Id::<User>::mint());
        let (channel_id, _) = open_channel(&mut state);
        let file = Validated::<ContentHash>::parse("cd".repeat(32)).expect("a content hash");
        drain(&sync);
        let media_ref = file.clone();
        command(
            &mut state,
            channel_id,
            MurmureCommand::ShowPicture { media_ref },
        );
        let [(first, meta, _)] = downloads(&drain(&sync))[..] else {
            panic!("one fetch");
        };

        log_out(&mut state);
        signed_in(&mut state, Id::<User>::mint());
        state.hold_channel(
            ChannelState::create(channel_id, ChannelType::Public, None, NOW).expect("create"),
        );
        drain(&sync);
        let media_ref = file.clone();
        command(
            &mut state,
            channel_id,
            MurmureCommand::ShowPicture { media_ref },
        );
        let fetched = downloads(&drain(&sync));
        assert!(
            matches!(fetched.as_slice(), [(id, ..)] if *id != first),
            "the next session fetches anew, got {fetched:?}"
        );

        let mut png = Vec::new();
        image::RgbaImage::from_pixel(1, 1, image::Rgba([1, 2, 3, 255]))
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .expect("png");
        drain(&ui);
        state.handle_logic_event(LogicEvent::Rest {
            id: Some(first),
            evt: RestEvent::MediaDownloaded {
                document_id: meta,
                content_hash: file,
                data: png.into(),
            },
        });
        assert!(
            !drain(&ui).iter().any(|e| matches!(
                e,
                UiEvent::Channel {
                    event: MurmureEvent::Picture { .. },
                    ..
                }
            )),
            "the late answer reaches no view"
        );
    }

    /// The image of a preview, as `(request, document, file)`.
    #[cfg(not(target_arch = "wasm32"))]
    fn downloads(
        sent: &[SyncCommand],
    ) -> Vec<(
        Seq<Request>,
        Id<Document>,
        ambre_core::Validated<ambre_core::ContentHash>,
    )> {
        sent.iter()
            .filter_map(|command| match command {
                SyncCommand::Rest {
                    id,
                    cmd:
                        RestCommand::DownloadMedia {
                            document_id,
                            content_hash,
                        },
                } => Some((*id, *document_id, content_hash.clone())),
                _ => None,
            })
            .collect()
    }

    /// An image a view paints is fetched once under the channel's meta, and reaches the view decoded, never as its file. One
    /// the relay refuses reaches it as nothing.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn a_preview_is_fetched_once_and_reaches_the_view_decoded() {
        let (mut state, ui, sync) = harness();
        let (link, ends) = crate::converter_thread::converter_channels(&Doorbell::new());
        state.offload = crate::converter_thread::Offload::over(link);
        let (mut thread, _) = crate::converter_thread::ConverterThread::new(ends);
        signed_in(&mut state, Id::<User>::mint());
        let (channel_id, _) = open_channel(&mut state);
        let meta = state.rt().channels()[&channel_id].meta_id().document_id();
        let file = Validated::<ContentHash>::parse("cd".repeat(32)).expect("a content hash");
        drain(&sync);
        for _ in 0..2 {
            let media_ref = file.clone();
            command(
                &mut state,
                channel_id,
                MurmureCommand::ShowPicture { media_ref },
            );
        }
        let asked = downloads(&drain(&sync));
        let [(id, document_id, hash)] = asked.as_slice() else {
            panic!("one fetch, got {asked:?}");
        };
        assert_eq!((*document_id, hash.as_str()), (meta, file.as_str()));
        let mut png = Vec::new();
        image::RgbaImage::from_pixel(4, 2, image::Rgba([9, 8, 7, 255]))
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .expect("png");
        drain(&ui);
        state.handle_logic_event(LogicEvent::Rest {
            id: Some(*id),
            evt: RestEvent::MediaDownloaded {
                document_id: meta,
                content_hash: file.clone(),
                data: png.into(),
            },
        });
        // The PNG decodes on the converter thread, never here (`ARCH-097`).
        assert!(drain(&ui).is_empty(), "the preview waits on the module");
        state.handle_logic_event(LogicEvent::ConverterFetched {
            family: ambre_core::ConverterFamily::Images,
            module: ambre_core::Payload::new(Vec::new()),
        });
        thread.serve();
        state.take_converted();
        let got = drain(&ui);
        let [
            UiEvent::Channel {
                channel_id: to,
                event:
                    MurmureEvent::Picture {
                        media_ref,
                        picture: Some(picture),
                    },
            },
        ] = got.as_slice()
        else {
            panic!("one decoded picture, got {} events", got.len());
        };
        assert_eq!((*to, media_ref), (channel_id, &file));
        assert_eq!(picture.size, ambre_core::Size::new(Px::new(4), Px::new(2)));
        assert_eq!(picture.frames[0].pixels.as_bytes()[..4], [9, 8, 7, 255]);

        let media_ref = file.clone();
        command(
            &mut state,
            channel_id,
            MurmureCommand::ShowPicture { media_ref },
        );
        let [(again, _, _)] = downloads(&drain(&sync))[..] else {
            panic!("a fetch once the first was answered");
        };
        state.handle_logic_event(LogicEvent::ApiError {
            id: Some(again),
            request: ambre_bus::ApiRequest::DownloadMedia,
            failure: ambre_bus::ApiFailure::Relay {
                status: ambre_bus::HttpStatus::new(404),
                code: None,
            },
        });
        let got = drain(&ui);
        assert!(
            matches!(
                got.as_slice(),
                [UiEvent::Channel {
                    event: MurmureEvent::Picture { picture: None, .. },
                    ..
                }]
            ),
            "a refused fetch reaches the view as nothing, got {} events",
            got.len()
        );
    }

    /// `ARCH-097`: a JPEG preview decodes on the converter
    /// thread, once the images module loads. A module that does not load
    /// shows no preview, and the next paint asks again.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn a_jpeg_preview_decodes_on_the_converter_thread_or_shows_none() {
        let (mut state, ui, sync) = harness();
        let (link, ends) = crate::converter_thread::converter_channels(&Doorbell::new());
        state.offload = crate::converter_thread::Offload::over(link);
        let (mut thread, _) = crate::converter_thread::ConverterThread::new(ends);
        signed_in(&mut state, Id::<User>::mint());
        let (channel_id, _) = open_channel(&mut state);
        let meta = state.rt().channels()[&channel_id].meta_id().document_id();
        let file = Validated::<ContentHash>::parse("ef".repeat(32)).expect("a content hash");
        let mut jpeg = Vec::new();
        image::codecs::jpeg::JpegEncoder::new(&mut jpeg)
            .encode_image(&image::RgbImage::from_pixel(
                960,
                320,
                image::Rgb([200, 0, 0]),
            ))
            .expect("a JPEG encodes");
        let show_and_arrive = |state: &mut LogicState| {
            command(
                state,
                channel_id,
                MurmureCommand::ShowPicture {
                    media_ref: file.clone(),
                },
            );
            let [(id, _, _)] = downloads(&drain(&sync))[..] else {
                panic!("one fetch");
            };
            state.handle_logic_event(LogicEvent::Rest {
                id: Some(id),
                evt: RestEvent::MediaDownloaded {
                    document_id: meta,
                    content_hash: file.clone(),
                    data: jpeg.clone().into(),
                },
            });
        };
        drain(&ui);
        show_and_arrive(&mut state);
        assert!(drain(&ui).is_empty(), "the preview waits on the module");
        state.handle_logic_event(LogicEvent::ConverterRefused {
            family: ambre_core::ConverterFamily::Images,
            cause: ambre_core::ConverterRefusal::NotFetched,
        });
        let got = drain(&ui);
        assert!(
            matches!(
                got.as_slice(),
                [UiEvent::Channel {
                    event: MurmureEvent::Picture { picture: None, .. },
                    ..
                }]
            ),
            "no preview, got {} events",
            got.len()
        );
        drain(&sync);
        // The next paint asks again, and the module now loads.
        show_and_arrive(&mut state);
        state.handle_logic_event(LogicEvent::ConverterFetched {
            family: ambre_core::ConverterFamily::Images,
            module: ambre_core::Payload::new(Vec::new()),
        });
        assert!(
            drain(&ui).is_empty(),
            "the decode runs off the Logic thread"
        );
        thread.serve();
        state.take_converted();
        let got = drain(&ui);
        let [
            UiEvent::Channel {
                event:
                    MurmureEvent::Picture {
                        picture: Some(picture),
                        ..
                    },
                ..
            },
        ] = got.as_slice()
        else {
            panic!("one decoded preview, got {} events", got.len());
        };
        assert_eq!(
            picture.size,
            ambre_core::Size::new(Px::new(480), Px::new(160))
        );
        assert_eq!(picture.frames.len(), 1);
        // No document was written on the way: the preview is the view's.
        assert!(
            !drain(&sync)
                .iter()
                .any(|c| matches!(c, SyncCommand::SendUpdate { .. })),
            "a preview writes no document"
        );
    }

    // --- Who watches a channel, and who types in it ---

    fn at(offset: u64) -> Instant<Authored> {
        Instant::<Authored>::new(NOW.get() + offset)
    }

    fn command(state: &mut LogicState, channel_id: Id<Channel>, command: MurmureCommand) {
        state.handle_logic_command(LogicCommand::Channel {
            channel_id,
            command,
        });
    }

    /// The ephemerals that left for the Sync worker, by document. Any other
    /// command fails the test.
    fn ephemerals(
        sync: &Receiver<SyncCommand>,
    ) -> Vec<(
        Id<Document>,
        ambre_core::Payload<ambre_core::EphemeralBytes>,
    )> {
        drain(sync)
            .into_iter()
            .map(|sent| match sent {
                SyncCommand::SendEphemeral {
                    document_id,
                    payload,
                } => (document_id, payload),
                other => panic!("only an ephemeral leaves, got {other:?}"),
            })
            .collect()
    }

    #[expect(
        clippy::type_complexity,
        reason = "each `Id<_>` names its kind, and no alias may hide it"
    )]
    fn watchers(ui: &Receiver<UiEvent>) -> Vec<(Id<Channel>, Vec<Id<User>>, Vec<Id<User>>)> {
        drain(ui)
            .into_iter()
            .filter_map(|event| match event {
                UiEvent::Channel {
                    channel_id,
                    event:
                        MurmureEvent::Watchers {
                            watching, typing, ..
                        },
                } => Some((channel_id, watching, typing)),
                _ => None,
            })
            .collect()
    }

    fn hear(state: &mut LogicState, document_id: Id<Document>, from: Id<User>, sample: Sample) {
        state.handle_logic_event(LogicEvent::RemoteEphemeral {
            document_id,
            from_user_id: from,
            payload: sample.bytes(),
        });
    }

    /// The view's tick sends this user's watching sample on the channel's
    /// `meta` alone, once per refresh period, with no setting to turn on.
    /// Only the members of the channel hold the key Sync seals it with (the
    /// Murmure typing and presence spec).
    #[test]
    fn a_channel_tick_sends_its_watching_sample_on_its_meta_alone() {
        let (mut state, _ui, sync) = harness();
        signed_in(&mut state, Id::<User>::mint());
        let (channel_id, _active) = open_channel(&mut state);
        let meta = state.rt().channels()[&channel_id].meta_id().document_id();
        drain(&sync);

        command(&mut state, channel_id, MurmureCommand::Tick { now: NOW });
        assert_eq!(ephemerals(&sync), [(meta, Sample::Watching.bytes())]);
        command(
            &mut state,
            channel_id,
            MurmureCommand::Tick { now: at(1_000) },
        );
        assert!(ephemerals(&sync).is_empty(), "within the refresh period");
        let refreshed = MurmureCommand::Tick {
            now: at(ambre_murmure::REFRESH_PERIOD.get()),
        };
        command(&mut state, channel_id, refreshed);
        assert_eq!(ephemerals(&sync), [(meta, Sample::Watching.bytes())]);
    }

    /// A keystroke sends a typing sample once per typing period, as an
    /// ephemeral of the `meta`. No document of the channel moves.
    #[test]
    fn typing_leaves_as_an_ephemeral_and_writes_no_document() {
        let (mut state, _ui, sync) = harness();
        signed_in(&mut state, Id::<User>::mint());
        let (channel_id, _active) = open_channel(&mut state);
        let channel = &state.rt().channels()[&channel_id];
        let meta = channel.meta_id().document_id();
        let versions = |state: &LogicState| {
            let channel = &state.rt().channels()[&channel_id];
            channel
                .document_ids()
                .into_iter()
                .map(|id| channel.snapshot(id).expect("held"))
                .collect::<Vec<_>>()
        };
        let before = versions(&state);
        drain(&sync);

        for offset in [0, 500, ambre_murmure::TYPING_REFRESH.get()] {
            command(
                &mut state,
                channel_id,
                MurmureCommand::Typing {
                    now: at(offset),
                    thread: None,
                },
            );
        }
        assert_eq!(
            ephemerals(&sync),
            [
                (meta, Sample::Typing.bytes()),
                (meta, Sample::Typing.bytes())
            ]
        );
        assert_eq!(versions(&state), before);
    }

    /// A key in a thread's composer sends a sample naming its root, at once
    /// after one in the channel. A member heard there shows under that root,
    /// and out of the channel's list.
    #[test]
    fn typing_in_a_thread_names_its_root() {
        let (mut state, ui, sync) = harness();
        signed_in(&mut state, Id::<User>::mint());
        let (channel_id, _active) = open_channel(&mut state);
        let meta = state.rt().channels()[&channel_id].meta_id().document_id();
        command(&mut state, channel_id, MurmureCommand::Tick { now: NOW });
        drain(&sync);
        drain(&ui);
        let root = Id::<Message>::mint();
        for thread in [None, Some(root)] {
            let typing = MurmureCommand::Typing { now: NOW, thread };
            command(&mut state, channel_id, typing);
        }
        let sent = [Sample::Typing, Sample::TypingIn(root)].map(|s| (meta, s.bytes()));
        assert_eq!(ephemerals(&sync), sent);

        let bob = Id::<User>::mint();
        hear(&mut state, meta, bob, Sample::TypingIn(root));
        let shown: Vec<_> = drain(&ui)
            .into_iter()
            .filter_map(|event| match event {
                UiEvent::Channel {
                    event:
                        MurmureEvent::Watchers {
                            typing, in_threads, ..
                        },
                    ..
                } => Some((typing, in_threads)),
                _ => None,
            })
            .collect();
        assert_eq!(shown, [(Vec::new(), vec![(root, vec![bob])])]);
    }

    /// A sample heard shows its member, and a newcomer gets this user's own
    /// sample at once. Typing expires once no sample refreshes it, and the
    /// relay's notice of a leave drops the member.
    #[test]
    fn a_sample_heard_shows_its_member_until_the_expiry_or_a_leave() {
        let (mut state, ui, sync) = harness();
        let me = Id::<User>::mint();
        signed_in(&mut state, me);
        let (channel_id, _active) = open_channel(&mut state);
        let meta = state.rt().channels()[&channel_id].meta_id().document_id();
        command(&mut state, channel_id, MurmureCommand::Tick { now: NOW });
        drain(&sync);
        drain(&ui);
        let bob = Id::<User>::mint();

        hear(&mut state, meta, bob, Sample::Typing);
        assert_eq!(watchers(&ui), [(channel_id, vec![bob], vec![bob])]);
        assert_eq!(ephemerals(&sync), [(meta, Sample::Watching.bytes())]);
        hear(&mut state, meta, me, Sample::Typing);
        assert!(
            watchers(&ui).is_empty(),
            "this user's own sample shows nothing"
        );

        let expired = MurmureCommand::Tick {
            now: at(ambre_murmure::TYPING_EXPIRY.get() + 1),
        };
        command(&mut state, channel_id, expired);
        assert_eq!(watchers(&ui), [(channel_id, vec![bob], Vec::new())]);

        state.handle_logic_event(LogicEvent::PeerLeft {
            document_id: meta,
            user_id: bob,
        });
        assert_eq!(watchers(&ui), [(channel_id, Vec::new(), Vec::new())]);
    }

    /// A sample names no channel: the same bytes leave for two channels, each
    /// on its own `meta`. One heard on a channel shows in that channel alone.
    #[test]
    fn a_sample_shows_in_its_own_channel_alone() {
        let (mut state, ui, sync) = harness();
        signed_in(&mut state, Id::<User>::mint());
        let (first, _) = open_channel(&mut state);
        let (second, _) = open_channel(&mut state);
        let meta_of = |state: &LogicState, id| state.rt().channels()[&id].meta_id().document_id();
        let (first_meta, second_meta) = (meta_of(&state, first), meta_of(&state, second));
        drain(&sync);

        command(&mut state, first, MurmureCommand::Tick { now: NOW });
        command(&mut state, second, MurmureCommand::Tick { now: NOW });
        assert_eq!(
            ephemerals(&sync),
            [
                (first_meta, Sample::Watching.bytes()),
                (second_meta, Sample::Watching.bytes())
            ]
        );
        drain(&ui);
        let bob = Id::<User>::mint();
        hear(&mut state, first_meta, bob, Sample::Watching);
        assert_eq!(watchers(&ui), [(first, vec![bob], Vec::new())]);
    }

    // --- The members of a channel, and the mentions they screen ---

    fn member(user_id: Id<User>) -> MemberInfo {
        MemberInfo {
            user_id,
            display_name: ambre_core::Validated::<ambre_core::DisplayName>::parse("bob").unwrap(),
            email: ambre_core::Email::from("bob@example.org"),
            role: Role::Editor,
            standing: ambre_bus::MemberStanding::Verified,
        }
    }

    /// The channel events that reached the view.
    fn channel_events(ui: &Receiver<UiEvent>) -> Vec<(Id<Channel>, MurmureEvent)> {
        drain(ui)
            .into_iter()
            .filter_map(|event| match event {
                UiEvent::Channel { channel_id, event } => Some((channel_id, event)),
                _ => None,
            })
            .collect()
    }

    /// The summaries of the updates that left for the Sync worker.
    fn summaries(sync: &Receiver<SyncCommand>) -> Vec<ambre_bus::ActivitySummary> {
        drain(sync)
            .into_iter()
            .filter_map(|sent| match sent {
                SyncCommand::SendUpdate {
                    summary: Some(summary),
                    ..
                } => Some(summary),
                _ => None,
            })
            .collect()
    }

    /// The members the relay lists on a channel's `meta` reach the view. A
    /// mention of a user outside them leaves no mark in the summary (the
    /// Murmure mentions spec).
    #[test]
    fn the_members_of_a_channel_reach_the_view_and_screen_its_mentions() {
        let (mut state, ui, sync) = harness();
        let alice = Id::<User>::mint();
        signed_in(&mut state, alice);
        let (channel_id, _active) = open_channel(&mut state);
        let meta = state.rt().channels()[&channel_id].meta_id().document_id();
        drain(&ui);
        drain(&sync);
        let (bob, eve) = (Id::<User>::mint(), Id::<User>::mint());
        let members = vec![member(alice), member(bob)];
        state.handle_logic_event(LogicEvent::MembersChanged {
            document_id: meta,
            members: members.clone(),
        });
        assert_eq!(
            channel_events(&ui),
            [(channel_id, MurmureEvent::Members { members })]
        );

        let mention = |target, range| Mention { target, range };
        let draft = send(
            "@bob @eve",
            vec![
                mention(
                    MentionTarget::User(bob),
                    Span::<Char>::from_usize_range(0..4),
                ),
                mention(
                    MentionTarget::User(eve),
                    Span::<Char>::from_usize_range(5..9),
                ),
            ],
        );
        command(&mut state, channel_id, draft);
        let [summary] = summaries(&sync).try_into().expect("one update");
        assert_eq!(summary.mentions, [bob]);
    }

    /// An `@channel` past the member threshold asks the view first and
    /// sends nothing. The confirmed resend leaves and notifies every member
    /// (the Murmure mentions spec).
    #[test]
    fn an_at_channel_past_the_threshold_asks_before_it_sends() {
        let (mut state, ui, sync) = harness();
        let alice = Id::<User>::mint();
        signed_in(&mut state, alice);
        let (channel_id, active) = open_channel(&mut state);
        let meta = state.rt().channels()[&channel_id].meta_id().document_id();
        let members: Vec<MemberInfo> = (0..=ambre_murmure::CONFIRM_CHANNEL_PAST.get())
            .map(|_| member(Id::<User>::mint()))
            .collect();
        state.handle_logic_event(LogicEvent::MembersChanged {
            document_id: meta,
            members,
        });
        drain(&ui);
        drain(&sync);
        let all = vec![Mention {
            target: MentionTarget::Channel,
            range: Span::<Char>::from_usize_range(0..8),
        }];

        command(&mut state, channel_id, send("@channel", all.clone()));
        let draft = MessageDraft {
            text: "@channel".to_owned(),
            mentions: all.clone(),
            styles: Vec::new(),
        };
        let asked = MurmureEvent::ConfirmChannelMention {
            members: ambre_core::Count::<ambre_core::Members>::new(
                ambre_murmure::CONFIRM_CHANNEL_PAST.get() + 1,
            ),
            held: HeldSend {
                draft: draft.clone(),
                attachments: Vec::new(),
                reply: None,
            },
        };
        assert_eq!(channel_events(&ui), [(channel_id, asked)]);
        assert!(drain(&sync).is_empty(), "nothing leaves unconfirmed");

        let confirmed = MurmureCommand::SendMessage {
            draft: draft.clone(),
            attachments: Vec::new(),
            now: NOW,
            confirmed: ambre_murmure::message::Confirmation::Confirmed,
        };
        command(&mut state, channel_id, confirmed);
        let [summary] = summaries(&sync).try_into().expect("one update");
        assert_eq!(summary.mentions_all, ambre_bus::MentionScope::All);
        drain(&ui);

        // A reply is held with its thread, so the view sends it back there.
        let root = state.rt().channels()[&channel_id]
            .messages(active)
            .expect("held")[0]
            .id;
        let reply = MurmureCommand::Reply {
            root,
            draft: draft.clone(),
            also_in_channel: ambre_murmure::message::ReplyEcho::AlsoInChannel,
            now: NOW,
            confirmed: ambre_murmure::message::Confirmation::Unconfirmed,
        };
        command(&mut state, channel_id, reply);
        let held = HeldSend {
            draft,
            attachments: Vec::new(),
            reply: Some(HeldReply {
                root,
                also_in_channel: ambre_murmure::message::ReplyEcho::AlsoInChannel,
            }),
        };
        let asked = MurmureEvent::ConfirmChannelMention {
            members: ambre_core::Count::<ambre_core::Members>::new(
                ambre_murmure::CONFIRM_CHANNEL_PAST.get() + 1,
            ),
            held,
        };
        assert_eq!(channel_events(&ui), [(channel_id, asked)]);
        assert!(drain(&sync).is_empty(), "no reply leaves unconfirmed");
    }

    // --- The close of a full segment ---

    /// An open channel whose segments close past `size` messages.
    fn small_channel(state: &mut LogicState, size: usize) -> (Id<Channel>, Id<Segment>) {
        signed_in(state, Id::<User>::mint());
        let channel_id = Id::<Channel>::mint();
        let channel = ChannelState::create(channel_id, ChannelType::Public, None, NOW)
            .expect("create")
            .with_segment_size(ambre_core::Count::<ambre_bus::Messages>::of(size));
        let active = channel.active_id();
        state.hold_channel(channel);
        (channel_id, active)
    }

    /// The closes Logic asked of the Sync worker.
    fn closes(sync: &Receiver<SyncCommand>) -> Vec<(Id<Segment>, Id<Segment>)> {
        drain(sync)
            .into_iter()
            .filter_map(|c| match c {
                SyncCommand::CloseSegment {
                    closing, successor, ..
                } => Some((closing, successor)),
                _ => None,
            })
            .collect()
    }

    /// The close of the Murmure segments spec, in Logic. The send that fills
    /// the segment asks for one close. The relay's answer leaves as a `meta`
    /// delta, and the successor opens and takes the next message.
    #[test]
    fn a_full_segment_closes_into_the_successor_the_relay_names() {
        let (mut state, _ui, sync) = harness();
        let (channel_id, old) = small_channel(&mut state, 1);
        command(&mut state, channel_id, send("one", Vec::new()));
        let [(closing, successor)] = closes(&sync).try_into().expect("one close");
        assert_eq!(closing, old);
        command(&mut state, channel_id, send("two", Vec::new()));
        assert!(closes(&sync).is_empty(), "the close is asked once");

        state.handle_logic_event(LogicEvent::SegmentClosed {
            channel_id,
            closing,
            active: successor,
            successor: ambre_murmure::Successor::Created,
        });
        let meta = state.rt().channels()[&channel_id].meta_id().document_id();
        let sent = drain(&sync);
        assert!(sent.iter().any(|c| matches!(
            c,
            SyncCommand::SendUpdate { document_id, summary: None, .. } if *document_id == meta
        )));
        assert!(sent.iter().any(|c| matches!(
            c,
            SyncCommand::OpenDocument { document_id, .. }
                if *document_id == successor.document_id()
        )));
        let channel = &state.rt().channels()[&channel_id];
        assert_eq!(
            (channel.active_id(), channel.closing_id()),
            (successor, Some(old))
        );
    }

    /// Holds `SYNC-164` in Logic. A failed close asks again on the next send
    /// that finds the segment full, with the successor it already created.
    /// An answer nobody asked for changes nothing.
    #[test]
    fn a_failed_close_is_asked_again() {
        let (mut state, _ui, sync) = harness();
        let (channel_id, old) = small_channel(&mut state, 1);
        command(&mut state, channel_id, send("one", Vec::new()));
        let [(closing, successor)] = closes(&sync).try_into().expect("one close");
        state.handle_logic_event(LogicEvent::SegmentCloseFailed {
            channel_id,
            closing,
            failure: ambre_bus::ApiFailure::NoAnswer,
        });
        state.handle_logic_event(LogicEvent::SegmentClosed {
            channel_id,
            closing,
            active: Id::<Segment>::mint(),
            successor: ambre_murmure::Successor::Created,
        });
        assert_eq!(state.rt().channels()[&channel_id].active_id(), old);
        command(&mut state, channel_id, send("two", Vec::new()));
        assert_eq!(
            closes(&sync),
            [(closing, successor)],
            "the close is asked again, into the same successor"
        );
        state.handle_logic_event(LogicEvent::SegmentClosed {
            channel_id,
            closing,
            active: successor,
            successor: ambre_murmure::Successor::Created,
        });
        assert_eq!(state.rt().channels()[&channel_id].active_id(), successor);
        assert!(state.rt().close_successors().is_empty());
    }

    /// A rotation of the channel's `meta` closes its active segment at the
    /// next tick, far below its size (the Murmure channels spec). A rotation
    /// of another document closes nothing.
    #[test]
    fn a_rotation_of_meta_closes_the_active_segment_at_the_next_tick() {
        let (mut state, _ui, sync) = harness();
        let (channel_id, old) = small_channel(&mut state, 100);
        command(&mut state, channel_id, send("one", Vec::new()));
        state.handle_logic_event(LogicEvent::KeyRotated {
            document_id: Id::<Document>::mint(),
        });
        command(&mut state, channel_id, MurmureCommand::Tick { now: NOW });
        assert!(closes(&sync).is_empty(), "another document rotated");

        let meta = state.rt().channels()[&channel_id].meta_id().document_id();
        state.handle_logic_event(LogicEvent::KeyRotated { document_id: meta });
        command(&mut state, channel_id, MurmureCommand::Tick { now: NOW });
        let [(closing, _)] = closes(&sync).try_into().expect("one close");
        assert_eq!(closing, old);
    }

    // --- The finalization of a closing segment ---

    /// A channel whose first segment closed into a successor. Returns the
    /// closing segment.
    fn closed_channel(
        state: &mut LogicState,
        sync: &Receiver<SyncCommand>,
    ) -> (Id<Channel>, Id<Segment>) {
        let (channel_id, _old) = small_channel(state, 1);
        command(state, channel_id, send("one", Vec::new()));
        let [(closing, successor)] = closes(sync).try_into().expect("one close");
        state.handle_logic_event(LogicEvent::SegmentClosed {
            channel_id,
            closing,
            active: successor,
            successor: ambre_murmure::Successor::Created,
        });
        drain(sync);
        (channel_id, closing)
    }

    /// The finalize commands Logic sent: a lease request names no horizon.
    fn finalizes(
        sync: &Receiver<SyncCommand>,
    ) -> Vec<(Id<Segment>, Option<ambre_core::Seq<ambre_core::Relay>>)> {
        drain(sync)
            .into_iter()
            .filter_map(|c| match c {
                SyncCommand::TakeFinalizeLease { segment, .. } => Some((segment, None)),
                SyncCommand::FinalizeSegment {
                    segment, horizon, ..
                } => Some((segment, Some(horizon))),
                _ => None,
            })
            .collect()
    }

    /// The finalization of the Murmure segments spec, in Logic. The tick
    /// past the close asks for the lease, and the relay's deferral is waited
    /// out. The snapshot leaves once the deltas up to the horizon are in,
    /// and the accepted commit leaves as a `meta` delta.
    #[test]
    fn a_closing_segment_finalizes_once_its_horizon_is_in() {
        let (mut state, _ui, sync) = harness();
        let (channel_id, closing) = closed_channel(&mut state, &sync);
        command(&mut state, channel_id, MurmureCommand::Tick { now: NOW });
        assert_eq!(finalizes(&sync), [(closing, None)], "the lease is asked");
        state.handle_logic_event(LogicEvent::FinalizeLeaseAnswered {
            channel_id,
            segment: closing,
            answer: LeaseAnswer::Deferred {
                retry_in: TimeSpan::<Ms>::new(5_000),
            },
        });
        command(
            &mut state,
            channel_id,
            MurmureCommand::Tick { now: at(1_000) },
        );
        command(
            &mut state,
            channel_id,
            MurmureCommand::Tick { now: at(5_999) },
        );
        assert!(finalizes(&sync).is_empty(), "the deferral runs");
        command(
            &mut state,
            channel_id,
            MurmureCommand::Tick { now: at(6_000) },
        );
        assert_eq!(finalizes(&sync), [(closing, None)]);

        state.handle_logic_event(LogicEvent::FinalizeLeaseAnswered {
            channel_id,
            segment: closing,
            answer: LeaseAnswer::Granted {
                lease: Id::<Lease>::mint(),
                horizon: ambre_core::Seq::<ambre_core::Relay>::new(1),
                expires_in: TimeSpan::<Ms>::new(30_000),
            },
        });
        command(
            &mut state,
            channel_id,
            MurmureCommand::Tick { now: at(7_000) },
        );
        assert!(finalizes(&sync).is_empty(), "the horizon is not in");
        state.route(|m| {
            m.record_channel_seq(
                closing.document_id(),
                ambre_core::Seq::<ambre_core::Relay>::new(1),
                None,
            )
        });
        command(
            &mut state,
            channel_id,
            MurmureCommand::Tick { now: at(8_000) },
        );
        assert_eq!(
            finalizes(&sync),
            [(closing, Some(ambre_core::Seq::<ambre_core::Relay>::new(1)))]
        );

        state.handle_logic_event(LogicEvent::SegmentFinalized {
            channel_id,
            segment: closing,
        });
        let meta = state.rt().channels()[&channel_id].meta_id().document_id();
        assert!(drain(&sync).iter().any(|c| matches!(
            c,
            SyncCommand::SendUpdate { document_id, summary: None, .. } if *document_id == meta
        )));
        assert_eq!(state.rt().channels()[&channel_id].closing_id(), None);
    }

    /// Ask, take and commit the lease of `segment` at the horizon 1.
    fn finalize_at_one(
        state: &mut LogicState,
        sync: &Receiver<SyncCommand>,
        channel_id: Id<Channel>,
        segment: Id<Segment>,
        now: Instant<Authored>,
    ) {
        command(state, channel_id, MurmureCommand::Tick { now });
        assert_eq!(finalizes(sync), [(segment, None)], "the lease is asked");
        state.handle_logic_event(LogicEvent::FinalizeLeaseAnswered {
            channel_id,
            segment,
            answer: LeaseAnswer::Granted {
                lease: Id::<Lease>::mint(),
                horizon: ambre_core::Seq::<ambre_core::Relay>::new(1),
                expires_in: TimeSpan::<Ms>::new(30_000),
            },
        });
        command(state, channel_id, MurmureCommand::Tick { now });
        assert_eq!(
            finalizes(sync),
            [(segment, Some(ambre_core::Seq::<ambre_core::Relay>::new(1)))]
        );
        state.handle_logic_event(LogicEvent::SegmentFinalized {
            channel_id,
            segment,
        });
    }

    // --- A segment that cannot open: a band in the list ---

    /// A channel whose one final segment this client has not loaded, with
    /// the view's ask for it already sent. Returns the final segment.
    fn unloaded_final(
        state: &mut LogicState,
        sync: &Receiver<SyncCommand>,
    ) -> (Id<Channel>, Id<Segment>, Id<Segment>) {
        let (channel_id, closed) = closed_channel(state, sync);
        state.route(|m| {
            m.record_channel_seq(
                closed.document_id(),
                ambre_core::Seq::<ambre_core::Relay>::new(1),
                None,
            )
        });
        finalize_at_one(state, sync, channel_id, closed, NOW);
        let channel = &state.rt().channels()[&channel_id];
        let (meta, active) = (channel.meta_id(), channel.active_id());
        let meta_bytes = channel.snapshot(meta.document_id()).expect("meta");
        let segments = HashMap::from([(
            active,
            channel.snapshot(active.document_id()).expect("active"),
        )]);
        let reopened = ChannelState::open(channel_id, meta, &meta_bytes, &segments).expect("open");
        state.hold_channel(crate::identified(reopened));
        drain(sync);
        command(
            state,
            channel_id,
            MurmureCommand::LoadOlder { segment: active },
        );
        assert_eq!(sent(sync), [Sent::Open(closed.document_id())]);
        (channel_id, closed, active)
    }

    /// A channel whose closing segment is listed, opening in a second
    /// client: `meta` and the active segment are in, the closing one not.
    fn opening_past_closing() -> (
        LogicState,
        Receiver<UiEvent>,
        Receiver<SyncCommand>,
        Id<Channel>,
        Id<Segment>,
    ) {
        let (mut first, _first_ui, first_sync) = harness();
        let (channel_id, closing) = closed_channel(&mut first, &first_sync);
        let stored = first
            .rt_mut()
            .channels_mut()
            .remove(&channel_id)
            .expect("channel");
        let (mut state, ui, sync) = harness();
        state.handle_logic_command(LogicCommand::OpenChannel { channel_id });
        state.handle_logic_event(LogicEvent::ChannelDocuments {
            channel_id,
            documents: listing(&stored),
        });
        serve(&mut state, &stored, stored.meta_id().document_id());
        serve(&mut state, &stored, stored.active_id().document_id());
        drain(&sync);
        drain(&ui);
        (state, ui, sync, channel_id, closing)
    }

    /// A closing segment whose key never came is given up: the channel opens
    /// without it, shows it as a band, and ends its session.
    #[test]
    fn a_closing_segment_with_no_key_still_opens_the_channel() {
        let (mut state, ui, sync, channel_id, closing) = opening_past_closing();
        state.handle_logic_event(LogicEvent::DocumentOpenFailed {
            document_id: closing.document_id(),
            cause: OpenFailure::KeyNotReceived,
        });
        assert_eq!(sent(&sync), [Sent::Close(closing.document_id())]);
        assert!(state.rt().opening().is_empty());
        assert_eq!(state.rt().channels()[&channel_id].closing_id(), None);
        assert_eq!(bands(&ui), [(closing, Unreadable::KeyNotReceived)]);
    }

    /// A transient failure of the closing segment still ends the opening: a
    /// later open can clear it.
    #[test]
    fn a_transient_failure_of_the_closing_segment_ends_the_opening() {
        let (mut state, ui, _sync, channel_id, closing) = opening_past_closing();
        state.handle_logic_event(LogicEvent::DocumentOpenFailed {
            document_id: closing.document_id(),
            cause: OpenFailure::Transient,
        });
        assert!(state.rt().opening().is_empty());
        assert!(!state.rt().channels().contains_key(&channel_id));
        assert!(drain(&ui).iter().any(|e| matches!(
            e,
            UiEvent::Channel {
                event: MurmureEvent::OpenFailed,
                ..
            }
        )));
    }

    fn bands(ui: &Receiver<UiEvent>) -> Vec<(Id<Segment>, Unreadable)> {
        drain(ui)
            .into_iter()
            .filter_map(|e| match e {
                UiEvent::Channel {
                    event: MurmureEvent::Unreadable { segment, cause, .. },
                    ..
                } => Some((segment, cause)),
                _ => None,
            })
            .collect()
    }

    /// A key that never reached this device shows the segment as one band,
    /// and the segment can be asked for again.
    #[test]
    fn a_key_not_received_shows_one_band_for_the_segment() {
        let (mut state, ui, sync) = harness();
        let (channel_id, segment, active) = unloaded_final(&mut state, &sync);
        drain(&ui);
        state.handle_logic_event(LogicEvent::DocumentOpenFailed {
            document_id: segment.document_id(),
            cause: OpenFailure::KeyNotReceived,
        });
        assert_eq!(bands(&ui), [(segment, Unreadable::KeyNotReceived)]);
        command(
            &mut state,
            channel_id,
            MurmureCommand::LoadOlder { segment: active },
        );
        assert_eq!(sent(&sync), [Sent::Open(segment.document_id())]);
    }

    /// Holds `SYNC-231` on the Logic side. An open channel whose log starts
    /// outside its space ends whole. Every document leaves Sync, and the
    /// channel leaves memory. The view drops it as one that failed to open. The same failure of a second document of it shows nothing more.
    #[test]
    fn a_channel_rooted_outside_its_space_ends_and_fails_its_open() {
        let (mut state, ui, sync) = harness();
        let (channel_id, active) = small_channel(&mut state, 8);
        let meta = state.rt().channels()[&channel_id].meta_id();
        drain(&ui);
        drain(&sync);
        state.handle_logic_event(LogicEvent::DocumentOpenFailed {
            document_id: meta.document_id(),
            cause: OpenFailure::RootOutsideSpace,
        });
        let closed: HashSet<Id<Document>> = sent(&sync)
            .into_iter()
            .filter_map(|s| match s {
                Sent::Close(document_id) => Some(document_id),
                _ => None,
            })
            .collect();
        assert_eq!(
            closed,
            HashSet::from([meta.document_id(), active.document_id()]),
            "every document of the channel leaves Sync"
        );
        assert!(!state.rt().channels().contains_key(&channel_id));
        assert_eq!(
            channel_events(&ui),
            [(channel_id, MurmureEvent::OpenFailed)],
            "the view drops the channel as one that failed to open"
        );
        state.handle_logic_event(LogicEvent::DocumentOpenFailed {
            document_id: active.document_id(),
            cause: OpenFailure::RootOutsideSpace,
        });
        assert!(
            drain(&ui).is_empty(),
            "no second notice, no bare document failure"
        );
        assert_eq!(sent(&sync), [Sent::Close(active.document_id())]);
    }

    /// Holds `SYNC-231` for a channel still opening: the failure ends the
    /// opening, and the view drops the channel.
    #[test]
    fn an_opening_rooted_outside_its_space_ends() {
        let (mut state, ui, sync) = harness();
        let (channel_id, stored) = stored_channel();
        state.handle_logic_command(LogicCommand::OpenChannel { channel_id });
        state.handle_logic_event(LogicEvent::ChannelDocuments {
            channel_id,
            documents: listing(&stored),
        });
        drain(&sync);
        drain(&ui);
        state.handle_logic_event(LogicEvent::DocumentOpenFailed {
            document_id: stored.meta_id().document_id(),
            cause: OpenFailure::RootOutsideSpace,
        });
        assert_eq!(sent(&sync), [Sent::Close(stored.meta_id().document_id())]);
        assert_eq!(
            channel_events(&ui),
            [(channel_id, MurmureEvent::OpenFailed)]
        );
        assert!(state.rt().opening().is_empty());
    }

    /// A relay out of reach is no missing key: no band, and the segment can
    /// be asked for again.
    #[test]
    fn a_transient_failure_shows_no_band() {
        let (mut state, ui, sync) = harness();
        let (channel_id, segment, active) = unloaded_final(&mut state, &sync);
        drain(&ui);
        state.handle_logic_event(LogicEvent::DocumentOpenFailed {
            document_id: segment.document_id(),
            cause: OpenFailure::Transient,
        });
        assert!(bands(&ui).is_empty());
        command(
            &mut state,
            channel_id,
            MurmureCommand::LoadOlder { segment: active },
        );
        assert_eq!(sent(&sync), [Sent::Open(segment.document_id())]);
    }

    /// A band asked for again opens its segment once more, and only once
    /// while it is on its way (the Murmure engine spec).
    #[test]
    fn a_final_band_asked_again_opens_its_segment() {
        let (mut state, ui, sync) = harness();
        let (channel_id, segment, active) = unloaded_final(&mut state, &sync);
        state.handle_logic_event(LogicEvent::DocumentOpenFailed {
            document_id: segment.document_id(),
            cause: OpenFailure::KeyNotReceived,
        });
        drain(&ui);
        drain(&sync);
        for _ in 0..2 {
            command(&mut state, channel_id, MurmureCommand::Retry { segment });
        }
        assert_eq!(sent(&sync), [Sent::Open(segment.document_id())]);
        command(
            &mut state,
            channel_id,
            MurmureCommand::Retry { segment: active },
        );
        assert!(sent(&sync).is_empty(), "a live segment is no band");
    }

    /// The closing segment the channel opened without is fetched again by a
    /// new opening of the channel (the Murmure engine spec).
    #[test]
    fn a_closing_band_asked_again_opens_the_channel_anew() {
        let (mut state, ui, sync, channel_id, closing) = opening_past_closing();
        state.handle_logic_event(LogicEvent::DocumentOpenFailed {
            document_id: closing.document_id(),
            cause: OpenFailure::KeyNotReceived,
        });
        drain(&ui);
        drain(&sync);
        command(
            &mut state,
            channel_id,
            MurmureCommand::Retry { segment: closing },
        );
        assert!(!state.rt().channels().contains_key(&channel_id));
        assert!(state.rt().opening().contains_key(&channel_id));
        let asked = sent(&sync);
        assert!(asked.contains(&Sent::List(channel_id)), "{asked:?}");
    }

    /// Bytes that opened under their key and then fail the check show the
    /// segment as damaged, never as a missing key.
    #[test]
    fn a_segment_that_fails_its_check_shows_as_damaged() {
        let (mut state, ui, sync) = harness();
        let (_, segment, _) = unloaded_final(&mut state, &sync);
        drain(&ui);
        state.handle_logic_event(LogicEvent::RemoteSnapshot {
            document_id: segment.document_id(),
            covered_seq: ambre_core::Seq::<ambre_core::Relay>::new(1),
            payload: (vec![0xff; 8]).into(),
        });
        state.handle_logic_event(LogicEvent::DocumentReady {
            document_id: segment.document_id(),
            role: ambre_bus::Role::Editor,
        });
        assert_eq!(bands(&ui), [(segment, Unreadable::Damaged)]);
        assert!(sent(&sync).contains(&Sent::Close(segment.document_id())));
    }

    /// A report dropped the final snapshot of a segment. The view names who
    /// sealed it and who reported it, and this client seals the segment
    /// again with no `meta` delta (the Murmure segments spec).
    #[test]
    fn a_dropped_final_snapshot_is_shown_and_sealed_again() {
        let (mut state, ui, sync) = harness();
        let (channel_id, closing) = closed_channel(&mut state, &sync);
        state.route(|m| {
            m.record_channel_seq(
                closing.document_id(),
                ambre_core::Seq::<ambre_core::Relay>::new(1),
                None,
            )
        });
        finalize_at_one(&mut state, &sync, channel_id, closing, NOW);
        drain(&sync);
        drain(&ui);

        let (author, reporter) = (Id::<User>::mint(), Id::<User>::mint());
        let dropped = |state: &mut LogicState, after| {
            state.handle_logic_event(LogicEvent::SnapshotDropped {
                document_id: closing.document_id(),
                author: Some(author),
                reporter: Some(reporter),
                after,
            });
        };
        dropped(&mut state, ambre_core::AfterDrop::Keep);
        let shown = channel_events(&ui);
        assert!(
            matches!(
                shown.as_slice(),
                [(_, MurmureEvent::SnapshotDropped { segment, author: Some(a), reporter: Some(r) })]
                    if *segment == closing && *a == author && *r == reporter
            ),
            "the view was told {shown:?}"
        );
        command(&mut state, channel_id, MurmureCommand::Tick { now: at(1) });
        assert!(finalizes(&sync).is_empty(), "another member seals it");

        dropped(&mut state, ambre_core::AfterDrop::Refinalize);
        finalize_at_one(&mut state, &sync, channel_id, closing, at(2));
        assert!(
            !drain(&sync)
                .iter()
                .any(|c| matches!(c, SyncCommand::SendUpdate { .. })),
            "meta marks it final already"
        );
        assert_eq!(state.rt().channels()[&channel_id].finalizing_id(), None);
    }

    /// A message the relay refused for the lock leaves again for the active
    /// segment, and the view moves it there (the Murmure segments spec).
    #[test]
    fn a_message_refused_by_the_lock_leaves_for_the_active_segment() {
        let (mut state, ui, sync) = harness();
        let (channel_id, old) = small_channel(&mut state, 1);
        command(&mut state, channel_id, send("one", Vec::new()));
        let mut refused = None;
        let mut successor = None;
        for sent in drain(&sync) {
            match sent {
                SyncCommand::SendUpdate {
                    document_id,
                    payload,
                    ..
                } if document_id == old.document_id() => refused = Some(payload),
                SyncCommand::CloseSegment { successor: s, .. } => successor = Some(s),
                _ => {}
            }
        }
        let (refused, successor) = (refused.expect("the message"), successor.expect("a close"));
        state.handle_logic_event(LogicEvent::SegmentClosed {
            channel_id,
            closing: old,
            active: successor,
            successor: ambre_murmure::Successor::Created,
        });
        let peer = crate::test_peer();
        state
            .rt_mut()
            .channels_mut()
            .get_mut(&channel_id)
            .expect("open")
            .adopt_session(
                successor.document_id(),
                peer,
                ambre_core::Payload::<ambre_core::PeerEntryBytes>::new(
                    peer.get().to_le_bytes().to_vec(),
                ),
            )
            .expect("adopt");
        drain(&sync);
        drain(&ui);

        state.handle_logic_event(LogicEvent::DeltasLocked {
            document_id: old.document_id(),
            deltas: vec![refused],
        });
        assert!(drain(&sync).iter().any(|c| matches!(
            c,
            SyncCommand::SendUpdate { document_id, summary: Some(_), .. }
                if *document_id == successor.document_id()
        )));
        let moved = channel_events(&ui);
        assert!(
            matches!(
                moved.as_slice(),
                [(_, MurmureEvent::Messages { segment, messages })]
                    if *segment == successor && messages[0].text == "one"
            ),
            "the view was told {moved:?}"
        );
        let channel = &state.rt().channels()[&channel_id];
        assert!(channel.messages(old).expect("old").is_empty());
    }

    /// A closed channel whose closing segment the relay locked, with the id
    /// of its one message.
    fn locked_channel(
        state: &mut LogicState,
        sync: &Receiver<SyncCommand>,
    ) -> (Id<Channel>, Id<Segment>, Id<Message>) {
        let (channel_id, closing) = closed_channel(state, sync);
        let active = state.rt().channels()[&channel_id].active_id();
        state.grant_identity(active.document_id());
        state.handle_logic_event(LogicEvent::DeltasLocked {
            document_id: closing.document_id(),
            deltas: Vec::new(),
        });
        drain(sync);
        let message = state.rt().channels()[&channel_id]
            .messages(closing)
            .expect("held")[0]
            .id;
        (channel_id, closing, message)
    }

    fn edit_to(
        segment: Id<Segment>,
        message: Id<Message>,
        text: &str,
        now: Instant<Authored>,
    ) -> MurmureCommand {
        MurmureCommand::EditMessage {
            at: segment.into(),
            message,
            draft: MessageDraft {
                text: text.to_owned(),
                mentions: Vec::new(),
                styles: Vec::new(),
            },
            now,
        }
    }

    /// The amendments document Logic asked the Sync worker to create.
    fn created_amendments(sync: &Receiver<SyncCommand>) -> Vec<(Id<Segment>, Id<Document>)> {
        drain(sync)
            .into_iter()
            .filter_map(|c| match c {
                SyncCommand::CreateAmendments {
                    segment,
                    amendments,
                    ..
                } => Some((segment, amendments)),
                _ => None,
            })
            .collect()
    }

    /// An edit of a locked segment waits for the amendments document it
    /// creates. The link leaves as a `meta` delta and the document opens.
    /// The edit leaves once the document went up whole (the Murmure messages
    /// spec).
    #[test]
    fn an_edit_of_a_locked_segment_waits_for_the_amendments_it_creates() {
        let (mut state, ui, sync) = harness();
        let (channel_id, closing, message) = locked_channel(&mut state, &sync);
        drain(&ui);
        command(
            &mut state,
            channel_id,
            edit_to(closing, message, "two", at(1)),
        );
        let [(segment, amendments)] = created_amendments(&sync).try_into().expect("one create");
        assert_eq!(segment, closing);

        state.handle_logic_event(LogicEvent::AmendmentsLinked {
            channel_id,
            segment: closing,
            amendments,
        });
        let meta = state.rt().channels()[&channel_id].meta_id().document_id();
        let sent = drain(&sync);
        assert!(sent.iter().any(|c| matches!(c,
            SyncCommand::SendUpdate { document_id, .. } if *document_id == meta)));
        assert!(sent.iter().any(|c| matches!(c,
            SyncCommand::OpenDocument { document_id, .. } if *document_id == amendments)));
        assert!(
            !sent.iter().any(|c| matches!(c,
                SyncCommand::SendUpdate { document_id, .. } if *document_id == amendments)),
            "the edit waits for the document"
        );

        state.grant_identity(amendments);
        state.handle_logic_event(LogicEvent::DocumentReady {
            document_id: amendments,
            role: ambre_bus::Role::Owner,
        });
        let updates: Vec<bool> = drain(&sync)
            .into_iter()
            .filter_map(|c| match c {
                SyncCommand::SendUpdate {
                    document_id,
                    summary,
                    ..
                } if document_id == amendments => Some(summary.is_some()),
                _ => None,
            })
            .collect();
        assert_eq!(updates, [false, true], "the whole document, then the edit");
        let shown = channel_events(&ui);
        assert!(
            shown.iter().any(|(_, event)| matches!(event,
                MurmureEvent::Messages { segment, messages }
                    if *segment == closing && messages[0].text == "two" && messages[0].edited == ambre_murmure::message::Revision::Edited)),
            "the view was told {shown:?}"
        );
    }

    /// A first reply to a root of a locked segment waits for the amendments
    /// it creates, then for its thread. The link leaves under the amendments,
    /// never under the locked segment (the Murmure threads spec).
    #[test]
    fn a_reply_to_a_locked_root_waits_for_its_amendments() {
        let (mut state, ui, sync) = harness();
        let (channel_id, closing, root) = locked_channel(&mut state, &sync);
        drain(&ui);
        command(&mut state, channel_id, reply_to(root, "late", false));
        let [(segment, amendments)] = created_amendments(&sync).try_into().expect("one create");
        assert_eq!(segment, closing);
        assert_eq!(
            state.rt().murmure_waiting().get(&amendments).map(Vec::len),
            Some(1)
        );
        state.handle_logic_event(LogicEvent::AmendmentsLinked {
            channel_id,
            segment: closing,
            amendments,
        });
        drain(&sync);
        state.grant_identity(amendments);
        state.handle_logic_event(LogicEvent::DocumentReady {
            document_id: amendments,
            role: ambre_bus::Role::Owner,
        });
        let [(segment, created, thread)] =
            created_threads(&sync).try_into().expect("then the thread");
        assert_eq!((segment, created), (closing, root));

        state.handle_logic_event(LogicEvent::ThreadLinked {
            channel_id,
            segment,
            root,
            thread,
        });
        // The root names its thread once the thread went up.
        state.grant_identity(thread);
        state.handle_logic_event(LogicEvent::DocumentReady {
            document_id: thread,
            role: ambre_bus::Role::Owner,
        });
        let sent = drain(&sync);
        assert!(sent.iter().any(|c| matches!(c,
            SyncCommand::SendUpdate { document_id, summary: None, .. }
                if *document_id == amendments)));
        assert!(
            !sent.iter().any(|c| matches!(c,
                SyncCommand::SendUpdate { document_id, .. }
                    if *document_id == closing.document_id())),
            "the locked segment takes no delta"
        );
    }

    /// A link that lost its race names another member's document. What
    /// waited on this client's own waits on the winner, which opens. A
    /// failed link drops this client's own, so the next edit creates again.
    #[test]
    fn a_lost_or_failed_link_moves_the_edit_to_another_document() {
        let (mut state, _ui, sync) = harness();
        let (channel_id, closing, message) = locked_channel(&mut state, &sync);
        command(
            &mut state,
            channel_id,
            edit_to(closing, message, "two", at(1)),
        );
        let [(_, mine)] = created_amendments(&sync).try_into().expect("one create");
        let winner = Id::<Document>::mint();
        state.handle_logic_event(LogicEvent::AmendmentsLinked {
            channel_id,
            segment: closing,
            amendments: winner,
        });
        assert!(drain(&sync).iter().any(|c| matches!(c,
            SyncCommand::OpenDocument { document_id, .. } if *document_id == winner)));
        assert!(!state.rt().murmure_waiting().contains_key(&mine));
        assert_eq!(
            state.rt().murmure_waiting().get(&winner).map(Vec::len),
            Some(1)
        );

        let (channel_id, closing, message) = locked_channel(&mut state, &sync);
        command(
            &mut state,
            channel_id,
            edit_to(closing, message, "two", at(1)),
        );
        let [(_, first)] = created_amendments(&sync).try_into().expect("one create");
        state.handle_logic_event(LogicEvent::AmendmentsLinkFailed {
            channel_id,
            segment: closing,
            amendments: first,
            failure: ambre_bus::ApiFailure::NoAnswer,
        });
        assert!(
            !state.rt().murmure_waiting().contains_key(&first),
            "the edit is dropped"
        );
        command(
            &mut state,
            channel_id,
            edit_to(closing, message, "three", at(2)),
        );
        let [(_, second)] = created_amendments(&sync)
            .try_into()
            .expect("a create again");
        assert_ne!(second, first);
    }

    // --- The fold of the amendments of a final segment ---

    /// The fold commands Logic sent: a lease request names no reach.
    fn folds(
        sync: &Receiver<SyncCommand>,
    ) -> Vec<(Id<Segment>, Option<ambre_core::Seq<ambre_core::Relay>>)> {
        drain(sync)
            .into_iter()
            .filter_map(|c| match c {
                SyncCommand::TakeFoldLease { segment, .. } => Some((segment, None)),
                SyncCommand::FoldSegment {
                    segment,
                    amendments_seq,
                    ..
                } => Some((segment, Some(amendments_seq))),
                _ => None,
            })
            .collect()
    }

    /// A final segment whose one message this client edited into amendments
    /// that went up whole. Returns the segment, the message and the
    /// amendments.
    fn amended_channel(
        state: &mut LogicState,
        sync: &Receiver<SyncCommand>,
    ) -> (Id<Channel>, Id<Segment>, Id<Message>, Id<Document>) {
        let (channel_id, closing) = closed_channel(state, sync);
        state.route(|m| {
            m.record_channel_seq(
                closing.document_id(),
                ambre_core::Seq::<ambre_core::Relay>::new(1),
                None,
            )
        });
        finalize_at_one(state, sync, channel_id, closing, NOW);
        drain(sync);
        let message = state.rt().channels()[&channel_id]
            .messages(closing)
            .expect("held")[0]
            .id;
        command(state, channel_id, edit_to(closing, message, "two", at(1)));
        let [(_, amendments)] = created_amendments(sync).try_into().expect("one create");
        state.handle_logic_event(LogicEvent::AmendmentsLinked {
            channel_id,
            segment: closing,
            amendments,
        });
        state.grant_identity(amendments);
        state.handle_logic_event(LogicEvent::DocumentReady {
            document_id: amendments,
            role: ambre_bus::Role::Owner,
        });
        drain(sync);
        (channel_id, closing, message, amendments)
    }

    /// The fold of the Murmure segments spec, in Logic. The tick asks for
    /// the lease, the granted lease sends the fold snapshot, and a commit
    /// waits out the fold delay. A failure asks again past its retry delay.
    #[test]
    fn a_final_segment_with_amendments_asks_and_commits_its_fold() {
        let (mut state, _ui, sync) = harness();
        let (channel_id, segment, _, amendments) = amended_channel(&mut state, &sync);
        command(&mut state, channel_id, MurmureCommand::Tick { now: at(2) });
        assert_eq!(folds(&sync), [(segment, None)], "the lease is asked");
        state.handle_logic_event(LogicEvent::SegmentFoldFailed {
            channel_id,
            segment,
            failure: ambre_bus::ApiFailure::NoAnswer,
        });
        command(&mut state, channel_id, MurmureCommand::Tick { now: at(3) });
        command(
            &mut state,
            channel_id,
            MurmureCommand::Tick { now: at(2_002) },
        );
        assert!(folds(&sync).is_empty(), "the retry delay runs");
        command(
            &mut state,
            channel_id,
            MurmureCommand::Tick { now: at(2_003) },
        );
        assert_eq!(folds(&sync), [(segment, None)], "the lease is asked again");

        state.handle_logic_event(LogicEvent::FoldLeaseAnswered {
            channel_id,
            segment,
            answer: FoldAnswer::Granted {
                lease: Id::<Lease>::mint(),
                horizon: ambre_core::Seq::<ambre_core::Relay>::new(1),
                amendments,
                amendments_seq: ambre_core::Seq::<ambre_core::Relay>::new(0),
                expires_in: TimeSpan::<Ms>::new(30_000),
            },
        });
        command(
            &mut state,
            channel_id,
            MurmureCommand::Tick { now: at(2_004) },
        );
        assert_eq!(
            folds(&sync),
            [(segment, Some(ambre_core::Seq::<ambre_core::Relay>::new(0)))],
            "the fold leaves"
        );
        state.handle_logic_event(LogicEvent::SegmentFolded {
            channel_id,
            segment,
        });
        command(
            &mut state,
            channel_id,
            MurmureCommand::Tick { now: at(2_005) },
        );
        assert!(folds(&sync).is_empty(), "the fold delay runs");
    }

    /// A confirmation of this client settled the fold. `meta` marks the
    /// amendments folded, the document closes, and the view reads the
    /// segment as folded. The edit the relay never sequenced is written
    /// again at the next tick (the Murmure segments spec).
    #[test]
    fn a_settled_fold_marks_meta_closes_the_amendments_and_replays_the_rest() {
        let (mut state, ui, sync) = harness();
        let (channel_id, segment, _, amendments) = amended_channel(&mut state, &sync);
        drain(&ui);
        state.handle_logic_event(LogicEvent::AmendmentsFolded {
            document_id: segment.document_id(),
            amendments,
        });
        let meta = state.rt().channels()[&channel_id].meta_id().document_id();
        let sent = drain(&sync);
        assert!(sent.iter().any(|c| matches!(c,
            SyncCommand::SendUpdate { document_id, .. } if *document_id == meta)));
        assert!(sent.iter().any(|c| matches!(c,
            SyncCommand::CloseDocument { document_id } if *document_id == amendments)));
        let shown = channel_events(&ui);
        assert!(
            shown.iter().any(|(_, event)| matches!(event,
                MurmureEvent::Messages { segment: s, messages }
                    if *s == segment && messages[0].text == "one" && messages[0].edited == ambre_murmure::message::Revision::Original)),
            "the view reads the fold {shown:?}"
        );
        state.handle_logic_event(LogicEvent::AmendmentsFolded {
            document_id: segment.document_id(),
            amendments,
        });
        assert!(
            !drain(&sync).iter().any(|c| matches!(c,
                SyncCommand::SendUpdate { document_id, .. } if *document_id == meta)),
            "a fold marked already sends no delta"
        );

        command(&mut state, channel_id, MurmureCommand::Tick { now: at(3) });
        let [(again, other)] = created_amendments(&sync).try_into().expect("a replay");
        assert_eq!(again, segment);
        assert_ne!(other, amendments, "the replay goes to new amendments");
    }

    /// Amendments the relay reports gone settle as a marked fold. `meta`
    /// marks them and the document closes. The edit it never sequenced is
    /// written again at the next tick. A gone document no channel holds as
    /// amendments changes nothing (the Murmure segments spec).
    #[test]
    fn amendments_reported_gone_settle_as_a_marked_fold() {
        let (mut state, _ui, sync) = harness();
        let (channel_id, segment, _, amendments) = amended_channel(&mut state, &sync);
        state.handle_logic_event(LogicEvent::DocumentGone {
            document_id: Id::<Document>::mint(),
        });
        state.handle_logic_event(LogicEvent::DocumentGone {
            document_id: segment.document_id(),
        });
        assert!(drain(&sync).is_empty(), "nothing to settle");

        state.handle_logic_event(LogicEvent::DocumentGone {
            document_id: amendments,
        });
        let meta = state.rt().channels()[&channel_id].meta_id().document_id();
        let sent = drain(&sync);
        assert!(sent.iter().any(|c| matches!(c,
            SyncCommand::SendUpdate { document_id, .. } if *document_id == meta)));
        assert!(sent.iter().any(|c| matches!(c,
            SyncCommand::CloseDocument { document_id } if *document_id == amendments)));
        state.handle_logic_event(LogicEvent::DocumentGone {
            document_id: amendments,
        });
        assert!(drain(&sync).is_empty(), "settled once");

        command(&mut state, channel_id, MurmureCommand::Tick { now: at(3) });
        let [(again, other)] = created_amendments(&sync).try_into().expect("a replay");
        assert_eq!(again, segment);
        assert_ne!(other, amendments, "the replay goes to new amendments");
    }

    /// An edit the lock refused is replayed as an amendment at the
    /// channel's next tick. The view reads the segment as the relay holds
    /// it until then (the Murmure messages spec).
    #[test]
    fn an_edit_refused_by_the_lock_is_replayed_at_the_next_tick() {
        let (mut state, ui, sync) = harness();
        let (channel_id, closing) = closed_channel(&mut state, &sync);
        let active = state.rt().channels()[&channel_id].active_id();
        state.grant_identity(active.document_id());
        let message = state.rt().channels()[&channel_id]
            .messages(closing)
            .expect("held")[0]
            .id;
        drain(&sync);
        command(
            &mut state,
            channel_id,
            edit_to(closing, message, "two", at(1)),
        );
        let refused: Vec<ambre_core::Payload<ambre_core::DeltaBytes>> = drain(&sync)
            .into_iter()
            .filter_map(|c| match c {
                SyncCommand::SendUpdate {
                    document_id,
                    payload,
                    ..
                } if document_id == closing.document_id() => Some(payload),
                _ => None,
            })
            .collect();
        assert_eq!(refused.len(), 1, "the edit left in the segment");
        drain(&ui);

        state.handle_logic_event(LogicEvent::DeltasLocked {
            document_id: closing.document_id(),
            deltas: refused,
        });
        let shown = channel_events(&ui);
        assert!(
            shown.iter().any(|(_, event)| matches!(event,
                MurmureEvent::Messages { segment, messages }
                    if *segment == closing && messages[0].text == "one")),
            "the view was told {shown:?}"
        );
        assert!(
            created_amendments(&sync).is_empty(),
            "the replay waits for a tick"
        );
        command(&mut state, channel_id, MurmureCommand::Tick { now: at(2) });
        let [(segment, _)] = created_amendments(&sync).try_into().expect("the replay");
        assert_eq!(segment, closing);
    }

    /// The verdict Logic answers a pending snapshot of `document_id` with.
    fn channel_verdict(
        state: &mut LogicState,
        sync: &Receiver<SyncCommand>,
        document_id: Id<Document>,
        covered: Seq<Relay>,
        payload: Vec<u8>,
    ) -> SnapshotVerdict {
        state.handle_logic_event(LogicEvent::PendingSnapshot {
            document_id,
            snapshot_id: Id::<Snapshot>::genesis(document_id),
            covered_seq: covered,
            payload: payload.into(),
            purpose: ambre_bus::SnapshotPurpose::Confirm,
        });
        drain(sync)
            .into_iter()
            .find_map(|c| match c {
                SyncCommand::SnapshotVerdict { verdict, .. } => Some(verdict),
                _ => None,
            })
            .expect("a pending snapshot of a segment is answered")
    }

    /// A snapshot of a segment is checked against the state this client
    /// holds (the Murmure segments spec). A gap below its horizon leaves it
    /// unchecked, and bytes no state reads are a mismatch.
    #[test]
    fn a_segment_snapshot_is_checked_against_the_held_state() {
        let (mut state, _ui, sync) = harness();
        let (channel_id, segment) = small_channel(&mut state, 4);
        command(&mut state, channel_id, send("one", Vec::new()));
        drain(&sync);
        let document_id = segment.document_id();
        let (payload, covered) = state
            .route(|m| m.channel_snapshot(document_id))
            .expect("held");
        assert!(matches!(
            channel_verdict(
                &mut state,
                &sync,
                document_id,
                covered,
                payload.clone().into_bytes()
            ),
            SnapshotVerdict::Match { .. }
        ));
        assert_eq!(
            channel_verdict(&mut state, &sync, document_id, covered, b"loro?".to_vec()),
            SnapshotVerdict::Mismatch
        );
        state.route(|m| {
            m.record_channel_seq(
                document_id,
                ambre_core::Seq::<ambre_core::Relay>::new(2),
                None,
            )
        });
        assert_eq!(
            channel_verdict(
                &mut state,
                &sync,
                document_id,
                ambre_core::Seq::<ambre_core::Relay>::new(2),
                payload.into_bytes()
            ),
            SnapshotVerdict::Unchecked,
            "the delta numbered 1 is missing"
        );
    }

    /// A copy the Sync worker withdrew never asks again; a failed commit
    /// asks again past its delay.
    #[test]
    fn a_withdrawn_copy_stops_and_a_failed_one_asks_again() {
        let (mut state, _ui, sync) = harness();
        let (channel_id, closing) = closed_channel(&mut state, &sync);
        command(&mut state, channel_id, MurmureCommand::Tick { now: NOW });
        drain(&sync);
        state.handle_logic_event(LogicEvent::SegmentFinalizeFailed {
            channel_id,
            segment: closing,
            end: ambre_core::FinalizeEnd::Failed,
            failure: ambre_bus::ApiFailure::NoAnswer,
        });
        command(&mut state, channel_id, MurmureCommand::Tick { now: at(1) });
        command(
            &mut state,
            channel_id,
            MurmureCommand::Tick {
                now: at(1 + ambre_murmure::FINALIZE_RETRY.get()),
            },
        );
        assert_eq!(finalizes(&sync), [(closing, None)], "asked again");

        state.handle_logic_event(LogicEvent::SegmentFinalizeFailed {
            channel_id,
            segment: closing,
            end: ambre_core::FinalizeEnd::Withdrawn,
            failure: ambre_bus::ApiFailure::NoAnswer,
        });
        command(
            &mut state,
            channel_id,
            MurmureCommand::Tick { now: at(1 << 40) },
        );
        assert!(finalizes(&sync).is_empty(), "a withdrawn copy never asks");
    }

    /// A close another member wrote reaches this client in `meta`, which
    /// fetches the successor it names (the Murmure segments spec).
    #[test]
    fn a_remote_close_fetches_its_successor() {
        let (mut state, _ui, sync) = harness();
        let (channel_id, old) = small_channel(&mut state, 1);
        let channel = &state.rt().channels()[&channel_id];
        let meta = channel.meta_id();
        let segments = HashMap::from([(old, channel.snapshot(old.document_id()).expect("old"))]);
        let meta_bytes = channel.snapshot(meta.document_id()).expect("meta");
        let mut bob = crate::identified(
            ChannelState::open(channel_id, meta, &meta_bytes, &segments).expect("bob"),
        );
        let successor = Id::<Segment>::mint();
        let closed = bob
            .close_into(old, successor, ambre_murmure::Successor::Created, NOW)
            .expect("bob closes");
        let closed_meta = closed.meta;
        drain(&sync);

        assert!(state.route(|m| m.apply_channel_remote(
            meta.document_id(),
            &[(None, ambre_core::LoroPayloadRef::Delta(&closed_meta))]
        )));
        assert!(
            sent(&sync).contains(&Sent::Open(successor.document_id())),
            "the successor is fetched"
        );
        assert!(state.route(|m| m.apply_channel_remote(
            meta.document_id(),
            &[(None, ambre_core::LoroPayloadRef::Delta(&closed_meta))]
        )));
        assert!(
            !sent(&sync).contains(&Sent::Open(successor.document_id())),
            "fetched once"
        );
    }

    /// A successor whose open failed is fetched again by a tick past its
    /// delay (the Murmure segments spec).
    #[test]
    fn a_tick_fetches_again_a_successor_that_failed_to_open() {
        let (mut state, _ui, sync) = harness();
        let (channel_id, old) = small_channel(&mut state, 1);
        let channel = &state.rt().channels()[&channel_id];
        let meta = channel.meta_id();
        let segments = HashMap::from([(old, channel.snapshot(old.document_id()).expect("old"))]);
        let meta_bytes = channel.snapshot(meta.document_id()).expect("meta");
        let mut bob = crate::identified(
            ChannelState::open(channel_id, meta, &meta_bytes, &segments).expect("bob"),
        );
        let successor = Id::<Segment>::mint();
        let closed = bob
            .close_into(old, successor, ambre_murmure::Successor::Created, NOW)
            .expect("bob closes");
        let closed_meta = closed.meta;
        assert!(state.route(|m| m.apply_channel_remote(
            meta.document_id(),
            &[(None, ambre_core::LoroPayloadRef::Delta(&closed_meta))]
        )));
        drain(&sync);

        state.handle_logic_event(LogicEvent::DocumentOpenFailed {
            document_id: successor.document_id(),
            cause: OpenFailure::KeyNotReceived,
        });
        let tick = |ms: u64| MurmureCommand::Tick {
            now: Instant::<Authored>::new(NOW.get() + ms),
        };
        command(&mut state, channel_id, tick(0));
        command(&mut state, channel_id, tick(999));
        assert!(
            !sent(&sync).contains(&Sent::Open(successor.document_id())),
            "not before its delay"
        );
        command(&mut state, channel_id, tick(1_000));
        assert!(
            sent(&sync).contains(&Sent::Open(successor.document_id())),
            "fetched again"
        );
    }

    /// A closed channel whose `meta` moves is listed again, and each segment
    /// it did not hold is watched once and counts (the Murmure unread spec).
    #[test]
    fn a_closed_channel_watches_a_segment_opened_since() {
        let (mut state, ui, sync) = harness();
        let (channel_id, active, meta) = closed_counted(&mut state);
        drain(&sync);
        state.handle_logic_event(LogicEvent::Activity {
            document_id: meta.document_id(),
            counters: activity(0),
        });
        assert_eq!(sent(&sync), [Sent::List(channel_id)]);

        let successor = Id::<Segment>::mint();
        let listing = || LogicEvent::ChannelDocuments {
            channel_id,
            documents: vec![
                ChannelDocument {
                    document_id: meta.document_id(),
                    kind: ChannelDocumentKind::Meta,
                },
                ChannelDocument {
                    document_id: active.document_id(),
                    kind: ChannelDocumentKind::Segment,
                },
                ChannelDocument {
                    document_id: successor.document_id(),
                    kind: ChannelDocumentKind::Segment,
                },
            ],
        };
        state.handle_logic_event(listing());
        let watched: Vec<SyncCommand> = drain(&sync);
        assert!(
            matches!(
                watched.as_slice(),
                [SyncCommand::WatchSegment { segment, meta: m }] if *segment == successor && *m == meta
            ),
            "one watch for the new segment, got {watched:?}"
        );
        state.handle_logic_event(listing());
        assert!(drain(&sync).is_empty(), "a segment is watched once");
        assert!(
            !state.rt().channels().contains_key(&channel_id),
            "the channel stays closed"
        );

        drain(&ui);
        state.handle_logic_event(LogicEvent::Activity {
            document_id: successor.document_id(),
            counters: activity(2),
        });
        assert!(drain(&ui).iter().any(|event| matches!(
            event,
            UiEvent::ChannelActivity { channels }
                if channels.first().is_some_and(|mark| mark.unread == Some(ambre_core::Count::<Unread>::new(2)))
        )));
    }

    // --- The threads of a channel ---

    fn reply_to(root: Id<Message>, text: &str, also_in_channel: bool) -> MurmureCommand {
        MurmureCommand::Reply {
            root,
            draft: MessageDraft {
                text: text.to_owned(),
                mentions: Vec::new(),
                styles: Vec::new(),
            },
            also_in_channel: if also_in_channel {
                ambre_murmure::message::ReplyEcho::AlsoInChannel
            } else {
                ambre_murmure::message::ReplyEcho::ThreadOnly
            },
            now: at(1),
            confirmed: ambre_murmure::message::Confirmation::Unconfirmed,
        }
    }

    /// The thread documents Logic asked the Sync worker to create.
    fn created_threads(
        sync: &Receiver<SyncCommand>,
    ) -> Vec<(Id<Segment>, Id<Message>, Id<Document>)> {
        drain(sync)
            .into_iter()
            .filter_map(|c| match c {
                SyncCommand::CreateThread {
                    segment,
                    root,
                    thread,
                    ..
                } => Some((segment, root, thread)),
                _ => None,
            })
            .collect()
    }

    /// An open channel whose active segment holds one message, sent by a
    /// signed-in user. Returns its root.
    fn rooted_channel(
        state: &mut LogicState,
        sync: &Receiver<SyncCommand>,
    ) -> (Id<Channel>, Id<Segment>, Id<Message>) {
        let (channel_id, active) = open_channel(state);
        command(state, channel_id, send("root", Vec::new()));
        drain(sync);
        let root = state.rt().channels()[&channel_id]
            .messages(active)
            .expect("held")[0]
            .id;
        (channel_id, active, root)
    }

    /// A first reply waits for the thread it creates. The link names the
    /// thread in the root and opens it. The reply leaves once the thread
    /// went up whole, and its copy leaves under the active segment.
    #[test]
    fn a_first_reply_waits_for_the_thread_it_creates() {
        let (mut state, ui, sync) = harness();
        signed_in(&mut state, Id::<User>::mint());
        let (channel_id, active, root) = rooted_channel(&mut state, &sync);
        drain(&ui);
        command(&mut state, channel_id, reply_to(root, "a reply", true));
        let [(segment, created, thread)] = created_threads(&sync).try_into().expect("one create");
        assert_eq!((segment, created), (active, root));
        assert_eq!(
            state.rt().murmure_waiting().get(&thread).map(Vec::len),
            Some(1)
        );

        state.handle_logic_event(LogicEvent::ThreadLinked {
            channel_id,
            segment,
            root,
            thread,
        });
        let sent = drain(&sync);
        assert!(sent.iter().any(|c| matches!(c,
            SyncCommand::OpenDocument { document_id, .. } if *document_id == thread)));
        assert!(
            !sent
                .iter()
                .any(|c| matches!(c, SyncCommand::SendUpdate { .. })),
            "the reply and the link wait for the thread, {sent:?}"
        );

        state.grant_identity(thread);
        state.handle_logic_event(LogicEvent::DocumentReady {
            document_id: thread,
            role: ambre_bus::Role::Owner,
        });
        let updates: Vec<(Id<Document>, bool)> = drain(&sync)
            .into_iter()
            .filter_map(|c| match c {
                SyncCommand::SendUpdate {
                    document_id,
                    summary,
                    ..
                } => Some((document_id, summary.is_some())),
                _ => None,
            })
            .collect();
        assert_eq!(
            updates,
            [
                (thread, false),
                (thread, true),
                (active.document_id(), true),
                (active.document_id(), false)
            ],
            "the whole thread, the reply, its copy, then the link in the root"
        );
        let shown = channel_events(&ui);
        assert!(
            shown.iter().any(|(_, event)| matches!(event,
                MurmureEvent::ThreadMessages { root: r, thread: t, messages }
                    if *r == root && *t == thread && messages[0].text == "a reply")),
            "the view was told {shown:?}"
        );
        assert!(
            shown.iter().any(|(_, event)| matches!(event,
                MurmureEvent::Messages { segment, messages }
                    if *segment == active
                        && messages[0].thread.is_some_and(|head| head.replies.get() == 1))),
            "the root's head counts the reply, {shown:?}"
        );
    }

    /// A member who reads the root's link opens a thread the relay holds
    /// whole (the Murmure threads spec). So the root names this client's own thread only
    /// once its first update left, and a thread lost before that names
    /// nothing.
    #[test]
    fn a_root_names_its_own_thread_once_the_thread_went_up() {
        let (mut state, _ui, sync) = harness();
        signed_in(&mut state, Id::<User>::mint());
        let (channel_id, active, root) = rooted_channel(&mut state, &sync);
        command(&mut state, channel_id, reply_to(root, "a reply", false));
        let [(segment, _, thread)] = created_threads(&sync).try_into().expect("one create");
        state.handle_logic_event(LogicEvent::ThreadLinked {
            channel_id,
            segment,
            root,
            thread,
        });
        state.grant_identity(thread);
        let early = drain(&sync);
        assert!(
            !early
                .iter()
                .any(|c| matches!(c, SyncCommand::SendUpdate { .. })),
            "nothing leaves before the thread does, {early:?}"
        );

        state.handle_logic_event(LogicEvent::DocumentReady {
            document_id: thread,
            role: ambre_bus::Role::Owner,
        });
        let sent: Vec<Id<Document>> = drain(&sync)
            .into_iter()
            .filter_map(|c| match c {
                SyncCommand::SendUpdate { document_id, .. } => Some(document_id),
                _ => None,
            })
            .collect();
        assert_eq!(sent, [thread, thread, active.document_id()]);
        let named = state.rt().channels()[&channel_id]
            .messages(active)
            .expect("held")[0]
            .thread
            .is_some();
        assert!(named, "the root names its thread");

        // A thread whose session ends before it went up leaves the root
        // unnamed: the next reply registers it again.
        let (channel_id, _, root) = rooted_channel(&mut state, &sync);
        command(&mut state, channel_id, reply_to(root, "lost", false));
        let [(segment, _, thread)] = created_threads(&sync).try_into().expect("one create");
        state.handle_logic_event(LogicEvent::ThreadLinked {
            channel_id,
            segment,
            root,
            thread,
        });
        state.route(|m| m.forget_murmure_session(thread));
        state.route(|m| m.name_root(thread));
        assert!(
            !drain(&sync)
                .iter()
                .any(|c| matches!(c, SyncCommand::SendUpdate { .. })),
            "a forgotten thread names no root"
        );
    }

    /// A change on the reply of `root`, in its thread: a react, an edit or a
    /// delete by `kind`.
    fn thread_change(root: Id<Message>, reply: Id<Message>, kind: &str) -> MurmureCommand {
        let at = MessageAt::Thread(root);
        match kind {
            "react" => MurmureCommand::React {
                at,
                message: reply,
                emoji: ambre_bus::Emoji::from("\u{1F44D}"),
                now: NOW,
            },
            "unreact" => MurmureCommand::Unreact {
                at,
                message: reply,
                emoji: ambre_bus::Emoji::from("\u{1F44D}"),
            },
            "edit" => MurmureCommand::EditMessage {
                at,
                message: reply,
                draft: MessageDraft {
                    text: "edited".to_owned(),
                    mentions: Vec::new(),
                    styles: Vec::new(),
                },
                now: NOW,
            },
            _ => MurmureCommand::DeleteMessage {
                at,
                message: reply,
                now: NOW,
            },
        }
    }

    /// Each change on a reply leaves for its thread, and the view reads the
    /// replies. A delete moves the root's count too. A thread not held takes
    /// no change.
    #[test]
    fn a_change_on_a_reply_leaves_for_its_thread() {
        let (mut state, ui, sync) = harness();
        signed_in(&mut state, Id::<User>::mint());
        let (channel_id, active, root) = rooted_channel(&mut state, &sync);
        command(&mut state, channel_id, reply_to(root, "a reply", false));
        let [(segment, _, thread)] = created_threads(&sync).try_into().expect("one create");
        state.handle_logic_event(LogicEvent::ThreadLinked {
            channel_id,
            segment,
            root,
            thread,
        });
        state.grant_identity(thread);
        state.handle_logic_event(LogicEvent::DocumentReady {
            document_id: thread,
            role: ambre_bus::Role::Owner,
        });
        drain(&sync);
        drain(&ui);
        let Some(MurmureEvent::ThreadMessages { messages, .. }) =
            state.rt().channels()[&channel_id].thread_view(thread)
        else {
            panic!("the thread is held");
        };
        let reply = messages[0].id;
        for kind in ["react", "unreact", "edit", "delete"] {
            command(&mut state, channel_id, thread_change(root, reply, kind));
            let sent: Vec<Id<Document>> = drain(&sync)
                .into_iter()
                .filter_map(|c| match c {
                    SyncCommand::SendUpdate { document_id, .. } => Some(document_id),
                    _ => None,
                })
                .collect();
            assert_eq!(sent, [thread], "{kind} writes the thread");
            let shown = channel_events(&ui);
            assert!(
                shown.iter().any(|(_, event)| matches!(event,
                    MurmureEvent::ThreadMessages { thread: t, .. } if *t == thread)),
                "{kind}: the view was told {shown:?}"
            );
            let counted = shown.iter().any(|(_, event)| {
                matches!(event,
                MurmureEvent::Messages { segment, messages }
                    if *segment == active
                        && messages[0].thread.is_some_and(|head| head.replies.get() == 0))
            });
            assert_eq!(counted, kind == "delete", "{kind}: {shown:?}");
        }
        let Some(MurmureEvent::ThreadMessages { messages, .. }) =
            state.rt().channels()[&channel_id].thread_view(thread)
        else {
            panic!("the thread is held");
        };
        assert_eq!(
            messages[0].deleted,
            ambre_murmure::message::Deletion::Deleted
        );
        command(
            &mut state,
            channel_id,
            thread_change(Id::<Message>::mint(), reply, "edit"),
        );
        assert!(drain(&sync).is_empty(), "a thread not held takes nothing");
        assert!(state.rt().murmure_waiting().is_empty());
    }

    /// An edit and a delete of a reply also sent to the channel reach its
    /// copy in the active segment as well. A reaction stays on the reply.
    #[test]
    fn a_copy_follows_the_edit_and_the_delete_of_its_reply() {
        let (mut state, ui, sync) = harness();
        signed_in(&mut state, Id::<User>::mint());
        let (channel_id, active, root) = rooted_channel(&mut state, &sync);
        command(&mut state, channel_id, reply_to(root, "a reply", true));
        let [(segment, _, thread)] = created_threads(&sync).try_into().expect("one create");
        state.handle_logic_event(LogicEvent::ThreadLinked {
            channel_id,
            segment,
            root,
            thread,
        });
        state.grant_identity(thread);
        state.handle_logic_event(LogicEvent::DocumentReady {
            document_id: thread,
            role: ambre_bus::Role::Owner,
        });
        drain(&sync);
        drain(&ui);
        let Some(MurmureEvent::ThreadMessages { messages, .. }) =
            state.rt().channels()[&channel_id].thread_view(thread)
        else {
            panic!("the thread is held");
        };
        let reply = messages[0].id;
        let copy = |state: &LogicState| {
            state.rt().channels()[&channel_id]
                .messages(active)
                .expect("held")[1]
                .clone()
        };
        for (kind, written) in [
            ("react", vec![thread]),
            ("edit", vec![thread, active.document_id()]),
            ("delete", vec![thread, active.document_id()]),
        ] {
            command(&mut state, channel_id, thread_change(root, reply, kind));
            let sent: Vec<Id<Document>> = drain(&sync)
                .into_iter()
                .filter_map(|c| match c {
                    SyncCommand::SendUpdate { document_id, .. } => Some(document_id),
                    _ => None,
                })
                .collect();
            assert_eq!(sent, written, "{kind}");
            if kind == "edit" {
                assert_eq!(copy(&state).text, "edited", "the copy reads the edit");
                assert_eq!(
                    copy(&state).edited,
                    ambre_murmure::message::Revision::Edited
                );
            }
        }
        assert_eq!(
            copy(&state).deleted,
            ambre_murmure::message::Deletion::Deleted,
            "the copy is deleted with its reply"
        );
        assert!(copy(&state).text.is_empty());
        assert!(
            copy(&state).reactions.is_empty(),
            "the reaction stayed on the reply"
        );
    }

    /// A link that lost its race names another member's thread. What waited
    /// on this client's own waits on the winner, which opens. A failed link
    /// drops this client's own, so the next reply creates again.
    #[test]
    fn a_lost_or_failed_thread_link_moves_the_reply_to_another_document() {
        let (mut state, _ui, sync) = harness();
        signed_in(&mut state, Id::<User>::mint());
        let (channel_id, segment, root) = rooted_channel(&mut state, &sync);
        command(&mut state, channel_id, reply_to(root, "two", false));
        let [(_, _, mine)] = created_threads(&sync).try_into().expect("one create");
        let winner = Id::<Document>::mint();
        state.handle_logic_event(LogicEvent::ThreadLinked {
            channel_id,
            segment,
            root,
            thread: winner,
        });
        assert!(drain(&sync).iter().any(|c| matches!(c,
            SyncCommand::OpenDocument { document_id, .. } if *document_id == winner)));
        assert!(!state.rt().murmure_waiting().contains_key(&mine));
        assert_eq!(
            state.rt().murmure_waiting().get(&winner).map(Vec::len),
            Some(1)
        );

        let (channel_id, segment, root) = rooted_channel(&mut state, &sync);
        command(&mut state, channel_id, reply_to(root, "two", false));
        let [(_, _, first)] = created_threads(&sync).try_into().expect("one create");
        state.handle_logic_event(LogicEvent::ThreadLinkFailed {
            channel_id,
            segment,
            root,
            thread: first,
            failure: ambre_bus::ApiFailure::NoAnswer,
        });
        assert!(
            !state.rt().murmure_waiting().contains_key(&first),
            "the reply is dropped"
        );
        command(&mut state, channel_id, reply_to(root, "three", false));
        let [(_, _, second)] = created_threads(&sync).try_into().expect("a create again");
        assert_ne!(second, first);
    }

    /// A root another member threaded names its thread. The member who did
    /// not link it opens the thread, and its view reads the replies once the
    /// bytes arrived.
    #[test]
    fn a_root_another_member_threaded_opens_its_thread() {
        let mut m = members();
        m.deliver("root", 1);
        let root = m.alice.messages(m.active).expect("held")[0].id;
        let ThreadTarget::Create(thread) = m.alice.reply_target(root, NOW).expect("target") else {
            panic!("a root with no thread creates one");
        };
        let peer = crate::test_peer();
        m.alice
            .adopt_session(
                thread,
                peer,
                ambre_core::Payload::<ambre_core::PeerEntryBytes>::new(
                    peer.get().to_le_bytes().to_vec(),
                ),
            )
            .expect("adopt");
        let named = m
            .alice
            .thread_linked(m.active, root, thread)
            .expect("link")
            .root
            .expect("the root names it");
        let draft = MessageDraft {
            text: "a reply".to_owned(),
            mentions: Vec::new(),
            styles: Vec::new(),
        };
        m.alice
            .reply(
                root,
                &draft,
                m.author,
                NOW,
                ambre_murmure::message::ReplyEcho::ThreadOnly,
            )
            .expect("reply");
        let document_id = m.active.document_id();
        m.bob.handle_logic_event(LogicEvent::RemoteUpdate {
            document_id,
            delta: ambre_bus::SignedDelta {
                peer: m.alice.peer(document_id).expect("a session"),
                user: Some(m.author),
                ..crate::remote_delta(2, named.delta)
            },
        });
        assert!(drain(&m.bob_sync).iter().any(|c| matches!(c,
            SyncCommand::OpenDocument { document_id, .. } if *document_id == thread)));

        drain(&m.bob_ui);
        serve(&mut m.bob, &m.alice, thread);
        let shown = channel_events(&m.bob_ui);
        assert!(
            shown.iter().any(|(_, event)| matches!(event,
                MurmureEvent::ThreadMessages { thread: t, messages, .. }
                    if *t == thread && messages[0].text == "a reply")),
            "bob's view was told {shown:?}"
        );
    }

    /// The reply on a copy whose root lies in a final segment not loaded.
    /// The probe is read and closed, the segment loads, and the view is told
    /// the root. A thread the client never reached ends the seek.
    #[test]
    fn the_reply_on_a_far_copy_loads_its_root_then_names_it() {
        let (mut state, ui, sync) = harness();
        let (channel_id, closed) = closed_channel(&mut state, &sync);
        let channel = state
            .rt_mut()
            .channels_mut()
            .get_mut(&channel_id)
            .expect("open");
        let root = channel.messages(closed).expect("held")[0].id;
        let ThreadTarget::Create(thread) = channel.reply_target(root, NOW).expect("target") else {
            panic!("a root with no thread creates one");
        };
        let peer = crate::test_peer();
        channel
            .adopt_session(
                thread,
                peer,
                ambre_core::Payload::<ambre_core::PeerEntryBytes>::new(
                    peer.get().to_le_bytes().to_vec(),
                ),
            )
            .expect("adopt");
        channel.thread_linked(closed, root, thread).expect("link");
        let probe = channel.snapshot(thread).expect("thread");
        state.route(|m| {
            m.record_channel_seq(
                closed.document_id(),
                ambre_core::Seq::<ambre_core::Relay>::new(1),
                None,
            )
        });
        finalize_at_one(&mut state, &sync, channel_id, closed, NOW);
        let channel = &state.rt().channels()[&channel_id];
        let (meta, active) = (channel.meta_id(), channel.active_id());
        let meta_bytes = channel.snapshot(meta.document_id()).expect("meta");
        let closed_bytes = channel.snapshot(closed.document_id()).expect("closed");
        let segments = HashMap::from([(
            active,
            channel.snapshot(active.document_id()).expect("active"),
        )]);
        let reopened = ChannelState::open(channel_id, meta, &meta_bytes, &segments).expect("open");
        state.hold_channel(crate::identified(reopened));
        drain(&sync);
        drain(&ui);
        let deliver =
            |state: &mut LogicState,
             document_id,
             payload: &ambre_core::Payload<ambre_core::SnapshotBytes>| {
                state.handle_logic_event(LogicEvent::RemoteSnapshot {
                    document_id,
                    covered_seq: ambre_core::Seq::<ambre_core::Relay>::new(1),
                    payload: payload.clone(),
                });
                state.handle_logic_event(LogicEvent::DocumentReady {
                    document_id,
                    role: ambre_bus::Role::Editor,
                });
            };

        command(&mut state, channel_id, MurmureCommand::SeekRoot { thread });
        assert_eq!(sent(&sync), [Sent::Open(thread)], "the probe");
        deliver(&mut state, thread, &probe);
        assert_eq!(
            sent(&sync),
            [Sent::Close(thread), Sent::Open(closed.document_id())]
        );
        deliver(&mut state, closed.document_id(), &closed_bytes);
        assert!(
            sent(&sync).contains(&Sent::Open(thread)),
            "the thread opens"
        );
        let found = channel_events(&ui).into_iter().any(|(_, event)| {
            event
                == MurmureEvent::RootFound {
                    thread,
                    segment: closed,
                    root,
                }
        });
        assert!(found, "the view is told the root");

        let unknown = Id::<Document>::mint();
        command(
            &mut state,
            channel_id,
            MurmureCommand::SeekRoot { thread: unknown },
        );
        assert_eq!(sent(&sync), [Sent::Open(unknown)]);
        state.handle_logic_event(LogicEvent::DocumentOpenFailed {
            document_id: unknown,
            cause: OpenFailure::Transient,
        });
        assert!(sent(&sync).is_empty(), "the seek ends");
        let lost = channel_events(&ui)
            .into_iter()
            .filter(|(_, event)| *event == MurmureEvent::RootLost { thread: unknown })
            .count();
        assert_eq!(lost, 1, "the view is told once");
    }

    /// A reply that mentions me in a thread the open channel holds counts
    /// on the channel's mark, as a mention and never as unread.
    #[test]
    fn a_mention_in_a_held_thread_counts_on_its_open_channel() {
        let mut m = members();
        held_mention(&mut m);
        assert_eq!(
            m.bob.route(|r| r.counted(m.channel_id)),
            Some(ChannelActivity {
                channel_id: m.channel_id,
                active: ambre_bus::Activity::Active,
                unread: Some(ambre_core::Count::<Unread>::new(0)),
                mentions: Some(ambre_core::Count::<Mentions>::new(1)),
            })
        );
    }

    /// Marking a held thread read moves its cursor to what this client was
    /// delivered, so its mention stops counting. A thread not held moves
    /// nothing.
    #[test]
    fn marking_a_thread_read_moves_its_cursor_to_its_last_reply() {
        let mut m = members();
        let thread = held_mention(&mut m);
        drain(&m.bob_sync);
        drain(&m.bob_ui);
        let mark = |m: &mut Members, thread| {
            m.bob.handle_logic_command(LogicCommand::Channel {
                channel_id: m.channel_id,
                command: MurmureCommand::MarkThreadRead { thread },
            });
        };
        mark(&mut m, Id::<Document>::mint());
        assert!(drain(&m.bob_sync).is_empty());
        mark(&mut m, thread);
        let asked = drain(&m.bob_sync);
        assert!(
            matches!(asked.as_slice(), [SyncCommand::MarkThreadRead { thread: t }] if *t == thread),
            "Sync was asked {asked:?}"
        );
        assert_eq!(
            m.bob.rt().read_cursors().get(&thread),
            Some(&ambre_murmure::logic::runtime::ReadCursor {
                read: Seq::<Relay>::new(1),
                active: ambre_bus::Activity::Quiet,
            })
        );
        let shown = drain(&m.bob_ui);
        let [
            UiEvent::ChannelActivity { channels },
            UiEvent::Channel { .. },
        ] = shown.as_slice()
        else {
            panic!("expected the channel's mark and its first unread, got {shown:?}");
        };
        assert_eq!(
            channels[0].mentions,
            Some(ambre_core::Count::<Mentions>::new(0))
        );
        assert_eq!(channels[0].active, ambre_bus::Activity::Quiet);
    }

    /// Bob holds a thread of the channel with one reply of Alice's that
    /// mentions him, and has read every segment. Returns the thread.
    fn held_mention(m: &mut Members) -> Id<Document> {
        m.deliver("root", 1);
        let root = m.alice.messages(m.active).expect("held")[0].id;
        let ThreadTarget::Create(thread) = m.alice.reply_target(root, NOW).expect("target") else {
            panic!("a root with no thread creates one");
        };
        let peer = crate::test_peer();
        m.alice
            .adopt_session(
                thread,
                peer,
                ambre_core::Payload::<ambre_core::PeerEntryBytes>::new(
                    peer.get().to_le_bytes().to_vec(),
                ),
            )
            .expect("adopt");
        let named = m
            .alice
            .thread_linked(m.active, root, thread)
            .expect("link")
            .root
            .expect("the root names it");
        let bob = m.bob.my_user_id.expect("signed in");
        let draft = MessageDraft {
            text: "@bob".to_owned(),
            styles: Vec::new(),
            mentions: vec![ambre_murmure::message::Mention {
                target: ambre_murmure::message::MentionTarget::User(bob),
                range: Span::<Char>::from_usize_range(0..4),
            }],
        };
        m.alice
            .reply(
                root,
                &draft,
                m.author,
                NOW,
                ambre_murmure::message::ReplyEcho::ThreadOnly,
            )
            .expect("reply");
        let document_id = m.active.document_id();
        m.bob.handle_logic_event(LogicEvent::RemoteUpdate {
            document_id,
            delta: ambre_bus::SignedDelta {
                peer: m.alice.peer(document_id).expect("a session"),
                user: Some(m.author),
                ..crate::remote_delta(2, named.delta)
            },
        });
        serve(&mut m.bob, &m.alice, thread);
        m.bob.rt_mut().read_cursors_mut().insert(
            document_id,
            ambre_murmure::logic::runtime::ReadCursor {
                read: Seq::<Relay>::new(2),
                active: ambre_bus::Activity::Quiet,
            },
        );
        thread
    }

    // --- A thread not open ---

    /// The relay's activity answer, naming `documents` of `channel_id`.
    fn answer(
        state: &mut LogicState,
        channel_id: Id<Channel>,
        documents: &[(Id<Document>, Seq<Relay>)],
    ) {
        state.handle_logic_event(LogicEvent::ChannelActivity {
            documents: documents
                .iter()
                .map(|&(document_id, read)| DocumentActivity {
                    channel_id,
                    document_id,
                    kind: ChannelDocumentKind::Thread,
                    active: ambre_bus::Activity::Active,
                    read,
                    last: None,
                })
                .collect(),
        });
    }

    fn watched(sync: &Receiver<SyncCommand>) -> Vec<(Id<Document>, Id<Meta>, Seq<Relay>)> {
        drain(sync)
            .into_iter()
            .filter_map(|c| match c {
                SyncCommand::WatchThread {
                    thread,
                    meta,
                    after,
                } => Some((thread, meta, after)),
                SyncCommand::OpenDocument { document_id, .. } => {
                    panic!("a signal loaded {document_id}")
                }
                _ => None,
            })
            .collect()
    }

    /// A signal on a thread not held asks the relay once where it lies. The
    /// answer places it, and it is watched from its read cursor under its
    /// channel's `meta`, with no document loaded (the Murmure threads spec).
    /// A thread the answer does not name is dropped, and a watched thread's
    /// next signal changes nothing.
    #[test]
    fn a_signalled_thread_is_watched_from_its_read_cursor() {
        let (mut state, _ui, sync) = harness();
        signed_in(&mut state, Id::<User>::mint());
        let (channel_id, _) = open_channel(&mut state);
        let meta = state.rt().channels()[&channel_id].meta_id();
        drain(&sync);
        let (thread, gone) = (Id::<Document>::mint(), Id::<Document>::mint());
        for document_id in [thread, gone] {
            state.handle_logic_event(LogicEvent::ThreadActivity { document_id });
        }
        let asked = drain(&sync);
        assert!(
            matches!(asked.as_slice(), [SyncCommand::FetchActivity]),
            "expected one activity request, got {asked:?}"
        );
        answer(&mut state, channel_id, &[(thread, Seq::<Relay>::new(3))]);
        assert_eq!(watched(&sync), [(thread, meta, Seq::<Relay>::new(3))]);
        state.handle_logic_event(LogicEvent::ThreadActivity {
            document_id: thread,
        });
        assert!(drain(&sync).is_empty(), "a watched thread was asked again");

        // A thread the channel already places is watched at once.
        let placed = Id::<Document>::mint();
        answer(&mut state, channel_id, &[(placed, Seq::<Relay>::new(0))]);
        state.handle_logic_event(LogicEvent::ThreadActivity {
            document_id: placed,
        });
        assert_eq!(watched(&sync), [(placed, meta, Seq::<Relay>::new(0))]);
    }

    /// A thread the open channel holds hears its deltas on its own topic.
    /// A thread of a channel this session never opened shows the relay's
    /// mark alone. Neither signal watches anything.
    #[test]
    fn a_signal_on_a_held_thread_or_an_unopened_channel_watches_nothing() {
        let (mut state, _ui, sync) = harness();
        signed_in(&mut state, Id::<User>::mint());
        let (channel_id, active) = open_channel(&mut state);
        let unopened = Id::<Channel>::mint();
        let thread = Id::<Document>::mint();
        answer(&mut state, unopened, &[(thread, Seq::<Relay>::new(0))]);
        drain(&sync);
        for document_id in [active.document_id(), thread] {
            state.handle_logic_event(LogicEvent::ThreadActivity { document_id });
        }
        assert!(drain(&sync).is_empty(), "channel {channel_id} watched");
    }

    /// Holds the mention rule of the Murmure threads spec: a mention in a
    /// watched thread counts on its channel, open or closed. A reply counts
    /// no unread message, and the channel is marked by the mention alone.
    #[test]
    fn a_mention_in_a_watched_thread_counts_on_its_channel() {
        let (mut state, ui, sync) = harness();
        let (closed, _, _) = closed_counted(&mut state);
        let (open, active) = open_channel(&mut state);
        state.rt_mut().read_cursors_mut().insert(
            active.document_id(),
            ambre_murmure::logic::runtime::ReadCursor {
                read: Seq::<Relay>::new(0),
                active: ambre_bus::Activity::Quiet,
            },
        );
        let mut threads = Vec::new();
        for channel_id in [closed, open] {
            let thread = Id::<Document>::mint();
            answer(&mut state, channel_id, &[(thread, Seq::<Relay>::new(0))]);
            state.handle_logic_event(LogicEvent::ThreadActivity {
                document_id: thread,
            });
            threads.push((channel_id, thread));
        }
        assert_eq!(watched(&sync).len(), 2);
        drain(&ui);
        for &(channel_id, thread) in &threads {
            state.handle_logic_event(LogicEvent::Activity {
                document_id: thread,
                counters: ambre_core::activity::ActivityCounters {
                    mentions: ambre_core::Count::<ambre_core::activity::MentionUpdates>::new(1),
                    ..activity(2)
                },
            });
            let shown = drain(&ui);
            let [
                UiEvent::ChannelActivity { channels },
                UiEvent::NewMessage {
                    channel_id: news,
                    mention: ambre_bus::MentionState::Mentioned,
                    ..
                },
            ] = shown.as_slice()
            else {
                panic!("expected the mark of {channel_id}, then a mention, got {shown:?}");
            };
            assert_eq!(*news, channel_id);
            assert_eq!(
                channels,
                &[ChannelActivity {
                    channel_id,
                    active: ambre_bus::Activity::Active,
                    unread: Some(ambre_core::Count::<Unread>::new(0)),
                    mentions: Some(ambre_core::Count::<Mentions>::new(1)),
                }]
            );
        }
    }

    /// A closed channel with no count at close is marked by a mention in
    /// its thread, with no count. The end of the channel ends the light
    /// subscription of each thread it watched.
    #[test]
    fn a_watched_thread_marks_a_closed_channel_and_ends_with_it() {
        let (mut state, ui, sync) = harness();
        signed_in(&mut state, Id::<User>::mint());
        let (channel_id, _) = open_channel(&mut state);
        state.handle_logic_command(LogicCommand::CloseChannel { channel_id });
        let thread = Id::<Document>::mint();
        answer(&mut state, channel_id, &[(thread, Seq::<Relay>::new(0))]);
        state.handle_logic_event(LogicEvent::ThreadActivity {
            document_id: thread,
        });
        drain(&ui);
        drain(&sync);
        state.handle_logic_event(LogicEvent::Activity {
            document_id: thread,
            counters: ambre_core::activity::ActivityCounters {
                mentions: ambre_core::Count::<ambre_core::activity::MentionUpdates>::new(1),
                ..activity(0)
            },
        });
        let shown = drain(&ui);
        let [
            UiEvent::ChannelActivity { channels },
            UiEvent::NewMessage {
                mention: ambre_bus::MentionState::Mentioned,
                ..
            },
        ] = shown.as_slice()
        else {
            panic!("expected the channel's mark, then a mention, got {shown:?}");
        };
        assert_eq!(
            channels,
            &[ChannelActivity {
                channel_id,
                active: ambre_bus::Activity::Active,
                unread: None,
                mentions: None,
            }]
        );
        state.route(|m| m.end_channel(channel_id));
        assert!(
            drain(&sync).iter().any(|c| matches!(c,
                SyncCommand::CloseDocument { document_id } if *document_id == thread)),
            "the watched thread outlived its channel"
        );
        assert!(state.rt().watched_threads().next().is_none());
    }

    // --- The call message of a channel (`ambre-spec-murmure-calls.md`) ---

    /// The calls the Sync worker was asked about, in order. Any other command is dropped.
    fn asked(sync: &Receiver<SyncCommand>) -> Vec<Id<ambre_core::id::Call>> {
        drain(sync)
            .into_iter()
            .filter_map(|cmd| match cmd {
                SyncCommand::AskCallRoster { call, .. } => Some(call),
                _ => None,
            })
            .collect()
    }

    /// The documents an update left for, in order.
    fn updated(sync: &Receiver<SyncCommand>) -> Vec<Id<Document>> {
        drain(sync)
            .into_iter()
            .filter_map(|cmd| match cmd {
                SyncCommand::SendUpdate { document_id, .. } => Some(document_id),
                _ => None,
            })
            .collect()
    }

    /// The Logic half: the start the Sync worker heard posts the
    /// call message in the active segment, and the view shows it. The last leave it heard
    /// writes the end in the same segment.
    #[test]
    fn a_start_and_a_last_leave_heard_write_the_call_message_and_its_end() {
        let (mut state, ui, sync) = harness();
        signed_in(&mut state, Id::<User>::mint());
        let (channel_id, active) = open_channel(&mut state);
        drain(&sync);
        drain(&ui);
        let call = Id::<ambre_core::id::Call>::mint();
        state.handle_logic_event(LogicEvent::CallStarted {
            call,
            channel: channel_id,
            at: NOW,
        });
        assert_eq!(updated(&sync), [active.document_id()]);
        let shown = drain(&ui).into_iter().any(|event| {
            matches!(event, UiEvent::Channel {
                event: MurmureEvent::Messages { ref messages, .. }, ..
            } if messages.iter().any(|m| m.call.as_ref().is_some_and(|v| v.call == call)))
        });
        assert!(shown, "the view shows the call message");
        state.handle_logic_event(LogicEvent::CallEnded {
            call,
            channel: channel_id,
            at: at(1_000),
        });
        assert_eq!(updated(&sync), [active.document_id()]);
        assert!(state.rt().channels()[&channel_id].live_calls().is_empty());
    }

    /// The tick of a shown channel asks who is in each live call, once per
    /// interval at most. A call the relay holds no state of is asked about no more, and a
    /// channel with no live call asks nothing.
    #[test]
    fn a_tick_asks_who_is_in_each_live_call_once_per_interval() {
        let (mut state, _ui, sync) = harness();
        signed_in(&mut state, Id::<User>::mint());
        let (channel_id, _) = open_channel(&mut state);
        command(&mut state, channel_id, MurmureCommand::Tick { now: NOW });
        assert!(asked(&sync).is_empty(), "no call, no ask");
        let (first, second) = (
            Id::<ambre_core::id::Call>::mint(),
            Id::<ambre_core::id::Call>::mint(),
        );
        for call in [first, second] {
            state.handle_logic_event(LogicEvent::CallStarted {
                call,
                channel: channel_id,
                at: NOW,
            });
        }
        let interval = ambre_murmure::logic::call::ROSTER_INTERVAL.get();
        command(&mut state, channel_id, MurmureCommand::Tick { now: at(1) });
        assert_eq!(asked(&sync), [first, second]);
        command(
            &mut state,
            channel_id,
            MurmureCommand::Tick { now: at(1_000) },
        );
        assert!(asked(&sync).is_empty(), "within the interval");
        state.handle_logic_event(LogicEvent::CallRoster {
            call: first,
            channel: channel_id,
            members: ambre_bus::CallMembers::Gone,
        });
        command(
            &mut state,
            channel_id,
            MurmureCommand::Tick {
                now: at(1 + interval),
            },
        );
        assert_eq!(asked(&sync), [second], "a gone call is asked no more");
    }

    /// The last ask of who is in a call is the session's.
    /// After a logout, the next session that shows the same channel asks at
    /// its first tick, within the interval of the previous ask.
    #[test]
    fn a_logout_forgets_when_each_channel_last_asked_who_is_in_its_calls() {
        let (mut state, _ui, sync) = harness();
        signed_in(&mut state, Id::<User>::mint());
        let (channel_id, _) = open_channel(&mut state);
        let call = Id::<ambre_core::id::Call>::mint();
        state.handle_logic_event(LogicEvent::CallStarted {
            call,
            channel: channel_id,
            at: NOW,
        });
        command(&mut state, channel_id, MurmureCommand::Tick { now: at(1) });
        assert_eq!(asked(&sync), [call]);

        log_out(&mut state);
        signed_in(&mut state, Id::<User>::mint());
        state.hold_channel(
            ChannelState::create(channel_id, ChannelType::Public, None, NOW).expect("create"),
        );
        state.handle_logic_event(LogicEvent::CallStarted {
            call,
            channel: channel_id,
            at: NOW,
        });
        drain(&sync);
        command(
            &mut state,
            channel_id,
            MurmureCommand::Tick { now: at(1_000) },
        );
        assert_eq!(asked(&sync), [call], "the next session asks at once");
    }

    /// Who is in a call reaches the view, and no document of
    /// the channel moves.
    #[test]
    fn who_is_in_a_call_reaches_the_view_and_writes_nothing() {
        let (mut state, ui, sync) = harness();
        let me = Id::<User>::mint();
        signed_in(&mut state, me);
        let (channel_id, _) = open_channel(&mut state);
        let call = Id::<ambre_core::id::Call>::mint();
        state.handle_logic_event(LogicEvent::CallStarted {
            call,
            channel: channel_id,
            at: NOW,
        });
        drain(&sync);
        drain(&ui);
        let channel = &state.rt().channels()[&channel_id];
        let before: Vec<_> = channel
            .document_ids()
            .into_iter()
            .map(|id| channel.snapshot(id).expect("held"))
            .collect();
        let members = ambre_bus::CallMembers::Live(vec![me]);
        state.handle_logic_event(LogicEvent::CallRoster {
            call,
            channel: channel_id,
            members: members.clone(),
        });
        assert!(drain(&sync).is_empty(), "nothing leaves for the relay");
        let channel = &state.rt().channels()[&channel_id];
        let after: Vec<_> = channel
            .document_ids()
            .into_iter()
            .map(|id| channel.snapshot(id).expect("held"))
            .collect();
        assert_eq!(before, after, "no document moves");
        assert!(matches!(
            drain(&ui).as_slice(),
            [UiEvent::Channel { event: MurmureEvent::CallMembers { call: c, members: m }, .. }]
                if *c == call && *m == members
        ));
    }
}
