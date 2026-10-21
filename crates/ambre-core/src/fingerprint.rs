//! The short, readable digest of a public key a user checks out of band.

use serde::{Deserialize, Serialize};

/// The fingerprint of a public key, grouped for reading aloud
/// (`ambre-spec-key-verification.md`). A distinct type, so a display name or
/// the key's own hex never takes its place in the trust check.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct KeyFingerprint(pub String);

impl KeyFingerprint {
    /// The fingerprint, as the trust dialog shows it.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
