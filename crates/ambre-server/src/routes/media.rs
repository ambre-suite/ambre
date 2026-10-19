use axum::extract::{Path, State};
use axum::response::IntoResponse;
use std::sync::Arc;

use crate::auth;
use crate::db;
use crate::error::AppError;
use crate::extract::Json;
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
///
/// The client keys the address by the document key. The relay holds a name it
/// cannot compute from any file it has (`SRV-059`).
#[derive(serde::Deserialize)]
pub struct UploadUrlRequest {
    pub content_hash: String,
    /// Declared size of the encrypted blob. Validated against `media_max_bytes`
    /// and bound into the presigned PUT URL so the upload cannot exceed it.
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
    if !m.role.can_write_document() {
        return Err(AppError::Forbidden);
    }
    if !s3::is_valid_content_hash(&req.content_hash) {
        return Err(AppError::BadRequest("invalid content_hash".into()));
    }
    // Reject an oversized blob before handing out an upload URL. Bind the
    // declared size into the presigned PUT, so the actual upload cannot exceed
    // it.
    if req.size_bytes == 0 || req.size_bytes > state.config.media_max_bytes {
        return Err(AppError::PayloadTooLarge(format!(
            "media size {} bytes outside 1..={}",
            req.size_bytes, state.config.media_max_bytes
        )));
    }
    let key = s3::format_s3_media_key(&state.config.s3_env_prefix, &doc_id, &req.content_hash);
    let url = s3::generate_presigned_upload_url(
        &state.s3,
        &state.s3_bucket,
        &key,
        state.config.presigned_url_ttl_secs,
        req.size_bytes,
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
    // A link is signed only for an object the media index records in this
    // document, never for any well-formed address the caller supplies (`SRV-058`).
    if !db::document_holds_media(&state.db, &doc_id, &req.content_hash).await? {
        return Err(AppError::NotFound { resource: "media" });
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
/// linking the content hash to the S3 object. It names no media type
/// (`SYNC-053`).
#[derive(serde::Deserialize)]
pub struct ConfirmUploadRequest {
    pub content_hash: String,
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
    let m = db::get_document_member(&state.db, &doc_id, &s.user_id)
        .await?
        .ok_or(AppError::Forbidden)?;
    // Writing a `media` row must be gated exactly like `media_upload_url`
    // (owner/editor). A viewer cannot obtain an upload URL, so it must not be
    // able to register a media row either. Otherwise the two halves of the
    // upload flow would disagree on who may add media.
    if !m.role.can_write_document() {
        return Err(AppError::Forbidden);
    }
    if !s3::is_valid_content_hash(&req.content_hash) {
        return Err(AppError::BadRequest("invalid content_hash".into()));
    }
    // Mirror the upload-url cap so a recorded `media` row can never claim a size
    // the upload path would have refused.
    if req.size_bytes <= 0 || req.size_bytes as u64 > state.config.media_max_bytes {
        return Err(AppError::PayloadTooLarge(format!(
            "media size {} bytes outside 1..={}",
            req.size_bytes, state.config.media_max_bytes
        )));
    }
    let key = s3::format_s3_media_key(&state.config.s3_env_prefix, &doc_id, &req.content_hash);
    db::insert_media(
        &state.db,
        &doc_id,
        &s.user_id,
        &req.content_hash,
        req.size_bytes,
        &key,
    )
    .await?;
    Ok(axum::Json(MediaConfirmed {
        status: ambre_core::WriteStatus::Confirmed,
        content_hash: req.content_hash,
    }))
}

/// The answer of a media confirmation, naming the media it recorded.
#[derive(serde::Serialize)]
pub struct MediaConfirmed {
    pub status: ambre_core::WriteStatus,
    pub content_hash: String,
}

/// Erases the caller's upload of a blob (`SRV-139`).
///
/// Only a member who uploaded the blob may erase it (`SRV-140`). The object
/// goes with the last uploader's claim, so a second member who attached the
/// same file keeps it. Nothing else of the document is touched (`SRV-141`).
/// A failed object delete rolls the claim back, so the client's retry finds
/// it again.
pub async fn media_erase(
    State(state): State<Arc<AppState>>,
    auth: auth::AuthSession,
    Path((document_id, content_hash)): Path<(String, String)>,
) -> Result<impl IntoResponse, AppError> {
    let s = auth;
    let doc_id = parse_doc_id(&document_id)?;
    db::get_document_member(&state.db, &doc_id, &s.user_id)
        .await?
        .ok_or(AppError::Forbidden)?;
    if !s3::is_valid_content_hash(&content_hash) {
        return Err(AppError::BadRequest("invalid content_hash".into()));
    }
    let mut tx = state.db.begin().await?;
    match db::erase_media_claim(&mut tx, &doc_id, &s.user_id, &content_hash).await? {
        db::MediaErase::NotUploader => return Err(AppError::Forbidden),
        db::MediaErase::Kept => {}
        db::MediaErase::Last { s3_key } => {
            state
                .s3
                .delete_object()
                .bucket(&state.s3_bucket)
                .key(&s3_key)
                .send()
                .await
                .map_err(|e| AppError::S3(format!("delete {s3_key}: {e}")))?;
        }
    }
    tx.commit().await?;
    Ok(axum::Json(MediaErased {
        status: ambre_core::WriteStatus::Confirmed,
        content_hash,
    }))
}

/// The answer of an erase, naming the media the caller no longer holds.
#[derive(serde::Serialize)]
pub struct MediaErased {
    pub status: ambre_core::WriteStatus,
    pub content_hash: String,
}
