use super::*;

impl SyncState {
    // -- Sharing (HTTP REST + crypto) ------------------------------------------

    /// Invites a user to a document:
    /// 1. Searches for the invitee by email via `GET /api/users/search`.
    /// 2. Wraps the document key for the invitee using X25519 ECDH.
    /// 3. POSTs the wrapped key to `/api/documents/{id}/share`.
    ///
    /// On the WASM path, the async HTTP work is spawned via `spawn_local` and
    /// the result flows back as [`SyncEvent::InviteSent`].
    /// TOFU phase 1 (`ambre-spec-key-verification.md`): resolve the invitee and
    /// fetch their exchange key + fingerprint, without wrapping anything.
    /// Delegates to [`sharing::fetch_invitee_key_task`], generic over the
    /// transport so it is unit-tested natively.
    pub(crate) fn handle_fetch_invitee_key(
        &self,
        id: Seq<Request>,
        document_id: Id<Document>,
        invitee_email: ambre_core::Email,
    ) {
        let event_tx = self.event_tx.clone();
        let logic_tx = self.logic_tx.clone();
        let api_base = self.api_base.clone();
        let url_scheme = self.url_scheme;
        let auth = self.auth_context();

        let Some(tx) = event_tx else {
            // No transport (test/headless): nothing to fetch, no reply possible.
            let _ = logic_tx.send(LogicEvent::ApiError {
                id: None,
                request: ApiRequest::FetchInviteeKey,
                failure: ApiFailure::Local(ambre_core::LocalFailure::NoRelay),
            });
            return;
        };

        let params = sharing::FetchInviteeKeyParams {
            id,
            document_id,
            url_scheme,
            api_base,
            invitee_email,
        };
        let http = sharing::GlooHttp { auth };
        crate::spawn(async move {
            sharing::fetch_invitee_key_task(&http, params, &tx, &logic_tx).await;
        });
    }

    /// Fetch the exchange key of `user_id`, wrapping nothing. Delegates to
    /// [`sharing::fetch_member_key_task`], as the invitee's fetch does.
    pub(crate) fn handle_fetch_member_key(&self, id: Seq<Request>, user_id: Id<User>) {
        let logic_tx = self.logic_tx.clone();
        let Some(tx) = self.event_tx.clone() else {
            let _ = logic_tx.send(LogicEvent::ApiError {
                id: None,
                request: ApiRequest::FetchMemberKey,
                failure: ApiFailure::Local(ambre_core::LocalFailure::NoRelay),
            });
            return;
        };
        let params = sharing::FetchMemberKeyParams {
            id,
            user_id,
            url_scheme: self.url_scheme,
            api_base: self.api_base.clone(),
        };
        let http = sharing::GlooHttp {
            auth: self.auth_context(),
        };
        crate::spawn(async move {
            sharing::fetch_member_key_task(&http, params, &tx, &logic_tx).await;
        });
    }

    /// Invites a user to a document (TOFU phase 2). A document the relay does
    /// not hold yet has no member log to append to. The invite waits, and
    /// the creation starts: [`SyncState::flush_pending_invites`] resumes it
    /// (`SYNC-246`).
    pub(crate) fn handle_invite_to_document(
        &mut self,
        id: Seq<Request>,
        document_id: Id<Document>,
        invitee_email: Option<ambre_core::Email>,
        invitee: Id<User>,
        role: Role,
        expect_exchange_pub_key: ambre_core::FixedBytes<ambre_core::ExchangePublicKey>,
    ) {
        // With a transport but no session keys, no creation can start: the
        // invite falls through to the refusal below instead of waiting.
        let can_create =
            self.event_tx.is_none() || (self.exchange_key.is_some() && self.author().is_some());
        if let Some(doc_state) = self
            .documents
            .get_mut(&document_id)
            .filter(|d| can_create && d.existence == crate::Existence::LocalOnly)
        {
            doc_state.pending_invites.push(PendingInvite {
                id,
                invitee_email,
                invitee,
                role,
                expect_exchange_pub_key,
            });
            self.trigger_lazy_create(document_id);
            return;
        }
        let event_tx = self.event_tx.clone();
        let logic_tx = self.logic_tx.clone();
        let api_base = self.api_base.clone();
        let url_scheme = self.url_scheme;
        let auth = self.auth_context();
        let exchange_key = self.exchange_key.clone();
        // An invitation wraps the key the walk of the log settled
        // (`SYNC-234`).
        let doc_key = match self.documents.get(&document_id).map(|ds| ds.sealing_key()) {
            Some(Ok(doc_key)) => Some(doc_key.clone()),
            Some(Err(refused)) => {
                let _ = logic_tx.send(LogicEvent::ApiError {
                    id: None,
                    request: ApiRequest::InviteToDocument,
                    failure: crate::http::Reported::failure(&refused),
                });
                return;
            }
            None => None,
        };

        let Some(tx) = event_tx else {
            self.reply(
                id,
                RestEvent::InviteSent {
                    document_id,
                    user_id: Id::<User>::mint(),
                    display_name: invitee_email
                        .and_then(|email| ambre_core::cut_display_name_to_fit(email.as_str()))
                        .unwrap_or_else(|| ambre_core::display_handle(&invitee)),
                    // The mock relay seeds the session keys at once, so the
                    // signing key is there; the zero pin names no key.
                    inviter: self
                        .author_pub_key()
                        .map_or(FixedBytes::from_array([0; 32]), |key| {
                            ambre_crypto::inviter_pin(&key)
                        }),
                },
            );
            return;
        };
        let (Some(exchange_key), Some(author)) = (exchange_key, self.author()) else {
            let _ = logic_tx.send(LogicEvent::ApiError {
                id: None,
                request: ApiRequest::InviteToDocument,
                failure: ApiFailure::Local(ambre_core::LocalFailure::NotSignedIn),
            });
            return;
        };
        // The invite walks from the log the open accepted (audit `P26`).
        let (Some(doc_key), Some(footing)) = (doc_key, self.footing(document_id)) else {
            let _ = logic_tx.send(LogicEvent::ApiError {
                id: None,
                request: ApiRequest::InviteToDocument,
                failure: ApiFailure::Local(ambre_core::LocalFailure::DocumentNotOpen),
            });
            return;
        };

        // Orchestration (search -> wrap -> share) lives in `sharing::invite_task`,
        // generic over the HTTP transport so it is unit-tested natively. Here we
        // only carry the gathered state into the spawned task.
        let params = sharing::InviteParams {
            id,
            document_id,
            url_scheme,
            api_base,
            invitee_email,
            invitee,
            role,
            exchange_key,
            doc_key,
            expect_exchange_pub_key,
            // The signing key the confirmed fingerprint covered, when this
            // session showed it for that very exchange key (audit `P44`).
            expect_signing_pub_key: self
                .shown_identities
                .get(&invitee)
                .filter(|shown| shown.exchange == expect_exchange_pub_key)
                .map(|shown| shown.signing),
            author,
            footing,
            rule: self.log_rule(document_id),
        };
        let http = sharing::GlooHttp { auth };
        crate::spawn(async move {
            sharing::invite_task(&http, params, &tx, &logic_tx).await;
        });
    }

    /// Resumes the invites that waited for the document's lazy creation
    /// (`SYNC-246`). A creation that failed, or a close that came first, ends
    /// each of them with a refusal, so no invite is lost in silence
    /// (`SYNC-247`).
    pub(crate) fn flush_pending_invites(
        &mut self,
        document_id: Id<Document>,
        failure: Option<&http::Failed>,
    ) {
        let pending = self
            .documents
            .get_mut(&document_id)
            .map(|d| std::mem::take(&mut d.pending_invites))
            .unwrap_or_default();
        for invite in pending {
            match failure {
                None => self.handle_invite_to_document(
                    invite.id,
                    document_id,
                    invite.invitee_email,
                    invite.invitee,
                    invite.role,
                    invite.expect_exchange_pub_key,
                ),
                Some(failed) => {
                    crate::log::warning!(
                        "Sync: invite {} waits on a document not created: {failed}",
                        invite.id
                    );
                    self.send_to_logic(LogicEvent::ApiError {
                        id: Some(invite.id),
                        request: ApiRequest::InviteToDocument,
                        failure: failed.failure,
                    });
                }
            }
        }
    }

    /// Revokes a member from a document and rotates the document key:
    /// 1. GETs the members from the server.
    /// 2. Generates a fresh [`DocumentKey`] via CSPRNG.
    /// 3. Wraps the new key via X25519 ECDH for each member but the one removed.
    /// 4. POSTs the removal and every copy to `/api/documents/{id}/revoke`. The
    ///    server applies both in one transaction and then broadcasts
    ///    `WsServerMsg::KeyRotationRequired` (`SYNC-126`).
    /// 5. Emits [`SyncEvent::RevocationComplete`], which causes
    ///    [`handle_sync_event`] to swap the local key and push the old key
    ///    into the [`SyncDocumentState::old_keys`] LRU cache.
    pub(crate) fn handle_revoke_from_document(
        &self,
        id: Seq<Request>,
        document_id: Id<Document>,
        user_id: Id<User>,
    ) {
        let event_tx = self.event_tx.clone();
        let logic_tx = self.logic_tx.clone();
        let api_base = self.api_base.clone();
        let url_scheme = self.url_scheme;
        let auth = self.auth_context();
        let exchange_key = self.exchange_key.clone();

        // The revoke links the key the walk of the log settled to the new
        // one (`SYNC-234`).
        if let Some(Err(refused)) = self.documents.get(&document_id).map(|ds| ds.sealing_key()) {
            let _ = logic_tx.send(LogicEvent::ApiError {
                id: None,
                request: ApiRequest::RevokeFromDocument,
                failure: crate::http::Reported::failure(&refused),
            });
            return;
        }
        let Some(tx) = event_tx else {
            self.reply(id, RestEvent::RevocationComplete { document_id });
            return;
        };
        let (Some(exchange_key), Some(author)) = (exchange_key, self.author()) else {
            let _ = logic_tx.send(LogicEvent::ApiError {
                id: None,
                request: ApiRequest::RevokeFromDocument,
                failure: ApiFailure::Local(ambre_core::LocalFailure::NotSignedIn),
            });
            return;
        };
        // The revoke seals the current key under the new one, so it runs
        // from an open document only (`SYNC-116`). It walks from the log the
        // open accepted (audit `P26`).
        let Some((previous, footing)) = self.documents.get(&document_id).and_then(|doc_state| {
            self.footing(document_id)
                .map(|footing| (doc_state.doc_key.clone(), footing))
        }) else {
            let _ = logic_tx.send(LogicEvent::ApiError {
                id: None,
                request: ApiRequest::RevokeFromDocument,
                failure: ApiFailure::Local(ambre_core::LocalFailure::DocumentNotOpen),
            });
            return;
        };

        // Orchestration (members -> new key -> one revoke POST) lives in
        // `sharing::revoke_task`. The old key is dropped/zeroized when the local
        // swap happens in `handle_sync_event` on `RevocationComplete`.
        let params = sharing::RevokeParams {
            id,
            document_id,
            user_id,
            url_scheme,
            api_base,
            exchange_key,
            previous,
            author,
            footing,
            rule: self.log_rule(document_id),
        };
        let http = sharing::GlooHttp { auth };
        crate::spawn(async move {
            sharing::revoke_task(&http, params, &tx, &logic_tx).await;
        });
    }

    /// Where the log of `document_id` stands for this client, once an open or
    /// a creation accepted it (audits `P26`, `P27`).
    /// The later of the footing the document holds and the one its log
    /// holds. A head this client walked through another document of the
    /// same channel, or through a re-key, then counts too.
    pub(crate) fn footing(&self, document_id: Id<Document>) -> Option<crate::key_vouch::Footing> {
        let mut footing = self
            .documents
            .get(&document_id)
            .and_then(|doc| doc.footing)?;
        if let Some(log) = self.log_footings.get(&footing.document()) {
            footing.advance(*log);
        }
        Some(footing)
    }

    /// Moves the footing of the log `footing` walks on to it, when it is
    /// later than the one held (audit `P27`).
    pub(crate) fn log_walked_to(&mut self, footing: crate::key_vouch::Footing) {
        self.log_footings
            .entry(footing.document())
            .and_modify(|held| held.advance(footing))
            .or_insert(footing);
    }

    /// The footing of the log a catalogue entry seals `anchor` for, when
    /// this client walked that log earlier in the session.
    pub(crate) fn log_footing_at(
        &self,
        anchor: ambre_core::FixedBytes<ambre_protocol::EntryDigest>,
    ) -> Option<crate::key_vouch::Footing> {
        self.log_footings
            .values()
            .find_map(|footing| footing.sealed_at(anchor))
    }

    /// The signer of the entries this user appends to a membership log
    /// (`SYNC-132`), once signed in.
    pub(crate) fn author(&self) -> Option<sharing::Author> {
        Some(sharing::Author {
            user_id: self.user_id?,
            signing_key: self.signing_key.clone()?,
        })
    }

    /// Leaves a document: DELETEs this user's own share and nothing else
    /// (`SYNC-127`, `SYNC-130`). [`SyncEvent::DocumentLeft`] closes it locally.
    pub(crate) fn handle_leave_document(&self, id: Seq<Request>, document_id: Id<Document>) {
        let (Some(tx), Some(author), Some(exchange_key)) = (
            self.event_tx.clone(),
            self.author(),
            self.exchange_key.clone(),
        ) else {
            let _ = self.logic_tx.send(LogicEvent::ApiError {
                id: None,
                request: ApiRequest::LeaveDocument,
                failure: ApiFailure::Local(ambre_core::LocalFailure::NotSignedIn),
            });
            return;
        };
        let logic_tx = self.logic_tx.clone();
        let params = sharing::LeaveParams {
            id,
            document_id,
            author,
            url_scheme: self.url_scheme,
            api_base: self.api_base.clone(),
            exchange_key,
        };
        let http = sharing::GlooHttp {
            auth: self.auth_context(),
        };
        crate::spawn(async move {
            sharing::leave_task(&http, params, &tx, &logic_tx).await;
        });
    }

    /// Fetches the member list for a document via `GET /api/documents/{id}/members`,
    /// maps the server response into [`MemberInfo`] values (plaintext names,
    /// decoded roles), and replies with [`RestEvent::DocumentMembers`]. With no
    /// request id it refreshes the member directory alone, and replies nothing.
    pub(crate) fn handle_list_document_members(
        &mut self,
        id: Option<Seq<Request>>,
        document_id: Id<Document>,
    ) {
        // Number the listing, so an answer older than the directory held
        // changes nothing (`SYNC-037`). A document not open holds no directory.
        let asked = self
            .documents
            .get_mut(&document_id)
            .map_or(ambre_core::Seq::ZERO, |doc| doc.directory_asked.issue());
        let event_tx = self.event_tx.clone();
        let api_base = self.api_base.clone();
        let url_scheme = self.url_scheme;
        let auth = self.auth_context();

        if let (Some(tx), Some(exchange_key)) = (event_tx, self.exchange_key.clone()) {
            let params = sharing::ListMembersParams {
                id,
                asked,
                document_id,
                url_scheme,
                api_base,
                exchange_key,
                rule: self.log_rule(document_id),
                log_document: self.log_document(document_id),
                footing: self.footing(document_id),
            };
            let http = sharing::GlooHttp { auth };
            crate::spawn(async move {
                sharing::list_members_task(&http, params, &tx).await;
            });
        } else if let Some(id) = id {
            self.reply(
                id,
                RestEvent::DocumentMembers {
                    document_id,
                    members: Vec::new(),
                },
            );
        }
    }

    /// Updates a member's role via `PATCH /api/documents/{id}/members/{uid}`,
    /// with the entry this owner signs for it (`SYNC-132`). Replies with
    /// [`RestEvent::MemberRoleUpdated`] on success.
    pub(crate) fn handle_update_document_member_role(
        &self,
        id: Seq<Request>,
        document_id: Id<Document>,
        user_id: Id<User>,
        role: Role,
    ) {
        let logic_tx = self.logic_tx.clone();
        if let Some(tx) = self.event_tx.clone() {
            let Some(author) = self.author() else {
                let _ = logic_tx.send(LogicEvent::ApiError {
                    id: None,
                    request: ApiRequest::UpdateDocumentMemberRole,
                    failure: ApiFailure::Local(ambre_core::LocalFailure::NotSignedIn),
                });
                return;
            };
            // The change walks from the log the open accepted (audit `P26`).
            let Some(footing) = self.footing(document_id) else {
                let _ = logic_tx.send(LogicEvent::ApiError {
                    id: None,
                    request: ApiRequest::UpdateDocumentMemberRole,
                    failure: ApiFailure::Local(ambre_core::LocalFailure::DocumentNotOpen),
                });
                return;
            };
            let params = sharing::RoleChangeParams {
                id,
                document_id,
                user_id,
                role,
                url_scheme: self.url_scheme,
                api_base: self.api_base.clone(),
                author,
                footing,
                rule: self.log_rule(document_id),
            };
            let http = sharing::GlooHttp {
                auth: self.auth_context(),
            };
            crate::spawn(async move {
                sharing::role_change_task(&http, params, &tx, &logic_tx).await;
            });
        } else {
            crate::log::info!(
                "Sync: UpdateDocumentMemberRole {document_id:?} {user_id:?} {role:?}"
            );
            self.reply(
                id,
                RestEvent::MemberRoleUpdated {
                    document_id,
                    user_id,
                    role,
                },
            );
        }
    }

    /// Searches users via `GET /api/users/search?query=...`, maps the server
    /// response into [`UserInfo`] values, and replies with
    /// [`RestEvent::UserSearchResults`]. The query is percent-encoded whole
    /// before it is placed in the URL (`SYNC-179`).
    pub(crate) fn handle_search_users(
        &self,
        id: Seq<Request>,
        query: ambre_core::Validated<ambre_core::UserSearch>,
    ) {
        let event_tx = self.event_tx.clone();
        let api_base = self.api_base.clone();
        let url_scheme = self.url_scheme;
        let auth = self.auth_context();

        if let Some(tx) = event_tx {
            crate::spawn(async move {
                let url = crate::http::Url::<crate::http::ApiRoot>::of(url_scheme, &api_base)
                    .route("/users/search")
                    .query(http::QueryKey::Query, query.as_str());
                match http::get_json_auth::<http::UserSearchResult>(&url, auth).await {
                    Ok(resp) => {
                        // Each id was parsed when the answer was read, so none
                        // is invented here (`SYNC-038`).
                        let users: Vec<UserInfo> = resp
                            .users
                            .into_iter()
                            .map(|u| UserInfo {
                                user_id: u.id,
                                display_name: u.display_name,
                                email: u.email,
                            })
                            .collect();
                        let _ = tx.send(SyncEvent::UserSearchReady { id, users });
                    }
                    Err(e) => {
                        let _ = tx.send(SyncEvent::http_error(id, e));
                    }
                }
            });
        } else {
            // The query is now a whole address, so no log line names it
            // (`SYNC-292`).
            drop(query);
            crate::log::info!("Sync: SearchUsers with no relay");
            self.reply(id, RestEvent::UserSearchResults { users: Vec::new() });
        }
    }
}

/// An invite asked for while its document is local-only, kept as the command
/// carried it until the relay holds the document (`SYNC-246`).
pub(crate) struct PendingInvite {
    id: Seq<Request>,
    invitee_email: Option<ambre_core::Email>,
    invitee: Id<User>,
    role: Role,
    expect_exchange_pub_key: ambre_core::FixedBytes<ambre_core::ExchangePublicKey>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bare_state() -> (SyncState, ambre_bus::conduit::Receiver<LogicEvent>) {
        let (sink, rx) = LogicSink::channel();
        let state = SyncState::new(
            sink,
            crate::http::Url::<crate::RelayHost>::new(String::new()),
        );
        (state, rx)
    }

    /// Audits `P26`, `P27`: a document stands on the later of the footing
    /// it holds and the one its log walked to. One no open accepted stands
    /// on none.
    #[test]
    fn a_document_stands_on_the_later_of_its_footing_and_its_logs() {
        use crate::membership_log::tests::{DOC, Person, created};
        let (mut state, _rx) = bare_state();
        let alice = Person::new(1);
        let entries = created(&alice, &Person::new(2));
        let anchor = crate::membership_log::digest(&entries[0]).expect("an anchor");
        state.documents.insert(
            DOC,
            SyncDocumentState::new(
                DOC,
                DocumentKey::from_array([0x41; 32]),
                DocType::Prose,
                crate::Existence::LocalOnly,
            ),
        );
        assert_eq!(state.footing(DOC), None);
        let held = crate::key_vouch::Footing::sealed_in_catalogue(DOC, anchor);
        state.documents.get_mut(&DOC).expect("held").footing = Some(held);
        assert_eq!(state.footing(DOC), Some(held));
        let encoded = crate::membership_log::encode(&entries[0]).expect("an encoding");
        let walked = crate::key_vouch::Footing::created(DOC, &encoded, &alice.public())
            .expect("the creator's footing");
        state.log_walked_to(walked);
        assert_eq!(state.footing(DOC), Some(walked));
    }

    /// `SYNC-292`: a user search with no relay logs that it ran, and never
    /// the address it searched.
    #[test]
    fn a_user_search_logs_no_searched_address() {
        let (state, rx) = bare_state();
        let logged = crate::log::tests::logged_by("SearchUsers", || {
            let query = ambre_core::Validated::<ambre_core::UserSearch>::parse("ada@example.org")
                .expect("a search");
            state.handle_search_users(ambre_bus::next_request(), query);
        });
        for message in &logged {
            assert!(
                !message.contains("ada") && !message.contains("example.org"),
                "{message}"
            );
        }
        assert!(
            rx.try_iter().any(|e| matches!(
                e,
                LogicEvent::Rest {
                    evt: RestEvent::UserSearchResults { .. },
                    ..
                }
            )),
            "the search went unanswered"
        );
    }

    /// How the Sync worker ended a waiting invite, if it did.
    #[derive(Debug, PartialEq)]
    enum Ended {
        Landed,
        Refused,
    }

    /// The outcome of a lazy creation the test plays.
    #[derive(Clone, Copy)]
    enum Creation {
        Created,
        Failed,
        ClosedFirst,
    }

    /// `SYNC-246`, `SYNC-247`: an invite on a document the relay does not hold
    /// yet waits for its creation. It goes ahead once the document is
    /// created, and is refused when the creation fails or a close comes first.
    #[test]
    fn an_invite_waits_for_its_document_on_the_relay() {
        let (mut state, rx) = bare_state();
        let ended = |rx: &ambre_bus::conduit::Receiver<LogicEvent>| {
            rx.try_iter()
                .filter_map(|event| match event {
                    LogicEvent::Rest {
                        evt: RestEvent::InviteSent { .. },
                        ..
                    } => Some(Ended::Landed),
                    LogicEvent::ApiError {
                        request: ApiRequest::InviteToDocument,
                        ..
                    } => Some(Ended::Refused),
                    _ => None,
                })
                .collect::<Vec<_>>()
        };
        let key = crate::sharing::own_exchange_key(&StaticSecret::from([2; 32]));
        for creation in [Creation::Created, Creation::Failed, Creation::ClosedFirst] {
            let document_id = Id::<Document>::mint();
            state.documents.insert(
                document_id,
                SyncDocumentState::new(
                    document_id,
                    DocumentKey::from_array([0x41; 32]),
                    DocType::Prose,
                    crate::Existence::LocalOnly,
                ),
            );
            let id = ambre_bus::next_request();
            state.handle_invite_to_document(
                id,
                document_id,
                None,
                Id::<User>::mint(),
                Role::Editor,
                key,
            );
            assert!(
                ended(&rx).is_empty(),
                "the invite went ahead of its document"
            );

            let expected = match creation {
                Creation::Created => {
                    state.handle_sync_event(SyncEvent::DocumentMaterialized { document_id });
                    Ended::Landed
                }
                Creation::Failed => {
                    state.handle_sync_event(SyncEvent::DocumentCreateFailed {
                        document_id,
                        failure: http::Failed::local(ambre_core::LocalFailure::NoRelay),
                    });
                    Ended::Refused
                }
                Creation::ClosedFirst => {
                    state.handle_command(SyncCommand::CloseDocument { document_id });
                    Ended::Refused
                }
            };
            assert_eq!(ended(&rx), vec![expected]);
        }
    }

    /// With a transport but only half of the session keys, no creation can
    /// start. An invite on a local-only document is refused at once, and
    /// waits for nothing.
    #[test]
    fn an_invite_with_half_the_session_keys_is_refused_at_once() {
        let (mut state, rx) = bare_state();
        let (tx, _events) = ambre_bus::conduit::unbounded::<SyncEvent>();
        state.set_event_tx(tx);
        state.exchange_key = Some(StaticSecret::from([2; 32]));
        let document_id = Id::<Document>::mint();
        state.documents.insert(
            document_id,
            SyncDocumentState::new(
                document_id,
                DocumentKey::from_array([0x41; 32]),
                DocType::Prose,
                crate::Existence::LocalOnly,
            ),
        );
        let key = crate::sharing::own_exchange_key(&StaticSecret::from([3; 32]));
        state.handle_invite_to_document(
            ambre_bus::next_request(),
            document_id,
            None,
            Id::<User>::mint(),
            Role::Editor,
            key,
        );
        let refusals: Vec<ApiFailure> = rx
            .try_iter()
            .filter_map(|event| match event {
                LogicEvent::ApiError {
                    request: ApiRequest::InviteToDocument,
                    failure,
                    ..
                } => Some(failure),
                _ => None,
            })
            .collect();
        assert_eq!(
            refusals,
            vec![ApiFailure::Local(ambre_core::LocalFailure::NotSignedIn)]
        );
        assert!(state.documents[&document_id].pending_invites.is_empty());
    }

    /// `SYNC-186`: with no relay, each member request answers the Logic
    /// thread at once, so no view waits on it.
    #[test]
    fn a_member_request_with_no_relay_answers_at_once() {
        let (mut state, rx) = bare_state();
        let (document_id, user) = (Id::<Document>::mint(), Id::<User>::mint());
        let id = Seq::<Request>::new(1);
        let mut answers = Vec::new();
        let mut count = |label: &str| {
            let events: Vec<LogicEvent> = rx.try_iter().collect();
            assert_eq!(events.len(), 1, "{label}: {events:?}");
            answers.extend(events);
        };
        state.handle_fetch_invitee_key(id, document_id, ambre_core::Email::new("a@b.test".into()));
        count("invitee key");
        state.handle_fetch_member_key(id, user);
        count("member key");
        let key = crate::sharing::own_exchange_key(&StaticSecret::from([2; 32]));
        state.handle_invite_to_document(id, document_id, None, user, Role::Editor, key);
        count("invite");
        state.handle_revoke_from_document(id, document_id, user);
        count("revoke");
        state.handle_leave_document(id, document_id);
        count("leave");
        state.handle_update_document_member_role(id, document_id, user, Role::Viewer);
        count("role");
        let search =
            ambre_core::Validated::<ambre_core::UserSearch>::parse("ada").expect("a search");
        state.handle_search_users(id, search);
        count("search");
    }

    /// `SYNC-132`: the signer of log entries is this user, once signed in,
    /// and nobody before.
    #[test]
    fn the_author_is_the_signed_in_user() {
        let (mut state, _rx) = bare_state();
        assert!(state.author().is_none());
        let user = Id::<User>::mint();
        let signing_key = ed25519_dalek::SigningKey::from_bytes(&[4; 32]);
        state.user_id = Some(user);
        assert!(state.author().is_none(), "no signing key yet");
        state.signing_key = Some(signing_key.clone());
        let author = state.author().expect("signed in");
        assert_eq!(author.user_id, user);
        assert_eq!(
            author.public(),
            sharing::Author {
                user_id: user,
                signing_key
            }
            .public()
        );
    }
}
