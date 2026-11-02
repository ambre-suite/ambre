//! The session identity of one Murmure document
//! (`ambre-spec-op-authorship.md section 3`).
//!
//! Every channel document and every space document is opened on its own.
//! Each gets its own PeerID from the Sync worker. A document writes
//! nothing before its PeerID lands: an operation under any other PeerID never
//! travels (`AUTHOR-029`).

use ambre_core::{DeltaBytes, OpCounter, Payload, PeerEntryBytes};
use loro::{CommitOptions, ExportMode, IdSpan, LoroDoc, VersionVector};

use crate::error::MurmureError;

/// The PeerID one document writes under in this session, and how far its
/// operations have left for the relay.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Session {
    peer: ambre_core::PeerId,
    /// The counter of this PeerID up to which every operation was exported.
    sent: OpCounter,
}

/// The counter `versions` holds for `peer`: the operations of `peer` they
/// name stop before it. Zero for a peer they do not name.
fn counter_of(versions: &VersionVector, peer: ambre_core::PeerId) -> OpCounter {
    OpCounter::new(versions.get(&peer.get()).copied().unwrap_or(0))
}

impl Session {
    /// Write `doc` under `peer` from now on, and write its registry entry.
    ///
    /// The entry is not exported here: it rides with the first delta, and
    /// never travels alone (`AUTHOR-009`).
    ///
    /// # Errors
    /// [`MurmureError::Crdt`] when Loro refuses the PeerID or the write.
    pub(crate) fn adopt(
        doc: &LoroDoc,
        peer: ambre_core::PeerId,
        entry: Payload<PeerEntryBytes>,
    ) -> Result<Self, MurmureError> {
        doc.commit();
        doc.set_peer_id(peer.get())?;
        let sent = counter_of(&doc.oplog_vv(), peer);
        doc.get_map(ambre_core::crdt::FoundationRoot::Peers.as_str())
            .insert(ambre_core::PeerKey::of(peer).as_str(), entry.into_bytes())?;
        doc.commit_with(
            CommitOptions::new().origin(ambre_core::crdt::CommitOrigin::Peers.as_str()),
        );
        Ok(Self { peer, sent })
    }

    /// The PeerID this session writes under.
    pub(crate) fn peer(self) -> ambre_core::PeerId {
        self.peer
    }

    /// The relay refused every operation of this session past what `kept`
    /// holds, and the document dropped them. The next export starts there.
    pub(crate) fn rewind(&mut self, kept: &VersionVector) {
        self.sent = self.sent.min(counter_of(kept, self.peer));
    }

    /// Commit the pending writes, and export what this session wrote since its
    /// last export: its own operations alone, whatever arrived meanwhile
    /// (`AUTHOR-008`). The first export carries the registry entry.
    ///
    /// # Errors
    /// [`MurmureError::Crdt`] when Loro refuses the export.
    pub(crate) fn export(&mut self, doc: &LoroDoc) -> Result<Payload<DeltaBytes>, MurmureError> {
        doc.commit();
        let end = counter_of(&doc.oplog_vv(), self.peer);
        let span = IdSpan::new(self.peer.get(), self.sent.get(), end.get());
        let delta = doc.export(ExportMode::updates_in_range(vec![span]))?;
        self.sent = end;
        Ok(Payload::<DeltaBytes>::new(delta))
    }

    /// [`Session::export`] as a JSON update, the form a segment delta travels
    /// in (`MURMURE-223`). The range starts where this session last exported,
    /// and holds no other PeerID.
    ///
    /// # Errors
    /// [`MurmureError::Crdt`] when the update does not serialise.
    pub(crate) fn export_json(
        &mut self,
        doc: &LoroDoc,
    ) -> Result<Payload<DeltaBytes>, MurmureError> {
        doc.commit();
        let end = doc.oplog_vv();
        let mut start = end.clone();
        start.insert(self.peer.get(), self.sent.get());
        let update = doc.export_json_updates_without_peer_compression(&start, &end);
        self.sent = counter_of(&end, self.peer);
        crate::delta::encode(&update)
    }
}

/// A fresh PeerID per call, as the Sync worker issues one per opened document.
#[cfg(test)]
pub(crate) fn next_peer() -> ambre_core::PeerId {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0x1000);
    ambre_core::PeerId::new(NEXT.fetch_add(1, Ordering::Relaxed))
}
