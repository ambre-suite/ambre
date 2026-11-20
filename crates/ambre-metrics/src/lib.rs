//! The metrics of the relay and of the SFU, in the Prometheus text format
//! (`SRV-257`, `CALL-101`).
//!
//! A label value is the spelling of a variant of a closed `enum`, a
//! [`Label`]. So no label carries a runtime value: no identifier, no address
//! (`ZK-029`, `ZK-031`). Each metric holds one cell per variant from its start.
//! So the count of series is fixed when the process starts, and a series
//! reads zero before its first event.
//!
//! The crate is a few atomics and a text writer. The official client and the
//! `metrics` facade each accept a label value as a runtime text, which is the
//! leak this crate makes unwritable.

use std::fmt::Write as _;
use std::marker::PhantomData;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use ambre_core::{
    Count, CountKind, Credential, DebugNames, KeyKind, LabelValues, MetricNames, ScrapeToken,
    StaticKey,
};

#[cfg(test)]
mod tests;

/// How many events a counter adds, or how many things a gauge holds.
pub enum Events {}

impl CountKind for Events {
    type Repr = u64;
    const NAME: StaticKey<DebugNames> = StaticKey::new("EventCount");
}

/// The closed set of values one label of a metric takes.
///
/// The implementor is a fieldless `enum`. [`Label::ALL`] lists each variant
/// once, and the metric holds one series per entry, so the count of series
/// never grows at run time.
pub trait Label: Copy + PartialEq + 'static {
    /// The name of the label, or `None` for a metric with one series and no
    /// label.
    const KEY: Option<StaticKey<MetricNames>>;
    /// Every value, each once, in the order the exposition writes them.
    const ALL: &'static [Self];
    /// The spelling of this value in the exposition.
    fn value(self) -> StaticKey<LabelValues>;
}

/// The one value of a metric that carries no label.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Whole {
    /// The one series.
    All,
}

impl Label for Whole {
    const KEY: Option<StaticKey<MetricNames>> = None;
    const ALL: &'static [Self] = &[Self::All];
    fn value(self) -> StaticKey<LabelValues> {
        StaticKey::new("")
    }
}

/// The text of a scrape, built metric by metric.
#[derive(Default)]
pub struct Exposition {
    text: String,
}

impl Exposition {
    /// An empty exposition.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The whole text, as the scrape route answers it.
    #[must_use]
    pub fn into_text(self) -> String {
        self.text
    }

    /// The two comment lines that open a metric.
    fn header(&mut self, name: StaticKey<MetricNames>, help: StaticKey<MetricNames>, kind: Kind) {
        let name = name.as_str();
        let _ = writeln!(self.text, "# HELP {name} {}", escaped(help, Quote::Off));
        let _ = writeln!(self.text, "# TYPE {name} {}", kind.word());
    }

    /// One sample: the metric name, an optional suffix, the labels, the value.
    fn sample<L: Label>(
        &mut self,
        name: StaticKey<MetricNames>,
        suffix: Suffix,
        label: L,
        value: &dyn std::fmt::Display,
    ) {
        let _ = write!(self.text, "{}{}", name.as_str(), suffix.text());
        if let Some(key) = L::KEY {
            let _ = write!(
                self.text,
                "{{{}=\"{}\"}}",
                key.as_str(),
                escaped(label.value(), Quote::On)
            );
        }
        let _ = writeln!(self.text, " {value}");
    }
}

/// The kind a `# TYPE` line names.
#[derive(Clone, Copy)]
enum Kind {
    Counter,
    Gauge,
    Histogram,
}

impl Kind {
    /// The word of the text format.
    const fn word(self) -> &'static str {
        match self {
            Self::Counter => "counter",
            Self::Gauge => "gauge",
            Self::Histogram => "histogram",
        }
    }
}

/// The suffix a sample of a histogram adds to the metric name.
#[derive(Clone, Copy)]
enum Suffix {
    None,
    Sum,
    Count,
}

impl Suffix {
    /// The text of the suffix.
    const fn text(self) -> &'static str {
        match self {
            Self::None => "",
            Self::Sum => "_sum",
            Self::Count => "_count",
        }
    }
}

/// Whether a text sits between double quotes, where a quote is escaped too.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Quote {
    Off,
    On,
}

/// The spelling of `key` with the escapes of the text format: a backslash
/// and a line feed, and a double quote inside a label value.
fn escaped<K: KeyKind>(key: StaticKey<K>, quote: Quote) -> String {
    let text = key.as_str();
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '"' if quote == Quote::On => out.push_str("\\\""),
            c => out.push(c),
        }
    }
    out
}

/// One cell per value of `L`, each at zero.
fn cells<L: Label>() -> Vec<AtomicU64> {
    L::ALL.iter().map(|_| AtomicU64::new(0)).collect()
}

/// The cell of `label`. Every value of `L` has one, so `None` never comes
/// from a value of [`Label::ALL`].
fn cell<L: Label>(cells: &[AtomicU64], label: L) -> Option<&AtomicU64> {
    L::ALL
        .iter()
        .position(|l| *l == label)
        .and_then(|at| cells.get(at))
}

/// A count that only goes up, one series per value of `L`.
pub struct Counter<L: Label> {
    name: StaticKey<MetricNames>,
    help: StaticKey<MetricNames>,
    cells: Vec<AtomicU64>,
    label: PhantomData<fn() -> L>,
}

impl<L: Label> Counter<L> {
    /// A counter at zero in each series.
    #[must_use]
    pub fn new(name: StaticKey<MetricNames>, help: StaticKey<MetricNames>) -> Self {
        Self {
            name,
            help,
            cells: cells::<L>(),
            label: PhantomData,
        }
    }

    /// Counts one event of `label`.
    pub fn inc(&self, label: L) {
        self.add(label, Count::new(1));
    }

    /// Counts `n` events of `label`.
    pub fn add(&self, label: L, n: Count<Events>) {
        if let Some(cell) = cell(&self.cells, label) {
            cell.fetch_add(n.get(), Ordering::Relaxed);
        }
    }

    /// The events of `label` counted so far.
    #[must_use]
    pub fn read(&self, label: L) -> Count<Events> {
        Count::new(cell(&self.cells, label).map_or(0, |c| c.load(Ordering::Relaxed)))
    }

    /// Writes the metric into `out`.
    pub fn write(&self, out: &mut Exposition) {
        out.header(self.name, self.help, Kind::Counter);
        for (label, cell) in L::ALL.iter().zip(&self.cells) {
            out.sample(
                self.name,
                Suffix::None,
                *label,
                &cell.load(Ordering::Relaxed),
            );
        }
    }
}

/// A level that goes up and down, one series per value of `L`.
pub struct Gauge<L: Label> {
    name: StaticKey<MetricNames>,
    help: StaticKey<MetricNames>,
    cells: Vec<AtomicU64>,
    label: PhantomData<fn() -> L>,
}

impl<L: Label> Gauge<L> {
    /// A gauge at zero in each series.
    #[must_use]
    pub fn new(name: StaticKey<MetricNames>, help: StaticKey<MetricNames>) -> Self {
        Self {
            name,
            help,
            cells: cells::<L>(),
            label: PhantomData,
        }
    }

    /// Sets the level of `label`.
    pub fn set(&self, label: L, level: Count<Events>) {
        if let Some(cell) = cell(&self.cells, label) {
            cell.store(level.get(), Ordering::Relaxed);
        }
    }

    /// Raises the level of `label` by one.
    pub fn inc(&self, label: L) {
        if let Some(cell) = cell(&self.cells, label) {
            let _ = cell.try_update(Ordering::Relaxed, Ordering::Relaxed, |v| {
                Some(v.saturating_add(1))
            });
        }
    }

    /// Lowers the level of `label` by one, held at zero.
    pub fn dec(&self, label: L) {
        if let Some(cell) = cell(&self.cells, label) {
            let _ = cell.try_update(Ordering::Relaxed, Ordering::Relaxed, |v| {
                Some(v.saturating_sub(1))
            });
        }
    }

    /// The level of `label`.
    #[must_use]
    pub fn read(&self, label: L) -> Count<Events> {
        Count::new(cell(&self.cells, label).map_or(0, |c| c.load(Ordering::Relaxed)))
    }

    /// Writes the metric into `out`.
    pub fn write(&self, out: &mut Exposition) {
        out.header(self.name, self.help, Kind::Gauge);
        for (label, cell) in L::ALL.iter().zip(&self.cells) {
            out.sample(
                self.name,
                Suffix::None,
                *label,
                &cell.load(Ordering::Relaxed),
            );
        }
    }
}

/// The upper bound of a bucket of a histogram, inclusive.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Bound {
    /// A bucket that holds every duration up to this one.
    Upto(Duration),
    /// The last bucket, which holds every duration.
    Infinite,
}

/// The spread of durations, in buckets fixed at the start.
///
/// The buckets are cumulative, as the text format reads them. A duration
/// above the last finite bound counts in the infinite bucket alone.
pub struct Histogram {
    name: StaticKey<MetricNames>,
    help: StaticKey<MetricNames>,
    bounds: &'static [(Duration, StaticKey<LabelValues>)],
    /// One cell per bound, then the infinite bucket.
    cells: Vec<AtomicU64>,
    /// The sum of every duration observed, in nanoseconds.
    sum: AtomicU64,
}

impl Histogram {
    /// A histogram over `bounds`, each a duration and its spelling in
    /// seconds, in increasing order.
    #[must_use]
    pub fn new(
        name: StaticKey<MetricNames>,
        help: StaticKey<MetricNames>,
        bounds: &'static [(Duration, StaticKey<LabelValues>)],
    ) -> Self {
        Self {
            name,
            help,
            bounds,
            cells: bounds
                .iter()
                .map(|_| AtomicU64::new(0))
                .chain(std::iter::once(AtomicU64::new(0)))
                .collect(),
            sum: AtomicU64::new(0),
        }
    }

    /// Counts one duration.
    pub fn observe(&self, took: Duration) {
        let at = self
            .bounds
            .iter()
            .position(|(bound, _)| took <= *bound)
            .unwrap_or(self.bounds.len());
        if let Some(cell) = self.cells.get(at) {
            cell.fetch_add(1, Ordering::Relaxed);
        }
        let nanos = u64::try_from(took.as_nanos()).unwrap_or(u64::MAX);
        let _ = self
            .sum
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |v| {
                Some(v.saturating_add(nanos))
            });
    }

    /// How many durations fell at or under `bound`.
    #[must_use]
    pub fn read(&self, bound: Bound) -> Count<Events> {
        let upto = match bound {
            Bound::Upto(d) => self.bounds.iter().take_while(|(b, _)| *b <= d).count(),
            Bound::Infinite => self.cells.len(),
        };
        Count::new(
            self.cells
                .iter()
                .take(upto)
                .map(|c| c.load(Ordering::Relaxed))
                .fold(0, u64::saturating_add),
        )
    }

    /// Writes the metric into `out`: the cumulative buckets, the sum in
    /// seconds and the count.
    pub fn write(&self, out: &mut Exposition) {
        out.header(self.name, self.help, Kind::Histogram);
        let bucket = StaticKey::<MetricNames>::new("le");
        let mut total = 0_u64;
        // The text of each bound is a `const` table entry, never a runtime value.
        let labels = self
            .bounds
            .iter()
            .map(|(_, text)| *text)
            .chain(std::iter::once(StaticKey::<LabelValues>::new("+Inf")));
        for (label, cell) in labels.zip(&self.cells) {
            total = total.saturating_add(cell.load(Ordering::Relaxed));
            let _ = writeln!(
                out.text,
                "{}_bucket{{{}=\"{}\"}} {total}",
                self.name.as_str(),
                bucket.as_str(),
                label.as_str()
            );
        }
        let nanos = self.sum.load(Ordering::Relaxed);
        let seconds = Duration::from_nanos(nanos).as_secs_f64();
        out.sample(self.name, Suffix::Sum, Whole::All, &seconds);
        out.sample(self.name, Suffix::Count, Whole::All, &total);
    }
}

/// Who may read a scrape of a metrics listener (`SRV-307`, `CALL-141`).
///
/// A listener on the loopback answers any process of its host. A listener on
/// any other address answers only a scrape that presents the token the
/// operator set. A token, once set, binds the loopback too.
#[derive(Clone, Debug)]
pub enum ScrapeAccess {
    /// The listener sits on the loopback and the operator set no token.
    Loopback,
    /// A scrape presents this token as a bearer credential.
    Token(Credential<ScrapeToken>),
}

/// Why a metrics listener refuses to start.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ScrapeRefusal {
    /// The address leaves the loopback and the operator set no token.
    TokenMissing,
}

impl std::fmt::Display for ScrapeRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TokenMissing => f.write_str(
                "a metrics address off the loopback needs a scrape token: set one, or bind the loopback",
            ),
        }
    }
}

impl std::error::Error for ScrapeRefusal {}

/// What a scrape receives.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Scrape {
    /// The listener answers the metrics.
    Admitted,
    /// The listener answers `401 Unauthorized` and no metric.
    Refused,
}

impl ScrapeAccess {
    /// The access of a listener bound to `address`, under the `token` the
    /// operator set, if any.
    ///
    /// # Errors
    ///
    /// [`ScrapeRefusal::TokenMissing`] when `address` leaves the loopback and
    /// `token` is `None`. The unspecified address leaves it.
    pub fn for_listener(
        address: SocketAddr,
        token: Option<Credential<ScrapeToken>>,
    ) -> Result<Self, ScrapeRefusal> {
        match token {
            Some(token) => Ok(Self::Token(token)),
            None if address.ip().is_loopback() => Ok(Self::Loopback),
            None => Err(ScrapeRefusal::TokenMissing),
        }
    }

    /// What a scrape that presents the bearer credential `presented`
    /// receives. The comparison runs in constant time over the token.
    #[must_use]
    pub fn admits(&self, presented: Option<&Credential<ScrapeToken>>) -> Scrape {
        match (self, presented) {
            (Self::Loopback, _) => Scrape::Admitted,
            (Self::Token(token), Some(presented)) if token.matches(presented) => Scrape::Admitted,
            (Self::Token(_), _) => Scrape::Refused,
        }
    }
}
