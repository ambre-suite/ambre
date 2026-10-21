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
/// text, and a read past the cap is refused.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub struct Name(String);

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

    pub fn into_string(self) -> String {
        self.0
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
/// and a read of one that breaks the rule is refused.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DisplayName(String);

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

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_string(self) -> String {
        self.0
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
