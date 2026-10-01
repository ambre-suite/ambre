//! Protocol version byte for network messages.
//!
//! Defines the current version of the wire format and provides helpers for
//! version-aware serialization/deserialization.

use ambre_core::version::Version;

/// Current version of the wire protocol.
///
/// This is the version byte that must be prepended to every postcard-serialized
/// network message, both a WebSocket frame and a REST payload.
pub const CURRENT_VERSION: Version = Version::V3;

/// Checks that the provided version is supported.
///
/// Returns `Ok(())` if the version is `CURRENT_VERSION`, otherwise returns an
/// error appropriate for the situation (downgrade/upgrade).
///
/// # Errors
///
/// [`VersionError::UnsupportedVersion`] on any version other than
/// [`CURRENT_VERSION`], carrying the version it read.
pub fn check_version(version: Version) -> Result<(), VersionError> {
    if version == CURRENT_VERSION {
        Ok(())
    } else {
        Err(VersionError::UnsupportedVersion(version))
    }
}

/// Errors related to version checking.
#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum VersionError {
    /// The encountered version is not supported by this client/server.
    #[error("unsupported wire protocol version: {0}")]
    UnsupportedVersion(Version),
}
