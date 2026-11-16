//! The half of the `.ambre` container that the core itself names
//! (`ambre-spec-docfile.md`):
//!
//! - the space of the version byte;
//! - the width and the role of the salt of a sealed file;
//! - the role of the digest of a whole file.
//!
//! The reader and the writer of the container live in `ambre-client-core`,
//! which the relay does not link (`ARCH-117`). These markers stay here for two
//! reasons. [`FixedRole`] is sealed to this crate, so a role of another crate
//! does not compile. The fixture manifest of this crate also records a
//! container version and a file digest.

use crate::fixed_bytes::{DebugForm, FixedRole};
use crate::version::{Current, OpenRange, Version, VersionKind};

/// The bit of the version byte that partitions the two lineages: clear for a
/// plaintext container, set for a passphrase-sealed one.
///
/// The version byte changes, not a field beside it. A build that predates the
/// sealed lineage reads `0x80` as a container version it does not know, and
/// refuses the file. That is the whole of `DOCFILE-010`.
pub const SEALED_BIT: u8 = 0x80;

/// Argon2id salt width in a sealed header.
pub const SALT_LEN: usize = 16;

/// The role of the Argon2id salt of one sealed `.ambre`, fresh per file:
/// `FixedBytes<FileSalt>`. Public in the file, so not a secret; distinct from
/// the account's [`MasterKeySalt`](crate::MasterKeySalt), which no file ever
/// carries.
pub enum FileSalt {}

impl FixedRole for FileSalt {
    const NAME: crate::StaticKey<crate::DebugNames> = crate::StaticKey::new("FileSalt");
    const DEBUG: DebugForm = DebugForm::Length;
    type Array = [u8; SALT_LEN];
}

/// The space of the version byte of an `.ambre` container, offset 4.
///
/// Any byte reads as one: what the file carries is a fact about the file, and
/// the readers decide what to do with it. Its [`SEALED_BIT`] names the
/// lineage, so one value says both which lineage and which version
/// (`DOCFILE-010`). The order compares two versions of one lineage; the walk
/// of `ambre_client_core::migrate_ambre` reaches it only for a plaintext one.
pub enum Container {}

impl VersionKind for Container {
    type Repr = u8;
    const NAME: crate::StaticKey<crate::DebugNames> = crate::StaticKey::new("ContainerVersion");
}

impl OpenRange for Container {}

/// The container version this build writes, and the only one it parses
/// without migrating first: `ambre_client_core::AMBRE_VERSION`.
impl Current for Container {
    const CURRENT: Version<Self> = Version::new(0x01);
}

impl Version<Container> {
    /// Whether it names the sealed lineage: [`SEALED_BIT`] is set.
    #[must_use]
    pub const fn is_sealed(self) -> bool {
        self.get() & SEALED_BIT != 0
    }
}

/// The keyless digest of a whole `.ambre` file, header included, so a flipped
/// `document_id` byte changes it. The fixture sidecar records it to catch a
/// flipped byte. It is never a link of the snapshot chain. A readable format
/// sees it as hex text, the shape the sidecar always held. The role of
/// `FixedBytes<FileDigest>`.
pub enum FileDigest {}

impl FixedRole for FileDigest {
    const NAME: crate::StaticKey<crate::DebugNames> = crate::StaticKey::new("FileDigest");
    type Array = [u8; 32];
}
