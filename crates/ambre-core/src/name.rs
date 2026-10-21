//! The longest name a document, a channel, a space or a category takes.
//!
//! One cap for every name a member types, the cap of a display name too. The
//! relay refuses a sealed name past it (`SRV-230`). Every client surface
//! refuses one before it is written or sealed (`SYNC-178`, and the Murmure
//! channels spec).

/// The longest name, in characters (`ambre-spec-budgets.md`, section 6).
pub const NAME_MAX_CHARS: usize = 128;

/// What the user reads when a name is refused for its length. Every surface
/// shows the same words.
pub const NAME_TOO_LONG: &str = "A name takes 128 characters at most.";

/// Whether `name` is short enough to be taken.
pub fn name_fits(name: &str) -> bool {
    name.chars().count() <= NAME_MAX_CHARS
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
}
