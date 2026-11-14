use axum::extract::{Query, State};
use axum::response::IntoResponse;
use std::sync::Arc;

use ambre_core::id::{Document, Id, Space, User};
use ambre_core::{ExchangePublicKey, FixedBytes, PublicKey, Role};

use crate::auth::{self};
use crate::db;
use crate::error::{AppError, Resource};
use crate::extract::{Json, PathId};
use crate::redis_ops;
use crate::state::AppState;
use ambre_core::Flag as _;
use ambre_core::{Sec, TimeSpan};

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
    /// User-supplied display name (N3). A body whose name breaks the rule
    /// does not decode (`SRV-234`).
    pub display_name: ambre_core::Validated<ambre_core::DisplayName>,
    pub master_key_enc: ambre_core::Encoded<ambre_core::WrappedMasterKey>,
    pub master_key_salt: ambre_core::FixedBytes<ambre_core::MasterKeySalt>,
    /// A key that is not 32 bytes of hexadecimal refuses the body when it is
    /// read, before any row is written (`SRV-232`, `SRV-242`).
    pub signing_pubkey: ambre_core::FixedBytes<ambre_core::PublicKey>,
    pub exchange_pubkey: ambre_core::FixedBytes<ambre_core::ExchangePublicKey>,
}

/// Bounds the client-supplied email: syntactically plausible and
/// length-bounded. Full email verification is out of scope; this only rejects
/// obvious garbage and unbounded input. The display name needs no check here:
/// a `Validated<DisplayName>` holds its rule.
fn validate_email(email: &ambre_core::Email) -> Result<(), AppError> {
    let email = email.as_str().trim();
    let email_ok = (3..=254).contains(&email.len())
        && email.contains('@')
        && !email.starts_with('@')
        && !email.ends_with('@');
    if email_ok {
        Ok(())
    } else {
        Err(AppError::BadRequest("invalid email".into()))
    }
}
/// Response after successful account setup. Carries the freshly minted token
/// pair (kept in client memory) and the newly assigned user ID.
#[derive(serde::Serialize)]
pub struct SetupAccountResponse {
    #[serde(with = "ambre_core::serde_id::text")]
    pub user_id: Id<User>,
    pub access_token: ambre_protocol::ws::Credential<ambre_protocol::ws::AccessToken>,
    pub refresh_token: ambre_protocol::ws::Credential<ambre_protocol::ws::RefreshToken>,
    pub access_expires_in: TimeSpan<Sec>,
}

pub async fn setup_account(
    State(state): State<Arc<AppState>>,
    body: Json<SetupAccountRequest>,
) -> Result<impl IntoResponse, AppError> {
    let req = body.into_inner();
    validate_email(&req.email)?;
    let display_name = req.display_name;
    let mk_enc = req.master_key_enc;
    let mk_salt = req.master_key_salt;
    let (spk, epk) = (req.signing_pubkey, req.exchange_pubkey);

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
    #[serde(with = "ambre_core::serde_id::text")]
    pub user_id: Id<User>,
    pub email: ambre_core::Email,
    pub display_name: ambre_core::Validated<ambre_core::DisplayName>,
}

pub async fn me(
    State(state): State<Arc<AppState>>,
    auth: auth::AccountSession,
) -> Result<impl IntoResponse, AppError> {
    let s = auth;
    let u = db::get_user(&state.db, &s.user_id)
        .await?
        .ok_or(AppError::NotFound {
            resource: Resource::User,
        })?;
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
    pub refresh_token: Option<ambre_protocol::ws::Credential<ambre_protocol::ws::RefreshToken>>,
}

pub async fn logout(
    State(state): State<Arc<AppState>>,
    auth: auth::AccountSession,
    body: Json<LogoutRequest>,
) -> Result<impl IntoResponse, AppError> {
    let req = body.into_inner();
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
    #[serde(with = "ambre_core::serde_id::text")]
    pub id: Id<Document>,
    #[serde(with = "ambre_core::serde_id::text")]
    pub owner_id: Id<User>,
    #[serde(with = "ambre_core::serde_enums::doc_type")]
    pub doc_type: ambre_core::DocType,
    pub name_enc: Option<ambre_core::Encoded<ambre_core::SealedName>>,
    /// Serialized as its lowercase word.
    pub role: Role,
    pub doc_key_enc: ambre_core::Encoded<ambre_core::WrappedDocKey>,
    /// Exchange public key (hex) of whoever wrapped `doc_key_enc`. The client
    /// ECDH-unwraps the doc key with it to decrypt the name for the launcher.
    /// `null` when unknown (self-wrapped owner row): the client falls back to
    /// its own key.
    pub wrapper_exchange_pub_key: Option<FixedBytes<ExchangePublicKey>>,
    /// Total member count (routing metadata). `> 1` => shared.
    pub member_count: ambre_core::Count<ambre_core::Members>,
    /// The relay's clock when it last wrote the row, as a bare number of Unix
    /// milliseconds (`SRV-243`).
    pub updated_at: ambre_core::Instant<ambre_core::Relayed>,
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
                wrapper_exchange_pub_key: d.wrapper_exchange_pubkey,
                member_count: d.member_count,
                updated_at: d.updated_at,
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
    pub name_enc: Option<ambre_core::Encoded<ambre_core::SealedName>>,
    pub doc_key_enc: ambre_core::Encoded<ambre_core::WrappedDocKey>,
    /// Optional client-generated document id (lazy creation). When present the
    /// server materializes the row under this id idempotently.
    #[serde(default, with = "ambre_core::serde_id::text::option")]
    pub document_id: Option<Id<Document>>,
    /// The first entry of the membership log, signed by the creator
    /// (`SYNC-132`).
    #[serde(default)]
    pub entry: Option<ambre_core::Encoded<ambre_core::EncodedLogEntry>>,
    /// The document is a Murmure space, which its members list
    /// (`SRV-216`).
    #[serde(default, with = "ambre_core::serde_enums::space_flag")]
    pub space: ambre_core::DocumentScope,
}
#[derive(serde::Serialize)]
pub struct CreateDocumentResponse {
    #[serde(with = "ambre_core::serde_id::text")]
    pub document_id: Id<Document>,
    #[serde(with = "ambre_core::serde_enums::doc_type")]
    pub doc_type: ambre_core::DocType,
}

pub async fn create_document(
    State(state): State<Arc<AppState>>,
    auth: auth::AuthSession,
    body: Json<CreateDocumentRequest>,
) -> Result<impl IntoResponse, AppError> {
    let req = body.into_inner();
    let s = auth;

    let name_enc = req.name_enc;
    // Only a Murmure document is a space (`SRV-217`).
    if req.space == ambre_core::DocumentScope::Space && req.doc_type != ambre_core::DocType::Murmure
    {
        return Err(AppError::BadRequest("a space is a Murmure document".into()));
    }
    let client_id = req.document_id;
    let (doc, insertion) = db::create_document(
        &state.db,
        &s.user_id,
        req.doc_type,
        name_enc.as_ref(),
        client_id.as_ref(),
    )
    .await?;
    let created = insertion == db::Insertion::Inserted;
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
            resource: Resource::Document,
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
            &db::NewMember {
                user_id: &s.user_id,
                role: Role::Owner,
                doc_key_enc: &doc_key_enc,
                invited_by: &s.user_id,
            },
            entry.as_ref(),
        )
        .await?;
        // An owner's retry registers the space it may have missed.
        if req.space == ambre_core::DocumentScope::Space
            && doc.doc_type == ambre_core::DocType::Murmure
        {
            db::register_space(&state.db, &Id::<Space>::from_document(doc_id)).await?;
        }
    }
    tracing::debug!(document_id = %doc_id, owner_id = %s.user_id, doc_type = %req.doc_type, created, "document create");
    Ok(axum::Json(CreateDocumentResponse {
        document_id: doc_id,
        doc_type: req.doc_type,
    }))
}

// -- Get Document --------------------------------------------------

/// Full document details including the calling user's membership info. It
/// carries no time of the document: no client reads one (`SRV-245`).
#[derive(serde::Serialize)]
pub struct GetDocumentResponse {
    #[serde(with = "ambre_core::serde_id::text")]
    pub id: Id<Document>,
    #[serde(with = "ambre_core::serde_id::text")]
    pub owner_id: Id<User>,
    #[serde(with = "ambre_core::serde_enums::doc_type")]
    pub doc_type: ambre_core::DocType,
    pub name_enc: Option<ambre_core::Encoded<ambre_core::SealedName>>,
    /// Serialized as its lowercase word.
    pub role: Option<Role>,
    pub doc_key_enc: Option<ambre_core::Encoded<ambre_core::WrappedDocKey>>,
}

pub async fn get_document(
    State(state): State<Arc<AppState>>,
    auth: auth::AuthSession,
    ids: PathId<Id<Document>>,
) -> Result<impl IntoResponse, AppError> {
    let doc_id = ids.into_inner();
    // The extractor refuses an anonymous caller before the register is read, so
    // no query runs on behalf of nobody (`SRV-018`).
    let doc = db::get_document(&state.db, &doc_id)
        .await?
        .ok_or(AppError::NotFound {
            resource: Resource::Document,
        })?;
    let mem = db::get_document_member(&state.db, &doc_id, &auth.user_id).await?;
    // Non-members get nothing: leaking `owner_id`/`doc_type` (and even the
    // document's existence) to an arbitrary caller who holds only the UUID is an
    // unnecessary metadata disclosure. The owner keeps access even without a
    // `document_members` row (they may have been the creator pre-membership).
    let is_owner = auth.user_id == doc.owner_id;
    if mem.is_none() && !is_owner {
        return Err(AppError::NotFound {
            resource: Resource::Document,
        });
    }
    Ok(axum::Json(GetDocumentResponse {
        id: doc.id,
        owner_id: doc.owner_id,
        doc_type: doc.doc_type,
        name_enc: doc.name_enc,
        role: mem.as_ref().map(|m| m.role),
        doc_key_enc: mem.map(|m| m.doc_key_enc.clone()),
    }))
}

// -- Rename / Delete -----------------------------------------------

/// Request body for renaming a document. The name is E2EE encrypted.
#[derive(serde::Deserialize)]
pub struct RenameDocumentRequest {
    pub name_enc: ambre_core::Encoded<ambre_core::SealedName>,
}

pub async fn rename_document(
    State(state): State<Arc<AppState>>,
    auth: auth::AuthSession,
    ids: PathId<Id<Document>>,
    body: Json<RenameDocumentRequest>,
) -> Result<impl IntoResponse, AppError> {
    let doc_id = ids.into_inner();
    let req = body.into_inner();
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
    ids: PathId<Id<Document>>,
) -> Result<impl IntoResponse, AppError> {
    let doc_id = ids.into_inner();
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
    tracing::debug!(document_id = %doc_id, by = %s.user_id, "document deleted");
    Ok(super::Done::json(ambre_core::WriteStatus::Deleted))
}

// -- Users ---------------------------------------------------------

/// User info returned by search.
#[derive(serde::Serialize)]
pub struct UserInfoJson {
    #[serde(with = "ambre_core::serde_id::text")]
    pub id: Id<User>,
    pub email: ambre_core::Email,
    /// Whether the email was provider-verified (OIDC) vs self-asserted at
    /// `/auth/setup` (#3). The share UI should flag an unverified recipient for
    /// the TOFU out-of-band key check. Additive field; older clients ignore it.
    pub email_verified: crate::db::EmailProof,
    pub display_name: ambre_core::Validated<ambre_core::DisplayName>,
    pub signing_pubkey: FixedBytes<PublicKey>,
    pub exchange_pubkey: FixedBytes<ExchangePublicKey>,
}
#[derive(serde::Serialize)]
pub struct UserSearchResult {
    pub users: Vec<UserInfoJson>,
}
/// Public keys for a user, fetched by the inviter during sharing.
#[derive(serde::Serialize)]
pub struct PubkeysResponse {
    pub signing_pubkey: FixedBytes<PublicKey>,
    pub exchange_pubkey: FixedBytes<ExchangePublicKey>,
}

/// The query string of `GET /users/search`. A text too short to search reads
/// as no search, and answers no user.
#[derive(serde::Deserialize)]
pub struct SearchUsersParams {
    #[serde(default, deserialize_with = "search_text")]
    query: Option<db::Validated<db::UserSearch>>,
}

/// Reads the `query` parameter through the rule of a [`db::UserSearch`].
fn search_text<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<db::Validated<db::UserSearch>>, D::Error> {
    let text: std::borrow::Cow<'de, str> = serde::Deserialize::deserialize(deserializer)?;
    Ok(db::Validated::<db::UserSearch>::parse(text).ok())
}

pub async fn search_users(
    State(state): State<Arc<AppState>>,
    auth: auth::AuthSession,
    Query(params): Query<SearchUsersParams>,
) -> Result<impl IntoResponse, AppError> {
    let _s = auth;
    let Some(query) = params.query else {
        return Ok(axum::Json(UserSearchResult { users: Vec::new() }));
    };
    let users = db::search_users(&state.db, &query).await?;
    let json: Vec<_> = users
        .into_iter()
        .map(|u| {
            let user_id = u.id;
            UserInfoJson {
                id: user_id,
                email: u.email,
                email_verified: u.email_verified,
                display_name: u.display_name,
                signing_pubkey: u.signing_pubkey,
                exchange_pubkey: u.exchange_pubkey,
            }
        })
        .collect();
    Ok(axum::Json(UserSearchResult { users: json }))
}

pub async fn get_pubkeys(
    State(state): State<Arc<AppState>>,
    auth: auth::AuthSession,
    ids: PathId<Id<User>>,
) -> Result<impl IntoResponse, AppError> {
    let uid = ids.into_inner();
    let _s = auth;
    let u = db::get_user(&state.db, &uid)
        .await?
        .ok_or(AppError::NotFound {
            resource: Resource::User,
        })?;
    // An account that has not registered its keys has none to share to, and
    // answers like an unknown one (`SRV-163`).
    let (Some(signing_pubkey), Some(exchange_pubkey)) = (u.signing_pubkey, u.exchange_pubkey)
    else {
        return Err(AppError::NotFound {
            resource: Resource::User,
        });
    };
    Ok(axum::Json(PubkeysResponse {
        signing_pubkey,
        exchange_pubkey,
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
    #[serde(with = "ambre_core::serde_id::text")]
    pub user_id: Id<User>,
    pub email: ambre_core::Email,
    /// Whether the member's email was provider-verified (#3). Additive field;
    /// older clients ignore it.
    pub email_verified: crate::db::EmailProof,
    pub display_name: ambre_core::Validated<ambre_core::DisplayName>,
    /// Serialized as its lowercase word.
    pub role: Role,
    pub signing_pub_key: FixedBytes<PublicKey>,
    pub exchange_pub_key: FixedBytes<ExchangePublicKey>,
    pub doc_key_enc: ambre_core::Encoded<ambre_core::WrappedDocKey>,
    /// Exchange public key of whoever wrapped `doc_key_enc` (the inviter, or the
    /// member themselves at creation). The recipient unwraps via ECDH against
    /// THIS key, not its own. Falls back to the member's own key for legacy rows.
    pub wrapper_exchange_pub_key: FixedBytes<ExchangePublicKey>,
}

pub async fn list_document_members(
    State(state): State<Arc<AppState>>,
    auth: auth::AuthSession,
    ids: PathId<Id<Document>>,
) -> Result<impl IntoResponse, AppError> {
    let doc_id = ids.into_inner();
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
            resource: Resource::Document,
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
                signing_pub_key: m.signing_pubkey,
                exchange_pub_key: m.exchange_pubkey,
                doc_key_enc: m.doc_key_enc.clone(),
                wrapper_exchange_pub_key: m.wrapper_exchange_pubkey.unwrap_or(m.exchange_pubkey),
            }
        })
        .collect();
    // A member with write access that reads a debt rotates before it sends a
    // frame (`ambre-spec-key-rotation.md`, `SYNC-128`).
    let rotation_owed =
        ambre_core::RotationDebt::from_bool(db::rotation_owed(&state.db, &doc_id).await?);
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
    pub rotation_owed: ambre_core::RotationDebt,
    /// The signed membership entries, each in URL-safe base64.
    pub log: Vec<ambre_core::Encoded<ambre_core::EncodedLogEntry>>,
}

/// Request body for changing a member's role.
#[derive(serde::Deserialize)]
pub struct UpdateMemberRoleRequest {
    /// A role word, parsed by the handler: an unknown word answers 409.
    #[serde(with = "ambre_core::serde_enums::role")]
    pub role: ambre_core::Role,
    /// The signed entry that gives the role (`SYNC-132`).
    #[serde(default)]
    pub entry: Option<ambre_core::Encoded<ambre_core::EncodedLogEntry>>,
}

pub async fn update_document_member_role(
    State(state): State<Arc<AppState>>,
    auth: auth::AuthSession,
    ids: PathId<(Id<Document>, Id<User>)>,
    body: Json<UpdateMemberRoleRequest>,
) -> Result<impl IntoResponse, AppError> {
    let (doc_id, target) = ids.into_inner();
    let req = body.into_inner();
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
    tracing::debug!(document_id = %doc_id, target_user = %target, role = %role, by = %s.user_id, "member role updated");
    super::share::members_changed(&state, &doc_id).await;
    Ok(super::Done::json(ambre_core::WriteStatus::RoleUpdated))
}

#[cfg(test)]
mod tests {
    use super::*;

    // -- Identity validation (N3) -------------------------------------------

    #[test]
    fn validate_email_accepts_plausible_input() {
        assert!(validate_email(&ambre_core::Email::from("alice@example.com")).is_ok());
        assert!(validate_email(&ambre_core::Email::from("  bob@ambre.app  ")).is_ok());
    }

    #[test]
    fn validate_email_rejects_bad_email() {
        assert!(validate_email(&ambre_core::Email::from("no-at-sign")).is_err());
        assert!(validate_email(&ambre_core::Email::from("@example.com")).is_err());
        assert!(validate_email(&ambre_core::Email::from("alice@")).is_err());
        assert!(validate_email(&ambre_core::Email::from("")).is_err());
        let long = format!("{}@x.com", "a".repeat(300));
        assert!(validate_email(&ambre_core::Email::from(long.as_str())).is_err());
    }

    /// A setup body whose display name is blank or past the cap does not
    /// decode, so no route reads it (`SRV-234`).
    #[test]
    fn a_setup_body_with_a_bad_display_name_does_not_decode() {
        let body = |name: &str| {
            serde_json::json!({
                "email": "alice@example.com",
                "display_name": name,
                "master_key_enc": "AQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEB",
                "master_key_salt": hex::encode([2u8; 32]),
                "signing_pubkey": hex::encode([3u8; 32]),
                "exchange_pubkey": hex::encode([4u8; 32]),
            })
        };
        let decode = |name: &str| serde_json::from_value::<SetupAccountRequest>(body(name));
        let padded = decode("  Bob  ").expect("a padded name decodes");
        assert_eq!(padded.display_name.as_str(), "Bob");
        assert!(decode("").is_err());
        assert!(decode("   ").is_err());
        assert!(decode(&"x".repeat(129)).is_err());
    }
}
