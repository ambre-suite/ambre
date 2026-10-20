//! The deltas that wait on an operation this client does not hold
//! (`ambre-spec-op-authorship.md`, `AUTHOR-025`).
//!
//! The CRDT engine keeps a change whose dependencies it lacks, and applies it
//! once they arrive. A dependency in a later delta arrives with it. One under a
//! PeerID no session signs, such as another build's scaffolding, never does.
//! Nothing then tells the user why the document stopped short.
//!
//! So each delta's missing dependencies are noted at its import, and dropped as
//! the document grows. One still missing once every delta below it is held
//! can only be an operation no delta carries, and the document is reported.

use std::collections::BTreeMap;

use ambre_core::Seq;
use loro::{ID, LoroDoc};

/// The relay's number of each imported delta still missing a dependency, and
/// the dependencies it misses.
#[derive(Debug, Default)]
pub(crate) struct Waiting {
    deps: BTreeMap<Seq, Vec<ID>>,
    /// Set once the document was reported, so the user is told once per open.
    reported: bool,
}

impl Waiting {
    /// Note what the delta numbered `seq` waits on, read after its import.
    /// A seq of 0 is no relay's number and is ignored.
    pub(crate) fn note(&mut self, seq: Seq, payload: &[u8], doc: &LoroDoc) {
        if seq == Seq::ZERO {
            return;
        }
        let missing = missing(payload, doc);
        if !missing.is_empty() {
            self.deps.insert(seq, missing);
        }
    }

    /// Drop every dependency the document now holds, and every delta left
    /// with none.
    pub(crate) fn settle(&mut self, doc: &LoroDoc) {
        let held = doc.oplog_vv();
        self.deps.retain(|_, ids| {
            ids.retain(|id| !held.includes_id(*id));
            !ids.is_empty()
        });
    }

    /// `true` the first time a delta at or below `held_through` still waits.
    /// Every delta below it is held, so what it waits on never arrives.
    pub(crate) fn stalled(&mut self, held_through: Seq) -> bool {
        if self.reported || self.deps.range(..=held_through).next().is_none() {
            return false;
        }
        self.reported = true;
        true
    }
}

/// The dependencies of a delta the document does not hold.
fn missing(payload: &[u8], doc: &LoroDoc) -> Vec<ID> {
    let Ok(meta) = LoroDoc::decode_import_blob_meta(payload, false) else {
        return Vec::new();
    };
    let held = doc.oplog_vv();
    meta.start_frontiers
        .iter()
        .filter(|id| !held.includes_id(*id))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use loro::{ExportMode, VersionVector};

    /// A document under `peer` holding `text` on top of `base`'s history.
    fn on_top(base: &LoroDoc, peer: u64, text: &str) -> (LoroDoc, Vec<u8>) {
        let doc = LoroDoc::new();
        doc.import(&base.export(ExportMode::all_updates()).expect("export"))
            .expect("import");
        doc.set_peer_id(peer).expect("peer");
        let from: VersionVector = doc.oplog_vv();
        doc.get_text("t").insert(0, text).expect("insert");
        doc.commit();
        let delta = doc.export(ExportMode::updates(&from)).expect("export");
        (doc, delta)
    }

    fn scaffold(peer: u64) -> LoroDoc {
        let doc = LoroDoc::new();
        doc.set_peer_id(peer).expect("peer");
        doc.get_map("meta").insert("v", 1).expect("insert");
        doc.commit();
        doc
    }

    #[test]
    fn a_delta_on_a_scaffold_the_reader_never_wrote_waits_on_it() {
        let (_, delta) = on_top(&scaffold(7), 1, "a");
        let reader = scaffold(8);
        reader.import(&delta).expect("import");
        let mut waiting = Waiting::default();
        waiting.note(ambre_core::Seq(1), &delta, &reader);
        waiting.settle(&reader);
        assert!(
            waiting.stalled(ambre_core::Seq(1)),
            "the scaffold of peer 7 never arrives"
        );
        assert!(
            !waiting.stalled(ambre_core::Seq(1)),
            "the document is reported once"
        );
    }

    #[test]
    fn a_delta_on_the_readers_own_scaffold_waits_on_nothing() {
        let (_, delta) = on_top(&scaffold(7), 1, "a");
        let reader = scaffold(7);
        reader.import(&delta).expect("import");
        let mut waiting = Waiting::default();
        waiting.note(ambre_core::Seq(1), &delta, &reader);
        assert!(!waiting.stalled(ambre_core::Seq(1)));
    }

    #[test]
    fn a_delta_waiting_on_one_not_yet_held_is_not_reported() {
        let base = scaffold(7);
        let (first_doc, first) = on_top(&base, 1, "a");
        let (_, second) = on_top(&first_doc, 2, "b");
        let reader = scaffold(7);
        reader.import(&second).expect("import");
        let mut waiting = Waiting::default();
        waiting.note(ambre_core::Seq(2), &second, &reader);
        assert!(
            !waiting.stalled(ambre_core::Seq(0)),
            "seq 1 is not held yet"
        );
        reader.import(&first).expect("import");
        waiting.settle(&reader);
        assert!(
            !waiting.stalled(ambre_core::Seq(2)),
            "seq 1 carried what seq 2 waited on"
        );
    }
}
