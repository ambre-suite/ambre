// Host tests of the Murmure runtime, driven through the Logic dispatcher.

use super::*;
use ambre_bus::{Authored, Instant, SealedChannelKey};
use ambre_core::{Name, Validated};
use ambre_murmure::message::{PrivateChannelDraft, PublicChannelDraft, SpaceCommand, SpaceEvent};
use ambre_murmure::{ChannelState, ChannelType, Listing, SpaceState};

#[cfg(test)]
mod tests {
    use super::*;
    use ambre_bus::{ChannelTopic, Role};
    use ambre_core::id::Category;

    const NOW: Instant<Authored> = Instant::<Authored>::new(1_700_000_000_000);

    /// The name `text` is, within the cap.
    fn fit_name(text: &str) -> Validated<Name> {
        Validated::<Name>::parse(text).expect("a name within the cap")
    }

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

    fn ready(state: &mut LogicState, document_id: Id<Document>, role: Role) {
        state.handle_logic_event(LogicEvent::DocumentReady { document_id, role });
    }

    fn sealed(byte: u8) -> SealedChannelKey {
        SealedChannelKey::from_bytes(vec![byte; 72])
    }

    fn named(text: &str) -> Validated<Name> {
        Validated::<Name>::parse(text).unwrap()
    }

    fn draft(name: &str) -> PublicChannelDraft {
        PublicChannelDraft {
            channel_id: Id::<Channel>::mint(),
            name: named(name),
            topic: ambre_core::Validated::<ChannelTopic>::parse("the weekly figures").ok(),
            category: None,
            position: ambre_bus::Rank::new(1),
        }
    }

    /// The catalogue of the last catalogue event the view got.
    fn last_catalogue(
        shown: &[UiEvent],
    ) -> Option<(
        Option<Validated<Name>>,
        Vec<ambre_murmure::message::CatalogueEntry>,
    )> {
        shown.iter().rev().find_map(|e| match e {
            UiEvent::Space {
                event: SpaceEvent::Catalogue { name, channels, .. },
                ..
            } => Some((name.clone(), channels.clone())),
            _ => None,
        })
    }

    fn updates_of(sent: &[SyncCommand], document: Id<Document>) -> usize {
        sent.iter()
            .filter(|c| matches!(c, SyncCommand::SendUpdate { document_id, .. } if *document_id == document))
            .count()
    }

    /// A space created here, its document ready, and the channels drained.
    fn created_space(
        state: &mut LogicState,
        ui: &Receiver<UiEvent>,
        sync: &Receiver<SyncCommand>,
    ) -> Id<Space> {
        let space_id = Id::<Space>::mint();
        state.handle_logic_command(LogicCommand::CreateSpace {
            space_id,
            name: Validated::<Name>::parse("Acme").unwrap(),
            now: NOW,
        });
        state.grant_identity(space_id.document_id());
        ready(state, space_id.document_id(), Role::Owner);
        let _ = (drain(ui), drain(sync));
        space_id
    }

    /// The space document opened from `stored`, ready under `role` and under
    /// a session identity, and the channels drained.
    fn opened_space(
        state: &mut LogicState,
        stored: &SpaceState,
        role: Role,
        ui: &Receiver<UiEvent>,
        sync: &Receiver<SyncCommand>,
    ) {
        let space_id = stored.space_id();
        state.handle_logic_command(LogicCommand::OpenSpace { space_id });
        // The Sync worker issues the identity as it opens the document.
        state.grant_identity(space_id.document_id());
        state.handle_logic_event(LogicEvent::RemoteSnapshot {
            document_id: space_id.document_id(),
            covered_seq: ambre_core::Seq::<ambre_core::Relay>::new(1),
            payload: (stored.snapshot().expect("snapshot")),
        });
        ready(state, space_id.document_id(), role);
        let _ = (drain(ui), drain(sync));
    }

    /// `space` under a session identity of its own, as another client
    /// writing it would hold it.
    fn identified_space(mut space: SpaceState) -> SpaceState {
        let peer = crate::test_peer();
        space
            .adopt_session(
                peer,
                ambre_core::Payload::<ambre_core::PeerEntryBytes>::new(
                    peer.get().to_le_bytes().to_vec(),
                ),
            )
            .expect("adopt");
        space
    }

    // --- The client side of a new space ---

    /// A new space shows at once. Its whole document is its first update,
    /// sent once the Sync worker holds the session. That upload makes its
    /// creator the owner on the relay, and marks the document a space.
    #[test]
    fn a_new_space_shows_at_once_and_uploads_whole_once_its_session_is_ready() {
        let (mut state, ui, sync) = harness();
        let space_id = Id::<Space>::mint();
        state.handle_logic_command(LogicCommand::CreateSpace {
            space_id,
            name: Validated::<Name>::parse("Acme").unwrap(),
            now: NOW,
        });
        let sent = drain(&sync);
        assert!(matches!(
            sent.as_slice(),
            [SyncCommand::CreateSpace { space_id: s }] if *s == space_id
        ));
        assert_eq!(
            last_catalogue(&drain(&ui)),
            Some((Some(named("Acme")), Vec::new()))
        );

        let peer = state.grant_identity(space_id.document_id());
        assert_eq!(
            updates_of(&drain(&sync), space_id.document_id()),
            0,
            "nothing leaves before the Sync worker holds the document"
        );
        ready(&mut state, space_id.document_id(), Role::Owner);
        let sent: Vec<SyncCommand> = drain(&sync)
            .into_iter()
            .filter(|c| matches!(c, SyncCommand::SendUpdate { .. }))
            .collect();
        let [
            SyncCommand::SendUpdate {
                document_id,
                payload,
                summary: None,
            },
        ] = sent.as_slice()
        else {
            panic!("one whole upload, got {sent:?}");
        };
        assert_eq!(*document_id, space_id.document_id());
        assert!(
            crate::authored_by(payload, peer),
            "the upload is written under the session's PeerID alone"
        );
        let reopened =
            SpaceState::open(space_id, &[payload.into()]).expect("the upload is the space");
        assert_eq!(
            reopened.name().as_ref().map(ambre_core::Validated::as_str),
            Some("Acme")
        );
        assert!(
            drain(&ui).is_empty(),
            "the space document is no document of the view"
        );
    }

    /// A space the relay holds opens from the payloads that came before its
    /// ready signal.
    #[test]
    fn a_space_opens_from_the_payloads_before_its_ready_signal() {
        let (mut state, ui, sync) = harness();
        let space_id = Id::<Space>::mint();
        let stored = SpaceState::create(space_id, &fit_name("Acme"), NOW).expect("create");
        state.handle_logic_command(LogicCommand::OpenSpace { space_id });
        let _ = drain(&sync);
        state.handle_logic_event(LogicEvent::RemoteSnapshot {
            document_id: space_id.document_id(),
            covered_seq: ambre_core::Seq::<ambre_core::Relay>::new(1),
            payload: (stored.snapshot().expect("snapshot")),
        });
        assert!(
            drain(&ui).is_empty(),
            "nothing shows before the ready signal"
        );
        ready(&mut state, space_id.document_id(), Role::Editor);
        assert_eq!(
            last_catalogue(&drain(&ui)),
            Some((Some(named("Acme")), Vec::new()))
        );
        assert_eq!(
            updates_of(&drain(&sync), space_id.document_id()),
            0,
            "an opened space uploads nothing"
        );
    }

    #[test]
    fn a_space_with_nothing_readable_fails_to_open() {
        let (mut state, ui, sync) = harness();
        let space_id = Id::<Space>::mint();
        state.handle_logic_command(LogicCommand::OpenSpace { space_id });
        let _ = drain(&sync);
        ready(&mut state, space_id.document_id(), Role::Editor);
        assert!(matches!(
            drain(&ui).as_slice(),
            [UiEvent::Space {
                event: SpaceEvent::OpenFailed,
                ..
            }]
        ));
        assert!(matches!(
            drain(&sync).as_slice(),
            [SyncCommand::CloseDocument { .. }]
        ));
        assert!(state.rt().spaces().next().is_none());
    }

    // --- Creating a public channel ---

    /// The relay creates the channel and answers its sealed key. The
    /// catalogue lists it with that key, and its two documents open and go up
    /// whole.
    #[test]
    fn a_public_channel_is_listed_with_its_sealed_key_once_the_relay_holds_it() {
        let (mut state, ui, sync) = harness();
        let space_id = created_space(&mut state, &ui, &sync);
        let draft = draft("budget");
        let channel_id = draft.channel_id;
        state.handle_logic_command(LogicCommand::Space {
            space_id,
            command: SpaceCommand::CreatePublicChannel { draft, now: NOW },
        });
        let sent = drain(&sync);
        let [
            SyncCommand::CreateChannel {
                channel_id: asked,
                space_id: of,
                meta,
                segment,
                visibility: ambre_core::Visibility::Public,
                name: None,
            },
        ] = sent.as_slice()
        else {
            panic!("one public creation, got {sent:?}");
        };
        assert_eq!((*asked, *of), (channel_id, space_id));
        let (meta, segment) = (meta.document_id(), segment.document_id());

        state.handle_logic_event(LogicEvent::ChannelCreated {
            channel_id,
            key: Some(sealed(7)),
        });
        let sent = drain(&sync);
        assert_eq!(
            updates_of(&sent, space_id.document_id()),
            1,
            "one catalogue write"
        );
        let opened: Vec<Id<Document>> = sent
            .iter()
            .filter_map(|c| match c {
                SyncCommand::OpenDocument { document_id, .. } => Some(*document_id),
                _ => None,
            })
            .collect();
        assert_eq!(opened, [meta, segment]);
        let (_, catalogue) = last_catalogue(&drain(&ui)).expect("the catalogue");
        let [entry] = catalogue.as_slice() else {
            panic!("one entry, got {catalogue:?}");
        };
        assert_eq!(
            (
                entry.channel_id,
                entry.name.as_str(),
                entry.topic.as_ref().map(ambre_core::Validated::as_str)
            ),
            (channel_id, "budget", Some("the weekly figures"))
        );
        let space = &state.rt().space(space_id).expect("an open space");
        assert_eq!(space.sealed_key(channel_id), Ok(sealed(7)));

        for document_id in [meta, segment] {
            state.grant_identity(document_id);
            ready(&mut state, document_id, Role::Owner);
        }
        let sent = drain(&sync);
        assert_eq!(
            (updates_of(&sent, meta), updates_of(&sent, segment)),
            (1, 1)
        );
        assert!(state.rt().channels().contains_key(&channel_id));
    }

    /// A viewer changes no catalogue, so its client asks the relay for
    /// nothing. The relay refuses the write in any case.
    #[test]
    fn a_viewer_creates_no_channel_and_adds_no_category() {
        let (mut state, ui, sync) = harness();
        let space_id = Id::<Space>::mint();
        let stored = SpaceState::create(space_id, &fit_name("Acme"), NOW).expect("create");
        state.handle_logic_command(LogicCommand::OpenSpace { space_id });
        state.handle_logic_event(LogicEvent::RemoteSnapshot {
            document_id: space_id.document_id(),
            covered_seq: ambre_core::Seq::<ambre_core::Relay>::new(1),
            payload: (stored.snapshot().expect("snapshot")),
        });
        ready(&mut state, space_id.document_id(), Role::Viewer);
        let _ = (drain(&ui), drain(&sync));

        state.handle_logic_command(LogicCommand::Space {
            space_id,
            command: SpaceCommand::CreatePublicChannel {
                draft: draft("x"),
                now: NOW,
            },
        });
        state.handle_logic_command(LogicCommand::Space {
            space_id,
            command: SpaceCommand::AddCategory {
                category_id: Id::<Category>::mint(),
                name: Validated::<Name>::parse("Teams").unwrap(),
                position: ambre_bus::Rank::FIRST,
            },
        });
        assert!(drain(&sync).is_empty());
        assert!(matches!(
            drain(&ui).as_slice(),
            [UiEvent::Space {
                event: SpaceEvent::ChannelFailed { .. },
                ..
            }]
        ));
        assert!(
            state
                .rt()
                .space(space_id)
                .expect("an open space")
                .catalogue()
                .is_empty()
        );
    }

    #[test]
    fn a_refused_creation_tells_the_view_and_lists_nothing() {
        let (mut state, ui, sync) = harness();
        let space_id = created_space(&mut state, &ui, &sync);
        let draft = draft("budget");
        let channel_id = draft.channel_id;
        state.handle_logic_command(LogicCommand::Space {
            space_id,
            command: SpaceCommand::CreatePublicChannel { draft, now: NOW },
        });
        let _ = drain(&sync);
        state.handle_logic_event(LogicEvent::ChannelRequestFailed {
            channel_id,
            failure: ambre_bus::ApiFailure::NoAnswer,
        });
        assert!(matches!(
            drain(&ui).as_slice(),
            [UiEvent::Space { event: SpaceEvent::ChannelFailed { channel_id: id, .. }, .. }]
                if *id == channel_id
        ));
        assert!(drain(&sync).is_empty());
        assert!(
            state
                .rt()
                .space(space_id)
                .expect("an open space")
                .catalogue()
                .is_empty()
        );
        // A late answer for a creation given up lists nothing either.
        state.handle_logic_event(LogicEvent::ChannelCreated {
            channel_id,
            key: Some(sealed(1)),
        });
        assert!(drain(&sync).is_empty());
    }

    // --- Creating a private channel ---

    /// A viewer of the space creates a private channel. The relay is asked
    /// for a private one, and once it holds it, the two documents open and go
    /// up whole. The catalogue is never written, and lists nothing.
    #[test]
    fn a_private_channel_writes_no_catalogue() {
        let (mut state, ui, sync) = harness();
        let space_id = Id::<Space>::mint();
        let stored = SpaceState::create(space_id, &fit_name("Acme"), NOW).expect("create");
        opened_space(&mut state, &stored, Role::Viewer, &ui, &sync);
        let channel_id = Id::<Channel>::mint();

        state.handle_logic_command(LogicCommand::Space {
            space_id,
            command: SpaceCommand::CreatePrivateChannel {
                draft: PrivateChannelDraft {
                    channel_id,
                    name: Validated::<Name>::parse("board").unwrap(),
                },
                now: NOW,
            },
        });
        let sent = drain(&sync);
        let [
            SyncCommand::CreateChannel {
                channel_id: asked,
                space_id: of,
                meta,
                segment,
                visibility: ambre_core::Visibility::Private,
                name,
            },
        ] = sent.as_slice()
        else {
            panic!("one private creation, got {sent:?}");
        };
        assert_eq!((*asked, *of), (channel_id, space_id));
        assert_eq!(
            name.as_ref().map(Validated::as_str),
            Some("board"),
            "sealed beside meta"
        );
        let (meta, segment) = (meta.document_id(), segment.document_id());

        state.handle_logic_event(LogicEvent::ChannelCreated {
            channel_id,
            key: None,
        });
        let sent = drain(&sync);
        assert_eq!(updates_of(&sent, space_id.document_id()), 0);
        for document_id in [meta, segment] {
            state.grant_identity(document_id);
            ready(&mut state, document_id, Role::Owner);
        }
        let sent = drain(&sync);
        assert_eq!(updates_of(&sent, space_id.document_id()), 0);
        assert_eq!(
            (updates_of(&sent, meta), updates_of(&sent, segment)),
            (1, 1)
        );
        let channel = &state.rt().channels()[&channel_id];
        assert_eq!(channel.channel_type(), Some(ChannelType::Private));
        assert_eq!(
            channel.name().as_ref().map(ambre_core::Validated::as_str),
            Some("board")
        );
        assert!(
            state
                .rt()
                .space(space_id)
                .expect("an open space")
                .catalogue()
                .is_empty()
        );
        assert!(drain(&ui).iter().any(|e| matches!(
            e,
            UiEvent::Space { event: SpaceEvent::ChannelCreated { channel_id: id }, .. }
                if *id == channel_id
        )));
    }

    // --- A self-join ---

    /// Bob's client holds a space where Alice listed a channel. The join hands
    /// the Sync worker the sealed key as the catalogue holds it. It writes no
    /// catalogue, and opens the channel once the relay accepted the member.
    #[test]
    fn a_self_join_hands_the_sealed_key_to_sync_and_writes_no_catalogue() {
        let space_id = Id::<Space>::mint();
        let mut alice =
            identified_space(SpaceState::create(space_id, &fit_name("Acme"), NOW).expect("create"));
        let listed = ChannelState::create(
            Id::<Channel>::mint(),
            ChannelType::Public,
            Some(&fit_name("general")),
            NOW,
        )
        .expect("channel");
        alice
            .list_channel(
                &listed,
                &Listing {
                    topic: None,
                    category: None,
                    position: ambre_bus::Rank::FIRST,
                    key: sealed(9),
                },
            )
            .expect("list");
        let channel_id = listed.channel_id();

        let (mut bob, ui, sync) = harness();
        bob.handle_logic_command(LogicCommand::OpenSpace { space_id });
        bob.handle_logic_event(LogicEvent::RemoteSnapshot {
            document_id: space_id.document_id(),
            covered_seq: ambre_core::Seq::<ambre_core::Relay>::new(1),
            payload: (alice.snapshot().expect("snapshot")),
        });
        ready(&mut bob, space_id.document_id(), Role::Editor);
        let _ = (drain(&ui), drain(&sync));
        let before = bob.rt().space(space_id).expect("an open space").version();

        bob.handle_logic_command(LogicCommand::Space {
            space_id,
            command: SpaceCommand::JoinChannel { channel_id },
        });
        let sent = drain(&sync);
        let [
            SyncCommand::JoinChannel {
                channel_id: asked,
                space_id: of,
                key,
            },
        ] = sent.as_slice()
        else {
            panic!("one join, got {sent:?}");
        };
        assert_eq!((*asked, *of), (channel_id, space_id));
        assert_eq!(*key, sealed(9), "the sealed bytes, unopened");
        assert_eq!(
            bob.rt().space(space_id).expect("an open space").version(),
            before,
            "no catalogue write"
        );

        bob.handle_logic_event(LogicEvent::ChannelJoined { channel_id });
        assert!(matches!(
            drain(&ui).as_slice(),
            [UiEvent::Space { event: SpaceEvent::Joined { channel_id: id }, .. }] if *id == channel_id
        ));
        let sent = drain(&sync);
        assert!(
            matches!(
                sent.as_slice(),
                [SyncCommand::OpenChannel { channel_id: id, listed_in: Some(of) }]
                    if *id == channel_id && *of == space_id
            ),
            "a listed channel's log admits self-joins"
        );
        assert_eq!(updates_of(&sent, space_id.document_id()), 0);
    }

    // --- A rotation of the space ---

    /// Bob's client opens a space where Alice listed one channel under
    /// `sealed(9)`, at `role`. Returns it with its Sync queue drained.
    fn listed_space_as(role: Role) -> (LogicState, Id<Space>, Id<Channel>, Receiver<SyncCommand>) {
        let space_id = Id::<Space>::mint();
        let mut alice =
            identified_space(SpaceState::create(space_id, &fit_name("Acme"), NOW).expect("create"));
        let listed = ChannelState::create(
            Id::<Channel>::mint(),
            ChannelType::Public,
            Some(&fit_name("general")),
            NOW,
        )
        .expect("channel");
        let listing = Listing {
            topic: None,
            category: None,
            position: ambre_bus::Rank::FIRST,
            key: sealed(9),
        };
        alice.list_channel(&listed, &listing).expect("list");
        let (mut bob, _ui, sync) = harness();
        bob.handle_logic_command(LogicCommand::OpenSpace { space_id });
        bob.handle_logic_event(LogicEvent::RemoteSnapshot {
            document_id: space_id.document_id(),
            covered_seq: ambre_core::Seq::<ambre_core::Relay>::new(1),
            payload: (alice.snapshot().expect("snapshot")),
        });
        ready(&mut bob, space_id.document_id(), role);
        (bob, space_id, listed.channel_id(), sync)
    }

    /// Holds `SYNC-230` on the Logic side. Bob opens a public channel before
    /// the space that lists it, so its listing names no space. Once the
    /// space opens, Sync learns the catalogue lists that channel. A channel
    /// Bob never opened is not marked.
    #[test]
    fn a_space_opened_after_its_channel_marks_that_channel_listed() {
        let space_id = Id::<Space>::mint();
        let mut alice =
            identified_space(SpaceState::create(space_id, &fit_name("Acme"), NOW).expect("create"));
        let [opened, unopened] = ["general", "random"].map(|name| {
            ChannelState::create(
                Id::<Channel>::mint(),
                ChannelType::Public,
                Some(&fit_name(name)),
                NOW,
            )
            .expect("channel")
        });
        for channel in [&opened, &unopened] {
            let listing = Listing {
                topic: None,
                category: None,
                position: ambre_bus::Rank::FIRST,
                key: sealed(9),
            };
            alice.list_channel(channel, &listing).expect("list");
        }
        let (mut bob, _ui, sync) = harness();
        bob.handle_logic_command(LogicCommand::OpenChannel {
            channel_id: opened.channel_id(),
        });
        assert!(matches!(
            drain(&sync).as_slice(),
            [SyncCommand::OpenChannel {
                listed_in: None,
                ..
            }]
        ));
        bob.handle_logic_command(LogicCommand::OpenSpace { space_id });
        bob.handle_logic_event(LogicEvent::RemoteSnapshot {
            document_id: space_id.document_id(),
            covered_seq: ambre_core::Seq::<ambre_core::Relay>::new(1),
            payload: (alice.snapshot().expect("snapshot")),
        });
        ready(&mut bob, space_id.document_id(), Role::Editor);
        let marked: Vec<(Id<Channel>, Id<Space>)> = drain(&sync)
            .iter()
            .filter_map(|c| match c {
                SyncCommand::MarkChannelListed {
                    channel_id,
                    space_id,
                } => Some((*channel_id, *space_id)),
                _ => None,
            })
            .collect();
        assert_eq!(marked, vec![(opened.channel_id(), space_id)]);
    }

    /// Whether `sent` hands Sync the catalogue of `space_id` as `channels`.
    fn hands(
        sent: &[SyncCommand],
        space_id: Id<Space>,
        channels: &[(Id<Channel>, SealedChannelKey)],
    ) -> bool {
        sent.iter().any(|c| {
            matches!(c, SyncCommand::RotateSpace { space_id: s, channels: held }
                if *s == space_id && held.as_slice() == channels)
        })
    }

    /// An admin hands Sync each sealed entry of the catalogue, so a rotation
    /// of the space re-keys the stale ones. A viewer writes no catalogue,
    /// and hands nothing (the Murmure spaces spec).
    #[test]
    fn an_admin_hands_the_sealed_catalogue_to_sync() {
        let (_, space_id, channel_id, sync) = listed_space_as(Role::Editor);
        assert!(hands(&drain(&sync), space_id, &[(channel_id, sealed(9))]));
        let (_, space_id, _, sync) = listed_space_as(Role::Viewer);
        let sent = drain(&sync);
        assert!(
            !sent
                .iter()
                .any(|c| matches!(c, SyncCommand::RotateSpace { .. })),
            "a viewer hands nothing: {sent:?}"
        );
        assert!(!hands(&sent, space_id, &[]));
    }

    /// A re-keyed channel's entry takes the new key, sent as one update, and
    /// Sync holds the catalogue as it now stands. The same key a second time
    /// writes nothing (the Murmure spaces spec).
    #[test]
    fn a_resealed_channel_writes_its_new_key_once() {
        let (mut bob, ui, sync) = harness();
        let space_id = created_space(&mut bob, &ui, &sync);
        let (channel_id, ..) = public_channel(&mut bob, space_id, "general", &ui, &sync);
        let _ = drain(&sync);
        let resealed = LogicEvent::ChannelKeyResealed {
            space_id,
            channel_id,
            key: sealed(11),
        };
        bob.handle_logic_event(resealed.clone());
        let state = &bob.rt().space(space_id).expect("an open space");
        assert_eq!(state.sealed_key(channel_id).expect("listed"), sealed(11));
        let sent = drain(&sync);
        assert_eq!(updates_of(&sent, space_id.document_id()), 1);
        assert!(hands(&sent, space_id, &[(channel_id, sealed(11))]));
        bob.handle_logic_event(resealed);
        assert_eq!(updates_of(&drain(&sync), space_id.document_id()), 0);
        bob.handle_logic_event(LogicEvent::ChannelKeyResealed {
            space_id: Id::<Space>::mint(),
            channel_id,
            key: sealed(8),
        });
        assert!(drain(&sync).is_empty(), "no such space");
    }

    // --- Deleting a channel ---

    /// A public channel created in a space, held with its two documents
    /// ready: its id, `meta` and first segment.
    fn public_channel(
        state: &mut LogicState,
        space_id: Id<Space>,
        name: &str,
        ui: &Receiver<UiEvent>,
        sync: &Receiver<SyncCommand>,
    ) -> (Id<Channel>, Id<Document>, Id<Document>) {
        let draft = draft(name);
        let channel_id = draft.channel_id;
        state.handle_logic_command(LogicCommand::Space {
            space_id,
            command: SpaceCommand::CreatePublicChannel { draft, now: NOW },
        });
        state.handle_logic_event(LogicEvent::ChannelCreated {
            channel_id,
            key: Some(sealed(7)),
        });
        let channel = &state.rt().channels()[&channel_id];
        let (meta, segment) = (
            channel.meta_id().document_id(),
            channel.active_id().document_id(),
        );
        for document_id in [meta, segment] {
            state.grant_identity(document_id);
            ready(state, document_id, Role::Owner);
        }
        let _ = (drain(ui), drain(sync));
        (channel_id, meta, segment)
    }

    /// The owner deletes a public channel. Nothing changes before the relay
    /// answers. Then the channel goes, each of its sessions ends so the Sync
    /// worker forgets their keys, and the catalogue drops its entry. The
    /// other channel of the space stays as it was.
    #[test]
    fn a_deleted_channel_leaves_the_client_and_the_catalogue_once_the_relay_answers() {
        let (mut state, ui, sync) = harness();
        let space_id = created_space(&mut state, &ui, &sync);
        let (channel_id, meta, segment) =
            public_channel(&mut state, space_id, "budget", &ui, &sync);
        let (kept, ..) = public_channel(&mut state, space_id, "general", &ui, &sync);

        state.handle_logic_command(LogicCommand::Space {
            space_id,
            command: SpaceCommand::DeleteChannel { channel_id },
        });
        assert!(matches!(
            drain(&sync).as_slice(),
            [SyncCommand::DeleteChannel { channel_id: id }] if *id == channel_id
        ));
        assert!(drain(&ui).is_empty(), "nothing shows before the answer");
        assert!(state.rt().channels().contains_key(&channel_id));
        assert_eq!(
            state
                .rt()
                .space(space_id)
                .expect("an open space")
                .catalogue()
                .len(),
            2
        );

        state.handle_logic_event(LogicEvent::ChannelDeleted { channel_id });
        let sent = drain(&sync);
        let mut closed: Vec<Id<Document>> = sent
            .iter()
            .filter_map(|c| match c {
                SyncCommand::CloseDocument { document_id } => Some(*document_id),
                _ => None,
            })
            .collect();
        closed.sort_by_key(|d| *d.as_bytes());
        let mut expected = [meta, segment];
        expected.sort_by_key(|d| *d.as_bytes());
        assert_eq!(closed, expected, "both sessions end, and only theirs");
        assert_eq!(updates_of(&sent, space_id.document_id()), 1);
        assert!(!state.rt().channels().contains_key(&channel_id));
        assert!(state.rt().channels().contains_key(&kept));

        let shown = drain(&ui);
        assert!(shown.iter().any(|e| matches!(
            e,
            UiEvent::Space { event: SpaceEvent::ChannelDeleted { channel_id: id }, .. }
                if *id == channel_id
        )));
        let (_, catalogue) = last_catalogue(&shown).expect("the catalogue");
        let listed: Vec<Id<Channel>> = catalogue.iter().map(|e| e.channel_id).collect();
        assert_eq!(listed, [kept]);

        // An answer nobody here asked for drops nothing.
        state.handle_logic_event(LogicEvent::ChannelDeleted { channel_id: kept });
        assert!(drain(&sync).is_empty());
        assert!(state.rt().channels().contains_key(&kept));
    }

    /// A space admin's client asks the relay which channels it holds each
    /// time the space goes live. It unlists an entry the answer lacks, but
    /// never one listed since it asked. A viewer asks nothing.
    #[test]
    fn an_admin_unlists_an_entry_whose_channel_the_relay_no_longer_holds() {
        let (mut state, ui, sync) = harness();
        let space_id = created_space(&mut state, &ui, &sync);
        let (gone, ..) = public_channel(&mut state, space_id, "budget", &ui, &sync);
        let (kept, ..) = public_channel(&mut state, space_id, "general", &ui, &sync);

        ready(&mut state, space_id.document_id(), Role::Editor);
        let sent = drain(&sync);
        assert!(sent.iter().any(|c| matches!(
            c,
            SyncCommand::ListSpaceChannels { space_id: s } if *s == space_id
        )));
        // Another admin lists a channel after the relay's read.
        let (late, ..) = public_channel(&mut state, space_id, "late", &ui, &sync);

        state.handle_logic_event(LogicEvent::SpaceChannels {
            space_id,
            channels: vec![kept],
        });
        assert_eq!(updates_of(&drain(&sync), space_id.document_id()), 1);
        let (_, catalogue) = last_catalogue(&drain(&ui)).expect("the catalogue");
        let mut listed: Vec<Id<Channel>> = catalogue.iter().map(|e| e.channel_id).collect();
        listed.sort_by_key(|id| *id.as_bytes());
        let mut expected = [kept, late];
        expected.sort_by_key(|id| *id.as_bytes());
        assert_eq!(listed, expected, "{gone} goes, and only it");

        // An answer nobody here asked for unlists nothing.
        state.handle_logic_event(LogicEvent::SpaceChannels {
            space_id,
            channels: Vec::new(),
        });
        assert!(drain(&sync).is_empty());

        let (mut viewer, ui, sync) = harness();
        let payload = state
            .rt()
            .space(space_id)
            .expect("an open space")
            .snapshot()
            .expect("snapshot");
        viewer.handle_logic_command(LogicCommand::OpenSpace { space_id });
        viewer.grant_identity(space_id.document_id());
        viewer.handle_logic_event(LogicEvent::RemoteSnapshot {
            document_id: space_id.document_id(),
            covered_seq: ambre_core::Seq::<ambre_core::Relay>::new(1),
            payload,
        });
        ready(&mut viewer, space_id.document_id(), Role::Viewer);
        assert!(
            !drain(&sync)
                .iter()
                .any(|c| matches!(c, SyncCommand::ListSpaceChannels { .. }))
        );
        assert_eq!(last_catalogue(&drain(&ui)).expect("shown").1.len(), 2);
    }

    /// The relay's notice ends a delete asked here as its answer would: the
    /// sessions end, the catalogue drops the entry and the view learns it.
    /// The answer after it changes nothing. A channel this client did not
    /// delete goes on the same notice, and the view learns it is gone.
    #[test]
    fn a_delete_ends_on_the_relays_notice() {
        let (mut state, ui, sync) = harness();
        let me = Id::<User>::mint();
        state.set_user(me);
        let space_id = created_space(&mut state, &ui, &sync);
        let (channel_id, meta, _) = public_channel(&mut state, space_id, "budget", &ui, &sync);
        let (other, other_meta, _) = public_channel(&mut state, space_id, "general", &ui, &sync);
        let removed = |document_id| LogicEvent::MemberRevoked {
            document_id,
            revoked_user_id: me,
        };

        state.handle_logic_command(LogicCommand::Space {
            space_id,
            command: SpaceCommand::DeleteChannel { channel_id },
        });
        let _ = (drain(&sync), drain(&ui));
        state.handle_logic_event(removed(meta));
        let sent = drain(&sync);
        assert_eq!(updates_of(&sent, space_id.document_id()), 1, "unlisted");
        assert!(!state.rt().channels().contains_key(&channel_id));
        let shown = drain(&ui);
        assert!(shown.iter().any(|e| matches!(
            e,
            UiEvent::Space { event: SpaceEvent::ChannelDeleted { channel_id: id }, .. }
                if *id == channel_id
        )));
        state.handle_logic_event(LogicEvent::ChannelDeleted { channel_id });
        assert!(drain(&sync).is_empty() && drain(&ui).is_empty());

        state.handle_logic_event(removed(other_meta));
        assert!(!state.rt().channels().contains_key(&other));
        assert_eq!(
            updates_of(&drain(&sync), space_id.document_id()),
            0,
            "only the owner unlists"
        );
        assert!(drain(&ui).iter().any(|e| matches!(
            e,
            UiEvent::Channel { channel_id: id, event: ambre_murmure::message::MurmureEvent::Left }
                if *id == other
        )));
    }

    /// The relay refuses the delete: the view is told, and the channel, its
    /// sessions and its catalogue entry all stay as they were.
    #[test]
    fn a_refused_delete_keeps_the_channel_whole() {
        let (mut state, ui, sync) = harness();
        let space_id = created_space(&mut state, &ui, &sync);
        let (channel_id, ..) = public_channel(&mut state, space_id, "budget", &ui, &sync);
        let before = state.rt().space(space_id).expect("an open space").version();

        state.handle_logic_command(LogicCommand::Space {
            space_id,
            command: SpaceCommand::DeleteChannel { channel_id },
        });
        let _ = drain(&sync);
        state.handle_logic_event(LogicEvent::ChannelRequestFailed {
            channel_id,
            failure: ambre_bus::ApiFailure::NoAnswer,
        });
        assert!(matches!(
            drain(&ui).as_slice(),
            [UiEvent::Space { event: SpaceEvent::ChannelFailed { channel_id: id, .. }, .. }]
                if *id == channel_id
        ));
        assert!(
            drain(&sync).is_empty(),
            "no session ends, nothing is written"
        );
        assert!(state.rt().channels().contains_key(&channel_id));
        assert_eq!(
            state.rt().space(space_id).expect("an open space").version(),
            before
        );
        assert_eq!(
            state
                .rt()
                .space(space_id)
                .expect("an open space")
                .sealed_key(channel_id),
            Ok(sealed(7))
        );

        // A late yes for a delete given up drops nothing either.
        state.handle_logic_event(LogicEvent::ChannelDeleted { channel_id });
        assert!(state.rt().channels().contains_key(&channel_id));
    }

    // --- Leaving a channel ---

    /// A member leaves a public channel. Nothing changes before the relay
    /// answers. Then the channel goes and each of its sessions ends, so the
    /// Sync worker forgets their keys. The catalogue keeps listing it, and
    /// nothing is written to the space.
    #[test]
    fn a_left_channel_leaves_the_client_once_the_relay_answers() {
        let (mut state, ui, sync) = harness();
        let space_id = created_space(&mut state, &ui, &sync);
        let (channel_id, meta, segment) =
            public_channel(&mut state, space_id, "budget", &ui, &sync);
        let (kept, ..) = public_channel(&mut state, space_id, "general", &ui, &sync);
        let before = state.rt().space(space_id).expect("an open space").version();

        state.handle_logic_command(LogicCommand::Space {
            space_id,
            command: SpaceCommand::LeaveChannel { channel_id },
        });
        assert!(matches!(
            drain(&sync).as_slice(),
            [SyncCommand::LeaveChannel { channel_id: id }] if *id == channel_id
        ));
        assert!(drain(&ui).is_empty(), "nothing shows before the answer");
        assert!(state.rt().channels().contains_key(&channel_id));

        state.handle_logic_event(LogicEvent::ChannelLeft { channel_id });
        let mut closed: Vec<Id<Document>> = drain(&sync)
            .iter()
            .map(|c| match c {
                SyncCommand::CloseDocument { document_id } => *document_id,
                other => panic!("only sessions end, got {other:?}"),
            })
            .collect();
        closed.sort_by_key(|d| *d.as_bytes());
        let mut expected = [meta, segment];
        expected.sort_by_key(|d| *d.as_bytes());
        assert_eq!(closed, expected, "both sessions end, and only theirs");
        assert!(!state.rt().channels().contains_key(&channel_id));
        assert!(state.rt().channels().contains_key(&kept));
        assert_eq!(
            state.rt().space(space_id).expect("an open space").version(),
            before
        );
        assert_eq!(
            state
                .rt()
                .space(space_id)
                .expect("an open space")
                .catalogue()
                .len(),
            2
        );
        assert!(matches!(
            drain(&ui).as_slice(),
            [UiEvent::Space { event: SpaceEvent::ChannelLeft { channel_id: id }, .. }]
                if *id == channel_id
        ));

        // An answer nobody here asked for drops nothing.
        state.handle_logic_event(LogicEvent::ChannelLeft { channel_id: kept });
        assert!(drain(&sync).is_empty());
        assert!(state.rt().channels().contains_key(&kept));
    }

    /// The relay tells this client its user left a channel,
    /// from another client. The channel goes with its sessions, and the view
    /// learns it. A removal of someone else, or from a document no channel
    /// here holds, drops nothing. A leave asked here ends on the notice.
    #[test]
    fn a_leave_from_another_client_drops_the_channel_here() {
        let (mut state, ui, sync) = harness();
        let me = Id::<User>::mint();
        state.set_user(me);
        let space_id = created_space(&mut state, &ui, &sync);
        let (channel_id, meta, segment) =
            public_channel(&mut state, space_id, "budget", &ui, &sync);
        let (kept, kept_meta, _) = public_channel(&mut state, space_id, "general", &ui, &sync);
        let revoked = |document_id, revoked_user_id| LogicEvent::MemberRevoked {
            document_id,
            revoked_user_id,
        };

        state.handle_logic_event(revoked(kept_meta, Id::<User>::mint()));
        state.handle_logic_event(revoked(segment, me));
        assert!(
            drain(&sync).is_empty(),
            "a session ended for no leave of ours"
        );
        assert!(
            state.rt().channels().contains_key(&kept)
                && state.rt().channels().contains_key(&channel_id)
        );
        let _ = drain(&ui);

        state.handle_logic_event(revoked(meta, me));
        let mut closed: Vec<Id<Document>> = drain(&sync)
            .iter()
            .map(|c| match c {
                SyncCommand::CloseDocument { document_id } => *document_id,
                other => panic!("only sessions end, got {other:?}"),
            })
            .collect();
        closed.sort_by_key(|d| *d.as_bytes());
        let mut expected = [meta, segment];
        expected.sort_by_key(|d| *d.as_bytes());
        assert_eq!(closed, expected);
        assert!(!state.rt().channels().contains_key(&channel_id));
        assert!(state.rt().channels().contains_key(&kept));
        let told = drain(&ui);
        assert!(
            told.iter().any(|e| matches!(
                e,
                UiEvent::Channel { channel_id: id, event: ambre_murmure::message::MurmureEvent::Left }
                    if *id == channel_id
            )),
            "the view was told {told:?}"
        );

        // A leave asked here ends on the notice. Its answer, lost after it,
        // changes nothing.
        state.handle_logic_command(LogicCommand::Space {
            space_id,
            command: SpaceCommand::LeaveChannel { channel_id: kept },
        });
        let _ = (drain(&sync), drain(&ui));
        state.handle_logic_event(revoked(kept_meta, me));
        assert!(!state.rt().channels().contains_key(&kept));
        assert!(
            drain(&sync)
                .iter()
                .all(|c| matches!(c, SyncCommand::CloseDocument { .. })),
            "only sessions end"
        );
        let told = drain(&ui);
        assert!(
            matches!(
                told.as_slice(),
                [UiEvent::Space { event: SpaceEvent::ChannelLeft { channel_id: id }, .. }, ..]
                    if *id == kept
            ),
            "the view was told {told:?}"
        );
        state.handle_logic_event(LogicEvent::ChannelRequestFailed {
            channel_id: kept,
            failure: ambre_bus::ApiFailure::NoAnswer,
        });
        assert!(drain(&ui).is_empty(), "no failure shown for a leave done");
    }

    /// The relay refuses the leave: the view is told, and the channel and its
    /// sessions stay as they were.
    #[test]
    fn a_refused_leave_keeps_the_channel_whole() {
        let (mut state, ui, sync) = harness();
        let space_id = created_space(&mut state, &ui, &sync);
        let (channel_id, ..) = public_channel(&mut state, space_id, "budget", &ui, &sync);

        state.handle_logic_command(LogicCommand::Space {
            space_id,
            command: SpaceCommand::LeaveChannel { channel_id },
        });
        let _ = drain(&sync);
        state.handle_logic_event(LogicEvent::ChannelRequestFailed {
            channel_id,
            failure: ambre_bus::ApiFailure::NoAnswer,
        });
        assert!(matches!(
            drain(&ui).as_slice(),
            [UiEvent::Space { event: SpaceEvent::ChannelFailed { channel_id: id, .. }, .. }]
                if *id == channel_id
        ));
        assert!(drain(&sync).is_empty(), "no session ends");
        assert!(state.rt().channels().contains_key(&channel_id));

        // A late yes for a leave given up drops nothing either.
        state.handle_logic_event(LogicEvent::ChannelLeft { channel_id });
        assert!(state.rt().channels().contains_key(&channel_id));
    }

    /// A delete and a leave still unanswered at the logout
    /// belong to the user who asked them. Their late answers reach no view
    /// and write nothing in the next session.
    #[test]
    fn a_logout_forgets_the_deletes_and_leaves_the_relay_has_not_answered() {
        let (mut state, ui, sync) = harness();
        let space_id = created_space(&mut state, &ui, &sync);
        let (deleted, ..) = public_channel(&mut state, space_id, "budget", &ui, &sync);
        let (left, ..) = public_channel(&mut state, space_id, "general", &ui, &sync);
        state.handle_logic_command(LogicCommand::Space {
            space_id,
            command: SpaceCommand::DeleteChannel {
                channel_id: deleted,
            },
        });
        state.handle_logic_command(LogicCommand::Space {
            space_id,
            command: SpaceCommand::LeaveChannel { channel_id: left },
        });
        state.handle_logic_command(LogicCommand::Rest {
            id: ambre_bus::next_request(),
            cmd: ambre_bus::RestCommand::Logout,
        });
        state.handle_logic_event(LogicEvent::Rest {
            id: None,
            evt: ambre_bus::RestEvent::SessionReady {
                user_id: Id::<ambre_core::id::User>::mint(),
                display_name: Validated::<ambre_core::DisplayName>::parse("bob").unwrap(),
                email: ambre_core::Email::from("bob@example.com"),
            },
        });
        let _ = (drain(&ui), drain(&sync));

        state.handle_logic_event(LogicEvent::ChannelDeleted {
            channel_id: deleted,
        });
        state.handle_logic_event(LogicEvent::ChannelLeft { channel_id: left });
        let shown = drain(&ui);
        assert!(shown.is_empty(), "a late answer reaches no view: {shown:?}");
        assert!(drain(&sync).is_empty(), "a late answer writes nothing");
    }

    #[test]
    fn joining_a_channel_the_catalogue_does_not_list_asks_nothing() {
        let (mut state, ui, sync) = harness();
        let space_id = created_space(&mut state, &ui, &sync);
        state.handle_logic_command(LogicCommand::Space {
            space_id,
            command: SpaceCommand::JoinChannel {
                channel_id: Id::<Channel>::mint(),
            },
        });
        assert!(drain(&sync).is_empty());
        assert!(matches!(
            drain(&ui).as_slice(),
            [UiEvent::Space {
                event: SpaceEvent::ChannelFailed { .. },
                ..
            }]
        ));
    }

    #[test]
    fn a_remote_catalogue_delta_reaches_the_open_space() {
        let space_id = Id::<Space>::mint();
        let mut alice =
            identified_space(SpaceState::create(space_id, &fit_name("Acme"), NOW).expect("create"));
        let first = alice.export_all().expect("export");
        let (mut bob, ui, sync) = harness();
        bob.handle_logic_command(LogicCommand::OpenSpace { space_id });
        bob.handle_logic_event(LogicEvent::RemoteUpdate {
            document_id: space_id.document_id(),
            delta: crate::remote_delta(1, first),
        });
        ready(&mut bob, space_id.document_id(), Role::Editor);
        let _ = (drain(&ui), drain(&sync));

        let listed = ChannelState::create(
            Id::<Channel>::mint(),
            ChannelType::Public,
            Some(&fit_name("general")),
            NOW,
        )
        .expect("channel");
        let delta = alice
            .list_channel(
                &listed,
                &Listing {
                    topic: None,
                    category: None,
                    position: ambre_bus::Rank::FIRST,
                    key: sealed(2),
                },
            )
            .expect("list");
        bob.handle_logic_event(LogicEvent::RemoteUpdate {
            document_id: space_id.document_id(),
            delta: crate::remote_delta(2, delta),
        });
        let (_, catalogue) = last_catalogue(&drain(&ui)).expect("the catalogue");
        assert_eq!(catalogue.len(), 1);
        assert_eq!(catalogue[0].name.as_str(), "general");
    }

    #[test]
    fn closing_a_space_ends_its_session_and_forgets_it() {
        let (mut state, ui, sync) = harness();
        let space_id = created_space(&mut state, &ui, &sync);
        state.handle_logic_command(LogicCommand::CloseSpace { space_id });
        assert!(matches!(
            drain(&sync).as_slice(),
            [SyncCommand::CloseDocument { .. }]
        ));
        assert!(state.rt().spaces().next().is_none());
        let empty = ambre_core::Payload::<ambre_core::DeltaBytes>::default();
        assert!(!state.route(|m| m.apply_space_remote(
            space_id.document_id(),
            &[ambre_core::LoroPayloadRef::Delta(&empty)]
        )));
    }

    // --- The members of a space, and their presence ---

    use ambre_bus::{MemberInfo, Request, RestCommand};
    use ambre_core::Seq;
    use ambre_murmure::PRESENCE_EXPIRY;
    use ambre_murmure::message::{MemberView, Presence};

    /// The member list of the last members event the view got.
    fn last_members(shown: &[UiEvent]) -> Option<Vec<MemberView>> {
        shown.iter().rev().find_map(|e| match e {
            UiEvent::Space {
                event: SpaceEvent::Members { members },
                ..
            } => Some(members.clone()),
            _ => None,
        })
    }

    /// Every presence sample sent, with the document it went on.
    fn samples(
        sent: &[SyncCommand],
    ) -> Vec<(
        Id<Document>,
        ambre_core::Payload<ambre_core::EphemeralBytes>,
    )> {
        sent.iter()
            .filter_map(|c| match c {
                SyncCommand::SendEphemeral {
                    document_id,
                    payload,
                } => Some((*document_id, payload.clone())),
                _ => None,
            })
            .collect()
    }

    /// The member listing this thread asked for, with the document it names.
    fn member_request(sent: &[SyncCommand]) -> Option<(Seq<Request>, Id<Document>)> {
        sent.iter().find_map(|c| match c {
            SyncCommand::Rest {
                id,
                cmd: RestCommand::ListDocumentMembers { document_id },
            } => Some((*id, *document_id)),
            _ => None,
        })
    }

    fn member(user_id: Id<User>, name: &str) -> MemberInfo {
        MemberInfo {
            user_id,
            display_name: ambre_core::Validated::<ambre_core::DisplayName>::parse(name).unwrap(),
            email: ambre_core::Email::new(format!("{name}@example.org")),
            role: Role::Editor,
            standing: ambre_bus::MemberStanding::Verified,
        }
    }

    /// A space the relay holds, opened with these members, and the channels
    /// drained.
    fn space_with(
        state: &mut LogicState,
        ui: &Receiver<UiEvent>,
        sync: &Receiver<SyncCommand>,
        members: Vec<MemberInfo>,
    ) -> Id<Space> {
        let space_id = Id::<Space>::mint();
        let stored = SpaceState::create(space_id, &fit_name("Acme"), NOW).expect("create");
        state.handle_logic_command(LogicCommand::OpenSpace { space_id });
        state.handle_logic_event(LogicEvent::RemoteSnapshot {
            document_id: space_id.document_id(),
            covered_seq: ambre_core::Seq::<ambre_core::Relay>::new(1),
            payload: (stored.snapshot().expect("snapshot")),
        });
        ready(state, space_id.document_id(), Role::Editor);
        state.handle_logic_event(LogicEvent::MembersChanged {
            document_id: space_id.document_id(),
            members,
        });
        state.handle_logic_command(LogicCommand::Space {
            space_id,
            command: SpaceCommand::RefreshPresence { now: NOW },
        });
        let _ = (drain(ui), drain(sync));
        space_id
    }

    fn sample_from(
        state: &mut LogicState,
        space_id: Id<Space>,
        from: Id<User>,
        payload: ambre_core::Payload<ambre_core::EphemeralBytes>,
    ) {
        state.handle_logic_event(LogicEvent::RemoteEphemeral {
            document_id: space_id.document_id(),
            from_user_id: from,
            payload,
        });
    }

    /// Opening a space sends this user's presence at once, on the space
    /// document and on no other. Only the members of the space hold that
    /// key, so no one outside it learns of the open.
    #[test]
    fn opening_a_space_sends_this_users_presence_on_its_document_alone() {
        let (mut state, _ui, sync) = harness();
        let space_id = Id::<Space>::mint();
        let stored = SpaceState::create(space_id, &fit_name("Acme"), NOW).expect("create");
        state.handle_logic_command(LogicCommand::OpenSpace { space_id });
        state.handle_logic_event(LogicEvent::RemoteSnapshot {
            document_id: space_id.document_id(),
            covered_seq: ambre_core::Seq::<ambre_core::Relay>::new(1),
            payload: (stored.snapshot().expect("snapshot")),
        });
        assert!(
            samples(&drain(&sync)).is_empty(),
            "no sample before the open"
        );
        ready(&mut state, space_id.document_id(), Role::Editor);
        assert_eq!(
            samples(&drain(&sync)),
            [(space_id.document_id(), ambre_murmure::presence_sample())]
        );
    }

    /// The member list is the relay's rows for the space document: Sync's
    /// reading becomes a members event, never a service event of the view.
    /// Listing them opens no other document.
    #[test]
    fn the_member_list_holds_the_rows_of_the_space_document_and_opens_nothing_else() {
        let (mut state, ui, sync) = harness();
        let space_id = Id::<Space>::mint();
        let stored = SpaceState::create(space_id, &fit_name("Acme"), NOW).expect("create");
        state.handle_logic_command(LogicCommand::OpenSpace { space_id });
        state.handle_logic_event(LogicEvent::RemoteSnapshot {
            document_id: space_id.document_id(),
            covered_seq: ambre_core::Seq::<ambre_core::Relay>::new(1),
            payload: (stored.snapshot().expect("snapshot")),
        });
        let _ = drain(&sync);
        ready(&mut state, space_id.document_id(), Role::Editor);
        let sent = drain(&sync);
        let (alice, bob) = (Id::<User>::mint(), Id::<User>::mint());
        state.handle_logic_event(LogicEvent::MembersChanged {
            document_id: space_id.document_id(),
            members: vec![member(bob, "Bob"), member(alice, "Alice")],
        });
        let shown = drain(&ui);
        assert!(!shown.iter().any(|e| matches!(e, UiEvent::Rest { .. })));
        let members = last_members(&shown).expect("a member list");
        let ids: Vec<Id<User>> = members.iter().map(|m| m.user_id).collect();
        assert_eq!(ids, [alice, bob]);
        assert!(members.iter().all(|m| m.presence == Presence::Offline));
        let opened = sent
            .iter()
            .chain(drain(&sync).iter())
            .filter(|c| matches!(c, SyncCommand::OpenDocument { .. }))
            .count();
        assert_eq!(opened, 0, "the list opens no document");
    }

    /// Each change of members Sync hands on replaces the rows of an open
    /// space. A newcomer joins the list, and a member who left drops out.
    #[test]
    fn a_change_of_members_while_the_space_is_open_reads_the_rows_again() {
        let (mut state, ui, sync) = harness();
        let (alice, bob, carol) = (Id::<User>::mint(), Id::<User>::mint(), Id::<User>::mint());
        let space_id = space_with(
            &mut state,
            &ui,
            &sync,
            vec![member(alice, "Alice"), member(bob, "Bob")],
        );
        let changed = |members| LogicEvent::MembersChanged {
            document_id: space_id.document_id(),
            members,
        };
        state.handle_logic_event(changed(vec![member(alice, "Alice")]));
        state.handle_logic_event(changed(vec![
            member(alice, "Alice"),
            member(carol, "Carol"),
        ]));
        let members = last_members(&drain(&ui)).expect("a member list");
        let ids: Vec<Id<User>> = members.iter().map(|m| m.user_id).collect();
        assert_eq!(ids, [alice, carol]);

        // A document no space holds asks for nothing.
        state.handle_logic_event(LogicEvent::MembersChanged {
            document_id: Id::<Document>::mint(),
            members: Vec::new(),
        });
        assert!(member_request(&drain(&sync)).is_none());
        assert!(last_members(&drain(&ui)).is_none());
    }

    /// Holds `SYNC-151`: the rows a change of members hands on become the
    /// list of an open space. It asks the relay for nothing more.
    #[test]
    fn a_change_of_members_hands_the_rows_and_costs_the_space_no_request() {
        let (mut state, ui, sync) = harness();
        let (alice, carol) = (Id::<User>::mint(), Id::<User>::mint());
        let space_id = space_with(&mut state, &ui, &sync, vec![member(alice, "Alice")]);

        state.handle_logic_event(LogicEvent::MembersChanged {
            document_id: space_id.document_id(),
            members: vec![member(alice, "Alice"), member(carol, "Carol")],
        });
        assert!(member_request(&drain(&sync)).is_none(), "no request");
        let members = last_members(&drain(&ui)).expect("a member list");
        let ids: Vec<Id<User>> = members.iter().map(|m| m.user_id).collect();
        assert_eq!(ids, [alice, carol]);
    }

    /// Opening a space asks the relay for no member rows. Rows Sync hands on while the space opens wait for it, and only
    /// the last of them shows once it is open.
    #[test]
    fn an_opening_space_asks_for_no_rows_and_shows_the_last_ones_sync_read() {
        let (mut state, ui, sync) = harness();
        let (alice, bob) = (Id::<User>::mint(), Id::<User>::mint());
        let space_id = Id::<Space>::mint();
        let stored = SpaceState::create(space_id, &fit_name("Acme"), NOW).expect("create");
        state.handle_logic_command(LogicCommand::OpenSpace { space_id });
        let changed = |members| LogicEvent::MembersChanged {
            document_id: space_id.document_id(),
            members,
        };
        state.handle_logic_event(changed(vec![member(alice, "Alice")]));
        state.handle_logic_event(changed(vec![member(alice, "Alice"), member(bob, "Bob")]));
        assert!(
            last_members(&drain(&ui)).is_none(),
            "nothing before the open"
        );

        state.handle_logic_event(LogicEvent::RemoteSnapshot {
            document_id: space_id.document_id(),
            covered_seq: ambre_core::Seq::<ambre_core::Relay>::new(1),
            payload: (stored.snapshot().expect("snapshot")),
        });
        ready(&mut state, space_id.document_id(), Role::Editor);
        assert!(member_request(&drain(&sync)).is_none(), "no listing asked");
        let members = last_members(&drain(&ui)).expect("the rows at open");
        let ids: Vec<Id<User>> = members.iter().map(|m| m.user_id).collect();
        assert_eq!(ids, [alice, bob]);
    }

    /// A sample turns its member online, and gets this user's sample back
    /// once, so the newcomer sees who is online without waiting a refresh
    /// period. A second sample of the same member asks nothing more.
    #[test]
    fn a_sample_shows_its_member_online_and_a_newcomer_is_answered_once() {
        let (mut state, ui, sync) = harness();
        let bob = Id::<User>::mint();
        let space_id = space_with(&mut state, &ui, &sync, vec![member(bob, "Bob")]);
        sample_from(&mut state, space_id, bob, ambre_murmure::presence_sample());
        let members = last_members(&drain(&ui)).expect("a member list");
        assert_eq!(members[0].presence, Presence::Online);
        assert_eq!(samples(&drain(&sync)).len(), 1, "one answer");

        sample_from(&mut state, space_id, bob, ambre_murmure::presence_sample());
        assert!(drain(&ui).is_empty());
        assert!(samples(&drain(&sync)).is_empty());
    }

    /// A member whose last sample is older than the expiry shows offline at
    /// the next tick, and each tick sends this user's sample.
    #[test]
    fn a_member_with_no_sample_within_the_expiry_shows_offline() {
        let (mut state, ui, sync) = harness();
        let bob = Id::<User>::mint();
        let space_id = space_with(&mut state, &ui, &sync, vec![member(bob, "Bob")]);
        sample_from(&mut state, space_id, bob, ambre_murmure::presence_sample());
        let _ = (drain(&ui), drain(&sync));

        let tick = |state: &mut LogicState, offset: u64| {
            state.handle_logic_command(LogicCommand::Space {
                space_id,
                command: SpaceCommand::RefreshPresence {
                    now: Instant::<Authored>::new(NOW.get() + offset),
                },
            });
        };
        tick(&mut state, PRESENCE_EXPIRY.get());
        assert!(last_members(&drain(&ui)).is_none(), "Bob is still online");
        assert_eq!(samples(&drain(&sync)).len(), 1);

        tick(&mut state, PRESENCE_EXPIRY.get() + 1);
        let members = last_members(&drain(&ui)).expect("a member list");
        assert_eq!(members[0].presence, Presence::Offline);
        assert_eq!(samples(&drain(&sync)).len(), 1);
    }

    /// The relay's word that a member left turns them offline at once. The
    /// departure of another tab of this user turns no
    /// one offline: this tab answers with its sample.
    #[test]
    fn a_departure_turns_its_member_offline_and_another_tab_of_this_user_answers() {
        let (mut state, ui, sync) = harness();
        let (me, bob) = (Id::<User>::mint(), Id::<User>::mint());
        state.set_user(me);
        let space_id = space_with(
            &mut state,
            &ui,
            &sync,
            vec![member(me, "Me"), member(bob, "Bob")],
        );
        sample_from(&mut state, space_id, bob, ambre_murmure::presence_sample());
        let _ = (drain(&ui), drain(&sync));

        state.handle_logic_event(LogicEvent::PeerLeft {
            document_id: space_id.document_id(),
            user_id: bob,
        });
        let members = last_members(&drain(&ui)).expect("a member list");
        let presence = |id: Id<User>| members.iter().find(|m| m.user_id == id).map(|m| m.presence);
        assert_eq!(presence(bob), Some(Presence::Offline));
        assert_eq!(presence(me), Some(Presence::Online));

        state.handle_logic_event(LogicEvent::PeerLeft {
            document_id: space_id.document_id(),
            user_id: me,
        });
        assert!(drain(&ui).is_empty());
        assert_eq!(samples(&drain(&sync)).len(), 1, "this tab answers");
    }

    /// An ephemeral that is no presence sample is dropped without an error,
    /// and changes nothing.
    #[test]
    fn a_sample_that_does_not_decode_is_dropped() {
        let (mut state, ui, sync) = harness();
        let bob = Id::<User>::mint();
        let space_id = space_with(&mut state, &ui, &sync, vec![member(bob, "Bob")]);
        sample_from(
            &mut state,
            space_id,
            bob,
            ambre_core::Payload::<ambre_core::EphemeralBytes>::new(vec![9, 9, 9]),
        );
        assert!(drain(&ui).is_empty());
        assert!(drain(&sync).is_empty());
    }

    // --- The last owner leaving ---

    /// `member` at `role`.
    fn at(user_id: Id<User>, role: Role) -> MemberInfo {
        MemberInfo {
            role,
            ..member(user_id, "m")
        }
    }

    /// The leave of the last owner waits while the owner role passes to the
    /// first editor by identifier, then goes under the view's id. A refused
    /// hand-off fails that leave, which never left (the Murmure spaces spec).
    #[test]
    fn the_last_owner_hands_the_role_on_before_leaving() {
        let (mut state, ui, sync) = harness();
        let me = Id::<User>::from_bytes([5; 16]);
        state.set_user(me);
        let (low, high, viewer) = (
            Id::<User>::from_bytes([1; 16]),
            Id::<User>::from_bytes([2; 16]),
            Id::<User>::from_bytes([0; 16]),
        );
        let rows = vec![
            at(me, Role::Owner),
            at(high, Role::Editor),
            at(low, Role::Editor),
            at(viewer, Role::Viewer),
        ];
        let space_id = space_with(&mut state, &ui, &sync, rows);
        let document_id = space_id.document_id();
        let leave = ambre_bus::next_request();
        state.handle_logic_command(LogicCommand::Rest {
            id: leave,
            cmd: RestCommand::LeaveDocument { document_id },
        });
        let sent = drain(&sync);
        let [SyncCommand::Rest { id: change, cmd }] = sent.as_slice() else {
            panic!("one role change, got {sent:?}");
        };
        assert!(matches!(
            cmd,
            RestCommand::UpdateDocumentMemberRole { document_id: d, user_id, role: Role::Owner }
                if *d == document_id && *user_id == low
        ));
        state.handle_logic_event(LogicEvent::Rest {
            id: Some(*change),
            evt: RestEvent::MemberRoleUpdated {
                document_id,
                user_id: low,
                role: Role::Owner,
            },
        });
        assert!(matches!(
            drain(&sync).as_slice(),
            [SyncCommand::Rest { id, cmd: RestCommand::LeaveDocument { document_id: d } }]
                if *id == leave && *d == document_id
        ));
        assert!(
            drain(&ui)
                .iter()
                .all(|e| !matches!(e, UiEvent::Rest { .. }))
        );

        // A refusal answers the leave the view asked for.
        state.handle_logic_command(LogicCommand::Rest {
            id: leave,
            cmd: RestCommand::LeaveDocument { document_id },
        });
        let Some(SyncCommand::Rest { id: change, .. }) = drain(&sync).pop() else {
            panic!("a role change");
        };
        state.handle_logic_event(LogicEvent::ApiError {
            id: Some(change),
            request: ambre_bus::ApiRequest::UpdateDocumentMemberRole,
            failure: ambre_bus::ApiFailure::Relay {
                status: ambre_bus::HttpStatus::new(403),
                code: None,
            },
        });
        assert!(drain(&sync).is_empty(), "no leave");
        assert!(drain(&ui).iter().any(|e| matches!(
            e,
            UiEvent::ApiError { id: Some(id), .. } if *id == leave
        )));
    }

    /// A leave goes at once in three cases (the Murmure spaces spec). Another
    /// owner stays, no one else stays, or the document is no open space.
    #[test]
    fn a_leave_another_owner_covers_goes_at_once() {
        let me = Id::<User>::from_bytes([5; 16]);
        let other = Id::<User>::from_bytes([6; 16]);
        for rows in [
            vec![at(me, Role::Owner), at(other, Role::Owner)],
            vec![at(me, Role::Owner)],
            vec![at(me, Role::Editor), at(other, Role::Owner)],
        ] {
            let (mut state, ui, sync) = harness();
            state.set_user(me);
            let space_id = space_with(&mut state, &ui, &sync, rows);
            for document_id in [space_id.document_id(), Id::<Document>::mint()] {
                state.handle_logic_command(LogicCommand::Rest {
                    id: ambre_bus::next_request(),
                    cmd: RestCommand::LeaveDocument { document_id },
                });
                assert!(matches!(
                    drain(&sync).as_slice(),
                    [SyncCommand::Rest {
                        cmd: RestCommand::LeaveDocument { .. },
                        ..
                    }]
                ));
            }
        }
    }

    /// The relay's notice that this user left a space, or lost it, closes the
    /// space and tells the view once. Another user's removal, or a document
    /// no space holds, changes nothing (the Murmure spaces spec).
    #[test]
    fn a_space_the_user_lost_closes_and_the_view_learns_it() {
        let (mut state, ui, sync) = harness();
        let me = Id::<User>::mint();
        state.set_user(me);
        let space_id = created_space(&mut state, &ui, &sync);
        let revoked = |document_id, revoked_user_id| LogicEvent::MemberRevoked {
            document_id,
            revoked_user_id,
        };
        state.handle_logic_event(revoked(space_id.document_id(), Id::<User>::mint()));
        state.handle_logic_event(revoked(Id::<Document>::mint(), me));
        assert!(state.rt().space(space_id).is_some(), "kept");
        assert!(drain(&sync).is_empty());
        let _ = drain(&ui);

        state.handle_logic_event(revoked(space_id.document_id(), me));
        assert!(state.rt().space(space_id).is_none());
        assert!(matches!(
            drain(&sync).as_slice(),
            [SyncCommand::CloseDocument { document_id }] if *document_id == space_id.document_id()
        ));
        let left = |shown: &[UiEvent]| {
            shown
                .iter()
                .filter(|e| {
                    matches!(e, UiEvent::Space { space_id: s, event: SpaceEvent::Left } if *s == space_id)
                })
                .count()
        };
        assert_eq!(left(&drain(&ui)), 1);
        state.handle_logic_event(revoked(space_id.document_id(), me));
        assert_eq!(left(&drain(&ui)), 0, "once");
    }
}
