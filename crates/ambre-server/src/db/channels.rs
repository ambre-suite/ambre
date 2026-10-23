// ambre-server/src/db/channels.rs
// The channel register: which documents a channel groups, and their kind
// (`ambre-spec-server.md section 3`). Access to each of them is the member
// rows of the channel's `meta` document, resolved in `members.rs`.

use super::*;
use ambre_core::ChannelDocumentKind;

/// Insert one channel document, by its id, channel and kind. A segment enters
/// `active`, and no other kind carries a state. A macro rather than a `const`,
/// so `concat!` keeps each query `&'static`.
macro_rules! insert_channel_document {
    () => {
        "INSERT INTO channel_documents (document_id, channel_id, kind, state) \
         VALUES ($1, $2, $3, CASE WHEN $3 = 'segment' THEN 'active' END)"
    };
}

/// One document a channel groups.
#[derive(Debug)]
pub struct ChannelDocumentRow {
    pub document_id: ambre_core::DocumentId,
    pub kind: ambre_core::ChannelDocumentKind,
}

/// The documents a channel groups, `meta` first, then by identifier.
///
/// An unknown channel groups nothing, so it answers an empty list.
pub async fn list_channel_documents(
    pool: &PgPool,
    channel_id: &ambre_core::ChannelId,
) -> Result<Vec<ChannelDocumentRow>, AppError> {
    let rows = sqlx::query(
        "SELECT document_id, kind FROM channel_documents WHERE channel_id = $1 \
         ORDER BY kind <> 'meta', document_id",
    )
    .bind(id_to_uuid(*channel_id))
    .fetch_all(pool)
    .await?;
    rows.into_iter()
        .map(|r| {
            let document_id: uuid::Uuid = r.try_get("document_id")?;
            let kind: String = r.try_get("kind")?;
            Ok(ChannelDocumentRow {
                document_id: uuid_row_to_id(document_id),
                kind: decode_kind(&kind)?,
            })
        })
        .collect()
}

/// A stored kind. A word no build wrote is a decode error: the column's
/// check constraint admits none.
pub(super) fn decode_kind(word: &str) -> Result<ambre_core::ChannelDocumentKind, AppError> {
    ambre_core::ChannelDocumentKind::parse(word).ok_or_else(|| {
        AppError::Database(sqlx::Error::Decode(
            format!("unknown channel document kind {word:?}").into(),
        ))
    })
}

/// What the relay holds of a channel: its space and its public flag
/// (`ZK-015`), and the `meta` document its access resolves to.
#[derive(Debug)]
pub struct ChannelRow {
    /// Absent for a direct message.
    pub space_id: Option<ambre_core::SpaceId>,
    pub is_public: bool,
    /// Absent only for a channel whose `meta` was never registered.
    pub meta_id: Option<ambre_core::MetaId>,
}

/// The channel, or `None` when the relay holds no such channel.
pub async fn get_channel(
    pool: &PgPool,
    channel_id: &ambre_core::ChannelId,
) -> Result<Option<ChannelRow>, AppError> {
    let row = sqlx::query(
        "SELECT c.space_id, c.is_public, d.document_id AS meta_id FROM channels c \
         LEFT JOIN channel_documents d ON d.channel_id = c.id AND d.kind = 'meta' \
         WHERE c.id = $1",
    )
    .bind(id_to_uuid(*channel_id))
    .fetch_optional(pool)
    .await?;
    row.map(|r| {
        let space_id: Option<uuid::Uuid> = r.try_get("space_id")?;
        let meta_id: Option<uuid::Uuid> = r.try_get("meta_id")?;
        Ok(ChannelRow {
            space_id: space_id.map(uuid_row_to_id),
            is_public: r.try_get("is_public")?,
            meta_id: meta_id.map(uuid_row_to_id),
        })
    })
    .transpose()
}

/// Hold `space` as a space. A second registration changes nothing.
pub async fn register_space(pool: &PgPool, space: &ambre_core::SpaceId) -> Result<(), AppError> {
    sqlx::query("INSERT INTO spaces (id) VALUES ($1) ON CONFLICT DO NOTHING")
        .bind(id_to_uuid(*space))
        .execute(pool)
        .await?;
    Ok(())
}

/// Every space whose document holds a row of `user`, by identifier
/// (`SRV-216`).
pub async fn list_member_spaces(
    pool: &PgPool,
    user: &ambre_core::UserId,
) -> Result<Vec<ambre_core::SpaceId>, AppError> {
    let ids: Vec<uuid::Uuid> = sqlx::query_scalar(
        "SELECT s.id FROM spaces s JOIN document_members m ON m.document_id = s.id \
         WHERE m.user_id = $1 ORDER BY s.id",
    )
    .bind(id_to_uuid(*user))
    .fetch_all(pool)
    .await?;
    Ok(ids.into_iter().map(uuid_row_to_id).collect())
}

/// Every user holding a row of `document`, when it is a space; none for any
/// other document (`SRV-218`).
pub async fn space_members(
    pool: &PgPool,
    document: &ambre_core::DocumentId,
) -> Result<Vec<ambre_core::UserId>, AppError> {
    let ids: Vec<uuid::Uuid> = sqlx::query_scalar(
        "SELECT m.user_id FROM spaces s JOIN document_members m ON m.document_id = s.id \
         WHERE s.id = $1 ORDER BY m.user_id",
    )
    .bind(id_to_uuid(*document))
    .fetch_all(pool)
    .await?;
    Ok(ids.into_iter().map(uuid_row_to_id).collect())
}

/// The channels the relay holds as public in `space`, by identifier.
pub async fn list_space_public_channels(
    pool: &PgPool,
    space: &ambre_core::SpaceId,
) -> Result<Vec<ambre_core::ChannelId>, AppError> {
    let ids: Vec<uuid::Uuid> =
        sqlx::query_scalar("SELECT id FROM channels WHERE space_id = $1 AND is_public ORDER BY id")
            .bind(id_to_uuid(*space))
            .fetch_all(pool)
            .await?;
    Ok(ids.into_iter().map(uuid_row_to_id).collect())
}

/// A channel `user` is a member of, as their listing answers it.
#[derive(Debug, PartialEq, Eq)]
pub struct MemberChannelRow {
    pub id: ambre_core::ChannelId,
    /// `None` for a direct message.
    pub space_id: Option<ambre_core::SpaceId>,
    pub is_public: bool,
    pub meta_id: ambre_core::MetaId,
    /// The name sealed beside `meta`, if any (`SRV-219`).
    pub name_enc: Option<ambre_core::SealedName>,
    /// The caller's copy of the channel key, and the exchange key of whoever
    /// wrapped it, `None` for a self-wrapped row.
    pub doc_key_enc: ambre_core::WrappedDocKey,
    pub wrapper_exchange_pubkey: Option<ambre_core::ExchangePublicKey>,
    /// For a direct message, the display names of its other members, by
    /// name then id. Empty for a channel of a space (`SRV-220`).
    pub peers: Vec<ambre_core::DisplayName>,
    /// The highest sequence number the register holds for `meta`. The
    /// ephemeral store may hold a higher one not yet flushed.
    pub meta_durable_last: ambre_core::Seq,
}

/// Every channel whose `meta` holds a row of `user`, by identifier
/// (`SRV-191`), with the name sealed beside its `meta` (`SRV-219`). A direct
/// message also lists its other members (`SRV-220`).
pub async fn list_member_channels(
    pool: &PgPool,
    user: &ambre_core::UserId,
) -> Result<Vec<MemberChannelRow>, AppError> {
    let rows = sqlx::query(
        "SELECT c.id, c.space_id, c.is_public, d.document_id, doc.name_enc, m.doc_key_enc, \
         inv.exchange_pubkey AS wrapper_exchange_pubkey, \
         CASE WHEN c.space_id IS NULL THEN ARRAY(SELECT u.display_name \
           FROM document_members pm JOIN users u ON u.id = pm.user_id \
           WHERE pm.document_id = d.document_id AND pm.user_id <> $1 \
           ORDER BY u.display_name, u.id) ELSE ARRAY[]::text[] END AS peers, \
         GREATEST( \
             COALESCE((SELECT MAX(u.seq) FROM updates u \
                       WHERE u.document_id = d.document_id), 0), \
             COALESCE((SELECT MAX(s.covered_seq) FROM snapshots s \
                       WHERE s.document_id = d.document_id), 0) \
         ) AS meta_durable_last \
         FROM channels c \
         JOIN channel_documents d ON d.channel_id = c.id AND d.kind = 'meta' \
         JOIN documents doc ON doc.id = d.document_id \
         JOIN document_members m ON m.document_id = d.document_id \
         LEFT JOIN users inv ON inv.id = m.invited_by \
         WHERE m.user_id = $1 ORDER BY c.id",
    )
    .bind(id_to_uuid(*user))
    .fetch_all(pool)
    .await?;
    rows.into_iter()
        .map(|r| {
            let space_id: Option<uuid::Uuid> = r.try_get("space_id")?;
            Ok(MemberChannelRow {
                id: uuid_row_to_id(r.try_get("id")?),
                space_id: space_id.map(uuid_row_to_id),
                is_public: r.try_get("is_public")?,
                meta_id: ambre_core::MetaId::from_document(uuid_row_to_id(
                    r.try_get("document_id")?,
                )),
                name_enc: r
                    .try_get::<Option<Vec<u8>>, _>("name_enc")?
                    .map(ambre_core::SealedName::new),
                doc_key_enc: ambre_core::WrappedDocKey::new(r.try_get("doc_key_enc")?),
                wrapper_exchange_pubkey: decode_opt_exchange_key(
                    r.try_get("wrapper_exchange_pubkey")?,
                )?,
                peers: decode_display_names(r.try_get("peers")?)?,
                meta_durable_last: decode_seq(r.try_get("meta_durable_last")?)?,
            })
        })
        .collect()
}

/// The channel a document belongs to, or `None` for a document outside
/// every channel.
pub async fn channel_of_document(
    pool: &PgPool,
    document_id: &ambre_core::DocumentId,
) -> Result<Option<ambre_core::ChannelId>, AppError> {
    let channel: Option<uuid::Uuid> =
        sqlx::query_scalar("SELECT channel_id FROM channel_documents WHERE document_id = $1")
            .bind(id_to_uuid(*document_id))
            .fetch_optional(pool)
            .await?;
    Ok(channel.map(uuid_row_to_id))
}

/// The role `user_id` holds on a document, or `None` for a stranger to it.
///
/// # Errors
/// A stored role no build wrote is a decode error: the column's check
/// constraint admits none.
pub async fn member_role(
    pool: &PgPool,
    document_id: &ambre_core::DocumentId,
    user_id: &ambre_core::UserId,
) -> Result<Option<ambre_core::Role>, AppError> {
    let role: Option<String> = sqlx::query_scalar(
        "SELECT role FROM document_members WHERE document_id = $1 AND user_id = $2",
    )
    .bind(id_to_uuid(*document_id))
    .bind(id_to_uuid(*user_id))
    .fetch_optional(pool)
    .await?;
    Ok(decode_role_opt(role)?)
}

/// A channel to register: its space, its flag, and its first two documents.
#[derive(Debug, Clone, Copy)]
pub struct NewChannel {
    pub channel_id: ambre_core::ChannelId,
    pub space_id: ambre_core::SpaceId,
    pub is_public: bool,
    pub meta_id: ambre_core::MetaId,
    pub segment_id: ambre_core::SegmentId,
}

/// Register a channel with its `meta` and its first segment, in one
/// transaction.
///
/// A retry of the same registration answers `Ok`, so a client that lost the
/// answer may send it again. A channel id or a document already registered
/// otherwise fails with a 409, and nothing is written.
pub async fn register_channel(pool: &PgPool, new: &NewChannel) -> Result<(), AppError> {
    let channel = id_to_uuid(new.channel_id);
    let space = id_to_uuid(new.space_id);
    let mut tx = pool.begin().await?;
    sqlx::query(
        "INSERT INTO channels (id, space_id, is_public) VALUES ($1, $2, $3) \
         ON CONFLICT (id) DO NOTHING",
    )
    .bind(channel)
    .bind(space)
    .bind(new.is_public)
    .execute(&mut *tx)
    .await?;
    let (space_id, is_public): (Option<uuid::Uuid>, bool) =
        sqlx::query_as("SELECT space_id, is_public FROM channels WHERE id = $1")
            .bind(channel)
            .fetch_one(&mut *tx)
            .await?;
    let conflict = || AppError::Conflict {
        detail: "the channel or one of its documents is already registered".into(),
    };
    if space_id != Some(space) || is_public != new.is_public {
        return Err(conflict());
    }
    for (document, kind) in [
        (new.meta_id.document_id(), ChannelDocumentKind::Meta),
        (new.segment_id.document_id(), ChannelDocumentKind::Segment),
    ] {
        let document = id_to_uuid(document);
        sqlx::query(concat!(
            insert_channel_document!(),
            " ON CONFLICT DO NOTHING"
        ))
        .bind(document)
        .bind(channel)
        .bind(kind.as_str())
        .execute(&mut *tx)
        .await?;
        let held: Option<(uuid::Uuid, String)> =
            sqlx::query_as("SELECT channel_id, kind FROM channel_documents WHERE document_id = $1")
                .bind(document)
                .fetch_optional(&mut *tx)
                .await?;
        if held.as_ref().map(|(c, k)| (*c, k.as_str())) != Some((channel, kind.as_str())) {
            return Err(conflict());
        }
    }
    tx.commit().await?;
    Ok(())
}

/// Close the active segment `closing` of a channel into `successor`, in one
/// transaction (`SRV-194`), and answer the channel's active segment after it.
///
/// The channel row is locked first, so two closes take turns. The second
/// finds `closing` already closing, and answers the successor the first
/// registered (`SRV-195`). A close while another segment of the channel is
/// still closing is refused with a 409 (`SRV-196`). So is a `closing` that is
/// not the active segment, or a successor already registered.
pub async fn close_segment(
    pool: &PgPool,
    channel_id: &ambre_core::ChannelId,
    closing: ambre_core::SegmentId,
    successor: ambre_core::SegmentId,
) -> Result<ambre_core::SegmentId, AppError> {
    let channel = id_to_uuid(*channel_id);
    let conflict = |detail: &str| AppError::Conflict {
        detail: detail.into(),
    };
    let mut tx = pool.begin().await?;
    sqlx::query_scalar::<_, uuid::Uuid>("SELECT id FROM channels WHERE id = $1 FOR UPDATE")
        .bind(channel)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(AppError::NotFound {
            resource: crate::error::Resource::Channel,
        })?;
    let live: Vec<(uuid::Uuid, String)> = sqlx::query_as(
        "SELECT document_id, state FROM channel_documents \
         WHERE channel_id = $1 AND state IN ('active', 'closing')",
    )
    .bind(channel)
    .fetch_all(&mut *tx)
    .await?;
    let of = |state: &str| live.iter().find(|(_, s)| s == state).map(|(d, _)| *d);
    let (active, closing_now) = (of("active"), of("closing"));
    let closing = id_to_uuid(closing.document_id());
    if closing_now == Some(closing) {
        let active = active.ok_or_else(|| conflict("the channel has no active segment"))?;
        return Ok(ambre_core::SegmentId::from_document(uuid_row_to_id(active)));
    }
    if active != Some(closing) {
        return Err(conflict("the segment is not the active one"));
    }
    if closing_now.is_some() {
        return Err(conflict("another segment of the channel is still closing"));
    }
    sqlx::query(
        "UPDATE channel_documents SET state = 'closing', closed_at = now() \
         WHERE document_id = $1",
    )
    .bind(closing)
    .execute(&mut *tx)
    .await?;
    let inserted = sqlx::query(concat!(
        insert_channel_document!(),
        " ON CONFLICT DO NOTHING"
    ))
    .bind(id_to_uuid(successor.document_id()))
    .bind(channel)
    .bind(ChannelDocumentKind::Segment.as_str())
    .execute(&mut *tx)
    .await?;
    if inserted.rows_affected() != 1 {
        return Err(conflict("the successor is already registered"));
    }
    tx.commit().await?;
    Ok(successor)
}

/// Whether a finalize lease locked `document_id` against deltas (`SRV-114`).
/// The lock is its horizon, so a segment with one is locked. A document
/// outside every channel is not locked.
pub async fn segment_locked(
    pool: &PgPool,
    document_id: &ambre_core::DocumentId,
) -> Result<bool, AppError> {
    let locked: Option<bool> = sqlx::query_scalar(
        "SELECT horizon IS NOT NULL FROM channel_documents WHERE document_id = $1",
    )
    .bind(id_to_uuid(*document_id))
    .fetch_optional(pool)
    .await?;
    Ok(locked.unwrap_or(false))
}

/// The closing segments of the channel of `document_id` that a rotation
/// retired the key of, and that no lock holds yet (`SRV-147`).
pub async fn retired_unlocked_segments(
    pool: &PgPool,
    document_id: &ambre_core::DocumentId,
) -> Result<Vec<ambre_core::DocumentId>, AppError> {
    let rows: Vec<uuid::Uuid> = sqlx::query_scalar(
        "SELECT s.document_id FROM channel_documents s \
         JOIN channel_documents d ON d.channel_id = s.channel_id \
         WHERE d.document_id = $1 AND s.kind = 'segment' AND s.state = 'closing' \
           AND s.key_retired AND s.horizon IS NULL",
    )
    .bind(id_to_uuid(*document_id))
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(uuid_row_to_id).collect())
}

/// Record the lock of `segment` at its delta `horizon`, unless a lock holds
/// it already: a lock is set once.
pub async fn record_lock(
    pool: &PgPool,
    segment: &ambre_core::DocumentId,
    horizon: ambre_core::Seq,
) -> Result<(), AppError> {
    sqlx::query(
        "UPDATE channel_documents SET horizon = $2 WHERE document_id = $1 AND horizon IS NULL",
    )
    .bind(id_to_uuid(*segment))
    .bind(horizon.to_i64_saturating())
    .execute(pool)
    .await?;
    Ok(())
}

/// Register `amendments` as the one amendments document of the locked
/// segment `segment` (`SRV-201`), and answer the one the segment holds after.
///
/// The channel row is locked first, so two registrations take turns. The
/// second finds the first one's document, and is answered it (`SRV-202`).
/// A segment of another channel, or not yet locked, is refused with a 409.
/// So is an `amendments` already registered.
pub async fn register_amendments(
    pool: &PgPool,
    channel_id: &ambre_core::ChannelId,
    segment: ambre_core::SegmentId,
    amendments: ambre_core::DocumentId,
) -> Result<ambre_core::DocumentId, AppError> {
    let channel = id_to_uuid(*channel_id);
    let conflict = |detail: &str| AppError::Conflict {
        detail: detail.into(),
    };
    let mut tx = pool.begin().await?;
    sqlx::query_scalar::<_, uuid::Uuid>("SELECT id FROM channels WHERE id = $1 FOR UPDATE")
        .bind(channel)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(AppError::NotFound {
            resource: crate::error::Resource::Channel,
        })?;
    let segment = id_to_uuid(segment.document_id());
    let locked: Option<bool> = sqlx::query_scalar(
        "SELECT horizon IS NOT NULL FROM channel_documents \
         WHERE document_id = $1 AND channel_id = $2 AND kind = 'segment'",
    )
    .bind(segment)
    .bind(channel)
    .fetch_optional(&mut *tx)
    .await?;
    if locked != Some(true) {
        return Err(conflict(
            "the segment is not a locked segment of the channel",
        ));
    }
    let held: Option<uuid::Uuid> =
        sqlx::query_scalar("SELECT document_id FROM channel_documents WHERE amends = $1")
            .bind(segment)
            .fetch_optional(&mut *tx)
            .await?;
    if let Some(held) = held {
        return Ok(uuid_row_to_id(held));
    }
    let inserted = sqlx::query(
        "INSERT INTO channel_documents (document_id, channel_id, kind, amends) \
         VALUES ($1, $2, $3, $4) ON CONFLICT DO NOTHING",
    )
    .bind(id_to_uuid(amendments))
    .bind(channel)
    .bind(ChannelDocumentKind::Amendments.as_str())
    .bind(segment)
    .execute(&mut *tx)
    .await?;
    if inserted.rows_affected() != 1 {
        return Err(conflict("the amendments document is already registered"));
    }
    tx.commit().await?;
    Ok(amendments)
}

/// Register `thread` as the one thread document of the root message `root`
/// (`SRV-213`), and answer the one the root holds after.
///
/// The channel row is locked first, so two registrations take turns. The
/// second finds the first one's document, and is answered it (`SRV-214`).
/// A `thread` already registered is refused with a 409.
pub async fn register_thread(
    pool: &PgPool,
    channel_id: &ambre_core::ChannelId,
    root: ambre_core::MessageId,
    thread: ambre_core::DocumentId,
) -> Result<ambre_core::DocumentId, AppError> {
    let channel = id_to_uuid(*channel_id);
    let root = id_to_uuid(root);
    let mut tx = pool.begin().await?;
    sqlx::query_scalar::<_, uuid::Uuid>("SELECT id FROM channels WHERE id = $1 FOR UPDATE")
        .bind(channel)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(AppError::NotFound {
            resource: crate::error::Resource::Channel,
        })?;
    let held: Option<uuid::Uuid> = sqlx::query_scalar(
        "SELECT document_id FROM channel_documents WHERE channel_id = $1 AND thread_root = $2",
    )
    .bind(channel)
    .bind(root)
    .fetch_optional(&mut *tx)
    .await?;
    if let Some(held) = held {
        return Ok(uuid_row_to_id(held));
    }
    let inserted = sqlx::query(
        "INSERT INTO channel_documents (document_id, channel_id, kind, thread_root) \
         VALUES ($1, $2, $3, $4) ON CONFLICT DO NOTHING",
    )
    .bind(id_to_uuid(thread))
    .bind(channel)
    .bind(ChannelDocumentKind::Thread.as_str())
    .bind(root)
    .execute(&mut *tx)
    .await?;
    if inserted.rows_affected() != 1 {
        return Err(AppError::Conflict {
            detail: "the thread document is already registered".into(),
        });
    }
    tx.commit().await?;
    Ok(thread)
}

/// How long a finalize lease waits, and how long it lasts
/// (`ambre-spec-budgets.md section 8`).
#[derive(Debug, Clone, Copy)]
pub struct LeaseTerms {
    /// Since the close and since the latest delta, before any lease.
    pub grace: std::time::Duration,
    /// How long a granted lease lasts.
    pub lease: std::time::Duration,
    /// The wait answered while a delta still waits to be flushed.
    pub pending_retry: std::time::Duration,
}

/// The answer to a finalize lease request.
#[derive(Debug, PartialEq, Eq)]
pub enum FinalizeLease {
    /// The caller holds the lease. It writes the snapshot of every delta up
    /// to `horizon`, and commits it before `expires_in` runs out.
    Granted {
        lease_id: ambre_core::LeaseId,
        horizon: ambre_core::Seq,
        expires_in: std::time::Duration,
    },
    /// No lease now: ask again after `retry_in`.
    Wait { retry_in: std::time::Duration },
}

/// Take the finalize lease of the closing segment `segment` of a channel
/// (`SRV-113`), in one transaction that locks its row.
///
/// A segment not yet locked waits out the grace period since its close and
/// since its latest delta (`SRV-199`). Then `lock` locks it in the store and
/// answers the last delta number it holds, or `None` while a delta still
/// waits to be flushed. The register then records the lock and its horizon
/// for good (`SRV-114`). A live lease answers its time left. An expired one
/// gives way to a new lease (`SRV-119`). A segment that is not closing, or
/// not of the channel, is refused with a 409.
pub async fn take_finalize_lease(
    pool: &PgPool,
    channel_id: &ambre_core::ChannelId,
    segment: ambre_core::SegmentId,
    terms: LeaseTerms,
    lock: impl AsyncFnOnce() -> Result<Option<ambre_core::Seq>, AppError>,
) -> Result<FinalizeLease, AppError> {
    let document = id_to_uuid(segment.document_id());
    let mut tx = pool.begin().await?;
    let row: Option<LeaseRow> = sqlx::query_as(
        "SELECT d.state, d.horizon, d.final_snapshot, \
           CEIL(EXTRACT(EPOCH FROM GREATEST(COALESCE(d.closed_at, now()), \
             COALESCE((SELECT max(u.created_at) FROM updates u \
                       WHERE u.document_id = d.document_id), '-infinity')) \
             + make_interval(secs => $3) - now()) * 1000)::BIGINT AS grace_left_ms, \
           CEIL(EXTRACT(EPOCH FROM d.lease_until - now()) * 1000)::BIGINT AS lease_left_ms \
         FROM channel_documents d WHERE d.document_id = $1 AND d.channel_id = $2 \
         FOR UPDATE OF d",
    )
    .bind(document)
    .bind(id_to_uuid(*channel_id))
    .bind(terms.grace.as_secs_f64())
    .fetch_optional(&mut *tx)
    .await?;
    let Some(LeaseRow {
        state: Some(state),
        horizon,
        final_snapshot,
        grace_left,
        lease_left,
    }) = row
    else {
        return Err(not_closing());
    };
    if !finalizable(state, final_snapshot) {
        return Err(not_closing());
    }
    let wait = |retry_in| FinalizeLease::Wait { retry_in };
    let horizon = match (horizon, grace_left) {
        (Some(horizon), _) => horizon,
        (None, Some(left)) => return Ok(wait(left)),
        (None, None) => {
            let Some(seq) = lock().await? else {
                return Ok(FinalizeLease::Wait {
                    retry_in: terms.pending_retry,
                });
            };
            let stored = i64::try_from(seq.0)
                .map_err(|_| AppError::Internal("delta number out of range".into()))?;
            sqlx::query("UPDATE channel_documents SET horizon = $2 WHERE document_id = $1")
                .bind(document)
                .bind(stored)
                .execute(&mut *tx)
                .await?;
            seq
        }
    };
    if let Some(left) = lease_left {
        tx.commit().await?;
        return Ok(wait(left));
    }
    let lease_id = ambre_core::LeaseId::new();
    sqlx::query(
        "UPDATE channel_documents SET lease_id = $2, \
         lease_until = now() + make_interval(secs => $3) WHERE document_id = $1",
    )
    .bind(document)
    .bind(id_to_uuid(lease_id))
    .bind(terms.lease.as_secs_f64())
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(FinalizeLease::Granted {
        lease_id,
        horizon,
        expires_in: terms.lease,
    })
}

/// What a lease request reads of a segment, the waits by the register's
/// clock.
#[derive(sqlx::FromRow)]
struct LeaseRow {
    #[sqlx(try_from = "PgOptState")]
    state: Option<ambre_core::SegmentState>,
    #[sqlx(try_from = "PgOptSeq")]
    horizon: Option<ambre_core::Seq>,
    #[sqlx(try_from = "PgOptId")]
    final_snapshot: Option<ambre_core::SnapshotId>,
    #[sqlx(try_from = "PgTimeLeft", rename = "grace_left_ms")]
    grace_left: Option<std::time::Duration>,
    #[sqlx(try_from = "PgTimeLeft", rename = "lease_left_ms")]
    lease_left: Option<std::time::Duration>,
}

/// What a finalize commit reads of a segment.
#[derive(sqlx::FromRow)]
struct CommitRow {
    #[sqlx(try_from = "PgOptState")]
    state: Option<ambre_core::SegmentState>,
    /// The live or expired lease, if any.
    #[sqlx(try_from = "PgOptId")]
    held: Option<ambre_core::LeaseId>,
    live: bool,
    #[sqlx(try_from = "PgOptSeq")]
    horizon: Option<ambre_core::Seq>,
    #[sqlx(try_from = "PgOptId")]
    final_snapshot: Option<ambre_core::SnapshotId>,
}

/// The delta number a snapshot of `document` folds in, `None` for a snapshot
/// of another document or none at all.
async fn covered_seq_of(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    snapshot: ambre_core::SnapshotId,
    document: ambre_core::DocumentId,
) -> Result<Option<ambre_core::Seq>, AppError> {
    let covered: Option<i64> =
        sqlx::query_scalar("SELECT covered_seq FROM snapshots WHERE id = $1 AND document_id = $2")
            .bind(id_to_uuid(snapshot))
            .bind(id_to_uuid(document))
            .fetch_optional(&mut **tx)
            .await?;
    Ok(covered.map(decode_seq).transpose()?)
}

/// Whether a segment takes a lease and a commit: it is closing, or a report
/// dropped its final snapshot (the Murmure segments spec).
fn finalizable(
    state: ambre_core::SegmentState,
    final_snapshot: Option<ambre_core::SnapshotId>,
) -> bool {
    state == ambre_core::SegmentState::Closing
        || (state == ambre_core::SegmentState::Final && final_snapshot.is_none())
}

/// The refusal of a lease or a commit on a segment that is not closing.
fn not_closing() -> AppError {
    AppError::Conflict {
        detail: "the segment is not closing".into(),
    }
}

/// Accept the finalization of `segment` under `lease_id`, by its snapshot
/// `snapshot_id` (`SRV-117`). The segment becomes `final`, and its lease ends.
///
/// A commit without the live lease is refused with a 409 (`SRV-116`). So is a
/// snapshot of another document, or one that does not cover the horizon
/// exactly. A retry of an accepted commit answers `Ok`. Only the row of
/// `segment` is written (`SRV-120`).
pub async fn commit_finalization(
    pool: &PgPool,
    channel_id: &ambre_core::ChannelId,
    segment: ambre_core::SegmentId,
    lease_id: ambre_core::LeaseId,
    snapshot_id: ambre_core::SnapshotId,
) -> Result<(), AppError> {
    let document = id_to_uuid(segment.document_id());
    let conflict = |detail: &str| AppError::Conflict {
        detail: detail.into(),
    };
    let mut tx = pool.begin().await?;
    let row: Option<CommitRow> = sqlx::query_as(
        "SELECT state, lease_id AS held, COALESCE(lease_until > now(), false) AS live, horizon, \
             final_snapshot FROM channel_documents \
             WHERE document_id = $1 AND channel_id = $2 FOR UPDATE",
    )
    .bind(document)
    .bind(id_to_uuid(*channel_id))
    .fetch_optional(&mut *tx)
    .await?;
    let Some(CommitRow {
        state: Some(state),
        held,
        live,
        horizon,
        final_snapshot,
    }) = row
    else {
        return Err(not_closing());
    };
    if state == ambre_core::SegmentState::Final && final_snapshot == Some(snapshot_id) {
        return Ok(());
    }
    if !finalizable(state, final_snapshot) {
        return Err(not_closing());
    }
    if held != Some(lease_id) || !live {
        return Err(conflict("the caller holds no live lease"));
    }
    let covered = covered_seq_of(&mut tx, snapshot_id, segment.document_id()).await?;
    let snapshot_id = id_to_uuid(snapshot_id);
    if covered.is_none() || covered != horizon {
        return Err(conflict("the snapshot does not cover the horizon"));
    }
    sqlx::query(
        "UPDATE channel_documents SET state = 'final', finalized_at = now(), \
         final_snapshot = $2, lease_id = NULL, lease_until = NULL \
         WHERE document_id = $1",
    )
    .bind(document)
    .bind(snapshot_id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

/// How long a fold lease waits, and how long it lasts
/// (`ambre-spec-budgets.md section 8`).
#[derive(Debug, Clone, Copy)]
pub struct FoldTerms {
    /// Since the last fold, or since the amendments document was created.
    pub delay: std::time::Duration,
    /// How long a granted lease lasts.
    pub lease: std::time::Duration,
}

/// The answer to a fold lease request.
#[derive(Debug, PartialEq, Eq)]
pub enum FoldLease {
    /// The caller holds the lease. It folds `amendments` up to its delta
    /// `amendments_seq` into the snapshot of the segment at `horizon`.
    Granted {
        lease_id: ambre_core::LeaseId,
        horizon: ambre_core::Seq,
        amendments: ambre_core::DocumentId,
        amendments_seq: ambre_core::Seq,
        expires_in: std::time::Duration,
    },
    /// No lease now: ask again after `retry_in`.
    Wait { retry_in: std::time::Duration },
}

/// What a fold lease request reads of a segment and of its amendments, the
/// waits by the register's clock.
#[derive(sqlx::FromRow)]
struct FoldRow {
    #[sqlx(try_from = "PgOptState")]
    state: Option<ambre_core::SegmentState>,
    #[sqlx(try_from = "PgOptSeq")]
    horizon: Option<ambre_core::Seq>,
    #[sqlx(try_from = "PgOptId")]
    final_snapshot: Option<ambre_core::SnapshotId>,
    #[sqlx(try_from = "PgTimeLeft", rename = "lease_left_ms")]
    lease_left: Option<std::time::Duration>,
    #[sqlx(try_from = "PgOptId")]
    amendments: Option<ambre_core::DocumentId>,
    #[sqlx(try_from = "PgOptSeq")]
    folded_seq: Option<ambre_core::Seq>,
    #[sqlx(try_from = "PgTimeLeft", rename = "delay_left_ms")]
    delay_left: Option<std::time::Duration>,
    #[sqlx(try_from = "PgOptSeq")]
    stored_seq: Option<ambre_core::Seq>,
}

/// Take the fold lease of the final segment `segment` of a channel
/// (`SRV-204`), in one transaction that locks its row.
///
/// A fold is owed while the amendments document holds a delta past the last
/// fold (`SRV-205`). `relayed` answers the last delta number the store gave
/// it. The lease then waits out the delay since the last fold, or since the
/// document was created (`SRV-206`). A live lease answers its time left. A
/// segment of another channel, not finalized, without amendments or with
/// nothing to fold, is refused with a 409.
pub async fn take_fold_lease(
    pool: &PgPool,
    channel_id: &ambre_core::ChannelId,
    segment: ambre_core::SegmentId,
    terms: FoldTerms,
    relayed: impl AsyncFnOnce(ambre_core::DocumentId) -> Result<ambre_core::Seq, AppError>,
) -> Result<FoldLease, AppError> {
    let document = id_to_uuid(segment.document_id());
    let conflict = |detail: &str| AppError::Conflict {
        detail: detail.into(),
    };
    let mut tx = pool.begin().await?;
    let row: Option<FoldRow> = sqlx::query_as(
        "SELECT d.state, d.horizon, d.final_snapshot, \
           CEIL(EXTRACT(EPOCH FROM d.lease_until - now()) * 1000)::BIGINT AS lease_left_ms, \
           a.document_id AS amendments, \
           CASE WHEN a.folded_by IS NOT NULL THEN a.folded_seq END AS folded_seq, \
           CEIL(EXTRACT(EPOCH FROM COALESCE(CASE WHEN a.folded_by IS NOT NULL \
             THEN a.folded_at END, o.created_at) + make_interval(secs => $3) - now()) \
             * 1000)::BIGINT AS delay_left_ms, \
           (SELECT max(u.seq) FROM updates u WHERE u.document_id = a.document_id) AS stored_seq \
         FROM channel_documents d \
         LEFT JOIN channel_documents a ON a.amends = d.document_id \
         LEFT JOIN documents o ON o.id = a.document_id \
         WHERE d.document_id = $1 AND d.channel_id = $2 AND d.kind = 'segment' \
         FOR UPDATE OF d",
    )
    .bind(document)
    .bind(id_to_uuid(*channel_id))
    .bind(terms.delay.as_secs_f64())
    .fetch_optional(&mut *tx)
    .await?;
    let Some(FoldRow {
        state: _,
        horizon: Some(horizon),
        final_snapshot: Some(_),
        lease_left,
        amendments,
        folded_seq,
        delay_left,
        stored_seq,
    }) = row.filter(|row| row.state == Some(ambre_core::SegmentState::Final))
    else {
        return Err(conflict("the segment is not finalized"));
    };
    let Some(amendments) = amendments else {
        return Err(conflict("the segment has no amendments"));
    };
    let seq = relayed(amendments)
        .await?
        .max(stored_seq.unwrap_or_default());
    if seq <= folded_seq.unwrap_or_default() {
        return Err(conflict("nothing to fold"));
    }
    if let Some(retry_in) = delay_left.or(lease_left) {
        return Ok(FoldLease::Wait { retry_in });
    }
    let lease_id = ambre_core::LeaseId::new();
    sqlx::query(
        "UPDATE channel_documents SET lease_id = $2, \
         lease_until = now() + make_interval(secs => $3) WHERE document_id = $1",
    )
    .bind(document)
    .bind(id_to_uuid(lease_id))
    .bind(terms.lease.as_secs_f64())
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(FoldLease::Granted {
        lease_id,
        horizon,
        amendments,
        amendments_seq: seq,
        expires_in: terms.lease,
    })
}

/// What a fold commit reads of a segment and of its amendments.
#[derive(sqlx::FromRow)]
struct FoldCommitRow {
    #[sqlx(try_from = "PgOptState")]
    state: Option<ambre_core::SegmentState>,
    #[sqlx(try_from = "PgOptId")]
    held: Option<ambre_core::LeaseId>,
    live: bool,
    #[sqlx(try_from = "PgOptSeq")]
    horizon: Option<ambre_core::Seq>,
    #[sqlx(try_from = "PgOptId")]
    final_snapshot: Option<ambre_core::SnapshotId>,
    #[sqlx(try_from = "PgOptId")]
    amendments: Option<ambre_core::DocumentId>,
    #[sqlx(try_from = "PgOptId")]
    folded_by: Option<ambre_core::SnapshotId>,
    #[sqlx(try_from = "PgOptSeq")]
    folded_seq: Option<ambre_core::Seq>,
}

/// Record the fold of the amendments of `segment` up to their delta
/// `amendments_seq` by the snapshot `snapshot_id`, under `lease_id`
/// (`SRV-207`). The lease ends, and the final snapshot stays until a member
/// confirms the fold (`SRV-208`).
///
/// A commit without the live lease is refused with a 409. So is a snapshot of
/// another document, one that does not cover the horizon, or a fold that
/// reaches no further than the last one. A retry of an accepted commit
/// answers `Ok`.
pub async fn commit_fold(
    pool: &PgPool,
    channel_id: &ambre_core::ChannelId,
    segment: ambre_core::SegmentId,
    lease_id: ambre_core::LeaseId,
    snapshot_id: ambre_core::SnapshotId,
    amendments_seq: ambre_core::Seq,
) -> Result<(), AppError> {
    let document = id_to_uuid(segment.document_id());
    let conflict = |detail: &str| AppError::Conflict {
        detail: detail.into(),
    };
    let seq = i64::try_from(amendments_seq.0)
        .map_err(|_| conflict("the delta number is out of range"))?;
    let mut tx = pool.begin().await?;
    let row: Option<FoldCommitRow> = sqlx::query_as(
        "SELECT d.state, d.lease_id AS held, COALESCE(d.lease_until > now(), false) AS live, \
           d.horizon, d.final_snapshot, a.document_id AS amendments, a.folded_by, a.folded_seq \
         FROM channel_documents d \
         LEFT JOIN channel_documents a ON a.amends = d.document_id \
         WHERE d.document_id = $1 AND d.channel_id = $2 AND d.kind = 'segment' \
         FOR UPDATE OF d",
    )
    .bind(document)
    .bind(id_to_uuid(*channel_id))
    .fetch_optional(&mut *tx)
    .await?;
    let Some(row) = row.filter(|row| row.state == Some(ambre_core::SegmentState::Final)) else {
        return Err(conflict("the segment is not finalized"));
    };
    let Some(amendments) = row.amendments else {
        return Err(conflict("the segment has no amendments"));
    };
    if row.folded_by == Some(snapshot_id) && row.folded_seq == Some(amendments_seq) {
        return Ok(());
    }
    if row.final_snapshot.is_none() {
        return Err(conflict("the segment is not finalized"));
    }
    if row.held != Some(lease_id) || !row.live {
        return Err(conflict("the caller holds no live lease"));
    }
    if row.folded_by.is_some()
        && row
            .folded_seq
            .is_some_and(|folded| amendments_seq <= folded)
    {
        return Err(conflict("the fold reaches no further than the last one"));
    }
    let covered = covered_seq_of(&mut tx, snapshot_id, segment.document_id()).await?;
    let snapshot_id = id_to_uuid(snapshot_id);
    if covered.is_none() || covered != row.horizon {
        return Err(conflict("the snapshot does not cover the horizon"));
    }
    sqlx::query(
        "UPDATE channel_documents SET folded_by = $2, folded_seq = $3, folded_at = now() \
         WHERE document_id = $1",
    )
    .bind(id_to_uuid(amendments))
    .bind(snapshot_id)
    .bind(seq)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "UPDATE channel_documents SET lease_id = NULL, lease_until = NULL WHERE document_id = $1",
    )
    .bind(document)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

/// A fold a confirmation settled: the amendments document it folds, its last
/// delta number the fold holds, and the objects of the snapshots it replaced.
#[derive(Debug, PartialEq, Eq)]
pub struct SettledFold {
    pub amendments: ambre_core::DocumentId,
    pub folded_seq: ambre_core::Seq,
    pub dropped_keys: Vec<ambre_core::ObjectKey>,
}

/// Settle the fold `snapshot_id` makes of the amendments of the segment
/// `document_id`, once a member other than its author confirmed it
/// (`SRV-208`). The snapshot becomes the final one, and every other snapshot
/// of the segment goes, since each still holds what the amendments erase
/// (`SRV-209`). Answers the fold, `None` when the snapshot folds nothing.
pub async fn settle_fold(
    pool: &PgPool,
    document_id: &ambre_core::DocumentId,
    snapshot_id: &ambre_core::SnapshotId,
) -> Result<Option<SettledFold>, AppError> {
    let (document, snapshot) = (id_to_uuid(*document_id), id_to_uuid(*snapshot_id));
    let mut tx = pool.begin().await?;
    let fold: Option<(uuid::Uuid, Option<i64>)> = sqlx::query_as(
        "SELECT document_id, folded_seq FROM channel_documents \
         WHERE amends = $1 AND folded_by = $2 FOR UPDATE",
    )
    .bind(document)
    .bind(snapshot)
    .fetch_optional(&mut *tx)
    .await?;
    let Some((amendments, folded_seq)) = fold else {
        return Ok(None);
    };
    sqlx::query(
        "UPDATE channel_documents SET final_snapshot = $2, finalized_at = now() \
         WHERE document_id = $1",
    )
    .bind(document)
    .bind(snapshot)
    .execute(&mut *tx)
    .await?;
    let dropped_keys: Vec<String> = sqlx::query_scalar(
        "DELETE FROM snapshots WHERE document_id = $1 AND id <> $2 RETURNING s3_key",
    )
    .bind(document)
    .bind(snapshot)
    .fetch_all(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(Some(SettledFold {
        amendments: uuid_row_to_id(amendments),
        folded_seq: ambre_core::Seq(folded_seq.unwrap_or(0).unsigned_abs()),
        dropped_keys: dropped_keys
            .into_iter()
            .map(ambre_core::ObjectKey)
            .collect(),
    }))
}

/// The kind of a channel document, or `None` for a document outside every
/// channel.
pub async fn channel_document_kind(
    pool: &PgPool,
    document_id: &ambre_core::DocumentId,
) -> Result<Option<ChannelDocumentKind>, AppError> {
    let kind: Option<String> =
        sqlx::query_scalar("SELECT kind FROM channel_documents WHERE document_id = $1")
            .bind(id_to_uuid(*document_id))
            .fetch_optional(pool)
            .await?;
    kind.as_deref().map(decode_kind).transpose()
}

/// The channel a document belongs to and its kind there, in one read, or
/// `None` for a document outside every channel.
pub async fn channel_place(
    pool: &PgPool,
    document_id: &ambre_core::DocumentId,
) -> Result<Option<(ambre_core::ChannelId, ChannelDocumentKind)>, AppError> {
    let row: Option<(uuid::Uuid, String)> =
        sqlx::query_as("SELECT channel_id, kind FROM channel_documents WHERE document_id = $1")
            .bind(id_to_uuid(*document_id))
            .fetch_optional(pool)
            .await?;
    row.map(|(channel, kind)| Ok((uuid_row_to_id(channel), decode_kind(&kind)?)))
        .transpose()
}

/// Delete a channel, and mark the delete of every document it groups
/// (`SRV-142`). The caller empties them as a deleted document is emptied,
/// and the pass finishes any it misses (`SRV-165`). Answers the documents
/// marked, empty for an unknown channel.
pub async fn delete_channel(
    pool: &PgPool,
    channel_id: &ambre_core::ChannelId,
    sweep_due: ambre_core::UnixSeconds,
) -> Result<DeletedChannel, AppError> {
    let channel = id_to_uuid(*channel_id);
    let mut tx = pool.begin().await?;
    // `meta` first, as a removal locks it, so a delete racing the last
    // leave waits for it rather than deadlocking.
    let meta: Option<uuid::Uuid> = sqlx::query_scalar(
        "SELECT id FROM documents WHERE id = (SELECT document_id FROM channel_documents \
         WHERE channel_id = $1 AND kind = 'meta') FOR NO KEY UPDATE",
    )
    .bind(channel)
    .fetch_optional(&mut *tx)
    .await?;
    // Read under the lock on `meta`, which an added member waits for, so
    // none joins between this list and the delete.
    let members: Vec<uuid::Uuid> = sqlx::query_scalar(
        "SELECT user_id FROM document_members WHERE document_id = (SELECT document_id \
         FROM channel_documents WHERE channel_id = $1 AND kind = 'meta')",
    )
    .bind(channel)
    .fetch_all(&mut *tx)
    .await?;
    let documents = delete_channel_in(&mut tx, *channel_id, sweep_due).await?;
    tx.commit().await?;
    Ok(DeletedChannel {
        meta: meta.map(uuid_row_to_id),
        documents,
        members: members.into_iter().map(uuid_row_to_id).collect(),
    })
}

/// Deletes every channel of `space` inside `tx`, each as [`delete_channel`]
/// deletes one, in identifier order (`SRV-143`).
pub(super) async fn delete_space_channels_in(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    space: ambre_core::SpaceId,
    sweep_due: ambre_core::UnixSeconds,
) -> Result<Vec<DeletedChannel>, AppError> {
    let space = id_to_uuid(space);
    let channels: Vec<(uuid::Uuid, Option<uuid::Uuid>)> = sqlx::query_as(
        "SELECT c.id, d.document_id FROM channels c \
         LEFT JOIN channel_documents d ON d.channel_id = c.id AND d.kind = 'meta' \
         WHERE c.space_id = $1 ORDER BY c.id",
    )
    .bind(space)
    .fetch_all(&mut **tx)
    .await?;
    let mut deleted = Vec::with_capacity(channels.len());
    for (channel, meta) in channels {
        let channel: ambre_core::ChannelId = uuid_row_to_id(channel);
        let meta: Option<ambre_core::DocumentId> = meta.map(uuid_row_to_id);
        let members: Vec<uuid::Uuid> = match meta {
            Some(meta) => {
                super::members::lock_document(tx, meta).await?;
                sqlx::query_scalar("SELECT user_id FROM document_members WHERE document_id = $1")
                    .bind(id_to_uuid(meta))
                    .fetch_all(&mut **tx)
                    .await?
            }
            None => Vec::new(),
        };
        let documents = delete_channel_in(tx, channel, sweep_due).await?;
        deleted.push(DeletedChannel {
            meta,
            documents,
            members: members.into_iter().map(uuid_row_to_id).collect(),
        });
    }
    Ok(deleted)
}

/// What [`delete_channel`] removed.
pub struct DeletedChannel {
    /// Its `meta`, absent only for a channel whose `meta` was never
    /// registered.
    pub meta: Option<ambre_core::DocumentId>,
    /// Every document the channel grouped, marked for deletion.
    pub documents: Vec<ambre_core::DocumentId>,
    /// The members of its `meta`, as the delete found them.
    pub members: Vec<ambre_core::UserId>,
}

/// [`delete_channel`] inside `tx`.
///
/// The channel row is locked first. A document joining the channel takes a
/// key share on that row, so none lands between the listing and the delete.
/// The channel row goes now, and its `channel_documents` rows with it, so no
/// member reaches a document through the channel past the commit.
pub(super) async fn delete_channel_in(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    channel: ambre_core::ChannelId,
    sweep_due: ambre_core::UnixSeconds,
) -> Result<Vec<ambre_core::DocumentId>, AppError> {
    let channel = id_to_uuid(channel);
    let held: Option<uuid::Uuid> =
        sqlx::query_scalar("SELECT id FROM channels WHERE id = $1 FOR UPDATE")
            .bind(channel)
            .fetch_optional(&mut **tx)
            .await?;
    if held.is_none() {
        return Ok(Vec::new());
    }
    let documents: Vec<ambre_core::DocumentId> = sqlx::query_scalar::<_, PgId>(
        "SELECT document_id FROM channel_documents WHERE channel_id = $1",
    )
    .bind(channel)
    .fetch_all(&mut **tx)
    .await?
    .into_iter()
    .map(Into::into)
    .collect();
    super::documents::begin_deletions_in(tx, &documents, sweep_due).await?;
    sqlx::query("DELETE FROM channels WHERE id = $1")
        .bind(channel)
        .execute(&mut **tx)
        .await?;
    Ok(documents)
}

/// The `meta` of each private channel of `space` that holds a row of `user`.
/// A leave of the space would leave each of them owing a rotation.
pub async fn private_channels_of_member(
    pool: &PgPool,
    space: &ambre_core::DocumentId,
    user: &ambre_core::UserId,
) -> Result<Vec<ambre_core::DocumentId>, AppError> {
    let metas: Vec<uuid::Uuid> = sqlx::query_scalar(
        "SELECT d.document_id FROM channels c \
         JOIN channel_documents d ON d.channel_id = c.id AND d.kind = 'meta' \
         JOIN document_members m ON m.document_id = d.document_id AND m.user_id = $2 \
         WHERE c.space_id = $1 AND NOT c.is_public ORDER BY d.document_id",
    )
    .bind(id_to_uuid(*space))
    .bind(id_to_uuid(*user))
    .fetch_all(pool)
    .await?;
    Ok(metas.into_iter().map(uuid_row_to_id).collect())
}

/// Takes `user` out of every channel of `space` they are a member of, inside
/// `tx` and under the lock on the space document (`SRV-136`). `None` when
/// `space` is the space of no channel.
///
/// A channel that is not public owes a rotation, and goes with its documents
/// when its last member leaves (`SRV-142`). A public one owes nothing: the
/// rotation of the space covers it. The last-owner lockout holds for none of
/// them. The leave is the member's, and an owner of the space stands in for
/// the owner of each channel. Each `meta` is locked in identifier order,
/// after the space, as a self-join takes them.
pub(super) async fn leave_space_channels_in(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    space: ambre_core::SpaceId,
    user: ambre_core::UserId,
    sweep_due: ambre_core::UnixSeconds,
) -> Result<Option<super::SpaceLeave>, AppError> {
    let space = id_to_uuid(space);
    let user_typed = user;
    let user = id_to_uuid(user);
    let any: bool =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM channels WHERE space_id = $1)")
            .bind(space)
            .fetch_one(&mut **tx)
            .await?;
    if !any {
        return Ok(None);
    }
    let channels: Vec<(uuid::Uuid, uuid::Uuid, bool)> = sqlx::query_as(
        "SELECT c.id, d.document_id, c.is_public FROM channels c \
         JOIN channel_documents d ON d.channel_id = c.id AND d.kind = 'meta' \
         JOIN document_members m ON m.document_id = d.document_id AND m.user_id = $2 \
         WHERE c.space_id = $1 ORDER BY d.document_id",
    )
    .bind(space)
    .bind(user)
    .fetch_all(&mut **tx)
    .await?;
    let mut leave = super::SpaceLeave::default();
    for (channel, meta, public) in channels {
        let channel: ambre_core::ChannelId = uuid_row_to_id(channel);
        let meta_id: ambre_core::DocumentId = uuid_row_to_id(meta);
        super::members::lock_document(tx, meta_id).await?;
        let members: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM document_members WHERE document_id = $1")
                .bind(meta)
                .fetch_one(&mut **tx)
                .await?;
        super::members::drop_member_row(tx, meta_id, user_typed).await?;
        if members == 1 && !public {
            let documents = delete_channel_in(tx, channel, sweep_due).await?;
            leave.deleted.push((meta_id, documents));
            continue;
        }
        if !public {
            super::members::owe_rotation(tx, meta_id).await?;
            leave.owed.push(meta_id);
        }
        leave.left.push(meta_id);
    }
    Ok(Some(leave))
}

/// The channel whose `meta` is `document_id`, with its public flag. `None`
/// for any other document.
pub(super) async fn channel_of_meta(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    document_id: ambre_core::DocumentId,
) -> Result<Option<ChannelMeta>, AppError> {
    let document_id = id_to_uuid(document_id);
    let row: Option<(uuid::Uuid, bool)> = sqlx::query_as(
        "SELECT c.id, c.is_public FROM channel_documents d \
         JOIN channels c ON c.id = d.channel_id \
         WHERE d.document_id = $1 AND d.kind = 'meta'",
    )
    .bind(document_id)
    .fetch_optional(&mut **tx)
    .await?;
    Ok(row.map(|(channel, public)| ChannelMeta {
        id: uuid_row_to_id(channel),
        visibility: if public {
            ambre_core::Visibility::Public
        } else {
            ambre_core::Visibility::Private
        },
    }))
}

/// The channel whose `meta` a document is, with whether its space lists it.
#[derive(Debug, Clone, Copy)]
pub(super) struct ChannelMeta {
    pub(super) id: ambre_core::ChannelId,
    pub(super) visibility: ambre_core::Visibility,
}

/// Add `user_id` to the members of a channel's `meta`, unless a row is there.
///
/// Answers whether it wrote a row. An existing row is left as it is, so a
/// self-join never lowers an owner nor swaps the key of a member. The entry
/// the joining member signed lands with a new row alone, in one transaction
/// and under the document lock every membership change takes (`SRV-107`).
///
/// The membership of `space` is read again under a share lock on the space
/// document, taken before `meta`'s. A leave of the space holds that lock
/// while it empties every channel of it, so no join lands behind it
/// (`SRV-136`). A caller no longer a member is refused with a 403.
#[allow(clippy::too_many_arguments)]
pub async fn add_channel_member_if_absent(
    pool: &PgPool,
    space: &ambre_core::SpaceId,
    meta_id: &ambre_core::MetaId,
    user_id: &ambre_core::UserId,
    role: ambre_core::Role,
    doc_key_enc: &ambre_core::WrappedDocKey,
    entry: Option<&LogEntry>,
) -> Result<bool, AppError> {
    let uid = id_to_uuid(*user_id);
    let did = id_to_uuid(*meta_id);
    let space = id_to_uuid(space.document_id());
    let mut tx = pool.begin().await?;
    sqlx::query("SELECT 1 FROM documents WHERE id = $1 FOR SHARE")
        .bind(space)
        .fetch_optional(&mut *tx)
        .await?;
    let member: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM document_members WHERE document_id = $1 AND user_id = $2)",
    )
    .bind(space)
    .bind(uid)
    .fetch_one(&mut *tx)
    .await?;
    if !member {
        return Err(AppError::Forbidden);
    }
    super::members::lock_document(&mut tx, meta_id.document_id()).await?;
    let done = sqlx::query(
        "INSERT INTO document_members (document_id, user_id, role, doc_key_enc, invited_by) \
         VALUES ($1, $2, $3, $4, $2) ON CONFLICT (document_id, user_id) DO NOTHING",
    )
    .bind(did)
    .bind(uid)
    .bind(role.as_str())
    .bind(doc_key_enc.as_bytes())
    .execute(&mut *tx)
    .await?;
    let joined = done.rows_affected() == 1;
    if joined {
        super::members::record_change(&mut tx, meta_id.document_id(), entry).await?;
    }
    tx.commit().await?;
    Ok(joined)
}

/// A direct message to create: its documents, its members, and the log of
/// its `meta` (`SRV-175`).
pub struct NewDirectMessage<'a> {
    pub channel_id: ambre_core::ChannelId,
    pub meta_id: ambre_core::MetaId,
    pub segment_id: ambre_core::SegmentId,
    /// Who creates it: a member too, and the author of every entry.
    pub creator: ambre_core::UserId,
    /// Every member, the creator included, with the key wrapped for them.
    pub members: &'a [(ambre_core::UserId, ambre_core::WrappedDocKey)],
    /// The log of `meta`, in position order.
    pub entries: &'a [LogEntry],
}

/// What a direct message request found or made.
#[derive(Debug, PartialEq, Eq)]
pub enum DirectMessage {
    /// The relay made the direct message the request named.
    Created,
    /// A direct message of exactly these members was there already, so
    /// nothing was written (`SRV-176`).
    Existing(ambre_core::ChannelId),
}

/// Find the direct message of exactly `new.members`, or create it in one
/// transaction (`SRV-175`, `SRV-176`).
///
/// An advisory lock on the sorted member set holds the lookup and the
/// creation together. So two concurrent requests for one set meet there, and
/// the second finds the first. The new channel belongs to no space and is not
/// public. Every member gets an editor row on `meta`, so nobody owns it. The
/// segment gets no row: its access resolves to `meta` (`access_document!`).
pub async fn find_or_create_direct_message(
    pool: &PgPool,
    new: &NewDirectMessage<'_>,
) -> Result<DirectMessage, AppError> {
    let mut members: Vec<uuid::Uuid> = new.members.iter().map(|(u, _)| id_to_uuid(*u)).collect();
    members.sort_unstable();
    let lock_key = members
        .iter()
        .map(uuid::Uuid::to_string)
        .collect::<Vec<_>>()
        .join(",");
    let mut tx = pool.begin().await?;
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
        .bind(format!("direct:{lock_key}"))
        .execute(&mut *tx)
        .await?;
    let existing: Option<uuid::Uuid> = sqlx::query_scalar(
        "SELECT c.id FROM channels c \
         JOIN channel_documents d ON d.channel_id = c.id AND d.kind = 'meta' \
         WHERE c.space_id IS NULL AND (SELECT array_agg(m.user_id ORDER BY m.user_id) \
         FROM document_members m WHERE m.document_id = d.document_id) = $1 \
         ORDER BY c.id LIMIT 1",
    )
    .bind(&members)
    .fetch_optional(&mut *tx)
    .await?;
    if let Some(existing) = existing {
        return Ok(DirectMessage::Existing(uuid_row_to_id(existing)));
    }
    let conflict = || AppError::Conflict {
        detail: "the channel or one of its documents already exists".into(),
    };
    let creator = id_to_uuid(new.creator);
    let channel = id_to_uuid(new.channel_id);
    for document in [new.meta_id.document_id(), new.segment_id.document_id()] {
        let done = sqlx::query(
            "INSERT INTO documents (id, owner_id, doc_type) VALUES ($1, $2, 'murmure') \
             ON CONFLICT (id) DO NOTHING",
        )
        .bind(id_to_uuid(document))
        .bind(creator)
        .execute(&mut *tx)
        .await?;
        if done.rows_affected() != 1 {
            return Err(conflict());
        }
    }
    let done = sqlx::query(
        "INSERT INTO channels (id, space_id, is_public) VALUES ($1, NULL, false) \
         ON CONFLICT (id) DO NOTHING",
    )
    .bind(channel)
    .execute(&mut *tx)
    .await?;
    if done.rows_affected() != 1 {
        return Err(conflict());
    }
    for (document, kind) in [
        (new.meta_id.document_id(), ChannelDocumentKind::Meta),
        (new.segment_id.document_id(), ChannelDocumentKind::Segment),
    ] {
        sqlx::query(insert_channel_document!())
            .bind(id_to_uuid(document))
            .bind(channel)
            .bind(kind.as_str())
            .execute(&mut *tx)
            .await?;
    }
    let meta = id_to_uuid(new.meta_id);
    for (member, doc_key_enc) in new.members {
        sqlx::query(
            "INSERT INTO document_members (document_id, user_id, role, doc_key_enc, invited_by) \
             VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(meta)
        .bind(id_to_uuid(*member))
        .bind(ambre_core::Role::Editor.as_str())
        .bind(doc_key_enc.as_bytes())
        .bind(creator)
        .execute(&mut *tx)
        .await?;
    }
    for entry in new.entries {
        super::members::append_entry(&mut tx, new.meta_id.document_id(), entry).await?;
    }
    tx.commit().await?;
    Ok(DirectMessage::Created)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ambre_core::{ChannelId, DocumentId, MessageId, UserId};

    async fn user(pool: &PgPool, email: &str) -> UserId {
        crate::test_helpers::create_test_user(pool, email).await
    }

    /// A Murmure document owned by `owner`, with `owner` as its one member.
    async fn document(pool: &PgPool, owner: UserId) -> DocumentId {
        crate::test_helpers::create_test_document(pool, owner, "murmure").await
    }

    async fn channel(pool: &PgPool) -> ChannelId {
        let id = ChannelId::new();
        sqlx::query("INSERT INTO channels (id) VALUES ($1)")
            .bind(id_to_uuid(id))
            .execute(pool)
            .await
            .expect("insert channel");
        id
    }

    async fn group(
        pool: &PgPool,
        channel: ChannelId,
        document: DocumentId,
        kind: ChannelDocumentKind,
    ) {
        sqlx::query(insert_channel_document!())
            .bind(id_to_uuid(document))
            .bind(id_to_uuid(channel))
            .bind(kind.as_str())
            .execute(pool)
            .await
            .expect("group document");
    }

    async fn grant(pool: &PgPool, document: DocumentId, member: UserId, owner: UserId) {
        add_document_member(
            pool,
            &document,
            &member,
            ambre_core::Role::Editor,
            &ambre_core::WrappedDocKey::new([6u8; 32].to_vec()),
            &owner,
            None,
        )
        .await
        .expect("add member");
    }

    /// A channel of `owner`: its `meta` and one segment.
    async fn channel_of(pool: &PgPool, owner: UserId) -> (ChannelId, DocumentId, DocumentId) {
        let id = channel(pool).await;
        let meta = document(pool, owner).await;
        let segment = document(pool, owner).await;
        group(pool, id, meta, ChannelDocumentKind::Meta).await;
        group(pool, id, segment, ChannelDocumentKind::Segment).await;
        (id, meta, segment)
    }

    /// `SRV-103`: a member of the channel reaches each document it groups,
    /// with the role and the key of the `meta` document.
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn channel_membership_reaches_every_document_it_groups(pool: PgPool) {
        let owner = user(&pool, "owner@test.dev").await;
        let bob = user(&pool, "bob@test.dev").await;
        let (_, meta, segment) = channel_of(&pool, owner).await;
        grant(&pool, meta, bob, owner).await;

        let row = get_document_member(&pool, &segment, &bob)
            .await
            .expect("query")
            .expect("a member of the channel reaches its segment");
        assert_eq!(row.role, ambre_core::Role::Editor);
        assert_eq!(
            row.doc_key_enc,
            ambre_core::WrappedDocKey::new([6u8; 32].to_vec()),
            "the key sealed on `meta`"
        );

        let listed: Vec<ambre_core::UserId> = list_document_members(&pool, &segment)
            .await
            .expect("query")
            .into_iter()
            .map(|m| m.user_id)
            .collect();
        assert_eq!(listed.len(), 2, "the segment lists the members of `meta`");
        assert!(listed.contains(&bob));
    }

    /// `SRV-103`: membership reaches no document of another channel, and a
    /// segment is closed to a caller who is no member of its channel.
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn channel_membership_reaches_no_other_document(pool: PgPool) {
        let owner = user(&pool, "owner@test.dev").await;
        let bob = user(&pool, "bob@test.dev").await;
        let (_, meta, _) = channel_of(&pool, owner).await;
        let (_, _, other_segment) = channel_of(&pool, owner).await;
        grant(&pool, meta, bob, owner).await;

        assert!(
            get_document_member(&pool, &other_segment, &bob)
                .await
                .expect("query")
                .is_none(),
            "a member of one channel reaches no segment of another"
        );
        let outsider = user(&pool, "eve@test.dev").await;
        let (_, _, segment) = channel_of(&pool, owner).await;
        assert!(
            get_document_member(&pool, &segment, &outsider)
                .await
                .expect("query")
                .is_none()
        );
    }

    /// `SRV-103`: a member row on a segment itself grants nothing. Only the
    /// `meta` document decides.
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn a_member_row_on_a_segment_grants_nothing(pool: PgPool) {
        let owner = user(&pool, "owner@test.dev").await;
        let eve = user(&pool, "eve@test.dev").await;
        let (_, _, segment) = channel_of(&pool, owner).await;
        grant(&pool, segment, eve, owner).await;

        assert!(
            get_document_member(&pool, &segment, &eve)
                .await
                .expect("query")
                .is_none(),
            "a stray row on a segment must not open it"
        );
        let listed = list_document_members(&pool, &segment).await.expect("query");
        assert!(listed.iter().all(|m| m.user_id != eve));
    }

    /// A channel with no `meta` opens none of its documents, even to the
    /// owner of the document itself.
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn a_channel_with_no_meta_opens_nothing(pool: PgPool) {
        let owner = user(&pool, "owner@test.dev").await;
        let id = channel(&pool).await;
        let segment = document(&pool, owner).await;
        group(&pool, id, segment, ChannelDocumentKind::Segment).await;
        assert!(
            get_document_member(&pool, &segment, &owner)
                .await
                .expect("query")
                .is_none()
        );
    }

    /// A document outside every channel keeps its own member rows.
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn a_document_outside_a_channel_keeps_its_own_members(pool: PgPool) {
        let owner = user(&pool, "owner@test.dev").await;
        let doc = document(&pool, owner).await;
        let row = get_document_member(&pool, &doc, &owner)
            .await
            .expect("query")
            .expect("the owner of an ordinary document");
        assert_eq!(row.role, ambre_core::Role::Owner);
    }

    async fn states(pool: &PgPool, channel: ChannelId) -> Vec<(DocumentId, String)> {
        let rows: Vec<(uuid::Uuid, String)> = sqlx::query_as(
            "SELECT document_id, state FROM channel_documents \
             WHERE channel_id = $1 AND kind = 'segment' ORDER BY state",
        )
        .bind(id_to_uuid(channel))
        .fetch_all(pool)
        .await
        .expect("query");
        rows.into_iter()
            .map(|(d, s)| (uuid_row_to_id(d), s))
            .collect()
    }

    fn segment(document: DocumentId) -> ambre_core::SegmentId {
        ambre_core::SegmentId::from_document(document)
    }

    /// `SRV-194`, `SRV-197`: a close marks the active segment closing and
    /// registers its successor as active, in one transaction. Another
    /// channel keeps its segments.
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn a_close_registers_the_successor_and_marks_the_segment_closing(pool: PgPool) {
        let owner = user(&pool, "owner@test.dev").await;
        let (id, _, first) = channel_of(&pool, owner).await;
        let (other, _, other_first) = channel_of(&pool, owner).await;
        let next = document(&pool, owner).await;
        let active = close_segment(&pool, &id, segment(first), segment(next))
            .await
            .expect("close");
        assert_eq!(active, segment(next));
        assert_eq!(
            states(&pool, id).await,
            [(next, "active".to_owned()), (first, "closing".to_owned())]
        );
        assert_eq!(
            channel_document_kind(&pool, &next).await.expect("query"),
            Some(ChannelDocumentKind::Segment)
        );
        assert_eq!(
            states(&pool, other).await,
            [(other_first, "active".to_owned())]
        );
    }

    /// `SRV-198`: a close asked again with the successor it registered is
    /// answered that successor, and registers nothing more.
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn a_close_asked_again_answers_its_own_successor(pool: PgPool) {
        let owner = user(&pool, "owner@test.dev").await;
        let (id, _, first) = channel_of(&pool, owner).await;
        let next = document(&pool, owner).await;
        for _ in 0..2 {
            let active = close_segment(&pool, &id, segment(first), segment(next))
                .await
                .expect("close");
            assert_eq!(active, segment(next));
        }
        assert_eq!(
            states(&pool, id).await,
            [(next, "active".to_owned()), (first, "closing".to_owned())]
        );
    }

    /// `SRV-195`: two closes of one segment race, and both answer the one
    /// successor that was registered.
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn two_racing_closes_register_one_successor(pool: PgPool) {
        let owner = user(&pool, "owner@test.dev").await;
        let (id, _, first) = channel_of(&pool, owner).await;
        let (a, b) = (document(&pool, owner).await, document(&pool, owner).await);
        let (left, right) = tokio::join!(
            close_segment(&pool, &id, segment(first), segment(a)),
            close_segment(&pool, &id, segment(first), segment(b)),
        );
        let (left, right) = (left.expect("close"), right.expect("close"));
        assert_eq!(left, right, "both closes answer the same successor");
        let winner = left.document_id();
        assert!(winner == a || winner == b);
        assert_eq!(
            states(&pool, id).await,
            [(winner, "active".to_owned()), (first, "closing".to_owned())],
            "the losing successor is never registered"
        );
    }

    /// Lock `segment` as a first finalize lease would, at `horizon`.
    async fn lock(pool: &PgPool, segment: DocumentId, horizon: i64) {
        sqlx::query("UPDATE channel_documents SET horizon = $2 WHERE document_id = $1")
            .bind(id_to_uuid(segment))
            .bind(horizon)
            .execute(pool)
            .await
            .expect("lock segment");
    }

    /// The amendments documents of a channel, with the segment each amends.
    async fn amended(pool: &PgPool, channel: ChannelId) -> Vec<(DocumentId, DocumentId)> {
        let rows: Vec<(uuid::Uuid, uuid::Uuid)> = sqlx::query_as(
            "SELECT document_id, amends FROM channel_documents \
             WHERE channel_id = $1 AND kind = 'amendments'",
        )
        .bind(id_to_uuid(channel))
        .fetch_all(pool)
        .await
        .expect("amendments");
        rows.into_iter()
            .map(|(d, s)| (uuid_row_to_id(d), uuid_row_to_id(s)))
            .collect()
    }

    /// `SRV-201`: a locked segment takes one amendments document, which the
    /// channel then groups. Asked again, it answers that document.
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn a_locked_segment_registers_one_amendments_document(pool: PgPool) {
        let owner = user(&pool, "owner@test.dev").await;
        let (id, _, first) = channel_of(&pool, owner).await;
        let next = document(&pool, owner).await;
        close_segment(&pool, &id, segment(first), segment(next))
            .await
            .expect("close");
        lock(&pool, first, 7).await;
        let amendments = document(&pool, owner).await;
        for _ in 0..2 {
            let held = register_amendments(&pool, &id, segment(first), amendments)
                .await
                .expect("register");
            assert_eq!(held, amendments);
        }
        assert_eq!(amended(&pool, id).await, [(amendments, first)]);
        assert_eq!(
            states(&pool, id).await,
            [(next, "active".to_owned()), (first, "closing".to_owned())],
            "the amended segment keeps its state"
        );
        let horizon: Option<i64> =
            sqlx::query_scalar("SELECT horizon FROM channel_documents WHERE document_id = $1")
                .bind(id_to_uuid(first))
                .fetch_one(&pool)
                .await
                .expect("horizon");
        assert_eq!(horizon, Some(7), "and its lock");
        let kinds: Vec<_> = list_channel_documents(&pool, &id)
            .await
            .expect("listing")
            .into_iter()
            .filter(|d| d.document_id == amendments)
            .map(|d| d.kind)
            .collect();
        assert_eq!(kinds, [ChannelDocumentKind::Amendments]);
    }

    /// `SRV-201`: a segment not yet locked, or of another channel, takes no
    /// amendments document, and nothing is registered.
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn an_unlocked_or_foreign_segment_registers_no_amendments(pool: PgPool) {
        let owner = user(&pool, "owner@test.dev").await;
        let (id, _, first) = channel_of(&pool, owner).await;
        let (other, _, foreign) = channel_of(&pool, owner).await;
        lock(&pool, foreign, 3).await;
        let amendments = document(&pool, owner).await;
        let open = register_amendments(&pool, &id, segment(first), amendments).await;
        assert!(matches!(open, Err(AppError::Conflict { .. })), "{open:?}");
        let elsewhere = register_amendments(&pool, &id, segment(foreign), amendments).await;
        assert!(
            matches!(elsewhere, Err(AppError::Conflict { .. })),
            "{elsewhere:?}"
        );
        let unknown =
            register_amendments(&pool, &ChannelId::new(), segment(foreign), amendments).await;
        assert!(matches!(unknown, Err(AppError::NotFound { .. })));
        assert!(amended(&pool, id).await.is_empty());
        assert!(amended(&pool, other).await.is_empty());
    }

    /// `SRV-202`: two registrations for one segment race, and both answer the
    /// one document that was registered.
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn two_racing_registrations_keep_one_amendments_document(pool: PgPool) {
        let owner = user(&pool, "owner@test.dev").await;
        let (id, _, first) = channel_of(&pool, owner).await;
        lock(&pool, first, 1).await;
        let (a, b) = (document(&pool, owner).await, document(&pool, owner).await);
        let (left, right) = tokio::join!(
            register_amendments(&pool, &id, segment(first), a),
            register_amendments(&pool, &id, segment(first), b),
        );
        let (left, right) = (left.expect("register"), right.expect("register"));
        assert_eq!(left, right, "both answer the same document");
        assert!(left == a || left == b);
        assert_eq!(amended(&pool, id).await, [(left, first)]);
    }

    /// Every row of the channel that is not a thread, each column read.
    async fn rows_beside_threads(pool: &PgPool, channel: ChannelId) -> Vec<String> {
        sqlx::query_scalar(
            "SELECT to_jsonb(d)::text FROM channel_documents d \
             WHERE channel_id = $1 AND kind <> 'thread' ORDER BY document_id",
        )
        .bind(id_to_uuid(channel))
        .fetch_all(pool)
        .await
        .expect("query")
    }

    /// `SRV-213`, `SRV-215`: a root takes one thread document, which the
    /// channel then groups. Asked again, it answers that document. Another
    /// root of the same channel takes its own. No other row moves.
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn a_root_registers_one_thread_document(pool: PgPool) {
        let owner = user(&pool, "owner@test.dev").await;
        let (id, _, _) = channel_of(&pool, owner).await;
        let before = rows_beside_threads(&pool, id).await;
        let (root, other) = (MessageId::new(), MessageId::new());
        let thread = document(&pool, owner).await;
        for _ in 0..2 {
            let held = register_thread(&pool, &id, root, thread)
                .await
                .expect("register");
            assert_eq!(held, thread);
        }
        let second = document(&pool, owner).await;
        let held = register_thread(&pool, &id, other, second)
            .await
            .expect("register");
        assert_eq!(held, second);
        let threads: Vec<_> = list_channel_documents(&pool, &id)
            .await
            .expect("listing")
            .into_iter()
            .filter(|d| d.kind == ChannelDocumentKind::Thread)
            .map(|d| d.document_id)
            .collect();
        assert_eq!(threads.len(), 2);
        assert!(threads.contains(&thread) && threads.contains(&second));
        assert_eq!(rows_beside_threads(&pool, id).await, before);
        let unknown = register_thread(&pool, &ChannelId::new(), root, thread).await;
        assert!(matches!(unknown, Err(AppError::NotFound { .. })));
    }

    /// `SRV-214`: two registrations for one root race, and both answer the
    /// one document that was registered.
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn two_racing_registrations_keep_one_thread_document(pool: PgPool) {
        let owner = user(&pool, "owner@test.dev").await;
        let (id, _, _) = channel_of(&pool, owner).await;
        let root = MessageId::new();
        let (a, b) = (document(&pool, owner).await, document(&pool, owner).await);
        let (left, right) = tokio::join!(
            register_thread(&pool, &id, root, a),
            register_thread(&pool, &id, root, b),
        );
        let (left, right) = (left.expect("register"), right.expect("register"));
        assert_eq!(left, right, "both answer the same document");
        assert!(left == a || left == b);
        let threads = list_channel_documents(&pool, &id)
            .await
            .expect("listing")
            .into_iter()
            .filter(|d| d.kind == ChannelDocumentKind::Thread)
            .count();
        assert_eq!(threads, 1);
    }

    /// `SRV-196`: a close while another segment of the channel is closing is
    /// refused. So is a close of a segment that is not the active one.
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn a_close_while_a_segment_is_closing_is_refused(pool: PgPool) {
        let owner = user(&pool, "owner@test.dev").await;
        let (id, _, first) = channel_of(&pool, owner).await;
        let (second, third) = (document(&pool, owner).await, document(&pool, owner).await);
        close_segment(&pool, &id, segment(first), segment(second))
            .await
            .expect("close");
        let refused = close_segment(&pool, &id, segment(second), segment(third)).await;
        assert!(
            matches!(refused, Err(AppError::Conflict { .. })),
            "{refused:?}"
        );
        let stale = close_segment(&pool, &id, segment(third), segment(first)).await;
        assert!(matches!(stale, Err(AppError::Conflict { .. })), "{stale:?}");
        assert_eq!(states(&pool, id).await.len(), 2, "nothing was registered");
        let unknown = close_segment(&pool, &ChannelId::new(), segment(first), segment(third)).await;
        assert!(matches!(unknown, Err(AppError::NotFound { .. })));
    }

    /// The database holds one active and one closing segment per channel,
    /// whatever writes it.
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn a_channel_holds_one_active_segment(pool: PgPool) {
        let owner = user(&pool, "owner@test.dev").await;
        let (id, _, _) = channel_of(&pool, owner).await;
        let second = document(&pool, owner).await;
        let refused = sqlx::query(insert_channel_document!())
            .bind(id_to_uuid(second))
            .bind(id_to_uuid(id))
            .bind(ChannelDocumentKind::Segment.as_str())
            .execute(&pool)
            .await;
        assert!(refused.is_err(), "a second active segment is refused");
    }

    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn the_listing_puts_meta_first(pool: PgPool) {
        let owner = user(&pool, "owner@test.dev").await;
        let (id, meta, segment) = channel_of(&pool, owner).await;
        let listed = list_channel_documents(&pool, &id).await.expect("query");
        let got: Vec<_> = listed.iter().map(|r| (r.document_id, r.kind)).collect();
        assert_eq!(
            got,
            [
                (meta, ChannelDocumentKind::Meta),
                (segment, ChannelDocumentKind::Segment)
            ]
        );
        assert!(
            list_channel_documents(&pool, &ChannelId::new())
                .await
                .expect("query")
                .is_empty()
        );
    }

    /// `ZK-015`: the channel tables hold the channel's space and public flag,
    /// each document's channel and kind, and the state of each segment. A
    /// segment adds its close time, its finalize lease, and the horizon, time
    /// and snapshot of its finalization, and whether a rotation retired its
    /// key. An amendments document names the
    /// segment it amends, and the snapshot, reach and time of its fold. The
    /// cursor table holds a member, a
    /// document and a number, and the space table a document alone.
    /// Nothing else: no name, no member and no key. A
    /// column added here widens what the relay learns, so it fails this test
    /// before it ships.
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn the_channel_tables_hold_only_what_the_relay_may_learn(pool: PgPool) {
        let columns: Vec<(String, String)> = sqlx::query_as(
            "SELECT table_name::text, column_name::text FROM information_schema.columns \
             WHERE table_schema = current_schema() \
             AND (table_name LIKE 'channel%' OR table_name IN ('read_cursors', 'spaces')) \
             ORDER BY table_name, column_name",
        )
        .fetch_all(&pool)
        .await
        .expect("query");
        let got: Vec<(&str, &str)> = columns
            .iter()
            .map(|(t, c)| (t.as_str(), c.as_str()))
            .collect();
        assert_eq!(
            got,
            [
                ("channel_documents", "amends"),
                ("channel_documents", "channel_id"),
                ("channel_documents", "closed_at"),
                ("channel_documents", "document_id"),
                ("channel_documents", "final_snapshot"),
                ("channel_documents", "finalized_at"),
                ("channel_documents", "folded_at"),
                ("channel_documents", "folded_by"),
                ("channel_documents", "folded_seq"),
                ("channel_documents", "horizon"),
                ("channel_documents", "key_retired"),
                ("channel_documents", "kind"),
                ("channel_documents", "lease_id"),
                ("channel_documents", "lease_until"),
                ("channel_documents", "state"),
                ("channel_documents", "thread_root"),
                ("channels", "id"),
                ("channels", "is_public"),
                ("channels", "space_id"),
                ("read_cursors", "document_id"),
                ("read_cursors", "seq"),
                ("read_cursors", "user_id"),
                ("spaces", "id"),
            ]
        );
    }
}
