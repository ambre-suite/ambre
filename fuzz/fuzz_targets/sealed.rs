//! Any bytes the relay sends, object storage returns or a key-holding peer
//! seals open to a payload or to a typed error. The same holds for a
//! passphrase-sealed `.ambre` file.
#![no_main]

use std::sync::OnceLock;

use ambre_core::{FileSalt, FixedBytes, Passphrase, Payload, SealedContainer};
use ambre_crypto::FileKey;
use libfuzzer_sys::fuzz_target;

/// Derived once: the derivation costs tens of megabytes and a few hundred
/// milliseconds, and the fuzzer needs a key, not a fresh one per input.
fn file_key() -> &'static FileKey {
    static KEY: OnceLock<FileKey> = OnceLock::new();
    KEY.get_or_init(|| {
        match FileKey::from_passphrase(
            &Passphrase::new("passphrase".to_owned()),
            &FixedBytes::<FileSalt>::from_array([0x33; 16]),
        ) {
            Ok(key) => key,
            Err(e) => panic!("a fixed salt is always accepted: {e:?}"),
        }
    })
}

fuzz_target!(|data: &[u8]| {
    let _ = ambre_sync::fuzz::open_relayed_update(data);
    let _ = ambre_sync::fuzz::open_stored_snapshot(data);
    let _ = ambre_sync::fuzz::open_authenticated(data);
    if let Ok(file) =
        ambre_core::docfile::parse_sealed_header(&Payload::<SealedContainer>::new(data.to_vec()))
    {
        let _ =
            ambre_crypto::aead::open_file(&file.nonce, &file.ciphertext, &file.aad(), file_key());
    }
});
