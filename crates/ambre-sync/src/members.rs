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
        id: RequestId,
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
                failure: ApiFailure::Local,
                message: "Sync transport unavailable".into(),
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
        wasm_bindgen_futures::spawn_local(async move {
            sharing::fetch_invitee_key_task(&http, params, &tx, &logic_tx).await;
        });
    }

    /// Fetch the exchange key of `user_id`, wrapping nothing. Delegates to
    /// [`sharing::fetch_member_key_task`], as the invitee's fetch does.
    pub(crate) fn handle_fetch_member_key(&self, id: RequestId, user_id: Id<User>) {
        let logic_tx = self.logic_tx.clone();
        let Some(tx) = self.event_tx.clone() else {
            let _ = logic_tx.send(LogicEvent::ApiError {
                id: None,
                request: ApiRequest::FetchMemberKey,
                failure: ApiFailure::Local,
                message: "Sync transport unavailable".into(),
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
        wasm_bindgen_futures::spawn_local(async move {
            sharing::fetch_member_key_task(&http, params, &tx, &logic_tx).await;
        });
    }

    pub(crate) fn handle_invite_to_document(
        &self,
        id: RequestId,
        document_id: Id<Document>,
        invitee_email: Option<ambre_core::Email>,
        invitee: Id<User>,
        role: Role,
        expect_exchange_pub_key: ambre_core::ExchangePublicKey,
    ) {
        let event_tx = self.event_tx.clone();
        let logic_tx = self.logic_tx.clone();
        let api_base = self.api_base.clone();
        let url_scheme = self.url_scheme;
        let auth = self.auth_context();
        let exchange_key = self.exchange_key.clone();
        let doc_key = self
            .documents
            .get(&document_id)
            .map(|ds| ds.doc_key.clone());

        let Some(tx) = event_tx else {
            self.reply(
                id,
                RestEvent::InviteSent {
                    document_id,
                    user_id: Id::<User>::mint(),
                    display_name: invitee_email
                        .and_then(|email| ambre_core::DisplayName::cut_to_fit(&email.0))
                        .unwrap_or_else(|| ambre_core::DisplayName::handle_of(&invitee)),
                },
            );
            return;
        };
        let (Some(exchange_key), Some(author)) = (exchange_key, self.author()) else {
            let _ = logic_tx.send(LogicEvent::ApiError {
                id: None,
                request: ApiRequest::InviteToDocument,
                failure: ApiFailure::Local,
                message: "Not authenticated".into(),
            });
            return;
        };
        let Some(doc_key) = doc_key else {
            let _ = logic_tx.send(LogicEvent::ApiError {
                id: None,
                request: ApiRequest::InviteToDocument,
                failure: ApiFailure::Local,
                message: "Document not open".into(),
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
            author,
        };
        let http = sharing::GlooHttp { auth };
        wasm_bindgen_futures::spawn_local(async move {
            sharing::invite_task(&http, params, &tx, &logic_tx).await;
        });
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
        id: RequestId,
        document_id: Id<Document>,
        user_id: Id<User>,
    ) {
        let event_tx = self.event_tx.clone();
        let logic_tx = self.logic_tx.clone();
        let api_base = self.api_base.clone();
        let url_scheme = self.url_scheme;
        let auth = self.auth_context();
        let exchange_key = self.exchange_key.clone();

        let Some(tx) = event_tx else {
            self.reply(id, RestEvent::RevocationComplete { document_id });
            return;
        };
        let (Some(exchange_key), Some(author)) = (exchange_key, self.author()) else {
            let _ = logic_tx.send(LogicEvent::ApiError {
                id: None,
                request: ApiRequest::RevokeFromDocument,
                failure: ApiFailure::Local,
                message: "Not authenticated".into(),
            });
            return;
        };
        // The revoke seals the current key under the new one, so it runs
        // from an open document only (`SYNC-116`).
        let Some(previous) = self
            .documents
            .get(&document_id)
            .map(|doc_state| doc_state.doc_key.clone())
        else {
            let _ = logic_tx.send(LogicEvent::ApiError {
                id: None,
                request: ApiRequest::RevokeFromDocument,
                failure: ApiFailure::Local,
                message: "the document is not open".into(),
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
        };
        let http = sharing::GlooHttp { auth };
        wasm_bindgen_futures::spawn_local(async move {
            sharing::revoke_task(&http, params, &tx, &logic_tx).await;
        });
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
    pub(crate) fn handle_leave_document(&self, id: RequestId, document_id: Id<Document>) {
        let (Some(tx), Some(author), Some(exchange_key)) = (
            self.event_tx.clone(),
            self.author(),
            self.exchange_key.clone(),
        ) else {
            let _ = self.logic_tx.send(LogicEvent::ApiError {
                id: None,
                request: ApiRequest::LeaveDocument,
                failure: ApiFailure::Local,
                message: "Not authenticated".into(),
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
        wasm_bindgen_futures::spawn_local(async move {
            sharing::leave_task(&http, params, &tx, &logic_tx).await;
        });
    }

    /// Fetches the member list for a document via `GET /api/documents/{id}/members`,
    /// maps the server response into [`MemberInfo`] values (plaintext names,
    /// decoded roles), and replies with [`RestEvent::DocumentMembers`]. With no
    /// request id it refreshes the member directory alone, and replies nothing.
    pub(crate) fn handle_list_document_members(
        &mut self,
        id: Option<RequestId>,
        document_id: Id<Document>,
    ) {
        // Number the listing, so an answer older than the directory held
        // changes nothing (`SYNC-037`). A document not open holds no directory.
        let asked = self.documents.get_mut(&document_id).map_or(
            crate::generation::RequestGeneration::ZERO,
            |doc| {
                doc.directory_asked = doc.directory_asked.next();
                doc.directory_asked
            },
        );
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
            };
            let http = sharing::GlooHttp { auth };
            wasm_bindgen_futures::spawn_local(async move {
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
        id: RequestId,
        document_id: Id<Document>,
        user_id: Id<User>,
        role: Role,
    ) {
        let logic_tx = self.logic_tx.clone();
        if let Some(tx) = self.event_tx.clone() {
            let (Some(author), Some(exchange_key)) = (self.author(), self.exchange_key.clone())
            else {
                let _ = logic_tx.send(LogicEvent::ApiError {
                    id: None,
                    request: ApiRequest::UpdateDocumentMemberRole,
                    failure: ApiFailure::Local,
                    message: "Not authenticated".into(),
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
                exchange_key,
                author,
            };
            let http = sharing::GlooHttp {
                auth: self.auth_context(),
            };
            wasm_bindgen_futures::spawn_local(async move {
                sharing::role_change_task(&http, params, &tx, &logic_tx).await;
            });
        } else {
            tracing::info!("Sync: UpdateDocumentMemberRole {document_id:?} {user_id:?} {role:?}");
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
    pub(crate) fn handle_search_users(&self, id: RequestId, query: ambre_core::UserSearch) {
        let event_tx = self.event_tx.clone();
        let api_base = self.api_base.clone();
        let url_scheme = self.url_scheme;
        let auth = self.auth_context();

        if let Some(tx) = event_tx {
            wasm_bindgen_futures::spawn_local(async move {
                let url = crate::http::ApiRoot::of(url_scheme, &api_base)
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
            tracing::info!("Sync: SearchUsers '{}'", query.as_str());
            self.reply(id, RestEvent::UserSearchResults { users: Vec::new() });
        }
    }
}
