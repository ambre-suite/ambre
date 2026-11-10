//! The write path of the inline mark registry
//! (`ambre-spec-prose-schema.md section 3`).
//!
//! A handler never states an expansion policy. The document was configured from
//! [`crate::mark::expand_type_for`] at construction, so the policy has one
//! author and one place (`PROSE-007`). A key the registry does not declare is
//! refused here rather than left to the CRDT engine, so the refusal names the
//! registry.

use crate::message::{DocChar, MarkKey};
use ambre_core::Opaque;
use ambre_core::index::{Len, Pos, Span};
use std::collections::BTreeMap;

use loro::TreeID;

use loro::LoroValue;

use crate::doc;
use crate::events::{
    ActiveMarks, BlockId, BlockType, ProseEvent, ProseOutcome, ProseRefusal, TriState, tree_id,
};
use crate::mark::{self, MarkRoot};
use crate::style::{BlockContext, ResolvedStyle, StoredMarkValues, resolve};

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
        range: Span<DocChar>,
        key: &MarkKey,
        value: Option<&str>,
    ) -> ProseOutcome {
        let Some(body) = self.body_of(id) else {
            return Self::err_no_block(id);
        };
        if mark::is_owned_anchor(key) {
            return ProseOutcome::refused(ProseRefusal::AnchorKey(key.clone()));
        }
        let span = ordered(range);
        let (start, end) = (span.start.to_usize(), span.end.to_usize());
        if end > body.len_unicode() || start == end {
            return Self::err_bad_range();
        }
        // The grammar of the key is checked before the document is touched, so
        // no unparseable value ever reaches the format (`PROSE-FEAT-022`).
        let stored = match mark::validate_value(key, value) {
            Ok(canonical) => canonical,
            Err(refusal) => return ProseOutcome::refused(refused_as(key, value, &refusal)),
        };
        if let Err(e) = body.mark(start..end, key.as_str(), stored.as_str()) {
            return Self::err_write(e);
        }
        self.commit_with(self.links_touched(Some(key)))
    }

    /// The link list, when the command touched links: a mark under the link
    /// key, or a clear, which takes links along. Nothing otherwise.
    fn links_touched(&self, key: Option<&MarkKey>) -> Vec<ProseEvent> {
        if key.is_some_and(|key| key.as_str() != MarkRoot::Link.as_str()) {
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
        range: Span<DocChar>,
        key: &MarkKey,
    ) -> ProseOutcome {
        let Some(body) = self.body_of(id) else {
            return Self::err_no_block(id);
        };
        if !mark::is_registered(key) {
            return ProseOutcome::refused(ProseRefusal::UnknownMark(key.clone()));
        }
        if mark::is_owned_anchor(key) {
            return ProseOutcome::refused(ProseRefusal::AnchorKey(key.clone()));
        }
        let span = ordered(range);
        let (start, end) = (span.start.to_usize(), span.end.to_usize());
        if end > body.len_unicode() || start == end {
            return Self::err_bad_range();
        }
        if let Err(e) = body.unmark(start..end, key.as_str()) {
            return Self::err_write(e);
        }
        self.commit_with(self.links_touched(Some(key)))
    }

    /// Remove every formatting mark from a range.
    ///
    /// The formatting keys alone (`ambre-spec-prose-schema.md section 3`). A
    /// comment, a suggestion, a footnote and a cross reference are anchors
    /// rather than formatting, and each has a feature that owns its removal.
    pub(super) fn clear_inline_marks(&mut self, id: &TreeID, range: Span<DocChar>) -> ProseOutcome {
        let Some(body) = self.body_of(id) else {
            return Self::err_no_block(id);
        };
        let span = ordered(range);
        let (start, end) = (span.start.to_usize(), span.end.to_usize());
        if end > body.len_unicode() || start == end {
            return Self::err_bad_range();
        }
        for root in mark::FORMATTING_KEYS {
            let key = root.as_str();
            if let Err(e) = body.unmark(start..end, key) {
                tracing::warn!("Prose: the text refused to clear the mark {key}: {e:?}");
            }
        }
        self.commit_with(self.links_touched(None))
    }

    /// One block's text, run by run, each with the style the cascade resolves
    /// for it (`PROSE-FEAT-311`).
    ///
    /// The read the view draws from, and the one a test asserts on. It writes
    /// nothing: the resolution is a read of the marks, the block and the
    /// document defaults.
    #[must_use]
    pub fn resolved_runs(&self, id: &BlockId) -> Vec<(String, ResolvedStyle)> {
        let tree = doc::blocks(&self.doc);
        let context = BlockContext {
            block_type: doc::block_type_of(&tree, &tree_id(*id)).unwrap_or(BlockType::Paragraph),
            heading_level: doc::heading_level_of(&tree, &tree_id(*id))
                .map(doc::StoredLevel::clamped),
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

impl ProseState {
    /// The marks a selection carries, one state per control of the format
    /// bar (`PROSE-UI-054`). It writes nothing (`PROSE-UI-099`).
    ///
    /// A key carried by some of the selected text and not all of it is
    /// mixed, whichever end carries it (`PROSE-UI-096`).
    pub(super) fn read_marks(
        &self,
        anchor: (BlockId, Pos<DocChar>),
        focus: (BlockId, Pos<DocChar>),
    ) -> ProseOutcome {
        let mut fold = MarkFold::default();
        for (block, span) in self.selected_spans(anchor, focus) {
            let Some(LoroValue::List(runs)) = self.marks_of(&block) else {
                continue;
            };
            let mut at = Pos::<DocChar>::ZERO;
            for run in runs.iter() {
                let LoroValue::Map(fields) = run else {
                    continue;
                };
                let Some(LoroValue::String(text)) = fields.get(RUN_TEXT) else {
                    continue;
                };
                let len = ambre_core::char_len(text).as_axis::<DocChar>();
                let covered = span.end.min(at + len).len_from(span.start.max(at));
                if !covered.is_zero() {
                    fold.take(&marks_of_run(run));
                }
                at += len;
            }
        }
        let mut outcome = ProseOutcome::nothing();
        outcome.display.push(ProseEvent::SelectionMarks {
            anchor,
            focus,
            marks: fold.finish(),
        });
        outcome
    }

    /// The text a selection covers, one span per block, in document order.
    /// A block with no body, a table among them, contributes nothing.
    /// Apply or remove one mark over every block a selection covers, in one
    /// commit: one gesture is one undo item. A key the bar does not name is
    /// refused, since only a presence mark needs no value.
    pub(super) fn format_selection(
        &mut self,
        anchor: (BlockId, Pos<DocChar>),
        focus: (BlockId, Pos<DocChar>),
        key: &MarkKey,
        action: crate::message::MarkAction,
    ) -> ProseOutcome {
        // The whole key, not its root: `b:x` names no control of the bar.
        if !mark::MarkRoot::exactly(key).is_some_and(|root| BAR_KEYS.contains(&root)) {
            return ProseOutcome::refused(ProseRefusal::UnknownMark(key.clone()));
        }
        let spans: Vec<_> = self
            .selected_spans(anchor, focus)
            .into_iter()
            .filter(|(_, span)| !span.is_empty())
            .collect();
        if spans.is_empty() {
            return Self::err_bad_range();
        }
        for (block, range) in spans {
            let span = ordered(range);
            let (start, end) = (span.start.to_usize(), span.end.to_usize());
            let Some(body) = self.body_of(&tree_id(block)) else {
                return Self::err_no_block(&tree_id(block));
            };
            if end > body.len_unicode() {
                return Self::err_bad_range();
            }
            let written = match action {
                crate::message::MarkAction::Apply => body.mark(start..end, key.as_str(), ""),
                crate::message::MarkAction::Remove => body.unmark(start..end, key.as_str()),
            };
            if let Err(e) = written {
                return Self::err_write(e);
            }
        }
        self.commit_with(Vec::new())
    }

    pub(super) fn selected_spans(
        &self,
        anchor: (BlockId, Pos<DocChar>),
        focus: (BlockId, Pos<DocChar>),
    ) -> Vec<(BlockId, Span<DocChar>)> {
        if anchor.0 == focus.0 {
            let (a, b) = (anchor.1.min(focus.1), anchor.1.max(focus.1));
            return vec![(anchor.0, Span::<DocChar>::new(a, b))];
        }
        let order = self.document_order();
        let at = |spot: (BlockId, Pos<DocChar>)| order.iter().position(|id| *id == tree_id(spot.0));
        let (Some(a), Some(f)) = (at(anchor), at(focus)) else {
            return Vec::new();
        };
        let ((head, head_at), (tail, tail_at)) = if a <= f {
            ((anchor, a), (focus, f))
        } else {
            ((focus, f), (anchor, a))
        };
        order
            .get(head_at..=tail_at)
            .unwrap_or_default()
            .iter()
            .filter_map(|id| {
                let len = Len::<DocChar>::of(self.body_of(id)?.len_unicode()).as_end();
                let block = crate::events::block_id(*id);
                let start = if block == head.0 {
                    head.1
                } else {
                    Pos::<DocChar>::ZERO
                };
                let end = if block == tail.0 { tail.1 } else { len };
                Some((block, Span::<DocChar>::new(start, end.min(len))))
            })
            .collect()
    }
}

/// The five on-or-off keys of the format bar, in the order of
/// [`ActiveMarks`].
const BAR_KEYS: [MarkRoot; 5] = [
    MarkRoot::Bold,
    MarkRoot::Italic,
    MarkRoot::Underline,
    MarkRoot::Strike,
    MarkRoot::Code,
];

/// What the runs folded so far say of one bar key. The bar reads whether
/// some run carries the key and whether some run lacks it, never how many.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum KeySeen {
    /// No run folded yet.
    #[default]
    Unseen,
    /// Every run folded carries the key.
    AllOn,
    /// No run folded carries the key.
    AllOff,
    /// Some runs carry the key and some lack it.
    Mixed,
}

impl KeySeen {
    /// What the fold says once one more run, which carries the key or not,
    /// is folded in.
    fn with(self, carried: bool) -> Self {
        match (self, carried) {
            (Self::Unseen | Self::AllOn, true) => Self::AllOn,
            (Self::Unseen | Self::AllOff, false) => Self::AllOff,
            _ => Self::Mixed,
        }
    }

    /// The state the bar shows for the key.
    fn state(self) -> TriState {
        match self {
            Self::Unseen | Self::AllOff => TriState::Off,
            Self::AllOn => TriState::On,
            Self::Mixed => TriState::Mixed,
        }
    }
}

/// What the selected runs carry, run by run.
#[derive(Default)]
struct MarkFold {
    /// Per bar key: whether the runs carry it.
    seen: [KeySeen; 5],
    /// The link every run carries so far: `None` before the first run, and
    /// `Some(None)` once two runs disagree or one carries none.
    link: Option<Option<ambre_core::LinkUrl>>,
}

impl MarkFold {
    fn take(&mut self, marks: &BTreeMap<MarkKey, Opaque<StoredMarkValues>>) {
        for (key, seen) in BAR_KEYS.iter().zip(self.seen.iter_mut()) {
            *seen = seen.with(marks.contains_key(key.as_str()));
        }
        let this = marks
            .get(MarkRoot::Link.as_str())
            .map(|value| ambre_core::LinkUrl::new(value.as_str().to_owned()));
        self.link = match self.link.take() {
            None => Some(this),
            Some(held) if held == this => Some(held),
            Some(_) => Some(None),
        };
    }

    fn finish(self) -> ActiveMarks {
        let [bold, italic, underline, strikethrough, code] = self.seen.map(KeySeen::state);
        ActiveMarks {
            bold,
            italic,
            underline,
            strikethrough,
            code,
            link: self.link.flatten(),
        }
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
fn refused_as(key: &MarkKey, value: Option<&str>, error: &mark::MarkValueError) -> ProseRefusal {
    match error {
        mark::MarkValueError::UnknownKey(key) => ProseRefusal::UnknownMark(key.clone()),
        _ => ProseRefusal::BadMarkValue {
            key: key.clone(),
            value: value.unwrap_or_default().to_owned(),
        },
    }
}

/// One run's marks, as the cascade reads them.
///
/// A run Loro carries no mark field for carries no mark, which is an empty set
/// rather than a missing one.
fn marks_of_run(fields: &LoroValue) -> BTreeMap<MarkKey, Opaque<StoredMarkValues>> {
    let LoroValue::Map(fields) = fields else {
        return BTreeMap::new();
    };
    let Some(LoroValue::Map(marks)) = fields.get(RUN_MARKS) else {
        return BTreeMap::new();
    };
    marks
        .iter()
        .map(|(key, value)| {
            let stored = match value {
                LoroValue::String(text) => Opaque::<StoredMarkValues>::new(text.to_string()),
                _ => Opaque::<StoredMarkValues>::default(),
            };
            (MarkKey::new(key.clone()), stored)
        })
        .collect()
}

/// The two ends of a range, smaller first.
fn ordered(range: Span<DocChar>) -> Span<DocChar> {
    let (a, b) = (range.start, range.end);
    if a <= b {
        range
    } else {
        Span::<DocChar>::new(b, a)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use crate::events::{Align, ParagraphStyle, ProseCommand, ProseEvent};
    use crate::logic::ProseState;
    use crate::message::DocChar;
    use ambre_core::index::{Pos, Span};
    use ambre_core::{PrintLength, Pt};

    /// The fold of one key reads as the count of the runs that carry it and
    /// of the runs that lack it. Every order of up to four runs holds it.
    #[test]
    fn a_folded_key_reads_as_the_counts_of_its_runs() {
        use super::KeySeen;
        use crate::message::TriState;
        for runs in 0_u32..(1 << 4) {
            for len in 0..=4 {
                let carried: Vec<bool> = (0..len).map(|at| runs & (1 << at) != 0).collect();
                let on = carried.iter().filter(|c| **c).count();
                let off = carried.len() - on;
                let counted = match (on, off) {
                    (0, _) => TriState::Off,
                    (_, 0) => TriState::On,
                    _ => TriState::Mixed,
                };
                let folded = carried
                    .iter()
                    .fold(KeySeen::Unseen, |seen, c| seen.with(*c));
                assert_eq!(folded.state(), counted, "{carried:?}");
            }
        }
    }

    fn document_with_text(text: &str) -> (ProseState, crate::events::BlockId) {
        let mut state = ProseState::new().expect("a document is built");
        let block = state.root_blocks()[0];
        state.handle_command(ProseCommand::InsertText {
            block_id: block,
            pos: Pos::<DocChar>::ZERO,
            text: text.to_owned(),
        });
        (state, block)
    }

    /// Whether the rich-text value carries `key` anywhere.
    fn carries_mark(state: &ProseState, block: &crate::events::BlockId, key: &str) -> bool {
        format!("{:?}", state.marks_of(block)).contains(key)
    }

    /// The width of the first line `block` is laid out in, on the layout
    /// the view is handed.
    fn laid_width(state: &mut ProseState, block: crate::events::BlockId) -> f32 {
        let Some(ProseEvent::LayoutShaped { layout }) = state.layout_event() else {
            panic!("the edit is laid out");
        };
        let shaped = layout
            .get::<crate::layout::run::ShapedLayout>()
            .expect("a shaped layout");
        let run = shaped
            .blocks
            .get(&crate::events::tree_id(block))
            .expect("the run");
        (run.data().full_width()).get()
    }

    /// `PROSE-UI-207`: a bold or italic mark reaches the glyphs it covers
    /// on the next layout, and leaves them when it is removed.
    #[test]
    fn a_bold_or_italic_mark_reaches_the_glyphs() {
        let (mut state, block) = document_with_text("mmmm mmmm");
        state.load_faces(&crate::layout::fonts::shipped_faces());
        let plain = laid_width(&mut state, block);
        for key in ["b", "i"] {
            state.handle_command(ProseCommand::ApplyMark {
                block_id: block,
                range: Span::<DocChar>::from_usize_range(0..4),
                key: key.into(),
                value: None,
            });
            let marked = laid_width(&mut state, block);
            assert!((marked - plain).abs() > 0.5, "{key} changes the glyphs");
            state.handle_command(ProseCommand::RemoveMark {
                block_id: block,
                range: Span::<DocChar>::from_usize_range(0..4),
                key: key.into(),
            });
            let back = laid_width(&mut state, block);
            assert!((back - plain).abs() < 0.01, "removing {key} restores them");
        }
    }

    #[test]
    fn a_size_reaches_the_document_in_its_canonical_spelling() {
        let (mut state, block) = document_with_text("hello");
        state.handle_command(ProseCommand::ApplyMark {
            block_id: block,
            range: Span::<DocChar>::from_usize_range(0..5),
            key: "fs".into(),
            value: Some(crate::mark::test_value("11.0")),
        });
        let rendered = format!("{:?}", state.marks_of(&block));
        assert!(rendered.contains("\"11\""), "{rendered}");
    }

    #[test]
    fn a_key_the_registry_does_not_declare_is_refused() {
        let (mut state, block) = document_with_text("hello");
        let outcome = state.handle_command(ProseCommand::ApplyMark {
            block_id: block,
            range: Span::<DocChar>::from_usize_range(0..5),
            key: "no-such-key".into(),
            value: None,
        });
        assert!(outcome.update.is_none());
    }

    #[test]
    fn clearing_the_formatting_leaves_an_anchor_in_place() {
        let (mut state, block) = document_with_text("hello");
        state.handle_command(ProseCommand::ApplyMark {
            block_id: block,
            range: Span::<DocChar>::from_usize_range(0..5),
            key: "b".into(),
            value: None,
        });
        anchor(&mut state, &block, 0..5);
        state.handle_command(ProseCommand::ClearInlineMarks {
            block_id: block,
            range: Span::<DocChar>::from_usize_range(0..5),
        });
        assert!(carries_mark(&state, &block, "cm"));
        assert!(!carries_mark(&state, &block, "\"b\""));
    }

    #[test]
    fn an_empty_range_is_refused() {
        let (mut state, block) = document_with_text("hello");
        let outcome = state.handle_command(ProseCommand::ApplyMark {
            block_id: block,
            range: Span::<DocChar>::from_usize_range(2..2),
            key: "b".into(),
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
            range: Span::<DocChar>::from_usize_range(0..3),
            key: "b".into(),
            value: None,
        });
        let bolded: String = runs(&state, &block)
            .into_iter()
            .filter(|(_, style)| style.bold.as_bool())
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
            range: Span::<DocChar>::from_usize_range(0..5),
            key: "fg".into(),
            // The opaque eight-digit spelling of the same colour.
            value: Some(crate::mark::test_value("#3366CCFF")),
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
                range: Span::<DocChar>::from_usize_range(0..5),
                key: key.into(),
                value: value.map(crate::mark::test_value),
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
                range: Span::<DocChar>::from_usize_range(0..5),
                key: key.into(),
                value: Some(crate::mark::test_value("x")),
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
            range: Span::<DocChar>::from_usize_range(0..5),
            key: "a".into(),
            value: Some(crate::mark::test_value("https://example.org")),
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
            pos: Pos::<DocChar>::new(5),
            note_text: "A note.".to_owned(),
        });
        let before = state.export_snapshot();
        let outcome = state.handle_command(ProseCommand::RemoveMark {
            block_id: block,
            range: Span::<DocChar>::from_usize_range(0..6),
            key: "fn".into(),
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
            range: Span::<DocChar>::from_usize_range(0..5),
            key: "b".into(),
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
            range: Span::<DocChar>::from_usize_range(0..5),
            key: "i".into(),
            value: None,
        });

        assert_eq!(state.text_of(&block).unwrap_or_default(), "hello");
        let (_, style) = runs(&state, &block).remove(0);
        assert!(style.bold.as_bool() && style.italic.as_bool());
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
                range: Span::<DocChar>::from_usize_range(0..7),
                key: key.into(),
                value: None,
            });
        }

        state.handle_command(ProseCommand::RemoveMark {
            block_id: block,
            range: Span::<DocChar>::from_usize_range(0..3),
            key: "b".into(),
        });

        assert_eq!(state.text_of(&block).unwrap_or_default(), "one two");
        let resolved = runs(&state, &block);
        let bold: String = resolved
            .iter()
            .filter(|(_, style)| style.bold.as_bool())
            .map(|(text, _)| text.clone())
            .collect();
        // The key covers no character of the range, and every one outside it.
        assert_eq!(bold, " two");
        assert!(
            resolved.iter().all(|(_, style)| style.italic.as_bool()),
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
                range: Span::<DocChar>::from_usize_range(0..5),
                key: key.into(),
                value: value.map(crate::mark::test_value),
            });
        }

        anchor(&mut state, &block, 0..5);
        state.handle_command(ProseCommand::ClearInlineMarks {
            block_id: block,
            range: Span::<DocChar>::from_usize_range(0..5),
        });

        assert_eq!(state.text_of(&block).unwrap_or_default(), "hello");
        let (_, style) = runs(&state, &block).remove(0);
        assert!(!style.bold.as_bool() && !style.italic.as_bool());
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
            range: Span::<DocChar>::from_usize_range(0..5),
            key: "b".into(),
            value: None,
        });
        let wanted = ParagraphStyle {
            align: Some(Align::Center),
            space_before: Some(PrintLength::<Pt>::new(4.0)),
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
        assert!(runs(&state, &block).remove(0).1.bold.as_bool());
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
                range: Span::<DocChar>::from_usize_range(0..3),
                key: key.into(),
                value: value.map(crate::mark::test_value),
            });
            state.handle_command(ProseCommand::InsertText {
                block_id: block,
                pos: Pos::<DocChar>::new(3),
                text: "X".to_owned(),
            });
            let covered: String = runs(&state, &block)
                .into_iter()
                .filter(|(_, style)| style.bold.as_bool() || style.link.is_some())
                .map(|(text, _)| text)
                .collect();
            assert_eq!(covered == "oneX", continues, "{key}");
        }
    }
}
