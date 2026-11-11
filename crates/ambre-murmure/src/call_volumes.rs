//! The volumes of a call (the Murmure calls spec, section 3, the volumes, and
//! `ambre-spec-call-media.md section 3`, the volumes and the gain).
//!
//! The page holds the audio elements, the graph of the gain and the storage. This module decides:
//! * which volume each audio slot plays at, from the sender it carries and the member of that
//!   sender;
//! * what the storage of the browser keeps of the volumes and of the gain;
//! * what the meter shows of a window of samples;
//! * whether the panel offers a volume for a participant at all.
//!
//! It runs on the host, so its tests need no browser. Nothing here reaches a worker: a volume
//! and a gain stay on the page and in the storage of this browser (`ZK-027`).

use std::collections::{BTreeMap, HashMap};

use ambre_core::id::{Id, User};
use ambre_core::index::Pos;
use ambre_core::{
    AudioSlots, Bounded, MeterLevel, MicrophoneGain, ParticipantVolume, SenderIndex, Seq,
    SlotSender, StaticKey, StorageKeys,
};

/// The local storage key of the volume of each member.
const VOLUMES_KEY: StaticKey<StorageKeys> = StaticKey::new("ambre.call.volumes");

/// The local storage key of the gain of the microphone, absent for the automatic gain.
const GAIN_KEY: StaticKey<StorageKeys> = StaticKey::new("ambre.call.gain");

/// Whether an audio element of this browser takes the volume the page sets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VolumeSupport {
    /// The volume of an element is read-only, as on iOS: the panel offers no volume for a
    /// participant (`MURMURE-615`).
    ReadOnly,
    /// The element plays at the volume the page sets.
    Writable,
}

impl VolumeSupport {
    /// What the page read when it set a probe element to `asked`: the volume read back.
    #[must_use]
    pub fn read(asked: Bounded<ParticipantVolume>, read_back: Bounded<ParticipantVolume>) -> Self {
        if asked == read_back {
            Self::Writable
        } else {
            Self::ReadOnly
        }
    }

    /// Whether the panel offers a volume for each participant (`MURMURE-614`, `MURMURE-615`).
    #[must_use]
    pub fn offers_volumes(self) -> bool {
        self == Self::Writable
    }
}

/// The volume of each member this browser set, by member. A member with no entry plays at the
/// full level of its element.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Volumes {
    by_member: HashMap<Id<User>, Bounded<ParticipantVolume>>,
}

impl Volumes {
    /// The key of the local storage that holds the volumes. They stay on the device, and no
    /// document holds them (`MURMURE-618`, `MURMURE-619`).
    #[must_use]
    pub const fn storage_key() -> StaticKey<StorageKeys> {
        VOLUMES_KEY
    }

    /// The volumes the local storage holds as `stored`. No entry, or an entry that does not
    /// parse, is no volume set.
    #[must_use]
    pub fn from_stored(stored: Option<&str>) -> Self {
        let pairs: Vec<(Id<User>, Bounded<ParticipantVolume>)> = stored
            .and_then(|text| serde_json::from_str(text).ok())
            .unwrap_or_default();
        Self {
            by_member: pairs.into_iter().collect(),
        }
    }

    /// The text the local storage keeps.
    #[must_use]
    pub fn to_stored(&self) -> String {
        let pairs: Vec<(&Id<User>, &Bounded<ParticipantVolume>)> = self.by_member.iter().collect();
        serde_json::to_string(&pairs).unwrap_or_default()
    }

    /// The volume of `member`.
    #[must_use]
    pub fn of(&self, member: Id<User>) -> Bounded<ParticipantVolume> {
        self.by_member.get(&member).copied().unwrap_or_default()
    }

    /// Sets the volume of `member`. The full level removes its entry.
    pub fn set(&mut self, member: Id<User>, volume: Bounded<ParticipantVolume>) {
        if volume == Bounded::<ParticipantVolume>::FULL {
            self.by_member.remove(&member);
        } else {
            self.by_member.insert(member, volume);
        }
    }
}

/// The gain of the microphone.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum Gain {
    /// The automatic gain of the browser sets the level.
    #[default]
    Automatic,
    /// A gain set by hand, with the automatic gain off (`CALL-089`).
    ByHand(Bounded<MicrophoneGain>),
}

impl Gain {
    /// The key of the local storage that holds the gain (`MURMURE-618`).
    #[must_use]
    pub const fn storage_key() -> StaticKey<StorageKeys> {
        GAIN_KEY
    }

    /// The gain the local storage holds as `stored`: no entry, or one out of bounds, is the
    /// automatic gain.
    #[must_use]
    pub fn from_stored(stored: Option<&str>) -> Self {
        stored
            .and_then(|text| serde_json::from_str::<Bounded<MicrophoneGain>>(text).ok())
            .map_or(Self::Automatic, Self::ByHand)
    }

    /// The text the local storage keeps, `None` for the automatic gain, whose entry goes.
    #[must_use]
    pub fn to_stored(self) -> Option<String> {
        match self {
            Self::Automatic => None,
            Self::ByHand(gain) => serde_json::to_string(&gain).ok(),
        }
    }

    /// The factor the graph of the send side applies: the captured level for the automatic
    /// gain, which sends the capture track with no graph.
    #[must_use]
    pub fn factor(self) -> Bounded<MicrophoneGain> {
        match self {
            Self::Automatic => Bounded::<MicrophoneGain>::UNCHANGED,
            Self::ByHand(gain) => gain,
        }
    }
}

/// What the page knows of the audio slots of its call: the sender each slot carries, and the
/// member of each sender index the Sync worker admitted.
#[derive(Debug, Default)]
pub struct Playout {
    slots: BTreeMap<Pos<AudioSlots>, Seq<SenderIndex>>,
    members: BTreeMap<Seq<SenderIndex>, Id<User>>,
}

impl Playout {
    /// Reads a note of the call worker: the slot carries a new sender (`CALL-084`). Answers the
    /// volume the slot plays at from now on, the volume of the member of the new sender
    /// (`CALL-088`). A sender the Sync worker has not named yet plays at the full level, until
    /// [`Playout::admitted`] names it.
    pub fn carries(&mut self, note: SlotSender, volumes: &Volumes) -> Bounded<ParticipantVolume> {
        self.slots.insert(note.slot, note.sender);
        self.volume_of(note.slot, volumes)
    }

    /// The Sync worker admitted `sender`, whose join names `member`. Answers each slot that
    /// carries it, which takes the volume of that member (`CALL-087`).
    pub fn admitted(&mut self, sender: Seq<SenderIndex>, member: Id<User>) -> Vec<Pos<AudioSlots>> {
        self.members.insert(sender, member);
        self.slots_of(member)
    }

    /// The session of `sender` left the call.
    pub fn departed(&mut self, sender: Seq<SenderIndex>) {
        self.members.remove(&sender);
    }

    /// The volume the slot `slot` plays at.
    #[must_use]
    pub fn volume_of(
        &self,
        slot: Pos<AudioSlots>,
        volumes: &Volumes,
    ) -> Bounded<ParticipantVolume> {
        self.slots
            .get(&slot)
            .and_then(|sender| self.members.get(sender))
            .map_or(Bounded::<ParticipantVolume>::FULL, |member| {
                volumes.of(*member)
            })
    }

    /// Each slot that carries a sender of `member`. A member who left and joined again holds
    /// two sender indexes, and its volume follows both (`CALL-087`).
    #[must_use]
    pub fn slots_of(&self, member: Id<User>) -> Vec<Pos<AudioSlots>> {
        self.slots
            .iter()
            .filter(|(_, sender)| self.members.get(sender) == Some(&member))
            .map(|(slot, _)| *slot)
            .collect()
    }

    /// The members in the call, each once, whose volume the panel offers (`MURMURE-614`).
    #[must_use]
    pub fn participants(&self) -> Vec<Id<User>> {
        let mut members: Vec<Id<User>> = Vec::new();
        for member in self.members.values() {
            if !members.contains(member) {
                members.push(*member);
            }
        }
        members
    }
}

/// The volume a slider of the panel spells as `percent`, from 0 to 100, or `None` for a text
/// that names none.
#[must_use]
pub fn volume_from_slider(percent: &str) -> Option<Bounded<ParticipantVolume>> {
    percent
        .parse::<u16>()
        .ok()
        .and_then(|percent| Bounded::new(f32::from(percent) / 100.0))
}

/// The gain a slider of the panel spells as `percent`, from 0 to the cap, or `None` for a text
/// that names none.
#[must_use]
pub fn gain_from_slider(percent: &str) -> Option<Bounded<MicrophoneGain>> {
    percent
        .parse::<u16>()
        .ok()
        .and_then(|percent| Bounded::new(f32::from(percent) / 100.0))
}

/// The percent a slider of the panel shows for `volume`, which [`volume_from_slider`] reads
/// back.
#[must_use]
pub fn slider_of_volume(volume: Bounded<ParticipantVolume>) -> String {
    format!("{:.0}", volume.get() * 100.0)
}

/// The percent a slider of the panel shows for `gain`, which [`gain_from_slider`] reads back.
#[must_use]
pub fn slider_of_gain(gain: Bounded<MicrophoneGain>) -> String {
    format!("{:.0}", gain.get() * 100.0)
}

/// The level a meter shows for a window of `samples`, each in -1 to 1: their root mean square.
/// A sample past full scale reads as full scale, and an empty window as silence.
#[must_use]
pub fn level(samples: &[f32]) -> Bounded<MeterLevel> {
    if samples.is_empty() {
        return Bounded::<MeterLevel>::MIN;
    }
    let squares: f64 = samples
        .iter()
        .map(|sample| f64::from(sample.clamp(-1.0, 1.0)).powi(2))
        .sum();
    #[expect(
        clippy::cast_precision_loss,
        reason = "a window holds a few thousand samples, far below the 2^52 a float holds whole"
    )]
    let mean = squares / samples.len() as f64;
    #[expect(
        clippy::cast_possible_truncation,
        reason = "the root mean square of samples in -1..=1 lies in 0..=1, which an f32 holds"
    )]
    let rms = mean.sqrt() as f32;
    Bounded::new(rms.clamp(0.0, 1.0)).unwrap_or(Bounded::<MeterLevel>::MIN)
}

#[cfg(test)]
mod tests {
    use super::{Gain, Playout, VolumeSupport, Volumes, level};
    use ambre_core::id::{Id, User};
    use ambre_core::index::Pos;
    use ambre_core::{Bounded, MeterLevel, MicrophoneGain, ParticipantVolume, Seq, SlotSender};

    fn volume(value: f32) -> Bounded<ParticipantVolume> {
        Bounded::new(value).unwrap_or_else(|| panic!("a volume of {value}"))
    }

    fn note(slot: u32, sender: u64) -> SlotSender {
        SlotSender {
            slot: Pos::new(slot),
            sender: Seq::new(sender),
        }
    }

    /// `CALL-085`, `CALL-086`, `CALL-087`: a member turned down to nothing plays silence in the
    /// slot that carries it, and in the slot it moves to. The other member keeps its own volume,
    /// in the slot it moves to as well.
    #[test]
    fn a_volume_follows_its_member_from_slot_to_slot() {
        let (bob, carol) = (Id::<User>::mint(), Id::<User>::mint());
        let mut volumes = Volumes::default();
        volumes.set(bob, Bounded::<ParticipantVolume>::SILENT);
        volumes.set(carol, volume(0.5));
        let mut playout = Playout::default();
        assert_eq!(playout.admitted(Seq::new(2), bob), Vec::new());
        playout.admitted(Seq::new(3), carol);
        assert_eq!(
            playout.carries(note(0, 2), &volumes),
            Bounded::<ParticipantVolume>::SILENT
        );
        assert_eq!(playout.carries(note(1, 3), &volumes), volume(0.5));
        // The slots swap their senders.
        assert_eq!(playout.carries(note(0, 3), &volumes), volume(0.5));
        assert_eq!(
            playout.carries(note(1, 2), &volumes),
            Bounded::<ParticipantVolume>::SILENT
        );
        assert_eq!(playout.slots_of(bob), vec![Pos::new(1)]);
        assert_eq!(playout.slots_of(carol), vec![Pos::new(0)]);
    }

    /// `CALL-087`: a member who joins again under a new sender index keeps its volume. A note
    /// that comes before the Sync worker names the sender plays at the full level, and the name
    /// then hands the slot to the volume of its member.
    #[test]
    fn a_new_sender_index_of_one_member_keeps_its_volume() {
        let bob = Id::<User>::mint();
        let mut volumes = Volumes::default();
        volumes.set(bob, Bounded::<ParticipantVolume>::SILENT);
        let mut playout = Playout::default();
        playout.admitted(Seq::new(2), bob);
        playout.carries(note(0, 2), &volumes);
        playout.departed(Seq::new(2));
        assert_eq!(
            playout.carries(note(2, 7), &volumes),
            Bounded::<ParticipantVolume>::FULL
        );
        assert_eq!(playout.admitted(Seq::new(7), bob), vec![Pos::new(2)]);
        assert_eq!(
            playout.volume_of(Pos::new(2), &volumes),
            Bounded::<ParticipantVolume>::SILENT
        );
        assert_eq!(playout.participants(), vec![bob]);
    }

    /// `CALL-088`: a slot takes the volume of its new sender at the note itself, whatever the
    /// former sender played at.
    #[test]
    fn a_slot_takes_the_volume_of_its_new_sender_at_the_note() {
        let (bob, carol) = (Id::<User>::mint(), Id::<User>::mint());
        let mut volumes = Volumes::default();
        volumes.set(bob, Bounded::<ParticipantVolume>::SILENT);
        let mut playout = Playout::default();
        playout.admitted(Seq::new(2), bob);
        playout.admitted(Seq::new(3), carol);
        playout.carries(note(0, 2), &volumes);
        assert_eq!(
            playout.carries(note(0, 3), &volumes),
            Bounded::<ParticipantVolume>::FULL
        );
        assert_eq!(
            playout.carries(note(0, 2), &volumes),
            Bounded::<ParticipantVolume>::SILENT
        );
    }

    /// `MURMURE-618`: the volumes and the gain read back from the local storage. A stored text
    /// that does not parse, or a value out of bounds, is no choice.
    #[test]
    fn the_volumes_and_the_gain_read_back_from_the_local_storage() {
        let bob = Id::<User>::mint();
        let mut volumes = Volumes::default();
        volumes.set(bob, volume(0.25));
        let stored = volumes.to_stored();
        assert_eq!(Volumes::from_stored(Some(&stored)), volumes);
        assert_eq!(Volumes::from_stored(Some("{not json")), Volumes::default());
        assert_eq!(
            Volumes::from_stored(Some(&stored.replace("0.25", "3.0"))),
            Volumes::default(),
            "a volume past full scale"
        );
        volumes.set(bob, Bounded::<ParticipantVolume>::FULL);
        assert_eq!(volumes, Volumes::default(), "the full level keeps no entry");

        let gain = Gain::ByHand(Bounded::new(1.5).unwrap_or_else(|| panic!("a gain")));
        let stored = gain.to_stored();
        assert_eq!(Gain::from_stored(stored.as_deref()), gain);
        assert_eq!(Gain::Automatic.to_stored(), None);
        assert_eq!(Gain::from_stored(None), Gain::Automatic);
        assert_eq!(Gain::from_stored(Some("9")), Gain::Automatic);
    }

    /// The bounds of the call microphone gain row of `ambre-spec-budgets.md section 1`.
    #[test]
    fn a_gain_past_its_cap_is_refused() {
        assert!(Bounded::<MicrophoneGain>::new(2.0).is_some());
        assert!(Bounded::<MicrophoneGain>::new(2.01).is_none());
        assert!(Bounded::<MicrophoneGain>::new(-0.01).is_none());
        assert_eq!(
            Gain::Automatic.factor(),
            Bounded::<MicrophoneGain>::UNCHANGED
        );
    }

    /// `MURMURE-616`: the meter shows the root mean square of a window, silence for none.
    #[test]
    fn the_meter_shows_the_root_mean_square_of_a_window() {
        assert_eq!(level(&[]), Bounded::<MeterLevel>::MIN);
        assert_eq!(level(&[0.0; 64]), Bounded::<MeterLevel>::MIN);
        let full: Vec<f32> = (0..64)
            .map(|i| if i % 2 == 0 { 1.0 } else { -1.0 })
            .collect();
        assert_eq!(level(&full), Bounded::<MeterLevel>::MAX);
        assert_eq!(level(&[3.0, -3.0]), Bounded::<MeterLevel>::MAX);
        let half = level(&[0.5, -0.5, 0.5, -0.5]).get();
        assert!((half - 0.5).abs() < 1e-6, "{half}");
    }

    /// `MURMURE-617`: a slider spells a volume and a gain in percent, and reads them back.
    #[test]
    fn a_slider_spells_a_volume_and_a_gain_in_percent() {
        assert_eq!(
            super::volume_from_slider("0"),
            Some(Bounded::<ParticipantVolume>::SILENT)
        );
        assert_eq!(
            super::volume_from_slider("100"),
            Some(Bounded::<ParticipantVolume>::FULL)
        );
        assert_eq!(super::volume_from_slider("101"), None);
        assert_eq!(super::volume_from_slider("half"), None);
        assert_eq!(
            super::gain_from_slider("200"),
            Bounded::<MicrophoneGain>::new(2.0)
        );
        assert_eq!(super::gain_from_slider("201"), None);
        assert_eq!(super::slider_of_volume(volume(0.25)), "25");
        assert_eq!(super::slider_of_gain(Bounded::<MicrophoneGain>::MAX), "200");
    }

    /// `MURMURE-614`, `MURMURE-615`: a volume read back unchanged offers the volumes, and a
    /// volume the element kept does not.
    #[test]
    fn a_read_only_volume_offers_no_volume_for_a_participant() {
        let asked = volume(0.5);
        assert!(VolumeSupport::read(asked, asked).offers_volumes());
        assert!(!VolumeSupport::read(asked, Bounded::<ParticipantVolume>::FULL).offers_volumes());
    }
}
