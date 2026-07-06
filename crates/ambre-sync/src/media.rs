use super::*;

impl SyncState {
    // -- Media (upload / download via presigned S3 URLs) -----------------------

    /// Uploads a media blob to S3 via the presigned-URL flow:
    /// 1. Hashes the plaintext data (BLAKE2b-256 -> content_hash).
    /// 2. Encrypts the data with the document key (AEAD, "ambre-media" aad).
    /// 3. Requests a presigned S3 PUT URL from the server.
    /// 4. PUTs the encrypted blob (nonce || ciphertext) directly to S3.
    /// 5. Confirms the upload via `POST /api/documents/{id}/media`.
    /// 6. Emits [`SyncEvent::MediaUploaded`].
    pub(crate) fn handle_upload_media(
        &self,
        id: RequestId,
        document_id: DocumentId,
        data: Vec<u8>,
        mime_type: String,
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
            let content_hash = {
                let mut hasher = blake2::Blake2b512::new();
                hasher.update(&data);
                hex::encode(&hasher.finalize()[..32])
            };
            self.reply(
                id,
                RestEvent::MediaUploaded {
                    document_id,
                    content_hash,
                },
            );
            return;
        };
        let Some(doc_key) = doc_key else {
            let _ = logic_tx.send(LogicEvent::ApiError {
                request: "UploadMedia".into(),
                code: 404,
                message: "Document not open".into(),
            });
            return;
        };

        wasm_bindgen_futures::spawn_local(async move {
            let (nonce, ciphertext) = ambre_crypto::aead::seal(&data, b"ambre-media", &doc_key);
            let encrypted_size = (ambre_crypto::aead::NONCE_LEN + ciphertext.len()) as u64;

            let content_hash = {
                let mut hasher = blake2::Blake2b512::new();
                hasher.update(&data);
                hex::encode(&hasher.finalize()[..32])
            };

            let upload_url_req =
                format!("{url_scheme}://{api_base}/api/documents/{document_id}/media/upload-url");
            let body = http::UploadUrlRequest {
                content_hash: content_hash.clone(),
                mime_type: mime_type.clone(),
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
                        request: "UploadMedia".into(),
                        code: 500,
                        message: format!("upload-url: {e}"),
                    });
                    return;
                }
            };

            let mut blob = nonce.to_vec();
            blob.extend_from_slice(&ciphertext);
            // S3 presigned PUT goes straight to object storage, not our API:
            // no session, no CSRF header.
            if let Err(e) = http::put_bytes(&upload_url, &blob, "application/octet-stream").await {
                let _ = logic_tx.send(LogicEvent::ApiError {
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
                mime_type,
                size_bytes: encrypted_size as i64,
            };
            if let Err(e) =
                http::post_json_auth::<_, serde_json::Value>(&confirm_url, &confirm_body, auth)
                    .await
            {
                let _ = logic_tx.send(LogicEvent::ApiError {
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
    /// 3. Splits the nonce (first [`ambre_crypto::aead::NONCE_LEN`] bytes) from
    ///    the ciphertext and decrypts with the document key.
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
                        request: "DownloadMedia".into(),
                        code: 500,
                        message: format!("S3 GET: {e}"),
                    });
                    return;
                }
            };

            if blob.len() < ambre_crypto::aead::NONCE_LEN {
                let _ = logic_tx.send(LogicEvent::ApiError {
                    request: "DownloadMedia".into(),
                    code: 500,
                    message: "Blob too short".into(),
                });
                return;
            }
            let (nonce_bytes, ciphertext) = blob.split_at(ambre_crypto::aead::NONCE_LEN);
            let nonce: [u8; ambre_crypto::aead::NONCE_LEN] =
                nonce_bytes.try_into().expect("nonce len");

            let data = match ambre_crypto::aead::open(&nonce, ciphertext, b"ambre-media", &doc_key)
            {
                Ok(d) => d,
                Err(e) => {
                    let _ = logic_tx.send(LogicEvent::ApiError {
                        request: "DownloadMedia".into(),
                        code: 500,
                        message: format!("Decrypt: {e}"),
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
