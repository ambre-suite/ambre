use axum::extract::{Json, Path, State};
use axum::response::IntoResponse;
use axum_extra::extract::cookie::CookieJar;
use std::sync::Arc;

use crate::auth;
use crate::db;
use crate::error::AppError;
use crate::oidc::{self, CallbackRequest, CallbackResponse};
use crate::redis_ops;
use crate::state::AppState;

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
    pub csrf_token: String,
    /// Opaque session id. Dev-only: the client carries it in the WS URL query
    /// (`/api/ws?session_id=...`) because the browser does not attach the
    /// session cookie to a worker-initiated WebSocket handshake under COEP.
    pub session_id: String,
}

/// `POST /auth/dev/login` (dev only)
///
/// Authenticates as `username`, creating the user on the fly. Bypasses OIDC and
/// the passphrase/master-key dance: the client derives its keys deterministically
/// from the username and submits the pubkeys here. Establishes a Redis session
/// plus the session + CSRF cookies, exactly like `oidc_callback`. Returns 401
/// unless `Config::dev_mode` is set, so a production server always rejects it.
pub async fn dev_login(
    State(state): State<Arc<AppState>>,
    jar: CookieJar,
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

    let session_id = uuid::Uuid::new_v4().to_string();
    let sd = redis_ops::SessionData {
        user_id,
        signing_pubkey,
        session_id: session_id.clone(),
    };
    redis_ops::session_create(&state.redis, &session_id, &sd, state.config.session_ttl_secs)
        .await?;
    tracing::debug!(user_id = %user_id, provider = "dev", "login");

    let mut cookie = auth::create_session_cookie(&session_id, state.config.session_ttl_secs);
    let csrf_token = uuid::Uuid::new_v4().to_string();
    let mut csrf_cookie =
        crate::middleware::csrf::create_csrf_cookie(&csrf_token, state.config.session_ttl_secs);
    // Dev runs over plain http://localhost, where browsers (notably Firefox) drop
    // `Secure` cookies and `SameSite=Strict` can block the cross-context worker
    // fetch. Relax both: dev_login is gated by dev_mode anyway.
    use axum_extra::extract::cookie::SameSite;
    cookie.set_secure(false);
    cookie.set_same_site(SameSite::Lax);
    csrf_cookie.set_secure(false);
    csrf_cookie.set_same_site(SameSite::Lax);
    Ok((
        jar.add(cookie).add(csrf_cookie),
        axum::Json(DevLoginResponse {
            user_id: user_id.to_string(),
            display_name: user.display_name,
            csrf_token,
            session_id,
        }),
    ))
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

    let _ = redis_ops::oidc_state_set(
        &state.redis,
        &login_resp.state,
        &auth_state.pkce_verifier_secret,
        &auth_state.nonce.secret().clone(),
    )
    .await;

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
    jar: CookieJar,
    Path(provider): Path<String>,
    Json(req): Json<CallbackRequest>,
) -> Result<impl IntoResponse, AppError> {
    let provider_state = state
        .oidc_clients
        .get(&provider)
        .ok_or(AppError::UnknownProvider(provider.clone()))?;

    let (pkce_verifier, nonce_str) =
        redis_ops::oidc_state_take(&state.redis, &req.state).await?;

    use openidconnect::Nonce;
    let stored_state = oidc::OidcAuthState {
        pkce_verifier_secret: pkce_verifier,
        nonce: Nonce::new(nonce_str),
    };

    let claims = oidc::exchange_code(provider_state, req.code, &stored_state).await?;

    let oidc_subject = claims.subject().to_string();
    let email = claims
        .email()
        .map(|e| e.to_string())
        .unwrap_or_default();
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

    let user = db::upsert_user_from_oidc(
        &state.db,
        &provider,
        &oidc_subject,
        &email,
        &display_name,
    )
    .await?;

    let user_id = ambre_core::UserId::from_uuid(ambre_core::Uuid::from_bytes(*user.id.as_bytes()));
    let signing_pubkey = ambre_core::PublicKey::from_slice(&user.signing_pubkey)
        .unwrap_or(ambre_core::PublicKey::from_array([0u8; 32]));

    let session_id = uuid::Uuid::new_v4().to_string();
    let sd = redis_ops::SessionData {
        user_id,
        signing_pubkey,
        session_id: session_id.clone(),
    };
    redis_ops::session_create(&state.redis, &session_id, &sd, state.config.session_ttl_secs)
        .await?;
    tracing::debug!(user_id = %user_id, provider = %provider, "login");

    let cookie = auth::create_session_cookie(&session_id, state.config.session_ttl_secs);
    let csrf_token = uuid::Uuid::new_v4().to_string();
    let csrf_cookie = crate::middleware::csrf::create_csrf_cookie(&csrf_token, state.config.session_ttl_secs);
    Ok((
        jar.add(cookie).add(csrf_cookie),
        axum::Json(CallbackResponse {
            user_id: user_id.to_string(),
            email,
            display_name,
            is_new_user: user.master_key_enc.is_none(),
        }),
    ))
}
