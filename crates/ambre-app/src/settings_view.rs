//! What the page makes of the user settings (`ambre-spec-user-settings.md`),
//! testable on the host.
//!
//! The boot reads the boot hint before the first frame, and every
//! `SettingsChanged` of the Logic thread moves the theme and the hint.

use ambre_canvas::Theme;
use ambre_client_core::{BootHint, ThemeChoice};

/// The theme `choice` draws, `system` being the theme the system asks for.
pub(crate) const fn rendered(choice: ThemeChoice, system: Theme) -> Theme {
    match choice {
        ThemeChoice::Light => Theme::Light,
        ThemeChoice::Dark => Theme::Dark,
        ThemeChoice::System => system,
    }
}

/// The theme a page boots in: the one the boot hint names, or the theme the
/// system asks for without one (`SETTINGS-032`, `UI-THREAD-065`). The boot
/// reads no other store (`UI-THREAD-066`).
pub(crate) fn boot_theme(hint: Option<&BootHint>, system: Theme) -> Theme {
    hint.map_or(system, |hint| rendered(hint.theme, system))
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

    /// `UI-THREAD-065`: a page boots in the theme its boot hint names. With
    /// no hint, or a hint at `system`, it boots in the system's theme.
    #[test]
    fn a_page_boots_in_the_theme_of_its_boot_hint() {
        for system in [Theme::Light, Theme::Dark] {
            assert_eq!(
                boot_theme(Some(&hint(ThemeChoice::Light)), system),
                Theme::Light
            );
            assert_eq!(
                boot_theme(Some(&hint(ThemeChoice::Dark)), system),
                Theme::Dark
            );
            assert_eq!(boot_theme(Some(&hint(ThemeChoice::System)), system), system);
            assert_eq!(boot_theme(None, system), system);
        }
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
