//! Any bytes a user imports onto a board, or a peer sends as a delta, read to
//! a result or to a typed error. The `.ambre` payload is wrapped in a valid
//! container, so the input reaches the schema gate and the CRDT engine.
#![no_main]

use ambre_core::{
    DeltaBytes, DocType, DocumentId, FileBytes, SnapshotBytes, Uuid, docfile::write_ambre_plaintext,
};
use ambre_toile::ToileState;
use ambre_toile::logic::interop::{Limits, import_raster, import_svg};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let limits = Limits::default();
    let picked = FileBytes::new(data.to_vec());
    let _ = import_svg(&picked, &limits);
    let _ = import_raster(&picked, &limits);
    let file = write_ambre_plaintext(
        DocType::Toile,
        DocumentId::from_uuid(Uuid::from_bytes([7; 16])),
        &SnapshotBytes::new(data.to_vec()),
    );
    let _ = ToileState::new().import_ambre_plaintext(&file);
    let _ = ToileState::new().apply_remote(&DeltaBytes::new(data.to_vec()));
});
