use axum::extract::State;
use axum::response::IntoResponse;
use std::sync::Arc;

use ambre_core::id::{Document, Id};

use crate::auth;
use crate::db;
use crate::error::{AppError, Resource};
use crate::extract::{Json, PathId};
use crate::s3;
use crate::state::AppState;

/// Request for a presigned S3 upload URL. The client uploads the encrypted
/// blob directly; the server inserts the `media` row on confirmation.
///
/// The client keys the address by the document key. The relay holds a name it
/// cannot compute from any file it has (`SRV-059`).
#[derive(serde::Deserialize)]
pub struct UploadUrlRequest {
    pub content_hash: ambre_core::Validated<ambre_core::ContentHash>,
    /// Declared size of the encrypted blob. Validated against `media_max_size`
    /// and bound into the presigned PUT URL so the upload cannot exceed it.
    #[serde(rename = "size_bytes")]
    pub size: ambre_core::Count<ambre_core::Octets>,
}
#[derive(serde::Serialize)]
pub struct UploadUrlResponse {
    pub upload_url: ambre_core::Credential<ambre_core::PresignedUrl>,
    pub s3_key: ambre_core::ObjectKey,
}

pub async fn media_upload_url(
    State(state): State<Arc<AppState>>,
    auth: auth::AuthSession,
    ids: PathId<Id<Document>>,
    body: Json<UploadUrlRequest>,
) -> Result<impl IntoResponse, AppError> {
    let doc_id = ids.into_inner();
    let req = body.into_inner();
    let s = auth;
    let m = db::get_document_member(&state.db, &doc_id, &s.user_id)
        .await?
        .ok_or(AppError::Forbidden)?;
    if !m.role.can_write_document() {
        return Err(AppError::Forbidden);
    }
    // Reject an oversized blob before handing out an upload URL. Bind the
    // declared size into the presigned PUT, so the actual upload cannot exceed
    // it.
    if req.size == ambre_core::Count::<ambre_core::Octets>::new(0)
        || req.size > state.config.media_max_size
    {
        return Err(AppError::PayloadTooLarge(format!(
            "media size {} bytes outside 1..={}",
            req.size.get(),
            state.config.media_max_size.get()
        )));
    }
    // A held address already names a blob other members read. A second link
    // would let this writer replace it, and each reader would then fail the
    // address check (`SRV-271`, audit `P16`). The client confirms its claim
    // without a put on this refusal (`SYNC-228`).
    if db::document_holds_media(&state.db, &doc_id, &req.content_hash).await? {
        return Err(AppError::Conflict {
            detail: "the document already holds this media".to_owned(),
        });
    }
    let key = s3::format_s3_media_key(&state.config.s3_env_prefix, &doc_id, &req.content_hash);
    let url = s3::generate_presigned_upload_url(
        &state.s3,
        &state.s3_bucket,
        &key,
        state.config.presigned_url_ttl,
        req.size,
        // The document key names a media address, not the digest of its bytes.
        // The client asks for no checksum of a media blob yet (audit `N26`).
        None,
        state.config.s3_public_endpoint.as_ref(),
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
    pub content_hash: ambre_core::Validated<ambre_core::ContentHash>,
}
#[derive(serde::Serialize)]
pub struct DownloadUrlResponse {
    pub download_url: ambre_core::Credential<ambre_core::PresignedUrl>,
}

pub async fn media_download_url(
    State(state): State<Arc<AppState>>,
    auth: auth::AuthSession,
    ids: PathId<Id<Document>>,
    body: Json<DownloadUrlRequest>,
) -> Result<impl IntoResponse, AppError> {
    let doc_id = ids.into_inner();
    let req = body.into_inner();
    let s = auth;
    db::get_document_member(&state.db, &doc_id, &s.user_id)
        .await?
        .ok_or(AppError::Forbidden)?;
    // A link is signed only for an object the media index records in this
    // document, never for any well-formed address the caller supplies (`SRV-058`).
    if !db::document_holds_media(&state.db, &doc_id, &req.content_hash).await? {
        return Err(AppError::NotFound {
            resource: Resource::Media,
        });
    }
    let key = s3::format_s3_media_key(&state.config.s3_env_prefix, &doc_id, &req.content_hash);
    let url = s3::generate_presigned_download_url(
        &state.s3,
        &state.s3_bucket,
        &key,
        state.config.presigned_url_ttl,
        state.config.s3_public_endpoint.as_ref(),
    )
    .await?;
    Ok(axum::Json(DownloadUrlResponse { download_url: url }))
}

/// Confirmation that an upload completed. The server inserts a `media` row
/// linking the content hash to the S3 object. It names no media type
/// (`SYNC-053`). An address the document does not hold yet is refused with a
/// 404 while no object fills it (`SRV-292`).
#[derive(serde::Deserialize)]
pub struct ConfirmUploadRequest {
    pub content_hash: ambre_core::Validated<ambre_core::ContentHash>,
    #[serde(rename = "size_bytes")]
    pub size: ambre_core::Count<ambre_core::Octets>,
}

pub async fn media_confirm_upload(
    State(state): State<Arc<AppState>>,
    auth: auth::AuthSession,
    ids: PathId<Id<Document>>,
    body: Json<ConfirmUploadRequest>,
) -> Result<impl IntoResponse, AppError> {
    let doc_id = ids.into_inner();
    let req = body.into_inner();
    let s = auth;
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
    // Mirror the upload-url cap so a recorded `media` row can never claim a size
    // the upload path would have refused.
    if req.size == ambre_core::Count::<ambre_core::Octets>::new(0)
        || req.size > state.config.media_max_size
    {
        return Err(AppError::PayloadTooLarge(format!(
            "media size {} bytes outside 1..={}",
            req.size.get(),
            state.config.media_max_size.get()
        )));
    }
    let key = s3::format_s3_media_key(&state.config.s3_env_prefix, &doc_id, &req.content_hash);
    // A held address takes a second claim without a put (`SYNC-228`). Any
    // other address is recorded only once its object exists. A row with no
    // object would refuse the address to every member for good (`SRV-271`,
    // `SRV-292`).
    if !db::document_holds_media(&state.db, &doc_id, &req.content_hash).await?
        && !s3::holds_object(&state.s3, &state.s3_bucket, &key).await?
    {
        return Err(AppError::NotFound {
            resource: Resource::Media,
        });
    }
    db::insert_media(
        &state.db,
        &doc_id,
        &s.user_id,
        &req.content_hash,
        req.size,
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
    pub content_hash: ambre_core::Validated<ambre_core::ContentHash>,
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
    ids: PathId<(Id<Document>, ambre_core::Validated<ambre_core::ContentHash>)>,
) -> Result<impl IntoResponse, AppError> {
    let (doc_id, content_hash) = ids.into_inner();
    let s = auth;
    db::get_document_member(&state.db, &doc_id, &s.user_id)
        .await?
        .ok_or(AppError::Forbidden)?;
    let mut tx = state.db.begin().await?;
    match db::erase_media_claim(&mut tx, &doc_id, &s.user_id, &content_hash).await? {
        db::MediaErase::NotUploader => return Err(AppError::Forbidden),
        db::MediaErase::Kept => {}
        db::MediaErase::Last { s3_key } => {
            state
                .s3
                .delete_object()
                .bucket(state.s3_bucket.as_str())
                .key(&s3_key)
                .send()
                .await
                .map_err(|e| {
                    AppError::S3(crate::error::FaultText::of("delete a media object", e))
                })?;
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
    pub content_hash: ambre_core::Validated<ambre_core::ContentHash>,
}
