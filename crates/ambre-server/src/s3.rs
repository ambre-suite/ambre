//! S3 utilities — presigned URLs, object keys, delta flusher.
//!
//! ## S3 Lifecycle Policies
//!
//! Recommended lifecycle rules for messaging segments (> 12 months):
//!
//! ```json
//! {
//!   "Rules": [
//!     {
//!       "Id": "expire-old-messaging-segments",
//!       "Filter": { "Prefix": "<s3_env_prefix>/documents/*/deltas/" },
//!       "Status": "Enabled",
//!       "Expiration": { "Days": 365 }
//!     },
//!     {
//!       "Id": "expire-old-snapshots",
//!       "Filter": { "Prefix": "<s3_env_prefix>/documents/*/snapshots/" },
//!       "Status": "Enabled",
//!       "Expiration": { "Days": 365 }
//!     }
//!   ]
//! }
//! ```
//!
//! These are applied at the S3 bucket level (AWS Console / Terraform / Pulumi).
//! They complement the server-side snapshot compaction which keeps at most
//! 10 snapshots per document in PostgreSQL.
//!
//! ## Valkey 8+ Migration
//!
//! Current minimum: Redis 7.4+ / Valkey 8.0+.
//! Valkey 8.0 adds per-field `HEXPIRE` (expire individual hash fields,
//! useful for presence TTLs: expire `players:{doc_id}:{user_id}` per-field
//! instead of whole-token TTL). Migration path: upgrade Valkey cluster,
//! then replace `SETEX` with `HEXPIRE` in `redis_ops::presence_*` functions.

use aws_sdk_s3::Client as S3Client;
use aws_sdk_s3::presigning::PresigningConfig;
use aws_sdk_s3::primitives::ByteStream;
use rustis::commands::ScanOptions;
use rustis::commands::{GenericCommands, ListCommands};
use std::time::Duration;
#[allow(clippy::single_component_path_imports)]
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

/// Validates a client-supplied media `content_hash` before it is interpolated
/// into an S3 object key.
///
/// The spec mandates a BLAKE2b-256 digest rendered as 64 lowercase hex chars.
/// Enforcing that shape rejects path-traversal payloads (`../`, absolute keys)
/// that would otherwise let a caller read/write objects outside the document's
/// own media prefix.
pub fn is_valid_content_hash(content_hash: &str) -> bool {
    content_hash.len() == 64
        && content_hash
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Validates that a client-supplied snapshot `s3_key` actually belongs to the
/// document the caller is authorized on, before it is persisted and later handed
/// back as a presigned download URL.
///
/// Without this, a writer could register a snapshot row whose `s3_key` points at
/// another document's object (`.../documents/{other}/snapshots/...`); any member
/// of THIS document could then obtain a presigned URL for that foreign blob
/// (cross-document IDOR). We require the exact per-document snapshot prefix and a
/// single, slash-free object id (which also rejects `../` traversal).
pub fn is_valid_snapshot_key(
    env_prefix: &str,
    document_id: &ambre_core::DocumentId,
    s3_key: &str,
) -> bool {
    let expected_prefix = format!("{env_prefix}/documents/{document_id}/snapshots/");
    let Some(id) = s3_key.strip_prefix(&expected_prefix) else {
        return false;
    };
    !id.is_empty() && !id.contains('/')
}

/// S3 object key for an encrypted media blob (content-addressed by hash).
pub fn format_s3_media_key(
    env_prefix: &str,
    document_id: &ambre_core::DocumentId,
    content_hash: &str,
) -> String {
    format!("{env_prefix}/documents/{document_id}/media/{content_hash}")
}

/// Generates a presigned S3 PUT URL valid for `ttl_secs`, bound to exactly
/// `content_length` bytes.
///
/// The client uploads the encrypted blob directly to S3; the server never sees
/// the plaintext body. Setting `content_length` puts `Content-Length` into the
/// SigV4 signed headers, so S3 rejects any upload whose body length differs from
/// the size the caller declared (and the caller validated against its cap). This
/// stops a member from writing an arbitrarily large object to the bucket through
/// an otherwise key-only presigned URL.
pub async fn generate_presigned_upload_url(
    s3: &S3Client,
    bucket: &str,
    key: &str,
    ttl_secs: u64,
    content_length: u64,
) -> Result<String, AppError> {
    let config = PresigningConfig::expires_in(Duration::from_secs(ttl_secs))
        .map_err(|e| AppError::S3(format!("presigning config: {e}")))?;

    s3.put_object()
        .bucket(bucket)
        .key(key)
        .content_length(content_length as i64)
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

/// Fetches an object's raw bytes from S3 (used to replay a persisted delta).
///
/// Returns `Ok(None)` when the key does not exist yet (e.g. a freshly written
/// delta still pending in Redis, not yet flushed), so the caller can fall back
/// to the pending list rather than treating it as a hard error.
pub async fn fetch_object(
    s3: &S3Client,
    bucket: &str,
    key: &str,
) -> Result<Option<Vec<u8>>, AppError> {
    match s3.get_object().bucket(bucket).key(key).send().await {
        Ok(out) => {
            let data = out
                .body
                .collect()
                .await
                .map_err(|e| AppError::S3(format!("read object body: {e}")))?;
            Ok(Some(data.into_bytes().to_vec()))
        }
        Err(e) => {
            // A missing key is an expected miss (not-yet-flushed), not an error.
            // Use `as_service_error` (non-consuming, returns Option) rather than
            // `into_service_error`, which PANICS on a non-service error such as a
            // network/dispatch failure.
            if e.as_service_error()
                .map(|se| se.is_no_such_key())
                .unwrap_or(false)
            {
                Ok(None)
            } else {
                Err(AppError::S3(format!("get_object {key}: {e}")))
            }
        }
    }
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
            let opts = ScanOptions::default()
                .match_pattern("doc:*:pending")
                .count(100);
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
///
/// The whole drain is serialized per document by a Redis lock: the flusher runs
/// once per server instance, and without the lock two instances could `LRANGE`
/// the same list, both upload, then both `LTRIM` the front by their own count --
/// trimming away deltas a client `RPUSH`ed during the window (durable loss of an
/// already-sequenced, acknowledged delta). A losing instance simply skips this
/// document; the next 15 s cycle picks it up. The client write path never takes
/// this lock, so collaboration latency is unaffected.
async fn flush_doc_pending(state: &crate::state::AppState, pending_key: &str) {
    let Some(lock_key) = pending_key
        .strip_suffix(":pending")
        .map(|prefix| format!("{prefix}:flush:lock"))
    else {
        return;
    };
    // TTL comfortably exceeds one flush cycle; if the holder dies mid-flush the
    // lock auto-expires and the next cycle retries.
    let token = uuid::Uuid::new_v4().to_string();
    match crate::redis_ops::try_acquire_flush_lock(&state.redis, &lock_key, &token, 60).await {
        Ok(true) => {}
        Ok(false) => return,
        Err(e) => {
            tracing::error!(key = pending_key, error = %e, "flush: lock acquire failed");
            return;
        }
    }

    flush_doc_pending_locked(state, pending_key).await;

    if let Err(e) = crate::redis_ops::release_flush_lock(&state.redis, &lock_key, &token).await {
        tracing::warn!(key = pending_key, error = %e, "flush: lock release failed");
    }
}

/// Drain body, run under the per-document flush lock held by [`flush_doc_pending`].
async fn flush_doc_pending_locked(state: &crate::state::AppState, pending_key: &str) {
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

    // Dead-letter list for permanently-bad entries (malformed JSON / base64),
    // derived from the pending key so we do not need to re-parse the doc id.
    let deadletter_key = pending_key
        .strip_suffix(":pending")
        .map(|prefix| format!("{prefix}:deadletter"));

    // Count of contiguous FRONT entries we have finished with -- either
    // uploaded to S3 or dead-lettered as poison. This is the exact `ltrim`
    // start index. It must track BOTH outcomes: the old code trimmed by the
    // upload count alone, so a `continue`d poison entry ahead of a good one
    // desynced the index (re-uploading the good entry every cycle) and a
    // trailing poison entry was retried forever. We only advance the front and
    // stop on the first TRANSIENT (retryable) failure, so the trim never skips
    // an entry we still need to retry.
    let mut processed: isize = 0;
    for blob_str in &blobs {
        // Never log `blob_str`: it carries `update_b64`, the E2EE ciphertext.
        // Only routing metadata (redis key, byte length, and once parsed the
        // s3_key / seq) is safe to surface (C1).
        let parsed: Option<serde_json::Value> = serde_json::from_str(blob_str).ok();
        let poison = |reason: &str, seq: u64| {
            tracing::error!(
                key = pending_key,
                seq,
                blob_len = blob_str.len(),
                reason,
                "flush: dead-lettering poison entry"
            );
        };

        // Resolve the routing fields; any structural defect is a permanent
        // poison entry (it will never parse), so dead-letter it and advance
        // past it rather than blocking the queue forever.
        let fields = parsed.as_ref().and_then(|p| {
            let s3_key = p["s3_key"].as_str()?;
            let update_b64 = p["update_b64"].as_str()?;
            let seq = p["seq"].as_u64().unwrap_or(0);
            use base64::Engine;
            let wire = base64::engine::general_purpose::URL_SAFE
                .decode(update_b64)
                .ok()?;
            Some((s3_key.to_string(), seq, wire))
        });

        let Some((s3_key, seq, update_wire)) = fields else {
            let seq = parsed.as_ref().and_then(|p| p["seq"].as_u64()).unwrap_or(0);
            poison("malformed json / missing fields / bad base64", seq);
            if let Some(dlk) = &deadletter_key {
                let _: Result<usize, _> = state.redis.rpush(dlk, [blob_str]).await;
            }
            processed += 1;
            continue;
        };

        let body = ByteStream::from(update_wire);
        match state
            .s3
            .put_object()
            .bucket(&state.s3_bucket)
            .key(&s3_key)
            .body(body)
            .send()
            .await
        {
            Ok(_) => {
                tracing::debug!(s3_key, seq, "flushed delta to S3");
                processed += 1;
            }
            Err(e) => {
                // Transient (retryable) failure: stop here so the front trim
                // keeps this entry and everything after it for the next cycle.
                tracing::error!(s3_key, seq, error = %e, "flush: S3 upload failed, will retry");
                break;
            }
        }
    }

    if processed > 0 {
        let _: Result<(), _> = state.redis.ltrim(pending_key, processed, -1).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ambre_core::{DocumentId, SnapshotId};

    #[test]
    fn snapshot_key_format() {
        let doc_id = DocumentId::from_bytes([0u8; 16]);
        let snap_id = SnapshotId::from_bytes([1u8; 16]);
        let key = format_s3_snapshot_key("dev", &doc_id, &snap_id);
        assert!(key.starts_with("dev/documents/"));
        assert!(key.contains("/snapshots/"));
        assert_eq!(key, format!("dev/documents/{doc_id}/snapshots/{snap_id}"));
    }

    #[test]
    fn snapshot_key_differs_per_env() {
        let doc_id = DocumentId::from_bytes([0u8; 16]);
        let snap_id = SnapshotId::from_bytes([1u8; 16]);
        let dev_key = format_s3_snapshot_key("dev", &doc_id, &snap_id);
        let prod_key = format_s3_snapshot_key("prod", &doc_id, &snap_id);
        assert_ne!(dev_key, prod_key);
        assert!(prod_key.starts_with("prod/"));
    }

    #[test]
    fn delta_key_zero_pads_seq() {
        let doc_id = DocumentId::from_bytes([2u8; 16]);
        let key = format_s3_delta_key("dev", &doc_id, 42);
        assert!(key.ends_with("deltas/0000000042"));
    }

    #[test]
    fn delta_key_pads_small_seq() {
        let doc_id = DocumentId::from_bytes([2u8; 16]);
        let key = format_s3_delta_key("dev", &doc_id, 7);
        assert!(key.ends_with("deltas/0000000007"));
    }

    #[test]
    fn delta_key_handles_large_seq() {
        let doc_id = DocumentId::from_bytes([2u8; 16]);
        let key = format_s3_delta_key("dev", &doc_id, 9999999999u64);
        assert!(key.ends_with("deltas/9999999999"));
    }

    #[test]
    fn media_key_content_addressed() {
        let doc_id = DocumentId::from_bytes([3u8; 16]);
        let hash = "abc123def456";
        let key = format_s3_media_key("dev", &doc_id, hash);
        assert!(key.ends_with("media/abc123def456"));
        assert!(key.contains("/documents/"));
    }

    #[test]
    fn snapshot_key_validation_scopes_to_document() {
        let doc = DocumentId::from_bytes([9u8; 16]);
        let other = DocumentId::from_bytes([8u8; 16]);
        let valid = format_s3_snapshot_key("dev", &doc, &SnapshotId::from_bytes([1u8; 16]));
        assert!(is_valid_snapshot_key("dev", &doc, &valid));

        // Another document's snapshot prefix is rejected (cross-document IDOR).
        let foreign = format_s3_snapshot_key("dev", &other, &SnapshotId::from_bytes([1u8; 16]));
        assert!(!is_valid_snapshot_key("dev", &doc, &foreign));

        // Wrong env prefix, path traversal, empty id, and nested paths are rejected.
        assert!(!is_valid_snapshot_key("prod", &doc, &valid));
        assert!(!is_valid_snapshot_key(
            "dev",
            &doc,
            &format!("dev/documents/{doc}/snapshots/../../{other}/snapshots/x")
        ));
        assert!(!is_valid_snapshot_key(
            "dev",
            &doc,
            &format!("dev/documents/{doc}/snapshots/")
        ));
        assert!(!is_valid_snapshot_key(
            "dev",
            &doc,
            &format!("dev/documents/{doc}/snapshots/a/b")
        ));
    }

    /// Builds an offline S3 client (static credentials, no network) for
    /// presigning, which is a purely local SigV4 computation.
    fn test_s3_client() -> S3Client {
        let conf = aws_sdk_s3::Config::builder()
            .region(aws_sdk_s3::config::Region::new("us-east-1"))
            .credentials_provider(aws_sdk_s3::config::Credentials::new(
                "test-access",
                "test-secret",
                None,
                None,
                "test",
            ))
            .build();
        aws_sdk_s3::Client::from_conf(conf)
    }

    /// The presigned upload URL must bind `Content-Length`: S3 then rejects any
    /// body whose length differs from the declared (and cap-validated) size, so
    /// the key-scoped URL cannot be used to write an arbitrarily large object.
    #[tokio::test]
    async fn presigned_upload_url_signs_content_length() {
        let s3 = test_s3_client();
        let url =
            generate_presigned_upload_url(&s3, "bucket", "dev/documents/x/media/y", 900, 4096)
                .await
                .expect("presign");
        let lower = url.to_lowercase();
        // SigV4 lists signed headers in `X-Amz-SignedHeaders`; content-length
        // must be among them for S3 to enforce it.
        let signed = lower
            .split("x-amz-signedheaders=")
            .nth(1)
            .expect("signed headers present");
        assert!(
            signed.contains("content-length"),
            "content-length must be a signed header, got: {url}"
        );
    }

    #[test]
    fn content_hash_validation() {
        let valid = "a".repeat(64);
        assert!(is_valid_content_hash(&valid));
        assert!(is_valid_content_hash(&"0123456789abcdef".repeat(4)));

        // Path traversal payloads and non-canonical shapes are rejected.
        assert!(!is_valid_content_hash("../../snapshots/x"));
        assert!(!is_valid_content_hash(&format!("../{}", "a".repeat(60))));
        assert!(!is_valid_content_hash(&"A".repeat(64))); // uppercase
        assert!(!is_valid_content_hash(&"g".repeat(64))); // non-hex
        assert!(!is_valid_content_hash(&"a".repeat(63))); // too short
        assert!(!is_valid_content_hash(&"a".repeat(65))); // too long
        assert!(!is_valid_content_hash(""));
    }
}
