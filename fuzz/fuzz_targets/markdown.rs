//! Any bytes a user imports as Markdown read to a form or to a typed error,
//! through the markup engine the host links in place, with the request and
//! answer bytes a module reads and writes.
#![no_main]

use ambre_convert::Converter;
use ambre_core::{ConverterFamily, Payload};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Ok(converter) = Converter::load(ConverterFamily::Markup, &Payload::new(Vec::new())) else {
        return;
    };
    let file = Payload::<ambre_client_core::FileBytes>::new(data.to_vec());
    // The request and the answer the host runs, in place.
    let Ok(request) = ambre_prose::interop::markdown::read_request(&file) else {
        return;
    };
    let _ = ambre_prose::interop::markdown::read_answer(converter.convert(&request));
});
