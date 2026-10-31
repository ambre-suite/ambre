use super::*;
use crate::error::SyncError;
use crate::http::Reported;

impl SyncState {
    // -- Document CRUD (HTTP REST) ---------------------------------------------

    pub(crate) fn handle_list_documents(&self, id: RequestId) {
        let event_tx = self.event_tx.clone();

        if let Some(tx) = event_tx {
            let api_base = self.api_base.clone();
            let url_scheme = self.url_scheme;
            let auth = self.auth_context();
            let exchange_key = self.exchange_key.clone();
            wasm_bindgen_futures::spawn_local(async move {
                let url = crate::http::ApiRoot::of(url_scheme, &api_base).route("/documents");
                match http::get_json_auth::<http::DocumentListResponse>(&url, auth.clone()).await {
                    Ok(resp) => {
                        let mut docs = Vec::with_capacity(resp.documents.len());
                        for e in resp.documents {
                            let document_id = e.id;
                            // E2EE: the name arrives as `name_enc`. Decrypt it
                            // here (Sync) so the Logic/UI threads only ever see
                            // plaintext (`ambre-spec-launcher.md`, CHROME-044). A
                            // failed unwrap/decrypt renders as `None` ("Untitled").
                            let name = match (e.name_enc.as_ref(), exchange_key.as_ref()) {
                                (Some(name_enc), Some(ek)) => {
                                    let listed = ListedName {
                                        document_id,
                                        doc_key_enc: &e.doc_key_enc,
                                        wrapper_exchange_pub_key: &e.wrapper_exchange_pub_key,
                                        name_enc,
                                    };
                                    listed.open(ek, url_scheme, &api_base, &auth).await
                                }
                                _ => None,
                            };
                            docs.push(DocumentSummary {
                                document_id,
                                doc_type: e.doc_type,
                                name,
                                role: e.role,
                                // RFC3339 -> epoch ms via the JS Date parser
                                // (NaN -> 0 when the server timestamp is absent).
                                updated_at: {
                                    let ms = js_sys::Date::parse(e.updated_at.as_str());
                                    ambre_bus::Timestamp(if ms.is_nan() { 0 } else { ms as u64 })
                                },
                                member_count: e.member_count.max(ambre_core::MemberCount::ONE),
                            });
                        }
                        let _ = tx.send(SyncEvent::DocumentList {
                            id,
                            documents: docs,
                        });
                    }
                    Err(e) => {
                        let _ = tx.send(SyncEvent::http_error(id, e));
                    }
                }
            });
        } else {
            let documents: Vec<DocumentSummary> = self
                .doc_metadata
                .iter()
                .map(|(&document_id, (doc_type, name))| DocumentSummary {
                    document_id,
                    doc_type: *doc_type,
                    name: Some(name.clone()),
                    role: Role::Owner,
                    updated_at: ambre_bus::Timestamp(0),
                    member_count: ambre_core::MemberCount::ONE,
                })
                .collect();
            let count = documents.len();
            self.reply(id, RestEvent::DocumentList { documents });
            tracing::info!("Sync: ListDocuments ({count} docs)");
        }
    }

    /// Seals a document name with the full envelope, bound to its document
    /// (`SYNC-018`, `SYNC-068`). A [`ambre_core::Name`] never runs past its
    /// budget, so the relay never holds one it would refuse (`SYNC-178`).
    pub(crate) fn seal_document_name(
        document_id: Id<Document>,
        doc_key: &DocumentKey,
        signing_key: &SigningKey,
        name: &ambre_core::Name,
    ) -> Result<ambre_core::SealedName, pipeline::PipelineError> {
        let sealed = pipeline::seal_object(
            &name.to_plaintext(),
            &pipeline::name_aad(document_id),
            doc_key,
            signing_key,
        )?;
        Ok(sealed.into_name())
    }

    /// The document a create answer names, refused unless the client minted
    /// it. The name is sealed under that id, and no other id opens it
    /// (`SYNC-136`). A malformed id never reaches here: the answer is refused
    /// when it is read.
    fn created_document(
        minted: Id<Document>,
        answered: Id<Document>,
    ) -> Result<Id<Document>, crate::error::SyncError> {
        if answered == minted {
            Ok(minted)
        } else {
            Err(crate::error::SyncError::WrongDocument { minted, answered })
        }
    }

    pub(crate) fn handle_create_document(
        &mut self,
        id: RequestId,
        doc_type: DocType,
        name: ambre_core::Name,
    ) {
        let event_tx = self.event_tx.clone();

        if let Some(tx) = event_tx {
            let api_base = self.api_base.clone();
            let url_scheme = self.url_scheme;
            let auth = self.auth_context();
            let exchange_key = self.exchange_key.clone();
            let author = self.author();
            wasm_bindgen_futures::spawn_local(async move {
                // E2EE: the server only ever stores ciphertext. Generate a fresh
                // DocumentKey, seal the name with it, and wrap the key for the
                // owner (ourselves) via X25519. We can then recover it on open.
                let (Some(exchange_key), Some(author)) = (exchange_key, author) else {
                    let _ = tx.send(SyncEvent::http_error(
                        id,
                        SyncError::SessionLocked.at("CreateDocument"),
                    ));
                    return;
                };
                let doc_key = DocumentKey::generate();
                // The client mints the id, so the name is bound to its document
                // before the relay has seen either (`SYNC-068`).
                let document_id = Id::<Document>::mint();

                // Sealing and wrapping fail only on a bug. The document is not
                // created then, and the caller reads why.
                let name_enc = if name.is_empty() {
                    Ok(None)
                } else {
                    Self::seal_document_name(document_id, &doc_key, &author.signing_key, &name)
                        .map(Some)
                        .map_err(|e| SyncError::from(e).at("CreateDocument: name seal failed"))
                };
                // The copy carries the anchor of the log its creator starts
                // (`SYNC-104`, `SYNC-132`).
                let sealed = name_enc.and_then(|name_enc| {
                    sharing::creation(document_id, &doc_key, &exchange_key, &author)
                        .map(|created| (name_enc, created))
                        .map_err(|e| e.at("CreateDocument"))
                });
                let (name_enc, (doc_key_enc, entry)) = match sealed {
                    Ok(pair) => pair,
                    Err(failed) => {
                        let _ = tx.send(SyncEvent::http_error(id, failed));
                        return;
                    }
                };

                let url = crate::http::ApiRoot::of(url_scheme, &api_base).route("/documents");
                let body = http::CreateDocumentRequest {
                    doc_type,
                    name_enc,
                    doc_key_enc,
                    document_id: Some(document_id),
                    entry: Some(entry),
                    space: ambre_core::DocumentScope::Personal,
                };
                match http::post_json_auth::<_, http::CreateDocumentResponse>(&url, &body, auth)
                    .await
                {
                    Ok(resp) => match Self::created_document(document_id, resp.document_id) {
                        Ok(document_id) => {
                            let _ = tx.send(SyncEvent::DocumentCreated {
                                id,
                                document_id,
                                doc_type,
                            });
                        }
                        Err(message) => {
                            let _ = tx.send(SyncEvent::http_error(id, message));
                        }
                    },
                    Err(e) => {
                        let _ = tx.send(SyncEvent::http_error(id, e));
                    }
                }
            });
        } else {
            tracing::info!("Sync: CreateDocument {doc_type:?} '{name}'");
            let document_id = Id::<Document>::mint();
            self.doc_metadata.insert(document_id, (doc_type, name));
            self.reply(
                id,
                RestEvent::DocumentCreated {
                    document_id,
                    doc_type,
                },
            );
        }
    }

    pub(crate) fn handle_delete_document(&mut self, id: RequestId, document_id: Id<Document>) {
        let event_tx = self.event_tx.clone();

        if let Some(tx) = event_tx {
            let api_base = self.api_base.clone();
            let url_scheme = self.url_scheme;
            let auth = self.auth_context();
            let doc_id = document_id;
            wasm_bindgen_futures::spawn_local(async move {
                let url = crate::http::ApiRoot::of(url_scheme, &api_base)
                    .route(format!("/documents/{doc_id}"));
                match http::delete_auth(&url, auth).await {
                    Ok(()) => {
                        let _ = tx.send(SyncEvent::DocumentDeleted {
                            id,
                            document_id: doc_id,
                        });
                    }
                    Err(e) => {
                        let _ = tx.send(SyncEvent::http_error(id, e));
                    }
                }
            });
        } else {
            tracing::info!("Sync: DeleteDocument {document_id:?}");
            self.doc_metadata.remove(&document_id);
            self.reply(id, RestEvent::DocumentDeleted { document_id });
        }
    }

    pub(crate) fn handle_rename_document(
        &mut self,
        id: RequestId,
        document_id: Id<Document>,
        name: ambre_core::Name,
    ) {
        let event_tx = self.event_tx.clone();

        if let Some(tx) = event_tx {
            let api_base = self.api_base.clone();
            let url_scheme = self.url_scheme;
            let auth = self.auth_context();
            let doc_id = document_id;
            let cached_doc_key = self
                .documents
                .get(&document_id)
                .map(|ds| ds.doc_key.clone());
            let exchange_key = self.exchange_key.clone();
            let signing_key = self.signing_key.clone();
            wasm_bindgen_futures::spawn_local(async move {
                let Some(signing_key) = signing_key else {
                    let _ = tx.send(SyncEvent::http_error(
                        id,
                        SyncError::SessionLocked.at("RenameDocument"),
                    ));
                    return;
                };
                // E2EE: the new name is sealed with the document key before it
                // leaves the client. The server only ever stores ciphertext.
                //
                // The doc is usually open (inline rename from its header), but the
                // launcher renames a closed doc. Fall back to fetching the member
                // list and ECDH-unwrapping our wrapped key, exactly as on open.
                let doc_key = match cached_doc_key {
                    Some(k) => k,
                    None => {
                        let Some(ek) = exchange_key else {
                            let _ = tx.send(SyncEvent::http_error(
                                id,
                                SyncError::SessionLocked.at("RenameDocument"),
                            ));
                            return;
                        };
                        let members_url = crate::http::ApiRoot::of(url_scheme, &api_base)
                            .route(format!("/documents/{doc_id}/members"));
                        match http::get_json_auth::<http::MembersResponse>(
                            &members_url,
                            auth.clone(),
                        )
                        .await
                        {
                            Ok(resp) => match Self::unwrap_document_key(&ek, &resp.members) {
                                Ok(k) => k,
                                Err(e) => {
                                    let _ = tx.send(SyncEvent::http_error(
                                        id,
                                        e.at("RenameDocument: key unwrap failed"),
                                    ));
                                    return;
                                }
                            },
                            Err(e) => {
                                let _ = tx.send(SyncEvent::http_error(id, e));
                                return;
                            }
                        }
                    }
                };
                let name_enc = match Self::seal_document_name(doc_id, &doc_key, &signing_key, &name)
                {
                    Ok(name_enc) => name_enc,
                    Err(e) => {
                        let failed = SyncError::from(e).at("RenameDocument: name seal failed");
                        let _ = tx.send(SyncEvent::http_error(id, failed));
                        return;
                    }
                };

                // PATCH /api/documents/{id} (matches the server route).
                let url = crate::http::ApiRoot::of(url_scheme, &api_base)
                    .route(format!("/documents/{doc_id}"));
                let body = http::RenameDocumentRequest { name_enc };
                match http::patch_json_auth(&url, &body, auth).await {
                    Ok(()) => {
                        let _ = tx.send(SyncEvent::DocumentRenamed {
                            id,
                            document_id: doc_id,
                        });
                    }
                    Err(e) => {
                        let _ = tx.send(SyncEvent::http_error(id, e));
                    }
                }
            });
        } else {
            tracing::info!("Sync: RenameDocument {document_id:?} '{name}'");
            if let Some((_doc_type, old_name)) = self.doc_metadata.get_mut(&document_id) {
                *old_name = name;
            }
            self.reply(id, RestEvent::DocumentRenamed { document_id });
        }
    }
}

/// One sealed name of a listing, with the wrapped key that opens it: a
/// document of the launcher, or the meta of a channel. Keys are base64 of
/// `nonce || ciphertext`; the wrapper's exchange key is hex, empty for a
/// self-wrapped owner row.
pub(crate) struct ListedName<'a> {
    pub(crate) document_id: Id<Document>,
    pub(crate) doc_key_enc: &'a ambre_core::WrappedDocKey,
    pub(crate) wrapper_exchange_pub_key: &'a ambre_core::ExchangeKeyHex,
    pub(crate) name_enc: &'a ambre_core::SealedName,
}

impl ListedName<'_> {
    /// The plaintext name. A name sealed before the last rotation names an
    /// older key, which the chain recovers (`SYNC-042`, `SYNC-119`). `None` on
    /// any decode or crypto failure: the launcher shows "Untitled".
    pub(crate) async fn open(
        &self,
        exchange_key: &x25519_dalek::StaticSecret,
        url_scheme: crate::http::UrlScheme,
        api_base: &crate::http::RelayHost,
        auth: &http::AuthContext,
    ) -> Option<ambre_core::Name> {
        let current = self.unwrap_key(exchange_key)?;
        // One request more, and only for a name older than the current key.
        let links = if pipeline::object_key(self.name_enc).ok()? == current.id() {
            Vec::new()
        } else {
            http::fetch_key_links(url_scheme, api_base, self.document_id, auth.clone()).await
        };
        open_name(self.document_id, &current, self.name_enc, links)
    }

    /// The current key, ECDH-unwrapped against the wrapper's exchange key.
    fn unwrap_key(&self, exchange_key: &x25519_dalek::StaticSecret) -> Option<DocumentKey> {
        unwrap_listed_key(
            self.doc_key_enc,
            self.wrapper_exchange_pub_key,
            exchange_key,
        )
    }
}

/// The key a listing wrapped for this user, ECDH-unwrapped against the
/// wrapper's exchange key. The key is base64 of `nonce || ciphertext`, and
/// the wrapper is hex, empty for a self-wrapped owner row. `None` on any
/// decode or crypto failure.
pub(crate) fn unwrap_listed_key(
    doc_key_enc: &ambre_core::WrappedDocKey,
    wrapper_exchange_pub_key: &ambre_core::ExchangeKeyHex,
    exchange_key: &x25519_dalek::StaticSecret,
) -> Option<DocumentKey> {
    let (nonce, ciphertext) = doc_key_enc.parts()?;
    let wrapped = ambre_crypto::WrappedKey { nonce, ciphertext };
    let wrapper_hex = if wrapper_exchange_pub_key.is_empty() {
        // Self-wrapped owner row: the DH peer is our own public key.
        ambre_core::ExchangeKeyHex::encode(x25519_dalek::PublicKey::from(exchange_key).as_bytes())
    } else {
        wrapper_exchange_pub_key.clone()
    };
    let peer_pub = crate::sharing::decode_exchange_pubkey(&wrapper_hex).ok()?;
    ambre_crypto::ecdh::unwrap_document_key(exchange_key, &peer_pub, &wrapped).ok()
}

/// Opens a sealed name with the key it names: `current`, or one the chain
/// `links` reach from it. `None` on any failure: a name sealed for another
/// document, a forged author, a key out of reach, a name past the cap.
pub(crate) fn open_name(
    document_id: Id<Document>,
    current: &DocumentKey,
    sealed: &ambre_core::SealedName,
    links: Vec<ambre_protocol::KeyLink>,
) -> Option<ambre_core::Name> {
    let chain = crate::chain::Chain::new(document_id, links);
    let wanted = pipeline::object_key(sealed).ok()?;
    let key = crate::chain::find_key(document_id, current, &[], &chain, wanted)?;
    let plaintext = pipeline::open_object(sealed, &pipeline::name_aad(document_id), &key).ok()?;
    // The name is read in place and copied once it is taken, so a refused
    // name leaves no copy behind. The wrapper wipes the opened bytes.
    plaintext.with_plaintext(|bytes| {
        let text = std::str::from_utf8(bytes)
            .ok()
            .filter(|text| ambre_core::name_fits(text))?;
        ambre_core::Name::new(text).ok()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ambre_crypto::MasterSeed;

    /// The seal that create and rename send holds the name under the
    /// document key (`SYNC-067`). The relay's bytes carry no plaintext, name
    /// that key, and open under it alone.
    #[test]
    fn a_document_name_reaches_the_relay_sealed_under_the_document_key() {
        let document_id = Id::<Document>::mint();
        let signing_key = MasterSeed::from_array([5; 32]).derive_signing_key();
        let doc_key = DocumentKey::generate();
        let name = "Quarterly budget";
        let sent = SyncState::seal_document_name(
            document_id,
            &doc_key,
            &signing_key,
            &ambre_core::Name::new(name).unwrap(),
        )
        .unwrap();
        let bytes = sent.as_bytes().to_vec();
        assert!(!bytes.windows(name.len()).any(|w| w == name.as_bytes()));
        assert_eq!(pipeline::object_key(&sent).unwrap(), doc_key.id());
        assert_eq!(
            open_name(document_id, &doc_key, &sent, Vec::new())
                .as_ref()
                .map(ambre_core::Name::as_str),
            Some(name)
        );
        let other = DocumentKey::generate();
        assert_eq!(open_name(document_id, &other, &sent, Vec::new()), None);
    }

    /// `SYNC-178`: a name past its budget is refused before it is sealed: no
    /// [`ambre_core::Name`] holds it. The longest name, in four-byte
    /// characters, seals within what the relay reads, and an empty one is the
    /// overhead alone.
    #[test]
    fn a_name_past_its_budget_is_not_sealed() {
        let document_id = Id::<Document>::mint();
        let doc_key = DocumentKey::generate();
        let signing_key = MasterSeed::from_array([5; 32]).derive_signing_key();
        let seal = |name: &str| {
            let name = ambre_core::Name::new(name).unwrap();
            SyncState::seal_document_name(document_id, &doc_key, &signing_key, &name)
        };
        let longest = "\u{1F600}".repeat(ambre_core::SealedName::MAX_CHARS);
        assert_eq!(
            seal(&longest).unwrap().as_bytes().len(),
            ambre_core::SealedName::MAX_LEN
        );
        assert_eq!(
            seal("").unwrap().as_bytes().len(),
            ambre_core::SealedName::OVERHEAD
        );
        let past = "a".repeat(ambre_core::SealedName::MAX_CHARS + 1);
        assert_eq!(ambre_core::Name::new(past), Err(ambre_core::NameTooLong));
    }

    /// A sealed name that is no text, or that runs past the cap, opens to no
    /// name. A member who seals one by hand gets the same answer as a forgery.
    #[test]
    fn a_sealed_name_that_is_no_text_or_past_the_cap_opens_to_none() {
        let document_id = Id::<Document>::mint();
        let doc_key = DocumentKey::generate();
        let signing_key = MasterSeed::from_array([5; 32]).derive_signing_key();
        let seal = |bytes: Vec<u8>| {
            let sealed = pipeline::seal_object(
                &ambre_core::SecretBytes::new(bytes),
                &pipeline::name_aad(document_id),
                &doc_key,
                &signing_key,
            )
            .unwrap();
            sealed.into_name()
        };
        let at_cap = "a".repeat(ambre_core::NAME_MAX_CHARS);
        let past = "a".repeat(ambre_core::NAME_MAX_CHARS + 1);
        let open =
            |bytes: &[u8]| open_name(document_id, &doc_key, &seal(bytes.to_vec()), Vec::new());
        assert_eq!(
            open(at_cap.as_bytes())
                .as_ref()
                .map(ambre_core::Name::as_str),
            Some(at_cap.as_str())
        );
        assert_eq!(open(past.as_bytes()), None);
        assert_eq!(open(&[0xff, 0xfe]), None);
    }

    /// A name sealed three rotations ago opens with the current key and the
    /// chain, and stays closed without the chain (`SYNC-042`).
    #[test]
    fn a_name_sealed_before_a_rotation_opens_through_the_chain() {
        let document_id = Id::<Document>::mint();
        let signing_key = MasterSeed::from_array([5; 32]).derive_signing_key();
        let keys: Vec<_> = (0..4).map(|_| DocumentKey::generate()).collect();
        let links: Vec<_> = keys
            .windows(2)
            .map(|pair| pipeline::seal_link(document_id, &pair[0], &pair[1], &signing_key))
            .collect::<Result<_, _>>()
            .unwrap();
        let name = SyncState::seal_document_name(
            document_id,
            &keys[0],
            &signing_key,
            &ambre_core::Name::new("Budget").unwrap(),
        )
        .unwrap();
        let current = &keys[3];
        assert_eq!(
            open_name(document_id, current, &name, links.clone())
                .as_ref()
                .map(ambre_core::Name::as_str),
            Some("Budget")
        );
        assert_eq!(open_name(document_id, current, &name, Vec::new()), None);
        // Another document's chain opens nothing here.
        assert_eq!(
            open_name(Id::<Document>::mint(), current, &name, links),
            None
        );
    }

    /// A create answer that names another document, or no document, fails:
    /// only the minted id opens the name sealed under it (`SYNC-136`).
    #[test]
    fn a_create_fails_on_a_foreign_answer() {
        let minted = Id::<Document>::mint();
        let answer = SyncState::created_document(minted, minted);
        assert_eq!(answer.ok(), Some(minted));
        assert!(matches!(
            SyncState::created_document(minted, Id::<Document>::mint()),
            Err(crate::error::SyncError::WrongDocument { .. })
        ));
        let read = |id: String| {
            let body = serde_json::json!({ "document_id": id, "doc_type": "prose" });
            serde_json::from_value::<crate::http::CreateDocumentResponse>(body)
        };
        assert_eq!(
            read(minted.to_string()).map(|r| r.document_id).ok(),
            Some(minted)
        );
        assert!(read("not an id".into()).is_err());
    }
}
