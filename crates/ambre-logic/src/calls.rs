//! The call messages of the open channels
//! (`specs/5-Murmure/features/ambre-spec-murmure-calls.md section 3`).
//!
//! The Sync worker says when the relay seated the starter of a call, and when it counted a
//! leave of this session as the last. This module turns each into a write of the channel:
//! the call message, then the end. It also asks the Sync worker who is in each live call of a
//! shown channel, on the tick of the view, once per interval of the budget row
//! (`MURMURE-595`). The answer goes to the view and to no document (`MURMURE-597`).

use std::collections::HashSet;

use ambre_bus::UiEvent;
use ambre_bus::{CallMembers, SyncCommand};
use ambre_core::id::{Call, Channel, Id};
use ambre_core::{Authored, Instant};
use ambre_murmure::message::{MurmureCommand, MurmureEvent};

use crate::LogicState;

/// The asks of who is in the calls of one channel.
#[derive(Debug, Default)]
pub(crate) struct CallAsks {
    /// The tick of the last ask, `None` before the first.
    last: Option<Instant<Authored>>,
    /// The calls the relay answered it holds no state of: they ended, and no tick asks again.
    gone: HashSet<Id<Call>>,
}

impl LogicState {
    /// The relay seated the starter of `call` at `at`: the call message goes into `channel`
    /// (`MURMURE-408`).
    pub(crate) fn call_started(
        &mut self,
        call: Id<Call>,
        channel: Id<Channel>,
        at: Instant<Authored>,
    ) {
        self.handle_channel_command(channel, MurmureCommand::PostCall { call, now: at });
    }

    /// The relay counted this session's leave as the last of `call`: its end goes into the
    /// call message (`MURMURE-591`).
    pub(crate) fn call_ended(
        &mut self,
        call: Id<Call>,
        channel: Id<Channel>,
        at: Instant<Authored>,
    ) {
        self.handle_channel_command(channel, MurmureCommand::EndCall { call, now: at });
    }

    /// The relay answered who is in `call`. The view shows it, and a call the relay holds no
    /// state of is asked about no more (`MURMURE-592`).
    pub(crate) fn call_roster(
        &mut self,
        call: Id<Call>,
        channel: Id<Channel>,
        members: CallMembers,
    ) {
        if members == CallMembers::Gone {
            self.call_asks.entry(channel).or_default().gone.insert(call);
        }
        self.send_to_ui(UiEvent::Channel {
            channel_id: channel,
            event: MurmureEvent::CallMembers { call, members },
        });
    }

    /// The view ticked `channel` at `now`, so it shows that channel. Once per interval of the
    /// budget row, each live call of its held segments is asked about (`MURMURE-595`).
    pub(crate) fn ask_call_rosters(&mut self, channel: Id<Channel>, now: Instant<Authored>) {
        let Some(state) = self.channels.get(&channel) else {
            return;
        };
        let asks = self.call_asks.entry(channel).or_default();
        let due = asks.last.is_none_or(|last| {
            now.since_epoch()
                .get()
                .saturating_sub(last.since_epoch().get())
                >= ambre_murmure::call::ROSTER_INTERVAL.get()
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
            self.send_to_sync(SyncCommand::AskCallRoster {
                call,
                channel,
                meta,
            });
        }
    }
}
