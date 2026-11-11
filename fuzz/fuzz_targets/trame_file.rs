//! Any bytes a user imports into a workbook, or a peer sends as a delta, read
//! to a result or to a typed error. The `.ambre` payload is wrapped in a valid
//! container, so the input reaches the schema gate and the CRDT engine.
#![no_main]

use ambre_bus::interop::InteropRequest;
use ambre_convert::Converter;
use ambre_core::id::{Document, Id};
use ambre_core::index::Pos;
use ambre_core::{
    ConverterFamily, DeltaBytes, DocType, FileBytes, Payload, SnapshotBytes, Uuid,
    docfile::write_ambre_plaintext,
};
use ambre_trame::message::{CsvImport, TrameImport};
use ambre_trame::{Col, HeaderRow, Row, TrameCommand, TrameState};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // The host links the CSV reader of the tables module in place, so the
    // bytes reach the parser the module runs.
    let Ok(tables) = Converter::load(ConverterFamily::Tables, &Payload::new(Vec::new())) else {
        return;
    };
    let mut state = TrameState::new();
    let csv = CsvImport {
        data: Payload::<FileBytes>::new(data.to_vec()),
        delimiter: None,
        header: HeaderRow::Absent,
        target_sheet_id: None,
        start_row: Pos::<Row>::ZERO,
        start_col: Pos::<Col>::ZERO,
    };
    let _ = state.handle_conversion(
        TrameCommand::Interop(InteropRequest::Import(TrameImport::Csv(csv))),
        &tables,
    );
    let file = write_ambre_plaintext(
        DocType::Trame,
        Id::<Document>::from_uuid(Uuid::from_bytes([7; 16])),
        &Payload::<SnapshotBytes>::new(data.to_vec()),
    );
    let _ = TrameState::new().import_ambre_plaintext(&file);
    let _ = TrameState::new().apply_remote(&Payload::<DeltaBytes>::new(data.to_vec()));
});
