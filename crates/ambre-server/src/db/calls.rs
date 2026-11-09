//! The roster of a call (`ambre-spec-call-media.md` section 3).
//!
//! Per call, the relay keeps its channel, the pseudonym of its room on the SFU, its start and its
//! end. It keeps the public key of each participant session while its socket is open. It keeps
//! nothing else of a call (`ZK-022`).

use super::*;

use ambre_core::id::Call;
use ambre_core::{Count, PublicKey, RoomPseudonym};
use ambre_protocol::signaling::{CallParticipants, CallRefusal, PARTICIPANT_CAP};

/// What a join gets from the roster.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CallJoin {
    /// The session is in the roster. The room of the call, and the key of each session that
    /// was in the call before it.
    Admitted {
        room: FixedBytes<RoomPseudonym>,
        roster: Vec<FixedBytes<PublicKey>>,
    },
    /// The roster refuses the session, which the relay states in a notice.
    Refused(CallRefusal),
}

/// Whether a leave emptied the call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallLeave {
    /// Another session is still in the call.
    Continues,
    /// The last session left, and the call has ended.
    Ended,
}

/// Whether `user` is a member of `channel`: a member row on its `meta` document (`SRV-047`).
///
/// # Errors
/// A database error.
pub async fn is_channel_member(
    pool: &PgPool,
    channel: &Id<Channel>,
    user: &Id<User>,
) -> Result<bool, AppError> {
    let found: Option<i32> = sqlx::query_scalar(
        "SELECT 1 FROM channel_documents d \
         JOIN document_members m ON m.document_id = d.document_id \
         WHERE d.channel_id = $1 AND d.kind = 'meta' AND m.user_id = $2",
    )
    .bind(id_to_uuid(*channel))
    .bind(id_to_uuid(*user))
    .fetch_optional(pool)
    .await?;
    Ok(found.is_some())
}

/// Puts one session of `key` into the roster of `call`, in one transaction.
///
/// The first join starts the call in `channel`, under the room pseudonym `room`. A later join
/// reads the room the first one stored. The row of the call is locked, so two joins never both
/// take the last seat under the cap (`CALL-021`).
///
/// # Errors
/// A database error.
pub async fn join_call(
    pool: &PgPool,
    call: &Id<Call>,
    channel: &Id<Channel>,
    key: &FixedBytes<PublicKey>,
    room: &FixedBytes<RoomPseudonym>,
) -> Result<CallJoin, AppError> {
    let mut tx = pool.begin().await?;
    sqlx::query(
        "INSERT INTO calls (id, channel_id, room) VALUES ($1, $2, $3) \
         ON CONFLICT (id) DO NOTHING",
    )
    .bind(id_to_uuid(*call))
    .bind(id_to_uuid(*channel))
    .bind(room.as_slice())
    .execute(&mut *tx)
    .await?;
    let row = sqlx::query(
        "SELECT channel_id, room, ended_at IS NOT NULL AS ended FROM calls \
         WHERE id = $1 FOR UPDATE",
    )
    .bind(id_to_uuid(*call))
    .fetch_one(&mut *tx)
    .await?;
    let stored_channel: Id<Channel> = uuid_row_to_id(row.try_get("channel_id")?);
    if stored_channel != *channel {
        return Ok(CallJoin::Refused(CallRefusal::NotMember));
    }
    if row.try_get::<bool, _>("ended")? {
        return Ok(CallJoin::Refused(CallRefusal::Ended));
    }
    let room = FixedBytes::<RoomPseudonym>::from_slice(&row.try_get::<Vec<u8>, _>("room")?)
        .map_err(|e| sqlx::Error::Decode(Box::new(e)))?;
    let keys: Vec<Vec<u8>> = sqlx::query_scalar(
        "SELECT signing_pubkey FROM call_participants WHERE call_id = $1 ORDER BY joined_at",
    )
    .bind(id_to_uuid(*call))
    .fetch_all(&mut *tx)
    .await?;
    if Count::<CallParticipants>::of(keys.len()) >= PARTICIPANT_CAP {
        return Ok(CallJoin::Refused(CallRefusal::Full));
    }
    let roster = keys
        .into_iter()
        .map(decode_key)
        .collect::<Result<Vec<_>, _>>()?;
    sqlx::query("INSERT INTO call_participants (call_id, signing_pubkey) VALUES ($1, $2)")
        .bind(id_to_uuid(*call))
        .bind(key.as_slice())
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(CallJoin::Admitted { room, roster })
}

/// Takes one session of `key` out of the roster of `call`. The last one out ends the call.
///
/// # Errors
/// A database error.
pub async fn leave_call(
    pool: &PgPool,
    call: &Id<Call>,
    key: &FixedBytes<PublicKey>,
) -> Result<CallLeave, AppError> {
    let mut tx = pool.begin().await?;
    sqlx::query("SELECT 1 FROM calls WHERE id = $1 FOR UPDATE")
        .bind(id_to_uuid(*call))
        .fetch_optional(&mut *tx)
        .await?;
    // Two sessions of one key are two equal rows, so removing either one is the same leave.
    sqlx::query(
        "DELETE FROM call_participants WHERE ctid = (SELECT ctid FROM call_participants \
         WHERE call_id = $1 AND signing_pubkey = $2 LIMIT 1)",
    )
    .bind(id_to_uuid(*call))
    .bind(key.as_slice())
    .execute(&mut *tx)
    .await?;
    let ended = sqlx::query(
        "UPDATE calls SET ended_at = now() WHERE id = $1 AND ended_at IS NULL \
         AND NOT EXISTS (SELECT 1 FROM call_participants WHERE call_id = $1)",
    )
    .bind(id_to_uuid(*call))
    .execute(&mut *tx)
    .await?
    .rows_affected();
    tx.commit().await?;
    Ok(if ended > 0 {
        CallLeave::Ended
    } else {
        CallLeave::Continues
    })
}
