//! The call of one participant session, as the Sync worker drives it
//! (`ambre-spec-call-media.md section 3`, signalling).
//!
//! The client reads each frame of the relay's call socket and of the socket of the SFU. It
//! answers the steps the transport carries out. These are the frames to send, the socket of the
//! SFU to open, and the handoffs of the sender keys for the call worker. It keeps the keys of the
//! session in a [`CallKeys`], and reads no clock and no socket itself. So the host tier runs
//! two clients over a model of the relay, and the wasm transport runs the same steps.
//!
//! The relay gives the session its sender index in the admission. The client signs its join
//! under that index, and announces it. The relay carries each signed join to the other
//! sessions, and each wrap to the seat of its index alone.

use ambre_core::id::{Call, Channel, Id, User};
use ambre_core::{
    AccessToken, Authored, Count, CountKind, Credential, Instant, Ms, SenderIndex, Seq, TimeSpan,
    UnixMs, Validated,
};
use ambre_crypto::call::{SignedJoin, WrappedSenderKey};
use ambre_protocol::signaling::{
    CallClientMsg, CallRefusal, CallServerMsg, Seated, SfuMsg, SfuSocket,
};
use ed25519_dalek::SigningKey;

use crate::call::{Arrival, CallKeys, Joining, Moves, Refusal, SWITCH_DELAY};
use crate::membership_log::Log;
use ambre_crypto::call::KeyHandoff;

/// The call a session joins, and the channel it belongs to. A command of the bus that joins
/// a call names both, and the Sync worker joins through [`CallClient::opening`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct CallTarget {
    pub(crate) call: Id<Call>,
    pub(crate) channel: Id<Channel>,
}

/// Whether the session starts its call or joins one that runs (`CALL-068`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Opening {
    /// The call runs: the join of a call message.
    Join,
    /// The session starts the call, and posts its call message once seated (the Murmure calls spec).
    Start,
}

/// Why a call ended for this session.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CallEnd {
    /// The relay refused the join, with its notice.
    Refused(CallRefusal),
    /// The relay lost the state of the call (`SRV-249`).
    Lost,
    /// The keys of the session cannot go on: a generation past the width of a key
    /// identifier, or a sender index past it.
    Exhausted,
}

/// What the transport carries out after one input of the client, in order. The frames to the
/// relay go first. Then the socket of the SFU opens, its frames leave, and the handoffs go to
/// the call worker.
#[derive(Debug, Default)]
pub(crate) struct Steps {
    pub(crate) relay: Vec<CallClientMsg>,
    pub(crate) open_sfu: Option<Validated<SfuSocket>>,
    pub(crate) sfu: Vec<SfuMsg>,
    pub(crate) handoffs: Vec<KeyHandoff>,
    /// The signals for the peer connection of the UI thread: the TURN servers and the
    /// session descriptions of the SFU.
    pub(crate) page: Vec<ambre_bus::CallSignal>,
    /// The delay after which the client asks to be ticked: a fresh key waits for its switch.
    pub(crate) tick_in: Option<TimeSpan<Ms>>,
    pub(crate) ended: Option<CallEnd>,
    /// The call this session started, once the relay seated it: its call message is owed
    /// (the Murmure calls spec).
    pub(crate) started: Option<CallTarget>,
}

impl Steps {
    fn ended(end: CallEnd) -> Self {
        Self {
            ended: Some(end),
            ..Self::default()
        }
    }
}

/// The delay of the switch timer: the switch delay, and a margin so the tick finds the
/// switch due.
const SWITCH_TICK: TimeSpan<Ms> = TimeSpan::new(SWITCH_DELAY.get() + 10);

/// A number of wraps the client holds for a sender it has not admitted yet.
enum HeldWraps {}

impl CountKind for HeldWraps {
    type Repr = u64;
    const NAME: ambre_core::StaticKey<ambre_core::DebugNames> =
        ambre_core::StaticKey::new("HeldWraps");
}

/// The most wraps the client holds for senders it has not admitted. A sender sends one wrap
/// per held key, and holds two keys at most, so a few calls of joins fit.
const HELD_WRAPS: Count<HeldWraps> = Count::new(64);

/// Where the session stands in the call.
enum Phase {
    /// The join is sent, and the admission has not come back.
    Admitting,
    /// The session holds its seat and its keys, boxed: the keys outweigh the other phases.
    Seated {
        keys: Box<CallKeys>,
        /// The wraps from a sender whose join has not arrived yet, oldest first.
        held: Vec<(Seq<SenderIndex>, WrappedSenderKey)>,
    },
    /// The call ended for this session: it reads no more frames.
    Ended,
}

/// The call of one participant session.
pub(crate) struct CallClient {
    target: CallTarget,
    member: Id<User>,
    opening: Opening,
    phase: Phase,
}

impl CallClient {
    /// The client of `member` joining `target`. It sends nothing until the transport opens the
    /// call socket and sends [`CallClient::join_frame`].
    #[cfg(test)]
    pub(crate) const fn new(target: CallTarget, member: Id<User>) -> Self {
        Self::opening(target, member, Opening::Join)
    }

    /// The client of `member` that joins `target` as `opening` says: a start creates the call.
    pub(crate) const fn opening(target: CallTarget, member: Id<User>, opening: Opening) -> Self {
        Self {
            target,
            member,
            opening,
            phase: Phase::Admitting,
        }
    }

    /// The call the client joins.
    pub(crate) const fn target(&self) -> CallTarget {
        self.target
    }

    /// The sender index of the session, once the relay seated it.
    #[cfg(test)]
    pub(crate) fn own(&self) -> Option<Seq<SenderIndex>> {
        match &self.phase {
            Phase::Seated { keys, .. } => Some(keys.own()),
            Phase::Admitting | Phase::Ended => None,
        }
    }

    /// The first frame of the call socket: the access token, then the channel. A start asks
    /// the relay to create the call (`CALL-068`).
    pub(crate) const fn join_frame(&self, token: Credential<AccessToken>) -> CallClientMsg {
        let channel = self.target.channel;
        match self.opening {
            Opening::Join => CallClientMsg::Join { token, channel },
            Opening::Start => CallClientMsg::Start { token, channel },
        }
    }

    /// Whether the relay seated this session, so a leave frame has a seat to take out.
    pub(crate) const fn is_seated(&self) -> bool {
        matches!(self.phase, Phase::Seated { .. })
    }

    /// Reads one frame of the relay's call socket. `identity` signs the join, and `log` is the
    /// membership log of the channel, which admits each joiner (`CALL-019`).
    pub(crate) fn on_relay(
        &mut self,
        msg: CallServerMsg,
        identity: &SigningKey,
        log: &Log,
        now: Instant<UnixMs>,
    ) -> Steps {
        match msg {
            CallServerMsg::Admitted {
                admission,
                sfu,
                sender,
                roster,
            } => self.seat(identity, sender, now, log, roster, sfu, admission),
            CallServerMsg::Refused(refusal) => {
                self.phase = Phase::Ended;
                Steps::ended(CallEnd::Refused(refusal))
            }
            CallServerMsg::Lost => {
                self.phase = Phase::Ended;
                Steps::ended(CallEnd::Lost)
            }
            // The transport reads the answer to a leave: the client left before it came.
            CallServerMsg::Departed(_) => Steps::default(),
            // A seated session hands each renewal to the SFU, which ends a seat whose
            // admission expires with none (audit `N25`).
            CallServerMsg::Renewed(admission) => {
                let mut steps = Steps::default();
                if self.is_seated() {
                    steps.sfu.push(SfuMsg::Admission(admission));
                }
                steps
            }
            CallServerMsg::Joined(seated) => self.meet(&seated, log, Arrival::New),
            CallServerMsg::Left(sender) => self.part(sender, now),
            CallServerMsg::Wrap { from, to, wrap } => {
                let mut steps = Steps::default();
                let Phase::Seated { keys, held } = &mut self.phase else {
                    return steps;
                };
                if to != keys.own() {
                    return steps;
                }
                let Ok(wrapped) = WrappedSenderKey::from_bytes(&wrap) else {
                    return steps;
                };
                match keys.receive(from, &wrapped) {
                    Ok(handoff) => steps.handoffs.push(handoff),
                    // The join of `from` comes on the same topic before its wraps. A relay
                    // of several processes may still reorder them, so the wrap waits.
                    Err(Refusal::UnknownSender)
                        if Count::<HeldWraps>::of(held.len()) < HELD_WRAPS =>
                    {
                        held.push((from, wrapped));
                    }
                    Err(_) => {}
                }
                steps
            }
        }
    }

    /// Seats the session under `sender`: draws its keys, announces its join, and meets each
    /// session of the roster.
    #[expect(
        clippy::too_many_arguments,
        reason = "the fields of one admission frame, each read once"
    )]
    fn seat(
        &mut self,
        identity: &SigningKey,
        sender: Seq<SenderIndex>,
        now: Instant<UnixMs>,
        log: &Log,
        roster: Vec<Seated>,
        sfu: Option<Validated<SfuSocket>>,
        admission: Credential<ambre_core::CallAdmission>,
    ) -> Steps {
        if !matches!(self.phase, Phase::Admitting) {
            return Steps::default();
        }
        let joining = Joining {
            call: self.target.call,
            channel: self.target.channel,
            member: self.member,
            sender,
            at: Instant::<Authored>::after_epoch(now.since_epoch()),
        };
        let Ok((keys, handoff)) = CallKeys::start(identity, joining) else {
            self.phase = Phase::Ended;
            return Steps::ended(CallEnd::Exhausted);
        };
        let mut steps = Steps::default();
        steps
            .relay
            .push(CallClientMsg::Announce(keys.join().to_bytes()));
        steps.handoffs.push(handoff);
        if let Some(sfu) = sfu {
            steps.open_sfu = Some(sfu);
            steps.sfu.push(SfuMsg::Admission(admission));
        }
        self.phase = Phase::Seated {
            keys: Box::new(keys),
            held: Vec::new(),
        };
        // The call message follows the seat, so it never names a call the relay did not hold.
        if self.opening == Opening::Start {
            steps.started = Some(self.target);
        }
        for seated in &roster {
            let more = self.meet(seated, log, Arrival::Present);
            steps.relay.extend(more.relay);
            steps.handoffs.extend(more.handoffs);
            steps.page.extend(more.page);
            steps.ended = steps.ended.or(more.ended);
        }
        steps
    }

    /// Admits the session that announced `seated`, and wraps the keys of this session to it.
    fn meet(&mut self, seated: &Seated, log: &Log, arrival: Arrival) -> Steps {
        let mut steps = Steps::default();
        let Phase::Seated { keys, held } = &mut self.phase else {
            return steps;
        };
        let Ok(join) = SignedJoin::from_bytes(&seated.join) else {
            return steps;
        };
        // A join signed under another index than its seat is no join of that seat. Each wrap
        // to it would name one index and bind another (`CALL-061`).
        if join.verified().map(|claim| claim.sender) != Ok(seated.sender) {
            return steps;
        }
        let moves = match keys.admit(&join, log, arrival) {
            Ok(moves) => moves,
            Err(Refusal::Exhausted) => {
                self.phase = Phase::Ended;
                return Steps::ended(CallEnd::Exhausted);
            }
            // A join seen already, of another call, or of no member: no key goes to it.
            Err(_) => return steps,
        };
        Self::carry(&mut steps, moves);
        // The page applies the volume of this member to each slot that carries the index.
        if let Ok(claim) = join.verified() {
            steps.page.push(ambre_bus::CallSignal::Admitted {
                sender: seated.sender,
                member: claim.member,
            });
        }
        // The wraps the joiner sent before its join arrived open now.
        let (theirs, rest): (Vec<_>, Vec<_>) = std::mem::take(held)
            .into_iter()
            .partition(|(from, _)| *from == seated.sender);
        *held = rest;
        steps.handoffs.extend(
            theirs
                .iter()
                .filter_map(|(from, wrapped)| keys.receive(*from, wrapped).ok()),
        );
        steps
    }

    /// The session of `sender` left: this session drops its keys and switches its own.
    fn part(&mut self, sender: Seq<SenderIndex>, now: Instant<UnixMs>) -> Steps {
        let Phase::Seated { keys, held } = &mut self.phase else {
            return Steps::default();
        };
        held.retain(|(from, _)| *from != sender);
        match keys.depart(sender, now) {
            Ok(moves) => {
                let mut steps = Steps::default();
                Self::carry(&mut steps, moves);
                steps.page.push(ambre_bus::CallSignal::Departed(sender));
                steps.tick_in = Some(SWITCH_TICK);
                steps
            }
            Err(_) => {
                self.phase = Phase::Ended;
                Steps::ended(CallEnd::Exhausted)
            }
        }
    }

    /// Reads the membership log of the channel again. A member it no longer holds is out of
    /// the call for this session, which switches its key (`CALL-028`, `CALL-060`).
    pub(crate) fn heed(&mut self, log: &Log, now: Instant<UnixMs>) -> Steps {
        let Phase::Seated { keys, .. } = &mut self.phase else {
            return Steps::default();
        };
        match keys.heed(log, now) {
            Ok(moves) => {
                let mut steps = Steps::default();
                let switched = !moves.wraps.is_empty() || !moves.handoffs.is_empty();
                Self::carry(&mut steps, moves);
                steps.tick_in = switched.then_some(SWITCH_TICK);
                steps
            }
            Err(_) => {
                self.phase = Phase::Ended;
                Steps::ended(CallEnd::Exhausted)
            }
        }
    }

    /// Moves the session to its fresh key once the switch delay has passed (`CALL-027`).
    pub(crate) fn tick(&mut self, now: Instant<UnixMs>) -> Steps {
        let mut steps = Steps::default();
        if let Phase::Seated { keys, .. } = &mut self.phase {
            steps.handoffs.extend(keys.tick(now));
            // A switch still pending asks another tick: the wall clock may have
            // stepped back since the timer was armed (`CALL-118`).
            if keys.switch_pending() {
                steps.tick_in = Some(SWITCH_TICK);
            }
        }
        steps
    }

    /// Reads one message of the socket of the SFU. The TURN servers and each session description
    /// go on to the peer connection of the UI thread.
    pub(crate) fn on_sfu(&mut self, msg: SfuMsg) -> Steps {
        let mut steps = Steps::default();
        let signal = match msg {
            SfuMsg::Ice(servers) => ambre_bus::CallSignal::Ice(servers),
            SfuMsg::Offer(sdp) => ambre_bus::CallSignal::Offer(sdp),
            SfuMsg::Answer(sdp) => ambre_bus::CallSignal::Answer(sdp),
            SfuMsg::Unshared => ambre_bus::CallSignal::ShareTakenOver,
            SfuMsg::Admission(_) | SfuMsg::Share | SfuMsg::Videos(_) => return steps,
        };
        steps.page.push(signal);
        steps
    }

    /// Writes the wraps of `moves` as frames for the relay, and keeps its handoffs.
    fn carry(steps: &mut Steps, moves: Moves) {
        steps
            .relay
            .extend(moves.wraps.into_iter().map(|wrap| CallClientMsg::Wrap {
                to: wrap.to,
                wrap: wrap.wrapped.to_bytes(),
            }));
        steps.handoffs.extend(moves.handoffs);
    }
}

#[cfg(test)]
#[path = "call_client_tests.rs"]
mod tests;
