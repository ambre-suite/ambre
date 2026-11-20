use super::*;
use crate::rekey::Refusal;
use ambre_core::{Bounded, RefusalCode};
use ambre_core::{Ms, TimeSpan};

/// The delay before Sync asks again for the first stopped replay of a
/// document. Each later stop doubles it (`ambre-spec-budgets.md` section 8).
const REPLAY_RETRY_BASE: TimeSpan<Ms> = TimeSpan::<Ms>::new(1_000);

/// The stops Sync asks again after. One more reaches Logic as an error
/// (`ambre-spec-budgets.md` section 8, `SYNC-154`).
const REPLAY_RETRY_LIMIT: Count<ReplayStopCount> = Count::new(5);

/// The stops of one replay counted against its budget, from one.
pub(crate) enum ReplayStopCount {}

impl ambre_core::NarrowCount for ReplayStopCount {
    const NAME: ambre_core::StaticKey<ambre_core::DebugNames> =
        ambre_core::StaticKey::new("ReplayStops");
}

/// The delay before asking again after the `stop`-th stop of one replay,
/// counted from one. Past the budget there is none (`SYNC-154`).
fn replay_retry_delay(stop: Count<ReplayStopCount>) -> Option<TimeSpan<Ms>> {
    let doublings = stop
        .get()
        .checked_sub(1)
        .filter(|doublings| *doublings < REPLAY_RETRY_LIMIT.get())?;
    2_u64
        .checked_pow(doublings)
        .and_then(|factor| REPLAY_RETRY_BASE.get().checked_mul(factor))
        .map(TimeSpan::<Ms>::new)
}

/// The shortest time between two replay asks of one document on one socket.
/// The relay answers nothing to a second ask inside its window (`SRV-086`).
/// So the spacing outlasts that window, with a margin for the frames in
/// flight (`ambre-spec-budgets.md` section 8).
const REPLAY_ASK_SPACING: TimeSpan<Ms> = TimeSpan::<Ms>::new(1_250);

// The relay's window is a protocol constant, and a relay that raised it past
// the spacing would drop the second ask with no answer (`SYNC-191`).
const _: () = assert!(
    REPLAY_ASK_SPACING.get() > ambre_protocol::ws::REPLAY_MIN_INTERVAL.get(),
    "REPLAY_ASK_SPACING must outlast the relay's REPLAY_MIN_INTERVAL"
);

/// Whether a replay ask leaves now, or waits for the relay's window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AskSlot {
    Now,
    Held,
}

/// The last replay ask of one document on the open socket.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ReplayAsk {
    /// When the socket sent it.
    at: ambre_core::Instant<ambre_core::UnixMs>,
    /// The cursor of an ask that came inside the window, sent once the
    /// window passes. Two such asks keep the lower cursor.
    held: Option<Seq<Relay>>,
}

/// The stops of one document's replay since its budget last started.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ReplayStops {
    /// The stops counted against the budget, from one.
    count: Count<ReplayStopCount>,
    /// The last delta the latest stop named.
    after: Seq<Relay>,
    /// The ticket of the timer armed for the latest stop, if any.
    ticket: ambre_core::Seq<crate::generation::ReplayTicket>,
}

/// Whether a socket frame of `len` bytes is read at all, checked before the
/// frame is copied into the worker's memory (`SYNC-277`, audit `W206`). The
/// ceiling is a row of `ambre-spec-budgets.md` section 1. A replay page of
/// the relay is one frame, so the ceiling sits far above one delta.
#[cfg_attr(
    not(any(test, target_arch = "wasm32")),
    expect(dead_code, reason = "only the wasm socket callback reads a frame")
)]
pub(crate) fn frame_fits(len: ambre_core::Count<ambre_core::Octets>) -> bool {
    const FRAME_CEILING: ambre_core::Count<ambre_core::Octets> =
        ambre_core::Count::new(128 * 1024 * 1024);
    len <= FRAME_CEILING
}

impl SyncState {
    /// Sends the access token as the socket's first frame. The relay reads
    /// nothing else until this frame opens the session (`SRV-031`).
    pub(crate) fn authenticate_socket(&mut self) {
        // The frame shares the token's wrapper; no bare copy of it is made.
        let Some(token) = self
            .tokens
            .borrow()
            .as_ref()
            .map(|t| t.access_token().clone())
        else {
            crate::log::warning!("Sync: socket opened before the access token is ready");
            return;
        };
        self.ws_send(&WsClientMsg::Authenticate { token });
    }

    // -- WebSocket lifecycle ---------------------------------------------------

    /// Opens the session-level multiplexed WebSocket. Called after an unlock or
    /// a key setup. If the socket is already present (reconnection), it is
    /// replaced. All open documents are automatically re-subscribed.
    #[cfg(target_arch = "wasm32")]
    // The wasm twin: a host build never compiles it, so cargo-mutants skips it.
    #[cfg_attr(test, mutants::skip)]
    pub(crate) fn open_websocket(&mut self) {
        if self.update_owed || self.inbound_held {
            return;
        }
        // The socket mirrors the REST scheme (`socket_address`). A
        // hardcoded `wss://` fails on a localhost http dev server (TLS error,
        // WS close code 1015).
        if self.tokens.borrow().is_none() {
            crate::log::warning!("Sync: WebSocket open requested before access token is ready");
            return;
        }
        let url = crate::http::Url::<crate::http::Socket>::of(self.url_scheme, &self.api_base);
        match WebSocket::new(url.as_str()) {
            Ok(ws) => {
                ws.set_binary_type(web_sys::BinaryType::Arraybuffer);
                self.wire_ws_callbacks(&ws);
                self.ws = Some(ws);
                // The token and the subscriptions are sent from the `onopen`
                // handler, NOT here. The socket is still CONNECTING at this point
                // and `ws_send` would be dropped. The server would then hold a
                // socket that never authenticates, and close it.
                crate::log::info!("Sync: WebSocket connecting ({url})");
            }
            Err(e) => {
                crate::log::error!("Sync: WebSocket::new({url}) failed: {e:?}");
            }
        }
    }

    /// Native / host-test stub: does nothing (mock transport handles WS).
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn open_websocket(&mut self) {
        if self.update_owed || self.inbound_held {
            return;
        }
        self.socket_opens = self.socket_opens.incremented();
        crate::log::info!("Sync: WebSocket opened (native mock)");
    }

    /// (Re)subscribes every server-side document once the socket is OPEN, and
    /// pulls anything missed while disconnected. Called from the `onopen` handler
    /// so it works identically for the first connection and every reconnect
    /// (after a network drop or a silent reauth). Local-only documents are not
    /// subscribed here -- they subscribe after lazy creation materializes them.
    pub(crate) fn resubscribe_all(&mut self) {
        // Each replay below starts afresh, so its stops count from zero
        // (`SYNC-154`). The relay's window counts per socket (`SYNC-188`).
        self.replay_stops.clear();
        self.replay_asks.borrow_mut().clear();
        for (doc_id, last_seq) in self
            .documents
            .iter()
            .filter(|(_, d)| d.existence == crate::Existence::OnRelay)
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
        // A channel watched from the head has no cursor to replay from yet.
        for document_id in self.light_heads() {
            self.ws_send(&WsClientMsg::Subscribe { document_id });
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
                // The copy into the worker's memory comes after the check:
                // that memory never shrinks (`SYNC-277`, audit `W206`).
                let len = ambre_core::Count::<ambre_core::Octets>::new(u64::from(
                    array_buf.byte_length(),
                ));
                if !frame_fits(len) {
                    crate::log::error!(
                        "Sync: a socket frame of {} bytes passes its ceiling",
                        len.get()
                    );
                    return;
                }
                let frame = ambre_core::Payload::<WireFrame>::new(
                    js_sys::Uint8Array::new(&array_buf).to_vec(),
                );
                match decode_server_msg(&frame) {
                    Ok(server_msg) => {
                        let _ = tx.send(SyncEvent::WsMessage(server_msg));
                    }
                    Err(e) if e.is_from_newer_peer() => {
                        let _ = tx.send(SyncEvent::WireOutdated);
                    }
                    Err(e) => crate::log::error!("Sync: WS decode error: {e:?}"),
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
                    code: Bounded::<CloseCode>::new(ce.code()),
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
            lifecycle::Next::Wait { delay } => self.arm_reconnect_timer(delay),
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
    // The wasm twin: a host build never compiles it, so cargo-mutants skips it.
    #[cfg_attr(test, mutants::skip)]
    fn jitter() -> f64 {
        js_sys::Math::random()
    }

    /// Native: no jitter, so a host test reads the exact delays.
    #[cfg(not(target_arch = "wasm32"))]
    fn jitter() -> f64 {
        0.0
    }

    /// Asks the worker's timers for a [`SyncEvent::ReconnectNow`] in
    /// `delay`. The shell only schedules: the lifecycle decides what the
    /// timer means when it fires.
    #[cfg(target_arch = "wasm32")]
    fn arm_reconnect_timer(&mut self, delay: TimeSpan<Ms>) {
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
                i32::try_from(delay.get()).unwrap_or(i32::MAX),
            );
            crate::log::warning!("Sync: reconnect scheduled in {}ms", delay.get());
        }
        // Keep the closure alive until it fires (replaces any previous pending one).
        self._reconnect_timer = Some(cb);
    }

    /// Native: no timer. A host test advances the clock and sends
    /// [`SyncEvent::ReconnectNow`] itself.
    #[cfg(not(target_arch = "wasm32"))]
    fn arm_reconnect_timer(&mut self, _delay: TimeSpan<Ms>) {}

    /// Asks the worker's timers for `event` in `delay`, such as a
    /// [`SyncEvent::ReplayRetry`]. The closure runs once and frees itself, so
    /// several documents can wait at once.
    #[cfg(target_arch = "wasm32")]
    pub(crate) fn arm_event_timer(&self, event: SyncEvent, delay: TimeSpan<Ms>) {
        use wasm_bindgen::JsCast;

        let Some(tx) = self.event_tx.clone() else {
            return;
        };
        let cb = Closure::once_into_js(move || {
            let _ = tx.send(event);
        });
        if let Ok(scope) = js_sys::global().dyn_into::<web_sys::DedicatedWorkerGlobalScope>() {
            let _ = scope.set_timeout_with_callback_and_timeout_and_arguments_0(
                cb.unchecked_ref(),
                i32::try_from(delay.get()).unwrap_or(i32::MAX),
            );
        }
    }

    /// Native: no timer. A host test sends the event itself.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn arm_event_timer(&self, _event: SyncEvent, _delay: TimeSpan<Ms>) {}

    /// The relay stopped a replay. Ask again after a delay that doubles at
    /// each stop (`SYNC-153`). A stop past the one before starts the budget
    /// again (`SYNC-158`). Past the budget, report it once and wait for the
    /// next reconnect (`SYNC-154`).
    fn replay_stopped(&mut self, document_id: Id<Document>, after_seq: Seq<Relay>) {
        let stop = match self.replay_stops.get(&document_id) {
            Some(last) if after_seq <= last.after => last.count.incremented(),
            _ => Count::new(1),
        };
        crate::log::warning!(
            "Sync: replay of {document_id:?} stopped after {after_seq:?} (stop {stop})"
        );
        let delay = replay_retry_delay(stop);
        let ticket = if delay.is_some() {
            self.replay_ticket.issue()
        } else {
            self.replay_ticket.latest()
        };
        self.replay_stops.insert(
            document_id,
            ReplayStops {
                count: stop,
                after: after_seq,
                ticket,
            },
        );
        if let Some(delay) = delay {
            let retry = SyncEvent::ReplayRetry {
                document_id,
                ticket,
            };
            self.arm_event_timer(retry, delay);
        } else if stop == REPLAY_RETRY_LIMIT.incremented() {
            crate::log::warning!("Sync: replay of {document_id:?} stopped {stop} times");
            // A first replay given up is an open that failed: no ready would
            // ever follow it, and nothing of the document is kept (`SYNC-157`).
            let opening = self
                .documents
                .get(&document_id)
                .is_some_and(|doc| doc.awaiting_initial_hydration);
            if opening {
                self.handle_close_document(document_id);
                self.replay_stops.remove(&document_id);
                self.send_to_logic(LogicEvent::DocumentOpenFailed {
                    document_id,
                    cause: OpenFailure::Transient,
                });
            } else {
                self.send_to_logic(LogicEvent::ApiError {
                    id: None,
                    request: ApiRequest::Websocket,
                    failure: ApiFailure::refused(RefusalCode::InternalError),
                });
            }
        }
    }

    /// Whether a replay ask of `document_id` may leave now. The relay answers
    /// nothing to a second ask inside its window (`SRV-086`). So an ask that
    /// comes inside it is held, and leaves once the window passes
    /// (`SYNC-188`).
    pub(crate) fn space_replay_ask(
        &self,
        document_id: Id<Document>,
        after_seq: Seq<Relay>,
    ) -> AskSlot {
        let now = (self.clock)();
        let mut asks = self.replay_asks.borrow_mut();
        let Some(ask) = asks.get_mut(&document_id) else {
            asks.insert(
                document_id,
                ReplayAsk {
                    at: now,
                    held: None,
                },
            );
            return AskSlot::Now;
        };
        let due = ask.at.plus(REPLAY_ASK_SPACING);
        if now >= due {
            *ask = ReplayAsk {
                at: now,
                held: None,
            };
            return AskSlot::Now;
        }
        match ask.held {
            Some(held) => ask.held = Some(held.min(after_seq)),
            None => {
                ask.held = Some(after_seq);
                self.arm_event_timer(SyncEvent::ReplayAskDue { document_id }, due.since(now));
            }
        }
        AskSlot::Held
    }

    /// Move the worker's clock past the relay's replay window, as the time
    /// between two asks a test models does.
    #[cfg(test)]
    pub(crate) fn pass_replay_window(&mut self) {
        let before = Rc::clone(&self.clock);
        self.clock = Rc::new(move || before().plus(REPLAY_ASK_SPACING));
    }

    /// The window of a held replay ask passed. A document let go since asks
    /// nothing. A timer that fires early holds the ask again.
    fn replay_ask_due(&mut self, document_id: Id<Document>) {
        let held = self
            .replay_asks
            .borrow_mut()
            .get_mut(&document_id)
            .and_then(|ask| ask.held.take());
        let subscribed =
            self.documents.contains_key(&document_id) || self.light.contains_key(&document_id);
        if let Some(after_seq) = held.filter(|_| subscribed) {
            self.ws_send(&WsClientMsg::RequestMissing {
                document_id,
                after_seq,
            });
        }
    }

    /// A replay timer fired. A later stop or a reconnect since it was armed
    /// makes it stale. Otherwise ask again from the document's own cursor,
    /// which the delivered part of the replay already advanced (`SYNC-153`).
    fn retry_replay(
        &mut self,
        document_id: Id<Document>,
        ticket: ambre_core::Seq<crate::generation::ReplayTicket>,
    ) {
        let live = self
            .replay_stops
            .get(&document_id)
            .is_some_and(|stops| stops.ticket == ticket && stops.count <= REPLAY_RETRY_LIMIT);
        if !live {
            return;
        }
        let cursor = match self.documents.get(&document_id) {
            Some(doc) if doc.existence == crate::Existence::OnRelay => Some(doc.last_seq),
            Some(_) => None,
            None => self.light_cursor(document_id),
        };
        if let Some(after_seq) = cursor {
            self.ws_send(&WsClientMsg::RequestMissing {
                document_id,
                after_seq,
            });
        }
    }

    /// Silently renews the access token after a WS auth rejection (close
    /// [`ambre_protocol::ws::RelayClose::UNAUTHORIZED`]) and signals the
    /// outcome via [`SyncEvent::Reauthenticated`]. Runs the refresh off the
    /// select loop so it does not block; the shared token store is updated in
    /// place on success.
    #[cfg(target_arch = "wasm32")]
    // The wasm twin: a host build never compiles it, so cargo-mutants skips it.
    #[cfg_attr(test, mutants::skip)]
    pub(crate) fn reauthenticate(&mut self) {
        let Some(tx) = self.event_tx.clone() else {
            return;
        };
        let auth = self.auth_context();
        let stale = self
            .tokens
            .borrow()
            .as_ref()
            .map(|t| t.access_token().clone());
        crate::spawn(async move {
            let renewed = match stale {
                Some(stale) => auth.renew(&stale).await,
                None => Err(crate::http::HttpError::Local(
                    ambre_core::LocalFailure::NotSignedIn,
                )),
            };
            let ok = match renewed {
                Ok(()) => true,
                Err(e) => {
                    crate::log::warning!("Sync: token refresh failed: {e}");
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
        self.handle_ws_disconnected(None, reason.to_owned());
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
    pub(crate) fn handle_ws_disconnected(
        &mut self,
        code: Option<Bounded<CloseCode>>,
        reason: String,
    ) {
        crate::log::warning!("Sync: WS disconnected (code={code:?}): {reason}");
        self.ws = None;
        self._onmessage = None;
        self._onclose = None;
        self._onerror = None;
        for doc_id in self.documents.keys().copied().collect::<Vec<_>>() {
            self.send_to_logic(LogicEvent::ConnectionStatus {
                document_id: doc_id,
                connected: ambre_core::LinkState::Disconnected,
            });
        }
    }

    /// Hands a close with `code`, or an error with none, to the lifecycle,
    /// then carries out what it decided.
    fn socket_closed(&mut self, code: Option<Bounded<CloseCode>>) {
        let demand = if self.documents.is_empty() {
            crate::lifecycle::Demand::Unneeded
        } else {
            crate::lifecycle::Demand::Needed
        };
        let next = self
            .lifecycle
            .closed(code, demand, (self.clock)(), Self::jitter());
        self.follow(next);
    }

    /// Processes one [`SyncEvent`] received from the internal event queue.
    /// Dispatches WebSocket frames to the existing [`handle_ws_message`]
    /// pipeline, applies state mutations from HTTP result events, and handles
    /// disconnection / errors.
    /// Hold the keys a setup just stored, and the pair the relay bound to
    /// them, then open the socket the setup owed (`SYNC-133`).
    fn adopt_new_keys(
        &mut self,
        master_seed: MasterSeed,
        signing_key: SigningKey,
        exchange_key: StaticSecret,
        tokens: http::TokenPair,
    ) {
        self.master_seed = Some(master_seed);
        self.signing_key = Some(signing_key);
        self.exchange_key = Some(exchange_key);
        self.hold_tokens(tokens);
        self.setup_owed = false;
        self.open_websocket();
        self.flush_pending_opens();
    }

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
                } else if let Some(username) = self.dev_user() {
                    // Refresh token also dead (for example server restarted with a fresh
                    // ephemeral key and empty Redis): re-login with the same
                    // deterministic dev identity. SessionReady reopens the socket.
                    crate::log::warning!(
                        "Sync: refresh failed, re-running dev login for silent recovery"
                    );
                    self.handle_dev_login(None, username);
                } else {
                    // Real session with no usable refresh: surface so the UI can
                    // prompt re-auth. Fall back to backoff to avoid a tight loop.
                    crate::log::error!("Sync: reauth failed and no dev fallback; backing off");
                    self.schedule_reconnect();
                }
            }
            SyncEvent::WsError => {
                crate::log::error!("Sync: WebSocket error");
                // A raw socket `onerror` is not always followed by a clean
                // `onclose`, so treat it as a disconnect. Tear down the dead
                // socket and reconnect on the same backoff path as
                // `WsDisconnected` (M10). Without the teardown, an `onerror`
                // with no close would leave `self.ws` set and stall the
                // `ReconnectNow` guard (`self.ws.is_none()`). A following
                // `onclose`, if any, is idempotent here.
                self.handle_ws_disconnected(None, "ws error".to_owned());
                self.socket_closed(None);
            }
            SyncEvent::RenewalDue { ticket } => self.renewal_due(ticket),
            SyncEvent::TokensRenewed => self.tokens_renewed(),
            SyncEvent::CallLog { meta, log } => self.call_log(meta, log),
            SyncEvent::CallRelayOpened => self.call_relay_opened(),
            SyncEvent::CallRelay(msg) => self.call_relay(msg),
            SyncEvent::CallRelayClosed => self.call_relay_closed(),
            SyncEvent::CallSfuOpened => self.call_sfu_opened(),
            SyncEvent::CallSfu(msg) => self.call_sfu(msg),
            SyncEvent::CallTick => self.call_tick(),
            SyncEvent::CallLeaveDue => self.call_leave_due(),
            SyncEvent::CallRosterRead {
                call,
                channel,
                meta,
                roster,
                log,
            } => self.call_roster_read(call, channel, meta, roster, log),
            SyncEvent::ReconnectNow => {
                self._reconnect_timer = None;
                let next = self.lifecycle.poll((self.clock)());
                self.follow(next);
            }
            SyncEvent::ReplayRetry {
                document_id,
                ticket,
            } => self.retry_replay(document_id, ticket),
            SyncEvent::ResendRetry {
                document_id,
                ticket,
            } => self.resend_failed(document_id, ticket),
            SyncEvent::ReplayAskDue { document_id } => self.replay_ask_due(document_id),
            SyncEvent::CursorsDue => self.send_owed_cursors(),
            SyncEvent::SessionReady {
                id,
                user_id,
                display_name,
                email,
                address,
                master_seed,
                signing_key,
                exchange_key,
                tokens,
            } => {
                self.user_id = Some(user_id);
                self.master_seed = Some(master_seed);
                self.signing_key = Some(signing_key);
                self.exchange_key = Some(exchange_key);
                self.hold_tokens(tokens);
                self.setup_owed = false;
                self.open_websocket();
                self.flush_pending_opens();
                self.announce_session(
                    id,
                    RestEvent::SessionReady {
                        user_id,
                        display_name,
                        email,
                    },
                    address,
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
                self.hold_tokens(tokens);
                self.setup_owed = keys_stored == ambre_bus::KeySetup::Absent;
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
            SyncEvent::KeysSetUp {
                id,
                recovery_words,
                master_seed,
                signing_key,
                exchange_key,
                tokens,
                user_id,
                display_name,
                email,
                address,
            } => {
                self.user_id = Some(user_id);
                self.adopt_new_keys(master_seed, signing_key, exchange_key, tokens);
                // The words first, then the session: the page shows the words
                // before the list (`CHROME-099`).
                self.reply(id, RestEvent::AccountCreated { recovery_words });
                self.announce_session(
                    None,
                    RestEvent::SessionReady {
                        user_id,
                        display_name,
                        email,
                    },
                    address,
                );
            }
            SyncEvent::AccountRecovered {
                id,
                master_seed,
                signing_key,
                exchange_key,
                tokens,
                user_id,
                display_name,
                email,
                address,
            } => {
                self.reply(id, RestEvent::AccountRecovered);
                // A session that holds keys already re-sealed its own seed. The
                // words matched its signing key, so it holds the recovered
                // keys, and its socket stays (`SYNC-294`, `CHROME-135`).
                if self.master_seed.is_some() {
                    return;
                }
                self.user_id = Some(user_id);
                self.adopt_new_keys(master_seed, signing_key, exchange_key, tokens);
                // The form closes on the answer, then the list replaces the
                // gate: no second unlock under the new passphrase (`SYNC-293`).
                self.announce_session(
                    None,
                    RestEvent::SessionReady {
                        user_id,
                        display_name,
                        email,
                    },
                    address,
                );
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
            // A Murmure open that raced an open under another type installs
            // nothing: its key met the checks of a channel, not of this
            // document (`SYNC-260`).
            SyncEvent::OpenDocumentReady {
                document_id,
                doc_type,
                ..
            } if doc_type == DocType::Murmure && self.typed_opens.contains(&document_id) => {
                crate::log::warning!(
                    "Sync: Murmure open of {document_id:?} dropped: held as another type"
                );
            }
            SyncEvent::OpenDocumentReady {
                document_id,
                doc_key,
                snapshot_payload,
                snapshot_id,
                covered_seq,
                held_snapshot,
                doc_type,
                existence,
                signing_keys,
                role,
                rotation_owed,
                links,
                writers,
                departed,
            } => {
                let log_document = self.log_document(document_id);
                // A listed name is checked against this walk (`SYNC-135`).
                self.walked_writers.insert(log_document, writers.clone());
                let doc_state = self
                    .documents
                    .entry(document_id)
                    .insert_entry(SyncDocumentState::new(
                        document_id,
                        doc_key,
                        doc_type,
                        existence,
                    ))
                    .into_mut();
                doc_state.chain = crate::chain::Chain::new(log_document, links);
                doc_state.set_signing_keys(signing_keys);
                doc_state.set_writers(writers);
                doc_state.role = role;
                // The open checked the key against this departure: a debt it
                // owes is the relay's word below (`SYNC-222`, `SYNC-223`).
                doc_state.departed = departed;
                // A writer pays an owed rotation before its first frame; a
                // reader sends none, so it owes nothing (`SYNC-128`).
                if rotation_owed == ambre_core::RotationDebt::Owed && role.can_write_document() {
                    doc_state.rekey.found_debt();
                }
                // The blob object storage holds is the sealed snapshot. It is
                // opened here, and only a snapshot that opens becomes the known
                // parent and gates the replay (`SYNC-030`). One that does not
                // falls back to the full replay from genesis, so no history
                // is skipped on the strength of a snapshot nobody applied.
                let served = pipeline::ServedSnapshot {
                    snapshot_id,
                    covered_seq,
                };
                let opened =
                    snapshot_payload.map(|start| doc_state.open_start_blob(&start, served));
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
                            covered_seq,
                        });
                    }
                    Some(Err(e)) => {
                        crate::log::warning!("Sync: the latest snapshot does not open: {e}");
                        self.send_to_logic(LogicEvent::SyncWarning {
                            document_id,
                            code: SyncWarningCode::DecryptFailed,
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
                let awaiting_replay = existence == crate::Existence::OnRelay && self.ws_connected();
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
                        .unwrap_or(Seq::<Relay>::new(0));
                    self.ws_send(&WsClientMsg::RequestMissing {
                        document_id,
                        after_seq,
                    });
                }
                self.send_to_logic(LogicEvent::ConnectionStatus {
                    document_id,
                    connected: ambre_core::LinkState::connected_if(self.ws_connected()),
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
                if existence == crate::Existence::LocalOnly {
                    self.flush_pre_session_updates(document_id);
                }
            }
            SyncEvent::OpenDocumentFailed { document_id, cause } => {
                crate::log::error!("Sync: open document {document_id:?} failed: {cause}");
                self.send_to_logic(LogicEvent::DocumentOpenFailed { document_id, cause });
            }
            SyncEvent::RootsVouched { roots } => self.murmure_roots.extend(roots),
            SyncEvent::ChannelListed {
                channel_id,
                listed_in,
                result,
            } => {
                self.channel_listed(channel_id, listed_in, result);
            }
            SyncEvent::LogWalked {
                document_id,
                word,
                heads,
            } => {
                if let Some(doc_state) = self.documents.get_mut(&document_id) {
                    let changed = doc_state.word != word;
                    doc_state.word = word;
                    doc_state.head_watch.read(heads);
                    if changed {
                        self.space_word_changed(document_id);
                    }
                }
            }
            SyncEvent::FootingAdvanced {
                document_id,
                footing,
            } => {
                if let Some(doc_state) = self.documents.get_mut(&document_id) {
                    match &mut doc_state.footing {
                        Some(held) => held.advance(footing),
                        None => doc_state.footing = Some(footing),
                    }
                    self.log_walked_to(footing);
                    self.keep_log(document_id);
                }
            }
            SyncEvent::ChannelRotated {
                space_id,
                channel_id,
                result,
            } => self.channel_rotated(space_id, channel_id, result),
            SyncEvent::ActivityFetched { result } => self.activity_fetched(result),
            SyncEvent::ConverterFetched { family, result } => {
                self.converter_fetched(family, result)
            }
            SyncEvent::SpaceChannelsListed { space_id, result } => {
                self.space_channels_listed(space_id, result);
            }
            SyncEvent::MemberChannelsListed { asked, result } => {
                self.member_channels_listed(asked, result);
            }
            SyncEvent::ChannelCreated { channel_id, result } => {
                // The creator lists a public channel in its catalogue. So its
                // log admits a self-join from the start, as on any later open
                // (`SYNC-106`, `SYNC-190`).
                if let Some((meta, space_id)) = self.creating_public.remove(&channel_id)
                    && result.is_ok()
                {
                    self.self_join_logs.insert(meta, space_id);
                }
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
            SyncEvent::SegmentClosed {
                channel_id,
                closing,
                result,
            } => {
                self.send_to_logic(match result {
                    Ok(crate::http::ClosedSegmentResponse { active, created }) => {
                        LogicEvent::SegmentClosed {
                            channel_id,
                            closing,
                            active,
                            successor: if created == ambre_core::Creation::Created {
                                ambre_bus::Successor::Created
                            } else {
                                ambre_bus::Successor::Fetched
                            },
                        }
                    }
                    Err(error) => LogicEvent::SegmentCloseFailed {
                        channel_id,
                        closing,
                        failure: crate::http::Reported::at(&error, "a channel request").failure,
                    },
                });
            }
            SyncEvent::AmendmentsLinked {
                channel_id,
                segment,
                amendments,
                result,
            } => {
                self.send_to_logic(match result {
                    Ok(held) => LogicEvent::AmendmentsLinked {
                        channel_id,
                        segment,
                        amendments: held,
                    },
                    Err(error) => LogicEvent::AmendmentsLinkFailed {
                        channel_id,
                        segment,
                        amendments,
                        failure: crate::http::Reported::at(&error, "a channel request").failure,
                    },
                });
            }
            SyncEvent::ThreadLinked {
                channel_id,
                segment,
                root,
                thread,
                result,
            } => {
                self.send_to_logic(match result {
                    Ok(held) => LogicEvent::ThreadLinked {
                        channel_id,
                        segment,
                        root,
                        thread: held,
                    },
                    Err(error) => LogicEvent::ThreadLinkFailed {
                        channel_id,
                        segment,
                        root,
                        thread,
                        failure: crate::http::Reported::at(&error, "a channel request").failure,
                    },
                });
            }
            SyncEvent::FinalizeLeaseAnswered {
                channel_id,
                segment,
                result,
            } => match result {
                Ok(answer) => self.send_to_logic(LogicEvent::FinalizeLeaseAnswered {
                    channel_id,
                    segment,
                    answer,
                }),
                // A segment no longer closing, or whose snapshot another
                // member sealed again, needs no lease from this client.
                Err(error) => {
                    let end = if error.is_conflict() {
                        ambre_core::FinalizeEnd::Withdrawn
                    } else {
                        ambre_core::FinalizeEnd::Failed
                    };
                    self.finalize_failed(channel_id, segment, end, &error);
                }
            },
            SyncEvent::SegmentFinalized {
                channel_id,
                segment,
                result,
            } => match result {
                Ok(()) => self.send_to_logic(LogicEvent::SegmentFinalized {
                    channel_id,
                    segment,
                }),
                Err(error) => {
                    self.finalize_failed(
                        channel_id,
                        segment,
                        ambre_core::FinalizeEnd::Failed,
                        &error,
                    );
                }
            },
            SyncEvent::FoldLeaseAnswered {
                channel_id,
                segment,
                result,
            } => match result {
                Ok(answer) => self.send_to_logic(LogicEvent::FoldLeaseAnswered {
                    channel_id,
                    segment,
                    answer,
                }),
                Err(error) => self.fold_failed(channel_id, segment, &error),
            },
            SyncEvent::SegmentFolded {
                channel_id,
                segment,
                result,
            } => match result {
                Ok(()) => self.send_to_logic(LogicEvent::SegmentFolded {
                    channel_id,
                    segment,
                }),
                Err(error) => self.fold_failed(channel_id, segment, &error),
            },
            SyncEvent::AmendmentsFolded {
                document_id,
                amendments,
            } => self.send_to_logic(LogicEvent::AmendmentsFolded {
                document_id,
                amendments,
            }),
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
                self.new_spaces.remove(&document_id);
                let pending = match self.documents.get_mut(&document_id) {
                    Some(doc_state) => {
                        doc_state.existence = crate::Existence::OnRelay;
                        doc_state.creating = false;
                        std::mem::take(&mut doc_state.pending_updates)
                    }
                    None => Vec::new(),
                };
                if self.ws_connected() {
                    self.ws_send(&WsClientMsg::Subscribe { document_id });
                }
                crate::log::info!(
                    "Sync: document {document_id:?} materialized, flushing {} update(s)",
                    pending.len()
                );
                for body in pending {
                    self.send_body(document_id, body);
                }
                self.flush_pending_uploads(document_id, None);
                self.flush_pending_invites(document_id, None);
                self.release_creations(document_id);
            }
            SyncEvent::DocumentCreateFailed {
                document_id,
                failure,
            } => {
                if let Some(doc_state) = self.documents.get_mut(&document_id) {
                    doc_state.creating = false;
                }
                crate::log::error!("Sync: lazy create failed for {document_id:?}: {failure}");
                self.flush_pending_uploads(document_id, Some(&failure));
                self.flush_pending_invites(document_id, Some(&failure));
                self.refuse_creations(document_id);
            }
            SyncEvent::HttpError { id: _id, failure } => {
                self.send_to_logic(LogicEvent::ApiError {
                    id: None,
                    request: ApiRequest::Rest,
                    failure,
                });
            }
            SyncEvent::InviteeKeyReady {
                id,
                document_id,
                user_id,
                display_name,
                email,
                email_proof,
                exchange_pub_key,
                signing_pub_key,
                fingerprint,
            } => {
                self.shown_identities.insert(
                    user_id,
                    ambre_crypto::IdentityKeys {
                        exchange: exchange_pub_key,
                        signing: signing_pub_key,
                    },
                );
                self.reply(
                    id,
                    RestEvent::InviteeKey {
                        document_id,
                        user_id,
                        display_name,
                        email,
                        email_proof,
                        exchange_pub_key,
                        fingerprint,
                    },
                );
            }
            SyncEvent::MemberKeyReady {
                id,
                user_id,
                exchange_pub_key,
                signing_pub_key,
                fingerprint,
            } => {
                self.shown_identities.insert(
                    user_id,
                    ambre_crypto::IdentityKeys {
                        exchange: exchange_pub_key,
                        signing: signing_pub_key,
                    },
                );
                self.reply(
                    id,
                    RestEvent::MemberKey {
                        user_id,
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
                inviter,
                invitee,
            } => {
                // This client signed the invitee into a Murmure log, as an
                // open would have verified them there (`SYNC-237`).
                if self
                    .documents
                    .get(&document_id)
                    .is_some_and(|doc| doc.doc_type == DocType::Murmure)
                {
                    self.murmure_roots.insert(invitee);
                }
                self.reply(
                    id,
                    RestEvent::InviteSent {
                        document_id,
                        user_id,
                        display_name,
                        inviter,
                    },
                );
            }
            SyncEvent::RevocationComplete {
                id,
                document_id,
                new_doc_key,
                link,
            } => {
                // A revoke by an owner is no departure (`SYNC-223`).
                let key = new_doc_key.id();
                self.install_key(document_id, new_doc_key, [link], None);
                self.send_to_logic(LogicEvent::KeyRotated { document_id });
                self.snapshot_after_rotation(document_id, key);
                self.reply(id, RestEvent::RevocationComplete { document_id });
                // The relay may push no frame of the removal: the call of the channel reads
                // the log this client just signed into (`CALL-136`).
                self.read_call_log(document_id);
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
                self.drive_space_rotation(document_id);
            }
            SyncEvent::DebtSettled {
                document_id,
                new_doc_key,
                link,
                departed,
            } => {
                let key = new_doc_key.id();
                self.install_key(document_id, new_doc_key, [link], departed);
                self.send_to_logic(LogicEvent::KeyRotated { document_id });
                self.snapshot_after_rotation(document_id, key);
            }
            SyncEvent::DebtGone { document_id } => self.fetch_rotated_key(document_id),
            SyncEvent::DebtUnsettled {
                document_id,
                message,
            } => {
                crate::log::warning!(
                    "Sync: the owed rotation of {document_id:?} failed: {message}"
                );
                if let Some(doc_state) = self.documents.get_mut(&document_id) {
                    doc_state.rekey.payment_failed();
                }
            }
            SyncEvent::DocumentMembersReady {
                id,
                asked,
                document_id,
                word,
                rotation_owed,
                members,
                signing_keys,
                heads,
                writers,
                departed,
                revoked,
                root,
            } => {
                // The `meta` of a channel never opened walked its log: its
                // light subscriptions count the summaries of its writers
                // alone from now on (`RESID-032`).
                if !self.documents.contains_key(&document_id) {
                    self.light_walked(document_id, &writers);
                }
                // Keep presence resolvable as membership evolves. A document that
                // is not open locally has no session to refresh -- the next open
                // rebuilds the directory from scratch. An answer older than the
                // directory held changes nothing (`SYNC-037`).
                let mut word_changed = false;
                let mut reread = false;
                let log_document = self.log_document(document_id);
                let fresh = match self.documents.get_mut(&document_id) {
                    Some(doc_state) if asked > doc_state.directory_held => {
                        doc_state.directory_held = asked;
                        doc_state.set_signing_keys(signing_keys);
                        // A walk that drops a writer the open walked is not
                        // the log the open checked: its word changes nothing
                        // (`SYNC-214`).
                        if doc_state.writers.grows_into(&writers) {
                            // A revoke that caps a writer below a delta this
                            // document applied leaves a write the log refuses
                            // in its state. The document is read again
                            // (`SYNC-300`).
                            if doc_state.applied_past_cap(&writers) {
                                reread = true;
                            }
                            // A listed name is checked against this walk
                            // (`SYNC-135`).
                            self.walked_writers.insert(log_document, writers.clone());
                            doc_state.set_writers(writers);
                            word_changed = doc_state.word != word;
                            doc_state.word = word;
                        } else {
                            crate::log::warning!(
                                "[Sync] doc={document_id}: a member listing walked a log that drops a writer; kept the log of the open"
                            );
                        }
                        // A leave owes a rotation the writer pays (`SYNC-128`).
                        // A departure past the one the current key was
                        // checked against owes it too, whatever the relay
                        // says: the leaver holds that key (`SYNC-223`). A
                        // walk that drops a writer still counts here, since a
                        // debt recorded costs one rotation and no secret.
                        let left = departed > doc_state.departed;
                        if left {
                            doc_state.departed = departed;
                        }
                        if (rotation_owed == ambre_core::RotationDebt::Owed || left)
                            && doc_state.role.can_write_document()
                        {
                            doc_state.rekey.found_debt();
                        }
                        // A revoke whose key the current key does not lead to
                        // left this client sealing under a key the revoked
                        // member holds. Edits wait for that key, which is
                        // fetched now in case the relay withheld its notice
                        // (`SYNC-224`).
                        let behind = revoked
                            .filter(|_| doc_state.role.can_write_document())
                            .filter(|revoke| doc_state.key_named(revoke.key).is_none());
                        if let Some(revoke) = behind {
                            doc_state.rekey.await_key(revoke.key);
                        }
                        Some(behind.is_some())
                    }
                    _ => None,
                };
                // A walk that finds the log rooted outside its space ends the
                // open, whatever an earlier walk accepted (`SYNC-231`).
                let rooted_outside =
                    fresh.is_some() && root == crate::membership_log::Root::Stranger;
                if rooted_outside {
                    self.fail_rooted_outside(document_id);
                } else if let Some(behind) = fresh {
                    if behind {
                        self.fetch_rotated_key(document_id);
                    }
                    self.check_served_heads(document_id, asked, heads);
                    // A delta held for its signer is settled by the writers
                    // this answer walked (`SYNC-211`).
                    self.settle_unvouched(document_id, asked);
                    // The answer of a walk owed lets the document land and
                    // seal again (`SYNC-232`).
                    self.release_walk(document_id, asked);
                    self.drive_space_rotation(document_id);
                }
                if word_changed {
                    self.space_word_changed(document_id);
                }
                if reread {
                    crate::log::warning!(
                        "[Sync] doc={document_id}: a revoke capped a writer below a delta applied; reading the document again"
                    );
                    self.send_to_logic(LogicEvent::DocumentReread { document_id });
                }
                match id {
                    Some(id) => self.reply(
                        id,
                        RestEvent::DocumentMembers {
                            document_id,
                            members,
                        },
                    ),
                    // A refresh no view asked for: the view takes its rows
                    // from this answer rather than asking again (`SYNC-151`).
                    None if fresh.is_some() => self.send_to_logic(LogicEvent::MembersChanged {
                        document_id,
                        members,
                    }),
                    None => {}
                }
            }
            SyncEvent::DocumentMembersFailed { document_id, asked } => {
                self.listing_failed(document_id, asked);
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
                departed,
            } => {
                // Swap the local key cache. This is the remaining-member path: a
                // rotation is not a revocation. Nothing is reported as a member
                // loss here; a revoked user is told via `DocumentRevoked`.
                self.install_key(document_id, new_doc_key, links, departed);
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
            SyncEvent::MediaErased {
                id,
                document_id,
                content_hash,
            } => {
                self.reply(
                    id,
                    RestEvent::MediaErased {
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

    /// This user no longer reaches `document_id`. Close it here, and tell the
    /// Logic thread they were removed from it, so the view reacts. It can be
    /// the `meta` of a channel never opened. Unless it is held as another
    /// kind, the user's channels are listed again (the Murmure layout spec).
    fn document_removed(&mut self, document_id: Id<Document>) {
        let other_kind = self
            .documents
            .remove(&document_id)
            .is_some_and(|d| d.doc_type != DocType::Murmure);
        self.light.remove(&document_id);
        if let Some(uid) = self.user_id {
            self.send_to_logic(LogicEvent::MemberRevoked {
                document_id,
                revoked_user_id: uid,
            });
        }
        if !other_kind {
            self.handle_list_member_channels();
        }
    }

    /// Demultiplex an inbound [`WsServerMsg`] received from the WebSocket.
    /// Decoding from wire bytes is done in the `onmessage` callback;
    /// this method receives the already-decoded message.
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
                // The relay's word that the members changed reaches this
                // client only from here on. A change committed before it,
                // after the open read its members or while the socket was
                // down, is read now (`SYNC-150`).
                if self
                    .documents
                    .get(&document_id)
                    .is_some_and(|doc| doc.existence == crate::Existence::OnRelay)
                {
                    self.handle_list_document_members(None, document_id);
                }
                self.send_to_logic(LogicEvent::ConnectionStatus {
                    document_id,
                    connected: ambre_core::LinkState::Connected,
                });
            }

            WsServerMsg::Update { seq, update } => {
                crate::log::debug!(
                    "[Sync<-Server] WS Update seq={} doc={} clock={}",
                    seq.get(),
                    update.public_data.document_id,
                    update.public_data.clock.get(),
                );
                self.receive_update(seq, update);
            }

            WsServerMsg::Ephemeral(eph) => {
                let doc_id = eph.public_data.document_id;
                let author_pk = eph.public_data.pub_key;
                // The head a sample carries is read before the open. The
                // sample a hidden rotation brings is under a key this client
                // lacks (`SYNC-263`). Our own head names nothing new.
                let authored_by_us = self
                    .signing_key
                    .as_ref()
                    .is_some_and(|sk| sk.verifying_key().to_bytes() == *author_pk.as_array());
                if !authored_by_us {
                    self.note_signed_head(crate::withheld::HeadFrame::Ephemeral(&eph));
                }
                let Some(doc_state) = self.documents.get_mut(&doc_id) else {
                    return;
                };

                // Presence is keyed by the sender's canonical `Id<User>`
                // (`ambre-spec-membership.md`), resolved from the authoring
                // Ed25519 key before delivery so a peer cannot spoof another
                // user's cursor. The client-side key directory (member signing
                // keys / trusted-keys) is not wired yet. An unattributable
                // sample is dropped rather than shown under a guessed identity.
                // Presence is best-effort; a dropped sample is corrected by the
                // next one.
                let Some(from_user_id) = doc_state.resolve_ephemeral_author(&author_pk) else {
                    crate::log::debug!(
                        "Sync: dropping ephemeral from unattributable author {author_pk:?}"
                    );
                    return;
                };
                let opened = doc_state
                    .open_ephemeral_named(&eph)
                    .and_then(|body| crate::pipeline::split_presence_body(&body));
                // Anti-replay: reject ephemerals with a non-monotonic counter for
                // the same open of the same author. The sealed body names the
                // open, so a reopened author counts from 1 again (`SYNC-287`).
                // The counter map is bounded (see `accept_ephemeral_counter` /
                // `MAX_EPHEMERAL_REMOTE_AUTHORS`). Only a sample whose signature
                // verified moves the counter, so a forged one cannot silence its
                // author (`SYNC-216`).
                if let Ok(body) = &opened
                    && !doc_state.accept_ephemeral_counter(
                        &author_pk,
                        &body.session,
                        eph.public_data.counter,
                    )
                {
                    crate::log::debug!(
                        "Sync: dropping replayed ephemeral from {:?} (counter {})",
                        author_pk,
                        eph.public_data.counter.get()
                    );
                    return;
                }
                let opened = opened.map(|body| body.sample);
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
                let Some(user_id) = doc_state.resolve_ephemeral_author(&pub_key) else {
                    crate::log::debug!("Sync: dropping PeerLeft from an unattributable key");
                    return;
                };
                self.send_to_logic(LogicEvent::PeerLeft {
                    document_id,
                    user_id,
                });
            }

            WsServerMsg::SnapshotDropped {
                document_id,
                author,
                reporter,
                after,
                ..
            } => {
                // Both keys resolve as a cursor's does: the member list names
                // the user.
                let Some(doc_state) = self.documents.get(&document_id) else {
                    return;
                };
                // The author of the dropped snapshot would seal it again, so
                // it leaves the segment to another member.
                let own = self
                    .signing_key
                    .as_ref()
                    .is_some_and(|key| key.verifying_key().to_bytes() == *author.as_array());
                self.send_to_logic(LogicEvent::SnapshotDropped {
                    document_id,
                    author: doc_state.resolve_ephemeral_author(&author),
                    reporter: doc_state.resolve_ephemeral_author(&reporter),
                    // The author would seal the same snapshot again.
                    after: if own {
                        ambre_core::AfterDrop::Keep
                    } else {
                        after
                    },
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
                    // The batch reaches Logic under the document the relay names, and the
                    // update opens under the document its sealed metadata names. A relay that
                    // files one document's update in another's replay would move its plaintext
                    // across a membership boundary, so the two must agree (`SYNC-203`).
                    if seq_update.update.public_data.document_id != document_id {
                        crate::log::warning!(
                            "Sync: a replay of {document_id:?} carries an update of another document, dropping"
                        );
                        continue;
                    }
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
                crate::log::info!("Sync: SnapshotAvailable {snapshot_id:?} for {document_id:?}");
                // Announced is not held: fetch it first (`SYNC-030`).
                self.fetch_announced_snapshot(document_id, snapshot_id, covered_seq);
            }

            WsServerMsg::KeyRotationRequired { document_id } => {
                // Remaining-member re-key path only. Being revoked is a separate,
                // targeted signal now (`DocumentRevoked`), so this message never
                // closes a document.
                self.fetch_rotated_key(document_id);
                // A rotation follows a removal: the call of the channel heeds it (`CALL-060`).
                self.read_call_log(document_id);
            }

            WsServerMsg::KeyRefused {
                document_id,
                key_id,
                clock,
            } => self.handle_key_refused(document_id, key_id, clock),

            WsServerMsg::ClockRefused {
                document_id,
                clock,
                expected,
                signature,
            } => {
                crate::log::info!(
                    "Sync: clock {} refused doc={document_id}, expected {}",
                    clock.get(),
                    expected.get()
                );
                self.handle_clock_refused(document_id, expected, signature);
            }

            WsServerMsg::DeltaRefused {
                document_id,
                signature,
                code,
            } => self.handle_delta_refused(document_id, signature, code),

            WsServerMsg::MembersChanged { document_id } => {
                // Only an open document holds a directory and a view; the next
                // open reads the rows from scratch. The directory is read
                // again here, whatever application holds the document, so a
                // newcomer's samples resolve (`SYNC-037`). Its answer reaches
                // the view too, which asks for nothing more (`SYNC-151`).
                if self.documents.contains_key(&document_id) {
                    self.handle_list_document_members(None, document_id);
                }
                // The call of the channel reads its log again (`CALL-060`).
                self.read_call_log(document_id);
            }

            WsServerMsg::ThreadActivity { document_id } => self.thread_activity(document_id),

            // A member this user invited lost the address it was invited
            // under (`SRV-295`). An open document reads its rows again, so the
            // view shows the member unproven and without the address.
            WsServerMsg::AddressReclaimed {
                document_id,
                member,
            } => {
                crate::log::info!("Sync: the address of {member} was reclaimed on {document_id}");
                if self.documents.contains_key(&document_id) {
                    self.handle_list_document_members(None, document_id);
                }
            }

            WsServerMsg::DocumentRevoked { document_id } => {
                // The server removed us from this document (targeted push on our
                // personal topic).
                crate::log::info!("Sync: DocumentRevoked {document_id:?}");
                self.document_removed(document_id);
            }

            WsServerMsg::DocumentShared {
                document_id,
                doc_type,
            } => {
                crate::log::info!("Sync: DocumentShared {document_id:?} ({doc_type:?})");
                self.send_to_logic(LogicEvent::DocumentShared {
                    document_id,
                    doc_type,
                });
                // A channel reaches a new member as the share of its meta
                // (`SRV-192`): the listing says which channel it is.
                if doc_type == DocType::Murmure {
                    self.handle_list_member_channels();
                }
            }

            // Only a subscription is refused so: a removal missed while the
            // socket was down (`SYNC-160`).
            WsServerMsg::Error {
                document_id: Some(document_id),
                code: RefusalCode::Forbidden,
            } if self.documents.contains_key(&document_id) => {
                crate::log::info!("Sync: subscription refused for {document_id}");
                self.document_removed(document_id);
            }
            // The relay erased a document still held here, as a fold erases
            // its amendments: it is dropped and Logic settles it (`SYNC-167`).
            WsServerMsg::Error {
                document_id: Some(document_id),
                code: RefusalCode::NotFound,
            } if self.documents.contains_key(&document_id) => {
                crate::log::info!("Sync: {document_id} is gone from the relay");
                self.documents.remove(&document_id);
                self.light.remove(&document_id);
                self.send_to_logic(LogicEvent::DocumentGone { document_id });
            }
            WsServerMsg::Error { document_id, code } => {
                crate::log::error!("Sync: socket refusal {code} for {document_id:?}");
                self.send_to_logic(LogicEvent::ApiError {
                    id: None,
                    request: ApiRequest::Websocket,
                    failure: ApiFailure::refused(code),
                });
            }

            WsServerMsg::ReplayStopped {
                document_id,
                after_seq,
            } => self.replay_stopped(document_id, after_seq),
        }
    }
}

impl SyncState {
    /// Fetches this client's newly wrapped key, unwraps it and installs it.
    /// A members answer that says a rotation is owed carries no new key: a
    /// member left, and the debt is recorded instead (`SYNC-128`).
    pub(crate) fn fetch_rotated_key(&self, document_id: Id<Document>) {
        let (Some(tx), Some(ek)) = (self.event_tx.clone(), self.exchange_key.clone()) else {
            return;
        };
        let api_base = self.api_base.clone();
        let url_scheme = self.url_scheme;
        let auth = self.auth_context();
        let rule = self.log_rule(document_id);
        let log_document = self.log_document(document_id);
        let doc_type = self.documents.get(&document_id).map(|doc| doc.doc_type);
        // A departure this client already walked stands, even when the log
        // served now omits it (`SYNC-223`).
        let departed = self
            .documents
            .get(&document_id)
            .and_then(|doc| doc.departed);
        let roots = doc_type.map(|doc_type| self.roots(document_id, doc_type));
        // The footing the session holds for the log. The served log walks
        // from it, never from the anchor of the served wrap (audit `W200`).
        let held = self.footing(document_id);
        crate::spawn(async move {
            let members_url = crate::http::Url::<crate::http::ApiRoot>::of(url_scheme, &api_base)
                .route(format!("/documents/{document_id}/members"));
            let resp = match http::get_json_auth::<http::MembersResponse>(
                &members_url,
                auth.clone(),
            )
            .await
            {
                Ok(resp) => resp,
                Err(e) => {
                    crate::log::error!("Sync: KeyRotationRequired fetch members failed: {e}");
                    return;
                }
            };
            if resp.rotation_owed == ambre_core::RotationDebt::Owed {
                let _ = tx.send(SyncEvent::DebtFound { document_id });
                return;
            }

            // The new key was wrapped by the rotator, and the unwrap runs
            // against their exchange key, not our own.
            let (new_doc_key, anchor) =
                match Self::unwrap_anchored(&ek, log_document, &resp.members) {
                    Ok(unwrapped) => unwrapped,
                    Err(e) => {
                        crate::log::error!("Sync: KeyRotationRequired unwrap failed: {e}");
                        return;
                    }
                };

            // The link the rotator left reaches back from the new key
            // (`SYNC-117`). It counts only when a writer the log names
            // signed it (`SYNC-121`).
            let links =
                match http::try_fetch_key_links(url_scheme, &api_base, document_id, auth).await {
                    Ok(links) => links,
                    Err(e) => {
                        crate::log::error!("Sync: KeyRotationRequired fetch links failed: {e}");
                        return;
                    }
                };
            // A log cut before a revoke this client walked, or rooted
            // elsewhere, installs no key (audit `W200`).
            let (log, footing) = match crate::sharing::walk_from_footing(
                held,
                log_document,
                anchor,
                &resp.log,
                &rule,
            ) {
                Ok(walked) => walked,
                Err(e) => {
                    crate::log::error!("Sync: KeyRotationRequired log refused: {e}");
                    let _ = tx.send(SyncEvent::DebtUnsettled {
                        document_id,
                        message: format!("the served log was refused: {e}"),
                    });
                    return;
                }
            };
            let links = crate::chain::signed_by_writers(links, &log.link_signers());
            // The new key passes the checks of an open before it becomes
            // current (`SYNC-200`, `SYNC-201`, `SYNC-202`).
            let own = crate::sharing::own_exchange_key(&ek);
            // A document no longer open checks nothing, as before.
            let checked = roots.is_some();
            let vouch = crate::key_vouch::KeyVouch::of(
                &log,
                &rule,
                &own,
                &resp.members,
                &roots.unwrap_or_default(),
                resp.rotation_owed,
            )
            .knowing(departed);
            if checked && let Err(refusal) = vouch.check(log_document, &new_doc_key, &links) {
                crate::log::error!("Sync: KeyRotationRequired key refused: {refusal:?}");
                // A payment the relay answered settled stays owed: the next
                // edit tries again, and none seals meanwhile (`SYNC-223`).
                let _ = tx.send(SyncEvent::DebtUnsettled {
                    document_id,
                    message: format!("the rotated key was refused: {refusal:?}"),
                });
                return;
            }
            let _ = tx.send(SyncEvent::KeyRotated {
                document_id,
                new_doc_key,
                links,
                departed: vouch.departed(),
            });
            if let Some(footing) = footing {
                let _ = tx.send(SyncEvent::FootingAdvanced {
                    document_id,
                    footing,
                });
            }
        });
    }

    /// The relay refused our frame at `clock` because the document owes a
    /// rotation (`SRV-123`). The clock is still free. Under the current key the
    /// debt is paid first; under an older one the frame is sent again now
    /// (`SYNC-128`, `SYNC-129`).
    pub(crate) fn handle_key_refused(
        &mut self,
        document_id: Id<Document>,
        key_id: FixedBytes<KeyId>,
        clock: Seq<DeltaAuthor>,
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
    use super::{REPLAY_RETRY_LIMIT, frame_fits, replay_retry_delay};
    use crate::http::{AccessToken, Credential, TokenPair};
    use crate::{LogicEvent, LogicSink, SyncDocumentState, SyncEvent, SyncState};
    use ambre_bus::DocType;
    use ambre_core::id::{Document, Id, User};
    use ambre_core::{Bounded, RefusalCode, Relay, Seq};
    use ambre_core::{Instant, UnixMs};
    use ambre_crypto::{DocumentKey, MasterSeed};
    use ambre_protocol::ws::{CloseCode, RelayClose};
    use ambre_protocol::ws::{WsClientMsg, WsServerMsg, decode_client_msg};

    /// `SYNC-277`, audit `W206`: a frame up to its ceiling is read, and one
    /// past it is not copied at all.
    #[test]
    fn a_socket_frame_past_its_ceiling_is_not_read() {
        let mib = |n: u64| ambre_core::Count::<ambre_core::Octets>::new(n * 1024 * 1024);
        assert!(frame_fits(mib(4)));
        assert!(frame_fits(mib(128)));
        assert!(!frame_fits(mib(128).plus(ambre_core::Count::new(1))));
    }

    /// A worker holding one open document, and the queue of what it tells Logic.
    fn with_open_document() -> (SyncState, ambre_bus::conduit::Receiver<LogicEvent>) {
        let (sink, rx) = LogicSink::channel();
        let mut state = SyncState::new(
            sink,
            crate::http::Url::<crate::RelayHost>::new(String::new()),
        );
        let id = Id::<Document>::mint();
        state.documents.insert(
            id,
            SyncDocumentState::new(
                id,
                DocumentKey::generate(),
                DocType::Prose,
                crate::Existence::LocalOnly,
            ),
        );
        (state, rx)
    }

    fn closed(code: Option<Bounded<CloseCode>>) -> SyncEvent {
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
        state.clock = std::rc::Rc::new(move || Instant::<UnixMs>::new(clock.get()));
        let fire_at = |state: &mut SyncState, at: u64| {
            now.set(at);
            state.handle_sync_event(SyncEvent::ReconnectNow);
            state.socket_opens.get()
        };

        state.handle_sync_event(closed(Bounded::<CloseCode>::new(1006)));
        assert_eq!(fire_at(&mut state, 249), 0, "early: nothing reopens");
        assert_eq!(fire_at(&mut state, 250), 1);

        // Opened, then refused before any frame: the wait doubles.
        state.handle_sync_event(SyncEvent::WsOpened);
        state.handle_sync_event(closed(Bounded::<CloseCode>::new(1006)));
        assert_eq!(fire_at(&mut state, 749), 1);
        assert_eq!(fire_at(&mut state, 750), 2);

        // A frame proves the session live, so the next wait is the first.
        let document_id = *state.documents.keys().next().expect("one document");
        state.handle_ws_message(ambre_protocol::ws::WsServerMsg::Subscribed { document_id });
        state.handle_sync_event(closed(Bounded::<CloseCode>::new(1006)));
        assert_eq!(fire_at(&mut state, 999), 2);
        assert_eq!(fire_at(&mut state, 1000), 3);
        assert_eq!(fire_at(&mut state, 5000), 3, "one open per deadline");
    }

    /// `SYNC-059`: while the inbound path holds the socket closed, only an
    /// edit is kept for the reopen. Any other frame is dropped: the reopen
    /// subscribes again on its own.
    #[test]
    fn while_held_only_an_edit_is_kept_for_the_reopen() {
        let (mut state, _rx) = with_open_document();
        state.inbound_held = true;
        let document_id = *state.documents.keys().next().expect("one document");
        state.push_frame(&WsClientMsg::Subscribe { document_id });
        assert!(state.held_outbox.borrow().is_empty());
    }

    /// `SYNC-045`: a presence sample from a member opens under the key it
    /// names. It reaches Logic as the bytes its author sealed, under the
    /// member its signing key resolves to.
    #[test]
    fn a_remote_ephemeral_reaches_logic_as_its_author_sealed_it() {
        let (sink, rx) = LogicSink::channel();
        let mut state = SyncState::new(
            sink,
            crate::http::Url::<crate::RelayHost>::new(String::new()),
        );
        let document_id = Id::<Document>::mint();
        let doc_key = DocumentKey::from_array([0x11; 32]);
        let signing = MasterSeed::from_array([0x22; 32]).derive_signing_key();
        let author = ambre_core::FixedBytes::from_array(signing.verifying_key().to_bytes());
        let alice = Id::<User>::mint();
        let mut doc_state = SyncDocumentState::new(
            document_id,
            doc_key.clone(),
            DocType::Prose,
            crate::Existence::LocalOnly,
        );
        doc_state.set_signing_keys([(author, alice)]);
        state.documents.insert(document_id, doc_state);
        let public_data = ambre_protocol::EphemeralPublicData {
            document_id,
            pub_key: author,
            key_id: doc_key.id(),
            counter: Seq::<ambre_protocol::messages::Ephemerals>::new(1),
            log_head: ambre_protocol::LogHead::EMPTY,
            log_entry: None,
        };
        let session = ambre_core::FixedBytes::from_array([5; 16]);
        let eph = crate::pipeline::seal_ephemeral(
            &crate::pipeline::presence_body(
                &session,
                &ambre_core::Payload::new(b"cursor@42".to_vec()),
            ),
            public_data,
            &doc_key,
            &signing,
        )
        .expect("seal");
        state.handle_ws_message(WsServerMsg::Ephemeral(eph));
        let delivered = std::iter::from_fn(|| rx.try_recv().ok()).find_map(|event| match event {
            LogicEvent::RemoteEphemeral {
                from_user_id,
                payload,
                ..
            } => Some((from_user_id, payload)),
            _ => None,
        });
        let Some((from, payload)) = delivered else {
            panic!("the sample reaches Logic");
        };
        assert_eq!(from, alice);
        assert_eq!(payload.as_bytes(), b"cursor@42");
    }

    /// `SYNC-216`: a sample whose signature fails moves no replay counter.
    /// The relay forges one under Alice's key at the highest counter, and
    /// her genuine sample at counter 2 still reaches Logic.
    #[test]
    fn a_forged_sample_moves_no_replay_counter() {
        let (sink, rx) = LogicSink::channel();
        let mut state = SyncState::new(
            sink,
            crate::http::Url::<crate::RelayHost>::new(String::new()),
        );
        let document_id = Id::<Document>::mint();
        let doc_key = DocumentKey::from_array([0x11; 32]);
        let signing = MasterSeed::from_array([0x22; 32]).derive_signing_key();
        let forger = MasterSeed::from_array([0x33; 32]).derive_signing_key();
        let author = ambre_core::FixedBytes::from_array(signing.verifying_key().to_bytes());
        let mut doc_state = SyncDocumentState::new(
            document_id,
            doc_key.clone(),
            DocType::Prose,
            crate::Existence::LocalOnly,
        );
        doc_state.set_signing_keys([(author, Id::<User>::mint())]);
        state.documents.insert(document_id, doc_state);
        let sample = |counter: u64, by: &ed25519_dalek::SigningKey| {
            let public_data = ambre_protocol::EphemeralPublicData {
                document_id,
                pub_key: author,
                key_id: doc_key.id(),
                counter: Seq::<ambre_protocol::messages::Ephemerals>::new(counter),
                log_head: ambre_protocol::LogHead::EMPTY,
                log_entry: None,
            };
            crate::pipeline::seal_ephemeral(
                &crate::pipeline::presence_body(
                    &ambre_core::FixedBytes::from_array([5; 16]),
                    &ambre_core::Payload::new(b"cursor".to_vec()),
                ),
                public_data,
                &doc_key,
                by,
            )
            .expect("seal")
        };
        state.handle_ws_message(WsServerMsg::Ephemeral(sample(u64::MAX, &forger)));
        state.handle_ws_message(WsServerMsg::Ephemeral(sample(2, &signing)));
        let delivered = std::iter::from_fn(|| rx.try_recv().ok())
            .filter(|event| matches!(event, LogicEvent::RemoteEphemeral { .. }))
            .count();
        assert_eq!(delivered, 1, "the genuine sample reaches Logic");
    }

    /// `SYNC-066`: a sign-out closes the socket and clears every key. A drop
    /// reported after it reopens nothing, where the same drop before it does.
    #[test]
    fn a_sign_out_closes_the_socket_and_clears_every_key() {
        // A drop, then the reconnect timer once the backoff has passed.
        let reopens = |state: &mut SyncState| {
            let now = std::rc::Rc::new(std::cell::Cell::new(0_u64));
            let clock = now.clone();
            state.clock = std::rc::Rc::new(move || Instant::<UnixMs>::new(clock.get()));
            state.handle_sync_event(closed(Bounded::<CloseCode>::new(1006)));
            now.set(60_000);
            state.handle_sync_event(SyncEvent::ReconnectNow);
            state.socket_opens.get()
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
            .insert(id, ambre_core::FixedBytes::from_array([1; 16]));
        *state.tokens.borrow_mut() = Some(TokenPair::new(
            Credential::new("access".to_owned()),
            Credential::new("refresh".to_owned()),
            ambre_core::TimeSpan::<ambre_core::Sec>::new(900),
        ));
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
        renewed.handle_sync_event(closed(Some(Bounded::<CloseCode>::UNAUTHORIZED)));
        assert_eq!(renewed.socket_opens.get(), 1);

        let (mut state, _rx) = with_open_document();
        state.handle_sync_event(closed(Some(Bounded::<CloseCode>::SETUP_REQUIRED)));
        state.handle_sync_event(SyncEvent::ReconnectNow);
        state.handle_sync_event(SyncEvent::Reauthenticated { ok: true });
        assert_eq!(state.socket_opens.get(), 0);

        let master_seed = MasterSeed::from_array([7u8; 32]);
        state.handle_sync_event(SyncEvent::KeysSetUp {
            id: ambre_bus::next_request(),
            recovery_words: ambre_bus::Confidential::<ambre_bus::RecoveryWords>::default(),
            signing_key: master_seed.derive_signing_key(),
            exchange_key: master_seed.derive_exchange_key(),
            master_seed,
            tokens: TokenPair::new(
                Credential::new("access".to_owned()),
                Credential::new("refresh".to_owned()),
                ambre_core::TimeSpan::<ambre_core::Sec>::new(900),
            ),
            user_id: Id::<User>::mint(),
            display_name: ambre_core::Validated::<ambre_core::DisplayName>::parse("Ada").unwrap(),
            email: "ada@example.org".into(),
            address: ambre_core::AddressStanding::Held,
        });
        assert_eq!(state.socket_opens.get(), 1);
    }

    /// A socket refused for a key setup still owed reaches Logic as a setup to
    /// complete, and as nothing else (`SYNC-134`).
    #[test]
    fn a_socket_refused_for_a_setup_owed_reaches_logic_as_a_setup() {
        let (mut state, rx) = with_open_document();
        state.handle_sync_event(closed(Some(Bounded::<CloseCode>::SETUP_REQUIRED)));
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
        state.handle_sync_event(closed(Some(Bounded::<CloseCode>::UNAUTHORIZED)));
        state.handle_sync_event(SyncEvent::ReconnectNow);
        state.handle_sync_event(SyncEvent::Reauthenticated { ok: true });
        assert_eq!(state.socket_opens.get(), 0);
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
        use ambre_protocol::messages::{Relay, Seq};
        use ambre_protocol::ws::{WsClientMsg, decode_client_msg};

        let (sink, _rx) = LogicSink::channel();
        let mut state = SyncState::new(
            sink,
            crate::http::Url::<crate::RelayHost>::new(String::new()),
        );
        state.signing_key = Some(MasterSeed::generate().derive_signing_key());
        let id = Id::<Document>::mint();
        let mut doc = SyncDocumentState::new(
            id,
            DocumentKey::generate(),
            DocType::Prose,
            crate::Existence::OnRelay,
        );
        doc.last_seq = Seq::<Relay>::new(7);
        state.documents.insert(id, doc);
        state
            .session_nonces
            .insert(id, ambre_core::FixedBytes::from_array([1; 16]));

        state.handle_command(SyncCommand::HoldInbound);
        state.handle_sync_event(closed(Bounded::<CloseCode>::new(1006)));
        state.handle_sync_event(SyncEvent::ReconnectNow);
        state.handle_sync_event(SyncEvent::Reauthenticated { ok: true });
        assert_eq!(state.socket_opens.get(), 0, "held: nothing reopens");
        state.handle_command(SyncCommand::SendUpdate {
            document_id: id,
            payload: b"typed during the hold".to_vec().into(),
            summary: None,
        });
        assert_eq!(state.held_outbox.borrow().len(), 1, "the edit is kept");

        let (tx, wire) = ambre_bus::conduit::unbounded();
        state.set_mock_ws(tx);
        state.handle_command(SyncCommand::ReleaseInbound);
        assert_eq!(state.socket_opens.get(), 1);
        state.handle_sync_event(SyncEvent::WsOpened);
        let sent: Vec<WsClientMsg> = wire
            .try_iter()
            .map(|bytes| decode_client_msg(&bytes).expect("decode"))
            .collect();
        assert!(matches!(sent[0], WsClientMsg::Subscribe { document_id } if document_id == id));
        assert!(matches!(
            sent[1],
            WsClientMsg::RequestMissing {
                after_seq,
                ..
            } if after_seq.get() == 7
        ));
        assert!(matches!(sent[2], WsClientMsg::Update(_)));
        assert_eq!(sent.len(), 3);
    }

    /// A reopened socket subscribes again to every document the relay holds
    /// (`SYNC-033`), and asks each for what it missed past its own cursor
    /// (`SYNC-086`). A document the relay never created gets neither.
    #[test]
    fn a_reconnect_resubscribes_and_pulls_every_relay_document() {
        use ambre_protocol::messages::{Relay, Seq};
        use ambre_protocol::ws::{WsClientMsg, decode_client_msg};

        let (sink, _rx) = LogicSink::channel();
        let mut state = SyncState::new(
            sink,
            crate::http::Url::<crate::RelayHost>::new(String::new()),
        );
        let mut cursors = std::collections::HashMap::new();
        for last in [7, 12] {
            let id = Id::<Document>::mint();
            let mut doc = SyncDocumentState::new(
                id,
                DocumentKey::generate(),
                DocType::Trame,
                crate::Existence::OnRelay,
            );
            doc.last_seq = Seq::<Relay>::new(last);
            state.documents.insert(id, doc);
            cursors.insert(id, Seq::<Relay>::new(last));
        }
        let local = Id::<Document>::mint();
        state.documents.insert(
            local,
            SyncDocumentState::new(
                local,
                DocumentKey::generate(),
                DocType::Trame,
                crate::Existence::LocalOnly,
            ),
        );

        let (tx, wire) = ambre_bus::conduit::unbounded();
        state.set_mock_ws(tx);
        state.handle_sync_event(SyncEvent::WsOpened);
        let sent: Vec<WsClientMsg> = wire
            .try_iter()
            .filter_map(|bytes| decode_client_msg(&bytes).ok())
            .collect();

        let subscribed: std::collections::HashSet<Id<Document>> = sent
            .iter()
            .filter_map(|m| match m {
                WsClientMsg::Subscribe { document_id } => Some(*document_id),
                _ => None,
            })
            .collect();
        let pulled: std::collections::HashMap<Id<Document>, Seq<Relay>> = sent
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
            crate::http::Url::<crate::http::Socket>::of(
                crate::http::UrlScheme::Https,
                &crate::http::Url::<crate::http::RelayHost>::new("relay.example".to_owned())
            )
            .as_str(),
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
        state.documents.get_mut(&id).unwrap().existence = crate::Existence::OnRelay;
        *state.tokens.borrow_mut() = Some(TokenPair::new(
            Credential::new("acc-s3cr3t".to_owned()),
            Credential::new("ref-s3cr3t".to_owned()),
            ambre_core::TimeSpan::<ambre_core::Sec>::new(900),
        ));
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
        assert_eq!(token.secret().expose(), "acc-s3cr3t");
        assert!(!format!("{:?}", sent[0]).contains("s3cr3t"));
        assert!(matches!(sent[1], WsClientMsg::Subscribe { document_id } if document_id == id));
    }

    /// A token pair logged by `Debug` prints neither token (`SYNC-061`).
    #[test]
    fn a_logged_token_pair_carries_no_token() {
        let pair = TokenPair::new(
            Credential::new("acc-s3cr3t".to_owned()),
            Credential::new("ref-s3cr3t".to_owned()),
            ambre_core::TimeSpan::<ambre_core::Sec>::new(900),
        );
        let shown = format!("{pair:?}");
        assert!(!shown.contains("s3cr3t"), "{shown}");
    }

    /// Both tokens wipe their bytes when dropped (`SYNC-100`): each holds an
    /// `ambre_core::Secret`, and the bound fails the build the day the
    /// wrapper stops wiping.
    #[test]
    fn session_tokens_wipe_on_drop() {
        fn wipes_on_drop<T: zeroize::ZeroizeOnDrop>(_: &T) {}
        let token = Credential::<AccessToken>::new("acc-s3cr3t".to_owned());
        wipes_on_drop(token.secret());
    }

    /// A worker holding one relay document at seq 7, and its socket's wire.
    fn with_relay_document() -> (
        SyncState,
        ambre_bus::conduit::Receiver<LogicEvent>,
        ambre_bus::conduit::Receiver<ambre_core::Payload<ambre_protocol::WireFrame>>,
        Id<Document>,
    ) {
        let (mut state, rx) = with_open_document();
        let id = *state.documents.keys().next().unwrap();
        let doc = state.documents.get_mut(&id).unwrap();
        doc.existence = crate::Existence::OnRelay;
        doc.last_seq = Seq::<Relay>::new(7);
        let (tx, wire) = ambre_bus::conduit::unbounded();
        state.set_mock_ws(tx);
        (state, rx, wire, id)
    }

    fn stopped(document_id: Id<Document>) -> SyncEvent {
        stopped_after(document_id, 3)
    }

    fn stopped_after(document_id: Id<Document>, after: u64) -> SyncEvent {
        SyncEvent::WsMessage(WsServerMsg::ReplayStopped {
            document_id,
            after_seq: Seq::<Relay>::new(after),
        })
    }

    fn retry(
        document_id: Id<Document>,
        ticket: ambre_core::Seq<crate::generation::ReplayTicket>,
    ) -> SyncEvent {
        SyncEvent::ReplayRetry {
            document_id,
            ticket,
        }
    }

    /// The ticket of the timer armed for the latest stop of `id`.
    fn armed(
        state: &SyncState,
        id: Id<Document>,
    ) -> ambre_core::Seq<crate::generation::ReplayTicket> {
        state.replay_stops[&id].ticket
    }

    /// The stops counted against the budget of `id`.
    fn counted(state: &SyncState, id: Id<Document>) -> u32 {
        state.replay_stops[&id].count.get()
    }

    fn sent(
        wire: &ambre_bus::conduit::Receiver<ambre_core::Payload<ambre_protocol::WireFrame>>,
    ) -> Vec<WsClientMsg> {
        wire.try_iter()
            .map(|bytes| decode_client_msg(&bytes).expect("decode"))
            .collect()
    }

    /// `SYNC-188`: the relay answers nothing to a second replay ask of one
    /// document inside its throttle window. So a second ask is held, and
    /// leaves once the window passed, with the lowest cursor held. A timer
    /// that fires early holds it again, a document let go asks nothing, and
    /// a reopened socket asks at once.
    #[test]
    fn a_replay_ask_inside_the_relay_window_waits_for_it_to_pass() {
        let (mut state, _rx, wire, id) = with_relay_document();
        let clock = std::rc::Rc::new(std::cell::Cell::new(1_000_000_u64));
        let now = std::rc::Rc::clone(&clock);
        state.clock = std::rc::Rc::new(move || Instant::<UnixMs>::new(now.get()));
        let ask = |after: u64| WsClientMsg::RequestMissing {
            document_id: id,
            after_seq: Seq::<Relay>::new(after),
        };
        let asked = || -> Vec<u64> {
            sent(&wire)
                .into_iter()
                .filter_map(|m| match m {
                    WsClientMsg::RequestMissing { after_seq, .. } => Some(after_seq.get()),
                    _ => None,
                })
                .collect()
        };
        let pass = |ms: u64| clock.set(clock.get().saturating_add(ms));
        let due = || SyncEvent::ReplayAskDue { document_id: id };

        state.ws_send(&ask(5));
        state.ws_send(&ask(3));
        state.ws_send(&ask(4));
        assert_eq!(asked(), [5], "the asks inside the window are held");
        pass(1_000);
        state.handle_sync_event(due());
        assert_eq!(
            asked(),
            Vec::<u64>::new(),
            "a timer that fires early asks nothing"
        );
        pass(250);
        state.handle_sync_event(due());
        assert_eq!(asked(), [3], "the lowest cursor, once the window passed");

        state.ws_send(&ask(6));
        let doc = state.documents.remove(&id).expect("held");
        pass(1_250);
        state.handle_sync_event(due());
        assert_eq!(asked(), Vec::<u64>::new(), "a document let go asks nothing");

        state.documents.insert(id, doc);
        state.ws_send(&ask(7));
        let _ = asked();
        state.handle_sync_event(SyncEvent::WsOpened);
        assert_eq!(asked(), [7], "a reopened socket asks at once");
    }

    /// `SYNC-153`: a stopped replay is asked again from the document's own
    /// cursor once its timer fires, and not before. A timer a later stop
    /// replaced asks nothing.
    #[test]
    fn a_stopped_replay_is_asked_again_from_the_cursor_when_its_timer_fires() {
        let (mut state, _rx, wire, id) = with_relay_document();
        state.handle_sync_event(stopped(id));
        let first = armed(&state, id);
        assert!(sent(&wire).is_empty(), "no retry before the timer");
        state.handle_sync_event(stopped(id));
        state.handle_sync_event(retry(id, first));
        assert!(sent(&wire).is_empty(), "a stale timer asks nothing");
        state.handle_sync_event(retry(id, armed(&state, id)));
        let asked = sent(&wire);
        assert_eq!(asked.len(), 1);
        assert!(matches!(
            asked[0],
            WsClientMsg::RequestMissing { document_id, after_seq } if document_id == id && after_seq.get() == 7
        ));
    }

    /// `SYNC-153`: the delay starts at 1 s and doubles. Five stops wait 31 s
    /// in all, and a sixth arms no timer (`ambre-spec-budgets.md` section 8).
    #[test]
    fn the_replay_retry_delay_doubles_up_to_its_budget() {
        let delays: Vec<Option<u64>> = (0..=6)
            .map(|stop| replay_retry_delay(ambre_core::Count::new(stop)).map(|d| d.get()))
            .collect();
        assert_eq!(
            delays,
            [
                None,
                Some(1_000),
                Some(2_000),
                Some(4_000),
                Some(8_000),
                Some(16_000),
                None
            ]
        );
        assert_eq!(delays.iter().flatten().sum::<u64>(), 31_000);
    }

    /// `SYNC-154`: past its budget a stopped replay reaches Logic as one
    /// error, and its timer asks nothing. A reopened socket counts afresh.
    #[test]
    fn a_replay_stopped_past_its_budget_reaches_logic_once() {
        let (mut state, rx, wire, id) = with_relay_document();
        for _ in 0..8 {
            state.handle_sync_event(stopped(id));
        }
        let errors = rx
            .try_iter()
            .filter(|e| {
                matches!(
                    e,
                    LogicEvent::ApiError {
                        failure: ambre_bus::ApiFailure::Relay {
                            status,
                            ..
                        },
                        ..
                    } if status.get() == 500
                )
            })
            .count();
        assert_eq!(errors, 1);
        state.handle_sync_event(retry(id, armed(&state, id)));
        assert!(sent(&wire).is_empty(), "no retry past the budget");

        state.handle_sync_event(SyncEvent::WsOpened);
        let _ = sent(&wire);
        state.handle_sync_event(stopped(id));
        assert_eq!(counted(&state, id), 1);
        state.pass_replay_window();
        state.handle_sync_event(retry(id, armed(&state, id)));
        assert!(matches!(
            sent(&wire)[..],
            [WsClientMsg::RequestMissing {
                after_seq,
                ..
            }] if after_seq.get() == 7
        ));
    }

    /// `SYNC-157`: a first replay given up is an open that failed. The
    /// document is let go, and neither a ready nor a generic error follows.
    #[test]
    fn a_first_replay_given_up_fails_the_open() {
        let (mut state, rx, wire, id) = with_relay_document();
        state
            .documents
            .get_mut(&id)
            .unwrap()
            .awaiting_initial_hydration = true;
        for _ in 0..=REPLAY_RETRY_LIMIT.get() {
            state.handle_sync_event(stopped(id));
        }
        let told: Vec<LogicEvent> = rx.try_iter().collect();
        assert!(
            matches!(
                told[..],
                [LogicEvent::DocumentOpenFailed { document_id, cause: ambre_bus::OpenFailure::Transient, .. }]
                    if document_id == id
            ),
            "a stopped replay is no missing key (`SYNC-173`): {told:?}"
        );
        assert!(!state.documents.contains_key(&id), "nothing of it is kept");
        assert!(matches!(
            sent(&wire)[..],
            [WsClientMsg::Unsubscribe { document_id }] if document_id == id
        ));
    }

    /// `SYNC-158`: a stop past the one before starts the budget again, so a
    /// replay that keeps moving on is never given up. A stop at the same
    /// delta, or before it, still counts. A timer armed before the reset
    /// asks nothing after it.
    #[test]
    fn a_replay_that_moves_on_keeps_its_budget() {
        let (mut state, rx, wire, id) = with_relay_document();
        for after in 1..=12 {
            state.handle_sync_event(stopped_after(id, after));
            assert_eq!(counted(&state, id), 1, "stop after {after}");
        }
        assert_eq!(rx.try_iter().count(), 0, "a moving replay reports nothing");

        let before_reset = armed(&state, id);
        state.handle_sync_event(stopped_after(id, 12));
        state.handle_sync_event(stopped_after(id, 11));
        assert_eq!(counted(&state, id), 3);
        state.handle_sync_event(stopped_after(id, 20));
        assert_eq!(counted(&state, id), 1);
        state.handle_sync_event(retry(id, before_reset));
        assert!(sent(&wire).is_empty(), "a timer from before the reset");
        state.handle_sync_event(retry(id, armed(&state, id)));
        assert_eq!(sent(&wire).len(), 1);
    }

    /// `SYNC-160`: a subscription refused for want of access closes its
    /// document and reports this user removed from it. Another code, or a
    /// document not held here, stays a refusal.
    #[test]
    fn a_refused_resubscription_closes_its_document() {
        let (mut state, rx, _wire, id) = with_relay_document();
        let me = Id::<User>::mint();
        state.user_id = Some(me);
        let refused = |document_id, code| {
            SyncEvent::WsMessage(WsServerMsg::Error {
                document_id: Some(document_id),
                code,
            })
        };

        state.handle_sync_event(refused(id, RefusalCode::RateLimited));
        state.handle_sync_event(refused(Id::<Document>::mint(), RefusalCode::Forbidden));
        assert!(state.documents.contains_key(&id));
        state.handle_sync_event(refused(id, RefusalCode::Forbidden));
        assert!(!state.documents.contains_key(&id));

        let told: Vec<LogicEvent> = rx.try_iter().collect();
        assert!(
            matches!(
                told[..],
                [
                    LogicEvent::ApiError { failure: ambre_bus::ApiFailure::Relay { status: status_1, .. }, .. },
                    LogicEvent::ApiError { failure: ambre_bus::ApiFailure::Relay { status: status_2, .. }, .. },
                    LogicEvent::MemberRevoked { document_id, revoked_user_id },
                ] if document_id == id && revoked_user_id == me && status_1.get() == 429 && status_2.get() == 403
            ),
            "{told:?}"
        );
    }

    /// `SYNC-167`: a document the relay no longer holds is dropped and
    /// reported gone. The same refusal for a document not held stays one.
    #[test]
    fn a_not_found_refusal_for_a_held_document_reports_it_gone() {
        let (mut state, rx, _wire, id) = with_relay_document();
        let refused = |document_id| {
            SyncEvent::WsMessage(WsServerMsg::Error {
                document_id: Some(document_id),
                code: RefusalCode::NotFound,
            })
        };

        state.handle_sync_event(refused(Id::<Document>::mint()));
        assert!(state.documents.contains_key(&id));
        state.handle_sync_event(refused(id));
        assert!(!state.documents.contains_key(&id));

        let told: Vec<LogicEvent> = rx.try_iter().collect();
        assert!(
            matches!(
                told[..],
                [
                    LogicEvent::ApiError { failure: ambre_bus::ApiFailure::Relay { status, .. }, .. },
                    LogicEvent::DocumentGone { document_id },
                ] if document_id == id && status.get() == 404
            ),
            "{told:?}"
        );
    }

    /// `SYNC-059`: holding the inbound path reports every open document
    /// disconnected. The release reopens only while documents wait, no key
    /// setup is owed and no socket is open.
    #[test]
    fn the_inbound_hold_reports_the_documents_and_the_release_reopens_when_wanted() {
        let (mut state, rx) = with_open_document();
        let id = *state.documents.keys().next().expect("one document");
        state.hold_inbound();
        assert!(matches!(
            rx.try_iter().collect::<Vec<_>>().as_slice(),
            [LogicEvent::ConnectionStatus { document_id, connected: ambre_core::LinkState::Disconnected }]
                if *document_id == id
        ));
        state.setup_owed = true;
        state.release_inbound();
        assert_eq!(state.socket_opens.get(), 0, "a setup is owed");

        let (mut empty, _rx) = with_open_document();
        empty.documents.clear();
        empty.hold_inbound();
        empty.release_inbound();
        assert_eq!(empty.socket_opens.get(), 0, "no document waits");

        let (mut wanted, _rx) = with_open_document();
        wanted.hold_inbound();
        wanted.release_inbound();
        assert_eq!(wanted.socket_opens.get(), 1);
    }

    /// `SYNC-070`: a dropped socket reports every open document disconnected.
    #[test]
    fn a_dropped_socket_reports_every_document_disconnected() {
        let (mut state, rx) = with_open_document();
        let id = *state.documents.keys().next().expect("one document");
        state.handle_sync_event(closed(Bounded::<CloseCode>::new(1006)));
        assert!(rx.try_iter().any(|event| matches!(
            event,
            LogicEvent::ConnectionStatus { document_id, connected: ambre_core::LinkState::Disconnected }
                if document_id == id
        )));
    }

    /// `SYNC-063`: a renewal that fails with no dev identity backs off, and
    /// the socket reopens once the wait is over.
    #[test]
    fn a_failed_renewal_backs_off_before_the_reopen() {
        let (mut state, _rx) = with_open_document();
        let now = std::rc::Rc::new(std::cell::Cell::new(0_u64));
        let clock = now.clone();
        state.clock = std::rc::Rc::new(move || Instant::<UnixMs>::new(clock.get()));
        state.handle_sync_event(SyncEvent::Reauthenticated { ok: false });
        now.set(249);
        state.handle_sync_event(SyncEvent::ReconnectNow);
        assert_eq!(state.socket_opens.get(), 0, "early: nothing reopens");
        now.set(250);
        state.handle_sync_event(SyncEvent::ReconnectNow);
        assert_eq!(state.socket_opens.get(), 1);
    }

    /// `SYNC-153`: a replay retry asks the relay for a document it holds,
    /// and never for a local one.
    #[test]
    fn a_replay_retry_asks_nothing_for_a_local_document() {
        let (mut state, _rx, wire, id) = with_relay_document();
        state.handle_sync_event(stopped(id));
        state.documents.get_mut(&id).expect("open").existence = crate::Existence::LocalOnly;
        let ticket = armed(&state, id);
        state.handle_sync_event(retry(id, ticket));
        assert!(
            !sent(&wire)
                .iter()
                .any(|m| matches!(m, WsClientMsg::RequestMissing { .. })),
            "a local document asked the relay"
        );
    }

    /// A removal or a share lists the user's channels again for a channel,
    /// and for no other kind of document (the Murmure layout spec).
    #[test]
    fn a_removal_or_a_share_lists_the_channels_again_for_a_channel_alone() {
        let (mut state, _rx, _wire, prose) = with_relay_document();
        let listed = |state: &SyncState| state.member_channels_asked.latest();
        let before = listed(&state);
        state.handle_ws_message(WsServerMsg::DocumentShared {
            document_id: Id::<Document>::mint(),
            doc_type: DocType::Prose,
        });
        assert_eq!(listed(&state), before, "a prose share");
        state.handle_ws_message(WsServerMsg::DocumentShared {
            document_id: Id::<Document>::mint(),
            doc_type: DocType::Murmure,
        });
        let after_share = listed(&state);
        assert_ne!(after_share, before, "a channel share");

        let forbidden = |document_id| WsServerMsg::Error {
            document_id: Some(document_id),
            code: RefusalCode::Forbidden,
        };
        state.handle_ws_message(forbidden(prose));
        assert_eq!(listed(&state), after_share, "a prose removal");
        let meta = Id::<Document>::mint();
        state.documents.insert(
            meta,
            SyncDocumentState::new(
                meta,
                DocumentKey::generate(),
                DocType::Murmure,
                crate::Existence::OnRelay,
            ),
        );
        state.handle_ws_message(forbidden(meta));
        assert_ne!(listed(&state), after_share, "a channel removal");
    }

    /// The answer to a segment close names whether the successor was created
    /// or fetched, as the relay said (the Murmure segments spec).
    #[test]
    fn a_closed_segment_reaches_logic_with_how_its_successor_came() {
        let (mut state, rx) = with_open_document();
        for (created, successor) in [
            (ambre_core::Creation::Created, ambre_bus::Successor::Created),
            (
                ambre_core::Creation::Existing,
                ambre_bus::Successor::Fetched,
            ),
        ] {
            state.handle_sync_event(SyncEvent::SegmentClosed {
                channel_id: Id::<ambre_core::id::Channel>::mint(),
                closing: Id::<ambre_core::id::Segment>::mint(),
                result: Ok(crate::http::ClosedSegmentResponse {
                    active: Id::<ambre_core::id::Segment>::mint(),
                    created,
                }),
            });
            assert!(matches!(
                rx.try_iter().collect::<Vec<_>>().as_slice(),
                [LogicEvent::SegmentClosed { successor: got, .. }] if *got == successor
            ));
        }
    }

    /// `SYNC-128`: a members answer that says a rotation is owed records the
    /// debt for a writer alone, and an answer that says none records none.
    #[test]
    fn a_members_answer_records_an_owed_rotation_for_a_writer_alone() {
        let answer = |document_id, rotation_owed| SyncEvent::DocumentMembersReady {
            id: None,
            asked: ambre_core::Issuer::<crate::generation::Listing>::default().issue(),
            document_id,
            word: crate::membership_log::SpaceWord::default(),
            rotation_owed,
            members: Vec::new(),
            signing_keys: Vec::new(),
            heads: Default::default(),
            writers: Default::default(),
            departed: None,
            revoked: None,
            root: Default::default(),
        };
        for (role, owed, records) in [
            (
                ambre_bus::Role::Editor,
                ambre_core::RotationDebt::Owed,
                true,
            ),
            (
                ambre_bus::Role::Viewer,
                ambre_core::RotationDebt::Owed,
                false,
            ),
            (
                ambre_bus::Role::Editor,
                ambre_core::RotationDebt::Settled,
                false,
            ),
        ] {
            let (mut state, _rx) = with_open_document();
            let id = *state.documents.keys().next().expect("one document");
            state.documents.get_mut(&id).expect("open").role = role;
            state.handle_sync_event(answer(id, owed));
            assert_eq!(
                state.documents[&id].rekey.owes(),
                records,
                "{role:?} {owed:?}"
            );
        }
    }

    /// `SRV-183`: a socket refusal reaches Logic with the status its code
    /// answers.
    #[test]
    fn a_socket_refusal_reaches_logic_with_its_status() {
        let (mut state, rx, _wire, id) = with_relay_document();
        state.handle_sync_event(SyncEvent::WsMessage(WsServerMsg::Error {
            document_id: Some(id),
            code: RefusalCode::PayloadTooLarge,
        }));
        let told: Vec<LogicEvent> = rx.try_iter().collect();
        assert!(matches!(
            told[..],
            [LogicEvent::ApiError {
                failure: ambre_bus::ApiFailure::Relay {
                    status,
                    ..
                },
                ..
            }] if status.get() == 413
        ));
    }
}
