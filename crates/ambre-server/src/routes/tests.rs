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
    use ambre_core::id::{Document, Id, Snapshot, User};
    use ambre_protocol::ws::{CloseCode, WsClientMsg, WsServerMsg, encode_client_msg};

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
                "/documents/{id}/deltas",
                axum::routing::get(routes::get_deltas),
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
                "/documents/{id}/revoke",
                axum::routing::post(routes::revoke_and_rotate),
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
                "/documents/{id}/key-links",
                axum::routing::get(routes::list_key_links),
            )
            .route(
                "/documents/{id}/media",
                axum::routing::post(routes::media_confirm_upload),
            )
            .route(
                "/documents/{id}/snapshot",
                axum::routing::get(routes::get_snapshot_metadata).post(routes::create_snapshot),
            )
            .route(
                "/documents/{id}/snapshots/{snapshot_id}/confirm",
                axum::routing::post(routes::confirm_snapshot),
            )
            .route(
                "/documents/{id}/snapshots/{snapshot_id}/report",
                axum::routing::post(routes::report_snapshot),
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
            "master_key_enc": base64_encode(&[1u8; 72]),
            "master_key_salt": hex::encode([2u8; 32]),
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
            "master_key_enc": base64_encode(&[9u8; 72]),
            "master_key_salt": hex::encode([8u8; 32]),
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
            "doc_key_enc": base64_encode(&[5u8; 72]),
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

    // `SRV-172`: the provider route reaches its handler with the provider read
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

    // `SRV-022`: a provider's return, through the shipped router, exchanges its
    // code against the provider and signs the person in. The session it opens
    // answers `/auth/me` as that person, and the same return is not taken twice.
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn a_provider_return_signs_the_person_in(pool: sqlx::PgPool) {
        use crate::oidc::stub::{CLIENT_ID, EMAIL, NAME, StubProvider};
        let stub = StubProvider::spawn().await;
        let clients = crate::oidc::start_oidc_clients(Some(&crate::oidc::OidcRegistration {
            issuer: openidconnect::IssuerUrl::new(stub.issuer.clone()).unwrap(),
            client_id: openidconnect::ClientId::new(CLIENT_ID.to_owned()),
            client_secret: crate::oidc::OidcSecret::new("s".to_owned()),
            redirect_uri: openidconnect::RedirectUrl::new("https://app/cb".to_owned()).unwrap(),
        }))
        .await
        .unwrap();
        let app = shipped_router(test_app_state_with(pool, clients).await);
        let salt = uuid::Uuid::new_v4().into_bytes();
        let peer = std::net::SocketAddr::from(([10, 22, salt[0], salt[1]], 0));
        let send = |mut req: Request<Body>| {
            req.extensions_mut()
                .insert(axum::extract::ConnectInfo(peer));
            let app = app.clone();
            async move {
                let resp = app.oneshot(req).await.unwrap();
                let status = resp.status();
                let body = axum::body::to_bytes(resp.into_body(), 65536).await.unwrap();
                (
                    status,
                    serde_json::from_slice::<serde_json::Value>(&body).unwrap_or_default(),
                )
            }
        };

        let (status, login) = send(
            Request::get("/api/auth/google/login")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{login}");
        let code = stub.authorize(login["url"].as_str().unwrap());
        let callback = serde_json::json!({ "code": code, "state": login["state"] }).to_string();
        let post = || {
            Request::post("/api/auth/google/callback")
                .header("content-type", "application/json")
                .body(Body::from(callback.clone()))
                .unwrap()
        };

        let (status, signed_in) = send(post()).await;
        assert_eq!(status, StatusCode::OK, "{signed_in}");
        assert_eq!(signed_in["email"], EMAIL);
        assert_eq!(signed_in["display_name"], NAME);
        assert_eq!(signed_in["is_new_user"], true);

        let token = signed_in["access_token"].as_str().unwrap();
        let (status, me) = send(
            Request::get("/api/auth/me")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{me}");
        assert_eq!(me["email"], EMAIL);
        assert_eq!(me["user_id"], signed_in["user_id"]);

        let (status, replayed) = send(post()).await;
        assert_ne!(status, StatusCode::OK, "{replayed}");
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
            "doc_key_enc": base64_encode(&[5u8; 72]),
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
        use rustis::commands::{GenericCommands, HashCommands, StringCommands};
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

        let doc = Id::<Document>::parse_str(&doc_id).unwrap();
        let other = Id::<Document>::parse_str(&other_id).unwrap();
        let keys = crate::redis_keys::DocumentStateKeys::of(&doc);
        let other_keys = crate::redis_keys::DocumentStateKeys::of(&other);
        let [seq, clocks, ..] = keys.texts();
        let [other_seq, ..] = other_keys.texts();
        state.redis.set(seq, 7).await.unwrap();
        let aa = ambre_core::FixedBytes::<ambre_core::PublicKey>::from_array([0xaa; 32]);
        state
            .redis
            .hset(clocks, [(aa.to_hex(), "3")])
            .await
            .unwrap();
        state.redis.set(other_seq, 4).await.unwrap();
        crate::redis_ops::doc_session_open(&state.redis, &doc, &aa)
            .await
            .unwrap();

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
            .exists(crate::redis_keys::DocumentStateKeys::of(&doc).texts())
            .await
            .unwrap();
        assert_eq!(left, 0, "no relay key survives the delete");
        let kept: usize = state.redis.exists([other_seq]).await.unwrap();
        assert_eq!(kept, 1, "another document keeps its counter");

        // The writes an update and a presence sample take, landing late.
        let redis = &state.redis;
        fn gone<T>(r: Result<T, AppError>) -> bool {
            matches!(r, Err(AppError::NotFound { .. }))
        }
        assert!(gone(
            crate::redis_ops::verify_clock_and_assign_seq(
                redis,
                &doc,
                &aa,
                ambre_protocol::messages::Clock(4)
            )
            .await
        ));
        let frame = crate::payload::Payload::new(vec![1]);
        assert!(gone(
            crate::redis_ops::append_pending_delta(
                redis,
                &doc,
                ambre_core::Seq(8),
                &ambre_core::ObjectKey::from("k"),
                &frame
            )
            .await
        ));
        assert!(gone(
            crate::redis_ops::push_dead_letter(
                redis,
                &doc,
                ambre_core::Seq(8),
                &ambre_core::ObjectKey::from("k"),
                &frame
            )
            .await
        ));
        let sample = ambre_protocol::messages::Ciphertext(b"blob".to_vec());
        assert!(gone(
            crate::redis_ops::doc_presence_update(
                redis,
                &doc,
                &aa,
                &sample,
                ambre_core::Seconds(15)
            )
            .await
        ));
        crate::redis_ops::seed_clock_state(
            redis,
            &doc,
            ambre_core::Seq(9),
            &[(aa, ambre_protocol::messages::Clock(9))],
        )
        .await
        .unwrap();
        crate::redis_ops::doc_session_open(redis, &doc, &aa)
            .await
            .unwrap();
        assert!(gone(
            crate::redis_ops::push_poison_entry(redis, &doc, &frame).await
        ));
        let left: usize = redis
            .exists(crate::redis_keys::DocumentStateKeys::of(&doc).texts())
            .await
            .unwrap();
        assert_eq!(left, 0, "a late write recreates no key");

        // An object put after the sweep, then the second sweep once it is due.
        stub.put(object(&doc_id, "media/late"));
        sqlx::query("UPDATE document_deletions SET sweep_due = 0")
            .execute(&state.db)
            .await
            .unwrap();
        crate::deletion::finish_deletions(&state).await;
        assert_eq!(stub.keys(), [object(&other_id, "deltas/0000000001")]);
        let marks: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM document_deletions")
            .fetch_one(&state.db)
            .await
            .unwrap();
        assert_eq!(marks, 0, "a done sweep drops the mark");

        let gone_key = crate::redis_keys::GoneKey::of(&doc);
        let _ = redis.del([other_seq, gone_key.as_str()]).await;
    }

    // `SRV-165`: a delete the object store stops keeps its mark, and the
    // flusher's pass finishes it. The pass sets the tombstone again after a
    // reset lost it, and drops the mark once the second sweep has run.
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn a_delete_interrupted_is_finished_by_the_pass(pool: sqlx::PgPool) {
        use rustis::commands::{GenericCommands, StringCommands};
        let setup_state = test_app_state(pool.clone()).await;
        let setup = build_test_router(setup_state);
        let auth = setup_and_get_auth(&setup).await;
        let doc_id = create_test_doc(&setup, &auth).await;
        let doc = Id::<Document>::parse_str(&doc_id).unwrap();
        let env = Config::default().s3_env_prefix;
        let object = |rest: &str| format!("{env}/documents/{doc_id}/{rest}");
        let marks = |pool: sqlx::PgPool| async move {
            let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM document_deletions")
                .fetch_one(&pool)
                .await
                .unwrap();
            n
        };

        // The object store is down: the delete fails past its mark.
        let down =
            test_app_state_with_s3(pool.clone(), offline_s3(Some("http://127.0.0.1:1"))).await;
        let keys = crate::redis_keys::DocumentStateKeys::of(&doc);
        let [seq, ..] = keys.texts();
        down.redis.set(seq, 7).await.unwrap();
        let resp = build_test_router(down.clone())
            .oneshot(
                Request::delete(format!("/api/documents/{doc_id}"))
                    .header("authorization", &auth)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_ne!(resp.status(), StatusCode::OK);
        assert_eq!(
            marks(pool.clone()).await,
            1,
            "the mark outlives the failure"
        );
        assert!(
            crate::db::get_document(&down.db, &doc)
                .await
                .unwrap()
                .is_some()
        );

        // A reset of the ephemeral store loses the tombstone.
        let gone_key = crate::redis_keys::GoneKey::of(&doc);
        let _: usize = down.redis.del([gone_key.as_str()]).await.unwrap();

        // The store is back, and the pass finishes the delete.
        let stub = S3Stub::spawn([object("snapshots/s1"), object("media/abc")]).await;
        let state = test_app_state_with_s3(pool.clone(), offline_s3(Some(&stub.endpoint))).await;
        crate::deletion::finish_deletions(&state).await;
        assert!(stub.keys().is_empty(), "the objects are gone");
        assert!(
            crate::db::get_document(&state.db, &doc)
                .await
                .unwrap()
                .is_none()
        );
        let left: usize = state
            .redis
            .exists(crate::redis_keys::DocumentStateKeys::of(&doc).texts())
            .await
            .unwrap();
        assert_eq!(left, 0, "no relay key survives the pass");
        let fenced: usize = state.redis.exists([gone_key.as_str()]).await.unwrap();
        assert_eq!(fenced, 1, "the pass sets the tombstone again");
        assert_eq!(
            marks(pool.clone()).await,
            1,
            "the second sweep is not due yet"
        );

        // A late object, then the pass once the sweep is due.
        stub.put(object("media/late"));
        sqlx::query("UPDATE document_deletions SET sweep_due = 0")
            .execute(&state.db)
            .await
            .unwrap();
        crate::deletion::finish_deletions(&state).await;
        assert!(
            stub.keys().is_empty(),
            "the second sweep removes the late object"
        );
        assert_eq!(marks(pool).await, 0, "a done delete drops its mark");

        let _: usize = state.redis.del([gone_key.as_str()]).await.unwrap();
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
            "doc_key_enc": base64_encode(&[9u8; 72]),
            "key_id": key_id_hex(1),
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

    // `SRV-224`: an invite names its invitee by id as it does by email. One
    // naming both, or neither, is refused and adds no row.
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn an_invite_names_its_invitee_by_id_or_by_email_once(pool: sqlx::PgPool) {
        let state = test_app_state(pool).await;
        let app = build_test_router(state.clone());
        let auth = setup_and_get_auth(&app).await;
        let doc_id = create_test_doc(&app, &auth).await;
        let share = format!("/api/documents/{doc_id}/share");
        let email = "by-id@test.com";
        let invitee = crate::test_helpers::create_test_user(&state.db, email).await;
        let body = |email: Option<&str>, id: Option<String>| {
            let mut body = serde_json::json!({
                "role": "editor",
                "doc_key_enc": base64_encode(&[9u8; 72]),
                "key_id": key_id_hex(1),
            });
            if let Some(email) = email {
                body["invitee_email"] = email.into();
            }
            if let Some(id) = id {
                body["invitee_id"] = id.into();
            }
            body
        };
        let member = || async {
            let document =
                Id::<Document>::from_bytes(*uuid::Uuid::parse_str(&doc_id).unwrap().as_bytes());
            crate::db::get_document_member(&state.db, &document, &invitee)
                .await
                .unwrap()
        };

        let both = body(Some(email), Some(invitee.to_string()));
        assert_eq!(
            post_json(&app, &auth, share.clone(), both).await,
            StatusCode::CONFLICT
        );
        let neither = body(None, None);
        assert_eq!(
            post_json(&app, &auth, share.clone(), neither).await,
            StatusCode::CONFLICT
        );
        assert!(member().await.is_none(), "a refused invite adds no row");

        let unknown = body(None, Some(Id::<User>::mint().to_string()));
        assert_eq!(
            post_json(&app, &auth, share.clone(), unknown).await,
            StatusCode::NOT_FOUND
        );
        let by_id = body(None, Some(invitee.to_string()));
        assert_eq!(post_json(&app, &auth, share, by_id).await, StatusCode::OK);
        assert!(
            member().await.is_some(),
            "the invite by id adds the invitee"
        );
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

    // `SRV-005`: the member list of an invented document and of a document the
    // caller does not belong to draw the same 404. The client opens a new
    // document on that 404, so a 403 here leaves every new document loading.
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn members_of_an_absent_or_foreign_document_are_not_found(pool: sqlx::PgPool) {
        let state = test_app_state(pool).await;
        let app = build_test_router(state.clone());
        let auth = setup_and_get_auth(&app).await;
        let doc_id = create_test_doc(&app, &auth).await;
        let stranger = setup_named(&app, "stranger@ambre.app", 0x51).await;
        let invented = uuid::Uuid::new_v4().to_string();

        for (who, id) in [(&auth, &invented), (&stranger, &doc_id)] {
            let resp = app
                .clone()
                .oneshot(
                    Request::get(format!("/api/documents/{id}/members"))
                        .header("authorization", who.as_str())
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(resp.status(), StatusCode::NOT_FOUND, "{id}");
        }
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
            "doc_key_enc": base64_encode(&[9u8; 72]),
            "key_id": key_id_hex(1),
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
        assert_eq!(members[0]["doc_key_enc"], base64_encode(&[5u8; 72]));

        // Rotate: re-wrap the document key for the remaining owner.
        let new_key = base64_encode(&[0xABu8; 72]);
        let rotate_body = serde_json::json!({
            "keys": [ { "user_id": owner_id, "doc_key_enc": new_key } ],
            "members": [ owner_id ],
            "link": link_text(&doc_id, 2, 1),
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

    /// An account registered under the signing key `seed` derives, with its
    /// id and its `Authorization` header.
    async fn setup_signed(
        app: &Router,
        state: &AppState,
        email: &str,
        seed: u8,
    ) -> (String, ed25519_dalek::SigningKey, Id<User>) {
        let key = ed25519_dalek::SigningKey::from_bytes(&[seed; 32]);
        let body = serde_json::json!({
            "email": email,
            "display_name": "Signer",
            "master_key_enc": base64_encode(&[1u8; 72]),
            "master_key_salt": hex::encode([2u8; 32]),
            "signing_pubkey": hex::encode(key.verifying_key().to_bytes()),
            "exchange_pubkey": hex::encode([seed; 32]),
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
        let auth = format!("Bearer {}", access_token_of(resp).await);
        let id = Id::<User>::parse_str(&doc_user_id(state, email).await).unwrap();
        (auth, key, id)
    }

    /// `entry` signed by `key`, in the form a request carries it.
    fn sealed(
        mut entry: ambre_protocol::MembershipEntry,
        key: &ed25519_dalek::SigningKey,
    ) -> String {
        use ed25519_dalek::Signer;
        entry.signature = ambre_core::FixedBytes::<ambre_protocol::Signature>::from_array(
            key.sign(entry.signed_bytes().unwrap().as_bytes())
                .to_bytes(),
        );
        base64_encode(entry.encode().unwrap().as_bytes())
    }

    /// An unsigned entry of `doc` at `position`. The relay walks no digest,
    /// so the previous one is filler.
    fn entry_at(
        doc: Id<Document>,
        position: u64,
        author: Id<User>,
        member: (Id<User>, &ed25519_dalek::SigningKey),
        act: ambre_protocol::MembershipAct,
        role: ambre_core::Role,
    ) -> ambre_protocol::MembershipEntry {
        ambre_protocol::MembershipEntry {
            document: doc,
            position: ambre_protocol::LogPosition::new(position),
            member: member.0,
            member_key: ambre_core::FixedBytes::<ambre_core::PublicKey>::from_array(
                member.1.verifying_key().to_bytes(),
            ),
            act,
            role,
            key: (act == ambre_protocol::MembershipAct::Invite)
                .then(|| ambre_core::FixedBytes::<ambre_core::KeyId>::from_array([1; 16])),
            previous: (position > 0).then_some(
                ambre_core::FixedBytes::<ambre_protocol::EntryDigest>::from_array(
                    [position as u8; 32],
                ),
            ),
            author,
            signature: ambre_core::FixedBytes::<ambre_protocol::Signature>::from_array([0; 64]),
        }
    }

    /// A request with a JSON body.
    fn json_request(
        method: &str,
        uri: &str,
        auth: &str,
        body: &serde_json::Value,
    ) -> Request<Body> {
        Request::builder()
            .method(method)
            .uri(uri)
            .header("content-type", "application/json")
            .header("authorization", auth)
            .body(Body::from(serde_json::to_string(body).unwrap()))
            .unwrap()
    }

    /// The `log` a members answer serves.
    async fn served_log(app: &Router, auth: &str, doc_id: &str) -> Vec<serde_json::Value> {
        let resp = app
            .clone()
            .oneshot(get_with(&format!("/api/documents/{doc_id}/members"), auth))
            .await
            .unwrap();
        let bytes = axum::body::to_bytes(resp.into_body(), 64 * 1024)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        json["log"].as_array().unwrap().clone()
    }

    /// Holds `SRV-104` and `SYNC-107`. The register keeps each signed entry
    /// beside the member rows it changes, in order. Appending one leaves the
    /// earlier entries as they were. The relay refuses an entry for a place
    /// another took, one the caller did not sign, and one naming another
    /// change. It refuses a change of a logged document that carries none.
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn the_register_keeps_each_signed_entry_beside_the_rows(pool: sqlx::PgPool) {
        use ambre_core::Role;
        use ambre_protocol::MembershipAct;
        let state = test_app_state(pool).await;
        let app = build_test_router(state.clone());
        let (owner, owner_key, owner_id) = setup_signed(&app, &state, "owner@log.test", 41).await;
        let (_bob, bob_key, bob_id) = setup_signed(&app, &state, "bob@log.test", 42).await;
        let doc = Id::<Document>::mint();
        let doc_id = doc.to_string();
        let me = (owner_id, &owner_key);
        let bob = (bob_id, &bob_key);

        let create = sealed(
            entry_at(doc, 0, owner_id, me, MembershipAct::Create, Role::Owner),
            &owner_key,
        );
        let body = serde_json::json!({
            "doc_type": "prose", "document_id": doc_id,
            "doc_key_enc": base64_encode(&[5u8; 72]), "entry": create,
        });
        let (status, _) =
            status_and_code(&app, json_request("POST", "/api/documents", &owner, &body)).await;
        assert_eq!(status, StatusCode::OK);

        let invite = sealed(
            entry_at(doc, 1, owner_id, bob, MembershipAct::Invite, Role::Editor),
            &owner_key,
        );
        let body = serde_json::json!({
            "invitee_email": "bob@log.test", "role": "editor",
            "doc_key_enc": base64_encode(&[9u8; 72]), "key_id": key_id_hex(1), "entry": invite,
        });
        let share = format!("/api/documents/{doc_id}/share");
        let (status, _) = status_and_code(&app, json_request("POST", &share, &owner, &body)).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            served_log(&app, &owner, &doc_id).await,
            [create.as_str(), invite.as_str()]
        );

        let role = format!("/api/documents/{doc_id}/members/{bob_id}");
        let change = |position, key| {
            let entry = entry_at(
                doc,
                position,
                owner_id,
                bob,
                MembershipAct::ChangeRole,
                Role::Viewer,
            );
            serde_json::json!({ "role": "viewer", "entry": sealed(entry, key) })
        };
        // Signed by a key the caller does not hold.
        let (status, code) = status_and_code(
            &app,
            json_request("PATCH", &role, &owner, &change(2, &bob_key)),
        )
        .await;
        assert_eq!(
            (status, code.as_str()),
            (StatusCode::UNPROCESSABLE_ENTITY, "INVALID_SIGNATURE")
        );
        // Signed for a place another entry took.
        let (status, code) = status_and_code(
            &app,
            json_request("PATCH", &role, &owner, &change(1, &owner_key)),
        )
        .await;
        assert_eq!((status, code.as_str()), (StatusCode::CONFLICT, "LOG_MOVED"));
        // A role change carrying the invite's entry records another change.
        let body = serde_json::json!({ "role": "viewer", "entry": invite });
        let (status, _) = status_and_code(&app, json_request("PATCH", &role, &owner, &body)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        // A valid entry with a byte past its end (`SRV-227`): the bytes the
        // relay stores must be the entry's own encoding.
        let mut padded = base64_decode(change(2, &owner_key)["entry"].as_str().unwrap());
        padded.push(0);
        let body = serde_json::json!({ "role": "viewer", "entry": base64_encode(&padded) });
        let (status, _) = status_and_code(&app, json_request("PATCH", &role, &owner, &body)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        // No entry at all, on a document that keeps a log.
        let body = serde_json::json!({ "role": "viewer" });
        let (status, _) = status_and_code(&app, json_request("PATCH", &role, &owner, &body)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(served_log(&app, &owner, &doc_id).await.len(), 2);

        let changed = change(2, &owner_key);
        let (status, _) =
            status_and_code(&app, json_request("PATCH", &role, &owner, &changed)).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            served_log(&app, &owner, &doc_id).await,
            [
                create.as_str(),
                invite.as_str(),
                changed["entry"].as_str().unwrap()
            ]
        );
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
            "doc_key_enc": base64_encode(&[9u8; 72]),
            "key_id": key_id_hex(1),
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

    /// A session opened on one instance is served, renewed and ended on the
    /// other. The token minted by one is accepted by the other (`SRV-024`).
    /// No state of the session lives in either process (`SRV-021`). After a
    /// sign-out, the refresh token is refused on both (`SRV-026`).
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn a_session_crosses_instances_and_a_sign_out_ends_it_on_both(pool: sqlx::PgPool) {
        let one = crate::test_helpers::test_app_state(pool.clone()).await;
        let other = crate::test_helpers::second_instance(&one).await;
        let (app_one, app_other) = (shipped_router(one.clone()), shipped_router(other));
        let user_id = crate::test_helpers::create_test_user(&pool, "both@ambre.app").await;
        let key = ambre_core::FixedBytes::<ambre_core::PublicKey>::from_array([3u8; 32]);
        let minted = crate::auth::issue_session(&one, user_id, Some(key))
            .await
            .unwrap();
        let refresh = |app: &Router, token: &str| {
            let request = Request::post("/api/auth/refresh")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({ "refresh_token": token }).to_string(),
                ))
                .unwrap();
            app.clone().oneshot(with_peer(request))
        };

        let bearer = format!("Bearer {}", minted.access_token.secret().expose());
        let (listed, _) = status_and_code(&app_other, get_with("/api/documents", &bearer)).await;
        assert_eq!(listed, StatusCode::OK, "a token minted elsewhere");

        let renewed = refresh(&app_other, minted.refresh_token.secret().expose())
            .await
            .unwrap();
        assert_eq!(renewed.status(), StatusCode::OK);
        let body = axum::body::to_bytes(renewed.into_body(), 4096)
            .await
            .unwrap();
        let pair: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let (access, refresh_token) = (
            format!("Bearer {}", pair["access_token"].as_str().unwrap()),
            pair["refresh_token"].as_str().unwrap().to_owned(),
        );
        let (listed, _) = status_and_code(&app_one, get_with("/api/documents", &access)).await;
        assert_eq!(listed, StatusCode::OK, "a token renewed elsewhere");

        let sign_out = json_request(
            "POST",
            "/api/auth/logout",
            &access,
            &serde_json::json!({ "refresh_token": refresh_token }),
        );
        let (signed_out, _) = status_and_code(&app_other, sign_out).await;
        assert_eq!(signed_out, StatusCode::OK);
        for app in [&app_one, &app_other] {
            let refused = refresh(app, &refresh_token).await.unwrap();
            assert_eq!(refused.status(), StatusCode::UNAUTHORIZED);
        }
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

    /// `SRV-092`: a sign-out that presents another person's refresh token
    /// leaves it alive, and the caller's own token is the one it revokes.
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn a_sign_out_revokes_no_token_of_another_caller(pool: sqlx::PgPool) {
        let state = test_app_state(pool).await;
        let app = build_test_router(state.clone());
        let mine = crate::auth::issue_session(&state, Id::<User>::mint(), None)
            .await
            .unwrap();
        let theirs = crate::auth::issue_session(&state, Id::<User>::mint(), None)
            .await
            .unwrap();
        let sign_out = |refresh: &str| {
            Request::post("/api/auth/logout")
                .header(
                    "authorization",
                    format!("Bearer {}", mine.access_token.secret().expose()),
                )
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({ "refresh_token": refresh }).to_string(),
                ))
                .unwrap()
        };

        let resp = app
            .clone()
            .oneshot(sign_out(theirs.refresh_token.secret().expose()))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let alive = crate::redis_ops::refresh_get(&state.redis, &theirs.refresh_token).await;
        assert!(alive.unwrap().is_some(), "a stranger's session survives");

        let resp = app
            .oneshot(sign_out(mine.refresh_token.secret().expose()))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let gone = crate::redis_ops::refresh_get(&state.redis, &mine.refresh_token).await;
        assert!(gone.unwrap().is_none(), "the caller's own session ends");
    }

    /// `SRV-025`: a deployment that configures a day still mints a token that
    /// expires within fifteen minutes; a shorter figure is kept as set.
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn an_access_token_never_outlives_fifteen_minutes(pool: sqlx::PgPool) {
        let base = test_app_state(pool).await;
        for (configured, minted) in [(24 * 3600, 900), (60, 60)] {
            let state = AppState {
                config: Arc::new(Config {
                    access_ttl: ambre_core::Seconds(configured),
                    ..Config::default()
                }),
                ..(*base).clone()
            };
            let tokens = crate::auth::issue_session(&state, Id::<User>::mint(), None)
                .await
                .unwrap();
            let claims = state.jwt.verify(&tokens.access_token).unwrap();
            assert_eq!(claims.exp.since(claims.iat), ambre_core::Seconds(minted));
            assert_eq!(tokens.access_expires_in, ambre_core::Seconds(minted));
        }
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

    /// `SRV-075`: a page of the delta index never passes a thousand rows,
    /// whatever the caller asks, and never falls below one.
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn a_delta_page_never_exceeds_the_declared_cap(pool: sqlx::PgPool) {
        let state = test_app_state(pool.clone()).await;
        let app = build_test_router(state.clone());
        let auth = setup_and_get_auth(&app).await;
        let doc_id = create_test_doc(&app, &auth).await;
        let doc = Id::<Document>::parse_str(&doc_id).unwrap();
        let snapshot = Id::<Snapshot>::from_bytes([0u8; 16]);
        for seq in 1..=1_005u64 {
            crate::db::insert_update(
                &pool,
                &doc,
                &ambre_core::FixedBytes::<ambre_core::PublicKey>::from_array([1u8; 32]),
                &snapshot,
                ambre_protocol::messages::Clock(seq),
                ambre_core::Seq(seq),
                &ambre_core::ObjectKey::from("k"),
            )
            .await
            .unwrap();
        }
        for (asked, served) in [("5000", 1_000), ("0", 1), ("-3", 1)] {
            let resp = app
                .clone()
                .oneshot(
                    Request::get(format!(
                        "/api/documents/{doc_id}/deltas?after_seq=0&limit={asked}"
                    ))
                    .header("authorization", &auth)
                    .body(Body::empty())
                    .unwrap(),
                )
                .await
                .unwrap();
            let status = resp.status();
            let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
                .await
                .unwrap();
            assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
            let page: serde_json::Value = serde_json::from_slice(&body).unwrap();
            assert_eq!(
                page["deltas"].as_array().unwrap().len(),
                served,
                "limit={asked}"
            );
        }
    }

    fn base64_encode(data: &[u8]) -> String {
        use base64::Engine;
        base64::engine::general_purpose::URL_SAFE.encode(data)
    }

    fn base64_decode(text: &str) -> Vec<u8> {
        use base64::Engine;
        base64::engine::general_purpose::URL_SAFE
            .decode(text)
            .unwrap()
    }

    /// The identifier of test key `key`, as an invitation names it.
    fn key_id_hex(key: u8) -> String {
        hex::encode([key; 16])
    }

    /// A rotation's link from key `key` back to key `previous`, as a client
    /// posts it. The relay opens nothing, so the seal is filler.
    fn link_text(doc_id: &str, key: u8, previous: u8) -> String {
        let link = ambre_protocol::KeyLink {
            document_id: Id::<Document>::parse_str(doc_id).unwrap(),
            key: ambre_core::FixedBytes::<ambre_core::KeyId>::from_array([key; 16]),
            previous: ambre_core::FixedBytes::<ambre_core::KeyId>::from_array([previous; 16]),
            sealed: ambre_core::Payload::new(vec![key; 200]),
        };
        base64_encode(link.encode().unwrap().as_bytes())
    }

    async fn test_app_state(pool: sqlx::PgPool) -> Arc<AppState> {
        test_app_state_with(pool, empty_oidc_clients()).await
    }

    /// Like [`test_app_state`], with `oidc_clients` as the configured providers.
    async fn test_app_state_with(
        pool: sqlx::PgPool,
        oidc_clients: crate::oidc::OidcClients,
    ) -> Arc<AppState> {
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
            s3_bucket: crate::s3::BucketName::new("test-bucket".to_owned()),
            config: Arc::new(Config::default()),
            jwt: Arc::new(crate::jwt::JwtKeys::from_seed(
                &crate::jwt::JwtSeed::from_array([7u8; 32]),
            )),
            oidc_clients,
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
            dev_mode: crate::state::DevMode::On,
            ..Config::default()
        };
        let pubsub = crate::ws::PubSubHub::spawn(redis.clone());
        Arc::new(AppState {
            db: pool,
            redis,
            pubsub,
            s3,
            s3_bucket: crate::s3::BucketName::new("test-bucket".to_owned()),
            config: Arc::new(config),
            jwt: Arc::new(crate::jwt::JwtKeys::from_seed(
                &crate::jwt::JwtSeed::from_array([7u8; 32]),
            )),
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

    /// `SRV-234`: a development sign-in stores its name trimmed, and one past
    /// the cap does not decode, so no row is written.
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn a_dev_sign_in_stores_a_display_name_that_fits(pool: sqlx::PgPool) {
        let state = test_app_state_dev(pool.clone()).await;
        let app = build_test_router(state);
        let login = |name: String| {
            Request::post("/api/auth/dev/login")
                .header("content-type", "application/json")
                .body(Body::from(dev_login_body(&name).to_string()))
                .unwrap()
        };
        let resp = app.clone().oneshot(login("  ada  ".into())).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let stored: String = sqlx::query_scalar("SELECT display_name FROM users")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(stored, "ada");
        let long = "a".repeat(ambre_core::NAME_MAX_CHARS + 1);
        let resp = app.oneshot(login(long)).await.unwrap();
        assert_eq!(resp.status(), StatusCode::CONFLICT);
        let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM users")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(rows, 1, "the refused name wrote no row");
    }

    fn store_master_keys_body() -> serde_json::Value {
        serde_json::json!({
            "master_key_enc": base64_encode(&[1u8; 72]),
            "master_key_salt": hex::encode([2u8; 32]),
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

    /// `SRV-232`: a public key of the wrong length is refused before any row
    /// is written. So is it at the account setup, the dev sign-in and the key
    /// registration, for the signing key and for the exchange key.
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn a_public_key_of_the_wrong_length_is_refused_and_written_nowhere(pool: sqlx::PgPool) {
        let state = test_app_state_dev(pool).await;
        let app = build_test_router(state.clone());
        let post = |uri: &str, body: serde_json::Value, auth: Option<&str>| {
            let mut req = Request::post(uri).header("content-type", "application/json");
            if let Some(auth) = auth {
                req = req.header("authorization", auth);
            }
            req.body(Body::from(body.to_string())).unwrap()
        };
        let short = hex::encode([5u8; 31]);
        let users = || async {
            sqlx::query_scalar::<_, i64>("SELECT count(*) FROM users")
                .fetch_one(&state.db)
                .await
                .unwrap()
        };
        for key in ["signing_pubkey", "exchange_pubkey"] {
            let mut body = serde_json::json!({
                "email": "short@ambre.app",
                "display_name": "Short",
                "master_key_enc": base64_encode(&[1u8; 72]),
                "master_key_salt": hex::encode([2u8; 32]),
                "signing_pubkey": hex::encode([3u8; 32]),
                "exchange_pubkey": hex::encode([4u8; 32]),
            });
            body[key] = serde_json::json!(short);
            let resp = app
                .clone()
                .oneshot(post("/api/auth/setup", body, None))
                .await;
            assert_eq!(resp.unwrap().status(), StatusCode::CONFLICT, "setup {key}");
        }
        for key in ["signing_pub_key", "exchange_pub_key"] {
            let mut body = dev_login_body("shorty");
            body[key] = serde_json::json!(short);
            let resp = app
                .clone()
                .oneshot(post("/api/auth/dev/login", body, None))
                .await;
            assert_eq!(resp.unwrap().status(), StatusCode::CONFLICT, "dev {key}");
        }
        assert_eq!(users().await, 0, "no account was created");

        let resp = app
            .clone()
            .oneshot(post("/api/auth/dev/login", dev_login_body("alice"), None))
            .await
            .unwrap();
        let auth = format!("Bearer {}", access_token_of(resp).await);
        for key in ["signing_pubkey", "exchange_pubkey"] {
            let mut body = store_master_keys_body();
            body[key] = serde_json::json!(short);
            let resp = app
                .clone()
                .oneshot(
                    Request::put("/api/keys/master")
                        .header("content-type", "application/json")
                        .header("authorization", &auth)
                        .body(Body::from(body.to_string()))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(resp.status(), StatusCode::CONFLICT, "store {key}");
        }
        let stored: Option<Vec<u8>> =
            sqlx::query_scalar("SELECT master_key_enc FROM users WHERE email = 'alice@dev.local'")
                .fetch_one(&state.db)
                .await
                .unwrap();
        assert_eq!(stored, None, "no key material was written");
        assert_eq!(put_master_keys(&app, &auth).await, StatusCode::OK);
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
            ambre_core::OidcProvider::Google,
            &openidconnect::SubjectIdentifier::new("sub-1".to_owned()),
            &ambre_core::Email::from("new@example.com"),
            &ambre_core::DisplayName::new("New").unwrap(),
        )
        .await
        .unwrap();
        assert!(user.master_key_enc.is_none());
        assert!(user.signing_pubkey.is_none() && user.exchange_pubkey.is_none());
        let tokens = crate::auth::issue_session(&state, user.id, None)
            .await
            .unwrap();
        let setup = format!("Bearer {}", tokens.access_token.secret().expose());

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
        let session = crate::auth::authenticate_token(
            &state,
            &ambre_protocol::ws::Credential::new(token.clone()),
        )
        .unwrap();
        assert_eq!(
            session.signing_pubkey,
            ambre_core::FixedBytes::<ambre_core::PublicKey>::from_array([3u8; 32])
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
            ambre_core::OidcProvider::Google,
            &openidconnect::SubjectIdentifier::new("sub-2".to_owned()),
            &ambre_core::Email::from("keyless@example.com"),
            &ambre_core::DisplayName::new("Keyless").unwrap(),
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
                "doc_key_enc": base64_encode(&[9u8; 72]),
                "key_id": key_id_hex(1),
            "key_id": key_id_hex(1),
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
        .bind(crate::db::id_to_uuid(keyless.id))
        .bind(vec![9u8; 32])
        .execute(&state.db)
        .await
        .unwrap();
        assert_eq!(member_count(&app, &auth, &doc_id).await, 1);

        sqlx::query(
            "UPDATE users SET master_key_enc = $2, master_key_salt = $3, \
             signing_pubkey = $4, exchange_pubkey = $5 WHERE id = $1",
        )
        .bind(crate::db::id_to_uuid(keyless.id))
        .bind(vec![1u8; 56])
        .bind(vec![2u8; 32])
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
        let user = crate::db::upsert_user_from_oidc(
            &pool,
            ambre_core::OidcProvider::Google,
            &openidconnect::SubjectIdentifier::new("sub-3".to_owned()),
            &ambre_core::Email::from("c@example.com"),
            &ambre_core::DisplayName::new("C").unwrap(),
        )
        .await
        .unwrap();
        let half = sqlx::query("UPDATE users SET signing_pubkey = $2 WHERE id = $1")
            .bind(crate::db::id_to_uuid(user.id))
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
        let user = crate::db::upsert_user_from_oidc(
            &state.db,
            ambre_core::OidcProvider::Google,
            &openidconnect::SubjectIdentifier::new("sub-4".to_owned()),
            &ambre_core::Email::from("d@example.com"),
            &ambre_core::DisplayName::new("D").unwrap(),
        )
        .await
        .unwrap();
        let tokens = crate::auth::issue_session(&state, user.id, None)
            .await
            .unwrap();
        assert_eq!(
            first_close_code(state, tokens.access_token.secret().expose()).await,
            Some(CloseCode::SETUP_REQUIRED)
        );
    }

    /// `SRV-046`: an upgrade refused on authentication closes with a code of its
    /// own. The client reads it as a cue to renew rather than back off.
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn a_refused_token_closes_the_socket_with_its_own_code(pool: sqlx::PgPool) {
        let state = test_app_state(pool).await;
        assert_eq!(
            first_close_code(state, "not-a-token").await,
            Some(CloseCode::UNAUTHORIZED)
        );
    }

    /// `SRV-031`: the socket address is never read for a credential. A valid
    /// token in its query, followed by any frame but `Authenticate`, opens no
    /// session.
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn a_token_in_the_socket_address_opens_no_session(pool: sqlx::PgPool) {
        use tokio_tungstenite::tungstenite::Message;

        let state = test_app_state(pool).await;
        let app = build_test_router(state.clone());
        let auth = setup_and_get_auth(&app).await;
        let token = auth.trim_start_matches("Bearer ");
        let subscribe = WsClientMsg::Subscribe {
            document_id: Id::<Document>::mint(),
        };
        let reply = first_reply(state, &format!("?token={token}"), &subscribe).await;
        let Message::Close(Some(frame)) = reply else {
            panic!("expected a close frame, got {reply:?}");
        };
        assert_eq!(u16::from(frame.code), 4401);
    }

    /// `SRV-031`: the token travels in the socket's first frame, and that frame
    /// alone opens the session: the next frame is answered, not closed on.
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn a_token_in_the_first_frame_opens_the_session(pool: sqlx::PgPool) {
        use futures_util::{SinkExt, StreamExt};
        use tokio_tungstenite::tungstenite::Message;

        let state = test_app_state(pool).await;
        let app = build_test_router(state.clone());
        let auth = setup_and_get_auth(&app).await;
        let token = auth.trim_start_matches("Bearer ");
        let mut socket = open_socket(state, "").await;
        socket
            .send(Message::Binary(authenticate(token).into()))
            .await
            .unwrap();
        let subscribe = WsClientMsg::Subscribe {
            document_id: Id::<Document>::mint(),
        };
        socket
            .send(Message::Binary(
                encode_client_msg(&subscribe).into_bytes().into(),
            ))
            .await
            .unwrap();
        let reply = tokio::time::timeout(std::time::Duration::from_secs(5), socket.next())
            .await
            .expect("the server answers")
            .expect("a frame before the stream ends")
            .unwrap();
        let Message::Binary(bytes) = reply else {
            panic!("expected an answer, got {reply:?}");
        };
        let answer =
            ambre_protocol::ws::decode_server_msg(&ambre_protocol::WireFrame::new(bytes.to_vec()))
                .unwrap();
        assert!(
            matches!(
                answer,
                WsServerMsg::Error {
                    code: ambre_core::RefusalCode::Forbidden,
                    ..
                }
            ),
            "{answer:?}"
        );
    }

    /// `SRV-183`: a frame that does not decode concerns no document, and its
    /// refusal names none. The socket stays open.
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn a_frame_that_does_not_decode_is_refused_for_no_document(pool: sqlx::PgPool) {
        use futures_util::SinkExt;
        use tokio_tungstenite::tungstenite::Message;

        let state = test_app_state(pool).await;
        let app = build_test_router(state.clone());
        let auth = setup_and_get_auth(&app).await;
        let token = auth.trim_start_matches("Bearer ");
        let mut socket = open_socket(state, "").await;
        socket
            .send(Message::Binary(authenticate(token).into()))
            .await
            .unwrap();
        let mut garbage = encode_client_msg(&WsClientMsg::Subscribe {
            document_id: Id::<Document>::mint(),
        })
        .into_bytes();
        garbage[1] = 0xff;
        socket.send(Message::Binary(garbage.into())).await.unwrap();
        let answer = next_server_msg(&mut socket).await;
        assert!(
            matches!(
                answer,
                Some(WsServerMsg::Error {
                    document_id: None,
                    code: ambre_core::RefusalCode::BadRequest,
                })
            ),
            "{answer:?}"
        );
    }

    /// `SRV-101`: a panic while one socket is served ends that socket alone. It
    /// still leaves its document, so a peer on the same relay hears it go, and
    /// the peer's socket keeps being answered.
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn a_panicking_socket_ends_alone_and_its_peer_hears_it_leave(pool: sqlx::PgPool) {
        use futures_util::{SinkExt, StreamExt};
        use tokio_tungstenite::tungstenite::Message;

        let state = test_app_state(pool).await;
        let app = build_test_router(state.clone());
        let owner = setup_and_get_auth(&app).await;
        let doc_id = create_test_doc(&app, &owner).await;
        let peer_email = "peer@ambre.app";
        let peer = setup_named(&app, peer_email, 5).await;
        invite(&app, &owner, &doc_id, peer_email, "editor").await;
        let document_id = Id::<Document>::parse_str(&doc_id).unwrap();

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("ws://{}/api/ws", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

        let mut victim = subscribed_socket(&url, &owner, document_id).await;
        let mut watcher = subscribed_socket(&url, &peer, document_id).await;

        let panic_frame = WsClientMsg::Unsubscribe {
            document_id: crate::ws::PANIC_DOCUMENT,
        };
        victim
            .send(Message::Binary(
                encode_client_msg(&panic_frame).into_bytes().into(),
            ))
            .await
            .unwrap();
        let ended = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while let Some(Ok(frame)) = victim.next().await {
                if matches!(frame, Message::Close(_)) {
                    break;
                }
            }
        })
        .await;
        assert!(ended.is_ok(), "the socket that panicked ends");

        let owner_key = ambre_core::FixedBytes::<ambre_core::PublicKey>::from_array([3u8; 32]);
        let heard_leave = loop {
            match next_server_msg(&mut watcher).await {
                Some(WsServerMsg::PeerLeft {
                    document_id: left,
                    pub_key,
                }) if left == document_id && pub_key == owner_key => break true,
                Some(_) => continue,
                None => break false,
            }
        };
        assert!(heard_leave, "the peer hears the panicked socket leave");
        assert_refused_unknown_document(&mut watcher).await;
    }

    /// `SRV-045`, `SRV-051`, `SRV-073`, `SRV-099`: two instances serve one
    /// document. Each holds one subscription on its topic, however many sockets
    /// watch it there. A clean close and a dropped connection are both heard
    /// on either instance. Once an instance's last socket goes, its
    /// subscription goes with it.
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn the_live_set_crosses_instances_and_a_close_leaves_nothing(pool: sqlx::PgPool) {
        use rustis::commands::PubSubCommands;

        let one = crate::test_helpers::test_app_state(pool).await;
        let two = crate::test_helpers::second_instance(&one).await;
        let app = build_test_router(one.clone());
        let owner = setup_and_get_auth(&app).await;
        let doc_id = create_test_doc(&app, &owner).await;
        let peer = setup_named(&app, "near@ambre.app", 5).await;
        let far = setup_named(&app, "far@ambre.app", 6).await;
        invite(&app, &owner, &doc_id, "near@ambre.app", "editor").await;
        invite(&app, &owner, &doc_id, "far@ambre.app", "editor").await;
        let document_id = Id::<Document>::parse_str(&doc_id).unwrap();
        let serve = |state: Arc<AppState>| async move {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let url = format!("ws://{}/api/ws", listener.local_addr().unwrap());
            let app = build_test_router(state);
            tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
            url
        };
        let (url_one, url_two) = (serve(one.clone()).await, serve(two).await);
        let topic = format!("doc:{document_id}");
        let subscriptions = || async {
            let counts: std::collections::HashMap<String, usize> =
                one.redis.pub_sub_numsub([topic.as_str()]).await.unwrap();
            counts.get(&topic).copied().unwrap_or(0)
        };

        let mut owner_socket = subscribed_socket(&url_one, &owner, document_id).await;
        let peer_socket = subscribed_socket(&url_one, &peer, document_id).await;
        let mut far_socket = subscribed_socket(&url_two, &far, document_id).await;
        assert_eq!(subscriptions().await, 2, "one per instance, not per socket");

        owner_socket.close(None).await.unwrap();
        assert!(
            heard_leave(&mut far_socket, document_id, 3).await,
            "a clean close"
        );
        drop(peer_socket);
        assert!(
            heard_leave(&mut far_socket, document_id, 5).await,
            "a dropped connection"
        );

        let mut left = subscriptions().await;
        for _ in 0..50 {
            if left == 1 {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            left = subscriptions().await;
        }
        assert_eq!(left, 1, "only the other instance still subscribes");
    }

    /// Whether `socket` hears the peer of key `[key; 32]` leave `document_id`
    /// before it ends or falls silent.
    async fn heard_leave<S>(socket: &mut S, document_id: Id<Document>, key: u8) -> bool
    where
        S: futures_util::Stream<
                Item = Result<
                    tokio_tungstenite::tungstenite::Message,
                    tokio_tungstenite::tungstenite::Error,
                >,
            > + Unpin,
    {
        let key = ambre_core::FixedBytes::<ambre_core::PublicKey>::from_array([key; 32]);
        loop {
            match next_server_msg(socket).await {
                Some(WsServerMsg::PeerLeft {
                    document_id: left,
                    pub_key,
                }) if left == document_id && pub_key == key => return true,
                Some(_) => continue,
                None => return false,
            }
        }
    }

    /// A socket on `url`, authenticated with `auth` and subscribed to
    /// `document_id`. Its hub route is in place on return.
    async fn subscribed_socket(
        url: &str,
        auth: &str,
        document_id: Id<Document>,
    ) -> tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>
    {
        use futures_util::SinkExt;
        use tokio_tungstenite::tungstenite::Message;

        let mut socket = tokio_tungstenite::connect_async(url).await.unwrap().0;
        let token = auth.trim_start_matches("Bearer ");
        socket
            .send(Message::Binary(authenticate(token).into()))
            .await
            .unwrap();
        let subscribe = WsClientMsg::Subscribe { document_id };
        socket
            .send(Message::Binary(
                encode_client_msg(&subscribe).into_bytes().into(),
            ))
            .await
            .unwrap();
        assert!(matches!(
            next_server_msg(&mut socket).await,
            Some(WsServerMsg::Subscribed { .. })
        ));
        // The socket serves its frames in order, so the answer to a second one
        // means the first one's hub route is in place.
        assert_refused_unknown_document(&mut socket).await;
        socket
    }

    /// Subscribes `socket` to a document nobody created, and checks the relay
    /// answers with the refusal.
    async fn assert_refused_unknown_document<S>(socket: &mut S)
    where
        S: futures_util::Sink<tokio_tungstenite::tungstenite::Message>
            + futures_util::Stream<
                Item = Result<
                    tokio_tungstenite::tungstenite::Message,
                    tokio_tungstenite::tungstenite::Error,
                >,
            > + Unpin,
        S::Error: std::fmt::Debug,
    {
        use futures_util::SinkExt;
        use tokio_tungstenite::tungstenite::Message;

        let probe = WsClientMsg::Subscribe {
            document_id: Id::<Document>::mint(),
        };
        socket
            .send(Message::Binary(
                encode_client_msg(&probe).into_bytes().into(),
            ))
            .await
            .unwrap();
        let answer = loop {
            match next_server_msg(socket).await {
                Some(WsServerMsg::Error { code, .. }) => break Some(code),
                Some(_) => continue,
                None => break None,
            }
        };
        assert_eq!(
            answer,
            Some(ambre_core::RefusalCode::Forbidden),
            "the socket is still answered"
        );
    }

    /// The next message the server sends on `socket`, or `None` once the
    /// socket ends or stays silent for five seconds.
    async fn next_server_msg<S>(socket: &mut S) -> Option<WsServerMsg>
    where
        S: futures_util::Stream<
                Item = Result<
                    tokio_tungstenite::tungstenite::Message,
                    tokio_tungstenite::tungstenite::Error,
                >,
            > + Unpin,
    {
        use futures_util::StreamExt;
        use tokio_tungstenite::tungstenite::Message;

        loop {
            let frame = tokio::time::timeout(std::time::Duration::from_secs(5), socket.next())
                .await
                .ok()??
                .ok()?;
            match frame {
                Message::Binary(bytes) => {
                    return ambre_protocol::ws::decode_server_msg(&ambre_protocol::WireFrame::new(
                        bytes.to_vec(),
                    ))
                    .ok();
                }
                Message::Close(_) => return None,
                _ => continue,
            }
        }
    }

    /// The `Authenticate` frame for `token`, encoded.
    fn authenticate(token: &str) -> Vec<u8> {
        encode_client_msg(&WsClientMsg::Authenticate {
            token: ambre_protocol::ws::Credential::new(token.to_owned()),
        })
        .into_bytes()
    }

    /// Serves the router on a loopback port and opens the socket at the address
    /// followed by `query`.
    async fn open_socket(
        state: Arc<AppState>,
        query: &str,
    ) -> tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>
    {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let app = build_test_router(state);
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let url = format!("ws://{addr}/api/ws{query}");
        tokio_tungstenite::connect_async(url).await.unwrap().0
    }

    /// Opens the socket, sends `first` as its first frame, and returns the
    /// frame the server answers with.
    async fn first_reply(
        state: Arc<AppState>,
        query: &str,
        first: &WsClientMsg,
    ) -> tokio_tungstenite::tungstenite::Message {
        use futures_util::{SinkExt, StreamExt};
        use tokio_tungstenite::tungstenite::Message;

        let mut socket = open_socket(state, query).await;
        socket
            .send(Message::Binary(
                encode_client_msg(first).into_bytes().into(),
            ))
            .await
            .unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(5), socket.next())
            .await
            .expect("the server answers")
            .expect("a frame before the stream ends")
            .unwrap()
    }

    /// Opens the socket, authenticates with `token`, and returns the code of
    /// the close frame the server sends first.
    async fn first_close_code(state: Arc<AppState>, token: &str) -> Option<CloseCode> {
        use tokio_tungstenite::tungstenite::Message;

        let first = WsClientMsg::Authenticate {
            token: ambre_protocol::ws::Credential::new(token.to_owned()),
        };
        let reply = first_reply(state, "", &first).await;
        let Message::Close(Some(frame)) = reply else {
            panic!("expected a close frame, got {reply:?}");
        };
        CloseCode::new(u16::from(frame.code))
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
            "master_key_enc": base64_encode(&[1u8; 72]),
            "master_key_salt": hex::encode([2u8; 32]),
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
            "master_key_enc": base64_encode(&[1u8; 72]),
            "master_key_salt": hex::encode([2u8; 32]),
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
            "doc_key_enc": base64_encode(&[9u8; 72]),
            "key_id": key_id_hex(1),
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

    use crate::test_helpers::{S3Stub, offline_s3};

    /// Like [`test_app_state`], with the given S3 client.
    async fn test_app_state_with_s3(pool: sqlx::PgPool, s3: aws_sdk_s3::Client) -> Arc<AppState> {
        crate::test_helpers::with_s3(&*test_app_state(pool).await, s3)
    }

    async fn test_app_state_presigning(pool: sqlx::PgPool) -> Arc<AppState> {
        test_app_state_with_s3(pool, offline_s3(None)).await
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

    /// Sends `DELETE` on `uri` as `auth`, and answers the status.
    async fn delete_status(app: &Router, auth: &str, uri: String) -> StatusCode {
        app.clone()
            .oneshot(
                Request::delete(uri)
                    .header("authorization", auth)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap()
            .status()
    }

    // `SRV-138` to `SRV-141`: each uploader holds a claim on a blob. A member
    // who uploaded nothing is refused. The object stays while another uploader
    // holds it, and the last erase removes the object and its row and nothing
    // else of the document.
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn an_erase_removes_a_blob_once_its_last_uploader_asks(pool: sqlx::PgPool) {
        let setup_state = test_app_state(pool.clone()).await;
        let setup = build_test_router(setup_state);
        let owner = setup_and_get_auth(&setup).await;
        let doc_id = create_test_doc(&setup, &owner).await;
        let second = setup_named(&setup, "second@ambre.app", 5).await;
        let third = setup_named(&setup, "third@ambre.app", 6).await;
        invite(&setup, &owner, &doc_id, "second@ambre.app", "editor").await;
        invite(&setup, &owner, &doc_id, "third@ambre.app", "editor").await;

        let hash = "e".repeat(64);
        let env = Config::default().s3_env_prefix;
        let doc = Id::<Document>::parse_str(&doc_id).unwrap();
        let media = crate::s3::format_s3_media_key(
            &env,
            &doc,
            &ambre_core::ContentHash::parse(&hash).expect("a hash"),
        );
        let snapshot = format!("{env}/documents/{doc_id}/snapshots/s1");
        let stub = S3Stub::spawn([media.0.clone(), snapshot.clone()]).await;
        let state = test_app_state_with_s3(pool, offline_s3(Some(&stub.endpoint))).await;
        let app = shipped_router(state.clone());
        let confirm = serde_json::json!({ "content_hash": hash, "size_bytes": 7 });
        let erase = format!("/api/documents/{doc_id}/media/{hash}");
        for member in [&owner, &second] {
            let uri = format!("/api/documents/{doc_id}/media");
            assert_eq!(
                post_json(&app, member, uri, confirm.clone()).await,
                StatusCode::OK
            );
        }
        let hash = ambre_core::ContentHash::parse(&hash).expect("a hash");
        let held = || crate::db::document_holds_media(&state.db, &doc, &hash);

        assert_eq!(
            delete_status(&app, &third, erase.clone()).await,
            StatusCode::FORBIDDEN,
            "a member who uploaded nothing erases nothing"
        );
        assert_eq!(
            delete_status(&app, &owner, erase.clone()).await,
            StatusCode::OK
        );
        assert!(held().await.unwrap(), "the second uploader keeps the blob");
        assert!(stub.object(media.as_str()).is_some());

        assert_eq!(
            delete_status(&app, &second, erase.clone()).await,
            StatusCode::OK
        );
        assert!(!held().await.unwrap(), "the last erase removes the row");
        assert_eq!(stub.keys(), vec![snapshot], "and the object alone");
        assert_eq!(
            delete_status(&app, &owner, erase).await,
            StatusCode::FORBIDDEN,
            "an erased claim is not erased twice"
        );
    }

    /// Posts `body` as JSON and answers the JSON the route returns, once it
    /// answers 200.
    async fn post_for_json(
        app: &Router,
        auth: &str,
        uri: String,
        body: serde_json::Value,
    ) -> serde_json::Value {
        let resp = app
            .clone()
            .oneshot(
                Request::post(uri)
                    .header("content-type", "application/json")
                    .header("authorization", auth)
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(resp.into_body(), 64 * 1024)
            .await
            .unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    // `SYNC-139`: a blob put through the relay's upload link comes back, byte
    // for byte, through its download link. Both legs reach a real object store.
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn a_download_fetches_the_bytes_an_upload_put(pool: sqlx::PgPool) {
        let (s3, bucket) = crate::test_helpers::storage_service().await;
        let state = crate::test_helpers::with_storage(&*test_app_state(pool).await, s3, bucket);
        let setup = build_test_router(state.clone());
        let app = shipped_router(state);
        let owner = setup_and_get_auth(&setup).await;
        let doc = create_test_doc(&setup, &owner).await;
        // Every byte value, at a length no block size divides.
        let blob: Vec<u8> = (0..=255u8).cycle().take(4099).collect();
        let hash = "0123456789abcdef".repeat(4);
        let size = blob.len() as u64;
        // The store is local: no proxy stands between the test and it.
        let http = reqwest::Client::builder().no_proxy().build().unwrap();

        let upload = post_for_json(
            &app,
            &owner,
            format!("/api/documents/{doc}/media/upload-url"),
            serde_json::json!({ "content_hash": hash, "size_bytes": size }),
        )
        .await;
        let put = http
            .put(upload["upload_url"].as_str().unwrap())
            .body(blob.clone())
            .send()
            .await
            .unwrap();
        assert!(put.status().is_success(), "upload: {}", put.status());
        post_for_json(
            &app,
            &owner,
            format!("/api/documents/{doc}/media"),
            serde_json::json!({ "content_hash": hash, "size_bytes": size }),
        )
        .await;

        let download = post_for_json(
            &app,
            &owner,
            format!("/api/documents/{doc}/media/download-url"),
            serde_json::json!({ "content_hash": hash }),
        )
        .await;
        let get = http
            .get(download["download_url"].as_str().unwrap())
            .send()
            .await
            .unwrap();
        assert!(get.status().is_success(), "download: {}", get.status());
        assert_eq!(get.bytes().await.unwrap().as_ref(), blob.as_slice());
    }

    // `ZK-008`: no presigned URL is signed before the document's access check.
    // An outsider gets none of the four (`SRV-002`), a viewer gets neither
    // upload URL (`SRV-056`).
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn a_presigned_url_is_signed_only_after_the_access_check(pool: sqlx::PgPool) {
        let state = test_app_state_presigning(pool).await;
        let setup = build_test_router(state.clone());
        let app = shipped_router(state);
        let owner = setup_and_get_auth(&setup).await;
        let doc_id = create_test_doc(&setup, &owner).await;
        let viewer_email = "viewer@ambre.app";
        let viewer = setup_named(&setup, viewer_email, 5).await;
        invite(&setup, &owner, &doc_id, viewer_email, "viewer").await;
        let outsider = setup_named(&setup, "outsider@ambre.app", 6).await;
        let hash = "a".repeat(64);
        let upload = [
            (
                "media/upload-url",
                serde_json::json!({ "content_hash": hash, "size_bytes": 10 }),
            ),
            (
                "snapshot/upload-url",
                serde_json::json!({ "size_bytes": 10 }),
            ),
        ];
        let download = [
            (
                "media/download-url",
                serde_json::json!({ "content_hash": hash }),
            ),
            (
                "snapshot/download-url",
                serde_json::json!({ "snapshot_id": doc_id }),
            ),
        ];

        for (path, body) in upload.iter().chain(download.iter()) {
            let uri = format!("/api/documents/{doc_id}/{path}");
            let status = post_json(&app, &outsider, uri, body.clone()).await;
            assert_eq!(status, StatusCode::FORBIDDEN, "outsider, {path}");
        }
        for (path, body) in &upload {
            let uri = format!("/api/documents/{doc_id}/{path}");
            let status = post_json(&app, &viewer, uri.clone(), body.clone()).await;
            assert_eq!(status, StatusCode::FORBIDDEN, "viewer, {path}");
            assert_eq!(
                post_json(&app, &owner, uri, body.clone()).await,
                StatusCode::OK,
                "{path}"
            );
        }
    }

    // `SRV-078`: issuing a link neither reads nor writes the object it names.
    // Storage here refuses every connection, so any such call would fail.
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn issuing_a_link_never_touches_the_object(pool: sqlx::PgPool) {
        let state = test_app_state_with_s3(pool, offline_s3(Some("http://127.0.0.1:1"))).await;
        let setup = build_test_router(state.clone());
        let app = shipped_router(state);
        let owner = setup_and_get_auth(&setup).await;
        let doc_id = create_test_doc(&setup, &owner).await;
        let hash = "e".repeat(64);
        let confirm = serde_json::json!({ "content_hash": hash, "size_bytes": 7 });
        let uri = format!("/api/documents/{doc_id}/media");
        assert_eq!(post_json(&app, &owner, uri, confirm).await, StatusCode::OK);
        let snapshot_id = upload_snapshot(&app, &owner, &doc_id, 0).await;
        let links = [
            (
                "media/upload-url",
                serde_json::json!({ "content_hash": hash, "size_bytes": 7 }),
            ),
            (
                "media/download-url",
                serde_json::json!({ "content_hash": hash }),
            ),
            (
                "snapshot/upload-url",
                serde_json::json!({ "size_bytes": 7 }),
            ),
            (
                "snapshot/download-url",
                serde_json::json!({ "snapshot_id": snapshot_id }),
            ),
        ];

        for (path, body) in links {
            let uri = format!("/api/documents/{doc_id}/{path}");
            let status = post_json(&app, &owner, uri, body).await;
            assert_eq!(status, StatusCode::OK, "{path}");
        }
    }

    // `SRV-055`: a blob never exceeds the cap its kind declares. Media is
    // refused above its cap on the link and on the confirmation, a snapshot
    // on the link. Each upload link signs the declared length.
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn a_blob_above_its_kind_cap_is_refused(pool: sqlx::PgPool) {
        let state = test_app_state_presigning(pool).await;
        let media_cap = state.config.media_max_size.0;
        let snapshot_cap = state.config.snapshot_max_size.0;
        let setup = build_test_router(state.clone());
        let app = shipped_router(state);
        let owner = setup_and_get_auth(&setup).await;
        let doc_id = create_test_doc(&setup, &owner).await;
        let hash = "f".repeat(64);
        let media = |size: u64| serde_json::json!({ "content_hash": hash, "size_bytes": size });
        let snapshot = |size: u64| serde_json::json!({ "size_bytes": size });
        let cases = [
            ("media/upload-url", media(media_cap), StatusCode::OK),
            (
                "media/upload-url",
                media(media_cap + 1),
                StatusCode::PAYLOAD_TOO_LARGE,
            ),
            ("media/upload-url", media(0), StatusCode::PAYLOAD_TOO_LARGE),
            ("media", media(media_cap + 1), StatusCode::PAYLOAD_TOO_LARGE),
            ("media", media(media_cap), StatusCode::OK),
            (
                "snapshot/upload-url",
                snapshot(snapshot_cap),
                StatusCode::OK,
            ),
            (
                "snapshot/upload-url",
                snapshot(snapshot_cap + 1),
                StatusCode::PAYLOAD_TOO_LARGE,
            ),
            (
                "snapshot/upload-url",
                snapshot(0),
                StatusCode::PAYLOAD_TOO_LARGE,
            ),
        ];

        for (path, body, expected) in cases {
            let uri = format!("/api/documents/{doc_id}/{path}");
            let status = post_json(&app, &owner, uri, body.clone()).await;
            assert_eq!(status, expected, "{path} {body}");
        }
        for path in ["media/upload-url", "snapshot/upload-url"] {
            let resp = app
                .clone()
                .oneshot(
                    Request::post(format!("/api/documents/{doc_id}/{path}"))
                        .header("content-type", "application/json")
                        .header("authorization", &owner)
                        .body(Body::from(media(7).to_string()))
                        .unwrap(),
                )
                .await
                .unwrap();
            let bytes = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
            let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
            let url = json["upload_url"].as_str().unwrap();
            let signed = url
                .split(['?', '&'])
                .find_map(|kv| kv.strip_prefix("X-Amz-SignedHeaders="))
                .unwrap();
            assert!(signed.contains("content-length"), "{path}: {url}");
        }
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

    /// `SRV-228`: no client builds a parent proof, so a confirmation that
    /// carries one is refused and writes no row. A `null` proof reads as none.
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn a_snapshot_carrying_a_parent_proof_is_refused(pool: sqlx::PgPool) {
        let state = test_app_state(pool).await;
        let app = build_test_router(state.clone());
        // setup_and_get_auth registers signing_pubkey [3u8; 32].
        let owner = setup_and_get_auth(&app).await;
        let doc_id = create_test_doc(&app, &owner).await;
        let snap_uuid = uuid::Uuid::new_v4();
        let confirm = |proof: serde_json::Value| {
            let body = serde_json::json!({
                "snapshot_id": snap_uuid.to_string(),
                "author_pubkey": hex::encode([3u8; 32]),
                "parent_snapshot_proof": proof,
                "update_clocks": {},
                "s3_key": format!("dev-ambre/documents/{doc_id}/snapshots/{snap_uuid}"),
            });
            Request::post(format!("/api/documents/{doc_id}/snapshot"))
                .header("content-type", "application/json")
                .header("authorization", &owner)
                .body(Body::from(serde_json::to_string(&body).unwrap()))
                .unwrap()
        };
        let rows = || async {
            sqlx::query_scalar::<_, i64>(
                "SELECT count(*) FROM snapshots WHERE document_id::text = $1",
            )
            .bind(&doc_id)
            .fetch_one(&state.db)
            .await
            .unwrap()
        };

        let resp = app
            .clone()
            .oneshot(confirm(serde_json::json!(base64_encode(&[1u8; 32]))))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        assert_eq!(rows().await, 0);

        let resp = app.oneshot(confirm(serde_json::Value::Null)).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(rows().await, 1);
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
            "doc_key_enc": base64_encode(&[9u8; 72]),
            "key_id": key_id_hex(1),
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

    /// Sets up an account whose signing key is `key`, so it can sign what the
    /// relay verifies. Returns its `Authorization` header.
    async fn setup_signing(app: &Router, email: &str, key: &ed25519_dalek::SigningKey) -> String {
        let body = serde_json::json!({
            "email": email,
            "display_name": "Signer",
            "master_key_enc": base64_encode(&[1u8; 72]),
            "master_key_salt": hex::encode([2u8; 32]),
            "signing_pubkey": hex::encode(key.verifying_key().to_bytes()),
            "exchange_pubkey": hex::encode([8u8; 32]),
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
        format!("Bearer {}", access_token_of(resp).await)
    }

    /// The owner of `setup_and_get_auth` uploads a snapshot covering `covered`,
    /// and gets its id back.
    async fn upload_snapshot(app: &Router, owner: &str, doc_id: &str, covered: i64) -> String {
        let body = serde_json::json!({
            "snapshot_id": uuid::Uuid::new_v4().to_string(),
            "author_pubkey": hex::encode([3u8; 32]),
            "update_clocks": {},
            "s3_key": format!("dev-ambre/documents/{doc_id}/snapshots/{}", uuid::Uuid::new_v4()),
            "covered_seq": covered,
        });
        let resp = app
            .clone()
            .oneshot(
                Request::post(format!("/api/documents/{doc_id}/snapshot"))
                    .header("content-type", "application/json")
                    .header("authorization", owner)
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(resp.into_body(), 1024).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        json["snapshot_id"].as_str().unwrap().to_owned()
    }

    /// The snapshot an open is served, as its JSON, `None` on a 404.
    async fn served_snapshot(app: &Router, auth: &str, doc_id: &str) -> Option<serde_json::Value> {
        let resp = app
            .clone()
            .oneshot(
                Request::get(format!("/api/documents/{doc_id}/snapshot"))
                    .header("authorization", auth)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        if resp.status() == StatusCode::NOT_FOUND {
            return None;
        }
        let bytes = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
        Some(serde_json::from_slice(&bytes).unwrap())
    }

    /// `SRV-126`, `SRV-128`, `SRV-129`: an upload is announced to the
    /// document's members, with the horizon it claims. Nobody is served it until
    /// it is confirmed. Its author's confirmation makes it `self`, served beside
    /// no reference yet, and another member's makes it `confirmed`.
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn a_snapshot_is_served_once_confirmed(pool: sqlx::PgPool) {
        use ambre_protocol::ws::decode_server_msg;
        use futures_util::StreamExt;
        use rustis::commands::PubSubCommands;

        let state = test_app_state(pool).await;
        let app = build_test_router(state.clone());
        let owner = setup_and_get_auth(&app).await;
        let doc_id = create_test_doc(&app, &owner).await;
        let checker = setup_named(&app, "checker@ambre.app", 43).await;
        invite(&app, &owner, &doc_id, "checker@ambre.app", "viewer").await;

        let listener = rustis::client::Client::connect("redis://localhost:6379")
            .await
            .unwrap();
        let mut stream = listener.subscribe(format!("doc:{doc_id}")).await.unwrap();
        let snap = upload_snapshot(&app, &owner, &doc_id, 0).await;
        let heard = tokio::time::timeout(std::time::Duration::from_secs(5), stream.next())
            .await
            .expect("the announcement arrives")
            .unwrap()
            .unwrap();
        assert!(matches!(
            decode_server_msg(&ambre_protocol::WireFrame::new(heard.payload().to_vec())).unwrap(),
            WsServerMsg::SnapshotAvailable { snapshot_id, .. } if snapshot_id.to_string() == snap
        ));
        assert!(served_snapshot(&app, &owner, &doc_id).await.is_none());

        let confirm = format!("/api/documents/{doc_id}/snapshots/{snap}/confirm");
        let status = post_json(&app, &owner, confirm.clone(), serde_json::json!({})).await;
        assert_eq!(status, StatusCode::OK);
        let served = served_snapshot(&app, &owner, &doc_id).await.unwrap();
        assert_eq!(served["status"], "self");
        assert_eq!(served["snapshot_id"], snap.as_str());
        assert!(served["reference"].is_null());

        let status = post_json(&app, &checker, confirm, serde_json::json!({})).await;
        assert_eq!(status, StatusCode::OK);
        let served = served_snapshot(&app, &checker, &doc_id).await.unwrap();
        assert_eq!(served["status"], "confirmed");

        let outsider = setup_named(&app, "outsider@ambre.app", 44).await;
        let other = format!("/api/documents/{doc_id}/snapshots/{snap}/confirm");
        let status = post_json(&app, &outsider, other, serde_json::json!({})).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
    }

    /// `SRV-133`, `SRV-134`, `SRV-135`, `AUTHOR-023`: a report signed by its
    /// reporter drops a pending snapshot, keeps every delta, and asks the
    /// reporter for the next one. An unsigned report changes nothing, and a
    /// snapshot somebody confirmed outlives a report.
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn a_signed_report_drops_a_pending_snapshot_alone(pool: sqlx::PgPool) {
        use ambre_protocol::snapshot_report::report_transcript;
        use ambre_protocol::ws::decode_server_msg;
        use ed25519_dalek::Signer;
        use futures_util::StreamExt;
        use rustis::commands::PubSubCommands;

        let stub = crate::test_helpers::S3Stub::spawn([]).await;
        let base = test_app_state(pool).await;
        let state = crate::test_helpers::with_s3(
            &base,
            crate::test_helpers::offline_s3(Some(&stub.endpoint)),
        );
        let app = build_test_router(state.clone());
        let owner = setup_and_get_auth(&app).await;
        let doc_id = create_test_doc(&app, &owner).await;
        let key = ed25519_dalek::SigningKey::from_bytes(&[7u8; 32]);
        let reporter = setup_signing(&app, "reporter@ambre.app", &key).await;
        invite(&app, &owner, &doc_id, "reporter@ambre.app", "editor").await;
        let reporter_id = doc_user_id(&state, "reporter@ambre.app").await;

        let document = Id::<Document>::parse_str(&doc_id).unwrap();
        for seq in 1..=3i64 {
            crate::db::insert_update(
                &state.db,
                &document,
                &ambre_core::FixedBytes::<ambre_core::PublicKey>::from_array([3u8; 32]),
                &Id::<Snapshot>::mint(),
                ambre_protocol::messages::Clock(seq.unsigned_abs()),
                ambre_core::Seq(seq.unsigned_abs()),
                &ambre_core::ObjectKey(format!("k{seq}")),
            )
            .await
            .unwrap();
        }
        let snap = upload_snapshot(&app, &owner, &doc_id, 3).await;
        let snapshot = Id::<Snapshot>::parse_str(&snap).unwrap();
        let report = format!("/api/documents/{doc_id}/snapshots/{snap}/report");
        let signed = |author: [u8; 32]| {
            let transcript = report_transcript(
                document,
                snapshot,
                &ambre_core::FixedBytes::<ambre_core::PublicKey>::from_array(author),
            );
            serde_json::json!({ "signature": hex::encode(key.sign(transcript.as_bytes()).to_bytes()) })
        };

        // Signed over another author: refused, and the snapshot stays.
        let status = post_json(&app, &reporter, report.clone(), signed([9u8; 32])).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        let held = crate::db::get_snapshot(&state.db, &document, &snapshot).await;
        assert!(held.unwrap().is_some());

        let listener = rustis::client::Client::connect("redis://localhost:6379")
            .await
            .unwrap();
        let mut stream = listener
            .subscribe(format!("user:{reporter_id}"))
            .await
            .unwrap();
        let status = post_json(&app, &reporter, report, signed([3u8; 32])).await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        let held = crate::db::get_snapshot(&state.db, &document, &snapshot).await;
        assert!(held.unwrap().is_none(), "the pending snapshot is dropped");
        let deltas = crate::db::get_updates_after(
            &state.db,
            &document,
            ambre_core::Seq(0),
            crate::db::QueryLimit::saturating(10),
        )
        .await;
        assert_eq!(deltas.unwrap().len(), 3, "no delta is dropped with it");
        let heard = tokio::time::timeout(std::time::Duration::from_secs(5), stream.next())
            .await
            .expect("the reporter is asked for the next snapshot")
            .unwrap()
            .unwrap();
        assert!(matches!(
            decode_server_msg(&ambre_protocol::WireFrame::new(heard.payload().to_vec())).unwrap(),
            WsServerMsg::SnapshotRequired { .. }
        ));

        // A snapshot the reporter itself confirmed outlives a report.
        let kept = upload_snapshot(&app, &owner, &doc_id, 3).await;
        let confirm = format!("/api/documents/{doc_id}/snapshots/{kept}/confirm");
        post_json(&app, &reporter, confirm, serde_json::json!({})).await;
        let kept_id = Id::<Snapshot>::parse_str(&kept).unwrap();
        let transcript = report_transcript(
            document,
            kept_id,
            &ambre_core::FixedBytes::<ambre_core::PublicKey>::from_array([3u8; 32]),
        );
        let body = serde_json::json!({ "signature": hex::encode(key.sign(transcript.as_bytes()).to_bytes()) });
        let report = format!("/api/documents/{doc_id}/snapshots/{kept}/report");
        assert_eq!(
            post_json(&app, &reporter, report, body).await,
            StatusCode::NO_CONTENT
        );
        let held = crate::db::get_snapshot(&state.db, &document, &kept_id).await;
        assert!(held.unwrap().is_some(), "a confirmed snapshot stays");
    }

    async fn doc_user_id(state: &AppState, email: &str) -> String {
        crate::db::get_user_by_email(&state.db, &ambre_core::Email::from(email))
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
        assert_eq!(owner_row["doc_key_enc"], base64_encode(&[5u8; 72]));

        // Nominal path: the editor may still invite a fresh viewer.
        let viewer_email = "viewer2@ambre.app";
        crate::test_helpers::create_test_user(&state.db, viewer_email).await;
        assert_eq!(
            invite_status(&app, &editor, &doc_id, viewer_email, "viewer").await,
            StatusCode::OK
        );
    }

    /// `SRV-088`: a removal tells the person removed, on their own topic. The
    /// document's topic and the member who stays learn only that the members
    /// changed and that a key is owed. Neither names anybody.
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn a_removal_reaches_the_person_removed_alone(pool: sqlx::PgPool) {
        use ambre_protocol::ws::decode_server_msg;
        use futures_util::StreamExt;
        use rustis::commands::PubSubCommands;

        let state = test_app_state(pool).await;
        let app = build_test_router(state.clone());
        let owner = setup_and_get_auth(&app).await;
        let doc_id = create_test_doc(&app, &owner).await;
        setup_named(&app, "gone@ambre.app", 41).await;
        setup_named(&app, "stays@ambre.app", 42).await;
        invite(&app, &owner, &doc_id, "gone@ambre.app", "editor").await;
        invite(&app, &owner, &doc_id, "stays@ambre.app", "editor").await;
        let gone = doc_user_id(&state, "gone@ambre.app").await;
        let stays = doc_user_id(&state, "stays@ambre.app").await;

        let topics = [
            format!("doc:{doc_id}"),
            format!("user:{gone}"),
            format!("user:{stays}"),
        ];
        let listener = rustis::client::Client::connect("redis://localhost:6379")
            .await
            .unwrap();
        let mut stream = listener.subscribe(topics.clone()).await.unwrap();
        assert_eq!(
            revoke_status(&app, &owner, &doc_id, &gone).await,
            StatusCode::OK
        );
        // One end marker per topic: a topic's messages arrive in order, so all
        // that the removal sent has arrived once the three markers have.
        let end = b"end".to_vec();
        for topic in &topics {
            let _: usize = state
                .redis
                .publish(topic, rustis::resp::RefBulkString::new(&end))
                .await
                .unwrap();
        }

        let mut heard: std::collections::HashMap<String, Vec<WsServerMsg>> = Default::default();
        let mut ended = 0;
        while ended < topics.len() {
            let m = tokio::time::timeout(std::time::Duration::from_secs(5), stream.next())
                .await
                .expect("the markers arrive")
                .unwrap()
                .unwrap();
            if m.payload() == end.as_slice() {
                ended += 1;
                continue;
            }
            let topic = m.channel_str().unwrap().to_owned();
            heard.entry(topic).or_default().push(
                decode_server_msg(&ambre_protocol::WireFrame::new(m.payload().to_vec())).unwrap(),
            );
        }

        assert!(matches!(
            heard.remove(&topics[1]).unwrap_or_default().as_slice(),
            [WsServerMsg::DocumentRevoked { .. }]
        ));
        assert!(matches!(
            heard.remove(&topics[0]).unwrap_or_default().as_slice(),
            [
                WsServerMsg::MembersChanged { .. },
                WsServerMsg::KeyRotationRequired { .. }
            ]
        ));
        assert!(
            heard.is_empty(),
            "the member who stays hears nothing: {heard:?}"
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
            "doc_key_enc": base64_encode(&[5u8; 72]),
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
            "keys": [ { "user_id": editor_uid, "doc_key_enc": base64_encode(&[0xABu8; 72]) } ],
            "members": [ editor_uid ],
            "link": link_text(&doc_id, 2, 1),
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

    /// Sends a request with an optional JSON body and returns its status.
    async fn status_of(
        app: &Router,
        request: axum::http::request::Builder,
        auth: &str,
        body: Option<serde_json::Value>,
    ) -> StatusCode {
        let request = request.header("authorization", auth);
        let request = match body {
            Some(json) => request
                .header("content-type", "application/json")
                .body(Body::from(json.to_string())),
            None => request.body(Body::empty()),
        };
        app.clone()
            .oneshot(request.unwrap())
            .await
            .unwrap()
            .status()
    }

    /// Whether the member list says the document owes a rotation.
    async fn owes_rotation(app: &Router, auth: &str, doc_id: &str) -> bool {
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
        let bytes = axum::body::to_bytes(resp.into_body(), 64 * 1024)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        json["rotation_owed"].as_bool().unwrap()
    }

    /// Whether the ephemeral store mirrors a rotation owed by the document.
    async fn debt_mirrored(state: &AppState, doc_id: &str) -> bool {
        use rustis::commands::GenericCommands;
        let doc = Id::<Document>::from_uuid(ambre_core::Uuid::parse_str(doc_id).unwrap());
        let n: usize = state
            .redis
            .exists(crate::redis_keys::RotationOwedKey::of(&doc).as_str())
            .await
            .unwrap();
        n == 1
    }

    /// `SRV-145`, `SRV-122`: an editor may remove themselves, and only
    /// themselves. The removal posts no rotation, so the document owes one, in
    /// the register and in its mirror.
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn a_member_leaves_alone_and_the_document_owes_a_rotation(pool: sqlx::PgPool) {
        let state = test_app_state(pool).await;
        let app = build_test_router(state.clone());
        let owner = setup_and_get_auth(&app).await;
        let doc_id = create_test_doc(&app, &owner).await;
        let editor = setup_named(&app, "leaver@test.com", 72).await;
        invite(&app, &owner, &doc_id, "leaver@test.com", "editor").await;
        let editor_uid = doc_user_id(&state, "leaver@test.com").await;
        let owner_uid = list_members(&app, &owner, &doc_id)
            .await
            .into_iter()
            .find(|m| m["user_id"] != editor_uid.as_str())
            .unwrap()["user_id"]
            .as_str()
            .unwrap()
            .to_string();
        assert!(!owes_rotation(&app, &owner, &doc_id).await);

        let others = Request::delete(format!("/api/documents/{doc_id}/share/{owner_uid}"));
        assert_eq!(
            status_of(&app, others, &editor, None).await,
            StatusCode::FORBIDDEN,
            "an editor removes nobody but themselves"
        );

        let own = Request::delete(format!("/api/documents/{doc_id}/share/{editor_uid}"));
        assert_eq!(status_of(&app, own, &editor, None).await, StatusCode::OK);
        assert_eq!(list_members(&app, &owner, &doc_id).await.len(), 1);
        assert!(owes_rotation(&app, &owner, &doc_id).await);
        assert!(debt_mirrored(&state, &doc_id).await);
    }

    /// `SRV-144`, `SRV-124`: while a rotation is owed an editor may post it,
    /// and the rotation that lands clears the debt and its mirror.
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn an_owed_rotation_is_open_to_an_editor_and_clears_the_debt(pool: sqlx::PgPool) {
        let state = test_app_state(pool).await;
        let app = build_test_router(state.clone());
        let owner = setup_and_get_auth(&app).await;
        let doc_id = create_test_doc(&app, &owner).await;
        let stayer = setup_named(&app, "stayer@test.com", 73).await;
        invite(&app, &owner, &doc_id, "stayer@test.com", "editor").await;
        let leaver = setup_named(&app, "gone@test.com", 74).await;
        invite(&app, &owner, &doc_id, "gone@test.com", "editor").await;
        let stayer_uid = doc_user_id(&state, "stayer@test.com").await;
        let leaver_uid = doc_user_id(&state, "gone@test.com").await;

        let leave = Request::delete(format!("/api/documents/{doc_id}/share/{leaver_uid}"));
        assert_eq!(status_of(&app, leave, &leaver, None).await, StatusCode::OK);
        assert!(owes_rotation(&app, &owner, &doc_id).await);

        let members: Vec<String> = list_members(&app, &owner, &doc_id)
            .await
            .iter()
            .map(|m| m["user_id"].as_str().unwrap().to_string())
            .collect();
        let keys = serde_json::json!({
            "keys": [ { "user_id": stayer_uid, "doc_key_enc": base64_encode(&[0xCDu8; 72]) } ],
            "members": members,
            "link": link_text(&doc_id, 2, 1),
        });
        let rotate = Request::post(format!("/api/documents/{doc_id}/rotate-key"));
        assert_eq!(
            status_of(&app, rotate, &stayer, Some(keys.clone())).await,
            StatusCode::OK,
            "an editor settles an owed rotation"
        );
        assert!(!owes_rotation(&app, &owner, &doc_id).await);
        assert!(!debt_mirrored(&state, &doc_id).await);

        let again = Request::post(format!("/api/documents/{doc_id}/rotate-key"));
        assert_eq!(
            status_of(&app, again, &stayer, Some(keys)).await,
            StatusCode::FORBIDDEN,
            "with the debt settled, rotating is the owner's again"
        );
    }

    /// `SYNC-126`, `SYNC-041`: one request removes the member and installs the
    /// new key for everyone who stays. No debt is left behind.
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn a_revoke_removes_and_rotates_in_one_request(pool: sqlx::PgPool) {
        let state = test_app_state(pool).await;
        let app = build_test_router(state.clone());
        let owner = setup_and_get_auth(&app).await;
        let doc_id = create_test_doc(&app, &owner).await;
        let _ = setup_named(&app, "revokee@test.com", 75).await;
        invite(&app, &owner, &doc_id, "revokee@test.com", "editor").await;
        let revokee_uid = doc_user_id(&state, "revokee@test.com").await;
        let owner_uid = list_members(&app, &owner, &doc_id)
            .await
            .into_iter()
            .find(|m| m["user_id"] != revokee_uid.as_str())
            .unwrap()["user_id"]
            .as_str()
            .unwrap()
            .to_string();

        let new_key = base64_encode(&[0xEFu8; 72]);
        let body = serde_json::json!({
            "user_id": revokee_uid,
            "keys": [ { "user_id": owner_uid, "doc_key_enc": new_key } ],
            "members": [ owner_uid ],
            "link": link_text(&doc_id, 2, 1),
        });
        let revoke = Request::post(format!("/api/documents/{doc_id}/revoke"));
        assert_eq!(
            status_of(&app, revoke, &owner, Some(body)).await,
            StatusCode::OK
        );
        assert_eq!(
            key_links(&app, &owner, &doc_id).await,
            vec![link_text(&doc_id, 2, 1)],
            "the link lands with the copies"
        );
        let members = list_members(&app, &owner, &doc_id).await;
        assert_eq!(members.len(), 1, "the member is removed");
        assert_eq!(members[0]["doc_key_enc"], new_key, "and the key rotated");
        assert!(!owes_rotation(&app, &owner, &doc_id).await);
        assert!(!debt_mirrored(&state, &doc_id).await);
    }

    /// `SYNC-047`: the rotation notice follows the stored copies. A revoke the
    /// store refuses publishes nothing, and a member that hears the notice of
    /// one that lands already reads the new copy.
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn the_rotation_notice_follows_the_stored_copies(pool: sqlx::PgPool) {
        let state = test_app_state(pool).await;
        let app = build_test_router(state.clone());
        let owner = setup_and_get_auth(&app).await;
        let doc_id = create_test_doc(&app, &owner).await;
        let _ = setup_named(&app, "leaving@test.com", 80).await;
        let stayer = setup_named(&app, "staying@test.com", 81).await;
        invite(&app, &owner, &doc_id, "leaving@test.com", "editor").await;
        invite(&app, &owner, &doc_id, "staying@test.com", "editor").await;
        let leaving_uid = doc_user_id(&state, "leaving@test.com").await;
        let kept: Vec<String> = list_members(&app, &owner, &doc_id)
            .await
            .iter()
            .map(|m| m["user_id"].as_str().unwrap().to_string())
            .filter(|uid| *uid != leaving_uid)
            .collect();

        let (tx, mut notices) = tokio::sync::mpsc::channel(8);
        state
            .pubsub
            .subscribe(
                crate::ws::Topic::document(&Id::<Document>::parse_str(&doc_id).unwrap()),
                crate::ws::ConnectionId::named("watcher"),
                tx,
            )
            .await
            .unwrap();
        let new_key = base64_encode(&[0xE7; 72]);
        let revoke_keeping = |read: &[String]| {
            let keys: Vec<_> = read
                .iter()
                .map(|uid| serde_json::json!({ "user_id": uid, "doc_key_enc": new_key }))
                .collect();
            serde_json::json!({
                "user_id": leaving_uid,
                "keys": keys,
                "members": read,
                "link": link_text(&doc_id, 2, 1),
            })
        };
        let revoke = || Request::post(format!("/api/documents/{doc_id}/revoke"));

        // The read misses the member who stays, so the store refuses it whole.
        let refused = status_of(&app, revoke(), &owner, Some(revoke_keeping(&kept[..1]))).await;
        assert_eq!(refused, StatusCode::CONFLICT);
        let quiet = tokio::time::timeout(std::time::Duration::from_millis(300), notices.recv());
        assert!(quiet.await.is_err(), "a refused store publishes no notice");

        let landing = tokio::spawn({
            let app = app.clone();
            let owner = owner.clone();
            let body = revoke_keeping(&kept);
            let request = revoke();
            async move { status_of(&app, request, &owner, Some(body)).await }
        });
        let heard = tokio::time::timeout(std::time::Duration::from_secs(5), notices.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(
            ambre_protocol::ws::decode_server_msg(&ambre_protocol::WireFrame::new(
                heard.expose_bytes().to_vec()
            )),
            Ok(WsServerMsg::KeyRotationRequired { .. })
        ));
        let read = list_members(&app, &stayer, &doc_id).await;
        assert!(
            read.iter().all(|m| m["doc_key_enc"] == new_key),
            "the notice found every copy stored: {read:?}"
        );
        assert_eq!(landing.await.unwrap(), StatusCode::OK);
    }

    /// `SRV-167`, `SRV-168`: an invitation and a revoke race on one document.
    /// The revoke read the members before the invitation landed, so it is
    /// refused whole; read again, it wraps for the newcomer too. An invitation
    /// wrapping the key the revoke replaced is refused. Every member left holds
    /// a copy of the current key.
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn a_member_invited_during_a_revoke_gets_the_new_key(pool: sqlx::PgPool) {
        let state = test_app_state(pool).await;
        let app = build_test_router(state.clone());
        let owner = setup_and_get_auth(&app).await;
        let doc_id = create_test_doc(&app, &owner).await;
        let _ = setup_named(&app, "revokee@test.com", 77).await;
        invite(&app, &owner, &doc_id, "revokee@test.com", "editor").await;
        let revokee_uid = doc_user_id(&state, "revokee@test.com").await;
        let uids = |members: Vec<serde_json::Value>| -> Vec<String> {
            members
                .iter()
                .map(|m| m["user_id"].as_str().unwrap().to_string())
                .filter(|uid| *uid != revokee_uid)
                .collect()
        };
        // The revoke reads the members it keeps: the owner alone.
        let read_before = uids(list_members(&app, &owner, &doc_id).await);
        assert_eq!(read_before.len(), 1);

        // The invitation lands between the read and the post.
        let _ = setup_named(&app, "newcomer@test.com", 78).await;
        invite(&app, &owner, &doc_id, "newcomer@test.com", "editor").await;
        let newcomer_uid = doc_user_id(&state, "newcomer@test.com").await;

        let revoke_with = |read: Vec<String>| {
            let keys: Vec<_> = read
                .iter()
                .map(|uid| serde_json::json!({ "user_id": uid, "doc_key_enc": base64_encode(&[0xE2; 72]) }))
                .collect();
            serde_json::json!({
                "user_id": revokee_uid,
                "keys": keys,
                "members": read,
                "link": link_text(&doc_id, 2, 1),
            })
        };
        let revoke = || Request::post(format!("/api/documents/{doc_id}/revoke"));
        let resp = app
            .clone()
            .oneshot(
                revoke()
                    .header("authorization", &owner)
                    .header("content-type", "application/json")
                    .body(Body::from(revoke_with(read_before).to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::CONFLICT);
        let bytes = axum::body::to_bytes(resp.into_body(), 64 * 1024)
            .await
            .unwrap();
        let refusal: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(refusal["error"], "MEMBERS_CHANGED");
        assert_eq!(
            list_members(&app, &owner, &doc_id).await.len(),
            3,
            "a refused revoke removes no one"
        );
        assert!(key_links(&app, &owner, &doc_id).await.is_empty());

        // Read again, the revoke names the newcomer and wraps for them.
        let read_again = uids(list_members(&app, &owner, &doc_id).await);
        assert!(read_again.contains(&newcomer_uid));
        assert_eq!(
            status_of(&app, revoke(), &owner, Some(revoke_with(read_again))).await,
            StatusCode::OK
        );
        let members = list_members(&app, &owner, &doc_id).await;
        assert_eq!(members.len(), 2);
        assert!(
            members
                .iter()
                .all(|m| m["doc_key_enc"] == base64_encode(&[0xE2; 72])),
            "every member left holds the current key: {members:?}"
        );

        // An invitation that wrapped the key the revoke replaced is refused.
        let _ = setup_named(&app, "late@test.com", 79).await;
        let stale = serde_json::json!({
            "invitee_email": "late@test.com",
            "role": "editor",
            "doc_key_enc": base64_encode(&[9u8; 72]),
            "key_id": key_id_hex(1),
        });
        let share = || Request::post(format!("/api/documents/{doc_id}/share"));
        assert_eq!(
            status_of(&app, share(), &owner, Some(stale)).await,
            StatusCode::CONFLICT
        );
        assert_eq!(list_members(&app, &owner, &doc_id).await.len(), 2);
        let current = serde_json::json!({
            "invitee_email": "late@test.com",
            "role": "editor",
            "doc_key_enc": base64_encode(&[0xE2; 72]),
            "key_id": key_id_hex(2),
        });
        assert_eq!(
            status_of(&app, share(), &owner, Some(current)).await,
            StatusCode::OK
        );
    }

    /// The next message the relay published on `stream`, decoded. Panics past
    /// a second, so a missing broadcast fails the test rather than hanging it.
    async fn next_broadcast(
        stream: &mut rustis::client::PubSubStream,
    ) -> ambre_protocol::ws::WsServerMsg {
        use futures_util::StreamExt;
        let msg = tokio::time::timeout(std::time::Duration::from_secs(1), stream.next())
            .await
            .expect("a broadcast within a second")
            .expect("an open stream")
            .expect("a message");
        ambre_protocol::ws::decode_server_msg(&ambre_protocol::WireFrame::new(
            msg.payload().to_vec(),
        ))
        .expect("a server message")
    }

    /// SRV-177: an invitation, a change of role and a removal each tell the
    /// connections subscribed to the document that its members changed. The
    /// message names the document alone.
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn a_change_of_members_reaches_the_subscribers_of_the_document(pool: sqlx::PgPool) {
        use ambre_protocol::ws::WsServerMsg;
        use rustis::commands::PubSubCommands;
        let state = test_app_state(pool).await;
        let app = build_test_router(state.clone());
        let owner = setup_and_get_auth(&app).await;
        let doc_id = create_test_doc(&app, &owner).await;
        let document_id = Id::<Document>::parse_str(&doc_id).expect("a document id");
        // A connection of its own: a subscriber sharing the relay's client
        // would desynchronise its replies (see `PubSubHub`).
        let subscriber = rustis::client::Client::connect("redis://localhost:6379")
            .await
            .expect("redis");
        let mut stream = subscriber
            .subscribe(format!("doc:{doc_id}"))
            .await
            .expect("a subscription");
        let changed =
            |msg| matches!(msg, WsServerMsg::MembersChanged { document_id: d } if d == document_id);

        setup_named(&app, "joiner@test.com", 93).await;
        invite(&app, &owner, &doc_id, "joiner@test.com", "viewer").await;
        assert!(changed(next_broadcast(&mut stream).await));

        let joiner_id = doc_user_id(&state, "joiner@test.com").await;
        assert_eq!(
            patch_role_status(&app, &owner, &doc_id, &joiner_id, "editor").await,
            StatusCode::OK
        );
        assert!(changed(next_broadcast(&mut stream).await));

        assert_eq!(
            revoke_status(&app, &owner, &doc_id, &joiner_id).await,
            StatusCode::OK
        );
        assert!(changed(next_broadcast(&mut stream).await));
    }

    /// The links a member reads back, oldest first.
    async fn key_links(app: &Router, auth: &str, doc_id: &str) -> Vec<String> {
        let resp = app
            .clone()
            .oneshot(
                Request::get(format!("/api/documents/{doc_id}/key-links"))
                    .header("authorization", auth)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        json["links"]
            .as_array()
            .unwrap()
            .iter()
            .map(|l| l.as_str().unwrap().to_string())
            .collect()
    }

    /// `SYNC-117`, `SYNC-118`: every rotation's link is kept, in the order
    /// the rotations landed. Three rotations are refused whole, with no copy
    /// changed. One link does not chain from the last key (`SRV-193`), one
    /// belongs to another document, and one is missing. A non-member reads
    /// no link.
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn the_relay_keeps_every_link_in_rotation_order(pool: sqlx::PgPool) {
        let state = test_app_state(pool).await;
        let app = build_test_router(state.clone());
        let owner = setup_and_get_auth(&app).await;
        let doc_id = create_test_doc(&app, &owner).await;
        let owner_uid = list_members(&app, &owner, &doc_id).await[0]["user_id"]
            .as_str()
            .unwrap()
            .to_string();
        let rotation = |copy: u8, link: String| {
            serde_json::json!({
                "keys": [ { "user_id": owner_uid, "doc_key_enc": base64_encode(&[copy; 72]) } ],
                "members": [ owner_uid ],
                "link": link,
            })
        };
        let rotate = || Request::post(format!("/api/documents/{doc_id}/rotate-key"));

        for (copy, key, previous) in [(0xA1, 2, 1), (0xA2, 3, 2), (0xA3, 4, 3)] {
            let body = rotation(copy, link_text(&doc_id, key, previous));
            assert_eq!(
                status_of(&app, rotate(), &owner, Some(body)).await,
                StatusCode::OK
            );
        }
        let chained = vec![
            link_text(&doc_id, 2, 1),
            link_text(&doc_id, 3, 2),
            link_text(&doc_id, 4, 3),
        ];
        assert_eq!(key_links(&app, &owner, &doc_id).await, chained);

        // A rotation racing one that landed first chains from a stale key.
        let stale = rotation(0xB1, link_text(&doc_id, 5, 3));
        assert_eq!(
            status_of(&app, rotate(), &owner, Some(stale)).await,
            StatusCode::CONFLICT
        );
        let other_doc = create_test_doc(&app, &owner).await;
        let misfiled = rotation(0xB2, link_text(&other_doc, 5, 4));
        assert_eq!(
            status_of(&app, rotate(), &owner, Some(misfiled)).await,
            StatusCode::BAD_REQUEST
        );
        let bare = serde_json::json!({
            "keys": [ { "user_id": owner_uid, "doc_key_enc": base64_encode(&[0xB3; 72]) } ],
            "members": [ owner_uid ],
        });
        assert!(
            status_of(&app, rotate(), &owner, Some(bare))
                .await
                .is_client_error()
        );
        assert_eq!(key_links(&app, &owner, &doc_id).await, chained);
        let members = list_members(&app, &owner, &doc_id).await;
        assert_eq!(members[0]["doc_key_enc"], base64_encode(&[0xA3; 72]));

        let stranger = setup_named(&app, "stranger@test.com", 76).await;
        let read = Request::get(format!("/api/documents/{doc_id}/key-links"));
        assert_eq!(
            status_of(&app, read, &stranger, None).await,
            StatusCode::FORBIDDEN
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
            "doc_key_enc": base64_encode(&[6u8; 72]),
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
