//! The controls of a call in the header of a channel (the Murmure calls spec, section 3, the
//! call controls).
//!
//! The page holds the microphone and the peer connection, so the phase of its call lives
//! there. This module decides what the header offers in each phase, and what it tells the
//! user when the browser refuses the microphone. It runs on the host, so its tests need no
//! browser.

use ambre_core::id::{Channel, Id};

/// Why the browser gave the page no microphone (`MURMURE-602`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MicrophoneRefusal {
    /// The user, or a policy of the page, refused the capture.
    Blocked,
    /// The device holds no microphone.
    Missing,
    /// Any other failure: a device in use, or a browser with no capture.
    Unavailable,
}

impl MicrophoneRefusal {
    /// The refusal that the name of a capture error states. The name is the one the browser
    /// gives the `DOMException` of `getUserMedia`. A name this function does not know reads as
    /// `Unavailable`, so every failure tells the user something.
    pub fn from_error_name(name: &str) -> Self {
        match name {
            "NotAllowedError" | "SecurityError" => Self::Blocked,
            "NotFoundError" | "OverconstrainedError" => Self::Missing,
            _ => Self::Unavailable,
        }
    }

    /// What the header tells the user: that the call did not start, and why.
    pub fn notice(self) -> &'static str {
        match self {
            Self::Blocked => "Microphone blocked: the call did not start",
            Self::Missing => "No microphone found: the call did not start",
            Self::Unavailable => "The microphone did not open: the call did not start",
        }
    }
}

/// Whether the microphone of the page sends its sound into the call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Microphone {
    /// The others hear the member.
    Live,
    /// The others hear silence from the member (`CALL-074`).
    Muted,
}

impl Microphone {
    /// The other state: a press of mute.
    #[must_use]
    pub fn toggled(self) -> Self {
        match self {
            Self::Live => Self::Muted,
            Self::Muted => Self::Live,
        }
    }
}

/// Why the browser gave the call no camera (`MURMURE-623`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CameraRefusal {
    /// The user, or a policy of the page, refused the capture.
    Blocked,
    /// The device holds no camera.
    Missing,
    /// Any other failure: a camera in use, or a browser with no capture.
    Unavailable,
}

impl CameraRefusal {
    /// The refusal that the name of a capture error states, read as for a microphone. A name
    /// this function does not know reads as `Unavailable`.
    pub fn from_error_name(name: &str) -> Self {
        match MicrophoneRefusal::from_error_name(name) {
            MicrophoneRefusal::Blocked => Self::Blocked,
            MicrophoneRefusal::Missing => Self::Missing,
            MicrophoneRefusal::Unavailable => Self::Unavailable,
        }
    }

    /// What the header tells the user: that the camera did not open, why, and that the call
    /// goes on.
    pub fn notice(self) -> &'static str {
        match self {
            Self::Blocked => "Camera blocked: the call goes on with its audio",
            Self::Missing => "No camera found: the call goes on with its audio",
            Self::Unavailable => "The camera did not open: the call goes on with its audio",
        }
    }
}

/// The camera of the call of the page (the Murmure calls spec, section 3, the camera).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Camera {
    /// No camera goes into the call.
    #[default]
    Off,
    /// A press waits for the answer of the browser on the camera.
    Asking,
    /// The camera goes into the call.
    On,
    /// The last press found no camera: the call goes on with its audio (`MURMURE-623`).
    Refused(CameraRefusal),
}

/// What a press of the camera button does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CameraPress {
    /// The press opens the camera.
    Open,
    /// The press stops the camera.
    Stop,
    /// The press does nothing: the browser is still asked.
    Wait,
}

impl Camera {
    /// What a press of the camera button does in this state (`MURMURE-622`).
    pub fn press(self) -> CameraPress {
        match self {
            Self::Off | Self::Refused(_) => CameraPress::Open,
            Self::On => CameraPress::Stop,
            Self::Asking => CameraPress::Wait,
        }
    }

    /// Whether the button reads pressed: the camera goes into the call (`MURMURE-624`).
    pub fn sends(self) -> bool {
        self == Self::On
    }

    /// The refusal the header shows, until the next press or the end of the call.
    pub fn refusal(self) -> Option<CameraRefusal> {
        match self {
            Self::Refused(refusal) => Some(refusal),
            Self::Off | Self::Asking | Self::On => None,
        }
    }

    /// The accessible name of the button: what a press does.
    pub fn label(self) -> &'static str {
        match self.press() {
            CameraPress::Stop => "Turn the camera off",
            CameraPress::Open | CameraPress::Wait => "Turn the camera on",
        }
    }
}

/// The phase of the call of the page. A page holds one call at a time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CallPhase {
    /// No call, and nothing to tell.
    #[default]
    Idle,
    /// A call gesture waits for the answer of the browser on the microphone.
    Asking,
    /// The last call gesture found no microphone: no call started (`MURMURE-601`).
    Refused(MicrophoneRefusal),
    /// The page is in a call.
    InCall(Microphone),
}

impl CallPhase {
    /// The refusal the header shows, until the next call gesture (`MURMURE-602`).
    pub fn refusal(self) -> Option<MicrophoneRefusal> {
        match self {
            Self::Refused(refusal) => Some(refusal),
            Self::Idle | Self::Asking | Self::InCall(_) => None,
        }
    }

    /// Whether a call gesture may open a call now: the page is in none, and asks for none.
    pub fn accepts_a_gesture(self) -> bool {
        matches!(self, Self::Idle | Self::Refused(_))
    }
}

/// What the header offers for a call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallControls {
    /// No control: no channel is shown, or the browser is asked for the microphone.
    None,
    /// The call button, which starts a call in this channel (`MURMURE-599`).
    Start(Id<Channel>),
    /// Mute, in this state, and hang up (`MURMURE-603`).
    During(Microphone),
}

impl CallControls {
    /// What the header offers in `phase`, on a pane that shows `shown`. A page in a call offers
    /// mute and hang up whatever channel it shows, since the call goes on in any pane.
    pub fn of(phase: CallPhase, shown: Option<Id<Channel>>) -> Self {
        match (phase, shown) {
            (CallPhase::InCall(microphone), _) => Self::During(microphone),
            (CallPhase::Asking, _) | (CallPhase::Idle | CallPhase::Refused(_), None) => Self::None,
            (CallPhase::Idle | CallPhase::Refused(_), Some(channel)) => Self::Start(channel),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `MURMURE-599`, `MURMURE-603`: a shown channel offers the call button, and a call offers
    /// mute and hang up in its place, whatever the pane shows.
    #[test]
    fn a_channel_offers_the_call_button_and_a_call_offers_mute_and_hang_up_in_its_place() {
        let channel = Id::<Channel>::mint();
        assert_eq!(
            CallControls::of(CallPhase::Idle, Some(channel)),
            CallControls::Start(channel)
        );
        assert_eq!(CallControls::of(CallPhase::Idle, None), CallControls::None);
        for shown in [Some(channel), None] {
            for microphone in [Microphone::Live, Microphone::Muted] {
                assert_eq!(
                    CallControls::of(CallPhase::InCall(microphone), shown),
                    CallControls::During(microphone)
                );
            }
            // A gesture that waits for the microphone offers no second one.
            assert_eq!(
                CallControls::of(CallPhase::Asking, shown),
                CallControls::None
            );
        }
    }

    /// `MURMURE-622`, `MURMURE-624`: a press opens the camera, a second stops it, and a press
    /// while the browser is asked does nothing. The button reads pressed while the camera goes
    /// into the call, and its name says what a press does.
    #[test]
    fn a_press_opens_the_camera_and_a_second_stops_it() {
        assert_eq!(Camera::default(), Camera::Off);
        assert_eq!(Camera::Off.press(), CameraPress::Open);
        assert_eq!(Camera::On.press(), CameraPress::Stop);
        assert_eq!(Camera::Asking.press(), CameraPress::Wait);
        assert!(Camera::On.sends());
        for camera in [
            Camera::Off,
            Camera::Asking,
            Camera::Refused(CameraRefusal::Blocked),
        ] {
            assert!(!camera.sends(), "{camera:?}");
            assert_eq!(camera.label(), "Turn the camera on");
        }
        assert_eq!(Camera::On.label(), "Turn the camera off");
    }

    /// `MURMURE-623`: a refused camera says why and that the call goes on, and a press may try
    /// again. Each error the browser names tells the user why.
    #[test]
    fn a_refused_camera_says_why_and_keeps_the_call() {
        let cases = [
            ("NotAllowedError", CameraRefusal::Blocked),
            ("SecurityError", CameraRefusal::Blocked),
            ("NotFoundError", CameraRefusal::Missing),
            ("OverconstrainedError", CameraRefusal::Missing),
            ("NotReadableError", CameraRefusal::Unavailable),
            ("", CameraRefusal::Unavailable),
        ];
        for (name, refusal) in cases {
            assert_eq!(CameraRefusal::from_error_name(name), refusal, "{name}");
            let camera = Camera::Refused(refusal);
            assert_eq!(camera.refusal(), Some(refusal));
            assert_eq!(camera.press(), CameraPress::Open);
            assert!(
                refusal.notice().contains("goes on with its audio"),
                "{refusal:?}"
            );
        }
        assert_ne!(
            CameraRefusal::Blocked.notice(),
            CameraRefusal::Missing.notice()
        );
        assert_eq!(Camera::On.refusal(), None);
    }

    /// `MURMURE-602`: a refusal stays in view with the call button, so the user can try again.
    /// Each error the browser names tells the user why.
    #[test]
    fn a_refusal_says_why_and_leaves_the_call_button() {
        let channel = Id::<Channel>::mint();
        let cases = [
            ("NotAllowedError", MicrophoneRefusal::Blocked),
            ("SecurityError", MicrophoneRefusal::Blocked),
            ("NotFoundError", MicrophoneRefusal::Missing),
            ("OverconstrainedError", MicrophoneRefusal::Missing),
            ("NotReadableError", MicrophoneRefusal::Unavailable),
            ("", MicrophoneRefusal::Unavailable),
        ];
        for (name, refusal) in cases {
            assert_eq!(MicrophoneRefusal::from_error_name(name), refusal, "{name}");
            let phase = CallPhase::Refused(refusal);
            assert_eq!(phase.refusal(), Some(refusal));
            assert!(phase.accepts_a_gesture());
            assert_eq!(
                CallControls::of(phase, Some(channel)),
                CallControls::Start(channel)
            );
            assert!(refusal.notice().contains("did not start"), "{refusal:?}");
        }
        assert_ne!(
            MicrophoneRefusal::Blocked.notice(),
            MicrophoneRefusal::Missing.notice()
        );
        assert!(!CallPhase::Asking.accepts_a_gesture());
        assert!(!CallPhase::InCall(Microphone::Live).accepts_a_gesture());
        assert_eq!(Microphone::Live.toggled(), Microphone::Muted);
        assert_eq!(Microphone::Muted.toggled(), Microphone::Live);
    }
}
