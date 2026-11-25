//! The settings document of the signed-in user (`ambre-spec-user-settings.md`).
//!
//! At each unlock the worker asks the relay which document holds the
//! settings of the account, and claims one when the account has none. The
//! document then opens like any document: its key is the self-wrap every
//! document takes (`SETTINGS-017`), so no new derivation exists.

use super::*;
use crate::error::SyncError;
use crate::http::{AuthContext, Transport, Url};

/// The answer of both settings routes.
#[derive(serde::Deserialize)]
pub(crate) struct SettingsDocumentResponse {
    #[serde(with = "ambre_core::serde_id::text")]
    pub document_id: Id<Document>,
}

/// The settings document a first unlock offers to the relay.
#[derive(serde::Serialize)]
pub(crate) struct ClaimSettingsRequest {
    #[serde(with = "ambre_core::serde_id::text")]
    pub document_id: Id<Document>,
    pub doc_key_enc: ambre_core::Encoded<ambre_core::WrappedDocKey>,
    pub entry: ambre_core::Encoded<ambre_core::EncodedLogEntry>,
}

/// The settings document of the account: the one the relay names, or a new
/// one this call claims (`SETTINGS-007`, `SETTINGS-008`). Two first unlocks
/// at once both read the document the relay kept (`SETTINGS-009`).
pub(crate) async fn settings_document_via(
    transport: &impl Transport,
    root: &Url<crate::http::ApiRoot>,
    auth: &AuthContext,
    exchange_key: &x25519_dalek::StaticSecret,
    author: &sharing::Author,
) -> Result<Id<Document>, SyncError> {
    let route = root.route("/settings");
    let known: Option<SettingsDocumentResponse> =
        http::get_json_opt_via(transport, &route, auth).await?;
    if let Some(known) = known {
        return Ok(known.document_id);
    }
    let document_id = Id::<Document>::mint();
    let doc_key = DocumentKey::generate();
    let (doc_key_enc, entry) = sharing::creation(document_id, &doc_key, exchange_key, author)?;
    let body = ClaimSettingsRequest {
        document_id,
        doc_key_enc,
        entry,
    };
    let claimed: SettingsDocumentResponse =
        http::post_json_auth_via(transport, &route, &body, auth).await?;
    Ok(claimed.document_id)
}

impl SyncState {
    /// Finds the settings document of the account once the session is
    /// ready, and names it to the Logic thread, which opens it.
    pub(crate) fn find_settings_document(&self) {
        let (Some(tx), Some(exchange_key), Some(author)) = (
            self.event_tx.clone(),
            self.exchange_key.clone(),
            self.author(),
        ) else {
            return;
        };
        let root = Url::<crate::http::ApiRoot>::of(self.url_scheme, &self.api_base);
        let auth = self.auth_context();
        crate::spawn(async move {
            match settings_document_via(&http::Fetch, &root, &auth, &exchange_key, &author).await {
                Ok(document_id) => {
                    let _ = tx.send(SyncEvent::SettingsDocumentFound { document_id });
                }
                Err(error) => {
                    crate::log::warning!("Sync: no settings document this session: {error}");
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::scripted::{Scripted, session};

    fn author() -> (sharing::Author, x25519_dalek::StaticSecret) {
        let author = sharing::Author {
            user_id: Id::<User>::mint(),
            signing_key: ed25519_dalek::SigningKey::from_bytes(&[7; 32]),
        };
        (author, x25519_dalek::StaticSecret::from([9; 32]))
    }

    fn root() -> Url<crate::http::ApiRoot> {
        Url::<crate::http::ApiRoot>::of(
            crate::http::UrlScheme::Https,
            &Url::<crate::http::RelayHost>::new("relay.test".to_owned()),
        )
    }

    /// `SETTINGS-007`: an account that holds a settings document opens it,
    /// and claims nothing.
    #[tokio::test]
    async fn a_known_settings_document_is_opened_without_a_claim() {
        let known = Id::<Document>::mint();
        let transport = Scripted::new([Ok((
            200,
            serde_json::json!({ "document_id": known.to_string() }),
        ))]);
        let (author, exchange) = author();
        let found =
            settings_document_via(&transport, &root(), &session("a", "r"), &exchange, &author)
                .await;
        assert_eq!(found.ok(), Some(known));
        assert_eq!(transport.calls().len(), 1, "one read, no claim");
    }

    /// `SETTINGS-041`: the exchange key derives from the Master Seed, and a
    /// recovery or a new device unlocks the same seed. So the self-wrap of the
    /// creation opens on that device with no re-wrap, and the seed of another
    /// account opens nothing.
    #[test]
    fn the_settings_self_wrap_opens_under_the_key_a_recovery_derives() {
        let seed = MasterSeed::generate();
        let words = seed.recovery_words();
        let author = sharing::Author {
            user_id: Id::<User>::mint(),
            signing_key: seed.derive_signing_key(),
        };
        let first = seed.derive_exchange_key();
        let document = Id::<Document>::mint();
        let doc_key = DocumentKey::generate();
        let (doc_key_enc, _entry) =
            sharing::creation(document, &doc_key, &first, &author).expect("a creation");
        let registered = sharing::own_exchange_key(&first);
        drop((seed, first));

        let recovered = MasterSeed::from_recovery_words(&words).expect("the words of the account");
        let derived = recovered.derive_exchange_key();
        assert_eq!(
            sharing::own_exchange_key(&derived),
            registered,
            "a recovery derives the exchange key the relay registered"
        );
        let row = [(&registered, &doc_key_enc, None)];
        let (opened, anchor) =
            sharing::unwrap_own(&derived, document, row).expect("the self-wrap opens");
        assert_eq!(opened.id(), doc_key.id(), "the same document key");
        assert!(
            anchor.is_some(),
            "the anchor of the creation rides the wrap"
        );

        let stranger = MasterSeed::generate().derive_exchange_key();
        let stranger_pub = sharing::own_exchange_key(&stranger);
        let row = [(&stranger_pub, &doc_key_enc, Some(&registered))];
        assert!(
            sharing::unwrap_own(&stranger, document, row).is_err(),
            "another seed opens nothing"
        );
    }

    /// `SETTINGS-008`: an account with none claims one, and
    /// opens whichever document the relay answers, its own or another
    /// device's.
    #[tokio::test]
    async fn an_account_with_none_claims_one_and_opens_the_relay_answer() {
        let winner = Id::<Document>::mint();
        let transport = Scripted::new([
            Ok((404, serde_json::json!({}))),
            Ok((
                200,
                serde_json::json!({ "document_id": winner.to_string() }),
            )),
        ]);
        let (author, exchange) = author();
        let found =
            settings_document_via(&transport, &root(), &session("a", "r"), &exchange, &author)
                .await;
        assert_eq!(found.ok(), Some(winner));
        let calls = transport.calls();
        assert_eq!(calls.len(), 2);
        assert!(calls.iter().all(|(_, url)| url.ends_with("/api/settings")));
        let sent = transport.sent.borrow();
        let (_, body) = sent[1].body.as_ref().expect("the claim carries a body");
        let claim: serde_json::Value =
            serde_json::from_slice(body.as_bytes()).expect("a JSON body");
        assert!(
            claim["entry"].is_string(),
            "the first entry rides the claim"
        );
        assert!(
            claim["doc_key_enc"].is_string(),
            "the self-wrap rides the claim"
        );
        let mut fields: Vec<&str> = claim
            .as_object()
            .expect("an object")
            .keys()
            .map(String::as_str)
            .collect();
        fields.sort_unstable();
        assert_eq!(
            fields,
            ["doc_key_enc", "document_id", "entry"],
            "no setting rides the claim (`ZK-054`)"
        );
    }
}
