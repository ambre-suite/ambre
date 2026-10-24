use axum::extract::{Query, State};
use axum::response::IntoResponse;
use std::sync::Arc;

use crate::auth;
use crate::db;
use crate::error::{AppError, Resource};
use crate::extract::{Json, PathId};
use crate::s3;
use crate::state::AppState;

/// Summary of the latest snapshot (id + S3 key).
#[derive(serde::Serialize)]
pub struct SnapshotResponse {
    #[serde(with = "ambre_core::serde_id::snapshot_id")]
    pub snapshot_id: ambre_core::SnapshotId,
    pub s3_key: ambre_core::ObjectKey,
    /// Highest delta `seq` folded into this snapshot, so the client can gate its
    /// post-open `RequestMissing` and skip the already-replayed history.
    pub covered_seq: ambre_core::Seq,
    /// Who vouched for it: `confirmed` or `self` (`SRV-128`).
    pub status: db::SnapshotStatus,
    /// The author's signing key, hex, which a self-confirmed snapshot is checked
    /// against (`AUTHOR-022`).
    pub author_pubkey: ambre_core::SigningKeyHex,
    /// For a self-confirmed snapshot, the reference snapshot an open starts
    /// from, `None` when no member other than an author confirmed one yet.
    pub reference: Option<ReferenceSnapshot>,
}

/// The reference snapshot named beside a self-confirmed one (`SRV-131`).
#[derive(serde::Serialize)]
pub struct ReferenceSnapshot {
    #[serde(with = "ambre_core::serde_id::snapshot_id")]
    pub snapshot_id: ambre_core::SnapshotId,
    pub covered_seq: ambre_core::Seq,
}

pub async fn get_snapshot_metadata(
    State(state): State<Arc<AppState>>,
    auth: auth::AuthSession,
    PathId(doc_id): PathId<ambre_core::DocumentId>,
) -> Result<impl IntoResponse, AppError> {
    let s = auth;
    db::get_document_member(&state.db, &doc_id, &s.user_id)
        .await?
        .ok_or(AppError::Forbidden)?;
    let snap = db::get_latest_snapshot(&state.db, &doc_id)
        .await?
        .ok_or(AppError::NotFound {
            resource: Resource::Snapshot,
        })?;
    let reference = match snap.status {
        db::SnapshotStatus::SelfConfirmed => db::get_reference_snapshot(&state.db, &doc_id)
            .await?
            .map(|r| ReferenceSnapshot {
                snapshot_id: r.id,
                covered_seq: r.covered_seq,
            }),
        _ => None,
    };
    let snap_id = snap.id;
    Ok(axum::Json(SnapshotResponse {
        snapshot_id: snap_id,
        s3_key: snap.s3_key,
        covered_seq: snap.covered_seq,
        status: snap.status,
        author_pubkey: ambre_core::SigningKeyHex::encode(&snap.author_pubkey),
        reference,
    }))
}

/// The answer to a confirmation: the amendments document it erased, if the
/// snapshot folded one in (`SRV-211`).
#[derive(Debug, serde::Serialize)]
pub struct ConfirmedSnapshotJson {
    #[serde(with = "ambre_core::serde_id::document_id::option")]
    pub folded: Option<ambre_core::DocumentId>,
}

/// Records that the caller checked a snapshot and vouches for it (`SRV-126`).
/// Any member may confirm, since any member holds the deltas. The author's
/// own confirmation makes it `self`, anyone else's `confirmed`.
///
/// A `confirmed` snapshot that folds amendments in becomes the final one of
/// its segment (`SRV-208`), and the snapshots it replaces go (`SRV-209`). The
/// amendments are sealed and deleted, unless a delta came past the fold
/// (`SRV-211`).
pub async fn confirm_snapshot(
    State(state): State<Arc<AppState>>,
    auth: auth::AuthSession,
    PathId((doc_id, snap_id)): PathId<(ambre_core::DocumentId, ambre_core::SnapshotId)>,
) -> Result<axum::Json<ConfirmedSnapshotJson>, AppError> {
    db::get_document_member(&state.db, &doc_id, &auth.user_id)
        .await?
        .ok_or(AppError::Forbidden)?;
    let confirmer = &auth.signing_pubkey;
    let status = db::confirm_snapshot(&state.db, &doc_id, &snap_id, confirmer)
        .await?
        .ok_or(AppError::NotFound {
            resource: Resource::Snapshot,
        })?;
    if status != db::SnapshotStatus::Confirmed {
        return Ok(axum::Json(ConfirmedSnapshotJson { folded: None }));
    }
    let Some(fold) = db::settle_fold(&state.db, &doc_id, &snap_id).await? else {
        return Ok(axum::Json(ConfirmedSnapshotJson { folded: None }));
    };
    for key in &fold.dropped_keys {
        let deleted = state
            .s3
            .delete_object()
            .bucket(state.s3_bucket.as_str())
            .key(key);
        if let Err(e) = deleted.send().await {
            tracing::warn!(s3_key = %key, error = %e, "replaced snapshot: S3 delete failed");
        }
    }
    if !crate::redis_ops::seal_folded(&state.redis, &fold.amendments, fold.folded_seq).await? {
        return Ok(axum::Json(ConfirmedSnapshotJson { folded: None }));
    }
    crate::deletion::delete_document(&state, &fold.amendments).await?;
    tracing::info!(document_id = %doc_id, amendments = %fold.amendments, "folded amendments erased");
    Ok(axum::Json(ConfirmedSnapshotJson {
        folded: Some(fold.amendments),
    }))
}

/// A signed mismatch report (`AUTHOR-023`).
#[derive(serde::Deserialize)]
pub struct ReportSnapshotRequest {
    /// The reporter's Ed25519 signature over
    /// [`ambre_protocol::snapshot_report::report_transcript`], hex.
    pub signature: ambre_core::SignatureHex,
}

/// Drops a pending snapshot a member found at odds with its deltas (`SRV-133`).
/// Every delta stays (`SRV-135`), and the reporter is asked for the next
/// snapshot (`SRV-134`). A snapshot somebody confirmed stays: one report
/// does not outweigh a check that passed.
pub async fn report_snapshot(
    State(state): State<Arc<AppState>>,
    auth: auth::AuthSession,
    PathId((doc_id, snap_id)): PathId<(ambre_core::DocumentId, ambre_core::SnapshotId)>,
    Json(req): Json<ReportSnapshotRequest>,
) -> Result<impl IntoResponse, AppError> {
    use ed25519_dalek::{Signature, VerifyingKey};
    db::get_document_member(&state.db, &doc_id, &auth.user_id)
        .await?
        .ok_or(AppError::Forbidden)?;
    let snap = db::get_snapshot(&state.db, &doc_id, &snap_id)
        .await?
        .ok_or(AppError::NotFound {
            resource: Resource::Snapshot,
        })?;
    let author = snap.author_pubkey;
    let signature = hex::decode(req.signature.as_str())
        .ok()
        .and_then(|bytes| Signature::from_slice(&bytes).ok())
        .ok_or(AppError::InvalidSignature)?;
    let transcript = ambre_protocol::snapshot_report::report_transcript(doc_id, snap_id, &author);
    VerifyingKey::from_bytes(auth.signing_pubkey.as_array())
        .and_then(|key| key.verify_strict(&transcript, &signature))
        .map_err(|_| AppError::InvalidSignature)?;
    if snap.status != db::SnapshotStatus::Pending {
        return Ok(axum::http::StatusCode::NO_CONTENT);
    }
    if let Some(dropped) = db::drop_pending_snapshot(&state.db, &doc_id, &snap_id).await? {
        tracing::info!(document_id = %doc_id, snapshot_id = %snap_id, "snapshot reported, dropped");
        let key = dropped.s3_key;
        if let Err(e) = state
            .s3
            .delete_object()
            .bucket(state.s3_bucket.as_str())
            .key(&key)
            .send()
            .await
        {
            tracing::warn!(s3_key = %key, error = %e, "reported snapshot: S3 delete failed");
        }
        // Every member sees who sealed it and who reported it (the Murmure
        // segments spec).
        let shown = crate::payload::Payload::server_msg(
            &ambre_protocol::ws::WsServerMsg::SnapshotDropped {
                document_id: doc_id,
                snapshot_id: snap_id,
                author,
                reporter: auth.signing_pubkey,
                after: if dropped.refinalize {
                    ambre_core::AfterDrop::Refinalize
                } else {
                    ambre_core::AfterDrop::Keep
                },
            },
        );
        let _ = crate::redis_ops::doc_broadcast(&state.redis, &doc_id, &shown).await;
        let ask = crate::payload::Payload::server_msg(
            &ambre_protocol::ws::WsServerMsg::SnapshotRequired {
                document_id: doc_id,
            },
        );
        let _ = crate::redis_ops::user_broadcast(&state.redis, &auth.user_id, &ask).await;
    }
    Ok(axum::http::StatusCode::NO_CONTENT)
}

/// Request to confirm a snapshot upload. The server inserts metadata and
/// broadcasts `SnapshotAvailable` to all subscribers.
#[derive(serde::Deserialize)]
pub struct CreateSnapshotRequest {
    #[allow(dead_code)]
    #[serde(with = "ambre_core::serde_id::snapshot_id")]
    pub snapshot_id: ambre_core::SnapshotId,
    pub author_pubkey: ambre_core::SigningKeyHex,
    /// No client builds a parent proof yet, so one that arrives is refused
    /// rather than stored unread (`SRV-228`). `null` reads as none.
    pub parent_snapshot_proof: Option<serde::de::IgnoredAny>,
    pub update_clocks: serde_json::Value,
    pub s3_key: ambre_core::ObjectKey,
    /// Highest delta `seq` the snapshot author had applied at upload time.
    /// Defaults to 0 for older clients (full replay preserved).
    #[serde(default)]
    pub covered_seq: ambre_core::Seq,
}
#[derive(serde::Serialize)]
pub struct CreateSnapshotResponse {
    #[serde(with = "ambre_core::serde_id::snapshot_id")]
    pub snapshot_id: ambre_core::SnapshotId,
}

pub async fn create_snapshot(
    State(state): State<Arc<AppState>>,
    auth: auth::AuthSession,
    PathId(doc_id): PathId<ambre_core::DocumentId>,
    Json(req): Json<CreateSnapshotRequest>,
) -> Result<impl IntoResponse, AppError> {
    let s = auth;
    let m = db::get_document_member(&state.db, &doc_id, &s.user_id)
        .await?
        .ok_or(AppError::Forbidden)?;
    if !m.role.can_write_document() {
        return Err(AppError::Forbidden);
    }

    // The `s3_key` is client-supplied; it MUST resolve inside this document's own
    // snapshot prefix. Otherwise a writer could register a row pointing at another
    // document's blob, later downloadable by this document's members (IDOR).
    if !s3::is_valid_snapshot_key(&state.config.s3_env_prefix, &doc_id, &req.s3_key) {
        return Err(AppError::BadRequest("invalid s3_key for document".into()));
    }

    let apk = hex::decode(req.author_pubkey.as_str()).map_err(|e| AppError::Conflict {
        detail: e.to_string(),
    })?;
    // Bind the snapshot author to the session identity, like the WS `Update`
    // author-key check. Each user has one signing key (the token's `spk` claim).
    // A member may only create a snapshot under their own key, never attribute
    // one to another member.
    if apk != s.signing_pubkey.as_slice() {
        return Err(AppError::Forbidden);
    }
    if req.parent_snapshot_proof.is_some() {
        return Err(AppError::BadRequest(
            "parent_snapshot_proof: no client builds one".into(),
        ));
    }

    // A snapshot cannot fold in a delta the relay never numbered. A horizon past
    // the counter would stop the relay asking for snapshots of the document.
    // A cold counter reads 0 and bounds nothing.
    let last = crate::redis_ops::current_seq(&state.redis, &doc_id).await?;
    let covered = req.covered_seq;
    if last > ambre_core::Seq(0) && covered > last {
        return Err(AppError::BadRequest(
            "covered_seq past the last delta".into(),
        ));
    }

    let sid = db::create_snapshot(
        &state.db,
        &doc_id,
        &s.signing_pubkey,
        &req.update_clocks,
        &req.s3_key,
        covered,
    )
    .await?;
    let msg =
        crate::payload::Payload::server_msg(&ambre_protocol::ws::WsServerMsg::SnapshotAvailable {
            document_id: doc_id,
            snapshot_id: sid,
            covered_seq: covered,
        });
    let _ = crate::redis_ops::doc_broadcast(&state.redis, &doc_id, &msg).await;
    Ok(axum::Json(CreateSnapshotResponse { snapshot_id: sid }))
}

/// Request for a presigned snapshot upload URL. `size` is the size of the
/// encrypted snapshot blob. It is validated against `snapshot_max_size`, and
/// bound into the presigned PUT URL so the upload cannot exceed it.
#[derive(serde::Deserialize)]
pub struct SnapshotUploadUrlRequest {
    #[serde(rename = "size_bytes")]
    pub size: ambre_core::ByteSize,
}

/// Presigned PUT URL for direct client-to-S3 upload of an encrypted snapshot.
#[derive(serde::Serialize)]
pub struct SnapshotUploadUrlResponse {
    pub upload_url: ambre_core::PresignedUrl,
    pub s3_key: ambre_core::ObjectKey,
}

pub async fn snapshot_upload_url(
    State(state): State<Arc<AppState>>,
    auth: auth::AuthSession,
    PathId(doc_id): PathId<ambre_core::DocumentId>,
    Json(req): Json<SnapshotUploadUrlRequest>,
) -> Result<impl IntoResponse, AppError> {
    let s = auth;
    let m = db::get_document_member(&state.db, &doc_id, &s.user_id)
        .await?
        .ok_or(AppError::Forbidden)?;
    if !m.role.can_write_document() {
        return Err(AppError::Forbidden);
    }
    // Reject an oversized snapshot before handing out an upload URL. Bind the
    // declared size into the presigned PUT, so the actual upload cannot exceed
    // it.
    if req.size == ambre_core::ByteSize(0) || req.size > state.config.snapshot_max_size {
        return Err(AppError::PayloadTooLarge(format!(
            "snapshot size {} bytes outside 1..={}",
            req.size.0, state.config.snapshot_max_size.0
        )));
    }

    let snapshot_id = ambre_core::SnapshotId::new();
    let key = s3::format_s3_snapshot_key(&state.config.s3_env_prefix, &doc_id, &snapshot_id);
    let url = s3::generate_presigned_upload_url(
        &state.s3,
        &state.s3_bucket,
        &key,
        state.config.presigned_url_ttl,
        req.size,
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
    #[serde(with = "ambre_core::serde_id::snapshot_id")]
    pub snapshot_id: ambre_core::SnapshotId,
}
#[derive(serde::Serialize)]
pub struct SnapshotDownloadUrlResponse {
    pub download_url: ambre_core::PresignedUrl,
}

pub async fn snapshot_download_url(
    State(state): State<Arc<AppState>>,
    auth: auth::AuthSession,
    PathId(doc_id): PathId<ambre_core::DocumentId>,
    Json(req): Json<SnapshotDownloadUrlRequest>,
) -> Result<impl IntoResponse, AppError> {
    let s = auth;
    db::get_document_member(&state.db, &doc_id, &s.user_id)
        .await?
        .ok_or(AppError::Forbidden)?;
    let snap_id = req.snapshot_id;
    // Resolve the snapshot scoped to the path document. A member of doc A must
    // never obtain a download URL for a snapshot of doc B (cross-document IDOR).
    let snap = db::get_snapshot(&state.db, &doc_id, &snap_id)
        .await?
        .ok_or(AppError::NotFound {
            resource: Resource::Snapshot,
        })?;
    let url = s3::generate_presigned_download_url(
        &state.s3,
        &state.s3_bucket,
        &snap.s3_key,
        state.config.presigned_url_ttl,
    )
    .await?;
    Ok(axum::Json(SnapshotDownloadUrlResponse {
        download_url: url,
    }))
}

/// Query parameters for delta retrieval (reconnect catch-up).
#[derive(serde::Deserialize)]
pub struct DeltasQuery {
    pub after_seq: Option<ambre_core::Seq>,
    pub limit: Option<db::QueryLimit>,
}
/// One delta entry in the response list.
#[derive(serde::Serialize)]
pub struct DeltaEntry {
    pub seq: ambre_core::Seq,
    pub clock: ambre_protocol::messages::Clock,
    pub author_pubkey: ambre_core::SigningKeyHex,
    /// The object of its own, for a delta over the inline size. A delta in the
    /// hot tail or in a batch has none, and is read over the socket replay.
    pub s3_key: Option<ambre_core::ObjectKey>,
}
#[derive(serde::Serialize)]
pub struct DeltasResponse {
    pub deltas: Vec<DeltaEntry>,
}

pub async fn get_deltas(
    State(state): State<Arc<AppState>>,
    auth: auth::AuthSession,
    PathId(doc_id): PathId<ambre_core::DocumentId>,
    Query(params): Query<DeltasQuery>,
) -> Result<impl IntoResponse, AppError> {
    let s = auth;
    db::get_document_member(&state.db, &doc_id, &s.user_id)
        .await?
        .ok_or(AppError::Forbidden)?;
    let after = params.after_seq.unwrap_or(ambre_core::Seq::ZERO);
    // The page size arrives clamped (M16): an unbounded `limit` would force a
    // full-table scan of `updates` and a large response allocation.
    let limit = params.limit.unwrap_or(db::QueryLimit::DEFAULT);
    let updates = db::get_updates_after(&state.db, &doc_id, after, limit).await?;
    let deltas: Vec<_> = updates
        .into_iter()
        .map(|u| DeltaEntry {
            seq: u.seq,
            clock: u.clock,
            author_pubkey: ambre_core::SigningKeyHex::encode(&u.author_pubkey),
            s3_key: u.s3_key,
        })
        .collect();
    Ok(axum::Json(DeltasResponse { deltas }))
}
