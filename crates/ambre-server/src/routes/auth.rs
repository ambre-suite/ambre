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
/// before any session exists: a dev-login field when `dev_mode` is set, the
/// OIDC button otherwise. Exposes only this one non-sensitive flag.
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
    pub user_id: String,
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
    let signing_pubkey = ambre_core::PublicKey::from_slice(&user.signing_pubkey)
        .unwrap_or(ambre_core::PublicKey::from_array([0u8; 32]));

    let tokens = auth::issue_session(&state, user_id, signing_pubkey).await?;
    tracing::debug!(user_id = %user_id, provider = "dev", "login");

    Ok(axum::Json(DevLoginResponse {
        user_id: user_id.to_string(),
        display_name: user.display_name,
        access_token: tokens.access_token,
        refresh_token: tokens.refresh_token,
        access_expires_in: tokens.access_expires_in,
    }))
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
/// Atomically claims the refresh token in Redis, rotating it so that a stolen
/// token replayed after its first legitimate use is rejected, and returns a new
/// access + refresh pair. A revoked, expired, or already-claimed token yields 401.
///
/// The claim has to be atomic: reading the record and then deleting it would let
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
/// stores the transient auth state in Redis (TTL 10 min), and returns the
/// URL + state so the client can redirect the browser to the OIDC provider.
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
    // `SessionExpired` on callback -- a confusing failure after credentials were
    // already handed over. Surface the failure now instead.
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
/// exchanges the code for tokens, verifies the ID token, upserts the user,
/// creates a Redis session, and sets the session cookie.
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
    // Reject an unverified email: it is the resolution key for document sharing
    // (`get_user_by_email`), so binding an unverified address to an account would
    // let an attacker squat a victim's email and receive documents shared to it.
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
    let signing_pubkey = ambre_core::PublicKey::from_slice(&user.signing_pubkey)
        .unwrap_or(ambre_core::PublicKey::from_array([0u8; 32]));

    let tokens = auth::issue_session(&state, user_id, signing_pubkey).await?;
    tracing::debug!(user_id = %user_id, provider = %provider, "login");

    Ok(axum::Json(CallbackResponse {
        user_id: user_id.to_string(),
        email,
        display_name,
        is_new_user: user.master_key_enc.is_none(),
        access_token: tokens.access_token,
        refresh_token: tokens.refresh_token,
        access_expires_in: tokens.access_expires_in,
    }))
}
