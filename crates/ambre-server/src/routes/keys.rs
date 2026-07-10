use axum::extract::{Json, State};
use axum::response::IntoResponse;
use std::sync::Arc;

use crate::auth;
use crate::db;
use crate::error::AppError;
use crate::state::AppState;

/// Request body for first-time master key upload.
/// All binary fields are hex- or base64url-encoded.
#[derive(serde::Deserialize)]
pub struct StoreMasterKeysRequest {
    pub master_key_enc: String,
    pub master_key_salt: String,
    pub signing_pubkey: String,
    pub exchange_pubkey: String,
}

pub async fn store_master_keys(
    State(state): State<Arc<AppState>>,
    auth: auth::AuthSession,
    Json(req): Json<StoreMasterKeysRequest>,
) -> Result<impl IntoResponse, AppError> {
    use base64::Engine;
    let s = auth;
    let mk_enc = base64::engine::general_purpose::URL_SAFE
        .decode(&req.master_key_enc)
        .map_err(|e| AppError::Conflict {
            detail: e.to_string(),
        })?;
    let mk_salt = base64::engine::general_purpose::URL_SAFE
        .decode(&req.master_key_salt)
        .map_err(|e| AppError::Conflict {
            detail: e.to_string(),
        })?;
    let spk = hex::decode(&req.signing_pubkey).map_err(|e| AppError::Conflict {
        detail: e.to_string(),
    })?;
    let epk = hex::decode(&req.exchange_pubkey).map_err(|e| AppError::Conflict {
        detail: e.to_string(),
    })?;

    // Write the master key material and the pubkeys in ONE statement: they
    // describe a single identity, so a partial write (new master_key_enc but
    // stale pubkeys, or vice versa) would leave the account unable to unwrap its
    // own material. A single UPDATE to the one row is atomic without a
    // transaction round-trip.
    let uid = uuid::Uuid::from_bytes(*s.user_id.as_uuid().as_bytes());
    sqlx::query(
        "UPDATE users SET master_key_enc = $2, master_key_salt = $3, signing_pubkey = $4, exchange_pubkey = $5, updated_at = now() WHERE id = $1"
    )
    .bind(uid)
    .bind(&mk_enc)
    .bind(&mk_salt)
    .bind(&spk)
    .bind(&epk)
    .execute(&state.db)
    .await?;

    // The signing pubkey just changed, so the claims carried by the current
    // access token (and any refresh) are now stale. Issue a fresh pair bound to
    // the new key and return it; the client swaps its in-memory tokens.
    let new_signing_pubkey =
        ambre_core::PublicKey::from_slice(&spk).map_err(|e| AppError::Conflict {
            detail: e.to_string(),
        })?;
    let tokens = auth::issue_session(&state, s.user_id, new_signing_pubkey).await?;
    tracing::info!(user_id = %s.user_id, "master key stored");

    Ok(axum::Json(serde_json::json!({
        "status": "stored",
        "access_token": tokens.access_token,
        "refresh_token": tokens.refresh_token,
        "access_expires_in": tokens.access_expires_in,
    })))
}

/// Response containing the full key material for the authenticated user.
/// The client decrypts `master_key_enc` with the User Key derived from the passphrase.
#[derive(serde::Serialize)]
pub struct MasterKeysResponse {
    pub master_key_enc: String,
    pub master_key_salt: String,
    pub signing_pubkey: String,
    pub exchange_pubkey: String,
}

pub async fn get_master_keys(
    State(state): State<Arc<AppState>>,
    auth: auth::AuthSession,
) -> Result<impl IntoResponse, AppError> {
    use base64::Engine;
    let s = auth;
    let u = db::get_user(&state.db, &s.user_id)
        .await?
        .ok_or(AppError::NotFound { resource: "user" })?;
    Ok(axum::Json(MasterKeysResponse {
        master_key_enc: u
            .master_key_enc
            .as_ref()
            .map(|v| base64::engine::general_purpose::URL_SAFE.encode(v))
            .unwrap_or_default(),
        master_key_salt: u
            .master_key_salt
            .as_ref()
            .map(|v| base64::engine::general_purpose::URL_SAFE.encode(v))
            .unwrap_or_default(),
        signing_pubkey: hex::encode(&u.signing_pubkey),
        exchange_pubkey: hex::encode(&u.exchange_pubkey),
    }))
}

/// Request to rotate the master key after a passphrase change.
/// Only `master_key_enc` and `master_key_salt` change; the seed is unchanged.
#[derive(serde::Deserialize)]
pub struct RotateMasterKeysRequest {
    pub master_key_enc: String,
    pub master_key_salt: String,
}

pub async fn rotate_master_keys(
    State(state): State<Arc<AppState>>,
    auth: auth::AuthSession,
    Json(req): Json<RotateMasterKeysRequest>,
) -> Result<impl IntoResponse, AppError> {
    use base64::Engine;
    let s = auth;
    let mk_enc = base64::engine::general_purpose::URL_SAFE
        .decode(&req.master_key_enc)
        .map_err(|e| AppError::Conflict {
            detail: e.to_string(),
        })?;
    let mk_salt = base64::engine::general_purpose::URL_SAFE
        .decode(&req.master_key_salt)
        .map_err(|e| AppError::Conflict {
            detail: e.to_string(),
        })?;
    db::update_master_key(&state.db, &s.user_id, &mk_enc, &mk_salt).await?;
    tracing::info!(user_id = %s.user_id, "master key rotated");
    Ok(axum::Json(serde_json::json!({"status":"rotated"})))
}
