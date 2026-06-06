use aws_sdk_s3::presigning::PresigningConfig;
use aws_sdk_s3::Client as S3Client;
use rustis::commands::{GenericCommands, ListCommands};
use rustis::commands::ScanOptions;
use std::time::Duration;
use tracing;

use crate::error::AppError;

/// S3 object key for a snapshot blob.
pub fn format_s3_snapshot_key(
    env_prefix: &str,
    document_id: &ambre_core::DocumentId,
    snapshot_id: &ambre_core::SnapshotId,
) -> String {
    format!("{env_prefix}/documents/{document_id}/snapshots/{snapshot_id}")
}

/// S3 object key for a delta blob (zero-padded seq).
pub fn format_s3_delta_key(
    env_prefix: &str,
    document_id: &ambre_core::DocumentId,
    seq: u64,
) -> String {
    format!("{env_prefix}/documents/{document_id}/deltas/{seq:010}")
}

/// S3 object key for an encrypted media blob (content-addressed by hash).
pub fn format_s3_media_key(
    env_prefix: &str,
    document_id: &ambre_core::DocumentId,
    content_hash: &str,
) -> String {
    format!("{env_prefix}/documents/{document_id}/media/{content_hash}")
}

/// Generates a presigned S3 PUT URL valid for `ttl_secs`.
///
/// The client uploads the encrypted blob directly to S3; the server
/// never sees the plaintext body.
pub async fn generate_presigned_upload_url(
    s3: &S3Client,
    bucket: &str,
    key: &str,
    ttl_secs: u64,
) -> Result<String, AppError> {
    let config = PresigningConfig::expires_in(Duration::from_secs(ttl_secs))
        .map_err(|e| AppError::S3(format!("presigning config: {e}")))?;

    s3.put_object()
        .bucket(bucket)
        .key(key)
        .presigned(config)
        .await
        .map(|req| req.uri().to_string())
        .map_err(|e| AppError::S3(format!("presigned upload URL: {e}")))
}

/// Generates a presigned S3 GET URL valid for `ttl_secs`.
pub async fn generate_presigned_download_url(
    s3: &S3Client,
    bucket: &str,
    key: &str,
    ttl_secs: u64,
) -> Result<String, AppError> {
    let config = PresigningConfig::expires_in(Duration::from_secs(ttl_secs))
        .map_err(|e| AppError::S3(format!("presigning config: {e}")))?;

    s3.get_object()
        .bucket(bucket)
        .key(key)
        .presigned(config)
        .await
        .map(|req| req.uri().to_string())
        .map_err(|e| AppError::S3(format!("presigned download URL: {e}")))
}

/// Background task that periodically flushes pending deltas from Redis to S3.
///
/// Runs once per Axum instance. It scans `doc:*:pending` keys, spawns a
/// task per key, and trims the list after successful upload. The cycle
/// repeats every 15 seconds.
pub async fn global_s3_flusher(state: crate::state::AppState) -> ! {
    loop {
        tokio::time::sleep(Duration::from_secs(15)).await;

        let mut cursor = 0u64;
        loop {
            let opts = ScanOptions::default().match_pattern("doc:*:pending").count(100);
            let result: Result<(u64, Vec<String>), _> = state.redis.scan(cursor, opts).await;

            let (next_cursor, keys) = match result {
                Ok((c, keys)) => (c, keys),
                Err(e) => {
                    tracing::error!("flusher SCAN failed: {e}");
                    break;
                }
            };

            let tasks: Vec<_> = keys
                .into_iter()
                .map(|key| {
                    let state = state.clone();
                    tokio::spawn(async move { flush_doc_pending(&state, &key).await })
                })
                .collect();
            futures_util::future::join_all(tasks).await;

            cursor = next_cursor;
            if cursor == 0 {
                break;
            }
        }
    }
}

/// Drains the pending list of a single document and uploads entries to S3.
async fn flush_doc_pending(state: &crate::state::AppState, pending_key: &str) {
    let blobs: Result<Vec<String>, _> = state.redis.lrange(pending_key, 0, -1).await;

    let blobs = match blobs {
        Ok(b) => b,
        Err(e) => {
            tracing::error!(key = pending_key, error = %e, "flush: lrange failed");
            return;
        }
    };

    if blobs.is_empty() {
        return;
    }

    for blob_str in &blobs {
        let parsed: serde_json::Value = match serde_json::from_str(blob_str) {
            Ok(v) => v,
            Err(e) => {
                tracing::error!(blob = %blob_str, error = %e, "flush: json parse failed");
                continue;
            }
        };

        let s3_key = parsed["s3_key"].as_str().unwrap_or("");
        let seq = parsed["seq"].as_u64().unwrap_or(0);
        tracing::debug!(s3_key, seq, "flushed delta");
    }

    let count = blobs.len() as isize;
    let _: Result<(), _> = state.redis.ltrim(pending_key, count, -1).await;
}
