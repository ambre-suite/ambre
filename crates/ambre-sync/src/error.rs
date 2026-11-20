// ambre-sync/src/error.rs
// ============================================================================
// Why a step of the worker failed, as a value the caller matches.
//
// A step that asks the relay keeps the relay's answer whole, its status and
// its code with it. The sink then reads a setup still owed or a newer relay
// from the value (`SYNC-134`, `SYNC-152`), never from text. A step that fails
// on this device names its cause from one closed set. Text is written once,
// by `Display`, for a log line and a toast.
// ============================================================================

use ambre_bus::ApiFailure;
use ambre_core::LocalFailure;
use ambre_core::id::{Document, Id, User};

use crate::http::{HttpError, Reported};
use crate::pipeline::PipelineError;

/// Why a step of the worker failed.
#[derive(Debug, thiserror::Error)]
pub(crate) enum SyncError {
    /// The relay answered, or did not: its answer, kept whole.
    #[error("{0}")]
    Http(HttpError),
    /// The session holds no keys yet, or no longer.
    #[error("session locked")]
    SessionLocked,
    /// The worker runs with no relay.
    #[error("no relay on this transport")]
    NoRelay,
    /// The document the step needs has no open session here.
    #[error("the document is not open")]
    NotOpen,
    /// The log of the document walks again, or a walk found it rooted
    /// outside its space: nothing is sealed under its key (`SYNC-232`).
    #[error("the log of the document is not settled")]
    LogUnsettled,
    /// The document is open here and not yet on the relay.
    #[error("the document is not on the relay yet")]
    NotOnRelay,
    /// A members answer holds no row for this user's exchange key.
    #[error("own member entry not found")]
    OwnRowMissing,
    /// Bytes too short, or of the wrong size, for the `part` they stand for.
    #[error("{0} does not decode")]
    Malformed(Part),
    /// The bytes name a key this client does not hold.
    #[error("the bytes name a key out of reach")]
    KeyOutOfReach,
    /// A catalogue entry carries no anchor of its channel's log.
    #[error("the catalogue entry carries no anchor")]
    Unanchored,
    /// The served log, walked from the anchor the open accepted, misses the
    /// head this client walked or signed (audits `P26`, `P27`).
    #[error("the served log does not extend the log this client holds")]
    LogDiverged,
    /// An entry this client signed has no digest: it does not encode.
    #[error("an entry of the log does not encode")]
    NoDigest,
    /// A direct message names more members than its budget (`SYNC-180`).
    #[error(
        "a direct message holds at most {} members, its creator included",
        ambre_protocol::membership::DIRECT_MESSAGE_MAX_MEMBERS
    )]
    TooManyMembers,
    /// A member of a direct message comes with no signing key.
    #[error("a member has no signing key")]
    MemberUnsigned,
    /// The exchange key the relay serves for a member is not the one pinned.
    #[error("the exchange key of {0} is not the one pinned")]
    PinMismatch(Id<User>),
    /// The relay created another document than the one this client minted
    /// (`SYNC-136`).
    #[error("the relay answered {answered}, not the minted {minted}")]
    WrongDocument {
        /// The id this client minted.
        minted: Id<Document>,
        /// The id the relay answered.
        answered: Id<Document>,
    },
    /// A downloaded blob is not the one its address names (`SYNC-052`).
    #[error("the blob is not the one its address names")]
    BlobMismatch,
    /// A wrap, a seal or an open of a key failed.
    #[error("key crypto: {0}")]
    Crypto(#[from] ambre_crypto::CryptoError),
    /// A seal or an open of the pipeline failed.
    #[error(transparent)]
    Pipeline(#[from] PipelineError),
    /// An entry or a link did not encode.
    #[error("encode: {0}")]
    Encode(#[from] ambre_protocol::error::SerializationError),
}

/// What bytes that did not decode stood for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Part {
    /// A member's Ed25519 signing key, as the relay served it.
    SigningKey,
    /// A member's X25519 exchange key, as the relay served it.
    ExchangeKey,
    /// A channel key sealed for a space's catalogue.
    SealedChannelKey,
    /// The join head of a public channel, which names no last entry.
    JoinHead,
}

impl std::fmt::Display for Part {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::SigningKey => "the signing key",
            Self::ExchangeKey => "the exchange key",
            Self::SealedChannelKey => "the sealed channel key",
            Self::JoinHead => "the join head",
        })
    }
}

impl From<HttpError> for SyncError {
    fn from(error: HttpError) -> Self {
        Self::Http(error)
    }
}

impl SyncError {
    /// The kind of a step that failed on this device, as the bus carries it.
    /// `None` for the relay's answer, which keeps its own failure.
    pub(crate) const fn local(&self) -> Option<LocalFailure> {
        Some(match self {
            Self::Http(_) => return None,
            Self::SessionLocked => LocalFailure::SessionLocked,
            Self::NoRelay => LocalFailure::NoRelay,
            Self::NotOpen => LocalFailure::DocumentNotOpen,
            Self::NotOnRelay => LocalFailure::NotOnRelay,
            Self::LogUnsettled => LocalFailure::LogUnsettled,
            Self::OwnRowMissing => LocalFailure::OwnRowMissing,
            Self::Malformed(_) => LocalFailure::Malformed,
            Self::KeyOutOfReach => LocalFailure::KeyOutOfReach,
            Self::Unanchored => LocalFailure::Unanchored,
            Self::LogDiverged => LocalFailure::LogDiverged,
            Self::NoDigest => LocalFailure::NoDigest,
            Self::TooManyMembers => LocalFailure::TooManyMembers,
            Self::MemberUnsigned => LocalFailure::MemberUnsigned,
            Self::PinMismatch(user) => LocalFailure::PinMismatch(*user),
            Self::WrongDocument { .. } => LocalFailure::WrongDocument,
            Self::BlobMismatch => LocalFailure::BlobMismatch,
            Self::Crypto(_) | Self::Pipeline(_) => LocalFailure::Crypto,
            Self::Encode(_) => LocalFailure::Encode,
        })
    }
}

impl Reported for SyncError {
    fn failure(&self) -> ApiFailure {
        match self {
            Self::Http(error) => error.failure(),
            other => ApiFailure::Local(other.local().unwrap_or(LocalFailure::Crypto)),
        }
    }
}

/// A step that failed on this device reads as a local failure, its text
/// logged here. The relay's answer stays the one it was.
impl From<SyncError> for HttpError {
    fn from(error: SyncError) -> Self {
        match error {
            SyncError::Http(error) => error,
            other => {
                crate::log::warning!("Sync: {other}");
                Self::Local(other.local().unwrap_or(LocalFailure::Crypto))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    /// A malformed answer names the part that did not decode, so the log
    /// and the failure shown say which one.
    #[test]
    fn a_malformed_answer_names_its_part() {
        for (part, text) in [
            (Part::SigningKey, "the signing key"),
            (Part::SealedChannelKey, "the sealed channel key"),
            (Part::JoinHead, "the join head"),
        ] {
            assert_eq!(
                SyncError::Malformed(part).to_string(),
                format!("{text} does not decode")
            );
        }
    }

    use super::*;

    /// A relay refusal carried through a step keeps its code, and a failure
    /// on this device reads as local.
    #[test]
    fn a_step_keeps_the_relay_answer_it_met() {
        let refused = HttpError::refused(
            crate::http::Method::Delete,
            &crate::http::Target::relay("https://api.test/share"),
            ambre_core::HttpStatus::new(403),
            Some(ambre_core::RefusalCode::SetupRequired),
        );
        let step = SyncError::from(refused.clone());
        assert_eq!(step.failure(), refused.failure());
        assert_eq!(HttpError::from(step), refused);
        assert_eq!(
            SyncError::SessionLocked.failure(),
            ApiFailure::Local(LocalFailure::SessionLocked)
        );
        assert_eq!(
            HttpError::from(SyncError::Malformed(Part::JoinHead)).failure(),
            ApiFailure::Local(LocalFailure::Malformed)
        );
    }
}
