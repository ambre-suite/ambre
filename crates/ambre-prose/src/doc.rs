//! The Prose document format: the containers, the keys, and the three reads
//! the engine reaches them through (`ambre-spec-prose-schema.md`).
//!
//! Nothing here holds state. Every function takes a `LoroDoc` or one of its
//! containers and answers a value. The engine of [`crate::logic`] is therefore
//! the one place a document is mutated (`PROSE-FEAT-005`).
//!
//! **Why a root text container per block.** Loro opens a text container from
//! the document by name, not from a map entry. A body nested in the node map
//! would be one more container on the merge path, for an address the code never
//! needs. The name joins the tag `t` and the tree id with `:`. A `TreeID`
//! renders as two decimal numbers joined by `@`, so neither half carries the
//! separator (`PROSE-088`).

use loro::{
    LoroDoc, LoroMap, LoroMovableList, LoroText, LoroTree, LoroValue, TreeID, TreeParentId,
};

use ambre_bus::app::prose::{
    Align, BlockType, LineHeight, LineHeightMode, ListStyle, OrderedStyle, ParagraphStyle,
};

// -----------------------------------------------------------------------------
// Container names and keys (`ambre-spec-prose-schema.md section 3`)
// -----------------------------------------------------------------------------

/// The document register: the schema version, the title, the stamps, the
/// language, and the page sub-map.
pub(crate) const META: &str = ambre_core::schema::SCHEMA_META_MAP;
/// The block hierarchy.
pub(crate) const BLOCKS: &str = "blocks";
/// The user's own paragraph styles. The nine built-in names are compiled in and
/// never stored (`PROSE-093`).
pub(crate) const STYLES: &str = "styles";
/// The comments, the suggestions and the footnotes, one entry each.
pub(crate) const ANNOTS: &str = "annots";

/// The page sub-map of [`META`].
const PAGE: &str = "page";
/// The typographic defaults sub-map of [`META`].
const DEFAULTS: &str = "defaults";

/// The tag a body key joins to a tree id.
const BODY_TAG: &str = "t";
/// The separator of a body key. Neither half can carry it (`PROSE-088`).
const BODY_SEPARATOR: char = ':';

/// The block type discriminant.
pub(crate) const K_TYPE: &str = "t";
/// The style id: a key in [`STYLES`], or one of the nine built-in names.
pub(crate) const K_STYLE: &str = "s";
/// The type-specific attribute map.
pub(crate) const K_ATTRS: &str = "a";

/// The scaffolding marker. `true` on a root paragraph a peer grew because the
/// tree held none, and on no other block (`PROSE-094`).
///
/// Loro refuses to raise a tombstone, so a tree that empties cannot re-grow the
/// block it lost. Each peer grows one of its own, and the merge keeps one. The
/// marker tells a grown block from one the user asked for, so every peer drops
/// the surplus alike.
pub(crate) const K_REGROWN: &str = "rg";

/// The hiding marker, holding the Loro peer id of the session that deleted
/// the block, in decimal (`PROSE-122`). A user's deletion writes it rather
/// than removing the node (`PROSE-118`). An undo removes the key, and the
/// block comes back under its own `TreeID` with its body (`PROSE-120`).
pub(crate) const K_HIDDEN: &str = "gn";

/// The heading level attribute, `1..=6`.
pub(crate) const A_LEVEL: &str = "level";
/// The code block's language. An empty value reads as plain.
pub(crate) const A_LANG: &str = "lang";

/// A list item's marker kind: 0 bullet, 1 ordered, 2 checkbox.
pub(crate) const A_MARKER: &str = "ls";
/// How an ordered item's ordinal is spelled, on an ordered item alone.
pub(crate) const A_SPELLING: &str = "lo";
/// Whether a checkbox item is ticked, on a checkbox item alone.
pub(crate) const A_TICKED: &str = "tc";

/// A table's column widths in points: a movable list, one entry per column
/// (`PROSE-FEAT-044`). A list container rather than a value, so a column
/// inserted and a column resized at the same moment both survive the merge.
pub(crate) const A_COL_WIDTHS: &str = "col_widths";
/// A table's merged regions: a map from the anchor cell's id to a region
/// record (`PROSE-009`). A map container, so two regions written at once are
/// two keys rather than one value overwritten.
pub(crate) const A_MERGED: &str = "merged";
/// The region record's extent: the bottom-right cell's id.
pub(crate) const R_EXTENT: &str = "e";
/// The region record's stamp: the Lamport time it was written at, which is
/// what "the later of two regions" reads (`PROSE-FEAT-254`).
pub(crate) const R_STAMP: &str = "n";

/// A cell's background, as `"#rrggbb"`.
pub(crate) const A_CELL_BG: &str = "bg";
/// The four border widths of a cell, in points.
pub(crate) const A_EDGE_TOP: &str = "bt";
pub(crate) const A_EDGE_BOTTOM: &str = "bb";
pub(crate) const A_EDGE_LEFT: &str = "bl";
pub(crate) const A_EDGE_RIGHT: &str = "br";
/// The border colour of a cell, as `"#rrggbb"`.
pub(crate) const A_EDGE_COLOR: &str = "bc";
/// A cell's padding, in points.
pub(crate) const A_PADDING: &str = "pad";
/// A cell's vertical alignment: 0 top, 1 middle, 2 bottom.
pub(crate) const A_VALIGN: &str = "va";

/// The deepest heading level a table of contents lists, one to six.
pub(crate) const A_MAX_LEVEL: &str = "max_level";
/// Whether a table of contents numbers its entries.
pub(crate) const A_NUMBERED: &str = "numbered";

/// The width a column takes when the command that makes it names none
/// (`PROSE-FEAT-232`): one inch, in points.
pub const DEFAULT_COLUMN_WIDTH_PT: f32 = 72.0;

/// The nine paragraph-style keys, flat in the node map, one key per property
/// (`PROSE-030`).
pub(crate) const K_ALIGN: &str = "align";
pub(crate) const K_INDENT_FIRST: &str = "indent_first";
pub(crate) const K_INDENT_LEFT: &str = "indent_left";
pub(crate) const K_INDENT_RIGHT: &str = "indent_right";
pub(crate) const K_SPACE_BEFORE: &str = "space_before";
pub(crate) const K_SPACE_AFTER: &str = "space_after";
pub(crate) const K_LINE_HEIGHT: &str = "line_height";
pub(crate) const K_KEEP_TOGETHER: &str = "keep_together";
pub(crate) const K_KEEP_WITH_NEXT: &str = "keep_with_next";

/// The two keys of the `line_height` sub-map.
const LH_MODE: &str = "mode";
const LH_VALUE: &str = "value";

/// The nine built-in style names. A document written by an older client renders
/// them from the client rather than from stale copies of its own
/// (`PROSE-093`).
pub const BUILT_IN_STYLES: [&str; 9] = [
    "body",
    "heading-1",
    "heading-2",
    "heading-3",
    "heading-4",
    "heading-5",
    "heading-6",
    "code",
    "quote",
];

/// Whether `style_id` is one of the nine names every build compiles in.
#[must_use]
pub fn is_built_in_style(style_id: &ambre_bus::app::prose::StyleId) -> bool {
    BUILT_IN_STYLES.contains(&style_id.0.as_str())
}

// -----------------------------------------------------------------------------
// The body key (`PROSE-088`)
// -----------------------------------------------------------------------------

/// The name of a block's body text container.
#[must_use]
pub fn body_key(id: &TreeID) -> String {
    format!("{BODY_TAG}{BODY_SEPARATOR}{id}")
}

/// The pair a body key joins, or `None` for a name that is not one.
///
/// The inverse of [`body_key`], and what makes the key a pair rather than an
/// opaque string (`PROSE-088`). A tree id renders as two decimal numbers joined
/// by `@`, so the split takes the first separator and the rest is the id.
#[must_use]
pub fn parse_body_key(key: &str) -> Option<TreeID> {
    let (tag, id) = key.split_once(BODY_SEPARATOR)?;
    if tag != BODY_TAG {
        return None;
    }
    TreeID::try_from(id).ok()
}

/// A leaf block's body text container.
///
/// One of the three functions the engine reaches the format through
/// (`features/ambre-spec-prose-engine.md section 3`). It opens the container
/// whether or not the block holds text yet. Loro mints an empty one on first
/// read, and an empty body is the state a fresh paragraph is in.
#[must_use]
pub fn get_block_text(doc: &LoroDoc, id: &TreeID) -> LoroText {
    doc.get_text(body_key(id))
}

// -----------------------------------------------------------------------------
// Block reads
// -----------------------------------------------------------------------------

/// The block tree.
#[must_use]
pub(crate) fn blocks(doc: &LoroDoc) -> LoroTree {
    doc.get_tree(BLOCKS)
}

/// A live block's node map, or `None` when the id names no live block.
///
/// A tombstoned block answers `None`. It is gone as far as every command is
/// concerned, and no path of this module reaches one.
#[must_use]
pub(crate) fn node_of(tree: &LoroTree, id: &TreeID) -> Option<LoroMap> {
    if tree.is_node_deleted(id).unwrap_or(true) || is_hidden(tree, id) {
        return None;
    }
    tree.get_meta(*id).ok()
}

/// Whether the block, or a block on its parent chain, carries the hiding
/// marker (`PROSE-119`). The walk is as long as the nesting is deep.
#[must_use]
pub(crate) fn is_hidden(tree: &LoroTree, id: &TreeID) -> bool {
    let mut current = *id;
    loop {
        if carries_hiding(tree, &current) {
            return true;
        }
        match tree.parent(current) {
            Some(TreeParentId::Node(parent)) => current = parent,
            _ => return false,
        }
    }
}

/// Whether the block itself carries the hiding marker.
fn carries_hiding(tree: &LoroTree, id: &TreeID) -> bool {
    tree.get_meta(*id)
        .is_ok_and(|node| node.get(K_HIDDEN).is_some())
}

/// Hide a block and the subtree under it, in place of removing it
/// (`PROSE-118`). The marker names the session that hid it, the one peer whose
/// undo can bring it back.
///
/// # Errors
/// The CRDT engine's refusal, as text.
pub(crate) fn hide_block(doc: &LoroDoc, tree: &LoroTree, id: &TreeID) -> Result<(), String> {
    let node = tree.get_meta(*id).map_err(|e| format!("{e:?}"))?;
    node.insert(K_HIDDEN, doc.peer_id().to_string())
        .map_err(|e| format!("{e:?}"))
}

/// The type a live block carries, or `None` when the id names no live block or
/// the discriminant is one this build does not know.
#[must_use]
pub fn block_type_of(tree: &LoroTree, id: &TreeID) -> Option<BlockType> {
    let node = node_of(tree, id)?;
    let code = u8::try_from(read_i64(&node, K_TYPE)?).ok()?;
    BlockType::from_code(code)
}

/// The level a live heading carries, exactly as the document holds it.
///
/// Raw on purpose. An import is the only way a level outside one to six reaches
/// a document, and it stays as it came (`PROSE-FEAT-091`). The clamp belongs to
/// the numbering, which is the one reader that cannot work outside the range
/// (`PROSE-FEAT-029`). `None` for a block that is not a live heading, or whose
/// level is missing or unreadable.
#[must_use]
pub(crate) fn heading_level_of(tree: &LoroTree, id: &TreeID) -> Option<u8> {
    if block_type_of(tree, id) != Some(BlockType::Heading) {
        return None;
    }
    let attrs = map_read(&node_of(tree, id)?, K_ATTRS)?;
    u8::try_from(read_i64(&attrs, A_LEVEL)?).ok()
}

/// The marker a live list item carries.
///
/// A block that is no live list item answers `None`. So does one whose marker
/// is missing or unreadable.
///
/// A block this build reads no marker on breaks a numbering run, the way a
/// paragraph does (`PROSE-FEAT-026`). An unknown marker is not a run to join.
#[must_use]
pub(crate) fn list_style_of(tree: &LoroTree, id: &TreeID) -> Option<ListStyle> {
    if block_type_of(tree, id) != Some(BlockType::ListItem) {
        return None;
    }
    let attrs = map_read(&node_of(tree, id)?, K_ATTRS)?;
    match u8::try_from(read_i64(&attrs, A_MARKER)?).ok()? {
        0 => Some(ListStyle::Bullet),
        1 => {
            let code = u8::try_from(read_i64(&attrs, A_SPELLING)?).ok()?;
            OrderedStyle::from_code(code).map(ListStyle::Ordered)
        }
        2 => Some(ListStyle::Todo(read_bool(&attrs, A_TICKED)?)),
        _ => None,
    }
}

/// A live block's attribute map, or `None` when it carries none. Read-only: it
/// never mints the map it looked for (`PROSE-079`).
#[must_use]
pub(crate) fn attrs_read(tree: &LoroTree, id: &TreeID) -> Option<LoroMap> {
    map_read(&node_of(tree, id)?, K_ATTRS)
}

/// The two settings a live table of contents carries: the deepest level it
/// lists and whether it numbers its entries.
///
/// A missing or unreadable depth reads as six and a missing switch as off, so
/// a block an older client wrote still lists something.
#[must_use]
pub(crate) fn toc_settings(tree: &LoroTree, id: &TreeID) -> Option<(u8, bool)> {
    if block_type_of(tree, id) != Some(BlockType::Toc) {
        return None;
    }
    let attrs = attrs_read(tree, id);
    let level = attrs
        .as_ref()
        .and_then(|a| read_i64(a, A_MAX_LEVEL))
        .and_then(|n| u8::try_from(n).ok())
        .unwrap_or(6);
    let numbered = attrs
        .as_ref()
        .and_then(|a| read_bool(a, A_NUMBERED))
        .unwrap_or(false);
    Some((level, numbered))
}

/// A table's column widths list, or `None` when the table carries none.
#[must_use]
pub(crate) fn col_widths_read(tree: &LoroTree, table: &TreeID) -> Option<LoroMovableList> {
    attrs_read(tree, table)?
        .get(A_COL_WIDTHS)
        .and_then(|value| value.into_container().ok())
        .and_then(|container| container.into_movable_list().ok())
}

/// A table's column widths, in points, in column order.
#[must_use]
pub(crate) fn col_widths(tree: &LoroTree, table: &TreeID) -> Vec<f32> {
    let Some(list) = col_widths_read(tree, table) else {
        return Vec::new();
    };
    (0..list.len())
        .map(
            |index| match list.get(index).and_then(|v| v.into_value().ok()) {
                Some(LoroValue::Double(n)) => n as f32,
                Some(LoroValue::I64(n)) => n as f32,
                _ => DEFAULT_COLUMN_WIDTH_PT,
            },
        )
        .collect()
}

/// A table's region map, or `None` when the table carries none.
#[must_use]
pub(crate) fn merged_read(tree: &LoroTree, table: &TreeID) -> Option<LoroMap> {
    map_read(&attrs_read(tree, table)?, A_MERGED)
}

/// One merged region as the document holds it: two corners and a stamp,
/// never a span (`PROSE-009`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Region {
    pub anchor: TreeID,
    pub extent: TreeID,
    pub stamp: ambre_core::LamportStamp,
}

/// Every region a table's map holds, in anchor order. An entry whose record
/// does not read is left out: it names no rectangle.
#[must_use]
pub(crate) fn regions(tree: &LoroTree, table: &TreeID) -> Vec<Region> {
    let Some(map) = merged_read(tree, table) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    map.for_each(|key, value| {
        let Ok(LoroValue::Map(record)) = value.into_value() else {
            return;
        };
        let Ok(anchor) = TreeID::try_from(key) else {
            return;
        };
        let extent = match record.get(R_EXTENT) {
            Some(LoroValue::String(text)) => TreeID::try_from(text.as_str()).ok(),
            _ => None,
        };
        let stamp = match record.get(R_STAMP) {
            Some(LoroValue::I64(n)) => ambre_core::LamportStamp::from_i64(*n),
            _ => ambre_core::LamportStamp::default(),
        };
        if let Some(extent) = extent {
            out.push(Region {
                anchor,
                extent,
                stamp,
            });
        }
    });
    out.sort_by_key(|region| region.anchor);
    out
}

/// The style id a block carries, or `None` when it carries none. An absent key
/// means the block type's own built-in style (`PROSE-096`).
#[must_use]
pub fn style_of(tree: &LoroTree, id: &TreeID) -> Option<String> {
    let node = node_of(tree, id)?;
    read_string(&node, K_STYLE)
}

/// The live children of a parent, or of the root when `parent` is `None`.
#[must_use]
pub(crate) fn children_of(tree: &LoroTree, parent: Option<TreeID>) -> Vec<TreeID> {
    let parent: TreeParentId = parent.into();
    let mut children = tree.children(parent).unwrap_or_default();
    // A hidden block reads as gone (`PROSE-119`).
    children.retain(|id| !carries_hiding(tree, id));
    children
}

/// The live root blocks of the body tree.
#[must_use]
pub(crate) fn roots(tree: &LoroTree) -> Vec<TreeID> {
    let mut roots = children_of(tree, None);
    // A band or a note is a root and no part of the flow (`PROSE-006`,
    // `PROSE-FEAT-305`).
    roots.retain(|id| !is_aside(tree, id));
    roots
}

/// The root blocks a peer grew for want of any, still empty and still childless,
/// sorted by `TreeID` (`PROSE-094`).
///
/// The merge keeps the first of them and drops the rest. Every peer reads that
/// alike, because the marker and the order are both in the document.
#[must_use]
pub(crate) fn grown_empty_roots(doc: &LoroDoc, tree: &LoroTree) -> Vec<TreeID> {
    let mut ids: Vec<TreeID> = roots(tree)
        .into_iter()
        .filter(|id| {
            node_of(tree, id).is_some_and(|node| read_bool(&node, K_REGROWN) == Some(true))
        })
        .filter(|id| get_block_text(doc, id).is_empty())
        .filter(|id| children_of(tree, Some(*id)).is_empty())
        .collect();
    ids.sort();
    ids
}

/// Whether `ancestor` sits on the parent chain of `id`, or is `id` itself.
///
/// A move reads it to refuse a block moved under its own descendant. Such a
/// move takes the subtree out of the tree and into a cycle.
#[must_use]
pub(crate) fn is_ancestor_of(tree: &LoroTree, ancestor: &TreeID, id: &TreeID) -> bool {
    let mut current = *id;
    loop {
        if current == *ancestor {
            return true;
        }
        match tree.parent(current) {
            Some(TreeParentId::Node(parent)) => current = parent,
            _ => return false,
        }
    }
}

/// Whether a block of `child` type may sit under `parent`, per the nesting
/// table of `ambre-spec-prose-schema.md section 4` (`PROSE-095`).
///
/// A parent that names no live block refuses every child, and so does one whose
/// discriminant this build does not know. An unknown shape is not one to write
/// into.
#[must_use]
pub(crate) fn nesting_allows(tree: &LoroTree, parent: Option<TreeID>, child: BlockType) -> bool {
    match parent {
        None => child.accepted_at_root(),
        Some(parent) => block_type_of(tree, &parent).is_some_and(|p| p.accepts(child)),
    }
}

// -----------------------------------------------------------------------------
// The paragraph style, one key per property (`PROSE-030`)
// -----------------------------------------------------------------------------

/// The sub-map a key holds, opened once and never replaced.
///
/// `LoroMap::insert_container` mints a fresh container and drops the one the key
/// held, taking every other key with it. Every nested register here is written
/// by more than one path: the page format beside the margins, one block
/// attribute beside another. So a write reaches the map that is already there,
/// and mints one only when the key is empty (`PROSE-092`).
pub(crate) fn map_at(parent: &LoroMap, key: &str) -> Option<LoroMap> {
    let existing = parent
        .get(key)
        .and_then(|value| value.into_container().ok())
        .and_then(|container| container.into_map().ok());
    match existing {
        Some(map) => Some(map),
        None => parent
            .insert_container(key, LoroMap::new())
            .map_err(|e| tracing::warn!("Prose: the register {key} refused a container: {e}"))
            .ok(),
    }
}

/// The movable list a key holds, opened once and never replaced, for the same
/// reason as [`map_at`].
pub(crate) fn movable_list_at(parent: &LoroMap, key: &str) -> Option<LoroMovableList> {
    let existing = parent
        .get(key)
        .and_then(|value| value.into_container().ok())
        .and_then(|container| container.into_movable_list().ok());
    match existing {
        Some(list) => Some(list),
        None => parent
            .insert_container(key, LoroMovableList::new())
            .map_err(|e| tracing::warn!("Prose: the list {key} refused a container: {e}"))
            .ok(),
    }
}

/// The `meta` register, or `None` when the document has none.
///
/// A reader takes this path and never `get_map`, which attaches the root
/// container it did not find. Laying the document out writes nothing to it
/// (`PROSE-079`).
#[must_use]
pub(crate) fn meta_read(doc: &LoroDoc) -> Option<LoroMap> {
    doc.get_by_str_path(META)
        .and_then(|value| value.into_container().ok())
        .and_then(|container| container.into_map().ok())
}

/// The block tree, or `None` when the document has none, for the same reason
/// as [`meta_read`].
#[must_use]
pub(crate) fn blocks_read(doc: &LoroDoc) -> Option<LoroTree> {
    doc.get_by_str_path(BLOCKS)
        .and_then(|value| value.into_container().ok())
        .and_then(|container| container.into_tree().ok())
}

/// The map a key already holds, or `None` when the key is empty.
///
/// The read-only half of [`map_at`]. A reader must never mint the container it
/// went looking for: laying the document out writes nothing to it
/// (`PROSE-079`).
#[must_use]
pub(crate) fn map_read(parent: &LoroMap, key: &str) -> Option<LoroMap> {
    parent
        .get(key)
        .and_then(|value| value.into_container().ok())
        .and_then(|container| container.into_map().ok())
}

/// Write the properties the caller named, and no other key (`PROSE-096`).
///
/// `None` is "leave this property alone". Two peers writing two different
/// properties therefore touch two different keys, and the merge keeps both
/// (`PROSE-092`).
///
/// # Errors
/// The CRDT engine's own refusal of a write.
pub(crate) fn write_paragraph_style(
    node: &LoroMap,
    style: &ParagraphStyle,
) -> Result<(), loro::LoroError> {
    if let Some(align) = style.align {
        node.insert(K_ALIGN, i64::from(align.code()))?;
    }
    for (key, value) in [
        (K_INDENT_FIRST, style.indent_first),
        (K_INDENT_LEFT, style.indent_left),
        (K_INDENT_RIGHT, style.indent_right),
        (K_SPACE_BEFORE, style.space_before),
        (K_SPACE_AFTER, style.space_after),
    ] {
        if let Some(points) = value {
            node.insert(key, f64::from(points))?;
        }
    }
    for (key, value) in [
        (K_KEEP_TOGETHER, style.keep_together),
        (K_KEEP_WITH_NEXT, style.keep_with_next),
    ] {
        if let Some(flag) = value {
            node.insert(key, flag)?;
        }
    }
    if let Some(line_height) = style.line_height {
        let Some(map) = map_at(node, K_LINE_HEIGHT) else {
            return Err(loro::LoroError::NotFoundError(
                "the line-height register refused a container".into(),
            ));
        };
        map.insert(LH_MODE, i64::from(line_height.mode.code()))?;
        map.insert(LH_VALUE, f64::from(line_height.value))?;
    }
    Ok(())
}

/// The properties of `wanted` whose value is not the one already stored
/// (`PROSE-FEAT-025`).
///
/// A property the command did not carry stays absent, and so does one whose
/// value is already in the document. The command is coarse and the storage is
/// per key, which is what lets two authors change two properties of one
/// paragraph at once (`PROSE-FEAT-176`).
#[must_use]
pub(crate) fn changed_paragraph_style(
    stored: &ParagraphStyle,
    wanted: &ParagraphStyle,
) -> ParagraphStyle {
    /// A property survives the diff when it was asked for and differs.
    fn kept<T: PartialEq + Copy>(stored: Option<T>, wanted: Option<T>) -> Option<T> {
        wanted.filter(|value| stored != Some(*value))
    }
    ParagraphStyle {
        align: kept(stored.align, wanted.align),
        indent_first: kept(stored.indent_first, wanted.indent_first),
        indent_left: kept(stored.indent_left, wanted.indent_left),
        indent_right: kept(stored.indent_right, wanted.indent_right),
        space_before: kept(stored.space_before, wanted.space_before),
        space_after: kept(stored.space_after, wanted.space_after),
        line_height: kept(stored.line_height, wanted.line_height),
        keep_together: kept(stored.keep_together, wanted.keep_together),
        keep_with_next: kept(stored.keep_with_next, wanted.keep_with_next),
    }
}

/// The paragraph style a block carries. An absent key is an unset property, and
/// the default is what the cascade resolves (`PROSE-096`).
#[must_use]
pub fn read_paragraph_style(tree: &LoroTree, id: &TreeID) -> ParagraphStyle {
    let Some(node) = node_of(tree, id) else {
        return ParagraphStyle::default();
    };
    ParagraphStyle {
        align: read_i64(&node, K_ALIGN)
            .and_then(|n| u8::try_from(n).ok())
            .and_then(Align::from_code),
        indent_first: read_f32(&node, K_INDENT_FIRST),
        indent_left: read_f32(&node, K_INDENT_LEFT),
        indent_right: read_f32(&node, K_INDENT_RIGHT),
        space_before: read_f32(&node, K_SPACE_BEFORE),
        space_after: read_f32(&node, K_SPACE_AFTER),
        line_height: read_line_height(&node),
        keep_together: read_bool(&node, K_KEEP_TOGETHER),
        keep_with_next: read_bool(&node, K_KEEP_WITH_NEXT),
    }
}

fn read_line_height(node: &LoroMap) -> Option<LineHeight> {
    let map = node
        .get(K_LINE_HEIGHT)
        .and_then(|v| v.into_container().ok())
        .and_then(|c| c.into_map().ok())?;
    let mode = u8::try_from(read_i64(&map, LH_MODE)?)
        .ok()
        .and_then(LineHeightMode::from_code)?;
    let value = read_f64(&map, LH_VALUE)?;
    Some(LineHeight {
        mode,
        value: value as f32,
    })
}

// -----------------------------------------------------------------------------
// The page format (`PROSE-032`)
// -----------------------------------------------------------------------------

/// The two page orientations, and the one rule that binds each to its two
/// dimensions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Orientation {
    Portrait,
    Landscape,
}

impl Orientation {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Portrait => "portrait",
            Self::Landscape => "landscape",
        }
    }

    /// Whether the two dimensions agree with the orientation. A square page
    /// agrees with both, because neither dimension is the greater one.
    #[must_use]
    pub fn agrees_with(self, width_mm: f32, height_mm: f32) -> bool {
        match self {
            Self::Portrait => width_mm <= height_mm,
            Self::Landscape => width_mm >= height_mm,
        }
    }
}

/// A4 portrait, in millimetres: the page a fresh document opens on.
const A4_WIDTH_MM: f32 = 210.0;
const A4_HEIGHT_MM: f32 = 297.0;
/// The four margins of a fresh document, in millimetres.
const DEFAULT_MARGIN_MM: f32 = 20.0;

/// The two text defaults of a fresh document that carry a number.
///
/// The family is a value, not a key. It lives in `DocDefaults::default` alone,
/// because a string constant in this module is read as a schema key
/// (`SCHEMA-018`).
const DEFAULT_FONT_SIZE_PT: f32 = 11.0;
const DEFAULT_LINE_HEIGHT: f32 = 1.5;

/// The page format a document declares, in millimetres.
///
/// Read out of `meta/page` and converted to pixels by the layout, which is the
/// one place a page number or a measured height exists (`PROSE-065`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PageFormat {
    pub width_mm: f32,
    pub height_mm: f32,
    pub margin_top_mm: f32,
    pub margin_bottom_mm: f32,
    pub margin_left_mm: f32,
    pub margin_right_mm: f32,
}

impl Default for PageFormat {
    /// A4 portrait with the four default margins: what `seed` writes.
    fn default() -> Self {
        Self {
            width_mm: A4_WIDTH_MM,
            height_mm: A4_HEIGHT_MM,
            margin_top_mm: DEFAULT_MARGIN_MM,
            margin_bottom_mm: DEFAULT_MARGIN_MM,
            margin_left_mm: DEFAULT_MARGIN_MM,
            margin_right_mm: DEFAULT_MARGIN_MM,
        }
    }
}

/// The three document defaults `meta/defaults` holds: what a block that names
/// no style of its own is measured with.
///
/// The full four-level cascade is the inline-styles feature's
/// (`features/ambre-spec-prose-inline-styles.md`). This is its bottom level,
/// which is what pagination needs to shape a run.
#[derive(Clone, Debug, PartialEq)]
pub struct DocDefaults {
    pub font_family: String,
    pub font_size_pt: f32,
    /// A multiple of the font size, never an absolute length.
    pub line_height: f32,
}

impl Default for DocDefaults {
    /// What `seed` writes.
    fn default() -> Self {
        Self {
            font_family: "Inter".to_string(),
            font_size_pt: DEFAULT_FONT_SIZE_PT,
            line_height: DEFAULT_LINE_HEIGHT,
        }
    }
}

/// The defaults `meta/defaults` holds.
///
/// A key that is missing, or that holds a number no text can be set in, falls
/// back for that key alone. A size of zero would measure every block as a
/// zero-height one, and place a whole document on one page.
#[must_use]
pub fn read_defaults(doc: &LoroDoc) -> DocDefaults {
    let fallback = DocDefaults::default();
    let Some(defaults) = meta_read(doc).and_then(|meta| map_read(&meta, DEFAULTS)) else {
        return fallback;
    };
    let read = |key: &str, default: f32| -> f32 {
        read_f32(&defaults, key)
            .filter(|number| number.is_finite() && *number > 0.0)
            .unwrap_or(default)
    };
    DocDefaults {
        font_family: read_string(&defaults, "font_family")
            .filter(|name| !name.is_empty())
            .unwrap_or(fallback.font_family),
        font_size_pt: read("font_size_pt", fallback.font_size_pt),
        line_height: read("line_height", fallback.line_height),
    }
}

/// The page format `meta/page` holds.
///
/// A key that is missing, or that holds a number no page can have, falls back
/// to the default for that key alone. A document with one bad margin lays out
/// on the right sheet rather than on none.
#[must_use]
pub fn read_page_format(doc: &LoroDoc) -> PageFormat {
    let fallback = PageFormat::default();
    let Some(page) = meta_read(doc).and_then(|meta| map_read(&meta, PAGE)) else {
        return fallback;
    };
    let read = |key: &str, default: f32| -> f32 {
        read_f32(&page, key)
            .filter(|number| number.is_finite() && *number >= 0.0)
            .unwrap_or(default)
    };
    let width_mm = read("width_mm", fallback.width_mm);
    let height_mm = read("height_mm", fallback.height_mm);
    PageFormat {
        width_mm: if width_mm > 0.0 {
            width_mm
        } else {
            fallback.width_mm
        },
        height_mm: if height_mm > 0.0 {
            height_mm
        } else {
            fallback.height_mm
        },
        margin_top_mm: read("margin_top_mm", fallback.margin_top_mm),
        margin_bottom_mm: read("margin_bottom_mm", fallback.margin_bottom_mm),
        margin_left_mm: read("margin_left_mm", fallback.margin_left_mm),
        margin_right_mm: read("margin_right_mm", fallback.margin_right_mm),
    }
}

/// Write the page format, and refuse an orientation its two dimensions
/// contradict (`PROSE-032`).
///
/// The one place the three keys are written together, so no path can leave the
/// register holding a portrait page wider than it is tall.
///
/// # Errors
/// `false` when the orientation contradicts the dimensions, or when the CRDT
/// engine refuses the write. The register is then left as it was.
pub(crate) fn write_page_format(
    doc: &LoroDoc,
    orientation: Orientation,
    width_mm: f32,
    height_mm: f32,
) -> bool {
    if !width_mm.is_finite() || !height_mm.is_finite() || width_mm <= 0.0 || height_mm <= 0.0 {
        return false;
    }
    if !orientation.agrees_with(width_mm, height_mm) {
        return false;
    }
    // The register the document already holds: minting a fresh one would drop
    // the four margins `seed` wrote beside these three keys.
    let Some(page) = map_at(&doc.get_map(META), PAGE) else {
        return false;
    };
    page.insert("orientation", orientation.as_str()).is_ok()
        && page.insert("width_mm", f64::from(width_mm)).is_ok()
        && page.insert("height_mm", f64::from(height_mm)).is_ok()
}

// -----------------------------------------------------------------------------
// Construction
// -----------------------------------------------------------------------------

/// Seed a fresh document: the registers every reader expects, and the one root
/// paragraph the tree is never without (`PROSE-027`).
///
/// The caller commits. Everything written here is scaffolding, so it rides the
/// commit that precedes any user edit and no document ever persists without it.
///
/// Returns the root paragraph, or `None` when the CRDT engine refused the
/// scaffolding. A caller that meets `None` holds a document it must not open.
pub(crate) fn seed(doc: &LoroDoc) -> Option<TreeID> {
    let meta = doc.get_map(META);
    meta.insert("title", "").ok()?;
    meta.insert("lang", "en").ok()?;
    meta.insert("pageless", false).ok()?;
    if !write_page_format(doc, Orientation::Portrait, A4_WIDTH_MM, A4_HEIGHT_MM) {
        return None;
    }
    let page = map_at(&meta, PAGE)?;
    for key in [
        "margin_top_mm",
        "margin_bottom_mm",
        "margin_left_mm",
        "margin_right_mm",
    ] {
        page.insert(key, f64::from(DEFAULT_MARGIN_MM)).ok()?;
    }
    let defaults = map_at(&meta, DEFAULTS)?;
    let text = DocDefaults::default();
    defaults
        .insert("font_family", text.font_family.as_str())
        .ok()?;
    defaults
        .insert("font_size_pt", f64::from(text.font_size_pt))
        .ok()?;
    defaults
        .insert("line_height", f64::from(text.line_height))
        .ok()?;
    // The two registers no seeded key belongs to are opened all the same, so
    // every peer addresses one container rather than minting a rival one.
    let _ = doc.get_map(STYLES);
    let _ = doc.get_map(ANNOTS);
    grow_root(doc)
}

/// Grow one root paragraph, marked as grown (`PROSE-094`).
///
/// Called when the tree holds no root at all: at construction, and after a
/// deletion that emptied it.
pub(crate) fn grow_root(doc: &LoroDoc) -> Option<TreeID> {
    let id = create_block(doc, None, BlockType::Paragraph)?;
    let tree = blocks(doc);
    node_of(&tree, &id)?.insert(K_REGROWN, true).ok()?;
    Some(id)
}

/// Mint one block of `block_type` under `parent`, and write the two keys every
/// block carries.
///
/// The caller has already checked the nesting rule; this writes. A block with a
/// body opens its text container here, so the body exists before the first
/// keystroke reaches it.
pub(crate) fn create_block(
    doc: &LoroDoc,
    parent: Option<TreeID>,
    block_type: BlockType,
) -> Option<TreeID> {
    let tree = blocks(doc);
    let parent: TreeParentId = parent.into();
    let id = tree
        .create(parent)
        .map_err(|e| tracing::warn!("Prose: the tree refused a block: {e}"))
        .ok()?;
    let node = tree.get_meta(id).ok()?;
    node.insert(K_TYPE, i64::from(block_type.code())).ok()?;
    if block_type.has_body() {
        // Opening the body attaches the container to the document, so a peer
        // that receives the block receives an empty body with it.
        let _ = get_block_text(doc, &id);
    }
    Some(id)
}

// -----------------------------------------------------------------------------
// The bands (`features/ambre-spec-prose-headers-footers.md`)
// -----------------------------------------------------------------------------

/// The band marker, `true` on a block that holds a header or a footer. Such a
/// block is a root of the block tree kept out of the body flow (`PROSE-006`).
pub(crate) const K_BAND: &str = "hb";
/// The four band registers of [`META`].
pub(crate) const HEADER: &str = "header";
pub(crate) const FOOTER: &str = "footer";
pub(crate) const HEADER_FIRST: &str = "header_first";
pub(crate) const FOOTER_FIRST: &str = "footer_first";
/// The one setting that turns both first-page bands on (`PROSE-FEAT-063`).
pub(crate) const DIFFERENT_FIRST_PAGE: &str = "different_first_page";
/// Whether a side is shown, in the header and footer registers alone.
pub(crate) const BAND_ENABLED: &str = "enabled";
/// The band block's `TreeID`, as the string Loro prints it.
pub(crate) const BAND_BLOCK: &str = "block_id";
/// The last-modified stamp, Unix milliseconds by the clock of the peer that
/// wrote last (`PROSE-FEAT-066`).
pub(crate) const UPDATED_AT: &str = "updated_at";

/// One band: the block that holds its text and its alignment. Nothing of the
/// text is here (`PROSE-FEAT-281`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Band {
    pub block: Option<TreeID>,
    pub align: Align,
}

impl Default for Band {
    fn default() -> Self {
        Self {
            block: None,
            align: Align::Left,
        }
    }
}

/// The four bands and the three settings over them, as the document holds
/// them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Bands {
    pub header: Band,
    pub footer: Band,
    pub header_first: Band,
    pub footer_first: Band,
    pub header_enabled: bool,
    pub footer_enabled: bool,
    pub different_first_page: bool,
}

impl Bands {
    /// Whether `block` is the text of one of the four bands.
    #[must_use]
    pub fn holds(&self, block: &TreeID) -> bool {
        [
            self.header,
            self.footer,
            self.header_first,
            self.footer_first,
        ]
        .iter()
        .any(|band| band.block.as_ref() == Some(block))
    }

    /// The header and the footer drawn on one zero-based page. The first
    /// page's own pair while the setting is on, the regular pair otherwise
    /// (`PROSE-FEAT-169`, `PROSE-UI-133`). A hidden side draws nothing.
    #[must_use]
    pub fn on_page(&self, page: u32) -> (Option<Band>, Option<Band>) {
        let first = page == 0 && self.different_first_page;
        let (header, footer) = if first {
            (self.header_first, self.footer_first)
        } else {
            (self.header, self.footer)
        };
        let drawn = |band: Band, shown: bool| (shown && band.block.is_some()).then_some(band);
        (
            drawn(header, self.header_enabled),
            drawn(footer, self.footer_enabled),
        )
    }

    /// The register key of the band whose block is `id`, if one names it.
    #[must_use]
    pub(crate) fn register_of(&self, id: &TreeID) -> Option<&'static str> {
        [
            (self.header, HEADER),
            (self.footer, FOOTER),
            (self.header_first, HEADER_FIRST),
            (self.footer_first, FOOTER_FIRST),
        ]
        .into_iter()
        .find(|(band, _)| band.block == Some(*id))
        .map(|(_, key)| key)
    }
}

/// The spelling of a band alignment in the document.
#[must_use]
pub(crate) fn band_align_name(align: Align) -> Option<&'static str> {
    match align {
        Align::Left => Some("left"),
        Align::Center => Some("center"),
        Align::Right => Some("right"),
        Align::Justify => None,
    }
}

fn read_band(meta: &LoroMap, key: &str) -> Band {
    let Some(map) = map_read(meta, key) else {
        return Band::default();
    };
    let block = read_string(&map, BAND_BLOCK).and_then(|s| TreeID::try_from(s.as_str()).ok());
    let align = match read_string(&map, K_ALIGN).as_deref() {
        Some("center") => Align::Center,
        Some("right") => Align::Right,
        _ => Align::Left,
    };
    Band { block, align }
}

/// The bands as the document holds them. A document written before them
/// reads as two hidden sides and no block (`PROSE-FEAT-064`).
#[must_use]
pub fn read_bands(doc: &LoroDoc) -> Bands {
    let Some(meta) = meta_read(doc) else {
        return Bands::default();
    };
    let enabled = |key: &str| {
        map_read(&meta, key).is_some_and(|map| read_bool(&map, BAND_ENABLED) == Some(true))
    };
    let tree = blocks_read(doc);
    let live = |band: Band| Band {
        block: band.block.filter(|id| {
            tree.as_ref()
                .is_some_and(|tree| tree.contains(*id) && node_of(tree, id).is_some())
        }),
        ..band
    };
    Bands {
        header: live(read_band(&meta, HEADER)),
        footer: live(read_band(&meta, FOOTER)),
        header_first: live(read_band(&meta, HEADER_FIRST)),
        footer_first: live(read_band(&meta, FOOTER_FIRST)),
        header_enabled: enabled(HEADER),
        footer_enabled: enabled(FOOTER),
        different_first_page: read_bool(&meta, DIFFERENT_FIRST_PAGE) == Some(true),
    }
}

/// Whether a block holds a band rather than body text.
#[must_use]
pub(crate) fn is_band(tree: &LoroTree, id: &TreeID) -> bool {
    node_of(tree, id).is_some_and(|node| read_bool(&node, K_BAND) == Some(true))
}

/// Whether a block holds a footnote's text rather than body text.
#[must_use]
pub(crate) fn is_note(tree: &LoroTree, id: &TreeID) -> bool {
    node_of(tree, id).is_some_and(|node| read_bool(&node, K_NOTE) == Some(true))
}

/// Whether a block stands outside the body flow: a band or a note. Such a
/// block is one paragraph a register or an entry names, so no structural
/// command may split, move or delete it.
#[must_use]
pub(crate) fn is_aside(tree: &LoroTree, id: &TreeID) -> bool {
    node_of(tree, id).is_some_and(|node| {
        read_bool(&node, K_BAND) == Some(true) || read_bool(&node, K_NOTE) == Some(true)
    })
}

/// The last-modified stamp, or `None` before any peer wrote one.
#[must_use]
pub fn read_updated_at(doc: &LoroDoc) -> Option<i64> {
    meta_read(doc).and_then(|meta| read_i64(&meta, UPDATED_AT))
}

/// The document title, empty when none was set.
#[must_use]
pub fn read_title(doc: &LoroDoc) -> String {
    meta_read(doc)
        .and_then(|meta| read_string(&meta, "title"))
        .unwrap_or_default()
}

/// The document language, a BCP 47 tag.
#[must_use]
pub fn read_lang(doc: &LoroDoc) -> String {
    meta_read(doc)
        .and_then(|meta| read_string(&meta, "lang"))
        .unwrap_or_default()
}

// -----------------------------------------------------------------------------
// Value readers
// -----------------------------------------------------------------------------

pub(crate) fn read_i64(map: &LoroMap, key: &str) -> Option<i64> {
    match map.get(key)?.into_value() {
        Ok(LoroValue::I64(n)) => Some(n),
        _ => None,
    }
}

fn read_f64(map: &LoroMap, key: &str) -> Option<f64> {
    match map.get(key)?.into_value() {
        Ok(LoroValue::Double(n)) => Some(n),
        Ok(LoroValue::I64(n)) => Some(n as f64),
        _ => None,
    }
}

fn read_f32(map: &LoroMap, key: &str) -> Option<f32> {
    read_f64(map, key).map(|n| n as f32)
}

pub(crate) fn read_bool(map: &LoroMap, key: &str) -> Option<bool> {
    match map.get(key)?.into_value() {
        Ok(LoroValue::Bool(b)) => Some(b),
        _ => None,
    }
}

pub(crate) fn read_string(map: &LoroMap, key: &str) -> Option<String> {
    match map.get(key)?.into_value() {
        Ok(LoroValue::String(s)) => Some(s.to_string()),
        _ => None,
    }
}

// -----------------------------------------------------------------------------
// Anchors: a footnote call and a cross reference
// -----------------------------------------------------------------------------

/// The character an anchor occupies in a body: a footnote call, a cross
/// reference, a band field. The mark on it says which, and one deletion takes
/// it whole.
pub const CARRIER: char = '\u{fffc}';

/// The mark a footnote call carries: the note's identity in `annots`.
pub(crate) const K_FOOTNOTE: &str = "fn";

/// The mark a cross reference carries: its record, in hex (`PROSE-FEAT-072`).
pub(crate) const K_CROSS_REF: &str = "ref";

/// An annotation's kind, in one field of one type (`PROSE-031`).
pub(crate) const ANNOT_KIND: &str = "kind";

/// The text child of an annotation entry: a reply's body.
pub(crate) const ANNOT_TEXT: &str = "text";

/// The note marker, `true` on a block that holds a footnote's text. Such a
/// block is a root of the block tree kept out of the body flow, and the
/// note's entry names it (`PROSE-FEAT-305`).
pub(crate) const K_NOTE: &str = "nb";

/// The block of a footnote's text, named by its entry as the string Loro
/// prints a `TreeID`.
pub(crate) const ANNOT_NOTE_BLOCK: &str = "block_id";

/// The root of a comment's mark. Each thread writes its own key under it,
/// `cm:{id}`, so two threads over one character never share a value.
pub(crate) const K_COMMENT: &str = "cm";

/// The cached anchor of a comment: the block, and the span in its offsets.
pub(crate) const ANNOT_ANCHOR_BLOCK: &str = "anchor_block";
pub(crate) const ANNOT_ANCHOR_START: &str = "anchor_start";
pub(crate) const ANNOT_ANCHOR_END: &str = "anchor_end";

/// Whether a comment thread is closed.
pub(crate) const ANNOT_RESOLVED: &str = "resolved";

/// A comment's list of replies, the opening one first.
pub(crate) const ANNOT_REPLIES: &str = "replies";

/// The fields of one reply.
pub(crate) const REPLY_ID: &str = "id";
pub(crate) const REPLY_AUTHOR: &str = "author";
pub(crate) const REPLY_USER: &str = "user_id";
pub(crate) const REPLY_CREATED: &str = "created_at";

/// Every live block, in document order: the depth-first pre-order walk of the
/// tree, which is the document flow. The bands are left out.
#[must_use]
pub(crate) fn flow(tree: &LoroTree) -> Vec<TreeID> {
    let mut order = Vec::new();
    let mut stack = roots(tree);
    stack.reverse();
    while let Some(id) = stack.pop() {
        order.push(id);
        let mut children = children_of(tree, Some(id));
        children.reverse();
        stack.extend(children);
    }
    order
}

/// One character carrying an anchor mark: where it sits and the mark's value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Anchor {
    pub block: TreeID,
    /// A count of code points into the block's body.
    pub offset: u32,
    pub value: String,
}

/// Every character carrying `key`, in document order.
#[must_use]
pub(crate) fn anchors(document: &LoroDoc, key: &str) -> Vec<Anchor> {
    let tree = blocks(document);
    let mut out = Vec::new();
    for id in flow(&tree) {
        if !block_type_of(&tree, &id).is_some_and(BlockType::has_body) {
            continue;
        }
        let mut offset: u32 = 0;
        for span in get_block_text(document, &id).to_delta() {
            let loro::TextDelta::Insert { insert, attributes } = span else {
                continue;
            };
            let length = u32::try_from(insert.chars().count()).unwrap_or(u32::MAX);
            let value = attributes
                .as_ref()
                .and_then(|attributes| attributes.get(key))
                .and_then(|value| match value {
                    LoroValue::String(value) => Some(value.to_string()),
                    _ => None,
                });
            if let Some(value) = value {
                for step in 0..length {
                    out.push(Anchor {
                        block: id,
                        offset: offset.saturating_add(step),
                        value: value.clone(),
                    });
                }
            }
            offset = offset.saturating_add(length);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_seeded_document_reads_back_the_a4_format_it_was_written_with() {
        let doc = LoroDoc::new();
        seed(&doc);
        assert_eq!(read_page_format(&doc), PageFormat::default());
    }

    #[test]
    fn a_document_with_no_page_register_reads_the_default_format() {
        let doc = LoroDoc::new();
        assert_eq!(read_page_format(&doc), PageFormat::default());
    }

    #[test]
    fn a_width_no_page_can_have_falls_back_for_that_key_alone() {
        let doc = LoroDoc::new();
        seed(&doc);
        let page = map_at(&doc.get_map(META), PAGE).expect("seed writes the page register");
        page.insert("width_mm", 0.0)
            .expect("the register takes a double");
        page.insert("margin_left_mm", 5.0)
            .expect("the register takes a double");
        let format = read_page_format(&doc);
        assert_eq!(format.width_mm, PageFormat::default().width_mm);
        assert_eq!(format.margin_left_mm, 5.0);
    }

    #[test]
    fn a_body_key_re_reads_as_the_pair_it_joins() {
        let id = TreeID::new(0xDEAD_BEEF_0BAD_F00D, 42);
        let key = body_key(&id);
        assert_eq!(parse_body_key(&key), Some(id));
    }

    #[test]
    fn neither_half_of_a_body_key_carries_the_separator() {
        let id = TreeID::new(u64::MAX, i32::MAX);
        let key = body_key(&id);
        assert_eq!(key.matches(BODY_SEPARATOR).count(), 1);
        assert_eq!(parse_body_key(&key), Some(id));
    }

    #[test]
    fn a_name_that_is_not_a_body_key_reads_as_none() {
        assert_eq!(parse_body_key("blocks"), None);
        assert_eq!(parse_body_key("x:0@1"), None);
        assert_eq!(parse_body_key("t:not-an-id"), None);
    }

    #[test]
    fn a_seeded_document_holds_one_root_paragraph() {
        let doc = LoroDoc::new();
        let root = seed(&doc).expect("the scaffolding is written");
        doc.commit();
        let tree = blocks(&doc);
        assert_eq!(roots(&tree), vec![root]);
        assert_eq!(block_type_of(&tree, &root), Some(BlockType::Paragraph));
    }

    #[test]
    fn a_portrait_page_wider_than_it_is_tall_is_refused() {
        let doc = LoroDoc::new();
        assert!(seed(&doc).is_some());
        assert!(!write_page_format(
            &doc,
            Orientation::Portrait,
            297.0,
            210.0
        ));
        assert!(write_page_format(
            &doc,
            Orientation::Landscape,
            297.0,
            210.0
        ));
    }

    #[test]
    fn an_unset_property_is_the_absence_of_its_key() {
        let doc = LoroDoc::new();
        let root = seed(&doc).expect("seeded");
        let tree = blocks(&doc);
        let node = node_of(&tree, &root).expect("a live block");
        assert_eq!(
            read_paragraph_style(&tree, &root),
            ParagraphStyle::default()
        );
        write_paragraph_style(
            &node,
            &ParagraphStyle {
                align: Some(Align::Center),
                ..ParagraphStyle::default()
            },
        )
        .expect("the write lands");
        assert_eq!(
            read_paragraph_style(&tree, &root).align,
            Some(Align::Center)
        );
        assert_eq!(read_paragraph_style(&tree, &root).indent_left, None);
        assert!(node.get(K_INDENT_LEFT).is_none());
    }

    #[test]
    fn the_nine_built_in_style_names_are_not_user_styles() {
        let style = |name: &str| ambre_bus::app::prose::StyleId(name.to_owned());
        assert!(is_built_in_style(&style("quote")));
        assert!(is_built_in_style(&style("heading-6")));
        assert!(!is_built_in_style(&style("my-style")));
    }

    /// `write_page_format` used to mint a fresh page register, dropping the four
    /// margins `seed` had written beside the three keys it writes.
    #[test]
    fn a_second_page_format_leaves_the_four_margins_in_place() {
        let doc = LoroDoc::new();
        seed(&doc).expect("the scaffolding is written");
        doc.commit();
        assert!(write_page_format(
            &doc,
            Orientation::Landscape,
            297.0,
            210.0
        ));
        doc.commit();
        let page = map_at(&doc.get_map(META), PAGE).expect("the page register");
        for key in [
            "margin_top_mm",
            "margin_bottom_mm",
            "margin_left_mm",
            "margin_right_mm",
        ] {
            assert!(page.get(key).is_some(), "{key} was dropped");
        }
        assert!(page.get("orientation").is_some());
    }
}
