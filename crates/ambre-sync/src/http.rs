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
