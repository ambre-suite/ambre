//! The relay's numbers of a document's deltas, against the version each
//! one carries (`ambre-spec-op-authorship.md` section 3).
//!
//! The snapshot check of the Logic runtime keeps one per document it holds.
//! The Murmure runtime keeps one per channel document. So the type lives
//! here, in the one shared crate that joins Loro and the core types
//! (`SRV-083`, `RESID-010`).

use std::collections::BTreeMap;

use ambre_core::{Relay, Seq};
use loro::{Counter, LoroDoc, PeerID, VersionVector};

/// What a client learned of a document's sequenced deltas since its last
/// snapshot: the relay's number of each, and how far it carries its signer.
///
/// The check needs it to tell which operations a snapshot must hold, since a
/// snapshot names only the last delta it covers (`SRV-083`).
#[derive(Debug, Default)]
pub struct SeqLedger {
    /// The last sequence number a snapshot this client imported covers. The
    /// deltas at or below it are folded into `base`.
    floor: Seq<Relay>,
    /// The version the document held at `floor`.
    base: VersionVector,
    /// Each delta past `floor`: its PeerID and the counter it ends at, or
    /// `None` for a delta refused or unreadable, which carries nothing.
    ends: BTreeMap<Seq<Relay>, Option<(PeerID, Counter)>>,
}

impl SeqLedger {
    /// Start again from a snapshot covering every delta up to `covered`, after
    /// which the document stood at `base`. The deltas past `covered` already
    /// recorded stay.
    pub fn reset(&mut self, covered: Seq<Relay>, base: VersionVector) {
        self.floor = covered;
        self.base = base;
        self.ends = self.ends.split_off(&covered.next());
    }

    /// Start again from a snapshot covering every delta up to `covered`, for
    /// a document no snapshot check reads: only its horizon counts. A
    /// snapshot covering less than the floor changes nothing.
    pub fn raise_floor(&mut self, covered: Seq<Relay>) {
        if covered > self.floor {
            self.reset(covered, VersionVector::default());
        }
    }

    /// Record the delta the relay numbered `seq`. `payload` is `None` when the
    /// delta was refused. A seq of 0 is no relay's number and is ignored.
    pub fn record(
        &mut self,
        seq: Seq<Relay>,
        payload: Option<&ambre_core::Payload<ambre_core::DeltaBytes>>,
    ) {
        if seq == Seq::<Relay>::ZERO || seq <= self.floor {
            return;
        }
        let end = payload
            .and_then(|delta| LoroDoc::decode_import_blob_meta(delta.as_bytes(), false).ok())
            .and_then(|meta| {
                let mut peers = meta.partial_end_vv.iter();
                match (peers.next(), peers.next()) {
                    (Some((peer, end)), None) => Some((*peer, *end)),
                    _ => None,
                }
            });
        self.ends.insert(seq, end);
    }

    /// The last seq a snapshot this client imported covers.
    #[must_use]
    pub fn floor(&self) -> Seq<Relay> {
        self.floor
    }

    /// The last seq this client holds with no gap below it. A snapshot built
    /// here covers up to it and no further.
    #[must_use]
    pub fn contiguous_end(&self) -> Seq<Relay> {
        let mut end = self.floor;
        for seq in self.ends.keys() {
            if *seq != end.next() {
                break;
            }
            end = *seq;
        }
        end
    }

    /// Fold every delta up to `covered` into the base, once a snapshot that
    /// covers them matched. A gap below `covered` leaves the ledger as it is.
    pub fn fold_through(&mut self, covered: Seq<Relay>) {
        if covered <= self.floor || self.contiguous_end() < covered {
            return;
        }
        let rest = self.ends.split_off(&covered.next());
        for (peer, end) in std::mem::replace(&mut self.ends, rest)
            .into_values()
            .flatten()
        {
            if self.base.get(&peer).copied().unwrap_or(0) < end {
                self.base.insert(peer, end);
            }
        }
        self.floor = covered;
    }

    /// The version the document held at the floor: what an earlier snapshot
    /// brought, which no delta past the floor traces.
    #[must_use]
    pub fn base(&self) -> &VersionVector {
        &self.base
    }

    /// The PeerID and the end counter of each readable delta past the floor,
    /// up to `covered`. A refused or unreadable delta carries none.
    pub fn ends_through(&self, covered: Seq<Relay>) -> impl Iterator<Item = (PeerID, Counter)> {
        self.ends.range(..=covered).filter_map(|(_, end)| *end)
    }
}
