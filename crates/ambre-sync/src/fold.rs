// ambre-sync/src/fold.rs
// ============================================================================
// The fold of the amendments of a final segment, on the Sync side (the Murmure
// segments spec). The Logic thread asks for the lease and hands over the fold
// snapshot; this worker speaks to the relay, which arbitrates both acts
// (`SRV-204`, `SRV-207`).
// ============================================================================

use super::*;
use ambre_bus::{ChannelId, FoldAnswer, LeaseId, SegmentId, Seq};

use crate::updates::{SnapshotUpload, upload_snapshot};

/// Ask the relay for the fold lease of `segment`.
pub(crate) async fn take_fold_lease(
    transport: &impl http::Transport,
    auth: &http::AuthContext,
    base: &crate::http::ApiRoot,
    channel_id: ChannelId,
    segment: SegmentId,
) -> Result<FoldAnswer, String> {
    let url = base.route(format!(
        "/channels/{channel_id}/segments/{segment}/fold-lease"
    ));
    let answer: http::FoldLeaseResponse =
        http::post_json_auth_via(transport, &url, &serde_json::json!({}), auth).await?;
    Ok(match answer.lease {
        Some(granted) => FoldAnswer::Granted {
            lease: granted.lease_id,
            horizon: granted.horizon,
            amendments: granted.amendments,
            amendments_seq: granted.amendments_seq,
            expires_in: granted.expires_in,
        },
        None => FoldAnswer::Deferred {
            retry_in: answer.retry_in,
        },
    })
}

/// The lease a fold commits under, and how far it reaches into the
/// amendments.
#[derive(Debug, Clone, Copy)]
pub(crate) struct FoldReach {
    pub lease: LeaseId,
    pub amendments_seq: Seq,
}

/// Upload the fold snapshot of `segment`, then commit it under the lease of
/// `reach` (`SRV-207`).
pub(crate) async fn commit_fold(
    transport: &impl http::Transport,
    auth: &http::AuthContext,
    base: &crate::http::ApiRoot,
    channel_id: ChannelId,
    segment: SegmentId,
    reach: FoldReach,
    upload: SnapshotUpload,
) -> Result<(), String> {
    let snapshot_id = upload_snapshot(transport, auth, base, upload).await?;
    let url = base.route(format!("/channels/{channel_id}/segments/{segment}/fold"));
    let body = http::FoldSegmentRequest {
        lease_id: reach.lease,
        snapshot_id,
        amendments_seq: reach.amendments_seq,
    };
    http::send_json_auth_via(transport, http::Method::Post, &url, &body, auth).await
}

impl SyncState {
    /// Ask the relay for the fold lease of `segment`.
    pub(crate) fn handle_take_fold_lease(&self, channel_id: ChannelId, segment: SegmentId) {
        let Some(tx) = self.event_tx.clone() else {
            return self.fold_failed(channel_id, segment, "no relay on this transport");
        };
        let base = crate::http::ApiRoot::of(self.url_scheme, &self.api_base);
        let auth = self.auth_context();
        wasm_bindgen_futures::spawn_local(async move {
            let result = take_fold_lease(&http::Fetch, &auth, &base, channel_id, segment).await;
            let _ = tx.send(SyncEvent::FoldLeaseAnswered {
                channel_id,
                segment,
                result,
            });
        });
    }

    /// Seal `payload` as the fold snapshot of `segment` up to `horizon`,
    /// upload it and commit it under `lease`. The segment is final, so no
    /// edit of this client is pending on it.
    pub(crate) fn handle_fold_segment(
        &self,
        channel_id: ChannelId,
        segment: SegmentId,
        reach: FoldReach,
        horizon: Seq,
        payload: &[u8],
    ) {
        let document_id = segment.document_id();
        let upload = match self.snapshot_upload(document_id, payload, horizon) {
            Ok(upload) => upload,
            Err(message) => return self.fold_failed(channel_id, segment, &message),
        };
        let Some(tx) = self.event_tx.clone() else {
            return self.fold_failed(channel_id, segment, "no relay on this transport");
        };
        let base = crate::http::ApiRoot::of(self.url_scheme, &self.api_base);
        let auth = self.auth_context();
        wasm_bindgen_futures::spawn_local(async move {
            let result = commit_fold(
                &http::Fetch,
                &auth,
                &base,
                channel_id,
                segment,
                reach,
                upload,
            )
            .await;
            let _ = tx.send(SyncEvent::SegmentFolded {
                channel_id,
                segment,
                result,
            });
        });
    }

    /// Tell the Logic thread a fold lease request or a fold commit of
    /// `segment` failed.
    pub(crate) fn fold_failed(&self, channel_id: ChannelId, segment: SegmentId, message: &str) {
        tracing::warn!("[Sync] fold of {segment} failed: {message}");
        self.send_to_logic(LogicEvent::SegmentFoldFailed {
            channel_id,
            segment,
            message: message.to_owned(),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::scripted::{Scripted, session};
    use crate::updates::{Vouch, send_vouch};

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
            blob: pipeline::snapshot_blob(b"fold", public_data, &key, &sk).expect("seal"),
        }
    }

    /// The relay's answer reads as a lease naming the amendments, or as a
    /// deferral (the Murmure segments spec).
    #[tokio::test]
    async fn a_fold_lease_request_reads_the_grant_or_the_deferral() {
        let (channel, segment) = (ChannelId::new(), SegmentId::new());
        let (lease, amendments) = (LeaseId::new(), DocumentId::new());
        let transport = Scripted::new([
            Ok((
                200,
                serde_json::json!({
                    "lease": {
                        "lease_id": lease.to_string(),
                        "horizon": 41,
                        "amendments": amendments.to_string(),
                        "amendments_seq": 7,
                        "expires_in": 30_000,
                    },
                    "retry_in": 0,
                }),
            )),
            Ok((
                200,
                serde_json::json!({ "lease": null, "retry_in": 86_400_000 }),
            )),
        ]);
        let auth = session("access-1", "refresh-1");
        let granted = take_fold_lease(&transport, &auth, &root(), channel, segment).await;
        assert_eq!(
            granted,
            Ok(FoldAnswer::Granted {
                lease,
                horizon: ambre_bus::Seq(41),
                amendments,
                amendments_seq: ambre_bus::Seq(7),
                expires_in: ambre_core::Millis(30_000)
            })
        );
        let deferred = take_fold_lease(&transport, &auth, &root(), channel, segment).await;
        assert_eq!(
            deferred,
            Ok(FoldAnswer::Deferred {
                retry_in: ambre_core::Millis(86_400_000)
            })
        );
        let url = format!("{BASE}/api/channels/{channel}/segments/{segment}/fold-lease");
        assert_eq!(
            transport.calls(),
            vec![(http::Method::Post, url.clone()), (http::Method::Post, url)]
        );
    }

    /// The fold snapshot uploads at the horizon, then the commit names the
    /// lease, the snapshot and how far the amendments reach (the Murmure
    /// segments spec).
    #[tokio::test]
    async fn a_fold_commit_uploads_at_the_horizon_then_names_the_reach() {
        let (channel, segment) = (ChannelId::new(), SegmentId::new());
        let (lease, given) = (LeaseId::new(), SnapshotId::new());
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
        let upload = upload(segment.document_id(), ambre_bus::Seq(41));
        let reach = FoldReach {
            lease,
            amendments_seq: ambre_bus::Seq(7),
        };
        let committed =
            commit_fold(&transport, &auth, &root(), channel, segment, reach, upload).await;
        assert_eq!(committed, Ok(()));
        let sent = transport.sent.borrow();
        let json = |at: usize| -> serde_json::Value {
            serde_json::from_slice(&sent[at].body.as_ref().expect("a body").1).expect("json")
        };
        assert_eq!(json(2)["covered_seq"], 41, "the fold covers the horizon");
        let commit = sent.len() - 1;
        assert_eq!(
            sent[commit].url,
            format!("{BASE}/api/channels/{channel}/segments/{segment}/fold")
        );
        assert_eq!(
            json(commit),
            serde_json::json!({
                "lease_id": lease.to_string(),
                "snapshot_id": given.to_string(),
                "amendments_seq": 7,
            })
        );
    }

    /// A confirmation that settled a fold names the amendments the relay
    /// erased (the Murmure segments spec).
    #[tokio::test]
    async fn a_confirmation_that_settles_a_fold_names_the_erased_amendments() {
        let (document, snapshot, amendments) =
            (DocumentId::new(), SnapshotId::new(), DocumentId::new());
        let transport = Scripted::new([Ok((
            200,
            serde_json::json!({ "folded": amendments.to_string() }),
        ))]);
        let auth = session("access-1", "refresh-1");
        let folded = send_vouch(
            &transport,
            &auth,
            &root(),
            document,
            snapshot,
            &Vouch::Confirm,
        )
        .await;
        assert_eq!(folded, Ok(Some(amendments)));
    }
}
