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

/// Declares a closed set whose members each have one word. The enum, `as_str`,
/// `ALL`, `FromStr` and serde all come from the one list, so the wire cannot
/// spell a member otherwise than `as_str` does.
macro_rules! closed_words {
    (
        $(#[$meta:meta])*
        $vis:vis enum $name:ident {
            $( $(#[$vmeta:meta])* $variant:ident => $word:literal, )+
        }
    ) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        $vis enum $name {
            $( $(#[$vmeta])* $variant, )+
        }

        impl $name {
            /// Every member of the set, in the order it is declared.
            pub const ALL: &'static [Self] = &[$(Self::$variant),+];

            /// The words of the set, in the order of `ALL`.
            const WORDS: &'static [&'static str] = &[$($word),+];

            /// The one word of this member, on the wire and in a message.
            pub const fn as_str(self) -> &'static str {
                match self {
                    $( Self::$variant => $word, )+
                }
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(self.as_str())
            }
        }

        /// Parses a member from its word, exactly as `as_str` writes it.
        impl FromStr for $name {
            type Err = StatusError;

            fn from_str(word: &str) -> Result<Self, Self::Err> {
                match word {
                    $( $word => Ok(Self::$variant), )+
                    _ => Err(StatusError(word.to_string())),
                }
            }
        }

        impl Serialize for $name {
            fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.serialize_str(self.as_str())
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                struct Word;

                impl serde::de::Visitor<'_> for Word {
                    type Value = $name;

                    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                        f.write_str(concat!("a word of ", stringify!($name)))
                    }

                    // An unknown word reads as serde's unknown variant, as a
                    // derived enum's does (`SYNC-152`).
                    fn visit_str<E: serde::de::Error>(self, word: &str) -> Result<$name, E> {
                        word.parse()
                            .map_err(|_| E::unknown_variant(word, $name::WORDS))
                    }
                }

                deserializer.deserialize_str(Word)
            }
        }
    };
}

closed_words! {
    /// The code a refusal of the relay names, which Sync matches (`SRV-182`).
    pub enum RefusalCode {
        /// No valid session.
        Unauthorized => "UNAUTHORIZED",
        /// The role does not allow the request.
        Forbidden => "FORBIDDEN",
        /// The account still owes its key setup.
        SetupRequired => "SETUP_REQUIRED",
        /// The resource is not there, or not for this caller.
        NotFound => "NOT_FOUND",
        /// The request contradicts what the relay holds.
        Conflict => "CONFLICT",
        /// The writer's clock does not follow the relay's.
        ClockMismatch => "CLOCK_MISMATCH",
        /// The document owes a key rotation.
        RotationOwed => "ROTATION_OWED",
        /// A member joined after the rotation read the members.
        MembersChanged => "MEMBERS_CHANGED",
        /// The key an invitation wraps is no longer the document's.
        KeyChanged => "KEY_CHANGED",
        /// Another membership entry took the place this one was signed for.
        LogMoved => "LOG_MOVED",
        /// A signature does not verify.
        InvalidSignature => "INVALID_SIGNATURE",
        /// The document asks for a snapshot first.
        SnapshotRequired => "SNAPSHOT_REQUIRED",
        /// Another writer holds the compaction lock.
        CompactionLockHeld => "COMPACTION_LOCK_HELD",
        /// No identity provider of that name.
        UnknownProvider => "UNKNOWN_PROVIDER",
        /// The request does not read.
        BadRequest => "BAD_REQUEST",
        /// The body exceeds its ceiling.
        PayloadTooLarge => "PAYLOAD_TOO_LARGE",
        /// The caller sent too many requests in the window.
        RateLimited => "RATE_LIMITED",
        /// The rate limiter cannot be read, so an auth request is refused.
        RateLimiterUnavailable => "RATE_LIMITER_UNAVAILABLE",
        /// The relay failed; the cause stays in its log.
        InternalError => "INTERNAL_ERROR",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Holds `SRV-182`: the wire and a parse spell every refusal code as
    /// `as_str` does. An unknown word or a lowercase one is serde's unknown
    /// variant, which Sync reads as a newer relay's.
    #[test]
    fn every_refusal_code_reads_and_writes_as_its_word_alone() {
        for &code in RefusalCode::ALL {
            let json = serde_json::to_value(code).unwrap();
            assert_eq!(json, serde_json::json!(code.as_str()));
            assert_eq!(serde_json::from_value::<RefusalCode>(json).unwrap(), code);
            assert_eq!(code.as_str().parse(), Ok(code));
            assert_eq!(code.to_string(), code.as_str());
        }
        for word in ["forbidden", "QUOTA_EXCEEDED"] {
            let refused = serde_json::from_value::<RefusalCode>(serde_json::json!(word));
            let error = refused.unwrap_err();
            assert!(error.is_data(), "{error}");
            assert!(
                error
                    .to_string()
                    .starts_with(&format!("unknown variant `{word}`")),
                "{error}"
            );
            assert_eq!(
                word.parse::<RefusalCode>(),
                Err(StatusError(word.to_string()))
            );
        }
        assert!(serde_json::from_value::<RefusalCode>(serde_json::json!(3)).is_err());
    }

    /// Holds `SRV-182`: each refusal code keeps the word the relay answered
    /// before it was typed. A code added to the set must add its row here.
    #[test]
    fn each_refusal_code_keeps_its_word() {
        use RefusalCode::*;
        let frozen = [
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
        ];
        assert_eq!(
            frozen.len(),
            RefusalCode::ALL.len(),
            "a code has no frozen word"
        );
        for (code, word) in frozen {
            assert_eq!(code.as_str(), word);
        }
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
