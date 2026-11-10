//! The client of a call on the UI thread (`ambre-spec-call-media.md section 3`, the client of
//! a call).
//!
//! The UI thread holds what only a page holds: the microphone and the peer connection to the
//! SFU (`ARCH-005`). A join spawns the call worker and gives it, and the Sync worker, the two
//! ends of one message channel. The Sync worker hands the sender keys on it, so no key enters
//! the Main instance (`CALL-064`). Each sender and each receiver of the peer connection takes
//! a transform that runs in the call worker. A sender takes it before its track, so no chunk
//! leaves in the clear (`CALL-001`, `CALL-011`).
//!
//! The Sync worker holds the socket of the SFU. So the session descriptions and the TURN
//! servers cross the worker hop, and this module reads them on arrival. The peer connection
//! opens no data channel (`CALL-010`), and the SFU is its one peer (`CALL-008`, `CALL-009`).

use std::cell::RefCell;
use std::rc::Rc;

use ambre_bus::{CallSignal, LogicEvent, PageDescription, SyncCommand};
use ambre_core::id::{Call, Channel, Id};
use ambre_core::{CallWorkerNote, Payload, TransformSide, Validated};
use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;

use crate::platform::{post_to_worker_with_port, spawn_call_worker};

/// The call the page is in.
struct Session {
    /// The call worker, which holds the keys of this call alone.
    worker: web_sys::Worker,
    /// The signals of the Sync worker, in arrival order, for the task that drives the call.
    signals: ambre_bus::conduit::UnboundedSender<CallSignal>,
}

thread_local! {
    /// The Sync worker, which the join and the session descriptions reach.
    static SYNC: RefCell<Option<web_sys::Worker>> = const { RefCell::new(None) };
    /// The call of the page, one at a time.
    static SESSION: RefCell<Option<Session>> = const { RefCell::new(None) };
}

/// Gives the call client the Sync worker the page spawned.
pub(crate) fn install(sync: &web_sys::Worker) {
    SYNC.with(|slot| *slot.borrow_mut() = Some(sync.clone()));
}

/// Posts `cmd` to the Sync worker, with `port` in the transfer list when one is given.
fn to_sync(cmd: &SyncCommand, port: Option<&web_sys::MessagePort>) {
    let Ok(bytes) = postcard::to_allocvec(cmd) else {
        tracing::error!("[Call] failed to encode {cmd:?}");
        return;
    };
    SYNC.with(|slot| {
        if let Some(sync) = slot.borrow().as_ref() {
            post_to_worker_with_port(sync, &Payload::new(bytes), port);
        }
    });
}

/// Takes the event `event` of the Sync worker when it is a signal of a call, and answers
/// whether it took it. A call signal never reaches the Logic thread.
pub(crate) fn heard(event: LogicEvent) -> Option<LogicEvent> {
    let LogicEvent::Call(signal) = event else {
        return Some(event);
    };
    SESSION.with(|slot| {
        if let Some(session) = slot.borrow().as_ref() {
            let _ = session.signals.send(signal);
        }
    });
    None
}

/// Joins the call `call` of the channel `channel`. A page in a call joins no second one, and
/// an id that does not parse joins nothing. The call message of a channel calls it, and
/// a belt calls it from the page with no gesture (`CALL-065`).
#[wasm_bindgen]
pub fn join_call(call: &str, channel: &str) {
    let (Ok(call), Ok(channel)) = (
        Id::<Call>::parse_str(call),
        Id::<Channel>::parse_str(channel),
    ) else {
        tracing::warn!("[Call] a join names no call or no channel");
        return;
    };
    if SESSION.with(|slot| slot.borrow().is_some()) {
        return;
    }
    let worker = spawn_call_worker();
    let (signals, rx) = ambre_bus::conduit::unbounded::<CallSignal>();
    let on_note = {
        let worker = worker.clone();
        Closure::<dyn FnMut(web_sys::MessageEvent)>::new(move |e: web_sys::MessageEvent| {
            let note = e
                .data()
                .as_f64()
                .and_then(|n| u8::try_from(n as u32).ok())
                .and_then(CallWorkerNote::from_byte);
            match note {
                Some(CallWorkerNote::Ready) => hand_ports(&worker, call, channel),
                // The keys are gone with the call: the instance goes too.
                Some(CallWorkerNote::Erased) => worker.terminate(),
                Some(CallWorkerNote::Oversize) => {
                    tracing::warn!("[Call] the encoder handed a frame past the chunk size");
                }
                None => {}
            }
        })
    };
    worker.set_onmessage(Some(on_note.as_ref().unchecked_ref()));
    // The worker lives for the call, and its note handler with it.
    on_note.forget();
    SESSION.with(|slot| {
        *slot.borrow_mut() = Some(Session {
            worker: worker.clone(),
            signals,
        });
    });
    wasm_bindgen_futures::spawn_local(drive(worker, rx));
}

/// Leaves the call of the page. The Sync worker closes both sockets and hands the call
/// worker the end of the call, which drops every key of it (`CALL-029`).
#[wasm_bindgen]
pub fn leave_call() {
    to_sync(&SyncCommand::LeaveCall, None);
}

/// Leaves the channel `channel`, when the id parses. The members who stay
/// read their log again, and a call of the channel switches its keys away from this member
/// (`CALL-060`). The Logic thread hears the answer of the relay. The control of a channel
/// that leaves it lands with the members of a channel; a belt calls this from the page.
#[wasm_bindgen]
pub fn leave_channel(channel: &str) {
    if let Ok(channel_id) = Id::<Channel>::parse_str(channel) {
        to_sync(&SyncCommand::LeaveChannel { channel_id }, None);
    }
}

/// Gives the two ends of one message channel to the call worker and the Sync worker, with
/// the join. The keys then cross between the two workers alone (`CALL-064`).
fn hand_ports(worker: &web_sys::Worker, call: Id<Call>, channel: Id<Channel>) {
    let Ok(channel_pair) = web_sys::MessageChannel::new() else {
        tracing::error!("[Call] no message channel for the keys of the call");
        return;
    };
    let to_worker = channel_pair.port2();
    if worker
        .post_message_with_transfer(&JsValue::NULL, &js_sys::Array::of1(&to_worker))
        .is_err()
    {
        tracing::error!("[Call] the port did not reach the call worker");
        return;
    }
    to_sync(
        &SyncCommand::JoinCall { call, channel },
        Some(&channel_pair.port1()),
    );
}

/// A transform of the call worker, for one side.
fn transform(worker: &web_sys::Worker, side: TransformSide) -> Option<JsValue> {
    let options = js_sys::Object::new();
    js_sys::Reflect::set(&options, &"side".into(), &side.as_str().into()).ok()?;
    let class = js_sys::Reflect::get(&js_sys::global(), &"RTCRtpScriptTransform".into()).ok()?;
    let class: js_sys::Function = class.dyn_into().ok()?;
    js_sys::Reflect::construct(&class, &js_sys::Array::of2(worker, &options)).ok()
}

/// Attaches a transform of `side` to `target`, a sender or a receiver, or answers `None`
/// when the browser has no chunk transform.
fn attach(target: &JsValue, worker: &web_sys::Worker, side: TransformSide) -> Option<()> {
    let t = transform(worker, side)?;
    js_sys::Reflect::set(target, &"transform".into(), &t)
        .ok()
        .filter(|set| *set)
        .map(|_| ())
}

/// The microphone, with the echo canceller, the noise filter and the gain control of the
/// browser on.
async fn microphone() -> Option<web_sys::MediaStream> {
    let devices = web_sys::window()?.navigator().media_devices().ok()?;
    let audio = js_sys::Object::new();
    for flag in ["echoCancellation", "noiseSuppression", "autoGainControl"] {
        js_sys::Reflect::set(&audio, &flag.into(), &JsValue::TRUE).ok()?;
    }
    let constraints = web_sys::MediaStreamConstraints::new();
    constraints.set_audio(&audio);
    let stream = devices.get_user_media_with_constraints(&constraints).ok()?;
    JsFuture::from(stream).await.ok()?.dyn_into().ok()
}

/// The peer connection to the SFU, under the TURN servers `servers`. Its one sender takes
/// the transform of the call worker, then the microphone (`CALL-011`). Each receiver the SFU
/// adds takes the open transform as it arrives, and plays in an audio element of its own.
fn connect(
    worker: &web_sys::Worker,
    servers: &[ambre_protocol::signaling::IceServer],
    mic: &web_sys::MediaStream,
    players: &Rc<RefCell<Vec<web_sys::HtmlAudioElement>>>,
) -> Option<web_sys::RtcPeerConnection> {
    let config = web_sys::RtcConfiguration::new();
    let servers = serde_json::to_string(servers)
        .ok()
        .and_then(|text| js_sys::JSON::parse(&text).ok())?;
    config.set_ice_servers(&servers);
    let pc = web_sys::RtcPeerConnection::new_with_configuration(&config).ok()?;
    let init = web_sys::RtcRtpTransceiverInit::new();
    init.set_direction(web_sys::RtcRtpTransceiverDirection::Sendrecv);
    init.set_streams(&js_sys::Array::of1(mic));
    // The encoder of the sender stays under the constant bitrate the SFU asks, a second bound
    // on the size of each frame (`CALL-067`).
    let encoding = js_sys::Object::new();
    js_sys::Reflect::set(
        &encoding,
        &"maxBitrate".into(),
        &ambre_core::AUDIO_BITRATE.get().into(),
    )
    .ok()?;
    js_sys::Reflect::set(
        &init,
        &"sendEncodings".into(),
        &js_sys::Array::of1(&encoding),
    )
    .ok()?;
    let transceiver = pc.add_transceiver_with_str_and_init("audio", &init);
    let sender = transceiver.sender();
    if attach(&sender, worker, TransformSide::Send).is_none() {
        tracing::error!("[Call] the browser has no chunk transform: no audio is sent");
        pc.close();
        return None;
    }
    let receiver = transceiver.receiver();
    attach(&receiver, worker, TransformSide::Receive);
    let track = mic.get_audio_tracks().get(0).dyn_into().ok();
    // The track comes after the transform, so the first chunk is sealed too.
    let _ = sender.replace_track(track.as_ref());
    let on_track = {
        let worker = worker.clone();
        let players = Rc::clone(players);
        Closure::<dyn FnMut(web_sys::RtcTrackEvent)>::new(move |e: web_sys::RtcTrackEvent| {
            let receiver = e.receiver();
            let attached = js_sys::Reflect::get(&receiver, &"transform".into())
                .is_ok_and(|t| !t.is_null() && !t.is_undefined());
            if !attached {
                attach(&receiver, &worker, TransformSide::Receive);
            }
            // A detached element plays the track: the browser cancels the echo of what a
            // stream plays, and no element enters the page.
            let player = web_sys::HtmlAudioElement::new().ok();
            if let (Ok(stream), Some(player)) = (web_sys::MediaStream::new(), player) {
                stream.add_track(&e.track());
                player.set_autoplay(true);
                player.set_src_object(Some(&stream));
                let _ = player.play();
                players.borrow_mut().push(player);
            }
        })
    };
    pc.set_ontrack(Some(on_track.as_ref().unchecked_ref()));
    // The peer connection lives for the call, and its track handler with it.
    on_track.forget();
    Some(pc)
}

/// Sets `sdp` of `kind` as the remote description of `pc`, or answers `None` when the peer
/// connection refuses it.
async fn remote(
    pc: &web_sys::RtcPeerConnection,
    kind: web_sys::RtcSdpType,
    sdp: &Validated<ambre_protocol::signaling::SessionDescription>,
) -> Option<()> {
    let init = web_sys::RtcSessionDescriptionInit::new(kind);
    init.set_sdp(sdp.as_str());
    JsFuture::from(pc.set_remote_description(&init))
        .await
        .ok()
        .map(|_| ())
}

/// Sets the local description of `pc` from `made`, an offer or an answer, and answers its
/// text.
async fn local(
    pc: &web_sys::RtcPeerConnection,
    made: js_sys::Promise,
) -> Option<Validated<ambre_protocol::signaling::SessionDescription>> {
    let made = JsFuture::from(made).await.ok()?;
    let init: web_sys::RtcSessionDescriptionInit = made.unchecked_into();
    JsFuture::from(pc.set_local_description(&init)).await.ok()?;
    let sdp = pc.local_description()?.sdp();
    Validated::parse(sdp).ok()
}

/// Drives the call: the peer connection comes up with the TURN servers, then answers each
/// session description of the SFU in turn. The end of the call closes it.
async fn drive(worker: web_sys::Worker, rx: ambre_bus::conduit::Receiver<CallSignal>) {
    let mic = microphone().await;
    if mic.is_none() {
        tracing::error!("[Call] no microphone: the call sends no audio");
    }
    let players: Rc<RefCell<Vec<web_sys::HtmlAudioElement>>> = Rc::default();
    let mut pc: Option<web_sys::RtcPeerConnection> = None;
    while let Ok(signal) = rx.recv().await {
        match (signal, pc.as_ref()) {
            (CallSignal::Ice(servers), None) => {
                let Some(mic) = mic.as_ref() else {
                    continue;
                };
                pc = connect(&worker, &servers, mic, &players);
                if let Some(pc) = pc.as_ref()
                    && let Some(offer) = local(pc, pc.create_offer()).await
                {
                    to_sync(
                        &SyncCommand::CallDescription(PageDescription::Offer(offer)),
                        None,
                    );
                }
            }
            (CallSignal::Offer(sdp), Some(pc)) => {
                if remote(pc, web_sys::RtcSdpType::Offer, &sdp).await.is_some()
                    && let Some(answer) = local(pc, pc.create_answer()).await
                {
                    to_sync(
                        &SyncCommand::CallDescription(PageDescription::Answer(answer)),
                        None,
                    );
                }
            }
            (CallSignal::Answer(sdp), Some(pc)) => {
                remote(pc, web_sys::RtcSdpType::Answer, &sdp).await;
            }
            (CallSignal::Ended, _) => break,
            // A signal out of turn: the SFU sends none.
            (CallSignal::Ice(_) | CallSignal::Offer(_) | CallSignal::Answer(_), _) => {}
        }
    }
    if let Some(pc) = pc {
        pc.close();
    }
    if let Some(mic) = mic {
        mic.get_tracks()
            .iter()
            .filter_map(|t| t.dyn_into::<web_sys::MediaStreamTrack>().ok())
            .for_each(|t| t.stop());
    }
    for player in players.borrow().iter() {
        let _ = player.pause();
        player.set_src_object(None);
    }
    SESSION.with(|slot| {
        slot.borrow_mut()
            .take_if(|session| session.worker == worker);
    });
}
