// ambre-murmure/src/logic/runtime/spaces.rs
// ============================================================================
// Murmure spaces (the Murmure spaces and channels specs). This holds the
// space document and its catalogue, the creation and the delete of a
// channel, and the self-join. A space is one document, never an entry of
// `docs`. No key reaches this thread: the
// catalogue key travels sealed, and only the Sync worker opens it.
// ============================================================================

use super::*;
use crate::message::{PrivateChannelDraft, PublicChannelDraft, SpaceCommand, SpaceEvent};
use crate::{
    ChannelState, ChannelType, Listing, Roster, SpaceState, is_presence_sample, presence_sample,
};
use ambre_bus::{ApiFailure, Authored, Instant, MemberInfo, SealedChannelKey};
use ambre_core::{Name, Validated};

/// An open space, the role the relay gave this user on its document, and
/// its members with their presence.
pub(crate) struct OpenSpace {
    pub(super) state: SpaceState,
    role: Role,
    roster: Roster,
    /// The catalogue entries as they stood when this client asked the relay
    /// which channels it holds. Only these may be unlisted on its answer:
    /// an entry listed since names a channel the relay held by then.
    checked: Option<Vec<Id<Channel>>>,
    /// The role of each member, as Sync's last listing gave them: the last
    /// owner hands the role on before leaving (the Murmure spaces spec).
    roles: Vec<(Id<User>, Role)>,
}

/// A space on its way to open, with what arrived before its document was
/// ready.
#[derive(Default)]
pub(crate) struct Opening {
    payloads: Vec<ambre_core::LoroPayload>,
    /// The last member rows Sync handed on, applied once the space is open.
    members: Option<Vec<MemberInfo>>,
}

/// What a new channel comes with: a public one its catalogue entry, a
/// private one its name.
enum NewChannel {
    Public(PublicChannelDraft),
    Private(Validated<Name>),
}

/// A channel the relay is creating, until it answers.
pub(crate) struct Creating {
    space_id: Id<Space>,
    channel: ChannelState,
    /// What the catalogue will list, for a public channel. `None` for a
    /// private one, which no catalogue lists.
    listing: Option<PublicChannelDraft>,
}

impl<D> Runtime<D> {
    /// Create a space: one new document, whose creator the relay makes its
    /// owner on the first upload. It is written again under its PeerID once
    /// its identity lands, and its whole content leaves once the Sync worker
    /// holds the document.
    pub fn create_space(
        &mut self,
        space_id: Id<Space>,
        name: &Validated<Name>,
        now: Instant<Authored>,
    ) {
        if self.spaces.contains_key(&space_id) || self.space_opening.contains_key(&space_id) {
            return;
        }
        let state = match SpaceState::create(space_id, name, now) {
            Ok(state) => state,
            Err(error) => return self.space_open_failed(space_id, &error),
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
        self.ask(SyncCommand::CreateSpace { space_id });
        self.show_catalogue(space_id);
    }

    /// Open a space the relay holds. Its payloads wait until the document is
    /// ready, then open it whole.
    pub fn open_space(&mut self, space_id: Id<Space>) {
        if self.spaces.contains_key(&space_id) || self.space_opening.contains_key(&space_id) {
            return;
        }
        self.space_opening.insert(space_id, Opening::default());
        self.ask(SyncCommand::OpenDocument {
            document_id: space_id.document_id(),
            doc_type: DocType::Murmure,
            meta: None,
        });
    }

    /// Drop a space, open or opening, with the channels it was creating.
    pub fn close_space(&mut self, space_id: Id<Space>) {
        let held = self.spaces.remove(&space_id).is_some();
        let opening = self.space_opening.remove(&space_id).is_some();
        self.creating.retain(|_, c| c.space_id != space_id);
        self.joining.retain(|_, s| *s != space_id);
        self.deleting.retain(|_, s| *s != space_id);
        self.leaving.retain(|_, s| *s != space_id);
        self.forget_murmure_session(space_id.document_id());
        if held || opening {
            self.ask(SyncCommand::CloseDocument {
                document_id: space_id.document_id(),
            });
        }
    }

    /// Send the whole content of a document this client created, once the
    /// Sync worker holds it: the first update creates it on the relay. The
    /// commands that waited on it follow.
    pub fn flush_unsent(&mut self, document_id: Id<Document>) {
        if let Some(payload) = self.unsent.remove(&document_id) {
            self.ask(SyncCommand::SendUpdate {
                document_id,
                payload,
                summary: None,
            });
            self.release_waiting(document_id);
            self.name_root(document_id);
        }
    }

    /// Adopt a session identity on the open space whose document it names.
    /// `None` when no open space holds that document.
    pub(crate) fn adopt_space_session(
        &mut self,
        document_id: Id<Document>,
        peer: ambre_core::PeerId,
        entry: &ambre_core::PeerEntry,
    ) -> Option<Result<Option<ambre_core::Payload<ambre_core::DeltaBytes>>, crate::MurmureError>>
    {
        let space = self
            .spaces
            .get_mut(&Id::<Space>::from_document(document_id))?;
        Some(space.state.adopt_session(
            peer,
            ambre_core::Payload::<ambre_core::PeerEntryBytes>::from(entry),
        ))
    }

    /// The document of a space is ready. Returns `false` when no space asked
    /// for it.
    pub fn space_document_ready(&mut self, document_id: Id<Document>, role: Role) -> bool {
        let space_id = Id::<Space>::from_document(document_id);
        if let Some(space) = self.spaces.get_mut(&space_id) {
            space.role = role;
            self.space_live(space_id);
            return true;
        }
        let Some(opening) = self.space_opening.remove(&space_id) else {
            return false;
        };
        let borrowed: Vec<ambre_core::LoroPayloadRef<'_>> = opening
            .payloads
            .iter()
            .map(ambre_core::LoroPayload::borrowed)
            .collect();
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
                self.mark_listed_channels(space_id);
                self.space_live(space_id);
                if let Some(members) = opening.members {
                    self.set_space_rows(space_id, members);
                }
            }
            Err(error) => {
                self.forget_murmure_session(document_id);
                self.ask(SyncCommand::CloseDocument { document_id });
                self.space_open_failed(space_id, &error);
            }
        }
        true
    }

    /// The document of a space could not open. Returns `false` when no space
    /// asked for it.
    pub fn space_document_failed(
        &mut self,
        document_id: Id<Document>,
        cause: &dyn std::fmt::Display,
    ) -> bool {
        let space_id = Id::<Space>::from_document(document_id);
        if self.space_opening.remove(&space_id).is_none() && !self.spaces.contains_key(&space_id) {
            return false;
        }
        self.forget_murmure_session(document_id);
        self.space_open_failed(space_id, cause);
        true
    }

    fn space_open_failed(&self, space_id: Id<Space>, cause: &dyn std::fmt::Display) {
        tracing::warn!("Logic: space {space_id} failed to open: {cause}");
        self.emit(UiEvent::Space {
            space_id,
            event: SpaceEvent::OpenFailed,
        });
    }

    /// Whether the open space that holds `document_id` leaves `peer` free for
    /// the session that opened with `nonce` (`AUTHOR-033`). A document no open
    /// space holds claims nothing here. A channel runs its own check. A space
    /// still opening keeps its payloads unchecked against the registry.
    #[must_use]
    pub fn space_peer_unclaimed(
        &self,
        document_id: Id<Document>,
        peer: ambre_core::PeerId,
        nonce: &ambre_core::FixedBytes<ambre_core::SessionNonce>,
    ) -> bool {
        self.spaces
            .get(&Id::<Space>::from_document(document_id))
            .is_none_or(|space| space.state.peer_unclaimed(peer, nonce))
    }

    /// Apply remote payloads to the space that holds `document_id`. Returns
    /// `false` when no space, open or opening, holds it.
    pub fn apply_space_remote(
        &mut self,
        document_id: Id<Document>,
        payloads: &[ambre_core::LoroPayloadRef<'_>],
    ) -> bool {
        let space_id = Id::<Space>::from_document(document_id);
        if let Some(opening) = self.space_opening.get_mut(&space_id) {
            opening
                .payloads
                .extend(payloads.iter().map(|p| p.to_payload()));
            return true;
        }
        let Some(space) = self.spaces.get_mut(&space_id) else {
            return false;
        };
        for payload in payloads {
            if let Err(error) = space.state.apply_remote(*payload) {
                tracing::warn!("Logic: delta for space {space_id} refused: {error}");
            }
        }
        self.show_catalogue(space_id);
        self.mark_listed_channels(space_id);
        true
    }

    /// Tell Sync which open or opening channels the catalogue of `space_id`
    /// lists. A channel opened before its space walked its log under the
    /// rule of a document, so Sync walks it again under the space's
    /// (`SYNC-230`). Sync ignores a mark it already holds.
    fn mark_listed_channels(&self, space_id: Id<Space>) {
        let Some(space) = self.spaces.get(&space_id) else {
            return;
        };
        space
            .state
            .catalogue()
            .iter()
            .map(|entry| entry.channel_id)
            .filter(|channel_id| {
                self.channels.contains_key(channel_id) || self.opening.contains_key(channel_id)
            })
            .for_each(|channel_id| {
                self.ask(SyncCommand::MarkChannelListed {
                    channel_id,
                    space_id,
                });
            });
    }

    /// The snapshot of a space document, for the relay's compaction, with the
    /// relay's number of the last delta it folds in (`SRV-083`).
    pub fn space_snapshot(
        &self,
        document_id: Id<Document>,
    ) -> Option<(ambre_core::Payload<ambre_core::SnapshotBytes>, Seq<Relay>)> {
        let space = self.spaces.get(&Id::<Space>::from_document(document_id))?;
        let payload = space
            .state
            .snapshot()
            .map_err(|e| tracing::warn!("Logic: snapshot of space {document_id} failed: {e}"))
            .ok()?;
        let covered = self
            .channel_ledgers
            .get(&document_id)
            .map_or(Seq::<Relay>::ZERO, SeqLedger::contiguous_end);
        Some((payload, covered))
    }

    /// Hand the view the name, the categories and the catalogue of a space.
    fn show_catalogue(&self, space_id: Id<Space>) {
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
            self.ask(SyncCommand::RotateSpace {
                space_id,
                channels: sealed,
            });
        }
        self.emit(UiEvent::Space {
            space_id,
            event: SpaceEvent::Catalogue {
                name: space.state.name(),
                role: space.role,
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
        space_id: Id<Space>,
        channel_id: Id<Channel>,
        key: &SealedChannelKey,
    ) {
        let Some(space) = self.spaces.get_mut(&space_id) else {
            return;
        };
        match space.state.reseal_channel(channel_id, key) {
            Ok(Some(payload)) => {
                self.ask(SyncCommand::SendUpdate {
                    document_id: space_id.document_id(),
                    payload,
                    summary: None,
                });
                self.show_catalogue(space_id);
            }
            Ok(None) => {}
            Err(error) => tracing::warn!("Logic: resealing {channel_id} failed: {error}"),
        }
    }

    fn channel_failed(
        &self,
        space_id: Id<Space>,
        channel_id: Id<Channel>,
        cause: &dyn std::fmt::Display,
    ) {
        tracing::warn!("Logic: channel {channel_id} of space {space_id}: {cause}");
        self.emit(UiEvent::Space {
            space_id,
            event: SpaceEvent::ChannelFailed { channel_id },
        });
    }

    /// Run one command on an open space (`BUS-027`).
    pub fn handle_space_command(&mut self, space_id: Id<Space>, command: SpaceCommand) {
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
            return self.wait_for_session(document_id, HeldCommand::Space(space_id, command));
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
                match space.state.put_category(category_id, &name, position) {
                    Ok(delta) => {
                        self.ask(SyncCommand::SendUpdate {
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
                        &"a viewer of the space creates no channel",
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
                        return self.channel_failed(space_id, channel_id, &error);
                    }
                };
                self.create_channel(space_id, channel, NewChannel::Public(draft));
            }
            SpaceCommand::CreatePrivateChannel { draft, now } => {
                // Any member of the space may: the channel is listed nowhere.
                let PrivateChannelDraft { channel_id, name } = draft;
                match ChannelState::create(channel_id, ChannelType::Private, Some(&name), now) {
                    Ok(channel) => {
                        self.create_channel(space_id, channel, NewChannel::Private(name));
                    }
                    Err(error) => self.channel_failed(space_id, channel_id, &error),
                }
            }
            SpaceCommand::JoinChannel { channel_id } => {
                // The key the catalogue holds, sealed: the Sync worker opens
                // it. The join writes no catalogue.
                match space.state.sealed_key(channel_id) {
                    Ok(key) => {
                        self.joining.insert(channel_id, space_id);
                        self.ask(SyncCommand::JoinChannel {
                            channel_id,
                            space_id,
                            key,
                        });
                    }
                    Err(error) => self.channel_failed(space_id, channel_id, &error),
                }
            }
            SpaceCommand::DeleteChannel { channel_id } => {
                // Only the relay knows who owns the channel. Nothing changes
                // here before its answer, so a refusal leaves the channel
                // whole.
                self.deleting.insert(channel_id, space_id);
                self.ask(SyncCommand::DeleteChannel { channel_id });
            }
            SpaceCommand::LeaveChannel { channel_id } => {
                // As with a delete, nothing changes before the relay answers.
                self.leaving.insert(channel_id, space_id);
                self.ask(SyncCommand::LeaveChannel { channel_id });
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
    fn space_live(&mut self, space_id: Id<Space>) {
        self.send_presence(space_id);
        self.check_catalogue(space_id);
    }

    /// Ask the relay which channels it holds as public in the space. A space
    /// admin then drops an entry that outlived its channel (the Murmure
    /// spaces spec). A viewer writes no catalogue, and an empty one has
    /// nothing to drop.
    fn check_catalogue(&mut self, space_id: Id<Space>) {
        let Some(space) = self.spaces.get_mut(&space_id) else {
            return;
        };
        let listed: Vec<Id<Channel>> = space
            .state
            .catalogue()
            .iter()
            .map(|entry| entry.channel_id)
            .collect();
        if space.role == Role::Viewer || listed.is_empty() {
            return;
        }
        space.checked = Some(listed);
        self.ask(SyncCommand::ListSpaceChannels { space_id });
    }

    /// The relay's channels of the space. Unlist each entry checked when
    /// this client asked and absent from the answer: its channel is gone.
    pub fn space_channels(&mut self, space_id: Id<Space>, channels: &[Id<Channel>]) {
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
            self.ask(SyncCommand::SendUpdate {
                document_id: space_id.document_id(),
                payload,
                summary: None,
            });
        }
        self.show_catalogue(space_id);
    }

    /// Send this user's presence sample on the space document, and on no
    /// other: only the members of the space hold its key.
    fn send_presence(&self, space_id: Id<Space>) {
        self.ask(SyncCommand::SendEphemeral {
            document_id: space_id.document_id(),
            payload: presence_sample(),
        });
    }

    /// Sync read the members of a document again and hands their rows on.
    /// Returns `false` when no space, open or opening, holds it. An open space
    /// takes the rows and asks for nothing (`SYNC-151`). An opening one keeps
    /// the last rows until it is open (the Murmure members spec).
    pub fn space_members_changed(
        &mut self,
        document_id: Id<Document>,
        members: Vec<MemberInfo>,
    ) -> bool {
        let space_id = Id::<Space>::from_document(document_id);
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
    fn set_space_rows(&mut self, space_id: Id<Space>, members: Vec<MemberInfo>) {
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
    pub fn hand_off_before_leave(
        &mut self,
        leave: Seq<Request>,
        document_id: Id<Document>,
    ) -> bool {
        let (Some(space), Some(me)) = (
            self.spaces.get(&Id::<Space>::from_document(document_id)),
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
        let change = ambre_bus::next_request();
        self.handing_off.insert(change, (leave, document_id));
        self.ask(SyncCommand::Rest {
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
    pub fn handed_off(&mut self, id: Seq<Request>, evt: &RestEvent) -> bool {
        let Some((leave, document_id)) = self.handing_off.remove(&id) else {
            return false;
        };
        if let RestEvent::MemberRoleUpdated { .. } = evt {
            self.ask(SyncCommand::Rest {
                id: leave,
                cmd: RestCommand::LeaveDocument { document_id },
            });
        } else {
            tracing::warn!("Logic: the hand-off before leaving {document_id} answered {evt:?}");
        }
        true
    }

    /// Hand the view the member list of a space.
    fn show_members(&self, space_id: Id<Space>) {
        let Some(space) = self.spaces.get(&space_id) else {
            return;
        };
        self.emit(UiEvent::Space {
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
    pub fn space_presence_heard(
        &mut self,
        document_id: Id<Document>,
        from: Id<User>,
        payload: &ambre_core::Payload<ambre_core::EphemeralBytes>,
    ) -> bool {
        let space_id = Id::<Space>::from_document(document_id);
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
    pub fn space_peer_left(&mut self, document_id: Id<Document>, user: Id<User>) -> bool {
        let space_id = Id::<Space>::from_document(document_id);
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
    pub(crate) fn spaces_know_me(&mut self, me: Id<User>) {
        for space in self.spaces.values_mut() {
            space.roster.set_me(me);
        }
    }

    /// Ask the relay for a new channel, and hold it until the relay answers.
    /// A public channel comes with its catalogue entry. A private one comes
    /// with its name, sealed beside meta so it shows before the channel opens.
    fn create_channel(&mut self, space_id: Id<Space>, channel: ChannelState, kind: NewChannel) {
        let channel_id = channel.channel_id();
        let (listing, name) = match kind {
            NewChannel::Public(draft) => (Some(draft), None),
            NewChannel::Private(name) => (None, Some(name)),
        };
        self.ask(SyncCommand::CreateChannel {
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
    pub(crate) fn listing_space(&self, channel_id: Id<Channel>) -> Option<Id<Space>> {
        self.spaces
            .iter()
            .find(|(_, space)| space.state.sealed_key(channel_id).is_ok())
            .map(|(space_id, _)| *space_id)
    }

    /// The relay holds the new channel. List a public one in the catalogue
    /// with its sealed key, and send the first content of its two documents.
    /// A private one writes no catalogue.
    pub fn channel_created(&mut self, channel_id: Id<Channel>, key: Option<SealedChannelKey>) {
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
                return self.channel_failed(
                    space_id,
                    channel_id,
                    &"the relay answered no sealed key",
                );
            };
            let listing = Listing {
                topic: draft.topic.as_ref(),
                category: draft.category,
                position: draft.position,
                key,
            };
            let delta = match space.state.list_channel(&channel, &listing) {
                Ok(delta) => delta,
                Err(error) => return self.channel_failed(space_id, channel_id, &error),
            };
            self.ask(SyncCommand::SendUpdate {
                document_id: space_id.document_id(),
                payload: delta,
                summary: None,
            });
        }
        self.hold_created(channel);
        self.emit(UiEvent::Space {
            space_id,
            event: SpaceEvent::ChannelCreated { channel_id },
        });
        self.show_catalogue(space_id);
    }

    /// The relay made this user a member of the channel: open it.
    pub fn channel_joined(&mut self, channel_id: Id<Channel>) {
        let Some(space_id) = self.joining.remove(&channel_id) else {
            return;
        };
        self.emit(UiEvent::Space {
            space_id,
            event: SpaceEvent::Joined { channel_id },
        });
        self.open_channel(channel_id);
    }

    /// The relay deleted the channel: drop it, open or opening, and end the
    /// session of each document, so the Sync worker forgets their keys. A
    /// public one leaves the catalogue.
    pub fn channel_deleted(&mut self, channel_id: Id<Channel>) {
        let Some(space_id) = self.deleting.remove(&channel_id) else {
            return;
        };
        self.drop_channel(channel_id);
        if let Some(space) = self.spaces.get_mut(&space_id) {
            match space.state.unlist_channel(channel_id) {
                Ok(Some(delta)) => self.ask(SyncCommand::SendUpdate {
                    document_id: space_id.document_id(),
                    payload: delta,
                    summary: None,
                }),
                Ok(None) => {}
                Err(error) => tracing::warn!("Logic: unlisting {channel_id} failed: {error}"),
            }
        }
        self.emit(UiEvent::Space {
            space_id,
            event: SpaceEvent::ChannelDeleted { channel_id },
        });
        self.show_catalogue(space_id);
    }

    /// The relay removed this user from the channel: drop it as a delete
    /// does. The catalogue keeps a public one, for the next member to join.
    pub fn channel_left(&mut self, channel_id: Id<Channel>) {
        let Some(space_id) = self.leaving.remove(&channel_id) else {
            return;
        };
        self.drop_channel(channel_id);
        self.emit(UiEvent::Space {
            space_id,
            event: SpaceEvent::ChannelLeft { channel_id },
        });
    }

    /// The relay removed this user from `document_id`. Is it a space held
    /// here? Then the user left it, an owner removed them, or it was deleted.
    /// The space closes, and the view learns it (the Murmure spaces spec).
    pub fn space_revoked(&mut self, document_id: Id<Document>) {
        let space_id = Id::<Space>::from_document(document_id);
        if !self.spaces.contains_key(&space_id) && !self.space_opening.contains_key(&space_id) {
            return;
        }
        self.close_space(space_id);
        self.emit(UiEvent::Space {
            space_id,
            event: SpaceEvent::Left,
        });
    }

    /// The relay removed this user from `document_id` (`SRV-187`,
    /// `SRV-188`). Is it the `meta` of a channel held here? Then the user
    /// left it from another client, or its owner deleted it. A leave or a
    /// delete asked here ends on the notice, so an answer lost after it keeps
    /// nothing.
    pub fn channel_revoked(&mut self, document_id: Id<Document>) {
        let meta = Id::<Meta>::from_document(document_id);
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
        self.emit(UiEvent::Channel {
            channel_id,
            event: crate::message::MurmureEvent::Left,
        });
    }

    /// Drop a channel, open, opening or closed, with its read cursors. The
    /// session or light subscription of each document ends, so the Sync
    /// worker forgets their keys.
    fn drop_channel(&mut self, channel_id: Id<Channel>) {
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
    pub fn channel_request_failed(&mut self, channel_id: Id<Channel>, failure: ApiFailure) {
        if self.direct.remove(&channel_id).is_some() {
            return self.direct_failed(channel_id, &failure);
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
        self.channel_failed(space_id, channel_id, &failure);
    }
}
