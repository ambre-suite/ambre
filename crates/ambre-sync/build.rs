//! Hands the converter module's digest to the compiler (`ARCH-080`).
//!
//! `scripts/converter_build.py` builds the module before Trunk builds the
//! instances (`Trunk.toml`, the `pre_build` hook), and writes the module's
//! BLAKE2b-256 digest in hex beside it. The Sync worker carries that digest,
//! and refuses fetched bytes of any other. A wasm build made without the hook
//! carries none, and the worker answers that this build has no converter.
//!
//! The host build reads nothing: no host code fetches the module.

use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    if std::env::var("CARGO_CFG_TARGET_ARCH").as_deref() != Ok("wasm32") {
        return;
    }
    let Ok(manifest) = std::env::var("CARGO_MANIFEST_DIR") else {
        return;
    };
    // The path the build script writes, from the workspace root.
    let digest: PathBuf = [
        &manifest,
        "..",
        "..",
        "target",
        "converter",
        "ambre-convert.digest",
    ]
    .iter()
    .collect();
    println!("cargo:rerun-if-changed={}", digest.display());
    if let Ok(hex) = std::fs::read_to_string(&digest) {
        println!("cargo:rustc-env=AMBRE_CONVERTER_DIGEST={}", hex.trim());
    }
}
