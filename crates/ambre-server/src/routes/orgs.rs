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

/// Privilege rank of an organization role. Higher binds more capability.
/// `owner` (2) > `admin` (1) > `member` (0). An unknown role ranks below
/// everything so it can never be granted.
fn org_role_rank(role: &str) -> Option<u8> {
    match role {
        "member" => Some(0),
        "admin" => Some(1),
        "owner" => Some(2),
        _ => None,
    }
}

fn parse_org_id(s: &str) -> Result<uuid::Uuid, AppError> {
    uuid::Uuid::parse_str(s).map_err(|_| AppError::NotFound {
        resource: "organization",
    })
}

fn parse_user_id_str(s: &str) -> Result<uuid::Uuid, AppError> {
    uuid::Uuid::parse_str(s).map_err(|_| AppError::NotFound { resource: "user" })
}

/// Validates the escrow parameters at org creation.
///
/// Shamir k-of-n threshold reconstruction is not yet implemented (see
/// `recover_master_key`, N1). Until it lands, recovery is single-authority
/// (owner-only), so the schema must not advertise a quorum guarantee the server
/// cannot keep: reject any threshold above 1. Basic bounds also keep
/// `1 <= threshold <= total` coherent.
fn validate_shamir_params(threshold: i16, total: i16) -> Result<(), AppError> {
    if total < 1 || threshold < 1 || threshold > total {
        return Err(AppError::Conflict {
            detail: "shamir_threshold/shamir_total out of range (need 1 <= threshold <= total)"
                .into(),
        });
    }
    if threshold > 1 {
        return Err(AppError::Conflict {
            detail: "shamir_threshold > 1 is not yet supported: threshold reconstruction is \
                     unimplemented and recovery is single-authority (owner-only)"
                .into(),
        });
    }
    Ok(())
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

    validate_shamir_params(req.shamir_threshold, req.shamir_total)?;

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

    let caller = require_org_role(&state.db, org_id, uid, &["owner", "admin"]).await?;

    let member_user_id = parse_user_id_str(&req.user_id)?;

    // `add_org_member` upserts (ON CONFLICT DO UPDATE SET role), so this handler
    // is also the role-change path. Two clamps keep it from being an escalation
    // vector, mirroring the document `invite` N2 clamp:
    //   1. A caller may only grant a role at or below their own rank, so an admin
    //      can never mint (or self-promote to) an owner.
    //   2. A caller may only touch a member currently at or below their own rank,
    //      so an admin can never demote or lock out an owner via the upsert.
    let caller_rank = org_role_rank(&caller.role).ok_or(AppError::Forbidden)?;
    let granted_rank = org_role_rank(&req.role).ok_or(AppError::Conflict {
        detail: "invalid role".into(),
    })?;
    if granted_rank > caller_rank {
        return Err(AppError::Forbidden);
    }
    if let Some(existing) = db::get_org_member(&state.db, org_id, member_user_id).await? {
        let target_rank = org_role_rank(&existing.role).ok_or(AppError::Forbidden)?;
        if target_rank > caller_rank {
            return Err(AppError::Forbidden);
        }
    }

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
    let caller = require_org_role(&state.db, org_id, uid, &["owner", "admin"]).await?;

    // Same rank clamp as `add_member`: a caller may only remove a member at or
    // below their own rank, so an admin can never remove (or lock out) the owner.
    let caller_rank = org_role_rank(&caller.role).ok_or(AppError::Forbidden)?;
    if let Some(existing) = db::get_org_member(&state.db, org_id, target_uid).await? {
        let target_rank = org_role_rank(&existing.role).ok_or(AppError::Forbidden)?;
        if target_rank > caller_rank {
            return Err(AppError::Forbidden);
        }
    }

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

    // Validate the role before hitting the DB so an unknown value returns a clean
    // 409 instead of surfacing the `role IN (...)` CHECK violation as a raw 500.
    org_role_rank(&req.role).ok_or(AppError::Conflict {
        detail: "invalid role".into(),
    })?;

    db::update_org_member_role(&state.db, org_id, target_uid, &req.role).await?;
    Ok(axum::Json(serde_json::json!({ "status": "updated" })))
}

/// `POST /organizations/{id}/recover/{uid}`
///
/// Allows an organization **owner** to recover a member's encrypted master key.
///
/// SINGLE-AUTHORITY recovery: Shamir k-of-n threshold reconstruction is not yet
/// implemented, so a lone owner can release a member's key. Recovery is therefore
/// deliberately restricted to the `owner` role (not `admin`) and org creation
/// rejects `shamir_threshold > 1`, so the data model never advertises a quorum the
/// server does not enforce (N1). When true threshold reconstruction lands, this
/// gate is replaced by "collect k shares before releasing anything".
///
/// Returns the target user's `org_master_key_enc` (hex-encoded bytes). The caller
/// decrypts it client-side with their copy of the organization private key, then
/// re-encrypts it for the target user. Every recovery is logged (`log_org_recovery`).
pub async fn recover_master_key(
    State(state): State<Arc<AppState>>,
    auth: auth::AuthSession,
    Path((org_id, target_uid)): Path<(String, String)>,
) -> Result<impl IntoResponse, AppError> {
    let session = auth;
    let org_id = parse_org_id(&org_id)?;
    let caller_uid = parse_user_id_str(&session.user_id.to_string())?;
    let target_uid = parse_user_id_str(&target_uid)?;

    require_org_role(&state.db, org_id, caller_uid, &["owner"]).await?;

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

    // -- Role rank ordering -------------------------------------------------

    #[test]
    fn org_role_rank_ordering() {
        assert!(org_role_rank("owner") > org_role_rank("admin"));
        assert!(org_role_rank("admin") > org_role_rank("member"));
        assert_eq!(org_role_rank("nonsense"), None);
    }

    /// An admin may grant admin/member but never owner, and the grant clamp
    /// rejects any role above the caller's own rank.
    #[test]
    fn admin_cannot_grant_owner() {
        let caller = org_role_rank("admin").unwrap();
        assert!(org_role_rank("owner").unwrap() > caller); // owner grant blocked
        assert!(org_role_rank("admin").unwrap() <= caller);
        assert!(org_role_rank("member").unwrap() <= caller);
    }

    // -- Shamir validation (N1) ---------------------------------------------

    #[test]
    fn shamir_threshold_1_accepted() {
        assert!(validate_shamir_params(1, 1).is_ok());
        assert!(validate_shamir_params(1, 5).is_ok());
    }

    #[test]
    fn shamir_threshold_above_1_rejected_until_reconstruction_lands() {
        // The data model must not advertise a k-of-n quorum the server does not
        // enforce; recovery is single-authority (owner-only) for now.
        assert!(validate_shamir_params(2, 3).is_err());
        assert!(validate_shamir_params(3, 5).is_err());
    }

    #[test]
    fn shamir_out_of_range_rejected() {
        assert!(validate_shamir_params(0, 5).is_err()); // threshold < 1
        assert!(validate_shamir_params(1, 0).is_err()); // total < 1
        assert!(validate_shamir_params(2, 1).is_err()); // threshold > total
    }

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
                .route("/auth/setup", axum::routing::post(routes::setup_account))
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

        // -- Role clamp on member add/update (privilege escalation) ---------

        use base64::Engine as _;

        /// Setup an account under `email`/`spk` and return a ready
        /// `Authorization: Bearer <jwt>` header. Each caller must pass a distinct
        /// `spk` byte: the `users.signing_pubkey` column is unique.
        async fn setup_org_user(app: &Router, email: &str, spk: u8) -> String {
            let body = serde_json::json!({
                "email": email,
                "display_name": "Org User",
                "master_key_enc": base64::engine::general_purpose::URL_SAFE.encode([1u8; 32]),
                "master_key_salt": base64::engine::general_purpose::URL_SAFE.encode([2u8; 16]),
                "signing_pubkey": hex::encode([spk; 32]),
                "exchange_pubkey": hex::encode([spk; 32]),
            });
            let resp = app
                .clone()
                .oneshot(
                    Request::post("/api/auth/setup")
                        .header("content-type", "application/json")
                        .body(Body::from(body.to_string()))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(resp.status(), StatusCode::OK);
            let bytes = axum::body::to_bytes(resp.into_body(), 8192).await.unwrap();
            let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
            format!("Bearer {}", json["access_token"].as_str().unwrap())
        }

        /// POST a member add/update as `auth`, returning the response status.
        async fn post_member(
            app: &Router,
            auth: &str,
            org_id: &str,
            user_id: &str,
            role: &str,
        ) -> StatusCode {
            let body = serde_json::json!({ "user_id": user_id, "role": role });
            app.clone()
                .oneshot(
                    Request::post(format!("/api/organizations/{org_id}/members"))
                        .header("content-type", "application/json")
                        .header("authorization", auth)
                        .body(Body::from(body.to_string()))
                        .unwrap(),
                )
                .await
                .unwrap()
                .status()
        }

        /// DELETE a member as `auth`, returning the response status.
        async fn delete_member(app: &Router, auth: &str, org_id: &str, uid: &str) -> StatusCode {
            app.clone()
                .oneshot(
                    Request::delete(format!("/api/organizations/{org_id}/members/{uid}"))
                        .header("authorization", auth)
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap()
                .status()
        }

        /// PATCH a member's role as `auth`, returning the response status.
        async fn patch_member_role(
            app: &Router,
            auth: &str,
            org_id: &str,
            uid: &str,
            role: &str,
        ) -> StatusCode {
            let body = serde_json::json!({ "role": role });
            app.clone()
                .oneshot(
                    Request::patch(format!("/api/organizations/{org_id}/members/{uid}"))
                        .header("content-type", "application/json")
                        .header("authorization", auth)
                        .body(Body::from(body.to_string()))
                        .unwrap(),
                )
                .await
                .unwrap()
                .status()
        }

        /// Create an org owned by `owner` and return its id.
        async fn create_org(app: &Router, owner: &str) -> String {
            let create = serde_json::json!({
                "name": "Acme",
                "org_pubkey": hex::encode([5u8; 32]),
                "shamir_threshold": 1,
                "shamir_total": 1,
            });
            let resp = app
                .clone()
                .oneshot(
                    Request::post("/api/organizations")
                        .header("content-type", "application/json")
                        .header("authorization", owner)
                        .body(Body::from(create.to_string()))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(resp.status(), StatusCode::CREATED);
            let bytes = axum::body::to_bytes(resp.into_body(), 8192).await.unwrap();
            serde_json::from_slice::<serde_json::Value>(&bytes).unwrap()["id"]
                .as_str()
                .unwrap()
                .to_string()
        }

        async fn user_id(state: &crate::state::AppState, email: &str) -> String {
            crate::db::get_user_by_email(&state.db, email)
                .await
                .unwrap()
                .unwrap()
                .id
                .to_string()
        }

        /// An org `admin` may add admins/members but must not be able to mint an
        /// `owner` (self-promotion) nor demote the existing owner via the upsert
        /// path. The document `invite` N2 clamp, applied to organizations.
        #[sqlx::test]
        #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
        async fn admin_cannot_escalate_via_add_member(pool: sqlx::PgPool) {
            let state = crate::test_helpers::test_app_state(pool).await;
            let app = build_org_router(state.clone());

            let owner = setup_org_user(&app, "owner@ambre.app", 10).await;
            let admin = setup_org_user(&app, "admin@ambre.app", 11).await;
            let _third = setup_org_user(&app, "member@ambre.app", 12).await;
            let owner_uid = crate::db::get_user_by_email(&state.db, "owner@ambre.app")
                .await
                .unwrap()
                .unwrap()
                .id
                .to_string();
            let admin_uid = crate::db::get_user_by_email(&state.db, "admin@ambre.app")
                .await
                .unwrap()
                .unwrap()
                .id
                .to_string();
            let member_uid = crate::db::get_user_by_email(&state.db, "member@ambre.app")
                .await
                .unwrap()
                .unwrap()
                .id
                .to_string();

            // Owner creates the org and promotes `admin` to admin.
            let create = serde_json::json!({
                "name": "Acme",
                "org_pubkey": hex::encode([5u8; 32]),
                "shamir_threshold": 1,
                "shamir_total": 1,
            });
            let resp = app
                .clone()
                .oneshot(
                    Request::post("/api/organizations")
                        .header("content-type", "application/json")
                        .header("authorization", &owner)
                        .body(Body::from(create.to_string()))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(resp.status(), StatusCode::CREATED);
            let bytes = axum::body::to_bytes(resp.into_body(), 8192).await.unwrap();
            let org_id = serde_json::from_slice::<serde_json::Value>(&bytes).unwrap()["id"]
                .as_str()
                .unwrap()
                .to_string();
            assert_eq!(
                post_member(&app, &owner, &org_id, &admin_uid, "admin").await,
                StatusCode::CREATED
            );

            // Escalation attempts by the admin are refused.
            assert_eq!(
                post_member(&app, &admin, &org_id, &admin_uid, "owner").await,
                StatusCode::FORBIDDEN,
                "admin must not self-promote to owner"
            );
            assert_eq!(
                post_member(&app, &admin, &org_id, &owner_uid, "member").await,
                StatusCode::FORBIDDEN,
                "admin must not demote the owner"
            );

            // The nominal path still works: admin adds a plain member.
            assert_eq!(
                post_member(&app, &admin, &org_id, &member_uid, "member").await,
                StatusCode::CREATED
            );
        }

        /// An org `admin` must not be able to remove the owner via DELETE, but
        /// may still remove a plain member (nominal path).
        #[sqlx::test]
        #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
        async fn admin_cannot_remove_owner(pool: sqlx::PgPool) {
            let state = crate::test_helpers::test_app_state(pool).await;
            let app = build_org_router(state.clone());

            let owner = setup_org_user(&app, "owner@ambre.app", 20).await;
            let admin = setup_org_user(&app, "admin@ambre.app", 21).await;
            let _third = setup_org_user(&app, "member@ambre.app", 22).await;
            let owner_uid = user_id(&state, "owner@ambre.app").await;
            let admin_uid = user_id(&state, "admin@ambre.app").await;
            let member_uid = user_id(&state, "member@ambre.app").await;

            let org_id = create_org(&app, &owner).await;
            assert_eq!(
                post_member(&app, &owner, &org_id, &admin_uid, "admin").await,
                StatusCode::CREATED
            );
            assert_eq!(
                post_member(&app, &owner, &org_id, &member_uid, "member").await,
                StatusCode::CREATED
            );

            // Admin must not remove the owner.
            assert_eq!(
                delete_member(&app, &admin, &org_id, &owner_uid).await,
                StatusCode::FORBIDDEN,
                "admin must not remove the owner"
            );

            // Nominal path: admin removes a plain member.
            assert_eq!(
                delete_member(&app, &admin, &org_id, &member_uid).await,
                StatusCode::OK
            );
        }

        /// `update_member_role` must reject an unknown role with a clean 409
        /// rather than letting the DB CHECK violation surface as a 500.
        #[sqlx::test]
        #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
        async fn update_member_role_rejects_invalid_role(pool: sqlx::PgPool) {
            let state = crate::test_helpers::test_app_state(pool).await;
            let app = build_org_router(state.clone());

            let owner = setup_org_user(&app, "owner@ambre.app", 30).await;
            let _member = setup_org_user(&app, "member@ambre.app", 31).await;
            let member_uid = user_id(&state, "member@ambre.app").await;

            let org_id = create_org(&app, &owner).await;
            assert_eq!(
                post_member(&app, &owner, &org_id, &member_uid, "member").await,
                StatusCode::CREATED
            );

            // Invalid role is rejected cleanly.
            assert_eq!(
                patch_member_role(&app, &owner, &org_id, &member_uid, "superuser").await,
                StatusCode::CONFLICT,
                "unknown role must be a clean 409, not a raw DB error"
            );

            // Nominal path: a valid role update succeeds.
            assert_eq!(
                patch_member_role(&app, &owner, &org_id, &member_uid, "admin").await,
                StatusCode::OK
            );
        }
    }
}
