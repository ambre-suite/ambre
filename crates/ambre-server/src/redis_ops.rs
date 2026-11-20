use crate::error::{AppError, Resource};
use ambre_core::id::{Channel, Document, Id, User};
use rustis::client::Client;
use rustis::commands::{
    GenericCommands, HashCommands, InfoSection, ListCommands, PubSubCommands, ScriptingCommands,
    ServerCommands, StringCommands,
};
use rustis::resp::RefBulkString;

use crate::oidc::{OidcAuthState, OidcSecret, OidcSecretSinks};
use crate::payload::StoredPayload;
use crate::redis_keys::{
    Clocks, DeadLetter, DocKey, DocumentStateKeys, FlushDirtySet, FlushLock, Gone, Locked,
    PendingKey, Presence, RotationOwed, SeqCounter, Sessions, SnapshotAsked,
};
use ambre_core::{FixedBytes, PublicKey};
use ambre_protocol::messages::{DeltaAuthor, Seq};
use ambre_protocol::ws::{Credential, RefreshToken};

/// Atomic Lua script that validates a client clock and assigns a global
/// monotonic sequence number.
///
/// - `KEYS[1]` = `doc:{id}:clocks` (hash: pub_key_hex -> last_clock)
/// - `KEYS[2]` = `doc:{id}:seq`   (counter)
/// - `KEYS[3]` = `doc:{id}:gone`  (tombstone, see [`mark_document_gone`])
/// - `KEYS[4]` = `doc:{id}:rotation-owed` (see [`DocKey<RotationOwed>`])
/// - `KEYS[5]` = `doc:{id}:locked` (see [`DocKey<Locked>`])
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
    document_id: &Id<Document>,
    pub_key: &FixedBytes<PublicKey>,
    clock: Seq<DeltaAuthor>,
) -> Result<ambre_core::Seq<ambre_core::Relay>, AppError> {
    let pub_key_hex = pub_key.to_hex();
    let clocks_key = DocKey::<Clocks>::of(document_id);
    let seq_key = DocKey::<SeqCounter>::of(document_id);
    let gone_key = DocKey::<Gone>::of(document_id);
    let owed_key = DocKey::<RotationOwed>::of(document_id);
    let locked_key = DocKey::<Locked>::of(document_id);

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
    Ok(ambre_core::Seq::<ambre_core::Relay>::new(
        u64::try_from(result).unwrap_or(0),
    ))
}

/// Reads the script's `CLOCK_MISMATCH:<current>` refusal out of a store error.
/// The expected clock is the one after the current. A document being deleted
/// is not found; anything else stays a store failure.
fn clock_refusal(error: &str, got: Seq<DeltaAuthor>) -> AppError {
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
            expected: Seq::<DeltaAuthor>::new(current.saturating_add(1)),
            got,
        },
        None => AppError::Redis(crate::error::FaultText::of(
            "clock validation failed",
            error,
        )),
    }
}

/// The sequence counter of each document, in the order asked. `None` where
/// the ephemeral store holds no counter: the document was never written, or
/// the store was reset and nothing wrote since.
pub async fn document_seqs(
    redis: &Client,
    document_ids: &[Id<Document>],
) -> Result<Vec<Option<ambre_core::Seq<ambre_core::Relay>>>, AppError> {
    if document_ids.is_empty() {
        return Ok(Vec::new());
    }
    let keys: Vec<String> = document_ids
        .iter()
        .map(|id| DocKey::<SeqCounter>::of(id).as_str().to_owned())
        .collect();
    let counters = redis
        .mget::<Vec<Option<u64>>>(keys)
        .await
        .map_err(|e| AppError::Redis(crate::error::FaultText::of("sequence counters", e)))?;
    Ok(counters
        .into_iter()
        .map(|c| c.map(ambre_core::Seq::<ambre_core::Relay>::new))
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
    document_id: &Id<Document>,
) -> Result<bool, AppError> {
    let clocks_key = DocKey::<Clocks>::of(document_id);
    let existing: usize = redis.exists(clocks_key.as_str()).await.map_err(|e| {
        AppError::Redis(crate::error::FaultText::of("clock state EXISTS failed", e))
    })?;
    Ok(existing == 0)
}

/// Reseeds the Redis clock/seq state from the durable per-author high-water marks
/// (see [`REHYDRATE_CLOCK_LUA`]). `author_clocks` are `(pub_key_hex, max_clock)`.
/// A no-op when there is no durable history (a brand-new document).
///
/// The counter is seeded even with no author clock. A document whose deltas
/// the trim removed still holds its snapshot horizons. A counter left at 0
/// would number the next delta under them (`SRV-288`).
pub async fn seed_clock_state(
    redis: &Client,
    document_id: &Id<Document>,
    max_seq: ambre_core::Seq<ambre_core::Relay>,
    author_clocks: &[(FixedBytes<PublicKey>, Seq<DeltaAuthor>)],
) -> Result<(), AppError> {
    if author_clocks.is_empty() && max_seq == ambre_core::Seq::<ambre_core::Relay>::new(0) {
        return Ok(());
    }
    let clocks_key = DocKey::<Clocks>::of(document_id);
    let seq_key = DocKey::<SeqCounter>::of(document_id);
    let gone_key = DocKey::<Gone>::of(document_id);
    let mut argv = vec![max_seq.get().to_string()];
    for (key, clock) in author_clocks {
        argv.push(key.to_hex());
        argv.push(clock.to_string());
    }
    redis
        .eval::<i64>(
            REHYDRATE_CLOCK_LUA,
            [clocks_key.as_str(), seq_key.as_str(), gone_key.as_str()],
            argv.iter().map(String::as_str).collect::<Vec<_>>(),
        )
        .await
        .map_err(|e| {
            AppError::Redis(crate::error::FaultText::of(
                "clock state rehydrate failed",
                e,
            ))
        })?;
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
///
/// # Errors
///
/// The push itself failed: the list does not hold the delta. A dirty mark
/// that fails after the push is no error: it comes back as
/// [`Queued::MarkMissed`] (`SRV-262`).
pub async fn append_pending_delta(
    redis: &Client,
    document_id: &Id<Document>,
    seq: ambre_core::Seq<ambre_core::Relay>,
    s3_key: &ambre_core::ObjectKey,
    update_wire: &StoredPayload,
) -> Result<Queued, AppError> {
    let key = PendingKey::of(document_id);
    // Store the FULL sealed Update wire (ciphertext + nonce + public_data +
    // commitment + signature), not just the ciphertext. A reconnecting or
    // cross-device client replays these verbatim, and needs every field to
    // decrypt and verify. The flusher persists the same bytes to S3.
    let entry = StoredPayload::pending_entry(seq, s3_key, update_wire);
    // The script names the keys of the document alone: the tombstone check and
    // the RPUSH, which answers the new length of the list. The dirty set is a
    // key of another slot, so its SADD is a command of its own (`SRV-253`).
    //
    // The SADD runs after the RPUSH, and only when the list was empty before
    // it. The push that filled a list set its mark. `clear_pending_dirty_if_empty`
    // sets it again for a list it finds not empty. So one SADD per drained
    // list, not per delta.
    // A relay that dies between the RPUSH and the SADD leaves the list
    // unmarked, and the reconciling sweep of the flusher finds it.
    const LUA: &str = r#"
if redis.call('EXISTS', KEYS[2]) == 1 then
    return redis.error_reply('DOCUMENT_GONE')
end
return redis.call('RPUSH', KEYS[1], ARGV[1])
"#;
    let gone_key = DocKey::<Gone>::of(document_id);
    let length: i64 = redis
        .eval::<i64>(
            LUA,
            [key.as_str(), gone_key.as_str()],
            [RefBulkString::new(entry.expose_bytes())],
        )
        .await
        .map_err(|e| store_refusal(&e.to_string(), "pending push failed"))?;
    if length != 1 {
        return Ok(Queued::Marked);
    }
    // The entry is queued: a failed mark delays its flush until the next
    // sweep, and loses nothing. So the failure is no `Err`, which the caller
    // would answer by a dead letter of a delta the list already holds. The
    // store may not log it (`SRV-001`), so it hands it to the caller.
    Ok(match mark_pending_dirty(redis, &key).await {
        Ok(()) => Queued::Marked,
        Err(e) => Queued::MarkMissed(e),
    })
}

/// What became of the dirty mark of a delta the pending list took.
///
/// The list holds the delta under both variants. The caller logs a missed
/// mark and counts it, since the flush of that list waits for the next
/// reconciling sweep (`SRV-262`).
#[must_use]
#[derive(Debug)]
pub enum Queued {
    /// The dirty set names the list: the next flusher cycle visits it.
    Marked,
    /// The dirty set refused the mark of a list the push had just filled.
    MarkMissed(AppError),
}

/// The refusal every tombstone-guarded script answers for a document being
/// deleted (see [`mark_document_gone`]).
const DOCUMENT_GONE: &str = "DOCUMENT_GONE";

/// The refusal the clock and presence scripts answer for a document that owes
/// a rotation (see [`DocKey<RotationOwed>`]).
const ROTATION_OWED: &str = "ROTATION_OWED";

/// The refusal the clock script answers for a locked segment (see
/// [`DocKey<Locked>`]).
const SEGMENT_LOCKED: &str = "SEGMENT_LOCKED";

/// Reads a guarded script's refusal out of a store error: a document being
/// deleted is not found, anything else is a store failure.
fn store_refusal(error: &str, what: &str) -> AppError {
    if error.contains(DOCUMENT_GONE) {
        AppError::NotFound {
            resource: Resource::Document,
        }
    } else {
        AppError::Redis(crate::error::FaultText::of(what, error))
    }
}

/// RPUSHes `entry` onto a document's dead-letter list unless its tombstone
/// exists. The flusher and the write path use it, so neither recreates a
/// purged list.
async fn guarded_rpush(
    redis: &Client,
    list_key: &DocKey<DeadLetter>,
    gone_key: &DocKey<Gone>,
    entry: &StoredPayload,
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
    document_id: &Id<Document>,
    entry: &StoredPayload,
) -> Result<(), AppError> {
    guarded_rpush(
        redis,
        &DocKey::<DeadLetter>::of(document_id),
        &DocKey::<Gone>::of(document_id),
        entry,
    )
    .await
}

/// The entries of a pending list, front first.
pub async fn pending_entries(
    redis: &Client,
    pending_key: &PendingKey,
) -> Result<Vec<StoredPayload>, AppError> {
    let entries: Vec<String> = redis
        .lrange(pending_key.as_str(), 0, -1)
        .await
        .map_err(|e| AppError::Redis(crate::error::FaultText::of("pending lrange failed", e)))?;
    Ok(entries
        .into_iter()
        .map(|entry| StoredPayload::new(entry.into_bytes()))
        .collect())
}

/// Records `pending_key` in the dirty set so the flusher will visit it.
pub async fn mark_pending_dirty(redis: &Client, pending_key: &PendingKey) -> Result<(), AppError> {
    use rustis::commands::SetCommands;
    redis
        .sadd(FlushDirtySet.as_str(), [pending_key.as_str()])
        .await
        .map_err(|e| AppError::Redis(crate::error::FaultText::of("mark pending dirty", e)))?;
    Ok(())
}

/// Returns the pending keys currently marked dirty (the flusher's work list).
/// A member this relay never writes names no list to flush, and is skipped.
pub async fn dirty_pending_keys(redis: &Client) -> Result<Vec<PendingKey>, AppError> {
    use rustis::commands::SetCommands;
    let members: Vec<String> = redis
        .smembers(FlushDirtySet.as_str())
        .await
        .map_err(|e| AppError::Redis(crate::error::FaultText::of("dirty pending keys", e)))?;
    Ok(members
        .iter()
        .filter_map(|member| PendingKey::parse(member))
        .collect())
}

/// Clears `pending_key` from the dirty set, but keeps it there when its
/// pending list is not empty. Run after a drain.
///
/// The dirty set and the list sit in two slots, so no script may read one and
/// write the other (`SRV-253`). Three commands run in this order instead:
/// SREM the mark, read LLEN, and SADD the mark again when the list holds an
/// entry. An append RPUSHes before it SADDs. So an append whose RPUSH lands
/// after the LLEN finds an empty list, and SADDs after the SREM. One whose
/// RPUSH lands before the LLEN makes the LLEN count it, and the mark is set
/// again. Either way an acknowledged delta is never left unmarked.
pub async fn clear_pending_dirty_if_empty(
    redis: &Client,
    pending_key: &PendingKey,
) -> Result<(), AppError> {
    use rustis::commands::SetCommands;
    let _: usize = redis
        .srem(FlushDirtySet.as_str(), [pending_key.as_str()])
        .await
        .map_err(|e| AppError::Redis(crate::error::FaultText::of("clear pending dirty", e)))?;
    let length: usize = redis
        .llen(pending_key.as_str())
        .await
        .map_err(|e| AppError::Redis(crate::error::FaultText::of("clear pending dirty", e)))?;
    if length > 0 {
        mark_pending_dirty(redis, pending_key).await?;
    }
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
    document_id: &Id<Document>,
    seq: ambre_core::Seq<ambre_core::Relay>,
    s3_key: &ambre_core::ObjectKey,
    update_wire: &StoredPayload,
) -> Result<(), AppError> {
    guarded_rpush(
        redis,
        &DocKey::<DeadLetter>::of(document_id),
        &DocKey::<Gone>::of(document_id),
        &StoredPayload::pending_entry(seq, s3_key, update_wire),
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
    document_id: &Id<Document>,
) -> Result<std::collections::HashMap<ambre_core::Seq<ambre_core::Relay>, StoredPayload>, AppError>
{
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
    lock_key: &DocKey<FlushLock>,
    token: &LockToken,
    ttl: TimeSpan<Sec>,
) -> Result<bool, AppError> {
    let ttl = ttl.get().to_string();
    let res: Option<String> = redis
        .eval::<Option<String>>(
            ACQUIRE_LUA,
            [lock_key.as_str()],
            [token.as_str(), ttl.as_str()],
        )
        .await
        .map_err(|e| AppError::Redis(crate::error::FaultText::of("flush lock acquire", e)))?;
    Ok(res.is_some())
}

/// `SET key token NX EX ttl`: the key is set only when it stands unset.
const ACQUIRE_LUA: &str = "return redis.call('SET', KEYS[1], ARGV[1], 'NX', 'EX', ARGV[2])";

/// Releases the per-document flush lock, but only if `token` still matches
/// (check-and-del). An expired-then-reacquired lock owned by another instance
/// is never dropped by a slow predecessor.
pub async fn release_flush_lock(
    redis: &Client,
    lock_key: &DocKey<FlushLock>,
    token: &LockToken,
) -> Result<(), AppError> {
    const LUA: &str = "if redis.call('GET', KEYS[1]) == ARGV[1] then return redis.call('DEL', KEYS[1]) else return 0 end";
    let _: i64 = redis
        .eval::<i64>(LUA, [lock_key.as_str()], [token.as_str()])
        .await
        .map_err(|e| AppError::Redis(crate::error::FaultText::of("flush lock release", e)))?;
    Ok(())
}

/// Claims the one standing snapshot request of a document for `ttl`
/// (`SRV-038`). Returns `true` when no request stood, so this caller asks. Every
/// instance shares the claim, so one threshold crossing asks once.
pub async fn claim_snapshot_ask(
    redis: &Client,
    document_id: &Id<Document>,
    ttl: TimeSpan<Sec>,
) -> Result<bool, AppError> {
    let key = DocKey::<SnapshotAsked>::of(document_id);
    let ttl = ttl.get().to_string();
    let res: Option<String> = redis
        .eval::<Option<String>>(ACQUIRE_LUA, [key.as_str()], ["1", ttl.as_str()])
        .await
        .map_err(|e| AppError::Redis(crate::error::FaultText::of("snapshot ask claim", e)))?;
    Ok(res.is_some())
}

/// The last delta number the relay gave the document, 0 when its counter is
/// cold.
pub async fn current_seq(
    redis: &Client,
    document_id: &Id<Document>,
) -> Result<ambre_core::Seq<ambre_core::Relay>, AppError> {
    let seq: Option<u64> = redis
        .get(DocKey::<SeqCounter>::of(document_id).as_str())
        .await
        .map_err(|e| AppError::Redis(crate::error::FaultText::of("current seq", e)))?;
    Ok(ambre_core::Seq::<ambre_core::Relay>::new(seq.unwrap_or(0)))
}

/// Retrieves a refresh-token record from Redis.
///
/// Refresh keys follow the pattern `refresh:{SHA-256 of the token}` with fields `user_id` and
/// `signing_pubkey_hex`. Returns `None` when the token is unknown, expired, or
/// was revoked (logout / rotation).
pub async fn refresh_get(
    redis: &Client,
    token: &Credential<RefreshToken>,
) -> Result<Option<RefreshData>, AppError> {
    // `hgetall` returns field/value pairs; rustis deserializes them as a map.
    // (Reading into `Vec<String>` fails with "invalid type: sequence, expected a
    // string" because each pair is itself a sequence.)
    let fields: std::collections::HashMap<String, String> =
        crate::secrets::token::refresh_key(token, |key| redis.hgetall(key))
            .await
            .map_err(|e| AppError::Redis(crate::error::FaultText::of("refresh get failed", e)))?;

    if fields.is_empty() {
        return Ok(None);
    }

    let user_id = fields.get("user_id").cloned().unwrap_or_default();

    let user_id = ambre_core::Uuid::parse_str(&user_id)
        .map(Id::<User>::from_uuid)
        .map_err(|e| {
            AppError::Redis(crate::error::FaultText::of("invalid user_id in refresh", e))
        })?;

    // An absent field is a session without keys; a present one must parse.
    let signing_pubkey = fields
        .get("signing_pubkey_hex")
        .map(|hex| FixedBytes::<PublicKey>::from_hex(hex))
        .transpose()
        .map_err(|e| {
            AppError::Redis(crate::error::FaultText::of("invalid pubkey in refresh", e))
        })?;

    Ok(Some(RefreshData {
        user_id,
        signing_pubkey,
    }))
}

/// Writes the fields of a hash and its lifetime in one script, so no record
/// ever exists without its lifetime (`SRV-291`). Two commands left a window: a
/// relay that died between them kept the record for good.
///
/// - `KEYS[1]` = the record
/// - `ARGV[1]` = the lifetime, in seconds
/// - `ARGV[2..]` = field, value pairs
const HASH_WITH_TTL_LUA: &str = r#"
redis.call('HSET', KEYS[1], unpack(ARGV, 2))
redis.call('EXPIRE', KEYS[1], ARGV[1])
return 1
"#;

/// Creates a refresh-token record in Redis with the configured TTL.
pub async fn refresh_create(
    redis: &Client,
    token: &Credential<RefreshToken>,
    data: &RefreshData,
    ttl: TimeSpan<Sec>,
) -> Result<(), AppError> {
    let ttl_secs = ttl.get().to_string();
    let user_id = data.user_id.to_string();
    let signing_pubkey_hex = data.signing_pubkey.as_ref().map(FixedBytes::to_hex);
    let mut args = vec![ttl_secs.as_str(), "user_id", user_id.as_str()];
    if let Some(hex) = &signing_pubkey_hex {
        args.extend(["signing_pubkey_hex", hex.as_str()]);
    }
    crate::secrets::token::refresh_key(token, |key| {
        redis.eval::<i64>(HASH_WITH_TTL_LUA, [key], args)
    })
    .await
    .map_err(|e| AppError::Redis(crate::error::FaultText::of("refresh create", e)))?;
    Ok(())
}

/// Deletes a refresh-token record (logout, or the old token during rotation).
pub async fn refresh_delete(
    redis: &Client,
    token: &Credential<RefreshToken>,
) -> Result<(), AppError> {
    crate::secrets::token::refresh_key(token, |key| redis.del([key]))
        .await
        .map_err(|e| AppError::Redis(crate::error::FaultText::of("refresh delete failed", e)))?;
    Ok(())
}

/// Claims a refresh-token record, returning `true` only for the caller that
/// actually removed the key.
///
/// `DEL` is atomic and reports how many keys it removed, so when several callers
/// race on the same token exactly one observes `1`. That caller owns the claim;
/// every other one observes `0` and must be rejected as a replay.
pub async fn refresh_claim(
    redis: &Client,
    token: &Credential<RefreshToken>,
) -> Result<bool, AppError> {
    let removed: usize = crate::secrets::token::refresh_key(token, |key| redis.del([key]))
        .await
        .map_err(|e| AppError::Redis(crate::error::FaultText::of("refresh claim del", e)))?;
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
    token: &Credential<RefreshToken>,
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
/// All active subscribers (across Axum instances) receive the message. Every
/// broadcast of the relay goes out on a sharded channel, in the slot of its
/// topic (`SRV-254`).
pub async fn doc_broadcast(
    redis: &Client,
    document_id: &Id<Document>,
    payload: &StoredPayload,
) -> Result<(), AppError> {
    let channel = crate::ws::Topic::document(document_id);
    // The payload is binary (encrypted ciphertext). rustis serializes a raw
    // `&[u8]` as a sequence of integers (a serde limitation) and panics. Wrap it
    // in `RefBulkString` so it is sent as a single RESP bulk string.
    redis
        .spublish(channel.as_str(), RefBulkString::new(payload.expose_bytes()))
        .await
        .map_err(|e| AppError::Redis(crate::error::FaultText::of("doc broadcast", e)))?;
    Ok(())
}

/// Publishes a message to a user's personal PubSub topic (`user:{id}`).
///
/// Reaches every live connection of that user, whatever documents they have
/// open. Used for server-initiated, non-document-scoped pushes such as
/// `DocumentShared` on a fresh invite.
pub async fn user_broadcast(
    redis: &Client,
    user_id: &Id<User>,
    payload: &StoredPayload,
) -> Result<(), AppError> {
    let channel = crate::ws::Topic::user(user_id);
    redis
        .spublish(channel.as_str(), RefBulkString::new(payload.expose_bytes()))
        .await
        .map_err(|e| AppError::Redis(crate::error::FaultText::of("user broadcast", e)))?;
    Ok(())
}

/// Tells every connection subscribed to a document that its members changed
/// (`SRV-177`). The message names the document alone, which the topic already
/// names. Best-effort, like every broadcast: the member rows stay the source of
/// truth, and a client reads them again at its next open.
pub async fn members_changed(redis: &Client, document_id: &Id<Document>) {
    let msg = crate::payload::StoredPayload::server_msg(
        &ambre_protocol::ws::WsServerMsg::MembersChanged {
            document_id: *document_id,
        },
    );
    let _ = doc_broadcast(redis, document_id, &msg).await;
}

/// Tells every socket watching a document of `channel_id` that its thread
/// `thread` took a delta (`SRV-161`). The signal names the thread alone, which
/// the relay already routes on (`SRV-162`). Best-effort: a client that missed
/// it counts the delta at the next one, or reads it when it opens the thread.
pub async fn thread_activity(
    redis: &Client,
    channel_id: &Id<Channel>,
    thread: &Id<Document>,
) -> Result<(), AppError> {
    let msg = StoredPayload::server_msg(&ambre_protocol::ws::WsServerMsg::ThreadActivity {
        document_id: *thread,
    });
    let topic = crate::ws::Topic::channel(channel_id);
    redis
        .spublish(topic.as_str(), RefBulkString::new(msg.expose_bytes()))
        .await
        .map_err(|e| AppError::Redis(crate::error::FaultText::of("thread activity", e)))?;
    Ok(())
}

/// Publishes one signal of a call on its topic: a join, a leave or a wrap (`SRV-049`).
/// Best-effort, as the fan-out of the document plane: a socket that misses a join reads it
/// in the roster of its own admission.
pub async fn call_signal(
    redis: &Client,
    call: &Id<ambre_core::id::Call>,
    msg: &ambre_protocol::signaling::CallServerMsg,
) -> Result<(), AppError> {
    let frame = StoredPayload::seal(msg).map_err(|e| AppError::Internal(format!("{e}")))?;
    redis
        .spublish(
            crate::ws::Topic::call(call).as_str(),
            RefBulkString::new(frame.expose_bytes()),
        )
        .await
        .map_err(|e| AppError::Redis(crate::error::FaultText::of("call signal", e)))?;
    Ok(())
}

/// What a join gets from the state of a call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CallJoin {
    /// The session holds a seat. The room of the call, and the sender index the session
    /// holds, which no earlier join of the call held (`CALL-056`).
    Admitted {
        room: FixedBytes<ambre_core::RoomPseudonym>,
        sender: ambre_core::Seq<ambre_core::SenderIndex>,
    },
    /// The call refuses the session, which the relay states in a notice.
    Refused(ambre_protocol::signaling::CallRefusal),
}

/// Whether a seat still holds its call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CallHold {
    /// The state of the call and the seat are in the store, under a fresh time to live. The
    /// seats whose field expired since the last heartbeat of any seat are gone, and the relay
    /// announces their leave (`CALL-063`).
    Held {
        gone: Vec<ambre_core::Seq<ambre_core::SenderIndex>>,
    },
    /// The state of the call or the seat is gone: the call ended for this seat.
    Lost,
}

/// Whether a first frame starts its call or joins one that runs (`CALL-068`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallOpening {
    /// The call runs: a join of a call the store holds no state of is refused as ended.
    Join,
    /// The join creates the state of the call when the store holds none.
    Start,
}

impl CallOpening {
    /// The word the join script reads.
    const fn word(self) -> &'static str {
        match self {
            Self::Join => "join",
            Self::Start => "start",
        }
    }
}

/// The most seats one signing key holds in one call at once: the devices of
/// one member (`SRV-306`). Past it, the
/// member reads the call as full, and the other members keep their seats.
const MEMBER_SEAT_CAP: ambre_core::Count<ambre_protocol::signaling::CallParticipants> =
    ambre_core::Count::new(4);

/// The most sender indexes one signing key takes in one call. An index is
/// never given back, so a member who joins and leaves in a loop would spend
/// every index of the call. Past it, the member reads the call as exhausted
/// (`SRV-306`).
const MEMBER_JOIN_CAP: ambre_core::Seq<ambre_core::SenderIndex> = ambre_core::Seq::new(256);

/// Takes a seat of `call` for the session of `key`, in one script (`SRV-248`).
///
/// A start creates the state of the call in `channel`, under the room pseudonym `room`, which
/// the caller draws from the generator of the OS. A join of a call with no state is refused as
/// ended, so a lost or ended call never starts again (`CALL-068`). A later join reads the room
/// the start stored.
///
/// The script takes the seat and the index at once. So two joins never both take the
/// last seat under the cap (`CALL-021`), nor one sender index (`CALL-056`). The counter of the
/// call only grows, and a leave gives no index back. One key holds at most
/// [`MEMBER_SEAT_CAP`] seats and takes at most [`MEMBER_JOIN_CAP`] indexes (`SRV-306`). Every
/// key, and the field of the seat, take the time to live `ttl`.
///
/// # Errors
/// A store error, or an answer the script does not write.
pub async fn join_call(
    redis: &Client,
    call: &Id<ambre_core::id::Call>,
    channel: &Id<Channel>,
    key: &FixedBytes<PublicKey>,
    room: &FixedBytes<ambre_core::RoomPseudonym>,
    opening: CallOpening,
    ttl: TimeSpan<Sec>,
) -> Result<CallJoin, AppError> {
    use ambre_protocol::signaling::{CallRefusal, PARTICIPANT_CAP};
    // HKEYS skips a field whose time to live ran out, so a seat no heartbeat holds counts
    // for nothing.
    const LUA: &str = r#"
if redis.call('EXISTS', KEYS[1]) == 0 then
    if ARGV[7] ~= 'start' then return {'ended'} end
    redis.call('HSET', KEYS[1], 'channel', ARGV[1], 'room', ARGV[2], 'next', 0)
end
local state = redis.call('HMGET', KEYS[1], 'channel', 'room', 'next')
if state[1] ~= ARGV[1] then return {'channel'} end
if #redis.call('HKEYS', KEYS[2]) >= tonumber(ARGV[4]) then return {'full'} end
local held = 0
for _, holder in ipairs(redis.call('HVALS', KEYS[2])) do
    if holder == ARGV[3] then held = held + 1 end
end
if held >= tonumber(ARGV[8]) then return {'full'} end
local taken = tonumber(redis.call('HGET', KEYS[1], 'taken:' .. ARGV[3])) or 0
if taken >= tonumber(ARGV[9]) then return {'exhausted'} end
local sender = tonumber(state[3])
if sender >= tonumber(ARGV[5]) then return {'exhausted'} end
redis.call('HSET', KEYS[1], 'next', sender + 1)
redis.call('HINCRBY', KEYS[1], 'taken:' .. ARGV[3], 1)
redis.call('HSET', KEYS[2], tostring(sender), ARGV[3])
redis.call('HEXPIRE', KEYS[2], ARGV[6], 'FIELDS', 1, tostring(sender))
redis.call('SADD', KEYS[4], tostring(sender))
for i = 1, 4 do redis.call('EXPIRE', KEYS[i], ARGV[6]) end
return {'admitted', state[2], tostring(sender)}
"#;
    let keys = crate::redis_keys::CallStateKeys::of(call);
    let answer: Vec<String> = redis
        .eval(
            LUA,
            keys.texts(),
            [
                channel.to_string(),
                room.to_hex(),
                key.to_hex(),
                PARTICIPANT_CAP.get().to_string(),
                ambre_core::SENDER_INDEX_LIMIT.get().to_string(),
                ttl.get().to_string(),
                opening.word().to_owned(),
                MEMBER_SEAT_CAP.get().to_string(),
                MEMBER_JOIN_CAP.get().to_string(),
            ],
        )
        .await
        .map_err(|e| AppError::Redis(crate::error::FaultText::of("call join", e)))?;
    let malformed = || AppError::Internal(format!("a call join answered {answer:?}"));
    match answer.as_slice() {
        [word] if word == "channel" => Ok(CallJoin::Refused(CallRefusal::NotMember)),
        [word] if word == "full" => Ok(CallJoin::Refused(CallRefusal::Full)),
        [word] if word == "exhausted" => Ok(CallJoin::Refused(CallRefusal::Exhausted)),
        [word] if word == "ended" => Ok(CallJoin::Refused(CallRefusal::Ended)),
        [word, room, sender] if word == "admitted" => Ok(CallJoin::Admitted {
            room: FixedBytes::from_hex(room).map_err(|_| malformed())?,
            sender: ambre_core::Seq::new(sender.parse().map_err(|_| malformed())?),
        }),
        _ => Err(malformed()),
    }
}

/// Keeps the signed join the seat of `sender` announced, under the time to live of its seat.
/// Only the first announce of a seat counts: a second one changes nothing, and reads `false`.
///
/// # Errors
/// A store error.
pub async fn announce_join(
    redis: &Client,
    call: &Id<ambre_core::id::Call>,
    sender: ambre_core::Seq<ambre_core::SenderIndex>,
    join: &ambre_core::Payload<ambre_core::SignedJoins>,
    ttl: TimeSpan<Sec>,
) -> Result<bool, AppError> {
    const LUA: &str = r#"
if redis.call('HEXISTS', KEYS[2], ARGV[1]) == 0 then return 0 end
if redis.call('HSETNX', KEYS[3], ARGV[1], ARGV[2]) == 0 then return 0 end
redis.call('HEXPIRE', KEYS[3], ARGV[3], 'FIELDS', 1, ARGV[1])
redis.call('EXPIRE', KEYS[3], ARGV[3])
return 1
"#;
    let keys = crate::redis_keys::CallStateKeys::of(call);
    let kept: i64 = redis
        .eval(
            LUA,
            keys.texts(),
            [
                sender.get().to_string(),
                hex::encode(join.as_bytes()),
                ttl.get().to_string(),
            ],
        )
        .await
        .map_err(|e| AppError::Redis(crate::error::FaultText::of("call announce", e)))?;
    Ok(kept == 1)
}

/// Each seat of `call` but the one of `except` that announced its join, by sender index.
///
/// # Errors
/// A store error. A field the relay did not write is skipped.
pub async fn seated(
    redis: &Client,
    call: &Id<ambre_core::id::Call>,
    except: ambre_core::Seq<ambre_core::SenderIndex>,
) -> Result<Vec<ambre_protocol::signaling::Seated>, AppError> {
    let keys = crate::redis_keys::CallStateKeys::of(call);
    let joins: std::collections::HashMap<String, String> = redis
        .hgetall(keys.joins.as_str())
        .await
        .map_err(|e| AppError::Redis(crate::error::FaultText::of("call roster", e)))?;
    let mut seated: Vec<_> = joins
        .into_iter()
        .filter_map(|(sender, join)| {
            Some(ambre_protocol::signaling::Seated {
                sender: ambre_core::Seq::new(sender.parse().ok()?),
                join: ambre_core::Payload::new(hex::decode(join).ok()?),
            })
        })
        .filter(|seated| seated.sender != except)
        .collect();
    seated.sort_by_key(|seated| seated.sender);
    Ok(seated)
}

/// The heartbeat of the seat of `sender` in the call state of `room`. It refreshes the time
/// to live of every key of `call`, and of the fields of this seat. A state, a room or a seat
/// the store lost reads
/// [`CallHold::Lost`], and the call ends for that seat (`SRV-249`).
///
/// # Errors
/// A store error.
pub async fn hold_call(
    redis: &Client,
    call: &Id<ambre_core::id::Call>,
    sender: ambre_core::Seq<ambre_core::SenderIndex>,
    room: &FixedBytes<ambre_core::RoomPseudonym>,
    ttl: TimeSpan<Sec>,
) -> Result<CallHold, AppError> {
    // Another room is a fresh call that a join started after this one was lost.
    const LUA: &str = r#"
if redis.call('HGET', KEYS[1], 'room') ~= ARGV[3] then return {'lost'} end
if redis.call('HEXISTS', KEYS[2], ARGV[1]) == 0 then return {'lost'} end
for i = 1, 4 do redis.call('EXPIRE', KEYS[i], ARGV[2]) end
redis.call('HEXPIRE', KEYS[2], ARGV[2], 'FIELDS', 1, ARGV[1])
if redis.call('HEXISTS', KEYS[3], ARGV[1]) == 1 then
    redis.call('HEXPIRE', KEYS[3], ARGV[2], 'FIELDS', 1, ARGV[1])
end
local answer = {'held'}
for _, sender in ipairs(redis.call('SMEMBERS', KEYS[4])) do
    if redis.call('HEXISTS', KEYS[2], sender) == 0 then
        redis.call('SREM', KEYS[4], sender)
        redis.call('HDEL', KEYS[3], sender)
        table.insert(answer, sender)
    end
end
return answer
"#;
    let keys = crate::redis_keys::CallStateKeys::of(call);
    let held: Vec<String> = redis
        .eval(
            LUA,
            keys.texts(),
            [
                sender.get().to_string(),
                ttl.get().to_string(),
                room.to_hex(),
            ],
        )
        .await
        .map_err(|e| AppError::Redis(crate::error::FaultText::of("call hold", e)))?;
    match held.split_first() {
        Some((word, gone)) if word == "held" => Ok(CallHold::Held {
            gone: gone
                .iter()
                .filter_map(|sender| sender.parse().ok())
                .map(ambre_core::Seq::new)
                .collect(),
        }),
        _ => Ok(CallHold::Lost),
    }
}

/// Takes the seat of `sender` out of `call`, and answers whether it was the last
/// (`CALL-069`). The last one out deletes every key of the call: a later join is refused as
/// ended (`CALL-068`). A seat already gone counts as no last leave, so one end is written once.
///
/// # Errors
/// A store error.
pub async fn leave_call(
    redis: &Client,
    call: &Id<ambre_core::id::Call>,
    sender: ambre_core::Seq<ambre_core::SenderIndex>,
) -> Result<ambre_protocol::signaling::Departure, AppError> {
    use ambre_protocol::signaling::Departure;
    const LUA: &str = r#"
local held = redis.call('HDEL', KEYS[2], ARGV[1])
redis.call('HDEL', KEYS[3], ARGV[1])
redis.call('SREM', KEYS[4], ARGV[1])
if #redis.call('HKEYS', KEYS[2]) == 0 then
    redis.call('DEL', KEYS[1], KEYS[2], KEYS[3], KEYS[4])
    return held
end
return 0
"#;
    let keys = crate::redis_keys::CallStateKeys::of(call);
    let last: i64 = redis
        .eval(LUA, keys.texts(), [sender.get().to_string()])
        .await
        .map_err(|e| AppError::Redis(crate::error::FaultText::of("call leave", e)))?;
    Ok(if last == 1 {
        Departure::Last
    } else {
        Departure::Others
    })
}

/// Who is in `call`, for an ask from a member of `channel` (`CALL-070`): the signed join of
/// each seat that still holds its field. A call of another channel reads as one with no state
/// (`CALL-071`). The script reads, and writes nothing (`CALL-072`).
///
/// # Errors
/// A store error.
pub async fn call_roster(
    redis: &Client,
    call: &Id<ambre_core::id::Call>,
    channel: &Id<Channel>,
) -> Result<ambre_protocol::signaling::CallRoster, AppError> {
    use ambre_protocol::signaling::{CallRoster, Seated};
    // HKEYS skips a seat whose time to live ran out. So a crashed seat drops out before the
    // heartbeat of another finds it gone, and a call with no seat left reads as gone.
    const LUA: &str = r#"
if redis.call('HGET', KEYS[1], 'channel') ~= ARGV[1] then return {'gone'} end
local seats = redis.call('HKEYS', KEYS[2])
if #seats == 0 then return {'gone'} end
local answer = {'live'}
for _, sender in ipairs(seats) do
    local join = redis.call('HGET', KEYS[3], sender)
    if join then
        table.insert(answer, sender)
        table.insert(answer, join)
    end
end
return answer
"#;
    let keys = crate::redis_keys::CallStateKeys::of(call);
    let read: Vec<String> = redis
        .eval(LUA, keys.texts(), [channel.to_string()])
        .await
        .map_err(|e| AppError::Redis(crate::error::FaultText::of("call roster", e)))?;
    let Some((word, seats)) = read.split_first() else {
        return Ok(CallRoster::Gone);
    };
    if word != "live" {
        return Ok(CallRoster::Gone);
    }
    let mut seated: Vec<Seated> = seats
        .as_chunks::<2>()
        .0
        .iter()
        .filter_map(|[sender, join]| {
            Some(Seated {
                sender: ambre_core::Seq::new(sender.parse().ok()?),
                join: ambre_core::Payload::new(hex::decode(join).ok()?),
            })
        })
        .collect();
    seated.sort_by_key(|seated| seated.sender);
    Ok(CallRoster::Live(seated))
}

/// Stores an opaque presence blob in Redis and sets a TTL.
///
/// The blob is the encrypted ephemeral ciphertext -- the server never
/// sees the plaintext cursor position or display name. A document being
/// deleted takes no presence (`SRV-017`).
pub async fn doc_presence_update(
    redis: &Client,
    document_id: &Id<Document>,
    pub_key: &FixedBytes<PublicKey>,
    presence: &ambre_core::Payload<ambre_protocol::messages::Ciphertext>,
    ttl: TimeSpan<Sec>,
) -> Result<(), AppError> {
    let pub_key_hex = pub_key.to_hex();
    use base64::Engine;
    let presence_b64 = base64::engine::general_purpose::URL_SAFE.encode(presence.as_bytes());
    // Expire THIS member's field individually (Redis 7.4+ `HEXPIRE`, `SRV-241`),
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
    let key = DocKey::<Presence>::of(document_id);
    let gone_key = DocKey::<Gone>::of(document_id);
    let owed_key = DocKey::<RotationOwed>::of(document_id);
    let ttl = ttl.get().to_string();
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
    document_id: &Id<Document>,
    pub_key: &FixedBytes<PublicKey>,
) -> Result<(), AppError> {
    let key = DocKey::<Presence>::of(document_id);
    redis
        .hdel(key.as_str(), pub_key.to_hex().as_str())
        .await
        .map_err(|e| AppError::Redis(crate::error::FaultText::of("presence remove", e)))?;
    Ok(())
}

/// How long a session count lives after its last open. One user signs every
/// session with the same key, so the count is what tells their sessions apart.
/// An instance that dies closes none of its sessions; the expiry bounds the
/// count that death leaves behind (`PRESENCE-015`).
const SESSION_COUNT_TTL: TimeSpan<Sec> = TimeSpan::<Sec>::new(24 * 3600);

/// Counts one more session of `pub_key_hex` watching the document, on every
/// instance (`1-Foundations/ambre-spec-presence.md`, `PRESENCE-015`). A
/// document being deleted takes no count (`SRV-017`).
pub async fn doc_session_open(
    redis: &Client,
    document_id: &Id<Document>,
    pub_key: &FixedBytes<PublicKey>,
) -> Result<(), AppError> {
    let pub_key_hex = pub_key.to_hex();
    const LUA: &str = r#"
if redis.call('EXISTS', KEYS[2]) == 1 then
    return 0
end
redis.call('HINCRBY', KEYS[1], ARGV[2], 1)
redis.call('HEXPIRE', KEYS[1], ARGV[1], 'FIELDS', 1, ARGV[2])
return 1
"#;
    let key = DocKey::<Sessions>::of(document_id);
    let gone_key = DocKey::<Gone>::of(document_id);
    let ttl = SESSION_COUNT_TTL.get().to_string();
    let _: i64 = redis
        .eval(
            LUA,
            [key.as_str(), gone_key.as_str()],
            [ttl.as_str(), pub_key_hex.as_str()],
        )
        .await
        .map_err(|e| AppError::Redis(crate::error::FaultText::of("session open", e)))?;
    Ok(())
}

/// Counts one session of `pub_key_hex` gone from the document. Answers whether
/// it was the last, which is when the author has left (`PRESENCE-015`).
///
/// A count that is missing or already spent answers `true`: a departure the
/// relay cannot count against an open session is a departure.
pub async fn doc_session_close(
    redis: &Client,
    document_id: &Id<Document>,
    pub_key: &FixedBytes<PublicKey>,
) -> Result<bool, AppError> {
    let pub_key_hex = pub_key.to_hex();
    const LUA: &str = r#"
local left = redis.call('HINCRBY', KEYS[1], ARGV[1], -1)
if left <= 0 then
    redis.call('HDEL', KEYS[1], ARGV[1])
    return 1
end
return 0
"#;
    let key = DocKey::<Sessions>::of(document_id);
    let last: i64 = redis
        .eval(LUA, [key.as_str()], [pub_key_hex.as_str()])
        .await
        .map_err(|e| AppError::Redis(crate::error::FaultText::of("session close", e)))?;
    Ok(last == 1)
}

/// Sets the tombstone of a document for `ttl`: the first step of a
/// delete, before any object or key is removed.
pub async fn mark_document_gone(
    redis: &Client,
    document_id: &Id<Document>,
    ttl: TimeSpan<Sec>,
) -> Result<(), AppError> {
    const LUA: &str = "return redis.call('SET', KEYS[1], '1', 'EX', ARGV[1])";
    let gone_key = DocKey::<Gone>::of(document_id);
    let ttl = ttl.get().to_string();
    let _: String = redis
        .eval::<String>(LUA, [gone_key.as_str()], [ttl.as_str()])
        .await
        .map_err(|e| AppError::Redis(crate::error::FaultText::of("mark document gone", e)))?;
    Ok(())
}

/// Removes every key the relay holds for a deleted document (`SRV-017`):
///
/// * the sequence counter and the per-author clock hash;
/// * the pending list and the dead-letter list;
/// * the presence hash and the session counts;
/// * the mirror of an owed rotation and the segment lock.
///
/// One script, so no reader sees a half-purged document.
///
/// The same script renews the tombstone for `ttl`. The second sweep of
/// the objects is scheduled in the register, which a reset of this store does
/// not lose (`SRV-057`, `SRV-165`).
///
/// The dirty mark of the pending list sits in another slot, so a command of
/// its own drops it after the script (`SRV-253`). A mark that outlives the
/// list costs one empty visit of the flusher, which then drops it.
pub async fn purge_document_state(
    redis: &Client,
    document_id: &Id<Document>,
    ttl: TimeSpan<Sec>,
) -> Result<(), AppError> {
    use rustis::commands::SetCommands;
    const LUA: &str = r#"
redis.call('DEL', KEYS[1], KEYS[2], KEYS[3], KEYS[4], KEYS[5], KEYS[6], KEYS[7], KEYS[8])
redis.call('SET', KEYS[9], '1', 'EX', ARGV[1])
return 1
"#;
    let keys = DocumentStateKeys::of(document_id);
    let gone_key = DocKey::<Gone>::of(document_id);
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
                gone_key.as_str(),
            ],
            [ttl.get().to_string()],
        )
        .await
        .map_err(|e| AppError::Redis(crate::error::FaultText::of("purge document state", e)))?;
    let _: usize = redis
        .srem(FlushDirtySet.as_str(), [keys.pending.as_str()])
        .await
        .map_err(|e| AppError::Redis(crate::error::FaultText::of("purge dirty mark", e)))?;
    Ok(())
}

/// Lock a segment against deltas, unless one still waits to be flushed.
/// Returns the last delta number the relay gave it, the horizon of its final
/// snapshot, or `None` while a delta is still pending. One script, so no
/// delta is numbered between the check and the lock.
pub async fn lock_segment(
    redis: &Client,
    document_id: &Id<Document>,
) -> Result<Option<ambre_core::Seq<ambre_core::Relay>>, AppError> {
    const LUA: &str = r#"
if redis.call('LLEN', KEYS[1]) > 0 then
    return -1
end
redis.call('SET', KEYS[2], '1')
return tonumber(redis.call('GET', KEYS[3])) or 0
"#;
    let pending = PendingKey::of(document_id);
    let locked = DocKey::<Locked>::of(document_id);
    let seq = DocKey::<SeqCounter>::of(document_id);
    let answer: i64 = redis
        .eval::<i64>(
            LUA,
            [pending.as_str(), locked.as_str(), seq.as_str()],
            Vec::<String>::new(),
        )
        .await
        .map_err(|e| AppError::Redis(crate::error::FaultText::of("lock segment", e)))?;
    Ok(ambre_core::Seq::<ambre_core::Relay>::try_from(answer).ok())
}

/// Lock a segment against deltas at once, a delta still unflushed included,
/// and answer the last delta number it gave. A rotation cannot wait for a
/// flush, and a delta numbered before it stays under the lock (`SRV-147`).
pub async fn lock_segment_now(
    redis: &Client,
    document_id: &Id<Document>,
) -> Result<ambre_core::Seq<ambre_core::Relay>, AppError> {
    const LUA: &str = r#"
redis.call('SET', KEYS[1], '1')
return tonumber(redis.call('GET', KEYS[2])) or 0
"#;
    let locked = DocKey::<Locked>::of(document_id);
    let seq = DocKey::<SeqCounter>::of(document_id);
    let answer: i64 = redis
        .eval::<i64>(LUA, [locked.as_str(), seq.as_str()], Vec::<String>::new())
        .await
        .map_err(|e| AppError::Redis(crate::error::FaultText::of("lock segment now", e)))?;
    Ok(ambre_core::Seq::<ambre_core::Relay>::new(
        answer.unsigned_abs(),
    ))
}

/// Seal an amendments document a fold holds up to its delta `folded_seq`
/// (`SRV-211`). The seal is the segment lock, so the clock script refuses
/// every later delta. Answers `false`, and seals nothing, while a delta waits
/// to be flushed or when the document took a delta past the fold. One script,
/// so no delta is numbered between the check and the seal.
pub async fn seal_folded(
    redis: &Client,
    document_id: &Id<Document>,
    folded_seq: ambre_core::Seq<ambre_core::Relay>,
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
    let locked = DocKey::<Locked>::of(document_id);
    let seq = DocKey::<SeqCounter>::of(document_id);
    let sealed: i64 = redis
        .eval::<i64>(
            LUA,
            [pending.as_str(), locked.as_str(), seq.as_str()],
            [folded_seq.get().to_string()],
        )
        .await
        .map_err(|e| AppError::Redis(crate::error::FaultText::of("seal folded", e)))?;
    Ok(sealed == 1)
}

/// Sets the mirror of a segment's lock. Nothing clears it but the purge of a
/// deleted document: a lock holds for good.
pub async fn mirror_segment_lock(
    redis: &Client,
    document_id: &Id<Document>,
) -> Result<(), AppError> {
    redis
        .set(DocKey::<Locked>::of(document_id).as_str(), "1")
        .await
        .map_err(|e| AppError::Redis(crate::error::FaultText::of("mirror segment lock", e)))
}

/// Whether a document owes a key rotation, as its mirror holds it
/// (`SRV-123`): the flag the members answer carries too.
pub use ambre_core::RotationDebt;
use ambre_core::{Sec, TimeSpan};

/// Sets or clears the mirror of an owed rotation.
pub async fn mirror_rotation_owed(
    redis: &Client,
    document_id: &Id<Document>,
    debt: RotationDebt,
) -> Result<(), AppError> {
    const LUA: &str = r#"
if ARGV[1] == '1' then
    redis.call('SET', KEYS[1], '1')
    return 1
end
return redis.call('DEL', KEYS[1])
"#;
    let key = DocKey::<RotationOwed>::of(document_id);
    let flag = match debt {
        RotationDebt::Owed => "1",
        RotationDebt::Settled => "0",
    };
    let _: i64 = redis
        .eval::<i64>(LUA, [key.as_str()], [flag])
        .await
        .map_err(|e| AppError::Redis(crate::error::FaultText::of("mirror rotation owed", e)))?;
    Ok(())
}

/// Identity bound to a refresh token in Redis.
///
/// Kept as a simple struct so callers do not need to manually parse the flat
/// HGETALL response. Mirrors what a minted access JWT carries in its claims.
#[derive(Debug, Clone)]
pub struct RefreshData {
    pub user_id: Id<User>,
    /// `None` for an account that has not registered its keys (`SRV-027`).
    pub signing_pubkey: Option<FixedBytes<PublicKey>>,
}

/// Stores OIDC authorization state in Redis with a 10-minute TTL.
///
/// The state is keyed by the CSRF token secret so it can be retrieved
/// during the callback to validate the PKCE verifier and nonce.
pub async fn oidc_state_set(
    redis: &Client,
    csrf_state: &ambre_core::Credential<ambre_core::OidcState>,
    auth_state: &OidcAuthState,
) -> Result<(), AppError> {
    // The script takes its copy of the key and of the verifier before it is
    // sent. The record and its lifetime are one script, as for a refresh
    // record (`SRV-291`).
    crate::secrets::token::oidc_state_key(csrf_state, |key| {
        auth_state.pkce_verifier_secret.present_stored(|verifier| {
            redis.eval::<i64>(
                HASH_WITH_TTL_LUA,
                [key],
                [
                    "600",
                    "pkce_verifier",
                    verifier,
                    "nonce",
                    auth_state.nonce.secret().as_str(),
                ],
            )
        })
    })
    .await
    .map_err(|e| AppError::Redis(crate::error::FaultText::of("oidc state set", e)))?;
    Ok(())
}

/// Retrieves and deletes OIDC authorization state from Redis.
///
/// Returns the PKCE verifier and the nonce on success, or an error if the
/// state expired or was not found.
pub async fn oidc_state_take(
    redis: &Client,
    csrf_state: &ambre_core::Credential<ambre_core::OidcState>,
) -> Result<OidcAuthState, AppError> {
    // Read into a `HashMap`, NOT a `Vec<String>`. Under RESP3 rustis returns a hash
    // reply as a map, and deserializing that into a flat `Vec<String>` fails with
    // "invalid type: sequence, expected a string". That surfaced here as a spurious
    // `SessionExpired` on EVERY OIDC callback (login fully broken). Mirror
    // `refresh_get`, which reads the same shape correctly.
    // Both commands take their copy of the key before the read is sent.
    let (get, delete) = crate::secrets::token::oidc_state_key(csrf_state, |key| {
        (redis.hgetall(key), redis.del([key]))
    });
    let fields: std::collections::HashMap<String, String> = get
        .await
        .map_err(|e| AppError::Redis(crate::error::FaultText::of("oidc state get", e)))?;

    if fields.is_empty() {
        return Err(AppError::SessionExpired);
    }

    let verifier = fields.get("pkce_verifier").cloned().unwrap_or_default();
    let nonce = fields.get("nonce").cloned().unwrap_or_default();

    delete
        .await
        .map_err(|e| AppError::Redis(crate::error::FaultText::of("oidc state delete", e)))?;

    Ok(OidcAuthState {
        pkce_verifier_secret: ambre_core::Confidential::<OidcSecret>::new(verifier),
        nonce: openidconnect::Nonce::new(nonce),
    })
}

/// A Redis server version, read from the `redis_version` line of `INFO server`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct StoreVersion {
    pub major: u16,
    pub minor: u16,
}

/// The oldest Redis the relay runs on. The presence and session scripts call
/// `HEXPIRE`, which Redis 7.4 introduced (`SRV-241`).
pub const MIN_STORE_VERSION: StoreVersion = StoreVersion { major: 7, minor: 4 };

/// Reads the server version from an `INFO server` reply.
///
/// Answers `None` when the reply has no `redis_version` line, or when the line
/// does not start with `major.minor`.
pub fn parse_store_version(info: &str) -> Option<StoreVersion> {
    let version = info
        .lines()
        .find_map(|line| line.trim().strip_prefix("redis_version:"))?;
    let mut parts = version.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    Some(StoreVersion { major, minor })
}

/// Refuses an `INFO server` reply that names no version, or a version older
/// than [`MIN_STORE_VERSION`].
fn check_store_version(info: &str) -> Result<StoreVersion, AppError> {
    let version = parse_store_version(info).ok_or_else(|| {
        AppError::Redis(crate::error::FaultText::bare(
            "INFO server names no redis_version",
        ))
    })?;
    if version < MIN_STORE_VERSION {
        return Err(AppError::Redis(crate::error::FaultText::of(
            "the store is too old for HEXPIRE",
            format_args!(
                "Redis {}.{}, and the relay needs {}.{} or later",
                version.major, version.minor, MIN_STORE_VERSION.major, MIN_STORE_VERSION.minor
            ),
        )));
    }
    Ok(version)
}

/// One connection to each primary node of the store. A command that reads one
/// node alone needs them: a sweep of the keyspace (`SRV-255`), or the version
/// of each server. A single server is its own one primary, and the call hands
/// back the same client.
///
/// On a cluster the call reads `CLUSTER NODES`. It opens a connection of its
/// own to each primary that serves a slot, with the credentials of `redis`.
/// The caller drops them once the sweep ends.
///
/// # Errors
///
/// `AppError::Redis` when the cluster does not answer, names no primary, or
/// one primary refuses the connection.
pub async fn primaries(redis: &Client) -> Result<Vec<Client>, AppError> {
    use rustis::client::ServerConfig;
    use rustis::commands::ClusterCommands;
    if !matches!(redis.config().server, ServerConfig::Cluster(_)) {
        return Ok(vec![redis.clone()]);
    }
    let nodes: String = redis
        .cluster_nodes()
        .await
        .map_err(|e| AppError::Redis(crate::error::FaultText::of("CLUSTER NODES", e)))?;
    let servers = primary_servers(&nodes);
    if servers.is_empty() {
        return Err(AppError::Redis(crate::error::FaultText::bare(
            "CLUSTER NODES names no primary",
        )));
    }
    let mut clients = Vec::with_capacity(servers.len());
    for server in servers {
        let mut config = redis.config().clone();
        config.server = server;
        clients.push(Client::connect(config).await.map_err(|e| {
            AppError::Redis(crate::error::FaultText::of("connect to a primary", e))
        })?);
    }
    Ok(clients)
}

/// The primaries a `CLUSTER NODES` reply names: each node flagged `master`
/// that serves at least one slot, and is neither failed nor without an
/// address. A replica, and a primary that serves no slot, hold no key a sweep
/// owes a read.
fn primary_servers(nodes: &str) -> Vec<rustis::client::ServerConfig> {
    nodes
        .lines()
        .filter_map(|line| {
            let fields: Vec<&str> = line.split_whitespace().collect();
            let (address, flags) = (fields.get(1)?, fields.get(2)?);
            let flags: Vec<&str> = flags.split(',').collect();
            let primary = flags.contains(&"master")
                && !flags
                    .iter()
                    .any(|flag| matches!(*flag, "fail" | "fail?" | "noaddr" | "handshake"));
            // The ninth field and on are the slots the node serves.
            if !primary || fields.len() <= 8 {
                return None;
            }
            // `ip:port@cport`, then an optional `,hostname`. An IPv6 address
            // holds colons of its own, so the port is the part after the last.
            let endpoint = address.split(['@', ',']).next()?;
            let (host, port) = endpoint.rsplit_once(':')?;
            Some(rustis::client::ServerConfig::Standalone {
                host: host
                    .trim_start_matches('[')
                    .trim_end_matches(']')
                    .to_owned(),
                port: port.parse().ok()?,
            })
        })
        .collect()
}

/// Refuses to boot on a Redis older than [`MIN_STORE_VERSION`] (`SRV-241`).
///
/// An older server accepts the connection and every other script. The failure
/// would then surface only at the first presence sample, as an unknown command.
///
/// # Errors
///
/// `AppError::Redis` when `INFO` fails, names no version, or names an older one.
///
/// A cluster answers for each primary, and the oldest one decides.
pub async fn require_store_version(redis: &Client) -> Result<StoreVersion, AppError> {
    let mut oldest = None;
    for node in primaries(redis).await? {
        let info: String = node
            .info([InfoSection::Server])
            .await
            .map_err(|e| AppError::Redis(crate::error::FaultText::of("INFO server", e)))?;
        let version = check_store_version(&info)?;
        oldest = Some(oldest.map_or(version, |o: StoreVersion| o.min(version)));
    }
    oldest.ok_or_else(|| AppError::Redis(crate::error::FaultText::bare("no primary answered INFO")))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `SRV-255`: the sweep reads each primary that serves a slot, at the
    /// address before the cluster bus port. A replica, a failed primary and a
    /// primary with no slot hold nothing a sweep owes a read.
    #[test]
    fn the_primaries_are_the_masters_that_serve_a_slot() {
        let nodes = "\
a1 127.0.0.1:7001@17001 myself,master - 0 0 1 connected 0-5460
a2 127.0.0.1:7002@17002,node-2 master - 0 0 2 connected 5461-10922
a3 ::1:7003@17003 master - 0 0 3 connected 10923-16383
r1 127.0.0.1:7004@17004 slave a1 0 0 1 connected
f1 127.0.0.1:7005@17005 master,fail - 0 0 4 disconnected 100
e1 127.0.0.1:7006@17006 master - 0 0 5 connected
";
        let servers: Vec<String> = primary_servers(nodes)
            .iter()
            .map(|server| match server {
                rustis::client::ServerConfig::Standalone { host, port } => format!("{host} {port}"),
                _ => "other".to_owned(),
            })
            .collect();
        assert_eq!(servers, ["127.0.0.1 7001", "127.0.0.1 7002", "::1 7003"]);
        assert!(primary_servers("").is_empty());
    }

    #[test]
    fn a_store_older_than_the_minimum_is_refused() {
        let info = |v: &str| format!("# Server\r\nredis_version:{v}\r\nredis_mode:standalone\r\n");
        assert_eq!(
            check_store_version(&info("7.4.0")).ok(),
            Some(StoreVersion { major: 7, minor: 4 })
        );
        assert_eq!(
            check_store_version(&info("8.2.1")).ok(),
            Some(StoreVersion { major: 8, minor: 2 })
        );
        assert!(matches!(
            check_store_version(&info("7.2.5")),
            Err(AppError::Redis(_))
        ));
        assert!(matches!(
            check_store_version(&info("6.10.0")),
            Err(AppError::Redis(_))
        ));
        assert!(matches!(
            check_store_version("# Server\r\nredis_mode:standalone\r\n"),
            Err(AppError::Redis(_))
        ));
        assert!(matches!(
            check_store_version(&info("unstable")),
            Err(AppError::Redis(_))
        ));
    }

    /// A presence sample: its bytes stand for an encrypted cursor.
    pub(super) fn sample() -> ambre_core::Payload<ambre_protocol::messages::Ciphertext> {
        ambre_core::Payload::<ambre_protocol::messages::Ciphertext>::new(b"blob".to_vec())
    }

    #[test]
    fn the_script_refusal_becomes_the_typed_conflict() {
        let err = clock_refusal(
            "ERR Error running script: CLOCK_MISMATCH:4",
            Seq::<DeltaAuthor>::new(7),
        );
        assert!(
            matches!(
                err,
                AppError::ClockMismatch {
                    expected,
                    got
                } if expected.get() == 5 && got.get() == 7
            ),
            "{err:?}"
        );
        let err = clock_refusal("connection reset by peer", Seq::<DeltaAuthor>::new(7));
        assert!(matches!(err, AppError::Redis(_)), "{err:?}");
    }
}

#[cfg(test)]
#[cfg(feature = "test-helpers")]
mod lua_tests {
    use super::*;
    use ambre_core::{Sec, TimeSpan};

    async fn connect_redis() -> rustis::client::Client {
        // These tests run only under `--features db`, where the backing Redis is
        // expected up (`just services-up`). A missing connection is a broken test
        // environment, not a reason to silently pass, so fail loudly.
        crate::test_helpers::connect_test_redis()
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
        let doc_id = Id::<Document>::mint();
        let pk_hex = &FixedBytes::<PublicKey>::from_array([0xa1; 32]);

        mirror_rotation_owed(&redis, &doc_id, RotationDebt::Owed)
            .await
            .unwrap();
        let refused =
            verify_clock_and_assign_seq(&redis, &doc_id, pk_hex, Seq::<DeltaAuthor>::new(1)).await;
        assert!(
            matches!(refused, Err(AppError::RotationOwed)),
            "{refused:?}"
        );
        let presence = doc_presence_update(
            &redis,
            &doc_id,
            pk_hex,
            &super::tests::sample(),
            TimeSpan::<Sec>::new(15),
        )
        .await;
        assert!(
            matches!(presence, Err(AppError::RotationOwed)),
            "{presence:?}"
        );
        let seq: usize = redis.exists(format!("doc:{{{doc_id}}}:seq")).await.unwrap();
        assert_eq!(seq, 0, "a refused frame takes no sequence number");

        mirror_rotation_owed(&redis, &doc_id, RotationDebt::Settled)
            .await
            .unwrap();
        let seq = verify_clock_and_assign_seq(&redis, &doc_id, pk_hex, Seq::<DeltaAuthor>::new(1))
            .await
            .expect("the same clock is accepted once the debt clears");
        assert_eq!(seq, ambre_core::Seq::<ambre_core::Relay>::new(1));

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
        let doc_id = Id::<Document>::mint();
        let pk_hex = &FixedBytes::<PublicKey>::from_array([0xb1; 32]);
        let pending = format!("doc:{{{doc_id}}}:pending");

        for clock in 1..=2 {
            verify_clock_and_assign_seq(&redis, &doc_id, pk_hex, Seq::<DeltaAuthor>::new(clock))
                .await
                .expect("an open segment takes deltas");
        }
        let _: usize = redis.rpush(&pending, "entry").await.unwrap();
        assert_eq!(lock_segment(&redis, &doc_id).await.unwrap(), None);
        let locked: usize = redis
            .exists(DocKey::<Locked>::of(&doc_id).as_str())
            .await
            .unwrap();
        assert_eq!(locked, 0, "a pending delta holds the lock back");

        let _: usize = redis.del(&pending).await.unwrap();
        assert_eq!(
            lock_segment(&redis, &doc_id).await.unwrap(),
            Some(ambre_core::Seq::<ambre_core::Relay>::new(2))
        );
        let refused =
            verify_clock_and_assign_seq(&redis, &doc_id, pk_hex, Seq::<DeltaAuthor>::new(3)).await;
        assert!(
            matches!(refused, Err(AppError::SegmentLocked)),
            "{refused:?}"
        );
        let seq: Option<u64> = redis.get(format!("doc:{{{doc_id}}}:seq")).await.unwrap();
        assert_eq!(seq, Some(2), "a refused delta takes no sequence number");
        let clock: Option<u64> = redis
            .hget(DocKey::<Clocks>::of(&doc_id).as_str(), pk_hex.to_hex())
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
        let doc_id = Id::<Document>::mint();
        let pk_hex = &FixedBytes::<PublicKey>::from_array([0xd1; 32]);
        assert_eq!(
            lock_segment_now(&redis, &doc_id).await.unwrap(),
            ambre_core::Seq::<ambre_core::Relay>::new(0)
        );
        let _ = redis
            .del(DocumentStateKeys::of(&doc_id).texts())
            .await
            .unwrap();

        for clock in 1..=2 {
            verify_clock_and_assign_seq(&redis, &doc_id, pk_hex, Seq::<DeltaAuthor>::new(clock))
                .await
                .expect("an open segment takes deltas");
        }
        let _: usize = redis
            .rpush(format!("doc:{{{doc_id}}}:pending"), "entry")
            .await
            .unwrap();
        assert_eq!(
            lock_segment_now(&redis, &doc_id).await.unwrap(),
            ambre_core::Seq::<ambre_core::Relay>::new(2)
        );
        let refused =
            verify_clock_and_assign_seq(&redis, &doc_id, pk_hex, Seq::<DeltaAuthor>::new(3)).await;
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
        let doc_id = Id::<Document>::mint();
        let pk_hex = &FixedBytes::<PublicKey>::from_array([0xc1; 32]);
        let pending = format!("doc:{{{doc_id}}}:pending");

        for clock in 1..=2 {
            verify_clock_and_assign_seq(&redis, &doc_id, pk_hex, Seq::<DeltaAuthor>::new(clock))
                .await
                .expect("amendments take deltas");
        }
        assert!(
            !seal_folded(
                &redis,
                &doc_id,
                ambre_core::Seq::<ambre_core::Relay>::new(1)
            )
            .await
            .unwrap(),
            "a delta came past the fold"
        );
        let _: usize = redis.rpush(&pending, "entry").await.unwrap();
        assert!(
            !seal_folded(
                &redis,
                &doc_id,
                ambre_core::Seq::<ambre_core::Relay>::new(2)
            )
            .await
            .unwrap(),
            "a delta waits to be flushed"
        );
        let locked: usize = redis
            .exists(DocKey::<Locked>::of(&doc_id).as_str())
            .await
            .unwrap();
        assert_eq!(locked, 0, "a refused seal locks nothing");

        let _: usize = redis.del(&pending).await.unwrap();
        assert!(
            seal_folded(
                &redis,
                &doc_id,
                ambre_core::Seq::<ambre_core::Relay>::new(2)
            )
            .await
            .unwrap()
        );
        let refused =
            verify_clock_and_assign_seq(&redis, &doc_id, pk_hex, Seq::<DeltaAuthor>::new(3)).await;
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
        let doc_id = Id::<Document>::mint();
        let pk_hex = &FixedBytes::<PublicKey>::from_array([0xab; 32]);

        let seq = verify_clock_and_assign_seq(&redis, &doc_id, pk_hex, Seq::<DeltaAuthor>::new(1))
            .await
            .expect("first update should succeed");
        assert_eq!(seq, ambre_core::Seq::<ambre_core::Relay>::new(1));

        let _ = redis
            .del([
                DocKey::<Clocks>::of(&doc_id).as_str(),
                DocKey::<SeqCounter>::of(&doc_id).as_str(),
            ])
            .await
            .unwrap();
    }

    #[tokio::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs redis; run with --features db")]
    async fn sequential_clock_succeeds() {
        let redis = connect_redis().await;
        let doc_id = Id::<Document>::mint();
        let pk = &FixedBytes::<PublicKey>::from_array([0xde; 32]);

        let s1 = verify_clock_and_assign_seq(&redis, &doc_id, pk, Seq::<DeltaAuthor>::new(1))
            .await
            .unwrap();
        let s2 = verify_clock_and_assign_seq(&redis, &doc_id, pk, Seq::<DeltaAuthor>::new(2))
            .await
            .unwrap();
        let s3 = verify_clock_and_assign_seq(&redis, &doc_id, pk, Seq::<DeltaAuthor>::new(3))
            .await
            .unwrap();
        assert_eq!(s1, ambre_core::Seq::<ambre_core::Relay>::new(1));
        assert_eq!(s2, ambre_core::Seq::<ambre_core::Relay>::new(2));
        assert_eq!(s3, ambre_core::Seq::<ambre_core::Relay>::new(3));

        let _ = redis
            .del([
                DocKey::<Clocks>::of(&doc_id).as_str(),
                DocKey::<SeqCounter>::of(&doc_id).as_str(),
            ])
            .await
            .unwrap();
    }

    #[tokio::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs redis; run with --features db")]
    async fn clock_mismatch_rejected() {
        let redis = connect_redis().await;
        let doc_id = Id::<Document>::mint();
        let pk = &FixedBytes::<PublicKey>::from_array([0x11; 32]);

        verify_clock_and_assign_seq(&redis, &doc_id, pk, Seq::<DeltaAuthor>::new(1))
            .await
            .unwrap();
        let err = verify_clock_and_assign_seq(&redis, &doc_id, pk, Seq::<DeltaAuthor>::new(3))
            .await
            .unwrap_err();
        assert!(
            matches!(
                err,
                AppError::ClockMismatch {
                    expected,
                    got
                } if expected.get() == 2 && got.get() == 3
            ),
            "{err:?}"
        );

        let _ = redis
            .del([
                DocKey::<Clocks>::of(&doc_id).as_str(),
                DocKey::<SeqCounter>::of(&doc_id).as_str(),
            ])
            .await
            .unwrap();
    }

    #[tokio::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs redis; run with --features db")]
    async fn duplicate_clock_rejected() {
        let redis = connect_redis().await;
        let doc_id = Id::<Document>::mint();
        let pk = &FixedBytes::<PublicKey>::from_array([0x22; 32]);

        verify_clock_and_assign_seq(&redis, &doc_id, pk, Seq::<DeltaAuthor>::new(1))
            .await
            .unwrap();
        let err = verify_clock_and_assign_seq(&redis, &doc_id, pk, Seq::<DeltaAuthor>::new(1))
            .await
            .unwrap_err();
        assert!(
            matches!(
                err,
                AppError::ClockMismatch {
                    expected,
                    got
                } if expected.get() == 2 && got.get() == 1
            ),
            "{err:?}"
        );

        let _ = redis
            .del([
                DocKey::<Clocks>::of(&doc_id).as_str(),
                DocKey::<SeqCounter>::of(&doc_id).as_str(),
            ])
            .await
            .unwrap();
    }

    #[tokio::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs redis; run with --features db")]
    async fn independent_clients_have_independent_clocks() {
        let redis = connect_redis().await;
        let doc_id = Id::<Document>::mint();
        let pk_a = &FixedBytes::<PublicKey>::from_array([0xaa; 32]);
        let pk_b = &FixedBytes::<PublicKey>::from_array([0xbb; 32]);

        let s_a = verify_clock_and_assign_seq(&redis, &doc_id, pk_a, Seq::<DeltaAuthor>::new(1))
            .await
            .unwrap();
        let s_b = verify_clock_and_assign_seq(&redis, &doc_id, pk_b, Seq::<DeltaAuthor>::new(1))
            .await
            .unwrap();
        let s_a2 = verify_clock_and_assign_seq(&redis, &doc_id, pk_a, Seq::<DeltaAuthor>::new(2))
            .await
            .unwrap();

        assert_eq!(s_a, ambre_core::Seq::<ambre_core::Relay>::new(1));
        assert_eq!(s_b, ambre_core::Seq::<ambre_core::Relay>::new(2));
        assert_eq!(s_a2, ambre_core::Seq::<ambre_core::Relay>::new(3));

        let _ = redis
            .del([
                DocKey::<Clocks>::of(&doc_id).as_str(),
                DocKey::<SeqCounter>::of(&doc_id).as_str(),
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
    use ambre_core::{Sec, TimeSpan};

    async fn connect_redis() -> rustis::client::Client {
        // These tests run only under `--features db`, where the backing Redis is
        // expected up (`just services-up`). A missing connection is a broken test
        // environment, not a reason to silently pass, so fail loudly.
        crate::test_helpers::connect_test_redis()
            .await
            .expect("db-tier tests require a reachable Redis (REDIS_URL / just services-up)")
    }

    #[tokio::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs redis; run with --features db")]
    async fn flush_lock_is_mutually_exclusive() {
        let redis = connect_redis().await;
        let lock_key = DocKey::<FlushLock>::of(&Id::<Document>::mint());
        let _ = redis.del([lock_key.as_str()]).await;

        // First caller acquires; a concurrent instance cannot while it is held.
        assert!(
            try_acquire_flush_lock(
                &redis,
                &lock_key,
                &LockToken("tokA".to_owned()),
                TimeSpan::<Sec>::new(60)
            )
            .await
            .unwrap()
        );
        assert!(
            !try_acquire_flush_lock(
                &redis,
                &lock_key,
                &LockToken("tokB".to_owned()),
                TimeSpan::<Sec>::new(60)
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
                TimeSpan::<Sec>::new(60)
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
                TimeSpan::<Sec>::new(60)
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
        let doc = Id::<Document>::mint();
        let ab = FixedBytes::<PublicKey>::from_array([0xab; 32]);
        let mut clients = Vec::new();
        for _ in 0..16 {
            clients.push(connect_redis().await);
        }
        let calls = clients
            .iter()
            .map(|redis| verify_clock_and_assign_seq(redis, &doc, &ab, Seq::<DeltaAuthor>::new(1)));
        let results = futures_util::future::join_all(calls).await;

        let numbered: Vec<ambre_core::Seq<ambre_core::Relay>> = results
            .iter()
            .filter_map(|r| r.as_ref().ok())
            .copied()
            .collect();
        assert_eq!(
            numbered,
            [ambre_core::Seq::<ambre_core::Relay>::new(1)],
            "results {results:?}"
        );
        let refused = results
            .iter()
            .filter(|r| {
                matches!(
                    r,
                    Err(AppError::ClockMismatch {
                        expected,
                        got
                    }) if expected.get() == 2 && got.get() == 1
                )
            })
            .count();
        assert_eq!(refused, 15);
        let redis = connect_redis().await;
        let seq: u64 = redis.get(format!("doc:{{{doc}}}:seq")).await.unwrap();
        assert_eq!(seq, 1);
        assert_eq!(
            verify_clock_and_assign_seq(&redis, &doc, &ab, Seq::<DeltaAuthor>::new(2))
                .await
                .unwrap(),
            ambre_core::Seq::<ambre_core::Relay>::new(2)
        );
        let _ = redis
            .del([
                format!("doc:{{{doc}}}:seq"),
                format!("doc:{{{doc}}}:clocks"),
            ])
            .await;
    }

    #[tokio::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs redis; run with --features db")]
    async fn dirty_set_marks_on_append_and_clears_only_when_drained() {
        use rustis::commands::SetCommands;
        let redis = connect_redis().await;
        let doc = Id::<Document>::mint();
        let pending_key = PendingKey::of(&doc);
        let _ = redis.del([pending_key.as_str()]).await;
        // Start from a clean marker for this doc's key.
        let _: Result<usize, _> = redis
            .srem(FlushDirtySet.as_str(), [pending_key.as_str()])
            .await;

        // Appending a delta marks the pending list dirty.
        let queued = append_pending_delta(
            &redis,
            &doc,
            ambre_core::Seq::<ambre_core::Relay>::new(1),
            &ambre_core::ObjectKey::from("s3-x"),
            &StoredPayload::new(vec![1, 2, 3]),
        )
        .await
        .unwrap();
        assert!(matches!(queued, Queued::Marked), "the store takes the mark");
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

    /// `SRV-262`: a dirty set that refuses the mark after the push leaves the
    /// delta in its list, and the store answers the missed mark typed.
    #[tokio::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs redis; run with --features db")]
    async fn a_refused_mark_keeps_the_delta_and_answers_the_miss() {
        let redis = crate::test_helpers::connect_document_keys_redis()
            .await
            .expect("db-tier tests require a reachable Redis");
        let doc = Id::<Document>::mint();

        let queued = append_pending_delta(
            &redis,
            &doc,
            ambre_core::Seq::<ambre_core::Relay>::new(1),
            &ambre_core::ObjectKey::from("s3-x"),
            &StoredPayload::new(vec![1, 2, 3]),
        )
        .await
        .expect("the push reaches a key of the document");

        assert!(
            matches!(queued, Queued::MarkMissed(AppError::Redis(_))),
            "{queued:?}"
        );
        let held = get_pending_deltas_map(&redis, &doc).await.unwrap();
        assert_eq!(held.len(), 1, "the list keeps the delta");
        let _ = redis.del([PendingKey::of(&doc).as_str()]).await;
    }

    /// `SRV-253`: three operations pair a list of one document with the dirty
    /// set. Each runs on a cluster, which refuses a script over two slots:
    ///
    /// * the append marks the list;
    /// * the clear keeps the mark of a list that holds an entry;
    /// * the purge drops the list and its mark.
    #[tokio::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs redis; run with --features db")]
    async fn the_dirty_set_and_a_document_never_share_a_script() {
        use rustis::commands::SetCommands;
        let redis = connect_redis().await;
        let marked = |key: PendingKey| {
            let redis = redis.clone();
            async move {
                redis
                    .sismember(FlushDirtySet.as_str(), key.as_str())
                    .await
                    .expect("sismember")
            }
        };
        let doc = Id::<Document>::mint();
        let pending = PendingKey::of(&doc);
        for seq in 1..=2 {
            let queued = append_pending_delta(
                &redis,
                &doc,
                ambre_core::Seq::<ambre_core::Relay>::new(seq),
                &ambre_core::ObjectKey::from("s3-x"),
                &StoredPayload::new(vec![1, 2, 3]),
            )
            .await
            .expect("an append runs on any store");
            assert!(matches!(queued, Queued::Marked), "the store takes the mark");
        }
        assert!(marked(pending.clone()).await, "the append marks the list");

        clear_pending_dirty_if_empty(&redis, &pending)
            .await
            .expect("a clear runs on any store");
        assert!(
            marked(pending.clone()).await,
            "a list with entries keeps its mark"
        );

        purge_document_state(&redis, &doc, TimeSpan::<Sec>::new(60))
            .await
            .expect("a purge runs on any store");
        assert!(!marked(pending.clone()).await, "the purge drops the mark");
        let length: usize = redis.llen(pending.as_str()).await.expect("llen");
        assert_eq!(length, 0, "the purge drops the list");
        let _ = redis.del([DocKey::<Gone>::of(&doc).as_str()]).await;
        let _: Result<usize, _> = redis.srem(FlushDirtySet.as_str(), [pending.as_str()]).await;
    }

    /// `SRV-254`: each broadcast of the relay reaches a sharded subscriber of
    /// its topic. A classic publish reaches none: a cluster does not copy it
    /// to the shard channels, and neither does one server.
    #[tokio::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs redis; run with --features db")]
    async fn each_broadcast_reaches_a_sharded_subscriber_of_its_topic() {
        use futures_util::StreamExt;
        let redis = connect_redis().await;
        let document = Id::<Document>::mint();
        let user = Id::<User>::mint();
        let channel = Id::<Channel>::mint();
        let call = Id::<ambre_core::id::Call>::mint();
        let listener = connect_redis().await;
        let topics = [
            crate::ws::Topic::document(&document),
            crate::ws::Topic::user(&user),
            crate::ws::Topic::channel(&channel),
            crate::ws::Topic::call(&call),
        ];
        let mut stream = listener
            .ssubscribe(topics[0].as_str())
            .await
            .expect("ssubscribe");
        for topic in &topics[1..] {
            stream.ssubscribe(topic.as_str()).await.expect("ssubscribe");
        }

        let frame = StoredPayload::new(b"frame".to_vec());
        doc_broadcast(&redis, &document, &frame).await.expect("doc");
        user_broadcast(&redis, &user, &frame).await.expect("user");
        thread_activity(&redis, &channel, &document)
            .await
            .expect("channel");
        call_signal(
            &redis,
            &call,
            &ambre_protocol::signaling::CallServerMsg::Lost,
        )
        .await
        .expect("call");

        let mut heard = std::collections::BTreeSet::new();
        while heard.len() < topics.len() {
            let Ok(Some(Ok(message))) =
                tokio::time::timeout(std::time::Duration::from_secs(5), stream.next()).await
            else {
                break;
            };
            heard.insert(message.channel_str().expect("a text topic").to_owned());
        }
        let expected: std::collections::BTreeSet<_> =
            topics.iter().map(|t| t.as_str().to_owned()).collect();
        assert_eq!(heard, expected, "a broadcast missed its sharded subscriber");
    }

    #[tokio::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs redis; run with --features db")]
    async fn pending_map_roundtrips_seq_to_wire() {
        let redis = connect_redis().await;
        let doc = Id::<Document>::mint();
        let key = format!("doc:{{{doc}}}:pending");
        let _ = redis.del([key.as_str()]).await;

        let queued = append_pending_delta(
            &redis,
            &doc,
            ambre_core::Seq::<ambre_core::Relay>::new(7),
            &ambre_core::ObjectKey::from("s3-a"),
            &StoredPayload::new(vec![1, 2, 3]),
        )
        .await
        .unwrap();

        assert!(matches!(queued, Queued::Marked), "the store takes the mark");
        let queued = append_pending_delta(
            &redis,
            &doc,
            ambre_core::Seq::<ambre_core::Relay>::new(8),
            &ambre_core::ObjectKey::from("s3-b"),
            &StoredPayload::new(vec![4, 5]),
        )
        .await
        .unwrap();
        assert!(matches!(queued, Queued::Marked), "the store takes the mark");

        let map = get_pending_deltas_map(&redis, &doc).await.unwrap();
        assert_eq!(map.len(), 2);
        assert_eq!(
            map.get(&ambre_core::Seq::<ambre_core::Relay>::new(7))
                .map(StoredPayload::expose_bytes),
            Some(&[1u8, 2, 3][..])
        );
        assert_eq!(
            map.get(&ambre_core::Seq::<ambre_core::Relay>::new(8))
                .map(StoredPayload::expose_bytes),
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
        let doc = Id::<Document>::mint();
        let key = format!("doc:{{{doc}}}:presence");
        let pk = &FixedBytes::<PublicKey>::from_array([0x0b; 32]);
        let _ = redis.del([key.as_str()]).await;

        doc_presence_update(
            &redis,
            &doc,
            pk,
            &super::tests::sample(),
            TimeSpan::<Sec>::new(15),
        )
        .await
        .unwrap();
        let present: bool = redis.hexists(&key, pk.to_hex()).await.unwrap();
        assert!(present, "the sample is in the hash");

        doc_presence_remove(&redis, &doc, pk).await.unwrap();
        let present: bool = redis.hexists(&key, pk.to_hex()).await.unwrap();
        assert!(!present, "the departure removed the field");

        // Removing a field nobody wrote is not an error: a connection that never
        // sent presence still announces its departure.
        doc_presence_remove(
            &redis,
            &doc,
            &FixedBytes::<PublicKey>::from_array([0x99; 32]),
        )
        .await
        .unwrap();

        let _ = redis.del([key.as_str()]).await;
    }

    #[tokio::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs redis; run with --features db")]
    async fn oidc_state_roundtrips() {
        let redis = connect_redis().await;
        let csrf =
            ambre_core::Credential::<ambre_core::OidcState>::new(uuid::Uuid::new_v4().to_string());
        let kept = OidcAuthState {
            pkce_verifier_secret: ambre_core::Confidential::<OidcSecret>::new(
                "verifier-xyz".to_owned(),
            ),
            nonce: openidconnect::Nonce::new("nonce-abc".to_owned()),
        };
        oidc_state_set(&redis, &csrf, &kept)
            .await
            .expect("oidc state set");
        let taken = oidc_state_take(&redis, &csrf)
            .await
            .expect("oidc state take");
        assert_eq!(
            taken.pkce_verifier_secret.present_stored(str::to_owned),
            "verifier-xyz"
        );
        assert_eq!(taken.nonce.secret(), "nonce-abc");
    }

    #[tokio::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs redis; run with --features db")]
    async fn presence_field_self_expires() {
        let redis = connect_redis().await;
        let doc = Id::<Document>::mint();
        let key = format!("doc:{{{doc}}}:presence");
        let pk = &FixedBytes::<PublicKey>::from_array([0xef; 32]);
        let _ = redis.del([key.as_str()]).await;

        doc_presence_update(
            &redis,
            &doc,
            pk,
            &super::tests::sample(),
            TimeSpan::<Sec>::new(15),
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
        let doc = Id::<Document>::mint();
        let clocks_key = format!("doc:{{{doc}}}:clocks");
        let seq_key = format!("doc:{{{doc}}}:seq");
        let author = &FixedBytes::<PublicKey>::from_array([0xef; 32]);
        // Cold Redis (as after a restart): neither key exists.
        let _ = redis.del([clocks_key.as_str(), seq_key.as_str()]).await;
        assert!(clock_state_is_cold(&redis, &doc).await.unwrap());

        // The durable index says this author reached clock 5, global seq 5.
        seed_clock_state(
            &redis,
            &doc,
            ambre_core::Seq::<ambre_core::Relay>::new(5),
            &[(*author, Seq::<DeltaAuthor>::new(5))],
        )
        .await
        .expect("seed");

        // No longer cold, and the author's next update (clock 6) validates.
        assert!(!clock_state_is_cold(&redis, &doc).await.unwrap());
        let seq = verify_clock_and_assign_seq(&redis, &doc, author, Seq::<DeltaAuthor>::new(6))
            .await
            .expect("clock 6 should validate against a rehydrated current=5");
        assert_eq!(
            seq,
            ambre_core::Seq::<ambre_core::Relay>::new(6),
            "seq counter must continue past the durable max"
        );

        // A stale clock (already consumed) is still rejected.
        assert!(
            verify_clock_and_assign_seq(&redis, &doc, author, Seq::<DeltaAuthor>::new(6))
                .await
                .is_err()
        );

        let _ = redis.del([clocks_key.as_str(), seq_key.as_str()]).await;
    }

    async fn seed_refresh(redis: &Client) -> Credential<RefreshToken> {
        let token = Credential::<RefreshToken>::new(uuid::Uuid::new_v4().to_string());
        let data = RefreshData {
            user_id: Id::<User>::mint(),
            signing_pubkey: Some(FixedBytes::<PublicKey>::from_array([3u8; 32])),
        };
        refresh_create(redis, &token, &data, TimeSpan::<Sec>::new(60))
            .await
            .expect("refresh create");
        token
    }

    /// `SRV-291`: a refresh record and a sign-in state carry their lifetime
    /// from the one script that writes them, with every field readable back.
    #[tokio::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs redis; run with --features db")]
    async fn a_record_is_written_with_its_lifetime() {
        let redis = connect_redis().await;
        let token = seed_refresh(&redis).await;
        let ttl: i64 = crate::secrets::token::refresh_key(&token, |key| redis.ttl(key))
            .await
            .unwrap();
        assert!((1..=60).contains(&ttl), "the record expires: {ttl}");
        let data = refresh_get(&redis, &token).await.unwrap().expect("record");
        assert_eq!(
            data.signing_pubkey,
            Some(FixedBytes::<PublicKey>::from_array([3u8; 32]))
        );

        let csrf =
            ambre_core::Credential::<ambre_core::OidcState>::new(uuid::Uuid::new_v4().to_string());
        let kept = OidcAuthState {
            pkce_verifier_secret: ambre_core::Confidential::<OidcSecret>::new("v".to_owned()),
            nonce: openidconnect::Nonce::new("n".to_owned()),
        };
        oidc_state_set(&redis, &csrf, &kept).await.unwrap();
        let ttl: i64 = crate::secrets::token::oidc_state_key(&csrf, |key| redis.ttl(key))
            .await
            .unwrap();
        assert!((1..=600).contains(&ttl), "the sign-in state expires: {ttl}");
        let _ = refresh_delete(&redis, &token).await;
        let _ = oidc_state_take(&redis, &csrf).await;
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
