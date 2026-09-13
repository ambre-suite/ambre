//! Toile's half of the fixture loader surface (`ambre-spec-fixtures.md`).
//!
//! `shape_of` and `canonical_dump` are declared in `ambre-logic` over `DocState`;
//! their per-app bodies live here, next to the schema they read, for the same
//! reason puts generators next to their engine: a schema change and the code
//! that decodes it move in one commit. `ambre-logic` only dispatches.
//!
//! Fixture code (I4): the whole module sits behind the non-default `fixtures`
//! feature and is unreachable from any client build. The determinism seam
//! (`ToileState::with_peer`) is *not* here -- it is engine API, documented in
//! `toile-engine` and ungated, as is
//! [`ToileState::import_snapshot`].

use std::collections::{BTreeMap, HashMap};

use ambre_core::{
    DocType, FixtureGen, FixtureShape, ShapeBody, fixture_peer_count, seeded_document_id,
};
use loro::LoroValue;

use kurbo::{Point, Size};

use crate::logic::command::{InitContent, ToileCommand, ToileEvent, ZTarget};
use crate::logic::model::{
    Anchor, ConnectorId, ElementId, ElementKind, EndCaps, NodeId, PathType, ShapeKind,
};

use super::{CONNECTORS, ELEMENTS, ToileState};

impl ToileState {
    /// The coarse shape of this board, as the manifest records it.
    ///
    /// Counted from the CRDT rather than declared by the generator: checks the
    /// manifest against a recomputed shape, and a generator declaring its own
    /// would assert only that it agrees with itself.
    pub fn fixture_shape(&self) -> FixtureShape {
        let mut elements = 0u32;
        let mut frames = 0u16;
        if let LoroValue::Map(map) = self.doc.get_map(ELEMENTS).get_deep_value() {
            for value in map.values() {
                elements = elements.saturating_add(1);
                if let LoroValue::Map(fields) = value
                    && matches!(fields.get("type"), Some(LoroValue::String(t)) if t.as_str() == "frame")
                {
                    frames = frames.saturating_add(1);
                }
            }
        }
        let connectors = match self.doc.get_map(CONNECTORS).get_value() {
            LoroValue::Map(map) => u32::try_from(map.len()).unwrap_or(u32::MAX),
            _ => 0,
        };
        FixtureShape {
            // The loader's own pinned peer is not an author of the document.
            peers: fixture_peer_count(self.doc.oplog_vv().keys().copied()),
            body: ShapeBody::Toile {
                elements,
                frames,
                connectors,
            },
        }
    }

    /// The canonical semantic form of this board: every user-visible value, in a
    /// deterministic order, as text.
    ///
    /// **Not** the CRDT -- two boards agreeing on content still differ in op
    /// history, peer ids and commit timestamps, so no byte or state comparison
    /// can express "same document by a different route". This is what the
    /// frozen belt of compares, and text means a failure prints a readable
    /// diff rather than a length mismatch.
    ///
    /// **Nodes are walked in paint order and named by their position in it**, not
    /// by their [`ElementId`](crate::logic::model::ElementId): an id is
    /// `(peer, counter)` and is therefore exactly what two documents reached by
    /// different routes are allowed to differ in. Z-order *is* user-visible, so
    /// it makes a canonical index. Every id-valued field -- a connector endpoint,
    /// a group's `children` -- is rewritten to that index for the same reason.
    ///
    /// **Floats are formatted shortest-round-trip** (Rust's `{}` for `f64`),
    /// which is deterministic for a given value. It is *not* a tolerance: two
    /// paths that compute geometry differently to the last bit differ here, and
    /// that is intended -- a migration is not supposed to move geometry at all.
    pub fn canonical_dump(&self) -> String {
        // The paint order, materialized: `self.layers` is derived from the
        // `layers` movable list and is the order the user sees.
        let order = self.layer_order();
        let mut position: HashMap<String, usize> = HashMap::new();
        for (i, node) in order.iter().enumerate() {
            let key = match node {
                NodeId::Element(id) => id.as_str().to_owned(),
                NodeId::Connector(id) => id.as_str().to_owned(),
            };
            position.insert(key, i);
        }

        let elements = match self.doc.get_map(ELEMENTS).get_deep_value() {
            LoroValue::Map(m) => m,
            _ => Default::default(),
        };
        let connectors = match self.doc.get_map(CONNECTORS).get_deep_value() {
            LoroValue::Map(m) => m,
            _ => Default::default(),
        };

        let mut out = String::from("toile\n");
        for (i, node) in order.iter().enumerate() {
            match node {
                NodeId::Element(id) => {
                    let Some(fields) = elements.get(id.as_str()) else {
                        continue;
                    };
                    out.push_str(&format!("[{i}] element "));
                    out.push_str(&render(fields, &position));
                    out.push('\n');
                }
                NodeId::Connector(id) => {
                    let Some(fields) = connectors.get(id.as_str()) else {
                        continue;
                    };
                    out.push_str(&format!("[{i}] connector "));
                    out.push_str(&render(fields, &position));
                    out.push('\n');
                }
            }
        }
        // Anything the `layers` list does not mention is invisible to the user
        // but still in the document. Reported after the ordered nodes, sorted by
        // rendered content so the section is itself deterministic, and counted
        // rather than named -- an unlayered node has no canonical index.
        let mut orphans: Vec<String> = elements
            .iter()
            .filter(|(id, _)| !position.contains_key(*id))
            .map(|(_, fields)| format!("unlayered element {}\n", render(fields, &position)))
            .chain(
                connectors
                    .iter()
                    .filter(|(id, _)| !position.contains_key(*id))
                    .map(|(_, fields)| {
                        format!("unlayered connector {}\n", render(fields, &position))
                    }),
            )
            .collect();
        orphans.sort();
        for line in orphans {
            out.push_str(&line);
        }
        out
    }
}

/// Render one decoded CRDT value as canonical text.
///
/// Map keys are emitted in sorted order (`BTreeMap`), so the hash-map iteration
/// order Rust reseeds per process cannot leak into the output. Any string that
/// names a node in `position` becomes `#<paint index>`, which is what makes a
/// group's `children` and a connector's endpoints comparable across two
/// documents built by different routes.
fn render(value: &LoroValue, position: &HashMap<String, usize>) -> String {
    match value {
        LoroValue::Null => "null".to_owned(),
        LoroValue::Bool(b) => b.to_string(),
        LoroValue::Double(d) => format!("{d}"),
        LoroValue::I64(i) => i.to_string(),
        // Hex, not a byte-array debug print: a postcard-encoded `ElementStyle`
        // is opaque either way, and hex keeps the line short enough to diff.
        LoroValue::Binary(bytes) => format!("0x{}", hex_of(bytes)),
        LoroValue::String(s) => match position.get(s.as_str()) {
            Some(i) => format!("#{i}"),
            None => format!("{:?}", s.as_str()),
        },
        LoroValue::List(items) => {
            let rendered: Vec<String> = items.iter().map(|v| render(v, position)).collect();
            format!("[{}]", rendered.join(", "))
        }
        LoroValue::Map(map) => {
            let sorted: BTreeMap<&str, &LoroValue> =
                map.iter().map(|(k, v)| (k.as_str(), v)).collect();
            let rendered: Vec<String> = sorted
                .into_iter()
                .map(|(k, v)| format!("{k}={}", render(v, position)))
                .collect();
            format!("{{{}}}", rendered.join(" "))
        }
        // A container handle never survives `get_deep_value`, which resolves it
        // into the branches above. Named so a future Loro value type is a
        // compile-visible gap rather than a silent one.
        LoroValue::Container(id) => format!("<container {id:?}>"),
    }
}

fn hex_of(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

// -----------------------------------------------------------------------------
// The correctness generators
// -----------------------------------------------------------------------------

/// Which board a generator builds.
///
/// One closed enum plus a `match`, rather than one type per document: the
/// registry holds trait objects, so the alternative is two near-identical impl
/// blocks. Here the compiler checks that every variant declares a name, a seed,
/// a version and a body.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ToileFixture {
    /// A frame holding shapes and text, with one restacked node and one locked
    /// element. The plain board, and the one whose paint order is not its
    /// creation order.
    ShapesAndFrame,
    /// Four shapes, one group over two of them, and three connectors with
    /// labels. The document whose canonical dump has to rewrite every
    /// id-valued field to a paint index (`FIXTURE-016`).
    ConnectorsAndGroups,
}

/// Toile's half of the shipping registry (`ambre-logic`'s `REGISTRY`).
///
/// Declared next to the generators rather than spelled out in `ambre-logic`. A
/// generator added here reaches the corpus without a second edit one crate down.
/// That second edit is the one a contributor forgets.
pub const TOILE_FIXTURES: &[&dyn FixtureGen] = &[
    &ToileFixture::ShapesAndFrame,
    &ToileFixture::ConnectorsAndGroups,
];

impl FixtureGen for ToileFixture {
    fn name(&self) -> &'static str {
        match self {
            Self::ShapesAndFrame => "toile/shapes_and_frame",
            Self::ConnectorsAndGroups => "toile/connectors_and_groups",
        }
    }

    fn doc_type(&self) -> DocType {
        DocType::Toile
    }

    fn seed(&self) -> u64 {
        // Distinct per generator, so no two artifacts share a `document_id` and
        // a copied file is visible in the diff rather than plausible.
        match self {
            Self::ShapesAndFrame => 0x5EED_70115_u64 << 16 | 0x0001,
            Self::ConnectorsAndGroups => 0x5EED_70115_u64 << 16 | 0x0002,
        }
    }

    fn generator_version(&self) -> u32 {
        1
    }

    fn build(&self) -> Vec<u8> {
        let seed = self.seed();
        let mut state = ToileState::with_peer(seed);
        match self {
            Self::ShapesAndFrame => build_shapes_and_frame(&mut state),
            Self::ConnectorsAndGroups => build_connectors_and_groups(&mut state),
        }
        state.export_ambre_plaintext(seeded_document_id(seed))
    }
}

/// A frame, six shapes, two stickies and a text node, then one restack and one
/// lock.
///
/// The restack is the point. It separates the paint order from the creation
/// order. A canonical dump that walked the `elements` map instead of the
/// `layers` list then shows up in the artifact rather than staying plausible.
fn build_shapes_and_frame(state: &mut ToileState) {
    create(
        state,
        ElementKind::Frame,
        Point::new(-40.0, -40.0),
        Size::new(640.0, 400.0),
        InitContent::None,
    );
    let shapes = [ShapeKind::Rect, ShapeKind::Ellipse, ShapeKind::Triangle];
    let ids: Vec<ElementId> = (0..6)
        .map(|i| {
            create(
                state,
                ElementKind::Shape(shapes[i % shapes.len()]),
                Point::new(f64::from(i as u32) * 90.0, 60.0),
                Size::new(70.0, 50.0),
                InitContent::None,
            )
        })
        .collect();
    for (i, body) in ["first note", "second note"].iter().enumerate() {
        create(
            state,
            ElementKind::Sticky,
            Point::new(f64::from(i as u32) * 140.0, 220.0),
            Size::new(120.0, 120.0),
            InitContent::Text((*body).to_owned()),
        );
    }
    let caption = create(
        state,
        ElementKind::Text,
        Point::new(0.0, 300.0),
        Size::new(300.0, 40.0),
        InitContent::Text("quarterly plan".to_owned()),
    );
    state.handle_command(ToileCommand::Reorder {
        node: NodeId::Element(caption),
        to: ZTarget::Back,
    });
    state.handle_command(ToileCommand::LockElements {
        ids: vec![ids[0].clone()],
        locked: true,
    });
}

/// Four shapes, a group over two of them, and three connectors, one captioned.
///
/// Every one of those carries an id-valued field: a group's `children`, a
/// connector's endpoints. The canonical dump rewrites each such field to a paint
/// index. Two documents built by different routes then compare equal.
fn build_connectors_and_groups(state: &mut ToileState) {
    let ids: Vec<ElementId> = (0..4)
        .map(|i| {
            create(
                state,
                ElementKind::Shape(ShapeKind::Rect),
                Point::new(f64::from(i as u32) * 160.0, 0.0),
                Size::new(100.0, 60.0),
                // No text seed: `CreateElement` accepts one for a sticky or a
                // text node only, and rejects it on a shape.
                InitContent::None,
            )
        })
        .collect();
    for (from, to, path_type) in [
        (0, 1, PathType::Straight),
        (1, 2, PathType::Elbow),
        (2, 3, PathType::Curved),
    ] {
        state.handle_command(ToileCommand::CreateConnector {
            from: ids[from].clone(),
            from_anch: Anchor::Right,
            to: ids[to].clone(),
            to_anch: Anchor::Left,
            path_type,
            arrows: EndCaps {
                start: false,
                end: true,
            },
        });
        state.handle_command(ToileCommand::SetConnectorLabel {
            id: last_connector(state),
            text: format!("{from} to {to}"),
        });
    }
    // Grouped after the connectors, so the group's `children` and the
    // connectors' endpoints name nodes that the paint order already holds.
    state.handle_command(ToileCommand::GroupElements {
        ids: vec![ids[2].clone(), ids[3].clone()],
    });
}

/// Create one element and return the id the engine minted.
///
/// Panics when the reply carries no `ElementCreated`. A generator drives
/// commands it chose itself. A refusal is therefore a defect in this module,
/// not a runtime condition (`FixtureGen::build`).
fn create(
    state: &mut ToileState,
    kind: ElementKind,
    at: Point,
    size: Size,
    content: InitContent,
) -> ElementId {
    let outcome = state.handle_command(ToileCommand::CreateElement {
        kind,
        at,
        size,
        content,
        fill: None,
    });
    outcome
        .display
        .iter()
        .find_map(|event| match event {
            ToileEvent::ElementCreated(id) => Some(id.clone()),
            _ => None,
        })
        .unwrap_or_else(|| panic!("CreateElement must report the element it minted: {kind:?}"))
}

/// The id of the connector a `CreateConnector` just minted.
///
/// Read off the paint order rather than off the reply, because the reply carries
/// no connector id. Reading is not writing: `FIXTURE-004` forbids a generator
/// *building* through the containers, and this one still builds through
/// `handle_command` alone.
fn last_connector(state: &ToileState) -> ConnectorId {
    state
        .layer_order()
        .iter()
        .rev()
        .find_map(|node| match node {
            NodeId::Connector(id) => Some(id.clone()),
            NodeId::Element(_) => None,
        })
        .unwrap_or_else(|| panic!("CreateConnector must leave a connector in the paint order"))
}
