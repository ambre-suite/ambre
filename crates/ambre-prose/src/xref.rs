//! What a cross reference stores and what it shows
//! (`features/ambre-spec-prose-cross-references.md`).
//!
//! A reference is one carrier character marked `ref`, whose value is a
//! length-prefixed record in hex (`PROSE-FEAT-072`). The record names the
//! target by its tree id, never by its text (`PROSE-FEAT-073`). What it shows
//! is resolved here from a layout and the heading numbers, and never written
//! back (`PROSE-FEAT-129`).

use loro::{LoroDoc, TreeID};
use serde::{Deserialize, Serialize};

use crate::doc;
use crate::events::{CrossRefKind, ResolvedCrossRef, block_id};
use crate::layout::pass::PageLayout;
use crate::logic::Roots;
use crate::numbering;

/// The record's wire form. `postcard` prefixes a label with its length, so
/// no character of it can be read as a delimiter.
#[derive(Serialize, Deserialize)]
struct Record {
    peer: ambre_core::PeerId,
    counter: ambre_core::OpCounter,
    kind: CrossRefKind,
}

/// The value the carrier mark of a reference stores: its [`Record`] in hex,
/// two digits a byte. [`decode`] reads it back. It spells a label of the
/// document, so it has no `Debug`.
#[derive(Clone, PartialEq, Eq)]
pub struct CrossRefMark(String);

impl CrossRefMark {
    /// The hex text, as the mark stores it.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The mark value of a reference to `target`.
#[must_use]
pub fn encode(target: TreeID, kind: &CrossRefKind) -> CrossRefMark {
    let record = Record {
        peer: ambre_core::PeerId::new(target.peer),
        counter: ambre_core::OpCounter::new(target.counter),
        kind: kind.clone(),
    };
    CrossRefMark(
        postcard::to_allocvec(&record)
            .map(|bytes| bytes.iter().map(|byte| format!("{byte:02x}")).collect())
            .unwrap_or_default(),
    )
}

/// The target and the kind a mark value names, or `None` for a value that is
/// not a record this build writes.
#[must_use]
pub fn decode(value: &str) -> Option<(TreeID, CrossRefKind)> {
    if !value.len().is_multiple_of(2) || !value.is_ascii() {
        return None;
    }
    let bytes: Option<Vec<u8>> = (0..value.len())
        .step_by(2)
        .map(|at| {
            value
                .get(at..at + 2)
                .and_then(|pair| u8::from_str_radix(pair, 16).ok())
        })
        .collect();
    let record: Record = postcard::from_bytes(&bytes?).ok()?;
    Some((
        TreeID::new(record.peer.get(), record.counter.get()),
        record.kind,
    ))
}

/// What a reference to `target` shows, or `None` for the broken mark
/// (`PROSE-FEAT-014`).
///
/// A label needs no target and survives it (`PROSE-UI-048`). Otherwise the
/// target is read by its own id and no other: a gone target is broken, never
/// a neighbour's value (`PROSE-FEAT-132`). A page is the one the target starts
/// on in `layout` (`PROSE-FEAT-130`); a section is the number its heading is
/// drawn with (`PROSE-FEAT-131`). A document without the tree holds no
/// target, and the read attaches nothing (`PROSE-142`).
#[must_use]
pub fn resolve(
    document: &LoroDoc,
    roots: &Roots,
    layout: Option<&PageLayout>,
    target: &TreeID,
    kind: &CrossRefKind,
) -> Option<String> {
    if let CrossRefKind::CustomText(label) = kind {
        return Some(label.clone());
    }
    let tree = doc::blocks_read(document, roots)?;
    doc::node_of(&tree, target)?;
    match kind {
        CrossRefKind::PageNumber => layout?
            .blocks
            .iter()
            .find(|placed| placed.block_id == *target)
            .map(|placed| placed.page_start.number().to_string()),
        CrossRefKind::HeadingNumber => {
            numbering::compute_heading_number(&tree, target).map(|number| number.to_string())
        }
        CrossRefKind::CustomText(_) => None,
    }
}

/// Every reference of the document, in document order, resolved against
/// `layout`. A document without the tree holds no reference, and the read
/// attaches nothing (`PROSE-142`).
#[must_use]
pub fn resolve_all(
    document: &LoroDoc,
    roots: &Roots,
    layout: Option<&PageLayout>,
) -> Vec<ResolvedCrossRef> {
    doc::anchors(document, roots, crate::mark::MarkRoot::CrossRef, decode)
        .into_iter()
        .map(|anchor| {
            let (target, kind) = anchor.value;
            ResolvedCrossRef {
                block_id: block_id(anchor.block),
                offset: anchor.offset,
                target: block_id(target),
                shown: resolve(document, roots, layout, &target, &kind),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_label_holding_any_character_reads_back_whole() {
        let target = TreeID::new(7, 42);
        for label in ["see: 1:2", "a|b,c", "", "\u{e9}t\u{e9} \u{1f600}"] {
            let kind = CrossRefKind::CustomText(label.to_owned());
            assert_eq!(decode(encode(target, &kind).as_str()), Some((target, kind)));
        }
        for kind in [CrossRefKind::PageNumber, CrossRefKind::HeadingNumber] {
            assert_eq!(decode(encode(target, &kind).as_str()), Some((target, kind)));
        }
    }

    #[test]
    fn a_value_is_a_length_prefixed_record_and_never_a_joined_string() {
        let target = TreeID::new(1, 2);
        let value = encode(target, &CrossRefKind::CustomText(":".to_owned()));
        assert!(value.as_str().bytes().all(|byte| byte.is_ascii_hexdigit()));
        assert!(
            !value.as_str().contains(':'),
            "no separator reaches the stored value"
        );
        assert_eq!(decode("zz"), None);
        // The kind is the record's last byte: a code no kind holds is not read.
        let page = encode(target, &CrossRefKind::PageNumber);
        let head = page
            .as_str()
            .strip_suffix("00")
            .expect("a page number ends in its code");
        assert_eq!(
            decode(&format!("{head}00")),
            Some((target, CrossRefKind::PageNumber))
        );
        assert_eq!(decode(&format!("{head}03")), None);
        assert_eq!(decode("1:2"), None);
    }
}
