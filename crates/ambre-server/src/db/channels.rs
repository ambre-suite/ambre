// ambre-server/src/db/channels.rs
// The channel register: which documents a channel groups, and their kind
// (`ambre-spec-server.md section 3`). Access to each of them is the member
// rows of the channel's `meta` document, resolved in `members.rs`.

use super::*;
use ambre_core::ChannelDocumentKind;

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
        sqlx::query(
            "INSERT INTO channel_documents (document_id, channel_id, kind) VALUES ($1, $2, $3) \
             ON CONFLICT DO NOTHING",
        )
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

/// Delete a channel, and mark the delete of every document it groups
/// (`SRV-142`). The caller empties them as a deleted document is emptied,
/// and the pass finishes any it misses (`SRV-165`). Answers the documents
/// marked, empty for an unknown channel.
pub async fn delete_channel(
    pool: &PgPool,
    channel_id: &ambre_core::ChannelId,
    sweep_due: i64,
) -> Result<Vec<ambre_core::DocumentId>, AppError> {
    let channel = id_to_uuid(*channel_id);
    let mut tx = pool.begin().await?;
    // `meta` first, as a removal locks it, so a delete racing the last
    // leave waits for it rather than deadlocking.
    sqlx::query(
        "SELECT 1 FROM documents WHERE id = (SELECT document_id FROM channel_documents \
         WHERE channel_id = $1 AND kind = 'meta') FOR NO KEY UPDATE",
    )
    .bind(channel)
    .fetch_optional(&mut *tx)
    .await?;
    let deleted = delete_channel_in(&mut tx, channel, sweep_due).await?;
    tx.commit().await?;
    Ok(deleted)
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
pub async fn add_channel_member_if_absent(
    pool: &PgPool,
    meta_id: &ambre_core::MetaId,
    user_id: &ambre_core::UserId,
    role: ambre_core::Role,
    doc_key_enc: &[u8],
    entry: Option<&LogEntry>,
) -> Result<bool, AppError> {
    let uid = id_to_uuid(*user_id);
    let did = id_to_uuid(*meta_id);
    let mut tx = pool.begin().await?;
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
        sqlx::query(
            "INSERT INTO channel_documents (document_id, channel_id, kind) VALUES ($1, $2, $3)",
        )
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

        let listed: Vec<uuid::Uuid> = list_document_members(&pool, &segment)
            .await
            .expect("query")
            .into_iter()
            .map(|m| m.user_id)
            .collect();
        assert_eq!(listed.len(), 2, "the segment lists the members of `meta`");
        assert!(listed.contains(&id_to_uuid(bob)));
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
        assert!(listed.iter().all(|m| m.user_id != id_to_uuid(eve)));
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
    /// and each document's channel and kind. The cursor table holds a member,
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
