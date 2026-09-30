//! The schema shape belt: the key set and the value types a build writes,
//! frozen per `SCHEMA_VERSION` (`MURMURE-437`, `SCHEMA-018`).
//!
//! A shape is a channel and a space with every value removed: one line per
//! reachable path, each naming the type under it. A line starts with the
//! document kind, since a channel is several documents (`MURMURE-021`). `*`
//! stands for a list index, and `{id}` for the identifier keying a map entry.
//!
//! The walk reads containers, not the deep value. A deep value turns a
//! `LoroText` into a string. A body stored as a plain string is the drift
//! `MURMURE-004` forbids, so the two must render apart.
//!
//! [`every_declared_key_appears_in_the_shape`] reads `schema.rs` and demands a
//! line for every key it declares, so a key no scenario reaches cannot drop
//! out of the freeze unseen.

use std::collections::BTreeSet;

use ambre_bus::{MentionTarget, MessageDraft, SealedChannelKey, Timestamp, Topic};
use ambre_core::{CategoryId, ChannelId, SpaceId, UserId};
use loro::{Container, LoroDoc, LoroValue, ValueOrContainer};

use crate::schema::{DocumentKind, SCHEMA_VERSION};
use crate::{ChannelState, ChannelType, Listing, SpaceState};

/// The frozen shapes, one section per `SCHEMA_VERSION`.
const FROZEN: &str = include_str!("../schema/shapes.txt");

/// The source the key declarations are read from.
const SCHEMA_SOURCE: &str = include_str!("schema.rs");

/// The segment a list index collapses to.
const ANY: &str = "*";

/// The segment an identifier key collapses to.
const ID: &str = "{id}";

/// The maps keyed by an identifier, whose keys are values and not schema.
const KEYED_BY_ID: [&str; 2] = ["space:categories", "space:channels"];

/// The declared `&str` constants that are stored values, not keys.
const NOT_SCHEMA_KEYS: [&str; 0] = [];

/// A channel's shape: the frozen lines, and the key segments met on the way.
#[derive(Default)]
struct Shape {
    lines: BTreeSet<String>,
    keys: BTreeSet<String>,
}

/// The type a plain value holds, in the spelling the freeze uses.
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

impl Shape {
    /// Record `path`, then descend into it when it is a container.
    fn walk(&mut self, path: &str, item: ValueOrContainer) {
        match item {
            ValueOrContainer::Value(value) => {
                self.lines.insert(format!("{path}: {}", type_name(&value)));
            }
            ValueOrContainer::Container(Container::Map(map)) => {
                self.lines.insert(format!("{path}: map"));
                let by_id = KEYED_BY_ID.contains(&path);
                map.for_each(|key, child| {
                    if by_id {
                        self.walk(&format!("{path}/{ID}"), child);
                    } else {
                        self.keys.insert(key.to_owned());
                        self.walk(&format!("{path}/{key}"), child);
                    }
                });
            }
            ValueOrContainer::Container(Container::List(list)) => {
                self.lines.insert(format!("{path}: list"));
                list.for_each(|child| self.walk(&format!("{path}/{ANY}"), child));
            }
            ValueOrContainer::Container(Container::Text(_)) => {
                self.lines.insert(format!("{path}: text"));
            }
            // Murmure writes no tree, movable list or counter. Naming the
            // container kind still moves a line if one ever appears.
            ValueOrContainer::Container(other) => {
                self.lines.insert(format!("{path}: {:?}", other.get_type()));
            }
        }
    }

    /// Walk every root of one document, under its kind.
    fn walk_doc(&mut self, kind: DocumentKind, doc: &LoroDoc) {
        let LoroValue::Map(roots) = doc.get_value() else {
            return;
        };
        for (name, root) in roots.iter() {
            let LoroValue::Container(id) = root else {
                continue;
            };
            let Some(container) = doc.get_container(id.clone()) else {
                continue;
            };
            self.keys.insert(name.clone());
            self.walk(
                &format!("{}:{name}", kind.as_str()),
                ValueOrContainer::Container(container),
            );
        }
    }
}

/// A channel that reaches every key the schema declares: a name, and a
/// message in the active segment.
fn channel_of_every_key() -> ChannelState {
    let mut state = ChannelState::create(
        ChannelId::new(),
        ChannelType::Public,
        Some("general"),
        Timestamp(1_700_000_000_000),
    )
    .expect("create");
    state
        .send(
            &MessageDraft {
                text: "hello".to_owned(),
                mentions: vec![MentionTarget::Channel],
            },
            UserId::new(),
            Timestamp(1_700_000_000_001),
        )
        .expect("send");
    state
}

/// A space that reaches every key the schema declares: a name, a category,
/// and a listed channel with a topic and a category.
fn space_of_every_key(channel: &ChannelState) -> SpaceState {
    let mut space =
        SpaceState::create(SpaceId::new(), "Acme", Timestamp(1_700_000_000_000)).expect("create");
    let category = CategoryId::new();
    space.put_category(category, "Teams", 0).expect("category");
    let topic = Topic::new("everything").expect("a topic");
    space
        .list_channel(
            channel,
            &Listing {
                topic: Some(&topic),
                category: Some(category),
                position: 0,
                key: SealedChannelKey::from_bytes(vec![1; 72]),
            },
        )
        .expect("list");
    space
}

fn shape_of(state: &ChannelState) -> Shape {
    let mut shape = Shape::default();
    for (kind, id) in [
        (DocumentKind::Meta, state.meta_id().document_id()),
        (DocumentKind::Segment, state.active_id().document_id()),
    ] {
        shape.walk_doc(kind, state.doc(id).expect("held"));
    }
    shape.walk_doc(DocumentKind::Space, space_of_every_key(state).doc());
    shape
}

fn render(shape: &Shape) -> String {
    let mut out = String::new();
    for line in &shape.lines {
        out.push_str(line);
        out.push('\n');
    }
    out
}

/// The section of the freeze that names this version, if any.
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

/// Every `const NAME: &str = "literal";` the schema source declares.
fn declared_string_consts(source: &str) -> Vec<String> {
    let mut found = Vec::new();
    for line in source.lines() {
        let Some((_, rest)) = line.trim_start().split_once("const ") else {
            continue;
        };
        let Some((_, tail)) = rest.split_once(": &str = ") else {
            continue;
        };
        let Some(literal) = tail.strip_prefix('"').and_then(|t| t.split('"').next()) else {
            continue;
        };
        if !literal.is_empty() && !NOT_SCHEMA_KEYS.contains(&literal) {
            found.push(literal.to_owned());
        }
    }
    found
}

#[test]
fn the_shape_a_build_writes_is_the_shape_frozen_for_its_version() {
    let written = render(&shape_of(&channel_of_every_key()));
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
         SCHEMA_VERSION and freeze the new shape as its own section (`MURMURE-020`)."
    );
}

#[test]
fn every_declared_key_appears_in_the_shape() {
    let shape = shape_of(&channel_of_every_key());
    let declared = declared_string_consts(SCHEMA_SOURCE);
    assert!(!declared.is_empty(), "the declaration reader found no key");
    for key in declared {
        assert!(
            shape.keys.contains(&key),
            "`{key}` is declared as a schema key and no line of the shape \
             carries it. The scenario has to write it before the freeze can hold it."
        );
    }
}

#[test]
fn a_body_stored_as_a_string_moves_the_shape() {
    let state = channel_of_every_key();
    let before = render(&shape_of(&state));
    let doc = state.doc(state.active_id().document_id()).expect("active");
    let Some(ValueOrContainer::Container(Container::Map(message))) =
        doc.get_list(crate::schema::MESSAGES).get(0)
    else {
        panic!("a message is a map container");
    };
    message
        .insert(crate::schema::MESSAGE_CONTENT, "flat")
        .expect("overwrite");
    doc.commit();
    assert_ne!(render(&shape_of(&state)), before);
}

#[test]
fn a_value_changing_leaves_the_shape_alone() {
    assert_eq!(
        render(&shape_of(&channel_of_every_key())),
        render(&shape_of(&channel_of_every_key()))
    );
}
