use super::*;
use crate::rekey::Refusal;
use ambre_protocol::ws::AccessToken;

/// The socket address. It carries no credential: the token travels in the
/// socket's first frame, so no log line, proxy record or URL holds it
/// (`SYNC-061`, `ambre-spec-session.md`, `SRV-031`).
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub(crate) fn socket_address(scheme: &str, api_base: &str) -> String {
    format!("{scheme}://{api_base}/api/ws")
}

impl SyncState {
    /// Sends the access token as the socket's first frame. The relay reads
    /// nothing else until this frame opens the session (`SRV-031`).
    pub(crate) fn authenticate_socket(&mut self) {
        let Some(token) = self
            .tokens
            .borrow()
            .as_ref()
            .map(|t| AccessToken::new(t.access_token.expose().to_owned()))
        else {
            tracing::warn!("Sync: socket opened before the access token is ready");
            return;
        };
        self.ws_send(&WsClientMsg::Authenticate { token });
    }

    // -- WebSocket lifecycle ---------------------------------------------------

    /// Opens the session-level multiplexed WebSocket. Called after Unlock or
    /// SetupAccount. If the socket is already present (reconnection), it is
    /// replaced. All open documents are automatically re-subscribed.
    #[cfg(target_arch = "wasm32")]
    pub(crate) fn open_websocket(&mut self) {
        if self.update_owed || self.inbound_held {
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
        if self.tokens.borrow().is_none() {
            tracing::warn!("Sync: WebSocket open requested before access token is ready");
            return;
        }
        let url = socket_address(ws_scheme, &self.api_base);
        match WebSocket::new(&url) {
            Ok(ws) => {
                ws.set_binary_type(web_sys::BinaryType::Arraybuffer);
                self.wire_ws_callbacks(&ws);
                self.ws = Some(ws);
                // The token and the subscriptions are sent from the `onopen`
                // handler, NOT here. The socket is still CONNECTING at this point
                // and `ws_send` would be dropped. The server would then hold a
                // socket that never authenticates, and close it.
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
        if self.update_owed || self.inbound_held {
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
        // An evicted document still counts what it missed (`RESID-014`).
        for (document_id, seen) in self.light_cursors() {
            self.ws_send(&WsClientMsg::Subscribe { document_id });
            self.ws_send(&WsClientMsg::RequestMissing {
                document_id,
                after_seq: seen,
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

    /// Arms the reconnect backoff after a failed attempt, so a rejected
    /// upgrade does not trigger a tight reconnect storm.
    pub(crate) fn schedule_reconnect(&mut self) {
        let next = self.lifecycle.back_off((self.clock)(), Self::jitter());
        self.follow(next);
    }

    /// Carries out what the socket lifecycle decided.
    fn follow(&mut self, next: lifecycle::Next) {
        match next {
            lifecycle::Next::Idle => {}
            lifecycle::Next::Open => {
                // Only when still wanted and not already reopened. A timer
                // armed before a setup refusal must not retry it.
                if self.ws.is_none() && !self.documents.is_empty() && !self.setup_owed {
                    self.open_websocket();
                }
            }
            lifecycle::Next::Renew => self.reauthenticate(),
            lifecycle::Next::Wait { delay_ms } => self.arm_reconnect_timer(delay_ms),
            lifecycle::Next::SetupOwed => {
                // The socket reopens once a session with keys is stored
                // (`SYNC-133`). Meanwhile the client is told a setup is owed
                // (`SYNC-134`).
                self.setup_owed = true;
                self.send_to_logic(LogicEvent::SetupRequired);
            }
        }
    }

    /// The fraction of a reconnect delay added to it, so clients cut off
    /// together do not all return together.
    #[cfg(target_arch = "wasm32")]
    fn jitter() -> f64 {
        js_sys::Math::random()
    }

    /// Native: no jitter, so a host test reads the exact delays.
    #[cfg(not(target_arch = "wasm32"))]
    fn jitter() -> f64 {
        0.0
    }

    /// Asks the worker's timers for a [`SyncEvent::ReconnectNow`] in
    /// `delay_ms`. The shell only schedules: the lifecycle decides what the
    /// timer means when it fires.
    #[cfg(target_arch = "wasm32")]
    fn arm_reconnect_timer(&mut self, delay_ms: u64) {
        use wasm_bindgen::JsCast;

        let Some(tx) = self.event_tx.clone() else {
            return;
        };
        let cb = Closure::wrap(Box::new(move || {
            let _ = tx.send(SyncEvent::ReconnectNow);
        }) as Box<dyn FnMut()>);
        if let Ok(scope) = js_sys::global().dyn_into::<web_sys::DedicatedWorkerGlobalScope>() {
            let _ = scope.set_timeout_with_callback_and_timeout_and_arguments_0(
                cb.as_ref().unchecked_ref(),
                i32::try_from(delay_ms).unwrap_or(i32::MAX),
            );
            tracing::warn!("Sync: reconnect scheduled in {delay_ms}ms");
        }
        // Keep the closure alive until it fires (replaces any previous pending one).
        self._reconnect_timer = Some(cb);
    }

    /// Native: no timer. A host test advances the clock and sends
    /// [`SyncEvent::ReconnectNow`] itself.
    #[cfg(not(target_arch = "wasm32"))]
    fn arm_reconnect_timer(&mut self, _delay_ms: u64) {}

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

    /// Closes the socket with its handlers detached, so its close schedules
    /// no reconnect, then reports every document disconnected.
    fn close_quietly(&mut self, reason: &str) {
        if let Some(ws) = self.ws.take() {
            ws.set_onmessage(None);
            ws.set_onclose(None);
            ws.set_onerror(None);
            let _ = ws.close();
        }
        self.handle_ws_disconnected(0, reason.to_owned());
    }

    /// Stops reading the relay while the path back to the Logic thread is
    /// full (`SYNC-059`). A browser socket cannot pause, so it is closed. The
    /// relay keeps every delta, and the reopen asks for what was missed.
    pub(crate) fn hold_inbound(&mut self) {
        if self.inbound_held {
            return;
        }
        self.inbound_held = true;
        self.close_quietly("inbound held");
    }

    /// Reopens the socket once the path back has room. The open resubscribes
    /// every document from its last sequence, then sends what was kept.
    pub(crate) fn release_inbound(&mut self) {
        if !self.inbound_held {
            return;
        }
        self.inbound_held = false;
        if self.ws.is_none() && !self.documents.is_empty() && !self.setup_owed {
            self.lifecycle.reset();
            self.open_websocket();
        }
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

    /// Hands a close with `code`, or an error read as code 0, to the
    /// lifecycle, then carries out what it decided.
    fn socket_closed(&mut self, code: u16) {
        let wanted = !self.documents.is_empty();
        let next = self
            .lifecycle
            .closed(code, wanted, (self.clock)(), Self::jitter());
        self.follow(next);
    }

    /// Processes one [`SyncEvent`] received from the internal event queue.
    /// Dispatches WebSocket frames to the existing [`handle_ws_message`]
    /// pipeline, applies state mutations from HTTP result events, and handles
    /// disconnection / errors.
    #[allow(dead_code)]
    pub(crate) fn handle_sync_event(&mut self, event: SyncEvent) {
        match event {
            SyncEvent::WsMessage(msg) => self.handle_ws_message(msg),
            SyncEvent::WsOpened => {
                self.authenticate_socket();
                self.resubscribe_all();
                self.flush_held_outbox();
            }
            SyncEvent::WireOutdated => {
                if self.update_owed {
                    return;
                }
                // Every later frame would fail the same way, and no retry
                // cures it. Close for good, and say once that an update is
                // owed (`SYNC-137`).
                self.update_owed = true;
                self.close_quietly("wire outdated");
                self.send_to_logic(LogicEvent::UpdateRequired);
            }
            SyncEvent::WsDisconnected { code, reason } => {
                self.handle_ws_disconnected(code, reason);
                self.socket_closed(code);
            }
            SyncEvent::Reauthenticated { ok } => {
                if ok {
                    // New access token in the shared store: reopen promptly.
                    let next = self.lifecycle.renewed();
                    self.follow(next);
                } else if let Some(username) = self.dev_username.clone() {
                    // Refresh token also dead (for example server restarted with a fresh
                    // ephemeral key and empty Redis): re-login with the same
                    // deterministic dev identity. SessionReady reopens the socket.
                    tracing::warn!(
                        "Sync: refresh failed, re-running dev login for silent recovery"
                    );
                    self.handle_dev_login(RequestId::UNSOLICITED, username);
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
                self.socket_closed(0);
            }
            SyncEvent::ReconnectNow => {
                self._reconnect_timer = None;
                let next = self.lifecycle.poll((self.clock)());
                self.follow(next);
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
                held_snapshot,
                doc_type,
                server_created,
                signing_keys,
                role,
                rotation_owed,
                links,
            } => {
                let doc_state = self
                    .documents
                    .entry(document_id)
                    .insert_entry(SyncDocumentState::new(
                        document_id,
                        doc_key,
                        doc_type,
                        server_created,
                    ))
                    .into_mut();
                doc_state.chain = crate::chain::Chain::new(document_id, links);
                doc_state.set_signing_keys(signing_keys);
                doc_state.role = role;
                // A writer pays an owed rotation before its first frame; a
                // reader sends none, so it owes nothing (`SYNC-128`).
                if rotation_owed && role.can_write_document() {
                    doc_state.rekey.found_debt();
                }
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
                            covered_seq: covered_seq.0,
                        });
                    }
                    Some(Err(e)) => {
                        self.send_to_logic(LogicEvent::SyncWarning {
                            document_id,
                            code: SyncWarningCode::DecryptFailed,
                            message: format!("the latest snapshot does not open: {e}"),
                        });
                    }
                    // No snapshot, or a reopen that resumes from the local
                    // copy: the replay starts after its horizon, 0 when none
                    // (`RESID-010`).
                    None => {
                        doc_state.current_snapshot_id = snapshot_id;
                        doc_state.known_snapshots.insert(snapshot_id);
                        doc_state.last_seq = covered_seq;
                    }
                }
                // A snapshot another user confirmed alone waits for the replay,
                // then is checked (`AUTHOR-022`). The deltas parented on it are
                // signed one by one, so they need not wait for it.
                if let Some(held) = held_snapshot
                    && let Some(doc_state) = self.documents.get_mut(&document_id)
                {
                    doc_state.known_snapshots.insert(held.snapshot_id);
                    doc_state.held_snapshot = Some(held);
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
                self.send_to_logic(LogicEvent::DocumentOpenFailed {
                    document_id,
                    message,
                });
            }
            SyncEvent::ChannelListed {
                channel_id,
                self_join,
                result,
            } => {
                self.channel_listed(channel_id, self_join, result);
            }
            SyncEvent::ActivityFetched { result } => self.activity_fetched(result),
            SyncEvent::ChannelCreated { channel_id, result } => {
                self.channel_answered(
                    channel_id,
                    result.map(|key| LogicEvent::ChannelCreated { channel_id, key }),
                );
            }
            SyncEvent::ChannelJoined { channel_id, result } => {
                self.channel_answered(
                    channel_id,
                    result.map(|()| LogicEvent::ChannelJoined { channel_id }),
                );
            }
            SyncEvent::ChannelDeleted { channel_id, result } => {
                self.channel_answered(
                    channel_id,
                    result.map(|()| LogicEvent::ChannelDeleted { channel_id }),
                );
            }
            SyncEvent::ChannelLeft { channel_id, result } => {
                self.channel_answered(
                    channel_id,
                    result.map(|()| LogicEvent::ChannelLeft { channel_id }),
                );
            }
            SyncEvent::DirectMessageOpened { requested, result } => {
                self.channel_answered(
                    requested,
                    result.map(|channel_id| LogicEvent::DirectMessageOpened {
                        requested,
                        channel_id,
                    }),
                );
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
                for body in pending {
                    self.send_body(document_id, body);
                }
                self.flush_pending_uploads(document_id, None);
            }
            SyncEvent::DocumentCreateFailed {
                document_id,
                message,
            } => {
                if let Some(doc_state) = self.documents.get_mut(&document_id) {
                    doc_state.creating = false;
                }
                tracing::error!("Sync: lazy create failed for {document_id:?}: {message}");
                self.flush_pending_uploads(document_id, Some(&message));
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
                link,
            } => {
                self.install_key(document_id, new_doc_key, [link]);
                self.reply(id, RestEvent::RevocationComplete { document_id });
            }
            SyncEvent::DocumentLeft { id, document_id } => {
                // Gone from the relay's list: nothing of this document is ours
                // to keep open (`SYNC-127`).
                self.documents.remove(&document_id);
                self.reply(id, RestEvent::DocumentLeft { document_id });
            }
            SyncEvent::DebtFound { document_id } => {
                if let Some(doc_state) = self.documents.get_mut(&document_id)
                    && doc_state.role.can_write_document()
                {
                    doc_state.rekey.found_debt();
                }
            }
            SyncEvent::DebtSettled {
                document_id,
                new_doc_key,
                link,
            } => self.install_key(document_id, new_doc_key, [link]),
            SyncEvent::DebtGone { document_id } => self.fetch_rotated_key(document_id),
            SyncEvent::DebtUnsettled {
                document_id,
                message,
            } => {
                tracing::warn!("Sync: the owed rotation of {document_id:?} failed: {message}");
                if let Some(doc_state) = self.documents.get_mut(&document_id) {
                    doc_state.rekey.payment_failed();
                }
            }
            SyncEvent::DocumentMembersReady {
                id,
                asked,
                document_id,
                members,
                signing_keys,
            } => {
                // Keep presence resolvable as membership evolves. A document that
                // is not open locally has no session to refresh -- the next open
                // rebuilds the directory from scratch. An answer older than the
                // directory held changes nothing (`SYNC-037`).
                if let Some(doc_state) = self.documents.get_mut(&document_id)
                    && asked > doc_state.directory_held
                {
                    doc_state.directory_held = asked;
                    doc_state.set_signing_keys(signing_keys);
                }
                if let Some(id) = id {
                    self.reply(
                        id,
                        RestEvent::DocumentMembers {
                            document_id,
                            members,
                        },
                    );
                }
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
                links,
            } => {
                // Swap the local key cache. This is the remaining-member path: a
                // rotation is not a revocation. Nothing is reported as a member
                // loss here; a revoked user is told via `DocumentRevoked`.
                self.install_key(document_id, new_doc_key, links);
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
                covered_seq,
                blob,
            } => self.adopt_fetched_snapshot(document_id, snapshot_id, covered_seq, blob),
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
        // Any wire message from the server proves the connection is authenticated and
        // live, so the reconnect backoff resets. A bare `onopen` is NOT enough:
        // an auth-rejected upgrade still returns 101 (then the server drops it),
        // firing `onopen` without ever sending a message. Resetting on `onopen`
        // there would spin a tight reconnect loop (storm). A rejected connection
        // never reaches this point, so its backoff keeps growing as intended.
        self.lifecycle.frame();
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
                // sample is dropped rather than shown under a guessed identity.
                // Presence is best-effort; a dropped sample is corrected by the
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
                let mut deltas: Vec<SignedDelta> = Vec::new();
                for seq_update in updates {
                    match self.ingest_update(seq_update.seq, seq_update.update) {
                        Some(Ok(delta)) => deltas.push(delta),
                        // The `Err` arm only warns; the builder is never called.
                        Some(Err(e)) => {
                            self.deliver_remote(document_id, Some(Err(e)), |delta| {
                                LogicEvent::RemoteUpdateBatch {
                                    document_id,
                                    deltas: vec![delta],
                                }
                            });
                        }
                        None => {}
                    }
                }
                if !deltas.is_empty() {
                    self.send_to_logic(LogicEvent::RemoteUpdateBatch {
                        document_id,
                        deltas,
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
                    // Every delta the relay kept is replayed: the snapshot the
                    // open held back can be checked now (`AUTHOR-022`). Its
                    // verdict says the document is ready instead.
                    if !self.check_held_snapshot(document_id) {
                        self.send_to_logic(LogicEvent::DocumentReady { document_id, role });
                    }
                }
            }

            WsServerMsg::SnapshotRequired { document_id } => {
                self.send_to_logic(LogicEvent::SnapshotNeeded { document_id });
            }

            WsServerMsg::SnapshotAvailable {
                document_id,
                snapshot_id,
                covered_seq,
            } => {
                tracing::info!("Sync: SnapshotAvailable {snapshot_id:?} for {document_id:?}");
                // Announced is not held: fetch it first (`SYNC-030`).
                self.fetch_announced_snapshot(document_id, snapshot_id, covered_seq);
            }

            WsServerMsg::KeyRotationRequired { document_id } => {
                // Remaining-member re-key path only. Being revoked is a separate,
                // targeted signal now (`DocumentRevoked`), so this message never
                // closes a document.
                self.fetch_rotated_key(document_id);
            }

            WsServerMsg::KeyRefused {
                document_id,
                key_id,
                clock,
            } => self.handle_key_refused(document_id, key_id, clock.0),

            WsServerMsg::ClockRefused {
                document_id,
                clock,
                expected,
                signature,
            } => {
                tracing::info!(
                    "Sync: clock {} refused doc={document_id}, expected {}",
                    clock.0,
                    expected.0
                );
                self.handle_clock_refused(document_id, expected.0, signature);
            }

            WsServerMsg::MembersChanged { document_id } => {
                // Only an open document holds a directory and a view; the next
                // open reads the rows from scratch. The directory is read
                // again here, whatever application holds the document, so a
                // newcomer's samples resolve (`SYNC-037`). A view that shows
                // the members asks for them on its own.
                if self.documents.contains_key(&document_id) {
                    self.handle_list_document_members(None, document_id);
                    self.send_to_logic(LogicEvent::MembersChanged { document_id });
                }
            }

            WsServerMsg::DocumentRevoked { document_id } => {
                // The server removed us from this document (targeted push on our
                // personal topic). Close it locally and tell the Logic thread we
                // were revoked so the UI can react.
                tracing::info!("Sync: DocumentRevoked {document_id:?}");
                self.documents.remove(&document_id);
                self.light.remove(&document_id);
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

impl SyncState {
    /// Fetches this client's newly wrapped key, unwraps it and installs it.
    /// A members answer that says a rotation is owed carries no new key: a
    /// member left, and the debt is recorded instead (`SYNC-128`).
    pub(crate) fn fetch_rotated_key(&self, document_id: DocumentId) {
        let (Some(tx), Some(ek)) = (self.event_tx.clone(), self.exchange_key.clone()) else {
            return;
        };
        let api_base = self.api_base.clone();
        let url_scheme = self.url_scheme;
        let auth = self.auth_context();
        let self_join = self.self_join_rule(document_id);
        wasm_bindgen_futures::spawn_local(async move {
            let members_url =
                format!("{url_scheme}://{api_base}/api/documents/{document_id}/members");
            let resp = match http::get_json_auth::<http::MembersResponse>(
                &members_url,
                auth.clone(),
            )
            .await
            {
                Ok(resp) => resp,
                Err(e) => {
                    tracing::error!("Sync: KeyRotationRequired fetch members failed: {e}");
                    return;
                }
            };
            if resp.rotation_owed {
                let _ = tx.send(SyncEvent::DebtFound { document_id });
                return;
            }

            // The new key was wrapped by the rotator, and the unwrap runs
            // against their exchange key, not our own.
            let (new_doc_key, anchor) = match Self::unwrap_anchored(&ek, &resp.members) {
                Ok(unwrapped) => unwrapped,
                Err(e) => {
                    tracing::error!("Sync: KeyRotationRequired unwrap failed: {e}");
                    return;
                }
            };

            // The link the rotator left reaches back from the new key
            // (`SYNC-117`). It counts only when a writer the log names
            // signed it (`SYNC-121`).
            let links = http::fetch_key_links(url_scheme, &api_base, document_id, auth).await;
            let log = crate::sharing::walk_served_under(document_id, anchor, &resp.log, self_join);
            let links = crate::chain::signed_by_writers(links, log.signers());
            let _ = tx.send(SyncEvent::KeyRotated {
                document_id,
                new_doc_key,
                links,
            });
        });
    }

    /// The relay refused our frame at `clock` because the document owes a
    /// rotation (`SRV-123`). The clock is still free. Under the current key the
    /// debt is paid first; under an older one the frame is sent again now
    /// (`SYNC-128`, `SYNC-129`).
    pub(crate) fn handle_key_refused(
        &mut self,
        document_id: DocumentId,
        key_id: KeyId,
        clock: u64,
    ) {
        let Some(doc_state) = self.documents.get_mut(&document_id) else {
            return;
        };
        let current = doc_state.doc_key.id();
        match doc_state.rekey.refused(key_id, clock, current) {
            Refusal::Stale | Refusal::Wait => {}
            Refusal::Settle => self.start_settle(document_id),
            Refusal::Resend => {
                let refused = doc_state.rekey.take_refused(current);
                self.resend_refused(document_id, refused, Vec::new());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::socket_address;
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

    /// `SYNC-064`, `SYNC-098`: the worker walks its lifecycle on its own
    /// clock. A dropped socket waits out the backoff before it reopens, and a
    /// timer that fires early opens nothing. A refusal before any frame
    /// doubles the wait, and a frame resets it.
    #[test]
    fn a_dropped_socket_reopens_on_the_clock_and_a_frame_resets_the_wait() {
        let (mut state, _rx) = with_open_document();
        let now = std::rc::Rc::new(std::cell::Cell::new(0_u64));
        let clock = now.clone();
        state.clock = std::rc::Rc::new(move || clock.get());
        let fire_at = |state: &mut SyncState, at: u64| {
            now.set(at);
            state.handle_sync_event(SyncEvent::ReconnectNow);
            state.socket_opens
        };

        state.handle_sync_event(closed(1006));
        assert_eq!(fire_at(&mut state, 249), 0, "early: nothing reopens");
        assert_eq!(fire_at(&mut state, 250), 1);

        // Opened, then refused before any frame: the wait doubles.
        state.handle_sync_event(SyncEvent::WsOpened);
        state.handle_sync_event(closed(1006));
        assert_eq!(fire_at(&mut state, 749), 1);
        assert_eq!(fire_at(&mut state, 750), 2);

        // A frame proves the session live, so the next wait is the first.
        let document_id = *state.documents.keys().next().expect("one document");
        state.handle_ws_message(ambre_protocol::ws::WsServerMsg::Subscribed { document_id });
        state.handle_sync_event(closed(1006));
        assert_eq!(fire_at(&mut state, 999), 2);
        assert_eq!(fire_at(&mut state, 1000), 3);
        assert_eq!(fire_at(&mut state, 5000), 3, "one open per deadline");
    }

    /// `SYNC-066`: a sign-out closes the socket and clears every key. A drop
    /// reported after it reopens nothing, where the same drop before it does.
    #[test]
    fn a_sign_out_closes_the_socket_and_clears_every_key() {
        // A drop, then the reconnect timer once the backoff has passed.
        let reopens = |state: &mut SyncState| {
            let now = std::rc::Rc::new(std::cell::Cell::new(0_u64));
            let clock = now.clone();
            state.clock = std::rc::Rc::new(move || clock.get());
            state.handle_sync_event(closed(1006));
            now.set(60_000);
            state.handle_sync_event(SyncEvent::ReconnectNow);
            state.socket_opens
        };
        let (mut control, _rx) = with_open_document();
        assert_eq!(reopens(&mut control), 1, "control: a drop reopens");

        let (mut state, rx) = with_open_document();
        let master_seed = MasterSeed::from_array([7u8; 32]);
        state.signing_key = Some(master_seed.derive_signing_key());
        state.exchange_key = Some(master_seed.derive_exchange_key());
        state.master_seed = Some(master_seed);
        let id = *state.documents.keys().next().expect("one document");
        state
            .session_nonces
            .insert(id, ambre_core::SessionNonce([1; 16]));
        *state.tokens.borrow_mut() = Some(TokenPair {
            access_token: SecretToken::from("access".to_owned()),
            refresh_token: SecretToken::from("refresh".to_owned()),
            access_expires_in: 60,
        });
        let (tx, wire) = ambre_bus::conduit::unbounded();
        state.set_mock_ws(tx);

        state.handle_logout();
        assert!(wire.is_disconnected(), "the socket is closed");
        assert!(state.master_seed.is_none());
        assert!(state.signing_key.is_none());
        assert!(state.exchange_key.is_none());
        assert!(state.tokens.borrow().is_none());
        assert!(state.documents.is_empty());
        assert!(state.session_nonces.is_empty());

        assert_eq!(reopens(&mut state), 0, "nothing reopens");
        assert!(rx.try_iter().next().is_none(), "nothing reaches Logic");
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

    /// A hold stops reading the relay until the release, and nothing else
    /// reopens the socket meanwhile (`SYNC-059`). The reopen asks for what was
    /// missed, then sends the edit sealed during the hold.
    #[test]
    fn a_held_inbound_reopens_only_on_release_and_catches_up() {
        use ambre_bus::SyncCommand;
        use ambre_protocol::messages::Seq;
        use ambre_protocol::ws::{WsClientMsg, decode_client_msg};

        let (sink, _rx) = LogicSink::channel();
        let mut state = SyncState::new(sink, String::new(), None);
        state.signing_key = Some(MasterSeed::generate().derive_signing_key());
        let id = DocumentId::new();
        let mut doc = SyncDocumentState::new(id, DocumentKey::generate(), DocType::Prose, true);
        doc.last_seq = Seq(7);
        state.documents.insert(id, doc);
        state
            .session_nonces
            .insert(id, ambre_core::SessionNonce([1; 16]));

        state.handle_command(SyncCommand::HoldInbound);
        state.handle_sync_event(closed(1006));
        state.handle_sync_event(SyncEvent::ReconnectNow);
        state.handle_sync_event(SyncEvent::Reauthenticated { ok: true });
        assert_eq!(state.socket_opens, 0, "held: nothing reopens");
        state.handle_command(SyncCommand::SendUpdate {
            document_id: id,
            payload: b"typed during the hold".to_vec(),
            summary: None,
        });
        assert_eq!(state.held_outbox.borrow().len(), 1, "the edit is kept");

        let (tx, wire) = ambre_bus::conduit::unbounded();
        state.set_mock_ws(tx);
        state.handle_command(SyncCommand::ReleaseInbound);
        assert_eq!(state.socket_opens, 1);
        state.handle_sync_event(SyncEvent::WsOpened);
        let sent: Vec<WsClientMsg> = wire
            .try_iter()
            .map(|bytes| decode_client_msg(&bytes).expect("decode"))
            .collect();
        assert!(matches!(sent[0], WsClientMsg::Subscribe { document_id } if document_id == id));
        assert!(matches!(
            sent[1],
            WsClientMsg::RequestMissing {
                after_seq: Seq(7),
                ..
            }
        ));
        assert!(matches!(sent[2], WsClientMsg::Update(_)));
        assert_eq!(sent.len(), 3);
    }

    /// A reopened socket subscribes again to every document the relay holds
    /// (`SYNC-033`), and asks each for what it missed past its own cursor
    /// (`SYNC-086`). A document the relay never created gets neither.
    #[test]
    fn a_reconnect_resubscribes_and_pulls_every_relay_document() {
        use ambre_protocol::messages::Seq;
        use ambre_protocol::ws::{WsClientMsg, decode_client_msg};

        let (sink, _rx) = LogicSink::channel();
        let mut state = SyncState::new(sink, String::new(), None);
        let mut cursors = std::collections::HashMap::new();
        for last in [7, 12] {
            let id = DocumentId::new();
            let mut doc = SyncDocumentState::new(id, DocumentKey::generate(), DocType::Trame, true);
            doc.last_seq = Seq(last);
            state.documents.insert(id, doc);
            cursors.insert(id, Seq(last));
        }
        let local = DocumentId::new();
        state.documents.insert(
            local,
            SyncDocumentState::new(local, DocumentKey::generate(), DocType::Trame, false),
        );

        let (tx, wire) = ambre_bus::conduit::unbounded();
        state.set_mock_ws(tx);
        state.handle_sync_event(SyncEvent::WsOpened);
        let sent: Vec<WsClientMsg> = wire
            .try_iter()
            .filter_map(|bytes| decode_client_msg(&bytes).ok())
            .collect();

        let subscribed: std::collections::HashSet<DocumentId> = sent
            .iter()
            .filter_map(|m| match m {
                WsClientMsg::Subscribe { document_id } => Some(*document_id),
                _ => None,
            })
            .collect();
        let pulled: std::collections::HashMap<DocumentId, Seq> = sent
            .iter()
            .filter_map(|m| match m {
                WsClientMsg::RequestMissing {
                    document_id,
                    after_seq,
                } => Some((*document_id, *after_seq)),
                _ => None,
            })
            .collect();
        assert_eq!(subscribed, cursors.keys().copied().collect());
        assert_eq!(pulled, cursors);
        assert_eq!(
            sent.len(),
            4,
            "one subscription and one pull each: {sent:?}"
        );
    }

    /// The socket address carries no query, so no log line, proxy record or
    /// URL holds a token (`SYNC-061`, `SRV-031`).
    #[test]
    fn the_socket_address_carries_no_token() {
        assert_eq!(
            socket_address("wss", "relay.example"),
            "wss://relay.example/api/ws"
        );
    }

    /// An opened socket sends its token as its first frame, before any
    /// subscription (`SRV-031`). The frame prints no byte of the token.
    #[test]
    fn an_opened_socket_authenticates_before_it_subscribes() {
        use ambre_protocol::ws::{WsClientMsg, decode_client_msg};

        let (mut state, _rx) = with_open_document();
        let id = *state.documents.keys().next().unwrap();
        state.documents.get_mut(&id).unwrap().server_created = true;
        *state.tokens.borrow_mut() = Some(TokenPair {
            access_token: SecretToken::from("acc-s3cr3t".to_owned()),
            refresh_token: SecretToken::from("ref-s3cr3t".to_owned()),
            access_expires_in: 60,
        });
        let (tx, wire) = ambre_bus::conduit::unbounded();
        state.set_mock_ws(tx);
        state.handle_sync_event(SyncEvent::WsOpened);
        let sent: Vec<WsClientMsg> = wire
            .try_iter()
            .map(|bytes| decode_client_msg(&bytes).expect("decode"))
            .collect();
        let WsClientMsg::Authenticate { token } = &sent[0] else {
            panic!("the first frame is {:?}", sent[0]);
        };
        assert_eq!(token.expose(), "acc-s3cr3t");
        assert!(!format!("{:?}", sent[0]).contains("s3cr3t"));
        assert!(matches!(sent[1], WsClientMsg::Subscribe { document_id } if document_id == id));
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
