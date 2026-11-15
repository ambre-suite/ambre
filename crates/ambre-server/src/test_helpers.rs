#![cfg(test)]

use std::collections::HashMap;
use std::sync::Arc;

use ambre_core::id::{Document, Id, User};
use sqlx::PgPool;

use crate::db;
use crate::jwt::JwtKeys;
use crate::oidc::OidcClients;
use crate::state::{AppState, Config};

/// Empty OIDC clients map -- no providers configured.
pub fn empty_oidc_clients() -> OidcClients {
    Arc::new(HashMap::new())
}

/// Connects to the Redis of the db tier: `REDIS_URL`, which `.cargo/config.toml`
/// sets to the one server of `just services-up`. `just test-db-cluster` points
/// it at the cluster of `just cluster-up`, so every test runs against both.
pub async fn connect_test_redis() -> rustis::Result<rustis::client::Client> {
    let url = std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://localhost:6379".into());
    rustis::client::Client::connect(url.as_str()).await
}

/// The user of [`connect_document_keys_redis`], and its password.
const DOCUMENT_KEYS_USER: &str = "ambre-document-keys";
const DOCUMENT_KEYS_PASSWORD: &str = "document-keys";

/// Connects to the Redis of the db tier as a user that reaches the keys of
/// a document alone. A command on any other key, the dirty set among them,
/// fails with `NOPERM`. So a test breaks one step of the store for its own
/// connection, and every other test keeps the shared keys whole.
///
/// The ACL rule reaches every node of a cluster.
pub async fn connect_document_keys_redis() -> rustis::Result<rustis::client::Client> {
    use rustis::commands::ServerCommands;
    let admin = connect_test_redis().await?;
    admin
        .acl_setuser(
            DOCUMENT_KEYS_USER,
            [
                "reset",
                "on",
                &format!(">{DOCUMENT_KEYS_PASSWORD}"),
                "~doc:*",
                "&*",
                "+@all",
            ],
        )
        .await?;
    let url = std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://localhost:6379".into());
    let mut config = rustis::client::IntoConfig::into_config(url.as_str())?;
    config.username = Some(DOCUMENT_KEYS_USER.to_owned());
    config.password = Some(DOCUMENT_KEYS_PASSWORD.to_owned());
    rustis::client::Client::connect(config).await
}

/// Like the state given, with another Redis connection and a hub on it.
pub fn with_redis(base: &AppState, redis: rustis::client::Client) -> Arc<AppState> {
    let pubsub = crate::ws::PubSubHub::spawn(redis.clone());
    Arc::new(AppState {
        db: base.db.clone(),
        redis,
        pubsub,
        s3: base.s3.clone(),
        s3_bucket: base.s3_bucket.clone(),
        config: Arc::clone(&base.config),
        jwt: Arc::clone(&base.jwt),
        oidc_clients: Arc::clone(&base.oidc_clients),
    })
}

/// Builds a test [`AppState`] with a real Redis connection (`REDIS_URL`, see
/// [`connect_test_redis`]) and a bare S3 client. Requires a running Redis;
/// S3-operations are not exercised by the auth-gating tests that use this.
pub async fn test_app_state(pool: sqlx::PgPool) -> Arc<AppState> {
    let redis = crate::test_helpers::connect_test_redis()
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
        s3_bucket: crate::s3::StoreText::<crate::s3::BucketName>::new("test-bucket".to_owned()),
        config: Arc::new(Config::default()),
        jwt: Arc::new(
            JwtKeys::from_seed(&crate::jwt::JwtSeed::from_array(TEST_JWT_SEED))
                .with_admission(&crate::jwt::JwtSeed::from_array(TEST_ADMISSION_SEED)),
        ),
        oidc_clients: empty_oidc_clients(),
    })
}

/// The token signing seed every test instance derives its keys from, as a
/// deployment's instances share `JWT_SIGNING_SEED`.
pub(crate) const TEST_JWT_SEED: [u8; 32] = [9u8; 32];

/// The seed of the admission key of every test instance: a key of its own, as in a deployment.
const TEST_ADMISSION_SEED: [u8; 32] = [8u8; 32];

/// A second relay instance beside `base`, as a deployment runs it: the same
/// Postgres, Redis server and configuration, and keys derived from the same
/// seed. It opens its own Redis connection, because a subscription belongs
/// to one, and spawns its own hub (`ambre-spec-testing.md` section 5.C).
pub async fn second_instance(base: &AppState) -> Arc<AppState> {
    let redis = crate::test_helpers::connect_test_redis()
        .await
        .expect("need Redis for tests");
    let pubsub = crate::ws::PubSubHub::spawn(redis.clone());
    Arc::new(AppState {
        db: base.db.clone(),
        redis,
        pubsub,
        s3: base.s3.clone(),
        s3_bucket: base.s3_bucket.clone(),
        config: Arc::clone(&base.config),
        jwt: Arc::new(
            JwtKeys::from_seed(&crate::jwt::JwtSeed::from_array(TEST_JWT_SEED))
                .with_admission(&crate::jwt::JwtSeed::from_array(TEST_ADMISSION_SEED)),
        ),
        oidc_clients: Arc::clone(&base.oidc_clients),
    })
}

/// Whether the register of deletes still marks `document`.
pub async fn deletion_marked(pool: &PgPool, document: Id<Document>) -> bool {
    sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS (SELECT 1 FROM document_deletions WHERE document_id = $1)",
    )
    .bind(uuid::Uuid::from_bytes(*document.as_bytes()))
    .fetch_one(pool)
    .await
    .expect("deletion register")
}

/// Makes the second sweep of every marked delete due now.
pub async fn make_sweeps_due(pool: &PgPool) {
    sqlx::query("UPDATE document_deletions SET sweep_due = 0")
        .execute(pool)
        .await
        .expect("sweep due");
}

/// Creates a test user directly in the database (bypasses OIDC flow).
pub async fn create_test_user(pool: &PgPool, email: &str) -> Id<User> {
    let user = db::create_user(
        pool,
        &ambre_core::Email::from(email),
        &ambre_core::Validated::<ambre_core::DisplayName>::parse("Test User").unwrap(),
        &ambre_core::Encoded::new([1u8; 32].to_vec()),
        &ambre_core::FixedBytes::<ambre_core::MasterKeySalt>::from_array([2u8; 32]),
        &ambre_core::FixedBytes::<ambre_core::PublicKey>::from_array([3u8; 32]),
        &ambre_core::FixedBytes::<ambre_core::ExchangePublicKey>::from_array([4u8; 32]),
    )
    .await
    .expect("failed to create test user");
    user.id
}

/// Like the state given, with another S3 client.
pub fn with_s3(base: &AppState, s3: aws_sdk_s3::Client) -> Arc<AppState> {
    with_storage(base, s3, base.s3_bucket.clone())
}

/// Like the state given, with another S3 client and bucket.
pub fn with_storage(
    base: &AppState,
    s3: aws_sdk_s3::Client,
    bucket: crate::s3::StoreText<crate::s3::BucketName>,
) -> Arc<AppState> {
    Arc::new(AppState {
        db: base.db.clone(),
        redis: base.redis.clone(),
        pubsub: base.pubsub.clone(),
        s3,
        s3_bucket: bucket,
        config: Arc::clone(&base.config),
        jwt: Arc::clone(&base.jwt),
        oidc_clients: Arc::clone(&base.oidc_clients),
    })
}

/// An S3 client with static credentials and a region. It presigns offline,
/// and it reaches `endpoint` when one is given.
pub fn offline_s3(endpoint: Option<&str>) -> aws_sdk_s3::Client {
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

/// The object store of the database tier, and its bucket, which this call
/// creates when it is missing. `S3_ENDPOINT` names the store. The credentials
/// and the bucket default to those of the local stack, `scripts/garage_init.py`.
///
/// A run with no `S3_ENDPOINT` fails, so the tier cannot pass without a store.
pub async fn storage_service() -> (
    aws_sdk_s3::Client,
    crate::s3::StoreText<crate::s3::BucketName>,
) {
    let endpoint = std::env::var("S3_ENDPOINT")
        .expect("S3_ENDPOINT: the database tier needs an object store (just services-up)");
    let env = |name: &str, default: &str| std::env::var(name).unwrap_or_else(|_| default.into());
    let conf = aws_sdk_s3::Config::builder()
        .behavior_version(aws_sdk_s3::config::BehaviorVersion::latest())
        .region(aws_sdk_s3::config::Region::new("us-east-1"))
        .credentials_provider(aws_sdk_s3::config::Credentials::new(
            env("AWS_ACCESS_KEY_ID", "GK1c7374dda8bf9b67e1ed2022"),
            env(
                "AWS_SECRET_ACCESS_KEY",
                "b64c22af0c1f051c482b80d9cbde6751304fd0d084a565d2a5fb0342021899ca",
            ),
            None,
            None,
            "db-tier",
        ))
        .endpoint_url(endpoint)
        .force_path_style(true)
        .build();
    let s3 = aws_sdk_s3::Client::from_conf(conf);
    let bucket = env("S3_BUCKET", "ambre-dev");
    if s3.head_bucket().bucket(&bucket).send().await.is_err() {
        s3.create_bucket()
            .bucket(&bucket)
            .send()
            .await
            .expect("create the database tier's bucket");
    }
    (
        s3,
        crate::s3::StoreText::<crate::s3::BucketName>::new(bucket),
    )
}

/// What an [`S3Stub`] holds: each object's bytes, how often each was read, and
/// how long a read of each is held before it is answered.
#[derive(Default)]
struct StubStore {
    objects: std::collections::BTreeMap<String, Vec<u8>>,
    gets: HashMap<String, usize>,
    delays: HashMap<String, std::time::Duration>,
}

/// An in-process object store over path-style requests. It answers the calls
/// the relay makes: `PutObject`, `GetObject`, `DeleteObject`, `ListObjectsV2`
/// and `DeleteObjects`.
pub struct S3Stub {
    store: Arc<std::sync::Mutex<StubStore>>,
    pub endpoint: String,
}

impl S3Stub {
    /// Starts a stub holding `keys`, each an empty object.
    pub async fn spawn(keys: impl IntoIterator<Item = String>) -> Self {
        use axum::body::Body;
        use axum::http::Request;
        let store = StubStore {
            objects: keys.into_iter().map(|k| (k, Vec::new())).collect(),
            ..StubStore::default()
        };
        let store = Arc::new(std::sync::Mutex::new(store));
        let held = Arc::clone(&store);
        let app = axum::Router::new().fallback(move |req: Request<Body>| {
            let held = Arc::clone(&held);
            async move { Self::answer(&held, req).await }
        });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await });
        Self { store, endpoint }
    }

    async fn answer(
        held: &std::sync::Mutex<StubStore>,
        req: axum::http::Request<axum::body::Body>,
    ) -> axum::response::Response {
        use axum::http::{Method, StatusCode};
        use axum::response::IntoResponse;
        let query = req.uri().query().unwrap_or("").to_string();
        let key = req
            .uri()
            .path()
            .trim_start_matches('/')
            .split_once('/')
            .map(|(_, k)| k.replace("%2F", "/"))
            .unwrap_or_default();
        let method = req.method().clone();
        let chunked = req
            .headers()
            .get("content-encoding")
            .is_some_and(|v| v.to_str().unwrap_or("").contains("aws-chunked"));
        let body = axum::body::to_bytes(req.into_body(), 1 << 24)
            .await
            .unwrap()
            .to_vec();
        let body = if chunked { unchunk(&body) } else { body };
        let delay = held.lock().unwrap().delays.get(&key).copied();
        if let Some(delay) = delay
            && method == Method::GET
        {
            tokio::time::sleep(delay).await;
        }
        let xmlns = r#"xmlns="http://s3.amazonaws.com/doc/2006-03-01/""#;
        let mut store = held.lock().unwrap();
        if method == Method::PUT && !key.is_empty() {
            store.objects.insert(key, body);
            return StatusCode::OK.into_response();
        }
        if method == Method::GET && !key.is_empty() {
            *store.gets.entry(key.clone()).or_default() += 1;
            return match store.objects.get(&key) {
                Some(bytes) => bytes.clone().into_response(),
                None => (
                    StatusCode::NOT_FOUND,
                    format!("<Error><Code>NoSuchKey</Code><Key>{key}</Key></Error>"),
                )
                    .into_response(),
            };
        }
        if method == Method::DELETE && !key.is_empty() {
            store.objects.remove(&key);
            return StatusCode::NO_CONTENT.into_response();
        }
        if method == Method::POST && query.contains("delete") {
            let text = String::from_utf8_lossy(&body);
            for part in text.split("<Key>").skip(1) {
                store.objects.remove(part.split("</Key>").next().unwrap());
            }
            return format!("<DeleteResult {xmlns}></DeleteResult>").into_response();
        }
        let prefix = query
            .split('&')
            .find_map(|kv| kv.strip_prefix("prefix="))
            .map(|p| p.replace("%2F", "/"))
            .unwrap_or_default();
        let contents: String = store
            .objects
            .keys()
            .filter(|k| k.starts_with(&prefix))
            .map(|k| format!("<Contents><Key>{k}</Key><Size>1</Size></Contents>"))
            .collect();
        format!(
            "<ListBucketResult {xmlns}><Name>test-bucket</Name><Prefix>{prefix}</Prefix>\
             <IsTruncated>false</IsTruncated>{contents}</ListBucketResult>"
        )
        .into_response()
    }

    /// Every key held, in order.
    pub fn keys(&self) -> Vec<String> {
        self.store.lock().unwrap().objects.keys().cloned().collect()
    }

    /// An object a late writer puts behind the relay's back, as a presigned
    /// upload does.
    pub fn put(&self, key: String) {
        self.store.lock().unwrap().objects.insert(key, Vec::new());
    }

    /// Holds every read of `key` for `delay` before answering it, so reads
    /// issued together complete in the order the test picks.
    pub fn delay(&self, key: String, delay: std::time::Duration) {
        self.store.lock().unwrap().delays.insert(key, delay);
    }

    /// The bytes of an object, when held.
    pub fn object(&self, key: &str) -> Option<Vec<u8>> {
        self.store.lock().unwrap().objects.get(key).cloned()
    }

    /// How many times an object was read.
    pub fn gets(&self, key: &str) -> usize {
        self.store
            .lock()
            .unwrap()
            .gets
            .get(key)
            .copied()
            .unwrap_or(0)
    }
}

/// Decodes an `aws-chunked` body: hex sizes, each followed by its chunk, up
/// to the empty chunk the trailers follow.
fn unchunk(body: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut rest = body;
    loop {
        let Some(eol) = rest.windows(2).position(|w| w == b"\r\n") else {
            return out;
        };
        let head = String::from_utf8_lossy(&rest[..eol]);
        let size =
            usize::from_str_radix(head.split(';').next().unwrap_or("0").trim(), 16).unwrap_or(0);
        if size == 0 {
            return out;
        }
        let start = eol + 2;
        out.extend_from_slice(&rest[start..start + size]);
        rest = &rest[start + size + 2..];
    }
}

/// Creates a test document and adds the owner as an owner member.
pub async fn create_test_document(
    pool: &PgPool,
    owner_id: Id<User>,
    doc_type: &str,
) -> Id<Document> {
    let doc_type = doc_type.parse().expect("a document type word");
    let (doc, _created) = db::create_document(pool, &owner_id, doc_type, None, None)
        .await
        .expect("failed to create test document");
    let doc_id = doc.id;
    db::add_document_member(
        pool,
        &doc_id,
        &db::NewMember {
            user_id: &owner_id,
            role: ambre_core::Role::Owner,
            doc_key_enc: &ambre_core::Encoded::new([5u8; 32].to_vec()),
            invited_by: &owner_id,
        },
        None,
    )
    .await
    .expect("failed to add owner member");
    doc_id
}
