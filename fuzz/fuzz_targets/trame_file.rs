//! Any bytes a user imports into a workbook, or a peer sends as a delta, read
//! to a result or to a typed error. The `.ambre` payload is wrapped in a valid
//! container, so the input reaches the schema gate and the CRDT engine.
#![no_main]

use ambre_core::{DocType, Uuid, docfile::write_ambre_plaintext};
use ambre_trame::{TrameCommand, TrameState};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let mut state = TrameState::new();
    let _ = state.handle_command(TrameCommand::ImportCsv {
        data: data.to_vec(),
        delimiter: None,
        has_header: false,
        target_sheet_id: None,
        start_row: 0,
        start_col: 0,
    });
    let file = write_ambre_plaintext(DocType::Trame, Uuid::from_bytes([7; 16]), data);
    let _ = TrameState::new().import_ambre_plaintext(&file);
    let _ = TrameState::new().apply_remote(data);
});
