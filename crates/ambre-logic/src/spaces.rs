// ambre-logic/src/spaces.rs
// ============================================================================
// Murmure spaces: the space document and its catalogue, the creation of a
// public channel, and the self-join (the Murmure spaces spec). A space is one
// document, never an entry of `docs`. No key reaches this thread: the
// catalogue key travels sealed, and only the Sync worker opens it.
// ============================================================================

use super::*;
use ambre_bus::{
    ChannelId, PublicChannelDraft, SealedChannelKey, SpaceCommand, SpaceEvent, SpaceId, Timestamp,
};
use ambre_murmure::{ChannelState, ChannelType, Listing, SpaceState};

/// An open space, and the role the relay gave this user on its document.
pub(crate) struct OpenSpace {
    state: SpaceState,
    role: Role,
}

/// A public channel the relay is creating, until it answers.
pub(crate) struct Creating {
    space_id: SpaceId,
    channel: ChannelState,
    draft: PublicChannelDraft,
}

impl LogicState {
    /// Create a space: one new document, whose creator the relay makes its
    /// owner on the first upload. Its whole content leaves once the Sync
    /// worker holds the session.
    pub(crate) fn create_space(&mut self, space_id: SpaceId, name: &str, now: Timestamp) {
        if self.spaces.contains_key(&space_id) || self.space_opening.contains_key(&space_id) {
            return;
        }
        let state = match SpaceState::create(space_id, name, now) {
            Ok(state) => state,
            Err(error) => return self.space_open_failed(space_id, error.to_string()),
        };
        match state.export_all() {
            Ok(export) => {
                self.unsent.insert(state.document_id(), export);
            }
            Err(error) => return self.space_open_failed(space_id, error.to_string()),
        }
        let document_id = state.document_id();
        self.spaces.insert(
            space_id,
            OpenSpace {
                state,
                role: Role::Owner,
            },
        );
        self.send_to_sync(SyncCommand::OpenDocument {
            document_id,
            doc_type: DocType::Murmure,
        });
        self.show_catalogue(space_id);
    }

    /// Open a space the relay holds. Its payloads wait until the document is
    /// ready, then open it whole.
    pub(crate) fn open_space(&mut self, space_id: SpaceId) {
        if self.spaces.contains_key(&space_id) || self.space_opening.contains_key(&space_id) {
            return;
        }
        self.space_opening.insert(space_id, Vec::new());
        self.send_to_sync(SyncCommand::OpenDocument {
            document_id: space_id.document_id(),
            doc_type: DocType::Murmure,
        });
    }

    /// Drop a space, open or opening, with the channels it was creating.
    pub(crate) fn close_space(&mut self, space_id: SpaceId) {
        let held = self.spaces.remove(&space_id).is_some();
        let opening = self.space_opening.remove(&space_id).is_some();
        self.creating.retain(|_, c| c.space_id != space_id);
        self.joining.retain(|_, s| *s != space_id);
        self.unsent.remove(&space_id.document_id());
        if held || opening {
            self.send_to_sync(SyncCommand::CloseDocument {
                document_id: space_id.document_id(),
            });
        }
    }

    /// Send the whole content of a document this client created, once the
    /// Sync worker holds its session: the first update creates it on the
    /// relay.
    pub(crate) fn flush_unsent(&mut self, document_id: DocumentId) {
        if let Some(payload) = self.unsent.remove(&document_id) {
            self.send_to_sync(SyncCommand::SendUpdate {
                document_id,
                payload,
                summary: None,
            });
        }
    }

    /// The document of a space is ready. Returns `false` when no space asked
    /// for it.
    pub(crate) fn space_document_ready(&mut self, document_id: DocumentId, role: Role) -> bool {
        let space_id = SpaceId::from_document(document_id);
        if let Some(space) = self.spaces.get_mut(&space_id) {
            space.role = role;
            return true;
        }
        let Some(payloads) = self.space_opening.remove(&space_id) else {
            return false;
        };
        let borrowed: Vec<&[u8]> = payloads.iter().map(Vec::as_slice).collect();
        match SpaceState::open(space_id, &borrowed) {
            Ok(state) => {
                self.spaces.insert(space_id, OpenSpace { state, role });
                self.show_catalogue(space_id);
            }
            Err(error) => {
                self.send_to_sync(SyncCommand::CloseDocument { document_id });
                self.space_open_failed(space_id, error.to_string());
            }
        }
        true
    }

    /// The document of a space could not open. Returns `false` when no space
    /// asked for it.
    pub(crate) fn space_document_failed(&mut self, document_id: DocumentId, message: &str) -> bool {
        let space_id = SpaceId::from_document(document_id);
        if self.space_opening.remove(&space_id).is_none() && !self.spaces.contains_key(&space_id) {
            return false;
        }
        self.space_open_failed(space_id, message.to_owned());
        true
    }

    fn space_open_failed(&self, space_id: SpaceId, message: String) {
        tracing::warn!("Logic: space {space_id} failed to open: {message}");
        self.send_to_ui(UiEvent::Space {
            space_id,
            event: SpaceEvent::OpenFailed { message },
        });
    }

    /// Apply remote payloads to the space that holds `document_id`. Returns
    /// `false` when no space, open or opening, holds it.
    pub(crate) fn apply_space_remote(
        &mut self,
        document_id: DocumentId,
        payloads: &[&[u8]],
    ) -> bool {
        let space_id = SpaceId::from_document(document_id);
        if let Some(buffer) = self.space_opening.get_mut(&space_id) {
            buffer.extend(payloads.iter().map(|p| p.to_vec()));
            return true;
        }
        let Some(space) = self.spaces.get_mut(&space_id) else {
            return false;
        };
        for payload in payloads {
            if let Err(error) = space.state.apply_remote(payload) {
                tracing::warn!("Logic: delta for space {space_id} refused: {error}");
            }
        }
        self.show_catalogue(space_id);
        true
    }

    /// The snapshot of a space document, for the relay's compaction, with the
    /// relay's number of the last delta it folds in (`SRV-083`).
    pub(crate) fn space_snapshot(&self, document_id: DocumentId) -> Option<(Vec<u8>, u64)> {
        let space = self.spaces.get(&SpaceId::from_document(document_id))?;
        let payload = space
            .state
            .snapshot()
            .map_err(|e| tracing::warn!("Logic: snapshot of space {document_id} failed: {e}"))
            .ok()?;
        let covered = self
            .channel_ledgers
            .get(&document_id)
            .map_or(0, crate::snapshot_check::SeqLedger::contiguous_end);
        Some((payload, covered))
    }

    /// Hand the view the name, the categories and the catalogue of a space.
    fn show_catalogue(&self, space_id: SpaceId) {
        let Some(space) = self.spaces.get(&space_id) else {
            return;
        };
        self.send_to_ui(UiEvent::Space {
            space_id,
            event: SpaceEvent::Catalogue {
                name: space.state.name(),
                categories: space.state.categories(),
                channels: space.state.catalogue(),
            },
        });
    }

    fn channel_failed(&self, space_id: SpaceId, channel_id: ChannelId, message: String) {
        tracing::warn!("Logic: channel {channel_id} of space {space_id}: {message}");
        self.send_to_ui(UiEvent::Space {
            space_id,
            event: SpaceEvent::ChannelFailed {
                channel_id,
                message,
            },
        });
    }

    /// Run one command on an open space (`BUS-027`).
    pub(crate) fn handle_space_command(&mut self, space_id: SpaceId, command: SpaceCommand) {
        let Some(space) = self.spaces.get_mut(&space_id) else {
            tracing::warn!("Logic: command for space {space_id}, which is not open; dropped");
            return;
        };
        let admin = space.role != Role::Viewer;
        match command {
            SpaceCommand::AddCategory {
                category_id,
                name,
                position,
            } => {
                // A viewer changes no catalogue. The relay refuses the write
                // too: this check only spares the round trip.
                if !admin {
                    tracing::warn!("Logic: a viewer adds no category to space {space_id}");
                    return;
                }
                match space.state.put_category(category_id, &name, position) {
                    Ok(delta) => {
                        self.send_to_sync(SyncCommand::SendUpdate {
                            document_id: space_id.document_id(),
                            payload: delta,
                            summary: None,
                        });
                        self.show_catalogue(space_id);
                    }
                    Err(error) => tracing::warn!("Logic: category refused: {error}"),
                }
            }
            SpaceCommand::CreatePublicChannel { draft, now } => {
                let channel_id = draft.channel_id;
                if !admin {
                    return self.channel_failed(
                        space_id,
                        channel_id,
                        "a viewer of the space creates no channel".into(),
                    );
                }
                let channel = match ChannelState::create(
                    channel_id,
                    ChannelType::Public,
                    Some(&draft.name),
                    now,
                ) {
                    Ok(channel) => channel,
                    Err(error) => {
                        return self.channel_failed(space_id, channel_id, error.to_string());
                    }
                };
                self.send_to_sync(SyncCommand::CreatePublicChannel {
                    channel_id,
                    space_id,
                    meta: channel.meta_id(),
                    segment: channel.active_id(),
                });
                self.creating.insert(
                    channel_id,
                    Creating {
                        space_id,
                        channel,
                        draft,
                    },
                );
            }
            SpaceCommand::JoinChannel { channel_id } => {
                // The key the catalogue holds, sealed: the Sync worker opens
                // it. The join writes no catalogue.
                match space.state.sealed_key(channel_id) {
                    Ok(key) => {
                        self.joining.insert(channel_id, space_id);
                        self.send_to_sync(SyncCommand::JoinChannel {
                            channel_id,
                            space_id,
                            key,
                        });
                    }
                    Err(error) => self.channel_failed(space_id, channel_id, error.to_string()),
                }
            }
        }
    }

    /// The relay holds the new channel: list it in the catalogue with its
    /// sealed key, and send the first content of its two documents.
    pub(crate) fn public_channel_created(&mut self, channel_id: ChannelId, key: SealedChannelKey) {
        let Some(Creating {
            space_id,
            channel,
            draft,
        }) = self.creating.remove(&channel_id)
        else {
            return;
        };
        let Some(space) = self.spaces.get_mut(&space_id) else {
            return;
        };
        let listing = Listing {
            topic: draft.topic.as_ref(),
            category: draft.category,
            position: draft.position,
            key,
        };
        let delta = match space.state.list_channel(&channel, &listing) {
            Ok(delta) => delta,
            Err(error) => return self.channel_failed(space_id, channel_id, error.to_string()),
        };
        self.send_to_sync(SyncCommand::SendUpdate {
            document_id: space_id.document_id(),
            payload: delta,
            summary: None,
        });
        for document_id in [
            channel.meta_id().document_id(),
            channel.active_id().document_id(),
        ] {
            match channel.export_all(document_id) {
                Ok(export) => {
                    self.unsent.insert(document_id, export);
                }
                Err(error) => tracing::warn!("Logic: export of {document_id} failed: {error}"),
            }
            self.send_to_sync(SyncCommand::OpenDocument {
                document_id,
                doc_type: DocType::Murmure,
            });
        }
        self.channels.insert(channel_id, channel);
        self.send_to_ui(UiEvent::Space {
            space_id,
            event: SpaceEvent::ChannelCreated { channel_id },
        });
        self.show_catalogue(space_id);
    }

    /// The relay made this user a member of the channel: open it.
    pub(crate) fn channel_joined(&mut self, channel_id: ChannelId) {
        let Some(space_id) = self.joining.remove(&channel_id) else {
            return;
        };
        self.send_to_ui(UiEvent::Space {
            space_id,
            event: SpaceEvent::Joined { channel_id },
        });
        self.open_channel(channel_id);
    }

    /// A creation or a join failed on the relay, which changed nothing.
    pub(crate) fn channel_request_failed(&mut self, channel_id: ChannelId, message: String) {
        let space_id = match (
            self.creating.remove(&channel_id),
            self.joining.remove(&channel_id),
        ) {
            (Some(creating), _) => creating.space_id,
            (None, Some(space_id)) => space_id,
            (None, None) => return,
        };
        self.channel_failed(space_id, channel_id, message);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ambre_bus::{CategoryId, Role, Topic};

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

    fn ready(state: &mut LogicState, document_id: DocumentId, role: Role) {
        state.handle_logic_event(LogicEvent::DocumentReady { document_id, role });
    }

    fn sealed(byte: u8) -> SealedChannelKey {
        SealedChannelKey::from_bytes(vec![byte; 72])
    }

    fn draft(name: &str) -> PublicChannelDraft {
        PublicChannelDraft {
            channel_id: ChannelId::new(),
            name: name.to_owned(),
            topic: Topic::new("the weekly figures"),
            category: None,
            position: 1,
        }
    }

    /// The catalogue of the last catalogue event the view got.
    fn last_catalogue(
        shown: &[UiEvent],
    ) -> Option<(Option<String>, Vec<ambre_bus::CatalogueEntry>)> {
        shown.iter().rev().find_map(|e| match e {
            UiEvent::Space {
                event: SpaceEvent::Catalogue { name, channels, .. },
                ..
            } => Some((name.clone(), channels.clone())),
            _ => None,
        })
    }

    fn updates_of(sent: &[SyncCommand], document: DocumentId) -> usize {
        sent.iter()
            .filter(|c| matches!(c, SyncCommand::SendUpdate { document_id, .. } if *document_id == document))
            .count()
    }

    /// A space created here, its document ready, and the channels drained.
    fn created_space(
        state: &mut LogicState,
        ui: &Receiver<UiEvent>,
        sync: &Receiver<SyncCommand>,
    ) -> SpaceId {
        let space_id = SpaceId::new();
        state.handle_logic_command(LogicCommand::CreateSpace {
            space_id,
            name: "Acme".to_owned(),
            now: NOW,
        });
        ready(state, space_id.document_id(), Role::Owner);
        let _ = (drain(ui), drain(sync));
        space_id
    }

    // --- The client side of a new space ---

    /// A new space shows at once. Its whole document is its first update,
    /// sent once the Sync worker holds the session. That upload makes its
    /// creator the owner on the relay.
    #[test]
    fn a_new_space_shows_at_once_and_uploads_whole_once_its_session_is_ready() {
        let (mut state, ui, sync) = harness();
        let space_id = SpaceId::new();
        state.handle_logic_command(LogicCommand::CreateSpace {
            space_id,
            name: "Acme".to_owned(),
            now: NOW,
        });
        let sent = drain(&sync);
        assert!(matches!(
            sent.as_slice(),
            [SyncCommand::OpenDocument { document_id, doc_type: DocType::Murmure }]
                if *document_id == space_id.document_id()
        ));
        assert_eq!(
            last_catalogue(&drain(&ui)),
            Some((Some("Acme".to_owned()), Vec::new()))
        );

        ready(&mut state, space_id.document_id(), Role::Owner);
        let sent = drain(&sync);
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
        let reopened = SpaceState::open(space_id, &[payload]).expect("the upload is the space");
        assert_eq!(reopened.name().as_deref(), Some("Acme"));
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
        let space_id = SpaceId::new();
        let stored = SpaceState::create(space_id, "Acme", NOW).expect("create");
        state.handle_logic_command(LogicCommand::OpenSpace { space_id });
        let _ = drain(&sync);
        state.handle_logic_event(LogicEvent::RemoteSnapshot {
            document_id: space_id.document_id(),
            payload: stored.snapshot().expect("snapshot"),
        });
        assert!(
            drain(&ui).is_empty(),
            "nothing shows before the ready signal"
        );
        ready(&mut state, space_id.document_id(), Role::Editor);
        assert_eq!(
            last_catalogue(&drain(&ui)),
            Some((Some("Acme".to_owned()), Vec::new()))
        );
        assert!(drain(&sync).is_empty(), "an opened space uploads nothing");
    }

    #[test]
    fn a_space_with_nothing_readable_fails_to_open() {
        let (mut state, ui, sync) = harness();
        let space_id = SpaceId::new();
        state.handle_logic_command(LogicCommand::OpenSpace { space_id });
        let _ = drain(&sync);
        ready(&mut state, space_id.document_id(), Role::Editor);
        assert!(matches!(
            drain(&ui).as_slice(),
            [UiEvent::Space {
                event: SpaceEvent::OpenFailed { .. },
                ..
            }]
        ));
        assert!(matches!(
            drain(&sync).as_slice(),
            [SyncCommand::CloseDocument { .. }]
        ));
        assert!(state.spaces.is_empty());
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
            SyncCommand::CreatePublicChannel {
                channel_id: asked,
                space_id: of,
                meta,
                segment,
            },
        ] = sent.as_slice()
        else {
            panic!("one creation, got {sent:?}");
        };
        assert_eq!((*asked, *of), (channel_id, space_id));
        let (meta, segment) = (meta.document_id(), segment.document_id());

        state.handle_logic_event(LogicEvent::ChannelCreated {
            channel_id,
            key: sealed(7),
        });
        let sent = drain(&sync);
        assert_eq!(
            updates_of(&sent, space_id.document_id()),
            1,
            "one catalogue write"
        );
        let opened: Vec<DocumentId> = sent
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
                entry.topic.as_ref().map(Topic::as_str)
            ),
            (channel_id, "budget", Some("the weekly figures"))
        );
        let space = &state.spaces[&space_id].state;
        assert_eq!(space.sealed_key(channel_id), Ok(sealed(7)));

        ready(&mut state, meta, Role::Owner);
        ready(&mut state, segment, Role::Owner);
        let sent = drain(&sync);
        assert_eq!(
            (updates_of(&sent, meta), updates_of(&sent, segment)),
            (1, 1)
        );
        assert!(state.channels.contains_key(&channel_id));
    }

    /// A viewer changes no catalogue, so its client asks the relay for
    /// nothing. The relay refuses the write in any case.
    #[test]
    fn a_viewer_creates_no_channel_and_adds_no_category() {
        let (mut state, ui, sync) = harness();
        let space_id = SpaceId::new();
        let stored = SpaceState::create(space_id, "Acme", NOW).expect("create");
        state.handle_logic_command(LogicCommand::OpenSpace { space_id });
        state.handle_logic_event(LogicEvent::RemoteSnapshot {
            document_id: space_id.document_id(),
            payload: stored.snapshot().expect("snapshot"),
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
                category_id: CategoryId::new(),
                name: "Teams".to_owned(),
                position: 0,
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
        assert!(state.spaces[&space_id].state.catalogue().is_empty());
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
            message: "403".to_owned(),
        });
        assert!(matches!(
            drain(&ui).as_slice(),
            [UiEvent::Space { event: SpaceEvent::ChannelFailed { channel_id: id, .. }, .. }]
                if *id == channel_id
        ));
        assert!(drain(&sync).is_empty());
        assert!(state.spaces[&space_id].state.catalogue().is_empty());
        // A late answer for a creation given up lists nothing either.
        state.handle_logic_event(LogicEvent::ChannelCreated {
            channel_id,
            key: sealed(1),
        });
        assert!(drain(&sync).is_empty());
    }

    // --- A self-join ---

    /// Bob's client holds a space where Alice listed a channel. The join hands
    /// the Sync worker the sealed key as the catalogue holds it. It writes no
    /// catalogue, and opens the channel once the relay accepted the member.
    #[test]
    fn a_self_join_hands_the_sealed_key_to_sync_and_writes_no_catalogue() {
        let space_id = SpaceId::new();
        let mut alice = SpaceState::create(space_id, "Acme", NOW).expect("create");
        let listed =
            ChannelState::create(ChannelId::new(), ChannelType::Public, Some("general"), NOW)
                .expect("channel");
        alice
            .list_channel(
                &listed,
                &Listing {
                    topic: None,
                    category: None,
                    position: 0,
                    key: sealed(9),
                },
            )
            .expect("list");
        let channel_id = listed.channel_id();

        let (mut bob, ui, sync) = harness();
        bob.handle_logic_command(LogicCommand::OpenSpace { space_id });
        bob.handle_logic_event(LogicEvent::RemoteSnapshot {
            document_id: space_id.document_id(),
            payload: alice.snapshot().expect("snapshot"),
        });
        ready(&mut bob, space_id.document_id(), Role::Editor);
        let _ = (drain(&ui), drain(&sync));
        let before = bob.spaces[&space_id].state.version();

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
            bob.spaces[&space_id].state.version(),
            before,
            "no catalogue write"
        );

        bob.handle_logic_event(LogicEvent::ChannelJoined { channel_id });
        assert!(matches!(
            drain(&ui).as_slice(),
            [UiEvent::Space { event: SpaceEvent::Joined { channel_id: id }, .. }] if *id == channel_id
        ));
        let sent = drain(&sync);
        assert!(matches!(
            sent.as_slice(),
            [SyncCommand::OpenChannel { channel_id: id }] if *id == channel_id
        ));
        assert_eq!(updates_of(&sent, space_id.document_id()), 0);
    }

    #[test]
    fn joining_a_channel_the_catalogue_does_not_list_asks_nothing() {
        let (mut state, ui, sync) = harness();
        let space_id = created_space(&mut state, &ui, &sync);
        state.handle_logic_command(LogicCommand::Space {
            space_id,
            command: SpaceCommand::JoinChannel {
                channel_id: ChannelId::new(),
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
        let space_id = SpaceId::new();
        let mut alice = SpaceState::create(space_id, "Acme", NOW).expect("create");
        let first = alice.export_all().expect("export");
        let (mut bob, ui, sync) = harness();
        bob.handle_logic_command(LogicCommand::OpenSpace { space_id });
        bob.handle_logic_event(LogicEvent::RemoteUpdate {
            document_id: space_id.document_id(),
            payload: first,
        });
        ready(&mut bob, space_id.document_id(), Role::Editor);
        let _ = (drain(&ui), drain(&sync));

        let listed =
            ChannelState::create(ChannelId::new(), ChannelType::Public, Some("general"), NOW)
                .expect("channel");
        let delta = alice
            .list_channel(
                &listed,
                &Listing {
                    topic: None,
                    category: None,
                    position: 0,
                    key: sealed(2),
                },
            )
            .expect("list");
        bob.handle_logic_event(LogicEvent::RemoteUpdate {
            document_id: space_id.document_id(),
            payload: delta,
        });
        let (_, catalogue) = last_catalogue(&drain(&ui)).expect("the catalogue");
        assert_eq!(catalogue.len(), 1);
        assert_eq!(catalogue[0].name, "general");
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
        assert!(state.spaces.is_empty());
        assert!(!state.apply_space_remote(space_id.document_id(), &[&[]]));
    }
}
