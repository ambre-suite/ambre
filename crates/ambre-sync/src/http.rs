use std::cell::RefCell;
use std::rc::Rc;

use gloo_net::http::Response;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct TokenPair {
    pub access_token: String,
    pub refresh_token: String,
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
    refresh_token: String,
}

impl AuthContext {
    fn access_token(&self) -> Result<String, String> {
        self.tokens
            .borrow()
            .as_ref()
            .map(|t| t.access_token.clone())
            .ok_or_else(|| "auth required: no access token".to_owned())
    }

    fn refresh_token(&self) -> Result<String, String> {
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

fn bearer(token: &str) -> String {
    format!("Bearer {token}")
}

async fn parse_json<T: serde::de::DeserializeOwned>(
    method: &str,
    url: &str,
    resp: Response,
) -> Result<T, String> {
    if !resp.ok() {
        return Err(format!("{method} {url}: HTTP {}", resp.status()));
    }
    resp.json::<T>()
        .await
        .map_err(|e| format!("{method} {url}: parse error: {e}"))
}

async fn expect_ok(method: &str, url: &str, resp: Response) -> Result<(), String> {
    if !resp.ok() {
        return Err(format!("{method} {url}: HTTP {}", resp.status()));
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
        .send()
        .await
        .map_err(|e| format!("GET {url}: {e}"))?;
    if resp.status() == 401 {
        auth.refresh().await?;
        let resp = gloo_net::http::Request::get(url)
            .header("Authorization", &bearer(&auth.access_token()?))
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
    let send = |token: String| {
        gloo_net::http::Request::post(url)
            .header("Content-Type", "application/json")
            .header("Authorization", &bearer(&token))
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
        .send()
        .await
        .map_err(|e| format!("DELETE {url}: {e}"))?;
    if resp.status() == 401 {
        auth.refresh().await?;
        let resp = gloo_net::http::Request::delete(url)
            .header("Authorization", &bearer(&auth.access_token()?))
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
    let send = |token: String| {
        gloo_net::http::Request::patch(url)
            .header("Content-Type", "application/json")
            .header("Authorization", &bearer(&token))
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

#[derive(Serialize)]
#[allow(dead_code)]
pub(crate) struct SetupAccountRequest {
    pub passphrase: String,
    pub master_key_enc: String,
    pub master_key_salt: String,
    pub signing_pubkey: String,
    pub exchange_pubkey: String,
}

#[derive(Deserialize)]
#[allow(dead_code)]
pub(crate) struct SetupAccountResponse {
    pub user_id: String,
    pub access_token: String,
    pub refresh_token: String,
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
    pub access_token: String,
    pub refresh_token: String,
    pub access_expires_in: u64,
}

#[derive(Deserialize)]
pub(crate) struct MasterKeyResponse {
    pub master_key_enc: String,
    pub master_key_salt: String,
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
    pub access_token: String,
    pub refresh_token: String,
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
    #[allow(dead_code)]
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
}

#[derive(Deserialize)]
#[allow(dead_code)]
pub(crate) struct SnapshotPresignedResponse {
    pub url: String,
    pub snapshot_id: String,
}

#[derive(Serialize)]
#[allow(dead_code)]
pub(crate) struct SnapshotMetadataRequest {
    pub snapshot_id: String,
    pub ciphertext_hash: String,
    pub size: u64,
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
    #[allow(dead_code)]
    pub signing_pub_key: String,
    pub exchange_pub_key: String,
    #[allow(dead_code)]
    pub doc_key_enc: String,
    pub role: String,
}

#[derive(Serialize)]
pub(crate) struct UploadUrlRequest {
    pub content_hash: String,
    pub mime_type: String,
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

#[derive(Serialize)]
pub(crate) struct ConfirmUploadRequest {
    pub content_hash: String,
    pub mime_type: String,
    pub size_bytes: i64,
}
