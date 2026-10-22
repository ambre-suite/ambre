// ambre-sync/src/finalize.rs
// ============================================================================
// The finalization of a closing segment, on the Sync side (the Murmure
// segments spec). The Logic thread asks for the lease and hands over the
// shallow snapshot; this worker speaks to the relay. The relay arbitrates
// both acts (`SRV-113`, `SRV-116`).
// ============================================================================

use super::*;
use ambre_bus::{ChannelId, LeaseAnswer, LeaseId, SegmentId, Seq};

use crate::updates::{SnapshotUpload, upload_snapshot};

/// Ask the relay for the finalize lease of `segment`.
pub(crate) async fn take_lease(
    transport: &impl http::Transport,
    auth: &http::AuthContext,
    base: &crate::http::ApiRoot,
    channel_id: ChannelId,
    segment: SegmentId,
) -> Result<LeaseAnswer, String> {
    let url = base.route(format!("/channels/{channel_id}/segments/{segment}/lease"));
    let answer: http::FinalizeLeaseResponse =
        http::post_json_auth_via(transport, &url, &serde_json::json!({}), auth).await?;
    Ok(match answer.lease {
        Some(granted) => LeaseAnswer::Granted {
            lease: granted.lease_id,
            horizon: granted.horizon,
            expires_in: granted.expires_in,
        },
        None => LeaseAnswer::Deferred {
            retry_in: answer.retry_in,
        },
    })
}

/// Upload the final snapshot of `segment`, then commit it under `lease`
/// (`SRV-116`). The relay checks the snapshot covers the horizon.
pub(crate) async fn commit_final(
    transport: &impl http::Transport,
    auth: &http::AuthContext,
    base: &crate::http::ApiRoot,
    channel_id: ChannelId,
    segment: SegmentId,
    lease: LeaseId,
    upload: SnapshotUpload,
) -> Result<(), String> {
    let snapshot_id = upload_snapshot(transport, auth, base, upload).await?;
    let url = base.route(format!(
        "/channels/{channel_id}/segments/{segment}/finalize"
    ));
    let body = http::FinalizeSegmentRequest {
        lease_id: lease,
        snapshot_id,
    };
    http::send_json_auth_via(transport, http::Method::Post, &url, &body, auth).await
}

impl SyncState {
    /// Ask the relay for the finalize lease of `segment`.
    pub(crate) fn handle_take_finalize_lease(&self, channel_id: ChannelId, segment: SegmentId) {
        let Some(tx) = self.event_tx.clone() else {
            return self.finalize_failed(channel_id, segment, false, "no relay on this transport");
        };
        let base = crate::http::ApiRoot::of(self.url_scheme, &self.api_base);
        let auth = self.auth_context();
        wasm_bindgen_futures::spawn_local(async move {
            let result = take_lease(&http::Fetch, &auth, &base, channel_id, segment).await;
            let _ = tx.send(SyncEvent::FinalizeLeaseAnswered {
                channel_id,
                segment,
                result,
            });
        });
    }

    /// Seal `payload` as the final snapshot of `segment` up to `horizon`,
    /// upload it and commit it under `lease`.
    ///
    /// A copy holding an edit the relay refused is not the relay's copy, so
    /// it withdraws: another member finalizes the segment. A copy with an
    /// edit still unanswered waits, and asks again later.
    pub(crate) fn handle_finalize_segment(
        &self,
        channel_id: ChannelId,
        segment: SegmentId,
        lease: LeaseId,
        horizon: Seq,
        payload: &[u8],
    ) {
        let document_id = segment.document_id();
        let Some(doc_state) = self.documents.get(&document_id) else {
            return self.finalize_failed(channel_id, segment, false, "the segment is not open");
        };
        if doc_state.rekey.holds_refused() {
            return self.finalize_failed(channel_id, segment, true, "the relay refused an edit");
        }
        if !doc_state.rekey.settled() {
            return self.finalize_failed(channel_id, segment, false, "an edit is unanswered");
        }
        let upload = match self.snapshot_upload(document_id, payload, horizon) {
            Ok(upload) => upload,
            Err(message) => return self.finalize_failed(channel_id, segment, false, &message),
        };
        let Some(tx) = self.event_tx.clone() else {
            return self.finalize_failed(channel_id, segment, false, "no relay on this transport");
        };
        let base = crate::http::ApiRoot::of(self.url_scheme, &self.api_base);
        let auth = self.auth_context();
        wasm_bindgen_futures::spawn_local(async move {
            let result = commit_final(
                &http::Fetch,
                &auth,
                &base,
                channel_id,
                segment,
                lease,
                upload,
            )
            .await;
            let _ = tx.send(SyncEvent::SegmentFinalized {
                channel_id,
                segment,
                result,
            });
        });
    }

    /// Tell the Logic thread a lease request or a commit of `segment` failed.
    pub(crate) fn finalize_failed(
        &self,
        channel_id: ChannelId,
        segment: SegmentId,
        withdrawn: bool,
        message: &str,
    ) {
        tracing::warn!("[Sync] finalization of {segment} failed: {message}");
        self.send_to_logic(LogicEvent::SegmentFinalizeFailed {
            channel_id,
            segment,
            withdrawn,
            message: message.to_owned(),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::scripted::{Scripted, session};

    const BASE: &str = "https://relay.test";

    /// The root the calls under test reach, under [`BASE`].
    fn root() -> crate::http::ApiRoot {
        crate::http::ApiRoot::of(
            "https",
            &crate::http::RelayHost::new("relay.test".to_owned()),
        )
    }

    /// A sealed snapshot of `document_id` covering the deltas up to `horizon`.
    fn upload(document_id: DocumentId, horizon: Seq) -> SnapshotUpload {
        let key = DocumentKey::generate();
        let sk = MasterSeed::generate().derive_signing_key();
        let public_data = SnapshotPublicData {
            document_id,
            pub_key: PublicKey::from_array(sk.verifying_key().to_bytes()),
            key_id: key.id(),
            snapshot_id: SnapshotId::new(),
            parent_snapshot_proof: Vec::new(),
            parent_snapshot_update_clocks: Vec::new(),
        };
        SnapshotUpload {
            document_id,
            author_pubkey: ambre_core::SigningKeyHex::encode(sk.verifying_key().to_bytes()),
            covered_seq: horizon,
            blob: pipeline::snapshot_blob(b"final", public_data, &key, &sk).expect("seal"),
        }
    }

    /// The relay's answer reads as a lease or a deferral (the Murmure
    /// segments spec).
    #[tokio::test]
    async fn a_lease_request_reads_the_grant_or_the_deferral() {
        let (channel, segment) = (ChannelId::new(), SegmentId::new());
        let lease = LeaseId::new();
        let transport = Scripted::new([
            Ok((
                200,
                serde_json::json!({
                    "lease": { "lease_id": lease.to_string(), "horizon": 41, "expires_in": 30_000 },
                    "retry_in": 0,
                }),
            )),
            Ok((
                200,
                serde_json::json!({ "lease": null, "retry_in": 86_400_000 }),
            )),
        ]);
        let auth = session("access-1", "refresh-1");
        let granted = take_lease(&transport, &auth, &root(), channel, segment).await;
        assert_eq!(
            granted,
            Ok(LeaseAnswer::Granted {
                lease,
                horizon: ambre_bus::Seq(41),
                expires_in: ambre_core::Millis(30_000)
            })
        );
        let deferred = take_lease(&transport, &auth, &root(), channel, segment).await;
        assert_eq!(
            deferred,
            Ok(LeaseAnswer::Deferred {
                retry_in: ambre_core::Millis(86_400_000)
            })
        );
        let url = format!("{BASE}/api/channels/{channel}/segments/{segment}/lease");
        assert_eq!(
            transport.calls(),
            vec![(http::Method::Post, url.clone()), (http::Method::Post, url)]
        );
    }

    /// The final snapshot uploads at the horizon, then the commit names the
    /// lease and the snapshot the relay registered (the Murmure segments
    /// spec).
    #[tokio::test]
    async fn a_commit_uploads_the_snapshot_at_the_horizon_then_names_it() {
        let (channel, segment) = (ChannelId::new(), SegmentId::new());
        let (lease, given) = (LeaseId::new(), SnapshotId::new());
        let document = segment.document_id();
        let link = serde_json::json!({
            "upload_url": "https://store.test/o?sig=x",
            "s3_key": "snapshots/o",
        });
        let transport = Scripted::new([
            Ok((200, link)),
            Ok((200, serde_json::Value::Null)),
            Ok((200, serde_json::json!({ "snapshot_id": given.to_string() }))),
            Ok((200, serde_json::json!({ "folded": null }))),
            Ok((204, serde_json::Value::Null)),
        ]);
        let auth = session("access-1", "refresh-1");
        let committed = commit_final(
            &transport,
            &auth,
            &root(),
            channel,
            segment,
            lease,
            upload(document, ambre_bus::Seq(41)),
        )
        .await;
        assert_eq!(committed, Ok(()));
        let sent = transport.sent.borrow();
        let json = |at: usize| -> serde_json::Value {
            serde_json::from_slice(&sent[at].body.as_ref().expect("a body").1).expect("json")
        };
        assert_eq!(
            json(2)["covered_seq"],
            41,
            "the snapshot covers the horizon"
        );
        let commit = sent.len() - 1;
        assert_eq!(
            sent[commit].url,
            format!("{BASE}/api/channels/{channel}/segments/{segment}/finalize")
        );
        assert_eq!(
            json(commit),
            serde_json::json!({
                "lease_id": lease.to_string(),
                "snapshot_id": given.to_string(),
            })
        );
    }
}
