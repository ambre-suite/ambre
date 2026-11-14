use super::*;
use crate::error::SyncError;
use crate::http::Reported;
use crate::sharing::GlooHttp;
use serde::Serialize;
use serde::de::DeserializeOwned;

use ambre_core::{FixedBytes, MediaAddress, Payload};

/// Seals a blob for one document and the address it is stored under
/// (`SYNC-051`, `SYNC-074`).
fn seal_blob(
    data: &ambre_core::Payload<ambre_core::MediaBytes>,
    document_id: Id<Document>,
    address: &FixedBytes<MediaAddress>,
    doc_key: &DocumentKey,
    signing_key: &SigningKey,
) -> Result<pipeline::SealedBlob, pipeline::PipelineError> {
    pipeline::object_blob(
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
    blob: &Payload<pipeline::FetchedBlob>,
    document_id: Id<Document>,
    content_hash: &ambre_core::Validated<ambre_core::ContentHash>,
    doc_key: &DocumentKey,
) -> Result<ambre_core::Payload<ambre_core::MediaBytes>, SyncError> {
    let address = FixedBytes::<MediaAddress>::of_hash(content_hash);
    let data = pipeline::open_object(blob, &pipeline::media_aad(document_id, &address), doc_key)?;
    // The media file keeps its bytes in the clear by role, so they move out
    // of the wrapper with no copy. A blob the address refuses is wiped.
    let data = ambre_core::Payload::new(data.into_plaintext());
    if ambre_crypto::commitment::media_address(doc_key, &data) != address {
        data.wipe();
        return Err(SyncError::BlobMismatch);
    }
    Ok(data)
}

/// Opens a downloaded blob with the one key it names: the current key, a
/// cached one, or one the chain recovers (`SYNC-042`, `SYNC-045`).
fn open_blob_named(
    blob: &Payload<pipeline::FetchedBlob>,
    content_hash: &ambre_core::Validated<ambre_core::ContentHash>,
    keyring: &crate::chain::Keyring,
) -> Result<ambre_core::Payload<ambre_core::MediaBytes>, SyncError> {
    let wanted = pipeline::object_key(blob)?;
    let key = keyring.find(wanted).ok_or(SyncError::KeyOutOfReach)?;
    open_blob(blob, keyring.document_id, content_hash, &key)
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
    ///
    /// A document the relay does not hold yet gets no link. The upload waits,
    /// and its creation starts: [`SyncState::flush_pending_uploads`] resumes it
    /// (`SYNC-145`).
    pub(crate) fn handle_upload_media(
        &mut self,
        id: Seq<Request>,
        document_id: Id<Document>,
        data: ambre_core::Payload<ambre_core::MediaBytes>,
    ) {
        if let Some(doc_state) = self
            .documents
            .get_mut(&document_id)
            .filter(|d| d.existence == crate::Existence::LocalOnly)
        {
            doc_state.pending_uploads.push((id, data));
            self.trigger_lazy_create(document_id);
            return;
        }
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
                request: ApiRequest::UploadMedia,
                failure: ApiFailure::Local(ambre_core::LocalFailure::DocumentNotOpen),
            });
            return;
        };
        let address = ambre_crypto::commitment::media_address(&doc_key, &data);
        let content_hash = address.content_hash();
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
                request: ApiRequest::UploadMedia,
                failure: ApiFailure::Local(ambre_core::LocalFailure::SessionLocked),
            });
            return;
        };

        let upload = Upload {
            id,
            document_id,
            data,
            address,
            doc_key,
            signing_key,
        };
        wasm_bindgen_futures::spawn_local(async move {
            let base = crate::http::Url::<crate::http::ApiRoot>::of(url_scheme, &api_base)
                .route(format!("/documents/{document_id}"));
            upload_media(&GlooHttp { auth }, &base, upload, &tx, &logic_tx).await;
        });
    }

    /// Resumes the uploads that waited for the document's lazy creation
    /// (`SYNC-145`). A creation that failed ends each of them with an error,
    /// so no request waits for good (`SYNC-146`).
    pub(crate) fn flush_pending_uploads(
        &mut self,
        document_id: Id<Document>,
        failure: Option<&http::Failed>,
    ) {
        let pending = self
            .documents
            .get_mut(&document_id)
            .map(|d| std::mem::take(&mut d.pending_uploads))
            .unwrap_or_default();
        for (id, data) in pending {
            match failure {
                None => self.handle_upload_media(id, document_id, data),
                Some(failed) => {
                    tracing::warn!("Sync: upload {id} waits on a document not created: {failed}");
                    self.send_to_logic(LogicEvent::ApiError {
                        id: Some(id),
                        request: ApiRequest::UploadMedia,
                        failure: failed.failure,
                    });
                }
            }
        }
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
        id: Seq<Request>,
        document_id: Id<Document>,
        content_hash: ambre_core::Validated<ambre_core::ContentHash>,
    ) {
        let event_tx = self.event_tx.clone();
        let logic_tx = self.logic_tx.clone();
        let api_base = self.api_base.clone();
        let url_scheme = self.url_scheme;
        let auth = self.auth_context();
        // A blob sealed before a rotation names an older key, which the
        // chain recovers (`SYNC-042`, `SYNC-119`).
        let keyring = self.documents.get(&document_id).map(|ds| ds.keyring());

        let Some(tx) = event_tx else {
            self.reply(
                id,
                RestEvent::MediaDownloaded {
                    document_id,
                    content_hash,
                    data: ambre_core::Payload::default(),
                },
            );
            return;
        };
        let Some(keyring) = keyring else {
            let _ = logic_tx.send(LogicEvent::ApiError {
                id: Some(id),
                request: ApiRequest::DownloadMedia,
                failure: ApiFailure::Local(ambre_core::LocalFailure::DocumentNotOpen),
            });
            return;
        };

        wasm_bindgen_futures::spawn_local(async move {
            let base = crate::http::Url::<crate::http::ApiRoot>::of(url_scheme, &api_base)
                .route(format!("/documents/{document_id}"));
            let http = GlooHttp { auth };
            download_media(&http, &base, id, content_hash, &keyring, &tx, &logic_tx).await;
        });
    }

    /// Erases this user's upload of a blob. It needs no key and no open
    /// document: the relay checks the uploader, and the object holds nothing
    /// it could read (`SRV-139`).
    pub(crate) fn handle_erase_media(
        &self,
        id: Seq<Request>,
        document_id: Id<Document>,
        content_hash: ambre_core::Validated<ambre_core::ContentHash>,
    ) {
        let Some(tx) = self.event_tx.clone() else {
            self.reply(
                id,
                RestEvent::MediaErased {
                    document_id,
                    content_hash,
                },
            );
            return;
        };
        let logic_tx = self.logic_tx.clone();
        let auth = self.auth_context();
        let base = crate::http::Url::<crate::http::ApiRoot>::of(self.url_scheme, &self.api_base)
            .route(format!("/documents/{document_id}"));
        wasm_bindgen_futures::spawn_local(async move {
            let http = GlooHttp { auth };
            erase_media(&http, &base, id, document_id, content_hash, &tx, &logic_tx).await;
        });
    }
}

/// The requests the two media legs make, abstracted over the transport so
/// that a host test drives each step failing.
pub(crate) trait MediaHttp {
    /// An authenticated request to the relay.
    async fn post_json<B: Serialize, R: DeserializeOwned>(
        &self,
        url: &crate::http::Url<http::Route>,
        body: &B,
    ) -> Result<R, http::HttpError>;
    /// A sealed blob to a presigned object-storage link.
    async fn put_sealed(
        &self,
        url: &ambre_core::Credential<ambre_core::PresignedUrl>,
        blob: &pipeline::SealedBlob,
    ) -> Result<(), http::HttpError>;
    /// A blob from a presigned object-storage link.
    async fn fetch_bytes(
        &self,
        url: &ambre_core::Credential<ambre_core::PresignedUrl>,
    ) -> Result<Payload<pipeline::FetchedBlob>, http::HttpError>;
    /// An authenticated `DELETE` to the relay, answering its status.
    async fn delete(
        &self,
        url: &crate::http::Url<http::Route>,
    ) -> Result<ambre_core::HttpStatus, http::HttpError>;
}

impl MediaHttp for GlooHttp {
    async fn post_json<B: Serialize, R: DeserializeOwned>(
        &self,
        url: &crate::http::Url<http::Route>,
        body: &B,
    ) -> Result<R, http::HttpError> {
        http::post_json_auth::<B, R>(url, body, self.auth.clone()).await
    }
    async fn put_sealed(
        &self,
        url: &ambre_core::Credential<ambre_core::PresignedUrl>,
        blob: &pipeline::SealedBlob,
    ) -> Result<(), http::HttpError> {
        http::put_sealed(url, blob).await
    }
    async fn fetch_bytes(
        &self,
        url: &ambre_core::Credential<ambre_core::PresignedUrl>,
    ) -> Result<Payload<pipeline::FetchedBlob>, http::HttpError> {
        http::fetch_bytes(url).await
    }
    async fn delete(
        &self,
        url: &crate::http::Url<http::Route>,
    ) -> Result<ambre_core::HttpStatus, http::HttpError> {
        http::delete_status_auth(url, self.auth.clone()).await
    }
}

/// Reports a failed media leg to the caller that asked for it.
fn report(logic_tx: &LogicSink, id: Seq<Request>, request: ApiRequest, error: impl http::Reported) {
    tracing::warn!("Sync: {} {id} failed: {error}", request.as_str());
    let _ = logic_tx.send(LogicEvent::ApiError {
        id: Some(id),
        request,
        failure: error.failure(),
    });
}

/// One upload a caller asked for: the plaintext, where it goes, and the keys
/// that seal it.
pub(crate) struct Upload {
    id: Seq<Request>,
    document_id: Id<Document>,
    data: ambre_core::Payload<ambre_core::MediaBytes>,
    address: FixedBytes<MediaAddress>,
    doc_key: DocumentKey,
    signing_key: SigningKey,
}

/// Seals and uploads one blob, then answers the caller (`SYNC-076`). A
/// success answers with the address. A failure at any step is reported, and
/// the success event is never sent.
async fn upload_media(
    http: &impl MediaHttp,
    base: &crate::http::Url<http::Route>,
    upload: Upload,
    tx: &ambre_bus::conduit::UnboundedSender<SyncEvent>,
    logic_tx: &LogicSink,
) {
    let Upload {
        id,
        document_id,
        data,
        address,
        doc_key,
        signing_key,
    } = upload;
    let content_hash = address.content_hash();
    let outcome = match seal_blob(&data, document_id, &address, &doc_key, &signing_key) {
        Ok(blob) => upload_leg(http, base, &content_hash, &blob).await,
        Err(e) => {
            tracing::warn!("Sync: seal: {e}");
            Err(http::Failed::local(ambre_core::LocalFailure::Crypto))
        }
    };
    match outcome {
        Ok(()) => {
            let _ = tx.send(SyncEvent::MediaUploaded {
                id,
                document_id,
                content_hash,
            });
        }
        Err(message) => report(logic_tx, id, ApiRequest::UploadMedia, message),
    }
}

/// Downloads and opens one blob, then answers the caller (`SYNC-055`). A
/// success answers with the whole plaintext. A failure is reported, and no
/// byte of the blob is sent.
async fn download_media(
    http: &impl MediaHttp,
    base: &crate::http::Url<http::Route>,
    id: Seq<Request>,
    content_hash: ambre_core::Validated<ambre_core::ContentHash>,
    keyring: &crate::chain::Keyring,
    tx: &ambre_bus::conduit::UnboundedSender<SyncEvent>,
    logic_tx: &LogicSink,
) {
    match download_leg(http, base, &content_hash, keyring).await {
        Ok(data) => {
            let _ = tx.send(SyncEvent::MediaDownloaded {
                id,
                document_id: keyring.document_id,
                content_hash,
                data,
            });
        }
        Err(message) => report(logic_tx, id, ApiRequest::DownloadMedia, message),
    }
}

/// Asks the relay to erase this user's upload, then answers the caller.
///
/// A yes erased it. A refusal (403) or an absent document (404) leaves nothing
/// of this user's to erase. A retry after a lost yes meets the same, so both
/// answer as erased. Anything else is reported, and the caller retries it.
async fn erase_media(
    http: &impl MediaHttp,
    base: &crate::http::Url<http::Route>,
    id: Seq<Request>,
    document_id: Id<Document>,
    content_hash: ambre_core::Validated<ambre_core::ContentHash>,
    tx: &ambre_bus::conduit::UnboundedSender<SyncEvent>,
    logic_tx: &LogicSink,
) {
    match http
        .delete(&base.join(format!("/media/{content_hash}")))
        .await
    {
        Ok(status) if status.is_success() || matches!(status.get(), 403 | 404) => {
            let _ = tx.send(SyncEvent::MediaErased {
                id,
                document_id,
                content_hash,
            });
        }
        Ok(status) => {
            let refused = http::Failed {
                failure: ApiFailure::Relay { status, code: None },
            };
            report(logic_tx, id, ApiRequest::EraseMedia, refused);
        }
        Err(message) => report(logic_tx, id, ApiRequest::EraseMedia, message),
    }
}

/// Asks for a link, puts the sealed blob, then confirms it (`SYNC-076`). Each
/// step returns before the next runs, so a failed put confirms nothing.
async fn upload_leg(
    http: &impl MediaHttp,
    base: &crate::http::Url<http::Route>,
    content_hash: &ambre_core::Validated<ambre_core::ContentHash>,
    blob: &pipeline::SealedBlob,
) -> Result<(), http::Failed> {
    let size = blob.len() as u64;
    let link: http::UploadUrlResponse = http
        .post_json(
            &base.join("/media/upload-url"),
            &http::UploadUrlRequest {
                content_hash: content_hash.clone(),
                size: ambre_core::Count::<ambre_core::Octets>::new(size),
            },
        )
        .await
        .map_err(|e| e.at("upload-url"))?;
    // The presigned PUT goes straight to object storage, not to the relay.
    http.put_sealed(&link.upload_url, blob)
        .await
        .map_err(|e| e.at("S3 PUT"))?;
    http.post_json::<_, http::WriteAnswer>(
        &base.join("/media"),
        &http::ConfirmUploadRequest {
            content_hash: content_hash.clone(),
            size: ambre_core::Count::<ambre_core::Octets>::new(size),
        },
    )
    .await
    .and_then(|answer| answer.expect(ambre_core::WriteStatus::Confirmed))
    .map_err(|e| e.at("confirm"))
}

/// Asks for a link, fetches the blob, then opens it whole (`SYNC-055`). A
/// failed step returns before any data is built, so nothing half-read leaves.
async fn download_leg(
    http: &impl MediaHttp,
    base: &crate::http::Url<http::Route>,
    content_hash: &ambre_core::Validated<ambre_core::ContentHash>,
    keyring: &crate::chain::Keyring,
) -> Result<ambre_core::Payload<ambre_core::MediaBytes>, http::Failed> {
    let link: http::DownloadUrlResponse = http
        .post_json(
            &base.join("/media/download-url"),
            &http::DownloadUrlRequest {
                content_hash: content_hash.clone(),
            },
        )
        .await
        .map_err(|e| e.at("download-url"))?;
    let blob = http
        .fetch_bytes(&link.download_url)
        .await
        .map_err(|e| e.at("S3 GET"))?;
    open_blob_named(&blob, content_hash, keyring).map_err(|e| {
        tracing::warn!("Sync: Open: {e}");
        http::Failed::local(ambre_core::LocalFailure::Crypto)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `SYNC-186`: with no relay, a download and an erase each answer the
    /// Logic thread at once, so no view waits on them.
    #[test]
    fn a_media_request_with_no_relay_answers_at_once() {
        let (sink, rx) = LogicSink::channel();
        let state = SyncState::new(
            sink,
            crate::http::Url::<crate::RelayHost>::new(String::new()),
        );
        let document_id = Id::<Document>::mint();
        let hash = || {
            ambre_core::Validated::<ambre_core::ContentHash>::parse("cd".repeat(32)).expect("hex")
        };
        state.handle_download_media(Seq::<Request>::new(1), document_id, hash());
        state.handle_erase_media(Seq::<Request>::new(2), document_id, hash());
        let answers: Vec<LogicEvent> = rx.try_iter().collect();
        assert!(
            matches!(
                answers.as_slice(),
                [
                    LogicEvent::Rest {
                        evt: RestEvent::MediaDownloaded { .. },
                        ..
                    },
                    LogicEvent::Rest {
                        evt: RestEvent::MediaErased { .. },
                        ..
                    },
                ]
            ),
            "{answers:?}"
        );
    }

    /// The address the worker answers with is keyed by the document key: the
    /// same bytes in two documents get two addresses (`SYNC-050`).
    #[test]
    fn an_upload_is_addressed_under_the_document_key() {
        let (sink, rx) = LogicSink::channel();
        let mut state = SyncState::new(
            sink,
            crate::http::Url::<crate::RelayHost>::new(String::new()),
        );
        let picture = b"one picture, two documents".to_vec();
        let mut addresses = Vec::new();
        for byte in [0x31, 0x32] {
            let document_id = Id::<Document>::mint();
            let key = DocumentKey::from_array([byte; 32]);
            let doc_state = SyncDocumentState::new(
                document_id,
                key.clone(),
                DocType::Toile,
                crate::Existence::OnRelay,
            );
            state.documents.insert(document_id, doc_state);
            state.handle_upload_media(
                ambre_bus::next_request(),
                document_id,
                picture.clone().into(),
            );
            let answer = rx.try_iter().find_map(|event| match event {
                LogicEvent::Rest {
                    evt: RestEvent::MediaUploaded { content_hash, .. },
                    ..
                } => Some(content_hash),
                _ => None,
            });
            let address = answer.expect("the upload answered with an address");
            let expected = ambre_crypto::commitment::media_address(&key, &picture.clone().into());
            assert_eq!(address, expected.content_hash());
            addresses.push(address);
        }
        assert_ne!(addresses[0], addresses[1]);
    }

    /// An upload for a document the relay does not hold waits for its
    /// creation. A creation ends the wait with an address, and a failed one
    /// with an error (`SYNC-145`, `SYNC-146`).
    #[test]
    fn an_upload_waits_for_its_document_on_the_relay() {
        let (sink, rx) = LogicSink::channel();
        let mut state = SyncState::new(
            sink,
            crate::http::Url::<crate::RelayHost>::new(String::new()),
        );
        let picture = b"a picture on a new board".to_vec();
        let answered = |rx: &ambre_bus::conduit::Receiver<LogicEvent>| {
            rx.try_iter()
                .filter_map(|event| match event {
                    LogicEvent::Rest {
                        evt: RestEvent::MediaUploaded { .. },
                        ..
                    } => Some(Ok(())),
                    LogicEvent::ApiError {
                        request: ApiRequest::UploadMedia,
                        ..
                    } => Some(Err(())),
                    _ => None,
                })
                .collect::<Vec<_>>()
        };
        for created in [true, false] {
            let document_id = Id::<Document>::mint();
            let key = DocumentKey::from_array([0x41; 32]);
            let doc_state = SyncDocumentState::new(
                document_id,
                key,
                DocType::Toile,
                crate::Existence::LocalOnly,
            );
            state.documents.insert(document_id, doc_state);
            state.handle_upload_media(
                ambre_bus::next_request(),
                document_id,
                picture.clone().into(),
            );
            assert!(
                answered(&rx).is_empty(),
                "the upload went ahead of its document"
            );

            let event = if created {
                SyncEvent::DocumentMaterialized { document_id }
            } else {
                SyncEvent::DocumentCreateFailed {
                    document_id,
                    failure: http::Failed::local(ambre_core::LocalFailure::NoRelay),
                }
            };
            state.handle_sync_event(event);
            let expected = if created { Ok(()) } else { Err(()) };
            assert_eq!(answered(&rx), vec![expected]);
        }
    }

    /// A transport that records each step and fails the one named.
    struct FakeMedia {
        fail: &'static str,
        served: Vec<u8>,
        /// The status a `DELETE` answers, when it is not the failing step.
        erase_status: ambre_core::HttpStatus,
        steps: std::cell::RefCell<Vec<&'static str>>,
    }

    impl FakeMedia {
        fn new(fail: &'static str, served: Vec<u8>) -> Self {
            Self {
                fail,
                served,
                erase_status: ambre_core::HttpStatus::new(200),
                steps: Default::default(),
            }
        }
        fn step(&self, name: &'static str) -> Result<(), http::HttpError> {
            self.steps.borrow_mut().push(name);
            if name == self.fail {
                return Err(http::HttpError::Local(ambre_core::LocalFailure::NoRelay));
            }
            Ok(())
        }
    }

    /// The relay base every media test addresses.
    fn base() -> crate::http::Url<crate::http::Route> {
        crate::http::Url::<crate::http::ApiRoot>::of(
            crate::http::UrlScheme::Https,
            &crate::http::Url::<crate::http::RelayHost>::new("relay.test".to_owned()),
        )
        .route("/documents/d")
    }

    impl MediaHttp for FakeMedia {
        async fn post_json<B: Serialize, R: DeserializeOwned>(
            &self,
            url: &crate::http::Url<crate::http::Route>,
            _body: &B,
        ) -> Result<R, http::HttpError> {
            let name = match url.as_str().rsplit('/').next() {
                Some("upload-url") => "link",
                Some("download-url") => "link",
                _ => "confirm",
            };
            self.step(name)?;
            let answer = serde_json::json!({
                "upload_url": "put", "download_url": "get", "s3_key": "k", "status": "confirmed",
            });
            serde_json::from_value(answer)
                .map_err(|_| http::HttpError::Local(ambre_core::LocalFailure::UnexpectedAnswer))
        }
        async fn put_sealed(
            &self,
            _url: &ambre_core::Credential<ambre_core::PresignedUrl>,
            _blob: &pipeline::SealedBlob,
        ) -> Result<(), http::HttpError> {
            self.step("put")
        }
        async fn fetch_bytes(
            &self,
            _url: &ambre_core::Credential<ambre_core::PresignedUrl>,
        ) -> Result<Payload<pipeline::FetchedBlob>, http::HttpError> {
            self.step("fetch")?;
            Ok(Payload::<pipeline::FetchedBlob>::new(self.served.clone()))
        }
        async fn delete(
            &self,
            url: &crate::http::Url<crate::http::Route>,
        ) -> Result<ambre_core::HttpStatus, http::HttpError> {
            assert!(
                url.as_str()
                    .ends_with(&format!("/media/{}", "cd".repeat(32))),
                "{url}"
            );
            self.step("delete")?;
            Ok(self.erase_status)
        }
    }

    /// Holds `SYNC-174`: an erase the relay answered, yes or refusal, reaches
    /// the caller as erased, since nothing of this user's is left either way.
    /// One that failed to reach an answer, or met a server error, is reported
    /// for a retry (`SYNC-176`).
    #[tokio::test]
    async fn an_erase_answers_erased_unless_it_must_be_retried() {
        let (logic, logic_rx) = LogicSink::channel();
        let (tx, events) = ambre_bus::conduit::unbounded::<SyncEvent>();
        let document_id = Id::<Document>::mint();
        for (fail, status, erased) in [
            ("", 200, true),
            ("", 403, true),
            ("", 404, true),
            ("", 503, false),
            ("delete", 200, false),
        ] {
            let mut http = FakeMedia::new(fail, Vec::new());
            http.erase_status = ambre_core::HttpStatus::new(status);
            let id = ambre_bus::next_request();
            let hash = ambre_core::Validated::<ambre_core::ContentHash>::parse("cd".repeat(32))
                .expect("a hash");
            erase_media(&http, &base(), id, document_id, hash, &tx, &logic).await;
            let (errors, sent) = answers(&logic_rx, &events);
            if erased {
                assert!(errors.is_empty(), "{status}");
                assert!(matches!(
                    &sent[..],
                    [SyncEvent::MediaErased { id: answered, .. }] if *answered == id
                ));
            } else {
                assert_eq!(errors, [(Some(id), "EraseMedia".to_owned())], "{status}");
                assert!(sent.is_empty());
            }
        }
    }

    /// The answers one request drew: the error reports the application got,
    /// and the success events the worker sent.
    #[expect(
        clippy::type_complexity,
        reason = "a test helper returns the pair its callers destructure"
    )]
    fn answers(
        logic: &ambre_bus::conduit::Receiver<LogicEvent>,
        events: &ambre_bus::conduit::Receiver<SyncEvent>,
    ) -> (Vec<(Option<Seq<Request>>, String)>, Vec<SyncEvent>) {
        let errors = logic
            .try_iter()
            .filter_map(|event| match event {
                LogicEvent::ApiError { id, request, .. } => Some((id, request.to_string())),
                _ => None,
            })
            .collect();
        (errors, events.try_iter().collect())
    }

    /// Holds `SYNC-076`: an upload failing at any step confirms nothing and
    /// reports the failure to the caller. A whole one confirms once, last, and
    /// answers with the address (`SYNC-075`).
    #[tokio::test]
    async fn a_failed_upload_step_confirms_nothing() {
        let key = DocumentKey::from_array([0x31; 32]);
        let document_id = Id::<Document>::mint();
        let address = ambre_crypto::commitment::media_address(&key, &b"a picture".to_vec().into());
        let upload = |id| Upload {
            id,
            document_id,
            data: b"a picture".to_vec().into(),
            address,
            doc_key: key.clone(),
            signing_key: SigningKey::from_bytes(&[0x07; 32]),
        };
        let (logic, logic_rx) = LogicSink::channel();
        let (tx, events) = ambre_bus::conduit::unbounded::<SyncEvent>();

        for (fail, ran) in [
            ("link", &["link"][..]),
            ("put", &["link", "put"][..]),
            ("confirm", &["link", "put", "confirm"][..]),
        ] {
            let http = FakeMedia::new(fail, Vec::new());
            let id = ambre_bus::next_request();
            upload_media(&http, &base(), upload(id), &tx, &logic).await;
            assert_eq!(*http.steps.borrow(), ran);
            let (errors, sent) = answers(&logic_rx, &events);
            assert_eq!(errors, [(Some(id), "UploadMedia".to_owned())], "{fail}");
            assert!(sent.is_empty(), "{fail} answered a success");
        }
        let http = FakeMedia::new("", Vec::new());
        let id = ambre_bus::next_request();
        upload_media(&http, &base(), upload(id), &tx, &logic).await;
        assert_eq!(*http.steps.borrow(), ["link", "put", "confirm"]);
        let (errors, sent) = answers(&logic_rx, &events);
        assert!(errors.is_empty());
        assert!(matches!(
            &sent[..],
            [SyncEvent::MediaUploaded { id: answered, content_hash, .. }]
                if *answered == id && *content_hash == address.content_hash()
        ));
    }

    /// Holds `SYNC-055`: a download that fails to fetch, or serves a short,
    /// altered or foreign blob, reports the failure and delivers no byte. A
    /// whole one delivers the plaintext (`SYNC-077`).
    #[tokio::test]
    async fn a_blob_that_fails_to_open_delivers_nothing() {
        let key = DocumentKey::from_array([0x31; 32]);
        let document_id = Id::<Document>::mint();
        let (blob, hash) = sealed(b"a picture", document_id, &key);
        let blob = blob.as_bytes().to_vec();
        let keyring = crate::chain::Keyring {
            document_id,
            current: key.clone(),
            previous: Vec::new(),
            chain: crate::chain::Chain::new(document_id, Vec::new()),
        };
        let mut altered = blob.clone();
        *altered.last_mut().unwrap() ^= 1;
        let foreign = sealed(b"a picture", Id::<Document>::mint(), &key)
            .0
            .as_bytes()
            .to_vec();
        let (logic, logic_rx) = LogicSink::channel();
        let (tx, events) = ambre_bus::conduit::unbounded::<SyncEvent>();

        for (fail, served) in [
            ("link", blob.clone()),
            ("fetch", blob.clone()),
            ("", blob[..blob.len() / 2].to_vec()),
            ("", altered),
            ("", foreign),
        ] {
            let http = FakeMedia::new(fail, served);
            let id = ambre_bus::next_request();
            download_media(&http, &base(), id, hash.clone(), &keyring, &tx, &logic).await;
            let (errors, sent) = answers(&logic_rx, &events);
            assert_eq!(errors, [(Some(id), "DownloadMedia".to_owned())], "{fail}");
            assert!(sent.is_empty(), "{fail} delivered bytes");
        }
        let http = FakeMedia::new("", blob);
        let id = ambre_bus::next_request();
        download_media(&http, &base(), id, hash.clone(), &keyring, &tx, &logic).await;
        let (errors, sent) = answers(&logic_rx, &events);
        assert!(errors.is_empty());
        assert!(matches!(
            &sent[..],
            [SyncEvent::MediaDownloaded { id: answered, content_hash, data, .. }]
                if *answered == id && *content_hash == hash && data == b"a picture"
        ));
    }

    fn sealed(
        data: &[u8],
        document_id: Id<Document>,
        key: &DocumentKey,
    ) -> (
        Payload<pipeline::FetchedBlob>,
        ambre_core::Validated<ambre_core::ContentHash>,
    ) {
        let signing = SigningKey::from_bytes(&[0x07; 32]);
        let data = ambre_core::Payload::new(data.to_vec());
        let address = ambre_crypto::commitment::media_address(key, &data);
        let blob = seal_blob(&data, document_id, &address, key, &signing).unwrap();
        (
            Payload::<pipeline::FetchedBlob>::new(blob.into_bytes()),
            address.content_hash(),
        )
    }

    /// A picture sealed before two rotations opens with the current key and
    /// the chain, and not without it (`SYNC-042`).
    #[test]
    fn a_blob_sealed_before_a_rotation_opens_through_the_chain() {
        let document_id = Id::<Document>::mint();
        let signing = SigningKey::from_bytes(&[0x07; 32]);
        let keys: Vec<_> = (0..3).map(|_| DocumentKey::generate()).collect();
        let links: Vec<_> = keys
            .windows(2)
            .map(|pair| pipeline::seal_link(document_id, &pair[0], &pair[1], &signing).unwrap())
            .collect();
        let (blob, address) = sealed(b"a picture", document_id, &keys[0]);
        let keyring = |links| crate::chain::Keyring {
            document_id,
            current: keys[2].clone(),
            previous: Vec::new(),
            chain: crate::chain::Chain::new(document_id, links),
        };
        assert_eq!(
            open_blob_named(&blob, &address, &keyring(links)).unwrap(),
            b"a picture"
        );
        assert!(open_blob_named(&blob, &address, &keyring(Vec::new())).is_err());
    }

    /// A blob sealed for one document does not open under another, even one
    /// that holds the same key (`SYNC-051`).
    #[test]
    fn a_blob_opens_under_its_own_document_only() {
        let key = DocumentKey::from_array([0x31; 32]);
        let own = Id::<Document>::mint();
        let (blob, address) = sealed(b"a picture", own, &key);
        assert_eq!(open_blob(&blob, own, &address, &key).unwrap(), b"a picture");
        assert!(open_blob(&blob, Id::<Document>::mint(), &address, &key).is_err());
    }

    /// A blob sealed under one address does not open under another address
    /// of the same document (`SYNC-074`).
    #[test]
    fn a_blob_opens_under_its_own_address_only() {
        let key = DocumentKey::from_array([0x31; 32]);
        let document_id = Id::<Document>::mint();
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
        let document_id = Id::<Document>::mint();
        let (_, wanted) = sealed(b"a picture", document_id, &key);
        let (served, _) = sealed(b"another picture", document_id, &key);
        assert!(open_blob(&served, document_id, &wanted, &key).is_err());

        let address = ambre_crypto::commitment::media_address(&key, &b"a picture".to_vec().into());
        let signing = SigningKey::from_bytes(&[0x07; 32]);
        let other = ambre_core::Payload::new(b"other bytes".to_vec());
        let forged = seal_blob(&other, document_id, &address, &key, &signing).unwrap();
        let forged = &Payload::<pipeline::FetchedBlob>::new(forged.into_bytes());
        assert_eq!(
            pipeline::open_object(forged, &pipeline::media_aad(document_id, &address), &key)
                .unwrap()
                .into_plaintext(),
            b"other bytes"
        );
        assert!(open_blob(forged, document_id, &wanted, &key).is_err());
        // A text that is no address never reaches the opener: no hash spells it.
        assert_eq!(
            ambre_core::Validated::<ambre_core::ContentHash>::parse("not hex").ok(),
            None
        );
    }
}
