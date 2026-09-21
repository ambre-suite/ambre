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

use loro::{LoroDoc, LoroMap, LoroText, LoroTree, LoroValue, TreeID, TreeParentId};

use ambre_bus::app::prose::{Align, BlockType, LineHeight, LineHeightMode, ParagraphStyle};

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

/// The heading level attribute, `1..=6`.
pub(crate) const A_LEVEL: &str = "level";
/// The code block's language. An empty value reads as plain.
pub(crate) const A_LANG: &str = "lang";

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
pub fn is_built_in_style(style_id: &str) -> bool {
    BUILT_IN_STYLES.contains(&style_id)
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
    if tree.is_node_deleted(id).unwrap_or(true) {
        return None;
    }
    tree.get_meta(*id).ok()
}

/// The type a live block carries, or `None` when the id names no live block or
/// the discriminant is one this build does not know.
#[must_use]
pub fn block_type_of(tree: &LoroTree, id: &TreeID) -> Option<BlockType> {
    let node = node_of(tree, id)?;
    let code = u8::try_from(read_i64(&node, K_TYPE)?).ok()?;
    BlockType::from_code(code)
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
    tree.children(parent).unwrap_or_default()
}

/// The live root blocks of the body tree.
#[must_use]
pub(crate) fn roots(tree: &LoroTree) -> Vec<TreeID> {
    children_of(tree, None)
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
        let map = node.insert_container(K_LINE_HEIGHT, LoroMap::new())?;
        map.insert(LH_MODE, i64::from(line_height.mode.code()))?;
        map.insert(LH_VALUE, f64::from(line_height.value))?;
    }
    Ok(())
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
    let Ok(page) = doc
        .get_map(META)
        .insert_container(PAGE, LoroMap::new())
        .map_err(|e| tracing::warn!("Prose: the page register refused a write: {e}"))
    else {
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
    let page = meta
        .get(PAGE)
        .and_then(|v| v.into_container().ok())
        .and_then(|c| c.into_map().ok())?;
    for key in [
        "margin_top_mm",
        "margin_bottom_mm",
        "margin_left_mm",
        "margin_right_mm",
    ] {
        page.insert(key, f64::from(DEFAULT_MARGIN_MM)).ok()?;
    }
    let defaults = meta.insert_container(DEFAULTS, LoroMap::new()).ok()?;
    defaults.insert("font_family", "Inter").ok()?;
    defaults.insert("font_size_pt", 11.0).ok()?;
    defaults.insert("line_height", 1.5).ok()?;
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
// Value readers
// -----------------------------------------------------------------------------

fn read_i64(map: &LoroMap, key: &str) -> Option<i64> {
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

fn read_bool(map: &LoroMap, key: &str) -> Option<bool> {
    match map.get(key)?.into_value() {
        Ok(LoroValue::Bool(b)) => Some(b),
        _ => None,
    }
}

fn read_string(map: &LoroMap, key: &str) -> Option<String> {
    match map.get(key)?.into_value() {
        Ok(LoroValue::String(s)) => Some(s.to_string()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert!(is_built_in_style("quote"));
        assert!(is_built_in_style("heading-6"));
        assert!(!is_built_in_style("my-style"));
    }
}
