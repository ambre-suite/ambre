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
        document_id: DocumentId,
        invitee_email: String,
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
                request: "FetchInviteeKey".into(),
                code: 503,
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

    pub(crate) fn handle_invite_to_document(
        &self,
        id: RequestId,
        document_id: DocumentId,
        invitee_email: String,
        role: Role,
        expect_exchange_pub_key: [u8; 32],
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
                    user_id: UserId::new(),
                    display_name: invitee_email.clone(),
                },
            );
            return;
        };
        let Some(exchange_key) = exchange_key else {
            let _ = logic_tx.send(LogicEvent::ApiError {
                id: None,
                request: "InviteToDocument".into(),
                code: 401,
                message: "Not authenticated".into(),
            });
            return;
        };
        let Some(doc_key) = doc_key else {
            let _ = logic_tx.send(LogicEvent::ApiError {
                id: None,
                request: "InviteToDocument".into(),
                code: 404,
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
            role,
            exchange_key,
            doc_key,
            expect_exchange_pub_key,
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
        document_id: DocumentId,
        user_id: UserId,
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
        let Some(exchange_key) = exchange_key else {
            let _ = logic_tx.send(LogicEvent::ApiError {
                id: None,
                request: "RevokeFromDocument".into(),
                code: 401,
                message: "Not authenticated".into(),
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
        };
        let http = sharing::GlooHttp { auth };
        wasm_bindgen_futures::spawn_local(async move {
            sharing::revoke_task(&http, params, &tx, &logic_tx).await;
        });
    }

    /// Leaves a document: DELETEs this user's own share and nothing else
    /// (`SYNC-127`, `SYNC-130`). [`SyncEvent::DocumentLeft`] closes it locally.
    pub(crate) fn handle_leave_document(&self, id: RequestId, document_id: DocumentId) {
        let (Some(tx), Some(user_id)) = (self.event_tx.clone(), self.user_id) else {
            let _ = self.logic_tx.send(LogicEvent::ApiError {
                id: None,
                request: "LeaveDocument".into(),
                code: 401,
                message: "Not authenticated".into(),
            });
            return;
        };
        let logic_tx = self.logic_tx.clone();
        let params = sharing::LeaveParams {
            id,
            document_id,
            user_id,
            url_scheme: self.url_scheme,
            api_base: self.api_base.clone(),
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
    /// decoded roles), and replies with [`RestEvent::DocumentMembers`].
    pub(crate) fn handle_list_document_members(&self, id: RequestId, document_id: DocumentId) {
        let event_tx = self.event_tx.clone();
        let api_base = self.api_base.clone();
        let url_scheme = self.url_scheme;
        let auth = self.auth_context();

        if let Some(tx) = event_tx {
            let params = sharing::ListMembersParams {
                id,
                document_id,
                url_scheme,
                api_base,
            };
            let http = sharing::GlooHttp { auth };
            wasm_bindgen_futures::spawn_local(async move {
                sharing::list_members_task(&http, params, &tx).await;
            });
        } else {
            self.reply(
                id,
                RestEvent::DocumentMembers {
                    document_id,
                    members: Vec::new(),
                },
            );
        }
    }

    /// Updates a member's role via `PATCH /api/documents/{id}/members/{uid}`.
    /// Sends the new role as a JSON body and replies with
    /// [`RestEvent::MemberRoleUpdated`] on success.
    pub(crate) fn handle_update_document_member_role(
        &self,
        id: RequestId,
        document_id: DocumentId,
        user_id: UserId,
        role: Role,
    ) {
        let event_tx = self.event_tx.clone();
        let logic_tx = self.logic_tx.clone();
        let api_base = self.api_base.clone();
        let url_scheme = self.url_scheme;
        let auth = self.auth_context();

        if let Some(tx) = event_tx {
            wasm_bindgen_futures::spawn_local(async move {
                let url = format!(
                    "{url_scheme}://{api_base}/api/documents/{document_id}/members/{user_id}"
                );
                let body = serde_json::json!({ "role": role.to_string() });
                match http::patch_json_auth(&url, &body, auth).await {
                    Ok(()) => {
                        let _ = tx.send(SyncEvent::MemberRoleUpdatedEvent {
                            id,
                            document_id,
                            user_id,
                            role,
                        });
                    }
                    Err(e) => {
                        let _ = logic_tx.send(LogicEvent::ApiError {
                            id: None,
                            request: "UpdateDocumentMemberRole".into(),
                            code: 500,
                            message: e,
                        });
                    }
                }
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
    /// [`RestEvent::UserSearchResults`]. The query is percent-encoded for the
    /// `@` character before being placed in the URL.
    pub(crate) fn handle_search_users(&self, id: RequestId, query: String) {
        let event_tx = self.event_tx.clone();
        let api_base = self.api_base.clone();
        let url_scheme = self.url_scheme;
        let auth = self.auth_context();

        if let Some(tx) = event_tx {
            wasm_bindgen_futures::spawn_local(async move {
                let url = format!(
                    "{url_scheme}://{api_base}/api/users/search?query={}",
                    query.replace('@', "%40")
                );
                match http::get_json_auth::<http::UserSearchResult>(&url, auth).await {
                    Ok(resp) => {
                        // A result whose id does not parse is left out, never
                        // given an invented one (`SYNC-038`).
                        let users: Vec<UserInfo> = resp
                            .users
                            .into_iter()
                            .filter_map(|u| {
                                Some(UserInfo {
                                    user_id: UserId::parse_str(&u.id).ok()?,
                                    display_name: u.display_name,
                                    email: u.email,
                                })
                            })
                            .collect();
                        let _ = tx.send(SyncEvent::UserSearchReady { id, users });
                    }
                    Err(e) => {
                        let _ = tx.send(SyncEvent::HttpError { id, message: e });
                    }
                }
            });
        } else {
            tracing::info!("Sync: SearchUsers '{query}'");
            self.reply(id, RestEvent::UserSearchResults { users: Vec::new() });
        }
    }
}
