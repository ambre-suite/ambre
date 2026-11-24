//! The removals from a channel: a departure of this user, or a revoke of a
//! member, on the log of its `meta` (`SYNC-320`).
//!
//! The Sync worker lists the documents of the channel from the relay, and a
//! relay may hide one. Only this thread reads the sealed `meta`, so each
//! removal leaves with the documents the `meta` names. The census refuses a
//! listing that leaves one out. A channel not open here opens first, and the
//! removal waits for it. A channel opened for a removal alone closes again.

use super::*;
use ambre_bus::{ApiFailure, ApiRequest};

/// A removal from a channel that waits for the documents its `meta` names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Removal {
    /// This user's departure, which the view asked for.
    Leave,
    /// A revoke of `user_id` from the log of `document_id`, under the
    /// request id the view gave it.
    Revoke {
        id: Seq<Request>,
        document_id: Id<Document>,
        user_id: Id<User>,
    },
}

impl<D> Runtime<D> {
    /// Send `removal` with the documents the sealed `meta` of `channel_id`
    /// names. A channel not open here opens first, and the removal waits for
    /// it (`SYNC-320`).
    pub(crate) fn remove_from_channel(&mut self, channel_id: Id<Channel>, removal: Removal) {
        let Some(channel) = self.channels.get(&channel_id) else {
            if !self.opening.contains_key(&channel_id) {
                self.open_channel(channel_id);
                self.opened_for_removal.insert(channel_id);
            }
            self.removals.entry(channel_id).or_default().push(removal);
            return;
        };
        let named = channel.named_documents();
        let meta = channel.meta_id().document_id();
        self.ask(match removal {
            Removal::Leave => SyncCommand::LeaveChannel { channel_id, named },
            Removal::Revoke {
                id,
                document_id,
                user_id,
            } if document_id == meta => SyncCommand::RevokeFromChannel {
                id,
                document_id,
                user_id,
                named,
            },
            // A closed channel holds its documents by identifier alone: the
            // revoke named one of them that is no `meta`, so it goes as it
            // came.
            Removal::Revoke {
                id,
                document_id,
                user_id,
            } => SyncCommand::Rest {
                id,
                cmd: RestCommand::RevokeFromDocument {
                    document_id,
                    user_id,
                },
            },
        });
    }

    /// `channel_id` opened: each removal that waited on it leaves now. A
    /// channel opened for them alone closes again.
    pub(crate) fn release_removals(&mut self, channel_id: Id<Channel>) {
        for removal in self.removals.remove(&channel_id).unwrap_or_default() {
            self.remove_from_channel(channel_id, removal);
        }
        if self.opened_for_removal.remove(&channel_id) {
            self.close_channel(channel_id);
        }
    }

    /// The open of `channel_id` failed: each removal that waited on it fails
    /// without a word from the relay, and nothing leaves for the Sync worker.
    pub(crate) fn fail_removals(&mut self, channel_id: Id<Channel>) {
        self.opened_for_removal.remove(&channel_id);
        let failure = ApiFailure::Local(ambre_core::LocalFailure::ChannelNotOpen);
        for removal in self.removals.remove(&channel_id).unwrap_or_default() {
            match removal {
                Removal::Leave => self.channel_request_failed(channel_id, failure),
                Removal::Revoke { id, .. } => self.emit(UiEvent::ApiError {
                    id: Some(id),
                    request: ApiRequest::RevokeFromDocument,
                    failure,
                }),
            }
        }
    }

    /// A revoke the view asked for under `id`. Is `document_id` the `meta` of
    /// a channel this thread knows? Then the revoke leaves with the documents
    /// that `meta` names, once the channel is open (`SYNC-320`). Returns
    /// `false` for any other document: the request goes on as it came.
    pub fn revoke_in_channel(
        &mut self,
        id: Seq<Request>,
        document_id: Id<Document>,
        user_id: Id<User>,
    ) -> bool {
        let Some(channel_id) = self.channel_of_meta_anywhere(document_id) else {
            return false;
        };
        self.remove_from_channel(
            channel_id,
            Removal::Revoke {
                id,
                document_id,
                user_id,
            },
        );
        true
    }
}
