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

    let cookie = auth::create_session_cookie(&session_id, state.config.session_ttl_secs);
    Ok((
        jar.add(cookie),
        axum::Json(CallbackResponse {
            user_id: user_id.to_string(),
            email,
            display_name,
            is_new_user: user.master_key_enc.is_none(),
        }),
    ))
}
