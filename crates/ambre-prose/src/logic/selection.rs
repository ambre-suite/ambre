//! The commands that write one value over every block a selection covers
//! (`features/ambre-spec-prose-toolbar.md section 3`). Each writes a value
//! mark, the paragraph properties, the kind of each paragraph, one step of
//! indent, or a default font.
//!
//! Each command writes every block it reaches, then commits once. One
//! gesture is therefore one undo item, whatever the number of blocks
//! (`CHROME-173`).

use loro::TreeID;
use loro::TreeParentId;

use crate::doc;
use crate::events::{BlockId, BlockType, ParagraphStyle, ProseOutcome, ProseRefusal, tree_id};
use crate::mark::{self, MarkRoot};
use crate::message::{DocChar, IndentStep, MarkKey, MarkValue, ParagraphKind};
use ambre_core::crdt::CrdtName as _;
use ambre_core::index::Pos;
use ambre_core::{PrintLength, Pt};

use super::ProseState;
use super::lists::{parent_of, previous_sibling};

/// One step of left indent, for a block that is no list item. Half an inch
/// is the step of the office formats (`PROSE-UI-267`).
const INDENT_STEP: PrintLength<Pt> = PrintLength::new(36.0);

/// Whether a gesture wrote a block so far. A gesture that wrote none exports
/// no delta and leaves no undo item (`PROSE-047`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Changed {
    Nothing,
    Written,
}

impl Changed {
    /// The fold of two steps: written once one of them wrote.
    fn or(self, other: Self) -> Self {
        if self == Self::Written || other == Self::Written {
            Self::Written
        } else {
            Self::Nothing
        }
    }
}

/// The keys a value mark of the context panel writes: a colour, a
/// highlight, a size, a font and a link. Each needs a value, which the
/// presence marks of the format bar do not.
const VALUE_KEYS: [MarkRoot; 5] = [
    MarkRoot::Ink,
    MarkRoot::Highlight,
    MarkRoot::FontSize,
    MarkRoot::FontFamily,
    MarkRoot::Link,
];

/// The block type a kind writes.
fn type_of(kind: ParagraphKind) -> BlockType {
    match kind {
        ParagraphKind::Body | ParagraphKind::Quote => BlockType::Paragraph,
        ParagraphKind::Heading(_) => BlockType::Heading,
        ParagraphKind::List(_) => BlockType::ListItem,
    }
}

impl ProseState {
    /// Write one value mark over every block a selection covers, or remove it
    /// with `None`, in one commit (`PROSE-UI-188`, `PROSE-UI-190`).
    ///
    /// The value is checked under `key` before the first write, so a value
    /// the registry refuses leaves the document as it was
    /// (`PROSE-FEAT-022`).
    pub(super) fn mark_selection(
        &mut self,
        anchor: (BlockId, Pos<DocChar>),
        focus: (BlockId, Pos<DocChar>),
        key: &MarkKey,
        value: Option<&MarkValue>,
    ) -> ProseOutcome {
        // The whole key, not its root: `fs:x` names no control.
        if !MarkRoot::exactly(key).is_some_and(|root| VALUE_KEYS.contains(&root)) {
            return ProseOutcome::refused(ProseRefusal::UnknownMark(key.clone()));
        }
        let stored = match value {
            Some(value) => match mark::validate_value(key, Some(value.as_str())) {
                Ok(canonical) => Some(canonical),
                Err(_) => {
                    return ProseOutcome::refused(ProseRefusal::BadMarkValue {
                        key: key.clone(),
                        value: value.as_str().to_owned(),
                    });
                }
            },
            None => None,
        };
        let spans: Vec<_> = self
            .selected_spans(anchor, focus)
            .into_iter()
            .filter(|(_, span)| !span.is_empty())
            .collect();
        if spans.is_empty() {
            return Self::err_bad_range();
        }
        for (block, span) in spans {
            let (start, end) = (span.start.to_usize(), span.end.to_usize());
            let Some(body) = self.body_of(&tree_id(block)) else {
                return Self::err_no_block(&tree_id(block));
            };
            if end > body.len_unicode() {
                return Self::err_bad_range();
            }
            let written = match &stored {
                Some(value) => body.mark(start..end, key.as_str(), value.as_str()),
                None => body.unmark(start..end, key.as_str()),
            };
            if let Err(e) = written {
                return Self::err_write(e);
            }
        }
        let links = if key.as_str() == MarkRoot::Link.as_str() {
            vec![crate::events::ProseEvent::LinksUpdated {
                links: crate::link::links(&self.doc, &self.roots),
            }]
        } else {
            Vec::new()
        };
        self.commit_with(links)
    }

    /// Write the paragraph properties `style` names on every block a
    /// selection covers, in one commit (`PROSE-UI-259`).
    ///
    /// Only a property whose value differs is written, so a write that
    /// changes nothing exports no delta (`PROSE-FEAT-025`).
    pub(super) fn set_selection_paragraph(
        &mut self,
        anchor: (BlockId, Pos<DocChar>),
        focus: (BlockId, Pos<DocChar>),
        style: &ParagraphStyle,
    ) -> ProseOutcome {
        let blocks = self.block_spans(anchor, focus);
        if blocks.is_empty() {
            return Self::err_bad_range();
        }
        let tree = doc::blocks(&self.doc);
        let mut wrote = false;
        for (block, _) in blocks {
            let id = tree_id(block);
            let Some(node) = doc::node_of(&tree, &id) else {
                return Self::err_no_block(&id);
            };
            let changed =
                doc::changed_paragraph_style(&doc::read_paragraph_style(&tree, &id), style);
            if changed.is_empty() {
                continue;
            }
            if let Err(e) = doc::write_paragraph_style(&node, &changed) {
                return Self::err_write(e);
            }
            wrote = true;
        }
        if !wrote {
            return ProseOutcome::nothing();
        }
        self.commit_with(Vec::new())
    }

    /// Make every paragraph, heading and list item a selection covers the
    /// kind `kind` names, in one commit (`PROSE-UI-189`, `PROSE-UI-260`).
    ///
    /// A code block or a table among them is left as it is. Each block is
    /// checked before the first write: a heading the tree may not hold where
    /// the block sits refuses the whole command (`PROSE-095`).
    pub(super) fn set_selection_kind(
        &mut self,
        anchor: (BlockId, Pos<DocChar>),
        focus: (BlockId, Pos<DocChar>),
        kind: ParagraphKind,
    ) -> ProseOutcome {
        let tree = doc::blocks(&self.doc);
        let targets: Vec<TreeID> = self
            .block_spans(anchor, focus)
            .into_iter()
            .map(|(block, _)| tree_id(block))
            // The paragraph style control turns these three into one another.
            .filter(|id| {
                matches!(
                    doc::block_type_of(&tree, id),
                    Some(BlockType::Paragraph | BlockType::Heading | BlockType::ListItem)
                )
            })
            .collect();
        if targets.is_empty() {
            return Self::err_bad_range();
        }
        let wanted = type_of(kind);
        for id in &targets {
            let parent = match tree.parent(*id) {
                Some(TreeParentId::Node(parent)) => Some(parent),
                Some(TreeParentId::Root) => None,
                _ => return Self::err_no_block(id),
            };
            if !doc::nesting_allows(&tree, parent, wanted) {
                return Self::err_nesting();
            }
        }
        for id in &targets {
            let written = Self::release_children(&tree, id, kind)
                .and_then(|()| Self::write_kind(&tree, id, kind));
            if let Err(e) = written {
                return Self::err_write(e);
            }
        }
        self.commit_with(Vec::new())
    }

    /// Move the blocks a list item holds just after it, in their order,
    /// unless `kind` is a list. A heading and a paragraph hold no block
    /// (`PROSE-UI-279`). The parent of the item held a list item, so it
    /// holds each child too.
    fn release_children(
        tree: &loro::LoroTree,
        id: &TreeID,
        kind: ParagraphKind,
    ) -> Result<(), doc::WriteError> {
        if matches!(kind, ParagraphKind::List(_)) {
            return Ok(());
        }
        let children = tree.children(TreeParentId::Node(*id)).unwrap_or_default();
        for child in children.iter().rev() {
            tree.mov_after(*child, *id)?;
        }
        Ok(())
    }

    /// Move every block a selection covers one step in or out, in one commit
    /// (`PROSE-UI-267`).
    ///
    /// A list item changes level, as a press of Tab does. Any other block
    /// moves its left indent one step, never left of the margin. A gesture
    /// that moves no block writes nothing.
    pub(super) fn indent_selection(
        &mut self,
        anchor: (BlockId, Pos<DocChar>),
        focus: (BlockId, Pos<DocChar>),
        step: IndentStep,
    ) -> ProseOutcome {
        let blocks: Vec<TreeID> = self
            .block_spans(anchor, focus)
            .into_iter()
            .map(|(block, _)| tree_id(block))
            .collect();
        if blocks.is_empty() {
            return Self::err_bad_range();
        }
        let written = match step {
            IndentStep::In => self.indent_in(&blocks),
            IndentStep::Out => self.indent_out(&blocks),
        };
        match written {
            Ok(Changed::Nothing) => ProseOutcome::nothing(),
            Ok(Changed::Written) => self.commit_with(Vec::new()),
            Err(e) => Self::err_write(e),
        }
    }

    /// Move the left indent of one block that is no list item one
    /// [`INDENT_STEP`], never left of the margin.
    fn step_indent(&self, id: &TreeID, step: IndentStep) -> Result<Changed, doc::WriteError> {
        let tree = doc::blocks(&self.doc);
        let node = doc::node_of(&tree, id)
            .ok_or(doc::WriteError::Refused(doc::RefusalCause::BlockVanished))?;
        let held = doc::read_paragraph_style(&tree, id);
        let left = held.indent_left.unwrap_or(PrintLength::ZERO);
        let moved = match step {
            IndentStep::In => left + INDENT_STEP,
            IndentStep::Out => (left - INDENT_STEP).max(PrintLength::ZERO),
        };
        let wanted = ParagraphStyle {
            indent_left: Some(moved),
            ..ParagraphStyle::default()
        };
        let changed = doc::changed_paragraph_style(&held, &wanted);
        if changed.is_empty() {
            return Ok(Changed::Nothing);
        }
        doc::write_paragraph_style(&node, &changed)?;
        Ok(Changed::Written)
    }

    /// One step in, first block first. An item whose block moved with an
    /// item before it in this gesture stays where its parent took it.
    fn indent_in(&self, blocks: &[TreeID]) -> Result<Changed, doc::WriteError> {
        let tree = doc::blocks(&self.doc);
        let mut moved: Vec<TreeID> = Vec::new();
        let mut changed = Changed::Nothing;
        for id in blocks {
            if doc::block_type_of(&tree, id) != Some(BlockType::ListItem) {
                changed = changed.or(self.step_indent(id, IndentStep::In)?);
                continue;
            }
            let mut above = parent_of(&tree, id);
            while let Some(parent) = above {
                if moved.contains(&parent) {
                    break;
                }
                above = parent_of(&tree, &parent);
            }
            if above.is_some() {
                continue;
            }
            let Some(previous) = previous_sibling(&tree, id).filter(|previous| {
                doc::block_type_of(&tree, previous) == Some(BlockType::ListItem)
            }) else {
                continue;
            };
            tree.mov(*id, TreeParentId::Node(previous))?;
            moved.push(*id);
            changed = Changed::Written;
        }
        Ok(changed)
    }

    /// One step out, last block first, so items raised past one parent keep
    /// their order. A top-level item leaves its list, as a body paragraph.
    fn indent_out(&self, blocks: &[TreeID]) -> Result<Changed, doc::WriteError> {
        let tree = doc::blocks(&self.doc);
        blocks
            .iter()
            .rev()
            .try_fold(Changed::Nothing, |changed, id| {
                if doc::block_type_of(&tree, id) != Some(BlockType::ListItem) {
                    return Ok(changed.or(self.step_indent(id, IndentStep::Out)?));
                }
                match parent_of(&tree, id)
                    .filter(|parent| doc::block_type_of(&tree, parent) == Some(BlockType::ListItem))
                {
                    Some(parent) => tree.mov_after(*id, parent)?,
                    None => {
                        Self::release_children(&tree, id, ParagraphKind::Body)?;
                        Self::write_kind(&tree, id, ParagraphKind::Body)?;
                    }
                }
                Ok(Changed::Written)
            })
    }

    /// Write one block's type, its style name and the attributes of `kind`.
    ///
    /// The attribute map stays, and each key of another kind is deleted in
    /// it. A concurrent write to one key of the map is then never dropped
    /// with a fresh container.
    fn write_kind(
        tree: &loro::LoroTree,
        id: &TreeID,
        kind: ParagraphKind,
    ) -> Result<(), doc::WriteError> {
        let node = doc::node_of(tree, id)
            .ok_or(doc::WriteError::Refused(doc::RefusalCause::BlockVanished))?;
        node.insert(doc::NodeKey::Type, i64::from(type_of(kind).code()))?;
        match kind {
            ParagraphKind::Quote => {
                node.insert(doc::NodeKey::Style, doc::BuiltInStyle::Quote.name())?;
            }
            _ => node.delete(doc::NodeKey::Style)?,
        }
        let attrs = doc::attrs_at(&node)
            .ok_or(doc::WriteError::Refused(doc::RefusalCause::AttrMapRefused))?;
        match kind {
            ParagraphKind::Body | ParagraphKind::Quote => {
                for key in [
                    doc::AttrKey::Level,
                    doc::AttrKey::Marker,
                    doc::AttrKey::Spelling,
                    doc::AttrKey::Ticked,
                ] {
                    attrs.delete(key)?;
                }
            }
            ParagraphKind::Heading(level) => {
                attrs.insert(doc::AttrKey::Level, i64::from(level.get()))?;
                for key in [
                    doc::AttrKey::Marker,
                    doc::AttrKey::Spelling,
                    doc::AttrKey::Ticked,
                ] {
                    attrs.delete(key)?;
                }
            }
            ParagraphKind::List(style) => {
                Self::write_list_style(&attrs, style)?;
                attrs.delete(doc::AttrKey::Level)?;
            }
        }
        Ok(())
    }

    /// Set the body font of the document (`PROSE-UI-261`). One write, so
    /// one undo takes it back.
    pub(super) fn set_default_font(&mut self, family: &ambre_core::FontFamily) -> ProseOutcome {
        if doc::read_defaults(&self.doc, &self.roots).font_family == *family {
            return ProseOutcome::nothing();
        }
        if let Err(e) = doc::write_default_font(&self.doc, family) {
            return Self::err_write(e);
        }
        self.commit_with(Vec::new())
    }

    /// The defaults the document holds now: its fonts, its size, its line
    /// height. A read for the panel and for a test.
    #[must_use]
    pub fn defaults(&self) -> doc::DocDefaults {
        doc::read_defaults(&self.doc, &self.roots)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use crate::events::{
        Align, BlockId, BlockType, ListStyle, ParagraphStyle, ProseCommand, ProseEvent,
        ProseOutcome,
    };
    use crate::logic::ProseState;
    use crate::mark::MarkRoot;
    use crate::message::{
        DocChar, HeadingLevel, IndentStep, OrderedStyle, ParagraphKind, SelectionValues,
    };
    use ambre_commands::Reading;
    use ambre_core::crdt::CrdtName as _;
    use ambre_core::index::Pos;
    use ambre_core::{FontFamily, PrintLength, Pt};

    type Spot = (BlockId, Pos<DocChar>);

    /// A document of two paragraphs, "hello" and "world", and the selection
    /// from the start of the first to the end of the second.
    pub(crate) fn two_paragraphs(state: &mut ProseState) -> (BlockId, BlockId, Spot, Spot) {
        let first = state.root_blocks()[0];
        state.handle_command(ProseCommand::InsertText {
            block_id: first,
            pos: Pos::<DocChar>::ZERO,
            text: "hello".to_owned(),
        });
        state.handle_command(ProseCommand::SplitBlock {
            block_id: first,
            pos: Pos::<DocChar>::new(5),
        });
        let second = state.root_blocks()[1];
        state.handle_command(ProseCommand::InsertText {
            block_id: second,
            pos: Pos::<DocChar>::ZERO,
            text: "world".to_owned(),
        });
        (
            first,
            second,
            (first, Pos::<DocChar>::ZERO),
            (second, Pos::<DocChar>::new(5)),
        )
    }

    fn fresh() -> (ProseState, BlockId, BlockId, Spot, Spot) {
        let mut state = ProseState::new().expect("a document is built");
        let (first, second, anchor, focus) = two_paragraphs(&mut state);
        (state, first, second, anchor, focus)
    }

    /// The values the selection reads back.
    pub(crate) fn values(state: &mut ProseState, anchor: Spot, focus: Spot) -> SelectionValues {
        state
            .handle_command(ProseCommand::ReadMarks { anchor, focus })
            .display
            .into_iter()
            .find_map(|e| match e {
                ProseEvent::SelectionMarks { marks, .. } => Some(marks.values),
                _ => None,
            })
            .expect("the selection is read")
    }

    fn refused(outcome: &ProseOutcome) -> bool {
        outcome
            .display
            .iter()
            .any(|e| matches!(e, ProseEvent::Error(_)))
    }

    fn value(key: MarkRoot, raw: &str) -> crate::message::MarkValue {
        crate::mark::validate_value(&key.key(), Some(raw)).expect("a valid value")
    }

    fn mark(
        state: &mut ProseState,
        anchor: Spot,
        focus: Spot,
        key: MarkRoot,
        raw: Option<&str>,
    ) -> ProseOutcome {
        state.handle_command(ProseCommand::MarkSelection {
            anchor,
            focus,
            key: key.key(),
            value: raw.map(|raw| value(key, raw)),
        })
    }

    /// A font over two blocks is one command, reads back on the whole
    /// selection, and one undo takes it off both.
    #[test]
    fn a_font_over_two_blocks_is_one_undo_item() {
        let (mut state, _, _, anchor, focus) = fresh();
        let out = mark(
            &mut state,
            anchor,
            focus,
            MarkRoot::FontFamily,
            Some("Lora"),
        );
        assert!(out.update.is_some(), "the font is written");
        assert_eq!(
            values(&mut state, anchor, focus).font_family,
            Reading::Uniform(Some(FontFamily::from_label("Lora")))
        );
        state.handle_command(ProseCommand::Undo);
        assert_eq!(
            values(&mut state, anchor, focus).font_family,
            Reading::Uniform(None),
            "one undo takes the font off both blocks"
        );
    }

    /// A colour, a highlight and a size reach the whole selection, and a
    /// value the registry refuses writes nothing (`PROSE-UI-190`).
    #[test]
    fn a_colour_a_highlight_and_a_size_reach_the_whole_selection() {
        let (mut state, _, _, anchor, focus) = fresh();
        mark(&mut state, anchor, focus, MarkRoot::Ink, Some("#B4530B"));
        mark(
            &mut state,
            anchor,
            focus,
            MarkRoot::Highlight,
            Some("#ffff00"),
        );
        mark(&mut state, anchor, focus, MarkRoot::FontSize, Some("14.50"));
        let read = values(&mut state, anchor, focus);
        assert_eq!(
            read.color,
            Reading::Uniform(crate::mark::parse_color("#b4530b").ok())
        );
        assert_eq!(
            read.highlight,
            Reading::Uniform(crate::mark::parse_color("#ffff00").ok())
        );
        assert_eq!(
            read.font_size,
            Reading::Uniform(ambre_core::FontSize::new(14.5))
        );

        let before = state.export_snapshot();
        let out = state.handle_command(ProseCommand::MarkSelection {
            anchor,
            focus,
            key: MarkRoot::FontSize.key(),
            value: Some(crate::mark::test_value("2000")),
        });
        assert!(refused(&out), "a size past the cap is shown to the author");
        assert!(out.update.is_none());
        assert_eq!(state.export_snapshot(), before);
    }

    /// The link covers the selection, and `None` takes it off again
    /// (`PROSE-UI-188`).
    #[test]
    fn a_link_covers_the_selection_and_none_takes_it_off() {
        let (mut state, _, _, anchor, focus) = fresh();
        let out = mark(
            &mut state,
            anchor,
            focus,
            MarkRoot::Link,
            Some("https://example.org"),
        );
        assert!(
            out.display
                .iter()
                .any(|e| matches!(e, ProseEvent::LinksUpdated { links } if !links.is_empty())),
            "the link list follows"
        );
        let read = state.handle_command(ProseCommand::ReadMarks { anchor, focus });
        let link = read.display.iter().find_map(|e| match e {
            ProseEvent::SelectionMarks { marks, .. } => Some(marks.link.clone()),
            _ => None,
        });
        assert_eq!(
            link,
            Some(Some(ambre_core::LinkUrl::new(
                "https://example.org".to_owned()
            )))
        );
        mark(&mut state, anchor, focus, MarkRoot::Link, None);
        let read = state.handle_command(ProseCommand::ReadMarks { anchor, focus });
        assert!(read.display.iter().any(|e| matches!(
            e,
            ProseEvent::SelectionMarks { marks, .. } if marks.link.is_none()
        )));
    }

    /// The value command takes the five value keys alone: a presence mark
    /// and an anchor are refused, and write nothing.
    #[test]
    fn the_value_command_refuses_a_presence_mark_and_an_anchor() {
        let (mut state, _, _, anchor, focus) = fresh();
        let before = state.export_snapshot();
        for key in [MarkRoot::Bold, MarkRoot::Comment, MarkRoot::Footnote] {
            let out = state.handle_command(ProseCommand::MarkSelection {
                anchor,
                focus,
                key: key.key(),
                value: Some(crate::mark::test_value("x")),
            });
            assert!(refused(&out), "{key:?}");
        }
        assert_eq!(state.export_snapshot(), before);
    }

    /// An alignment and a line spacing over two blocks are one command
    /// each, and one undo takes the last back on both (`PROSE-UI-259`).
    #[test]
    fn a_paragraph_property_over_two_blocks_is_one_undo_item() {
        let (mut state, first, second, anchor, focus) = fresh();
        let centre = ParagraphStyle {
            align: Some(Align::Center),
            indent_left: Some(PrintLength::<Pt>::new(18.0)),
            ..ParagraphStyle::default()
        };
        let out = state.handle_command(ProseCommand::SetSelectionParagraph {
            anchor,
            focus,
            style: centre,
        });
        assert!(out.update.is_some());
        for block in [first, second] {
            assert_eq!(state.paragraph_style_of(&block).align, Some(Align::Center));
        }
        let read = values(&mut state, anchor, focus);
        assert_eq!(read.align, Reading::Uniform(Align::Center));
        assert_eq!(
            read.indent_left,
            Reading::Uniform(PrintLength::<Pt>::new(18.0))
        );
        // The same write again changes nothing, and exports no delta.
        let again = state.handle_command(ProseCommand::SetSelectionParagraph {
            anchor,
            focus,
            style: centre,
        });
        assert!(again.update.is_none());
        state.handle_command(ProseCommand::Undo);
        for block in [first, second] {
            assert_eq!(
                state.paragraph_style_of(&block).align,
                None,
                "one undo, both blocks"
            );
        }
    }

    /// Two paragraphs become two list items in one command, and back to
    /// body in another; each is one undo item (`PROSE-UI-260`).
    #[test]
    fn a_selection_becomes_a_list_and_back() {
        let (mut state, first, second, anchor, focus) = fresh();
        let list = ParagraphKind::List(ListStyle::Ordered(OrderedStyle::Decimal));
        state.handle_command(ProseCommand::SetSelectionKind {
            anchor,
            focus,
            kind: list,
        });
        for block in [first, second] {
            assert_eq!(state.block_type_of(&block), Some(BlockType::ListItem));
            assert_eq!(state.text_of(&block).as_deref().map(str::len), Some(5));
        }
        assert_eq!(
            values(&mut state, anchor, focus).kind,
            Reading::Uniform(Some(list))
        );
        state.handle_command(ProseCommand::SetSelectionKind {
            anchor,
            focus,
            kind: ParagraphKind::Body,
        });
        for block in [first, second] {
            assert_eq!(state.block_type_of(&block), Some(BlockType::Paragraph));
            assert_eq!(
                state.list_style_of(&block),
                None,
                "no marker is left behind"
            );
        }
        state.handle_command(ProseCommand::Undo);
        for block in [first, second] {
            assert_eq!(
                state.block_type_of(&block),
                Some(BlockType::ListItem),
                "one undo"
            );
        }
        state.handle_command(ProseCommand::Undo);
        for block in [first, second] {
            assert_eq!(state.block_type_of(&block), Some(BlockType::Paragraph));
        }
    }

    /// A heading, a quote and the body each land on every block of the
    /// selection, and the reading names each (`PROSE-UI-189`).
    #[test]
    fn a_heading_a_quote_and_the_body_land_on_every_block() {
        let (mut state, first, second, anchor, focus) = fresh();
        for kind in [
            ParagraphKind::Heading(HeadingLevel::H2),
            ParagraphKind::Quote,
            ParagraphKind::Heading(HeadingLevel::H1),
            ParagraphKind::Body,
        ] {
            let out = state.handle_command(ProseCommand::SetSelectionKind {
                anchor,
                focus,
                kind,
            });
            assert!(out.update.is_some(), "{kind:?}");
            assert_eq!(
                values(&mut state, anchor, focus).kind,
                Reading::Uniform(Some(kind))
            );
            for block in [first, second] {
                let quoted = state.style_of(&block).map(|s| s.as_str().to_owned());
                assert_eq!(
                    quoted.as_deref() == Some("quote"),
                    kind == ParagraphKind::Quote,
                    "{kind:?}"
                );
            }
        }
        // Two kinds over one selection read mixed.
        state.handle_command(ProseCommand::SetSelectionKind {
            anchor,
            focus: (first, Pos::<DocChar>::new(2)),
            kind: ParagraphKind::Quote,
        });
        assert_eq!(values(&mut state, anchor, focus).kind, Reading::Mixed);
    }

    /// A heading the tree may not hold where a block sits refuses the whole
    /// command, and nothing is written (`PROSE-UI-264`).
    #[test]
    fn a_heading_inside_a_list_item_refuses_the_whole_command() {
        let (mut state, first, second, anchor, focus) = fresh();
        state.handle_command(ProseCommand::SetSelectionKind {
            anchor,
            focus,
            kind: ParagraphKind::List(ListStyle::Bullet),
        });
        state.handle_command(ProseCommand::IndentList { block_id: second });
        assert_eq!(state.parent_of(&second), Some(first));
        let before = state.export_snapshot();
        let out = state.handle_command(ProseCommand::SetSelectionKind {
            anchor,
            focus,
            kind: ParagraphKind::Heading(HeadingLevel::H1),
        });
        assert!(refused(&out));
        assert_eq!(state.export_snapshot(), before);
    }

    /// The reading reports a value the whole selection holds, and mixed
    /// where two parts disagree (`PROSE-UI-258`).
    #[test]
    fn the_reading_reports_each_value_and_mixed_where_parts_disagree() {
        let (mut state, first, _, anchor, focus) = fresh();
        let fresh_read = values(&mut state, anchor, focus);
        assert_eq!(fresh_read, SelectionValues::default());
        let head = (first, Pos::<DocChar>::new(2));
        mark(&mut state, anchor, head, MarkRoot::FontSize, Some("20"));
        mark(&mut state, anchor, head, MarkRoot::Ink, Some("#2f6fd6"));
        state.handle_command(ProseCommand::SetSelectionParagraph {
            anchor,
            focus: head,
            style: ParagraphStyle {
                line_height: crate::events::LineSpacingMultiple::new(2.0)
                    .map(crate::events::LineHeight::Multiple),
                ..ParagraphStyle::default()
            },
        });
        let read = values(&mut state, anchor, focus);
        assert_eq!(read.font_size, Reading::Mixed);
        assert_eq!(read.color, Reading::Mixed);
        assert_eq!(read.line_height, Reading::Mixed);
        assert_eq!(read.font_family, Reading::Uniform(None));
        assert_eq!(read.align, Reading::Uniform(Align::Left));
        let part = values(&mut state, anchor, head);
        assert_eq!(
            part.font_size,
            Reading::Uniform(ambre_core::FontSize::new(20.0))
        );
    }

    /// The body font is written, read, resolved for each run, and one undo
    /// takes it back (`PROSE-UI-261`).
    #[test]
    fn the_body_font_is_written_and_reaches_every_run() {
        let (mut state, first, second, _, _) = fresh();
        let body = FontFamily::from_label("Noto Serif");
        let out = state.handle_command(ProseCommand::SetDefaultFont {
            family: body.clone(),
        });
        assert!(out.update.is_some());
        assert_eq!(state.defaults().font_family, body);
        for block in [first, second] {
            assert_eq!(
                state.resolved_runs(&block)[0].1.font_family,
                Some(body.clone())
            );
        }
        let Some(ProseEvent::LayoutShaped { layout }) = state.layout_event() else {
            panic!("the change is laid out");
        };
        let shaped = layout
            .get::<crate::layout::run::ShapedLayout>()
            .expect("a shaped layout");
        assert_eq!(
            shaped.layout.fonts.body, body,
            "the panel reads the layout's font"
        );
        // The same font again writes nothing.
        let again = state.handle_command(ProseCommand::SetDefaultFont {
            family: FontFamily::from_label("Noto Serif"),
        });
        assert!(again.update.is_none());
        state.handle_command(ProseCommand::Undo);
        assert_eq!(
            state.defaults().font_family,
            crate::doc::DocDefaults::default().font_family
        );
    }

    /// A document of one paragraph per entry, and the selection from the
    /// start of the first to the end of the last.
    fn document(texts: &[&str]) -> (ProseState, Vec<BlockId>, Spot, Spot) {
        let mut state = ProseState::new().expect("a document is built");
        let first = state.root_blocks()[0];
        let mut last = first;
        for (at, text) in texts.iter().enumerate() {
            if at > 0 {
                let len = state.text_of(&last).unwrap_or_default().chars().count();
                state.handle_command(ProseCommand::SplitBlock {
                    block_id: last,
                    pos: Pos::<DocChar>::new(u32::try_from(len).expect("short")),
                });
                last = state.root_blocks()[at];
            }
            state.handle_command(ProseCommand::InsertText {
                block_id: last,
                pos: Pos::<DocChar>::ZERO,
                text: (*text).to_owned(),
            });
        }
        let blocks = state.root_blocks();
        let end = u32::try_from(texts.last().map_or(0, |t| t.chars().count())).expect("short");
        (
            state,
            blocks,
            (first, Pos::<DocChar>::ZERO),
            (last, Pos::<DocChar>::new(end)),
        )
    }

    fn kind(state: &mut ProseState, anchor: Spot, focus: Spot, kind: ParagraphKind) {
        let out = state.handle_command(ProseCommand::SetSelectionKind {
            anchor,
            focus,
            kind,
        });
        assert!(out.update.is_some(), "{kind:?} writes");
    }

    fn indent(state: &mut ProseState, anchor: Spot, focus: Spot, step: IndentStep) -> ProseOutcome {
        state.handle_command(ProseCommand::IndentSelection {
            anchor,
            focus,
            step,
        })
    }

    /// `PROSE-UI-267`: an indent nests each item under the one before it and
    /// moves each paragraph one step, and the way out never passes the
    /// margin. A selection that ends at the start of a block leaves that
    /// block where it is.
    #[test]
    fn an_indent_moves_each_block_one_step() {
        let (mut state, blocks, anchor, focus) = document(&["one", "two"]);
        let step = PrintLength::<Pt>::new(36.0);
        let left =
            |state: &ProseState, block: &BlockId| state.paragraph_style_of(block).indent_left;
        assert!(
            indent(
                &mut state,
                anchor,
                (blocks[1], Pos::<DocChar>::ZERO),
                IndentStep::In
            )
            .update
            .is_some()
        );
        assert_eq!(left(&state, &blocks[0]), Some(step));
        assert_eq!(
            left(&state, &blocks[1]),
            None,
            "no character of it is selected"
        );
        state.handle_command(ProseCommand::Undo);

        indent(&mut state, anchor, focus, IndentStep::In);
        assert!(blocks.iter().all(|b| left(&state, b) == Some(step)));
        indent(&mut state, anchor, focus, IndentStep::Out);
        assert!(
            blocks
                .iter()
                .all(|b| left(&state, b) == Some(PrintLength::ZERO))
        );
        let out = indent(&mut state, anchor, focus, IndentStep::Out);
        assert!(out.update.is_none(), "at the margin, nothing moves");

        kind(
            &mut state,
            anchor,
            focus,
            ParagraphKind::List(ListStyle::Bullet),
        );
        indent(&mut state, anchor, focus, IndentStep::In);
        assert_eq!(
            state.parent_of(&blocks[1]),
            Some(blocks[0]),
            "the first has no item before it"
        );
        indent(&mut state, anchor, focus, IndentStep::Out);
        assert_eq!(state.parent_of(&blocks[1]), None);
        assert_eq!(
            state.block_type_of(&blocks[0]),
            Some(BlockType::Paragraph),
            "the top item leaves"
        );
    }

    /// `PROSE-UI-278`: a nested list leaves for the body with every block in
    /// its order and its text, and no paragraph holds a block. One undo
    /// puts the nesting back.
    #[test]
    fn leaving_a_nested_list_keeps_every_block_in_its_order() {
        let (mut state, blocks, anchor, focus) = document(&["one", "two", "three"]);
        kind(
            &mut state,
            anchor,
            focus,
            ParagraphKind::List(ListStyle::Bullet),
        );
        indent(
            &mut state,
            (blocks[1], Pos::<DocChar>::ZERO),
            focus,
            IndentStep::In,
        );
        assert_eq!(state.parent_of(&blocks[1]), Some(blocks[0]));
        assert_eq!(state.parent_of(&blocks[2]), Some(blocks[0]));
        kind(&mut state, anchor, focus, ParagraphKind::Body);
        assert_eq!(state.root_blocks(), blocks, "the order survives");
        assert!(
            blocks
                .iter()
                .all(|b| state.block_type_of(b) == Some(BlockType::Paragraph))
        );
        let texts: Vec<_> = blocks.iter().filter_map(|b| state.text_of(b)).collect();
        assert_eq!(texts, ["one", "two", "three"]);
        state.handle_command(ProseCommand::Undo);
        assert_eq!(state.parent_of(&blocks[2]), Some(blocks[0]), "one undo");
    }

    /// `PROSE-UI-279`: an item that becomes a heading takes the item it
    /// holds out with it, as a list item just after it.
    #[test]
    fn a_heading_from_an_item_takes_its_items_out() {
        let (mut state, blocks, anchor, focus) = document(&["one", "two"]);
        kind(
            &mut state,
            anchor,
            focus,
            ParagraphKind::List(ListStyle::Bullet),
        );
        indent(
            &mut state,
            (blocks[1], Pos::<DocChar>::ZERO),
            focus,
            IndentStep::In,
        );
        assert_eq!(state.parent_of(&blocks[1]), Some(blocks[0]));
        let first = (blocks[0], Pos::<DocChar>::new(3));
        kind(
            &mut state,
            anchor,
            first,
            ParagraphKind::Heading(HeadingLevel::H1),
        );
        assert_eq!(state.root_blocks(), blocks, "the item follows its heading");
        assert_eq!(state.block_type_of(&blocks[0]), Some(BlockType::Heading));
        assert_eq!(state.block_type_of(&blocks[1]), Some(BlockType::ListItem));
    }

    /// `PROSE-UI-271`: a size and an alignment over two blocks change no
    /// text, no bold and no heading level.
    #[test]
    fn a_text_section_write_leaves_the_text_and_every_other_property() {
        let (mut state, blocks, anchor, focus) = document(&["hello", "world"]);
        state.handle_command(ProseCommand::ApplyMark {
            block_id: blocks[0],
            range: ambre_core::index::Span::<DocChar>::from_usize_range(0..2),
            key: MarkRoot::Bold.key(),
            value: None,
        });
        state.handle_command(ProseCommand::SetHeadingLevel {
            block_id: blocks[1],
            level: Some(HeadingLevel::H3),
        });
        let texts = |state: &ProseState| -> Vec<String> {
            blocks.iter().filter_map(|b| state.text_of(b)).collect()
        };
        let carries = |state: &ProseState, block: &BlockId, key: &str| {
            format!("{:?}", state.marks_of(block)).contains(&format!("\"{key}\""))
        };
        let before = texts(&state);
        mark(&mut state, anchor, focus, MarkRoot::FontSize, Some("18"));
        state.handle_command(ProseCommand::SetSelectionParagraph {
            anchor,
            focus,
            style: ParagraphStyle {
                align: Some(Align::Right),
                ..ParagraphStyle::default()
            },
        });
        assert_eq!(texts(&state), before);
        assert!(carries(&state, &blocks[0], "b"), "the bold stays");
        assert!(!carries(&state, &blocks[1], "b"));
        assert_eq!(
            state.heading_level_of(&blocks[1]).map(|l| l.get()),
            Some(3),
            "the heading keeps its level"
        );
        assert!(
            blocks
                .iter()
                .all(|b| state.paragraph_style_of(b).indent_left.is_none()),
            "no other paragraph setting is written"
        );
    }

    /// A quote is laid out italic and set in from the left, unless the
    /// paragraph names its own indent (`PROSE-UI-263`).
    #[test]
    fn a_quote_is_laid_out_italic_and_set_in() {
        let (mut state, first, second, anchor, focus) = fresh();
        state.handle_command(ProseCommand::SetSelectionKind {
            anchor,
            focus,
            kind: ParagraphKind::Quote,
        });
        state.handle_command(ProseCommand::SetParagraphStyle {
            block_id: second,
            style: ParagraphStyle {
                indent_left: Some(PrintLength::<Pt>::new(6.0)),
                ..ParagraphStyle::default()
            },
        });
        let defaults = state.defaults();
        let tree = crate::doc::blocks(&state.doc);
        let laid = |block: BlockId| {
            crate::layout::measure::EffectiveStyle::resolve(
                &defaults,
                &tree,
                &crate::events::tree_id(block),
                ambre_core::Weight::Regular,
            )
        };
        let quote = laid(first);
        assert_eq!(quote.slant, ambre_core::Slant::Italic);
        assert_eq!(quote.indent_left, crate::doc::QUOTE_INDENT.to_doc_px());
        assert_eq!(
            laid(second).indent_left,
            PrintLength::<Pt>::new(6.0).to_doc_px(),
            "the paragraph's own indent wins"
        );
        let _ = MarkRoot::Bold.as_str();
    }
}
