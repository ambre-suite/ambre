use axum::extract::{Json, State};
use axum::response::IntoResponse;
use axum_extra::extract::cookie::CookieJar;
use std::sync::Arc;

use crate::auth;
use crate::db;
use crate::error::AppError;
use crate::redis_ops;
use crate::state::AppState;

/// Request body for first-time master key upload.
/// All binary fields are hex- or base64url-encoded.
#[derive(serde::Deserialize)]
pub struct StoreMasterKeysRequest { pub master_key_enc: String, pub master_key_salt: String, pub signing_pubkey: String, pub exchange_pubkey: String }

pub async fn store_master_keys(
    State(state): State<Arc<AppState>>,
    jar: CookieJar,
    Json(req): Json<StoreMasterKeysRequest>,
) -> Result<impl IntoResponse, AppError> {
    use base64::Engine;
    let s = auth::authenticate(&state, &jar).await?;
    let mk_enc = base64::engine::general_purpose::URL_SAFE.decode(&req.master_key_enc).map_err(|e| AppError::Conflict{detail:e.to_string()})?;
    let mk_salt = base64::engine::general_purpose::URL_SAFE.decode(&req.master_key_salt).map_err(|e| AppError::Conflict{detail:e.to_string()})?;
    let spk = hex::decode(&req.signing_pubkey).map_err(|e| AppError::Conflict{detail:e.to_string()})?;
    let epk = hex::decode(&req.exchange_pubkey).map_err(|e| AppError::Conflict{detail:e.to_string()})?;

    db::update_master_key(&state.db, &s.user_id, &mk_enc, &mk_salt).await?;
    let uid = uuid::Uuid::from_bytes(*s.user_id.as_uuid().as_bytes());
    sqlx::query("UPDATE users SET signing_pubkey = $2, exchange_pubkey = $3 WHERE id = $1").bind(uid).bind(&spk).bind(&epk).execute(&state.db).await?;

    let new_signing_pubkey = ambre_core::PublicKey::from_slice(&spk).map_err(|e| AppError::Conflict{detail:e.to_string()})?;
    let sd = redis_ops::SessionData { user_id: s.user_id, signing_pubkey: new_signing_pubkey, session_id: s.session_id.clone() };
    redis_ops::session_create(&state.redis, &s.session_id, &sd, state.config.session_ttl_secs).await?;
    tracing::info!(user_id = %s.user_id, "master key stored");

    Ok(axum::Json(serde_json::json!({"status":"stored"})))
}

/// Response containing the full key material for the authenticated user.
/// The client decrypts `master_key_enc` with the User Key derived from the passphrase.
#[derive(serde::Serialize)]
pub struct MasterKeysResponse { pub master_key_enc: String, pub master_key_salt: String, pub signing_pubkey: String, pub exchange_pubkey: String }

pub async fn get_master_keys(
    State(state): State<Arc<AppState>>,
    jar: CookieJar,
) -> Result<impl IntoResponse, AppError> {
    use base64::Engine;
    let s = auth::authenticate(&state, &jar).await?;
    let u = db::get_user(&state.db, &s.user_id).await?.ok_or(AppError::NotFound{resource:"user"})?;
    Ok(axum::Json(MasterKeysResponse {
        master_key_enc: u.master_key_enc.as_ref().map(|v| base64::engine::general_purpose::URL_SAFE.encode(v)).unwrap_or_default(),
        master_key_salt: u.master_key_salt.as_ref().map(|v| base64::engine::general_purpose::URL_SAFE.encode(v)).unwrap_or_default(),
        signing_pubkey: hex::encode(&u.signing_pubkey),
        exchange_pubkey: hex::encode(&u.exchange_pubkey),
    }))
}

/// Request to rotate the master key after a passphrase change.
/// Only `master_key_enc` and `master_key_salt` change; the seed is unchanged.
#[derive(serde::Deserialize)]
pub struct RotateMasterKeysRequest { pub master_key_enc: String, pub master_key_salt: String }

pub async fn rotate_master_keys(
    State(state): State<Arc<AppState>>,
    jar: CookieJar,
    Json(req): Json<RotateMasterKeysRequest>,
) -> Result<impl IntoResponse, AppError> {
    use base64::Engine;
    let s = auth::authenticate(&state, &jar).await?;
    let mk_enc = base64::engine::general_purpose::URL_SAFE.decode(&req.master_key_enc).map_err(|e| AppError::Conflict{detail:e.to_string()})?;
    let mk_salt = base64::engine::general_purpose::URL_SAFE.decode(&req.master_key_salt).map_err(|e| AppError::Conflict{detail:e.to_string()})?;
    db::update_master_key(&state.db, &s.user_id, &mk_enc, &mk_salt).await?;
    tracing::info!(user_id = %s.user_id, "master key rotated");
    Ok(axum::Json(serde_json::json!({"status":"rotated"})))
}
