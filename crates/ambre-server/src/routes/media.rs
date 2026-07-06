use axum::extract::{Json, Path, State};
use axum::response::IntoResponse;
use std::sync::Arc;

use crate::auth;
use crate::db;
use crate::error::AppError;
use crate::s3;
use crate::state::AppState;

fn parse_doc_id(s: &str) -> Result<ambre_core::DocumentId, AppError> {
    ambre_core::Uuid::parse_str(s)
        .map(ambre_core::DocumentId::from_uuid)
        .map_err(|_| AppError::NotFound {
            resource: "document",
        })
}

/// Request for a presigned S3 upload URL. The client uploads the encrypted
/// blob directly; the server inserts the `media` row on confirmation.
#[derive(serde::Deserialize)]
pub struct UploadUrlRequest {
    pub content_hash: String,
    #[allow(dead_code)]
    pub mime_type: String,
    #[allow(dead_code)]
    pub size_bytes: u64,
}
#[derive(serde::Serialize)]
pub struct UploadUrlResponse {
    pub upload_url: String,
    pub s3_key: String,
}

pub async fn media_upload_url(
    State(state): State<Arc<AppState>>,
    auth: auth::AuthSession,
    Path(document_id): Path<String>,
    Json(req): Json<UploadUrlRequest>,
) -> Result<impl IntoResponse, AppError> {
    let s = auth;
    let doc_id = parse_doc_id(&document_id)?;
    let m = db::get_document_member(&state.db, &doc_id, &s.user_id)
        .await?
        .ok_or(AppError::Forbidden)?;
    if !["owner", "editor"].contains(&m.role.as_str()) {
        return Err(AppError::Forbidden);
    }
    if !s3::is_valid_content_hash(&req.content_hash) {
        return Err(AppError::BadRequest("invalid content_hash".into()));
    }
    let key = s3::format_s3_media_key(&state.config.s3_env_prefix, &doc_id, &req.content_hash);
    let url = s3::generate_presigned_upload_url(
        &state.s3,
        &state.s3_bucket,
        &key,
        state.config.presigned_url_ttl_secs,
    )
    .await?;
    Ok(axum::Json(UploadUrlResponse {
        upload_url: url,
        s3_key: key,
    }))
}

/// Request for a presigned S3 download URL by content hash.
#[derive(serde::Deserialize)]
pub struct DownloadUrlRequest {
    pub content_hash: String,
}
#[derive(serde::Serialize)]
pub struct DownloadUrlResponse {
    pub download_url: String,
}

pub async fn media_download_url(
    State(state): State<Arc<AppState>>,
    auth: auth::AuthSession,
    Path(document_id): Path<String>,
    Json(req): Json<DownloadUrlRequest>,
) -> Result<impl IntoResponse, AppError> {
    let s = auth;
    let doc_id = parse_doc_id(&document_id)?;
    db::get_document_member(&state.db, &doc_id, &s.user_id)
        .await?
        .ok_or(AppError::Forbidden)?;
    if !s3::is_valid_content_hash(&req.content_hash) {
        return Err(AppError::BadRequest("invalid content_hash".into()));
    }
    let key = s3::format_s3_media_key(&state.config.s3_env_prefix, &doc_id, &req.content_hash);
    let url = s3::generate_presigned_download_url(
        &state.s3,
        &state.s3_bucket,
        &key,
        state.config.presigned_url_ttl_secs,
    )
    .await?;
    Ok(axum::Json(DownloadUrlResponse { download_url: url }))
}

/// Confirmation that an upload completed. The server inserts a `media` row
/// linking the content hash to the S3 object.
#[derive(serde::Deserialize)]
pub struct ConfirmUploadRequest {
    pub content_hash: String,
    pub mime_type: String,
    pub size_bytes: i64,
}

pub async fn media_confirm_upload(
    State(state): State<Arc<AppState>>,
    auth: auth::AuthSession,
    Path(document_id): Path<String>,
    Json(req): Json<ConfirmUploadRequest>,
) -> Result<impl IntoResponse, AppError> {
    let s = auth;
    let doc_id = parse_doc_id(&document_id)?;
    db::get_document_member(&state.db, &doc_id, &s.user_id)
        .await?
        .ok_or(AppError::Forbidden)?;
    if !s3::is_valid_content_hash(&req.content_hash) {
        return Err(AppError::BadRequest("invalid content_hash".into()));
    }
    let key = s3::format_s3_media_key(&state.config.s3_env_prefix, &doc_id, &req.content_hash);
    db::insert_media(
        &state.db,
        &doc_id,
        &s.user_id,
        &req.content_hash,
        &req.mime_type,
        req.size_bytes,
        &key,
    )
    .await?;
    Ok(axum::Json(
        serde_json::json!({"status":"confirmed","content_hash":req.content_hash}),
    ))
}
