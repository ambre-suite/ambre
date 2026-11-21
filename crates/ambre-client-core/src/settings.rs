//! The typed catalogue of the user settings (`ambre-spec-user-settings.md`).
//!
//! A setting is one entry of one section of the settings document: a frozen
//! key, a value type and a default. This module declares the shapes and the
//! catalogue. The document that stores them lives in the Logic thread, and
//! the relay never reads a setting (`ZK-053`).

use std::marker::PhantomData;

use ambre_core::{SettingKeys, StaticKey};

use crate::locale::{DocumentLocale, NumberConvention};

/// The part of the settings document one owner writes (`SETTINGS-003`).
///
/// Every application has a section, even one that holds no setting yet. A
/// section with no entry is an empty map. So a later setting adds a
/// declaration, and never a section.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum SettingsSection {
    /// The shell: the theme, the side and the fold of the context panel.
    Shell,
    /// The interface language, the reader's own locale and their convention.
    Locale,
    /// Prose.
    Prose,
    /// Trame.
    Trame,
    /// Toile.
    Toile,
    /// Murmure.
    Murmure,
    /// The macro approvals and secret values.
    Macros,
}

impl SettingsSection {
    /// Every section, in the order the document lists them.
    pub const ALL: [Self; 7] = [
        Self::Shell,
        Self::Locale,
        Self::Prose,
        Self::Trame,
        Self::Toile,
        Self::Murmure,
        Self::Macros,
    ];

    /// The frozen name of the section's map in the settings document.
    #[must_use]
    pub const fn word(self) -> &'static str {
        match self {
            Self::Shell => "shell",
            Self::Locale => "locale",
            Self::Prose => "prose",
            Self::Trame => "trame",
            Self::Toile => "toile",
            Self::Murmure => "murmure",
            Self::Macros => "macros",
        }
    }
}

/// The text a setting value takes in the settings document.
///
/// The document holds words, never a binary form. A value a later release
/// adds stays a word that an older client keeps unread (`SETTINGS-038`).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct SettingWord(String);

impl SettingWord {
    /// Wraps a stored text. Any text is a word; reading it is the value's
    /// job, and an unknown word reads as the default (`SETTINGS-021`).
    #[must_use]
    pub fn new(text: impl Into<String>) -> Self {
        Self(text.into())
    }

    /// The stored text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A value a setting may hold. `Default` is the bound that gives every
/// setting its default (`SETTINGS-019`).
pub trait SettingValue: Default + Clone + PartialEq {
    /// The word the settings document stores for this value.
    fn to_word(&self) -> SettingWord;

    /// The value a stored word spells, `None` for a word this release does
    /// not know.
    fn from_word(word: &SettingWord) -> Option<Self>;
}

/// One setting of the catalogue: its section, its frozen key and its value
/// type.
pub struct Setting<V> {
    section: SettingsSection,
    key: StaticKey<SettingKeys>,
    value: PhantomData<fn() -> V>,
}

impl<V> Clone for Setting<V> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<V> Copy for Setting<V> {}

impl<V> std::fmt::Debug for Setting<V> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Setting({}.{})", self.section.word(), self.key.as_str())
    }
}

impl<V: SettingValue> Setting<V> {
    /// Declares a setting. The key is frozen once released (`SETTINGS-020`).
    #[must_use]
    pub const fn new(section: SettingsSection, key: &'static str) -> Self {
        Self {
            section,
            key: StaticKey::new(key),
            value: PhantomData,
        }
    }

    /// The section that owns this setting.
    #[must_use]
    pub const fn section(&self) -> SettingsSection {
        self.section
    }

    /// The key of this setting inside its section.
    #[must_use]
    pub const fn key(&self) -> StaticKey<SettingKeys> {
        self.key
    }

    /// The value a stored word reads as: the default when the word is absent
    /// or unknown to this release (`SETTINGS-010`, `SETTINGS-021`).
    #[must_use]
    pub fn read(&self, stored: Option<&SettingWord>) -> V {
        stored.and_then(V::from_word).unwrap_or_default()
    }

    /// The section and the key, without the value type.
    #[must_use]
    pub const fn entry(&self) -> SettingEntry {
        SettingEntry {
            section: self.section,
            key: self.key,
        }
    }
}

/// Where a setting sits in the document, its value type left out.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SettingEntry {
    /// The owning section.
    pub section: SettingsSection,
    /// The key inside the section.
    pub key: StaticKey<SettingKeys>,
}

/// The theme a user chose. `System` follows the scheme of each device.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize,
)]
pub enum ThemeChoice {
    /// The theme each device's system asks for.
    #[default]
    System,
    /// The light theme on every device.
    Light,
    /// The dark theme on every device.
    Dark,
}

impl SettingValue for ThemeChoice {
    fn to_word(&self) -> SettingWord {
        SettingWord::new(match self {
            Self::System => "system",
            Self::Light => "light",
            Self::Dark => "dark",
        })
    }

    fn from_word(word: &SettingWord) -> Option<Self> {
        match word.as_str() {
            "system" => Some(Self::System),
            "light" => Some(Self::Light),
            "dark" => Some(Self::Dark),
            _ => None,
        }
    }
}

/// The side the context panel docks to.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize,
)]
pub enum PanelSide {
    /// The left edge.
    Left,
    /// The right edge, the default.
    #[default]
    Right,
}

impl SettingValue for PanelSide {
    fn to_word(&self) -> SettingWord {
        SettingWord::new(match self {
            Self::Left => "left",
            Self::Right => "right",
        })
    }

    fn from_word(word: &SettingWord) -> Option<Self> {
        match word.as_str() {
            "left" => Some(Self::Left),
            "right" => Some(Self::Right),
            _ => None,
        }
    }
}

/// Whether the context panel of one application is open or folded to its
/// rail.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize,
)]
pub enum PanelFold {
    /// The panel shows its sections, the default.
    #[default]
    Open,
    /// The panel shows its rail alone.
    Folded,
}

impl SettingValue for PanelFold {
    fn to_word(&self) -> SettingWord {
        SettingWord::new(match self {
            Self::Open => "open",
            Self::Folded => "folded",
        })
    }

    fn from_word(word: &SettingWord) -> Option<Self> {
        match word.as_str() {
            "open" => Some(Self::Open),
            "folded" => Some(Self::Folded),
            _ => None,
        }
    }
}

/// A locale as a setting: the empty word is no locale, and a malformed tag
/// is a word this release does not read.
impl SettingValue for DocumentLocale {
    fn to_word(&self) -> SettingWord {
        SettingWord::new(self.as_str().unwrap_or_default())
    }

    fn from_word(word: &SettingWord) -> Option<Self> {
        match word.as_str() {
            "" => Some(Self::NONE),
            text => Self::parse(text).ok(),
        }
    }
}

impl SettingValue for NumberConvention {
    fn to_word(&self) -> SettingWord {
        SettingWord::new(match self {
            Self::Document => "document",
            Self::TypeMine => "type-mine",
            Self::AllMine => "all-mine",
        })
    }

    fn from_word(word: &SettingWord) -> Option<Self> {
        match word.as_str() {
            "document" => Some(Self::Document),
            "type-mine" => Some(Self::TypeMine),
            "all-mine" => Some(Self::AllMine),
            _ => None,
        }
    }
}

/// An application that lays out a context panel.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum PanelApplication {
    /// Prose.
    Prose,
    /// Trame.
    Trame,
    /// Toile.
    Toile,
}

/// The settings of the catalogue. Each key is frozen once released
/// (`SETTINGS-020`): a test pins the list.
pub mod catalogue {
    use super::{
        DocumentLocale, NumberConvention, PanelApplication, PanelFold, PanelSide, Setting,
        SettingEntry, SettingsSection, ThemeChoice,
    };

    /// The theme (`SETTINGS-023`).
    pub const THEME: Setting<ThemeChoice> = Setting::new(SettingsSection::Shell, "theme");
    /// The side of the context panel, one for every application
    /// (`SETTINGS-025`).
    pub const PANEL_SIDE: Setting<PanelSide> = Setting::new(SettingsSection::Shell, "panel.side");
    /// The fold of the context panel in Prose (`SETTINGS-026`).
    pub const PANEL_FOLD_PROSE: Setting<PanelFold> =
        Setting::new(SettingsSection::Shell, "panel.fold.prose");
    /// The fold of the context panel in Trame (`SETTINGS-026`).
    pub const PANEL_FOLD_TRAME: Setting<PanelFold> =
        Setting::new(SettingsSection::Shell, "panel.fold.trame");
    /// The fold of the context panel in Toile (`SETTINGS-026`).
    pub const PANEL_FOLD_TOILE: Setting<PanelFold> =
        Setting::new(SettingsSection::Shell, "panel.fold.toile");
    /// The interface language; no locale means the browser's language
    /// (`SETTINGS-024`).
    pub const INTERFACE_LANGUAGE: Setting<DocumentLocale> =
        Setting::new(SettingsSection::Locale, "interface.language");
    /// The reader's own locale (`SETTINGS-027`).
    pub const READER_LOCALE: Setting<DocumentLocale> =
        Setting::new(SettingsSection::Locale, "reader.locale");
    /// The reader's number convention (`SETTINGS-027`).
    pub const NUMBER_CONVENTION: Setting<NumberConvention> =
        Setting::new(SettingsSection::Locale, "number.convention");

    /// The fold setting of one application's panel.
    #[must_use]
    pub const fn panel_fold(application: PanelApplication) -> Setting<PanelFold> {
        match application {
            PanelApplication::Prose => PANEL_FOLD_PROSE,
            PanelApplication::Trame => PANEL_FOLD_TRAME,
            PanelApplication::Toile => PANEL_FOLD_TOILE,
        }
    }

    /// Every setting of the catalogue, by place.
    pub const ALL: [SettingEntry; 8] = [
        THEME.entry(),
        PANEL_SIDE.entry(),
        PANEL_FOLD_PROSE.entry(),
        PANEL_FOLD_TRAME.entry(),
        PANEL_FOLD_TOILE.entry(),
        INTERFACE_LANGUAGE.entry(),
        READER_LOCALE.entry(),
        NUMBER_CONVENTION.entry(),
    ];
}

/// Every setting of the catalogue, read: what the Logic thread hands the
/// view whenever the settings change. A setting the document does not hold
/// reads as its default (`SETTINGS-010`).
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct UserSettings {
    /// The theme (`SETTINGS-023`).
    pub theme: ThemeChoice,
    /// The side of the context panel (`SETTINGS-025`).
    pub panel_side: PanelSide,
    /// The fold of the context panel in Prose (`SETTINGS-026`).
    pub panel_fold_prose: PanelFold,
    /// The fold of the context panel in Trame (`SETTINGS-026`).
    pub panel_fold_trame: PanelFold,
    /// The fold of the context panel in Toile (`SETTINGS-026`).
    pub panel_fold_toile: PanelFold,
    /// The interface language; none means the browser's (`SETTINGS-024`).
    pub interface_language: DocumentLocale,
    /// The reader's own locale (`SETTINGS-027`).
    pub reader_locale: DocumentLocale,
    /// The reader's number convention (`SETTINGS-027`).
    pub number_convention: NumberConvention,
}

impl UserSettings {
    /// The fold of one application's panel.
    #[must_use]
    pub const fn panel_fold(&self, application: PanelApplication) -> PanelFold {
        match application {
            PanelApplication::Prose => self.panel_fold_prose,
            PanelApplication::Trame => self.panel_fold_trame,
            PanelApplication::Toile => self.panel_fold_toile,
        }
    }

    /// The boot hint these settings give the device (`SETTINGS-029`).
    #[must_use]
    pub fn boot_hint(&self) -> BootHint {
        BootHint {
            theme: self.theme,
            interface_language: self.interface_language.clone(),
        }
    }

    /// Whether `change` moves the boot hint, which the device then writes
    /// again (`SETTINGS-037`).
    #[must_use]
    pub const fn moves_boot_hint(change: &SettingChange) -> bool {
        matches!(
            change,
            SettingChange::Theme(_) | SettingChange::InterfaceLanguage(_)
        )
    }
}

/// One change of one setting, as the view asks for it. The Logic thread
/// writes it into the settings document (`SETTINGS-012`).
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum SettingChange {
    /// The theme.
    Theme(ThemeChoice),
    /// The side of the context panel.
    PanelSide(PanelSide),
    /// The fold of one application's panel.
    PanelFold(PanelApplication, PanelFold),
    /// The interface language.
    InterfaceLanguage(DocumentLocale),
    /// The reader's own locale.
    ReaderLocale(DocumentLocale),
    /// The reader's number convention.
    NumberConvention(NumberConvention),
}

/// The two settings a page reads before the unlock, kept in the clear on the
/// device (`SETTINGS-028`). It mirrors the account and holds no choice of its
/// own.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct BootHint {
    /// The theme of the account.
    pub theme: ThemeChoice,
    /// The interface language of the account.
    pub interface_language: DocumentLocale,
}

/// The text a boot hint takes in the browser's local storage.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BootHintText(String);

/// The version word that opens a boot hint text. A hint of another version
/// reads as no hint, so the page boots as a fresh device does.
const BOOT_HINT_VERSION: &str = "v1";

impl BootHintText {
    /// Wraps a text read back from the storage.
    #[must_use]
    pub fn new(text: impl Into<String>) -> Self {
        Self(text.into())
    }

    /// The text to store.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl BootHint {
    /// The text this hint stores: the version, the theme word and the
    /// language word, joined by `;`.
    #[must_use]
    pub fn encode(&self) -> BootHintText {
        BootHintText(format!(
            "{BOOT_HINT_VERSION};{};{}",
            self.theme.to_word().as_str(),
            self.interface_language.to_word().as_str()
        ))
    }

    /// The hint a stored text spells, `None` for a text of another version
    /// or shape. An unknown theme or language reads as its default.
    #[must_use]
    pub fn decode(text: &BootHintText) -> Option<Self> {
        let mut parts = text.as_str().split(';');
        if parts.next()? != BOOT_HINT_VERSION {
            return None;
        }
        let theme = SettingWord::new(parts.next()?);
        let language = SettingWord::new(parts.next()?);
        if parts.next().is_some() {
            return None;
        }
        Some(Self {
            theme: catalogue::THEME.read(Some(&theme)),
            interface_language: catalogue::INTERFACE_LANGUAGE.read(Some(&language)),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::catalogue::{self, ALL};
    use super::*;

    /// The frozen register of `SETTINGS-020`. A key renamed, moved or
    /// dropped fails here: a new meaning takes a new key, appended.
    const FROZEN: [(&str, &str); 8] = [
        ("shell", "theme"),
        ("shell", "panel.side"),
        ("shell", "panel.fold.prose"),
        ("shell", "panel.fold.trame"),
        ("shell", "panel.fold.toile"),
        ("locale", "interface.language"),
        ("locale", "reader.locale"),
        ("locale", "number.convention"),
    ];

    #[test]
    fn every_released_key_keeps_its_section_and_its_name() {
        let live: Vec<(&str, &str)> = ALL
            .iter()
            .map(|entry| (entry.section.word(), entry.key.as_str()))
            .collect();
        assert_eq!(live, FROZEN);
    }

    #[test]
    fn each_setting_belongs_to_one_section_under_one_key() {
        let mut seen = std::collections::HashSet::new();
        for entry in ALL {
            assert!(seen.insert(entry), "{entry:?} is declared twice");
        }
        let words: std::collections::HashSet<&str> = SettingsSection::ALL
            .iter()
            .map(|section| section.word())
            .collect();
        assert_eq!(words.len(), SettingsSection::ALL.len());
    }

    #[test]
    fn an_absent_or_unknown_word_reads_as_the_default() {
        assert_eq!(catalogue::THEME.read(None), ThemeChoice::System);
        let later = SettingWord::new("high-contrast");
        assert_eq!(catalogue::THEME.read(Some(&later)), ThemeChoice::System);
        assert_eq!(catalogue::PANEL_SIDE.read(None), PanelSide::Right);
        assert_eq!(
            catalogue::PANEL_FOLD_TRAME.read(Some(&SettingWord::new("half"))),
            PanelFold::Open
        );
        assert_eq!(
            catalogue::INTERFACE_LANGUAGE.read(Some(&SettingWord::new("not a tag"))),
            DocumentLocale::NONE
        );
        assert_eq!(
            catalogue::NUMBER_CONVENTION.read(None),
            NumberConvention::TypeMine
        );
    }

    #[test]
    fn every_value_round_trips_through_its_word() {
        for theme in [ThemeChoice::System, ThemeChoice::Light, ThemeChoice::Dark] {
            assert_eq!(ThemeChoice::from_word(&theme.to_word()), Some(theme));
        }
        for side in [PanelSide::Left, PanelSide::Right] {
            assert_eq!(PanelSide::from_word(&side.to_word()), Some(side));
        }
        for fold in [PanelFold::Open, PanelFold::Folded] {
            assert_eq!(PanelFold::from_word(&fold.to_word()), Some(fold));
        }
        for convention in [
            NumberConvention::Document,
            NumberConvention::TypeMine,
            NumberConvention::AllMine,
        ] {
            assert_eq!(
                NumberConvention::from_word(&convention.to_word()),
                Some(convention)
            );
        }
        let french = DocumentLocale::parse("fr-CA").ok();
        for locale in [Some(DocumentLocale::NONE), french].into_iter().flatten() {
            assert_eq!(DocumentLocale::from_word(&locale.to_word()), Some(locale));
        }
    }

    #[test]
    fn a_boot_hint_round_trips_and_refuses_another_version() {
        let hint = BootHint {
            theme: ThemeChoice::Dark,
            interface_language: DocumentLocale::parse("fr").unwrap_or(DocumentLocale::NONE),
        };
        assert_eq!(BootHint::decode(&hint.encode()), Some(hint));
        assert_eq!(BootHint::decode(&BootHintText::new("v2;dark;fr")), None);
        assert_eq!(BootHint::decode(&BootHintText::new("v1;dark")), None);
        assert_eq!(BootHint::decode(&BootHintText::new("v1;dark;fr;x")), None);
        assert_eq!(
            BootHint::decode(&BootHintText::new("v1;sepia;")),
            Some(BootHint::default())
        );
    }

    #[test]
    fn the_panel_fold_of_each_application_is_its_own_setting() {
        let folds = [
            catalogue::panel_fold(PanelApplication::Prose).entry(),
            catalogue::panel_fold(PanelApplication::Trame).entry(),
            catalogue::panel_fold(PanelApplication::Toile).entry(),
        ];
        assert_ne!(folds[0], folds[1]);
        assert_ne!(folds[1], folds[2]);
        assert_ne!(folds[0], folds[2]);
    }
}
