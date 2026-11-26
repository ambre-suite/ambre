//! The ring of a call started in a direct message (the Murmure calls spec, section 3, the
//! ring).
//!
//! The page of the other member rings: an alert offers an accept and a decline, and a tone
//! sounds. The page of the caller waits, and its header says so. The ring of each side ends at
//! an answer, a decline or the timeout of the budget row. This module decides each step. It
//! reads no clock and no DOM, so its tests run on the host. The page owns the clock, the tone,
//! the alert and the call.

use ambre_bus::{Authored, CallCue, CueKind, Instant};
use ambre_core::id::{Call, Channel, Id, User};
use ambre_core::{Ms, Name, TimeSpan, Validated};

/// How long a call rings with no answer, the call ring row of `ambre-spec-budgets.md
/// section 1` (`MURMURE-643`).
pub const RING_TIMEOUT: TimeSpan<Ms> = TimeSpan::new(30_000);

/// Whether the channel of a cue is a direct message. A call of any other channel rings no
/// page: its members join from the call message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Conversation {
    /// A direct message between two members.
    Direct,
    /// A channel of a space, or a group.
    Channel,
}

/// Whether the page is in a call, or asks for the microphone of one. A page on the line rings
/// for no other call: the call message still shows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Line {
    /// The page is in no call.
    Free,
    /// The page is in a call, or opens one.
    Busy,
}

/// A call that rings on this page, until its accept, its decline or its timeout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Incoming {
    /// The call the call message names.
    pub call: Id<Call>,
    /// The direct message of the call.
    pub channel: Id<Channel>,
    /// The name the sidebar shows for the direct message: the other member.
    pub caller: Option<Validated<Name>>,
    /// The instant the ring stops with no answer.
    pub until: Instant<Authored>,
}

/// A call this page started in a direct message, until the other member answers or declines,
/// or the timeout passes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Outgoing {
    /// The call this page started.
    pub call: Id<Call>,
    /// The direct message of the call.
    pub channel: Id<Channel>,
    /// The instant the page stops waiting and hangs up.
    pub until: Instant<Authored>,
}

/// The ring of the page: none, a call that rings here, or a call that rings elsewhere.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Ring {
    /// Nothing rings.
    #[default]
    Quiet,
    /// A call of another member rings on this page (`MURMURE-641`).
    Incoming(Incoming),
    /// A call of this page rings on the page of the other member.
    Outgoing(Outgoing),
}

/// Why the ring of a call this page started ended with no answer. The header says it until
/// the next call gesture (`MURMURE-645`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unanswered {
    /// The other member declined.
    Declined,
    /// The timeout passed with no answer.
    NoAnswer,
}

impl Unanswered {
    /// What the header tells the caller: that the call ended, and why.
    pub fn notice(self) -> &'static str {
        match self {
            Self::Declined => ambre_commands::tr!("Call declined", "Appel refus\u{e9}"),
            Self::NoAnswer => ambre_commands::tr!(
                "No answer: the call ended",
                "Pas de r\u{e9}ponse\u{a0}: l'appel est termin\u{e9}"
            ),
        }
    }
}

/// What a step of the ring asks the page to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RingAct {
    /// Join the call, as Join call in the menu of its message does: the microphone first
    /// (`CALL-076`).
    Join {
        /// The call to join.
        call: Id<Call>,
        /// Its direct message.
        channel: Id<Channel>,
    },
    /// Write the decline into the call message (`MURMURE-642`).
    Decline {
        /// The call to decline.
        call: Id<Call>,
        /// Its direct message.
        channel: Id<Channel>,
    },
    /// Hang up the call of the page, which nobody answered (`MURMURE-644`).
    HangUp(Unanswered),
}

impl Ring {
    /// Whether a tone sounds and the alert shows: a call rings on this page.
    pub fn rings_here(&self) -> bool {
        matches!(self, Self::Incoming(_))
    }

    /// The instant the ring ends with no answer, `None` while nothing rings.
    pub fn deadline(&self) -> Option<Instant<Authored>> {
        match self {
            Self::Quiet => None,
            Self::Incoming(incoming) => Some(incoming.until),
            Self::Outgoing(outgoing) => Some(outgoing.until),
        }
    }

    /// The ring after the cue `cue` of `channel`, heard at `now` (`MURMURE-641`).
    ///
    /// A ring cue of a direct message rings a free page. A cue older than the timeout rings
    /// nothing: a reconnect replays old cues. A decline or an end of the
    /// call that rings here stops it. A decline of the call this page started hangs it up.
    #[must_use]
    pub fn heard(
        &self,
        channel: Id<Channel>,
        cue: CallCue,
        conversation: Conversation,
        line: Line,
        caller: Option<Validated<Name>>,
        now: Instant<Authored>,
    ) -> (Self, Option<RingAct>) {
        match (self, cue.kind) {
            (Self::Quiet, CueKind::Ring) => {
                let until = cue.at.plus(RING_TIMEOUT);
                let rings = conversation == Conversation::Direct && line == Line::Free;
                if rings && now < until {
                    let incoming = Incoming {
                        call: cue.call,
                        channel,
                        caller,
                        until,
                    };
                    (Self::Incoming(incoming), None)
                } else {
                    (self.clone(), None)
                }
            }
            (Self::Incoming(incoming), CueKind::Decline | CueKind::End)
                if incoming.call == cue.call =>
            {
                (Self::Quiet, None)
            }
            (Self::Outgoing(outgoing), CueKind::Decline) if outgoing.call == cue.call => {
                (Self::Quiet, Some(RingAct::HangUp(Unanswered::Declined)))
            }
            _ => (self.clone(), None),
        }
    }

    /// The ring once the clock reads `now`. A ring past its timeout stops: here with nothing
    /// written, and on the caller with a hang-up (`MURMURE-643`, `MURMURE-644`).
    #[must_use]
    pub fn tick(&self, now: Instant<Authored>) -> (Self, Option<RingAct>) {
        match self {
            Self::Incoming(incoming) if now >= incoming.until => (Self::Quiet, None),
            Self::Outgoing(outgoing) if now >= outgoing.until => {
                (Self::Quiet, Some(RingAct::HangUp(Unanswered::NoAnswer)))
            }
            _ => (self.clone(), None),
        }
    }

    /// The ring once the member presses accept: the page joins the call (`MURMURE-642`).
    #[must_use]
    pub fn accept(&self) -> (Self, Option<RingAct>) {
        match self {
            Self::Incoming(incoming) => (
                Self::Quiet,
                Some(RingAct::Join {
                    call: incoming.call,
                    channel: incoming.channel,
                }),
            ),
            _ => (self.clone(), None),
        }
    }

    /// The ring once the member presses decline: the page writes the decline (`MURMURE-642`).
    #[must_use]
    pub fn decline(&self) -> (Self, Option<RingAct>) {
        match self {
            Self::Incoming(incoming) => (
                Self::Quiet,
                Some(RingAct::Decline {
                    call: incoming.call,
                    channel: incoming.channel,
                }),
            ),
            _ => (self.clone(), None),
        }
    }

    /// The ring once this page started `call` in `channel` at `now`. A start in a direct
    /// message waits for the other member, and any other start waits for nobody.
    #[must_use]
    pub fn started(
        &self,
        call: Id<Call>,
        channel: Id<Channel>,
        conversation: Conversation,
        now: Instant<Authored>,
    ) -> Self {
        match conversation {
            Conversation::Direct => Self::Outgoing(Outgoing {
                call,
                channel,
                until: now.plus(RING_TIMEOUT),
            }),
            Conversation::Channel => self.clone(),
        }
    }

    /// The ring once the call of this page admitted `member`. Another member in the call
    /// answered it, so the caller waits no more. The page itself is no answer.
    #[must_use]
    pub fn admitted(&self, member: Id<User>, me: Option<Id<User>>) -> Self {
        match self {
            Self::Outgoing(_) if Some(member) != me => Self::Quiet,
            _ => self.clone(),
        }
    }

    /// The ring once the call of this page ended, by a hang-up or a leave. A call that ended
    /// waits for nobody.
    #[must_use]
    pub fn hung_up(&self) -> Self {
        match self {
            Self::Outgoing(_) => Self::Quiet,
            _ => self.clone(),
        }
    }
}

/// The command that writes the decline of `call` into the call message of `channel`, stamped
/// `now` (`MURMURE-642`). The Logic thread opens the channel when it is not open.
pub fn decline_command(
    call: Id<Call>,
    channel: Id<Channel>,
    now: Instant<Authored>,
) -> crate::message::LogicCommand {
    crate::message::LogicCommand::Channel {
        channel_id: channel,
        command: crate::message::MurmureCommand::DeclineCall { call, now },
    }
}

/// The text of the ring alert: who calls. A screen reader reads it when the alert shows.
pub fn ring_title(caller: Option<&Validated<Name>>) -> String {
    caller.map_or_else(
        || ambre_commands::tr!("Incoming call", "Appel entrant").to_owned(),
        |name| format!("{} is calling", name.as_str()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const START: Instant<Authored> = Instant::new(1_000_000);

    fn at(ms: u64) -> Instant<Authored> {
        START.plus(TimeSpan::new(ms))
    }

    fn cue(call: Id<Call>, kind: CueKind) -> CallCue {
        CallCue {
            call,
            kind,
            at: START,
        }
    }

    fn incoming(call: Id<Call>, channel: Id<Channel>) -> Ring {
        Ring::Quiet
            .heard(
                channel,
                cue(call, CueKind::Ring),
                Conversation::Direct,
                Line::Free,
                None,
                at(10),
            )
            .0
    }

    /// `MURMURE-641`: a ring cue of a direct message rings a free page until its timeout. A
    /// cue of a channel, a page on the line and a cue older than the timeout ring nothing.
    #[test]
    fn a_ring_cue_of_a_direct_message_rings_a_free_page_until_its_timeout() {
        let (call, channel) = (Id::<Call>::mint(), Id::<Channel>::mint());
        let ring = incoming(call, channel);
        assert_eq!(
            ring,
            Ring::Incoming(Incoming {
                call,
                channel,
                caller: None,
                until: START.plus(RING_TIMEOUT),
            })
        );
        assert!(ring.rings_here());
        let quiet = |conversation, line, now| {
            Ring::Quiet
                .heard(
                    channel,
                    cue(call, CueKind::Ring),
                    conversation,
                    line,
                    None,
                    now,
                )
                .0
        };
        assert_eq!(
            quiet(Conversation::Channel, Line::Free, at(10)),
            Ring::Quiet
        );
        assert_eq!(quiet(Conversation::Direct, Line::Busy, at(10)), Ring::Quiet);
        assert_eq!(
            quiet(Conversation::Direct, Line::Free, at(RING_TIMEOUT.get())),
            Ring::Quiet,
            "a replayed cue rings nothing"
        );
        assert_eq!(ring.tick(at(RING_TIMEOUT.get() - 1)), (ring.clone(), None));
        assert_eq!(ring.tick(at(RING_TIMEOUT.get())), (Ring::Quiet, None));
    }

    /// `MURMURE-642`: accept joins the call, and decline writes the decline. Each ends the
    /// ring.
    #[test]
    fn accept_joins_the_call_and_decline_writes_the_decline() {
        let (call, channel) = (Id::<Call>::mint(), Id::<Channel>::mint());
        let ring = incoming(call, channel);
        assert_eq!(
            ring.accept(),
            (Ring::Quiet, Some(RingAct::Join { call, channel }))
        );
        assert_eq!(
            ring.decline(),
            (Ring::Quiet, Some(RingAct::Decline { call, channel }))
        );
        assert_eq!(Ring::Quiet.accept(), (Ring::Quiet, None));
    }

    /// A decline or an end of the call that rings stops the ring, and a cue of another call
    /// leaves it.
    #[test]
    fn a_decline_or_an_end_of_the_call_stops_its_ring() {
        let (call, channel) = (Id::<Call>::mint(), Id::<Channel>::mint());
        let ring = incoming(call, channel);
        for kind in [CueKind::Decline, CueKind::End] {
            let step = ring.heard(
                channel,
                cue(call, kind),
                Conversation::Direct,
                Line::Free,
                None,
                at(20),
            );
            assert_eq!(step, (Ring::Quiet, None));
        }
        let other = ring.heard(
            channel,
            cue(Id::<Call>::mint(), CueKind::End),
            Conversation::Direct,
            Line::Free,
            None,
            at(20),
        );
        assert_eq!(other, (ring.clone(), None));
    }

    /// `MURMURE-644`: the call this page started in a direct message waits. A decline hangs it
    /// up as declined, and the timeout as unanswered. The other member admitted ends the wait.
    #[test]
    fn a_decline_or_the_timeout_ends_the_ring_of_the_caller() {
        let (call, channel) = (Id::<Call>::mint(), Id::<Channel>::mint());
        let (me, other) = (Id::<User>::mint(), Id::<User>::mint());
        let waiting = Ring::Quiet.started(call, channel, Conversation::Direct, START);
        assert!(!waiting.rings_here());
        assert_eq!(waiting.deadline(), Some(START.plus(RING_TIMEOUT)));
        let declined = waiting.heard(
            channel,
            cue(call, CueKind::Decline),
            Conversation::Direct,
            Line::Busy,
            None,
            at(50),
        );
        assert_eq!(
            declined,
            (Ring::Quiet, Some(RingAct::HangUp(Unanswered::Declined)))
        );
        assert_eq!(
            waiting.tick(at(RING_TIMEOUT.get())),
            (Ring::Quiet, Some(RingAct::HangUp(Unanswered::NoAnswer)))
        );
        assert_eq!(
            waiting.admitted(me, Some(me)),
            waiting,
            "the page is no answer"
        );
        assert_eq!(waiting.admitted(other, Some(me)), Ring::Quiet);
        assert_eq!(waiting.hung_up(), Ring::Quiet);
        assert_eq!(
            Ring::Quiet.started(call, channel, Conversation::Channel, START),
            Ring::Quiet,
            "a call of a channel waits for nobody"
        );
    }

    /// `MURMURE-645`: the header tells a decline from no answer.
    #[test]
    fn the_header_says_why_a_ring_ended_with_no_answer() {
        assert_eq!(Unanswered::Declined.notice(), "Call declined");
        assert_eq!(Unanswered::NoAnswer.notice(), "No answer: the call ended");
    }

    /// The alert names the caller when the sidebar knows the name.
    #[test]
    fn the_alert_names_the_caller() {
        let name = Validated::<Name>::parse("Ada").expect("a name");
        assert_eq!(ring_title(Some(&name)), "Ada is calling");
        assert_eq!(ring_title(None), "Incoming call");
    }
}
