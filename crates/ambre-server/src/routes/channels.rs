// ambre-server/src/routes/channels.rs
// The channel routes (`ambre-spec-server.md section 4`). They list the
// documents a channel groups and register a public channel in its space. A
// member of that space joins a public channel alone.

use ambre_core::Role;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use std::sync::Arc;

use crate::auth;
use crate::db;
use crate::error::AppError;
use crate::state::AppState;

/// One document of a channel, as the listing answers it.
#[derive(serde::Serialize)]
pub struct ChannelDocumentJson {
    pub document_id: String,
    pub kind: &'static str,
}

/// `GET /api/channels/{id}/documents`: the documents the channel groups, with
/// their kind, to a member of the channel alone (`SRV-103`).
///
/// An unknown channel and a channel the caller is no member of answer the
/// same 403. So the route tells a stranger nothing of which channels exist.
pub async fn list_channel_documents(
    State(state): State<Arc<AppState>>,
    auth: auth::AuthSession,
    Path(channel_id): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    let channel_id = ambre_core::Uuid::parse_str(&channel_id)
        .map(ambre_core::ChannelId::from_uuid)
        .map_err(|_| AppError::Forbidden)?;
    let documents = db::list_channel_documents(&state.db, &channel_id).await?;
    let meta = documents
        .iter()
        .find(|d| d.kind == ambre_core::ChannelDocumentKind::Meta)
        .ok_or(AppError::Forbidden)?;
    db::get_document_member(&state.db, &meta.document_id, &auth.user_id)
        .await?
        .ok_or(AppError::Forbidden)?;
    let json: Vec<_> = documents
        .into_iter()
        .map(|d| ChannelDocumentJson {
            document_id: d.document_id.to_string(),
            kind: d.kind.as_str(),
        })
        .collect();
    Ok(axum::Json(serde_json::json!({ "documents": json })))
}

/// The body of a public channel registration. Every id is one the client
/// generated, and both documents already exist on the relay.
#[derive(serde::Deserialize)]
pub struct CreateChannelRequest {
    pub channel_id: String,
    pub space_id: String,
    pub meta_id: String,
    pub segment_id: String,
}

/// The body of a self-join: the channel key, wrapped for the caller.
#[derive(serde::Deserialize)]
pub struct JoinChannelRequest {
    pub doc_key_enc: String,
}

/// Parse an id of the path or the body. A malformed id answers the 403 of an
/// id the caller may not use, so it tells nothing apart.
fn parse<T: From<ambre_core::Uuid>>(s: &str) -> Result<T, AppError> {
    ambre_core::Uuid::parse_str(s)
        .map(T::from)
        .map_err(|_| AppError::Forbidden)
}

/// A Murmure document outside every channel but `channel`, or a 403.
async fn free_murmure_document(
    state: &AppState,
    document_id: &ambre_core::DocumentId,
    channel: Option<ambre_core::ChannelId>,
) -> Result<db::DocumentRow, AppError> {
    let document = db::get_document(&state.db, document_id)
        .await?
        .filter(|d| d.doc_type == "murmure")
        .ok_or(AppError::Forbidden)?;
    let held = db::channel_of_document(&state.db, document_id).await?;
    if held.is_some() && held != channel {
        return Err(AppError::Forbidden);
    }
    Ok(document)
}

/// `POST /api/channels`: register a public channel of a space, with its `meta`
/// and its first segment.
///
/// The caller owns both documents, and administers the space: an owner or an
/// editor of its document. A viewer changes no catalogue, so it registers no
/// public channel. The members of `meta` become the members of the channel
/// (`SRV-103`), and the space document gains no row.
pub async fn create_channel(
    State(state): State<Arc<AppState>>,
    auth: auth::AuthSession,
    axum::Json(req): axum::Json<CreateChannelRequest>,
) -> Result<impl IntoResponse, AppError> {
    let new = db::NewChannel {
        channel_id: parse(&req.channel_id)?,
        space_id: parse(&req.space_id)?,
        is_public: true,
        meta_id: parse(&req.meta_id)?,
        segment_id: parse(&req.segment_id)?,
    };
    let (space, meta, segment) = (
        new.space_id.document_id(),
        new.meta_id.document_id(),
        new.segment_id.document_id(),
    );
    if space == meta || space == segment || meta == segment {
        return Err(AppError::Forbidden);
    }
    free_murmure_document(&state, &space, None).await?;
    let space_role = db::member_role(&state.db, &space, &auth.user_id)
        .await?
        .ok_or(AppError::Forbidden)?;
    if !space_role.can_write_document() {
        return Err(AppError::Forbidden);
    }
    let caller = uuid::Uuid::from_bytes(*ambre_core::Uuid::from(auth.user_id).as_bytes());
    for document_id in [meta, segment] {
        let document = free_murmure_document(&state, &document_id, Some(new.channel_id)).await?;
        let role = db::member_role(&state.db, &document_id, &auth.user_id).await?;
        if document.owner_id != caller || role != Some(Role::Owner) {
            return Err(AppError::Forbidden);
        }
    }
    db::register_channel(&state.db, &new).await?;
    tracing::info!(channel_id = %new.channel_id, space_id = %new.space_id, "channel registered");
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /api/channels/{id}/join`: a member of a space joins a public channel
/// of that space alone, with the channel key wrapped for itself.
///
/// The relay holds the channel public and in a space (`SRV-106`), and the
/// caller a member of that space (`SRV-105`). Every refusal is the same 403,
/// so a stranger learns nothing of the channel. The new row on `meta` holds
/// the caller's role in the space, capped at editor (`SRV-108`). A caller
/// already a member keeps its row, and the answer is the same.
pub async fn join_channel(
    State(state): State<Arc<AppState>>,
    auth: auth::AuthSession,
    Path(channel_id): Path<String>,
    axum::Json(req): axum::Json<JoinChannelRequest>,
) -> Result<impl IntoResponse, AppError> {
    use base64::Engine;
    let channel_id: ambre_core::ChannelId = parse(&channel_id)?;
    let channel = db::get_channel(&state.db, &channel_id)
        .await?
        .filter(|c| c.is_public)
        .ok_or(AppError::Forbidden)?;
    let (Some(space_id), Some(meta_id)) = (channel.space_id, channel.meta_id) else {
        return Err(AppError::Forbidden);
    };
    let space_role = db::member_role(&state.db, &space_id.document_id(), &auth.user_id)
        .await?
        .ok_or(AppError::Forbidden)?;
    let role = joined_role(space_role).ok_or(AppError::Forbidden)?;
    let doc_key_enc = base64::engine::general_purpose::URL_SAFE
        .decode(&req.doc_key_enc)
        .map_err(|e| AppError::Conflict {
            detail: e.to_string(),
        })?;
    let joined =
        db::add_channel_member_if_absent(&state.db, &meta_id, &auth.user_id, role, &doc_key_enc)
            .await?;
    tracing::info!(channel_id = %channel_id, user_id = %auth.user_id, joined, "channel self-join");
    Ok(StatusCode::NO_CONTENT)
}

/// The role a self-join grants on `meta`: the caller's role in the space,
/// capped at editor (`SRV-108`). `None` for a role no document holds.
fn joined_role(space_role: Role) -> Option<Role> {
    match space_role {
        Role::Viewer => Some(Role::Viewer),
        Role::Owner | Role::Editor => Some(Role::Editor),
        Role::Admin | Role::Member => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ambre_core::{ChannelId, DocumentId, MetaId, PublicKey, SegmentId, SpaceId, UserId};
    use base64::Engine;

    fn session(user_id: UserId) -> auth::AuthSession {
        auth::AuthSession {
            user_id,
            signing_pubkey: PublicKey::from_array([7; 32]),
        }
    }

    fn status<T: IntoResponse>(answer: Result<T, AppError>) -> StatusCode {
        match answer {
            Ok(r) => r.into_response().status(),
            Err(e) => e.into_response().status(),
        }
    }

    async fn user(state: &AppState, email: &str) -> UserId {
        crate::test_helpers::create_test_user(&state.db, email).await
    }

    async fn document(state: &AppState, owner: UserId) -> DocumentId {
        crate::test_helpers::create_test_document(&state.db, owner, "murmure").await
    }

    async fn grant(state: &AppState, document: DocumentId, member: UserId, role: Role) {
        db::add_document_member(&state.db, &document, &member, role, &[6; 32], &member)
            .await
            .expect("grant");
    }

    /// A space of `owner`, with `members` at the given roles.
    async fn space(state: &AppState, owner: UserId, members: &[(UserId, Role)]) -> SpaceId {
        let space = document(state, owner).await;
        for &(member, role) in members {
            grant(state, space, member, role).await;
        }
        SpaceId::from_document(space)
    }

    struct Channel {
        id: ChannelId,
        meta: MetaId,
        segment: SegmentId,
    }

    async fn register(
        state: &Arc<AppState>,
        who: UserId,
        space: SpaceId,
        channel: &Channel,
    ) -> StatusCode {
        status(
            create_channel(
                State(state.clone()),
                session(who),
                axum::Json(CreateChannelRequest {
                    channel_id: channel.id.to_string(),
                    space_id: space.to_string(),
                    meta_id: channel.meta.to_string(),
                    segment_id: channel.segment.to_string(),
                }),
            )
            .await,
        )
    }

    /// Two documents of `creator`, not yet a channel.
    async fn new_channel(state: &AppState, creator: UserId) -> Channel {
        Channel {
            id: ChannelId::new(),
            meta: MetaId::from_document(document(state, creator).await),
            segment: SegmentId::from_document(document(state, creator).await),
        }
    }

    /// A public channel of `space`, registered by `creator`.
    async fn public_channel(state: &Arc<AppState>, creator: UserId, space: SpaceId) -> Channel {
        let channel = new_channel(state, creator).await;
        assert_eq!(
            register(state, creator, space, &channel).await,
            StatusCode::NO_CONTENT
        );
        channel
    }

    async fn join(state: &Arc<AppState>, who: UserId, channel: ChannelId, key: u8) -> StatusCode {
        status(
            join_channel(
                State(state.clone()),
                session(who),
                Path(channel.to_string()),
                axum::Json(JoinChannelRequest {
                    doc_key_enc: base64::engine::general_purpose::URL_SAFE.encode([key; 48]),
                }),
            )
            .await,
        )
    }

    async fn role_on(state: &AppState, document: DocumentId, who: UserId) -> Option<Role> {
        db::member_role(&state.db, &document, &who)
            .await
            .expect("query")
    }

    async fn member_ids(state: &AppState, document: DocumentId) -> Vec<uuid::Uuid> {
        let mut ids: Vec<uuid::Uuid> = db::list_document_members(&state.db, &document)
            .await
            .expect("query")
            .into_iter()
            .map(|m| m.user_id)
            .collect();
        ids.sort();
        ids
    }

    fn uuid_of(id: UserId) -> uuid::Uuid {
        uuid::Uuid::from_bytes(*ambre_core::Uuid::from(id).as_bytes())
    }

    // --- The Murmure spaces spec: a space is a document, its creator the owner ---

    /// Creating a space is creating a Murmure document: its creator is its
    /// owner, and its one member.
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn a_new_space_has_its_creator_as_its_one_owner(pool: sqlx::PgPool) {
        let state = crate::test_helpers::test_app_state(pool).await;
        let alice = user(&state, "alice@test.dev").await;
        let space = DocumentId::new();
        let answer = crate::routes::create_document(
            State(state.clone()),
            session(alice),
            axum::Json(crate::routes::CreateDocumentRequest {
                doc_type: "murmure".into(),
                name_enc: None,
                doc_key_enc: base64::engine::general_purpose::URL_SAFE.encode([1u8; 48]),
                document_id: Some(space.to_string()),
            }),
        )
        .await;
        assert_eq!(status(answer), StatusCode::OK);
        assert_eq!(role_on(&state, space, alice).await, Some(Role::Owner));
        assert_eq!(member_ids(&state, space).await, [uuid_of(alice)]);
    }

    // --- Registering a public channel ---

    /// An editor of the space registers a channel: its documents are listed,
    /// the relay holds it public in the space, and no membership moves.
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn an_editor_of_the_space_registers_a_public_channel(pool: sqlx::PgPool) {
        let state = crate::test_helpers::test_app_state(pool).await;
        let alice = user(&state, "alice@test.dev").await;
        let bob = user(&state, "bob@test.dev").await;
        let space = space(&state, alice, &[(bob, Role::Editor)]).await;
        let space_members = member_ids(&state, space.document_id()).await;
        let channel = public_channel(&state, bob, space).await;

        let held = db::get_channel(&state.db, &channel.id)
            .await
            .expect("query")
            .expect("registered");
        assert!(held.is_public);
        assert_eq!(held.space_id, Some(space));
        assert_eq!(held.meta_id, Some(channel.meta));
        let listed = db::list_channel_documents(&state.db, &channel.id)
            .await
            .expect("query");
        assert_eq!(listed.len(), 2);
        assert_eq!(member_ids(&state, space.document_id()).await, space_members);
        assert_eq!(
            member_ids(&state, channel.meta.document_id()).await,
            [uuid_of(bob)]
        );
        assert_eq!(
            register(&state, bob, space, &channel).await,
            StatusCode::NO_CONTENT,
            "a retry answers the same"
        );
    }

    fn uuid_of_doc(id: DocumentId) -> uuid::Uuid {
        uuid::Uuid::from_bytes(*ambre_core::Uuid::from(id).as_bytes())
    }

    /// A viewer of the space changes no catalogue, so it registers no public
    /// channel, and neither does a stranger to the space.
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn a_viewer_or_a_stranger_registers_no_channel(pool: sqlx::PgPool) {
        let state = crate::test_helpers::test_app_state(pool).await;
        let alice = user(&state, "alice@test.dev").await;
        let vera = user(&state, "vera@test.dev").await;
        let eve = user(&state, "eve@test.dev").await;
        let space = space(&state, alice, &[(vera, Role::Viewer)]).await;
        for who in [vera, eve] {
            let channel = new_channel(&state, who).await;
            assert_eq!(
                register(&state, who, space, &channel).await,
                StatusCode::FORBIDDEN
            );
            assert!(
                db::get_channel(&state.db, &channel.id)
                    .await
                    .expect("query")
                    .is_none()
            );
        }
    }

    /// A caller registers only documents it owns, and a document of one
    /// channel never enters another.
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn a_channel_takes_only_free_documents_of_its_creator(pool: sqlx::PgPool) {
        let state = crate::test_helpers::test_app_state(pool).await;
        let alice = user(&state, "alice@test.dev").await;
        let bob = user(&state, "bob@test.dev").await;
        let space = space(&state, alice, &[(bob, Role::Editor)]).await;
        let of_alice = new_channel(&state, alice).await;
        assert_eq!(
            register(&state, bob, space, &of_alice).await,
            StatusCode::FORBIDDEN,
            "Bob owns neither document"
        );
        let first = public_channel(&state, bob, space).await;
        let reused = Channel {
            id: ChannelId::new(),
            meta: first.meta,
            segment: SegmentId::from_document(document(&state, bob).await),
        };
        assert_eq!(
            register(&state, bob, space, &reused).await,
            StatusCode::FORBIDDEN
        );
        let into_a_channel = new_channel(&state, bob).await;
        assert_eq!(
            register(
                &state,
                bob,
                SpaceId::from_document(first.segment.document_id()),
                &into_a_channel
            )
            .await,
            StatusCode::FORBIDDEN,
            "a channel document is no space"
        );
    }

    // --- A self-join ---

    /// A member of the space joins a public channel alone, and reaches its
    /// segment with the key it wrapped for itself.
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn a_member_of_the_space_joins_a_public_channel_alone(pool: sqlx::PgPool) {
        let state = crate::test_helpers::test_app_state(pool).await;
        let alice = user(&state, "alice@test.dev").await;
        let bob = user(&state, "bob@test.dev").await;
        let space = space(&state, alice, &[(bob, Role::Editor)]).await;
        let channel = public_channel(&state, alice, space).await;
        assert_eq!(
            join(&state, bob, channel.id, 9).await,
            StatusCode::NO_CONTENT
        );
        let row = db::get_document_member(&state.db, &channel.segment.document_id(), &bob)
            .await
            .expect("query")
            .expect("the join reaches the segment");
        assert_eq!(row.doc_key_enc, [9; 48]);
        assert_eq!(row.invited_by, Some(uuid_of(bob)));
    }

    /// `SRV-105`: a caller outside the space is refused, a guest of another
    /// channel of the space included. A guest holds a row on a channel, never
    /// on the space document.
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn a_self_join_from_outside_the_space_is_refused(pool: sqlx::PgPool) {
        let state = crate::test_helpers::test_app_state(pool).await;
        let alice = user(&state, "alice@test.dev").await;
        let eve = user(&state, "eve@test.dev").await;
        let carol = user(&state, "carol@test.dev").await;
        let space = space(&state, alice, &[]).await;
        let general = public_channel(&state, alice, space).await;
        let budget = public_channel(&state, alice, space).await;
        grant(&state, budget.meta.document_id(), carol, Role::Editor).await;
        assert_eq!(
            role_on(&state, space.document_id(), carol).await,
            None,
            "a guest"
        );

        for who in [eve, carol] {
            assert_eq!(
                join(&state, who, general.id, 9).await,
                StatusCode::FORBIDDEN
            );
            assert_eq!(role_on(&state, general.meta.document_id(), who).await, None);
        }
    }

    /// `SRV-106`: a channel the relay does not hold as public is refused, a
    /// direct message and an unknown channel alike, with the same answer.
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn a_self_join_of_a_channel_not_held_public_is_refused(pool: sqlx::PgPool) {
        let state = crate::test_helpers::test_app_state(pool).await;
        let alice = user(&state, "alice@test.dev").await;
        let bob = user(&state, "bob@test.dev").await;
        let space = space(&state, alice, &[(bob, Role::Editor)]).await;
        let mut closed = Vec::new();
        for space_id in [Some(uuid_of_doc(space.document_id())), None] {
            let channel = new_channel(&state, alice).await;
            sqlx::query("INSERT INTO channels (id, space_id, is_public) VALUES ($1, $2, false)")
                .bind(uuid::Uuid::from_bytes(
                    *ambre_core::Uuid::from(channel.id).as_bytes(),
                ))
                .bind(space_id)
                .execute(&state.db)
                .await
                .expect("channel");
            closed.push(channel);
        }
        for channel in &closed {
            assert_eq!(
                join(&state, bob, channel.id, 9).await,
                StatusCode::FORBIDDEN
            );
        }
        assert_eq!(
            join(&state, bob, ChannelId::new(), 9).await,
            StatusCode::FORBIDDEN
        );
    }

    /// `SRV-108`: a self-join grants the role held on the space, capped at
    /// editor. An owner of the space joins as an editor, a viewer as a
    /// viewer, and a member already there keeps its row.
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn a_self_join_grants_no_role_above_editor(pool: sqlx::PgPool) {
        let state = crate::test_helpers::test_app_state(pool).await;
        let alice = user(&state, "alice@test.dev").await;
        let bob = user(&state, "bob@test.dev").await;
        let vera = user(&state, "vera@test.dev").await;
        let space = space(&state, alice, &[(bob, Role::Editor), (vera, Role::Viewer)]).await;
        let channel = public_channel(&state, bob, space).await;

        assert_eq!(
            join(&state, alice, channel.id, 1).await,
            StatusCode::NO_CONTENT
        );
        assert_eq!(
            join(&state, vera, channel.id, 2).await,
            StatusCode::NO_CONTENT
        );
        assert_eq!(
            join(&state, bob, channel.id, 3).await,
            StatusCode::NO_CONTENT
        );
        assert_eq!(
            role_on(&state, channel.meta.document_id(), alice).await,
            Some(Role::Editor)
        );
        assert_eq!(
            role_on(&state, channel.meta.document_id(), vera).await,
            Some(Role::Viewer)
        );
        assert_eq!(
            role_on(&state, channel.meta.document_id(), bob).await,
            Some(Role::Owner),
            "a join never lowers a member"
        );
        let creator = db::get_document_member(&state.db, &channel.meta.document_id(), &bob)
            .await
            .expect("query")
            .expect("the creator");
        assert_eq!(creator.doc_key_enc, [5; 32], "nor swaps its key");
    }

    /// A self-join writes the channel's `meta` alone: the space document
    /// keeps its members.
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn a_self_join_changes_no_member_of_the_space(pool: sqlx::PgPool) {
        let state = crate::test_helpers::test_app_state(pool).await;
        let alice = user(&state, "alice@test.dev").await;
        let bob = user(&state, "bob@test.dev").await;
        let space = space(&state, alice, &[(bob, Role::Viewer)]).await;
        let before = member_ids(&state, space.document_id()).await;
        let channel = public_channel(&state, alice, space).await;
        assert_eq!(
            join(&state, bob, channel.id, 9).await,
            StatusCode::NO_CONTENT
        );
        assert_eq!(member_ids(&state, space.document_id()).await, before);
        assert_eq!(
            role_on(&state, space.document_id(), bob).await,
            Some(Role::Viewer)
        );
    }
}
