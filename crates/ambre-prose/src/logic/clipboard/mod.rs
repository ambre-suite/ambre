//! Copy and paste, where the document lives
//! (`features/ambre-spec-prose-clipboard.md`).
//!
//! A copy walks the whole selection in the document (`PROSE-UI-040`). It
//! answers with the plain text every application reads (`PROSE-UI-108`), and
//! with this application's own bytes, which keep the marks and the blocks.
//! A rich paste writes those bytes back in one commit, so one undo takes it
//! back (`PROSE-UI-107`). A copy also writes HTML, and a paste reads the HTML
//! another editor wrote, both through the blocks this module defines.

mod html_read;
mod html_write;
mod sink;

use loro::{LoroValue, TreeID, TreeParentId};
use serde::{Deserialize, Serialize};

use ambre_bus::app::prose::{
    BlockInit, CodeLang, DocCount, DocOffset, DocSpan, HeadingLevel, ListStyle, OrderedStyle,
};

use super::ProseState;
use super::text::{MarkedRun, marked_runs, write_runs};
use crate::doc::{self, StoredLevel};
use crate::events::{BlockId, ProseEvent, ProseOutcome, block_id, tree_id};
use crate::mark::{self, FORMATTING_KEYS, MarkRoot};
use ambre_bus::app::prose::{BlockType, MarkKey};

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
    Heading(HeadingLevel),
    Bullet,
    Ordered(OrderedStyle),
    Todo(crate::events::TodoMark),
    /// A code block, `None` for a plain one. The payload spells it as the
    /// text the block stores, the empty text for a plain block.
    Code(#[serde(with = "stored_lang")] Option<CodeLang>),
}

/// The code language as the payload spells it: the stored text, so the bytes
/// stay those a payload of a bare string wrote.
mod stored_lang {
    use ambre_bus::app::prose::CodeLang;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    #[expect(
        clippy::ref_option,
        reason = "serde's `with` hands the field by reference"
    )]
    pub(super) fn serialize<S: Serializer>(
        lang: &Option<CodeLang>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        CodeLang::stored(lang.as_ref()).serialize(serializer)
    }

    pub(super) fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Option<CodeLang>, D::Error> {
        String::deserialize(deserializer).map(CodeLang::new)
    }
}

/// A run of text and the formatting marks it carries. Anchors stay behind:
/// a comment or a suggestion does not travel with its text.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct ClipRun {
    text: String,
    marks: Vec<(MarkKey, ClipValue)>,
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

    /// The value a paste writes under `key`, or `None` for a mark it drops
    /// (`PROSE-UI-234`). Only a key that is a formatting root exactly passes,
    /// as a copy writes it. Its value must pass [`pasted`], the check an HTML
    /// paste makes too. A flag set by a boolean is stored as the one spelling
    /// of "present" (`PROSE-008`), and a flag cleared by one is dropped. The
    /// bytes a paste reads may come from a crafted page, so the copy that
    /// wrote them proves nothing.
    fn checked(&self, key: &MarkKey) -> Option<LoroValue> {
        let root = MarkRoot::parse(key.as_str()).filter(|root| FORMATTING_KEYS.contains(root))?;
        let raw = match self {
            Self::Bool(true) => None,
            Self::Text(raw) => Some(raw.as_str()),
            Self::Bool(false) | Self::Int(_) | Self::Float(_) => return None,
        };
        pasted(root, raw).map(|value| LoroValue::from(value.as_str()))
    }
}

/// The longest link and family name a paste keeps, in bytes.
const HREF_MAX: usize = 2048;
const FAMILY_MAX: usize = 64;

/// The value a paste writes under `root`, from either half of the clipboard
/// (`PROSE-UI-220`, `PROSE-UI-234`). The value passes the grammar of a typed
/// mark first. A paste then adds two limits: a link a reader follows, and a
/// family name of bounded length and plain characters.
fn pasted(root: MarkRoot, raw: Option<&str>) -> Option<mark::MarkValue> {
    let value = mark::validate_value(&root.key(), raw).ok()?;
    let kept = match root {
        MarkRoot::Link => safe_link(value.as_str()),
        MarkRoot::FontFamily => safe_family(value.as_str()),
        _ => true,
    };
    kept.then_some(value)
}

/// A link a paste keeps: one a reader follows to a page or a mail, never one
/// that runs.
fn safe_link(href: &str) -> bool {
    let lower = href.get(..8).unwrap_or(href).to_ascii_lowercase();
    href.len() <= HREF_MAX
        && ["http://", "https://", "mailto:"]
            .iter()
            .any(|scheme| lower.starts_with(scheme))
}

/// A family name a paste keeps: not empty, within [`FAMILY_MAX`], and free of
/// a control character or one that closes a style declaration.
fn safe_family(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= FAMILY_MAX
        && name
            .chars()
            .all(|c| !c.is_control() && !matches!(c, '<' | '>' | ';' | '"'))
}

impl ClipKind {
    fn init(&self) -> BlockInit {
        match self {
            Self::Paragraph => BlockInit::Paragraph,
            Self::Heading(level) => BlockInit::Heading { level: *level },
            Self::Bullet => BlockInit::ListItem {
                style: ListStyle::Bullet,
            },
            Self::Ordered(style) => BlockInit::ListItem {
                style: ListStyle::Ordered(*style),
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
            .filter(|(key, _)| {
                MarkRoot::parse(key.as_str()).is_some_and(|root| FORMATTING_KEYS.contains(&root))
            })
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
                .filter_map(|(key, value)| Some((key.clone(), value.checked(key)?)))
                .collect(),
        }
    }
}

fn marked(block: &ClipBlock) -> Vec<MarkedRun> {
    block.runs.iter().map(ClipRun::marked).collect()
}

fn len_of(block: &ClipBlock) -> DocCount {
    block
        .runs
        .iter()
        .fold(DocCount::ZERO, |sum, run| sum + DocCount::of(&run.text))
}

impl ProseState {
    /// The kind a block pastes back as.
    fn clip_kind(&self, id: &TreeID) -> ClipKind {
        let tree = doc::blocks(&self.doc);
        match doc::block_type_of(&tree, id) {
            Some(BlockType::Heading) => ClipKind::Heading(
                doc::heading_level_of(&tree, id).map_or(HeadingLevel::H1, StoredLevel::clamped),
            ),
            Some(BlockType::ListItem) => match doc::list_style_of(&tree, id) {
                Some(ListStyle::Ordered(style)) => ClipKind::Ordered(style),
                Some(ListStyle::Todo(done)) => ClipKind::Todo(done),
                _ => ClipKind::Bullet,
            },
            Some(BlockType::CodeBlock) => {
                ClipKind::Code(self.code_lang_of(&block_id(*id)).and_then(CodeLang::new))
            }
            _ => ClipKind::Paragraph,
        }
    }

    /// The payload of a whole selection, whatever the view shows of it
    /// (`PROSE-UI-040`). It writes nothing (`PROSE-UI-109`).
    pub(super) fn copy_range(
        &self,
        anchor: (BlockId, DocOffset),
        focus: (BlockId, DocOffset),
    ) -> ProseOutcome {
        let mut blocks = Vec::new();
        let mut text = String::new();
        for (block, span) in self.selected_spans(anchor, focus) {
            let id = tree_id(block);
            let Some(body) = self.body_of(&id) else {
                continue;
            };
            let runs: Vec<ClipRun> = marked_runs(&body, span)
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
        let html = html_write::write(&blocks);
        let payload = Payload {
            magic: MAGIC,
            blocks,
        };
        match postcard::to_allocvec(&payload) {
            Ok(payload) => outcome.display.push(ProseEvent::Clipboard {
                text,
                html,
                payload: ambre_core::ClipboardBytes::new(payload),
            }),
            Err(e) => tracing::warn!("Prose: a copy could not be encoded: {e}"),
        }
        outcome
    }

    /// Write plain text of several lines at `at`, one paragraph per line,
    /// the way a rich payload of paragraphs lands (`PROSE-UI-042`).
    fn paste_text(&mut self, at: (TreeID, DocOffset), text: &str) -> ProseOutcome {
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
    ///
    /// The richest half the clipboard held wins: this application's bytes,
    /// then HTML, then the plain text. Bytes another application wrote do not
    /// decode, and HTML with no text in it reads as no block. Each then falls
    /// through to the next half.
    pub(super) fn paste(
        &mut self,
        at: (TreeID, DocOffset),
        text: &str,
        html: Option<&str>,
        bytes: Option<&ambre_core::ClipboardBytes>,
    ) -> ProseOutcome {
        if let Some(Ok(payload)) =
            bytes.map(|bytes| postcard::from_bytes::<Payload>(bytes.as_bytes()))
            && payload.magic == MAGIC
        {
            return self.paste_blocks(at, payload);
        }
        if let Some(html) = html {
            let blocks = html_read::read(html);
            if !blocks.is_empty() {
                return self.paste_blocks(
                    at,
                    Payload {
                        magic: MAGIC,
                        blocks,
                    },
                );
            }
        }
        self.paste_text(at, text)
    }

    fn paste_blocks(&mut self, at: (TreeID, DocOffset), payload: Payload) -> ProseOutcome {
        let Some(body) = self.body_of(&at.0) else {
            return Self::err_no_block(&at.0);
        };
        let pos = at.1;
        let len = DocCount::from_len(body.len_unicode()).as_end();
        if pos > len {
            return Self::err_bad_range();
        }
        let Some((first, rest)) = payload.blocks.split_first() else {
            return ProseOutcome::nothing();
        };
        if len == DocOffset::ZERO
            && let Err(refused) = self.adopt(&at.0, first)
        {
            return refused;
        }
        let Some((last, middle)) = rest.split_last() else {
            if let Err(e) = write_runs(&body, pos, &marked(first)) {
                return Self::err_write(e);
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
        let tail = marked_runs(&body, DocSpan::new(pos, len));
        if pos < len
            && let Err(e) = body.delete(pos.to_usize(), len.count_from(pos).to_usize())
        {
            return Self::err_write(e);
        }
        if let Err(e) = write_runs(&body, pos, &marked(first)) {
            return Self::err_write(e);
        }
        let mut previous = at.0;
        for block in middle.iter().chain([last]) {
            let init = block.kind.init();
            let id = match self.mint_placed(None, Some(previous), init.block_type()) {
                Ok(id) => id,
                Err(refused) => return refused,
            };
            if let Err(e) = self.write_init(&id, &init) {
                return Self::err_write(e);
            }
            let text = doc::get_block_text(&self.doc, &id);
            if let Err(e) = write_runs(&text, DocOffset::ZERO, &marked(block)) {
                return Self::err_write(e);
            }
            previous = id;
        }
        let text = doc::get_block_text(&self.doc, &previous);
        let offset = len_of(last).as_end();
        if let Err(e) = write_runs(&text, offset, &tail) {
            return Self::err_write(e);
        }
        self.commit_with(vec![ProseEvent::CaretPlaced {
            block: block_id(previous),
            offset,
        }])
    }

    /// Give an empty paragraph the type of the first pasted block.
    ///
    /// Word and Google Docs do the same: a heading pasted on an empty line
    /// lands as a heading. A typed target, or one whose parent refuses the
    /// type, keeps its own.
    fn adopt(&mut self, id: &TreeID, first: &ClipBlock) -> Result<(), ProseOutcome> {
        let tree = doc::blocks(&self.doc);
        let init = first.kind.init();
        let kind = init.block_type();
        if kind == BlockType::Paragraph
            || doc::block_type_of(&tree, id) != Some(BlockType::Paragraph)
        {
            return Ok(());
        }
        let parent = match tree.parent(*id) {
            Some(TreeParentId::Node(parent)) => Some(parent),
            Some(TreeParentId::Root) => None,
            _ => return Ok(()),
        };
        if !doc::nesting_allows(&tree, parent, kind) {
            return Ok(());
        }
        let Some(node) = doc::node_of(&tree, id) else {
            return Err(Self::err_no_block(id));
        };
        if let Err(e) = node.insert(doc::NodeKey::Type, i64::from(kind.code())) {
            return Err(Self::err_write(e));
        }
        self.write_init(id, &init).map_err(Self::err_write)
    }
}

#[cfg(test)]
#[cfg(not(target_arch = "wasm32"))]
mod tests {
    use super::*;
    use crate::events::{BlockInit, ProseCommand};
    use crate::view::tests::append;
    use ambre_bus::app::prose::OrderedStyle;
    use ambre_bus::app::prose::{DocOffset, DocSpan};

    fn copied(
        state: &mut ProseState,
        anchor: (BlockId, u32),
        focus: (BlockId, u32),
    ) -> (String, ambre_core::ClipboardBytes, bool) {
        let (anchor, focus) = (
            (anchor.0, DocOffset::new(anchor.1)),
            (focus.0, DocOffset::new(focus.1)),
        );
        let outcome = state.handle_command(ProseCommand::CopyRange { anchor, focus });
        let wrote = outcome.update.is_some();
        let Some(ProseEvent::Clipboard { text, payload, .. }) = outcome.display.into_iter().next()
        else {
            panic!("a copy answers with the clipboard");
        };
        (text, payload, wrote)
    }

    fn document() -> (ProseState, BlockId) {
        let state = ProseState::with_peer(ambre_core::PeerId(1)).expect("a document is built");
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
        let heading = append(
            &mut state,
            BlockInit::Heading {
                level: HeadingLevel::H2,
            },
            "Title",
        );
        let bold = append(&mut state, BlockInit::Paragraph, "bold text");
        state.handle_command(ProseCommand::ApplyMark {
            block_id: bold,
            range: DocSpan::from_usize_range(0..4),
            key: "b".into(),
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
        state.handle_command(ProseCommand::Paste {
            at: (target, DocOffset::ZERO),
            text,
            html: None,
            payload: Some(payload),
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

    /// A run of one character that carries one mark, as a crafted payload
    /// spells it.
    fn run(key: &str, value: ClipValue) -> ClipRun {
        ClipRun {
            text: "x".to_owned(),
            marks: vec![(MarkKey::from(key), value)],
        }
    }

    fn text(raw: &str) -> ClipValue {
        ClipValue::Text(raw.to_owned())
    }

    /// The marks a paste of `runs`, built by hand, writes into a new document.
    fn pasted_marks(runs: Vec<ClipRun>) -> Vec<(String, LoroValue)> {
        let (mut state, first) = document();
        let len = runs.len();
        let payload = Payload {
            magic: MAGIC,
            blocks: vec![ClipBlock {
                kind: ClipKind::Paragraph,
                runs,
            }],
        };
        let bytes = postcard::to_allocvec(&payload).expect("a payload encodes");
        state.handle_command(ProseCommand::Paste {
            at: (first, DocOffset::ZERO),
            text: "x".repeat(len),
            html: None,
            payload: Some(ambre_core::ClipboardBytes::new(bytes)),
        });
        assert_eq!(state.text_of(&first), Some("x".repeat(len)));
        let body = state.body_of(&tree_id(first)).expect("the paragraph");
        let span = DocSpan::new(
            DocOffset::ZERO,
            DocCount::from_len(body.len_unicode()).as_end(),
        );
        marked_runs(&body, span)
            .into_iter()
            .flat_map(|run| run.marks)
            .map(|(key, value)| (key.as_str().to_owned(), value))
            .collect()
    }

    /// `PROSE-UI-234`: a crafted payload writes only the formatting marks a
    /// typed mark may carry. A colour, a size or a flag value the grammar
    /// refuses is dropped, and so is an anchor and a value of no mark shape.
    #[test]
    fn a_crafted_payload_writes_only_the_marks_a_typed_mark_may() {
        let written = pasted_marks(vec![
            run("fg", text("red")),
            run("fs", text("9999")),
            run("b", text("yes")),
            run("cm:x", text("a-thread")),
            run("zz", text("")),
            run("fg", ClipValue::Int(5)),
            run("bg", text("#FFFF00")),
            run("i", text("")),
        ]);
        assert_eq!(
            written,
            vec![
                ("bg".to_owned(), LoroValue::from("#ffff00")),
                ("i".to_owned(), LoroValue::from("")),
            ]
        );
    }

    /// `PROSE-UI-234`, `PROSE-UI-220`: a pasted link passes the check an HTML
    /// paste makes. A target that runs, or one longer than a paste keeps,
    /// never lands, so no export writes it.
    #[test]
    fn a_crafted_link_lands_only_when_a_reader_may_follow_it() {
        let long = format!("https://example.org/{}", "a".repeat(HREF_MAX));
        let written = pasted_marks(vec![
            run("a", text("javascript:alert(1)")),
            run("a", text("data:text/html,<script>alert(1)</script>")),
            run("a", text(&long)),
            run("a", text("https://example.org/page")),
        ]);
        assert_eq!(
            written,
            vec![("a".to_owned(), LoroValue::from("https://example.org/page"))]
        );
    }

    /// `PROSE-UI-234`: a pasted family name is capped as an HTML paste caps
    /// it, so a crafted payload cannot write a name of any length.
    #[test]
    fn a_crafted_family_name_lands_only_within_the_cap() {
        let written = pasted_marks(vec![
            run("ff", text(&"A".repeat(100_000))),
            run("ff", text("Georgia")),
        ]);
        assert_eq!(written, vec![("ff".to_owned(), LoroValue::from("Georgia"))]);
    }

    /// `PROSE-UI-234`: a key lands only when it is a formatting root exactly,
    /// as a copy writes it. A root with a suffix is no key of the registry.
    #[test]
    fn a_crafted_key_lands_only_when_it_names_a_root_exactly() {
        let written = pasted_marks(vec![
            run("b:zz", text("")),
            run("fg:x", text("#ff0000")),
            run("s", text("")),
        ]);
        assert_eq!(written, vec![("s".to_owned(), LoroValue::from(""))]);
    }

    /// `PROSE-UI-234`, `PROSE-008`: a flag set by a boolean lands in the one
    /// spelling of "present", and a flag cleared by one does not land.
    #[test]
    fn a_crafted_flag_lands_as_present_or_not_at_all() {
        let written = pasted_marks(vec![
            run("u", ClipValue::Bool(false)),
            run("i", ClipValue::Bool(true)),
        ]);
        assert_eq!(written, vec![("i".to_owned(), LoroValue::from(""))]);
    }

    /// `PROSE-UI-107`: a typed ordinal style and code language keep the bytes
    /// a payload of a bare code and a bare string wrote.
    #[test]
    fn a_typed_kind_keeps_its_payload_bytes() {
        let bytes = |kind: &ClipKind| postcard::to_allocvec(kind).expect("a kind encodes");
        assert_eq!(bytes(&ClipKind::Ordered(OrderedStyle::LowerRoman)), [3, 3]);
        assert_eq!(bytes(&ClipKind::Code(None)), [5, 0]);
        assert_eq!(
            bytes(&ClipKind::Code(CodeLang::new("rs"))),
            [5, 2, b'r', b's']
        );
        assert!(postcard::from_bytes::<ClipKind>(&[3, 9]).is_err());
        assert_eq!(
            postcard::from_bytes::<ClipKind>(&[5, 0]).ok(),
            Some(ClipKind::Code(None))
        );
    }

    /// `PROSE-UI-042`: bytes this application did not write paste as their
    /// plain text, one paragraph per line.
    #[test]
    fn foreign_bytes_paste_as_plain_text() {
        let (mut state, first) = document();
        state.handle_command(ProseCommand::Paste {
            at: (first, DocOffset::ZERO),
            text: "one\ntwo".to_owned(),
            html: None,
            payload: Some(ambre_core::ClipboardBytes::new(b"not a payload".to_vec())),
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
        let outcome = state.handle_command(ProseCommand::Paste {
            at: (middle, DocOffset::new(4)),
            text: "A\nB".to_owned(),
            html: None,
            payload: None,
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

    fn clipboard(
        state: &mut ProseState,
        anchor: (BlockId, u32),
        focus: (BlockId, u32),
    ) -> ProseEvent {
        let (anchor, focus) = (
            (anchor.0, DocOffset::new(anchor.1)),
            (focus.0, DocOffset::new(focus.1)),
        );
        let outcome = state.handle_command(ProseCommand::CopyRange { anchor, focus });
        outcome.display.into_iter().next().expect("a copy answers")
    }

    /// The text, the type and the marks of each root block past `from`.
    fn shape(state: &ProseState, from: usize) -> Vec<(String, Option<BlockType>, String)> {
        state.root_blocks()[from..]
            .iter()
            .map(|block| {
                (
                    state.text_of(block).unwrap_or_default(),
                    state.block_type_of(block),
                    format!("{:?}", state.marks_of(block)),
                )
            })
            .collect()
    }

    /// `PROSE-UI-219`, `PROSE-UI-222`: the HTML half of a copy, pasted alone,
    /// lands as the rich half does: the same blocks, types and marks.
    #[test]
    fn the_html_half_alone_pastes_what_the_rich_half_does() {
        let (mut state, _) = document();
        let heading = append(
            &mut state,
            BlockInit::Heading {
                level: HeadingLevel::H2,
            },
            "Title",
        );
        let body = append(&mut state, BlockInit::Paragraph, "bold and red and linked");
        for (range, key, value) in [
            ((0, 4), "b", None),
            ((9, 12), "fg", Some("#ff0000")),
            ((17, 23), "a", Some("https://example.com/?a=1&b=2")),
        ] {
            state.handle_command(ProseCommand::ApplyMark {
                block_id: body,
                range: DocSpan::new(DocOffset::new(range.0), DocOffset::new(range.1)),
                key: key.into(),
                value: value.map(str::to_owned),
            });
        }
        append(
            &mut state,
            BlockInit::ListItem {
                style: ListStyle::Ordered(OrderedStyle::UpperRoman),
            },
            "item",
        );
        let code = append(
            &mut state,
            BlockInit::CodeBlock {
                lang: ambre_bus::app::prose::CodeLang::new("rust"),
            },
            "a < b && c",
        );
        let ProseEvent::Clipboard {
            text,
            html,
            payload,
        } = clipboard(&mut state, (heading, 0), (code, 10))
        else {
            panic!("a copy answers with the clipboard");
        };
        let count = state.root_blocks().len();
        let rich = append(&mut state, BlockInit::Paragraph, "");
        state.handle_command(ProseCommand::Paste {
            at: (rich, DocOffset::ZERO),
            text: text.clone(),
            html: None,
            payload: Some(payload),
        });
        let middle = state.root_blocks().len();
        let from_html = append(&mut state, BlockInit::Paragraph, "");
        state.handle_command(ProseCommand::Paste {
            at: (from_html, DocOffset::ZERO),
            text,
            html: Some(html),
            payload: None,
        });
        let original = shape(&state, count - 4);
        assert_eq!(shape(&state, count)[..4], original[..4]);
        assert_eq!(shape(&state, middle)[..4], original[..4]);
    }

    /// `PROSE-UI-042`: the richest half wins. This application's bytes beat
    /// the HTML, the HTML beats the text, and HTML that reads as no block
    /// leaves the text to land.
    #[test]
    fn the_richest_half_the_clipboard_holds_wins() {
        let (mut state, first) = document();
        state.handle_command(ProseCommand::InsertText {
            block_id: first,
            pos: DocOffset::ZERO,
            text: "own".to_owned(),
        });
        let ProseEvent::Clipboard { payload, .. } = clipboard(&mut state, (first, 0), (first, 3))
        else {
            panic!("a copy answers with the clipboard");
        };
        let paste = |state: &mut ProseState,
                     html: Option<&str>,
                     payload: Option<ambre_core::ClipboardBytes>| {
            let target = append(state, BlockInit::Paragraph, "");
            state.handle_command(ProseCommand::Paste {
                at: (target, DocOffset::ZERO),
                text: "plain".to_owned(),
                html: html.map(str::to_owned),
                payload,
            });
            state.text_of(&target).unwrap_or_default()
        };
        assert_eq!(paste(&mut state, Some("<p>html</p>"), Some(payload)), "own");
        assert_eq!(
            paste(
                &mut state,
                Some("<p>html</p>"),
                Some(ambre_core::ClipboardBytes::new(b"foreign".to_vec()))
            ),
            "html"
        );
        assert_eq!(paste(&mut state, Some("<script>x</script>"), None), "plain");
        let deep = format!("{}x", "<div>".repeat(10_000));
        assert_eq!(paste(&mut state, Some(&deep), None), "plain");
    }

    /// One root block as golden text: its type and attributes on one line.
    /// Each run follows on its own line, its text escaped to ASCII and its
    /// marks sorted.
    fn golden_block(state: &ProseState, block: &BlockId) -> String {
        let mut kind = match state.block_type_of(block) {
            Some(kind) => format!("{kind:?}"),
            None => "?".to_owned(),
        };
        if let Some(level) = state.heading_level_of(block) {
            kind.push_str(&format!(" level={level}"));
        }
        if let Some(style) = state.list_style_of(block) {
            kind.push_str(&format!(" list={style:?}"));
        }
        if let Some(lang) = state.code_lang_of(block).filter(|lang| !lang.is_empty()) {
            kind.push_str(&format!(" lang={lang}"));
        }
        let mut out = kind;
        let Some(LoroValue::List(runs)) = state.marks_of(block) else {
            return out;
        };
        for run in runs.iter() {
            let LoroValue::Map(fields) = run else {
                continue;
            };
            let text = match fields.get("insert") {
                Some(LoroValue::String(text)) => text.to_string(),
                _ => String::new(),
            };
            out.push_str(&format!("\n  \"{}\"", text.escape_default()));
            if let Some(LoroValue::Map(marks)) = fields.get("attributes") {
                let mut marks: Vec<String> = marks
                    .iter()
                    .map(|(key, value)| match value {
                        LoroValue::String(value) if value.is_empty() => key.to_string(),
                        LoroValue::String(value) => {
                            format!("{key}={}", value.escape_default())
                        }
                        other => format!("{key}={other:?}"),
                    })
                    .collect();
                marks.sort();
                for mark in marks {
                    out.push(' ');
                    out.push_str(&mark);
                }
            }
        }
        out
    }

    /// `PROSE-UI-219`, `PROSE-UI-224`: each HTML fixture lands as the golden
    /// file beside it says. The paste command writes it into an empty
    /// document, and every block, attribute, run and mark value is compared.
    ///
    /// The golden is text a reviewer reads, not a document: a failure names
    /// the line that moved. `AMBRE_BLESS=1` rewrites the goldens from what the
    /// paste produced, and the diff is then the review.
    #[test]
    fn each_html_fixture_pastes_as_its_golden_says() {
        let dir =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/logic/clipboard/fixtures");
        let bless = std::env::var_os("AMBRE_BLESS").is_some();
        let mut failures = Vec::new();
        for name in ["word", "google-docs", "web-page", "pdfjs"] {
            let html = std::fs::read_to_string(dir.join(format!("{name}.html")))
                .expect("the fixture is committed");
            let (mut state, first) = document();
            state.handle_command(ProseCommand::Paste {
                at: (first, DocOffset::ZERO),
                text: String::new(),
                html: Some(html),
                payload: None,
            });
            let mut got: String = state
                .root_blocks()
                .iter()
                .map(|block| golden_block(&state, block))
                .collect::<Vec<_>>()
                .join("\n");
            got.push('\n');
            let path = dir.join(format!("{name}.expected"));
            if bless {
                std::fs::write(&path, &got).expect("the golden is written");
                continue;
            }
            let want = std::fs::read_to_string(&path).unwrap_or_default();
            if want != got {
                failures.push(format!("== {name}.expected\n-- want\n{want}-- got\n{got}"));
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }
}
