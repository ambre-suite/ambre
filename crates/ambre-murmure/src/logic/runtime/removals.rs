//! The removals from a channel: a departure of this user, or a revoke of a
//! member, on the log of its `meta` (`SYNC-320`, `SYNC-321`).
//!
//! The Sync worker lists the documents of the channel from the relay, and a
//! relay may hide one. Only this thread reads the sealed documents. Each
//! removal thus leaves with the documents they name: the segments and
//! amendments the `meta` names, and the threads those name. The census refuses a
//! listing that leaves one out. A channel not open here opens first, and the
//! removal waits for it and for the read of its threads. A channel opened
//! for a removal alone closes again.

use super::channels::open_in_channel;
use super::*;
use crate::message::MurmureEvent;
use crate::{ChannelState, SweepStep};
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

/// The removals of a channel whose census set threads apart, and those
/// threads (`MURMURE-738`).
#[derive(Debug, Default)]
pub(crate) struct Unproven {
    /// Each removal, with the space of a departure, which its answer needs.
    removals: Vec<(Removal, Option<Id<Space>>)>,
    /// The threads the censuses set apart.
    threads: Vec<Id<Document>>,
}

impl<D> Runtime<D> {
    /// The census of `removal` from `channel_id` set `threads` apart, and
    /// its failure follows. Keep the removal, and ask the owner whether to
    /// erase those threads (`MURMURE-737`).
    pub fn threads_unproven(
        &mut self,
        channel_id: Id<Channel>,
        removal: ambre_bus::ChannelRemoval,
        threads: Vec<Id<Document>>,
    ) {
        let (removal, space) = match removal {
            ambre_bus::ChannelRemoval::Leave => {
                (Removal::Leave, self.leaving.get(&channel_id).copied())
            }
            ambre_bus::ChannelRemoval::Revoke {
                id,
                document_id,
                user_id,
            } => (
                Removal::Revoke {
                    id,
                    document_id,
                    user_id,
                },
                None,
            ),
        };
        let held = self.unproven.entry(channel_id).or_default();
        held.removals.push((removal, space));
        for thread in threads {
            if !held.threads.contains(&thread) {
                held.threads.push(thread);
            }
        }
        let threads = held.threads.clone();
        self.emit(UiEvent::Channel {
            channel_id,
            event: MurmureEvent::ThreadsUnproven { threads },
        });
    }

    /// The owner confirmed the erasure of `threads` in `channel_id`. Each
    /// removal the census set apart runs again (`MURMURE-738`). It erases the
    /// threads of `threads` it set apart, and no other. A confirmation with
    /// no such thread runs nothing.
    pub(crate) fn erase_unproven(&mut self, channel_id: Id<Channel>, threads: &[Id<Document>]) {
        let Some(held) = self.unproven.remove(&channel_id) else {
            return;
        };
        let erase: Vec<_> = held
            .threads
            .iter()
            .copied()
            .filter(|t| threads.contains(t))
            .collect();
        if erase.is_empty() {
            self.unproven.insert(channel_id, held);
            return;
        }
        self.confirmed.insert(channel_id, erase);
        for (removal, space) in held.removals {
            if let Some(space_id) = space {
                self.leaving.insert(channel_id, space_id);
            }
            self.remove_from_channel(channel_id, removal);
        }
    }

    /// Send `removal` with the documents the sealed state of `channel_id`
    /// names. A channel not open here opens first, and the removal waits for
    /// it (`SYNC-320`). An open channel first reads the threads its segments
    /// and amendments name, and the removal waits for that read
    /// (`SYNC-321`).
    pub(crate) fn remove_from_channel(&mut self, channel_id: Id<Channel>, removal: Removal) {
        let Some(channel) = self.channels.get(&channel_id) else {
            if !self.opening.contains_key(&channel_id) {
                self.open_channel(channel_id);
                self.opened_for_removal.insert(channel_id);
            }
            self.removals.entry(channel_id).or_default().push(removal);
            return;
        };
        let meta = channel.meta_id().document_id();
        if let Some(cmd) = revoke_elsewhere(removal, meta) {
            return self.ask(cmd);
        }
        self.removals.entry(channel_id).or_default().push(removal);
        self.advance_sweep(channel_id);
    }

    /// `channel_id` opened: each removal that waited on it leaves once the
    /// read of its threads ends. A channel opened for them alone closes then.
    pub(crate) fn release_removals(&mut self, channel_id: Id<Channel>) {
        let Some(meta) = self
            .channels
            .get(&channel_id)
            .map(|channel| channel.meta_id().document_id())
        else {
            return;
        };
        for removal in self.removals.remove(&channel_id).unwrap_or_default() {
            match revoke_elsewhere(removal, meta) {
                Some(cmd) => self.ask(cmd),
                None => self.removals.entry(channel_id).or_default().push(removal),
            }
        }
        if self.removals.contains_key(&channel_id) {
            self.advance_sweep(channel_id);
        } else if self.opened_for_removal.remove(&channel_id) {
            self.close_channel(channel_id);
        }
    }

    /// Move the read of the threads the removals of `channel_id` wait on
    /// one step (`SYNC-321`). Once it ends, each removal leaves with every
    /// document the sealed state names, and a channel opened for them alone
    /// closes.
    pub(crate) fn advance_sweep(&mut self, channel_id: Id<Channel>) {
        if !self.removals.contains_key(&channel_id) {
            return;
        }
        let Some(channel) = self.channels.get_mut(&channel_id) else {
            return;
        };
        let meta = channel.meta_id();
        match channel.sweep_step() {
            SweepStep::Open(document_id) => self.ask(open_in_channel(meta, document_id)),
            SweepStep::Wait => {}
            SweepStep::Done(swept) => {
                let document_id = meta.document_id();
                let erase = self.confirmed.remove(&channel_id).unwrap_or_default();
                for removal in self.removals.remove(&channel_id).unwrap_or_default() {
                    let (named, threads) = (swept.documents.clone(), swept.threads.clone());
                    let (receipts, erase) = (swept.receipts.clone(), erase.clone());
                    self.ask(match removal {
                        Removal::Leave => SyncCommand::LeaveChannel {
                            channel_id,
                            named,
                            threads,
                            receipts,
                            erase,
                        },
                        Removal::Revoke { id, user_id, .. } => SyncCommand::RevokeFromChannel {
                            id,
                            document_id,
                            user_id,
                            named,
                            threads,
                            receipts,
                            erase,
                        },
                    });
                }
                if self.opened_for_removal.remove(&channel_id) {
                    self.close_channel(channel_id);
                }
            }
        }
    }

    /// A document the read of the threads of `channel_id` opened has all
    /// its bytes: read it, close it, and go on. A document it cannot read
    /// fails every removal that waits on it (`SYNC-321`).
    pub(crate) fn swept_ready(&mut self, channel_id: Id<Channel>, document_id: Id<Document>) {
        let Some(channel) = self.channels.get_mut(&channel_id) else {
            return;
        };
        let meta = channel.meta_id();
        let read = channel.sweep_ready(document_id);
        self.forget_murmure_session(document_id);
        self.ask(SyncCommand::CloseDocument { document_id });
        match read {
            Ok(Some(next)) => self.ask(open_in_channel(meta, next)),
            Ok(None) => self.advance_sweep(channel_id),
            Err(error) => {
                tracing::warn!("Logic: {document_id} names no thread a removal can read: {error}");
                self.sweep_failed(channel_id);
            }
        }
    }

    /// The read of the threads of `channel_id` failed: each removal that
    /// waited on it fails, and nothing leaves for the Sync worker. A channel
    /// opened for them alone closes.
    pub(crate) fn sweep_failed(&mut self, channel_id: Id<Channel>) {
        if self.opened_for_removal.remove(&channel_id) {
            self.fail_removals(channel_id);
            self.close_channel(channel_id);
        } else {
            self.fail_removals(channel_id);
        }
    }

    /// The open of `channel_id`, or the read of its threads, failed. Each
    /// removal that waited on it fails without a word from the relay.
    /// Nothing leaves for the Sync worker. The document the read held open
    /// closes.
    pub(crate) fn fail_removals(&mut self, channel_id: Id<Channel>) {
        if let Some(document_id) = self
            .channels
            .get_mut(&channel_id)
            .and_then(ChannelState::end_sweep)
        {
            self.forget_murmure_session(document_id);
            self.ask(SyncCommand::CloseDocument { document_id });
        }
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

/// The request a revoke on `document_id` makes when that document is no
/// `meta` of the channel: it goes as it came. `None` for a departure, and
/// for a revoke on `meta`, which wait for the documents it names.
fn revoke_elsewhere(removal: Removal, meta: Id<Document>) -> Option<SyncCommand> {
    match removal {
        Removal::Revoke {
            id,
            document_id,
            user_id,
        } if document_id != meta => Some(SyncCommand::Rest {
            id,
            cmd: RestCommand::RevokeFromDocument {
                document_id,
                user_id,
            },
        }),
        Removal::Leave | Removal::Revoke { .. } => None,
    }
}
