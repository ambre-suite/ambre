// ambre-server/src/db/channels.rs
// The channel register: which documents a channel groups, and their kind
// (`ambre-spec-server.md section 3`). Access to each of them is the member
// rows of the channel's `meta` document, resolved in `members.rs`.

use super::*;

/// One document a channel groups.
#[derive(Debug)]
pub struct ChannelDocumentRow {
    pub document_id: uuid::Uuid,
    /// `meta`, `segment`, `amendments` or `thread`.
    pub kind: String,
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
            Ok(ChannelDocumentRow {
                document_id: r.try_get("document_id")?,
                kind: r.try_get("kind")?,
            })
        })
        .collect()
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

    async fn group(pool: &PgPool, channel: ChannelId, document: DocumentId, kind: &str) {
        sqlx::query(
            "INSERT INTO channel_documents (document_id, channel_id, kind) VALUES ($1, $2, $3)",
        )
        .bind(id_to_uuid(document))
        .bind(id_to_uuid(channel))
        .bind(kind)
        .execute(pool)
        .await
        .expect("group document");
    }

    async fn grant(pool: &PgPool, document: DocumentId, member: UserId, owner: UserId) {
        add_document_member(pool, &document, &member, "editor", &[6u8; 32], &owner)
            .await
            .expect("add member");
    }

    /// A channel of `owner`: its `meta` and one segment.
    async fn channel_of(pool: &PgPool, owner: UserId) -> (ChannelId, DocumentId, DocumentId) {
        let id = channel(pool).await;
        let meta = document(pool, owner).await;
        let segment = document(pool, owner).await;
        group(pool, id, meta, "meta").await;
        group(pool, id, segment, "segment").await;
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
        assert_eq!(row.role, "editor");
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
        group(&pool, id, segment, "segment").await;
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
        assert_eq!(row.role, "owner");
    }

    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn the_listing_puts_meta_first(pool: PgPool) {
        let owner = user(&pool, "owner@test.dev").await;
        let (id, meta, segment) = channel_of(&pool, owner).await;
        let listed = list_channel_documents(&pool, &id).await.expect("query");
        let got: Vec<(uuid::Uuid, &str)> = listed
            .iter()
            .map(|r| (r.document_id, r.kind.as_str()))
            .collect();
        assert_eq!(
            got,
            [(id_to_uuid(meta), "meta"), (id_to_uuid(segment), "segment")]
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
