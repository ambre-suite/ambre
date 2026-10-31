//! The schema shape belt: the key set and the value types a build writes,
//! frozen per `SCHEMA_VERSION` (`SCHEMA-018`).
//!
//! A shape is the board with every value removed: one line per reachable path,
//! each naming the type the value under it holds. Adding a key, dropping one,
//! or changing an `i64` into an `f64` moves a line; moving an element from
//! `x: 0.0` to `x: 40.0` does not. The frozen text is therefore a diff a
//! reviewer reads, and a build that writes a new shape under an unchanged
//! `SCHEMA_VERSION` turns red.
//!
//! **One segment collapses.** Every element, connector, thread, reaction and
//! media entry is keyed by a minted id, a user id or a content hash. A shape
//! that kept them would differ between two runs of the same build, so each
//! becomes [`ANY`].
//!
//! **The binary registers.** Six of them hold a `postcard` blob rather than a
//! scalar. `postcard` decodes a blob whole: a field added, dropped, reordered
//! or retyped changes what the bytes mean. `binary` alone would say
//! nothing about that, so the shape also carries the encoding of one canonical
//! value per blob, as hex. The layout is then frozen with the rest, and a
//! layout change moves a line.
//!
//! **What holds the freeze honest.** A shape is only as wide as the scenario
//! that builds it. A key no command reaches leaves no line, so dropping it
//! would go unnoticed. [`every_declared_key_appears_in_the_shape`] closes that
//! gap. It reads the `state` sources at compile time. Every key they declare,
//! as a `const` or as a literal handed to `insert`, must carry a line.
//!
//! **What it cannot hold.** A value whose meaning moves under an unchanged key
//! and an unchanged type leaves the shape identical (`SCHEMA-017`). That half
//! is the reading belt's, in `schema_reading.rs`, over this same scenario. A
//! `LoroText` reads as `text` and a register as `string`, so a body demoted
//! from a text container moves a line.

use ambre_core::Px;
use ambre_core::Rgba;
use ambre_core::{CharCount, CharOffset};
use std::collections::BTreeSet;

use kurbo::{BezPath, Point, Size};
use loro::{Container, ExportMode, LoroDoc, LoroValue, ValueOrContainer};

use ambre_bus::DocType;
use ambre_core::id::{Document, Element, Id, User};
use ambre_core::schema::{migration_peer, step_fingerprint};

use crate::logic::autolayout::{Align, AutoLayout, Direction, Sizing};
use crate::logic::command::{
    CommentAnchor, InitContent, Reaction, ReactionKind, TextDelta, ToileCommand, ToileEvent,
    ToileOutcome,
};
use crate::logic::interop::{ContentHash, MediaBlob};
use crate::logic::model::{
    Anchor, ElementKind, ElementStyle, EndCaps, FontSpec, LinkTarget, NodeId, PathType, Shadow,
    ShapeKind, StrokePoint,
};
use crate::logic::schema::{MIGRATIONS, SCHEMA_VERSION};

use super::{
    BoardKey, ConfigKey, ConnectorKey, ElementKey, MediaKey, MessageKey, Root, ThreadKey,
    ToileState,
};
use ambre_bus::{Authored, Instant};

/// The frozen shapes, one section per `SCHEMA_VERSION`.
const FROZEN: &str = include_str!("../../../schema/shapes.txt");

/// The address the sample board's picture and its media entry share. Every
/// reader checks a stored hash, so the sample spells a real one.
fn sample_hash() -> ContentHash {
    ContentHash::parse(&"0".repeat(64)).expect("sixty-four hex digits")
}

/// The frozen scaffolding identities, one section per `SCHEMA_VERSION`.
const SCAFFOLD: &str = include_str!("../../../schema/scaffold.txt");

/// What each migration step writes, one section per `SCHEMA_VERSION`.
const STEPS: &str = include_str!("../../../schema/migrations.txt");

/// The sources the key declarations are read from, so the coverage assertion
/// cannot drift from the modules it guards. Every module that writes a key into
/// the document is here; one that stops writing keys may leave.
pub(super) const KEY_SOURCES: [&str; 10] = [
    include_str!("mod.rs"),
    include_str!("boolean.rs"),
    include_str!("clipboard.rs"),
    include_str!("codec.rs"),
    include_str!("comments.rs"),
    include_str!("connectors.rs"),
    include_str!("elements.rs"),
    include_str!("groups.rs"),
    include_str!("import.rs"),
    include_str!("presentation.rs"),
];

/// The segment a minted id, a user id or a content hash collapses to.
pub(super) const ANY: &str = "*";

/// The declared `&str` constants that name something other than a schema key,
/// and so carry no line of their own. `CommitOrigin::Comment` is a Loro commit origin,
/// read back from a subscription rather than written into a register.
const NOT_SCHEMA_KEYS: [&str; 1] = ["cmt"];

/// The roots whose keys are schema words rather than data. Every other root is
/// a dictionary keyed by an id, so its keys collapse to [`ANY`].
pub(super) fn keys_are_schema_words(root: &str) -> bool {
    matches!(
        Root::parse(root),
        Some(Root::Board | Root::PresentationConfig | Root::Meta)
    )
}

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

/// A board's shape: the lines the freeze holds, plus the key segments the
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

/// The type a container holds, in the spelling of [`type_name`]. A text is
/// `text` and never `string`: a body demoted to a register then moves a line
/// (`TOILE-030`).
fn container_name(container: &Container) -> &'static str {
    match container {
        Container::Map(_) => "map",
        Container::List(_) | Container::MovableList(_) => "list",
        Container::Text(_) => "text",
        _ => "container",
    }
}

impl Shape {
    /// Record one child at `path`, and everything below it.
    fn visit(&mut self, path: &str, child: ValueOrContainer) {
        match child {
            ValueOrContainer::Value(value) => {
                self.record(path, &value);
                walk_value(path, &value, self);
            }
            ValueOrContainer::Container(container) => {
                self.lines
                    .insert(format!("{path}: {}", container_name(&container)));
                walk(path, &container, true, self);
            }
        }
    }
}

/// Walk a container, recording every path below it.
///
/// `schema_words` says whether this level's keys are the schema's own. Below a
/// dictionary level the keys are schema words again: an id names an entry, and
/// the entry's fields are the schema's. A list holds entries and never keys, so
/// its elements all fold onto one path.
fn walk(prefix: &str, container: &Container, schema_words: bool, shape: &mut Shape) {
    let item = format!("{prefix}/{ANY}");
    match container {
        Container::Map(map) => map.for_each(|key, child| {
            shape.keys.insert(key.to_owned());
            let segment = if schema_words { key } else { ANY };
            shape.visit(&format!("{prefix}/{segment}"), child);
        }),
        Container::List(list) => list.for_each(|child| shape.visit(&item, child)),
        Container::MovableList(list) => list.for_each(|child| shape.visit(&item, child)),
        _ => {}
    }
}

/// Walk a plain value the way [`walk`] walks a container. No register holds a
/// map or a list today, so this records nothing until one does.
fn walk_value(prefix: &str, value: &LoroValue, shape: &mut Shape) {
    match value {
        LoroValue::Map(entries) => {
            for (key, child) in entries.iter() {
                shape.keys.insert(key.clone());
                let path = format!("{prefix}/{key}");
                shape.record(&path, child);
                walk_value(&path, child, shape);
            }
        }
        LoroValue::List(items) => {
            let path = format!("{prefix}/{ANY}");
            for child in items.iter() {
                shape.record(&path, child);
                walk_value(&path, child, shape);
            }
        }
        _ => {}
    }
}

/// The shape of a board: every root register, everything under it, and the
/// `postcard` layouts its binary registers carry.
fn shape_of(doc: &LoroDoc) -> Shape {
    let mut shape = Shape::default();
    let LoroValue::Map(roots) = doc.get_value() else {
        return shape;
    };
    for (name, value) in roots.iter() {
        let LoroValue::Container(id) = value else {
            continue;
        };
        let Some(container) = doc.get_container(id.clone()) else {
            continue;
        };
        shape.keys.insert(name.clone());
        shape
            .lines
            .insert(format!("{name}: {}", container_name(&container)));
        walk(name, &container, keys_are_schema_words(name), &mut shape);
    }
    for (path, hex) in postcard_layouts() {
        shape.lines.insert(format!("postcard/{path}: {hex}"));
    }
    shape
}

/// The `postcard` layout of each binary register, as the encoding of one
/// canonical value. An enum carries one line per variant, since a variant
/// reordered moves only its own tag.
///
/// Every field is set, and to a value of its own. A reordering of two
/// same-typed fields then moves the bytes like any other layout change. Nothing
/// here is spelled with `..Default::default()`: a field added to one of these
/// records must fail to compile rather than encode silently.
fn postcard_layouts() -> Vec<(String, String)> {
    let style = ElementStyle {
        fill: Some(Rgba::from_bits(0x0203_0401)),
        stroke: Some(Rgba::from_bits(0x0607_0805)),
        stroke_w: ambre_core::StoredBoardLength::new(1.5),
        opacity: ambre_core::Opacity::new(0.25).unwrap(),
        corner: ambre_core::StoredBoardLength::new(3.5),
        font: FontSpec {
            family: Some(ambre_bus::app::toile::FontFamily::Other("f".to_owned())),
            size: ambre_bus::app::toile::FontSizeChoice::try_from(7.5).unwrap(),
            weight: ambre_core::FontWeight::BOLD,
            italic: ambre_core::Slant::Italic,
            underline: ambre_core::Underline::Underlined,
            strikethrough: ambre_core::Strikethrough::Struck,
        },
        shadow: Some(Shadow {
            color: Rgba::from_bits(0x0a0b_0c09),
            dx: ambre_core::StoredBoardLength::new(11.5),
            dy: ambre_core::StoredBoardLength::new(12.5),
            blur: ambre_core::StoredBoardLength::new(13.5),
        }),
        text_color: Some(Rgba::from_bits(0x0e0f_100d)),
    };
    let layout = AutoLayout {
        direction: Direction::Vertical,
        gap: ambre_core::BoardLength::new(1.5),
        padding: ambre_bus::app::toile::Padding {
            top: ambre_core::BoardLength::new(2.5),
            right: ambre_core::BoardLength::new(3.5),
            bottom: ambre_core::BoardLength::new(4.5),
            left: ambre_core::BoardLength::new(5.5),
        },
        align: Align::End,
        sizing: Sizing::Fill,
    };
    let points = vec![StrokePoint {
        x: ambre_core::StoredBoardLength::new(1.5),
        y: ambre_core::StoredBoardLength::new(2.5),
        pressure: ambre_bus::app::toile::UnitFraction::HALF,
    }];
    vec![
        // `to_bytes` prefixes the style with its own version byte, so the frozen
        // hex holds the framing as well as the layout.
        (format!("elements/{ANY}/style"), hex(&style.to_bytes())),
        (
            format!("elements/{ANY}/link#url"),
            hex(&LinkTarget::Url("u".into()).to_bytes()),
        ),
        (
            format!("elements/{ANY}/link#frame"),
            hex(&LinkTarget::Frame(Id::<Element>::from_text("e".to_owned())).to_bytes()),
        ),
        (
            format!("elements/{ANY}/link#document"),
            hex(&LinkTarget::Document {
                document_id: Id::<Document>::from_bytes([1; 16]),
                doc_type: DocType::Toile,
            }
            .to_bytes()),
        ),
        (format!("elements/{ANY}/autolayout"), hex(&encode(&layout))),
        (format!("elements/{ANY}/points"), hex(&encode(&points))),
        (format!("connectors/{ANY}/style"), hex(&style.to_bytes())),
        (
            format!("comments/{ANY}/anchor#element"),
            hex(&encode(&CommentAnchor::Element(Id::<Element>::from_text(
                "e".to_owned(),
            )))),
        ),
        (
            format!("comments/{ANY}/anchor#point"),
            hex(&encode(&CommentAnchor::Point(kurbo::Point::new(1.5, 2.5)))),
        ),
        (
            format!("reactions/{ANY}#emoji"),
            hex(&encode(&Reaction {
                kind: ReactionKind::Emoji(ambre_bus::Emoji::from("e")),
                ts: Instant::<Authored>::new(1),
            })),
        ),
        (
            format!("reactions/{ANY}#vote"),
            hex(&encode(&Reaction {
                kind: ReactionKind::Vote(ambre_bus::app::toile::VoteWeight::new(2)),
                ts: Instant::<Authored>::new(3),
            })),
        ),
    ]
}

/// One record's `postcard` encoding.
fn encode<T: serde::Serialize>(record: &T) -> Vec<u8> {
    postcard::to_allocvec(record).unwrap_or_default()
}

/// A blob as lower-case hex.
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
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
fn frozen_section(version: ambre_core::schema::SchemaVersion) -> Option<String> {
    section_of(FROZEN, version)
}

/// The section of a frozen text that names this version.
pub(super) fn section_of(
    frozen: &str,
    version: ambre_core::schema::SchemaVersion,
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

/// The part of a source that ships in a client build. A key a test writes is
/// not a schema key, and the freeze must not be asked to hold it.
fn shipped(source: &str) -> &str {
    source.split("#[cfg(test)]").next().unwrap_or(source)
}

/// Every key a source declares: the `const NAME: &str = "literal";` vocabulary,
/// plus every literal handed to `insert` or `insert_container`.
///
/// Toile writes most of its element fields as literals rather than through
/// constants. Reading only the constants would leave the wider half of the
/// schema uncovered.
pub(super) fn declared_keys(source: &str) -> Vec<String> {
    let mut found = Vec::new();
    for line in shipped(source).lines() {
        let bare = line.trim_start();
        // The declaration's visibility is not part of the key it names.
        if let Some(rest) = bare
            .split_once("const ")
            .filter(|(before, _)| before.is_empty() || before.starts_with("pub"))
            .map(|(_, rest)| rest)
            && let Some((_, tail)) = rest.split_once(": &str = ")
            && let Some(literal) = tail.strip_prefix('"').and_then(|t| t.split('"').next())
        {
            found.push(literal.to_owned());
        }
        for call in ["insert(\"", "insert_container(\""] {
            let mut rest = line;
            while let Some((_, tail)) = rest.split_once(call) {
                if let Some(literal) = tail.split('"').next() {
                    found.push(literal.to_owned());
                }
                rest = tail;
            }
        }
    }
    found.retain(|k| !k.is_empty() && !NOT_SCHEMA_KEYS.contains(&k.as_str()));
    found.sort();
    found.dedup();
    found
}

/// Every name the schema enums declare: the roots and the keys of each map.
pub(super) fn schema_names() -> Vec<String> {
    let mut found: Vec<&str> = Root::ALL.iter().map(|n| n.as_str()).collect();
    found.extend(BoardKey::ALL.iter().map(|n| n.as_str()));
    found.extend(ElementKey::ALL.iter().map(|n| n.as_str()));
    found.extend(ConnectorKey::ALL.iter().map(|n| n.as_str()));
    found.extend(MediaKey::ALL.iter().map(|n| n.as_str()));
    found.extend(ConfigKey::ALL.iter().map(|n| n.as_str()));
    found.extend(ThreadKey::ALL.iter().map(|n| n.as_str()));
    found.extend(MessageKey::ALL.iter().map(|n| n.as_str()));
    let mut found: Vec<String> = found.into_iter().map(str::to_owned).collect();
    found.sort();
    found.dedup();
    found
}

/// The id of the element a command reports as created.
fn created(outcome: &ToileOutcome) -> Id<Element> {
    outcome
        .display
        .iter()
        .find_map(|e| match e {
            ToileEvent::ElementCreated { id, .. } => Some(id.clone()),
            _ => None,
        })
        .expect("the command creates an element")
}

/// A board that reaches every key the schema declares.
///
/// One element of every kind that carries a payload. A frame with children,
/// notes and a layout spec. A connector with a label, a reaction, a comment
/// thread with a reply, and a media entry. It is deliberately not a realistic
/// board: its job is width, and the coverage assertion fails when it stops
/// having it.
pub(super) fn board_of_every_key() -> ToileState {
    let mut st = ToileState::with_peer(ambre_core::PeerId::new(1));
    st.set_user_id(Id::<User>::from_bytes([7; 16]));

    let create = |st: &mut ToileState, kind, at: Point, content, correlate| {
        created(&st.handle_command(ToileCommand::CreateElement {
            kind,
            at,
            size: Size::new(10.0, 10.0),
            content,
            fill: None,
            correlate,
        }))
    };
    let rect = create(
        &mut st,
        ElementKind::Shape(ShapeKind::Rect),
        Point::new(0.0, 0.0),
        InitContent::None,
        None,
    );
    let sticky = create(
        &mut st,
        ElementKind::Sticky,
        Point::new(40.0, 0.0),
        InitContent::Text("note".to_owned()),
        None,
    );
    let frame = create(
        &mut st,
        ElementKind::Frame,
        Point::new(0.0, 80.0),
        InitContent::None,
        None,
    );
    let slide = st.frame(&frame).expect("a frame");
    let _image = create(
        &mut st,
        ElementKind::Image,
        Point::new(0.0, 200.0),
        InitContent::Image(crate::logic::command::MediaRef::Stored(sample_hash())),
        None,
    );
    let _embed = create(
        &mut st,
        ElementKind::Embed,
        Point::new(40.0, 200.0),
        InitContent::Embed(
            ambre_bus::app::toile::WebAddress::parse("https://example.invalid")
                .expect("a safe address"),
        ),
        None,
    );
    let _doc_embed = create(
        &mut st,
        ElementKind::DocEmbed,
        Point::new(80.0, 200.0),
        InitContent::DocEmbed {
            document_id: Id::<Document>::from_bytes([1; 16]),
            doc_type: DocType::Toile,
        },
        None,
    );
    let mut path = BezPath::new();
    path.move_to(Point::new(0.0, 0.0));
    path.line_to(Point::new(5.0, 5.0));
    let _path_shape = create(
        &mut st,
        ElementKind::Shape(ShapeKind::Path),
        Point::new(120.0, 200.0),
        InitContent::Path(path),
        None,
    );

    st.handle_command(ToileCommand::SetText {
        id: sticky.clone(),
        base: "note".to_owned(),
        text: "hello".to_owned(),
    });
    st.handle_command(ToileCommand::RestyleElements {
        ids: vec![rect.clone()],
        style: ElementStyle::default(),
    });
    st.handle_command(ToileCommand::SetLink {
        id: rect.clone(),
        link: Some(LinkTarget::Url("https://example.invalid".into())),
    });
    st.handle_command(ToileCommand::LockElements {
        ids: vec![rect.clone()],
        lock: ambre_bus::app::toile::LockState::Locked,
    });
    st.handle_command(ToileCommand::SetFrameChildren {
        frame: frame.clone(),
        add: vec![sticky.clone()],
        remove: Vec::new(),
    });
    st.handle_command(ToileCommand::RenameFrame {
        frame: frame.clone(),
        name: "Slide".to_owned(),
    });
    st.handle_command(ToileCommand::SetAutoLayout {
        frame: slide.clone(),
        layout: Some(AutoLayout {
            direction: Direction::Horizontal,
            gap: ambre_core::BoardLength::new(8.0),
            padding: ambre_bus::app::toile::Padding::uniform(ambre_core::BoardLength::new(4.0)),
            align: Align::Start,
            sizing: Sizing::Hug,
        }),
    });
    st.handle_command(ToileCommand::SetSlideOrder {
        order: vec![slide.clone()],
    });
    st.handle_command(ToileCommand::EditNotes {
        frame: slide,
        base: String::new(),
        delta: TextDelta {
            at: CharOffset::ZERO,
            delete: CharCount::ZERO,
            insert: "say this".to_owned(),
        },
    });
    st.handle_command(ToileCommand::CommitStroke {
        points: vec![
            StrokePoint::full_pressure(
                ambre_core::StoredBoardLength::new(0.0),
                ambre_core::StoredBoardLength::new(0.0),
            ),
            StrokePoint::full_pressure(
                ambre_core::StoredBoardLength::new(1.0),
                ambre_core::StoredBoardLength::new(1.0),
            ),
        ],
        style: ElementStyle::default(),
    });
    st.handle_command(ToileCommand::CreateConnector {
        from: rect.clone(),
        from_anch: Anchor::Right,
        to: sticky.clone(),
        to_anch: Anchor::Left,
        path_type: PathType::Straight,
        arrows: EndCaps {
            start: ambre_bus::app::toile::Cap::Plain,
            end: ambre_bus::app::toile::Cap::Arrow,
        },
    });
    let connector = st
        .layer_order()
        .iter()
        .rev()
        .find_map(|n| match n {
            NodeId::Connector(c) => Some(c.clone()),
            NodeId::Element(_) => None,
        })
        .expect("the connector is in the paint order");
    st.handle_command(ToileCommand::SetConnectorLabel {
        id: connector,
        base: String::new(),
        text: "edge".to_owned(),
    });
    st.handle_command(ToileCommand::React {
        target: rect.clone(),
        reaction: Some(Reaction {
            kind: ReactionKind::Emoji(ambre_bus::Emoji::from("+1")),
            ts: Instant::<Authored>::new(1),
        }),
    });
    let opened = st.handle_command(ToileCommand::CreateComment {
        anchor: CommentAnchor::Element(rect),
        body: "a remark".to_owned(),
        ts: Instant::<Authored>::new(2),
    });
    let thread = opened
        .display
        .iter()
        .find_map(|e| match e {
            ToileEvent::CommentThreadOpened(t) => Some(t.clone()),
            _ => None,
        })
        .expect("the comment opens a thread");
    st.handle_command(ToileCommand::ReplyComment {
        thread: thread.clone(),
        body: "an answer".to_owned(),
        ts: Instant::<Authored>::new(3),
    });
    st.handle_command(ToileCommand::ResolveThread {
        thread,
        state: ambre_core::ThreadState::Resolved,
    });
    // The media map is seeded by the importer, which no command reaches from
    // here: its entries are the blobs an import carries. The thumbnail is a
    // real PNG, so the reading belt (`SCHEMA-017`) reads its size back.
    let mut thumb = Vec::new();
    image::RgbaImage::new(1, 2)
        .write_to(
            &mut std::io::Cursor::new(&mut thumb),
            image::ImageFormat::Png,
        )
        .expect("a 1x2 image encodes");
    st.seed_media(&[MediaBlob {
        media: crate::logic::command::MediaRef::Stored(sample_hash()),
        plaintext: ambre_core::Payload::<ambre_core::MediaBytes>::new(vec![1]),
        mime: ambre_core::MediaType::from("image/png"),
        size: ambre_core::Size::new(Px::new(1), Px::new(2)),
        thumb: ambre_core::Payload::<ambre_core::ThumbnailPng>::new(thumb),
    }]);
    st.doc.commit();
    st
}

#[test]
fn the_shape_a_build_writes_is_the_shape_frozen_for_its_version() {
    let state = board_of_every_key();
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
         SCHEMA_VERSION {SCHEMA_VERSION}. A key, a value type or a `postcard` \
         layout moved, so raise SCHEMA_VERSION and freeze the new shape as its \
         own section (`SCHEMA-018`)."
    );
}

/// Every build of one version writes the same scaffolding (`AUTHOR-024`).
/// The identity hashes the scaffolding's bytes, so it moves with any of them.
#[test]
fn the_scaffold_a_build_writes_is_the_scaffold_frozen_for_its_version() {
    let written = format!("{:016x}\n", super::scaffold_identity());
    assert_eq!(
        section_of(SCAFFOLD, SCHEMA_VERSION).as_deref(),
        Some(written.as_str()),
        "the scaffolding this build writes is not the one frozen for \
         SCHEMA_VERSION {SCHEMA_VERSION} in `schema/scaffold.txt`. Another \
         build of that version writes it under another identity, and a delta \
         that waits on it never applies. Raise SCHEMA_VERSION and append a section."
    );
}

/// The canonical board of the version a step leaves: that version's layout,
/// holding what the step rewrites. Each step reads its own, so a step added
/// later never moves what an earlier one writes. The registry holds no step
/// yet, so no version has one.
fn canonical(from: ambre_core::schema::SchemaVersion) -> LoroDoc {
    panic!("the step from version {from} has no canonical board: write one here")
}

/// Each step of the registry, run as the gate runs it on its canonical board.
/// One line per step: `from -> to` and what it wrote.
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
            (step.apply)(&scratch).expect("the step runs on its canonical board");
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
         migrates the same board another way. Restore the step, or raise \
         SCHEMA_VERSION and append a section."
    );
}

#[test]
fn every_declared_key_appears_in_the_shape() {
    let state = board_of_every_key();
    let shape = shape_of(&state.doc);
    let declared = KEY_SOURCES.iter().flat_map(|source| declared_keys(source));
    for key in declared.chain(schema_names()) {
        {
            assert!(
                shape.keys.contains(&key),
                "`{key}` is declared as a schema key and no line of the shape \
                 carries it. Either the scenario stopped reaching it, or the \
                 key is new and the scenario has to write it before the freeze \
                 can hold it."
            );
        }
    }
}

/// `SCHEMA-018`: the shape holds the keys and the types, so a changed value
/// does not move it.
#[test]
fn a_value_changing_leaves_the_shape_alone() {
    let first = board_of_every_key();
    let mut second = board_of_every_key();
    let id = second
        .layer_order()
        .iter()
        .find_map(|n| match n {
            NodeId::Element(e) => Some(e.clone()),
            NodeId::Connector(_) => None,
        })
        .expect("the scenario creates an element");
    second.handle_command(ToileCommand::LockElements {
        ids: vec![id],
        lock: ambre_bus::app::toile::LockState::Unlocked,
    });
    assert_eq!(
        render(&shape_of(&first.doc)),
        render(&shape_of(&second.doc)),
        "the shape reads the types and not the values"
    );
}

#[test]
fn a_key_appearing_moves_the_shape() {
    let state = board_of_every_key();
    let before = render(&shape_of(&state.doc));
    state
        .doc
        .get_map(ambre_core::crdt::FoundationRoot::Meta.as_str())
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
    let state = board_of_every_key();
    let before = render(&shape_of(&state.doc));
    // A board colour is stored as an `i64`; the same key under a float is the
    // drift a reviewer cannot be asked to spot.
    super::board(&state.doc)
        .insert(BoardKey::BgColor, 1.0_f64)
        .expect("the register takes a key");
    assert_ne!(
        before,
        render(&shape_of(&state.doc)),
        "a value type changing has to move the shape"
    );
}
