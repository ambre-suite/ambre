//! The schema shape belt: the key set and the value types a build writes,
//! frozen per `SCHEMA_VERSION` (`PROSE-029`).
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
//! gap: it reads `doc.rs` and the mark registry at compile time, and demands a
//! line for each key they declare. A new key with no scenario behind it fails
//! there rather than passing silently.
//!
//! **What it cannot hold.** A value whose meaning moves under an unchanged key
//! and an unchanged type leaves the shape identical (`SCHEMA-017`). That half
//! stays with the reviewer.

use std::collections::BTreeSet;

use loro::{LoroDoc, LoroValue, TreeID};

use crate::doc;
use crate::events::{
    Align, BlockInit, CellBorder, LineHeight, LineHeightMode, ListStyle, OrderedStyle,
    ParagraphStyle, ProseCommand, VAlign, tree_id,
};
use crate::logic::ProseState;
use crate::schema::SCHEMA_VERSION;

/// The frozen shapes, one section per `SCHEMA_VERSION`.
const FROZEN: &str = include_str!("../../schema/shapes.txt");

/// The source the key declarations are read from, so the coverage assertion
/// cannot drift from the module it guards.
const DOC_SOURCE: &str = include_str!("../doc.rs");
const MARK_SOURCE: &str = include_str!("../mark.rs");

/// The segment a tree node, a body container or a user-named register collapses
/// to. Their names carry a `TreeID` or a peer id, so a shape that kept them
/// would differ between two runs of the same build.
const ANY: &str = "*";

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
        if name != doc::BLOCKS {
            walk_map(name, value, &mut shape);
        }
    }

    walk_tree(doc, &mut shape);
    walk_marks(doc, &mut shape);
    shape
}

/// The body container, under the one name every body collapses to.
const ANY_BODY: &str = "t:*";

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
            &format!("{}/{ANY}", doc::BLOCKS),
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
fn frozen_section(version: u32) -> Option<String> {
    let header = format!("== v{version}");
    let mut taken = String::new();
    let mut inside = false;
    for line in FROZEN.lines() {
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
fn declared_string_consts(source: &str) -> Vec<String> {
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

/// Every `&str` element of the mark registry's arrays.
fn declared_mark_roots(source: &str) -> Vec<String> {
    let mut found = declared_string_consts(source);
    for name in ["FORMATTING_KEYS", "ANCHOR_KEYS"] {
        let Some(start) = source.find(&format!("{name}: [&str;")) else {
            continue;
        };
        let Some(open) = source[start..].find('[').map(|i| start + i) else {
            continue;
        };
        let Some(open) = source[open + 1..].find('[').map(|i| open + 1 + i) else {
            continue;
        };
        let Some(close) = source[open..].find(']').map(|i| open + i) else {
            continue;
        };
        for piece in source[open + 1..close].split(',') {
            let key = piece.trim().trim_matches('"');
            if !key.is_empty() {
                found.push(key.to_owned());
            }
        }
    }
    found
}

/// A document that reaches every key the schema declares.
///
/// One block per shape, every paragraph property set, and one mark per
/// registered root. It is deliberately not a realistic document: its job is
/// width, and the coverage assertion fails when it stops having it.
fn document_of_every_key() -> ProseState {
    let mut state = ProseState::with_peer(1).expect("a document is built");
    let root = state.root_blocks()[0];
    state.handle_command(ProseCommand::InsertText {
        block_id: root,
        pos: 0,
        text: "The shape of a paragraph.".to_owned(),
    });
    state.handle_command(ProseCommand::SetBlockStyle {
        block_id: root,
        style_id: "Body".to_owned(),
    });
    state.handle_command(ProseCommand::SetParagraphStyle {
        block_id: root,
        style: ParagraphStyle {
            align: Some(Align::Center),
            indent_first: Some(1.0),
            indent_left: Some(2.0),
            indent_right: Some(3.0),
            space_before: Some(4.0),
            space_after: Some(5.0),
            line_height: Some(LineHeight {
                mode: LineHeightMode::Multiple,
                value: 1.5,
            }),
            keep_together: Some(true),
            keep_with_next: Some(true),
        },
    });
    for init in [
        BlockInit::Heading { level: 2 },
        BlockInit::ListItem {
            style: ListStyle::Bullet,
        },
        BlockInit::ListItem {
            style: ListStyle::Ordered(OrderedStyle::LowerRoman),
        },
        BlockInit::ListItem {
            style: ListStyle::Todo(true),
        },
        BlockInit::CodeBlock {
            lang: "rust".to_owned(),
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
        max_level: 3,
        numbered: true,
    });
    write_the_bands(&mut state);
    write_a_note_and_a_reference(&mut state, root);
    hide_the_rule(&mut state);
    write_a_thread(&mut state, root);
    write_a_suggestion(&mut state, root);
    for key in mark_roots_to_apply() {
        // The style path refuses an anchor its own command owns
        // (`PROSE-FEAT-298`). The scenario still has to reach every root, so
        // it lays those marks on the body directly.
        if crate::mark::is_owned_anchor(&key) {
            let body = doc::get_block_text(&state.doc, &crate::events::tree_id(root));
            let value = mark_value_for(&key).unwrap_or_default();
            if body.mark(0..3, &key, value.as_str()).is_err() {
                panic!("the scenario could not lay the {key} mark");
            }
            state.doc.commit();
            continue;
        }
        state.handle_command(ProseCommand::ApplyMark {
            block_id: root,
            range: (0, 3),
            key: key.clone(),
            value: mark_value_for(&key),
        });
    }
    state
}

/// Both sides shown, the first page's own bands on, one band aligned, a
/// field in it, and the last-modified stamp.
fn write_the_bands(state: &mut ProseState) {
    use crate::events::{DateFieldFormat, HeaderField, HfTarget};
    state.handle_command(ProseCommand::SetHeaderEnabled { enabled: true });
    state.handle_command(ProseCommand::SetFooterEnabled { enabled: true });
    state.handle_command(ProseCommand::SetDifferentFirstPage { enabled: true });
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
        pos: 0,
    });
    state.handle_command(ProseCommand::StampModified { now_ms: 1 });
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
    state.set_user_id(ambre_core::UserId::from_bytes([7; 16]));
    state.handle_command(ProseCommand::AddComment {
        block_id: root,
        range: (0, 1),
        text: "A comment.".to_owned(),
        now_ms: 1,
    });
    let Some(thread) = state.comments().into_iter().next() else {
        panic!("the scenario opened a thread");
    };
    state.handle_command(ProseCommand::ReplyToComment {
        comment_id: thread.comment_id,
        text: "A reply.".to_owned(),
        now_ms: 2,
    });
}

/// A suggested insertion and a suggested deletion on `root`, the mode off
/// again after them.
fn write_a_suggestion(state: &mut ProseState, root: crate::events::BlockId) {
    state.handle_command(ProseCommand::EnableTrackChanges { enabled: true });
    state.handle_command(ProseCommand::InsertText {
        block_id: root,
        pos: 0,
        text: "S".to_owned(),
    });
    state.handle_command(ProseCommand::DeleteText {
        block_id: root,
        range: (3, 4),
    });
    state.handle_command(ProseCommand::EnableTrackChanges { enabled: false });
}

/// A footnote and a cross reference to `root`, both called from `root`.
fn write_a_note_and_a_reference(state: &mut ProseState, root: crate::events::BlockId) {
    state.handle_command(ProseCommand::InsertFootnote {
        block_id: root,
        pos: 0,
        note_text: "A note.".to_owned(),
    });
    state.handle_command(ProseCommand::InsertCrossRef {
        block_id: root,
        pos: 0,
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
        rows: 2,
        cols: 2,
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
        start: (0, 0),
        end: (0, 1),
    });
    let last = crate::logic::tables::tests::cell_at(state, tree_id(table), 1, 1);
    let cell = crate::events::block_id(last);
    state.handle_command(ProseCommand::SetCellStyle {
        cell_id: cell,
        bg: Some(0x00ff_8800),
        valign: Some(VAlign::Middle),
        border: Some(CellBorder {
            width_pt: 0.5,
            color: 0x0000_0000,
        }),
        padding_pt: Some(4.0),
    });
}

/// The mark key written for each registered root.
///
/// The header-field root is registered alone and addressed through its four
/// children, so it is written as one of them (`mark.rs`).
fn mark_roots_to_apply() -> Vec<String> {
    let mut keys: Vec<String> = declared_mark_roots(MARK_SOURCE)
        .into_iter()
        .map(|root| if root == "f" { "f:pn".to_owned() } else { root })
        .collect();
    keys.sort();
    keys.dedup();
    keys
}

/// A value the key's own grammar accepts (`PROSE-FEAT-022`).
///
/// A flag key takes none, a colour takes a hexadecimal record and a size takes
/// a number. Every other key takes any text.
fn mark_value_for(key: &str) -> Option<String> {
    match key {
        "b" | "i" | "u" | "s" | "sup" | "sub" | "c" => None,
        "fg" | "bg" => Some("#ff8800".to_owned()),
        "fs" => Some("11".to_owned()),
        _ => Some("x".to_owned()),
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
         (`PROSE-029`)."
    );
}

#[test]
fn every_declared_key_appears_in_the_shape() {
    let state = document_of_every_key();
    let shape = shape_of(&state.doc);
    let mut declared = declared_string_consts(DOC_SOURCE);
    declared.extend(declared_mark_roots(MARK_SOURCE));
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
        pos: 0,
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
    state
        .doc
        .get_map(doc::META)
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
    state
        .doc
        .get_map(doc::META)
        .insert("title", 1_i64)
        .expect("the register takes a key");
    assert_ne!(
        before,
        render(&shape_of(&state.doc)),
        "a value type changing has to move the shape"
    );
}
