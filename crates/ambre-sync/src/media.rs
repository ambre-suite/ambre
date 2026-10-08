use super::*;
use crate::sharing::GlooHttp;
use serde::Serialize;
use serde::de::DeserializeOwned;

type Address = [u8; ambre_crypto::commitment::TAG_LEN];

/// Seals a blob for one document and the address it is stored under
/// (`SYNC-051`, `SYNC-074`).
fn seal_blob(
    data: &[u8],
    document_id: DocumentId,
    address: &Address,
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

/// Opens a downloaded blob with the one key it names: the current key, a
/// cached one, or one the chain recovers (`SYNC-042`, `SYNC-045`).
fn open_blob_named(
    blob: &[u8],
    content_hash: &str,
    keyring: &crate::chain::Keyring,
) -> Result<Vec<u8>, String> {
    let wanted = pipeline::object_key(blob).map_err(|e| e.to_string())?;
    let key = keyring
        .find(wanted)
        .ok_or_else(|| "the blob names a key out of reach".to_owned())?;
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
            let base = format!("{url_scheme}://{api_base}/api/documents/{document_id}");
            let outcome = match seal_blob(&data, document_id, &address, &doc_key, &signing_key) {
                Ok(blob) => upload_leg(&GlooHttp { auth }, &base, &content_hash, &blob).await,
                Err(e) => Err(format!("seal: {e}")),
            };
            match outcome {
                Ok(()) => {
                    let _ = tx.send(SyncEvent::MediaUploaded {
                        id,
                        document_id,
                        content_hash,
                    });
                }
                Err(message) => report(&logic_tx, id, "UploadMedia", message),
            }
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
        // A blob sealed before a rotation names an older key, which the
        // chain recovers (`SYNC-042`, `SYNC-119`).
        let keyring = self.documents.get(&document_id).map(|ds| ds.keyring());

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
        let Some(keyring) = keyring else {
            let _ = logic_tx.send(LogicEvent::ApiError {
                id: Some(id),
                request: "DownloadMedia".into(),
                code: 404,
                message: "Document not open".into(),
            });
            return;
        };

        wasm_bindgen_futures::spawn_local(async move {
            let base = format!("{url_scheme}://{api_base}/api/documents/{document_id}");
            match download_leg(&GlooHttp { auth }, &base, &content_hash, &keyring).await {
                Ok(data) => {
                    let _ = tx.send(SyncEvent::MediaDownloaded {
                        id,
                        document_id,
                        content_hash,
                        data,
                    });
                }
                Err(message) => report(&logic_tx, id, "DownloadMedia", message),
            }
        });
    }
}

/// The requests the two media legs make, abstracted over the transport so
/// that a host test drives each step failing.
#[allow(async_fn_in_trait)] // single-threaded worker; futures are intentionally !Send
pub(crate) trait MediaHttp {
    /// An authenticated request to the relay.
    async fn post_json<B: Serialize, R: DeserializeOwned>(
        &self,
        url: &str,
        body: &B,
    ) -> Result<R, String>;
    /// A sealed blob to a presigned object-storage link.
    async fn put_sealed(&self, url: &str, blob: &pipeline::SealedBlob) -> Result<(), String>;
    /// A blob from a presigned object-storage link.
    async fn fetch_bytes(&self, url: &str) -> Result<Vec<u8>, String>;
}

impl MediaHttp for GlooHttp {
    async fn post_json<B: Serialize, R: DeserializeOwned>(
        &self,
        url: &str,
        body: &B,
    ) -> Result<R, String> {
        http::post_json_auth::<B, R>(url, body, self.auth.clone()).await
    }
    async fn put_sealed(&self, url: &str, blob: &pipeline::SealedBlob) -> Result<(), String> {
        http::put_sealed(url, blob).await
    }
    async fn fetch_bytes(&self, url: &str) -> Result<Vec<u8>, String> {
        http::fetch_bytes(url).await
    }
}

/// Reports a failed media leg to the caller that asked for it.
fn report(logic_tx: &LogicSink, id: RequestId, request: &str, message: String) {
    let _ = logic_tx.send(LogicEvent::ApiError {
        id: Some(id),
        request: request.into(),
        code: 500,
        message,
    });
}

/// Asks for a link, puts the sealed blob, then confirms it (`SYNC-076`). Each
/// step returns before the next runs, so a failed put confirms nothing.
async fn upload_leg(
    http: &impl MediaHttp,
    base: &str,
    content_hash: &str,
    blob: &pipeline::SealedBlob,
) -> Result<(), String> {
    let size = blob.len() as u64;
    let link: http::UploadUrlResponse = http
        .post_json(
            &format!("{base}/media/upload-url"),
            &http::UploadUrlRequest {
                content_hash: content_hash.to_owned(),
                size_bytes: size,
            },
        )
        .await
        .map_err(|e| format!("upload-url: {e}"))?;
    // The presigned PUT goes straight to object storage, not to the relay.
    http.put_sealed(&link.upload_url, blob)
        .await
        .map_err(|e| format!("S3 PUT: {e}"))?;
    http.post_json::<_, serde_json::Value>(
        &format!("{base}/media"),
        &http::ConfirmUploadRequest {
            content_hash: content_hash.to_owned(),
            size_bytes: size as i64,
        },
    )
    .await
    .map_err(|e| format!("confirm: {e}"))?;
    Ok(())
}

/// Asks for a link, fetches the blob, then opens it whole (`SYNC-055`). A
/// failed step returns before any data is built, so nothing half-read leaves.
async fn download_leg(
    http: &impl MediaHttp,
    base: &str,
    content_hash: &str,
    keyring: &crate::chain::Keyring,
) -> Result<Vec<u8>, String> {
    let link: http::DownloadUrlResponse = http
        .post_json(
            &format!("{base}/media/download-url"),
            &http::DownloadUrlRequest {
                content_hash: content_hash.to_owned(),
            },
        )
        .await
        .map_err(|e| format!("download-url: {e}"))?;
    let blob = http
        .fetch_bytes(&link.download_url)
        .await
        .map_err(|e| format!("S3 GET: {e}"))?;
    open_blob_named(&blob, content_hash, keyring).map_err(|e| format!("Open: {e}"))
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

    /// A transport that records each step and fails the one named.
    struct FakeMedia {
        fail: &'static str,
        served: Vec<u8>,
        steps: std::cell::RefCell<Vec<&'static str>>,
    }

    impl FakeMedia {
        fn new(fail: &'static str, served: Vec<u8>) -> Self {
            Self {
                fail,
                served,
                steps: Default::default(),
            }
        }
        fn step(&self, name: &'static str) -> Result<(), String> {
            self.steps.borrow_mut().push(name);
            if name == self.fail {
                return Err(format!("{name} refused"));
            }
            Ok(())
        }
    }

    impl MediaHttp for FakeMedia {
        async fn post_json<B: Serialize, R: DeserializeOwned>(
            &self,
            url: &str,
            _body: &B,
        ) -> Result<R, String> {
            let name = match url.rsplit('/').next() {
                Some("upload-url") => "link",
                Some("download-url") => "link",
                _ => "confirm",
            };
            self.step(name)?;
            let answer = serde_json::json!({
                "upload_url": "put", "download_url": "get", "s3_key": "k", "status": "ok",
            });
            serde_json::from_value(answer).map_err(|e| e.to_string())
        }
        async fn put_sealed(&self, _url: &str, _blob: &pipeline::SealedBlob) -> Result<(), String> {
            self.step("put")
        }
        async fn fetch_bytes(&self, _url: &str) -> Result<Vec<u8>, String> {
            self.step("fetch")?;
            Ok(self.served.clone())
        }
    }

    /// Holds `SYNC-076`: an upload failing at any step reports the step and
    /// confirms nothing, and a whole one confirms once, last.
    #[tokio::test]
    async fn a_failed_upload_step_confirms_nothing() {
        let key = DocumentKey::from_array([0x31; 32]);
        let document_id = DocumentId::new();
        let signing = SigningKey::from_bytes(&[0x07; 32]);
        let address = ambre_crypto::commitment::media_address(&key, b"a picture");
        let blob = seal_blob(b"a picture", document_id, &address, &key, &signing).unwrap();
        let hash = hex::encode(address);

        for (fail, ran) in [
            ("link", &["link"][..]),
            ("put", &["link", "put"][..]),
            ("confirm", &["link", "put", "confirm"][..]),
        ] {
            let http = FakeMedia::new(fail, Vec::new());
            let outcome = upload_leg(&http, "base", &hash, &blob).await;
            assert!(outcome.is_err(), "{fail}");
            assert_eq!(*http.steps.borrow(), ran);
        }
        let http = FakeMedia::new("", Vec::new());
        assert!(upload_leg(&http, "base", &hash, &blob).await.is_ok());
        assert_eq!(*http.steps.borrow(), ["link", "put", "confirm"]);
    }

    /// Holds `SYNC-055`: a download that fails to fetch, or serves a short,
    /// altered or foreign blob, reports and delivers nothing. A whole one
    /// delivers the bytes.
    #[tokio::test]
    async fn a_blob_that_fails_to_open_delivers_nothing() {
        let key = DocumentKey::from_array([0x31; 32]);
        let document_id = DocumentId::new();
        let (blob, hash) = sealed(b"a picture", document_id, &key);
        let keyring = crate::chain::Keyring {
            document_id,
            current: key.clone(),
            previous: Vec::new(),
            chain: crate::chain::Chain::new(document_id, Vec::new()),
        };
        let mut altered = blob.clone();
        *altered.last_mut().unwrap() ^= 1;
        let (foreign, _) = sealed(b"a picture", DocumentId::new(), &key);

        for (fail, served) in [
            ("link", blob.clone()),
            ("fetch", blob.clone()),
            ("", blob[..blob.len() / 2].to_vec()),
            ("", altered),
            ("", foreign),
        ] {
            let http = FakeMedia::new(fail, served);
            let outcome = download_leg(&http, "base", &hash, &keyring).await;
            assert!(outcome.is_err(), "{fail} delivered {outcome:?}");
        }
        let http = FakeMedia::new("", blob);
        assert_eq!(
            download_leg(&http, "base", &hash, &keyring).await.unwrap(),
            b"a picture"
        );
    }

    fn sealed(data: &[u8], document_id: DocumentId, key: &DocumentKey) -> (Vec<u8>, String) {
        let signing = SigningKey::from_bytes(&[0x07; 32]);
        let address = ambre_crypto::commitment::media_address(key, data);
        let blob = seal_blob(data, document_id, &address, key, &signing).unwrap();
        (blob.as_bytes().to_vec(), hex::encode(address))
    }

    /// A picture sealed before two rotations opens with the current key and
    /// the chain, and not without it (`SYNC-042`).
    #[test]
    fn a_blob_sealed_before_a_rotation_opens_through_the_chain() {
        let document_id = DocumentId::new();
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
        let forged = forged.as_bytes();
        assert_eq!(
            pipeline::open_object(forged, &pipeline::media_aad(document_id, &address), &key)
                .unwrap(),
            b"other bytes"
        );
        assert!(open_blob(forged, document_id, &wanted, &key).is_err());
        assert!(open_blob(forged, document_id, "not hex", &key).is_err());
    }
}
