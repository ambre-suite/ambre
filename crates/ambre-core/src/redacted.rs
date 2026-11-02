//! Text that a log never prints.
//!
//! A type holding plaintext a member wrote composes a [`Redacted`]: a search,
//! a name, an address, a cell's text. Its `Debug` form is the length alone,
//! `N chars`. The derived `Debug` of a newtype over it reads `Name(12 chars)`,
//! and a log carries none of the text. Serde sees the text itself: the wire
//! and the store keep their bytes.

use serde::{Deserialize, Serialize};
use std::fmt;

/// A text whose `Debug` form is its length in code points.
///
/// It has no `Display` and no `Deref`: the text leaves it through
/// [`Redacted::as_str`] or [`Redacted::into_inner`] alone, at the one site
/// that reads it.
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Redacted<T>(T);

impl<T> Redacted<T> {
    /// Wraps `text`.
    pub const fn new(text: T) -> Self {
        Self(text)
    }

    /// The text, given up.
    pub fn into_inner(self) -> T {
        self.0
    }
}

impl<T: AsRef<str>> Redacted<T> {
    /// The text, for the one site that reads it.
    pub fn as_str(&self) -> &str {
        self.0.as_ref()
    }
}

/// `N chars`: the length alone, so a log carries none of the text.
impl<T: AsRef<str>> fmt::Debug for Redacted<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&crate::RedactedChars::of(self.0.as_ref()), f)
    }
}

#[cfg(test)]
mod tests {
    use super::Redacted;

    /// A newtype over a redacted text, as a domain type composes one.
    #[derive(Debug)]
    struct Secretish(Redacted<String>);

    #[test]
    fn debug_prints_the_length_and_serde_the_text() {
        let text = Redacted::new("h\u{e9}llo".to_owned());
        assert_eq!(format!("{text:?}"), "5 chars");
        let wrapped = Secretish(text.clone());
        assert_eq!(format!("{wrapped:?}"), "Secretish(5 chars)");
        assert_eq!(wrapped.0, text);
        assert_eq!(
            serde_json::to_string(&text).ok().as_deref(),
            Some("\"h\u{e9}llo\"")
        );
        assert_eq!(text.as_str(), "h\u{e9}llo");
    }
}
