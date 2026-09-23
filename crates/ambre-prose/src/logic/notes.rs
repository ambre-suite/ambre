//! The three footnote commands (`features/ambre-spec-prose-footnotes.md`).
//!
//! A note is an entry of `annots` holding its body, and a call is one carrier
//! character marked with the entry's identity. The identity is minted once, by
//! the peer that inserts the note (`PROSE-FEAT-096`, `PROSE-087`).

use ambre_core::Uuid;
use loro::{LoroMap, LoroText, TreeID, UpdateOptions};

use crate::doc;
use crate::events::{ProseEvent, ProseOutcome, ProseRefusal};
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
        if doc::is_band(&doc::blocks(&self.doc), block) {
            return ProseOutcome::refused(ProseRefusal::FootnoteInBand);
        }
        let Some(body) = self.body_of(block) else {
            return Self::err_no_block(block);
        };
        let at = pos as usize;
        if at > body.len_unicode() {
            return Self::err_bad_range();
        }
        let id = Uuid::new();
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

    /// The entry of a new note: its kind and its body.
    fn write_note(&self, id: &Uuid, text: &str) -> Result<(), String> {
        let annots = self.doc.get_map(doc::ANNOTS);
        let entry = annots
            .insert_container(&id.to_hyphenated(), LoroMap::new())
            .map_err(|e| format!("{e:?}"))?;
        entry
            .insert(doc::ANNOT_KIND, crate::note::KIND_FOOTNOTE)
            .map_err(|e| format!("{e:?}"))?;
        let body = entry
            .insert_container(doc::ANNOT_TEXT, LoroText::new())
            .map_err(|e| format!("{e:?}"))?;
        body.insert(0, text).map_err(|e| format!("{e:?}"))
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
}
