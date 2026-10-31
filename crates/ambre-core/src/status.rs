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
pub struct StatusError(String);

impl StatusError {
    /// Wraps a raw value. The constructor checks nothing.
    pub const fn new(value: String) -> Self {
        Self(value)
    }

    /// Returns the raw text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

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
        /// A finalize lease locked the segment against deltas.
        SegmentLocked => "SEGMENT_LOCKED",
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

impl RefusalCode {
    /// The HTTP status a refusal under this code answers. A socket refusal
    /// carries the code alone, and Sync reads its status from here.
    #[must_use]
    pub const fn status(self) -> HttpStatus {
        HttpStatus(match self {
            Self::Unauthorized => 401,
            Self::Forbidden | Self::SetupRequired => 403,
            Self::NotFound => 404,
            Self::Conflict
            | Self::ClockMismatch
            | Self::RotationOwed
            | Self::SegmentLocked
            | Self::MembersChanged
            | Self::KeyChanged
            | Self::LogMoved
            | Self::SnapshotRequired
            | Self::CompactionLockHeld => 409,
            Self::InvalidSignature => 422,
            Self::UnknownProvider | Self::BadRequest => 400,
            Self::PayloadTooLarge => 413,
            Self::RateLimited => 429,
            Self::InternalError => 500,
            Self::RateLimiterUnavailable => 503,
        })
    }
}

/// An HTTP status the relay answered. Only an answer builds one: a failure
/// with no answer has no status, and [`ApiFailure`] names it apart.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct HttpStatus(u16);

impl HttpStatus {
    /// Wraps a raw value. The constructor checks nothing.
    pub const fn new(value: u16) -> Self {
        Self(value)
    }

    /// Returns the raw value.
    pub const fn get(self) -> u16 {
        self.0
    }

    /// Whether the status is a success, `2xx`.
    #[must_use]
    pub const fn is_success(self) -> bool {
        self.0 >= 200 && self.0 < 300
    }

    /// Whether the status refuses the request itself, `4xx`: the same
    /// request meets the same answer.
    #[must_use]
    pub const fn is_client_error(self) -> bool {
        self.0 >= 400 && self.0 < 500
    }
}

impl fmt::Display for HttpStatus {
    /// The bare number, `409` for a conflict.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// Why an answer of the relay did not read. Variant 0 is the value that was
/// `false`, so the bus frame reads the same.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum UnreadableCause {
    /// The body is not the shape the request expects.
    #[default]
    Malformed,
    /// The body names a word of a closed set this build does not know, which
    /// only a newer relay writes.
    UnknownWord,
}

/// Why a request the user's action asked for failed. The relay's own answer
/// carries its status and the refusal code its body named. Every other case
/// is named without a status, so no code stands for an answer that never came.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ApiFailure {
    /// The relay refused the request with `status`. `code` is the refusal
    /// code its body named, when the status may carry one.
    Relay {
        /// The status the relay answered.
        status: HttpStatus,
        /// The code the refusal named, if any.
        code: Option<RefusalCode>,
    },
    /// No answer arrived: the transport failed, or the deadline passed.
    NoAnswer,
    /// The answer did not read, for `cause`.
    Unreadable {
        /// Whether the body was malformed, or named a word this build does not
        /// know.
        cause: UnreadableCause,
    },
    /// The request was refused or failed on this device, not by the relay.
    /// Examples are a name too long or a key that does not decode.
    Local,
}

impl ApiFailure {
    /// The relay's refusal `code` under the status it answers.
    #[must_use]
    pub const fn refused(code: RefusalCode) -> Self {
        Self::Relay {
            status: code.status(),
            code: Some(code),
        }
    }

    /// The status the relay answered, or `None` when no answer came.
    #[must_use]
    pub const fn status(self) -> Option<HttpStatus> {
        match self {
            Self::Relay { status, .. } => Some(status),
            _ => None,
        }
    }

    /// The refusal code the relay named, if any.
    #[must_use]
    pub const fn code(self) -> Option<RefusalCode> {
        match self {
            Self::Relay { code, .. } => code,
            _ => None,
        }
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
            (SegmentLocked, "SEGMENT_LOCKED"),
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
