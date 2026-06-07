use serde::{Deserialize, Serialize};

/// HTTP GET → JSON deserialized response.
pub(crate) async fn get_json<T: serde::de::DeserializeOwned>(url: &str) -> Result<T, String> {
    let resp = gloo_net::http::Request::get(url)
        .send()
        .await
        .map_err(|e| format!("GET {url}: {e}"))?;
    if !resp.ok() {
        return Err(format!("GET {url}: HTTP {}", resp.status()));
    }
    resp.json::<T>()
        .await
        .map_err(|e| format!("GET {url}: parse error: {e}"))
}

pub(crate) async fn post_json<T: Serialize, R: serde::de::DeserializeOwned>(
    url: &str,
    body: &T,
) -> Result<R, String> {
    let body_json =
        serde_json::to_vec(body).map_err(|e| format!("POST {url}: serialize error: {e}"))?;
    let req = gloo_net::http::Request::post(url)
        .header("Content-Type", "application/json")
        .body(body_json)
        .map_err(|e| format!("POST {url}: body error: {e}"))?;
    let resp = req.send()
        .await
        .map_err(|e| format!("POST {url}: {e}"))?;
    if !resp.ok() {
        return Err(format!("POST {url}: HTTP {}", resp.status()));
    }
    resp.json::<R>()
        .await
        .map_err(|e| format!("POST {url}: parse error: {e}"))
}

pub(crate) async fn delete(url: &str) -> Result<(), String> {
    let resp = gloo_net::http::Request::delete(url)
        .send()
        .await
        .map_err(|e| format!("DELETE {url}: {e}"))?;
    if !resp.ok() {
        return Err(format!("DELETE {url}: HTTP {}", resp.status()));
    }
    Ok(())
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

#[allow(dead_code)]
pub(crate) async fn put_bytes(url: &str, data: &[u8], mime_type: &str) -> Result<(), String> {
    let req = gloo_net::http::Request::put(url)
        .header("Content-Type", mime_type)
        .body(data.to_vec())
        .map_err(|e| format!("PUT {url}: body error: {e}"))?;
    let resp = req.send()
        .await
        .map_err(|e| format!("PUT {url}: {e}"))?;
    if !resp.ok() {
        return Err(format!("PUT {url}: HTTP {}", resp.status()));
    }
    Ok(())
}

pub(crate) async fn put_json<T: Serialize>(url: &str, body: &T) -> Result<(), String> {
    let body_json =
        serde_json::to_vec(body).map_err(|e| format!("PUT {url}: serialize error: {e}"))?;
    let req = gloo_net::http::Request::put(url)
        .header("Content-Type", "application/json")
        .body(body_json)
        .map_err(|e| format!("PUT {url}: body error: {e}"))?;
    let resp = req.send()
        .await
        .map_err(|e| format!("PUT {url}: {e}"))?;
    if !resp.ok() {
        return Err(format!("PUT {url}: HTTP {}", resp.status()));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// REST API types (JSON request/response bodies)
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub(crate) struct MasterKeyResponse {
    pub master_key_enc: String,
    pub master_key_salt: String,
}

#[derive(Serialize)]
pub(crate) struct SetupKeyRequest {
    pub master_key_enc: String,
    pub master_key_salt: String,
    pub signing_pub_key: String,
    pub exchange_pub_key: String,
}

#[derive(Deserialize)]
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
    pub document_id: String,
    pub doc_type: u8,
    pub name_enc: String,
    pub role: String,
    pub updated_at: u64,
}

#[derive(Serialize)]
pub(crate) struct CreateDocumentRequest {
    pub doc_type: u8,
    pub name_enc: String,
}

#[derive(Deserialize)]
#[allow(dead_code)]
pub(crate) struct CreateDocumentResponse {
    pub document_id: String,
    pub doc_type: u8,
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

// ── User search ────────────────────────────────────────────────────────────

/// Response from `GET /api/users/search?query=...`.
#[derive(Deserialize)]
pub(crate) struct UserSearchResult {
    pub users: Vec<UserInfoEntry>,
}

/// A single user returned by the user-search endpoint. Includes hex-encoded
/// public keys so the sync thread can perform ECDH wrapping for invites.
#[derive(Deserialize)]
pub(crate) struct UserInfoEntry {
    pub id: String,
    pub email: String,
    pub display_name: String,
    #[allow(dead_code)]
    pub signing_pubkey: String,
    /// Hex-encoded X25519 public key used for document-key wrapping.
    pub exchange_pubkey: String,
}

// ── Pubkeys ────────────────────────────────────────────────────────────────

/// Response from `GET /api/users/{id}/pubkeys` — used to fetch a specific
/// user's public keys during the sharing flow.
#[derive(Deserialize)]
#[allow(dead_code)]
pub(crate) struct PubkeysResponse {
    pub signing_pubkey: String,
    pub exchange_pubkey: String,
}

// ── Sharing ────────────────────────────────────────────────────────────────

/// Request body for `POST /api/documents/{id}/share`.
/// `doc_key_enc` is the DocumentKey wrapped for the invitee via X25519 ECDH,
/// base64 URL-safe encoded (nonce || ciphertext).
#[derive(Serialize)]
pub(crate) struct InviteRequest {
    pub invitee_email: String,
    pub role: String,
    pub doc_key_enc: String,
}

/// Response from a successful `POST /api/documents/{id}/share`.
#[derive(Deserialize)]
pub(crate) struct InviteResponse {
    pub user_id: String,
    pub display_name: String,
}

/// Request body for `POST /api/documents/{id}/rotate-key` — carries the new
/// DocumentKey wrapped for each remaining member after a revocation.
#[derive(Serialize)]
pub(crate) struct RotateKeyRequest {
    pub keys: Vec<RotateKeyEntry>,
}

/// One entry in a [`RotateKeyRequest`]: a user and their re-encrypted
/// DocumentKey (base64 URL-safe, nonce || ciphertext).
#[derive(Serialize)]
pub(crate) struct RotateKeyEntry {
    pub user_id: String,
    pub doc_key_enc: String,
}

/// Response from `POST /api/documents/{id}/rotate-key`.
#[derive(Deserialize)]
#[allow(dead_code)]
pub(crate) struct RotateKeyResponse {
    pub status: String,
}

// ── Document members (decoded) ─────────────────────────────────────────────

/// Response from `GET /api/documents/{id}/members` (extended payload with
/// plaintext display names and emails, already decrypted by the server).
#[derive(Deserialize)]
pub(crate) struct DocumentMembersResponse {
    pub members: Vec<DocumentMemberEntry>,
}

/// A single member entry returned by the list-members endpoint. `doc_key_enc`
/// is the hex-encoded DocumentKey wrapped for this member via X25519 ECDH
/// (nonce || ciphertext). `exchange_pub_key` is the hex-encoded X25519 public
/// key used for key unwrapping.
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

// -- Media ------------------------------------------------------------------

/// Request body for `POST /api/documents/{id}/media/upload-url`: asks the
/// server for a presigned S3 PUT URL. `content_hash` is the BLAKE2b-256 hex
/// digest of the plaintext media bytes. `size_bytes` is the size of the
/// *encrypted* blob (nonce 24 + ciphertext).
#[derive(Serialize)]
pub(crate) struct UploadUrlRequest {
    pub content_hash: String,
    pub mime_type: String,
    pub size_bytes: u64,
}

/// Response from `POST /api/documents/{id}/media/upload-url`. The client PUTs
/// the encrypted blob directly to `upload_url` (no server proxy).
#[derive(Deserialize)]
pub(crate) struct UploadUrlResponse {
    pub upload_url: String,
    #[allow(dead_code)]
    pub s3_key: String,
}

/// Request body for `POST /api/documents/{id}/media/download-url`: asks the
/// server for a presigned S3 GET URL by content hash.
#[derive(Serialize)]
pub(crate) struct DownloadUrlRequest {
    pub content_hash: String,
}

/// Response from `POST /api/documents/{id}/media/download-url`. The client GETs
/// the encrypted blob directly from `download_url`.
#[derive(Deserialize)]
pub(crate) struct DownloadUrlResponse {
    pub download_url: String,
}

/// Request body for `POST /api/documents/{id}/media`: confirms that the
/// upload completed so the server inserts a `media` row.
#[derive(Serialize)]
pub(crate) struct ConfirmUploadRequest {
    pub content_hash: String,
    pub mime_type: String,
    pub size_bytes: i64,
}
