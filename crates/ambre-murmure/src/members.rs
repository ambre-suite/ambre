//! The members of an open space and their presence
//! (`features/ambre-spec-murmure-members.md section 3`).
//!
//! The member rows come from the relay's table of the space document
//! (`MURMURE-031`). The presence of each member rides the ephemerals of that
//! document (`MURMURE-444`). The engine reads no clock. The view's tick stamps
//! every sample heard since the tick before, so a sample is dated to within one
//! refresh period.

use std::collections::HashMap;

use ambre_bus::{Authored, Instant, MemberView, Presence};
use ambre_core::id::{Id, User};
use ambre_core::{Ms, TimeSpan};

/// How often a client that holds a space open sends its presence sample, in
/// milliseconds (`ambre-spec-budgets.md section 8`).
pub const REFRESH_PERIOD: TimeSpan<Ms> = TimeSpan::<Ms>::new(60_000);

/// How long a presence sample keeps its member online, in milliseconds
/// (`ambre-spec-budgets.md section 8`). Three refresh periods: one for the
/// tick that dates a sample, one for a sample lost on the way, and one for
/// two drifting timers.
pub const PRESENCE_EXPIRY: TimeSpan<Ms> = REFRESH_PERIOD.times(3);

/// The first byte of a presence sample. Another ephemeral of the space
/// document takes another tag, so neither reads as the other.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
enum SampleTag {
    /// A presence sample of the space.
    Presence = 0x50,
}

impl SampleTag {
    /// The byte that opens the sample.
    const fn byte(self) -> u8 {
        self as u8
    }
}

/// The state byte of a presence sample.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
enum SampleState {
    /// The member is online.
    Online = 1,
}

impl SampleState {
    /// The byte after the tag.
    const fn byte(self) -> u8 {
        self as u8
    }
}

/// The two bytes of the one presence sample a space carries.
const SAMPLE: [u8; 2] = [SampleTag::Presence.byte(), SampleState::Online.byte()];

/// The presence sample of a space: its tag, then the state, and nothing else.
/// It names no channel, so it cannot show which channel a member reads
/// (`MURMURE-446`).
pub fn presence_sample() -> ambre_core::Payload<ambre_core::EphemeralBytes> {
    ambre_core::Payload::<ambre_core::EphemeralBytes>::new(SAMPLE.to_vec())
}

/// Whether `payload` is a presence sample of a space. A sample that does not
/// decode is dropped without an error (`PRESENCE-013`).
pub fn is_presence_sample(payload: &ambre_core::Payload<ambre_core::EphemeralBytes>) -> bool {
    payload.as_bytes() == SAMPLE
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
        .filter(|member| {
            query.is_empty() || member.display_name.as_str().to_lowercase().contains(&query)
        })
        .collect()
}

/// The members of a space, and when each one was last heard from.
///
/// `Debug` counts the members alone: each one carries the name a member
/// chose.
#[derive(Default)]
pub struct Roster {
    /// The relay's member rows: each member and their display name.
    members: Vec<(Id<User>, ambre_core::Validated<ambre_core::DisplayName>)>,
    /// The members heard from, with the tick that dates their last sample.
    /// `None` for a sample heard before the first tick: the next tick dates
    /// it.
    heard: HashMap<Id<User>, Option<Instant<Authored>>>,
    /// The latest tick of the view, `None` before the first.
    now: Option<Instant<Authored>>,
    /// This user, online as long as the space is open.
    me: Option<Id<User>>,
}

impl std::fmt::Debug for Roster {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Roster")
            .field("members", &self.members.len())
            .field("heard", &self.heard)
            .field("now", &self.now)
            .field("me", &self.me)
            .finish()
    }
}

impl Roster {
    /// A roster with no member yet, held open by `me`.
    pub fn new(me: Option<Id<User>>) -> Self {
        Self {
            me,
            ..Self::default()
        }
    }

    /// Replace the member rows with the relay's answer. A member outside it
    /// shows nowhere, whatever sample came from them (`MURMURE-440`).
    pub fn set_members(
        &mut self,
        members: impl IntoIterator<Item = (Id<User>, ambre_core::Validated<ambre_core::DisplayName>)>,
    ) {
        self.members = members.into_iter().collect();
    }

    /// Set this user, once the session names them.
    pub fn set_me(&mut self, me: Id<User>) {
        self.me = Some(me);
    }

    /// A presence sample came from `user`. Returns whether they were offline
    /// before, so the caller can answer a member who just came online with
    /// its own sample. A newcomer then sees who is online without waiting a
    /// refresh period.
    pub fn heard(&mut self, user: Id<User>) -> bool {
        self.heard.insert(user, self.now).is_none()
    }

    /// The relay says `user` stopped watching the space (`PRESENCE-014`).
    /// Returns whether they were online.
    pub fn left(&mut self, user: Id<User>) -> bool {
        self.heard.remove(&user).is_some()
    }

    /// The view's tick: date the samples heard since the tick before, and turn
    /// offline every member whose last sample is older than the expiry
    /// (`MURMURE-445`). Returns whether a member turned offline.
    pub fn tick(&mut self, now: Instant<Authored>) -> bool {
        self.now = Some(now);
        let before = self.heard.len();
        self.heard.retain(|_, stamp| {
            let at = *stamp.get_or_insert(now);
            now.since(at) <= PRESENCE_EXPIRY
        });
        self.heard.len() != before
    }

    /// The presence of `user`: this user is online, and every other member
    /// while a sample of theirs has not expired.
    pub fn presence(&self, user: Id<User>) -> Presence {
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
                member.display_name.as_str().to_lowercase(),
                *member.user_id.as_bytes(),
            )
        });
        members
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The debug form of a roster counts its members, and names none.
    #[test]
    fn a_roster_debugs_its_member_count_alone() {
        let mut roster = Roster::new(None);
        let name =
            ambre_core::Validated::<ambre_core::DisplayName>::parse("Secretary").expect("a name");
        roster.set_members([(Id::<User>::mint(), name)]);
        let shown = format!("{roster:?}");
        assert!(!shown.contains("Secretary"), "{shown}");
        assert!(shown.contains("members: 1"), "{shown}");
    }

    const T0: Instant<Authored> = Instant::<Authored>::new(1_700_000_000_000);

    fn at(offset: u64) -> Instant<Authored> {
        Instant::<Authored>::new(T0.get() + offset)
    }

    fn names(view: &[MemberView]) -> Vec<&str> {
        view.iter().map(|m| m.display_name.as_str()).collect()
    }

    fn roster(members: &[(Id<User>, &str)]) -> Roster {
        let mut roster = Roster::new(None);
        roster.set_members(members.iter().map(|(id, name)| {
            (
                *id,
                ambre_core::Validated::<ambre_core::DisplayName>::parse(*name).unwrap(),
            )
        }));
        roster.tick(T0);
        roster
    }

    /// The list holds the relay's rows and no one else: a sample from a user
    /// outside them adds no one (`MURMURE-440`).
    #[test]
    fn the_list_holds_the_member_rows_and_no_one_else() {
        let (alice, bob, mallory) = (Id::<User>::mint(), Id::<User>::mint(), Id::<User>::mint());
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
        let bob = Id::<User>::mint();
        let mut roster = roster(&[(bob, "Bob")]);
        assert_eq!(roster.presence(bob), Presence::Offline);
        assert!(roster.heard(bob), "Bob was offline");
        assert_eq!(roster.presence(bob), Presence::Online);
        assert!(
            !roster.tick(at(PRESENCE_EXPIRY.get())),
            "still within the expiry"
        );
        assert_eq!(roster.presence(bob), Presence::Online);
        assert!(roster.tick(at(PRESENCE_EXPIRY.get() + 1)));
        assert_eq!(roster.presence(bob), Presence::Offline);
    }

    /// A sample refreshed each period keeps its member online across many
    /// periods, even when every sample lands just before the next tick.
    #[test]
    fn a_member_refreshed_each_period_stays_online() {
        let bob = Id::<User>::mint();
        let mut roster = roster(&[(bob, "Bob")]);
        for period in 1..10 {
            roster.heard(bob);
            roster.tick(at(period * REFRESH_PERIOD.get()));
            assert_eq!(roster.presence(bob), Presence::Online, "period {period}");
        }
    }

    /// A sample heard before the first tick is dated by that tick, not by the
    /// epoch, so it does not expire at once.
    #[test]
    fn a_sample_before_the_first_tick_is_dated_by_it() {
        let bob = Id::<User>::mint();
        let mut roster = Roster::new(None);
        roster.set_members([(
            bob,
            ambre_core::Validated::<ambre_core::DisplayName>::parse("Bob").unwrap(),
        )]);
        roster.heard(bob);
        assert!(!roster.tick(T0));
        assert_eq!(roster.presence(bob), Presence::Online);
    }

    /// The relay's word that a member left turns them offline at once. This
    /// user stays online while the space is open.
    #[test]
    fn a_departure_turns_its_member_offline_and_this_user_stays_online() {
        let (me, bob) = (Id::<User>::mint(), Id::<User>::mint());
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
        let bob = Id::<User>::mint();
        let mut roster = roster(&[(bob, "Bob")]);
        assert!(roster.heard(bob));
        assert!(!roster.heard(bob));
    }

    /// The online members come first, then the others, each group by display
    /// name, whatever the case (`MURMURE-450`).
    #[test]
    fn the_list_shows_the_online_members_first_each_group_by_name() {
        let ids: Vec<Id<User>> = (0..4).map(|_| Id::<User>::mint()).collect();
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
        assert_eq!(sample.len(), ambre_bus::ByteSize::new(2));
        assert!(is_presence_sample(&sample));
        let sample_of =
            |bytes: &[u8]| ambre_core::Payload::<ambre_core::EphemeralBytes>::new(bytes.to_vec());
        assert!(!is_presence_sample(&sample_of(&[])));
        let (tag, online) = (SampleTag::Presence.byte(), SampleState::Online.byte());
        assert!(!is_presence_sample(&sample_of(&[tag])));
        assert!(!is_presence_sample(&sample_of(&[tag, online, 0])));
        assert!(!is_presence_sample(&sample_of(&[0, online])));
    }

    /// The sample keeps the bytes every earlier build sent: the tag `0x50`,
    /// then the state `1`. A peer on another build reads it so.
    #[test]
    fn the_sample_keeps_its_bytes() {
        assert_eq!(presence_sample(), [0x50, 1]);
        let earlier = ambre_core::Payload::<ambre_core::EphemeralBytes>::new(vec![0x50, 1]);
        assert!(is_presence_sample(&earlier));
    }

    /// The filter keeps the members whose name holds what the user types,
    /// whatever the case, in the order of the list (`MURMURE-453`).
    #[test]
    fn the_filter_keeps_the_members_whose_name_matches() {
        let roster = roster(&[
            (Id::<User>::mint(), "Bob"),
            (Id::<User>::mint(), "Alice"),
            (Id::<User>::mint(), "Jacob"),
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
