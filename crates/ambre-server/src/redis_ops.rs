use crate::error::AppError;
use rustis::client::Client;
use rustis::commands::{
    GenericCommands, HashCommands, ListCommands, PubSubCommands, ScriptingCommands,
};
use rustis::resp::RefBulkString;

/// Atomic Lua script that validates a client clock and assigns a global
/// monotonic sequence number.
///
/// - `KEYS[1]` = `doc:{id}:clocks` (hash: pub_key_hex -> last_clock)
/// - `KEYS[2]` = `doc:{id}:seq`   (counter)
/// - `KEYS[3]` = `doc:{id}:gone`  (tombstone, see [`mark_document_gone`])
/// - `ARGV[1]` = pub_key_hex
/// - `ARGV[2]` = incoming clock (must be last_clock + 1)
///
/// Rejects the update if the clock is not strictly sequential, or if the
/// document is being deleted.
///
/// # Why the clock hash is intentionally not pruned
///
/// `doc:{id}:clocks` grows one field per distinct authoring signing key. That is
/// bounded by the document's active device set: each member device signs with
/// one identity key. It is deliberately never expired or trimmed:
///
/// - A TTL that dropped the hash on a cold document would reset `current` to 0.
///   A legitimate author returning with its persisted monotonic clock at N > 1
///   would then fail `N == 0 + 1` forever. The strict-clock invariant would lock
///   them out of their own document.
/// - Per-author pruning on member revoke is not possible here either. The server
///   only ever sees the signing key inside an update envelope, and has no
///   `user_id -> signing_key` mapping to target.
///
/// Removing an entry would also reopen the replay window this hash exists to
/// close (a pruned author could resubmit old signed updates from clock 1). The
/// membership bound is the correct and safe ceiling.
pub const CLOCK_AND_SEQ_LUA: &str = r#"
if redis.call('EXISTS', KEYS[3]) == 1 then
    return redis.error_reply('DOCUMENT_GONE')
end
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
/// global `seq` if the clock is exactly `last_clock + 1`. A refused clock is the
/// typed [`AppError::ClockMismatch`], carrying the clock the relay expected and
/// the one it received (`SRV-037`); any other failure is the store's.
pub async fn verify_clock_and_assign_seq(
    redis: &Client,
    document_id: &ambre_core::DocumentId,
    pub_key_hex: &str,
    clock: u64,
) -> Result<u64, AppError> {
    let clocks_key = format!("doc:{document_id}:clocks");
    let seq_key = format!("doc:{document_id}:seq");
    let gone_key = document_gone_key(document_id);

    let result: i64 = redis
        .eval::<i64>(
            CLOCK_AND_SEQ_LUA,
            [clocks_key.as_str(), seq_key.as_str(), gone_key.as_str()],
            [pub_key_hex, &clock.to_string()],
        )
        .await
        .map_err(|e| clock_refusal(&e.to_string(), clock))?;

    Ok(result as u64)
}

/// Reads the script's `CLOCK_MISMATCH:<current>` refusal out of a store error.
/// The expected clock is the one after the current. A document being deleted
/// is not found; anything else stays a store failure.
fn clock_refusal(error: &str, got: u64) -> AppError {
    if error.contains(DOCUMENT_GONE) {
        return AppError::NotFound {
            resource: "document",
        };
    }
    let current = error.split_once("CLOCK_MISMATCH:").and_then(|(_, rest)| {
        let digits = rest.trim_start();
        let end = digits
            .find(|c: char| !c.is_ascii_digit())
            .unwrap_or(digits.len());
        digits[..end].parse::<u64>().ok()
    });
    match current {
        Some(current) => AppError::ClockMismatch {
            expected: current + 1,
            got,
        },
        None => AppError::Redis(format!("clock validation failed: {error}")),
    }
}

/// Reseeds a cold Redis clock/seq state to the durable high-water marks, using
/// `MAX` semantics. It can never lower a value a concurrent live update already
/// raised.
///
/// - `KEYS[1]` = `doc:{id}:clocks` (hash: pub_key_hex -> last_clock)
/// - `KEYS[2]` = `doc:{id}:seq`    (counter)
/// - `KEYS[3]` = `doc:{id}:gone`   (tombstone: a document being deleted is not reseeded)
/// - `ARGV[1]` = durable max seq
/// - `ARGV[2i], ARGV[2i+1]` = pub_key_hex, durable max clock (pairs)
const REHYDRATE_CLOCK_LUA: &str = r#"
if redis.call('EXISTS', KEYS[3]) == 1 then return 0 end
local seqmax = tonumber(ARGV[1])
local cur = tonumber(redis.call('GET', KEYS[2])) or 0
if seqmax > cur then redis.call('SET', KEYS[2], seqmax) end
local i = 2
while i < #ARGV do
    local existing = tonumber(redis.call('HGET', KEYS[1], ARGV[i])) or 0
    local incoming = tonumber(ARGV[i + 1])
    if incoming > existing then redis.call('HSET', KEYS[1], ARGV[i], incoming) end
    i = i + 2
end
return 1
"#;

/// Whether the document's clock-validation state is absent from Redis. True after a
/// Redis reset while Postgres kept the history. The per-author clock hash was the
/// durable authority and is now gone, so it must be rehydrated before any update is
/// validated. Otherwise `CLOCK_MISMATCH`, since clients hold advanced clocks.
pub async fn clock_state_is_cold(
    redis: &Client,
    document_id: &ambre_core::DocumentId,
) -> Result<bool, AppError> {
    let clocks_key = format!("doc:{document_id}:clocks");
    let existing: usize = redis
        .exists(clocks_key)
        .await
        .map_err(|e| AppError::Redis(format!("clock state EXISTS failed: {e}")))?;
    Ok(existing == 0)
}

/// Reseeds the Redis clock/seq state from the durable per-author high-water marks
/// (see [`REHYDRATE_CLOCK_LUA`]). `author_clocks` are `(pub_key_hex, max_clock)`.
/// A no-op when there is no durable history (a brand-new document).
pub async fn seed_clock_state(
    redis: &Client,
    document_id: &ambre_core::DocumentId,
    max_seq: i64,
    author_clocks: &[(String, i64)],
) -> Result<(), AppError> {
    if author_clocks.is_empty() {
        return Ok(());
    }
    let clocks_key = format!("doc:{document_id}:clocks");
    let seq_key = format!("doc:{document_id}:seq");
    let gone_key = document_gone_key(document_id);
    let mut argv = vec![max_seq.to_string()];
    for (hex, clock) in author_clocks {
        argv.push(hex.clone());
        argv.push(clock.to_string());
    }
    redis
        .eval::<i64>(
            REHYDRATE_CLOCK_LUA,
            [clocks_key.as_str(), seq_key.as_str(), gone_key.as_str()],
            argv.iter().map(String::as_str).collect::<Vec<_>>(),
        )
        .await
        .map_err(|e| AppError::Redis(format!("clock state rehydrate failed: {e}")))?;
    Ok(())
}

/// Appends a delta entry to the pending list for this document.
///
/// The entry is a JSON blob `{ seq, s3_key, update_b64 }` carrying the full
/// sealed Update wire. The global S3 flusher picks it up every 15 seconds
/// and uploads it to S3. The same bytes are replayed verbatim on reconnect.
///
/// A document being deleted takes no entry, and the call is refused as not
/// found. An update in flight cannot recreate the list the delete purged
/// (`SRV-017`).
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
    // commitment + signature), not just the ciphertext. A reconnecting or
    // cross-device client replays these verbatim, and needs every field to
    // decrypt and verify. The flusher persists the same bytes to S3.
    let update_b64 = base64::engine::general_purpose::URL_SAFE.encode(update_wire);
    let blob = serde_json::json!({
        "seq": seq,
        "s3_key": s3_key,
        "update_b64": update_b64,
    })
    .to_string();
    // One script: the tombstone check, the RPUSH, then the dirty mark. The
    // flusher iterates only documents marked dirty (SMEMBERS), instead of
    // SCANning the whole keyspace every cycle. The RPUSH-then-SADD order is
    // load-bearing: `clear_pending_dirty_if_empty` relies on the entry being in
    // the list whenever the marker exists, and the script runs atomically.
    const LUA: &str = r#"
if redis.call('EXISTS', KEYS[3]) == 1 then
    return redis.error_reply('DOCUMENT_GONE')
end
redis.call('RPUSH', KEYS[1], ARGV[1])
redis.call('SADD', KEYS[2], KEYS[1])
return 1
"#;
    let gone_key = document_gone_key(document_id);
    redis
        .eval::<i64>(
            LUA,
            [key.as_str(), FLUSH_DIRTY_SET, gone_key.as_str()],
            [blob.as_str()],
        )
        .await
        .map_err(|e| store_refusal(&e.to_string(), "pending push failed"))?;
    Ok(())
}

/// The refusal every tombstone-guarded script answers for a document being
/// deleted (see [`mark_document_gone`]).
const DOCUMENT_GONE: &str = "DOCUMENT_GONE";

/// Reads a guarded script's refusal out of a store error: a document being
/// deleted is not found, anything else is a store failure.
fn store_refusal(error: &str, what: &str) -> AppError {
    if error.contains(DOCUMENT_GONE) {
        AppError::NotFound {
            resource: "document",
        }
    } else {
        AppError::Redis(format!("{what}: {error}"))
    }
}

/// RPUSHes `blob` onto `list_key` unless `gone_key` exists. The flusher and
/// the dead-letter path use it, so neither recreates a purged list.
async fn guarded_rpush(
    redis: &Client,
    list_key: &str,
    gone_key: &str,
    blob: &str,
) -> Result<(), AppError> {
    const LUA: &str = r#"
if redis.call('EXISTS', KEYS[2]) == 1 then
    return redis.error_reply('DOCUMENT_GONE')
end
return redis.call('RPUSH', KEYS[1], ARGV[1])
"#;
    let _: i64 = redis
        .eval::<i64>(LUA, [list_key, gone_key], [blob])
        .await
        .map_err(|e| store_refusal(&e.to_string(), "guarded push failed"))?;
    Ok(())
}

/// Dead-letters a poison pending entry the flusher found, unless the document
/// is being deleted. `prefix` is the document's `doc:{id}` key prefix.
pub async fn push_poison_entry(redis: &Client, prefix: &str, blob: &str) -> Result<(), AppError> {
    guarded_rpush(
        redis,
        &format!("{prefix}:deadletter"),
        &format!("{prefix}:gone"),
        blob,
    )
    .await
}

/// Redis SET holding the `doc:{id}:pending` keys that currently have unflushed
/// deltas. The flusher reads this instead of SCANning the keyspace.
pub const FLUSH_DIRTY_SET: &str = "flush:dirty";

/// Records `pending_key` in the dirty set so the flusher will visit it.
pub async fn mark_pending_dirty(redis: &Client, pending_key: &str) -> Result<(), AppError> {
    use rustis::commands::SetCommands;
    redis
        .sadd(FLUSH_DIRTY_SET, [pending_key])
        .await
        .map_err(|e| AppError::Redis(format!("mark pending dirty: {e}")))?;
    Ok(())
}

/// Returns the pending keys currently marked dirty (the flusher's work list).
pub async fn dirty_pending_keys(redis: &Client) -> Result<Vec<String>, AppError> {
    use rustis::commands::SetCommands;
    redis
        .smembers(FLUSH_DIRTY_SET)
        .await
        .map_err(|e| AppError::Redis(format!("dirty pending keys: {e}")))
}

/// Atomically clears `pending_key` from the dirty set, but ONLY if its pending
/// list is now empty. Run after a drain. An entry appended concurrently (RPUSH
/// before its SADD, see `append_pending_delta`) is already visible in the list
/// here. So `LLEN > 0` keeps the marker, and the delta is retried next cycle:
/// an acknowledged delta is never stranded.
pub async fn clear_pending_dirty_if_empty(
    redis: &Client,
    pending_key: &str,
) -> Result<(), AppError> {
    const LUA: &str = "if redis.call('LLEN', KEYS[1]) == 0 then return redis.call('SREM', KEYS[2], ARGV[1]) else return 0 end";
    let _: i64 = redis
        .eval::<i64>(LUA, [pending_key, FLUSH_DIRTY_SET], [pending_key])
        .await
        .map_err(|e| AppError::Redis(format!("clear pending dirty: {e}")))?;
    Ok(())
}

/// Pushes a delta that could not be enqueued for S3 flushing onto a durable
/// dead-letter list (`doc:{id}:deadletter`). An operator can then inspect and
/// replay it (H1).
///
/// This is the fallback when `append_pending_delta` fails. The pending list is
/// the only path a sequenced delta takes to S3. Without a dead-letter the seq
/// is silently lost from the durable index, after it has already been assigned
/// and acknowledged. The list is never trimmed automatically; it is an operator
/// signal, not a hot path.
pub async fn push_dead_letter(
    redis: &Client,
    document_id: &ambre_core::DocumentId,
    seq: u64,
    s3_key: &str,
    update_wire: &[u8],
) -> Result<(), AppError> {
    use base64::Engine;
    let update_b64 = base64::engine::general_purpose::URL_SAFE.encode(update_wire);
    let blob = serde_json::json!({
        "seq": seq,
        "s3_key": s3_key,
        "update_b64": update_b64,
    })
    .to_string();
    guarded_rpush(
        redis,
        &format!("doc:{document_id}:deadletter"),
        &document_gone_key(document_id),
        &blob,
    )
    .await
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
/// `LRANGE` for EACH missed seq, inside the concurrent fan-out. Each one
/// re-parsed the entire list as JSON. That is O(N^2) on the pre-flush window.
/// Loading the list once turns every fallback into a map lookup.
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
/// runs once per server instance. Without this lock two instances can `LRANGE`
/// the same pending list, both upload, then both `LTRIM` the front by their own
/// processed count. Deltas `RPUSH`ed by clients during the flush window sit at
/// the tail, and get trimmed away by the second, stale `LTRIM`. That is a durable
/// loss of an already-sequenced, already-acknowledged delta; serializing the
/// flush per document closes that race. The write path (`RPUSH` in
/// `handle_update`) never touches this lock, so collaborators are unaffected.
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
/// (check-and-del). An expired-then-reacquired lock owned by another instance
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

    let user_id = ambre_core::Uuid::parse_str(&user_id)
        .map(ambre_core::UserId::from_uuid)
        .map_err(|e| AppError::Redis(format!("invalid user_id in refresh: {e}")))?;

    // An absent field is a session without keys; a present one must parse.
    let signing_pubkey = fields
        .get("signing_pubkey_hex")
        .map(|hex| ambre_core::PublicKey::from_hex(hex))
        .transpose()
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

    let user_id = data.user_id.to_string();
    let signing_pubkey_hex = data
        .signing_pubkey
        .as_ref()
        .map(ambre_core::PublicKey::to_hex);
    let mut fields = vec![("user_id", user_id.as_str())];
    if let Some(hex) = &signing_pubkey_hex {
        fields.push(("signing_pubkey_hex", hex.as_str()));
    }
    redis
        .hset(&key, fields)
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
/// the record before either deletes it. Both then mint a fresh session, and the
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

/// Publishes a message to the Redis PubSub topic for the given document.
///
/// All active subscribers (across Axum instances) receive the message.
pub async fn doc_broadcast(
    redis: &Client,
    document_id: &ambre_core::DocumentId,
    payload: &[u8],
) -> Result<(), AppError> {
    let channel = format!("doc:{document_id}");
    // The payload is binary (encrypted ciphertext). rustis serializes a raw
    // `&[u8]` as a sequence of integers (a serde limitation) and panics. Wrap it
    // in `RefBulkString` so it is sent as a single RESP bulk string.
    redis
        .publish(&channel, RefBulkString::new(payload))
        .await
        .map_err(|e| AppError::Redis(format!("doc broadcast: {e}")))?;
    Ok(())
}

/// Publishes a message to a user's personal PubSub topic (`user:{id}`).
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

/// Publishes a message to the Redis PubSub topic for the given channel.
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
/// sees the plaintext cursor position or display name. A document being
/// deleted takes no presence (`SRV-017`).
pub async fn doc_presence_update(
    redis: &Client,
    document_id: &ambre_core::DocumentId,
    pub_key_hex: &str,
    presence_enc_b64: &str,
    ttl_secs: u64,
) -> Result<(), AppError> {
    // Expire THIS member's field individually (Redis 7.4+ / Valkey 8 `HEXPIRE`),
    // so a member who leaves without a clean unsubscribe self-prunes from the
    // hash. Without a per-field TTL the presence hash grew one dead field per
    // distinct author forever. The tombstone check, the write and its TTL run
    // as one script, so a field never lands without its expiry.
    const LUA: &str = r#"
if redis.call('EXISTS', KEYS[2]) == 1 then
    return redis.error_reply('DOCUMENT_GONE')
end
redis.call('HSET', KEYS[1], ARGV[2], ARGV[3])
return redis.call('HEXPIRE', KEYS[1], ARGV[1], 'FIELDS', 1, ARGV[2])
"#;
    let key = format!("doc:{document_id}:presence");
    let gone_key = document_gone_key(document_id);
    let ttl = ttl_secs.to_string();
    redis
        .eval::<Vec<i64>>(
            LUA,
            [key.as_str(), gone_key.as_str()],
            [ttl.as_str(), pub_key_hex, presence_enc_b64],
        )
        .await
        .map_err(|e| store_refusal(&e.to_string(), "presence update"))?;
    Ok(())
}

/// Drop a member's presence field when they stop watching a document: a clean
/// unsubscribe or a closed connection (`1-Foundations/ambre-spec-presence.md`,
/// `PRESENCE-014`). The per-field TTL of `doc_presence_update` is the fallback
/// for a connection that dies without either; this is the prompt path.
///
/// Removing a field nobody wrote is not an error: the relay drops what it holds
/// and says so on the document topic either way.
pub async fn doc_presence_remove(
    redis: &Client,
    document_id: &ambre_core::DocumentId,
    pub_key_hex: &str,
) -> Result<(), AppError> {
    let key = format!("doc:{document_id}:presence");
    redis
        .hdel(&key, pub_key_hex)
        .await
        .map_err(|e| AppError::Redis(format!("presence remove: {e}")))?;
    Ok(())
}

/// The tombstone of a document being deleted. While it lives, every script
/// that writes a key of the document refuses. A write in flight when the
/// delete lands cannot recreate what the delete purges (`SRV-017`).
pub fn document_gone_key(document_id: &ambre_core::DocumentId) -> String {
    format!("doc:{document_id}:gone")
}

/// Redis sorted set of the documents whose object prefix is swept again, each
/// scored by the Unix second the sweep is due (`SRV-057`).
pub const SWEEP_DUE_SET: &str = "sweep:due";

/// Sets the tombstone of a document for `ttl_secs`: the first step of a
/// delete, before any object or key is removed.
pub async fn mark_document_gone(
    redis: &Client,
    document_id: &ambre_core::DocumentId,
    ttl_secs: u64,
) -> Result<(), AppError> {
    const LUA: &str = "return redis.call('SET', KEYS[1], '1', 'EX', ARGV[1])";
    let gone_key = document_gone_key(document_id);
    let ttl = ttl_secs.to_string();
    let _: String = redis
        .eval::<String>(LUA, [gone_key.as_str()], [ttl.as_str()])
        .await
        .map_err(|e| AppError::Redis(format!("mark document gone: {e}")))?;
    Ok(())
}

/// Removes every key the relay holds for a deleted document (`SRV-017`). That
/// is the sequence counter, the per-author clock hash, the pending list and
/// its dirty marker, the presence hash and the dead-letter list. One script, so
/// no reader sees a half-purged document.
///
/// The same script renews the tombstone for `ttl_secs` and schedules a second
/// sweep of the document's objects at `sweep_due` (Unix seconds). That sweep
/// catches an object a late writer put after the first one (`SRV-057`).
pub async fn purge_document_state(
    redis: &Client,
    document_id: &ambre_core::DocumentId,
    ttl_secs: u64,
    sweep_due: u64,
) -> Result<(), AppError> {
    const LUA: &str = r#"
redis.call('SREM', KEYS[6], KEYS[3])
redis.call('DEL', KEYS[1], KEYS[2], KEYS[3], KEYS[4], KEYS[5])
redis.call('SET', KEYS[7], '1', 'EX', ARGV[1])
return redis.call('ZADD', KEYS[8], ARGV[2], ARGV[3])
"#;
    let keys = document_state_keys(document_id);
    let gone_key = document_gone_key(document_id);
    let _: i64 = redis
        .eval(
            LUA,
            [
                keys[0].as_str(),
                keys[1].as_str(),
                keys[2].as_str(),
                keys[3].as_str(),
                keys[4].as_str(),
                FLUSH_DIRTY_SET,
                gone_key.as_str(),
                SWEEP_DUE_SET,
            ],
            [
                ttl_secs.to_string(),
                sweep_due.to_string(),
                document_id.to_string(),
            ],
        )
        .await
        .map_err(|e| AppError::Redis(format!("purge document state: {e}")))?;
    Ok(())
}

/// The documents whose second sweep is due at `now` (Unix seconds), at most
/// `limit` of them.
pub async fn due_sweeps(redis: &Client, now: u64, limit: usize) -> Result<Vec<String>, AppError> {
    const LUA: &str =
        "return redis.call('ZRANGEBYSCORE', KEYS[1], '-inf', ARGV[1], 'LIMIT', 0, ARGV[2])";
    redis
        .eval::<Vec<String>>(LUA, [SWEEP_DUE_SET], [now.to_string(), limit.to_string()])
        .await
        .map_err(|e| AppError::Redis(format!("due sweeps: {e}")))
}

/// Drops a document from the sweep schedule once its second sweep is done,
/// unless it was rescheduled past `now` meanwhile.
pub async fn sweep_done(redis: &Client, document_id: &str, now: u64) -> Result<(), AppError> {
    const LUA: &str = r#"
local due = redis.call('ZSCORE', KEYS[1], ARGV[1])
if due and tonumber(due) <= tonumber(ARGV[2]) then
    return redis.call('ZREM', KEYS[1], ARGV[1])
end
return 0
"#;
    let _: i64 = redis
        .eval::<i64>(
            LUA,
            [SWEEP_DUE_SET],
            [document_id.to_owned(), now.to_string()],
        )
        .await
        .map_err(|e| AppError::Redis(format!("sweep done: {e}")))?;
    Ok(())
}

/// The keys the relay holds for one document, in the order
/// [`purge_document_state`] passes them: seq, clocks, pending, presence,
/// dead-letter.
pub fn document_state_keys(document_id: &ambre_core::DocumentId) -> [String; 5] {
    [
        format!("doc:{document_id}:seq"),
        format!("doc:{document_id}:clocks"),
        format!("doc:{document_id}:pending"),
        format!("doc:{document_id}:presence"),
        format!("doc:{document_id}:deadletter"),
    ]
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
    /// `None` for an account that has not registered its keys (`SRV-027`).
    pub signing_pubkey: Option<ambre_core::PublicKey>,
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
    // Read into a `HashMap`, NOT a `Vec<String>`. Under RESP3 rustis returns a hash
    // reply as a map, and deserializing that into a flat `Vec<String>` fails with
    // "invalid type: sequence, expected a string". That surfaced here as a spurious
    // `SessionExpired` on EVERY OIDC callback (login fully broken). Mirror
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
    fn the_script_refusal_becomes_the_typed_conflict() {
        let err = clock_refusal("ERR Error running script: CLOCK_MISMATCH:4", 7);
        assert!(
            matches!(
                err,
                AppError::ClockMismatch {
                    expected: 5,
                    got: 7
                }
            ),
            "{err:?}"
        );
        let err = clock_refusal("connection reset by peer", 7);
        assert!(matches!(err, AppError::Redis(_)), "{err:?}");
    }

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
        assert!(
            matches!(
                err,
                AppError::ClockMismatch {
                    expected: 2,
                    got: 3
                }
            ),
            "{err:?}"
        );

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
        assert!(
            matches!(
                err,
                AppError::ClockMismatch {
                    expected: 2,
                    got: 1
                }
            ),
            "{err:?}"
        );

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
    async fn dirty_set_marks_on_append_and_clears_only_when_drained() {
        use rustis::commands::SetCommands;
        let redis = connect_redis().await;
        let doc = ambre_core::DocumentId::new();
        let pending_key = format!("doc:{doc}:pending");
        let _ = redis.del([pending_key.as_str()]).await;
        // Start from a clean marker for this doc's key.
        let _: Result<usize, _> = redis.srem(FLUSH_DIRTY_SET, [pending_key.as_str()]).await;

        // Appending a delta marks the pending list dirty.
        append_pending_delta(&redis, &doc, 1, "s3-x", &[1, 2, 3])
            .await
            .unwrap();
        let members = dirty_pending_keys(&redis).await.unwrap();
        assert!(
            members.contains(&pending_key),
            "append must mark the pending key dirty"
        );

        // A non-empty list must NOT be cleared (an acknowledged delta is pending).
        clear_pending_dirty_if_empty(&redis, &pending_key)
            .await
            .unwrap();
        let still: bool = redis
            .sismember(FLUSH_DIRTY_SET, pending_key.as_str())
            .await
            .unwrap();
        assert!(still, "a non-empty pending list must keep its dirty marker");

        // Once drained, the marker clears.
        let _ = redis.del([pending_key.as_str()]).await;
        clear_pending_dirty_if_empty(&redis, &pending_key)
            .await
            .unwrap();
        let gone: bool = redis
            .sismember(FLUSH_DIRTY_SET, pending_key.as_str())
            .await
            .unwrap();
        assert!(!gone, "a drained pending list must clear its dirty marker");

        let _ = redis.srem(FLUSH_DIRTY_SET, [pending_key.as_str()]).await;
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

    /// PRESENCE-014: a peer who stops watching is removed from the presence
    /// hash now, rather than left to the field's TTL.
    #[tokio::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs redis; run with --features db")]
    async fn presence_field_is_dropped_on_departure() {
        let redis = connect_redis().await;
        let doc = ambre_core::DocumentId::new();
        let key = format!("doc:{doc}:presence");
        let pk = "0badc0de0badc0de0badc0de0badc0de0badc0de0badc0de0badc0de0badc0de";
        let _ = redis.del([key.as_str()]).await;

        doc_presence_update(&redis, &doc, pk, "blob", 15)
            .await
            .unwrap();
        let present: bool = redis.hexists(&key, pk).await.unwrap();
        assert!(present, "the sample is in the hash");

        doc_presence_remove(&redis, &doc, pk).await.unwrap();
        let present: bool = redis.hexists(&key, pk).await.unwrap();
        assert!(!present, "the departure removed the field");

        // Removing a field nobody wrote is not an error: a connection that never
        // sent presence still announces its departure.
        doc_presence_remove(&redis, &doc, "never-wrote")
            .await
            .unwrap();

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

    // Reproduces the CLOCK_MISMATCH bug after a Redis reset. With a cold clock
    // state, reseeding it to the durable per-author high-water mark N must let the
    // author's next update at clock N+1 validate. It must not be rejected against a
    // phantom current=0.
    #[tokio::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs redis; run with --features db")]
    async fn seed_clock_state_unblocks_the_next_update_after_a_redis_reset() {
        let redis = connect_redis().await;
        let doc = ambre_core::DocumentId::new();
        let clocks_key = format!("doc:{doc}:clocks");
        let seq_key = format!("doc:{doc}:seq");
        let author = "deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef0";
        // Cold Redis (as after a restart): neither key exists.
        let _ = redis.del([clocks_key.as_str(), seq_key.as_str()]).await;
        assert!(clock_state_is_cold(&redis, &doc).await.unwrap());

        // The durable index says this author reached clock 5, global seq 5.
        seed_clock_state(&redis, &doc, 5, &[(author.to_string(), 5)])
            .await
            .expect("seed");

        // No longer cold, and the author's next update (clock 6) validates.
        assert!(!clock_state_is_cold(&redis, &doc).await.unwrap());
        let seq = verify_clock_and_assign_seq(&redis, &doc, author, 6)
            .await
            .expect("clock 6 should validate against a rehydrated current=5");
        assert_eq!(seq, 6, "seq counter must continue past the durable max");

        // A stale clock (already consumed) is still rejected.
        assert!(
            verify_clock_and_assign_seq(&redis, &doc, author, 6)
                .await
                .is_err()
        );

        let _ = redis.del([clocks_key.as_str(), seq_key.as_str()]).await;
    }

    async fn seed_refresh(redis: &Client) -> String {
        let token = uuid::Uuid::new_v4().to_string();
        let data = RefreshData {
            user_id: ambre_core::UserId::new(),
            signing_pubkey: Some(ambre_core::PublicKey::from_array([3u8; 32])),
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
