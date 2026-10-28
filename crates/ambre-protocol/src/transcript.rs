//! The bytes a signature covers.
//!
//! A membership entry and a mismatch report are each signed over a
//! transcript: a domain, then the fields the signature binds. The signer and
//! the verifier build it from the same fields, and neither stores it. With a
//! bare `Vec<u8>`, any byte string could reach a `verify` call. This type
//! comes from the two builders alone, so what is verified is what was signed.

/// The bytes one signature covers, built by
/// [`crate::MembershipEntry::signed_bytes`] or
/// [`crate::snapshot_report::report_transcript`]. `Debug` prints the length.
#[derive(Clone, PartialEq, Eq)]
pub struct SignedTranscript(Vec<u8>);

impl SignedTranscript {
    /// The transcript made of `bytes`, as one of the two builders wrote them.
    pub(crate) fn new(bytes: Vec<u8>) -> Self {
        Self(bytes)
    }

    /// The bytes a signer signs and a verifier checks.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

impl std::fmt::Debug for SignedTranscript {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "SignedTranscript({} bytes)", self.0.len())
    }
}
