use std::cell::RefCell;
use std::rc::Rc;

use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

/// A session token: an access or a refresh token. It wipes its bytes when
/// dropped, so a sign-out leaves no token in the worker's memory
/// (`SYNC-100`). Its `Debug` prints no byte of it, so no log can carry one
/// (`SYNC-061`). Serialized and parsed as the bare string the relay exchanges.
#[derive(Clone, Default, Deserialize, Serialize, Zeroize, ZeroizeOnDrop)]
#[serde(transparent)]
pub(crate) struct SecretToken(String);

impl SecretToken {
    /// The token's text, for the one header or query that carries it.
    pub(crate) fn expose(&self) -> &str {
        &self.0
    }
}

impl From<String> for SecretToken {
    fn from(token: String) -> Self {
        Self(token)
    }
}

impl std::fmt::Debug for SecretToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SecretToken(<redacted>)")
    }
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct TokenPair {
    pub access_token: SecretToken,
    pub refresh_token: SecretToken,
    #[allow(dead_code)]
    pub access_expires_in: u64,
}

pub(crate) type TokenStore = Rc<RefCell<Option<TokenPair>>>;

#[derive(Clone)]
pub(crate) struct AuthContext {
    pub api_base: String,
    pub url_scheme: &'static str,
    pub tokens: TokenStore,
}

#[derive(Serialize)]
struct RefreshRequest {
    refresh_token: SecretToken,
}

impl AuthContext {
    fn access_token(&self) -> Result<SecretToken, String> {
        self.tokens
            .borrow()
            .as_ref()
            .map(|t| t.access_token.clone())
            .ok_or_else(|| "auth required: no access token".to_owned())
    }

    fn refresh_token(&self) -> Result<SecretToken, String> {
        self.tokens
            .borrow()
            .as_ref()
            .map(|t| t.refresh_token.clone())
            .ok_or_else(|| "auth required: no refresh token".to_owned())
    }

    /// Renews the token pair over the browser's `fetch`. The socket calls it
    /// on its own refusal, which only the wasm build reaches.
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    pub(crate) async fn refresh(&self) -> Result<(), String> {
        self.refresh_via(&Fetch).await
    }

    /// Renews the token pair through `transport`. The refresh token is the
    /// one credential the request carries.
    async fn refresh_via(&self, transport: &impl Transport) -> Result<(), String> {
        let url = format!("{}://{}/api/auth/refresh", self.url_scheme, self.api_base);
        let body = RefreshRequest {
            refresh_token: self.refresh_token()?,
        };
        let next = post_json_via::<_, TokenPair>(transport, &url, &body).await?;
        *self.tokens.borrow_mut() = Some(next);
        Ok(())
    }
}

/// The `Authorization` header value, wiped once the request holds its copy.
fn bearer(token: &SecretToken) -> Zeroizing<String> {
    Zeroizing::new(format!("Bearer {}", token.expose()))
}

/// Per-request timeout for every Sync HTTP call (M9). Without it a hung TCP
/// connection or a stalled proxy would block the Sync worker's request future
/// forever (the browser `fetch` has no default timeout).
const REQUEST_TIMEOUT_MS: u32 = 30_000;

/// The method of one request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Method {
    /// `GET`.
    Get,
    /// `POST`.
    Post,
    /// `PUT`.
    Put,
    /// `PATCH`.
    Patch,
    /// `DELETE`.
    Delete,
}

impl Method {
    /// The method's name, as an error message carries it.
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Get => "GET",
            Self::Post => "POST",
            Self::Put => "PUT",
            Self::Patch => "PATCH",
            Self::Delete => "DELETE",
        }
    }
}

/// One request as the transport sends it. The policy below builds it, and
/// the transport only moves it.
#[derive(Clone)]
pub(crate) struct Outgoing {
    /// The method.
    pub method: Method,
    /// Where it goes.
    pub url: String,
    /// The `Authorization` value, on a request to the relay that needs one.
    pub bearer: Option<Zeroizing<String>>,
    /// The content type and the bytes of the body, if any.
    pub body: Option<(&'static str, Vec<u8>)>,
    /// How long the transport waits before it aborts the request.
    pub timeout_ms: u32,
}

impl Outgoing {
    /// A request with no body and no credential, under the per-request
    /// timeout.
    fn new(method: Method, url: &str) -> Self {
        Self {
            method,
            url: url.to_owned(),
            bearer: None,
            body: None,
            timeout_ms: REQUEST_TIMEOUT_MS,
        }
    }

    /// A request carrying `body` as JSON.
    fn json<T: Serialize>(method: Method, url: &str, body: &T) -> Result<Self, String> {
        let bytes = serde_json::to_vec(body)
            .map_err(|e| format!("{} {url}: serialize error: {e}", method.as_str()))?;
        Ok(Self {
            body: Some(("application/json", bytes)),
            ..Self::new(method, url)
        })
    }
}

/// One answer: its status and its body.
pub(crate) struct Incoming {
    /// The HTTP status.
    pub status: u16,
    /// The body, empty when there is none.
    pub body: Vec<u8>,
}

/// What moves a request and brings its answer back. The renewal, the replay
/// and the error mapping live above it, so a host test drives them through a
/// scripted transport.
#[allow(async_fn_in_trait)] // single-threaded worker; futures are intentionally !Send
pub(crate) trait Transport {
    /// Sends `request`. An error is a request that got no answer at all.
    async fn send(&self, request: &Outgoing) -> Result<Incoming, String>;

    /// Resolves once `ms` milliseconds have passed.
    async fn elapse(&self, ms: u32);
}

/// The browser's `fetch`, the transport of the running worker.
pub(crate) struct Fetch;

impl Method {
    /// The same method, as gloo-net names it.
    fn gloo(self) -> gloo_net::http::Method {
        match self {
            Self::Get => gloo_net::http::Method::GET,
            Self::Post => gloo_net::http::Method::POST,
            Self::Put => gloo_net::http::Method::PUT,
            Self::Patch => gloo_net::http::Method::PATCH,
            Self::Delete => gloo_net::http::Method::DELETE,
        }
    }
}

impl Transport for Fetch {
    async fn send(&self, request: &Outgoing) -> Result<Incoming, String> {
        // A new signal per request, because a timeout signal starts counting
        // down the moment it is created. gloo-net copies it into the request,
        // so it may be dropped here while `fetch` still holds it.
        let signal = web_sys::AbortSignal::timeout_with_u32(request.timeout_ms);
        let mut builder = gloo_net::http::RequestBuilder::new(&request.url)
            .method(request.method.gloo())
            .abort_signal(Some(&signal));
        if let Some(bearer) = &request.bearer {
            builder = builder.header("Authorization", bearer);
        }
        let sent = match &request.body {
            Some((content_type, bytes)) => {
                builder
                    .header("Content-Type", content_type)
                    .body(bytes.clone())
                    .map_err(|e| format!("body error: {e}"))?
                    .send()
                    .await
            }
            None => builder.send().await,
        };
        let resp = sent.map_err(|e| e.to_string())?;
        let body = resp
            .binary()
            .await
            .map_err(|e| format!("binary error: {e}"))?;
        Ok(Incoming {
            status: resp.status(),
            body,
        })
    }

    async fn elapse(&self, ms: u32) {
        use wasm_bindgen::JsCast;

        let Ok(scope) = js_sys::global().dyn_into::<web_sys::DedicatedWorkerGlobalScope>() else {
            // No timer to wait on: never resolve, and leave the abort signal
            // to end the request.
            return std::future::pending().await;
        };
        let timer = js_sys::Promise::new(&mut |resolve, _reject| {
            let _ = scope.set_timeout_with_callback_and_timeout_and_arguments_0(
                &resolve,
                i32::try_from(ms).unwrap_or(i32::MAX),
            );
        });
        let _ = wasm_bindgen_futures::JsFuture::from(timer).await;
    }
}

/// The relay's error code for a request refused because the account still owes
/// its key setup (HTTP 403). An error message ends with it exactly when the
/// relay refused the request for that reason (`SYNC-134`).
const SETUP_REQUIRED: &str = "SETUP_REQUIRED";

/// The body the relay sends with a refusal. Only its code is read.
#[derive(Deserialize)]
struct RefusalBody {
    error: String,
}

/// The relay's code for a rotation refused because a member joined after it
/// read the members (HTTP 409, `SYNC-143`).
const MEMBERS_CHANGED: &str = "MEMBERS_CHANGED";

/// The relay's code for an invitation refused because a rotation replaced the
/// key it wraps (HTTP 409, `SYNC-143`).
const KEY_CHANGED: &str = "KEY_CHANGED";

/// The relay's code for a membership entry refused because another entry
/// took its place in the log first (HTTP 409, `SRV-104`).
const LOG_MOVED: &str = "LOG_MOVED";

/// The error a refused request reads as. `code` is the relay's error code,
/// kept only with the status that may carry it. A 403 carries a setup still
/// owed, and a 409 a membership or a key that moved under the request.
fn refusal(method: &str, url: &str, status: u16, code: Option<&str>) -> String {
    match (status, code) {
        (403, Some(SETUP_REQUIRED)) => format!("{method} {url}: {SETUP_REQUIRED}"),
        (409, Some(code @ (MEMBERS_CHANGED | KEY_CHANGED | LOG_MOVED))) => {
            format!("{method} {url}: {code}")
        }
        _ => format!("{method} {url}: HTTP {status}"),
    }
}

/// Whether `message` is the error of a request the relay refused for a key
/// setup still owed. The code closes the message, after the URL, so no URL or
/// other status can end with it.
pub(crate) fn is_setup_refusal(message: &str) -> bool {
    message.ends_with(&format!(": {SETUP_REQUIRED}"))
}

/// Whether `message` is a rotation the relay refused because the members
/// changed since it read them.
pub(crate) fn is_members_changed(message: &str) -> bool {
    message.ends_with(&format!(": {MEMBERS_CHANGED}"))
}

/// Whether `message` is an invitation the relay refused because the key it
/// wraps is no longer the document's.
pub(crate) fn is_key_changed(message: &str) -> bool {
    message.ends_with(&format!(": {KEY_CHANGED}"))
}

/// Whether `message` is a membership entry the relay refused because the log
/// moved since the writer read it.
pub(crate) fn is_log_moved(message: &str) -> bool {
    message.ends_with(&format!(": {LOG_MOVED}"))
}

/// The error a refused answer reads as, or `Ok` for a success. The relay's
/// code is read from the body only on the statuses that may carry one.
fn expect_ok(request: &Outgoing, resp: &Incoming) -> Result<(), String> {
    if (200..300).contains(&resp.status) {
        return Ok(());
    }
    let code = if resp.status == 403 || resp.status == 409 {
        serde_json::from_slice::<RefusalBody>(&resp.body)
            .ok()
            .map(|body| body.error)
    } else {
        None
    };
    Err(refusal(
        request.method.as_str(),
        &request.url,
        resp.status,
        code.as_deref(),
    ))
}

/// The JSON body of a successful answer.
fn read_json<R: serde::de::DeserializeOwned>(
    request: &Outgoing,
    resp: &Incoming,
) -> Result<R, String> {
    expect_ok(request, resp)?;
    serde_json::from_slice(&resp.body).map_err(|e| {
        format!(
            "{} {}: parse error: {e}",
            request.method.as_str(),
            request.url
        )
    })
}

/// Sends `request` once. A request that got no answer reads as its method
/// and its URL, then the transport's reason.
async fn exchange(transport: &impl Transport, request: &Outgoing) -> Result<Incoming, String> {
    use futures_util::future::{Either, select};

    // The deadline is raced here, not left to the transport alone, so a
    // request that never answers is abandoned on every transport (`SYNC-071`).
    let send = std::pin::pin!(transport.send(request));
    let deadline = std::pin::pin!(transport.elapse(request.timeout_ms));
    let answer = match select(send, deadline).await {
        Either::Left((answer, _)) => answer,
        Either::Right(((), _)) => Err(format!("no answer within {} ms", request.timeout_ms)),
    };
    answer.map_err(|e| format!("{} {}: {e}", request.method.as_str(), request.url))
}

/// Sends `request` under the session's access token. A 401 renews the pair
/// and replays the request once, with the new token (`SYNC-062`). The
/// answer to that replay is the caller's, whatever it is.
async fn exchange_auth(
    transport: &impl Transport,
    auth: &AuthContext,
    request: &mut Outgoing,
) -> Result<Incoming, String> {
    request.bearer = Some(bearer(&auth.access_token()?));
    let resp = exchange(transport, request).await?;
    if resp.status != 401 {
        return Ok(resp);
    }
    auth.refresh_via(transport).await?;
    request.bearer = Some(bearer(&auth.access_token()?));
    exchange(transport, request).await
}

/// An authenticated GET of JSON, reading `404 Not Found` as `Ok(None)`.
pub(crate) async fn get_json_opt_via<T: serde::de::DeserializeOwned>(
    transport: &impl Transport,
    url: &str,
    auth: &AuthContext,
) -> Result<Option<T>, String> {
    let mut request = Outgoing::new(Method::Get, url);
    let resp = exchange_auth(transport, auth, &mut request).await?;
    if resp.status == 404 {
        return Ok(None);
    }
    read_json(&request, &resp).map(Some)
}

/// An authenticated GET of JSON.
pub(crate) async fn get_json_auth_via<T: serde::de::DeserializeOwned>(
    transport: &impl Transport,
    url: &str,
    auth: &AuthContext,
) -> Result<T, String> {
    let mut request = Outgoing::new(Method::Get, url);
    let resp = exchange_auth(transport, auth, &mut request).await?;
    read_json(&request, &resp)
}

/// A POST of JSON that carries no credential, answered with JSON.
pub(crate) async fn post_json_via<T: Serialize, R: serde::de::DeserializeOwned>(
    transport: &impl Transport,
    url: &str,
    body: &T,
) -> Result<R, String> {
    let request = Outgoing::json(Method::Post, url, body)?;
    let resp = exchange(transport, &request).await?;
    read_json(&request, &resp)
}

/// An authenticated POST of JSON, answered with JSON.
pub(crate) async fn post_json_auth_via<T: Serialize, R: serde::de::DeserializeOwned>(
    transport: &impl Transport,
    url: &str,
    body: &T,
    auth: &AuthContext,
) -> Result<R, String> {
    let mut request = Outgoing::json(Method::Post, url, body)?;
    let resp = exchange_auth(transport, auth, &mut request).await?;
    read_json(&request, &resp)
}

/// An authenticated request carrying `body` as JSON, to a route that answers
/// with no body.
pub(crate) async fn send_json_auth_via<T: Serialize>(
    transport: &impl Transport,
    method: Method,
    url: &str,
    body: &T,
    auth: &AuthContext,
) -> Result<(), String> {
    let mut request = Outgoing::json(method, url, body)?;
    let resp = exchange_auth(transport, auth, &mut request).await?;
    expect_ok(&request, &resp)
}

/// HTTP GET to JSON, treating `404 Not Found` as `Ok(None)`.
pub(crate) async fn get_json_opt_auth<T: serde::de::DeserializeOwned>(
    url: &str,
    auth: AuthContext,
) -> Result<Option<T>, String> {
    get_json_opt_via(&Fetch, url, &auth).await
}

/// HTTP GET to JSON deserialized response.
#[allow(dead_code)]
pub(crate) async fn get_json<T: serde::de::DeserializeOwned>(url: &str) -> Result<T, String> {
    let request = Outgoing::new(Method::Get, url);
    let resp = exchange(&Fetch, &request).await?;
    read_json(&request, &resp)
}

pub(crate) async fn get_json_auth<T: serde::de::DeserializeOwned>(
    url: &str,
    auth: AuthContext,
) -> Result<T, String> {
    get_json_auth_via(&Fetch, url, &auth).await
}

pub(crate) async fn post_json<T: Serialize, R: serde::de::DeserializeOwned>(
    url: &str,
    body: &T,
) -> Result<R, String> {
    post_json_via(&Fetch, url, body).await
}

pub(crate) async fn post_json_auth<T: Serialize, R: serde::de::DeserializeOwned>(
    url: &str,
    body: &T,
    auth: AuthContext,
) -> Result<R, String> {
    post_json_auth_via(&Fetch, url, body, &auth).await
}

/// The sealed blob of one snapshot of `document_id`: a presigned URL from the
/// relay, then the object itself. `None` when either step fails.
pub(crate) async fn download_snapshot(
    url_scheme: &str,
    api_base: &str,
    document_id: ambre_core::DocumentId,
    snapshot_id: &str,
    auth: AuthContext,
) -> Option<Vec<u8>> {
    let url =
        format!("{url_scheme}://{api_base}/api/documents/{document_id}/snapshot/download-url");
    let body = SnapshotDownloadUrlRequest {
        snapshot_id: snapshot_id.to_owned(),
    };
    let resp = post_json_auth::<_, SnapshotDownloadUrlResponse>(&url, &body, auth)
        .await
        .ok()?;
    fetch_bytes(&resp.download_url).await.ok()
}

pub(crate) async fn delete_auth(url: &str, auth: AuthContext) -> Result<(), String> {
    let mut request = Outgoing::new(Method::Delete, url);
    let resp = exchange_auth(&Fetch, &auth, &mut request).await?;
    expect_ok(&request, &resp)
}

/// PUTs a sealed blob to a presigned object-storage URL. It takes what the seal
/// built and nothing else, so no plaintext reaches object storage (`SYNC-049`,
/// `ZK-001`).
#[allow(dead_code)]
pub(crate) async fn put_sealed(
    url: &str,
    blob: &crate::pipeline::SealedBlob,
) -> Result<(), String> {
    put_sealed_via(&Fetch, url, blob).await
}

/// A sealed blob to a presigned object-storage link, with no credential: the
/// link is its own.
pub(crate) async fn put_sealed_via(
    transport: &impl Transport,
    url: &str,
    blob: &crate::pipeline::SealedBlob,
) -> Result<(), String> {
    let request = Outgoing {
        body: Some(("application/octet-stream", blob.as_bytes().to_vec())),
        ..Outgoing::new(Method::Put, url)
    };
    let resp = exchange(transport, &request).await?;
    expect_ok(&request, &resp)
}

pub(crate) async fn patch_json_auth<T: Serialize>(
    url: &str,
    body: &T,
    auth: AuthContext,
) -> Result<(), String> {
    send_json_auth_via(&Fetch, Method::Patch, url, body, &auth).await
}

/// HTTP DELETE carrying a JSON body: a departure sends the entry it signs
/// (`SYNC-132`).
pub(crate) async fn delete_json_auth<T: Serialize>(
    url: &str,
    body: &T,
    auth: AuthContext,
) -> Result<(), String> {
    send_json_auth_via(&Fetch, Method::Delete, url, body, &auth).await
}

/// HTTP PUT of a JSON body, expecting a success with no body to read.
pub(crate) async fn put_json_auth<T: Serialize>(
    url: &str,
    body: &T,
    auth: AuthContext,
) -> Result<(), String> {
    send_json_auth_via(&Fetch, Method::Put, url, body, &auth).await
}

/// HTTP POST of a JSON body, expecting a success with no body to read.
pub(crate) async fn post_json_auth_ok<T: Serialize>(
    url: &str,
    body: &T,
    auth: AuthContext,
) -> Result<(), String> {
    send_json_auth_via(&Fetch, Method::Post, url, body, &auth).await
}

pub(crate) async fn fetch_bytes(url: &str) -> Result<Vec<u8>, String> {
    let request = Outgoing::new(Method::Get, url);
    let resp = exchange(&Fetch, &request).await?;
    expect_ok(&request, &resp)?;
    Ok(resp.body)
}

/// Account-setup payload. Zero-Knowledge invariant (N4): the passphrase NEVER
/// leaves the client. The server only receives the wrapped master key and the
/// Argon2 salt needed to re-derive the unwrap key locally on future unlocks.
#[derive(Serialize)]
#[allow(dead_code)]
pub(crate) struct SetupAccountRequest {
    pub email: String,
    pub display_name: String,
    pub master_key_enc: String,
    pub master_key_salt: String,
    pub signing_pubkey: String,
    pub exchange_pubkey: String,
}

#[derive(Deserialize)]
#[allow(dead_code)]
pub(crate) struct SetupAccountResponse {
    pub user_id: String,
    pub access_token: SecretToken,
    pub refresh_token: SecretToken,
    pub access_expires_in: u64,
}

#[derive(Serialize)]
pub(crate) struct DevLoginRequest {
    pub username: String,
    pub signing_pub_key: String,
    pub exchange_pub_key: String,
}

#[derive(Deserialize)]
pub(crate) struct DevLoginResponse {
    pub user_id: String,
    pub display_name: String,
    pub access_token: SecretToken,
    pub refresh_token: SecretToken,
    pub access_expires_in: u64,
}

/// The relay's answer to `GET /api/auth/{provider}/login`.
#[derive(Deserialize)]
pub(crate) struct ProviderLoginResponse {
    pub url: String,
    pub state: String,
}

/// The provider's return, posted to `/api/auth/{provider}/callback`.
#[derive(Serialize)]
pub(crate) struct ProviderCallbackRequest {
    pub code: String,
    pub state: String,
}

/// The relay's answer to a provider return: who signed in, and their tokens.
#[derive(Deserialize)]
pub(crate) struct ProviderCallbackResponse {
    pub user_id: String,
    pub email: String,
    pub display_name: String,
    pub access_token: SecretToken,
    pub refresh_token: SecretToken,
    pub access_expires_in: u64,
}

#[derive(Deserialize)]
pub(crate) struct MasterKeyResponse {
    pub master_key_enc: String,
    pub master_key_salt: String,
}

impl MasterKeyResponse {
    /// Whether the account still owes its key setup: the relay holds no master
    /// material for it, and sends the fields empty.
    pub fn setup_owed(&self) -> bool {
        self.master_key_enc.is_empty()
    }
}

#[derive(Serialize)]
#[allow(dead_code)]
pub(crate) struct SetupKeyRequest {
    pub master_key_enc: String,
    pub master_key_salt: String,
    pub signing_pubkey: String,
    pub exchange_pubkey: String,
}

#[derive(Deserialize)]
#[allow(dead_code)]
pub(crate) struct StoreMasterKeysResponse {
    pub access_token: SecretToken,
    pub refresh_token: SecretToken,
    pub access_expires_in: u64,
}

#[derive(Deserialize)]
#[allow(dead_code)]
pub(crate) struct AuthMeResponse {
    pub user_id: String,
    pub display_name: String,
    pub email: String,
}

/// `GET /api/channels/{id}/documents`: the documents a channel groups.
#[derive(Deserialize)]
pub(crate) struct ChannelDocumentsResponse {
    pub documents: Vec<ChannelDocumentEntry>,
}

#[derive(Deserialize)]
pub(crate) struct ChannelDocumentEntry {
    pub document_id: String,
    /// `meta`, `segment`, `amendments` or `thread`.
    pub kind: String,
}

/// `GET /api/activity`: per segment and thread of every channel of the
/// caller, whether it holds something past the caller's cursor.
#[derive(Deserialize)]
pub(crate) struct ActivityResponse {
    pub documents: Vec<ActivityEntry>,
}

#[derive(Deserialize)]
pub(crate) struct ActivityEntry {
    pub channel_id: String,
    pub document_id: String,
    pub active: bool,
}

/// `PUT /api/documents/{id}/read-cursor`.
#[derive(Serialize)]
pub(crate) struct ReadCursorRequest {
    pub seq: u64,
}

/// `POST /api/read-cursors/mark-read`: the documents to move to their last
/// number, or every one when absent.
#[derive(Serialize)]
pub(crate) struct MarkReadRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub documents: Option<Vec<String>>,
}

#[derive(Deserialize)]
pub(crate) struct DocumentListResponse {
    pub documents: Vec<DocumentEntry>,
}

#[derive(Deserialize)]
pub(crate) struct DocumentEntry {
    pub id: String,
    pub doc_type: String,
    #[serde(default)]
    pub name_enc: Option<String>,
    pub role: String,
    /// The caller's wrapped copy of the DocumentKey (base64). Unwrapped via ECDH
    /// to decrypt `name_enc` for the launcher list.
    #[serde(default)]
    pub doc_key_enc: String,
    /// Exchange pubkey (hex) of whoever wrapped `doc_key_enc`. Empty => fall back
    /// to our own key (self-wrapped owner row).
    #[serde(default)]
    pub wrapper_exchange_pub_key: String,
    /// Total member count; `> 1` => shared.
    #[serde(default)]
    pub member_count: u32,
    pub updated_at: String,
}

#[derive(Serialize)]
pub(crate) struct CreateDocumentRequest {
    pub doc_type: String,
    pub name_enc: Option<String>,
    pub doc_key_enc: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub document_id: Option<String>,
    /// The first entry of the membership log, as `membership_log::encode`
    /// writes it (`SYNC-132`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entry: Option<String>,
}

#[derive(Deserialize)]
#[allow(dead_code)]
pub(crate) struct CreateDocumentResponse {
    pub document_id: String,
    pub doc_type: String,
}

#[derive(Serialize)]
pub(crate) struct RenameDocumentRequest {
    pub name_enc: String,
}

#[derive(Deserialize)]
pub(crate) struct MembersResponse {
    pub members: Vec<MemberEntry>,
    /// The document owes a rotation: a member left and nobody re-keyed since
    /// (`SRV-122`, `SYNC-128`).
    #[serde(default)]
    pub rotation_owed: bool,
    /// The membership log, oldest entry first (`SRV-104`).
    #[serde(default)]
    pub log: Vec<String>,
}

#[derive(Deserialize)]
#[allow(dead_code)]
pub(crate) struct MemberEntry {
    pub user_id: String,
    pub signing_pub_key: String,
    pub exchange_pub_key: String,
    pub doc_key_enc: String,
    pub role: String,
    /// Exchange public key of whoever wrapped `doc_key_enc`. The ECDH unwrap must
    /// use this peer key, not our own `exchange_pub_key`. Optional for resilience
    /// against older servers; the unwrap falls back to `exchange_pub_key`.
    #[serde(default)]
    pub wrapper_exchange_pub_key: String,
}

/// Response of `GET /api/documents/{id}/snapshot`: the latest snapshot's id and
/// its S3 key (mirrors the server's `SnapshotResponse`).
#[derive(Deserialize)]
pub(crate) struct SnapshotMetadataResponse {
    pub snapshot_id: String,
    #[allow(dead_code)]
    pub s3_key: String,
    /// Highest delta `seq` folded into the snapshot: the open path gates its
    /// `RequestMissing` on this to skip the already-replayed history.
    pub covered_seq: i64,
    /// `confirmed`, or `self` when only its author vouched for it (`SRV-128`).
    #[serde(default)]
    pub status: String,
    /// The author's signing key, hex. A `self` snapshot of this very user opens
    /// as it is.
    #[serde(default)]
    pub author_pubkey: String,
    /// For a `self` snapshot, the latest one another member confirmed, which
    /// the open starts from (`SRV-131`, `AUTHOR-022`).
    #[serde(default)]
    pub reference: Option<ReferenceSnapshot>,
}

/// The reference snapshot named beside a `self` one (`SRV-131`).
#[derive(Deserialize)]
pub(crate) struct ReferenceSnapshot {
    pub snapshot_id: String,
    pub covered_seq: i64,
}

/// Body of `POST /api/documents/{id}/snapshots/{snapshot_id}/report`: the
/// reporter's signature over the report transcript, hex (`AUTHOR-023`).
#[derive(Serialize)]
pub(crate) struct ReportSnapshotRequest {
    pub signature: String,
}

/// Response of `POST /api/documents/{id}/snapshot/upload-url`: a presigned PUT
/// URL plus the S3 key the server will reference when the upload is confirmed.
#[derive(Deserialize)]
pub(crate) struct SnapshotUploadUrlResponse {
    pub upload_url: String,
    pub s3_key: String,
}

/// Body of `POST /api/documents/{id}/snapshot` (confirm upload + insert metadata).
/// Mirrors the server's `CreateSnapshotRequest`.
#[derive(Serialize)]
pub(crate) struct CreateSnapshotRequest {
    pub snapshot_id: String,
    pub author_pubkey: String,
    pub parent_hash: Option<String>,
    pub parent_snapshot_proof: Option<String>,
    pub update_clocks: serde_json::Value,
    pub s3_key: String,
    /// Highest delta `seq` this client had applied when it took the snapshot.
    pub covered_seq: i64,
}

#[derive(Deserialize)]
pub(crate) struct CreateSnapshotResponse {
    pub snapshot_id: String,
}

/// Body of `POST /api/documents/{id}/snapshot/download-url`.
#[derive(Serialize)]
pub(crate) struct SnapshotDownloadUrlRequest {
    pub snapshot_id: String,
}

#[derive(Deserialize)]
pub(crate) struct SnapshotDownloadUrlResponse {
    pub download_url: String,
}

#[derive(Deserialize)]
pub(crate) struct UserSearchResult {
    pub users: Vec<UserInfoEntry>,
}

#[derive(Deserialize)]
pub(crate) struct UserInfoEntry {
    pub id: String,
    pub email: String,
    pub display_name: String,
    /// The key an invitation's entry names for the invitee (`SYNC-132`).
    pub signing_pubkey: String,
    pub exchange_pubkey: String,
}

#[derive(Deserialize)]
#[allow(dead_code)]
pub(crate) struct PubkeysResponse {
    pub signing_pubkey: String,
    pub exchange_pubkey: String,
}

#[derive(Serialize)]
pub(crate) struct InviteRequest {
    pub invitee_email: String,
    pub role: String,
    pub doc_key_enc: String,
    /// The key `doc_key_enc` wraps, in hex (`SYNC-143`).
    pub key_id: String,
    /// The entry the inviter signs for this invitation (`SYNC-132`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entry: Option<String>,
}

/// The body of a departure: the entry the leaver signs (`SYNC-132`).
#[derive(Serialize)]
pub(crate) struct LeaveRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entry: Option<String>,
}

/// The body of a role change: the role and the entry the owner signs for it
/// (`SYNC-132`).
#[derive(Serialize)]
pub(crate) struct RoleChangeRequest {
    pub role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entry: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct InviteResponse {
    pub user_id: String,
    pub display_name: String,
}

#[derive(Serialize)]
pub(crate) struct RotateKeyRequest {
    pub keys: Vec<RotateKeyEntry>,
    /// Every member the rotation read, wrapped for or not (`SYNC-143`).
    pub members: Vec<String>,
    /// The link to the key this rotation replaces, as [`encode_link`] writes
    /// it (`SYNC-118`).
    pub link: String,
}

/// One revoke: the member removed and the new key wrapped for everyone who
/// stays, applied by the relay in one transaction (`SYNC-126`).
#[derive(Serialize)]
pub(crate) struct RevokeRequest {
    pub user_id: String,
    pub keys: Vec<RotateKeyEntry>,
    /// Every member the revoke read and keeps, wrapped for or not (`SYNC-143`).
    pub members: Vec<String>,
    /// The link to the key this revoke replaces (`SYNC-118`).
    pub link: String,
    /// The entry the owner signs for this revoke (`SYNC-132`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entry: Option<String>,
}

/// The links of one document's chain, oldest first (`SYNC-117`).
#[derive(Deserialize)]
pub(crate) struct KeyLinksResponse {
    pub links: Vec<String>,
}

/// The form a link takes in a request: its postcard bytes, in base64.
pub(crate) fn encode_link(link: &ambre_protocol::KeyLink) -> Result<String, String> {
    use base64::Engine as _;
    let bytes = link.encode().map_err(|e| e.to_string())?;
    Ok(base64::engine::general_purpose::URL_SAFE.encode(bytes))
}

/// The links a response carries. One that does not decode is dropped: the
/// walk then stops before it, and the keys past it stay out of reach.
pub(crate) fn decode_links(resp: KeyLinksResponse) -> Vec<ambre_protocol::KeyLink> {
    use base64::Engine as _;
    resp.links
        .iter()
        .filter_map(|text| {
            let bytes = base64::engine::general_purpose::URL_SAFE
                .decode(text)
                .ok()?;
            ambre_protocol::KeyLink::decode(&bytes).ok()
        })
        .collect()
}

/// The links of `document_id`, or none when the relay cannot serve them: the
/// frames sealed under a key older than the cache then stay closed.
pub(crate) async fn fetch_key_links(
    url_scheme: &str,
    api_base: &str,
    document_id: ambre_core::DocumentId,
    auth: AuthContext,
) -> Vec<ambre_protocol::KeyLink> {
    let url = format!("{url_scheme}://{api_base}/api/documents/{document_id}/key-links");
    match get_json_auth::<KeyLinksResponse>(&url, auth).await {
        Ok(resp) => decode_links(resp),
        Err(e) => {
            tracing::warn!("Sync: key links of {document_id:?} unavailable: {e}");
            Vec::new()
        }
    }
}

#[derive(Serialize)]
pub(crate) struct RotateKeyEntry {
    pub user_id: String,
    pub doc_key_enc: String,
}

#[derive(Deserialize)]
#[allow(dead_code)]
pub(crate) struct RotateKeyResponse {
    pub status: String,
}

#[derive(Deserialize)]
pub(crate) struct DocumentMembersResponse {
    pub members: Vec<DocumentMemberEntry>,
    /// The document owes a rotation (`SRV-122`, `SYNC-128`).
    #[serde(default)]
    pub rotation_owed: bool,
    /// The membership log, oldest entry first (`SRV-104`).
    #[serde(default)]
    pub log: Vec<String>,
}

#[derive(Deserialize)]
pub(crate) struct DocumentMemberEntry {
    pub user_id: String,
    pub display_name: String,
    pub email: String,
    /// Ed25519 verifying key: feeds the presence directory (`presence_directory`).
    pub signing_pub_key: String,
    pub exchange_pub_key: String,
    pub doc_key_enc: String,
    pub role: String,
    /// Exchange key of whoever wrapped `doc_key_enc`. Our own row unwraps
    /// against it, which yields the log anchor (`SYNC-104`).
    #[serde(default)]
    pub wrapper_exchange_pub_key: String,
}

/// Asks for a link to put one sealed blob. It names the keyed address and the
/// sealed size, and nothing else of the blob (`SYNC-053`).
#[derive(Serialize)]
pub(crate) struct UploadUrlRequest {
    pub content_hash: String,
    pub size_bytes: u64,
}

#[derive(Deserialize)]
pub(crate) struct UploadUrlResponse {
    pub upload_url: String,
    #[allow(dead_code)]
    pub s3_key: String,
}

#[derive(Serialize)]
pub(crate) struct DownloadUrlRequest {
    pub content_hash: String,
}

#[derive(Deserialize)]
pub(crate) struct DownloadUrlResponse {
    pub download_url: String,
}

/// Confirms one put. The same two fields as [`UploadUrlRequest`] (`SYNC-053`).
#[derive(Serialize)]
pub(crate) struct ConfirmUploadRequest {
    pub content_hash: String,
    pub size_bytes: i64,
}

/// A transport that answers from a script and keeps every request it was
/// handed, for the host tests of the request policy.
#[cfg(test)]
pub(crate) mod scripted {
    use super::*;
    use std::cell::Cell;
    use std::collections::VecDeque;

    /// Answers each request with the next scripted answer. A request past the
    /// script fails at once, or never answers once `silent` is set.
    #[derive(Default)]
    pub(crate) struct Scripted {
        answers: RefCell<VecDeque<Result<Incoming, String>>>,
        /// Every request handed over, in order.
        pub sent: RefCell<Vec<Outgoing>>,
        /// Whether a request past the script hangs instead of failing.
        pub silent: Cell<bool>,
        /// Every deadline a silent transport waited out, in milliseconds.
        pub waited: RefCell<Vec<u32>>,
    }

    impl Scripted {
        /// A transport answering with `answers`, as a status and a JSON body,
        /// or as no answer at all.
        pub(crate) fn new(
            answers: impl IntoIterator<Item = Result<(u16, serde_json::Value), String>>,
        ) -> Self {
            let answers = answers
                .into_iter()
                .map(|answer| {
                    answer.map(|(status, body)| Incoming {
                        status,
                        body: serde_json::to_vec(&body).unwrap(),
                    })
                })
                .collect();
            Self {
                answers: RefCell::new(answers),
                ..Self::default()
            }
        }

        /// The method and the URL of each request handed over.
        pub(crate) fn calls(&self) -> Vec<(Method, String)> {
            self.sent
                .borrow()
                .iter()
                .map(|r| (r.method, r.url.clone()))
                .collect()
        }
    }

    impl Transport for Scripted {
        async fn send(&self, request: &Outgoing) -> Result<Incoming, String> {
            self.sent.borrow_mut().push(request.clone());
            let answer = self.answers.borrow_mut().pop_front();
            match answer {
                Some(answer) => answer,
                None if self.silent.get() => std::future::pending().await,
                None => Err("no answer scripted".to_owned()),
            }
        }

        /// A silent transport's clock jumps straight to the deadline. Any
        /// other never reaches it, so a scripted answer always wins.
        async fn elapse(&self, ms: u32) {
            if !self.silent.get() {
                return std::future::pending().await;
            }
            self.waited.borrow_mut().push(ms);
        }
    }

    /// A session holding the access token `access` and the refresh token
    /// `refresh`, against `https://relay.test`.
    pub(crate) fn session(access: &str, refresh: &str) -> AuthContext {
        AuthContext {
            api_base: "relay.test".to_owned(),
            url_scheme: "https",
            tokens: Rc::new(RefCell::new(Some(TokenPair {
                access_token: access.to_owned().into(),
                refresh_token: refresh.to_owned().into(),
                access_expires_in: 900,
            }))),
        }
    }

    /// A fresh token pair as the relay's refresh route answers it.
    pub(crate) fn renewed(access: &str) -> serde_json::Value {
        serde_json::json!({
            "access_token": access,
            "refresh_token": "refresh-2",
            "access_expires_in": 900,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::scripted::{Scripted, renewed, session};
    use super::*;

    /// The `Authorization` value a request carried, if any.
    fn bearer_of(request: &Outgoing) -> Option<String> {
        request.bearer.as_ref().map(|b| b.as_str().to_owned())
    }

    /// `SYNC-062`: a request refused for an expired token renews the pair
    /// once, with the refresh token alone. It then replays the same request
    /// under the new token. Every request carries the per-request timeout.
    #[tokio::test]
    async fn a_refused_token_is_renewed_and_the_request_replayed_once() {
        let transport = Scripted::new([
            Ok((401, serde_json::json!({}))),
            Ok((200, renewed("access-2"))),
            Ok((200, serde_json::json!({ "ok": 1 }))),
        ]);
        let auth = session("access-1", "refresh-1");
        let url = "https://relay.test/api/documents";

        let answer: serde_json::Value =
            post_json_auth_via(&transport, url, &serde_json::json!({ "n": 7 }), &auth)
                .await
                .unwrap();

        assert_eq!(answer, serde_json::json!({ "ok": 1 }));
        let sent = transport.sent.borrow();
        let [first, renewal, replay] = sent.as_slice() else {
            panic!("expected three requests, got {}", sent.len());
        };
        assert_eq!(bearer_of(first).as_deref(), Some("Bearer access-1"));
        assert_eq!(renewal.url, "https://relay.test/api/auth/refresh");
        assert_eq!(
            bearer_of(renewal),
            None,
            "the renewal carries no access token"
        );
        let asked: serde_json::Value =
            serde_json::from_slice(&renewal.body.as_ref().unwrap().1).unwrap();
        assert_eq!(asked, serde_json::json!({ "refresh_token": "refresh-1" }));
        assert_eq!(bearer_of(replay).as_deref(), Some("Bearer access-2"));
        assert_eq!(
            (replay.method, &replay.url, &replay.body),
            (first.method, &first.url, &first.body)
        );
        assert!(sent.iter().all(|r| r.timeout_ms == REQUEST_TIMEOUT_MS));
    }

    /// `SYNC-062`: the replay is the only one. A second refusal, or a renewal
    /// the relay refuses, ends the request with an error.
    #[tokio::test]
    async fn a_request_is_replayed_at_most_once() {
        let url = "https://relay.test/api/documents";
        let refused_twice = Scripted::new([
            Ok((401, serde_json::json!({}))),
            Ok((200, renewed("access-2"))),
            Ok((401, serde_json::json!({}))),
        ]);
        let auth = session("access-1", "refresh-1");
        let err = get_json_auth_via::<serde_json::Value>(&refused_twice, url, &auth)
            .await
            .unwrap_err();
        assert_eq!(err, format!("GET {url}: HTTP 401"));
        assert_eq!(refused_twice.sent.borrow().len(), 3);

        let renewal_refused = Scripted::new([
            Ok((401, serde_json::json!({}))),
            Ok((401, serde_json::json!({}))),
        ]);
        let auth = session("access-1", "refresh-1");
        let err = get_json_auth_via::<serde_json::Value>(&renewal_refused, url, &auth)
            .await
            .unwrap_err();
        assert!(err.ends_with("/api/auth/refresh: HTTP 401"), "{err}");
        assert_eq!(
            renewal_refused.sent.borrow().len(),
            2,
            "no replay without a token"
        );
    }

    /// `SYNC-071`: a request the relay never answers ends with an error once
    /// its deadline passes. It is sent once, and nothing is replayed.
    #[tokio::test]
    async fn a_request_with_no_answer_is_abandoned_at_its_deadline() {
        let transport = Scripted::default();
        transport.silent.set(true);
        let url = "https://relay.test/api/documents";

        let error = post_json_auth_via::<_, serde_json::Value>(
            &transport,
            url,
            &serde_json::json!({}),
            &session("access-1", "refresh-1"),
        )
        .await
        .unwrap_err();

        assert_eq!(
            error,
            format!("POST {url}: no answer within {REQUEST_TIMEOUT_MS} ms")
        );
        assert_eq!(transport.calls(), [(Method::Post, url.to_owned())]);
        assert_eq!(*transport.waited.borrow(), [REQUEST_TIMEOUT_MS]);
    }

    /// Each answer reads as what the caller acts on. A refusal keeps the
    /// relay's code on the status that carries it, and a 404 is an absence.
    /// A request with no answer names its method and its URL. None of them
    /// renews the token or replays the request (`SYNC-099`).
    #[tokio::test]
    async fn each_answer_maps_to_its_error() {
        let url = "https://relay.test/api/documents/d/members";
        let auth = session("access-1", "refresh-1");
        let body = serde_json::json!({});
        let answer = |status: u16, code: &str| Ok((status, serde_json::json!({ "error": code })));
        let transport = Scripted::new([
            answer(409, MEMBERS_CHANGED),
            answer(403, SETUP_REQUIRED),
            answer(500, SETUP_REQUIRED),
            Ok((404, serde_json::json!({}))),
            Err("network down".to_owned()),
        ]);
        let send = || send_json_auth_via(&transport, Method::Patch, url, &body, &auth);

        assert!(is_members_changed(&send().await.unwrap_err()));
        assert!(is_setup_refusal(&send().await.unwrap_err()));
        assert_eq!(send().await.unwrap_err(), format!("PATCH {url}: HTTP 500"));
        let absent = get_json_opt_via::<serde_json::Value>(&transport, url, &auth).await;
        assert_eq!(absent, Ok(None));
        assert_eq!(
            send().await.unwrap_err(),
            format!("PATCH {url}: network down")
        );
        assert_eq!(transport.calls().len(), 5, "one request per answer");
        assert!(transport.calls().iter().all(|(_, called)| called == url));
    }

    /// Only a 403 carrying the setup code reads as a setup refusal, and no
    /// URL makes another status read as one (`SYNC-134`).
    #[test]
    fn only_a_setup_refusal_reads_as_one() {
        let url = "https://relay.example/api/documents";
        assert!(is_setup_refusal(&refusal(
            "GET",
            url,
            403,
            Some(SETUP_REQUIRED)
        )));
        assert!(!is_setup_refusal(&refusal(
            "GET",
            url,
            403,
            Some("FORBIDDEN")
        )));
        assert!(!is_setup_refusal(&refusal("GET", url, 403, None)));
        assert!(!is_setup_refusal(&refusal(
            "GET",
            url,
            500,
            Some(SETUP_REQUIRED)
        )));
        let crafted = "https://relay.example/api/users/search?q=: SETUP_REQUIRED";
        assert!(!is_setup_refusal(&refusal("GET", crafted, 500, None)));
    }

    /// A 409 keeps the code of a membership or a key that moved, and no other
    /// status or code reads as either (`SYNC-143`).
    #[test]
    fn a_moved_membership_or_key_reads_as_its_own_refusal() {
        let url = "https://relay.example/api/documents/d/revoke";
        assert!(is_members_changed(&refusal(
            "POST",
            url,
            409,
            Some(MEMBERS_CHANGED)
        )));
        assert!(is_key_changed(&refusal(
            "POST",
            url,
            409,
            Some(KEY_CHANGED)
        )));
        assert!(!is_members_changed(&refusal(
            "POST",
            url,
            409,
            Some("CONFLICT")
        )));
        assert!(!is_members_changed(&refusal(
            "POST",
            url,
            500,
            Some(MEMBERS_CHANGED)
        )));
        assert!(!is_key_changed(&refusal(
            "POST",
            url,
            409,
            Some(MEMBERS_CHANGED)
        )));
    }

    /// An unlock finding no master material reads as a setup owed, not as a
    /// corrupted account (`SYNC-134`).
    #[test]
    fn an_account_without_master_material_owes_its_setup() {
        let owed = MasterKeyResponse {
            master_key_enc: String::new(),
            master_key_salt: String::new(),
        };
        assert!(owed.setup_owed());
        let stored = MasterKeyResponse {
            master_key_enc: "c2VhbGVk".into(),
            master_key_salt: "c2FsdA==".into(),
        };
        assert!(!stored.setup_owed());
    }

    /// Both legs of an upload carry the keyed address and the sealed size,
    /// and no other field: no media type, no plaintext hash (`SYNC-053`).
    #[test]
    fn an_upload_names_only_the_address_and_the_size() {
        let fields = |value: serde_json::Value| {
            let mut keys: Vec<String> = value
                .as_object()
                .expect("an object")
                .keys()
                .cloned()
                .collect();
            keys.sort();
            keys
        };
        let expected = ["content_hash", "size_bytes"];
        let ask = UploadUrlRequest {
            content_hash: "ab".repeat(32),
            size_bytes: 64,
        };
        let confirm = ConfirmUploadRequest {
            content_hash: "ab".repeat(32),
            size_bytes: 64,
        };
        assert_eq!(
            fields(serde_json::to_value(&ask).expect("serializes")),
            expected
        );
        assert_eq!(
            fields(serde_json::to_value(&confirm).expect("serializes")),
            expected
        );
    }
}
