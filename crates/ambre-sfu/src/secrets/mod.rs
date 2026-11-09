//! The one module of the SFU that reads a secret (`ARCH-045`, `CALL-014`).
//!
//! The SFU holds two kinds of secret:
//!
//!   * the TURN secret, which it shares with the TURN server alone and mints each credential
//!     from. The relay never holds it (`CALL-014`);
//!   * the admission a participant presents, a bearer token of the relay, which it verifies.
//!
//! Each sits in an [`ambre_core::Secret`]. The primitive gate admits a read of one here, and
//! nowhere else in the SFU. Each function below hands the text to one foreign call: the HMAC
//! of the TURN credential, or the hex decoder of the admission.

use ambre_core::{CallAdmission, Credential, FixedBytes, Payload, Secret, Signature, TurnPassword};
use ambre_protocol::signaling::{Admission, AdmissionDecodeError};
use base64::Engine;
use hmac::{Hmac, Mac};
use sha1::Sha1;

use crate::turn::TurnUsername;

/// The secret the SFU and the TURN server share: `SFU_TURN_SECRET`, which coturn reads as its
/// `static-auth-secret`. `Debug` prints no character of it.
#[derive(Debug)]
pub struct TurnSecret(Secret<String>);

impl TurnSecret {
    /// Wraps the text, as `std::env::var` returns it.
    pub fn new(text: String) -> Self {
        Self(Secret::new(text))
    }

    /// The password of the TURN credential `username` names. It is the HMAC-SHA1 of the
    /// username under the secret, in base64, as the REST API of coturn checks it.
    pub fn mint(&self, username: &TurnUsername) -> Credential<TurnPassword> {
        // HMAC takes a key of any length, so the construction cannot fail.
        let Ok(mut mac) = Hmac::<Sha1>::new_from_slice(self.0.expose().as_bytes()) else {
            return Credential::new(String::new());
        };
        mac.update(username.to_string().as_bytes());
        let tag = mac.finalize().into_bytes();
        Credential::new(base64::engine::general_purpose::STANDARD.encode(tag))
    }
}

/// The admission and the relay's signature a token carries. Nothing is verified here: the
/// caller checks the signature against the relay's key.
///
/// # Errors
///
/// [`AdmissionDecodeError::Malformed`] when the text is no hex, and the error of
/// [`Admission::read`] when the bytes spell no token.
pub fn read_admission(
    token: &Credential<CallAdmission>,
) -> Result<(Admission, FixedBytes<Signature>), AdmissionDecodeError> {
    let bytes =
        hex::decode(token.secret().expose()).map_err(|_| AdmissionDecodeError::Malformed)?;
    Admission::read(&Payload::new(bytes))
}

#[cfg(test)]
mod tests;
