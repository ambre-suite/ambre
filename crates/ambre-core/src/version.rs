//! Version byte for serialization.
//!
//! A single byte that identifies the format version of a serialized data structure.
//! Used in postcard serialization (first byte of the payload) and in network messages.

use serde::{Deserialize, Serialize};
use std::fmt;

use crate::error::VersionError;

/// A version byte.
///
/// The value is restricted to the range `0..=127` to keep the high bit free for future extensions.
/// Version 0 is reserved for development/placeholder; version 1 is the first stable format.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Version(u8);

impl Version {
    /// Minimum allowed version (0).
    pub const MIN: Self = Self(0);

    /// Maximum allowed version (127).
    pub const MAX: Self = Self(127);

    /// Version 0 - development/placeholder.
    pub const V0: Self = Self(0);

    /// Version 1 - first stable format.
    pub const V1: Self = Self(1);

    /// Creates a version from a `u8`, returning an error if the value exceeds 127.
    ///
    /// # Errors
    ///
    /// [`VersionError::OutOfRange`] above [`Version::MAX`], carrying `value`.
    pub fn new(value: u8) -> Result<Self, VersionError> {
        if value <= Self::MAX.0 {
            Ok(Self(value))
        } else {
            Err(VersionError::OutOfRange(value))
        }
    }

    /// Returns the underlying version byte.
    pub const fn as_u8(self) -> u8 {
        self.0
    }

    /// Checks whether this version is compatible with `other` under a simple
    /// major-version equality rule (same major version).
    pub fn compatible_with(self, other: Self) -> bool {
        self.0 == other.0
    }
}

impl fmt::Debug for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Version({})", self.0)
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "v{}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new() {
        assert_eq!(Version::new(0).unwrap().as_u8(), 0);
        assert_eq!(Version::new(127).unwrap().as_u8(), 127);
        assert!(Version::new(128).is_err());
        assert!(Version::new(255).is_err());
    }

    #[test]
    fn test_constants() {
        assert_eq!(Version::MIN.as_u8(), 0);
        assert_eq!(Version::MAX.as_u8(), 127);
        assert_eq!(Version::V0.as_u8(), 0);
        assert_eq!(Version::V1.as_u8(), 1);
    }

    #[test]
    fn test_compatible_with() {
        let v1 = Version::new(1).unwrap();
        let v2 = Version::new(2).unwrap();
        assert!(v1.compatible_with(v1));
        assert!(!v1.compatible_with(v2));
    }

    #[test]
    fn test_serde() {
        let v = Version::new(42).unwrap();
        let serialized = serde_json::to_string(&v).unwrap();
        let deserialized: Version = serde_json::from_str(&serialized).unwrap();
        assert_eq!(v, deserialized);
    }
}
