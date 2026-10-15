use axum::extract::{Json, Path, Query, State};
use axum::response::IntoResponse;
use std::collections::HashMap;
use std::sync::Arc;

use crate::auth::{self};
use crate::db;
use crate::error::AppError;
use crate::redis_ops;
use crate::state::AppState;

fn parse_doc_id(s: &str) -> Result<ambre_core::DocumentId, AppError> {
    ambre_core::Uuid::parse_str(s)
        .map(ambre_core::DocumentId::from_uuid)
        .map_err(|_| AppError::NotFound {
            resource: "document",
        })
}
fn parse_user_id(s: &str) -> Result<ambre_core::UserId, AppError> {
    ambre_core::Uuid::parse_str(s)
        .map(ambre_core::UserId::from_uuid)
        .map_err(|_| AppError::NotFound { resource: "user" })
}

// -- Setup Account -------------------------------------------------

/// Request body for first-time account creation.
///
/// Contains the encrypted master key, Argon2 salt, and public keys
/// derived from the master seed (Ed25519 for signing, X25519 for exchange).
/// Zero-Knowledge invariant (N4): the passphrase is NOT part of this payload
/// and must never be; the server only stores the wrapped key and the salt.
#[derive(serde::Deserialize)]
pub struct SetupAccountRequest {
    /// User-supplied login handle. Recorded verbatim (N3) and resolved by other
    /// members when sharing a document (`get_user_by_email`).
    pub email: String,
    /// User-supplied display name (N3).
    pub display_name: String,
    pub master_key_enc: String,
    pub master_key_salt: String,
    pub signing_pubkey: String,
    pub exchange_pubkey: String,
}

/// Bounds the client-supplied identity: a syntactically plausible, length-bounded
/// email and a non-empty, length-bounded display name. Full email verification is
/// out of scope; this only rejects obvious garbage and unbounded input.
fn validate_identity(email: &str, display_name: &str) -> Result<(), AppError> {
    let email = email.trim();
    let name = display_name.trim();
    let email_ok = (3..=254).contains(&email.len())
        && email.contains('@')
        && !email.starts_with('@')
        && !email.ends_with('@');
    if !email_ok {
        return Err(AppError::BadRequest("invalid email".into()));
    }
    if name.is_empty() || name.chars().count() > 128 {
        return Err(AppError::BadRequest("invalid display_name".into()));
    }
    Ok(())
}
/// Response after successful account setup. Carries the freshly minted token
/// pair (kept in client memory) and the newly assigned user ID.
#[derive(serde::Serialize)]
pub struct SetupAccountResponse {
    pub user_id: String,
    pub access_token: String,
    pub refresh_token: String,
    pub access_expires_in: u64,
}

pub async fn setup_account(
    State(state): State<Arc<AppState>>,
    Json(req): Json<SetupAccountRequest>,
) -> Result<impl IntoResponse, AppError> {
    use base64::Engine;
    validate_identity(&req.email, &req.display_name)?;
    let mk_enc = base64::engine::general_purpose::URL_SAFE
        .decode(&req.master_key_enc)
        .map_err(|e| AppError::Conflict {
            detail: e.to_string(),
        })?;
    let mk_salt = base64::engine::general_purpose::URL_SAFE
        .decode(&req.master_key_salt)
        .map_err(|e| AppError::Conflict {
            detail: e.to_string(),
        })?;
    let spk = hex::decode(&req.signing_pubkey).map_err(|e| AppError::Conflict {
        detail: e.to_string(),
    })?;
    let epk = hex::decode(&req.exchange_pubkey).map_err(|e| AppError::Conflict {
        detail: e.to_string(),
    })?;

    let user = db::create_user(
        &state.db,
        req.email.trim(),
        req.display_name.trim(),
        &mk_enc,
        &mk_salt,
        &spk,
        &epk,
    )
    .await?;
    let user_id = ambre_core::UserId::from_uuid(ambre_core::Uuid::from_bytes(*user.id.as_bytes()));
    let signing_pubkey =
        ambre_core::PublicKey::from_slice(&spk).map_err(|e| AppError::Conflict {
            detail: e.to_string(),
        })?;

    let tokens = auth::issue_session(&state, user_id, Some(signing_pubkey)).await?;
    tracing::debug!(user_id = %user_id, provider = "setup", "login");

    Ok(axum::Json(SetupAccountResponse {
        user_id: user_id.to_string(),
        access_token: tokens.access_token,
        refresh_token: tokens.refresh_token,
        access_expires_in: tokens.access_expires_in,
    }))
}

// -- Me ------------------------------------------------------------

/// Authenticated user profile returned by GET /auth/me.
#[derive(serde::Serialize)]
pub struct MeResponse {
    pub user_id: String,
    pub email: String,
    pub display_name: String,
}

pub async fn me(
    State(state): State<Arc<AppState>>,
    auth: auth::AccountSession,
) -> Result<impl IntoResponse, AppError> {
    let s = auth;
    let u = db::get_user(&state.db, &s.user_id)
        .await?
        .ok_or(AppError::NotFound { resource: "user" })?;
    let uid = ambre_core::UserId::from_uuid(ambre_core::Uuid::from_bytes(*u.id.as_bytes()));
    Ok(axum::Json(MeResponse {
        user_id: uid.to_string(),
        email: u.email,
        display_name: u.display_name,
    }))
}

// -- Logout --------------------------------------------------------

/// Request body for logout: the refresh token to revoke. The access token
/// stays valid until its short `exp`; revoking the refresh prevents renewal.
#[derive(serde::Deserialize)]
pub struct LogoutRequest {
    #[serde(default)]
    pub refresh_token: Option<String>,
}

pub async fn logout(
    State(state): State<Arc<AppState>>,
    auth: auth::AccountSession,
    Json(req): Json<LogoutRequest>,
) -> Result<impl IntoResponse, AppError> {
    if let Some(token) = req.refresh_token.as_deref() {
        // Only revoke a token that actually belongs to the caller. Without this an
        // authenticated user could delete another user's refresh token (session
        // revocation of a third party) by presenting it here. The token is an
        // unguessable UUID so this is defense-in-depth, but logout must act on the
        // caller's own credential only.
        match redis_ops::refresh_get(&state.redis, token).await? {
            Some(data) if data.user_id == auth.user_id => {
                redis_ops::refresh_delete(&state.redis, token).await?;
            }
            _ => {}
        }
    }
    tracing::debug!(user_id = %auth.user_id, "logout");
    Ok(axum::Json(serde_json::json!({ "status": "logged_out" })))
}

// -- List Documents ------------------------------------------------

/// One entry in the document list returned by GET /documents.
/// Contains the encrypted name, the caller's role, and their encrypted
/// copy of the document key.
#[derive(serde::Serialize)]
pub struct DocumentSummaryJson {
    pub id: String,
    pub owner_id: String,
    pub doc_type: String,
    pub name_enc: Option<String>,
    pub role: String,
    pub doc_key_enc: String,
    /// Exchange public key (hex) of whoever wrapped `doc_key_enc`. The client
    /// ECDH-unwraps the doc key with it to decrypt the name for the launcher.
    /// Empty when unknown (self-wrapped owner row): the client falls back to its
    /// own key.
    pub wrapper_exchange_pub_key: String,
    /// Total member count (routing metadata). `> 1` => shared.
    pub member_count: u32,
    pub updated_at: String,
}
/// Response wrapper for the document listing.
#[derive(serde::Serialize)]
pub struct DocumentListResponse {
    pub documents: Vec<DocumentSummaryJson>,
}

pub async fn list_documents(
    State(state): State<Arc<AppState>>,
    auth: auth::AuthSession,
) -> Result<impl IntoResponse, AppError> {
    use base64::Engine;
    let s = auth;
    let docs = db::list_documents_for_user(&state.db, &s.user_id).await?;
    let json: Vec<_> = docs
        .into_iter()
        .map(|d| DocumentSummaryJson {
            id: d.id.to_string(),
            owner_id: d.owner_id.to_string(),
            doc_type: d.doc_type,
            name_enc: d
                .name_enc
                .map(|n| base64::engine::general_purpose::URL_SAFE.encode(&n)),
            role: d.role,
            doc_key_enc: base64::engine::general_purpose::URL_SAFE.encode(&d.doc_key_enc),
            wrapper_exchange_pub_key: d
                .wrapper_exchange_pubkey
                .map(hex::encode)
                .unwrap_or_default(),
            member_count: d.member_count.max(0) as u32,
            updated_at: d.updated_at.to_rfc3339(),
        })
        .collect();
    Ok(axum::Json(DocumentListResponse { documents: json }))
}

// -- Create Document -----------------------------------------------

/// Request body for document creation.
/// `doc_key_enc` is the DocumentKey encrypted for the owner's X25519
/// exchange key, stored server-side for sharing.
#[derive(serde::Deserialize)]
pub struct CreateDocumentRequest {
    pub doc_type: String,
    pub name_enc: Option<String>,
    pub doc_key_enc: String,
    /// Optional client-generated document id (lazy creation). When present the
    /// server materializes the row under this id idempotently.
    #[serde(default)]
    pub document_id: Option<String>,
    /// The first entry of the membership log, signed by the creator
    /// (`SYNC-132`).
    #[serde(default)]
    pub entry: Option<String>,
}
#[derive(serde::Serialize)]
pub struct CreateDocumentResponse {
    pub document_id: String,
    pub doc_type: String,
}

pub async fn create_document(
    State(state): State<Arc<AppState>>,
    auth: auth::AuthSession,
    Json(req): Json<CreateDocumentRequest>,
) -> Result<impl IntoResponse, AppError> {
    use base64::Engine;
    let s = auth;
    if !["prose", "trame", "toile", "murmure"].contains(&req.doc_type.as_str()) {
        return Err(AppError::Conflict {
            detail: "invalid doc_type".into(),
        });
    }

    let name_enc = req
        .name_enc
        .map(|n| {
            base64::engine::general_purpose::URL_SAFE
                .decode(&n)
                .map_err(|e| AppError::Conflict {
                    detail: e.to_string(),
                })
        })
        .transpose()?;
    let client_id = req.document_id.as_deref().map(parse_doc_id).transpose()?;
    let (doc, created) = db::create_document(
        &state.db,
        &s.user_id,
        &req.doc_type,
        name_enc.as_deref(),
        client_id.as_ref(),
    )
    .await?;
    let doc_id =
        ambre_core::DocumentId::from_uuid(ambre_core::Uuid::from_bytes(*doc.id.as_bytes()));
    // Only the call that actually created the row (or an idempotent retry by the
    // same owner) registers the owner membership. A foreign caller hitting an
    // existing id never gains ownership.
    let owner_uuid =
        ambre_core::UserId::from_uuid(ambre_core::Uuid::from_bytes(*doc.owner_id.as_bytes()));
    // A foreign caller who supplied an id that already exists and belongs to
    // someone else must not learn it exists, nor its `doc_type`. Answer 404 as if
    // the id were free-but-unowned-by-them, instead of echoing back the row. The
    // legitimate owner's idempotent retry still succeeds below.
    if !created && owner_uuid != s.user_id {
        return Err(AppError::NotFound {
            resource: "document",
        });
    }
    if created || owner_uuid == s.user_id {
        let doc_key_enc = base64::engine::general_purpose::URL_SAFE
            .decode(&req.doc_key_enc)
            .map_err(|e| AppError::Conflict {
                detail: e.to_string(),
            })?;
        // An owner's retry finds the first entry already stored, and the
        // log is never rewritten (`SYNC-107`).
        let text = req.entry.as_deref().filter(|_| created);
        let expected = super::share::Expected {
            act: ambre_protocol::MembershipAct::Create,
            member: s.user_id,
            role: Some("owner"),
            key: None,
        };
        let entry = super::share::checked_entry(&state, &s, doc_id, text, expected).await?;
        db::add_document_member(
            &state.db,
            &doc_id,
            &s.user_id,
            "owner",
            &doc_key_enc,
            &s.user_id,
            entry.as_ref(),
        )
        .await?;
    }
    tracing::info!(document_id = %doc_id, owner_id = %s.user_id, doc_type = %req.doc_type, created, "document create");
    Ok(axum::Json(CreateDocumentResponse {
        document_id: doc_id.to_string(),
        doc_type: req.doc_type,
    }))
}

// -- Get Document --------------------------------------------------

/// Full document details including the calling user's membership info.
#[derive(serde::Serialize)]
pub struct GetDocumentResponse {
    pub id: String,
    pub owner_id: String,
    pub doc_type: String,
    pub name_enc: Option<String>,
    pub role: Option<String>,
    pub doc_key_enc: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

pub async fn get_document(
    State(state): State<Arc<AppState>>,
    auth: auth::AuthSession,
    Path(document_id): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    use base64::Engine;
    // The extractor refuses an anonymous caller before the register is read, so
    // no query runs on behalf of nobody (`SRV-018`).
    let doc_id = parse_doc_id(&document_id)?;
    let doc = db::get_document(&state.db, &doc_id)
        .await?
        .ok_or(AppError::NotFound {
            resource: "document",
        })?;
    let mem = db::get_document_member(&state.db, &doc_id, &auth.user_id).await?;
    // Non-members get nothing: leaking `owner_id`/`doc_type` (and even the
    // document's existence) to an arbitrary caller who holds only the UUID is an
    // unnecessary metadata disclosure. The owner keeps access even without a
    // `document_members` row (they may have been the creator pre-membership).
    let is_owner = auth.user_id.as_uuid().as_bytes() == doc.owner_id.as_bytes();
    if mem.is_none() && !is_owner {
        return Err(AppError::NotFound {
            resource: "document",
        });
    }
    Ok(axum::Json(GetDocumentResponse {
        id: doc.id.to_string(),
        owner_id: doc.owner_id.to_string(),
        doc_type: doc.doc_type,
        name_enc: doc
            .name_enc
            .map(|n| base64::engine::general_purpose::URL_SAFE.encode(&n)),
        role: mem.as_ref().map(|m| m.role.clone()),
        doc_key_enc: mem.map(|m| base64::engine::general_purpose::URL_SAFE.encode(&m.doc_key_enc)),
        created_at: doc.created_at.to_rfc3339(),
        updated_at: doc.updated_at.to_rfc3339(),
    }))
}

// -- Rename / Delete -----------------------------------------------

/// Request body for renaming a document. The name is E2EE encrypted.
#[derive(serde::Deserialize)]
pub struct RenameDocumentRequest {
    pub name_enc: String,
}

pub async fn rename_document(
    State(state): State<Arc<AppState>>,
    auth: auth::AuthSession,
    Path(document_id): Path<String>,
    Json(req): Json<RenameDocumentRequest>,
) -> Result<impl IntoResponse, AppError> {
    use base64::Engine;
    let s = auth;
    let doc_id = parse_doc_id(&document_id)?;
    let m = db::get_document_member(&state.db, &doc_id, &s.user_id)
        .await?
        .ok_or(AppError::Forbidden)?;
    if !["owner", "editor"].contains(&m.role.as_str()) {
        return Err(AppError::Forbidden);
    }
    let name_enc = base64::engine::general_purpose::URL_SAFE
        .decode(&req.name_enc)
        .map_err(|e| AppError::Conflict {
            detail: e.to_string(),
        })?;
    db::rename_document(&state.db, &doc_id, &name_enc).await?;
    tracing::debug!(document_id = %doc_id, by = %s.user_id, "document renamed");
    Ok(axum::Json(serde_json::json!({"status":"renamed"})))
}

pub async fn delete_document(
    State(state): State<Arc<AppState>>,
    auth: auth::AuthSession,
    Path(document_id): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    let s = auth;
    let doc_id = parse_doc_id(&document_id)?;
    let m = db::get_document_member(&state.db, &doc_id, &s.user_id)
        .await?
        .ok_or(AppError::Forbidden)?;
    if m.role != "owner" {
        return Err(AppError::Forbidden);
    }
    // A delete that fails past its mark still completes: the flusher's pass
    // finishes it (`SRV-165`).
    crate::deletion::delete_document(&state, &doc_id).await?;
    tracing::info!(document_id = %doc_id, by = %s.user_id, "document deleted");
    Ok(axum::Json(serde_json::json!({"status":"deleted"})))
}

// -- Users ---------------------------------------------------------

/// User info returned by search.
#[derive(serde::Serialize)]
pub struct UserInfoJson {
    pub id: String,
    pub email: String,
    /// Whether the email was provider-verified (OIDC) vs self-asserted at
    /// `/auth/setup` (#3). The share UI should flag an unverified recipient for
    /// the TOFU out-of-band key check. Additive field; older clients ignore it.
    pub email_verified: bool,
    pub display_name: String,
    pub signing_pubkey: String,
    pub exchange_pubkey: String,
}
#[derive(serde::Serialize)]
pub struct UserSearchResult {
    pub users: Vec<UserInfoJson>,
}
/// Public keys for a user, fetched by the inviter during sharing.
#[derive(serde::Serialize)]
pub struct PubkeysResponse {
    pub signing_pubkey: String,
    pub exchange_pubkey: String,
}

pub async fn search_users(
    State(state): State<Arc<AppState>>,
    auth: auth::AuthSession,
    Query(params): Query<HashMap<String, String>>,
) -> Result<impl IntoResponse, AppError> {
    let _s = auth;
    let q = params.get("query").map(|s| s.as_str()).unwrap_or("").trim();
    // Require a minimum query length. A 1-char (or empty) `ILIKE '%q%'` returns a
    // near-arbitrary slice of the whole user table, email + pubkeys. It turns the
    // sharing lookup into a directory-enumeration primitive.
    if q.chars().count() < 2 {
        return Ok(axum::Json(UserSearchResult { users: Vec::new() }));
    }
    let users = db::search_users(&state.db, q).await?;
    let json: Vec<_> = users
        .into_iter()
        .map(|u| UserInfoJson {
            id: u.id.to_string(),
            email: u.email,
            email_verified: u.email_verified,
            display_name: u.display_name,
            signing_pubkey: hex::encode(&u.signing_pubkey),
            exchange_pubkey: hex::encode(&u.exchange_pubkey),
        })
        .collect();
    Ok(axum::Json(UserSearchResult { users: json }))
}

pub async fn get_pubkeys(
    State(state): State<Arc<AppState>>,
    auth: auth::AuthSession,
    Path(user_id): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    let _s = auth;
    let uid = parse_user_id(&user_id)?;
    let u = db::get_user(&state.db, &uid)
        .await?
        .ok_or(AppError::NotFound { resource: "user" })?;
    // An account that has not registered its keys has none to share to, and
    // answers like an unknown one (`SRV-163`).
    let (Some(signing_pubkey), Some(exchange_pubkey)) = (u.signing_pubkey, u.exchange_pubkey)
    else {
        return Err(AppError::NotFound { resource: "user" });
    };
    Ok(axum::Json(PubkeysResponse {
        signing_pubkey: hex::encode(signing_pubkey),
        exchange_pubkey: hex::encode(exchange_pubkey),
    }))
}

// -- Members -------------------------------------------------------

/// Document member details for the member list endpoint.
/// Member entry for the member-list endpoint. Public keys are hex-encoded
/// (matching the user-search / pubkeys endpoints); `doc_key_enc` is base64
/// url-safe (matching the document list / create endpoints). A client locates
/// its own entry by `exchange_pub_key` and unwraps `doc_key_enc` via ECDH.
#[derive(serde::Serialize)]
pub struct MemberInfoJson {
    pub user_id: String,
    pub email: String,
    /// Whether the member's email was provider-verified (#3). Additive field;
    /// older clients ignore it.
    pub email_verified: bool,
    pub display_name: String,
    pub role: String,
    pub signing_pub_key: String,
    pub exchange_pub_key: String,
    pub doc_key_enc: String,
    /// Exchange public key of whoever wrapped `doc_key_enc` (the inviter, or the
    /// member themselves at creation). The recipient unwraps via ECDH against
    /// THIS key, not its own. Falls back to the member's own key for legacy rows.
    pub wrapper_exchange_pub_key: String,
}

pub async fn list_document_members(
    State(state): State<Arc<AppState>>,
    auth: auth::AuthSession,
    Path(document_id): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    use base64::Engine;
    let s = auth;
    let doc_id = parse_doc_id(&document_id)?;
    // A non-member and an absent document draw the same 404 (`SRV-005`). The
    // client reads that 404 as a document not created yet, and opens it locally.
    let _m = db::get_document_member(&state.db, &doc_id, &s.user_id)
        .await?
        .ok_or(AppError::NotFound {
            resource: "document",
        })?;
    let members = db::list_document_members(&state.db, &doc_id).await?;
    let json: Vec<_> = members
        .into_iter()
        .map(|m| MemberInfoJson {
            user_id: m.user_id.to_string(),
            email: m.email,
            email_verified: m.email_verified,
            display_name: m.display_name,
            role: m.role,
            signing_pub_key: hex::encode(&m.signing_pubkey),
            exchange_pub_key: hex::encode(&m.exchange_pubkey),
            doc_key_enc: base64::engine::general_purpose::URL_SAFE.encode(&m.doc_key_enc),
            wrapper_exchange_pub_key: hex::encode(
                m.wrapper_exchange_pubkey
                    .as_ref()
                    .unwrap_or(&m.exchange_pubkey),
            ),
        })
        .collect();
    // A member with write access that reads a debt rotates before it sends a
    // frame (`ambre-spec-key-rotation.md`, `SYNC-128`).
    let rotation_owed = db::rotation_owed(&state.db, &doc_id).await?;
    // The signed entries beside the rows, for a client to check them against
    // (`SRV-104`, `SYNC-108`).
    let log: Vec<String> = db::membership_log(&state.db, &doc_id)
        .await?
        .iter()
        .map(|bytes| base64::engine::general_purpose::URL_SAFE.encode(bytes))
        .collect();
    Ok(axum::Json(
        serde_json::json!({"members":json,"rotation_owed":rotation_owed,"log":log}),
    ))
}

/// Request body for changing a member's role.
#[derive(serde::Deserialize)]
pub struct UpdateMemberRoleRequest {
    pub role: String,
    /// The signed entry that gives the role (`SYNC-132`).
    #[serde(default)]
    pub entry: Option<String>,
}

pub async fn update_document_member_role(
    State(state): State<Arc<AppState>>,
    auth: auth::AuthSession,
    Path((document_id, user_id)): Path<(String, String)>,
    Json(req): Json<UpdateMemberRoleRequest>,
) -> Result<impl IntoResponse, AppError> {
    let s = auth;
    let doc_id = parse_doc_id(&document_id)?;
    let target = parse_user_id(&user_id)?;
    let m = db::get_document_member(&state.db, &doc_id, &s.user_id)
        .await?
        .ok_or(AppError::Forbidden)?;
    if m.role != "owner" {
        return Err(AppError::Forbidden);
    }
    if !["owner", "editor", "viewer"].contains(&req.role.as_str()) {
        return Err(AppError::Conflict {
            detail: "invalid role".into(),
        });
    }
    let expected = super::share::Expected {
        act: ambre_protocol::MembershipAct::ChangeRole,
        member: target,
        role: Some(&req.role),
        key: Some(None),
    };
    let entry =
        super::share::checked_entry(&state, &s, doc_id, req.entry.as_deref(), expected).await?;
    db::update_document_member_role(&state.db, &doc_id, &target, &req.role, entry.as_ref()).await?;
    tracing::info!(document_id = %doc_id, target_user = %target, role = %req.role, by = %s.user_id, "member role updated");
    Ok(axum::Json(serde_json::json!({"status":"role_updated"})))
}

#[cfg(test)]
mod tests {
    use super::*;

    // -- Identity validation (N3) -------------------------------------------

    #[test]
    fn validate_identity_accepts_plausible_input() {
        assert!(validate_identity("alice@example.com", "Alice").is_ok());
        assert!(validate_identity("  bob@ambre.app  ", "  Bob  ").is_ok());
    }

    #[test]
    fn validate_identity_rejects_bad_email() {
        assert!(validate_identity("no-at-sign", "Alice").is_err());
        assert!(validate_identity("@example.com", "Alice").is_err());
        assert!(validate_identity("alice@", "Alice").is_err());
        assert!(validate_identity("", "Alice").is_err());
        let long = format!("{}@x.com", "a".repeat(300));
        assert!(validate_identity(&long, "Alice").is_err());
    }

    #[test]
    fn validate_identity_rejects_bad_display_name() {
        assert!(validate_identity("alice@example.com", "").is_err());
        assert!(validate_identity("alice@example.com", "   ").is_err());
        let long = "x".repeat(129);
        assert!(validate_identity("alice@example.com", &long).is_err());
    }
}
