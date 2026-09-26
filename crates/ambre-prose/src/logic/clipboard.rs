//! Copy and paste, where the document lives
//! (`features/ambre-spec-prose-clipboard.md`).
//!
//! A copy walks the whole selection in the document (`PROSE-UI-040`). It
//! answers with the plain text every application reads (`PROSE-UI-108`), and
//! with this application's own bytes, which keep the marks and the blocks. A rich paste writes those bytes back in one commit, so one
//! undo takes it back (`PROSE-UI-107`).

use loro::{LoroValue, TreeID};
use serde::{Deserialize, Serialize};

use ambre_bus::app::prose::{BlockInit, ListStyle, OrderedStyle};

use super::ProseState;
use super::text::{MarkedRun, marked_runs, write_runs};
use crate::doc;
use crate::events::{BlockId, ProseEvent, ProseOutcome, block_id, tree_id};
use crate::mark::FORMATTING_KEYS;
use ambre_bus::app::prose::BlockType;

/// What opens the bytes, so a payload another Ambre application wrote under
/// the same clipboard type never decodes as this one.
const MAGIC: [u8; 4] = *b"APR1";

/// What a copy carries in this application's own bytes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct Payload {
    magic: [u8; 4],
    blocks: Vec<ClipBlock>,
}

/// One block of a selection: its type and the runs of text it covered.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct ClipBlock {
    kind: ClipKind,
    runs: Vec<ClipRun>,
}

/// The block types a copy keeps. Any other block with a body pastes as a
/// paragraph.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
enum ClipKind {
    Paragraph,
    Heading(u8),
    Bullet,
    Ordered(u8),
    Todo(bool),
    Code(String),
}

/// A run of text and the formatting marks it carries. Anchors stay behind:
/// a comment or a suggestion does not travel with its text.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct ClipRun {
    text: String,
    marks: Vec<(String, ClipValue)>,
}

/// A mark's value, in the shapes the registry writes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
enum ClipValue {
    Bool(bool),
    Int(i64),
    Float(f64),
    Text(String),
}

impl ClipValue {
    fn of(value: &LoroValue) -> Option<Self> {
        Some(match value {
            LoroValue::Bool(on) => Self::Bool(*on),
            LoroValue::I64(n) => Self::Int(*n),
            LoroValue::Double(x) => Self::Float(*x),
            LoroValue::String(text) => Self::Text(text.to_string()),
            _ => return None,
        })
    }

    fn value(&self) -> LoroValue {
        match self {
            Self::Bool(on) => LoroValue::from(*on),
            Self::Int(n) => LoroValue::from(*n),
            Self::Float(x) => LoroValue::from(*x),
            Self::Text(text) => LoroValue::from(text.as_str()),
        }
    }
}

impl ClipKind {
    fn init(&self) -> BlockInit {
        match self {
            Self::Paragraph => BlockInit::Paragraph,
            Self::Heading(level) => BlockInit::Heading { level: *level },
            Self::Bullet => BlockInit::ListItem {
                style: ListStyle::Bullet,
            },
            Self::Ordered(code) => BlockInit::ListItem {
                style: ListStyle::Ordered(
                    OrderedStyle::from_code(*code).unwrap_or(OrderedStyle::Decimal),
                ),
            },
            Self::Todo(done) => BlockInit::ListItem {
                style: ListStyle::Todo(*done),
            },
            Self::Code(lang) => BlockInit::CodeBlock { lang: lang.clone() },
        }
    }
}

impl ClipRun {
    fn of(run: MarkedRun) -> Self {
        let marks = run
            .marks
            .iter()
            .filter(|(key, _)| FORMATTING_KEYS.contains(&key.as_str()))
            .filter_map(|(key, value)| Some((key.clone(), ClipValue::of(value)?)))
            .collect();
        Self {
            text: run.text,
            marks,
        }
    }

    fn marked(&self) -> MarkedRun {
        MarkedRun {
            text: self.text.clone(),
            marks: self
                .marks
                .iter()
                .map(|(key, value)| (key.clone(), value.value()))
                .collect(),
        }
    }
}

fn marked(block: &ClipBlock) -> Vec<MarkedRun> {
    block.runs.iter().map(ClipRun::marked).collect()
}

fn len_of(block: &ClipBlock) -> u32 {
    block
        .runs
        .iter()
        .map(|run| crate::ui::chars(&run.text))
        .sum()
}

impl ProseState {
    /// The kind a block pastes back as.
    fn clip_kind(&self, id: &TreeID) -> ClipKind {
        let tree = doc::blocks(&self.doc);
        match doc::block_type_of(&tree, id) {
            Some(BlockType::Heading) => {
                ClipKind::Heading(doc::heading_level_of(&tree, id).unwrap_or(1))
            }
            Some(BlockType::ListItem) => match doc::list_style_of(&tree, id) {
                Some(ListStyle::Ordered(style)) => ClipKind::Ordered(style.code()),
                Some(ListStyle::Todo(done)) => ClipKind::Todo(done),
                _ => ClipKind::Bullet,
            },
            Some(BlockType::CodeBlock) => {
                ClipKind::Code(self.code_lang_of(&block_id(*id)).unwrap_or_default())
            }
            _ => ClipKind::Paragraph,
        }
    }

    /// The payload of a whole selection, whatever the view shows of it
    /// (`PROSE-UI-040`). It writes nothing (`PROSE-UI-109`).
    pub(super) fn copy_range(&self, anchor: (BlockId, u32), focus: (BlockId, u32)) -> ProseOutcome {
        let mut blocks = Vec::new();
        let mut text = String::new();
        for (block, (start, end)) in self.selected_spans(anchor, focus) {
            let id = tree_id(block);
            let Some(body) = self.body_of(&id) else {
                continue;
            };
            let runs: Vec<ClipRun> = marked_runs(&body, start as usize, end as usize)
                .into_iter()
                .map(ClipRun::of)
                .collect();
            if !blocks.is_empty() {
                text.push('\n');
            }
            text.extend(runs.iter().map(|run| run.text.as_str()));
            blocks.push(ClipBlock {
                kind: self.clip_kind(&id),
                runs,
            });
        }
        let mut outcome = ProseOutcome::nothing();
        let payload = Payload {
            magic: MAGIC,
            blocks,
        };
        match postcard::to_allocvec(&payload) {
            Ok(payload) => outcome
                .display
                .push(ProseEvent::Clipboard { text, payload }),
            Err(e) => tracing::warn!("Prose: a copy could not be encoded: {e}"),
        }
        outcome
    }

    /// Write plain text of several lines at `at`, one paragraph per line,
    /// the way a rich payload of paragraphs lands (`PROSE-UI-042`).
    pub(super) fn paste_text(&mut self, at: (TreeID, u32), text: &str) -> ProseOutcome {
        let blocks = text
            .split('\n')
            .map(|line| ClipBlock {
                kind: ClipKind::Paragraph,
                runs: vec![ClipRun {
                    text: line.chars().filter(|c| !c.is_control()).collect(),
                    marks: Vec::new(),
                }],
            })
            .collect();
        self.paste_blocks(
            at,
            Payload {
                magic: MAGIC,
                blocks,
            },
        )
    }

    /// Write a payload this application copied at `at`, in one commit. The
    /// first block's text joins the paragraph at `at`, and the others land as
    /// blocks of their own kind. The last one takes the text that followed
    /// `at` (`PROSE-UI-107`). Nothing outside that paragraph changes
    /// (`PROSE-UI-112`). The caret lands after what was pasted.
    pub(super) fn paste_rich(
        &mut self,
        at: (TreeID, u32),
        bytes: &[u8],
        text: &str,
    ) -> ProseOutcome {
        match postcard::from_bytes::<Payload>(bytes) {
            Ok(payload) if payload.magic == MAGIC => self.paste_blocks(at, payload),
            // Another application's bytes: its plain text comes in instead.
            _ => self.paste_text(at, text),
        }
    }

    fn paste_blocks(&mut self, at: (TreeID, u32), payload: Payload) -> ProseOutcome {
        let Some(body) = self.body_of(&at.0) else {
            return Self::err_no_block(&at.0);
        };
        let pos = at.1 as usize;
        let len = body.len_unicode();
        if pos > len {
            return Self::err_bad_range();
        }
        let Some((first, rest)) = payload.blocks.split_first() else {
            return ProseOutcome::nothing();
        };
        let Some((last, middle)) = rest.split_last() else {
            if let Err(e) = write_runs(&body, pos, &marked(first)) {
                return Self::err_write(&e);
            }
            return self.commit_with(vec![ProseEvent::CaretPlaced {
                block: block_id(at.0),
                offset: at.1 + len_of(first),
            }]);
        };
        // A band or a note is one block: a paste of several has nowhere to go.
        if doc::is_aside(&doc::blocks(&self.doc), &at.0) {
            return Self::err_nesting();
        }
        let tail = marked_runs(&body, pos, len);
        if pos < len
            && let Err(e) = body.delete(pos, len - pos)
        {
            return Self::err_write(&format!("{e:?}"));
        }
        if let Err(e) = write_runs(&body, pos, &marked(first)) {
            return Self::err_write(&e);
        }
        let mut previous = at.0;
        for block in middle.iter().chain([last]) {
            let init = block.kind.init();
            let id = match self.mint_placed(None, Some(previous), init.block_type()) {
                Ok(id) => id,
                Err(refused) => return refused,
            };
            if let Err(e) = self.write_init(&id, &init) {
                return Self::err_write(&e);
            }
            let text = doc::get_block_text(&self.doc, &id);
            if let Err(e) = write_runs(&text, 0, &marked(block)) {
                return Self::err_write(&e);
            }
            previous = id;
        }
        let text = doc::get_block_text(&self.doc, &previous);
        let offset = len_of(last);
        if let Err(e) = write_runs(&text, offset as usize, &tail) {
            return Self::err_write(&e);
        }
        self.commit_with(vec![ProseEvent::CaretPlaced {
            block: block_id(previous),
            offset,
        }])
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use crate::events::{BlockInit, ProseCommand};
    use crate::view::tests::append;

    fn copied(
        state: &mut ProseState,
        anchor: (BlockId, u32),
        focus: (BlockId, u32),
    ) -> (String, Vec<u8>, bool) {
        let outcome = state.handle_command(ProseCommand::CopyRange { anchor, focus });
        let wrote = outcome.update.is_some();
        let Some(ProseEvent::Clipboard { text, payload }) = outcome.display.into_iter().next()
        else {
            panic!("a copy answers with the clipboard");
        };
        (text, payload, wrote)
    }

    fn document() -> (ProseState, BlockId) {
        let state = ProseState::with_peer(1).expect("a document is built");
        let first = state.root_blocks()[0];
        (state, first)
    }

    /// `PROSE-UI-040`, `PROSE-UI-108`, `PROSE-UI-109`: a copy covers the whole
    /// selection, however many blocks it spans, and writes nothing.
    #[test]
    fn a_copy_covers_the_whole_selection_and_writes_nothing() {
        let (mut state, _) = document();
        let blocks: Vec<BlockId> = (0..200)
            .map(|n| append(&mut state, BlockInit::Paragraph, &format!("line {n}")))
            .collect();
        let (first, last) = (blocks[0], blocks[199]);
        let (text, _, wrote) = copied(&mut state, (first, 0), (last, 8));
        let lines: Vec<&str> = text.split('\n').collect();
        assert_eq!(lines.len(), 200);
        assert_eq!(lines[0], "line 0");
        assert_eq!(lines[199], "line 199");
        assert!(!wrote, "a copy writes nothing");
    }

    /// `PROSE-UI-107`: a paste of this application's bytes keeps the block
    /// types and the formatting marks, and anchors stay behind.
    #[test]
    fn a_rich_paste_keeps_types_and_marks() {
        let (mut state, _) = document();
        let heading = append(&mut state, BlockInit::Heading { level: 2 }, "Title");
        let bold = append(&mut state, BlockInit::Paragraph, "bold text");
        state.handle_command(ProseCommand::ApplyMark {
            block_id: bold,
            range: (0, 4),
            key: "b".to_owned(),
            value: None,
        });
        let item = append(
            &mut state,
            BlockInit::ListItem {
                style: ListStyle::Bullet,
            },
            "item",
        );
        let (text, payload, _) = copied(&mut state, (heading, 0), (item, 4));
        let target = append(&mut state, BlockInit::Paragraph, "");
        state.handle_command(ProseCommand::PasteRich {
            at: (target, 0),
            payload,
            text,
        });
        let roots = state.root_blocks();
        let pasted = &roots[roots.len() - 3..];
        assert_eq!(state.text_of(&pasted[0]).as_deref(), Some("Title"));
        assert_eq!(state.text_of(&pasted[1]).as_deref(), Some("bold text"));
        assert_eq!(state.text_of(&pasted[2]).as_deref(), Some("item"));
        assert_eq!(state.block_type_of(&pasted[1]), Some(BlockType::Paragraph));
        assert_eq!(state.block_type_of(&pasted[2]), Some(BlockType::ListItem));
        assert!(format!("{:?}", state.marks_of(&pasted[1])).contains("\"b\""));
    }

    /// `PROSE-UI-042`: bytes this application did not write paste as their
    /// plain text, one paragraph per line.
    #[test]
    fn foreign_bytes_paste_as_plain_text() {
        let (mut state, first) = document();
        state.handle_command(ProseCommand::PasteRich {
            at: (first, 0),
            payload: b"not a payload".to_vec(),
            text: "one\ntwo".to_owned(),
        });
        let roots = state.root_blocks();
        assert_eq!(state.text_of(&roots[0]).as_deref(), Some("one"));
        assert_eq!(state.text_of(&roots[1]).as_deref(), Some("two"));
    }

    /// `PROSE-UI-112`: a paste in the middle of a paragraph splits it around
    /// the pasted blocks and leaves its neighbours alone.
    #[test]
    fn a_paste_mid_paragraph_leaves_the_neighbours_alone() {
        let (mut state, _) = document();
        let before = append(&mut state, BlockInit::Paragraph, "before");
        let middle = append(&mut state, BlockInit::Paragraph, "headtail");
        let after = append(&mut state, BlockInit::Paragraph, "after");
        let outcome = state.handle_command(ProseCommand::PasteText {
            at: (middle, 4),
            text: "A\nB".to_owned(),
        });
        assert!(outcome.update.is_some());
        let roots = state.root_blocks();
        let at = roots
            .iter()
            .position(|b| *b == middle)
            .expect("the paragraph stays");
        assert_eq!(state.text_of(&before).as_deref(), Some("before"));
        assert_eq!(state.text_of(&roots[at]).as_deref(), Some("headA"));
        assert_eq!(state.text_of(&roots[at + 1]).as_deref(), Some("Btail"));
        assert_eq!(roots[at + 2], after);
        assert_eq!(state.text_of(&after).as_deref(), Some("after"));
    }
}
