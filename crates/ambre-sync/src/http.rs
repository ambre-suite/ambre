use std::cell::RefCell;
use std::rc::Rc;

use ambre_core::RefusalCode;
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

/// The body the relay sends with a refusal. Only its code is read, a word of
/// one closed set (`SRV-182`). A code this build does not hold fails the read,
/// so the refusal reads as its status alone.
#[derive(Deserialize)]
struct RefusalBody {
    error: RefusalCode,
}

/// The error a refused request reads as. `code` is the relay's error code,
/// kept only with the status that may carry it. A 403 carries a want of
/// access or a setup still owed (`SYNC-134`). A 409 carries a membership, a
/// key or a log that moved under the request (`SYNC-143`, `SRV-104`).
fn refusal(method: &str, url: &str, status: u16, code: Option<RefusalCode>) -> String {
    use RefusalCode::{Forbidden, KeyChanged, LogMoved, MembersChanged, SetupRequired};
    match (status, code) {
        (403, Some(code @ (SetupRequired | Forbidden)))
        | (409, Some(code @ (MembersChanged | KeyChanged | LogMoved))) => {
            format!("{method} {url}: {code}")
        }
        _ => format!("{method} {url}: HTTP {status}"),
    }
}

/// Whether `message` closes with the relay's `code`. The code comes after the
/// URL, so no URL or other status can end with it.
fn ends_with_code(message: &str, code: RefusalCode) -> bool {
    message.ends_with(&format!(": {code}"))
}

/// Whether `message` is the error of a request the relay refused for a key
/// setup still owed.
pub(crate) fn is_setup_refusal(message: &str) -> bool {
    ends_with_code(message, RefusalCode::SetupRequired)
}

/// Whether `message` is the error of a request the relay refused for want of
/// access to what it names.
pub(crate) fn is_forbidden(message: &str) -> bool {
    ends_with_code(message, RefusalCode::Forbidden)
}

/// Whether `message` is a rotation the relay refused because the members
/// changed since it read them.
pub(crate) fn is_members_changed(message: &str) -> bool {
    ends_with_code(message, RefusalCode::MembersChanged)
}

/// Whether `message` is an invitation the relay refused because the key it
/// wraps is no longer the document's.
pub(crate) fn is_key_changed(message: &str) -> bool {
    ends_with_code(message, RefusalCode::KeyChanged)
}

/// Whether `message` is a membership entry the relay refused because the log
/// moved since the writer read it.
pub(crate) fn is_log_moved(message: &str) -> bool {
    ends_with_code(message, RefusalCode::LogMoved)
}

/// Whether `message` is a request the relay refused with a bare 409: the state
/// it names moved past what the request expects.
pub(crate) fn is_conflict(message: &str) -> bool {
    message.ends_with(": HTTP 409")
}

/// Closes the error of an answer that names a word of a closed set this build
/// does not know.
const UNKNOWN_WORD: &str = "UNKNOWN_WORD";

/// Whether `message` is an answer that names a word of a closed set this
/// build does not know, which only a newer relay writes (`SYNC-152`).
pub(crate) fn is_unknown_word(message: &str) -> bool {
    message.ends_with(&format!(": {UNKNOWN_WORD}"))
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
        code,
    ))
}

/// The JSON body of a successful answer.
fn read_json<R: serde::de::DeserializeOwned>(
    request: &Outgoing,
    resp: &Incoming,
) -> Result<R, String> {
    expect_ok(request, resp)?;
    serde_json::from_slice(&resp.body).map_err(|e| {
        let method = request.method.as_str();
        // A word of a closed set this build does not know means the relay is
        // newer. The code closes the message, as a refusal's does (`SYNC-152`).
        if e.is_data() && e.to_string().starts_with("unknown variant `") {
            return format!("{method} {}: parse error: {e}: {UNKNOWN_WORD}", request.url);
        }
        format!("{method} {}: parse error: {e}", request.url)
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
    snapshot_id: ambre_core::SnapshotId,
    auth: AuthContext,
) -> Option<Vec<u8>> {
    let url =
        format!("{url_scheme}://{api_base}/api/documents/{document_id}/snapshot/download-url");
    let body = SnapshotDownloadUrlRequest { snapshot_id };
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
    #[serde(with = "ambre_core::serde_id::user_id")]
    pub user_id: ambre_core::UserId,
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
    #[serde(with = "ambre_core::serde_id::user_id")]
    pub user_id: ambre_core::UserId,
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
    #[serde(with = "ambre_core::serde_id::user_id")]
    pub user_id: ambre_core::UserId,
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
    #[serde(with = "ambre_core::serde_id::user_id")]
    pub user_id: ambre_core::UserId,
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
    #[serde(with = "ambre_core::serde_id::document_id")]
    pub document_id: ambre_core::DocumentId,
    /// `meta`, `segment`, `amendments` or `thread`.
    #[serde(with = "ambre_core::serde_enums::channel_document_kind")]
    pub kind: ambre_core::ChannelDocumentKind,
}

/// `GET /api/spaces/{id}/channels`: the channels the relay holds as public
/// in a space.
#[derive(Deserialize)]
pub(crate) struct SpaceChannelsResponse {
    #[serde(with = "ambre_core::serde_id::channel_id::vec")]
    pub channels: Vec<ambre_core::ChannelId>,
}

/// `GET /api/channels`: every channel the caller is a member of, and every
/// space.
#[derive(Deserialize)]
pub(crate) struct MemberChannelsResponse {
    pub channels: Vec<MemberChannelEntry>,
    #[serde(with = "ambre_core::serde_id::space_id::vec")]
    pub spaces: Vec<ambre_core::SpaceId>,
}

impl MemberChannelsResponse {
    /// The answer as the bus carries it: the channels, then the spaces.
    /// The names stay sealed; the caller opens them.
    pub(crate) fn listing(&self) -> (Vec<ambre_bus::MemberChannel>, Vec<ambre_core::SpaceId>) {
        let channels = self
            .channels
            .iter()
            .map(MemberChannelEntry::member_channel)
            .collect();
        (channels, self.spaces.clone())
    }
}

/// One channel of [`MemberChannelsResponse`]: its space, none for a direct
/// message, and whether the space lists it. It also carries the name sealed
/// beside its meta, with the caller's wrapped key of that meta (`SRV-219`).
#[derive(Deserialize)]
pub(crate) struct MemberChannelEntry {
    #[serde(with = "ambre_core::serde_id::channel_id")]
    pub channel_id: ambre_core::ChannelId,
    #[serde(with = "ambre_core::serde_id::space_id::option")]
    pub space_id: Option<ambre_core::SpaceId>,
    pub public: bool,
    #[serde(with = "ambre_core::serde_id::meta_id")]
    pub meta_id: ambre_core::MetaId,
    /// Base64 of the sealed name, none for a meta never named.
    #[serde(default)]
    pub name_enc: Option<String>,
    /// Base64 of `nonce || ciphertext`.
    pub doc_key_enc: String,
    /// Hex, empty for a self-wrapped row.
    pub wrapper_exchange_pub_key: String,
}

impl MemberChannelEntry {
    /// The entry as the bus carries it. A channel in no space is a direct
    /// message, whatever its flag says.
    pub(crate) fn member_channel(&self) -> ambre_bus::MemberChannel {
        let place = match (self.space_id, self.public) {
            (Some(space), true) => ambre_bus::ChannelPlace::Public(space),
            (Some(space), false) => ambre_bus::ChannelPlace::Private(space),
            (None, _) => ambre_bus::ChannelPlace::Direct,
        };
        ambre_bus::MemberChannel {
            channel_id: self.channel_id,
            place,
        }
    }
}

/// `GET /api/activity`: per segment and thread of every channel of the
/// caller, whether it holds something past the caller's cursor.
#[derive(Deserialize)]
pub(crate) struct ActivityResponse {
    pub documents: Vec<ActivityEntry>,
}

#[derive(Deserialize)]
pub(crate) struct ActivityEntry {
    #[serde(with = "ambre_core::serde_id::channel_id")]
    pub channel_id: ambre_core::ChannelId,
    #[serde(with = "ambre_core::serde_id::document_id")]
    pub document_id: ambre_core::DocumentId,
    pub active: bool,
    /// The caller's own cursor. A relay that predates it answers none, which
    /// reads as nothing read.
    #[serde(default)]
    pub read: ambre_core::Seq,
}

/// `PUT /api/documents/{id}/read-cursor`.
#[derive(Serialize)]
pub(crate) struct ReadCursorRequest {
    pub seq: ambre_core::Seq,
}

/// `POST /api/read-cursors/mark-read`: the documents to move to their last
/// number, or every one when absent.
#[derive(Serialize)]
pub(crate) struct MarkReadRequest {
    #[serde(
        skip_serializing_if = "Option::is_none",
        with = "ambre_core::serde_id::document_id::option_vec"
    )]
    pub documents: Option<Vec<ambre_core::DocumentId>>,
}

#[derive(Deserialize)]
pub(crate) struct DocumentListResponse {
    pub documents: Vec<DocumentEntry>,
}

#[derive(Deserialize)]
pub(crate) struct DocumentEntry {
    #[serde(with = "ambre_core::serde_id::document_id")]
    pub id: ambre_core::DocumentId,
    #[serde(with = "ambre_core::serde_enums::doc_type")]
    pub doc_type: ambre_core::DocType,
    #[serde(default)]
    pub name_enc: Option<String>,
    #[serde(with = "ambre_core::serde_enums::role")]
    pub role: ambre_core::Role,
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
    #[serde(with = "ambre_core::serde_enums::doc_type")]
    pub doc_type: ambre_core::DocType,
    pub name_enc: Option<String>,
    pub doc_key_enc: String,
    #[serde(
        skip_serializing_if = "Option::is_none",
        with = "ambre_core::serde_id::document_id::option"
    )]
    pub document_id: Option<ambre_core::DocumentId>,
    /// The first entry of the membership log, as `membership_log::encode`
    /// writes it (`SYNC-132`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entry: Option<String>,
    /// The document is a Murmure space, which its members list.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub space: bool,
}

/// `POST /api/channels`: a channel of a space, with its first two documents.
#[derive(Serialize)]
pub(crate) struct CreateChannelRequest {
    /// A public channel, which its catalogue lists, or a private one.
    pub public: bool,
    #[serde(with = "ambre_core::serde_id::channel_id")]
    pub channel_id: ambre_core::ChannelId,
    #[serde(with = "ambre_core::serde_id::space_id")]
    pub space_id: ambre_core::SpaceId,
    #[serde(with = "ambre_core::serde_id::meta_id")]
    pub meta_id: ambre_core::MetaId,
    #[serde(with = "ambre_core::serde_id::segment_id")]
    pub segment_id: ambre_core::SegmentId,
}

/// `POST /api/channels/{id}/segments`: close the active segment `closing`
/// into `successor`, a document this user created for it.
#[derive(Serialize)]
pub(crate) struct CloseSegmentRequest {
    #[serde(with = "ambre_core::serde_id::segment_id")]
    pub closing: ambre_core::SegmentId,
    #[serde(with = "ambre_core::serde_id::segment_id")]
    pub successor: ambre_core::SegmentId,
}

/// The active segment after a close. `created` is false when an earlier
/// close registered another successor.
#[derive(Deserialize)]
pub(crate) struct ClosedSegmentResponse {
    #[serde(with = "ambre_core::serde_id::segment_id")]
    pub active: ambre_core::SegmentId,
    pub created: bool,
}

/// `POST /api/channels/{id}/segments/{segment}/amendments`: the caller's new
/// document, to register as the amendments of the locked segment.
#[derive(Serialize)]
pub(crate) struct RegisterAmendmentsRequest {
    #[serde(with = "ambre_core::serde_id::document_id")]
    pub amendments: ambre_core::DocumentId,
}

/// The amendments document of a segment after a registration. `created` is
/// false when an earlier registration linked another.
#[derive(Deserialize)]
pub(crate) struct AmendmentsResponse {
    #[serde(with = "ambre_core::serde_id::document_id")]
    pub amendments: ambre_core::DocumentId,
    pub created: bool,
}

/// `POST /api/channels/{id}/threads`: the caller's new document, to register
/// as the thread of a root message.
#[derive(Serialize)]
pub(crate) struct RegisterThreadRequest {
    #[serde(with = "ambre_core::serde_id::message_id")]
    pub root: ambre_core::MessageId,
    #[serde(with = "ambre_core::serde_id::document_id")]
    pub thread: ambre_core::DocumentId,
}

/// The thread document of a root after a registration. `created` is false
/// when an earlier registration linked another.
#[derive(Deserialize)]
pub(crate) struct ThreadResponse {
    #[serde(with = "ambre_core::serde_id::document_id")]
    pub thread: ambre_core::DocumentId,
    pub created: bool,
}

/// `POST /api/channels/{id}/segments/{segment}/lease`: the lease, or none
/// and when to ask again.
#[derive(Deserialize)]
pub(crate) struct FinalizeLeaseResponse {
    pub lease: Option<GrantedLeaseResponse>,
    /// Milliseconds before asking again, `0` with a lease.
    pub retry_in: u64,
}

/// A granted finalize lease.
#[derive(Deserialize)]
pub(crate) struct GrantedLeaseResponse {
    #[serde(with = "ambre_core::serde_id::lease_id")]
    pub lease_id: ambre_core::LeaseId,
    /// The last delta the final snapshot covers.
    pub horizon: u64,
    /// Milliseconds before the lease expires.
    pub expires_in: u64,
}

/// `POST /api/channels/{id}/segments/{segment}/finalize`: commit the final
/// snapshot under the lease.
#[derive(Serialize)]
pub(crate) struct FinalizeSegmentRequest {
    #[serde(with = "ambre_core::serde_id::lease_id")]
    pub lease_id: ambre_core::LeaseId,
    #[serde(with = "ambre_core::serde_id::snapshot_id")]
    pub snapshot_id: ambre_core::SnapshotId,
}

/// `POST /api/channels/{id}/segments/{segment}/fold-lease`: the fold lease,
/// or none and when to ask again.
#[derive(Deserialize)]
pub(crate) struct FoldLeaseResponse {
    pub lease: Option<GrantedFoldResponse>,
    /// Milliseconds before asking again, `0` with a lease.
    pub retry_in: u64,
}

/// A granted fold lease.
#[derive(Deserialize)]
pub(crate) struct GrantedFoldResponse {
    #[serde(with = "ambre_core::serde_id::lease_id")]
    pub lease_id: ambre_core::LeaseId,
    /// The last delta of the segment the fold snapshot covers.
    pub horizon: u64,
    /// The amendments document to fold.
    #[serde(with = "ambre_core::serde_id::document_id")]
    pub amendments: ambre_core::DocumentId,
    /// The last delta of the amendments the fold holds at least.
    pub amendments_seq: u64,
    /// Milliseconds before the lease expires.
    pub expires_in: u64,
}

/// `POST /api/channels/{id}/segments/{segment}/fold`: commit the fold
/// snapshot under the lease.
#[derive(Serialize)]
pub(crate) struct FoldSegmentRequest {
    #[serde(with = "ambre_core::serde_id::lease_id")]
    pub lease_id: ambre_core::LeaseId,
    #[serde(with = "ambre_core::serde_id::snapshot_id")]
    pub snapshot_id: ambre_core::SnapshotId,
    /// The last delta of the amendments the fold holds.
    pub amendments_seq: u64,
}

/// `POST /api/documents/{id}/snapshots/{snapshot}/confirm`: the amendments
/// document the confirmation erased, when the snapshot folds one in.
#[derive(Deserialize)]
pub(crate) struct ConfirmSnapshotResponse {
    #[serde(with = "ambre_core::serde_id::document_id::option")]
    pub folded: Option<ambre_core::DocumentId>,
}

/// One member of a direct message, other than the caller, and the channel
/// key wrapped for them.
#[derive(Serialize)]
pub(crate) struct DirectMemberJson {
    #[serde(with = "ambre_core::serde_id::user_id")]
    pub user_id: ambre_core::UserId,
    pub doc_key_enc: String,
}

/// `POST /api/channels/direct`: a direct message of the caller and
/// `members`, which the relay creates or finds.
#[derive(Serialize)]
pub(crate) struct CreateDirectMessageRequest {
    #[serde(with = "ambre_core::serde_id::channel_id")]
    pub channel_id: ambre_core::ChannelId,
    #[serde(with = "ambre_core::serde_id::meta_id")]
    pub meta_id: ambre_core::MetaId,
    #[serde(with = "ambre_core::serde_id::segment_id")]
    pub segment_id: ambre_core::SegmentId,
    pub members: Vec<DirectMemberJson>,
    /// The channel key wrapped for the caller.
    pub doc_key_enc: String,
    /// The identifier of the channel key, in hex.
    #[serde(with = "ambre_core::serde_id::key_id")]
    pub key_id: ambre_core::KeyId,
    /// The log of `meta`: its `Create`, one editor `Invite` per member in
    /// the order of `members`, then the caller's own `ChangeRole` to editor.
    pub entries: Vec<String>,
}

/// The direct message a request reached: the one it created, or the one that
/// already held its members.
#[derive(Deserialize)]
pub(crate) struct DirectMessageResponse {
    #[serde(with = "ambre_core::serde_id::channel_id")]
    pub channel_id: ambre_core::ChannelId,
    #[allow(dead_code)]
    pub created: bool,
}

/// `POST /api/channels/{id}/join`: the channel key, wrapped for the caller.
#[derive(Serialize)]
pub(crate) struct JoinChannelRequest {
    pub doc_key_enc: String,
    /// The signed `SelfJoin` entry that extends meta's log, absent when the
    /// log is empty (`SYNC-132`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entry: Option<String>,
}

/// `GET /api/channels/{id}/join`: the end of the log a self-join extends.
#[derive(Deserialize)]
pub(crate) struct JoinHeadResponse {
    /// The document whose log the self-join extends: the channel's meta.
    #[serde(with = "ambre_core::serde_id::meta_id")]
    pub meta_id: ambre_core::MetaId,
    /// The role the relay grants the self-join, which the entry must name.
    pub role: ambre_bus::Role,
    /// The number of entries, the position the self-join takes.
    pub length: u64,
    /// The last entry (base64), absent on an empty log.
    #[serde(default)]
    pub last: Option<String>,
}

#[derive(Deserialize)]
#[allow(dead_code)]
pub(crate) struct CreateDocumentResponse {
    #[serde(with = "ambre_core::serde_id::document_id")]
    pub document_id: ambre_core::DocumentId,
    #[serde(with = "ambre_core::serde_enums::doc_type")]
    pub doc_type: ambre_core::DocType,
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
    #[serde(with = "ambre_core::serde_id::user_id")]
    pub user_id: ambre_core::UserId,
    pub signing_pub_key: String,
    pub exchange_pub_key: String,
    pub doc_key_enc: String,
    #[serde(with = "ambre_core::serde_enums::role")]
    pub role: ambre_core::Role,
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
    #[serde(with = "ambre_core::serde_id::snapshot_id")]
    pub snapshot_id: ambre_core::SnapshotId,
    #[allow(dead_code)]
    pub s3_key: String,
    /// Highest delta `seq` folded into the snapshot: the open path gates its
    /// `RequestMissing` on this to skip the already-replayed history.
    pub covered_seq: i64,
    /// `confirmed`, or `self` when only its author vouched for it (`SRV-128`).
    pub status: ambre_core::SnapshotStatus,
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
    #[serde(with = "ambre_core::serde_id::snapshot_id")]
    pub snapshot_id: ambre_core::SnapshotId,
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
    #[serde(with = "ambre_core::serde_id::snapshot_id")]
    pub snapshot_id: ambre_core::SnapshotId,
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
    #[serde(with = "ambre_core::serde_id::snapshot_id")]
    pub snapshot_id: ambre_core::SnapshotId,
}

/// Body of `POST /api/documents/{id}/snapshot/download-url`.
#[derive(Serialize)]
pub(crate) struct SnapshotDownloadUrlRequest {
    #[serde(with = "ambre_core::serde_id::snapshot_id")]
    pub snapshot_id: ambre_core::SnapshotId,
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
    #[serde(with = "ambre_core::serde_id::user_id")]
    pub id: ambre_core::UserId,
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
    #[serde(with = "ambre_core::serde_enums::role")]
    pub role: ambre_core::Role,
    pub doc_key_enc: String,
    /// The key `doc_key_enc` wraps, in hex (`SYNC-143`).
    #[serde(with = "ambre_core::serde_id::key_id")]
    pub key_id: ambre_core::KeyId,
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
    #[serde(with = "ambre_core::serde_enums::role")]
    pub role: ambre_core::Role,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entry: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct InviteResponse {
    #[serde(with = "ambre_core::serde_id::user_id")]
    pub user_id: ambre_core::UserId,
    pub display_name: String,
}

#[derive(Serialize)]
pub(crate) struct RotateKeyRequest {
    pub keys: Vec<RotateKeyEntry>,
    /// Every member the rotation read, wrapped for or not (`SYNC-143`).
    #[serde(with = "ambre_core::serde_id::user_id::vec")]
    pub members: Vec<ambre_core::UserId>,
    /// The link to the key this rotation replaces, as [`encode_link`] writes
    /// it (`SYNC-118`).
    pub link: String,
}

/// One revoke: the member removed and the new key wrapped for everyone who
/// stays, applied by the relay in one transaction (`SYNC-126`).
#[derive(Serialize)]
pub(crate) struct RevokeRequest {
    #[serde(with = "ambre_core::serde_id::user_id")]
    pub user_id: ambre_core::UserId,
    pub keys: Vec<RotateKeyEntry>,
    /// Every member the revoke read and keeps, wrapped for or not (`SYNC-143`).
    #[serde(with = "ambre_core::serde_id::user_id::vec")]
    pub members: Vec<ambre_core::UserId>,
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
    #[serde(with = "ambre_core::serde_id::user_id")]
    pub user_id: ambre_core::UserId,
    pub doc_key_enc: String,
}

#[derive(Deserialize)]
pub(crate) struct RotateKeyResponse {
    pub status: ambre_core::RotationStatus,
}

/// The answer of a relay write that returns its outcome (`SRV-181`). A field
/// beside the status, such as a confirmed media's hash, is not read.
#[derive(Debug, Deserialize)]
pub(crate) struct WriteAnswer {
    pub status: ambre_core::WriteStatus,
}

impl WriteAnswer {
    /// Whether the relay answered the outcome of the write it was asked.
    /// Another word of the set answers some other write.
    pub(crate) fn expect(self, asked: ambre_core::WriteStatus) -> Result<(), String> {
        if self.status == asked {
            Ok(())
        } else {
            Err(format!("answered {:?}, not {asked:?}", self.status))
        }
    }
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
    #[serde(with = "ambre_core::serde_id::user_id")]
    pub user_id: ambre_core::UserId,
    pub display_name: String,
    pub email: String,
    /// Ed25519 verifying key: feeds the presence directory (`presence_directory`).
    pub signing_pub_key: String,
    pub exchange_pub_key: String,
    pub doc_key_enc: String,
    /// The relay's column, which the signed log overrides (`SYNC-110`). An
    /// unknown word refuses the whole answer when it is read (`SRV-180`).
    #[serde(with = "ambre_core::serde_enums::role")]
    pub role: ambre_core::Role,
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
        let answer = |status: u16, code: RefusalCode| {
            Ok((status, serde_json::json!({ "error": code.as_str() })))
        };
        let transport = Scripted::new([
            answer(409, RefusalCode::MembersChanged),
            answer(403, RefusalCode::SetupRequired),
            answer(500, RefusalCode::SetupRequired),
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

    /// Holds `SRV-181`: a write answer reads the relay's typed body, beside
    /// any other field. Only the outcome of the write asked passes.
    #[test]
    fn a_write_answer_passes_the_outcome_asked_alone() {
        use ambre_core::WriteStatus::{Confirmed, Revoked};
        let read = |body| serde_json::from_value::<WriteAnswer>(body).unwrap();
        let confirmed = serde_json::json!({ "status": "confirmed", "content_hash": "ab" });
        assert_eq!(read(confirmed.clone()).expect(Confirmed), Ok(()));
        let error = read(confirmed).expect(Revoked).unwrap_err();
        assert_eq!(error, "answered Confirmed, not Revoked");
    }

    /// Holds `SYNC-152`: an answer naming a word this build does not know
    /// reads as one. An adapter or a derived enum may refuse it. A malformed
    /// identifier or a missing field does not read as one.
    #[tokio::test]
    async fn only_an_unknown_word_reads_as_one() {
        #[derive(Debug, serde::Deserialize)]
        struct Row {
            #[serde(with = "ambre_core::serde_id::user_id")]
            _user_id: ambre_core::UserId,
            #[serde(with = "ambre_core::serde_enums::role")]
            _role: ambre_core::Role,
            _status: ambre_core::SnapshotStatus,
        }
        let url = "https://relay.test/api/documents/d/members";
        let auth = session("access-1", "refresh-1");
        let user = &ambre_core::UserId::new().to_string();
        let row = |user: &str, role: &str, status: &str| {
            Ok((
                200,
                serde_json::json!({ "_user_id": user, "_role": role, "_status": status }),
            ))
        };
        let transport = Scripted::new([
            row(user, "editor", "self"),
            row(user, "auditor", "self"),
            row(user, "editor", "vouched"),
            row("not-an-id", "editor", "self"),
            Ok((200, serde_json::json!({ "_user_id": user }))),
        ]);
        let read = || get_json_auth_via::<Row>(&transport, url, &auth);

        assert!(read().await.is_ok());
        assert!(is_unknown_word(&read().await.unwrap_err()), "a role");
        assert!(is_unknown_word(&read().await.unwrap_err()), "a status");
        assert!(!is_unknown_word(&read().await.unwrap_err()), "an id");
        assert!(!is_unknown_word(&read().await.unwrap_err()), "a field");
    }

    /// Only a 403 carrying the setup code reads as a setup refusal, and no
    /// URL makes another status read as one (`SYNC-134`).
    #[test]
    fn only_a_setup_refusal_reads_as_one() {
        use RefusalCode::{Forbidden, SetupRequired};
        let url = "https://relay.example/api/documents";
        assert!(is_setup_refusal(&refusal(
            "GET",
            url,
            403,
            Some(SetupRequired)
        )));
        assert!(!is_setup_refusal(&refusal(
            "GET",
            url,
            403,
            Some(Forbidden)
        )));
        assert!(!is_setup_refusal(&refusal("GET", url, 403, None)));
        assert!(!is_setup_refusal(&refusal(
            "GET",
            url,
            500,
            Some(SetupRequired)
        )));
        let crafted = "https://relay.example/api/users/search?q=: SETUP_REQUIRED";
        assert!(!is_setup_refusal(&refusal("GET", crafted, 500, None)));
    }

    /// Only a 403 carrying the forbidden code reads as a want of access. A
    /// setup refusal, a bare 403 and another status do not.
    #[test]
    fn only_a_forbidden_refusal_reads_as_one() {
        use RefusalCode::{Forbidden, SetupRequired};
        let url = "https://relay.example/api/channels/c/documents";
        assert!(is_forbidden(&refusal("GET", url, 403, Some(Forbidden))));
        assert!(!is_forbidden(&refusal(
            "GET",
            url,
            403,
            Some(SetupRequired)
        )));
        assert!(!is_forbidden(&refusal("GET", url, 403, None)));
        assert!(!is_forbidden(&refusal("GET", url, 404, Some(Forbidden))));
    }

    /// A 409 with no code of its own reads as a conflict, and no other status
    /// or code does.
    #[test]
    fn a_bare_409_reads_as_a_conflict() {
        let url = "https://relay.example/api/channels/c/segments/s/lease";
        assert!(is_conflict(&refusal("POST", url, 409, None)));
        assert!(is_conflict(&refusal(
            "POST",
            url,
            409,
            Some(RefusalCode::Conflict)
        )));
        assert!(!is_conflict(&refusal(
            "POST",
            url,
            409,
            Some(RefusalCode::LogMoved)
        )));
        assert!(!is_conflict(&refusal("POST", url, 500, None)));
    }

    /// A 409 keeps the code of a membership or a key that moved, and no other
    /// status or code reads as either (`SYNC-143`).
    #[test]
    fn a_moved_membership_or_key_reads_as_its_own_refusal() {
        use RefusalCode::{Conflict, KeyChanged, LogMoved, MembersChanged};
        let url = "https://relay.example/api/documents/d/revoke";
        assert!(is_members_changed(&refusal(
            "POST",
            url,
            409,
            Some(MembersChanged)
        )));
        assert!(is_key_changed(&refusal("POST", url, 409, Some(KeyChanged))));
        assert!(is_log_moved(&refusal("POST", url, 409, Some(LogMoved))));
        assert!(!is_members_changed(&refusal(
            "POST",
            url,
            409,
            Some(Conflict)
        )));
        assert!(!is_members_changed(&refusal(
            "POST",
            url,
            500,
            Some(MembersChanged)
        )));
        assert!(!is_key_changed(&refusal(
            "POST",
            url,
            409,
            Some(MembersChanged)
        )));
    }

    /// Holds `SRV-182`: Sync reads the refusal's code from its closed set. A
    /// code this build does not hold reads as the bare status, never as a
    /// moved membership.
    #[tokio::test]
    async fn a_refusal_code_outside_the_set_reads_as_its_status() {
        let url = "https://relay.test/api/documents/d/rotate";
        let auth = session("access-1", "refresh-1");
        let body = serde_json::json!({});
        let transport = Scripted::new([
            Ok((
                409,
                serde_json::json!({ "error": "LOG_MOVED", "message": "m" }),
            )),
            Ok((
                409,
                serde_json::json!({ "error": "QUOTA_EXCEEDED", "message": "m" }),
            )),
            Ok((
                409,
                serde_json::json!({ "error": "log_moved", "message": "m" }),
            )),
        ]);
        let send = || send_json_auth_via(&transport, Method::Post, url, &body, &auth);

        assert!(is_log_moved(&send().await.unwrap_err()));
        assert_eq!(send().await.unwrap_err(), format!("POST {url}: HTTP 409"));
        assert_eq!(send().await.unwrap_err(), format!("POST {url}: HTTP 409"));
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
