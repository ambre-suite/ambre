//! What the first key store of an account signs (`SRV-294`).
//!
//! The first key store registers a signing key and an exchange key on the
//! account of the session. A session alone does not prove that the caller holds
//! the signing key it names: an outsider could register the key of a member.
//! The client therefore signs the account and the two keys with that signing
//! key. The relay verifies the signature under the key the body names, before
//! it writes anything.

use ambre_core::id::{Id, User};
use ambre_core::{ExchangePublicKey, FixedBytes, PublicKey};

use crate::transcript::SignedTranscript;

/// The domain the transcript opens with, so the signature serves no other use.
pub const REGISTRATION_DOMAIN: &[u8] = b"ambre-key-registration-v1";

/// The bytes a first key store signs: the domain, the account, the signing key
/// and the exchange key.
///
/// Each part has a fixed width, so no two inputs share a transcript. The
/// account binds the proof to one account, so a proof read on the wire
/// registers the key nowhere else.
#[must_use]
pub fn registration_transcript(
    account: &Id<User>,
    signing: &FixedBytes<PublicKey>,
    exchange: &FixedBytes<ExchangePublicKey>,
) -> SignedTranscript {
    let account = account.as_bytes();
    let (signing, exchange) = (signing.as_slice(), exchange.as_slice());
    // The parts are bounded by their types. The sum cannot reach the bound of
    // a `usize`, so a saturation never shortens the buffer.
    let len = REGISTRATION_DOMAIN
        .len()
        .saturating_add(account.len())
        .saturating_add(signing.len())
        .saturating_add(exchange.len());
    let mut transcript = Vec::with_capacity(len);
    transcript.extend_from_slice(REGISTRATION_DOMAIN);
    transcript.extend_from_slice(account);
    transcript.extend_from_slice(signing);
    transcript.extend_from_slice(exchange);
    SignedTranscript::new(transcript)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn signing(byte: u8) -> FixedBytes<PublicKey> {
        FixedBytes::from_array([byte; 32])
    }

    fn exchange(byte: u8) -> FixedBytes<ExchangePublicKey> {
        FixedBytes::from_array([byte; 32])
    }

    /// `SRV-294`: the proof names the account and both keys. Changing any of
    /// the three changes what is signed, and the domain sets it apart from
    /// every other signature of the account.
    #[test]
    fn the_transcript_names_the_account_and_both_keys() {
        let account = Id::<User>::from_bytes([1; 16]);
        let base = registration_transcript(&account, &signing(2), &exchange(3));
        assert!(base.as_bytes().starts_with(REGISTRATION_DOMAIN));
        assert_ne!(
            base,
            registration_transcript(&Id::<User>::from_bytes([9; 16]), &signing(2), &exchange(3))
        );
        assert_ne!(
            base,
            registration_transcript(&account, &signing(4), &exchange(3))
        );
        assert_ne!(
            base,
            registration_transcript(&account, &signing(2), &exchange(4))
        );
        // The two keys sit at fixed offsets: swapping them is another
        // transcript.
        assert_ne!(
            base,
            registration_transcript(&account, &signing(3), &exchange(2))
        );
    }
}
