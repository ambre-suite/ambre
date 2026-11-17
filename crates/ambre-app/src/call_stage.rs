//! What the video region shows of the call of the page (the Murmure calls spec, section 3, the
//! video region).
//!
//! The page gathers what it alone holds:
//! * the members the Sync worker admitted, and the video slot of each;
//! * the pictures the render runtime holds;
//! * the mute of the member;
//! * the level of each received audio slot.
//!
//! Murmure lays the region out and paints it
//! (`ambre_murmure::ui::call_videos`).
//!
//! The level of a slot is the audio level the browser reads from the header extension of each
//! received packet (`getSynchronizationSources`). The SFU already reads it in the clear to rank
//! speakers (`CALL-047`), and the page reads it on its own receivers. So outlining who speaks
//! adds nothing to what the SFU learns, and nothing leaves the page (`ZK-025`).

use std::cell::{Cell, RefCell};

use ambre_core::id::{Id, User};
use ambre_core::{Bounded, FracMs, Instant, MeterLevel, OWN_SLOT, PageLoad, SCREEN_SLOT, TimeSpan};
use ambre_murmure::ui::call_controls::CallPhase;
use ambre_murmure::ui::call_videos::{CallStage, Fill, Own, Participant, Voices};
use leptos::prelude::{ArcRwSignal, GetUntracked, Set};

use crate::call_volumes;

/// The interval between two readings of the levels of the audio slots: the call speaker row of
/// `ambre-spec-budgets.md section 1`.
const READ_EVERY: TimeSpan<FracMs> = TimeSpan::<FracMs>::new(100.0);

thread_local! {
    /// Who spoke last, by member.
    static VOICES: RefCell<Voices> = RefCell::new(Voices::default());
    /// The camera track the member's own tile draws, under `OWN_SLOT`.
    static OWN_CAMERA: RefCell<Option<web_sys::MediaStreamTrack>> = const { RefCell::new(None) };
    /// When the page last read the levels of the audio slots.
    static READ_AT: Cell<Option<Instant<PageLoad>>> = const { Cell::new(None) };
    /// How much of the channel region the call takes, which the fill button sets.
    static FILL: ArcRwSignal<Fill> = ArcRwSignal::new(Fill::Band);
    /// Whether the header shows the list of the participants (the Murmure calls spec, section 3, the video region).
    static ROSTER: ArcRwSignal<ambre_chrome::Openness> =
        ArcRwSignal::new(ambre_chrome::Openness::Closed);
}

/// How much of the channel region the call takes, for the fill button.
pub(crate) fn fill() -> ArcRwSignal<Fill> {
    FILL.with(ArcRwSignal::clone)
}

/// Widens the call to the whole channel region, or brings back the band: the press of the fill
/// button. Nothing leaves the page (the Murmure calls spec, section 3, the video region).
pub(crate) fn toggle_fill() {
    FILL.with(|fill| fill.set(fill.get_untracked().toggled()));
}

/// Whether the header shows the list of the participants of the call.
pub(crate) fn roster() -> ArcRwSignal<ambre_chrome::Openness> {
    ROSTER.with(ArcRwSignal::clone)
}

/// Opens the list of the participants: the tile that counts the others asks for it
/// (the Murmure calls spec, section 3, the video region).
pub(crate) fn show_roster() {
    ROSTER.with(|roster| roster.set(ambre_chrome::Openness::Open));
}

/// What the video region shows of the call of the page, `None` out of a call. `me` names the
/// member, whose own tile shows its initials.
pub(crate) fn stage(me: Option<Id<User>>) -> Option<CallStage> {
    let CallPhase::InCall(microphone) = crate::call::phase().get_untracked() else {
        return None;
    };
    let now = crate::host::now();
    hear(now);
    own_camera(crate::call::camera_track());
    let live = ambre_canvas::video::live();
    let others = call_volumes::participants()
        .get_untracked()
        .into_iter()
        .map(|member| Participant {
            member,
            video: call_volumes::videos_of(member)
                .into_iter()
                .find(|slot| live.contains(slot)),
            voice: VOICES.with(|voices| voices.borrow().of(member, now)),
        })
        .collect();
    Some(CallStage {
        others,
        screen: live.contains(&SCREEN_SLOT).then_some(SCREEN_SLOT),
        own: Own {
            member: me,
            video: live.contains(&OWN_SLOT).then_some(OWN_SLOT),
            microphone,
        },
        fill: FILL.with(GetUntracked::get_untracked),
    })
}

/// Hands the render runtime the camera the member sends, under `OWN_SLOT`, and takes it back
/// once the camera stops. The track is the one the video sender carries, so the preview adds no
/// capture and no session description (`CALL-100`).
fn own_camera(track: Option<web_sys::MediaStreamTrack>) {
    // A track is told apart by its identifier, which the browser draws per track.
    let shown = OWN_CAMERA.with(|own| own.borrow().as_ref().map(web_sys::MediaStreamTrack::id));
    if shown == track.as_ref().map(web_sys::MediaStreamTrack::id) {
        return;
    }
    match track.as_ref() {
        Some(track) => ambre_canvas::video::show(OWN_SLOT, track),
        None => ambre_canvas::video::forget(OWN_SLOT),
    }
    OWN_CAMERA.with(|own| *own.borrow_mut() = track);
}

/// Reads the level of each audio slot, once per interval, and keeps who speaks.
fn hear(now: Instant<PageLoad>) {
    let due = READ_AT.with(|at| at.get().is_none_or(|at| now - at >= READ_EVERY));
    if !due {
        return;
    }
    READ_AT.with(|at| at.set(Some(now)));
    for (receiver, slot) in crate::call::audio_receivers() {
        let (Some(member), Some(level)) = (call_volumes::member_in(slot), level_of(&receiver))
        else {
            continue;
        };
        VOICES.with(|voices| voices.borrow_mut().hear(member, level, now));
    }
}

/// The audio level the receiver heard last: of the source it heard most recently, from
/// `getSynchronizationSources`. A browser without it reads none.
fn level_of(receiver: &web_sys::RtcRtpReceiver) -> Option<Bounded<MeterLevel>> {
    let read = js_sys::Reflect::get(receiver, &"getSynchronizationSources".into()).ok()?;
    let read: js_sys::Function = wasm_bindgen::JsCast::dyn_into(read).ok()?;
    let sources: js_sys::Array = wasm_bindgen::JsCast::dyn_into(read.call0(receiver).ok()?).ok()?;
    let field = |source: &wasm_bindgen::JsValue, name: &str| {
        js_sys::Reflect::get(source, &name.into())
            .ok()
            .and_then(|v| v.as_f64())
    };
    let latest = sources.iter().max_by(|a, b| {
        let (a, b) = (field(a, "timestamp"), field(b, "timestamp"));
        a.unwrap_or_default().total_cmp(&b.unwrap_or_default())
    })?;
    #[expect(
        clippy::cast_possible_truncation,
        reason = "an audio level lies in 0..=1, which an f32 holds"
    )]
    let level = field(&latest, "audioLevel")? as f32;
    Bounded::<MeterLevel>::new(level)
}

/// Forgets who spoke: the call ended.
pub(crate) fn forget() {
    VOICES.with(|voices| voices.borrow_mut().clear());
    READ_AT.with(|at| at.set(None));
    OWN_CAMERA.with(|own| *own.borrow_mut() = None);
    FILL.with(|fill| fill.set(Fill::Band));
    ROSTER.with(|roster| roster.set(ambre_chrome::Openness::Closed));
}

#[cfg(test)]
mod tests {
    use super::*;
    use wasm_bindgen::JsCast;
    use wasm_bindgen_test::wasm_bindgen_test;

    /// A receiver whose `getSynchronizationSources` answers `sources`, a script.
    fn receiver(sources: &str) -> web_sys::RtcRtpReceiver {
        let read = js_sys::Function::new_no_args(&format!("return {sources};"));
        let receiver = js_sys::Object::new();
        js_sys::Reflect::set(&receiver, &"getSynchronizationSources".into(), &read).unwrap();
        receiver.unchecked_into()
    }

    /// The camera the member sends goes to the render runtime under `OWN_SLOT`,
    /// and leaves it once the camera stops. A new camera track takes the slot over.
    #[wasm_bindgen_test]
    fn the_own_camera_reaches_the_render_runtime_while_it_is_on() {
        let track = || {
            let canvas = web_sys::window()
                .and_then(|w| w.document())
                .and_then(|d| d.create_element("canvas").ok())
                .unwrap();
            let capture: js_sys::Function = js_sys::Reflect::get(&canvas, &"captureStream".into())
                .unwrap()
                .unchecked_into();
            let stream: web_sys::MediaStream = capture.call0(&canvas).unwrap().unchecked_into();
            stream
                .get_video_tracks()
                .get(0)
                .unchecked_into::<web_sys::MediaStreamTrack>()
        };
        assert!(!ambre_canvas::video::holds(OWN_SLOT));
        let first = track();
        // `Clone::clone` shares the track: the inherent `clone` of a track makes a new one.
        own_camera(Some(Clone::clone(&first)));
        assert!(ambre_canvas::video::holds(OWN_SLOT), "the camera is drawn");
        let second = track();
        own_camera(Some(Clone::clone(&second)));
        assert_eq!(
            OWN_CAMERA.with(|own| own.borrow().as_ref().map(web_sys::MediaStreamTrack::id)),
            Some(second.id())
        );
        own_camera(None);
        assert!(
            !ambre_canvas::video::holds(OWN_SLOT),
            "a stopped camera is drawn no more"
        );
        first.stop();
    }

    /// The page reads the level of a receiver from the source it heard last (the Murmure calls
    /// spec, section 3). So a slot that moved to a new sender reads the new one. A receiver that heard no source, or
    /// a browser without the read, gives no level.
    #[wasm_bindgen_test]
    fn the_level_of_a_slot_is_that_of_its_latest_source() {
        let heard = receiver(
            "[{timestamp: 2, audioLevel: 0.5}, {timestamp: 9, audioLevel: 0.04}, \
             {timestamp: 5, audioLevel: 0.9}]",
        );
        assert_eq!(level_of(&heard), Bounded::<MeterLevel>::new(0.04));
        assert_eq!(level_of(&receiver("[]")), None);
        let without: web_sys::RtcRtpReceiver = js_sys::Object::new().unchecked_into();
        assert_eq!(level_of(&without), None);
    }
}
