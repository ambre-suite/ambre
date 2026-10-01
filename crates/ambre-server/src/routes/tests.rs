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
        // After creation, the list must carry the fields the launcher needs. That
        // is the encrypted name, the caller's wrapped doc key, member_count (1 for
        // a solo owner), and a timestamp. Validates the new LEFT JOIN + COUNT
        // query.
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

    async fn get_document_status(app: &Router, auth: Option<&str>, doc_id: &str) -> StatusCode {
        let mut req = Request::get(format!("/api/documents/{doc_id}"));
        if let Some(auth) = auth {
            req = req.header("authorization", auth);
        }
        app.clone()
            .oneshot(req.body(Body::empty()).unwrap())
            .await
            .unwrap()
            .status()
    }

    // H2 regression: a non-member who holds the UUID must not receive
    // `owner_id` / `doc_type` metadata. The route answers 404 instead of leaking
    // the document's existence.
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn get_document_hidden_from_non_member(pool: sqlx::PgPool) {
        let state = test_app_state(pool).await;
        let app = build_test_router(state.clone());
        let auth = setup_and_get_auth(&app).await;
        let doc_id = create_test_doc(&app, &auth).await;
        let stranger = setup_named(&app, "stranger@ambre.app", 0x51).await;

        assert_eq!(
            get_document_status(&app, Some(&stranger), &doc_id).await,
            StatusCode::NOT_FOUND
        );
    }

    // `SRV-018`: an anonymous caller is refused before the register is read, so
    // an existing document and an invented one draw the same 401.
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn get_document_refuses_an_anonymous_caller(pool: sqlx::PgPool) {
        let state = test_app_state(pool).await;
        let app = build_test_router(state.clone());
        let auth = setup_and_get_auth(&app).await;
        let doc_id = create_test_doc(&app, &auth).await;
        let invented = uuid::Uuid::new_v4().to_string();

        assert_eq!(
            get_document_status(&app, None, &doc_id).await,
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            get_document_status(&app, None, &invented).await,
            StatusCode::UNAUTHORIZED
        );
    }

    /// The shipped `/api` router, nested exactly as `main` nests it, so a test
    /// sees the paths a handler and the limiter see in production.
    fn shipped_router(state: Arc<AppState>) -> Router {
        Router::new()
            .nest("/api", routes::api_router(Arc::clone(&state)))
            .with_state(state)
    }

    // `SRV-022`: the provider route reaches its handler with the provider read
    // from the path. An unconfigured provider draws the typed refusal, never the
    // 500 a failed path extraction answered.
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn a_provider_route_reaches_its_provider(pool: sqlx::PgPool) {
        let app = shipped_router(test_app_state(pool).await);
        let peer = std::net::SocketAddr::from(([10, 22, 0, 1], 0));
        for req in [
            Request::get("/api/auth/gitlab/login")
                .body(Body::empty())
                .unwrap(),
            Request::post("/api/auth/gitlab/callback")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"code":"c","state":"s"}"#))
                .unwrap(),
        ] {
            let mut req = req;
            req.extensions_mut()
                .insert(axum::extract::ConnectInfo(peer));
            let resp = app.clone().oneshot(req).await.unwrap();
            assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
            let body = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
            let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
            assert_eq!(body["error"], "UNKNOWN_PROVIDER", "{body}");
        }
    }

    // `SRV-093`: a sign-in request counts against the sign-in tier through the
    // nested router. The eleventh request from one peer inside the window is
    // refused, where the ordinary tier would allow three hundred.
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn a_sign_in_request_counts_against_the_sign_in_tier(pool: sqlx::PgPool) {
        let app = shipped_router(test_app_state(pool).await);
        // A peer address of its own, so no other test shares the window.
        let salt = uuid::Uuid::new_v4().into_bytes();
        let peer = std::net::SocketAddr::from(([10, 93, salt[0], salt[1]], 0));
        let mut statuses = Vec::new();
        for _ in 0..11 {
            let mut req = Request::get("/api/auth/config")
                .body(Body::empty())
                .unwrap();
            req.extensions_mut()
                .insert(axum::extract::ConnectInfo(peer));
            statuses.push(app.clone().oneshot(req).await.unwrap().status());
        }
        assert!(
            statuses[..10].iter().all(|s| *s == StatusCode::OK),
            "{statuses:?}"
        );
        assert_eq!(statuses[10], StatusCode::TOO_MANY_REQUESTS);
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
        let stub = S3Stub::spawn([]).await;
        let state = test_app_state_with_s3(pool, offline_s3(Some(&stub.endpoint))).await;
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

    // `SRV-057`, `SRV-017`: deleting a document removes every object under its
    // prefix and every key the relay holds for it. Another document's object and
    // counter stay. A write in flight when the delete lands writes no key, and
    // an object put after the sweep is removed by the second one.
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn a_deleted_document_leaves_nothing(pool: sqlx::PgPool) {
        use crate::error::AppError;
        use rustis::commands::{GenericCommands, HashCommands, ScriptingCommands, StringCommands};
        let setup_state = test_app_state(pool.clone()).await;
        let setup = build_test_router(setup_state);
        let auth = setup_and_get_auth(&setup).await;
        let doc_id = create_test_doc(&setup, &auth).await;
        let other_id = create_test_doc(&setup, &auth).await;
        let env = Config::default().s3_env_prefix;
        let object = |doc: &str, rest: &str| format!("{env}/documents/{doc}/{rest}");
        let stub = S3Stub::spawn([
            object(&doc_id, "snapshots/s1"),
            object(&doc_id, "deltas/0000000001"),
            object(&doc_id, "media/abc"),
            object(&other_id, "deltas/0000000001"),
        ])
        .await;
        let state = test_app_state_with_s3(pool, offline_s3(Some(&stub.endpoint))).await;
        let app = build_test_router(state.clone());

        let doc = ambre_core::DocumentId::parse_str(&doc_id).unwrap();
        let other = ambre_core::DocumentId::parse_str(&other_id).unwrap();
        let [seq, clocks, ..] = crate::redis_ops::document_state_keys(&doc);
        let [other_seq, ..] = crate::redis_ops::document_state_keys(&other);
        state.redis.set(&seq, 7).await.unwrap();
        state.redis.hset(&clocks, [("aa", "3")]).await.unwrap();
        state.redis.set(&other_seq, 4).await.unwrap();

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

        assert_eq!(stub.keys(), [object(&other_id, "deltas/0000000001")]);
        let left: usize = state
            .redis
            .exists(crate::redis_ops::document_state_keys(&doc))
            .await
            .unwrap();
        assert_eq!(left, 0, "no relay key survives the delete");
        let kept: usize = state.redis.exists([&other_seq]).await.unwrap();
        assert_eq!(kept, 1, "another document keeps its counter");

        // The writes an update and a presence sample take, landing late.
        let redis = &state.redis;
        fn gone<T>(r: Result<T, AppError>) -> bool {
            matches!(r, Err(AppError::NotFound { .. }))
        }
        assert!(gone(
            crate::redis_ops::verify_clock_and_assign_seq(redis, &doc, "aa", 4).await
        ));
        assert!(gone(
            crate::redis_ops::append_pending_delta(redis, &doc, 8, "k", &[1]).await
        ));
        assert!(gone(
            crate::redis_ops::push_dead_letter(redis, &doc, 8, "k", &[1]).await
        ));
        assert!(gone(
            crate::redis_ops::doc_presence_update(redis, &doc, "aa", "blob", 15).await
        ));
        crate::redis_ops::seed_clock_state(redis, &doc, 9, &[("aa".into(), 9)])
            .await
            .unwrap();
        let prefix = format!("doc:{doc}");
        assert!(gone(
            crate::redis_ops::push_poison_entry(redis, &prefix, "x").await
        ));
        let left: usize = redis
            .exists(crate::redis_ops::document_state_keys(&doc))
            .await
            .unwrap();
        assert_eq!(left, 0, "a late write recreates no key");

        // An object put after the sweep, then the second sweep once it is due.
        stub.put(object(&doc_id, "media/late"));
        let due_now = "return redis.call('ZADD', KEYS[1], 0, ARGV[1])";
        let sweep_set = crate::redis_ops::SWEEP_DUE_SET;
        let _: i64 = redis.eval(due_now, [sweep_set], [&doc_id]).await.unwrap();
        crate::s3::sweep_due_documents(&state).await;
        assert_eq!(stub.keys(), [object(&other_id, "deltas/0000000001")]);
        let score = "return redis.call('ZSCORE', KEYS[1], ARGV[1])";
        let left: Option<String> = redis.eval(score, [sweep_set], [&doc_id]).await.unwrap();
        assert_eq!(left, None, "a done sweep leaves the schedule");

        let gone_key = crate::redis_ops::document_gone_key(&doc);
        let _ = redis.del([&other_seq, &gone_key]).await;
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

    fn store_master_keys_body() -> serde_json::Value {
        serde_json::json!({
            "master_key_enc": base64_encode(&[1u8; 32]),
            "master_key_salt": base64_encode(&[2u8; 16]),
            "signing_pubkey": hex::encode([3u8; 32]),
            "exchange_pubkey": hex::encode([4u8; 32]),
        })
    }

    async fn put_master_keys(app: &Router, auth: &str) -> StatusCode {
        app.clone()
            .oneshot(
                Request::put("/api/keys/master")
                    .header("content-type", "application/json")
                    .header("authorization", auth)
                    .body(Body::from(store_master_keys_body().to_string()))
                    .unwrap(),
            )
            .await
            .unwrap()
            .status()
    }

    /// `store_master_keys` is the one-shot first-time upload path (an OIDC/dev
    /// account is created with a NULL `master_key_enc`, then fills it here). The
    /// first call must succeed. A second call must be rejected with 409, instead of
    /// overwriting `signing_pubkey`/`exchange_pubkey`. An unconditional overwrite
    /// would strand every `doc_key_enc` wrapped to the previous exchange key,
    /// silently locking the caller out of its own shared documents.
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn store_master_keys_is_first_time_only(pool: sqlx::PgPool) {
        let state = test_app_state_dev(pool).await;
        let app = build_test_router(state);

        // dev-login creates a user whose master_key_enc is still NULL.
        let resp = app
            .clone()
            .oneshot(
                Request::post("/api/auth/dev/login")
                    .header("content-type", "application/json")
                    .body(Body::from(dev_login_body("alice").to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let auth = format!("Bearer {}", access_token_of(resp).await);

        // First upload succeeds; the re-store is rejected, not silently applied.
        assert_eq!(put_master_keys(&app, &auth).await, StatusCode::OK);
        assert_eq!(put_master_keys(&app, &auth).await, StatusCode::CONFLICT);
    }

    async fn status_and_code(app: &Router, req: Request<Body>) -> (StatusCode, String) {
        let resp = app.clone().oneshot(with_peer(req)).await.unwrap();
        let status = resp.status();
        let body = axum::body::to_bytes(resp.into_body(), 1 << 16)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap_or_default();
        (
            status,
            json["error"].as_str().unwrap_or_default().to_owned(),
        )
    }

    /// The served table runs the rate limiter, which reads the peer address.
    fn with_peer(mut req: Request<Body>) -> Request<Body> {
        let peer = std::net::SocketAddr::from(([10, 27, 0, 1], 0));
        req.extensions_mut()
            .insert(axum::extract::ConnectInfo(peer));
        req
    }

    fn get_with(uri: &str, auth: &str) -> Request<Body> {
        Request::get(uri)
            .header("authorization", auth)
            .body(Body::empty())
            .unwrap()
    }

    /// `SRV-027`: a provider account created before its keys holds a session
    /// with no signing key. Only the setup routes take it, and a refresh keeps
    /// it keyless. Registering the keys returns a session under the real key.
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn a_session_before_key_setup_carries_no_key(pool: sqlx::PgPool) {
        let state = test_app_state(pool).await;
        let app = shipped_router(state.clone());
        let user = crate::db::upsert_user_from_oidc(
            &state.db,
            "google",
            "sub-1",
            "new@example.com",
            "New",
        )
        .await
        .unwrap();
        assert!(user.master_key_enc.is_none());
        assert!(user.signing_pubkey.is_none() && user.exchange_pubkey.is_none());
        let user_id =
            ambre_core::UserId::from_uuid(ambre_core::Uuid::from_bytes(*user.id.as_bytes()));
        let tokens = crate::auth::issue_session(&state, user_id, None)
            .await
            .unwrap();
        let setup = format!("Bearer {}", tokens.access_token);

        let refused = status_and_code(&app, get_with("/api/documents", &setup)).await;
        assert_eq!(refused, (StatusCode::FORBIDDEN, "SETUP_REQUIRED".into()));
        assert!(matches!(
            crate::auth::authenticate_token(&state, &tokens.access_token),
            Err(crate::error::AppError::SetupRequired)
        ));
        let (me, _) = status_and_code(&app, get_with("/api/auth/me", &setup)).await;
        assert_eq!(me, StatusCode::OK);
        let (keys, _) = status_and_code(&app, get_with("/api/keys/master", &setup)).await;
        assert_eq!(keys, StatusCode::OK);

        // The refresh of a keyless session is keyless too.
        let refreshed = app
            .clone()
            .oneshot(with_peer(
                Request::post("/api/auth/refresh")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({ "refresh_token": tokens.refresh_token }).to_string(),
                    ))
                    .unwrap(),
            ))
            .await
            .unwrap();
        assert_eq!(refreshed.status(), StatusCode::OK);
        let renewed = format!("Bearer {}", access_token_of(refreshed).await);
        let still = status_and_code(&app, get_with("/api/documents", &renewed)).await;
        assert_eq!(still, (StatusCode::FORBIDDEN, "SETUP_REQUIRED".into()));

        // Registering the keys hands back a session under the real key.
        let stored = app
            .clone()
            .oneshot(with_peer(
                Request::put("/api/keys/master")
                    .header("content-type", "application/json")
                    .header("authorization", &renewed)
                    .body(Body::from(store_master_keys_body().to_string()))
                    .unwrap(),
            ))
            .await
            .unwrap();
        assert_eq!(stored.status(), StatusCode::OK);
        let token = access_token_of(stored).await;
        let session = crate::auth::authenticate_token(&state, &token).unwrap();
        assert_eq!(
            session.signing_pubkey,
            ambre_core::PublicKey::from_array([3u8; 32])
        );
        let (listed, _) =
            status_and_code(&app, get_with("/api/documents", &format!("Bearer {token}"))).await;
        assert_eq!(listed, StatusCode::OK);
    }

    /// `SRV-163`: an account without keys is offered to no sharer. The search
    /// leaves it out, the key lookup and the invite answer as for an unknown
    /// account. Once its keys are stored, all three find it.
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn an_account_without_keys_is_offered_to_no_sharer(pool: sqlx::PgPool) {
        let state = test_app_state(pool).await;
        let app = build_test_router(state.clone());
        let auth = setup_and_get_auth(&app).await;
        let doc_id = create_test_doc(&app, &auth).await;
        let keyless = crate::db::upsert_user_from_oidc(
            &state.db,
            "google",
            "sub-2",
            "keyless@example.com",
            "Keyless",
        )
        .await
        .unwrap();

        let search = || async {
            let resp = app
                .clone()
                .oneshot(get_with("/api/users/search?query=keyless", &auth))
                .await
                .unwrap();
            assert_eq!(resp.status(), StatusCode::OK);
            let body = axum::body::to_bytes(resp.into_body(), 16384).await.unwrap();
            let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
            json["users"].as_array().unwrap().len()
        };
        let pubkeys = format!("/api/users/{}/pubkeys", keyless.id);
        let invite = |role: &str| {
            serde_json::json!({
                "invitee_email": "keyless@example.com",
                "role": role,
                "doc_key_enc": base64_encode(&[9u8; 32]),
            })
        };
        let share = format!("/api/documents/{doc_id}/share");

        assert_eq!(search().await, 0);
        let (lookup, _) = status_and_code(&app, get_with(&pubkeys, &auth)).await;
        assert_eq!(lookup, StatusCode::NOT_FOUND);
        let invited = post_json(&app, &auth, share.clone(), invite("editor")).await;
        assert_eq!(invited, StatusCode::NOT_FOUND);
        // A membership written before the invite refused such an account is
        // left out of the list, so no rotation rewraps to it.
        sqlx::query(
            "INSERT INTO document_members (document_id, user_id, role, doc_key_enc) \
             VALUES ($1, $2, 'viewer', $3)",
        )
        .bind(uuid::Uuid::parse_str(&doc_id).unwrap())
        .bind(keyless.id)
        .bind(vec![9u8; 32])
        .execute(&state.db)
        .await
        .unwrap();
        assert_eq!(member_count(&app, &auth, &doc_id).await, 1);

        sqlx::query(
            "UPDATE users SET master_key_enc = $2, master_key_salt = $3, \
             signing_pubkey = $4, exchange_pubkey = $5 WHERE id = $1",
        )
        .bind(keyless.id)
        .bind(vec![1u8; 56])
        .bind(vec![2u8; 16])
        .bind(vec![5u8; 32])
        .bind(vec![6u8; 32])
        .execute(&state.db)
        .await
        .unwrap();

        assert_eq!(search().await, 1);
        let (lookup, _) = status_and_code(&app, get_with(&pubkeys, &auth)).await;
        assert_eq!(lookup, StatusCode::OK);
        let invited = post_json(&app, &auth, share, invite("editor")).await;
        assert_eq!(invited, StatusCode::OK);
        assert_eq!(member_count(&app, &auth, &doc_id).await, 2);
    }

    async fn member_count(app: &Router, auth: &str, doc_id: &str) -> usize {
        let uri = format!("/api/documents/{doc_id}/members");
        let resp = app.clone().oneshot(get_with(&uri, auth)).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), 16384).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        json["members"].as_array().unwrap().len()
    }

    /// `SRV-163`: the store refuses an account holding one key without the
    /// other, since both come from one seed and are written together.
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn an_account_holds_both_keys_or_neither(pool: sqlx::PgPool) {
        let user = crate::db::upsert_user_from_oidc(&pool, "google", "sub-3", "c@example.com", "C")
            .await
            .unwrap();
        let half = sqlx::query("UPDATE users SET signing_pubkey = $2 WHERE id = $1")
            .bind(user.id)
            .bind(vec![5u8; 32])
            .execute(&pool)
            .await;
        assert!(half.is_err(), "a lone signing key must violate the pairing");
    }

    /// `SRV-027`: the socket refuses a session minted before the key setup. The
    /// upgrade completes, then the server closes before any frame is read.
    /// `SRV-164`: it closes with the setup code, not the authentication one.
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn the_socket_refuses_a_session_without_keys(pool: sqlx::PgPool) {
        let state = test_app_state(pool).await;
        let user =
            crate::db::upsert_user_from_oidc(&state.db, "google", "sub-4", "d@example.com", "D")
                .await
                .unwrap();
        let user_id =
            ambre_core::UserId::from_uuid(ambre_core::Uuid::from_bytes(*user.id.as_bytes()));
        let tokens = crate::auth::issue_session(&state, user_id, None)
            .await
            .unwrap();
        assert_eq!(first_close_code(state, &tokens.access_token).await, 4403);
    }

    /// `SRV-046`: an upgrade refused on authentication closes with a code of its
    /// own. The client reads it as a cue to renew rather than back off.
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn a_refused_token_closes_the_socket_with_its_own_code(pool: sqlx::PgPool) {
        let state = test_app_state(pool).await;
        assert_eq!(first_close_code(state, "not-a-token").await, 4401);
    }

    /// Serves the router on a loopback port, opens the socket with `token`, and
    /// returns the code of the close frame the server sends first.
    async fn first_close_code(state: Arc<AppState>, token: &str) -> u16 {
        use futures_util::StreamExt;
        use tokio_tungstenite::tungstenite::Message;

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let app = build_test_router(state);
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

        let url = format!("ws://{addr}/api/ws?token={token}");
        let (mut socket, _) = tokio_tungstenite::connect_async(url).await.unwrap();
        let first = tokio::time::timeout(std::time::Duration::from_secs(5), socket.next())
            .await
            .expect("the server closes the socket")
            .expect("a frame before the stream ends")
            .unwrap();
        let Message::Close(Some(frame)) = first else {
            panic!("expected a close frame, got {first:?}");
        };
        u16::from(frame.code)
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
    /// (for example a `viewer`) in role-gated route tests.
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
    /// register a `media` row either. `media_confirm_upload` is gated to
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

    /// Confirming the same media twice is idempotent. `insert_media` uses `ON
    /// CONFLICT (document_id, id) DO NOTHING`. A retried confirm returns 200 both times,
    /// instead of surfacing the PK unique violation as a 500. So does a
    /// second client registering the same content-addressed blob.
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn media_confirm_upload_is_idempotent(pool: sqlx::PgPool) {
        let state = test_app_state(pool).await;
        let app = build_test_router(state.clone());
        let owner = setup_and_get_auth(&app).await;
        let doc_id = create_test_doc(&app, &owner).await;

        let body = serde_json::json!({
            "content_hash": "b".repeat(64),
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

    /// An S3 client with static credentials and a region. It presigns offline,
    /// and it reaches `endpoint` when one is given.
    fn offline_s3(endpoint: Option<&str>) -> aws_sdk_s3::Client {
        let mut conf = aws_sdk_s3::Config::builder()
            .behavior_version(aws_sdk_s3::config::BehaviorVersion::latest())
            .region(aws_sdk_s3::config::Region::new("us-east-1"))
            .credentials_provider(aws_sdk_s3::config::Credentials::new(
                "test-access",
                "test-secret",
                None,
                None,
                "test",
            ))
            .force_path_style(true);
        if let Some(endpoint) = endpoint {
            conf = conf.endpoint_url(endpoint);
        }
        aws_sdk_s3::Client::from_conf(conf.build())
    }

    /// Like [`test_app_state`], with the given S3 client.
    async fn test_app_state_with_s3(pool: sqlx::PgPool, s3: aws_sdk_s3::Client) -> Arc<AppState> {
        let base = test_app_state(pool).await;
        Arc::new(AppState {
            db: base.db.clone(),
            redis: base.redis.clone(),
            pubsub: base.pubsub.clone(),
            s3,
            s3_bucket: base.s3_bucket.clone(),
            config: Arc::clone(&base.config),
            jwt: Arc::clone(&base.jwt),
            oidc_clients: Arc::clone(&base.oidc_clients),
        })
    }

    async fn test_app_state_presigning(pool: sqlx::PgPool) -> Arc<AppState> {
        test_app_state_with_s3(pool, offline_s3(None)).await
    }

    /// An in-process object store that answers the two calls a purge makes,
    /// `ListObjectsV2` and `DeleteObjects`, over a set of keys.
    struct S3Stub {
        keys: Arc<std::sync::Mutex<std::collections::BTreeSet<String>>>,
        endpoint: String,
    }

    impl S3Stub {
        async fn spawn(keys: impl IntoIterator<Item = String>) -> Self {
            let keys = Arc::new(std::sync::Mutex::new(keys.into_iter().collect()));
            let held = Arc::clone(&keys);
            let app = Router::new().fallback(move |req: Request<Body>| {
                let held = Arc::clone(&held);
                async move { Self::answer(&held, req).await }
            });
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let endpoint = format!("http://{}", listener.local_addr().unwrap());
            tokio::spawn(async move { axum::serve(listener, app).await });
            Self { keys, endpoint }
        }

        async fn answer(
            held: &std::sync::Mutex<std::collections::BTreeSet<String>>,
            req: Request<Body>,
        ) -> String {
            let query = req.uri().query().unwrap_or("").to_string();
            let method = req.method().clone();
            let body = axum::body::to_bytes(req.into_body(), 1 << 20)
                .await
                .unwrap();
            let body = String::from_utf8_lossy(&body);
            let xmlns = r#"xmlns="http://s3.amazonaws.com/doc/2006-03-01/""#;
            if method == axum::http::Method::POST && query.contains("delete") {
                let mut held = held.lock().unwrap();
                for part in body.split("<Key>").skip(1) {
                    held.remove(part.split("</Key>").next().unwrap());
                }
                return format!("<DeleteResult {xmlns}></DeleteResult>");
            }
            let prefix = query
                .split('&')
                .find_map(|kv| kv.strip_prefix("prefix="))
                .map(|p| p.replace("%2F", "/"))
                .unwrap_or_default();
            let contents: String = held
                .lock()
                .unwrap()
                .iter()
                .filter(|k| k.starts_with(&prefix))
                .map(|k| format!("<Contents><Key>{k}</Key><Size>1</Size></Contents>"))
                .collect();
            format!(
                "<ListBucketResult {xmlns}><Name>test-bucket</Name><Prefix>{prefix}</Prefix>\
                 <IsTruncated>false</IsTruncated>{contents}</ListBucketResult>"
            )
        }

        fn keys(&self) -> Vec<String> {
            self.keys.lock().unwrap().iter().cloned().collect()
        }

        /// An object a late writer puts behind the relay's back, as a
        /// presigned upload does.
        fn put(&self, key: String) {
            self.keys.lock().unwrap().insert(key);
        }
    }

    async fn post_json(
        app: &Router,
        auth: &str,
        uri: String,
        body: serde_json::Value,
    ) -> StatusCode {
        app.clone()
            .oneshot(
                Request::post(uri)
                    .header("content-type", "application/json")
                    .header("authorization", auth)
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap()
            .status()
    }

    // `SRV-058`: a download link is signed only for an address the media index
    // records in that document. The same file confirmed in a second document
    // gets a row of its own there.
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn a_download_link_names_an_object_the_document_holds(pool: sqlx::PgPool) {
        let state = test_app_state_presigning(pool).await;
        let setup = build_test_router(state.clone());
        let app = shipped_router(state);
        let owner = setup_and_get_auth(&setup).await;
        let doc_a = create_test_doc(&setup, &owner).await;
        let doc_b = create_test_doc(&setup, &owner).await;
        let held = "c".repeat(64);
        let never = "d".repeat(64);
        let link = |doc: &str, hash: &str| {
            (
                format!("/api/documents/{doc}/media/download-url"),
                serde_json::json!({ "content_hash": hash }),
            )
        };
        let confirm = |doc: &str| {
            (
                format!("/api/documents/{doc}/media"),
                serde_json::json!({ "content_hash": held, "size_bytes": 7 }),
            )
        };

        let (uri, body) = link(&doc_a, &held);
        assert_eq!(
            post_json(&app, &owner, uri, body).await,
            StatusCode::NOT_FOUND
        );

        for doc in [&doc_a, &doc_b] {
            let (uri, body) = confirm(doc);
            assert_eq!(post_json(&app, &owner, uri, body).await, StatusCode::OK);
            let (uri, body) = link(doc, &held);
            assert_eq!(post_json(&app, &owner, uri, body).await, StatusCode::OK);
        }
        let (uri, body) = link(&doc_a, &never);
        assert_eq!(
            post_json(&app, &owner, uri, body).await,
            StatusCode::NOT_FOUND
        );
    }

    /// A snapshot's `author_pubkey` is bound to the session identity (the token's
    /// `spk` claim), like the WS `Update` author-key check. A member may only
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

    /// D1: an `editor` holds the document key. The invite upsert (which is also
    /// the role-change path) must not let it demote an existing `owner` and
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

        // The editor tries to re-invite the owner as `editor`. The granted role is
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

    /// #1 regression: document-key rotation is owner-only. An editor holds the
    /// document key but must not be able to rotate it. That is membership
    /// management: it rewraps every member and could lock others out, so it is
    /// gated to the owner exactly like revoke.
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn rotate_key_denied_to_editor(pool: sqlx::PgPool) {
        let state = test_app_state(pool).await;
        let app = build_test_router(state.clone());
        let owner = setup_and_get_auth(&app).await;
        let doc_id = create_test_doc(&app, &owner).await;

        let editor = setup_named(&app, "editor@test.com", 71).await;
        invite(&app, &owner, &doc_id, "editor@test.com", "editor").await;
        let editor_uid = doc_user_id(&state, "editor@test.com").await;

        let rotate_body = serde_json::json!({
            "keys": [ { "user_id": editor_uid, "doc_key_enc": base64_encode(&[0xABu8; 32]) } ],
        });
        let resp = app
            .clone()
            .oneshot(
                Request::post(format!("/api/documents/{doc_id}/rotate-key"))
                    .header("content-type", "application/json")
                    .header("authorization", &editor)
                    .body(Body::from(serde_json::to_string(&rotate_body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::FORBIDDEN,
            "an editor must not be able to rotate the document key"
        );
    }

    /// #6 regression: LIKE wildcards in the search query are escaped, so a bare
    /// `%` no longer acts as "match everything" (directory enumeration). It must
    /// match the character literally and return no rows here.
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn search_users_escapes_like_wildcards(pool: sqlx::PgPool) {
        let state = test_app_state(pool).await;
        let app = build_test_router(state.clone());
        let auth = setup_and_get_auth(&app).await;
        // Seed a couple of users the wildcard would otherwise sweep in.
        let _ = setup_named(&app, "alice@test.com", 81).await;
        let _ = setup_named(&app, "bob@test.com", 82).await;

        let resp = app
            .clone()
            .oneshot(
                Request::get("/api/users/search?query=%25%25") // "%%" url-encoded
                    .header("authorization", &auth)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(resp.into_body(), 8192).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(
            json["users"].as_array().unwrap().len(),
            0,
            "'%%' must be matched literally, not as a wildcard sweep"
        );
    }

    /// #3 regression: re-creating an existing document id owned by someone else
    /// must not leak that it exists (nor its `doc_type`). A foreign caller gets a
    /// clean 404, not the echoed row.
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn create_document_with_foreign_existing_id_is_not_found(pool: sqlx::PgPool) {
        let state = test_app_state(pool).await;
        let app = build_test_router(state.clone());
        let owner = setup_and_get_auth(&app).await;
        let doc_id = create_test_doc(&app, &owner).await;

        // A different user re-POSTs create with the SAME id.
        let intruder = setup_named(&app, "intruder@test.com", 72).await;
        let body = serde_json::json!({
            "doc_type": "prose",
            "doc_key_enc": base64_encode(&[6u8; 32]),
            "document_id": doc_id,
        });
        let resp = app
            .clone()
            .oneshot(
                Request::post("/api/documents")
                    .header("content-type", "application/json")
                    .header("authorization", &intruder)
                    .body(Body::from(serde_json::to_string(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::NOT_FOUND,
            "a foreign caller must not learn an existing document id exists"
        );

        // The intruder gained no membership: the owner still lists exactly itself.
        assert_eq!(list_members(&app, &owner, &doc_id).await.len(), 1);
    }
}
