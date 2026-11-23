//! Copy and paste, where the document lives
//! (`features/ambre-spec-prose-clipboard.md`).
//!
//! A copy walks the whole selection in the document (`PROSE-UI-040`). It
//! answers with the plain text every application reads (`PROSE-UI-108`), and
//! with this application's own bytes, which keep the marks and the blocks.
//! A rich paste writes those bytes back in one commit, so one undo takes it
//! back (`PROSE-UI-107`). A copy also writes HTML, and a paste reads the HTML
//! another editor wrote, both through the blocks this module defines.

#[cfg(test)]
#[cfg(not(target_arch = "wasm32"))]
mod html_paste;
mod html_write;
mod table_paste;

use ambre_core::index::{Len, Pos, Span};
use loro::{LoroValue, TreeID, TreeParentId};
use serde::{Deserialize, Serialize};

use crate::message::{BlockInit, DocChar, HeadingLevel, ListStyle};
use ambre_prose_form::clip::{
    ClipBlock, ClipItem, ClipKind, ClipRun, ClipValue, safe_family, safe_link,
};

use super::ProseState;
use super::text::{MarkedRun, marked_runs, write_runs};
use crate::doc::{self, StoredLevel};
use crate::events::{BlockId, ProseEvent, ProseOutcome, block_id, tree_id};
use crate::mark::{self, FORMATTING_KEYS, MarkRoot};
use crate::message::{BlockType, MarkKey};

/// What opens the bytes, so a payload another Ambre application wrote under
/// the same clipboard type never decodes as this one.
const MAGIC: ambre_core::FixedBytes<ambre_core::FormatMagic> =
    ambre_core::FixedBytes::from_array(*b"APR1");

/// What a copy carries in this application's own bytes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct Payload {
    magic: ambre_core::FixedBytes<ambre_core::FormatMagic>,
    blocks: Vec<ClipBlock>,
}

/// The payload this application wrote, when `bytes` decode as one.
fn own_payload(bytes: Option<&ambre_core::Payload<ambre_core::ClipboardBytes>>) -> Option<Payload> {
    bytes
        .and_then(|bytes| postcard::from_bytes::<Payload>(bytes.as_bytes()).ok())
        .filter(|payload| payload.magic == MAGIC)
}

/// Whether `bytes` hold a payload this application wrote: a paste of them
/// reads no HTML, so it waits on no converter.
pub(super) fn is_own(bytes: Option<&ambre_core::Payload<ambre_core::ClipboardBytes>>) -> bool {
    own_payload(bytes).is_some()
}

fn clip_value(value: &LoroValue) -> Option<ClipValue> {
    Some(match value {
        LoroValue::Bool(on) => ClipValue::Bool(*on),
        LoroValue::I64(n) => ClipValue::Int(*n),
        LoroValue::Double(x) => ClipValue::Float(*x),
        LoroValue::String(text) => ClipValue::Text(text.to_string()),
        _ => return None,
    })
}

/// The value a paste writes under `key`, or `None` for a mark it drops
/// (`PROSE-UI-234`). Only a key that is a formatting root exactly passes,
/// as a copy writes it. Its value must pass [`pasted`], the check the markup
/// module makes on an HTML paste too. A flag set by a boolean is stored as
/// the one spelling of "present" (`PROSE-008`), and a flag cleared by one is
/// dropped. The bytes a paste reads may come from a crafted page, so the copy
/// or the module that wrote them proves nothing.
fn checked(value: &ClipValue, key: &MarkKey) -> Option<LoroValue> {
    let root = MarkRoot::exactly(key).filter(|root| FORMATTING_KEYS.contains(root))?;
    let raw = match value {
        ClipValue::Bool(true) => None,
        ClipValue::Text(raw) => Some(raw.as_str()),
        ClipValue::Bool(false) | ClipValue::Int(_) | ClipValue::Float(_) => return None,
    };
    pasted(root, raw).map(|value| LoroValue::from(value.as_str()))
}

/// The value a paste writes under `root`, from either half of the clipboard
/// (`PROSE-UI-220`, `PROSE-UI-234`). The value passes the grammar of a typed
/// mark first. A paste then adds two limits: a link a reader follows, and a
/// family name of bounded length and plain characters. A size past the cap
/// lands at the cap, as a stored one reads (`CANVAS-088`).
pub(in crate::logic) fn pasted(root: MarkRoot, raw: Option<&str>) -> Option<mark::MarkValue> {
    if matches!(root, MarkRoot::FontSize) {
        let size = mark::read_font_size(raw?)?;
        let points = mark::canonical_number(size);
        return mark::validate_value(&root.key(), Some(&points)).ok();
    }
    let value = mark::validate_value(&root.key(), raw).ok()?;
    let kept = match root {
        MarkRoot::Link => safe_link(value.as_str()),
        MarkRoot::FontFamily => safe_family(value.as_str()),
        _ => true,
    };
    kept.then_some(value)
}

/// The block a pasted kind writes.
fn init(kind: &ClipKind) -> BlockInit {
    match kind {
        ClipKind::Paragraph => BlockInit::Paragraph,
        ClipKind::Heading(level) => BlockInit::Heading { level: *level },
        ClipKind::Bullet => BlockInit::ListItem {
            style: ListStyle::Bullet,
        },
        ClipKind::Ordered(style) => BlockInit::ListItem {
            style: ListStyle::Ordered(*style),
        },
        ClipKind::Todo(done) => BlockInit::ListItem {
            style: ListStyle::Todo(*done),
        },
        ClipKind::Code(lang) => BlockInit::CodeBlock { lang: lang.clone() },
    }
}

/// The run a copy writes for one marked run of the document.
fn clip_run(run: MarkedRun) -> ClipRun {
    let marks = run
        .marks
        .iter()
        .filter(|(key, _)| {
            MarkRoot::exactly(key).is_some_and(|root| FORMATTING_KEYS.contains(&root))
        })
        .filter_map(|(key, value)| Some((key.clone(), clip_value(value)?)))
        .collect();
    ClipRun {
        text: run.text,
        marks,
    }
}

/// The runs of `block` a paste writes, each mark checked again.
fn marked(block: &ClipBlock) -> Vec<MarkedRun> {
    block
        .runs
        .iter()
        .map(|run| MarkedRun {
            text: run.text.clone(),
            marks: run
                .marks
                .iter()
                .filter_map(|(key, value)| Some((key.clone(), checked(value, key)?)))
                .collect(),
        })
        .collect()
}

fn len_of(block: &ClipBlock) -> Len<DocChar> {
    block.runs.iter().fold(Len::<DocChar>::ZERO, |sum, run| {
        sum + ambre_core::char_len(&run.text).as_axis::<DocChar>()
    })
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
            Some(BlockType::CodeBlock) => ClipKind::Code(self.code_lang_of(&block_id(*id))),
            _ => ClipKind::Paragraph,
        }
    }

    /// The payload of a whole selection, whatever the view shows of it
    /// (`PROSE-UI-040`). It writes nothing (`PROSE-UI-109`).
    pub(super) fn copy_range(
        &self,
        anchor: (BlockId, Pos<DocChar>),
        focus: (BlockId, Pos<DocChar>),
    ) -> ProseOutcome {
        let mut blocks = Vec::new();
        let mut text = String::new();
        for (block, span) in self.selected_spans(anchor, focus) {
            let id = tree_id(block);
            let Some(body) = self.body_of(&id) else {
                continue;
            };
            let runs: Vec<ClipRun> = marked_runs(&body, span).into_iter().map(clip_run).collect();
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
                payload: ambre_core::Payload::<ambre_core::ClipboardBytes>::new(payload),
            }),
            Err(e) => tracing::warn!("Prose: a copy could not be encoded: {e}"),
        }
        outcome
    }

    /// Write plain text of several lines at `at`, one paragraph per line,
    /// the way a rich payload of paragraphs lands (`PROSE-UI-042`).
    fn paste_text(&mut self, at: (TreeID, Pos<DocChar>), text: &str) -> ProseOutcome {
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
    /// through to the next half. The markup module reads the HTML half into
    /// `html`: the blocks a copy writes, and the tables between them. Each
    /// of their marks is checked here again. A paste whose module could not
    /// load holds no item, and falls through to the text.
    ///
    /// A table lands as a table where the caret's block may sit beside one
    /// (`PROSE-UI-225`). Inside a cell, a list, a band or a note it lands as
    /// one block or more per cell, as it read before tables landed.
    pub(super) fn paste(
        &mut self,
        at: (TreeID, Pos<DocChar>),
        text: &str,
        html: Vec<ClipItem>,
        bytes: Option<&ambre_core::Payload<ambre_core::ClipboardBytes>>,
    ) -> ProseOutcome {
        if let Some(payload) = own_payload(bytes) {
            return self.paste_blocks(at, payload);
        }
        // A table lands beside the caret's block when its parent takes one,
        // and the block sits in no band or note.
        let tree = doc::blocks(&self.doc);
        let parent = match tree.parent(at.0) {
            Some(TreeParentId::Node(parent)) => Some(parent),
            _ => None,
        };
        let tables = html.iter().any(|item| matches!(item, ClipItem::Table(_)))
            && !doc::is_aside(&tree, &at.0)
            && doc::nesting_allows(&tree, parent, BlockType::Table);
        if tables {
            return self.paste_items(at, html);
        }
        if !html.is_empty() {
            return self.paste_blocks(
                at,
                Payload {
                    magic: MAGIC,
                    blocks: html.into_iter().flat_map(ClipItem::flattened).collect(),
                },
            );
        }
        self.paste_text(at, text)
    }

    /// Take the selection from `anchor` to `focus` out, then paste where it
    /// collapsed, as one undo item (`CHROME-184`). The ends arrive in either
    /// order.
    ///
    /// A paste that writes nothing takes the deletion back, so the selection
    /// text stands as it did, and the history keeps no step of it.
    pub(super) fn paste_over(
        &mut self,
        anchor: (TreeID, Pos<DocChar>),
        focus: (TreeID, Pos<DocChar>),
        text: &str,
        html: Vec<ClipItem>,
        bytes: Option<&ambre_core::Payload<ambre_core::ClipboardBytes>>,
    ) -> ProseOutcome {
        // The deletion and the paste each commit, and the hold makes the two
        // commits one step. The steps before it stay aside, so a refusal
        // gives both sides of the history back (`UNDO-021`).
        self.history.hold();
        let outcome = match self.take_range(anchor, focus) {
            Ok(at) => {
                self.doc.commit();
                self.paste(at, text, html, bytes)
            }
            Err(refused) => refused,
        };
        if outcome.update.is_some() {
            self.history.release();
            return outcome;
        }
        // A write left pending joins the held step, so the inverse takes it
        // back with the deletion.
        self.doc.commit();
        let _ = self.history.withdraw(&self.doc);
        self.doc
            .set_next_commit_origin(ambre_core::crdt::CommitOrigin::Recorded.as_str());
        self.doc.commit();
        self.history.restore();
        // The deletion and its inverse are operations of the log all the
        // same: they leave as a delta, and the view hears of the text.
        let mut flushed = self.commit_with(Vec::new());
        flushed.display.extend(outcome.display);
        flushed
    }

    fn paste_blocks(&mut self, at: (TreeID, Pos<DocChar>), payload: Payload) -> ProseOutcome {
        let Some(body) = self.body_of(&at.0) else {
            return Self::err_no_block(&at.0);
        };
        let pos = at.1;
        let len = Len::<DocChar>::of(body.len_unicode()).as_end();
        if pos > len {
            return Self::err_bad_range();
        }
        let Some((first, rest)) = payload.blocks.split_first() else {
            return ProseOutcome::nothing();
        };
        if len == Pos::<DocChar>::ZERO
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
        let tail = marked_runs(&body, Span::<DocChar>::new(pos, len));
        if pos < len
            && let Err(e) = body.delete(pos.to_usize(), len.len_from(pos).to_usize())
        {
            return Self::err_write(e);
        }
        if let Err(e) = write_runs(&body, pos, &marked(first)) {
            return Self::err_write(e);
        }
        let mut previous = at.0;
        for block in middle.iter().chain([last]) {
            let init = init(&block.kind);
            let id = match self.mint_placed(None, Some(previous), init.block_type()) {
                Ok(id) => id,
                Err(refused) => return refused,
            };
            if let Err(e) = self.write_init(&id, &init) {
                return Self::err_write(e);
            }
            let text = doc::get_block_text(&self.doc, &id);
            if let Err(e) = write_runs(&text, Pos::<DocChar>::ZERO, &marked(block)) {
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
        let init = init(&first.kind);
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
    use crate::link::HREF_MAX;
    use crate::message::DocChar;
    use crate::message::{CodeLang, OrderedStyle};
    use crate::view::tests::append;
    use ambre_core::Validated;
    use ambre_prose_form::clip::FAMILY_MAX;

    #[test]
    fn a_copied_run_prints_its_text_by_length_alone() {
        let run = ClipRun {
            text: "copied".into(),
            marks: vec![(MarkKey::from("ff"), ClipValue::Text("Family".into()))],
        };
        assert!(crate::redact::tests::hides(&run, "copied"));
        assert!(crate::redact::tests::hides(&run, "Family"));
    }

    fn copied(
        state: &mut ProseState,
        anchor: (BlockId, u32),
        focus: (BlockId, u32),
    ) -> (
        String,
        ambre_core::Payload<ambre_core::ClipboardBytes>,
        bool,
    ) {
        let (anchor, focus) = (
            (anchor.0, Pos::<DocChar>::new(anchor.1)),
            (focus.0, Pos::<DocChar>::new(focus.1)),
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
        let state = ProseState::with_peer(ambre_core::PeerId::new(1)).expect("a document is built");
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
            range: Span::<DocChar>::from_usize_range(0..4),
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
        state.handle_converted(ProseCommand::Paste {
            at: (target, Pos::<DocChar>::ZERO),
            over: None,
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
        state.handle_converted(ProseCommand::Paste {
            at: (first, Pos::<DocChar>::ZERO),
            over: None,
            text: "x".repeat(len),
            html: None,
            payload: Some(ambre_core::Payload::<ambre_core::ClipboardBytes>::new(
                bytes,
            )),
        });
        assert_eq!(state.text_of(&first), Some("x".repeat(len)));
        let body = state.body_of(&tree_id(first)).expect("the paragraph");
        let span = Span::<DocChar>::new(
            Pos::<DocChar>::ZERO,
            Len::<DocChar>::of(body.len_unicode()).as_end(),
        );
        marked_runs(&body, span)
            .into_iter()
            .flat_map(|run| run.marks)
            .map(|(key, value)| (key.as_str().to_owned(), value))
            .collect()
    }

    /// A copy keeps the bytes every earlier build wrote: the magic `APR1`,
    /// then the blocks. A payload an earlier build put on the clipboard
    /// pastes, and one this build writes reads the same there.
    #[test]
    fn a_payload_keeps_its_bytes() {
        let payload = Payload {
            magic: MAGIC,
            blocks: vec![ClipBlock {
                kind: ClipKind::Paragraph,
                runs: vec![run("b", text(""))],
            }],
        };
        let earlier = [
            0x41, 0x50, 0x52, 0x31, 0x01, 0x00, 0x01, 0x01, 0x78, 0x01, 0x01, 0x62, 0x03, 0x00,
        ];
        let bytes = postcard::to_allocvec(&payload).expect("a payload encodes");
        assert_eq!(bytes, earlier);
        assert_eq!(
            postcard::from_bytes::<Payload>(&earlier).ok(),
            Some(payload)
        );
    }

    /// `PROSE-UI-234`: a crafted payload writes only the formatting marks a
    /// typed mark may carry. A colour or a flag value the grammar refuses is
    /// dropped, and so is an anchor and a value of no mark shape. A size past
    /// the cap lands at the cap (`CANVAS-088`).
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
                ("fs".to_owned(), LoroValue::from("1638")),
                ("bg".to_owned(), LoroValue::from("#ffff00")),
                ("i".to_owned(), LoroValue::from("")),
            ]
        );
    }

    /// `PROSE-UI-234`: a pasted link passes the check an HTML paste makes. A target that runs, or one longer than a paste keeps,
    /// never lands, so no export writes it.
    #[test]
    fn a_crafted_link_lands_only_when_a_reader_may_follow_it() {
        let long = format!(
            "https://example.org/{}",
            "a".repeat(HREF_MAX.saturating_len().to_usize())
        );
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

    /// `PROSE-UI-234`: a flag set by a boolean lands in the one spelling of
    /// "present", and a flag cleared by one does not land.
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
            bytes(&ClipKind::Code(Validated::<CodeLang>::parse("rs").ok())),
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
        state.handle_converted(ProseCommand::Paste {
            at: (first, Pos::<DocChar>::ZERO),
            over: None,
            text: "one\ntwo".to_owned(),
            html: None,
            payload: Some(ambre_core::Payload::<ambre_core::ClipboardBytes>::new(
                b"not a payload".to_vec(),
            )),
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
        let outcome = state.handle_converted(ProseCommand::Paste {
            at: (middle, Pos::<DocChar>::new(4)),
            over: None,
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
            (anchor.0, Pos::<DocChar>::new(anchor.1)),
            (focus.0, Pos::<DocChar>::new(focus.1)),
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
                range: Span::<DocChar>::new(
                    Pos::<DocChar>::new(range.0),
                    Pos::<DocChar>::new(range.1),
                ),
                key: key.into(),
                value: value.map(crate::mark::test_value),
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
                lang: ambre_core::Validated::<crate::message::CodeLang>::parse("rust").ok(),
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
        state.handle_converted(ProseCommand::Paste {
            at: (rich, Pos::<DocChar>::ZERO),
            over: None,
            text: text.clone(),
            html: None,
            payload: Some(payload),
        });
        let middle = state.root_blocks().len();
        let from_html = append(&mut state, BlockInit::Paragraph, "");
        state.handle_converted(ProseCommand::Paste {
            at: (from_html, Pos::<DocChar>::ZERO),
            over: None,
            text,
            html: Some(html),
            payload: None,
        });
        let original = shape(&state, count - 4);
        assert_eq!(shape(&state, count)[..4], original[..4]);
        assert_eq!(shape(&state, middle)[..4], original[..4]);
    }

    /// `PROSE-UI-235`: a peer writes mark values straight into the document,
    /// unchecked. The HTML half writes no target a paste would refuse, and no
    /// colour or size outside its canonical spelling; the text stays.
    #[test]
    fn a_peer_mark_value_reaches_the_html_half_only_when_safe() {
        let (mut state, _) = document();
        let block = append(&mut state, BlockInit::Paragraph, "link ink mark size");
        let body = crate::doc::get_block_text(&state.doc, &crate::events::tree_id(block));
        let long = format!(
            "https://example.org/{}",
            "a".repeat(HREF_MAX.saturating_len().to_usize())
        );
        let family = "A".repeat(FAMILY_MAX.saturating_len().to_usize() + 1);
        for (range, key, value) in [
            (0..4, "ff", family.as_str()),
            (0..4, "a", "javascript:alert(1)"),
            (5..8, "a", long.as_str()),
            (5..8, "fg", "red;background:url(https://evil/beacon)"),
            (9..13, "bg", "#00ff00\"><img src=x>"),
            (14..18, "fs", "12pt;background:url(https://evil/beacon)"),
        ] {
            body.mark(range, key, value).expect("the peer's mark lands");
        }
        state.doc.commit();
        let ProseEvent::Clipboard { text, html, .. } =
            clipboard(&mut state, (block, 0), (block, 18))
        else {
            panic!("a copy answers with the clipboard");
        };
        assert_eq!(text, "link ink mark size");
        assert!(html.contains("link ink mark size"), "{html}");
        for unsafe_part in ["href", "url(", "javascript", "<img", "style"] {
            assert!(!html.contains(unsafe_part), "{unsafe_part} in {html}");
        }
    }

    /// `PROSE-UI-235`: a valid peer value is written in its canonical
    /// spelling.
    #[test]
    fn a_peer_mark_value_is_written_canonical() {
        let (mut state, _) = document();
        let block = append(&mut state, BlockInit::Paragraph, "link ink");
        let body = crate::doc::get_block_text(&state.doc, &crate::events::tree_id(block));
        for (range, key, value) in [
            (0..4, "a", "HTTPS://example.org/"),
            (5..8, "fg", "#FF0000"),
            (5..8, "fs", "12.0"),
        ] {
            body.mark(range, key, value).expect("the peer's mark lands");
        }
        state.doc.commit();
        let ProseEvent::Clipboard { html, .. } = clipboard(&mut state, (block, 0), (block, 8))
        else {
            panic!("a copy answers with the clipboard");
        };
        assert!(
            html.contains("<a href=\"HTTPS://example.org/\">link</a>"),
            "{html}"
        );
        assert!(
            html.contains("<span style=\"color:#ff0000;font-size:12pt;\">ink</span>"),
            "{html}"
        );
    }

    /// `PROSE-UI-042`: the richest half wins. This application's bytes beat
    /// the HTML, the HTML beats the text, and HTML that reads as no block
    /// leaves the text to land.
    #[test]
    fn the_richest_half_the_clipboard_holds_wins() {
        let (mut state, first) = document();
        state.handle_command(ProseCommand::InsertText {
            block_id: first,
            pos: Pos::<DocChar>::ZERO,
            text: "own".to_owned(),
        });
        let ProseEvent::Clipboard { payload, .. } = clipboard(&mut state, (first, 0), (first, 3))
        else {
            panic!("a copy answers with the clipboard");
        };
        let paste =
            |state: &mut ProseState,
             html: Option<&str>,
             payload: Option<ambre_core::Payload<ambre_core::ClipboardBytes>>| {
                let target = append(state, BlockInit::Paragraph, "");
                state.handle_converted(ProseCommand::Paste {
                    at: (target, Pos::<DocChar>::ZERO),
                    over: None,
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
                Some(ambre_core::Payload::<ambre_core::ClipboardBytes>::new(
                    b"foreign".to_vec()
                ))
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
        if let Some(lang) = state.code_lang_of(block) {
            kind.push_str(&format!(" lang={}", lang.as_str()));
        }
        let mut out = kind;
        if state.block_type_of(block) == Some(BlockType::Table) {
            let table = tree_id(*block);
            let tree = doc::blocks(state.document());
            for row in state.grid_of(&table).expect("a table").cells {
                let cells: Vec<String> = row
                    .iter()
                    .map(|cell| {
                        doc::children_of(&tree, Some(*cell))
                            .iter()
                            .map(|child| {
                                let text = state.text_of(&block_id(*child)).unwrap_or_default();
                                format!("\"{}\"", text.escape_default())
                            })
                            .collect::<Vec<_>>()
                            .join(" ")
                    })
                    .collect();
                out.push_str(&format!("\n  | {} |", cells.join(" | ")));
            }
            for (_, rect) in state.merged_rects(&table) {
                out.push_str(&format!(
                    "\n  merged {},{} to {},{}",
                    rect.top_left.row.get(),
                    rect.top_left.col.get(),
                    rect.bottom_right.row.get(),
                    rect.bottom_right.col.get()
                ));
            }
            return out;
        }
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
            state.handle_converted(ProseCommand::Paste {
                at: (first, Pos::<DocChar>::ZERO),
                over: None,
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

    /// Paste `html` at `at`, the HTML half alone.
    fn paste_html(state: &mut ProseState, at: (BlockId, u32), html: &str) -> ProseOutcome {
        state.handle_converted(ProseCommand::Paste {
            at: (at.0, Pos::<DocChar>::new(at.1)),
            over: None,
            text: String::new(),
            html: Some(html.to_owned()),
            payload: None,
        })
    }

    /// Every root block, a table drawn as its rows, as the golden draws it.
    fn drawn(state: &ProseState) -> Vec<String> {
        state
            .root_blocks()
            .iter()
            .map(|block| golden_block(state, block))
            .collect()
    }

    const MERGED_TABLE: &str = concat!(
        "<table><tr><th colspan=\"2\">Region</th><th>Q1</th></tr>",
        "<tr><td rowspan=\"2\">North</td><td>a</td><td><b>12</b></td></tr>",
        "<tr><td>b</td><td><ul><li>one</li><li>two</li></ul></td></tr></table>",
    );

    /// `PROSE-UI-225`: a pasted table lands as a table of its grid, each
    /// cell with its blocks and marks, each merged cell as one region. A
    /// paragraph follows it, where the caret lands, and one undo takes the
    /// whole paste back.
    #[test]
    fn an_html_table_pastes_as_a_table_with_its_merged_cells() {
        let (mut state, first) = document();
        let outcome = paste_html(&mut state, (first, 0), MERGED_TABLE);
        assert_eq!(
            drawn(&state),
            [
                concat!(
                    "Table\n",
                    "  | \"Region\" | \"\" | \"Q1\" |\n",
                    "  | \"North\" | \"a\" | \"12\" |\n",
                    "  | \"\" | \"b\" | \"one\" \"two\" |\n",
                    "  merged 0,0 to 0,1\n",
                    "  merged 1,0 to 2,0",
                ),
                "Paragraph",
            ]
        );
        let roots = state.root_blocks();
        assert!(outcome.display.contains(&ProseEvent::CaretPlaced {
            block: roots[1],
            offset: Pos::<DocChar>::ZERO,
        }));
        let table = tree_id(roots[0]);
        let tree = doc::blocks(state.document());
        let grid = state.grid_of(&table).expect("a table");
        let cell = |row: usize, col: usize| doc::children_of(&tree, Some(grid.cells[row][col]));
        let bold = block_id(cell(1, 2)[0]);
        assert!(format!("{:?}", state.marks_of(&bold)).contains("\"b\""));
        let list = cell(2, 2);
        assert_eq!(
            state.block_type_of(&block_id(list[1])),
            Some(BlockType::ListItem)
        );
        state.handle_command(ProseCommand::Undo);
        assert_eq!(drawn(&state), ["Paragraph"]);
    }

    /// `PROSE-UI-225`: a table pasted inside a paragraph splits it. The text
    /// before the caret joins the first pasted block, and the text after it
    /// joins the last one.
    #[test]
    fn a_table_pasted_inside_a_paragraph_splits_it() {
        let (mut state, _) = document();
        let target = append(&mut state, BlockInit::Paragraph, "headtail");
        paste_html(
            &mut state,
            (target, 4),
            "<p>x</p><table><tr><td>1</td><td>2</td></tr></table><p>y</p>",
        );
        let drawn = drawn(&state);
        assert_eq!(
            drawn[drawn.len() - 3..],
            [
                "Paragraph\n  \"headx\"",
                "Table\n  | \"1\" | \"2\" |",
                "Paragraph\n  \"ytail\""
            ]
        );
    }

    /// `PROSE-UI-225`: a table pasted at the start of a paragraph lands
    /// before it, and the paragraph keeps its text.
    #[test]
    fn a_table_pasted_at_the_start_of_a_paragraph_lands_before_it() {
        let (mut state, _) = document();
        let target = append(&mut state, BlockInit::Paragraph, "kept");
        paste_html(
            &mut state,
            (target, 0),
            "<table><tr><td>1</td></tr></table>",
        );
        let drawn = drawn(&state);
        assert_eq!(
            drawn[drawn.len() - 2..],
            ["Table\n  | \"1\" |", "Paragraph\n  \"kept\""]
        );
    }

    /// `PROSE-UI-240`: a table pasted inside a table cell, where no table
    /// may sit, lands as one paragraph per cell.
    #[test]
    fn a_table_pasted_inside_a_cell_lands_as_its_cells() {
        let (mut state, table) =
            crate::logic::tables::tests::document_with_table(ambre_core::PeerId::new(1), 1, 1);
        let cell = crate::logic::tables::tests::cell_at(&state, table, 0, 0);
        let paragraph = crate::logic::tables::tests::first_block_of(&state, cell);
        paste_html(
            &mut state,
            (block_id(paragraph), 0),
            "<table><tr><td>a</td><td>b</td></tr></table>",
        );
        let tree = doc::blocks(state.document());
        let texts: Vec<String> = doc::children_of(&tree, Some(cell))
            .iter()
            .map(|child| state.text_of(&block_id(*child)).unwrap_or_default())
            .collect();
        assert_eq!(texts, ["a", "b"]);
        assert_eq!(
            state
                .root_blocks()
                .iter()
                .filter(|block| state.block_type_of(block) == Some(BlockType::Table))
                .count(),
            1
        );
    }

    /// `CHROME-184`: a paste over a selection that writes nothing
    /// takes the deletion back. The selected text stands, and both sides of
    /// the history stand as they did before the paste.
    #[test]
    fn a_paste_over_a_selection_that_writes_nothing_leaves_the_selection() {
        let (mut state, _) = document();
        let block = append(&mut state, BlockInit::Paragraph, "hello world");
        state.handle_command(ProseCommand::InsertText {
            block_id: block,
            pos: Pos::<DocChar>::new(11),
            text: "!".into(),
        });
        let _ = state.handle_command(ProseCommand::Undo);
        let empty = postcard::to_allocvec(&Payload {
            magic: MAGIC,
            blocks: Vec::new(),
        })
        .expect("an empty payload encodes");
        let outcome = state.handle_command(ProseCommand::Paste {
            at: (block, Pos::<DocChar>::ZERO),
            over: Some((block, Pos::<DocChar>::new(5))),
            text: String::new(),
            html: None,
            payload: Some(ambre_core::Payload::new(empty)),
        });
        assert_eq!(state.text_of(&block).as_deref(), Some("hello world"));
        assert!(
            outcome.update.is_some(),
            "the deletion and its inverse leave as a delta"
        );
        let _ = state.handle_command(ProseCommand::Redo);
        assert_eq!(
            state.text_of(&block).as_deref(),
            Some("hello world!"),
            "the redo side stands"
        );
        let _ = state.handle_command(ProseCommand::Undo);
        assert_eq!(
            state.text_of(&block).as_deref(),
            Some("hello world"),
            "the undo side stands"
        );
    }
}
