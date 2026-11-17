//! Any bytes a converter module answers as a Prose form import on an author
//! and a peer: the form lands whole on both, or the author receives a
//! refusal and both hold the document as it was (`PROSE-IO-112`).
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Err(breach) =
        ambre_prose::logic::fuzz::import_form(&ambre_core::Payload::new(data.to_vec()))
    {
        panic!("the import broke its outcome: {breach:?}");
    }
});
