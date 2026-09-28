//! Any bytes a user opens as an `.ambre` file read to a header or to a typed
//! error, on the plaintext and the sealed lineage alike.
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = ambre_core::docfile::classify(data);
    let _ = ambre_core::docfile::parse_header(data);
    let _ = ambre_core::docfile::read_ambre_plaintext(data);
    let _ = ambre_core::docfile::parse_sealed_header(data);
});
