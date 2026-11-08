//! The guard of one path per seed: each seeded run of `sim/tests.rs`, in separate processes,
//! must trace the same path (`CALL-053`).
//!
//! Two processes differ in what the code does not choose. That covers the seed of a hash
//! map, the generator of the OS, the wall clock and the instant the process starts. A run
//! that reads any of them on its path diverges between two processes and not inside one.
//! So the guard starts the test binary again, several times, and compares the traces the
//! runs print.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::process::{Command, Stdio};

use super::net::Net;

/// The variable that turns the trace on in a child process.
const TRACE: &str = "AMBRE_SIM_TRACE";

/// The variable that sets the count of processes. CI runs the default. `just sim-paths`
/// runs 20, in the release build.
const PROCESSES: &str = "AMBRE_SIM_PROCESSES";
const DEFAULT_PROCESSES: usize = 3;

/// The prefix of a trace line on standard error.
const PREFIX: &str = "path ";

impl Net {
    /// The trace of the path of this run: its seed and the digest of every datagram. It adds
    /// each switch of layer or of sender that a receiver saw, with its simulated time.
    fn trace(&self) -> String {
        let mut line = format!(
            "seed {}: {} datagrams, digest {:016x}; switches:",
            self.seed, self.path.datagrams, self.path.digest
        );
        for (receiver, m) in self.members.iter().enumerate() {
            for rx in m.synth.video.values() {
                for pair in rx.arrivals.windows(2) {
                    if let [a, b] = pair
                        && (a.layer != b.layer || a.sender != b.sender)
                    {
                        let at = (b.at - self.epoch).as_micros();
                        let _ = write!(
                            line,
                            " r{receiver} {}/{}->{}/{} at {at} us,",
                            a.sender, a.layer, b.sender, b.layer
                        );
                    }
                }
            }
        }
        line
    }
}

impl Drop for Net {
    /// Prints the trace of the run under [`TRACE`], with the test that ran it. libtest names
    /// the thread of each test after it.
    fn drop(&mut self) {
        if std::env::var_os(TRACE).is_some() {
            let test = std::thread::current().name().unwrap_or("?").to_owned();
            eprintln!("{PREFIX}{test} {}", self.trace());
        }
    }
}

/// The traces one process printed: per test, its runs in order.
type Traces = BTreeMap<String, Vec<String>>;

/// Starts a child process that runs every test of `sim::tests` with the trace on.
fn child() -> std::process::Child {
    let exe = std::env::current_exe().expect("the path of the test binary");
    Command::new(exe)
        .args(["sim::tests::", "--nocapture"])
        .env(TRACE, "1")
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("a child process")
}

/// Reads the trace lines of one finished child.
fn traces(stderr: &[u8]) -> Traces {
    let mut traces = Traces::new();
    for line in String::from_utf8_lossy(stderr).lines() {
        let Some(rest) = line.strip_prefix(PREFIX) else {
            continue;
        };
        let (test, trace) = rest.split_once(' ').unwrap_or((rest, ""));
        traces
            .entry(test.to_owned())
            .or_default()
            .push(trace.to_owned());
    }
    traces
}

/// `CALL-053`: each seeded run follows one path in separate processes. The child processes
/// run concurrently, so each one runs under the load of the others.
#[test]
fn each_seed_follows_one_path_in_separate_processes() {
    let count = std::env::var(PROCESSES)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_PROCESSES);
    let children: Vec<_> = (0..count).map(|_| child()).collect();
    let runs: Vec<Traces> = children
        .into_iter()
        .map(|c| {
            let out = c.wait_with_output().expect("the child ends");
            assert!(
                out.status.success(),
                "a child process failed a test of the simulation"
            );
            traces(&out.stderr)
        })
        .collect();
    let first = runs.first().expect("one process at least");
    assert!(
        first.len() >= 10,
        "the trace names {} tests: {:?}",
        first.len(),
        first.keys()
    );
    for (test, paths) in first {
        for (index, run) in runs.iter().enumerate().skip(1) {
            let other = run.get(test).cloned().unwrap_or_default();
            for (a, b) in paths.iter().zip(&other) {
                assert_eq!(a, b, "{test}: process {index} diverges from process 0");
            }
            assert_eq!(
                paths.len(),
                other.len(),
                "{test}: process {index} traced another count of runs"
            );
        }
    }
    eprintln!(
        "{count} processes traced {} runs of {} tests on one path each",
        first.values().map(Vec::len).sum::<usize>(),
        first.len()
    );
}
