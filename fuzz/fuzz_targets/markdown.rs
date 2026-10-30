//! Any bytes a user imports as Markdown read to a form or to a typed error.
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let file = ambre_core::FileBytes::new(data.to_vec());
    let _ = ambre_prose::interop::markdown_read::read(&file);
});
