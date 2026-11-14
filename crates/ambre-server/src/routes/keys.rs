use axum::extract::State;
use axum::response::IntoResponse;
use std::sync::Arc;

use ambre_core::{ExchangePublicKey, FixedBytes, MasterKeySalt, PublicKey};

use crate::auth;
use crate::db;
use crate::error::{AppError, Resource};
use crate::extract::Json;
use crate::state::AppState;
use ambre_core::{Sec, TimeSpan};

/// Request body for first-time master key upload.
/// All binary fields are hex- or base64url-encoded. A key that is not 32
/// bytes of hexadecimal refuses the body when it is read, before any row is
/// written (`SRV-232`, `SRV-242`).
#[derive(serde::Deserialize)]
pub struct StoreMasterKeysRequest {
    pub master_key_enc: ambre_core::Encoded<ambre_core::WrappedMasterKey>,
    pub master_key_salt: FixedBytes<MasterKeySalt>,
    pub signing_pubkey: FixedBytes<PublicKey>,
    pub exchange_pubkey: FixedBytes<ExchangePublicKey>,
}

pub async fn store_master_keys(
    State(state): State<Arc<AppState>>,
    auth: auth::AccountSession,
    body: Json<StoreMasterKeysRequest>,
) -> Result<impl IntoResponse, AppError> {
    let req = body.into_inner();
    let s = auth;
    let mk_enc = req.master_key_enc;
    let mk_salt = req.master_key_salt;
    let (spk, epk) = (req.signing_pubkey, req.exchange_pubkey);

    // Write the master key material and the pubkeys in ONE statement. They
    // describe a single identity. A partial write (new master_key_enc but stale
    // pubkeys, or vice versa) would leave the account unable to unwrap its own
    // material. A single UPDATE to the one row is atomic without a transaction
    // round-trip.
    //
    // First-time only: the `master_key_enc IS NULL` guard makes this endpoint the
    // one-shot setup path it is documented to be. Without it, a second call would
    // overwrite `signing_pubkey`/`exchange_pubkey` unconditionally. Every
    // `doc_key_enc` already wrapped to the previous exchange key on shared
    // documents would then become undecryptable. The caller would silently lose
    // access to its own shared documents. A legitimate passphrase change keeps the
    // seed and goes through `rotate_master_keys`, which touches only enc + salt. A
    // re-store here is always a mistake, and is rejected.
    let uid = uuid::Uuid::from_bytes(*s.user_id.uuid().as_bytes());
    let result = sqlx::query(
        "UPDATE users SET master_key_enc = $2, master_key_salt = $3, signing_pubkey = $4, exchange_pubkey = $5 WHERE id = $1 AND master_key_enc IS NULL"
    )
    .bind(uid)
    .bind(mk_enc.as_bytes())
    .bind(mk_salt.as_slice())
    .bind(spk.as_slice())
    .bind(epk.as_slice())
    .execute(&state.db)
    .await?;
    if result.rows_affected() == 0 {
        // The row already holds master key material (or does not exist): this is a
        // re-store, not the first-time upload. Reject rather than clobber the
        // identity keys.
        return Err(AppError::Conflict {
            detail: "master key already set; use rotate to change the passphrase".into(),
        });
    }

    // The signing pubkey just changed, so the claims carried by the current
    // access token (and any refresh) are now stale. Issue a fresh pair bound to
    // the new key and return it; the client swaps its in-memory tokens.
    let new_signing_pubkey = spk;
    let tokens = auth::issue_session(&state, s.user_id, Some(new_signing_pubkey)).await?;
    tracing::debug!(user_id = %s.user_id, "master key stored");

    Ok(axum::Json(KeysStoredResponse {
        status: ambre_core::WriteStatus::Stored,
        access_token: tokens.access_token,
        refresh_token: tokens.refresh_token,
        access_expires_in: tokens.access_expires_in,
    }))
}

/// The answer of a first key store: the fresh token pair, bound to the new
/// signing key.
#[derive(serde::Serialize)]
pub struct KeysStoredResponse {
    pub status: ambre_core::WriteStatus,
    pub access_token: ambre_protocol::ws::Credential<ambre_protocol::ws::AccessToken>,
    pub refresh_token: ambre_protocol::ws::Credential<ambre_protocol::ws::RefreshToken>,
    /// Access-token lifetime in seconds.
    pub access_expires_in: TimeSpan<Sec>,
}

/// Response containing the full key material for the authenticated user.
/// The client decrypts `master_key_enc` with the User Key derived from the passphrase.
/// Each is `null` until the key setup: no empty value stands for an absent one.
#[derive(serde::Serialize)]
pub struct MasterKeysResponse {
    pub master_key_enc: Option<ambre_core::Encoded<ambre_core::WrappedMasterKey>>,
    pub master_key_salt: Option<FixedBytes<MasterKeySalt>>,
    pub signing_pubkey: Option<FixedBytes<PublicKey>>,
    pub exchange_pubkey: Option<FixedBytes<ExchangePublicKey>>,
}

pub async fn get_master_keys(
    State(state): State<Arc<AppState>>,
    auth: auth::AccountSession,
) -> Result<impl IntoResponse, AppError> {
    let s = auth;
    let u = db::get_user(&state.db, &s.user_id)
        .await?
        .ok_or(AppError::NotFound {
            resource: Resource::User,
        })?;
    Ok(axum::Json(MasterKeysResponse {
        master_key_enc: u.master_key_enc,
        master_key_salt: u.master_key_salt,
        signing_pubkey: u.signing_pubkey,
        exchange_pubkey: u.exchange_pubkey,
    }))
}

/// What a key rotation answers, the master key's or a document's (`SRV-180`).
#[derive(serde::Serialize)]
pub struct RotatedResponse {
    pub status: ambre_core::RotationStatus,
}

impl RotatedResponse {
    /// The answer of a rotation that landed.
    pub(crate) fn rotated() -> axum::Json<Self> {
        axum::Json(Self {
            status: ambre_core::RotationStatus::Rotated,
        })
    }
}

/// Request to rotate the master key after a passphrase change.
/// Only `master_key_enc` and `master_key_salt` change; the seed is unchanged.
#[derive(serde::Deserialize)]
pub struct RotateMasterKeysRequest {
    pub master_key_enc: ambre_core::Encoded<ambre_core::WrappedMasterKey>,
    pub master_key_salt: FixedBytes<MasterKeySalt>,
}

pub async fn rotate_master_keys(
    State(state): State<Arc<AppState>>,
    auth: auth::AuthSession,
    body: Json<RotateMasterKeysRequest>,
) -> Result<impl IntoResponse, AppError> {
    let req = body.into_inner();
    let s = auth;
    let mk_enc = req.master_key_enc;
    let mk_salt = req.master_key_salt;
    db::update_master_key(&state.db, &s.user_id, &mk_enc, &mk_salt).await?;
    tracing::debug!(user_id = %s.user_id, "master key rotated");
    Ok(RotatedResponse::rotated())
}
