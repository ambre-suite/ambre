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
use crate::events::{
    BlockType, HeadingLevel, ProseEvent, ProseOutcome, TableOfContents, TocNumbering, block_id,
};
use crate::toc;

use super::ProseState;

impl ProseState {
    /// A table of contents at a place in the tree, listing headings down to
    /// `max_level`, numbered or not (`PROSE-FEAT-115`).
    ///
    /// The block carries the two settings and nothing else: no entry is
    /// written, and no heading is touched (`PROSE-FEAT-116`). The depth is one
    /// of the six levels (`PROSE-FEAT-292`).
    pub(super) fn insert_toc(
        &mut self,
        parent_id: Option<TreeID>,
        after_id: Option<TreeID>,
        max_level: HeadingLevel,
        numbered: TocNumbering,
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
        max_level: HeadingLevel,
        numbered: TocNumbering,
    ) -> ProseOutcome {
        if doc::block_type_of(&doc::blocks(&self.doc), id) != Some(BlockType::Toc) {
            return Self::err_no_block(id);
        }
        self.write_toc_settings(id, max_level, numbered)
    }

    /// The shared write of the two commands, then the entries as the settled
    /// layout lists them (`PROSE-FEAT-291`).
    fn write_toc_settings(
        &mut self,
        id: &TreeID,
        max_level: HeadingLevel,
        numbered: TocNumbering,
    ) -> ProseOutcome {
        let tree = doc::blocks(&self.doc);
        let attrs = doc::node_of(&tree, id).and_then(|node| doc::attrs_at(&node));
        let Some(attrs) = attrs else {
            return Self::err_write(doc::WriteError::Refused(
                doc::RefusalCause::BlockAttrsRefused,
            ));
        };
        let level = i64::from(max_level.get());
        let written = attrs
            .insert(doc::AttrKey::MaxLevel, LoroValue::I64(level))
            .and_then(|()| {
                attrs.insert(
                    doc::AttrKey::Numbered,
                    LoroValue::Bool(numbered.is_numbered()),
                )
            });
        if let Err(e) = written {
            return Self::err_write(e);
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

    /// Recount the four text counts and answer the five figures
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
        display.push(ProseEvent::LinksUpdated {
            links: crate::link::links(&self.doc),
        });
        ProseOutcome::display_only(display)
    }

    /// Take the page count of the layout on hand into the figures, and leave
    /// the four counts alone (`PROSE-FEAT-126`, `PROSE-FEAT-127`).
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
    use crate::events::{HeadingLevel, TocNumbering};
    use ambre_bus::app::prose::DocChar;
    use ambre_core::index::Pos;
    use loro::LoroValue;

    use super::ProseState;
    use crate::count::compute_word_count;
    use crate::doc;
    use crate::events::{
        BlockId, BlockInit, BlockType, ProseCommand, ProseEvent, TableOfContents, tree_id,
    };
    use crate::ui::{ProseUiState, RECOUNT_DELAY};
    use ambre_core::{Instant, PageLoad};

    /// A document laid out on the shipped faces, holding nothing yet.
    fn fresh() -> ProseState {
        let mut state =
            ProseState::with_peer(ambre_core::PeerId::new(1)).expect("a document is built");
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
                pos: Pos::<DocChar>::ZERO,
                text: text.to_owned(),
            });
        }
        id
    }

    fn heading(state: &mut ProseState, level: u8, text: &str) -> BlockId {
        let level = HeadingLevel::new(level).expect("a level of one to six");
        append(state, BlockInit::Heading { level }, text)
    }

    /// A table of contents after the first block, answering its id and the
    /// events the insert produced.
    fn insert_toc(state: &mut ProseState, max_level: u8, numbered: TocNumbering) -> BlockId {
        let first = state.root_blocks()[0];
        state.handle_command(ProseCommand::InsertToc {
            parent_id: None,
            after_id: Some(first),
            max_level: HeadingLevel::new(max_level).expect("a depth of one to six"),
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
        let toc = insert_toc(&mut state, 6, TocNumbering::Plain);
        heading(&mut state, 1, "Alpha");
        append(&mut state, BlockInit::Paragraph, "Body text.");
        heading(&mut state, 2, "Beta");
        let built = settled(&mut state, toc);
        assert_eq!(texts(&built), ["Alpha", "Beta"]);
        let levels: Vec<HeadingLevel> = built.entries.iter().map(|entry| entry.level).collect();
        assert_eq!(levels, [HeadingLevel::H1, HeadingLevel::H2]);
    }

    #[test]
    fn an_entry_reads_the_page_its_heading_starts_on() {
        let mut state = fresh();
        let toc = insert_toc(&mut state, 6, TocNumbering::Plain);
        heading(&mut state, 1, "First");
        append(&mut state, BlockInit::PageBreak, "");
        heading(&mut state, 1, "Second");
        let built = settled(&mut state, toc);
        let pages: Vec<Option<u32>> = built
            .entries
            .iter()
            .map(|entry| entry.page.map(|page| page.get()))
            .collect();
        assert_eq!(pages, [Some(1), Some(2)]);
    }

    /// A heading the layout places on no page draws no page number. It once
    /// drew page zero, which no reader's page is.
    #[test]
    fn a_heading_no_layout_places_draws_no_page_number() {
        let mut state = fresh();
        let toc = insert_toc(&mut state, 6, TocNumbering::Plain);
        heading(&mut state, 1, "Lost");
        let tree = doc::blocks(state.document());
        let built = crate::toc::build(
            state.document(),
            &tree,
            &tree_id(toc),
            &std::collections::HashMap::new(),
        )
        .expect("a live table of contents");
        assert_eq!(crate::toc::lines(&built), "Lost");
    }

    #[test]
    fn the_block_holds_its_two_settings_and_no_entry() {
        let mut state = fresh();
        let toc = insert_toc(&mut state, 3, TocNumbering::Numbered);
        heading(&mut state, 1, "A heading");
        let _ = settled(&mut state, toc);
        let tree = doc::blocks(state.document());
        let attrs = doc::attrs_read(&tree, &tree_id(toc)).expect("the settings");
        let mut keys: Vec<String> = attrs.map().keys().map(|key| key.to_string()).collect();
        keys.sort();
        assert_eq!(
            keys,
            [
                doc::AttrKey::MaxLevel.as_str(),
                doc::AttrKey::Numbered.as_str()
            ]
        );
        assert!(doc::children_of(&tree, Some(tree_id(toc))).is_empty());
        assert_eq!(state.text_of(&toc), None);
    }

    #[test]
    fn a_heading_deeper_than_the_depth_is_left_out() {
        let mut state = fresh();
        let toc = insert_toc(&mut state, 2, TocNumbering::Plain);
        heading(&mut state, 1, "One");
        heading(&mut state, 3, "Three");
        heading(&mut state, 2, "Two");
        assert_eq!(texts(&settled(&mut state, toc)), ["One", "Two"]);
    }

    #[test]
    fn a_heading_number_shows_only_while_numbered() {
        let mut state = fresh();
        let toc = insert_toc(&mut state, 6, TocNumbering::Plain);
        heading(&mut state, 1, "One");
        heading(&mut state, 2, "One point one");
        let plain = settled(&mut state, toc);
        assert!(
            plain
                .entries
                .iter()
                .all(|entry| entry.heading_number.is_none())
        );
        state.handle_command(ProseCommand::UpdateTocConfig {
            block_id: toc,
            max_level: HeadingLevel::H6,
            numbered: TocNumbering::Numbered,
        });
        let numbered = settled(&mut state, toc);
        let numbers: Vec<Option<Vec<u32>>> = numbered
            .entries
            .iter()
            .map(|entry| entry.heading_number.as_ref().map(|n| n.parts().to_vec()))
            .collect();
        assert_eq!(numbers, [Some(vec![1]), Some(vec![1, 1])]);
    }

    #[test]
    fn two_converged_peers_list_the_same_entries() {
        let mut alice = fresh();
        let toc = insert_toc(&mut alice, 6, TocNumbering::Plain);
        heading(&mut alice, 1, "One");
        heading(&mut alice, 2, "Two");
        heading(&mut alice, 3, "Three");
        let seed = alice.export_snapshot();
        let mut bob =
            ProseState::with_peer(ambre_core::PeerId::new(2)).expect("a document is built");
        bob.load_faces(&crate::layout::fonts::shipped_faces());
        bob.import_snapshot(&seed).expect("the seed imports");
        let from_bob = bob
            .handle_command(ProseCommand::UpdateTocConfig {
                block_id: toc,
                max_level: HeadingLevel::H2,
                numbered: TocNumbering::Numbered,
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
        let toc = insert_toc(&mut state, 3, TocNumbering::Numbered);
        assert_eq!(state.root_blocks().get(1), Some(&toc));
        assert_eq!(state.root_blocks().first(), Some(&first));
        let tree = doc::blocks(state.document());
        assert_eq!(
            doc::toc_settings(&tree, &tree_id(toc)),
            Some(settings(HeadingLevel::H3, TocNumbering::Numbered))
        );
    }

    #[test]
    fn inserting_one_touches_no_heading() {
        let mut state = fresh();
        let kept = heading(&mut state, 2, "Kept as it was");
        let before = meta_of(&state, kept);
        let toc = insert_toc(&mut state, 6, TocNumbering::Numbered);
        let _ = settled(&mut state, toc);
        assert_eq!(meta_of(&state, kept), before);
        assert_eq!(state.text_of(&kept).as_deref(), Some("Kept as it was"));
    }

    #[test]
    fn a_settings_change_lands_on_the_block() {
        let mut state = fresh();
        let toc = insert_toc(&mut state, 6, TocNumbering::Plain);
        state.handle_command(ProseCommand::UpdateTocConfig {
            block_id: toc,
            max_level: HeadingLevel::H4,
            numbered: TocNumbering::Numbered,
        });
        let tree = doc::blocks(state.document());
        assert_eq!(
            doc::toc_settings(&tree, &tree_id(toc)),
            Some(settings(HeadingLevel::H4, TocNumbering::Numbered))
        );
    }

    #[test]
    fn the_next_layout_lists_by_the_settings_just_written() {
        let mut state = fresh();
        let toc = insert_toc(&mut state, 6, TocNumbering::Plain);
        heading(&mut state, 1, "One");
        heading(&mut state, 2, "Two");
        let outcome = state.handle_command(ProseCommand::UpdateTocConfig {
            block_id: toc,
            max_level: HeadingLevel::H1,
            numbered: TocNumbering::Numbered,
        });
        let listed = outcome.display.into_iter().find_map(|event| match event {
            ProseEvent::TocUpdated { toc: built, .. } => Some(built),
            _ => None,
        });
        let listed = listed.expect("the command answers the entries");
        assert_eq!(texts(&listed), ["One"]);
        assert_eq!(
            (listed.max_level, listed.numbered),
            (HeadingLevel::H1, TocNumbering::Numbered)
        );
    }

    #[test]
    fn changing_one_block_leaves_another_and_the_headings_alone() {
        let mut state = fresh();
        let kept = heading(&mut state, 1, "A heading");
        let first = insert_toc(&mut state, 6, TocNumbering::Plain);
        let second = append(&mut state, BlockInit::Paragraph, "");
        state.handle_command(ProseCommand::InsertToc {
            parent_id: None,
            after_id: Some(second),
            max_level: HeadingLevel::H2,
            numbered: TocNumbering::Numbered,
        });
        let other = *state.root_blocks().get(4).expect("the second block");
        let (heading_before, other_before) = (meta_of(&state, kept), meta_of(&state, other));
        state.handle_command(ProseCommand::UpdateTocConfig {
            block_id: first,
            max_level: HeadingLevel::H3,
            numbered: TocNumbering::Numbered,
        });
        assert_eq!(meta_of(&state, kept), heading_before);
        assert_eq!(meta_of(&state, other), other_before);
    }

    fn settings(max_level: HeadingLevel, numbered: TocNumbering) -> doc::TocSettings {
        doc::TocSettings {
            max_level,
            numbered,
        }
    }

    /// A command carries one of the six levels, so a depth outside them
    /// reaches a block only from a client that wrote one. It reads as the
    /// nearer end (`PROSE-FEAT-292`).
    #[test]
    fn a_stored_depth_outside_one_to_six_reads_as_the_nearer_end() {
        let mut state = fresh();
        let toc = insert_toc(&mut state, 3, TocNumbering::Plain);
        for (stored, read) in [(0, HeadingLevel::H1), (200, HeadingLevel::H6)] {
            let tree = doc::blocks(state.document());
            let node = doc::node_of(&tree, &tree_id(toc)).expect("a live block");
            doc::attrs_at(&node)
                .expect("the attribute map")
                .insert(doc::AttrKey::MaxLevel, LoroValue::I64(stored))
                .expect("the depth is stored");
            state.document().commit();
            let tree = doc::blocks(state.document());
            assert_eq!(
                doc::toc_settings(&tree, &tree_id(toc)),
                Some(settings(read, TocNumbering::Plain))
            );
        }
    }

    // -- the word count ----------------------------------------------------

    /// Feed every display event of a command to the view.
    fn show(view: &mut ProseUiState, events: &[ProseEvent]) {
        for event in events {
            view.apply_event(event);
        }
    }

    #[test]
    fn the_counts_shown_are_those_of_the_text_once_settled() {
        let mut state = fresh();
        let mut view = ProseUiState::default();
        let block = state.root_blocks()[0];
        for (at, word) in ["one ", "two ", "three"].iter().enumerate() {
            let outcome = state.handle_command(ProseCommand::InsertText {
                block_id: block,
                pos: Pos::<DocChar>::from_usize(at * 4),
                text: (*word).to_owned(),
            });
            show(&mut view, &outcome.display);
            assert_eq!(view.tick(Instant::<PageLoad>::new(0.0)), None);
        }
        let command = view
            .tick(Instant::<PageLoad>::new(RECOUNT_DELAY.get()))
            .expect("the recount is due");
        let outcome = state.handle_command(command);
        show(&mut view, &outcome.display);
        let text = compute_word_count(state.document());
        assert_eq!(view.word_count.words, text.words);
        assert_eq!(view.word_count.words.get(), 3);
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
        assert!(pages.get() >= 2);
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
            pos: Pos::<DocChar>::ZERO,
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
    fn a_new_layout_leaves_the_four_counts_alone() {
        let mut state = fresh();
        let block = state.root_blocks()[0];
        state.handle_command(ProseCommand::InsertText {
            block_id: block,
            pos: Pos::<DocChar>::ZERO,
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
            pos: Pos::<DocChar>::ZERO,
            text: "Some words to count.".to_owned(),
        });
        let before = state.document().get_deep_value();
        let _ = state.paginate(&[]);
        state.handle_command(ProseCommand::CountWords);
        assert_eq!(state.document().get_deep_value(), before);
    }
}
