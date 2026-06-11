use axum::extract::{Json, Path, State};
use axum::response::IntoResponse;
use std::sync::Arc;

use crate::auth;
use crate::db;
use crate::error::AppError;
use crate::state::AppState;

fn parse_doc_id(s: &str) -> Result<ambre_core::DocumentId, AppError> { ambre_core::Uuid::parse_str(s).map(ambre_core::DocumentId::from_uuid).map_err(|_| AppError::NotFound{resource:"document"}) }
fn parse_user_id(s: &str) -> Result<ambre_core::UserId, AppError> { ambre_core::Uuid::parse_str(s).map(ambre_core::UserId::from_uuid).map_err(|_| AppError::NotFound{resource:"user"}) }

/// Request to add a user to a document. `doc_key_enc` is the DocumentKey
/// encrypted with the invitee's X25519 exchange key.
#[derive(serde::Deserialize)]
pub struct InviteRequest { pub invitee_email: String, pub role: String, pub doc_key_enc: String }
#[derive(serde::Serialize)]
pub struct InviteResponse { pub user_id: String, pub display_name: String }

pub async fn invite_to_document(
    State(state): State<Arc<AppState>>,
    auth: auth::AuthSession,
    Path(document_id): Path<String>,
    Json(req): Json<InviteRequest>,
) -> Result<impl IntoResponse, AppError> {
    use base64::Engine;
    let s = auth;
    let doc_id = parse_doc_id(&document_id)?;
    let m = db::get_document_member(&state.db, &doc_id, &s.user_id).await?.ok_or(AppError::Forbidden)?;
    if !["owner","editor"].contains(&m.role.as_str()) { return Err(AppError::Forbidden); }
    if !["owner","editor","viewer"].contains(&req.role.as_str()) { return Err(AppError::Conflict{detail:"invalid role".into()}); }

    let invitee = db::get_user_by_email(&state.db, &req.invitee_email).await?.ok_or(AppError::NotFound{resource:"user"})?;
    let invitee_id = ambre_core::UserId::from_uuid(ambre_core::Uuid::from_bytes(*invitee.id.as_bytes()));
    let doc_key_enc = base64::engine::general_purpose::URL_SAFE.decode(&req.doc_key_enc).map_err(|e| AppError::Conflict{detail:e.to_string()})?;
    db::add_document_member(&state.db, &doc_id, &invitee_id, &req.role, &doc_key_enc, &s.user_id).await?;
    // Log the resolved invitee_id (UUID), never the email passed in the request.
    tracing::info!(document_id = %doc_id, invitee = %invitee_id, role = %req.role, by = %s.user_id, "member invited");
    Ok(axum::Json(InviteResponse { user_id: invitee_id.to_string(), display_name: invitee.display_name }))
}

pub async fn revoke_from_document(
    State(state): State<Arc<AppState>>,
    auth: auth::AuthSession,
    Path((document_id, user_id)): Path<(String, String)>,
) -> Result<impl IntoResponse, AppError> {
    let s = auth;
    let doc_id = parse_doc_id(&document_id)?;
    let target = parse_user_id(&user_id)?;
    let m = db::get_document_member(&state.db, &doc_id, &s.user_id).await?.ok_or(AppError::Forbidden)?;
    if m.role != "owner" { return Err(AppError::Forbidden); }
    db::remove_document_member(&state.db, &doc_id, &target).await?;
    tracing::info!(document_id = %doc_id, target_user = %target, by = %s.user_id, "member revoked");
    Ok(axum::Json(serde_json::json!({"status":"revoked"})))
}

/// Request to rotate the Document Key after revoking a member.
/// Contains new `doc_key_enc` values for each remaining member.
#[derive(serde::Deserialize)]
pub struct RotateKeyRequest { pub keys: Vec<RotateKeyEntry> }
/// One entry in the rotation payload: a user and their re-encrypted DocumentKey.
#[derive(serde::Deserialize)]
pub struct RotateKeyEntry { pub user_id: String, pub doc_key_enc: String }

pub async fn rotate_document_key(
    State(state): State<Arc<AppState>>,
    auth: auth::AuthSession,
    Path(document_id): Path<String>,
    Json(req): Json<RotateKeyRequest>,
) -> Result<impl IntoResponse, AppError> {
    use base64::Engine;
    let s = auth;
    let doc_id = parse_doc_id(&document_id)?;
    let m = db::get_document_member(&state.db, &doc_id, &s.user_id).await?.ok_or(AppError::Forbidden)?;
    if !["owner","editor"].contains(&m.role.as_str()) { return Err(AppError::Forbidden); }

    let mut keys = Vec::with_capacity(req.keys.len());
    for e in &req.keys {
        let uid = parse_user_id(&e.user_id)?;
        let enc = base64::engine::general_purpose::URL_SAFE.decode(&e.doc_key_enc).map_err(|er| AppError::Conflict{detail:er.to_string()})?;
        keys.push((uid, enc));
    }
    db::rotate_document_keys(&state.db, &doc_id, &keys).await?;
    tracing::info!(document_id = %doc_id, by = %s.user_id, members = keys.len(), "document key rotated");
    let msg = ambre_protocol::ws::encode_server_msg(&ambre_protocol::ws::WsServerMsg::KeyRotationRequired { document_id: doc_id, revoked_user_id: s.user_id });
    let _ = crate::redis_ops::doc_broadcast(&state.redis, &doc_id, &msg).await;
    Ok(axum::Json(serde_json::json!({"status":"rotated"})))
}
