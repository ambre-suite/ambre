// ambre-logic/src/spaces.rs
// ============================================================================
// Murmure spaces (the Murmure spaces and channels specs). This holds the
// space document and its catalogue, the creation and the delete of a
// channel, and the self-join. A space is one document, never an entry of
// `docs`. No key reaches this thread: the
// catalogue key travels sealed, and only the Sync worker opens it.
// ============================================================================

use super::*;
use ambre_bus::{
    ChannelId, MemberInfo, PrivateChannelDraft, PublicChannelDraft, SealedChannelKey, SpaceCommand,
    SpaceEvent, SpaceId, Timestamp,
};
use ambre_core::Name;
use ambre_murmure::{
    ChannelState, ChannelType, Listing, Roster, SpaceState, is_presence_sample, presence_sample,
};

/// An open space, the role the relay gave this user on its document, and
/// its members with their presence.
pub(crate) struct OpenSpace {
    state: SpaceState,
    role: Role,
    roster: Roster,
    /// The catalogue entries as they stood when this client asked the relay
    /// which channels it holds. Only these may be unlisted on its answer:
    /// an entry listed since names a channel the relay held by then.
    checked: Option<Vec<ChannelId>>,
    /// The role of each member, as Sync's last listing gave them: the last
    /// owner hands the role on before leaving (the Murmure spaces spec).
    roles: Vec<(UserId, Role)>,
}

/// A space on its way to open, with what arrived before its document was
/// ready.
#[derive(Default)]
pub(crate) struct Opening {
    payloads: Vec<Vec<u8>>,
    /// The last member rows Sync handed on, applied once the space is open.
    members: Option<Vec<MemberInfo>>,
}

/// What a new channel comes with: a public one its catalogue entry, a
/// private one its name.
enum NewChannel {
    Public(PublicChannelDraft),
    Private(Name),
}

/// A channel the relay is creating, until it answers.
pub(crate) struct Creating {
    space_id: SpaceId,
    channel: ChannelState,
    /// What the catalogue will list, for a public channel. `None` for a
    /// private one, which no catalogue lists.
    listing: Option<PublicChannelDraft>,
}

impl LogicState {
    /// Create a space: one new document, whose creator the relay makes its
    /// owner on the first upload. It is written again under its PeerID once
    /// its identity lands, and its whole content leaves once the Sync worker
    /// holds the document.
    pub(crate) fn create_space(&mut self, space_id: SpaceId, name: &str, now: Timestamp) {
        if self.spaces.contains_key(&space_id) || self.space_opening.contains_key(&space_id) {
            return;
        }
        let Ok(name) = Name::new(name) else {
            return self.refuse_long_name(name, ambre_bus::ApiRequest::CreateSpace);
        };
        let state = match SpaceState::create(space_id, &name, now) {
            Ok(state) => state,
            Err(error) => return self.space_open_failed(space_id, error.to_string()),
        };
        self.spaces.insert(
            space_id,
            OpenSpace {
                state,
                role: Role::Owner,
                roster: Roster::new(self.my_user_id),
                checked: None,
                roles: Vec::new(),
            },
        );
        // Sync tells the relay it is a space, so each member lists it.
        self.send_to_sync(SyncCommand::CreateSpace { space_id });
        self.show_catalogue(space_id);
    }

    /// Open a space the relay holds. Its payloads wait until the document is
    /// ready, then open it whole.
    pub(crate) fn open_space(&mut self, space_id: SpaceId) {
        if self.spaces.contains_key(&space_id) || self.space_opening.contains_key(&space_id) {
            return;
        }
        self.space_opening.insert(space_id, Opening::default());
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
        self.deleting.retain(|_, s| *s != space_id);
        self.leaving.retain(|_, s| *s != space_id);
        self.forget_murmure_session(space_id.document_id());
        if held || opening {
            self.send_to_sync(SyncCommand::CloseDocument {
                document_id: space_id.document_id(),
            });
        }
    }

    /// Send the whole content of a document this client created, once the
    /// Sync worker holds it: the first update creates it on the relay. The
    /// commands that waited on it follow.
    pub(crate) fn flush_unsent(&mut self, document_id: DocumentId) {
        if let Some(payload) = self.unsent.remove(&document_id) {
            self.send_to_sync(SyncCommand::SendUpdate {
                document_id,
                payload: payload.into(),
                summary: None,
            });
            self.release_waiting(document_id);
        }
    }

    /// Adopt a session identity on the open space whose document it names.
    /// `None` when no open space holds that document.
    pub(crate) fn adopt_space_session(
        &mut self,
        document_id: DocumentId,
        peer: ambre_core::PeerId,
        entry: &ambre_core::PeerEntry,
    ) -> Option<Result<Option<Vec<u8>>, ambre_murmure::MurmureError>> {
        let space = self.spaces.get_mut(&SpaceId::from_document(document_id))?;
        Some(space.state.adopt_session(peer, entry.to_bytes()))
    }

    /// The document of a space is ready. Returns `false` when no space asked
    /// for it.
    pub(crate) fn space_document_ready(&mut self, document_id: DocumentId, role: Role) -> bool {
        let space_id = SpaceId::from_document(document_id);
        if let Some(space) = self.spaces.get_mut(&space_id) {
            space.role = role;
            self.space_live(space_id);
            return true;
        }
        let Some(opening) = self.space_opening.remove(&space_id) else {
            return false;
        };
        let borrowed: Vec<&[u8]> = opening.payloads.iter().map(Vec::as_slice).collect();
        match SpaceState::open(space_id, &borrowed) {
            Ok(state) => {
                let roster = Roster::new(self.my_user_id);
                self.spaces.insert(
                    space_id,
                    OpenSpace {
                        state,
                        role,
                        roster,
                        checked: None,
                        roles: Vec::new(),
                    },
                );
                // The identity landed while the space was opening.
                self.settle_murmure_session(document_id);
                self.show_catalogue(space_id);
                self.space_live(space_id);
                if let Some(members) = opening.members {
                    self.set_space_rows(space_id, members);
                }
            }
            Err(error) => {
                self.forget_murmure_session(document_id);
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
        self.forget_murmure_session(document_id);
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
        if let Some(opening) = self.space_opening.get_mut(&space_id) {
            opening.payloads.extend(payloads.iter().map(|p| p.to_vec()));
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
    pub(crate) fn space_snapshot(
        &self,
        document_id: DocumentId,
    ) -> Option<(ambre_core::SnapshotBytes, Seq)> {
        let space = self.spaces.get(&SpaceId::from_document(document_id))?;
        let payload = space
            .state
            .snapshot()
            .map_err(|e| tracing::warn!("Logic: snapshot of space {document_id} failed: {e}"))
            .ok()?;
        let covered = self
            .channel_ledgers
            .get(&document_id)
            .map_or(Seq::ZERO, crate::snapshot_check::SeqLedger::contiguous_end);
        Some((ambre_core::SnapshotBytes::new(payload), covered))
    }

    /// Hand the view the name, the categories and the catalogue of a space.
    fn show_catalogue(&self, space_id: SpaceId) {
        let Some(space) = self.spaces.get(&space_id) else {
            return;
        };
        let channels = space.state.catalogue();
        // An admin re-keys each public channel once the space key rotates
        // (the Murmure spaces spec). Sync reads which entries an older space
        // key sealed, so it holds the catalogue as it now stands.
        if space.role != Role::Viewer && !channels.is_empty() {
            let sealed = channels
                .iter()
                .filter_map(|entry| {
                    let key = space.state.sealed_key(entry.channel_id).ok()?;
                    Some((entry.channel_id, key))
                })
                .collect();
            self.send_to_sync(SyncCommand::RotateSpace {
                space_id,
                channels: sealed,
            });
        }
        self.send_to_ui(UiEvent::Space {
            space_id,
            event: SpaceEvent::Catalogue {
                name: space.state.name(),
                categories: space.state.categories(),
                channels,
            },
        });
    }

    /// Sync re-keyed a public channel after a rotation of the space. Its
    /// catalogue entry takes the new sealed key, and nothing else changes
    /// (the Murmure spaces spec).
    pub(crate) fn channel_resealed(
        &mut self,
        space_id: SpaceId,
        channel_id: ChannelId,
        key: &SealedChannelKey,
    ) {
        let Some(space) = self.spaces.get_mut(&space_id) else {
            return;
        };
        match space.state.reseal_channel(channel_id, key) {
            Ok(Some(payload)) => {
                self.send_to_sync(SyncCommand::SendUpdate {
                    document_id: space_id.document_id(),
                    payload: payload.into(),
                    summary: None,
                });
                self.show_catalogue(space_id);
            }
            Ok(None) => {}
            Err(error) => tracing::warn!("Logic: resealing {channel_id} failed: {error}"),
        }
    }

    /// Tell the user why a name no [`Name`] holds was refused, before
    /// anything is written (the Murmure channels spec).
    pub(crate) fn refuse_long_name(&self, name: &str, request: ambre_bus::ApiRequest) {
        tracing::warn!(
            "Logic: {request} refused a name of {} characters",
            name.chars().count()
        );
        self.send_to_ui(UiEvent::ApiError {
            id: None,
            request,
            failure: ambre_bus::ApiFailure::Local,
            message: ambre_core::NAME_TOO_LONG.to_owned(),
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
        let Some(space) = self.spaces.get(&space_id) else {
            tracing::warn!("Logic: command for space {space_id}, which is not open; dropped");
            return;
        };
        // A write before the document may be written waits: under any other
        // PeerID it would never travel (`AUTHOR-026`, `AUTHOR-029`). Creating
        // a public channel and deleting one write the catalogue on the relay's
        // answer, so they wait too. A refused write would leave the relay
        // holding a channel no catalogue lists. A viewer's write is refused
        // at once, below, and waits on nothing.
        let document_id = space_id.document_id();
        let writes = matches!(
            command,
            SpaceCommand::AddCategory { .. }
                | SpaceCommand::CreatePublicChannel { .. }
                | SpaceCommand::DeleteChannel { .. }
        );
        if writes
            && space.role != Role::Viewer
            && !self.murmure_writable(document_id, space.state.identified())
        {
            return self.wait_for_session(
                document_id,
                crate::channels::HeldCommand::Space(space_id, command),
            );
        }
        let Some(space) = self.spaces.get_mut(&space_id) else {
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
                let Ok(name) = Name::new(name.as_str()) else {
                    return self.refuse_long_name(&name, ambre_bus::ApiRequest::AddCategory);
                };
                match space.state.put_category(category_id, &name, position) {
                    Ok(delta) => {
                        self.send_to_sync(SyncCommand::SendUpdate {
                            document_id: space_id.document_id(),
                            payload: delta.into(),
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
                let Ok(name) = Name::new(draft.name.as_str()) else {
                    return self
                        .refuse_long_name(&draft.name, ambre_bus::ApiRequest::CreatePublicChannel);
                };
                let channel =
                    match ChannelState::create(channel_id, ChannelType::Public, Some(&name), now) {
                        Ok(channel) => channel,
                        Err(error) => {
                            return self.channel_failed(space_id, channel_id, error.to_string());
                        }
                    };
                self.create_channel(space_id, channel, NewChannel::Public(draft));
            }
            SpaceCommand::CreatePrivateChannel { draft, now } => {
                // Any member of the space may: the channel is listed nowhere.
                let PrivateChannelDraft { channel_id, name } = draft;
                let Ok(name) = Name::new(name.as_str()) else {
                    return self
                        .refuse_long_name(&name, ambre_bus::ApiRequest::CreatePrivateChannel);
                };
                match ChannelState::create(channel_id, ChannelType::Private, Some(&name), now) {
                    Ok(channel) => {
                        self.create_channel(space_id, channel, NewChannel::Private(name));
                    }
                    Err(error) => self.channel_failed(space_id, channel_id, error.to_string()),
                }
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
            SpaceCommand::DeleteChannel { channel_id } => {
                // Only the relay knows who owns the channel. Nothing changes
                // here before its answer, so a refusal leaves the channel
                // whole.
                self.deleting.insert(channel_id, space_id);
                self.send_to_sync(SyncCommand::DeleteChannel { channel_id });
            }
            SpaceCommand::LeaveChannel { channel_id } => {
                // As with a delete, nothing changes before the relay answers.
                self.leaving.insert(channel_id, space_id);
                self.send_to_sync(SyncCommand::LeaveChannel { channel_id });
            }
            SpaceCommand::RefreshPresence { now } => {
                let expired = space.roster.tick(now);
                self.send_presence(space_id);
                if expired {
                    self.show_members(space_id);
                }
            }
        }
    }

    // --- The members of a space, and their presence ---

    /// The document of a space is live (the Murmure members spec). Show this
    /// user online to the other members. The member rows come from Sync,
    /// which reads them once its subscription holds (`SYNC-150`).
    fn space_live(&mut self, space_id: SpaceId) {
        self.send_presence(space_id);
        self.check_catalogue(space_id);
    }

    /// Ask the relay which channels it holds as public in the space. A space
    /// admin then drops an entry that outlived its channel (the Murmure
    /// spaces spec). A viewer writes no catalogue, and an empty one has
    /// nothing to drop.
    fn check_catalogue(&mut self, space_id: SpaceId) {
        let Some(space) = self.spaces.get_mut(&space_id) else {
            return;
        };
        let listed: Vec<ChannelId> = space
            .state
            .catalogue()
            .iter()
            .map(|entry| entry.channel_id)
            .collect();
        if space.role == Role::Viewer || listed.is_empty() {
            return;
        }
        space.checked = Some(listed);
        self.send_to_sync(SyncCommand::ListSpaceChannels { space_id });
    }

    /// The relay's channels of the space. Unlist each entry checked when
    /// this client asked and absent from the answer: its channel is gone.
    pub(crate) fn space_channels(&mut self, space_id: SpaceId, channels: &[ChannelId]) {
        let Some(space) = self.spaces.get_mut(&space_id) else {
            return;
        };
        let Some(checked) = space.checked.take() else {
            return;
        };
        let mut deltas = Vec::new();
        for channel_id in checked.into_iter().filter(|id| !channels.contains(id)) {
            match space.state.unlist_channel(channel_id) {
                Ok(Some(delta)) => deltas.push(delta),
                Ok(None) => {}
                Err(error) => tracing::warn!("Logic: unlisting {channel_id} failed: {error}"),
            }
        }
        if deltas.is_empty() {
            return;
        }
        for payload in deltas {
            self.send_to_sync(SyncCommand::SendUpdate {
                document_id: space_id.document_id(),
                payload: payload.into(),
                summary: None,
            });
        }
        self.show_catalogue(space_id);
    }

    /// Send this user's presence sample on the space document, and on no
    /// other: only the members of the space hold its key.
    fn send_presence(&self, space_id: SpaceId) {
        self.send_to_sync(SyncCommand::SendEphemeral {
            document_id: space_id.document_id(),
            payload: presence_sample().into(),
        });
    }

    /// Sync read the members of a document again and hands their rows on.
    /// Returns `false` when no space, open or opening, holds it. An open space
    /// takes the rows and asks for nothing (`SYNC-151`). An opening one keeps
    /// the last rows until it is open (the Murmure members spec).
    pub(crate) fn space_members_changed(
        &mut self,
        document_id: DocumentId,
        members: Vec<MemberInfo>,
    ) -> bool {
        let space_id = SpaceId::from_document(document_id);
        if let Some(opening) = self.space_opening.get_mut(&space_id) {
            opening.members = Some(members);
            return true;
        }
        if !self.spaces.contains_key(&space_id) {
            return false;
        }
        // Sync hands on only a reading newer than the last it held. The
        // channel keeps their order, so the last rows to arrive are newest.
        self.set_space_rows(space_id, members);
        true
    }

    /// These rows become the member list of a space.
    fn set_space_rows(&mut self, space_id: SpaceId, members: Vec<MemberInfo>) {
        let Some(space) = self.spaces.get_mut(&space_id) else {
            return;
        };
        space.roles = members.iter().map(|m| (m.user_id, m.role)).collect();
        space.roster.set_members(
            members
                .into_iter()
                .map(|member| (member.user_id, member.display_name)),
        );
        self.show_members(space_id);
    }

    /// The view asks to leave the document `document_id`. Say it is an open
    /// space, this user is its last owner, and others stay. The owner role
    /// then first passes to one of them, and the leave waits (the Murmure
    /// spaces spec). Returns whether it waits.
    ///
    /// The successor is the first editor by identifier, else the first
    /// viewer: the same member on every client that reads the same rows.
    pub(crate) fn hand_off_before_leave(
        &mut self,
        leave: RequestId,
        document_id: DocumentId,
    ) -> bool {
        let (Some(space), Some(me)) = (
            self.spaces.get(&SpaceId::from_document(document_id)),
            self.my_user_id,
        ) else {
            return false;
        };
        let owners = space.roles.iter().filter(|(_, role)| *role == Role::Owner);
        if !owners.map(|(user, _)| *user).eq([me]) {
            return false;
        }
        let first = |wanted: Role| {
            space
                .roles
                .iter()
                .filter(|(_, role)| *role == wanted)
                .map(|(user, _)| *user)
                .min_by_key(|user| *user.as_bytes())
        };
        let Some(successor) = first(Role::Editor).or_else(|| first(Role::Viewer)) else {
            return false;
        };
        let change = RequestId::next();
        self.handing_off.insert(change, (leave, document_id));
        self.send_to_sync(SyncCommand::Rest {
            id: change,
            cmd: RestCommand::UpdateDocumentMemberRole {
                document_id,
                user_id: successor,
                role: Role::Owner,
            },
        });
        true
    }

    /// The answer to a role change this thread asked for before a leave.
    /// The leave it held goes now, under the id the view gave it. Returns
    /// `false` for any other answer.
    pub(crate) fn handed_off(&mut self, id: RequestId, evt: &RestEvent) -> bool {
        let Some((leave, document_id)) = self.handing_off.remove(&id) else {
            return false;
        };
        if let RestEvent::MemberRoleUpdated { .. } = evt {
            self.send_to_sync(SyncCommand::Rest {
                id: leave,
                cmd: RestCommand::LeaveDocument { document_id },
            });
        } else {
            tracing::warn!("Logic: the hand-off before leaving {document_id} answered {evt:?}");
        }
        true
    }

    /// Hand the view the member list of a space.
    fn show_members(&self, space_id: SpaceId) {
        let Some(space) = self.spaces.get(&space_id) else {
            return;
        };
        self.send_to_ui(UiEvent::Space {
            space_id,
            event: SpaceEvent::Members {
                members: space.roster.view(),
            },
        });
    }

    /// An ephemeral on a document. Returns `false` when no space, open or
    /// opening, holds it. An ephemeral that is no presence sample is dropped
    /// without an error (`PRESENCE-013`).
    ///
    /// A member heard for the first time gets this user's sample at once, so
    /// a newcomer sees who is online without waiting a refresh period.
    pub(crate) fn space_presence_heard(
        &mut self,
        document_id: DocumentId,
        from: UserId,
        payload: &[u8],
    ) -> bool {
        let space_id = SpaceId::from_document(document_id);
        if self.space_opening.contains_key(&space_id) {
            return true;
        }
        let Some(space) = self.spaces.get_mut(&space_id) else {
            return false;
        };
        if !is_presence_sample(payload) || Some(from) == self.my_user_id {
            return true;
        }
        if space.roster.heard(from) {
            self.send_presence(space_id);
            self.show_members(space_id);
        }
        true
    }

    /// The relay says a peer stopped watching a document. Returns `false`
    /// when no space, open or opening, holds it.
    ///
    /// The departure of another connection of this user turns no one
    /// offline. This connection answers with its sample, so the other
    /// members see this user online again.
    pub(crate) fn space_peer_left(&mut self, document_id: DocumentId, user: UserId) -> bool {
        let space_id = SpaceId::from_document(document_id);
        if self.space_opening.contains_key(&space_id) {
            return true;
        }
        let Some(space) = self.spaces.get_mut(&space_id) else {
            return false;
        };
        if Some(user) == self.my_user_id {
            self.send_presence(space_id);
        } else if space.roster.left(user) {
            self.show_members(space_id);
        }
        true
    }

    /// The session names this user: every open space shows them online.
    pub(crate) fn spaces_know_me(&mut self, me: UserId) {
        for space in self.spaces.values_mut() {
            space.roster.set_me(me);
        }
    }

    /// Ask the relay for a new channel, and hold it until the relay answers.
    /// A public channel comes with its catalogue entry. A private one comes
    /// with its name, sealed beside meta so it shows before the channel opens.
    fn create_channel(&mut self, space_id: SpaceId, channel: ChannelState, kind: NewChannel) {
        let channel_id = channel.channel_id();
        let (listing, name) = match kind {
            NewChannel::Public(draft) => (Some(draft), None),
            NewChannel::Private(name) => (None, Some(name)),
        };
        self.send_to_sync(SyncCommand::CreateChannel {
            channel_id,
            space_id,
            meta: channel.meta_id(),
            segment: channel.active_id(),
            visibility: if listing.is_some() {
                ambre_core::Visibility::Public
            } else {
                ambre_core::Visibility::Private
            },
            name,
        });
        self.creating.insert(
            channel_id,
            Creating {
                space_id,
                channel,
                listing,
            },
        );
    }

    /// The open space whose catalogue lists `channel_id`, if any.
    pub(crate) fn listing_space(&self, channel_id: ChannelId) -> Option<SpaceId> {
        self.spaces
            .iter()
            .find(|(_, space)| space.state.sealed_key(channel_id).is_ok())
            .map(|(space_id, _)| *space_id)
    }

    /// The relay holds the new channel. List a public one in the catalogue
    /// with its sealed key, and send the first content of its two documents.
    /// A private one writes no catalogue.
    pub(crate) fn channel_created(&mut self, channel_id: ChannelId, key: Option<SealedChannelKey>) {
        let Some(Creating {
            space_id,
            channel,
            listing,
        }) = self.creating.remove(&channel_id)
        else {
            return;
        };
        let Some(space) = self.spaces.get_mut(&space_id) else {
            return;
        };
        if let Some(draft) = listing {
            let Some(key) = key else {
                let message = "the relay answered no sealed key".to_owned();
                return self.channel_failed(space_id, channel_id, message);
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
                payload: delta.into(),
                summary: None,
            });
        }
        self.hold_created(channel);
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

    /// The relay deleted the channel: drop it, open or opening, and end the
    /// session of each document, so the Sync worker forgets their keys. A
    /// public one leaves the catalogue.
    pub(crate) fn channel_deleted(&mut self, channel_id: ChannelId) {
        let Some(space_id) = self.deleting.remove(&channel_id) else {
            return;
        };
        self.drop_channel(channel_id);
        if let Some(space) = self.spaces.get_mut(&space_id) {
            match space.state.unlist_channel(channel_id) {
                Ok(Some(delta)) => self.send_to_sync(SyncCommand::SendUpdate {
                    document_id: space_id.document_id(),
                    payload: delta.into(),
                    summary: None,
                }),
                Ok(None) => {}
                Err(error) => tracing::warn!("Logic: unlisting {channel_id} failed: {error}"),
            }
        }
        self.send_to_ui(UiEvent::Space {
            space_id,
            event: SpaceEvent::ChannelDeleted { channel_id },
        });
        self.show_catalogue(space_id);
    }

    /// The relay removed this user from the channel: drop it as a delete
    /// does. The catalogue keeps a public one, for the next member to join.
    pub(crate) fn channel_left(&mut self, channel_id: ChannelId) {
        let Some(space_id) = self.leaving.remove(&channel_id) else {
            return;
        };
        self.drop_channel(channel_id);
        self.send_to_ui(UiEvent::Space {
            space_id,
            event: SpaceEvent::ChannelLeft { channel_id },
        });
    }

    /// The relay removed this user from `document_id`. Is it a space held
    /// here? Then the user left it, an owner removed them, or it was deleted.
    /// The space closes, and the view learns it (the Murmure spaces spec).
    pub(crate) fn space_revoked(&mut self, document_id: DocumentId) {
        let space_id = SpaceId::from_document(document_id);
        if !self.spaces.contains_key(&space_id) && !self.space_opening.contains_key(&space_id) {
            return;
        }
        self.close_space(space_id);
        self.send_to_ui(UiEvent::Space {
            space_id,
            event: SpaceEvent::Left,
        });
    }

    /// The relay removed this user from `document_id` (`SRV-187`,
    /// `SRV-188`). Is it the `meta` of a channel held here? Then the user
    /// left it from another client, or its owner deleted it. A leave or a
    /// delete asked here ends on the notice, so an answer lost after it keeps
    /// nothing.
    pub(crate) fn channel_revoked(&mut self, document_id: DocumentId) {
        let meta = ambre_bus::MetaId::from_document(document_id);
        let open = self
            .channels
            .iter()
            .find(|(_, c)| c.meta_id() == meta)
            .map(|(id, _)| *id);
        let opening = || {
            self.opening
                .iter()
                .find(|(_, o)| o.as_ref().is_some_and(|o| o.meta_id() == meta))
                .map(|(id, _)| *id)
        };
        let closed = || {
            self.closed_channels
                .iter()
                .find(|(_, c)| c.holds(document_id))
                .map(|(id, _)| *id)
        };
        let Some(channel_id) = open.or_else(opening).or_else(closed) else {
            return;
        };
        if self.leaving.contains_key(&channel_id) {
            return self.channel_left(channel_id);
        }
        if self.deleting.contains_key(&channel_id) {
            return self.channel_deleted(channel_id);
        }
        self.drop_channel(channel_id);
        self.send_to_ui(UiEvent::Channel {
            channel_id,
            event: ambre_bus::MurmureEvent::Left,
        });
    }

    /// Drop a channel, open, opening or closed, with its read cursors. The
    /// session or light subscription of each document ends, so the Sync
    /// worker forgets their keys.
    fn drop_channel(&mut self, channel_id: ChannelId) {
        let cursored = match (
            self.channels.get(&channel_id),
            self.closed_channels.get(&channel_id),
        ) {
            (Some(channel), _) => channels::cursored_documents(channel),
            (None, Some(closed)) => closed.cursored(),
            (None, None) => Vec::new(),
        };
        for document_id in cursored {
            self.read_cursors.remove(&document_id);
        }
        self.listed_channels.forget(channel_id);
        self.end_channel(channel_id);
    }

    /// A creation, a join, a delete or a leave failed on the relay, which
    /// changed nothing. A channel the user asked to delete or to leave stays
    /// as it was.
    pub(crate) fn channel_request_failed(&mut self, channel_id: ChannelId, message: String) {
        if self.direct.remove(&channel_id).is_some() {
            return self.direct_failed(channel_id, message);
        }
        let space_id = match (
            self.creating.remove(&channel_id),
            self.joining.remove(&channel_id),
            self.deleting.remove(&channel_id),
            self.leaving.remove(&channel_id),
        ) {
            (Some(creating), ..) => creating.space_id,
            (None, Some(space_id), ..)
            | (None, None, Some(space_id), _)
            | (None, None, None, Some(space_id)) => space_id,
            (None, None, None, None) => return,
        };
        self.channel_failed(space_id, channel_id, message);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ambre_bus::{CategoryId, Role, Topic};

    const NOW: Timestamp = Timestamp(1_700_000_000_000);

    /// The name `text` is, within the cap.
    fn fit_name(text: &str) -> Name {
        Name::new(text).expect("a name within the cap")
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
            covered_seq: ambre_core::Seq(1),
            payload: (stored.snapshot().expect("snapshot")).into(),
        });
        ready(state, space_id.document_id(), role);
        let _ = (drain(ui), drain(sync));
    }

    /// `space` under a session identity of its own, as another client
    /// writing it would hold it.
    fn identified_space(mut space: SpaceState) -> SpaceState {
        let peer = crate::test_peer();
        space
            .adopt_session(peer, peer.0.to_le_bytes().to_vec())
            .expect("adopt");
        space
    }

    // --- The client side of a new space ---

    /// A space named past the cap writes nothing, and the user reads why (the
    /// Murmure channels spec).
    #[test]
    fn a_name_past_its_cap_writes_nothing() {
        let (mut state, ui, sync) = harness();
        let long = "a".repeat(ambre_core::NAME_MAX_CHARS + 1);
        let refused = |ui: &_| {
            matches!(
                drain(ui).as_slice(),
                [UiEvent::ApiError { message, .. }] if message == ambre_core::NAME_TOO_LONG
            )
        };
        let space_id = SpaceId::new();
        state.handle_logic_command(LogicCommand::CreateSpace {
            space_id,
            name: long.clone(),
            now: NOW,
        });
        assert!(drain(&sync).is_empty());
        assert!(refused(&ui));
        assert!(!state.spaces.contains_key(&space_id));
    }

    /// A new space shows at once. Its whole document is its first update,
    /// sent once the Sync worker holds the session. That upload makes its
    /// creator the owner on the relay, and marks the document a space.
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
            [SyncCommand::CreateSpace { space_id: s }] if *s == space_id
        ));
        assert_eq!(
            last_catalogue(&drain(&ui)),
            Some((Some("Acme".to_owned()), Vec::new()))
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
            crate::authored_by(payload.as_bytes(), peer),
            "the upload is written under the session's PeerID alone"
        );
        let reopened =
            SpaceState::open(space_id, &[payload.as_bytes()]).expect("the upload is the space");
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
        let stored = SpaceState::create(space_id, &fit_name("Acme"), NOW).expect("create");
        state.handle_logic_command(LogicCommand::OpenSpace { space_id });
        let _ = drain(&sync);
        state.handle_logic_event(LogicEvent::RemoteSnapshot {
            document_id: space_id.document_id(),
            covered_seq: ambre_core::Seq(1),
            payload: (stored.snapshot().expect("snapshot")).into(),
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
        assert_eq!(
            updates_of(&drain(&sync), space_id.document_id()),
            0,
            "an opened space uploads nothing"
        );
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

        for document_id in [meta, segment] {
            state.grant_identity(document_id);
            ready(&mut state, document_id, Role::Owner);
        }
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
        let stored = SpaceState::create(space_id, &fit_name("Acme"), NOW).expect("create");
        state.handle_logic_command(LogicCommand::OpenSpace { space_id });
        state.handle_logic_event(LogicEvent::RemoteSnapshot {
            document_id: space_id.document_id(),
            covered_seq: ambre_core::Seq(1),
            payload: (stored.snapshot().expect("snapshot")).into(),
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
        let space_id = SpaceId::new();
        let stored = SpaceState::create(space_id, &fit_name("Acme"), NOW).expect("create");
        opened_space(&mut state, &stored, Role::Viewer, &ui, &sync);
        let channel_id = ChannelId::new();

        state.handle_logic_command(LogicCommand::Space {
            space_id,
            command: SpaceCommand::CreatePrivateChannel {
                draft: PrivateChannelDraft {
                    channel_id,
                    name: "board".to_owned(),
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
            name.as_ref().map(Name::as_str),
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
        let channel = &state.channels[&channel_id];
        assert_eq!(channel.channel_type(), Some(ChannelType::Private));
        assert_eq!(channel.name().as_deref(), Some("board"));
        assert!(state.spaces[&space_id].state.catalogue().is_empty());
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
        let space_id = SpaceId::new();
        let mut alice =
            identified_space(SpaceState::create(space_id, &fit_name("Acme"), NOW).expect("create"));
        let listed = ChannelState::create(
            ChannelId::new(),
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
            covered_seq: ambre_core::Seq(1),
            payload: (alice.snapshot().expect("snapshot")).into(),
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
    fn listed_space_as(role: Role) -> (LogicState, SpaceId, ChannelId, Receiver<SyncCommand>) {
        let space_id = SpaceId::new();
        let mut alice =
            identified_space(SpaceState::create(space_id, &fit_name("Acme"), NOW).expect("create"));
        let listed = ChannelState::create(
            ChannelId::new(),
            ChannelType::Public,
            Some(&fit_name("general")),
            NOW,
        )
        .expect("channel");
        let listing = Listing {
            topic: None,
            category: None,
            position: 0,
            key: sealed(9),
        };
        alice.list_channel(&listed, &listing).expect("list");
        let (mut bob, _ui, sync) = harness();
        bob.handle_logic_command(LogicCommand::OpenSpace { space_id });
        bob.handle_logic_event(LogicEvent::RemoteSnapshot {
            document_id: space_id.document_id(),
            covered_seq: ambre_core::Seq(1),
            payload: (alice.snapshot().expect("snapshot")).into(),
        });
        ready(&mut bob, space_id.document_id(), role);
        (bob, space_id, listed.channel_id(), sync)
    }

    /// Whether `sent` hands Sync the catalogue of `space_id` as `channels`.
    fn hands(
        sent: &[SyncCommand],
        space_id: SpaceId,
        channels: &[(ChannelId, SealedChannelKey)],
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
        let state = &bob.spaces[&space_id].state;
        assert_eq!(state.sealed_key(channel_id).expect("listed"), sealed(11));
        let sent = drain(&sync);
        assert_eq!(updates_of(&sent, space_id.document_id()), 1);
        assert!(hands(&sent, space_id, &[(channel_id, sealed(11))]));
        bob.handle_logic_event(resealed);
        assert_eq!(updates_of(&drain(&sync), space_id.document_id()), 0);
        bob.handle_logic_event(LogicEvent::ChannelKeyResealed {
            space_id: SpaceId::new(),
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
        space_id: SpaceId,
        name: &str,
        ui: &Receiver<UiEvent>,
        sync: &Receiver<SyncCommand>,
    ) -> (ChannelId, DocumentId, DocumentId) {
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
        let channel = &state.channels[&channel_id];
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
        assert!(state.channels.contains_key(&channel_id));
        assert_eq!(state.spaces[&space_id].state.catalogue().len(), 2);

        state.handle_logic_event(LogicEvent::ChannelDeleted { channel_id });
        let sent = drain(&sync);
        let mut closed: Vec<DocumentId> = sent
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
        assert!(!state.channels.contains_key(&channel_id));
        assert!(state.channels.contains_key(&kept));

        let shown = drain(&ui);
        assert!(shown.iter().any(|e| matches!(
            e,
            UiEvent::Space { event: SpaceEvent::ChannelDeleted { channel_id: id }, .. }
                if *id == channel_id
        )));
        let (_, catalogue) = last_catalogue(&shown).expect("the catalogue");
        let listed: Vec<ChannelId> = catalogue.iter().map(|e| e.channel_id).collect();
        assert_eq!(listed, [kept]);

        // An answer nobody here asked for drops nothing.
        state.handle_logic_event(LogicEvent::ChannelDeleted { channel_id: kept });
        assert!(drain(&sync).is_empty());
        assert!(state.channels.contains_key(&kept));
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
        let mut listed: Vec<ChannelId> = catalogue.iter().map(|e| e.channel_id).collect();
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
        let payload = state.spaces[&space_id].state.snapshot().expect("snapshot");
        viewer.handle_logic_command(LogicCommand::OpenSpace { space_id });
        viewer.grant_identity(space_id.document_id());
        viewer.handle_logic_event(LogicEvent::RemoteSnapshot {
            document_id: space_id.document_id(),
            covered_seq: ambre_core::Seq(1),
            payload: payload.into(),
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
        let me = UserId::new();
        state.my_user_id = Some(me);
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
        assert!(!state.channels.contains_key(&channel_id));
        let shown = drain(&ui);
        assert!(shown.iter().any(|e| matches!(
            e,
            UiEvent::Space { event: SpaceEvent::ChannelDeleted { channel_id: id }, .. }
                if *id == channel_id
        )));
        state.handle_logic_event(LogicEvent::ChannelDeleted { channel_id });
        assert!(drain(&sync).is_empty() && drain(&ui).is_empty());

        state.handle_logic_event(removed(other_meta));
        assert!(!state.channels.contains_key(&other));
        assert_eq!(
            updates_of(&drain(&sync), space_id.document_id()),
            0,
            "only the owner unlists"
        );
        assert!(drain(&ui).iter().any(|e| matches!(
            e,
            UiEvent::Channel { channel_id: id, event: ambre_bus::MurmureEvent::Left }
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
        let before = state.spaces[&space_id].state.version();

        state.handle_logic_command(LogicCommand::Space {
            space_id,
            command: SpaceCommand::DeleteChannel { channel_id },
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
        assert!(
            drain(&sync).is_empty(),
            "no session ends, nothing is written"
        );
        assert!(state.channels.contains_key(&channel_id));
        assert_eq!(state.spaces[&space_id].state.version(), before);
        assert_eq!(
            state.spaces[&space_id].state.sealed_key(channel_id),
            Ok(sealed(7))
        );

        // A late yes for a delete given up drops nothing either.
        state.handle_logic_event(LogicEvent::ChannelDeleted { channel_id });
        assert!(state.channels.contains_key(&channel_id));
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
        let before = state.spaces[&space_id].state.version();

        state.handle_logic_command(LogicCommand::Space {
            space_id,
            command: SpaceCommand::LeaveChannel { channel_id },
        });
        assert!(matches!(
            drain(&sync).as_slice(),
            [SyncCommand::LeaveChannel { channel_id: id }] if *id == channel_id
        ));
        assert!(drain(&ui).is_empty(), "nothing shows before the answer");
        assert!(state.channels.contains_key(&channel_id));

        state.handle_logic_event(LogicEvent::ChannelLeft { channel_id });
        let mut closed: Vec<DocumentId> = drain(&sync)
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
        assert!(!state.channels.contains_key(&channel_id));
        assert!(state.channels.contains_key(&kept));
        assert_eq!(state.spaces[&space_id].state.version(), before);
        assert_eq!(state.spaces[&space_id].state.catalogue().len(), 2);
        assert!(matches!(
            drain(&ui).as_slice(),
            [UiEvent::Space { event: SpaceEvent::ChannelLeft { channel_id: id }, .. }]
                if *id == channel_id
        ));

        // An answer nobody here asked for drops nothing.
        state.handle_logic_event(LogicEvent::ChannelLeft { channel_id: kept });
        assert!(drain(&sync).is_empty());
        assert!(state.channels.contains_key(&kept));
    }

    /// The relay tells this client its user left a channel,
    /// from another client. The channel goes with its sessions, and the view
    /// learns it. A removal of someone else, or from a document no channel
    /// here holds, drops nothing. A leave asked here ends on the notice.
    #[test]
    fn a_leave_from_another_client_drops_the_channel_here() {
        let (mut state, ui, sync) = harness();
        let me = UserId::new();
        state.my_user_id = Some(me);
        let space_id = created_space(&mut state, &ui, &sync);
        let (channel_id, meta, segment) =
            public_channel(&mut state, space_id, "budget", &ui, &sync);
        let (kept, kept_meta, _) = public_channel(&mut state, space_id, "general", &ui, &sync);
        let revoked = |document_id, revoked_user_id| LogicEvent::MemberRevoked {
            document_id,
            revoked_user_id,
        };

        state.handle_logic_event(revoked(kept_meta, UserId::new()));
        state.handle_logic_event(revoked(segment, me));
        assert!(
            drain(&sync).is_empty(),
            "a session ended for no leave of ours"
        );
        assert!(state.channels.contains_key(&kept) && state.channels.contains_key(&channel_id));
        let _ = drain(&ui);

        state.handle_logic_event(revoked(meta, me));
        let mut closed: Vec<DocumentId> = drain(&sync)
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
        assert!(!state.channels.contains_key(&channel_id));
        assert!(state.channels.contains_key(&kept));
        let told = drain(&ui);
        assert!(
            told.iter().any(|e| matches!(
                e,
                UiEvent::Channel { channel_id: id, event: ambre_bus::MurmureEvent::Left }
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
        assert!(!state.channels.contains_key(&kept));
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
            message: "the answer was lost".to_owned(),
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
            message: "409".to_owned(),
        });
        assert!(matches!(
            drain(&ui).as_slice(),
            [UiEvent::Space { event: SpaceEvent::ChannelFailed { channel_id: id, .. }, .. }]
                if *id == channel_id
        ));
        assert!(drain(&sync).is_empty(), "no session ends");
        assert!(state.channels.contains_key(&channel_id));

        // A late yes for a leave given up drops nothing either.
        state.handle_logic_event(LogicEvent::ChannelLeft { channel_id });
        assert!(state.channels.contains_key(&channel_id));
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
            ChannelId::new(),
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
                    position: 0,
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

    // --- The members of a space, and their presence ---

    use ambre_bus::{MemberInfo, MemberView, Presence, RequestId, RestCommand};
    use ambre_murmure::PRESENCE_EXPIRY_MS;

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
    fn samples(sent: &[SyncCommand]) -> Vec<(DocumentId, Vec<u8>)> {
        sent.iter()
            .filter_map(|c| match c {
                SyncCommand::SendEphemeral {
                    document_id,
                    payload,
                } => Some((*document_id, payload.clone().into_bytes())),
                _ => None,
            })
            .collect()
    }

    /// The member listing this thread asked for, with the document it names.
    fn member_request(sent: &[SyncCommand]) -> Option<(RequestId, DocumentId)> {
        sent.iter().find_map(|c| match c {
            SyncCommand::Rest {
                id,
                cmd: RestCommand::ListDocumentMembers { document_id },
            } => Some((*id, *document_id)),
            _ => None,
        })
    }

    fn member(user_id: UserId, name: &str) -> MemberInfo {
        MemberInfo {
            user_id,
            display_name: ambre_core::DisplayName::new(name).unwrap(),
            email: ambre_core::Email(format!("{name}@example.org")),
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
    ) -> SpaceId {
        let space_id = SpaceId::new();
        let stored = SpaceState::create(space_id, &fit_name("Acme"), NOW).expect("create");
        state.handle_logic_command(LogicCommand::OpenSpace { space_id });
        state.handle_logic_event(LogicEvent::RemoteSnapshot {
            document_id: space_id.document_id(),
            covered_seq: ambre_core::Seq(1),
            payload: (stored.snapshot().expect("snapshot")).into(),
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

    fn sample_from(state: &mut LogicState, space_id: SpaceId, from: UserId, payload: Vec<u8>) {
        state.handle_logic_event(LogicEvent::RemoteEphemeral {
            document_id: space_id.document_id(),
            from_user_id: from,
            payload: payload.into(),
        });
    }

    /// Opening a space sends this user's presence at once, on the space
    /// document and on no other. Only the members of the space hold that
    /// key, so no one outside it learns of the open.
    #[test]
    fn opening_a_space_sends_this_users_presence_on_its_document_alone() {
        let (mut state, _ui, sync) = harness();
        let space_id = SpaceId::new();
        let stored = SpaceState::create(space_id, &fit_name("Acme"), NOW).expect("create");
        state.handle_logic_command(LogicCommand::OpenSpace { space_id });
        state.handle_logic_event(LogicEvent::RemoteSnapshot {
            document_id: space_id.document_id(),
            covered_seq: ambre_core::Seq(1),
            payload: (stored.snapshot().expect("snapshot")).into(),
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
        let space_id = SpaceId::new();
        let stored = SpaceState::create(space_id, &fit_name("Acme"), NOW).expect("create");
        state.handle_logic_command(LogicCommand::OpenSpace { space_id });
        state.handle_logic_event(LogicEvent::RemoteSnapshot {
            document_id: space_id.document_id(),
            covered_seq: ambre_core::Seq(1),
            payload: (stored.snapshot().expect("snapshot")).into(),
        });
        let _ = drain(&sync);
        ready(&mut state, space_id.document_id(), Role::Editor);
        let sent = drain(&sync);
        let (alice, bob) = (UserId::new(), UserId::new());
        state.handle_logic_event(LogicEvent::MembersChanged {
            document_id: space_id.document_id(),
            members: vec![member(bob, "Bob"), member(alice, "Alice")],
        });
        let shown = drain(&ui);
        assert!(!shown.iter().any(|e| matches!(e, UiEvent::Rest { .. })));
        let members = last_members(&shown).expect("a member list");
        let ids: Vec<UserId> = members.iter().map(|m| m.user_id).collect();
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
        let (alice, bob, carol) = (UserId::new(), UserId::new(), UserId::new());
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
        let ids: Vec<UserId> = members.iter().map(|m| m.user_id).collect();
        assert_eq!(ids, [alice, carol]);

        // A document no space holds asks for nothing.
        state.handle_logic_event(LogicEvent::MembersChanged {
            document_id: DocumentId::new(),
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
        let (alice, carol) = (UserId::new(), UserId::new());
        let space_id = space_with(&mut state, &ui, &sync, vec![member(alice, "Alice")]);

        state.handle_logic_event(LogicEvent::MembersChanged {
            document_id: space_id.document_id(),
            members: vec![member(alice, "Alice"), member(carol, "Carol")],
        });
        assert!(member_request(&drain(&sync)).is_none(), "no request");
        let members = last_members(&drain(&ui)).expect("a member list");
        let ids: Vec<UserId> = members.iter().map(|m| m.user_id).collect();
        assert_eq!(ids, [alice, carol]);
    }

    /// Opening a space asks the relay for no member rows. Rows Sync hands on while the space opens wait for it, and only
    /// the last of them shows once it is open.
    #[test]
    fn an_opening_space_asks_for_no_rows_and_shows_the_last_ones_sync_read() {
        let (mut state, ui, sync) = harness();
        let (alice, bob) = (UserId::new(), UserId::new());
        let space_id = SpaceId::new();
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
            covered_seq: ambre_core::Seq(1),
            payload: (stored.snapshot().expect("snapshot")).into(),
        });
        ready(&mut state, space_id.document_id(), Role::Editor);
        assert!(member_request(&drain(&sync)).is_none(), "no listing asked");
        let members = last_members(&drain(&ui)).expect("the rows at open");
        let ids: Vec<UserId> = members.iter().map(|m| m.user_id).collect();
        assert_eq!(ids, [alice, bob]);
    }

    /// A sample turns its member online, and gets this user's sample back
    /// once, so the newcomer sees who is online without waiting a refresh
    /// period. A second sample of the same member asks nothing more.
    #[test]
    fn a_sample_shows_its_member_online_and_a_newcomer_is_answered_once() {
        let (mut state, ui, sync) = harness();
        let bob = UserId::new();
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
        let bob = UserId::new();
        let space_id = space_with(&mut state, &ui, &sync, vec![member(bob, "Bob")]);
        sample_from(&mut state, space_id, bob, ambre_murmure::presence_sample());
        let _ = (drain(&ui), drain(&sync));

        let tick = |state: &mut LogicState, offset: u64| {
            state.handle_logic_command(LogicCommand::Space {
                space_id,
                command: SpaceCommand::RefreshPresence {
                    now: Timestamp(NOW.0 + offset),
                },
            });
        };
        tick(&mut state, PRESENCE_EXPIRY_MS.0);
        assert!(last_members(&drain(&ui)).is_none(), "Bob is still online");
        assert_eq!(samples(&drain(&sync)).len(), 1);

        tick(&mut state, PRESENCE_EXPIRY_MS.0 + 1);
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
        let (me, bob) = (UserId::new(), UserId::new());
        state.my_user_id = Some(me);
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
        let presence = |id: UserId| members.iter().find(|m| m.user_id == id).map(|m| m.presence);
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
        let bob = UserId::new();
        let space_id = space_with(&mut state, &ui, &sync, vec![member(bob, "Bob")]);
        sample_from(&mut state, space_id, bob, vec![9, 9, 9]);
        assert!(drain(&ui).is_empty());
        assert!(drain(&sync).is_empty());
    }

    // --- The last owner leaving ---

    /// `member` at `role`.
    fn at(user_id: UserId, role: Role) -> MemberInfo {
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
        let me = UserId::from_bytes([5; 16]);
        state.my_user_id = Some(me);
        let (low, high, viewer) = (
            UserId::from_bytes([1; 16]),
            UserId::from_bytes([2; 16]),
            UserId::from_bytes([0; 16]),
        );
        let rows = vec![
            at(me, Role::Owner),
            at(high, Role::Editor),
            at(low, Role::Editor),
            at(viewer, Role::Viewer),
        ];
        let space_id = space_with(&mut state, &ui, &sync, rows);
        let document_id = space_id.document_id();
        let leave = RequestId::next();
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
            id: *change,
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
                status: ambre_bus::HttpStatus(403),
                code: None,
            },
            message: "HTTP 403".into(),
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
        let me = UserId::from_bytes([5; 16]);
        let other = UserId::from_bytes([6; 16]);
        for rows in [
            vec![at(me, Role::Owner), at(other, Role::Owner)],
            vec![at(me, Role::Owner)],
            vec![at(me, Role::Editor), at(other, Role::Owner)],
        ] {
            let (mut state, ui, sync) = harness();
            state.my_user_id = Some(me);
            let space_id = space_with(&mut state, &ui, &sync, rows);
            for document_id in [space_id.document_id(), DocumentId::new()] {
                state.handle_logic_command(LogicCommand::Rest {
                    id: RequestId::next(),
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
        let me = UserId::new();
        state.my_user_id = Some(me);
        let space_id = created_space(&mut state, &ui, &sync);
        let revoked = |document_id, revoked_user_id| LogicEvent::MemberRevoked {
            document_id,
            revoked_user_id,
        };
        state.handle_logic_event(revoked(space_id.document_id(), UserId::new()));
        state.handle_logic_event(revoked(DocumentId::new(), me));
        assert!(state.spaces.contains_key(&space_id), "kept");
        assert!(drain(&sync).is_empty());
        let _ = drain(&ui);

        state.handle_logic_event(revoked(space_id.document_id(), me));
        assert!(!state.spaces.contains_key(&space_id));
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
