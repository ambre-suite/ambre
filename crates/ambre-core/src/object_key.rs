//! The key the object store holds a blob under.

use serde::{Deserialize, Serialize};

/// The key of one object in the object store, as the relay names it to the
/// client that puts or gets it (`ambre-spec-object-store.md`). A distinct
/// type, so a content hash or a URL never takes its place.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ObjectKey(String);

impl ObjectKey {
    /// Wraps a raw value. The constructor checks nothing.
    pub const fn new(value: String) -> Self {
        Self(value)
    }

    /// The key text.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Returns the raw text, for a foreign call that takes it by value.
    pub fn into_inner(self) -> String {
        self.0
    }
}

impl From<&str> for ObjectKey {
    fn from(text: &str) -> Self {
        Self(text.to_owned())
    }
}

impl From<String> for ObjectKey {
    fn from(text: String) -> Self {
        Self(text)
    }
}

impl std::fmt::Display for ObjectKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// The key as the object store's client takes it.
impl From<&ObjectKey> for String {
    fn from(key: &ObjectKey) -> Self {
        key.0.clone()
    }
}
