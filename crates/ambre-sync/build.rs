//! Hands the digest of each converter module to the compiler (`ARCH-080`).
//!
//! `scripts/converter_build.py` builds one module per format family before
//! Trunk builds the instances (`Trunk.toml`, the `pre_build` hook). It writes
//! each module's BLAKE2b-256 digest in hex beside it. The Sync worker carries
//! the digests, and refuses fetched bytes of any other. A wasm build made
//! without the hook carries none, and the worker answers that this build has
//! no converter of that family.
//!
//! The host build reads nothing: no host code fetches the module.

use std::path::PathBuf;

/// The name of each converter family, as its module file carries it.
const FAMILIES: [&str; 5] = ["tables", "pdf", "markup", "images", "ooxml"];

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    if std::env::var("CARGO_CFG_TARGET_ARCH").as_deref() != Ok("wasm32") {
        return;
    }
    let Ok(manifest) = std::env::var("CARGO_MANIFEST_DIR") else {
        return;
    };
    // The families of `ambre_core::ConverterFamily`, by the name each file
    // carries. A build script links no crate of the workspace, so the list is
    // spelled here, and `src/converter.rs` reads one variable per family.
    for family in FAMILIES {
        // The path the build script writes, from the workspace root.
        let file = format!("ambre-convert-{family}.digest");
        let digest: PathBuf = [&manifest, "..", "..", "target", "converter", file.as_str()]
            .iter()
            .collect();
        println!("cargo:rerun-if-changed={}", digest.display());
        if let Ok(hex) = std::fs::read_to_string(&digest) {
            println!(
                "cargo:rustc-env=AMBRE_CONVERTER_DIGEST_{}={}",
                family.to_ascii_uppercase(),
                hex.trim()
            );
        }
    }
}
