//! Who is in a call, for a reader of its call message
//! (`5-Murmure/features/ambre-spec-murmure-calls.md section 3`, who is in the call).
//!
//! The Logic thread asks on the tick of a channel the view shows (the Murmure calls spec). The
//! Sync worker reads the seats of the call from the relay. It checks each signed join against
//! the membership log of the channel, as a participant does before it wraps a key. A join that
//! fails the check names no one. The log of each channel stays in the worker. A join of a
//! member it does not hold reads the log again at the next ask.

use ambre_bus::{CallMembers, LogicEvent};
use ambre_core::id::{Call, Channel, Document, Id, User};
use ambre_crypto::call::SignedJoin;
use ambre_protocol::signaling::{CallRoster, ChannelCall, ChannelCalls};

#[cfg(target_arch = "wasm32")]
use crate::SyncEvent;
use crate::SyncState;
use crate::membership_log::Log;

/// The members `roster` seats in `call` of `channel`, as the log `log` admits them. The second
/// answer says whether a join named a member the log does not hold under its key. A join of another call,
/// under another index than its seat, or whose signature fails, is dropped and names no one.
pub(crate) fn members_in(
    call: Id<Call>,
    channel: Id<Channel>,
    roster: &CallRoster,
    log: &Log,
) -> (CallMembers, Unknown) {
    let seated = match roster {
        CallRoster::Gone => return (CallMembers::Gone, Unknown::None),
        CallRoster::Live(seated) => seated,
    };
    let mut members = Vec::<Id<User>>::new();
    let mut unknown = Unknown::None;
    for seat in seated {
        let Ok(join) = SignedJoin::from_bytes(&seat.join) else {
            continue;
        };
        let Ok(claim) = join.verified() else {
            continue;
        };
        if claim.call != call || claim.channel != channel || claim.sender != seat.sender {
            continue;
        }
        if log.present_key(claim.member) == Some(claim.identity) {
            if !members.contains(&claim.member) {
                members.push(claim.member);
            }
        } else {
            unknown = Unknown::Some;
        }
    }
    ambre_core::order::sort_by_id(&mut members, |member| *member);
    (CallMembers::Live(members), unknown)
}

/// Whether a verified join named a member the log of the channel does not hold.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Unknown {
    /// Every verified join named a member of the log.
    None,
    /// One did: the log may be older than the member, so the next ask reads it again.
    Some,
}

impl SyncState {
    /// Asks the relay who is in `call` of `channel`, whose `meta` document is `meta`. The
    /// answer arrives as [`SyncEvent::CallRosterRead`]. The log of the channel is read with it
    /// when the worker holds none.
    pub(crate) fn ask_call_roster(
        &mut self,
        call: Id<Call>,
        channel: Id<Channel>,
        meta: Id<Document>,
    ) {
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.roster_asks.push(call);
            let _ = (channel, meta);
        }
        #[cfg(target_arch = "wasm32")]
        {
            let Some(tx) = self.event_tx.clone() else {
                return;
            };
            let log = match (
                self.roster_logs.contains_key(&meta),
                self.exchange_key.clone(),
            ) {
                (false, Some(exchange_key)) => Some(crate::sharing::CallLogParams {
                    document_id: meta,
                    url_scheme: self.url_scheme,
                    api_base: self.api_base.clone(),
                    exchange_key,
                    rule: self.log_rule(meta),
                    roots: self.roots(meta, ambre_core::DocType::Murmure),
                }),
                _ => None,
            };
            let (scheme, base) = (self.url_scheme, self.api_base.clone());
            let http = crate::sharing::GlooHttp {
                auth: self.auth_context(),
            };
            crate::spawn(async move {
                let read =
                    crate::sharing::call_roster_task(&http, call, channel, scheme, &base, log)
                        .await;
                let _ = tx.send(SyncEvent::CallRosterRead {
                    call,
                    channel,
                    meta,
                    roster: read.roster,
                    log: read.log,
                });
            });
        }
    }

    /// The relay answered an ask of who is in `call`. A fresh `log` of the channel is kept.
    /// A failed read answers nothing: the next tick asks again. A roster with no log to check
    /// it names no one, so it answers nothing either.
    pub(crate) fn call_roster_read(
        &mut self,
        call: Id<Call>,
        channel: Id<Channel>,
        meta: Id<Document>,
        roster: Option<CallRoster>,
        log: Option<Log>,
    ) {
        if let Some(log) = log {
            self.roster_logs.insert(meta, log);
        }
        let Some(roster) = roster else {
            return;
        };
        let members = match (&roster, self.roster_logs.get(&meta)) {
            (CallRoster::Gone, _) => CallMembers::Gone,
            (CallRoster::Live(_), None) => return,
            (CallRoster::Live(_), Some(log)) => {
                let (members, unknown) = members_in(call, channel, &roster, log);
                if unknown == Unknown::Some {
                    self.roster_logs.remove(&meta);
                }
                members
            }
        };
        self.send_to_logic(LogicEvent::CallRoster {
            call,
            channel,
            members,
        });
    }

    /// Asks the relay for the live call of each channel of the user (`CALL-162`). The answer
    /// arrives as [`crate::SyncEvent::ChannelCallsFetched`].
    pub(crate) fn fetch_channel_calls(&self) {
        let Some(tx) = self.event_tx.clone() else {
            crate::log::warning!("Sync: no relay on this transport, no live calls");
            return;
        };
        let url = crate::http::Url::<crate::http::ApiRoot>::of(self.url_scheme, &self.api_base)
            .route("/channel-calls");
        let auth = self.auth_context();
        crate::spawn(async move {
            let result = crate::http::get_json_auth::<ChannelCalls>(&url, auth)
                .await
                .map(|answer| answer.calls);
            let _ = tx.send(crate::SyncEvent::ChannelCallsFetched { result });
        });
    }

    /// The relay answered an ask of the live calls. A failed read answers nothing: the next
    /// listing asks again.
    pub(crate) fn channel_calls_fetched(
        &self,
        result: Result<Vec<ChannelCall>, crate::http::HttpError>,
    ) {
        match result {
            Ok(calls) => self.send_to_logic(LogicEvent::ChannelCalls {
                calls: calls.into_iter().map(|c| (c.channel, c.call)).collect(),
            }),
            Err(e) => crate::log::warning!("Sync: the live calls were not read: {e}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use ambre_core::id::{Call, Channel, Id};
    use ambre_core::{Authored, Instant, Role, Seq};
    use ambre_crypto::call::{CallExchangeKey, JoinClaim, SignedJoin};
    use ambre_protocol::MembershipAct;
    use ambre_protocol::signaling::{CallRoster, Seated};

    use super::{Unknown, members_in};
    use crate::membership_log::tests::{DOC, Person, append};
    use crate::membership_log::{Log, digest, no_self_join};
    use ambre_bus::CallMembers;

    const CALL: Id<Call> = Id::<Call>::from_bytes([41; 16]);
    const CHANNEL: Id<Channel> = Id::<Channel>::from_bytes([42; 16]);

    /// The log of a channel that `owner` created, where it invited each of `members`.
    fn log_of(owner: &Person, members: &[&Person]) -> Log {
        let mut entries = Vec::new();
        append(
            &mut entries,
            owner,
            owner,
            MembershipAct::Create,
            Role::Owner,
        );
        for member in members {
            append(
                &mut entries,
                owner,
                member,
                MembershipAct::Invite,
                Role::Editor,
            );
        }
        Log::walk(
            DOC,
            entries.first().and_then(digest),
            &entries,
            no_self_join,
        )
    }

    /// The seat of `person` under `sender`, its join signed for `call`.
    fn seat(person: &Person, call: Id<Call>, sender: u64) -> Seated {
        seat_in(person, call, CHANNEL, sender)
    }

    /// The seat of `person` under `sender`, its join signed for `call` in `channel`.
    fn seat_in(person: &Person, call: Id<Call>, channel: Id<Channel>, sender: u64) -> Seated {
        let claim = JoinClaim {
            call,
            channel,
            member: person.id,
            identity: person.public(),
            at: Instant::<Authored>::new(1),
            sender: Seq::new(sender),
            exchange: CallExchangeKey::generate().public(),
        };
        Seated {
            sender: Seq::new(sender),
            join: SignedJoin::sign(&person.key, claim)
                .expect("a join")
                .to_bytes(),
        }
    }

    /// Carol reads Ada and Bob in the call, each once, from their
    /// signed joins. Mallory is no member of the log, so her join names no one, and the log is
    /// read again. A join of another call, a join under another index than its seat and a
    /// forged signature name no one either.
    #[test]
    fn a_join_the_log_does_not_hold_names_no_one_in_the_call() {
        let (owner, ada, bob, mallory) = (
            Person::new(1),
            Person::new(2),
            Person::new(3),
            Person::new(4),
        );
        let log = log_of(&owner, &[&ada, &bob]);
        let mut misplaced = seat(&bob, CALL, 7);
        misplaced.sender = Seq::new(8);
        let mut forged = seat(&ada, CALL, 9);
        let mut bytes = forged.join.as_bytes().to_vec();
        if let Some(last) = bytes.last_mut() {
            *last ^= 1;
        }
        forged.join = ambre_core::Payload::new(bytes);
        let roster = CallRoster::Live(vec![
            seat(&ada, CALL, 0),
            seat(&bob, CALL, 1),
            seat(&ada, CALL, 2),
            seat(&bob, Id::<Call>::from_bytes([43; 16]), 3),
            misplaced,
            forged,
        ]);
        let (members, unknown) = members_in(CALL, CHANNEL, &roster, &log);
        let mut want = vec![ada.id, bob.id];
        want.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));
        assert_eq!(members, CallMembers::Live(want));
        assert_eq!(unknown, Unknown::None);

        let with_mallory = CallRoster::Live(vec![seat(&ada, CALL, 0), seat(&mallory, CALL, 1)]);
        let (members, unknown) = members_in(CALL, CHANNEL, &with_mallory, &log);
        assert_eq!(members, CallMembers::Live(vec![ada.id]));
        assert_eq!(unknown, Unknown::Some, "the log is read again");
    }

    /// A stray join names no one, even for a member of the log. It names another call,
    /// another channel, or another index than its seat.
    #[test]
    fn a_join_that_names_another_seat_names_no_one() {
        let (owner, bob) = (Person::new(1), Person::new(3));
        let log = log_of(&owner, &[&bob]);
        let mut misplaced = seat(&bob, CALL, 7);
        misplaced.sender = Seq::new(8);
        for stray in [
            seat(&bob, Id::<Call>::from_bytes([43; 16]), 3),
            seat_in(&bob, CALL, Id::<Channel>::from_bytes([44; 16]), 4),
            misplaced,
        ] {
            let roster = CallRoster::Live(vec![stray]);
            let (members, unknown) = members_in(CALL, CHANNEL, &roster, &log);
            assert_eq!(members, CallMembers::Live(Vec::new()));
            assert_eq!(unknown, Unknown::None);
        }
    }

    /// A log that names every member of a roster stays: the next roster reads against it
    /// without a fresh read of the log.
    #[test]
    fn a_log_that_names_every_member_is_kept_for_the_next_roster() {
        let (owner, ada) = (Person::new(1), Person::new(2));
        let (sink, rx) = crate::LogicSink::channel();
        let mut state = crate::SyncState::new(
            sink,
            crate::http::Url::<crate::RelayHost>::new("relay.example".to_owned()),
        );
        let roster = || CallRoster::Live(vec![seat(&ada, CALL, 0)]);
        state.call_roster_read(
            CALL,
            CHANNEL,
            DOC,
            Some(roster()),
            Some(log_of(&owner, &[&ada])),
        );
        state.call_roster_read(CALL, CHANNEL, DOC, Some(roster()), None);
        let heard: Vec<_> = std::iter::from_fn(|| rx.try_recv().ok()).collect();
        assert_eq!(heard.len(), 2, "{heard:?}");
        assert!(heard.iter().all(|event| matches!(
            event,
            ambre_bus::LogicEvent::CallRoster { members: CallMembers::Live(m), .. } if *m == [ada.id]
        )));
    }

    /// A roster the relay holds no state of reads as gone, whatever the log.
    #[test]
    fn a_call_the_relay_holds_no_state_of_reads_as_gone() {
        let owner = Person::new(1);
        let (members, _) = members_in(CALL, CHANNEL, &CallRoster::Gone, &log_of(&owner, &[]));
        assert_eq!(members, CallMembers::Gone);
    }
}
