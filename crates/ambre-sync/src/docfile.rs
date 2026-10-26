//! Passphrase-sealed `.ambre` files.
//!
//! Two service requests, and the only ones in the plane that never touch the
//! relay: [`RestCommand::SealDocFile`] seals a plaintext container, and
//! [`RestCommand::OpenDocFile`] opens it back. This crate holds them because
//! cryptography runs in the Sync worker and nowhere else (`ARCH-004`).
//!
//! Neither reads session state. A user without a session, or offline, still
//! seals and opens their own files, under this file's passphrase.
//!
//! Both handlers are synchronous: the Argon2id pass is pure computation, and it
//! holds this worker's event loop for a few hundred milliseconds.
use ambre_core::{parse_sealed_header, sealed_aad, write_ambre_sealed};
use ambre_crypto::keys::FileKey;

use super::*;

impl SyncState {
    /// Seals a whole plaintext container under a passphrase.
    ///
    /// Draws a fresh salt, derives the file key, and seals the plaintext under
    /// the sealed header's non-nonce prefix as associated data. No other salt
    /// and no other sealed layout can then carry this ciphertext.
    ///
    /// The ~64 MiB of the derivation is the largest transient this worker
    /// allocates, and the call releases it. The interface thread keeps painting
    /// throughout, which is why the work sits over here.
    pub(crate) fn handle_seal_doc_file(
        &self,
        id: RequestId,
        plaintext: SecretBytes,
        passphrase: ambre_core::Passphrase,
    ) {
        let Ok((key, salt)) = FileKey::for_new_file(&passphrase) else {
            tracing::warn!("Sync: SealDocFile: key derivation failed");
            self.reply(id, RestEvent::DocFileSealFailed);
            return;
        };
        match ambre_crypto::aead::seal_file(&plaintext, &sealed_aad(&salt), &key) {
            Ok((nonce, ciphertext)) => {
                let data = write_ambre_sealed(&salt, &nonce, &ciphertext);
                self.reply(id, RestEvent::DocFileSealed { data });
            }
            Err(_) => {
                tracing::warn!("Sync: SealDocFile: seal failed");
                self.reply(id, RestEvent::DocFileSealFailed);
            }
        }
    }

    /// Opens a sealed file, yielding the plaintext container it holds.
    ///
    /// The salt comes from the file, so each file has its own key. Every attempt
    /// pays the work factor, a wrong attempt included, which is what makes
    /// guessing expensive. Every failure answers the same way. Poly1305
    /// cannot tell a wrong passphrase from a tampered file, and the header is
    /// too thin to tell them apart earlier (`DOCFILE-016`).
    pub(crate) fn handle_open_doc_file(
        &self,
        id: RequestId,
        sealed: ambre_core::SealedContainer,
        passphrase: ambre_core::Passphrase,
    ) {
        let Ok(envelope) = parse_sealed_header(sealed.as_bytes()) else {
            tracing::warn!("Sync: OpenDocFile: not a sealed .ambre file");
            self.reply(id, RestEvent::DocFileOpenFailed);
            return;
        };
        let Ok(key) = FileKey::from_passphrase(&passphrase, &envelope.salt) else {
            tracing::warn!("Sync: OpenDocFile: key derivation failed");
            self.reply(id, RestEvent::DocFileOpenFailed);
            return;
        };
        match ambre_crypto::aead::open_file(
            &envelope.nonce,
            &envelope.ciphertext,
            &envelope.aad(),
            &key,
        ) {
            Ok(plaintext) => self.reply(id, RestEvent::DocFileOpened { plaintext }),
            Err(_) => {
                tracing::warn!("Sync: OpenDocFile: wrong passphrase or tampered file");
                self.reply(id, RestEvent::DocFileOpenFailed);
            }
        }
    }
}
