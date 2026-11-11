//! The volumes of a call on the UI thread (the Murmure calls spec, section 3, the volumes, and
//! `ambre-spec-call-media.md section 3`, the volumes and the gain).
//!
//! The page keeps the volume of each member and the gain in the local storage of this browser.
//! No volume and no gain reaches a worker (`ZK-027`). Murmure decides which volume each slot
//! plays at (`ambre_murmure::call_volumes`). This module reads the browser, holds the graph of
//! the gain on the send side and reads the meter.
//!
//! The graph never touches the playout. Each slot plays through its element, whose volume the
//! page sets, so the browser keeps cancelling its echo (`CALL-077`).

use std::cell::RefCell;

use ambre_core::id::{Id, User};
use ambre_core::index::Pos;
use ambre_core::{
    AudioSlots, Bounded, MeterLevel, ParticipantVolume, SenderIndex, Seq, SlotSender,
};
use ambre_murmure::call_volumes::{Gain, Playout, VolumeSupport, Volumes};
use leptos::prelude::{ArcRwSignal, GetUntracked, Set, Update};
use wasm_bindgen::JsCast;

thread_local! {
    /// The volume of each member, read once from the local storage.
    static VOLUMES: ArcRwSignal<Volumes> = ArcRwSignal::new(stored_volumes());
    /// The gain of the microphone, read once from the local storage.
    static GAIN: ArcRwSignal<Gain> = ArcRwSignal::new(stored_gain());
    /// The members in the call, whose volume the panel offers.
    static PARTICIPANTS: ArcRwSignal<Vec<Id<User>>> = ArcRwSignal::new(Vec::new());
    /// The level the microphone sends, for the meter.
    static LEVEL: ArcRwSignal<Bounded<MeterLevel>> = ArcRwSignal::new(Bounded::<MeterLevel>::MIN);
    /// The sender each slot carries and the member of each sender, for the call of the page.
    static PLAYOUT: RefCell<Playout> = RefCell::new(Playout::default());
}

/// The volume of each member.
pub(crate) fn volumes() -> ArcRwSignal<Volumes> {
    VOLUMES.with(ArcRwSignal::clone)
}

/// The gain of the microphone.
pub(crate) fn gain() -> ArcRwSignal<Gain> {
    GAIN.with(ArcRwSignal::clone)
}

/// The members in the call.
pub(crate) fn participants() -> ArcRwSignal<Vec<Id<User>>> {
    PARTICIPANTS.with(ArcRwSignal::clone)
}

/// The level the microphone sends.
pub(crate) fn level() -> ArcRwSignal<Bounded<MeterLevel>> {
    LEVEL.with(ArcRwSignal::clone)
}

/// The local storage of this browser, when it has one.
fn storage() -> Option<web_sys::Storage> {
    web_sys::window().and_then(|window| window.local_storage().ok().flatten())
}

/// The volumes the local storage holds.
fn stored_volumes() -> Volumes {
    let text = storage().and_then(|s| s.get_item(Volumes::storage_key().as_str()).ok().flatten());
    Volumes::from_stored(text.as_deref())
}

/// The gain the local storage holds.
fn stored_gain() -> Gain {
    let text = storage().and_then(|s| s.get_item(Gain::storage_key().as_str()).ok().flatten());
    Gain::from_stored(text.as_deref())
}

/// Whether an audio element of this browser takes the volume the page sets. The page sets a
/// probe element to half and reads it back. A read-only volume, as on iOS, keeps its own (the
/// Murmure calls spec, section 3, the volumes). The test reads the behaviour, never the name of
/// the browser.
pub(crate) fn support() -> VolumeSupport {
    let Ok(probe) = web_sys::HtmlAudioElement::new() else {
        return VolumeSupport::ReadOnly;
    };
    let asked = Bounded::<ParticipantVolume>::new(0.5).unwrap_or_default();
    probe.set_volume(f64::from(asked.get()));
    #[expect(
        clippy::cast_possible_truncation,
        reason = "an element volume lies in 0..=1, which an f32 holds"
    )]
    let read_back = Bounded::<ParticipantVolume>::new(probe.volume() as f32).unwrap_or_default();
    VolumeSupport::read(asked, read_back)
}

/// Sets the volume of `member` and keeps it on this browser alone (the Murmure calls spec,
/// section 3, the volumes). Answers each slot that carries the member, with the volume it now plays at.
pub(crate) fn set(
    member: Id<User>,
    volume: Bounded<ParticipantVolume>,
) -> Vec<(Pos<AudioSlots>, Bounded<ParticipantVolume>)> {
    let stored = VOLUMES.with(|volumes| {
        volumes.update(|volumes| volumes.set(member, volume));
        volumes.get_untracked().to_stored()
    });
    if let Some(storage) = storage() {
        let _ = storage.set_item(Volumes::storage_key().as_str(), &stored);
    }
    PLAYOUT
        .with(|playout| playout.borrow().slots_of(member))
        .into_iter()
        .map(|slot| (slot, volume))
        .collect()
}

/// Keeps `gain` on this browser alone. The automatic gain removes the entry.
pub(crate) fn keep_gain(gain: Gain) {
    if let Some(storage) = storage() {
        let key = Gain::storage_key();
        let _ = match gain.to_stored() {
            Some(text) => storage.set_item(key.as_str(), &text),
            None => storage.remove_item(key.as_str()),
        };
    }
    GAIN.with(|signal| signal.set(gain));
}

/// Reads a note of the call worker: the slot carries a new sender. Answers the volume the slot
/// plays at from now on (`CALL-088`).
pub(crate) fn carried(note: SlotSender) -> Bounded<ParticipantVolume> {
    let volumes = VOLUMES.with(GetUntracked::get_untracked);
    PLAYOUT.with(|playout| playout.borrow_mut().carries(note, &volumes))
}

/// The volume the slot `slot` plays at.
pub(crate) fn volume_of(slot: Pos<AudioSlots>) -> Bounded<ParticipantVolume> {
    let volumes = VOLUMES.with(GetUntracked::get_untracked);
    PLAYOUT.with(|playout| playout.borrow().volume_of(slot, &volumes))
}

/// The Sync worker admitted `sender`, of `member`. Answers each slot that carries it, with the
/// volume of that member (`CALL-087`).
pub(crate) fn admitted(
    sender: Seq<SenderIndex>,
    member: Id<User>,
) -> Vec<(Pos<AudioSlots>, Bounded<ParticipantVolume>)> {
    let volume = VOLUMES.with(|volumes| volumes.get_untracked().of(member));
    let slots = PLAYOUT.with(|playout| playout.borrow_mut().admitted(sender, member));
    publish_participants();
    slots.into_iter().map(|slot| (slot, volume)).collect()
}

/// The session of `sender` left the call.
pub(crate) fn departed(sender: Seq<SenderIndex>) {
    PLAYOUT.with(|playout| playout.borrow_mut().departed(sender));
    publish_participants();
}

/// Forgets the slots and the senders of the call that ended.
pub(crate) fn forget() {
    PLAYOUT.with(|playout| *playout.borrow_mut() = Playout::default());
    publish_participants();
    quiet_meter();
}

/// Hands the members in the call to the panel.
fn publish_participants() {
    let members = PLAYOUT.with(|playout| playout.borrow().participants());
    PARTICIPANTS.with(|signal| signal.set(members));
}

/// The graph of the send side. The capture feeds a gain, then the analyser of the meter. The
/// analyser feeds the track the sender takes under a gain set by hand. The graph never touches
/// the playout (`ambre-spec-call-media.md section 3`, the gain).
pub(crate) struct GainGraph {
    context: web_sys::AudioContext,
    /// The source of the current capture track, which a change of microphone replaces.
    source: Option<web_sys::MediaStreamAudioSourceNode>,
    gain: web_sys::GainNode,
    analyser: web_sys::AnalyserNode,
    destination: web_sys::MediaStreamAudioDestinationNode,
}

impl GainGraph {
    /// A graph at `gain`, fed by no capture yet. A context starts suspended outside a gesture,
    /// so the graph asks it to run.
    pub(crate) fn new(gain: Gain) -> Option<Self> {
        let context = web_sys::AudioContext::new().ok()?;
        let gain_node = context.create_gain().ok()?;
        let analyser = context.create_analyser().ok()?;
        let destination = context.create_media_stream_destination().ok()?;
        // The analyser passes its input on unchanged. In line before the destination, the
        // destination pulls it: Firefox reads nothing from an analyser that no node pulls.
        gain_node.connect_with_audio_node(&analyser).ok()?;
        analyser.connect_with_audio_node(&destination).ok()?;
        let graph = Self {
            context,
            source: None,
            gain: gain_node,
            analyser,
            destination,
        };
        graph.set(gain);
        let _ = graph.context.resume();
        Some(graph)
    }

    /// Sets the factor of the gain node.
    pub(crate) fn set(&self, gain: Gain) {
        self.gain.gain().set_value(gain.factor().get());
    }

    /// Feeds `track`, a capture track, to the gain in place of the one before. The browser
    /// cancelled the echo of it already, inside the capture (`CALL-091`).
    pub(crate) fn feed(&mut self, track: &web_sys::MediaStreamTrack) {
        if let Some(old) = self.source.take() {
            let _ = old.disconnect();
        }
        let Ok(stream) = web_sys::MediaStream::new() else {
            return;
        };
        stream.add_track(track);
        self.source = self
            .context
            .create_media_stream_source(&stream)
            .ok()
            .filter(|source| source.connect_with_audio_node(&self.gain).is_ok());
    }

    /// The track the sender takes under a gain set by hand: the output of the gain.
    pub(crate) fn track(&self) -> Option<web_sys::MediaStreamTrack> {
        self.destination
            .stream()
            .get_audio_tracks()
            .get(0)
            .dyn_into()
            .ok()
    }

    /// Asks the context to run, from a gesture of the member.
    pub(crate) fn wake(&self) {
        if self.context.state() != web_sys::AudioContextState::Running {
            let _ = self.context.resume();
        }
    }

    /// The level of the last window the analyser read. The samples cross through an array of
    /// the JavaScript heap. The memory of the Main instance is shared between its threads, and
    /// the analyser refuses a view of shared memory.
    fn level(&self) -> Bounded<MeterLevel> {
        let window = js_sys::Float32Array::new_with_length(self.analyser.fft_size());
        self.analyser
            .get_float_time_domain_data_with_f32_array(&window);
        ambre_murmure::call_volumes::level(&window.to_vec())
    }

    /// Ends the graph with its call.
    pub(crate) fn close(&self) {
        let _ = self.context.close();
    }
}

/// Reads the meter at the interval of the call microphone meter row of `ambre-spec-budgets.md
/// section 1`. Answers the handle of the reads, which the panel that shows the meter clears
/// when it goes. Each panel holds its own, so a panel drawn again stops none of another.
pub(crate) fn start_meter() -> Option<leptos::prelude::IntervalHandle> {
    let tick = || {
        if let Some(read) = crate::call::with_graph(GainGraph::level) {
            LEVEL.with(|level| level.set(read));
        }
    };
    leptos::prelude::set_interval_with_handle(tick, METER_INTERVAL.to_duration()).ok()
}

/// Shows silence on the meter once no panel reads it.
fn quiet_meter() {
    LEVEL.with(|level| level.set(Bounded::<MeterLevel>::MIN));
}

/// The interval between two readings of the meter: the call microphone meter row of
/// `ambre-spec-budgets.md section 1`.
const METER_INTERVAL: ambre_core::TimeSpan<ambre_core::Ms> = ambre_core::TimeSpan::new(100);
