//! Format and schema versions.
//!
//! Every version is one generic type, [`Version<K>`], over a marker `K` that
//! names its space. A container version never compares to a schema version,
//! and a protocol byte never stands in for either:
//!
//! ```compile_fail,E0308
//! use ambre_core::Container;
//! use ambre_core::version::{Current, Protocol, Version};
//! let _: Version<Container> = Version::<Protocol>::CURRENT;
//! ```
//!
//! The marker carries what differs between two spaces:
//! * [`VersionKind::Repr`], the number the version holds: a byte or a `u32`;
//! * [`VersionKind::NAME`], the name `Debug` prints, so a log still reads
//!   `Version::<Schema>::new(3)`;
//! * [`Current::CURRENT`], for a space with one version this build writes;
//! * [`OpenRange`], for a space where every number is a version. A space
//!   with a rule has its own fallible constructor instead.
//!
//! The wire form is the one of the representation: `Version<K>` is
//! `#[serde(transparent)]`.

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::marker::PhantomData;

use crate::error::VersionError;

mod sealed {
    /// Closes [`super::VersionRepr`]: a version is a byte or a `u32`.
    pub trait Sealed {}
}

/// The number a version holds. Sealed: a `u8` or a `u32`.
pub trait VersionRepr:
    sealed::Sealed
    + Copy
    + Ord
    + Hash
    + fmt::Debug
    + fmt::Display
    + fmt::LowerHex
    + Serialize
    + DeserializeOwned
    + 'static
{
    /// The number after this one, held at the top.
    #[must_use]
    fn successor(self) -> Self;
}

impl sealed::Sealed for u8 {}
impl VersionRepr for u8 {
    fn successor(self) -> Self {
        self.saturating_add(1)
    }
}

impl sealed::Sealed for u32 {}
impl VersionRepr for u32 {
    fn successor(self) -> Self {
        self.saturating_add(1)
    }
}

/// The marker of one space of versions.
///
/// The marker is an uninhabited type: it names the space and holds no value.
/// The trait is open, because a space may live in the crate that reads it.
pub trait VersionKind: 'static {
    /// The number a version of the space holds.
    type Repr: VersionRepr;

    /// The name `Debug` prints, as the hand-written type printed its own.
    const NAME: crate::StaticKey<crate::DebugNames>;
}

/// A space where this build writes one version, and reads it without a
/// migration.
///
/// The schema version and the generator version have no such value. Each
/// application and each generator counts them on its own, beside the code
/// that bumps them.
pub trait Current: VersionKind + Sized {
    /// The version this build writes.
    const CURRENT: Version<Self>;
}

/// A space where every number of the representation is a version.
///
/// Only such a space has the infallible [`Version::new`]. A space with a rule
/// keeps a fallible constructor of its own.
pub trait OpenRange: VersionKind {}

/// One version of the space `K`.
#[derive(Serialize, Deserialize)]
#[serde(transparent, bound = "")]
pub struct Version<K: VersionKind> {
    raw: K::Repr,
    #[serde(skip)]
    kind: PhantomData<fn() -> K>,
}

impl<K: VersionKind> Version<K> {
    /// The version `raw`, for this module alone.
    const fn at(raw: K::Repr) -> Self {
        Self {
            raw,
            kind: PhantomData,
        }
    }

    /// The number the version holds, as its wire, store or register writes
    /// it.
    #[must_use]
    pub const fn get(self) -> K::Repr {
        self.raw
    }
}

impl<K: OpenRange> Version<K> {
    /// The version `raw`. Every number of the space is one.
    #[must_use]
    pub const fn new(raw: K::Repr) -> Self {
        Self::at(raw)
    }

    /// The version a bump after this one declares, held at the top.
    #[must_use]
    pub fn next(self) -> Self {
        Self::at(self.raw.successor())
    }
}

impl<K: Current> Version<K> {
    /// The version this build writes.
    pub const CURRENT: Self = K::CURRENT;
}

// By hand, every impl below: a derive would bound the marker, which has no
// value to clone, compare or hash.

impl<K: VersionKind> Clone for Version<K> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<K: VersionKind> Copy for Version<K> {}

impl<K: VersionKind> PartialEq for Version<K> {
    fn eq(&self, other: &Self) -> bool {
        self.raw == other.raw
    }
}

impl<K: VersionKind> Eq for Version<K> {}

impl<K: VersionKind> PartialOrd for Version<K> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl<K: VersionKind> Ord for Version<K> {
    fn cmp(&self, other: &Self) -> Ordering {
        self.raw.cmp(&other.raw)
    }
}

impl<K: VersionKind> Hash for Version<K> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.raw.hash(state);
    }
}

/// `NAME(n)`, as the hand-written type printed itself.
impl<K: VersionKind> fmt::Debug for Version<K> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple(K::NAME.as_str()).field(&self.raw).finish()
    }
}

/// The bare number.
impl<K: VersionKind> fmt::Display for Version<K> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.raw, f)
    }
}

impl<K: VersionKind> fmt::LowerHex for Version<K> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::LowerHex::fmt(&self.raw, f)
    }
}

/// The version byte of the wire protocol, the first byte of a postcard
/// payload.
///
/// The value is restricted to the range `0..=127` to keep the high bit free
/// for future extensions. Version 0 is reserved for development; version 1 is
/// the first stable format.
pub enum Protocol {}

impl VersionKind for Protocol {
    type Repr = u8;
    const NAME: crate::StaticKey<crate::DebugNames> = crate::StaticKey::new("Version");
}

impl Current for Protocol {
    const CURRENT: Version<Self> = Version::V3;
}

impl Version<Protocol> {
    /// Minimum allowed version (0).
    pub const MIN: Self = Self::at(0);

    /// Maximum allowed version (127).
    pub const MAX: Self = Self::at(127);

    /// Version 0 - development/placeholder.
    pub const V0: Self = Self::at(0);

    /// Version 1 - first stable format.
    pub const V1: Self = Self::at(1);

    /// Version 2 - an update carries the log head its author read
    /// (`SYNC-206`).
    pub const V2: Self = Self::at(2);

    /// Version 3 - an update carries the log entry its head names
    /// (`SYNC-218`).
    pub const V3: Self = Self::at(3);

    /// Creates a version from a `u8`, returning an error if the value exceeds
    /// 127.
    ///
    /// # Errors
    ///
    /// [`VersionError::OutOfRange`] above [`Self::MAX`], carrying `value`.
    pub const fn try_new(value: u8) -> Result<Self, VersionError> {
        if value <= Self::MAX.raw {
            Ok(Self::at(value))
        } else {
            Err(VersionError::OutOfRange(value))
        }
    }

    /// Checks whether this version is compatible with `other` under a simple
    /// major-version equality rule (same major version).
    #[must_use]
    pub fn compatible_with(self, other: Self) -> bool {
        self == other
    }
}

#[cfg(test)]
mod tests {
    use super::{Protocol, Version};

    #[test]
    fn test_new() {
        assert_eq!(Version::<Protocol>::try_new(0).unwrap().get(), 0);
        assert_eq!(Version::<Protocol>::try_new(127).unwrap().get(), 127);
        assert!(Version::<Protocol>::try_new(128).is_err());
        assert!(Version::<Protocol>::try_new(255).is_err());
    }

    #[test]
    fn test_constants() {
        assert_eq!(Version::<Protocol>::MIN.get(), 0);
        assert_eq!(Version::<Protocol>::MAX.get(), 127);
        assert_eq!(Version::<Protocol>::V0.get(), 0);
        assert_eq!(Version::<Protocol>::V1.get(), 1);
        assert_eq!(Version::<Protocol>::CURRENT, Version::V3);
    }

    #[test]
    fn test_compatible_with() {
        let v1 = Version::<Protocol>::try_new(1).unwrap();
        let v2 = Version::<Protocol>::try_new(2).unwrap();
        assert!(v1.compatible_with(v1));
        assert!(!v1.compatible_with(v2));
    }

    #[test]
    fn test_serde() {
        let v = Version::<Protocol>::try_new(42).unwrap();
        let serialized = serde_json::to_string(&v).unwrap();
        assert_eq!(serialized, "42");
        let deserialized: Version<Protocol> = serde_json::from_str(&serialized).unwrap();
        assert_eq!(v, deserialized);
        assert_eq!(format!("{v:?}"), "Version(42)");
    }
}
