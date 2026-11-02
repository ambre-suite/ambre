//! The word count: four tallies of the text, and the page count beside them
//! (`features/ambre-spec-prose-word-count.md`).
//!
//! Nothing here writes. The tallies are a function of the text, recomputed on
//! demand and stored nowhere (`PROSE-FEAT-018`). The page count is not a tally
//! at all: the engine stamps it from the layout it last took.
//!
//! **The units.** A character is a grapheme cluster, the unit a reader sees
//! (`PROSE-FEAT-039`). A word is a UAX #29 word segment holding a letter or a
//! digit, so a string of punctuation counts nothing and `3.14` counts one. A
//! paragraph is a text block holding a visible character, so the empty
//! paragraph a new document opens on counts nothing (`PROSE-FEAT-122`).

use loro::{LoroDoc, LoroTree, TreeID};
use unicode_segmentation::UnicodeSegmentation;

use crate::doc;
use crate::events::{GraphemeCount, ParagraphCount, WordCount, WordTotal};

/// The four text tallies of the whole document, `pages` left at zero.
///
/// Every text block of the tree is counted, the ones inside a table cell
/// included, in the order of the document flow. A table of contents stores no
/// text, so it adds nothing.
#[must_use]
pub fn compute_word_count(document: &LoroDoc) -> WordCount {
    let mut count = WordCount::default();
    let Some(tree) = doc::blocks_read(document) else {
        return count;
    };
    let mut flow = Vec::new();
    // The flow starts at the body roots, so a band is never counted.
    for root in doc::roots(&tree) {
        flow.push(root);
        collect(&tree, Some(root), &mut flow);
    }
    for id in flow {
        let has_body = doc::block_type_of(&tree, &id).is_some_and(|t| t.has_body());
        if has_body {
            tally(&doc::get_block_text(document, &id).to_string(), &mut count);
        }
    }
    count
}

/// Every live block under `parent`, depth first, which is the document flow.
fn collect(tree: &LoroTree, parent: Option<TreeID>, out: &mut Vec<TreeID>) {
    for id in doc::children_of(tree, parent) {
        out.push(id);
        collect(tree, Some(id), out);
    }
}

/// Add one block's text to the running tallies.
///
/// The break between two paragraphs is whitespace the tree does not store. It
/// counts once with the spaces, between two blocks that each hold a visible
/// character, and never without them (`PROSE-FEAT-120`).
fn tally(text: &str, count: &mut WordCount) {
    let mut clusters = 0_usize;
    let mut visible = 0_usize;
    for cluster in text.graphemes(true) {
        clusters += 1;
        if !cluster.chars().all(char::is_whitespace) {
            visible += 1;
        }
    }
    // A document past four billion clusters reads as the largest figure
    // rather than wrapping: every tally holds at its top.
    count.words = count
        .words
        .plus(WordTotal::of_len(text.unicode_words().count()));
    count.characters = count.characters.plus(GraphemeCount::of_len(visible));
    count.characters_with_spaces = count
        .characters_with_spaces
        .plus(GraphemeCount::of_len(clusters));
    if visible == 0 {
        return;
    }
    if count.paragraphs > ParagraphCount::ZERO {
        count.characters_with_spaces = count.characters_with_spaces.incremented();
    }
    count.paragraphs = count.paragraphs.incremented();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::{BlockInit, ProseCommand, block_id};
    use crate::logic::ProseState;
    use ambre_bus::app::prose::DocChar;
    use ambre_core::index::{Count, Pos};

    /// A document of one paragraph per string, in order.
    fn document_of(paragraphs: &[&str]) -> ProseState {
        let mut state =
            ProseState::with_peer(ambre_core::PeerId::new(1)).expect("a document is built");
        let mut last = state.root_blocks()[0];
        for (index, text) in paragraphs.iter().enumerate() {
            if index > 0 {
                state.handle_command(ProseCommand::InsertBlock {
                    parent_id: None,
                    after_id: Some(last),
                    init: BlockInit::Paragraph,
                    style_id: None,
                });
                last = *state.root_blocks().last().expect("a block was inserted");
            }
            state.handle_command(ProseCommand::InsertText {
                block_id: last,
                pos: Pos::<DocChar>::ZERO,
                text: (*text).to_owned(),
            });
        }
        state
    }

    fn count_of(paragraphs: &[&str]) -> WordCount {
        compute_word_count(document_of(paragraphs).document())
    }

    #[test]
    fn an_empty_document_counts_zero_for_the_four_tallies() {
        let state = ProseState::with_peer(ambre_core::PeerId::new(1)).expect("a document is built");
        assert_eq!(compute_word_count(state.document()), WordCount::default());
    }

    #[test]
    fn a_character_is_a_grapheme_cluster_and_not_a_code_point() {
        // An accented letter written as two code points, and a family emoji
        // joined from seven: two characters to the reader.
        let count =
            count_of(&["e\u{301}\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}\u{200D}\u{1F466}"]);
        assert_eq!(count.characters.get(), 2);
        assert_eq!(count.characters_with_spaces.get(), 2);
    }

    #[test]
    fn whitespace_and_the_break_between_paragraphs_count_only_with_the_spaces() {
        let count = count_of(&["one two", "three\tfour"]);
        assert_eq!(count.characters.get(), 15);
        // Two spaces, one tab, and the break between the two paragraphs.
        assert_eq!(count.characters_with_spaces.get(), 18);
        assert_eq!(count.paragraphs.get(), 2);
        assert_eq!(count.words.get(), 4);
    }

    #[test]
    fn a_word_holds_a_letter_or_a_digit() {
        let count = count_of(&["-- ... well-known 3.14 !"]);
        // `well` and `known` split at the hyphen; the number is one word, and
        // the punctuation counts nothing.
        assert_eq!(count.words.get(), 3);
    }

    #[test]
    fn an_empty_paragraph_between_two_is_neither_a_paragraph_nor_a_break() {
        let count = count_of(&["a", "", "b"]);
        assert_eq!(count.paragraphs.get(), 2);
        assert_eq!(count.characters_with_spaces.get(), 3);
    }

    #[test]
    fn the_text_of_a_table_cell_is_counted() {
        let mut state = document_of(&["before"]);
        let root = state.root_blocks()[0];
        state.handle_command(ProseCommand::InsertTable {
            parent_id: None,
            after_id: Some(root),
            rows: Count::<crate::events::grid::Row>::new(1),
            cols: Count::<crate::events::grid::Col>::new(1),
        });
        let table = crate::logic::tables::tests::the_table(&state);
        let cell = crate::logic::tables::tests::cell_at(&state, table, 0, 0);
        let paragraph = crate::logic::tables::tests::first_block_of(&state, cell);
        state.handle_command(ProseCommand::InsertText {
            block_id: block_id(paragraph),
            pos: Pos::<DocChar>::ZERO,
            text: "inside".to_owned(),
        });
        let count = compute_word_count(state.document());
        assert_eq!(count.words.get(), 2);
        assert_eq!(count.paragraphs.get(), 2);
    }

    #[test]
    fn counting_writes_nothing_to_the_document() {
        let state = document_of(&["some words", "and more"]);
        let before = state.document().oplog_vv();
        let _ = compute_word_count(state.document());
        assert_eq!(state.document().oplog_vv(), before);
    }
}
