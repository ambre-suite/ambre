//! Document type enumeration.
//!
//! Each Ambre document belongs to one of these four types.
//! The numeric values are stable across serialization (postcard, network).

use serde::{Deserialize, Serialize};
use std::fmt;

/// Type of an Ambre document.
///
/// The variants are assigned explicit discriminants that are used in binary
/// serialization (postcard) and in the database.
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DocType {
    /// Prose – rich‑text collaborative editor.
    Prose = 0,
    /// Trame – spreadsheet with formulas and real‑time collaboration.
    Trame = 1,
    /// Toile – infinite canvas whiteboard and presentations.
    Toile = 2,
    /// Murmure – end‑to‑end encrypted messaging channels.
    Murmure = 3,
}

impl DocType {
    /// Converts a `u8` to a `DocType`, returning `None` if the value is invalid.
    pub fn from_u8(value: u8) -> Option<Self> {
        match value {
            0 => Some(Self::Prose),
            1 => Some(Self::Trame),
            2 => Some(Self::Toile),
            3 => Some(Self::Murmure),
            _ => None,
        }
    }

    /// Returns the `u8` discriminant of this document type.
    pub const fn as_u8(self) -> u8 {
        self as u8
    }

    /// Returns a human‑readable name (English) for the document type.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Prose => "Prose",
            Self::Trame => "Trame",
            Self::Toile => "Toile",
            Self::Murmure => "Murmure",
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
        }
    }

    /// The document types the launcher "new document" picker can create, in
    /// display order (`ambre-spec-chrome.md §2.G`). This is the single source of
    /// truth the app-agnostic type picker iterates -- it never hardcodes a type.
    ///
    /// Only types whose Logic state can actually be instantiated
    /// (`ambre_logic::DocState::new`) are listed: `Prose` and `Murmure` return
    /// `None` there (no engine yet -- `Prose` lands in Epic `E5`), so creating
    /// one would yield a document that never renders. `Prose` joins this list the
    /// moment its Logic state exists -- a one-entry change, no picker rework.
    pub const fn creatable() -> &'static [DocType] {
        &[Self::Trame, Self::Toile]
    }

    /// Returns a short description of the document type.
    pub const fn description(self) -> &'static str {
        match self {
            Self::Prose => "Rich‑text collaborative editor",
            Self::Trame => "Spreadsheet with formulas and real‑time collaboration",
            Self::Toile => "Infinite canvas whiteboard and presentations",
            Self::Murmure => "End‑to‑end encrypted messaging channels",
        }
    }
}

impl fmt::Display for DocType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.name())
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
        assert_eq!(DocType::from_u8(4), None);
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
        // Prose/Murmure are excluded until their Logic state exists.
        assert_eq!(DocType::creatable(), &[DocType::Trame, DocType::Toile]);
        assert!(!DocType::creatable().contains(&DocType::Prose));
        assert!(!DocType::creatable().contains(&DocType::Murmure));
    }

    #[test]
    fn test_serde() {
        let dt = DocType::Trame;
        let serialized = serde_json::to_string(&dt).unwrap();
        let deserialized: DocType = serde_json::from_str(&serialized).unwrap();
        assert_eq!(dt, deserialized);
    }
}
