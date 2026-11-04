//! The hex text of the access-token signing seed, as the relay reads it.

use ambre_core::Secret;
use ed25519_dalek::SigningKey;

use crate::jwt::JwtSeedRefusal;

/// The text of `JWT_SIGNING_SEED`: 64 hex characters, which are the seed
/// too. It holds the text in a [`Secret`], so the text is wiped when it
/// drops. `Debug` prints no character of it.
#[derive(Debug)]
pub struct SeedText(Secret<String>);

impl SeedText {
    /// Wraps the text, as `std::env::var` returns it.
    pub fn new(text: String) -> Self {
        Self(Secret::new(text))
    }

    /// The signing key the text names. The text, trimmed, decodes in place
    /// into the wrapper the key is built in. Named in `src/jwt.rs` alone
    /// (`ARCH-049`).
    ///
    /// # Errors
    ///
    /// [`JwtSeedRefusal::NotHex`] when the text is not hex, and
    /// [`JwtSeedRefusal::Length`] when it decodes to other than 32 bytes.
    /// Neither quotes the text.
    pub fn seed_key(&self) -> Result<SigningKey, JwtSeedRefusal> {
        let text = self.0.expose().trim();
        crate::jwt::signing_key(|seed| {
            hex::decode_to_slice(text, seed).map_err(|e| match e {
                hex::FromHexError::InvalidStringLength
                    if text.bytes().all(|b| b.is_ascii_hexdigit()) =>
                {
                    JwtSeedRefusal::Length(ambre_core::Count::<ambre_core::Octets>::of(
                        text.len() / 2,
                    ))
                }
                _ => JwtSeedRefusal::NotHex,
            })
        })
    }
}
