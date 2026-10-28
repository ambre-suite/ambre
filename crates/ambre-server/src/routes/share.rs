use axum::extract::State;
use axum::response::IntoResponse;
use std::sync::Arc;

use ambre_core::Role;

use crate::auth;
use crate::db;
use crate::error::{AppError, Resource};
use crate::extract::{Json, PathId};
use crate::state::AppState;

/// What a route expects of the membership entry its request carries.
pub(crate) struct Expected {
    pub act: ambre_protocol::MembershipAct,
    /// The member the change adds, removes or changes.
    pub member: ambre_core::UserId,
    /// The role the entry gives, `None` for a removal.
    pub role: Option<Role>,
    /// The key the entry names, when the route knows it. That is the key an
    /// invite wraps, the key a revoke installs, or none for a role change.
    pub key: Option<Option<ambre_core::KeyId>>,
}

/// Decodes and checks the signed membership entry a request carries
/// (`SRV-104`). The entry must record the change the route makes, about
/// the member it names and under the key that member registered. The caller
/// must have signed it with their own key. The relay checks the signature
/// and nothing past it: the digests are the clients' to walk.
pub(crate) async fn checked_entry(
    state: &AppState,
    auth: &auth::AuthSession,
    doc_id: ambre_core::DocumentId,
    entry: Option<&ambre_core::EncodedLogEntry>,
    expected: Expected,
) -> Result<Option<db::LogEntry>, AppError> {
    use ed25519_dalek::{Signature, VerifyingKey};
    let Some(sent) = entry else {
        return Ok(None);
    };
    let entry = ambre_protocol::MembershipEntry::decode(sent)
        .map_err(|e| AppError::BadRequest(format!("entry: {e}")))?;
    let member_key = db::get_user(&state.db, &expected.member)
        .await?
        .and_then(|user| user.signing_pubkey);
    let first = entry.act == ambre_protocol::MembershipAct::Create;
    let records = entry.well_formed()
        && entry.document == doc_id
        && entry.author == auth.user_id
        && entry.member == expected.member
        && entry.act == expected.act
        && expected.role.is_none_or(|role| entry.role == role)
        && expected.key.is_none_or(|key| entry.key == key)
        && first == entry.position.is_first()
        && member_key.as_ref().map(ambre_core::PublicKey::as_slice)
            == Some(entry.member_key.as_slice());
    if !records {
        return Err(AppError::BadRequest(
            "entry: does not record this change".into(),
        ));
    }
    let signed = entry
        .signed_bytes()
        .map_err(|e| AppError::BadRequest(format!("entry: {e}")))?;
    let signature = Signature::from_bytes(&entry.signature.0);
    VerifyingKey::from_bytes(auth.signing_pubkey.as_array())
        .and_then(|key| key.verify_strict(signed.as_bytes(), &signature))
        .map_err(|_| AppError::InvalidSignature)?;
    Ok(Some(db::LogEntry {
        position: entry.position,
        bytes: sent.clone(),
    }))
}

/// Request to add a user to a document. `doc_key_enc` is the DocumentKey
/// encrypted with the invitee's X25519 exchange key.
#[derive(serde::Deserialize)]
pub struct InviteRequest {
    /// The invitee by email, as a document's share dialog names them.
    #[serde(default)]
    pub invitee_email: Option<ambre_core::Email>,
    /// The invitee by identity, as a member of a space is known to the
    /// others. A request names one of the two (`SRV-224`).
    #[serde(default, with = "ambre_core::serde_id::user_id::option")]
    pub invitee_id: Option<ambre_core::UserId>,
    /// A role word, parsed by the handler: an unknown word answers 409, as it
    /// always has, rather than the extractor's 422.
    #[serde(with = "ambre_core::serde_enums::role")]
    pub role: ambre_core::Role,
    pub doc_key_enc: ambre_core::WrappedDocKey,
    /// The key `doc_key_enc` wraps, in hex. A rotation that replaced it
    /// refuses the invitation (`SRV-168`).
    #[serde(with = "ambre_core::serde_id::key_id")]
    pub key_id: ambre_core::KeyId,
    /// The signed entry the invite appends to the log (`SYNC-105`).
    #[serde(default)]
    pub entry: Option<ambre_core::EncodedLogEntry>,
}
#[derive(serde::Serialize)]
pub struct InviteResponse {
    #[serde(with = "ambre_core::serde_id::user_id")]
    pub user_id: ambre_core::UserId,
    pub display_name: ambre_core::DisplayName,
    /// Whether the invitee's email was provider-verified (OIDC) rather than
    /// self-asserted at `/auth/setup` (#3). The document key is wrapped to the
    /// invitee's exchange key here. An unverified recipient is a squat candidate,
    /// so the client should surface the TOFU out-of-band key check before trusting
    /// the wrap. Additive field; older clients ignore it.
    pub email_verified: bool,
}

pub async fn invite_to_document(
    State(state): State<Arc<AppState>>,
    auth: auth::AuthSession,
    PathId(doc_id): PathId<ambre_core::DocumentId>,
    Json(req): Json<InviteRequest>,
) -> Result<impl IntoResponse, AppError> {
    let s = auth;
    let m = db::get_document_member(&state.db, &doc_id, &s.user_id)
        .await?
        .ok_or(AppError::Forbidden)?;
    // The caller must be able to invite (owner/editor) and may only grant a role
    // at or below their own rank (N2). Without this clamp an `editor` could mint an
    // `owner`, who could then revoke/rotate -- a privilege escalation. Only an owner
    // may grant `owner`.
    let caller_rank = m.role.document_rank().ok_or(AppError::Forbidden)?;
    if !m.role.can_write_document() {
        return Err(AppError::Forbidden);
    }
    // The rows of a channel's `meta` grant every document of it, so an
    // invite into a channel goes there and nowhere else (`SRV-190`).
    let mut channel_space = None;
    if let Some(channel_id) = db::channel_of_document(&state.db, &doc_id).await? {
        let channel = db::get_channel(&state.db, &channel_id).await?;
        let meta = channel.as_ref().and_then(|channel| channel.meta_id);
        if meta.map(|meta| meta.document_id()) != Some(doc_id) {
            return Err(AppError::Conflict {
                detail: "a channel takes its invites on its meta document".into(),
            });
        }
        channel_space = channel.and_then(|channel| channel.space_id);
    }
    let invalid_role = || AppError::Conflict {
        detail: "invalid role".into(),
    };
    let role = req.role;
    let granted_rank = role.document_rank().ok_or_else(invalid_role)?;
    if granted_rank > caller_rank {
        return Err(AppError::Forbidden);
    }

    // An account without keys cannot hold a wrap. The key lookup and the search
    // never offer it, and the invite refuses it the same way (`SRV-163`).
    let found = match (&req.invitee_email, req.invitee_id) {
        (Some(email), None) => db::get_user_by_email(&state.db, email).await?,
        (None, Some(user_id)) => db::get_user(&state.db, &user_id).await?,
        _ => {
            return Err(AppError::Conflict {
                detail: "an invite names its invitee by email or by id, once".into(),
            });
        }
    };
    let invitee = found
        .filter(|u| u.exchange_pubkey.is_some())
        .ok_or(AppError::NotFound {
            resource: Resource::User,
        })?;
    let invitee_id = invitee.id;
    // A channel of a space admits the members of that space alone; a direct
    // message lies in no space (`SRV-225`).
    if let Some(space) = channel_space
        && db::member_role(&state.db, &space.document_id(), &invitee_id)
            .await?
            .is_none()
    {
        return Err(AppError::Conflict {
            detail: "the invitee is not a member of the space of the channel".into(),
        });
    }
    // `add_document_member` upserts (ON CONFLICT DO UPDATE), so invite is also
    // the role-change path for an EXISTING member. Clamp the target too: a caller
    // may only touch a member currently at or below their own rank. Without this
    // an `editor` could re-invite an existing `owner` as `editor`. An editor
    // holds the document key, so it could supply a valid wrap. That demotes an
    // owner and overwrites their key entry, despite never being an owner.
    if let Some(existing) = db::get_document_member(&state.db, &doc_id, &invitee_id).await? {
        let target_rank = existing.role.document_rank().ok_or(AppError::Forbidden)?;
        if target_rank > caller_rank {
            return Err(AppError::Forbidden);
        }
    }
    let doc_key_enc = req.doc_key_enc;
    let key_id = req.key_id;
    let expected = Expected {
        act: ambre_protocol::MembershipAct::Invite,
        member: invitee_id,
        role: Some(role),
        // The entry names the key the copy wraps (`SYNC-123`).
        key: Some(Some(key_id)),
    };
    let entry = checked_entry(&state, &s, doc_id, req.entry.as_ref(), expected).await?;
    db::invite_document_member(
        &state.db,
        &doc_id,
        &invitee_id,
        role,
        &doc_key_enc,
        &s.user_id,
        &key_id,
        entry.as_ref(),
    )
    .await?;
    // Log the resolved invitee_id (UUID), never the email passed in the request.
    tracing::info!(document_id = %doc_id, invitee = %invitee_id, role = %role, by = %s.user_id, "member invited");

    // Live push: tell the invitee's connections a document was shared so their
    // list refreshes without a manual reload. Best-effort -- the membership row
    // is the source of truth; a missed push self-heals on the next list fetch.
    if let Some(doc_type) = db::get_document(&state.db, &doc_id)
        .await?
        .map(|d| d.doc_type)
    {
        let msg =
            crate::payload::Payload::server_msg(&ambre_protocol::ws::WsServerMsg::DocumentShared {
                document_id: doc_id,
                doc_type,
            });
        let _ = crate::redis_ops::user_broadcast(&state.redis, &invitee_id, &msg).await;
    }
    // The members who hold the document open read the rows again (`SRV-177`).
    crate::redis_ops::members_changed(&state.redis, &doc_id).await;
    Ok(axum::Json(InviteResponse {
        user_id: invitee_id,
        display_name: invitee.display_name,
        email_verified: invitee.email_verified,
    }))
}

/// The body a removal may carry: the signed entry that records it.
#[derive(serde::Deserialize)]
pub struct RemovalRequest {
    #[serde(default)]
    pub entry: Option<ambre_core::EncodedLogEntry>,
}

pub async fn revoke_from_document(
    State(state): State<Arc<AppState>>,
    auth: auth::AuthSession,
    PathId((doc_id, target)): PathId<(ambre_core::DocumentId, ambre_core::UserId)>,
    body: Option<Json<RemovalRequest>>,
) -> Result<impl IntoResponse, AppError> {
    let s = auth;
    let m = db::get_document_member(&state.db, &doc_id, &s.user_id)
        .await?
        .ok_or(AppError::Forbidden)?;
    // A member may always remove themselves (`SRV-145`). Removing anyone else
    // is the owner's alone (`SRV-015`).
    if target != s.user_id && m.role != Role::Owner {
        return Err(AppError::Forbidden);
    }
    let expected = Expected {
        act: ambre_protocol::MembershipAct::Revoke,
        member: target,
        role: None,
        key: Some(None),
    };
    let text = body.as_ref().and_then(|Json(body)| body.entry.as_ref());
    let entry = checked_entry(&state, &s, doc_id, text, expected).await?;
    // The removal posts no rotation, so the document owes one (`SRV-122`). The
    // mirror is raised first: no delta slips in between the commit and it.
    // A leave of a space raises it on each private channel it leaves too.
    let mut owing = vec![doc_id];
    owing.extend(db::private_channels_of_member(&state.db, &doc_id, &target).await?);
    let sweep_due = crate::deletion::sweep_due(&state);
    let removed = match raise_debt_mirrors(&state, &owing).await {
        Ok(()) => {
            db::remove_document_member(&state.db, &doc_id, &target, entry.as_ref(), sweep_due).await
        }
        Err(e) => Err(e),
    };
    let removed = match removed {
        Ok(removed) => removed,
        Err(e) => {
            restore_debt_mirrors(&state, &owing).await;
            return Err(e);
        }
    };
    tracing::info!(document_id = %doc_id, target_user = %target, by = %s.user_id, removed = ?removed, "member removed");
    // The members who stay read the rows again (`SRV-177`), unless no one stays.
    if !matches!(removed, db::Removed::ChannelDeleted(_)) {
        crate::redis_ops::members_changed(&state.redis, &doc_id).await;
    }
    match removed {
        db::Removed::Owed => {}
        db::Removed::SpaceLeft(leave) => {
            // A channel the mirror was raised on and the leave did not leave
            // owing, as one deleted with it, goes back to the register.
            let unowed: Vec<_> = owing
                .iter()
                .skip(1)
                .filter(|meta| !leave.owed.contains(meta))
                .copied()
                .collect();
            restore_debt_mirrors(&state, &unowed).await;
            announce_space_leave(&state, &leave, &target, sweep_due).await;
        }
        // A public channel owes nothing (`SRV-122`): the mirror goes back to
        // what the register says.
        db::Removed::Kept => {
            restore_debt_mirror(&state, &doc_id).await;
            notify_removed(&state, &doc_id, &target).await;
            return Ok(super::Done::json(ambre_core::WriteStatus::Revoked));
        }
        // No member is left (`SRV-142`), but the other clients of the one who
        // left still hold the channel (`SRV-187`).
        db::Removed::ChannelDeleted(documents) => {
            tracing::info!(document_id = %doc_id, "last member left, channel deleted");
            crate::deletion::empty_marked(&state, &documents, sweep_due).await;
            notify_removed(&state, &doc_id, &target).await;
            return Ok(super::Done::json(ambre_core::WriteStatus::Revoked));
        }
    }

    notify_removed(&state, &doc_id, &target).await;
    // Every member who stays learns of the debt as it learns of a new key. It
    // re-reads the member list, which now says a rotation is owed.
    notify_rotation(&state, &doc_id).await;
    Ok(super::Done::json(ambre_core::WriteStatus::Revoked))
}

/// Request to remove a member and rotate the key in one step
/// (`ambre-spec-key-rotation.md`, `SYNC-126`).
#[derive(serde::Deserialize)]
pub struct RevokeRequest {
    #[serde(with = "ambre_core::serde_id::user_id")]
    pub user_id: ambre_core::UserId,
    pub keys: Vec<RotateKeyEntry>,
    /// Every member the revoke read and keeps, wrapped for or not (`SRV-167`).
    #[serde(with = "ambre_core::serde_id::user_id::vec")]
    pub members: Vec<ambre_core::UserId>,
    /// The link back to the key the revoke replaces (`SYNC-118`).
    pub link: ambre_core::EncodedKeyLink,
    /// The signed entry that records the removal (`SYNC-132`).
    #[serde(default)]
    pub entry: Option<ambre_core::EncodedLogEntry>,
}

/// Removes a member and installs the new key for everyone who stays, in one
/// transaction. Either both land, or nothing changes (`SYNC-041`, `SYNC-126`).
pub async fn revoke_and_rotate(
    State(state): State<Arc<AppState>>,
    auth: auth::AuthSession,
    PathId(doc_id): PathId<ambre_core::DocumentId>,
    Json(req): Json<RevokeRequest>,
) -> Result<impl IntoResponse, AppError> {
    let s = auth;
    let target = req.user_id;
    let m = db::get_document_member(&state.db, &doc_id, &s.user_id)
        .await?
        .ok_or(AppError::Forbidden)?;
    if m.role != Role::Owner {
        return Err(AppError::Forbidden);
    }
    let keys = decode_keys(&req.keys)?;
    let members = req.members;
    let link = decode_link(&req.link, doc_id)?;
    let expected = Expected {
        act: ambre_protocol::MembershipAct::Revoke,
        member: target,
        role: None,
        // The entry names the key the rotation installs.
        key: Some(Some(link.key)),
    };
    let entry = checked_entry(&state, &s, doc_id, req.entry.as_ref(), expected).await?;
    // A removal from a space leaves each of its private channels owing a
    // rotation (`SRV-136`), so their mirrors are raised first.
    let owing = db::private_channels_of_member(&state.db, &doc_id, &target).await?;
    let sweep_due = crate::deletion::sweep_due(&state);
    let revoked = match raise_debt_mirrors(&state, &owing).await {
        Ok(()) => {
            db::revoke_and_rotate(
                &state.db,
                &doc_id,
                &target,
                &s.user_id,
                &keys,
                &members,
                &link,
                entry.as_ref(),
                sweep_due,
            )
            .await
        }
        Err(e) => Err(e),
    };
    let left = match revoked {
        Ok(left) => left,
        Err(e) => {
            restore_debt_mirrors(&state, &owing).await;
            return Err(e);
        }
    };
    tracing::info!(document_id = %doc_id, target_user = %target, by = %s.user_id, members = keys.len(), "member revoked, key rotated");
    // The rotation landed with the removal, so no debt stands (`SRV-124`).
    restore_debt_mirror(&state, &doc_id).await;
    super::channels::lock_retired_segments(&state, &doc_id).await;
    if let Some(leave) = left {
        let unowed: Vec<_> = owing
            .iter()
            .filter(|meta| !leave.owed.contains(meta))
            .copied()
            .collect();
        restore_debt_mirrors(&state, &unowed).await;
        announce_space_leave(&state, &leave, &target, sweep_due).await;
    }

    notify_removed(&state, &doc_id, &target).await;
    notify_rotation(&state, &doc_id).await;
    Ok(super::Done::json(ambre_core::WriteStatus::Revoked))
}

/// Whether `user` is an admin, an owner or an editor, of the space of the
/// public channel whose `meta` is `document_id` (`SRV-146`). The link that
/// admin signs is checked by its role in the space (the Murmure spaces
/// spec).
pub(crate) async fn space_admin_of_public_meta(
    state: &AppState,
    document_id: &ambre_core::DocumentId,
    user: &ambre_core::UserId,
) -> Result<bool, AppError> {
    let Some(channel_id) = db::channel_of_document(&state.db, document_id).await? else {
        return Ok(false);
    };
    let Some(channel) = db::get_channel(&state.db, &channel_id).await? else {
        return Ok(false);
    };
    let is_meta = channel.meta_id.map(|m| m.document_id()) == Some(*document_id);
    let (true, true, Some(space)) = (
        channel.visibility == ambre_core::Visibility::Public,
        is_meta,
        channel.space_id,
    ) else {
        return Ok(false);
    };
    let role = db::member_role(&state.db, &space.document_id(), user).await?;
    Ok(matches!(role, Some(Role::Owner | Role::Editor)))
}

/// Raises the debt mirror of each of `documents`, in order. The first that
/// fails stops the rest.
async fn raise_debt_mirrors(
    state: &AppState,
    documents: &[ambre_core::DocumentId],
) -> Result<(), AppError> {
    for doc_id in documents {
        crate::redis_ops::mirror_rotation_owed(
            &state.redis,
            doc_id,
            crate::redis_ops::RotationDebt::Owed,
        )
        .await?;
    }
    Ok(())
}

/// [`restore_debt_mirror`] for each of `documents`.
async fn restore_debt_mirrors(state: &AppState, documents: &[ambre_core::DocumentId]) {
    for doc_id in documents {
        restore_debt_mirror(state, doc_id).await;
    }
}

/// Tells each channel a leave of its space touched (`SRV-136`). The members
/// who stay re-read the rows, and those of a private channel learn its debt.
/// The member who left drops each channel, a deleted one included
/// (`SRV-187`), whose documents are emptied (`SRV-165`).
async fn announce_space_leave(
    state: &AppState,
    leave: &db::SpaceLeave,
    target: &ambre_core::UserId,
    sweep_due: ambre_core::UnixSeconds,
) {
    for meta in &leave.left {
        crate::redis_ops::members_changed(&state.redis, meta).await;
        notify_removed(state, meta, target).await;
    }
    for meta in &leave.owed {
        notify_rotation(state, meta).await;
    }
    for (meta, documents) in &leave.deleted {
        crate::deletion::empty_marked(state, documents, sweep_due).await;
        notify_removed(state, meta, target).await;
    }
}

/// Notifies the removed user directly on their personal topic so their client
/// closes the document. Targeted (not a doc broadcast): only the removed user
/// learns they were removed, and no remaining member can mistake this for their
/// own removal. Best-effort -- the membership row is the source of truth.
pub(crate) async fn notify_removed(
    state: &AppState,
    doc_id: &ambre_core::DocumentId,
    target: &ambre_core::UserId,
) {
    let msg =
        crate::payload::Payload::server_msg(&ambre_protocol::ws::WsServerMsg::DocumentRevoked {
            document_id: *doc_id,
        });
    let _ = crate::redis_ops::user_broadcast(&state.redis, target, &msg).await;
}

/// Tells every member still on the document to re-read its member list: a new
/// wrapped copy, or a rotation owed. No user id: it is not a removal signal.
async fn notify_rotation(state: &AppState, doc_id: &ambre_core::DocumentId) {
    let msg = crate::payload::Payload::server_msg(
        &ambre_protocol::ws::WsServerMsg::KeyRotationRequired {
            document_id: *doc_id,
        },
    );
    let _ = crate::redis_ops::doc_broadcast(&state.redis, doc_id, &msg).await;
}

/// Rewrites the debt mirror from the register, which is the authority. A
/// failure is logged: a subscribe rewrites the mirror again.
async fn restore_debt_mirror(state: &AppState, doc_id: &ambre_core::DocumentId) {
    let restored = match db::rotation_owed(&state.db, doc_id).await {
        Ok(owed) => {
            let debt = crate::redis_ops::RotationDebt::owed_if(owed);
            crate::redis_ops::mirror_rotation_owed(&state.redis, doc_id, debt).await
        }
        Err(e) => Err(e),
    };
    if let Err(e) = restored {
        tracing::error!(document_id = %doc_id, error = %e, "rotation debt mirror not restored");
    }
}

/// Decodes the wrapped copies of a rotation payload.
fn decode_keys(
    entries: &[RotateKeyEntry],
) -> Result<Vec<(ambre_core::UserId, ambre_core::WrappedDocKey)>, AppError> {
    Ok(entries
        .iter()
        .map(|e| (e.user_id, e.doc_key_enc.clone()))
        .collect())
}

/// Decodes a rotation's link and refuses one the relay must not store: a
/// malformed one, one past the size bound, one of another document.
fn decode_link(
    sent: &ambre_core::EncodedKeyLink,
    doc_id: ambre_core::DocumentId,
) -> Result<ambre_protocol::KeyLink, AppError> {
    let link = ambre_protocol::KeyLink::decode(sent)
        .map_err(|e| AppError::BadRequest(format!("link: {e}")))?;
    link.validate()
        .map_err(|e| AppError::BadRequest(format!("link: {e}")))?;
    if link.document_id != doc_id {
        return Err(AppError::BadRequest("link: another document's".into()));
    }
    Ok(link)
}

/// Every link of a document, oldest first, for a member to walk back from
/// the key it holds (`SYNC-117`, `SYNC-119`).
pub async fn list_key_links(
    State(state): State<Arc<AppState>>,
    auth: auth::AuthSession,
    PathId(doc_id): PathId<ambre_core::DocumentId>,
) -> Result<impl IntoResponse, AppError> {
    // The admin who re-keys a public channel reads the chain it extends
    // (`SRV-146`).
    if db::get_document_member(&state.db, &doc_id, &auth.user_id)
        .await?
        .is_none()
        && !space_admin_of_public_meta(&state, &doc_id, &auth.user_id).await?
    {
        return Err(AppError::Forbidden);
    }
    let links = db::document_key_links(&state.db, &doc_id).await?;
    Ok(axum::Json(KeyLinksResponse { links }))
}

/// Every link of a document, each in URL-safe base64, oldest first.
#[derive(serde::Serialize)]
pub struct KeyLinksResponse {
    pub links: Vec<ambre_core::EncodedKeyLink>,
}

/// Request to rotate the Document Key after revoking a member.
/// Contains new `doc_key_enc` values for each remaining member.
#[derive(serde::Deserialize)]
pub struct RotateKeyRequest {
    pub keys: Vec<RotateKeyEntry>,
    /// Every member the rotation read, wrapped for or not (`SRV-167`).
    #[serde(with = "ambre_core::serde_id::user_id::vec")]
    pub members: Vec<ambre_core::UserId>,
    /// The link back to the key the rotation replaces (`SYNC-118`).
    pub link: ambre_core::EncodedKeyLink,
}
/// One entry in the rotation payload: a user and their re-encrypted DocumentKey.
#[derive(serde::Deserialize)]
pub struct RotateKeyEntry {
    #[serde(with = "ambre_core::serde_id::user_id")]
    pub user_id: ambre_core::UserId,
    pub doc_key_enc: ambre_core::WrappedDocKey,
}

pub async fn rotate_document_key(
    State(state): State<Arc<AppState>>,
    auth: auth::AuthSession,
    PathId(doc_id): PathId<ambre_core::DocumentId>,
    Json(req): Json<RotateKeyRequest>,
) -> Result<impl IntoResponse, AppError> {
    let s = auth;
    let m = db::get_document_member(&state.db, &doc_id, &s.user_id).await?;
    // Owner-only outside a debt. Rotating the document key rewraps EVERY
    // member's `doc_key_enc`, and makes the caller the wrapper of record. An
    // editor allowed here could omit members from the payload, leaving their
    // key stale after rotation -> lockout. While a rotation is owed, every
    // frame is refused until one lands. So any member with write access may
    // settle it (`SRV-144`), and that risk is the price of not stalling. An
    // admin of a space re-keys a public channel of it, member or not
    // (`SRV-146`).
    let may_rotate = match m.map(|m| m.role) {
        Some(Role::Owner) => true,
        Some(Role::Editor) if db::rotation_owed(&state.db, &doc_id).await? => true,
        _ => space_admin_of_public_meta(&state, &doc_id, &s.user_id).await?,
    };
    if !may_rotate {
        return Err(AppError::Forbidden);
    }

    let keys = decode_keys(&req.keys)?;
    let members = req.members;
    let link = decode_link(&req.link, doc_id)?;
    db::rotate_document_keys(&state.db, &doc_id, &s.user_id, &keys, &members, &link).await?;
    tracing::info!(document_id = %doc_id, by = %s.user_id, members = keys.len(), "document key rotated");
    // The rotation landed, so the debt is cleared (`SRV-124`).
    restore_debt_mirror(&state, &doc_id).await;
    super::channels::lock_retired_segments(&state, &doc_id).await;
    // Tell every REMAINING member to re-fetch its freshly wrapped key. This is
    // a re-key signal, not a revocation.
    notify_rotation(&state, &doc_id).await;
    Ok(super::keys::RotatedResponse::rotated())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn doc_role_rank_ordering() {
        assert!(Role::Owner.document_rank() > Role::Editor.document_rank());
        assert!(Role::Editor.document_rank() > Role::Viewer.document_rank());
        assert_eq!(Role::Admin.document_rank(), None);
        assert_eq!(Role::Member.document_rank(), None);
    }

    /// N2: an editor may grant editor/viewer but never owner.
    #[test]
    fn editor_cannot_grant_owner() {
        let caller = Role::Editor.document_rank().unwrap();
        assert!(Role::Owner.document_rank().unwrap() > caller); // owner grant blocked
        assert!(Role::Editor.document_rank().unwrap() <= caller);
        assert!(Role::Viewer.document_rank().unwrap() <= caller);
    }

    #[test]
    fn owner_can_grant_any_valid_role() {
        let caller = Role::Owner.document_rank().unwrap();
        for role in [Role::Owner, Role::Editor, Role::Viewer] {
            assert!(role.document_rank().unwrap() <= caller);
        }
    }
}
