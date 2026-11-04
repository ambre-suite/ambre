//! The short, readable digest of a public key a user checks out of band.

use crate::{OpaqueKind, OpaqueLength, WireOpaque};

/// The kind of the fingerprint of a public key, grouped for reading aloud
/// (`ambre-spec-key-verification.md`): `Opaque<KeyFingerprints>`. A distinct
/// kind, so a display name or the key's own hex never takes its place in the
/// trust check. The trust dialog shows it through `as_str`. It encodes as
/// the bare string it replaced.
pub enum KeyFingerprints {}

impl OpaqueKind for KeyFingerprints {
    const NAME: &'static str = "KeyFingerprint";
    const LENGTH: OpaqueLength = OpaqueLength::Bytes;
}

impl WireOpaque for KeyFingerprints {}
