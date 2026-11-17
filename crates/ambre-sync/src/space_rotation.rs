//! The rotation of a space's public channels once the space key rotates (the
//! Murmure spaces spec).
//!
//! A leave of the space owes its key a rotation. A public channel owes
//! nothing of its own: the relay drops the leaver from each of them. Yet the
//! key that left with the leaver still opens every catalogue entry sealed
//! under it. A space admin therefore settles the space at once. It then
//! re-keys each public channel whose entry names an older space key.

use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;

use ambre_bus::{LogicEvent, SealedChannelKey};
use ambre_core::id::{Channel, Document, Id, Space};
use ambre_crypto::ecdh::KeyWrapper;

use crate::SyncState;
use crate::channels::{sealed_under, unseal_channel_key};

/// The catalogue of one open space, as the Logic thread last handed it.
pub(crate) struct SpaceRotation {
    space_id: Id<Space>,
    channels: Vec<(Id<Channel>, SealedChannelKey)>,
    /// The channels whose rotation runs, so a second drive starts none twice.
    in_flight: HashSet<Id<Channel>>,
    /// The wrapper the running channel rotations share, so each member
    /// costs one exchange for the whole leave (`SYNC-264`). It drops once
    /// no rotation runs, and its keys are wiped with it (`SYNC-265`).
    wrapper: Option<Rc<RefCell<KeyWrapper>>>,
}

/// What a drive of one space's rotation does next.
#[derive(Debug, PartialEq, Eq)]
enum Step {
    /// Nothing: no catalogue, no writer, or every entry is current.
    Nothing,
    /// The space key owes a rotation: pay it before any channel.
    Settle,
    /// Re-key these channels, whose entries name an older space key.
    Rotate(Vec<(Id<Channel>, SealedChannelKey)>),
}

impl SyncState {
    /// The Logic thread handed the catalogue of an open space this user
    /// administers. A rotation already running keeps running.
    pub(crate) fn handle_rotate_space(
        &mut self,
        space_id: Id<Space>,
        channels: Vec<(Id<Channel>, SealedChannelKey)>,
    ) {
        let document_id = space_id.document_id();
        let (in_flight, wrapper) = self
            .space_rotations
            .remove(&document_id)
            .map(|held| (held.in_flight, held.wrapper))
            .unwrap_or_default();
        self.space_rotations.insert(
            document_id,
            SpaceRotation {
                space_id,
                channels,
                in_flight,
                wrapper,
            },
        );
        self.drive_space_rotation(document_id);
    }

    /// Moves the rotation of the space `document_id` one step: pay the space
    /// debt, else re-key each stale channel. A document that is no space
    /// this client administers does nothing.
    pub(crate) fn drive_space_rotation(&mut self, document_id: Id<Document>) {
        match self.space_step(document_id) {
            Step::Nothing => {}
            Step::Settle => {
                let pays = self
                    .documents
                    .get_mut(&document_id)
                    .is_some_and(|space| space.rekey.pay_now());
                if pays {
                    self.start_settle(document_id);
                }
            }
            Step::Rotate(stale) => {
                for (channel_id, sealed) in stale {
                    self.start_channel_rotation(document_id, channel_id, &sealed);
                }
            }
        }
    }

    /// What the rotation of the space `document_id` does next.
    fn space_step(&self, document_id: Id<Document>) -> Step {
        let (Some(rotation), Some(space)) = (
            self.space_rotations.get(&document_id),
            self.documents.get(&document_id),
        ) else {
            return Step::Nothing;
        };
        if space.existence == crate::Existence::LocalOnly || !space.role.can_write_document() {
            return Step::Nothing;
        }
        if space.rekey.owes() {
            return Step::Settle;
        }
        let current = space.doc_key.id();
        let stale: Vec<_> = rotation
            .channels
            .iter()
            .filter(|(channel_id, sealed)| {
                sealed_under(sealed) != Some(current) && !rotation.in_flight.contains(channel_id)
            })
            .cloned()
            .collect();
        if stale.is_empty() {
            Step::Nothing
        } else {
            Step::Rotate(stale)
        }
    }

    /// Starts the rotation of one public channel, in a spawned task. An entry
    /// this client cannot open is left as it is, and reported.
    fn start_channel_rotation(
        &mut self,
        document_id: Id<Document>,
        channel_id: Id<Channel>,
        sealed: &SealedChannelKey,
    ) {
        let Some(space) = self.documents.get(&document_id) else {
            return;
        };
        let keys = space.keyring();
        // The walk of meta heeds the word of this space (audit `N30`).
        let word = space.word.clone();
        let prepared =
            unseal_channel_key(channel_id, sealed, &keys).and_then(|(previous, anchor)| {
                let anchor = anchor.ok_or(crate::error::SyncError::Unanchored)?;
                match (
                    self.event_tx.clone(),
                    self.exchange_key.clone(),
                    self.signing_key.clone(),
                ) {
                    (Some(tx), Some(exchange_key), Some(signing_key)) => {
                        Ok((tx, exchange_key, signing_key, previous, anchor))
                    }
                    _ => Err(crate::error::SyncError::SessionLocked),
                }
            });
        let (tx, exchange_key, signing_key, previous, anchor) = match prepared {
            Ok(prepared) => prepared,
            Err(message) => {
                crate::log::warning!("Sync: channel {channel_id} keeps its key: {message}");
                return;
            }
        };
        let Some(rotation) = self.space_rotations.get_mut(&document_id) else {
            return;
        };
        rotation.in_flight.insert(channel_id);
        let wrapper = Rc::clone(
            rotation
                .wrapper
                .get_or_insert_with(|| Rc::new(RefCell::new(KeyWrapper::new(&exchange_key)))),
        );
        let params = crate::sharing::RotateChannelParams {
            space_id: rotation.space_id,
            channel_id,
            url_scheme: self.url_scheme,
            api_base: self.api_base.clone(),
            exchange_key,
            signing_key,
            previous,
            anchor,
            space_key: keys.current,
            wrapper,
            word,
            // The re-key walks through the latest head of meta this client
            // walked or signed in the session (audit `P27`).
            footing: self.log_footing_at(anchor),
        };
        let http = crate::sharing::GlooHttp {
            auth: self.auth_context(),
        };
        crate::spawn(async move {
            crate::sharing::rotate_channel_task(&http, params, &tx).await;
        });
    }

    /// The answer to one channel's rotation. A new entry goes to the Logic
    /// thread, which writes it into the catalogue. A failure leaves the entry
    /// stale, for the next drive.
    pub(crate) fn channel_rotated(
        &mut self,
        space_id: Id<Space>,
        channel_id: Id<Channel>,
        result: Result<(SealedChannelKey, crate::key_vouch::Footing), crate::http::HttpError>,
    ) {
        let result = result.map(|(key, footing)| {
            self.log_walked_to(footing);
            key
        });
        if let Some(rotation) = self.space_rotations.get_mut(&space_id.document_id()) {
            rotation.in_flight.remove(&channel_id);
            if rotation.in_flight.is_empty() {
                rotation.wrapper = None;
            }
            if let Ok(key) = &result
                && let Some(entry) = rotation.channels.iter_mut().find(|(c, _)| *c == channel_id)
            {
                entry.1 = key.clone();
            }
        }
        match result {
            Ok(key) => self.send_to_logic(LogicEvent::ChannelKeyResealed {
                space_id,
                channel_id,
                key,
            }),
            Err(message) => {
                crate::log::warning!(
                    "Sync: the rotation of channel {channel_id} failed: {message}"
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::channels::seal_channel_key;
    use crate::{LogicSink, SyncDocumentState};
    use ambre_bus::{DocType, Role};
    use ambre_crypto::DocumentKey;
    use ambre_protocol::EntryDigest;

    /// A state holding the open space `space_id` under `space_key`, at
    /// `role`, with the catalogue `channels`.
    fn space(
        space_id: Id<Space>,
        space_key: &DocumentKey,
        role: Role,
        channels: Vec<(Id<Channel>, SealedChannelKey)>,
    ) -> (SyncState, ambre_bus::conduit::Receiver<LogicEvent>) {
        let (sink, rx) = LogicSink::channel();
        let mut state = SyncState::new(
            sink,
            crate::http::Url::<crate::RelayHost>::new(String::new()),
        );
        let document_id = space_id.document_id();
        let mut doc = SyncDocumentState::new(
            document_id,
            space_key.clone(),
            DocType::Murmure,
            crate::Existence::OnRelay,
        );
        doc.role = role;
        state.documents.insert(document_id, doc);
        state.space_rotations.insert(
            document_id,
            SpaceRotation {
                space_id,
                channels,
                in_flight: HashSet::new(),
                wrapper: None,
            },
        );
        (state, rx)
    }

    fn entry(channel_id: Id<Channel>, space_key: &DocumentKey) -> SealedChannelKey {
        let anchor = ambre_core::FixedBytes::<EntryDigest>::from_array([7; 32]);
        seal_channel_key(channel_id, &DocumentKey::generate(), space_key, anchor).unwrap()
    }

    /// A footing of the `meta` of `channel_id`, as a re-key leaves it.
    fn walked(channel_id: Id<Channel>) -> crate::key_vouch::Footing {
        crate::key_vouch::Footing::sealed_in_catalogue(
            Id::<Document>::mint(),
            anchor_of(channel_id),
        )
    }

    /// An anchor distinct for each channel.
    fn anchor_of(channel_id: Id<Channel>) -> ambre_core::FixedBytes<EntryDigest> {
        let mut bytes = [0; 32];
        bytes[..16].copy_from_slice(channel_id.as_bytes());
        ambre_core::FixedBytes::from_array(bytes)
    }

    /// Audit `P27`. The footing a re-key leaves lands on the log of its
    /// `meta`. The next re-key of that channel in the session then walks
    /// through the same head. A failure lands nothing, and no other anchor
    /// finds it.
    #[test]
    fn a_rekey_leaves_its_footing_for_the_next_one() {
        let (key, space_id) = (DocumentKey::generate(), Id::<Space>::mint());
        let (first, second) = (Id::<Channel>::mint(), Id::<Channel>::mint());
        let (mut state, _rx) = space(space_id, &key, Role::Owner, Vec::new());
        let footing = walked(first);
        state.channel_rotated(space_id, first, Ok((entry(first, &key), footing)));
        assert_eq!(state.log_footing_at(anchor_of(first)), Some(footing));
        state.channel_rotated(
            space_id,
            second,
            Err(crate::http::HttpError::Local(
                ambre_core::LocalFailure::NoRelay,
            )),
        );
        assert_eq!(state.log_footing_at(anchor_of(second)), None);
    }

    /// A debt of the space is paid before any channel, and only a writer of
    /// the space drives anything (the Murmure spaces spec).
    #[test]
    fn a_space_debt_is_paid_before_any_channel() {
        let (old, space_id, channel) = (
            DocumentKey::generate(),
            Id::<Space>::mint(),
            Id::<Channel>::mint(),
        );
        let catalogue = vec![(channel, entry(channel, &old))];
        let (mut state, _rx) = space(space_id, &DocumentKey::generate(), Role::Owner, catalogue);
        let document_id = space_id.document_id();
        let doc = state.documents.get_mut(&document_id).unwrap();
        doc.rekey.found_debt();
        assert_eq!(state.space_step(document_id), Step::Settle);
        let doc = state.documents.get_mut(&document_id).unwrap();
        doc.role = Role::Viewer;
        assert_eq!(state.space_step(document_id), Step::Nothing, "a viewer");
        assert_eq!(
            state.space_step(Id::<Document>::mint()),
            Step::Nothing,
            "no space"
        );
    }

    /// Once the space key is current, every entry sealed under an older key
    /// is stale, and none sealed under the current one. A channel already
    /// rotating starts again only once its answer lands, which sends the new
    /// entry to the Logic thread.
    #[test]
    fn the_entries_an_older_space_key_sealed_are_rotated_once() {
        let (old, new, space_id) = (
            DocumentKey::generate(),
            DocumentKey::generate(),
            Id::<Space>::mint(),
        );
        let (stale, current) = (Id::<Channel>::mint(), Id::<Channel>::mint());
        let catalogue = vec![(stale, entry(stale, &old)), (current, entry(current, &new))];
        let (mut state, rx) = space(space_id, &new, Role::Editor, catalogue.clone());
        let document_id = space_id.document_id();
        let step = state.space_step(document_id);
        assert_eq!(step, Step::Rotate(vec![catalogue[0].clone()]));

        let rotation = state.space_rotations.get_mut(&document_id).unwrap();
        rotation.in_flight.insert(stale);
        assert_eq!(state.space_step(document_id), Step::Nothing, "it runs");

        let resealed = entry(stale, &new);
        state.channel_rotated(space_id, stale, Ok((resealed.clone(), walked(stale))));
        assert_eq!(state.space_step(document_id), Step::Nothing, "it landed");
        match rx.try_recv() {
            Ok(LogicEvent::ChannelKeyResealed {
                channel_id, key, ..
            }) => {
                assert_eq!((channel_id, key), (stale, resealed));
            }
            other => panic!("expected ChannelKeyResealed, got {other:?}"),
        }

        let rotation = state.space_rotations.get_mut(&document_id).unwrap();
        rotation.in_flight.insert(current);
        state.channel_rotated(
            space_id,
            current,
            Err(crate::http::HttpError::Local(
                ambre_core::LocalFailure::NoRelay,
            )),
        );
        assert!(rx.try_recv().is_err(), "a failure tells Logic nothing");
        let rotation = state.space_rotations.get(&document_id).unwrap();
        assert!(rotation.in_flight.is_empty(), "the next drive tries again");
    }

    /// `SYNC-265`: the channel rotations of one leave share one wrapper. A
    /// new catalogue keeps it while a rotation runs. It drops once the last
    /// answer lands, success or failure, and no other owner keeps it.
    #[test]
    fn the_shared_wrapper_drops_once_no_channel_rotates() {
        let (key, space_id) = (DocumentKey::generate(), Id::<Space>::mint());
        let (first, second) = (Id::<Channel>::mint(), Id::<Channel>::mint());
        let (mut state, _rx) = space(space_id, &key, Role::Owner, Vec::new());
        let document_id = space_id.document_id();
        let held = Rc::new(RefCell::new(KeyWrapper::new(
            &ambre_crypto::MasterSeed::from_array([3; 32]).derive_exchange_key(),
        )));
        let watch = Rc::downgrade(&held);
        let rotation = state.space_rotations.get_mut(&document_id).unwrap();
        rotation.in_flight.extend([first, second]);
        rotation.wrapper = Some(held);
        state.handle_rotate_space(space_id, Vec::new());

        state.channel_rotated(space_id, first, Ok((entry(first, &key), walked(first))));
        assert!(watch.upgrade().is_some(), "one channel still rotates");
        state.channel_rotated(
            space_id,
            second,
            Err(crate::http::HttpError::Local(
                ambre_core::LocalFailure::NoRelay,
            )),
        );
        assert!(
            watch.upgrade().is_none(),
            "the wrapper and its keys are gone"
        );
    }

    /// A new catalogue keeps the rotations already running.
    #[test]
    fn a_new_catalogue_keeps_the_rotations_running() {
        let (key, space_id, channel) = (
            DocumentKey::generate(),
            Id::<Space>::mint(),
            Id::<Channel>::mint(),
        );
        let (mut state, _rx) = space(space_id, &key, Role::Viewer, Vec::new());
        let document_id = space_id.document_id();
        let rotation = state.space_rotations.get_mut(&document_id).unwrap();
        rotation.in_flight.insert(channel);
        state.handle_rotate_space(space_id, vec![(channel, entry(channel, &key))]);
        let rotation = state.space_rotations.get(&document_id).unwrap();
        assert!(rotation.in_flight.contains(&channel));
        assert_eq!(rotation.channels.len(), 1);
    }
}
