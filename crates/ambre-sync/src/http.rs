use std::cell::RefCell;
use std::rc::Rc;

use gloo_net::http::Response;
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

    pub(crate) async fn refresh(&self) -> Result<(), String> {
        let url = format!("{}://{}/api/auth/refresh", self.url_scheme, self.api_base);
        let body = RefreshRequest {
            refresh_token: self.refresh_token()?,
        };
        let next = post_json::<_, TokenPair>(&url, &body).await?;
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

/// Fresh `AbortSignal` that fires after [`REQUEST_TIMEOUT_MS`], aborting the
/// associated `fetch`. A new signal is minted per request because a timeout
/// signal starts counting down the moment it is created.
///
/// gloo-net copies the signal into the request's `RequestInit` (a JS object) as
/// soon as `abort_signal` is called. The returned value may then be dropped on
/// the Rust side while `fetch` still holds it alive.
fn timeout_signal() -> web_sys::AbortSignal {
    web_sys::AbortSignal::timeout_with_u32(REQUEST_TIMEOUT_MS)
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

/// The error a refused request reads as. `code` is the relay's error code,
/// read only for a 403: the one status that may stand for a setup still owed.
fn refusal(method: &str, url: &str, status: u16, code: Option<&str>) -> String {
    if status == 403 && code == Some(SETUP_REQUIRED) {
        format!("{method} {url}: {SETUP_REQUIRED}")
    } else {
        format!("{method} {url}: HTTP {status}")
    }
}

/// Whether `message` is the error of a request the relay refused for a key
/// setup still owed. The code closes the message, after the URL, so no URL or
/// other status can end with it.
pub(crate) fn is_setup_refusal(message: &str) -> bool {
    message.ends_with(&format!(": {SETUP_REQUIRED}"))
}

async fn refused(method: &str, url: &str, resp: Response) -> String {
    let status = resp.status();
    let code = if status == 403 {
        resp.json::<RefusalBody>().await.ok().map(|body| body.error)
    } else {
        None
    };
    refusal(method, url, status, code.as_deref())
}

async fn parse_json<T: serde::de::DeserializeOwned>(
    method: &str,
    url: &str,
    resp: Response,
) -> Result<T, String> {
    if !resp.ok() {
        return Err(refused(method, url, resp).await);
    }
    resp.json::<T>()
        .await
        .map_err(|e| format!("{method} {url}: parse error: {e}"))
}

async fn expect_ok(method: &str, url: &str, resp: Response) -> Result<(), String> {
    if !resp.ok() {
        return Err(refused(method, url, resp).await);
    }
    Ok(())
}

/// HTTP GET to JSON, treating `404 Not Found` as `Ok(None)`.
pub(crate) async fn get_json_opt_auth<T: serde::de::DeserializeOwned>(
    url: &str,
    auth: AuthContext,
) -> Result<Option<T>, String> {
    let resp = gloo_net::http::Request::get(url)
        .header("Authorization", &bearer(&auth.access_token()?))
        .abort_signal(Some(&timeout_signal()))
        .send()
        .await
        .map_err(|e| format!("GET {url}: {e}"))?;
    if resp.status() == 404 {
        return Ok(None);
    }
    if resp.status() == 401 {
        auth.refresh().await?;
        let resp = gloo_net::http::Request::get(url)
            .header("Authorization", &bearer(&auth.access_token()?))
            .abort_signal(Some(&timeout_signal()))
            .send()
            .await
            .map_err(|e| format!("GET {url}: {e}"))?;
        if resp.status() == 404 {
            return Ok(None);
        }
        return parse_json("GET", url, resp).await.map(Some);
    }
    parse_json("GET", url, resp).await.map(Some)
}

/// HTTP GET to JSON deserialized response.
#[allow(dead_code)]
pub(crate) async fn get_json<T: serde::de::DeserializeOwned>(url: &str) -> Result<T, String> {
    let resp = gloo_net::http::Request::get(url)
        .abort_signal(Some(&timeout_signal()))
        .send()
        .await
        .map_err(|e| format!("GET {url}: {e}"))?;
    parse_json("GET", url, resp).await
}

pub(crate) async fn get_json_auth<T: serde::de::DeserializeOwned>(
    url: &str,
    auth: AuthContext,
) -> Result<T, String> {
    let resp = gloo_net::http::Request::get(url)
        .header("Authorization", &bearer(&auth.access_token()?))
        .abort_signal(Some(&timeout_signal()))
        .send()
        .await
        .map_err(|e| format!("GET {url}: {e}"))?;
    if resp.status() == 401 {
        auth.refresh().await?;
        let resp = gloo_net::http::Request::get(url)
            .header("Authorization", &bearer(&auth.access_token()?))
            .abort_signal(Some(&timeout_signal()))
            .send()
            .await
            .map_err(|e| format!("GET {url}: {e}"))?;
        return parse_json("GET", url, resp).await;
    }
    parse_json("GET", url, resp).await
}

pub(crate) async fn post_json<T: Serialize, R: serde::de::DeserializeOwned>(
    url: &str,
    body: &T,
) -> Result<R, String> {
    let body_json =
        serde_json::to_vec(body).map_err(|e| format!("POST {url}: serialize error: {e}"))?;
    let resp = gloo_net::http::Request::post(url)
        .header("Content-Type", "application/json")
        .abort_signal(Some(&timeout_signal()))
        .body(body_json)
        .map_err(|e| format!("POST {url}: body error: {e}"))?
        .send()
        .await
        .map_err(|e| format!("POST {url}: {e}"))?;
    parse_json("POST", url, resp).await
}

pub(crate) async fn post_json_auth<T: Serialize, R: serde::de::DeserializeOwned>(
    url: &str,
    body: &T,
    auth: AuthContext,
) -> Result<R, String> {
    let body_json =
        serde_json::to_vec(body).map_err(|e| format!("POST {url}: serialize error: {e}"))?;
    let send = |token: SecretToken| {
        gloo_net::http::Request::post(url)
            .header("Content-Type", "application/json")
            .header("Authorization", &bearer(&token))
            .abort_signal(Some(&timeout_signal()))
            .body(body_json.clone())
    };
    let resp = send(auth.access_token()?)
        .map_err(|e| format!("POST {url}: body error: {e}"))?
        .send()
        .await
        .map_err(|e| format!("POST {url}: {e}"))?;
    if resp.status() == 401 {
        auth.refresh().await?;
        let resp = send(auth.access_token()?)
            .map_err(|e| format!("POST {url}: body error: {e}"))?
            .send()
            .await
            .map_err(|e| format!("POST {url}: {e}"))?;
        return parse_json("POST", url, resp).await;
    }
    parse_json("POST", url, resp).await
}

pub(crate) async fn delete_auth(url: &str, auth: AuthContext) -> Result<(), String> {
    let resp = gloo_net::http::Request::delete(url)
        .header("Authorization", &bearer(&auth.access_token()?))
        .abort_signal(Some(&timeout_signal()))
        .send()
        .await
        .map_err(|e| format!("DELETE {url}: {e}"))?;
    if resp.status() == 401 {
        auth.refresh().await?;
        let resp = gloo_net::http::Request::delete(url)
            .header("Authorization", &bearer(&auth.access_token()?))
            .abort_signal(Some(&timeout_signal()))
            .send()
            .await
            .map_err(|e| format!("DELETE {url}: {e}"))?;
        return expect_ok("DELETE", url, resp).await;
    }
    expect_ok("DELETE", url, resp).await
}

#[allow(dead_code)]
pub(crate) async fn put_bytes(url: &str, data: &[u8], mime_type: &str) -> Result<(), String> {
    let resp = gloo_net::http::Request::put(url)
        .header("Content-Type", mime_type)
        .abort_signal(Some(&timeout_signal()))
        .body(data.to_vec())
        .map_err(|e| format!("PUT {url}: body error: {e}"))?
        .send()
        .await
        .map_err(|e| format!("PUT {url}: {e}"))?;
    expect_ok("PUT", url, resp).await
}

pub(crate) async fn patch_json_auth<T: Serialize>(
    url: &str,
    body: &T,
    auth: AuthContext,
) -> Result<(), String> {
    let body_json =
        serde_json::to_vec(body).map_err(|e| format!("PATCH {url}: serialize error: {e}"))?;
    let send = |token: SecretToken| {
        gloo_net::http::Request::patch(url)
            .header("Content-Type", "application/json")
            .header("Authorization", &bearer(&token))
            .abort_signal(Some(&timeout_signal()))
            .body(body_json.clone())
    };
    let resp = send(auth.access_token()?)
        .map_err(|e| format!("PATCH {url}: body error: {e}"))?
        .send()
        .await
        .map_err(|e| format!("PATCH {url}: {e}"))?;
    if resp.status() == 401 {
        auth.refresh().await?;
        let resp = send(auth.access_token()?)
            .map_err(|e| format!("PATCH {url}: body error: {e}"))?
            .send()
            .await
            .map_err(|e| format!("PATCH {url}: {e}"))?;
        return expect_ok("PATCH", url, resp).await;
    }
    expect_ok("PATCH", url, resp).await
}

pub(crate) async fn fetch_bytes(url: &str) -> Result<Vec<u8>, String> {
    let resp = gloo_net::http::Request::get(url)
        .abort_signal(Some(&timeout_signal()))
        .send()
        .await
        .map_err(|e| format!("GET {url}: {e}"))?;
    if !resp.ok() {
        return Err(format!("GET {url}: HTTP {}", resp.status()));
    }
    resp.binary()
        .await
        .map_err(|e| format!("GET {url}: binary error: {e}"))
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
    #[allow(dead_code)]
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
}

#[derive(Deserialize)]
pub(crate) struct InviteResponse {
    pub user_id: String,
    pub display_name: String,
}

#[derive(Serialize)]
pub(crate) struct RotateKeyRequest {
    pub keys: Vec<RotateKeyEntry>,
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
}

#[derive(Deserialize)]
pub(crate) struct DocumentMemberEntry {
    pub user_id: String,
    pub display_name: String,
    pub email: String,
    /// Ed25519 verifying key: feeds the presence directory (`presence_directory`).
    pub signing_pub_key: String,
    pub exchange_pub_key: String,
    #[allow(dead_code)]
    pub doc_key_enc: String,
    pub role: String,
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

#[cfg(test)]
mod tests {
    use super::*;

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
