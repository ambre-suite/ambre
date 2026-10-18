// ambre-sync/src/sharing.rs
// ============================================================================
// Pure transforms for the document-sharing flow (invite + revoke key rotation).
//
// These hold the crypto-glue logic -- hex/base64 decoding, X25519 ECDH key
// wrapping, and server-response mapping -- with NO IO. The surrounding handlers
// in `lib.rs` only do `fetch` (wasm-only) plus queue plumbing. Pulling the
// decisions out here makes them unit-testable natively (`cargo test`), without
// a browser or a live server. ECDH primitive correctness itself is covered by
// `ambre-crypto` (`wrap_unwrap_roundtrip`); here we test the GLUE: peer-key
// selection, the `nonce || ciphertext` wire format, and best-effort rotation.
// ============================================================================

use ambre_crypto::DocumentKey;
use ambre_protocol::{EntryDigest, MembershipAct};
use base64::Engine;
use ed25519_dalek::SigningKey;
use serde::Serialize;
use serde::de::DeserializeOwned;
use x25519_dalek::StaticSecret;

use ambre_bus::{
    ChannelDocumentKind, ChannelId, DocumentId, LogicEvent, MemberInfo, RequestId, Role, UserId,
};

use crate::http;
use crate::membership_log::{self, Draft, Log, LogRule};
use crate::{LogicSink, SyncEvent};

/// Decodes a hex-encoded X25519 public key into its 32 raw bytes.
pub(crate) fn decode_exchange_pubkey(hex_str: &str) -> Result<[u8; 32], String> {
    let bytes = hex::decode(hex_str).map_err(|e| format!("hex decode pubkey: {e}"))?;
    bytes
        .try_into()
        .map_err(|_| "exchange pubkey is not 32 bytes".to_string())
}

/// Wraps `doc_key` for the holder of `peer_pub` via X25519 ECDH and returns the
/// base64 url-safe `nonce || ciphertext` blob the server stores as `doc_key_enc`.
/// Only that peer (using its own secret against our public key) can unwrap it.
/// The anchor of the membership log is sealed beside the key (`SYNC-104`), so
/// the recipient learns where the log starts from the member who wrapped.
pub(crate) fn wrap_anchored_b64(
    my_xk: &StaticSecret,
    peer_pub: &[u8; 32],
    doc_key: &DocumentKey,
    anchor: Option<EntryDigest>,
) -> Result<String, ambre_crypto::CryptoError> {
    // Fails if the peer public key is non-contributory (low-order): we refuse to
    // wrap under a predictable KEK rather than hand out a forgeable share.
    let anchor = anchor.map(|EntryDigest(bytes)| bytes);
    let wrapped = ambre_crypto::ecdh::wrap_anchored(my_xk, peer_pub, doc_key, anchor.as_ref())?;
    let mut enc = wrapped.nonce.to_vec();
    enc.extend_from_slice(&wrapped.ciphertext);
    Ok(base64::engine::general_purpose::URL_SAFE.encode(&enc))
}

/// Unwraps our own copy of the key among `rows`, given as `(exchange key,
/// wrapped key, wrapper's exchange key)`, and the log anchor sealed with it.
///
/// Our row is the one carrying our exchange key. The exchange runs against
/// whoever wrapped, never against our own key (`SYNC-034`). A row with no
/// wrapper recorded falls back to the member's own key, for an older relay.
pub(crate) fn unwrap_own<'a>(
    exchange_key: &StaticSecret,
    rows: impl IntoIterator<Item = (&'a str, &'a str, &'a str)>,
) -> Result<(DocumentKey, Option<EntryDigest>), String> {
    let ours = hex::encode(x25519_dalek::PublicKey::from(exchange_key).as_bytes());
    let (own, enc, wrapper) = rows
        .into_iter()
        .find(|(own, _, _)| *own == ours)
        .ok_or("own member entry not found")?;
    let encrypted = base64::engine::general_purpose::URL_SAFE
        .decode(enc)
        .map_err(|e| format!("base64 decode doc_key_enc: {e}"))?;
    let (nonce, ciphertext) = encrypted
        .split_first_chunk::<24>()
        .map(|(nonce, ciphertext)| (*nonce, ciphertext.to_vec()))
        .ok_or("wrapped key too short")?;
    let wrapper = if wrapper.is_empty() { own } else { wrapper };
    let peer = decode_exchange_pubkey(wrapper)?;
    let wrapped = ambre_crypto::WrappedKey { nonce, ciphertext };
    ambre_crypto::ecdh::unwrap_anchored(exchange_key, &peer, &wrapped)
        .map(|(key, anchor)| (key, anchor.map(EntryDigest)))
        .map_err(|e| format!("ECDH unwrap: {e}"))
}

/// Decodes a hex-encoded Ed25519 verifying key into its 32 raw bytes.
pub(crate) fn decode_signing_pubkey(hex_str: &str) -> Result<[u8; 32], String> {
    let bytes = hex::decode(hex_str).map_err(|e| format!("hex decode signing key: {e}"))?;
    bytes
        .try_into()
        .map_err(|_| "signing key is not 32 bytes".to_string())
}

/// The user who signs the entries a change appends (`SYNC-132`).
pub(crate) struct Author {
    pub user_id: UserId,
    pub signing_key: SigningKey,
}

/// The log a members answer serves, walked from the anchor sealed with our
/// own copy of the key (`SYNC-104`). The anchor is `None` when our row does
/// not unwrap or carries none.
pub(crate) fn served_log(
    document_id: DocumentId,
    exchange_key: &StaticSecret,
    resp: &http::DocumentMembersResponse,
) -> (Log, Option<EntryDigest>) {
    served_log_under(document_id, exchange_key, resp, &LogRule::document())
}

/// [`served_log`], under the rule of the document's log (`SYNC-106`).
pub(crate) fn served_log_under(
    document_id: DocumentId,
    exchange_key: &StaticSecret,
    resp: &http::DocumentMembersResponse,
    rule: &LogRule,
) -> (Log, Option<EntryDigest>) {
    let rows = resp.members.iter().map(|m| {
        (
            m.exchange_pub_key.as_str(),
            m.doc_key_enc.as_str(),
            m.wrapper_exchange_pub_key.as_str(),
        )
    });
    let anchor = unwrap_own(exchange_key, rows)
        .ok()
        .and_then(|(_, anchor)| anchor);
    (
        walk_served_under(document_id, anchor, &resp.log, rule),
        anchor,
    )
}

/// Walks the entries a members answer serves from `anchor`, under the rule
/// of the document's log (`SYNC-106`). A public channel's `meta` admits a
/// self-join and heeds its space; any other log does neither.
pub(crate) fn walk_served_under(
    document_id: DocumentId,
    anchor: Option<EntryDigest>,
    texts: &[String],
    rule: &LogRule,
) -> Log {
    let entries = membership_log::decode(texts);
    Log::walk_under(document_id, anchor, &entries, rule)
}

/// The entry `draft` makes after `log`, signed by `author`, in request form.
/// `None` for a document whose log is empty: it predates the log, and no
/// entry of it could link to a first one.
fn entry_after(log: &Log, draft: Draft, author: &Author) -> Result<Option<String>, String> {
    let next = log.next();
    if next.0 == 0 {
        return Ok(None);
    }
    let entry = membership_log::sign(draft, next, author.user_id, &author.signing_key)?;
    membership_log::encode(&entry).map(Some)
}

/// The creator's copy of a new document's key, and the first entry of its
/// log in request form (`SYNC-132`). The creator signs the entry, and the
/// copy seals its digest as the anchor (`SYNC-104`).
pub(crate) fn creation(
    document_id: DocumentId,
    doc_key: &DocumentKey,
    exchange_key: &StaticSecret,
    author: &Author,
) -> Result<(String, String), String> {
    let draft = Draft {
        document: document_id,
        member: author.user_id,
        member_key: author.signing_key.verifying_key().to_bytes(),
        act: MembershipAct::Create,
        role: Role::Owner,
        key: Some(doc_key.id()),
    };
    let entry = membership_log::sign(draft, (0, None), author.user_id, &author.signing_key)?;
    let anchor = membership_log::digest(&entry).ok_or("the first entry does not encode")?;
    let own_pub = x25519_dalek::PublicKey::from(exchange_key);
    let doc_key_enc = wrap_anchored_b64(exchange_key, own_pub.as_bytes(), doc_key, Some(anchor))
        .map_err(|e| format!("key wrap failed: {e}"))?;
    Ok((doc_key_enc, membership_log::encode(&entry)?))
}

/// Builds the `POST /documents/{id}/share` body: ECDH-wraps `doc_key` for the
/// invitee whose hex exchange pubkey is `invitee_pub_hex`. The wrap seals the
/// log `anchor` beside the key (`SYNC-104`), and the body carries the signed
/// `entry`.
pub(crate) fn build_invite_request(
    my_xk: &StaticSecret,
    invitee_pub_hex: &str,
    invitee_email: String,
    doc_key: &DocumentKey,
    role: Role,
    anchor: Option<EntryDigest>,
    entry: Option<String>,
) -> Result<http::InviteRequest, String> {
    let peer = decode_exchange_pubkey(invitee_pub_hex)?;
    Ok(http::InviteRequest {
        invitee_email,
        role,
        doc_key_enc: wrap_anchored_b64(my_xk, &peer, doc_key, anchor).map_err(|e| e.to_string())?,
        // The relay refuses the copy if a rotation replaced this key
        // between the read and the post (`SYNC-143`).
        key_id: doc_key.id(),
        entry,
    })
}

/// Re-wraps `new_doc_key` for every remaining member after a revoke (the rotate
/// payload), with the log `anchor` sealed beside it (`SYNC-104`). A member
/// whose exchange key does not parse cannot be re-wrapped for. The rotation
/// goes on for the others, and that member is returned in the second list so
/// the caller reports it (`SYNC-043`). Nothing is skipped in silence.
pub(crate) fn rotate_entries_for(
    my_xk: &StaticSecret,
    new_doc_key: &DocumentKey,
    anchor: Option<EntryDigest>,
    members: &[http::DocumentMemberEntry],
) -> (Vec<http::RotateKeyEntry>, Vec<String>) {
    let mut unwrapped = Vec::new();
    let entries = members
        .iter()
        .filter_map(|m| {
            let wrapped = decode_exchange_pubkey(&m.exchange_pub_key)
                .ok()
                .and_then(|peer| wrap_anchored_b64(my_xk, &peer, new_doc_key, anchor).ok());
            match wrapped {
                Some(doc_key_enc) => Some(http::RotateKeyEntry {
                    user_id: m.user_id,
                    doc_key_enc,
                }),
                None => {
                    unwrapped.push(m.display_name.clone());
                    None
                }
            }
        })
        .collect();
    (entries, unwrapped)
}

/// Builds the presence directory (Ed25519 signing key -> canonical `UserId`)
/// from any members response, given each entry's `(user_id, signing_pub_key)`
/// (`ambre-spec-membership.md`). Both the open path and the member-list path
/// feed it, hence the pair input rather than one of the two entry structs. The
/// identifier was parsed when the response was read, so only the key can fail.
///
/// An entry whose key does not parse is skipped rather than failing the whole
/// response. A member we cannot key on costs that one peer's cursor, while
/// refusing everything would cost the document its presence entirely.
pub(crate) fn presence_directory<'a>(
    entries: impl IntoIterator<Item = (UserId, &'a str)>,
) -> Vec<([u8; 32], UserId)> {
    entries
        .into_iter()
        .filter_map(|(user_id, signing_pub_key)| {
            let key: [u8; 32] = hex::decode(signing_pub_key).ok()?.try_into().ok()?;
            Some((key, user_id))
        })
        .collect()
}

/// Picks this client's own role out of a members response, given `(user_id,
/// role)` pairs and our canonical `UserId` (architecture).
///
/// Every failure path -- no session id, no row for us, a role that is not a
/// document one -- yields `Role::Viewer`. That is the safe direction for a UX
/// gate. It hides affordances we may in fact have, whereas guessing `Owner`
/// would offer edits the server then rejects. An unknown role word never gets
/// here: it refused the answer when it was read (`SRV-180`).
pub(crate) fn own_role(
    entries: impl IntoIterator<Item = (UserId, Role)>,
    my_user_id: Option<UserId>,
) -> Role {
    let Some(me) = my_user_id else {
        return Role::Viewer;
    };
    entries
        .into_iter()
        .find(|(user_id, _)| *user_id == me)
        .map(|(_, role)| role)
        .filter(|role| role.is_valid_for_document())
        .unwrap_or(Role::Viewer)
}

/// Maps server member entries into bus `MemberInfo` (plaintext name/email,
/// typed role). Each identifier and each role was parsed when the response was
/// read, so a malformed one refused the whole answer there. An invented id
/// would name a person nothing can act on (`SYNC-038`).
///
/// Each row carries what `log` says of it. A verified row shows the role of
/// its latest valid entry, whatever the relay's row says (`SYNC-110`).
pub(crate) fn map_members(entries: Vec<http::DocumentMemberEntry>, log: &Log) -> Vec<MemberInfo> {
    entries
        .into_iter()
        .map(|m| {
            let (standing, logged) = log.standing(m.user_id);
            MemberInfo {
                user_id: m.user_id,
                display_name: m.display_name,
                email: m.email,
                role: logged.unwrap_or(m.role),
                standing,
            }
        })
        .collect()
}

// ============================================================================
// Transport boundary + orchestration tasks.
//
// The three sharing handlers in `lib.rs` used to inline their `fetch` calls and
// their queue plumbing inside a `spawn_local`. That made the orchestration --
// which call fires when, which error maps to which event -- reachable only from
// a real browser. Abstracting the HTTP surface behind `ShareHttp` lets the tasks
// below run on `tokio` in native tests against a `FakeHttp`. Every error branch
// is asserted without a browser or a live server. Production wires the real
// `GlooHttp` (fetch + Bearer/refresh via `AuthContext`); the handlers keep only
// their synchronous preconditions and the `spawn_local`.
// ============================================================================

/// The HTTP operations the sharing tasks need, abstracted over the transport.
/// Authentication (Bearer token + refresh-on-401) is the implementation's
/// concern, not the task's -- the real impl carries an `AuthContext`.
#[allow(async_fn_in_trait)] // single-threaded worker; futures are intentionally !Send
pub(crate) trait ShareHttp {
    async fn get_json<T: DeserializeOwned>(&self, url: &str) -> Result<T, String>;
    async fn post_json<B: Serialize, R: DeserializeOwned>(
        &self,
        url: &str,
        body: &B,
    ) -> Result<R, String>;
    async fn delete<B: Serialize>(&self, url: &str, body: &B) -> Result<(), String>;
    async fn patch<B: Serialize>(&self, url: &str, body: &B) -> Result<(), String>;
}

/// Production transport: authenticated `gloo_net` fetch (wasm at runtime, but it
/// compiles natively too). Thin pass-through to the `http` helpers.
pub(crate) struct GlooHttp {
    pub auth: http::AuthContext,
}

impl ShareHttp for GlooHttp {
    async fn get_json<T: DeserializeOwned>(&self, url: &str) -> Result<T, String> {
        http::get_json_auth::<T>(url, self.auth.clone()).await
    }
    async fn post_json<B: Serialize, R: DeserializeOwned>(
        &self,
        url: &str,
        body: &B,
    ) -> Result<R, String> {
        http::post_json_auth::<B, R>(url, body, self.auth.clone()).await
    }
    async fn delete<B: Serialize>(&self, url: &str, body: &B) -> Result<(), String> {
        http::delete_json_auth(url, body, self.auth.clone()).await
    }
    async fn patch<B: Serialize>(&self, url: &str, body: &B) -> Result<(), String> {
        http::patch_json_auth(url, body, self.auth.clone()).await
    }
}

/// Sends a `LogicEvent::ApiError` (the error path for the sharing tasks).
fn api_error(logic_tx: &LogicSink, request: &str, code: u16, message: String) {
    let _ = logic_tx.send(LogicEvent::ApiError {
        id: None,
        request: request.into(),
        code,
        message,
    });
}

/// Inputs for [`invite_task`], gathered from `SyncState` before spawning.
pub(crate) struct InviteParams {
    pub id: RequestId,
    pub document_id: DocumentId,
    pub url_scheme: &'static str,
    pub api_base: String,
    pub invitee_email: String,
    pub role: Role,
    pub exchange_key: StaticSecret,
    pub doc_key: DocumentKey,
    /// The exchange key the user pinned in TOFU phase 1
    /// (`ambre-spec-key-verification.md`). Sync re-fetched key must still equal it, or the
    /// wrap is refused.
    pub expect_exchange_pub_key: [u8; 32],
    /// Signs the invitation's entry in the log.
    pub author: Author,
}

/// Invite orchestration: search the invitee by email, ECDH-wrap the document key
/// for them, POST the share. Success -> `SyncEvent::InviteSent`; every failure ->
/// `LogicEvent::ApiError` with the leg-specific code.
///
/// The re-fetched exchange key is compared against `expect_exchange_pub_key` (the
/// value pinned in phase 1) **before** wrapping. This closes the TOCTOU hole
/// where a compromised server serves an honest key to the fingerprint check and a
/// substituted one to the wrap. A mismatch fails the invite; nothing is ever
/// wrapped to an unconfirmed key.
pub(crate) async fn invite_task<H: ShareHttp>(
    http: &H,
    p: InviteParams,
    event_tx: &ambre_bus::conduit::UnboundedSender<SyncEvent>,
    logic_tx: &LogicSink,
) {
    let search_url = format!(
        "{}://{}/api/users/search?query={}",
        p.url_scheme,
        p.api_base,
        p.invitee_email.replace('@', "%40")
    );
    let users = match http.get_json::<http::UserSearchResult>(&search_url).await {
        Ok(resp) => resp.users,
        Err(e) => {
            return api_error(
                logic_tx,
                "InviteToDocument",
                500,
                format!("User search failed: {e}"),
            );
        }
    };
    let Some(invitee) = users.iter().find(|u| u.email == p.invitee_email) else {
        return api_error(logic_tx, "InviteToDocument", 404, "User not found".into());
    };
    // TOFU pin: the re-fetched key must match the one confirmed in phase 1.
    match decode_exchange_pubkey(&invitee.exchange_pubkey) {
        Ok(fetched) if fetched == p.expect_exchange_pub_key => {}
        Ok(_) => {
            return api_error(
                logic_tx,
                "InviteToDocument",
                409,
                "Invitee key changed since confirmation; invite refused".into(),
            );
        }
        Err(e) => return api_error(logic_tx, "InviteToDocument", 500, e),
    }
    // The invitation's entry names the invitee by identity and signing key.
    let invitee_id = invitee.id;
    let invitee_key = match decode_signing_pubkey(&invitee.signing_pubkey) {
        Ok(key) => key,
        Err(e) => return api_error(logic_tx, "InviteToDocument", 500, e),
    };
    let members_url = format!(
        "{}://{}/api/documents/{}/members",
        p.url_scheme, p.api_base, p.document_id
    );
    let invite_url = format!(
        "{}://{}/api/documents/{}/share",
        p.url_scheme, p.api_base, p.document_id
    );
    let mut tries = 0;
    let posted = loop {
        tries += 1;
        // The entry takes the place after the log the relay serves now.
        let members = match http
            .get_json::<http::DocumentMembersResponse>(&members_url)
            .await
        {
            Ok(resp) => resp,
            Err(e) => {
                return api_error(
                    logic_tx,
                    "InviteToDocument",
                    500,
                    format!("Get members failed: {e}"),
                );
            }
        };
        let (log, anchor) = served_log(p.document_id, &p.exchange_key, &members);
        let draft = Draft {
            document: p.document_id,
            member: invitee_id,
            member_key: invitee_key,
            act: MembershipAct::Invite,
            role: p.role,
            key: Some(p.doc_key.id()),
        };
        let body = match entry_after(&log, draft, &p.author).and_then(|entry| {
            build_invite_request(
                &p.exchange_key,
                &invitee.exchange_pubkey,
                p.invitee_email.clone(),
                &p.doc_key,
                p.role,
                anchor,
                entry,
            )
        }) {
            Ok(req) => req,
            Err(message) => return api_error(logic_tx, "InviteToDocument", 500, message),
        };
        match http
            .post_json::<_, http::InviteResponse>(&invite_url, &body)
            .await
        {
            // Another entry took the place first: read the log again.
            Err(e) if http::is_log_moved(&e) && tries < ROTATION_TRIES => {}
            posted => break posted,
        }
    };
    match posted {
        Ok(resp) => {
            let _ = event_tx.send(SyncEvent::InviteSent {
                id: p.id,
                document_id: p.document_id,
                user_id: resp.user_id,
                display_name: resp.display_name,
            });
        }
        // A rotation landed first: the copy wraps a key the relay refuses.
        // Nothing was added, and an invitation sent again wraps the new key.
        Err(e) if http::is_key_changed(&e) => api_error(
            logic_tx,
            "InviteToDocument",
            409,
            "The document's key changed during the invitation; invite again".into(),
        ),
        Err(e) => api_error(logic_tx, "InviteToDocument", 500, e),
    }
}

/// Inputs for [`fetch_invitee_key_task`] (TOFU phase 1).
pub(crate) struct FetchInviteeKeyParams {
    pub id: RequestId,
    pub document_id: DocumentId,
    pub url_scheme: &'static str,
    pub api_base: String,
    pub invitee_email: String,
}

/// TOFU phase 1 (`ambre-spec-key-verification.md`): resolve the invitee by email and
/// fetch their exchange key, compute its fingerprint, and answer with
/// `SyncEvent::InviteeKeyReady`. **No key is wrapped and nothing is shared** --
/// the Main thread makes the trust decision before phase 2 (`invite_task`).
/// Every failure maps to `LogicEvent::ApiError`.
pub(crate) async fn fetch_invitee_key_task<H: ShareHttp>(
    http: &H,
    p: FetchInviteeKeyParams,
    event_tx: &ambre_bus::conduit::UnboundedSender<SyncEvent>,
    logic_tx: &LogicSink,
) {
    let search_url = format!(
        "{}://{}/api/users/search?query={}",
        p.url_scheme,
        p.api_base,
        p.invitee_email.replace('@', "%40")
    );
    let users = match http.get_json::<http::UserSearchResult>(&search_url).await {
        Ok(resp) => resp.users,
        Err(e) => {
            return api_error(
                logic_tx,
                "FetchInviteeKey",
                500,
                format!("User search failed: {e}"),
            );
        }
    };
    let Some(invitee) = users.iter().find(|u| u.email == p.invitee_email) else {
        return api_error(logic_tx, "FetchInviteeKey", 404, "User not found".into());
    };
    let exchange_pub_key = match decode_exchange_pubkey(&invitee.exchange_pubkey) {
        Ok(key) => key,
        Err(e) => return api_error(logic_tx, "FetchInviteeKey", 500, e),
    };
    let fingerprint = ambre_crypto::fingerprint(&exchange_pub_key);
    // The pin is filed under this id. An invented one would make the invitee
    // new on every invitation, and a substituted key would read as a first
    // contact (`CHROME-074`).
    let _ = event_tx.send(SyncEvent::InviteeKeyReady {
        id: p.id,
        document_id: p.document_id,
        user_id: invitee.id,
        display_name: invitee.display_name.clone(),
        email: invitee.email.clone(),
        exchange_pub_key,
        fingerprint,
    });
}

/// How many times a rotation is posted when members keep joining between its
/// read and its post (`SYNC-143`). Each try reads the members again.
const ROTATION_TRIES: usize = 3;

/// Inputs for [`revoke_task`], gathered from `SyncState` before spawning.
pub(crate) struct RevokeParams {
    pub id: RequestId,
    pub document_id: DocumentId,
    pub user_id: UserId,
    pub url_scheme: &'static str,
    pub api_base: String,
    pub exchange_key: StaticSecret,
    /// The key the revoke replaces, sealed under the new one (`SYNC-116`).
    pub previous: DocumentKey,
    /// Signs the link the revoke leaves, and its entry in the log.
    pub author: Author,
}

/// Revoke orchestration: fetch the members, generate a fresh `DocumentKey` and
/// wrap it for everyone but the member removed. Then POST the removal with
/// every copy in one request (`SYNC-126`). The relay applies both or neither,
/// so a failure leaves the member in and the key unchanged (`SYNC-041`).
/// Success -> `SyncEvent::RevocationComplete` (the caller swaps the local key);
/// every failure -> `LogicEvent::ApiError`.
pub(crate) async fn revoke_task<H: ShareHttp>(
    http: &H,
    p: RevokeParams,
    event_tx: &ambre_bus::conduit::UnboundedSender<SyncEvent>,
    logic_tx: &LogicSink,
) {
    let members_url = format!(
        "{}://{}/api/documents/{}/members",
        p.url_scheme, p.api_base, p.document_id
    );
    let new_doc_key = DocumentKey::generate();
    let link = match link_to(
        p.document_id,
        &p.previous,
        &new_doc_key,
        &p.author.signing_key,
    ) {
        Ok(link) => link,
        Err(e) => return api_error(logic_tx, "RevokeFromDocument", 500, e),
    };
    let revoke_url = format!(
        "{}://{}/api/documents/{}/revoke",
        p.url_scheme, p.api_base, p.document_id
    );
    let mut tries = 0;
    let unwrapped = loop {
        tries += 1;
        let resp = match http
            .get_json::<http::DocumentMembersResponse>(&members_url)
            .await
        {
            Ok(resp) => resp,
            Err(e) => {
                return api_error(
                    logic_tx,
                    "RevokeFromDocument",
                    500,
                    format!("Get members failed: {e}"),
                );
            }
        };
        let (log, anchor) = served_log(p.document_id, &p.exchange_key, &resp);
        // Compared parsed, so a spelling of the same id cannot keep the member in.
        let (removed, staying): (Vec<_>, Vec<_>) = resp
            .members
            .into_iter()
            .partition(|m| m.user_id == p.user_id);
        let entry = match removed.first().map(|m| {
            let draft = Draft {
                document: p.document_id,
                member: p.user_id,
                member_key: decode_signing_pubkey(&m.signing_pub_key)?,
                act: MembershipAct::Revoke,
                role: m.role,
                key: Some(new_doc_key.id()),
            };
            entry_after(&log, draft, &p.author)
        }) {
            Some(Ok(entry)) => entry,
            // No row for the member: the relay refuses the removal itself.
            None => None,
            Some(Err(e)) => return api_error(logic_tx, "RevokeFromDocument", 500, e),
        };
        let (keys, unwrapped) = rotate_entries_for(&p.exchange_key, &new_doc_key, anchor, &staying);
        let body = http::RevokeRequest {
            user_id: p.user_id,
            keys,
            members: staying.into_iter().map(|m| m.user_id).collect(),
            link: link.1.clone(),
            entry,
        };
        match http
            .post_json::<_, http::WriteAnswer>(&revoke_url, &body)
            .await
            .and_then(|answer| answer.expect(ambre_core::WriteStatus::Revoked))
        {
            Ok(()) => break unwrapped,
            // A member joined between the read and the post, or another
            // entry took the place of this one, and the relay changed
            // nothing. The next try reads them again (`SYNC-143`).
            Err(e)
                if (http::is_members_changed(&e) || http::is_log_moved(&e))
                    && tries < ROTATION_TRIES => {}
            Err(e) => {
                return api_error(
                    logic_tx,
                    "RevokeFromDocument",
                    500,
                    format!("Revoke failed: {e}"),
                );
            }
        }
    };

    let _ = event_tx.send(SyncEvent::RevocationComplete {
        id: p.id,
        document_id: p.document_id,
        new_doc_key,
        link: link.0,
    });
    // The rotation went through for everyone else. A member it could not
    // re-wrap for has lost the document; the user who revoked is told who
    // (`SYNC-043`).
    if !unwrapped.is_empty() {
        api_error(
            logic_tx,
            "RevokeFromDocument",
            422,
            format!(
                "the new key could not be shared with {}: their key is unreadable",
                unwrapped.join(", ")
            ),
        );
    }
}

/// Inputs for [`leave_task`].
pub(crate) struct LeaveParams {
    pub id: RequestId,
    pub document_id: DocumentId,
    /// This user, who signs their own revoke: a departure removes no one else.
    pub author: Author,
    pub url_scheme: &'static str,
    pub api_base: String,
    pub exchange_key: StaticSecret,
}

/// Departure orchestration: DELETE this user's own share with the revoke they
/// sign, and nothing else. No key is generated and none is posted, so the
/// members who stay never trust a key the leaver chose (`SYNC-127`,
/// `SYNC-130`). The relay records the rotation as owed. Success ->
/// `SyncEvent::DocumentLeft`; failure -> `LogicEvent::ApiError`.
pub(crate) async fn leave_task<H: ShareHttp>(
    http: &H,
    p: LeaveParams,
    event_tx: &ambre_bus::conduit::UnboundedSender<SyncEvent>,
    logic_tx: &LogicSink,
) {
    let left = leave_document(
        http,
        p.document_id,
        &p.author,
        p.url_scheme,
        &p.api_base,
        &p.exchange_key,
    )
    .await;
    match left {
        Ok(()) => {
            let _ = event_tx.send(SyncEvent::DocumentLeft {
                id: p.id,
                document_id: p.document_id,
            });
        }
        Err(e) => api_error(logic_tx, "LeaveDocument", 500, e),
    }
}

/// DELETEs this user's own share of `document_id` with the revoke they sign
/// on the log the relay serves. A log that moved meanwhile is read again.
async fn leave_document<H: ShareHttp>(
    http: &H,
    document_id: DocumentId,
    author: &Author,
    url_scheme: &str,
    api_base: &str,
    exchange_key: &StaticSecret,
) -> Result<(), String> {
    let me = author.user_id;
    let members_url = format!("{url_scheme}://{api_base}/api/documents/{document_id}/members");
    let url = format!("{url_scheme}://{api_base}/api/documents/{document_id}/share/{me}");
    let mut tries = 0;
    loop {
        tries += 1;
        let resp = http
            .get_json::<http::DocumentMembersResponse>(&members_url)
            .await
            .map_err(|e| format!("Get members failed: {e}"))?;
        let (log, _) = served_log(document_id, exchange_key, &resp);
        let role = resp
            .members
            .iter()
            .find(|m| m.user_id == me)
            .map_or(Role::Viewer, |m| m.role);
        let draft = Draft {
            document: document_id,
            member: me,
            member_key: author.signing_key.verifying_key().to_bytes(),
            act: MembershipAct::Revoke,
            role,
            key: None,
        };
        let entry = entry_after(&log, draft, author)?;
        let body = http::LeaveRequest { entry };
        match http.delete(&url, &body).await {
            Ok(()) => return Ok(()),
            Err(e) if http::is_log_moved(&e) && tries < ROTATION_TRIES => {}
            Err(e) => return Err(format!("Leave failed: {e}")),
        }
    }
}

/// Inputs for [`leave_channel_task`], gathered from `SyncState` before
/// spawning.
pub(crate) struct ChannelLeaveParams {
    pub channel_id: ChannelId,
    /// This user, who signs their own revoke on the channel's `meta`.
    pub author: Author,
    pub url_scheme: &'static str,
    pub api_base: String,
    pub exchange_key: StaticSecret,
}

/// Leave a channel. Find its `meta`, the document whose rows are its members,
/// then leave that document as any other. The members who stay owe the
/// rotation (`SYNC-127`, `SYNC-130`).
///
/// Nothing lands on any failure. The caller hears the reason.
pub(crate) async fn leave_channel_task<H: ShareHttp>(
    http: &H,
    p: ChannelLeaveParams,
) -> Result<(), String> {
    let (scheme, base) = (p.url_scheme, &p.api_base);
    let url = format!("{scheme}://{base}/api/channels/{}/documents", p.channel_id);
    let listed = http
        .get_json::<http::ChannelDocumentsResponse>(&url)
        .await?;
    let meta = crate::channels::channel_documents(&listed.documents)
        .into_iter()
        .find(|d| d.kind == ChannelDocumentKind::Meta)
        .ok_or("the channel has no meta document")?
        .document_id;
    leave_document(http, meta, &p.author, scheme, base, &p.exchange_key).await
}

/// Inputs for [`settle_task`].
pub(crate) struct SettleParams {
    pub document_id: DocumentId,
    pub url_scheme: &'static str,
    pub api_base: String,
    pub exchange_key: StaticSecret,
    /// The key the rotation replaces, sealed under the new one (`SYNC-116`).
    pub previous: DocumentKey,
    /// Signs the link the rotation leaves.
    pub signing_key: SigningKey,
}

/// Inputs for [`role_change_task`].
pub(crate) struct RoleChangeParams {
    pub id: RequestId,
    pub document_id: DocumentId,
    /// The member whose role changes.
    pub user_id: UserId,
    pub role: Role,
    pub url_scheme: &'static str,
    pub api_base: String,
    pub exchange_key: StaticSecret,
    /// The owner who signs the change's entry.
    pub author: Author,
}

/// Role-change orchestration: read the log, sign the change's entry and
/// PATCH it with the role (`SYNC-132`). Success ->
/// `SyncEvent::MemberRoleUpdatedEvent`; failure -> `LogicEvent::ApiError`.
pub(crate) async fn role_change_task<H: ShareHttp>(
    http: &H,
    p: RoleChangeParams,
    event_tx: &ambre_bus::conduit::UnboundedSender<SyncEvent>,
    logic_tx: &LogicSink,
) {
    let members_url = format!(
        "{}://{}/api/documents/{}/members",
        p.url_scheme, p.api_base, p.document_id
    );
    let url = format!(
        "{}://{}/api/documents/{}/members/{}",
        p.url_scheme, p.api_base, p.document_id, p.user_id
    );
    let mut tries = 0;
    loop {
        tries += 1;
        let resp = match http
            .get_json::<http::DocumentMembersResponse>(&members_url)
            .await
        {
            Ok(resp) => resp,
            Err(e) => {
                return api_error(
                    logic_tx,
                    "UpdateDocumentMemberRole",
                    500,
                    format!("Get members failed: {e}"),
                );
            }
        };
        let (log, _) = served_log(p.document_id, &p.exchange_key, &resp);
        let Some(target) = resp.members.iter().find(|m| m.user_id == p.user_id) else {
            return api_error(
                logic_tx,
                "UpdateDocumentMemberRole",
                404,
                "the member is no longer in the document".into(),
            );
        };
        let entry = decode_signing_pubkey(&target.signing_pub_key).and_then(|member_key| {
            let draft = Draft {
                document: p.document_id,
                member: p.user_id,
                member_key,
                act: MembershipAct::ChangeRole,
                role: p.role,
                key: None,
            };
            entry_after(&log, draft, &p.author)
        });
        let entry = match entry {
            Ok(entry) => entry,
            Err(e) => return api_error(logic_tx, "UpdateDocumentMemberRole", 500, e),
        };
        let body = http::RoleChangeRequest {
            role: p.role,
            entry,
        };
        match http.patch(&url, &body).await {
            Ok(()) => break,
            Err(e) if http::is_log_moved(&e) && tries < ROTATION_TRIES => {}
            Err(e) => return api_error(logic_tx, "UpdateDocumentMemberRole", 500, e),
        }
    }
    let _ = event_tx.send(SyncEvent::MemberRoleUpdatedEvent {
        id: p.id,
        document_id: p.document_id,
        user_id: p.user_id,
        role: p.role,
    });
}

/// The link from `next` back to `previous`, and the text a request carries.
fn link_to(
    document_id: DocumentId,
    previous: &DocumentKey,
    next: &DocumentKey,
    signing_key: &SigningKey,
) -> Result<(ambre_protocol::KeyLink, String), String> {
    let link = crate::pipeline::seal_link(document_id, previous, next, signing_key)
        .map_err(|e| format!("Seal key link failed: {e}"))?;
    let text = http::encode_link(&link)?;
    Ok((link, text))
}

/// Pays a rotation the document owes (`SYNC-128`). It fetches the members and,
/// while the debt stands, posts a fresh key wrapped for each of them. A debt
/// someone else already paid ends in `SyncEvent::DebtGone`, and the caller
/// fetches the key they chose. A failure ends in `SyncEvent::DebtUnsettled`,
/// and the next frame tries again.
pub(crate) async fn settle_task<H: ShareHttp>(
    http: &H,
    p: SettleParams,
    event_tx: &ambre_bus::conduit::UnboundedSender<SyncEvent>,
) {
    let document_id = p.document_id;
    let unsettled = |message: String| {
        let _ = event_tx.send(SyncEvent::DebtUnsettled {
            document_id,
            message,
        });
    };
    let members_url = format!(
        "{}://{}/api/documents/{}/members",
        p.url_scheme, p.api_base, document_id
    );
    let rotate_url = format!(
        "{}://{}/api/documents/{}/rotate-key",
        p.url_scheme, p.api_base, document_id
    );
    let new_doc_key = DocumentKey::generate();
    let (link, text) = match link_to(document_id, &p.previous, &new_doc_key, &p.signing_key) {
        Ok(link) => link,
        Err(e) => return unsettled(e),
    };
    let mut tries = 0;
    loop {
        tries += 1;
        let resp = match http
            .get_json::<http::DocumentMembersResponse>(&members_url)
            .await
        {
            Ok(resp) => resp,
            Err(e) => return unsettled(format!("Get members failed: {e}")),
        };
        if !resp.rotation_owed {
            let _ = event_tx.send(SyncEvent::DebtGone { document_id });
            return;
        }
        let (_, anchor) = served_log(document_id, &p.exchange_key, &resp);
        let (keys, unwrapped) =
            rotate_entries_for(&p.exchange_key, &new_doc_key, anchor, &resp.members);
        if !unwrapped.is_empty() {
            tracing::warn!(
                "Sync: the owed rotation skips {} member(s) with an unreadable key",
                unwrapped.len()
            );
        }
        let body = http::RotateKeyRequest {
            keys,
            members: resp.members.into_iter().map(|m| m.user_id).collect(),
            link: text.clone(),
        };
        match http
            .post_json::<_, http::RotateKeyResponse>(&rotate_url, &body)
            .await
        {
            // The one word of the set; another refuses the answer (`SRV-180`).
            Ok(http::RotateKeyResponse {
                status: ambre_core::RotationStatus::Rotated,
            }) => {
                let _ = event_tx.send(SyncEvent::DebtSettled {
                    document_id,
                    new_doc_key,
                    link,
                });
                return;
            }
            // A member joined since the read (`SYNC-143`): read them again.
            Err(e) if http::is_members_changed(&e) && tries < ROTATION_TRIES => {}
            Err(e) => return unsettled(format!("Rotate key failed: {e}")),
        }
    }
}

/// Inputs for [`rotate_channel_task`].
pub(crate) struct RotateChannelParams {
    pub space_id: ambre_bus::SpaceId,
    pub channel_id: ChannelId,
    pub url_scheme: &'static str,
    pub api_base: String,
    pub exchange_key: StaticSecret,
    /// Signs the link the rotation leaves.
    pub signing_key: SigningKey,
    /// The channel key the catalogue holds, which the rotation replaces.
    pub previous: DocumentKey,
    /// The anchor of meta's log, sealed beside the catalogue's key.
    pub anchor: EntryDigest,
    /// The current space key, which seals the new catalogue entry.
    pub space_key: DocumentKey,
}

/// Rotates the key of a public channel after a rotation of its space (the
/// Murmure spaces spec). A space admin re-keys meta, member of it or not
/// (`SRV-146`), for the members the relay still lists. The new key goes back
/// sealed under the current space key, for the catalogue. The answer is a
/// `SyncEvent::ChannelRotated`.
pub(crate) async fn rotate_channel_task<H: ShareHttp>(
    http: &H,
    p: RotateChannelParams,
    event_tx: &ambre_bus::conduit::UnboundedSender<SyncEvent>,
) {
    let result = rotate_channel(http, &p).await;
    let _ = event_tx.send(SyncEvent::ChannelRotated {
        space_id: p.space_id,
        channel_id: p.channel_id,
        result,
    });
}

/// The body of [`rotate_channel_task`]: the new catalogue entry.
async fn rotate_channel<H: ShareHttp>(
    http: &H,
    p: &RotateChannelParams,
) -> Result<ambre_bus::SealedChannelKey, String> {
    let base = format!("{}://{}/api", p.url_scheme, p.api_base);
    let head = http
        .get_json::<http::JoinHeadResponse>(&format!("{base}/channels/{}/join", p.channel_id))
        .await
        .map_err(|e| format!("Get the join head failed: {e}"))?;
    let meta = head.meta_id.document_id();
    let members_url = format!("{base}/documents/{meta}/members");
    let rotate_url = format!("{base}/documents/{meta}/rotate-key");
    let new_key = DocumentKey::generate();
    let (_, text) = link_to(meta, &p.previous, &new_key, &p.signing_key)?;
    let mut tries = 0;
    loop {
        tries += 1;
        let resp = http
            .get_json::<http::DocumentMembersResponse>(&members_url)
            .await
            .map_err(|e| format!("Get members failed: {e}"))?;
        let (keys, unwrapped) =
            rotate_entries_for(&p.exchange_key, &new_key, Some(p.anchor), &resp.members);
        if !unwrapped.is_empty() {
            tracing::warn!(
                "Sync: the channel rotation skips {} member(s) with an unreadable key",
                unwrapped.len()
            );
        }
        let body = http::RotateKeyRequest {
            keys,
            members: resp.members.into_iter().map(|m| m.user_id).collect(),
            link: text.clone(),
        };
        match http
            .post_json::<_, http::RotateKeyResponse>(&rotate_url, &body)
            .await
        {
            Ok(http::RotateKeyResponse {
                status: ambre_core::RotationStatus::Rotated,
            }) => break,
            // A member joined since the read (`SYNC-143`): read them again.
            Err(e) if http::is_members_changed(&e) && tries < ROTATION_TRIES => {}
            Err(e) => return Err(format!("Rotate key failed: {e}")),
        }
    }
    crate::channels::seal_channel_key(p.channel_id, &new_key, &p.space_key, p.anchor)
}

/// Inputs for [`list_members_task`].
pub(crate) struct ListMembersParams {
    /// The request to answer, `None` for a refresh of the directory alone.
    pub id: Option<RequestId>,
    /// Which listing of this document this is, so an answer older than the
    /// directory held changes nothing (`SYNC-037`).
    pub asked: u64,
    pub document_id: DocumentId,
    pub url_scheme: &'static str,
    pub api_base: String,
    /// Unwraps our own row, whose seal carries the log anchor (`SYNC-104`).
    pub exchange_key: StaticSecret,
    /// The rule the document's log walks under (`SYNC-106`).
    pub rule: LogRule,
}

/// Member-list orchestration: GET the members, map them, emit
/// `SyncEvent::DocumentMembersReady`; on failure emit `SyncEvent::HttpError`.
pub(crate) async fn list_members_task<H: ShareHttp>(
    http: &H,
    p: ListMembersParams,
    event_tx: &ambre_bus::conduit::UnboundedSender<SyncEvent>,
) {
    let url = format!(
        "{}://{}/api/documents/{}/members",
        p.url_scheme, p.api_base, p.document_id
    );
    match http.get_json::<http::DocumentMembersResponse>(&url).await {
        Ok(resp) => {
            // Refresh the presence directory off the same response
            // (`ambre-spec-membership.md`). Membership just changed often enough to ask
            // for the list. This is where a new member becomes resolvable and a revoked
            // one stops resolving, with no extra round-trip.
            let signing_keys = presence_directory(
                resp.members
                    .iter()
                    .map(|m| (m.user_id, m.signing_pub_key.as_str())),
            );
            // Each row carries what the log says of it (`SYNC-108`).
            let (log, _) = served_log_under(p.document_id, &p.exchange_key, &resp, &p.rule);
            let word = membership_log::SpaceWord::of(&log);
            let rotation_owed = resp.rotation_owed;
            let members = map_members(resp.members, &log);
            let _ = event_tx.send(SyncEvent::DocumentMembersReady {
                id: p.id,
                asked: p.asked,
                document_id: p.document_id,
                word,
                rotation_owed,
                members,
                signing_keys,
            });
        }
        Err(e) => match p.id {
            Some(id) => {
                let _ = event_tx.send(SyncEvent::HttpError { id, message: e });
            }
            // No one waits on a refresh of the directory alone: the
            // directory it held stays, and the next change asks again.
            None => tracing::warn!("Sync: the members could not be read again: {e}"),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ambre_crypto::MasterSeed;
    use x25519_dalek::PublicKey;

    /// A deterministic (secret, hex-public) X25519 pair for a fake peer.
    fn keypair(seed: u8) -> (StaticSecret, String) {
        let xk = MasterSeed::from_array([seed; 32]).derive_exchange_key();
        let pub_hex = hex::encode(PublicKey::from(&xk).to_bytes());
        (xk, pub_hex)
    }

    fn member(uid: &str, pub_hex: &str) -> http::DocumentMemberEntry {
        http::DocumentMemberEntry {
            user_id: UserId::parse_str(uid).expect("a test id"),
            display_name: "Member".into(),
            email: "m@test.local".into(),
            signing_pub_key: String::new(),
            exchange_pub_key: pub_hex.to_string(),
            doc_key_enc: String::new(),
            role: Role::Editor,
            wrapper_exchange_pub_key: String::new(),
        }
    }

    /// Decode helper: the inverse of `wrap_anchored_b64`, used by recipients.
    fn unwrap_b64(
        recipient: &StaticSecret,
        wrapper_pub: &[u8; 32],
        doc_key_enc: &str,
    ) -> Result<DocumentKey, String> {
        unwrap_anchor_b64(recipient, wrapper_pub, doc_key_enc).map(|(key, _)| key)
    }

    /// As [`unwrap_b64`], with the anchor sealed beside the key.
    fn unwrap_anchor_b64(
        recipient: &StaticSecret,
        wrapper_pub: &[u8; 32],
        doc_key_enc: &str,
    ) -> Result<(DocumentKey, Option<EntryDigest>), String> {
        let own = hex::encode(PublicKey::from(recipient).to_bytes());
        let wrapper = hex::encode(wrapper_pub);
        unwrap_own(recipient, [(own.as_str(), doc_key_enc, wrapper.as_str())])
    }

    #[test]
    fn decode_exchange_pubkey_rejects_bad_input() {
        assert!(decode_exchange_pubkey("not-hex").is_err());
        // 31 bytes -> wrong length.
        assert!(decode_exchange_pubkey(&"ab".repeat(31)).is_err());
        assert!(decode_exchange_pubkey(&"ab".repeat(32)).is_ok());
    }

    #[test]
    fn invite_request_wraps_for_the_invitee_only() {
        let (my_xk, my_pub_hex) = keypair(1);
        let my_pub: [u8; 32] = decode_exchange_pubkey(&my_pub_hex).unwrap();
        let (invitee_xk, invitee_pub_hex) = keypair(2);
        let (stranger_xk, _) = keypair(3);
        let doc_key = DocumentKey::generate();

        let req = build_invite_request(
            &my_xk,
            &invitee_pub_hex,
            "invitee@test.local".into(),
            &doc_key,
            Role::Editor,
            None,
            None,
        )
        .expect("valid pubkey");

        assert_eq!(req.invitee_email, "invitee@test.local");
        assert_eq!(req.role, Role::Editor);
        // The invitee unwraps against the inviter's public key: succeeds.
        assert!(unwrap_b64(&invitee_xk, &my_pub, &req.doc_key_enc).is_ok());
        // A stranger cannot (AEAD fails closed under the wrong shared secret).
        assert!(unwrap_b64(&stranger_xk, &my_pub, &req.doc_key_enc).is_err());
    }

    #[test]
    fn invite_request_propagates_bad_pubkey() {
        let (my_xk, _) = keypair(1);
        let err = build_invite_request(
            &my_xk,
            "zz-not-hex",
            "x@y.z".into(),
            &DocumentKey::generate(),
            Role::Viewer,
            None,
            None,
        );
        assert!(err.is_err());
    }

    #[test]
    fn rotate_entries_cover_valid_members_and_report_malformed() {
        let (my_xk, my_pub_hex) = keypair(1);
        let my_pub: [u8; 32] = decode_exchange_pubkey(&my_pub_hex).unwrap();
        let (a_xk, a_pub) = keypair(2);
        let (_b_xk, b_pub) = keypair(3);
        let new_key = DocumentKey::generate();

        let members = vec![
            member("11111111-1111-1111-1111-111111111111", &a_pub),
            member("22222222-2222-2222-2222-222222222222", "garbage"),
            member("33333333-3333-3333-3333-333333333333", &b_pub),
        ];
        let (entries, unwrapped) = rotate_entries_for(&my_xk, &new_key, None, &members);

        // The malformed member is returned for the report; the two valid ones
        // are wrapped (`SYNC-043`).
        assert_eq!(unwrapped.len(), 1);
        assert_eq!(entries.len(), 2);
        let id = |s| UserId::parse_str(s).unwrap();
        assert_eq!(
            entries[0].user_id,
            id("11111111-1111-1111-1111-111111111111")
        );
        assert_eq!(
            entries[1].user_id,
            id("33333333-3333-3333-3333-333333333333")
        );
        // Member A can unwrap its own rotated entry.
        assert!(unwrap_b64(&a_xk, &my_pub, &entries[0].doc_key_enc).is_ok());
    }

    #[test]
    fn map_members_parses_roles_and_refuses_malformed_ids() {
        let read = |user_id: &str, role: &str| {
            serde_json::from_value::<http::DocumentMemberEntry>(serde_json::json!({
                "user_id": user_id,
                "display_name": "Member",
                "email": "m@test.local",
                "signing_pub_key": "",
                "exchange_pub_key": "ab",
                "doc_key_enc": "",
                "role": role,
            }))
        };
        let sound_id = "44444444-4444-4444-4444-444444444444";
        // An unparsable id refuses the row when it is read, so nothing is
        // left to replace by an invented one (`SYNC-038`).
        assert!(read("not-a-uuid", "editor").is_err());
        // So does a role word no build wrote, and a spelling of a known one
        // (`SRV-180`).
        assert!(read(sound_id, "bogus").is_err());
        assert!(read(sound_id, "Editor").is_err());
        let sound = read(sound_id, "editor").expect("a sound row");
        let none = Log::walk(DocumentId::new(), None, &[], membership_log::no_self_join);
        let mapped = map_members(vec![sound], &none);
        assert_eq!(mapped[0].user_id, UserId::parse_str(sound_id).unwrap());
        assert_eq!(mapped[0].role, Role::Editor);
    }

    // -- Orchestration tasks (Tier 2): driven natively against a fake transport.

    use std::cell::RefCell;
    use std::collections::VecDeque;

    /// One recorded HTTP call (method + url + serialized body for POSTs).
    #[derive(Debug)]
    enum Call {
        Get(String),
        Post(String, serde_json::Value),
        Delete(String, serde_json::Value),
        Patch(String, serde_json::Value),
    }

    /// A scripted [`ShareHttp`]. It returns the queued responses in order and
    /// records every call, so a test can assert what the task did (and did NOT)
    /// fire. A response is `Ok(json)` (deserialized into the requested type) or
    /// `Err(message)` (the transport-level failure the real fetch would yield).
    struct FakeHttp {
        responses: RefCell<VecDeque<Result<serde_json::Value, String>>>,
        calls: RefCell<Vec<Call>>,
    }

    impl FakeHttp {
        fn new(responses: Vec<Result<serde_json::Value, String>>) -> Self {
            Self {
                responses: RefCell::new(responses.into()),
                calls: RefCell::new(Vec::new()),
            }
        }
        fn next(&self) -> Result<serde_json::Value, String> {
            self.responses
                .borrow_mut()
                .pop_front()
                .expect("unexpected extra HTTP call")
        }
        fn calls(&self) -> std::cell::Ref<'_, Vec<Call>> {
            self.calls.borrow()
        }
    }

    impl ShareHttp for FakeHttp {
        async fn get_json<T: DeserializeOwned>(&self, url: &str) -> Result<T, String> {
            self.calls.borrow_mut().push(Call::Get(url.to_string()));
            let v = self.next()?;
            serde_json::from_value(v).map_err(|e| e.to_string())
        }
        async fn post_json<B: Serialize, R: DeserializeOwned>(
            &self,
            url: &str,
            body: &B,
        ) -> Result<R, String> {
            let bv = serde_json::to_value(body).unwrap();
            self.calls
                .borrow_mut()
                .push(Call::Post(url.to_string(), bv));
            let v = self.next()?;
            serde_json::from_value(v).map_err(|e| e.to_string())
        }
        async fn delete<B: Serialize>(&self, url: &str, body: &B) -> Result<(), String> {
            let bv = serde_json::to_value(body).unwrap();
            self.calls
                .borrow_mut()
                .push(Call::Delete(url.to_string(), bv));
            self.next().map(|_| ())
        }
        async fn patch<B: Serialize>(&self, url: &str, body: &B) -> Result<(), String> {
            let bv = serde_json::to_value(body).unwrap();
            self.calls
                .borrow_mut()
                .push(Call::Patch(url.to_string(), bv));
            self.next().map(|_| ())
        }
    }

    /// The author every task test signs as.
    fn author() -> Author {
        Author {
            user_id: UserId::new(),
            signing_key: MasterSeed::from_array([9; 32]).derive_signing_key(),
        }
    }

    /// A hex public key of 32 bytes, as the relay serves it.
    fn signing_hex(key: &SigningKey) -> String {
        key.verifying_key()
            .to_bytes()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect()
    }

    /// Fresh event + logic queues with their receivers.
    fn channels() -> (
        ambre_bus::conduit::UnboundedSender<SyncEvent>,
        ambre_bus::conduit::Receiver<SyncEvent>,
        LogicSink,
        ambre_bus::conduit::Receiver<LogicEvent>,
    ) {
        let (etx, erx) = ambre_bus::conduit::unbounded();
        let (sink, lrx) = LogicSink::channel();
        (etx, erx, sink, lrx)
    }

    fn user_search_json(email: &str, exchange_pub_hex: &str) -> serde_json::Value {
        serde_json::json!({ "users": [ {
            "id": "55555555-5555-5555-5555-555555555555",
            "email": email,
            "display_name": "Invitee",
            "signing_pubkey": "00".repeat(32),
            "exchange_pubkey": exchange_pub_hex,
        } ] })
    }

    fn invite_params(my_xk: &StaticSecret, expect: [u8; 32]) -> InviteParams {
        InviteParams {
            id: RequestId::next(),
            document_id: DocumentId::new(),
            url_scheme: "https",
            api_base: "api.test".into(),
            invitee_email: "invitee@test.local".into(),
            role: Role::Editor,
            exchange_key: my_xk.clone(),
            doc_key: DocumentKey::generate(),
            expect_exchange_pub_key: expect,
            author: author(),
        }
    }

    #[tokio::test]
    async fn invite_success_posts_wrapped_key_and_emits_invitesent() {
        let (my_xk, my_pub_hex) = keypair(1);
        let my_pub = decode_exchange_pubkey(&my_pub_hex).unwrap();
        let (invitee_xk, invitee_pub_hex) = keypair(2);
        let invitee_pub = decode_exchange_pubkey(&invitee_pub_hex).unwrap();

        let http = FakeHttp::new(vec![
            Ok(user_search_json("invitee@test.local", &invitee_pub_hex)),
            Ok(serde_json::json!({ "members": [] })),
            Ok(serde_json::json!({
                "user_id": "55555555-5555-5555-5555-555555555555",
                "display_name": "Invitee",
            })),
        ]);
        let (etx, erx, sink, lrx) = channels();
        invite_task(&http, invite_params(&my_xk, invitee_pub), &etx, &sink).await;

        // Success event, no error.
        match erx.try_recv().expect("InviteSent emitted") {
            SyncEvent::InviteSent { display_name, .. } => assert_eq!(display_name, "Invitee"),
            _ => panic!("unexpected event (want InviteSent)"),
        }
        assert!(lrx.try_recv().is_err(), "no ApiError on success");

        // Three calls: search, members, then share. The share body wraps a key
        // only the invitee can unwrap, and a document with no log gets no entry.
        let calls = http.calls();
        assert_eq!(calls.len(), 3);
        match &calls[2] {
            Call::Post(url, body) => {
                assert!(url.ends_with("/share"));
                let enc = body["doc_key_enc"].as_str().unwrap();
                assert!(unwrap_b64(&invitee_xk, &my_pub, enc).is_ok());
                assert!(body.get("entry").is_none());
            }
            other => panic!("expected share POST, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn invite_user_not_found_emits_404_and_skips_share() {
        let (my_xk, _) = keypair(1);
        let http = FakeHttp::new(vec![Ok(serde_json::json!({ "users": [] }))]);
        let (etx, erx, sink, lrx) = channels();
        invite_task(&http, invite_params(&my_xk, [0u8; 32]), &etx, &sink).await;

        match lrx.try_recv().expect("ApiError emitted") {
            LogicEvent::ApiError { code, .. } => assert_eq!(code, 404),
            other => panic!("unexpected: {other:?}"),
        }
        assert!(erx.try_recv().is_err(), "no success event");
        // Only the search ran; no share POST attempted.
        assert_eq!(http.calls().len(), 1);
    }

    #[tokio::test]
    async fn invite_search_failure_emits_500() {
        let (my_xk, _) = keypair(1);
        let http = FakeHttp::new(vec![Err("network down".into())]);
        let (etx, _erx, sink, lrx) = channels();
        invite_task(&http, invite_params(&my_xk, [0u8; 32]), &etx, &sink).await;

        match lrx.try_recv().expect("ApiError emitted") {
            LogicEvent::ApiError { code, message, .. } => {
                assert_eq!(code, 500);
                assert!(message.contains("User search failed"));
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[tokio::test]
    async fn invite_pin_mismatch_refuses_and_never_wraps() {
        // The server serves a DIFFERENT key at phase 2 than the one pinned at
        // phase 1 (the TOCTOU attack). The pin must refuse before any wrap.
        let (my_xk, _) = keypair(1);
        let (_confirmed_xk, confirmed_hex) = keypair(2);
        let confirmed = decode_exchange_pubkey(&confirmed_hex).unwrap();
        let (_substituted_xk, substituted_hex) = keypair(9);

        let http = FakeHttp::new(vec![Ok(user_search_json(
            "invitee@test.local",
            &substituted_hex, // server returns the substituted key
        ))]);
        let (etx, erx, sink, lrx) = channels();
        // We confirmed `confirmed`, not the substituted key the server now serves.
        invite_task(&http, invite_params(&my_xk, confirmed), &etx, &sink).await;

        match lrx.try_recv().expect("ApiError emitted") {
            LogicEvent::ApiError { code, message, .. } => {
                assert_eq!(code, 409);
                assert!(message.contains("key changed"), "got: {message}");
            }
            other => panic!("unexpected: {other:?}"),
        }
        assert!(erx.try_recv().is_err(), "no InviteSent on mismatch");
        // Only the search ran: nothing was wrapped, no share POST fired.
        assert_eq!(http.calls().len(), 1);
    }

    fn fetch_key_params() -> FetchInviteeKeyParams {
        FetchInviteeKeyParams {
            id: RequestId::next(),
            document_id: DocumentId::new(),
            url_scheme: "https",
            api_base: "api.test".into(),
            invitee_email: "invitee@test.local".into(),
        }
    }

    #[tokio::test]
    async fn fetch_invitee_key_emits_key_and_matching_fingerprint() {
        let (_invitee_xk, invitee_pub_hex) = keypair(2);
        let invitee_pub = decode_exchange_pubkey(&invitee_pub_hex).unwrap();

        let http = FakeHttp::new(vec![Ok(user_search_json(
            "invitee@test.local",
            &invitee_pub_hex,
        ))]);
        let (etx, erx, sink, lrx) = channels();
        fetch_invitee_key_task(&http, fetch_key_params(), &etx, &sink).await;

        match erx.try_recv().expect("InviteeKeyReady emitted") {
            SyncEvent::InviteeKeyReady {
                exchange_pub_key,
                fingerprint,
                email,
                ..
            } => {
                assert_eq!(exchange_pub_key, invitee_pub);
                assert_eq!(email, "invitee@test.local");
                // The fingerprint is the one the chrome will display: it matches
                // an independent recomputation over the same key.
                assert_eq!(fingerprint, ambre_crypto::fingerprint(&invitee_pub));
            }
            _ => panic!("unexpected event (want InviteeKeyReady)"),
        }
        assert!(lrx.try_recv().is_err(), "no ApiError on success");
        // Phase 1 only searches: no wrapping, no share POST.
        assert_eq!(http.calls().len(), 1);
    }

    #[tokio::test]
    async fn fetch_invitee_key_unknown_user_emits_404() {
        let http = FakeHttp::new(vec![Ok(serde_json::json!({ "users": [] }))]);
        let (etx, erx, sink, lrx) = channels();
        fetch_invitee_key_task(&http, fetch_key_params(), &etx, &sink).await;

        match lrx.try_recv().expect("ApiError emitted") {
            LogicEvent::ApiError { code, .. } => assert_eq!(code, 404),
            other => panic!("unexpected: {other:?}"),
        }
        assert!(erx.try_recv().is_err(), "no key event for unknown user");
    }

    fn revoke_params(my_xk: &StaticSecret) -> RevokeParams {
        RevokeParams {
            id: RequestId::next(),
            document_id: DocumentId::new(),
            user_id: UserId::new(),
            url_scheme: "https",
            api_base: "api.test".into(),
            exchange_key: my_xk.clone(),
            previous: DocumentKey::generate(),
            author: author(),
        }
    }

    /// The link a request carried, read back as the relay would.
    fn posted_link(body: &serde_json::Value) -> ambre_protocol::KeyLink {
        let text = body["link"].as_str().expect("the request carries a link");
        let bytes = base64::engine::general_purpose::URL_SAFE
            .decode(text)
            .unwrap();
        ambre_protocol::KeyLink::decode(&bytes).unwrap()
    }

    /// A members row, for the revoke and the debt tests.
    fn member_row(uid: &str, name: &str, pub_hex: &str) -> serde_json::Value {
        serde_json::json!({
            "user_id": uid,
            "display_name": name, "email": "m@test.local",
            "signing_pub_key": "00".repeat(32), "exchange_pub_key": pub_hex,
            "doc_key_enc": "", "role": "editor",
        })
    }

    /// A revoke posts the removal and the copies in one request, and wraps the
    /// new key for everyone but the member removed (`SYNC-041`, `SYNC-126`).
    #[tokio::test]
    async fn a_revoke_posts_the_removal_and_the_copies_in_one_request() {
        let (my_xk, _) = keypair(1);
        let (_a_xk, a_pub) = keypair(2);
        let (_c_xk, c_pub) = keypair(3);
        let params = revoke_params(&my_xk);
        let removed = params.user_id.to_string();
        let document_id = params.document_id;
        let previous = params.previous.id();

        let http = FakeHttp::new(vec![
            Ok(serde_json::json!({ "members": [
                member_row("11111111-1111-1111-1111-111111111111", "A", &a_pub),
                member_row(&removed, "Carol", &c_pub),
            ] })), // GET members
            Ok(serde_json::json!({ "status": "revoked" })), // POST revoke
        ]);
        let (etx, erx, sink, lrx) = channels();
        revoke_task(&http, params, &etx, &sink).await;

        let SyncEvent::RevocationComplete {
            new_doc_key, link, ..
        } = erx.try_recv().expect("RevocationComplete")
        else {
            panic!("expected RevocationComplete");
        };
        assert!(lrx.try_recv().is_err());
        // The link the revoke posted seals the key it replaced under the new
        // one, and the client keeps the same link (`SYNC-116`, `SYNC-118`).
        let opened = crate::pipeline::open_link(&link, document_id, &new_doc_key).unwrap();
        assert_eq!(opened.id(), previous);

        let calls = http.calls();
        assert_eq!(calls.len(), 2, "one read, then one write");
        assert!(matches!(&calls[0], Call::Get(u) if u.ends_with("/members")));
        match &calls[1] {
            Call::Post(url, body) => {
                assert!(url.ends_with("/revoke"));
                assert_eq!(body["user_id"], removed.as_str());
                let keys = body["keys"].as_array().unwrap();
                assert_eq!(keys.len(), 1, "no copy for the member removed");
                assert_eq!(keys[0]["user_id"], "11111111-1111-1111-1111-111111111111");
                assert_eq!(posted_link(body), link);
            }
            other => panic!("expected the revoke POST, got {other:?}"),
        }
    }

    /// `SYNC-143`: a member joined between the revoke's read and its post. The
    /// relay refused it whole, so the revoke reads the members again and wraps
    /// the same new key for the newcomer too. Each post names what it read.
    #[tokio::test]
    async fn a_revoke_refused_for_a_newcomer_is_built_again_for_them() {
        let (my_xk, _) = keypair(1);
        let (_a_xk, a_pub) = keypair(2);
        let (_c_xk, c_pub) = keypair(3);
        let (_n_xk, n_pub) = keypair(4);
        let params = revoke_params(&my_xk);
        let removed = params.user_id.to_string();
        let a = "11111111-1111-1111-1111-111111111111";
        let newcomer = "44444444-4444-4444-4444-444444444444";

        let http = FakeHttp::new(vec![
            Ok(serde_json::json!({ "members": [
                member_row(a, "A", &a_pub), member_row(&removed, "Carol", &c_pub),
            ] })),
            Err("POST https://api.test/revoke: MEMBERS_CHANGED".into()),
            Ok(serde_json::json!({ "members": [
                member_row(a, "A", &a_pub), member_row(&removed, "Carol", &c_pub),
                member_row(newcomer, "New", &n_pub),
            ] })),
            Ok(serde_json::json!({ "status": "revoked" })),
        ]);
        let (etx, erx, sink, lrx) = channels();
        revoke_task(&http, params, &etx, &sink).await;

        let Ok(SyncEvent::RevocationComplete { link, .. }) = erx.try_recv() else {
            panic!("expected RevocationComplete");
        };
        assert!(
            lrx.try_recv().is_err(),
            "a retry that lands reports nothing"
        );
        let calls = http.calls();
        assert_eq!(calls.len(), 4, "read, refused post, read again, post");
        let (Call::Post(_, first), Call::Post(_, second)) = (&calls[1], &calls[3]) else {
            panic!("expected two posts: {calls:?}");
        };
        assert_eq!(first["members"], serde_json::json!([a]));
        assert_eq!(second["members"], serde_json::json!([a, newcomer]));
        let wrapped: Vec<_> = second["keys"]
            .as_array()
            .unwrap()
            .iter()
            .map(|k| k["user_id"].as_str().unwrap())
            .collect();
        assert_eq!(wrapped, vec![a, newcomer], "the newcomer gets a copy");
        assert_eq!(posted_link(first), posted_link(second), "one new key");
        assert_eq!(posted_link(second), link);
    }

    /// `SYNC-143`: members that keep joining end the revoke after its tries,
    /// with the refusal reported and no key swapped.
    #[tokio::test]
    async fn a_revoke_gives_up_after_its_tries() {
        let (my_xk, _) = keypair(1);
        let (_a_xk, a_pub) = keypair(2);
        let members = serde_json::json!({ "members": [
            member_row("11111111-1111-1111-1111-111111111111", "A", &a_pub),
        ] });
        let mut script = Vec::new();
        for _ in 0..ROTATION_TRIES {
            script.push(Ok(members.clone()));
            script.push(Err("POST https://api.test/revoke: MEMBERS_CHANGED".into()));
        }
        let http = FakeHttp::new(script);
        let (etx, erx, sink, lrx) = channels();
        revoke_task(&http, revoke_params(&my_xk), &etx, &sink).await;
        assert!(erx.try_recv().is_err(), "no key swapped");
        assert!(matches!(
            lrx.try_recv(),
            Ok(LogicEvent::ApiError { code: 500, .. })
        ));
        assert_eq!(http.calls().len(), 2 * ROTATION_TRIES);
    }

    /// `SYNC-143`: an owed rotation refused for a newcomer reads the members
    /// again, and the payment wraps for the newcomer too.
    #[tokio::test]
    async fn an_owed_rotation_refused_for_a_newcomer_is_paid_for_them() {
        let (my_xk, my_pub) = keypair(1);
        let (_n_xk, n_pub) = keypair(4);
        let me = "11111111-1111-1111-1111-111111111111";
        let newcomer = "44444444-4444-4444-4444-444444444444";
        let http = FakeHttp::new(vec![
            Ok(
                serde_json::json!({ "members": [ member_row(me, "Me", &my_pub) ],
                "rotation_owed": true }),
            ),
            Err("POST https://api.test/rotate-key: MEMBERS_CHANGED".into()),
            Ok(serde_json::json!({ "members": [
                member_row(me, "Me", &my_pub), member_row(newcomer, "New", &n_pub),
            ], "rotation_owed": true })),
            Ok(serde_json::json!({ "status": "rotated" })),
        ]);
        let (etx, erx, _sink, _lrx) = channels();
        settle_task(&http, settle_params(&my_xk), &etx).await;
        assert!(matches!(erx.try_recv(), Ok(SyncEvent::DebtSettled { .. })));
        let calls = http.calls();
        let Call::Post(_, body) = &calls[3] else {
            panic!("expected the second payment: {calls:?}");
        };
        assert_eq!(body["members"], serde_json::json!([me, newcomer]));
        assert_eq!(body["keys"].as_array().unwrap().len(), 2);
    }

    /// `SYNC-143`: an invitation names the key its copy wraps, so the relay
    /// can refuse one a rotation overtook.
    #[test]
    fn an_invitation_names_the_key_it_wraps() {
        let (my_xk, _) = keypair(1);
        let (_i_xk, i_pub) = keypair(2);
        let doc_key = DocumentKey::generate();
        let req = build_invite_request(
            &my_xk,
            &i_pub,
            "i@test".into(),
            &doc_key,
            Role::Editor,
            None,
            None,
        )
        .unwrap();
        assert_eq!(req.key_id, doc_key.id());
    }

    /// `SYNC-143`: an invitation the relay refused for a key that changed is
    /// reported as such, and nothing is sent as invited.
    #[tokio::test]
    async fn an_invitation_overtaken_by_a_rotation_is_reported() {
        let (my_xk, _) = keypair(1);
        let (_i_xk, i_pub) = keypair(2);
        let expect = decode_exchange_pubkey(&i_pub).unwrap();
        let http = FakeHttp::new(vec![
            Ok(user_search_json("invitee@test.local", &i_pub)),
            Ok(serde_json::json!({ "members": [] })),
            Err("POST https://api.test/share: KEY_CHANGED".into()),
        ]);
        let (etx, erx, sink, lrx) = channels();
        invite_task(&http, invite_params(&my_xk, expect), &etx, &sink).await;
        assert!(erx.try_recv().is_err());
        assert!(matches!(
            lrx.try_recv(),
            Ok(LogicEvent::ApiError { code: 409, .. })
        ));
    }

    /// A member who leaves a channel finds its `meta`. They post their own
    /// removal there, signed and alone: the members who stay owe the
    /// rotation.
    #[tokio::test]
    async fn leaving_a_channel_posts_this_users_removal_on_its_meta() {
        let owner = Peer::new(1);
        let bob = Peer::new(2);
        let mut logged = Logged::new(&owner);
        logged.invite(&owner, &bob, Role::Editor);
        let me = bob.author.user_id;
        let meta = logged.document_id;
        let channel_id = ChannelId::new();
        let listing = serde_json::json!({ "documents": [
            { "document_id": DocumentId::new().to_string(), "kind": "segment" },
            { "document_id": meta.to_string(), "kind": "meta" },
        ]});
        let http = FakeHttp::new(vec![
            Ok(listing),
            Ok(logged.answer()),
            Ok(serde_json::json!({ "status": "left" })),
        ]);
        let params = ChannelLeaveParams {
            channel_id,
            author: same(&bob.author),
            url_scheme: "https",
            api_base: "api.test".into(),
            exchange_key: bob.xk.clone(),
        };
        assert_eq!(leave_channel_task(&http, params).await, Ok(()));

        let calls = http.calls();
        assert_eq!(calls.len(), 3, "the listing, the members, the removal");
        assert!(
            matches!(&calls[0], Call::Get(url) if url.ends_with(&format!("/channels/{channel_id}/documents")))
        );
        assert!(matches!(&calls[1], Call::Get(url) if url.ends_with(&format!("/{meta}/members"))));
        let Call::Delete(url, body) = &calls[2] else {
            panic!("expected the leave DELETE, got {:?}", calls[2]);
        };
        assert!(url.ends_with(&format!("/documents/{meta}/share/{me}")));
        let entry = posted_entry(body);
        assert_eq!(
            (entry.member, entry.author, entry.act),
            (me, me, MembershipAct::Revoke)
        );
        assert!(body.get("keys").is_none(), "the leaver picks no key");
    }

    /// A leave the relay refuses, or a channel with no `meta`, answers the
    /// reason, and nothing more is posted.
    #[tokio::test]
    async fn a_refused_leave_answers_its_reason() {
        let params = || ChannelLeaveParams {
            channel_id: ChannelId::new(),
            author: author(),
            url_scheme: "https",
            api_base: "api.test".into(),
            exchange_key: keypair(1).0,
        };
        let http = FakeHttp::new(vec![Err("404".into())]);
        assert_eq!(leave_channel_task(&http, params()).await, Err("404".into()));

        let no_meta = serde_json::json!({ "documents": [
            { "document_id": DocumentId::new().to_string(), "kind": "segment" },
        ]});
        let http = FakeHttp::new(vec![Ok(no_meta)]);
        assert!(leave_channel_task(&http, params()).await.is_err());
        assert_eq!(http.calls().len(), 1, "nothing is posted");

        let listing = serde_json::json!({ "documents": [
            { "document_id": DocumentId::new().to_string(), "kind": "meta" },
        ]});
        let http = FakeHttp::new(vec![
            Ok(listing),
            Ok(serde_json::json!({ "members": [] })),
            Err("HTTP 403".into()),
        ]);
        assert!(leave_channel_task(&http, params()).await.is_err());
        assert_eq!(http.calls().len(), 3, "a refusal is not tried again");
    }

    /// A revoke the relay refuses changes nothing here either: no key is
    /// swapped, and the user is told (`SYNC-041`).
    #[tokio::test]
    async fn a_refused_revoke_swaps_no_key() {
        let (my_xk, _) = keypair(1);
        let (_a_xk, a_pub) = keypair(2);
        let http = FakeHttp::new(vec![
            Ok(serde_json::json!({ "members": [
                member_row("11111111-1111-1111-1111-111111111111", "A", &a_pub),
            ] })),
            Err("HTTP 500".into()),
        ]);
        let (etx, erx, sink, lrx) = channels();
        revoke_task(&http, revoke_params(&my_xk), &etx, &sink).await;
        assert!(erx.try_recv().is_err(), "no key to install");
        assert!(matches!(
            lrx.try_recv().expect("ApiError"),
            LogicEvent::ApiError { code: 500, .. }
        ));
        assert_eq!(http.calls().len(), 2);
    }

    /// Leaving deletes this user's own share and posts nothing else: no key is
    /// generated, so none reaches the members who stay (`SYNC-127`, `SYNC-130`).
    #[tokio::test]
    async fn leaving_posts_the_removal_alone_and_chooses_no_key() {
        let (my_xk, _) = keypair(1);
        let author = author();
        let me = author.user_id;
        let http = FakeHttp::new(vec![
            Ok(serde_json::json!({ "members": [] })),
            Ok(serde_json::json!({ "status": "left" })),
        ]);
        let (etx, erx, sink, lrx) = channels();
        let params = LeaveParams {
            id: RequestId::next(),
            document_id: DocumentId::new(),
            author,
            url_scheme: "https",
            api_base: "api.test".into(),
            exchange_key: my_xk,
        };
        leave_task(&http, params, &etx, &sink).await;
        assert!(matches!(
            erx.try_recv().expect("DocumentLeft"),
            SyncEvent::DocumentLeft { .. }
        ));
        assert!(lrx.try_recv().is_err());
        let calls = http.calls();
        assert_eq!(
            calls.len(),
            2,
            "the members read, then the removal: no POST"
        );
        assert!(matches!(&calls[0], Call::Get(_)));
        assert!(matches!(&calls[1], Call::Delete(u, _) if u.ends_with(&format!("/share/{me}"))));
    }

    fn settle_params(my_xk: &StaticSecret) -> SettleParams {
        SettleParams {
            document_id: DocumentId::new(),
            url_scheme: "https",
            api_base: "api.test".into(),
            exchange_key: my_xk.clone(),
            previous: DocumentKey::generate(),
            signing_key: MasterSeed::from_array([9; 32]).derive_signing_key(),
        }
    }

    /// An owed rotation is paid with a fresh key wrapped for every member. The
    /// key reaches the caller only once the relay stored it (`SYNC-128`).
    #[tokio::test]
    async fn an_owed_rotation_is_paid_for_every_member() {
        let (my_xk, my_pub) = keypair(1);
        let (_a_xk, a_pub) = keypair(2);
        let http = FakeHttp::new(vec![
            Ok(serde_json::json!({ "rotation_owed": true, "members": [
                member_row("11111111-1111-1111-1111-111111111111", "Me", &my_pub),
                member_row("22222222-2222-2222-2222-222222222222", "A", &a_pub),
            ] })),
            Ok(serde_json::json!({ "status": "rotated" })),
        ]);
        let (etx, erx, _sink, _lrx) = channels();
        let params = settle_params(&my_xk);
        let (document_id, previous) = (params.document_id, params.previous.id());
        settle_task(&http, params, &etx).await;
        let SyncEvent::DebtSettled {
            new_doc_key, link, ..
        } = erx.try_recv().expect("DebtSettled")
        else {
            panic!("expected DebtSettled");
        };
        let opened = crate::pipeline::open_link(&link, document_id, &new_doc_key).unwrap();
        assert_eq!(opened.id(), previous, "the rotation leaves its link");
        match &http.calls()[1] {
            Call::Post(url, body) => {
                assert!(url.ends_with("/rotate-key"));
                assert_eq!(body["keys"].as_array().unwrap().len(), 2);
                assert_eq!(posted_link(body), link);
            }
            other => panic!("expected the rotate POST, got {other:?}"),
        }
    }

    /// A space admin re-keys a public channel for the members meta lists, and
    /// reads them again when one joined meanwhile (the Murmure spaces spec).
    /// The new key comes back sealed under the current space key, with meta's
    /// anchor. The link it leaves opens the old key.
    #[tokio::test]
    async fn a_public_channel_is_rekeyed_under_the_current_space_key() {
        let (my_xk, my_pub) = keypair(1);
        let (_a_xk, a_pub) = keypair(2);
        let (space_id, channel_id) = (ambre_bus::SpaceId::new(), ChannelId::new());
        let meta = ambre_core::MetaId::new();
        let head = serde_json::json!({
            "meta_id": meta.to_string(), "role": "editor", "length": 1,
        });
        let one = serde_json::json!({ "members": [
            member_row("11111111-1111-1111-1111-111111111111", "Me", &my_pub),
        ] });
        let two = serde_json::json!({ "members": [
            member_row("11111111-1111-1111-1111-111111111111", "Me", &my_pub),
            member_row("22222222-2222-2222-2222-222222222222", "A", &a_pub),
        ] });
        let changed = format!("POST x: {}", ambre_core::RefusalCode::MembersChanged);
        let http = FakeHttp::new(vec![
            Ok(head),
            Ok(one),
            Err(changed),
            Ok(two),
            Ok(serde_json::json!({ "status": "rotated" })),
        ]);
        let (etx, erx, _sink, _lrx) = channels();
        let (previous, space_key) = (DocumentKey::generate(), DocumentKey::generate());
        let anchor = EntryDigest([5; 32]);
        let params = RotateChannelParams {
            space_id,
            channel_id,
            url_scheme: "https",
            api_base: "api.test".into(),
            exchange_key: my_xk.clone(),
            signing_key: MasterSeed::from_array([9; 32]).derive_signing_key(),
            previous: previous.clone(),
            anchor,
            space_key: space_key.clone(),
        };
        rotate_channel_task(&http, params, &etx).await;
        let SyncEvent::ChannelRotated {
            space_id: s,
            channel_id: c,
            result: Ok(sealed),
        } = erx.try_recv().expect("ChannelRotated")
        else {
            panic!("expected a rotated channel");
        };
        assert_eq!((s, c), (space_id, channel_id));
        assert_eq!(crate::channels::sealed_under(&sealed), Some(space_key.id()));
        let keys = crate::channels::tests::ring(&space_key, &[]);
        let (new_key, sealed_anchor) =
            crate::channels::unseal_channel_key(channel_id, &sealed, &keys).unwrap();
        assert_eq!(sealed_anchor, Some(anchor));
        {
            let calls = http.calls();
            assert_eq!(calls.len(), 5);
            assert!(
                matches!(&calls[0], Call::Get(u) if u.ends_with(&format!("/channels/{channel_id}/join")))
            );
            let rotate = format!("/documents/{}/rotate-key", meta.document_id());
            match &calls[4] {
                Call::Post(url, body) => {
                    assert!(url.ends_with(&rotate));
                    assert_eq!(body["keys"].as_array().unwrap().len(), 2, "read again");
                    let link = posted_link(body);
                    let opened =
                        crate::pipeline::open_link(&link, meta.document_id(), &new_key).unwrap();
                    assert_eq!(opened.id(), previous.id());
                }
                other => panic!("expected the rotate POST, got {other:?}"),
            }
        }

        // A refusal ends the rotation with its error.
        let http = FakeHttp::new(vec![Err("HTTP 404".into())]);
        let params = RotateChannelParams {
            space_id,
            channel_id,
            url_scheme: "https",
            api_base: "api.test".into(),
            exchange_key: my_xk,
            signing_key: MasterSeed::from_array([9; 32]).derive_signing_key(),
            previous,
            anchor,
            space_key,
        };
        rotate_channel_task(&http, params, &etx).await;
        assert!(matches!(
            erx.try_recv(),
            Ok(SyncEvent::ChannelRotated { result: Err(_), .. })
        ));
    }

    /// A debt already paid posts nothing, and a failed payment leaves the debt
    /// for the next edit (`SYNC-128`).
    #[tokio::test]
    async fn a_paid_debt_posts_nothing_and_a_failed_one_is_retried() {
        let (my_xk, _) = keypair(1);
        let http = FakeHttp::new(vec![Ok(serde_json::json!({ "members": [] }))]);
        let (etx, erx, _sink, _lrx) = channels();
        settle_task(&http, settle_params(&my_xk), &etx).await;
        assert!(matches!(
            erx.try_recv().expect("DebtGone"),
            SyncEvent::DebtGone { .. }
        ));
        assert_eq!(http.calls().len(), 1);

        let http = FakeHttp::new(vec![
            Ok(serde_json::json!({ "rotation_owed": true, "members": [] })),
            Err("HTTP 403".into()),
        ]);
        settle_task(&http, settle_params(&my_xk), &etx).await;
        assert!(matches!(
            erx.try_recv().expect("DebtUnsettled"),
            SyncEvent::DebtUnsettled { .. }
        ));
    }

    /// A malformed identifier from the server stops the key verification. The
    /// search answer is refused when read, so no key is pinned under an
    /// invented id (`CHROME-074`).
    #[tokio::test]
    async fn fetch_invitee_key_refuses_a_malformed_id() {
        let (_invitee_xk, invitee_pub_hex) = keypair(2);
        let mut json = user_search_json("invitee@test.local", &invitee_pub_hex);
        json["users"][0]["id"] = serde_json::json!("not-a-uuid");
        let http = FakeHttp::new(vec![Ok(json)]);
        let (etx, erx, sink, lrx) = channels();
        fetch_invitee_key_task(&http, fetch_key_params(), &etx, &sink).await;
        assert!(erx.try_recv().is_err(), "no key to pin");
        assert!(matches!(
            lrx.try_recv().expect("ApiError"),
            LogicEvent::ApiError { code: 500, .. }
        ));
    }

    /// A member the rotation cannot re-wrap for is reported by name, after the
    /// rotation went through for the others (`SYNC-043`).
    #[tokio::test]
    async fn revoke_reports_a_member_it_could_not_rewrap_for() {
        let (my_xk, _) = keypair(1);
        let (_a_xk, a_pub) = keypair(2);
        let http = FakeHttp::new(vec![
            Ok(serde_json::json!({ "members": [
                {
                    "user_id": "11111111-1111-1111-1111-111111111111",
                    "display_name": "A", "email": "a@test.local",
                    "signing_pub_key": "00", "exchange_pub_key": a_pub,
                    "doc_key_enc": "", "role": "editor",
                },
                {
                    "user_id": "22222222-2222-2222-2222-222222222222",
                    "display_name": "Broken", "email": "b@test.local",
                    "signing_pub_key": "00", "exchange_pub_key": "garbage",
                    "doc_key_enc": "", "role": "editor",
                },
            ] })),
            Ok(serde_json::json!({ "status": "revoked" })),
        ]);
        let (etx, erx, sink, lrx) = channels();
        revoke_task(&http, revoke_params(&my_xk), &etx, &sink).await;
        assert!(matches!(
            erx.try_recv().expect("RevocationComplete"),
            SyncEvent::RevocationComplete { .. }
        ));
        match lrx.try_recv().expect("the skipped member is reported") {
            LogicEvent::ApiError { message, .. } => {
                assert!(message.contains("Broken"), "{message}")
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[tokio::test]
    async fn revoke_stops_when_the_members_cannot_be_read() {
        let (my_xk, _) = keypair(1);
        let http = FakeHttp::new(vec![Err("403".into())]);
        let (etx, erx, sink, lrx) = channels();
        revoke_task(&http, revoke_params(&my_xk), &etx, &sink).await;

        match lrx.try_recv().expect("ApiError emitted") {
            LogicEvent::ApiError { code, message, .. } => {
                assert_eq!(code, 500);
                assert!(message.contains("Get members failed"));
            }
            other => panic!("unexpected: {other:?}"),
        }
        assert!(erx.try_recv().is_err());
        // Stopped after the failed read: nothing posted, so nothing removed.
        assert_eq!(http.calls().len(), 1);
    }

    fn list_params() -> ListMembersParams {
        ListMembersParams {
            id: Some(RequestId::next()),
            asked: 1,
            document_id: DocumentId::new(),
            url_scheme: "https",
            api_base: "api.test".into(),
            exchange_key: keypair(1).0,
            rule: LogRule::document(),
        }
    }

    #[tokio::test]
    async fn list_members_maps_on_success_and_errors_otherwise() {
        // Success.
        let http = FakeHttp::new(vec![Ok(serde_json::json!({ "members": [ {
            "user_id": "11111111-1111-1111-1111-111111111111",
            "display_name": "A", "email": "a@test.local",
            "signing_pub_key": "00", "exchange_pub_key": "ab",
            "doc_key_enc": "", "role": "viewer",
        } ] }))]);
        let (etx, erx, _sink, _lrx) = channels();
        list_members_task(&http, list_params(), &etx).await;
        match erx.try_recv().expect("DocumentMembersReady") {
            SyncEvent::DocumentMembersReady { members, .. } => {
                assert_eq!(members.len(), 1);
                assert_eq!(members[0].role, Role::Viewer);
            }
            _ => panic!("unexpected event (want DocumentMembersReady)"),
        }

        // Failure -> HttpError.
        let http = FakeHttp::new(vec![Err("boom".into())]);
        let (etx, erx, _sink, _lrx) = channels();
        list_members_task(&http, list_params(), &etx).await;
        assert!(matches!(
            erx.try_recv().expect("HttpError"),
            SyncEvent::HttpError { .. }
        ));
    }

    // -- Presence directory (`sync`, "Presence identity") ----------------------

    #[test]
    fn presence_directory_maps_valid_entries() {
        let uid = UserId::parse_str("11111111-1111-1111-1111-111111111111").unwrap();
        let key_hex = "aa".repeat(32);
        let dir = presence_directory([(uid, key_hex.as_str())]);

        assert_eq!(dir, vec![([0xaa; 32], uid)]);
    }

    #[test]
    fn presence_directory_skips_unparsable_rows_without_inventing_ids() {
        // A bad row must vanish. A fabricated id would attribute a cursor to
        // the wrong person, which is what keying presence on the UserId
        // prevents. A malformed id never gets here: the answer was refused.
        let good_uid = UserId::parse_str("11111111-1111-1111-1111-111111111111").unwrap();
        let good_key = "bb".repeat(32);
        let short_key = "bb".repeat(31); // 31 bytes: not a valid Ed25519 key
        let dir = presence_directory([
            (good_uid, "zz-not-hex"),
            (good_uid, short_key.as_str()),
            (good_uid, good_key.as_str()),
        ]);

        assert_eq!(dir, vec![([0xbb; 32], good_uid)]);
    }

    // -- Own role (architecture) -----------------------------------------

    #[test]
    fn own_role_reads_our_own_row() {
        let me = UserId::parse_str("11111111-1111-1111-1111-111111111111").unwrap();
        let other = UserId::parse_str("22222222-2222-2222-2222-222222222222").unwrap();
        let role = own_role([(other, Role::Owner), (me, Role::Viewer)], Some(me));

        // Our own row wins: a peer being owner says nothing about our access.
        assert_eq!(role, Role::Viewer);
    }

    #[test]
    fn own_role_falls_back_to_viewer_when_unresolvable() {
        let me = UserId::parse_str("11111111-1111-1111-1111-111111111111").unwrap();
        let other = UserId::parse_str("22222222-2222-2222-2222-222222222222").unwrap();

        // No session id yet.
        assert_eq!(own_role([(other, Role::Owner)], None), Role::Viewer);
        // No row for us in the response.
        assert_eq!(own_role([(other, Role::Owner)], Some(me)), Role::Viewer);
        // A valid role that is not a document role (channel / org side).
        assert_eq!(own_role([(me, Role::Admin)], Some(me)), Role::Viewer);
    }

    #[test]
    fn own_role_reads_writable_roles() {
        let me = UserId::parse_str("11111111-1111-1111-1111-111111111111").unwrap();
        for role in [Role::Owner, Role::Editor] {
            assert_eq!(own_role([(me, role)], Some(me)), role);
            assert!(role.can_write_document());
        }
    }

    #[tokio::test]
    async fn list_members_refreshes_the_presence_directory() {
        let uid = "11111111-1111-1111-1111-111111111111";
        let key_hex = "cc".repeat(32);
        let http = FakeHttp::new(vec![Ok(serde_json::json!({ "members": [ {
            "user_id": uid,
            "display_name": "A", "email": "a@test.local",
            "signing_pub_key": key_hex, "exchange_pub_key": "ab",
            "doc_key_enc": "", "role": "viewer",
        } ] }))]);
        let (etx, erx, _sink, _lrx) = channels();
        list_members_task(&http, list_params(), &etx).await;

        match erx.try_recv().expect("DocumentMembersReady") {
            SyncEvent::DocumentMembersReady { signing_keys, .. } => {
                assert_eq!(
                    signing_keys,
                    vec![([0xcc; 32], UserId::parse_str(uid).unwrap())]
                );
            }
            _ => panic!("unexpected event (want DocumentMembersReady)"),
        }
    }

    // -- The signed membership log (`ambre-spec-membership.md`).

    use ambre_bus::MemberStanding;

    /// A copy of `author`, for a task that takes its own.
    fn same(author: &Author) -> Author {
        Author {
            user_id: author.user_id,
            signing_key: author.signing_key.clone(),
        }
    }

    /// One member of a logged document: an identity, a signing key and an
    /// exchange key.
    struct Peer {
        author: Author,
        xk: StaticSecret,
    }

    impl Peer {
        fn new(seed: u8) -> Self {
            Self {
                author: Author {
                    user_id: UserId::from_bytes([seed; 16]),
                    signing_key: MasterSeed::from_array([seed; 32]).derive_signing_key(),
                },
                xk: keypair(seed).0,
            }
        }

        fn exchange_hex(&self) -> String {
            hex::encode(PublicKey::from(&self.xk).to_bytes())
        }
    }

    /// A document `owner` created with a log: its key, the rows and the
    /// entries the relay serves for it.
    struct Logged {
        document_id: DocumentId,
        doc_key: DocumentKey,
        anchor: EntryDigest,
        owner_exchange: String,
        rows: Vec<serde_json::Value>,
        log: Vec<String>,
    }

    impl Logged {
        fn new(owner: &Peer) -> Self {
            let document_id = DocumentId::new();
            let doc_key = DocumentKey::generate();
            let (enc, entry) = creation(document_id, &doc_key, &owner.xk, &owner.author).unwrap();
            let anchor = membership_log::decode(std::slice::from_ref(&entry))
                .first()
                .and_then(membership_log::digest)
                .unwrap();
            let mut logged = Self {
                document_id,
                doc_key,
                anchor,
                owner_exchange: owner.exchange_hex(),
                rows: Vec::new(),
                log: vec![entry],
            };
            logged.row(owner, enc, Role::Owner);
            logged
        }

        fn row(&mut self, peer: &Peer, doc_key_enc: String, role: Role) {
            self.rows.push(serde_json::json!({
                "user_id": peer.author.user_id.to_string(),
                "display_name": "Member", "email": "m@test.local",
                "signing_pub_key": signing_hex(&peer.author.signing_key),
                "exchange_pub_key": peer.exchange_hex(),
                "wrapper_exchange_pub_key": self.owner_exchange,
                "doc_key_enc": doc_key_enc, "role": role.to_string(),
            }));
        }

        /// The log walked from the anchor, with `more` entries after it.
        fn walk(&self, more: &[&str]) -> Log {
            let mut texts = self.log.clone();
            texts.extend(more.iter().map(|text| text.to_string()));
            walk_served_under(
                self.document_id,
                Some(self.anchor),
                &texts,
                &LogRule::document(),
            )
        }

        /// `owner` invites `peer`: the entry joins the log and the row joins
        /// the members, with a copy sealing the anchor.
        fn invite(&mut self, owner: &Peer, peer: &Peer, role: Role) {
            let draft = Draft {
                document: self.document_id,
                member: peer.author.user_id,
                member_key: peer.author.signing_key.verifying_key().to_bytes(),
                act: MembershipAct::Invite,
                role,
                key: Some(self.doc_key.id()),
            };
            let entry = entry_after(&self.walk(&[]), draft, &owner.author)
                .unwrap()
                .unwrap();
            self.log.push(entry);
            let peer_pub = PublicKey::from(&peer.xk).to_bytes();
            let enc =
                wrap_anchored_b64(&owner.xk, &peer_pub, &self.doc_key, Some(self.anchor)).unwrap();
            self.row(peer, enc, role);
        }

        fn answer(&self) -> serde_json::Value {
            serde_json::json!({ "members": self.rows, "log": self.log })
        }
    }

    /// The entry a request body carries, decoded.
    fn posted_entry(body: &serde_json::Value) -> ambre_protocol::MembershipEntry {
        let text = body["entry"]
            .as_str()
            .expect("the request carries an entry");
        membership_log::decode(&[text.to_string()])
            .pop()
            .expect("the entry decodes")
    }

    /// Holds `SYNC-132` and `SYNC-104` at the creation: the creator signs the
    /// first entry, and its own copy of the key seals that entry's digest.
    #[test]
    fn a_new_document_signs_its_first_entry_and_seals_its_digest() {
        let owner = Peer::new(1);
        let logged = Logged::new(&owner);
        let resp: http::DocumentMembersResponse = serde_json::from_value(logged.answer()).unwrap();
        let (log, anchor) = served_log(logged.document_id, &owner.xk, &resp);
        assert_eq!(anchor, Some(logged.anchor));
        assert_eq!(
            log.standing(owner.author.user_id),
            (MemberStanding::Verified, Some(Role::Owner))
        );
        assert_eq!(log.wraps(owner.author.user_id), Some(logged.doc_key.id()));
    }

    /// Holds `SYNC-105`, `SYNC-123` and `SYNC-132` at the invite. One request
    /// carries the copy and the entry. The entry names the key the copy wraps
    /// and links to the log, and the copy seals the anchor (`SYNC-104`).
    #[tokio::test]
    async fn an_invite_posts_its_signed_entry_beside_the_copy() {
        let owner = Peer::new(1);
        let logged = Logged::new(&owner);
        let (invitee_xk, invitee_hex) = keypair(2);
        let mut params = invite_params(&owner.xk, decode_exchange_pubkey(&invitee_hex).unwrap());
        params.document_id = logged.document_id;
        params.doc_key = logged.doc_key.clone();
        params.author = same(&owner.author);
        let http = FakeHttp::new(vec![
            Ok(user_search_json("invitee@test.local", &invitee_hex)),
            Ok(logged.answer()),
            Ok(serde_json::json!({
                "user_id": "55555555-5555-5555-5555-555555555555",
                "display_name": "Invitee",
            })),
        ]);
        let (etx, erx, sink, _lrx) = channels();
        invite_task(&http, params, &etx, &sink).await;
        assert!(matches!(erx.try_recv(), Ok(SyncEvent::InviteSent { .. })));

        let calls = http.calls();
        let Call::Post(_, body) = &calls[2] else {
            panic!("expected the share POST: {calls:?}");
        };
        let entry = posted_entry(body);
        let invitee = UserId::parse_str("55555555-5555-5555-5555-555555555555").unwrap();
        assert_eq!(entry.act, MembershipAct::Invite);
        assert_eq!(entry.member, invitee);
        assert_eq!(entry.key, Some(logged.doc_key.id()));
        let text = body["entry"].as_str().unwrap();
        assert_eq!(
            logged.walk(&[text]).standing(invitee),
            (MemberStanding::Verified, Some(Role::Editor))
        );
        let owner_pub = PublicKey::from(&owner.xk).to_bytes();
        let enc = body["doc_key_enc"].as_str().unwrap();
        let (key, anchor) = unwrap_anchor_b64(&invitee_xk, &owner_pub, enc).unwrap();
        assert_eq!(key.id(), logged.doc_key.id());
        assert_eq!(anchor, Some(logged.anchor));
    }

    /// An invite whose place another entry took is signed again after that
    /// entry, so the log never forks (`SYNC-132`).
    #[tokio::test]
    async fn an_invite_overtaken_in_the_log_is_signed_again_after_it() {
        let owner = Peer::new(1);
        let mut logged = Logged::new(&owner);
        let first = logged.answer();
        logged.invite(&owner, &Peer::new(3), Role::Viewer);
        let (_invitee_xk, invitee_hex) = keypair(2);
        let mut params = invite_params(&owner.xk, decode_exchange_pubkey(&invitee_hex).unwrap());
        params.document_id = logged.document_id;
        params.doc_key = logged.doc_key.clone();
        params.author = same(&owner.author);
        let http = FakeHttp::new(vec![
            Ok(user_search_json("invitee@test.local", &invitee_hex)),
            Ok(first),
            Err("POST https://api.test/share: LOG_MOVED".into()),
            Ok(logged.answer()),
            Ok(serde_json::json!({
                "user_id": "55555555-5555-5555-5555-555555555555",
                "display_name": "Invitee",
            })),
        ]);
        let (etx, erx, sink, lrx) = channels();
        invite_task(&http, params, &etx, &sink).await;
        assert!(matches!(erx.try_recv(), Ok(SyncEvent::InviteSent { .. })));
        assert!(lrx.try_recv().is_err());
        let calls = http.calls();
        let positions: Vec<u64> = calls
            .iter()
            .filter_map(|call| match call {
                Call::Post(_, body) => Some(posted_entry(body).position),
                _ => None,
            })
            .collect();
        assert_eq!(positions, vec![1, 2]);
    }

    /// Holds `SYNC-132` at the revoke: the request carries the entry that
    /// removes the member and names the key the rotation installs.
    #[tokio::test]
    async fn a_revoke_posts_the_entry_that_removes_the_member() {
        let owner = Peer::new(1);
        let carol = Peer::new(3);
        let mut logged = Logged::new(&owner);
        logged.invite(&owner, &carol, Role::Editor);
        let mut params = revoke_params(&owner.xk);
        params.document_id = logged.document_id;
        params.user_id = carol.author.user_id;
        params.previous = logged.doc_key.clone();
        params.author = same(&owner.author);
        let http = FakeHttp::new(vec![
            Ok(logged.answer()),
            Ok(serde_json::json!({ "status": "revoked" })),
        ]);
        let (etx, erx, sink, _lrx) = channels();
        revoke_task(&http, params, &etx, &sink).await;
        let Ok(SyncEvent::RevocationComplete { new_doc_key, .. }) = erx.try_recv() else {
            panic!("expected RevocationComplete");
        };
        let calls = http.calls();
        let Call::Post(_, body) = &calls[1] else {
            panic!("expected the revoke POST: {calls:?}");
        };
        let entry = posted_entry(body);
        assert_eq!(entry.act, MembershipAct::Revoke);
        assert_eq!(entry.key, Some(new_doc_key.id()));
        let text = body["entry"].as_str().unwrap();
        let log = logged.walk(&[text]);
        assert_eq!(
            log.standing(carol.author.user_id).0,
            MemberStanding::Revoked
        );
    }

    /// Holds `SYNC-132` at a leave: the member signs their own removal.
    #[tokio::test]
    async fn a_member_who_leaves_signs_their_own_removal() {
        let owner = Peer::new(1);
        let bob = Peer::new(2);
        let mut logged = Logged::new(&owner);
        logged.invite(&owner, &bob, Role::Editor);
        let params = LeaveParams {
            id: RequestId::next(),
            document_id: logged.document_id,
            author: same(&bob.author),
            url_scheme: "https",
            api_base: "api.test".into(),
            exchange_key: bob.xk.clone(),
        };
        let http = FakeHttp::new(vec![
            Ok(logged.answer()),
            Ok(serde_json::json!({ "status": "left" })),
        ]);
        let (etx, erx, sink, _lrx) = channels();
        leave_task(&http, params, &etx, &sink).await;
        assert!(matches!(erx.try_recv(), Ok(SyncEvent::DocumentLeft { .. })));
        let calls = http.calls();
        let Call::Delete(_, body) = &calls[1] else {
            panic!("expected the removal: {calls:?}");
        };
        let entry = posted_entry(body);
        assert_eq!(
            (entry.member, entry.author),
            (bob.author.user_id, bob.author.user_id)
        );
        let text = body["entry"].as_str().unwrap();
        let log = logged.walk(&[text]);
        assert_eq!(log.standing(bob.author.user_id).0, MemberStanding::Revoked);
    }

    /// Holds `SYNC-132` and `SYNC-110` at a role change: the owner signs the
    /// entry, and the role the log then gives is the new one.
    #[tokio::test]
    async fn a_role_change_posts_the_entry_that_gives_the_role() {
        let owner = Peer::new(1);
        let bob = Peer::new(2);
        let mut logged = Logged::new(&owner);
        logged.invite(&owner, &bob, Role::Editor);
        let params = RoleChangeParams {
            id: RequestId::next(),
            document_id: logged.document_id,
            user_id: bob.author.user_id,
            role: Role::Viewer,
            url_scheme: "https",
            api_base: "api.test".into(),
            exchange_key: owner.xk.clone(),
            author: same(&owner.author),
        };
        let http = FakeHttp::new(vec![
            Ok(logged.answer()),
            Ok(serde_json::json!({ "status": "updated" })),
        ]);
        let (etx, erx, sink, _lrx) = channels();
        role_change_task(&http, params, &etx, &sink).await;
        assert!(matches!(
            erx.try_recv(),
            Ok(SyncEvent::MemberRoleUpdatedEvent {
                role: Role::Viewer,
                ..
            })
        ));
        let calls = http.calls();
        let Call::Patch(url, body) = &calls[1] else {
            panic!("expected the role PATCH: {calls:?}");
        };
        assert!(url.ends_with(&format!("/members/{}", bob.author.user_id)));
        assert_eq!(body["role"], "viewer");
        let text = body["entry"].as_str().unwrap();
        let log = logged.walk(&[text]);
        assert_eq!(log.standing(bob.author.user_id).1, Some(Role::Viewer));
    }

    /// Holds `SYNC-108` and `SYNC-110` at the list: a row no valid entry
    /// reaches reads unverified, and a logged row takes the log's role.
    #[test]
    fn the_member_list_shows_what_the_log_verifies() {
        let owner = Peer::new(1);
        let bob = Peer::new(2);
        let mut logged = Logged::new(&owner);
        logged.invite(&owner, &bob, Role::Viewer);
        // The relay claims a role the log never gave, and adds a stranger.
        logged.rows[1]["role"] = "owner".into();
        logged.row(&Peer::new(9), String::new(), Role::Editor);
        let resp: http::DocumentMembersResponse = serde_json::from_value(logged.answer()).unwrap();
        let (log, _) = served_log(logged.document_id, &owner.xk, &resp);
        let shown = map_members(resp.members, &log);
        let standing: Vec<_> = shown.iter().map(|m| (m.standing, m.role)).collect();
        assert_eq!(
            standing,
            vec![
                (MemberStanding::Verified, Role::Owner),
                (MemberStanding::Verified, Role::Viewer),
                (MemberStanding::Unverified, Role::Editor),
            ]
        );
    }

    /// Holds `SYNC-106` at the list. A self-join verifies only in a log the
    /// caller walks under the public-channel rule: the meta of a channel a
    /// catalogue lists.
    #[tokio::test]
    async fn the_member_list_admits_a_self_join_only_in_a_marked_log() {
        let owner = Peer::new(1);
        let joiner = Peer::new(2);
        let mut logged = Logged::new(&owner);
        let draft = Draft {
            document: logged.document_id,
            member: joiner.author.user_id,
            member_key: joiner.author.signing_key.verifying_key().to_bytes(),
            act: MembershipAct::SelfJoin,
            role: Role::Editor,
            key: Some(logged.doc_key.id()),
        };
        let entry = entry_after(&logged.walk(&[]), draft, &joiner.author)
            .unwrap()
            .unwrap();
        logged.log.push(entry);
        logged.row(&joiner, String::new(), Role::Editor);
        let rules: [(membership_log::SelfJoinRule, MemberStanding); 2] = [
            (
                membership_log::public_channel_self_join,
                MemberStanding::Verified,
            ),
            (membership_log::no_self_join, MemberStanding::Unverified),
        ];
        for (rule, standing) in rules {
            let http = FakeHttp::new(vec![Ok(logged.answer())]);
            let (etx, erx, _sink, _lrx) = channels();
            let mut params = list_params();
            params.document_id = logged.document_id;
            params.exchange_key = owner.xk.clone();
            params.rule = LogRule {
                self_join: rule,
                space: None,
            };
            list_members_task(&http, params, &etx).await;
            match erx.try_recv().expect("DocumentMembersReady") {
                SyncEvent::DocumentMembersReady { members, .. } => {
                    assert_eq!(members[1].standing, standing);
                }
                _ => panic!("unexpected event (want DocumentMembersReady)"),
            }
        }
    }
}
