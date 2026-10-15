//! S3 utilities -- presigned URLs, object keys, delta flusher.
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
//! ## S3 CORS
//!
//! The browser puts and fetches media through the presigned links itself.
//! So the bucket must allow `PUT` and `GET` from the application's origin.
//! Without that rule, every picture upload fails in the browser. minio
//! allows every origin by default; AWS S3 allows none.
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
/// document the caller is authorized on. The check runs before the key is
/// persisted and later handed back as a presigned download URL.
///
/// Without this, a writer could register a snapshot row whose `s3_key` points at
/// another document's object (`.../documents/{other}/snapshots/...`). Any member
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

/// The longest a presigned URL may live: 15 minutes (`ZK-014`, `SRV-052`).
pub const PRESIGNED_URL_MAX_TTL_SECS: u64 = 15 * 60;

/// The lifetime a presigned URL is signed for: the configured one, held under
/// the ceiling. A misconfigured day comes out as 15 minutes.
fn presign_lifetime(ttl_secs: u64) -> Duration {
    Duration::from_secs(ttl_secs.min(PRESIGNED_URL_MAX_TTL_SECS))
}

/// Generates a presigned S3 PUT URL valid for `ttl_secs`, bound to exactly
/// `content_length` bytes.
///
/// The client uploads the encrypted blob directly to S3; the server never sees
/// the plaintext body. Setting `content_length` puts `Content-Length` into the
/// SigV4 signed headers. S3 then rejects any upload whose body length differs
/// from the size the caller declared (and the caller validated against its cap).
/// This stops a member from writing an arbitrarily large object to the bucket
/// through an otherwise key-only presigned URL.
pub async fn generate_presigned_upload_url(
    s3: &S3Client,
    bucket: &str,
    key: &str,
    ttl_secs: u64,
    content_length: u64,
) -> Result<String, AppError> {
    let config = PresigningConfig::expires_in(presign_lifetime(ttl_secs))
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
    let config = PresigningConfig::expires_in(presign_lifetime(ttl_secs))
        .map_err(|e| AppError::S3(format!("presigning config: {e}")))?;

    s3.get_object()
        .bucket(bucket)
        .key(key)
        .presigned(config)
        .await
        .map(|req| req.uri().to_string())
        .map_err(|e| AppError::S3(format!("presigned download URL: {e}")))
}

/// The prefix every object of one document lives under: its snapshots, its
/// deltas and its media.
pub fn format_s3_document_prefix(env_prefix: &str, document_id: &ambre_core::DocumentId) -> String {
    format!("{env_prefix}/documents/{document_id}/")
}

/// Deletes every object under `prefix`, a page of at most a thousand keys at a
/// time. Returns how many it deleted. An object that fails to delete fails the
/// call, so the caller can refuse to forget the document (`SRV-057`).
pub async fn delete_prefix(s3: &S3Client, bucket: &str, prefix: &str) -> Result<usize, AppError> {
    use aws_sdk_s3::types::{Delete, ObjectIdentifier};
    let mut deleted = 0;
    let mut token: Option<String> = None;
    loop {
        let page = s3
            .list_objects_v2()
            .bucket(bucket)
            .prefix(prefix)
            .set_continuation_token(token.take())
            .send()
            .await
            .map_err(|e| AppError::S3(format!("list {prefix}: {e}")))?;
        let objects = page
            .contents()
            .iter()
            .filter_map(|o| o.key())
            .map(|key| ObjectIdentifier::builder().key(key).build())
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| AppError::S3(format!("object identifier: {e}")))?;
        if !objects.is_empty() {
            let count = objects.len();
            let delete = Delete::builder()
                .set_objects(Some(objects))
                .quiet(true)
                .build()
                .map_err(|e| AppError::S3(format!("delete request: {e}")))?;
            let out = s3
                .delete_objects()
                .bucket(bucket)
                .delete(delete)
                .send()
                .await
                .map_err(|e| AppError::S3(format!("delete under {prefix}: {e}")))?;
            if let Some(failed) = out.errors().first() {
                return Err(AppError::S3(format!(
                    "delete {}: {}",
                    failed.key().unwrap_or("?"),
                    failed.message().unwrap_or("refused")
                )));
            }
            deleted += count;
        }
        match page.next_continuation_token() {
            Some(next) if page.is_truncated() == Some(true) => token = Some(next.to_string()),
            _ => return Ok(deleted),
        }
    }
}

/// Fetches an object's raw bytes from S3 (used to replay a persisted delta).
///
/// Returns `Ok(None)` when the key does not exist yet, for example a freshly written
/// delta still pending in Redis, not yet flushed. The caller can then fall back to
/// the pending list, rather than treating it as a hard error.
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
/// Runs once per Axum instance, every 15 seconds. The work list is the
/// `flush:dirty` SET, the documents that actually have unflushed deltas,
/// maintained by `append_pending_delta`. The common path is therefore O(dirty
/// docs), instead of an O(keyspace) `SCAN doc:*:pending` every cycle. A full
/// SCAN reconciliation runs only occasionally, and on the first cycle, seeding
/// the set on upgrade. It re-marks any pending key missing from the set, so no
/// acknowledged delta can be stranded by a lost marker.
///
/// A cycle that panics is lost alone, and the next cycle runs (`SRV-101`). A
/// document's flush already runs on a task of its own, so its panic ends that
/// document's flush and nothing else.
pub async fn global_s3_flusher(state: crate::state::AppState) -> ! {
    // One full-keyspace reconciliation every RECONCILE_EVERY cycles (~5 min at a
    // 15 s cycle). Cycle 0 reconciles first, seeding the dirty set from any
    // pre-existing pending lists on a fresh deploy/upgrade.
    const RECONCILE_EVERY: u32 = 20;
    let mut cycle: u32 = 0;

    loop {
        let reconcile = cycle == 0;
        cycle = (cycle + 1) % RECONCILE_EVERY;
        crate::supervise::contained("flusher cycle", flush_cycle(&state, reconcile)).await;
    }
}

/// One flusher cycle: the reconciliation when it is due, the wait, then every
/// dirty document's flush and the pass over unfinished deletes.
async fn flush_cycle(state: &crate::state::AppState, reconcile: bool) {
    if reconcile {
        reconcile_dirty_set(state).await;
    }

    tokio::time::sleep(Duration::from_secs(15)).await;

    let keys = match crate::redis_ops::dirty_pending_keys(&state.redis).await {
        Ok(keys) => keys,
        Err(e) => {
            tracing::error!("flusher dirty-set read failed: {e}");
            return;
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

    crate::deletion::finish_deletions(state).await;
}

/// How long the per-document flush lock lives. It comfortably exceeds one
/// flush; if the holder dies mid-flush the lock expires and the next cycle
/// retries.
const FLUSH_LOCK_TTL_SECS: u64 = 60;

/// How long a write that began before a delete may still land after it
/// (`SRV-057`). A presigned upload link stays valid for its lifetime. A flush
/// holds its lock at most `FLUSH_LOCK_TTL_SECS`, and a minute of margin covers
/// the requests in flight. The tombstone lives this long, and the
/// second sweep of the objects runs once it has passed.
pub fn late_write_window_secs(presigned_url_ttl_secs: u64) -> u64 {
    presign_lifetime(presigned_url_ttl_secs).as_secs() + FLUSH_LOCK_TTL_SECS + 60
}

/// Seconds since the Unix epoch, the clock a delete's sweep is scheduled in.
pub fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// Re-marks every existing `doc:*:pending` key in the dirty set via a single
/// keyspace SCAN. Bounds the SCAN to a low frequency. A pending list which
/// somehow missed its `SADD` is still eventually flushed, rather than
/// stranded. That covers an upgrade from before the dirty-set existed, or a
/// crash between RPUSH and SADD.
async fn reconcile_dirty_set(state: &crate::state::AppState) {
    let mut cursor = 0u64;
    loop {
        let opts = ScanOptions::default()
            .match_pattern("doc:*:pending")
            .count(100);
        let result: Result<(u64, Vec<String>), _> = state.redis.scan(cursor, opts).await;
        let (next_cursor, keys) = match result {
            Ok((c, keys)) => (c, keys),
            Err(e) => {
                tracing::error!("flusher reconcile SCAN failed: {e}");
                return;
            }
        };
        for key in &keys {
            if let Err(e) = crate::redis_ops::mark_pending_dirty(&state.redis, key).await {
                tracing::warn!(key, error = %e, "flusher reconcile: mark dirty failed");
            }
        }
        cursor = next_cursor;
        if cursor == 0 {
            return;
        }
    }
}

/// Drains the pending list of a single document and uploads entries to S3.
///
/// The whole drain is serialized per document by a Redis lock. The flusher runs
/// once per server instance. Without the lock two instances could `LRANGE` the
/// same list, both upload, then both `LTRIM` the front by their own count. That
/// trims away deltas a client `RPUSH`ed during the window, a durable loss of an
/// already-sequenced, acknowledged delta. A losing instance simply skips this
/// document; the next 15 s cycle picks it up. The client write path never takes
/// this lock, so collaboration latency is unaffected.
async fn flush_doc_pending(state: &crate::state::AppState, pending_key: &str) {
    let Some(lock_key) = pending_key
        .strip_suffix(":pending")
        .map(|prefix| format!("{prefix}:flush:lock"))
    else {
        return;
    };
    let token = uuid::Uuid::new_v4().to_string();
    match crate::redis_ops::try_acquire_flush_lock(
        &state.redis,
        &lock_key,
        &token,
        FLUSH_LOCK_TTL_SECS,
    )
    .await
    {
        Ok(true) => {}
        Ok(false) => return,
        Err(e) => {
            tracing::error!(key = pending_key, error = %e, "flush: lock acquire failed");
            return;
        }
    }

    flush_doc_pending_locked(state, pending_key).await;

    // Drop the dirty marker iff the list fully drained (atomic LLEN-then-SREM).
    // A transient S3 failure leaves entries, so LLEN > 0 keeps the marker and the
    // document is revisited next cycle. Done under the lock so a losing instance
    // never clears a marker for a list it did not drain.
    if let Err(e) = crate::redis_ops::clear_pending_dirty_if_empty(&state.redis, pending_key).await
    {
        tracing::warn!(key = pending_key, error = %e, "flush: clear dirty marker failed");
    }

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

    // Key prefix of the document, for the dead-letter list of permanently-bad
    // entries (malformed JSON / base64). It spares re-parsing the doc id.
    let doc_prefix = pending_key.strip_suffix(":pending");

    let mut sink = S3Sink { state, doc_prefix };
    let processed = drain_front(pending_key, &blobs, &mut sink).await;

    if processed > 0 {
        let _: Result<(), _> = state.redis.ltrim(pending_key, processed, -1).await;
    }
}

/// Where a drain sends each pending entry: the store, or the dead letters.
trait DrainSink {
    /// Stores one delta under its object key. An error is retryable.
    fn store(
        &mut self,
        s3_key: &str,
        wire: Vec<u8>,
    ) -> impl Future<Output = Result<(), String>> + Send;
    /// Keeps an entry that will never parse where an operator can read it.
    fn dead_letter(&mut self, blob: &str) -> impl Future<Output = ()> + Send;
}

/// The production sink: the object store, and the document's dead letters.
struct S3Sink<'a> {
    state: &'a crate::state::AppState,
    doc_prefix: Option<&'a str>,
}

impl DrainSink for S3Sink<'_> {
    async fn store(&mut self, s3_key: &str, wire: Vec<u8>) -> Result<(), String> {
        self.state
            .s3
            .put_object()
            .bucket(&self.state.s3_bucket)
            .key(s3_key)
            .body(ByteStream::from(wire))
            .send()
            .await
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    async fn dead_letter(&mut self, blob: &str) {
        if let Some(prefix) = self.doc_prefix {
            let _ = crate::redis_ops::push_poison_entry(&self.state.redis, prefix, blob).await;
        }
    }
}

/// Walks the pending entries from the front and returns how many it finished
/// with: stored, or dead-lettered as permanently unreadable (`SRV-035`).
///
/// The count is the exact `ltrim` start index, so it tracks both outcomes. It
/// stops at the first failed store, which the next cycle retries. The trim
/// therefore never removes an entry that was neither stored nor dead-lettered.
async fn drain_front(pending_key: &str, blobs: &[String], sink: &mut impl DrainSink) -> isize {
    let mut processed: isize = 0;
    for blob_str in blobs {
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

        // Resolve the routing fields. Any structural defect is a permanent
        // poison entry that will never parse, so dead-letter it and advance
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
            sink.dead_letter(blob_str).await;
            processed += 1;
            continue;
        };

        match sink.store(&s3_key, update_wire).await {
            Ok(()) => {
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
    processed
}

#[cfg(test)]
mod tests {
    use super::*;
    use ambre_core::{DocumentId, SnapshotId};

    /// A pending entry as `append_pending_delta` stores it.
    fn entry(s3_key: &str, seq: u64) -> String {
        use base64::Engine;
        serde_json::json!({
            "seq": seq,
            "s3_key": s3_key,
            "update_b64": base64::engine::general_purpose::URL_SAFE.encode([seq as u8; 4]),
        })
        .to_string()
    }

    /// Holds `SRV-035`: a store that fails mid-batch stops the walk there, so
    /// the trim keeps the failed entry and every one after it.
    #[tokio::test]
    async fn a_partial_flush_trims_only_what_it_stored() {
        let blobs = vec![
            entry("d/1", 1),
            "not json".to_owned(),
            entry("d/3", 3),
            entry("d/4", 4),
            entry("d/5", 5),
        ];
        /// Refuses one key, and records what it stored and dead-lettered.
        #[derive(Default)]
        struct Flaky {
            stored: Vec<String>,
            dead: Vec<String>,
        }
        impl DrainSink for Flaky {
            async fn store(&mut self, s3_key: &str, _wire: Vec<u8>) -> Result<(), String> {
                if s3_key == "d/4" {
                    return Err("unavailable".to_owned());
                }
                self.stored.push(s3_key.to_owned());
                Ok(())
            }
            async fn dead_letter(&mut self, blob: &str) {
                self.dead.push(blob.to_owned());
            }
        }
        let mut sink = Flaky::default();
        let processed = drain_front("doc:x:pending", &blobs, &mut sink).await;

        // Stored, dead-lettered, stored: the failure at the fourth stops it.
        assert_eq!(processed, 3);
        assert_eq!(sink.stored, ["d/1", "d/3"]);
        assert_eq!(sink.dead, ["not json"]);
        // The fifth entry was never tried, and the trim keeps it with the fourth.
        let kept = &blobs[processed as usize..];
        assert_eq!(kept, [entry("d/4", 4), entry("d/5", 5)]);
    }

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

    /// The presigned upload URL must bind `Content-Length`. S3 then rejects any
    /// body whose length differs from the declared (and cap-validated) size.
    /// The key-scoped URL cannot be used to write an arbitrarily large object.
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

    /// `ZK-014`, `SRV-052`: a configured lifetime above 15 minutes is signed
    /// as 15 minutes, on both the upload and the download URL.
    #[tokio::test]
    async fn a_presigned_url_never_outlives_fifteen_minutes() {
        let s3 = test_s3_client();
        let day = 24 * 3600;
        let up = generate_presigned_upload_url(&s3, "bucket", "k", day, 1)
            .await
            .expect("presign");
        let down = generate_presigned_download_url(&s3, "bucket", "k", day)
            .await
            .expect("presign");
        for url in [up, down] {
            assert!(url.contains("X-Amz-Expires=900"), "{url}");
        }
        let short = generate_presigned_download_url(&s3, "bucket", "k", 60)
            .await
            .expect("presign");
        assert!(short.contains("X-Amz-Expires=60"), "{short}");
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
