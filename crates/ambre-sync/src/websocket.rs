use super::*;
use zeroize::Zeroizing;

/// The socket address, whose query carries the access token. Its `Display`
/// and `Debug` print the address without the query, so logging it logs no
/// token (`SYNC-061`), and the text is wiped when dropped.
pub(crate) struct SocketUrl(Zeroizing<String>);

impl SocketUrl {
    /// `{scheme}://{api_base}/api/ws?token={token}`, the token already encoded.
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    pub(crate) fn new(scheme: &str, api_base: &str, token: &str) -> Self {
        Self(Zeroizing::new(format!(
            "{scheme}://{api_base}/api/ws?token={token}"
        )))
    }

    /// The whole address, for the one call that opens the socket.
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    pub(crate) fn expose(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for SocketUrl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let address = self
            .0
            .split_once('?')
            .map_or(self.0.as_str(), |(address, _)| address);
        write!(f, "{address}?token=<redacted>")
    }
}

impl std::fmt::Debug for SocketUrl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(self, f)
    }
}

impl SyncState {
    // -- WebSocket lifecycle ---------------------------------------------------

    /// Opens the session-level multiplexed WebSocket. Called after Unlock or
    /// SetupAccount. If the socket is already present (reconnection), it is
    /// replaced. All open documents are automatically re-subscribed.
    #[cfg(target_arch = "wasm32")]
    pub(crate) fn open_websocket(&mut self) {
        if self.update_owed {
            return;
        }
        // Mirror the REST scheme: `ws` on localhost (http), `wss` otherwise.
        // A hardcoded `wss://` fails on a localhost http dev server (TLS error,
        // WS close code 1015).
        let ws_scheme = if self.url_scheme == "http" {
            "ws"
        } else {
            "wss"
        };
        let Some(access_token) = self
            .tokens
            .borrow()
            .as_ref()
            .map(|t| t.access_token.clone())
        else {
            tracing::warn!("Sync: WebSocket open requested before access token is ready");
            return;
        };
        let token = Zeroizing::new(
            js_sys::encode_uri_component(access_token.expose())
                .as_string()
                .unwrap_or_else(|| access_token.expose().to_owned()),
        );
        let url = SocketUrl::new(ws_scheme, &self.api_base, &token);
        match WebSocket::new(url.expose()) {
            Ok(ws) => {
                ws.set_binary_type(web_sys::BinaryType::Arraybuffer);
                self.wire_ws_callbacks(&ws);
                self.ws = Some(ws);
                // Subscriptions are (re)sent from the `onopen` handler, NOT here.
                // The socket is still CONNECTING at this point and `ws_send` would
                // be dropped. The server would then have a connection but no
                // subscription, and every later Update gets 403 "not subscribed".
                tracing::info!("Sync: WebSocket connecting ({url})");
            }
            Err(e) => {
                tracing::error!("Sync: WebSocket::new({url}) failed: {e:?}");
            }
        }
    }

    /// Native / host-test stub: does nothing (mock transport handles WS).
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn open_websocket(&mut self) {
        if self.update_owed {
            return;
        }
        self.socket_opens += 1;
        tracing::info!("Sync: WebSocket opened (native mock)");
    }

    /// (Re)subscribes every server-side document once the socket is OPEN, and
    /// pulls anything missed while disconnected. Called from the `onopen` handler
    /// so it works identically for the first connection and every reconnect
    /// (after a network drop or a silent reauth). Local-only documents are not
    /// subscribed here -- they subscribe after lazy creation materializes them.
    pub(crate) fn resubscribe_all(&mut self) {
        for (doc_id, last_seq) in self
            .documents
            .iter()
            .filter(|(_, d)| d.server_created)
            .map(|(id, d)| (*id, d.last_seq))
            .collect::<Vec<_>>()
        {
            self.ws_send(&WsClientMsg::Subscribe {
                document_id: doc_id,
            });
            self.ws_send(&WsClientMsg::RequestMissing {
                document_id: doc_id,
                after_seq: last_seq,
            });
        }
    }

    /// Attaches onmessage / onclose / onerror closures to `ws`. Each closure
    /// pushes the decoded event into `self.event_tx`, so the non-blocking select
    /// loop in [`run_wasm`] processes them without blocking the JS event loop.
    #[cfg(target_arch = "wasm32")]
    pub(crate) fn wire_ws_callbacks(&mut self, ws: &WebSocket) {
        let event_tx = self.event_tx.clone();

        if let Some(tx) = event_tx.clone() {
            let onmessage = Closure::wrap(Box::new(move |e: JsValue| {
                let msg: MessageEvent = match e.dyn_into() {
                    Ok(m) => m,
                    Err(_) => return,
                };
                let data = msg.data();
                let array_buf: js_sys::ArrayBuffer = match data.dyn_into() {
                    Ok(a) => a,
                    Err(_) => return,
                };
                let bytes = js_sys::Uint8Array::new(&array_buf).to_vec();
                match decode_server_msg(&bytes) {
                    Ok(server_msg) => {
                        let _ = tx.send(SyncEvent::WsMessage(server_msg));
                    }
                    Err(e) if e.is_from_newer_peer() => {
                        let _ = tx.send(SyncEvent::WireOutdated);
                    }
                    Err(e) => tracing::error!("Sync: WS decode error: {e:?}"),
                }
            }) as Box<dyn FnMut(JsValue)>);
            ws.set_onmessage(Some(onmessage.as_ref().unchecked_ref()));
            self._onmessage = Some(onmessage);
        }

        if let Some(tx) = event_tx.clone() {
            let onclose = Closure::wrap(Box::new(move |e: JsValue| {
                let ce: CloseEvent = match e.dyn_into() {
                    Ok(c) => c,
                    Err(_) => return,
                };
                let _ = tx.send(SyncEvent::WsDisconnected {
                    code: ce.code(),
                    reason: ce.reason(),
                });
            }) as Box<dyn FnMut(JsValue)>);
            ws.set_onclose(Some(onclose.as_ref().unchecked_ref()));
            self._onclose = Some(onclose);
        }

        if let Some(tx) = event_tx.clone() {
            let onerror = Closure::wrap(Box::new(move |_e: JsValue| {
                let _ = tx.send(SyncEvent::WsError);
            }) as Box<dyn FnMut(JsValue)>);
            ws.set_onerror(Some(onerror.as_ref().unchecked_ref()));
            self._onerror = Some(onerror);
        }

        if let Some(tx) = event_tx {
            let onopen = Closure::wrap(Box::new(move |_e: JsValue| {
                let _ = tx.send(SyncEvent::WsOpened);
            }) as Box<dyn FnMut(JsValue)>);
            ws.set_onopen(Some(onopen.as_ref().unchecked_ref()));
            self._onopen = Some(onopen);
        }
    }

    /// Schedules a reconnect attempt after an exponential backoff (with jitter)
    /// so a rejected upgrade does not trigger a tight reconnect storm. An HTTP
    /// 429 from the rate limiter is one. Each failed attempt lengthens the delay;
    /// a successful open resets the counter via [`SyncEvent::WsConnected`].
    #[cfg(target_arch = "wasm32")]
    pub(crate) fn schedule_reconnect(&mut self) {
        use wasm_bindgen::JsCast;

        let attempt = self.reconnect_attempts;
        self.reconnect_attempts = attempt.saturating_add(1);

        // 250 ms base, doubling, capped at 10 s, plus up to 100% jitter.
        let base = 250u32.saturating_mul(1u32 << attempt.min(6));
        let capped = base.min(10_000);
        let jitter = (js_sys::Math::random() * capped as f64) as u32;
        let delay_ms = (capped + jitter) as i32;

        let Some(tx) = self.event_tx.clone() else {
            return;
        };
        let cb = Closure::wrap(Box::new(move || {
            let _ = tx.send(SyncEvent::ReconnectNow);
        }) as Box<dyn FnMut()>);

        if let Ok(scope) = js_sys::global().dyn_into::<web_sys::DedicatedWorkerGlobalScope>() {
            let _ = scope.set_timeout_with_callback_and_timeout_and_arguments_0(
                cb.as_ref().unchecked_ref(),
                delay_ms,
            );
            tracing::warn!(
                "Sync: reconnect scheduled in {delay_ms}ms (attempt {})",
                attempt + 1
            );
        }
        // Keep the closure alive until it fires (replaces any previous pending one).
        self._reconnect_timer = Some(cb);
    }

    /// Native / host-test stub: reconnect immediately (mock transport, no timer).
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn schedule_reconnect(&mut self) {
        self.open_websocket();
    }

    /// Silently renews the access token after a WS auth rejection (close
    /// [`WS_CLOSE_UNAUTHORIZED`]) and signals the outcome via
    /// [`SyncEvent::Reauthenticated`]. Runs the refresh off the select loop so it
    /// does not block; the shared token store is updated in place on success.
    #[cfg(target_arch = "wasm32")]
    pub(crate) fn reauthenticate(&mut self) {
        let Some(tx) = self.event_tx.clone() else {
            return;
        };
        let auth = self.auth_context();
        wasm_bindgen_futures::spawn_local(async move {
            let ok = match auth.refresh().await {
                Ok(()) => true,
                Err(e) => {
                    tracing::warn!("Sync: token refresh failed: {e}");
                    false
                }
            };
            let _ = tx.send(SyncEvent::Reauthenticated { ok });
        });
    }

    /// Native / host-test stub: no token refresh, just reconnect.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn reauthenticate(&mut self) {
        self.open_websocket();
    }

    /// Called by the select loop on a `WsDisconnected` event. Clears the socket
    /// and notifies the Logic thread that every open document lost its
    /// connection. A reconnection is attempted immediately.
    #[allow(dead_code)]
    pub(crate) fn handle_ws_disconnected(&mut self, code: u16, reason: String) {
        tracing::warn!("Sync: WS disconnected (code={code}): {reason}");
        self.ws = None;
        self._onmessage = None;
        self._onclose = None;
        self._onerror = None;
        for doc_id in self.documents.keys().copied().collect::<Vec<_>>() {
            self.send_to_logic(LogicEvent::ConnectionStatus {
                document_id: doc_id,
                connected: false,
            });
        }
    }

    /// Processes one [`SyncEvent`] received from the internal event queue.
    /// Dispatches WebSocket frames to the existing [`handle_ws_message`]
    /// pipeline, applies state mutations from HTTP result events, and handles
    /// disconnection / errors.
    #[allow(dead_code)]
    pub(crate) fn handle_sync_event(&mut self, event: SyncEvent) {
        match event {
            SyncEvent::WsMessage(msg) => self.handle_ws_message(msg),
            SyncEvent::WsOpened => self.resubscribe_all(),
            SyncEvent::WireOutdated => {
                if self.update_owed {
                    return;
                }
                // Every later frame would fail the same way, and no retry
                // cures it. Close for good, and say once that an update is
                // owed (`SYNC-137`).
                self.update_owed = true;
                if let Some(ws) = self.ws.take() {
                    ws.set_onmessage(None);
                    ws.set_onclose(None);
                    ws.set_onerror(None);
                    let _ = ws.close();
                }
                self.handle_ws_disconnected(0, "wire outdated".to_owned());
                self.send_to_logic(LogicEvent::UpdateRequired);
            }
            SyncEvent::WsDisconnected { code, reason } => {
                self.handle_ws_disconnected(code, reason);
                if code == WS_CLOSE_SETUP_REQUIRED {
                    // A renewed session would have no keys either, so neither
                    // renew nor retry. The socket reopens once a session with
                    // keys is stored (`SYNC-133`). Meanwhile the rest of the
                    // client is told a setup is owed (`SYNC-134`).
                    self.setup_owed = true;
                    self.send_to_logic(LogicEvent::SetupRequired);
                } else if self.documents.is_empty() {
                    // No open document: nothing to reconnect for.
                } else if code == WS_CLOSE_UNAUTHORIZED {
                    // The server rejected our (expired/rotated) JWT. Refresh the
                    // token silently and reopen, instead of hammering with a
                    // dead token (which the backoff alone could never fix).
                    self.reauthenticate();
                } else {
                    self.schedule_reconnect();
                }
            }
            SyncEvent::Reauthenticated { ok } => {
                if ok {
                    // New access token in the shared store: reopen promptly.
                    self.reconnect_attempts = 0;
                    if self.ws.is_none() && !self.documents.is_empty() && !self.setup_owed {
                        self.open_websocket();
                    }
                } else if let Some(username) = self.dev_username.clone() {
                    // Refresh token also dead (for example server restarted with a fresh
                    // ephemeral key and empty Redis): re-login with the same
                    // deterministic dev identity. SessionReady reopens the socket.
                    tracing::warn!(
                        "Sync: refresh failed, re-running dev login for silent recovery"
                    );
                    self.handle_dev_login(RequestId::next(), username);
                } else {
                    // Real session with no usable refresh: surface so the UI can
                    // prompt re-auth. Fall back to backoff to avoid a tight loop.
                    tracing::error!("Sync: reauth failed and no dev fallback; backing off");
                    self.schedule_reconnect();
                }
            }
            SyncEvent::WsError => {
                tracing::error!("Sync: WebSocket error");
                // A raw socket `onerror` is not always followed by a clean
                // `onclose`, so treat it as a disconnect. Tear down the dead
                // socket and reconnect on the same backoff path as
                // `WsDisconnected` (M10). Without the teardown, an `onerror`
                // with no close would leave `self.ws` set and stall the
                // `ReconnectNow` guard (`self.ws.is_none()`). A following
                // `onclose`, if any, is idempotent here.
                self.handle_ws_disconnected(0, "ws error".to_owned());
                if !self.documents.is_empty() {
                    self.schedule_reconnect();
                }
            }
            SyncEvent::ReconnectNow => {
                self._reconnect_timer = None;
                // Only reconnect if still wanted and not already reconnected. A
                // timer armed before a setup refusal must not retry it.
                if self.ws.is_none() && !self.documents.is_empty() && !self.setup_owed {
                    self.open_websocket();
                }
            }
            SyncEvent::SessionReady {
                id,
                user_id,
                display_name,
                email,
                master_seed,
                signing_key,
                exchange_key,
                tokens,
            } => {
                self.user_id = Some(user_id);
                self.master_seed = Some(master_seed);
                self.signing_key = Some(signing_key);
                self.exchange_key = Some(exchange_key);
                *self.tokens.borrow_mut() = Some(tokens);
                self.setup_owed = false;
                self.open_websocket();
                self.flush_pending_opens();
                self.reply(
                    id,
                    RestEvent::SessionReady {
                        user_id,
                        display_name,
                        email,
                    },
                );
            }
            SyncEvent::ProviderSignedIn {
                id,
                user_id,
                display_name,
                email,
                keys_stored,
                tokens,
            } => {
                // The tokens stop here (`CHROME-096`). No key is held yet, so no
                // socket opens: the unlock or the setup opens it.
                self.user_id = Some(user_id);
                *self.tokens.borrow_mut() = Some(tokens);
                self.setup_owed = !keys_stored;
                self.reply(
                    id,
                    RestEvent::ProviderSignedIn {
                        user_id,
                        display_name,
                        email,
                        keys_stored,
                    },
                );
            }
            SyncEvent::AccountCreated {
                id,
                recovery_words,
                master_seed,
                signing_key,
                exchange_key,
                tokens,
            } => {
                self.master_seed = Some(master_seed);
                self.signing_key = Some(signing_key);
                self.exchange_key = Some(exchange_key);
                *self.tokens.borrow_mut() = Some(tokens);
                self.setup_owed = false;
                self.open_websocket();
                self.flush_pending_opens();
                self.reply(id, RestEvent::AccountCreated { recovery_words });
            }
            SyncEvent::DocumentList { id, documents } => {
                self.reply(id, RestEvent::DocumentList { documents });
            }
            SyncEvent::DocumentCreated {
                id,
                document_id,
                doc_type,
            } => {
                self.reply(
                    id,
                    RestEvent::DocumentCreated {
                        document_id,
                        doc_type,
                    },
                );
            }
            SyncEvent::DocumentDeleted { id, document_id } => {
                self.reply(id, RestEvent::DocumentDeleted { document_id });
            }
            SyncEvent::DocumentRenamed { id, document_id } => {
                self.reply(id, RestEvent::DocumentRenamed { document_id });
            }
            SyncEvent::OpenDocumentReady {
                document_id,
                doc_key,
                snapshot_payload,
                snapshot_id,
                covered_seq,
                doc_type,
                server_created,
                signing_keys,
                role,
            } => {
                self.documents.insert(
                    document_id,
                    SyncDocumentState::new(document_id, doc_key, doc_type, server_created),
                );
                let doc_state = self.documents.get_mut(&document_id).expect("just inserted");
                doc_state.set_signing_keys(signing_keys);
                doc_state.role = role;
                // The blob object storage holds is the sealed snapshot. It is
                // opened here, and only a snapshot that opens becomes the known
                // parent and gates the replay (`SYNC-030`). One that does not
                // falls back to the full replay from genesis, so no history
                // is skipped on the strength of a snapshot nobody applied.
                let opened = snapshot_payload.map(|blob| doc_state.open_snapshot_blob(&blob));
                match opened {
                    Some(Ok(payload)) => {
                        doc_state.current_snapshot_id = snapshot_id;
                        doc_state.known_snapshots.insert(snapshot_id);
                        // Advance the replay cursor past the history folded into
                        // the snapshot. The RequestMissing below then fetches only
                        // newer deltas instead of replaying everything since
                        // genesis.
                        doc_state.last_seq = covered_seq;
                        self.send_to_logic(LogicEvent::RemoteSnapshot {
                            document_id,
                            payload,
                        });
                    }
                    Some(Err(e)) => {
                        self.send_to_logic(LogicEvent::SyncWarning {
                            document_id,
                            code: SyncWarningCode::DecryptFailed,
                            message: format!("the latest snapshot does not open: {e}"),
                        });
                    }
                    None => {
                        doc_state.current_snapshot_id = snapshot_id;
                        doc_state.known_snapshots.insert(snapshot_id);
                    }
                }
                // Only subscribe over WS if the document actually exists
                // server-side; a local-only doc subscribes after lazy creation.
                let awaiting_replay = server_created && self.ws_connected();
                if awaiting_replay {
                    self.ws_send(&WsClientMsg::Subscribe { document_id });
                    // Subscribing alone does not replay history (the server only
                    // forwards live updates to subscribers). On a fresh open the
                    // loaded snapshot (if any) covers state up to its own point.
                    // Any deltas committed after it must be pulled explicitly. That
                    // includes the whole history when no snapshot was ever
                    // uploaded. `last_seq` was just set to the snapshot's covered
                    // seq (0 when none), so this requests only the deltas the
                    // snapshot does not already contain.
                    let after_seq = self
                        .documents
                        .get(&document_id)
                        .map(|d| d.last_seq)
                        .unwrap_or(Seq(0));
                    self.ws_send(&WsClientMsg::RequestMissing {
                        document_id,
                        after_seq,
                    });
                }
                self.send_to_logic(LogicEvent::ConnectionStatus {
                    document_id,
                    connected: self.ws_connected(),
                });
                // Hydration-complete signal (dismisses the UI loading overlay).
                // When a replay was requested above, defer it to the matching
                // `MissingUpdates`. The document is only fully hydrated once the
                // deltas the snapshot does not cover (the whole history for a
                // delta-only doc) have arrived. Without a replay (local-only or
                // offline open) there is nothing to wait for, so signal now.
                if awaiting_replay {
                    if let Some(doc_state) = self.documents.get_mut(&document_id) {
                        doc_state.awaiting_initial_hydration = true;
                    }
                } else {
                    self.send_to_logic(LogicEvent::DocumentReady { document_id, role });
                }
                // A local-only doc issues no RequestMissing (nothing to replay),
                // so flush any buffered pre-session updates here -- they will take
                // the lazy-create path. A server-side doc instead flushes from the
                // `MissingUpdates` handler, after its clock has caught up.
                if !server_created {
                    self.flush_pre_session_updates(document_id);
                }
            }
            SyncEvent::OpenDocumentFailed {
                document_id,
                message,
            } => {
                tracing::error!("Sync: open document {document_id:?} failed: {message}");
                self.send_to_logic(LogicEvent::AuthError { message });
            }
            SyncEvent::DocumentMaterialized { document_id } => {
                let pending = match self.documents.get_mut(&document_id) {
                    Some(doc_state) => {
                        doc_state.server_created = true;
                        doc_state.creating = false;
                        std::mem::take(&mut doc_state.pending_updates)
                    }
                    None => Vec::new(),
                };
                if self.ws_connected() {
                    self.ws_send(&WsClientMsg::Subscribe { document_id });
                }
                tracing::info!(
                    "Sync: document {document_id:?} materialized, flushing {} update(s)",
                    pending.len()
                );
                for payload in pending {
                    self.handle_send_update(document_id, payload);
                }
            }
            SyncEvent::DocumentCreateFailed {
                document_id,
                message,
            } => {
                if let Some(doc_state) = self.documents.get_mut(&document_id) {
                    doc_state.creating = false;
                }
                tracing::error!("Sync: lazy create failed for {document_id:?}: {message}");
            }
            SyncEvent::HttpError { id: _id, message } => {
                self.send_to_logic(LogicEvent::ApiError {
                    id: None,
                    request: "rest".into(),
                    code: 500,
                    message,
                });
            }
            SyncEvent::InviteeKeyReady {
                id,
                document_id,
                user_id,
                display_name,
                email,
                exchange_pub_key,
                fingerprint,
            } => {
                self.reply(
                    id,
                    RestEvent::InviteeKey {
                        document_id,
                        user_id,
                        display_name,
                        email,
                        exchange_pub_key,
                        fingerprint,
                    },
                );
            }
            SyncEvent::InviteSent {
                id,
                document_id,
                user_id,
                display_name,
            } => {
                self.reply(
                    id,
                    RestEvent::InviteSent {
                        document_id,
                        user_id,
                        display_name,
                    },
                );
            }
            SyncEvent::RevocationComplete {
                id,
                document_id,
                new_doc_key,
            } => {
                if let Some(doc_state) = self.documents.get_mut(&document_id) {
                    doc_state.rotate_key(new_doc_key);
                }
                self.reply(id, RestEvent::RevocationComplete { document_id });
            }
            SyncEvent::DocumentMembersReady {
                id,
                document_id,
                members,
                signing_keys,
            } => {
                // Keep presence resolvable as membership evolves. A document that
                // is not open locally has no session to refresh -- the next open
                // rebuilds the directory from scratch.
                if let Some(doc_state) = self.documents.get_mut(&document_id) {
                    doc_state.set_signing_keys(signing_keys);
                }
                self.reply(
                    id,
                    RestEvent::DocumentMembers {
                        document_id,
                        members,
                    },
                );
            }
            SyncEvent::UserSearchReady { id, users } => {
                self.reply(id, RestEvent::UserSearchResults { users });
            }
            SyncEvent::MemberRoleUpdatedEvent {
                id,
                document_id,
                user_id,
                role,
            } => {
                self.reply(
                    id,
                    RestEvent::MemberRoleUpdated {
                        document_id,
                        user_id,
                        role,
                    },
                );
            }
            SyncEvent::KeyRotated {
                document_id,
                new_doc_key,
            } => {
                // Swap the local key cache. This is the remaining-member path: a
                // rotation is not a revocation. Nothing is reported as a member
                // loss here; a revoked user is told via `DocumentRevoked`.
                if let Some(doc_state) = self.documents.get_mut(&document_id) {
                    doc_state.rotate_key(new_doc_key);
                }
            }
            SyncEvent::MediaUploaded {
                id,
                document_id,
                content_hash,
            } => {
                self.reply(
                    id,
                    RestEvent::MediaUploaded {
                        document_id,
                        content_hash,
                    },
                );
            }
            SyncEvent::MediaDownloaded {
                id,
                document_id,
                content_hash,
                data,
            } => {
                self.reply(
                    id,
                    RestEvent::MediaDownloaded {
                        document_id,
                        content_hash,
                        data,
                    },
                );
            }
            SyncEvent::SnapshotFetched {
                document_id,
                snapshot_id,
                blob,
            } => self.adopt_fetched_snapshot(document_id, snapshot_id, blob),
            SyncEvent::SnapshotUploaded {
                document_id,
                snapshot_id,
            } => {
                // Adopt the freshly uploaded snapshot as the current parent and
                // release anything quarantined under it (mirrors the remote
                // `SnapshotAvailable` path).
                self.snapshot_known(document_id, snapshot_id);
            }
        }
    }

    // -- Inbound: Server -> Sync (WebSocket demux) -----------------------------

    /// Demultiplex an inbound [`WsServerMsg`] received from the WebSocket.
    /// Decoding from wire bytes is done in the `onmessage` callback;
    /// this method receives the already-decoded message.
    #[allow(dead_code)]
    pub(crate) fn handle_ws_message(&mut self, server_msg: WsServerMsg) {
        // Any frame from the server proves the connection is authenticated and
        // live, so the reconnect backoff resets. A bare `onopen` is NOT enough:
        // an auth-rejected upgrade still returns 101 (then the server drops it),
        // firing `onopen` without ever sending a frame. Resetting on `onopen`
        // there would spin a tight reconnect loop (storm). A rejected connection
        // never reaches this point, so its backoff keeps growing as intended.
        self.reconnect_attempts = 0;
        match server_msg {
            WsServerMsg::Subscribed { document_id } => {
                self.send_to_logic(LogicEvent::ConnectionStatus {
                    document_id,
                    connected: true,
                });
            }

            WsServerMsg::Update { seq, update } => {
                tracing::debug!(
                    "[Sync<-Server] WS Update seq={} doc={} clock={}",
                    seq.0,
                    update.public_data.document_id,
                    update.public_data.clock.0,
                );
                self.receive_update(seq, update);
            }

            WsServerMsg::Ephemeral(eph) => {
                let doc_id = eph.public_data.document_id;
                let Some(doc_state) = self.documents.get_mut(&doc_id) else {
                    return;
                };

                // Anti-replay: reject ephemerals with a non-monotonic counter for
                // the same author. The counter map is bounded (see
                // `accept_ephemeral_counter` / `MAX_EPHEMERAL_REMOTE_AUTHORS`).
                let author_pk = *eph.public_data.pub_key.as_array();
                if !doc_state.accept_ephemeral_counter(author_pk, eph.public_data.counter.0) {
                    tracing::debug!(
                        "Sync: dropping replayed ephemeral from {:?} (counter {})",
                        author_pk,
                        eph.public_data.counter.0
                    );
                    return;
                }

                // Presence is keyed by the sender's canonical `UserId`
                // (`ambre-spec-membership.md`), resolved from the authoring
                // Ed25519 key before delivery so a peer cannot spoof another
                // user's cursor. The client-side key directory (member signing
                // keys / trusted-keys) is not wired yet. An unattributable
                // frame is dropped rather than shown under a guessed identity.
                // Presence is best-effort; a dropped frame is corrected by the
                // next one.
                let Some(from_user_id) = doc_state.resolve_ephemeral_author(author_pk) else {
                    tracing::debug!(
                        "Sync: dropping ephemeral from unattributable author {author_pk:?}"
                    );
                    return;
                };
                let opened = doc_state.open_ephemeral_named(&eph);
                self.deliver_remote(doc_id, Some(opened), |payload| {
                    LogicEvent::RemoteEphemeral {
                        document_id: doc_id,
                        from_user_id,
                        payload,
                    }
                });
            }

            WsServerMsg::PeerLeft {
                document_id,
                pub_key,
            } => {
                // A departure is attributed by the same path as a cursor: the
                // relay names the key, and the member list resolves it
                // (`PRESENCE-014`, `PRESENCE-003`). A key we cannot attribute is
                // dropped -- the peer then ages out of the live set, which is
                // what happened before this message existed.
                let Some(doc_state) = self.documents.get(&document_id) else {
                    return;
                };
                let Some(user_id) = doc_state.resolve_ephemeral_author(*pub_key.as_array()) else {
                    tracing::debug!("Sync: dropping PeerLeft from an unattributable key");
                    return;
                };
                self.send_to_logic(LogicEvent::PeerLeft {
                    document_id,
                    user_id,
                });
            }

            WsServerMsg::MissingUpdates {
                document_id,
                updates,
            } => {
                // Decrypt the whole replay, then deliver it as one batch so the
                // UI repaints once, not once per delta. A cell that only appears
                // in the last delta would otherwise flicker in. Updates whose
                // parent snapshot is unknown are held in quarantine by
                // `ingest_update` and released later. Decrypt failures surface as
                // their own `SyncWarning`, preserving the per-update path.
                let mut payloads: Vec<Vec<u8>> = Vec::new();
                for seq_update in updates {
                    match self.ingest_update(seq_update.seq, seq_update.update) {
                        Some(Ok(payload)) => payloads.push(payload),
                        Some(Err(e)) => self.deliver_remote(document_id, Some(Err(e)), |_| {
                            unreachable!("Err arm never builds an event")
                        }),
                        None => {}
                    }
                }
                if !payloads.is_empty() {
                    self.send_to_logic(LogicEvent::RemoteUpdateBatch {
                        document_id,
                        payloads,
                    });
                }
                // The replay has advanced `local_clock` past everything the
                // server already holds for us. Any update buffered before the
                // session was ready can now be sealed with a fresh, conflict-free
                // clock.
                self.flush_pre_session_updates(document_id);
                // Initial catch-up done: the document is now fully hydrated
                // (snapshot + every delta the server holds). Signal the UI to
                // dismiss its loading overlay -- exactly once, on the first
                // replay after open. A later reconnect replay leaves the flag
                // cleared, so no duplicate fires.
                if let Some(doc_state) = self.documents.get_mut(&document_id)
                    && doc_state.awaiting_initial_hydration
                {
                    doc_state.awaiting_initial_hydration = false;
                    let role = doc_state.role;
                    self.send_to_logic(LogicEvent::DocumentReady { document_id, role });
                }
            }

            WsServerMsg::SnapshotRequired { document_id } => {
                self.send_to_logic(LogicEvent::SnapshotNeeded { document_id });
            }

            WsServerMsg::SnapshotAvailable {
                document_id,
                snapshot_id,
            } => {
                tracing::info!("Sync: SnapshotAvailable {snapshot_id:?} for {document_id:?}");
                // Announced is not held: fetch it first (`SYNC-030`).
                self.fetch_announced_snapshot(document_id, snapshot_id);
            }

            WsServerMsg::KeyRotationRequired { document_id } => {
                // Remaining-member re-key path only. Being revoked is a separate,
                // targeted signal now (`DocumentRevoked`), so this message never
                // closes a document. Fetch our freshly wrapped key, unwrap it, and
                // rotate the local cache.
                let event_tx = self.event_tx.clone();
                let api_base = self.api_base.clone();
                let url_scheme = self.url_scheme;
                let exchange_key = self.exchange_key.clone();
                let doc_id = document_id;
                let auth = self.auth_context();

                if let Some(tx) = event_tx {
                    let Some(ek) = exchange_key else { return };
                    wasm_bindgen_futures::spawn_local(async move {
                        let members_url =
                            format!("{url_scheme}://{api_base}/api/documents/{doc_id}/members");
                        let members =
                            match http::get_json_auth::<http::MembersResponse>(&members_url, auth)
                                .await
                            {
                                Ok(resp) => resp.members,
                                Err(e) => {
                                    tracing::error!(
                                        "Sync: KeyRotationRequired fetch members failed: {e}"
                                    );
                                    return;
                                }
                            };

                        let our_exchange_pub = {
                            let pk = x25519_dalek::PublicKey::from(&ek);
                            hex::encode(pk.as_bytes())
                        };

                        let entry = match members
                            .iter()
                            .find(|m| m.exchange_pub_key == our_exchange_pub)
                        {
                            Some(e) => e,
                            None => {
                                tracing::warn!(
                                    "Sync: KeyRotationRequired: own member entry not found"
                                );
                                return;
                            }
                        };

                        let new_doc_key = match (|| -> Result<DocumentKey, String> {
                            let encrypted = base64::engine::general_purpose::URL_SAFE
                                .decode(&entry.doc_key_enc)
                                .map_err(|e| format!("base64 decode doc_key_enc: {e}"))?;
                            if encrypted.len() < 24 {
                                return Err("wrapped key too short".into());
                            }
                            let nonce: [u8; 24] = encrypted[..24]
                                .try_into()
                                .map_err(|_| "invalid nonce length")?;
                            let ciphertext = encrypted[24..].to_vec();
                            // The new key was wrapped by the rotator. Unwrap
                            // against THEIR exchange key, not our own (fallback to
                            // our own for older servers without the wrapper field).
                            let wrapper_hex = if entry.wrapper_exchange_pub_key.is_empty() {
                                &entry.exchange_pub_key
                            } else {
                                &entry.wrapper_exchange_pub_key
                            };
                            let peer_pub: [u8; 32] = hex::decode(wrapper_hex)
                                .map_err(|e| format!("hex decode pubkey: {e}"))?
                                .try_into()
                                .map_err(|_| "invalid pubkey length")?;
                            let wrapped = ambre_crypto::WrappedKey { nonce, ciphertext };
                            ambre_crypto::ecdh::unwrap_document_key(&ek, &peer_pub, &wrapped)
                                .map_err(|e| format!("ECDH unwrap: {e}"))
                        })() {
                            Ok(k) => k,
                            Err(e) => {
                                tracing::error!("Sync: KeyRotationRequired unwrap failed: {e}");
                                return;
                            }
                        };

                        let _ = tx.send(SyncEvent::KeyRotated {
                            document_id: doc_id,
                            new_doc_key,
                        });
                    });
                }
            }

            WsServerMsg::DocumentRevoked { document_id } => {
                // The server removed us from this document (targeted push on our
                // personal topic). Close it locally and tell the Logic thread we
                // were revoked so the UI can react.
                tracing::info!("Sync: DocumentRevoked {document_id:?}");
                self.documents.remove(&document_id);
                if let Some(uid) = self.user_id {
                    self.send_to_logic(LogicEvent::MemberRevoked {
                        document_id,
                        revoked_user_id: uid,
                    });
                }
            }

            WsServerMsg::DocumentShared {
                document_id,
                doc_type,
            } => {
                tracing::info!("Sync: DocumentShared {document_id:?} ({doc_type:?})");
                self.send_to_logic(LogicEvent::DocumentShared {
                    document_id,
                    doc_type,
                });
            }

            WsServerMsg::ChannelSubscribed { channel_id } => {
                self.send_to_logic(LogicEvent::ChannelConnectionStatus {
                    channel_id,
                    connected: true,
                });
            }

            WsServerMsg::ChannelUpdate {
                channel_id,
                seq: _,
                update,
            } => {
                let _ = (channel_id, update);
            }

            WsServerMsg::ChannelEphemeral {
                channel_id,
                ephemeral,
            } => {
                let _ = (channel_id, ephemeral);
            }

            WsServerMsg::Error { code, message } => {
                tracing::error!("Sync: server error {code}: {message}");
                self.send_to_logic(LogicEvent::ApiError {
                    id: None,
                    request: "websocket".into(),
                    code,
                    message,
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::SocketUrl;
    use crate::http::{SecretToken, TokenPair};
    use crate::{
        LogicEvent, LogicSink, SyncDocumentState, SyncEvent, SyncState, WS_CLOSE_SETUP_REQUIRED,
        WS_CLOSE_UNAUTHORIZED,
    };
    use ambre_bus::DocType;
    use ambre_core::DocumentId;
    use ambre_crypto::{DocumentKey, MasterSeed};

    /// A worker holding one open document, and the queue of what it tells Logic.
    fn with_open_document() -> (SyncState, ambre_bus::conduit::Receiver<LogicEvent>) {
        let (sink, rx) = LogicSink::channel();
        let mut state = SyncState::new(sink, String::new(), None);
        let id = DocumentId::new();
        state.documents.insert(
            id,
            SyncDocumentState::new(id, DocumentKey::generate(), DocType::Prose, false),
        );
        (state, rx)
    }

    fn closed(code: u16) -> SyncEvent {
        SyncEvent::WsDisconnected {
            code,
            reason: String::new(),
        }
    }

    /// A socket refused for a key setup still owed is neither renewed nor
    /// retried, by a timer armed before or a renewal landing after (`SYNC-133`).
    /// Once a session with keys is stored, it reopens.
    #[test]
    fn a_socket_refused_for_a_setup_owed_is_neither_renewed_nor_retried() {
        // Control: the authentication code does reopen, so a count of zero
        // below is the refusal's doing and not the harness's.
        let (mut renewed, _rx) = with_open_document();
        renewed.handle_sync_event(closed(WS_CLOSE_UNAUTHORIZED));
        assert_eq!(renewed.socket_opens, 1);

        let (mut state, _rx) = with_open_document();
        state.handle_sync_event(closed(WS_CLOSE_SETUP_REQUIRED));
        state.handle_sync_event(SyncEvent::ReconnectNow);
        state.handle_sync_event(SyncEvent::Reauthenticated { ok: true });
        assert_eq!(state.socket_opens, 0);

        let master_seed = MasterSeed::from_array([7u8; 32]);
        state.handle_sync_event(SyncEvent::AccountCreated {
            id: ambre_bus::RequestId::next(),
            recovery_words: String::new(),
            signing_key: master_seed.derive_signing_key(),
            exchange_key: master_seed.derive_exchange_key(),
            master_seed,
            tokens: TokenPair {
                access_token: SecretToken::from("access".to_owned()),
                refresh_token: SecretToken::from("refresh".to_owned()),
                access_expires_in: 60,
            },
        });
        assert_eq!(state.socket_opens, 1);
    }

    /// A socket refused for a key setup still owed reaches Logic as a setup to
    /// complete, and as nothing else (`SYNC-134`).
    #[test]
    fn a_socket_refused_for_a_setup_owed_reaches_logic_as_a_setup() {
        let (mut state, rx) = with_open_document();
        state.handle_sync_event(closed(WS_CLOSE_SETUP_REQUIRED));
        let told: Vec<LogicEvent> = rx.try_iter().collect();
        assert!(told.iter().any(|e| matches!(e, LogicEvent::SetupRequired)));
        assert!(!told.iter().any(|e| matches!(
            e,
            LogicEvent::ApiError { .. } | LogicEvent::AuthError { .. }
        )));
    }

    /// A frame of a newer wire version reaches Logic once as an update to
    /// install, never as an error (`SYNC-137`). No timer or renewal reopens
    /// the socket.
    #[test]
    fn a_newer_wire_version_reaches_logic_as_an_update_and_closes_for_good() {
        let (mut state, rx) = with_open_document();
        state.handle_sync_event(SyncEvent::WireOutdated);
        state.handle_sync_event(SyncEvent::WireOutdated);
        state.handle_sync_event(closed(WS_CLOSE_UNAUTHORIZED));
        state.handle_sync_event(SyncEvent::ReconnectNow);
        state.handle_sync_event(SyncEvent::Reauthenticated { ok: true });
        assert_eq!(state.socket_opens, 0);
        let told: Vec<LogicEvent> = rx.try_iter().collect();
        let updates = told
            .iter()
            .filter(|e| matches!(e, LogicEvent::UpdateRequired))
            .count();
        assert_eq!(updates, 1);
        assert!(!told.iter().any(|e| matches!(
            e,
            LogicEvent::ApiError { .. } | LogicEvent::AuthError { .. }
        )));
    }

    /// A logged socket address carries no token (`SYNC-061`), whichever way
    /// it is formatted.
    #[test]
    fn a_logged_socket_url_carries_no_token() {
        let url = SocketUrl::new("wss", "relay.example", "s3cr3t-token");
        assert!(url.expose().ends_with("?token=s3cr3t-token"));
        for shown in [format!("{url}"), format!("{url:?}")] {
            assert!(!shown.contains("s3cr3t"), "{shown}");
            assert!(shown.starts_with("wss://relay.example/api/ws"), "{shown}");
        }
    }

    /// A token pair logged by `Debug` prints neither token (`SYNC-061`).
    #[test]
    fn a_logged_token_pair_carries_no_token() {
        let pair = TokenPair {
            access_token: SecretToken::from("acc-s3cr3t".to_owned()),
            refresh_token: SecretToken::from("ref-s3cr3t".to_owned()),
            access_expires_in: 60,
        };
        let shown = format!("{pair:?}");
        assert!(!shown.contains("s3cr3t"), "{shown}");
    }

    /// Both tokens wipe their bytes when dropped (`SYNC-100`): the bound fails
    /// the build the day either stops doing so.
    #[test]
    fn session_tokens_wipe_on_drop() {
        fn wipes_on_drop<T: zeroize::ZeroizeOnDrop>() {}
        wipes_on_drop::<SecretToken>();
    }
}
