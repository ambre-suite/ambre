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

use ambre_core::{FixtureShape, ShapeBody};
use loro::LoroValue;

use crate::logic::model::NodeId;

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
            peers: u8::try_from(self.doc.oplog_vv().len()).unwrap_or(u8::MAX),
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
