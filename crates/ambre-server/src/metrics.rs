//! The metrics of the relay, in the Prometheus text format, on a listener of
//! their own (`SRV-257`).
//!
//! The public listener serves no metric: an operator scrapes the address of
//! `METRICS_ADDR`, the loopback by default. An address off the loopback needs
//! the token of `METRICS_TOKEN`, which each scrape presents as a bearer
//! credential (`SRV-307`). Each label is a closed `enum` of
//! this module. So no series names a document, a user, a call, a connection
//! or an address (`ZK-029`, `ZK-031`).

use std::net::SocketAddr;
use std::sync::LazyLock;
use std::time::{Duration, Instant};

use ambre_core::{Credential, LabelValues, MetricNames, ScrapeToken, StaticKey};
use ambre_metrics::{Counter, Exposition, Gauge, Histogram, Label, Scrape, ScrapeAccess};
use axum::Router;
use axum::extract::{Request, State};
use axum::http::{StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::routing::get;

#[cfg(test)]
#[path = "metrics_tests.rs"]
mod tests;

/// The address of the metrics listener when `METRICS_ADDR` is unset: the
/// loopback, so a deployment that forgets the variable exposes nothing.
const FALLBACK: SocketAddr =
    SocketAddr::new(std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST), 9464);

/// The route the scrape reads.
pub const ROUTE: &str = "/metrics";

/// The media type of the Prometheus text format, version 0.0.4.
const TEXT_FORMAT: &str = "text/plain; version=0.0.4; charset=utf-8";

/// The kind of an open socket.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Socket {
    /// The socket of the document plane (`/api/ws`).
    Document,
    /// The signalling socket of a call (`/api/calls/{call}/socket`).
    Call,
}

impl Label for Socket {
    const KEY: Option<StaticKey<MetricNames>> = Some(StaticKey::new("socket"));
    const ALL: &'static [Self] = &[Self::Document, Self::Call];
    fn value(self) -> StaticKey<LabelValues> {
        StaticKey::new(match self {
            Self::Document => "document",
            Self::Call => "call",
        })
    }
}

/// What became of a delta a socket sent.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Delta {
    /// The delta took a sequence number and reached its subscribers.
    Sequenced,
    /// The relay refused it: a role, a key, a signature or a clock.
    Refused,
}

impl Label for Delta {
    const KEY: Option<StaticKey<MetricNames>> = Some(StaticKey::new("outcome"));
    const ALL: &'static [Self] = &[Self::Sequenced, Self::Refused];
    fn value(self) -> StaticKey<LabelValues> {
        StaticKey::new(match self {
            Self::Sequenced => "sequenced",
            Self::Refused => "refused",
        })
    }
}

/// A failure of the relay that loses no delta but needs an operator: each
/// one is also an error log line.
///
/// A new failure is a new variant, and its series exists from the start.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Failure {
    /// The index row of a sequenced delta was not written (`SRV-033`).
    IndexWrite,
    /// A sequenced delta did not reach its pending list (`SRV-034`).
    PendingAppend,
    /// A pending list took a delta, but the dirty set missed its mark: the
    /// flush waits for the reconciling sweep (`SRV-262`).
    DirtyMark,
    /// A delta the index or the pending list missed did not reach the
    /// dead-letter list either.
    DeadLetter,
    /// The broadcast of a sequenced delta failed: its subscribers replay it.
    Broadcast,
}

impl Label for Failure {
    const KEY: Option<StaticKey<MetricNames>> = Some(StaticKey::new("failure"));
    const ALL: &'static [Self] = &[
        Self::IndexWrite,
        Self::PendingAppend,
        Self::DirtyMark,
        Self::DeadLetter,
        Self::Broadcast,
    ];
    fn value(self) -> StaticKey<LabelValues> {
        StaticKey::new(match self {
            Self::IndexWrite => "index_write",
            Self::PendingAppend => "pending_append",
            Self::DirtyMark => "dirty_mark",
            Self::DeadLetter => "dead_letter",
            Self::Broadcast => "broadcast",
        })
    }
}

/// The bounds of the sequencing histogram, and their spelling in seconds.
const SEQUENCING_BOUNDS: &[(Duration, StaticKey<LabelValues>)] = &[
    (Duration::from_millis(1), StaticKey::new("0.001")),
    (Duration::from_millis(5), StaticKey::new("0.005")),
    (Duration::from_millis(10), StaticKey::new("0.01")),
    (Duration::from_millis(25), StaticKey::new("0.025")),
    (Duration::from_millis(50), StaticKey::new("0.05")),
    (Duration::from_millis(100), StaticKey::new("0.1")),
    (Duration::from_millis(250), StaticKey::new("0.25")),
    (Duration::from_millis(1000), StaticKey::new("1")),
];

/// Every metric of the relay.
pub struct RelayMetrics {
    /// The sockets open now, by kind.
    pub sockets: Gauge<Socket>,
    /// The deltas the relay sequenced or refused.
    pub deltas: Counter<Delta>,
    /// The time from the clock check of a delta to its broadcast: the
    /// sequencer, the index write, the pending list and the broadcast.
    pub sequencing: Histogram,
    /// The failures an operator acts on.
    pub failures: Counter<Failure>,
}

impl RelayMetrics {
    /// Every metric at zero.
    fn new() -> Self {
        Self {
            sockets: Gauge::new(
                StaticKey::new("ambre_relay_sockets"),
                StaticKey::new("Sockets open now, by kind."),
            ),
            deltas: Counter::new(
                StaticKey::new("ambre_relay_deltas_total"),
                StaticKey::new("Deltas the relay sequenced or refused."),
            ),
            sequencing: Histogram::new(
                StaticKey::new("ambre_relay_sequencing_seconds"),
                StaticKey::new("Time from the clock check of a delta to its broadcast."),
                SEQUENCING_BOUNDS,
            ),
            failures: Counter::new(
                StaticKey::new("ambre_relay_failures_total"),
                StaticKey::new("Failures that lose no delta and need an operator."),
            ),
        }
    }

    /// The text of a scrape. The destructure names every field, so a new
    /// metric does not compile until it writes itself here.
    pub fn exposition(&self) -> String {
        let Self {
            sockets,
            deltas,
            sequencing,
            failures,
        } = self;
        let mut out = Exposition::new();
        sockets.write(&mut out);
        deltas.write(&mut out);
        sequencing.write(&mut out);
        failures.write(&mut out);
        out.into_text()
    }
}

/// The metrics of this process.
pub static RELAY: LazyLock<RelayMetrics> = LazyLock::new(RelayMetrics::new);

/// Counts one failure of `failure`. The caller logs it too.
pub fn failed(failure: Failure) {
    RELAY.failures.inc(failure);
}

/// An open socket of one kind, counted until it drops.
pub struct OpenSocket(Socket);

impl OpenSocket {
    /// Counts one more socket of `kind`.
    pub fn of(kind: Socket) -> Self {
        RELAY.sockets.inc(kind);
        Self(kind)
    }
}

impl Drop for OpenSocket {
    fn drop(&mut self) {
        RELAY.sockets.dec(self.0);
    }
}

/// The time one delta spends from its clock check to its broadcast.
pub struct Sequencing(Instant);

impl Sequencing {
    /// Starts the clock of one delta.
    pub fn start() -> Self {
        Self(Instant::now())
    }

    /// Counts the delta sequenced, and the time it took.
    pub fn done(self) {
        RELAY.sequencing.observe(self.0.elapsed());
        RELAY.deltas.inc(Delta::Sequenced);
    }
}

/// The router of the metrics listener: the scrape route alone, behind
/// `access` (`SRV-307`). Its answers carry the headers of every answer of the
/// relay (`SRV-270`), a refusal included.
pub fn router(access: ScrapeAccess) -> Router {
    Router::new()
        .route(ROUTE, get(scrape))
        .layer(axum::middleware::from_fn_with_state(access, guard))
        .layer(axum::middleware::map_response(crate::stamp))
}

/// Answers `401 Unauthorized` to a scrape `access` refuses, before any
/// metric is read.
async fn guard(State(access): State<ScrapeAccess>, request: Request, next: Next) -> Response {
    let presented = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .map(|text| Credential::<ScrapeToken>::new(text.to_owned()));
    match access.admits(presented.as_ref()) {
        Scrape::Admitted => next.run(request).await,
        Scrape::Refused => StatusCode::UNAUTHORIZED.into_response(),
    }
}

/// Answers a scrape.
async fn scrape() -> impl IntoResponse {
    ([(header::CONTENT_TYPE, TEXT_FORMAT)], RELAY.exposition())
}

/// The address `METRICS_ADDR` sets, or the loopback when it sets none.
///
/// # Errors
///
/// The variable holds no socket address.
pub fn address() -> Result<SocketAddr, std::net::AddrParseError> {
    std::env::var("METRICS_ADDR").map_or(Ok(FALLBACK), |text| text.parse())
}

/// Serves the metrics on `listener`, behind `access`, until it fails.
///
/// # Errors
///
/// The I/O error of the listener.
pub async fn serve(listener: tokio::net::TcpListener, access: ScrapeAccess) -> std::io::Result<()> {
    axum::serve(listener, router(access)).await
}
