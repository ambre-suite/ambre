// ambre-server/src/routes/calls.rs
// The ask of who is in a call (`ambre-spec-call-media.md section 3`, the roster).

use ambre_core::id::{Call, Channel, Id};
use axum::extract::{Query, State};
use axum::response::IntoResponse;
use std::sync::Arc;

use crate::auth;
use crate::db;
use crate::error::AppError;
use crate::extract::OpaquePathId;
use crate::state::AppState;
use ambre_protocol::signaling::CallRoster;

/// The query of an ask: the channel the call message of the asker names.
#[derive(serde::Deserialize)]
pub struct RosterQuery {
    #[serde(with = "ambre_core::serde_id::text")]
    pub channel: Id<Channel>,
}

/// `GET /api/calls/{call}/roster?channel={channel}`: the signed join of each seat of the call,
/// to a member of the channel, in the call or not (`CALL-070`).
///
/// An asker who is no member of the channel, and a call of another channel, read
/// [`CallRoster::Gone`], as a call the relay holds no state of. So the route tells a stranger
/// nothing of which calls run (`CALL-071`). It reads the state of the call and writes
/// nothing, and it logs no ask (`CALL-072`, `ZK-024`).
pub async fn call_roster(
    State(state): State<Arc<AppState>>,
    auth: auth::AuthSession,
    call: OpaquePathId<Id<Call>>,
    Query(query): Query<RosterQuery>,
) -> Result<impl IntoResponse, AppError> {
    let call = call.into_inner();
    if !db::is_channel_member(&state.db, &query.channel, &auth.user_id).await? {
        return Ok(axum::Json(CallRoster::Gone));
    }
    let roster = crate::redis_ops::call_roster(&state.redis, &call, &query.channel).await?;
    Ok(axum::Json(roster))
}
