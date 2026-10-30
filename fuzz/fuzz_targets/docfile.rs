//! Any bytes a user opens as an `.ambre` file read to a header or to a typed
//! error, on the plaintext and the sealed lineage alike.
#![no_main]

use ambre_core::{PickedContainer, PlaintextContainer, SealedContainer};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = ambre_core::docfile::classify(&PickedContainer::new(data.to_vec()));
    let _ = ambre_core::docfile::parse_header(data);
    let _ = ambre_core::docfile::read_ambre_plaintext(&PlaintextContainer::new(data.to_vec()));
    let _ = ambre_core::docfile::parse_sealed_header(&SealedContainer::new(data.to_vec()));
});
