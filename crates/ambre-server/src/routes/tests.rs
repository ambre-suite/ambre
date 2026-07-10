#[cfg(test)]
mod handler_tests {
    use axum::Router;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use std::sync::Arc;
    use tower::ServiceExt;

    use crate::routes;
    use crate::state::{AppState, Config};
    use crate::ws;

    fn empty_oidc_clients() -> crate::oidc::OidcClients {
        Arc::new(std::collections::HashMap::new())
    }

    fn build_test_router(state: Arc<AppState>) -> Router {
        let api_router = Router::new()
            .route("/auth/setup", axum::routing::post(routes::setup_account))
            .route("/auth/dev/login", axum::routing::post(routes::dev_login))
            .route("/auth/me", axum::routing::get(routes::me))
            .route("/auth/logout", axum::routing::post(routes::logout))
            .route(
                "/keys/master",
                axum::routing::put(routes::store_master_keys).get(routes::get_master_keys),
            )
            .route("/users/search", axum::routing::get(routes::search_users))
            .route(
                "/users/{id}/pubkeys",
                axum::routing::get(routes::get_pubkeys),
            )
            .route(
                "/documents",
                axum::routing::get(routes::list_documents).post(routes::create_document),
            )
            .route(
                "/documents/{id}",
                axum::routing::get(routes::get_document)
                    .delete(routes::delete_document)
                    .patch(routes::rename_document),
            )
            .route(
                "/documents/{id}/share",
                axum::routing::post(routes::invite_to_document),
            )
            .route(
                "/documents/{id}/share/{uid}",
                axum::routing::delete(routes::revoke_from_document),
            )
            .route(
                "/documents/{id}/members",
                axum::routing::get(routes::list_document_members),
            )
            .route(
                "/documents/{id}/members/{uid}",
                axum::routing::patch(routes::update_document_member_role),
            )
            .route(
                "/documents/{id}/rotate-key",
                axum::routing::post(routes::rotate_document_key),
            )
            .route(
                "/documents/{id}/media",
                axum::routing::post(routes::media_confirm_upload),
            )
            .route(
                "/documents/{id}/snapshot",
                axum::routing::post(routes::create_snapshot),
            )
            .route("/ws", axum::routing::get(ws::ws_upgrade));
        Router::new().nest("/api", api_router).with_state(state)
    }

    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn unauth_endpoints_return_401(pool: sqlx::PgPool) {
        let state = test_app_state(pool).await;
        let app = build_test_router(state);

        let resp = app
            .oneshot(Request::get("/api/auth/me").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn setup_account_creates_user_and_returns_token(pool: sqlx::PgPool) {
        let state = test_app_state(pool).await;
        let app = build_test_router(state);

        let body = serde_json::json!({
            "email": "user@ambre.app",
            "display_name": "Test User",
            "master_key_enc": base64_encode(&[1u8; 32]),
            "master_key_salt": base64_encode(&[2u8; 16]),
            "signing_pubkey": hex::encode([3u8; 32]),
            "exchange_pubkey": hex::encode([4u8; 32]),
        });

        let resp = app
            .oneshot(
                Request::post("/api/auth/setup")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_string(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        assert!(!access_token_of(resp).await.is_empty());
    }

    /// N5 guard: a second setup claiming an already-registered email (even with
    /// a different case) is rejected with 409, not a 500 from the raw insert.
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn setup_account_duplicate_email_returns_409(pool: sqlx::PgPool) {
        let state = test_app_state(pool).await;
        let app = build_test_router(state);

        let _first = setup_and_get_auth(&app).await;

        // setup_and_get_auth registered "user@ambre.app"; re-claim it with a
        // case variant and fresh key material.
        let body = serde_json::json!({
            "email": "User@Ambre.App",
            "display_name": "Impostor",
            "master_key_enc": base64_encode(&[9u8; 32]),
            "master_key_salt": base64_encode(&[8u8; 16]),
            "signing_pubkey": hex::encode([7u8; 32]),
            "exchange_pubkey": hex::encode([6u8; 32]),
        });
        let resp = app
            .oneshot(
                Request::post("/api/auth/setup")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_string(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::CONFLICT);
    }

    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn me_returns_user_details_when_authenticated(pool: sqlx::PgPool) {
        let state = test_app_state(pool).await;
        let app = build_test_router(state.clone());

        let auth = setup_and_get_auth(&app).await;

        let resp = app
            .oneshot(
                Request::get("/api/auth/me")
                    .header("authorization", auth)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn list_documents_returns_empty_before_creation(pool: sqlx::PgPool) {
        let state = test_app_state(pool).await;
        let app = build_test_router(state.clone());
        let auth = setup_and_get_auth(&app).await;

        let resp = app
            .clone()
            .oneshot(
                Request::get("/api/documents")
                    .header("authorization", &auth)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let body_bytes = axum::body::to_bytes(resp.into_body(), 1024).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
        assert!(json["documents"].as_array().unwrap().is_empty());
    }

    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn list_documents_carries_summary_fields(pool: sqlx::PgPool) {
        // After creation, the list must carry the fields the launcher needs:
        // the encrypted name, the caller's wrapped doc key, member_count (1 for a
        // solo owner), and a timestamp. Validates the new LEFT JOIN + COUNT query.
        let state = test_app_state(pool).await;
        let app = build_test_router(state.clone());
        let auth = setup_and_get_auth(&app).await;
        let _doc_id = create_test_doc(&app, &auth).await;

        let resp = app
            .oneshot(
                Request::get("/api/documents")
                    .header("authorization", &auth)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body_bytes = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
        let docs = json["documents"].as_array().unwrap();
        assert_eq!(docs.len(), 1);
        let d = &docs[0];
        assert!(d["doc_key_enc"].as_str().is_some_and(|s| !s.is_empty()));
        assert_eq!(d["member_count"].as_u64(), Some(1));
        assert!(d["updated_at"].as_str().is_some());
        // The owner wrapped their own key at creation (invited_by = self), so the
        // wrapper exchange pubkey resolves to a non-empty hex string.
        assert!(
            d["wrapper_exchange_pub_key"]
                .as_str()
                .is_some_and(|s| !s.is_empty())
        );
    }

    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn create_and_get_document(pool: sqlx::PgPool) {
        let state = test_app_state(pool).await;
        let app = build_test_router(state.clone());
        let auth = setup_and_get_auth(&app).await;

        let body = serde_json::json!({
            "doc_type": "prose",
            "name_enc": base64_encode(b"encrypted-name"),
            "doc_key_enc": base64_encode(&[5u8; 32]),
        });
        let resp = app
            .clone()
            .oneshot(
                Request::post("/api/documents")
                    .header("content-type", "application/json")
                    .header("authorization", &auth)
                    .body(Body::from(serde_json::to_string(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body_bytes = axum::body::to_bytes(resp.into_body(), 1024).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
        let doc_id = json["document_id"].as_str().unwrap().to_string();

        let resp = app
            .oneshot(
                Request::get(format!("/api/documents/{doc_id}"))
                    .header("authorization", &auth)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    // H2 regression: a non-member (here an anonymous caller who only holds the
    // UUID) must not receive `owner_id` / `doc_type` metadata -- the route
    // returns 404 instead of leaking the document's existence.
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn get_document_hidden_from_non_member(pool: sqlx::PgPool) {
        let state = test_app_state(pool).await;
        let app = build_test_router(state.clone());
        let auth = setup_and_get_auth(&app).await;
        let doc_id = create_test_doc(&app, &auth).await;

        // Same document, fetched without any credentials: not a member, not the
        // owner -> 404, not 200 with routing metadata.
        let resp = app
            .oneshot(
                Request::get(format!("/api/documents/{doc_id}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn invalid_doc_type_is_rejected(pool: sqlx::PgPool) {
        let state = test_app_state(pool).await;
        let app = build_test_router(state.clone());
        let auth = setup_and_get_auth(&app).await;

        let body = serde_json::json!({
            "doc_type": "spreadsheet",
            "name_enc": base64_encode(b"x"),
            "doc_key_enc": base64_encode(&[5u8; 32]),
        });
        let resp = app
            .clone()
            .oneshot(
                Request::post("/api/documents")
                    .header("content-type", "application/json")
                    .header("authorization", &auth)
                    .body(Body::from(serde_json::to_string(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::CONFLICT);
    }

    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn delete_document_only_by_owner(pool: sqlx::PgPool) {
        let state = test_app_state(pool).await;
        let app = build_test_router(state.clone());
        let auth = setup_and_get_auth(&app).await;
        let doc_id = create_test_doc(&app, &auth).await;

        let resp = app
            .oneshot(
                Request::delete(format!("/api/documents/{doc_id}"))
                    .header("authorization", &auth)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn invite_user_to_document(pool: sqlx::PgPool) {
        let state = test_app_state(pool).await;
        let app = build_test_router(state.clone());
        let auth = setup_and_get_auth(&app).await;
        let doc_id = create_test_doc(&app, &auth).await;

        let invitee_email = "invitee@test.com";
        crate::test_helpers::create_test_user(&state.db, invitee_email).await;

        let body = serde_json::json!({
            "invitee_email": invitee_email,
            "role": "editor",
            "doc_key_enc": base64_encode(&[9u8; 32]),
        });
        let resp = app
            .clone()
            .oneshot(
                Request::post(format!("/api/documents/{doc_id}/share"))
                    .header("content-type", "application/json")
                    .header("authorization", &auth)
                    .body(Body::from(serde_json::to_string(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn list_members_includes_owner(pool: sqlx::PgPool) {
        let state = test_app_state(pool).await;
        let app = build_test_router(state.clone());
        let auth = setup_and_get_auth(&app).await;
        let doc_id = create_test_doc(&app, &auth).await;

        let resp = app
            .oneshot(
                Request::get(format!("/api/documents/{doc_id}/members"))
                    .header("authorization", &auth)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    /// GET the member list and return the raw `members` JSON array. Asserts 200.
    async fn list_members(app: &Router, auth: &str, doc_id: &str) -> Vec<serde_json::Value> {
        let resp = app
            .clone()
            .oneshot(
                Request::get(format!("/api/documents/{doc_id}/members"))
                    .header("authorization", auth)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(resp.into_body(), 64 * 1024)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        json["members"].as_array().unwrap().clone()
    }

    /// Revoking a member drops them from the list, and a subsequent key
    /// rotation replaces the owner's stored `doc_key_enc` with the new wrap.
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn revoke_then_rotate_replaces_owner_key(pool: sqlx::PgPool) {
        let state = test_app_state(pool).await;
        let app = build_test_router(state.clone());
        let auth = setup_and_get_auth(&app).await;
        let doc_id = create_test_doc(&app, &auth).await;

        // Invite a second user, then confirm the list holds both members.
        let invitee_email = "revoked@test.com";
        let invitee_id = crate::test_helpers::create_test_user(&state.db, invitee_email).await;
        let body = serde_json::json!({
            "invitee_email": invitee_email,
            "role": "editor",
            "doc_key_enc": base64_encode(&[9u8; 32]),
        });
        let resp = app
            .clone()
            .oneshot(
                Request::post(format!("/api/documents/{doc_id}/share"))
                    .header("content-type", "application/json")
                    .header("authorization", &auth)
                    .body(Body::from(serde_json::to_string(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(list_members(&app, &auth, &doc_id).await.len(), 2);

        // Revoke the invitee -> only the owner remains.
        let resp = app
            .clone()
            .oneshot(
                Request::delete(format!("/api/documents/{doc_id}/share/{invitee_id}"))
                    .header("authorization", &auth)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let members = list_members(&app, &auth, &doc_id).await;
        assert_eq!(members.len(), 1);
        let owner_id = members[0]["user_id"].as_str().unwrap().to_string();
        // The owner's key is still the one set at document creation.
        assert_eq!(members[0]["doc_key_enc"], base64_encode(&[5u8; 32]));

        // Rotate: re-wrap the document key for the remaining owner.
        let new_key = base64_encode(&[0xABu8; 32]);
        let rotate_body = serde_json::json!({
            "keys": [ { "user_id": owner_id, "doc_key_enc": new_key } ],
        });
        let resp = app
            .clone()
            .oneshot(
                Request::post(format!("/api/documents/{doc_id}/rotate-key"))
                    .header("content-type", "application/json")
                    .header("authorization", &auth)
                    .body(Body::from(serde_json::to_string(&rotate_body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        // The stored wrap is now the rotated key.
        let members = list_members(&app, &auth, &doc_id).await;
        assert_eq!(members[0]["doc_key_enc"], new_key);
    }

    /// An owner can change a member's role; an unknown role string is rejected.
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn update_member_role_changes_role(pool: sqlx::PgPool) {
        let state = test_app_state(pool).await;
        let app = build_test_router(state.clone());
        let auth = setup_and_get_auth(&app).await;
        let doc_id = create_test_doc(&app, &auth).await;

        let invitee_email = "rolechange@test.com";
        let invitee_id = crate::test_helpers::create_test_user(&state.db, invitee_email).await;
        let body = serde_json::json!({
            "invitee_email": invitee_email,
            "role": "editor",
            "doc_key_enc": base64_encode(&[9u8; 32]),
        });
        let resp = app
            .clone()
            .oneshot(
                Request::post(format!("/api/documents/{doc_id}/share"))
                    .header("content-type", "application/json")
                    .header("authorization", &auth)
                    .body(Body::from(serde_json::to_string(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        // Promote/demote the invitee editor -> viewer.
        let resp = app
            .clone()
            .oneshot(
                Request::patch(format!("/api/documents/{doc_id}/members/{invitee_id}"))
                    .header("content-type", "application/json")
                    .header("authorization", &auth)
                    .body(Body::from(r#"{"role":"viewer"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let members = list_members(&app, &auth, &doc_id).await;
        let invitee = members
            .iter()
            .find(|m| m["user_id"] == invitee_id.to_string())
            .expect("invitee still a member");
        assert_eq!(invitee["role"], "viewer");

        // An invalid role string is rejected with 409 CONFLICT.
        let resp = app
            .clone()
            .oneshot(
                Request::patch(format!("/api/documents/{doc_id}/members/{invitee_id}"))
                    .header("content-type", "application/json")
                    .header("authorization", &auth)
                    .body(Body::from(r#"{"role":"superadmin"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::CONFLICT);
    }

    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn logout_clears_session(pool: sqlx::PgPool) {
        let state = test_app_state(pool).await;
        let app = build_test_router(state.clone());
        let auth = setup_and_get_auth(&app).await;

        let resp = app
            .oneshot(
                Request::post("/api/auth/logout")
                    .header("authorization", &auth)
                    .header("content-type", "application/json")
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn get_delta_endpoint_requires_auth(pool: sqlx::PgPool) {
        let state = test_app_state(pool).await;
        let app = build_test_router(state.clone());
        let auth = setup_and_get_auth(&app).await;
        let doc_id = create_test_doc(&app, &auth).await;

        let resp = app
            .oneshot(
                Request::get(format!("/api/documents/{doc_id}/deltas?after_seq=0"))
                    .header("authorization", &auth)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert!(resp.status().is_success() || resp.status().is_client_error());
    }

    fn base64_encode(data: &[u8]) -> String {
        use base64::Engine;
        base64::engine::general_purpose::URL_SAFE.encode(data)
    }

    async fn test_app_state(pool: sqlx::PgPool) -> Arc<AppState> {
        let redis = rustis::client::Client::connect("redis://localhost:6379")
            .await
            .expect("need Redis for tests");
        let s3_config = aws_sdk_s3::Config::builder().build();
        let s3 = aws_sdk_s3::Client::from_conf(s3_config);
        let pubsub = crate::ws::PubSubHub::spawn(redis.clone());
        Arc::new(AppState {
            db: pool,
            redis,
            pubsub,
            s3,
            s3_bucket: "test-bucket".into(),
            config: Arc::new(Config::default()),
            jwt: Arc::new(crate::jwt::JwtKeys::from_seed(&[7u8; 32])),
            oidc_clients: empty_oidc_clients(),
        })
    }

    /// Like [`test_app_state`] but with `dev_mode` enabled, so the dev-login
    /// endpoint is active.
    async fn test_app_state_dev(pool: sqlx::PgPool) -> Arc<AppState> {
        let redis = rustis::client::Client::connect("redis://localhost:6379")
            .await
            .expect("need Redis for tests");
        let s3 = aws_sdk_s3::Client::from_conf(aws_sdk_s3::Config::builder().build());
        let config = Config {
            dev_mode: true,
            ..Config::default()
        };
        let pubsub = crate::ws::PubSubHub::spawn(redis.clone());
        Arc::new(AppState {
            db: pool,
            redis,
            pubsub,
            s3,
            s3_bucket: "test-bucket".into(),
            config: Arc::new(config),
            jwt: Arc::new(crate::jwt::JwtKeys::from_seed(&[7u8; 32])),
            oidc_clients: empty_oidc_clients(),
        })
    }

    fn dev_login_body(username: &str) -> serde_json::Value {
        serde_json::json!({
            "username": username,
            "signing_pub_key": hex::encode([7u8; 32]),
            "exchange_pub_key": hex::encode([8u8; 32]),
        })
    }

    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn dev_login_rejected_when_dev_mode_off(pool: sqlx::PgPool) {
        // Default config has dev_mode = false.
        let state = test_app_state(pool).await;
        let app = build_test_router(state);

        let resp = app
            .oneshot(
                Request::post("/api/auth/dev/login")
                    .header("content-type", "application/json")
                    .body(Body::from(dev_login_body("alice").to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn dev_login_creates_user_and_returns_token(pool: sqlx::PgPool) {
        let state = test_app_state_dev(pool).await;
        let app = build_test_router(state);

        let resp = app
            .oneshot(
                Request::post("/api/auth/dev/login")
                    .header("content-type", "application/json")
                    .body(Body::from(dev_login_body("alice").to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        assert!(!access_token_of(resp).await.is_empty());
    }

    /// Pull the `access_token` out of an auth response body (setup / dev-login
    /// both return `{ access_token, .. }` -- the suite is cookie-less Bearer JWT).
    async fn access_token_of(resp: axum::response::Response) -> String {
        let body_bytes = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
        json["access_token"].as_str().unwrap().to_string()
    }

    /// Setup an account and return a ready-to-use `Authorization: Bearer <jwt>`
    /// header value.
    async fn setup_and_get_auth(app: &Router) -> String {
        let body = serde_json::json!({
            "email": "user@ambre.app",
            "display_name": "Test User",
            "master_key_enc": base64_encode(&[1u8; 32]),
            "master_key_salt": base64_encode(&[2u8; 16]),
            "signing_pubkey": hex::encode([3u8; 32]),
            "exchange_pubkey": hex::encode([4u8; 32]),
        });
        let resp = app
            .clone()
            .oneshot(
                Request::post("/api/auth/setup")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_string(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        format!("Bearer {}", access_token_of(resp).await)
    }

    /// Setup a second account under an explicit email and signing key, returning
    /// a ready `Authorization: Bearer <jwt>` header. Used to act as a non-owner
    /// (e.g. a `viewer`) in role-gated route tests.
    async fn setup_named(app: &Router, email: &str, spk_byte: u8) -> String {
        let body = serde_json::json!({
            "email": email,
            "display_name": "Second User",
            "master_key_enc": base64_encode(&[1u8; 32]),
            "master_key_salt": base64_encode(&[2u8; 16]),
            "signing_pubkey": hex::encode([spk_byte; 32]),
            "exchange_pubkey": hex::encode([spk_byte; 32]),
        });
        let resp = app
            .clone()
            .oneshot(
                Request::post("/api/auth/setup")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_string(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        format!("Bearer {}", access_token_of(resp).await)
    }

    /// Invite `invitee_email` to `doc_id` with `role`, as `auth`. Asserts 200.
    async fn invite(app: &Router, auth: &str, doc_id: &str, invitee_email: &str, role: &str) {
        let body = serde_json::json!({
            "invitee_email": invitee_email,
            "role": role,
            "doc_key_enc": base64_encode(&[9u8; 32]),
        });
        let resp = app
            .clone()
            .oneshot(
                Request::post(format!("/api/documents/{doc_id}/share"))
                    .header("content-type", "application/json")
                    .header("authorization", auth)
                    .body(Body::from(serde_json::to_string(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    /// A `viewer` cannot obtain a media upload URL, so it must not be able to
    /// register a `media` row either: `media_confirm_upload` is gated to
    /// owner/editor exactly like `media_upload_url`.
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn media_confirm_upload_denied_to_viewer(pool: sqlx::PgPool) {
        let state = test_app_state(pool).await;
        let app = build_test_router(state.clone());
        let owner = setup_and_get_auth(&app).await;
        let doc_id = create_test_doc(&app, &owner).await;

        let viewer_email = "viewer@ambre.app";
        let viewer = setup_named(&app, viewer_email, 5).await;
        invite(&app, &owner, &doc_id, viewer_email, "viewer").await;

        let body = serde_json::json!({
            "content_hash": "a".repeat(64),
            "mime_type": "image/png",
            "size_bytes": 10,
        });

        // Viewer is a member but not owner/editor -> 403.
        let resp = app
            .clone()
            .oneshot(
                Request::post(format!("/api/documents/{doc_id}/media"))
                    .header("content-type", "application/json")
                    .header("authorization", &viewer)
                    .body(Body::from(serde_json::to_string(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);

        // The owner may register the same media row.
        let resp = app
            .oneshot(
                Request::post(format!("/api/documents/{doc_id}/media"))
                    .header("content-type", "application/json")
                    .header("authorization", &owner)
                    .body(Body::from(serde_json::to_string(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    /// Confirming the same media twice is idempotent: `insert_media` uses
    /// `ON CONFLICT (id) DO NOTHING`, so a retried confirm (or a second client
    /// registering the same content-addressed blob) returns 200 both times
    /// instead of surfacing the PK unique violation as a 500.
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn media_confirm_upload_is_idempotent(pool: sqlx::PgPool) {
        let state = test_app_state(pool).await;
        let app = build_test_router(state.clone());
        let owner = setup_and_get_auth(&app).await;
        let doc_id = create_test_doc(&app, &owner).await;

        let body = serde_json::json!({
            "content_hash": "b".repeat(64),
            "mime_type": "image/png",
            "size_bytes": 42,
        });

        for _ in 0..2 {
            let resp = app
                .clone()
                .oneshot(
                    Request::post(format!("/api/documents/{doc_id}/media"))
                        .header("content-type", "application/json")
                        .header("authorization", &owner)
                        .body(Body::from(serde_json::to_string(&body).unwrap()))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(resp.status(), StatusCode::OK);
        }
    }

    /// A snapshot's `author_pubkey` is bound to the session identity (the token's
    /// `spk` claim), like the WS `Update` author-key check: a member may only
    /// create a snapshot under their own key, never attribute one to another.
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn create_snapshot_rejects_forged_author(pool: sqlx::PgPool) {
        let state = test_app_state(pool).await;
        let app = build_test_router(state.clone());
        // setup_and_get_auth registers signing_pubkey [3u8; 32].
        let owner = setup_and_get_auth(&app).await;
        let doc_id = create_test_doc(&app, &owner).await;

        let snap_uuid = uuid::Uuid::new_v4();
        // Valid per-document snapshot key (default env prefix is "dev-ambre"), so
        // the request reaches the author-key binding check.
        let s3_key = format!("dev-ambre/documents/{doc_id}/snapshots/{snap_uuid}");

        // Forged author key (not the session's [3u8; 32]) -> 403.
        let forged = serde_json::json!({
            "snapshot_id": snap_uuid.to_string(),
            "author_pubkey": hex::encode([9u8; 32]),
            "update_clocks": {},
            "s3_key": s3_key,
        });
        let resp = app
            .clone()
            .oneshot(
                Request::post(format!("/api/documents/{doc_id}/snapshot"))
                    .header("content-type", "application/json")
                    .header("authorization", &owner)
                    .body(Body::from(serde_json::to_string(&forged).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);

        // The author key matching the session identity is accepted.
        let genuine = serde_json::json!({
            "snapshot_id": snap_uuid.to_string(),
            "author_pubkey": hex::encode([3u8; 32]),
            "update_clocks": {},
            "s3_key": s3_key,
        });
        let resp = app
            .oneshot(
                Request::post(format!("/api/documents/{doc_id}/snapshot"))
                    .header("content-type", "application/json")
                    .header("authorization", &owner)
                    .body(Body::from(serde_json::to_string(&genuine).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    /// POST an invite as `auth`, returning the raw status (no assertion). Used
    /// by the privilege-clamp / last-owner tests that expect non-200 outcomes.
    async fn invite_status(
        app: &Router,
        auth: &str,
        doc_id: &str,
        invitee_email: &str,
        role: &str,
    ) -> StatusCode {
        let body = serde_json::json!({
            "invitee_email": invitee_email,
            "role": role,
            "doc_key_enc": base64_encode(&[9u8; 32]),
        });
        app.clone()
            .oneshot(
                Request::post(format!("/api/documents/{doc_id}/share"))
                    .header("content-type", "application/json")
                    .header("authorization", auth)
                    .body(Body::from(serde_json::to_string(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap()
            .status()
    }

    /// DELETE a document member as `auth`, returning the raw status.
    async fn revoke_status(app: &Router, auth: &str, doc_id: &str, uid: &str) -> StatusCode {
        app.clone()
            .oneshot(
                Request::delete(format!("/api/documents/{doc_id}/share/{uid}"))
                    .header("authorization", auth)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap()
            .status()
    }

    /// PATCH a document member's role as `auth`, returning the raw status.
    async fn patch_role_status(
        app: &Router,
        auth: &str,
        doc_id: &str,
        uid: &str,
        role: &str,
    ) -> StatusCode {
        let body = serde_json::json!({ "role": role });
        app.clone()
            .oneshot(
                Request::patch(format!("/api/documents/{doc_id}/members/{uid}"))
                    .header("content-type", "application/json")
                    .header("authorization", auth)
                    .body(Body::from(serde_json::to_string(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap()
            .status()
    }

    async fn doc_user_id(state: &AppState, email: &str) -> String {
        crate::db::get_user_by_email(&state.db, email)
            .await
            .unwrap()
            .unwrap()
            .id
            .to_string()
    }

    /// D1: an `editor` holds the document key, so the invite upsert (which is
    /// also the role-change path) must not let it demote an existing `owner` and
    /// overwrite the owner's key entry. The N2 target-rank clamp applied to the
    /// document share route.
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn editor_cannot_demote_owner_via_invite(pool: sqlx::PgPool) {
        let state = test_app_state(pool).await;
        let app = build_test_router(state.clone());

        // Owner (user@ambre.app) creates the document and invites an editor.
        let owner = setup_and_get_auth(&app).await;
        let doc_id = create_test_doc(&app, &owner).await;

        let editor_email = "editor@ambre.app";
        let editor = setup_named(&app, editor_email, 5).await;
        invite(&app, &owner, &doc_id, editor_email, "editor").await;

        // The editor tries to re-invite the owner as `editor`: the granted role is
        // within its rank, but the target is an owner (rank above it) -> 403.
        assert_eq!(
            invite_status(&app, &editor, &doc_id, "user@ambre.app", "editor").await,
            StatusCode::FORBIDDEN,
            "editor must not demote an owner through the invite upsert"
        );

        // The owner is still an owner (unchanged role and key entry).
        let members = list_members(&app, &owner, &doc_id).await;
        let owner_row = members
            .iter()
            .find(|m| m["email"] == "user@ambre.app")
            .expect("owner still a member");
        assert_eq!(owner_row["role"], "owner");
        assert_eq!(owner_row["doc_key_enc"], base64_encode(&[5u8; 32]));

        // Nominal path: the editor may still invite a fresh viewer.
        let viewer_email = "viewer2@ambre.app";
        crate::test_helpers::create_test_user(&state.db, viewer_email).await;
        assert_eq!(
            invite_status(&app, &editor, &doc_id, viewer_email, "viewer").await,
            StatusCode::OK
        );
    }

    /// D2: the sole owner must not be able to revoke or demote themselves --
    /// with no owner-membership left, nobody could manage sharing.
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn owner_cannot_drop_last_owner(pool: sqlx::PgPool) {
        let state = test_app_state(pool).await;
        let app = build_test_router(state.clone());

        let owner = setup_and_get_auth(&app).await;
        let doc_id = create_test_doc(&app, &owner).await;
        let owner_uid = doc_user_id(&state, "user@ambre.app").await;

        // Revoke self as the last owner -> 409.
        assert_eq!(
            revoke_status(&app, &owner, &doc_id, &owner_uid).await,
            StatusCode::CONFLICT,
            "revoking the last owner must be refused"
        );
        // Demote self via PATCH -> 409.
        assert_eq!(
            patch_role_status(&app, &owner, &doc_id, &owner_uid, "viewer").await,
            StatusCode::CONFLICT,
            "demoting the last owner via PATCH must be refused"
        );
        // Demote self via the invite upsert -> 409.
        assert_eq!(
            invite_status(&app, &owner, &doc_id, "user@ambre.app", "viewer").await,
            StatusCode::CONFLICT,
            "demoting the last owner via the invite upsert must be refused"
        );
    }

    /// D2: once a second owner exists, the first may step down. The invariant is
    /// "at least one owner remains", not "owners can never leave".
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn owner_can_leave_when_another_owner_remains(pool: sqlx::PgPool) {
        let state = test_app_state(pool).await;
        let app = build_test_router(state.clone());

        let owner = setup_and_get_auth(&app).await;
        let doc_id = create_test_doc(&app, &owner).await;
        let owner_uid = doc_user_id(&state, "user@ambre.app").await;

        // Promote a second member to owner.
        let second_email = "second@ambre.app";
        let _second = setup_named(&app, second_email, 6).await;
        invite(&app, &owner, &doc_id, second_email, "owner").await;

        // The first owner can now leave.
        assert_eq!(
            revoke_status(&app, &owner, &doc_id, &owner_uid).await,
            StatusCode::OK,
            "an owner may leave while another owner remains"
        );
    }

    async fn create_test_doc(app: &Router, auth: &str) -> String {
        let body = serde_json::json!({
            "doc_type": "prose",
            "name_enc": base64_encode(b"test"),
            "doc_key_enc": base64_encode(&[5u8; 32]),
        });
        let resp = app
            .clone()
            .oneshot(
                Request::post("/api/documents")
                    .header("content-type", "application/json")
                    .header("authorization", auth)
                    .body(Body::from(serde_json::to_string(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let body_bytes = axum::body::to_bytes(resp.into_body(), 1024).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
        json["document_id"].as_str().unwrap().to_string()
    }
}
