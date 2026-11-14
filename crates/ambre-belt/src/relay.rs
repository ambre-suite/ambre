//! The relay, run as a deployment runs it: the `ambre-server` binary.
//!
//! The belt starts the binary on a free port in dev mode, so the page signs in
//! with a name and no provider. The page reaches it through the belt's own
//! server, which forwards `/api` there, as Trunk's proxy does in development.
//! The relay needs its three backing services (`just services-up`). It signs
//! its object store requests with the keys of the local store, never with the
//! shell's AWS keys: `BELT_S3_ACCESS_KEY` and `BELT_S3_SECRET_KEY` name others.

use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// One running relay process, killed when the test ends.
pub struct Relay {
    pub base: String,
    process: Child,
}

/// The relay binary: `BELT_RELAY`, or the debug build of the workspace.
fn binary() -> String {
    std::env::var("BELT_RELAY").unwrap_or_else(|_| {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/debug/ambre-server")
            .display()
            .to_string()
    })
}

/// The variable `name`, or `default` when it is unset.
fn belt_var(name: &str, default: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| default.to_owned())
}

impl Relay {
    /// Starts the relay and waits until it answers.
    pub async fn start() -> Self {
        Self::start_with(&[]).await
    }

    /// As [`Relay::start`], with the variables `env` set: the admission key of a call and the
    /// address of its SFU, for a call test.
    pub async fn start_with(env: &[(&str, String)]) -> Self {
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .and_then(|l| l.local_addr())
            .map(|a| a.port())
            .expect("a free local port");
        let process = Command::new(binary())
            .env("BIND_ADDR", format!("127.0.0.1:{port}"))
            // Each relay of a parallel run scrapes on a port of its own.
            .env("METRICS_ADDR", "127.0.0.1:0")
            .env("DEV_MODE", "true")
            // The belt's server is the reverse proxy, and gives each page an
            // address of its own (`serve::page_address`).
            .env("TRUST_PROXY_HEADERS", "true")
            .env(
                "S3_ENDPOINT",
                std::env::var("S3_ENDPOINT").unwrap_or_else(|_| "http://localhost:9000".into()),
            )
            // The relay reads its object store keys from the AWS variables. An
            // AWS profile in the shell would sign the uploads with a key the
            // local store refuses, so the belt names the store's own keys. They
            // are the dev key of `scripts/garage_init.sh`.
            .env(
                "AWS_ACCESS_KEY_ID",
                belt_var("BELT_S3_ACCESS_KEY", "GK1c7374dda8bf9b67e1ed2022"),
            )
            .env(
                "AWS_SECRET_ACCESS_KEY",
                belt_var(
                    "BELT_S3_SECRET_KEY",
                    "b64c22af0c1f051c482b80d9cbde6751304fd0d084a565d2a5fb0342021899ca",
                ),
            )
            .envs(env.iter().map(|(k, v)| (*k, v.as_str())))
            .stdout(Stdio::null())
            .stderr(if std::env::var_os("BELT_VERBOSE").is_some() {
                Stdio::inherit()
            } else {
                Stdio::null()
            })
            .spawn()
            .expect("start the relay: run `cargo build -p ambre-server`, or set BELT_RELAY");
        let base = format!("http://127.0.0.1:{port}");
        let http = reqwest::Client::builder()
            .no_proxy()
            .build()
            .expect("an HTTP client");
        let probe = crate::serve::page_address();
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            let up = http
                .get(format!("{base}/api/auth/config"))
                .header(crate::serve::FORWARDED_FOR, &probe)
                .send()
                .await;
            if up.is_ok_and(|r| r.status().is_success()) {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "the relay did not answer: are postgres, redis and the object store up?"
            );
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
        Self { base, process }
    }
}

impl Drop for Relay {
    fn drop(&mut self) {
        let _ = self.process.kill();
        let _ = self.process.wait();
    }
}
