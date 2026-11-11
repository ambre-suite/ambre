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
//!
//! A call gesture opens the microphone before anything else. A refusal sends no command, so
//! no seat and no call message follow, and the header shows why (`CALL-076`).
//! A mute disables the track and keeps it on its sender, so the SFU reads a silence
//! (`CALL-075`, `ZK-025`).
//!
//! A change of microphone opens the new one first, then replaces the track on the sender. So
//! the encoder and the transform run through it (`CALL-078`, `CALL-079`). Each audio slot plays
//! through an element of its own, which the chosen speaker routes (`CALL-077`, `CALL-082`).

use std::cell::RefCell;
use std::rc::Rc;

use ambre_bus::{CallSignal, LogicEvent, PageDescription, SyncCommand};
use ambre_core::id::{Call, Channel, Id};
use ambre_core::{CallWorkerNote, Opaque, Payload, TransformSide, Validated};
use ambre_murmure::call_controls::{CallPhase, Microphone, MicrophoneRefusal};
use ambre_murmure::call_devices::{DeviceChoice, DeviceIds, DeviceKind, Fallback, Resolved};
use leptos::prelude::{ArcRwSignal, GetUntracked, Set, Update};
use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;

use crate::call_devices;
use crate::platform::{post_to_worker_with_port, spawn_call_worker};

/// The audio elements of a call, one per audio slot the SFU offers (`CALL-077`).
type Players = Rc<RefCell<Vec<web_sys::HtmlAudioElement>>>;

/// The call the page is in.
struct Session {
    /// The call worker, which holds the keys of this call alone.
    worker: web_sys::Worker,
    /// The signals of the Sync worker, in arrival order, for the task that drives the call.
    signals: ambre_bus::conduit::UnboundedSender<CallSignal>,
    /// The track of the microphone, which a mute disables (`CALL-074`).
    track: Option<web_sys::MediaStreamTrack>,
    /// The device the track of the microphone was opened for.
    microphone: DeviceChoice,
    /// The sender of the microphone, once the peer connection exists.
    sender: Option<web_sys::RtcRtpSender>,
    /// The audio element of each slot (`CALL-077`).
    players: Players,
    /// The speaker each element plays on.
    speaker: DeviceChoice,
}

thread_local! {
    /// The Sync worker, which the join and the session descriptions reach.
    static SYNC: RefCell<Option<web_sys::Worker>> = const { RefCell::new(None) };
    /// The call of the page, one at a time.
    static SESSION: RefCell<Option<Session>> = const { RefCell::new(None) };
    /// The phase of the call of the page, which the call controls of the header read.
    static PHASE: ArcRwSignal<CallPhase> = ArcRwSignal::new(CallPhase::Idle);
}

/// The phase of the call of the page, for the header (the Murmure calls spec).
pub(crate) fn phase() -> ArcRwSignal<CallPhase> {
    PHASE.with(ArcRwSignal::clone)
}

/// Gives the call client the Sync worker the page spawned, and listens for a change of the
/// device list of the browser.
pub(crate) fn install(sync: &web_sys::Worker) {
    SYNC.with(|slot| *slot.borrow_mut() = Some(sync.clone()));
    if let Some(devices) = web_sys::window().and_then(|w| w.navigator().media_devices().ok()) {
        let on_change = Closure::<dyn FnMut()>::new(recheck);
        let _ = devices
            .add_event_listener_with_callback("devicechange", on_change.as_ref().unchecked_ref());
        // The page lives as long as the listener.
        on_change.forget();
    }
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
/// an id that does not parse joins nothing. A belt calls it from the page with no gesture
/// (`CALL-065`).
#[wasm_bindgen]
pub fn join_call(call: &str, channel: &str) {
    let (Ok(call), Ok(channel)) = (
        Id::<Call>::parse_str(call),
        Id::<Channel>::parse_str(channel),
    ) else {
        tracing::warn!("[Call] a join names no call or no channel");
        return;
    };
    join(call, channel);
}

/// Starts a call in the channel `channel`, and answers its identifier, empty when the id
/// does not parse or the page is in a call. The relay creates the call, and the Logic thread
/// posts its call message once the relay seats this session (the Murmure calls spec). A
/// refused microphone starts nothing under the identifier (`CALL-076`). A belt calls it from
/// the page, with no gesture.
#[wasm_bindgen]
pub fn start_call(channel: &str) -> String {
    let Ok(channel) = Id::<Channel>::parse_str(channel) else {
        tracing::warn!("[Call] a start names no channel");
        return String::new();
    };
    start(channel)
        .map(|call| call.to_string())
        .unwrap_or_default()
}

/// Starts a call in `channel`, from the call button of the header (the Murmure calls spec),
/// and answers its identifier. A page in a call, or one that asks for the microphone, starts none.
pub(crate) fn start(channel: Id<Channel>) -> Option<Id<Call>> {
    if !PHASE.with(|phase| phase.get_untracked().accepts_a_gesture()) {
        return None;
    }
    // Each start mints its own identifier, so two racing starts make two calls
    // (the Murmure calls spec). The random bits of the identifier come from the generator of the OS.
    let call = Id::<Call>::mint();
    open(call, channel, Opening::Start);
    Some(call)
}

/// Mutes the microphone of the call, or unmutes it. The track stays on its sender, disabled, so
/// the browser encodes silence at the rate and the size of speech (`CALL-074`, `CALL-075`). No
/// command leaves the page: the relay and the SFU hear of no mute (`ZK-025`).
pub(crate) fn toggle_mute() {
    let Some(microphone) = PHASE.with(|phase| match phase.get_untracked() {
        CallPhase::InCall(microphone) => Some(microphone.toggled()),
        CallPhase::Idle | CallPhase::Asking | CallPhase::Refused(_) => None,
    }) else {
        return;
    };
    SESSION.with(|slot| {
        if let Some(track) = slot.borrow().as_ref().and_then(|s| s.track.as_ref()) {
            track.set_enabled(microphone == Microphone::Live);
        }
    });
    PHASE.with(|phase| phase.set(CallPhase::InCall(microphone)));
}

/// Joins the call `call` of `channel` from its call message (the Murmure calls spec).
pub(crate) fn join(call: Id<Call>, channel: Id<Channel>) {
    open(call, channel, Opening::Join);
}

/// Whether the page starts its call or joins one that runs.
#[derive(Clone, Copy)]
enum Opening {
    Join,
    Start,
}

/// Opens the call `call` of `channel` on this page: the microphone, then the call worker, then
/// the command that joins or starts it. A page in a call, or one that asks for the microphone,
/// opens no second one. A refused microphone ends the steps before the call worker, so no
/// command leaves the page (`CALL-076`).
fn open(call: Id<Call>, channel: Id<Channel>, opening: Opening) {
    if SESSION.with(|slot| slot.borrow().is_some()) {
        return;
    }
    let accepted = PHASE.with(|phase| {
        let accepts = phase.get_untracked().accepts_a_gesture();
        if accepts {
            phase.set(CallPhase::Asking);
        }
        accepts
    });
    if !accepted {
        return;
    }
    let chosen = call_devices::choice(DeviceKind::Microphone).get_untracked();
    wasm_bindgen_futures::spawn_local(async move {
        match microphone(&chosen).await {
            Ok(opened) => {
                PHASE.with(|phase| phase.set(CallPhase::InCall(Microphone::Live)));
                if let Some(gone) = opened.fell_back {
                    call_devices::fell_back(gone);
                }
                seat(call, channel, opening, opened);
                // The grant names the devices, so the panel lists them now.
                call_devices::refresh().await;
            }
            Err(refusal) => {
                tracing::warn!("[Call] no microphone ({refusal:?}): no call opens");
                PHASE.with(|phase| phase.set(CallPhase::Refused(refusal)));
            }
        }
    });
}

/// Spawns the call worker of `call` once the page holds the microphone `mic`. Then it hands the
/// call worker and the Sync worker the command that joins or starts the call.
fn seat(call: Id<Call>, channel: Id<Channel>, opening: Opening, opened: Opened) {
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
                Some(CallWorkerNote::Ready) => hand_ports(&worker, call, channel, opening),
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
    let track = first_track(&opened.stream);
    if let Some(track) = track.as_ref() {
        watch_end(track);
    }
    SESSION.with(|slot| {
        *slot.borrow_mut() = Some(Session {
            worker: worker.clone(),
            signals,
            track,
            microphone: opened.on,
            sender: None,
            players: Rc::default(),
            speaker: call_devices::choice(DeviceKind::Speaker).get_untracked(),
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
fn hand_ports(worker: &web_sys::Worker, call: Id<Call>, channel: Id<Channel>, opening: Opening) {
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
    let command = match opening {
        Opening::Join => SyncCommand::JoinCall { call, channel },
        Opening::Start => SyncCommand::StartCall { call, channel },
    };
    to_sync(&command, Some(&channel_pair.port1()));
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

/// A microphone the page opened.
struct Opened {
    /// The stream of its one track.
    stream: web_sys::MediaStream,
    /// The device it was opened for.
    on: DeviceChoice,
    /// The chosen microphone, when it was gone and the default one serves (`CALL-081`).
    fell_back: Option<Fallback>,
}

/// The microphone `choice` names, with the echo canceller, the noise filter and the gain
/// control of the browser on. A chosen device the browser no longer has gives way to the
/// default one (`CALL-081`). A failure answers the refusal it names (the Murmure calls spec).
async fn microphone(choice: &DeviceChoice) -> Result<Opened, MicrophoneRefusal> {
    let Some(id) = choice.stored() else {
        return capture(None).await.map(|stream| Opened {
            stream,
            on: DeviceChoice::Default,
            fell_back: None,
        });
    };
    match capture(Some(id)).await {
        Ok(stream) => Ok(Opened {
            stream,
            on: choice.clone(),
            fell_back: None,
        }),
        Err(MicrophoneRefusal::Missing) => capture(None).await.map(|stream| Opened {
            stream,
            on: DeviceChoice::Default,
            fell_back: Some(Fallback::Microphone),
        }),
        Err(refusal) => Err(refusal),
    }
}

/// Opens the microphone `device`, or the default one for `None`.
async fn capture(
    device: Option<&Opaque<DeviceIds>>,
) -> Result<web_sys::MediaStream, MicrophoneRefusal> {
    let devices = web_sys::window()
        .and_then(|window| window.navigator().media_devices().ok())
        .ok_or(MicrophoneRefusal::Unavailable)?;
    let audio = js_sys::Object::new();
    for flag in ["echoCancellation", "noiseSuppression", "autoGainControl"] {
        js_sys::Reflect::set(&audio, &flag.into(), &JsValue::TRUE)
            .map_err(|_| MicrophoneRefusal::Unavailable)?;
    }
    if let Some(device) = device {
        // `exact`, so a device that went away fails as `OverconstrainedError`, and never opens
        // another one in silence.
        let exact = js_sys::Object::new();
        js_sys::Reflect::set(&exact, &"exact".into(), &device.as_str().into())
            .and_then(|_| js_sys::Reflect::set(&audio, &"deviceId".into(), &exact))
            .map_err(|_| MicrophoneRefusal::Unavailable)?;
    }
    let constraints = web_sys::MediaStreamConstraints::new();
    constraints.set_audio(&audio);
    let stream = devices
        .get_user_media_with_constraints(&constraints)
        .map_err(|e| refusal_of(&e))?;
    JsFuture::from(stream)
        .await
        .map_err(|e| refusal_of(&e))?
        .dyn_into()
        .map_err(|_| MicrophoneRefusal::Unavailable)
}

/// The one audio track of `stream`.
fn first_track(stream: &web_sys::MediaStream) -> Option<web_sys::MediaStreamTrack> {
    stream.get_audio_tracks().get(0).dyn_into().ok()
}

/// Checks the devices of the call again once `track` ends: a microphone unplugged ends its
/// track (`CALL-081`).
fn watch_end(track: &web_sys::MediaStreamTrack) {
    let on_end = Closure::<dyn FnMut()>::new(recheck);
    let _ = track.add_event_listener_with_callback("ended", on_end.as_ref().unchecked_ref());
    // The track lives for the call at most, and its handler with it.
    on_end.forget();
}

/// Moves the call to the microphone `choice`. A page in no call does nothing: the next call
/// opens the stored choice. The new track takes the mute state of the call (`CALL-080`). It then
/// replaces the old one on the sender, so the transform and the keys stay (`CALL-078`,
/// `CALL-079`).
pub(crate) fn switch_microphone(choice: DeviceChoice) {
    if SESSION.with(|slot| slot.borrow().is_none()) {
        return;
    }
    wasm_bindgen_futures::spawn_local(async move {
        let opened = match microphone(&choice).await {
            Ok(opened) => opened,
            Err(refusal) => {
                tracing::warn!(
                    "[Call] the microphone did not open ({refusal:?}): the call keeps its own"
                );
                return;
            }
        };
        let Some(track) = first_track(&opened.stream) else {
            return;
        };
        if let Some(gone) = opened.fell_back {
            call_devices::fell_back(gone);
        }
        watch_end(&track);
        // The swap and the mute state read the session at one instant. So a mute pressed
        // during the capture lands on the track the call sends.
        let swapped = SESSION.with(|slot| {
            let mut slot = slot.borrow_mut();
            let session = slot.as_mut()?;
            // `Clone::clone` copies the handle. The inherent `clone` of a track would ask the
            // browser for a second track, which neither the mute nor the sender would hold.
            let old = session.track.replace(Clone::clone(&track));
            session.microphone = opened.on.clone();
            Some((old, session.sender.clone()))
        });
        let Some((old, sender)) = swapped else {
            // The call ended during the capture.
            track.stop();
            return;
        };
        let live = PHASE
            .with(|phase| matches!(phase.get_untracked(), CallPhase::InCall(Microphone::Live)));
        track.set_enabled(live);
        if let Some(sender) = sender {
            let _ = JsFuture::from(sender.replace_track(Some(&track))).await;
        }
        if let Some(old) = old {
            old.stop();
        }
    });
}

/// Routes every audio slot of the call to the speaker `choice` (`CALL-082`). A speaker the
/// browser refuses gives way to the default output (`CALL-083`).
pub(crate) fn switch_speaker(choice: DeviceChoice) {
    let Some(players) = SESSION.with(|slot| {
        let mut slot = slot.borrow_mut();
        let session = slot.as_mut()?;
        session.speaker = choice.clone();
        Some(Rc::clone(&session.players))
    }) else {
        return;
    };
    let players: Vec<web_sys::HtmlAudioElement> = players.borrow().clone();
    for player in players {
        route_or_fall_back(player, choice.clone());
    }
}

/// Routes `player` to `choice`, or to the default output once the browser refuses it.
fn route_or_fall_back(player: web_sys::HtmlAudioElement, choice: DeviceChoice) {
    wasm_bindgen_futures::spawn_local(async move {
        if call_devices::route(&player, &choice).await.is_none() && choice != DeviceChoice::Default
        {
            speaker_gone();
        }
    });
}

/// The chosen speaker went away. Every slot plays on the default output (`CALL-083`), and the
/// header says so (the Murmure calls spec, section 3, the devices).
fn speaker_gone() {
    let Some(players) = SESSION.with(|slot| {
        let mut slot = slot.borrow_mut();
        let session = slot.as_mut()?;
        session.speaker = DeviceChoice::Default;
        Some(Rc::clone(&session.players))
    }) else {
        return;
    };
    call_devices::fell_back(Fallback::Speaker);
    let players: Vec<web_sys::HtmlAudioElement> = players.borrow().clone();
    for player in players {
        wasm_bindgen_futures::spawn_local(async move {
            call_devices::route(&player, &DeviceChoice::Default).await;
        });
    }
}

/// Reads the device list again, on a change of it or the end of a track. A microphone that
/// ended, or that the browser no longer lists, gives way to the chosen one if listed, or to the
/// default one (`CALL-081`). A chosen speaker the browser no longer lists gives way to the
/// default output (`CALL-083`).
fn recheck() {
    wasm_bindgen_futures::spawn_local(async {
        let listed = call_devices::refresh().await;
        let Some((ended, on, speaker)) = SESSION.with(|slot| {
            slot.borrow().as_ref().map(|session| {
                let ended = session.track.as_ref().is_none_or(|track| {
                    track.ready_state() == web_sys::MediaStreamTrackState::Ended
                });
                (ended, session.microphone.clone(), session.speaker.clone())
            })
        }) else {
            return;
        };
        let microphones: Vec<_> = listed
            .iter()
            .filter(|device| device.kind == DeviceKind::Microphone)
            .cloned()
            .collect();
        if ended || on.resolve(&microphones) == Resolved::Gone {
            let chosen = call_devices::choice(DeviceKind::Microphone).get_untracked();
            let target = match chosen.resolve(&microphones) {
                Resolved::Chosen(_) => chosen,
                Resolved::Default | Resolved::Gone => DeviceChoice::Default,
            };
            call_devices::fell_back(Fallback::Microphone);
            switch_microphone(target);
        }
        let speakers: Vec<_> = listed
            .iter()
            .filter(|device| device.kind == DeviceKind::Speaker)
            .cloned()
            .collect();
        if speaker.resolve(&speakers) == Resolved::Gone {
            speaker_gone();
        }
    });
}

/// The refusal that the error `error` of a capture names, read from the name of its
/// `DOMException`.
fn refusal_of(error: &JsValue) -> MicrophoneRefusal {
    let name = js_sys::Reflect::get(error, &"name".into())
        .ok()
        .and_then(|name| name.as_string())
        .unwrap_or_default();
    MicrophoneRefusal::from_error_name(&name)
}

/// The peer connection to the SFU, under the TURN servers `servers`. Its one sender takes
/// the transform of the call worker, then the microphone (`CALL-011`). Each receiver the SFU
/// adds takes the open transform as it arrives, and plays in an audio element of its own.
fn connect(
    worker: &web_sys::Worker,
    servers: &[ambre_protocol::signaling::IceServer],
) -> Option<web_sys::RtcPeerConnection> {
    let (track, players) = SESSION.with(|slot| {
        slot.borrow()
            .as_ref()
            .map(|session| (session.track.clone(), Rc::clone(&session.players)))
    })?;
    let mic = web_sys::MediaStream::new().ok()?;
    if let Some(track) = track.as_ref() {
        mic.add_track(track);
    }
    let config = web_sys::RtcConfiguration::new();
    let servers = serde_json::to_string(servers)
        .ok()
        .and_then(|text| js_sys::JSON::parse(&text).ok())?;
    config.set_ice_servers(&servers);
    let pc = web_sys::RtcPeerConnection::new_with_configuration(&config).ok()?;
    let init = web_sys::RtcRtpTransceiverInit::new();
    init.set_direction(web_sys::RtcRtpTransceiverDirection::Sendrecv);
    init.set_streams(&js_sys::Array::of1(&mic));
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
    // The track comes after the transform, so the first chunk is sealed too.
    let _ = sender.replace_track(track.as_ref());
    SESSION.with(|slot| {
        if let Some(session) = slot.borrow_mut().as_mut() {
            session.sender = Some(sender.clone());
        }
    });
    let on_track = {
        let worker = worker.clone();
        let players = Rc::clone(&players);
        Closure::<dyn FnMut(web_sys::RtcTrackEvent)>::new(move |e: web_sys::RtcTrackEvent| {
            let receiver = e.receiver();
            let attached = js_sys::Reflect::get(&receiver, &"transform".into())
                .is_ok_and(|t| !t.is_null() && !t.is_undefined());
            if !attached {
                attach(&receiver, &worker, TransformSide::Receive);
            }
            // A video slot plays nothing here: the canvas draws it once video lands, and an
            // audio element of it would route no sound.
            if e.track().kind() != "audio" {
                return;
            }
            // A detached element plays each slot: the browser cancels the echo of what a
            // stream plays, and no element enters the page. The element has a volume and an
            // output of its own (`CALL-077`).
            let player = web_sys::HtmlAudioElement::new().ok();
            if let (Ok(stream), Some(player)) = (web_sys::MediaStream::new(), player) {
                stream.add_track(&e.track());
                player.set_autoplay(true);
                player.set_src_object(Some(&stream));
                let speaker = SESSION.with(|slot| {
                    slot.borrow()
                        .as_ref()
                        .map(|session| session.speaker.clone())
                });
                if let Some(speaker) = speaker.filter(|s| *s != DeviceChoice::Default) {
                    // A slot that arrives after the choice plays on it too (`CALL-082`).
                    route_or_fall_back(player.clone(), speaker);
                }
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
    let mut pc: Option<web_sys::RtcPeerConnection> = None;
    while let Ok(signal) = rx.recv().await {
        match (signal, pc.as_ref()) {
            (CallSignal::Ice(servers), None) => {
                pc = connect(&worker, &servers);
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
    let ended = SESSION.with(|slot| {
        slot.borrow_mut()
            .take_if(|session| session.worker == worker)
    });
    if let Some(session) = ended {
        if let Some(track) = session.track {
            track.stop();
        }
        for player in session.players.borrow().iter() {
            let _ = player.pause();
            player.set_src_object(None);
        }
    }
    call_devices::forget_fallback();
    PHASE.with(|phase| {
        phase.update(|phase| {
            if matches!(phase, CallPhase::InCall(_)) {
                *phase = CallPhase::Idle;
            }
        });
    });
}
