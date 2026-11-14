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
//! prints the length alone. Serde sees the text itself. [`Rule::LOGGED_AS`]
//! alone decides what a log prints, through `Debug`. [`Displayed`] gives a
//! `Display` to a rule whose text the code renders back.

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

    /// The name `Debug` prints the text under, as `Name("text")`, when the
    /// text is safe in a log: a tag, a digest, a code word. It alone decides
    /// what a log prints. `None`, the default, prints the length alone, so a
    /// rule that sets nothing leaks nothing.
    const LOGGED_AS: Option<&'static str> = None;

    /// The text the rule keeps for `text`, which it may normalize, as a trim
    /// does.
    ///
    /// # Errors
    ///
    /// [`Rule::Error`] when `text` breaks the rule.
    fn admit(text: String) -> Result<String, Self::Error>;
}

/// A text that a person writes and reads, with no rule: an alternative text,
/// a caption. The marker alone tells it from another text of the same
/// owner, so two such texts never swap. Its rule admits every text as it is,
/// and its door [`Validated::new`] cannot fail.
pub trait Free: 'static {}

impl<V: Free> Rule for V {
    type Error = std::convert::Infallible;

    fn admit(text: String) -> Result<String, Self::Error> {
        Ok(text)
    }
}

/// The capability of a rule whose text the code renders back, such as the
/// output of a formula: a [`Validated`] of it has a `Display`. `Display` can
/// still reach a log through `%`. So a rule opts in only when a renderer must
/// write its text. A rule that does not opt in has no `to_string`.
///
/// ```compile_fail,E0277
/// use ambre_core::{Rule, Validated};
/// enum Secretish {}
/// impl Rule for Secretish {
///     type Error = &'static str;
///     fn admit(text: String) -> Result<String, Self::Error> { Ok(text) }
/// }
/// fn show(text: &Validated<Secretish>) -> String {
///     format!("{text}")
/// }
/// ```
pub trait Displayed: Rule {}

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

impl<V: Free> Validated<V> {
    /// The text, which a rule of no constraint admits as it is.
    pub fn new(text: impl Into<String>) -> Self {
        Self::admitted(text.into())
    }
}

/// The empty text, which a free rule admits.
impl<V: Free> Default for Validated<V> {
    fn default() -> Self {
        Self::new(String::new())
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

/// `N chars`, as [`Redacted`] prints it, unless the rule names the text
/// through [`Rule::LOGGED_AS`]: then `Name("text")`, as a derive prints a
/// newtype over a `String`.
impl<V: Rule> fmt::Debug for Validated<V> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match V::LOGGED_AS {
            Some(name) => f.debug_tuple(name).field(&self.as_str()).finish(),
            None => fmt::Debug::fmt(&self.text, f),
        }
    }
}

/// The text as admitted, for a rule a renderer writes, through [`Displayed`].
impl<V: Displayed> fmt::Display for Validated<V> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
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

#[cfg(test)]
mod tests {
    use super::{Displayed, Rule, Validated};

    /// A rule that sets nothing, as a text a member wrote takes.
    enum Masked {}

    impl Rule for Masked {
        type Error = &'static str;

        fn admit(text: String) -> Result<String, Self::Error> {
            if text.is_empty() {
                Err("empty")
            } else {
                Ok(text)
            }
        }
    }

    /// A text with no rule, as a caption takes.
    enum Note {}

    impl super::Free for Note {}

    #[test]
    fn a_free_text_admits_every_text_and_prints_its_length_alone() {
        let note = Validated::<Note>::new("  any text ");
        assert_eq!(note.as_str(), "  any text ");
        assert_eq!(
            format!("{note:?}"),
            format!("{:?}", crate::Redacted::new("  any text "))
        );
        assert!(Validated::<Note>::default().as_str().is_empty());
        let bytes = postcard::to_allocvec(&note).expect("a text encodes");
        assert_eq!(
            bytes,
            postcard::to_allocvec("  any text ").expect("a str encodes")
        );
        assert_eq!(postcard::from_bytes::<Validated<Note>>(&bytes), Ok(note));
    }

    /// A rule that opts in to a log, as a tag or a digest does.
    enum Named {}

    impl Rule for Named {
        type Error = &'static str;

        const LOGGED_AS: Option<&'static str> = Some("Named");

        fn admit(text: String) -> Result<String, Self::Error> {
            Ok(text.to_ascii_uppercase())
        }
    }

    impl Displayed for Named {}

    #[test]
    fn a_rule_that_sets_nothing_prints_the_length_alone() {
        let text = Validated::<Masked>::parse("secret").unwrap();
        assert_eq!(format!("{text:?}"), "6 chars");
        assert_eq!(format!("{:?}", Some(text)), "Some(6 chars)");
    }

    #[test]
    fn a_logged_rule_prints_the_text_under_its_name() {
        let text = Validated::<Named>::parse("sum").unwrap();
        assert_eq!(format!("{text:?}"), "Named(\"SUM\")");
        assert_eq!(text.to_string(), "SUM");
    }

    /// The serde form is the bare string in postcard and JSON alike. A decode
    /// goes through the rule: it refuses and normalises as a parse.
    #[test]
    fn serde_reads_and_writes_the_bare_string_through_the_rule() {
        let text = Validated::<Named>::parse("sum").unwrap();
        assert_eq!(serde_json::to_string(&text).unwrap(), "\"SUM\"");
        assert_eq!(
            postcard::to_allocvec(&text).unwrap(),
            postcard::to_allocvec("SUM").unwrap()
        );
        let read: Validated<Named> = serde_json::from_str("\"sum\"").unwrap();
        assert_eq!(read, text);
        assert!(serde_json::from_str::<Validated<Masked>>("\"\"").is_err());
        let bytes = postcard::to_allocvec("").unwrap();
        assert!(postcard::from_bytes::<Validated<Masked>>(&bytes).is_err());
    }
}
