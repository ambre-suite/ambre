//! Document type enumeration.
//!
//! Each Ambre document belongs to one of these four types.
//! The numeric values are stable across serialization (postcard, network).

use crate::error::DocTypeError;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

/// Type of an Ambre document.
///
/// The variants are assigned explicit discriminants that are used in binary
/// serialization (postcard) and in the database.
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DocType {
    /// Prose - rich-text collaborative editor.
    Prose = 0,
    /// Trame - spreadsheet with formulas and real-time collaboration.
    Trame = 1,
    /// Toile - infinite canvas whiteboard and presentations.
    Toile = 2,
    /// Murmure - end-to-end encrypted messaging channels.
    Murmure = 3,
    /// The settings of one user: a document with one member, opened by no
    /// application (`ambre-spec-user-settings.md`).
    Settings = 4,
}

impl DocType {
    /// Converts a `u8` to a `DocType`, returning `None` if the value is invalid.
    pub fn from_u8(value: u8) -> Option<Self> {
        match value {
            0 => Some(Self::Prose),
            1 => Some(Self::Trame),
            2 => Some(Self::Toile),
            3 => Some(Self::Murmure),
            4 => Some(Self::Settings),
            _ => None,
        }
    }

    /// Returns the `u8` discriminant of this document type.
    pub const fn as_u8(self) -> u8 {
        self as u8
    }

    /// The lowercase word the relay stores, the JSON wire carries and a route
    /// path names, which [`DocType::from_str`] reads back.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Prose => "prose",
            Self::Trame => "trame",
            Self::Toile => "toile",
            Self::Murmure => "murmure",
            Self::Settings => "settings",
        }
    }

    /// Returns a human-readable name (English) for the document type.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Prose => "Prose",
            Self::Trame => "Trame",
            Self::Toile => "Toile",
            Self::Murmure => "Murmure",
            Self::Settings => "Settings",
        }
    }

    /// Short monogram glyph for the launcher document card (one badge per type).
    /// ASCII-only (no emoji): the card styles it as a coloured tile. Distinct
    /// across Trame/Toile (both start with "T").
    pub const fn icon(self) -> &'static str {
        match self {
            Self::Prose => "Pr",
            Self::Trame => "Tr",
            Self::Toile => "To",
            Self::Murmure => "Mu",
            Self::Settings => "Se",
        }
    }

    /// Generic, type-describing label for icon-less contexts (browser tab title,
    /// share invitations, future global search). Distinct from the brand
    /// [`Self::name`] ("Trame"): here the document is described generically
    /// ("Spreadsheet") so an unnamed doc reads as "Untitled - Spreadsheet".
    pub const fn generic_label(self) -> &'static str {
        match self {
            Self::Prose => "Document",
            Self::Trame => "Spreadsheet",
            Self::Toile => "Whiteboard",
            Self::Murmure => "Channel",
            Self::Settings => "Settings",
        }
    }

    /// The document types the launcher "new document" picker can create, in
    /// display order (`ambre-spec-launcher.md`, CHROME-038). This is the single
    /// source of truth the app-agnostic type picker iterates -- it never
    /// hardcodes a type.
    ///
    /// Only types whose Logic state can actually be instantiated
    /// (`ambre_logic::DocState::new`) are listed. `Murmure` returns `None`
    /// there (no engine yet), and creating one would yield a document that
    /// never renders.
    pub const fn creatable() -> &'static [DocType] {
        &[Self::Trame, Self::Toile, Self::Prose]
    }

    /// Returns a short description of the document type.
    pub const fn description(self) -> &'static str {
        match self {
            Self::Prose => "Rich-text collaborative editor",
            Self::Trame => "Spreadsheet with formulas and real-time collaboration",
            Self::Toile => "Infinite canvas whiteboard and presentations",
            Self::Murmure => "End-to-end encrypted messaging channels",
            Self::Settings => "The settings of one user",
        }
    }
}

impl fmt::Display for DocType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.name())
    }
}

/// Parses an owned document type word, as a database row holds it.
impl TryFrom<String> for DocType {
    type Error = DocTypeError;

    fn try_from(word: String) -> Result<Self, Self::Error> {
        word.parse()
    }
}

/// Parses a document type from its lowercase word, exactly as `as_str` writes
/// it. The match is strict, so `"Prose"` is refused and each set holds one
/// spelling.
impl FromStr for DocType {
    type Err = DocTypeError;

    fn from_str(word: &str) -> Result<Self, Self::Err> {
        match word {
            "prose" => Ok(Self::Prose),
            "trame" => Ok(Self::Trame),
            "toile" => Ok(Self::Toile),
            "murmure" => Ok(Self::Murmure),
            "settings" => Ok(Self::Settings),
            _ => Err(DocTypeError::InvalidString(word.to_string())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_discriminants() {
        assert_eq!(DocType::Prose as u8, 0);
        assert_eq!(DocType::Trame as u8, 1);
        assert_eq!(DocType::Toile as u8, 2);
        assert_eq!(DocType::Murmure as u8, 3);
    }

    #[test]
    fn test_from_u8() {
        assert_eq!(DocType::from_u8(0), Some(DocType::Prose));
        assert_eq!(DocType::from_u8(1), Some(DocType::Trame));
        assert_eq!(DocType::from_u8(2), Some(DocType::Toile));
        assert_eq!(DocType::from_u8(3), Some(DocType::Murmure));
        assert_eq!(DocType::from_u8(4), Some(DocType::Settings));
        assert_eq!(DocType::from_u8(5), None);
        assert_eq!(DocType::from_u8(255), None);
    }

    #[test]
    fn test_as_u8() {
        assert_eq!(DocType::Prose.as_u8(), 0);
        assert_eq!(DocType::Trame.as_u8(), 1);
        assert_eq!(DocType::Toile.as_u8(), 2);
        assert_eq!(DocType::Murmure.as_u8(), 3);
    }

    #[test]
    fn test_name() {
        assert_eq!(DocType::Prose.name(), "Prose");
        assert_eq!(DocType::Trame.name(), "Trame");
        assert_eq!(DocType::Toile.name(), "Toile");
        assert_eq!(DocType::Murmure.name(), "Murmure");
    }

    #[test]
    fn test_creatable_matches_logic_support() {
        // The picker offers exactly the launcher-creatable types, in order.
        // Murmure is excluded until its Logic state exists.
        assert_eq!(
            DocType::creatable(),
            &[DocType::Trame, DocType::Toile, DocType::Prose]
        );
        assert!(!DocType::creatable().contains(&DocType::Murmure));
    }

    /// Every document type reads back from its word, and only from it.
    #[test]
    fn each_type_reads_back_from_its_word_only() {
        for doc_type in [
            DocType::Prose,
            DocType::Trame,
            DocType::Toile,
            DocType::Murmure,
            DocType::Settings,
        ] {
            assert_eq!(doc_type.as_str().parse(), Ok(doc_type));
            assert!(doc_type.name().parse::<DocType>().is_err());
        }
        assert_eq!(
            DocType::try_from("spreadsheet".to_string()),
            Err(DocTypeError::InvalidString("spreadsheet".to_string()))
        );
    }

    #[test]
    fn test_serde() {
        let dt = DocType::Trame;
        let serialized = serde_json::to_string(&dt).unwrap();
        let deserialized: DocType = serde_json::from_str(&serialized).unwrap();
        assert_eq!(dt, deserialized);
    }
}
