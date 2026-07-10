use crate::error::AppError;
use rustis::client::Client;
use rustis::commands::{
    GenericCommands, HashCommands, ListCommands, PubSubCommands, ScriptingCommands,
};
use rustis::resp::RefBulkString;

/// Atomic Lua script that validates a client clock and assigns a
/// global monotonic sequence number.
///
/// - `KEYS[1]` = `doc:{id}:clocks` (hash: pub_key_hex -> last_clock)
/// - `KEYS[2]` = `doc:{id}:seq`   (counter)
/// - `ARGV[1]` = pub_key_hex
/// - `ARGV[2]` = incoming clock (must be last_clock + 1)
///
/// Rejects the update if the clock is not strictly sequential.
///
/// # Why the clock hash is intentionally not pruned
///
/// `doc:{id}:clocks` grows one field per distinct authoring signing key, which
/// is bounded by the document's active device set (each member device signs
/// with one identity key). It is deliberately never expired or trimmed:
///
/// - A TTL that dropped the hash on a cold document would reset `current` to 0.
///   A legitimate author returning with its persisted monotonic clock at N > 1
///   would then fail `N == 0 + 1` forever -- the strict-clock invariant would
///   lock them out of their own document.
/// - Per-author pruning on member revoke is not possible here either: the
///   server only ever sees the signing key inside an update envelope and has no
///   `user_id -> signing_key` mapping to target.
///
/// Removing an entry would also reopen the replay window this hash exists to
/// close (a pruned author could resubmit old signed updates from clock 1). The
/// membership bound is the correct and safe ceiling.
pub const CLOCK_AND_SEQ_LUA: &str = r#"
local current  = tonumber(redis.call('HGET', KEYS[1], ARGV[1])) or 0
local incoming = tonumber(ARGV[2])
if incoming ~= current + 1 then
    return redis.error_reply('CLOCK_MISMATCH:' .. current)
end
redis.call('HSET', KEYS[1], ARGV[1], incoming)
return redis.call('INCR', KEYS[2])
"#;

/// Validates a client's logical clock and assigns a global sequence number.
///
/// Executes `CLOCK_AND_SEQ_LUA` atomically in Redis. Returns the assigned
/// global `seq` if the clock is exactly `last_clock + 1`, otherwise returns
/// `AppError::Redis` with a `CLOCK_MISMATCH` detail.
pub async fn verify_clock_and_assign_seq(
    redis: &Client,
    document_id: &ambre_core::DocumentId,
    pub_key_hex: &str,
    clock: u64,
) -> Result<u64, AppError> {
    let clocks_key = format!("doc:{document_id}:clocks");
    let seq_key = format!("doc:{document_id}:seq");

    let result: i64 = redis
        .eval::<i64>(
            CLOCK_AND_SEQ_LUA,
            [clocks_key.as_str(), seq_key.as_str()],
            [pub_key_hex, &clock.to_string()],
        )
        .await
        .map_err(|e| AppError::Redis(format!("clock validation failed: {e}")))?;

    Ok(result as u64)
}

/// Appends a delta entry to the pending list for this document.
///
/// The entry is a JSON blob `{ seq, s3_key, update_b64 }` carrying the full
/// sealed Update wire, which the global S3 flusher picks up every 15 seconds
/// and uploads to S3. The same bytes are replayed verbatim on reconnect.
pub async fn append_pending_delta(
    redis: &Client,
    document_id: &ambre_core::DocumentId,
    seq: u64,
    s3_key: &str,
    update_wire: &[u8],
) -> Result<(), AppError> {
    use base64::Engine;
    let key = format!("doc:{document_id}:pending");
    // Store the FULL sealed Update wire (ciphertext + nonce + public_data +
    // commitment + signature), not just the ciphertext: a reconnecting or
    // cross-device client replays these verbatim and needs every field to
    // decrypt and verify. The flusher persists the same bytes to S3.
    let update_b64 = base64::engine::general_purpose::URL_SAFE.encode(update_wire);
    let blob = serde_json::json!({
        "seq": seq,
        "s3_key": s3_key,
        "update_b64": update_b64,
    })
    .to_string();
    redis
        .rpush(&key, [&blob])
        .await
        .map_err(|e| AppError::Redis(format!("pending push failed: {e}")))?;
    Ok(())
}

/// Pushes a delta that could not be enqueued for S3 flushing onto a durable
/// dead-letter list (`doc:{id}:deadletter`) so an operator can inspect and
/// replay it (H1).
///
/// This is the fallback when `append_pending_delta` fails: the pending list is
/// the only path a sequenced delta takes to S3, so without a dead-letter the
/// seq is silently lost from the durable index after it has already been
/// assigned and acknowledged. The list is never trimmed automatically; it is an
/// operator signal, not a hot path.
pub async fn push_dead_letter(
    redis: &Client,
    document_id: &ambre_core::DocumentId,
    seq: u64,
    s3_key: &str,
    update_wire: &[u8],
) -> Result<(), AppError> {
    use base64::Engine;
    let key = format!("doc:{document_id}:deadletter");
    let update_b64 = base64::engine::general_purpose::URL_SAFE.encode(update_wire);
    let blob = serde_json::json!({
        "seq": seq,
        "s3_key": s3_key,
        "update_b64": update_b64,
    })
    .to_string();
    redis
        .rpush(&key, [&blob])
        .await
        .map_err(|e| AppError::Redis(format!("dead-letter push failed: {e}")))?;
    Ok(())
}

/// Loads the whole pending list of a document into a `seq -> full Update wire`
/// map in ONE round-trip.
///
/// Deltas live in `doc:{id}:pending` until the background flusher uploads them
/// to S3 and trims the list. A replay requested before the flush finds them here
/// rather than in S3, so the two stores together cover every seq.
///
/// Replay resolves each missed seq from S3 first and falls back to a still-
/// pending delta. Doing that fallback with a per-seq lookup issued one full
/// `LRANGE` (plus a JSON re-parse of the entire list) for EACH missed seq inside
/// the concurrent fan-out -- O(N^2) on the pre-flush window. Loading the list
/// once turns every fallback into a map lookup.
pub async fn get_pending_deltas_map(
    redis: &Client,
    document_id: &ambre_core::DocumentId,
) -> Result<std::collections::HashMap<u64, Vec<u8>>, AppError> {
    use base64::Engine;
    let key = format!("doc:{document_id}:pending");
    let blobs: Vec<String> = redis
        .lrange(&key, 0, -1)
        .await
        .map_err(|e| AppError::Redis(format!("pending lrange failed: {e}")))?;
    let mut map = std::collections::HashMap::with_capacity(blobs.len());
    for blob in &blobs {
        let Ok(parsed) = serde_json::from_str::<serde_json::Value>(blob) else {
            continue;
        };
        let (Some(seq), Some(b64)) = (parsed["seq"].as_u64(), parsed["update_b64"].as_str()) else {
            continue;
        };
        if let Ok(wire) = base64::engine::general_purpose::URL_SAFE.decode(b64) {
            map.insert(seq, wire);
        }
    }
    Ok(map)
}

/// Tries to acquire the per-document flush lock (`SET lock_key token NX EX ttl`).
///
/// Returns `true` if this caller now holds the lock. The background S3 flusher
/// runs once per server instance; without this lock two instances can `LRANGE`
/// the same pending list, both upload, then both `LTRIM` the front by their own
/// processed count. Deltas `RPUSH`ed by clients during the flush window sit at
/// the tail and get trimmed away by the second, stale `LTRIM` -- a durable loss
/// of an already-sequenced, already-acknowledged delta. Serializing the flush
/// per document closes that race. The write path (`RPUSH` in `handle_update`)
/// never touches this lock, so collaborators are unaffected.
pub async fn try_acquire_flush_lock(
    redis: &Client,
    lock_key: &str,
    token: &str,
    ttl_secs: u64,
) -> Result<bool, AppError> {
    const LUA: &str = "return redis.call('SET', KEYS[1], ARGV[1], 'NX', 'EX', ARGV[2])";
    let ttl = ttl_secs.to_string();
    let res: Option<String> = redis
        .eval::<Option<String>>(LUA, [lock_key], [token, ttl.as_str()])
        .await
        .map_err(|e| AppError::Redis(format!("flush lock acquire: {e}")))?;
    Ok(res.is_some())
}

/// Releases the per-document flush lock, but only if `token` still matches
/// (check-and-del) so an expired-then-reacquired lock owned by another instance
/// is never dropped by a slow predecessor.
pub async fn release_flush_lock(
    redis: &Client,
    lock_key: &str,
    token: &str,
) -> Result<(), AppError> {
    const LUA: &str = "if redis.call('GET', KEYS[1]) == ARGV[1] then return redis.call('DEL', KEYS[1]) else return 0 end";
    let _: i64 = redis
        .eval::<i64>(LUA, [lock_key], [token])
        .await
        .map_err(|e| AppError::Redis(format!("flush lock release: {e}")))?;
    Ok(())
}

/// Retrieves a refresh-token record from Redis.
///
/// Refresh keys follow the pattern `refresh:{token}` with fields `user_id` and
/// `signing_pubkey_hex`. Returns `None` when the token is unknown, expired, or
/// was revoked (logout / rotation).
pub async fn refresh_get(redis: &Client, token: &str) -> Result<Option<RefreshData>, AppError> {
    let key = format!("refresh:{token}");
    // `hgetall` returns field/value pairs; rustis deserializes them as a map.
    // (Reading into `Vec<String>` fails with "invalid type: sequence, expected a
    // string" because each pair is itself a sequence.)
    let fields: std::collections::HashMap<String, String> = redis
        .hgetall(&key)
        .await
        .map_err(|e| AppError::Redis(format!("refresh get failed: {e}")))?;

    if fields.is_empty() {
        return Ok(None);
    }

    let user_id = fields.get("user_id").cloned().unwrap_or_default();
    let signing_pubkey_hex = fields
        .get("signing_pubkey_hex")
        .cloned()
        .unwrap_or_default();

    let user_id = ambre_core::Uuid::parse_str(&user_id)
        .map(ambre_core::UserId::from_uuid)
        .map_err(|e| AppError::Redis(format!("invalid user_id in refresh: {e}")))?;

    let signing_pubkey = ambre_core::PublicKey::from_hex(&signing_pubkey_hex)
        .map_err(|e| AppError::Redis(format!("invalid pubkey in refresh: {e}")))?;

    Ok(Some(RefreshData {
        user_id,
        signing_pubkey,
    }))
}

/// Creates a refresh-token record in Redis with the configured TTL.
pub async fn refresh_create(
    redis: &Client,
    token: &str,
    data: &RefreshData,
    ttl_secs: u64,
) -> Result<(), AppError> {
    let key = format!("refresh:{token}");

    redis
        .hset(
            &key,
            [
                ("user_id", data.user_id.to_string().as_str()),
                ("signing_pubkey_hex", data.signing_pubkey.to_hex().as_str()),
            ],
        )
        .await
        .map_err(|e| AppError::Redis(format!("refresh create hset: {e}")))?;

    redis
        .expire(&key, ttl_secs, None)
        .await
        .map_err(|e| AppError::Redis(format!("refresh create expire: {e}")))?;
    Ok(())
}

/// Deletes a refresh-token record (logout, or the old token during rotation).
pub async fn refresh_delete(redis: &Client, token: &str) -> Result<(), AppError> {
    let key = format!("refresh:{token}");
    redis
        .del([&key])
        .await
        .map_err(|e| AppError::Redis(format!("refresh delete failed: {e}")))?;
    Ok(())
}

/// Claims a refresh-token record, returning `true` only for the caller that
/// actually removed the key.
///
/// `DEL` is atomic and reports how many keys it removed, so when several callers
/// race on the same token exactly one observes `1`. That caller owns the claim;
/// every other one observes `0` and must be rejected as a replay.
pub async fn refresh_claim(redis: &Client, token: &str) -> Result<bool, AppError> {
    let key = format!("refresh:{token}");
    let removed: usize = redis
        .del([&key])
        .await
        .map_err(|e| AppError::Redis(format!("refresh claim del: {e}")))?;
    Ok(removed == 1)
}

/// Reads a refresh-token record and claims it, yielding the data only to the
/// caller that won the claim.
///
/// A `refresh_get` followed by an unchecked `refresh_delete` is not sufficient
/// for rotation. Two concurrent requests presenting the same token both observe
/// the record before either deletes it, so both mint a fresh session and the
/// replay goes undetected. Gating on the [`refresh_claim`] result closes that.
pub async fn refresh_consume(redis: &Client, token: &str) -> Result<Option<RefreshData>, AppError> {
    let Some(data) = refresh_get(redis, token).await? else {
        return Ok(None);
    };
    if !refresh_claim(redis, token).await? {
        return Ok(None);
    }
    Ok(Some(data))
}

/// Registers this WebSocket session as an active subscriber of the document.
///
/// Used for per-document presence tracking and broadcast targeting.
pub async fn doc_subscribe_session(
    redis: &Client,
    document_id: &ambre_core::DocumentId,
    session_id: &str,
) -> Result<(), AppError> {
    let key = format!("doc:{document_id}:sessions");
    redis
        .hset(&key, [(session_id, "1")])
        .await
        .map_err(|e| AppError::Redis(format!("doc subscribe session: {e}")))?;
    Ok(())
}

/// Removes this WebSocket session from the document's active subscribers.
pub async fn doc_unsubscribe_session(
    redis: &Client,
    document_id: &ambre_core::DocumentId,
    session_id: &str,
) -> Result<(), AppError> {
    let key = format!("doc:{document_id}:sessions");
    redis
        .hdel(&key, [session_id])
        .await
        .map_err(|e| AppError::Redis(format!("doc unsubscribe session: {e}")))?;
    Ok(())
}

/// Publishes a message to the Redis PubSub channel for the given document.
///
/// All active subscribers (across Axum instances) receive the message.
pub async fn doc_broadcast(
    redis: &Client,
    document_id: &ambre_core::DocumentId,
    payload: &[u8],
) -> Result<(), AppError> {
    let channel = format!("doc:{document_id}");
    // The payload is binary (encrypted ciphertext). rustis serializes a raw
    // `&[u8]` as a sequence of integers (a serde limitation) and panics; wrap it
    // in `RefBulkString` so it is sent as a single RESP bulk string.
    redis
        .publish(&channel, RefBulkString::new(payload))
        .await
        .map_err(|e| AppError::Redis(format!("doc broadcast: {e}")))?;
    Ok(())
}

/// Publishes a message to a user's personal PubSub channel (`user:{id}`).
///
/// Reaches every live connection of that user, whatever documents they have
/// open. Used for server-initiated, non-document-scoped pushes such as
/// `DocumentShared` on a fresh invite.
pub async fn user_broadcast(
    redis: &Client,
    user_id: &ambre_core::UserId,
    payload: &[u8],
) -> Result<(), AppError> {
    let channel = format!("user:{user_id}");
    redis
        .publish(&channel, RefBulkString::new(payload))
        .await
        .map_err(|e| AppError::Redis(format!("user broadcast: {e}")))?;
    Ok(())
}

/// Publishes a message to the Redis PubSub channel for the given channel.
pub async fn chan_broadcast(
    redis: &Client,
    channel_id: &ambre_core::ChannelId,
    payload: &[u8],
) -> Result<(), AppError> {
    let chan = format!("chan:{channel_id}");
    redis
        .publish(&chan, RefBulkString::new(payload))
        .await
        .map_err(|e| AppError::Redis(format!("channel broadcast: {e}")))?;
    Ok(())
}

/// Stores an opaque presence blob in Redis and sets a TTL.
///
/// The blob is the encrypted ephemeral ciphertext -- the server never
/// sees the plaintext cursor position or display name.
pub async fn doc_presence_update(
    redis: &Client,
    document_id: &ambre_core::DocumentId,
    pub_key_hex: &str,
    presence_enc_b64: &str,
    ttl_secs: u64,
) -> Result<(), AppError> {
    let key = format!("doc:{document_id}:presence");
    redis
        .hset(&key, [(pub_key_hex, presence_enc_b64)])
        .await
        .map_err(|e| AppError::Redis(format!("presence update: {e}")))?;

    // Expire THIS member's field individually (Redis 7.4+ / Valkey 8 `HEXPIRE`),
    // so a member who leaves without a clean unsubscribe self-prunes from the
    // hash. Without a per-field TTL the presence hash grew one dead field per
    // distinct author forever. `HEXPIRE key ttl FIELDS 1 field` returns an array
    // of per-field status codes; the outcome is advisory here (a missing field or
    // unsupported server is non-fatal for a fire-and-forget presence write).
    const HEXPIRE_LUA: &str =
        "return redis.call('HEXPIRE', KEYS[1], ARGV[1], 'FIELDS', 1, ARGV[2])";
    let ttl = ttl_secs.to_string();
    redis
        .eval::<Vec<i64>>(HEXPIRE_LUA, [key.as_str()], [ttl.as_str(), pub_key_hex])
        .await
        .map_err(|e| AppError::Redis(format!("presence hexpire: {e}")))?;
    Ok(())
}

/// Redis key for the per-document clock hash.
#[allow(dead_code)]
pub fn format_doc_clocks_key(document_id: &ambre_core::DocumentId) -> String {
    format!("doc:{document_id}:clocks")
}

/// Identity bound to a refresh token in Redis.
///
/// Kept as a simple struct so callers do not need to manually parse the flat
/// HGETALL response. Mirrors what a minted access JWT carries in its claims.
#[derive(Debug, Clone)]
pub struct RefreshData {
    pub user_id: ambre_core::UserId,
    pub signing_pubkey: ambre_core::PublicKey,
}

/// Stores OIDC authorization state in Redis with a 10-minute TTL.
///
/// The state is keyed by the CSRF token secret so it can be retrieved
/// during the callback to validate the PKCE verifier and nonce.
pub async fn oidc_state_set(
    redis: &Client,
    csrf_state: &str,
    pkce_verifier_secret: &str,
    nonce: &str,
) -> Result<(), AppError> {
    let key = format!("oidc:state:{csrf_state}");
    redis
        .hset(
            &key,
            [("pkce_verifier", pkce_verifier_secret), ("nonce", nonce)],
        )
        .await
        .map_err(|e| AppError::Redis(format!("oidc state set: {e}")))?;
    redis
        .expire(&key, 600, None)
        .await
        .map_err(|e| AppError::Redis(format!("oidc state expire: {e}")))?;
    Ok(())
}

/// Retrieves and deletes OIDC authorization state from Redis.
///
/// Returns `(pkce_verifier_secret, nonce)` on success, or an error if the state
/// expired or was not found.
pub async fn oidc_state_take(
    redis: &Client,
    csrf_state: &str,
) -> Result<(String, String), AppError> {
    let key = format!("oidc:state:{csrf_state}");
    // Read into a `HashMap`, NOT a `Vec<String>`: under RESP3 rustis returns a
    // hash reply as a map, and deserializing that into a flat `Vec<String>` fails
    // with "invalid type: sequence, expected a string" -- which surfaced here as a
    // spurious `SessionExpired` on EVERY OIDC callback (login fully broken). Mirror
    // `refresh_get`, which reads the same shape correctly.
    let fields: std::collections::HashMap<String, String> = redis
        .hgetall(&key)
        .await
        .map_err(|e| AppError::Redis(format!("oidc state get: {e}")))?;

    if fields.is_empty() {
        return Err(AppError::SessionExpired);
    }

    let pkce_verifier = fields.get("pkce_verifier").cloned().unwrap_or_default();
    let nonce = fields.get("nonce").cloned().unwrap_or_default();

    redis
        .del([&key])
        .await
        .map_err(|e| AppError::Redis(format!("oidc state delete: {e}")))?;

    Ok((pkce_verifier, nonce))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_doc_clocks_key_contains_doc_id() {
        let doc_id = ambre_core::DocumentId::from_bytes([0xabu8; 16]);
        let key = format_doc_clocks_key(&doc_id);
        assert!(key.starts_with("doc:"));
        assert!(key.ends_with(":clocks"));
        assert!(key.contains(&doc_id.to_string()));
    }
}

#[cfg(all(test, feature = "test-helpers"))]
mod lua_tests {
    use super::*;

    async fn connect_redis() -> rustis::client::Client {
        // These tests run only under `--features db`, where the backing Redis is
        // expected up (`just services-up`). A missing connection is a broken test
        // environment, not a reason to silently pass, so fail loudly.
        let url = std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://localhost:6379".into());
        rustis::client::Client::connect(url.as_str())
            .await
            .expect("db-tier tests require a reachable Redis (REDIS_URL / just services-up)")
    }

    #[tokio::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs redis; run with --features db")]
    async fn first_update_succeeds() {
        let redis = connect_redis().await;
        let doc_id = ambre_core::DocumentId::new();
        let pk_hex = "abc123abc123abc123abc123abc123abc123abc123abc123abc123abc123abc1";

        let seq = verify_clock_and_assign_seq(&redis, &doc_id, pk_hex, 1)
            .await
            .expect("first update should succeed");
        assert_eq!(seq, 1);

        let _ = redis
            .del([format_doc_clocks_key(&doc_id), format!("doc:{doc_id}:seq")])
            .await
            .unwrap();
    }

    #[tokio::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs redis; run with --features db")]
    async fn sequential_clock_succeeds() {
        let redis = connect_redis().await;
        let doc_id = ambre_core::DocumentId::new();
        let pk = "def456def456def456def456def456def456def456def456def456def456def4";

        let s1 = verify_clock_and_assign_seq(&redis, &doc_id, pk, 1)
            .await
            .unwrap();
        let s2 = verify_clock_and_assign_seq(&redis, &doc_id, pk, 2)
            .await
            .unwrap();
        let s3 = verify_clock_and_assign_seq(&redis, &doc_id, pk, 3)
            .await
            .unwrap();
        assert_eq!(s1, 1);
        assert_eq!(s2, 2);
        assert_eq!(s3, 3);

        let _ = redis
            .del([format_doc_clocks_key(&doc_id), format!("doc:{doc_id}:seq")])
            .await
            .unwrap();
    }

    #[tokio::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs redis; run with --features db")]
    async fn clock_mismatch_rejected() {
        let redis = connect_redis().await;
        let doc_id = ambre_core::DocumentId::new();
        let pk = "1111111111111111111111111111111111111111111111111111111111111111";

        verify_clock_and_assign_seq(&redis, &doc_id, pk, 1)
            .await
            .unwrap();
        let err = verify_clock_and_assign_seq(&redis, &doc_id, pk, 3)
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::Redis(_)));

        let _ = redis
            .del([format_doc_clocks_key(&doc_id), format!("doc:{doc_id}:seq")])
            .await
            .unwrap();
    }

    #[tokio::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs redis; run with --features db")]
    async fn duplicate_clock_rejected() {
        let redis = connect_redis().await;
        let doc_id = ambre_core::DocumentId::new();
        let pk = "2222222222222222222222222222222222222222222222222222222222222222";

        verify_clock_and_assign_seq(&redis, &doc_id, pk, 1)
            .await
            .unwrap();
        let err = verify_clock_and_assign_seq(&redis, &doc_id, pk, 1)
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::Redis(_)));

        let _ = redis
            .del([format_doc_clocks_key(&doc_id), format!("doc:{doc_id}:seq")])
            .await
            .unwrap();
    }

    #[tokio::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs redis; run with --features db")]
    async fn independent_clients_have_independent_clocks() {
        let redis = connect_redis().await;
        let doc_id = ambre_core::DocumentId::new();
        let pk_a = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let pk_b = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

        let s_a = verify_clock_and_assign_seq(&redis, &doc_id, pk_a, 1)
            .await
            .unwrap();
        let s_b = verify_clock_and_assign_seq(&redis, &doc_id, pk_b, 1)
            .await
            .unwrap();
        let s_a2 = verify_clock_and_assign_seq(&redis, &doc_id, pk_a, 2)
            .await
            .unwrap();

        assert_eq!(s_a, 1);
        assert_eq!(s_b, 2);
        assert_eq!(s_a2, 3);

        let _ = redis
            .del([format_doc_clocks_key(&doc_id), format!("doc:{doc_id}:seq")])
            .await
            .unwrap();
    }

    #[test]
    fn lua_script_contains_key_validation() {
        assert!(CLOCK_AND_SEQ_LUA.contains("CLOCK_MISMATCH"));
        assert!(CLOCK_AND_SEQ_LUA.contains("HGET"));
        assert!(CLOCK_AND_SEQ_LUA.contains("INCR"));
        assert!(CLOCK_AND_SEQ_LUA.contains("+ 1"));
        assert!(CLOCK_AND_SEQ_LUA.contains("HSET"));
    }
}

// Redis integration tests for the flush lock, pending-delta map, and per-field
// presence TTL. Gated by the `db` feature (like the other integration tests) so
// they run under `./run_tests.sh db`, not only behind `test-helpers`.
#[cfg(test)]
mod redis_integration_tests {
    use super::*;

    async fn connect_redis() -> rustis::client::Client {
        // These tests run only under `--features db`, where the backing Redis is
        // expected up (`just services-up`). A missing connection is a broken test
        // environment, not a reason to silently pass, so fail loudly.
        let url = std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://localhost:6379".into());
        rustis::client::Client::connect(url.as_str())
            .await
            .expect("db-tier tests require a reachable Redis (REDIS_URL / just services-up)")
    }

    #[tokio::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs redis; run with --features db")]
    async fn flush_lock_is_mutually_exclusive() {
        let redis = connect_redis().await;
        let lock_key = format!("test:flush:lock:{}", ambre_core::DocumentId::new());
        let _ = redis.del([lock_key.as_str()]).await;

        // First caller acquires; a concurrent instance cannot while it is held.
        assert!(
            try_acquire_flush_lock(&redis, &lock_key, "tokA", 60)
                .await
                .unwrap()
        );
        assert!(
            !try_acquire_flush_lock(&redis, &lock_key, "tokB", 60)
                .await
                .unwrap()
        );

        // A non-owner (wrong token) must not be able to release it.
        release_flush_lock(&redis, &lock_key, "tokB").await.unwrap();
        assert!(
            !try_acquire_flush_lock(&redis, &lock_key, "tokB", 60)
                .await
                .unwrap()
        );

        // The real owner releases; the lock is now free again.
        release_flush_lock(&redis, &lock_key, "tokA").await.unwrap();
        assert!(
            try_acquire_flush_lock(&redis, &lock_key, "tokB", 60)
                .await
                .unwrap()
        );

        let _ = redis.del([lock_key.as_str()]).await;
    }

    #[tokio::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs redis; run with --features db")]
    async fn pending_map_roundtrips_seq_to_wire() {
        let redis = connect_redis().await;
        let doc = ambre_core::DocumentId::new();
        let key = format!("doc:{doc}:pending");
        let _ = redis.del([key.as_str()]).await;

        append_pending_delta(&redis, &doc, 7, "s3-a", &[1, 2, 3])
            .await
            .unwrap();
        append_pending_delta(&redis, &doc, 8, "s3-b", &[4, 5])
            .await
            .unwrap();

        let map = get_pending_deltas_map(&redis, &doc).await.unwrap();
        assert_eq!(map.len(), 2);
        assert_eq!(map.get(&7).map(Vec::as_slice), Some(&[1u8, 2, 3][..]));
        assert_eq!(map.get(&8).map(Vec::as_slice), Some(&[4u8, 5][..]));

        let _ = redis.del([key.as_str()]).await;
    }

    #[tokio::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs redis; run with --features db")]
    async fn oidc_state_roundtrips() {
        let redis = connect_redis().await;
        let csrf = uuid::Uuid::new_v4().to_string();
        oidc_state_set(&redis, &csrf, "verifier-xyz", "nonce-abc")
            .await
            .expect("oidc state set");
        let (v, n) = oidc_state_take(&redis, &csrf)
            .await
            .expect("oidc state take");
        assert_eq!(v, "verifier-xyz");
        assert_eq!(n, "nonce-abc");
    }

    #[tokio::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs redis; run with --features db")]
    async fn presence_field_self_expires() {
        let redis = connect_redis().await;
        let doc = ambre_core::DocumentId::new();
        let key = format!("doc:{doc}:presence");
        let pk = "deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef0";
        let _ = redis.del([key.as_str()]).await;

        doc_presence_update(&redis, &doc, pk, "blob", 15)
            .await
            .unwrap();

        // HTTL returns the remaining per-field TTL; a positive value proves the
        // field will self-prune instead of leaking forever.
        let ttl: Vec<i64> = redis
            .eval::<Vec<i64>>(
                "return redis.call('HTTL', KEYS[1], 'FIELDS', 1, ARGV[1])",
                [key.as_str()],
                [pk],
            )
            .await
            .unwrap();
        assert_eq!(ttl.len(), 1);
        assert!(
            ttl[0] > 0 && ttl[0] <= 15,
            "expected a positive ttl, got {ttl:?}"
        );

        let _ = redis.del([key.as_str()]).await;
    }

    async fn seed_refresh(redis: &Client) -> String {
        let token = uuid::Uuid::new_v4().to_string();
        let data = RefreshData {
            user_id: ambre_core::UserId::new(),
            signing_pubkey: ambre_core::PublicKey::from_array([3u8; 32]),
        };
        refresh_create(redis, &token, &data, 60)
            .await
            .expect("refresh create");
        token
    }

    /// Reproduces the rotation race: two requests bearing the same token both
    /// read the record before either deletes it. Only the caller whose `DEL`
    /// actually removed the key may claim it; the other is a replay.
    #[tokio::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs redis; run with --features db")]
    async fn only_one_racing_caller_claims_a_refresh_token() {
        let redis = connect_redis().await;
        let token = seed_refresh(&redis).await;

        // Both callers observe the record: this is the interleaving that makes an
        // unchecked delete unsafe.
        assert!(refresh_get(&redis, &token).await.expect("get a").is_some());
        assert!(refresh_get(&redis, &token).await.expect("get b").is_some());

        assert!(refresh_claim(&redis, &token).await.expect("claim a"));
        assert!(
            !refresh_claim(&redis, &token).await.expect("claim b"),
            "a token already claimed by a concurrent request must not be claimable again"
        );
    }

    /// End to end on the rotation path: the second presentation of a token that
    /// was already consumed yields nothing, so the handler answers 401.
    #[tokio::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs redis; run with --features db")]
    async fn refresh_consume_rejects_a_replayed_token() {
        let redis = connect_redis().await;
        let token = seed_refresh(&redis).await;

        assert!(
            refresh_consume(&redis, &token)
                .await
                .expect("first")
                .is_some()
        );
        assert!(
            refresh_consume(&redis, &token)
                .await
                .expect("replay")
                .is_none()
        );
    }
}
