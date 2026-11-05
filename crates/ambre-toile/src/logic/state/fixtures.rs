//! Toile's half of the fixture loader surface (`ambre-spec-fixtures.md`).
//!
//! `shape_of` and `canonical_dump` are declared in `ambre-logic` over `DocState`.
//! Their per-app bodies live here, next to the schema they read, for the same
//! reason the corpus puts generators next to their engine. A schema change and
//! the code that decodes it move in one commit. `ambre-logic` only dispatches.
//!
//! Fixture code (I4). The whole module sits behind the non-default `fixtures`
//! feature and is unreachable from any client build. The determinism seam
//! (`ToileState::with_peer`) is *not* here -- it is engine API, documented in
//! `toile-engine` and ungated, as is [`ToileState::import_snapshot`].
//! A generator aborts on a corpus it cannot build, so the panic lints of
//! `ARCH-028` do not hold here.

#![expect(
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "a fixture generator aborts on a corpus it cannot build"
)]

use ambre_core::index::Len;
use ambre_core::{Px, Version};
use std::collections::{BTreeMap, HashMap};

use ambre_core::id::{Connector, Element, Id};
use ambre_core::{
    Board, BoardFrames, Connectors, Count, DocType, Elements, FixtureGen, FixtureName, FixtureSeed,
    FixtureShape, Generator, ShapeBody, fixture_peer_count, seeded_document_id,
};
use loro::LoroValue;

use kurbo::{Point, Size};

use crate::logic::command::{ClipboardPayload, InitContent, ToileCommand, ToileEvent, ZTarget};
use crate::logic::model::{Anchor, ElementKind, EndCaps, NodeId, PathType, ShapeKind};

use super::ToileState;
use super::codec::value_has_type;
use super::names::{FieldValues, Root};

impl ToileState {
    /// The coarse shape of this board, as the manifest records it.
    ///
    /// Counted from the CRDT rather than declared by the generator. This
    /// checks the manifest against a recomputed shape; a generator declaring
    /// its own would assert only that it agrees with itself.
    pub fn fixture_shape(&self) -> FixtureShape {
        let mut elements = Count::<Elements>::ZERO;
        let mut frames = Count::<BoardFrames>::ZERO;
        if let LoroValue::Map(map) = self.doc.get_map(Root::Elements.as_str()).get_deep_value() {
            for value in map.values() {
                elements = elements.incremented();
                if let LoroValue::Map(fields) = value
                    && value_has_type(FieldValues::new(fields), ElementKind::Frame)
                {
                    frames = frames.incremented();
                }
            }
        }
        let connectors = match self.doc.get_map(Root::Connectors.as_str()).get_value() {
            LoroValue::Map(map) => Count::<Connectors>::of(map.len()),
            _ => Count::<Connectors>::ZERO,
        };
        FixtureShape {
            // The loader's own pinned peer is not an author of the document.
            peers: fixture_peer_count(
                self.doc
                    .oplog_vv()
                    .keys()
                    .copied()
                    .map(ambre_core::PeerId::new),
                super::scaffold_identity(),
            ),
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
    /// **Not** the CRDT. Two boards agreeing on content still differ in op
    /// history, peer ids and commit timestamps. No byte or state comparison can
    /// express "same document by a different route". This is what the frozen belt
    /// compares, and text means a failure prints a readable diff rather than a
    /// length mismatch.
    ///
    /// **Nodes are walked in paint order and named by their position in it**, not
    /// by their [`Id<Element>`](ambre_core::id::Id). An id is `(peer,
    /// counter)`, and is therefore exactly what two documents reached by
    /// different routes are allowed to differ in. Z-order *is* user-visible, so
    /// it makes a canonical index. Every id-valued field -- a connector endpoint,
    /// a group's `children` -- is rewritten to that index for the same reason.
    ///
    /// **Floats are formatted shortest-round-trip** (Rust's `{}` for `f64`),
    /// which is deterministic for a given value. It is *not* a tolerance. Two
    /// paths that compute geometry differently to the last bit differ here, and
    /// that is intended. A migration is not supposed to move geometry at all.
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

        let elements = match self.doc.get_map(Root::Elements.as_str()).get_deep_value() {
            LoroValue::Map(m) => m,
            _ => Default::default(),
        };
        let connectors = match self.doc.get_map(Root::Connectors.as_str()).get_deep_value() {
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
        // rendered content so the section is itself deterministic. Counted
        // rather than named: an unlayered node has no canonical index.
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
/// names a node in `position` becomes `#<paint index>`. That is what makes a
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
    fn name(&self) -> Result<ambre_core::Validated<FixtureName>, ambre_core::Refused<FixtureName>> {
        ambre_core::Validated::parse(match self {
            Self::ShapesAndFrame => "toile/shapes_and_frame",
            Self::ConnectorsAndGroups => "toile/connectors_and_groups",
        })
    }

    fn doc_type(&self) -> DocType {
        DocType::Toile
    }

    fn seed(&self) -> FixtureSeed {
        // Distinct per generator, so no two artifacts share a `document_id` and
        // a copied file is visible in the diff rather than plausible.
        FixtureSeed::new(match self {
            Self::ShapesAndFrame => 0x5EED_70115_u64 << 16 | 0x0001,
            Self::ConnectorsAndGroups => 0x5EED_70115_u64 << 16 | 0x0002,
        })
    }

    fn generator_version(&self) -> Version<Generator> {
        // 2: a colour is stored `0xRRGGBBAA`. An artifact frozen before
        // stores `0xAARRGGBB`, and no migration rewrites it.
        Version::<Generator>::new(2)
    }

    fn build(&self) -> ambre_core::Payload<ambre_core::PlaintextContainer> {
        let seed = self.seed();
        let mut state = ToileState::with_peer(seed.peer());
        match self {
            Self::ShapesAndFrame => build_shapes_and_frame(&mut state),
            Self::ConnectorsAndGroups => build_connectors_and_groups(&mut state),
        }
        state
            .export_ambre_plaintext(seeded_document_id(seed))
            .expect("the document reads")
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
    let ids: Vec<Id<Element>> = (0..6)
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
        lock: crate::message::LockState::Locked,
    });
}

/// Four shapes, a group over two of them, and three connectors, one captioned.
///
/// Every one of those carries an id-valued field: a group's `children`, a
/// connector's endpoints. The canonical dump rewrites each such field to a paint
/// index. Two documents built by different routes then compare equal.
fn build_connectors_and_groups(state: &mut ToileState) {
    let ids: Vec<Id<Element>> = (0..4)
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
                start: crate::message::Cap::Plain,
                end: crate::message::Cap::Arrow,
            },
        });
        state.handle_command(ToileCommand::SetConnectorLabel {
            id: last_connector(state),
            base: String::new(),
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
) -> Id<Element> {
    let outcome = state.handle_command(ToileCommand::CreateElement {
        kind,
        at,
        size,
        content,
        fill: None,
        correlate: None,
    });
    outcome
        .display
        .iter()
        .find_map(|event| match event {
            ToileEvent::ElementCreated { id, .. } => Some(id.clone()),
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
fn last_connector(state: &ToileState) -> Id<Connector> {
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

// -----------------------------------------------------------------------------
// The scale generators
// -----------------------------------------------------------------------------

/// Which board a scale generator builds (`FIXTURE-022`).
///
/// A second enum beside [`ToileFixture`], not two more variants of it. The two
/// registries go to two directories. Git carries a correctness artifact and
/// never a scale one. One registry per destination therefore keeps
/// `just fixtures` from writing a multi-megabyte file into the working copy.
///
/// * Every variant takes a figure `ambre-spec-budgets.md` already states. This
///   module invents no size and repeats no target.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ToileBenchFixture {
    /// 50 000 shapes spread over a wide plane. The visible-set query and index
    /// update budgets name that figure.
    Board50k,
    /// 2 000 shapes inside one default viewport. The board paint budget names
    /// that figure, and it counts *visible* elements, which a spread board
    /// cannot supply.
    DenseViewport2k,
    /// 50 frames, each holding eight elements and a speaker note. The PDF
    /// export budget names 50 slides.
    Deck50,
}

/// Toile's half of the scale registry (`ambre-logic`'s `BENCH_REGISTRY`).
pub const TOILE_BENCH_FIXTURES: &[&dyn FixtureGen] = &[
    &ToileBenchFixture::Board50k,
    &ToileBenchFixture::DenseViewport2k,
    &ToileBenchFixture::Deck50,
];

impl FixtureGen for ToileBenchFixture {
    fn name(&self) -> Result<ambre_core::Validated<FixtureName>, ambre_core::Refused<FixtureName>> {
        ambre_core::Validated::parse(match self {
            Self::Board50k => "toile/bench_board_50k",
            Self::DenseViewport2k => "toile/bench_dense_viewport_2k",
            Self::Deck50 => "toile/bench_deck_50",
        })
    }

    fn doc_type(&self) -> DocType {
        DocType::Toile
    }

    fn seed(&self) -> FixtureSeed {
        // A range of its own, disjoint from the correctness seeds above, so no
        // scale artifact shares a `document_id` with a committed one.
        FixtureSeed::new(match self {
            Self::Board50k => 0x5EED_B0C4_0001_0001,
            Self::DenseViewport2k => 0x5EED_B0C4_0001_0002,
            Self::Deck50 => 0x5EED_B0C4_0001_0003,
        })
    }

    fn generator_version(&self) -> Version<Generator> {
        Version::<Generator>::new(1)
    }

    fn build(&self) -> ambre_core::Payload<ambre_core::PlaintextContainer> {
        let seed = self.seed();
        let mut state = ToileState::with_peer(seed.peer());
        match self {
            // 100 tiles of 500 on a 4 000-unit pitch: far wider than one
            // viewport, so a cull that returned everything shows up as a figure.
            Self::Board50k => build_spread_board(
                &mut state,
                Count::<Tiles>::new(100),
                Px::<Board>::new(4_000.0),
                Px::<Board>::new(160.0),
            ),
            // One tile of 500, then three more inside the 1280 by 800 default
            // camera, so every element of this board is a visible element.
            Self::DenseViewport2k => build_spread_board(
                &mut state,
                Count::<Tiles>::new(4),
                Px::<Board>::new(600.0),
                Px::<Board>::new(24.0),
            ),
            Self::Deck50 => build_deck(&mut state, Len::<crate::message::Slide>::new(50)),
        }
        state
            .export_ambre_plaintext(seeded_document_id(seed))
            .expect("the document reads")
    }
}

/// How many copies of the one built block a spread board holds.
pub enum Tiles {}

impl ambre_core::CountKind for Tiles {
    type Repr = u32;
    const NAME: &'static str = "TileCount";
}

/// Elements of one [`TILE`] block, spread over `tiles` copies on a
/// `tile_pitch`-unit lattice. Each block is 25 by 20 shapes on an
/// `element_pitch`-unit lattice.
///
/// **One block built, then pasted.** `CreateElement` derives the viewport once
/// per call. Building 50 000 elements one at a time therefore costs the square
/// of the count, and a measured run passed 20 minutes without finishing. A
/// paste re-mints a whole payload under one command and one derivation.
///
/// * The generator still drives `handle_command` alone (`FIXTURE-004`). Copy
///   and paste are user commands, and this board is one a user could produce.
///   Nothing here writes a container.
///
/// * The three shape kinds alternate inside a block, so a tessellation bench
///   meets a mix rather than one geometry repeated.
fn build_spread_board(
    state: &mut ToileState,
    tiles: Count<Tiles>,
    tile_pitch: Px<Board>,
    element_pitch: Px<Board>,
) {
    const COLS: u32 = 25;
    const ROWS: u32 = 20;
    let (tiles, tile_pitch, element_pitch) = (tiles.get(), tile_pitch.get(), element_pitch.get());
    let kinds = [ShapeKind::Rect, ShapeKind::Ellipse, ShapeKind::Triangle];
    let ids: Vec<Id<Element>> = (0..ROWS)
        .flat_map(|row| (0..COLS).map(move |col| (row, col)))
        .enumerate()
        .map(|(index, (row, col))| {
            create(
                state,
                ElementKind::Shape(kinds[index % kinds.len()]),
                Point::new(
                    f64::from(col) * element_pitch,
                    f64::from(row) * element_pitch,
                ),
                Size::new(element_pitch * 0.6, element_pitch * 0.4),
                InitContent::None,
            )
        })
        .collect();
    // The board already holds the first block, so the copy lands `tiles - 1`
    // times. A square lattice of tiles keeps the board as wide as it is tall.
    let Some(payload) = copy_payload(state, ids) else {
        return;
    };
    let side = tiles.isqrt().max(1);
    for tile in 1..tiles {
        let (row, col) = (tile / side, tile % side);
        state.handle_command(ToileCommand::PasteElements {
            payload: payload.clone(),
            at: Point::new(f64::from(col) * tile_pitch, f64::from(row) * tile_pitch),
        });
    }
}

/// The clipboard payload of `ids`, read back off the `CopyElements` reply.
///
/// `None` when the reply carries no payload. A generator drives a command it
/// chose itself, so that is a defect here rather than a runtime condition.
fn copy_payload(state: &mut ToileState, ids: Vec<Id<Element>>) -> Option<ClipboardPayload> {
    state
        .handle_command(ToileCommand::CopyElements { ids })
        .display
        .into_iter()
        .find_map(|event| match event {
            ToileEvent::Clipboard(payload) => Some(payload),
            _ => None,
        })
}

/// `slides` frames in a row, each holding eight elements and a speaker note.
///
/// The frames declare their children and the deck declares its order. The
/// export walks that order and that membership, never the spatial overlap.
fn build_deck(state: &mut ToileState, slides: Len<crate::message::Slide>) {
    const PITCH: Px<Board> = Px::<Board>::new(1_400.0);
    let mut order = Vec::with_capacity(slides.to_usize());
    for slide in 0..slides.get() {
        let origin = f64::from(slide) * PITCH.get();
        let frame = create(
            state,
            ElementKind::Frame,
            Point::new(origin, 0.0),
            Size::new(1_280.0, 720.0),
            InitContent::None,
        );
        let title = create(
            state,
            ElementKind::Text,
            Point::new(origin + 60.0, 60.0),
            Size::new(900.0, 80.0),
            InitContent::Text(format!("slide {}", slide + 1)),
        );
        let mut children = vec![title];
        children.extend((0..7).map(|i| {
            let kind = if i % 2 == 0 {
                ElementKind::Shape(ShapeKind::Rect)
            } else {
                ElementKind::Sticky
            };
            let content = match kind {
                ElementKind::Sticky => InitContent::Text(format!("point {i} of slide {slide}")),
                _ => InitContent::None,
            };
            create(
                state,
                kind,
                Point::new(origin + 60.0 + f64::from(i) * 160.0, 220.0),
                Size::new(140.0, 120.0),
                content,
            )
        }));
        state.handle_command(ToileCommand::SetFrameChildren {
            frame: frame.clone(),
            add: children,
            remove: Vec::new(),
        });
        order.extend(state.frame(&frame));
    }
    state.handle_command(ToileCommand::SetSlideOrder { order });
}
