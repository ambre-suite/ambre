use axum::extract::{Json, Path, Query, State};
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
fn parse_snap_id(s: &str) -> Result<ambre_core::SnapshotId, AppError> {
    ambre_core::Uuid::parse_str(s)
        .map(ambre_core::SnapshotId::from_uuid)
        .map_err(|_| AppError::NotFound {
            resource: "snapshot",
        })
}

/// Summary of the latest snapshot (id + S3 key).
#[derive(serde::Serialize)]
pub struct SnapshotResponse {
    pub snapshot_id: String,
    pub s3_key: String,
    /// Highest delta `seq` folded into this snapshot, so the client can gate its
    /// post-open `RequestMissing` and skip the already-replayed history.
    pub covered_seq: i64,
}

pub async fn get_snapshot_metadata(
    State(state): State<Arc<AppState>>,
    auth: auth::AuthSession,
    Path(document_id): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    let s = auth;
    let doc_id = parse_doc_id(&document_id)?;
    db::get_document_member(&state.db, &doc_id, &s.user_id)
        .await?
        .ok_or(AppError::Forbidden)?;
    let snap = db::get_latest_snapshot(&state.db, &doc_id)
        .await?
        .ok_or(AppError::NotFound {
            resource: "snapshot",
        })?;
    Ok(axum::Json(SnapshotResponse {
        snapshot_id: snap.id.to_string(),
        s3_key: snap.s3_key,
        covered_seq: snap.covered_seq,
    }))
}

/// Request to confirm a snapshot upload. The server inserts metadata and
/// broadcasts `SnapshotAvailable` to all subscribers.
#[derive(serde::Deserialize)]
pub struct CreateSnapshotRequest {
    #[allow(dead_code)]
    pub snapshot_id: String,
    pub author_pubkey: String,
    pub parent_hash: Option<String>,
    pub parent_snapshot_proof: Option<String>,
    pub update_clocks: serde_json::Value,
    pub s3_key: String,
    /// Highest delta `seq` the snapshot author had applied at upload time.
    /// Defaults to 0 for older clients (full replay preserved).
    #[serde(default)]
    pub covered_seq: i64,
}
#[derive(serde::Serialize)]
pub struct CreateSnapshotResponse {
    pub snapshot_id: String,
}

pub async fn create_snapshot(
    State(state): State<Arc<AppState>>,
    auth: auth::AuthSession,
    Path(document_id): Path<String>,
    Json(req): Json<CreateSnapshotRequest>,
) -> Result<impl IntoResponse, AppError> {
    use base64::Engine;
    let s = auth;
    let doc_id = parse_doc_id(&document_id)?;
    let m = db::get_document_member(&state.db, &doc_id, &s.user_id)
        .await?
        .ok_or(AppError::Forbidden)?;
    if !["owner", "editor"].contains(&m.role.as_str()) {
        return Err(AppError::Forbidden);
    }

    let apk = hex::decode(&req.author_pubkey).map_err(|e| AppError::Conflict {
        detail: e.to_string(),
    })?;
    let ph = req
        .parent_hash
        .map(|h| {
            hex::decode(&h).map_err(|e| AppError::Conflict {
                detail: e.to_string(),
            })
        })
        .transpose()?;
    let pp = req
        .parent_snapshot_proof
        .map(|p| {
            base64::engine::general_purpose::URL_SAFE
                .decode(&p)
                .map_err(|e| AppError::Conflict {
                    detail: e.to_string(),
                })
        })
        .transpose()?;

    let sid = db::create_snapshot(
        &state.db,
        &doc_id,
        &apk,
        ph.as_deref(),
        pp.as_deref(),
        &req.update_clocks,
        &req.s3_key,
        req.covered_seq,
    )
    .await?;
    let msg = ambre_protocol::ws::encode_server_msg(
        &ambre_protocol::ws::WsServerMsg::SnapshotAvailable {
            document_id: doc_id,
            snapshot_id: sid,
        },
    );
    let _ = crate::redis_ops::doc_broadcast(&state.redis, &doc_id, &msg).await;
    Ok(axum::Json(CreateSnapshotResponse {
        snapshot_id: sid.to_string(),
    }))
}

/// Presigned PUT URL for direct client-to-S3 upload of an encrypted snapshot.
#[derive(serde::Serialize)]
pub struct SnapshotUploadUrlResponse {
    pub upload_url: String,
    pub s3_key: String,
}

pub async fn snapshot_upload_url(
    State(state): State<Arc<AppState>>,
    auth: auth::AuthSession,
    Path(document_id): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    let s = auth;
    let doc_id = parse_doc_id(&document_id)?;
    let m = db::get_document_member(&state.db, &doc_id, &s.user_id)
        .await?
        .ok_or(AppError::Forbidden)?;
    if !["owner", "editor"].contains(&m.role.as_str()) {
        return Err(AppError::Forbidden);
    }

    let snapshot_id = ambre_core::SnapshotId::new();
    let key = s3::format_s3_snapshot_key(&state.config.s3_env_prefix, &doc_id, &snapshot_id);
    let url = s3::generate_presigned_upload_url(
        &state.s3,
        &state.s3_bucket,
        &key,
        state.config.presigned_url_ttl_secs,
    )
    .await?;
    Ok(axum::Json(SnapshotUploadUrlResponse {
        upload_url: url,
        s3_key: key,
    }))
}

/// Request a presigned GET URL to download a specific snapshot blob.
#[derive(serde::Deserialize)]
pub struct SnapshotDownloadUrlRequest {
    pub snapshot_id: String,
}
#[derive(serde::Serialize)]
pub struct SnapshotDownloadUrlResponse {
    pub download_url: String,
}

pub async fn snapshot_download_url(
    State(state): State<Arc<AppState>>,
    auth: auth::AuthSession,
    Path(document_id): Path<String>,
    Json(req): Json<SnapshotDownloadUrlRequest>,
) -> Result<impl IntoResponse, AppError> {
    let s = auth;
    let doc_id = parse_doc_id(&document_id)?;
    db::get_document_member(&state.db, &doc_id, &s.user_id)
        .await?
        .ok_or(AppError::Forbidden)?;
    let snap_id = parse_snap_id(&req.snapshot_id)?;
    // Resolve the snapshot scoped to the path document: a member of doc A must
    // never obtain a download URL for a snapshot of doc B (cross-document IDOR).
    let snap = db::get_snapshot(&state.db, &doc_id, &snap_id)
        .await?
        .ok_or(AppError::NotFound {
            resource: "snapshot",
        })?;
    let url = s3::generate_presigned_download_url(
        &state.s3,
        &state.s3_bucket,
        &snap.s3_key,
        state.config.presigned_url_ttl_secs,
    )
    .await?;
    Ok(axum::Json(SnapshotDownloadUrlResponse {
        download_url: url,
    }))
}

/// Query parameters for delta retrieval (reconnect catch-up).
#[derive(serde::Deserialize)]
pub struct DeltasQuery {
    pub after_seq: Option<i64>,
    pub limit: Option<i64>,
}
/// One delta entry in the response list. Clients fetch the actual blob from S3.
#[derive(serde::Serialize)]
pub struct DeltaEntry {
    pub seq: i64,
    pub clock: i64,
    pub author_pubkey: String,
    pub s3_key: String,
}
#[derive(serde::Serialize)]
pub struct DeltasResponse {
    pub deltas: Vec<DeltaEntry>,
}

pub async fn get_deltas(
    State(state): State<Arc<AppState>>,
    auth: auth::AuthSession,
    Path(document_id): Path<String>,
    Query(params): Query<DeltasQuery>,
) -> Result<impl IntoResponse, AppError> {
    let s = auth;
    let doc_id = parse_doc_id(&document_id)?;
    db::get_document_member(&state.db, &doc_id, &s.user_id)
        .await?
        .ok_or(AppError::Forbidden)?;
    let after = params.after_seq.unwrap_or(0);
    let limit = params.limit.unwrap_or(100);
    let updates = db::get_updates_after(&state.db, &doc_id, after, limit).await?;
    let deltas: Vec<_> = updates
        .into_iter()
        .map(|u| DeltaEntry {
            seq: u.seq,
            clock: u.clock,
            author_pubkey: hex::encode(&u.author_pubkey),
            s3_key: u.s3_key,
        })
        .collect();
    Ok(axum::Json(DeltasResponse { deltas }))
}
