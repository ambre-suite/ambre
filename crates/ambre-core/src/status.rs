//! The status words the relay answers, shared with the client as closed sets.
//!
//! Each set has one parser, which refuses a word no build wrote, so Sync and
//! the relay read the same spelling (`SRV-180`).

use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

/// Who vouched for a snapshot (`SRV-126`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SnapshotStatus {
    /// Nobody yet. Never served, and never moves a trim.
    Pending,
    /// Its own author. Served beside the reference snapshot.
    #[serde(rename = "self")]
    SelfConfirmed,
    /// Another member. Moves the trim.
    Confirmed,
}

impl SnapshotStatus {
    /// The word the relay stores and answers, which the migration checks.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::SelfConfirmed => "self",
            Self::Confirmed => "confirmed",
        }
    }
}

impl fmt::Display for SnapshotStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A word that names no status of its set.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("unknown status: {0}")]
pub struct StatusError(pub String);

/// Parses a snapshot status from its word, exactly as `as_str` writes it.
impl FromStr for SnapshotStatus {
    type Err = StatusError;

    fn from_str(word: &str) -> Result<Self, Self::Err> {
        match word {
            "pending" => Ok(Self::Pending),
            "self" => Ok(Self::SelfConfirmed),
            "confirmed" => Ok(Self::Confirmed),
            _ => Err(StatusError(word.to_string())),
        }
    }
}

/// What a key rotation answers. The set holds one word today, typed so that a
/// second one is a compile error on both sides rather than a silent string.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RotationStatus {
    /// The new key generation is stored for every member.
    Rotated,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Holds `SRV-180`: every snapshot status reads back from its word, on the
    /// wire and from a row, and an unknown word is refused.
    #[test]
    fn each_snapshot_status_reads_back_from_its_word_only() {
        use SnapshotStatus::{Confirmed, Pending, SelfConfirmed};
        for status in [Pending, SelfConfirmed, Confirmed] {
            assert_eq!(status.as_str().parse(), Ok(status));
            let json = serde_json::to_value(status).unwrap();
            assert_eq!(json, serde_json::json!(status.as_str()));
            assert_eq!(
                serde_json::from_value::<SnapshotStatus>(json).unwrap(),
                status
            );
        }
        assert_eq!(
            "Self".parse::<SnapshotStatus>(),
            Err(StatusError("Self".to_string()))
        );
        assert!(serde_json::from_value::<SnapshotStatus>(serde_json::json!("vouched")).is_err());
    }

    /// Holds `SRV-180`: the rotation answer reads its one word and no other.
    #[test]
    fn the_rotation_status_reads_its_one_word() {
        let json = serde_json::to_value(RotationStatus::Rotated).unwrap();
        assert_eq!(json, serde_json::json!("rotated"));
        assert_eq!(
            serde_json::from_value::<RotationStatus>(json).unwrap(),
            RotationStatus::Rotated
        );
        assert!(serde_json::from_value::<RotationStatus>(serde_json::json!("done")).is_err());
    }
}
