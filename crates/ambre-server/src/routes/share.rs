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
}
#[derive(serde::Serialize)]
pub struct InviteResponse {
    pub user_id: String,
    pub display_name: String,
    /// Whether the invitee's email was provider-verified (OIDC) rather than
    /// self-asserted at `/auth/setup` (#3). The document key is wrapped to the
    /// invitee's exchange key here; an unverified recipient is a squat candidate,
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
    if caller_rank < doc_role_rank("editor").expect("editor is a known role") {
        return Err(AppError::Forbidden);
    }
    let granted_rank = doc_role_rank(&req.role).ok_or(AppError::Conflict {
        detail: "invalid role".into(),
    })?;
    if granted_rank > caller_rank {
        return Err(AppError::Forbidden);
    }

    let invitee = db::get_user_by_email(&state.db, &req.invitee_email)
        .await?
        .ok_or(AppError::NotFound { resource: "user" })?;
    let invitee_id =
        ambre_core::UserId::from_uuid(ambre_core::Uuid::from_bytes(*invitee.id.as_bytes()));
    // `add_document_member` upserts (ON CONFLICT DO UPDATE), so invite is also
    // the role-change path for an EXISTING member. Clamp the target too: a caller
    // may only touch a member currently at or below their own rank. Without this
    // an `editor` could re-invite an existing `owner` as `editor` and, since an
    // editor holds the document key, supply a valid wrap -- demoting an owner and
    // overwriting their key entry despite never being an owner.
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
    db::add_document_member(
        &state.db,
        &doc_id,
        &invitee_id,
        &req.role,
        &doc_key_enc,
        &s.user_id,
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
    if m.role != "owner" {
        return Err(AppError::Forbidden);
    }
    db::remove_document_member(&state.db, &doc_id, &target).await?;
    tracing::info!(document_id = %doc_id, target_user = %target, by = %s.user_id, "member revoked");

    // Notify the revoked user directly on their personal channel so their client
    // closes the document. Targeted (not a doc broadcast): only the removed user
    // learns they were revoked, and no remaining member can mistake this for their
    // own revocation. Best-effort -- the membership row is the source of truth.
    let msg =
        ambre_protocol::ws::encode_server_msg(&ambre_protocol::ws::WsServerMsg::DocumentRevoked {
            document_id: doc_id,
        });
    let _ = crate::redis_ops::user_broadcast(&state.redis, &target, &msg).await;

    Ok(axum::Json(serde_json::json!({"status":"revoked"})))
}

/// Request to rotate the Document Key after revoking a member.
/// Contains new `doc_key_enc` values for each remaining member.
#[derive(serde::Deserialize)]
pub struct RotateKeyRequest {
    pub keys: Vec<RotateKeyEntry>,
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
    use base64::Engine;
    let s = auth;
    let doc_id = parse_doc_id(&document_id)?;
    let m = db::get_document_member(&state.db, &doc_id, &s.user_id)
        .await?
        .ok_or(AppError::Forbidden)?;
    // Owner-only, symmetric with `revoke_from_document`. Rotating the document key
    // rewraps EVERY member's `doc_key_enc` and makes the caller the wrapper of
    // record, so an editor allowed here could omit members from the payload
    // (leaving their key stale after rotation -> lockout) or re-key the document
    // out from under the owner. Key rotation is membership management; it belongs
    // to the owner exactly like revoke.
    if m.role != "owner" {
        return Err(AppError::Forbidden);
    }

    let mut keys = Vec::with_capacity(req.keys.len());
    for e in &req.keys {
        let uid = parse_user_id(&e.user_id)?;
        let enc = base64::engine::general_purpose::URL_SAFE
            .decode(&e.doc_key_enc)
            .map_err(|er| AppError::Conflict {
                detail: er.to_string(),
            })?;
        keys.push((uid, enc));
    }
    db::rotate_document_keys(&state.db, &doc_id, &s.user_id, &keys).await?;
    tracing::info!(document_id = %doc_id, by = %s.user_id, members = keys.len(), "document key rotated");
    // Tell every REMAINING member to re-fetch its freshly wrapped key. No user id:
    // this is a re-key signal, not a revocation, so the rotator (an owner) never
    // appears as "revoked" and cannot kick itself out of its own document.
    let msg = ambre_protocol::ws::encode_server_msg(
        &ambre_protocol::ws::WsServerMsg::KeyRotationRequired {
            document_id: doc_id,
        },
    );
    let _ = crate::redis_ops::doc_broadcast(&state.redis, &doc_id, &msg).await;
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
