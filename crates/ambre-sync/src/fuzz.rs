//! The sealed open path, as the fuzz targets in `fuzz/` drive it (`ARCH-029`).
//!
//! Two kinds of sender reach this path. The relay can send any bytes, and the
//! signature check stops them. A peer who holds the document key can seal any
//! plaintext, and the decompression reads it after every check has passed.
//! Each entry point below opens one of the two under a fixed key.

use ambre_core::id::{self, Document, Id};
use ambre_core::{FixedBytes, Payload, PublicKey};
use ambre_crypto::{DocumentKey, MasterSeed, envelope};
use ambre_protocol::messages::Clock;
use ambre_protocol::public_data::UpdatePublicData;
use ambre_protocol::{Snapshot, Update};

pub use crate::pipeline::PipelineError;
use crate::pipeline::{FetchedBlob, UpdateFrame, decode_snapshot_blob, open_snapshot, open_update};

/// The document key every entry point opens under.
fn doc_key() -> DocumentKey {
    DocumentKey::from_array([0x11; 32])
}

/// Opens `bytes` as an update the relay sent.
///
/// # Errors
///
/// [`PipelineError::Postcard`] for bytes that are not an update, and every
/// error of the open path for one that is.
pub fn open_relayed_update(bytes: &[u8]) -> Result<Vec<u8>, PipelineError> {
    let update: Update = postcard::from_bytes(bytes).map_err(|_| PipelineError::Postcard)?;
    open_update(&update, &doc_key()).map(UpdateFrame::into_bytes)
}

/// Opens `bytes` as a snapshot blob read from object storage.
///
/// # Errors
///
/// [`PipelineError::Postcard`] for bytes that are not a snapshot, and every
/// error of the open path for one that is.
pub fn open_stored_snapshot(bytes: &[u8]) -> Result<Vec<u8>, PipelineError> {
    let named: Snapshot = postcard::from_bytes(bytes).map_err(|_| PipelineError::Postcard)?;
    let snapshot = decode_snapshot_blob(
        &Payload::<FetchedBlob>::new(bytes.to_vec()),
        named.public_data.document_id,
    )?;
    open_snapshot(&snapshot, &doc_key()).map(ambre_core::Payload::into_bytes)
}

/// Opens an update whose sealed plaintext is `plaintext`, as a peer holding
/// the document key would send it. The first byte names the compression.
///
/// # Errors
///
/// Every error of the decompression, which reads `plaintext` after the
/// signature and the tag have both verified.
pub fn open_authenticated(plaintext: &[u8]) -> Result<Vec<u8>, PipelineError> {
    let key = doc_key();
    let signing = MasterSeed::from_array([0x22; 32]).derive_signing_key();
    let public_data = UpdatePublicData {
        document_id: Id::<Document>::mint(),
        pub_key: FixedBytes::<PublicKey>::from_array(signing.verifying_key().to_bytes()),
        key_id: key.id(),
        snapshot_id: Id::<id::Snapshot>::mint(),
        clock: Clock::new(1),
    };
    let aad = postcard::to_allocvec(&public_data)
        .map(ambre_core::Aad::new)
        .map_err(|_| PipelineError::Postcard)?;
    let plaintext = ambre_core::Confidential::<ambre_core::Plaintext>::new(plaintext.to_vec());
    let sealed = envelope::seal(&plaintext, &aad, &key, &signing)?;
    let update = Update {
        ciphertext: sealed.ciphertext,
        nonce: sealed.nonce,
        public_data,
        commitment_tag: sealed.commitment_tag,
        signature: sealed.signature,
    };
    open_update(&update, &key).map(UpdateFrame::into_bytes)
}
