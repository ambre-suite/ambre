//! Blind Relay server for Ambre.
//!
//! This is a native Rust binary (Axum, Tokio) that acts as a zero‑knowledge relay:
//! it sees only metadata (`document_id`, `pub_key`, `clock`, `seq`, `snapshot_id`, `nonce`)
//! and never sees plaintext CRDT content, document keys, or master seeds.

mod state;
mod ws;

#[tokio::main]
async fn main() -> Result<(), String> {
    Ok(())
}
