//! The interface language of the page (`SETTINGS-024`, `SETTINGS-033`).
//!
//! The page holds one language at a time. The launcher sets it from the boot
//! hint before the first frame, and again from every settings change. Every
//! view of a label reads it through [`language`], which a reactive scope
//! tracks, so a change redraws each label that read it.

use std::cell::Cell;

use ambre_client_core::DocumentLocale;
use ambre_commands::Locale;
use leptos::prelude::{ArcTrigger, Notify, Track};

thread_local! {
    // The UI thread alone draws labels. A trigger beside a cell costs Main
    // less than a signal of `Locale`. A signal of a new type compiles its
    // own copy of the reactive code. The `Arc` trigger needs no reactive
    // owner, so an event handler reads it as a view does.
    static LANGUAGE: (Cell<Locale>, ArcTrigger) = (Cell::new(Locale::default()), ArcTrigger::new());
}

/// The interface language of the page. Inside a reactive scope, the scope
/// tracks it and runs again on a change.
#[must_use]
pub fn language() -> Locale {
    LANGUAGE.with(|(locale, changed)| {
        changed.track();
        locale.get()
    })
}

/// Make `locale` the interface language of the page.
pub fn set_language(locale: Locale) {
    LANGUAGE.with(|(shown, changed)| {
        if shown.replace(locale) != locale {
            changed.notify();
        }
    });
}

/// The language the page shows for a stored tag. A tag of none takes the
/// browser's language (`SETTINGS-024`). A French tag, of any region, shows
/// French; every other tag shows English, the language every label is
/// declared in.
#[must_use]
pub fn interface_locale(stored: &DocumentLocale, browser: &DocumentLocale) -> Locale {
    let tag = stored.as_str().or(browser.as_str()).unwrap_or_default();
    let primary = tag.split(['-', '_']).next().unwrap_or_default();
    if primary.eq_ignore_ascii_case("fr") {
        Locale::Fr
    } else {
        Locale::En
    }
}

/// The tag the settings store for `locale`: its bare language subtag.
#[must_use]
pub fn stored_tag(locale: Locale) -> DocumentLocale {
    let tag = match locale {
        Locale::En => "en",
        Locale::Fr => "fr",
    };
    DocumentLocale::parse(tag).unwrap_or(DocumentLocale::NONE)
}

/// The name of a language in that language, which a person reads in the
/// picker. `Validated<LanguageName>`: no rule bounds it.
pub enum LanguageName {}

impl ambre_core::Free for LanguageName {}

impl ambre_core::Displayed for LanguageName {}

/// The name of `locale` in its own language, as the picker lists it. A
/// reader finds their language whatever the interface shows.
#[must_use]
pub fn endonym(locale: Locale) -> ambre_core::Validated<LanguageName> {
    ambre_core::Validated::new(match locale {
        Locale::En => "English",
        Locale::Fr => "Fran\u{e7}ais",
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tag(text: &str) -> DocumentLocale {
        DocumentLocale::parse(text).expect("a tag")
    }

    #[test]
    fn a_french_tag_of_any_region_shows_french() {
        for text in ["fr", "fr-FR", "fr-CA", "FR-ch"] {
            assert_eq!(
                interface_locale(&tag(text), &DocumentLocale::NONE),
                Locale::Fr,
                "{text}"
            );
        }
    }

    #[test]
    fn every_other_tag_shows_english() {
        for text in ["en", "en-GB", "de", "es", "frr"] {
            assert_eq!(
                interface_locale(&tag(text), &DocumentLocale::NONE),
                Locale::En,
                "{text}"
            );
        }
    }

    #[test]
    fn a_tag_of_none_takes_the_browser_language() {
        let none = DocumentLocale::NONE;
        assert_eq!(interface_locale(&none, &tag("fr-BE")), Locale::Fr);
        assert_eq!(interface_locale(&none, &tag("en-US")), Locale::En);
        assert_eq!(interface_locale(&none, &none), Locale::En);
        // A stored choice wins over the browser.
        assert_eq!(interface_locale(&tag("en"), &tag("fr")), Locale::En);
    }

    #[test]
    fn the_stored_tag_of_each_language_reads_back_as_that_language() {
        for locale in Locale::ALL {
            assert_eq!(interface_locale(&stored_tag(locale), &tag("de")), locale);
        }
    }

    /// `SETTINGS-042`: a label read in a reactive scope reads the new
    /// language once the page changes it, with no other signal.
    #[test]
    fn a_label_read_in_a_reactive_scope_follows_a_new_language() {
        use leptos::prelude::{ArcMemo, Get};
        const BOLD: ambre_commands::TitleWords = ambre_commands::TitleWords {
            en: "Bold",
            fr: "Gras",
        };
        set_language(Locale::En);
        let label = ArcMemo::new(|_| BOLD.pick(&language()).into_string());
        assert_eq!(label.get(), "Bold");
        set_language(Locale::Fr);
        assert_eq!(label.get(), "Gras");
        set_language(Locale::En);
        assert_eq!(label.get(), "Bold");
    }

    #[test]
    fn the_page_language_reads_back_what_was_set() {
        set_language(Locale::Fr);
        assert_eq!(language(), Locale::Fr);
        set_language(Locale::En);
        assert_eq!(language(), Locale::En);
    }
}
