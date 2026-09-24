//! The write path of the inline mark registry
//! (`ambre-spec-prose-schema.md section 3`).
//!
//! A handler never states an expansion policy. The document was configured from
//! [`crate::mark::expand_type_for`] at construction, so the policy has one
//! author and one place (`PROSE-007`). A key the registry does not declare is
//! refused here rather than left to the CRDT engine, so the refusal names the
//! registry.

use loro::TreeID;

use loro::LoroValue;

use crate::doc;
use crate::events::{BlockId, BlockType, ProseEvent, ProseOutcome, ProseRefusal, tree_id};
use crate::mark;
use crate::style::{BlockContext, MarkSet, ResolvedStyle, resolve};

use super::ProseState;

impl ProseState {
    /// Mark a range under one registered key.
    ///
    /// The value reaches the document in its canonical spelling. Two peers writing
    /// the same size therefore produce the same bytes. The merge then holds one
    /// value rather than a choice between two spellings (`PROSE-008`,
    /// `PROSE-091`). A key whose presence alone is the value is stored as the
    /// empty string, which is the one spelling of "present".
    pub(super) fn apply_mark(
        &mut self,
        id: &TreeID,
        range: (u32, u32),
        key: &str,
        value: Option<&str>,
    ) -> ProseOutcome {
        let Some(body) = self.body_of(id) else {
            return Self::err_no_block(id);
        };
        if mark::is_owned_anchor(key) {
            return ProseOutcome::refused(ProseRefusal::AnchorKey(key.to_owned()));
        }
        let (start, end) = ordered(range);
        if end > body.len_unicode() || start == end {
            return Self::err_bad_range();
        }
        // The grammar of the key is checked before the document is touched, so
        // no unparseable value ever reaches the format (`PROSE-FEAT-022`).
        let stored = match mark::validate_value(key, value) {
            Ok(canonical) => canonical,
            Err(refusal) => return ProseOutcome::refused(refused_as(key, value, &refusal)),
        };
        if let Err(e) = body.mark(start..end, key, stored.as_str()) {
            return Self::err_write(&format!("{e:?}"));
        }
        self.commit_with(self.links_touched(Some(key)))
    }

    /// The link list, when the command touched links: a mark under the link
    /// key, or a clear, which takes links along. Nothing otherwise.
    fn links_touched(&self, key: Option<&str>) -> Vec<ProseEvent> {
        if key.is_some_and(|key| key != "a") {
            return Vec::new();
        }
        vec![ProseEvent::LinksUpdated {
            links: crate::link::links(&self.doc),
        }]
    }

    /// Remove one mark from a range.
    pub(super) fn remove_mark(
        &mut self,
        id: &TreeID,
        range: (u32, u32),
        key: &str,
    ) -> ProseOutcome {
        let Some(body) = self.body_of(id) else {
            return Self::err_no_block(id);
        };
        if !mark::is_registered(key) {
            return ProseOutcome::refused(ProseRefusal::UnknownMark(key.to_owned()));
        }
        if mark::is_owned_anchor(key) {
            return ProseOutcome::refused(ProseRefusal::AnchorKey(key.to_owned()));
        }
        let (start, end) = ordered(range);
        if end > body.len_unicode() || start == end {
            return Self::err_bad_range();
        }
        if let Err(e) = body.unmark(start..end, key) {
            return Self::err_write(&format!("{e:?}"));
        }
        self.commit_with(self.links_touched(Some(key)))
    }

    /// Remove every formatting mark from a range.
    ///
    /// The formatting keys alone (`ambre-spec-prose-schema.md section 3`). A
    /// comment, a suggestion, a footnote and a cross reference are anchors
    /// rather than formatting, and each has a feature that owns its removal.
    pub(super) fn clear_inline_marks(&mut self, id: &TreeID, range: (u32, u32)) -> ProseOutcome {
        let Some(body) = self.body_of(id) else {
            return Self::err_no_block(id);
        };
        let (start, end) = ordered(range);
        if end > body.len_unicode() || start == end {
            return Self::err_bad_range();
        }
        for key in mark::FORMATTING_KEYS {
            if let Err(e) = body.unmark(start..end, key) {
                tracing::warn!("Prose: the text refused to clear the mark {key}: {e:?}");
            }
        }
        self.commit_with(self.links_touched(None))
    }

    /// One block's text, run by run, each with the style the cascade resolves
    /// for it (`PROSE-FEAT-020`).
    ///
    /// The read the view draws from, and the one a test asserts on. It writes
    /// nothing: the resolution is a read of the marks, the block and the
    /// document defaults.
    #[must_use]
    pub fn resolved_runs(&self, id: &BlockId) -> Vec<(String, ResolvedStyle)> {
        let tree = doc::blocks(&self.doc);
        let context = BlockContext {
            block_type: doc::block_type_of(&tree, &tree_id(*id)).unwrap_or(BlockType::Paragraph),
            heading_level: doc::heading_level_of(&tree, &tree_id(*id)),
        };
        let defaults = doc::read_defaults(&self.doc);
        let Some(LoroValue::List(runs)) = self.marks_of(id) else {
            return Vec::new();
        };
        runs.iter()
            .filter_map(|run| {
                let LoroValue::Map(fields) = run else {
                    return None;
                };
                let Some(LoroValue::String(text)) = fields.get(RUN_TEXT) else {
                    return None;
                };
                Some((
                    text.to_string(),
                    resolve(&marks_of_run(run), context, &defaults),
                ))
            })
            .collect()
    }

    /// The marks a range carries, as the rich-text value Loro answers with.
    /// A read for a test and for the toolbar's own state.
    #[must_use]
    pub fn marks_of(&self, id: &BlockId) -> Option<loro::LoroValue> {
        self.body_of(&tree_id(*id))
            .map(|body| body.get_richtext_value())
    }
}

/// The field Loro nests a run's marks under. Its naming, not the schema's.
const RUN_MARKS: &str = "attributes";

/// The field Loro carries a run's characters under.
const RUN_TEXT: &str = "insert";

/// The refusal a value error is reported as.
///
/// The engine's refusal set is what the view reads, and the value error is the
/// reason behind one of its variants.
fn refused_as(key: &str, value: Option<&str>, error: &mark::MarkValueError) -> ProseRefusal {
    match error {
        mark::MarkValueError::UnknownKey(key) => ProseRefusal::UnknownMark(key.clone()),
        _ => ProseRefusal::BadMarkValue {
            key: key.to_owned(),
            value: value.unwrap_or_default().to_owned(),
        },
    }
}

/// One run's marks, as the cascade reads them.
///
/// A run Loro carries no mark field for carries no mark, which is an empty set
/// rather than a missing one.
fn marks_of_run(fields: &LoroValue) -> MarkSet {
    let LoroValue::Map(fields) = fields else {
        return MarkSet::new();
    };
    let Some(LoroValue::Map(marks)) = fields.get(RUN_MARKS) else {
        return MarkSet::new();
    };
    marks
        .iter()
        .map(|(key, value)| {
            let stored = match value {
                LoroValue::String(text) => text.to_string(),
                _ => String::new(),
            };
            (key.clone(), stored)
        })
        .collect()
}

/// The two ends of a range, smaller first.
fn ordered(range: (u32, u32)) -> (usize, usize) {
    let (a, b) = (range.0 as usize, range.1 as usize);
    if a <= b { (a, b) } else { (b, a) }
}

#[cfg(test)]
pub(crate) mod tests {
    use crate::events::{Align, ParagraphStyle, ProseCommand, ProseEvent};
    use crate::logic::ProseState;

    fn document_with_text(text: &str) -> (ProseState, crate::events::BlockId) {
        let mut state = ProseState::new().expect("a document is built");
        let block = state.root_blocks()[0];
        state.handle_command(ProseCommand::InsertText {
            block_id: block,
            pos: 0,
            text: text.to_owned(),
        });
        (state, block)
    }

    /// Whether the rich-text value carries `key` anywhere.
    fn carries_mark(state: &ProseState, block: &crate::events::BlockId, key: &str) -> bool {
        format!("{:?}", state.marks_of(block)).contains(key)
    }

    #[test]
    fn a_size_reaches_the_document_in_its_canonical_spelling() {
        let (mut state, block) = document_with_text("hello");
        state.handle_command(ProseCommand::ApplyMark {
            block_id: block,
            range: (0, 5),
            key: "fs".to_owned(),
            value: Some("11.0".to_owned()),
        });
        let rendered = format!("{:?}", state.marks_of(&block));
        assert!(rendered.contains("\"11\""), "{rendered}");
    }

    #[test]
    fn a_key_the_registry_does_not_declare_is_refused() {
        let (mut state, block) = document_with_text("hello");
        let outcome = state.handle_command(ProseCommand::ApplyMark {
            block_id: block,
            range: (0, 5),
            key: "no-such-key".to_owned(),
            value: None,
        });
        assert!(outcome.update.is_none());
    }

    #[test]
    fn clearing_the_formatting_leaves_an_anchor_in_place() {
        let (mut state, block) = document_with_text("hello");
        state.handle_command(ProseCommand::ApplyMark {
            block_id: block,
            range: (0, 5),
            key: "b".to_owned(),
            value: None,
        });
        anchor(&mut state, &block, 0..5);
        state.handle_command(ProseCommand::ClearInlineMarks {
            block_id: block,
            range: (0, 5),
        });
        assert!(carries_mark(&state, &block, "cm"));
        assert!(!carries_mark(&state, &block, "\"b\""));
    }

    #[test]
    fn an_empty_range_is_refused() {
        let (mut state, block) = document_with_text("hello");
        let outcome = state.handle_command(ProseCommand::ApplyMark {
            block_id: block,
            range: (2, 2),
            key: "b".to_owned(),
            value: None,
        });
        assert!(outcome.update.is_none());
    }

    /// Whether an outcome is the refusal rather than a change.
    fn refused(outcome: &crate::events::ProseOutcome) -> bool {
        matches!(outcome.display.as_slice(), [ProseEvent::Error(_)])
    }

    /// Anchors a comment over `range` the way its own command does: straight
    /// on the text, since a style command refuses the key.
    pub(crate) fn anchor(
        state: &mut ProseState,
        block: &crate::events::BlockId,
        range: std::ops::Range<usize>,
    ) {
        let text = crate::doc::get_block_text(&state.doc, &crate::events::tree_id(*block));
        text.mark(range, "cm", "a-comment-id")
            .expect("the anchor lands");
        state.doc.commit();
    }

    /// The reason an outcome was refused for, when it was.
    fn refusal(outcome: &crate::events::ProseOutcome) -> Option<crate::events::ProseRefusal> {
        outcome.display.iter().find_map(|event| match event {
            ProseEvent::Error(error) => Some(error.reason.clone()),
            _ => None,
        })
    }

    /// The runs of one block, as text and resolved style.
    fn runs(
        state: &ProseState,
        block: &crate::events::BlockId,
    ) -> Vec<(String, crate::style::ResolvedStyle)> {
        state.resolved_runs(block)
    }

    /// The mark covers the range asked for and no character outside it
    /// (`PROSE-FEAT-178`).
    #[test]
    fn a_mark_covers_the_range_and_no_character_outside_it() {
        let (mut state, block) = document_with_text("one two");
        state.handle_command(ProseCommand::ApplyMark {
            block_id: block,
            range: (0, 3),
            key: "b".to_owned(),
            value: None,
        });
        let bolded: String = runs(&state, &block)
            .into_iter()
            .filter(|(_, style)| style.bold)
            .map(|(text, _)| text)
            .collect();
        assert_eq!(bolded, "one");
    }

    /// The value stored is the canonical form of the value supplied
    /// (`PROSE-FEAT-290`, `PROSE-FEAT-175`).
    #[test]
    fn a_colour_reaches_the_document_in_one_spelling() {
        let (mut state, block) = document_with_text("hello");
        state.handle_command(ProseCommand::ApplyMark {
            block_id: block,
            range: (0, 5),
            key: "fg".to_owned(),
            // The opaque eight-digit spelling of the same colour.
            value: Some("#3366CCFF".to_owned()),
        });
        let rendered = format!("{:?}", state.marks_of(&block));
        assert!(rendered.contains("#3366cc\""), "{rendered}");
    }

    /// Reading a stored colour and writing it back leaves the bytes unchanged
    /// (`PROSE-FEAT-023`).
    #[test]
    fn a_stored_colour_written_back_leaves_the_bytes_unchanged() {
        for spelling in ["#3366cc", "#3366cc80", "#000000"] {
            let parsed = crate::mark::parse_color(spelling).expect("a colour");
            let canonical = crate::mark::canonical_color(parsed);
            assert_eq!(
                crate::mark::canonical_color(
                    crate::mark::parse_color(&canonical).expect("a colour")
                ),
                canonical,
                "{spelling}"
            );
        }
    }

    /// A value the key's grammar refuses is refused before the document is
    /// written, and reported (`PROSE-FEAT-022`).
    #[test]
    fn a_value_the_grammar_refuses_never_reaches_the_document() {
        let (mut state, block) = document_with_text("hello");
        let before = state.export_snapshot();
        for (key, value) in [
            ("fg", Some("not-a-colour")),
            ("fg", Some("#12345")),
            ("fs", Some("0.5")),
            ("fs", Some("2000")),
            ("fs", Some("eleven")),
            // A flag key takes no value, and a keyed one requires one.
            ("b", Some("true")),
            ("ff", None),
        ] {
            let outcome = state.handle_command(ProseCommand::ApplyMark {
                block_id: block,
                range: (0, 5),
                key: key.to_owned(),
                value: value.map(str::to_owned),
            });
            assert!(refused(&outcome), "{key} took {value:?}");
            assert!(outcome.update.is_none(), "{key} took {value:?}");
        }
        assert_eq!(state.export_snapshot(), before);
    }

    /// The style path refuses every anchor its own command owns, and takes
    /// the link, which it owns (`PROSE-FEAT-298`).
    #[test]
    fn a_style_command_never_writes_an_anchor_it_does_not_own() {
        let (mut state, block) = document_with_text("hello");
        let before = state.export_snapshot();
        let thread = format!("cm:{}", ambre_core::Uuid::new().to_hyphenated());
        for key in ["cm", thread.as_str(), "si", "sd", "fn", "ref", "f:pn"] {
            let outcome = state.handle_command(ProseCommand::ApplyMark {
                block_id: block,
                range: (0, 5),
                key: key.to_owned(),
                value: Some("x".to_owned()),
            });
            assert!(
                matches!(
                    refusal(&outcome),
                    Some(crate::events::ProseRefusal::AnchorKey(_))
                ),
                "{key} was written"
            );
        }
        assert_eq!(state.export_snapshot(), before);
        let link = state.handle_command(ProseCommand::ApplyMark {
            block_id: block,
            range: (0, 5),
            key: "a".to_owned(),
            value: Some("https://example.org".to_owned()),
        });
        assert!(link.update.is_some(), "the link is a style command's");
    }

    /// The style path cannot take an anchor off: a footnote call keeps its
    /// mark (`PROSE-FEAT-299`).
    #[test]
    fn a_style_command_never_removes_an_anchor_it_does_not_own() {
        let (mut state, block) = document_with_text("hello");
        state.handle_command(ProseCommand::InsertFootnote {
            block_id: block,
            pos: 5,
            note_text: "A note.".to_owned(),
        });
        let before = state.export_snapshot();
        let outcome = state.handle_command(ProseCommand::RemoveMark {
            block_id: block,
            range: (0, 6),
            key: "fn".to_owned(),
        });
        assert!(matches!(
            refusal(&outcome),
            Some(crate::events::ProseRefusal::AnchorKey(_))
        ));
        assert_eq!(state.export_snapshot(), before);
    }

    /// Applying one mark leaves the text, the other marks over the range, and
    /// the paragraph settings alone (`PROSE-FEAT-179`).
    #[test]
    fn applying_a_mark_leaves_the_text_and_the_other_keys_alone() {
        let (mut state, block) = document_with_text("hello");
        state.handle_command(ProseCommand::ApplyMark {
            block_id: block,
            range: (0, 5),
            key: "b".to_owned(),
            value: None,
        });
        state.handle_command(ProseCommand::SetParagraphStyle {
            block_id: block,
            style: ParagraphStyle {
                align: Some(Align::Center),
                ..ParagraphStyle::default()
            },
        });

        state.handle_command(ProseCommand::ApplyMark {
            block_id: block,
            range: (0, 5),
            key: "i".to_owned(),
            value: None,
        });

        assert_eq!(state.text_of(&block).unwrap_or_default(), "hello");
        let (_, style) = runs(&state, &block).remove(0);
        assert!(style.bold && style.italic);
        assert_eq!(state.paragraph_style_of(&block).align, Some(Align::Center));
    }

    /// Removing one key clears it over the range, keeps it outside, and leaves
    /// the text and the other keys alone (`PROSE-FEAT-180`, `PROSE-FEAT-287`,
    /// `PROSE-FEAT-181`).
    #[test]
    fn removing_one_key_clears_the_range_and_keeps_the_rest() {
        let (mut state, block) = document_with_text("one two");
        for key in ["b", "i"] {
            state.handle_command(ProseCommand::ApplyMark {
                block_id: block,
                range: (0, 7),
                key: key.to_owned(),
                value: None,
            });
        }

        state.handle_command(ProseCommand::RemoveMark {
            block_id: block,
            range: (0, 3),
            key: "b".to_owned(),
        });

        assert_eq!(state.text_of(&block).unwrap_or_default(), "one two");
        let resolved = runs(&state, &block);
        let bold: String = resolved
            .iter()
            .filter(|(_, style)| style.bold)
            .map(|(text, _)| text.clone())
            .collect();
        // The key covers no character of the range, and every one outside it.
        assert_eq!(bold, " two");
        assert!(
            resolved.iter().all(|(_, style)| style.italic),
            "the other key is untouched"
        );
    }

    /// Clearing removes the presentation marks and the link, keeps the text,
    /// and spares an anchor (`PROSE-FEAT-024`, `PROSE-FEAT-288`,
    /// `PROSE-FEAT-182`).
    #[test]
    fn clearing_strips_the_formatting_and_spares_the_anchors() {
        let (mut state, block) = document_with_text("hello");
        for (key, value) in [
            ("b", None),
            ("i", None),
            ("fg", Some("#3366cc")),
            ("a", Some("https://example.org")),
        ] {
            state.handle_command(ProseCommand::ApplyMark {
                block_id: block,
                range: (0, 5),
                key: key.to_owned(),
                value: value.map(str::to_owned),
            });
        }

        anchor(&mut state, &block, 0..5);
        state.handle_command(ProseCommand::ClearInlineMarks {
            block_id: block,
            range: (0, 5),
        });

        assert_eq!(state.text_of(&block).unwrap_or_default(), "hello");
        let (_, style) = runs(&state, &block).remove(0);
        assert!(!style.bold && !style.italic);
        assert_eq!(style.color, None);
        assert_eq!(style.link, None, "the link is presentation, and it goes");
        assert!(
            carries_mark(&state, &block, "cm"),
            "the comment carries an identity a reset must not destroy"
        );
    }

    /// Setting a paragraph's settings writes only what differs, and touches no
    /// other block (`PROSE-FEAT-025`, `PROSE-FEAT-183`).
    #[test]
    fn setting_a_paragraph_writes_only_what_differs() {
        let (mut state, block) = document_with_text("hello");
        state.handle_command(ProseCommand::ApplyMark {
            block_id: block,
            range: (0, 5),
            key: "b".to_owned(),
            value: None,
        });
        let wanted = ParagraphStyle {
            align: Some(Align::Center),
            space_before: Some(4.0),
            ..ParagraphStyle::default()
        };
        state.handle_command(ProseCommand::SetParagraphStyle {
            block_id: block,
            style: wanted,
        });
        // A second block, to hold the row that says no other block moves.
        state.handle_command(ProseCommand::InsertBlock {
            parent_id: None,
            after_id: Some(block),
            init: crate::events::BlockInit::Paragraph,
            style_id: None,
        });
        let other = *state.root_blocks().last().expect("the paragraph landed");
        let before = state.export_snapshot();

        // The same settings again: every property is already stored, so there
        // is nothing to write and no delta to export.
        let outcome = state.handle_command(ProseCommand::SetParagraphStyle {
            block_id: block,
            style: wanted,
        });
        assert!(outcome.update.is_none());
        assert_eq!(state.export_snapshot(), before);

        assert_eq!(state.text_of(&block).unwrap_or_default(), "hello");
        assert!(runs(&state, &block).remove(0).1.bold);
        assert_eq!(
            state.paragraph_style_of(&other).align,
            None,
            "no other block's settings moved"
        );
    }

    /// Typing at the edge of a span continues it, or does not, by the registry
    /// alone (`PROSE-FEAT-007`).
    ///
    /// The applying command names no policy. The document was configured from
    /// the registry at construction, so the same key behaves the same way
    /// whichever path wrote it.
    #[test]
    fn typing_at_a_span_edge_follows_the_registry() {
        for (key, value, continues) in
            [("b", None, true), ("a", Some("https://example.org"), false)]
        {
            let (mut state, block) = document_with_text("one");
            state.handle_command(ProseCommand::ApplyMark {
                block_id: block,
                range: (0, 3),
                key: key.to_owned(),
                value: value.map(str::to_owned),
            });
            state.handle_command(ProseCommand::InsertText {
                block_id: block,
                pos: 3,
                text: "X".to_owned(),
            });
            let covered: String = runs(&state, &block)
                .into_iter()
                .filter(|(_, style)| style.bold || style.link.is_some())
                .map(|(text, _)| text)
                .collect();
            assert_eq!(covered == "oneX", continues, "{key}");
        }
    }
}
