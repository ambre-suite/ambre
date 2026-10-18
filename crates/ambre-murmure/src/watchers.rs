//! Who watches an open channel, and who types in it
//! (`features/ambre-spec-murmure-typing-presence.md section 3`).
//!
//! Both indicators ride the ephemerals of the channel's `meta` document
//! (`MURMURE-121`), so they reach its members alone (`MURMURE-123`). The
//! engine reads no clock. The view ticks an open channel every
//! [`TICK_MS`], and each tick dates the samples heard since the tick before.

use std::collections::HashMap;

use ambre_bus::Timestamp;
use ambre_core::UserId;

use crate::{PRESENCE_EXPIRY_MS, REFRESH_PERIOD_MS};

/// How often the view ticks an open channel, in milliseconds
/// (`ambre-spec-budgets.md section 8`).
pub const TICK_MS: u64 = 1_000;

/// How often a member who keeps typing sends a typing sample, in
/// milliseconds (`ambre-spec-budgets.md section 8`).
pub const TYPING_REFRESH_MS: u64 = 3_000;

/// How long a typing sample keeps its member typing, in milliseconds
/// (`ambre-spec-budgets.md section 8`). Two refresh periods: one for the
/// tick that dates a sample, one for a sample lost on the way.
pub const TYPING_EXPIRY_MS: u64 = 2 * TYPING_REFRESH_MS;

/// The first byte of a channel sample. A space sample takes another tag, so
/// neither reads as the other.
const WATCHING_TAG: u8 = 0x57;
const TYPING_TAG: u8 = 0x54;

/// The state byte of a sample.
const ON: u8 = 1;

/// What a sample on the `meta` document of a channel says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sample {
    /// A connection of its member watches the channel (`MURMURE-126`).
    Watching,
    /// Its member types in the composer of the channel (`MURMURE-124`).
    Typing,
}

impl Sample {
    /// The bytes of the sample: its tag, then the state, and nothing else.
    /// They name no channel, so a sample cannot show which other channel a
    /// member reads (`MURMURE-127`).
    pub fn bytes(self) -> Vec<u8> {
        match self {
            Self::Watching => vec![WATCHING_TAG, ON],
            Self::Typing => vec![TYPING_TAG, ON],
        }
    }

    /// The sample `payload` holds. One that does not decode is `None`, and
    /// is dropped without an error (`PRESENCE-013`).
    pub fn read(payload: &[u8]) -> Option<Self> {
        match payload {
            [WATCHING_TAG, ON] => Some(Self::Watching),
            [TYPING_TAG, ON] => Some(Self::Typing),
            _ => None,
        }
    }
}

/// The other members who watch an open channel and who type in it, and when
/// this client last sent its own samples.
#[derive(Debug, Default)]
pub struct Watchers {
    /// The members heard watching, with the tick that dates their last
    /// sample. `None` for a sample heard before the first tick.
    watching: HashMap<UserId, Option<Timestamp>>,
    /// The members heard typing, dated the same way.
    typing: HashMap<UserId, Option<Timestamp>>,
    /// The latest tick of the view, `None` before the first.
    now: Option<Timestamp>,
    /// When this client last sent each sample.
    sent_watching: Option<Timestamp>,
    sent_typing: Option<Timestamp>,
}

impl Watchers {
    /// A sample came from `user`. Returns whether the indicators changed. A
    /// member who types watches too.
    pub fn heard(&mut self, user: UserId, sample: Sample) -> bool {
        let came = self.watching.insert(user, self.now).is_none();
        match sample {
            Sample::Watching => came,
            Sample::Typing => self.typing.insert(user, self.now).is_none() || came,
        }
    }

    /// Whether `user` was not heard watching before: the caller answers a
    /// newcomer with its own sample, so they see who watches at once.
    pub fn is_new(&self, user: UserId) -> bool {
        !self.watching.contains_key(&user)
    }

    /// The relay says `user` stopped watching the channel (`SRV-097`).
    /// Returns whether the indicators changed.
    pub fn left(&mut self, user: UserId) -> bool {
        let typed = self.typing.remove(&user).is_some();
        self.watching.remove(&user).is_some() || typed
    }

    /// The view's tick: date the samples heard since the tick before, and
    /// drop every one older than its expiry. Returns whether the indicators
    /// changed.
    pub fn tick(&mut self, now: Timestamp) -> bool {
        self.now = Some(now);
        let expire = |heard: &mut HashMap<UserId, Option<Timestamp>>, expiry: u64| {
            let before = heard.len();
            heard.retain(|_, stamp| {
                let at = *stamp.get_or_insert(now);
                now.0.saturating_sub(at.0) <= expiry
            });
            heard.len() != before
        };
        let typing = expire(&mut self.typing, TYPING_EXPIRY_MS);
        expire(&mut self.watching, PRESENCE_EXPIRY_MS) || typing
    }

    /// Whether this client owes its watching sample at `now`: none sent yet,
    /// or one refresh period since the last (`MURMURE-126`). Records it as
    /// sent when it is owed.
    pub fn owes_watching(&mut self, now: Timestamp) -> bool {
        owed(&mut self.sent_watching, now, REFRESH_PERIOD_MS)
    }

    /// Whether a keystroke at `now` owes a typing sample: none sent yet, or
    /// one typing refresh period since the last (`MURMURE-124`). Records it
    /// as sent when it is owed.
    pub fn owes_typing(&mut self, now: Timestamp) -> bool {
        owed(&mut self.sent_typing, now, TYPING_REFRESH_MS)
    }

    /// The members who watch, then those who type, each in the order of
    /// their identifiers, so the view never reorders between two draws.
    pub fn view(&self) -> (Vec<UserId>, Vec<UserId>) {
        let sorted = |heard: &HashMap<UserId, Option<Timestamp>>| {
            let mut users: Vec<UserId> = heard.keys().copied().collect();
            users.sort_by_key(|user| *user.as_bytes());
            users
        };
        (sorted(&self.watching), sorted(&self.typing))
    }
}

/// Whether a sample last sent at `sent` is owed again at `now`, recording it.
fn owed(sent: &mut Option<Timestamp>, now: Timestamp, period: u64) -> bool {
    let due = sent.is_none_or(|at| now.0.saturating_sub(at.0) >= period);
    if due {
        *sent = Some(now);
    }
    due
}

#[cfg(test)]
mod tests {
    use super::*;

    const T0: Timestamp = Timestamp(1_700_000_000_000);

    fn at(offset: u64) -> Timestamp {
        Timestamp(T0.0 + offset)
    }

    fn ticked() -> Watchers {
        let mut watchers = Watchers::default();
        watchers.tick(T0);
        watchers
    }

    // --- MURMURE-124 ---

    /// A keystroke owes a typing sample once per refresh period, and a member
    /// heard typing stops once no sample refreshes them within the expiry.
    #[test]
    fn typing_refreshes_while_it_lasts_and_expires_once_it_stops() {
        let mut watchers = ticked();
        assert!(watchers.owes_typing(T0), "the first keystroke sends");
        assert!(!watchers.owes_typing(at(TYPING_REFRESH_MS - 1)));
        assert!(watchers.owes_typing(at(TYPING_REFRESH_MS)));

        let bob = UserId::new();
        assert!(watchers.heard(bob, Sample::Typing));
        assert_eq!(watchers.view(), (vec![bob], vec![bob]));
        assert!(!watchers.tick(at(TYPING_EXPIRY_MS)), "within the expiry");
        assert!(watchers.tick(at(TYPING_EXPIRY_MS + 1)));
        assert_eq!(
            watchers.view(),
            (vec![bob], Vec::new()),
            "Bob still watches"
        );
    }

    // --- MURMURE-126 ---

    /// A member watches while their samples keep coming, and stops at the
    /// expiry or when the relay says they left. This client owes its own
    /// sample once per refresh period.
    #[test]
    fn a_member_watches_while_their_samples_come_and_stops_when_they_leave() {
        let mut watchers = ticked();
        let (bob, carol) = (UserId::new(), UserId::new());
        assert!(watchers.is_new(bob));
        assert!(watchers.heard(bob, Sample::Watching));
        assert!(!watchers.is_new(bob));
        assert!(!watchers.heard(bob, Sample::Watching), "no change");
        assert!(watchers.heard(carol, Sample::Watching));

        assert!(watchers.left(carol));
        assert!(!watchers.left(carol));
        assert!(!watchers.tick(at(PRESENCE_EXPIRY_MS)));
        assert!(watchers.tick(at(PRESENCE_EXPIRY_MS + 1)));
        assert_eq!(watchers.view(), (Vec::new(), Vec::new()));

        assert!(watchers.owes_watching(T0));
        assert!(!watchers.owes_watching(at(REFRESH_PERIOD_MS - 1)));
        assert!(watchers.owes_watching(at(REFRESH_PERIOD_MS)));
    }

    // --- MURMURE-127 ---

    /// A sample is its tag and its state: it names no channel. A payload of
    /// any other shape reads as no sample.
    #[test]
    fn a_sample_names_no_channel_and_nothing_else_reads_as_one() {
        for sample in [Sample::Watching, Sample::Typing] {
            assert_eq!(sample.bytes().len(), 2);
            assert_eq!(Sample::read(&sample.bytes()), Some(sample));
        }
        assert_ne!(Sample::Watching.bytes(), crate::presence_sample());
        for payload in [&[][..], &[WATCHING_TAG], &[TYPING_TAG, ON, 0], &[0, ON]] {
            assert_eq!(Sample::read(payload), None);
        }
    }
}
