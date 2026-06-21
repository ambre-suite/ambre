//! Ambre Blind Relay -- goose load test.
//!
//! Simulates concurrent users hitting REST endpoints. Metrics (latency
//! percentiles, throughput, error rate) are collected by goose and
//! printed every 5 seconds.
//!
//! ## Usage
//!
//! ```sh
//! # Default: 1000 users, 120s, localhost
//! ./scripts/load_test.sh
//!
//! # Custom:
//! cargo run --example goose_load_test -- \
//!   -H http://localhost:3000 -u 500 -r 50 -t 60s \
//!   --report-file report.html
//! ```
//!
//! ## CLI flags (goose built-in)
//!
//! | Flag              | Default                        |
//! |-------------------|--------------------------------|
//! | `-H, --host`      | `http://localhost:3000`        |
//! | `-u, --users`     | 1000                           |
//! | `-r, --hatch-rate`| 50                             |
//! | `-t, --run-time`  | 120s                           |
//! | `--report-file`   | (none: generate HTML report)   |

use goose::prelude::*;
use reqwest::cookie::Jar;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

// Number of active documents (user N maps to doc-(N % ACTIVE_DOCS)).
const ACTIVE_DOCS: usize = 100;

static NEXT_DOC_ID: AtomicUsize = AtomicUsize::new(0);

// -- On-start: per-user session -------------------------------------------

/// Sets up a per-user cookie jar with a session cookie so that
/// subsequent requests are authenticated.
async fn session_setup(user: &mut GooseUser) -> TransactionResult {
    let idx = NEXT_DOC_ID.fetch_add(1, Ordering::Relaxed);
    let doc_id = format!("doc-{}", (idx % ACTIVE_DOCS) + 1);
    let session_id = format!("load-{}", uuid::Uuid::new_v4());

    // Build a cookie jar containing the session cookie.
    let jar = Jar::default();
    jar.add_cookie_str(&format!("session_id={session_id}"), &user.base_url);
    jar.add_cookie_str(
        &format!("csrf_token=load-csrf-{session_id}"),
        &user.base_url,
    );

    let builder = reqwest::Client::builder()
        .cookie_store(true)
        .cookie_provider(Arc::new(jar))
        .timeout(std::time::Duration::from_secs(30));

    user.set_client_builder(builder).await?;

    // Store the doc_id for use by subsequent transactions.
    user.set_session_data(doc_id);
    Ok(())
}

// -- REST transactions ---------------------------------------------------

/// `GET /api/documents` — list all documents for the authenticated user.
async fn list_documents(user: &mut GooseUser) -> TransactionResult {
    let _goose = user.get("/api/documents").await?;
    Ok(())
}

/// `GET /api/documents/{id}` — fetch a single document's metadata.
async fn get_document(user: &mut GooseUser) -> TransactionResult {
    let doc_id = user.get_session_data::<String>().unwrap().clone();
    let _goose = user.get(&format!("/api/documents/{doc_id}")).await?;
    Ok(())
}

/// `GET /api/documents/{id}/snapshot` — fetch latest snapshot metadata.
async fn get_snapshot(user: &mut GooseUser) -> TransactionResult {
    let doc_id = user.get_session_data::<String>().unwrap().clone();
    let _goose = user
        .get(&format!("/api/documents/{doc_id}/snapshot"))
        .await?;
    Ok(())
}

/// `GET /api/documents/{id}/deltas` — fetch deltas since seq 0.
async fn get_deltas(user: &mut GooseUser) -> TransactionResult {
    let doc_id = user.get_session_data::<String>().unwrap().clone();
    let _goose = user
        .get(&format!("/api/documents/{doc_id}/deltas?from_seq=0"))
        .await?;
    Ok(())
}

// -- Main -----------------------------------------------------------------

#[tokio::main]
async fn main() -> Result<(), GooseError> {
    GooseAttack::initialize()?
        .register_scenario(
            scenario!("AmbreAPI")
                .set_wait_time(
                    std::time::Duration::from_millis(50),
                    std::time::Duration::from_millis(200),
                )?
                // On-start: each user creates its own session cookie.
                .register_transaction(transaction!(session_setup).set_on_start())
                // Weighted REST transactions.
                .register_transaction(transaction!(list_documents).set_weight(3)?)
                .register_transaction(transaction!(get_document).set_weight(3)?)
                .register_transaction(transaction!(get_snapshot).set_weight(2)?)
                .register_transaction(transaction!(get_deltas).set_weight(2)?),
        )
        .execute()
        .await?;

    Ok(())
}
