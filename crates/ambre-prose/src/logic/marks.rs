//! The write path of the inline mark registry
//! (`ambre-spec-prose-schema.md section 3`).
//!
//! A handler never states an expansion policy. The document was configured from
//! [`crate::mark::expand_type_for`] at construction, so the policy has one
//! author and one place (`PROSE-007`). A key the registry does not declare is
//! refused here rather than left to the CRDT engine, so the refusal names the
//! registry.

use loro::TreeID;

use crate::events::{BlockId, ProseOutcome, error_code, tree_id};
use crate::mark;

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
        if !mark::is_registered(key) {
            return ProseOutcome::refused(
                error_code::BAD_RANGE,
                format!("the mark registry declares no key {key}"),
            );
        }
        let (start, end) = ordered(range);
        if end > body.len_unicode() || start == end {
            return Self::err_bad_range();
        }
        let stored = match value {
            Some(raw) => match mark::canonical_value(key, raw) {
                Some(canonical) => canonical,
                None => {
                    return ProseOutcome::refused(
                        error_code::BAD_RANGE,
                        "the mark value has no canonical spelling",
                    );
                }
            },
            None => String::new(),
        };
        if let Err(e) = body.mark(start..end, key, stored.as_str()) {
            return Self::err_write(&format!("{e:?}"));
        }
        self.commit_with(Vec::new())
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
            return ProseOutcome::refused(
                error_code::BAD_RANGE,
                format!("the mark registry declares no key {key}"),
            );
        }
        let (start, end) = ordered(range);
        if end > body.len_unicode() || start == end {
            return Self::err_bad_range();
        }
        if let Err(e) = body.unmark(start..end, key) {
            return Self::err_write(&format!("{e:?}"));
        }
        self.commit_with(Vec::new())
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
        self.commit_with(Vec::new())
    }

    /// The marks a range carries, as the rich-text value Loro answers with.
    /// A read for a test and for the toolbar's own state.
    #[must_use]
    pub fn marks_of(&self, id: &BlockId) -> Option<loro::LoroValue> {
        self.body_of(&tree_id(*id))
            .map(|body| body.get_richtext_value())
    }
}

/// The two ends of a range, smaller first.
fn ordered(range: (u32, u32)) -> (usize, usize) {
    let (a, b) = (range.0 as usize, range.1 as usize);
    if a <= b { (a, b) } else { (b, a) }
}

#[cfg(test)]
mod tests {
    use crate::events::ProseCommand;
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
        state.handle_command(ProseCommand::ApplyMark {
            block_id: block,
            range: (0, 5),
            key: "cm".to_owned(),
            value: Some("a-comment-id".to_owned()),
        });
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
}
