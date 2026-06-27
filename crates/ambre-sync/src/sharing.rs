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
use x25519_dalek::StaticSecret;

use ambre_bus::{MemberInfo, Role, UserId};

use crate::http;

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
) -> String {
    let wrapped = ambre_crypto::ecdh::wrap_document_key(my_xk, peer_pub, doc_key);
    let mut enc = wrapped.nonce.to_vec();
    enc.extend_from_slice(&wrapped.ciphertext);
    base64::engine::general_purpose::URL_SAFE.encode(&enc)
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
        doc_key_enc: wrap_key_b64(my_xk, &peer, doc_key),
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
                doc_key_enc: wrap_key_b64(my_xk, &peer, new_doc_key),
            })
        })
        .collect()
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
}
