#![allow(
    clippy::expect_used,
    reason = "a test double fails the test that misuses it"
)]

use std::collections::HashMap;

use ambre_bus::{DocumentId, SnapshotId, UserId};
use ambre_protocol::messages::{Seq, Update};
use ambre_protocol::ws::{WsClientMsg, WsServerMsg};

/// In-process Blind Relay simulator for multi-client integration tests.
///
/// Manages document subscriptions, ordered update delivery (sequenced by a
/// global `Seq`), and snapshot tracking. Clients connect via bidirectional
/// `ambre_bus::conduit` queues, one queue pair per client. The full SecSync
/// pipeline (crypto, compression, quarantine) can then be exercised
/// without a network or real server.
///
/// Each client's outbound queue carries raw `WsClientMsg` bytes (encoded
/// by `ambre_protocol::ws::encode_client_msg`), and each client's inbound
/// queue delivers `WsServerMsg` values.
pub struct MockBlindRelay {
    /// Documents managed by this relay: client set -> list of updates (with
    /// their global sequence number).
    documents: HashMap<DocumentId, DocumentState>,
    /// Connected clients: user_id -> (client_tx for server->client msgs).
    clients: HashMap<UserId, ambre_bus::conduit::UnboundedSender<WsServerMsg>>,
    /// Global sequence number, monotonically increasing per relay instance.
    next_seq: u64,
}

struct DocumentState {
    updates: Vec<(Seq, Update)>,
    snapshots: Vec<SnapshotId>,
    #[allow(dead_code)]
    subscribers: Vec<UserId>,
}

impl MockBlindRelay {
    pub fn new() -> Self {
        Self {
            documents: HashMap::new(),
            clients: HashMap::new(),
            next_seq: 1,
        }
    }

    /// Register a client and process all of its queued messages.
    /// Returns a queue pair: the client sends raw `WsClientMsg` bytes here,
    /// and receives `WsServerMsg` events on the returned receiver.
    pub fn connect(
        &mut self,
        user_id: UserId,
    ) -> (
        ambre_bus::conduit::UnboundedSender<Vec<u8>>,
        ambre_bus::conduit::Receiver<WsServerMsg>,
    ) {
        let (client_tx, _relay_rx) = ambre_bus::conduit::unbounded::<Vec<u8>>();
        let (relay_tx, client_rx) = ambre_bus::conduit::unbounded::<WsServerMsg>();
        self.clients.insert(user_id, relay_tx);
        (client_tx, client_rx)
    }

    /// Process one encoded client message. Returns `true` if the message was
    /// handled, `false` if decoding failed.
    ///
    /// # Panics
    ///
    /// When the document entry it just inserted is absent, which no execution
    /// reaches.
    pub fn handle_message(&mut self, wire: &[u8]) -> bool {
        let msg = match ambre_protocol::ws::decode_client_msg(wire) {
            Ok(m) => m,
            Err(_) => return false,
        };
        match msg {
            WsClientMsg::Subscribe { document_id } => {
                let _state = self
                    .documents
                    .entry(document_id)
                    .or_insert_with(|| DocumentState {
                        updates: Vec::new(),
                        snapshots: Vec::new(),
                        subscribers: Vec::new(),
                    });
                // Send existing updates to the new subscriber.
                // (In a real test, the subscriber ID is tracked; here we broadcast.)
                let doc_state = self.documents.get(&document_id).expect("just inserted");
                for (_seq, update) in &doc_state.updates {
                    for tx in self.clients.values() {
                        let _ = tx.send(WsServerMsg::Update {
                            seq: Seq(self.next_seq),
                            update: update.clone(),
                        });
                    }
                }
            }
            WsClientMsg::Unsubscribe { document_id: _ } | WsClientMsg::Authenticate { .. } => {}
            WsClientMsg::Update(update) => {
                let doc_id = update.public_data.document_id;
                let state = self
                    .documents
                    .entry(doc_id)
                    .or_insert_with(|| DocumentState {
                        updates: Vec::new(),
                        snapshots: Vec::new(),
                        subscribers: Vec::new(),
                    });
                let seq = self.next_seq;
                self.next_seq += 1;
                state.updates.push((Seq(seq), update.clone()));
                for tx in self.clients.values() {
                    let _ = tx.send(WsServerMsg::Update {
                        seq: Seq(seq),
                        update: update.clone(),
                    });
                }
            }
            WsClientMsg::Ephemeral(eph) => {
                for tx in self.clients.values() {
                    let _ = tx.send(WsServerMsg::Ephemeral(eph.clone()));
                }
            }
            WsClientMsg::RequestMissing {
                document_id,
                after_seq,
            } => {
                if let Some(state) = self.documents.get(&document_id) {
                    let missing: Vec<_> = state
                        .updates
                        .iter()
                        .filter(|(s, _)| s.0 > after_seq.0)
                        .map(|(s, u)| ambre_protocol::messages::SeqUpdate {
                            seq: *s,
                            update: u.clone(),
                        })
                        .collect();
                    if !missing.is_empty() {
                        for tx in self.clients.values() {
                            let _ = tx.send(WsServerMsg::MissingUpdates {
                                document_id,
                                updates: missing.clone(),
                            });
                        }
                    }
                }
            }
            WsClientMsg::SubscribeChannel { channel_id: _ } => {}
            WsClientMsg::UnsubscribeChannel { channel_id: _ } => {}
            WsClientMsg::ChannelUpdate { .. } => {}
            WsClientMsg::ChannelEphemeral { .. } => {}
        }
        true
    }

    /// Drain all pending client messages and process them through the relay.
    pub fn drain_client(&mut self, client_rx: &ambre_bus::conduit::Receiver<Vec<u8>>) -> usize {
        let mut count = 0;
        while let Ok(wire) = client_rx.try_recv() {
            if self.handle_message(&wire) {
                count += 1;
            }
        }
        count
    }

    /// Create a snapshot for a document and notify subscribers.
    pub fn create_snapshot(&mut self, document_id: DocumentId, snapshot_id: SnapshotId) {
        let state = self
            .documents
            .entry(document_id)
            .or_insert_with(|| DocumentState {
                updates: Vec::new(),
                snapshots: Vec::new(),
                subscribers: Vec::new(),
            });
        state.snapshots.push(snapshot_id);
        for tx in self.clients.values() {
            let _ = tx.send(WsServerMsg::SnapshotAvailable {
                document_id,
                snapshot_id,
            });
        }
    }
}

impl Default for MockBlindRelay {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ambre_bus::DocumentId;
    use ambre_core::PublicKey;
    use ambre_protocol::messages::Clock;
    use ambre_protocol::public_data::UpdatePublicData;
    use ambre_protocol::ws::encode_client_msg;

    #[test]
    fn relay_forwards_update_to_all_subscribers() {
        let mut relay = MockBlindRelay::new();
        let user_a = UserId::new();
        let user_b = UserId::new();
        let doc = DocumentId::new();

        let (_tx_a, rx_a) = relay.connect(user_a);
        let (_tx_b, rx_b) = relay.connect(user_b);

        let update = Update {
            ciphertext: ambre_protocol::messages::Ciphertext(b"cipher".to_vec()),
            nonce: ambre_protocol::messages::Nonce([0u8; 24]),
            public_data: UpdatePublicData {
                document_id: doc,
                pub_key: PublicKey::from_array([1u8; 32]),
                key_id: ambre_core::KeyId::from_array([2u8; 16]),
                snapshot_id: SnapshotId::new(),
                clock: Clock(1),
            },
            commitment_tag: ambre_protocol::messages::CommitmentTag([0u8; 32]),
            signature: ambre_protocol::messages::Signature([0u8; 64]),
        };
        let wire = encode_client_msg(&WsClientMsg::Update(update));
        relay.handle_message(&wire);

        assert!(matches!(rx_a.try_recv(), Ok(WsServerMsg::Update { .. })));
        assert!(matches!(rx_b.try_recv(), Ok(WsServerMsg::Update { .. })));
    }
}
