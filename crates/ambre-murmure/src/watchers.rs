//! Who watches an open channel, and who types in it
//! (`features/ambre-spec-murmure-typing-presence.md section 3`).
//!
//! Both indicators ride the ephemerals of the channel's `meta` document
//! (`MURMURE-121`), so they reach its members alone (`MURMURE-123`). The
//! engine reads no clock. The view ticks an open channel every
//! [`TICK_PERIOD`], and each tick dates the samples heard since the tick before.

use std::collections::HashMap;

use ambre_bus::Timestamp;
use ambre_core::{ByteSize, MessageId, Millis, UserId};

use crate::{PRESENCE_EXPIRY, REFRESH_PERIOD};

/// How often the view ticks an open channel, in milliseconds
/// (`ambre-spec-budgets.md section 8`).
pub const TICK_PERIOD: Millis = Millis(1_000);

/// How often a member who keeps typing sends a typing sample, in
/// milliseconds (`ambre-spec-budgets.md section 8`).
pub const TYPING_REFRESH: Millis = Millis(3_000);

/// How long a typing sample keeps its member typing, in milliseconds
/// (`ambre-spec-budgets.md section 8`). Two refresh periods: one for the
/// tick that dates a sample, one for a sample lost on the way.
pub const TYPING_EXPIRY: Millis = TYPING_REFRESH.times(2);

/// The first byte of a channel sample. A space sample takes another tag, so
/// neither reads as the other.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
enum SampleTag {
    /// A connection watches the channel.
    Watching = 0x57,
    /// Its member types in the channel's composer.
    Typing = 0x54,
    /// Its member types in a thread's composer.
    TypingIn = 0x74,
}

impl SampleTag {
    /// The byte that opens the sample.
    const fn byte(self) -> u8 {
        self as u8
    }

    /// The tag `byte` names, `None` for a byte no sample opens with.
    fn from_byte(byte: u8) -> Option<Self> {
        [Self::Watching, Self::Typing, Self::TypingIn]
            .into_iter()
            .find(|tag| tag.byte() == byte)
    }
}

/// The state byte of a sample.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
enum SampleState {
    /// The sample holds.
    On = 1,
}

impl SampleState {
    /// The byte after the tag.
    const fn byte(self) -> u8 {
        self as u8
    }
}

/// The length of every sample: a tag, the state, and a root or zeros. One
/// length for all, so the sealed size never tells a thread from its channel.
const SAMPLE_LEN: ByteSize = ByteSize(18);

/// What a sample on the `meta` document of a channel says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sample {
    /// A connection of its member watches the channel (`MURMURE-126`).
    Watching,
    /// Its member types in the composer of the channel (`MURMURE-124`).
    Typing,
    /// Its member types in the composer of the thread under this root
    /// (`MURMURE-536`).
    TypingIn(MessageId),
}

impl Sample {
    /// The sample a keystroke sends, in the thread under `thread` or in the
    /// channel.
    pub fn typing(thread: Option<MessageId>) -> Self {
        thread.map_or(Self::Typing, Self::TypingIn)
    }

    /// The bytes of the sample: its tag, the state, then a thread's root or
    /// zeros. They name no channel, so a sample cannot show which other
    /// channel a member reads (`MURMURE-127`).
    pub fn bytes(self) -> ambre_core::EphemeralBytes {
        let (tag, root) = match self {
            Self::Watching => (SampleTag::Watching, [0; 16]),
            Self::Typing => (SampleTag::Typing, [0; 16]),
            Self::TypingIn(root) => (SampleTag::TypingIn, *root.as_bytes()),
        };
        let mut bytes = Vec::with_capacity(SAMPLE_LEN.as_len());
        bytes.extend([tag.byte(), SampleState::On.byte()]);
        bytes.extend(root);
        ambre_core::EphemeralBytes::new(bytes)
    }

    /// The sample `payload` holds. One that does not decode is `None`, and
    /// is dropped without an error (`PRESENCE-013`).
    pub fn read(payload: &ambre_core::EphemeralBytes) -> Option<Self> {
        let [tag, state, rest @ ..] = payload.as_bytes() else {
            return None;
        };
        if *state != SampleState::On.byte() {
            return None;
        }
        let root: [u8; 16] = rest.try_into().ok()?;
        let zeros = root == [0; 16];
        match SampleTag::from_byte(*tag)? {
            SampleTag::Watching if zeros => Some(Self::Watching),
            SampleTag::Typing if zeros => Some(Self::Typing),
            SampleTag::TypingIn if !zeros => Some(Self::TypingIn(MessageId::from_bytes(root))),
            _ => None,
        }
    }
}

/// Where a member types: the channel's composer, or a thread's by its root.
type Place = Option<MessageId>;

/// The other members who watch an open channel and who type in it, and when
/// this client last sent its own samples.
#[derive(Debug, Default)]
pub struct Watchers {
    /// The members heard watching, with the tick that dates their last
    /// sample. `None` for a sample heard before the first tick.
    watching: HashMap<UserId, Option<Timestamp>>,
    /// The members heard typing, dated the same way, with where they type.
    /// A member types in one place, the last their samples named.
    typing: HashMap<UserId, (Option<Timestamp>, Place)>,
    /// The latest tick of the view, `None` before the first.
    now: Option<Timestamp>,
    /// When this client last sent each sample, and where it typed.
    sent_watching: Option<Timestamp>,
    sent_typing: Option<(Timestamp, Place)>,
}

impl Watchers {
    /// A sample came from `user`. Returns whether the indicators changed. A
    /// member who types watches too.
    pub fn heard(&mut self, user: UserId, sample: Sample) -> bool {
        let came = self.watching.insert(user, self.now).is_none();
        let place = match sample {
            Sample::Watching => return came,
            Sample::Typing => None,
            Sample::TypingIn(root) => Some(root),
        };
        let before = self.typing.insert(user, (self.now, place));
        before.is_none_or(|(_, was)| was != place) || came
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
        let fresh = |stamp: &mut Option<Timestamp>, expiry: Millis| {
            let at = *stamp.get_or_insert(now);
            now.since(at) <= expiry
        };
        let (watching, typing) = (self.watching.len(), self.typing.len());
        self.typing
            .retain(|_, (stamp, _)| fresh(stamp, TYPING_EXPIRY));
        self.watching
            .retain(|_, stamp| fresh(stamp, PRESENCE_EXPIRY));
        self.watching.len() != watching || self.typing.len() != typing
    }

    /// Whether this client owes its watching sample at `now`: none sent yet,
    /// or one refresh period since the last (`MURMURE-126`). Records it as
    /// sent when it is owed.
    pub fn owes_watching(&mut self, now: Timestamp) -> bool {
        owed(&mut self.sent_watching, now, REFRESH_PERIOD)
    }

    /// Whether a keystroke at `now` owes a typing sample. The place is the
    /// thread under `thread`, or the channel with `None`. It is owed when
    /// none was sent, a typing refresh period after the last (`MURMURE-124`),
    /// or at once in a new place (`MURMURE-536`). Records it as sent when owed.
    pub fn owes_typing(&mut self, now: Timestamp, thread: Option<MessageId>) -> bool {
        let moved = self.sent_typing.is_some_and(|(_, place)| place != thread);
        let mut sent = self.sent_typing.map(|(at, _)| at).filter(|_| !moved);
        let due = owed(&mut sent, now, TYPING_REFRESH);
        if due {
            self.sent_typing = Some((now, thread));
        }
        due
    }

    /// The members who watch, then those who type in the channel. Each list
    /// keeps the order of the identifiers, so the view never reorders between
    /// two draws.
    pub fn view(&self) -> (Vec<UserId>, Vec<UserId>) {
        let mut watching: Vec<UserId> = self.watching.keys().copied().collect();
        watching.sort_by_key(|user| *user.as_bytes());
        (watching, self.typing_in(None))
    }

    /// The members who type in each thread, by its root, the roots and the
    /// members in the order of their identifiers (`MURMURE-536`).
    pub fn in_threads(&self) -> Vec<(MessageId, Vec<UserId>)> {
        let mut roots: Vec<MessageId> = self.typing.values().filter_map(|(_, at)| *at).collect();
        roots.sort_by_key(|root| *root.as_bytes());
        roots.dedup();
        roots
            .into_iter()
            .map(|root| (root, self.typing_in(Some(root))))
            .collect()
    }

    /// The members who type at `place`, in the order of their identifiers.
    fn typing_in(&self, place: Place) -> Vec<UserId> {
        let mut users: Vec<UserId> = self
            .typing
            .iter()
            .filter(|(_, (_, at))| *at == place)
            .map(|(user, _)| *user)
            .collect();
        users.sort_by_key(|user| *user.as_bytes());
        users
    }
}

/// Whether a sample last sent at `sent` is owed again at `now`, recording it.
fn owed(sent: &mut Option<Timestamp>, now: Timestamp, period: Millis) -> bool {
    let due = sent.is_none_or(|at| now.since(at) >= period);
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
        assert!(watchers.owes_typing(T0, None), "the first keystroke sends");
        assert!(!watchers.owes_typing(at(TYPING_REFRESH.0 - 1), None));
        assert!(watchers.owes_typing(at(TYPING_REFRESH.0), None));

        let bob = UserId::new();
        assert!(watchers.heard(bob, Sample::Typing));
        assert_eq!(watchers.view(), (vec![bob], vec![bob]));
        assert!(!watchers.tick(at(TYPING_EXPIRY.0)), "within the expiry");
        assert!(watchers.tick(at(TYPING_EXPIRY.0 + 1)));
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
        assert!(!watchers.tick(at(PRESENCE_EXPIRY.0)));
        assert!(watchers.tick(at(PRESENCE_EXPIRY.0 + 1)));
        assert_eq!(watchers.view(), (Vec::new(), Vec::new()));

        assert!(watchers.owes_watching(T0));
        assert!(!watchers.owes_watching(at(REFRESH_PERIOD.0 - 1)));
        assert!(watchers.owes_watching(at(REFRESH_PERIOD.0)));
    }

    // --- MURMURE-127 ---

    /// A sample is its tag, its state, and a root or zeros: it names no
    /// channel, and every one has the same length. A payload of any other
    /// shape reads as no sample.
    #[test]
    fn a_sample_names_no_channel_and_nothing_else_reads_as_one() {
        let root = MessageId::new();
        for sample in [Sample::Watching, Sample::Typing, Sample::TypingIn(root)] {
            assert_eq!(ByteSize::of_len(sample.bytes().len()), SAMPLE_LEN);
            assert_eq!(Sample::read(&sample.bytes()), Some(sample));
        }
        assert_ne!(Sample::Watching.bytes(), crate::presence_sample());
        let mut zero_root = Sample::Typing.bytes().into_bytes();
        zero_root[0] = SampleTag::TypingIn.byte();
        let mut rooted_watch = Sample::TypingIn(root).bytes().into_bytes();
        rooted_watch[0] = SampleTag::Watching.byte();
        let short = &Sample::Typing.bytes().into_bytes()[..SAMPLE_LEN.as_len() - 1];
        let mut unknown_tag = Sample::Typing.bytes().into_bytes();
        unknown_tag[0] = 0x55;
        let mut off = Sample::Typing.bytes().into_bytes();
        off[1] = 0;
        for payload in [
            &[][..],
            &[SampleTag::Watching.byte(), SampleState::On.byte()],
            short,
            &zero_root,
            &rooted_watch,
            &unknown_tag,
            &off,
        ] {
            let payload = ambre_core::EphemeralBytes::new(payload.to_vec());
            assert_eq!(Sample::read(&payload), None);
        }
    }

    /// Each sample keeps the bytes every earlier build sent: its tag, the
    /// state `1`, then the root or sixteen zeros. A peer on another build
    /// reads them so.
    #[test]
    fn a_sample_keeps_its_bytes() {
        let root = MessageId::from_bytes([0x42; 16]);
        let with = |tag: u8, rest: [u8; 16]| {
            let mut bytes = vec![tag, 1];
            bytes.extend(rest);
            bytes
        };
        assert_eq!(Sample::Watching.bytes(), with(0x57, [0; 16]));
        assert_eq!(Sample::Typing.bytes(), with(0x54, [0; 16]));
        assert_eq!(Sample::TypingIn(root).bytes(), with(0x74, [0x42; 16]));
        for sample in [Sample::Watching, Sample::Typing, Sample::TypingIn(root)] {
            let earlier = ambre_core::EphemeralBytes::new(sample.bytes().into_bytes());
            assert_eq!(Sample::read(&earlier), Some(sample));
        }
    }

    // --- MURMURE-536 ---

    /// A member typing in a thread types there alone: the channel's list
    /// leaves them out, and their move back to the channel changes both.
    /// This client owes a sample at once on a new place.
    #[test]
    fn typing_in_a_thread_is_told_apart_from_the_channel() {
        let mut watchers = ticked();
        let (bob, carol, root) = (UserId::new(), UserId::new(), MessageId::new());
        assert!(watchers.heard(bob, Sample::TypingIn(root)));
        assert!(!watchers.heard(bob, Sample::TypingIn(root)), "no change");
        assert!(watchers.heard(carol, Sample::Typing));
        assert_eq!(watchers.view().1, [carol]);
        assert_eq!(watchers.in_threads(), [(root, vec![bob])]);
        assert!(watchers.heard(bob, Sample::Typing), "Bob moved");
        assert!(watchers.in_threads().is_empty());

        assert!(watchers.owes_typing(T0, None));
        assert!(watchers.owes_typing(at(1), Some(root)), "a new place");
        assert!(!watchers.owes_typing(at(2), Some(root)));
        assert!(watchers.owes_typing(at(TYPING_REFRESH.0 + 1), Some(root)));
        assert_eq!(Sample::typing(Some(root)), Sample::TypingIn(root));
        assert_eq!(Sample::typing(None), Sample::Typing);
    }
}
