//! The schema reading belt: what each stored value means, frozen per
//! `SCHEMA_VERSION` (`SCHEMA-017`).
//!
//! The shape belt freezes the keys and the types. An indent stored in points
//! and read in pixels keeps both, so the shape stays green while every older
//! document lays out wrong. This belt freezes the other half: one line per
//! stored value, carrying the value as stored and what this build reads it as.
//!
//! **Two columns, two drifts.** A line reads `path = stored => reading`. A
//! second document that opened the first one's snapshot takes the reading,
//! through the readers the layout and the view use. A reader that moves its unit moves the reading. A writer and a
//! reader that move together leave the reading alone and move the stored
//! value. Either way the frozen text turns red under an unchanged
//! `SCHEMA_VERSION`.
//!
//! **Absolute units.** A reading names its unit: a length in px, a font size
//! in pt, a colour as `#rrggbbaa`. A block is named by its place in the tree,
//! one-based from the root, so the line says which block it is about.
//!
//! **What no reader reads.** A few registers are written and read by nothing
//! in this build. [`UNREAD`] names them, and their line says so. A reader
//! landing for one of them moves its line, which is the review it is owed.
//!
//! **What holds the freeze honest.** A stored value with no reading fails
//! [`every_stored_value_has_a_reading`], and a declared key the scenario does
//! not reach fails [`every_declared_key_has_a_reading`]. The scenario is the
//! shape belt's, so the two belts read the same document. Its values are
//! frozen here, so a command added to it lands with a raised version.

use ambre_bus::app::prose::DocOffset;
use ambre_core::Rgba;
use std::collections::BTreeSet;

use ambre_core::Uuid;
use ambre_core::schema::verdict;
use loro::{LoroValue, TreeID, TreeParentId};

use super::schema_shape::{
    ANY, ANY_BODY, declared_keys, declared_mark_roots, document_of_every_key, section_of,
};
use crate::doc;
use crate::events::{ParagraphStyle, block_id};
use crate::layout::geometry::PageBox;
use crate::layout::measure::EffectiveStyle;
use crate::layout::pass::{cell_style, column_widths_px};
use crate::logic::ProseState;
use crate::schema::SCHEMA_VERSION;
use crate::style::ResolvedStyle;

/// The frozen readings, one section per `SCHEMA_VERSION`.
const FROZEN: &str = include_str!("../../schema/readings.txt");

/// The foundation's version key, as a pattern.
const VERSION_KEY: &str = ambre_core::schema::MetaKey::SchemaVersion.as_str();

/// The stored paths no reader of this build reads. Each is written, kept and
/// merged, and nothing lays out or draws it yet.
const UNREAD: [&str; 3] = ["meta/page/orientation", "meta/pageless", "styles"];

/// One segment below a root: a map key, or a position in a list.
#[derive(Clone)]
enum Segment {
    Key(String),
    At(usize),
}

/// One stored value: the root it sits under, the segments down to it, and the
/// value. A mark is stored under its body, with the character it starts on.
struct Stored {
    root: String,
    segments: Vec<Segment>,
    value: LoroValue,
    offset: u32,
}

impl Stored {
    /// The map key at `depth`, if the segment there is one.
    fn key(&self, depth: usize) -> Option<&str> {
        match self.segments.get(depth)? {
            Segment::Key(key) => Some(key),
            Segment::At(_) => None,
        }
    }

    /// The list position at `depth`, if the segment there is one.
    fn at(&self, depth: usize) -> Option<usize> {
        match self.segments.get(depth)? {
            Segment::At(at) => Some(*at),
            Segment::Key(_) => None,
        }
    }

    /// The stored string, for a register that holds an id or a name.
    fn string(&self) -> Option<&str> {
        match &self.value {
            LoroValue::String(text) => Some(text),
            _ => None,
        }
    }
}

/// Every stored value of a document: the root registers walked to their
/// leaves, each block's metadata, each body, and the marks laid over it.
fn stored_values(state: &ProseState) -> Vec<Stored> {
    let mut found = Vec::new();
    let LoroValue::Map(roots) = state.doc.get_deep_value() else {
        return found;
    };
    for (root, value) in roots.iter() {
        if doc::Root::parse(root) == Some(doc::Root::Blocks) {
            continue;
        }
        walk(root, Vec::new(), value, &mut found);
        if let Some(block) = doc::parse_body_key(root) {
            marks(state, root, &block, &mut found);
        }
    }
    let tree = doc::blocks(&state.doc);
    for id in tree.nodes() {
        let Ok(meta) = tree.get_meta(id) else {
            continue;
        };
        let at = vec![Segment::Key(id.to_string())];
        walk(
            doc::Root::Blocks.as_str(),
            at,
            &meta.get_deep_value(),
            &mut found,
        );
    }
    found
}

fn walk(root: &str, segments: Vec<Segment>, value: &LoroValue, found: &mut Vec<Stored>) {
    let below = |segment| {
        let mut path = segments.clone();
        path.push(segment);
        path
    };
    match value {
        LoroValue::Map(entries) if !entries.is_empty() => {
            for (key, child) in entries.iter() {
                walk(root, below(Segment::Key(key.clone())), child, found);
            }
        }
        LoroValue::List(items) if !items.is_empty() => {
            for (at, item) in items.iter().enumerate() {
                walk(root, below(Segment::At(at)), item, found);
            }
        }
        _ => found.push(Stored {
            root: root.to_owned(),
            segments,
            value: value.clone(),
            offset: 0,
        }),
    }
}

/// The marks laid over one body, one value per run and key, each with the
/// character its run starts on.
fn marks(state: &ProseState, root: &str, block: &TreeID, found: &mut Vec<Stored>) {
    let mut offset = 0_u32;
    for span in doc::get_block_text(&state.doc, block).to_delta() {
        let loro::TextDelta::Insert { insert, attributes } = span else {
            continue;
        };
        for (key, value) in attributes.iter().flatten() {
            found.push(Stored {
                root: root.to_owned(),
                segments: vec![Segment::Key(format!("@{key}"))],
                value: value.clone(),
                offset,
            });
        }
        offset = offset.saturating_add(u32::try_from(insert.chars().count()).unwrap_or(0));
    }
}

/// A stored value in the one spelling the freeze uses.
fn stored_text(value: &LoroValue) -> String {
    match value {
        LoroValue::String(text) => format!("{:?}", text.as_str()),
        LoroValue::Binary(bytes) => bytes.iter().map(|b| format!("{b:02x}")).collect(),
        LoroValue::Double(n) => n.to_string(),
        LoroValue::I64(n) => n.to_string(),
        LoroValue::Bool(b) => b.to_string(),
        LoroValue::Map(entries) if entries.is_empty() => "{}".to_owned(),
        other => format!("{other:?}"),
    }
}

/// Whether a key names one entry, a block or an annotation, rather than a
/// schema word.
fn names_one(key: &str) -> bool {
    TreeID::try_from(key).is_ok() || Uuid::parse_str(key).is_ok()
}

/// The path a value is frozen under, as in the shape. A block, a body, an
/// annotation and a list position collapse to [`ANY`]. So does the thread a
/// comment or a suggestion mark carries.
fn frozen_path(stored: &Stored) -> String {
    let mut path = if doc::parse_body_key(&stored.root).is_some() {
        ANY_BODY.to_owned()
    } else {
        stored.root.clone()
    };
    for segment in &stored.segments {
        let word = match segment {
            Segment::Key(key) => match key.split_once(':') {
                Some((mark, id)) if Uuid::parse_str(id).is_ok() => format!("{mark}:{ANY}"),
                _ if names_one(key) => ANY.to_owned(),
                _ => key.clone(),
            },
            Segment::At(_) => ANY.to_owned(),
        };
        path.push('/');
        path.push_str(&word);
    }
    path
}

/// A block, by its place in the tree: one-based, a dot per level.
fn block_name(state: &ProseState, id: &TreeID) -> String {
    let tree = doc::blocks(&state.doc);
    let mut places = Vec::new();
    let mut current = *id;
    loop {
        let parent = tree.parent(current);
        let siblings = tree
            .children(parent.unwrap_or(TreeParentId::Root))
            .unwrap_or_default();
        let Some(at) = siblings.iter().position(|sibling| *sibling == current) else {
            return "a detached block".to_owned();
        };
        places.push((at + 1).to_string());
        match parent {
            Some(TreeParentId::Node(up)) => current = up,
            _ => break,
        }
    }
    places.reverse();
    format!("block {}", places.join("."))
}

/// A packed `0xRRGGBBAA` colour.
fn colour(rgba: Rgba) -> String {
    format!("#{:08x}", rgba.bits())
}

/// An author as it was stored: its text, or empty when the annotation carries
/// none. The frozen reading of an author stays the stored text.
fn author_text(author: Option<&ambre_core::DisplayName>) -> &str {
    author.map_or("", ambre_core::DisplayName::as_str)
}

/// What `reader` reads a stored value as, or `None` when no reader of this
/// build resolves it.
fn reading(reader: &ProseState, stored: &Stored) -> Option<String> {
    if UNREAD.contains(&frozen_path(stored).as_str()) {
        return Some("read by no reader of this build".to_owned());
    }
    if let Some(block) = doc::parse_body_key(&stored.root) {
        return match stored.key(0) {
            Some(mark) => mark_reading(reader, &block, mark.strip_prefix('@')?, stored),
            None => {
                let name = block_name(reader, &block);
                // A container keeps the body every block is given, and no
                // reader opens it.
                let tree = doc::blocks(&reader.doc);
                if doc::is_hidden(&tree, &block) {
                    return Some(format!(
                        "{name} hidden, so read by no reader until restored"
                    ));
                }
                let kind = doc::block_type_of(&tree, &block)?;
                Some(match reader.text_of(&block_id(block)) {
                    Some(text) => format!("{name} reads {text:?}"),
                    None => format!("{name} is a {kind:?}, whose body no reader opens"),
                })
            }
        };
    }
    match doc::Root::parse(&stored.root)? {
        doc::Root::Meta => meta_reading(reader, stored),
        doc::Root::Blocks => block_reading(reader, stored),
        doc::Root::Annots => annotation_reading(reader, stored),
        doc::Root::Styles => None,
    }
}

/// The reading of a document register, through the readers the layout and
/// the bands take.
fn meta_reading(reader: &ProseState, stored: &Stored) -> Option<String> {
    let document = &reader.doc;
    let stored_key = stored.key(0)?;
    if stored_key == VERSION_KEY {
        let LoroValue::I64(n) = stored.value else {
            return None;
        };
        let version = ambre_core::schema::SchemaVersion::new(u32::try_from(n).ok()?);
        return Some(format!(
            "version {version}, {:?}",
            verdict(version, SCHEMA_VERSION)
        ));
    }
    let key = doc::MetaKey::parse(stored_key)?;
    match key {
        doc::MetaKey::Title => Some(format!("titled {:?}", doc::read_title(document))),
        doc::MetaKey::Lang => Some(doc::read_lang(document).map_or_else(
            || "in no well-formed language".to_owned(),
            |tag| format!("in {:?}", tag.as_str()),
        )),
        doc::MetaKey::UpdatedAt => {
            let date = crate::band::civil_date(doc::read_updated_at(document)?);
            Some(format!("last modified {date} UTC"))
        }
        doc::MetaKey::Page => {
            let page = PageBox::from_mm(doc::read_page_format(document));
            let length = match doc::PageKey::parse(stored.key(1)?)? {
                doc::PageKey::WidthMm => page.page_width_px,
                doc::PageKey::HeightMm => page.page_height_px,
                doc::PageKey::MarginLeftMm => page.margin_left_px,
                doc::PageKey::MarginTopMm => page.margin_top_px,
                doc::PageKey::MarginRightMm => {
                    page.page_width_px - page.margin_left_px - page.content_width_px
                }
                doc::PageKey::MarginBottomMm => {
                    page.page_height_px - page.margin_top_px - page.content_height_px
                }
                doc::PageKey::Orientation => return None,
            };
            Some(format!("{} is {length} px", stored.key(1)?))
        }
        doc::MetaKey::Defaults => {
            let defaults = doc::read_defaults(document);
            let style = EffectiveStyle::from_parts(
                &defaults,
                &ParagraphStyle::default(),
                crate::layout::measure::Weight::Regular,
            );
            match doc::DefaultsKey::parse(stored.key(1)?)? {
                doc::DefaultsKey::FontFamily => {
                    Some(format!("body set in {:?}", style.font_family))
                }
                doc::DefaultsKey::FontSizePt => {
                    Some(format!("body set at {} px", style.font_size_px))
                }
                doc::DefaultsKey::LineHeight => Some(format!("body lines {:?}", style.line_height)),
            }
        }
        doc::MetaKey::DifferentFirstPage => Some(format!(
            "first page has its own bands {}",
            doc::read_bands(document).different_first_page
        )),
        doc::MetaKey::Header
        | doc::MetaKey::Footer
        | doc::MetaKey::HeaderFirst
        | doc::MetaKey::FooterFirst => {
            let bands = doc::read_bands(document);
            let (band, enabled) = match key {
                doc::MetaKey::Header => (bands.header, Some(bands.header_enabled)),
                doc::MetaKey::Footer => (bands.footer, Some(bands.footer_enabled)),
                doc::MetaKey::HeaderFirst => (bands.header_first, None),
                _ => (bands.footer_first, None),
            };
            let key = key.as_str();
            match doc::BandKey::parse(stored.key(1)?)? {
                doc::BandKey::Enabled => Some(format!("{key} shown {}", enabled?)),
                doc::BandKey::Block => Some(format!(
                    "{key} drawn from {}",
                    block_name(reader, &band.block?)
                )),
                doc::BandKey::Align => Some(format!("{key} aligned {:?}", band.align)),
            }
        }
        _ => None,
    }
}

/// The reading of a value in one block's metadata, through the style the
/// layout measures the block with and the table the layout draws.
fn block_reading(reader: &ProseState, stored: &Stored) -> Option<String> {
    let document = &reader.doc;
    let tree = doc::blocks(document);
    let id = TreeID::try_from(stored.key(0)?).ok()?;
    let name = block_name(reader, &id);
    // A hidden block is skipped by every reader until an undo restores it.
    let field = doc::NodeKey::parse(stored.key(1)?)?;
    if doc::is_hidden(&tree, &id) && field != doc::NodeKey::Hidden {
        return Some(format!(
            "{name} hidden, so read by no reader until restored"
        ));
    }
    let style = EffectiveStyle::resolve(
        &doc::read_defaults(document),
        &tree,
        &id,
        crate::layout::measure::Weight::Regular,
    );
    let what = match field {
        doc::NodeKey::Type => format!("is a {:?}", doc::block_type_of(&tree, &id)?),
        doc::NodeKey::Style => format!("styled {:?}", doc::style_of(&tree, &id)?.0),
        doc::NodeKey::Hidden => format!("hidden {}", doc::is_hidden(&tree, &id)),
        doc::NodeKey::Regrown => format!(
            "counted among the grown empty roots {}",
            doc::grown_empty_roots(document, &tree).contains(&id)
        ),
        doc::NodeKey::Band => format!("is a band {}", doc::is_band(&tree, &id)),
        doc::NodeKey::Note => format!("is a note {}", doc::is_note(&tree, &id)),
        doc::NodeKey::Align => format!("aligned {:?}", style.align),
        doc::NodeKey::IndentFirst => format!("first line indented {} px", style.indent_first_px),
        doc::NodeKey::IndentLeft => format!("indented {} px on the left", style.indent_left_px),
        doc::NodeKey::IndentRight => format!("indented {} px on the right", style.indent_right_px),
        doc::NodeKey::SpaceBefore => format!("{} px above", style.space_before_px),
        doc::NodeKey::SpaceAfter => format!("{} px below", style.space_after_px),
        doc::NodeKey::LineHeight => format!("lines {:?}", style.line_height),
        doc::NodeKey::KeepTogether => format!("kept together {}", style.keep_together),
        doc::NodeKey::KeepWithNext => format!("kept with the next {}", style.keep_with_next),
        doc::NodeKey::Attrs => attribute_reading(reader, &id, stored)?,
    };
    Some(format!("{name} {what}"))
}

/// The reading of one block attribute: a heading's level, a list's marker, a
/// table's grid, a cell's look.
fn attribute_reading(reader: &ProseState, id: &TreeID, stored: &Stored) -> Option<String> {
    let tree = doc::blocks(&reader.doc);
    let cell = || cell_style(&tree, id);
    let edge = |pick: fn(&crate::layout::run::CellBorderPx) -> f32| {
        cell().border.map_or(0.0, |border| pick(&border))
    };
    Some(match doc::AttrKey::parse(stored.key(2)?)? {
        doc::AttrKey::Level => format!("at level {}", doc::heading_level_of(&tree, id)?),
        doc::AttrKey::Lang => format!("in {:?}", reader.code_lang_of(&block_id(*id))?),
        doc::AttrKey::Marker | doc::AttrKey::Spelling | doc::AttrKey::Ticked => {
            format!("marked {:?}", doc::list_style_of(&tree, id)?)
        }
        doc::AttrKey::MaxLevel | doc::AttrKey::Numbered => {
            let toc = reader.table_of_contents(&block_id(*id))?;
            format!(
                "lists headings to level {}, numbered {}",
                toc.max_level, toc.numbered
            )
        }
        doc::AttrKey::ColWidths => {
            let at = stored.at(3)?;
            let stored_pt = doc::col_widths(&tree, id);
            let content = PageBox::from_mm(doc::read_page_format(&reader.doc)).content_width_px;
            let widths = column_widths_px(&stored_pt, stored_pt.len(), content);
            format!("column {} is {} px wide", at + 1, widths.get(at)?)
        }
        doc::AttrKey::Merged => {
            let anchor = TreeID::try_from(stored.key(3)?).ok()?;
            let region = doc::regions(&tree, id)
                .into_iter()
                .find(|region| region.anchor == anchor)?;
            match doc::RegionKey::parse(stored.key(4)?)? {
                doc::RegionKey::Extent => format!(
                    "merges {} through {}",
                    block_name(reader, &region.anchor),
                    block_name(reader, &region.extent)
                ),
                doc::RegionKey::Stamp => format!("merge stamped {}", region.stamp),
            }
        }
        doc::AttrKey::CellBg => format!("filled {}", colour(cell().fill?)),
        doc::AttrKey::EdgeTop => format!("top edge {} px", edge(|b| b.top)),
        doc::AttrKey::EdgeBottom => format!("bottom edge {} px", edge(|b| b.bottom)),
        doc::AttrKey::EdgeLeft => format!("left edge {} px", edge(|b| b.left)),
        doc::AttrKey::EdgeRight => format!("right edge {} px", edge(|b| b.right)),
        doc::AttrKey::EdgeColor => format!("edged {}", colour(cell().border?.color?)),
        doc::AttrKey::Padding => format!("padded {} px", cell().padding_px),
        doc::AttrKey::VAlign => format!("set {:?}", cell().valign),
        _ => return None,
    })
}

/// The reading of one annotation field, through the thread, the note or the
/// suggestion the view lists.
fn annotation_reading(reader: &ProseState, stored: &Stored) -> Option<String> {
    let id = Uuid::parse_str(stored.key(0)?).ok()?;
    let field = doc::AnnotKey::parse(stored.key(1)?)?;
    if let Some(thread) = reader
        .comments()
        .into_iter()
        .find(|t| *t.comment_id.as_uuid() == id)
    {
        let anchor = thread.anchor_block.map(crate::events::tree_id);
        return Some(match field {
            doc::AnnotKey::Kind => "a comment thread".to_owned(),
            doc::AnnotKey::AnchorBlock => format!("on {}", block_name(reader, &anchor?)),
            doc::AnnotKey::AnchorStart => format!("from character {}", thread.anchor_start.get()),
            doc::AnnotKey::AnchorEnd => format!("to character {}", thread.anchor_end.get()),
            doc::AnnotKey::Resolved => format!("resolved {}", thread.resolved),
            doc::AnnotKey::Replies => {
                let at = stored.at(2)?;
                let reply = thread.replies.get(at)?;
                let what = match doc::ReplyKey::parse(stored.key(3)?)? {
                    doc::ReplyKey::Id => format!("is {}", reply.reply_id.to_hyphenated()),
                    doc::ReplyKey::Author => format!("by {:?}", author_text(reply.author.as_ref())),
                    doc::ReplyKey::User => format!("by user {}", reply.user_id),
                    doc::ReplyKey::Created => format!("at {} ms", reply.created_at.0),
                    doc::ReplyKey::Text => format!("reads {:?}", reply.text),
                };
                format!("reply {} {what}", at + 1)
            }
            _ => return None,
        });
    }
    if let Some(note) = reader
        .footnotes()
        .into_iter()
        .find(|n| *n.footnote_id.as_uuid() == id)
    {
        return Some(match field {
            doc::AnnotKey::Kind => format!("footnote {:?}", note.ordinal),
            doc::AnnotKey::NoteBlock => format!("note reads {:?}", note.text),
            _ => return None,
        });
    }
    let suggestion = reader
        .suggestions()
        .into_iter()
        .find(|s| *s.suggestion_id.as_uuid() == id)?;
    Some(match field {
        doc::AnnotKey::Kind => format!("a suggested {:?}", suggestion.kind),
        doc::AnnotKey::Author => {
            format!("suggested by {:?}", author_text(suggestion.author.as_ref()))
        }
        doc::AnnotKey::User => format!("suggested by user {}", suggestion.user_id),
        doc::AnnotKey::NoteBlock => {
            let span = suggestion.spans.first()?;
            format!(
                "first span on {}",
                block_name(reader, &crate::events::tree_id(span.block_id))
            )
        }
        _ => return None,
    })
}

/// What a formatting mark makes of the run it lies on, read from the run the
/// cascade resolved.
fn formatting(style: &ResolvedStyle, mark: &str) -> Option<String> {
    let ResolvedStyle {
        bold,
        italic,
        underline,
        strikethrough,
        superscript,
        subscript,
        code_inline,
        color,
        background,
        font_size,
        font_family,
        link,
    } = style;
    Some(match mark {
        "b" => format!("bold {bold}"),
        "i" => format!("italic {italic}"),
        "u" => format!("underlined {underline}"),
        "s" => format!("struck {strikethrough}"),
        "sup" => format!("raised {superscript}"),
        "sub" => format!("lowered {subscript}"),
        "c" => format!("inline code {code_inline}"),
        "fg" => format!("ink {}", colour((*color)?)),
        "bg" => format!("highlight {}", colour((*background)?)),
        "fs" => format!("{font_size} pt"),
        "ff" => format!("set in {:?}", font_family.as_ref()?),
        "a" => format!("links to {:?}", link.as_ref()?),
        _ => return None,
    })
}

/// The reading of one mark, from where it starts.
fn mark_reading(
    reader: &ProseState,
    block: &TreeID,
    mark: &str,
    stored: &Stored,
) -> Option<String> {
    let name = block_name(reader, block);
    let at = stored.offset;
    let what = if mark.starts_with("f:") {
        let pages = reader
            .full_layout()
            .map_or(crate::events::PageCount::ONE, |layout| layout.page_count);
        format!(
            "band shows {:?} on page 1",
            crate::band::resolve(&reader.doc, block, crate::events::PageIndex::ZERO, pages)
        )
    } else if mark == doc::K_FOOTNOTE {
        let id = ambre_core::FootnoteId::parse_str(stored.string()?).ok();
        match reader
            .footnotes()
            .into_iter()
            .find(|n| Some(n.footnote_id) == id)
        {
            Some(note) => format!("calls note {:?}", note.ordinal),
            None => "calls no note".to_owned(),
        }
    } else if mark == doc::K_CROSS_REF {
        let resolved = crate::xref::resolve_all(&reader.doc, reader.full_layout());
        match resolved.into_iter().find(|r| {
            crate::events::tree_id(r.block_id) == *block && r.offset == DocOffset::new(at)
        }) {
            Some(found) => format!(
                "refers to {} showing {:?}",
                block_name(reader, &crate::events::tree_id(found.target)),
                found.shown
            ),
            None => "refers to nothing".to_owned(),
        }
    } else if let Some(thread) = mark.strip_prefix("cm:") {
        let id = ambre_core::CommentId::parse_str(thread).ok();
        match reader
            .comments()
            .into_iter()
            .find(|t| Some(t.comment_id) == id)
        {
            Some(thread) => format!(
                "anchors a thread over characters {}..{}",
                thread.anchor_start.get(),
                thread.anchor_end.get()
            ),
            None => "anchors no thread".to_owned(),
        }
    } else if let Some((kind, id)) = crate::suggest::of_key(mark) {
        match reader
            .suggestions()
            .into_iter()
            .find(|s| s.suggestion_id == id)
        {
            Some(found) => format!(
                "suggests {kind:?} by {:?}",
                author_text(found.author.as_ref())
            ),
            None => "suggests nothing".to_owned(),
        }
    } else if matches!(mark, "cm" | "si" | "sd") {
        // The bare root carries no thread: a reader keys its anchors by one.
        "anchors nothing".to_owned()
    } else {
        let runs = reader.resolved_runs(&block_id(*block));
        let mut start = 0_u32;
        let mut covering = None;
        for (text, style) in &runs {
            let end = start.saturating_add(u32::try_from(text.chars().count()).unwrap_or(0));
            if (start..end).contains(&at) {
                covering = Some(style);
                break;
            }
            start = end;
        }
        formatting(covering?, mark)?
    };
    Some(format!("{name} from character {at}: {what}"))
}

/// A document opened from `state`'s snapshot, through the full-state read
/// every reload takes, and laid out once as the view would.
fn reopened(state: &ProseState) -> ProseState {
    let mut reader = ProseState::with_peer(ambre_core::PeerId(2)).expect("a document is built");
    reader
        .import_snapshot(&state.export_snapshot())
        .expect("a document this build wrote reopens");
    reader.paginate(&[]);
    reader
}

/// A document's readings, as one reader reads them.
#[derive(Default)]
struct Readings {
    /// The lines the freeze holds.
    lines: BTreeSet<String>,
    /// The stored values no reading resolved.
    unread: Vec<String>,
    /// The roots and the keys above a resolved value, which the coverage
    /// assertion answers from.
    keys: BTreeSet<String>,
}

/// The readings of a document as `reader` reads it.
fn readings_of(state: &ProseState, reader: &ProseState) -> Readings {
    let mut readings = Readings::default();
    for stored in stored_values(state) {
        let path = frozen_path(&stored);
        let value = stored_text(&stored.value);
        match reading(reader, &stored) {
            Some(read) => {
                readings.lines.insert(format!("{path} = {value} => {read}"));
                readings.keys.extend(
                    path.split('/')
                        .map(|segment| segment.trim_start_matches('@').to_owned()),
                );
            }
            None => readings.unread.push(format!("{path} = {value}")),
        }
    }
    readings
}

fn render(lines: &BTreeSet<String>) -> String {
    lines.iter().map(|line| format!("{line}\n")).collect()
}

#[test]
fn the_reading_a_build_makes_is_the_reading_frozen_for_its_version() {
    let state = document_of_every_key();
    let written = render(&readings_of(&state, &reopened(&state)).lines);
    let Some(frozen) = section_of(FROZEN, SCHEMA_VERSION) else {
        panic!(
            "`schema/readings.txt` carries no `== v{SCHEMA_VERSION}` section. \
             The version rose without a freeze; add the section below:\n{written}"
        );
    };
    assert_eq!(
        written, frozen,
        "what this build stores or reads is not what was frozen for \
         SCHEMA_VERSION {SCHEMA_VERSION}. A value moved its unit or its \
         meaning, and every document written before reads wrong. Raise \
         SCHEMA_VERSION and freeze the new reading as its own section \
         (`SCHEMA-017`)."
    );
}

#[test]
fn every_stored_value_has_a_reading() {
    let state = document_of_every_key();
    let unread = readings_of(&state, &reopened(&state)).unread;
    assert!(
        unread.is_empty(),
        "no reader of this build resolves these stored values, so the freeze \
         holds nothing of what they mean. Write their reading in \
         `schema_reading.rs`, or name them in `UNREAD`:\n{}",
        unread.join("\n")
    );
}

#[test]
fn every_declared_key_has_a_reading() {
    let state = document_of_every_key();
    let keys = readings_of(&state, &reopened(&state)).keys;
    let mut declared = declared_keys();
    declared.extend(declared_mark_roots());
    for key in declared {
        let reached = keys.contains(&key)
            || keys
                .iter()
                .any(|seen| seen.split(':').next() == Some(key.as_str()));
        assert!(
            reached,
            "`{key}` is declared as a schema key and no reading carries it"
        );
    }
}

#[test]
fn a_stored_value_moving_moves_the_reading() {
    let state = document_of_every_key();
    let before = readings_of(&state, &reopened(&state)).lines;
    // The same indent written in pixels: the key and the type stay, so only
    // the value column can show it.
    let mut moved = document_of_every_key();
    let root = moved.root_blocks()[0];
    moved.handle_command(crate::events::ProseCommand::SetParagraphStyle {
        block_id: root,
        style: ParagraphStyle {
            indent_left: Some(2.0 * 96.0 / 72.0),
            ..ParagraphStyle::default()
        },
    });
    let after = readings_of(&moved, &reopened(&moved)).lines;
    assert_ne!(render(&before), render(&after));
}

#[test]
fn a_reader_moving_moves_the_reading() {
    let state = document_of_every_key();
    let before = readings_of(&state, &reopened(&state)).lines;
    // A reader that takes the stored page as inches rather than millimetres.
    // The stored bytes are the same; only what this build makes of them moved.
    // The page box is read from the document, so the reader is moved by
    // rewriting what it reads, on the reader's copy alone.
    let misread = reopened(&state);
    let page = doc::map_read(&doc::meta(&misread.doc), doc::MetaKey::Page)
        .map(doc::Fields::of)
        .expect("the document carries a page");
    page.insert(doc::PageKey::WidthMm, 210.0 * 25.4)
        .expect("the register takes a value");
    misread.doc.commit();
    let after = readings_of(&state, &misread).lines;
    assert_ne!(
        render(&before),
        render(&after),
        "a reader moving its unit has to move the reading, or the freeze \
         holds only the bytes"
    );
}
