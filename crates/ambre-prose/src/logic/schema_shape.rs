//! The schema shape belt: the key set and the value types a build writes,
//! frozen per `SCHEMA_VERSION` (`SCHEMA-018`).
//!
//! A shape is the document with every value removed: one line per reachable
//! path, each naming the type the value under it holds. Adding a key, dropping
//! one, or changing an `i64` into an `f64` moves a line; changing a title from
//! `"a"` to `"b"` does not. The frozen text is therefore a diff a reviewer
//! reads, and a build that writes a new shape under an unchanged
//! `SCHEMA_VERSION` turns red.
//!
//! **What holds the freeze honest.** A shape is only as wide as the scenario
//! that builds it. A key no command reaches leaves no line, so dropping it
//! would go unnoticed. [`every_declared_key_appears_in_the_shape`] closes that
//! gap: it demands a line for each key the schema enums of `doc.rs` and the
//! mark registry declare. A new key with no scenario behind it fails there
//! rather than passing silently.
//!
//! **What it cannot hold.** A value whose meaning moves under an unchanged key
//! and an unchanged type leaves the shape identical (`SCHEMA-017`). That half
//! is the reading belt's, in `schema_reading.rs`, over this same scenario.

use crate::message::DocChar;
use crate::message::MarkKey;
use ambre_core::crdt::CrdtName;
use ambre_core::index::{Len, Pos, Span};
use std::collections::BTreeSet;

use ambre_core::schema::{migration_peer, step_fingerprint};
use loro::{ExportMode, LoroDoc, LoroValue, TreeID};

use crate::doc;
use crate::events::HeadingLevel;
use crate::events::{
    Align, BlockInit, CellBorder, LineHeight, LineSpacingMultiple, ListStyle, OrderedStyle,
    ParagraphStyle, ProseCommand, VAlign, tree_id,
};
use crate::logic::ProseState;
use crate::schema::{MIGRATIONS, SCHEMA_VERSION};
use ambre_core::{Authored, Instant, PrintLength, Pt};

/// The frozen shapes, one section per `SCHEMA_VERSION`.
const FROZEN: &str = include_str!("../../schema/shapes.txt");

/// The frozen scaffolding identities, one section per `SCHEMA_VERSION`.
const SCAFFOLD: &str = include_str!("../../schema/scaffold.txt");

/// What each migration step writes, one section per `SCHEMA_VERSION`.
const STEPS: &str = include_str!("../../schema/migrations.txt");

/// The source the key declarations are read from, so the coverage assertion
/// cannot drift from the module it guards.
pub(super) const DOC_SOURCE: &str = include_str!("../doc.rs");

/// The segment a tree node, a body container or a user-named register collapses
/// to. Their names carry a `TreeID` or a peer id, so a shape that kept them
/// would differ between two runs of the same build.
pub(super) const ANY: &str = "*";

/// The type a value holds, named in the one spelling the freeze uses.
///
/// Not the Rust type and not Loro's. The number shapes Loro stores stay apart
/// here, because an `i64` key turning into an `f64` one is the drift this belt
/// exists to catch.
fn type_name(value: &LoroValue) -> &'static str {
    match value {
        LoroValue::Null => "null",
        LoroValue::Bool(_) => "bool",
        LoroValue::Double(_) => "f64",
        LoroValue::I64(_) => "i64",
        LoroValue::Binary(_) => "binary",
        LoroValue::String(_) => "string",
        LoroValue::List(_) => "list",
        LoroValue::Map(_) => "map",
        LoroValue::Container(_) => "container",
    }
}

/// A document's shape: the lines the freeze holds, plus the key segments the
/// coverage assertion answers from.
#[derive(Default)]
struct Shape {
    lines: BTreeSet<String>,
    keys: BTreeSet<String>,
}

impl Shape {
    /// Record one path and the type under it.
    fn record(&mut self, path: &str, value: &LoroValue) {
        self.lines.insert(format!("{path}: {}", type_name(value)));
    }
}

/// Walk a map, recording every key and descending into the nested maps.
///
/// A list is recorded and not entered: a table's column widths is the one
/// the schema holds, and its entries are numbers alone. The tree and the rich
/// text have their own readers below. A key naming a block, as a table's
/// region map does, or an annotation's identity, collapses to [`ANY`].
fn walk_map(prefix: &str, value: &LoroValue, shape: &mut Shape) {
    let LoroValue::Map(entries) = value else {
        return;
    };
    for (key, child) in entries.iter() {
        let names_one =
            TreeID::try_from(key.as_str()).is_ok() || ambre_core::Uuid::parse_str(key).is_ok();
        let key = if names_one {
            ANY.to_owned()
        } else {
            key.clone()
        };
        let path = format!("{prefix}/{key}");
        shape.keys.insert(key);
        shape.record(&path, child);
        match child {
            LoroValue::Map(_) => walk_map(&path, child, shape),
            // A list of maps, as a thread's replies are: each item is one
            // record of the same shape, read under [`ANY`].
            LoroValue::List(items) => {
                for item in items.iter() {
                    let item_path = format!("{path}/{ANY}");
                    if matches!(item, LoroValue::Map(_)) {
                        shape.record(&item_path, item);
                        walk_map(&item_path, item, shape);
                    }
                }
            }
            _ => {}
        }
    }
}

/// The shape of a document: the root registers, the tree nodes, and the bodies
/// with the marks laid over them.
fn shape_of(doc: &LoroDoc) -> Shape {
    let mut shape = Shape::default();
    let LoroValue::Map(roots) = doc.get_deep_value() else {
        return shape;
    };

    for (name, value) in roots.iter() {
        if doc::parse_body_key(name).is_some() {
            // Every body is one shape: the name carries a `TreeID`.
            shape.keys.insert(body_tag(name));
            shape.record(ANY_BODY, value);
            continue;
        }
        shape.keys.insert(name.clone());
        shape.record(name, value);
        if name != doc::Root::Blocks.as_str() {
            walk_map(name, value, &mut shape);
        }
    }

    walk_tree(doc, &mut shape);
    walk_marks(doc, &mut shape);
    shape
}

/// The body container, under the one name every body collapses to.
pub(super) const ANY_BODY: &str = "t:*";

/// The field a rich-text run carries its marks under. Loro's naming, not the
/// schema's, so it stays out of the frozen lines.
const RUN_MARKS: &str = "attributes";

/// The tag half of a body key, which is the key `doc.rs` declares.
fn body_tag(name: &str) -> String {
    name.split(':').next().unwrap_or(name).to_owned()
}

/// Every tree node's metadata map, merged into one shape.
///
/// The union is the point: a heading carries `a/level` and a paragraph does
/// not, and the schema holds both.
fn walk_tree(doc: &LoroDoc, shape: &mut Shape) {
    let tree = doc::blocks(doc);
    for id in tree.nodes() {
        let Ok(meta) = tree.get_meta(id) else {
            continue;
        };
        walk_map(
            &format!("{}/{ANY}", doc::Root::Blocks.as_str()),
            &meta.get_deep_value(),
            shape,
        );
    }
}

/// The mark keys laid over the bodies, which no deep value shows.
///
/// A mark lives in the rich-text runs rather than in a map, so the plain value
/// of a body is its characters alone.
fn walk_marks(doc: &LoroDoc, shape: &mut Shape) {
    let LoroValue::Map(roots) = doc.get_deep_value() else {
        return;
    };
    for name in roots.keys() {
        if doc::parse_body_key(name).is_none() {
            continue;
        }
        let LoroValue::List(runs) = doc.get_text(name.as_str()).get_richtext_value() else {
            continue;
        };
        for run in runs.iter() {
            let LoroValue::Map(fields) = run else {
                continue;
            };
            // Loro nests a run's marks under one field of its own naming; the
            // keys the schema declares are the ones inside it.
            let Some(LoroValue::Map(marks)) = fields.get(RUN_MARKS) else {
                continue;
            };
            for (key, value) in marks.iter() {
                // A comment's key carries its thread, `cm:{id}`: the id
                // collapses as an annotation's identity does.
                let key = match key.split_once(':') {
                    Some((root, id)) if ambre_core::Uuid::parse_str(id).is_ok() => {
                        format!("{root}:{ANY}")
                    }
                    _ => key.clone(),
                };
                shape.keys.insert(key.clone());
                shape.record(&format!("{ANY_BODY}/@{key}"), value);
            }
        }
    }
}

/// Render a shape as the freeze holds it: one line per path, sorted.
fn render(shape: &Shape) -> String {
    let mut out = String::new();
    for line in &shape.lines {
        out.push_str(line);
        out.push('\n');
    }
    out
}

/// The section of the freeze that names this version, or `None` when the freeze
/// carries no such section.
fn frozen_section(version: ambre_core::Version<ambre_core::schema::Schema>) -> Option<String> {
    section_of(FROZEN, version)
}

/// The section of a frozen text that names this version.
pub(super) fn section_of(
    frozen: &str,
    version: ambre_core::Version<ambre_core::schema::Schema>,
) -> Option<String> {
    let header = format!("== v{version}");
    let mut taken = String::new();
    let mut inside = false;
    for line in frozen.lines() {
        if line.starts_with("== ") {
            if inside {
                break;
            }
            inside = line.trim_end() == header;
            continue;
        }
        if inside && !line.is_empty() {
            taken.push_str(line);
            taken.push('\n');
        }
    }
    inside.then_some(taken)
}

/// Every `const NAME: &str = "literal";` a source declares.
///
/// The declarations are the schema's key vocabulary, and reading them here is
/// what makes a new key fail the coverage assertion rather than pass unseen.
pub(super) fn declared_string_consts(source: &str) -> Vec<String> {
    let mut found = Vec::new();
    for line in source.lines() {
        let bare = line.trim_start();
        // The declaration's visibility is not part of the key it names.
        let bare = bare
            .split_once("const ")
            .filter(|(before, _)| before.is_empty() || before.starts_with("pub"))
            .map(|(_, rest)| rest);
        let Some(rest) = bare else {
            continue;
        };
        let Some((_, tail)) = rest.split_once(": &str = ") else {
            continue;
        };
        let Some(literal) = tail.strip_prefix('"').and_then(|t| t.split('"').next()) else {
            continue;
        };
        if !literal.is_empty() {
            found.push(literal.to_owned());
        }
    }
    found
}

/// Every name the schema enums of `doc.rs` declare, through their `ALL`.
///
/// The attributes no command writes yet stay out ([`doc::UNWRITTEN_ATTRS`]).
/// The text mark roots are read through [`declared_mark_roots`].
pub(super) fn declared_keys() -> Vec<String> {
    fn names<K: Copy>(all: &[K], spell: fn(K) -> &'static str) -> Vec<String> {
        all.iter().map(|key| spell(*key).to_owned()).collect()
    }
    let mut found = names(doc::Root::ALL, doc::Root::as_str);
    found.extend(names(doc::MetaKey::ALL, doc::MetaKey::as_str));
    found.extend(names(doc::PageKey::ALL, doc::PageKey::as_str));
    found.extend(names(doc::DefaultsKey::ALL, doc::DefaultsKey::as_str));
    found.extend(names(doc::BandKey::ALL, doc::BandKey::as_str));
    found.extend(names(doc::NodeKey::ALL, doc::NodeKey::as_str));
    found.extend(names(doc::LineHeightKey::ALL, doc::LineHeightKey::as_str));
    let written: Vec<doc::AttrKey> = doc::AttrKey::ALL
        .iter()
        .copied()
        .filter(|key| !doc::UNWRITTEN_ATTRS.contains(key))
        .collect();
    found.extend(names(&written, doc::AttrKey::as_str));
    found.extend(names(doc::RegionKey::ALL, doc::RegionKey::as_str));
    found.extend(names(doc::AnnotKey::ALL, doc::AnnotKey::as_str));
    found.extend(names(doc::ReplyKey::ALL, doc::ReplyKey::as_str));
    found.extend(declared_string_consts(DOC_SOURCE));
    found
}

/// Every root the mark registry declares, through its `ALL`.
pub(super) fn declared_mark_roots() -> Vec<String> {
    crate::mark::MarkRoot::ALL
        .iter()
        .map(|root| root.as_str().to_owned())
        .collect()
}

/// A document that reaches every key the schema declares.
///
/// One block per shape, every paragraph property set, and one mark per
/// registered root. It is deliberately not a realistic document: its job is
/// width, and the coverage assertion fails when it stops having it.
/// The last child of the tree's root, a band or a note included.
/// An insert that names no anchor lands there.
fn last_child(state: &ProseState) -> Option<crate::message::BlockId> {
    let tree = doc::blocks(&state.doc);
    tree.children(None)?
        .last()
        .copied()
        .map(crate::events::block_id)
}

/// A picture block at the end of the document, with every key an image
/// carries: its address, size, text, caption and a non-default alignment.
fn write_a_picture(state: &mut ProseState) {
    use crate::message::{DisplaySize, ImageAlign};
    use ambre_core::{Doc, Px};
    state.handle_command(ProseCommand::InsertImage {
        parent_id: None,
        // No anchor: the block lands last among the root's children, after
        // the bands and the note.
        after_id: None,
        content_hash: crate::logic::tests::hash_of('a'),
        alt: ambre_core::Validated::new("A lighthouse."),
        size: DisplaySize::new(Px::<Doc>::new(320.0), Px::<Doc>::new(240.0)),
    });
    let Some(picture) = last_child(state) else {
        panic!("the scenario inserted a picture");
    };
    state.handle_command(ProseCommand::SetImageCaption {
        block_id: picture,
        caption: ambre_core::Validated::new("The lighthouse at dusk."),
    });
    state.handle_command(ProseCommand::SetImageAlign {
        block_id: picture,
        align: ImageAlign::Center,
    });
}

pub(super) fn document_of_every_key() -> ProseState {
    let mut state = ProseState::with_peer(ambre_core::PeerId::new(1)).expect("a document is built");
    let root = state.root_blocks()[0];
    state.handle_command(ProseCommand::InsertText {
        block_id: root,
        pos: Pos::<DocChar>::ZERO,
        text: "The shape of a paragraph.".to_owned(),
    });
    state.handle_command(ProseCommand::SetBlockStyle {
        block_id: root,
        style_id: crate::message::StyleId::new("Body".to_owned()),
    });
    state.handle_command(ProseCommand::SetParagraphStyle {
        block_id: root,
        style: ParagraphStyle {
            align: Some(Align::Center),
            indent_first: Some(PrintLength::<Pt>::new(1.0)),
            indent_left: Some(PrintLength::<Pt>::new(2.0)),
            indent_right: Some(PrintLength::<Pt>::new(3.0)),
            space_before: Some(PrintLength::<Pt>::new(4.0)),
            space_after: Some(PrintLength::<Pt>::new(5.0)),
            line_height: Some(LineHeight::Multiple(LineSpacingMultiple::ONE_AND_HALF)),
            keep_together: Some(crate::events::KeepTogether::Together),
            keep_with_next: Some(crate::events::KeepWithNext::WithNext),
        },
    });
    // Seed leaves the heading font absent, so only a pick writes the key.
    state.handle_command(ProseCommand::SetDefaultFont {
        font: crate::message::DefaultFont::Heading(Some(ambre_core::FontFamily::Serif)),
    });
    for init in [
        BlockInit::Heading {
            level: HeadingLevel::H2,
        },
        BlockInit::ListItem {
            style: ListStyle::Bullet,
        },
        BlockInit::ListItem {
            style: ListStyle::Ordered(OrderedStyle::LowerRoman),
        },
        BlockInit::ListItem {
            style: ListStyle::Todo(crate::events::TodoMark::Done),
        },
        BlockInit::CodeBlock {
            lang: ambre_core::Validated::<crate::message::CodeLang>::parse("rust").ok(),
        },
        BlockInit::HRule,
        BlockInit::PageBreak,
    ] {
        state.handle_command(ProseCommand::InsertBlock {
            parent_id: None,
            after_id: Some(root),
            init,
            style_id: None,
        });
    }
    write_a_table(&mut state, root);
    state.handle_command(ProseCommand::InsertToc {
        parent_id: None,
        after_id: Some(root),
        max_level: HeadingLevel::H3,
        numbered: crate::events::TocNumbering::Numbered,
    });
    write_the_bands(&mut state);
    write_a_note_and_a_reference(&mut state, root);
    hide_the_rule(&mut state);
    write_a_thread(&mut state, root);
    write_a_suggestion(&mut state, root);
    // No command sets the document locale yet, so the scenario writes the
    // register directly: the freeze still has to hold its type.
    if doc::meta(&state.doc)
        .insert(doc::MetaKey::Locale, "fr")
        .is_err()
    {
        panic!("the scenario could not write the locale");
    }
    state.doc.commit();
    for key in mark_roots_to_apply() {
        // The style path refuses an anchor its own command owns
        // (`PROSE-FEAT-298`). The scenario still has to reach every root, so
        // it lays those marks on the body directly.
        if crate::mark::is_owned_anchor(&key) {
            let body = doc::get_block_text(&state.doc, &crate::events::tree_id(root));
            let value = mark_value_for(key.as_str());
            let text = value.as_ref().map_or("", crate::mark::MarkValue::as_str);
            if body.mark(0..3, key.as_str(), text).is_err() {
                panic!("the scenario could not lay the {key} mark");
            }
            state.doc.commit();
            continue;
        }
        state.handle_command(ProseCommand::ApplyMark {
            block_id: root,
            range: Span::<DocChar>::from_usize_range(0..3),
            key: key.clone(),
            value: mark_value_for(key.as_str()),
        });
    }
    // Written last, so the blocks and the operations of every earlier line
    // keep the names and the counters version 1 froze them under.
    write_a_picture(&mut state);
    state
}

/// Both sides shown, the first page's own bands on, one band aligned, a
/// field in it, and the last-modified stamp.
fn write_the_bands(state: &mut ProseState) {
    use crate::events::{DateFieldFormat, HeaderField, HfTarget};
    state.handle_command(ProseCommand::SetHeaderEnabled {
        shown: crate::events::BandState::On,
    });
    state.handle_command(ProseCommand::SetFooterEnabled {
        shown: crate::events::BandState::On,
    });
    state.handle_command(ProseCommand::SetDifferentFirstPage {
        first_page: crate::events::FirstPage::OwnBands,
    });
    let Some(header) = state.bands().header else {
        panic!("the scenario showed the header");
    };
    state.handle_command(ProseCommand::SetBandAlign {
        block_id: header,
        align: Align::Center,
    });
    state.handle_command(ProseCommand::InsertHeaderField {
        field: HeaderField::Date(DateFieldFormat::Iso),
        target: HfTarget::FirstPage,
        pos: Pos::<DocChar>::ZERO,
    });
    state.handle_command(ProseCommand::StampModified {
        now: Instant::<Authored>::new(1),
    });
}

/// The horizontal rule deleted, so the hiding marker is written.
fn hide_the_rule(state: &mut ProseState) {
    let Some(rule) = state
        .root_blocks()
        .into_iter()
        .find(|id| state.block_type_of(id) == Some(crate::events::BlockType::HRule))
    else {
        panic!("the scenario inserted a rule");
    };
    state.handle_command(ProseCommand::DeleteBlock { block_id: rule });
}

/// A comment thread on `root`, with one reply after its opening one.
fn write_a_thread(state: &mut ProseState, root: crate::events::BlockId) {
    state.set_user_id(ambre_core::Id::from_bytes([7; 16]));
    state.handle_command(ProseCommand::AddComment {
        block_id: root,
        range: Span::<DocChar>::from_usize_range(0..1),
        text: "A comment.".to_owned(),
        now: Instant::<Authored>::new(1),
    });
    let Some(thread) = state.comments().into_iter().next() else {
        panic!("the scenario opened a thread");
    };
    state.handle_command(ProseCommand::ReplyToComment {
        comment_id: thread.comment_id,
        text: "A reply.".to_owned(),
        now: Instant::<Authored>::new(2),
    });
}

/// A suggested insertion and a suggested deletion on `root`, the mode off
/// again after them.
fn write_a_suggestion(state: &mut ProseState, root: crate::events::BlockId) {
    state.handle_command(ProseCommand::EnableTrackChanges {
        mode: crate::events::EditMode::Suggesting,
    });
    state.handle_command(ProseCommand::InsertText {
        block_id: root,
        pos: Pos::<DocChar>::ZERO,
        text: "S".to_owned(),
    });
    state.handle_command(ProseCommand::DeleteText {
        block_id: root,
        range: Span::<DocChar>::from_usize_range(3..4),
    });
    state.handle_command(ProseCommand::EnableTrackChanges {
        mode: crate::events::EditMode::Editing,
    });
}

/// A footnote and a cross reference to `root`, both called from `root`.
fn write_a_note_and_a_reference(state: &mut ProseState, root: crate::events::BlockId) {
    state.handle_command(ProseCommand::InsertFootnote {
        block_id: root,
        pos: Pos::<DocChar>::ZERO,
        note_text: "A note.".to_owned(),
    });
    state.handle_command(ProseCommand::InsertCrossRef {
        block_id: root,
        pos: Pos::<DocChar>::ZERO,
        target_block_id: root,
        kind: crate::events::CrossRefKind::PageNumber,
    });
}

/// A two by two table after `root`, its first row merged and its last cell
/// carrying every cell property.
fn write_a_table(state: &mut ProseState, root: crate::events::BlockId) {
    state.handle_command(ProseCommand::InsertTable {
        parent_id: None,
        after_id: Some(root),
        rows: Len::<crate::events::grid::Row>::new(2),
        cols: Len::<crate::events::grid::Col>::new(2),
    });
    let Some(table) = state
        .root_blocks()
        .into_iter()
        .find(|id| state.block_type_of(id) == Some(crate::events::BlockType::Table))
    else {
        panic!("the scenario inserted a table");
    };
    state.handle_command(ProseCommand::MergeTableCells {
        table_id: table,
        start: crate::logic::tables::tests::pos((0, 0)),
        end: crate::logic::tables::tests::pos((0, 1)),
    });
    let last = crate::logic::tables::tests::cell_at(state, tree_id(table), 1, 1);
    let cell = crate::events::block_id(last);
    state.handle_command(ProseCommand::SetCellStyle {
        cell_id: cell,
        bg: Some(ambre_core::Rgb::from_bits(0x00ff_8800)),
        valign: Some(VAlign::Middle),
        border: Some(CellBorder {
            width: PrintLength::<Pt>::new(0.5),
            color: ambre_core::Rgb::from_bits(0x0000_0000),
        }),
        padding: Some(PrintLength::<Pt>::new(4.0)),
    });
}

/// The mark key written for each registered root.
///
/// The header-field root is registered alone and addressed through its four
/// children, so it is written as one of them (`mark.rs`).
fn mark_roots_to_apply() -> Vec<MarkKey> {
    let mut keys: Vec<MarkKey> = declared_mark_roots()
        .into_iter()
        .map(|root| MarkKey::new(if root == "f" { "f:pn".to_owned() } else { root }))
        .collect();
    keys.sort();
    keys.dedup();
    keys
}

/// A value the key's own grammar accepts (`PROSE-FEAT-022`).
///
/// A flag key takes none, a colour takes a hexadecimal record and a size takes
/// a number. Every other key takes any text.
fn mark_value_for(key: &str) -> Option<crate::mark::MarkValue> {
    match key {
        "b" | "i" | "u" | "s" | "sup" | "sub" | "c" => None,
        "fg" | "bg" => Some(crate::mark::test_value("#ff8800")),
        "fs" => Some(crate::mark::test_value("11")),
        _ => Some(crate::mark::test_value("x")),
    }
}

#[test]
fn the_shape_a_build_writes_is_the_shape_frozen_for_its_version() {
    let state = document_of_every_key();
    let written = render(&shape_of(&state.doc));
    let Some(frozen) = frozen_section(SCHEMA_VERSION) else {
        panic!(
            "`schema/shapes.txt` carries no `== v{SCHEMA_VERSION}` section. \
             The version rose without a freeze; add the section below:\n{written}"
        );
    };
    assert_eq!(
        written, frozen,
        "the shape this build writes is not the shape frozen for \
         SCHEMA_VERSION {SCHEMA_VERSION}. A key or a value type moved, so raise \
         SCHEMA_VERSION and freeze the new shape as its own section \
         (`SCHEMA-018`)."
    );
}

/// Every build of one version writes the same scaffolding (`AUTHOR-024`).
/// The identity hashes the scaffolding's bytes, so it moves with any of them.
#[test]
fn the_scaffold_a_build_writes_is_the_scaffold_frozen_for_its_version() {
    let Some(identity) = super::scaffold_identity() else {
        panic!("the CRDT engine refused the scaffolding");
    };
    let written = format!("{identity:016x}\n");
    assert_eq!(
        section_of(SCAFFOLD, SCHEMA_VERSION).as_deref(),
        Some(written.as_str()),
        "the scaffolding this build writes is not the one frozen for \
         SCHEMA_VERSION {SCHEMA_VERSION} in `schema/scaffold.txt`. Another \
         build of that version writes it under another identity, and a delta \
         that waits on it never applies. Raise SCHEMA_VERSION and append a section."
    );
}

/// The canonical document of the version a step leaves: that version's
/// layout, holding what the step rewrites, written under a fixed identity.
/// Each step reads its own, so a step added later never moves what an earlier
/// one writes.
fn canonical(from: ambre_core::Version<ambre_core::schema::Schema>) -> LoroDoc {
    match from.get() {
        // Version 1: a document with no heading font. The step to 2 rewrites
        // nothing but the register.
        1 => {
            let doc = LoroDoc::new();
            doc.set_peer_id(1).expect("a fixed peer");
            let _ = doc
                .get_map(ambre_core::crdt::FoundationRoot::Meta.as_str())
                .insert(ambre_core::schema::MetaKey::SchemaVersion.as_str(), 1_i64);
            doc.commit();
            doc
        }
        _ => panic!("the step from version {from} has no canonical document: write one here"),
    }
}

/// Each step of the registry, run as the gate runs it on its canonical
/// document. One line per step: `from -> to` and what it wrote.
fn steps_written() -> Vec<String> {
    MIGRATIONS
        .iter()
        .map(|step| {
            let stored = canonical(step.from)
                .export(ExportMode::Snapshot)
                .expect("a snapshot");
            let scratch = LoroDoc::new();
            scratch.import(&stored).expect("the snapshot imports");
            let heads = scratch.oplog_frontiers();
            scratch
                .set_peer_id(
                    migration_peer(heads.iter().map(|id| {
                        (
                            ambre_core::PeerId::new(id.peer),
                            ambre_core::OpCounter::new(id.counter),
                        )
                    }))
                    .get(),
                )
                .expect("the migration identity");
            let before = scratch.oplog_vv();
            (step.apply)(&scratch).expect("the step runs on its canonical document");
            let written = scratch
                .export(ExportMode::updates(&before))
                .expect("the step's writes export");
            format!(
                "{} -> {} {}",
                step.from,
                step.to,
                step_fingerprint(&written)
            )
        })
        .collect()
}

/// Every build of one version runs each migration step alike (`AUTHOR-024`).
/// A step stamps the version it reaches, so a raised version moves every line
/// and freezes a section of its own.
#[test]
fn each_migration_step_writes_what_its_version_froze() {
    let listed: String = steps_written()
        .iter()
        .map(|line| format!("{line}\n"))
        .collect();
    assert_eq!(
        section_of(STEPS, SCHEMA_VERSION).as_deref(),
        Some(listed.as_str()),
        "a migration step this build runs writes other operations than the \
         ones frozen for SCHEMA_VERSION {SCHEMA_VERSION} in \
         `schema/migrations.txt`. A peer on another build of that version \
         migrates the same document another way. Restore the step, or raise \
         SCHEMA_VERSION and append a section."
    );
}

/// Version 2 adds one key to version 1 and moves no other line, so a
/// version 1 reader, which ignores a key it does not know, loses the
/// heading font alone. The key is optional: seed leaves it absent, and an
/// absent key reads as the body font (`PROSE-UI-266`, `PROSE-149`).
#[test]
fn version_2_adds_the_optional_heading_font_and_moves_no_other_line() {
    let lines = |v: u32| -> BTreeSet<String> {
        frozen_section(ambre_core::Version::new(v))
            .expect("a frozen section")
            .lines()
            .map(str::to_owned)
            .collect()
    };
    let (one, two) = (lines(1), lines(2));
    let added: Vec<_> = two.difference(&one).cloned().collect();
    assert_eq!(added, ["meta/defaults/heading_font_family: string"]);
    assert_eq!(one.difference(&two).count(), 0, "version 2 drops no line");

    let fresh = ProseState::with_peer(ambre_core::PeerId::new(1)).expect("a document is built");
    let path = "meta/defaults/heading_font_family";
    assert!(
        !render(&shape_of(&fresh.doc)).contains(path),
        "seed writes no heading font"
    );
    let defaults = fresh.defaults();
    assert_eq!(defaults.heading_font_family, None);
    let heading = crate::style::resolve(
        &std::collections::BTreeMap::new(),
        crate::style::BlockContext {
            block_type: crate::events::BlockType::Heading,
            heading_level: Some(HeadingLevel::H1),
        },
        &defaults,
    );
    assert_eq!(heading.font_family, Some(defaults.font_family));
}

#[test]
fn every_declared_key_appears_in_the_shape() {
    let state = document_of_every_key();
    let shape = shape_of(&state.doc);
    let mut declared = declared_keys();
    declared.extend(declared_mark_roots());
    for key in declared {
        let reached = shape.keys.contains(&key)
            || shape
                .keys
                .iter()
                .any(|seen| seen.split(':').next() == Some(key.as_str()));
        assert!(
            reached,
            "`{key}` is declared as a schema key and no line of the shape \
             carries it. Either the scenario stopped reaching it, or the key is \
             new and the scenario has to write it before the freeze can hold it."
        );
    }
}

#[test]
fn a_value_changing_leaves_the_shape_alone() {
    let first = document_of_every_key();
    let mut second = document_of_every_key();
    let block = second.root_blocks()[0];
    second.handle_command(ProseCommand::InsertText {
        block_id: block,
        pos: Pos::<DocChar>::ZERO,
        text: "A different sentence entirely.".to_owned(),
    });
    assert_eq!(
        render(&shape_of(&first.doc)),
        render(&shape_of(&second.doc)),
        "the shape reads the types and not the values"
    );
}

#[test]
fn a_key_appearing_moves_the_shape() {
    let state = document_of_every_key();
    let before = render(&shape_of(&state.doc));
    // The key is outside the schema on purpose, so it reaches the map raw.
    doc::root_map(&state.doc, doc::Root::Meta)
        .insert("a_key_no_build_writes", 1_i64)
        .expect("the register takes a key");
    assert_ne!(
        before,
        render(&shape_of(&state.doc)),
        "a key appearing has to move the shape, or the freeze holds nothing"
    );
}

#[test]
fn a_value_type_changing_moves_the_shape() {
    let state = document_of_every_key();
    let before = render(&shape_of(&state.doc));
    // `title` is seeded as a string; the same key under a number is the drift
    // a reviewer cannot be asked to spot.
    doc::meta(&state.doc)
        .insert(doc::MetaKey::Title, 1_i64)
        .expect("the register takes a key");
    assert_ne!(
        before,
        render(&shape_of(&state.doc)),
        "a value type changing has to move the shape"
    );
}
