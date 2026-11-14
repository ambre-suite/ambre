//! The microphone and the speaker of a call (the Murmure calls spec, section 3, the devices).
//!
//! The page reads the device list from the browser and holds the tracks. This module decides:
//! * what the device panel shows;
//! * which device a call opens from the stored choice;
//! * what the header says when a chosen device goes away.
//!
//! It runs on the host, so its tests need no browser.
//!
//! A device identifier and a label are [`Opaque`] texts with no serde form. So no frame of the
//! bus or of a socket can carry one, and a log prints their length alone (`ZK-026`).
//!
//! ```compile_fail,E0277
//! use ambre_core::Opaque;
//! use ambre_murmure::ui::call_devices::DeviceIds;
//! let _ = serde_json::to_string(&Opaque::<DeviceIds>::new("4caHma"));
//! ```
//!
//! ```compile_fail,E0277
//! use ambre_core::Opaque;
//! use ambre_murmure::ui::call_devices::DeviceLabels;
//! let _ = serde_json::to_string(&Opaque::<DeviceLabels>::new("USB headset"));
//! ```

use ambre_core::{Opaque, OpaqueKind, OpaqueLength, StaticKey, StorageKeys};

/// The kind of the identifier the browser gives a capture or an output device:
/// `Opaque<DeviceIds>`. It has no serde form, so it never leaves the page (`ZK-026`).
pub enum DeviceIds {}

impl OpaqueKind for DeviceIds {
    const NAME: ambre_core::StaticKey<ambre_core::DebugNames> =
        ambre_core::StaticKey::new("DeviceId");
    const LENGTH: OpaqueLength = OpaqueLength::Bytes;
}

/// The kind of the label the browser gives a device, its brand and model:
/// `Opaque<DeviceLabels>`. The panel shows it, and nothing else reads it (`ZK-026`).
pub enum DeviceLabels {}

impl OpaqueKind for DeviceLabels {
    const NAME: ambre_core::StaticKey<ambre_core::DebugNames> =
        ambre_core::StaticKey::new("DeviceLabel");
    const LENGTH: OpaqueLength = OpaqueLength::Chars;
}

/// The local storage key of the chosen microphone.
const MICROPHONE_KEY: StaticKey<StorageKeys> = StaticKey::new("ambre.call.microphone");

/// The local storage key of the chosen speaker.
const SPEAKER_KEY: StaticKey<StorageKeys> = StaticKey::new("ambre.call.speaker");

/// The two kinds of device a call uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceKind {
    /// A capture device of sound.
    Microphone,
    /// An output device of sound.
    Speaker,
}

impl DeviceKind {
    /// The key of the local storage that holds the choice of this kind. The choice stays on the
    /// device, and no document holds it (`MURMURE-612`, `MURMURE-613`).
    pub fn storage_key(self) -> StaticKey<StorageKeys> {
        match self {
            Self::Microphone => MICROPHONE_KEY,
            Self::Speaker => SPEAKER_KEY,
        }
    }
}

/// One device the browser lists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Device {
    /// Whether it captures or plays.
    pub kind: DeviceKind,
    /// Its identifier, empty before the browser grants the microphone.
    pub id: Opaque<DeviceIds>,
    /// Its label, empty before the browser grants the microphone.
    pub label: Opaque<DeviceLabels>,
}

impl Device {
    /// Whether the browser names the device: before a grant, it lists one device of each kind
    /// with no identifier and no label.
    pub fn is_named(&self) -> bool {
        !self.id.as_str().is_empty() && !self.label.as_str().is_empty()
    }
}

/// The device a member chose for one kind.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum DeviceChoice {
    /// The device the browser and the system pick.
    #[default]
    Default,
    /// The device of this identifier.
    Chosen(Opaque<DeviceIds>),
}

impl DeviceChoice {
    /// The choice that the local storage holds as `stored`. No entry, or an empty one, is the
    /// default device.
    pub fn from_stored(stored: Option<Opaque<DeviceIds>>) -> Self {
        stored
            .filter(|id| !id.as_str().is_empty())
            .map_or(Self::Default, Self::Chosen)
    }

    /// The identifier the local storage keeps, `None` for the default device, whose entry goes.
    pub fn stored(&self) -> Option<&Opaque<DeviceIds>> {
        match self {
            Self::Default => None,
            Self::Chosen(id) => Some(id),
        }
    }

    /// The device a call opens for this choice, given the devices of its kind the browser
    /// `listed` now. A chosen device the browser no longer lists gives way to the default one
    /// (`CALL-081`, `CALL-083`).
    pub fn resolve<'a>(&'a self, listed: &[Device]) -> Resolved<'a> {
        match self {
            Self::Default => Resolved::Default,
            Self::Chosen(id) if listed.iter().any(|device| device.id == *id) => {
                Resolved::Chosen(id)
            }
            Self::Chosen(_) => Resolved::Gone,
        }
    }
}

/// The device a call opens, read from a choice and the list of the browser.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resolved<'a> {
    /// The default device, as chosen.
    Default,
    /// The chosen device, which the browser lists.
    Chosen(&'a Opaque<DeviceIds>),
    /// The chosen device went away: the default one serves, and the header says so
    /// (`MURMURE-610`).
    Gone,
}

/// Whether the browser can send the audio of an element to a chosen output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Routing {
    /// The system picks the output: no element has `setSinkId`.
    Fixed,
    /// An element plays on the output its `setSinkId` names.
    Chosen,
}

/// One entry of a choice in the device panel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Entry {
    /// The default device of the kind.
    Default,
    /// A device the browser names.
    Device(Device),
}

impl Entry {
    /// The choice this entry makes.
    pub fn choice(&self) -> DeviceChoice {
        match self {
            Self::Default => DeviceChoice::Default,
            Self::Device(device) => DeviceChoice::Chosen(device.id.clone()),
        }
    }

    /// The text of the entry: the label of the browser, or the default device of `kind`.
    pub fn label(&self, kind: DeviceKind) -> &str {
        match (self, kind) {
            (Self::Device(device), _) => device.label.as_str(),
            (Self::Default, DeviceKind::Microphone) => "Default microphone",
            (Self::Default, DeviceKind::Speaker) => "Default speaker",
        }
    }
}

/// What the device panel shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Panel {
    /// The choice of microphone: the default one, then each device the browser names
    /// (`MURMURE-607`).
    pub microphones: Vec<Entry>,
    /// The choice of speaker, `None` where the browser cannot route a call (`MURMURE-609`).
    pub speakers: Option<Vec<Entry>>,
    /// Whether the browser has granted the microphone, which names the devices.
    pub names: Names,
}

/// Whether the browser names the devices it lists.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Names {
    /// The browser lists no name before the grant of the microphone (`MURMURE-608`).
    Hidden,
    /// Each device shows by its label.
    Shown,
}

impl Names {
    /// What the panel says while the names are hidden.
    pub fn hint(self) -> Option<&'static str> {
        match self {
            Self::Hidden => Some("Device names show once the microphone is allowed"),
            Self::Shown => None,
        }
    }
}

impl Panel {
    /// The panel for the devices the browser `listed`, on a browser that routes as `routing`.
    ///
    /// Before a grant of the microphone, the browser lists devices with no identifier and no
    /// label. The panel then shows the default microphone alone, and the hint (`MURMURE-608`).
    /// The choice of speaker shows where an element routes and the browser lists an output by
    /// its identifier, so no control does nothing (`MURMURE-609`).
    pub fn of(listed: &[Device], routing: Routing) -> Self {
        let microphones: Vec<&Device> = listed
            .iter()
            .filter(|device| device.kind == DeviceKind::Microphone)
            .collect();
        let names = if microphones.iter().all(|device| device.is_named()) {
            Names::Shown
        } else {
            Names::Hidden
        };
        let entries = |kind: DeviceKind| -> Vec<Entry> {
            std::iter::once(Entry::Default)
                .chain(
                    listed
                        .iter()
                        .filter(|device| device.kind == kind && device.is_named())
                        .cloned()
                        .map(Entry::Device),
                )
                .collect()
        };
        let microphones = match names {
            Names::Shown => entries(DeviceKind::Microphone),
            Names::Hidden => vec![Entry::Default],
        };
        let routes_to_a_named_output = routing == Routing::Chosen
            && names == Names::Shown
            && listed
                .iter()
                .any(|device| device.kind == DeviceKind::Speaker && device.is_named());
        Self {
            microphones,
            speakers: routes_to_a_named_output.then(|| entries(DeviceKind::Speaker)),
            names,
        }
    }
}

/// A chosen device that went away during a call, which the default one replaces
/// (`MURMURE-610`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fallback {
    /// The chosen microphone went away: the call sends the default one (`CALL-081`).
    Microphone,
    /// The chosen speaker went away: the call plays on the default output (`CALL-083`).
    Speaker,
}

impl Fallback {
    /// What the header tells the user, until a choice or the end of the call.
    pub fn notice(self) -> &'static str {
        match self {
            Self::Microphone => "Microphone gone: the call sends the default microphone",
            Self::Speaker => "Speaker gone: the call plays on the default speaker",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device(kind: DeviceKind, id: &str, label: &str) -> Device {
        Device {
            kind,
            id: Opaque::new(id),
            label: Opaque::new(label),
        }
    }

    /// `MURMURE-607`, `MURMURE-609`: a browser that names its devices shows the default
    /// microphone, then each one. A browser that routes shows the speakers the same way, and
    /// one that cannot route shows no choice of speaker.
    #[test]
    fn the_panel_lists_each_named_device_after_the_default_one() {
        let listed = [
            device(DeviceKind::Microphone, "m1", "Desk microphone"),
            device(DeviceKind::Speaker, "s1", "Desk speaker"),
            device(DeviceKind::Microphone, "m2", "USB headset"),
        ];
        let panel = Panel::of(&listed, Routing::Chosen);
        assert_eq!(panel.names, Names::Shown);
        assert_eq!(panel.names.hint(), None);
        let labels = |entries: &[Entry], kind| -> Vec<String> {
            entries.iter().map(|e| e.label(kind).to_owned()).collect()
        };
        assert_eq!(
            labels(&panel.microphones, DeviceKind::Microphone),
            ["Default microphone", "Desk microphone", "USB headset"]
        );
        let speakers = panel
            .speakers
            .expect("a browser that routes offers speakers");
        assert_eq!(
            labels(&speakers, DeviceKind::Speaker),
            ["Default speaker", "Desk speaker"]
        );
        assert_eq!(
            panel.microphones[2].choice(),
            DeviceChoice::Chosen(Opaque::new("m2"))
        );
        assert_eq!(panel.microphones[0].choice(), DeviceChoice::Default);

        // No `setSinkId`, or no output by its identifier: no control that does nothing.
        assert_eq!(Panel::of(&listed, Routing::Fixed).speakers, None);
        assert_eq!(Panel::of(&listed[..1], Routing::Chosen).speakers, None);
    }

    /// `MURMURE-608`: before the grant of the microphone, the browser lists devices with no
    /// identifier and no label. The panel shows the default microphone alone, and says why.
    #[test]
    fn before_the_grant_the_panel_shows_the_default_microphone_alone_and_says_why() {
        let listed = [
            device(DeviceKind::Microphone, "", ""),
            device(DeviceKind::Speaker, "", ""),
        ];
        let panel = Panel::of(&listed, Routing::Chosen);
        assert_eq!(panel.microphones, [Entry::Default]);
        assert_eq!(panel.names, Names::Hidden);
        assert!(panel.names.hint().is_some());
        assert_eq!(panel.speakers, None);
        // An empty list hides nothing: the default one serves.
        assert_eq!(
            Panel::of(&[], Routing::Chosen).microphones,
            [Entry::Default]
        );
    }

    /// `CALL-081`, `CALL-083`: a chosen device the browser no longer lists gives
    /// way to the default one, and reads as gone. The default choice never does.
    #[test]
    fn a_chosen_device_that_went_away_resolves_to_the_default_one() {
        let listed = [device(DeviceKind::Microphone, "m1", "Desk microphone")];
        let chosen = DeviceChoice::Chosen(Opaque::new("m1"));
        assert_eq!(
            chosen.resolve(&listed),
            Resolved::Chosen(&Opaque::new("m1"))
        );
        assert_eq!(chosen.resolve(&[]), Resolved::Gone);
        assert_eq!(DeviceChoice::Default.resolve(&[]), Resolved::Default);
        assert_ne!(Fallback::Microphone.notice(), Fallback::Speaker.notice());
        assert!(Fallback::Microphone.notice().contains("default"));
        assert!(Fallback::Speaker.notice().contains("default"));
    }

    /// `MURMURE-612`: the choice reads back from the local storage as it was stored, and the
    /// default one stores nothing.
    #[test]
    fn a_choice_reads_back_from_the_local_storage() {
        let chosen = DeviceChoice::Chosen(Opaque::new("m2"));
        assert_eq!(DeviceChoice::from_stored(chosen.stored().cloned()), chosen);
        assert_eq!(DeviceChoice::Default.stored(), None);
        assert_eq!(DeviceChoice::from_stored(None), DeviceChoice::Default);
        assert_eq!(
            DeviceChoice::from_stored(Some(Opaque::default())),
            DeviceChoice::Default
        );
        assert_ne!(
            DeviceKind::Microphone.storage_key().as_str(),
            DeviceKind::Speaker.storage_key().as_str()
        );
    }
}
