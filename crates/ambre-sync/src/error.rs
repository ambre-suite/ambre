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

use ambre_bus::{ApiFailure, DocumentId, UserId};

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
    PinMismatch(UserId),
    /// The relay created another document than the one this client minted
    /// (`SYNC-136`).
    #[error("the relay answered {answered}, not the minted {minted}")]
    WrongDocument {
        /// The id this client minted.
        minted: DocumentId,
        /// The id the relay answered.
        answered: DocumentId,
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
    /// A member's X25519 exchange key.
    ExchangeKey,
    /// A member's Ed25519 signing key.
    SigningKey,
    /// A wrapped copy of a document key.
    WrappedKey,
    /// A channel key sealed for a space's catalogue.
    SealedChannelKey,
    /// The join head of a public channel, which names no last entry.
    JoinHead,
    /// The address of a media blob.
    MediaAddress,
}

impl std::fmt::Display for Part {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::ExchangeKey => "the exchange key",
            Self::SigningKey => "the signing key",
            Self::WrappedKey => "the wrapped key",
            Self::SealedChannelKey => "the sealed channel key",
            Self::JoinHead => "the join head",
            Self::MediaAddress => "the media address",
        })
    }
}

impl From<HttpError> for SyncError {
    fn from(error: HttpError) -> Self {
        Self::Http(error)
    }
}

impl Reported for SyncError {
    fn failure(&self) -> ApiFailure {
        match self {
            Self::Http(error) => error.failure(),
            _ => ApiFailure::Local,
        }
    }
}

/// A step that failed on this device reads as a local failure. The relay's
/// answer stays the one it was.
impl From<SyncError> for HttpError {
    fn from(error: SyncError) -> Self {
        match error {
            SyncError::Http(error) => error,
            other => Self::Local(other.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A relay refusal carried through a step keeps its code, and a failure
    /// on this device reads as local.
    #[test]
    fn a_step_keeps_the_relay_answer_it_met() {
        let refused = HttpError::refused(
            crate::http::Method::Delete,
            &crate::http::Target::relay("https://api.test/share"),
            ambre_core::HttpStatus(403),
            Some(ambre_core::RefusalCode::SetupRequired),
        );
        let step = SyncError::from(refused.clone());
        assert_eq!(step.failure(), refused.failure());
        assert_eq!(HttpError::from(step), refused);
        assert_eq!(SyncError::SessionLocked.failure(), ApiFailure::Local);
        assert_eq!(
            HttpError::from(SyncError::Malformed(Part::WrappedKey)).failure(),
            ApiFailure::Local
        );
    }
}
