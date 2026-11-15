//! The short, readable digest of a public key a user checks out of band.

use crate::{OpaqueKind, OpaqueLength, WireOpaque};

/// The kind of the fingerprint of a public key, grouped for reading aloud
/// (`ambre-spec-key-verification.md`): `Opaque<KeyFingerprints>`. A distinct
/// kind, so a display name or the key's own hex never takes its place in the
/// trust check. The trust dialog shows it through `as_str`. It encodes as
/// the bare string it replaced.
pub enum KeyFingerprints {}

impl OpaqueKind for KeyFingerprints {
    const NAME: crate::StaticKey<crate::DebugNames> = crate::StaticKey::new("KeyFingerprint");
    const LENGTH: OpaqueLength = OpaqueLength::Bytes;
}

impl WireOpaque for KeyFingerprints {}

/// The role of the digest of an inviter's signing key that an invite link
/// carries (`ambre-spec-membership.md section 3`): `FixedBytes<InviterPin>`.
/// The invitee holds it out of band. A first open then checks the log the
/// relay serves against a key the relay did not choose (`SYNC-205`). The
/// link spells it in hexadecimal, in its fragment, which no request carries.
pub enum InviterPin {}

impl crate::FixedRole for InviterPin {
    const NAME: crate::StaticKey<crate::DebugNames> = crate::StaticKey::new("InviterPin");
    type Array = [u8; 32];
}
