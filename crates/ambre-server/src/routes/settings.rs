// ambre-server/src/routes/settings.rs
// The pointer from an account to its settings document
// (`ambre-spec-user-settings.md`).

use axum::extract::State;
use axum::response::IntoResponse;
use std::sync::Arc;

use ambre_core::id::{Document, Id};

use crate::auth;
use crate::db;
use crate::error::{AppError, Resource};
use crate::extract::Json;
use crate::state::AppState;

/// The document that holds the settings of the caller.
#[derive(serde::Serialize)]
pub struct SettingsDocumentResponse {
    #[serde(with = "ambre_core::serde_id::text")]
    pub document_id: Id<Document>,
}

/// `GET /settings`: the settings document of the caller, or not-found when
/// the account has none yet (`SETTINGS-007`).
pub async fn get_settings_document(
    State(state): State<Arc<AppState>>,
    auth: auth::AuthSession,
) -> Result<impl IntoResponse, AppError> {
    let document = db::settings_document(&state.db, &auth.user_id)
        .await?
        .ok_or(AppError::NotFound {
            resource: Resource::Document,
        })?;
    Ok(axum::Json(SettingsDocumentResponse {
        document_id: document,
    }))
}

/// The settings document a first unlock offers: its id, the self-wrap of
/// its key and the first entry of its log, signed by the user.
#[derive(serde::Deserialize)]
pub struct ClaimSettingsRequest {
    #[serde(with = "ambre_core::serde_id::text")]
    pub document_id: Id<Document>,
    pub doc_key_enc: ambre_core::Encoded<ambre_core::WrappedDocKey>,
    pub entry: ambre_core::Encoded<ambre_core::EncodedLogEntry>,
}

/// `POST /settings`: creates the settings document of the caller and points
/// the account at it, in one transaction. An account that already points at
/// one answers that one, and nothing is stored (`SETTINGS-002`,
/// `SETTINGS-008`, `SETTINGS-009`).
pub async fn claim_settings_document(
    State(state): State<Arc<AppState>>,
    auth: auth::AuthSession,
    body: Json<ClaimSettingsRequest>,
) -> Result<impl IntoResponse, AppError> {
    let req = body.into_inner();
    if let Some(existing) = db::settings_document(&state.db, &auth.user_id).await? {
        return Ok(axum::Json(SettingsDocumentResponse {
            document_id: existing,
        }));
    }
    let expected = super::share::Expected {
        act: ambre_protocol::MembershipAct::Create,
        member: auth.user_id,
        role: Some(ambre_core::Role::Owner),
        key: None,
    };
    let entry =
        super::share::checked_entry(&state, &auth, req.document_id, Some(&req.entry), expected)
            .await?
            .ok_or_else(|| {
                AppError::BadRequest("a settings document needs its first entry".into())
            })?;
    let claim = db::claim_settings_document(
        &state.db,
        &auth.user_id,
        &req.document_id,
        &req.doc_key_enc,
        &entry,
    )
    .await?;
    tracing::debug!(
        created = matches!(claim, db::SettingsClaim::Created(_)),
        "settings claim"
    );
    Ok(axum::Json(SettingsDocumentResponse {
        document_id: claim.document(),
    }))
}
