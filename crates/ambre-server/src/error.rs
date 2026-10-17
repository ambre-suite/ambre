use ambre_core::RefusalCode;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
#[allow(clippy::single_component_path_imports)]
use tracing;

/// Typed error covering all server failure modes.
///
/// Infrastructure errors (`Database`, `Redis`, `S3`) log the real cause
/// but return a generic "Internal server error" to the caller.
/// Application errors map to proper HTTP status codes and are forwarded.
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("Unauthorized")]
    Unauthorized,

    #[error("Bad request: {0}")]
    BadRequest(String),

    #[error("Payload too large: {0}")]
    PayloadTooLarge(String),

    #[error("Session expired")]
    SessionExpired,

    #[error("Forbidden: insufficient role")]
    Forbidden,

    /// The session belongs to an account that has not registered its keys
    /// yet. Only the key setup routes accept it (`SRV-027`).
    #[error("Key setup required")]
    SetupRequired,

    #[error("Not found: {resource}")]
    NotFound { resource: &'static str },

    #[error("Conflict: {detail}")]
    Conflict { detail: String },

    #[allow(dead_code)]
    #[error("Clock mismatch: expected {expected}, got {got}")]
    ClockMismatch { expected: u64, got: u64 },

    /// The document owes a rotation, so a frame under any key it held is
    /// refused (`SRV-123`).
    #[error("Rotation owed")]
    RotationOwed,

    /// A member joined after the rotation read the members, so it would hold
    /// no copy of the new key (`SRV-167`). The client reads them again.
    #[error("The members changed since the rotation read them")]
    MembersChanged,

    /// The key an invitation wraps is no longer the document's current one
    /// (`SRV-168`).
    #[error("The document key changed since the invitation wrapped it")]
    KeyChanged,

    /// Another membership entry took the place this one was signed for
    /// (`SRV-104`). The client reads the log again and signs after it.
    #[error("The membership log moved since the entry was signed")]
    LogMoved,

    #[error("Invalid signature")]
    InvalidSignature,

    #[allow(dead_code)]
    #[error("Snapshot required")]
    SnapshotRequired,

    #[allow(dead_code)]
    #[error("Compaction lock held")]
    CompactionLockHeld,

    #[allow(dead_code)]
    #[error("Unknown OIDC provider: {0}")]
    UnknownProvider(String),

    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("Redis error: {0}")]
    Redis(String),

    #[error("S3 error: {0}")]
    S3(String),

    #[error("Internal error: {0}")]
    Internal(String),

    #[error("Postcard decode error: {0}")]
    Decode(#[from] postcard::Error),

    #[allow(dead_code)]
    #[error("OIDC error: {0}")]
    Oidc(String),
}

impl AppError {
    /// Whether the error comes from a store or a dependency rather than from
    /// the caller's request.
    pub(crate) fn is_infrastructure(&self) -> bool {
        matches!(
            self,
            AppError::Database(_)
                | AppError::Redis(_)
                | AppError::S3(_)
                | AppError::Decode(_)
                | AppError::Oidc(_)
                | AppError::Internal(_)
        )
    }

    /// The code a refusal of this error names, on the request path and on the
    /// socket alike (`SRV-182`, `SRV-183`). Every variant chooses one.
    pub(crate) fn code(&self) -> RefusalCode {
        match self {
            AppError::Unauthorized | AppError::SessionExpired => RefusalCode::Unauthorized,
            AppError::Forbidden => RefusalCode::Forbidden,
            AppError::SetupRequired => RefusalCode::SetupRequired,
            AppError::NotFound { .. } => RefusalCode::NotFound,
            AppError::Conflict { .. } => RefusalCode::Conflict,
            AppError::ClockMismatch { .. } => RefusalCode::ClockMismatch,
            AppError::RotationOwed => RefusalCode::RotationOwed,
            AppError::MembersChanged => RefusalCode::MembersChanged,
            AppError::KeyChanged => RefusalCode::KeyChanged,
            AppError::LogMoved => RefusalCode::LogMoved,
            AppError::InvalidSignature => RefusalCode::InvalidSignature,
            AppError::SnapshotRequired => RefusalCode::SnapshotRequired,
            AppError::CompactionLockHeld => RefusalCode::CompactionLockHeld,
            AppError::UnknownProvider(_) => RefusalCode::UnknownProvider,
            AppError::BadRequest(_) => RefusalCode::BadRequest,
            AppError::PayloadTooLarge(_) => RefusalCode::PayloadTooLarge,
            AppError::Database(_)
            | AppError::Redis(_)
            | AppError::S3(_)
            | AppError::Internal(_)
            | AppError::Decode(_)
            | AppError::Oidc(_) => RefusalCode::InternalError,
        }
    }

    /// The text a caller reads, on the request path and on the socket alike.
    /// An infrastructure error says only that the server failed, and its cause
    /// stays in the log (`SRV-096`).
    pub(crate) fn caller_message(&self) -> String {
        if self.is_infrastructure() {
            "Internal server error".to_string()
        } else {
            self.to_string()
        }
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let code = self.code();
        let status =
            StatusCode::from_u16(code.status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);

        // Log by severity. A 5xx is a genuine server fault (Redis/DB/S3/...) and
        // must be loud. A 4xx is client-caused (bad auth, not found, conflict)
        // and is routine, so it stays at debug to keep `error` meaningful.
        if status.is_server_error() {
            tracing::error!(error = %self, code = %code);
        } else {
            tracing::debug!(error = %self, code = %code);
        }

        let message = self.caller_message();

        (status, ErrorBody::json(code, message)).into_response()
    }
}

/// The body of every refusal: the code a client matches, and a message for
/// a person (`SRV-181`). The code is a word of one closed set (`SRV-182`).
#[derive(serde::Serialize)]
pub(crate) struct ErrorBody {
    pub error: RefusalCode,
    pub message: String,
}

impl ErrorBody {
    /// The refusal under `code`.
    pub(crate) fn json(code: RefusalCode, message: impl Into<String>) -> axum::Json<Self> {
        axum::Json(Self {
            error: code,
            message: message.into(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;

    async fn extract_body(resp: Response) -> serde_json::Value {
        let body = resp.into_body();
        let bytes = to_bytes(body, usize::MAX).await.unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    #[tokio::test]
    async fn unauthorized_returns_401() {
        let r = AppError::Unauthorized.into_response();
        assert_eq!(r.status(), StatusCode::UNAUTHORIZED);
        let b = extract_body(r).await;
        assert_eq!(b["error"], "UNAUTHORIZED");
    }

    #[tokio::test]
    async fn session_expired_returns_401() {
        let r = AppError::SessionExpired.into_response();
        assert_eq!(r.status(), StatusCode::UNAUTHORIZED);
        let b = extract_body(r).await;
        assert_eq!(b["error"], "UNAUTHORIZED");
    }

    #[tokio::test]
    async fn forbidden_returns_403() {
        let r = AppError::Forbidden.into_response();
        assert_eq!(r.status(), StatusCode::FORBIDDEN);
        let b = extract_body(r).await;
        assert_eq!(b["error"], "FORBIDDEN");
    }

    #[tokio::test]
    async fn not_found_returns_404() {
        let r = AppError::NotFound {
            resource: "document",
        }
        .into_response();
        assert_eq!(r.status(), StatusCode::NOT_FOUND);
        let b = extract_body(r).await;
        assert_eq!(b["error"], "NOT_FOUND");
    }

    #[tokio::test]
    async fn conflict_returns_409() {
        let r = AppError::Conflict {
            detail: "already exists".into(),
        }
        .into_response();
        assert_eq!(r.status(), StatusCode::CONFLICT);
        let b = extract_body(r).await;
        assert_eq!(b["error"], "CONFLICT");
    }

    #[tokio::test]
    async fn clock_mismatch_returns_409() {
        let r = AppError::ClockMismatch {
            expected: 5,
            got: 7,
        }
        .into_response();
        assert_eq!(r.status(), StatusCode::CONFLICT);
        let b = extract_body(r).await;
        assert_eq!(b["error"], "CLOCK_MISMATCH");
    }

    #[tokio::test]
    async fn invalid_signature_returns_422() {
        let r = AppError::InvalidSignature.into_response();
        assert_eq!(r.status(), StatusCode::UNPROCESSABLE_ENTITY);
        let b = extract_body(r).await;
        assert_eq!(b["error"], "INVALID_SIGNATURE");
    }

    #[test]
    fn snapshot_required_returns_409() {
        let r = AppError::SnapshotRequired.into_response();
        assert_eq!(r.status(), StatusCode::CONFLICT);
    }

    #[test]
    fn compaction_lock_held_returns_409() {
        let r = AppError::CompactionLockHeld.into_response();
        assert_eq!(r.status(), StatusCode::CONFLICT);
    }

    #[test]
    fn unknown_provider_returns_400() {
        let r = AppError::UnknownProvider("gitlab".into()).into_response();
        assert_eq!(r.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn payload_too_large_returns_413() {
        let r = AppError::PayloadTooLarge("blob exceeds 100 MiB".into()).into_response();
        assert_eq!(r.status(), StatusCode::PAYLOAD_TOO_LARGE);
        let b = extract_body(r).await;
        assert_eq!(b["error"], "PAYLOAD_TOO_LARGE");
    }

    #[tokio::test]
    async fn infrastructure_errors_return_internal_server_error_message() {
        for err in [
            AppError::Database(sqlx::Error::PoolTimedOut),
            AppError::Redis("connection refused".into()),
            AppError::S3("access denied".into()),
            AppError::Decode(postcard::Error::SerdeDeCustom),
        ] {
            let r = err.into_response();
            assert_eq!(r.status(), StatusCode::INTERNAL_SERVER_ERROR);
            let b = extract_body(r).await;
            assert_eq!(b["error"], "INTERNAL_ERROR");
            assert_eq!(b["message"], "Internal server error");
        }
    }

    #[tokio::test]
    async fn application_errors_return_their_message() {
        let r = AppError::NotFound { resource: "user" }.into_response();
        let b = extract_body(r).await;
        assert_eq!(b["message"], "Not found: user");
    }

    #[tokio::test]
    async fn oidc_error_is_infrastructure_level() {
        let r = AppError::Oidc("provider timeout".into()).into_response();
        assert_eq!(r.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let b = extract_body(r).await;
        assert_eq!(b["error"], "INTERNAL_ERROR");
        assert_eq!(b["message"], "Internal server error");
    }

    #[test]
    fn database_from_sqlx_error() {
        use sqlx::Error as SqlxError;
        let e: AppError = SqlxError::PoolTimedOut.into();
        assert!(matches!(e, AppError::Database(_)));
    }

    #[test]
    fn decode_from_postcard_error() {
        let e: AppError = postcard::Error::SerdeDeCustom.into();
        assert!(matches!(e, AppError::Decode(_)));
    }
}
