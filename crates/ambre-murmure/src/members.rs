//! The members of an open space and their presence
//! (`features/ambre-spec-murmure-members.md section 3`).
//!
//! The member rows come from the relay's table of the space document
//! (`MURMURE-031`). The presence of each member rides the ephemerals of that
//! document (`MURMURE-444`). The engine reads no clock. The view's tick stamps
//! every sample heard since the tick before, so a sample is dated to within one
//! refresh period.

use std::collections::HashMap;

use ambre_bus::{MemberView, Presence, Timestamp};
use ambre_core::UserId;

/// How often a client that holds a space open sends its presence sample, in
/// milliseconds (`ambre-spec-budgets.md section 8`).
pub const REFRESH_PERIOD_MS: u64 = 60_000;

/// How long a presence sample keeps its member online, in milliseconds
/// (`ambre-spec-budgets.md section 8`). Three refresh periods: one for the
/// tick that dates a sample, one for a sample lost on the way, and one for
/// two drifting timers.
pub const PRESENCE_EXPIRY_MS: u64 = 3 * REFRESH_PERIOD_MS;

/// The first byte of a presence sample. Another ephemeral of the space
/// document takes another tag, so neither reads as the other.
const SAMPLE_TAG: u8 = 0x50;

/// The state byte of an online member.
const ONLINE: u8 = 1;

/// The presence sample of a space: its tag, then the state, and nothing else.
/// It names no channel, so it cannot show which channel a member reads
/// (`MURMURE-446`).
pub fn presence_sample() -> Vec<u8> {
    vec![SAMPLE_TAG, ONLINE]
}

/// Whether `payload` is a presence sample of a space. A sample that does not
/// decode is dropped without an error (`PRESENCE-013`).
pub fn is_presence_sample(payload: &[u8]) -> bool {
    payload == [SAMPLE_TAG, ONLINE]
}

/// The members of `members` whose display name answers `query`, in the order
/// of `members`: a case-insensitive substring match. An empty query answers
/// every member.
///
/// A pure function of what the client holds: nothing the user types reaches
/// the relay (`MURMURE-454`).
pub fn find_members<'a>(members: &'a [MemberView], query: &str) -> Vec<&'a MemberView> {
    let query = query.trim().to_lowercase();
    members
        .iter()
        .filter(|member| query.is_empty() || member.display_name.to_lowercase().contains(&query))
        .collect()
}

/// The members of a space, and when each one was last heard from.
#[derive(Debug, Default)]
pub struct Roster {
    /// The relay's member rows: each member and their display name.
    members: Vec<(UserId, String)>,
    /// The members heard from, with the tick that dates their last sample.
    /// `None` for a sample heard before the first tick: the next tick dates
    /// it.
    heard: HashMap<UserId, Option<Timestamp>>,
    /// The latest tick of the view, `None` before the first.
    now: Option<Timestamp>,
    /// This user, online as long as the space is open.
    me: Option<UserId>,
}

impl Roster {
    /// A roster with no member yet, held open by `me`.
    pub fn new(me: Option<UserId>) -> Self {
        Self {
            me,
            ..Self::default()
        }
    }

    /// Replace the member rows with the relay's answer. A member outside it
    /// shows nowhere, whatever sample came from them (`MURMURE-440`).
    pub fn set_members(&mut self, members: impl IntoIterator<Item = (UserId, String)>) {
        self.members = members.into_iter().collect();
    }

    /// Set this user, once the session names them.
    pub fn set_me(&mut self, me: UserId) {
        self.me = Some(me);
    }

    /// A presence sample came from `user`. Returns whether they were offline
    /// before, so the caller can answer a member who just came online with
    /// its own sample. A newcomer then sees who is online without waiting a
    /// refresh period.
    pub fn heard(&mut self, user: UserId) -> bool {
        self.heard.insert(user, self.now).is_none()
    }

    /// The relay says `user` stopped watching the space (`PRESENCE-014`).
    /// Returns whether they were online.
    pub fn left(&mut self, user: UserId) -> bool {
        self.heard.remove(&user).is_some()
    }

    /// The view's tick: date the samples heard since the tick before, and turn
    /// offline every member whose last sample is older than the expiry
    /// (`MURMURE-445`). Returns whether a member turned offline.
    pub fn tick(&mut self, now: Timestamp) -> bool {
        self.now = Some(now);
        let before = self.heard.len();
        self.heard.retain(|_, stamp| {
            let at = *stamp.get_or_insert(now);
            now.0.saturating_sub(at.0) <= PRESENCE_EXPIRY_MS
        });
        self.heard.len() != before
    }

    /// The presence of `user`: this user is online, and every other member
    /// while a sample of theirs has not expired.
    pub fn presence(&self, user: UserId) -> Presence {
        if Some(user) == self.me || self.heard.contains_key(&user) {
            Presence::Online
        } else {
            Presence::Offline
        }
    }

    /// The member list: the online members first, then the others, each
    /// group by display name (`MURMURE-450`). Two equal names keep the order
    /// of their identifiers, so the list never reorders between two draws.
    pub fn view(&self) -> Vec<MemberView> {
        let mut members: Vec<MemberView> = self
            .members
            .iter()
            .map(|(user_id, display_name)| MemberView {
                user_id: *user_id,
                display_name: display_name.clone(),
                presence: self.presence(*user_id),
            })
            .collect();
        members.sort_by_cached_key(|member| {
            (
                member.presence != Presence::Online,
                member.display_name.to_lowercase(),
                *member.user_id.as_bytes(),
            )
        });
        members
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const T0: Timestamp = Timestamp(1_700_000_000_000);

    fn at(offset: u64) -> Timestamp {
        Timestamp(T0.0 + offset)
    }

    fn names(view: &[MemberView]) -> Vec<&str> {
        view.iter().map(|m| m.display_name.as_str()).collect()
    }

    fn roster(members: &[(UserId, &str)]) -> Roster {
        let mut roster = Roster::new(None);
        roster.set_members(members.iter().map(|(id, name)| (*id, (*name).to_owned())));
        roster.tick(T0);
        roster
    }

    /// The list holds the relay's rows and no one else: a sample from a user
    /// outside them adds no one (`MURMURE-440`).
    #[test]
    fn the_list_holds_the_member_rows_and_no_one_else() {
        let (alice, bob, mallory) = (UserId::new(), UserId::new(), UserId::new());
        let mut roster = roster(&[(alice, "Alice"), (bob, "Bob")]);
        roster.heard(mallory);
        let view = roster.view();
        assert_eq!(view.len(), 2);
        assert!(view.iter().all(|m| m.user_id != mallory));
    }

    /// A sample turns its member online, and the expiry passed with no sample
    /// turns them offline again (`MURMURE-445`).
    #[test]
    fn a_member_turns_offline_once_no_sample_refreshes_them_within_the_expiry() {
        let bob = UserId::new();
        let mut roster = roster(&[(bob, "Bob")]);
        assert_eq!(roster.presence(bob), Presence::Offline);
        assert!(roster.heard(bob), "Bob was offline");
        assert_eq!(roster.presence(bob), Presence::Online);
        assert!(
            !roster.tick(at(PRESENCE_EXPIRY_MS)),
            "still within the expiry"
        );
        assert_eq!(roster.presence(bob), Presence::Online);
        assert!(roster.tick(at(PRESENCE_EXPIRY_MS + 1)));
        assert_eq!(roster.presence(bob), Presence::Offline);
    }

    /// A sample refreshed each period keeps its member online across many
    /// periods, even when every sample lands just before the next tick.
    #[test]
    fn a_member_refreshed_each_period_stays_online() {
        let bob = UserId::new();
        let mut roster = roster(&[(bob, "Bob")]);
        for period in 1..10 {
            roster.heard(bob);
            roster.tick(at(period * REFRESH_PERIOD_MS));
            assert_eq!(roster.presence(bob), Presence::Online, "period {period}");
        }
    }

    /// A sample heard before the first tick is dated by that tick, not by the
    /// epoch, so it does not expire at once.
    #[test]
    fn a_sample_before_the_first_tick_is_dated_by_it() {
        let bob = UserId::new();
        let mut roster = Roster::new(None);
        roster.set_members([(bob, "Bob".to_owned())]);
        roster.heard(bob);
        assert!(!roster.tick(T0));
        assert_eq!(roster.presence(bob), Presence::Online);
    }

    /// The relay's word that a member left turns them offline at once. This
    /// user stays online while the space is open.
    #[test]
    fn a_departure_turns_its_member_offline_and_this_user_stays_online() {
        let (me, bob) = (UserId::new(), UserId::new());
        let mut roster = roster(&[(me, "Me"), (bob, "Bob")]);
        roster.set_me(me);
        roster.heard(bob);
        assert!(roster.left(bob));
        assert_eq!(roster.presence(bob), Presence::Offline);
        assert!(!roster.left(me));
        assert_eq!(roster.presence(me), Presence::Online);
    }

    /// Only a new member answers true, so a client answers a newcomer once
    /// and two clients never answer each other forever.
    #[test]
    fn only_the_first_sample_of_a_member_reads_as_new() {
        let bob = UserId::new();
        let mut roster = roster(&[(bob, "Bob")]);
        assert!(roster.heard(bob));
        assert!(!roster.heard(bob));
    }

    /// The online members come first, then the others, each group by display
    /// name, whatever the case (`MURMURE-450`).
    #[test]
    fn the_list_shows_the_online_members_first_each_group_by_name() {
        let ids: Vec<UserId> = (0..4).map(|_| UserId::new()).collect();
        let mut roster = roster(&[
            (ids[0], "dora"),
            (ids[1], "Carl"),
            (ids[2], "bea"),
            (ids[3], "Abel"),
        ]);
        roster.heard(ids[0]);
        roster.heard(ids[1]);
        assert_eq!(names(&roster.view()), ["Carl", "dora", "Abel", "bea"]);
        let view = roster.view();
        assert!(view[..2].iter().all(|m| m.presence == Presence::Online));
        assert!(view[2..].iter().all(|m| m.presence == Presence::Offline));
    }

    /// The sample holds its tag and the state alone: no channel, no name, no
    /// identifier (`MURMURE-446`). It decodes, and nothing else does.
    #[test]
    fn the_sample_holds_the_state_alone() {
        let sample = presence_sample();
        assert_eq!(sample.len(), 2);
        assert!(is_presence_sample(&sample));
        assert!(!is_presence_sample(&[]));
        assert!(!is_presence_sample(&[SAMPLE_TAG]));
        assert!(!is_presence_sample(&[SAMPLE_TAG, ONLINE, 0]));
        assert!(!is_presence_sample(&[0, ONLINE]));
    }

    /// The filter keeps the members whose name holds what the user types,
    /// whatever the case, in the order of the list (`MURMURE-453`).
    #[test]
    fn the_filter_keeps_the_members_whose_name_matches() {
        let roster = roster(&[
            (UserId::new(), "Bob"),
            (UserId::new(), "Alice"),
            (UserId::new(), "Jacob"),
        ]);
        let view = roster.view();
        let found: Vec<&str> = find_members(&view, " OB ")
            .iter()
            .map(|m| m.display_name.as_str())
            .collect();
        assert_eq!(found, ["Bob", "Jacob"]);
        assert_eq!(find_members(&view, "").len(), 3);
        assert!(find_members(&view, "zed").is_empty());
    }
}
