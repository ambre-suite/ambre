//! Any bytes a user opens as an `.ambre` file read to a header or to a typed
//! error, on the plaintext and the sealed lineage alike.
#![no_main]

use ambre_core::{Payload, PickedContainer, PlaintextContainer, SealedContainer};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = ambre_core::docfile::classify(&Payload::<PickedContainer>::new(data.to_vec()));
    let plaintext = Payload::<PlaintextContainer>::new(data.to_vec());
    let _ = ambre_core::docfile::parse_header(&plaintext);
    let _ = ambre_core::docfile::read_ambre_plaintext(&plaintext);
    let _ = ambre_core::docfile::parse_sealed_header(&Payload::<SealedContainer>::new(
        data.to_vec(),
    ));
});
