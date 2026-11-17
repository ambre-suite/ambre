//! Rebuilds the relay when a migration changes.
//!
//! `sqlx::migrate!` in `src/main.rs` embeds `migrations/` at compile time.
//! Cargo does not track that directory on its own, so a new or edited
//! migration would leave a stale binary that runs the old schema.

fn main() {
    println!("cargo:rerun-if-changed=migrations");
}
