//! What the page makes of the user settings (`ambre-spec-user-settings.md`),
//! testable on the host.
//!
//! The boot reads the boot hint before the first frame, and every
//! `SettingsChanged` of the Logic thread moves the theme and the hint.

use ambre_canvas::Theme;
use ambre_client_core::{BootHint, ThemeChoice};

/// The theme a page shows before it reads any setting: the light set is
/// the unmarked stylesheet, but the shell boots the dark set today.
pub(crate) const UNSET_THEME: Theme = Theme::Dark;

/// The theme `choice` draws, `current` standing for the choice that follows
/// the system, which this page does not read yet.
pub(crate) const fn rendered(choice: ThemeChoice, current: Theme) -> Theme {
    match choice {
        ThemeChoice::Light => Theme::Light,
        ThemeChoice::Dark => Theme::Dark,
        ThemeChoice::System => current,
    }
}

/// The theme a page boots in: the one the boot hint names, or the unset
/// theme without a hint (`SETTINGS-032`).
pub(crate) fn boot_theme(hint: Option<&BootHint>) -> Theme {
    hint.map_or(UNSET_THEME, |hint| rendered(hint.theme, UNSET_THEME))
}

/// The choice a click on the theme switch writes: the theme it switched to.
pub(crate) const fn chosen(theme: Theme) -> ThemeChoice {
    match theme {
        Theme::Light => ThemeChoice::Light,
        Theme::Dark => ThemeChoice::Dark,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ambre_client_core::DocumentLocale;

    fn hint(theme: ThemeChoice) -> BootHint {
        BootHint {
            theme,
            interface_language: DocumentLocale::NONE,
        }
    }

    /// A page with a boot hint boots in the theme it names, and a page with
    /// none in the unset theme.
    #[test]
    fn a_page_boots_in_the_theme_of_its_boot_hint() {
        assert_eq!(boot_theme(Some(&hint(ThemeChoice::Light))), Theme::Light);
        assert_eq!(boot_theme(Some(&hint(ThemeChoice::Dark))), Theme::Dark);
        assert_eq!(boot_theme(Some(&hint(ThemeChoice::System))), UNSET_THEME);
        assert_eq!(boot_theme(None), UNSET_THEME);
    }

    /// A switch writes the theme it shows, and a choice draws the theme it
    /// names whatever the page shows.
    #[test]
    fn a_switch_writes_the_theme_it_shows() {
        for theme in [Theme::Light, Theme::Dark] {
            assert_eq!(rendered(chosen(theme), Theme::Dark), theme);
            assert_eq!(rendered(chosen(theme), Theme::Light), theme);
        }
        assert_eq!(rendered(ThemeChoice::System, Theme::Light), Theme::Light);
    }
}
