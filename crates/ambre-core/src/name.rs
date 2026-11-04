//! The longest name a document, a channel, a space or a category takes.
//!
//! One cap for every name a member types, the cap of a display name too. The
//! relay refuses a sealed name past it (`SRV-230`). Every client surface
//! refuses one before it is written or sealed (`SYNC-178`, and the Murmure
//! channels spec). The rule [`Name`] admits only a fitting text.

use crate::{Rule, Validated};

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

/// The rule of a name a member typed for a document, a channel, a space or a
/// category: at most [`NAME_MAX_CHARS`] characters, empty allowed. A
/// `Validated<Name>` holds one. It travels as its text, and a read past the
/// cap is refused. A document's name is sealed for the relay, so `Debug`
/// prints the length alone, and a log carries none of the text.
pub enum Name {}

impl Rule for Name {
    type Error = NameTooLong;

    fn admit(text: String) -> Result<String, NameTooLong> {
        if name_fits(&text) {
            Ok(text)
        } else {
            Err(NameTooLong)
        }
    }
}

/// The empty name, which fits the cap.
impl Default for Validated<Name> {
    fn default() -> Self {
        Self::admitted(String::new())
    }
}

/// The name `text` gives once cut to its first [`NAME_MAX_CHARS`]
/// characters. It serves a name composed from other names, which no member
/// typed and which may run past the cap.
#[must_use]
pub fn cut_name_to_fit(text: &str) -> Validated<Name> {
    Validated::admitted(text.chars().take(NAME_MAX_CHARS).collect())
}

/// Whether `name` is the empty name, which a member may leave a document
/// with.
#[must_use]
pub fn name_is_empty(name: &Validated<Name>) -> bool {
    name.as_str().is_empty()
}

/// `name` without its leading and trailing blanks. A trimmed name is never
/// longer, so it always fits the cap.
#[must_use]
pub fn trimmed_name(name: &Validated<Name>) -> Validated<Name> {
    Validated::admitted(name.as_str().trim().to_owned())
}

/// The UTF-8 bytes of `name`, as the plaintext its seal takes. A document's
/// name is sealed for the relay.
#[must_use]
pub fn name_plaintext(name: &Validated<Name>) -> crate::Confidential<crate::Plaintext> {
    crate::Confidential::new(name.as_str().as_bytes().to_vec())
}

/// A text refused as a display name: blank, or past [`NAME_MAX_CHARS`]
/// characters once trimmed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("a display name is blank or longer than {NAME_MAX_CHARS} characters")]
pub struct InvalidDisplayName;

/// The rule of the name an account shows to other members, as its owner sets
/// it: trimmed, never blank, at most [`NAME_MAX_CHARS`] characters. A
/// `Validated<DisplayName>` holds one. It travels as its text, and a read of
/// one that breaks the rule is refused, the relay's row reads too. `Debug`
/// prints the length alone, so a log carries none of the text.
pub enum DisplayName {}

impl Rule for DisplayName {
    type Error = InvalidDisplayName;

    fn admit(text: String) -> Result<String, InvalidDisplayName> {
        let trimmed = text.trim();
        if trimmed.is_empty() || !name_fits(trimmed) {
            Err(InvalidDisplayName)
        } else if trimmed.len() == text.len() {
            Ok(text)
        } else {
            Ok(trimmed.to_owned())
        }
    }
}

/// A stored text read as a display name, refused as a parse refuses one.
/// The relay's row reads go through it.
impl TryFrom<String> for Validated<DisplayName> {
    type Error = InvalidDisplayName;

    fn try_from(text: String) -> Result<Self, Self::Error> {
        Self::parse(text)
    }
}

/// The display name `text` gives once trimmed and cut to its first
/// [`NAME_MAX_CHARS`] characters. It serves a name the user did not type
/// here, such as an identity provider's. `None` when only blanks remain.
#[must_use]
pub fn cut_display_name_to_fit(text: &str) -> Option<Validated<DisplayName>> {
    let cut: String = text.trim().chars().take(NAME_MAX_CHARS).collect();
    Validated::parse(cut).ok()
}

/// The short handle a member who carries no usable name shows: the first
/// four bytes of `user` in hex. It always fits the rule.
#[must_use]
pub fn display_handle(user: &crate::Id<crate::id::User>) -> Validated<DisplayName> {
    let handle: String = user
        .as_bytes()
        .iter()
        .take(4)
        .map(|b| format!("{b:02x}"))
        .collect();
    Validated::admitted(handle)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A name's `Debug` form holds none of its text, so a log carries none.
    #[test]
    fn a_name_prints_its_length_and_none_of_its_text() {
        let name = Validated::<Name>::parse("Ada's plan").unwrap();
        assert_eq!(format!("{name:?}"), "10 chars");
        assert_eq!(format!("{:?}", Some(&name)), "Some(10 chars)");
    }

    /// A display name's `Debug` form holds none of its text either.
    #[test]
    fn a_display_name_prints_its_length_and_none_of_its_text() {
        let name = Validated::<DisplayName>::parse(" Ada Lovelace ").unwrap();
        assert_eq!(format!("{name:?}"), "12 chars");
    }

    #[test]
    fn a_name_past_the_cap_is_cut_to_it() {
        let long = format!("  {}b  ", "\u{1F600}".repeat(NAME_MAX_CHARS));
        let cut = cut_display_name_to_fit(&long).unwrap();
        assert_eq!(cut.as_str(), "\u{1F600}".repeat(NAME_MAX_CHARS));
        assert_eq!(cut_display_name_to_fit(" ada ").unwrap().as_str(), "ada");
        assert_eq!(cut_display_name_to_fit("   "), None);
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
        let name = Validated::<Name>::parse(longest.clone()).unwrap();
        assert_eq!(
            serde_json::to_string(&name).unwrap(),
            format!("\"{longest}\"")
        );
        assert_eq!(
            Validated::<Name>::parse("a".repeat(NAME_MAX_CHARS + 1)),
            Err(NameTooLong)
        );
        let past = format!("\"{}\"", "a".repeat(NAME_MAX_CHARS + 1));
        assert!(serde_json::from_str::<Validated<Name>>(&past).is_err());
    }

    #[test]
    fn a_display_name_is_trimmed_and_never_blank() {
        assert_eq!(
            Validated::<DisplayName>::parse("  Ada ").unwrap().as_str(),
            "Ada"
        );
        assert_eq!(
            Validated::<DisplayName>::parse("   "),
            Err(InvalidDisplayName)
        );
        assert!(Validated::<DisplayName>::parse("a".repeat(NAME_MAX_CHARS + 1)).is_err());
        assert!(serde_json::from_str::<Validated<DisplayName>>("\"\"").is_err());
    }

    /// A total door builds only a text the rule admits: the empty name, a
    /// cut, a trim and a handle each parse again to themselves.
    #[test]
    fn a_total_door_builds_only_what_the_rule_admits() {
        let long = "a".repeat(NAME_MAX_CHARS + 9);
        for name in [
            Validated::<Name>::default(),
            cut_name_to_fit(&long),
            trimmed_name(&Validated::<Name>::parse("  plan ").unwrap()),
        ] {
            assert_eq!(Validated::<Name>::parse(name.as_str()), Ok(name.clone()));
        }
        let handle = display_handle(&crate::Id::<crate::id::User>::from_bytes([0xab; 16]));
        assert_eq!(handle.as_str(), "abababab");
        assert_eq!(
            Validated::<DisplayName>::parse(handle.as_str()),
            Ok(handle.clone())
        );
    }

    /// Both names travel as the bare string, in postcard and JSON alike.
    #[test]
    fn a_name_travels_as_its_bare_text() {
        let name = Validated::<Name>::parse("plan").unwrap();
        let shown = Validated::<DisplayName>::parse(" Ada ").unwrap();
        assert_eq!(serde_json::to_string(&name).unwrap(), "\"plan\"");
        assert_eq!(serde_json::to_string(&shown).unwrap(), "\"Ada\"");
        assert_eq!(
            postcard::to_allocvec(&name).unwrap(),
            postcard::to_allocvec("plan").unwrap()
        );
        assert_eq!(
            postcard::to_allocvec(&shown).unwrap(),
            postcard::to_allocvec("Ada").unwrap()
        );
    }
}
