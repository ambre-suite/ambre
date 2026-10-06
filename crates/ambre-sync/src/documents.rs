use super::*;

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
                let url = format!("{url_scheme}://{api_base}/api/documents");
                match http::get_json_auth::<http::DocumentListResponse>(&url, auth.clone()).await {
                    Ok(resp) => {
                        let mut docs = Vec::with_capacity(resp.documents.len());
                        for e in resp.documents {
                            let document_id =
                                DocumentId::parse_str(&e.id).unwrap_or_else(|_| DocumentId::new());
                            // E2EE: the name arrives as `name_enc`. Decrypt it
                            // here (Sync) so the Logic/UI threads only ever see
                            // plaintext (`ambre-spec-launcher.md`, CHROME-044). A
                            // failed unwrap/decrypt renders as `None` ("Untitled").
                            let name = match (e.name_enc.as_deref(), exchange_key.as_ref()) {
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
                                doc_type: match e.doc_type.as_str() {
                                    "prose" => DocType::Prose,
                                    "toile" => DocType::Toile,
                                    "murmure" => DocType::Murmure,
                                    _ => DocType::Trame,
                                },
                                name,
                                role: match e.role.as_str() {
                                    "editor" => Role::Editor,
                                    "viewer" => Role::Viewer,
                                    _ => Role::Owner,
                                },
                                // RFC3339 -> epoch ms via the JS Date parser
                                // (NaN -> 0 when the server timestamp is absent).
                                updated_at: {
                                    let ms = js_sys::Date::parse(&e.updated_at);
                                    if ms.is_nan() { 0 } else { ms as u64 }
                                },
                                member_count: e.member_count.max(1),
                            });
                        }
                        let _ = tx.send(SyncEvent::DocumentList {
                            id,
                            documents: docs,
                        });
                    }
                    Err(e) => {
                        let _ = tx.send(SyncEvent::HttpError { id, message: e });
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
                    updated_at: 0,
                    member_count: 1,
                })
                .collect();
            let count = documents.len();
            self.reply(id, RestEvent::DocumentList { documents });
            tracing::info!("Sync: ListDocuments ({count} docs)");
        }
    }

    /// Seals a document name with the full envelope, bound to its document
    /// (`SYNC-018`, `SYNC-068`), and encodes it as base64 for the relay.
    pub(crate) fn seal_document_name(
        document_id: DocumentId,
        doc_key: &DocumentKey,
        signing_key: &SigningKey,
        name: &str,
    ) -> Result<String, pipeline::PipelineError> {
        let sealed = pipeline::seal_object(
            name.as_bytes(),
            &pipeline::name_aad(document_id),
            doc_key,
            signing_key,
        )?;
        Ok(base64::engine::general_purpose::URL_SAFE.encode(sealed))
    }

    /// The document a create answer names, refused unless the client minted
    /// it. The name is sealed under that id, and no other id opens it
    /// (`SYNC-136`).
    fn created_document(minted: DocumentId, answered: &str) -> Result<DocumentId, String> {
        match DocumentId::parse_str(answered) {
            Ok(answered) if answered == minted => Ok(minted),
            _ => Err(format!(
                "CreateDocument: the relay answered {answered}, not the minted {minted}"
            )),
        }
    }

    pub(crate) fn handle_create_document(
        &mut self,
        id: RequestId,
        doc_type: DocType,
        name: String,
    ) {
        let event_tx = self.event_tx.clone();

        if let Some(tx) = event_tx {
            let api_base = self.api_base.clone();
            let url_scheme = self.url_scheme;
            let auth = self.auth_context();
            let exchange_key = self.exchange_key.clone();
            let signing_key = self.signing_key.clone();
            wasm_bindgen_futures::spawn_local(async move {
                // E2EE: the server only ever stores ciphertext. Generate a fresh
                // DocumentKey, seal the name with it, and wrap the key for the
                // owner (ourselves) via X25519. We can then recover it on open.
                let (Some(exchange_key), Some(signing_key)) = (exchange_key, signing_key) else {
                    let _ = tx.send(SyncEvent::HttpError {
                        id,
                        message: "CreateDocument: session locked (no session keys)".into(),
                    });
                    return;
                };
                let doc_key = DocumentKey::generate();
                // The client mints the id, so the name is bound to its document
                // before the relay has seen either (`SYNC-068`).
                let document_id = DocumentId::new();

                let name_enc = if name.is_empty() {
                    None
                } else {
                    Some(
                        Self::seal_document_name(document_id, &doc_key, &signing_key, &name)
                            .expect("seal doc name"),
                    )
                };

                let own_pub = x25519_dalek::PublicKey::from(&exchange_key);
                let wrapped = ambre_crypto::ecdh::wrap_document_key(
                    &exchange_key,
                    own_pub.as_bytes(),
                    &doc_key,
                )
                .expect("wrap doc key for self");
                let mut enc = wrapped.nonce.to_vec();
                enc.extend_from_slice(&wrapped.ciphertext);
                let doc_key_enc = base64::engine::general_purpose::URL_SAFE.encode(&enc);

                let url = format!("{url_scheme}://{api_base}/api/documents");
                let body = http::CreateDocumentRequest {
                    doc_type: doc_type_to_api_string(doc_type),
                    name_enc,
                    doc_key_enc,
                    document_id: Some(document_id.to_string()),
                };
                match http::post_json_auth::<_, http::CreateDocumentResponse>(&url, &body, auth)
                    .await
                {
                    Ok(resp) => match Self::created_document(document_id, &resp.document_id) {
                        Ok(document_id) => {
                            let _ = tx.send(SyncEvent::DocumentCreated {
                                id,
                                document_id,
                                doc_type,
                            });
                        }
                        Err(message) => {
                            let _ = tx.send(SyncEvent::HttpError { id, message });
                        }
                    },
                    Err(e) => {
                        let _ = tx.send(SyncEvent::HttpError { id, message: e });
                    }
                }
            });
        } else {
            tracing::info!("Sync: CreateDocument {doc_type:?} '{name}'");
            let document_id = DocumentId::new();
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

    pub(crate) fn handle_delete_document(&mut self, id: RequestId, document_id: DocumentId) {
        let event_tx = self.event_tx.clone();

        if let Some(tx) = event_tx {
            let api_base = self.api_base.clone();
            let url_scheme = self.url_scheme;
            let auth = self.auth_context();
            let doc_id = document_id;
            wasm_bindgen_futures::spawn_local(async move {
                let url = format!("{url_scheme}://{api_base}/api/documents/{doc_id}");
                match http::delete_auth(&url, auth).await {
                    Ok(()) => {
                        let _ = tx.send(SyncEvent::DocumentDeleted {
                            id,
                            document_id: doc_id,
                        });
                    }
                    Err(e) => {
                        let _ = tx.send(SyncEvent::HttpError { id, message: e });
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
        document_id: DocumentId,
        name: String,
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
                    let _ = tx.send(SyncEvent::HttpError {
                        id,
                        message: "RenameDocument: session locked (no signing key)".into(),
                    });
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
                            let _ = tx.send(SyncEvent::HttpError {
                                id,
                                message: "RenameDocument: session locked (no exchange key)".into(),
                            });
                            return;
                        };
                        let members_url =
                            format!("{url_scheme}://{api_base}/api/documents/{doc_id}/members");
                        match http::get_json_auth::<http::MembersResponse>(
                            &members_url,
                            auth.clone(),
                        )
                        .await
                        {
                            Ok(resp) => match Self::unwrap_document_key(&ek, &resp.members) {
                                Ok(k) => k,
                                Err(e) => {
                                    let _ = tx.send(SyncEvent::HttpError {
                                        id,
                                        message: format!("RenameDocument: key unwrap failed: {e}"),
                                    });
                                    return;
                                }
                            },
                            Err(e) => {
                                let _ = tx.send(SyncEvent::HttpError { id, message: e });
                                return;
                            }
                        }
                    }
                };
                let name_enc = Self::seal_document_name(doc_id, &doc_key, &signing_key, &name)
                    .expect("seal doc name");

                // PATCH /api/documents/{id} (matches the server route).
                let url = format!("{url_scheme}://{api_base}/api/documents/{doc_id}");
                let body = http::RenameDocumentRequest { name_enc };
                match http::patch_json_auth(&url, &body, auth).await {
                    Ok(()) => {
                        let _ = tx.send(SyncEvent::DocumentRenamed {
                            id,
                            document_id: doc_id,
                        });
                    }
                    Err(e) => {
                        let _ = tx.send(SyncEvent::HttpError { id, message: e });
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

/// One sealed name of the document list, with the wrapped key that opens it.
/// Keys are base64 of `nonce || ciphertext`; the wrapper's exchange key is
/// hex, empty for a self-wrapped owner row.
struct ListedName<'a> {
    document_id: DocumentId,
    doc_key_enc: &'a str,
    wrapper_exchange_pub_key: &'a str,
    name_enc: &'a str,
}

impl ListedName<'_> {
    /// The plaintext name. A name sealed before the last rotation names an
    /// older key, which the chain recovers (`SYNC-042`, `SYNC-119`). `None` on
    /// any decode or crypto failure: the launcher shows "Untitled".
    async fn open(
        &self,
        exchange_key: &x25519_dalek::StaticSecret,
        url_scheme: &str,
        api_base: &str,
        auth: &http::AuthContext,
    ) -> Option<String> {
        let current = self.unwrap_key(exchange_key)?;
        let bytes = self.sealed()?;
        // One request more, and only for a name older than the current key.
        let links = if pipeline::object_key(&bytes).ok()? == current.id() {
            Vec::new()
        } else {
            http::fetch_key_links(url_scheme, api_base, self.document_id, auth.clone()).await
        };
        open_name(self.document_id, &current, &bytes, links)
    }

    /// The current key, ECDH-unwrapped against the wrapper's exchange key.
    fn unwrap_key(&self, exchange_key: &x25519_dalek::StaticSecret) -> Option<DocumentKey> {
        let wrapped_bytes = base64::engine::general_purpose::URL_SAFE
            .decode(self.doc_key_enc)
            .ok()?;
        if wrapped_bytes.len() < 24 {
            return None;
        }
        let nonce: [u8; 24] = wrapped_bytes[..24].try_into().ok()?;
        let wrapped = ambre_crypto::WrappedKey {
            nonce,
            ciphertext: wrapped_bytes[24..].to_vec(),
        };
        let wrapper_hex = if self.wrapper_exchange_pub_key.is_empty() {
            // Self-wrapped owner row: the DH peer is our own public key.
            hex::encode(x25519_dalek::PublicKey::from(exchange_key).as_bytes())
        } else {
            self.wrapper_exchange_pub_key.to_owned()
        };
        let peer_pub: [u8; 32] = hex::decode(wrapper_hex).ok()?.try_into().ok()?;
        ambre_crypto::ecdh::unwrap_document_key(exchange_key, &peer_pub, &wrapped).ok()
    }

    fn sealed(&self) -> Option<Vec<u8>> {
        base64::engine::general_purpose::URL_SAFE
            .decode(self.name_enc)
            .ok()
    }
}

/// Opens a sealed name with the key it names: `current`, or one the chain
/// `links` reach from it. `None` on any failure: a name sealed for another
/// document, a forged author, a key out of reach.
fn open_name(
    document_id: DocumentId,
    current: &DocumentKey,
    bytes: &[u8],
    links: Vec<ambre_protocol::KeyLink>,
) -> Option<String> {
    let chain = crate::chain::Chain::new(document_id, links);
    let wanted = pipeline::object_key(bytes).ok()?;
    let key = crate::chain::find_key(document_id, current, &[], &chain, wanted)?;
    let plaintext = pipeline::open_object(bytes, &pipeline::name_aad(document_id), &key).ok()?;
    String::from_utf8(plaintext).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ambre_crypto::MasterSeed;

    /// A name sealed three rotations ago opens with the current key and the
    /// chain, and stays closed without the chain (`SYNC-042`).
    #[test]
    fn a_name_sealed_before_a_rotation_opens_through_the_chain() {
        let document_id = DocumentId::new();
        let signing_key = MasterSeed::from_array([5; 32]).derive_signing_key();
        let keys: Vec<_> = (0..4).map(|_| DocumentKey::generate()).collect();
        let links: Vec<_> = keys
            .windows(2)
            .map(|pair| pipeline::seal_link(document_id, &pair[0], &pair[1], &signing_key))
            .collect::<Result<_, _>>()
            .unwrap();
        let name =
            SyncState::seal_document_name(document_id, &keys[0], &signing_key, "Budget").unwrap();
        let bytes = base64::engine::general_purpose::URL_SAFE
            .decode(name)
            .unwrap();
        let current = &keys[3];
        assert_eq!(
            open_name(document_id, current, &bytes, links.clone()).as_deref(),
            Some("Budget")
        );
        assert_eq!(open_name(document_id, current, &bytes, Vec::new()), None);
        // Another document's chain opens nothing here.
        assert_eq!(open_name(DocumentId::new(), current, &bytes, links), None);
    }

    /// A create answer that names another document, or no document, fails:
    /// only the minted id opens the name sealed under it (`SYNC-136`).
    #[test]
    fn a_create_fails_on_a_foreign_answer() {
        let minted = DocumentId::new();
        let answer = SyncState::created_document(minted, &minted.to_string());
        assert_eq!(answer, Ok(minted));
        let foreign = DocumentId::new().to_string();
        assert!(SyncState::created_document(minted, &foreign).is_err());
        assert!(SyncState::created_document(minted, "not an id").is_err());
    }
}
