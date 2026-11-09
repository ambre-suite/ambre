//! The admission of a participant to the SFU (`ambre-spec-call-media.md` section 3, `ZK-023`).
//!
//! The relay checks the channel and the cap, then signs an admission that names the call and
//! the participant by two pseudonyms. The SFU verifies the signature with the relay's public
//! key alone. It holds no key that mints an admission, and learns no channel, user or key.

use ambre_core::{CallAdmission, Credential, FixedBytes, Instant, PublicKey, UnixSec};
use ambre_protocol::signaling::Admission;
use ed25519_dalek::{Signature, VerifyingKey};

use crate::secrets;

/// Why the SFU refuses an admission. The socket closes, and the refusal names nothing of the
/// token.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum AdmissionRefusal {
    /// The token spells no admission.
    #[error("the token holds no admission")]
    Malformed,
    /// The relay's signature does not verify.
    #[error("the admission is not signed by the relay")]
    Forged,
    /// The admission expired.
    #[error("the admission expired")]
    Expired,
}

/// The public key of the relay, which signs every admission: `SFU_ADMISSION_KEY`, in hex.
#[derive(Clone, Copy, Debug)]
pub struct AdmissionKey(VerifyingKey);

impl AdmissionKey {
    /// The key whose public bytes are `key`.
    ///
    /// # Errors
    ///
    /// The error of `ed25519-dalek` when the bytes are no point of the curve.
    pub fn new(key: &FixedBytes<PublicKey>) -> Result<Self, ed25519_dalek::SignatureError> {
        VerifyingKey::from_bytes(key.as_array()).map(Self)
    }

    /// The admission `token` carries, once its signature verifies and it has not expired at
    /// `now`.
    ///
    /// # Errors
    ///
    /// [`AdmissionRefusal`] for a token that is malformed, forged or expired.
    pub fn admit(
        &self,
        token: &Credential<CallAdmission>,
        now: Instant<UnixSec>,
    ) -> Result<Admission, AdmissionRefusal> {
        let (admission, signature) =
            secrets::read_admission(token).map_err(|_| AdmissionRefusal::Malformed)?;
        let signed = admission
            .signed_bytes()
            .map_err(|_| AdmissionRefusal::Malformed)?;
        self.0
            .verify_strict(
                signed.as_bytes(),
                &Signature::from_bytes(signature.as_array()),
            )
            .map_err(|_| AdmissionRefusal::Forged)?;
        if admission.expires < now {
            return Err(AdmissionRefusal::Expired);
        }
        Ok(admission)
    }
}

/// A token signed by `key`, as the relay mints one, for a test.
#[cfg(test)]
pub(crate) fn signed(
    key: &ed25519_dalek::SigningKey,
    admission: &Admission,
) -> Credential<CallAdmission> {
    use ed25519_dalek::Signer;
    let signed = admission.signed_bytes().expect("an admission encodes");
    let signature = key.sign(signed.as_bytes()).to_bytes();
    admission
        .token(&FixedBytes::from_array(signature))
        .expect("a token encodes")
}
