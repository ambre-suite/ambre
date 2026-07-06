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
                match http::get_json_auth::<http::DocumentListResponse>(&url, auth).await {
                    Ok(resp) => {
                        let docs: Vec<DocumentSummary> = resp
                            .documents
                            .into_iter()
                            .map(|e| {
                                // E2EE: the name arrives as `name_enc`. Decrypt it
                                // here (Sync) so the Logic/UI threads only ever see
                                // plaintext (`ambre-spec-chrome.md 2.G` ZK note). A
                                // failed unwrap/decrypt renders as `None` ("Untitled").
                                let name = e.name_enc.as_deref().and_then(|name_enc| {
                                    exchange_key.as_ref().and_then(|ek| {
                                        Self::decrypt_document_name(
                                            ek,
                                            &e.doc_key_enc,
                                            &e.wrapper_exchange_pub_key,
                                            name_enc,
                                        )
                                    })
                                });
                                DocumentSummary {
                                    document_id: DocumentId::parse_str(&e.id)
                                        .unwrap_or_else(|_| DocumentId::new()),
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
                                }
                            })
                            .collect();
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

    /// Unwraps the DocumentKey from `doc_key_enc` (ECDH with the wrapper's
    /// exchange pubkey) and decrypts `name_enc` to plaintext. All inputs are
    /// base64 (keys) / base64 (name) blobs of `nonce || ciphertext`; the wrapper
    /// pubkey is hex, empty meaning "self-wrapped" (fall back to our own key).
    /// Returns `None` on any decode/crypto failure (the launcher shows "Untitled").
    pub(crate) fn decrypt_document_name(
        exchange_key: &x25519_dalek::StaticSecret,
        doc_key_enc: &str,
        wrapper_exchange_pub_key: &str,
        name_enc: &str,
    ) -> Option<String> {
        // 1. ECDH-unwrap the DocumentKey.
        let wrapped_bytes = base64::engine::general_purpose::URL_SAFE
            .decode(doc_key_enc)
            .ok()?;
        if wrapped_bytes.len() < 24 {
            return None;
        }
        let nonce: [u8; 24] = wrapped_bytes[..24].try_into().ok()?;
        let wrapped = ambre_crypto::WrappedKey {
            nonce,
            ciphertext: wrapped_bytes[24..].to_vec(),
        };
        let wrapper_hex = if wrapper_exchange_pub_key.is_empty() {
            // Self-wrapped owner row: the DH peer is our own public key.
            hex::encode(x25519_dalek::PublicKey::from(exchange_key).as_bytes())
        } else {
            wrapper_exchange_pub_key.to_owned()
        };
        let peer_pub: [u8; 32] = hex::decode(wrapper_hex).ok()?.try_into().ok()?;
        let doc_key =
            ambre_crypto::ecdh::unwrap_document_key(exchange_key, &peer_pub, &wrapped).ok()?;

        // 2. AEAD-open the name (same AAD as the seal in create/rename).
        let name_bytes = base64::engine::general_purpose::URL_SAFE
            .decode(name_enc)
            .ok()?;
        if name_bytes.len() < 24 {
            return None;
        }
        let name_nonce: [u8; 24] = name_bytes[..24].try_into().ok()?;
        let plaintext =
            ambre_crypto::aead::open(&name_nonce, &name_bytes[24..], b"ambre-doc-name", &doc_key)
                .ok()?;
        String::from_utf8(plaintext).ok()
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
            wasm_bindgen_futures::spawn_local(async move {
                // E2EE: the server only ever stores ciphertext. Generate a fresh
                // DocumentKey, seal the name with it, and wrap the key for the
                // owner (ourselves) via X25519 so we can recover it on open.
                let Some(exchange_key) = exchange_key else {
                    let _ = tx.send(SyncEvent::HttpError {
                        id,
                        message: "CreateDocument: session locked (no exchange key)".into(),
                    });
                    return;
                };
                let doc_key = DocumentKey::generate();

                let name_enc = if name.is_empty() {
                    None
                } else {
                    let (nonce, ct) =
                        ambre_crypto::aead::seal(name.as_bytes(), b"ambre-doc-name", &doc_key);
                    let mut buf = nonce.to_vec();
                    buf.extend_from_slice(&ct);
                    Some(base64::engine::general_purpose::URL_SAFE.encode(&buf))
                };

                let own_pub = x25519_dalek::PublicKey::from(&exchange_key);
                let wrapped = ambre_crypto::ecdh::wrap_document_key(
                    &exchange_key,
                    own_pub.as_bytes(),
                    &doc_key,
                );
                let mut enc = wrapped.nonce.to_vec();
                enc.extend_from_slice(&wrapped.ciphertext);
                let doc_key_enc = base64::engine::general_purpose::URL_SAFE.encode(&enc);

                let url = format!("{url_scheme}://{api_base}/api/documents");
                let body = http::CreateDocumentRequest {
                    doc_type: doc_type_to_api_string(doc_type),
                    name_enc,
                    doc_key_enc,
                    document_id: None,
                };
                match http::post_json_auth::<_, http::CreateDocumentResponse>(&url, &body, auth)
                    .await
                {
                    Ok(resp) => {
                        let document_id = DocumentId::parse_str(&resp.document_id)
                            .unwrap_or_else(|_| DocumentId::new());
                        let _ = tx.send(SyncEvent::DocumentCreated {
                            id,
                            document_id,
                            doc_type,
                        });
                    }
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
            wasm_bindgen_futures::spawn_local(async move {
                // E2EE: the new name is sealed with the document key before it
                // leaves the client. The server only ever stores ciphertext.
                //
                // The doc is usually open (inline rename from its header), but the
                // launcher renames a closed doc: fall back to fetching the member
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
                let (nonce, ct) =
                    ambre_crypto::aead::seal(name.as_bytes(), b"ambre-doc-name", &doc_key);
                let mut buf = nonce.to_vec();
                buf.extend_from_slice(&ct);
                let name_enc = base64::engine::general_purpose::URL_SAFE.encode(&buf);

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
