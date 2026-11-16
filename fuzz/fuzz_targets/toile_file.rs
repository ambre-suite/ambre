//! Any bytes a user imports onto a board, or a peer sends as a delta, read to
//! a result or to a typed error. The `.ambre` payload is wrapped in a valid
//! container, so the input reaches the schema gate and the CRDT engine. An SVG
//! file is read by the images engine the host links in place, then mapped. A
//! PowerPoint file is read by the office engine, then mapped onto the deck.
#![no_main]

use ambre_convert::{Conversion, Converted, Converter};
use ambre_core::ConverterFamily;
use ambre_core::id::{Document, Id};
use ambre_core::{
    DeltaBytes, DocType, FileBytes, Payload, SnapshotBytes, Uuid, docfile::write_ambre_plaintext,
};
use ambre_toile::ToileState;
use ambre_toile::logic::interop::{Limits, import_deck, import_raster, import_svg};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let limits = Limits::default();
    let picked = Payload::<FileBytes>::new(data.to_vec());
    if let Ok(converter) = Converter::load(ConverterFamily::Images, &Payload::new(Vec::new()))
        && let Ok(Converted::Svg(tree)) = converter.convert(&Conversion::ReadSvg {
            data: picked.clone(),
        })
    {
        let _ = import_svg(&tree, &limits);
    }
    if let Ok(converter) = Converter::load(ConverterFamily::Office, &Payload::new(Vec::new()))
        && let Ok(Converted::Deck { deck, thumbs }) = converter.convert(&Conversion::ReadPptx {
            data: picked.clone(),
            bound: limits.max_image,
        })
        && let Ok(read) = ambre_convert::decode_imported_deck(&deck)
    {
        let _ = import_deck(&read, &thumbs, ambre_bus::interop::FileFormat::Pptx, &limits);
    }
    let _ = import_raster(&picked, &limits);
    let file = write_ambre_plaintext(
        DocType::Toile,
        Id::<Document>::from_uuid(Uuid::from_bytes([7; 16])),
        &Payload::<SnapshotBytes>::new(data.to_vec()),
    );
    let _ = ToileState::new().import_ambre_plaintext(&file);
    let _ = ToileState::new().apply_remote(&Payload::<DeltaBytes>::new(data.to_vec()));
});
