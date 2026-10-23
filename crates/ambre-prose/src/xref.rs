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
use crate::numbering;

/// The record's wire form. `postcard` prefixes a label with its length, so
/// no character of it can be read as a delimiter.
#[derive(Serialize, Deserialize)]
struct Record {
    peer: ambre_core::PeerId,
    counter: ambre_core::OpCounter,
    kind: CrossRefKind,
}

/// The mark value of a reference to `target`.
#[must_use]
pub fn encode(target: TreeID, kind: &CrossRefKind) -> String {
    let record = Record {
        peer: ambre_core::PeerId(target.peer),
        counter: ambre_core::OpCounter(target.counter),
        kind: kind.clone(),
    };
    postcard::to_allocvec(&record)
        .map(|bytes| bytes.iter().map(|byte| format!("{byte:02x}")).collect())
        .unwrap_or_default()
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
    Some((TreeID::new(record.peer.0, record.counter.0), record.kind))
}

/// What a reference to `target` shows, or `None` for the broken mark
/// (`PROSE-FEAT-014`).
///
/// A label needs no target and survives it (`PROSE-UI-048`). Otherwise the
/// target is read by its own id and no other: a gone target is broken, never
/// a neighbour's value (`PROSE-FEAT-132`). A page is the one the target starts
/// on in `layout` (`PROSE-FEAT-130`); a section is the number its heading is
/// drawn with (`PROSE-FEAT-131`).
#[must_use]
pub fn resolve(
    document: &LoroDoc,
    layout: Option<&PageLayout>,
    target: &TreeID,
    kind: &CrossRefKind,
) -> Option<String> {
    if let CrossRefKind::CustomText(label) = kind {
        return Some(label.clone());
    }
    let tree = doc::blocks(document);
    doc::node_of(&tree, target)?;
    match kind {
        CrossRefKind::PageNumber => layout?
            .blocks
            .iter()
            .find(|placed| placed.block_id == *target)
            .map(|placed| placed.page_start.number().to_string()),
        CrossRefKind::SectionNumber => {
            let number = numbering::compute_section_number(&tree, target);
            (!number.is_empty()).then(|| {
                number
                    .iter()
                    .map(u32::to_string)
                    .collect::<Vec<_>>()
                    .join(".")
            })
        }
        CrossRefKind::CustomText(_) => None,
    }
}

/// Every reference of the document, in document order, resolved against
/// `layout`.
#[must_use]
pub fn resolve_all(document: &LoroDoc, layout: Option<&PageLayout>) -> Vec<ResolvedCrossRef> {
    doc::anchors(document, doc::K_CROSS_REF)
        .into_iter()
        .filter_map(|anchor| {
            let (target, kind) = decode(&anchor.value)?;
            Some(ResolvedCrossRef {
                block_id: block_id(anchor.block),
                offset: anchor.offset,
                target: block_id(target),
                shown: resolve(document, layout, &target, &kind),
            })
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
            assert_eq!(decode(&encode(target, &kind)), Some((target, kind)));
        }
        for kind in [CrossRefKind::PageNumber, CrossRefKind::SectionNumber] {
            assert_eq!(decode(&encode(target, &kind)), Some((target, kind)));
        }
    }

    #[test]
    fn a_value_is_a_length_prefixed_record_and_never_a_joined_string() {
        let target = TreeID::new(1, 2);
        let value = encode(target, &CrossRefKind::CustomText(":".to_owned()));
        assert!(value.bytes().all(|byte| byte.is_ascii_hexdigit()));
        assert!(
            !value.contains(':'),
            "no separator reaches the stored value"
        );
        assert_eq!(decode("zz"), None);
        // The kind is the record's last byte: a code no kind holds is not read.
        let page = encode(target, &CrossRefKind::PageNumber);
        let head = page
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
