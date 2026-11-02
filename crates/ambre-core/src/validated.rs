//! A text a rule checked once, at the door.
//!
//! A domain text with a rule composes a [`Validated`]: a name, a display
//! name, a defined name. The rule runs in [`Validated::parse`] and in the
//! decode, so no value that breaks it exists past the door. The domain type
//! keeps its own methods, and its marker names the rule:
//!
//! ```
//! use ambre_core::{Rule, Validated};
//! enum Shout {}
//! impl Rule for Shout {
//!     type Error = &'static str;
//!     fn admit(text: String) -> Result<String, Self::Error> {
//!         if text.chars().all(|c| !c.is_lowercase()) { Ok(text) } else { Err("not a shout") }
//!     }
//! }
//! assert!(Validated::<Shout>::parse("HEY").is_ok());
//! assert!(Validated::<Shout>::parse("hey").is_err());
//! ```
//!
//! The text is plaintext a member wrote, so it is held [`Redacted`]: `Debug`
//! prints the length alone. Serde sees the text itself.

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::cmp::Ordering;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::marker::PhantomData;

use crate::Redacted;

/// The rule of one kind of text.
///
/// The marker is an uninhabited type: it names the rule and holds no value.
/// The trait is open, because a rule lives in the crate of its domain type.
pub trait Rule: 'static {
    /// Why a text breaks the rule. A decode prints it.
    type Error: fmt::Display;

    /// The text the rule keeps for `text`, which it may normalize, as a trim
    /// does.
    ///
    /// # Errors
    ///
    /// [`Rule::Error`] when `text` breaks the rule.
    fn admit(text: String) -> Result<String, Self::Error>;
}

/// A text that the rule `V` admitted.
pub struct Validated<V: Rule> {
    text: Redacted<String>,
    rule: PhantomData<fn() -> V>,
}

impl<V: Rule> Validated<V> {
    /// The text the rule keeps for `text`.
    ///
    /// # Errors
    ///
    /// [`Rule::Error`] when `text` breaks the rule.
    pub fn parse(text: impl Into<String>) -> Result<Self, V::Error> {
        V::admit(text.into()).map(Self::admitted)
    }

    /// A text the caller built to fit the rule, such as a cut or a trim of an
    /// admitted one. The crate of the core types alone names it.
    pub(crate) const fn admitted(text: String) -> Self {
        Self {
            text: Redacted::new(text),
            rule: PhantomData,
        }
    }

    /// The text, for the one site that reads it.
    pub fn as_str(&self) -> &str {
        self.text.as_str()
    }

    /// The text, given up.
    pub fn into_string(self) -> String {
        self.text.into_inner()
    }
}

// By hand, every impl below: a derive would bound the marker, which has no
// value to clone, compare or hash.

impl<V: Rule> Clone for Validated<V> {
    fn clone(&self) -> Self {
        Self::admitted(self.as_str().to_owned())
    }
}

impl<V: Rule> PartialEq for Validated<V> {
    fn eq(&self, other: &Self) -> bool {
        self.text == other.text
    }
}

impl<V: Rule> Eq for Validated<V> {}

impl<V: Rule> PartialOrd for Validated<V> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl<V: Rule> Ord for Validated<V> {
    fn cmp(&self, other: &Self) -> Ordering {
        self.text.cmp(&other.text)
    }
}

impl<V: Rule> Hash for Validated<V> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.text.hash(state);
    }
}

/// `N chars`, as [`Redacted`] prints it.
impl<V: Rule> fmt::Debug for Validated<V> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&self.text, f)
    }
}

impl<V: Rule> Serialize for Validated<V> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

/// The text, refused at the door when it breaks the rule.
impl<'de, V: Rule> Deserialize<'de> for Validated<V> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Self::parse(String::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}
