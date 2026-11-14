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
//!
//! Every call carries one video sender, with three VP8 encodings of the camera (`CALL-096`). A
//! join that asks for the camera opens it after the microphone, and a camera that does not open
//! leaves the call on audio alone. The camera button opens or stops the camera during a call.
//! The sender takes or gives up its track by a replace, so the session is never renegotiated
//! (`CALL-100`). Each received video takes the open transform of the camera, then plays into a
//! detached element that the render runtime draws on the canvas (`CALL-015`).
//!
//! The first share of a call adds a screen sender, with one VP8 encoding (`CALL-115`). The share
//! button asks the browser for a screen or a window from its gesture, and the sender takes the
//! track by a replace (`CALL-103`). A refused or a cancelled capture leaves the call as it was
//! (`CALL-109`). The browser that ends the track stops the share (`CALL-108`). The page marks each
//! start of a share to the SFU, through the Sync worker.
//!
//! The tenth video receiver is the screen slot of the SFU, which opens through the transform of
//! the screen. The SFU forwards the last share alone, and tells the former sharer, whose share
//! stops (`CALL-106`, `CALL-107`).
//!
//! The page numbers each receive transform. The call worker tells it the sender of a slot at
//! its first chunk (`CALL-084`). The Sync worker tells it the member of each sender. So the
//! element of the slot plays at the volume of that member (`CALL-087`). A gain set by hand opens
//! the microphone with the automatic gain off. A graph on the send side alone carries it
//! (`CALL-089`, `CALL-091`).

use std::cell::RefCell;
use std::rc::Rc;

use ambre_bus::{CallSignal, LogicEvent, PageDescription, SyncCommand};
use ambre_core::id::{Call, Channel, Id};
use ambre_core::index::Pos;
use ambre_core::{
    AudioSlots, BitsPerSecond, Bounded, CallWorkerNote, Count, FixedBytes, Opaque,
    ParticipantVolume, Payload, SCREEN_SLOT, SlotSender, Track, TransformSide, Validated,
    VideoSlots,
};
use ambre_murmure::ui::call_controls::{
    CallPhase, Camera, CameraPress, CameraRefusal, Microphone, MicrophoneRefusal, Share,
    SharePress, ShareRefusal,
};
use ambre_murmure::ui::call_devices::{DeviceChoice, DeviceIds, DeviceKind, Fallback, Resolved};
use ambre_murmure::ui::call_volumes::Gain;
use leptos::prelude::{ArcRwSignal, GetUntracked, Set, Update};
use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;

use crate::call_devices;
use crate::call_volumes::{self, GainGraph};
use crate::platform::{post_to_worker_with_port, spawn_call_worker};

/// The audio element of one audio slot (`CALL-077`).
#[derive(Clone)]
struct Player {
    /// The slot, as the page numbered its receive transform.
    slot: Pos<AudioSlots>,
    /// The detached element that plays it.
    element: web_sys::HtmlAudioElement,
}

/// The width the camera opens at: the top size of the call video row of
/// `ambre-spec-budgets.md section 1`.
const CAMERA_WIDTH: u32 = 1280;
/// The height the camera opens at, from the same row.
const CAMERA_HEIGHT: u32 = 720;
/// The three encodings of the camera, lowest first, from the call video row of
/// `ambre-spec-budgets.md section 1` (`CALL-096`). Each holds the RTP stream id, the factor the
/// camera scales down by, and the bitrate cap.
const CAMERA_ENCODINGS: [(&str, f64, Count<BitsPerSecond>); 3] = [
    ("l", 4.0, Count::new(150_000)),
    ("m", 2.0, Count::new(500_000)),
    ("h", 1.0, Count::new(1_500_000)),
];

/// The widest screen the capture asks for: the call screen row of `ambre-spec-budgets.md
/// section 1`.
const SCREEN_WIDTH: u32 = 1920;
/// The tallest screen the capture asks for, from the same row.
const SCREEN_HEIGHT: u32 = 1080;
/// The frame rate the capture of the screen asks for at most, from the same row.
const SCREEN_CAPTURE_RATE: u32 = 30;
/// The one encoding of the screen: its frame rate cap and its bitrate cap, from the same row
/// (`CALL-115`).
const SCREEN_ENCODING: (u32, Count<BitsPerSecond>) = (30, Count::new(1_500_000));

/// The audio elements of a call, one per audio slot the SFU offers (`CALL-077`).
type Players = Rc<RefCell<Vec<Player>>>;

/// The call the page is in.
struct Session {
    /// The call worker, which holds the keys of this call alone.
    worker: web_sys::Worker,
    /// The signals of the Sync worker, in arrival order, for the task that drives the call, and
    /// the asks of the page to offer again.
    signals: ambre_bus::conduit::UnboundedSender<Drive>,
    /// The peer connection, once the SFU answered its TURN servers.
    pc: Option<web_sys::RtcPeerConnection>,
    /// The track of the microphone, which a mute disables (`CALL-074`).
    track: Option<web_sys::MediaStreamTrack>,
    /// The device the track of the microphone was opened for.
    microphone: DeviceChoice,
    /// The sender of the microphone, once the peer connection exists.
    sender: Option<web_sys::RtcRtpSender>,
    /// The track of the camera, while the camera goes into the call.
    camera: Option<web_sys::MediaStreamTrack>,
    /// The video sender, once the peer connection exists. The camera takes it by a replace.
    video: Option<web_sys::RtcRtpSender>,
    /// The track of the screen or the window the member shares, while it goes into the call.
    screen: Option<web_sys::MediaStreamTrack>,
    /// The transceiver of the screen, from the first share of the call on. A later share takes
    /// its sender by a replace (`CALL-112`).
    screen_sender: Option<web_sys::RtcRtpTransceiver>,
    /// The slot the next video receiver takes: the page numbers them in order.
    next_video: Pos<VideoSlots>,
    /// The audio element of each slot (`CALL-077`).
    players: Players,
    /// The speaker each element plays on.
    speaker: DeviceChoice,
    /// The slot of each receiver whose transform the page attached, numbered in order.
    receivers: Vec<(web_sys::RtcRtpReceiver, Pos<AudioSlots>)>,
    /// The graph of the gain, once a gain set by hand or the meter asked for it.
    graph: Option<GainGraph>,
}

thread_local! {
    /// The Sync worker, which the join and the session descriptions reach.
    static SYNC: RefCell<Option<web_sys::Worker>> = const { RefCell::new(None) };
    /// The call of the page, one at a time.
    static SESSION: RefCell<Option<Session>> = const { RefCell::new(None) };
    /// The phase of the call of the page, which the call controls of the header read.
    static PHASE: ArcRwSignal<CallPhase> = ArcRwSignal::new(CallPhase::Idle);
    /// The camera of the call of the page, which the camera button reads.
    static CAMERA: ArcRwSignal<Camera> = ArcRwSignal::new(Camera::Off);
    /// The share of a screen of the call of the page, which the share button reads.
    static SHARE: ArcRwSignal<Share> = ArcRwSignal::new(Share::Off);
}

/// The share of a screen of the call of the page, for the header (the Murmure calls spec,
/// section 3, the screen).
pub(crate) fn share() -> ArcRwSignal<Share> {
    SHARE.with(ArcRwSignal::clone)
}

/// The camera of the call of the page, for the header (the Murmure calls spec, section 3, the
/// camera).
pub(crate) fn camera() -> ArcRwSignal<Camera> {
    CAMERA.with(ArcRwSignal::clone)
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
            let _ = session.signals.send(Drive::Signal(signal));
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

/// Joins the call `call` of the channel `channel` with the camera on. A belt calls this from the
/// page with no gesture (`CALL-065`); a member presses the camera button once in the call.
#[wasm_bindgen]
pub fn join_video_call(call: &str, channel: &str) {
    let (Ok(call), Ok(channel)) = (
        Id::<Call>::parse_str(call),
        Id::<Channel>::parse_str(channel),
    ) else {
        tracing::warn!("[Call] a join names no call or no channel");
        return;
    };
    open(call, channel, Opening::Join, WithCamera::On);
}

/// Starts a call in the channel `channel` with the camera on, and answers its identifier, as
/// [`start_call`] does. A belt calls it from the page, with no gesture.
#[wasm_bindgen]
pub fn start_video_call(channel: &str) -> String {
    let Ok(channel) = Id::<Channel>::parse_str(channel) else {
        tracing::warn!("[Call] a start names no channel");
        return String::new();
    };
    start_with(channel, WithCamera::On)
        .map(|call| call.to_string())
        .unwrap_or_default()
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
    start_with(channel, WithCamera::Off)
}

/// Starts a call in `channel`, with the camera on or off, and answers its identifier.
fn start_with(channel: Id<Channel>, camera: WithCamera) -> Option<Id<Call>> {
    if !PHASE.with(|phase| phase.get_untracked().accepts_a_gesture()) {
        return None;
    }
    // Each start mints its own identifier, so two racing starts make two calls
    // (the Murmure calls spec). The random bits of the identifier come from the generator of the OS.
    let call = Id::<Call>::mint();
    open(call, channel, Opening::Start, camera);
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
    open(call, channel, Opening::Join, WithCamera::Off);
}

/// Opens the camera of the call or stops it, from the camera button (the Murmure calls spec,
/// section 3, the camera). The video sender takes the camera, or gives it up, by a replace: no
/// session description crosses the hop and the transform stays (`CALL-100`). A camera that does
/// not open leaves the call on its audio, and the header says why. Nothing is written into the
/// channel (the Murmure calls spec, section 3, the camera).
pub(crate) fn toggle_camera() {
    if SESSION.with(|slot| slot.borrow().is_none()) {
        return;
    }
    match CAMERA.with(|camera| camera.get_untracked().press()) {
        CameraPress::Wait => {}
        CameraPress::Stop => {
            let gone = SESSION.with(|slot| {
                let mut slot = slot.borrow_mut();
                let session = slot.as_mut()?;
                Some((session.camera.take(), session.video.clone()))
            });
            CAMERA.with(|camera| camera.set(Camera::Off));
            let Some((track, video)) = gone else {
                return;
            };
            wasm_bindgen_futures::spawn_local(async move {
                if let Some(video) = video {
                    let _ = JsFuture::from(video.replace_track(None)).await;
                }
                if let Some(track) = track {
                    track.stop();
                }
            });
        }
        CameraPress::Open => {
            CAMERA.with(|camera| camera.set(Camera::Asking));
            wasm_bindgen_futures::spawn_local(async {
                match open_camera().await {
                    Ok(track) => send_camera(track).await,
                    Err(refusal) => {
                        tracing::warn!("[Call] no camera ({refusal:?}): the call keeps its audio");
                        // A call that ended meanwhile shows no refusal.
                        let next = if SESSION.with(|slot| slot.borrow().is_some()) {
                            Camera::Refused(refusal)
                        } else {
                            Camera::Off
                        };
                        CAMERA.with(|camera| camera.set(next));
                    }
                }
            });
        }
    }
}

/// Shares a screen into the call or stops the share, from the share button (the Murmure calls
/// spec, section 3, the screen). The capture runs from the gesture of the press, as the browser
/// asks. The screen sender takes the track, or gives it up, by a replace: no session description
/// crosses the hop and the transform stays (`CALL-103`). A capture the browser refuses, or a
/// picker the member closes, leaves the call as it was, and the header says why (`CALL-109`).
/// Nothing is written into the channel. The Sync worker hears of the start alone, for the SFU
/// (`ZK-032`).
pub(crate) fn toggle_share() {
    if SESSION.with(|slot| slot.borrow().is_none()) {
        return;
    }
    match SHARE.with(|share| share.get_untracked().press()) {
        SharePress::Wait => {}
        SharePress::Stop => stop_share(Share::Off),
        SharePress::Open => {
            SHARE.with(|share| share.set(Share::Asking));
            wasm_bindgen_futures::spawn_local(async {
                match open_screen().await {
                    Ok(track) => send_screen(track).await,
                    Err(refusal) => {
                        tracing::warn!("[Call] no screen ({refusal:?}): the call goes on");
                        // A call that ended meanwhile shows no refusal.
                        let next = if SESSION.with(|slot| slot.borrow().is_some()) {
                            Share::Refused(refusal)
                        } else {
                            Share::Off
                        };
                        SHARE.with(|share| share.set(next));
                    }
                }
            });
        }
    }
}

/// Stops the share of the page, and leaves the share button in `next`: off, or taken over by
/// the screen of another member (`CALL-107`, `CALL-108`). The sender gives up its track by a
/// replace, then the capture stops.
fn stop_share(next: Share) {
    let gone = SESSION.with(|slot| {
        let mut slot = slot.borrow_mut();
        let session = slot.as_mut()?;
        Some((
            session.screen.take(),
            session
                .screen_sender
                .as_ref()
                .map(web_sys::RtcRtpTransceiver::sender),
        ))
    });
    SHARE.with(|share| share.set(next));
    let Some((track, sender)) = gone else {
        return;
    };
    wasm_bindgen_futures::spawn_local(async move {
        if let Some(sender) = sender {
            let _ = JsFuture::from(sender.replace_track(None)).await;
            activate(&sender, Encoding::Inactive).await;
        }
        if let Some(track) = track {
            track.stop();
        }
    });
}

/// Whether the one encoding of the screen sender sends.
#[derive(Clone, Copy)]
enum Encoding {
    Inactive,
    Active,
}

/// Turns the encoding of the screen sender on or off through its parameters, which renegotiates
/// nothing (`CALL-103`). An encoding off takes no share of the send bandwidth from the camera.
async fn activate(sender: &web_sys::RtcRtpSender, state: Encoding) {
    let call = |name: &str, args: &js_sys::Array| -> Option<JsValue> {
        let method: js_sys::Function = js_sys::Reflect::get(sender, &name.into())
            .ok()?
            .dyn_into()
            .ok()?;
        method.apply(sender, args).ok()
    };
    let Some(parameters) = call("getParameters", &js_sys::Array::new()) else {
        return;
    };
    let Ok(encodings) = js_sys::Reflect::get(&parameters, &"encodings".into()) else {
        return;
    };
    let active = JsValue::from_bool(matches!(state, Encoding::Active));
    for encoding in js_sys::Array::from(&encodings).iter() {
        let _ = js_sys::Reflect::set(&encoding, &"active".into(), &active);
    }
    if let Some(set) = call("setParameters", &js_sys::Array::of1(&parameters))
        .and_then(|p| p.dyn_into::<js_sys::Promise>().ok())
    {
        let _ = JsFuture::from(set).await;
    }
}

/// The screen or the window the member picks, at most the size and the frame rate of the call
/// screen row. The track carries the hint `detail`, so the encoder keeps the size of the text and
/// drops pictures. A failure answers the refusal the browser names (`CALL-109`).
async fn open_screen() -> Result<web_sys::MediaStreamTrack, ShareRefusal> {
    let devices = web_sys::window()
        .and_then(|window| window.navigator().media_devices().ok())
        .ok_or(ShareRefusal::Unavailable)?;
    let video = js_sys::Object::new();
    for (name, cap) in [
        ("width", SCREEN_WIDTH),
        ("height", SCREEN_HEIGHT),
        ("frameRate", SCREEN_CAPTURE_RATE),
    ] {
        let bound = js_sys::Object::new();
        js_sys::Reflect::set(&bound, &"max".into(), &cap.into())
            .and_then(|_| js_sys::Reflect::set(&video, &name.into(), &bound))
            .map_err(|_| ShareRefusal::Unavailable)?;
    }
    let constraints = web_sys::DisplayMediaStreamConstraints::new();
    constraints.set_video(&video);
    constraints.set_audio(&JsValue::FALSE);
    let refused = |e: JsValue| share_refusal_of(&e);
    let asked = devices
        .get_display_media_with_constraints(&constraints)
        .map_err(refused)?;
    let stream: web_sys::MediaStream = JsFuture::from(asked)
        .await
        .map_err(refused)?
        .dyn_into()
        .map_err(|_| ShareRefusal::Unavailable)?;
    let track: web_sys::MediaStreamTrack = stream
        .get_video_tracks()
        .get(0)
        .dyn_into()
        .map_err(|_| ShareRefusal::Missing)?;
    let _ = js_sys::Reflect::set(&track, &"contentHint".into(), &"detail".into());
    Ok(track)
}

/// Hands the screen `track` to the screen sender of the call, by a replace. The browser that
/// ends the track stops the share (`CALL-108`). A call that ended meanwhile stops the track.
async fn send_screen(track: web_sys::MediaStreamTrack) {
    // The first share of the call adds the screen transceiver. It then asks the drive to offer
    // again: one renegotiation, and none for a later share (`CALL-112`). A share before the peer
    // connection exists rides its first offer.
    let sender = SESSION.with(|slot| {
        let mut slot = slot.borrow_mut();
        let session = slot.as_mut()?;
        session.screen = Some(Clone::clone(&track));
        if session.screen_sender.is_none()
            && let Some(pc) = session.pc.as_ref()
        {
            session.screen_sender = screen_transceiver(pc, &session.worker);
            let _ = session.signals.send(Drive::Offer);
        }
        Some(
            session
                .screen_sender
                .as_ref()
                .map(web_sys::RtcRtpTransceiver::sender),
        )
    });
    let Some(sender) = sender else {
        track.stop();
        return;
    };
    let on_end = {
        let ended = Clone::clone(&track);
        Closure::<dyn FnMut()>::new(move || {
            let current = SESSION.with(|slot| {
                slot.borrow()
                    .as_ref()
                    .is_some_and(|session| session.screen.as_ref() == Some(&ended))
            });
            if current {
                stop_share(Share::Off);
            }
        })
    };
    // The handler attribute, not a listener: Firefox 160 calls no listener of a captured screen
    // track, and calls its `onended`.
    track.set_onended(Some(on_end.as_ref().unchecked_ref()));
    // The track lives for one share at most, and its handler with it.
    on_end.forget();
    SHARE.with(|share| share.set(Share::On));
    // A sender not made yet takes the screen from the session when the peer connection opens.
    if let Some(sender) = sender {
        activate(&sender, Encoding::Active).await;
        let _ = JsFuture::from(sender.replace_track(Some(&track))).await;
        mark_share();
    }
}

/// Tells the SFU, through the Sync worker, that the screen sender took a track. A share starts,
/// and the SFU forwards the share that started last (`CALL-106`). A still screen sends no
/// packet for seconds, so the SFU reads no start from the packets. The command carries
/// nothing, and the Sync worker writes it on the socket of the SFU alone (`ZK-032`).
fn mark_share() {
    to_sync(&SyncCommand::ShareStarted, None);
}

/// The SFU told the page that a later share took the screen of the room. A share of the page
/// that goes on stops, and the header says so (`CALL-107`). The SFU alone knows the order of the
/// starts. The screen slot cannot tell it. An unmute may never fire, and a chunk of the former
/// sharer may still be in flight when this page starts its own share.
fn another_screen() {
    let before = SHARE.with(|share| share.get_untracked());
    let after = before.on_another_screen();
    if after != before {
        stop_share(after);
    }
}

/// Hands the camera `track` to the video sender of the call, by a replace. A call that ended
/// meanwhile stops the track.
async fn send_camera(track: web_sys::MediaStreamTrack) {
    let video = SESSION.with(|slot| {
        let mut slot = slot.borrow_mut();
        let session = slot.as_mut()?;
        session.camera = Some(Clone::clone(&track));
        Some(session.video.clone())
    });
    let Some(video) = video else {
        track.stop();
        return;
    };
    CAMERA.with(|camera| camera.set(Camera::On));
    // A sender not made yet takes the camera from the session when the peer connection opens.
    if let Some(video) = video {
        let _ = JsFuture::from(video.replace_track(Some(&track))).await;
    }
}

/// Whether a join opens the camera.
#[derive(Clone, Copy)]
enum WithCamera {
    Off,
    On,
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
/// command leaves the page (`CALL-076`). The camera opens after the microphone when `camera`
/// asks for it, and a camera that does not open leaves the call on audio alone.
fn open(call: Id<Call>, channel: Id<Channel>, opening: Opening, camera: WithCamera) {
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
                let camera = match camera {
                    WithCamera::On => match open_camera().await {
                        Ok(track) => {
                            CAMERA.with(|camera| camera.set(Camera::On));
                            Some(track)
                        }
                        Err(refusal) => {
                            CAMERA.with(|camera| camera.set(Camera::Refused(refusal)));
                            None
                        }
                    },
                    WithCamera::Off => None,
                };
                PHASE.with(|phase| phase.set(CallPhase::InCall(Microphone::Live)));
                if let Some(gone) = opened.fell_back {
                    call_devices::fell_back(gone);
                }
                seat(call, channel, opening, opened, camera);
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

/// Spawns the call worker of `call` once the page holds the microphone `opened`, and the camera
/// `camera` when it opened. Then it hands the call worker and the Sync worker the command that
/// joins or starts the call.
fn seat(
    call: Id<Call>,
    channel: Id<Channel>,
    opening: Opening,
    opened: Opened,
    camera: Option<web_sys::MediaStreamTrack>,
) {
    let worker = spawn_call_worker();
    let (signals, rx) = ambre_bus::conduit::unbounded::<Drive>();
    let on_note = {
        let worker = worker.clone();
        Closure::<dyn FnMut(web_sys::MessageEvent)>::new(move |e: web_sys::MessageEvent| {
            if let Some(note) = slot_sender(&e.data()) {
                // The slot carries a new sender: it plays at the volume of its member at once
                // (`CALL-088`).
                play_at(note.slot, call_volumes::carried(note));
                return;
            }
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
            pc: None,
            track,
            microphone: opened.on,
            sender: None,
            camera,
            video: None,
            screen: None,
            screen_sender: None,
            next_video: Pos::<VideoSlots>::new(0),
            players: Rc::default(),
            speaker: call_devices::choice(DeviceKind::Speaker).get_untracked(),
            receivers: Vec::new(),
            graph: None,
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

/// The note of the call worker that a slot carries a new sender, when `data` is one: its bytes
/// in a typed array. A number is a [`CallWorkerNote`].
fn slot_sender(data: &JsValue) -> Option<SlotSender> {
    let bytes = data.dyn_ref::<js_sys::Uint8Array>()?.to_vec();
    FixedBytes::from_slice(&bytes)
        .ok()
        .map(|bytes| SlotSender::from_bytes(&bytes))
}

/// Sets the element of the slot `slot` to `volume`, once the element exists.
fn play_at(slot: Pos<AudioSlots>, volume: Bounded<ParticipantVolume>) {
    SESSION.with(|session| {
        let Some(session) = session.borrow().as_ref().map(|s| Rc::clone(&s.players)) else {
            return;
        };
        for player in session.borrow().iter().filter(|p| p.slot == slot) {
            player.element.set_volume(f64::from(volume.get()));
        }
    });
}

/// Sets the volume of `member` from the panel: each slot that carries it plays at it now
/// (`CALL-085`, `CALL-087`). Nothing reaches a worker (`CALL-095`).
pub(crate) fn set_volume(
    member: ambre_core::id::Id<ambre_core::id::User>,
    volume: Bounded<ParticipantVolume>,
) {
    for (slot, volume) in call_volumes::set(member, volume) {
        play_at(slot, volume);
    }
}

/// The next slot of the call: the count of receivers whose transform the page attached.
fn next_slot(receiver: &web_sys::RtcRtpReceiver) -> Option<Pos<AudioSlots>> {
    SESSION.with(|slot| {
        let mut slot = slot.borrow_mut();
        let session = slot.as_mut()?;
        let at = Pos::new(u32::try_from(session.receivers.len()).ok()?);
        session.receivers.push((receiver.clone(), at));
        Some(at)
    })
}

/// The slot of the next video receiver, which the page numbers in order of arrival.
fn next_video_slot() -> Option<Pos<VideoSlots>> {
    SESSION.with(|slot| {
        let mut slot = slot.borrow_mut();
        let session = slot.as_mut()?;
        let at = session.next_video;
        session.next_video = at.next();
        Some(at)
    })
}

/// The slot the page gave the receiver `receiver`.
fn slot_of(receiver: &web_sys::RtcRtpReceiver) -> Option<Pos<AudioSlots>> {
    SESSION.with(|slot| {
        slot.borrow()
            .as_ref()?
            .receivers
            .iter()
            .find(|(r, _)| r == receiver)
            .map(|(_, at)| *at)
    })
}

/// A transform of the call worker, for one side of the track `track`, and for the slot `slot`
/// of an audio receiver.
fn transform(
    worker: &web_sys::Worker,
    side: TransformSide,
    track: Track,
    slot: Option<Pos<AudioSlots>>,
) -> Option<JsValue> {
    let options = js_sys::Object::new();
    js_sys::Reflect::set(&options, &"side".into(), &side.as_str().into()).ok()?;
    js_sys::Reflect::set(&options, &"track".into(), &track.bits().into()).ok()?;
    if let Some(slot) = slot {
        js_sys::Reflect::set(&options, &"slot".into(), &slot.get().into()).ok()?;
    }
    let class = js_sys::Reflect::get(&js_sys::global(), &"RTCRtpScriptTransform".into()).ok()?;
    let class: js_sys::Function = class.dyn_into().ok()?;
    js_sys::Reflect::construct(&class, &js_sys::Array::of2(worker, &options)).ok()
}

/// Attaches a transform of `side` of the track `track` to `target`, a sender or a receiver of
/// the slot `slot`. Answers `None` when the browser has no chunk transform.
fn attach(
    target: &JsValue,
    worker: &web_sys::Worker,
    side: TransformSide,
    track: Track,
    slot: Option<Pos<AudioSlots>>,
) -> Option<()> {
    let t = transform(worker, side, track, slot)?;
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

/// The microphone `choice` names, with the echo canceller and the noise filter of the browser
/// on. Its automatic gain is on unless a gain is set by hand. A chosen device the browser no
/// longer has gives way to the default one (`CALL-081`). A failure answers the refusal it names (the Murmure calls spec).
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
    // The echo canceller and the noise filter stay on under any gain (`CALL-091`). A gain set
    // by hand turns the automatic gain off, so the two never fight (`CALL-089`).
    let automatic = call_volumes::gain().get_untracked() == Gain::Automatic;
    for (flag, on) in [
        ("echoCancellation", true),
        ("noiseSuppression", true),
        ("autoGainControl", automatic),
    ] {
        js_sys::Reflect::set(&audio, &flag.into(), &JsValue::from_bool(on))
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

/// The camera at the top size of the call video row, or the refusal the browser names when it
/// refuses it or has none. A call goes on with its audio alone then (the Murmure calls spec).
async fn open_camera() -> Result<web_sys::MediaStreamTrack, CameraRefusal> {
    let devices = web_sys::window()
        .and_then(|window| window.navigator().media_devices().ok())
        .ok_or(CameraRefusal::Unavailable)?;
    let video = js_sys::Object::new();
    js_sys::Reflect::set(&video, &"width".into(), &CAMERA_WIDTH.into())
        .and_then(|_| js_sys::Reflect::set(&video, &"height".into(), &CAMERA_HEIGHT.into()))
        .map_err(|_| CameraRefusal::Unavailable)?;
    let constraints = web_sys::MediaStreamConstraints::new();
    constraints.set_video(&video);
    let refused = |e: JsValue| camera_refusal_of(&e);
    let asked = devices
        .get_user_media_with_constraints(&constraints)
        .map_err(refused)?;
    let stream: web_sys::MediaStream = JsFuture::from(asked)
        .await
        .map_err(refused)?
        .dyn_into()
        .map_err(|_| CameraRefusal::Unavailable)?;
    stream
        .get_video_tracks()
        .get(0)
        .dyn_into()
        .map_err(|_| CameraRefusal::Missing)
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
        // A graph takes the new capture, so a gain set by hand stays (`CALL-094`).
        let sent = sent_track(Some(&track));
        let current = sender.as_ref().and_then(web_sys::RtcRtpSender::track);
        if let Some(sender) = sender
            && sent != current
        {
            let _ = JsFuture::from(sender.replace_track(sent.as_ref())).await;
        }
        if let Some(old) = old {
            old.stop();
        }
    });
}

/// The track the sender takes for the capture `capture`. A graph of the call takes the capture
/// first, for the meter. Under a gain set by hand, the sender takes the output of the graph,
/// which a gesture created; otherwise it takes the capture itself (`CALL-089`).
fn sent_track(capture: Option<&web_sys::MediaStreamTrack>) -> Option<web_sys::MediaStreamTrack> {
    let gain = call_volumes::gain().get_untracked();
    SESSION.with(|slot| {
        let mut slot = slot.borrow_mut();
        let session = slot.as_mut()?;
        if session.graph.is_none() && gain != Gain::Automatic {
            session.graph = GainGraph::new(gain);
        }
        let graph = session.graph.as_mut();
        match (graph, capture) {
            (Some(graph), Some(capture)) => {
                graph.feed(capture);
                graph.set(gain);
                match gain {
                    Gain::ByHand(_) => graph.track(),
                    Gain::Automatic => Some(Clone::clone(capture)),
                }
            }
            (_, capture) => capture.cloned(),
        }
    })
}

/// Reads the graph of the call, when it has one.
pub(crate) fn with_graph<T>(read: impl FnOnce(&GainGraph) -> T) -> Option<T> {
    SESSION.with(|slot| slot.try_borrow().ok()?.as_ref()?.graph.as_ref().map(read))
}

/// Gives the call a graph of its capture for the meter of the panel, from the gesture that
/// opened it, when it holds none. The sender keeps its track under the automatic gain.
pub(crate) fn watch_level() {
    SESSION.with(|slot| {
        let mut slot = slot.borrow_mut();
        let Some(session) = slot.as_mut() else {
            return;
        };
        if let Some(graph) = session.graph.as_ref() {
            graph.wake();
            return;
        }
        let Some(mut graph) = GainGraph::new(call_volumes::gain().get_untracked()) else {
            return;
        };
        if let Some(track) = session.track.as_ref() {
            graph.feed(track);
        }
        session.graph = Some(graph);
    });
}

/// Sets the gain of the microphone from the panel, and keeps it on this browser. A move to or
/// from the automatic gain opens the microphone again, with the automatic gain on or off
/// (`CALL-089`). A move between two gains set by hand sets the
/// graph alone. No session description and no key moves (`CALL-092`).
pub(crate) fn set_gain(gain: Gain) {
    let before = call_volumes::gain().get_untracked();
    call_volumes::keep_gain(gain);
    let microphone = SESSION.with(|slot| {
        let mut slot = slot.borrow_mut();
        let session = slot.as_mut()?;
        if session.graph.is_none() && gain != Gain::Automatic {
            session.graph = GainGraph::new(gain);
        }
        if let Some(graph) = session.graph.as_ref() {
            graph.wake();
            graph.set(gain);
        }
        Some(session.microphone.clone())
    });
    let Some(microphone) = microphone else {
        return;
    };
    if (before == Gain::Automatic) != (gain == Gain::Automatic) {
        switch_microphone(microphone);
    }
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
    let players: Vec<Player> = players.borrow().clone();
    for player in players {
        route_or_fall_back(player.element, choice.clone());
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
    let players: Vec<Player> = players.borrow().clone();
    for player in players {
        wasm_bindgen_futures::spawn_local(async move {
            call_devices::route(&player.element, &DeviceChoice::Default).await;
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

/// The refusal of a screen that the error `error` of a capture names. It reads the name of its
/// `DOMException` (the Murmure calls spec, section 3, the screen).
fn share_refusal_of(error: &JsValue) -> ShareRefusal {
    let name = js_sys::Reflect::get(error, &"name".into())
        .ok()
        .and_then(|name| name.as_string())
        .unwrap_or_default();
    ShareRefusal::from_error_name(&name)
}

/// The refusal of the camera that the error `error` of a capture names. It reads the name of
/// the `DOMException` (the Murmure calls spec, section 3, the camera).
fn camera_refusal_of(error: &JsValue) -> CameraRefusal {
    let name = js_sys::Reflect::get(error, &"name".into())
        .ok()
        .and_then(|name| name.as_string())
        .unwrap_or_default();
    CameraRefusal::from_error_name(&name)
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
    if attach(&sender, worker, TransformSide::Send, Track::Audio, None).is_none() {
        tracing::error!("[Call] the browser has no chunk transform: no audio is sent");
        pc.close();
        return None;
    }
    let receiver = transceiver.receiver();
    let first = next_slot(&receiver);
    attach(
        &receiver,
        worker,
        TransformSide::Receive,
        Track::Audio,
        first,
    );
    let Some(video) = video_sender(&pc, worker) else {
        tracing::error!("[Call] the video sender did not open: no call is held");
        pc.close();
        return None;
    };
    // The screen transceiver waits for the first share: an idle second video section keeps
    // Firefox from raising the layers of the camera (`CALL-112`). A share pressed before this
    // peer connection rides its first offer.
    let shares = SESSION.with(|slot| slot.borrow().as_ref().is_some_and(|s| s.screen.is_some()));
    let screen_sender = if shares {
        screen_transceiver(&pc, worker)
    } else {
        None
    };
    // A gain set by hand sends the output of its graph, and the automatic gain the capture.
    let sent = sent_track(track.as_ref());
    // The track comes after the transform, so the first chunk is sealed too.
    let _ = sender.replace_track(sent.as_ref());
    let (camera, screen) = SESSION.with(|slot| {
        let mut slot = slot.borrow_mut();
        let Some(session) = slot.as_mut() else {
            return (None, None);
        };
        session.sender = Some(sender.clone());
        session.video = Some(video.clone());
        session.screen_sender = screen_sender.clone();
        session.pc = Some(pc.clone());
        (session.camera.clone(), session.screen.clone())
    });
    if let Some(camera) = camera {
        // The track comes after the transform, so the first chunk of the camera is sealed too.
        let _ = video.replace_track(Some(&camera));
    }
    if let (Some(screen), Some(transceiver)) = (screen, screen_sender) {
        let sender = transceiver.sender();
        wasm_bindgen_futures::spawn_local(async move {
            activate(&sender, Encoding::Active).await;
            let _ = JsFuture::from(sender.replace_track(Some(&screen))).await;
            mark_share();
        });
    }
    let on_track = {
        let worker = worker.clone();
        let players = Rc::clone(&players);
        Closure::<dyn FnMut(web_sys::RtcTrackEvent)>::new(move |e: web_sys::RtcTrackEvent| {
            let receiver = e.receiver();
            let attached = js_sys::Reflect::get(&receiver, &"transform".into())
                .is_ok_and(|t| !t.is_null() && !t.is_undefined());
            // A video slot opens through the transform of the camera. A detached element plays
            // it, and the render runtime draws its picture on the canvas: no element enters the
            // page (`CALL-015`).
            // The tenth video slot is the screen slot of the SFU: it opens through the transform
            // of the screen alone (`CALL-110`).
            if e.track().kind() != "audio" {
                if !attached && let Some(slot) = next_video_slot() {
                    let track = if slot == SCREEN_SLOT {
                        Track::Screen
                    } else {
                        Track::Camera
                    };
                    attach(&receiver, &worker, TransformSide::Receive, track, None);
                    ambre_canvas::video::show(slot, &e.track());
                }
                return;
            }
            let slot = if attached {
                slot_of(&receiver)
            } else {
                let slot = next_slot(&receiver);
                attach(
                    &receiver,
                    &worker,
                    TransformSide::Receive,
                    Track::Audio,
                    slot,
                );
                slot
            };
            let Some(slot) = slot else {
                return;
            };
            // A detached element plays each slot: the browser cancels the echo of what a
            // stream plays, and no element enters the page. The element has a volume and an
            // output of its own (`CALL-077`).
            let player = web_sys::HtmlAudioElement::new().ok();
            if let (Ok(stream), Some(player)) = (web_sys::MediaStream::new(), player) {
                stream.add_track(&e.track());
                player.set_autoplay(true);
                player.set_src_object(Some(&stream));
                // A note may come before the element: the slot plays at the volume it carries.
                player.set_volume(f64::from(call_volumes::volume_of(slot).get()));
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
                players.borrow_mut().push(Player {
                    slot,
                    element: player,
                });
            }
        })
    };
    pc.set_ontrack(Some(on_track.as_ref().unchecked_ref()));
    // The peer connection lives for the call, and its track handler with it.
    on_track.forget();
    Some(pc)
}

/// The video sender of `pc`: three VP8 encodings of the camera, with the seal transform of the
/// call worker and no track yet (`CALL-096`). A camera takes it by a replace, so the session is
/// never renegotiated for it.
fn video_sender(
    pc: &web_sys::RtcPeerConnection,
    worker: &web_sys::Worker,
) -> Option<web_sys::RtcRtpSender> {
    let encodings = js_sys::Array::new();
    for (rid, down, cap) in CAMERA_ENCODINGS {
        let encoding = js_sys::Object::new();
        js_sys::Reflect::set(&encoding, &"rid".into(), &rid.into()).ok()?;
        js_sys::Reflect::set(&encoding, &"scaleResolutionDownBy".into(), &down.into()).ok()?;
        js_sys::Reflect::set(&encoding, &"maxBitrate".into(), &cap.get().into()).ok()?;
        encodings.push(&encoding);
    }
    let init = web_sys::RtcRtpTransceiverInit::new();
    init.set_direction(web_sys::RtcRtpTransceiverDirection::Sendonly);
    js_sys::Reflect::set(&init, &"sendEncodings".into(), &encodings).ok()?;
    let sender = pc
        .add_transceiver_with_str_and_init("video", &init)
        .sender();
    attach(&sender, worker, TransformSide::Send, Track::Camera, None)?;
    Some(sender)
}

/// The screen transceiver of `pc`, which the first share of a call adds. It holds one VP8
/// encoding, capped at the frame rate and the bitrate of the call screen row (`CALL-115`). Its sender holds the
/// seal transform of the screen and no track yet. Each share takes the sender by a replace, so no
/// later share renegotiates (`CALL-103`, `CALL-112`).
fn screen_transceiver(
    pc: &web_sys::RtcPeerConnection,
    worker: &web_sys::Worker,
) -> Option<web_sys::RtcRtpTransceiver> {
    let (frame_rate, bitrate) = SCREEN_ENCODING;
    let encoding = js_sys::Object::new();
    js_sys::Reflect::set(&encoding, &"maxFramerate".into(), &frame_rate.into()).ok()?;
    js_sys::Reflect::set(&encoding, &"maxBitrate".into(), &bitrate.get().into()).ok()?;
    let init = web_sys::RtcRtpTransceiverInit::new();
    init.set_direction(web_sys::RtcRtpTransceiverDirection::Sendonly);
    js_sys::Reflect::set(
        &init,
        &"sendEncodings".into(),
        &js_sys::Array::of1(&encoding),
    )
    .ok()?;
    let transceiver = pc.add_transceiver_with_str_and_init("video", &init);
    attach(
        &transceiver.sender(),
        worker,
        TransformSide::Send,
        Track::Screen,
        None,
    )?;
    Some(transceiver)
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

/// What drives the call. It is a signal of the Sync worker, or an ask of the page to offer again
/// once it added the screen transceiver (`CALL-112`).
enum Drive {
    Signal(CallSignal),
    Offer,
}

/// Offers again when the screen transceiver waits for its section and no offer is in flight.
/// The SFU drops an offer that crosses its own, so an answer to the SFU checks again.
async fn offer_screen(pc: &web_sys::RtcPeerConnection) {
    let waits = SESSION.with(|slot| {
        slot.borrow()
            .as_ref()
            .and_then(|s| s.screen_sender.as_ref())
            .is_some_and(|t| t.mid().is_none())
    });
    if waits
        && pc.signaling_state() == web_sys::RtcSignalingState::Stable
        && let Some(offer) = local(pc, pc.create_offer()).await
    {
        to_sync(
            &SyncCommand::CallDescription(PageDescription::Offer(offer)),
            None,
        );
    }
}

/// Drives the call: the peer connection comes up with the TURN servers, then answers each
/// session description of the SFU in turn. The first share offers again once (`CALL-112`). An
/// offer of the SFU that crosses it rolls it back: the page answers, then offers again. The end
/// of the call closes it.
async fn drive(worker: web_sys::Worker, rx: ambre_bus::conduit::Receiver<Drive>) {
    let mut pc: Option<web_sys::RtcPeerConnection> = None;
    while let Ok(drive) = rx.recv().await {
        let signal = match drive {
            Drive::Signal(signal) => signal,
            Drive::Offer => {
                if let Some(pc) = pc.as_ref() {
                    offer_screen(pc).await;
                }
                continue;
            }
        };
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
                // An offer of the page still in flight rolls back here: the SFU dropped it.
                if remote(pc, web_sys::RtcSdpType::Offer, &sdp).await.is_some()
                    && let Some(answer) = local(pc, pc.create_answer()).await
                {
                    to_sync(
                        &SyncCommand::CallDescription(PageDescription::Answer(answer)),
                        None,
                    );
                }
                offer_screen(pc).await;
            }
            (CallSignal::Answer(sdp), Some(pc)) => {
                remote(pc, web_sys::RtcSdpType::Answer, &sdp).await;
                offer_screen(pc).await;
            }
            (CallSignal::Ended, _) => break,
            (CallSignal::Admitted { sender, member }, _) => {
                for (slot, volume) in call_volumes::admitted(sender, member) {
                    play_at(slot, volume);
                }
            }
            (CallSignal::Departed(sender), _) => call_volumes::departed(sender),
            // A later share took the screen of the room: the share of this page stops
            // (`CALL-107`).
            (CallSignal::ShareTakenOver, _) => another_screen(),
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
        if let Some(camera) = session.camera {
            camera.stop();
        }
        if let Some(screen) = session.screen {
            screen.stop();
        }
        if let Some(graph) = session.graph {
            graph.close();
        }
        for player in session.players.borrow().iter() {
            let _ = player.element.pause();
            player.element.set_src_object(None);
        }
    }
    ambre_canvas::video::forget_all();
    CAMERA.with(|camera| camera.set(Camera::Off));
    SHARE.with(|share| share.set(Share::Off));
    call_volumes::forget();
    call_devices::forget_fallback();
    PHASE.with(|phase| {
        phase.update(|phase| {
            if matches!(phase, CallPhase::InCall(_)) {
                *phase = CallPhase::Idle;
            }
        });
    });
}
