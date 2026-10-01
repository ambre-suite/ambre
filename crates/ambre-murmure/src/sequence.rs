//! Where each message of a segment stands in the relay's order
//! (`features/ambre-spec-murmure-unread.md section 3`).
//!
//! A read cursor is a relay sequence number, and the relay numbers an update
//! after the author wrote it. So no message carries its own number. The engine
//! learns it from the deliveries. An update numbered `seq` holds a range of
//! operations, and a message created in that range is numbered `seq`. A snapshot hides its updates, so the member who exports one
//! appends the numbers it knows, framed before the Loro bytes. The relay sees
//! the frame sealed with the rest of the snapshot.

use std::collections::HashMap;

use ambre_core::{MessageId, Seq};
use loro::{Container, ContainerID, ContainerTrait, ID, LoroDoc, ValueOrContainer, VersionVector};

use crate::schema::{self, MESSAGE_ID, MESSAGES};

/// The four bytes a framed snapshot opens with. A Loro blob opens with
/// `loro`, so the two never read as each other.
const FRAME_MAGIC: &[u8; 4] = b"amsq";
/// One table row: a message identifier, then its number.
const ROW_LEN: usize = 16 + 8;

/// Where a payload stands in the relay's order of its document.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Order {
    /// One update, at the number the relay gave it.
    Update(Seq),
    /// A snapshot folding every update up to this number.
    Snapshot(Seq),
}

/// Where one message stands in the relay's order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Position {
    /// The relay numbered the update that created it.
    Exact(Seq),
    /// It came in a snapshot that folds every update up to this number, and
    /// no table gave its own.
    AtMost(Seq),
    /// Not numbered yet: the author's own message, before the relay echoes it.
    Unknown,
}

/// The operations one numbered update holds, per peer.
#[derive(Debug, Clone)]
struct Range {
    seq: Seq,
    start: VersionVector,
    end: VersionVector,
}

impl Range {
    /// Whether the operation `op` lies in the update.
    fn holds(&self, op: ID) -> bool {
        let start = self.start.get(&op.peer).copied().unwrap_or(0);
        let end = self.end.get(&op.peer).copied().unwrap_or(0);
        (start..end).contains(&op.counter)
    }
}

/// The relay's number of each message of one segment.
#[derive(Debug, Clone, Default)]
pub(crate) struct Sequence {
    /// Each message whose number is known. The lowest wins: an operation
    /// re-sent in a later update was created by the first.
    exact: HashMap<MessageId, Seq>,
    /// The horizon of the snapshots delivered: every message they held is
    /// numbered at most this.
    covered: Seq,
    /// The highest number delivered, snapshot or update.
    delivered: Seq,
    /// Updates noted and not yet matched against the document's messages.
    unresolved: Vec<Range>,
}

impl Sequence {
    /// Note one payload the relay delivered. An update keeps its range until
    /// [`Sequence::resolve`] matches it. A snapshot raises the horizon and
    /// adopts the numbers its table gives, up to that horizon.
    pub(crate) fn note(&mut self, order: Order, payload: &[u8]) {
        match order {
            Order::Update(seq) => {
                self.delivered = self.delivered.max(seq);
                let Ok(meta) = LoroDoc::decode_import_blob_meta(loro_part(payload), false) else {
                    tracing::warn!(
                        "[Murmure] update {} has no readable range; not numbered",
                        seq.0
                    );
                    return;
                };
                self.unresolved.push(Range {
                    seq,
                    start: meta.partial_start_vv,
                    end: meta.partial_end_vv,
                });
            }
            Order::Snapshot(covered) => {
                self.covered = self.covered.max(covered);
                self.delivered = self.delivered.max(covered);
                // A row past the horizon is a number no update of this
                // snapshot had: a writer's mistake, read as no number.
                for (id, seq) in table(payload).into_iter().flatten() {
                    if seq <= covered {
                        self.adopt(id, seq);
                    }
                }
            }
        }
    }

    /// Match every noted update against the messages of `doc`, once its
    /// bytes are in.
    pub(crate) fn resolve(&mut self, doc: &LoroDoc) {
        if !self.unresolved.is_empty() {
            self.resolve_created(&created(doc));
        }
    }

    /// Match every noted update against `messages`, each with the operation
    /// that created it, as [`created`] reads them.
    pub(crate) fn resolve_created(&mut self, messages: &[Created]) {
        let ranges = std::mem::take(&mut self.unresolved);
        if ranges.is_empty() {
            return;
        }
        for &(id, op) in messages {
            let found = ranges.iter().filter(|r| r.holds(op)).map(|r| r.seq).min();
            if let Some(seq) = found {
                self.adopt(id, seq);
            }
        }
    }

    fn adopt(&mut self, id: MessageId, seq: Seq) {
        self.exact
            .entry(id)
            .and_modify(|known| *known = (*known).min(seq))
            .or_insert(seq);
    }

    /// Where `id` stands.
    pub(crate) fn position(&self, id: MessageId) -> Position {
        match self.exact.get(&id) {
            Some(&seq) => Position::Exact(seq),
            None if self.covered > Seq::ZERO => Position::AtMost(self.covered),
            None => Position::Unknown,
        }
    }

    /// The highest number delivered for the document.
    pub(crate) fn delivered(&self) -> Seq {
        self.delivered
    }

    /// The numbers this member knows, framed before `loro`: the snapshot it
    /// hands the relay. With no number known, the bare Loro bytes.
    pub(crate) fn frame(&self, loro: Vec<u8>) -> Vec<u8> {
        if self.exact.is_empty() {
            return loro;
        }
        // Sorted, so two members knowing the same numbers frame the same bytes.
        let mut rows: Vec<([u8; 16], u64)> = self
            .exact
            .iter()
            .map(|(id, seq)| (*id.as_bytes(), seq.0))
            .collect();
        rows.sort_unstable();
        let count = u32::try_from(rows.len()).unwrap_or(u32::MAX);
        let mut out = Vec::with_capacity(8 + rows.len() * ROW_LEN + loro.len());
        out.extend_from_slice(FRAME_MAGIC);
        out.extend_from_slice(&count.to_le_bytes());
        for (id, seq) in rows.into_iter().take(count as usize) {
            out.extend_from_slice(&id);
            out.extend_from_slice(&seq.to_le_bytes());
        }
        out.extend_from_slice(&loro);
        out
    }
}

/// A message with the operation that created it.
pub(crate) type Created = (MessageId, ID);

/// Each message of `doc` with the operation that created it. A message with
/// no readable identifier is skipped: the view skips it too.
pub(crate) fn created(doc: &LoroDoc) -> Vec<Created> {
    let list = doc.get_list(MESSAGES);
    let mut out = Vec::with_capacity(list.len());
    list.for_each(|item| {
        let ValueOrContainer::Container(Container::Map(map)) = item else {
            return;
        };
        let ContainerID::Normal { peer, counter, .. } = map.id() else {
            return;
        };
        let Some(id) = schema::string(&map, MESSAGE_ID).and_then(|s| MessageId::parse_str(&s).ok())
        else {
            return;
        };
        out.push((id, ID::new(peer, counter)));
    });
    out
}

/// The frame of a snapshot and the Loro bytes after it.
struct Framed<'a> {
    rows: &'a [u8],
    loro: &'a [u8],
}

fn split(payload: &[u8]) -> Option<Framed<'_>> {
    let rest = payload.strip_prefix(FRAME_MAGIC)?;
    let (count, rest) = rest.split_first_chunk::<4>()?;
    let len = usize::try_from(u32::from_le_bytes(*count))
        .ok()?
        .checked_mul(ROW_LEN)?;
    (rest.len() >= len).then(|| {
        let (rows, loro) = rest.split_at(len);
        Framed { rows, loro }
    })
}

/// The Loro bytes of a payload: past the frame when it has one. A payload
/// that opens with the magic and holds no whole frame is handed on as it is.
/// Loro refuses it, so a truncated table never imports.
pub(crate) fn loro_part(payload: &[u8]) -> &[u8] {
    split(payload).map_or(payload, |f| f.loro)
}

/// The rows a framed snapshot carries, `None` for a bare one.
fn table(payload: &[u8]) -> Option<impl Iterator<Item = (MessageId, Seq)> + '_> {
    let framed = split(payload)?;
    Some(framed.rows.chunks_exact(ROW_LEN).filter_map(|row| {
        let (id, seq) = row.split_first_chunk::<16>()?;
        let seq = Seq(u64::from_le_bytes(seq.try_into().ok()?));
        Some((MessageId::from_bytes(*id), seq))
    }))
}
