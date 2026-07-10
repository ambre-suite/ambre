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

    #[error("Not found: {resource}")]
    NotFound { resource: &'static str },

    #[error("Conflict: {detail}")]
    Conflict { detail: String },

    #[allow(dead_code)]
    #[error("Clock mismatch: expected {expected}, got {got}")]
    ClockMismatch { expected: u64, got: u64 },

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

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, code) = match &self {
            AppError::Unauthorized | AppError::SessionExpired => {
                (StatusCode::UNAUTHORIZED, "UNAUTHORIZED")
            }
            AppError::Forbidden => (StatusCode::FORBIDDEN, "FORBIDDEN"),
            AppError::NotFound { .. } => (StatusCode::NOT_FOUND, "NOT_FOUND"),
            AppError::Conflict { .. } => (StatusCode::CONFLICT, "CONFLICT"),
            AppError::ClockMismatch { .. } => (StatusCode::CONFLICT, "CLOCK_MISMATCH"),
            AppError::InvalidSignature => (StatusCode::UNPROCESSABLE_ENTITY, "INVALID_SIGNATURE"),
            AppError::SnapshotRequired => (StatusCode::CONFLICT, "SNAPSHOT_REQUIRED"),
            AppError::CompactionLockHeld => (StatusCode::CONFLICT, "COMPACTION_LOCK_HELD"),
            AppError::UnknownProvider(_) => (StatusCode::BAD_REQUEST, "UNKNOWN_PROVIDER"),
            AppError::BadRequest(_) => (StatusCode::BAD_REQUEST, "BAD_REQUEST"),
            AppError::PayloadTooLarge(_) => (StatusCode::PAYLOAD_TOO_LARGE, "PAYLOAD_TOO_LARGE"),
            _ => (StatusCode::INTERNAL_SERVER_ERROR, "INTERNAL_ERROR"),
        };

        // Log by severity: a 5xx is a genuine server fault (Redis/DB/S3/...) and
        // must be loud; a 4xx is client-caused (bad auth, not found, conflict)
        // and is routine, so it stays at debug to keep `error` meaningful.
        if status.is_server_error() {
            tracing::error!(error = %self, code = code);
        } else {
            tracing::debug!(error = %self, code = code);
        }

        let message = match &self {
            AppError::Database(_)
            | AppError::Redis(_)
            | AppError::S3(_)
            | AppError::Decode(_)
            | AppError::Oidc(_)
            | AppError::Internal(_) => "Internal server error".to_string(),
            other => other.to_string(),
        };

        (
            status,
            axum::Json(serde_json::json!({
                "error": code,
                "message": message,
            })),
        )
            .into_response()
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
