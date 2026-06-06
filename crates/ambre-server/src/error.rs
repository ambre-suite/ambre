use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
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

    #[error("Session expired")]
    SessionExpired,

    #[error("Forbidden: insufficient role")]
    Forbidden,

    #[error("Not found: {resource}")]
    NotFound { resource: &'static str },

    #[error("Conflict: {detail}")]
    Conflict { detail: String },

    #[error("Clock mismatch: expected {expected}, got {got}")]
    ClockMismatch { expected: u64, got: u64 },

    #[error("Invalid signature")]
    InvalidSignature,

    #[error("Snapshot required")]
    SnapshotRequired,

    #[error("Compaction lock held")]
    CompactionLockHeld,

    #[error("Unknown OIDC provider: {0}")]
    UnknownProvider(String),

    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("Redis error: {0}")]
    Redis(String),

    #[error("S3 error: {0}")]
    S3(String),

    #[error("Postcard decode error: {0}")]
    Decode(#[from] postcard::Error),

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
            AppError::InvalidSignature => {
                (StatusCode::UNPROCESSABLE_ENTITY, "INVALID_SIGNATURE")
            }
            AppError::SnapshotRequired => (StatusCode::CONFLICT, "SNAPSHOT_REQUIRED"),
            AppError::CompactionLockHeld => {
                (StatusCode::CONFLICT, "COMPACTION_LOCK_HELD")
            }
            AppError::UnknownProvider(_) => (StatusCode::BAD_REQUEST, "UNKNOWN_PROVIDER"),
            _ => (StatusCode::INTERNAL_SERVER_ERROR, "INTERNAL_ERROR"),
        };

        tracing::error!(error = %self, code = code);

        let message = match &self {
            AppError::Database(_) | AppError::Redis(_) | AppError::S3(_) => {
                "Internal server error".to_string()
            }
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
