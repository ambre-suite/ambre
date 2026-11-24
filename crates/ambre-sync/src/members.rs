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
    ///
    /// On the log of a channel's `meta`, the census first opens each document
    /// of the channel. The revoke then caps the member in each one
    /// (`SYNC-312`). This request names no document of the sealed `meta`, so
    /// a revoke on such a log fails here: the Logic thread sends
    /// `SyncCommand::RevokeFromChannel` instead (`SYNC-320`).
    pub(crate) fn handle_revoke_from_document(
        &mut self,
        id: Seq<Request>,
        document_id: Id<Document>,
        user_id: Id<User>,
    ) {
        self.begin_removal(
            crate::census::Removal::Revoke {
                id,
                document_id,
                user_id,
            },
            None,
        );
    }

    /// The revoke of [`handle_revoke_from_document`](Self::handle_revoke_from_document),
    /// on the documents open here.
    pub(crate) fn revoke_now(
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
            applied: self
                .documents
                .get(&document_id)
                .map(SyncDocumentState::applied_clocks)
                .unwrap_or_default(),
            channel_applied: self.channel_applied(document_id),
        };
        let http = sharing::GlooHttp { auth };
        crate::spawn(async move {
            sharing::revoke_task(&http, params, &tx, &logic_tx).await;
        });
    }

    /// What each other open document of the channel whose `meta` is
    /// `meta` applied, those a census opened included. A revoke on that log
    /// caps its member in each one (`SYNC-311`, `SYNC-312`).
    pub(crate) fn channel_applied(
        &self,
        meta: Id<Document>,
    ) -> Vec<(Id<Document>, sharing::Applied)> {
        self.open_channel_documents(meta)
            .into_iter()
            .filter_map(|other| {
                self.documents
                    .get(&other)
                    .map(|doc| (other, doc.applied_clocks()))
            })
            .collect()
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

    /// Holds `kept`, where the log of `log_document` stood in an earlier
    /// session (`SYNC-290`). Its footing roots every later walk of that log
    /// in the session. A log the relay cut before an entry walked then
    /// installs no key. A footing this session holds already moves on to it
    /// only when it is later.
    ///
    /// The chain it stored is rebuilt, so a listed name opens through it
    /// with no request (`SYNC-308`). A chain the kept log no longer extends
    /// is refused, and so is one of a log this session walked from another
    /// anchor. A chain this session already holds is the later one, and
    /// stays.
    pub(crate) fn hold_kept_log(&mut self, log_document: Id<Document>, kept: ambre_bus::KeptLog) {
        let footing = crate::key_vouch::Footing::kept(log_document, &kept);
        let other_anchor = self
            .log_footings
            .get(&log_document)
            .is_some_and(|held| held.sealed_at(kept.anchor).is_none());
        self.log_walked_to(footing);
        match crate::chain::Chain::kept(log_document, &kept) {
            Some(chain) if !other_anchor => {
                self.kept_chains.entry(log_document).or_insert(chain);
            }
            _ => crate::log::warning!(
                "Sync: the kept chain of {log_document:?} is refused: its log no longer extends it"
            ),
        }
        self.kept_logs.entry(log_document).or_insert(kept);
    }

    /// Records where the log of `document_id` stands now, with the key and
    /// the links of the chain that key walks, within their bound. It hands
    /// that to the local store when it moved (`SYNC-290`, `SYNC-308`,
    /// `SYNC-313`). It is a no-op before a footing or a
    /// key exists.
    pub(crate) fn keep_log(&mut self, document_id: Id<Document>) {
        let (Some(footing), Some((key, links))) = (
            self.footing(document_id),
            self.documents
                .get(&document_id)
                .map(|doc| (doc.doc_key.id(), doc.chain.kept_links(doc.doc_key.id()))),
        ) else {
            return;
        };
        let kept = footing.to_kept(key, links);
        let log_document = footing.document();
        self.keep_chain(document_id);
        if self.kept_logs.get(&log_document) == Some(&kept) {
            return;
        }
        self.kept_logs.insert(log_document, kept.clone());
        self.send_to_logic(LogicEvent::LogKept {
            document_id: log_document,
            kept,
        });
    }

    /// Keeps the links `document_id` holds as the chain of its log, which a
    /// listed name walks with no request (`SYNC-302`). The open kept only
    /// the links a writer of the log signed (`SYNC-121`, `SYNC-204`).
    pub(crate) fn keep_chain(&mut self, document_id: Id<Document>) {
        let log_document = self.log_document(document_id);
        if let Some(doc) = self.documents.get(&document_id) {
            self.kept_chains.insert(log_document, doc.chain.clone());
        }
    }

    /// The chain of links each log kept, by the log's document (`SYNC-302`).
    #[cfg(test)]
    pub(crate) fn kept_chains(&self) -> &HashMap<Id<Document>, crate::chain::Chain> {
        &self.kept_chains
    }

    /// What a listed name opens under, for a listing task that outlives the
    /// borrow. It holds the key each log accepted last (`SYNC-291`) and the
    /// chain each log kept (`SYNC-302`).
    pub(crate) fn kept_names(&self) -> crate::documents::KeptNames {
        crate::documents::KeptNames {
            keys: self.kept_keys(),
            chains: self.kept_chains.clone(),
        }
    }

    /// The key id each log accepted last, by the log's document, for a
    /// listing task that outlives the borrow (`SYNC-291`).
    pub(crate) fn kept_keys(
        &self,
    ) -> HashMap<Id<Document>, ambre_core::FixedBytes<ambre_core::KeyId>> {
        self.kept_logs
            .iter()
            .map(|(document, kept)| (*document, kept.key))
            .collect()
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
    ///
    /// A document not open here opens for a census first, so the departure
    /// names the last clock this user wrote in it (`SYNC-314`).
    pub(crate) fn handle_leave_document(&mut self, id: Seq<Request>, document_id: Id<Document>) {
        self.begin_removal(crate::census::Removal::Leave { id, document_id }, None);
    }

    /// The departure of [`handle_leave_document`](Self::handle_leave_document),
    /// from a document open here.
    pub(crate) fn leave_now(&self, id: Seq<Request>, document_id: Id<Document>) {
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
        // The departure caps this user at the last clock they wrote, which
        // only an open document knows (`SYNC-306`). A cap read from nothing
        // would refuse their own history, so the leave fails closed.
        let Some(cap) = self.leaving_cap(document_id, &author) else {
            let _ = self.logic_tx.send(LogicEvent::ApiError {
                id: Some(id),
                request: ApiRequest::LeaveDocument,
                failure: ApiFailure::Local(ambre_core::LocalFailure::DocumentNotOpen),
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
            cap,
        };
        let http = sharing::GlooHttp {
            auth: self.auth_context(),
        };
        crate::spawn(async move {
            sharing::leave_task(&http, params, &tx, &logic_tx).await;
        });
    }

    /// The cap a departure of `document_id` by `author` names: the last
    /// clock the open document knows that author wrote (`SYNC-306`). `None`
    /// when the document is not open here.
    pub(crate) fn leaving_cap(
        &self,
        document_id: Id<Document>,
        author: &sharing::Author,
    ) -> Option<Seq<DeltaAuthor>> {
        let key = author.public();
        self.documents
            .get(&document_id)
            .map(|doc| doc.own_last_clock(&key))
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

    /// Holds `SYNC-290` in the worker. A footing this session walks goes to
    /// the local store with its key, once, and again only when it moves. The
    /// kept log the store hands the next session roots its walks. A log the
    /// relay cut before the kept head diverges, and the whole log walks.
    #[test]
    fn a_log_head_outlives_the_session_through_the_local_store() {
        use crate::key_vouch::Footing;
        use crate::membership_log::tests::{DOC, Person, append, created};
        use crate::membership_log::{LogRule, digest, encode};
        let (mut state, rx) = bare_state();
        let alice = Person::new(1);
        let mut entries = created(&alice, &Person::new(2));
        append(
            &mut entries,
            &alice,
            &Person::new(3),
            ambre_protocol::MembershipAct::Invite,
            Role::Editor,
        );
        let texts: Vec<_> = entries.iter().map(|e| encode(e).unwrap()).collect();
        let anchor = digest(&entries[0]).expect("an anchor");
        let key = DocumentKey::from_array([0x41; 32]);
        state.documents.insert(
            DOC,
            SyncDocumentState::new(
                DOC,
                key.clone(),
                DocType::Prose,
                crate::Existence::LocalOnly,
            ),
        );
        let walked = Footing::sealed_in_catalogue(DOC, anchor)
            .walk(&texts, &LogRule::document())
            .expect("the whole log walks")
            .footing();
        for _ in 0..2 {
            state.handle_sync_event(SyncEvent::FootingAdvanced {
                document_id: DOC,
                footing: walked,
            });
        }
        let kept: Vec<_> = rx
            .try_iter()
            .filter_map(|event| match event {
                LogicEvent::LogKept { document_id, kept } => Some((document_id, kept)),
                _ => None,
            })
            .collect();
        assert_eq!(kept, [(DOC, walked.to_kept(key.id(), Vec::new()))]);

        // The next session: the store hands the kept log back first.
        let (mut next, _rx) = bare_state();
        next.handle_command(SyncCommand::KeepLog {
            document_id: DOC,
            kept: kept[0].1.clone(),
        });
        let held = next.log_footings.get(&DOC).copied();
        assert_eq!(held, Some(walked));
        assert_eq!(next.kept_keys().get(&DOC), Some(&key.id()));
        let walk = |served: &[ambre_core::Encoded<ambre_core::EncodedLogEntry>]| {
            crate::sharing::walk_from_footing(held, DOC, Some(anchor), served, &LogRule::document())
        };
        assert!(
            matches!(walk(&texts[..2]), Err(crate::error::SyncError::LogDiverged)),
            "a log cut before the kept head installs nothing"
        );
        assert!(walk(&texts).is_ok());
    }

    /// Holds `SYNC-308`. The chain an open holds goes to the local store
    /// beside its kept log. The next session rebuilds it from the row, and
    /// a name sealed two rotations back opens through it. The worker refuses
    /// a stored chain with a link signed past the kept head, or with a link
    /// the kept key does not walk to. It refuses the chain of a log this
    /// session walked from another anchor too. The kept footing still holds.
    #[test]
    fn a_kept_chain_outlives_the_session_and_one_its_log_no_longer_extends_is_refused() {
        use crate::key_vouch::Footing;
        use crate::membership_log::tests::{DOC, Person, append, created};
        use crate::membership_log::{LogRule, digest, encode};
        let (mut state, rx) = bare_state();
        let alice = Person::new(1);
        let mut entries = created(&alice, &Person::new(2));
        append(
            &mut entries,
            &alice,
            &Person::new(3),
            ambre_protocol::MembershipAct::Invite,
            Role::Editor,
        );
        let texts: Vec<_> = entries.iter().map(|e| encode(e).unwrap()).collect();
        let anchor = digest(&entries[0]).expect("an anchor");
        let keys: Vec<DocumentKey> = (0..3).map(|_| DocumentKey::generate()).collect();
        let links: Vec<_> = keys
            .windows(2)
            .map(|pair| {
                crate::pipeline::seal_link(
                    DOC,
                    &pair[0],
                    &pair[1],
                    ambre_protocol::LogHead::EMPTY,
                    &alice.key,
                )
                .unwrap()
            })
            .collect();
        let mut doc = SyncDocumentState::new(
            DOC,
            keys[2].clone(),
            DocType::Prose,
            crate::Existence::LocalOnly,
        );
        doc.chain = crate::chain::Chain::new(DOC, links.clone());
        state.documents.insert(DOC, doc);
        let walked = Footing::sealed_in_catalogue(DOC, anchor)
            .walk(&texts, &LogRule::document())
            .expect("the whole log walks")
            .footing();
        state.handle_sync_event(SyncEvent::FootingAdvanced {
            document_id: DOC,
            footing: walked,
        });
        let kept = rx
            .try_iter()
            .find_map(|event| match event {
                LogicEvent::LogKept { kept, .. } => Some(kept),
                _ => None,
            })
            .expect("the log is kept");
        assert_eq!(kept.links.len(), 2, "the chain rides beside the head");

        // The next session reads the row back and rebuilds the chain.
        let row = ambre_bus::KeptLog::from_row(&kept.to_row().unwrap()).expect("a row");
        let (mut next, _rx) = bare_state();
        next.handle_command(SyncCommand::KeepLog {
            document_id: DOC,
            kept: row.clone(),
        });
        let rebuilt = next.kept_chains().get(&DOC).expect("the chain is rebuilt");
        assert_eq!(
            rebuilt.recover(&keys[2], keys[0].id()).map(|k| k.id()),
            Some(keys[0].id()),
            "two rotations back"
        );

        let refused = |kept: ambre_bus::KeptLog, walked_elsewhere: bool| {
            let (mut next, _rx) = bare_state();
            if walked_elsewhere {
                next.log_walked_to(Footing::sealed_in_catalogue(
                    DOC,
                    ambre_core::FixedBytes::from_array([0xEE; 32]),
                ));
            }
            next.handle_command(SyncCommand::KeepLog {
                document_id: DOC,
                kept,
            });
            assert_eq!(next.kept_keys().get(&DOC), Some(&keys[2].id()));
            next.kept_chains().get(&DOC).is_none()
        };
        let mut past_head = row.clone();
        if let Some(link) = past_head.links.last_mut() {
            link.head.position = ambre_core::Seq::new(99);
        }
        assert!(refused(past_head, false), "a link past the kept head");
        let mut left_over = row.clone();
        left_over.links.push(
            crate::pipeline::seal_link(
                DOC,
                &DocumentKey::generate(),
                &DocumentKey::generate(),
                ambre_protocol::LogHead::EMPTY,
                &alice.key,
            )
            .unwrap(),
        );
        assert!(
            refused(left_over, false),
            "a link the kept key never reaches"
        );
        assert!(
            refused(row.clone(), true),
            "a log walked from another anchor"
        );
        assert!(!refused(row, false), "the honest chain holds");
    }

    /// Holds `SYNC-313`. A document was rotated forty times, and its chain
    /// holds a link before a history boundary too. The row keeps the
    /// nearest [`crate::chain::KEPT_LINKS_LIMIT`] links of the walk back
    /// from its key, and drops the rest. The next session accepts the row
    /// under the checks of the reload. It reaches the key that many
    /// rotations back, and no older one.
    #[test]
    fn the_row_of_a_kept_log_keeps_the_nearest_links_its_key_walks() {
        use crate::chain::KEPT_LINKS_LIMIT;
        use crate::key_vouch::Footing;
        use crate::membership_log::tests::{DOC, Person, created};
        use crate::membership_log::{LogRule, digest, encode};
        let (mut state, rx) = bare_state();
        let alice = Person::new(1);
        let entries = created(&alice, &Person::new(2));
        let texts: Vec<_> = entries.iter().map(|e| encode(e).unwrap()).collect();
        let anchor = digest(&entries[0]).expect("an anchor");
        let bound = usize::try_from(KEPT_LINKS_LIMIT.get()).expect("a small bound");
        let rotations = bound + 8;
        let keys: Vec<DocumentKey> = (0..=rotations).map(|_| DocumentKey::generate()).collect();
        let seal = |previous: &DocumentKey, next: &DocumentKey| {
            crate::pipeline::seal_link(
                DOC,
                previous,
                next,
                ambre_protocol::LogHead::EMPTY,
                &alice.key,
            )
            .unwrap()
        };
        let mut links: Vec<_> = keys
            .windows(2)
            .map(|pair| seal(&pair[0], &pair[1]))
            .collect();
        // A rotation with no link cut the chain: this link is out of reach.
        links.push(seal(&DocumentKey::generate(), &DocumentKey::generate()));
        let current = keys.last().expect("a current key");
        let mut doc = SyncDocumentState::new(
            DOC,
            current.clone(),
            DocType::Prose,
            crate::Existence::LocalOnly,
        );
        doc.chain = crate::chain::Chain::new(DOC, links);
        state.documents.insert(DOC, doc);
        let walked = Footing::sealed_in_catalogue(DOC, anchor)
            .walk(&texts, &LogRule::document())
            .expect("the whole log walks")
            .footing();
        state.handle_sync_event(SyncEvent::FootingAdvanced {
            document_id: DOC,
            footing: walked,
        });
        let kept = rx
            .try_iter()
            .find_map(|event| match event {
                LogicEvent::LogKept { kept, .. } => Some(kept),
                _ => None,
            })
            .expect("the log is kept");
        assert_eq!(kept.links.len(), bound, "the row holds the bound");
        let nearest: Vec<_> = keys
            .iter()
            .rev()
            .take(bound + 1)
            .map(DocumentKey::id)
            .collect();
        assert!(
            kept.links
                .iter()
                .zip(nearest.windows(2))
                .all(|(link, pair)| link.key == pair[0] && link.previous == pair[1]),
            "nearest first, along the walk"
        );
        // The ceiling of `ambre-spec-budgets.md section 6`: each link of
        // this row names an empty head, and a head costs at most 41 B more.
        let bytes = kept.to_row().unwrap().as_bytes().len();
        assert!(bytes + bound * 41 <= 12 * 1024, "{bytes} B");

        let row = ambre_bus::KeptLog::from_row(&kept.to_row().unwrap()).expect("a row");
        let (mut next, _rx) = bare_state();
        next.handle_command(SyncCommand::KeepLog {
            document_id: DOC,
            kept: row,
        });
        let rebuilt = next
            .kept_chains()
            .get(&DOC)
            .expect("the pruned chain holds");
        let oldest_kept = &keys[rotations - bound];
        assert_eq!(
            rebuilt.recover(current, oldest_kept.id()).map(|k| k.id()),
            Some(oldest_kept.id()),
            "the key the bound reaches"
        );
        let past = &keys[rotations - bound - 1];
        assert!(
            rebuilt.recover(current, past.id()).is_none(),
            "no key past the bound"
        );
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
