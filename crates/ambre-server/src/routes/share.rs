use axum::extract::{Json, Path, State};
use axum::response::IntoResponse;
use std::sync::Arc;

use crate::auth;
use crate::db;
use crate::error::AppError;
use crate::state::AppState;

/// Privilege rank of a document role. Higher binds more capability.
/// `owner` (2) > `editor` (1) > `viewer` (0). An unknown role ranks below
/// everything so it can never be granted.
fn doc_role_rank(role: &str) -> Option<u8> {
    match role {
        "viewer" => Some(0),
        "editor" => Some(1),
        "owner" => Some(2),
        _ => None,
    }
}

fn parse_doc_id(s: &str) -> Result<ambre_core::DocumentId, AppError> {
    ambre_core::Uuid::parse_str(s)
        .map(ambre_core::DocumentId::from_uuid)
        .map_err(|_| AppError::NotFound {
            resource: "document",
        })
}
fn parse_user_id(s: &str) -> Result<ambre_core::UserId, AppError> {
    ambre_core::Uuid::parse_str(s)
        .map(ambre_core::UserId::from_uuid)
        .map_err(|_| AppError::NotFound { resource: "user" })
}

/// Maps the stored lowercase `doc_type` string to the typed enum the WS protocol
/// carries. Returns `None` for an unknown value (the share still succeeds; only
/// the live push is skipped).
fn doc_type_from_str(s: &str) -> Option<ambre_core::DocType> {
    match s {
        "prose" => Some(ambre_core::DocType::Prose),
        "trame" => Some(ambre_core::DocType::Trame),
        "toile" => Some(ambre_core::DocType::Toile),
        "murmure" => Some(ambre_core::DocType::Murmure),
        _ => None,
    }
}

/// Request to add a user to a document. `doc_key_enc` is the DocumentKey
/// encrypted with the invitee's X25519 exchange key.
#[derive(serde::Deserialize)]
pub struct InviteRequest {
    pub invitee_email: String,
    pub role: String,
    pub doc_key_enc: String,
    /// The key `doc_key_enc` wraps, in hex. A rotation that replaced it
    /// refuses the invitation (`SRV-168`).
    pub key_id: String,
}
#[derive(serde::Serialize)]
pub struct InviteResponse {
    pub user_id: String,
    pub display_name: String,
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
    Path(document_id): Path<String>,
    Json(req): Json<InviteRequest>,
) -> Result<impl IntoResponse, AppError> {
    use base64::Engine;
    let s = auth;
    let doc_id = parse_doc_id(&document_id)?;
    let m = db::get_document_member(&state.db, &doc_id, &s.user_id)
        .await?
        .ok_or(AppError::Forbidden)?;
    // The caller must be able to invite (owner/editor) and may only grant a role
    // at or below their own rank (N2). Without this clamp an `editor` could mint an
    // `owner`, who could then revoke/rotate -- a privilege escalation. Only an owner
    // may grant `owner`.
    let caller_rank = doc_role_rank(&m.role).ok_or(AppError::Forbidden)?;
    if doc_role_rank("editor").is_none_or(|editor| caller_rank < editor) {
        return Err(AppError::Forbidden);
    }
    let granted_rank = doc_role_rank(&req.role).ok_or(AppError::Conflict {
        detail: "invalid role".into(),
    })?;
    if granted_rank > caller_rank {
        return Err(AppError::Forbidden);
    }

    // An account without keys cannot hold a wrap. The key lookup and the search
    // never offer it, and the invite refuses it the same way (`SRV-163`).
    let invitee = db::get_user_by_email(&state.db, &req.invitee_email)
        .await?
        .filter(|u| u.exchange_pubkey.is_some())
        .ok_or(AppError::NotFound { resource: "user" })?;
    let invitee_id =
        ambre_core::UserId::from_uuid(ambre_core::Uuid::from_bytes(*invitee.id.as_bytes()));
    // `add_document_member` upserts (ON CONFLICT DO UPDATE), so invite is also
    // the role-change path for an EXISTING member. Clamp the target too: a caller
    // may only touch a member currently at or below their own rank. Without this
    // an `editor` could re-invite an existing `owner` as `editor`. An editor
    // holds the document key, so it could supply a valid wrap. That demotes an
    // owner and overwrites their key entry, despite never being an owner.
    if let Some(existing) = db::get_document_member(&state.db, &doc_id, &invitee_id).await? {
        let target_rank = doc_role_rank(&existing.role).ok_or(AppError::Forbidden)?;
        if target_rank > caller_rank {
            return Err(AppError::Forbidden);
        }
    }
    let doc_key_enc = base64::engine::general_purpose::URL_SAFE
        .decode(&req.doc_key_enc)
        .map_err(|e| AppError::Conflict {
            detail: e.to_string(),
        })?;
    let key_id = parse_key_id(&req.key_id)?;
    db::invite_document_member(
        &state.db,
        &doc_id,
        &invitee_id,
        &req.role,
        &doc_key_enc,
        &s.user_id,
        &key_id,
    )
    .await?;
    // Log the resolved invitee_id (UUID), never the email passed in the request.
    tracing::info!(document_id = %doc_id, invitee = %invitee_id, role = %req.role, by = %s.user_id, "member invited");

    // Live push: tell the invitee's connections a document was shared so their
    // list refreshes without a manual reload. Best-effort -- the membership row
    // is the source of truth; a missed push self-heals on the next list fetch.
    if let Some(doc_type) = db::get_document(&state.db, &doc_id)
        .await?
        .and_then(|d| doc_type_from_str(&d.doc_type))
    {
        let msg = ambre_protocol::ws::encode_server_msg(
            &ambre_protocol::ws::WsServerMsg::DocumentShared {
                document_id: doc_id,
                doc_type,
            },
        );
        let _ = crate::redis_ops::user_broadcast(&state.redis, &invitee_id, &msg).await;
    }
    Ok(axum::Json(InviteResponse {
        user_id: invitee_id.to_string(),
        display_name: invitee.display_name,
        email_verified: invitee.email_verified,
    }))
}

pub async fn revoke_from_document(
    State(state): State<Arc<AppState>>,
    auth: auth::AuthSession,
    Path((document_id, user_id)): Path<(String, String)>,
) -> Result<impl IntoResponse, AppError> {
    let s = auth;
    let doc_id = parse_doc_id(&document_id)?;
    let target = parse_user_id(&user_id)?;
    let m = db::get_document_member(&state.db, &doc_id, &s.user_id)
        .await?
        .ok_or(AppError::Forbidden)?;
    // A member may always remove themselves (`SRV-145`). Removing anyone else
    // is the owner's alone (`SRV-015`).
    if target != s.user_id && m.role != "owner" {
        return Err(AppError::Forbidden);
    }
    // The removal posts no rotation, so the document owes one (`SRV-122`). The
    // mirror is raised first: no frame slips in between the commit and it.
    crate::redis_ops::mirror_rotation_owed(&state.redis, &doc_id, true).await?;
    if let Err(e) = db::remove_document_member(&state.db, &doc_id, &target).await {
        restore_debt_mirror(&state, &doc_id).await;
        return Err(e);
    }
    tracing::info!(document_id = %doc_id, target_user = %target, by = %s.user_id, "member removed, rotation owed");

    notify_removed(&state, &doc_id, &target).await;
    // Every member who stays learns of the debt as it learns of a new key. It
    // re-reads the member list, which now says a rotation is owed.
    notify_rotation(&state, &doc_id).await;
    Ok(axum::Json(serde_json::json!({"status":"revoked"})))
}

/// Request to remove a member and rotate the key in one step
/// (`ambre-spec-key-rotation.md`, `SYNC-126`).
#[derive(serde::Deserialize)]
pub struct RevokeRequest {
    pub user_id: String,
    pub keys: Vec<RotateKeyEntry>,
    /// Every member the revoke read and keeps, wrapped for or not (`SRV-167`).
    pub members: Vec<String>,
    /// The link back to the key the revoke replaces (`SYNC-118`).
    pub link: String,
}

/// Removes a member and installs the new key for everyone who stays, in one
/// transaction. Either both land, or nothing changes (`SYNC-041`, `SYNC-126`).
pub async fn revoke_and_rotate(
    State(state): State<Arc<AppState>>,
    auth: auth::AuthSession,
    Path(document_id): Path<String>,
    Json(req): Json<RevokeRequest>,
) -> Result<impl IntoResponse, AppError> {
    let s = auth;
    let doc_id = parse_doc_id(&document_id)?;
    let target = parse_user_id(&req.user_id)?;
    let m = db::get_document_member(&state.db, &doc_id, &s.user_id)
        .await?
        .ok_or(AppError::Forbidden)?;
    if m.role != "owner" {
        return Err(AppError::Forbidden);
    }
    let keys = decode_keys(&req.keys)?;
    let members = decode_members(&req.members)?;
    let link = decode_link(&req.link, doc_id)?;
    db::revoke_and_rotate(
        &state.db, &doc_id, &target, &s.user_id, &keys, &members, &link,
    )
    .await?;
    tracing::info!(document_id = %doc_id, target_user = %target, by = %s.user_id, members = keys.len(), "member revoked, key rotated");
    // The rotation landed with the removal, so no debt stands (`SRV-124`).
    restore_debt_mirror(&state, &doc_id).await;

    notify_removed(&state, &doc_id, &target).await;
    notify_rotation(&state, &doc_id).await;
    Ok(axum::Json(serde_json::json!({"status":"revoked"})))
}

/// Notifies the removed user directly on their personal topic so their client
/// closes the document. Targeted (not a doc broadcast): only the removed user
/// learns they were removed, and no remaining member can mistake this for their
/// own removal. Best-effort -- the membership row is the source of truth.
async fn notify_removed(
    state: &AppState,
    doc_id: &ambre_core::DocumentId,
    target: &ambre_core::UserId,
) {
    let msg =
        ambre_protocol::ws::encode_server_msg(&ambre_protocol::ws::WsServerMsg::DocumentRevoked {
            document_id: *doc_id,
        });
    let _ = crate::redis_ops::user_broadcast(&state.redis, target, &msg).await;
}

/// Tells every member still on the document to re-read its member list: a new
/// wrapped copy, or a rotation owed. No user id: it is not a removal signal.
async fn notify_rotation(state: &AppState, doc_id: &ambre_core::DocumentId) {
    let msg = ambre_protocol::ws::encode_server_msg(
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
        Ok(owed) => crate::redis_ops::mirror_rotation_owed(&state.redis, doc_id, owed).await,
        Err(e) => Err(e),
    };
    if let Err(e) = restored {
        tracing::error!(document_id = %doc_id, error = %e, "rotation debt mirror not restored");
    }
}

/// Decodes the wrapped copies of a rotation payload.
fn decode_keys(entries: &[RotateKeyEntry]) -> Result<Vec<(ambre_core::UserId, Vec<u8>)>, AppError> {
    use base64::Engine;
    entries
        .iter()
        .map(|e| {
            let uid = parse_user_id(&e.user_id)?;
            let enc = base64::engine::general_purpose::URL_SAFE
                .decode(&e.doc_key_enc)
                .map_err(|er| AppError::Conflict {
                    detail: er.to_string(),
                })?;
            Ok((uid, enc))
        })
        .collect()
}

/// Decodes the members a rotation read.
fn decode_members(ids: &[String]) -> Result<Vec<ambre_core::UserId>, AppError> {
    ids.iter().map(|id| parse_user_id(id)).collect()
}

/// Decodes the hex identifier of the key an invitation wraps.
fn parse_key_id(text: &str) -> Result<ambre_core::KeyId, AppError> {
    let bytes: [u8; ambre_core::KEY_ID_LEN] = hex::decode(text)
        .ok()
        .and_then(|b| b.try_into().ok())
        .ok_or_else(|| AppError::BadRequest("key_id: not a key identifier".into()))?;
    Ok(ambre_core::KeyId::from_array(bytes))
}

/// Decodes a rotation's link and refuses one the relay must not store: a
/// malformed one, one past the size bound, one of another document.
fn decode_link(
    text: &str,
    doc_id: ambre_core::DocumentId,
) -> Result<ambre_protocol::KeyLink, AppError> {
    use base64::Engine;
    let bytes = base64::engine::general_purpose::URL_SAFE
        .decode(text)
        .map_err(|e| AppError::BadRequest(format!("link: {e}")))?;
    let link = ambre_protocol::KeyLink::decode(&bytes)
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
    Path(document_id): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    use base64::Engine;
    let doc_id = parse_doc_id(&document_id)?;
    db::get_document_member(&state.db, &doc_id, &auth.user_id)
        .await?
        .ok_or(AppError::Forbidden)?;
    let links: Vec<String> = db::document_key_links(&state.db, &doc_id)
        .await?
        .iter()
        .map(|bytes| base64::engine::general_purpose::URL_SAFE.encode(bytes))
        .collect();
    Ok(axum::Json(serde_json::json!({ "links": links })))
}

/// Request to rotate the Document Key after revoking a member.
/// Contains new `doc_key_enc` values for each remaining member.
#[derive(serde::Deserialize)]
pub struct RotateKeyRequest {
    pub keys: Vec<RotateKeyEntry>,
    /// Every member the rotation read, wrapped for or not (`SRV-167`).
    pub members: Vec<String>,
    /// The link back to the key the rotation replaces (`SYNC-118`).
    pub link: String,
}
/// One entry in the rotation payload: a user and their re-encrypted DocumentKey.
#[derive(serde::Deserialize)]
pub struct RotateKeyEntry {
    pub user_id: String,
    pub doc_key_enc: String,
}

pub async fn rotate_document_key(
    State(state): State<Arc<AppState>>,
    auth: auth::AuthSession,
    Path(document_id): Path<String>,
    Json(req): Json<RotateKeyRequest>,
) -> Result<impl IntoResponse, AppError> {
    let s = auth;
    let doc_id = parse_doc_id(&document_id)?;
    let m = db::get_document_member(&state.db, &doc_id, &s.user_id)
        .await?
        .ok_or(AppError::Forbidden)?;
    // Owner-only outside a debt. Rotating the document key rewraps EVERY
    // member's `doc_key_enc`, and makes the caller the wrapper of record. An
    // editor allowed here could omit members from the payload, leaving their
    // key stale after rotation -> lockout. While a rotation is owed, every
    // frame is refused until one lands. So any member with write access may
    // settle it (`SRV-144`), and that risk is the price of not stalling.
    let may_rotate = match m.role.as_str() {
        "owner" => true,
        "editor" => db::rotation_owed(&state.db, &doc_id).await?,
        _ => false,
    };
    if !may_rotate {
        return Err(AppError::Forbidden);
    }

    let keys = decode_keys(&req.keys)?;
    let members = decode_members(&req.members)?;
    let link = decode_link(&req.link, doc_id)?;
    db::rotate_document_keys(&state.db, &doc_id, &s.user_id, &keys, &members, &link).await?;
    tracing::info!(document_id = %doc_id, by = %s.user_id, members = keys.len(), "document key rotated");
    // The rotation landed, so the debt is cleared (`SRV-124`).
    restore_debt_mirror(&state, &doc_id).await;
    // Tell every REMAINING member to re-fetch its freshly wrapped key. This is
    // a re-key signal, not a revocation.
    notify_rotation(&state, &doc_id).await;
    Ok(axum::Json(serde_json::json!({"status":"rotated"})))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn doc_role_rank_ordering() {
        assert!(doc_role_rank("owner") > doc_role_rank("editor"));
        assert!(doc_role_rank("editor") > doc_role_rank("viewer"));
        assert_eq!(doc_role_rank("nonsense"), None);
    }

    /// N2: an editor may grant editor/viewer but never owner.
    #[test]
    fn editor_cannot_grant_owner() {
        let caller = doc_role_rank("editor").unwrap();
        assert!(doc_role_rank("owner").unwrap() > caller); // owner grant blocked
        assert!(doc_role_rank("editor").unwrap() <= caller);
        assert!(doc_role_rank("viewer").unwrap() <= caller);
    }

    #[test]
    fn owner_can_grant_any_valid_role() {
        let caller = doc_role_rank("owner").unwrap();
        for role in ["owner", "editor", "viewer"] {
            assert!(doc_role_rank(role).unwrap() <= caller);
        }
    }
}
