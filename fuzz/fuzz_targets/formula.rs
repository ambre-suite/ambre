//! Any text typed into a cell parses to an expression or to an error.
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|text: &str| {
    let _ = ambre_trame::formula::parser::parse_formula(text);
});
