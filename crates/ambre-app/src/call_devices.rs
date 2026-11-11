//! The devices of a call on the UI thread (the Murmure calls spec, section 3, the devices, and
//! `ambre-spec-call-media.md section 3`, devices).
//!
//! The page alone reads the device list of the browser and keeps the choice, in the local
//! storage of this browser. No device identifier and no label reaches a worker, so no frame of
//! a socket carries one (`ZK-026`). Murmure decides what the panel shows
//! (`ambre_murmure::call_devices`); this module reads the browser and routes the audio.

use ambre_core::Opaque;
use ambre_murmure::call_devices::{Device, DeviceChoice, DeviceKind, Fallback, Routing};
use leptos::prelude::{ArcRwSignal, GetUntracked, Set};
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::JsFuture;

thread_local! {
    /// The devices the browser listed last, for the device panel.
    static LISTED: ArcRwSignal<Vec<Device>> = ArcRwSignal::new(Vec::new());
    /// The chosen microphone, read once from the local storage.
    static MICROPHONE: ArcRwSignal<DeviceChoice> = ArcRwSignal::new(stored(DeviceKind::Microphone));
    /// The chosen speaker, read once from the local storage.
    static SPEAKER: ArcRwSignal<DeviceChoice> = ArcRwSignal::new(stored(DeviceKind::Speaker));
    /// The chosen device that went away during the call, which the header names.
    static FALLBACK: ArcRwSignal<Option<Fallback>> = ArcRwSignal::new(None);
}

/// The devices the browser listed last.
pub(crate) fn listed() -> ArcRwSignal<Vec<Device>> {
    LISTED.with(ArcRwSignal::clone)
}

/// The choice of `kind`.
pub(crate) fn choice(kind: DeviceKind) -> ArcRwSignal<DeviceChoice> {
    match kind {
        DeviceKind::Microphone => MICROPHONE.with(ArcRwSignal::clone),
        DeviceKind::Speaker => SPEAKER.with(ArcRwSignal::clone),
    }
}

/// The chosen device that went away during the call, if any.
pub(crate) fn fallback() -> ArcRwSignal<Option<Fallback>> {
    FALLBACK.with(ArcRwSignal::clone)
}

/// The choice of `kind` the local storage of this browser holds.
fn stored(kind: DeviceKind) -> DeviceChoice {
    crate::platform::device_choice(kind)
}

/// Keeps `choice` of `kind` in the local storage of this browser, and nowhere else (the Murmure
/// calls spec, section 3, the devices).
pub(crate) fn keep(kind: DeviceKind, choice: DeviceChoice) {
    crate::platform::keep_device_choice(kind, &choice);
    choice_set(kind, choice);
}

/// Sets the choice of `kind`, and clears the notice of a device of that kind gone.
fn choice_set(kind: DeviceKind, choice: DeviceChoice) {
    FALLBACK.with(|fallback| {
        let shown = fallback.get_untracked();
        let cleared = matches!(
            (shown, kind),
            (Some(Fallback::Microphone), DeviceKind::Microphone)
                | (Some(Fallback::Speaker), DeviceKind::Speaker)
        );
        if cleared {
            fallback.set(None);
        }
    });
    self::choice(kind).set(choice);
}

/// Shows that `gone`, a chosen device, went away and the default one serves.
pub(crate) fn fell_back(gone: Fallback) {
    FALLBACK.with(|fallback| fallback.set(Some(gone)));
}

/// Clears the notice of a device gone, at the end of a call.
pub(crate) fn forget_fallback() {
    FALLBACK.with(|fallback| fallback.set(None));
}

/// Reads the device list of the browser into [`listed`], and answers it. The pseudo devices
/// that Chrome lists, `default` and `communications`, stand for the system choice, which the
/// panel shows as its default entry.
pub(crate) async fn refresh() -> Vec<Device> {
    let devices = list().await;
    LISTED.with(|listed| listed.set(devices.clone()));
    devices
}

/// The microphones and the speakers the browser lists now.
async fn list() -> Vec<Device> {
    let Some(promise) = web_sys::window()
        .and_then(|window| window.navigator().media_devices().ok())
        .and_then(|devices| devices.enumerate_devices().ok())
    else {
        return Vec::new();
    };
    let Ok(infos) = JsFuture::from(promise).await else {
        return Vec::new();
    };
    js_sys::Array::from(&infos)
        .iter()
        .filter_map(|info| info.dyn_into::<web_sys::MediaDeviceInfo>().ok())
        .filter_map(|info| {
            let kind = match info.kind() {
                web_sys::MediaDeviceKind::Audioinput => DeviceKind::Microphone,
                web_sys::MediaDeviceKind::Audiooutput => DeviceKind::Speaker,
                _ => return None,
            };
            let id = info.device_id();
            // The system choice, which the default entry stands for.
            if id == "default" || id == "communications" {
                return None;
            }
            Some(Device {
                kind,
                id: Opaque::new(id),
                label: Opaque::new(info.label()),
            })
        })
        .collect()
}

/// Whether an audio element of this browser routes to a chosen output: it has `setSinkId`.
pub(crate) fn routing() -> Routing {
    let routes = js_sys::Reflect::get(&js_sys::global(), &"HTMLMediaElement".into())
        .and_then(|class| js_sys::Reflect::get(&class, &"prototype".into()))
        .and_then(|prototype| js_sys::Reflect::has(&prototype, &"setSinkId".into()))
        .unwrap_or_default();
    if routes {
        Routing::Chosen
    } else {
        Routing::Fixed
    }
}

/// Routes `player` to the speaker `choice`, the default output for the default choice. Answers
/// `None` when the browser refuses the output, for an output it no longer has (`CALL-083`).
pub(crate) async fn route(player: &web_sys::HtmlMediaElement, choice: &DeviceChoice) -> Option<()> {
    if routing() == Routing::Fixed {
        return Some(());
    }
    let sink = choice.stored().map_or("", Opaque::as_str);
    let set: js_sys::Function = js_sys::Reflect::get(player, &"setSinkId".into())
        .ok()?
        .dyn_into()
        .ok()?;
    let promise: js_sys::Promise = set.call1(player, &sink.into()).ok()?.dyn_into().ok()?;
    match JsFuture::from(promise).await {
        Ok(_) => Some(()),
        Err(refused) => {
            tracing::warn!("[Call] the browser refused the speaker: {refused:?}");
            None
        }
    }
}
