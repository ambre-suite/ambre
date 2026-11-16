// ambre-murmure/src/logic/runtime/direct.rs
// ============================================================================
// Murmure direct messages (the Murmure channels spec). A request names its
// members, and the relay answers with the direct message that holds exactly
// them, created or found. A direct message belongs to no space, so nothing
// here touches a catalogue. No key reaches this thread: the Sync worker wraps
// the channel key for each member.
// ============================================================================

use super::*;
use crate::message::{DirectMessageDraft, DirectMessageEvent, MurmureEvent};
use crate::{ChannelState, ChannelType};
use ambre_bus::{Authored, DirectMember, Instant, Role};

use super::channels::HeldCommand;

impl<D> Runtime<D> {
    /// Ask the relay for the direct message of this user and the draft's
    /// members. The channel it would create is built now, unnamed, and held
    /// until the relay answers.
    pub fn open_direct_message(&mut self, draft: DirectMessageDraft, now: Instant<Authored>) {
        let DirectMessageDraft {
            channel_id,
            members,
        } = draft;
        if self.direct.contains_key(&channel_id) {
            return;
        }
        if members.is_empty() {
            return self.direct_failed(channel_id, &"a direct message needs a member");
        }
        match ChannelState::create(channel_id, ChannelType::Direct, None, now) {
            Ok(channel) => {
                self.ask(SyncCommand::CreateDirectMessage {
                    channel_id,
                    meta: channel.meta_id(),
                    segment: channel.active_id(),
                    members,
                });
                self.direct.insert(channel_id, channel);
            }
            Err(error) => self.direct_failed(channel_id, &error),
        }
    }

    /// The relay answered a request with the direct message of its members.
    ///
    /// A new one is the channel built at the request: its content leaves as
    /// its first update. An existing one opens as any channel does, and the
    /// channel built at the request is dropped unsent. The request adds no
    /// member to it.
    pub fn direct_message_opened(&mut self, requested: Id<Channel>, channel_id: Id<Channel>) {
        let Some(channel) = self.direct.remove(&requested) else {
            return;
        };
        if channel_id == requested {
            self.hold_created(channel);
            self.emit(UiEvent::Channel {
                channel_id,
                event: MurmureEvent::Named { name: None },
            });
        } else {
            self.open_channel(channel_id);
        }
        self.emit(UiEvent::DirectMessage {
            requested,
            event: DirectMessageEvent::Opened { channel_id },
        });
    }

    /// A request failed, and the relay changed nothing.
    pub(crate) fn direct_failed(&self, requested: Id<Channel>, cause: &dyn std::fmt::Display) {
        tracing::warn!("Logic: direct message {requested}: {cause}");
        self.emit(UiEvent::DirectMessage {
            requested,
            event: DirectMessageEvent::Failed,
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
    pub fn announce_before_invite(&mut self, id: Seq<Request>, cmd: &RestCommand) -> bool {
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
                self.ask(SyncCommand::SendUpdate {
                    document_id: sent.segment.document_id(),
                    payload: sent.delta,
                    summary: Some(sent.summary),
                });
                self.emit(UiEvent::Channel {
                    channel_id,
                    event: sent.event,
                });
                false
            }
            Err(message) => {
                tracing::warn!("Logic: the notice of the new member failed: {message}");
                self.refuse_invite(id, ambre_core::LocalFailure::NoticeFailed);
                true
            }
        }
    }

    /// An invite asked from an open channel goes on its `meta`, as an editor,
    /// and takes the path of every invite, notice included. A member the
    /// channel already lists is refused: a second invite would wrap the key
    /// again, and announce a member who never left.
    pub fn invite_to_channel(
        &mut self,
        id: Seq<Request>,
        channel_id: Id<Channel>,
        invitee: DirectMember,
        now: Instant<Authored>,
    ) {
        let Some(meta) = self.channels.get(&channel_id).map(|c| c.meta_id()) else {
            return self.refuse_invite(id, ambre_core::LocalFailure::ChannelNotOpen);
        };
        let listed = self
            .channel_members
            .get(&channel_id)
            .is_some_and(|members| members.iter().any(|m| m.user_id == invitee.user_id));
        if listed {
            return self.refuse_invite(id, ambre_core::LocalFailure::AlreadyMember);
        }
        let cmd = RestCommand::InviteToDocument {
            document_id: meta.document_id(),
            invitee_email: None,
            invitee: invitee.user_id,
            role: Role::Editor,
            expect_exchange_pub_key: invitee.exchange_pub_key,
            now,
        };
        self.forward_invite(id, cmd);
    }

    /// Refuse an invite this thread did not let leave.
    pub(crate) fn refuse_invite(&self, id: Seq<Request>, cause: ambre_core::LocalFailure) {
        tracing::warn!("Logic: invite refused: {cause}");
        self.emit(UiEvent::ApiError {
            id: Some(id),
            request: ambre_bus::ApiRequest::InviteToDocument,
            failure: ambre_bus::ApiFailure::Local(cause),
        });
    }
}
