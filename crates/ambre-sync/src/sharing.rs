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
use ed25519_dalek::SigningKey;
use serde::Serialize;
use serde::de::DeserializeOwned;
use x25519_dalek::StaticSecret;

use ambre_bus::{
    ApiFailure, ApiRequest, ChannelDocumentKind, LogicEvent, MemberInfo, Request, Role,
};
use ambre_core::id::{Channel, Document, Id, Message, Segment, Space, User};
use ambre_core::{LocalFailure, Seq};

use crate::http;
use crate::membership_log::{self, Draft, Log, LogHead, LogRule};
use crate::{LogicSink, SyncEvent};
use ambre_core::{FixedBytes, RefusalCode};

use crate::error::{Part, SyncError};

/// Wraps `doc_key` for the holder of `peer_pub` via X25519 ECDH and returns the
/// base64 url-safe `nonce || ciphertext` blob the server stores as `doc_key_enc`.
/// Only that peer (using its own secret against our public key) can unwrap it.
/// The anchor of the membership log is sealed beside the key (`SYNC-104`), so
/// the recipient learns where the log starts from the member who wrapped.
pub(crate) fn wrap_anchored_b64(
    my_xk: &StaticSecret,
    peer_pub: &FixedBytes<ambre_core::ExchangePublicKey>,
    doc_key: &DocumentKey,
    anchor: Option<FixedBytes<EntryDigest>>,
) -> Result<ambre_core::Encoded<ambre_core::WrappedDocKey>, ambre_crypto::CryptoError> {
    // Fails if the peer public key is non-contributory (low-order): we refuse to
    // wrap under a predictable KEK rather than hand out a forgeable share.
    let wrapped = ambre_crypto::ecdh::wrap_anchored(my_xk, peer_pub, doc_key, anchor.as_ref())?;
    Ok(
        ambre_core::Encoded::<ambre_core::WrappedDocKey>::from_parts(
            &wrapped.nonce,
            &wrapped.ciphertext,
        ),
    )
}

/// Unwraps our own copy of the key among `rows`, given as `(exchange key,
/// wrapped key, wrapper's exchange key)`, and the log anchor sealed with it.
///
/// Our row is the one carrying our exchange key. The exchange runs against
/// whoever wrapped, never against our own key (`SYNC-034`). A row with no
/// wrapper recorded falls back to the member's own key, for an older relay.
pub(crate) fn unwrap_own<'a>(
    exchange_key: &StaticSecret,
    rows: impl IntoIterator<
        Item = (
            &'a FixedBytes<ambre_core::ExchangePublicKey>,
            &'a ambre_core::Encoded<ambre_core::WrappedDocKey>,
            Option<&'a FixedBytes<ambre_core::ExchangePublicKey>>,
        ),
    >,
) -> Result<(DocumentKey, Option<FixedBytes<EntryDigest>>), SyncError> {
    let ours = own_exchange_key(exchange_key);
    let (own, enc, wrapper) = rows
        .into_iter()
        .find(|(own, _, _)| **own == ours)
        .ok_or(SyncError::OwnRowMissing)?;
    let (nonce, ciphertext) = enc.parts().ok_or(SyncError::Malformed(Part::WrappedKey))?;
    let peer = wrapper.unwrap_or(own);
    let wrapped = ambre_crypto::WrappedKey { nonce, ciphertext };
    Ok(ambre_crypto::ecdh::unwrap_anchored(
        exchange_key,
        peer,
        &wrapped,
    )?)
}

/// The X25519 public key of `secret`: this user's own exchange key.
pub(crate) fn own_exchange_key(secret: &StaticSecret) -> FixedBytes<ambre_core::ExchangePublicKey> {
    FixedBytes::from_array(x25519_dalek::PublicKey::from(secret).to_bytes())
}

/// The user who signs the entries a change appends (`SYNC-132`).
pub(crate) struct Author {
    pub user_id: Id<User>,
    pub signing_key: SigningKey,
}

impl Author {
    /// The public half of the author's signing key.
    pub(crate) fn public(&self) -> FixedBytes<ambre_core::PublicKey> {
        FixedBytes::from_array(self.signing_key.verifying_key().to_bytes())
    }
}

/// The log a members answer serves, walked from the anchor sealed with our
/// own copy of the key (`SYNC-104`). The anchor is `None` when our row does
/// not unwrap or carries none.
pub(crate) fn served_log(
    document_id: Id<Document>,
    exchange_key: &StaticSecret,
    resp: &http::DocumentMembersResponse,
) -> (Log, Option<FixedBytes<EntryDigest>>) {
    served_log_under(document_id, exchange_key, resp, &LogRule::document())
}

/// [`served_log`], under the rule of the document's log (`SYNC-106`).
pub(crate) fn served_log_under(
    document_id: Id<Document>,
    exchange_key: &StaticSecret,
    resp: &http::DocumentMembersResponse,
    rule: &LogRule,
) -> (Log, Option<FixedBytes<EntryDigest>>) {
    // A row whose exchange key does not parse is no row of ours to unwrap.
    let rows = resp.members.iter().filter_map(|m| {
        Some((
            m.exchange_pub_key.key()?,
            &m.doc_key_enc,
            m.wrapper_exchange_pub_key.key(),
        ))
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
    document_id: Id<Document>,
    anchor: Option<FixedBytes<EntryDigest>>,
    texts: &[ambre_core::Encoded<ambre_core::EncodedLogEntry>],
    rule: &LogRule,
) -> Log {
    let entries = membership_log::decode(texts);
    Log::walk_under(document_id, anchor, &entries, rule)
}

/// The entry `draft` makes after `log`, signed by `author`, in request form.
/// `None` for a document whose log is empty: it predates the log, and no
/// entry of it could link to a first one.
fn entry_after(
    log: &Log,
    draft: Draft,
    author: &Author,
) -> Result<Option<ambre_core::Encoded<ambre_core::EncodedLogEntry>>, SyncError> {
    let next = log.next();
    if next.is_empty() {
        return Ok(None);
    }
    let entry = membership_log::sign(draft, next, author.user_id, &author.signing_key)?;
    Ok(Some(membership_log::encode(&entry)?))
}

/// The creator's copy of a new document's key, and the first entry of its
/// log in request form (`SYNC-132`). The creator signs the entry, and the
/// copy seals its digest as the anchor (`SYNC-104`).
pub(crate) fn creation(
    document_id: Id<Document>,
    doc_key: &DocumentKey,
    exchange_key: &StaticSecret,
    author: &Author,
) -> Result<
    (
        ambre_core::Encoded<ambre_core::WrappedDocKey>,
        ambre_core::Encoded<ambre_core::EncodedLogEntry>,
    ),
    SyncError,
> {
    let draft = Draft {
        document: document_id,
        member: author.user_id,
        member_key: FixedBytes::from_array(author.signing_key.verifying_key().to_bytes()),
        member_exchange_key: own_exchange_key(exchange_key),
        act: MembershipAct::Create,
        role: Role::Owner,
        key: Some(doc_key.id()),
    };
    let entry = membership_log::sign(draft, LogHead::EMPTY, author.user_id, &author.signing_key)?;
    let anchor = membership_log::digest(&entry).ok_or(SyncError::NoDigest)?;
    let own_pub = own_exchange_key(exchange_key);
    let doc_key_enc = wrap_anchored_b64(exchange_key, &own_pub, doc_key, Some(anchor))?;
    Ok((doc_key_enc, membership_log::encode(&entry)?))
}

/// Builds the `POST /documents/{id}/share` body: ECDH-wraps `doc_key` for the
/// invitee whose exchange pubkey is `invitee_pub`. The wrap seals the log
/// `anchor` beside the key (`SYNC-104`), and the body carries the signed
/// `entry`.
pub(crate) fn build_invite_request(
    my_xk: &StaticSecret,
    invitee_pub: &FixedBytes<ambre_core::ExchangePublicKey>,
    invitee: InviteeName,
    doc_key: &DocumentKey,
    role: Role,
    anchor: Option<FixedBytes<EntryDigest>>,
    entry: Option<ambre_core::Encoded<ambre_core::EncodedLogEntry>>,
) -> Result<http::InviteRequest, SyncError> {
    let (invitee_email, invitee_id) = match invitee {
        InviteeName::Email(email) => (Some(email), None),
        InviteeName::Id(id) => (None, Some(id)),
    };
    Ok(http::InviteRequest {
        invitee_email,
        invitee_id,
        role,
        doc_key_enc: wrap_anchored_b64(my_xk, invitee_pub, doc_key, anchor)?,
        // The relay refuses the copy if a rotation replaced this key
        // between the read and the post (`SYNC-143`).
        key_id: doc_key.id(),
        entry,
    })
}

/// Re-wraps `new_doc_key` for every remaining member after a revoke (the rotate
/// payload), with the log `anchor` sealed beside it (`SYNC-104`).
///
/// A copy goes to the exchange key a row serves only when a valid entry of
/// `log` signs that key for that member (`SYNC-198`).
/// So a row the relay adds, or a key it swaps, receives no copy. This
/// client's own row is the one exception: its key is the local secret's.
/// A member whose served key is malformed, unsigned, or refused by the wrap
/// cannot be re-wrapped for. The rotation goes on for the others, and that member is
/// returned in the second list so the caller reports it (`SYNC-043`).
/// Nothing is skipped in silence, and nothing is encrypted to a key that did
/// not parse.
pub(crate) fn rotate_entries_for(
    my_xk: &StaticSecret,
    new_doc_key: &DocumentKey,
    anchor: Option<FixedBytes<EntryDigest>>,
    log: &Log,
    members: &[http::DocumentMemberEntry],
) -> (
    Vec<http::RotateKeyEntry>,
    Vec<ambre_core::Validated<ambre_core::DisplayName>>,
) {
    let own = own_exchange_key(my_xk);
    let mut unwrapped = Vec::new();
    let entries = members
        .iter()
        .filter_map(|m| {
            // The key served on the row, kept only when the log signs it for
            // that member or it is this client's own.
            let recipient = m
                .exchange_pub_key
                .key()
                .copied()
                .filter(|key| *key == own || log.exchange_key(m.user_id) == Some(*key));
            let wrapped = recipient.ok_or(()).and_then(|key| {
                wrap_anchored_b64(my_xk, &key, new_doc_key, anchor).map_err(|_| ())
            });
            match wrapped {
                Ok(doc_key_enc) => Some(http::RotateKeyEntry {
                    user_id: m.user_id,
                    doc_key_enc,
                }),
                Err(()) => {
                    unwrapped.push(m.display_name.clone());
                    None
                }
            }
        })
        .collect();
    (entries, unwrapped)
}

/// Builds the presence directory (Ed25519 signing key -> canonical `Id<User>`)
/// from any members response, given each entry's `(user_id, signing_pub_key)`
/// (`ambre-spec-membership.md`). Both the open path and the member-list path
/// feed it, hence the pair input rather than one of the two entry structs.
/// The identifier parsed when the response was read. A row whose signing key
/// did not parse is skipped, never given a guessed identity (`SYNC-015`). The
/// count of skipped rows is logged, and none of the keys.
pub(crate) fn presence_directory<'a>(
    entries: impl IntoIterator<Item = (Id<User>, Option<&'a FixedBytes<ambre_core::PublicKey>>)>,
) -> Vec<(FixedBytes<ambre_core::PublicKey>, Id<User>)> {
    let mut skipped = 0usize;
    let directory = entries
        .into_iter()
        .filter_map(|(user_id, signing_pub_key)| {
            let key = signing_pub_key.copied();
            if key.is_none() {
                skipped = skipped.saturating_add(1);
            }
            key.map(|key| (key, user_id))
        })
        .collect();
    if skipped > 0 {
        tracing::warn!(
            "[Sync] presence directory skipped {skipped} rows whose signing key did not parse"
        );
    }
    directory
}

/// [`presence_directory`], keeping only the rows whose key `log` binds to
/// their member (`SYNC-215`). The relay cannot credit one member's delta or
/// cursor to another by serving that member's row with a foreign key.
pub(crate) fn presence_directory_under<'a>(
    log: &Log,
    entries: impl IntoIterator<Item = (Id<User>, Option<&'a FixedBytes<ambre_core::PublicKey>>)>,
) -> Vec<(FixedBytes<ambre_core::PublicKey>, Id<User>)> {
    let directory = presence_directory(entries);
    let held = directory.len();
    let bound: Vec<_> = directory
        .into_iter()
        .filter(|(key, member)| log.binds(key, *member))
        .collect();
    if bound.len() < held {
        tracing::warn!(
            "[Sync] presence directory dropped {} rows whose key the log binds to no such member",
            held.saturating_sub(bound.len())
        );
    }
    bound
}

/// Picks this client's own role out of a members response, given `(user_id,
/// role)` pairs and our canonical `Id<User>` (architecture).
///
/// Every failure path -- no session id, no row for us, a role that is not a
/// document one -- yields `Role::Viewer`. That is the safe direction for a UX
/// gate. It hides affordances we may in fact have, whereas guessing `Owner`
/// would offer edits the server then rejects. An unknown role word never gets
/// here: it refused the answer when it was read (`SRV-180`).
pub(crate) fn own_role(
    entries: impl IntoIterator<Item = (Id<User>, Role)>,
    my_user_id: Option<Id<User>>,
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
pub(crate) trait ShareHttp {
    async fn get_json<T: DeserializeOwned>(
        &self,
        url: &crate::http::Url<http::Route>,
    ) -> Result<T, http::HttpError>;
    async fn post_json<B: Serialize, R: DeserializeOwned>(
        &self,
        url: &crate::http::Url<http::Route>,
        body: &B,
    ) -> Result<R, http::HttpError>;
    async fn delete<B: Serialize>(
        &self,
        url: &crate::http::Url<http::Route>,
        body: &B,
    ) -> Result<(), http::HttpError>;
    async fn patch<B: Serialize>(
        &self,
        url: &crate::http::Url<http::Route>,
        body: &B,
    ) -> Result<(), http::HttpError>;
}

/// Production transport: authenticated `gloo_net` fetch (wasm at runtime, but it
/// compiles natively too). Thin pass-through to the `http` helpers.
pub(crate) struct GlooHttp {
    pub auth: http::AuthContext,
}

impl ShareHttp for GlooHttp {
    async fn get_json<T: DeserializeOwned>(
        &self,
        url: &crate::http::Url<http::Route>,
    ) -> Result<T, http::HttpError> {
        http::get_json_auth::<T>(url, self.auth.clone()).await
    }
    async fn post_json<B: Serialize, R: DeserializeOwned>(
        &self,
        url: &crate::http::Url<http::Route>,
        body: &B,
    ) -> Result<R, http::HttpError> {
        http::post_json_auth::<B, R>(url, body, self.auth.clone()).await
    }
    async fn delete<B: Serialize>(
        &self,
        url: &crate::http::Url<http::Route>,
        body: &B,
    ) -> Result<(), http::HttpError> {
        http::delete_json_auth(url, body, self.auth.clone()).await
    }
    async fn patch<B: Serialize>(
        &self,
        url: &crate::http::Url<http::Route>,
        body: &B,
    ) -> Result<(), http::HttpError> {
        http::patch_json_auth(url, body, self.auth.clone()).await
    }
}

/// Sends a `LogicEvent::ApiError` (the error path for the sharing tasks).
fn api_error(logic_tx: &LogicSink, request: ApiRequest, failure: ApiFailure) {
    let _ = logic_tx.send(LogicEvent::ApiError {
        id: None,
        request,
        failure,
    });
}

/// Reports `error` as the failure of `request`.
fn failed(logic_tx: &LogicSink, request: ApiRequest, error: impl http::Reported) {
    tracing::warn!("Sync: {} failed: {error}", request.as_str());
    api_error(logic_tx, request, error.failure());
}

/// Inputs for [`invite_task`], gathered from `SyncState` before spawning.
pub(crate) struct InviteParams {
    pub id: Seq<Request>,
    pub document_id: Id<Document>,
    pub url_scheme: crate::http::UrlScheme,
    pub api_base: crate::http::Url<crate::http::RelayHost>,
    /// The email phase 1 searched, or `None` for a member known by id.
    pub invitee_email: Option<ambre_core::Email>,
    /// The user phase 1 answered, whom the user confirmed.
    pub invitee: Id<User>,
    pub role: Role,
    pub exchange_key: StaticSecret,
    pub doc_key: DocumentKey,
    /// The exchange key the user pinned in TOFU phase 1
    /// (`ambre-spec-key-verification.md`). Sync re-fetched key must still equal it, or the
    /// wrap is refused.
    pub expect_exchange_pub_key: FixedBytes<ambre_core::ExchangePublicKey>,
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
    let invitee = match invitee_keys(http, &p).await {
        Ok(keys) => keys,
        Err(code) => {
            return api_error(logic_tx, ApiRequest::InviteToDocument, code);
        }
    };
    // TOFU pin: the re-fetched key must match the one confirmed in phase 1.
    if invitee.exchange_pubkey != p.expect_exchange_pub_key {
        tracing::warn!("Sync: invitee key changed since confirmation; invite refused");
        return api_error(
            logic_tx,
            ApiRequest::InviteToDocument,
            ApiFailure::Local(LocalFailure::PinMismatch(p.invitee)),
        );
    }
    // The invitation's entry names the invitee by identity and signing key.
    let invitee_id = p.invitee;
    let invitee_key = invitee.signing_pubkey;
    let members_url = crate::http::Url::<crate::http::ApiRoot>::of(p.url_scheme, &p.api_base)
        .route(format!("/documents/{}/members", p.document_id));
    let invite_url = crate::http::Url::<crate::http::ApiRoot>::of(p.url_scheme, &p.api_base)
        .route(format!("/documents/{}/share", p.document_id));
    let mut tries = ambre_core::Count::<RotationTries>::ZERO;
    let posted = loop {
        // Bounded by `ROTATION_TRIES`: the loop gives up at that count.
        tries = tries.incremented();
        // The entry takes the place after the log the relay serves now.
        let members = match http
            .get_json::<http::DocumentMembersResponse>(&members_url)
            .await
        {
            Ok(resp) => resp,
            Err(e) => {
                tracing::warn!("Sync: get members failed: {e}");
                return api_error(logic_tx, ApiRequest::InviteToDocument, e.failure());
            }
        };
        let (log, anchor) = served_log(p.document_id, &p.exchange_key, &members);
        let draft = Draft {
            document: p.document_id,
            member: invitee_id,
            member_key: invitee_key,
            // The key the user confirmed: the invite wraps for it alone.
            member_exchange_key: invitee.exchange_pubkey,
            act: MembershipAct::Invite,
            role: p.role,
            key: Some(p.doc_key.id()),
        };
        let body = match entry_after(&log, draft, &p.author).and_then(|entry| {
            build_invite_request(
                &p.exchange_key,
                &invitee.exchange_pubkey,
                invitee_named(&p),
                &p.doc_key,
                p.role,
                anchor,
                entry,
            )
        }) {
            Ok(req) => req,
            Err(message) => return failed(logic_tx, ApiRequest::InviteToDocument, message),
        };
        match http
            .post_json::<_, http::InviteResponse>(&invite_url, &body)
            .await
        {
            // Another entry took the place first: read the log again.
            Err(e) if e.is(RefusalCode::LogMoved) && tries < ROTATION_TRIES => {}
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
                // The invitee's link pins the key that signed the entry
                // (`SYNC-208`).
                inviter: ambre_crypto::inviter_pin(&p.author.public()),
            });
        }
        // A rotation landed first: the copy wraps a key the relay refuses.
        // Nothing was added, and an invitation sent again wraps the new key.
        Err(e) if e.is(RefusalCode::KeyChanged) => {
            api_error(logic_tx, ApiRequest::InviteToDocument, e.failure());
        }
        Err(e) => failed(logic_tx, ApiRequest::InviteToDocument, e),
    }
}

/// The keys the relay serves now for the invitee of `p`.
struct InviteeKeys {
    exchange_pubkey: FixedBytes<ambre_core::ExchangePublicKey>,
    signing_pubkey: FixedBytes<ambre_core::PublicKey>,
}

/// Fetch the keys of the confirmed invitee again, the way phase 1 found
/// them. An email is searched, and must still name the confirmed user
/// (`SYNC-161`). An id is asked for its keys alone (`SYNC-172`).
async fn invitee_keys<H: ShareHttp>(http: &H, p: &InviteParams) -> Result<InviteeKeys, ApiFailure> {
    let Some(email) = &p.invitee_email else {
        let url = crate::http::Url::<crate::http::ApiRoot>::of(p.url_scheme, &p.api_base)
            .route(format!("/users/{}/pubkeys", p.invitee));
        return match http.get_json::<http::PubkeysResponse>(&url).await {
            Ok(served) => Ok(InviteeKeys {
                exchange_pubkey: served.exchange_pubkey,
                signing_pubkey: served.signing_pubkey,
            }),
            Err(e) => {
                tracing::warn!("Sync: key fetch failed: {e}");
                Err(e.failure())
            }
        };
    };
    let search_url = crate::http::Url::<crate::http::ApiRoot>::of(p.url_scheme, &p.api_base).route(
        format!("/users/search?query={}", email.as_str().replace('@', "%40")),
    );
    let users = match http.get_json::<http::UserSearchResult>(&search_url).await {
        Ok(resp) => resp.users,
        Err(e) => {
            tracing::warn!("Sync: user search failed: {e}");
            return Err(e.failure());
        }
    };
    let Some(invitee) = users.into_iter().find(|u| u.email == *email) else {
        return Err(ApiFailure::Local(LocalFailure::UserNotFound));
    };
    // The email must still name the user confirmed in phase 1, whom a
    // notice of the channel may already name.
    if invitee.id != p.invitee {
        return Err(ApiFailure::Local(LocalFailure::InviteeChanged));
    }
    Ok(InviteeKeys {
        exchange_pubkey: invitee.exchange_pubkey,
        signing_pubkey: invitee.signing_pubkey,
    })
}

/// How the share names its invitee: by the email phase 1 searched, else by
/// the id alone (`SRV-224`).
fn invitee_named(p: &InviteParams) -> InviteeName {
    match &p.invitee_email {
        Some(email) => InviteeName::Email(email.clone()),
        None => InviteeName::Id(p.invitee),
    }
}

/// The one name a share carries for its invitee.
pub(crate) enum InviteeName {
    Email(ambre_core::Email),
    Id(Id<User>),
}

/// Inputs for [`fetch_invitee_key_task`] (TOFU phase 1).
pub(crate) struct FetchInviteeKeyParams {
    pub id: Seq<Request>,
    pub document_id: Id<Document>,
    pub url_scheme: crate::http::UrlScheme,
    pub api_base: crate::http::Url<crate::http::RelayHost>,
    pub invitee_email: ambre_core::Email,
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
    let search_url =
        crate::http::Url::<crate::http::ApiRoot>::of(p.url_scheme, &p.api_base).route(format!(
            "/users/search?query={}",
            p.invitee_email.as_str().replace('@', "%40")
        ));
    let users = match http.get_json::<http::UserSearchResult>(&search_url).await {
        Ok(resp) => resp.users,
        Err(e) => {
            tracing::warn!("Sync: user search failed: {e}");
            return api_error(logic_tx, ApiRequest::FetchInviteeKey, e.failure());
        }
    };
    let Some(invitee) = users.iter().find(|u| u.email == p.invitee_email) else {
        return api_error(
            logic_tx,
            ApiRequest::FetchInviteeKey,
            ApiFailure::Local(LocalFailure::UserNotFound),
        );
    };
    let exchange_pub_key = invitee.exchange_pubkey;
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

/// Inputs for [`fetch_member_key_task`].
pub(crate) struct FetchMemberKeyParams {
    pub id: Seq<Request>,
    pub user_id: Id<User>,
    pub url_scheme: crate::http::UrlScheme,
    pub api_base: crate::http::Url<crate::http::RelayHost>,
}

/// Fetch the exchange key the relay serves for a user the client already
/// names, and answer with `SyncEvent::MemberKeyReady`. Nothing is wrapped:
/// the Main thread compares the key with the pin first. The answer carries
/// the id the request named, never one the relay returns.
pub(crate) async fn fetch_member_key_task<H: ShareHttp>(
    http: &H,
    p: FetchMemberKeyParams,
    event_tx: &ambre_bus::conduit::UnboundedSender<SyncEvent>,
    logic_tx: &LogicSink,
) {
    let url = crate::http::Url::<crate::http::ApiRoot>::of(p.url_scheme, &p.api_base)
        .route(format!("/users/{}/pubkeys", p.user_id));
    let served = match http.get_json::<http::PubkeysResponse>(&url).await {
        Ok(served) => served,
        Err(e) => {
            tracing::warn!("Sync: key fetch failed: {e}");
            return api_error(logic_tx, ApiRequest::FetchMemberKey, e.failure());
        }
    };
    let exchange_pub_key = served.exchange_pubkey;
    let _ = event_tx.send(SyncEvent::MemberKeyReady {
        id: p.id,
        user_id: p.user_id,
        fingerprint: ambre_crypto::fingerprint(&exchange_pub_key),
        exchange_pub_key,
    });
}

/// How many times a rotation is posted when members keep joining between its
/// read and its post (`SYNC-143`). Each try reads the members again.
const ROTATION_TRIES: ambre_core::Count<RotationTries> = ambre_core::Count::new(3);

/// The posts of one rotation, the first included.
enum RotationTries {}

impl ambre_core::NarrowCount for RotationTries {
    const NAME: ambre_core::StaticKey<ambre_core::DebugNames> =
        ambre_core::StaticKey::new("RotationTries");
}

/// Inputs for [`revoke_task`], gathered from `SyncState` before spawning.
pub(crate) struct RevokeParams {
    pub id: Seq<Request>,
    pub document_id: Id<Document>,
    pub user_id: Id<User>,
    pub url_scheme: crate::http::UrlScheme,
    pub api_base: crate::http::Url<crate::http::RelayHost>,
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
    let members_url = crate::http::Url::<crate::http::ApiRoot>::of(p.url_scheme, &p.api_base)
        .route(format!("/documents/{}/members", p.document_id));
    let new_doc_key = DocumentKey::generate();
    let link = match link_to(
        p.document_id,
        &p.previous,
        &new_doc_key,
        &p.author.signing_key,
    ) {
        Ok(link) => link,
        Err(e) => return failed(logic_tx, ApiRequest::RevokeFromDocument, e),
    };
    let revoke_url = crate::http::Url::<crate::http::ApiRoot>::of(p.url_scheme, &p.api_base)
        .route(format!("/documents/{}/revoke", p.document_id));
    let mut tries = ambre_core::Count::<RotationTries>::ZERO;
    let unwrapped = loop {
        // Bounded by `ROTATION_TRIES`: the loop gives up at that count.
        tries = tries.incremented();
        let resp = match http
            .get_json::<http::DocumentMembersResponse>(&members_url)
            .await
        {
            Ok(resp) => resp,
            Err(e) => {
                tracing::warn!("Sync: get members failed: {e}");
                return api_error(logic_tx, ApiRequest::RevokeFromDocument, e.failure());
            }
        };
        let (log, anchor) = served_log(p.document_id, &p.exchange_key, &resp);
        // Compared parsed, so a spelling of the same id cannot keep the member in.
        let (removed, staying): (Vec<_>, Vec<_>) = resp
            .members
            .into_iter()
            .partition(|m| m.user_id == p.user_id);
        let entry = match removed.first().map(|m| {
            let member_key = m
                .signing_pub_key
                .key()
                .copied()
                .ok_or(SyncError::Malformed(Part::SigningKey))?;
            let member_exchange_key = m
                .exchange_pub_key
                .key()
                .copied()
                .ok_or(SyncError::Malformed(Part::ExchangeKey))?;
            let draft = Draft {
                document: p.document_id,
                member: p.user_id,
                member_key,
                member_exchange_key,
                act: MembershipAct::Revoke,
                role: m.role,
                key: Some(new_doc_key.id()),
            };
            entry_after(&log, draft, &p.author)
        }) {
            Some(Ok(entry)) => entry,
            // No row for the member: the relay refuses the removal itself.
            None => None,
            Some(Err(e)) => return failed(logic_tx, ApiRequest::RevokeFromDocument, e),
        };
        let (keys, unwrapped) =
            rotate_entries_for(&p.exchange_key, &new_doc_key, anchor, &log, &staying);
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
                if (e.is(RefusalCode::MembersChanged) || e.is(RefusalCode::LogMoved))
                    && tries < ROTATION_TRIES => {}
            Err(e) => {
                tracing::warn!("Sync: revoke failed: {e}");
                return api_error(logic_tx, ApiRequest::RevokeFromDocument, e.failure());
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
        let _ = logic_tx.send(LogicEvent::KeyNotShared {
            document_id: p.document_id,
            members: unwrapped,
        });
    }
}

/// Inputs for [`leave_task`].
pub(crate) struct LeaveParams {
    pub id: Seq<Request>,
    pub document_id: Id<Document>,
    /// This user, who signs their own revoke: a departure removes no one else.
    pub author: Author,
    pub url_scheme: crate::http::UrlScheme,
    pub api_base: crate::http::Url<crate::http::RelayHost>,
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
        Err(e) => failed(logic_tx, ApiRequest::LeaveDocument, e),
    }
}

/// DELETEs this user's own share of `document_id` with the revoke they sign
/// on the log the relay serves. A log that moved meanwhile is read again.
async fn leave_document<H: ShareHttp>(
    http: &H,
    document_id: Id<Document>,
    author: &Author,
    url_scheme: crate::http::UrlScheme,
    api_base: &crate::http::Url<crate::http::RelayHost>,
    exchange_key: &StaticSecret,
) -> Result<(), SyncError> {
    let me = author.user_id;
    let members_url = crate::http::Url::<crate::http::ApiRoot>::of(url_scheme, api_base)
        .route(format!("/documents/{document_id}/members"));
    let url = crate::http::Url::<crate::http::ApiRoot>::of(url_scheme, api_base)
        .route(format!("/documents/{document_id}/share/{me}"));
    let mut tries = ambre_core::Count::<RotationTries>::ZERO;
    loop {
        // Bounded by `ROTATION_TRIES`: the loop gives up at that count.
        tries = tries.incremented();
        let resp = http
            .get_json::<http::DocumentMembersResponse>(&members_url)
            .await?;
        let (log, _) = served_log(document_id, exchange_key, &resp);
        let role = resp
            .members
            .iter()
            .find(|m| m.user_id == me)
            .map_or(Role::Viewer, |m| m.role);
        let draft = Draft {
            document: document_id,
            member: me,
            member_key: FixedBytes::from_array(author.signing_key.verifying_key().to_bytes()),
            member_exchange_key: own_exchange_key(exchange_key),
            act: MembershipAct::Revoke,
            role,
            key: None,
        };
        let entry = entry_after(&log, draft, author)?;
        let body = http::LeaveRequest { entry };
        match http.delete(&url, &body).await {
            Ok(()) => return Ok(()),
            Err(e) if e.is(RefusalCode::LogMoved) && tries < ROTATION_TRIES => {}
            Err(e) => return Err(e.into()),
        }
    }
}

/// Inputs for [`leave_channel_task`], gathered from `SyncState` before
/// spawning.
pub(crate) struct ChannelLeaveParams {
    pub channel_id: Id<Channel>,
    /// This user, who signs their own revoke on the channel's `meta`.
    pub author: Author,
    pub url_scheme: crate::http::UrlScheme,
    pub api_base: crate::http::Url<crate::http::RelayHost>,
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
) -> Result<(), http::HttpError> {
    let (scheme, base) = (p.url_scheme, &p.api_base);
    let url = crate::http::Url::<crate::http::ApiRoot>::of(scheme, base)
        .route(format!("/channels/{}/documents", p.channel_id));
    let listed = http
        .get_json::<http::ChannelDocumentsResponse>(&url)
        .await?;
    let meta = crate::channels::channel_documents(&listed.documents)
        .into_iter()
        .find(|d| d.kind == ChannelDocumentKind::Meta)
        .ok_or(http::HttpError::Local(LocalFailure::NoMetaDocument))?
        .document_id;
    leave_document(http, meta, &p.author, scheme, base, &p.exchange_key)
        .await
        .map_err(http::HttpError::from)
}

/// The `meta` document of `channel`, whose membership log admits the joiners of its calls,
/// or `None` when the relay answers no such document.
pub(crate) async fn channel_meta<H: ShareHttp>(
    http: &H,
    channel: Id<Channel>,
    scheme: crate::http::UrlScheme,
    base: &crate::http::Url<crate::http::RelayHost>,
) -> Option<Id<Document>> {
    let url = crate::http::Url::<crate::http::ApiRoot>::of(scheme, base)
        .route(format!("/channels/{channel}/documents"));
    let listed = http
        .get_json::<http::ChannelDocumentsResponse>(&url)
        .await
        .ok()?;
    crate::channels::channel_documents(&listed.documents)
        .into_iter()
        .find(|d| d.kind == ChannelDocumentKind::Meta)
        .map(|d| d.document_id)
}

/// Inputs for [`close_segment_task`], gathered from `SyncState` before
/// spawning.
pub(crate) struct SegmentCloseParams {
    pub channel_id: Id<Channel>,
    pub closing: Id<Segment>,
    pub successor: Id<Segment>,
    /// The current key of the channel's `meta`, which opens every document
    /// of the channel.
    pub meta_key: DocumentKey,
    /// This user, who creates the successor.
    pub author: Author,
    pub url_scheme: crate::http::UrlScheme,
    pub api_base: crate::http::Url<crate::http::RelayHost>,
    pub exchange_key: StaticSecret,
}

/// Close the active segment of a channel (the Murmure segments spec).
/// Create the successor under the key of `meta`, then ask the relay to close
/// into it. Returns the relay's answer: the active segment after the close,
/// and whether it is this successor.
///
/// A close that lost its race deletes the successor it created, which no
/// channel lists (`SYNC-163`). A failed request deletes nothing: the relay
/// may have registered the successor before the answer was lost.
pub(crate) async fn close_segment_task<H: ShareHttp>(
    http: &H,
    p: SegmentCloseParams,
) -> Result<http::ClosedSegmentResponse, http::HttpError> {
    let base = crate::http::Url::<crate::http::ApiRoot>::of(p.url_scheme, &p.api_base);
    let successor = p.successor.document_id();
    let (doc_key_enc, entry) = creation(successor, &p.meta_key, &p.exchange_key, &p.author)?;
    let body = http::CreateDocumentRequest {
        doc_type: ambre_core::DocType::Murmure,
        name_enc: None,
        doc_key_enc,
        document_id: Some(successor),
        entry: Some(entry),
        space: ambre_core::DocumentScope::Personal,
    };
    http.post_json::<_, http::CreateDocumentResponse>(&base.route("/documents"), &body)
        .await?;
    let body = http::CloseSegmentRequest {
        closing: p.closing,
        successor: p.successor,
    };
    let url = base.route(format!("/channels/{}/segments", p.channel_id));
    let closed = http
        .post_json::<_, http::ClosedSegmentResponse>(&url, &body)
        .await?;
    if closed.created == ambre_core::Creation::Existing {
        let url = base.route(format!("/documents/{successor}"));
        if let Err(e) = http.delete(&url, &()).await {
            tracing::warn!("[Sync] the successor of a lost close stays on the relay: {e}");
        }
    }
    Ok(closed)
}

/// Inputs for [`amendments_task`], gathered from `SyncState` before
/// spawning.
pub(crate) struct AmendmentsParams {
    pub channel_id: Id<Channel>,
    pub segment: Id<Segment>,
    pub amendments: Id<Document>,
    /// The current key of the channel's `meta`, which opens every document
    /// of the channel.
    pub meta_key: DocumentKey,
    /// This user, who creates the document.
    pub author: Author,
    pub url_scheme: crate::http::UrlScheme,
    pub api_base: crate::http::Url<crate::http::RelayHost>,
    pub exchange_key: StaticSecret,
}

/// Create the amendments document of a locked segment under the key of
/// `meta`, then register it for the segment (`SRV-201`). Returns the
/// document the relay holds for the segment.
///
/// A registration that lost its race deletes the document it created
/// (`SYNC-166`). A failed request deletes nothing, as at a close.
pub(crate) async fn amendments_task<H: ShareHttp>(
    http: &H,
    p: AmendmentsParams,
) -> Result<Id<Document>, http::HttpError> {
    let base = crate::http::Url::<crate::http::ApiRoot>::of(p.url_scheme, &p.api_base);
    let (doc_key_enc, entry) = creation(p.amendments, &p.meta_key, &p.exchange_key, &p.author)?;
    let body = http::CreateDocumentRequest {
        doc_type: ambre_core::DocType::Murmure,
        name_enc: None,
        doc_key_enc,
        document_id: Some(p.amendments),
        entry: Some(entry),
        space: ambre_core::DocumentScope::Personal,
    };
    http.post_json::<_, http::CreateDocumentResponse>(&base.route("/documents"), &body)
        .await?;
    let body = http::RegisterAmendmentsRequest {
        amendments: p.amendments,
    };
    let url = base.route(format!(
        "/channels/{}/segments/{}/amendments",
        p.channel_id, p.segment
    ));
    let linked = http
        .post_json::<_, http::AmendmentsResponse>(&url, &body)
        .await?;
    if linked.created == ambre_core::Creation::Existing {
        let url = base.route(format!("/documents/{}", p.amendments));
        if let Err(e) = http.delete(&url, &()).await {
            tracing::warn!("[Sync] the amendments of a lost registration stay on the relay: {e}");
        }
    }
    Ok(linked.amendments)
}

/// Inputs for [`thread_task`], gathered from `SyncState` before spawning.
pub(crate) struct ThreadParams {
    pub channel_id: Id<Channel>,
    pub root: Id<Message>,
    pub thread: Id<Document>,
    /// The current key of the channel's `meta`, which opens every document
    /// of the channel.
    pub meta_key: DocumentKey,
    /// This user, who creates the document.
    pub author: Author,
    pub url_scheme: crate::http::UrlScheme,
    pub api_base: crate::http::Url<crate::http::RelayHost>,
    pub exchange_key: StaticSecret,
}

/// Create the thread document of a root under the key of `meta`, then
/// register it for the root (`SRV-213`). Returns the document the relay
/// holds for the root.
///
/// A registration that lost its race deletes the document it created
/// (`SYNC-169`). A failed request deletes nothing, as for amendments.
pub(crate) async fn thread_task<H: ShareHttp>(
    http: &H,
    p: ThreadParams,
) -> Result<Id<Document>, http::HttpError> {
    let base = crate::http::Url::<crate::http::ApiRoot>::of(p.url_scheme, &p.api_base);
    let (doc_key_enc, entry) = creation(p.thread, &p.meta_key, &p.exchange_key, &p.author)?;
    let body = http::CreateDocumentRequest {
        doc_type: ambre_core::DocType::Murmure,
        name_enc: None,
        doc_key_enc,
        document_id: Some(p.thread),
        entry: Some(entry),
        space: ambre_core::DocumentScope::Personal,
    };
    http.post_json::<_, http::CreateDocumentResponse>(&base.route("/documents"), &body)
        .await?;
    let body = http::RegisterThreadRequest {
        root: p.root,
        thread: p.thread,
    };
    let url = base.route(format!("/channels/{}/threads", p.channel_id));
    let linked = http
        .post_json::<_, http::ThreadResponse>(&url, &body)
        .await?;
    if linked.created == ambre_core::Creation::Existing {
        let url = base.route(format!("/documents/{}", p.thread));
        if let Err(e) = http.delete(&url, &()).await {
            tracing::warn!("[Sync] the thread of a lost registration stays on the relay: {e}");
        }
    }
    Ok(linked.thread)
}

/// Inputs for [`settle_task`].
pub(crate) struct SettleParams {
    pub document_id: Id<Document>,
    pub url_scheme: crate::http::UrlScheme,
    pub api_base: crate::http::Url<crate::http::RelayHost>,
    pub exchange_key: StaticSecret,
    /// The key the rotation replaces, sealed under the new one (`SYNC-116`).
    pub previous: DocumentKey,
    /// Signs the link the rotation leaves.
    pub signing_key: SigningKey,
}

/// Inputs for [`role_change_task`].
pub(crate) struct RoleChangeParams {
    pub id: Seq<Request>,
    pub document_id: Id<Document>,
    /// The member whose role changes.
    pub user_id: Id<User>,
    pub role: Role,
    pub url_scheme: crate::http::UrlScheme,
    pub api_base: crate::http::Url<crate::http::RelayHost>,
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
    let members_url = crate::http::Url::<crate::http::ApiRoot>::of(p.url_scheme, &p.api_base)
        .route(format!("/documents/{}/members", p.document_id));
    let url = crate::http::Url::<crate::http::ApiRoot>::of(p.url_scheme, &p.api_base).route(
        format!("/documents/{}/members/{}", p.document_id, p.user_id),
    );
    let mut tries = ambre_core::Count::<RotationTries>::ZERO;
    loop {
        // Bounded by `ROTATION_TRIES`: the loop gives up at that count.
        tries = tries.incremented();
        let resp = match http
            .get_json::<http::DocumentMembersResponse>(&members_url)
            .await
        {
            Ok(resp) => resp,
            Err(e) => {
                tracing::warn!("Sync: get members failed: {e}");
                return api_error(logic_tx, ApiRequest::UpdateDocumentMemberRole, e.failure());
            }
        };
        let (log, _) = served_log(p.document_id, &p.exchange_key, &resp);
        let Some(target) = resp.members.iter().find(|m| m.user_id == p.user_id) else {
            return api_error(
                logic_tx,
                ApiRequest::UpdateDocumentMemberRole,
                ApiFailure::Local(LocalFailure::MemberGone),
            );
        };
        let keys = target
            .signing_pub_key
            .key()
            .copied()
            .ok_or(SyncError::Malformed(Part::SigningKey))
            .and_then(|member_key| {
                target
                    .exchange_pub_key
                    .key()
                    .copied()
                    .ok_or(SyncError::Malformed(Part::ExchangeKey))
                    .map(|exchange| (member_key, exchange))
            });
        let entry = keys.and_then(|(member_key, member_exchange_key)| {
            let draft = Draft {
                document: p.document_id,
                member: p.user_id,
                member_key,
                member_exchange_key,
                act: MembershipAct::ChangeRole,
                role: p.role,
                key: None,
            };
            entry_after(&log, draft, &p.author)
        });
        let entry = match entry {
            Ok(entry) => entry,
            Err(e) => return failed(logic_tx, ApiRequest::UpdateDocumentMemberRole, e),
        };
        let body = http::RoleChangeRequest {
            role: p.role,
            entry,
        };
        match http.patch(&url, &body).await {
            Ok(()) => break,
            Err(e) if e.is(RefusalCode::LogMoved) && tries < ROTATION_TRIES => {}
            Err(e) => return failed(logic_tx, ApiRequest::UpdateDocumentMemberRole, e),
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
    document_id: Id<Document>,
    previous: &DocumentKey,
    next: &DocumentKey,
    signing_key: &SigningKey,
) -> Result<
    (
        ambre_protocol::KeyLink,
        ambre_core::Encoded<ambre_core::EncodedKeyLink>,
    ),
    SyncError,
> {
    let link = crate::pipeline::seal_link(document_id, previous, next, signing_key)?;
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
    let members_url = crate::http::Url::<crate::http::ApiRoot>::of(p.url_scheme, &p.api_base)
        .route(format!("/documents/{}/members", document_id));
    let rotate_url = crate::http::Url::<crate::http::ApiRoot>::of(p.url_scheme, &p.api_base)
        .route(format!("/documents/{}/rotate-key", document_id));
    let new_doc_key = DocumentKey::generate();
    let (link, text) = match link_to(document_id, &p.previous, &new_doc_key, &p.signing_key) {
        Ok(link) => link,
        Err(e) => return unsettled(e.to_string()),
    };
    let mut tries = ambre_core::Count::<RotationTries>::ZERO;
    loop {
        // Bounded by `ROTATION_TRIES`: the loop gives up at that count.
        tries = tries.incremented();
        let resp = match http
            .get_json::<http::DocumentMembersResponse>(&members_url)
            .await
        {
            Ok(resp) => resp,
            Err(e) => return unsettled(format!("Get members failed: {e}")),
        };
        if resp.rotation_owed == ambre_core::RotationDebt::Settled {
            let _ = event_tx.send(SyncEvent::DebtGone { document_id });
            return;
        }
        let (log, anchor) = served_log(document_id, &p.exchange_key, &resp);
        let (keys, unwrapped) =
            rotate_entries_for(&p.exchange_key, &new_doc_key, anchor, &log, &resp.members);
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
            Err(e) if e.is(RefusalCode::MembersChanged) && tries < ROTATION_TRIES => {}
            Err(e) => return unsettled(format!("Rotate key failed: {e}")),
        }
    }
}

/// Inputs for [`rotate_channel_task`].
pub(crate) struct RotateChannelParams {
    pub space_id: Id<Space>,
    pub channel_id: Id<Channel>,
    pub url_scheme: crate::http::UrlScheme,
    pub api_base: crate::http::Url<crate::http::RelayHost>,
    pub exchange_key: StaticSecret,
    /// Signs the link the rotation leaves.
    pub signing_key: SigningKey,
    /// The channel key the catalogue holds, which the rotation replaces.
    pub previous: DocumentKey,
    /// The anchor of meta's log, sealed beside the catalogue's key.
    pub anchor: FixedBytes<EntryDigest>,
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
) -> Result<ambre_bus::SealedChannelKey, http::HttpError> {
    let base = crate::http::Url::<crate::http::ApiRoot>::of(p.url_scheme, &p.api_base);
    let head = http
        .get_json::<http::JoinHeadResponse>(&base.route(format!("/channels/{}/join", p.channel_id)))
        .await
        .inspect_err(|e| tracing::warn!("Sync: get the join head failed: {e}"))?;
    let meta = head.meta_id.document_id();
    let members_url = base.route(format!("/documents/{meta}/members"));
    let rotate_url = base.route(format!("/documents/{meta}/rotate-key"));
    let new_key = DocumentKey::generate();
    let (_, text) = link_to(meta, &p.previous, &new_key, &p.signing_key)?;
    let mut tries = ambre_core::Count::<RotationTries>::ZERO;
    loop {
        // Bounded by `ROTATION_TRIES`: the loop gives up at that count.
        tries = tries.incremented();
        let resp = http
            .get_json::<http::DocumentMembersResponse>(&members_url)
            .await
            .inspect_err(|e| tracing::warn!("Sync: get members failed: {e}"))?;
        // A space admin may hold no row of meta. The walk starts at the
        // anchor the catalogue sealed, under the rule of a public channel.
        let log = walk_served_under(
            meta,
            Some(p.anchor),
            &resp.log,
            &LogRule {
                self_join: membership_log::public_channel_self_join,
                space: None,
            },
        );
        let (keys, unwrapped) = rotate_entries_for(
            &p.exchange_key,
            &new_key,
            Some(p.anchor),
            &log,
            &resp.members,
        );
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
            Err(e) if e.is(RefusalCode::MembersChanged) && tries < ROTATION_TRIES => {}
            Err(e) => {
                tracing::warn!("Sync: rotate key failed: {e}");
                return Err(e);
            }
        }
    }
    crate::channels::seal_channel_key(p.channel_id, &new_key, &p.space_key, p.anchor)
        .map_err(http::HttpError::from)
}

/// Inputs for [`list_members_task`].
pub(crate) struct ListMembersParams {
    /// The request to answer, `None` for a refresh of the directory alone.
    pub id: Option<Seq<Request>>,
    /// Which listing of this document this is, so an answer older than the
    /// directory held changes nothing (`SYNC-037`).
    pub asked: ambre_core::Seq<crate::generation::Listing>,
    pub document_id: Id<Document>,
    pub url_scheme: crate::http::UrlScheme,
    pub api_base: crate::http::Url<crate::http::RelayHost>,
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
    let url = crate::http::Url::<crate::http::ApiRoot>::of(p.url_scheme, &p.api_base)
        .route(format!("/documents/{}/members", p.document_id));
    match http.get_json::<http::DocumentMembersResponse>(&url).await {
        Ok(resp) => {
            // Refresh the presence directory off the same response
            // (`ambre-spec-membership.md`). Membership just changed often enough to ask
            // for the list. This is where a new member becomes resolvable and a revoked
            // one stops resolving, with no extra round-trip.
            // Each row carries what the log says of it (`SYNC-108`).
            let (log, _) = served_log_under(p.document_id, &p.exchange_key, &resp, &p.rule);
            let signing_keys = presence_directory_under(
                &log,
                resp.members
                    .iter()
                    .map(|m| (m.user_id, m.signing_pub_key.key())),
            );
            let word = membership_log::SpaceWord::of(&log);
            let heads = log.heads();
            let writers = membership_log::Writers::of(&log);
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
                heads,
                writers,
            });
        }
        Err(e) => match p.id {
            Some(id) => {
                let _ = event_tx.send(SyncEvent::http_error(id, e));
            }
            // No one waits on a refresh of the directory alone: the
            // directory it held stays, and the next change asks again. A
            // log head under check hears of it (`SYNC-220`).
            None => {
                tracing::warn!("Sync: the members could not be read again: {e}");
                let _ = event_tx.send(SyncEvent::DocumentMembersFailed {
                    document_id: p.document_id,
                    asked: p.asked,
                });
            }
        },
    }
}

/// What [`call_log_task`] reads: the `meta` document of the channel of a call.
pub(crate) struct CallLogParams {
    pub document_id: Id<Document>,
    pub url_scheme: crate::http::UrlScheme,
    pub api_base: crate::http::Url<crate::http::RelayHost>,
    /// Unwraps our own row, whose seal carries the log anchor (`SYNC-104`).
    pub exchange_key: StaticSecret,
    /// The rule the log walks under (`SYNC-106`).
    pub rule: LogRule,
}

/// Reads the membership log of the channel of a call, walked from our anchor, and answers
/// [`SyncEvent::CallLog`]. A failed read answers nothing: the call keeps the log it held,
/// and the next change of the members reads it again.
#[cfg_attr(
    all(not(test), not(target_arch = "wasm32")),
    expect(dead_code, reason = "only the wasm transport of a call reads its log")
)]
pub(crate) async fn call_log_task<H: ShareHttp>(
    http: &H,
    p: CallLogParams,
    event_tx: &ambre_bus::conduit::UnboundedSender<SyncEvent>,
) {
    let url = crate::http::Url::<crate::http::ApiRoot>::of(p.url_scheme, &p.api_base)
        .route(format!("/documents/{}/members", p.document_id));
    match http.get_json::<http::DocumentMembersResponse>(&url).await {
        Ok(resp) => {
            let (log, _) = served_log_under(p.document_id, &p.exchange_key, &resp, &p.rule);
            let _ = event_tx.send(SyncEvent::CallLog {
                meta: p.document_id,
                log,
            });
        }
        Err(e) => tracing::warn!("Sync: the log of the call was not read: {e}"),
    }
}

/// What an ask of who is in a call read. The seats are `None` on a failed read. The log of the
/// channel is there when the ask was given one to read.
#[cfg_attr(
    not(target_arch = "wasm32"),
    expect(dead_code, reason = "only the wasm transport asks the relay")
)]
pub(crate) struct RosterRead {
    pub roster: Option<ambre_protocol::signaling::CallRoster>,
    pub log: Option<Log>,
}

/// Asks the relay who is in `call` of `channel` (`CALL-070`). It reads the membership log of
/// the channel too when `log` names it (the Murmure calls spec).
#[cfg_attr(
    not(target_arch = "wasm32"),
    expect(dead_code, reason = "only the wasm transport asks the relay")
)]
pub(crate) async fn call_roster_task<H: ShareHttp>(
    http: &H,
    call: Id<ambre_core::id::Call>,
    channel: Id<Channel>,
    scheme: crate::http::UrlScheme,
    base: &crate::http::Url<crate::http::RelayHost>,
    log: Option<CallLogParams>,
) -> RosterRead {
    let url = crate::http::Url::<crate::http::ApiRoot>::of(scheme, base)
        .route(format!("/calls/{call}/roster?channel={channel}"));
    let roster = match http
        .get_json::<ambre_protocol::signaling::CallRoster>(&url)
        .await
    {
        Ok(roster) => Some(roster),
        Err(e) => {
            tracing::warn!("Sync: who is in the call was not read: {e}");
            None
        }
    };
    let log = match log {
        Some(p) => {
            let url = crate::http::Url::<crate::http::ApiRoot>::of(p.url_scheme, &p.api_base)
                .route(format!("/documents/{}/members", p.document_id));
            match http.get_json::<http::DocumentMembersResponse>(&url).await {
                Ok(resp) => {
                    Some(served_log_under(p.document_id, &p.exchange_key, &resp, &p.rule).0)
                }
                Err(e) => {
                    tracing::warn!("Sync: the log of the channel of a call was not read: {e}");
                    None
                }
            }
        }
        None => None,
    };
    RosterRead { roster, log }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The tries of a rotation as the tests count posts.
    const ROTATION_TRIES: usize = super::ROTATION_TRIES.get() as usize;
    use ambre_core::id::Meta;
    use ambre_crypto::MasterSeed;
    use x25519_dalek::PublicKey;

    /// A deterministic (secret, public) X25519 pair for a fake peer.
    fn keypair(seed: u8) -> (StaticSecret, FixedBytes<ambre_core::ExchangePublicKey>) {
        let xk = MasterSeed::from_array([seed; 32]).derive_exchange_key();
        let public = FixedBytes::from_array(PublicKey::from(&xk).to_bytes());
        (xk, public)
    }

    fn member(
        uid: &str,
        exchange_pub_key: FixedBytes<ambre_core::ExchangePublicKey>,
    ) -> http::DocumentMemberEntry {
        http::DocumentMemberEntry {
            user_id: Id::<User>::parse_str(uid).expect("a test id"),
            display_name: ambre_core::Validated::<ambre_core::DisplayName>::parse("Member")
                .unwrap(),
            email: "m@test.local".into(),
            signing_pub_key: http::ServedKey::of(FixedBytes::from_array([0; 32])),
            exchange_pub_key: http::ServedKey::of(exchange_pub_key),
            doc_key_enc: ambre_core::Encoded::new(Vec::new()),
            role: Role::Editor,
            wrapper_exchange_pub_key: http::ServedKey::of(exchange_pub_key),
        }
    }

    /// Decode helper: the inverse of `wrap_anchored_b64`, used by recipients.
    fn unwrap_b64(
        recipient: &StaticSecret,
        wrapper_pub: &FixedBytes<ambre_core::ExchangePublicKey>,
        doc_key_enc: &ambre_core::Encoded<ambre_core::WrappedDocKey>,
    ) -> Result<DocumentKey, SyncError> {
        unwrap_anchor_b64(recipient, wrapper_pub, doc_key_enc).map(|(key, _)| key)
    }

    /// As [`unwrap_b64`], with the anchor sealed beside the key.
    fn unwrap_anchor_b64(
        recipient: &StaticSecret,
        wrapper_pub: &FixedBytes<ambre_core::ExchangePublicKey>,
        doc_key_enc: &ambre_core::Encoded<ambre_core::WrappedDocKey>,
    ) -> Result<(DocumentKey, Option<FixedBytes<EntryDigest>>), SyncError> {
        let own = own_exchange_key(recipient);
        unwrap_own(recipient, [(&own, doc_key_enc, Some(wrapper_pub))])
    }

    /// An exchange key that is not hex of the right width refuses the answer
    /// that carries it when it is read. So no member row holds one.
    #[test]
    fn decode_exchange_pubkey_rejects_bad_input() {
        let read = |exchange_pubkey: String| {
            serde_json::from_value::<http::PubkeysResponse>(serde_json::json!({
                "signing_pubkey": "cd".repeat(32),
                "exchange_pubkey": exchange_pubkey,
            }))
        };
        assert!(read("not-hex".to_owned()).is_err());
        // 31 bytes -> wrong length.
        assert!(read("ab".repeat(31)).is_err());
        assert!(read("ab".repeat(32)).is_ok());
    }

    #[test]
    fn invite_request_wraps_for_the_invitee_only() {
        let (my_xk, my_pub) = keypair(1);
        let (invitee_xk, invitee_pub_hex) = keypair(2);
        let (stranger_xk, _) = keypair(3);
        let doc_key = DocumentKey::generate();

        let req = build_invite_request(
            &my_xk,
            &invitee_pub_hex,
            InviteeName::Email("invitee@test.local".into()),
            &doc_key,
            Role::Editor,
            None,
            None,
        )
        .expect("valid pubkey");

        assert_eq!(
            req.invitee_email.as_ref().map(ambre_core::Email::as_str),
            Some("invitee@test.local")
        );
        assert_eq!(req.invitee_id, None);
        assert_eq!(req.role, Role::Editor);
        // The invitee unwraps against the inviter's public key: succeeds.
        assert!(unwrap_b64(&invitee_xk, &my_pub, &req.doc_key_enc).is_ok());
        // A stranger cannot (AEAD fails closed under the wrong shared secret).
        assert!(unwrap_b64(&stranger_xk, &my_pub, &req.doc_key_enc).is_err());
    }

    #[test]
    fn invite_request_propagates_bad_pubkey() {
        let (my_xk, _) = keypair(1);
        // The identity point: a peer key with no contribution, which the wrap
        // refuses (`SYNC-035`).
        let err = build_invite_request(
            &my_xk,
            &FixedBytes::from_array([0; 32]),
            InviteeName::Email("x@y.z".into()),
            &DocumentKey::generate(),
            Role::Viewer,
            None,
            None,
        );
        assert!(err.is_err());
    }

    /// A log that signs, for each of `members`, the exchange key its row
    /// serves.
    fn signed_by_the_log(members: &[http::DocumentMemberEntry]) -> Log {
        let signed: Vec<_> = members
            .iter()
            .filter_map(|m| Some((m.user_id, *m.exchange_pub_key.key()?)))
            .collect();
        crate::membership_log::tests::signing(&signed)
    }

    /// Holds `SYNC-198`. A rotation wraps the next key for a row only under
    /// an exchange key a valid entry of the log signs. A row the relay adds,
    /// and a row whose key the relay swapped, each get no copy and are
    /// reported by name. This client's own row needs no entry.
    #[test]
    fn a_rotation_wraps_for_no_exchange_key_the_log_does_not_sign() {
        let (my_xk, my_pub) = keypair(1);
        let (alice_xk, alice_pub) = keypair(2);
        let (bob_signed_xk, bob_signed) = keypair(3);
        let (relay_xk, relay_pub) = keypair(4);
        let named = |name: &str, mut row: http::DocumentMemberEntry| {
            row.display_name = ambre_core::Validated::parse(name).unwrap();
            row
        };
        let me = || named("Me", member("10000000-0000-0000-0000-000000000000", my_pub));
        let alice = || {
            named(
                "Alice",
                member("20000000-0000-0000-0000-000000000000", alice_pub),
            )
        };
        // The relay serves its own key on Bob's row, and a row of its own.
        let bob = named(
            "Bob",
            member("30000000-0000-0000-0000-000000000000", relay_pub),
        );
        let mallory = named(
            "Mallory",
            member("40000000-0000-0000-0000-000000000000", relay_pub),
        );
        let log = crate::membership_log::tests::signing(&[
            (alice().user_id, alice_pub),
            (bob.user_id, bob_signed),
        ]);
        let rows = [me(), alice(), bob, mallory];
        let new_key = DocumentKey::generate();
        let (entries, unwrapped) = rotate_entries_for(&my_xk, &new_key, None, &log, &rows);
        let wrapped: Vec<_> = entries.iter().map(|e| e.user_id).collect();
        assert_eq!(wrapped, [me().user_id, alice().user_id]);
        let names: Vec<_> = unwrapped.iter().map(|n| n.as_str().to_owned()).collect();
        assert_eq!(names, ["Bob", "Mallory"]);
        // Alice opens her copy; the relay's key opens none, nor Bob's own.
        assert!(unwrap_b64(&alice_xk, &my_pub, &entries[1].doc_key_enc).is_ok());
        for entry in &entries {
            assert!(unwrap_b64(&relay_xk, &my_pub, &entry.doc_key_enc).is_err());
            assert!(unwrap_b64(&bob_signed_xk, &my_pub, &entry.doc_key_enc).is_err());
        }
        // A log that verifies nobody leaves this client's own copy alone.
        let unanchored = Log::walk(
            Id::<Document>::from_bytes([7; 16]),
            None,
            &[],
            crate::membership_log::no_self_join,
        );
        let (entries, unwrapped) =
            rotate_entries_for(&my_xk, &new_key, None, &unanchored, &[me(), alice()]);
        assert_eq!(
            entries.iter().map(|e| e.user_id).collect::<Vec<_>>(),
            [me().user_id]
        );
        assert_eq!(unwrapped.len(), 1);
    }

    proptest::proptest! {
        /// Holds `SYNC-198` over any mix of rows. A copy goes to a row exactly
        /// when the log signs the key it serves, or when the row is this
        /// client's own. Every other row is reported. Two paths agree: the
        /// rows the property marks, and the copies the rotation posts.
        #[test]
        fn a_copy_goes_to_a_signed_key_alone(kinds in proptest::collection::vec(0u8..4, 1..8)) {
            let (my_xk, my_pub) = keypair(1);
            let mut rows = Vec::new();
            let mut signed = Vec::new();
            let mut expected = Vec::new();
            for (place, kind) in kinds.iter().enumerate() {
                let seed = u8::try_from(place).unwrap() + 10;
                let (_, served) = keypair(seed);
                let uid = format!("{:08x}-0000-0000-0000-000000000000", place);
                // 0: signed, 1: injected, 2: swapped by the relay, 3: own.
                let row = member(&uid, if *kind == 3 { my_pub } else { served });
                match kind {
                    0 => signed.push((row.user_id, served)),
                    2 => signed.push((row.user_id, keypair(seed + 100).1)),
                    _ => {}
                }
                if matches!(kind, 0 | 3) {
                    expected.push(row.user_id);
                }
                rows.push(row);
            }
            let log = crate::membership_log::tests::signing(&signed);
            let (entries, unwrapped) =
                rotate_entries_for(&my_xk, &DocumentKey::generate(), None, &log, &rows);
            let wrapped: Vec<_> = entries.iter().map(|e| e.user_id).collect();
            proptest::prop_assert_eq!(&wrapped, &expected);
            proptest::prop_assert_eq!(wrapped.len() + unwrapped.len(), rows.len());
        }
    }

    #[test]
    fn rotate_entries_cover_valid_members_and_report_malformed() {
        let (my_xk, my_pub) = keypair(1);
        let (a_xk, a_pub) = keypair(2);
        let (_b_xk, b_pub) = keypair(3);
        let new_key = DocumentKey::generate();

        // The middle member's key is the identity point, which no wrap takes.
        let members = vec![
            member("11111111-1111-1111-1111-111111111111", a_pub),
            member(
                "22222222-2222-2222-2222-222222222222",
                FixedBytes::from_array([0; 32]),
            ),
            member("33333333-3333-3333-3333-333333333333", b_pub),
        ];
        let log = signed_by_the_log(&members);
        let (entries, unwrapped) = rotate_entries_for(&my_xk, &new_key, None, &log, &members);

        // The member the wrap refuses is returned for the report; the two
        // valid ones are wrapped (`SYNC-043`).
        assert_eq!(unwrapped.len(), 1);
        assert_eq!(entries.len(), 2);
        let id = |s| Id::<User>::parse_str(s).unwrap();
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

    /// `SYNC-043`: a member whose exchange key the relay serves malformed
    /// stays in the answer, is wrapped for by nobody, and is reported by name.
    #[test]
    fn a_malformed_exchange_key_reaches_the_report_by_name() {
        let (my_xk, _) = keypair(1);
        let (_, a_pub) = keypair(2);
        let row = |user_id: &str, name: &str, exchange: String| {
            serde_json::json!({
                "user_id": user_id,
                "display_name": name, "email": "m@test.local",
                "signing_pub_key": "cd".repeat(32), "exchange_pub_key": exchange,
                "doc_key_enc": ambre_core::Encoded::<ambre_core::WrappedDocKey>::new(vec![0; 72]),
                "role": "editor", "wrapper_exchange_pub_key": "ab".repeat(32),
            })
        };
        let answer = serde_json::from_value::<http::DocumentMembersResponse>(serde_json::json!({
            "members": [
                row("11111111-1111-1111-1111-111111111111", "Alice", a_pub.to_hex()),
                row("22222222-2222-2222-2222-222222222222", "Carol", "zz-not-hex".to_owned()),
            ]
        }))
        .expect("a malformed key never refuses the whole answer");
        let new_key = DocumentKey::generate();
        // The log signs a sound key for Carol: the malformed row alone keeps
        // her out.
        let log = crate::membership_log::tests::signing(&[
            (answer.members[0].user_id, a_pub),
            (answer.members[1].user_id, keypair(3).1),
        ]);
        let (entries, unwrapped) =
            rotate_entries_for(&my_xk, &new_key, None, &log, &answer.members);
        assert_eq!(entries.len(), 1);
        assert_eq!(
            entries[0].user_id,
            Id::<User>::parse_str("11111111-1111-1111-1111-111111111111").unwrap()
        );
        assert_eq!(
            unwrapped,
            [ambre_core::Validated::<ambre_core::DisplayName>::parse("Carol").unwrap()]
        );
    }

    #[test]
    fn map_members_parses_roles_and_refuses_malformed_ids() {
        // The whole members answer is read, as the relay serves it, so the
        // test pins the path production takes.
        let read = |user_id: &str, role: &str| {
            serde_json::from_value::<http::DocumentMembersResponse>(serde_json::json!({
                "members": [ {
                    "user_id": user_id,
                    "display_name": "Member",
                    "email": "m@test.local",
                    "signing_pub_key": "cd".repeat(32),
                    "exchange_pub_key": "ab".repeat(32),
                    "doc_key_enc": ambre_core::Encoded::<ambre_core::WrappedDocKey>::new(vec![0; 72]),
                    "role": role,
                    "wrapper_exchange_pub_key": "ab".repeat(32),
                } ]
            }))
        };
        let sound_id = "44444444-4444-4444-4444-444444444444";
        // An unparsable id refuses the answer when it is read, so nothing is
        // left to replace by an invented one (`SYNC-038`).
        assert!(read("not-a-uuid", "editor").is_err());
        // So does a role word no build wrote, and a spelling of a known one
        // (`SRV-180`).
        assert!(read(sound_id, "bogus").is_err());
        assert!(read(sound_id, "Editor").is_err());
        let sound = read(sound_id, "editor").expect("a sound answer");
        let none = Log::walk(
            Id::<Document>::mint(),
            None,
            &[],
            membership_log::no_self_join,
        );
        let mapped = map_members(sound.members, &none);
        assert_eq!(mapped[0].user_id, Id::<User>::parse_str(sound_id).unwrap());
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
        responses: RefCell<VecDeque<Result<serde_json::Value, http::HttpError>>>,
        calls: RefCell<Vec<Call>>,
    }

    impl FakeHttp {
        fn new(responses: Vec<Result<serde_json::Value, http::HttpError>>) -> Self {
            Self {
                responses: RefCell::new(responses.into()),
                calls: RefCell::new(Vec::new()),
            }
        }
        fn next(&self) -> Result<serde_json::Value, http::HttpError> {
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
        async fn get_json<T: DeserializeOwned>(
            &self,
            url: &crate::http::Url<http::Route>,
        ) -> Result<T, http::HttpError> {
            self.calls.borrow_mut().push(Call::Get(url.to_string()));
            let v = self.next()?;
            serde_json::from_value(v)
                .map_err(|_| http::HttpError::Local(ambre_core::LocalFailure::UnexpectedAnswer))
        }
        async fn post_json<B: Serialize, R: DeserializeOwned>(
            &self,
            url: &crate::http::Url<http::Route>,
            body: &B,
        ) -> Result<R, http::HttpError> {
            let bv = serde_json::to_value(body).unwrap();
            self.calls
                .borrow_mut()
                .push(Call::Post(url.to_string(), bv));
            let v = self.next()?;
            serde_json::from_value(v)
                .map_err(|_| http::HttpError::Local(ambre_core::LocalFailure::UnexpectedAnswer))
        }
        async fn delete<B: Serialize>(
            &self,
            url: &crate::http::Url<http::Route>,
            body: &B,
        ) -> Result<(), http::HttpError> {
            let bv = serde_json::to_value(body).unwrap();
            self.calls
                .borrow_mut()
                .push(Call::Delete(url.to_string(), bv));
            self.next().map(|_| ())
        }
        async fn patch<B: Serialize>(
            &self,
            url: &crate::http::Url<http::Route>,
            body: &B,
        ) -> Result<(), http::HttpError> {
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
            user_id: Id::<User>::mint(),
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

    /// The user the search answers, and whom phase 1 confirmed.
    const INVITEE: &str = "55555555-5555-5555-5555-555555555555";

    fn user_search_json(
        email: &str,
        exchange_pub_hex: &FixedBytes<ambre_core::ExchangePublicKey>,
    ) -> serde_json::Value {
        serde_json::json!({ "users": [ {
            "id": INVITEE,
            "email": email,
            "display_name": "Invitee",
            "signing_pubkey": "00".repeat(32),
            "exchange_pubkey": exchange_pub_hex,
        } ] })
    }

    fn invite_params(
        my_xk: &StaticSecret,
        expect: FixedBytes<ambre_core::ExchangePublicKey>,
    ) -> InviteParams {
        InviteParams {
            id: ambre_bus::next_request(),
            document_id: Id::<Document>::mint(),
            url_scheme: crate::http::UrlScheme::Https,
            api_base: crate::http::Url::<crate::http::RelayHost>::new("api.test".to_owned()),
            invitee_email: Some("invitee@test.local".into()),
            invitee: Id::<User>::parse_str(INVITEE).expect("an id"),
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
        let my_pub = my_pub_hex;
        let (invitee_xk, invitee_pub_hex) = keypair(2);
        let invitee_pub = invitee_pub_hex;

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

        // Success event, no error. It carries the pin of the key that signs
        // the inviter's entries, for the invite link (`SYNC-208`).
        match erx.try_recv().expect("InviteSent emitted") {
            SyncEvent::InviteSent {
                display_name,
                inviter,
                ..
            } => {
                assert_eq!(display_name.as_str(), "Invitee");
                assert_eq!(inviter, ambre_crypto::inviter_pin(&author().public()));
            }
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
                let enc =
                    &serde_json::from_value::<ambre_core::Encoded<ambre_core::WrappedDocKey>>(
                        body["doc_key_enc"].clone(),
                    )
                    .unwrap();
                assert!(unwrap_b64(&invitee_xk, &my_pub, enc).is_ok());
                assert!(body.get("entry").is_none());
            }
            other => panic!("expected share POST, got {other:?}"),
        }
    }

    // --- SYNC-172 ---

    /// An invitee known by id alone has its keys fetched by that id, and
    /// the share names the id, never an email.
    #[tokio::test]
    async fn an_invite_by_id_fetches_the_keys_of_that_id_and_shares_by_it() {
        let (my_xk, my_pub_hex) = keypair(1);
        let my_pub = my_pub_hex;
        let (invitee_xk, invitee_pub_hex) = keypair(2);
        let invitee_pub = invitee_pub_hex;
        let http = FakeHttp::new(vec![
            Ok(serde_json::json!({
                "signing_pubkey": "00".repeat(32),
                "exchange_pubkey": invitee_pub_hex,
            })),
            Ok(serde_json::json!({ "members": [] })),
            Ok(serde_json::json!({ "user_id": INVITEE, "display_name": "Invitee" })),
        ]);
        let (etx, erx, sink, lrx) = channels();
        let params = InviteParams {
            invitee_email: None,
            ..invite_params(&my_xk, invitee_pub)
        };
        invite_task(&http, params, &etx, &sink).await;

        assert!(matches!(erx.try_recv(), Ok(SyncEvent::InviteSent { .. })));
        assert!(lrx.try_recv().is_err(), "no ApiError on success");
        let calls = http.calls();
        assert!(
            matches!(&calls[0], Call::Get(url) if url.ends_with(&format!("/users/{INVITEE}/pubkeys"))),
            "the keys of the id are fetched, and no search runs: {calls:?}"
        );
        match &calls[2] {
            Call::Post(url, body) => {
                assert!(url.ends_with("/share"));
                assert_eq!(body["invitee_id"], INVITEE);
                assert!(body.get("invitee_email").is_none());
                let enc =
                    &serde_json::from_value::<ambre_core::Encoded<ambre_core::WrappedDocKey>>(
                        body["doc_key_enc"].clone(),
                    )
                    .unwrap();
                assert!(unwrap_b64(&invitee_xk, &my_pub, enc).is_ok());
            }
            other => panic!("expected share POST, got {other:?}"),
        }
    }

    /// A key served by id that differs from the pin refuses the invite
    /// before any wrap, as the email path does.
    #[tokio::test]
    async fn an_invite_by_id_refuses_a_key_other_than_the_pin() {
        let (my_xk, _) = keypair(1);
        let (_, confirmed_hex) = keypair(2);
        let confirmed = confirmed_hex;
        let (_, substituted_hex) = keypair(9);
        let http = FakeHttp::new(vec![Ok(serde_json::json!({
            "signing_pubkey": "00".repeat(32),
            "exchange_pubkey": substituted_hex,
        }))]);
        let (etx, erx, sink, lrx) = channels();
        let params = InviteParams {
            invitee_email: None,
            ..invite_params(&my_xk, confirmed)
        };
        invite_task(&http, params, &etx, &sink).await;

        assert!(matches!(
            lrx.try_recv(),
            Ok(LogicEvent::ApiError {
                failure: ApiFailure::Local(LocalFailure::PinMismatch(_)),
                ..
            })
        ));
        assert!(erx.try_recv().is_err(), "no InviteSent on mismatch");
        assert_eq!(http.calls().len(), 1, "only the key fetch ran");
    }

    #[tokio::test]
    async fn invite_user_not_found_emits_404_and_skips_share() {
        let (my_xk, _) = keypair(1);
        let http = FakeHttp::new(vec![Ok(serde_json::json!({ "users": [] }))]);
        let (etx, erx, sink, lrx) = channels();
        invite_task(
            &http,
            invite_params(&my_xk, FixedBytes::from_array([0u8; 32])),
            &etx,
            &sink,
        )
        .await;

        match lrx.try_recv().expect("ApiError emitted") {
            LogicEvent::ApiError { failure, .. } => {
                assert_eq!(failure, ApiFailure::Local(LocalFailure::UserNotFound));
            }
            other => panic!("unexpected: {other:?}"),
        }
        assert!(erx.try_recv().is_err(), "no success event");
        // Only the search ran; no share POST attempted.
        assert_eq!(http.calls().len(), 1);
    }

    #[tokio::test]
    async fn invite_search_failure_emits_500() {
        let (my_xk, _) = keypair(1);
        let http = FakeHttp::new(vec![Err(http::HttpError::Local(
            ambre_core::LocalFailure::NoRelay,
        ))]);
        let (etx, _erx, sink, lrx) = channels();
        invite_task(
            &http,
            invite_params(&my_xk, FixedBytes::from_array([0u8; 32])),
            &etx,
            &sink,
        )
        .await;

        match lrx.try_recv().expect("ApiError emitted") {
            LogicEvent::ApiError { failure, .. } => {
                assert_eq!(failure, ApiFailure::Local(LocalFailure::NoRelay))
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
        let confirmed = confirmed_hex;
        let (_substituted_xk, substituted_hex) = keypair(9);

        let http = FakeHttp::new(vec![Ok(user_search_json(
            "invitee@test.local",
            &substituted_hex, // server returns the substituted key
        ))]);
        let (etx, erx, sink, lrx) = channels();
        // We confirmed `confirmed`, not the substituted key the server now serves.
        invite_task(&http, invite_params(&my_xk, confirmed), &etx, &sink).await;

        match lrx.try_recv().expect("ApiError emitted") {
            LogicEvent::ApiError { failure, .. } => {
                assert!(matches!(
                    failure,
                    ApiFailure::Local(LocalFailure::PinMismatch(_))
                ));
            }
            other => panic!("unexpected: {other:?}"),
        }
        assert!(erx.try_recv().is_err(), "no InviteSent on mismatch");
        // Only the search ran: nothing was wrapped, no share POST fired.
        assert_eq!(http.calls().len(), 1);
    }

    #[tokio::test]
    async fn an_email_naming_another_user_refuses_the_invite() {
        // The relay answers the email with the confirmed key, but under
        // another user than the one phase 1 answered.
        let (my_xk, _) = keypair(1);
        let (_invitee_xk, invitee_hex) = keypair(2);
        let invitee_pub = invitee_hex;
        let http = FakeHttp::new(vec![Ok(user_search_json(
            "invitee@test.local",
            &invitee_hex,
        ))]);
        let (etx, erx, sink, lrx) = channels();
        let mut params = invite_params(&my_xk, invitee_pub);
        params.invitee = Id::<User>::mint();
        invite_task(&http, params, &etx, &sink).await;

        match lrx.try_recv().expect("ApiError emitted") {
            LogicEvent::ApiError { failure, .. } => {
                assert_eq!(failure, ApiFailure::Local(LocalFailure::InviteeChanged));
            }
            other => panic!("unexpected: {other:?}"),
        }
        assert!(erx.try_recv().is_err(), "no InviteSent for another user");
        assert_eq!(http.calls().len(), 1, "nothing wrapped, no share posted");
    }

    fn fetch_key_params() -> FetchInviteeKeyParams {
        FetchInviteeKeyParams {
            id: ambre_bus::next_request(),
            document_id: Id::<Document>::mint(),
            url_scheme: crate::http::UrlScheme::Https,
            api_base: crate::http::Url::<crate::http::RelayHost>::new("api.test".to_owned()),
            invitee_email: "invitee@test.local".into(),
        }
    }

    #[tokio::test]
    async fn fetch_invitee_key_emits_key_and_matching_fingerprint() {
        let (_invitee_xk, invitee_pub_hex) = keypair(2);
        let invitee_pub = invitee_pub_hex;

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
                assert_eq!(email, ambre_core::Email::from("invitee@test.local"));
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

    fn member_key_params(user_id: Id<User>) -> FetchMemberKeyParams {
        FetchMemberKeyParams {
            id: ambre_bus::next_request(),
            user_id,
            url_scheme: crate::http::UrlScheme::Https,
            api_base: crate::http::Url::<crate::http::RelayHost>::new("api.test".to_owned()),
        }
    }

    /// A member's key is fetched by the id the client names, and answered
    /// under that id with its fingerprint. One GET, and nothing is wrapped.
    #[tokio::test]
    async fn a_member_key_is_fetched_by_user_id_and_wraps_nothing() {
        let (_member_xk, member_pub_hex) = keypair(3);
        let member_pub = member_pub_hex;
        let user_id = Id::<User>::mint();
        let http = FakeHttp::new(vec![Ok(serde_json::json!({
            "signing_pubkey": "00".repeat(32),
            "exchange_pubkey": member_pub_hex,
        }))]);
        let (etx, erx, sink, lrx) = channels();
        fetch_member_key_task(&http, member_key_params(user_id), &etx, &sink).await;

        match erx.try_recv().expect("MemberKeyReady emitted") {
            SyncEvent::MemberKeyReady {
                user_id: answered,
                exchange_pub_key,
                fingerprint,
                ..
            } => {
                assert_eq!(answered, user_id);
                assert_eq!(exchange_pub_key, member_pub);
                assert_eq!(fingerprint, ambre_crypto::fingerprint(&member_pub));
            }
            _ => panic!("unexpected event (want MemberKeyReady)"),
        }
        assert!(lrx.try_recv().is_err(), "no ApiError on success");
        let calls = http.calls();
        assert_eq!(calls.len(), 1);
        assert!(
            matches!(&calls[0], Call::Get(url) if url == &format!("https://api.test/api/users/{user_id}/pubkeys"))
        );
    }

    /// A refused fetch or a malformed key answers an error, and no key.
    #[tokio::test]
    async fn a_member_key_that_fails_or_is_malformed_answers_an_error() {
        let malformed = serde_json::json!({
            "signing_pubkey": "00".repeat(32),
            "exchange_pubkey": "ab".repeat(31),
        });
        for answer in [
            Err(http::HttpError::Local(ambre_core::LocalFailure::NoRelay)),
            Ok(malformed),
        ] {
            let http = FakeHttp::new(vec![answer]);
            let (etx, erx, sink, lrx) = channels();
            fetch_member_key_task(&http, member_key_params(Id::<User>::mint()), &etx, &sink).await;
            assert!(erx.try_recv().is_err(), "no key answered");
            assert!(matches!(
                lrx.try_recv(),
                Ok(LogicEvent::ApiError {
                    failure: ApiFailure::Local(_),
                    ..
                })
            ));
        }
    }

    #[tokio::test]
    async fn fetch_invitee_key_unknown_user_emits_404() {
        let http = FakeHttp::new(vec![Ok(serde_json::json!({ "users": [] }))]);
        let (etx, erx, sink, lrx) = channels();
        fetch_invitee_key_task(&http, fetch_key_params(), &etx, &sink).await;

        match lrx.try_recv().expect("ApiError emitted") {
            LogicEvent::ApiError { failure, .. } => {
                assert_eq!(failure, ApiFailure::Local(LocalFailure::UserNotFound));
            }
            other => panic!("unexpected: {other:?}"),
        }
        assert!(erx.try_recv().is_err(), "no key event for unknown user");
    }

    fn revoke_params(my_xk: &StaticSecret) -> RevokeParams {
        RevokeParams {
            id: ambre_bus::next_request(),
            document_id: Id::<Document>::mint(),
            user_id: Id::<User>::mint(),
            url_scheme: crate::http::UrlScheme::Https,
            api_base: crate::http::Url::<crate::http::RelayHost>::new("api.test".to_owned()),
            exchange_key: my_xk.clone(),
            previous: DocumentKey::generate(),
            author: author(),
        }
    }

    /// The link a request carried, read back as the relay would.
    fn posted_link(body: &serde_json::Value) -> ambre_protocol::KeyLink {
        let link: ambre_core::Encoded<ambre_core::EncodedKeyLink> =
            serde_json::from_value(body["link"].clone()).expect("the request carries a link");
        ambre_protocol::KeyLink::decode(&link).unwrap()
    }

    /// A members row, for the revoke and the debt tests.
    fn member_row(
        uid: &str,
        name: &str,
        pub_hex: &FixedBytes<ambre_core::ExchangePublicKey>,
    ) -> serde_json::Value {
        serde_json::json!({
            "user_id": uid,
            "display_name": name, "email": "m@test.local",
            "signing_pub_key": "00".repeat(32), "exchange_pub_key": pub_hex,
            "doc_key_enc": ambre_core::Encoded::<ambre_core::WrappedDocKey>::new(vec![0; 72]), "role": "editor",
            "wrapper_exchange_pub_key": pub_hex,
        })
    }

    /// The log `owner` creates for `document_id`. It invites each of
    /// `invited` as an editor, under the exchange key given beside it. The
    /// answer is the entries in request form, and the row of `owner`, whose
    /// copy seals the anchor.
    fn log_inviting(
        document_id: Id<Document>,
        owner: &Author,
        owner_xk: &StaticSecret,
        invited: &[(&str, &FixedBytes<ambre_core::ExchangePublicKey>)],
    ) -> (
        Vec<ambre_core::Encoded<ambre_core::EncodedLogEntry>>,
        serde_json::Value,
    ) {
        let (enc, first) =
            creation(document_id, &DocumentKey::generate(), owner_xk, owner).unwrap();
        let mut entries = membership_log::decode(std::slice::from_ref(&first));
        for (uid, exchange) in invited {
            let next = LogHead {
                position: ambre_protocol::place_after(entries.len()),
                previous: entries.last().and_then(membership_log::digest),
            };
            let draft = Draft {
                document: document_id,
                member: Id::<User>::parse_str(uid).unwrap(),
                member_key: FixedBytes::from_array([0; 32]),
                member_exchange_key: **exchange,
                act: MembershipAct::Invite,
                role: Role::Editor,
                key: None,
            };
            entries.push(
                membership_log::sign(draft, next, owner.user_id, &owner.signing_key).unwrap(),
            );
        }
        let own_pub = own_exchange_key(owner_xk);
        let mut row = member_row(&owner.user_id.to_string(), "Me", &own_pub);
        row["doc_key_enc"] = serde_json::json!(enc);
        let texts = entries
            .iter()
            .map(|entry| membership_log::encode(entry).unwrap())
            .collect();
        (texts, row)
    }

    /// Holds `SYNC-199` at the revoke. A member row the relay serves with no
    /// entry of the log behind it receives no copy of the next key. The user
    /// who revoked is told its name.
    #[tokio::test]
    async fn a_row_the_relay_injects_receives_no_key_at_a_revoke() {
        let (my_xk, my_pub) = keypair(1);
        let (_a_xk, a_pub) = keypair(2);
        let (_c_xk, c_pub) = keypair(3);
        let (relay_xk, relay_pub) = keypair(4);
        let params = revoke_params(&my_xk);
        let removed = params.user_id.to_string();
        let a = "11111111-1111-1111-1111-111111111111";
        let (log, own) = log_inviting(
            params.document_id,
            &params.author,
            &my_xk,
            &[(a, &a_pub), (&removed, &c_pub)],
        );
        let http = FakeHttp::new(vec![
            Ok(serde_json::json!({ "members": [
                own,
                member_row(a, "A", &a_pub),
                member_row(&removed, "Carol", &c_pub),
                member_row("66666666-6666-6666-6666-666666666666", "Mallory", &relay_pub),
            ], "log": log })),
            Ok(serde_json::json!({ "status": "revoked" })),
        ]);
        let me = params.author.user_id.to_string();
        let (etx, erx, sink, lrx) = channels();
        revoke_task(&http, params, &etx, &sink).await;
        assert!(matches!(
            erx.try_recv(),
            Ok(SyncEvent::RevocationComplete { .. })
        ));
        let Ok(LogicEvent::KeyNotShared { members, .. }) = lrx.try_recv() else {
            panic!("the injected row is reported");
        };
        assert_eq!(
            members.iter().map(|m| m.as_str()).collect::<Vec<_>>(),
            ["Mallory"]
        );
        let calls = http.calls();
        let Call::Post(_, body) = &calls[1] else {
            panic!("expected the revoke POST: {calls:?}");
        };
        let keys = body["keys"].as_array().unwrap();
        let wrapped: Vec<_> = keys
            .iter()
            .map(|k| k["user_id"].as_str().unwrap())
            .collect();
        assert_eq!(wrapped, [me.as_str(), a]);
        // No copy the relay holds opens under the relay's own key.
        for key in keys {
            let enc: ambre_core::Encoded<ambre_core::WrappedDocKey> =
                serde_json::from_value(key["doc_key_enc"].clone()).unwrap();
            assert!(unwrap_b64(&relay_xk, &my_pub, &enc).is_err());
        }
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
        let me = params.author.user_id.to_string();
        let a = "11111111-1111-1111-1111-111111111111";
        let (log, own) = log_inviting(
            document_id,
            &params.author,
            &my_xk,
            &[(a, &a_pub), (&removed, &c_pub)],
        );

        let http = FakeHttp::new(vec![
            Ok(serde_json::json!({ "members": [
                own,
                member_row(a, "A", &a_pub),
                member_row(&removed, "Carol", &c_pub),
            ], "log": log })), // GET members
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
                let wrapped: Vec<_> = keys
                    .iter()
                    .map(|k| k["user_id"].as_str().unwrap())
                    .collect();
                assert_eq!(wrapped, [me.as_str(), a], "no copy for the member removed");
                assert_eq!(posted_entry(body).act, MembershipAct::Revoke);
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
        let me = params.author.user_id.to_string();
        let (log, own) = log_inviting(
            params.document_id,
            &params.author,
            &my_xk,
            &[(a, &a_pub), (&removed, &c_pub)],
        );
        let (log_after, own_after) = log_inviting(
            params.document_id,
            &params.author,
            &my_xk,
            &[(a, &a_pub), (&removed, &c_pub), (newcomer, &n_pub)],
        );

        let http = FakeHttp::new(vec![
            Ok(serde_json::json!({ "members": [
                own, member_row(a, "A", &a_pub), member_row(&removed, "Carol", &c_pub),
            ], "log": log })),
            Err(http::HttpError::refused(
                http::Method::Post,
                &crate::http::Target::relay("https://api.test/revoke"),
                ambre_core::HttpStatus::new(409),
                Some(RefusalCode::MembersChanged),
            )),
            Ok(serde_json::json!({ "members": [
                own_after, member_row(a, "A", &a_pub), member_row(&removed, "Carol", &c_pub),
                member_row(newcomer, "New", &n_pub),
            ], "log": log_after })),
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
        assert_eq!(first["members"], serde_json::json!([me, a]));
        assert_eq!(second["members"], serde_json::json!([me, a, newcomer]));
        let wrapped: Vec<_> = second["keys"]
            .as_array()
            .unwrap()
            .iter()
            .map(|k| k["user_id"].as_str().unwrap())
            .collect();
        assert_eq!(
            wrapped,
            vec![me.as_str(), a, newcomer],
            "the newcomer gets a copy"
        );
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
            script.push(Err(http::HttpError::refused(
                http::Method::Post,
                &crate::http::Target::relay("https://api.test/revoke"),
                ambre_core::HttpStatus::new(409),
                Some(RefusalCode::MembersChanged),
            )));
        }
        let http = FakeHttp::new(script);
        let (etx, erx, sink, lrx) = channels();
        revoke_task(&http, revoke_params(&my_xk), &etx, &sink).await;
        assert!(erx.try_recv().is_err(), "no key swapped");
        assert!(matches!(
            lrx.try_recv(),
            Ok(LogicEvent::ApiError {
                failure: ApiFailure::Relay {
                    status,
                    code: Some(RefusalCode::MembersChanged),
                },
                ..
            }) if status.get() == 409
        ));
        assert_eq!(http.calls().len(), 2 * ROTATION_TRIES);
    }

    /// `SYNC-143`: an owed rotation refused for a newcomer reads the members
    /// again, and the payment wraps for the newcomer too.
    #[tokio::test]
    async fn an_owed_rotation_refused_for_a_newcomer_is_paid_for_them() {
        let (my_xk, _) = keypair(1);
        let (_n_xk, n_pub) = keypair(4);
        let me = "11111111-1111-1111-1111-111111111111";
        let newcomer = "44444444-4444-4444-4444-444444444444";
        let params = settle_params(&my_xk);
        let owner = Author {
            user_id: Id::<User>::parse_str(me).unwrap(),
            signing_key: params.signing_key.clone(),
        };
        let (log, own) = log_inviting(params.document_id, &owner, &my_xk, &[]);
        let (log_after, own_after) =
            log_inviting(params.document_id, &owner, &my_xk, &[(newcomer, &n_pub)]);
        let http = FakeHttp::new(vec![
            Ok(serde_json::json!({ "members": [ own ],
                "rotation_owed": true, "log": log })),
            Err(http::HttpError::refused(
                http::Method::Post,
                &crate::http::Target::relay("https://api.test/rotate-key"),
                ambre_core::HttpStatus::new(409),
                Some(RefusalCode::MembersChanged),
            )),
            Ok(serde_json::json!({ "members": [
                own_after, member_row(newcomer, "New", &n_pub),
            ], "rotation_owed": true, "log": log_after })),
            Ok(serde_json::json!({ "status": "rotated" })),
        ]);
        let (etx, erx, _sink, _lrx) = channels();
        settle_task(&http, params, &etx).await;
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
            InviteeName::Email("i@test".into()),
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
        let expect = i_pub;
        let http = FakeHttp::new(vec![
            Ok(user_search_json("invitee@test.local", &i_pub)),
            Ok(serde_json::json!({ "members": [] })),
            Err(http::HttpError::refused(
                http::Method::Post,
                &crate::http::Target::relay("https://api.test/share"),
                ambre_core::HttpStatus::new(409),
                Some(RefusalCode::KeyChanged),
            )),
        ]);
        let (etx, erx, sink, lrx) = channels();
        invite_task(&http, invite_params(&my_xk, expect), &etx, &sink).await;
        assert!(erx.try_recv().is_err());
        assert!(matches!(
            lrx.try_recv(),
            Ok(LogicEvent::ApiError {
                failure: ApiFailure::Relay {
                    status,
                    code: Some(RefusalCode::KeyChanged),
                },
                ..
            }) if status.get() == 409
        ));

        // Another refusal is no rotation: it says so in its own words.
        let http = FakeHttp::new(vec![
            Ok(user_search_json("invitee@test.local", &i_pub)),
            Ok(serde_json::json!({ "members": [] })),
            Err(http::HttpError::refused(
                http::Method::Post,
                &crate::http::Target::relay("https://api.test/share"),
                ambre_core::HttpStatus::new(403),
                None,
            )),
        ]);
        let (etx, _erx, sink, lrx) = channels();
        invite_task(&http, invite_params(&my_xk, expect), &etx, &sink).await;
        assert!(matches!(
            lrx.try_recv(),
            Ok(LogicEvent::ApiError {
                failure: ApiFailure::Relay { code: None, .. },
                ..
            })
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
        let channel_id = Id::<Channel>::mint();
        let listing = serde_json::json!({ "documents": [
            { "document_id": Id::<Document>::mint().to_string(), "kind": "segment" },
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
            url_scheme: crate::http::UrlScheme::Https,
            api_base: crate::http::Url::<crate::http::RelayHost>::new("api.test".to_owned()),
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
            channel_id: Id::<Channel>::mint(),
            author: author(),
            url_scheme: crate::http::UrlScheme::Https,
            api_base: crate::http::Url::<crate::http::RelayHost>::new("api.test".to_owned()),
            exchange_key: keypair(1).0,
        };
        let http = FakeHttp::new(vec![Err(http::HttpError::Local(
            ambre_core::LocalFailure::NoRelay,
        ))]);
        assert_eq!(
            leave_channel_task(&http, params()).await,
            Err(http::HttpError::Local(ambre_core::LocalFailure::NoRelay))
        );

        let no_meta = serde_json::json!({ "documents": [
            { "document_id": Id::<Document>::mint().to_string(), "kind": "segment" },
        ]});
        let http = FakeHttp::new(vec![Ok(no_meta)]);
        assert!(leave_channel_task(&http, params()).await.is_err());
        assert_eq!(http.calls().len(), 1, "nothing is posted");

        let listing = serde_json::json!({ "documents": [
            { "document_id": Id::<Document>::mint().to_string(), "kind": "meta" },
        ]});
        let http = FakeHttp::new(vec![
            Ok(listing),
            Ok(serde_json::json!({ "members": [] })),
            Err(http::HttpError::Local(ambre_core::LocalFailure::NoRelay)),
        ]);
        assert!(leave_channel_task(&http, params()).await.is_err());
        assert_eq!(http.calls().len(), 3, "a refusal is not tried again");
    }

    fn close_params(successor: Id<Segment>) -> SegmentCloseParams {
        SegmentCloseParams {
            channel_id: Id::<Channel>::mint(),
            closing: Id::<Segment>::mint(),
            successor,
            meta_key: DocumentKey::generate(),
            author: author(),
            url_scheme: crate::http::UrlScheme::Https,
            api_base: crate::http::Url::<crate::http::RelayHost>::new("api.test".to_owned()),
            exchange_key: keypair(1).0,
        }
    }

    /// Holds `SYNC-163`. A close creates its successor, then asks the relay.
    /// A winner deletes nothing; a loser deletes the successor it created,
    /// and both learn the active segment.
    #[tokio::test]
    async fn a_lost_close_deletes_the_successor_it_created() {
        let created = serde_json::json!({
            "document_id": Id::<Document>::mint().to_string(), "doc_type": "murmure",
        });
        let mine = Id::<Segment>::mint();
        let answer = |active: Id<Segment>, created: bool| serde_json::json!({ "active": active.to_string(), "created": created });
        let http = FakeHttp::new(vec![Ok(created.clone()), Ok(answer(mine, true))]);
        let won = close_segment_task(&http, close_params(mine)).await;
        assert_eq!(
            won,
            Ok(http::ClosedSegmentResponse {
                active: mine,
                created: ambre_core::Creation::Created
            })
        );
        {
            let calls = http.calls();
            let [Call::Post(create, body), Call::Post(close, _)] = calls.as_slice() else {
                panic!("expected a create then a close, got {:?}", calls.len());
            };
            assert!(create.ends_with("/api/documents"));
            assert_eq!(body["document_id"], mine.document_id().to_string());
            assert!(close.ends_with("/segments"));
        }

        let winner = Id::<Segment>::mint();
        let http = FakeHttp::new(vec![
            Ok(created.clone()),
            Ok(answer(winner, false)),
            Ok(serde_json::Value::Null),
        ]);
        let lost = close_segment_task(&http, close_params(mine)).await;
        assert_eq!(
            lost,
            Ok(http::ClosedSegmentResponse {
                active: winner,
                created: ambre_core::Creation::Existing
            })
        );
        {
            let calls = http.calls();
            let Some(Call::Delete(url, _)) = calls.last() else {
                panic!("the loser deleted nothing");
            };
            assert!(url.ends_with(&format!("/documents/{}", mine.document_id())));
        }

        let http = FakeHttp::new(vec![
            Ok(created),
            Err(http::HttpError::Local(ambre_core::LocalFailure::NoRelay)),
        ]);
        let failed = close_segment_task(&http, close_params(mine)).await;
        assert_eq!(
            failed,
            Err(http::HttpError::Local(ambre_core::LocalFailure::NoRelay))
        );
        assert_eq!(http.calls().len(), 2, "a failed close deletes nothing");
    }

    /// `SYNC-164`: a close asked again names the successor it created. The
    /// relay's answer reads as this client's own close, so nothing is
    /// deleted.
    #[tokio::test]
    async fn a_close_asked_again_keeps_its_successor() {
        let mine = Id::<Segment>::mint();
        let created = serde_json::json!({
            "document_id": mine.document_id().to_string(), "doc_type": "murmure",
        });
        let answer = serde_json::json!({ "active": mine.to_string(), "created": true });
        let http = FakeHttp::new(vec![Ok(created), Ok(answer)]);
        let again = close_segment_task(&http, close_params(mine)).await;
        assert_eq!(
            again,
            Ok(http::ClosedSegmentResponse {
                active: mine,
                created: ambre_core::Creation::Created
            })
        );
        let calls = http.calls();
        let [Call::Post(_, create), Call::Post(_, close)] = calls.as_slice() else {
            panic!("expected a create then a close, got {:?}", calls.len());
        };
        assert_eq!(create["document_id"], mine.document_id().to_string());
        assert_eq!(close["successor"], mine.to_string());
    }

    fn amendments_params(amendments: Id<Document>) -> AmendmentsParams {
        AmendmentsParams {
            channel_id: Id::<Channel>::mint(),
            segment: Id::<Segment>::mint(),
            amendments,
            meta_key: DocumentKey::generate(),
            author: author(),
            url_scheme: crate::http::UrlScheme::Https,
            api_base: crate::http::Url::<crate::http::RelayHost>::new("api.test".to_owned()),
            exchange_key: keypair(1).0,
        }
    }

    /// Holds `SYNC-166`. The amendments document is created, then
    /// registered. A loser deletes its own and learns the winner's; a failed
    /// registration deletes nothing.
    #[tokio::test]
    async fn a_lost_registration_deletes_the_amendments_it_created() {
        let mine = Id::<Document>::mint();
        let created = serde_json::json!({
            "document_id": mine.to_string(), "doc_type": "murmure",
        });
        let answer = |held: Id<Document>, created: bool| serde_json::json!({ "amendments": held.to_string(), "created": created });
        let http = FakeHttp::new(vec![Ok(created.clone()), Ok(answer(mine, true))]);
        assert_eq!(
            amendments_task(&http, amendments_params(mine)).await,
            Ok(mine)
        );
        {
            let calls = http.calls();
            let [Call::Post(create, body), Call::Post(link, linked)] = calls.as_slice() else {
                panic!(
                    "expected a create then a registration, got {:?}",
                    calls.len()
                );
            };
            assert!(create.ends_with("/api/documents"));
            assert_eq!(body["document_id"], mine.to_string());
            assert!(link.ends_with("/amendments"));
            assert_eq!(linked["amendments"], mine.to_string());
        }

        let winner = Id::<Document>::mint();
        let http = FakeHttp::new(vec![
            Ok(created.clone()),
            Ok(answer(winner, false)),
            Ok(serde_json::Value::Null),
        ]);
        assert_eq!(
            amendments_task(&http, amendments_params(mine)).await,
            Ok(winner)
        );
        {
            let calls = http.calls();
            let Some(Call::Delete(url, _)) = calls.last() else {
                panic!("the loser deleted nothing");
            };
            assert!(url.ends_with(&format!("/documents/{mine}")));
        }

        let http = FakeHttp::new(vec![
            Ok(created),
            Err(http::HttpError::Local(ambre_core::LocalFailure::NoRelay)),
        ]);
        let failed = amendments_task(&http, amendments_params(mine)).await;
        assert_eq!(
            failed,
            Err(http::HttpError::Local(ambre_core::LocalFailure::NoRelay))
        );
        assert_eq!(
            http.calls().len(),
            2,
            "a failed registration deletes nothing"
        );
    }

    fn thread_params(thread: Id<Document>) -> ThreadParams {
        ThreadParams {
            channel_id: Id::<Channel>::mint(),
            root: Id::<Message>::mint(),
            thread,
            meta_key: DocumentKey::generate(),
            author: author(),
            url_scheme: crate::http::UrlScheme::Https,
            api_base: crate::http::Url::<crate::http::RelayHost>::new("api.test".to_owned()),
            exchange_key: keypair(1).0,
        }
    }

    /// Holds `SYNC-169`. The thread document is created, then registered
    /// for its root. A loser deletes its own and learns the winner's; a
    /// failed registration deletes nothing.
    #[tokio::test]
    async fn a_lost_registration_deletes_the_thread_it_created() {
        let mine = Id::<Document>::mint();
        let params = thread_params(mine);
        let root = params.root;
        let created = serde_json::json!({
            "document_id": mine.to_string(), "doc_type": "murmure",
        });
        let answer = |held: Id<Document>, created: bool| serde_json::json!({ "thread": held.to_string(), "created": created });
        let http = FakeHttp::new(vec![Ok(created.clone()), Ok(answer(mine, true))]);
        assert_eq!(thread_task(&http, params).await, Ok(mine));
        {
            let calls = http.calls();
            let [Call::Post(create, body), Call::Post(link, linked)] = calls.as_slice() else {
                panic!("expected a create then a registration");
            };
            assert!(create.ends_with("/api/documents"));
            assert_eq!(body["document_id"], mine.to_string());
            assert!(link.ends_with("/threads"));
            assert_eq!(linked["thread"], mine.to_string());
            assert_eq!(linked["root"], root.to_string());
        }

        let winner = Id::<Document>::mint();
        let http = FakeHttp::new(vec![
            Ok(created.clone()),
            Ok(answer(winner, false)),
            Ok(serde_json::Value::Null),
        ]);
        assert_eq!(thread_task(&http, thread_params(mine)).await, Ok(winner));
        {
            let calls = http.calls();
            let Some(Call::Delete(url, _)) = calls.last() else {
                panic!("the loser deleted nothing");
            };
            assert!(url.ends_with(&format!("/documents/{mine}")));
        }

        let http = FakeHttp::new(vec![
            Ok(created),
            Err(http::HttpError::Local(ambre_core::LocalFailure::NoRelay)),
        ]);
        let failed = thread_task(&http, thread_params(mine)).await;
        assert_eq!(
            failed,
            Err(http::HttpError::Local(ambre_core::LocalFailure::NoRelay))
        );
        assert_eq!(
            http.calls().len(),
            2,
            "a failed registration deletes nothing"
        );
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
            Err(http::HttpError::Local(ambre_core::LocalFailure::NoRelay)),
        ]);
        let (etx, erx, sink, lrx) = channels();
        revoke_task(&http, revoke_params(&my_xk), &etx, &sink).await;
        assert!(erx.try_recv().is_err(), "no key to install");
        assert!(matches!(
            lrx.try_recv().expect("ApiError"),
            LogicEvent::ApiError {
                failure: ApiFailure::Local(_),
                ..
            }
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
            id: ambre_bus::next_request(),
            document_id: Id::<Document>::mint(),
            author,
            url_scheme: crate::http::UrlScheme::Https,
            api_base: crate::http::Url::<crate::http::RelayHost>::new("api.test".to_owned()),
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

    /// A leave the relay refuses reaches the Logic thread as the relay's
    /// refusal, its code kept. A setup still owed then reads as the setup,
    /// never as a failure on this device.
    #[tokio::test]
    async fn a_refused_leave_keeps_the_relay_refusal() {
        let refusal = || {
            http::HttpError::refused(
                http::Method::Delete,
                &crate::http::Target::relay("https://api.test/share"),
                ambre_core::HttpStatus::new(403),
                Some(RefusalCode::SetupRequired),
            )
        };
        let http = FakeHttp::new(vec![
            Ok(serde_json::json!({ "members": [] })),
            Err(refusal()),
        ]);
        let (etx, _erx, sink, lrx) = channels();
        let params = LeaveParams {
            id: ambre_bus::next_request(),
            document_id: Id::<Document>::mint(),
            author: author(),
            url_scheme: crate::http::UrlScheme::Https,
            api_base: crate::http::Url::<crate::http::RelayHost>::new("api.test".to_owned()),
            exchange_key: keypair(1).0,
        };
        leave_task(&http, params, &etx, &sink).await;
        assert!(matches!(
            lrx.try_recv().expect("the refusal"),
            LogicEvent::SetupRequired
        ));

        let listing = serde_json::json!({ "documents": [
            { "document_id": Id::<Document>::mint().to_string(), "kind": "meta" },
        ]});
        let http = FakeHttp::new(vec![
            Ok(listing),
            Ok(serde_json::json!({ "members": [] })),
            Err(refusal()),
        ]);
        let params = ChannelLeaveParams {
            channel_id: Id::<Channel>::mint(),
            author: author(),
            url_scheme: crate::http::UrlScheme::Https,
            api_base: crate::http::Url::<crate::http::RelayHost>::new("api.test".to_owned()),
            exchange_key: keypair(1).0,
        };
        let left = leave_channel_task(&http, params).await;
        assert!(left.is_err_and(|e| e.is(RefusalCode::SetupRequired)));
    }

    fn settle_params(my_xk: &StaticSecret) -> SettleParams {
        SettleParams {
            document_id: Id::<Document>::mint(),
            url_scheme: crate::http::UrlScheme::Https,
            api_base: crate::http::Url::<crate::http::RelayHost>::new("api.test".to_owned()),
            exchange_key: my_xk.clone(),
            previous: DocumentKey::generate(),
            signing_key: MasterSeed::from_array([9; 32]).derive_signing_key(),
        }
    }

    /// An owed rotation is paid with a fresh key wrapped for every member. The
    /// key reaches the caller only once the relay stored it (`SYNC-128`).
    #[tokio::test]
    async fn an_owed_rotation_is_paid_for_every_member() {
        let (my_xk, _) = keypair(1);
        let (_a_xk, a_pub) = keypair(2);
        let params = settle_params(&my_xk);
        let owner = Author {
            user_id: Id::<User>::mint(),
            signing_key: params.signing_key.clone(),
        };
        let a = "22222222-2222-2222-2222-222222222222";
        let (log, own) = log_inviting(params.document_id, &owner, &my_xk, &[(a, &a_pub)]);
        let http = FakeHttp::new(vec![
            Ok(serde_json::json!({ "rotation_owed": true, "members": [
                own,
                member_row(a, "A", &a_pub),
            ], "log": log })),
            Ok(serde_json::json!({ "status": "rotated" })),
        ]);
        let (etx, erx, _sink, _lrx) = channels();
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
        let (space_id, channel_id) = (Id::<Space>::mint(), Id::<Channel>::mint());
        let meta = Id::<Meta>::mint();
        let head = serde_json::json!({
            "meta_id": meta.to_string(), "role": "editor", "length": 1,
        });
        // Meta's log, which another member created, signs A's key.
        let a = "22222222-2222-2222-2222-222222222222";
        let creator = Author {
            user_id: Id::<User>::mint(),
            signing_key: MasterSeed::from_array([8; 32]).derive_signing_key(),
        };
        let (log, _) = log_inviting(meta.document_id(), &creator, &keypair(8).0, &[(a, &a_pub)]);
        let anchor = membership_log::decode(&log[..1])
            .first()
            .and_then(membership_log::digest)
            .unwrap();
        let one = serde_json::json!({ "members": [
            member_row("11111111-1111-1111-1111-111111111111", "Me", &my_pub),
        ], "log": log });
        let two = serde_json::json!({ "members": [
            member_row("11111111-1111-1111-1111-111111111111", "Me", &my_pub),
            member_row(a, "A", &a_pub),
        ], "log": log });
        let changed = http::HttpError::refused(
            http::Method::Post,
            &crate::http::Target::relay("x"),
            ambre_core::HttpStatus::new(409),
            Some(RefusalCode::MembersChanged),
        );
        let http = FakeHttp::new(vec![
            Ok(head),
            Ok(one),
            Err(changed),
            Ok(two),
            Ok(serde_json::json!({ "status": "rotated" })),
        ]);
        let (etx, erx, _sink, _lrx) = channels();
        let (previous, space_key) = (DocumentKey::generate(), DocumentKey::generate());
        let params = RotateChannelParams {
            space_id,
            channel_id,
            url_scheme: crate::http::UrlScheme::Https,
            api_base: crate::http::Url::<crate::http::RelayHost>::new("api.test".to_owned()),
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
        let http = FakeHttp::new(vec![Err(http::HttpError::Local(
            ambre_core::LocalFailure::NoRelay,
        ))]);
        let params = RotateChannelParams {
            space_id,
            channel_id,
            url_scheme: crate::http::UrlScheme::Https,
            api_base: crate::http::Url::<crate::http::RelayHost>::new("api.test".to_owned()),
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

    /// `SYNC-143`: a rotation that keeps meeting newcomers ends after its
    /// tries, an owed one and a channel's alike. Another refusal ends a
    /// channel's at once.
    #[tokio::test]
    async fn a_rotation_that_keeps_meeting_newcomers_ends_after_the_tries() {
        let (my_xk, my_pub) = keypair(1);
        let members = serde_json::json!({ "rotation_owed": true, "members": [
            member_row("11111111-1111-1111-1111-111111111111", "Me", &my_pub),
        ] });
        let refused = |code| {
            http::HttpError::refused(
                http::Method::Post,
                &crate::http::Target::relay("x"),
                ambre_core::HttpStatus::new(409),
                code,
            )
        };
        let mut script = Vec::new();
        for _ in 0..ROTATION_TRIES {
            script.extend([
                Ok(members.clone()),
                Err(refused(Some(RefusalCode::MembersChanged))),
            ]);
        }
        let http = FakeHttp::new(script);
        let (etx, erx, _sink, _lrx) = channels();
        settle_task(&http, settle_params(&my_xk), &etx).await;
        assert!(matches!(
            erx.try_recv(),
            Ok(SyncEvent::DebtUnsettled { .. })
        ));
        assert_eq!(http.calls().len(), 2 * ROTATION_TRIES);

        let head = serde_json::json!({
            "meta_id": Id::<Meta>::mint().to_string(), "role": "editor", "length": 1,
        });
        let params = || RotateChannelParams {
            space_id: Id::<Space>::mint(),
            channel_id: Id::<Channel>::mint(),
            url_scheme: crate::http::UrlScheme::Https,
            api_base: crate::http::Url::<crate::http::RelayHost>::new("api.test".to_owned()),
            exchange_key: my_xk.clone(),
            signing_key: MasterSeed::from_array([9; 32]).derive_signing_key(),
            previous: DocumentKey::generate(),
            anchor: FixedBytes::from_array([5; 32]),
            space_key: DocumentKey::generate(),
        };
        let mut script = vec![Ok(head.clone())];
        for _ in 0..ROTATION_TRIES {
            script.extend([
                Ok(members.clone()),
                Err(refused(Some(RefusalCode::MembersChanged))),
            ]);
        }
        let http = FakeHttp::new(script);
        rotate_channel_task(&http, params(), &etx).await;
        assert!(matches!(
            erx.try_recv(),
            Ok(SyncEvent::ChannelRotated { result: Err(_), .. })
        ));
        assert_eq!(http.calls().len(), 1 + 2 * ROTATION_TRIES);

        let http = FakeHttp::new(vec![Ok(head), Ok(members), Err(refused(None))]);
        rotate_channel_task(&http, params(), &etx).await;
        assert!(matches!(
            erx.try_recv(),
            Ok(SyncEvent::ChannelRotated { result: Err(_), .. })
        ));
        assert_eq!(http.calls().len(), 3, "another refusal is not tried again");
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
            Err(http::HttpError::Local(ambre_core::LocalFailure::NoRelay)),
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
            LogicEvent::ApiError {
                failure: ApiFailure::Local(_),
                ..
            }
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
                    "signing_pub_key": "00".repeat(32), "exchange_pub_key": a_pub,
                    "doc_key_enc": ambre_core::Encoded::<ambre_core::WrappedDocKey>::new(vec![0; 72]), "role": "editor",
                    "wrapper_exchange_pub_key": a_pub,
                },
                {
                    // The identity point: a key no wrap takes (`SYNC-035`).
                    "user_id": "22222222-2222-2222-2222-222222222222",
                    "display_name": "Broken", "email": "b@test.local",
                    "signing_pub_key": "00".repeat(32), "exchange_pub_key": "00".repeat(32),
                    "doc_key_enc": ambre_core::Encoded::<ambre_core::WrappedDocKey>::new(vec![0; 72]), "role": "editor",
                    "wrapper_exchange_pub_key": "00".repeat(32),
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
            LogicEvent::KeyNotShared { members, .. } => {
                assert!(
                    members.iter().any(|m| m.as_str() == "Broken"),
                    "{members:?}"
                );
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[tokio::test]
    async fn revoke_stops_when_the_members_cannot_be_read() {
        let (my_xk, _) = keypair(1);
        let http = FakeHttp::new(vec![Err(http::HttpError::Local(
            ambre_core::LocalFailure::NoRelay,
        ))]);
        let (etx, erx, sink, lrx) = channels();
        revoke_task(&http, revoke_params(&my_xk), &etx, &sink).await;

        match lrx.try_recv().expect("ApiError emitted") {
            LogicEvent::ApiError { failure, .. } => {
                assert_eq!(failure, ApiFailure::Local(LocalFailure::NoRelay))
            }
            other => panic!("unexpected: {other:?}"),
        }
        assert!(erx.try_recv().is_err());
        // Stopped after the failed read: nothing posted, so nothing removed.
        assert_eq!(http.calls().len(), 1);
    }

    fn list_params() -> ListMembersParams {
        ListMembersParams {
            id: Some(ambre_bus::next_request()),
            asked: ambre_core::Seq::nth(1),
            document_id: Id::<Document>::mint(),
            url_scheme: crate::http::UrlScheme::Https,
            api_base: crate::http::Url::<crate::http::RelayHost>::new("api.test".to_owned()),
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
            "signing_pub_key": "00".repeat(32), "exchange_pub_key": "ab".repeat(32),
            "doc_key_enc": ambre_core::Encoded::<ambre_core::WrappedDocKey>::new(vec![0; 72]), "role": "viewer",
            "wrapper_exchange_pub_key": "ab".repeat(32),
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
        let http = FakeHttp::new(vec![Err(http::HttpError::Local(
            ambre_core::LocalFailure::NoRelay,
        ))]);
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
        let uid = Id::<User>::parse_str("11111111-1111-1111-1111-111111111111").unwrap();
        let key = FixedBytes::from_array([0xaa; 32]);
        let dir = presence_directory([(uid, Some(&key))]);

        assert_eq!(dir, vec![(FixedBytes::from_array([0xaa; 32]), uid)]);
    }

    /// Holds `SYNC-215`: the relay serves Alice's row with Mallory's key, so
    /// that Mallory's deltas read as Alice's. The log bound that key to
    /// Mallory, and the row is dropped; Mallory's own row stands. With no
    /// anchored log, every row stands, as before the log.
    #[test]
    fn a_row_naming_a_key_the_log_binds_to_another_member_is_dropped() {
        use crate::membership_log::tests::{DOC, Person, append, created};
        let (alice, bob, mallory) = (Person::new(1), Person::new(2), Person::new(9));
        let mut entries = created(&alice, &bob);
        append(
            &mut entries,
            &alice,
            &mallory,
            MembershipAct::Invite,
            Role::Editor,
        );
        let anchor = entries.first().and_then(membership_log::digest);
        let rows = [
            (alice.id, mallory.public()),
            (mallory.id, mallory.public()),
            (bob.id, bob.public()),
        ];
        let served = || rows.iter().map(|(user, key)| (*user, Some(key)));
        let log = Log::walk(DOC, anchor, &entries, membership_log::no_self_join);
        assert_eq!(
            presence_directory_under(&log, served()),
            vec![(mallory.public(), mallory.id), (bob.public(), bob.id)]
        );
        let unanchored = Log::walk(DOC, None, &entries, membership_log::no_self_join);
        assert_eq!(presence_directory_under(&unanchored, served()).len(), 3);
    }

    #[test]
    fn presence_directory_skips_unparsable_rows_without_inventing_ids() {
        // The answer reads whole, and the directory skips each row whose
        // signing key does not parse. A fabricated id would attribute a cursor
        // to the wrong person, which is what keying presence on the Id<User>
        // prevents.
        let row = |user_id: &str, signing_pub_key: String| {
            serde_json::json!({
                "user_id": user_id,
                "display_name": "A", "email": "a@test.local",
                "signing_pub_key": signing_pub_key, "exchange_pub_key": "ab".repeat(32),
                "doc_key_enc": ambre_core::Encoded::<ambre_core::WrappedDocKey>::new(vec![0; 72]),
                "role": "viewer", "wrapper_exchange_pub_key": "ab".repeat(32),
            })
        };
        let answer = serde_json::from_value::<http::DocumentMembersResponse>(serde_json::json!({
            "members": [
                row("33333333-3333-3333-3333-333333333333", "zz-not-hex".to_owned()),
                // 31 bytes: not a valid Ed25519 key.
                row("44444444-4444-4444-4444-444444444444", "bb".repeat(31)),
                row("11111111-1111-1111-1111-111111111111", "bb".repeat(32)),
            ]
        }))
        .expect("a malformed key never refuses the whole answer");
        assert_eq!(answer.members.len(), 3);
        let dir = presence_directory(
            answer
                .members
                .iter()
                .map(|m| (m.user_id, m.signing_pub_key.key())),
        );
        let good_uid = Id::<User>::parse_str("11111111-1111-1111-1111-111111111111").unwrap();
        assert_eq!(dir, vec![(FixedBytes::from_array([0xbb; 32]), good_uid)]);
    }

    // -- Own role (architecture) -----------------------------------------

    #[test]
    fn own_role_reads_our_own_row() {
        let me = Id::<User>::parse_str("11111111-1111-1111-1111-111111111111").unwrap();
        let other = Id::<User>::parse_str("22222222-2222-2222-2222-222222222222").unwrap();
        let role = own_role([(other, Role::Owner), (me, Role::Viewer)], Some(me));

        // Our own row wins: a peer being owner says nothing about our access.
        assert_eq!(role, Role::Viewer);
    }

    #[test]
    fn own_role_falls_back_to_viewer_when_unresolvable() {
        let me = Id::<User>::parse_str("11111111-1111-1111-1111-111111111111").unwrap();
        let other = Id::<User>::parse_str("22222222-2222-2222-2222-222222222222").unwrap();

        // No session id yet.
        assert_eq!(own_role([(other, Role::Owner)], None), Role::Viewer);
        // No row for us in the response.
        assert_eq!(own_role([(other, Role::Owner)], Some(me)), Role::Viewer);
        // A valid role that is not a document role (channel / org side).
        assert_eq!(own_role([(me, Role::Admin)], Some(me)), Role::Viewer);
    }

    #[test]
    fn own_role_reads_writable_roles() {
        let me = Id::<User>::parse_str("11111111-1111-1111-1111-111111111111").unwrap();
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
            "signing_pub_key": key_hex, "exchange_pub_key": "ab".repeat(32),
            "doc_key_enc": ambre_core::Encoded::<ambre_core::WrappedDocKey>::new(vec![0; 72]), "role": "viewer",
            "wrapper_exchange_pub_key": "ab".repeat(32),
        } ] }))]);
        let (etx, erx, _sink, _lrx) = channels();
        list_members_task(&http, list_params(), &etx).await;

        match erx.try_recv().expect("DocumentMembersReady") {
            SyncEvent::DocumentMembersReady { signing_keys, .. } => {
                assert_eq!(
                    signing_keys,
                    vec![(
                        FixedBytes::from_array([0xcc; 32]),
                        Id::<User>::parse_str(uid).unwrap()
                    )]
                );
            }
            _ => panic!("unexpected event (want DocumentMembersReady)"),
        }
    }

    // -- The signed membership log (`ambre-spec-membership.md`).

    use ambre_bus::MemberStanding;
    use ambre_core::Seq;
    use ambre_protocol::LogPlaces;

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
                    user_id: Id::<User>::from_bytes([seed; 16]),
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
        document_id: Id<Document>,
        doc_key: DocumentKey,
        anchor: FixedBytes<EntryDigest>,
        owner_exchange: String,
        rows: Vec<serde_json::Value>,
        log: Vec<ambre_core::Encoded<ambre_core::EncodedLogEntry>>,
    }

    impl Logged {
        fn new(owner: &Peer) -> Self {
            let document_id = Id::<Document>::mint();
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

        fn row(
            &mut self,
            peer: &Peer,
            doc_key_enc: ambre_core::Encoded<ambre_core::WrappedDocKey>,
            role: Role,
        ) {
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
            texts.extend(
                more.iter()
                    .map(|text| serde_json::from_value(serde_json::json!(text)).unwrap()),
            );
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
                member_key: FixedBytes::from_array(
                    peer.author.signing_key.verifying_key().to_bytes(),
                ),
                member_exchange_key: own_exchange_key(&peer.xk),
                act: MembershipAct::Invite,
                role,
                key: Some(self.doc_key.id()),
            };
            let entry = entry_after(&self.walk(&[]), draft, &owner.author)
                .unwrap()
                .unwrap();
            self.log.push(entry);
            let peer_pub = own_exchange_key(&peer.xk);
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
        membership_log::decode(&[serde_json::from_value(serde_json::json!(text)).unwrap()])
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
        let mut params = invite_params(&owner.xk, invitee_hex);
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
        let invitee = Id::<User>::parse_str("55555555-5555-5555-5555-555555555555").unwrap();
        assert_eq!(entry.act, MembershipAct::Invite);
        assert_eq!(entry.member, invitee);
        assert_eq!(entry.key, Some(logged.doc_key.id()));
        let text = body["entry"].as_str().unwrap();
        assert_eq!(
            logged.walk(&[text]).standing(invitee),
            (MemberStanding::Verified, Some(Role::Editor))
        );
        let owner_pub = own_exchange_key(&owner.xk);
        let enc = &serde_json::from_value::<ambre_core::Encoded<ambre_core::WrappedDocKey>>(
            body["doc_key_enc"].clone(),
        )
        .unwrap();
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
        let mut params = invite_params(&owner.xk, invitee_hex);
        params.document_id = logged.document_id;
        params.doc_key = logged.doc_key.clone();
        params.author = same(&owner.author);
        let http = FakeHttp::new(vec![
            Ok(user_search_json("invitee@test.local", &invitee_hex)),
            Ok(first),
            Err(http::HttpError::refused(
                http::Method::Post,
                &crate::http::Target::relay("https://api.test/share"),
                ambre_core::HttpStatus::new(409),
                Some(RefusalCode::LogMoved),
            )),
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
        let positions: Vec<Seq<LogPlaces>> = calls
            .iter()
            .filter_map(|call| match call {
                Call::Post(_, body) => Some(posted_entry(body).position),
                _ => None,
            })
            .collect();
        assert_eq!(
            positions,
            vec![Seq::<LogPlaces>::new(1), Seq::<LogPlaces>::new(2)]
        );
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
            id: ambre_bus::next_request(),
            document_id: logged.document_id,
            author: same(&bob.author),
            url_scheme: crate::http::UrlScheme::Https,
            api_base: crate::http::Url::<crate::http::RelayHost>::new("api.test".to_owned()),
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
        assert_eq!(entry.role, Role::Editor, "the role the leaver held");
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
            id: ambre_bus::next_request(),
            document_id: logged.document_id,
            user_id: bob.author.user_id,
            role: Role::Viewer,
            url_scheme: crate::http::UrlScheme::Https,
            api_base: crate::http::Url::<crate::http::RelayHost>::new("api.test".to_owned()),
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
        assert_eq!(
            posted_entry(body).member_key,
            FixedBytes::from_array(bob.author.signing_key.verifying_key().to_bytes()),
            "the entry names the key of the member it changes"
        );
        let text = body["entry"].as_str().unwrap();
        let log = logged.walk(&[text]);
        assert_eq!(log.standing(bob.author.user_id).1, Some(Role::Viewer));
    }

    /// The refusal of a log that moved, as the relay answers it.
    fn log_moved() -> http::HttpError {
        http::HttpError::refused(
            http::Method::Post,
            &crate::http::Target::relay("https://api.test/share"),
            ambre_core::HttpStatus::new(409),
            Some(RefusalCode::LogMoved),
        )
    }

    /// `SYNC-132`: a log that keeps moving ends an invite and a leave after
    /// their tries, each read again before it is signed again.
    #[tokio::test]
    async fn a_log_that_keeps_moving_ends_after_the_tries() {
        let owner = Peer::new(1);
        let bob = Peer::new(2);
        let mut logged = Logged::new(&owner);
        logged.invite(&owner, &bob, Role::Editor);

        let (_invitee_xk, invitee_hex) = keypair(3);
        let mut params = invite_params(&owner.xk, invitee_hex);
        params.document_id = logged.document_id;
        params.doc_key = logged.doc_key.clone();
        params.author = same(&owner.author);
        let mut script = vec![Ok(user_search_json("invitee@test.local", &invitee_hex))];
        for _ in 0..ROTATION_TRIES {
            script.extend([Ok(logged.answer()), Err(log_moved())]);
        }
        let http = FakeHttp::new(script);
        let (etx, erx, sink, lrx) = channels();
        invite_task(&http, params, &etx, &sink).await;
        assert!(erx.try_recv().is_err(), "nothing was invited");
        assert!(matches!(lrx.try_recv(), Ok(LogicEvent::ApiError { .. })));
        assert_eq!(http.calls().len(), 1 + 2 * ROTATION_TRIES);

        let params = LeaveParams {
            id: ambre_bus::next_request(),
            document_id: logged.document_id,
            author: same(&bob.author),
            url_scheme: crate::http::UrlScheme::Https,
            api_base: crate::http::Url::<crate::http::RelayHost>::new("api.test".to_owned()),
            exchange_key: bob.xk.clone(),
        };
        let mut script = Vec::new();
        for _ in 0..ROTATION_TRIES {
            script.extend([Ok(logged.answer()), Err(log_moved())]);
        }
        let http = FakeHttp::new(script);
        let (etx, erx, sink, lrx) = channels();
        leave_task(&http, params, &etx, &sink).await;
        assert!(erx.try_recv().is_err(), "nothing was left");
        assert!(matches!(lrx.try_recv(), Ok(LogicEvent::ApiError { .. })));
        assert_eq!(http.calls().len(), 2 * ROTATION_TRIES);
    }

    /// `SYNC-132`: a role change whose log keeps moving ends after its
    /// tries, and another refusal ends it at once.
    #[tokio::test]
    async fn a_role_change_ends_after_its_tries() {
        let owner = Peer::new(1);
        let bob = Peer::new(2);
        let mut logged = Logged::new(&owner);
        logged.invite(&owner, &bob, Role::Editor);
        let params = || RoleChangeParams {
            id: ambre_bus::next_request(),
            document_id: logged.document_id,
            user_id: bob.author.user_id,
            role: Role::Viewer,
            url_scheme: crate::http::UrlScheme::Https,
            api_base: crate::http::Url::<crate::http::RelayHost>::new("api.test".to_owned()),
            exchange_key: owner.xk.clone(),
            author: same(&owner.author),
        };
        let mut script = Vec::new();
        for _ in 0..ROTATION_TRIES {
            script.extend([Ok(logged.answer()), Err(log_moved())]);
        }
        let http = FakeHttp::new(script);
        let (etx, erx, sink, lrx) = channels();
        role_change_task(&http, params(), &etx, &sink).await;
        assert!(erx.try_recv().is_err());
        assert!(matches!(lrx.try_recv(), Ok(LogicEvent::ApiError { .. })));
        assert_eq!(http.calls().len(), 2 * ROTATION_TRIES);

        let forbidden = http::HttpError::refused(
            http::Method::Patch,
            &crate::http::Target::relay("x"),
            ambre_core::HttpStatus::new(403),
            None,
        );
        let http = FakeHttp::new(vec![Ok(logged.answer()), Err(forbidden)]);
        role_change_task(&http, params(), &etx, &sink).await;
        assert!(matches!(lrx.try_recv(), Ok(LogicEvent::ApiError { .. })));
        assert_eq!(http.calls().len(), 2, "another refusal is not tried again");
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
        logged.row(
            &Peer::new(9),
            ambre_core::Encoded::new(vec![0; 72]),
            Role::Editor,
        );
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
            member_key: FixedBytes::from_array(
                joiner.author.signing_key.verifying_key().to_bytes(),
            ),
            member_exchange_key: own_exchange_key(&joiner.xk),
            act: MembershipAct::SelfJoin,
            role: Role::Editor,
            key: Some(logged.doc_key.id()),
        };
        let entry = entry_after(&logged.walk(&[]), draft, &joiner.author)
            .unwrap()
            .unwrap();
        logged.log.push(entry);
        logged.row(&joiner, ambre_core::Encoded::new(vec![0; 72]), Role::Editor);
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

    /// The `meta` document of a channel is the row of that kind, wherever the relay lists
    /// it. A listing with none, or a failed read, names none.
    #[tokio::test]
    async fn the_meta_of_a_channel_is_the_row_of_that_kind() {
        let meta = Id::<Document>::mint();
        let channel = Id::<Channel>::mint();
        let base = crate::http::Url::<crate::http::RelayHost>::new("api.test".to_owned());
        let scheme = crate::http::UrlScheme::Https;
        let listing = |rows: serde_json::Value| {
            FakeHttp::new(vec![Ok(serde_json::json!({ "documents": rows }))])
        };
        let http = listing(serde_json::json!([
            { "document_id": Id::<Document>::mint().to_string(), "kind": "segment" },
            { "document_id": meta.to_string(), "kind": "meta" },
        ]));
        assert_eq!(
            channel_meta(&http, channel, scheme, &base).await,
            Some(meta)
        );
        let http = listing(serde_json::json!([
            { "document_id": Id::<Document>::mint().to_string(), "kind": "segment" },
        ]));
        assert_eq!(channel_meta(&http, channel, scheme, &base).await, None);
        let http = FakeHttp::new(vec![Err(http::HttpError::Local(
            ambre_core::LocalFailure::NoRelay,
        ))]);
        assert_eq!(channel_meta(&http, channel, scheme, &base).await, None);
    }

    /// The log of the channel of a call is read from the members of its `meta` and answered
    /// as `CallLog`. A failed read answers nothing.
    #[tokio::test]
    async fn the_log_of_a_call_is_read_from_its_meta() {
        let owner = Peer::new(1);
        let logged = Logged::new(&owner);
        let params = || CallLogParams {
            document_id: logged.document_id,
            url_scheme: crate::http::UrlScheme::Https,
            api_base: crate::http::Url::<crate::http::RelayHost>::new("api.test".to_owned()),
            exchange_key: owner.xk.clone(),
            rule: LogRule::document(),
        };
        let http = FakeHttp::new(vec![Ok(logged.answer())]);
        let (etx, erx, _sink, _lrx) = channels();
        call_log_task(&http, params(), &etx).await;
        match erx.try_recv() {
            Ok(SyncEvent::CallLog { meta, log }) => {
                assert_eq!(meta, logged.document_id);
                assert!(log.present_key(owner.author.user_id).is_some());
            }
            _ => panic!("no CallLog"),
        }
        let http = FakeHttp::new(vec![Err(http::HttpError::Local(
            ambre_core::LocalFailure::NoRelay,
        ))]);
        let (etx, erx, _sink, _lrx) = channels();
        call_log_task(&http, params(), &etx).await;
        assert!(erx.try_recv().is_err());
    }
}
