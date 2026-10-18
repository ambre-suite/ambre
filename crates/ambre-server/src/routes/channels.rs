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
use crate::extract::Json;
use crate::routes::share::{Expected, checked_entry};
use crate::state::AppState;
use ambre_protocol::MembershipAct;

/// One document of a channel, as the listing answers it.
#[derive(serde::Serialize)]
pub struct ChannelDocumentJson {
    #[serde(with = "ambre_core::serde_id::document_id")]
    pub document_id: ambre_core::DocumentId,
    #[serde(with = "ambre_core::serde_enums::channel_document_kind")]
    pub kind: ambre_core::ChannelDocumentKind,
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
            document_id: d.document_id,
            kind: d.kind,
        })
        .collect();
    Ok(super::DocumentRows::json(json))
}

/// The public channels of a space, as the listing answers them.
#[derive(serde::Serialize)]
pub struct SpaceChannelsJson {
    #[serde(with = "ambre_core::serde_id::channel_id::vec")]
    pub channels: Vec<ambre_core::ChannelId>,
}

/// Tells every connection of `user` that `meta` was shared with them. A
/// channel has no wire message of its own (`SRV-102`), so this is how a
/// member learns of one (`SRV-192`). Best-effort: the listing is the truth.
pub(crate) async fn share_notice(
    state: &AppState,
    user: &ambre_core::UserId,
    meta: ambre_core::DocumentId,
) {
    let msg =
        crate::payload::Payload::server_msg(&ambre_protocol::ws::WsServerMsg::DocumentShared {
            document_id: meta,
            doc_type: ambre_core::DocType::Murmure,
        });
    let _ = crate::redis_ops::user_broadcast(&state.redis, user, &msg).await;
}

/// One channel of the caller, as their listing answers it. `space_id` is
/// `null` for a direct message.
#[derive(serde::Serialize)]
pub struct MemberChannelJson {
    #[serde(with = "ambre_core::serde_id::channel_id")]
    pub channel_id: ambre_core::ChannelId,
    #[serde(with = "ambre_core::serde_id::space_id::option")]
    pub space_id: Option<ambre_core::SpaceId>,
    pub public: bool,
}

/// The channels of the caller.
#[derive(serde::Serialize)]
pub struct MemberChannelsJson {
    pub channels: Vec<MemberChannelJson>,
}

/// `GET /api/channels`: every channel the caller is a member of, by its
/// `meta` row, with its space and whether it is public (`SRV-191`).
pub async fn list_member_channels(
    State(state): State<Arc<AppState>>,
    auth: auth::AuthSession,
) -> Result<axum::Json<MemberChannelsJson>, AppError> {
    let channels = db::list_member_channels(&state.db, &auth.user_id)
        .await?
        .into_iter()
        .map(|row| MemberChannelJson {
            channel_id: row.id,
            space_id: row.space_id,
            public: row.is_public,
        })
        .collect();
    Ok(axum::Json(MemberChannelsJson { channels }))
}

/// `GET /api/spaces/{id}/channels`: the channels the relay holds as public
/// in the space, to a member of the space document alone (`SRV-189`).
///
/// An unknown space and a space the caller is no member of answer the same
/// 403. A private channel is never listed: the catalogue lists none.
pub async fn list_space_channels(
    State(state): State<Arc<AppState>>,
    auth: auth::AuthSession,
    Path(space_id): Path<String>,
) -> Result<axum::Json<SpaceChannelsJson>, AppError> {
    let space_id = ambre_core::Uuid::parse_str(&space_id)
        .map(ambre_core::SpaceId::from_uuid)
        .map_err(|_| AppError::Forbidden)?;
    db::get_document_member(&state.db, &space_id.document_id(), &auth.user_id)
        .await?
        .ok_or(AppError::Forbidden)?;
    let channels = db::list_space_public_channels(&state.db, &space_id).await?;
    Ok(axum::Json(SpaceChannelsJson { channels }))
}

/// The body of a channel registration. Every id is one the client generated,
/// and both documents already exist on the relay.
#[derive(serde::Deserialize)]
pub struct CreateChannelRequest {
    /// A public channel, which its catalogue lists, or a private one.
    pub public: bool,
    #[serde(with = "ambre_core::serde_id::channel_id")]
    pub channel_id: ambre_core::ChannelId,
    #[serde(with = "ambre_core::serde_id::space_id")]
    pub space_id: ambre_core::SpaceId,
    #[serde(with = "ambre_core::serde_id::meta_id")]
    pub meta_id: ambre_core::MetaId,
    #[serde(with = "ambre_core::serde_id::segment_id")]
    pub segment_id: ambre_core::SegmentId,
}

/// One member of a new direct message, other than its creator, with the
/// channel key wrapped for them alone.
#[derive(serde::Deserialize)]
pub struct DirectMemberJson {
    #[serde(with = "ambre_core::serde_id::user_id")]
    pub user_id: ambre_core::UserId,
    pub doc_key_enc: String,
}

/// The body of a direct message request. Every id is one the client
/// generated, and none exists on the relay yet.
#[derive(serde::Deserialize)]
pub struct CreateDirectMessageRequest {
    #[serde(with = "ambre_core::serde_id::channel_id")]
    pub channel_id: ambre_core::ChannelId,
    #[serde(with = "ambre_core::serde_id::meta_id")]
    pub meta_id: ambre_core::MetaId,
    #[serde(with = "ambre_core::serde_id::segment_id")]
    pub segment_id: ambre_core::SegmentId,
    /// Every member but the caller.
    pub members: Vec<DirectMemberJson>,
    /// The channel key wrapped for the caller.
    pub doc_key_enc: String,
    /// The identifier of the channel key, in hex, which the creation and every
    /// invite of the log name.
    #[serde(with = "ambre_core::serde_id::key_id")]
    pub key_id: ambre_core::KeyId,
    /// The log of `meta`, signed by the caller. In order, it holds its
    /// `Create`, one editor `Invite` per member in the order of `members`,
    /// then the caller's own `ChangeRole` to editor.
    pub entries: Vec<String>,
}

/// The direct message a request reached: the one it created, or the one
/// that already held its members.
#[derive(serde::Serialize)]
pub struct DirectMessageJson {
    #[serde(with = "ambre_core::serde_id::channel_id")]
    pub channel_id: ambre_core::ChannelId,
    pub created: bool,
}

/// The body of a self-join: the channel key wrapped for the caller. It also
/// carries the self-join as an entry of the log of `meta`, signed by the caller.
/// A `meta` whose log holds entries takes no self-join without one.
#[derive(serde::Deserialize)]
pub struct JoinChannelRequest {
    pub doc_key_enc: String,
    #[serde(default)]
    pub entry: Option<String>,
}

/// The end of the log of `meta`, which a joining member extends: the
/// position its entry takes and the entry it names as previous.
#[derive(serde::Serialize)]
pub struct JoinHeadJson {
    /// The document whose log the self-join extends.
    #[serde(with = "ambre_core::serde_id::meta_id")]
    pub meta_id: ambre_core::MetaId,
    /// The role the self-join grants, which its entry must name.
    pub role: Role,
    pub length: u64,
    /// The last entry, base64 url-safe, absent on an empty log.
    pub last: Option<String>,
}

/// The body of a segment close: the active segment the caller saw, and the
/// successor it created, which no channel groups yet.
#[derive(serde::Deserialize)]
pub struct CloseSegmentRequest {
    #[serde(with = "ambre_core::serde_id::segment_id")]
    pub closing: ambre_core::SegmentId,
    #[serde(with = "ambre_core::serde_id::segment_id")]
    pub successor: ambre_core::SegmentId,
}

/// The active segment after a close, and whether it is the caller's
/// successor. `false` means an earlier close registered another one.
#[derive(serde::Serialize)]
pub struct ClosedSegmentJson {
    #[serde(with = "ambre_core::serde_id::segment_id")]
    pub active: ambre_core::SegmentId,
    pub created: bool,
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
        .filter(|d| d.doc_type == ambre_core::DocType::Murmure)
        .ok_or(AppError::Forbidden)?;
    let held = db::channel_of_document(&state.db, document_id).await?;
    if held.is_some() && held != channel {
        return Err(AppError::Forbidden);
    }
    Ok(document)
}

/// `POST /api/channels`: register a channel of a space, with its `meta` and
/// its first segment.
///
/// The caller owns both documents, and is a member of the space. A public
/// channel also needs an owner or an editor of the space document: a viewer
/// changes no catalogue, so it registers no public channel. A private channel
/// is listed nowhere, so any member of the space registers one. The members of `meta` become the members of the channel
/// (`SRV-103`), and the space document gains no row.
pub async fn create_channel(
    State(state): State<Arc<AppState>>,
    auth: auth::AuthSession,
    Json(req): Json<CreateChannelRequest>,
) -> Result<impl IntoResponse, AppError> {
    let new = db::NewChannel {
        channel_id: req.channel_id,
        space_id: req.space_id,
        is_public: req.public,
        meta_id: req.meta_id,
        segment_id: req.segment_id,
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
    if new.is_public && !space_role.can_write_document() {
        return Err(AppError::Forbidden);
    }
    for document_id in [meta, segment] {
        let document = free_murmure_document(&state, &document_id, Some(new.channel_id)).await?;
        let role = db::member_role(&state.db, &document_id, &auth.user_id).await?;
        if document.owner_id != auth.user_id || role != Some(Role::Owner) {
            return Err(AppError::Forbidden);
        }
    }
    db::register_channel(&state.db, &new).await?;
    tracing::info!(channel_id = %new.channel_id, space_id = %new.space_id, public = new.is_public, "channel registered");
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /api/channels/{id}/segments`: close the active segment of a channel
/// into the caller's successor (`SRV-194`).
///
/// The caller writes to the channel: a viewer posts nothing, so it fills no
/// segment. The successor is a Murmure document the caller owns, as at a
/// registration. A close that lost its race answers the winner's successor
/// (`SRV-195`), and the caller deletes its own.
pub async fn close_segment(
    State(state): State<Arc<AppState>>,
    auth: auth::AuthSession,
    Path(channel_id): Path<String>,
    Json(req): Json<CloseSegmentRequest>,
) -> Result<axum::Json<ClosedSegmentJson>, AppError> {
    let channel_id: ambre_core::ChannelId = parse(&channel_id)?;
    let meta = db::get_channel(&state.db, &channel_id)
        .await?
        .and_then(|channel| channel.meta_id)
        .ok_or(AppError::Forbidden)?;
    let role = db::member_role(&state.db, &meta.document_id(), &auth.user_id).await?;
    if !role.is_some_and(Role::can_write_document) || req.closing == req.successor {
        return Err(AppError::Forbidden);
    }
    let successor = req.successor.document_id();
    let document = free_murmure_document(&state, &successor, Some(channel_id)).await?;
    let own_role = db::member_role(&state.db, &successor, &auth.user_id).await?;
    if document.owner_id != auth.user_id || own_role != Some(Role::Owner) {
        return Err(AppError::Forbidden);
    }
    let active = db::close_segment(&state.db, &channel_id, req.closing, req.successor).await?;
    let created = active == req.successor;
    tracing::info!(channel_id = %channel_id, closing = %req.closing, active = %active, created, "segment closed");
    Ok(axum::Json(ClosedSegmentJson { active, created }))
}

/// The grace period of a closed segment before its finalization, since its
/// close and since its latest delta (`ambre-spec-budgets.md section 8`).
pub const FINALIZE_GRACE: std::time::Duration = std::time::Duration::from_secs(24 * 3600);

/// How long a finalize lease lasts (`ambre-spec-budgets.md section 8`).
pub const FINALIZE_LEASE: std::time::Duration = std::time::Duration::from_secs(30);

/// The wait a lease request is answered while a delta of the segment still
/// waits to be flushed.
const FINALIZE_PENDING_RETRY: std::time::Duration = std::time::Duration::from_secs(2);

/// The answer to a lease request: the lease, or none and when to ask again.
#[derive(Debug, serde::Serialize)]
pub struct FinalizeLeaseJson {
    pub lease: Option<GrantedLeaseJson>,
    /// Milliseconds before asking again, `0` with a lease.
    pub retry_in: u64,
}

/// A granted finalize lease (`SRV-113`).
#[derive(Debug, serde::Serialize)]
pub struct GrantedLeaseJson {
    #[serde(with = "ambre_core::serde_id::lease_id")]
    pub lease_id: ambre_core::LeaseId,
    /// The last delta number the final snapshot covers.
    pub horizon: u64,
    /// Milliseconds before the lease expires.
    pub expires_in: u64,
}

/// The body of a finalize commit: the lease, and the snapshot that covers
/// the horizon.
#[derive(serde::Deserialize)]
pub struct FinalizeSegmentRequest {
    #[serde(with = "ambre_core::serde_id::lease_id")]
    pub lease_id: ambre_core::LeaseId,
    #[serde(with = "ambre_core::serde_id::snapshot_id")]
    pub snapshot_id: ambre_core::SnapshotId,
}

/// The channel and the segment of a finalize path, once the caller is a
/// writer of the channel. A viewer posts nothing, so it finalizes nothing.
async fn finalize_target(
    state: &AppState,
    auth: &auth::AuthSession,
    channel_id: &str,
    segment: &str,
) -> Result<(ambre_core::ChannelId, ambre_core::SegmentId), AppError> {
    let channel_id: ambre_core::ChannelId = parse(channel_id)?;
    let segment: ambre_core::SegmentId = parse(segment)?;
    let meta = db::get_channel(&state.db, &channel_id)
        .await?
        .and_then(|channel| channel.meta_id)
        .ok_or(AppError::Forbidden)?;
    let role = db::member_role(&state.db, &meta.document_id(), &auth.user_id).await?;
    if !role.is_some_and(Role::can_write_document) {
        return Err(AppError::Forbidden);
    }
    Ok((channel_id, segment))
}

/// `POST /api/channels/{id}/segments/{segment}/lease`: take the finalize lease
/// of a closing segment (`SRV-113`).
///
/// The first lease locks the segment against deltas for good, in the store
/// first, then in the register (`SRV-114`). It waits out the grace period
/// (`SRV-199`), and any delta still unflushed.
pub async fn take_finalize_lease(
    State(state): State<Arc<AppState>>,
    auth: auth::AuthSession,
    Path((channel_id, segment)): Path<(String, String)>,
) -> Result<axum::Json<FinalizeLeaseJson>, AppError> {
    let (channel_id, segment) = finalize_target(&state, &auth, &channel_id, &segment).await?;
    let terms = db::LeaseTerms {
        grace: FINALIZE_GRACE,
        lease: FINALIZE_LEASE,
        pending_retry: FINALIZE_PENDING_RETRY,
    };
    let document = segment.document_id();
    let lock = async || crate::redis_ops::lock_segment(&state.redis, &document).await;
    let answer = db::take_finalize_lease(&state.db, &channel_id, segment, terms, lock).await?;
    let millis = |d: std::time::Duration| u64::try_from(d.as_millis()).unwrap_or(u64::MAX);
    Ok(axum::Json(match answer {
        db::FinalizeLease::Granted {
            lease_id,
            horizon,
            expires_in,
        } => {
            tracing::info!(channel_id = %channel_id, segment = %segment, horizon, "finalize lease granted");
            FinalizeLeaseJson {
                lease: Some(GrantedLeaseJson {
                    lease_id,
                    horizon,
                    expires_in: millis(expires_in),
                }),
                retry_in: 0,
            }
        }
        db::FinalizeLease::Wait { retry_in } => FinalizeLeaseJson {
            lease: None,
            retry_in: millis(retry_in),
        },
    }))
}

/// `POST /api/channels/{id}/segments/{segment}/finalize`: commit the final
/// snapshot of a segment under the caller's live lease (`SRV-116`,
/// `SRV-117`). The segment becomes `final`.
pub async fn finalize_segment(
    State(state): State<Arc<AppState>>,
    auth: auth::AuthSession,
    Path((channel_id, segment)): Path<(String, String)>,
    Json(req): Json<FinalizeSegmentRequest>,
) -> Result<StatusCode, AppError> {
    let (channel_id, segment) = finalize_target(&state, &auth, &channel_id, &segment).await?;
    db::commit_finalization(
        &state.db,
        &channel_id,
        segment,
        req.lease_id,
        req.snapshot_id,
    )
    .await?;
    tracing::info!(channel_id = %channel_id, segment = %segment, snapshot_id = %req.snapshot_id, "segment finalized");
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /api/channels/direct`: open the direct message of the caller and
/// `members`, created when none holds exactly them (`SRV-175`, `SRV-176`).
///
/// The new channel belongs to no space, and every member is an editor of its
/// `meta`, the caller included, so nobody owns it. The log records why: the
/// caller creates `meta`, invites each member as an editor, then sets its own
/// role to editor. Every entry is checked before anything is written. A
/// direct message already holding exactly these members answers its id, and
/// the request writes nothing: no member is added to it.
pub async fn create_direct_message(
    State(state): State<Arc<AppState>>,
    auth: auth::AuthSession,
    Json(req): Json<CreateDirectMessageRequest>,
) -> Result<axum::Json<DirectMessageJson>, AppError> {
    use base64::Engine;
    let invalid = |detail: &str| AppError::BadRequest(detail.into());
    if req.meta_id.document_id() == req.segment_id.document_id() {
        return Err(invalid("meta and segment are one document"));
    }
    let mut seen = std::collections::HashSet::from([auth.user_id]);
    if req.members.is_empty() || !req.members.iter().all(|m| seen.insert(m.user_id)) {
        return Err(invalid(
            "a direct message needs distinct members besides the caller",
        ));
    }
    if req.entries.len() != req.members.len() + 2 {
        return Err(invalid(
            "one entry per member, plus the creation and the role",
        ));
    }
    for member in &req.members {
        db::get_user(&state.db, &member.user_id)
            .await?
            .ok_or(AppError::NotFound { resource: "user" })?;
    }
    let decode = |b64: &str| {
        base64::engine::general_purpose::URL_SAFE
            .decode(b64)
            .map_err(|e| AppError::BadRequest(e.to_string()))
    };
    let mut keys = vec![(auth.user_id, decode(&req.doc_key_enc)?)];
    for member in &req.members {
        keys.push((member.user_id, decode(&member.doc_key_enc)?));
    }
    let key_id = req.key_id;
    let meta = req.meta_id.document_id();
    let others: Vec<_> = req.members.iter().map(|m| m.user_id).collect();
    let mut entries = Vec::with_capacity(req.entries.len());
    for (text, (member, act, role)) in req
        .entries
        .iter()
        .zip(ambre_protocol::ownerless_log(auth.user_id, &others))
    {
        // The creation and each invite name the key they hand over, and the
        // role change names none.
        let key = (act != MembershipAct::ChangeRole).then_some(key_id);
        let expected = Expected {
            act,
            member,
            role: Some(role),
            key: Some(key),
        };
        let entry = checked_entry(&state, &auth, meta, Some(text), expected)
            .await?
            .ok_or_else(|| invalid("an entry is missing"))?;
        entries.push(entry);
    }
    let found = db::find_or_create_direct_message(
        &state.db,
        &db::NewDirectMessage {
            channel_id: req.channel_id,
            meta_id: req.meta_id,
            segment_id: req.segment_id,
            creator: auth.user_id,
            members: &keys,
            entries: &entries,
        },
    )
    .await?;
    let (channel_id, created) = match found {
        db::DirectMessage::Created => (req.channel_id, true),
        db::DirectMessage::Existing(channel_id) => (channel_id, false),
    };
    tracing::info!(channel_id = %channel_id, by = %auth.user_id, members = keys.len(), created, "direct message");
    // Each member's clients learn of the new channel as the share of its
    // `meta`, the creator's other clients included (`SRV-192`).
    if created {
        for (member, _) in &keys {
            share_notice(&state, member, meta).await;
        }
    }
    Ok(axum::Json(DirectMessageJson {
        channel_id,
        created,
    }))
}

/// `DELETE /api/channels/{id}`: delete a channel with every document it
/// groups, to an owner of its `meta` alone (`SRV-142`, `SRV-015`). An owner
/// of its space stands in for one on a public channel (`SRV-174`).
///
/// Every refusal is the same 403, so a stranger learns nothing of which
/// channels exist. Each document is marked in the same transaction, then
/// emptied as a deleted document is. One that fails is left to the pass
/// (`SRV-017`, `SRV-165`).
pub async fn delete_channel(
    State(state): State<Arc<AppState>>,
    auth: auth::AuthSession,
    Path(channel_id): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    let channel_id: ambre_core::ChannelId = parse(&channel_id)?;
    let channel = db::get_channel(&state.db, &channel_id)
        .await?
        .ok_or(AppError::Forbidden)?;
    let meta_id = channel.meta_id.ok_or(AppError::Forbidden)?;
    let space_owner = match channel.space_id.filter(|_| channel.is_public) {
        Some(space_id) => {
            db::member_role(&state.db, &space_id.document_id(), &auth.user_id).await?
                == Some(Role::Owner)
        }
        None => false,
    };
    let meta_role = db::member_role(&state.db, &meta_id.document_id(), &auth.user_id).await?;
    if !space_owner && meta_role != Some(Role::Owner) {
        return Err(AppError::Forbidden);
    }
    let sweep_due = crate::deletion::sweep_due_stored(&state);
    let deleted = db::delete_channel(&state.db, &channel_id, sweep_due).await?;
    tracing::info!(channel_id = %channel_id, by = %auth.user_id, documents = deleted.documents.len(), "channel deleted");
    crate::deletion::empty_marked(&state, &deleted.documents, sweep_due).await;
    // Every client of every member drops it, the owner's own included
    // (`SRV-188`).
    for member in &deleted.members {
        super::share::notify_removed(&state, &meta_id.document_id(), member).await;
    }
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /api/channels/{id}/join`: a member of a space joins a public channel
/// of that space alone, with the channel key wrapped for itself.
///
/// The relay holds the channel public and in a space (`SRV-106`), and the
/// caller a member of that space (`SRV-105`). Every refusal is the same 403,
/// so a stranger learns nothing of the channel. The new row on `meta` holds
/// the caller's role in the space, capped at editor (`SRV-108`). It lands
/// with the entry the caller signed (`SRV-107`). A caller already a member
/// keeps its row, records no entry, and gets the same answer.
pub async fn join_channel(
    State(state): State<Arc<AppState>>,
    auth: auth::AuthSession,
    Path(channel_id): Path<String>,
    Json(req): Json<JoinChannelRequest>,
) -> Result<impl IntoResponse, AppError> {
    use base64::Engine;
    let channel_id: ambre_core::ChannelId = parse(&channel_id)?;
    let Joinable {
        space_id,
        meta_id,
        role,
    } = joinable(&state, &auth, &channel_id).await?;
    let doc_key_enc = base64::engine::general_purpose::URL_SAFE
        .decode(&req.doc_key_enc)
        .map_err(|e| AppError::Conflict {
            detail: e.to_string(),
        })?;
    let entry = checked_entry(
        &state,
        &auth,
        meta_id.document_id(),
        req.entry.as_deref(),
        Expected {
            act: MembershipAct::SelfJoin,
            member: auth.user_id,
            role: Some(role),
            key: None,
        },
    )
    .await?;
    let joined = db::add_channel_member_if_absent(
        &state.db,
        &space_id,
        &meta_id,
        &auth.user_id,
        role,
        &doc_key_enc,
        entry.as_ref(),
    )
    .await?;
    tracing::info!(channel_id = %channel_id, user_id = %auth.user_id, joined, "channel self-join");
    if joined {
        crate::redis_ops::members_changed(&state.redis, &meta_id.document_id()).await;
    }
    Ok(StatusCode::NO_CONTENT)
}

/// `GET /api/channels/{id}/join`: the end of the log of `meta`, to a caller
/// the self-join would admit, with the role the join grants. A joining
/// member reads no log before it joins, so this is what its entry extends.
/// The refusals are the self-join's.
pub async fn join_channel_head(
    State(state): State<Arc<AppState>>,
    auth: auth::AuthSession,
    Path(channel_id): Path<String>,
) -> Result<axum::Json<JoinHeadJson>, AppError> {
    use base64::Engine;
    let channel_id: ambre_core::ChannelId = parse(&channel_id)?;
    let Joinable { meta_id, role, .. } = joinable(&state, &auth, &channel_id).await?;
    let log = db::membership_log(&state.db, &meta_id.document_id()).await?;
    Ok(axum::Json(JoinHeadJson {
        meta_id,
        role,
        length: u64::try_from(log.len()).unwrap_or(u64::MAX),
        last: log
            .last()
            .map(|e| base64::engine::general_purpose::URL_SAFE.encode(e)),
    }))
}

/// A channel the caller may join alone: its space, its `meta`, and the role
/// the join takes.
struct Joinable {
    space_id: ambre_core::SpaceId,
    meta_id: ambre_core::MetaId,
    role: Role,
}

/// The channel the caller may join alone.
///
/// The relay holds the channel public and in a space (`SRV-106`), and the
/// caller a member of that space (`SRV-105`). Every refusal is the same 403.
async fn joinable(
    state: &AppState,
    auth: &auth::AuthSession,
    channel_id: &ambre_core::ChannelId,
) -> Result<Joinable, AppError> {
    let channel = db::get_channel(&state.db, channel_id)
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
    Ok(Joinable {
        space_id,
        meta_id,
        role,
    })
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
    use ambre_core::{ChannelId, DocumentId, MetaId, SegmentId, SpaceId, UserId};
    use base64::Engine;

    /// The signing key every test user registers, so an entry one signs
    /// checks against the row and the session alike.
    fn signing_key() -> ed25519_dalek::SigningKey {
        ed25519_dalek::SigningKey::from_bytes(&[7; 32])
    }

    fn session(user_id: UserId) -> auth::AuthSession {
        auth::AuthSession {
            user_id,
            signing_pubkey: ambre_core::PublicKey::from_array(
                signing_key().verifying_key().to_bytes(),
            ),
        }
    }

    /// The entry `who` signs about themselves at `position` of `document`'s
    /// log, in the form a request carries it. The relay walks no digest, so
    /// the previous one is filler.
    fn signed_entry(
        document: DocumentId,
        position: u64,
        who: UserId,
        act: MembershipAct,
        role: Role,
    ) -> String {
        use ed25519_dalek::Signer;
        let key = signing_key();
        let mut entry = ambre_protocol::MembershipEntry {
            document,
            position,
            member: who,
            member_key: key.verifying_key().to_bytes(),
            act,
            role,
            key: None,
            previous: (position > 0).then_some(ambre_protocol::EntryDigest([1; 32])),
            author: who,
            signature: Vec::new(),
        };
        entry.signature = key
            .sign(&entry.signed_bytes().expect("signed bytes"))
            .to_bytes()
            .to_vec();
        base64::engine::general_purpose::URL_SAFE.encode(entry.encode().expect("encode"))
    }

    /// The length of `document`'s log.
    async fn log_length(state: &AppState, document: DocumentId) -> u64 {
        let log = db::membership_log(&state.db, &document).await.expect("log");
        u64::try_from(log.len()).expect("length")
    }

    /// Opens `meta`'s log with a `Create` entry of `creator`, as Sync does
    /// when it creates a channel.
    async fn open_log(state: &AppState, meta: DocumentId, creator: UserId) {
        let text = signed_entry(meta, 0, creator, MembershipAct::Create, Role::Owner);
        let bytes = base64::engine::general_purpose::URL_SAFE
            .decode(text)
            .expect("base64");
        let entry = db::LogEntry { position: 0, bytes };
        db::add_document_member(
            &state.db,
            &meta,
            &creator,
            Role::Owner,
            &[6; 32],
            &creator,
            Some(&entry),
        )
        .await
        .expect("open the log");
    }

    fn status<T: IntoResponse>(answer: Result<T, AppError>) -> StatusCode {
        match answer {
            Ok(r) => r.into_response().status(),
            Err(e) => e.into_response().status(),
        }
    }

    async fn user(state: &AppState, email: &str) -> UserId {
        let signing = signing_key().verifying_key().to_bytes();
        let row = db::create_user(
            &state.db,
            email,
            "Test User",
            &[1; 32],
            &[2; 16],
            &signing,
            &[4; 32],
        )
        .await
        .expect("user");
        row.id
    }

    async fn document(state: &AppState, owner: UserId) -> DocumentId {
        crate::test_helpers::create_test_document(&state.db, owner, "murmure").await
    }

    async fn grant(state: &AppState, document: DocumentId, member: UserId, role: Role) {
        db::add_document_member(&state.db, &document, &member, role, &[6; 32], &member, None)
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
        register_as(state, who, space, channel, true).await
    }

    async fn register_as(
        state: &Arc<AppState>,
        who: UserId,
        space: SpaceId,
        channel: &Channel,
        public: bool,
    ) -> StatusCode {
        status(
            create_channel(
                State(state.clone()),
                session(who),
                Json(CreateChannelRequest {
                    public,
                    channel_id: channel.id,
                    space_id: space,
                    meta_id: channel.meta,
                    segment_id: channel.segment,
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

    /// A self-join entry of `who`, extending the log of the channel's `meta`.
    /// A `meta` whose log is empty predates it, and its self-join carries
    /// none. A channel the caller may not join gets none either: the route
    /// refuses before it reads one.
    async fn join_entry(state: &AppState, who: UserId, channel: ChannelId) -> Option<String> {
        let Joinable {
            meta_id: meta,
            role,
            ..
        } = joinable(state, &session(who), &channel).await.ok()?;
        let position = log_length(state, meta.document_id()).await;
        (position > 0).then(|| {
            signed_entry(
                meta.document_id(),
                position,
                who,
                MembershipAct::SelfJoin,
                role,
            )
        })
    }

    async fn join(state: &Arc<AppState>, who: UserId, channel: ChannelId, key: u8) -> StatusCode {
        let entry = join_entry(state, who, channel).await;
        status(
            join_channel(
                State(state.clone()),
                session(who),
                Path(channel.to_string()),
                Json(JoinChannelRequest {
                    doc_key_enc: base64::engine::general_purpose::URL_SAFE.encode([key; 48]),
                    entry,
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

    async fn member_ids(state: &AppState, document: DocumentId) -> Vec<UserId> {
        let mut ids: Vec<UserId> = db::list_document_members(&state.db, &document)
            .await
            .expect("query")
            .into_iter()
            .map(|m| m.user_id)
            .collect();
        ids.sort_by_key(|id| *id.as_bytes());
        ids
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
            Json(crate::routes::CreateDocumentRequest {
                doc_type: ambre_core::DocType::Murmure,
                name_enc: None,
                doc_key_enc: base64::engine::general_purpose::URL_SAFE.encode([1u8; 48]),
                document_id: Some(space),
                entry: Some(signed_entry(
                    space,
                    0,
                    alice,
                    MembershipAct::Create,
                    Role::Owner,
                )),
            }),
        )
        .await;
        assert_eq!(status(answer), StatusCode::OK);
        assert_eq!(role_on(&state, space, alice).await, Some(Role::Owner));
        assert_eq!(member_ids(&state, space).await, [alice]);
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
        assert_eq!(member_ids(&state, channel.meta.document_id()).await, [bob]);
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
        assert_eq!(row.invited_by, Some(bob));
    }

    /// `SRV-177`: a self-join tells the connections subscribed to `meta` that
    /// its members changed. A caller already a member changes no row, and
    /// nothing is said.
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn a_self_join_tells_the_subscribers_of_meta(pool: sqlx::PgPool) {
        use futures_util::StreamExt;
        use rustis::commands::PubSubCommands;
        let state = crate::test_helpers::test_app_state(pool).await;
        let alice = user(&state, "alice@test.dev").await;
        let bob = user(&state, "bob@test.dev").await;
        let space = space(&state, alice, &[(bob, Role::Editor)]).await;
        let channel = public_channel(&state, alice, space).await;
        let meta = channel.meta.document_id();
        // A connection of its own: a subscriber sharing the relay's client
        // would desynchronise its replies (see `PubSubHub`).
        let subscriber = rustis::client::Client::connect("redis://localhost:6379")
            .await
            .expect("redis");
        let mut stream = subscriber
            .subscribe(format!("doc:{meta}"))
            .await
            .expect("a subscription");
        let wait = std::time::Duration::from_millis(500);

        assert_eq!(
            join(&state, bob, channel.id, 9).await,
            StatusCode::NO_CONTENT
        );
        let msg = tokio::time::timeout(wait, stream.next())
            .await
            .expect("a broadcast")
            .expect("an open stream")
            .expect("a message");
        assert!(matches!(
            ambre_protocol::ws::decode_server_msg(msg.payload()),
            Ok(ambre_protocol::ws::WsServerMsg::MembersChanged { document_id }) if document_id == meta
        ));

        assert_eq!(
            join(&state, bob, channel.id, 9).await,
            StatusCode::NO_CONTENT
        );
        assert!(
            tokio::time::timeout(wait, stream.next()).await.is_err(),
            "a join that adds no row says nothing"
        );
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

    /// An accepted self-join records the entry the joining member signed,
    /// beside its row, and the log of `meta` ends with it (`SRV-107`). A
    /// member already there records nothing.
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn an_accepted_self_join_stores_the_entry_the_member_signed(pool: sqlx::PgPool) {
        use base64::Engine;
        let state = crate::test_helpers::test_app_state(pool).await;
        let alice = user(&state, "alice@test.dev").await;
        let bob = user(&state, "bob@test.dev").await;
        let space = space(&state, alice, &[(bob, Role::Editor)]).await;
        let channel = public_channel(&state, bob, space).await;
        let meta = channel.meta.document_id();
        open_log(&state, meta, bob).await;

        let entry = join_entry(&state, alice, channel.id).await;
        assert!(entry.is_some(), "a logged meta takes the self-join's entry");
        let answer = join_channel(
            State(state.clone()),
            session(alice),
            Path(channel.id.to_string()),
            Json(JoinChannelRequest {
                doc_key_enc: base64::engine::general_purpose::URL_SAFE.encode([1; 48]),
                entry: entry.clone(),
            }),
        )
        .await;
        assert_eq!(status(answer), StatusCode::NO_CONTENT);
        let log = db::membership_log(&state.db, &meta).await.expect("log");
        assert_eq!(log.len(), 2);
        let sent = base64::engine::general_purpose::URL_SAFE
            .decode(entry.expect("entry"))
            .expect("base64");
        assert_eq!(log.last(), Some(&sent));

        assert_eq!(
            join(&state, bob, channel.id, 2).await,
            StatusCode::NO_CONTENT
        );
        assert_eq!(log_length(&state, meta).await, 2);
    }

    /// A self-join of a logged `meta` that carries no entry adds no member.
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn a_self_join_of_a_logged_meta_without_an_entry_adds_no_member(pool: sqlx::PgPool) {
        use base64::Engine;
        let state = crate::test_helpers::test_app_state(pool).await;
        let alice = user(&state, "alice@test.dev").await;
        let bob = user(&state, "bob@test.dev").await;
        let space = space(&state, alice, &[(bob, Role::Editor)]).await;
        let channel = public_channel(&state, bob, space).await;
        let meta = channel.meta.document_id();
        open_log(&state, meta, bob).await;

        let answer = join_channel(
            State(state.clone()),
            session(alice),
            Path(channel.id.to_string()),
            Json(JoinChannelRequest {
                doc_key_enc: base64::engine::general_purpose::URL_SAFE.encode([1; 48]),
                entry: None,
            }),
        )
        .await;
        assert_eq!(status(answer), StatusCode::BAD_REQUEST);
        assert_eq!(role_on(&state, meta, alice).await, None);
        assert_eq!(log_length(&state, meta).await, 1);
    }

    /// A self-join whose entry names another role than the one the relay
    /// grants adds no member.
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn a_self_join_whose_entry_claims_another_role_adds_no_member(pool: sqlx::PgPool) {
        use base64::Engine;
        let state = crate::test_helpers::test_app_state(pool).await;
        let alice = user(&state, "alice@test.dev").await;
        let bob = user(&state, "bob@test.dev").await;
        let space = space(&state, alice, &[(bob, Role::Editor)]).await;
        let channel = public_channel(&state, bob, space).await;
        let meta = channel.meta.document_id();
        open_log(&state, meta, bob).await;

        let entry = signed_entry(meta, 1, alice, MembershipAct::SelfJoin, Role::Owner);
        let answer = join_channel(
            State(state.clone()),
            session(alice),
            Path(channel.id.to_string()),
            Json(JoinChannelRequest {
                doc_key_enc: base64::engine::general_purpose::URL_SAFE.encode([1; 48]),
                entry: Some(entry),
            }),
        )
        .await;
        assert_eq!(status(answer), StatusCode::BAD_REQUEST);
        assert_eq!(role_on(&state, meta, alice).await, None);
    }

    /// The head of the log of `meta` reaches a caller the self-join would
    /// admit, and a stranger to the space gets the self-join's 403.
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn the_head_of_the_log_reaches_a_caller_the_self_join_would_admit(pool: sqlx::PgPool) {
        let state = crate::test_helpers::test_app_state(pool).await;
        let alice = user(&state, "alice@test.dev").await;
        let bob = user(&state, "bob@test.dev").await;
        let eve = user(&state, "eve@test.dev").await;
        let space = space(&state, alice, &[(bob, Role::Editor)]).await;
        let channel = public_channel(&state, bob, space).await;
        open_log(&state, channel.meta.document_id(), bob).await;

        let head = |who| {
            join_channel_head(
                State(state.clone()),
                session(who),
                Path(channel.id.to_string()),
            )
        };
        assert_eq!(status(head(eve).await), StatusCode::FORBIDDEN);
        // The head names the log and the role the entry must carry: the
        // space owner joins as an editor, after the channel's `Create`.
        let axum::Json(answer) = head(alice).await.expect("head");
        assert_eq!(answer.meta_id, channel.meta);
        assert_eq!(answer.role, Role::Editor);
        assert_eq!(answer.length, 1);
        assert!(answer.last.is_some());
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

    /// The links of a channel live under its `meta` document, and a segment
    /// reads them from there. The bytes stand for an encoded link: the
    /// relay stores them and opens none.
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn a_channel_link_is_kept_under_meta_and_served_for_a_segment(pool: sqlx::PgPool) {
        let state = crate::test_helpers::test_app_state(pool).await;
        let alice = user(&state, "alice@test.dev").await;
        let space = space(&state, alice, &[]).await;
        let channel = public_channel(&state, alice, space).await;
        sqlx::query(
            "INSERT INTO document_key_links (document_id, key_id, previous_key_id, link) \
             VALUES ($1, $2, $3, $4)",
        )
        .bind(uuid_of_doc(channel.meta.document_id()))
        .bind([2u8; 16].as_slice())
        .bind([1u8; 16].as_slice())
        .bind(b"link".as_slice())
        .execute(&state.db)
        .await
        .expect("insert a link");
        let served = db::document_key_links(&state.db, &channel.segment.document_id())
            .await
            .expect("links");
        assert_eq!(served, vec![b"link".to_vec()]);
        let alone = document(&state, alice).await;
        assert!(
            db::document_key_links(&state.db, &alone)
                .await
                .expect("links")
                .is_empty(),
            "a document outside the channel reads its own links"
        );
    }

    // --- The Murmure channels spec: private channels, leaves and deletes ---

    /// A private channel of `space`, registered by `creator`.
    async fn private_channel(state: &Arc<AppState>, creator: UserId, space: SpaceId) -> Channel {
        let channel = new_channel(state, creator).await;
        assert_eq!(
            register_as(state, creator, space, &channel, false).await,
            StatusCode::NO_CONTENT
        );
        channel
    }

    /// `who` invites `invitee` into `document` as an editor, with a copy of
    /// the key `key` names.
    async fn invite(
        state: &Arc<AppState>,
        who: UserId,
        document: DocumentId,
        invitee: &str,
        key: [u8; 16],
    ) -> StatusCode {
        status(
            crate::routes::share::invite_to_document(
                State(state.clone()),
                session(who),
                Path(document.to_string()),
                Json(crate::routes::share::InviteRequest {
                    invitee_email: invitee.into(),
                    role: Role::Editor,
                    doc_key_enc: base64::engine::general_purpose::URL_SAFE.encode([9u8; 48]),
                    key_id: ambre_core::KeyId::from_array(key),
                    entry: None,
                }),
            )
            .await,
        )
    }

    /// An invite on a private channel's `meta` opens every document of the
    /// channel to the invitee, with the key links kept under `meta`. One on
    /// the segment is refused, and adds no row anywhere.
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn an_invite_on_meta_opens_every_document_of_the_channel(pool: sqlx::PgPool) {
        let state = crate::test_helpers::test_app_state(pool).await;
        let alice = user(&state, "alice@test.dev").await;
        let carol = user(&state, "carol@test.dev").await;
        let space = space(&state, alice, &[]).await;
        let channel = private_channel(&state, alice, space).await;
        let meta = channel.meta.document_id();
        let segment = channel.segment.document_id();
        sqlx::query(
            "INSERT INTO document_key_links (document_id, key_id, previous_key_id, link) \
             VALUES ($1, $2, $3, $4)",
        )
        .bind(uuid_of_doc(meta))
        .bind([2u8; 16].as_slice())
        .bind([1u8; 16].as_slice())
        .bind(b"link".as_slice())
        .execute(&state.db)
        .await
        .expect("insert a link");

        assert_eq!(
            invite(&state, alice, segment, "carol@test.dev", [2; 16]).await,
            StatusCode::CONFLICT
        );
        let rows: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM document_members WHERE user_id = $1")
                .bind(uuid::Uuid::from_bytes(*carol.as_bytes()))
                .fetch_one(&state.db)
                .await
                .expect("count");
        assert_eq!(rows, 0, "the refused invite adds no row");

        assert_eq!(
            invite(&state, alice, meta, "carol@test.dev", [2; 16]).await,
            StatusCode::OK
        );
        let row = db::get_document_member(&state.db, &segment, &carol)
            .await
            .expect("query")
            .expect("carol reaches the segment");
        assert_eq!(row.doc_key_enc, vec![9u8; 48]);
        assert_eq!(
            db::document_key_links(&state.db, &segment)
                .await
                .expect("links"),
            vec![b"link".to_vec()],
            "the chain under meta serves the segment"
        );
    }

    /// A member of a space lists the channels the relay holds public in it.
    /// A private one, a deleted one and one of another space stay out. A
    /// stranger to the space, and a malformed id, get a 403.
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn a_member_of_a_space_lists_the_public_channels_it_holds(pool: sqlx::PgPool) {
        let state = crate::test_helpers::test_app_state(pool).await;
        let alice = user(&state, "alice@test.dev").await;
        let bob = user(&state, "bob@test.dev").await;
        let mallory = user(&state, "mallory@test.dev").await;
        let space = space(&state, alice, &[(bob, Role::Viewer)]).await;
        let other = self::space(&state, alice, &[]).await;
        let mut held = vec![
            public_channel(&state, alice, space).await.id,
            public_channel(&state, alice, space).await.id,
        ];
        held.sort_by_key(|id| *id.as_bytes());
        private_channel(&state, alice, space).await;
        public_channel(&state, alice, other).await;
        let gone = public_channel(&state, alice, space).await;
        assert_eq!(delete(&state, alice, gone.id).await, StatusCode::NO_CONTENT);

        let list = |who: UserId, id: String| {
            list_space_channels(State(state.clone()), session(who), Path(id))
        };
        let listed = list(bob, space.to_string()).await.expect("listed").0;
        assert_eq!(listed.channels, held);
        assert_eq!(
            status(list(mallory, space.to_string()).await),
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            status(list(bob, "not-a-space".into()).await),
            StatusCode::FORBIDDEN
        );
    }

    async fn delete(state: &Arc<AppState>, who: UserId, channel: ChannelId) -> StatusCode {
        status(
            delete_channel(
                State(state.clone()),
                session(who),
                Path(channel.to_string()),
            )
            .await,
        )
    }

    /// `who` leaves the channel, posting no rotation.
    async fn leave(
        state: &AppState,
        who: UserId,
        channel: &Channel,
    ) -> Result<db::Removed, AppError> {
        db::remove_document_member(
            &state.db,
            &channel.meta.document_id(),
            &who,
            None,
            crate::deletion::sweep_due_stored(state),
        )
        .await
    }

    async fn exists(state: &AppState, document: DocumentId) -> bool {
        db::get_document(&state.db, &document)
            .await
            .expect("query")
            .is_some()
    }

    async fn owes(state: &AppState, channel: &Channel) -> bool {
        db::rotation_owed(&state.db, &channel.meta.document_id())
            .await
            .expect("query")
    }

    /// A viewer of the space creates a private channel. The relay holds it
    /// in that space and not public, with its creator as its one member and
    /// owner. The space document keeps its members.
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn a_private_channel_has_its_creator_alone_and_names_its_space(pool: sqlx::PgPool) {
        let state = crate::test_helpers::test_app_state(pool).await;
        let alice = user(&state, "alice@test.dev").await;
        let vera = user(&state, "vera@test.dev").await;
        let space = space(&state, alice, &[(vera, Role::Viewer)]).await;
        let before = member_ids(&state, space.document_id()).await;

        let channel = private_channel(&state, vera, space).await;

        let held = db::get_channel(&state.db, &channel.id)
            .await
            .expect("query")
            .expect("the channel");
        assert_eq!(held.space_id, Some(space));
        assert!(!held.is_public);
        assert_eq!(held.meta_id, Some(channel.meta));
        assert_eq!(member_ids(&state, channel.meta.document_id()).await, [vera]);
        assert_eq!(
            role_on(&state, channel.meta.document_id(), vera).await,
            Some(Role::Owner)
        );
        assert_eq!(member_ids(&state, space.document_id()).await, before);
    }

    /// A private channel names a space its creator belongs to, and nobody
    /// joins it alone.
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn a_private_channel_needs_a_member_of_its_space_and_admits_no_self_join(
        pool: sqlx::PgPool,
    ) {
        let state = crate::test_helpers::test_app_state(pool).await;
        let alice = user(&state, "alice@test.dev").await;
        let bob = user(&state, "bob@test.dev").await;
        let mallory = user(&state, "mallory@test.dev").await;
        let space = space(&state, alice, &[(bob, Role::Editor)]).await;

        let stranger = new_channel(&state, mallory).await;
        assert_eq!(
            register_as(&state, mallory, space, &stranger, false).await,
            StatusCode::FORBIDDEN
        );
        let channel = private_channel(&state, alice, space).await;
        assert_eq!(
            join(&state, bob, channel.id, 9).await,
            StatusCode::FORBIDDEN
        );
        assert_eq!(role_on(&state, channel.meta.document_id(), bob).await, None);
    }

    /// The owner deletes a channel: every document it groups goes, with every
    /// object under each. Another channel keeps its documents and objects.
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn deleting_a_channel_deletes_its_documents_and_their_objects(pool: sqlx::PgPool) {
        let stub = crate::test_helpers::S3Stub::spawn([]).await;
        let base = crate::test_helpers::test_app_state(pool).await;
        let state = crate::test_helpers::with_s3(
            &base,
            crate::test_helpers::offline_s3(Some(&stub.endpoint)),
        );
        let alice = user(&state, "alice@test.dev").await;
        let space = space(&state, alice, &[]).await;
        let gone = private_channel(&state, alice, space).await;
        let kept = public_channel(&state, alice, space).await;
        let env = &state.config.s3_env_prefix;
        let documents = |c: &Channel| [c.meta.document_id(), c.segment.document_id()];
        for document in documents(&gone).into_iter().chain(documents(&kept)) {
            stub.put(crate::s3::format_s3_delta_key(env, &document, 1));
        }

        assert_eq!(delete(&state, alice, gone.id).await, StatusCode::NO_CONTENT);

        assert!(
            db::get_channel(&state.db, &gone.id)
                .await
                .expect("query")
                .is_none()
        );
        for document in documents(&gone) {
            assert!(!exists(&state, document).await);
            assert!(crate::test_helpers::deletion_marked(&state.db, document).await);
        }
        crate::test_helpers::make_sweeps_due(&state.db).await;
        crate::deletion::finish_deletions(&state).await;
        for document in documents(&gone) {
            assert!(!crate::test_helpers::deletion_marked(&state.db, document).await);
        }
        let mut left = stub.keys();
        left.sort();
        let mut expected: Vec<String> = documents(&kept)
            .iter()
            .map(|d| crate::s3::format_s3_delta_key(env, d, 1))
            .collect();
        expected.sort();
        assert_eq!(left, expected);
        for document in documents(&kept) {
            assert!(exists(&state, document).await);
        }
        assert_eq!(
            db::list_channel_documents(&state.db, &kept.id)
                .await
                .expect("query")
                .len(),
            2
        );
    }

    /// Only an owner of the channel deletes it. An editor, a stranger and an
    /// unknown channel get the same 403, and nothing goes.
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn a_delete_from_anyone_but_an_owner_is_refused(pool: sqlx::PgPool) {
        let state = crate::test_helpers::test_app_state(pool).await;
        let alice = user(&state, "alice@test.dev").await;
        let bob = user(&state, "bob@test.dev").await;
        let mallory = user(&state, "mallory@test.dev").await;
        let space = space(&state, alice, &[(bob, Role::Editor)]).await;
        let channel = public_channel(&state, alice, space).await;
        assert_eq!(
            join(&state, bob, channel.id, 9).await,
            StatusCode::NO_CONTENT
        );

        assert_eq!(delete(&state, bob, channel.id).await, StatusCode::FORBIDDEN);
        assert_eq!(
            delete(&state, mallory, channel.id).await,
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            delete(&state, alice, ChannelId::new()).await,
            StatusCode::FORBIDDEN
        );
        assert!(exists(&state, channel.meta.document_id()).await);
        assert!(exists(&state, channel.segment.document_id()).await);
    }

    /// The last owner of a public channel leaves it while a member stays.
    /// From then on an owner of the space deletes it, and an editor of the
    /// space does not. An owner of the space deletes no private channel.
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn an_owner_of_the_space_deletes_a_public_channel_its_last_owner_left(
        pool: sqlx::PgPool,
    ) {
        let state = crate::test_helpers::test_app_state(pool).await;
        let alice = user(&state, "alice@test.dev").await;
        let bob = user(&state, "bob@test.dev").await;
        let carol = user(&state, "carol@test.dev").await;
        let space = space(&state, alice, &[(bob, Role::Editor), (carol, Role::Editor)]).await;
        let channel = public_channel(&state, bob, space).await;
        let meta = channel.meta.document_id();
        assert_eq!(
            join(&state, carol, channel.id, 9).await,
            StatusCode::NO_CONTENT
        );

        assert_eq!(
            leave(&state, bob, &channel).await.expect("leave"),
            db::Removed::Kept
        );
        assert_eq!(member_ids(&state, meta).await, [carol]);
        assert_eq!(
            delete(&state, carol, channel.id).await,
            StatusCode::FORBIDDEN
        );
        let private = private_channel(&state, bob, space).await;
        assert_eq!(
            delete(&state, alice, private.id).await,
            StatusCode::FORBIDDEN
        );
        assert!(exists(&state, private.meta.document_id()).await);

        assert_eq!(
            delete(&state, alice, channel.id).await,
            StatusCode::NO_CONTENT
        );
        let held = db::get_channel(&state.db, &channel.id)
            .await
            .expect("query");
        assert!(held.is_none());
        assert!(crate::test_helpers::deletion_marked(&state.db, meta).await);
    }

    /// A member who leaves a private channel leaves its key owed. A member
    /// who leaves a public channel leaves it as it was.
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn a_leave_owes_a_rotation_on_a_private_channel_only(pool: sqlx::PgPool) {
        let state = crate::test_helpers::test_app_state(pool).await;
        let alice = user(&state, "alice@test.dev").await;
        let bob = user(&state, "bob@test.dev").await;
        let space = space(&state, alice, &[(bob, Role::Editor)]).await;
        let private = private_channel(&state, alice, space).await;
        grant(&state, private.meta.document_id(), bob, Role::Editor).await;
        let public = public_channel(&state, alice, space).await;
        assert_eq!(
            join(&state, bob, public.id, 9).await,
            StatusCode::NO_CONTENT
        );

        assert_eq!(
            leave(&state, bob, &private).await.expect("leave"),
            db::Removed::Owed
        );
        assert!(owes(&state, &private).await);
        assert_eq!(
            leave(&state, bob, &public).await.expect("leave"),
            db::Removed::Kept
        );
        assert!(!owes(&state, &public).await);
    }

    /// The last member of a private channel leaves, its owner included, and
    /// the channel goes with every document. Another channel stays.
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn the_last_member_leaving_a_private_channel_deletes_it(pool: sqlx::PgPool) {
        let state = crate::test_helpers::test_app_state(pool).await;
        let alice = user(&state, "alice@test.dev").await;
        let space = space(&state, alice, &[]).await;
        let channel = private_channel(&state, alice, space).await;
        let other = private_channel(&state, alice, space).await;

        let removed = leave(&state, alice, &channel).await.expect("leave");

        let mut expected = vec![channel.meta.document_id(), channel.segment.document_id()];
        expected.sort_by_key(|d| *d.as_bytes());
        let db::Removed::ChannelDeleted(mut deleted) = removed else {
            panic!("the channel stays: {removed:?}");
        };
        deleted.sort_by_key(|d| *d.as_bytes());
        assert_eq!(deleted, expected);
        assert!(
            db::get_channel(&state.db, &channel.id)
                .await
                .expect("query")
                .is_none()
        );
        // The removal marks the documents, and the route empties them: the
        // rows stay until then (`SRV-165`).
        for document in expected {
            assert!(crate::test_helpers::deletion_marked(&state.db, document).await);
        }
        assert!(exists(&state, other.meta.document_id()).await);
        assert_eq!(
            role_on(&state, other.meta.document_id(), alice).await,
            Some(Role::Owner)
        );
    }

    /// `SRV-187`: the last member of a private channel leaves through the
    /// removal route, which deletes it. Their own topic still hears that they
    /// left, so their other clients drop the channel.
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn a_leave_that_deletes_its_channel_still_reaches_its_member(pool: sqlx::PgPool) {
        use ambre_protocol::ws::{WsServerMsg, decode_server_msg};
        use futures_util::StreamExt;
        use rustis::commands::PubSubCommands;

        let state = crate::test_helpers::test_app_state(pool).await;
        let alice = user(&state, "alice@test.dev").await;
        let space = space(&state, alice, &[]).await;
        let channel = private_channel(&state, alice, space).await;
        let meta = channel.meta.document_id();

        let listener = rustis::client::Client::connect("redis://localhost:6379")
            .await
            .expect("redis");
        let mut stream = listener
            .subscribe(format!("user:{alice}"))
            .await
            .expect("subscribe");
        let answer = crate::routes::share::revoke_from_document(
            State(state.clone()),
            session(alice),
            Path((meta.to_string(), alice.to_string())),
            None,
        )
        .await;
        assert_eq!(status(answer), StatusCode::OK);
        let held = db::get_channel(&state.db, &channel.id)
            .await
            .expect("query");
        assert!(held.is_none(), "the channel stayed");

        let heard = tokio::time::timeout(std::time::Duration::from_secs(5), stream.next())
            .await
            .expect("the notice arrives")
            .expect("a message")
            .expect("readable");
        let notice = decode_server_msg(heard.payload()).expect("decodes");
        assert!(
            matches!(notice, WsServerMsg::DocumentRevoked { document_id } if document_id == meta),
            "heard {notice:?}"
        );
    }

    /// `SRV-188`: a delete reaches the topic of each member of the channel,
    /// the owner who asked it included. A member of the space outside the
    /// channel hears nothing.
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn a_delete_reaches_every_member_of_its_channel(pool: sqlx::PgPool) {
        use ambre_protocol::ws::{WsServerMsg, decode_server_msg};
        use futures_util::StreamExt;
        use rustis::commands::PubSubCommands;

        let state = crate::test_helpers::test_app_state(pool).await;
        let alice = user(&state, "alice@test.dev").await;
        let bob = user(&state, "bob@test.dev").await;
        let carol = user(&state, "carol@test.dev").await;
        let space = space(&state, alice, &[(bob, Role::Editor), (carol, Role::Editor)]).await;
        let channel = public_channel(&state, alice, space).await;
        let meta = channel.meta.document_id();
        assert_eq!(
            join(&state, bob, channel.id, 9).await,
            StatusCode::NO_CONTENT
        );

        let mut topics = Vec::new();
        for member in [alice, bob, carol] {
            let listener = rustis::client::Client::connect("redis://localhost:6379")
                .await
                .expect("redis");
            topics.push(
                listener
                    .subscribe(format!("user:{member}"))
                    .await
                    .expect("subscribe"),
            );
        }
        assert_eq!(
            delete(&state, alice, channel.id).await,
            StatusCode::NO_CONTENT
        );

        let mut outside = topics.pop().expect("carol's topic");
        for mut stream in topics {
            let heard = tokio::time::timeout(std::time::Duration::from_secs(5), stream.next())
                .await
                .expect("the notice arrives")
                .expect("a message")
                .expect("readable");
            let notice = decode_server_msg(heard.payload()).expect("decodes");
            assert!(
                matches!(notice, WsServerMsg::DocumentRevoked { document_id } if document_id == meta),
                "heard {notice:?}"
            );
        }
        let silent =
            tokio::time::timeout(std::time::Duration::from_millis(300), outside.next()).await;
        assert!(silent.is_err(), "a member outside the channel heard it");
    }

    /// An editor leaves a public channel, then its owner, the last member.
    /// Neither is refused, and the channel stays with its documents and
    /// owes no rotation. Another member of the space then joins it alone.
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn the_last_member_leaves_a_public_channel_for_the_next_to_join(pool: sqlx::PgPool) {
        let state = crate::test_helpers::test_app_state(pool).await;
        let alice = user(&state, "alice@test.dev").await;
        let bob = user(&state, "bob@test.dev").await;
        let carol = user(&state, "carol@test.dev").await;
        let space = space(&state, alice, &[(bob, Role::Editor), (carol, Role::Editor)]).await;
        let channel = public_channel(&state, alice, space).await;
        assert_eq!(
            join(&state, bob, channel.id, 9).await,
            StatusCode::NO_CONTENT
        );
        let meta = channel.meta.document_id();

        assert_eq!(
            leave(&state, bob, &channel).await.expect("leave"),
            db::Removed::Kept
        );
        assert_eq!(
            leave(&state, alice, &channel)
                .await
                .expect("the owner leaves last"),
            db::Removed::Kept
        );

        assert!(member_ids(&state, meta).await.is_empty());
        assert!(!owes(&state, &channel).await);
        assert!(
            db::get_channel(&state.db, &channel.id)
                .await
                .expect("query")
                .is_some_and(|c| c.is_public)
        );
        for document in [meta, channel.segment.document_id()] {
            assert!(exists(&state, document).await);
            assert!(!crate::test_helpers::deletion_marked(&state.db, document).await);
        }
        assert_eq!(
            join(&state, carol, channel.id, 7).await,
            StatusCode::NO_CONTENT
        );
        assert_eq!(role_on(&state, meta, carol).await, Some(Role::Editor));
    }

    /// The last owner of a private channel that keeps other members is
    /// refused the leave, and stays. Once the role is handed on, the leave
    /// goes through.
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn the_last_owner_hands_the_role_on_before_leaving(pool: sqlx::PgPool) {
        let state = crate::test_helpers::test_app_state(pool).await;
        let alice = user(&state, "alice@test.dev").await;
        let bob = user(&state, "bob@test.dev").await;
        let space = space(&state, alice, &[(bob, Role::Editor)]).await;
        let channel = private_channel(&state, alice, space).await;
        let meta = channel.meta.document_id();
        grant(&state, meta, bob, Role::Editor).await;

        let refused = leave(&state, alice, &channel).await;
        assert!(
            matches!(refused, Err(AppError::Conflict { .. })),
            "{refused:?}"
        );
        assert_eq!(role_on(&state, meta, alice).await, Some(Role::Owner));

        db::update_document_member_role(&state.db, &meta, &bob, Role::Owner, None)
            .await
            .expect("hand on");
        assert_eq!(
            leave(&state, alice, &channel).await.expect("leave"),
            db::Removed::Owed
        );
        assert!(exists(&state, meta).await);
        assert_eq!(member_ids(&state, meta).await, [bob]);
    }

    // --- Direct messages (`SRV-175`, `SRV-176`) ---

    /// The request of `creator` for a direct message with `others`, under
    /// fresh ids. Its log creates `meta`, invites each member as an editor,
    /// then sets the creator's own role to editor. Each member's key is
    /// filled with a byte of its own, so a test tells the wraps apart.
    fn direct_request(creator: UserId, others: &[UserId]) -> CreateDirectMessageRequest {
        let meta = MetaId::from_document(DocumentId::new());
        let entries = ambre_protocol::ownerless_log(creator, others)
            .zip(0u64..)
            .map(|((member, act, role), position)| {
                let key = (act != MembershipAct::ChangeRole).then_some(DIRECT_KEY);
                authored_entry(
                    meta.document_id(),
                    position,
                    member,
                    act,
                    role,
                    key,
                    creator,
                )
            })
            .collect();
        let key = |byte: u8| base64::engine::general_purpose::URL_SAFE.encode([byte; 48]);
        CreateDirectMessageRequest {
            channel_id: ChannelId::new(),
            meta_id: meta,
            segment_id: SegmentId::from_document(DocumentId::new()),
            members: others
                .iter()
                .zip(2u8..)
                .map(|(&user_id, byte)| DirectMemberJson {
                    user_id,
                    doc_key_enc: key(byte),
                })
                .collect(),
            doc_key_enc: key(1),
            key_id: DIRECT_KEY,
            entries,
        }
    }

    /// The channel key every test direct message names.
    const DIRECT_KEY: ambre_core::KeyId = ambre_core::KeyId::from_array([8; 16]);

    /// The entry `author` signs about `member` at `position` of `document`'s
    /// log, in the form a request carries it. The relay walks no digest, so
    /// the previous one is filler.
    fn authored_entry(
        document: DocumentId,
        position: u64,
        member: UserId,
        act: MembershipAct,
        role: Role,
        key: Option<ambre_core::KeyId>,
        author: UserId,
    ) -> String {
        use ed25519_dalek::Signer;
        let signing = signing_key();
        let mut entry = ambre_protocol::MembershipEntry {
            document,
            position,
            member,
            member_key: signing.verifying_key().to_bytes(),
            act,
            role,
            key,
            previous: (position > 0).then_some(ambre_protocol::EntryDigest([1; 32])),
            author,
            signature: Vec::new(),
        };
        entry.signature = signing
            .sign(&entry.signed_bytes().expect("signed bytes"))
            .to_bytes()
            .to_vec();
        base64::engine::general_purpose::URL_SAFE.encode(entry.encode().expect("encode"))
    }

    async fn open_direct(
        state: &Arc<AppState>,
        who: UserId,
        req: CreateDirectMessageRequest,
    ) -> Result<DirectMessageJson, AppError> {
        create_direct_message(State(state.clone()), session(who), Json(req))
            .await
            .map(|axum::Json(json)| json)
    }

    async fn direct_messages(state: &AppState) -> i64 {
        sqlx::query_scalar("SELECT COUNT(*) FROM channels WHERE space_id IS NULL")
            .fetch_one(&state.db)
            .await
            .expect("count")
    }

    /// `SRV-175`: a direct message belongs to no space and is not public.
    /// Every member is an editor of its `meta`, the creator included, so it
    /// has no owner, and nobody deletes it for the others.
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn a_direct_message_has_no_space_and_no_owner(pool: sqlx::PgPool) {
        let state = crate::test_helpers::test_app_state(pool).await;
        let alice = user(&state, "alice@test.dev").await;
        let bob = user(&state, "bob@test.dev").await;
        let carol = user(&state, "carol@test.dev").await;
        let req = direct_request(alice, &[bob, carol]);
        let (channel_id, meta) = (req.channel_id, req.meta_id.document_id());

        let answer = open_direct(&state, alice, req).await.expect("create");
        assert_eq!((answer.channel_id, answer.created), (channel_id, true));

        let channel = db::get_channel(&state.db, &channel_id)
            .await
            .expect("query")
            .expect("the channel exists");
        assert_eq!(channel.space_id, None);
        assert!(!channel.is_public);
        for who in [alice, bob, carol] {
            assert_eq!(role_on(&state, meta, who).await, Some(Role::Editor));
        }
        let log = db::membership_log(&state.db, &meta).await.expect("log");
        assert_eq!(log.len(), 4, "create, two invites, the creator's role");
        assert_eq!(
            delete(&state, alice, channel_id).await,
            StatusCode::FORBIDDEN
        );
        assert!(exists(&state, meta).await);
    }

    /// `SRV-175`: the relay stores, for each member, the key wrapped for
    /// that member, and reads none of them.
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn each_member_holds_the_key_wrapped_for_them(pool: sqlx::PgPool) {
        let state = crate::test_helpers::test_app_state(pool).await;
        let alice = user(&state, "alice@test.dev").await;
        let bob = user(&state, "bob@test.dev").await;
        let req = direct_request(alice, &[bob]);
        let meta = req.meta_id.document_id();
        open_direct(&state, alice, req).await.expect("create");

        for (who, byte) in [(alice, 1u8), (bob, 2)] {
            let row = db::get_document_member(&state.db, &meta, &who)
                .await
                .expect("query")
                .expect("a member");
            assert_eq!(row.doc_key_enc, [byte; 48]);
        }
    }

    /// `SRV-191`: a user lists each channel whose `meta` holds a row of
    /// theirs. Here that is a public and a private channel of a space, and a
    /// direct message with no space. A channel of someone else stays out.
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn a_user_lists_every_channel_they_are_a_member_of(pool: sqlx::PgPool) {
        let state = crate::test_helpers::test_app_state(pool).await;
        let alice = user(&state, "alice@test.dev").await;
        let bob = user(&state, "bob@test.dev").await;
        let carol = user(&state, "carol@test.dev").await;
        let space = space(&state, alice, &[(carol, Role::Editor)]).await;
        let public = public_channel(&state, alice, space).await;
        let private = private_channel(&state, alice, space).await;
        private_channel(&state, carol, space).await;
        let req = direct_request(alice, &[bob]);
        let direct = req.channel_id;
        open_direct(&state, alice, req).await.expect("create");

        let listed = |who: UserId| list_member_channels(State(state.clone()), session(who));
        let mut want = vec![
            (public.id, Some(space), true),
            (private.id, Some(space), false),
            (direct, None, false),
        ];
        want.sort_by_key(|(id, ..)| *id.as_bytes());
        let got: Vec<_> = listed(alice)
            .await
            .expect("listed")
            .0
            .channels
            .into_iter()
            .map(|c| (c.channel_id, c.space_id, c.public))
            .collect();
        assert_eq!(got, want);
        let bobs: Vec<_> = listed(bob)
            .await
            .expect("listed")
            .0
            .channels
            .into_iter()
            .map(|c| (c.channel_id, c.space_id, c.public))
            .collect();
        assert_eq!(bobs, vec![(direct, None, false)]);
    }

    /// The next message on `stream`, decoded, or `None` after `millis`.
    async fn next_notice(
        stream: &mut rustis::client::PubSubStream,
        millis: u64,
    ) -> Option<ambre_protocol::ws::WsServerMsg> {
        use futures_util::StreamExt;
        let heard = tokio::time::timeout(std::time::Duration::from_millis(millis), stream.next())
            .await
            .ok()?
            .expect("a message")
            .expect("readable");
        Some(ambre_protocol::ws::decode_server_msg(heard.payload()).expect("decodes"))
    }

    /// `SRV-192`: a new direct message reaches the topic of each of its
    /// members, its creator included, as the share of its `meta`. The same
    /// request again finds the channel and says nothing. An invite into a
    /// private channel reaches the invitee the same way.
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn a_member_a_channel_adds_hears_the_share_of_its_meta(pool: sqlx::PgPool) {
        use ambre_protocol::ws::WsServerMsg;
        use rustis::commands::PubSubCommands;

        let state = crate::test_helpers::test_app_state(pool).await;
        let alice = user(&state, "alice@test.dev").await;
        let bob = user(&state, "bob@test.dev").await;
        let carol = user(&state, "carol@test.dev").await;
        let mut topics = Vec::new();
        for member in [alice, bob, carol] {
            let listener = rustis::client::Client::connect("redis://localhost:6379")
                .await
                .expect("redis");
            topics.push(
                listener
                    .subscribe(format!("user:{member}"))
                    .await
                    .expect("subscribe"),
            );
        }
        let shared = |notice: Option<WsServerMsg>, meta: DocumentId| {
            matches!(
                notice,
                Some(WsServerMsg::DocumentShared { document_id, doc_type })
                    if document_id == meta && doc_type == ambre_core::DocType::Murmure
            )
        };

        let req = direct_request(alice, &[bob]);
        let meta = req.meta_id.document_id();
        open_direct(&state, alice, req).await.expect("create");
        for stream in &mut topics[..2] {
            assert!(shared(next_notice(stream, 5000).await, meta));
        }
        open_direct(&state, bob, direct_request(bob, &[alice]))
            .await
            .expect("open");
        for stream in &mut topics {
            assert!(next_notice(stream, 300).await.is_none(), "a reuse spoke");
        }

        let space = space(&state, alice, &[]).await;
        let channel = private_channel(&state, alice, space).await;
        let meta = channel.meta.document_id();
        assert_eq!(
            invite(&state, alice, meta, "carol@test.dev", [6; 16]).await,
            StatusCode::OK
        );
        assert!(shared(next_notice(&mut topics[2], 5000).await, meta));
    }

    /// `SRV-176`: a request for a set of members that already has a direct
    /// message answers it, from either side. It writes nothing: no channel,
    /// no document, no member.
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn the_same_members_reach_the_same_direct_message(pool: sqlx::PgPool) {
        let state = crate::test_helpers::test_app_state(pool).await;
        let alice = user(&state, "alice@test.dev").await;
        let bob = user(&state, "bob@test.dev").await;
        let first = direct_request(alice, &[bob]);
        let (channel_id, meta) = (first.channel_id, first.meta_id.document_id());
        open_direct(&state, alice, first).await.expect("create");

        let again = direct_request(bob, &[alice]);
        let (other_id, other_meta) = (again.channel_id, again.meta_id.document_id());
        let answer = open_direct(&state, bob, again).await.expect("open");
        assert_eq!((answer.channel_id, answer.created), (channel_id, false));
        assert!(
            db::get_channel(&state.db, &other_id)
                .await
                .expect("query")
                .is_none()
        );
        assert!(!exists(&state, other_meta).await);
        assert_eq!(member_ids(&state, meta).await.len(), 2);
        assert_eq!(
            db::membership_log(&state.db, &meta)
                .await
                .expect("log")
                .len(),
            3
        );
        assert_eq!(direct_messages(&state).await, 1);

        let wider = direct_request(alice, &[bob, user(&state, "carol@test.dev").await]);
        let answer = open_direct(&state, alice, wider).await.expect("create");
        assert!(answer.created, "one more member is another set");
        assert_eq!(direct_messages(&state).await, 2);
    }

    /// `SRV-176`: two requests racing for one set of members create one
    /// direct message, and both answer it.
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn racing_requests_create_one_direct_message(pool: sqlx::PgPool) {
        let state = crate::test_helpers::test_app_state(pool).await;
        let alice = user(&state, "alice@test.dev").await;
        let bob = user(&state, "bob@test.dev").await;
        let (a, b) = tokio::join!(
            open_direct(&state, alice, direct_request(alice, &[bob])),
            open_direct(&state, bob, direct_request(bob, &[alice])),
        );
        let (a, b) = (a.expect("alice"), b.expect("bob"));
        assert_eq!(a.channel_id, b.channel_id);
        assert!(a.created != b.created, "exactly one created it");
        assert_eq!(direct_messages(&state).await, 1);
    }

    /// `SRV-175`: a log that leaves the creator an owner, or a request that
    /// names the caller as a member, is refused, and nothing is written.
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn a_direct_message_with_an_owner_is_refused(pool: sqlx::PgPool) {
        let state = crate::test_helpers::test_app_state(pool).await;
        let alice = user(&state, "alice@test.dev").await;
        let bob = user(&state, "bob@test.dev").await;

        let mut owned = direct_request(alice, &[bob]);
        owned.entries.pop();
        assert!(matches!(
            open_direct(&state, alice, owned).await,
            Err(AppError::BadRequest(_))
        ));
        let mut swapped = direct_request(alice, &[bob]);
        let last = swapped.entries.len() - 1;
        swapped.entries[last] = authored_entry(
            swapped.meta_id.document_id(),
            u64::try_from(last).expect("position"),
            alice,
            MembershipAct::ChangeRole,
            Role::Owner,
            None,
            alice,
        );
        assert!(matches!(
            open_direct(&state, alice, swapped).await,
            Err(AppError::BadRequest(_))
        ));
        let with_self = direct_request(alice, &[alice]);
        assert!(matches!(
            open_direct(&state, alice, with_self).await,
            Err(AppError::BadRequest(_))
        ));
        assert_eq!(direct_messages(&state).await, 0);
    }

    // --- The Murmure spaces spec: leaving and deleting a space ---

    /// A relay state whose object store is a fake, for a delete to empty.
    async fn with_store(pool: sqlx::PgPool) -> (Arc<AppState>, crate::test_helpers::S3Stub) {
        let stub = crate::test_helpers::S3Stub::spawn([]).await;
        let base = crate::test_helpers::test_app_state(pool).await;
        let state = crate::test_helpers::with_s3(
            &base,
            crate::test_helpers::offline_s3(Some(&stub.endpoint)),
        );
        (state, stub)
    }

    /// `who` leaves `space`, posting no rotation.
    async fn leave_space(state: &Arc<AppState>, who: UserId, space: SpaceId) -> StatusCode {
        status(
            crate::routes::share::revoke_from_document(
                State(state.clone()),
                session(who),
                Path((space.document_id().to_string(), who.to_string())),
                None,
            )
            .await,
        )
    }

    /// The documents of `channel`.
    fn documents_of(channel: &Channel) -> [DocumentId; 2] {
        [channel.meta.document_id(), channel.segment.document_id()]
    }

    /// Bob in a public channel, a private one Alice shares with him, and a
    /// private one he alone holds, all in `space`.
    async fn bobs_channels(
        state: &Arc<AppState>,
        alice: UserId,
        bob: UserId,
        space: SpaceId,
    ) -> [Channel; 3] {
        let public = public_channel(state, alice, space).await;
        assert_eq!(join(state, bob, public.id, 3).await, StatusCode::NO_CONTENT);
        let shared = private_channel(state, alice, space).await;
        let invited = invite(
            state,
            alice,
            shared.meta.document_id(),
            "bob@test.dev",
            [5; 16],
        );
        assert!(invited.await.is_success());
        let own = private_channel(state, bob, space).await;
        [public, shared, own]
    }

    /// `SRV-136`: a member who leaves a space leaves every channel of it,
    /// private ones included. Each private channel that stays owes a
    /// rotation, and one they were the last member of goes. A join racing
    /// the leave is refused under the lock on the space.
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn leaving_a_space_leaves_every_channel_of_it(pool: sqlx::PgPool) {
        let (state, _stub) = with_store(pool).await;
        let alice = user(&state, "alice@test.dev").await;
        let bob = user(&state, "bob@test.dev").await;
        let space = space(&state, alice, &[(bob, Role::Editor)]).await;
        let [public, shared, own] = bobs_channels(&state, alice, bob, space).await;

        assert_eq!(leave_space(&state, bob, space).await, StatusCode::OK);

        assert_eq!(member_ids(&state, public.meta.document_id()).await, [alice]);
        assert_eq!(member_ids(&state, shared.meta.document_id()).await, [alice]);
        assert!(owes(&state, &shared).await);
        assert!(!owes(&state, &public).await);
        let channel = db::get_channel(&state.db, &own.id).await.expect("query");
        assert!(channel.is_none());
        for document in documents_of(&own) {
            assert!(!exists(&state, document).await);
        }
        let space_owes = db::rotation_owed(&state.db, &space.document_id()).await;
        assert!(space_owes.expect("query"));
        let late = db::add_channel_member_if_absent(
            &state.db,
            &space,
            &public.meta,
            &bob,
            Role::Editor,
            &[1; 48],
            None,
        )
        .await;
        assert!(matches!(late, Err(AppError::Forbidden)));
        assert_eq!(member_ids(&state, public.meta.document_id()).await, [alice]);
    }

    /// `SRV-136`: the owner who removes a member from a space with its
    /// rotation takes them out of every channel of it too.
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn a_removal_from_a_space_leaves_every_channel_of_it(pool: sqlx::PgPool) {
        let (state, _stub) = with_store(pool).await;
        let alice = user(&state, "alice@test.dev").await;
        let bob = user(&state, "bob@test.dev").await;
        let space = space(&state, alice, &[(bob, Role::Editor)]).await;
        let [public, shared, own] = bobs_channels(&state, alice, bob, space).await;
        let link = ambre_protocol::KeyLink {
            document_id: space.document_id(),
            key: ambre_core::KeyId::from_array([2; 16]),
            previous: ambre_core::KeyId::from_array([1; 16]),
            sealed: vec![2; 200],
        };
        let revoked = crate::routes::share::revoke_and_rotate(
            State(state.clone()),
            session(alice),
            Path(space.document_id().to_string()),
            Json(crate::routes::share::RevokeRequest {
                user_id: bob,
                keys: vec![crate::routes::share::RotateKeyEntry {
                    user_id: alice,
                    doc_key_enc: base64::engine::general_purpose::URL_SAFE.encode([4u8; 48]),
                }],
                members: vec![alice],
                link: base64::engine::general_purpose::URL_SAFE
                    .encode(link.encode().expect("encode")),
                entry: None,
            }),
        )
        .await;

        assert_eq!(status(revoked), StatusCode::OK);
        assert_eq!(member_ids(&state, space.document_id()).await, [alice]);
        assert_eq!(member_ids(&state, public.meta.document_id()).await, [alice]);
        assert_eq!(member_ids(&state, shared.meta.document_id()).await, [alice]);
        assert!(owes(&state, &shared).await);
        let channel = db::get_channel(&state.db, &own.id).await.expect("query");
        assert!(channel.is_none());
    }

    /// Leaving a space touches neither a direct message nor a channel of
    /// another space (the Murmure spaces spec).
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn leaving_a_space_keeps_direct_messages_and_other_spaces(pool: sqlx::PgPool) {
        let (state, _stub) = with_store(pool).await;
        let alice = user(&state, "alice@test.dev").await;
        let bob = user(&state, "bob@test.dev").await;
        let left = space(&state, alice, &[(bob, Role::Editor)]).await;
        let other = space(&state, alice, &[(bob, Role::Editor)]).await;
        let elsewhere = public_channel(&state, alice, other).await;
        assert_eq!(
            join(&state, bob, elsewhere.id, 3).await,
            StatusCode::NO_CONTENT
        );
        let direct = direct_request(alice, &[bob]);
        let direct_meta = direct.meta_id.document_id();
        open_direct(&state, alice, direct).await.expect("create");

        assert_eq!(leave_space(&state, bob, left).await, StatusCode::OK);

        let mut both = [alice, bob];
        both.sort_by_key(|id| *id.as_bytes());
        assert_eq!(member_ids(&state, elsewhere.meta.document_id()).await, both);
        assert_eq!(member_ids(&state, direct_meta).await, both);
        assert!(!owes(&state, &elsewhere).await);
    }

    async fn delete_space(state: &Arc<AppState>, who: UserId, space: SpaceId) -> StatusCode {
        status(
            crate::routes::delete_document(
                State(state.clone()),
                session(who),
                Path(space.document_id().to_string()),
            )
            .await,
        )
    }

    /// `SRV-143`: deleting a space deletes every channel of it with each of
    /// their documents, then the space. A direct message of its members
    /// stays, and so does a channel of another space.
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn deleting_a_space_deletes_every_channel_then_the_space(pool: sqlx::PgPool) {
        let (state, _stub) = with_store(pool).await;
        let alice = user(&state, "alice@test.dev").await;
        let bob = user(&state, "bob@test.dev").await;
        let gone = space(&state, alice, &[(bob, Role::Editor)]).await;
        let other = space(&state, alice, &[]).await;
        let channels = bobs_channels(&state, alice, bob, gone).await;
        let kept = public_channel(&state, alice, other).await;
        let direct = direct_request(alice, &[bob]);
        let direct_meta = direct.meta_id.document_id();
        open_direct(&state, alice, direct).await.expect("create");

        assert_eq!(delete_space(&state, alice, gone).await, StatusCode::OK);

        assert!(!exists(&state, gone.document_id()).await);
        for channel in &channels {
            let row = db::get_channel(&state.db, &channel.id)
                .await
                .expect("query");
            assert!(row.is_none());
            for document in documents_of(channel) {
                assert!(!exists(&state, document).await);
                assert!(crate::test_helpers::deletion_marked(&state.db, document).await);
            }
        }
        for document in documents_of(&kept).into_iter().chain([direct_meta]) {
            assert!(exists(&state, document).await);
        }
    }

    /// A space delete from an editor or a stranger is refused, and every
    /// channel of the space stays (the Murmure spaces spec).
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn a_space_delete_from_anyone_but_its_owner_is_refused(pool: sqlx::PgPool) {
        let (state, _stub) = with_store(pool).await;
        let alice = user(&state, "alice@test.dev").await;
        let bob = user(&state, "bob@test.dev").await;
        let mallory = user(&state, "mallory@test.dev").await;
        let space = space(&state, alice, &[(bob, Role::Editor)]).await;
        let channel = public_channel(&state, alice, space).await;

        for who in [bob, mallory] {
            assert_eq!(
                delete_space(&state, who, space).await,
                StatusCode::FORBIDDEN
            );
        }
        assert!(exists(&state, space.document_id()).await);
        for document in documents_of(&channel) {
            assert!(exists(&state, document).await);
        }
    }

    /// `who` re-keys `meta` from key 1 to key 2, wrapping for `members`.
    async fn rekey(
        state: &Arc<AppState>,
        who: UserId,
        meta: DocumentId,
        members: Vec<UserId>,
    ) -> StatusCode {
        let link = ambre_protocol::KeyLink {
            document_id: meta,
            key: ambre_core::KeyId::from_array([2; 16]),
            previous: ambre_core::KeyId::from_array([1; 16]),
            sealed: vec![2; 200],
        };
        let keys = members
            .iter()
            .map(|&user_id| crate::routes::share::RotateKeyEntry {
                user_id,
                doc_key_enc: base64::engine::general_purpose::URL_SAFE.encode([4u8; 48]),
            })
            .collect();
        status(
            crate::routes::share::rotate_document_key(
                State(state.clone()),
                session(who),
                Path(meta.to_string()),
                Json(crate::routes::share::RotateKeyRequest {
                    keys,
                    members,
                    link: base64::engine::general_purpose::URL_SAFE
                        .encode(link.encode().expect("encode")),
                }),
            )
            .await,
        )
    }

    /// What `who` reads of `meta`: its member list, then its key links.
    async fn read_access(
        state: &Arc<AppState>,
        who: UserId,
        meta: DocumentId,
    ) -> (StatusCode, StatusCode) {
        let members = crate::routes::list_document_members(
            State(state.clone()),
            session(who),
            Path(meta.to_string()),
        )
        .await;
        let links = crate::routes::share::list_key_links(
            State(state.clone()),
            session(who),
            Path(meta.to_string()),
        )
        .await;
        (status(members), status(links))
    }

    /// `SRV-146`: an admin of a space re-keys a public channel of it, though
    /// no member of the channel. It reads the members and the links first. A
    /// viewer of the space is refused, and so is a private channel.
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn an_admin_of_the_space_rekeys_a_public_channel_of_it(pool: sqlx::PgPool) {
        let state = crate::test_helpers::test_app_state(pool).await;
        let alice = user(&state, "alice@test.dev").await;
        let bob = user(&state, "bob@test.dev").await;
        let vera = user(&state, "vera@test.dev").await;
        let space = space(&state, alice, &[(bob, Role::Editor), (vera, Role::Viewer)]).await;
        let public = public_channel(&state, alice, space).await;
        let private = private_channel(&state, alice, space).await;
        let meta = public.meta.document_id();

        assert_eq!(
            read_access(&state, bob, meta).await,
            (StatusCode::OK, StatusCode::OK)
        );
        assert_eq!(
            read_access(&state, vera, meta).await,
            (StatusCode::NOT_FOUND, StatusCode::FORBIDDEN)
        );
        assert_eq!(
            rekey(&state, vera, meta, vec![alice]).await,
            StatusCode::FORBIDDEN
        );
        let private_meta = private.meta.document_id();
        assert_eq!(
            rekey(&state, bob, private_meta, vec![alice]).await,
            StatusCode::FORBIDDEN
        );
        assert_eq!(rekey(&state, bob, meta, vec![alice]).await, StatusCode::OK);
        assert_eq!(
            db::document_key_links(&state.db, &meta)
                .await
                .expect("links")
                .len(),
            1
        );
    }

    // --- Closing a segment into its successor ---

    async fn close(
        state: &Arc<AppState>,
        who: UserId,
        channel: &Channel,
        successor: DocumentId,
    ) -> Result<ClosedSegmentJson, StatusCode> {
        close_segment(
            State(state.clone()),
            session(who),
            Path(channel.id.to_string()),
            Json(CloseSegmentRequest {
                closing: channel.segment,
                successor: SegmentId::from_document(successor),
            }),
        )
        .await
        .map(|axum::Json(answer)| answer)
        .map_err(|e| e.into_response().status())
    }

    /// `SRV-194`, `SRV-195`: a writer closes the active segment into its own
    /// successor, and a second writer racing it is answered that successor.
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn a_writer_closes_a_segment_and_a_late_close_learns_the_winner(pool: sqlx::PgPool) {
        let state = crate::test_helpers::test_app_state(pool).await;
        let alice = user(&state, "alice@test.dev").await;
        let bob = user(&state, "bob@test.dev").await;
        let space = space(&state, alice, &[(bob, Role::Editor)]).await;
        let channel = public_channel(&state, alice, space).await;
        grant(&state, channel.meta.document_id(), bob, Role::Editor).await;

        let mine = document(&state, alice).await;
        let first = close(&state, alice, &channel, mine).await.expect("close");
        assert_eq!(first.active, SegmentId::from_document(mine));
        assert!(first.created);

        let theirs = document(&state, bob).await;
        let late = close(&state, bob, &channel, theirs).await.expect("close");
        assert_eq!(late.active, SegmentId::from_document(mine));
        assert!(!late.created, "the late close registered nothing");
        assert_eq!(
            db::channel_document_kind(&state.db, &theirs)
                .await
                .expect("query"),
            None
        );
    }

    /// A viewer fills no segment, so closes none, and a successor the caller
    /// does not own is refused.
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn a_close_needs_a_writer_and_its_own_successor(pool: sqlx::PgPool) {
        let state = crate::test_helpers::test_app_state(pool).await;
        let alice = user(&state, "alice@test.dev").await;
        let vera = user(&state, "vera@test.dev").await;
        let space = space(&state, alice, &[(vera, Role::Viewer)]).await;
        let channel = public_channel(&state, alice, space).await;
        grant(&state, channel.meta.document_id(), vera, Role::Viewer).await;

        let veras = document(&state, vera).await;
        assert_eq!(
            close(&state, vera, &channel, veras).await.err(),
            Some(StatusCode::FORBIDDEN)
        );
        assert_eq!(
            close(&state, alice, &channel, veras).await.err(),
            Some(StatusCode::FORBIDDEN),
            "the successor belongs to another member"
        );
        assert_eq!(
            close(&state, alice, &channel, channel.segment.document_id())
                .await
                .err(),
            Some(StatusCode::FORBIDDEN),
            "a segment is not its own successor"
        );
    }

    // --- Finalizing a closed segment ---

    async fn lease(
        state: &Arc<AppState>,
        who: UserId,
        channel: &Channel,
    ) -> Result<FinalizeLeaseJson, StatusCode> {
        take_finalize_lease(
            State(state.clone()),
            session(who),
            Path((channel.id.to_string(), channel.segment.to_string())),
        )
        .await
        .map(|axum::Json(answer)| answer)
        .map_err(|e| e.into_response().status())
    }

    async fn finalize(
        state: &Arc<AppState>,
        who: UserId,
        channel: &Channel,
        lease_id: ambre_core::LeaseId,
        snapshot_id: ambre_core::SnapshotId,
    ) -> StatusCode {
        status(
            finalize_segment(
                State(state.clone()),
                session(who),
                Path((channel.id.to_string(), channel.segment.to_string())),
                Json(FinalizeSegmentRequest {
                    lease_id,
                    snapshot_id,
                }),
            )
            .await,
        )
    }

    /// Run `sql` against the row of the channel's first segment.
    async fn on_segment(state: &AppState, channel: &Channel, sql: &str) {
        sqlx::query(sql)
            .bind(uuid_of_doc(channel.segment.document_id()))
            .execute(&state.db)
            .await
            .expect("update");
    }

    /// A delta of `document` at `seq`, stored `age` ago.
    async fn delta(state: &AppState, document: DocumentId, seq: i64, age: &str) {
        let snapshot = ambre_core::SnapshotId::new();
        db::insert_update(&state.db, &document, &[1; 32], &snapshot, seq, seq, "k")
            .await
            .expect("insert");
        sqlx::query(
            "UPDATE updates SET created_at = now() - $2::interval \
             WHERE document_id = $1 AND seq = $3",
        )
        .bind(uuid_of_doc(document))
        .bind(age)
        .bind(seq)
        .execute(&state.db)
        .await
        .expect("age");
    }

    /// A snapshot of `document` covering every delta up to `covered`.
    async fn snapshot(
        state: &AppState,
        document: DocumentId,
        covered: i64,
    ) -> ambre_core::SnapshotId {
        let clocks = serde_json::json!({});
        db::create_snapshot(
            &state.db, &document, &[1; 32], None, None, &clocks, "s", covered,
        )
        .await
        .expect("snapshot")
    }

    /// A public channel of `alice` and `bob`, its first segment closed, and the
    /// relay holding `seq` deltas for it.
    async fn closed_channel(state: &Arc<AppState>, seq: u64) -> (UserId, UserId, Channel) {
        use rustis::commands::StringCommands;
        let alice = user(state, "alice@test.dev").await;
        let bob = user(state, "bob@test.dev").await;
        let space = space(state, alice, &[(bob, Role::Editor)]).await;
        let channel = public_channel(state, alice, space).await;
        grant(state, channel.meta.document_id(), bob, Role::Editor).await;
        let successor = document(state, alice).await;
        close(state, alice, &channel, successor)
            .await
            .expect("close");
        let key = format!("doc:{}:seq", channel.segment.document_id());
        state.redis.set(key, seq).await.expect("seq");
        (alice, bob, channel)
    }

    async fn forget_store(state: &AppState, channel: &Channel) {
        use rustis::commands::GenericCommands;
        let keys = crate::redis_ops::document_state_keys(&channel.segment.document_id());
        let _: usize = state.redis.del(keys).await.expect("del");
    }

    /// `SRV-199`, `SRV-113`, `SRV-114`: no lease before the grace period has
    /// passed since the close and since the latest delta. The first lease
    /// locks the segment at its last delta number, and a second writer waits
    /// out the live lease.
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn a_lease_waits_out_the_grace_then_goes_to_one_writer_at_a_time(pool: sqlx::PgPool) {
        use rustis::commands::GenericCommands;
        let state = crate::test_helpers::test_app_state(pool).await;
        let (alice, bob, channel) = closed_channel(&state, 4).await;
        let segment = channel.segment.document_id();

        let early = lease(&state, alice, &channel).await.expect("lease");
        assert!(early.lease.is_none(), "the segment just closed");
        assert!(early.retry_in > FINALIZE_GRACE.as_millis() as u64 - 60_000);

        on_segment(
            &state,
            &channel,
            "UPDATE channel_documents SET closed_at = now() - interval '2 days' \
             WHERE document_id = $1",
        )
        .await;
        delta(&state, segment, 4, "1 hour").await;
        let quiet = lease(&state, alice, &channel).await.expect("lease");
        assert!(
            quiet.lease.is_none(),
            "a delta came within the grace period"
        );
        assert!(
            !db::segment_locked(&state.db, &segment)
                .await
                .expect("query")
        );

        sqlx::query("UPDATE updates SET created_at = now() - interval '2 days'")
            .execute(&state.db)
            .await
            .expect("age");
        let granted = lease(&state, alice, &channel).await.expect("lease");
        let held = granted.lease.expect("the grace period has passed");
        assert_eq!(held.horizon, 4);
        assert_eq!(held.expires_in, FINALIZE_LEASE.as_millis() as u64);
        assert!(
            db::segment_locked(&state.db, &segment)
                .await
                .expect("query")
        );
        let mirrored: usize = state
            .redis
            .exists(crate::redis_ops::segment_locked_key(&segment))
            .await
            .expect("exists");
        assert_eq!(mirrored, 1, "the store refuses every later delta");

        let second = lease(&state, bob, &channel).await.expect("lease");
        assert!(second.lease.is_none(), "one lease at a time");
        assert!(second.retry_in > 0 && second.retry_in <= FINALIZE_LEASE.as_millis() as u64);
        forget_store(&state, &channel).await;
    }

    /// `SRV-119`: an expired lease lets another writer take the lease, at the
    /// horizon the lock fixed.
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn an_expired_lease_goes_to_another_writer(pool: sqlx::PgPool) {
        let state = crate::test_helpers::test_app_state(pool).await;
        let (alice, bob, channel) = closed_channel(&state, 3).await;
        on_segment(
            &state,
            &channel,
            "UPDATE channel_documents SET closed_at = now() - interval '2 days' \
             WHERE document_id = $1",
        )
        .await;
        let first = lease(&state, alice, &channel).await.expect("lease");
        let first = first.lease.expect("granted");

        on_segment(
            &state,
            &channel,
            "UPDATE channel_documents SET lease_until = now() - interval '1 second' \
             WHERE document_id = $1",
        )
        .await;
        let second = lease(&state, bob, &channel).await.expect("lease");
        let second = second.lease.expect("the first lease expired");
        assert_ne!(second.lease_id, first.lease_id);
        assert_eq!(second.horizon, first.horizon);
        assert_eq!(
            finalize(
                &state,
                alice,
                &channel,
                first.lease_id,
                snapshot(&state, channel.segment.document_id(), 3).await,
            )
            .await,
            StatusCode::CONFLICT,
            "the expired lease commits nothing"
        );
        forget_store(&state, &channel).await;
    }

    /// `SRV-116`, `SRV-117`, `SRV-120`: a commit needs the live lease and a
    /// snapshot of the segment at its horizon. An accepted one records the
    /// time, and touches no delta of the segment or of any other document.
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn a_commit_needs_the_live_lease_and_a_snapshot_of_the_horizon(pool: sqlx::PgPool) {
        let state = crate::test_helpers::test_app_state(pool).await;
        let (alice, bob, channel) = closed_channel(&state, 2).await;
        let segment = channel.segment.document_id();
        let meta = channel.meta.document_id();
        delta(&state, segment, 2, "2 days").await;
        delta(&state, meta, 1, "2 days").await;
        on_segment(
            &state,
            &channel,
            "UPDATE channel_documents SET closed_at = now() - interval '2 days' \
             WHERE document_id = $1",
        )
        .await;
        let held = lease(&state, alice, &channel).await.expect("lease");
        let held = held.lease.expect("granted");
        let short = snapshot(&state, segment, 1).await;
        let elsewhere = snapshot(&state, meta, 2).await;
        let exact = snapshot(&state, segment, 2).await;

        for (who, lease_id, snapshot_id) in [
            (bob, ambre_core::LeaseId::new(), exact),
            (alice, held.lease_id, short),
            (alice, held.lease_id, elsewhere),
        ] {
            assert_eq!(
                finalize(&state, who, &channel, lease_id, snapshot_id).await,
                StatusCode::CONFLICT
            );
        }
        assert_eq!(
            finalize(&state, alice, &channel, held.lease_id, exact).await,
            StatusCode::NO_CONTENT
        );
        assert_eq!(
            finalize(&state, alice, &channel, held.lease_id, exact).await,
            StatusCode::NO_CONTENT,
            "a retry of the accepted commit"
        );
        let (state_word, finalized): (String, bool) = sqlx::query_as(
            "SELECT state, finalized_at IS NOT NULL FROM channel_documents WHERE document_id = $1",
        )
        .bind(uuid_of_doc(segment))
        .fetch_one(&state.db)
        .await
        .expect("row");
        assert_eq!((state_word.as_str(), finalized), ("final", true));
        let deltas: i64 = sqlx::query_scalar("SELECT count(*) FROM updates")
            .fetch_one(&state.db)
            .await
            .expect("count");
        assert_eq!(deltas, 2, "the commit purges no delta");
        let meta_final: Option<bool> = sqlx::query_scalar(
            "SELECT finalized_at IS NOT NULL FROM channel_documents WHERE document_id = $1",
        )
        .bind(uuid_of_doc(meta))
        .fetch_one(&state.db)
        .await
        .expect("row");
        assert_eq!(meta_final, Some(false));
        assert_eq!(
            lease(&state, bob, &channel).await.err(),
            Some(StatusCode::CONFLICT),
            "a final segment takes no lease"
        );
        forget_store(&state, &channel).await;
    }

    /// A report dropped the final snapshot of a segment: its finalization
    /// goes with it. Another writer takes the lease at the same horizon at
    /// once, and commits a new snapshot (the Murmure segments spec).
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn a_dropped_final_snapshot_is_finalized_again(pool: sqlx::PgPool) {
        let state = crate::test_helpers::test_app_state(pool).await;
        let (alice, bob, channel) = closed_channel(&state, 2).await;
        let segment = channel.segment.document_id();
        on_segment(
            &state,
            &channel,
            "UPDATE channel_documents SET closed_at = now() - interval '2 days' \
             WHERE document_id = $1",
        )
        .await;
        let held = lease(&state, alice, &channel).await.expect("lease");
        let held = held.lease.expect("granted");
        let sealed = snapshot(&state, segment, 2).await;
        assert_eq!(
            finalize(&state, alice, &channel, held.lease_id, sealed).await,
            StatusCode::NO_CONTENT
        );

        let other = snapshot(&state, segment, 2).await;
        let ignored = db::drop_pending_snapshot(&state.db, &segment, &other)
            .await
            .expect("drop")
            .expect("pending");
        assert!(
            !ignored.refinalize,
            "another snapshot leaves the segment final"
        );
        assert_eq!(
            lease(&state, bob, &channel).await.err(),
            Some(StatusCode::CONFLICT)
        );

        let dropped = db::drop_pending_snapshot(&state.db, &segment, &sealed)
            .await
            .expect("drop")
            .expect("pending");
        assert!(dropped.refinalize);
        let (word, finalized): (String, bool) = sqlx::query_as(
            "SELECT state, finalized_at IS NOT NULL FROM channel_documents WHERE document_id = $1",
        )
        .bind(uuid_of_doc(segment))
        .fetch_one(&state.db)
        .await
        .expect("row");
        assert_eq!((word.as_str(), finalized), ("final", false), "still locked");

        let again = lease(&state, bob, &channel).await.expect("lease");
        let again = again.lease.expect("no grace period this time");
        assert_eq!(again.horizon, 2);
        let resealed = snapshot(&state, segment, 2).await;
        assert_eq!(
            finalize(&state, bob, &channel, again.lease_id, resealed).await,
            StatusCode::NO_CONTENT
        );
        assert_eq!(
            lease(&state, alice, &channel).await.err(),
            Some(StatusCode::CONFLICT),
            "finalized again, it takes no lease"
        );
        forget_store(&state, &channel).await;
    }

    /// `SRV-118`: the deltas of a final segment stay while only its author
    /// vouched for the final snapshot. Another member's confirmation purges
    /// them all.
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn a_final_segment_keeps_its_deltas_until_another_member_confirms(pool: sqlx::PgPool) {
        let state = crate::test_helpers::test_app_state(pool).await;
        let (alice, _bob, channel) = closed_channel(&state, 2).await;
        let segment = channel.segment.document_id();
        delta(&state, segment, 1, "2 days").await;
        delta(&state, segment, 2, "2 days").await;
        on_segment(
            &state,
            &channel,
            "UPDATE channel_documents SET closed_at = now() - interval '2 days' \
             WHERE document_id = $1",
        )
        .await;
        let held = lease(&state, alice, &channel).await.expect("lease");
        let held = held.lease.expect("granted");
        let exact = snapshot(&state, segment, 2).await;
        let author = [1; 32];
        assert_eq!(
            db::confirm_snapshot(&state.db, &segment, &exact, &author)
                .await
                .expect("confirm"),
            Some(db::SnapshotStatus::Pending),
            "the author vouches for no final snapshot"
        );
        assert_eq!(
            finalize(&state, alice, &channel, held.lease_id, exact).await,
            StatusCode::NO_CONTENT
        );
        let count = async || -> i64 {
            sqlx::query_scalar("SELECT count(*) FROM updates WHERE document_id = $1")
                .bind(uuid_of_doc(segment))
                .fetch_one(&state.db)
                .await
                .expect("count")
        };
        db::purge_superseded_updates(&state.db)
            .await
            .expect("purge");
        assert_eq!(
            count().await,
            2,
            "an unconfirmed final snapshot purges nothing"
        );
        assert_eq!(
            db::confirm_snapshot(&state.db, &segment, &exact, &[2; 32])
                .await
                .expect("confirm"),
            Some(db::SnapshotStatus::Confirmed)
        );
        db::purge_superseded_updates(&state.db)
            .await
            .expect("purge");
        assert_eq!(
            count().await,
            0,
            "a confirmed final snapshot purges every delta"
        );
        forget_store(&state, &channel).await;
    }

    /// A viewer finalizes nothing, and the active segment takes no lease.
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn a_lease_needs_a_writer_and_a_closing_segment(pool: sqlx::PgPool) {
        let state = crate::test_helpers::test_app_state(pool).await;
        let alice = user(&state, "alice@test.dev").await;
        let vera = user(&state, "vera@test.dev").await;
        let space = space(&state, alice, &[(vera, Role::Viewer)]).await;
        let channel = public_channel(&state, alice, space).await;
        grant(&state, channel.meta.document_id(), vera, Role::Viewer).await;
        assert_eq!(
            lease(&state, vera, &channel).await.err(),
            Some(StatusCode::FORBIDDEN)
        );
        assert_eq!(
            lease(&state, alice, &channel).await.err(),
            Some(StatusCode::CONFLICT),
            "the active segment is not closing"
        );
    }
}
