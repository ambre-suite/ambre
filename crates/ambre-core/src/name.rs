//! The longest name a document, a channel, a space or a category takes.
//!
//! One cap for every name a member types, the cap of a display name too. The
//! relay refuses a sealed name past it (`SRV-230`). Every client surface
//! refuses one before it is written or sealed (`SYNC-178`, and the Murmure
//! channels spec). It builds a [`Name`], which only a fitting text makes.

use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// The longest name, in characters (`ambre-spec-budgets.md`, section 6).
pub const NAME_MAX_CHARS: usize = 128;

/// What the user reads when a name is refused for its length. Every surface
/// shows the same words.
pub const NAME_TOO_LONG: &str = "A name takes 128 characters at most.";

/// Whether `name` is short enough to be taken.
pub fn name_fits(name: &str) -> bool {
    name.chars().count() <= NAME_MAX_CHARS
}

/// A text refused as a name: past [`NAME_MAX_CHARS`] characters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("a name is longer than {NAME_MAX_CHARS} characters")]
pub struct NameTooLong;

/// A name a member typed for a document, a channel, a space or a category:
/// at most [`NAME_MAX_CHARS`] characters, empty allowed. It travels as its
/// text, and a read past the cap is refused. A document's name is sealed
/// for the relay, so `Debug` prints the length alone, and a log carries none
/// of the text.
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub struct Name(String);

impl fmt::Debug for Name {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Name({} chars)", self.0.chars().count())
    }
}

impl Name {
    /// The name `text` is.
    ///
    /// # Errors
    ///
    /// [`NameTooLong`] when `text` runs past [`NAME_MAX_CHARS`] characters.
    pub fn new(text: impl Into<String>) -> Result<Self, NameTooLong> {
        let text = text.into();
        if name_fits(&text) {
            Ok(Self(text))
        } else {
            Err(NameTooLong)
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// The name `text` gives once cut to its first [`NAME_MAX_CHARS`]
    /// characters. It serves a name composed from other names, which no
    /// member typed and which may run past the cap.
    #[must_use]
    pub fn cut_to_fit(text: &str) -> Self {
        Self(text.chars().take(NAME_MAX_CHARS).collect())
    }

    /// The name without its leading and trailing blanks. A trimmed name is
    /// never longer, so it always fits the cap.
    #[must_use]
    pub fn trimmed(&self) -> Self {
        Self(self.0.trim().to_owned())
    }

    pub fn into_string(self) -> String {
        self.0
    }

    /// The UTF-8 bytes of the name, as the plaintext its seal takes. A
    /// document's name is sealed for the relay.
    #[must_use]
    pub fn to_plaintext(&self) -> crate::SecretBytes {
        crate::SecretBytes::new(self.0.as_bytes().to_vec())
    }
}

impl fmt::Display for Name {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl Serialize for Name {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for Name {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Self::new(String::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

/// A text refused as a display name: blank, or past [`NAME_MAX_CHARS`]
/// characters once trimmed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("a display name is blank or longer than {NAME_MAX_CHARS} characters")]
pub struct InvalidDisplayName;

/// The name an account shows to other members, as its owner sets it: trimmed,
/// never blank, at most [`NAME_MAX_CHARS`] characters. It travels as its text,
/// and a read of one that breaks the rule is refused. `Debug` prints the
/// length alone, so a log carries none of the text.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct DisplayName(String);

impl fmt::Debug for DisplayName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "DisplayName({} chars)", self.0.chars().count())
    }
}

impl DisplayName {
    /// The display name `text` is once trimmed.
    ///
    /// # Errors
    ///
    /// [`InvalidDisplayName`] when the trimmed text is blank or runs past
    /// [`NAME_MAX_CHARS`] characters.
    pub fn new(text: &str) -> Result<Self, InvalidDisplayName> {
        let text = text.trim();
        if text.is_empty() || !name_fits(text) {
            return Err(InvalidDisplayName);
        }
        Ok(Self(text.to_owned()))
    }

    /// The display name `text` gives once trimmed and cut to its first
    /// [`NAME_MAX_CHARS`] characters. It serves a name the user did not type
    /// here, such as an identity provider's. `None` when only blanks remain.
    #[must_use]
    pub fn cut_to_fit(text: &str) -> Option<Self> {
        let cut: String = text.trim().chars().take(NAME_MAX_CHARS).collect();
        Self::new(&cut).ok()
    }

    /// The short handle a member who carries no usable name shows: the first
    /// four bytes of `user` in hex. It always fits the rule.
    #[must_use]
    pub fn handle_of(user: &crate::UserId) -> Self {
        let handle: String = user
            .as_bytes()
            .iter()
            .take(4)
            .map(|b| format!("{b:02x}"))
            .collect();
        Self(handle)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_string(self) -> String {
        self.0
    }
}

/// A stored or received text read as a display name, refused as
/// [`DisplayName::new`] refuses one. The relay's row reads go through it.
impl TryFrom<String> for DisplayName {
    type Error = InvalidDisplayName;

    fn try_from(text: String) -> Result<Self, Self::Error> {
        Self::new(&text)
    }
}

impl fmt::Display for DisplayName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl Serialize for DisplayName {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for DisplayName {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        Self::new(&text).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A name's `Debug` form holds none of its text, so a log carries none.
    #[test]
    fn a_name_prints_its_length_and_none_of_its_text() {
        let name = Name::new("Ada's plan").unwrap();
        assert_eq!(format!("{name:?}"), "Name(10 chars)");
        assert_eq!(format!("{:?}", Some(&name)), "Some(Name(10 chars))");
    }

    /// A display name's `Debug` form holds none of its text either.
    #[test]
    fn a_display_name_prints_its_length_and_none_of_its_text() {
        let name = DisplayName::new(" Ada Lovelace ").unwrap();
        assert_eq!(format!("{name:?}"), "DisplayName(12 chars)");
    }

    #[test]
    fn a_name_past_the_cap_is_cut_to_it() {
        let long = format!("  {}b  ", "\u{1F600}".repeat(NAME_MAX_CHARS));
        let cut = DisplayName::cut_to_fit(&long).unwrap();
        assert_eq!(cut.as_str(), "\u{1F600}".repeat(NAME_MAX_CHARS));
        assert_eq!(DisplayName::cut_to_fit(" ada ").unwrap().as_str(), "ada");
        assert_eq!(DisplayName::cut_to_fit("   "), None);
    }

    #[test]
    fn a_name_fits_up_to_its_cap_in_characters() {
        assert!(name_fits(&"\u{1F600}".repeat(NAME_MAX_CHARS)));
        assert!(!name_fits(&"a".repeat(NAME_MAX_CHARS + 1)));
        assert!(NAME_TOO_LONG.contains(&NAME_MAX_CHARS.to_string()));
    }

    #[test]
    fn a_name_is_built_and_read_only_up_to_its_cap() {
        let longest = "\u{1F600}".repeat(NAME_MAX_CHARS);
        let name = Name::new(longest.clone()).unwrap();
        assert_eq!(
            serde_json::to_string(&name).unwrap(),
            format!("\"{longest}\"")
        );
        assert_eq!(Name::new("a".repeat(NAME_MAX_CHARS + 1)), Err(NameTooLong));
        let past = format!("\"{}\"", "a".repeat(NAME_MAX_CHARS + 1));
        assert!(serde_json::from_str::<Name>(&past).is_err());
    }

    #[test]
    fn a_display_name_is_trimmed_and_never_blank() {
        assert_eq!(DisplayName::new("  Ada ").unwrap().as_str(), "Ada");
        assert_eq!(DisplayName::new("   "), Err(InvalidDisplayName));
        assert!(DisplayName::new(&"a".repeat(NAME_MAX_CHARS + 1)).is_err());
        assert!(serde_json::from_str::<DisplayName>("\"\"").is_err());
    }
}
