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

use ambre_core::crdt::CrdtName;
// The stored heading level lives with the intermediate form, which names it.
use ambre_core::crdt::FieldKey;
use ambre_core::index::{Pos, Span};
use ambre_core::{FontSize, LangTag, Mm, Pt, Validated};
use ambre_core::{PrintLength, StoredAsI64};
pub use ambre_prose_form::StoredLevel;
use loro::{
    LoroDoc, LoroMap, LoroMovableList, LoroText, LoroTree, LoroValue, TreeID, TreeParentId,
};

use crate::message::{
    Align, BandState, BlockType, DocChar, FirstPage, HeadingLevel, KeepTogether, KeepWithNext,
    LineHeight, LineHeightMode, LineSpacingMultiple, ListStyle, OrderedStyle, ParagraphStyle,
    StyleId, TocNumbering,
};
use ambre_core::{Authored, Instant};

// -----------------------------------------------------------------------------
// Container names and keys (`ambre-spec-prose-schema.md section 3`)
// -----------------------------------------------------------------------------

use ambre_core::Flag as _;
pub(crate) use ambre_crdt::{FieldValues, Fields};

/// Why a write to the document did not land.
#[derive(Debug)]
pub(crate) enum WriteError {
    /// The CRDT engine refused the write.
    Engine(loro::LoroError),
    /// The engine gave up diffing a whole body's new text.
    TextUpdate(loro::UpdateTimeoutError),
    /// A part of the document the write needs was not there, or could not be
    /// made. The cause names it, and its sentence reads in the log and the
    /// refusal alike.
    Refused(RefusalCause),
    /// A write that names its author, made before the session named its user.
    NoIdentity,
}

/// Which part of the document a refused write needed: one variant per cause,
/// so a caller and a test match the cause, never its sentence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RefusalCause {
    /// A comment entry carries no reply list.
    NoReplyList,
    /// A row appended to a table holds no cell.
    AppendedRowEmpty,
    /// A block's attribute map refused a child container.
    AttrMapRefused,
    /// A band block got no name.
    BandBlockUnnamed,
    /// A band register refused its alignment.
    BandAlignRefused,
    /// A band register refused a setting.
    BandSettingRefused,
    /// A block refused its attribute map.
    BlockAttrsRefused,
    /// A block left the tree before its attributes landed.
    BlockVanished,
    /// An export the Logic thread writes reached the document.
    FileByLogic,
    /// A new block left the tree before its style landed.
    NewBlockVanished,
    /// A note block is not in the tree.
    NoteBlockMissing,
    /// A table's region map refused a child container.
    RegionMapRefused,
    /// A reply holds no text container.
    ReplyEmpty,
    /// A table carries no column width list.
    NoWidthList,
    /// The tree refused a band block.
    TreeRefusedBandBlock,
    /// The tree refused a block.
    TreeRefusedBlock,
    /// The tree refused a table cell.
    TreeRefusedCell,
    /// The tree refused a note block.
    TreeRefusedNoteBlock,
    /// The tree refused a table row.
    TreeRefusedRow,
    /// A table's width list refused a child container.
    WidthListRefused,
    /// A paragraph length to write is not a finite number.
    NonFiniteLength,
}

impl RefusalCause {
    /// The sentence the log and the refusal carry.
    pub(crate) const fn sentence(self) -> &'static str {
        match self {
            Self::NoReplyList => "no reply list",
            Self::AppendedRowEmpty => "the appended row holds no cell",
            Self::AttrMapRefused => "the attribute map refused a container",
            Self::BandBlockUnnamed => "the band block could not be named",
            Self::BandAlignRefused => "the band register refused the alignment",
            Self::BandSettingRefused => "the band register refused the setting",
            Self::BlockAttrsRefused => "the block refused its attribute map",
            Self::BlockVanished => "the block vanished before its attributes landed",
            Self::FileByLogic => "the file is written by the Logic thread, not by the document",
            Self::NewBlockVanished => "the new block vanished before its style landed",
            Self::NoteBlockMissing => "the note block is not in the tree",
            Self::RegionMapRefused => "the region map refused a container",
            Self::ReplyEmpty => "the reply holds no text",
            Self::NoWidthList => "the table carries no width list",
            Self::TreeRefusedBandBlock => "the tree refused a band block",
            Self::TreeRefusedBlock => "the tree refused a block",
            Self::TreeRefusedCell => "the tree refused a cell",
            Self::TreeRefusedNoteBlock => "the tree refused a note block",
            Self::TreeRefusedRow => "the tree refused a row",
            Self::WidthListRefused => "the width list refused a container",
            Self::NonFiniteLength => "a paragraph length is not a finite number",
        }
    }
}

impl std::fmt::Display for RefusalCause {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.sentence())
    }
}

impl From<loro::LoroError> for WriteError {
    fn from(error: loro::LoroError) -> Self {
        Self::Engine(error)
    }
}

impl From<loro::UpdateTimeoutError> for WriteError {
    fn from(error: loro::UpdateTimeoutError) -> Self {
        Self::TextUpdate(error)
    }
}

impl std::fmt::Display for WriteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            // The engine's refusal reads as it always has: its debug form.
            Self::Engine(error) => write!(f, "{error:?}"),
            Self::TextUpdate(error) => write!(f, "{error:?}"),
            Self::Refused(cause) => cause.fmt(f),
            Self::NoIdentity => f.write_str("the session has no user"),
        }
    }
}

impl std::error::Error for WriteError {}

/// The refusal a command whose write did not land answers with.
impl From<WriteError> for crate::message::ProseRefusal {
    fn from(error: WriteError) -> Self {
        match error {
            WriteError::NoIdentity => Self::NoIdentity,
            // The author's input, not the engine, is at fault: the refusal
            // says so in the author's terms.
            WriteError::Refused(RefusalCause::NonFiniteLength) => Self::NonFiniteLength,
            WriteError::Engine(_) | WriteError::TextUpdate(_) | WriteError::Refused(_) => {
                tracing::warn!("Prose: the engine refused a write: {error}");
                Self::WriteFailed
            }
        }
    }
}

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
        /// The document locale, a BCP 47 tag: the key Trame and Toile share.
        Locale = "locale",
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
        /// An image's display width, in page pixels at zoom one: never the
        /// native width of the file.
        ImageWidth = "w",
        /// An image's display height, as its width.
        ImageHeight = "h",
        /// An image's caption.
        ImageCaption = "caption",
        /// An image's alternative text.
        ImageAlt = "alt",
        /// An image's alignment, the byte of `ImageAlign`.
        ImageAlign = "align",
        /// The address the media flow gave an image's bytes.
        ContentHash = "content_hash",
    }
}

/// The attributes the schema declares and the schema scenario does not write.
/// The images Story writes every image attribute, so none is left.
#[cfg(test)]
pub(crate) const UNWRITTEN_ATTRS: [AttrKey; 0] = [];

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
    /// The kinds an annotation entry declares under [`AnnotKey::Kind`], in one
    /// field of one type (`PROSE-031`).
    pub(crate) enum AnnotKind {
        Comment = "comment",
        Footnote = "footnote",
        Suggestion = "suggestion",
    }
}

/// The kind `entry` declares, or `None` for a word this build does not know.
#[must_use]
pub(crate) fn kind_of(entry: &Fields<AnnotKey>) -> Option<AnnotKind> {
    AnnotKind::parse(&read_string(entry, AnnotKey::Kind)?)
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
    Fields::new(root_map(doc, Root::Meta))
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
        .map(Fields::new)
}

/// The width a column takes when the command that makes it names none
/// (`PROSE-FEAT-232`): one inch.
pub const DEFAULT_COLUMN_WIDTH: PrintLength<Pt> = PrintLength::<Pt>::new(72.0);

/// The nine built-in styles. A document written by an older client renders
/// them from the client rather than from stale copies of its own
/// (`PROSE-093`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BuiltInStyle {
    Body,
    Heading(HeadingLevel),
    Code,
    Quote,
}

impl BuiltInStyle {
    /// The nine, body first.
    pub const ALL: [Self; 9] = [
        Self::Body,
        Self::Heading(HeadingLevel::H1),
        Self::Heading(HeadingLevel::H2),
        Self::Heading(HeadingLevel::H3),
        Self::Heading(HeadingLevel::H4),
        Self::Heading(HeadingLevel::H5),
        Self::Heading(HeadingLevel::H6),
        Self::Code,
        Self::Quote,
    ];

    /// The name the tree stores for it.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Body => "body",
            Self::Heading(HeadingLevel::H1) => "heading-1",
            Self::Heading(HeadingLevel::H2) => "heading-2",
            Self::Heading(HeadingLevel::H3) => "heading-3",
            Self::Heading(HeadingLevel::H4) => "heading-4",
            Self::Heading(HeadingLevel::H5) => "heading-5",
            Self::Heading(HeadingLevel::H6) => "heading-6",
            Self::Code => "code",
            Self::Quote => "quote",
        }
    }

    /// The built-in style a style id names, or `None` for a user's own.
    #[must_use]
    pub fn of(style_id: &StyleId) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|style| style.name() == style_id.as_str())
    }
}

/// Whether `style_id` is one of the nine names every build compiles in.
#[must_use]
pub fn is_built_in_style(style_id: &StyleId) -> bool {
    BuiltInStyle::of(style_id).is_some()
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
    tree.get_meta(*id).ok().map(Fields::new)
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
        .is_ok_and(|node| Fields::new(node).contains(NodeKey::Hidden))
}

/// Hide a block and the subtree under it, in place of removing it
/// (`PROSE-118`). The marker names the session that hid it, the one peer whose
/// undo can bring it back.
///
/// # Errors
/// The CRDT engine's refusal.
pub(crate) fn hide_block(doc: &LoroDoc, tree: &LoroTree, id: &TreeID) -> Result<(), WriteError> {
    let node = Fields::new(tree.get_meta(*id)?);
    node.insert(NodeKey::Hidden, doc.peer_id().to_string())?;
    Ok(())
}

/// The marker an undo of a block's creation hides the block under: the same
/// one a deletion writes, naming the session that hid it (`PROSE-134`). The
/// node keeps its identity, so the redo shows the same block, its body still
/// keyed by it.
#[must_use]
pub(crate) fn hiding() -> ambre_crdt::history::Hide {
    ambre_crdt::history::Hide {
        key: loro::InternalString::from(NodeKey::Hidden.as_str()),
        value: |doc| loro::LoroValue::from(doc.peer_id().to_string()),
        body: |id| {
            Some(loro::ContainerID::new_root(
                BodyKey::of(&id).as_str(),
                loro::ContainerType::Text,
            ))
        },
    }
}

/// The type a live block carries, or `None` when the id names no live block or
/// the discriminant is one this build does not know.
#[must_use]
pub fn block_type_of(tree: &LoroTree, id: &TreeID) -> Option<BlockType> {
    let node = node_of(tree, id)?;
    let code = u8::try_from(read_i64(&node, NodeKey::Type)?).ok()?;
    BlockType::from_code(code)
}

/// The level a live heading carries, exactly as the document holds it
/// ([`StoredLevel`]). `None` for a block that is not a live heading, or whose
/// level is missing or does not fit a byte.
#[must_use]
pub(crate) fn heading_level_of(tree: &LoroTree, id: &TreeID) -> Option<StoredLevel> {
    if block_type_of(tree, id) != Some(BlockType::Heading) {
        return None;
    }
    let attrs = attrs_read(tree, id)?;
    u8::try_from(read_i64(&attrs, AttrKey::Level)?)
        .ok()
        .map(StoredLevel::from_stored)
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
        2 => Some(ListStyle::Todo(crate::events::TodoMark::done_if(
            read_bool(&attrs, AttrKey::Ticked)?,
        ))),
        _ => None,
    }
}

/// A live block's attribute map, or `None` when it carries none. Read-only: it
/// never mints the map it looked for (`PROSE-079`).
#[must_use]
pub(crate) fn attrs_read(tree: &LoroTree, id: &TreeID) -> Option<Fields<AttrKey>> {
    map_read(&node_of(tree, id)?, NodeKey::Attrs).map(Fields::new)
}

/// A block's attribute map, minted when the node holds none, for the same
/// reason as [`map_at`].
pub(crate) fn attrs_at(node: &Fields<NodeKey>) -> Option<Fields<AttrKey>> {
    map_at(node, NodeKey::Attrs).map(Fields::new)
}

/// The two settings a live table of contents carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct TocSettings {
    /// The deepest level it lists.
    pub max_level: HeadingLevel,
    /// Whether it numbers its entries.
    pub numbered: TocNumbering,
}

/// The settings of a live table of contents.
///
/// A missing or unreadable depth reads as six and a missing switch as off, so
/// a block an older client wrote still lists something. A stored depth
/// outside one to six reads as the nearer end: it would list nothing or
/// everything (`PROSE-FEAT-292`).
#[must_use]
pub(crate) fn toc_settings(tree: &LoroTree, id: &TreeID) -> Option<TocSettings> {
    if block_type_of(tree, id) != Some(BlockType::Toc) {
        return None;
    }
    let attrs = attrs_read(tree, id);
    let max_level = attrs
        .as_ref()
        .and_then(|a| read_i64(a, AttrKey::MaxLevel))
        .and_then(|n| u8::try_from(n).ok())
        .map_or(HeadingLevel::MAX, HeadingLevel::clamp);
    let numbered = TocNumbering::numbered_if(
        attrs
            .as_ref()
            .and_then(|a| read_bool(a, AttrKey::Numbered))
            .unwrap_or(false),
    );
    Some(TocSettings {
        max_level,
        numbered,
    })
}

/// A table's column widths list, or `None` when the table carries none.
#[must_use]
pub(crate) fn col_widths_read(tree: &LoroTree, table: &TreeID) -> Option<LoroMovableList> {
    attrs_read(tree, table)?
        .get(AttrKey::ColWidths)
        .and_then(|value| value.into_container().ok())
        .and_then(|container| container.into_movable_list().ok())
}

/// A table's column widths, in points, in column order. A stored width that
/// is not a finite number reads as absent, and takes the default width.
#[must_use]
pub(crate) fn col_widths(tree: &LoroTree, table: &TreeID) -> Vec<PrintLength<Pt>> {
    let Some(list) = col_widths_read(tree, table) else {
        return Vec::new();
    };
    (0..list.len())
        .map(|index| {
            let stored = match list.get(index).and_then(|v| v.into_value().ok()) {
                Some(LoroValue::Double(n)) => Some(PrintLength::<Pt>::new(n as f32)),
                Some(LoroValue::I64(n)) => Some(PrintLength::<Pt>::new(n as f32)),
                _ => None,
            };
            stored
                .filter(|width| width.is_finite())
                .unwrap_or(DEFAULT_COLUMN_WIDTH)
        })
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
        let record = FieldValues::<RegionKey>::new(&record);
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
pub fn style_of(tree: &LoroTree, id: &TreeID) -> Option<StyleId> {
    let node = node_of(tree, id)?;
    read_string(&node, NodeKey::Style).map(StyleId::new)
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
    ids.sort_unstable();
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

/// The `meta` register, or `None` while the document holds no operation yet.
///
/// A reader never takes [`meta`] on a blank document: Loro would attach the
/// root it did not find. Laying the document out writes nothing to it
/// (`PROSE-079`). Past the first operation, every Prose document holds its
/// version in `meta` (`PROSE-028`), so taking it attaches nothing.
///
/// The root is taken by its typed identifier, never looked up by name. Before
/// Loro picks a root by name, it builds the value of each root of that name.
/// A tree a peer nested ten thousand deep under the name `meta` overflowed the
/// stack of the Logic thread (`PROSE-141`).
#[must_use]
pub(crate) fn meta_read(doc: &LoroDoc) -> Option<Fields<MetaKey>> {
    let blank = doc.get_pending_txn_len() == 0 && doc.state_frontiers().is_empty();
    (!blank).then(|| meta(doc))
}

/// The block tree, or `None` when the document state holds no operation yet.
///
/// A reader never takes [`blocks`] on a blank document: Loro would attach the
/// root it did not find. Laying the document out writes nothing to it
/// (`PROSE-079`). Past the first operation, every Prose document holds its
/// tree (`PROSE-027`), so taking it attaches nothing. A state that breaks that
/// rule gets an empty tree in its local state, and no operation.
///
/// The lookup by path [`meta_read`] takes is closed to the tree. Before Loro
/// picks a root by name, it builds the value of each candidate. The value of a
/// tree nests one call per level. A chain of nested blocks a peer wrote thus
/// overflowed the stack of the Logic thread. The lookup also cost the size of
/// the whole tree on every read (`PROSE-140`).
#[must_use]
pub(crate) fn blocks_read(doc: &LoroDoc) -> Option<LoroTree> {
    let blank = doc.get_pending_txn_len() == 0 && doc.state_frontiers().is_empty();
    (!blank).then(|| blocks(doc))
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
/// [`RefusalCause::NonFiniteLength`] when a length is not a finite number,
/// before any key is written. The CRDT engine's own refusal of a write.
pub(crate) fn write_paragraph_style(
    node: &Fields<NodeKey>,
    style: &ParagraphStyle,
) -> Result<(), WriteError> {
    let lengths = [
        (NodeKey::IndentFirst, style.indent_first),
        (NodeKey::IndentLeft, style.indent_left),
        (NodeKey::IndentRight, style.indent_right),
        (NodeKey::SpaceBefore, style.space_before),
        (NodeKey::SpaceAfter, style.space_after),
    ];
    // A reader takes a non-finite length for an absent one, so the store
    // never receives one.
    let line_height = style.line_height.map(LineHeight::stored_value);
    if lengths
        .iter()
        .any(|(_, value)| value.is_some_and(|points| !points.is_finite()))
        || line_height.is_some_and(|value| !value.is_finite())
    {
        return Err(WriteError::Refused(RefusalCause::NonFiniteLength));
    }
    if let Some(align) = style.align {
        node.insert(NodeKey::Align, i64::from(align.code()))?;
    }
    for (key, value) in lengths {
        if let Some(points) = value {
            node.insert(key, f64::from(points.get()))?;
        }
    }
    // The store keeps a `bool` per key (`PROSE-030`).
    if let Some(keep) = style.keep_together {
        node.insert(NodeKey::KeepTogether, keep.as_bool())?;
    }
    if let Some(keep) = style.keep_with_next {
        node.insert(NodeKey::KeepWithNext, keep.as_bool())?;
    }
    if let Some(line_height) = style.line_height {
        let Some(map) = map_at(node, NodeKey::LineHeight).map(Fields::<LineHeightKey>::new) else {
            return Err(WriteError::Engine(loro::LoroError::NotFoundError(
                "the line-height register refused a container".into(),
            )));
        };
        map.insert(LineHeightKey::Mode, i64::from(line_height.mode().code()))?;
        map.insert(LineHeightKey::Value, f64::from(line_height.stored_value()))?;
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
        indent_first: read_pt(&node, NodeKey::IndentFirst),
        indent_left: read_pt(&node, NodeKey::IndentLeft),
        indent_right: read_pt(&node, NodeKey::IndentRight),
        space_before: read_pt(&node, NodeKey::SpaceBefore),
        space_after: read_pt(&node, NodeKey::SpaceAfter),
        line_height: read_line_height(&node),
        keep_together: read_bool(&node, NodeKey::KeepTogether).map(KeepTogether::from_bool),
        keep_with_next: read_bool(&node, NodeKey::KeepWithNext).map(KeepWithNext::from_bool),
    }
}

fn read_line_height(node: &Fields<NodeKey>) -> Option<LineHeight> {
    let map: Fields<LineHeightKey> = node.get(NodeKey::LineHeight).and_then(fields_of)?;
    let mode = u8::try_from(read_i64(&map, LineHeightKey::Mode)?)
        .ok()
        .and_then(LineHeightMode::from_code)?;
    let value = read_f64(&map, LineHeightKey::Value)?;
    LineHeight::from_stored(mode, value as f32)
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
    pub fn agrees_with(self, width: PrintLength<Mm>, height: PrintLength<Mm>) -> bool {
        match self {
            Self::Portrait => width <= height,
            Self::Landscape => width >= height,
        }
    }
}

/// A4 portrait: the page a fresh document opens on.
const A4_WIDTH: PrintLength<Mm> = PrintLength::<Mm>::new(210.0);
const A4_HEIGHT: PrintLength<Mm> = PrintLength::<Mm>::new(297.0);
/// The four margins of a fresh document.
const DEFAULT_MARGIN: PrintLength<Mm> = PrintLength::<Mm>::new(20.0);

/// The two text defaults of a fresh document that carry a number.
///
/// The family is a value, not a key. It lives in `DocDefaults::default` alone,
/// because a string constant in this module is read as a schema key
/// (`SCHEMA-018`).
const DEFAULT_FONT_SIZE: FontSize = FontSize::from_whole_points(11);
const DEFAULT_LINE_HEIGHT: LineSpacingMultiple = LineSpacingMultiple::ONE_AND_HALF;

/// The page format a document declares, in millimetres.
///
/// Read out of `meta/page` and converted to pixels by the layout, which is the
/// one place a page number or a measured height exists (`PROSE-065`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PageFormat {
    pub width: PrintLength<Mm>,
    pub height: PrintLength<Mm>,
    pub margin_top: PrintLength<Mm>,
    pub margin_bottom: PrintLength<Mm>,
    pub margin_left: PrintLength<Mm>,
    pub margin_right: PrintLength<Mm>,
}

impl Default for PageFormat {
    /// A4 portrait with the four default margins: what `seed` writes.
    fn default() -> Self {
        Self {
            width: A4_WIDTH,
            height: A4_HEIGHT,
            margin_top: DEFAULT_MARGIN,
            margin_bottom: DEFAULT_MARGIN,
            margin_left: DEFAULT_MARGIN,
            margin_right: DEFAULT_MARGIN,
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
    pub font_family: ambre_core::FontFamily,
    pub font_size: FontSize,
    /// A multiple of the font size, never an absolute length.
    pub line_height: LineSpacingMultiple,
}

impl Default for DocDefaults {
    /// What `seed` writes.
    fn default() -> Self {
        Self {
            font_family: ambre_core::FontFamily::Other("Inter".to_owned()),
            font_size: DEFAULT_FONT_SIZE,
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
        .map(Fields::<DefaultsKey>::new)
    else {
        return fallback;
    };
    DocDefaults {
        font_family: read_string(&defaults, DefaultsKey::FontFamily)
            .filter(|name| !name.is_empty())
            .map_or(fallback.font_family, |name| {
                ambre_core::FontFamily::from_label(&name)
            }),
        font_size: read_f32(&defaults, DefaultsKey::FontSizePt)
            .filter(|number| *number > 0.0)
            .and_then(FontSize::new)
            .map_or(fallback.font_size, |size| {
                size.at_most(crate::mark::FONT_SIZE_MAX)
            }),
        line_height: read_f32(&defaults, DefaultsKey::LineHeight)
            .and_then(LineSpacingMultiple::new)
            .unwrap_or(fallback.line_height),
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
        .map(Fields::<PageKey>::new)
    else {
        return fallback;
    };
    let read = |key: PageKey, default: PrintLength<Mm>| -> PrintLength<Mm> {
        read_f32(&page, key)
            .filter(|number| number.is_finite() && *number >= 0.0)
            .map_or(default, PrintLength::<Mm>::new)
    };
    let width = read(PageKey::WidthMm, fallback.width);
    let height = read(PageKey::HeightMm, fallback.height);
    PageFormat {
        width: if width > PrintLength::<Mm>::ZERO {
            width
        } else {
            fallback.width
        },
        height: if height > PrintLength::<Mm>::ZERO {
            height
        } else {
            fallback.height
        },
        margin_top: read(PageKey::MarginTopMm, fallback.margin_top),
        margin_bottom: read(PageKey::MarginBottomMm, fallback.margin_bottom),
        margin_left: read(PageKey::MarginLeftMm, fallback.margin_left),
        margin_right: read(PageKey::MarginRightMm, fallback.margin_right),
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
    width: PrintLength<Mm>,
    height: PrintLength<Mm>,
) -> bool {
    if !width.is_finite()
        || !height.is_finite()
        || width <= PrintLength::<Mm>::ZERO
        || height <= PrintLength::<Mm>::ZERO
    {
        return false;
    }
    if !orientation.agrees_with(width, height) {
        return false;
    }
    // The register the document already holds: minting a fresh one would drop
    // the four margins `seed` wrote beside these three keys.
    let Some(page) = map_at(&meta(doc), MetaKey::Page).map(Fields::<PageKey>::new) else {
        return false;
    };
    page.insert(PageKey::Orientation, orientation.as_str())
        .is_ok()
        && page
            .insert(PageKey::WidthMm, f64::from(width.get()))
            .is_ok()
        && page
            .insert(PageKey::HeightMm, f64::from(height.get()))
            .is_ok()
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
    meta.insert(MetaKey::Pageless, false).ok()?;
    if !write_page_format(doc, Orientation::Portrait, A4_WIDTH, A4_HEIGHT) {
        return None;
    }
    let page = Fields::new(map_at(&meta, MetaKey::Page)?);
    for key in [
        PageKey::MarginTopMm,
        PageKey::MarginBottomMm,
        PageKey::MarginLeftMm,
        PageKey::MarginRightMm,
    ] {
        page.insert(key, f64::from(DEFAULT_MARGIN.get())).ok()?;
    }
    let defaults = Fields::new(map_at(&meta, MetaKey::Defaults)?);
    let text = DocDefaults::default();
    defaults
        .insert(DefaultsKey::FontFamily, text.font_family.label())
        .ok()?;
    defaults
        .insert(DefaultsKey::FontSizePt, f64::from(text.font_size.points()))
        .ok()?;
    defaults
        .insert(DefaultsKey::LineHeight, f64::from(text.line_height.get()))
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
    let node = Fields::new(tree.get_meta(id).ok()?);
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
    pub header_enabled: BandState,
    pub footer_enabled: BandState,
    pub different_first_page: FirstPage,
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
    pub fn on_page(&self, page: Pos<crate::events::Page>) -> (Option<Band>, Option<Band>) {
        let first = page == Pos::<crate::events::Page>::ZERO && self.different_first_page.has_own();
        let (header, footer) = if first {
            (self.header_first, self.footer_first)
        } else {
            (self.header, self.footer)
        };
        let drawn =
            |band: Band, shown: BandState| (shown.is_on() && band.block.is_some()).then_some(band);
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

ambre_core::crdt_names! {
    /// The alignments a band takes, by the name its register stores. A band
    /// is never justified (`PROSE-FEAT-171`).
    pub(crate) enum BandAlign {
        Left = "left",
        Center = "center",
        Right = "right",
    }
}

impl BandAlign {
    /// The band alignment `align` asks for, or `None` for justify.
    #[must_use]
    pub(crate) const fn of(align: Align) -> Option<Self> {
        match align {
            Align::Left => Some(Self::Left),
            Align::Center => Some(Self::Center),
            Align::Right => Some(Self::Right),
            Align::Justify => None,
        }
    }

    /// The paragraph alignment the band draws with.
    #[must_use]
    pub(crate) const fn align(self) -> Align {
        match self {
            Self::Left => Align::Left,
            Self::Center => Align::Center,
            Self::Right => Align::Right,
        }
    }
}

fn read_band(meta: &Fields<MetaKey>, key: MetaKey) -> Band {
    let Some(map) = map_read(meta, key).map(Fields::<BandKey>::new) else {
        return Band::default();
    };
    let block = read_string(&map, BandKey::Block).and_then(|s| TreeID::try_from(s.as_str()).ok());
    // A name this build does not know draws on the left, as before.
    let align = read_string(&map, BandKey::Align)
        .and_then(|name| BandAlign::parse(&name))
        .map_or(Align::Left, BandAlign::align);
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
        BandState::on_if(
            map_read(&meta, key)
                .is_some_and(|map| read_bool(&Fields::new(map), BandKey::Enabled) == Some(true)),
        )
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
        different_first_page: FirstPage::own_if(
            read_bool(&meta, MetaKey::DifferentFirstPage) == Some(true),
        ),
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
pub fn read_updated_at(doc: &LoroDoc) -> Option<Instant<Authored>> {
    meta_read(doc)
        .and_then(|meta| read_i64(&meta, MetaKey::UpdatedAt))
        .map(Instant::<Authored>::from_i64)
}

/// The document title, empty when none was set.
#[must_use]
pub fn read_title(doc: &LoroDoc) -> String {
    meta_read(doc)
        .and_then(|meta| read_string(&meta, MetaKey::Title))
        .unwrap_or_default()
}

/// The document locale, or `None` when the register is absent or holds no
/// well-formed tag. A peer may have written any string there; a malformed one
/// reads as no language rather than as a fixed one.
#[must_use]
pub fn read_locale(doc: &LoroDoc) -> Option<Validated<LangTag>> {
    meta_read(doc)
        .and_then(|meta| read_string(&meta, MetaKey::Locale))
        .and_then(|text| Validated::parse(text).ok())
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

/// A length in points stored under `key`. A value that is not a finite number
/// reads as absent, so the default style applies and the document still opens.
fn read_pt<K: FieldKey>(map: &Fields<K>, key: K) -> Option<PrintLength<Pt>> {
    read_f32(map, key)
        .map(PrintLength::<Pt>::new)
        .filter(|points| points.is_finite())
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

/// One character carrying an anchor mark: where it sits and the mark's value,
/// parsed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Anchor<V> {
    pub block: TreeID,
    /// A count of code points into the block's body.
    pub offset: Pos<DocChar>,
    pub value: V,
}

/// Every character carrying the mark `root`, in document order, with its
/// value as `parse` reads it. The mark is a footnote call or a cross
/// reference, whose key is its root alone. A value `parse` refuses names
/// nothing, and its character is left out.
#[must_use]
pub(crate) fn anchors<V: Clone>(
    document: &LoroDoc,
    root: crate::mark::MarkRoot,
    parse: impl Fn(&str) -> Option<V>,
) -> Vec<Anchor<V>> {
    let key = root.as_str();
    let tree = blocks(document);
    let mut out = Vec::new();
    for id in flow(&tree) {
        if !block_type_of(&tree, &id).is_some_and(BlockType::has_body) {
            continue;
        }
        let mut offset = Pos::<DocChar>::ZERO;
        for span in get_block_text(document, &id).to_delta() {
            let loro::TextDelta::Insert { insert, attributes } = span else {
                continue;
            };
            let length = ambre_core::char_len(&insert).as_axis::<DocChar>();
            let value = attributes
                .as_ref()
                .and_then(|attributes| attributes.get(key))
                .and_then(|value| match value {
                    LoroValue::String(value) => parse(value),
                    _ => None,
                });
            if let Some(value) = value {
                for at in Span::<DocChar>::from_len(offset, length) {
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

    /// A refused write names its cause by variant, and the refusal the
    /// command answers with still reads the sentence the log reads.
    #[test]
    fn a_refused_write_reads_its_cause_sentence() {
        let error = WriteError::Refused(RefusalCause::NoWidthList);
        assert!(matches!(
            error,
            WriteError::Refused(RefusalCause::NoWidthList)
        ));
        assert_eq!(error.to_string(), "the table carries no width list");
        assert_eq!(
            crate::message::ProseRefusal::from(WriteError::Refused(RefusalCause::TreeRefusedRow)),
            crate::message::ProseRefusal::WriteFailed
        );
    }

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
            .map(Fields::<PageKey>::new)
            .expect("seed writes the page register");
        page.insert(PageKey::WidthMm, 0.0)
            .expect("the register takes a double");
        page.insert(PageKey::MarginLeftMm, 5.0)
            .expect("the register takes a double");
        let format = read_page_format(&doc);
        assert_eq!(format.width, PageFormat::default().width);
        assert_eq!(format.margin_left, PrintLength::<Mm>::new(5.0));
    }

    /// A default size a peer stored past the cap reads as the cap
    /// (`CANVAS-088`).
    #[test]
    fn a_default_size_past_the_cap_reads_as_the_cap() {
        let doc = LoroDoc::new();
        seed(&doc);
        let defaults = map_at(&meta(&doc), MetaKey::Defaults)
            .map(Fields::<DefaultsKey>::new)
            .expect("seed writes the defaults register");
        defaults
            .insert(DefaultsKey::FontSizePt, 1.0e6)
            .expect("the register takes a double");
        assert_eq!(read_defaults(&doc).font_size, crate::mark::FONT_SIZE_MAX);
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
            PrintLength::<Mm>::new(297.0),
            PrintLength::<Mm>::new(210.0)
        ));
        assert!(write_page_format(
            &doc,
            Orientation::Landscape,
            PrintLength::<Mm>::new(297.0),
            PrintLength::<Mm>::new(210.0)
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

    /// A stored length that is not a finite number reads as absent, so the
    /// default style applies and the document opens.
    #[test]
    fn a_non_finite_stored_length_reads_as_absent() {
        let doc = LoroDoc::new();
        let root = seed(&doc).expect("seeded");
        let tree = blocks(&doc);
        let node = node_of(&tree, &root).expect("a live block");
        node.insert(NodeKey::IndentLeft, f64::NAN).expect("stored");
        node.insert(NodeKey::SpaceAfter, f64::INFINITY)
            .expect("stored");
        node.insert(NodeKey::SpaceBefore, f64::MAX).expect("stored");
        node.insert(NodeKey::IndentRight, 6.0).expect("stored");
        let style = read_paragraph_style(&tree, &root);
        assert_eq!(style.indent_left, None);
        assert_eq!(style.space_after, None);
        assert_eq!(style.space_before, None, "past the largest f32");
        assert_eq!(style.indent_right, Some(PrintLength::<Pt>::new(6.0)));
    }

    /// A write refuses a length that is not a finite number, and writes no
    /// other key of the style.
    #[test]
    fn a_non_finite_length_is_refused_before_any_write() {
        let doc = LoroDoc::new();
        let root = seed(&doc).expect("seeded");
        let tree = blocks(&doc);
        let node = node_of(&tree, &root).expect("a live block");
        for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let refused = write_paragraph_style(
                &node,
                &ParagraphStyle {
                    align: Some(Align::Center),
                    space_before: Some(PrintLength::<Pt>::new(bad)),
                    ..ParagraphStyle::default()
                },
            );
            assert!(
                matches!(
                    refused,
                    Err(WriteError::Refused(RefusalCause::NonFiniteLength))
                ),
                "{bad}"
            );
            assert!(node.get(NodeKey::Align).is_none());
            assert!(node.get(NodeKey::SpaceBefore).is_none());
            let refused = write_paragraph_style(
                &node,
                &ParagraphStyle {
                    align: Some(Align::Center),
                    line_height: Some(LineHeight::Exact(PrintLength::<Pt>::new(bad))),
                    ..ParagraphStyle::default()
                },
            );
            assert!(
                matches!(
                    refused,
                    Err(WriteError::Refused(RefusalCause::NonFiniteLength))
                ),
                "line height {bad}"
            );
            assert!(node.get(NodeKey::Align).is_none());
            assert!(node.get(NodeKey::LineHeight).is_none());
        }
        // The author is told the input is refused, not that the engine failed.
        let refusal =
            crate::message::ProseRefusal::from(WriteError::Refused(RefusalCause::NonFiniteLength));
        assert_eq!(refusal, crate::message::ProseRefusal::NonFiniteLength);
        assert!(!refusal.to_string().contains("engine"), "{refusal}");
    }

    /// A stored line height that is not a finite number reads as no setting,
    /// so the layout never receives one.
    #[test]
    fn a_non_finite_stored_line_height_reads_as_absent() {
        let doc = LoroDoc::new();
        let root = seed(&doc).expect("seeded");
        let tree = blocks(&doc);
        let node = node_of(&tree, &root).expect("a live block");
        for mode in [LineHeight::Exact, LineHeight::AtLeast] {
            write_paragraph_style(
                &node,
                &ParagraphStyle {
                    line_height: Some(mode(PrintLength::<Pt>::new(12.0))),
                    ..ParagraphStyle::default()
                },
            )
            .expect("the write lands");
            assert_eq!(
                read_paragraph_style(&tree, &root).line_height,
                Some(mode(PrintLength::<Pt>::new(12.0)))
            );
            let map = map_at(&node, NodeKey::LineHeight)
                .map(Fields::<LineHeightKey>::new)
                .expect("the register");
            for bad in [f64::NAN, f64::INFINITY, f64::MAX] {
                map.insert(LineHeightKey::Value, bad).expect("stored");
                assert_eq!(
                    read_paragraph_style(&tree, &root).line_height,
                    None,
                    "{bad}"
                );
            }
        }
    }

    /// A stored column width that is not a finite number reads as absent, and
    /// the column takes the default width.
    #[test]
    fn a_non_finite_stored_column_width_reads_as_the_default() {
        let doc = LoroDoc::new();
        let root = seed(&doc).expect("seeded");
        let tree = blocks(&doc);
        let node = node_of(&tree, &root).expect("a live block");
        let attrs = attrs_at(&node).expect("the attribute map");
        let widths = movable_list_at(&attrs, AttrKey::ColWidths).expect("the width list");
        for stored in [f64::NAN, f64::INFINITY, f64::MAX, 72.0] {
            widths.push(stored).expect("stored");
        }
        assert_eq!(
            col_widths(&tree, &root),
            [
                DEFAULT_COLUMN_WIDTH,
                DEFAULT_COLUMN_WIDTH,
                DEFAULT_COLUMN_WIDTH,
                PrintLength::<Pt>::new(72.0)
            ]
        );
    }

    #[test]
    fn the_nine_built_in_style_names_are_not_user_styles() {
        let style = |name: &str| crate::message::StyleId::new(name.to_owned());
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
            PrintLength::<Mm>::new(297.0),
            PrintLength::<Mm>::new(210.0)
        ));
        doc.commit();
        let page = map_at(&meta(&doc), MetaKey::Page)
            .map(Fields::<PageKey>::new)
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

    /// A tree ten thousand deep under the name `meta`, as a peer may write it.
    fn deep_tree_named(doc: &LoroDoc, name: &str) {
        let tree = doc.get_tree(name);
        let mut parent = None;
        for _ in 0..10_000 {
            parent = tree.create(parent).ok();
        }
    }

    /// `PROSE-141`: a peer nested a tree ten thousand deep under the name `meta`. The read of
    /// the `meta` register stays on a stack of 1 MiB. It takes the map root by its typed
    /// identifier, and never builds the value of the tree.
    #[test]
    fn a_deep_tree_under_the_meta_name_reads_on_a_small_stack() {
        let title = std::thread::Builder::new()
            .stack_size(1024 * 1024)
            .spawn(|| {
                let doc = LoroDoc::new();
                crate::schema::stamp(&doc);
                let _ = meta(&doc).insert(MetaKey::Title, "deep");
                deep_tree_named(&doc, Root::Meta.as_str());
                doc.commit();
                meta_read(&doc)
                    .and_then(|meta| meta.get(MetaKey::Title))
                    .and_then(|title| title.into_value().ok())
            })
            .expect("a thread")
            .join()
            .expect("no stack overflow");
        assert_eq!(title, Some(loro::LoroValue::from("deep")));
    }
}
