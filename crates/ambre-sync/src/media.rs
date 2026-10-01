use super::*;

/// Additional data of a sealed blob. One constant: binding a blob to its
/// document and its address is `SYNC-051` and `SYNC-074`.
const MEDIA_AAD: &[u8] = b"ambre-media";

impl SyncState {
    // -- Media (upload / download via presigned S3 URLs) -----------------------

    /// Uploads a media blob to S3 via the presigned-URL flow:
    /// 1. Computes the address, keyed by the document key (`SYNC-050`).
    /// 2. Seals the data with the full envelope: the cipher, the commitment
    ///    tag and the author signature (`SYNC-018`).
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
        let content_hash = hex::encode(ambre_crypto::commitment::media_address(&doc_key, &data));
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
            let blob = pipeline::seal_object(&data, MEDIA_AAD, &doc_key, &signing_key)
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
    ///    then the cipher under the document key ([`pipeline::open_object`]).
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

            let data = match pipeline::open_object(&blob, MEDIA_AAD, &doc_key) {
                Ok(d) => d,
                Err(e) => {
                    let _ = logic_tx.send(LogicEvent::ApiError {
                        id: Some(id),
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
}
