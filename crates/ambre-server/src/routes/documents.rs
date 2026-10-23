use axum::extract::{Query, State};
use axum::response::IntoResponse;
use std::collections::HashMap;
use std::sync::Arc;

use ambre_core::Role;

use crate::auth::{self};
use crate::db;
use crate::error::AppError;
use crate::extract::{Json, PathId};
use crate::redis_ops;
use crate::state::AppState;

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
    pub email: ambre_core::Email,
    /// User-supplied display name (N3).
    pub display_name: String,
    pub master_key_enc: ambre_core::WrappedMasterKey,
    pub master_key_salt: ambre_core::MasterKeySalt,
    pub signing_pubkey: ambre_core::SigningKeyHex,
    pub exchange_pubkey: ambre_core::ExchangeKeyHex,
}

/// Bounds the client-supplied identity: a syntactically plausible, length-bounded
/// email and a [`ambre_core::DisplayName`], the trimmed name it answers. Full
/// email verification is out of scope; this only rejects obvious garbage and
/// unbounded input.
fn validate_identity(
    email: &ambre_core::Email,
    display_name: &str,
) -> Result<ambre_core::DisplayName, AppError> {
    let email = email.as_str().trim();
    let email_ok = (3..=254).contains(&email.len())
        && email.contains('@')
        && !email.starts_with('@')
        && !email.ends_with('@');
    if !email_ok {
        return Err(AppError::BadRequest("invalid email".into()));
    }
    ambre_core::DisplayName::new(display_name)
        .map_err(|_| AppError::BadRequest("invalid display_name".into()))
}
/// Response after successful account setup. Carries the freshly minted token
/// pair (kept in client memory) and the newly assigned user ID.
#[derive(serde::Serialize)]
pub struct SetupAccountResponse {
    #[serde(with = "ambre_core::serde_id::user_id")]
    pub user_id: ambre_core::UserId,
    pub access_token: ambre_protocol::ws::AccessToken,
    pub refresh_token: ambre_protocol::ws::RefreshToken,
    pub access_expires_in: ambre_core::Seconds,
}

pub async fn setup_account(
    State(state): State<Arc<AppState>>,
    Json(req): Json<SetupAccountRequest>,
) -> Result<impl IntoResponse, AppError> {
    let display_name = validate_identity(&req.email, &req.display_name)?;
    let mk_enc = req.master_key_enc;
    let mk_salt = req.master_key_salt;
    let (spk, epk) = super::keys::account_keys(&req.signing_pubkey, &req.exchange_pubkey)?;

    let user = db::create_user(
        &state.db,
        &ambre_core::Email::from(req.email.as_str().trim()),
        &display_name,
        &mk_enc,
        &mk_salt,
        &spk,
        &epk,
    )
    .await?;
    let user_id = user.id;
    let signing_pubkey = spk;

    let tokens = auth::issue_session(&state, user_id, Some(signing_pubkey)).await?;
    tracing::debug!(user_id = %user_id, provider = "setup", "login");

    Ok(axum::Json(SetupAccountResponse {
        user_id,
        access_token: tokens.access_token,
        refresh_token: tokens.refresh_token,
        access_expires_in: tokens.access_expires_in,
    }))
}

// -- Me ------------------------------------------------------------

/// Authenticated user profile returned by GET /auth/me.
#[derive(serde::Serialize)]
pub struct MeResponse {
    #[serde(with = "ambre_core::serde_id::user_id")]
    pub user_id: ambre_core::UserId,
    pub email: ambre_core::Email,
    pub display_name: ambre_core::DisplayName,
}

pub async fn me(
    State(state): State<Arc<AppState>>,
    auth: auth::AccountSession,
) -> Result<impl IntoResponse, AppError> {
    let s = auth;
    let u = db::get_user(&state.db, &s.user_id)
        .await?
        .ok_or(AppError::NotFound { resource: "user" })?;
    let uid = u.id;
    Ok(axum::Json(MeResponse {
        user_id: uid,
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
    pub refresh_token: Option<ambre_protocol::ws::RefreshToken>,
}

pub async fn logout(
    State(state): State<Arc<AppState>>,
    auth: auth::AccountSession,
    Json(req): Json<LogoutRequest>,
) -> Result<impl IntoResponse, AppError> {
    if let Some(token) = req.refresh_token.as_ref() {
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
    Ok(super::Done::json(ambre_core::WriteStatus::LoggedOut))
}

// -- List Documents ------------------------------------------------

/// One entry in the document list returned by GET /documents.
/// Contains the encrypted name, the caller's role, and their encrypted
/// copy of the document key.
#[derive(serde::Serialize)]
pub struct DocumentSummaryJson {
    #[serde(with = "ambre_core::serde_id::document_id")]
    pub id: ambre_core::DocumentId,
    #[serde(with = "ambre_core::serde_id::user_id")]
    pub owner_id: ambre_core::UserId,
    #[serde(with = "ambre_core::serde_enums::doc_type")]
    pub doc_type: ambre_core::DocType,
    pub name_enc: Option<ambre_core::SealedName>,
    /// Serialized as its lowercase word.
    pub role: Role,
    pub doc_key_enc: ambre_core::WrappedDocKey,
    /// Exchange public key (hex) of whoever wrapped `doc_key_enc`. The client
    /// ECDH-unwraps the doc key with it to decrypt the name for the launcher.
    /// Empty when unknown (self-wrapped owner row): the client falls back to its
    /// own key.
    pub wrapper_exchange_pub_key: ambre_core::ExchangeKeyHex,
    /// Total member count (routing metadata). `> 1` => shared.
    pub member_count: u32,
    pub updated_at: ambre_core::Rfc3339,
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
    let s = auth;
    let docs = db::list_documents_for_user(&state.db, &s.user_id).await?;
    let json: Vec<_> = docs
        .into_iter()
        .map(|d| {
            let doc_id = d.id;
            let owner_id = d.owner_id;
            DocumentSummaryJson {
                id: doc_id,
                owner_id,
                doc_type: d.doc_type,
                name_enc: d.name_enc,
                role: d.role,
                doc_key_enc: d.doc_key_enc.clone(),
                wrapper_exchange_pub_key: d
                    .wrapper_exchange_pubkey
                    .map(ambre_core::ExchangeKeyHex::encode)
                    .unwrap_or_default(),
                member_count: d.member_count.max(0) as u32,
                updated_at: ambre_core::Rfc3339::formatted(d.updated_at.to_rfc3339()),
            }
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
    #[serde(with = "ambre_core::serde_enums::doc_type")]
    pub doc_type: ambre_core::DocType,
    pub name_enc: Option<ambre_core::SealedName>,
    pub doc_key_enc: ambre_core::WrappedDocKey,
    /// Optional client-generated document id (lazy creation). When present the
    /// server materializes the row under this id idempotently.
    #[serde(default, with = "ambre_core::serde_id::document_id::option")]
    pub document_id: Option<ambre_core::DocumentId>,
    /// The first entry of the membership log, signed by the creator
    /// (`SYNC-132`).
    #[serde(default)]
    pub entry: Option<ambre_core::EncodedLogEntry>,
    /// The document is a Murmure space, which its members list
    /// (`SRV-216`).
    #[serde(default)]
    pub space: bool,
}
#[derive(serde::Serialize)]
pub struct CreateDocumentResponse {
    #[serde(with = "ambre_core::serde_id::document_id")]
    pub document_id: ambre_core::DocumentId,
    #[serde(with = "ambre_core::serde_enums::doc_type")]
    pub doc_type: ambre_core::DocType,
}

pub async fn create_document(
    State(state): State<Arc<AppState>>,
    auth: auth::AuthSession,
    Json(req): Json<CreateDocumentRequest>,
) -> Result<impl IntoResponse, AppError> {
    let s = auth;

    let name_enc = req.name_enc;
    // Only a Murmure document is a space (`SRV-217`).
    if req.space && req.doc_type != ambre_core::DocType::Murmure {
        return Err(AppError::BadRequest("a space is a Murmure document".into()));
    }
    let client_id = req.document_id;
    let (doc, created) = db::create_document(
        &state.db,
        &s.user_id,
        req.doc_type,
        name_enc.as_ref(),
        client_id.as_ref(),
    )
    .await?;
    let doc_id = doc.id;
    // Only the call that actually created the row (or an idempotent retry by the
    // same owner) registers the owner membership. A foreign caller hitting an
    // existing id never gains ownership.
    let owner_uuid = doc.owner_id;
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
        let doc_key_enc = req.doc_key_enc;
        // An owner's retry finds the first entry already stored, and the
        // log is never rewritten (`SYNC-107`).
        let text = req.entry.as_ref().filter(|_| created);
        let expected = super::share::Expected {
            act: ambre_protocol::MembershipAct::Create,
            member: s.user_id,
            role: Some(ambre_core::Role::Owner),
            key: None,
        };
        let entry = super::share::checked_entry(&state, &s, doc_id, text, expected).await?;
        db::add_document_member(
            &state.db,
            &doc_id,
            &s.user_id,
            Role::Owner,
            &doc_key_enc,
            &s.user_id,
            entry.as_ref(),
        )
        .await?;
        // An owner's retry registers the space it may have missed.
        if req.space && doc.doc_type == ambre_core::DocType::Murmure {
            db::register_space(&state.db, &ambre_core::SpaceId::from_document(doc_id)).await?;
        }
    }
    tracing::info!(document_id = %doc_id, owner_id = %s.user_id, doc_type = %req.doc_type, created, "document create");
    Ok(axum::Json(CreateDocumentResponse {
        document_id: doc_id,
        doc_type: req.doc_type,
    }))
}

// -- Get Document --------------------------------------------------

/// Full document details including the calling user's membership info.
#[derive(serde::Serialize)]
pub struct GetDocumentResponse {
    #[serde(with = "ambre_core::serde_id::document_id")]
    pub id: ambre_core::DocumentId,
    #[serde(with = "ambre_core::serde_id::user_id")]
    pub owner_id: ambre_core::UserId,
    #[serde(with = "ambre_core::serde_enums::doc_type")]
    pub doc_type: ambre_core::DocType,
    pub name_enc: Option<ambre_core::SealedName>,
    /// Serialized as its lowercase word.
    pub role: Option<Role>,
    pub doc_key_enc: Option<ambre_core::WrappedDocKey>,
    pub created_at: ambre_core::Rfc3339,
    pub updated_at: ambre_core::Rfc3339,
}

pub async fn get_document(
    State(state): State<Arc<AppState>>,
    auth: auth::AuthSession,
    PathId(doc_id): PathId<ambre_core::DocumentId>,
) -> Result<impl IntoResponse, AppError> {
    // The extractor refuses an anonymous caller before the register is read, so
    // no query runs on behalf of nobody (`SRV-018`).
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
    let is_owner = auth.user_id == doc.owner_id;
    if mem.is_none() && !is_owner {
        return Err(AppError::NotFound {
            resource: "document",
        });
    }
    Ok(axum::Json(GetDocumentResponse {
        id: doc.id,
        owner_id: doc.owner_id,
        doc_type: doc.doc_type,
        name_enc: doc.name_enc,
        role: mem.as_ref().map(|m| m.role),
        doc_key_enc: mem.map(|m| m.doc_key_enc.clone()),
        created_at: ambre_core::Rfc3339::formatted(doc.created_at.to_rfc3339()),
        updated_at: ambre_core::Rfc3339::formatted(doc.updated_at.to_rfc3339()),
    }))
}

// -- Rename / Delete -----------------------------------------------

/// Request body for renaming a document. The name is E2EE encrypted.
#[derive(serde::Deserialize)]
pub struct RenameDocumentRequest {
    pub name_enc: ambre_core::SealedName,
}

pub async fn rename_document(
    State(state): State<Arc<AppState>>,
    auth: auth::AuthSession,
    PathId(doc_id): PathId<ambre_core::DocumentId>,
    Json(req): Json<RenameDocumentRequest>,
) -> Result<impl IntoResponse, AppError> {
    let s = auth;
    let m = db::get_document_member(&state.db, &doc_id, &s.user_id)
        .await?
        .ok_or(AppError::Forbidden)?;
    if !m.role.can_write_document() {
        return Err(AppError::Forbidden);
    }
    let name_enc = req.name_enc;
    db::rename_document(&state.db, &doc_id, &name_enc).await?;
    tracing::debug!(document_id = %doc_id, by = %s.user_id, "document renamed");
    Ok(super::Done::json(ambre_core::WriteStatus::Renamed))
}

pub async fn delete_document(
    State(state): State<Arc<AppState>>,
    auth: auth::AuthSession,
    PathId(doc_id): PathId<ambre_core::DocumentId>,
) -> Result<impl IntoResponse, AppError> {
    let s = auth;
    let m = db::get_document_member(&state.db, &doc_id, &s.user_id)
        .await?
        .ok_or(AppError::Forbidden)?;
    if m.role != Role::Owner {
        return Err(AppError::Forbidden);
    }
    // A delete that fails past its mark still completes: the flusher's pass
    // finishes it (`SRV-165`).
    crate::deletion::delete_document(&state, &doc_id).await?;
    tracing::info!(document_id = %doc_id, by = %s.user_id, "document deleted");
    Ok(super::Done::json(ambre_core::WriteStatus::Deleted))
}

// -- Users ---------------------------------------------------------

/// User info returned by search.
#[derive(serde::Serialize)]
pub struct UserInfoJson {
    #[serde(with = "ambre_core::serde_id::user_id")]
    pub id: ambre_core::UserId,
    pub email: ambre_core::Email,
    /// Whether the email was provider-verified (OIDC) vs self-asserted at
    /// `/auth/setup` (#3). The share UI should flag an unverified recipient for
    /// the TOFU out-of-band key check. Additive field; older clients ignore it.
    pub email_verified: bool,
    pub display_name: ambre_core::DisplayName,
    pub signing_pubkey: ambre_core::SigningKeyHex,
    pub exchange_pubkey: ambre_core::ExchangeKeyHex,
}
#[derive(serde::Serialize)]
pub struct UserSearchResult {
    pub users: Vec<UserInfoJson>,
}
/// Public keys for a user, fetched by the inviter during sharing.
#[derive(serde::Serialize)]
pub struct PubkeysResponse {
    pub signing_pubkey: ambre_core::SigningKeyHex,
    pub exchange_pubkey: ambre_core::ExchangeKeyHex,
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
        .map(|u| {
            let user_id = u.id;
            UserInfoJson {
                id: user_id,
                email: u.email,
                email_verified: u.email_verified,
                display_name: u.display_name,
                signing_pubkey: ambre_core::SigningKeyHex::encode(&u.signing_pubkey),
                exchange_pubkey: ambre_core::ExchangeKeyHex::encode(&u.exchange_pubkey),
            }
        })
        .collect();
    Ok(axum::Json(UserSearchResult { users: json }))
}

pub async fn get_pubkeys(
    State(state): State<Arc<AppState>>,
    auth: auth::AuthSession,
    PathId(uid): PathId<ambre_core::UserId>,
) -> Result<impl IntoResponse, AppError> {
    let _s = auth;
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
        signing_pubkey: ambre_core::SigningKeyHex::encode(signing_pubkey),
        exchange_pubkey: ambre_core::ExchangeKeyHex::encode(exchange_pubkey),
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
    #[serde(with = "ambre_core::serde_id::user_id")]
    pub user_id: ambre_core::UserId,
    pub email: ambre_core::Email,
    /// Whether the member's email was provider-verified (#3). Additive field;
    /// older clients ignore it.
    pub email_verified: bool,
    pub display_name: ambre_core::DisplayName,
    /// Serialized as its lowercase word.
    pub role: Role,
    pub signing_pub_key: ambre_core::SigningKeyHex,
    pub exchange_pub_key: ambre_core::ExchangeKeyHex,
    pub doc_key_enc: ambre_core::WrappedDocKey,
    /// Exchange public key of whoever wrapped `doc_key_enc` (the inviter, or the
    /// member themselves at creation). The recipient unwraps via ECDH against
    /// THIS key, not its own. Falls back to the member's own key for legacy rows.
    pub wrapper_exchange_pub_key: ambre_core::ExchangeKeyHex,
}

pub async fn list_document_members(
    State(state): State<Arc<AppState>>,
    auth: auth::AuthSession,
    PathId(doc_id): PathId<ambre_core::DocumentId>,
) -> Result<impl IntoResponse, AppError> {
    let s = auth;
    // A non-member and an absent document draw the same 404 (`SRV-005`). The
    // client reads that 404 as a document not created yet, and opens it locally.
    // The admin who re-keys a public channel reads whom it wraps for
    // (`SRV-146`).
    if db::get_document_member(&state.db, &doc_id, &s.user_id)
        .await?
        .is_none()
        && !super::space_admin_of_public_meta(&state, &doc_id, &s.user_id).await?
    {
        return Err(AppError::NotFound {
            resource: "document",
        });
    }
    let members = db::list_document_members(&state.db, &doc_id).await?;
    let json: Vec<_> = members
        .into_iter()
        .map(|m| {
            let user_id = m.user_id;
            MemberInfoJson {
                user_id,
                email: m.email,
                email_verified: m.email_verified,
                display_name: m.display_name,
                role: m.role,
                signing_pub_key: ambre_core::SigningKeyHex::encode(&m.signing_pubkey),
                exchange_pub_key: ambre_core::ExchangeKeyHex::encode(&m.exchange_pubkey),
                doc_key_enc: m.doc_key_enc.clone(),
                wrapper_exchange_pub_key: ambre_core::ExchangeKeyHex::encode(
                    m.wrapper_exchange_pubkey
                        .as_ref()
                        .unwrap_or(&m.exchange_pubkey),
                ),
            }
        })
        .collect();
    // A member with write access that reads a debt rotates before it sends a
    // frame (`ambre-spec-key-rotation.md`, `SYNC-128`).
    let rotation_owed = db::rotation_owed(&state.db, &doc_id).await?;
    // The signed entries beside the rows, for a client to check them against
    // (`SRV-104`, `SYNC-108`).
    let log = db::membership_log(&state.db, &doc_id).await?;
    Ok(axum::Json(MembersResponse {
        members: json,
        rotation_owed,
        log,
    }))
}

/// The members of a document, with what a client checks them against.
#[derive(serde::Serialize)]
pub struct MembersResponse {
    pub members: Vec<MemberInfoJson>,
    /// Whether a member with write access must rotate before it sends.
    pub rotation_owed: bool,
    /// The signed membership entries, each in URL-safe base64.
    pub log: Vec<ambre_core::EncodedLogEntry>,
}

/// Request body for changing a member's role.
#[derive(serde::Deserialize)]
pub struct UpdateMemberRoleRequest {
    /// A role word, parsed by the handler: an unknown word answers 409.
    #[serde(with = "ambre_core::serde_enums::role")]
    pub role: ambre_core::Role,
    /// The signed entry that gives the role (`SYNC-132`).
    #[serde(default)]
    pub entry: Option<ambre_core::EncodedLogEntry>,
}

pub async fn update_document_member_role(
    State(state): State<Arc<AppState>>,
    auth: auth::AuthSession,
    PathId((doc_id, target)): PathId<(ambre_core::DocumentId, ambre_core::UserId)>,
    Json(req): Json<UpdateMemberRoleRequest>,
) -> Result<impl IntoResponse, AppError> {
    let s = auth;
    let m = db::get_document_member(&state.db, &doc_id, &s.user_id)
        .await?
        .ok_or(AppError::Forbidden)?;
    if m.role != Role::Owner {
        return Err(AppError::Forbidden);
    }
    if !req.role.is_valid_for_document() {
        return Err(AppError::Conflict {
            detail: "invalid role".into(),
        });
    }
    let role = req.role;
    let expected = super::share::Expected {
        act: ambre_protocol::MembershipAct::ChangeRole,
        member: target,
        role: Some(role),
        key: Some(None),
    };
    let entry =
        super::share::checked_entry(&state, &s, doc_id, req.entry.as_ref(), expected).await?;
    db::update_document_member_role(&state.db, &doc_id, &target, role, entry.as_ref()).await?;
    tracing::info!(document_id = %doc_id, target_user = %target, role = %role, by = %s.user_id, "member role updated");
    crate::redis_ops::members_changed(&state.redis, &doc_id).await;
    Ok(super::Done::json(ambre_core::WriteStatus::RoleUpdated))
}

#[cfg(test)]
mod tests {
    use super::*;

    // -- Identity validation (N3) -------------------------------------------

    #[test]
    fn validate_identity_accepts_plausible_input() {
        assert!(validate_identity(&ambre_core::Email::from("alice@example.com"), "Alice").is_ok());
        assert!(
            validate_identity(&ambre_core::Email::from("  bob@ambre.app  "), "  Bob  ").is_ok()
        );
    }

    #[test]
    fn validate_identity_rejects_bad_email() {
        assert!(validate_identity(&ambre_core::Email::from("no-at-sign"), "Alice").is_err());
        assert!(validate_identity(&ambre_core::Email::from("@example.com"), "Alice").is_err());
        assert!(validate_identity(&ambre_core::Email::from("alice@"), "Alice").is_err());
        assert!(validate_identity(&ambre_core::Email::from(""), "Alice").is_err());
        let long = format!("{}@x.com", "a".repeat(300));
        assert!(validate_identity(&ambre_core::Email::from(long.as_str()), "Alice").is_err());
    }

    #[test]
    fn validate_identity_rejects_bad_display_name() {
        assert!(validate_identity(&ambre_core::Email::from("alice@example.com"), "").is_err());
        assert!(validate_identity(&ambre_core::Email::from("alice@example.com"), "   ").is_err());
        let long = "x".repeat(129);
        assert!(validate_identity(&ambre_core::Email::from("alice@example.com"), &long).is_err());
    }
}
