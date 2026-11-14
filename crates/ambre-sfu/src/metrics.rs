//! The metrics of the SFU, in the Prometheus text format, on a listener of
//! their own (`CALL-101`).
//!
//! Neither the media socket nor the signalling listener serves a metric. Each
//! label is a closed `enum` of this module, so no series names a room, a
//! participant or an address (`ZK-029`, `ZK-031`).

use std::sync::LazyLock;

use ambre_core::{LabelValues, MetricNames, StaticKey};
use ambre_metrics::{Counter, Exposition, Gauge, Label, Whole};
use axum::Router;
use axum::http::header;
use axum::response::IntoResponse;
use axum::routing::get;

/// The route the scrape reads.
pub const ROUTE: &str = "/metrics";

/// The media type of the Prometheus text format, version 0.0.4.
const TEXT_FORMAT: &str = "text/plain; version=0.0.4; charset=utf-8";

/// The way a datagram crosses the media socket.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Direction {
    /// A datagram a browser sent.
    In,
    /// A datagram the SFU sent.
    Out,
    /// A datagram the SFU dropped: the send buffer of the socket was full.
    Dropped,
}

impl Label for Direction {
    const KEY: Option<StaticKey<MetricNames>> = Some(StaticKey::new("direction"));
    const ALL: &'static [Self] = &[Self::In, Self::Out, Self::Dropped];
    fn value(self) -> StaticKey<LabelValues> {
        StaticKey::new(match self {
            Self::In => "in",
            Self::Out => "out",
            Self::Dropped => "dropped",
        })
    }
}

/// What became of an admission a browser presented.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Admission {
    /// The SFU seated the participant.
    Seated,
    /// The admission was missing, late, forged or expired.
    Refused,
    /// The room was full, or the admission already held a seat.
    Full,
}

impl Label for Admission {
    const KEY: Option<StaticKey<MetricNames>> = Some(StaticKey::new("outcome"));
    const ALL: &'static [Self] = &[Self::Seated, Self::Refused, Self::Full];
    fn value(self) -> StaticKey<LabelValues> {
        StaticKey::new(match self {
            Self::Seated => "seated",
            Self::Refused => "refused",
            Self::Full => "full",
        })
    }
}

/// Every metric of the SFU.
pub struct SfuMetrics {
    /// The participants seated now.
    pub participants: Gauge<Whole>,
    /// The rooms that seat a participant now.
    pub rooms: Gauge<Whole>,
    /// The datagrams of the media socket, by direction.
    pub datagrams: Counter<Direction>,
    /// The admissions, by outcome.
    pub admissions: Counter<Admission>,
}

impl SfuMetrics {
    /// Every metric at zero.
    fn new() -> Self {
        Self {
            participants: Gauge::new(
                StaticKey::new("ambre_sfu_participants"),
                StaticKey::new("Participants seated now."),
            ),
            rooms: Gauge::new(
                StaticKey::new("ambre_sfu_rooms"),
                StaticKey::new("Rooms that seat a participant now."),
            ),
            datagrams: Counter::new(
                StaticKey::new("ambre_sfu_datagrams_total"),
                StaticKey::new("Datagrams of the media socket, by direction."),
            ),
            admissions: Counter::new(
                StaticKey::new("ambre_sfu_admissions_total"),
                StaticKey::new("Admissions a browser presented, by outcome."),
            ),
        }
    }

    /// The text of a scrape. The destructure names every field, so a new
    /// metric does not compile until it writes itself here.
    #[must_use]
    pub fn exposition(&self) -> String {
        let Self {
            participants,
            rooms,
            datagrams,
            admissions,
        } = self;
        let mut out = Exposition::new();
        participants.write(&mut out);
        rooms.write(&mut out);
        datagrams.write(&mut out);
        admissions.write(&mut out);
        out.into_text()
    }
}

/// The metrics of this process.
pub static SFU: LazyLock<SfuMetrics> = LazyLock::new(SfuMetrics::new);

/// The router of the metrics listener: the scrape route alone.
pub fn router() -> Router {
    Router::new().route(ROUTE, get(scrape))
}

/// Answers a scrape.
async fn scrape() -> impl IntoResponse {
    ([(header::CONTENT_TYPE, TEXT_FORMAT)], SFU.exposition())
}
