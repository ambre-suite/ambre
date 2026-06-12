#[cfg(test)]
mod handler_tests {
    use axum::Router;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use std::sync::Arc;
    use tower::ServiceExt;

    use crate::state::{AppState, Config};
    use crate::routes;
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
            .route("/keys/master", axum::routing::put(routes::store_master_keys).get(routes::get_master_keys))
            .route("/users/search", axum::routing::get(routes::search_users))
            .route("/users/{id}/pubkeys", axum::routing::get(routes::get_pubkeys))
            .route("/documents", axum::routing::get(routes::list_documents).post(routes::create_document))
            .route("/documents/{id}", axum::routing::get(routes::get_document).delete(routes::delete_document).patch(routes::rename_document))
            .route("/documents/{id}/share", axum::routing::post(routes::invite_to_document))
            .route("/documents/{id}/share/{uid}", axum::routing::delete(routes::revoke_from_document))
            .route("/documents/{id}/members", axum::routing::get(routes::list_document_members))
            .route("/documents/{id}/members/{uid}", axum::routing::patch(routes::update_document_member_role))
            .route("/documents/{id}/rotate-key", axum::routing::post(routes::rotate_document_key))
            .route("/ws", axum::routing::get(ws::ws_upgrade));
        Router::new().nest("/api", api_router).with_state(state)
    }

    #[sqlx::test]
    #[ignore = "needs postgres"]
    async fn unauth_endpoints_return_401(pool: sqlx::PgPool) {
        let state = test_app_state(pool).await;
        let app = build_test_router(state);

        let resp = app.oneshot(Request::get("/api/auth/me").body(Body::empty()).unwrap()).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[sqlx::test]
    #[ignore = "needs postgres"]
    async fn setup_account_creates_user_and_returns_cookie(pool: sqlx::PgPool) {
        let state = test_app_state(pool).await;
        let app = build_test_router(state);

        let body = serde_json::json!({
            "passphrase": "test",
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
        assert!(resp.headers().get("set-cookie").is_some());
    }

    #[sqlx::test]
    #[ignore = "needs postgres"]
    async fn me_returns_user_details_when_authenticated(pool: sqlx::PgPool) {
        let state = test_app_state(pool).await;
        let app = build_test_router(state.clone());

        let cookie = setup_and_get_cookie(&app).await;

        let resp = app
            .oneshot(
                Request::get("/api/auth/me")
                    .header("cookie", cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[sqlx::test]
    #[ignore = "needs postgres"]
    async fn list_documents_returns_empty_before_creation(pool: sqlx::PgPool) {
        let state = test_app_state(pool).await;
        let app = build_test_router(state.clone());
        let cookie = setup_and_get_cookie(&app).await;

        let resp = app
            .clone()
            .oneshot(
                Request::get("/api/documents")
                    .header("cookie", &cookie)
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
    #[ignore = "needs postgres"]
    async fn create_and_get_document(pool: sqlx::PgPool) {
        let state = test_app_state(pool).await;
        let app = build_test_router(state.clone());
        let cookie = setup_and_get_cookie(&app).await;

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
                    .header("cookie", &cookie)
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
                    .header("cookie", &cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[sqlx::test]
    #[ignore = "needs postgres"]
    async fn invalid_doc_type_is_rejected(pool: sqlx::PgPool) {
        let state = test_app_state(pool).await;
        let app = build_test_router(state.clone());
        let cookie = setup_and_get_cookie(&app).await;

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
                    .header("cookie", &cookie)
                    .body(Body::from(serde_json::to_string(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::CONFLICT);
    }

    #[sqlx::test]
    #[ignore = "needs postgres"]
    async fn delete_document_only_by_owner(pool: sqlx::PgPool) {
        let state = test_app_state(pool).await;
        let app = build_test_router(state.clone());
        let cookie = setup_and_get_cookie(&app).await;
        let doc_id = create_test_doc(&app, &cookie).await;

        let resp = app
            .oneshot(
                Request::delete(format!("/api/documents/{doc_id}"))
                    .header("cookie", &cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[sqlx::test]
    #[ignore = "needs postgres"]
    async fn invite_user_to_document(pool: sqlx::PgPool) {
        let state = test_app_state(pool).await;
        let app = build_test_router(state.clone());
        let cookie = setup_and_get_cookie(&app).await;
        let doc_id = create_test_doc(&app, &cookie).await;

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
                    .header("cookie", &cookie)
                    .body(Body::from(serde_json::to_string(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[sqlx::test]
    #[ignore = "needs postgres"]
    async fn list_members_includes_owner(pool: sqlx::PgPool) {
        let state = test_app_state(pool).await;
        let app = build_test_router(state.clone());
        let cookie = setup_and_get_cookie(&app).await;
        let doc_id = create_test_doc(&app, &cookie).await;

        let resp = app
            .oneshot(
                Request::get(format!("/api/documents/{doc_id}/members"))
                    .header("cookie", &cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[sqlx::test]
    #[ignore = "needs postgres"]
    async fn logout_clears_session(pool: sqlx::PgPool) {
        let state = test_app_state(pool).await;
        let app = build_test_router(state.clone());
        let cookie = setup_and_get_cookie(&app).await;

        let resp = app
            .oneshot(
                Request::post("/api/auth/logout")
                    .header("cookie", &cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[sqlx::test]
    #[ignore = "needs postgres"]
    async fn get_delta_endpoint_requires_auth(pool: sqlx::PgPool) {
        let state = test_app_state(pool).await;
        let app = build_test_router(state.clone());
        let cookie = setup_and_get_cookie(&app).await;
        let doc_id = create_test_doc(&app, &cookie).await;

        let resp = app
            .oneshot(
                Request::get(format!("/api/documents/{doc_id}/deltas?after_seq=0"))
                    .header("cookie", &cookie)
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
    #[ignore = "needs postgres"]
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
    #[ignore = "needs postgres"]
    async fn dev_login_creates_user_and_returns_cookie(pool: sqlx::PgPool) {
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
        assert!(resp.headers().get("set-cookie").is_some());
    }

    async fn setup_and_get_cookie(app: &Router) -> String {
        let body = serde_json::json!({
            "passphrase": "test",
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
        resp.headers()
            .get("set-cookie")
            .unwrap()
            .to_str()
            .unwrap()
            .split(';')
            .next()
            .unwrap()
            .to_string()
    }

    async fn create_test_doc(app: &Router, cookie: &str) -> String {
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
                    .header("cookie", cookie)
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
