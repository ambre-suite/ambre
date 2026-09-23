//! What is read off the document and never written to it: the tables of
//! contents and the word count
//! (`features/ambre-spec-prose-table-of-contents.md`,
//! `features/ambre-spec-prose-word-count.md`).
//!
//! A table of contents stores two settings and no entry (`PROSE-FEAT-113`).
//! The word count stores nothing at all (`PROSE-FEAT-018`). Both are answered
//! as events, and the only write this module makes is the block and its two
//! settings.

use loro::{LoroValue, TreeID};

use crate::count;
use crate::doc;
use crate::events::{BlockType, ProseEvent, ProseOutcome, TableOfContents, block_id};
use crate::toc;

use super::ProseState;

impl ProseState {
    /// A table of contents at a place in the tree, listing headings down to
    /// `max_level`, numbered or not (`PROSE-FEAT-115`).
    ///
    /// The block carries the two settings and nothing else: no entry is
    /// written, and no heading is touched (`PROSE-FEAT-116`). The depth is
    /// brought inside one to six (`PROSE-FEAT-292`).
    pub(super) fn insert_toc(
        &mut self,
        parent_id: Option<TreeID>,
        after_id: Option<TreeID>,
        max_level: u8,
        numbered: bool,
    ) -> ProseOutcome {
        let id = match self.mint_placed(parent_id, after_id, BlockType::Toc) {
            Ok(id) => id,
            Err(refused) => return refused,
        };
        self.write_toc_settings(&id, max_level, numbered)
    }

    /// Rewrite the two settings of one table of contents (`PROSE-FEAT-117`).
    ///
    /// Another table of contents keeps its own, and no heading is touched
    /// (`PROSE-FEAT-118`).
    pub(super) fn update_toc_config(
        &mut self,
        id: &TreeID,
        max_level: u8,
        numbered: bool,
    ) -> ProseOutcome {
        if doc::block_type_of(&doc::blocks(&self.doc), id) != Some(BlockType::Toc) {
            return Self::err_no_block(id);
        }
        self.write_toc_settings(id, max_level, numbered)
    }

    /// The shared write of the two commands, then the entries as the settled
    /// layout lists them (`PROSE-FEAT-291`).
    fn write_toc_settings(&mut self, id: &TreeID, max_level: u8, numbered: bool) -> ProseOutcome {
        let tree = doc::blocks(&self.doc);
        let attrs = doc::node_of(&tree, id).and_then(|node| doc::map_at(&node, doc::K_ATTRS));
        let Some(attrs) = attrs else {
            return Self::err_write("the block refused its attribute map");
        };
        let level = i64::from(toc::clamp_level(max_level));
        let written = attrs
            .insert(doc::A_MAX_LEVEL, LoroValue::I64(level))
            .and_then(|()| attrs.insert(doc::A_NUMBERED, LoroValue::Bool(numbered)));
        if let Err(e) = written {
            return Self::err_write(&format!("{e:?}"));
        }
        let mut outcome = self.commit_with(Vec::new());
        // A settings change is a gesture of its own, never a keystroke, so it
        // may pay for the settling pass (`PROSE-084`).
        outcome.display.extend(self.settle_toc());
        outcome
    }

    /// Lay the document out, settle every table of contents against the pages
    /// it lists, and answer one event per block (`PROSE-019`).
    ///
    /// The pass is bounded and it is never run on the path that answers a
    /// keystroke (`PROSE-084`). It writes nothing.
    pub fn settle_toc(&mut self) -> Vec<ProseEvent> {
        let _ = self.paginate(&[]);
        if let Some(paginator) = self.paginator.as_mut() {
            let _ = paginator.settle_toc(&self.doc);
        }
        self.stamp_pages();
        let tree = doc::blocks(&self.doc);
        let pages = match self.paginator.as_ref() {
            Some(paginator) => toc::heading_pages(&tree, &paginator.layout().blocks),
            None => return Vec::new(),
        };
        tree.nodes()
            .into_iter()
            .filter(|id| doc::block_type_of(&tree, id) == Some(BlockType::Toc))
            .filter_map(|id| {
                let built = toc::build(&self.doc, &tree, &id, &pages)?;
                Some(ProseEvent::TocUpdated {
                    block_id: block_id(id),
                    toc: built,
                })
            })
            .collect()
    }

    /// The entries one table of contents lists in the layout on hand, for a
    /// test and for the view's own reads (`PROSE-FEAT-003`).
    #[must_use]
    pub fn table_of_contents(&self, id: &crate::events::BlockId) -> Option<TableOfContents> {
        let tree = doc::blocks(&self.doc);
        let blocks = self
            .paginator
            .as_ref()
            .map(|paginator| paginator.layout().blocks.as_slice())
            .unwrap_or_default();
        let pages = toc::heading_pages(&tree, blocks);
        toc::build(&self.doc, &tree, &crate::events::tree_id(*id), &pages)
    }

    /// Recount the four text tallies and answer the five figures
    /// (`PROSE-FEAT-119`).
    ///
    /// The page count is kept as the last layout stamped it: a recount is not
    /// a layout (`PROSE-FEAT-125`). Nothing is written, so no delta leaves.
    pub(super) fn count_words(&mut self) -> ProseOutcome {
        let pages = self.word_count.pages;
        self.word_count = count::compute_word_count(&self.doc);
        self.word_count.pages = pages;
        let mut display = vec![ProseEvent::WordCountUpdated {
            count: self.word_count,
        }];
        // The same pause refreshes the three lists read off the whole text.
        // A deletion orphans a note or a thread, and a reference's target
        // moves. One walk per pause, never one per keystroke.
        display.push(ProseEvent::FootnotesUpdated {
            notes: crate::note::entries(&self.doc),
        });
        display.push(ProseEvent::CommentsUpdated {
            threads: crate::comment::threads(&self.doc),
        });
        display.push(self.cross_refs_event());
        display.push(self.suggestions_event());
        ProseOutcome::display_only(display)
    }

    /// Take the page count of the layout on hand into the figures, and leave
    /// the four tallies alone (`PROSE-FEAT-126`, `PROSE-FEAT-127`).
    pub(super) fn stamp_pages(&mut self) {
        if let Some(paginator) = self.paginator.as_ref() {
            self.word_count.pages = paginator.layout().page_count;
        }
    }

    /// The five figures as they stand.
    #[must_use]
    pub fn word_count(&self) -> crate::events::WordCount {
        self.word_count
    }
}

#[cfg(test)]
mod tests {
    use loro::LoroValue;

    use super::ProseState;
    use crate::count::compute_word_count;
    use crate::doc;
    use crate::events::{
        BlockId, BlockInit, BlockType, ProseCommand, ProseEvent, TableOfContents, tree_id,
    };
    use crate::ui::{ProseUiState, RECOUNT_DELAY_MS};

    /// A document laid out on the shipped faces, holding nothing yet.
    fn fresh() -> ProseState {
        let mut state = ProseState::with_peer(1).expect("a document is built");
        state.load_faces(&crate::layout::fonts::shipped_faces());
        state
    }

    /// Append one block at the root, after the last one, and answer its id.
    fn append(state: &mut ProseState, init: BlockInit, text: &str) -> BlockId {
        let last = *state.root_blocks().last().expect("a root block");
        state.handle_command(ProseCommand::InsertBlock {
            parent_id: None,
            after_id: Some(last),
            init,
            style_id: None,
        });
        let id = *state.root_blocks().last().expect("the block was inserted");
        if !text.is_empty() {
            state.handle_command(ProseCommand::InsertText {
                block_id: id,
                pos: 0,
                text: text.to_owned(),
            });
        }
        id
    }

    fn heading(state: &mut ProseState, level: u8, text: &str) -> BlockId {
        append(state, BlockInit::Heading { level }, text)
    }

    /// A table of contents after the first block, answering its id and the
    /// events the insert produced.
    fn insert_toc(state: &mut ProseState, max_level: u8, numbered: bool) -> BlockId {
        let first = state.root_blocks()[0];
        state.handle_command(ProseCommand::InsertToc {
            parent_id: None,
            after_id: Some(first),
            max_level,
            numbered,
        });
        state
            .root_blocks()
            .into_iter()
            .find(|id| state.block_type_of(id) == Some(BlockType::Toc))
            .expect("a table of contents was inserted")
    }

    fn settled(state: &mut ProseState, toc: BlockId) -> TableOfContents {
        state
            .settle_toc()
            .into_iter()
            .find_map(|event| match event {
                ProseEvent::TocUpdated {
                    block_id,
                    toc: built,
                } if block_id == toc => Some(built),
                _ => None,
            })
            .expect("the settled layout lists the block")
    }

    fn texts(toc: &TableOfContents) -> Vec<&str> {
        toc.entries
            .iter()
            .map(|entry| entry.text.as_str())
            .collect()
    }

    /// The metadata a block carries, as a plain value.
    fn meta_of(state: &ProseState, id: BlockId) -> LoroValue {
        let tree = doc::blocks(state.document());
        tree.get_meta(tree_id(id))
            .map(|meta| meta.get_deep_value())
            .unwrap_or(LoroValue::Null)
    }

    // -- the table of contents ---------------------------------------------

    #[test]
    fn the_entries_are_the_headings_in_order_reading_their_own_text() {
        let mut state = fresh();
        let toc = insert_toc(&mut state, 6, false);
        heading(&mut state, 1, "Alpha");
        append(&mut state, BlockInit::Paragraph, "Body text.");
        heading(&mut state, 2, "Beta");
        let built = settled(&mut state, toc);
        assert_eq!(texts(&built), ["Alpha", "Beta"]);
        let levels: Vec<u8> = built.entries.iter().map(|entry| entry.level).collect();
        assert_eq!(levels, [1, 2]);
    }

    #[test]
    fn an_entry_reads_the_page_its_heading_starts_on() {
        let mut state = fresh();
        let toc = insert_toc(&mut state, 6, false);
        heading(&mut state, 1, "First");
        append(&mut state, BlockInit::PageBreak, "");
        heading(&mut state, 1, "Second");
        let built = settled(&mut state, toc);
        let pages: Vec<u32> = built.entries.iter().map(|entry| entry.page).collect();
        assert_eq!(pages, [1, 2]);
    }

    #[test]
    fn the_block_holds_its_two_settings_and_no_entry() {
        let mut state = fresh();
        let toc = insert_toc(&mut state, 3, true);
        heading(&mut state, 1, "A heading");
        let _ = settled(&mut state, toc);
        let tree = doc::blocks(state.document());
        let attrs = doc::attrs_read(&tree, &tree_id(toc)).expect("the settings");
        let mut keys: Vec<String> = attrs.keys().map(|key| key.to_string()).collect();
        keys.sort();
        assert_eq!(keys, [doc::A_MAX_LEVEL, doc::A_NUMBERED]);
        assert!(doc::children_of(&tree, Some(tree_id(toc))).is_empty());
        assert_eq!(state.text_of(&toc), None);
    }

    #[test]
    fn a_heading_deeper_than_the_depth_is_left_out() {
        let mut state = fresh();
        let toc = insert_toc(&mut state, 2, false);
        heading(&mut state, 1, "One");
        heading(&mut state, 3, "Three");
        heading(&mut state, 2, "Two");
        assert_eq!(texts(&settled(&mut state, toc)), ["One", "Two"]);
    }

    #[test]
    fn a_section_number_shows_only_while_numbered() {
        let mut state = fresh();
        let toc = insert_toc(&mut state, 6, false);
        heading(&mut state, 1, "One");
        heading(&mut state, 2, "One point one");
        let plain = settled(&mut state, toc);
        assert!(
            plain
                .entries
                .iter()
                .all(|entry| entry.section_number.is_none())
        );
        state.handle_command(ProseCommand::UpdateTocConfig {
            block_id: toc,
            max_level: 6,
            numbered: true,
        });
        let numbered = settled(&mut state, toc);
        let numbers: Vec<Option<Vec<u32>>> = numbered
            .entries
            .iter()
            .map(|entry| entry.section_number.clone())
            .collect();
        assert_eq!(numbers, [Some(vec![1]), Some(vec![1, 1])]);
    }

    #[test]
    fn two_converged_peers_list_the_same_entries() {
        let mut alice = fresh();
        let toc = insert_toc(&mut alice, 6, false);
        heading(&mut alice, 1, "One");
        heading(&mut alice, 2, "Two");
        heading(&mut alice, 3, "Three");
        let seed = alice.export_snapshot();
        let mut bob = ProseState::with_peer(2).expect("a document is built");
        bob.load_faces(&crate::layout::fonts::shipped_faces());
        bob.import_snapshot(&seed).expect("the seed imports");
        let from_bob = bob
            .handle_command(ProseCommand::UpdateTocConfig {
                block_id: toc,
                max_level: 2,
                numbered: true,
            })
            .update
            .expect("a delta");
        alice.apply_remote(&from_bob);
        let (seen_by_alice, seen_by_bob) = (settled(&mut alice, toc), settled(&mut bob, toc));
        assert_eq!(seen_by_alice, seen_by_bob);
        assert_eq!(texts(&seen_by_alice), ["One", "Two"]);
    }

    #[test]
    fn an_insert_lands_where_it_was_named_carrying_its_settings() {
        let mut state = fresh();
        let first = state.root_blocks()[0];
        let toc = insert_toc(&mut state, 3, true);
        assert_eq!(state.root_blocks().get(1), Some(&toc));
        assert_eq!(state.root_blocks().first(), Some(&first));
        let tree = doc::blocks(state.document());
        assert_eq!(doc::toc_settings(&tree, &tree_id(toc)), Some((3, true)));
    }

    #[test]
    fn inserting_one_touches_no_heading() {
        let mut state = fresh();
        let kept = heading(&mut state, 2, "Kept as it was");
        let before = meta_of(&state, kept);
        let toc = insert_toc(&mut state, 6, true);
        let _ = settled(&mut state, toc);
        assert_eq!(meta_of(&state, kept), before);
        assert_eq!(state.text_of(&kept).as_deref(), Some("Kept as it was"));
    }

    #[test]
    fn a_settings_change_lands_on_the_block() {
        let mut state = fresh();
        let toc = insert_toc(&mut state, 6, false);
        state.handle_command(ProseCommand::UpdateTocConfig {
            block_id: toc,
            max_level: 4,
            numbered: true,
        });
        let tree = doc::blocks(state.document());
        assert_eq!(doc::toc_settings(&tree, &tree_id(toc)), Some((4, true)));
    }

    #[test]
    fn the_next_layout_lists_by_the_settings_just_written() {
        let mut state = fresh();
        let toc = insert_toc(&mut state, 6, false);
        heading(&mut state, 1, "One");
        heading(&mut state, 2, "Two");
        let outcome = state.handle_command(ProseCommand::UpdateTocConfig {
            block_id: toc,
            max_level: 1,
            numbered: true,
        });
        let listed = outcome.display.into_iter().find_map(|event| match event {
            ProseEvent::TocUpdated { toc: built, .. } => Some(built),
            _ => None,
        });
        let listed = listed.expect("the command answers the entries");
        assert_eq!(texts(&listed), ["One"]);
        assert_eq!((listed.max_level, listed.numbered), (1, true));
    }

    #[test]
    fn changing_one_block_leaves_another_and_the_headings_alone() {
        let mut state = fresh();
        let kept = heading(&mut state, 1, "A heading");
        let first = insert_toc(&mut state, 6, false);
        let second = append(&mut state, BlockInit::Paragraph, "");
        state.handle_command(ProseCommand::InsertToc {
            parent_id: None,
            after_id: Some(second),
            max_level: 2,
            numbered: true,
        });
        let other = *state.root_blocks().get(4).expect("the second block");
        let (heading_before, other_before) = (meta_of(&state, kept), meta_of(&state, other));
        state.handle_command(ProseCommand::UpdateTocConfig {
            block_id: first,
            max_level: 3,
            numbered: true,
        });
        assert_eq!(meta_of(&state, kept), heading_before);
        assert_eq!(meta_of(&state, other), other_before);
    }

    #[test]
    fn a_depth_outside_one_to_six_is_brought_inside() {
        let mut state = fresh();
        let toc = insert_toc(&mut state, 0, false);
        let tree = doc::blocks(state.document());
        assert_eq!(doc::toc_settings(&tree, &tree_id(toc)), Some((1, false)));
        state.handle_command(ProseCommand::UpdateTocConfig {
            block_id: toc,
            max_level: 200,
            numbered: false,
        });
        let tree = doc::blocks(state.document());
        assert_eq!(doc::toc_settings(&tree, &tree_id(toc)), Some((6, false)));
    }

    // -- the word count ----------------------------------------------------

    /// Feed every display event of a command to the view.
    fn show(view: &mut ProseUiState, events: &[ProseEvent]) {
        for event in events {
            view.apply_event(event);
        }
    }

    #[test]
    fn the_tallies_shown_are_those_of_the_text_once_settled() {
        let mut state = fresh();
        let mut view = ProseUiState::default();
        let block = state.root_blocks()[0];
        for (at, word) in ["one ", "two ", "three"].iter().enumerate() {
            let outcome = state.handle_command(ProseCommand::InsertText {
                block_id: block,
                pos: u32::try_from(at * 4).unwrap_or(0),
                text: (*word).to_owned(),
            });
            show(&mut view, &outcome.display);
            assert_eq!(view.tick(0.0), None);
        }
        let command = view.tick(RECOUNT_DELAY_MS).expect("the recount is due");
        let outcome = state.handle_command(command);
        show(&mut view, &outcome.display);
        let text = compute_word_count(state.document());
        assert_eq!(view.word_count.words, text.words);
        assert_eq!(view.word_count.words, 3);
        assert_eq!(view.word_count.characters, text.characters);
        assert_eq!(
            view.word_count.characters_with_spaces,
            text.characters_with_spaces
        );
        assert_eq!(view.word_count.paragraphs, text.paragraphs);
    }

    #[test]
    fn the_page_count_is_the_one_of_the_layout_taken() {
        let mut state = fresh();
        append(&mut state, BlockInit::Paragraph, "one");
        append(&mut state, BlockInit::PageBreak, "");
        append(&mut state, BlockInit::Paragraph, "two");
        let pages = state.paginate(&[]).page_count;
        assert!(pages >= 2);
        assert_eq!(state.word_count().pages, pages);
        let outcome = state.handle_command(ProseCommand::CountWords);
        let reported = outcome.display.iter().find_map(|event| match event {
            ProseEvent::WordCountUpdated { count } => Some(count.pages),
            _ => None,
        });
        assert_eq!(reported, Some(pages));
    }

    #[test]
    fn a_recount_writes_nothing_and_keeps_the_page_count() {
        let mut state = fresh();
        let block = state.root_blocks()[0];
        state.handle_command(ProseCommand::InsertText {
            block_id: block,
            pos: 0,
            text: "words".to_owned(),
        });
        let pages = state.paginate(&[]).page_count;
        // An edit that adds a page, and no layout taken since.
        append(&mut state, BlockInit::PageBreak, "");
        append(&mut state, BlockInit::Paragraph, "more");
        let before = state.document().oplog_vv();
        let outcome = state.handle_command(ProseCommand::CountWords);
        assert!(outcome.update.is_none());
        assert_eq!(state.document().oplog_vv(), before);
        assert_eq!(state.word_count().pages, pages);
    }

    #[test]
    fn a_new_layout_leaves_the_four_tallies_alone() {
        let mut state = fresh();
        let block = state.root_blocks()[0];
        state.handle_command(ProseCommand::InsertText {
            block_id: block,
            pos: 0,
            text: "two words".to_owned(),
        });
        state.handle_command(ProseCommand::CountWords);
        let counted = state.word_count();
        append(&mut state, BlockInit::Paragraph, "and three more");
        let _ = state.paginate(&[]);
        let after = state.word_count();
        assert_eq!(
            (after.words, after.characters, after.paragraphs),
            (counted.words, counted.characters, counted.paragraphs)
        );
        assert_eq!(after.characters_with_spaces, counted.characters_with_spaces);
    }

    #[test]
    fn the_document_holds_none_of_the_five_figures() {
        let mut state = fresh();
        let block = state.root_blocks()[0];
        state.handle_command(ProseCommand::InsertText {
            block_id: block,
            pos: 0,
            text: "Some words to count.".to_owned(),
        });
        let before = state.document().get_deep_value();
        let _ = state.paginate(&[]);
        state.handle_command(ProseCommand::CountWords);
        assert_eq!(state.document().get_deep_value(), before);
    }
}
