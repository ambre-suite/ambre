//! What the relay binary refuses before it connects to any store (`SRV-256`).
//!
//! Each test starts the binary with a chosen environment. A refusal happens
//! before the first connection, so no test needs a backing service.

#![expect(
    clippy::expect_used,
    reason = "a test asserts by panicking (ARCH-028 holds outside the tests)"
)]

use std::process::{Command, Output};

/// The variables a test sets or removes. Cargo gives the local stack's
/// addresses to each test process (`.cargo/config.toml`), so each test
/// removes them all first.
const READ_AT_START: [&str; 6] = [
    "DATABASE_URL",
    "DEV_MODE",
    "S3_ENDPOINT",
    "AWS_ENDPOINT_URL",
    "AWS_ACCESS_KEY_ID",
    "AWS_SECRET_ACCESS_KEY",
];

/// Runs the relay with `vars` set on top of an environment cleared of
/// `READ_AT_START`, and waits for it to exit.
fn run_relay(vars: &[(&str, &str)]) -> Output {
    let mut relay = Command::new(env!("CARGO_BIN_EXE_ambre-server"));
    for name in READ_AT_START {
        relay.env_remove(name);
    }
    relay
        .envs(vars.iter().copied())
        .output()
        .expect("run the relay binary")
}

/// The text of `bytes`, for an assertion message.
fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

#[test]
fn the_relay_refuses_to_start_without_its_database_address() {
    let out = run_relay(&[]);
    assert!(!out.status.success());
    let stderr = text(&out.stderr);
    assert!(
        stderr.contains("DATABASE_URL must be set outside DEV_MODE"),
        "{stderr}"
    );
}

#[test]
fn the_relay_refuses_to_start_without_its_object_store_keys() {
    // An address no store answers on: the refusal comes before any connection.
    let database = ("DATABASE_URL", "postgres://nobody@127.0.0.1:1/none");
    let endpoint = ("S3_ENDPOINT", "http://127.0.0.1:1");
    let out = run_relay(&[database, endpoint]);
    assert!(!out.status.success());
    let stderr = text(&out.stderr);
    assert!(
        stderr.contains("AWS_ACCESS_KEY_ID must be set outside DEV_MODE"),
        "{stderr}"
    );

    let out = run_relay(&[database, endpoint, ("AWS_ACCESS_KEY_ID", "GKnamed")]);
    assert!(!out.status.success());
    let stderr = text(&out.stderr);
    assert!(
        stderr.contains("AWS_SECRET_ACCESS_KEY must be set outside DEV_MODE"),
        "{stderr}"
    );
}

#[test]
fn development_mode_falls_back_to_the_keys_of_the_local_stack() {
    // A database address that does not parse ends the run at the connection,
    // after the keys are read.
    let out = run_relay(&[
        ("DEV_MODE", "1"),
        ("DATABASE_URL", "not a database address"),
        ("S3_ENDPOINT", "http://127.0.0.1:1"),
    ]);
    assert!(!out.status.success());
    let stdout = text(&out.stdout);
    let stderr = text(&out.stderr);
    assert!(!stderr.contains("must be set"), "{stderr}");
    assert!(
        stdout.contains("AWS_ACCESS_KEY_ID unset: the relay uses the value of the local stack"),
        "{stdout}"
    );
}
