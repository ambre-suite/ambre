use crate::error::{AppError, Resource};
use rustis::client::Client;
use rustis::commands::{
    GenericCommands, HashCommands, ListCommands, PubSubCommands, ScriptingCommands, StringCommands,
};
use rustis::resp::RefBulkString;

use crate::oidc::{OidcAuthState, OidcSecret};
use crate::payload::Payload;
use crate::redis_keys::{
    ClocksKey, DeadLetterKey, DocumentStateKeys, FlushDirtySet, FlushLockKey, GoneKey, LockedKey,
    PendingKey, PresenceKey, RotationOwedKey, SeqKey, SessionsKey, SnapshotAskedKey,
};
use ambre_protocol::messages::Clock;
use ambre_protocol::ws::RefreshToken;

/// Atomic Lua script that validates a client clock and assigns a global
/// monotonic sequence number.
///
/// - `KEYS[1]` = `doc:{id}:clocks` (hash: pub_key_hex -> last_clock)
/// - `KEYS[2]` = `doc:{id}:seq`   (counter)
/// - `KEYS[3]` = `doc:{id}:gone`  (tombstone, see [`mark_document_gone`])
/// - `KEYS[4]` = `doc:{id}:rotation-owed` (see [`RotationOwedKey`])
/// - `KEYS[5]` = `doc:{id}:locked` (see [`LockedKey`])
/// - `ARGV[1]` = pub_key_hex
/// - `ARGV[2]` = incoming clock (must be last_clock + 1)
///
/// Rejects the update if the clock is not strictly sequential, if the
/// document is being deleted, or if it is a locked segment.
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
if redis.call('EXISTS', KEYS[4]) == 1 then
    return redis.error_reply('ROTATION_OWED')
end
if redis.call('EXISTS', KEYS[5]) == 1 then
    return redis.error_reply('SEGMENT_LOCKED')
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
///
/// A document that owes a rotation refuses with [`AppError::RotationOwed`]
/// before the clock is read. The frame then takes no sequence number and
/// consumes no clock (`SRV-123`, `SRV-125`). So does a locked segment, with
/// [`AppError::SegmentLocked`] (`SRV-115`).
pub async fn verify_clock_and_assign_seq(
    redis: &Client,
    document_id: &ambre_core::DocumentId,
    pub_key_hex: &ambre_core::SigningKeyHex,
    clock: Clock,
) -> Result<ambre_core::Seq, AppError> {
    let clocks_key = ClocksKey::of(document_id);
    let seq_key = SeqKey::of(document_id);
    let gone_key = GoneKey::of(document_id);
    let owed_key = RotationOwedKey::of(document_id);
    let locked_key = LockedKey::of(document_id);

    let result: i64 = redis
        .eval::<i64>(
            CLOCK_AND_SEQ_LUA,
            [
                clocks_key.as_str(),
                seq_key.as_str(),
                gone_key.as_str(),
                owed_key.as_str(),
                locked_key.as_str(),
            ],
            [pub_key_hex.as_str(), &clock.to_string()],
        )
        .await
        .map_err(|e| clock_refusal(&e.to_string(), clock))?;

    // The script answers `INCR`, which never falls below 1.
    Ok(ambre_core::Seq(u64::try_from(result).unwrap_or(0)))
}

/// Reads the script's `CLOCK_MISMATCH:<current>` refusal out of a store error.
/// The expected clock is the one after the current. A document being deleted
/// is not found; anything else stays a store failure.
fn clock_refusal(error: &str, got: Clock) -> AppError {
    if error.contains(DOCUMENT_GONE) {
        return AppError::NotFound {
            resource: Resource::Document,
        };
    }
    if error.contains(ROTATION_OWED) {
        return AppError::RotationOwed;
    }
    if error.contains(SEGMENT_LOCKED) {
        return AppError::SegmentLocked;
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
            expected: Clock(current.saturating_add(1)),
            got,
        },
        None => AppError::Redis(format!("clock validation failed: {error}")),
    }
}

/// The sequence counter of each document, in the order asked. `None` where
/// the ephemeral store holds no counter: the document was never written, or
/// the store was reset and nothing wrote since.
pub async fn document_seqs(
    redis: &Client,
    document_ids: &[ambre_core::DocumentId],
) -> Result<Vec<Option<ambre_core::Seq>>, AppError> {
    if document_ids.is_empty() {
        return Ok(Vec::new());
    }
    let keys: Vec<String> = document_ids
        .iter()
        .map(|id| SeqKey::of(id).as_str().to_owned())
        .collect();
    let counters = redis
        .mget::<Vec<Option<u64>>>(keys)
        .await
        .map_err(|e| AppError::Redis(format!("sequence counters: {e}")))?;
    Ok(counters
        .into_iter()
        .map(|c| c.map(ambre_core::Seq))
        .collect())
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
    let clocks_key = ClocksKey::of(document_id);
    let existing: usize = redis
        .exists(clocks_key.as_str())
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
    max_seq: ambre_core::Seq,
    author_clocks: &[(ambre_core::SigningKeyHex, Clock)],
) -> Result<(), AppError> {
    if author_clocks.is_empty() {
        return Ok(());
    }
    let clocks_key = ClocksKey::of(document_id);
    let seq_key = SeqKey::of(document_id);
    let gone_key = GoneKey::of(document_id);
    let mut argv = vec![max_seq.0.to_string()];
    for (hex, clock) in author_clocks {
        argv.push(hex.as_str().to_owned());
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
    seq: ambre_core::Seq,
    s3_key: &ambre_core::ObjectKey,
    update_wire: &Payload,
) -> Result<(), AppError> {
    let key = PendingKey::of(document_id);
    // Store the FULL sealed Update wire (ciphertext + nonce + public_data +
    // commitment + signature), not just the ciphertext. A reconnecting or
    // cross-device client replays these verbatim, and needs every field to
    // decrypt and verify. The flusher persists the same bytes to S3.
    let entry = Payload::pending_entry(seq, s3_key, update_wire);
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
    let gone_key = GoneKey::of(document_id);
    redis
        .eval::<i64>(
            LUA,
            [key.as_str(), FlushDirtySet.as_str(), gone_key.as_str()],
            [RefBulkString::new(entry.expose_bytes())],
        )
        .await
        .map_err(|e| store_refusal(&e.to_string(), "pending push failed"))?;
    Ok(())
}

/// The refusal every tombstone-guarded script answers for a document being
/// deleted (see [`mark_document_gone`]).
const DOCUMENT_GONE: &str = "DOCUMENT_GONE";

/// The refusal the clock and presence scripts answer for a document that owes
/// a rotation (see [`RotationOwedKey`]).
const ROTATION_OWED: &str = "ROTATION_OWED";

/// The refusal the clock script answers for a locked segment (see
/// [`LockedKey`]).
const SEGMENT_LOCKED: &str = "SEGMENT_LOCKED";

/// Reads a guarded script's refusal out of a store error: a document being
/// deleted is not found, anything else is a store failure.
fn store_refusal(error: &str, what: &str) -> AppError {
    if error.contains(DOCUMENT_GONE) {
        AppError::NotFound {
            resource: Resource::Document,
        }
    } else {
        AppError::Redis(format!("{what}: {error}"))
    }
}

/// RPUSHes `entry` onto a document's dead-letter list unless its tombstone
/// exists. The flusher and the write path use it, so neither recreates a
/// purged list.
async fn guarded_rpush(
    redis: &Client,
    list_key: &DeadLetterKey,
    gone_key: &GoneKey,
    entry: &Payload,
) -> Result<(), AppError> {
    const LUA: &str = r#"
if redis.call('EXISTS', KEYS[2]) == 1 then
    return redis.error_reply('DOCUMENT_GONE')
end
return redis.call('RPUSH', KEYS[1], ARGV[1])
"#;
    let _: i64 = redis
        .eval::<i64>(
            LUA,
            [list_key.as_str(), gone_key.as_str()],
            [RefBulkString::new(entry.expose_bytes())],
        )
        .await
        .map_err(|e| store_refusal(&e.to_string(), "guarded push failed"))?;
    Ok(())
}

/// Dead-letters a poison pending entry the flusher found, unless the document
/// is being deleted.
pub async fn push_poison_entry(
    redis: &Client,
    document_id: &ambre_core::DocumentId,
    entry: &Payload,
) -> Result<(), AppError> {
    guarded_rpush(
        redis,
        &DeadLetterKey::of(document_id),
        &GoneKey::of(document_id),
        entry,
    )
    .await
}

/// The entries of a pending list, front first.
pub async fn pending_entries(
    redis: &Client,
    pending_key: &PendingKey,
) -> Result<Vec<Payload>, AppError> {
    let entries: Vec<String> = redis
        .lrange(pending_key.as_str(), 0, -1)
        .await
        .map_err(|e| AppError::Redis(format!("pending lrange failed: {e}")))?;
    Ok(entries
        .into_iter()
        .map(|entry| Payload::new(entry.into_bytes()))
        .collect())
}

/// Records `pending_key` in the dirty set so the flusher will visit it.
pub async fn mark_pending_dirty(redis: &Client, pending_key: &PendingKey) -> Result<(), AppError> {
    use rustis::commands::SetCommands;
    redis
        .sadd(FlushDirtySet.as_str(), [pending_key.as_str()])
        .await
        .map_err(|e| AppError::Redis(format!("mark pending dirty: {e}")))?;
    Ok(())
}

/// Returns the pending keys currently marked dirty (the flusher's work list).
/// A member this relay never writes names no list to flush, and is skipped.
pub async fn dirty_pending_keys(redis: &Client) -> Result<Vec<PendingKey>, AppError> {
    use rustis::commands::SetCommands;
    let members: Vec<String> = redis
        .smembers(FlushDirtySet.as_str())
        .await
        .map_err(|e| AppError::Redis(format!("dirty pending keys: {e}")))?;
    Ok(members
        .iter()
        .filter_map(|member| PendingKey::parse(member))
        .collect())
}

/// Atomically clears `pending_key` from the dirty set, but ONLY if its pending
/// list is now empty. Run after a drain. An entry appended concurrently (RPUSH
/// before its SADD, see `append_pending_delta`) is already visible in the list
/// here. So `LLEN > 0` keeps the marker, and the delta is retried next cycle:
/// an acknowledged delta is never stranded.
pub async fn clear_pending_dirty_if_empty(
    redis: &Client,
    pending_key: &PendingKey,
) -> Result<(), AppError> {
    const LUA: &str = "if redis.call('LLEN', KEYS[1]) == 0 then return redis.call('SREM', KEYS[2], ARGV[1]) else return 0 end";
    let _: i64 = redis
        .eval::<i64>(
            LUA,
            [pending_key.as_str(), FlushDirtySet.as_str()],
            [pending_key.as_str()],
        )
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
    seq: ambre_core::Seq,
    s3_key: &ambre_core::ObjectKey,
    update_wire: &Payload,
) -> Result<(), AppError> {
    guarded_rpush(
        redis,
        &DeadLetterKey::of(document_id),
        &GoneKey::of(document_id),
        &Payload::pending_entry(seq, s3_key, update_wire),
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
) -> Result<std::collections::HashMap<ambre_core::Seq, Payload>, AppError> {
    let entries = pending_entries(redis, &PendingKey::of(document_id)).await?;
    let mut map = std::collections::HashMap::with_capacity(entries.len());
    for entry in &entries {
        // An entry that names no number cannot answer a replay.
        if let (Some(seq), Some((_, _, wire))) = (entry.pending_seq(), entry.read_pending()) {
            map.insert(seq, wire);
        }
    }
    Ok(map)
}

/// The value a lock holder writes into its lock key. A release compares it, so a
/// holder drops only the lock it took.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LockToken(String);

impl LockToken {
    /// A token no other holder shares: a random UUID.
    pub fn fresh() -> Self {
        Self(uuid::Uuid::new_v4().to_string())
    }

    /// The token text, as the lock key stores it.
    pub fn as_str(&self) -> &str {
        &self.0
    }
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
    lock_key: &FlushLockKey,
    token: &LockToken,
    ttl: ambre_core::Seconds,
) -> Result<bool, AppError> {
    let ttl = ttl.0.to_string();
    let res: Option<String> = redis
        .eval::<Option<String>>(
            ACQUIRE_LUA,
            [lock_key.as_str()],
            [token.as_str(), ttl.as_str()],
        )
        .await
        .map_err(|e| AppError::Redis(format!("flush lock acquire: {e}")))?;
    Ok(res.is_some())
}

/// `SET key token NX EX ttl`: the key is set only when it stands unset.
const ACQUIRE_LUA: &str = "return redis.call('SET', KEYS[1], ARGV[1], 'NX', 'EX', ARGV[2])";

/// Releases the per-document flush lock, but only if `token` still matches
/// (check-and-del). An expired-then-reacquired lock owned by another instance
/// is never dropped by a slow predecessor.
pub async fn release_flush_lock(
    redis: &Client,
    lock_key: &FlushLockKey,
    token: &LockToken,
) -> Result<(), AppError> {
    const LUA: &str = "if redis.call('GET', KEYS[1]) == ARGV[1] then return redis.call('DEL', KEYS[1]) else return 0 end";
    let _: i64 = redis
        .eval::<i64>(LUA, [lock_key.as_str()], [token.as_str()])
        .await
        .map_err(|e| AppError::Redis(format!("flush lock release: {e}")))?;
    Ok(())
}

/// Claims the one standing snapshot request of a document for `ttl`
/// (`SRV-038`). Returns `true` when no request stood, so this caller asks. Every
/// instance shares the claim, so one threshold crossing asks once.
pub async fn claim_snapshot_ask(
    redis: &Client,
    document_id: &ambre_core::DocumentId,
    ttl: ambre_core::Seconds,
) -> Result<bool, AppError> {
    let key = SnapshotAskedKey::of(document_id);
    let ttl = ttl.0.to_string();
    let res: Option<String> = redis
        .eval::<Option<String>>(ACQUIRE_LUA, [key.as_str()], ["1", ttl.as_str()])
        .await
        .map_err(|e| AppError::Redis(format!("snapshot ask claim: {e}")))?;
    Ok(res.is_some())
}

/// The last delta number the relay gave the document, 0 when its counter is
/// cold.
pub async fn current_seq(
    redis: &Client,
    document_id: &ambre_core::DocumentId,
) -> Result<ambre_core::Seq, AppError> {
    let seq: Option<u64> = redis
        .get(SeqKey::of(document_id).as_str())
        .await
        .map_err(|e| AppError::Redis(format!("current seq: {e}")))?;
    Ok(ambre_core::Seq(seq.unwrap_or(0)))
}

/// Retrieves a refresh-token record from Redis.
///
/// Refresh keys follow the pattern `refresh:{token}` with fields `user_id` and
/// `signing_pubkey_hex`. Returns `None` when the token is unknown, expired, or
/// was revoked (logout / rotation).
pub async fn refresh_get(
    redis: &Client,
    token: &RefreshToken,
) -> Result<Option<RefreshData>, AppError> {
    let key = format!("refresh:{}", token.expose());
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
    token: &RefreshToken,
    data: &RefreshData,
    ttl: ambre_core::Seconds,
) -> Result<(), AppError> {
    let ttl_secs = ttl.0;
    let key = format!("refresh:{}", token.expose());

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
pub async fn refresh_delete(redis: &Client, token: &RefreshToken) -> Result<(), AppError> {
    let key = format!("refresh:{}", token.expose());
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
pub async fn refresh_claim(redis: &Client, token: &RefreshToken) -> Result<bool, AppError> {
    let key = format!("refresh:{}", token.expose());
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
pub async fn refresh_consume(
    redis: &Client,
    token: &RefreshToken,
) -> Result<Option<RefreshData>, AppError> {
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
    payload: &Payload,
) -> Result<(), AppError> {
    let channel = crate::ws::Topic::document(document_id);
    // The payload is binary (encrypted ciphertext). rustis serializes a raw
    // `&[u8]` as a sequence of integers (a serde limitation) and panics. Wrap it
    // in `RefBulkString` so it is sent as a single RESP bulk string.
    redis
        .publish(channel.as_str(), RefBulkString::new(payload.expose_bytes()))
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
    payload: &Payload,
) -> Result<(), AppError> {
    let channel = crate::ws::Topic::user(user_id);
    redis
        .publish(channel.as_str(), RefBulkString::new(payload.expose_bytes()))
        .await
        .map_err(|e| AppError::Redis(format!("user broadcast: {e}")))?;
    Ok(())
}

/// Tells every connection subscribed to a document that its members changed
/// (`SRV-177`). The message names the document alone, which the topic already
/// names. Best-effort, like every broadcast: the member rows stay the source of
/// truth, and a client reads them again at its next open.
pub async fn members_changed(redis: &Client, document_id: &ambre_core::DocumentId) {
    let msg =
        crate::payload::Payload::server_msg(&ambre_protocol::ws::WsServerMsg::MembersChanged {
            document_id: *document_id,
        });
    let _ = doc_broadcast(redis, document_id, &msg).await;
}

/// Tells every socket watching a document of `channel_id` that its thread
/// `thread` took a delta (`SRV-161`). The signal names the thread alone, which
/// the relay already routes on (`SRV-162`). Best-effort: a client that missed
/// it counts the delta at the next one, or reads it when it opens the thread.
pub async fn thread_activity(
    redis: &Client,
    channel_id: &ambre_core::ChannelId,
    thread: &ambre_core::DocumentId,
) -> Result<(), AppError> {
    let msg = Payload::server_msg(&ambre_protocol::ws::WsServerMsg::ThreadActivity {
        document_id: *thread,
    });
    let topic = crate::ws::Topic::channel(channel_id);
    redis
        .publish(topic.as_str(), RefBulkString::new(msg.expose_bytes()))
        .await
        .map_err(|e| AppError::Redis(format!("thread activity: {e}")))?;
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
    pub_key_hex: &ambre_core::SigningKeyHex,
    presence: &ambre_protocol::messages::Ciphertext,
    ttl: ambre_core::Seconds,
) -> Result<(), AppError> {
    use base64::Engine;
    let presence_b64 = base64::engine::general_purpose::URL_SAFE.encode(&presence.0);
    // Expire THIS member's field individually (Redis 7.4+ / Valkey 8 `HEXPIRE`),
    // so a member who leaves without a clean unsubscribe self-prunes from the
    // hash. Without a per-field TTL the presence hash grew one dead field per
    // distinct author forever. The tombstone check, the write and its TTL run
    // as one script, so a field never lands without its expiry. A document that
    // owes a rotation refuses presence as it refuses a delta (`SRV-123`).
    const LUA: &str = r#"
if redis.call('EXISTS', KEYS[2]) == 1 then
    return redis.error_reply('DOCUMENT_GONE')
end
if redis.call('EXISTS', KEYS[3]) == 1 then
    return redis.error_reply('ROTATION_OWED')
end
redis.call('HSET', KEYS[1], ARGV[2], ARGV[3])
return redis.call('HEXPIRE', KEYS[1], ARGV[1], 'FIELDS', 1, ARGV[2])
"#;
    let key = PresenceKey::of(document_id);
    let gone_key = GoneKey::of(document_id);
    let owed_key = RotationOwedKey::of(document_id);
    let ttl = ttl.0.to_string();
    redis
        .eval::<Vec<i64>>(
            LUA,
            [key.as_str(), gone_key.as_str(), owed_key.as_str()],
            [ttl.as_str(), pub_key_hex.as_str(), presence_b64.as_str()],
        )
        .await
        .map_err(|e| {
            let e = e.to_string();
            if e.contains(ROTATION_OWED) {
                AppError::RotationOwed
            } else {
                store_refusal(&e, "presence update")
            }
        })?;
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
    pub_key_hex: &ambre_core::SigningKeyHex,
) -> Result<(), AppError> {
    let key = PresenceKey::of(document_id);
    redis
        .hdel(key.as_str(), pub_key_hex.as_str())
        .await
        .map_err(|e| AppError::Redis(format!("presence remove: {e}")))?;
    Ok(())
}

/// How long a session count lives after its last open. One user signs every
/// session with the same key, so the count is what tells their sessions apart.
/// An instance that dies closes none of its sessions; the expiry bounds the
/// count that death leaves behind (`PRESENCE-015`).
const SESSION_COUNT_TTL: ambre_core::Seconds = ambre_core::Seconds(24 * 3600);

/// Counts one more session of `pub_key_hex` watching the document, on every
/// instance (`1-Foundations/ambre-spec-presence.md`, `PRESENCE-015`). A
/// document being deleted takes no count (`SRV-017`).
pub async fn doc_session_open(
    redis: &Client,
    document_id: &ambre_core::DocumentId,
    pub_key_hex: &ambre_core::SigningKeyHex,
) -> Result<(), AppError> {
    const LUA: &str = r#"
if redis.call('EXISTS', KEYS[2]) == 1 then
    return 0
end
redis.call('HINCRBY', KEYS[1], ARGV[2], 1)
redis.call('HEXPIRE', KEYS[1], ARGV[1], 'FIELDS', 1, ARGV[2])
return 1
"#;
    let key = SessionsKey::of(document_id);
    let gone_key = GoneKey::of(document_id);
    let ttl = SESSION_COUNT_TTL.0.to_string();
    let _: i64 = redis
        .eval(
            LUA,
            [key.as_str(), gone_key.as_str()],
            [ttl.as_str(), pub_key_hex.as_str()],
        )
        .await
        .map_err(|e| AppError::Redis(format!("session open: {e}")))?;
    Ok(())
}

/// Counts one session of `pub_key_hex` gone from the document. Answers whether
/// it was the last, which is when the author has left (`PRESENCE-015`).
///
/// A count that is missing or already spent answers `true`: a departure the
/// relay cannot count against an open session is a departure.
pub async fn doc_session_close(
    redis: &Client,
    document_id: &ambre_core::DocumentId,
    pub_key_hex: &ambre_core::SigningKeyHex,
) -> Result<bool, AppError> {
    const LUA: &str = r#"
local left = redis.call('HINCRBY', KEYS[1], ARGV[1], -1)
if left <= 0 then
    redis.call('HDEL', KEYS[1], ARGV[1])
    return 1
end
return 0
"#;
    let key = SessionsKey::of(document_id);
    let last: i64 = redis
        .eval(LUA, [key.as_str()], [pub_key_hex.as_str()])
        .await
        .map_err(|e| AppError::Redis(format!("session close: {e}")))?;
    Ok(last == 1)
}

/// Sets the tombstone of a document for `ttl`: the first step of a
/// delete, before any object or key is removed.
pub async fn mark_document_gone(
    redis: &Client,
    document_id: &ambre_core::DocumentId,
    ttl: ambre_core::Seconds,
) -> Result<(), AppError> {
    const LUA: &str = "return redis.call('SET', KEYS[1], '1', 'EX', ARGV[1])";
    let gone_key = GoneKey::of(document_id);
    let ttl = ttl.0.to_string();
    let _: String = redis
        .eval::<String>(LUA, [gone_key.as_str()], [ttl.as_str()])
        .await
        .map_err(|e| AppError::Redis(format!("mark document gone: {e}")))?;
    Ok(())
}

/// Removes every key the relay holds for a deleted document (`SRV-017`). That
/// is the sequence counter, the per-author clock hash, the pending list and
/// its dirty marker. It is also the presence hash, the dead-letter list, the
/// mirror of an owed rotation and the session counts. One script, so no reader sees a half-purged
/// document.
///
/// The same script renews the tombstone for `ttl`. The second sweep of
/// the objects is scheduled in the register, which a reset of this store does
/// not lose (`SRV-057`, `SRV-165`).
pub async fn purge_document_state(
    redis: &Client,
    document_id: &ambre_core::DocumentId,
    ttl: ambre_core::Seconds,
) -> Result<(), AppError> {
    const LUA: &str = r#"
redis.call('SREM', KEYS[9], KEYS[3])
redis.call('DEL', KEYS[1], KEYS[2], KEYS[3], KEYS[4], KEYS[5], KEYS[6], KEYS[7], KEYS[8])
redis.call('SET', KEYS[10], '1', 'EX', ARGV[1])
return 1
"#;
    let keys = DocumentStateKeys::of(document_id);
    let gone_key = GoneKey::of(document_id);
    let [
        seq,
        clocks,
        pending,
        presence,
        dead_letter,
        owed,
        sessions,
        locked,
    ] = keys.texts();
    let _: i64 = redis
        .eval(
            LUA,
            [
                seq,
                clocks,
                pending,
                presence,
                dead_letter,
                owed,
                sessions,
                locked,
                FlushDirtySet.as_str(),
                gone_key.as_str(),
            ],
            [ttl.0.to_string()],
        )
        .await
        .map_err(|e| AppError::Redis(format!("purge document state: {e}")))?;
    Ok(())
}

/// Lock a segment against deltas, unless one still waits to be flushed.
/// Returns the last delta number the relay gave it, the horizon of its final
/// snapshot, or `None` while a delta is still pending. One script, so no
/// delta is numbered between the check and the lock.
pub async fn lock_segment(
    redis: &Client,
    document_id: &ambre_core::DocumentId,
) -> Result<Option<ambre_core::Seq>, AppError> {
    const LUA: &str = r#"
if redis.call('LLEN', KEYS[1]) > 0 then
    return -1
end
redis.call('SET', KEYS[2], '1')
return tonumber(redis.call('GET', KEYS[3])) or 0
"#;
    let pending = PendingKey::of(document_id);
    let locked = LockedKey::of(document_id);
    let seq = SeqKey::of(document_id);
    let answer: i64 = redis
        .eval::<i64>(
            LUA,
            [pending.as_str(), locked.as_str(), seq.as_str()],
            Vec::<String>::new(),
        )
        .await
        .map_err(|e| AppError::Redis(format!("lock segment: {e}")))?;
    Ok(ambre_core::Seq::try_from(answer).ok())
}

/// Lock a segment against deltas at once, a delta still unflushed included,
/// and answer the last delta number it gave. A rotation cannot wait for a
/// flush, and a delta numbered before it stays under the lock (`SRV-147`).
pub async fn lock_segment_now(
    redis: &Client,
    document_id: &ambre_core::DocumentId,
) -> Result<ambre_core::Seq, AppError> {
    const LUA: &str = r#"
redis.call('SET', KEYS[1], '1')
return tonumber(redis.call('GET', KEYS[2])) or 0
"#;
    let locked = LockedKey::of(document_id);
    let seq = SeqKey::of(document_id);
    let answer: i64 = redis
        .eval::<i64>(LUA, [locked.as_str(), seq.as_str()], Vec::<String>::new())
        .await
        .map_err(|e| AppError::Redis(format!("lock segment now: {e}")))?;
    Ok(ambre_core::Seq(answer.unsigned_abs()))
}

/// Seal an amendments document a fold holds up to its delta `folded_seq`
/// (`SRV-211`). The seal is the segment lock, so the clock script refuses
/// every later delta. Answers `false`, and seals nothing, while a delta waits
/// to be flushed or when the document took a delta past the fold. One script,
/// so no delta is numbered between the check and the seal.
pub async fn seal_folded(
    redis: &Client,
    document_id: &ambre_core::DocumentId,
    folded_seq: ambre_core::Seq,
) -> Result<bool, AppError> {
    const LUA: &str = r#"
if redis.call('LLEN', KEYS[1]) > 0 then
    return 0
end
if (tonumber(redis.call('GET', KEYS[3])) or 0) ~= tonumber(ARGV[1]) then
    return 0
end
redis.call('SET', KEYS[2], '1')
return 1
"#;
    let pending = PendingKey::of(document_id);
    let locked = LockedKey::of(document_id);
    let seq = SeqKey::of(document_id);
    let sealed: i64 = redis
        .eval::<i64>(
            LUA,
            [pending.as_str(), locked.as_str(), seq.as_str()],
            [folded_seq.0.to_string()],
        )
        .await
        .map_err(|e| AppError::Redis(format!("seal folded: {e}")))?;
    Ok(sealed == 1)
}

/// Sets the mirror of a segment's lock. Nothing clears it but the purge of a
/// deleted document: a lock holds for good.
pub async fn mirror_segment_lock(
    redis: &Client,
    document_id: &ambre_core::DocumentId,
) -> Result<(), AppError> {
    redis
        .set(LockedKey::of(document_id).as_str(), "1")
        .await
        .map_err(|e| AppError::Redis(format!("mirror segment lock: {e}")))
}

/// Whether a document owes a key rotation, as its mirror holds it
/// (`SRV-123`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RotationDebt {
    Settled,
    Owed,
}

impl RotationDebt {
    /// [`RotationDebt::Owed`] when the register says `owed`.
    pub fn owed_if(owed: bool) -> Self {
        if owed {
            RotationDebt::Owed
        } else {
            RotationDebt::Settled
        }
    }
}

/// Sets or clears the mirror of an owed rotation.
pub async fn mirror_rotation_owed(
    redis: &Client,
    document_id: &ambre_core::DocumentId,
    debt: RotationDebt,
) -> Result<(), AppError> {
    const LUA: &str = r#"
if ARGV[1] == '1' then
    redis.call('SET', KEYS[1], '1')
    return 1
end
return redis.call('DEL', KEYS[1])
"#;
    let key = RotationOwedKey::of(document_id);
    let flag = match debt {
        RotationDebt::Owed => "1",
        RotationDebt::Settled => "0",
    };
    let _: i64 = redis
        .eval::<i64>(LUA, [key.as_str()], [flag])
        .await
        .map_err(|e| AppError::Redis(format!("mirror rotation owed: {e}")))?;
    Ok(())
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
    csrf_state: &ambre_core::OidcState,
    auth_state: &OidcAuthState,
) -> Result<(), AppError> {
    let key = format!("oidc:state:{}", csrf_state.expose());
    redis
        .hset(
            &key,
            [
                ("pkce_verifier", auth_state.pkce_verifier_secret.expose()),
                ("nonce", auth_state.nonce.secret().as_str()),
            ],
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
/// Returns the PKCE verifier and the nonce on success, or an error if the
/// state expired or was not found.
pub async fn oidc_state_take(
    redis: &Client,
    csrf_state: &ambre_core::OidcState,
) -> Result<OidcAuthState, AppError> {
    let key = format!("oidc:state:{}", csrf_state.expose());
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

    Ok(OidcAuthState {
        pkce_verifier_secret: OidcSecret::new(pkce_verifier),
        nonce: openidconnect::Nonce::new(nonce),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A presence sample: its bytes stand for an encrypted cursor.
    pub(super) fn sample() -> ambre_protocol::messages::Ciphertext {
        ambre_protocol::messages::Ciphertext(b"blob".to_vec())
    }

    #[test]
    fn the_script_refusal_becomes_the_typed_conflict() {
        let err = clock_refusal("ERR Error running script: CLOCK_MISMATCH:4", Clock(7));
        assert!(
            matches!(
                err,
                AppError::ClockMismatch {
                    expected: Clock(5),
                    got: Clock(7)
                }
            ),
            "{err:?}"
        );
        let err = clock_refusal("connection reset by peer", Clock(7));
        assert!(matches!(err, AppError::Redis(_)), "{err:?}");
    }
}

#[cfg(test)]
#[cfg(feature = "test-helpers")]
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

    /// `SRV-123`, `SRV-125`: while the debt is mirrored, a delta and a presence
    /// sample are refused. The delta takes no sequence number and consumes no
    /// clock. Once the debt clears, the same clock is accepted at seq 1.
    #[tokio::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs redis; run with --features db")]
    async fn a_frame_owed_a_rotation_takes_no_seq_and_no_clock() {
        let redis = connect_redis().await;
        let doc_id = ambre_core::DocumentId::new();
        let pk_hex = &ambre_core::SigningKeyHex(
            "0aa10aa10aa10aa10aa10aa10aa10aa10aa10aa10aa10aa10aa10aa10aa10aa1".into(),
        );

        mirror_rotation_owed(&redis, &doc_id, RotationDebt::Owed)
            .await
            .unwrap();
        let refused = verify_clock_and_assign_seq(&redis, &doc_id, pk_hex, Clock(1)).await;
        assert!(
            matches!(refused, Err(AppError::RotationOwed)),
            "{refused:?}"
        );
        let presence = doc_presence_update(
            &redis,
            &doc_id,
            pk_hex,
            &super::tests::sample(),
            ambre_core::Seconds(15),
        )
        .await;
        assert!(
            matches!(presence, Err(AppError::RotationOwed)),
            "{presence:?}"
        );
        let seq: usize = redis.exists(format!("doc:{doc_id}:seq")).await.unwrap();
        assert_eq!(seq, 0, "a refused frame takes no sequence number");

        mirror_rotation_owed(&redis, &doc_id, RotationDebt::Settled)
            .await
            .unwrap();
        let seq = verify_clock_and_assign_seq(&redis, &doc_id, pk_hex, Clock(1))
            .await
            .expect("the same clock is accepted once the debt clears");
        assert_eq!(seq, ambre_core::Seq(1));

        let _ = redis
            .del(DocumentStateKeys::of(&doc_id).texts())
            .await
            .unwrap();
    }

    /// `SRV-114`, `SRV-115`: a lock waits for the pending deltas, then answers
    /// the last delta number. A later delta is refused with the lock's code,
    /// and takes no sequence number and consumes no clock.
    #[tokio::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs redis; run with --features db")]
    async fn a_locked_segment_refuses_every_later_delta() {
        let redis = connect_redis().await;
        let doc_id = ambre_core::DocumentId::new();
        let pk_hex = &ambre_core::SigningKeyHex(
            "0bb10bb10bb10bb10bb10bb10bb10bb10bb10bb10bb10bb10bb10bb10bb10bb1".into(),
        );
        let pending = format!("doc:{doc_id}:pending");

        for clock in 1..=2 {
            verify_clock_and_assign_seq(&redis, &doc_id, pk_hex, Clock(clock))
                .await
                .expect("an open segment takes deltas");
        }
        let _: usize = redis.rpush(&pending, "entry").await.unwrap();
        assert_eq!(lock_segment(&redis, &doc_id).await.unwrap(), None);
        let locked: usize = redis.exists(LockedKey::of(&doc_id).as_str()).await.unwrap();
        assert_eq!(locked, 0, "a pending delta holds the lock back");

        let _: usize = redis.del(&pending).await.unwrap();
        assert_eq!(
            lock_segment(&redis, &doc_id).await.unwrap(),
            Some(ambre_core::Seq(2))
        );
        let refused = verify_clock_and_assign_seq(&redis, &doc_id, pk_hex, Clock(3)).await;
        assert!(
            matches!(refused, Err(AppError::SegmentLocked)),
            "{refused:?}"
        );
        let seq: Option<u64> = redis.get(format!("doc:{doc_id}:seq")).await.unwrap();
        assert_eq!(seq, Some(2), "a refused delta takes no sequence number");
        let clock: Option<u64> = redis
            .hget(ClocksKey::of(&doc_id).as_str(), pk_hex.as_str())
            .await
            .unwrap();
        assert_eq!(clock, Some(2), "a refused delta consumes no clock");

        let _ = redis
            .del(DocumentStateKeys::of(&doc_id).texts())
            .await
            .unwrap();
    }

    /// `SRV-147`: a rotation's lock takes no wait for a pending delta. It
    /// answers the last delta number, and a later delta is refused.
    #[tokio::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs redis; run with --features db")]
    async fn a_rotation_locks_a_segment_with_a_delta_still_pending() {
        let redis = connect_redis().await;
        let doc_id = ambre_core::DocumentId::new();
        let pk_hex = &ambre_core::SigningKeyHex(
            "0dd10dd10dd10dd10dd10dd10dd10dd10dd10dd10dd10dd10dd10dd10dd10dd1".into(),
        );
        assert_eq!(
            lock_segment_now(&redis, &doc_id).await.unwrap(),
            ambre_core::Seq(0)
        );
        let _ = redis
            .del(DocumentStateKeys::of(&doc_id).texts())
            .await
            .unwrap();

        for clock in 1..=2 {
            verify_clock_and_assign_seq(&redis, &doc_id, pk_hex, Clock(clock))
                .await
                .expect("an open segment takes deltas");
        }
        let _: usize = redis
            .rpush(format!("doc:{doc_id}:pending"), "entry")
            .await
            .unwrap();
        assert_eq!(
            lock_segment_now(&redis, &doc_id).await.unwrap(),
            ambre_core::Seq(2)
        );
        let refused = verify_clock_and_assign_seq(&redis, &doc_id, pk_hex, Clock(3)).await;
        assert!(
            matches!(refused, Err(AppError::SegmentLocked)),
            "{refused:?}"
        );

        let _ = redis
            .del(DocumentStateKeys::of(&doc_id).texts())
            .await
            .unwrap();
    }

    /// `SRV-211`: the seal of folded amendments waits for the pending deltas,
    /// and holds back when a delta came past the fold. Once sealed, a later
    /// delta is refused.
    #[tokio::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs redis; run with --features db")]
    async fn a_fold_seals_only_the_amendments_it_holds() {
        let redis = connect_redis().await;
        let doc_id = ambre_core::DocumentId::new();
        let pk_hex = &ambre_core::SigningKeyHex(
            "0cc10cc10cc10cc10cc10cc10cc10cc10cc10cc10cc10cc10cc10cc10cc10cc1".into(),
        );
        let pending = format!("doc:{doc_id}:pending");

        for clock in 1..=2 {
            verify_clock_and_assign_seq(&redis, &doc_id, pk_hex, Clock(clock))
                .await
                .expect("amendments take deltas");
        }
        assert!(
            !seal_folded(&redis, &doc_id, ambre_core::Seq(1))
                .await
                .unwrap(),
            "a delta came past the fold"
        );
        let _: usize = redis.rpush(&pending, "entry").await.unwrap();
        assert!(
            !seal_folded(&redis, &doc_id, ambre_core::Seq(2))
                .await
                .unwrap(),
            "a delta waits to be flushed"
        );
        let locked: usize = redis.exists(LockedKey::of(&doc_id).as_str()).await.unwrap();
        assert_eq!(locked, 0, "a refused seal locks nothing");

        let _: usize = redis.del(&pending).await.unwrap();
        assert!(
            seal_folded(&redis, &doc_id, ambre_core::Seq(2))
                .await
                .unwrap()
        );
        let refused = verify_clock_and_assign_seq(&redis, &doc_id, pk_hex, Clock(3)).await;
        assert!(
            matches!(refused, Err(AppError::SegmentLocked)),
            "{refused:?}"
        );

        let _ = redis
            .del(DocumentStateKeys::of(&doc_id).texts())
            .await
            .unwrap();
    }

    #[tokio::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs redis; run with --features db")]
    async fn first_update_succeeds() {
        let redis = connect_redis().await;
        let doc_id = ambre_core::DocumentId::new();
        let pk_hex = &ambre_core::SigningKeyHex(
            "abc123abc123abc123abc123abc123abc123abc123abc123abc123abc123abc1".into(),
        );

        let seq = verify_clock_and_assign_seq(&redis, &doc_id, pk_hex, Clock(1))
            .await
            .expect("first update should succeed");
        assert_eq!(seq, ambre_core::Seq(1));

        let _ = redis
            .del([
                ClocksKey::of(&doc_id).as_str(),
                SeqKey::of(&doc_id).as_str(),
            ])
            .await
            .unwrap();
    }

    #[tokio::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs redis; run with --features db")]
    async fn sequential_clock_succeeds() {
        let redis = connect_redis().await;
        let doc_id = ambre_core::DocumentId::new();
        let pk = &ambre_core::SigningKeyHex(
            "def456def456def456def456def456def456def456def456def456def456def4".into(),
        );

        let s1 = verify_clock_and_assign_seq(&redis, &doc_id, pk, Clock(1))
            .await
            .unwrap();
        let s2 = verify_clock_and_assign_seq(&redis, &doc_id, pk, Clock(2))
            .await
            .unwrap();
        let s3 = verify_clock_and_assign_seq(&redis, &doc_id, pk, Clock(3))
            .await
            .unwrap();
        assert_eq!(s1, ambre_core::Seq(1));
        assert_eq!(s2, ambre_core::Seq(2));
        assert_eq!(s3, ambre_core::Seq(3));

        let _ = redis
            .del([
                ClocksKey::of(&doc_id).as_str(),
                SeqKey::of(&doc_id).as_str(),
            ])
            .await
            .unwrap();
    }

    #[tokio::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs redis; run with --features db")]
    async fn clock_mismatch_rejected() {
        let redis = connect_redis().await;
        let doc_id = ambre_core::DocumentId::new();
        let pk = &ambre_core::SigningKeyHex(
            "1111111111111111111111111111111111111111111111111111111111111111".into(),
        );

        verify_clock_and_assign_seq(&redis, &doc_id, pk, Clock(1))
            .await
            .unwrap();
        let err = verify_clock_and_assign_seq(&redis, &doc_id, pk, Clock(3))
            .await
            .unwrap_err();
        assert!(
            matches!(
                err,
                AppError::ClockMismatch {
                    expected: Clock(2),
                    got: Clock(3)
                }
            ),
            "{err:?}"
        );

        let _ = redis
            .del([
                ClocksKey::of(&doc_id).as_str(),
                SeqKey::of(&doc_id).as_str(),
            ])
            .await
            .unwrap();
    }

    #[tokio::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs redis; run with --features db")]
    async fn duplicate_clock_rejected() {
        let redis = connect_redis().await;
        let doc_id = ambre_core::DocumentId::new();
        let pk = &ambre_core::SigningKeyHex(
            "2222222222222222222222222222222222222222222222222222222222222222".into(),
        );

        verify_clock_and_assign_seq(&redis, &doc_id, pk, Clock(1))
            .await
            .unwrap();
        let err = verify_clock_and_assign_seq(&redis, &doc_id, pk, Clock(1))
            .await
            .unwrap_err();
        assert!(
            matches!(
                err,
                AppError::ClockMismatch {
                    expected: Clock(2),
                    got: Clock(1)
                }
            ),
            "{err:?}"
        );

        let _ = redis
            .del([
                ClocksKey::of(&doc_id).as_str(),
                SeqKey::of(&doc_id).as_str(),
            ])
            .await
            .unwrap();
    }

    #[tokio::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs redis; run with --features db")]
    async fn independent_clients_have_independent_clocks() {
        let redis = connect_redis().await;
        let doc_id = ambre_core::DocumentId::new();
        let pk_a = &ambre_core::SigningKeyHex(
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
        );
        let pk_b = &ambre_core::SigningKeyHex(
            "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
        );

        let s_a = verify_clock_and_assign_seq(&redis, &doc_id, pk_a, Clock(1))
            .await
            .unwrap();
        let s_b = verify_clock_and_assign_seq(&redis, &doc_id, pk_b, Clock(1))
            .await
            .unwrap();
        let s_a2 = verify_clock_and_assign_seq(&redis, &doc_id, pk_a, Clock(2))
            .await
            .unwrap();

        assert_eq!(s_a, ambre_core::Seq(1));
        assert_eq!(s_b, ambre_core::Seq(2));
        assert_eq!(s_a2, ambre_core::Seq(3));

        let _ = redis
            .del([
                ClocksKey::of(&doc_id).as_str(),
                SeqKey::of(&doc_id).as_str(),
            ])
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
        let lock_key = FlushLockKey::of(&ambre_core::DocumentId::new());
        let _ = redis.del([lock_key.as_str()]).await;

        // First caller acquires; a concurrent instance cannot while it is held.
        assert!(
            try_acquire_flush_lock(
                &redis,
                &lock_key,
                &LockToken("tokA".to_owned()),
                ambre_core::Seconds(60)
            )
            .await
            .unwrap()
        );
        assert!(
            !try_acquire_flush_lock(
                &redis,
                &lock_key,
                &LockToken("tokB".to_owned()),
                ambre_core::Seconds(60)
            )
            .await
            .unwrap()
        );

        // A non-owner (wrong token) must not be able to release it.
        release_flush_lock(&redis, &lock_key, &LockToken("tokB".to_owned()))
            .await
            .unwrap();
        assert!(
            !try_acquire_flush_lock(
                &redis,
                &lock_key,
                &LockToken("tokB".to_owned()),
                ambre_core::Seconds(60)
            )
            .await
            .unwrap()
        );

        // The real owner releases; the lock is now free again.
        release_flush_lock(&redis, &lock_key, &LockToken("tokA".to_owned()))
            .await
            .unwrap();
        assert!(
            try_acquire_flush_lock(
                &redis,
                &lock_key,
                &LockToken("tokB".to_owned()),
                ambre_core::Seconds(60)
            )
            .await
            .unwrap()
        );

        let _ = redis.del([lock_key.as_str()]).await;
    }

    /// `SRV-080`: one author's frame sent at once over sixteen connections, all
    /// at the same clock. The check and the number are one step, so one call is
    /// numbered, the others are refused, and the counter moves by one.
    #[tokio::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs redis; run with --features db")]
    async fn one_clock_sent_at_once_is_numbered_once() {
        let doc = ambre_core::DocumentId::new();
        let ab = ambre_core::SigningKeyHex("ab".into());
        let mut clients = Vec::new();
        for _ in 0..16 {
            clients.push(connect_redis().await);
        }
        let calls = clients
            .iter()
            .map(|redis| verify_clock_and_assign_seq(redis, &doc, &ab, Clock(1)));
        let results = futures_util::future::join_all(calls).await;

        let numbered: Vec<ambre_core::Seq> = results
            .iter()
            .filter_map(|r| r.as_ref().ok())
            .copied()
            .collect();
        assert_eq!(numbered, [ambre_core::Seq(1)], "results {results:?}");
        let refused = results
            .iter()
            .filter(|r| {
                matches!(
                    r,
                    Err(AppError::ClockMismatch {
                        expected: Clock(2),
                        got: Clock(1)
                    })
                )
            })
            .count();
        assert_eq!(refused, 15);
        let redis = connect_redis().await;
        let seq: u64 = redis.get(format!("doc:{doc}:seq")).await.unwrap();
        assert_eq!(seq, 1);
        assert_eq!(
            verify_clock_and_assign_seq(&redis, &doc, &ab, Clock(2))
                .await
                .unwrap(),
            ambre_core::Seq(2)
        );
        let _ = redis
            .del([format!("doc:{doc}:seq"), format!("doc:{doc}:clocks")])
            .await;
    }

    #[tokio::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs redis; run with --features db")]
    async fn dirty_set_marks_on_append_and_clears_only_when_drained() {
        use rustis::commands::SetCommands;
        let redis = connect_redis().await;
        let doc = ambre_core::DocumentId::new();
        let pending_key = PendingKey::of(&doc);
        let _ = redis.del([pending_key.as_str()]).await;
        // Start from a clean marker for this doc's key.
        let _: Result<usize, _> = redis
            .srem(FlushDirtySet.as_str(), [pending_key.as_str()])
            .await;

        // Appending a delta marks the pending list dirty.
        append_pending_delta(
            &redis,
            &doc,
            ambre_core::Seq(1),
            &ambre_core::ObjectKey::from("s3-x"),
            &Payload::new(vec![1, 2, 3]),
        )
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
            .sismember(FlushDirtySet.as_str(), pending_key.as_str())
            .await
            .unwrap();
        assert!(still, "a non-empty pending list must keep its dirty marker");

        // Once drained, the marker clears.
        let _ = redis.del([pending_key.as_str()]).await;
        clear_pending_dirty_if_empty(&redis, &pending_key)
            .await
            .unwrap();
        let gone: bool = redis
            .sismember(FlushDirtySet.as_str(), pending_key.as_str())
            .await
            .unwrap();
        assert!(!gone, "a drained pending list must clear its dirty marker");

        let _ = redis
            .srem(FlushDirtySet.as_str(), [pending_key.as_str()])
            .await;
    }

    #[tokio::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs redis; run with --features db")]
    async fn pending_map_roundtrips_seq_to_wire() {
        let redis = connect_redis().await;
        let doc = ambre_core::DocumentId::new();
        let key = format!("doc:{doc}:pending");
        let _ = redis.del([key.as_str()]).await;

        append_pending_delta(
            &redis,
            &doc,
            ambre_core::Seq(7),
            &ambre_core::ObjectKey::from("s3-a"),
            &Payload::new(vec![1, 2, 3]),
        )
        .await
        .unwrap();
        append_pending_delta(
            &redis,
            &doc,
            ambre_core::Seq(8),
            &ambre_core::ObjectKey::from("s3-b"),
            &Payload::new(vec![4, 5]),
        )
        .await
        .unwrap();

        let map = get_pending_deltas_map(&redis, &doc).await.unwrap();
        assert_eq!(map.len(), 2);
        assert_eq!(
            map.get(&ambre_core::Seq(7)).map(Payload::expose_bytes),
            Some(&[1u8, 2, 3][..])
        );
        assert_eq!(
            map.get(&ambre_core::Seq(8)).map(Payload::expose_bytes),
            Some(&[4u8, 5][..])
        );

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
        let pk = &ambre_core::SigningKeyHex(
            "0badc0de0badc0de0badc0de0badc0de0badc0de0badc0de0badc0de0badc0de".into(),
        );
        let _ = redis.del([key.as_str()]).await;

        doc_presence_update(
            &redis,
            &doc,
            pk,
            &super::tests::sample(),
            ambre_core::Seconds(15),
        )
        .await
        .unwrap();
        let present: bool = redis.hexists(&key, pk.as_str()).await.unwrap();
        assert!(present, "the sample is in the hash");

        doc_presence_remove(&redis, &doc, pk).await.unwrap();
        let present: bool = redis.hexists(&key, pk.as_str()).await.unwrap();
        assert!(!present, "the departure removed the field");

        // Removing a field nobody wrote is not an error: a connection that never
        // sent presence still announces its departure.
        doc_presence_remove(
            &redis,
            &doc,
            &ambre_core::SigningKeyHex("never-wrote".into()),
        )
        .await
        .unwrap();

        let _ = redis.del([key.as_str()]).await;
    }

    #[tokio::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs redis; run with --features db")]
    async fn oidc_state_roundtrips() {
        let redis = connect_redis().await;
        let csrf = ambre_core::OidcState::new(uuid::Uuid::new_v4().to_string());
        let kept = OidcAuthState {
            pkce_verifier_secret: OidcSecret::new("verifier-xyz".to_owned()),
            nonce: openidconnect::Nonce::new("nonce-abc".to_owned()),
        };
        oidc_state_set(&redis, &csrf, &kept)
            .await
            .expect("oidc state set");
        let taken = oidc_state_take(&redis, &csrf)
            .await
            .expect("oidc state take");
        assert_eq!(taken.pkce_verifier_secret.expose(), "verifier-xyz");
        assert_eq!(taken.nonce.secret(), "nonce-abc");
    }

    #[tokio::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs redis; run with --features db")]
    async fn presence_field_self_expires() {
        let redis = connect_redis().await;
        let doc = ambre_core::DocumentId::new();
        let key = format!("doc:{doc}:presence");
        let pk = &ambre_core::SigningKeyHex(
            "deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef0".into(),
        );
        let _ = redis.del([key.as_str()]).await;

        doc_presence_update(
            &redis,
            &doc,
            pk,
            &super::tests::sample(),
            ambre_core::Seconds(15),
        )
        .await
        .unwrap();

        //HTTL returns the remaining per-field TTL; a positive value proves the
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
        let author = &ambre_core::SigningKeyHex(
            "deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef0".into(),
        );
        // Cold Redis (as after a restart): neither key exists.
        let _ = redis.del([clocks_key.as_str(), seq_key.as_str()]).await;
        assert!(clock_state_is_cold(&redis, &doc).await.unwrap());

        // The durable index says this author reached clock 5, global seq 5.
        seed_clock_state(
            &redis,
            &doc,
            ambre_core::Seq(5),
            &[(author.clone(), Clock(5))],
        )
        .await
        .expect("seed");

        // No longer cold, and the author's next update (clock 6) validates.
        assert!(!clock_state_is_cold(&redis, &doc).await.unwrap());
        let seq = verify_clock_and_assign_seq(&redis, &doc, author, Clock(6))
            .await
            .expect("clock 6 should validate against a rehydrated current=5");
        assert_eq!(
            seq,
            ambre_core::Seq(6),
            "seq counter must continue past the durable max"
        );

        // A stale clock (already consumed) is still rejected.
        assert!(
            verify_clock_and_assign_seq(&redis, &doc, author, Clock(6))
                .await
                .is_err()
        );

        let _ = redis.del([clocks_key.as_str(), seq_key.as_str()]).await;
    }

    async fn seed_refresh(redis: &Client) -> RefreshToken {
        let token = RefreshToken::new(uuid::Uuid::new_v4().to_string());
        let data = RefreshData {
            user_id: ambre_core::UserId::new(),
            signing_pubkey: Some(ambre_core::PublicKey::from_array([3u8; 32])),
        };
        refresh_create(redis, &token, &data, ambre_core::Seconds(60))
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
