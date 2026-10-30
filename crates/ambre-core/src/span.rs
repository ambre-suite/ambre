//! Spans and instants of time, as the relay's JSON counts them.
//!
//! The relay counts a lease in milliseconds and a token's life in seconds.
//! A bare integer lets one reach a place that expects the other, a factor
//! of a thousand off. One type per unit keeps them apart, and an instant is
//! kept apart from a span. Each is `#[serde(transparent)]`, so it reads and
//! writes the bare integer.

use std::time::Duration;

use serde::{Deserialize, Serialize};

crate::define_tally! {
    /// How many times in a row one request has failed: the count a retry
    /// backs off by.
    pub struct FailureCount;
}

/// A span of time in whole milliseconds.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct Millis(pub u64);

impl Millis {
    /// The span `duration` covers, saturating past `u64::MAX` milliseconds.
    pub fn from_duration(duration: Duration) -> Self {
        Self(u64::try_from(duration.as_millis()).unwrap_or(u64::MAX))
    }

    /// The span, as the standard library counts it.
    pub const fn to_duration(self) -> Duration {
        Duration::from_millis(self.0)
    }

    /// The span `n` times over, saturating at `u64::MAX` milliseconds.
    #[must_use]
    pub const fn times(self, n: u64) -> Self {
        Self(self.0.saturating_mul(n))
    }

    /// The span doubled once per failure of `failures`, saturating at
    /// `u64::MAX` milliseconds: the delay of a retry that backs off.
    #[must_use]
    pub const fn doubled(self, failures: FailureCount) -> Self {
        match 1_u64.checked_shl(failures.get()) {
            Some(factor) => self.times(factor),
            None => Self(u64::MAX),
        }
    }

    /// The span as the `i32` a browser timer takes, saturating at
    /// `i32::MAX` milliseconds.
    pub fn to_timer_i32(self) -> i32 {
        i32::try_from(self.0).unwrap_or(i32::MAX)
    }

    /// The span as a distance between two [`PageTime`] instants.
    #[allow(clippy::cast_precision_loss)]
    pub const fn to_page_millis(self) -> PageMillis {
        PageMillis::new(self.0 as f64)
    }
}

/// A span of time in whole nanoseconds: the unit a benchmark harness reports.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct Nanos(pub u64);

impl Nanos {
    /// The span of a measured mean, a float of nanoseconds. A negative or NaN
    /// mean reads as zero, and a mean past `u64::MAX` saturates there.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    pub fn from_mean(mean: f64) -> Self {
        Self(mean.max(0.0) as u64)
    }
}

/// A span of time in whole seconds.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct Seconds(pub u64);

impl Seconds {
    /// The span, as the standard library counts it.
    pub const fn to_duration(self) -> Duration {
        Duration::from_secs(self.0)
    }
}

/// A wall-clock instant in Unix milliseconds, as a document stores it: the
/// time of a message, a comment or a reply.
///
/// Display only: a peer's clock is untrusted, so no rule orders or refuses a
/// write on it (`ambre-spec-murmure-schema.md section 2`). A view stamps it,
/// because the Logic thread reads no clock.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct Timestamp(pub u64);

impl std::fmt::Display for Timestamp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl Timestamp {
    /// The instant `span` after this one, saturating at `u64::MAX`.
    #[must_use]
    pub const fn plus(self, span: Millis) -> Self {
        Self(self.0.saturating_add(span.0))
    }

    /// The span from `earlier` to this instant, zero when `earlier` is later.
    pub const fn since(self, earlier: Self) -> Millis {
        Millis(self.0.saturating_sub(earlier.0))
    }

    /// The instant as a CRDT stores it, a signed 64-bit integer, saturating
    /// at `i64::MAX`.
    pub fn to_i64_saturating(self) -> i64 {
        i64::try_from(self.0).unwrap_or(i64::MAX)
    }

    /// An instant a CRDT stored. A negative value, which no client writes,
    /// reads as the epoch.
    pub fn from_i64(stored: i64) -> Self {
        Self(u64::try_from(stored).unwrap_or(0))
    }
}

/// A logical instant: the Lamport time a CRDT operation carries.
///
/// It counts operations, never milliseconds. Unlike a [`Timestamp`], a rule
/// may order on it: a write made after another was seen carries a greater
/// stamp. Two concurrent writes may share one, so such a rule breaks the tie
/// on something else.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct LamportStamp(pub u64);

impl std::fmt::Display for LamportStamp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl LamportStamp {
    /// The stamp as a CRDT stores it, a signed 64-bit integer, saturating at
    /// `i64::MAX`.
    pub fn to_i64_saturating(self) -> i64 {
        i64::try_from(self.0).unwrap_or(i64::MAX)
    }

    /// Read a stamp a CRDT stored. A negative value names no operation, and
    /// reads as zero, before every write.
    pub fn from_i64(stored: i64) -> Self {
        Self(u64::try_from(stored).unwrap_or(0))
    }
}

/// An instant on the page's monotonic clock, in milliseconds since the page
/// loaded, as `performance.now()` and a frame callback read it.
///
/// Never a wall clock: it starts again on every load, so no document stores
/// it. The distance between two instants is their difference, in
/// milliseconds, which a fling or a blink then measures against its own
/// constants.
#[derive(Debug, Clone, Copy, Default, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PageTime(pub f64);

impl PageTime {
    /// The instant `span` after this one.
    #[must_use]
    pub fn after(self, span: PageMillis) -> Self {
        Self(self.0 + span.get())
    }

    /// The instant `span` before this one.
    #[must_use]
    pub fn before(self, span: PageMillis) -> Self {
        Self(self.0 - span.get())
    }

    /// The span since the page loaded: the phase a periodic animation reads.
    #[must_use]
    pub fn since_load(self) -> PageMillis {
        PageMillis::new(self.0)
    }

    /// The later of the two instants.
    #[must_use]
    pub fn max(self, other: Self) -> Self {
        Self(self.0.max(other.0))
    }
}

/// The span from `earlier` to this instant, negative when `earlier` is
/// later.
impl std::ops::Sub for PageTime {
    type Output = PageMillis;

    fn sub(self, earlier: Self) -> PageMillis {
        PageMillis::new(self.0 - earlier.0)
    }
}

crate::unit::define_length! {
    /// A span of the page clock in milliseconds, in the `f64` the clock
    /// reads: the distance between two [`PageTime`] instants.
    ///
    /// A frame's cost, a blink period and a fling's decay are each one. The
    /// span may be fractional and negative, unlike a whole [`Millis`] of the
    /// relay's JSON.
    PageMillis(f64)
}

impl PageMillis {
    /// The span `duration` covers, to the fraction of a millisecond.
    #[must_use]
    pub fn from_duration(duration: Duration) -> Self {
        Self::new(duration.as_secs_f64() * 1000.0)
    }

    /// The total order of IEEE 754 over the two spans, so a list of spans
    /// sorts whatever it holds.
    #[must_use]
    pub fn total_cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0.total_cmp(&other.0)
    }
}

/// A frame rate: the frames a canvas showed per second of the page clock, as
/// a status bar reads it. Measured on the page, never stored or sent.
#[derive(Debug, Clone, Copy, Default, PartialEq, PartialOrd)]
pub struct FrameRate(f32);

impl FrameRate {
    /// No frame shown, or too few frames to measure.
    pub const ZERO: Self = Self(0.0);

    /// A rate of `per_second` frames a second.
    #[must_use]
    pub const fn new(per_second: f32) -> Self {
        Self(per_second)
    }

    /// The frames a second.
    #[must_use]
    pub const fn get(self) -> f32 {
        self.0
    }
}

/// [`Timestamp`] as a signed 64-bit integer, for a value a document already
/// stores that way: `#[serde(with = "ambre_core::span::timestamp_as_i64")]`.
/// A varint codec spells a signed and an unsigned integer apart, so the
/// field keeps the bytes it was frozen with.
pub mod timestamp_as_i64 {
    use super::Timestamp;
    use serde::{Deserialize, Deserializer, Serializer};

    /// Writes the instant as the `i64` the document holds.
    ///
    /// # Errors
    /// The serializer's own.
    pub fn serialize<S: Serializer>(ts: &Timestamp, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_i64(ts.to_i64_saturating())
    }

    /// Reads the `i64` the document holds.
    ///
    /// # Errors
    /// The deserializer's own.
    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Timestamp, D::Error> {
        i64::deserialize(deserializer).map(Timestamp::from_i64)
    }
}

/// An instant in whole milliseconds since the Unix epoch, read off the
/// relay's own clock: the sliding window of its rate limiter. Apart from a
/// [`Timestamp`], which an author's untrusted clock writes.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct UnixMillis(pub u64);

impl UnixMillis {
    /// The instant `span` after this one, saturating at `u64::MAX`.
    #[must_use]
    pub const fn plus(self, span: Millis) -> Self {
        Self(self.0.saturating_add(span.0))
    }

    /// The span from `earlier` to this instant, zero when `earlier` is later.
    #[must_use]
    pub const fn since(self, earlier: Self) -> Millis {
        Millis(self.0.saturating_sub(earlier.0))
    }

    /// The whole second the instant falls in.
    pub const fn to_unix_seconds(self) -> UnixSeconds {
        UnixSeconds(self.0 / 1000)
    }
}

/// An instant in whole seconds since the Unix epoch: the clock of a token's
/// `iat` and `exp`, and of a delete's second sweep.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct UnixSeconds(pub u64);

impl UnixSeconds {
    /// The instant `span` after this one, saturating at `u64::MAX`.
    #[must_use]
    pub const fn plus(self, span: Seconds) -> Self {
        Self(self.0.saturating_add(span.0))
    }

    /// The span from `earlier` to this instant, zero when `earlier` is later.
    pub const fn since(self, earlier: Self) -> Seconds {
        Seconds(self.0.saturating_sub(earlier.0))
    }

    /// The instant as a Postgres `BIGINT` stores it, saturating at `i64::MAX`.
    pub fn to_i64_saturating(self) -> i64 {
        i64::try_from(self.0).unwrap_or(i64::MAX)
    }
}

#[cfg(test)]
mod tests {
    use super::{LamportStamp, Millis, Seconds, Timestamp, UnixSeconds};
    use std::time::Duration;

    #[test]
    fn a_lamport_stamp_reads_back_what_a_crdt_stored() {
        assert_eq!(LamportStamp::from_i64(7).to_i64_saturating(), 7);
        assert_eq!(LamportStamp::from_i64(-1), LamportStamp(0));
        assert_eq!(LamportStamp(u64::MAX).to_i64_saturating(), i64::MAX);
        assert!(LamportStamp(3) > LamportStamp(2));
    }

    #[test]
    fn a_span_serializes_as_the_bare_integer_of_its_unit() {
        assert_eq!(serde_json::to_string(&Millis(1500)).unwrap(), "1500");
        assert_eq!(serde_json::to_string(&Seconds(900)).unwrap(), "900");
    }

    #[test]
    fn a_millisecond_span_scales_and_backs_off_without_overflow() {
        assert_eq!(Millis(2_000).times(3), Millis(6_000));
        assert_eq!(
            Millis(2_000).doubled(super::FailureCount::new(2)),
            Millis(8_000)
        );
        assert_eq!(
            Millis(2).doubled(super::FailureCount::new(64)),
            Millis(u64::MAX)
        );
        assert_eq!(Millis(u64::MAX).times(2), Millis(u64::MAX));
        assert_eq!(Millis(1_000).to_timer_i32(), 1_000);
        assert_eq!(Millis(u64::MAX).to_timer_i32(), i32::MAX);
        assert_eq!(Timestamp(10).plus(Millis(5)), Timestamp(15));
        assert_eq!(Timestamp(10).since(Timestamp(4)), Millis(6));
        assert_eq!(Timestamp(4).since(Timestamp(10)), Millis(0));
    }

    #[test]
    fn a_measured_mean_reads_as_whole_nanoseconds() {
        assert_eq!(super::Nanos::from_mean(12.7), super::Nanos(12));
        assert_eq!(super::Nanos::from_mean(-3.0), super::Nanos(0));
        assert_eq!(super::Nanos::from_mean(f64::NAN), super::Nanos(0));
        assert_eq!(serde_json::to_string(&super::Nanos(5)).unwrap(), "5");
    }

    #[test]
    fn a_span_converts_to_the_same_duration() {
        assert_eq!(Millis(1500).to_duration(), Duration::from_millis(1500));
        assert_eq!(Seconds(2).to_duration(), Duration::from_secs(2));
        assert_eq!(Millis::from_duration(Duration::from_secs(3)), Millis(3000));
    }

    #[test]
    fn two_page_instants_are_their_distance_in_milliseconds_apart() {
        let start = super::PageTime(100.0);
        let span = super::PageMillis::new(16.5);
        assert_eq!(start.after(span) - start, span);
        assert_eq!(start.before(span).after(span), start);
        assert_eq!(start.max(super::PageTime(40.0)), start);
    }

    #[test]
    fn a_timestamp_reads_back_what_a_crdt_stored() {
        assert_eq!(
            Timestamp::from_i64(Timestamp(42).to_i64_saturating()),
            Timestamp(42)
        );
        assert_eq!(Timestamp::from_i64(-1), Timestamp(0));
        assert_eq!(serde_json::to_string(&Timestamp(7)).unwrap(), "7");
    }

    #[test]
    fn a_timestamp_stored_as_i64_keeps_its_signed_spelling() {
        #[derive(serde::Serialize, serde::Deserialize, PartialEq, Debug)]
        struct Stored(#[serde(with = "super::timestamp_as_i64")] Timestamp);
        let bytes = postcard::to_allocvec(&Stored(Timestamp(3))).unwrap();
        assert_eq!(bytes, postcard::to_allocvec(&3_i64).unwrap());
        assert_eq!(
            postcard::from_bytes::<Stored>(&bytes).unwrap(),
            Stored(Timestamp(3))
        );
    }

    #[test]
    fn a_millisecond_instant_falls_in_its_second() {
        let at = super::UnixMillis(1_999).plus(Millis(1));
        assert_eq!(at, super::UnixMillis(2_000));
        assert_eq!(at.to_unix_seconds(), UnixSeconds(2));
    }

    #[test]
    fn an_instant_moves_by_a_span_and_never_goes_below_zero() {
        assert_eq!(UnixSeconds(100).plus(Seconds(20)), UnixSeconds(120));
        assert_eq!(UnixSeconds(100).since(UnixSeconds(40)), Seconds(60));
        assert_eq!(UnixSeconds(40).since(UnixSeconds(100)), Seconds(0));
        assert_eq!(UnixSeconds(u64::MAX).to_i64_saturating(), i64::MAX);
    }
}

/// An instant as RFC 3339 text, the form the relay's JSON writes a row's
/// creation and update time in, such as `2026-01-02T03:04:05+00:00`.
///
/// The relay formats it, so the type checks no syntax. A client reads it
/// through a lenient date parser, and an unreadable value reads as the epoch.
/// A distinct type keeps it apart from free text and from a name.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Rfc3339(String);

impl Rfc3339 {
    /// The instant a date formatter wrote as RFC 3339 text.
    pub fn formatted(text: String) -> Self {
        Self(text)
    }

    /// The text, for the date parser that reads it.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for Rfc3339 {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
