//! The table of contents, read off the headings and a layout
//! (`features/ambre-spec-prose-table-of-contents.md section 3`).
//!
//! Nothing here writes. The document holds the two settings of the block and
//! no entry (`PROSE-FEAT-113`). Every entry is a heading, read again on every
//! layout, so it cannot drift from the heading it names (`PROSE-FEAT-003`).
//!
//! The page an entry shows is a fact about one layout (`PROSE-FEAT-060`). The
//! block is itself on a page, so its height moves the pages it lists. The
//! paginator settles the two in a bounded pass (`PROSE-019`).

use std::collections::HashMap;

use loro::{LoroDoc, LoroTree, TreeID, TreeParentId};

use crate::doc;
use crate::events::{PageNumber, TableOfContents, TocEntry, block_id};
use crate::layout::place::BlockLayout;
use crate::numbering;

/// The entries one table of contents lists (`PROSE-FEAT-003`).
///
/// Every heading no deeper than the block's depth, in document order, reading
/// its own text (`PROSE-FEAT-061`). The section number is present only while
/// the block is numbered (`PROSE-FEAT-062`). `pages` maps a heading to the page
/// it starts on. A heading it does not name shows no page.
/// `None` when `toc` names no live table of contents.
#[must_use]
pub fn build(
    document: &LoroDoc,
    tree: &LoroTree,
    toc: &TreeID,
    pages: &HashMap<TreeID, PageNumber>,
) -> Option<TableOfContents> {
    let doc::TocSettings {
        max_level,
        numbered,
    } = doc::toc_settings(tree, toc)?;
    let entries = numbering::section_numbers(tree)
        .into_iter()
        .filter_map(|(heading, number)| {
            let level = doc::heading_level_of(tree, &heading)?.clamped();
            (level <= max_level).then(|| TocEntry {
                block_id: block_id(heading),
                level,
                text: doc::get_block_text(document, &heading).to_string(),
                section_number: numbered.then_some(number),
                page: pages.get(&heading).copied(),
            })
        })
        .collect();
    Some(TableOfContents {
        entries,
        max_level,
        numbered,
    })
}

/// The page every heading starts on in a layout (`PROSE-FEAT-060`).
///
/// A heading placed as a block of its own reads its own page. One inside a
/// table cell is not a placed block, so it reads the page its table starts on.
#[must_use]
pub fn heading_pages(tree: &LoroTree, blocks: &[BlockLayout]) -> HashMap<TreeID, PageNumber> {
    let placed: HashMap<TreeID, PageNumber> = blocks
        .iter()
        .map(|block| (block.block_id, block.page_start.number()))
        .collect();
    let mut pages = HashMap::new();
    for (heading, _) in numbering::section_numbers(tree) {
        let mut current = heading;
        loop {
            if let Some(page) = placed.get(&current) {
                pages.insert(heading, *page);
                break;
            }
            match tree.parent(current) {
                Some(TreeParentId::Node(parent)) => current = parent,
                _ => break,
            }
        }
    }
    pages
}

/// The text a table of contents is measured on: one line per entry, the
/// section number, the heading's text and the page.
///
/// It is what the block paints, so its height is the block's height. And a
/// page number that grows a digit may wrap a line (`PROSE-019`).
#[must_use]
pub fn lines(toc: &TableOfContents) -> String {
    let mut out = String::new();
    for (index, entry) in toc.entries.iter().enumerate() {
        if index > 0 {
            out.push('\n');
        }
        if let Some(number) = &entry.section_number {
            let parts: Vec<String> = number.iter().map(u32::to_string).collect();
            out.push_str(&parts.join("."));
            out.push(' ');
        }
        out.push_str(&entry.text);
        if let Some(page) = entry.page {
            out.push(' ');
            out.push_str(&page.to_string());
        }
    }
    out
}
