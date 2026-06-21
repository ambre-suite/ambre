//! Role enumeration for document, channel, and organization memberships.
//!
//! Defines the allowed values for the `role` field in `document_members`,
//! `channel_members`, and `organization_members` tables.
//!
//! The variants are serialized as lowercase strings (e.g., `"owner"`, `"editor"`)
//! for JSON REST APIs and as `u8` discriminants for binary serialization (postcard).

use crate::error::RoleError;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

/// All possible roles across Ambre.
///
/// The discriminant values are stable across serialization.
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    /// Owner of a document, channel, or organization.
    /// Full permissions, can delete, invite, revoke, and change roles.
    Owner = 0,
    /// Editor of a document (write access).
    Editor = 1,
    /// Viewer of a document (read‑only access).
    Viewer = 2,
    /// Administrator of a channel or organization.
    /// Can manage members but cannot delete the entity.
    Admin = 3,
    /// Member of a channel or organization (basic participation).
    Member = 4,
}

impl Role {
    /// Converts a `u8` to a `Role`, returning `None` if the value is invalid.
    pub fn from_u8(value: u8) -> Option<Self> {
        match value {
            0 => Some(Self::Owner),
            1 => Some(Self::Editor),
            2 => Some(Self::Viewer),
            3 => Some(Self::Admin),
            4 => Some(Self::Member),
            _ => None,
        }
    }

    /// Returns the `u8` discriminant of this role.
    pub const fn as_u8(self) -> u8 {
        self as u8
    }

    /// Returns the lowercase string representation used in JSON APIs and SQL CHECK constraints.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Owner => "owner",
            Self::Editor => "editor",
            Self::Viewer => "viewer",
            Self::Admin => "admin",
            Self::Member => "member",
        }
    }

    /// Returns `true` if this role is allowed for documents.
    pub const fn is_valid_for_document(self) -> bool {
        matches!(self, Self::Owner | Self::Editor | Self::Viewer)
    }

    /// Returns `true` if this role is allowed for channels.
    pub const fn is_valid_for_channel(self) -> bool {
        matches!(self, Self::Owner | Self::Admin | Self::Member)
    }

    /// Returns `true` if this role is allowed for organizations.
    pub const fn is_valid_for_organization(self) -> bool {
        matches!(self, Self::Owner | Self::Admin | Self::Member)
    }

    /// Returns the default role for a new document member (viewer).
    pub const fn default_document() -> Self {
        Self::Viewer
    }

    /// Returns the default role for a new channel member (member).
    pub const fn default_channel() -> Self {
        Self::Member
    }

    /// Returns the default role for a new organization member (member).
    pub const fn default_organization() -> Self {
        Self::Member
    }
}

impl fmt::Display for Role {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl std::convert::TryFrom<u8> for Role {
    type Error = RoleError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        Self::from_u8(value).ok_or(RoleError::InvalidDiscriminant(value))
    }
}

/// Parses a role from a string (case‑insensitive).
///
/// Returns `RoleError::InvalidString` if the string does not match any known role.
impl FromStr for Role {
    type Err = RoleError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "owner" => Ok(Self::Owner),
            "editor" => Ok(Self::Editor),
            "viewer" => Ok(Self::Viewer),
            "admin" => Ok(Self::Admin),
            "member" => Ok(Self::Member),
            _ => Err(RoleError::InvalidString(s.to_string())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_discriminants() {
        assert_eq!(Role::Owner as u8, 0);
        assert_eq!(Role::Editor as u8, 1);
        assert_eq!(Role::Viewer as u8, 2);
        assert_eq!(Role::Admin as u8, 3);
        assert_eq!(Role::Member as u8, 4);
    }

    #[test]
    fn test_from_u8() {
        assert_eq!(Role::from_u8(0), Some(Role::Owner));
        assert_eq!(Role::from_u8(1), Some(Role::Editor));
        assert_eq!(Role::from_u8(2), Some(Role::Viewer));
        assert_eq!(Role::from_u8(3), Some(Role::Admin));
        assert_eq!(Role::from_u8(4), Some(Role::Member));
        assert_eq!(Role::from_u8(5), None);
    }

    #[test]
    fn test_as_str() {
        assert_eq!(Role::Owner.as_str(), "owner");
        assert_eq!(Role::Editor.as_str(), "editor");
        assert_eq!(Role::Viewer.as_str(), "viewer");
        assert_eq!(Role::Admin.as_str(), "admin");
        assert_eq!(Role::Member.as_str(), "member");
    }

    #[test]
    fn test_from_str() {
        assert_eq!(Role::from_str("owner"), Ok(Role::Owner));
        assert_eq!(Role::from_str("EDITOR"), Ok(Role::Editor));
        assert_eq!(Role::from_str("Viewer"), Ok(Role::Viewer));
        assert_eq!(Role::from_str("admin"), Ok(Role::Admin));
        assert_eq!(Role::from_str("member"), Ok(Role::Member));
        assert_eq!(
            Role::from_str("guest"),
            Err(RoleError::InvalidString("guest".to_string()))
        );
    }

    #[test]
    fn test_valid_for_document() {
        assert!(Role::Owner.is_valid_for_document());
        assert!(Role::Editor.is_valid_for_document());
        assert!(Role::Viewer.is_valid_for_document());
        assert!(!Role::Admin.is_valid_for_document());
        assert!(!Role::Member.is_valid_for_document());
    }

    #[test]
    fn test_valid_for_channel() {
        assert!(Role::Owner.is_valid_for_channel());
        assert!(!Role::Editor.is_valid_for_channel());
        assert!(!Role::Viewer.is_valid_for_channel());
        assert!(Role::Admin.is_valid_for_channel());
        assert!(Role::Member.is_valid_for_channel());
    }

    #[test]
    fn test_serde() {
        let role = Role::Editor;
        let serialized = serde_json::to_string(&role).unwrap();
        assert_eq!(serialized, "\"editor\"");
        let deserialized: Role = serde_json::from_str(&serialized).unwrap();
        assert_eq!(role, deserialized);
    }
}
