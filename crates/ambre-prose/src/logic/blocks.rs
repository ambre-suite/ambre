//! The block lifecycle, the nesting guard and the two style registers
//! (`ambre-spec-prose-schema.md sections 3 and 4`).
//!
//! Every insert and every move passes the nesting table before it writes
//! (`PROSE-040`, `PROSE-095`). A converged document therefore holds only the
//! parentages section 4 declares, because no peer ever wrote another one.

use loro::{LoroMap, TreeID, TreeParentId};

use crate::doc;
use crate::events::{BlockId, BlockInit, BlockType, ParagraphStyle, ProseOutcome, tree_id};

/// The levels a heading may carry.
const HEADING_LEVELS: std::ops::RangeInclusive<u8> = 1..=6;

/// One heading level, brought inside the six the format declares.
fn clamp_level(level: u8) -> u8 {
    level.clamp(*HEADING_LEVELS.start(), *HEADING_LEVELS.end())
}

use super::ProseState;

/// The parent a placement actually lands under.
///
/// `mov_after` addresses a sibling position, so it reparents the block under
/// that sibling's own parent. A nesting check made against the parent the
/// command named would then guard a parentage the write never uses
/// (`PROSE-040`, `PROSE-095`). This resolves the landing parent before the
/// check. A named parent the sibling contradicts is refused rather than
/// silently overridden: the caller meant one of the two.
///
/// # Errors
/// `Err(())` when the sibling is not a live block, or when the two disagree.
fn landing_parent(
    tree: &loro::LoroTree,
    parent_id: Option<TreeID>,
    after_id: Option<TreeID>,
) -> Result<Option<TreeID>, ()> {
    let Some(after) = after_id else {
        return Ok(parent_id);
    };
    let landing = match tree.parent(after) {
        Some(TreeParentId::Node(parent)) => Some(parent),
        Some(TreeParentId::Root) => None,
        _ => return Err(()),
    };
    if parent_id.is_some() && parent_id != landing {
        return Err(());
    }
    Ok(landing)
}

impl ProseState {
    /// The type a live block carries, for a test and for the view's own reads.
    #[must_use]
    pub fn block_type_of(&self, id: &BlockId) -> Option<BlockType> {
        doc::block_type_of(&doc::blocks(&self.doc), &tree_id(*id))
    }

    /// The style id a live block carries, or `None` for the type's built-in.
    #[must_use]
    pub fn style_of(&self, id: &BlockId) -> Option<String> {
        doc::style_of(&doc::blocks(&self.doc), &tree_id(*id))
    }

    /// The paragraph style a live block carries.
    #[must_use]
    pub fn paragraph_style_of(&self, id: &BlockId) -> ParagraphStyle {
        doc::read_paragraph_style(&doc::blocks(&self.doc), &tree_id(*id))
    }

    /// The level a live heading carries.
    #[must_use]
    pub fn heading_level_of(&self, id: &BlockId) -> Option<u8> {
        let attrs = self.attrs_of(&tree_id(*id))?;
        match attrs.get(doc::A_LEVEL)?.into_value() {
            Ok(loro::LoroValue::I64(n)) => u8::try_from(n).ok(),
            _ => None,
        }
    }

    /// The language a live code block carries.
    #[must_use]
    pub fn code_lang_of(&self, id: &BlockId) -> Option<String> {
        let attrs = self.attrs_of(&tree_id(*id))?;
        match attrs.get(doc::A_LANG)?.into_value() {
            Ok(loro::LoroValue::String(lang)) => Some(lang.to_string()),
            _ => None,
        }
    }

    /// The content hash a live image block addresses.
    pub(super) fn image_hash(&self, id: &TreeID) -> Option<String> {
        let attrs = self.attrs_of(id)?;
        match attrs.get("content_hash")?.into_value() {
            Ok(loro::LoroValue::String(s)) => Some(s.to_string()),
            _ => None,
        }
    }

    /// The attribute map of a live block, or `None` when the type carries none.
    fn attrs_of(&self, id: &TreeID) -> Option<LoroMap> {
        let tree = doc::blocks(&self.doc);
        let node = doc::node_of(&tree, id)?;
        node.get(doc::K_ATTRS)
            .and_then(|v| v.into_container().ok())
            .and_then(|c| c.into_map().ok())
    }

    // -- the lifecycle -----------------------------------------------------

    /// Mint one block under a parent, after a sibling.
    ///
    /// The initializer carries the attributes the type needs at creation, so no
    /// follow-up command corrects a default this handler invented.
    pub(super) fn insert_block(
        &mut self,
        parent_id: Option<TreeID>,
        after_id: Option<TreeID>,
        init: &BlockInit,
        style_id: Option<&str>,
    ) -> ProseOutcome {
        let tree = doc::blocks(&self.doc);
        if let Some(parent) = parent_id
            && doc::node_of(&tree, &parent).is_none()
        {
            return Self::err_no_block(&parent);
        }
        if let Some(after) = after_id
            && doc::node_of(&tree, &after).is_none()
        {
            return Self::err_no_block(&after);
        }
        let Ok(parent_id) = landing_parent(&tree, parent_id, after_id) else {
            return Self::err_nesting();
        };
        if !doc::nesting_allows(&tree, parent_id, init.block_type()) {
            return Self::err_nesting();
        }
        let Some(id) = doc::create_block(&self.doc, parent_id, init.block_type()) else {
            return Self::err_write("the tree refused a block");
        };
        if let Err(e) = self.write_init(&id, init) {
            return Self::err_write(&e);
        }
        if let Some(style_id) = style_id
            && let Some(node) = doc::node_of(&tree, &id)
            && let Err(e) = node.insert(doc::K_STYLE, style_id)
        {
            return Self::err_write(&format!("{e:?}"));
        }
        if let Some(after) = after_id
            && let Err(e) = tree.mov_after(id, after)
        {
            return Self::err_write(&format!("{e:?}"));
        }
        self.commit_with(Vec::new())
    }

    /// Write the attributes one initializer carries.
    fn write_init(&self, id: &TreeID, init: &BlockInit) -> Result<(), String> {
        let tree = doc::blocks(&self.doc);
        let Some(node) = doc::node_of(&tree, id) else {
            return Err("the block vanished before its attributes landed".to_owned());
        };
        let attrs = |key: &str, value: loro::LoroValue| -> Result<(), String> {
            let map = doc::map_at(&node, doc::K_ATTRS)
                .ok_or_else(|| "the attribute map refused a container".to_owned())?;
            map.insert(key, value).map_err(|e| format!("{e:?}"))
        };
        match init {
            BlockInit::Heading { level } => {
                attrs(doc::A_LEVEL, i64::from(clamp_level(*level)).into())
            }
            BlockInit::CodeBlock { lang } => attrs(doc::A_LANG, lang.as_str().into()),
            // The three remaining types carry no attribute, so they carry no
            // attribute map either (`PROSE-096`).
            BlockInit::Paragraph | BlockInit::HRule | BlockInit::PageBreak => Ok(()),
        }
    }

    /// Mint one block of a type no `BlockInit` carries yet, for a test that
    /// binds the format rather than a gesture.
    ///
    /// It passes the same nesting guard every command does, so a test cannot
    /// build a shape section 4 refuses. Test-only: the gestures that create a
    /// table and a list item land with their own Story
    /// (`ambre-roadmap.md` E5).
    #[cfg(test)]
    pub(crate) fn insert_for_test(
        &mut self,
        parent_id: Option<BlockId>,
        block_type: BlockType,
    ) -> Option<BlockId> {
        let parent = parent_id.map(tree_id);
        if !doc::nesting_allows(&doc::blocks(&self.doc), parent, block_type) {
            return None;
        }
        let id = doc::create_block(&self.doc, parent, block_type)?;
        self.doc.commit();
        Some(crate::events::block_id(id))
    }

    /// Delete one block, and the subtree under it.
    ///
    /// A deletion that empties the tree is re-grown by the commit path from the
    /// tombstone every peer picks alike (`PROSE-027`, `PROSE-094`).
    pub(super) fn delete_block(&mut self, id: &TreeID) -> ProseOutcome {
        let tree = doc::blocks(&self.doc);
        if doc::node_of(&tree, id).is_none() {
            return Self::err_no_block(id);
        }
        if let Err(e) = tree.delete(*id) {
            return Self::err_write(&format!("{e:?}"));
        }
        self.commit_with(Vec::new())
    }

    /// Move one block to another parent, after a sibling.
    ///
    /// The block keeps its identity, so every reference to it still resolves:
    /// a comment anchor, a cross reference target, a band's block id
    /// (`PROSE-085`, `PROSE-086`).
    pub(super) fn move_block(
        &mut self,
        id: &TreeID,
        new_parent: Option<TreeID>,
        after_id: Option<TreeID>,
    ) -> ProseOutcome {
        let tree = doc::blocks(&self.doc);
        let Some(block_type) = doc::block_type_of(&tree, id) else {
            return Self::err_no_block(id);
        };
        if let Some(parent) = new_parent
            && doc::node_of(&tree, &parent).is_none()
        {
            return Self::err_no_block(&parent);
        }
        if let Some(after) = after_id
            && doc::node_of(&tree, &after).is_none()
        {
            return Self::err_no_block(&after);
        }
        // The sibling position decides the parent, so both guards below are
        // made against the parent the block actually lands under.
        let Ok(new_parent) = landing_parent(&tree, new_parent, after_id) else {
            return Self::err_nesting();
        };
        if let Some(parent) = new_parent {
            // A block moved under itself or under its own descendant leaves the
            // tree for a cycle, and no parentage of section 4 describes that.
            if parent == *id || doc::is_ancestor_of(&tree, id, &parent) {
                return Self::err_nesting();
            }
        }
        if !doc::nesting_allows(&tree, new_parent, block_type) {
            return Self::err_nesting();
        }
        let parent: TreeParentId = new_parent.into();
        if let Err(e) = tree.mov(*id, parent) {
            return Self::err_write(&format!("{e:?}"));
        }
        if let Some(after) = after_id
            && let Err(e) = tree.mov_after(*id, after)
        {
            return Self::err_write(&format!("{e:?}"));
        }
        self.commit_with(Vec::new())
    }

    // -- the two style registers -------------------------------------------

    /// Name the style a block renders with.
    ///
    /// A built-in name is accepted and stored as the key alone: the nine
    /// built-ins are compiled in, and `styles` holds the user's own styles
    /// (`PROSE-093`).
    pub(super) fn set_block_style(&mut self, id: &TreeID, style_id: &str) -> ProseOutcome {
        let tree = doc::blocks(&self.doc);
        let Some(node) = doc::node_of(&tree, id) else {
            return Self::err_no_block(id);
        };
        if let Err(e) = node.insert(doc::K_STYLE, style_id) {
            return Self::err_write(&format!("{e:?}"));
        }
        self.commit_with(Vec::new())
    }

    /// Set a heading's level, clamped to the six the format declares.
    pub(super) fn set_heading_level(&mut self, id: &TreeID, level: u8) -> ProseOutcome {
        if doc::block_type_of(&doc::blocks(&self.doc), id) != Some(BlockType::Heading) {
            return Self::err_no_block(id);
        }
        let level = clamp_level(level);
        self.write_attr(id, doc::A_LEVEL, i64::from(level).into())
    }

    /// Set a code block's language. An empty value reads as plain.
    pub(super) fn set_code_lang(&mut self, id: &TreeID, lang: &str) -> ProseOutcome {
        if doc::block_type_of(&doc::blocks(&self.doc), id) != Some(BlockType::CodeBlock) {
            return Self::err_no_block(id);
        }
        self.write_attr(id, doc::A_LANG, lang.into())
    }

    /// Write one attribute of a live block.
    fn write_attr(&mut self, id: &TreeID, key: &str, value: loro::LoroValue) -> ProseOutcome {
        let tree = doc::blocks(&self.doc);
        let Some(node) = doc::node_of(&tree, id) else {
            return Self::err_no_block(id);
        };
        let Some(attrs) = doc::map_at(&node, doc::K_ATTRS) else {
            return Self::err_write("the attribute map refused a container");
        };
        if let Err(e) = attrs.insert(key, value) {
            return Self::err_write(&format!("{e:?}"));
        }
        self.commit_with(Vec::new())
    }

    /// Write the paragraph properties the command named, and no other key.
    ///
    /// The command is coarse and the storage is one key per property, so two
    /// peers changing two properties lose neither (`PROSE-030`, `PROSE-092`).
    pub(super) fn set_paragraph_style(
        &mut self,
        id: &TreeID,
        style: &ParagraphStyle,
    ) -> ProseOutcome {
        let tree = doc::blocks(&self.doc);
        let Some(node) = doc::node_of(&tree, id) else {
            return Self::err_no_block(id);
        };
        if style.is_empty() {
            return ProseOutcome::nothing();
        }
        if let Err(e) = doc::write_paragraph_style(&node, style) {
            return Self::err_write(&format!("{e:?}"));
        }
        self.commit_with(Vec::new())
    }

    /// Copy a block's style id and its nine paragraph keys onto another block.
    ///
    /// The split reads it, because the new paragraph carries the source's type
    /// and paragraph style (`PROSE-038`). A heading carries its level too, or
    /// the copy would leave a heading with no level at all.
    pub(super) fn carry_style(&self, source: &TreeID, target: &TreeID) -> Result<(), String> {
        let tree = doc::blocks(&self.doc);
        let Some(target_node) = doc::node_of(&tree, target) else {
            return Err("the new block vanished before its style landed".to_owned());
        };
        if let Some(style_id) = doc::style_of(&tree, source) {
            target_node
                .insert(doc::K_STYLE, style_id.as_str())
                .map_err(|e| format!("{e:?}"))?;
        }
        doc::write_paragraph_style(&target_node, &doc::read_paragraph_style(&tree, source))
            .map_err(|e| format!("{e:?}"))?;
        // The attributes the split must carry. Without them the second half
        // of a heading has no level, and that of a code block no language.
        let level = self.heading_level_of(&crate::events::block_id(*source));
        let lang = self.code_lang_of(&crate::events::block_id(*source));
        if level.is_some() || lang.is_some() {
            let attrs = doc::map_at(&target_node, doc::K_ATTRS)
                .ok_or_else(|| "the attribute map refused a container".to_owned())?;
            if let Some(level) = level {
                attrs
                    .insert(doc::A_LEVEL, i64::from(level))
                    .map_err(|e| format!("{e:?}"))?;
            }
            if let Some(lang) = lang {
                attrs
                    .insert(doc::A_LANG, lang.as_str())
                    .map_err(|e| format!("{e:?}"))?;
            }
        }
        Ok(())
    }

    /// The parentage of a live block, for a test and for the view's outline.
    #[must_use]
    pub fn parent_of(&self, id: &BlockId) -> Option<BlockId> {
        match doc::blocks(&self.doc).parent(tree_id(*id)) {
            Some(TreeParentId::Node(parent)) => Some(crate::events::block_id(parent)),
            _ => None,
        }
    }

    /// Whether every live parentage is one section 4 declares (`PROSE-095`).
    ///
    /// A read, not a guard: the guard is the check each write already made.
    /// A test quantifies over it after a merge, where no single write can.
    #[must_use]
    pub fn parentages_are_declared(&self) -> bool {
        let tree = doc::blocks(&self.doc);
        self.document_order().into_iter().all(|id| {
            let Some(block_type) = doc::block_type_of(&tree, &id) else {
                return false;
            };
            match tree.parent(id) {
                Some(TreeParentId::Node(parent)) => {
                    doc::block_type_of(&tree, &parent).is_some_and(|p| p.accepts(block_type))
                }
                Some(TreeParentId::Root) => block_type.accepted_at_root(),
                _ => false,
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::events::{BlockInit, ProseCommand};
    use crate::logic::ProseState;

    #[test]
    fn a_table_row_is_refused_at_the_root() {
        let mut state = ProseState::new().expect("a document is built");
        // No initializer mints a row, so the guard is read through the nesting
        // table directly: a paragraph under a paragraph is refused the same way.
        let root = state.root_blocks()[0];
        let outcome = state.handle_command(ProseCommand::InsertBlock {
            parent_id: Some(root),
            after_id: None,
            init: BlockInit::Paragraph,
            style_id: None,
        });
        assert!(outcome.update.is_none());
        assert_eq!(state.root_blocks().len(), 1);
    }

    #[test]
    fn a_block_moved_under_its_own_descendant_is_refused() {
        let mut state = ProseState::new().expect("a document is built");
        let root = state.root_blocks()[0];
        let outcome = state.handle_command(ProseCommand::MoveBlock {
            block_id: root,
            new_parent: Some(root),
            after_id: None,
        });
        assert!(outcome.update.is_none());
        assert_eq!(state.parent_of(&root), None);
    }

    #[test]
    fn a_reference_to_a_block_survives_a_move() {
        let mut state = ProseState::new().expect("a document is built");
        let first = state.root_blocks()[0];
        state.handle_command(ProseCommand::InsertBlock {
            parent_id: None,
            after_id: Some(first),
            init: BlockInit::Paragraph,
            style_id: None,
        });
        let second = state.root_blocks()[1];
        state.handle_command(ProseCommand::InsertText {
            block_id: second,
            pos: 0,
            text: "carried".to_owned(),
        });
        state.handle_command(ProseCommand::MoveBlock {
            block_id: second,
            new_parent: None,
            after_id: None,
        });
        // The same id still names the same body after the move.
        assert_eq!(state.text_of(&second).as_deref(), Some("carried"));
        assert!(state.root_blocks().contains(&second));
    }

    /// A table is built through the format rather than through a command: no
    /// `BlockInit` mints one until the tables Story lands. The rule binds the
    /// reference and not the gesture, so the test builds the shape section 4
    /// declares and moves a paragraph through it.
    #[test]
    fn a_reference_survives_an_indent_and_a_move_into_a_table_cell() {
        let mut state = ProseState::new().expect("a document is built");
        let first = state.root_blocks()[0];
        state.handle_command(ProseCommand::InsertBlock {
            parent_id: None,
            after_id: Some(first),
            init: BlockInit::Paragraph,
            style_id: None,
        });
        let carried = state.root_blocks()[1];
        state.handle_command(ProseCommand::InsertText {
            block_id: carried,
            pos: 0,
            text: "carried".to_owned(),
        });

        // An indent: the paragraph becomes a child of a list item.
        let item = state
            .insert_for_test(None, crate::events::BlockType::ListItem)
            .expect("a list item");
        state.handle_command(ProseCommand::MoveBlock {
            block_id: carried,
            new_parent: Some(item),
            after_id: None,
        });
        assert_eq!(state.parent_of(&carried), Some(item));
        assert_eq!(state.text_of(&carried).as_deref(), Some("carried"));

        // And a move into a table cell, through the parentages of section 4.
        let table = state
            .insert_for_test(None, crate::events::BlockType::Table)
            .expect("a table");
        let row = state
            .insert_for_test(Some(table), crate::events::BlockType::TableRow)
            .expect("a row");
        let cell = state
            .insert_for_test(Some(row), crate::events::BlockType::TableCell)
            .expect("a cell");
        state.handle_command(ProseCommand::MoveBlock {
            block_id: carried,
            new_parent: Some(cell),
            after_id: None,
        });
        assert_eq!(state.parent_of(&carried), Some(cell));
        // The same id still names the same body after three moves
        // (`PROSE-085`, `PROSE-086`).
        assert_eq!(state.text_of(&carried).as_deref(), Some("carried"));
        assert!(state.parentages_are_declared());
    }

    #[test]
    fn a_heading_level_is_clamped_to_the_six_the_format_declares() {
        let mut state = ProseState::new().expect("a document is built");
        let first = state.root_blocks()[0];
        state.handle_command(ProseCommand::InsertBlock {
            parent_id: None,
            after_id: Some(first),
            init: BlockInit::Heading { level: 9 },
            style_id: None,
        });
        let heading = state.root_blocks()[1];
        assert_eq!(state.heading_level_of(&heading), Some(6));
        state.handle_command(ProseCommand::SetHeadingLevel {
            block_id: heading,
            level: 0,
        });
        assert_eq!(state.heading_level_of(&heading), Some(1));
    }

    #[test]
    fn a_paragraph_style_naming_no_property_writes_nothing() {
        let mut state = ProseState::new().expect("a document is built");
        let root = state.root_blocks()[0];
        let outcome = state.handle_command(ProseCommand::SetParagraphStyle {
            block_id: root,
            style: crate::events::ParagraphStyle::default(),
        });
        assert!(outcome.update.is_none());
    }

    #[test]
    fn every_parentage_of_a_fresh_document_is_declared() {
        let state = ProseState::new().expect("a document is built");
        assert!(state.parentages_are_declared());
    }

    /// `mov_after` reparents the block under the sibling's own parent. A
    /// nesting check made against the parent the command named therefore used
    /// to guard a parentage the write never used (`PROSE-095`).
    #[test]
    fn an_insert_after_a_table_row_does_not_land_a_paragraph_under_the_table() {
        use crate::events::BlockType;
        let mut state = ProseState::new().expect("a document is built");
        let table = state
            .insert_for_test(None, BlockType::Table)
            .expect("a table");
        let row = state
            .insert_for_test(Some(table), BlockType::TableRow)
            .expect("a row");
        let outcome = state.handle_command(ProseCommand::InsertBlock {
            parent_id: None,
            after_id: Some(row),
            init: BlockInit::Paragraph,
            style_id: None,
        });
        assert!(outcome.update.is_none());
        assert!(state.parentages_are_declared());
    }

    /// The same bypass on the move path: the sibling decided the parent, and
    /// the check was made against the one the command named.
    #[test]
    fn a_move_after_a_table_row_does_not_land_a_paragraph_under_the_table() {
        use crate::events::BlockType;
        let mut state = ProseState::new().expect("a document is built");
        let paragraph = state.root_blocks()[0];
        let table = state
            .insert_for_test(None, BlockType::Table)
            .expect("a table");
        let row = state
            .insert_for_test(Some(table), BlockType::TableRow)
            .expect("a row");
        let outcome = state.handle_command(ProseCommand::MoveBlock {
            block_id: paragraph,
            new_parent: None,
            after_id: Some(row),
        });
        assert!(outcome.update.is_none());
        assert_eq!(state.parent_of(&paragraph), None);
        assert!(state.parentages_are_declared());
    }

    /// A move naming a sibling that is not a live block is refused, rather than
    /// reaching the engine with a position no node answers.
    #[test]
    fn a_move_after_a_block_the_document_does_not_hold_is_refused() {
        let mut state = ProseState::new().expect("a document is built");
        let first = state.root_blocks()[0];
        state.handle_command(ProseCommand::InsertBlock {
            parent_id: None,
            after_id: Some(first),
            init: BlockInit::Paragraph,
            style_id: None,
        });
        let second = state.root_blocks()[1];
        state.handle_command(ProseCommand::DeleteBlock { block_id: second });
        let outcome = state.handle_command(ProseCommand::MoveBlock {
            block_id: first,
            new_parent: None,
            after_id: Some(second),
        });
        assert!(outcome.update.is_none());
    }
}
