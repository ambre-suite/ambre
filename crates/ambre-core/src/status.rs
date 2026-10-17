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

/// What a write of the relay answers when its outcome is the whole answer,
/// or rides beside the data it returns (`SRV-181`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WriteStatus {
    /// The session is closed on the relay.
    LoggedOut,
    /// The document holds its new name.
    Renamed,
    /// The document or the organization is gone.
    Deleted,
    /// The member holds the new role.
    RoleUpdated,
    /// The master key material is stored.
    Stored,
    /// The media is recorded against its document.
    Confirmed,
    /// The organization holds the new member.
    Added,
    /// The organization no longer holds the member.
    Removed,
    /// The organization member holds the new role.
    Updated,
    /// The member no longer reads the document.
    Revoked,
    /// The organization's copy of a master key is handed back.
    Recovered,
}

/// The code a refusal of the relay names, which Sync matches (`SRV-182`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RefusalCode {
    /// No valid session.
    Unauthorized,
    /// The role does not allow the request.
    Forbidden,
    /// The account still owes its key setup.
    SetupRequired,
    /// The resource is not there, or not for this caller.
    NotFound,
    /// The request contradicts what the relay holds.
    Conflict,
    /// The writer's clock does not follow the relay's.
    ClockMismatch,
    /// The document owes a key rotation.
    RotationOwed,
    /// A member joined after the rotation read the members.
    MembersChanged,
    /// The key an invitation wraps is no longer the document's.
    KeyChanged,
    /// Another membership entry took the place this one was signed for.
    LogMoved,
    /// A signature does not verify.
    InvalidSignature,
    /// The document asks for a snapshot first.
    SnapshotRequired,
    /// Another writer holds the compaction lock.
    CompactionLockHeld,
    /// No identity provider of that name.
    UnknownProvider,
    /// The request does not read.
    BadRequest,
    /// The body exceeds its ceiling.
    PayloadTooLarge,
    /// The caller sent too many requests in the window.
    RateLimited,
    /// The rate limiter cannot be read, so an auth request is refused.
    RateLimiterUnavailable,
    /// The relay failed; the cause stays in its log.
    InternalError,
}

impl RefusalCode {
    /// The word the relay answers, which Sync keeps at the end of an error.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Unauthorized => "UNAUTHORIZED",
            Self::Forbidden => "FORBIDDEN",
            Self::SetupRequired => "SETUP_REQUIRED",
            Self::NotFound => "NOT_FOUND",
            Self::Conflict => "CONFLICT",
            Self::ClockMismatch => "CLOCK_MISMATCH",
            Self::RotationOwed => "ROTATION_OWED",
            Self::MembersChanged => "MEMBERS_CHANGED",
            Self::KeyChanged => "KEY_CHANGED",
            Self::LogMoved => "LOG_MOVED",
            Self::InvalidSignature => "INVALID_SIGNATURE",
            Self::SnapshotRequired => "SNAPSHOT_REQUIRED",
            Self::CompactionLockHeld => "COMPACTION_LOCK_HELD",
            Self::UnknownProvider => "UNKNOWN_PROVIDER",
            Self::BadRequest => "BAD_REQUEST",
            Self::PayloadTooLarge => "PAYLOAD_TOO_LARGE",
            Self::RateLimited => "RATE_LIMITED",
            Self::RateLimiterUnavailable => "RATE_LIMITER_UNAVAILABLE",
            Self::InternalError => "INTERNAL_ERROR",
        }
    }
}

impl fmt::Display for RefusalCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Holds `SRV-182`: each refusal code keeps the word the relay answered
    /// before it was typed. The wire spells it as `as_str` does, and it reads
    /// back from it alone.
    #[test]
    fn each_refusal_code_keeps_its_word() {
        use RefusalCode::*;
        for (code, word) in [
            (Unauthorized, "UNAUTHORIZED"),
            (Forbidden, "FORBIDDEN"),
            (SetupRequired, "SETUP_REQUIRED"),
            (NotFound, "NOT_FOUND"),
            (Conflict, "CONFLICT"),
            (ClockMismatch, "CLOCK_MISMATCH"),
            (RotationOwed, "ROTATION_OWED"),
            (MembersChanged, "MEMBERS_CHANGED"),
            (KeyChanged, "KEY_CHANGED"),
            (LogMoved, "LOG_MOVED"),
            (InvalidSignature, "INVALID_SIGNATURE"),
            (SnapshotRequired, "SNAPSHOT_REQUIRED"),
            (CompactionLockHeld, "COMPACTION_LOCK_HELD"),
            (UnknownProvider, "UNKNOWN_PROVIDER"),
            (BadRequest, "BAD_REQUEST"),
            (PayloadTooLarge, "PAYLOAD_TOO_LARGE"),
            (RateLimited, "RATE_LIMITED"),
            (RateLimiterUnavailable, "RATE_LIMITER_UNAVAILABLE"),
            (InternalError, "INTERNAL_ERROR"),
        ] {
            assert_eq!(code.as_str(), word);
            let json = serde_json::to_value(code).unwrap();
            assert_eq!(json, serde_json::json!(word));
            assert_eq!(serde_json::from_value::<RefusalCode>(json).unwrap(), code);
        }
        assert!(serde_json::from_value::<RefusalCode>(serde_json::json!("forbidden")).is_err());
    }

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

    /// Holds `SRV-181`: each write status keeps the word the relay answered
    /// before it was typed, and reads back from it alone.
    #[test]
    fn each_write_status_keeps_its_word() {
        use WriteStatus::*;
        for (status, word) in [
            (LoggedOut, "logged_out"),
            (Renamed, "renamed"),
            (Deleted, "deleted"),
            (RoleUpdated, "role_updated"),
            (Stored, "stored"),
            (Confirmed, "confirmed"),
            (Added, "added"),
            (Removed, "removed"),
            (Updated, "updated"),
            (Revoked, "revoked"),
            (Recovered, "recovered"),
        ] {
            let json = serde_json::to_value(status).unwrap();
            assert_eq!(json, serde_json::json!(word));
            assert_eq!(serde_json::from_value::<WriteStatus>(json).unwrap(), status);
        }
        assert!(serde_json::from_value::<WriteStatus>(serde_json::json!("done")).is_err());
    }
}
