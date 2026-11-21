//! The call messages of the open channels
//! (`specs/5-Murmure/features/ambre-spec-murmure-calls.md section 3`).
//!
//! The Sync worker says when the relay seated the starter of a call, and when it counted a
//! leave of this session as the last. This module turns each into a write of the channel:
//! the call message, then the end. It also asks the Sync worker who is in each live call of a
//! shown channel. The tick of the view asks, once per interval of the budget row. The answer
//! goes to the view and to no document (the Murmure calls spec).

use std::collections::HashSet;

use crate::message::{MurmureCommand, MurmureEvent};
use ambre_bus::UiEvent;
use ambre_bus::{CallMembers, SyncCommand};
use ambre_core::id::{Call, Channel, Document, Id};
use ambre_core::{Authored, Instant};

use super::Runtime;

/// The asks of who is in the calls of one channel.
#[derive(Debug, Default)]
pub(crate) struct CallAsks {
    /// The tick of the last ask, `None` before the first.
    last: Option<Instant<Authored>>,
    /// The calls the relay answered it holds no state of: they ended, and no tick asks again.
    gone: HashSet<Id<Call>>,
}

impl<D> Runtime<D> {
    /// The relay seated the starter of `call` at `at`: the call message goes into `channel`
    /// (the Murmure calls spec).
    pub fn call_started(&mut self, call: Id<Call>, channel: Id<Channel>, at: Instant<Authored>) {
        self.handle_channel_command(channel, MurmureCommand::PostCall { call, now: at });
    }

    /// The relay counted this session's leave as the last of `call`: its end goes into the
    /// call message (the Murmure calls spec).
    pub fn call_ended(&mut self, call: Id<Call>, channel: Id<Channel>, at: Instant<Authored>) {
        self.handle_channel_command(channel, MurmureCommand::EndCall { call, now: at });
    }

    /// A join of `call` waits for the `meta` document of `channel`. The answer is the `meta`
    /// of the channel state this thread opened, open or closed since, whose sealed header
    /// names `channel` (`MURMURE-027`). A channel it never opened has none, and the join ends.
    /// The relay never names it, so it cannot seat a join under another channel's log
    /// (`CALL-149`, audit `P24`).
    ///
    /// A channel still opening answers once its open ends, either way. A join pressed as the
    /// channel shows waits for its state rather than end.
    pub fn call_meta_asked(&mut self, call: Id<Call>, channel: Id<Channel>) {
        if self.opening.contains_key(&channel) {
            self.meta_asks.push((call, channel));
            return;
        }
        let meta = self.call_meta_of(channel);
        self.ask(SyncCommand::CallMeta { call, meta });
    }

    /// The `meta` document of the channel state this thread holds for `channel`, open or
    /// closed (`CALL-149`).
    fn call_meta_of(&self, channel: Id<Channel>) -> Option<Id<Document>> {
        self.channels
            .get(&channel)
            .map(crate::ChannelState::meta_id)
            .or_else(|| self.closed_channels.get(&channel).map(|c| c.meta()))
            .map(|meta| meta.document_id())
    }

    /// Answers each held ask of a `meta` whose channel no longer opens: the open named it,
    /// or the open failed and names none (`CALL-149`). It runs after each handler, so no
    /// path out of an open leaves a join waiting.
    pub(super) fn settle_meta_asks(&mut self) {
        if self.meta_asks.is_empty() {
            return;
        }
        let asks = std::mem::take(&mut self.meta_asks);
        for (call, channel) in asks {
            if self.opening.contains_key(&channel) {
                self.meta_asks.push((call, channel));
            } else {
                let meta = self.call_meta_of(channel);
                self.ask(SyncCommand::CallMeta { call, meta });
            }
        }
    }

    /// The relay answered who is in `call`. The view shows it, and a call the relay holds no
    /// state of is asked about no more (the Murmure calls spec).
    pub fn call_roster(&mut self, call: Id<Call>, channel: Id<Channel>, members: CallMembers) {
        if members == CallMembers::Gone {
            self.call_asks.entry(channel).or_default().gone.insert(call);
        }
        self.emit(UiEvent::Channel {
            channel_id: channel,
            event: MurmureEvent::CallMembers { call, members },
        });
    }

    /// The view ticked `channel` at `now`, so it shows that channel. Once per interval of the
    /// budget row, each live call of its held segments is asked about (the Murmure calls spec).
    pub(crate) fn ask_call_rosters(&mut self, channel: Id<Channel>, now: Instant<Authored>) {
        let Some(state) = self.channels.get(&channel) else {
            return;
        };
        let asks = self.call_asks.entry(channel).or_default();
        let due = asks.last.is_none_or(|last| {
            now.since_epoch()
                .get()
                .saturating_sub(last.since_epoch().get())
                >= crate::logic::call::ROSTER_INTERVAL.get()
        });
        if !due {
            return;
        }
        let calls: Vec<Id<Call>> = state
            .live_calls()
            .into_iter()
            .filter(|call| !asks.gone.contains(call))
            .collect();
        if calls.is_empty() {
            return;
        }
        asks.last = Some(now);
        let meta = state.meta_id().document_id();
        for call in calls {
            self.ask(SyncCommand::AskCallRoster {
                call,
                channel,
                meta,
            });
        }
    }
}
