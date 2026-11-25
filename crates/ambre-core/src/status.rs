//! The status words the relay answers, shared with the client as closed sets.
//!
//! Each set has one parser, which refuses a word no build wrote, so Sync and
//! the relay read the same spelling (`SRV-180`).

use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

use crate::refused::{Refused, Refuses};

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

/// A word that names no status of its set: the marker of its
/// [`Refused<StatusWord>`], which echoes the word.
pub enum StatusWord {}

impl Refuses for StatusWord {
    type Raw = String;
    const NAME: crate::StaticKey<crate::DebugNames> = crate::StaticKey::new("StatusError");

    fn message(raw: &Self::Raw, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "unknown status: {raw}")
    }
}

/// Parses a snapshot status from its word, exactly as `as_str` writes it.
impl FromStr for SnapshotStatus {
    type Err = Refused<StatusWord>;

    fn from_str(word: &str) -> Result<Self, Self::Err> {
        match word {
            "pending" => Ok(Self::Pending),
            "self" => Ok(Self::SelfConfirmed),
            "confirmed" => Ok(Self::Confirmed),
            _ => Err(Refused::new(word.to_string())),
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
            type Err = Refused<StatusWord>;

            fn from_str(word: &str) -> Result<Self, Self::Err> {
                match word {
                    $( $word => Ok(Self::$variant), )+
                    _ => Err(Refused::new(word.to_string())),
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
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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
    /// The request was refused or failed on this device, not by the relay,
    /// for `cause`.
    Local(LocalFailure),
}

/// Why a step of the Sync worker failed on this device, before or after the
/// relay answered. The worker logs the detail it met. The bus carries the
/// kind alone, and the view makes its words (`BUS-046`).
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LocalFailure {
    /// The session holds no access token or no refresh token: nobody is
    /// signed in.
    NotSignedIn,
    /// The session holds no keys yet, or no longer.
    SessionLocked,
    /// The worker runs with no relay to ask.
    NoRelay,
    /// The document the step needs has no open session here.
    DocumentNotOpen,
    /// The document is open here and not yet on the relay.
    NotOnRelay,
    /// A members answer holds no row for this user's exchange key.
    OwnRowMissing,
    /// Bytes the relay served do not decode as the key, the wrap or the
    /// head they stand for.
    Malformed,
    /// The bytes name a key this client does not hold.
    KeyOutOfReach,
    /// A catalogue entry carries no anchor of its channel's log.
    Unanchored,
    /// An entry this client signed does not encode, so it has no digest.
    NoDigest,
    /// A direct message names more members than its budget (`SYNC-180`).
    TooManyMembers,
    /// A member of a direct message comes with no signing key.
    MemberUnsigned,
    /// The exchange key the relay serves for this user is not the one
    /// pinned.
    PinMismatch(crate::id::Id<crate::id::User>),
    /// The relay created another document than the one this client minted
    /// (`SYNC-136`).
    WrongDocument,
    /// A downloaded blob is not the one its address names (`SYNC-052`).
    BlobMismatch,
    /// The channel names no meta document.
    NoMetaDocument,
    /// An edit of this copy still waits for the relay's answer.
    EditPending,
    /// No user of the relay has the email the request names.
    UserNotFound,
    /// The invitee the relay serves now is not the one the user confirmed.
    InviteeChanged,
    /// The member the request names is no longer in the document.
    MemberGone,
    /// The channel the request needs is not open here.
    ChannelNotOpen,
    /// The invitee is already a member of the channel.
    AlreadyMember,
    /// The notice of a new member did not write, so the invite did not leave.
    NoticeFailed,
    /// The name the user typed was refused.
    NameRefused,
    /// The relay refused an edit of this copy, so the copy is not the relay's.
    EditRefused,
    /// A wrap, a seal or an open of a key failed.
    Crypto,
    /// A request body, an entry or a link did not encode.
    Encode,
    /// The relay answered a success other than the one the request asked
    /// for, or a body this step does not read.
    UnexpectedAnswer,
    /// The log of the document walks again, or a walk found it rooted
    /// outside its space, so nothing is sealed under its key.
    LogUnsettled,
    /// The log the relay serves does not extend the log this device checked,
    /// so nothing is written on it (audits `P26`, `P27`).
    LogDiverged,
    /// A removal from a channel would cap the member in more documents than
    /// one entry carries, so it leaves no document uncapped and fails
    /// (`SYNC-312`).
    TooManyChannelDocuments,
    /// The relay listed the documents of a channel without one that its
    /// sealed `meta` names, so a removal would leave that document uncapped
    /// (`SYNC-320`).
    ChannelDocumentHidden,
    /// The relay served keys for a present member other than the ones the
    /// membership log signs. Both keys derive from the Master Seed and never
    /// change, so the relay answered keys of its own (`SYNC-322`).
    KeyReplaced(crate::id::Id<crate::id::User>),
}

impl fmt::Display for LocalFailure {
    /// The words a person reads for the failure.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotSignedIn => f.write_str("you are not signed in"),
            Self::SessionLocked => f.write_str("the session is locked"),
            Self::NoRelay => f.write_str("no relay is reachable from this session"),
            Self::DocumentNotOpen => f.write_str("the document is not open"),
            Self::NotOnRelay => f.write_str("the document is not on the relay yet"),
            Self::OwnRowMissing => f.write_str("your member entry was not found"),
            Self::Malformed => f.write_str("the relay served a key that does not decode"),
            Self::KeyOutOfReach => f.write_str("the bytes name a key this device does not hold"),
            Self::Unanchored => f.write_str("the channel entry carries no anchor"),
            Self::NoDigest => f.write_str("an entry of the log does not encode"),
            Self::TooManyMembers => f.write_str("a direct message holds too many members"),
            Self::MemberUnsigned => f.write_str("a member has no signing key"),
            Self::PinMismatch(user) => {
                write!(f, "the key of {user} is not the one you verified")
            }
            Self::WrongDocument => f.write_str("the relay created another document"),
            Self::BlobMismatch => f.write_str("the file is not the one its address names"),
            Self::NoMetaDocument => f.write_str("the channel names no meta document"),
            Self::EditPending => f.write_str("an edit still waits for the relay"),
            Self::UserNotFound => f.write_str("no user has that email"),
            Self::InviteeChanged => f.write_str("the invitee changed since you confirmed them"),
            Self::MemberGone => f.write_str("the member is no longer in the document"),
            Self::ChannelNotOpen => f.write_str("the channel is not open"),
            Self::AlreadyMember => f.write_str("they are already a member of the channel"),
            Self::NoticeFailed => f.write_str("the notice of the new member did not write"),
            Self::NameRefused => f.write_str("the name could not be changed"),
            Self::LogUnsettled => f.write_str("the members of the channel are being checked"),
            Self::EditRefused => f.write_str("the relay refused an edit of this copy"),
            Self::Crypto => f.write_str("a key operation failed"),
            Self::Encode => f.write_str("the request did not encode"),
            Self::UnexpectedAnswer => f.write_str("the relay answered something unexpected"),
            Self::LogDiverged => {
                f.write_str("the relay served members that do not extend the ones you checked")
            }
            Self::TooManyChannelDocuments => {
                f.write_str("the channel holds more documents than one removal can cap")
            }
            Self::ChannelDocumentHidden => f.write_str("the relay hid a document of the channel"),
            Self::KeyReplaced(user) => {
                write!(
                    f,
                    "the relay served another key for {user} than the members signed"
                )
            }
        }
    }
}

/// The debug form is the words of [`fmt::Display`]: one table of words, which
/// the Main instance carries once (`ambre-spec-budgets.md section 5`).
impl fmt::Debug for LocalFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl fmt::Display for ApiFailure {
    /// The words a person reads for the failure.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Relay {
                code: Some(code), ..
            } => write!(f, "the relay refused the request ({code})"),
            Self::Relay { status, code: None } => {
                write!(f, "the relay refused the request (HTTP {status})")
            }
            Self::NoAnswer => f.write_str("the relay did not answer"),
            Self::Unreadable {
                cause: UnreadableCause::Malformed,
            } => f.write_str("the relay's answer did not read"),
            Self::Unreadable {
                cause: UnreadableCause::UnknownWord,
            } => f.write_str("the relay is newer than this build"),
            Self::Local(cause) => cause.fmt(f),
        }
    }
}

/// The debug form is the words of [`fmt::Display`]: one table of words, which
/// the Main instance carries once (`ambre-spec-budgets.md section 5`).
impl fmt::Debug for ApiFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
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
                Err(Refused::new(word.to_string()))
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
            Err(Refused::new("Self".to_string()))
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
