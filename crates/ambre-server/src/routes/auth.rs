use axum::extract::{Path, State};
use axum::response::IntoResponse;
use std::sync::Arc;

use crate::auth;
use crate::db;
use crate::error::AppError;
use crate::extract::Json;
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
        dev_mode: state.config.dev_mode == crate::state::DevMode::On,
    }))
}

/// Request body for the dev-only `POST /api/auth/dev/login`.
///
/// The pubkeys are hex-encoded and derived deterministically client-side from
/// the username; the server stores them so it can verify signatures.
#[derive(serde::Deserialize)]
pub struct DevLoginRequest {
    pub username: String,
    pub signing_pub_key: ambre_core::SigningKeyHex,
    pub exchange_pub_key: ambre_core::ExchangeKeyHex,
}

/// Response for `POST /api/auth/dev/login`.
#[derive(serde::Serialize)]
pub struct DevLoginResponse {
    #[serde(with = "ambre_core::serde_id::user_id")]
    pub user_id: ambre_core::UserId,
    pub display_name: String,
    /// Access JWT, kept in client memory and sent as `Authorization: Bearer`.
    pub access_token: ambre_protocol::ws::AccessToken,
    /// Opaque refresh token, exchanged at `/auth/refresh` for a new access JWT.
    pub refresh_token: ambre_protocol::ws::RefreshToken,
    /// Access-token lifetime in seconds, so the client can schedule a refresh.
    pub access_expires_in: ambre_core::Seconds,
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
    if state.config.dev_mode == crate::state::DevMode::Off {
        return Err(AppError::Unauthorized);
    }
    // The name is the account's display name, so it obeys the same bound
    // (`SRV-234`).
    let username = ambre_core::DisplayName::new(&req.username)
        .map_err(|e| AppError::BadRequest(format!("invalid username: {e}")))?;

    // A key of the wrong length is refused before any row is written
    // (`SRV-232`).
    let signing_pubkey = hex::decode(req.signing_pub_key.as_str())
        .map_err(|e| e.to_string())
        .and_then(|bytes| ambre_core::PublicKey::from_slice(&bytes).map_err(|e| e.to_string()))
        .map_err(|e| AppError::BadRequest(format!("invalid signing_pub_key: {e}")))?;
    let exchange_pubkey = hex::decode(req.exchange_pub_key.as_str())
        .map_err(|e| e.to_string())
        .and_then(|bytes| {
            ambre_core::ExchangePublicKey::from_slice(&bytes).map_err(|e| e.to_string())
        })
        .map_err(|e| AppError::BadRequest(format!("invalid exchange_pub_key: {e}")))?;

    let user = db::dev_upsert_user(&state.db, &username, &signing_pubkey, &exchange_pubkey).await?;

    let user_id = user.id;
    // The upsert above writes both keys, so a dev account always holds one.
    let signing_pubkey = account_session_key(&user)
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

/// The signing key a sign-in mints its session under.
///
/// A provider account is created before its keys exist, and its row holds no
/// key until the one-shot key write. Until then the session carries no key,
/// and only the setup routes accept it (`SRV-027`).
///
/// A stored key that does not parse never reaches here: the row read refuses
/// it, rather than a fallback key every such account would share.
fn account_session_key(user: &db::UserRow) -> Option<ambre_core::PublicKey> {
    user.signing_pubkey.clone()
}

/// Request body for `POST /api/auth/refresh`: the opaque refresh token.
#[derive(serde::Deserialize)]
pub struct RefreshRequest {
    pub refresh_token: ambre_protocol::ws::RefreshToken,
}

/// Response for `POST /api/auth/refresh`: a freshly rotated token pair.
#[derive(serde::Serialize)]
pub struct RefreshResponse {
    pub access_token: ambre_protocol::ws::AccessToken,
    pub refresh_token: ambre_protocol::ws::RefreshToken,
    pub access_expires_in: ambre_core::Seconds,
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
    redis_ops::oidc_state_set(&state.redis, &login_resp.state, &auth_state).await?;

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

    let stored_state = redis_ops::oidc_state_take(&state.redis, &req.state).await?;

    let claims =
        oidc::exchange_code(provider_state, req.code.expose().to_owned(), &stored_state).await?;

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
    let email = ambre_core::Email(claims.email().map(|e| e.to_string()).unwrap_or_default());
    let display_name = provider_display_name(
        claims.name().and_then(|l| l.get(None)).map(|s| s.as_str()),
        claims
            .given_name()
            .and_then(|l| l.get(None))
            .map(|s| s.as_str()),
        &email,
        &oidc_subject,
    )
    .ok_or_else(|| AppError::BadRequest("OIDC provider returned a blank subject".into()))?;

    let user =
        db::upsert_user_from_oidc(&state.db, &provider, &oidc_subject, &email, &display_name)
            .await?;

    let user_id = user.id;
    let signing_pubkey = account_session_key(&user);

    let tokens = auth::issue_session(&state, user_id, signing_pubkey).await?;
    tracing::debug!(user_id = %user_id, provider = %provider, "login");

    Ok(axum::Json(CallbackResponse {
        user_id,
        email,
        display_name: display_name.into_string(),
        is_new_user: user.master_key_enc.is_none(),
        access_token: tokens.access_token,
        refresh_token: tokens.refresh_token,
        access_expires_in: tokens.access_expires_in,
    }))
}

/// The display name a provider's claims give. It is the first of the full
/// name, the given name, the address and the subject that holds more than
/// blanks. A name past the cap is cut to it (`SRV-234`). `None` only for a
/// blank subject, which a provider never sends.
fn provider_display_name(
    name: Option<&str>,
    given_name: Option<&str>,
    email: &ambre_core::Email,
    subject: &str,
) -> Option<ambre_core::DisplayName> {
    [name, given_name, Some(email.as_str()), Some(subject)]
        .into_iter()
        .flatten()
        .find_map(ambre_core::DisplayName::cut_to_fit)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `SRV-234`: a provider's name is trimmed and cut to the cap, and a blank
    /// one falls through to the next claim.
    #[test]
    fn a_provider_name_is_trimmed_and_cut_to_the_cap() {
        let email = ambre_core::Email::from("ada@example.com");
        let long = "a".repeat(ambre_core::NAME_MAX_CHARS + 40);
        let cut = provider_display_name(Some(&long), None, &email, "sub").unwrap();
        assert_eq!(cut.as_str().chars().count(), ambre_core::NAME_MAX_CHARS);
        let padded = provider_display_name(Some("  Ada  "), None, &email, "sub").unwrap();
        assert_eq!(padded.as_str(), "Ada");
        let blank = provider_display_name(Some("  "), Some(" Ada "), &email, "sub").unwrap();
        assert_eq!(blank.as_str(), "Ada");
        let bare = ambre_core::Email::from("");
        let subject = provider_display_name(None, None, &bare, "sub-1").unwrap();
        assert_eq!(subject.as_str(), "sub-1");
        assert_eq!(provider_display_name(None, None, &bare, " "), None);
    }

    #[test]
    fn a_stored_key_that_does_not_parse_mints_no_session() {
        assert!(matches!(
            crate::db::decode_key(vec![3u8; 31]),
            Err(sqlx::Error::Decode(_))
        ));
        assert!(crate::db::decode_key(Vec::new()).is_err());
        assert_eq!(
            crate::db::decode_key(vec![3u8; 32]).unwrap(),
            ambre_core::PublicKey::from_array([3u8; 32])
        );
    }

    fn provider_row(signing_pubkey: Option<ambre_core::PublicKey>) -> db::UserRow {
        db::UserRow {
            id: ambre_core::UserId::new(),
            oidc_provider: Some("google".into()),
            oidc_subject: Some("subject".into()),
            email: "a@example.com".into(),
            email_verified: true,
            display_name: "A".into(),
            master_key_enc: signing_pubkey
                .as_ref()
                .map(|_| ambre_core::WrappedMasterKey::new(vec![1u8; 56])),
            master_key_salt: None,
            exchange_pubkey: signing_pubkey
                .as_ref()
                .map(|_| ambre_core::ExchangePublicKey::from_array([4u8; 32])),
            signing_pubkey,
        }
    }

    /// `SRV-027`: a provider account that has not registered its keys gets a
    /// session with no key, never one under a placeholder.
    #[test]
    fn a_provider_account_without_keys_gets_no_signing_key() {
        let fresh = provider_row(None);
        assert_eq!(account_session_key(&fresh), None);

        let set_up = provider_row(Some(ambre_core::PublicKey::from_array([3u8; 32])));
        assert_eq!(
            account_session_key(&set_up),
            Some(ambre_core::PublicKey::from_array([3u8; 32]))
        );
    }
}
