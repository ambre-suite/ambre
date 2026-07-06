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
    pub(crate) fn handle_invite_to_document(
        &self,
        id: RequestId,
        document_id: DocumentId,
        invitee_email: String,
        role: Role,
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
                request: "InviteToDocument".into(),
                code: 401,
                message: "Not authenticated".into(),
            });
            return;
        };
        let Some(doc_key) = doc_key else {
            let _ = logic_tx.send(LogicEvent::ApiError {
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
        };
        let http = sharing::GlooHttp { auth };
        wasm_bindgen_futures::spawn_local(async move {
            sharing::invite_task(&http, params, &tx, &logic_tx).await;
        });
    }

    /// Revokes a member from a document and rotates the document key:
    /// 1. DELETEs the share at `/api/documents/{id}/share/{uid}`.
    /// 2. Generates a fresh [`DocumentKey`] via CSPRNG.
    /// 3. GETs the remaining members from the server.
    /// 4. Wraps the new key for each remaining member via X25519 ECDH.
    /// 5. POSTs the wrapped keys to `/api/documents/{id}/rotate-key` (the
    ///    server atomically replaces the stored `doc_key_enc` entries and
    ///    broadcasts `WsServerMsg::KeyRotationRequired` to every client).
    /// 6. Emits [`SyncEvent::RevocationComplete`], which causes
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
                request: "RevokeFromDocument".into(),
                code: 401,
                message: "Not authenticated".into(),
            });
            return;
        };

        // Orchestration (DELETE -> new key -> members -> rotate) lives in
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
                        let users: Vec<UserInfo> = resp
                            .users
                            .into_iter()
                            .map(|u| UserInfo {
                                user_id: UserId::parse_str(&u.id).unwrap_or_else(|_| UserId::new()),
                                display_name: u.display_name,
                                email: u.email,
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
