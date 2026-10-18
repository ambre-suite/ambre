mod answers;
mod auth;
mod channels;
mod documents;
mod keys;
mod media;
mod orgs;
mod read_cursors;
mod share;
mod sync;

#[cfg(test)]
mod tests;

pub use answers::*;
pub use auth::*;
pub use channels::*;
pub use documents::*;
pub use keys::*;
pub use media::*;
pub use orgs::*;
pub use read_cursors::*;
pub use share::*;
pub use sync::*;

/// Every `/api` route, behind the rate limiter. The caller nests it under `/api`,
/// so a handler and the limiter both see the path with that prefix stripped.
///
/// The two provider routes carry the provider as a path segment, which is what
/// their handlers extract (`SRV-172`). A static route such as `/auth/dev/login`
/// takes priority over the segment.
pub fn api_router(
    state: std::sync::Arc<crate::state::AppState>,
) -> axum::Router<std::sync::Arc<crate::state::AppState>> {
    use std::sync::Arc;
    axum::Router::new()
        .route("/auth/{provider}/login", axum::routing::get(oidc_login))
        .route(
            "/auth/{provider}/callback",
            axum::routing::post(oidc_callback),
        )
        .route("/auth/setup", axum::routing::post(setup_account))
        .route("/auth/dev/login", axum::routing::post(dev_login))
        .route("/auth/config", axum::routing::get(auth_config))
        .route("/auth/refresh", axum::routing::post(refresh))
        .route("/auth/me", axum::routing::get(me))
        .route("/auth/logout", axum::routing::post(logout))
        .route(
            "/keys/master",
            axum::routing::put(store_master_keys).get(get_master_keys),
        )
        .route(
            "/keys/master/rotate",
            axum::routing::put(rotate_master_keys),
        )
        .route("/users/search", axum::routing::get(search_users))
        .route("/users/{id}/pubkeys", axum::routing::get(get_pubkeys))
        .route(
            "/documents",
            axum::routing::get(list_documents).post(create_document),
        )
        .route(
            "/documents/{id}",
            axum::routing::get(get_document)
                .delete(delete_document)
                .patch(rename_document),
        )
        .route(
            "/documents/{id}/snapshot",
            axum::routing::get(get_snapshot_metadata).post(create_snapshot),
        )
        .route(
            "/documents/{id}/snapshot/upload-url",
            axum::routing::post(snapshot_upload_url),
        )
        .route(
            "/documents/{id}/snapshot/download-url",
            axum::routing::post(snapshot_download_url),
        )
        .route(
            "/documents/{id}/snapshots/{snapshot_id}/confirm",
            axum::routing::post(confirm_snapshot),
        )
        .route(
            "/documents/{id}/snapshots/{snapshot_id}/report",
            axum::routing::post(report_snapshot),
        )
        .route("/documents/{id}/deltas", axum::routing::get(get_deltas))
        .route(
            "/documents/{id}/share",
            axum::routing::post(invite_to_document),
        )
        .route(
            "/documents/{id}/share/{uid}",
            axum::routing::delete(revoke_from_document),
        )
        .route(
            "/documents/{id}/revoke",
            axum::routing::post(revoke_and_rotate),
        )
        .route(
            "/documents/{id}/members",
            axum::routing::get(list_document_members),
        )
        .route(
            "/documents/{id}/members/{uid}",
            axum::routing::patch(update_document_member_role),
        )
        .route(
            "/documents/{id}/rotate-key",
            axum::routing::post(rotate_document_key),
        )
        .route(
            "/documents/{id}/key-links",
            axum::routing::get(list_key_links),
        )
        .route(
            "/documents/{id}/media/upload-url",
            axum::routing::post(media_upload_url),
        )
        .route(
            "/documents/{id}/media/download-url",
            axum::routing::post(media_download_url),
        )
        .route(
            "/documents/{id}/media",
            axum::routing::post(media_confirm_upload),
        )
        .route(
            "/channels",
            axum::routing::post(create_channel).get(list_member_channels),
        )
        .route(
            "/channels/direct",
            axum::routing::post(create_direct_message),
        )
        .route("/channels/{id}", axum::routing::delete(delete_channel))
        .route(
            "/channels/{id}/documents",
            axum::routing::get(list_channel_documents),
        )
        .route(
            "/channels/{id}/segments",
            axum::routing::post(close_segment),
        )
        .route(
            "/channels/{id}/segments/{segment}/lease",
            axum::routing::post(take_finalize_lease),
        )
        .route(
            "/channels/{id}/segments/{segment}/finalize",
            axum::routing::post(finalize_segment),
        )
        .route(
            "/spaces/{id}/channels",
            axum::routing::get(list_space_channels),
        )
        .route(
            "/channels/{id}/join",
            axum::routing::post(join_channel).get(join_channel_head),
        )
        .route(
            "/documents/{id}/read-cursor",
            axum::routing::put(set_read_cursor),
        )
        .route("/activity", axum::routing::get(get_activity))
        .route("/read-cursors/mark-read", axum::routing::post(mark_read))
        .route(
            "/organizations",
            axum::routing::get(list_orgs).post(create_org),
        )
        .route(
            "/organizations/{id}",
            axum::routing::get(get_org)
                .patch(update_org)
                .delete(delete_org),
        )
        .route(
            "/organizations/{id}/members",
            axum::routing::post(add_member).get(list_members),
        )
        .route(
            "/organizations/{id}/members/{uid}",
            axum::routing::delete(remove_member).patch(update_member_role),
        )
        .route(
            "/organizations/{id}/recover/{uid}",
            axum::routing::post(recover_master_key),
        )
        .route("/ws", axum::routing::get(crate::ws::ws_upgrade))
        .layer({
            let state = state.clone();
            axum::middleware::from_fn(
                move |req: axum::extract::Request, next: axum::middleware::Next| {
                    let state = Arc::clone(&state);
                    async move {
                        crate::middleware::rate_limit::rate_limit_inner(state, req, next).await
                    }
                },
            )
        })
}
