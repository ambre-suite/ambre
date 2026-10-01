use axum::extract::{Json, Path, State};
use axum::response::IntoResponse;
use std::sync::Arc;

use crate::auth;
use crate::db;
use crate::error::AppError;
use crate::oidc::{self, CallbackRequest, CallbackResponse};
use crate::redis_ops;
use crate::state::AppState;

/// Response for the unauthenticated `GET /api/auth/config`.
///
/// Lets the SPA decide which sign-in affordance to show on the landing page
/// before any session exists. That is a dev-login field when `dev_mode` is
/// set, the OIDC button otherwise. Exposes only this one non-sensitive
/// flag.
#[derive(serde::Serialize)]
pub struct AuthConfigResponse {
    pub dev_mode: bool,
}

/// `GET /auth/config` (unauthenticated)
///
/// Public bootstrap flag for the SPA's login UI. Reveals nothing beyond whether
/// dev login is enabled on this server.
pub async fn auth_config(
    State(state): State<Arc<AppState>>,
) -> Result<impl IntoResponse, AppError> {
    Ok(axum::Json(AuthConfigResponse {
        dev_mode: state.config.dev_mode,
    }))
}

/// Request body for the dev-only `POST /api/auth/dev/login`.
///
/// The pubkeys are hex-encoded and derived deterministically client-side from
/// the username; the server stores them so it can verify signatures.
#[derive(serde::Deserialize)]
pub struct DevLoginRequest {
    pub username: String,
    pub signing_pub_key: String,
    pub exchange_pub_key: String,
}

/// Response for `POST /api/auth/dev/login`.
#[derive(serde::Serialize)]
pub struct DevLoginResponse {
    #[serde(with = "ambre_core::serde_id::user_id")]
    pub user_id: ambre_core::UserId,
    pub display_name: String,
    /// Access JWT, kept in client memory and sent as `Authorization: Bearer`.
    pub access_token: String,
    /// Opaque refresh token, exchanged at `/auth/refresh` for a new access JWT.
    pub refresh_token: String,
    /// Access-token lifetime in seconds, so the client can schedule a refresh.
    pub access_expires_in: u64,
}

/// `POST /auth/dev/login` (dev only)
///
/// Authenticates as `username`, creating the user on the fly. Bypasses OIDC and
/// the passphrase/master-key dance: the client derives its keys deterministically
/// from the username and submits the pubkeys here. Mints an access JWT plus a
/// revocable refresh token, exactly like `oidc_callback`. Returns 401 unless
/// `Config::dev_mode` is set, so a production server always rejects it.
pub async fn dev_login(
    State(state): State<Arc<AppState>>,
    Json(req): Json<DevLoginRequest>,
) -> Result<impl IntoResponse, AppError> {
    if !state.config.dev_mode {
        return Err(AppError::Unauthorized);
    }
    if req.username.is_empty() {
        return Err(AppError::BadRequest("username must not be empty".into()));
    }

    let signing_pubkey = hex::decode(&req.signing_pub_key)
        .map_err(|e| AppError::BadRequest(format!("invalid signing_pub_key: {e}")))?;
    let exchange_pubkey = hex::decode(&req.exchange_pub_key)
        .map_err(|e| AppError::BadRequest(format!("invalid exchange_pub_key: {e}")))?;

    let user =
        db::dev_upsert_user(&state.db, &req.username, &signing_pubkey, &exchange_pubkey).await?;

    let user_id = ambre_core::UserId::from_uuid(ambre_core::Uuid::from_bytes(*user.id.as_bytes()));
    // The upsert above writes both keys, so a dev account always holds one.
    let signing_pubkey = account_session_key(&user)?
        .ok_or_else(|| AppError::Internal("dev account stored no signing key".into()))?;

    let tokens = auth::issue_session(&state, user_id, Some(signing_pubkey)).await?;
    tracing::debug!(user_id = %user_id, provider = "dev", "login");

    Ok(axum::Json(DevLoginResponse {
        user_id,
        display_name: user.display_name,
        access_token: tokens.access_token,
        refresh_token: tokens.refresh_token,
        access_expires_in: tokens.access_expires_in,
    }))
}

/// The signing key a session is minted under, read from the stored row.
///
/// A stored key that does not parse is refused rather than replaced. A fallback
/// key would mint a session every account in that state shares (`SRV-027`).
fn session_signing_key(stored: &[u8]) -> Result<ambre_core::PublicKey, AppError> {
    ambre_core::PublicKey::from_slice(stored)
        .map_err(|e| AppError::Internal(format!("stored signing key does not parse: {e}")))
}

/// The signing key a sign-in mints its session under.
///
/// A provider account is created before its keys exist, and its row holds no
/// key until the one-shot key write. Until then the session carries no key,
/// and only the setup routes accept it (`SRV-027`).
fn account_session_key(user: &db::UserRow) -> Result<Option<ambre_core::PublicKey>, AppError> {
    user.signing_pubkey
        .as_deref()
        .map(session_signing_key)
        .transpose()
}

/// Request body for `POST /api/auth/refresh`: the opaque refresh token.
#[derive(serde::Deserialize)]
pub struct RefreshRequest {
    pub refresh_token: String,
}

/// Response for `POST /api/auth/refresh`: a freshly rotated token pair.
#[derive(serde::Serialize)]
pub struct RefreshResponse {
    pub access_token: String,
    pub refresh_token: String,
    pub access_expires_in: u64,
}

/// `POST /auth/refresh` (unauthenticated; the refresh token is the credential)
///
/// Atomically claims the refresh token in Redis, and returns a new access +
/// refresh pair. The rotation means a stolen token replayed after its first
/// legitimate use is rejected. A revoked, expired, or already-claimed token yields
/// 401.
///
/// The claim has to be atomic. Reading the record and then deleting it would let
/// two concurrent requests bearing the same token both mint a session.
pub async fn refresh(
    State(state): State<Arc<AppState>>,
    Json(req): Json<RefreshRequest>,
) -> Result<impl IntoResponse, AppError> {
    let data = redis_ops::refresh_consume(&state.redis, &req.refresh_token)
        .await?
        .ok_or(AppError::Unauthorized)?;
    let tokens = auth::issue_session(&state, data.user_id, data.signing_pubkey).await?;
    Ok(axum::Json(RefreshResponse {
        access_token: tokens.access_token,
        refresh_token: tokens.refresh_token,
        access_expires_in: tokens.access_expires_in,
    }))
}

/// `GET /auth/{provider}/login`
///
/// Generates an OIDC authorization URL (with PKCE, CSRF state, and nonce),
/// and stores the transient auth state in Redis (TTL 10 min). Returns the
/// URL + state, so the client can redirect the browser to the OIDC
/// provider.
pub async fn oidc_login(
    State(state): State<Arc<AppState>>,
    Path(provider): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    let oidc_state = state
        .oidc_clients
        .get(&provider)
        .ok_or(AppError::UnknownProvider(provider.clone()))?;

    let (login_resp, auth_state) = oidc::login_url(oidc_state).await?;

    // Persisting the transient auth state must succeed: the callback validates
    // the returned CSRF `state` against this Redis entry. If the write is
    // dropped, the client authenticates at the provider and only then hits a
    // `SessionExpired` on callback. That is a confusing failure after
    // credentials were already handed over. Surface the failure now instead.
    redis_ops::oidc_state_set(
        &state.redis,
        &login_resp.state,
        &auth_state.pkce_verifier_secret,
        &auth_state.nonce.secret().clone(),
    )
    .await?;

    Ok(axum::Json(login_resp))
}

/// `POST /auth/{provider}/callback`
///
/// Receives the authorization code and CSRF state from the client (obtained
/// after the OIDC provider redirect). Validates the state against Redis,
/// exchanges the code for tokens, and verifies the ID token. Upserts the
/// user, creates a Redis session, and returns the token pair in the response
/// body. No cookie is set (`ambre-spec-sync.md`, `SYNC-060`).
pub async fn oidc_callback(
    State(state): State<Arc<AppState>>,
    Path(provider): Path<String>,
    Json(req): Json<CallbackRequest>,
) -> Result<impl IntoResponse, AppError> {
    let provider_state = state
        .oidc_clients
        .get(&provider)
        .ok_or(AppError::UnknownProvider(provider.clone()))?;

    let (pkce_verifier, nonce_str) = redis_ops::oidc_state_take(&state.redis, &req.state).await?;

    use openidconnect::Nonce;
    let stored_state = oidc::OidcAuthState {
        pkce_verifier_secret: pkce_verifier,
        nonce: Nonce::new(nonce_str),
    };

    let claims = oidc::exchange_code(provider_state, req.code, &stored_state).await?;

    let oidc_subject = claims.subject().to_string();
    // Reject an unverified email. It is the resolution key for document sharing
    // (`get_user_by_email`). Binding an unverified address to an account would
    // let an attacker squat a victim's email, and receive documents shared to it.
    // A provider that omits the claim (`None`) is treated as unverified.
    if claims.email().is_some() && claims.email_verified() != Some(true) {
        return Err(AppError::BadRequest(
            "OIDC provider returned an unverified email".into(),
        ));
    }
    let email = claims.email().map(|e| e.to_string()).unwrap_or_default();
    let display_name = claims
        .name()
        .and_then(|l| l.get(None).map(|s| s.to_string()))
        .filter(|s| !s.is_empty())
        .or_else(|| {
            claims
                .given_name()
                .and_then(|l| l.get(None).map(|s| s.to_string()))
        })
        .unwrap_or_else(|| email.clone());

    let user =
        db::upsert_user_from_oidc(&state.db, &provider, &oidc_subject, &email, &display_name)
            .await?;

    let user_id = ambre_core::UserId::from_uuid(ambre_core::Uuid::from_bytes(*user.id.as_bytes()));
    let signing_pubkey = account_session_key(&user)?;

    let tokens = auth::issue_session(&state, user_id, signing_pubkey).await?;
    tracing::debug!(user_id = %user_id, provider = %provider, "login");

    Ok(axum::Json(CallbackResponse {
        user_id,
        email,
        display_name,
        is_new_user: user.master_key_enc.is_none(),
        access_token: tokens.access_token,
        refresh_token: tokens.refresh_token,
        access_expires_in: tokens.access_expires_in,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stored_key_that_does_not_parse_mints_no_session() {
        assert!(matches!(
            session_signing_key(&[3u8; 31]),
            Err(AppError::Internal(_))
        ));
        assert!(session_signing_key(&[]).is_err());
        assert_eq!(
            session_signing_key(&[3u8; 32]).unwrap(),
            ambre_core::PublicKey::from_array([3u8; 32])
        );
    }

    fn provider_row(signing_pubkey: Option<Vec<u8>>) -> db::UserRow {
        db::UserRow {
            id: uuid::Uuid::new_v4(),
            oidc_provider: Some("google".into()),
            oidc_subject: Some("subject".into()),
            email: "a@example.com".into(),
            email_verified: true,
            display_name: "A".into(),
            master_key_enc: signing_pubkey.as_ref().map(|_| vec![1u8; 56]),
            master_key_salt: None,
            exchange_pubkey: signing_pubkey.as_ref().map(|_| vec![4u8; 32]),
            signing_pubkey,
            org_id: None,
        }
    }

    /// `SRV-027`: a provider account that has not registered its keys gets a
    /// session with no key, never one under a placeholder.
    #[test]
    fn a_provider_account_without_keys_gets_no_signing_key() {
        let fresh = provider_row(None);
        assert_eq!(account_session_key(&fresh).unwrap(), None);

        let set_up = provider_row(Some(vec![3u8; 32]));
        assert_eq!(
            account_session_key(&set_up).unwrap(),
            Some(ambre_core::PublicKey::from_array([3u8; 32]))
        );
    }
}
