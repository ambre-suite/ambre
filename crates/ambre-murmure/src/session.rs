//! The session identity of one Murmure document
//! (`ambre-spec-op-authorship.md section 3`).
//!
//! Every channel document and every space document is opened on its own.
//! Each gets its own PeerID from the Sync worker. A document writes
//! nothing before its PeerID lands: an operation under any other PeerID never
//! travels (`AUTHOR-029`).

use ambre_core::authorship::{PEERS_MAP, PEERS_ORIGIN, peer_key};
use loro::{CommitOptions, ExportMode, IdSpan, LoroDoc};

use crate::error::MurmureError;

/// The PeerID one document writes under in this session, and how far its
/// operations have left for the relay.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Session {
    peer: u64,
    /// The counter of this PeerID up to which every operation was exported.
    sent: i32,
}

impl Session {
    /// Write `doc` under `peer` from now on, and write its registry entry.
    ///
    /// The entry is not exported here: it rides with the first delta, and
    /// never travels alone (`AUTHOR-009`).
    ///
    /// # Errors
    /// [`MurmureError::Crdt`] when Loro refuses the PeerID or the write.
    pub(crate) fn adopt(doc: &LoroDoc, peer: u64, entry: Vec<u8>) -> Result<Self, MurmureError> {
        doc.commit();
        doc.set_peer_id(peer)?;
        let sent = doc.oplog_vv().get(&peer).copied().unwrap_or(0);
        doc.get_map(PEERS_MAP).insert(&peer_key(peer), entry)?;
        doc.commit_with(CommitOptions::new().origin(PEERS_ORIGIN));
        Ok(Self { peer, sent })
    }

    /// The PeerID this session writes under.
    pub(crate) fn peer(self) -> u64 {
        self.peer
    }

    /// Commit the pending writes, and export what this session wrote since its
    /// last export: its own operations alone, whatever arrived meanwhile
    /// (`AUTHOR-008`). The first export carries the registry entry.
    ///
    /// # Errors
    /// [`MurmureError::Crdt`] when Loro refuses the export.
    pub(crate) fn export(&mut self, doc: &LoroDoc) -> Result<Vec<u8>, MurmureError> {
        doc.commit();
        let end = doc.oplog_vv().get(&self.peer).copied().unwrap_or(0);
        let span = IdSpan::new(self.peer, self.sent, end);
        let delta = doc.export(ExportMode::updates_in_range(vec![span]))?;
        self.sent = end;
        Ok(delta)
    }

    /// [`Session::export`] as a JSON update, the form a segment delta travels
    /// in (`MURMURE-223`). The range starts where this session last exported,
    /// and holds no other PeerID.
    ///
    /// # Errors
    /// [`MurmureError::Crdt`] when the update does not serialise.
    pub(crate) fn export_json(&mut self, doc: &LoroDoc) -> Result<Vec<u8>, MurmureError> {
        doc.commit();
        let end = doc.oplog_vv();
        let mut start = end.clone();
        start.insert(self.peer, self.sent);
        let update = doc.export_json_updates_without_peer_compression(&start, &end);
        self.sent = end.get(&self.peer).copied().unwrap_or(0);
        crate::delta::encode(&update)
    }
}

/// A fresh PeerID per call, as the Sync worker issues one per opened document.
#[cfg(test)]
pub(crate) fn next_peer() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0x1000);
    NEXT.fetch_add(1, Ordering::Relaxed)
}
