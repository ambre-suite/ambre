use super::*;

type Address = [u8; ambre_crypto::commitment::TAG_LEN];

/// Seals a blob for one document and the address it is stored under
/// (`SYNC-051`, `SYNC-074`).
fn seal_blob(
    data: &[u8],
    document_id: DocumentId,
    address: &Address,
    doc_key: &DocumentKey,
    signing_key: &SigningKey,
) -> Result<Vec<u8>, pipeline::PipelineError> {
    pipeline::seal_object(
        data,
        &pipeline::media_aad(document_id, address),
        doc_key,
        signing_key,
    )
}

/// Opens a downloaded blob, refused unless it is the one the address names
/// (`SYNC-052`). The additional data refuses a blob sealed elsewhere. The
/// recomputed address refuses one a member sealed under this address over
/// other bytes.
fn open_blob(
    blob: &[u8],
    document_id: DocumentId,
    content_hash: &str,
    doc_key: &DocumentKey,
) -> Result<Vec<u8>, String> {
    let address: Address = hex::decode(content_hash)
        .ok()
        .and_then(|bytes| bytes.try_into().ok())
        .ok_or_else(|| format!("not an address: {content_hash}"))?;
    let data = pipeline::open_object(blob, &pipeline::media_aad(document_id, &address), doc_key)
        .map_err(|e| e.to_string())?;
    if ambre_crypto::commitment::media_address(doc_key, &data) != address {
        return Err(format!("the blob is not the one {content_hash} names"));
    }
    Ok(data)
}

impl SyncState {
    // -- Media (upload / download via presigned S3 URLs) -----------------------

    /// Uploads a media blob to S3 via the presigned-URL flow:
    /// 1. Computes the address, keyed by the document key (`SYNC-050`).
    /// 2. Seals the data with the full envelope: the cipher, the commitment
    ///    tag and the author signature (`SYNC-018`), bound to the document and
    ///    the address ([`seal_blob`]).
    /// 3. Requests a presigned S3 PUT URL from the server.
    /// 4. PUTs the sealed blob ([`pipeline::seal_object`]) directly to S3.
    /// 5. Confirms the upload via `POST /api/documents/{id}/media`, naming the
    ///    address and the sealed size only (`SYNC-053`).
    /// 6. Emits [`SyncEvent::MediaUploaded`].
    pub(crate) fn handle_upload_media(
        &self,
        id: RequestId,
        document_id: DocumentId,
        data: Vec<u8>,
    ) {
        let event_tx = self.event_tx.clone();
        let logic_tx = self.logic_tx.clone();
        let api_base = self.api_base.clone();
        let url_scheme = self.url_scheme;
        let auth = self.auth_context();
        let doc_key = self
            .documents
            .get(&document_id)
            .map(|ds| ds.doc_key.clone());

        let Some(doc_key) = doc_key else {
            let _ = logic_tx.send(LogicEvent::ApiError {
                id: Some(id),
                request: "UploadMedia".into(),
                code: 404,
                message: "Document not open".into(),
            });
            return;
        };
        let address = ambre_crypto::commitment::media_address(&doc_key, &data);
        let content_hash = hex::encode(address);
        let Some(tx) = event_tx else {
            self.reply(
                id,
                RestEvent::MediaUploaded {
                    document_id,
                    content_hash,
                },
            );
            return;
        };
        let Some(signing_key) = self.signing_key.clone() else {
            let _ = logic_tx.send(LogicEvent::ApiError {
                id: Some(id),
                request: "UploadMedia".into(),
                code: 401,
                message: "Session locked".into(),
            });
            return;
        };

        wasm_bindgen_futures::spawn_local(async move {
            let blob = seal_blob(&data, document_id, &address, &doc_key, &signing_key)
                .expect("seal media");
            let encrypted_size = blob.len() as u64;

            let upload_url_req =
                format!("{url_scheme}://{api_base}/api/documents/{document_id}/media/upload-url");
            let body = http::UploadUrlRequest {
                content_hash: content_hash.clone(),
                size_bytes: encrypted_size,
            };
            let upload_url = match http::post_json_auth::<_, http::UploadUrlResponse>(
                &upload_url_req,
                &body,
                auth.clone(),
            )
            .await
            {
                Ok(resp) => resp.upload_url,
                Err(e) => {
                    let _ = logic_tx.send(LogicEvent::ApiError {
                        id: Some(id),
                        request: "UploadMedia".into(),
                        code: 500,
                        message: format!("upload-url: {e}"),
                    });
                    return;
                }
            };

            // S3 presigned PUT goes straight to object storage, not our API:
            // no session, no CSRF header.
            if let Err(e) = http::put_bytes(&upload_url, &blob, "application/octet-stream").await {
                let _ = logic_tx.send(LogicEvent::ApiError {
                    id: Some(id),
                    request: "UploadMedia".into(),
                    code: 500,
                    message: format!("S3 PUT: {e}"),
                });
                return;
            }

            let confirm_url =
                format!("{url_scheme}://{api_base}/api/documents/{document_id}/media");
            let confirm_body = http::ConfirmUploadRequest {
                content_hash: content_hash.clone(),
                size_bytes: encrypted_size as i64,
            };
            if let Err(e) =
                http::post_json_auth::<_, serde_json::Value>(&confirm_url, &confirm_body, auth)
                    .await
            {
                let _ = logic_tx.send(LogicEvent::ApiError {
                    id: Some(id),
                    request: "UploadMedia".into(),
                    code: 500,
                    message: format!("confirm: {e}"),
                });
                return;
            }

            let _ = tx.send(SyncEvent::MediaUploaded {
                id,
                document_id,
                content_hash,
            });
        });
    }

    /// Downloads a media blob from S3 via the presigned-URL flow:
    /// 1. Requests a presigned S3 GET URL from the server by content_hash.
    /// 2. GETs the encrypted blob directly from S3.
    /// 3. Opens it in the envelope's order: the signature, the commitment tag,
    ///    then the cipher under the document key, and recomputes the address
    ///    ([`open_blob`]).
    /// 4. Emits [`SyncEvent::MediaDownloaded`] with the plaintext data.
    pub(crate) fn handle_download_media(
        &self,
        id: RequestId,
        document_id: DocumentId,
        content_hash: String,
    ) {
        let event_tx = self.event_tx.clone();
        let logic_tx = self.logic_tx.clone();
        let api_base = self.api_base.clone();
        let url_scheme = self.url_scheme;
        let auth = self.auth_context();
        let doc_key = self
            .documents
            .get(&document_id)
            .map(|ds| ds.doc_key.clone());

        let Some(tx) = event_tx else {
            self.reply(
                id,
                RestEvent::MediaDownloaded {
                    document_id,
                    content_hash,
                    data: Vec::new(),
                },
            );
            return;
        };
        let Some(doc_key) = doc_key else {
            let _ = logic_tx.send(LogicEvent::ApiError {
                id: Some(id),
                request: "DownloadMedia".into(),
                code: 404,
                message: "Document not open".into(),
            });
            return;
        };

        wasm_bindgen_futures::spawn_local(async move {
            let download_url_req =
                format!("{url_scheme}://{api_base}/api/documents/{document_id}/media/download-url");
            let body = http::DownloadUrlRequest {
                content_hash: content_hash.clone(),
            };
            let download_url = match http::post_json_auth::<_, http::DownloadUrlResponse>(
                &download_url_req,
                &body,
                auth,
            )
            .await
            {
                Ok(resp) => resp.download_url,
                Err(e) => {
                    let _ = logic_tx.send(LogicEvent::ApiError {
                        id: Some(id),
                        request: "DownloadMedia".into(),
                        code: 500,
                        message: format!("download-url: {e}"),
                    });
                    return;
                }
            };

            let blob = match http::fetch_bytes(&download_url).await {
                Ok(b) => b,
                Err(e) => {
                    let _ = logic_tx.send(LogicEvent::ApiError {
                        id: Some(id),
                        request: "DownloadMedia".into(),
                        code: 500,
                        message: format!("S3 GET: {e}"),
                    });
                    return;
                }
            };

            let data = match open_blob(&blob, document_id, &content_hash, &doc_key) {
                Ok(d) => d,
                Err(e) => {
                    let _ = logic_tx.send(LogicEvent::ApiError {
                        id: Some(id),
                        request: "DownloadMedia".into(),
                        code: 500,
                        message: format!("Open: {e}"),
                    });
                    return;
                }
            };

            let _ = tx.send(SyncEvent::MediaDownloaded {
                id,
                document_id,
                content_hash,
                data,
            });
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The address the worker answers with is keyed by the document key: the
    /// same bytes in two documents get two addresses (`SYNC-050`).
    #[test]
    fn an_upload_is_addressed_under_the_document_key() {
        let (sink, rx) = LogicSink::channel();
        let mut state = SyncState::new(sink, String::new(), None);
        let picture = b"one picture, two documents".to_vec();
        let mut addresses = Vec::new();
        for byte in [0x31, 0x32] {
            let document_id = DocumentId::new();
            let key = DocumentKey::from_array([byte; 32]);
            let doc_state = SyncDocumentState::new(document_id, key.clone(), DocType::Toile, true);
            state.documents.insert(document_id, doc_state);
            state.handle_upload_media(RequestId::next(), document_id, picture.clone());
            let answer = rx.try_iter().find_map(|event| match event {
                LogicEvent::Rest {
                    evt: RestEvent::MediaUploaded { content_hash, .. },
                    ..
                } => Some(content_hash),
                _ => None,
            });
            let address = answer.expect("the upload answered with an address");
            let expected = hex::encode(ambre_crypto::commitment::media_address(&key, &picture));
            assert_eq!(address, expected);
            addresses.push(address);
        }
        assert_ne!(addresses[0], addresses[1]);
    }

    fn sealed(data: &[u8], document_id: DocumentId, key: &DocumentKey) -> (Vec<u8>, String) {
        let signing = SigningKey::from_bytes(&[0x07; 32]);
        let address = ambre_crypto::commitment::media_address(key, data);
        let blob = seal_blob(data, document_id, &address, key, &signing).unwrap();
        (blob, hex::encode(address))
    }

    /// A blob sealed for one document does not open under another, even one
    /// that holds the same key (`SYNC-051`).
    #[test]
    fn a_blob_opens_under_its_own_document_only() {
        let key = DocumentKey::from_array([0x31; 32]);
        let own = DocumentId::new();
        let (blob, address) = sealed(b"a picture", own, &key);
        assert_eq!(open_blob(&blob, own, &address, &key).unwrap(), b"a picture");
        assert!(open_blob(&blob, DocumentId::new(), &address, &key).is_err());
    }

    /// A blob sealed under one address does not open under another address
    /// of the same document (`SYNC-074`).
    #[test]
    fn a_blob_opens_under_its_own_address_only() {
        let key = DocumentKey::from_array([0x31; 32]);
        let document_id = DocumentId::new();
        let (blob, _) = sealed(b"a picture", document_id, &key);
        let (_, other) = sealed(b"another picture", document_id, &key);
        assert!(open_blob(&blob, document_id, &other, &key).is_err());
    }

    /// A download refuses a blob the address does not name. One is served in
    /// place of another. The other is sealed by a member under the address,
    /// over other bytes, and the additional data alone accepts it (`SYNC-052`).
    #[test]
    fn a_download_refuses_a_blob_the_address_does_not_name() {
        let key = DocumentKey::from_array([0x31; 32]);
        let document_id = DocumentId::new();
        let (_, wanted) = sealed(b"a picture", document_id, &key);
        let (served, _) = sealed(b"another picture", document_id, &key);
        assert!(open_blob(&served, document_id, &wanted, &key).is_err());

        let address = ambre_crypto::commitment::media_address(&key, b"a picture");
        let signing = SigningKey::from_bytes(&[0x07; 32]);
        let forged = seal_blob(b"other bytes", document_id, &address, &key, &signing).unwrap();
        assert_eq!(
            pipeline::open_object(&forged, &pipeline::media_aad(document_id, &address), &key)
                .unwrap(),
            b"other bytes"
        );
        assert!(open_blob(&forged, document_id, &wanted, &key).is_err());
        assert!(open_blob(&forged, document_id, "not hex", &key).is_err());
    }
}
