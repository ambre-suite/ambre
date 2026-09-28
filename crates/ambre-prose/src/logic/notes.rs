//! The three footnote commands (`features/ambre-spec-prose-footnotes.md`).
//!
//! A note is an entry of `annots` naming the block that holds its text. A
//! call is one carrier character marked with the entry's identity. The identity is minted once, by
//! the peer that inserts the note (`PROSE-FEAT-096`, `PROSE-087`).

use ambre_core::Uuid;
use loro::{LoroMap, LoroText, TreeID, UpdateOptions};

use crate::doc;
use crate::events::{BlockType, ProseEvent, ProseOutcome, ProseRefusal};
use crate::note;

use super::ProseState;

impl ProseState {
    /// A note holding `text`, and one call to it at `pos` in `block`
    /// (`PROSE-FEAT-099`).
    ///
    /// The note's text goes to its entry and never into the block
    /// (`PROSE-FEAT-100`). A header or a footer is refused, since a band drawn
    /// on every page cannot own one page's note (`PROSE-FEAT-071`).
    pub(super) fn insert_footnote(&mut self, block: &TreeID, pos: u32, text: &str) -> ProseOutcome {
        let tree = doc::blocks(&self.doc);
        if doc::is_band(&tree, block) {
            return ProseOutcome::refused(ProseRefusal::FootnoteInBand);
        }
        if doc::is_note(&tree, block) {
            return ProseOutcome::refused(ProseRefusal::FootnoteInNote);
        }
        let Some(body) = self.body_of(block) else {
            return Self::err_no_block(block);
        };
        let at = pos as usize;
        if at > body.len_unicode() {
            return Self::err_bad_range();
        }
        let id = self.ids.mint();
        if let Err(e) = self.write_note(&id, text) {
            return Self::err_write(&e);
        }
        if let Err(e) = Self::write_carrier(&body, at, doc::K_FOOTNOTE, &id.to_hyphenated()) {
            return Self::err_write(&e);
        }
        self.commit_notes()
    }

    /// Rewrite the body of note `id`, and nothing else.
    pub(super) fn edit_footnote(&mut self, id: &Uuid, text: &str) -> ProseOutcome {
        let Some(body) = note::body(&self.doc, id) else {
            return ProseOutcome::refused(ProseRefusal::NoSuchFootnote(*id));
        };
        if let Err(e) = body.update(text, UpdateOptions::default()) {
            return Self::err_write(&format!("{e:?}"));
        }
        self.commit_notes()
    }

    /// Remove note `id` and every call still carrying it (`PROSE-FEAT-070`).
    ///
    /// Only the call characters leave the text they sat in
    /// (`PROSE-FEAT-101`). The calls are removed from the last one back, so an
    /// earlier offset still names its character.
    pub(super) fn delete_footnote(&mut self, id: &Uuid) -> ProseOutcome {
        if note::entry(&self.doc, id).is_none() {
            return ProseOutcome::refused(ProseRefusal::NoSuchFootnote(*id));
        }
        let key = id.to_hyphenated();
        let calls: Vec<doc::Anchor> = doc::anchors(&self.doc, doc::K_FOOTNOTE)
            .into_iter()
            .filter(|anchor| anchor.value == key)
            .collect();
        for call in calls.iter().rev() {
            let body = doc::get_block_text(&self.doc, &call.block);
            if let Err(e) = body.delete(call.offset as usize, 1) {
                return Self::err_write(&format!("{e:?}"));
            }
        }
        if let Some(block) = note::block(&self.doc, id)
            && let Err(e) = doc::blocks(&self.doc).delete(block)
        {
            return Self::err_write(&format!("{e:?}"));
        }
        if let Err(e) = self.doc.get_map(doc::ANNOTS).delete(&key) {
            return Self::err_write(&format!("{e:?}"));
        }
        self.commit_notes()
    }

    /// Every note, numbered or orphaned, as the view lists it.
    #[must_use]
    pub fn footnotes(&self) -> Vec<crate::events::FootnoteEntry> {
        note::entries(&self.doc)
    }

    /// The entry of a new note, and the block holding its text. That block is a
    /// root paragraph marked as a note, so the body flow never meets it
    /// (`PROSE-FEAT-305`).
    fn write_note(&self, id: &Uuid, text: &str) -> Result<(), String> {
        let block = write_note_block(&self.doc, text)?;
        let annots = self.doc.get_map(doc::ANNOTS);
        let entry = annots
            .insert_container(&id.to_hyphenated(), LoroMap::new())
            .map_err(|e| format!("{e:?}"))?;
        entry
            .insert(doc::ANNOT_KIND, crate::note::KIND_FOOTNOTE)
            .map_err(|e| format!("{e:?}"))?;
        entry
            .insert(doc::ANNOT_NOTE_BLOCK, block.to_string())
            .map(|_| ())
            .map_err(|e| format!("{e:?}"))
    }

    /// One carrier character at `at`, marked `key` = `value`. The mark never
    /// expands, so typing beside it stays plain text.
    pub(super) fn write_carrier(
        body: &LoroText,
        at: usize,
        key: &str,
        value: &str,
    ) -> Result<(), String> {
        let mut buffer = [0_u8; 4];
        body.insert(at, doc::CARRIER.encode_utf8(&mut buffer))
            .map_err(|e| format!("{e:?}"))?;
        body.mark(at..at + 1, key, value)
            .map_err(|e| format!("{e:?}"))
    }

    /// Commit a footnote command and answer the list it leaves.
    fn commit_notes(&mut self) -> ProseOutcome {
        let notes = note::entries(&self.doc);
        self.commit_with(vec![ProseEvent::FootnotesUpdated { notes }])
    }
}

/// A root paragraph marked as a note, holding `text`. The schema migration
/// writes a note's block through here too, so both mint it alike.
pub(crate) fn write_note_block(document: &loro::LoroDoc, text: &str) -> Result<TreeID, String> {
    let block = doc::create_block(document, None, BlockType::Paragraph)
        .ok_or_else(|| "the tree refused a note block".to_owned())?;
    let tree = doc::blocks(document);
    doc::node_of(&tree, &block)
        .ok_or_else(|| "the note block is not in the tree".to_owned())?
        .insert(doc::K_NOTE, true)
        .map_err(|e| format!("{e:?}"))?;
    doc::get_block_text(document, &block)
        .insert(0, text)
        .map_err(|e| format!("{e:?}"))?;
    Ok(block)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::{BlockId, BlockInit, FootnoteEntry, ProseCommand, block_id};
    use crate::logic::ProseOutcome;

    fn refusal(outcome: &ProseOutcome) -> Option<ProseRefusal> {
        outcome.display.iter().find_map(|event| match event {
            ProseEvent::Error(error) => Some(error.reason.clone()),
            _ => None,
        })
    }

    fn listed(outcome: &ProseOutcome) -> Vec<FootnoteEntry> {
        outcome
            .display
            .iter()
            .find_map(|event| match event {
                ProseEvent::FootnotesUpdated { notes } => Some(notes.clone()),
                _ => None,
            })
            .unwrap_or_default()
    }

    /// A document of paragraphs holding `texts`.
    fn document(peer: u64, texts: &[&str]) -> (ProseState, Vec<BlockId>) {
        let mut state = ProseState::with_peer(peer).expect("a document");
        let mut ids = state.root_blocks();
        for _ in 1..texts.len() {
            state.handle_command(ProseCommand::InsertBlock {
                parent_id: None,
                after_id: ids.last().copied(),
                init: BlockInit::Paragraph,
                style_id: None,
            });
            ids = state.root_blocks();
        }
        for (id, text) in ids.iter().zip(texts) {
            state.handle_command(ProseCommand::InsertText {
                block_id: *id,
                pos: 0,
                text: (*text).to_owned(),
            });
        }
        (state, ids)
    }

    fn add(state: &mut ProseState, block: BlockId, pos: u32, text: &str) -> Uuid {
        let before: Vec<Uuid> = state.footnotes().iter().map(|n| n.footnote_id).collect();
        state.handle_command(ProseCommand::InsertFootnote {
            block_id: block,
            pos,
            note_text: text.to_owned(),
        });
        state
            .footnotes()
            .into_iter()
            .map(|n| n.footnote_id)
            .find(|id| !before.contains(id))
            .expect("a new note")
    }

    fn numbers(state: &ProseState) -> Vec<(String, Option<u32>)> {
        state
            .footnotes()
            .into_iter()
            .map(|n| (n.text, n.ordinal))
            .collect()
    }

    #[test]
    fn a_note_lands_as_one_call_and_an_entry_holding_its_text() {
        let (mut state, ids) = document(1, &["Hello world"]);
        let id = add(&mut state, ids[0], 5, "A note.");
        assert_eq!(
            state.text_of(&ids[0]).as_deref(),
            Some("Hello\u{fffc} world"),
            "the block holds the call and none of the note's text"
        );
        let calls = doc::anchors(state.document(), doc::K_FOOTNOTE);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].offset, 5);
        assert_eq!(calls[0].value, id.to_hyphenated());
        assert_eq!(
            note::body(state.document(), &id).map(|t| t.to_string()),
            Some("A note.".to_owned())
        );
    }

    #[test]
    fn a_note_touches_no_other_note() {
        let (mut state, ids) = document(1, &["abc"]);
        let first = add(&mut state, ids[0], 3, "one");
        let before = note::body(state.document(), &first).map(|t| t.get_richtext_value());
        add(&mut state, ids[0], 0, "two");
        let after = note::body(state.document(), &first).map(|t| t.get_richtext_value());
        assert_eq!(before, after);
    }

    #[test]
    fn a_note_is_numbered_from_the_order_of_its_calls_and_stores_no_number() {
        let (mut state, ids) = document(1, &["first", "second"]);
        add(&mut state, ids[1], 6, "later");
        add(&mut state, ids[0], 5, "earlier");
        assert_eq!(
            numbers(&state),
            vec![
                ("earlier".to_owned(), Some(1)),
                ("later".to_owned(), Some(2))
            ]
        );
        let stored = format!(
            "{:?}",
            state.document().get_map(doc::ANNOTS).get_deep_value()
        );
        assert!(!stored.contains("ordinal") && !stored.contains("number"));
        state.handle_command(ProseCommand::MoveBlock {
            block_id: ids[0],
            new_parent: None,
            after_id: Some(ids[1]),
        });
        assert_eq!(
            numbers(&state),
            vec![
                ("later".to_owned(), Some(1)),
                ("earlier".to_owned(), Some(2))
            ],
            "moving a paragraph past another note swaps the numbers"
        );
    }

    #[test]
    fn a_note_keeps_its_identity_through_a_move_an_edit_and_a_remote_change() {
        let (mut alice, ids) = document(1, &["one", "two"]);
        let id = add(&mut alice, ids[0], 3, "note");
        let mut bob = ProseState::with_peer(2).expect("bob");
        bob.import_snapshot(&alice.export_snapshot())
            .expect("bob opens");
        alice.handle_command(ProseCommand::MoveBlock {
            block_id: ids[0],
            new_parent: None,
            after_id: Some(ids[1]),
        });
        alice.handle_command(ProseCommand::EditFootnote {
            footnote_id: id,
            new_text: "edited".to_owned(),
        });
        let from_bob = bob.handle_command(ProseCommand::InsertText {
            block_id: ids[1],
            pos: 0,
            text: "x".to_owned(),
        });
        if let Some(delta) = &from_bob.update {
            alice.apply_remote(delta);
        }
        assert_eq!(
            alice.footnotes(),
            vec![FootnoteEntry {
                footnote_id: id,
                ordinal: Some(1),
                text: "edited".to_owned(),
            }]
        );
    }

    #[test]
    fn two_converged_peers_count_the_same_numbers() {
        let (mut alice, ids) = document(1, &["para"]);
        let mut bob = ProseState::with_peer(2).expect("bob");
        bob.import_snapshot(&alice.export_snapshot())
            .expect("bob opens");
        let from_alice = alice.handle_command(ProseCommand::InsertFootnote {
            block_id: ids[0],
            pos: 4,
            note_text: "alice".to_owned(),
        });
        let from_bob = bob.handle_command(ProseCommand::InsertFootnote {
            block_id: ids[0],
            pos: 4,
            note_text: "bob".to_owned(),
        });
        alice.apply_remote(&from_bob.update.expect("bob's note travels"));
        bob.apply_remote(&from_alice.update.expect("alice's note travels"));
        assert_eq!(alice.footnotes(), bob.footnotes());
        let ordinals: Vec<_> = alice.footnotes().iter().map(|n| n.ordinal).collect();
        assert_eq!(ordinals, vec![Some(1), Some(2)]);
    }

    #[test]
    fn deleting_the_text_of_a_call_keeps_the_note_and_lists_it_as_orphaned() {
        let (mut state, ids) = document(1, &["abc"]);
        let id = add(&mut state, ids[0], 1, "kept");
        state.handle_command(ProseCommand::DeleteText {
            block_id: ids[0],
            range: (0, 3),
        });
        assert_eq!(state.text_of(&ids[0]).as_deref(), Some("c"));
        assert_eq!(
            state.footnotes(),
            vec![FootnoteEntry {
                footnote_id: id,
                ordinal: None,
                text: "kept".to_owned(),
            }]
        );
    }

    #[test]
    fn deleting_a_note_removes_its_body_and_its_calls_and_nothing_else() {
        let (mut state, ids) = document(1, &["abcdef"]);
        let gone = add(&mut state, ids[0], 2, "gone");
        let kept = add(&mut state, ids[0], 5, "kept");
        let kept_body = note::body(state.document(), &kept).map(|t| t.to_string());
        let outcome = state.handle_command(ProseCommand::DeleteFootnote { footnote_id: gone });
        assert!(note::entry(state.document(), &gone).is_none());
        assert_eq!(state.text_of(&ids[0]).as_deref(), Some("abcd\u{fffc}ef"));
        assert_eq!(
            listed(&outcome),
            vec![FootnoteEntry {
                footnote_id: kept,
                ordinal: Some(1),
                text: "kept".to_owned(),
            }]
        );
        assert_eq!(
            note::body(state.document(), &kept).map(|t| t.to_string()),
            kept_body
        );
    }

    #[test]
    fn a_note_deleted_while_a_peer_edits_it_leaves_no_call() {
        let (mut alice, ids) = document(1, &["text"]);
        let id = add(&mut alice, ids[0], 4, "body");
        let mut bob = ProseState::with_peer(2).expect("bob");
        bob.import_snapshot(&alice.export_snapshot())
            .expect("bob opens");
        let deleted = alice.handle_command(ProseCommand::DeleteFootnote { footnote_id: id });
        let edited = bob.handle_command(ProseCommand::EditFootnote {
            footnote_id: id,
            new_text: "changed".to_owned(),
        });
        alice.apply_remote(&edited.update.expect("the edit travels"));
        bob.apply_remote(&deleted.update.expect("the deletion travels"));
        for peer in [&alice, &bob] {
            assert!(peer.footnotes().is_empty());
            assert_eq!(peer.text_of(&ids[0]).as_deref(), Some("text"));
        }
    }

    #[test]
    fn an_entry_is_a_note_by_its_declared_kind_and_by_nothing_else() {
        let (mut state, ids) = document(1, &["text"]);
        let real = add(&mut state, ids[0], 4, "a note");
        let annots = state.document().get_map(doc::ANNOTS);
        let shaped_alike = Uuid::new();
        let entry = annots
            .insert_container(&shaped_alike.to_hyphenated(), LoroMap::new())
            .expect("an entry");
        entry
            .insert_container(doc::ANNOT_TEXT, LoroText::new())
            .expect("a text")
            .insert(0, "no kind")
            .expect("written");
        let other = Uuid::new();
        let entry = annots
            .insert_container(&other.to_hyphenated(), LoroMap::new())
            .expect("an entry");
        entry.insert(doc::ANNOT_KIND, "comment").expect("a kind");
        entry
            .insert_container(doc::ANNOT_TEXT, LoroText::new())
            .expect("a text");
        state.document().commit();
        assert_eq!(note::all(state.document()), vec![real]);
    }

    #[test]
    fn a_note_is_refused_in_a_band_and_the_refusal_is_reported() {
        let (mut state, _) = document(1, &["body"]);
        state.handle_command(ProseCommand::SetFooterEnabled { enabled: true });
        let footer = doc::read_bands(state.document())
            .footer
            .block
            .expect("the footer block");
        let before = state.document().oplog_vv();
        let outcome = state.handle_command(ProseCommand::InsertFootnote {
            block_id: block_id(footer),
            pos: 0,
            note_text: "no".to_owned(),
        });
        assert_eq!(refusal(&outcome), Some(ProseRefusal::FootnoteInBand));
        assert_eq!(state.document().oplog_vv(), before);
        assert_eq!(state.text_of(&block_id(footer)).as_deref(), Some(""));
    }

    /// `PROSE-FEAT-305`: a note's text lives in a block the body flow never
    /// meets. Deleting the note takes the block with it.
    #[test]
    fn a_note_holds_its_text_in_a_block_out_of_the_flow() {
        let (mut state, ids) = document(1, &["text"]);
        let id = add(&mut state, ids[0], 4, "a note");
        let block = note::block(state.document(), &id).expect("the note's block");
        assert_eq!(state.text_of(&block_id(block)).as_deref(), Some("a note"));
        assert_eq!(state.root_blocks(), ids, "the flow holds no note");
        // Typing in the note is typing in any block.
        state.handle_command(ProseCommand::InsertText {
            block_id: block_id(block),
            pos: 6,
            text: " more".to_owned(),
        });
        assert_eq!(
            note::body(state.document(), &id).map(|t| t.to_string()),
            Some("a note more".to_owned())
        );
        state.handle_command(ProseCommand::DeleteFootnote { footnote_id: id });
        let tree = doc::blocks(state.document());
        assert!(
            !doc::children_of(&tree, None).contains(&block),
            "the block went with it"
        );
    }

    /// `PROSE-FEAT-306`, `PROSE-FEAT-307`: a note's text holds no note and no
    /// comment, and does not split.
    #[test]
    fn a_note_refuses_a_note_a_comment_and_a_split() {
        let (mut state, ids) = document(1, &["text"]);
        let id = add(&mut state, ids[0], 4, "a note");
        let block = block_id(note::block(state.document(), &id).expect("the note's block"));
        let outcome = state.handle_command(ProseCommand::InsertFootnote {
            block_id: block,
            pos: 1,
            note_text: "inner".to_owned(),
        });
        assert_eq!(refusal(&outcome), Some(ProseRefusal::FootnoteInNote));
        let outcome = state.handle_command(ProseCommand::AddComment {
            block_id: block,
            range: (0, 2),
            text: "a thread".to_owned(),
            now_ms: 0,
        });
        assert_eq!(refusal(&outcome), Some(ProseRefusal::CommentInNote));
        let outcome = state.handle_command(ProseCommand::SplitBlock {
            block_id: block,
            pos: 2,
        });
        assert!(refusal(&outcome).is_some(), "a note does not split");
        assert_eq!(state.root_blocks(), ids);
    }

    /// `PROSE-FEAT-308`: a version 8 document keeps its notes. The text moves
    /// from the entry to a block of its own, its marks with it.
    #[test]
    fn a_version_8_note_moves_to_a_block_with_its_marks() {
        let (mut state, ids) = document(1, &["text"]);
        let id = add(&mut state, ids[0], 4, "old note");
        // Write the note back the way version 8 held it.
        let document = state.document();
        let block = note::block(document, &id).expect("the note's block");
        let entry = note::entry(document, &id).expect("the entry");
        let text = entry
            .insert_container(doc::ANNOT_TEXT, LoroText::new())
            .expect("a text");
        text.insert(0, "old note").expect("written");
        text.mark(0..3, "b", "true").expect("marked");
        entry.delete(doc::ANNOT_NOTE_BLOCK).expect("unnamed");
        doc::blocks(document).delete(block).expect("gone");
        document
            .get_map(ambre_core::schema::SCHEMA_META_MAP)
            .insert(ambre_core::schema::SCHEMA_VERSION_KEY, 8_i64)
            .expect("stamped");
        document.commit();
        let bytes = state.export_snapshot();

        let mut read = ProseState::with_peer(2).expect("a document");
        read.import_snapshot(&bytes).expect("version 8 migrates");
        let body = note::body(read.document(), &id).expect("the note's text");
        assert_eq!(body.to_string(), "old note");
        let bold = body.to_delta().into_iter().any(|delta| match delta {
            loro::TextDelta::Insert { insert, attributes } => {
                insert == "old" && attributes.is_some_and(|a| a.contains_key("b"))
            }
            _ => false,
        });
        assert!(bold, "the mark came with the text");
        let entry = note::entry(read.document(), &id).expect("the entry");
        assert!(
            entry.get(doc::ANNOT_TEXT).is_none(),
            "the entry holds no text"
        );
        assert_eq!(read.root_blocks().len(), 1, "the flow holds no note");
    }
}
