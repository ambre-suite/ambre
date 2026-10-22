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

use ambre_core::crdt::FieldKey;
use loro::{
    LoroDoc, LoroMap, LoroMovableList, LoroText, LoroTree, LoroValue, TreeID, TreeParentId,
};

use ambre_bus::app::prose::{
    Align, BlockType, DocCount, DocOffset, DocSpan, LineHeight, LineHeightMode, ListStyle,
    OrderedStyle, ParagraphStyle,
};

// -----------------------------------------------------------------------------
// Container names and keys (`ambre-spec-prose-schema.md section 3`)
// -----------------------------------------------------------------------------

ambre_core::loro_fields!(pub(crate));

ambre_core::crdt_names! {
    /// The root containers a Prose document opens by name. A body is a root as
    /// well, and [`BodyKey`] names it, since its name carries a tree id.
    pub(crate) enum Root: ContainerName {
        /// The document register: the schema version, the title, the stamps,
        /// the language, the page sub-map and the band registers.
        Meta = ambre_core::crdt::FoundationRoot::Meta.as_str(),
        /// The block hierarchy.
        Blocks = "blocks",
        /// The user's own paragraph styles. The nine built-in names are
        /// compiled in and never stored (`PROSE-093`).
        Styles = "styles",
        /// The comments, the suggestions and the footnotes, one entry each,
        /// keyed by the annotation's id.
        Annots = "annots",
    }
}

ambre_core::crdt_names! {
    /// The keys of the `meta` register.
    pub(crate) enum MetaKey: FieldKey {
        Title = "title",
        /// A BCP 47 tag.
        Lang = "lang",
        Pageless = "pageless",
        /// The page sub-map, keyed by [`PageKey`].
        Page = "page",
        /// The typographic defaults sub-map, keyed by [`DefaultsKey`].
        Defaults = "defaults",
        /// The four band registers, each keyed by [`BandKey`].
        Header = "header",
        Footer = "footer",
        HeaderFirst = "header_first",
        FooterFirst = "footer_first",
        /// The one setting that turns both first-page bands on
        /// (`PROSE-FEAT-063`).
        DifferentFirstPage = "different_first_page",
        /// The last-modified stamp, Unix milliseconds by the clock of the peer
        /// that wrote last (`PROSE-FEAT-066`).
        UpdatedAt = "updated_at",
    }
}

ambre_core::crdt_names! {
    /// The keys of the page sub-map, every length in millimetres.
    pub(crate) enum PageKey: FieldKey {
        Orientation = "orientation",
        WidthMm = "width_mm",
        HeightMm = "height_mm",
        MarginTopMm = "margin_top_mm",
        MarginBottomMm = "margin_bottom_mm",
        MarginLeftMm = "margin_left_mm",
        MarginRightMm = "margin_right_mm",
    }
}

ambre_core::crdt_names! {
    /// The keys of the typographic defaults sub-map.
    pub(crate) enum DefaultsKey: FieldKey {
        FontFamily = "font_family",
        FontSizePt = "font_size_pt",
        /// A multiple of the font size.
        LineHeight = "line_height",
    }
}

ambre_core::crdt_names! {
    /// The keys of one band register.
    pub(crate) enum BandKey: FieldKey {
        /// Whether a side is shown, in the header and footer registers alone.
        Enabled = "enabled",
        /// The band block's `TreeID`, as the string Loro prints it.
        Block = "block_id",
        /// The band's alignment, by name.
        Align = "align",
    }
}

ambre_core::crdt_names! {
    /// The keys of a block's node map.
    pub(crate) enum NodeKey: FieldKey {
        /// The block type discriminant.
        Type = "t",
        /// The style id: a key in `styles`, or one of the nine built-in names.
        Style = "s",
        /// The type-specific attribute map, keyed by [`AttrKey`].
        Attrs = "a",
        /// The scaffolding marker. `true` on a root paragraph a peer grew
        /// because the tree held none, and on no other block (`PROSE-094`).
        ///
        /// Loro refuses to raise a tombstone, so a tree that empties cannot
        /// re-grow the block it lost. Each peer grows one of its own, and the
        /// merge keeps one. The marker tells a grown block from one the user
        /// asked for, so every peer drops the surplus alike.
        Regrown = "rg",
        /// The hiding marker, holding the Loro peer id of the session that
        /// deleted the block, in decimal (`PROSE-122`). A user's deletion
        /// writes it rather than removing the node (`PROSE-118`). An undo
        /// removes the key, and the block comes back under its own `TreeID`
        /// with its body (`PROSE-120`).
        Hidden = "gn",
        /// The band marker, `true` on a block that holds a header or a
        /// footer. Such a block is a root of the block tree kept out of the
        /// body flow (`PROSE-006`).
        Band = "hb",
        /// The note marker, `true` on a block that holds a footnote's text.
        /// Such a block is a root of the block tree kept out of the body
        /// flow, and the note's entry names it (`PROSE-FEAT-305`).
        Note = "nb",
        /// The nine paragraph-style keys, flat in the node map, one key per
        /// property (`PROSE-030`).
        Align = "align",
        IndentFirst = "indent_first",
        IndentLeft = "indent_left",
        IndentRight = "indent_right",
        SpaceBefore = "space_before",
        SpaceAfter = "space_after",
        /// The line height sub-map, keyed by [`LineHeightKey`].
        LineHeight = "line_height",
        KeepTogether = "keep_together",
        KeepWithNext = "keep_with_next",
    }
}

ambre_core::crdt_names! {
    /// The two keys of the `line_height` sub-map.
    pub(crate) enum LineHeightKey: FieldKey {
        Mode = "mode",
        Value = "value",
    }
}

ambre_core::crdt_names! {
    /// The keys of a block's attribute map. Each applies to some block types
    /// alone.
    pub(crate) enum AttrKey: FieldKey {
        /// The heading level attribute, `1..=6`.
        Level = "level",
        /// The code block's language. An empty value reads as plain.
        Lang = "lang",
        /// A list item's marker kind: 0 bullet, 1 ordered, 2 checkbox.
        Marker = "ls",
        /// How an ordered item's ordinal is spelled, on an ordered item alone.
        Spelling = "lo",
        /// Whether a checkbox item is ticked, on a checkbox item alone.
        Ticked = "tc",
        /// A table's column widths in points: a movable list, one entry per
        /// column (`PROSE-FEAT-044`). A list container rather than a value,
        /// so a column inserted and a column resized at the same moment both
        /// survive the merge.
        ColWidths = "col_widths",
        /// A table's merged regions: a map from the anchor cell's id to a
        /// region record keyed by [`RegionKey`] (`PROSE-009`). A map
        /// container, so two regions written at once are two keys rather than
        /// one value overwritten.
        Merged = "merged",
        /// A cell's background, as `"#rrggbb"`.
        CellBg = "bg",
        /// The four border widths of a cell, in points.
        EdgeTop = "bt",
        EdgeBottom = "bb",
        EdgeLeft = "bl",
        EdgeRight = "br",
        /// The border colour of a cell, as `"#rrggbb"`.
        EdgeColor = "bc",
        /// A cell's padding, in points.
        Padding = "pad",
        /// A cell's vertical alignment: 0 top, 1 middle, 2 bottom.
        VAlign = "va",
        /// The deepest heading level a table of contents lists, one to six.
        MaxLevel = "max_level",
        /// Whether a table of contents numbers its entries.
        Numbered = "numbered",
        /// An image's box height. No command writes it yet, so no scenario
        /// reaches it ([`UNWRITTEN_ATTRS`]).
        ImageHeight = "h",
        /// An image's caption. No command writes it yet.
        ImageCaption = "caption",
        /// The hash of an image's bytes. The image command writes it, and the
        /// schema scenario holds no image.
        ContentHash = "content_hash",
    }
}

/// The attributes the schema declares and the schema scenario does not write.
/// The two schema belts leave them out until the images Story lands
/// (`ambre-roadmap.md` E5).
#[cfg(test)]
pub(crate) const UNWRITTEN_ATTRS: [AttrKey; 3] = [
    AttrKey::ImageHeight,
    AttrKey::ImageCaption,
    AttrKey::ContentHash,
];

ambre_core::crdt_names! {
    /// The keys of a merged region's record.
    pub(crate) enum RegionKey: FieldKey {
        /// The bottom-right cell's id.
        Extent = "e",
        /// The Lamport time the region was written at, which is what "the
        /// later of two regions" reads (`PROSE-FEAT-254`).
        Stamp = "n",
    }
}

ambre_core::crdt_names! {
    /// The keys of an annotation entry. A comment, a suggestion and a
    /// footnote each write a part of the set.
    pub(crate) enum AnnotKey: FieldKey {
        /// An annotation's kind, in one field of one type (`PROSE-031`).
        Kind = "kind",
        /// The text child of an entry, before a footnote's text moved into a
        /// block.
        Text = "text",
        /// The block of a footnote's text, as the string Loro prints a
        /// `TreeID`.
        NoteBlock = "block_id",
        /// The cached anchor of a comment: the block, and the span in its
        /// offsets.
        AnchorBlock = "anchor_block",
        AnchorStart = "anchor_start",
        AnchorEnd = "anchor_end",
        /// Whether a comment thread is closed.
        Resolved = "resolved",
        /// A comment's list of replies, the opening one first, each keyed by
        /// [`ReplyKey`].
        Replies = "replies",
        /// A suggestion's author, by display name and by id.
        Author = "author",
        User = "user_id",
    }
}

ambre_core::crdt_names! {
    /// The keys of one reply of a comment.
    pub(crate) enum ReplyKey: FieldKey {
        Id = "id",
        Author = "author",
        User = "user_id",
        Created = "created_at",
        /// The reply's body, a text container.
        Text = "text",
    }
}

/// The tag a body key joins to a tree id.
const BODY_TAG: &str = "t";
/// The separator of a body key. Neither half can carry it (`PROSE-088`).
const BODY_SEPARATOR: char = ':';

/// A root container of the document, which the document opens by name.
#[must_use]
pub(crate) fn root_map(doc: &LoroDoc, root: Root) -> LoroMap {
    doc.get_map(root.as_str())
}

/// The `meta` register, attached when the document has none. A reader takes
/// [`meta_read`] instead.
#[must_use]
pub(crate) fn meta(doc: &LoroDoc) -> Fields<MetaKey> {
    Fields::of(root_map(doc, Root::Meta))
}

/// The `annots` dictionary, keyed by the annotation's id. Each entry is a map
/// keyed by [`AnnotKey`].
#[must_use]
pub(crate) fn annots(doc: &LoroDoc) -> LoroMap {
    root_map(doc, Root::Annots)
}

/// The map a stored entry holds, read as a map of the schema `K`, or `None`
/// for a value or a container of another kind.
#[must_use]
pub(crate) fn fields_of<K: ambre_core::crdt::FieldKey>(
    found: loro::ValueOrContainer,
) -> Option<Fields<K>> {
    found
        .into_container()
        .ok()
        .and_then(|container| container.into_map().ok())
        .map(Fields::of)
}

/// The width a column takes when the command that makes it names none
/// (`PROSE-FEAT-232`): one inch, in points.
pub const DEFAULT_COLUMN_WIDTH_PT: f32 = 72.0;

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

/// The name of a block's body text container: the tag `t` and the tree id,
/// joined by `:`. Built from a tree id alone, so no call site spells one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BodyKey(String);

impl BodyKey {
    /// The body name of block `id`.
    #[must_use]
    pub fn of(id: &TreeID) -> Self {
        Self(format!("{BODY_TAG}{BODY_SEPARATOR}{id}"))
    }

    /// The name as the document stores it, for the call that opens the body.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The pair a body key joins, or `None` for a name that is not one.
///
/// The inverse of [`BodyKey::of`], and what makes the key a pair rather than an
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
    doc.get_text(BodyKey::of(id).as_str())
}

// -----------------------------------------------------------------------------
// Block reads
// -----------------------------------------------------------------------------

/// The block tree.
#[must_use]
pub(crate) fn blocks(doc: &LoroDoc) -> LoroTree {
    doc.get_tree(Root::Blocks.as_str())
}

/// A live block's node map, or `None` when the id names no live block.
///
/// A tombstoned block answers `None`. It is gone as far as every command is
/// concerned, and no path of this module reaches one.
#[must_use]
pub(crate) fn node_of(tree: &LoroTree, id: &TreeID) -> Option<Fields<NodeKey>> {
    if tree.is_node_deleted(id).unwrap_or(true) || is_hidden(tree, id) {
        return None;
    }
    tree.get_meta(*id).ok().map(Fields::of)
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
        .is_ok_and(|node| Fields::of(node).contains(NodeKey::Hidden))
}

/// Hide a block and the subtree under it, in place of removing it
/// (`PROSE-118`). The marker names the session that hid it, the one peer whose
/// undo can bring it back.
///
/// # Errors
/// The CRDT engine's refusal, as text.
pub(crate) fn hide_block(doc: &LoroDoc, tree: &LoroTree, id: &TreeID) -> Result<(), String> {
    let node = Fields::of(tree.get_meta(*id).map_err(|e| format!("{e:?}"))?);
    node.insert(NodeKey::Hidden, doc.peer_id().to_string())
        .map_err(|e| format!("{e:?}"))
}

/// The type a live block carries, or `None` when the id names no live block or
/// the discriminant is one this build does not know.
#[must_use]
pub fn block_type_of(tree: &LoroTree, id: &TreeID) -> Option<BlockType> {
    let node = node_of(tree, id)?;
    let code = u8::try_from(read_i64(&node, NodeKey::Type)?).ok()?;
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
    let attrs = attrs_read(tree, id)?;
    u8::try_from(read_i64(&attrs, AttrKey::Level)?).ok()
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
    let attrs = attrs_read(tree, id)?;
    match u8::try_from(read_i64(&attrs, AttrKey::Marker)?).ok()? {
        0 => Some(ListStyle::Bullet),
        1 => {
            let code = u8::try_from(read_i64(&attrs, AttrKey::Spelling)?).ok()?;
            OrderedStyle::from_code(code).map(ListStyle::Ordered)
        }
        2 => Some(ListStyle::Todo(read_bool(&attrs, AttrKey::Ticked)?)),
        _ => None,
    }
}

/// A live block's attribute map, or `None` when it carries none. Read-only: it
/// never mints the map it looked for (`PROSE-079`).
#[must_use]
pub(crate) fn attrs_read(tree: &LoroTree, id: &TreeID) -> Option<Fields<AttrKey>> {
    map_read(&node_of(tree, id)?, NodeKey::Attrs).map(Fields::of)
}

/// A block's attribute map, minted when the node holds none, for the same
/// reason as [`map_at`].
pub(crate) fn attrs_at(node: &Fields<NodeKey>) -> Option<Fields<AttrKey>> {
    map_at(node, NodeKey::Attrs).map(Fields::of)
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
        .and_then(|a| read_i64(a, AttrKey::MaxLevel))
        .and_then(|n| u8::try_from(n).ok())
        .unwrap_or(6);
    let numbered = attrs
        .as_ref()
        .and_then(|a| read_bool(a, AttrKey::Numbered))
        .unwrap_or(false);
    Some((level, numbered))
}

/// A table's column widths list, or `None` when the table carries none.
#[must_use]
pub(crate) fn col_widths_read(tree: &LoroTree, table: &TreeID) -> Option<LoroMovableList> {
    attrs_read(tree, table)?
        .get(AttrKey::ColWidths)
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
    map_read(&attrs_read(tree, table)?, AttrKey::Merged)
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
        let record = FieldValues::<RegionKey>::of(&record);
        let extent = match record.get(RegionKey::Extent) {
            Some(LoroValue::String(text)) => TreeID::try_from(text.as_str()).ok(),
            _ => None,
        };
        let stamp = match record.get(RegionKey::Stamp) {
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
    read_string(&node, NodeKey::Style)
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
            node_of(tree, id).is_some_and(|node| read_bool(&node, NodeKey::Regrown) == Some(true))
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
pub(crate) fn map_at<K: FieldKey>(parent: &Fields<K>, key: K) -> Option<LoroMap> {
    let existing = parent
        .get(key)
        .and_then(|value| value.into_container().ok())
        .and_then(|container| container.into_map().ok());
    match existing {
        Some(map) => Some(map),
        None => parent
            .insert_container(key, LoroMap::new())
            .map_err(|e| {
                tracing::warn!(
                    "Prose: the register {} refused a container: {e}",
                    key.as_str()
                );
            })
            .ok(),
    }
}

/// The movable list a key holds, opened once and never replaced, for the same
/// reason as [`map_at`].
pub(crate) fn movable_list_at<K: FieldKey>(parent: &Fields<K>, key: K) -> Option<LoroMovableList> {
    let existing = parent
        .get(key)
        .and_then(|value| value.into_container().ok())
        .and_then(|container| container.into_movable_list().ok());
    match existing {
        Some(list) => Some(list),
        None => parent
            .insert_container(key, LoroMovableList::new())
            .map_err(|e| {
                tracing::warn!("Prose: the list {} refused a container: {e}", key.as_str())
            })
            .ok(),
    }
}

/// The `meta` register, or `None` when the document has none.
///
/// A reader takes this path and never `get_map`, which attaches the root
/// container it did not find. Laying the document out writes nothing to it
/// (`PROSE-079`).
#[must_use]
pub(crate) fn meta_read(doc: &LoroDoc) -> Option<Fields<MetaKey>> {
    doc.get_by_str_path(Root::Meta.as_str()).and_then(fields_of)
}

/// The block tree, or `None` when the document has none, for the same reason
/// as [`meta_read`].
#[must_use]
pub(crate) fn blocks_read(doc: &LoroDoc) -> Option<LoroTree> {
    doc.get_by_str_path(Root::Blocks.as_str())
        .and_then(|value| value.into_container().ok())
        .and_then(|container| container.into_tree().ok())
}

/// The map a key already holds, or `None` when the key is empty.
///
/// The read-only half of [`map_at`]. A reader must never mint the container it
/// went looking for: laying the document out writes nothing to it
/// (`PROSE-079`).
#[must_use]
pub(crate) fn map_read<K: FieldKey>(parent: &Fields<K>, key: K) -> Option<LoroMap> {
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
    node: &Fields<NodeKey>,
    style: &ParagraphStyle,
) -> Result<(), loro::LoroError> {
    if let Some(align) = style.align {
        node.insert(NodeKey::Align, i64::from(align.code()))?;
    }
    for (key, value) in [
        (NodeKey::IndentFirst, style.indent_first),
        (NodeKey::IndentLeft, style.indent_left),
        (NodeKey::IndentRight, style.indent_right),
        (NodeKey::SpaceBefore, style.space_before),
        (NodeKey::SpaceAfter, style.space_after),
    ] {
        if let Some(points) = value {
            node.insert(key, f64::from(points))?;
        }
    }
    for (key, value) in [
        (NodeKey::KeepTogether, style.keep_together),
        (NodeKey::KeepWithNext, style.keep_with_next),
    ] {
        if let Some(flag) = value {
            node.insert(key, flag)?;
        }
    }
    if let Some(line_height) = style.line_height {
        let Some(map) = map_at(node, NodeKey::LineHeight).map(Fields::<LineHeightKey>::of) else {
            return Err(loro::LoroError::NotFoundError(
                "the line-height register refused a container".into(),
            ));
        };
        map.insert(LineHeightKey::Mode, i64::from(line_height.mode.code()))?;
        map.insert(LineHeightKey::Value, f64::from(line_height.value))?;
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
        align: read_i64(&node, NodeKey::Align)
            .and_then(|n| u8::try_from(n).ok())
            .and_then(Align::from_code),
        indent_first: read_f32(&node, NodeKey::IndentFirst),
        indent_left: read_f32(&node, NodeKey::IndentLeft),
        indent_right: read_f32(&node, NodeKey::IndentRight),
        space_before: read_f32(&node, NodeKey::SpaceBefore),
        space_after: read_f32(&node, NodeKey::SpaceAfter),
        line_height: read_line_height(&node),
        keep_together: read_bool(&node, NodeKey::KeepTogether),
        keep_with_next: read_bool(&node, NodeKey::KeepWithNext),
    }
}

fn read_line_height(node: &Fields<NodeKey>) -> Option<LineHeight> {
    let map: Fields<LineHeightKey> = node.get(NodeKey::LineHeight).and_then(fields_of)?;
    let mode = u8::try_from(read_i64(&map, LineHeightKey::Mode)?)
        .ok()
        .and_then(LineHeightMode::from_code)?;
    let value = read_f64(&map, LineHeightKey::Value)?;
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
    let Some(defaults) = meta_read(doc)
        .and_then(|meta| map_read(&meta, MetaKey::Defaults))
        .map(Fields::<DefaultsKey>::of)
    else {
        return fallback;
    };
    let read = |key: DefaultsKey, default: f32| -> f32 {
        read_f32(&defaults, key)
            .filter(|number| number.is_finite() && *number > 0.0)
            .unwrap_or(default)
    };
    DocDefaults {
        font_family: read_string(&defaults, DefaultsKey::FontFamily)
            .filter(|name| !name.is_empty())
            .unwrap_or(fallback.font_family),
        font_size_pt: read(DefaultsKey::FontSizePt, fallback.font_size_pt),
        line_height: read(DefaultsKey::LineHeight, fallback.line_height),
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
    let Some(page) = meta_read(doc)
        .and_then(|meta| map_read(&meta, MetaKey::Page))
        .map(Fields::<PageKey>::of)
    else {
        return fallback;
    };
    let read = |key: PageKey, default: f32| -> f32 {
        read_f32(&page, key)
            .filter(|number| number.is_finite() && *number >= 0.0)
            .unwrap_or(default)
    };
    let width_mm = read(PageKey::WidthMm, fallback.width_mm);
    let height_mm = read(PageKey::HeightMm, fallback.height_mm);
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
        margin_top_mm: read(PageKey::MarginTopMm, fallback.margin_top_mm),
        margin_bottom_mm: read(PageKey::MarginBottomMm, fallback.margin_bottom_mm),
        margin_left_mm: read(PageKey::MarginLeftMm, fallback.margin_left_mm),
        margin_right_mm: read(PageKey::MarginRightMm, fallback.margin_right_mm),
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
    let Some(page) = map_at(&meta(doc), MetaKey::Page).map(Fields::<PageKey>::of) else {
        return false;
    };
    page.insert(PageKey::Orientation, orientation.as_str())
        .is_ok()
        && page.insert(PageKey::WidthMm, f64::from(width_mm)).is_ok()
        && page.insert(PageKey::HeightMm, f64::from(height_mm)).is_ok()
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
    let meta = meta(doc);
    meta.insert(MetaKey::Title, "").ok()?;
    meta.insert(MetaKey::Lang, "en").ok()?;
    meta.insert(MetaKey::Pageless, false).ok()?;
    if !write_page_format(doc, Orientation::Portrait, A4_WIDTH_MM, A4_HEIGHT_MM) {
        return None;
    }
    let page = Fields::of(map_at(&meta, MetaKey::Page)?);
    for key in [
        PageKey::MarginTopMm,
        PageKey::MarginBottomMm,
        PageKey::MarginLeftMm,
        PageKey::MarginRightMm,
    ] {
        page.insert(key, f64::from(DEFAULT_MARGIN_MM)).ok()?;
    }
    let defaults = Fields::of(map_at(&meta, MetaKey::Defaults)?);
    let text = DocDefaults::default();
    defaults
        .insert(DefaultsKey::FontFamily, text.font_family.as_str())
        .ok()?;
    defaults
        .insert(DefaultsKey::FontSizePt, f64::from(text.font_size_pt))
        .ok()?;
    defaults
        .insert(DefaultsKey::LineHeight, f64::from(text.line_height))
        .ok()?;
    // The two registers no seeded key belongs to are opened all the same, so
    // every peer addresses one container rather than minting a rival one.
    let _ = root_map(doc, Root::Styles);
    let _ = annots(doc);
    grow_root(doc)
}

/// Grow one root paragraph, marked as grown (`PROSE-094`).
///
/// Called when the tree holds no root at all: at construction, and after a
/// deletion that emptied it.
pub(crate) fn grow_root(doc: &LoroDoc) -> Option<TreeID> {
    let id = create_block(doc, None, BlockType::Paragraph)?;
    let tree = blocks(doc);
    node_of(&tree, &id)?.insert(NodeKey::Regrown, true).ok()?;
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
    let node = Fields::of(tree.get_meta(id).ok()?);
    node.insert(NodeKey::Type, i64::from(block_type.code()))
        .ok()?;
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
    pub(crate) fn register_of(&self, id: &TreeID) -> Option<MetaKey> {
        [
            (self.header, MetaKey::Header),
            (self.footer, MetaKey::Footer),
            (self.header_first, MetaKey::HeaderFirst),
            (self.footer_first, MetaKey::FooterFirst),
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

fn read_band(meta: &Fields<MetaKey>, key: MetaKey) -> Band {
    let Some(map) = map_read(meta, key).map(Fields::<BandKey>::of) else {
        return Band::default();
    };
    let block = read_string(&map, BandKey::Block).and_then(|s| TreeID::try_from(s.as_str()).ok());
    let align = match read_string(&map, BandKey::Align).as_deref() {
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
    let enabled = |key: MetaKey| {
        map_read(&meta, key)
            .is_some_and(|map| read_bool(&Fields::of(map), BandKey::Enabled) == Some(true))
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
        header: live(read_band(&meta, MetaKey::Header)),
        footer: live(read_band(&meta, MetaKey::Footer)),
        header_first: live(read_band(&meta, MetaKey::HeaderFirst)),
        footer_first: live(read_band(&meta, MetaKey::FooterFirst)),
        header_enabled: enabled(MetaKey::Header),
        footer_enabled: enabled(MetaKey::Footer),
        different_first_page: read_bool(&meta, MetaKey::DifferentFirstPage) == Some(true),
    }
}

/// Whether a block holds a band rather than body text.
#[must_use]
pub(crate) fn is_band(tree: &LoroTree, id: &TreeID) -> bool {
    node_of(tree, id).is_some_and(|node| read_bool(&node, NodeKey::Band) == Some(true))
}

/// Whether a block holds a footnote's text rather than body text.
#[must_use]
pub(crate) fn is_note(tree: &LoroTree, id: &TreeID) -> bool {
    node_of(tree, id).is_some_and(|node| read_bool(&node, NodeKey::Note) == Some(true))
}

/// Whether a block stands outside the body flow: a band or a note. Such a
/// block is one paragraph a register or an entry names, so no structural
/// command may split, move or delete it.
#[must_use]
pub(crate) fn is_aside(tree: &LoroTree, id: &TreeID) -> bool {
    node_of(tree, id).is_some_and(|node| {
        read_bool(&node, NodeKey::Band) == Some(true)
            || read_bool(&node, NodeKey::Note) == Some(true)
    })
}

/// The last-modified stamp, or `None` before any peer wrote one.
#[must_use]
pub fn read_updated_at(doc: &LoroDoc) -> Option<i64> {
    meta_read(doc).and_then(|meta| read_i64(&meta, MetaKey::UpdatedAt))
}

/// The document title, empty when none was set.
#[must_use]
pub fn read_title(doc: &LoroDoc) -> String {
    meta_read(doc)
        .and_then(|meta| read_string(&meta, MetaKey::Title))
        .unwrap_or_default()
}

/// The document language, a BCP 47 tag.
#[must_use]
pub fn read_lang(doc: &LoroDoc) -> String {
    meta_read(doc)
        .and_then(|meta| read_string(&meta, MetaKey::Lang))
        .unwrap_or_default()
}

// -----------------------------------------------------------------------------
// Value readers
// -----------------------------------------------------------------------------

pub(crate) fn read_i64<K: FieldKey>(map: &Fields<K>, key: K) -> Option<i64> {
    match map.get(key)?.into_value() {
        Ok(LoroValue::I64(n)) => Some(n),
        _ => None,
    }
}

fn read_f64<K: FieldKey>(map: &Fields<K>, key: K) -> Option<f64> {
    match map.get(key)?.into_value() {
        Ok(LoroValue::Double(n)) => Some(n),
        Ok(LoroValue::I64(n)) => Some(n as f64),
        _ => None,
    }
}

fn read_f32<K: FieldKey>(map: &Fields<K>, key: K) -> Option<f32> {
    read_f64(map, key).map(|n| n as f32)
}

pub(crate) fn read_bool<K: FieldKey>(map: &Fields<K>, key: K) -> Option<bool> {
    match map.get(key)?.into_value() {
        Ok(LoroValue::Bool(b)) => Some(b),
        _ => None,
    }
}

pub(crate) fn read_string<K: FieldKey>(map: &Fields<K>, key: K) -> Option<String> {
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

/// The root of a comment's mark. Each thread writes its own key under it,
/// `cm:{id}`, so two threads over one character never share a value.
pub(crate) const K_COMMENT: &str = "cm";

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
    pub offset: DocOffset,
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
        let mut offset = DocOffset::ZERO;
        for span in get_block_text(document, &id).to_delta() {
            let loro::TextDelta::Insert { insert, attributes } = span else {
                continue;
            };
            let length = DocCount::of(&insert);
            let value = attributes
                .as_ref()
                .and_then(|attributes| attributes.get(key))
                .and_then(|value| match value {
                    LoroValue::String(value) => Some(value.to_string()),
                    _ => None,
                });
            if let Some(value) = value {
                for at in DocSpan::from_count(offset, length) {
                    out.push(Anchor {
                        block: id,
                        offset: at,
                        value: value.clone(),
                    });
                }
            }
            offset += length;
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
        let page = map_at(&meta(&doc), MetaKey::Page)
            .map(Fields::<PageKey>::of)
            .expect("seed writes the page register");
        page.insert(PageKey::WidthMm, 0.0)
            .expect("the register takes a double");
        page.insert(PageKey::MarginLeftMm, 5.0)
            .expect("the register takes a double");
        let format = read_page_format(&doc);
        assert_eq!(format.width_mm, PageFormat::default().width_mm);
        assert_eq!(format.margin_left_mm, 5.0);
    }

    #[test]
    fn a_body_key_re_reads_as_the_pair_it_joins() {
        let id = TreeID::new(0xDEAD_BEEF_0BAD_F00D, 42);
        let key = BodyKey::of(&id);
        assert_eq!(parse_body_key(key.as_str()), Some(id));
    }

    #[test]
    fn neither_half_of_a_body_key_carries_the_separator() {
        let id = TreeID::new(u64::MAX, i32::MAX);
        let key = BodyKey::of(&id);
        assert_eq!(key.as_str().matches(BODY_SEPARATOR).count(), 1);
        assert_eq!(parse_body_key(key.as_str()), Some(id));
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
        assert!(node.get(NodeKey::IndentLeft).is_none());
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
        let page = map_at(&meta(&doc), MetaKey::Page)
            .map(Fields::<PageKey>::of)
            .expect("the page register");
        for key in [
            PageKey::MarginTopMm,
            PageKey::MarginBottomMm,
            PageKey::MarginLeftMm,
            PageKey::MarginRightMm,
        ] {
            assert!(page.contains(key), "{key:?} was dropped");
        }
        assert!(page.contains(PageKey::Orientation));
    }
}
