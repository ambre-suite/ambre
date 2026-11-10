//! The transport of a call in the Sync worker (`ambre-spec-call-media.md section 3`,
//! signalling). It holds the relay's call socket, the socket of the SFU and the membership log
//! of the channel. It holds the switch timer, and the port the handoffs leave on.
//!
//! [`CallClient`] decides; this module carries its steps out. The worker joins one call at a
//! time, so it holds the keys of the current call alone (`CALL-004`). The membership log is
//! read first, and the call socket opens once it holds. A change of the members of the
//! channel reads it again, and the client heeds it within the call (`CALL-060`).

use std::rc::Rc;

use ambre_core::Validated;
use ambre_core::id::{Call, Document, Id};
use ambre_crypto::call::KeyHandoff;
use ambre_protocol::signaling::{CallClientMsg, CallServerMsg, SfuMsg, SfuSocket};
#[cfg(target_arch = "wasm32")]
use wasm_bindgen::JsCast;

use crate::call_client::{CallClient, CallEnd, CallTarget, Opening, Steps};
use crate::membership_log::Log;
use crate::{SyncEvent, SyncState};
use ambre_bus::{CallSignal, LogicEvent, PageDescription};

/// Where the handoffs of the sender keys leave the Sync worker for the call worker. It is
/// the port of a message channel the UI thread created, so no key crosses the Main instance
/// (`CALL-064`). A host test collects the handoffs.
#[derive(Clone)]
pub(crate) struct CallPort(Rc<dyn Fn(KeyHandoff)>);

impl CallPort {
    /// A port that hands each handoff to `hand`.
    #[cfg_attr(
        all(not(test), not(target_arch = "wasm32")),
        expect(dead_code, reason = "only the wasm transport and the tests read it")
    )]
    pub(crate) fn new(hand: impl Fn(KeyHandoff) + 'static) -> Self {
        Self(Rc::new(hand))
    }

    /// The port `port` the UI thread transferred with a join. Each handoff crosses as the
    /// bytes of [`KeyHandoff::to_port`], in a buffer the post moves rather than copies.
    #[cfg(target_arch = "wasm32")]
    pub(crate) fn over(port: web_sys::MessagePort) -> Self {
        Self::new(move |handoff| {
            handoff.to_port().with_plaintext(|bytes| {
                let array = js_sys::Uint8Array::new_with_length(
                    u32::try_from(bytes.len()).unwrap_or_default(),
                );
                array.copy_from(bytes);
                let buffer = array.buffer();
                if port
                    .post_message_with_transferable(&buffer, &js_sys::Array::of1(&buffer))
                    .is_err()
                {
                    tracing::warn!("Sync: a key did not reach the call worker");
                }
            });
        })
    }
}

/// The ports the join commands brought, in arrival order, until each command takes one. The
/// worker entry reads the port of a message before it queues the command of that message. It
/// shares this queue with the state of the worker.
#[derive(Clone, Default)]
pub(crate) struct CallPorts(Rc<std::cell::RefCell<std::collections::VecDeque<CallPort>>>);

impl CallPorts {
    /// Holds the port that came with a join command, for [`SyncState::handle_join_call`].
    #[cfg_attr(
        all(not(test), not(target_arch = "wasm32")),
        expect(dead_code, reason = "only the wasm transport and the tests read it")
    )]
    pub(crate) fn offer(&self, port: CallPort) {
        self.0.borrow_mut().push_back(port);
    }

    /// The oldest port a join brought, which its command takes.
    fn take(&self) -> Option<CallPort> {
        self.0.borrow_mut().pop_front()
    }
}

/// A join that waits for the `meta` document of its channel.
pub(crate) struct Joining {
    target: CallTarget,
    opening: Opening,
    port: CallPort,
}

/// What the host build records in the place of the two sockets.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Default)]
pub(crate) struct Sent {
    /// How many reads of the membership log the link asked for.
    pub(crate) log_reads: ambre_core::Count<LogReads>,
    pub(crate) relay: Vec<CallClientMsg>,
    pub(crate) sfu_open: Option<Validated<SfuSocket>>,
    pub(crate) sfu: Vec<SfuMsg>,
}

/// A number of reads of the membership log of a call, which the host build counts.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) enum LogReads {}

#[cfg(not(target_arch = "wasm32"))]
impl ambre_core::CountKind for LogReads {
    type Repr = u32;
    const NAME: &'static str = "LogReads";
}

/// One open socket of a call and the callbacks that feed the event queue.
#[cfg(target_arch = "wasm32")]
struct Socket {
    ws: web_sys::WebSocket,
    _callbacks: Vec<wasm_bindgen::closure::Closure<dyn FnMut(wasm_bindgen::JsValue)>>,
}

/// The call the worker is in.
pub(crate) struct CallLink {
    client: CallClient,
    /// The `meta` document of the channel, whose log admits each joiner.
    meta: Id<Document>,
    /// The membership log of the channel, `None` until its first read.
    log: Option<Log>,
    port: CallPort,
    /// The frames for the SFU, held until its socket opens.
    sfu_queue: Vec<SfuMsg>,
    /// The client sent its leave frame, and waits for the relay to say whether it was the
    /// last (`CALL-069`). The keys and the peer connection are gone already.
    leaving: Leaving,
    #[cfg(target_arch = "wasm32")]
    relay: Option<Socket>,
    #[cfg(target_arch = "wasm32")]
    sfu: Option<Socket>,
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) sent: Sent,
}

/// Whether the session waits for the answer to its leave.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Leaving {
    /// The session is in the call.
    No,
    /// The leave frame left: the relay answers, or closes, or the leave delay runs out.
    Sent,
}

/// How long a leave waits for the relay to answer before the socket closes. The relay
/// answers within a round trip. A relay that does not answer leaves the end to the time to
/// live of the call (the Murmure calls spec).
const LEAVE_WAIT: ambre_core::TimeSpan<ambre_core::Ms> = ambre_core::TimeSpan::new(5_000);

impl CallLink {
    /// The client of the call.
    #[cfg(test)]
    pub(crate) const fn client(&self) -> &CallClient {
        &self.client
    }
}

impl SyncState {
    /// A command of the bus joins `target` (`CALL-065`). It takes the port that came with
    /// it, then reads which document of the channel is its `meta`. A join while a call is
    /// open, or while another join waits, changes nothing.
    pub(crate) fn handle_join_call(&mut self, target: CallTarget, opening: Opening) {
        let Some(port) = self.call_ports.take() else {
            tracing::warn!("Sync: a call joined with no port to the call worker");
            return;
        };
        if self.call.is_some() || self.call_joining.is_some() {
            return;
        }
        self.call_joining = Some(Joining {
            target,
            opening,
            port,
        });
        let Some(tx) = self.event_tx.clone() else {
            return;
        };
        let (scheme, base) = (self.url_scheme, self.api_base.clone());
        let http = crate::sharing::GlooHttp {
            auth: self.auth_context(),
        };
        wasm_bindgen_futures::spawn_local(async move {
            let meta = crate::sharing::channel_meta(&http, target.channel, scheme, &base).await;
            let _ = tx.send(SyncEvent::CallChannel {
                call: target.call,
                meta,
            });
        });
    }

    /// The `meta` document of the channel of the waiting join was read: the join goes on, or
    /// ends when the channel answered none.
    pub(crate) fn call_channel(&mut self, call: Id<Call>, meta: Option<Id<Document>>) {
        let Some(joining) = self
            .call_joining
            .take_if(|joining| joining.target.call == call)
        else {
            return;
        };
        match meta {
            Some(meta) => self.join_call(joining.target, meta, joining.port, joining.opening),
            None => {
                tracing::warn!("Sync: the channel of the call holds no meta document");
                self.send_to_logic(LogicEvent::Call(CallSignal::Ended));
            }
        }
    }

    /// A session description of the peer connection of the UI thread, for the SFU. It waits
    /// with the other frames until the socket of the SFU opens.
    pub(crate) fn handle_call_description(&mut self, description: PageDescription) {
        let Some(link) = self.call.as_mut() else {
            return;
        };
        link.sfu_queue.push(match description {
            PageDescription::Offer(sdp) => SfuMsg::Offer(sdp),
            PageDescription::Answer(sdp) => SfuMsg::Answer(sdp),
        });
        self.call_sfu_flush();
    }

    /// Joins `target`, whose channel's `meta` document is `meta`. The worker is in one call at
    /// a time: a join while a call is open changes nothing. The handoffs leave on `port`.
    /// A command of the bus that joins a call lands here.
    pub(crate) fn join_call(
        &mut self,
        target: CallTarget,
        meta: Id<Document>,
        port: CallPort,
        opening: Opening,
    ) {
        let Some(member) = self.user_id else {
            tracing::warn!("Sync: a call joined before the session is ready");
            return;
        };
        if self.call.is_some() {
            return;
        }
        self.call = Some(CallLink {
            client: CallClient::opening(target, member, opening),
            meta,
            log: None,
            port,
            sfu_queue: Vec::new(),
            leaving: Leaving::No,
            #[cfg(target_arch = "wasm32")]
            relay: None,
            #[cfg(target_arch = "wasm32")]
            sfu: None,
            #[cfg(not(target_arch = "wasm32"))]
            sent: Sent::default(),
        });
        self.read_call_log(meta);
    }

    /// Leaves the call, as a command of the bus asks. A seated session sends its leave frame
    /// and drops its keys and its peer connection at once. Its call socket stays open until
    /// the relay answers whether the seat was the last (`CALL-069`). Any other session ends
    /// its call at once.
    pub(crate) fn leave_call(&mut self) {
        let seated = self
            .call
            .as_ref()
            .is_some_and(|link| link.leaving == Leaving::No && link.client.is_seated());
        if !seated {
            self.end_call();
            return;
        }
        self.send_call_relay(&CallClientMsg::Leave);
        let Some(link) = self.call.as_mut() else {
            return;
        };
        link.leaving = Leaving::Sent;
        // The call worker drops every key of the call (`CALL-029`), and the peer connection
        // of the UI thread closes.
        (link.port.0)(KeyHandoff::End);
        #[cfg(target_arch = "wasm32")]
        if let Some(socket) = link.sfu.take() {
            socket.ws.set_onmessage(None);
            socket.ws.set_onclose(None);
            socket.ws.set_onopen(None);
            let _ = socket.ws.close();
        }
        self.send_to_logic(LogicEvent::Call(CallSignal::Ended));
        self.arm_event_timer(SyncEvent::CallLeaveDue, LEAVE_WAIT);
    }

    /// The relay answered the leave of this session. The leave it counts as the last ends
    /// the call, and the Logic thread writes the end into its call message (the Murmure calls spec).
    fn departed(&mut self, departure: ambre_protocol::signaling::Departure) {
        let Some(target) = self
            .call
            .as_ref()
            .filter(|link| link.leaving == Leaving::Sent)
            .map(|link| link.client.target())
        else {
            return;
        };
        if departure == ambre_protocol::signaling::Departure::Last {
            self.send_to_logic(LogicEvent::CallEnded {
                call: target.call,
                channel: target.channel,
                at: self.authored_now(),
            });
        }
        self.end_call();
    }

    /// The leave delay ran out with no answer of the relay: the socket closes, and the time
    /// to live of the call ends it.
    pub(crate) fn call_leave_due(&mut self) {
        if self
            .call
            .as_ref()
            .is_some_and(|link| link.leaving == Leaving::Sent)
        {
            self.end_call();
        }
    }

    /// Ends the call for this session: both sockets close, and the relay reads the close as
    /// the leave. The sockets close here, and no destructor runs on the way out
    /// (`ARCH-015`).
    fn end_call(&mut self) {
        if let Some(joining) = self.call_joining.take() {
            (joining.port.0)(KeyHandoff::End);
        }
        let Some(link) = self.call.take() else {
            return;
        };
        // A leave dropped the keys and closed the peer connection already.
        if link.leaving == Leaving::No {
            // The call worker drops every key of the call (`CALL-029`), and the peer
            // connection of the UI thread closes.
            (link.port.0)(KeyHandoff::End);
            self.send_to_logic(LogicEvent::Call(CallSignal::Ended));
        }
        #[cfg(target_arch = "wasm32")]
        for socket in [link.relay, link.sfu].into_iter().flatten() {
            socket.ws.set_onmessage(None);
            socket.ws.set_onclose(None);
            socket.ws.set_onopen(None);
            let _ = socket.ws.close();
        }
        #[cfg(not(target_arch = "wasm32"))]
        let _ = link;
    }

    /// Asks the relay for the membership log of `meta`, which arrives as
    /// [`SyncEvent::CallLog`]. A document that is not the channel of the call is ignored.
    pub(crate) fn read_call_log(&mut self, meta: Id<Document>) {
        if self.call.as_ref().is_none_or(|link| link.meta != meta) {
            return;
        }
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(link) = self.call.as_mut() {
            link.sent.log_reads = link.sent.log_reads.incremented();
        }
        #[cfg(target_arch = "wasm32")]
        {
            let (Some(tx), Some(exchange_key)) = (self.event_tx.clone(), self.exchange_key.clone())
            else {
                return;
            };
            let params = crate::sharing::CallLogParams {
                document_id: meta,
                url_scheme: self.url_scheme,
                api_base: self.api_base.clone(),
                exchange_key,
                rule: self.log_rule(meta),
            };
            let http = crate::sharing::GlooHttp {
                auth: self.auth_context(),
            };
            wasm_bindgen_futures::spawn_local(async move {
                crate::sharing::call_log_task(&http, params, &tx).await;
            });
        }
    }

    /// The membership log of `meta` was read. The first read opens the call socket; a later
    /// one is heeded within the call.
    pub(crate) fn call_log(&mut self, meta: Id<Document>, log: Log) {
        let now = (self.clock)();
        let Some(link) = self.call.as_mut().filter(|link| link.meta == meta) else {
            return;
        };
        let first = link.log.is_none();
        let steps = link.client.heed(&log, now);
        link.log = Some(log);
        if first {
            self.open_call_socket();
        } else {
            self.carry_call(steps);
        }
    }

    /// The call socket opened: its first frame joins.
    pub(crate) fn call_relay_opened(&mut self) {
        let Some(token) = self
            .tokens
            .borrow()
            .as_ref()
            .map(|t| t.access_token().clone())
        else {
            return;
        };
        let Some(link) = self.call.as_ref() else {
            return;
        };
        let frame = link.client.join_frame(token);
        self.send_call_relay(&frame);
    }

    /// A frame of the call socket.
    pub(crate) fn call_relay(&mut self, msg: CallServerMsg) {
        if let CallServerMsg::Departed(departure) = msg {
            self.departed(departure);
            return;
        }
        if self
            .call
            .as_ref()
            .is_some_and(|link| link.leaving == Leaving::Sent)
        {
            return;
        }
        let now = (self.clock)();
        let (Some(identity), Some(link)) = (self.signing_key.as_ref(), self.call.as_mut()) else {
            return;
        };
        let Some(log) = link.log.as_ref() else {
            return;
        };
        let steps = link.client.on_relay(msg, identity, log, now);
        self.carry_call(steps);
    }

    /// The instant of the clock of the worker, as a message stamps its time.
    fn authored_now(&self) -> ambre_core::Instant<ambre_core::Authored> {
        ambre_core::Instant::<ambre_core::Authored>::after_epoch((self.clock)().since_epoch())
    }

    /// The call socket closed: the call ended for this session.
    pub(crate) fn call_relay_closed(&mut self) {
        self.end_call();
    }

    /// The socket of the SFU opened: the frames held for it leave.
    pub(crate) fn call_sfu_opened(&mut self) {
        let queued = self
            .call
            .as_mut()
            .map(|link| std::mem::take(&mut link.sfu_queue))
            .unwrap_or_default();
        for msg in queued {
            self.send_call_sfu(msg);
        }
    }

    /// A message of the socket of the SFU.
    pub(crate) fn call_sfu(&mut self, msg: SfuMsg) {
        if let Some(link) = self.call.as_mut() {
            let steps = link.client.on_sfu(msg);
            self.carry_call(steps);
        }
    }

    /// The switch timer fired.
    pub(crate) fn call_tick(&mut self) {
        let now = (self.clock)();
        if let Some(link) = self.call.as_mut() {
            let steps = link.client.tick(now);
            self.carry_call(steps);
        }
    }

    /// Carries out the steps of the client, in their order.
    fn carry_call(&mut self, steps: Steps) {
        for frame in &steps.relay {
            self.send_call_relay(frame);
        }
        if let Some(address) = steps.open_sfu {
            self.open_call_sfu(address);
        }
        for signal in steps.page {
            self.send_to_logic(LogicEvent::Call(signal));
        }
        if let Some(target) = steps.started {
            self.send_to_logic(LogicEvent::CallStarted {
                call: target.call,
                channel: target.channel,
                at: self.authored_now(),
            });
        }
        if let Some(link) = self.call.as_mut() {
            link.sfu_queue.extend(steps.sfu);
            for handoff in steps.handoffs {
                (link.port.0)(handoff);
            }
        }
        self.call_sfu_flush();
        if let Some(delay) = steps.tick_in {
            self.arm_event_timer(SyncEvent::CallTick, delay);
        }
        if let Some(end) = steps.ended {
            tracing::info!("Sync: the call ended: {end:?}");
            // A join refused as ended tells the reader of the call message at once
            // (the Murmure calls spec).
            if end == CallEnd::Refused(ambre_protocol::CallRefusal::Ended)
                && let Some(target) = self.call.as_ref().map(|link| link.client.target())
            {
                self.send_to_logic(LogicEvent::CallRoster {
                    call: target.call,
                    channel: target.channel,
                    members: ambre_bus::CallMembers::Gone,
                });
            }
            self.end_call();
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl SyncState {
    fn open_call_socket(&mut self) {
        self.call_relay_opened();
    }

    fn send_call_relay(&mut self, frame: &CallClientMsg) {
        if let Some(link) = self.call.as_mut() {
            link.sent.relay.push(frame.clone());
        }
    }

    fn open_call_sfu(&mut self, address: Validated<SfuSocket>) {
        if let Some(link) = self.call.as_mut() {
            link.sent.sfu_open = Some(address);
        }
    }

    /// The host build opens the socket of the SFU at once.
    fn call_sfu_flush(&mut self) {
        if self
            .call
            .as_ref()
            .is_some_and(|link| link.sent.sfu_open.is_some())
        {
            self.call_sfu_opened();
        }
    }

    fn send_call_sfu(&mut self, msg: SfuMsg) {
        if let Some(link) = self.call.as_mut() {
            link.sent.sfu.push(msg);
        }
    }
}

#[cfg(target_arch = "wasm32")]
impl SyncState {
    /// Opens the call socket of the call, once its log holds.
    fn open_call_socket(&mut self) {
        let Some(target) = self.call.as_ref().map(|link| link.client.target()) else {
            return;
        };
        let url = crate::http::Url::<crate::http::CallSocket>::of(
            self.url_scheme,
            &self.api_base,
            target.call,
        );
        let Ok(ws) = web_sys::WebSocket::new(url.as_str()) else {
            tracing::error!("Sync: the call socket did not open");
            return;
        };
        ws.set_binary_type(web_sys::BinaryType::Arraybuffer);
        let socket = self.call_socket(
            ws,
            |data| {
                let buffer: js_sys::ArrayBuffer = data.dyn_into().ok()?;
                let bytes = js_sys::Uint8Array::new(&buffer);
                postcard::from_bytes::<CallServerMsg>(&bytes.to_vec())
                    .ok()
                    .map(SyncEvent::CallRelay)
            },
            SyncEvent::CallRelayOpened,
            SyncEvent::CallRelayClosed,
        );
        if let (Some(link), Some(socket)) = (self.call.as_mut(), socket) {
            link.relay = Some(socket);
        }
    }

    /// Opens the socket of the SFU at `address`.
    fn open_call_sfu(&mut self, address: Validated<SfuSocket>) {
        let Ok(ws) = web_sys::WebSocket::new(address.as_str()) else {
            tracing::error!("Sync: the socket of the SFU did not open");
            return;
        };
        let socket = self.call_socket(
            ws,
            |data| {
                let text = data.as_string()?;
                serde_json::from_str::<SfuMsg>(&text)
                    .ok()
                    .map(SyncEvent::CallSfu)
            },
            SyncEvent::CallSfuOpened,
            SyncEvent::CallRelayClosed,
        );
        if let (Some(link), Some(socket)) = (self.call.as_mut(), socket) {
            link.sfu = Some(socket);
        }
    }

    /// The wasm build sends the frames for the SFU from its `onopen` alone.
    fn call_sfu_flush(&mut self) {
        let open = self
            .call
            .as_ref()
            .and_then(|link| link.sfu.as_ref())
            .is_some_and(|socket| socket.ws.ready_state() == web_sys::WebSocket::OPEN);
        if open {
            self.call_sfu_opened();
        }
    }

    /// The socket `ws`, whose messages `read` turns into events, and whose open and close
    /// raise `opened` and `closed`.
    fn call_socket(
        &self,
        ws: web_sys::WebSocket,
        read: fn(wasm_bindgen::JsValue) -> Option<SyncEvent>,
        opened: SyncEvent,
        closed: SyncEvent,
    ) -> Option<Socket> {
        use wasm_bindgen::JsCast;
        use wasm_bindgen::closure::Closure;

        let tx = self.event_tx.clone()?;
        let on_message = {
            let tx = tx.clone();
            Closure::wrap(Box::new(move |e: wasm_bindgen::JsValue| {
                if let Some(event) = e
                    .dyn_into::<web_sys::MessageEvent>()
                    .ok()
                    .and_then(|m| read(m.data()))
                {
                    let _ = tx.send(event);
                }
            }) as Box<dyn FnMut(wasm_bindgen::JsValue)>)
        };
        let opened = std::cell::Cell::new(Some(opened));
        let on_open = {
            let tx = tx.clone();
            Closure::wrap(Box::new(move |_: wasm_bindgen::JsValue| {
                if let Some(event) = opened.take() {
                    let _ = tx.send(event);
                }
            }) as Box<dyn FnMut(wasm_bindgen::JsValue)>)
        };
        let closed = std::cell::Cell::new(Some(closed));
        let on_close = Closure::wrap(Box::new(move |_: wasm_bindgen::JsValue| {
            if let Some(event) = closed.take() {
                let _ = tx.send(event);
            }
        }) as Box<dyn FnMut(wasm_bindgen::JsValue)>);
        ws.set_onmessage(Some(on_message.as_ref().unchecked_ref()));
        ws.set_onopen(Some(on_open.as_ref().unchecked_ref()));
        ws.set_onclose(Some(on_close.as_ref().unchecked_ref()));
        Some(Socket {
            ws,
            _callbacks: vec![on_message, on_open, on_close],
        })
    }

    fn send_call_relay(&mut self, frame: &CallClientMsg) {
        let Some(socket) = self.call.as_ref().and_then(|link| link.relay.as_ref()) else {
            return;
        };
        // The bytes leave in a buffer of their own: a socket refuses a view of the shared
        // memory of the worker.
        if let Ok(bytes) = postcard::to_allocvec(frame) {
            let array = js_sys::Uint8Array::from(bytes.as_slice());
            if socket.ws.send_with_array_buffer(&array.buffer()).is_err() {
                tracing::warn!("Sync: a frame of the call socket was not sent");
            }
        }
    }

    fn send_call_sfu(&mut self, msg: SfuMsg) {
        let Some(socket) = self.call.as_ref().and_then(|link| link.sfu.as_ref()) else {
            return;
        };
        if let Ok(text) = serde_json::to_string(&msg) {
            let _ = socket.ws.send_with_str(&text);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use ambre_core::id::{Call, Channel, Document, Id};
    use ambre_core::{Count, Role, Sec, TimeSpan, Validated};
    use ambre_protocol::MembershipAct;
    use ambre_protocol::signaling::{CallClientMsg, CallServerMsg, SfuMsg};
    use ambre_protocol::ws::WsServerMsg;

    use super::{CallPort, Rc};
    use crate::call_client::{CallTarget, Opening};
    use crate::membership_log::tests::{DOC, Person, append};
    use crate::membership_log::{Log, digest, no_self_join};
    use crate::{LogicSink, SyncState};
    use ambre_bus::{LogicEvent, SyncCommand};
    use ambre_crypto::call::KeyHandoff;
    use ambre_protocol::signaling::Departure;

    const TARGET: CallTarget = CallTarget {
        call: Id::<Call>::from_bytes([31; 16]),
        channel: Id::<Channel>::from_bytes([32; 16]),
    };
    const META: Id<Document> = DOC;

    /// A signed-in worker of `person`, and the handoffs its port collected.
    fn worker(person: &Person) -> (SyncState, Rc<RefCell<Vec<KeyHandoff>>>, CallPort) {
        let (state, handed, port, _) = worker_heard(person);
        (state, handed, port)
    }

    /// As [`worker`], with the events the worker sends the UI thread.
    fn worker_heard(
        person: &Person,
    ) -> (
        SyncState,
        Rc<RefCell<Vec<KeyHandoff>>>,
        CallPort,
        ambre_bus::conduit::Receiver<LogicEvent>,
    ) {
        let (sink, rx) = LogicSink::channel();
        let mut state = SyncState::new(
            sink,
            crate::http::Url::<crate::RelayHost>::new("relay.example".to_owned()),
        );
        state.user_id = Some(person.id);
        state.signing_key = Some(person.key.clone());
        *state.tokens.borrow_mut() = Some(crate::http::TokenPair::new(
            ambre_core::Credential::new("access".to_owned()),
            ambre_core::Credential::new("refresh".to_owned()),
            TimeSpan::<Sec>::new(900),
        ));
        let handed = Rc::new(RefCell::new(Vec::new()));
        let port = {
            let handed = handed.clone();
            CallPort::new(move |handoff| handed.borrow_mut().push(handoff))
        };
        (state, handed, port, rx)
    }

    fn log_of(person: &Person) -> Log {
        let owner = Person::new(90);
        let mut entries = Vec::new();
        append(
            &mut entries,
            &owner,
            &owner,
            MembershipAct::Create,
            Role::Owner,
        );
        append(
            &mut entries,
            &owner,
            person,
            MembershipAct::Invite,
            Role::Editor,
        );
        Log::walk(
            DOC,
            entries.first().and_then(digest),
            &entries,
            no_self_join,
        )
    }

    /// `CALL-062`: a join reads the log of the channel first, then opens the call socket with
    /// the token. The admission opens the socket of the SFU with the admission first. The join
    /// is announced, and the key of the session leaves on the port. A second join while
    /// in a call changes nothing, and a lost call ends.
    #[test]
    fn a_join_reads_the_log_then_opens_the_sockets_and_hands_the_key_to_the_port() {
        let alice = Person::new(1);
        let (mut state, handed, port) = worker(&alice);
        state.join_call(TARGET, META, port.clone(), Opening::Join);
        let link = state.call.as_ref().expect("a call");
        assert_eq!(link.client().target(), TARGET);
        assert!(link.sent.relay.is_empty(), "the socket waits for the log");
        assert_eq!(link.sent.log_reads, Count::new(1));

        state.call_log(META, log_of(&alice));
        let link = state.call.as_ref().expect("a call");
        assert!(matches!(
            link.sent.relay.first(),
            Some(CallClientMsg::Join { channel, .. }) if *channel == TARGET.channel
        ));

        let sfu = Validated::parse("wss://sfu.example/call").expect("an address");
        state.call_relay(CallServerMsg::Admitted {
            admission: ambre_core::Credential::new("admission".to_owned()),
            sfu: Some(sfu.clone()),
            sender: ambre_core::Seq::new(3),
            roster: Vec::new(),
        });
        state.call_sfu(SfuMsg::Ice(Vec::new()));
        let link = state.call.as_ref().expect("a call");
        assert!(matches!(
            link.sent.relay.get(1),
            Some(CallClientMsg::Announce(_))
        ));
        assert_eq!(link.sent.sfu_open, Some(sfu));
        assert!(matches!(link.sent.sfu.first(), Some(SfuMsg::Admission(_))));
        assert!(matches!(
            handed.borrow().as_slice(),
            [KeyHandoff::Seal { .. }]
        ));

        state.join_call(
            CallTarget {
                call: Id::<Call>::from_bytes([33; 16]),
                ..TARGET
            },
            META,
            port,
            Opening::Join,
        );
        assert_eq!(
            state.call.as_ref().map(|l| l.client().target()),
            Some(TARGET)
        );
        state.call_relay(CallServerMsg::Lost);
        assert!(state.call.is_none(), "a lost call ends");
    }

    /// `CALL-060`: a change of the members of the channel of the call, or a rotation of its
    /// key, reads the log again. A change of another document reads nothing.
    #[test]
    fn a_change_of_the_members_of_the_channel_reads_the_log_again() {
        let alice = Person::new(1);
        let (mut state, _, port) = worker(&alice);
        state.join_call(TARGET, META, port, Opening::Join);
        state.handle_ws_message(WsServerMsg::MembersChanged {
            document_id: Id::<Document>::from_bytes([34; 16]),
        });
        state.handle_ws_message(WsServerMsg::MembersChanged { document_id: META });
        state.handle_ws_message(WsServerMsg::KeyRotationRequired { document_id: META });
        let reads = state.call.as_ref().map(|l| l.sent.log_reads);
        assert_eq!(reads, Some(Count::new(3)));
    }

    /// The call socket of a call is its own address on the relay, apart from the socket of
    /// the document plane (`SRV-049`).
    #[test]
    fn the_call_socket_is_an_address_of_its_own() {
        let url = crate::http::Url::<crate::http::CallSocket>::of(
            crate::http::UrlScheme::Https,
            &crate::http::Url::<crate::RelayHost>::new("relay.example".to_owned()),
            TARGET.call,
        );
        assert_eq!(
            url.as_str(),
            format!("wss://relay.example/api/calls/{}/socket", TARGET.call)
        );
    }

    /// `CALL-065`, `CALL-029`: a command of the bus joins the call and the channel it names,
    /// once the relay names the `meta` of that channel. The keys leave on the port that came
    /// with the command. The SFU's TURN servers reach the UI thread, and its description
    /// waits for the SFU. A leave hands the call worker the end of the call, and the UI
    /// thread hears it.
    #[test]
    fn a_join_command_seats_the_call_it_names_and_hands_the_keys_on_its_port() {
        let alice = Person::new(1);
        let (mut state, handed, port, rx) = worker_heard(&alice);
        state.handle_command(SyncCommand::JoinCall {
            call: TARGET.call,
            channel: TARGET.channel,
        });
        assert!(
            state.call_joining.is_none(),
            "a join with no port waits for nothing"
        );
        state.call_ports.offer(port);
        state.handle_command(SyncCommand::JoinCall {
            call: TARGET.call,
            channel: TARGET.channel,
        });
        assert!(
            state.call.is_none(),
            "the join waits for the meta of its channel"
        );
        state.call_channel(Id::<Call>::from_bytes([33; 16]), Some(META));
        assert!(
            state.call.is_none(),
            "the meta of another call joins nothing"
        );
        state.call_channel(TARGET.call, Some(META));
        assert_eq!(
            state.call.as_ref().map(|link| link.client().target()),
            Some(TARGET)
        );
        state.call_log(META, log_of(&alice));
        state.call_relay(CallServerMsg::Admitted {
            admission: ambre_core::Credential::new("admission".to_owned()),
            sfu: Some(Validated::parse("wss://sfu.example/call").expect("an address")),
            sender: ambre_core::Seq::new(3),
            roster: Vec::new(),
        });
        assert!(matches!(
            handed.borrow().as_slice(),
            [KeyHandoff::Seal { sender, .. }] if *sender == ambre_core::Seq::new(3)
        ));
        state.call_sfu(SfuMsg::Ice(Vec::new()));
        assert!(matches!(
            rx.try_recv(),
            Ok(LogicEvent::Call(ambre_bus::CallSignal::Ice(servers))) if servers.is_empty()
        ));
        state.handle_command(SyncCommand::CallDescription(
            ambre_bus::PageDescription::Offer(Validated::parse("v=0".to_owned()).expect("sdp")),
        ));
        let sent = state.call.as_ref().map(|l| l.sent.sfu.len());
        assert_eq!(sent, Some(2), "the admission, then the offer");
        state.handle_command(SyncCommand::LeaveCall);
        assert!(matches!(handed.borrow().last(), Some(KeyHandoff::End)));
        assert!(matches!(
            rx.try_recv(),
            Ok(LogicEvent::Call(ambre_bus::CallSignal::Ended))
        ));
        assert!(matches!(
            state.call.as_ref().and_then(|l| l.sent.relay.last()),
            Some(CallClientMsg::Leave)
        ));
        // Another seat holds the call: the leave writes no end.
        state.call_relay(CallServerMsg::Departed(Departure::Others));
        assert!(state.call.is_none());
        assert!(
            rx.try_recv().is_err(),
            "no end for a leave that was not the last"
        );
    }

    /// Seats `state` in its call, admitted under the sender index 0.
    fn admit(state: &mut SyncState) {
        state.call_relay(CallServerMsg::Admitted {
            admission: ambre_core::Credential::new("admission".to_owned()),
            sfu: None,
            sender: ambre_core::Seq::new(0),
            roster: Vec::new(),
        });
    }

    /// The Sync half: a start sends the start frame,
    /// and the Logic thread hears the start once the relay seats it, never before. A leave the
    /// relay counts as the last tells the Logic thread to write the end. A join refused as
    /// ended tells the reader that the relay holds no state of the call.
    #[test]
    fn a_start_is_heard_once_seated_and_the_last_leave_asks_for_the_end() {
        let alice = Person::new(1);
        let (mut state, _, port, rx) = worker_heard(&alice);
        state.join_call(TARGET, META, port.clone(), Opening::Start);
        state.call_log(META, log_of(&alice));
        assert!(matches!(
            state.call.as_ref().and_then(|l| l.sent.relay.first()),
            Some(CallClientMsg::Start { channel, .. }) if *channel == TARGET.channel
        ));
        assert!(rx.try_recv().is_err(), "no call message before the seat");
        admit(&mut state);
        assert!(matches!(
            rx.try_recv(),
            Ok(LogicEvent::CallStarted { call, channel, .. })
                if call == TARGET.call && channel == TARGET.channel
        ));
        state.leave_call();
        assert!(matches!(
            rx.try_recv(),
            Ok(LogicEvent::Call(ambre_bus::CallSignal::Ended))
        ));
        state.call_relay(CallServerMsg::Departed(Departure::Last));
        assert!(matches!(
            rx.try_recv(),
            Ok(LogicEvent::CallEnded { call, channel, .. })
                if call == TARGET.call && channel == TARGET.channel
        ));
        assert!(state.call.is_none());

        state.join_call(TARGET, META, port, Opening::Join);
        state.call_log(META, log_of(&alice));
        state.call_relay(CallServerMsg::Refused(ambre_protocol::CallRefusal::Ended));
        assert!(matches!(
            rx.try_recv(),
            Ok(LogicEvent::CallRoster {
                members: ambre_bus::CallMembers::Gone,
                ..
            })
        ));
        assert!(state.call.is_none());
    }

    /// A leave the relay never answers ends the call at the leave delay and
    /// writes no end. A join the relay never seated sends no leave frame.
    #[test]
    fn a_leave_with_no_answer_ends_the_call_and_writes_no_end() {
        let alice = Person::new(1);
        let (mut state, _, port, rx) = worker_heard(&alice);
        state.join_call(TARGET, META, port.clone(), Opening::Start);
        state.call_log(META, log_of(&alice));
        state.leave_call();
        assert!(state.call.is_none(), "an unseated session leaves at once");
        while rx.try_recv().is_ok() {}

        state.join_call(TARGET, META, port, Opening::Start);
        state.call_log(META, log_of(&alice));
        admit(&mut state);
        state.leave_call();
        state.call_leave_due();
        assert!(state.call.is_none());
        let heard: Vec<_> = std::iter::from_fn(|| rx.try_recv().ok()).collect();
        assert!(
            !heard
                .iter()
                .any(|e| matches!(e, LogicEvent::CallEnded { .. })),
            "{heard:?}"
        );
    }

    /// Who is in a call reaches the Logic thread once the log of the channel checks it. A
    /// roster with no log names no one. A member the log does not hold drops the log, so the
    /// next ask reads it again.
    #[test]
    fn who_is_in_a_call_reaches_the_logic_thread_once_the_log_checks_it() {
        use ambre_protocol::signaling::CallRoster;
        let alice = Person::new(1);
        let (mut state, _, _, rx) = worker_heard(&alice);
        state.handle_command(SyncCommand::AskCallRoster {
            call: TARGET.call,
            channel: TARGET.channel,
            meta: META,
        });
        assert_eq!(state.roster_asks, [TARGET.call]);
        state.call_roster_read(
            TARGET.call,
            TARGET.channel,
            META,
            Some(CallRoster::Live(Vec::new())),
            None,
        );
        assert!(rx.try_recv().is_err(), "no log, no one named");
        state.call_roster_read(
            TARGET.call,
            TARGET.channel,
            META,
            Some(CallRoster::Live(Vec::new())),
            Some(log_of(&alice)),
        );
        assert!(matches!(
            rx.try_recv(),
            Ok(LogicEvent::CallRoster { members: ambre_bus::CallMembers::Live(m), .. }) if m.is_empty()
        ));
        state.call_roster_read(
            TARGET.call,
            TARGET.channel,
            META,
            Some(CallRoster::Gone),
            None,
        );
        assert!(matches!(
            rx.try_recv(),
            Ok(LogicEvent::CallRoster {
                members: ambre_bus::CallMembers::Gone,
                ..
            })
        ));
    }
}
