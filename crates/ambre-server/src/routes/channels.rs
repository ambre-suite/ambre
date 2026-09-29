// ambre-server/src/routes/channels.rs
// The channel routes (`ambre-spec-server.md section 4`). One today: the
// documents a channel groups, which a client needs before it can open any of
// them. Creating a channel and joining one land with their own Stories.

use axum::extract::{Path, State};
use axum::response::IntoResponse;
use std::sync::Arc;

use crate::auth;
use crate::db;
use crate::error::AppError;
use crate::state::AppState;

/// One document of a channel, as the listing answers it.
#[derive(serde::Serialize)]
pub struct ChannelDocumentJson {
    pub document_id: String,
    pub kind: String,
}

/// `GET /api/channels/{id}/documents`: the documents the channel groups, with
/// their kind, to a member of the channel alone (`SRV-103`).
///
/// An unknown channel and a channel the caller is no member of answer the
/// same 403. So the route tells a stranger nothing of which channels exist.
pub async fn list_channel_documents(
    State(state): State<Arc<AppState>>,
    auth: auth::AuthSession,
    Path(channel_id): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    let channel_id = ambre_core::Uuid::parse_str(&channel_id)
        .map(ambre_core::ChannelId::from_uuid)
        .map_err(|_| AppError::Forbidden)?;
    let documents = db::list_channel_documents(&state.db, &channel_id).await?;
    let meta = documents
        .iter()
        .find(|d| d.kind == "meta")
        .ok_or(AppError::Forbidden)?;
    let meta_id = ambre_core::DocumentId::from_uuid(ambre_core::Uuid::from_bytes(
        *meta.document_id.as_bytes(),
    ));
    db::get_document_member(&state.db, &meta_id, &auth.user_id)
        .await?
        .ok_or(AppError::Forbidden)?;
    let json: Vec<_> = documents
        .into_iter()
        .map(|d| ChannelDocumentJson {
            document_id: d.document_id.to_string(),
            kind: d.kind,
        })
        .collect();
    Ok(axum::Json(serde_json::json!({ "documents": json })))
}
