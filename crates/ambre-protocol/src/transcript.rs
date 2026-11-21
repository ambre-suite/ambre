//! The bytes a signature covers.
//!
//! Each signed message of the protocol is signed over a transcript: a domain,
//! then the fields the signature binds. That covers a membership entry, a
//! mismatch report, a confirmation, a re-seal and a key store. The signer and the verifier build it from the same fields, and neither stores it. With a
//! bare `Vec<u8>`, any byte string could reach a `verify` call. This type
//! comes from the builders alone, so what is verified is what was signed.

/// The bytes one signature covers, built by
/// [`crate::MembershipEntry::signed_bytes`],
/// [`crate::snapshot_report::report_transcript`],
/// [`crate::snapshot_confirm::confirm_transcript`],
/// [`crate::master_rotation::rotation_transcript`] or
/// [`crate::key_registration::registration_transcript`]. `Debug` prints the length.
#[derive(Clone, PartialEq, Eq)]
pub struct SignedTranscript(Vec<u8>);

impl SignedTranscript {
    /// The transcript made of `bytes`, as one of the builders wrote them.
    pub(crate) fn new(bytes: Vec<u8>) -> Self {
        Self(bytes)
    }

    /// The bytes a signer signs and a verifier checks.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

/// The door the signer and the verifier of `ambre-crypto` take: the bytes the
/// builders wrote, unchanged.
impl ambre_core::Transcript for SignedTranscript {
    fn covered_bytes(&self) -> &[u8] {
        &self.0
    }
}

impl std::fmt::Debug for SignedTranscript {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "SignedTranscript({} bytes)", self.0.len())
    }
}
