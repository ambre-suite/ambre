//! The relay, run as a deployment runs it: the `ambre-server` binary.
//!
//! The belt starts the binary on a free port in dev mode, so the page signs in
//! with a name and no provider. The page reaches it through the belt's own
//! server, which forwards `/api` there, as Trunk's proxy does in development.
//! The relay needs its three backing services (`just services-up`).

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

impl Relay {
    /// Starts the relay and waits until it answers.
    pub async fn start() -> Self {
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .and_then(|l| l.local_addr())
            .map(|a| a.port())
            .expect("a free local port");
        let process = Command::new(binary())
            .env("BIND_ADDR", format!("127.0.0.1:{port}"))
            .env("DEV_MODE", "true")
            // The belt's server is the reverse proxy, and gives each page an
            // address of its own (`serve::page_address`).
            .env("TRUST_PROXY_HEADERS", "true")
            .env(
                "S3_ENDPOINT",
                std::env::var("S3_ENDPOINT").unwrap_or_else(|_| "http://localhost:9000".into()),
            )
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
