//! The rules of the session's time zone, which the Word mapping reads and
//! writes a date's wall time under (`PROSE-IO-084`, `PROSE-IO-085`).
//!
//! It links the IANA database, bundled, so it lives behind the `word`
//! feature: the Office module carries it and the Main instance never does.

use ambre_core::{Authored, Instant};

use crate::TimeZone;

/// A zone the database knows, its rules loaded.
#[derive(Clone, Debug)]
pub struct Zone(jiff::tz::TimeZone);

/// A zone name the database does not hold.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("the time zone of the session is not one the converter knows")]
pub struct UnknownZone;

impl Zone {
    /// The rules of `zone`.
    ///
    /// # Errors
    ///
    /// [`UnknownZone`] for a name the bundled database does not hold.
    pub fn resolve(zone: &TimeZone) -> Result<Self, UnknownZone> {
        jiff::tz::TimeZone::get(zone.name())
            .map(Self)
            .map_err(|_| UnknownZone)
    }

    /// UTC, whose wall time is the instant.
    #[must_use]
    pub fn utc() -> Self {
        Self(jiff::tz::TimeZone::UTC)
    }

    /// The instant the wall time `wall` names in this zone, `wall` held as
    /// if it were UTC. A wall time a clock skips or repeats at a change of
    /// offset reads as the offset before the change. `None` past the range
    /// a date holds.
    #[must_use]
    pub fn instant_of(&self, wall: Instant<Authored>) -> Option<Instant<Authored>> {
        let civil = jiff::tz::TimeZone::UTC.to_datetime(timestamp(wall)?);
        let instant = self.0.to_ambiguous_timestamp(civil).compatible().ok()?;
        instant_at(instant)
    }

    /// The wall time of `instant` in this zone, held as if it were UTC.
    /// `None` past the range a date holds.
    #[must_use]
    pub fn wall_of(&self, instant: Instant<Authored>) -> Option<Instant<Authored>> {
        let civil = self.0.to_datetime(timestamp(instant)?);
        instant_at(civil.to_zoned(jiff::tz::TimeZone::UTC).ok()?.timestamp())
    }
}

fn timestamp(at: Instant<Authored>) -> Option<jiff::Timestamp> {
    jiff::Timestamp::from_millisecond(i64::try_from(at.get()).ok()?).ok()
}

fn instant_at(at: jiff::Timestamp) -> Option<Instant<Authored>> {
    u64::try_from(at.as_millisecond()).ok().map(Instant::new)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paris() -> Zone {
        Zone::resolve(&TimeZone::named("Europe/Paris").expect("a name")).expect("a zone")
    }

    /// `PROSE-IO-084`: a wall time reads under the rules of its own date.
    /// 16:13 in Paris is 14:13 UTC in May, summer time, and 15:13 UTC in
    /// January, whatever the date of the import.
    #[test]
    fn a_wall_time_reads_under_the_rules_of_its_date() {
        let may = Instant::<Authored>::new(1_462_810_380_000); // 2016-05-09T16:13
        assert_eq!(
            paris().instant_of(may),
            Some(Instant::new(1_462_803_180_000))
        ); // 14:13Z
        let january = Instant::<Authored>::new(1_452_356_000_000); // 2016-01-09T16:13:20
        assert_eq!(
            paris().instant_of(january),
            Some(Instant::new(1_452_352_400_000))
        ); // 15:13:20Z
        assert_eq!(paris().wall_of(Instant::new(1_462_803_180_000)), Some(may));
        assert_eq!(Zone::utc().instant_of(may), Some(may));
    }

    /// A name the database does not hold is refused, never read as UTC.
    #[test]
    fn an_unknown_zone_is_refused() {
        let zone = TimeZone::named("Mars/Olympus_Mons").expect("a well-formed name");
        assert_eq!(Zone::resolve(&zone).err(), Some(UnknownZone));
    }
}
