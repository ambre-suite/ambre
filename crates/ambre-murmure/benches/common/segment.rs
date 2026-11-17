//! The synthetic segment both the criterion bench and the phone page measure.
//!
//! A segment is built in process, one delta per commit as a client sends it.
//! Most deltas are messages, and a few are reactions or edits. No fixture file
//! is read. The criterion bench and the wasm page of
//! `features/ambre-spec-murmure-phone-measures.md` include this one file, so
//! the phone measures the corpus the desktop figure came from.

use loro::{ExportMode, LoroDoc, LoroList, LoroMap, LoroText, VersionVector};

/// The writers of the channel, one Loro peer each.
const WRITERS: u64 = 20;

/// One synthetic segment: its document, and every delta in the order sent.
pub(crate) struct Segment {
    pub(crate) doc: LoroDoc,
    pub(crate) deltas: Vec<Vec<u8>>,
}

/// A body of 40 to 200 bytes, the length of a chat line.
fn body(i: usize) -> String {
    let words = [
        "ship",
        "the",
        "review",
        "today",
        "après",
        "le",
        "déploiement",
        "ok",
        "merci",
    ];
    let len = 6 + (i * 7919) % 30;
    (0..len)
        .map(|w| words[(i + w * 31) % words.len()])
        .collect::<Vec<_>>()
        .join(" ")
}

/// Commit the pending operations of `doc` and keep them as one delta: every
/// operation after `sent`, the version the previous delta ended on.
fn commit_delta(doc: &LoroDoc, sent: &mut VersionVector, deltas: &mut Vec<Vec<u8>>) {
    doc.commit();
    deltas.push(
        doc.export(ExportMode::updates(sent))
            .expect("export updates"),
    );
    *sent = doc.oplog_vv();
}

/// Build a segment of `count` messages. One message in five draws a reaction
/// and one in twenty an edit, each its own delta, as each is its own gesture.
pub(crate) fn build(count: usize) -> Segment {
    let doc = LoroDoc::new();
    let messages: LoroList = doc.get_list("messages");
    let mut deltas = Vec::with_capacity(count + count / 4);
    let mut sent = VersionVector::default();
    for i in 0..count {
        doc.set_peer_id(1 + (i as u64 * 7) % WRITERS)
            .expect("peer id");
        let message = messages
            .insert_container(i, LoroMap::new())
            .expect("message");
        message.insert("id", format!("m{i:016x}")).expect("id");
        message
            .insert("author", format!("u{:08x}", i % WRITERS as usize))
            .expect("author");
        message
            .insert("ts", 1_700_000_000_000_i64 + i as i64 * 30_000)
            .expect("ts");
        let content = message
            .insert_container("content", LoroText::new())
            .expect("content");
        // The message creates its reactions map, flat and empty, so no
        // reaction ever creates a container (`MURMURE-256`).
        let reactions = message
            .insert_container("reactions", LoroMap::new())
            .expect("reactions");
        content.insert(0, &body(i)).expect("body");
        commit_delta(&doc, &mut sent, &mut deltas);

        if i % 5 == 4 {
            doc.set_peer_id(1 + (i as u64 * 3) % WRITERS)
                .expect("peer id");
            reactions
                .insert(
                    "\u{1F44D}|u00000003",
                    1_700_000_000_000_i64 + i as i64 * 30_000 + 5_000,
                )
                .expect("reaction");
            commit_delta(&doc, &mut sent, &mut deltas);
        }
        if i % 20 == 19 {
            doc.set_peer_id(1 + (i as u64 * 7) % WRITERS)
                .expect("peer id");
            content.insert(0, "edit: ").expect("edit");
            message.insert("edited", true).expect("edited");
            commit_delta(&doc, &mut sent, &mut deltas);
        }
    }
    let segment = Segment { doc, deltas };
    assert_replays(&segment, count);
    segment
}

/// Stop when the deltas do not rebuild the segment: a bench over empty deltas
/// reports a figure that reads as an improvement.
pub(crate) fn assert_replays(segment: &Segment, count: usize) {
    let doc = LoroDoc::new();
    for delta in &segment.deltas {
        doc.import(delta).expect("import");
    }
    assert_eq!(
        doc.get_list("messages").len(),
        count,
        "the deltas miss messages"
    );
    assert_eq!(
        doc.get_deep_value(),
        segment.doc.get_deep_value(),
        "the deltas diverge"
    );
}
