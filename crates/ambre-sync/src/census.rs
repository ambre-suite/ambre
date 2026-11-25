//! The census of a removal: the open of each document a removal caps and
//! the Logic thread does not hold (`SYNC-312`, `SYNC-314`).
//!
//! A removal caps a member at the clock they reached in each document, and
//! only an open document knows that clock. A census opens each such document
//! as any open does, under the same walk, keys and caps. What the open would
//! tell the Logic thread reaches the census alone. Once the replay of every
//! document is done, the removal runs as it runs on open documents, and the
//! census closes what it opened.

use super::*;

/// The removal a census holds until its documents are read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Removal {
    /// A revoke of `user_id` on the log of `document_id`.
    Revoke {
        id: Seq<Request>,
        document_id: Id<Document>,
        user_id: Id<User>,
    },
    /// This user's departure of `document_id`.
    Leave {
        id: Seq<Request>,
        document_id: Id<Document>,
    },
    /// This user's departure of `channel_id`.
    LeaveChannel { channel_id: Id<Channel> },
}

/// What the Logic thread read in the sealed state of a channel for a
/// removal: the documents its `meta` names, and the threads those name.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Named {
    /// The segments and amendments the `meta` names (`SYNC-320`).
    pub documents: Vec<Id<Document>>,
    /// The threads those name (`SYNC-321`). A listing that leaves one out
    /// passes only under an owner's erasure of it (`SYNC-322`).
    pub threads: Vec<Id<Document>>,
    /// The link receipts those documents hold. A hidden thread under a valid
    /// one takes no erasure (`SYNC-330`).
    pub receipts: Vec<ambre_bus::LinkReceipt>,
    /// The hidden threads with no proof whose erasure the owner confirmed.
    /// The census erases no other thread with no proof (`SYNC-333`).
    pub erase: Vec<Id<Document>>,
}

/// A document a removal caps, as the relay lists it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Counted {
    pub document_id: Id<Document>,
    pub doc_type: DocType,
    /// The `meta` of its channel, whose log vouches for it (`SYNC-229`).
    pub meta: Option<Id<Meta>>,
}

/// One removal that waits on the open of its documents.
pub(crate) struct Census {
    removal: Removal,
    /// The documents whose replay is not done yet.
    awaiting: HashSet<Id<Document>>,
    /// The documents this census opened, which it closes at its end.
    opened: Vec<Id<Document>>,
    /// Whether an open of the census failed.
    failed: Option<ApiFailure>,
    /// The light subscriptions of the documents the Logic thread evicted.
    /// The census opens such a document from the relay, not from the
    /// evicted copy, and hands the subscription back at its end
    /// (`RESID-004`).
    evicted: Vec<(Id<Document>, crate::light::LightSubscription)>,
}

impl Census {
    /// Whether the census still waits on `document_id`.
    pub(crate) fn awaits(&self, document_id: Id<Document>) -> bool {
        self.awaiting.contains(&document_id)
    }
}

/// What the open of a document a census awaits tells it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Signal {
    /// The replay is done: the document knows every clock the relay keeps.
    Ready,
    /// The open failed, or holds a write past a cap and reads again.
    Failed,
}

/// The document an event about an open names, and what it tells a census
/// of it. `None` for an event no open of a document raises.
pub(crate) fn subject(event: &LogicEvent) -> Option<(Id<Document>, Option<Signal>)> {
    use LogicEvent as E;
    match event {
        E::DocumentReady { document_id, .. } => Some((*document_id, Some(Signal::Ready))),
        // The open holds its latest snapshot back until the replay is done,
        // then asks the Logic thread to check it. The replay already gave
        // every clock past the start (`SYNC-307`), so the census reads it as
        // done.
        E::PendingSnapshot {
            document_id,
            purpose: ambre_bus::SnapshotPurpose::Open,
            ..
        } => Some((*document_id, Some(Signal::Ready))),
        // A document read again holds a write past a cap: no clock of it is
        // trusted.
        E::DocumentOpenFailed { document_id, .. } | E::DocumentReread { document_id } => {
            Some((*document_id, Some(Signal::Failed)))
        }
        E::SessionIdentity { document_id, .. }
        | E::RemoteUpdate { document_id, .. }
        | E::RemoteUpdateBatch { document_id, .. }
        | E::RemoteSnapshot { document_id, .. }
        | E::PendingSnapshot { document_id, .. }
        | E::SnapshotDropped { document_id, .. }
        | E::RemoteEphemeral { document_id, .. }
        | E::PeerLeft { document_id, .. }
        | E::MembersChanged { document_id, .. }
        | E::ConnectionStatus { document_id, .. }
        | E::SyncWarning { document_id, .. }
        | E::SnapshotNeeded { document_id }
        | E::KeyRotated { document_id } => Some((*document_id, None)),
        _ => None,
    }
}

impl SyncState {
    /// Starts `removal`. A removal that caps a document the Logic thread
    /// does not hold lists the documents it caps first, then opens them
    /// (`SYNC-312`, `SYNC-314`). Any other runs at once.
    ///
    /// `named` holds what the sealed state of a channel names, as the Logic
    /// thread read it. A removal from a channel with no such list, or an
    /// empty one, fails before it lists anything (`SYNC-320`). A thread the
    /// listing leaves out passes under an owner's erasure alone (`SYNC-322`).
    pub(crate) fn begin_removal(&mut self, removal: Removal, named: Option<Named>) {
        let Some(tx) = self.event_tx.clone() else {
            return self.run_removal(removal);
        };
        let (url_scheme, api_base, auth) =
            (self.url_scheme, self.api_base.clone(), self.auth_context());
        let channel = match removal {
            Removal::Revoke { document_id, .. } => {
                match self
                    .listing_metas
                    .iter()
                    .find(|(_, meta)| **meta == document_id)
                {
                    Some((channel_id, _)) => Some(*channel_id),
                    // A `meta` whose channel no listing named leaves the
                    // documents of the channel out of reach: no cap names
                    // them, so the revoke fails (`SYNC-312`).
                    None if self
                        .channel_metas
                        .values()
                        .any(|m| m.document_id() == document_id) =>
                    {
                        return self.fail_removal(
                            removal,
                            ApiFailure::Local(ambre_core::LocalFailure::ChannelNotOpen),
                        );
                    }
                    // The Logic thread names the documents of a channel
                    // whose `meta` no listing here holds (`SYNC-320`).
                    None if named.is_some() => {
                        return self.fail_removal(
                            removal,
                            ApiFailure::Local(ambre_core::LocalFailure::ChannelNotOpen),
                        );
                    }
                    None => return self.run_removal(removal),
                }
            }
            Removal::Leave { document_id, .. } if self.documents.contains_key(&document_id) => {
                return self.run_removal(removal);
            }
            Removal::Leave { .. } => None,
            Removal::LeaveChannel { channel_id } => Some(channel_id),
        };
        // Only the sealed `meta` says which documents a channel holds. A
        // census with no word from it fails closed (`SYNC-320`).
        let named = match (channel, named) {
            (Some(_), Some(named)) if !named.documents.is_empty() => named,
            (Some(_), _) => {
                return self.fail_removal(
                    removal,
                    ApiFailure::Local(ambre_core::LocalFailure::ChannelNotOpen),
                );
            }
            (None, _) => Named::default(),
        };
        let erasing = channel.and_then(|channel_id| self.erasure_params(channel_id, removal));
        crate::spawn(async move {
            let root = crate::http::Url::<crate::http::ApiRoot>::of(url_scheme, &api_base);
            let result = match (channel, removal) {
                (Some(channel_id), _) => {
                    let url = root.route(format!("/channels/{channel_id}/documents"));
                    let revoked_on = match removal {
                        Removal::Revoke { document_id, .. } => Some(document_id),
                        Removal::Leave { .. } | Removal::LeaveChannel { .. } => None,
                    };
                    match http::get_json_auth::<http::ChannelDocumentsResponse>(&url, auth.clone())
                        .await
                    {
                        Ok(listed) => {
                            // A listing short of a named document fails
                            // before any erasure is read or posted.
                            let counted =
                                channel_counted(&listed.documents, revoked_on, &named.documents);
                            let hidden = hidden_threads(&listed.documents, &named.threads);
                            let erased = match (&counted, hidden.is_empty(), erasing) {
                                (Err(_), ..) | (Ok(_), true, _) => Ok(crate::erasure::Cleared::All),
                                (Ok(_), false, Some(params)) => {
                                    let params = crate::erasure::ErasureParams {
                                        hidden,
                                        receipts: named.receipts,
                                        confirmed: named.erase,
                                        ..params
                                    };
                                    crate::erasure::erase_hidden(
                                        &crate::sharing::GlooHttp { auth },
                                        params,
                                    )
                                    .await
                                }
                                // No footing of the `meta`, or no signed-in
                                // user: no erasure can be checked or signed.
                                (Ok(_), false, None) => Err(http::HttpError::Local(
                                    ambre_core::LocalFailure::ChannelDocumentHidden,
                                )),
                            };
                            match erased {
                                Ok(crate::erasure::Cleared::All) => counted,
                                Ok(crate::erasure::Cleared::Unproven(threads)) => {
                                    let _ = tx.send(SyncEvent::CensusUnproven {
                                        removal,
                                        channel_id,
                                        threads,
                                    });
                                    return;
                                }
                                Err(error) => Err(error),
                            }
                        }
                        Err(error) => Err(error),
                    }
                }
                (None, Removal::Leave { document_id, .. }) => {
                    let url = root.route(format!("/documents/{document_id}"));
                    http::get_json_auth::<http::DocumentTypeResponse>(&url, auth)
                        .await
                        .map(|typed| {
                            vec![Counted {
                                document_id,
                                doc_type: typed.doc_type,
                                meta: None,
                            }]
                        })
                }
                (None, _) => Ok(Vec::new()),
            };
            let _ = tx.send(SyncEvent::CensusPlanned { removal, result });
        });
    }

    /// The documents of `removal` as the relay listed them. Each one not
    /// open here opens for the census, and the removal waits on them all.
    /// A channel with more documents than one entry caps fails the removal:
    /// no document stays uncapped in silence (`SYNC-312`).
    pub(crate) fn census_planned(
        &mut self,
        removal: Removal,
        result: Result<Vec<Counted>, http::HttpError>,
    ) {
        let counted = match result {
            Ok(counted) => counted,
            Err(error) => return self.fail_removal(removal, error.failure()),
        };
        let closed: Vec<Counted> = counted
            .into_iter()
            .filter(|c| !self.documents.contains_key(&c.document_id))
            .collect();
        if closed.is_empty() {
            return self.run_removal(removal);
        }
        // A Murmure open refuses a document an open of another type claimed
        // for the session (`SYNC-260`): the census fails before it opens any.
        if closed
            .iter()
            .any(|c| c.doc_type == DocType::Murmure && self.typed_opens.contains(&c.document_id))
        {
            return self.fail_removal(
                removal,
                ApiFailure::Local(ambre_core::LocalFailure::DocumentNotOpen),
            );
        }
        let evicted = closed
            .iter()
            .filter_map(|c| {
                self.light
                    .remove(&c.document_id)
                    .map(|light| (c.document_id, light))
            })
            .collect();
        self.censuses.push(Census {
            removal,
            awaiting: closed.iter().map(|c| c.document_id).collect(),
            opened: closed.iter().map(|c| c.document_id).collect(),
            failed: None,
            evicted,
        });
        for c in closed {
            self.census_docs.insert(c.document_id);
            if let Some(meta) = c.meta {
                self.channel_metas.insert(c.document_id, meta);
            }
            // No type claims the document: the census holds it for the
            // removal alone (`SYNC-260`).
            self.open_session(c.document_id, c.doc_type);
        }
    }

    /// Hands each held signal to its census. A census whose documents are
    /// all read runs its removal, and a census with a failed open fails it.
    /// Either one then closes what it opened.
    pub(crate) fn settle_census(&mut self) {
        let signals = std::mem::take(&mut *self.census_signals.borrow_mut());
        if signals.is_empty() {
            return;
        }
        for (document_id, signal) in signals {
            for census in self.censuses.iter_mut().filter(|c| c.awaits(document_id)) {
                census.awaiting.remove(&document_id);
                if signal == Signal::Failed {
                    census.failed =
                        Some(ApiFailure::Local(ambre_core::LocalFailure::DocumentNotOpen));
                }
            }
        }
        let (ended, waiting): (Vec<Census>, Vec<Census>) = std::mem::take(&mut self.censuses)
            .into_iter()
            .partition(|c| c.awaiting.is_empty() || c.failed.is_some());
        self.censuses = waiting;
        for census in ended {
            match census.failed {
                Some(failure) => self.fail_removal(census.removal, failure),
                // The removal reads the clocks of every open document,
                // those the census opened included, before they close.
                None => self.run_removal(census.removal),
            }
            for document_id in census.opened {
                let still_held = self
                    .censuses
                    .iter()
                    .any(|c| c.opened.contains(&document_id));
                if !still_held && self.census_docs.remove(&document_id) {
                    self.handle_close_document(document_id);
                }
            }
            // An evicted document goes back to its light subscription, which
            // the close unsubscribed.
            for (document_id, light) in census.evicted {
                if self.documents.contains_key(&document_id) {
                    continue;
                }
                self.light.insert(document_id, light);
                if self.ws_connected() {
                    self.ws_send(&WsClientMsg::Subscribe { document_id });
                }
            }
        }
    }

    /// What an erasure of the hidden threads of `removal` from `channel_id`
    /// needs, with no hidden thread yet. `None` when the `meta` has no
    /// footing here or no user is signed in: a hidden thread then fails the
    /// removal (`SYNC-322`).
    fn erasure_params(
        &self,
        channel_id: Id<Channel>,
        removal: Removal,
    ) -> Option<crate::erasure::ErasureParams> {
        let meta = match removal {
            Removal::Revoke { document_id, .. } => document_id,
            Removal::Leave { .. } | Removal::LeaveChannel { .. } => {
                *self.listing_metas.get(&channel_id)?
            }
        };
        Some(crate::erasure::ErasureParams {
            url_scheme: self.url_scheme,
            api_base: self.api_base.clone(),
            channel_id,
            meta,
            hidden: Vec::new(),
            receipts: Vec::new(),
            confirmed: Vec::new(),
            footing: self.footing(meta)?,
            rule: self.log_rule(meta),
            author: self.author()?,
        })
    }

    /// The census of `removal` from `channel_id` set `threads` apart, and
    /// posted nothing (`SYNC-333`). The Logic thread learns which threads,
    /// then the removal fails: the owner confirms their erasure, and the
    /// removal runs again.
    pub(crate) fn census_unproven(
        &self,
        removal: Removal,
        channel_id: Id<Channel>,
        threads: Vec<Id<Document>>,
    ) {
        let named = match removal {
            Removal::Revoke {
                id,
                document_id,
                user_id,
            } => ambre_bus::ChannelRemoval::Revoke {
                id,
                document_id,
                user_id,
            },
            Removal::LeaveChannel { .. } => ambre_bus::ChannelRemoval::Leave,
            // A departure of one document lists no thread.
            Removal::Leave { .. } => {
                return self.fail_removal(
                    removal,
                    ApiFailure::Local(ambre_core::LocalFailure::ChannelDocumentHidden),
                );
            }
        };
        self.send_to_logic(LogicEvent::ThreadsUnproven {
            channel_id,
            removal: named,
            threads,
        });
        self.fail_removal(
            removal,
            ApiFailure::Local(ambre_core::LocalFailure::ThreadsUnproven),
        );
    }

    /// Runs `removal` on the documents open here.
    fn run_removal(&mut self, removal: Removal) {
        match removal {
            Removal::Revoke {
                id,
                document_id,
                user_id,
            } => self.revoke_now(id, document_id, user_id),
            Removal::Leave { id, document_id } => self.leave_now(id, document_id),
            Removal::LeaveChannel { channel_id } => self.leave_channel_now(channel_id),
        }
    }

    /// Answers `removal` with `failure`: nothing was posted.
    fn fail_removal(&self, removal: Removal, failure: ApiFailure) {
        crate::log::warning!("Sync: {removal:?} failed before it posted: {failure}");
        match removal {
            Removal::Revoke { id, .. } => self.send_to_logic(LogicEvent::ApiError {
                id: Some(id),
                request: ApiRequest::RevokeFromDocument,
                failure,
            }),
            Removal::Leave { id, .. } => self.send_to_logic(LogicEvent::ApiError {
                id: Some(id),
                request: ApiRequest::LeaveDocument,
                failure,
            }),
            Removal::LeaveChannel { channel_id } => {
                self.send_to_logic(LogicEvent::ChannelRequestFailed {
                    channel_id,
                    failure,
                });
            }
        }
    }
}

/// The threads of `threads` the relay's listing `entries` leaves out
/// (`SYNC-321`).
fn hidden_threads(
    entries: &[http::ChannelDocumentEntry],
    threads: &[Id<Document>],
) -> Vec<Id<Document>> {
    threads
        .iter()
        .copied()
        .filter(|thread| !entries.iter().any(|e| e.document_id == *thread))
        .collect()
}

/// The documents of a channel a removal caps beside its `meta`, each under
/// that `meta`. A listing may name no `meta`, or another one than the log a
/// revoke writes on. It then fails the removal, which would leave the
/// documents of the channel uncapped (`SYNC-312`). So does a listing that
/// leaves out a document of `named`, the documents the sealed `meta` names
/// (`SYNC-320`).
fn channel_counted(
    entries: &[http::ChannelDocumentEntry],
    revoked_on: Option<Id<Document>>,
    named: &[Id<Document>],
) -> Result<Vec<Counted>, http::HttpError> {
    if !named
        .iter()
        .all(|n| entries.iter().any(|e| e.document_id == *n))
    {
        return Err(http::HttpError::Local(
            ambre_core::LocalFailure::ChannelDocumentHidden,
        ));
    }
    let meta = entries
        .iter()
        .find(|e| e.kind == ambre_core::ChannelDocumentKind::Meta)
        .map(|e| e.document_id)
        .filter(|meta| revoked_on.is_none_or(|log| log == *meta))
        .ok_or(http::HttpError::Local(
            ambre_core::LocalFailure::NoMetaDocument,
        ))?;
    // The `meta` opens too when it is closed. A removal floors every
    // document it names no cap for at zero (`SYNC-315`). The `meta` thus
    // needs the clock of its own cap. It is the log, so it names no `meta`.
    let own = Counted {
        document_id: meta,
        doc_type: DocType::Murmure,
        meta: None,
    };
    Ok(std::iter::once(own)
        .chain(
            entries
                .iter()
                .filter(|e| e.kind != ambre_core::ChannelDocumentKind::Meta)
                .map(|e| Counted {
                    document_id: e.document_id,
                    doc_type: DocType::Murmure,
                    meta: Some(Id::<Meta>::from_document(meta)),
                }),
        )
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sharing::Author;

    const META: Id<Document> = Id::from_bytes([1; 16]);
    const THREAD: Id<Document> = Id::from_bytes([2; 16]);
    const SEGMENT: Id<Document> = Id::from_bytes([3; 16]);
    const CHANNEL: Id<Channel> = Id::from_bytes([4; 16]);

    /// A signed-in state with the `meta` of `CHANNEL` open, and no relay:
    /// a census opens its documents through the host path, which spawns
    /// nothing.
    fn signed_in() -> (SyncState, ambre_bus::conduit::Receiver<LogicEvent>, Author) {
        let (sink, rx) = LogicSink::channel();
        let mut state = SyncState::new(
            sink,
            crate::http::Url::<crate::RelayHost>::new(String::new()),
        );
        let signing_key = SigningKey::from_bytes(&[9; 32]);
        state.user_id = Some(Id::<User>::from_bytes([5; 16]));
        state.signing_key = Some(signing_key.clone());
        state.exchange_key = Some(StaticSecret::from([6; 32]));
        state.documents.insert(
            META,
            SyncDocumentState::new(
                META,
                DocumentKey::generate(),
                DocType::Murmure,
                crate::Existence::OnRelay,
            ),
        );
        state.listing_metas.insert(CHANNEL, META);
        let author = Author {
            user_id: Id::<User>::from_bytes([5; 16]),
            signing_key,
        };
        (state, rx, author)
    }

    fn under_meta(document_id: Id<Document>) -> Counted {
        Counted {
            document_id,
            doc_type: DocType::Murmure,
            meta: Some(Id::<Meta>::from_document(META)),
        }
    }

    /// The replay of `document_id` lands: its open says the document is
    /// ready, through the loop as a socket frame does.
    fn replayed(state: &mut SyncState, document_id: Id<Document>) {
        if let Some(doc) = state.documents.get_mut(&document_id) {
            doc.awaiting_initial_hydration = true;
        }
        state.handle_sync_event(SyncEvent::WsMessage(WsServerMsg::MissingUpdates {
            document_id,
            updates: Vec::new(),
        }));
    }

    /// The events of `rx` that name `document_id` as an open does.
    fn about(rx: &ambre_bus::conduit::Receiver<LogicEvent>, document_id: Id<Document>) -> usize {
        rx.try_iter()
            .filter(|e| subject(e).is_some_and(|(d, _)| d == document_id))
            .count()
    }

    /// `SYNC-312`: a revoke on the log of a channel's `meta` opens each
    /// document of the channel not open here. The revoke reads the member's
    /// clock in each one before they close. The Logic thread hears nothing
    /// of the census opens.
    #[test]
    fn a_channel_revoke_opens_each_closed_document_and_caps_the_member_there() {
        let (mut state, rx, _author) = signed_in();
        let carol = FixedBytes::<PublicKey>::from_array([7; 32]);
        let id = Seq::<Request>::new(3);
        let removal = Removal::Revoke {
            id,
            document_id: META,
            user_id: Id::<User>::from_bytes([8; 16]),
        };
        state.census_planned(removal, Ok(vec![under_meta(THREAD), under_meta(SEGMENT)]));
        assert!(state.census_docs.contains(&THREAD) && state.census_docs.contains(&SEGMENT));
        assert_eq!(state.channel_metas[&THREAD].document_id(), META);
        // The census opens read Carol's deltas; the revoke caps her there.
        state.documents[&THREAD].note_applied(&carol, Seq::<DeltaAuthor>::new(7));
        let caps: HashMap<_, _> = state.channel_applied(META).into_iter().collect();
        assert_eq!(caps[&THREAD].get(&carol), Some(&Seq::<DeltaAuthor>::new(7)));
        assert!(
            !caps[&SEGMENT].contains_key(&carol),
            "a document she never wrote caps her at zero"
        );

        replayed(&mut state, THREAD);
        assert_eq!(
            state.censuses.len(),
            1,
            "the segment's replay is not done yet"
        );
        assert!(state.documents.contains_key(&THREAD));
        replayed(&mut state, SEGMENT);
        assert!(state.censuses.is_empty());
        assert!(!state.documents.contains_key(&THREAD) && !state.documents.contains_key(&SEGMENT));
        assert!(state.census_docs.is_empty());
        let events: Vec<LogicEvent> = rx.try_iter().collect();
        assert!(
            events.iter().all(|e| subject(e).is_none()),
            "no census open reaches the Logic thread: {events:?}"
        );
        // With no relay, the revoke answers at once: it ran.
        assert!(events.iter().any(|e| matches!(
            e,
            LogicEvent::Rest { id: Some(i), evt: ambre_bus::RestEvent::RevocationComplete { .. } }
                if *i == id
        )));
    }

    /// `SYNC-314`: a departure of a document not open here opens it first.
    /// It names the last clock this user wrote in it, and the census closes
    /// the document once the departure ran.
    #[test]
    fn a_departure_of_a_closed_document_opens_it_to_name_the_cap() {
        let (mut state, rx, author) = signed_in();
        let document_id = Id::<Document>::from_bytes([10; 16]);
        let id = Seq::<Request>::new(4);
        assert_eq!(state.leaving_cap(document_id, &author), None);
        state.census_planned(
            Removal::Leave { id, document_id },
            Ok(vec![Counted {
                document_id,
                doc_type: DocType::Prose,
                meta: None,
            }]),
        );
        assert!(
            !state.typed_opens.contains(&document_id),
            "a census claims no type for the session"
        );
        state.documents[&document_id].note_applied(&author.public(), Seq::<DeltaAuthor>::new(12));
        assert_eq!(
            state.leaving_cap(document_id, &author),
            Some(Seq::<DeltaAuthor>::new(12))
        );
        replayed(&mut state, document_id);
        assert!(!state.documents.contains_key(&document_id));
        let events: Vec<LogicEvent> = rx.try_iter().collect();
        assert!(events.iter().all(|e| subject(e).is_none()), "{events:?}");
        // With no relay the departure answers at once: it ran.
        assert!(events.iter().any(|e| matches!(
            e,
            LogicEvent::ApiError {
                request: ApiRequest::LeaveDocument,
                ..
            }
        )));
    }

    /// `SYNC-312`: an open of the census may fail, or read its document
    /// again past a cap. Either one fails the removal before it posts, and
    /// the census closes every document it opened.
    #[test]
    fn a_census_open_that_fails_fails_the_removal() {
        for failing in [
            LogicEvent::DocumentOpenFailed {
                document_id: THREAD,
                cause: ambre_bus::OpenFailure::Transient,
            },
            LogicEvent::DocumentReread {
                document_id: THREAD,
            },
        ] {
            let (mut state, rx, _author) = signed_in();
            state.census_planned(
                Removal::LeaveChannel {
                    channel_id: CHANNEL,
                },
                Ok(vec![under_meta(THREAD), under_meta(SEGMENT)]),
            );
            state.send_to_logic(failing);
            state.settle_census();
            assert!(state.censuses.is_empty());
            assert!(state.census_docs.is_empty() && !state.documents.contains_key(&SEGMENT));
            let events: Vec<LogicEvent> = rx.try_iter().collect();
            assert_eq!(
                events
                    .iter()
                    .filter(|e| matches!(e, LogicEvent::ChannelRequestFailed { channel_id, failure }
                        if *channel_id == CHANNEL
                            && *failure == ApiFailure::Local(ambre_core::LocalFailure::DocumentNotOpen)))
                    .count(),
                1,
                "{events:?}"
            );
            assert!(events.iter().all(|e| subject(e).is_none()), "{events:?}");
        }
    }

    /// `SYNC-319`: a channel with more documents than one entry caps opens
    /// each of them for the census, and the removal waits on them all.
    #[test]
    fn a_channel_past_the_caps_of_one_entry_opens_every_document() {
        let (mut state, rx, _author) = signed_in();
        let id = Seq::<Request>::new(5);
        let many: Vec<Counted> = (0..=ambre_protocol::DOCUMENT_CAPS_MAX.get())
            .map(|i| {
                under_meta(Id::<Document>::from_bytes(
                    [u8::try_from(i + 20).expect("small"); 16],
                ))
            })
            .collect();
        let count = many.len();
        state.census_planned(
            Removal::Revoke {
                id,
                document_id: META,
                user_id: Id::<User>::from_bytes([8; 16]),
            },
            Ok(many),
        );
        assert_eq!(state.censuses.len(), 1, "the removal waits");
        assert_eq!(state.census_docs.len(), count, "every document opens");
        assert_eq!(state.documents.len(), count + 1, "and the meta");
        let failed = rx
            .try_iter()
            .any(|e| matches!(e, LogicEvent::ApiError { .. }));
        assert!(!failed, "no refusal past the caps of one entry");
    }

    /// `SYNC-312`: the Logic thread may open a document while its census
    /// runs. The open then reaches the Logic thread, and the census leaves
    /// the document open at its end.
    #[test]
    fn a_census_document_the_logic_thread_opens_stays_open() {
        let (mut state, rx, _author) = signed_in();
        state.census_planned(
            Removal::LeaveChannel {
                channel_id: CHANNEL,
            },
            Ok(vec![under_meta(THREAD)]),
        );
        state.handle_command(SyncCommand::OpenDocument {
            document_id: THREAD,
            doc_type: DocType::Murmure,
            meta: Some(Id::<Meta>::from_document(META)),
        });
        assert!(!state.census_docs.contains(&THREAD));
        replayed(&mut state, THREAD);
        assert!(
            state.censuses.is_empty(),
            "the census still heard the replay"
        );
        assert!(state.documents.contains_key(&THREAD));
        assert!(about(&rx, THREAD) > 0, "the Logic thread hears its open");
    }

    /// `SYNC-312`: a document the Logic thread evicted opens from the relay
    /// for the census, and goes back to its light subscription at the end.
    #[test]
    fn an_evicted_document_keeps_its_light_subscription_through_a_census() {
        let (mut state, _rx, _author) = signed_in();
        state.light.insert(
            THREAD,
            crate::light::LightSubscription::of(
                SyncDocumentState::new(
                    THREAD,
                    DocumentKey::generate(),
                    DocType::Murmure,
                    crate::Existence::OnRelay,
                ),
                Seq::<Relay>::ZERO,
            ),
        );
        state.census_planned(
            Removal::LeaveChannel {
                channel_id: CHANNEL,
            },
            Ok(vec![under_meta(THREAD)]),
        );
        assert!(
            !state.light.contains_key(&THREAD),
            "the census opens from the relay"
        );
        replayed(&mut state, THREAD);
        assert!(state.light.contains_key(&THREAD));
        assert!(!state.documents.contains_key(&THREAD));
    }

    /// `SYNC-312`: a listing with no `meta` fails the removal, and so does
    /// another `meta` than the log a revoke writes on. No cap would name the
    /// documents of the channel.
    #[test]
    fn a_listing_with_no_meta_or_another_fails() {
        let entry = |document_id, kind| http::ChannelDocumentEntry { document_id, kind };
        let listed = [
            entry(META, ambre_core::ChannelDocumentKind::Meta),
            entry(THREAD, ambre_core::ChannelDocumentKind::Thread),
        ];
        let counted = channel_counted(&listed, Some(META), &[THREAD]).expect("its own meta");
        let own = Counted {
            document_id: META,
            doc_type: DocType::Murmure,
            meta: None,
        };
        assert_eq!(counted, vec![own, under_meta(THREAD)], "the meta first");
        assert!(
            channel_counted(&listed, None, &[THREAD]).is_ok(),
            "a departure names no log"
        );
        assert!(channel_counted(&listed, Some(SEGMENT), &[THREAD]).is_err());
        assert!(channel_counted(&listed[1..], None, &[THREAD]).is_err());
    }

    /// `SYNC-315`: a departure floors every document it names no cap for at
    /// zero. A closed `meta` thus opens for the census like any document of
    /// the channel. The `meta` names no `meta` of its own.
    #[test]
    fn a_channel_departure_opens_its_closed_meta_for_its_own_cap() {
        let (mut state, _rx, _author) = signed_in();
        state.documents.remove(&META);
        let listed = [
            http::ChannelDocumentEntry {
                document_id: META,
                kind: ambre_core::ChannelDocumentKind::Meta,
            },
            http::ChannelDocumentEntry {
                document_id: THREAD,
                kind: ambre_core::ChannelDocumentKind::Thread,
            },
        ];
        let removal = Removal::LeaveChannel {
            channel_id: CHANNEL,
        };
        state.census_planned(removal, channel_counted(&listed, None, &[THREAD]));
        assert!(state.census_docs.contains(&META) && state.census_docs.contains(&THREAD));
        assert!(!state.channel_metas.contains_key(&META), "the log itself");
        assert_eq!(state.log_document(THREAD), META);
    }
    /// `SYNC-320`: a listing that leaves out a document the sealed `meta`
    /// names fails the removal, whatever else it lists. A listing that holds
    /// every named document, and more, passes.
    #[test]
    fn a_listing_that_hides_a_named_document_fails_the_removal() {
        let entry = |document_id, kind| http::ChannelDocumentEntry { document_id, kind };
        let listed = [
            entry(META, ambre_core::ChannelDocumentKind::Meta),
            entry(THREAD, ambre_core::ChannelDocumentKind::Thread),
            entry(SEGMENT, ambre_core::ChannelDocumentKind::Segment),
        ];
        let hidden = Id::<Document>::from_bytes([11; 16]);
        for revoked_on in [Some(META), None] {
            assert!(matches!(
                channel_counted(&listed, revoked_on, &[SEGMENT, hidden]),
                Err(http::HttpError::Local(
                    ambre_core::LocalFailure::ChannelDocumentHidden
                ))
            ));
            assert!(channel_counted(&listed, revoked_on, &[SEGMENT]).is_ok());
        }
        assert!(
            channel_counted(&listed[..2], Some(META), &[SEGMENT]).is_err(),
            "the segment the meta names is gone from the listing"
        );
    }

    /// `SYNC-322`: of the threads a removal names, only those the listing
    /// leaves out go to the erasure check. A segment the `meta` names takes
    /// no erasure: its absence fails the removal whatever the relay erased.
    #[test]
    fn only_a_hidden_thread_goes_to_the_erasure_check() {
        let entry = |document_id, kind| http::ChannelDocumentEntry { document_id, kind };
        let listed = [
            entry(META, ambre_core::ChannelDocumentKind::Meta),
            entry(THREAD, ambre_core::ChannelDocumentKind::Thread),
        ];
        let phantom = Id::<Document>::from_bytes([12; 16]);
        assert_eq!(hidden_threads(&listed, &[THREAD, phantom]), [phantom]);
        assert!(hidden_threads(&listed, &[THREAD]).is_empty());
        assert!(matches!(
            channel_counted(&listed, Some(META), &[SEGMENT]),
            Err(http::HttpError::Local(
                ambre_core::LocalFailure::ChannelDocumentHidden
            ))
        ));
    }

    /// `SYNC-320`: a removal from a channel with no named documents, or an
    /// empty list of them, fails before it asks the relay for anything. A
    /// revoke on a `meta` through the plain request names none, so it fails
    /// too. Nothing posts, and no census starts.
    #[test]
    fn a_channel_removal_with_no_named_documents_fails_before_it_lists() {
        let leave = Removal::LeaveChannel {
            channel_id: CHANNEL,
        };
        let revoke = Removal::Revoke {
            id: Seq::<Request>::new(6),
            document_id: META,
            user_id: Id::<User>::from_bytes([8; 16]),
        };
        let empty = || {
            Some(Named {
                documents: Vec::new(),
                threads: vec![THREAD],
                receipts: Vec::new(),
                erase: Vec::new(),
            })
        };
        for (removal, named) in [
            (leave, None),
            (leave, empty()),
            (revoke, None),
            (revoke, empty()),
        ] {
            let (mut state, rx, _author) = signed_in();
            let (tx, events) = ambre_bus::conduit::unbounded::<SyncEvent>();
            state.set_event_tx(tx);
            state.begin_removal(removal, named);
            let told: Vec<LogicEvent> = rx.try_iter().collect();
            let refused = ApiFailure::Local(ambre_core::LocalFailure::ChannelNotOpen);
            assert!(
                told.iter().any(|e| matches!(
                    e,
                    LogicEvent::ChannelRequestFailed { failure, .. }
                        | LogicEvent::ApiError { failure, .. } if *failure == refused
                )),
                "{told:?}"
            );
            assert!(events.try_iter().next().is_none(), "nothing was asked");
            assert!(state.censuses.is_empty() && state.census_docs.is_empty());
        }
    }

    /// `SYNC-333`: a census that set threads apart names them to the Logic
    /// thread with the removal it holds, then fails that removal, and
    /// nothing posts. A revoke keeps its request id, a departure its
    /// channel.
    #[test]
    fn a_census_that_sets_threads_apart_names_them_then_fails() {
        let user_id = Id::<User>::from_bytes([8; 16]);
        let revoke = Removal::Revoke {
            id: Seq::<Request>::new(6),
            document_id: META,
            user_id,
        };
        let leave = Removal::LeaveChannel {
            channel_id: CHANNEL,
        };
        let unproven = ApiFailure::Local(ambre_core::LocalFailure::ThreadsUnproven);
        for (removal, named) in [
            (
                revoke,
                ambre_bus::ChannelRemoval::Revoke {
                    id: Seq::<Request>::new(6),
                    document_id: META,
                    user_id,
                },
            ),
            (leave, ambre_bus::ChannelRemoval::Leave),
        ] {
            let (state, rx, _author) = signed_in();
            state.census_unproven(removal, CHANNEL, vec![THREAD]);
            let told: Vec<LogicEvent> = rx.try_iter().collect();
            assert!(
                matches!(
                    told.as_slice(),
                    [
                        LogicEvent::ThreadsUnproven { channel_id, removal, threads },
                        LogicEvent::ApiError { failure, .. }
                        | LogicEvent::ChannelRequestFailed { failure, .. },
                    ] if *channel_id == CHANNEL
                        && *removal == named
                        && threads == &[THREAD]
                        && *failure == unproven
                ),
                "{told:?}"
            );
        }
    }
}
