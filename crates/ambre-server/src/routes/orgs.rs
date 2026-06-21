use axum::extract::{Json, Path, State};
use axum::response::IntoResponse;
use std::sync::Arc;

use crate::auth;
use crate::db;
use crate::error::AppError;
use crate::state::AppState;

// -- Types ------------------------------------------------------------

/// Request body for creating an organization.
#[derive(serde::Deserialize)]
pub struct CreateOrgRequest {
    pub name: String,
    /// X25519 organization public key (hex encoded).
    pub org_pubkey: String,
    pub shamir_threshold: i16,
    pub shamir_total: i16,
    /// First member (owner) encrypted Shamir share (base64 or hex).
    pub owner_shamir_share_enc: Option<String>,
}
#[derive(serde::Serialize)]
pub struct CreateOrgResponse {
    pub id: String,
    pub name: String,
    pub org_pubkey: String,
    pub shamir_threshold: i16,
    pub shamir_total: i16,
}

fn org_to_response(org: &db::OrganizationRow) -> CreateOrgResponse {
    CreateOrgResponse {
        id: org.id.to_string(),
        name: org.name.clone(),
        org_pubkey: hex::encode(&org.org_pubkey),
        shamir_threshold: org.shamir_threshold,
        shamir_total: org.shamir_total,
    }
}

/// Request body for adding a member.
#[derive(serde::Deserialize)]
pub struct AddMemberRequest {
    pub user_id: String,
    pub role: String,
    /// Encrypted Shamir share for this member (base64 or hex).
    pub shamir_share_enc: Option<String>,
}

/// Request body for updating a member's role.
#[derive(serde::Deserialize)]
pub struct UpdateMemberRoleRequest {
    pub role: String,
}

#[derive(serde::Serialize)]
pub struct OrgMemberResponse {
    pub user_id: String,
    pub email: String,
    pub display_name: String,
    pub role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub has_shamir_share: Option<bool>,
}

fn member_to_response(m: &db::OrgMemberRow) -> OrgMemberResponse {
    OrgMemberResponse {
        user_id: m.user_id.to_string(),
        email: m.email.clone(),
        display_name: m.display_name.clone(),
        role: m.role.clone(),
        has_shamir_share: Some(m.shamir_share_enc.is_some()),
    }
}

/// Request body for updating an organization.
#[derive(serde::Deserialize)]
pub struct UpdateOrgRequest {
    pub name: String,
}

fn parse_org_id(s: &str) -> Result<uuid::Uuid, AppError> {
    uuid::Uuid::parse_str(s).map_err(|_| AppError::NotFound {
        resource: "organization",
    })
}

fn parse_user_id_str(s: &str) -> Result<uuid::Uuid, AppError> {
    uuid::Uuid::parse_str(s).map_err(|_| AppError::NotFound { resource: "user" })
}

async fn require_org_role(
    pool: &sqlx::PgPool,
    org_id: uuid::Uuid,
    user_id: uuid::Uuid,
    allowed_roles: &[&str],
) -> Result<db::OrgMembershipRow, AppError> {
    let m = db::get_org_member(pool, org_id, user_id)
        .await?
        .ok_or(AppError::Forbidden)?;
    if !allowed_roles.contains(&m.role.as_str()) {
        return Err(AppError::Forbidden);
    }
    Ok(m)
}

// -- Handlers ---------------------------------------------------------

/// `POST /organizations`
pub async fn create_org(
    State(state): State<Arc<AppState>>,
    auth: auth::AuthSession,
    Json(req): Json<CreateOrgRequest>,
) -> Result<impl IntoResponse, AppError> {
    let session = auth;
    let owner_uid = ambre_core::Uuid::parse_str(&session.user_id.to_string())
        .map_err(|_| AppError::Unauthorized)?;
    let owner_id = ambre_core::UserId::from_uuid(owner_uid);

    let org_pubkey = hex::decode(&req.org_pubkey).map_err(|e| AppError::Conflict {
        detail: format!("invalid org_pubkey hex: {e}"),
    })?;
    let owner_share_enc = req
        .owner_shamir_share_enc
        .as_deref()
        .map(hex::decode)
        .transpose()
        .map_err(|e| AppError::Conflict {
            detail: format!("invalid share hex: {e}"),
        })?;

    let org = db::create_organization(
        &state.db,
        &req.name,
        &org_pubkey,
        req.shamir_threshold,
        req.shamir_total,
        &owner_id,
        owner_share_enc.as_deref(),
    )
    .await?;

    Ok((
        axum::http::StatusCode::CREATED,
        axum::Json(org_to_response(&org)),
    ))
}

/// `GET /organizations`
pub async fn list_orgs(
    State(state): State<Arc<AppState>>,
    auth: auth::AuthSession,
) -> Result<impl IntoResponse, AppError> {
    let session = auth;
    let uid = ambre_core::Uuid::parse_str(&session.user_id.to_string())
        .map_err(|_| AppError::Unauthorized)?;
    let user_id = ambre_core::UserId::from_uuid(uid);

    let orgs = db::list_user_organizations(&state.db, &user_id).await?;
    let resp: Vec<CreateOrgResponse> = orgs.iter().map(org_to_response).collect();
    Ok(axum::Json(resp))
}

/// `GET /organizations/{id}`
pub async fn get_org(
    State(state): State<Arc<AppState>>,
    auth: auth::AuthSession,
    Path(org_id): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    let session = auth;
    let org_id = parse_org_id(&org_id)?;
    let uid = parse_user_id_str(&session.user_id.to_string())?;

    // Must be a member to view.
    require_org_role(&state.db, org_id, uid, &["owner", "admin", "member"]).await?;

    let org = db::get_organization(&state.db, org_id)
        .await?
        .ok_or(AppError::NotFound {
            resource: "organization",
        })?;
    Ok(axum::Json(org_to_response(&org)))
}

/// `PATCH /organizations/{id}`
pub async fn update_org(
    State(state): State<Arc<AppState>>,
    auth: auth::AuthSession,
    Path(org_id): Path<String>,
    Json(req): Json<UpdateOrgRequest>,
) -> Result<impl IntoResponse, AppError> {
    let session = auth;
    let org_id = parse_org_id(&org_id)?;
    let uid = parse_user_id_str(&session.user_id.to_string())?;

    require_org_role(&state.db, org_id, uid, &["owner"]).await?;

    let org = db::update_organization(&state.db, org_id, &req.name)
        .await?
        .ok_or(AppError::NotFound {
            resource: "organization",
        })?;
    Ok(axum::Json(org_to_response(&org)))
}

/// `DELETE /organizations/{id}`
pub async fn delete_org(
    State(state): State<Arc<AppState>>,
    auth: auth::AuthSession,
    Path(org_id): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    let session = auth;
    let org_id = parse_org_id(&org_id)?;
    let uid = parse_user_id_str(&session.user_id.to_string())?;

    require_org_role(&state.db, org_id, uid, &["owner"]).await?;

    let deleted = db::delete_organization(&state.db, org_id).await?;
    if !deleted {
        return Err(AppError::NotFound {
            resource: "organization",
        });
    }
    Ok(axum::Json(serde_json::json!({ "status": "deleted" })))
}

/// `POST /organizations/{id}/members`
pub async fn add_member(
    State(state): State<Arc<AppState>>,
    auth: auth::AuthSession,
    Path(org_id): Path<String>,
    Json(req): Json<AddMemberRequest>,
) -> Result<impl IntoResponse, AppError> {
    let session = auth;
    let org_id = parse_org_id(&org_id)?;
    let uid = parse_user_id_str(&session.user_id.to_string())?;

    require_org_role(&state.db, org_id, uid, &["owner", "admin"]).await?;

    let member_user_id = parse_user_id_str(&req.user_id)?;

    let share_enc = req
        .shamir_share_enc
        .as_deref()
        .map(hex::decode)
        .transpose()
        .map_err(|e| AppError::Conflict {
            detail: format!("invalid share hex: {e}"),
        })?;

    db::add_org_member(
        &state.db,
        org_id,
        member_user_id,
        &req.role,
        share_enc.as_deref(),
    )
    .await?;

    Ok((
        axum::http::StatusCode::CREATED,
        axum::Json(serde_json::json!({ "status": "added" })),
    ))
}

/// `GET /organizations/{id}/members`
pub async fn list_members(
    State(state): State<Arc<AppState>>,
    auth: auth::AuthSession,
    Path(org_id): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    let session = auth;
    let org_id = parse_org_id(&org_id)?;
    let uid = parse_user_id_str(&session.user_id.to_string())?;

    require_org_role(&state.db, org_id, uid, &["owner", "admin", "member"]).await?;

    let members = db::list_org_members(&state.db, org_id).await?;
    let resp: Vec<OrgMemberResponse> = members.iter().map(member_to_response).collect();
    Ok(axum::Json(resp))
}

/// `DELETE /organizations/{id}/members/{uid}`
pub async fn remove_member(
    State(state): State<Arc<AppState>>,
    auth: auth::AuthSession,
    Path((org_id, target_uid)): Path<(String, String)>,
) -> Result<impl IntoResponse, AppError> {
    let session = auth;
    let org_id = parse_org_id(&org_id)?;
    let uid = parse_user_id_str(&session.user_id.to_string())?;
    let target_uid = parse_user_id_str(&target_uid)?;

    // Owner or admin can remove. Members cannot remove others.
    require_org_role(&state.db, org_id, uid, &["owner", "admin"]).await?;

    let removed = db::remove_org_member(&state.db, org_id, target_uid).await?;
    if !removed {
        return Err(AppError::NotFound {
            resource: "organization member",
        });
    }
    Ok(axum::Json(serde_json::json!({ "status": "removed" })))
}

/// `PATCH /organizations/{id}/members/{uid}`
pub async fn update_member_role(
    State(state): State<Arc<AppState>>,
    auth: auth::AuthSession,
    Path((org_id, target_uid)): Path<(String, String)>,
    Json(req): Json<UpdateMemberRoleRequest>,
) -> Result<impl IntoResponse, AppError> {
    let session = auth;
    let org_id = parse_org_id(&org_id)?;
    let uid = parse_user_id_str(&session.user_id.to_string())?;
    let target_uid = parse_user_id_str(&target_uid)?;

    require_org_role(&state.db, org_id, uid, &["owner"]).await?;

    db::update_org_member_role(&state.db, org_id, target_uid, &req.role).await?;
    Ok(axum::Json(serde_json::json!({ "status": "updated" })))
}

/// `POST /organizations/{id}/recover/{uid}`
///
/// Allows an organization admin/owner to recover a member's encrypted
/// master key. The caller must be an admin or owner of the organization.
///
/// Returns the target user's `org_master_key_enc` (hex-encoded bytes).
/// The caller is responsible for decrypting it client-side using their
/// copy of the organization private key, then re-encrypting it for the
/// target user.
///
/// Shamir's Secret Sharing (threshold recovery) will be added in a
/// future iteration as specified in the roadmap.
pub async fn recover_master_key(
    State(state): State<Arc<AppState>>,
    auth: auth::AuthSession,
    Path((org_id, target_uid)): Path<(String, String)>,
) -> Result<impl IntoResponse, AppError> {
    let session = auth;
    let org_id = parse_org_id(&org_id)?;
    let caller_uid = parse_user_id_str(&session.user_id.to_string())?;
    let target_uid = parse_user_id_str(&target_uid)?;

    require_org_role(&state.db, org_id, caller_uid, &["owner", "admin"]).await?;

    // Verify target user is a member of this org.
    db::get_org_member(&state.db, org_id, target_uid)
        .await?
        .ok_or(AppError::NotFound {
            resource: "organization member",
        })?;

    // Get the target user's encrypted org master key.
    let org_master_key_enc = db::get_user_org_master_key_enc(&state.db, target_uid)
        .await?
        .ok_or(AppError::NotFound {
            resource: "org master key",
        })?;

    // Log the recovery.
    db::log_org_recovery(&state.db, org_id, target_uid, caller_uid, None).await?;

    Ok(axum::Json(serde_json::json!({
        "status": "recovered",
        "master_key_enc": hex::encode(&org_master_key_enc),
    })))
}

#[cfg(test)]
mod tests {
    use super::*;

    // -- Parsers ------------------------------------------------------------

    #[test]
    fn parse_org_id_valid() {
        let id = "00000000-0000-0000-0000-000000000001";
        assert!(parse_org_id(id).is_ok());
    }

    #[test]
    fn parse_org_id_invalid() {
        assert!(parse_org_id("not-a-uuid").is_err());
    }

    #[test]
    fn parse_user_id_str_valid() {
        let id = "00000000-0000-0000-0000-000000000001";
        assert!(parse_user_id_str(id).is_ok());
    }

    #[test]
    fn parse_user_id_str_invalid() {
        assert!(parse_user_id_str("not-a-uuid").is_err());
    }

    // -- Response formatting ------------------------------------------------

    #[test]
    fn org_to_response_formats_correctly() {
        let org = db::OrganizationRow {
            id: uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(),
            name: "Test Org".into(),
            org_pubkey: vec![0u8; 32],
            shamir_threshold: 3,
            shamir_total: 5,
            created_at: chrono::Utc::now(),
        };
        let resp = org_to_response(&org);
        assert_eq!(resp.id, "00000000-0000-0000-0000-000000000001");
        assert_eq!(resp.name, "Test Org");
        assert_eq!(resp.shamir_threshold, 3);
        assert_eq!(resp.shamir_total, 5);
        assert_eq!(resp.org_pubkey, hex::encode(vec![0u8; 32]));
    }

    #[test]
    fn member_to_response_with_share() {
        let member = db::OrgMemberRow {
            user_id: uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(),
            email: "user@test.com".into(),
            display_name: "Test User".into(),
            role: "admin".into(),
            shamir_share_enc: Some(vec![1, 2, 3]),
            created_at: chrono::Utc::now(),
        };
        let resp = member_to_response(&member);
        assert_eq!(resp.user_id, "00000000-0000-0000-0000-000000000001");
        assert_eq!(resp.email, "user@test.com");
        assert_eq!(resp.display_name, "Test User");
        assert_eq!(resp.role, "admin");
        assert_eq!(resp.has_shamir_share, Some(true));
    }

    #[test]
    fn member_to_response_without_share() {
        let member = db::OrgMemberRow {
            user_id: uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000002").unwrap(),
            email: "other@test.com".into(),
            display_name: "Other User".into(),
            role: "member".into(),
            shamir_share_enc: None,
            created_at: chrono::Utc::now(),
        };
        let resp = member_to_response(&member);
        assert_eq!(resp.role, "member");
        assert_eq!(resp.has_shamir_share, Some(false));
    }

    // -- Integration tests (require Postgres + Redis) -----------------------

    #[cfg(test)]
    mod integration {
        use crate::routes;
        use axum::Router;
        use axum::body::Body;
        use axum::http::{Request, StatusCode};
        use tower::ServiceExt;

        fn build_org_router(state: std::sync::Arc<crate::state::AppState>) -> axum::Router {
            let api_router = Router::new()
                .route(
                    "/organizations",
                    axum::routing::get(routes::list_orgs).post(routes::create_org),
                )
                .route(
                    "/organizations/{id}",
                    axum::routing::get(routes::get_org)
                        .patch(routes::update_org)
                        .delete(routes::delete_org),
                )
                .route(
                    "/organizations/{id}/members",
                    axum::routing::post(routes::add_member).get(routes::list_members),
                )
                .route(
                    "/organizations/{id}/members/{uid}",
                    axum::routing::delete(routes::remove_member).patch(routes::update_member_role),
                )
                .route(
                    "/organizations/{id}/recover/{uid}",
                    axum::routing::post(routes::recover_master_key),
                );
            Router::new().nest("/api", api_router).with_state(state)
        }

        #[sqlx::test]
        #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
        #[allow(unreachable_code, unused_variables)]
        async fn list_organizations_requires_auth(pool: sqlx::PgPool) {
            let state = crate::test_helpers::test_app_state(pool).await;
            let app = build_org_router(state);
            let resp = app
                .oneshot(
                    Request::get("/api/organizations")
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
        }

        #[sqlx::test]
        #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
        #[allow(unreachable_code, unused_variables)]
        async fn get_unknown_organization_returns_unauthorized(pool: sqlx::PgPool) {
            let state = crate::test_helpers::test_app_state(pool).await;
            let app = build_org_router(state);
            let resp = app
                .oneshot(
                    Request::get("/api/organizations/00000000-0000-0000-0000-000000000001")
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
        }

        #[sqlx::test]
        #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
        #[allow(unreachable_code, unused_variables)]
        async fn delete_organization_requires_auth(pool: sqlx::PgPool) {
            let state = crate::test_helpers::test_app_state(pool).await;
            let app = build_org_router(state);
            let resp = app
                .oneshot(
                    Request::delete("/api/organizations/00000000-0000-0000-0000-000000000001")
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
        }

        #[sqlx::test]
        #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
        #[allow(unreachable_code, unused_variables)]
        async fn recovery_requires_auth(pool: sqlx::PgPool) {
            let state = crate::test_helpers::test_app_state(pool).await;
            let app = build_org_router(state);
            let resp = app
                .oneshot(
                    Request::post(
                        "/api/organizations/00000000-0000-0000-0000-000000000001/recover/00000000-0000-0000-0000-000000000002",
                    )
                    .body(Body::empty())
                    .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
        }
    }
}
