//! The writes of the text section over a whole selection: a value mark, and a
//! block property (`features/ambre-spec-prose-toolbar.md section 1.5`).
//!
//! Each gesture writes every block the selection covers and commits once, so
//! one undo takes the whole gesture back (`CHROME-173`). A block outside the
//! reach of a property keeps what it holds. A heading level skips a list
//! item, and a list marker skips a heading.

use ambre_core::index::Pos;
use ambre_core::{PrintLength, Pt};
use loro::{TreeID, TreeParentId};

use crate::doc;
use crate::events::{
    BlockFormat, BlockId, BlockType, HeadingLevel, IndentStep, ListStyle, ParagraphStyle,
    ProseOutcome, ProseRefusal, tree_id,
};
use crate::mark::{self, MarkRoot};
use crate::message::{DocChar, MarkKey, MarkValue};

use super::ProseState;
use super::lists::{parent_of, previous_sibling};

/// One step of left indent, for a block that is no list item. Half an inch
/// is the step of the office formats.
const INDENT_STEP: PrintLength<Pt> = PrintLength::new(36.0);

/// The keys the value controls of the text section write. The on-or-off keys
/// go through the format bar's command, and an anchor through its own.
const VALUE_KEYS: [MarkRoot; 5] = [
    MarkRoot::FontFamily,
    MarkRoot::FontSize,
    MarkRoot::Ink,
    MarkRoot::Highlight,
    MarkRoot::Link,
];

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

/// The refusal of a store write, as the engine names it.
fn refused(error: loro::LoroError) -> ProseRefusal {
    ProseRefusal::from(doc::WriteError::from(error))
}

impl ProseState {
    /// Write one value mark over every block a selection covers, or take it
    /// off, in one commit (`PROSE-UI-263`, `PROSE-UI-268`).
    ///
    /// The value is checked again under `key` before any block is written,
    /// so a refused value leaves every block as it was (`PROSE-UI-264`).
    pub(super) fn mark_selection(
        &mut self,
        anchor: (BlockId, Pos<DocChar>),
        focus: (BlockId, Pos<DocChar>),
        key: &MarkKey,
        value: Option<&MarkValue>,
    ) -> ProseOutcome {
        let Some(root) = MarkRoot::exactly(key).filter(|root| VALUE_KEYS.contains(root)) else {
            return ProseOutcome::refused(ProseRefusal::UnknownMark(key.clone()));
        };
        let stored = match value.map(|value| mark::validate_value(key, Some(value.as_str()))) {
            None => None,
            Some(Ok(canonical)) => Some(canonical),
            Some(Err(error)) => {
                let raw = value.map(MarkValue::as_str);
                return ProseOutcome::refused(super::marks::refused_as(key, raw, &error));
            }
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
                Some(canonical) => body.mark(start..end, key.as_str(), canonical.as_str()),
                None => body.unmark(start..end, key.as_str()),
            };
            if let Err(e) = written {
                return Self::err_write(e);
            }
        }
        let display = if root == MarkRoot::Link {
            self.links_touched(Some(key))
        } else {
            Vec::new()
        };
        self.commit_with(display)
    }

    /// Write one block property over every block a selection covers, in one
    /// commit (`PROSE-UI-265`, `PROSE-UI-266`, `PROSE-UI-267`).
    ///
    /// A gesture that changes no block writes nothing.
    pub(super) fn format_blocks(
        &mut self,
        anchor: (BlockId, Pos<DocChar>),
        focus: (BlockId, Pos<DocChar>),
        format: BlockFormat,
    ) -> ProseOutcome {
        let blocks = self.formatted_blocks(anchor, focus);
        if blocks.is_empty() {
            return Self::err_bad_range();
        }
        let written = match format {
            BlockFormat::Paragraph(style) => self.write_paragraph_styles(&blocks, &style),
            BlockFormat::Heading(level) => self.write_headings(&blocks, level),
            BlockFormat::List(Some(style)) => self.enter_list(&blocks, style),
            BlockFormat::List(None) => self.leave_list(&blocks),
            BlockFormat::Indent(step) => self.indent_blocks(&blocks, step),
        };
        match written {
            Ok(Changed::Nothing) => ProseOutcome::nothing(),
            Ok(Changed::Written) => self.commit_with(Vec::new()),
            Err(refusal) => ProseOutcome::refused(refusal),
        }
    }

    /// The blocks a block property reaches, in document order.
    ///
    /// A selection that ends at the start of a block leaves that block out:
    /// no character of it is selected. A selection with no character at all
    /// keeps the block of its caret.
    fn formatted_blocks(
        &self,
        anchor: (BlockId, Pos<DocChar>),
        focus: (BlockId, Pos<DocChar>),
    ) -> Vec<TreeID> {
        let spans = self.selected_spans(anchor, focus);
        let any_text = spans.iter().any(|(_, span)| !span.is_empty());
        spans
            .into_iter()
            .filter(|(_, span)| !any_text || !span.is_empty())
            .map(|(block, _)| tree_id(block))
            .collect()
    }

    /// Write the paragraph settings `style` names on each block, and only the
    /// properties that differ from what the block stores (`PROSE-FEAT-025`).
    fn write_paragraph_styles(
        &self,
        blocks: &[TreeID],
        style: &ParagraphStyle,
    ) -> Result<Changed, ProseRefusal> {
        blocks.iter().try_fold(Changed::Nothing, |changed, id| {
            Ok(changed.or(self.write_paragraph_style_of(id, style)?))
        })
    }

    /// The write of one block's paragraph settings, diffed against the store.
    fn write_paragraph_style_of(
        &self,
        id: &TreeID,
        style: &ParagraphStyle,
    ) -> Result<Changed, ProseRefusal> {
        let tree = doc::blocks(&self.doc);
        let Some(node) = doc::node_of(&tree, id) else {
            return Err(ProseRefusal::NoSuchBlock(crate::events::block_id(*id)));
        };
        let changed = doc::changed_paragraph_style(&doc::read_paragraph_style(&tree, id), style);
        if changed.is_empty() {
            return Ok(Changed::Nothing);
        }
        doc::write_paragraph_style(&node, &changed).map_err(ProseRefusal::from)?;
        Ok(Changed::Written)
    }

    /// Set the paragraph style of each paragraph and heading. A block already
    /// at that style, and a block of any other type, keeps what it holds.
    fn write_headings(
        &self,
        blocks: &[TreeID],
        level: Option<HeadingLevel>,
    ) -> Result<Changed, ProseRefusal> {
        let tree = doc::blocks(&self.doc);
        blocks.iter().try_fold(Changed::Nothing, |changed, id| {
            let held = match doc::block_type_of(&tree, id) {
                Some(BlockType::Paragraph) => None,
                Some(BlockType::Heading) => doc::heading_level_of(&tree, id),
                _ => return Ok(changed),
            };
            if held == level.map(doc::StoredLevel::of) {
                return Ok(changed);
            }
            self.write_heading_level(id, level)?;
            Ok(Changed::Written)
        })
    }

    /// Make each paragraph a list item of `style`, where the nesting table
    /// allows one, and give each item `style` (`PROSE-UI-266`).
    fn enter_list(&self, blocks: &[TreeID], style: ListStyle) -> Result<Changed, ProseRefusal> {
        let tree = doc::blocks(&self.doc);
        blocks.iter().try_fold(Changed::Nothing, |changed, id| {
            let Some(node) = doc::node_of(&tree, id) else {
                return Ok(changed);
            };
            match doc::block_type_of(&tree, id) {
                Some(BlockType::Paragraph) => {
                    if !doc::nesting_allows(&tree, parent_of(&tree, id), BlockType::ListItem) {
                        return Ok(changed);
                    }
                    node.insert(doc::NodeKey::Type, i64::from(BlockType::ListItem.code()))
                        .map_err(refused)?;
                }
                Some(BlockType::ListItem) if doc::list_style_of(&tree, id) != Some(style) => {}
                _ => return Ok(changed),
            }
            let Some(attrs) = doc::attrs_at(&node) else {
                return Err(ProseRefusal::from(doc::WriteError::Refused(
                    doc::RefusalCause::AttrMapRefused,
                )));
            };
            Self::write_list_style(&attrs, style).map_err(ProseRefusal::from)?;
            Ok(Changed::Written)
        })
    }

    /// Take each list item out of its list, as a paragraph at the top of the
    /// tree, in the order the items read (`PROSE-UI-266`).
    ///
    /// The items are taken last first. Each one lands just after the block
    /// that held it, so the order of the selection survives the walk.
    fn leave_list(&self, blocks: &[TreeID]) -> Result<Changed, ProseRefusal> {
        let tree = doc::blocks(&self.doc);
        blocks
            .iter()
            .rev()
            .try_fold(Changed::Nothing, |changed, id| {
                if doc::block_type_of(&tree, id) != Some(BlockType::ListItem) {
                    return Ok(changed);
                }
                while let Some(parent) = parent_of(&tree, id)
                    .filter(|parent| doc::block_type_of(&tree, parent) == Some(BlockType::ListItem))
                {
                    tree.mov_after(*id, parent).map_err(refused)?;
                }
                self.unmake_item(id)?;
                Ok(Changed::Written)
            })
    }

    /// Turn one item into a paragraph in its place. Its own items move out
    /// first, just after it, since a paragraph holds no child.
    fn unmake_item(&self, id: &TreeID) -> Result<(), ProseRefusal> {
        let tree = doc::blocks(&self.doc);
        let Some(node) = doc::node_of(&tree, id) else {
            return Err(ProseRefusal::NoSuchBlock(crate::events::block_id(*id)));
        };
        let children = tree.children(TreeParentId::Node(*id)).unwrap_or_default();
        for child in children.iter().rev() {
            tree.mov_after(*child, *id).map_err(refused)?;
        }
        node.insert(doc::NodeKey::Type, i64::from(BlockType::Paragraph.code()))
            .map_err(refused)?;
        node.delete(doc::NodeKey::Attrs).map_err(refused)
    }

    /// Move each block one step in or out (`PROSE-UI-267`).
    ///
    /// A list item changes level, as a press of Tab does. Any other block
    /// moves its left indent one step, never left of the margin.
    fn indent_blocks(&self, blocks: &[TreeID], step: IndentStep) -> Result<Changed, ProseRefusal> {
        match step {
            IndentStep::In => self.indent_in(blocks),
            IndentStep::Out => self.indent_out(blocks),
        }
    }

    /// One step in, first block first. An item whose block moved with an
    /// item before it in this gesture stays where its parent took it.
    fn indent_in(&self, blocks: &[TreeID]) -> Result<Changed, ProseRefusal> {
        let tree = doc::blocks(&self.doc);
        let mut moved: Vec<TreeID> = Vec::new();
        let mut changed = Changed::Nothing;
        for id in blocks {
            if doc::block_type_of(&tree, id) != Some(BlockType::ListItem) {
                let left = doc::read_paragraph_style(&tree, id)
                    .indent_left
                    .unwrap_or(PrintLength::ZERO);
                let style = ParagraphStyle {
                    indent_left: Some(left + INDENT_STEP),
                    ..ParagraphStyle::default()
                };
                changed = changed.or(self.write_paragraph_style_of(id, &style)?);
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
            tree.mov(*id, TreeParentId::Node(previous))
                .map_err(refused)?;
            moved.push(*id);
            changed = Changed::Written;
        }
        Ok(changed)
    }

    /// One step out, last block first, so items raised past one parent keep
    /// their order. A top-level item leaves its list.
    fn indent_out(&self, blocks: &[TreeID]) -> Result<Changed, ProseRefusal> {
        let tree = doc::blocks(&self.doc);
        blocks
            .iter()
            .rev()
            .try_fold(Changed::Nothing, |changed, id| {
                if doc::block_type_of(&tree, id) != Some(BlockType::ListItem) {
                    let left = doc::read_paragraph_style(&tree, id)
                        .indent_left
                        .unwrap_or(PrintLength::ZERO);
                    let style = ParagraphStyle {
                        indent_left: Some((left - INDENT_STEP).max(PrintLength::ZERO)),
                        ..ParagraphStyle::default()
                    };
                    return Ok(changed.or(self.write_paragraph_style_of(id, &style)?));
                }
                match parent_of(&tree, id)
                    .filter(|parent| doc::block_type_of(&tree, parent) == Some(BlockType::ListItem))
                {
                    Some(parent) => tree.mov_after(*id, parent).map_err(refused)?,
                    None => self.unmake_item(id)?,
                }
                Ok(Changed::Written)
            })
    }
}

#[cfg(test)]
mod tests {
    use crate::events::{
        ActiveMarks, Align, BlockFormat, BlockId, BlockType, HeadingLevel, IndentStep, ListStyle,
        ParagraphStyle, ProseCommand, ProseEvent, ProseRefusal, Reading,
    };
    use crate::logic::ProseState;
    use crate::mark::MarkRoot;
    use crate::message::{DocChar, MarkValue};
    use ambre_core::crdt::CrdtName as _;
    use ambre_core::index::Pos;
    use ambre_core::{PrintLength, Pt};

    type Spot = (BlockId, Pos<DocChar>);

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

    /// The reason an outcome was refused for, when it was.
    fn refusal(outcome: &crate::events::ProseOutcome) -> Option<ProseRefusal> {
        outcome.display.iter().find_map(|event| match event {
            ProseEvent::Error(error) => Some(error.reason.clone()),
            _ => None,
        })
    }

    fn value(root: MarkRoot, raw: &str) -> MarkValue {
        crate::mark::validate_value(&root.key(), Some(raw)).expect("a valid value")
    }

    fn carries(state: &ProseState, block: &BlockId, key: &str) -> bool {
        format!("{:?}", state.marks_of(block)).contains(&format!("\"{key}\""))
    }

    fn read(state: &mut ProseState, anchor: Spot, focus: Spot) -> ActiveMarks {
        state
            .handle_command(ProseCommand::ReadMarks { anchor, focus })
            .display
            .into_iter()
            .find_map(|event| match event {
                ProseEvent::SelectionMarks { marks, .. } => Some(marks),
                _ => None,
            })
            .expect("the marks are read")
    }

    fn blocks_of(state: &mut ProseState, anchor: Spot, focus: Spot, format: BlockFormat) {
        let out = state.handle_command(ProseCommand::FormatBlocks {
            anchor,
            focus,
            format,
        });
        assert!(out.update.is_some(), "{format:?} writes");
    }

    /// `PROSE-UI-263`: a size, a colour, a highlight and a family over two
    /// blocks each write both in one command, and one undo takes each back.
    #[test]
    fn a_value_over_two_blocks_is_one_undo_item() {
        let (mut state, blocks, anchor, focus) = document(&["hello", "world"]);
        for (root, raw) in [
            (MarkRoot::FontSize, "18"),
            (MarkRoot::Ink, "#3366cc"),
            (MarkRoot::Highlight, "#ffee00"),
            (MarkRoot::FontFamily, "Serif"),
        ] {
            let out = state.handle_command(ProseCommand::MarkSelection {
                anchor,
                focus,
                key: root.key(),
                value: Some(value(root, raw)),
            });
            assert!(out.update.is_some(), "{root:?} writes");
            assert!(blocks.iter().all(|b| carries(&state, b, root.as_str())));
            state.handle_command(ProseCommand::Undo);
            assert!(
                blocks.iter().all(|b| !carries(&state, b, root.as_str())),
                "one undo takes {root:?} off both blocks"
            );
        }
    }

    /// `PROSE-UI-264`: a value the grammar of its key refuses, and a key the
    /// value controls do not write, are refused before any block is written.
    #[test]
    fn a_refused_value_writes_no_block_of_the_selection() {
        let (mut state, blocks, anchor, focus) = document(&["hello", "world"]);
        let out = state.handle_command(ProseCommand::MarkSelection {
            anchor,
            focus,
            key: MarkRoot::Ink.key(),
            value: Some(value(MarkRoot::FontSize, "18")),
        });
        assert!(matches!(
            refusal(&out),
            Some(ProseRefusal::BadMarkValue { .. })
        ));
        for key in [MarkRoot::Bold, MarkRoot::Comment] {
            let out = state.handle_command(ProseCommand::MarkSelection {
                anchor,
                focus,
                key: key.key(),
                value: None,
            });
            assert!(matches!(refusal(&out), Some(ProseRefusal::UnknownMark(_))));
        }
        assert!(blocks.iter().all(|b| !carries(&state, b, "fg")));
        assert!(blocks.iter().all(|b| !carries(&state, b, "b")));
    }

    /// `PROSE-UI-268`: a link covers the whole selection, and the panel reads
    /// it back. A write of none takes it off, and one undo puts it back.
    #[test]
    fn a_selection_takes_a_link_and_gives_it_back() {
        let (mut state, blocks, anchor, focus) = document(&["hello", "world"]);
        let out = state.handle_command(ProseCommand::MarkSelection {
            anchor,
            focus,
            key: MarkRoot::Link.key(),
            value: Some(value(MarkRoot::Link, "https://example.org")),
        });
        assert!(
            out.display
                .iter()
                .any(|e| matches!(e, ProseEvent::LinksUpdated { .. }))
        );
        assert_eq!(
            read(&mut state, anchor, focus).link,
            Some(ambre_core::LinkUrl::new("https://example.org".to_owned()))
        );
        state.handle_command(ProseCommand::MarkSelection {
            anchor,
            focus,
            key: MarkRoot::Link.key(),
            value: None,
        });
        assert!(blocks.iter().all(|b| !carries(&state, b, "a")));
        state.handle_command(ProseCommand::Undo);
        assert!(blocks.iter().all(|b| carries(&state, b, "a")));
    }

    /// `PROSE-UI-265`: an alignment and a paragraph style reach every block
    /// of the selection in one command, and one undo takes each back.
    #[test]
    fn a_block_property_over_two_blocks_is_one_undo_item() {
        let (mut state, blocks, anchor, focus) = document(&["hello", "world"]);
        let centred = ParagraphStyle {
            align: Some(Align::Center),
            ..ParagraphStyle::default()
        };
        blocks_of(&mut state, anchor, focus, BlockFormat::Paragraph(centred));
        assert!(
            blocks
                .iter()
                .all(|b| state.paragraph_style_of(b).align == Some(Align::Center))
        );
        state.handle_command(ProseCommand::Undo);
        assert!(
            blocks
                .iter()
                .all(|b| state.paragraph_style_of(b).align.is_none())
        );

        blocks_of(
            &mut state,
            anchor,
            focus,
            BlockFormat::Heading(Some(HeadingLevel::H2)),
        );
        assert!(
            blocks
                .iter()
                .all(|b| state.block_type_of(b) == Some(BlockType::Heading))
        );
        assert_eq!(
            read(&mut state, anchor, focus).heading,
            Reading::Uniform(Some(HeadingLevel::H2))
        );
        blocks_of(&mut state, anchor, focus, BlockFormat::Heading(None));
        assert!(
            blocks
                .iter()
                .all(|b| state.block_type_of(b) == Some(BlockType::Paragraph))
        );
        state.handle_command(ProseCommand::Undo);
        assert!(
            blocks
                .iter()
                .all(|b| state.block_type_of(b) == Some(BlockType::Heading)),
            "one undo puts both headings back"
        );
    }

    /// `PROSE-UI-266`: the paragraphs of a selection become list items and
    /// back, their text kept, each direction one undo item.
    #[test]
    fn a_paragraph_becomes_a_list_item_and_back() {
        let (mut state, blocks, anchor, focus) = document(&["one", "two", "three"]);
        blocks_of(
            &mut state,
            anchor,
            focus,
            BlockFormat::List(Some(ListStyle::Bullet)),
        );
        assert!(
            blocks
                .iter()
                .all(|b| state.list_style_of(b) == Some(ListStyle::Bullet))
        );
        assert_eq!(
            read(&mut state, anchor, focus).list,
            Reading::Uniform(Some(ListStyle::Bullet))
        );
        // Nest the last two under the first, then take the whole list out.
        blocks_of(
            &mut state,
            (blocks[1], Pos::<DocChar>::ZERO),
            focus,
            BlockFormat::Indent(IndentStep::In),
        );
        assert_eq!(state.parent_of(&blocks[1]), Some(blocks[0]));
        assert_eq!(state.parent_of(&blocks[2]), Some(blocks[0]));
        blocks_of(&mut state, anchor, focus, BlockFormat::List(None));
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

    /// `PROSE-UI-267`: an indent nests each item under the one before it and
    /// moves each paragraph one step, and the way out never passes the
    /// margin.
    #[test]
    fn an_indent_moves_each_block_one_step() {
        let (mut state, blocks, anchor, focus) = document(&["one", "two"]);
        let step = PrintLength::<Pt>::new(36.0);
        blocks_of(
            &mut state,
            anchor,
            focus,
            BlockFormat::Indent(IndentStep::In),
        );
        assert!(
            blocks
                .iter()
                .all(|b| state.paragraph_style_of(b).indent_left == Some(step))
        );
        blocks_of(
            &mut state,
            anchor,
            focus,
            BlockFormat::Indent(IndentStep::Out),
        );
        assert!(
            blocks
                .iter()
                .all(|b| state.paragraph_style_of(b).indent_left == Some(PrintLength::ZERO))
        );
        let out = state.handle_command(ProseCommand::FormatBlocks {
            anchor,
            focus,
            format: BlockFormat::Indent(IndentStep::Out),
        });
        assert!(out.update.is_none(), "at the margin, nothing moves");

        blocks_of(
            &mut state,
            anchor,
            focus,
            BlockFormat::List(Some(ListStyle::Bullet)),
        );
        blocks_of(
            &mut state,
            anchor,
            focus,
            BlockFormat::Indent(IndentStep::In),
        );
        assert_eq!(
            state.parent_of(&blocks[1]),
            Some(blocks[0]),
            "the first has no item before it"
        );
        blocks_of(
            &mut state,
            anchor,
            focus,
            BlockFormat::Indent(IndentStep::Out),
        );
        assert_eq!(state.parent_of(&blocks[1]), None);
        assert_eq!(
            state.block_type_of(&blocks[0]),
            Some(BlockType::Paragraph),
            "the top item leaves"
        );
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
        let before = texts(&state);
        state.handle_command(ProseCommand::MarkSelection {
            anchor,
            focus,
            key: MarkRoot::FontSize.key(),
            value: Some(value(MarkRoot::FontSize, "18")),
        });
        blocks_of(
            &mut state,
            anchor,
            focus,
            BlockFormat::Paragraph(ParagraphStyle {
                align: Some(Align::Right),
                ..ParagraphStyle::default()
            }),
        );
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

    /// `PROSE-UI-269`: the panel reads one value where every run or block
    /// agrees, and mixed where two disagree.
    #[test]
    fn the_selection_reads_each_value_uniform_or_mixed() {
        let (mut state, blocks, anchor, focus) = document(&["hello", "world"]);
        let before = read(&mut state, anchor, focus);
        assert!(
            matches!(before.font_family, Reading::Uniform(Some(_))),
            "the default face"
        );
        assert_eq!(before.color, Reading::Uniform(None));
        assert_eq!(before.align, Reading::Uniform(Some(Align::Left)));
        assert_eq!(before.list, Reading::Uniform(None));
        state.handle_command(ProseCommand::MarkSelection {
            anchor,
            focus: (blocks[0], Pos::<DocChar>::new(5)),
            key: MarkRoot::Ink.key(),
            value: Some(value(MarkRoot::Ink, "#3366cc")),
        });
        state.handle_command(ProseCommand::FormatBlocks {
            anchor,
            focus: (blocks[0], Pos::<DocChar>::new(5)),
            format: BlockFormat::Paragraph(ParagraphStyle {
                align: Some(Align::Right),
                ..ParagraphStyle::default()
            }),
        });
        let both = read(&mut state, anchor, focus);
        assert_eq!(both.color, Reading::Mixed);
        assert_eq!(both.align, Reading::Mixed);
        assert!(matches!(both.font_size, Reading::Uniform(Some(_))));
        let first = read(&mut state, anchor, (blocks[0], Pos::<DocChar>::new(5)));
        assert_eq!(
            first.color,
            Reading::Uniform(Some(ambre_core::Rgba::from_channels(
                0x33, 0x66, 0xcc, 0xff
            )))
        );
        assert_eq!(first.align, Reading::Uniform(Some(Align::Right)));
    }
}
