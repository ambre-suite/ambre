// ambre-sync/src/sharing.rs
// ============================================================================
// Pure transforms for the document-sharing flow (invite + revoke key rotation).
//
// These hold the crypto-glue logic -- hex/base64 decoding, X25519 ECDH key
// wrapping, and server-response mapping -- with NO IO. The surrounding handlers
// in `lib.rs` only do `fetch` (wasm-only) plus channel plumbing, so pulling the
// decisions out here makes them unit-testable natively (`cargo test`), without a
// browser or a live server. ECDH primitive correctness itself is covered by
// `ambre-crypto` (`wrap_unwrap_roundtrip`); here we test the GLUE: peer-key
// selection, the `nonce || ciphertext` wire format, and best-effort rotation.
// ============================================================================

use ambre_crypto::DocumentKey;
use base64::Engine;
use serde::Serialize;
use serde::de::DeserializeOwned;
use x25519_dalek::StaticSecret;

use ambre_bus::{DocumentId, LogicEvent, MemberInfo, RequestId, Role, UserId};

use crate::http;
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
pub(crate) fn wrap_key_b64(
    my_xk: &StaticSecret,
    peer_pub: &[u8; 32],
    doc_key: &DocumentKey,
) -> Result<String, ambre_crypto::CryptoError> {
    // Fails if the peer public key is non-contributory (low-order): we refuse to
    // wrap under a predictable KEK rather than hand out a forgeable share.
    let wrapped = ambre_crypto::ecdh::wrap_document_key(my_xk, peer_pub, doc_key)?;
    let mut enc = wrapped.nonce.to_vec();
    enc.extend_from_slice(&wrapped.ciphertext);
    Ok(base64::engine::general_purpose::URL_SAFE.encode(&enc))
}

/// Builds the `POST /documents/{id}/share` body: ECDH-wraps `doc_key` for the
/// invitee whose hex exchange pubkey is `invitee_pub_hex`.
pub(crate) fn build_invite_request(
    my_xk: &StaticSecret,
    invitee_pub_hex: &str,
    invitee_email: String,
    doc_key: &DocumentKey,
    role: Role,
) -> Result<http::InviteRequest, String> {
    let peer = decode_exchange_pubkey(invitee_pub_hex)?;
    Ok(http::InviteRequest {
        invitee_email,
        role: role.to_string(),
        doc_key_enc: wrap_key_b64(my_xk, &peer, doc_key).map_err(|e| e.to_string())?,
    })
}

/// Re-wraps `new_doc_key` for every remaining member after a revoke (the rotate
/// payload). Members with a malformed exchange pubkey are skipped rather than
/// failing the whole rotation -- a best-effort that keeps the rest converging.
pub(crate) fn rotate_entries_for(
    my_xk: &StaticSecret,
    new_doc_key: &DocumentKey,
    members: &[http::DocumentMemberEntry],
) -> Vec<http::RotateKeyEntry> {
    members
        .iter()
        .filter_map(|m| {
            let peer = decode_exchange_pubkey(&m.exchange_pub_key).ok()?;
            Some(http::RotateKeyEntry {
                user_id: m.user_id.clone(),
                doc_key_enc: wrap_key_b64(my_xk, &peer, new_doc_key).ok()?,
            })
        })
        .collect()
}

/// Builds the presence directory (Ed25519 signing key -> canonical `UserId`)
/// from any members response, given each entry's `(user_id, signing_pub_key)`
/// as raw strings (`ambre-spec-membership.md`). Both the open
/// path and the member-list path feed it, hence the string-pair input rather
/// than one of the two entry structs.
///
/// An entry whose id or key does not parse is skipped rather than failing the
/// whole response: a member we cannot key on costs that one peer's cursor, while
/// refusing everything would cost the document its presence entirely. Unlike
/// [`map_members`], nothing is invented for a bad row -- a fabricated id here
/// would attribute a cursor to the wrong person, which is exactly what keying
/// presence on the canonical `UserId` exists to prevent.
pub(crate) fn presence_directory<'a>(
    entries: impl IntoIterator<Item = (&'a str, &'a str)>,
) -> Vec<([u8; 32], UserId)> {
    entries
        .into_iter()
        .filter_map(|(user_id, signing_pub_key)| {
            let user_id = UserId::parse_str(user_id).ok()?;
            let key: [u8; 32] = hex::decode(signing_pub_key).ok()?.try_into().ok()?;
            Some((key, user_id))
        })
        .collect()
}

/// Picks this client's own role out of a members response, given `(user_id,
/// role)` pairs and our canonical `UserId` (architecture).
///
/// Every failure path -- no session id, no row for us, an unparsable id, an
/// unknown role string -- yields `Role::Viewer`. That is the safe direction for
/// a UX gate: it hides affordances we may in fact have, whereas guessing
/// `Owner` would offer edits the server then rejects. A document we genuinely
/// own always has a well-formed row saying so.
pub(crate) fn own_role<'a>(
    entries: impl IntoIterator<Item = (&'a str, &'a str)>,
    my_user_id: Option<UserId>,
) -> Role {
    let Some(me) = my_user_id else {
        return Role::Viewer;
    };
    entries
        .into_iter()
        .find(|(user_id, _)| UserId::parse_str(user_id).is_ok_and(|id| id == me))
        .and_then(|(_, role)| role.parse().ok())
        .filter(|role: &Role| role.is_valid_for_document())
        .unwrap_or(Role::Viewer)
}

/// Maps server member entries into bus `MemberInfo` (plaintext name/email,
/// parsed role). Unknown roles fall back to viewer, unparsable ids to a fresh
/// id -- the list is display-only, so it degrades instead of dropping rows.
pub(crate) fn map_members(entries: Vec<http::DocumentMemberEntry>) -> Vec<MemberInfo> {
    entries
        .into_iter()
        .map(|m| MemberInfo {
            user_id: UserId::parse_str(&m.user_id).unwrap_or_else(|_| UserId::new()),
            display_name: m.display_name,
            email: m.email,
            role: m.role.parse().unwrap_or(Role::Viewer),
        })
        .collect()
}

// ============================================================================
// Transport boundary + orchestration tasks.
//
// The three sharing handlers in `lib.rs` used to inline their `fetch` calls and
// their channel plumbing inside a `spawn_local`, which made the orchestration
// (which call fires when, which error maps to which event) reachable only from a
// real browser. Abstracting the HTTP surface behind `ShareHttp` lets the tasks
// below run on `tokio` in native tests against a `FakeHttp`, so every error
// branch is asserted without a browser or a live server. Production wires the
// real `GlooHttp` (fetch + Bearer/refresh via `AuthContext`); the handlers keep
// only their synchronous preconditions and the `spawn_local`.
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
    async fn delete(&self, url: &str) -> Result<(), String>;
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
    async fn delete(&self, url: &str) -> Result<(), String> {
        http::delete_auth(url, self.auth.clone()).await
    }
}

/// Sends a `LogicEvent::ApiError` (the error channel for the sharing tasks).
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
}

/// Invite orchestration: search the invitee by email, ECDH-wrap the document key
/// for them, POST the share. Success -> `SyncEvent::InviteSent`; every failure ->
/// `LogicEvent::ApiError` with the leg-specific code.
///
/// The re-fetched exchange key is compared against `expect_exchange_pub_key`
/// (the value pinned in phase 1) **before** wrapping: this closes the TOCTOU
/// hole where a compromised server serves an honest key to the fingerprint
/// check and a substituted one to the wrap. A mismatch fails the invite; nothing
/// is ever wrapped to an unconfirmed key.
pub(crate) async fn invite_task<H: ShareHttp>(
    http: &H,
    p: InviteParams,
    event_tx: &flume::Sender<SyncEvent>,
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
    let body = match build_invite_request(
        &p.exchange_key,
        &invitee.exchange_pubkey,
        p.invitee_email.clone(),
        &p.doc_key,
        p.role,
    ) {
        Ok(req) => req,
        Err(message) => return api_error(logic_tx, "InviteToDocument", 500, message),
    };
    let invite_url = format!(
        "{}://{}/api/documents/{}/share",
        p.url_scheme, p.api_base, p.document_id
    );
    match http
        .post_json::<_, http::InviteResponse>(&invite_url, &body)
        .await
    {
        Ok(resp) => {
            let user_id = UserId::parse_str(&resp.user_id).unwrap_or_else(|_| UserId::new());
            let _ = event_tx.send(SyncEvent::InviteSent {
                id: p.id,
                document_id: p.document_id,
                user_id,
                display_name: resp.display_name,
            });
        }
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
    event_tx: &flume::Sender<SyncEvent>,
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
    let user_id = UserId::parse_str(&invitee.id).unwrap_or_else(|_| UserId::new());
    let _ = event_tx.send(SyncEvent::InviteeKeyReady {
        id: p.id,
        document_id: p.document_id,
        user_id,
        display_name: invitee.display_name.clone(),
        email: invitee.email.clone(),
        exchange_pub_key,
        fingerprint,
    });
}

/// Inputs for [`revoke_task`], gathered from `SyncState` before spawning.
pub(crate) struct RevokeParams {
    pub id: RequestId,
    pub document_id: DocumentId,
    pub user_id: UserId,
    pub url_scheme: &'static str,
    pub api_base: String,
    pub exchange_key: StaticSecret,
}

/// Revoke orchestration: DELETE the share, generate a fresh `DocumentKey`, fetch
/// the remaining members, re-wrap the new key for each, POST the rotation.
/// Success -> `SyncEvent::RevocationComplete` (the caller swaps the local key);
/// every failure -> `LogicEvent::ApiError`.
pub(crate) async fn revoke_task<H: ShareHttp>(
    http: &H,
    p: RevokeParams,
    event_tx: &flume::Sender<SyncEvent>,
    logic_tx: &LogicSink,
) {
    let revoke_url = format!(
        "{}://{}/api/documents/{}/share/{}",
        p.url_scheme, p.api_base, p.document_id, p.user_id
    );
    if let Err(e) = http.delete(&revoke_url).await {
        return api_error(
            logic_tx,
            "RevokeFromDocument",
            500,
            format!("Revoke failed: {e}"),
        );
    }

    let new_doc_key = DocumentKey::generate();

    let members_url = format!(
        "{}://{}/api/documents/{}/members",
        p.url_scheme, p.api_base, p.document_id
    );
    let members = match http
        .get_json::<http::DocumentMembersResponse>(&members_url)
        .await
    {
        Ok(resp) => resp.members,
        Err(e) => {
            return api_error(
                logic_tx,
                "RevokeFromDocument",
                500,
                format!("Get members failed: {e}"),
            );
        }
    };

    let keys = rotate_entries_for(&p.exchange_key, &new_doc_key, &members);
    let rotate_url = format!(
        "{}://{}/api/documents/{}/rotate-key",
        p.url_scheme, p.api_base, p.document_id
    );
    let body = http::RotateKeyRequest { keys };
    if let Err(e) = http
        .post_json::<_, serde_json::Value>(&rotate_url, &body)
        .await
    {
        return api_error(
            logic_tx,
            "RevokeFromDocument",
            500,
            format!("Rotate key failed: {e}"),
        );
    }

    let _ = event_tx.send(SyncEvent::RevocationComplete {
        id: p.id,
        document_id: p.document_id,
        new_doc_key,
    });
}

/// Inputs for [`list_members_task`].
pub(crate) struct ListMembersParams {
    pub id: RequestId,
    pub document_id: DocumentId,
    pub url_scheme: &'static str,
    pub api_base: String,
}

/// Member-list orchestration: GET the members, map them, emit
/// `SyncEvent::DocumentMembersReady`; on failure emit `SyncEvent::HttpError`.
pub(crate) async fn list_members_task<H: ShareHttp>(
    http: &H,
    p: ListMembersParams,
    event_tx: &flume::Sender<SyncEvent>,
) {
    let url = format!(
        "{}://{}/api/documents/{}/members",
        p.url_scheme, p.api_base, p.document_id
    );
    match http.get_json::<http::DocumentMembersResponse>(&url).await {
        Ok(resp) => {
            // Refresh the presence directory off the same response
            // (`ambre-spec-membership.md`): membership just changed often enough to ask
            // for the list, so this is where a new member becomes resolvable and
            // a revoked one stops resolving -- no extra round-trip.
            let signing_keys = presence_directory(
                resp.members
                    .iter()
                    .map(|m| (m.user_id.as_str(), m.signing_pub_key.as_str())),
            );
            let members = map_members(resp.members);
            let _ = event_tx.send(SyncEvent::DocumentMembersReady {
                id: p.id,
                document_id: p.document_id,
                members,
                signing_keys,
            });
        }
        Err(e) => {
            let _ = event_tx.send(SyncEvent::HttpError {
                id: p.id,
                message: e,
            });
        }
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
            user_id: uid.to_string(),
            display_name: "Member".into(),
            email: "m@test.local".into(),
            signing_pub_key: String::new(),
            exchange_pub_key: pub_hex.to_string(),
            doc_key_enc: String::new(),
            role: "editor".into(),
        }
    }

    /// Decode helper: the inverse of `wrap_key_b64`, used by recipients.
    fn unwrap_b64(
        recipient: &StaticSecret,
        wrapper_pub: &[u8; 32],
        doc_key_enc: &str,
    ) -> Result<DocumentKey, String> {
        let raw = base64::engine::general_purpose::URL_SAFE
            .decode(doc_key_enc)
            .map_err(|e| e.to_string())?;
        let nonce_len = raw.len() - 32 - 16; // XChaCha nonce; AEAD tag is 16
        let wrapped = ambre_crypto::WrappedKey {
            nonce: raw[..nonce_len].try_into().map_err(|_| "bad nonce")?,
            ciphertext: raw[nonce_len..].to_vec(),
        };
        ambre_crypto::ecdh::unwrap_document_key(recipient, wrapper_pub, &wrapped)
            .map_err(|e| format!("{e:?}"))
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
        )
        .expect("valid pubkey");

        assert_eq!(req.invitee_email, "invitee@test.local");
        assert_eq!(req.role, "editor");
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
        );
        assert!(err.is_err());
    }

    #[test]
    fn rotate_entries_cover_valid_members_and_skip_malformed() {
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
        let entries = rotate_entries_for(&my_xk, &new_key, &members);

        // The malformed member is dropped; the two valid ones are wrapped.
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].user_id, "11111111-1111-1111-1111-111111111111");
        assert_eq!(entries[1].user_id, "33333333-3333-3333-3333-333333333333");
        // Member A can unwrap its own rotated entry.
        assert!(unwrap_b64(&a_xk, &my_pub, &entries[0].doc_key_enc).is_ok());
    }

    #[test]
    fn map_members_parses_roles_and_falls_back() {
        let entries = vec![member("44444444-4444-4444-4444-444444444444", "ab"), {
            let mut m = member("not-a-uuid", "ab");
            m.role = "bogus".into();
            m
        }];
        let mapped = map_members(entries);
        assert_eq!(mapped.len(), 2);
        assert_eq!(mapped[0].role, Role::Editor);
        // Unknown role -> viewer; unparsable id -> a fresh (non-nil) id.
        assert_eq!(mapped[1].role, Role::Viewer);
    }

    // -- Orchestration tasks (Tier 2): driven natively against a fake transport.

    use std::cell::RefCell;
    use std::collections::VecDeque;

    /// One recorded HTTP call (method + url + serialized body for POSTs).
    #[derive(Debug)]
    enum Call {
        Get(String),
        Post(String, serde_json::Value),
        Delete(String),
    }

    /// A scripted [`ShareHttp`]: returns the queued responses in order and
    /// records every call so a test can assert what the task did (and did NOT)
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
        async fn delete(&self, url: &str) -> Result<(), String> {
            self.calls.borrow_mut().push(Call::Delete(url.to_string()));
            self.next().map(|_| ())
        }
    }

    /// Fresh event + logic channels with their receivers.
    fn channels() -> (
        flume::Sender<SyncEvent>,
        flume::Receiver<SyncEvent>,
        LogicSink,
        flume::Receiver<LogicEvent>,
    ) {
        let (etx, erx) = flume::unbounded();
        let (sink, lrx) = LogicSink::channel();
        (etx, erx, sink, lrx)
    }

    fn user_search_json(email: &str, exchange_pub_hex: &str) -> serde_json::Value {
        serde_json::json!({ "users": [ {
            "id": "55555555-5555-5555-5555-555555555555",
            "email": email,
            "display_name": "Invitee",
            "signing_pubkey": "00",
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

        // Two calls: search then share; the share body wraps a key only the
        // invitee can unwrap.
        let calls = http.calls();
        assert_eq!(calls.len(), 2);
        match &calls[1] {
            Call::Post(url, body) => {
                assert!(url.ends_with("/share"));
                let enc = body["doc_key_enc"].as_str().unwrap();
                assert!(unwrap_b64(&invitee_xk, &my_pub, enc).is_ok());
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
        }
    }

    #[tokio::test]
    async fn revoke_success_deletes_then_rotates_remaining_members() {
        let (my_xk, _) = keypair(1);
        let (_a_xk, a_pub) = keypair(2);

        let http = FakeHttp::new(vec![
            Ok(serde_json::json!({ "status": "revoked" })), // DELETE
            Ok(serde_json::json!({ "members": [ {
                "user_id": "11111111-1111-1111-1111-111111111111",
                "display_name": "A", "email": "a@test.local",
                "signing_pub_key": "00", "exchange_pub_key": a_pub,
                "doc_key_enc": "", "role": "editor",
            } ] })), // GET members
            Ok(serde_json::json!({ "status": "rotated" })), // POST rotate
        ]);
        let (etx, erx, sink, lrx) = channels();
        revoke_task(&http, revoke_params(&my_xk), &etx, &sink).await;

        assert!(matches!(
            erx.try_recv().expect("RevocationComplete"),
            SyncEvent::RevocationComplete { .. }
        ));
        assert!(lrx.try_recv().is_err());

        let calls = http.calls();
        assert_eq!(calls.len(), 3);
        assert!(matches!(&calls[0], Call::Delete(u) if u.contains("/share/")));
        assert!(matches!(&calls[1], Call::Get(u) if u.ends_with("/members")));
        match &calls[2] {
            Call::Post(url, body) => {
                assert!(url.ends_with("/rotate-key"));
                assert_eq!(body["keys"].as_array().unwrap().len(), 1);
            }
            other => panic!("expected rotate POST, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn revoke_delete_failure_emits_500_and_stops() {
        let (my_xk, _) = keypair(1);
        let http = FakeHttp::new(vec![Err("403".into())]);
        let (etx, erx, sink, lrx) = channels();
        revoke_task(&http, revoke_params(&my_xk), &etx, &sink).await;

        match lrx.try_recv().expect("ApiError emitted") {
            LogicEvent::ApiError { code, message, .. } => {
                assert_eq!(code, 500);
                assert!(message.contains("Revoke failed"));
            }
            other => panic!("unexpected: {other:?}"),
        }
        assert!(erx.try_recv().is_err());
        // Stopped after the failed DELETE: no members fetch, no rotation.
        assert_eq!(http.calls().len(), 1);
    }

    fn list_params() -> ListMembersParams {
        ListMembersParams {
            id: RequestId::next(),
            document_id: DocumentId::new(),
            url_scheme: "https",
            api_base: "api.test".into(),
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
        let uid = "11111111-1111-1111-1111-111111111111";
        let key_hex = "aa".repeat(32);
        let dir = presence_directory([(uid, key_hex.as_str())]);

        assert_eq!(dir, vec![([0xaa; 32], UserId::parse_str(uid).unwrap())]);
    }

    #[test]
    fn presence_directory_skips_unparsable_rows_without_inventing_ids() {
        // Unlike `map_members` (display-only, degrades with a fresh id), a bad row
        // here must vanish: a fabricated id would attribute a cursor to the wrong
        // person, which is what keying presence on the canonical UserId prevents.
        let good_uid = "11111111-1111-1111-1111-111111111111";
        let good_key = "bb".repeat(32);
        let short_key = "bb".repeat(31); // 31 bytes: not a valid Ed25519 key
        let dir = presence_directory([
            ("not-a-uuid", good_key.as_str()),
            (good_uid, "zz-not-hex"),
            (good_uid, short_key.as_str()),
            (good_uid, good_key.as_str()),
        ]);

        assert_eq!(
            dir,
            vec![([0xbb; 32], UserId::parse_str(good_uid).unwrap())]
        );
    }

    // -- Own role (architecture) -----------------------------------------

    #[test]
    fn own_role_reads_our_own_row() {
        let me = "11111111-1111-1111-1111-111111111111";
        let other = "22222222-2222-2222-2222-222222222222";
        let role = own_role(
            [(other, "owner"), (me, "viewer")],
            Some(UserId::parse_str(me).unwrap()),
        );

        // Our own row wins: a peer being owner says nothing about our access.
        assert_eq!(role, Role::Viewer);
    }

    #[test]
    fn own_role_falls_back_to_viewer_when_unresolvable() {
        let me = UserId::parse_str("11111111-1111-1111-1111-111111111111").unwrap();
        let me_str = me.to_string();
        let other = "22222222-2222-2222-2222-222222222222";

        // No session id yet.
        assert_eq!(own_role([(other, "owner")], None), Role::Viewer);
        // No row for us in the response.
        assert_eq!(own_role([(other, "owner")], Some(me)), Role::Viewer);
        // Our row carries a role string this client does not know.
        assert_eq!(
            own_role([(me_str.as_str(), "wizard")], Some(me)),
            Role::Viewer
        );
        // A valid role that is not a document role (channel / org side).
        assert_eq!(
            own_role([(me_str.as_str(), "admin")], Some(me)),
            Role::Viewer
        );
    }

    #[test]
    fn own_role_reads_writable_roles() {
        let me = UserId::parse_str("11111111-1111-1111-1111-111111111111").unwrap();
        let me_str = me.to_string();
        for (wire, expected) in [("owner", Role::Owner), ("editor", Role::Editor)] {
            assert_eq!(own_role([(me_str.as_str(), wire)], Some(me)), expected);
            assert!(expected.can_write_document());
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
}
