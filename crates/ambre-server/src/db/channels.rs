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
fn decode_kind(word: &str) -> Result<ambre_core::ChannelDocumentKind, AppError> {
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
}

/// Every channel whose `meta` holds a row of `user`, by identifier
/// (`SRV-191`).
pub async fn list_member_channels(
    pool: &PgPool,
    user: &ambre_core::UserId,
) -> Result<Vec<MemberChannelRow>, AppError> {
    let rows = sqlx::query(
        "SELECT c.id, c.space_id, c.is_public FROM channels c \
         JOIN channel_documents d ON d.channel_id = c.id AND d.kind = 'meta' \
         JOIN document_members m ON m.document_id = d.document_id \
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
            resource: "channel",
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
    sqlx::query("UPDATE channel_documents SET state = 'closing' WHERE document_id = $1")
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

/// Delete a channel, and mark the delete of every document it groups
/// (`SRV-142`). The caller empties them as a deleted document is emptied,
/// and the pass finishes any it misses (`SRV-165`). Answers the documents
/// marked, empty for an unknown channel.
pub async fn delete_channel(
    pool: &PgPool,
    channel_id: &ambre_core::ChannelId,
    sweep_due: i64,
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
    let documents = delete_channel_in(&mut tx, channel, sweep_due).await?;
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
    space: uuid::Uuid,
    sweep_due: i64,
) -> Result<Vec<DeletedChannel>, AppError> {
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
        let members: Vec<uuid::Uuid> = match meta {
            Some(meta) => {
                super::members::lock_document(tx, meta).await?;
                sqlx::query_scalar("SELECT user_id FROM document_members WHERE document_id = $1")
                    .bind(meta)
                    .fetch_all(&mut **tx)
                    .await?
            }
            None => Vec::new(),
        };
        let documents = delete_channel_in(tx, channel, sweep_due).await?;
        deleted.push(DeletedChannel {
            meta: meta.map(uuid_row_to_id),
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
    channel: uuid::Uuid,
    sweep_due: i64,
) -> Result<Vec<ambre_core::DocumentId>, AppError> {
    let held: Option<uuid::Uuid> =
        sqlx::query_scalar("SELECT id FROM channels WHERE id = $1 FOR UPDATE")
            .bind(channel)
            .fetch_optional(&mut **tx)
            .await?;
    if held.is_none() {
        return Ok(Vec::new());
    }
    let documents: Vec<uuid::Uuid> =
        sqlx::query_scalar("SELECT document_id FROM channel_documents WHERE channel_id = $1")
            .bind(channel)
            .fetch_all(&mut **tx)
            .await?;
    super::documents::begin_deletions_in(tx, &documents, sweep_due).await?;
    sqlx::query("DELETE FROM channels WHERE id = $1")
        .bind(channel)
        .execute(&mut **tx)
        .await?;
    Ok(documents.into_iter().map(uuid_row_to_id).collect())
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
    space: uuid::Uuid,
    user: uuid::Uuid,
    sweep_due: i64,
) -> Result<Option<super::SpaceLeave>, AppError> {
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
        super::members::lock_document(tx, meta).await?;
        let members: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM document_members WHERE document_id = $1")
                .bind(meta)
                .fetch_one(&mut **tx)
                .await?;
        super::members::drop_member_row(tx, meta, user).await?;
        let meta_id = uuid_row_to_id(meta);
        if members == 1 && !public {
            let documents = delete_channel_in(tx, channel, sweep_due).await?;
            leave.deleted.push((meta_id, documents));
            continue;
        }
        if !public {
            super::members::owe_rotation(tx, meta).await?;
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
    document_id: uuid::Uuid,
) -> Result<Option<(uuid::Uuid, bool)>, AppError> {
    Ok(sqlx::query_as(
        "SELECT c.id, c.is_public FROM channel_documents d \
         JOIN channels c ON c.id = d.channel_id \
         WHERE d.document_id = $1 AND d.kind = 'meta'",
    )
    .bind(document_id)
    .fetch_optional(&mut **tx)
    .await?)
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
    doc_key_enc: &[u8],
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
    super::members::lock_document(&mut tx, did).await?;
    let done = sqlx::query(
        "INSERT INTO document_members (document_id, user_id, role, doc_key_enc, invited_by) \
         VALUES ($1, $2, $3, $4, $2) ON CONFLICT (document_id, user_id) DO NOTHING",
    )
    .bind(did)
    .bind(uid)
    .bind(role.as_str())
    .bind(doc_key_enc)
    .execute(&mut *tx)
    .await?;
    let joined = done.rows_affected() == 1;
    if joined {
        super::members::record_change(&mut tx, did, entry).await?;
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
    pub members: &'a [(ambre_core::UserId, Vec<u8>)],
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
        .bind(doc_key_enc)
        .bind(creator)
        .execute(&mut *tx)
        .await?;
    }
    for entry in new.entries {
        super::members::append_entry(&mut tx, meta, entry).await?;
    }
    tx.commit().await?;
    Ok(DirectMessage::Created)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ambre_core::{ChannelId, DocumentId, UserId};

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
            &[6u8; 32],
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
        assert_eq!(row.doc_key_enc, [6u8; 32], "the key sealed on `meta`");

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
    /// each document's channel and kind, and the state of each segment. The
    /// cursor table holds a member,
    /// a document and a number. Nothing else: no name, no member,
    /// no key and no timestamp. A column added here widens what the relay
    /// learns, so it fails this test before it ships.
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn the_channel_tables_hold_only_what_the_relay_may_learn(pool: PgPool) {
        let columns: Vec<(String, String)> = sqlx::query_as(
            "SELECT table_name::text, column_name::text FROM information_schema.columns \
             WHERE table_schema = current_schema() AND (table_name LIKE 'channel%' OR table_name = 'read_cursors') \
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
                ("channel_documents", "channel_id"),
                ("channel_documents", "document_id"),
                ("channel_documents", "kind"),
                ("channel_documents", "state"),
                ("channels", "id"),
                ("channels", "is_public"),
                ("channels", "space_id"),
                ("read_cursors", "document_id"),
                ("read_cursors", "seq"),
                ("read_cursors", "user_id"),
            ]
        );
    }
}
