use std::time::Duration;

use ambre_core::{Count, LabelValues, MetricNames, StaticKey};

use super::{Bound, Counter, Exposition, Gauge, Histogram, Label, Whole};

/// A closed label of two values, as a process declares one.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Outcome {
    Stored,
    Refused,
}

impl Label for Outcome {
    const KEY: Option<StaticKey<MetricNames>> = Some(StaticKey::new("outcome"));
    const ALL: &'static [Self] = &[Self::Stored, Self::Refused];
    fn value(self) -> StaticKey<LabelValues> {
        StaticKey::new(match self {
            Self::Stored => "stored",
            Self::Refused => "refused",
        })
    }
}

const BOUNDS: &[(Duration, StaticKey<LabelValues>)] = &[
    (Duration::from_millis(1), StaticKey::new("0.001")),
    (Duration::from_millis(10), StaticKey::new("0.01")),
];

// Each kind writes the lines of the Prometheus text format, and a series of
// every value exists before its first event.
#[test]
fn each_metric_writes_the_text_format_with_every_series_from_the_start() {
    let deltas = Counter::<Outcome>::new(
        StaticKey::new("ambre_test_deltas_total"),
        StaticKey::new("Deltas, by outcome."),
    );
    let sockets = Gauge::<Whole>::new(
        StaticKey::new("ambre_test_sockets"),
        StaticKey::new("Open sockets."),
    );
    let took = Histogram::new(
        StaticKey::new("ambre_test_seconds"),
        StaticKey::new("Time to store."),
        BOUNDS,
    );
    deltas.inc(Outcome::Stored);
    deltas.add(Outcome::Stored, Count::new(2));
    sockets.inc(Whole::All);
    sockets.inc(Whole::All);
    sockets.dec(Whole::All);
    took.observe(Duration::from_micros(500));
    took.observe(Duration::from_millis(5));
    took.observe(Duration::from_secs(1));
    let mut out = Exposition::new();
    deltas.write(&mut out);
    sockets.write(&mut out);
    took.write(&mut out);
    assert_eq!(
        out.into_text(),
        "# HELP ambre_test_deltas_total Deltas, by outcome.\n\
         # TYPE ambre_test_deltas_total counter\n\
         ambre_test_deltas_total{outcome=\"stored\"} 3\n\
         ambre_test_deltas_total{outcome=\"refused\"} 0\n\
         # HELP ambre_test_sockets Open sockets.\n\
         # TYPE ambre_test_sockets gauge\n\
         ambre_test_sockets 1\n\
         # HELP ambre_test_seconds Time to store.\n\
         # TYPE ambre_test_seconds histogram\n\
         ambre_test_seconds_bucket{le=\"0.001\"} 1\n\
         ambre_test_seconds_bucket{le=\"0.01\"} 2\n\
         ambre_test_seconds_bucket{le=\"+Inf\"} 3\n\
         ambre_test_seconds_sum 1.0055\n\
         ambre_test_seconds_count 3\n"
    );
    assert_eq!(took.read(Bound::Upto(Duration::from_millis(10))).get(), 2);
    assert_eq!(took.read(Bound::Infinite).get(), 3);
}

// A gauge never goes below zero, and a set replaces the level.
#[test]
fn a_gauge_holds_at_zero_and_a_set_replaces_its_level() {
    let rooms = Gauge::<Whole>::new(StaticKey::new("r"), StaticKey::new("h"));
    rooms.dec(Whole::All);
    assert!(rooms.read(Whole::All).is_zero());
    rooms.set(Whole::All, Count::new(7));
    assert_eq!(rooms.read(Whole::All).get(), 7);
}

// A help text and a label value are escaped as the text format reads them.
#[test]
fn a_help_text_and_a_label_value_are_escaped() {
    let help = StaticKey::<MetricNames>::new("a\\b\nc\"d");
    assert_eq!(super::escaped(help, super::Quote::Off), "a\\\\b\\nc\"d");
    let value = StaticKey::<LabelValues>::new("a\"b");
    assert_eq!(super::escaped(value, super::Quote::On), "a\\\"b");
}
