//! The avatar of a user with no picture: their initials on their identity
//! colour (the profile spec). Every application draws it through here, so one
//! user reads the same everywhere.
//!
//! The colour comes from the caller, derived from the user's identifier by the
//! shared presence helper. This crate takes no dependency on the draw layer.

use ambre_core::Rgb;
use leptos::prelude::*;

/// Up to two uppercase initials of a display name. Two words give the first
/// letter of each, and a single word its first two letters. A name with no
/// letter reads as a dash.
pub fn avatar_initials(name: &str) -> String {
    let mut words = name.split_whitespace();
    let letters: String = match (words.next(), words.next()) {
        (Some(first), Some(second)) => first
            .chars()
            .take(1)
            .chain(second.chars().take(1))
            .collect(),
        (Some(only), None) => only.chars().take(2).collect(),
        (None, _) => String::new(),
    };
    if letters.is_empty() {
        "-".to_owned()
    } else {
        letters.to_uppercase()
    }
}

/// A user's initials on their identity colour. The name is the title, so a
/// pointer reads it whole. A screen reader skips the initials, and reads the
/// name beside the avatar instead.
#[component]
pub fn Avatar(
    /// The display name the initials come from.
    #[prop(into)]
    name: Signal<ambre_core::DisplayName>,
    /// The colour of the user's identity.
    #[prop(into)]
    color: Signal<Rgb>,
) -> impl IntoView {
    view! {
        <span
            class="ambre-avatar"
            aria-hidden="true"
            title=move || name.get().into_string()
            style:background=move || color.get().to_string()
        >
            {move || name.with(|name| avatar_initials(name.as_str()))}
        </span>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two words give a letter each, one word its first two, and a name with
    /// no letter a dash (`PROFILE-002`).
    #[test]
    fn initials_come_from_the_first_two_words_or_letters() {
        assert_eq!(avatar_initials("Alice"), "AL");
        assert_eq!(avatar_initials("ada lovelace byron"), "AL");
        assert_eq!(avatar_initials("  Bob  Marley "), "BM");
        assert_eq!(avatar_initials("é"), "É");
        assert_eq!(avatar_initials("   "), "-");
        assert_eq!(avatar_initials(""), "-");
    }
}
