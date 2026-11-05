//! Spans and instants of time, one marker type per unit and per time base.
//!
//! The relay counts a lease in milliseconds and a token's life in seconds.
//! A bare integer lets one reach a place that expects the other, a factor
//! of a thousand off. A [`TimeSpan`] carries its unit as a type parameter,
//! and an [`Instant`] carries its time base, so the compiler refuses the
//! slip. An instant is kept apart from a span. Each reads and writes the
//! bare number of its unit, as `#[serde(transparent)]` writes it.
//!
//! | Unit | Number | Example |
//! | :--- | :--- | :--- |
//! | [`Ms`] | `u64` | a lease, a retry delay |
//! | [`Sec`] | `u64` | a token's life, a retention window |
//! | [`Ns`] | `u64` | a benchmark mean |
//! | [`FracMs`] | `f64` | a frame's cost, a blink period |
//! | [`Day`] | `i64` | the days between two spreadsheet dates |
//! | [`FracDay`] | `f64` | a spreadsheet date with its time of day |
//!
//! | Time base | Unit | Example |
//! | :--- | :--- | :--- |
//! | [`Authored`] | [`Ms`] | the time of a message, which an author's clock writes |
//! | [`Relayed`] | [`Ms`] | the update time of a listed document, which the relay stamps |
//! | [`UnixMs`] | [`Ms`] | the window of the relay's rate limiter |
//! | [`UnixSec`] | [`Sec`] | a token's `exp`, a delete's second sweep |
//! | [`PageLoad`] | [`FracMs`] | a frame callback, read on the page's monotonic clock |
//! | [`SheetDay`] | [`Day`] | a whole spreadsheet date, its days from 1899-12-30 |
//! | [`SheetDateTime`] | [`FracDay`] | a spreadsheet date with its time of day in the fraction |
//!
//! Both traits are sealed: each marker is an enum with no variant, and the
//! list in this module is the whole set. The two spreadsheet bases live here
//! for that reason: the seal keeps every unit in this module, though only
//! the Trame calendar reads them.
//!
//! A unit a clock reads divides one second, so it converts to and from a
//! [`Duration`]: it implements [`ClockUnit`]. A day is no such unit. A
//! spreadsheet counts its days apart from any clock, so [`Day`] and
//! [`FracDay`] stay off that trait and never meet a [`Duration`].

use crate::Count;
use std::cmp::Ordering;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::marker::PhantomData;
use std::ops::{Add, AddAssign, Div, Mul, Neg, Sub, SubAssign};
use std::time::Duration;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// How many times in a row one request has failed: the count a retry
/// backs off by.
pub enum Failures {}

impl crate::CountKind for Failures {
    type Repr = u32;
    const NAME: &'static str = "FailureCount";
}

mod sealed {
    /// Closes [`super::Tick`], [`super::TimeUnit`] and [`super::TimeBase`]
    /// to the types this module lists.
    pub trait Sealed {}

    impl Sealed for u64 {}
    impl Sealed for i64 {}
    impl Sealed for f64 {}
    impl Sealed for super::Ms {}
    impl Sealed for super::Sec {}
    impl Sealed for super::Ns {}
    impl Sealed for super::FracMs {}
    impl Sealed for super::Day {}
    impl Sealed for super::FracDay {}
    impl Sealed for super::Months {}
    impl Sealed for super::Authored {}
    impl Sealed for super::Relayed {}
    impl Sealed for super::UnixMs {}
    impl Sealed for super::UnixSec {}
    impl Sealed for super::PageLoad {}
    impl Sealed for super::SheetDay {}
    impl Sealed for super::SheetDateTime {}
}

/// The number a unit counts its ticks in: a whole `u64`, a signed `i64` or
/// an `f64`.
///
/// The trait holds what every number shares. A whole `u64` saturates at
/// zero where a method of its own says so; a signed `i64` keeps its sign.
pub trait Tick:
    sealed::Sealed
    + Copy
    + Default
    + PartialEq
    + PartialOrd
    + fmt::Debug
    + fmt::Display
    + Serialize
    + DeserializeOwned
    + 'static
{
    /// The count of zero ticks.
    const ZERO: Self;
}

/// A number a clock unit counts in, so a [`Duration`] converts into it.
pub trait ClockTick: Tick {
    /// The ticks of the unit `U` that `duration` covers. A whole count
    /// rounds down and saturates at `u64::MAX`. The bound keeps a unit to
    /// the number it holds, so `u64` never counts a float unit.
    fn of_duration<U: ClockUnit<Repr = Self>>(duration: Duration) -> Self;
}

impl Tick for u64 {
    const ZERO: Self = 0;
}

impl Tick for i64 {
    const ZERO: Self = 0;
}

impl Tick for f64 {
    const ZERO: Self = 0.0;
}

impl ClockTick for u64 {
    fn of_duration<U: ClockUnit<Repr = Self>>(duration: Duration) -> Self {
        let ticks = duration
            .as_nanos()
            .saturating_mul(u128::from(U::PER_SECOND))
            / u128::from(Ns::PER_SECOND);
        Self::try_from(ticks).unwrap_or(Self::MAX)
    }
}

impl ClockTick for f64 {
    fn of_duration<U: ClockUnit<Repr = Self>>(duration: Duration) -> Self {
        duration.as_secs_f64() * U::PER_SECOND as Self
    }
}

/// The marker of one unit of time.
///
/// The unit fixes the number a span holds, so a unit never holds two kinds
/// of number.
pub trait TimeUnit: sealed::Sealed + 'static {
    /// The number one tick of the unit counts in.
    type Repr: Tick;

    /// The name `Debug` prints, as the hand-written type printed its own.
    const NAME: &'static str;
}

/// A unit a clock reads. It divides one second into a whole count of ticks
/// that also divides a billion, so a whole span converts to a [`Duration`]
/// exactly.
pub trait ClockUnit: TimeUnit<Repr: ClockTick> {
    /// The ticks of the unit in one second.
    const PER_SECOND: u64;
}

/// The marker of one time base: the clock an instant was read on, and the
/// epoch it counts from.
///
/// Two bases of one unit stay apart. A peer's clock is untrusted, and the
/// relay's is not, so an instant of one never compares to the other.
pub trait TimeBase: sealed::Sealed + 'static {
    /// The unit an instant of the base counts in.
    type Unit: TimeUnit;

    /// The name `Debug` prints, as the hand-written type printed its own.
    const NAME: &'static str;
}

/// Whole milliseconds.
pub enum Ms {}

/// Whole seconds.
pub enum Sec {}

/// Whole nanoseconds: the unit a benchmark harness reports.
pub enum Ns {}

/// Milliseconds in the `f64` the page clock reads. A span of the unit may
/// be fractional and negative, unlike a whole [`Ms`] of the relay's JSON.
pub enum FracMs {}

impl TimeUnit for Ms {
    type Repr = u64;
    const NAME: &'static str = "Millis";
}

impl ClockUnit for Ms {
    const PER_SECOND: u64 = 1_000;
}

impl TimeUnit for Sec {
    type Repr = u64;
    const NAME: &'static str = "Seconds";
}

impl ClockUnit for Sec {
    const PER_SECOND: u64 = 1;
}

impl TimeUnit for Ns {
    type Repr = u64;
    const NAME: &'static str = "Nanos";
}

impl ClockUnit for Ns {
    const PER_SECOND: u64 = 1_000_000_000;
}

/// Whole days of a spreadsheet calendar. A count of them is signed: the days
/// from a later date back to an earlier one are negative.
///
/// A day is no clock unit, so no [`Duration`] converts into it:
///
/// ```compile_fail,E0599
/// use ambre_core::{Day, TimeSpan};
/// let _ = TimeSpan::<Day>::from_duration(std::time::Duration::from_secs(86_400));
/// ```
///
/// And a spreadsheet date never meets an instant of a clock:
///
/// ```compile_fail,E0308
/// use ambre_core::{Instant, SheetDay, UnixSec};
/// let _ = Instant::<SheetDay>::new(3) - Instant::<UnixSec>::new(3);
/// ```
pub enum Day {}

/// Days of a spreadsheet calendar in an `f64`, the time of day in the
/// fraction.
pub enum FracDay {}

impl TimeUnit for FracMs {
    type Repr = f64;
    const NAME: &'static str = "PageMillis";
}

impl ClockUnit for FracMs {
    const PER_SECOND: u64 = 1_000;
}

impl TimeUnit for Day {
    type Repr = i64;
    const NAME: &'static str = "DayCount";
}

impl TimeUnit for FracDay {
    type Repr = f64;
    const NAME: &'static str = "FracDays";
}

/// Whole months of a spreadsheet calendar, as `DATE` and `EDATE` count them
/// from January of some year. A count of them is signed, and it may run past
/// December or below January. A month has no fixed length, so a span of months
/// converts into no other unit.
pub enum Months {}

impl TimeUnit for Months {
    type Repr = i64;
    const NAME: &'static str = "MonthCount";
}

impl TimeSpan<Months> {
    /// The two spans together, held at either end of `i64`.
    #[must_use]
    pub const fn plus(self, other: Self) -> Self {
        Self::new(self.ticks.saturating_add(other.ticks))
    }
}

/// A wall clock in Unix milliseconds, as an author's clock reads it. A
/// document stores its instant: the time of a message, a comment or a reply.
///
/// Display only: a peer's clock is untrusted, so no rule orders or refuses a
/// write on it (`ambre-spec-murmure-schema.md section 2`). A view stamps it,
/// because the Logic thread reads no clock.
pub enum Authored {}

/// A wall clock in Unix milliseconds, as the relay's clock reads it when the
/// relay writes a row: the update time of a listed document.
///
/// A client reads it and stamps nothing on it. Apart from [`Authored`], which
/// an untrusted author's clock writes, so one never stands for the other:
///
/// ```compile_fail,E0308
/// use ambre_core::{Authored, Instant, Relayed};
/// let _: Instant<Relayed> = Instant::<Authored>::new(3);
/// ```
///
/// Apart from [`UnixMs`] too, the reader's own clock. The two meet at
/// [`Instant::age_at`] alone, for display, and never through `since`:
///
/// ```compile_fail,E0308
/// use ambre_core::{Instant, Relayed, UnixMs};
/// let _ = Instant::<Relayed>::new(3).since(Instant::<UnixMs>::new(1));
/// ```
pub enum Relayed {}

/// Whole milliseconds since the Unix epoch, read off the reader's own
/// clock. The sliding window of the relay's rate limiter reads it, and so
/// does the deadline of a client's reconnect. Apart from [`Authored`],
/// which an untrusted author's clock writes.
pub enum UnixMs {}

/// Whole seconds since the Unix epoch: the clock of a token's `iat` and
/// `exp`, and of a delete's second sweep.
pub enum UnixSec {}

/// The page's monotonic clock, in milliseconds since the page loaded, as
/// `performance.now()` and a frame callback read it.
///
/// Never a wall clock: it starts again on every load, so no document stores
/// an instant of it.
pub enum PageLoad {}

/// Whole days from the spreadsheet epoch, 1899-12-30, as a formula reads a
/// date. Every count of days is a date, before the epoch too.
pub enum SheetDay {}

/// Days from the spreadsheet epoch with the time of day in the fraction, as
/// a cell's number reads as a date and a time.
pub enum SheetDateTime {}

impl TimeBase for Authored {
    type Unit = Ms;
    const NAME: &'static str = "Timestamp";
}

impl TimeBase for Relayed {
    type Unit = Ms;
    const NAME: &'static str = "RelayMillis";
}

impl TimeBase for UnixMs {
    type Unit = Ms;
    const NAME: &'static str = "UnixMillis";
}

impl TimeBase for UnixSec {
    type Unit = Sec;
    const NAME: &'static str = "UnixSeconds";
}

impl TimeBase for PageLoad {
    type Unit = FracMs;
    const NAME: &'static str = "PageTime";
}

impl TimeBase for SheetDay {
    type Unit = Day;
    const NAME: &'static str = "DaySerial";
}

impl TimeBase for SheetDateTime {
    type Unit = FracDay;
    const NAME: &'static str = "DateSerial";
}

/// A span of time, counted in the unit `U` names.
///
/// The unit fixes the number the span holds (`TimeUnit::Repr`). A span of
/// one unit meets a span of another only through [`TimeSpan::to_floor`] or
/// a named method. `new` wraps the number, and `get` unwraps it for a
/// foreign API. No rule bounds a span, so the type has no fallible
/// constructor.
pub struct TimeSpan<U: TimeUnit> {
    ticks: U::Repr,
    unit: PhantomData<fn() -> U>,
}

impl<U: TimeUnit> TimeSpan<U> {
    /// The span of zero ticks.
    pub const ZERO: Self = Self::new(<U::Repr as Tick>::ZERO);

    /// The span of `ticks` units. The constructor checks nothing.
    #[must_use]
    pub const fn new(ticks: U::Repr) -> Self {
        Self {
            ticks,
            unit: PhantomData,
        }
    }

    /// The number of units, for a foreign API that takes the number.
    #[must_use]
    pub const fn get(self) -> U::Repr {
        self.ticks
    }
}

impl<U: ClockUnit> TimeSpan<U> {
    /// The span `duration` covers. A whole unit rounds down and saturates
    /// at `u64::MAX`; a float unit keeps the fraction.
    #[must_use]
    pub fn from_duration(duration: Duration) -> Self {
        Self::new(<U::Repr as ClockTick>::of_duration::<U>(duration))
    }
}

impl<U: ClockUnit<Repr = u64>> TimeSpan<U> {
    /// The span in the unit `V`, rounded down to a whole tick of `V` and
    /// saturating at `u64::MAX` ticks.
    #[must_use]
    pub const fn to_floor<V: ClockUnit<Repr = u64>>(self) -> TimeSpan<V> {
        let wide = (self.ticks as u128) * (V::PER_SECOND as u128) / (U::PER_SECOND as u128);
        TimeSpan::new(if wide > u64::MAX as u128 {
            u64::MAX
        } else {
            wide as u64
        })
    }

    /// The span, as the standard library counts it. The unit divides a
    /// billion, so the conversion is exact.
    #[must_use]
    pub const fn to_duration(self) -> Duration {
        let secs = self.ticks / U::PER_SECOND;
        let rest = self.ticks % U::PER_SECOND;
        // `rest` is below `PER_SECOND`, so the nanoseconds stay below a
        // billion and fit the `u32` of `Duration::new`.
        let nanos = rest * (1_000_000_000 / U::PER_SECOND);
        Duration::new(secs, nanos as u32)
    }

    /// The span `n` times over, saturating at `u64::MAX` ticks.
    #[must_use]
    pub const fn times(self, n: u64) -> Self {
        Self::new(self.ticks.saturating_mul(n))
    }

    /// The span doubled once per failure of `failures`, saturating at
    /// `u64::MAX` ticks: the delay of a retry that backs off.
    #[must_use]
    pub const fn doubled(self, failures: Count<Failures>) -> Self {
        match 1_u64.checked_shl(failures.get()) {
            Some(factor) => self.times(factor),
            None => Self::new(u64::MAX),
        }
    }
}

impl TimeSpan<Ms> {
    /// The span as the `i32` a browser timer takes, saturating at
    /// `i32::MAX` milliseconds.
    #[must_use]
    pub fn to_timer_i32(self) -> i32 {
        i32::try_from(self.ticks).unwrap_or(i32::MAX)
    }

    /// The span as a distance between two instants of the page clock.
    #[must_use]
    pub const fn to_fractional(self) -> TimeSpan<FracMs> {
        TimeSpan::new(self.ticks as f64)
    }
}

impl TimeSpan<Ns> {
    /// The span of a measured mean, a float of nanoseconds. A negative or NaN
    /// mean reads as zero, and a mean past `u64::MAX` saturates there.
    #[must_use]
    pub fn from_mean(mean: f64) -> Self {
        Self::new(mean.max(0.0) as u64)
    }
}

impl TimeSpan<FracMs> {
    /// The smaller of the two spans.
    #[must_use]
    pub fn min(self, other: Self) -> Self {
        Self::new(self.ticks.min(other.ticks))
    }

    /// The larger of the two spans.
    #[must_use]
    pub fn max(self, other: Self) -> Self {
        Self::new(self.ticks.max(other.ticks))
    }

    /// The span without its sign.
    #[must_use]
    pub fn abs(self) -> Self {
        Self::new(self.ticks.abs())
    }

    /// The span clamped into `lo ..= hi`, never a panic: a NaN reads as
    /// `lo`, and `hi` wins when the bounds cross.
    #[must_use]
    pub fn clamp(self, lo: Self, hi: Self) -> Self {
        Self::new(self.ticks.max(lo.ticks).min(hi.ticks))
    }

    /// The span rounded to a whole number of milliseconds.
    #[must_use]
    pub fn round(self) -> Self {
        Self::new(self.ticks.round())
    }

    /// The span rounded down to a whole number of milliseconds.
    #[must_use]
    pub fn floor(self) -> Self {
        Self::new(self.ticks.floor())
    }

    /// Whether the span is a finite number.
    #[must_use]
    pub fn is_finite(self) -> bool {
        self.ticks.is_finite()
    }

    /// The total order of IEEE 754 over the two spans, so a list of spans
    /// sorts whatever it holds.
    #[must_use]
    pub fn total_cmp(&self, other: &Self) -> Ordering {
        self.ticks.total_cmp(&other.ticks)
    }
}

impl Add for TimeSpan<FracMs> {
    type Output = Self;
    fn add(self, other: Self) -> Self {
        Self::new(self.ticks + other.ticks)
    }
}

impl Sub for TimeSpan<FracMs> {
    type Output = Self;
    fn sub(self, other: Self) -> Self {
        Self::new(self.ticks - other.ticks)
    }
}

impl AddAssign for TimeSpan<FracMs> {
    fn add_assign(&mut self, other: Self) {
        self.ticks += other.ticks;
    }
}

impl SubAssign for TimeSpan<FracMs> {
    fn sub_assign(&mut self, other: Self) {
        self.ticks -= other.ticks;
    }
}

impl Neg for TimeSpan<FracMs> {
    type Output = Self;
    fn neg(self) -> Self {
        Self::new(-self.ticks)
    }
}

/// The span scaled by a bare number.
impl Mul<f64> for TimeSpan<FracMs> {
    type Output = Self;
    fn mul(self, factor: f64) -> Self {
        Self::new(self.ticks * factor)
    }
}

/// The span divided by a bare number.
impl Div<f64> for TimeSpan<FracMs> {
    type Output = Self;
    fn div(self, divisor: f64) -> Self {
        Self::new(self.ticks / divisor)
    }
}

/// The ratio of two spans, a bare number.
impl Div for TimeSpan<FracMs> {
    type Output = f64;
    fn div(self, other: Self) -> f64 {
        self.ticks / other.ticks
    }
}

impl std::iter::Sum for TimeSpan<FracMs> {
    fn sum<I: Iterator<Item = Self>>(iter: I) -> Self {
        Self::new(iter.map(|span| span.ticks).sum())
    }
}

// Every impl below is written by hand. A derive would bound the marker,
// which has no value to clone, compare or hash.

impl<U: TimeUnit> Clone for TimeSpan<U> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<U: TimeUnit> Copy for TimeSpan<U> {}

impl<U: TimeUnit> Default for TimeSpan<U> {
    fn default() -> Self {
        Self::ZERO
    }
}

impl<U: TimeUnit> PartialEq for TimeSpan<U> {
    fn eq(&self, other: &Self) -> bool {
        self.ticks == other.ticks
    }
}

impl<U: TimeUnit> Eq for TimeSpan<U> where U::Repr: Eq {}

impl<U: TimeUnit> PartialOrd for TimeSpan<U> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        self.ticks.partial_cmp(&other.ticks)
    }
}

impl<U: TimeUnit> Ord for TimeSpan<U>
where
    U::Repr: Ord,
{
    fn cmp(&self, other: &Self) -> Ordering {
        self.ticks.cmp(&other.ticks)
    }
}

impl<U: TimeUnit> Hash for TimeSpan<U>
where
    U::Repr: Hash,
{
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.ticks.hash(state);
    }
}

/// `NAME(n)`, as the hand-written span printed itself.
impl<U: TimeUnit> fmt::Debug for TimeSpan<U> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple(U::NAME).field(&self.ticks).finish()
    }
}

/// The bare number, as `#[serde(transparent)]` writes it.
impl<U: TimeUnit> Serialize for TimeSpan<U> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.ticks.serialize(serializer)
    }
}

/// The bare number, as `#[serde(transparent)]` reads it.
impl<'de, U: TimeUnit> Deserialize<'de> for TimeSpan<U> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        U::Repr::deserialize(deserializer).map(Self::new)
    }
}

/// An instant on the time base `C`: the span since the base's epoch.
///
/// Two instants of one base give the span between them. An instant moves by
/// a span of its base's unit, and by no other. `new` wraps the number, and
/// `get` unwraps it for a foreign API.
pub struct Instant<C: TimeBase> {
    since_epoch: TimeSpan<C::Unit>,
    base: PhantomData<fn() -> C>,
}

impl<C: TimeBase> Instant<C> {
    /// The epoch of the base.
    pub const EPOCH: Self = Self::after_epoch(TimeSpan::ZERO);

    /// The instant `ticks` units after the epoch. The constructor checks
    /// nothing.
    #[must_use]
    pub const fn new(ticks: <C::Unit as TimeUnit>::Repr) -> Self {
        Self::after_epoch(TimeSpan::new(ticks))
    }

    /// The number of units since the epoch, for a foreign API.
    #[must_use]
    pub const fn get(self) -> <C::Unit as TimeUnit>::Repr {
        self.since_epoch.get()
    }

    /// The instant `span` after the epoch.
    #[must_use]
    pub const fn after_epoch(span: TimeSpan<C::Unit>) -> Self {
        Self {
            since_epoch: span,
            base: PhantomData,
        }
    }

    /// The span since the epoch: for the page clock, the phase a periodic
    /// animation reads.
    #[must_use]
    pub const fn since_epoch(self) -> TimeSpan<C::Unit> {
        self.since_epoch
    }
}

impl<C: TimeBase> Instant<C>
where
    C::Unit: TimeUnit<Repr = u64>,
{
    /// The instant `span` after this one, saturating at `u64::MAX`.
    #[must_use]
    pub const fn plus(self, span: TimeSpan<C::Unit>) -> Self {
        Self::new(self.get().saturating_add(span.get()))
    }

    /// The span from `earlier` to this instant, zero when `earlier` is later.
    #[must_use]
    pub const fn since(self, earlier: Self) -> TimeSpan<C::Unit> {
        TimeSpan::new(self.get().saturating_sub(earlier.get()))
    }
}

impl Instant<UnixMs> {
    /// The whole second the instant falls in.
    #[must_use]
    pub const fn to_unix_seconds(self) -> Instant<UnixSec> {
        Instant::after_epoch(self.since_epoch.to_floor::<Sec>())
    }
}

impl Instant<Relayed> {
    /// The span from this instant to `now`, read on the reader's clock, zero
    /// when the reader's clock runs behind the relay's.
    ///
    /// The one place a relay instant meets the reader's clock. Two clocks
    /// drift apart, so the span is good for a display, such as "2 h ago",
    /// and for no rule.
    #[must_use]
    pub const fn age_at(self, now: Instant<UnixMs>) -> TimeSpan<Ms> {
        TimeSpan::new(now.get().saturating_sub(self.get()))
    }
}

impl Instant<PageLoad> {
    /// The instant `span` after this one.
    #[must_use]
    pub fn after(self, span: TimeSpan<FracMs>) -> Self {
        Self::after_epoch(self.since_epoch + span)
    }

    /// The instant `span` before this one.
    #[must_use]
    pub fn before(self, span: TimeSpan<FracMs>) -> Self {
        Self::after_epoch(self.since_epoch - span)
    }

    /// The later of the two instants.
    #[must_use]
    pub fn max(self, other: Self) -> Self {
        Self::after_epoch(self.since_epoch.max(other.since_epoch))
    }
}

impl Instant<SheetDay> {
    /// The Unix epoch, 1970-01-01, as a spreadsheet date.
    pub const UNIX_EPOCH: Self = Self::new(25_569);
}

/// The days from `earlier` to this date, negative when `earlier` is later,
/// saturating at either end of `i64`.
impl Sub for Instant<SheetDay> {
    type Output = TimeSpan<Day>;

    fn sub(self, earlier: Self) -> TimeSpan<Day> {
        TimeSpan::new(self.get().saturating_sub(earlier.get()))
    }
}

impl Instant<SheetDateTime> {
    /// The instant `unix_days` days after the Unix epoch.
    #[must_use]
    pub fn from_unix_days(unix_days: f64) -> Self {
        Self::new(unix_days + Instant::<SheetDay>::UNIX_EPOCH.get() as f64)
    }

    /// The day it falls on, its time of day dropped. A serial past either
    /// end of `i64` saturates there, and a NaN reads as the epoch.
    #[must_use]
    pub fn day(self) -> Instant<SheetDay> {
        Instant::new(self.get().floor() as i64)
    }

    /// The time of day, as a fraction of a day from 0 up to 1.
    #[must_use]
    pub fn time_of_day(self) -> f64 {
        self.get() - self.get().floor()
    }
}

/// The span from `earlier` to this instant, negative when `earlier` is
/// later.
impl Sub for Instant<PageLoad> {
    type Output = TimeSpan<FracMs>;

    fn sub(self, earlier: Self) -> TimeSpan<FracMs> {
        self.since_epoch - earlier.since_epoch
    }
}

impl<C: TimeBase> Clone for Instant<C> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<C: TimeBase> Copy for Instant<C> {}

impl<C: TimeBase> Default for Instant<C> {
    fn default() -> Self {
        Self::EPOCH
    }
}

impl<C: TimeBase> PartialEq for Instant<C> {
    fn eq(&self, other: &Self) -> bool {
        self.since_epoch == other.since_epoch
    }
}

impl<C: TimeBase> Eq for Instant<C> where TimeSpan<C::Unit>: Eq {}

impl<C: TimeBase> PartialOrd for Instant<C> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        self.since_epoch.partial_cmp(&other.since_epoch)
    }
}

impl<C: TimeBase> Ord for Instant<C>
where
    TimeSpan<C::Unit>: Ord,
{
    fn cmp(&self, other: &Self) -> Ordering {
        self.since_epoch.cmp(&other.since_epoch)
    }
}

impl<C: TimeBase> Hash for Instant<C>
where
    TimeSpan<C::Unit>: Hash,
{
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.since_epoch.hash(state);
    }
}

/// `NAME(n)`, as the hand-written instant printed itself.
impl<C: TimeBase> fmt::Debug for Instant<C> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple(C::NAME).field(&self.get()).finish()
    }
}

/// The bare number of units since the epoch.
impl<C: TimeBase> fmt::Display for Instant<C> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.get().fmt(f)
    }
}

/// The bare number, as `#[serde(transparent)]` writes it.
impl<C: TimeBase> Serialize for Instant<C> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.since_epoch.serialize(serializer)
    }
}

/// The bare number, as `#[serde(transparent)]` reads it.
impl<'de, C: TimeBase> Deserialize<'de> for Instant<C> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        TimeSpan::deserialize(deserializer).map(Self::after_epoch)
    }
}

/// A whole count that a CRDT or a Postgres `BIGINT` stores as a signed
/// 64-bit integer, since neither has an unsigned one.
///
/// An implementor names its count once; the two conversions are shared.
pub trait StoredAsI64: Copy {
    /// The unsigned count the value holds.
    fn stored_count(self) -> u64;

    /// The value of the unsigned count `count`.
    fn from_stored_count(count: u64) -> Self;

    /// The value as the store holds it, saturating at `i64::MAX`.
    fn to_i64_saturating(self) -> i64 {
        i64::try_from(self.stored_count()).unwrap_or(i64::MAX)
    }

    /// A value the store held. A negative value, which no writer stores,
    /// reads as zero: the epoch of an instant, before every write for a
    /// stamp.
    fn from_i64(stored: i64) -> Self {
        Self::from_stored_count(u64::try_from(stored).unwrap_or(0))
    }
}

impl<C: TimeBase> StoredAsI64 for Instant<C>
where
    C::Unit: TimeUnit<Repr = u64>,
{
    fn stored_count(self) -> u64 {
        self.get()
    }

    fn from_stored_count(count: u64) -> Self {
        Self::new(count)
    }
}

/// A logical instant: the Lamport time a CRDT operation carries.
///
/// It counts operations, never milliseconds, so it has no time base and
/// moves by no span. Unlike an [`Instant<Authored>`], a rule may order on
/// it: a write made after another was seen carries a greater stamp. Two
/// concurrent writes may share one, so such a rule breaks the tie on
/// something else. So it is no [`crate::Seq`]: many peers issue stamps, and
/// two may hand out one value.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct LamportStamp(u64);

impl fmt::Display for LamportStamp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl LamportStamp {
    /// Wraps a raw value. The constructor checks nothing.
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// Returns the raw value.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

impl StoredAsI64 for LamportStamp {
    fn stored_count(self) -> u64 {
        self.0
    }

    fn from_stored_count(count: u64) -> Self {
        Self(count)
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

/// A [`StoredAsI64`] value as a signed 64-bit integer, for a value a
/// document already stores that way:
/// `#[serde(with = "ambre_core::span::stored_as_i64")]`. A varint codec
/// spells a signed and an unsigned integer apart, so the field keeps the
/// bytes it was frozen with.
pub mod stored_as_i64 {
    use super::StoredAsI64;
    use serde::{Deserialize, Deserializer, Serializer};

    /// Writes the value as the `i64` the document holds.
    ///
    /// # Errors
    /// The serializer's own.
    pub fn serialize<T: StoredAsI64, S: Serializer>(
        value: &T,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        serializer.serialize_i64(value.to_i64_saturating())
    }

    /// Reads the `i64` the document holds.
    ///
    /// # Errors
    /// The deserializer's own.
    pub fn deserialize<'de, T: StoredAsI64, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<T, D::Error> {
        i64::deserialize(deserializer).map(T::from_i64)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Authored, Day, Failures, FracMs, Instant, LamportStamp, Ms, Ns, PageLoad, Relayed, Sec,
        SheetDateTime, SheetDay, StoredAsI64, TimeSpan, UnixMs, UnixSec,
    };
    use crate::Count;
    use std::time::Duration;

    #[test]
    fn a_lamport_stamp_reads_back_what_a_crdt_stored() {
        assert_eq!(LamportStamp::from_i64(7).to_i64_saturating(), 7);
        assert_eq!(LamportStamp::from_i64(-1), LamportStamp::new(0));
        assert_eq!(LamportStamp::new(u64::MAX).to_i64_saturating(), i64::MAX);
        assert!(LamportStamp::new(3) > LamportStamp::new(2));
    }

    #[test]
    fn a_span_serializes_as_the_bare_integer_of_its_unit() {
        assert_eq!(
            serde_json::to_string(&TimeSpan::<Ms>::new(1500)).unwrap(),
            "1500"
        );
        assert_eq!(
            serde_json::to_string(&TimeSpan::<Sec>::new(900)).unwrap(),
            "900"
        );
        assert_eq!(
            serde_json::from_str::<TimeSpan<Ms>>("1500").unwrap(),
            TimeSpan::new(1500)
        );
    }

    #[test]
    fn a_span_and_an_instant_debug_print_their_former_names() {
        assert_eq!(format!("{:?}", TimeSpan::<Ms>::new(3)), "Millis(3)");
        assert_eq!(format!("{:?}", Instant::<Authored>::new(3)), "Timestamp(3)");
        assert_eq!(
            format!("{:?}", Instant::<PageLoad>::new(1.5)),
            "PageTime(1.5)"
        );
    }

    #[test]
    fn a_millisecond_span_scales_and_backs_off_without_overflow() {
        let ms = TimeSpan::<Ms>::new;
        assert_eq!(ms(2_000).times(3), ms(6_000));
        assert_eq!(ms(2_000).doubled(Count::<Failures>::new(2)), ms(8_000));
        assert_eq!(ms(2).doubled(Count::<Failures>::new(64)), ms(u64::MAX));
        assert_eq!(ms(u64::MAX).times(2), ms(u64::MAX));
        assert_eq!(ms(1_000).to_timer_i32(), 1_000);
        assert_eq!(ms(u64::MAX).to_timer_i32(), i32::MAX);
        let at = Instant::<Authored>::new;
        assert_eq!(at(10).plus(ms(5)), at(15));
        assert_eq!(at(10).since(at(4)), ms(6));
        assert_eq!(at(4).since(at(10)), ms(0));
    }

    #[test]
    fn a_span_converts_between_units_rounding_down_and_saturating() {
        assert_eq!(
            TimeSpan::<Ms>::new(1_999).to_floor::<Sec>(),
            TimeSpan::new(1)
        );
        assert_eq!(
            TimeSpan::<Sec>::new(3).to_floor::<Ms>(),
            TimeSpan::new(3_000)
        );
        assert_eq!(
            TimeSpan::<Sec>::new(u64::MAX).to_floor::<Ns>(),
            TimeSpan::new(u64::MAX)
        );
        assert_eq!(
            TimeSpan::<Ms>::new(250).to_fractional(),
            TimeSpan::<FracMs>::new(250.0)
        );
    }

    #[test]
    fn a_measured_mean_reads_as_whole_nanoseconds() {
        assert_eq!(TimeSpan::<Ns>::from_mean(12.7), TimeSpan::new(12));
        assert_eq!(TimeSpan::<Ns>::from_mean(-3.0), TimeSpan::new(0));
        assert_eq!(TimeSpan::<Ns>::from_mean(f64::NAN), TimeSpan::new(0));
        assert_eq!(serde_json::to_string(&TimeSpan::<Ns>::new(5)).unwrap(), "5");
    }

    #[test]
    fn a_span_converts_to_the_same_duration() {
        assert_eq!(
            TimeSpan::<Ms>::new(1500).to_duration(),
            Duration::from_millis(1500)
        );
        assert_eq!(
            TimeSpan::<Sec>::new(2).to_duration(),
            Duration::from_secs(2)
        );
        assert_eq!(
            TimeSpan::<Ns>::new(1_000_000_007).to_duration(),
            Duration::from_nanos(1_000_000_007)
        );
        assert_eq!(
            TimeSpan::<Ms>::from_duration(Duration::from_secs(3)),
            TimeSpan::new(3000)
        );
        assert_eq!(
            TimeSpan::<Ms>::from_duration(Duration::MAX),
            TimeSpan::new(u64::MAX)
        );
        assert_eq!(
            TimeSpan::<FracMs>::from_duration(Duration::from_micros(1_500)),
            TimeSpan::new(1.5)
        );
    }

    #[test]
    fn two_page_instants_are_their_distance_in_milliseconds_apart() {
        let start = Instant::<PageLoad>::new(100.0);
        let span = TimeSpan::<FracMs>::new(16.5);
        assert_eq!(start.after(span) - start, span);
        assert_eq!(start.before(span).after(span), start);
        assert_eq!(start.max(Instant::new(40.0)), start);
        assert_eq!(start.since_epoch(), TimeSpan::new(100.0));
    }

    #[test]
    fn a_timestamp_reads_back_what_a_crdt_stored() {
        let at = Instant::<Authored>::new;
        assert_eq!(Instant::from_i64(at(42).to_i64_saturating()), at(42));
        assert_eq!(Instant::<Authored>::from_i64(-1), at(0));
        assert_eq!(serde_json::to_string(&at(7)).unwrap(), "7");
    }

    #[test]
    fn a_timestamp_stored_as_i64_keeps_its_signed_spelling() {
        #[derive(serde::Serialize, serde::Deserialize, PartialEq, Debug)]
        struct Stored(#[serde(with = "super::stored_as_i64")] Instant<Authored>);
        let bytes = postcard::to_allocvec(&Stored(Instant::new(3))).unwrap();
        assert_eq!(bytes, postcard::to_allocvec(&3_i64).unwrap());
        assert_eq!(
            postcard::from_bytes::<Stored>(&bytes).unwrap(),
            Stored(Instant::new(3))
        );
    }

    #[test]
    fn a_millisecond_instant_falls_in_its_second() {
        let at = Instant::<UnixMs>::new(1_999).plus(TimeSpan::new(1));
        assert_eq!(at, Instant::new(2_000));
        assert_eq!(at.to_unix_seconds(), Instant::<UnixSec>::new(2));
    }

    #[test]
    fn an_instant_moves_by_a_span_and_never_goes_below_zero() {
        let at = Instant::<UnixSec>::new;
        let secs = TimeSpan::<Sec>::new;
        assert_eq!(at(100).plus(secs(20)), at(120));
        assert_eq!(at(100).since(at(40)), secs(60));
        assert_eq!(at(40).since(at(100)), secs(0));
        assert_eq!(at(u64::MAX).to_i64_saturating(), i64::MAX);
    }

    /// A spreadsheet date measures signed days back to another, where a
    /// relay instant stops at zero.
    #[test]
    fn a_sheet_day_measures_signed_days_and_saturates_at_either_end() {
        let day = Instant::<SheetDay>::new;
        assert_eq!(day(46_294) - day(46_265), TimeSpan::<Day>::new(29));
        assert_eq!(day(46_265) - day(46_294), TimeSpan::<Day>::new(-29));
        assert_eq!(day(i64::MIN) - day(1), TimeSpan::<Day>::new(i64::MIN));
        assert_eq!(day(i64::MAX) - day(-1), TimeSpan::<Day>::new(i64::MAX));
        assert!(day(-1) < Instant::EPOCH);
        assert_eq!(Instant::<SheetDay>::UNIX_EPOCH, day(25_569));
    }

    #[test]
    fn a_sheet_date_time_falls_on_its_day_and_keeps_its_time() {
        let at = Instant::<SheetDateTime>::new(46_294.75);
        assert_eq!(at.day(), Instant::<SheetDay>::new(46_294));
        assert!((at.time_of_day() - 0.75).abs() < f64::EPSILON);
        let before = Instant::<SheetDateTime>::new(-0.25);
        assert_eq!(before.day(), Instant::<SheetDay>::new(-1));
        assert!((before.time_of_day() - 0.75).abs() < f64::EPSILON);
        assert_eq!(
            Instant::<SheetDateTime>::from_unix_days(1.5),
            Instant::new(25_570.5)
        );
        assert_eq!(
            Instant::<SheetDateTime>::new(f64::NAN).day(),
            Instant::EPOCH
        );
    }

    /// The three spreadsheet types write the bare `i64` or `f64` the
    /// hand-written newtypes held, and print their former names.
    #[test]
    fn a_sheet_date_writes_its_bare_number_and_prints_its_former_name() {
        let day = Instant::<SheetDay>::new(-7);
        let count = TimeSpan::<Day>::new(-29);
        let at = Instant::<SheetDateTime>::new(46_294.75);
        assert_eq!(
            postcard::to_allocvec(&day).unwrap(),
            postcard::to_allocvec(&-7_i64).unwrap()
        );
        assert_eq!(
            postcard::to_allocvec(&count).unwrap(),
            postcard::to_allocvec(&-29_i64).unwrap()
        );
        assert_eq!(
            postcard::to_allocvec(&at).unwrap(),
            postcard::to_allocvec(&46_294.75_f64).unwrap()
        );
        assert_eq!(serde_json::to_string(&day).unwrap(), "-7");
        assert_eq!(serde_json::to_string(&at).unwrap(), "46294.75");
        let bytes = postcard::to_allocvec(&count).unwrap();
        assert_eq!(
            postcard::from_bytes::<TimeSpan<Day>>(&bytes).unwrap(),
            count
        );
        assert_eq!(
            serde_json::from_str::<Instant<SheetDateTime>>("46294.75").unwrap(),
            at
        );
        assert_eq!(format!("{day:?}"), "DaySerial(-7)");
        assert_eq!(format!("{count:?}"), "DayCount(-29)");
        assert_eq!(format!("{at:?}"), "DateSerial(46294.75)");
    }

    #[test]
    fn a_relay_instant_measures_the_span_since_another_and_never_goes_below_zero() {
        let at = Instant::<UnixMs>::new;
        let ms = TimeSpan::<Ms>::new;
        assert_eq!(at(1_000).since(at(250)), ms(750));
        assert_eq!(at(250).since(at(1_000)), ms(0));
        assert_eq!(at(7).since(at(7)), ms(0));
    }

    #[test]
    fn a_relayed_instant_ages_on_the_readers_clock_and_never_below_zero() {
        let stamped = Instant::<Relayed>::new(1_000);
        let ms = TimeSpan::<Ms>::new;
        assert_eq!(stamped.age_at(Instant::<UnixMs>::new(4_000)), ms(3_000));
        assert_eq!(stamped.age_at(Instant::<UnixMs>::new(400)), ms(0));
        assert_eq!(serde_json::to_string(&stamped).unwrap(), "1000");
        assert_eq!(format!("{stamped:?}"), "RelayMillis(1000)");
    }
}
