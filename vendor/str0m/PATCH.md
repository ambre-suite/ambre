# Patched copy of str0m 0.25.0

Upstream: https://github.com/algesten/str0m (crate `str0m` 0.25.0, commit
5f2183ce7c67117cd1960dfeb11bed11fc188b66), licensed MIT OR Apache-2.0 by Martin
Algesten and the str0m contributors. The workspace uses this copy through
`[patch.crates-io]` in the root `Cargo.toml`.

The copy holds `src/`, `Cargo.toml`, `LICENSE-MIT.txt` and `README.md` of the
published crate. The examples, the integration tests, the documents, the logo,
the test certificate and its scripts, `CHANGELOG.md`, `README.tpl`,
`Cargo.lock` and `Cargo.toml.orig` are left out.

## Why

The SFU runs a congestion control of its own per receiver
(`crates/ambre-sfu/src/congestion.rs`). str0m keeps the transport-wide feedback
and its estimate to itself, so the copy opens two doors: it hands the feedback
out, and it takes an estimate in. The simulation of the SFU also needs str0m to
pick among its streams in a fixed order.

## The change

Each change against the published crate, read on 2026-11-08 by a `diff -r`
against the crate from crates.io. Each added public item carries the words
"Patched in by Ambre" in its rustdoc.

* `Cargo.toml`: drops the `[[example]]` and `[[test]]` tables, which name the
  files left out. Nothing else differs.
* `src/config.rs`: `RtcConfig` gains the field `twcc_acks`, false by default,
  and the builder `RtcConfig::enable_twcc_acks`.
* `src/lib.rs`: `Event` gains the variant `EgressTwccFeedback(Vec<bwe::TwccAck>)`.
* `src/bwe/api.rs`:
  * the struct `bwe::TwccAck`: one packet of a feedback report, with its
    transport-wide sequence number, its send time, its size, its remote arrival
    or none when lost, and the arrival of the report;
  * the method `Bwe::set_estimate`, which hands the estimate of the
    application to the session.
* `src/session.rs`:
  * the field `twcc_acks`, present when the configuration enables it;
  * the handling of a TWCC report collects each send record into it, as one
    `TwccAck`, before the estimator of str0m reads the same records;
  * `poll_event` emits the collected acks as `Event::EgressTwccFeedback` before
    any other event of the session;
  * the method `set_bwe_estimate`, which sets the external estimate and
    configures the pacer again.
* `src/bwe/mod.rs`, the wrapper `Bwe` over the estimator:
  * the field `external`, the estimate of the application, and its setter
    `set_external`;
  * `last_estimate` answers `external` when set, so the pacer and the padding
    follow it;
  * `handle_timeout` starts no probe cluster while `external` is set;
  * `poll_estimate` answers none while `external` is set, so the session emits
    no `Event::EgressBitrateEstimate`;
  * `is_overusing` answers false while `external` is set, so the pacer control
    keeps the padding the application asks for.
* `src/streams/mod.rs`: `Streams` keeps `streams_rx`, `rx_lookup` and
  `streams_tx` in a `BTreeMap` by SSRC, where upstream keeps a `HashMap`. The
  pacer picks among streams in the order of that map, and the seed of a
  `HashMap` changes with each process. With the map sorted and the `fastrand`
  generator of the thread seeded by the caller, a simulated call follows one
  path. The copy itself seeds nothing.

Drop this directory and the patch line once str0m ships a way to run an
external congestion control and a fixed stream order.
