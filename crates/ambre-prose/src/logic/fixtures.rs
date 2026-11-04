//! Prose's half of the fixture corpus (`ambre-spec-fixtures.md`).
//!
//! `shape_of` and `canonical_dump` are declared in `ambre-logic` over
//! `DocState`. Their per-application bodies live here, beside the schema they
//! read, for the same reason the corpus puts a generator next to its engine. A
//! schema change and the code that decodes it move in one commit.
//!
//! Fixture code. The whole module sits behind the non-default `fixtures`
//! feature and no client build reaches it. The determinism seam
//! (`ProseState::with_peer`) is engine API and stays ungated, as does
//! `ProseState::import_snapshot`.

use ambre_core::Version;
use ambre_core::index::{Len, Pos, Span};
use std::collections::{BTreeMap, HashMap};

use ambre_bus::app::grid::{CellPos, Col, Row};
use ambre_bus::app::prose::DocChar;
use ambre_core::id::{Footnote, Id, User};
use ambre_core::{
    Char, Count, DocBlocks, DocType, FixtureGen, FixtureName, FixtureSeed, FixtureShape, Generator,
    ShapeBody, fixture_peer_count, seeded_document_id,
};
use loro::{LoroTree, LoroValue, TextDelta, TreeID};

use crate::doc;
use crate::events::{
    BlockInit, HeaderField, HfTarget, ProseCommand, ProseEvent, ProseRefusal, block_id, tree_id,
};

use super::ProseState;
use ambre_core::{Authored, Instant, Ms, TimeSpan};

impl ProseState {
    /// The coarse shape of this document, as a manifest records it.
    ///
    /// Counted from the CRDT rather than declared by a generator. A generator
    /// declaring its own shape would assert only that it agrees with itself.
    #[must_use]
    pub fn fixture_shape(&self) -> FixtureShape {
        let blocks = Count::<DocBlocks>::of(self.document_order().len());
        let characters = self
            .document_order()
            .iter()
            .filter_map(|id| self.body_of(id))
            .fold(Len::<Char>::ZERO, |total, body| {
                total + Len::<Char>::of(body.len_unicode())
            });
        FixtureShape {
            // The loader's own pinned peer is not an author of the document.
            // `u64::MAX` is never a peer: without a scaffolding identity,
            // nothing is subtracted.
            peers: fixture_peer_count(
                self.doc
                    .oplog_vv()
                    .keys()
                    .copied()
                    .map(ambre_core::PeerId::new),
                super::scaffold_identity().unwrap_or(ambre_core::PeerId::new(u64::MAX)),
            ),
            body: ShapeBody::Prose { blocks, characters },
        }
    }

    /// The canonical semantic form: every user-visible value, in document
    /// order, as text.
    ///
    /// **Not** the CRDT. Two documents agreeing on content still differ in
    /// operation history, peer ids and commit timestamps. A byte comparison
    /// cannot express "same document by a different route".
    ///
    /// **Blocks are named by their place**, never by their `TreeID`. The walk
    /// takes the flow, then the bands, then the notes as the text calls them.
    /// **An annotation is named by its first mention** in that walk. Both ids
    /// are exactly what two routes to one document may differ in
    /// (`FIXTURE-016`). A reply id and a hiding peer are masked for the same
    /// reason.
    #[must_use]
    pub fn canonical_dump(&self) -> String {
        let tree = doc::blocks(&self.doc);
        let order = self.dump_order(&tree);
        let names = PlaceNames::of(&order);

        let mut out = String::from("prose\n");
        for (i, id) in order.iter().enumerate() {
            let meta = doc::node_of(&tree, id)
                .map(|node| node.map().get_deep_value())
                .unwrap_or(LoroValue::Null);
            let parent = match tree.parent(*id) {
                Some(loro::TreeParentId::Node(parent)) => names.rename(&parent.to_string()),
                _ => "root".to_owned(),
            };
            out.push_str(&format!("[{i}] under {parent} {}\n", render(&meta, &names)));
            let delta = doc::get_block_text(&self.doc, id).to_delta();
            if !delta.is_empty() {
                out.push_str(&format!("[{i}] text {}\n", render_delta(&delta, &names)));
            }
        }
        for root in [doc::Root::Meta, doc::Root::Styles] {
            let label = root.as_str();
            let value = doc::root_map(&self.doc, root).get_deep_value();
            out.push_str(&format!("{label} {}\n", render(&value, &names)));
        }
        let annots = match doc::annots(&self.doc).get_deep_value() {
            LoroValue::Map(map) => map
                .iter()
                .map(|(k, v)| Annotation {
                    id: k.clone(),
                    body: render(v, &names),
                })
                .collect::<Vec<_>>(),
            _ => Vec::new(),
        };
        rename_annotations(out, annots)
    }

    /// Every live block the dump names, in its canonical order.
    fn dump_order(&self, tree: &LoroTree) -> Vec<TreeID> {
        let mut order = self.document_order();
        let bands = self.bands();
        for band in [
            bands.header,
            bands.footer,
            bands.header_first,
            bands.footer_first,
        ]
        .into_iter()
        .flatten()
        {
            push_subtree(tree, tree_id(band), &mut order);
        }
        // A note in the order its carrier sits in the text.
        let mut notes: Vec<TreeID> = Vec::new();
        for id in order.clone() {
            for span in doc::get_block_text(&self.doc, &id).to_delta() {
                let TextDelta::Insert {
                    attributes: Some(attributes),
                    ..
                } = span
                else {
                    continue;
                };
                if let Some(LoroValue::String(note)) =
                    attributes.get(crate::mark::MarkRoot::Footnote.as_str())
                    && let Some(uuid) = Id::<Footnote>::parse_str(note.as_str()).ok()
                    && let Some(block) = crate::note::block(&self.doc, &uuid)
                {
                    notes.push(block);
                }
            }
        }
        for note in notes {
            push_subtree(tree, note, &mut order);
        }
        order
    }
}

/// `root` and its live descendants in pre-order, each once.
fn push_subtree(tree: &LoroTree, root: TreeID, order: &mut Vec<TreeID>) {
    let mut stack = vec![root];
    while let Some(id) = stack.pop() {
        if order.contains(&id) {
            continue;
        }
        order.push(id);
        let mut children = doc::children_of(tree, Some(id));
        children.reverse();
        stack.extend(children);
    }
}

/// The place label of each block a dump names, keyed by its id as a
/// string. The first block of the walk is `#0`.
struct PlaceNames(HashMap<String, String>);

impl PlaceNames {
    /// Each block of `order` named by its index in it.
    fn of(order: &[TreeID]) -> Self {
        Self(
            order
                .iter()
                .enumerate()
                .map(|(i, id)| (id.to_string(), format!("#{i}")))
                .collect(),
        )
    }

    /// A block id by its place, or any other string quoted.
    fn rename(&self, value: &str) -> String {
        self.0
            .get(value)
            .cloned()
            .unwrap_or_else(|| format!("{value:?}"))
    }
}

/// One annotation of a dump: its id as the store keys it, and its rendered
/// body.
struct Annotation {
    id: String,
    body: String,
}

/// Name every annotation by its first mention in `dump`, then list them.
///
/// An annotation id is a UUID, so a plain substring replacement cannot hit
/// anything but itself. One nobody mentions is listed last, by content.
fn rename_annotations(mut dump: String, annots: Vec<Annotation>) -> String {
    let mut mentioned: Vec<(usize, String, String)> = Vec::new();
    let mut silent: Vec<String> = Vec::new();
    for Annotation { id, body } in annots {
        match dump.find(id.as_str()) {
            Some(at) => mentioned.push((at, id, body)),
            None => silent.push(body),
        }
    }
    mentioned.sort();
    let mut section = String::new();
    for (i, (_, id, body)) in mentioned.iter().enumerate() {
        let token = format!("@{i}");
        dump = dump.replace(id.as_str(), &token);
        section.push_str(&format!("annot {token} {body}\n"));
    }
    silent.sort();
    for body in silent {
        section.push_str(&format!("annot unmentioned {body}\n"));
    }
    // A body may name another annotation, a reply its thread for one.
    for (i, (_, id, _)) in mentioned.iter().enumerate() {
        section = section.replace(id.as_str(), &format!("@{i}"));
    }
    dump.push_str(&section);
    dump
}

/// One text body as its runs, each with its marks in key order.
fn render_delta(delta: &[TextDelta], names: &PlaceNames) -> String {
    let runs: Vec<String> = delta
        .iter()
        .filter_map(|span| match span {
            TextDelta::Insert { insert, attributes } => {
                let marks = attributes
                    .as_ref()
                    .map(|a| {
                        let sorted: BTreeMap<&str, &LoroValue> =
                            a.iter().map(|(k, v)| (k.as_str(), v)).collect();
                        sorted
                            .into_iter()
                            .map(|(k, v)| format!("{k}={}", render(v, names)))
                            .collect::<Vec<_>>()
                            .join(" ")
                    })
                    .unwrap_or_default();
                Some(format!("{insert:?}{{{marks}}}"))
            }
            _ => None,
        })
        .collect();
    runs.join(" ")
}

/// One decoded CRDT value as canonical text, map keys sorted.
fn render(value: &LoroValue, names: &PlaceNames) -> String {
    match value {
        LoroValue::Null => "null".to_owned(),
        LoroValue::Bool(b) => b.to_string(),
        LoroValue::Double(d) => format!("{d}"),
        LoroValue::I64(i) => i.to_string(),
        LoroValue::Binary(bytes) => format!("{} bytes", bytes.len()),
        LoroValue::String(s) => names.rename(s.as_str()),
        LoroValue::List(items) => {
            let rendered: Vec<String> = items.iter().map(|v| render(v, names)).collect();
            format!("[{}]", rendered.join(", "))
        }
        LoroValue::Map(map) => {
            let sorted: BTreeMap<&str, &LoroValue> =
                map.iter().map(|(k, v)| (k.as_str(), v)).collect();
            let rendered: Vec<String> = sorted
                .into_iter()
                .map(|(k, v)| match k {
                    // Minted per route: the reply is named by its place in
                    // the list, the hiding session by nothing. The walk runs
                    // over every map, so it compares the stored spellings.
                    _ if k == doc::ReplyKey::Id.as_str() || k == doc::NodeKey::Hidden.as_str() => {
                        format!("{k}=<masked>")
                    }
                    _ => format!("{k}={}", render(v, names)),
                })
                .collect();
            format!("{{{}}}", rendered.join(" "))
        }
        LoroValue::Container(id) => format!("<container {id:?}>"),
    }
}

// -----------------------------------------------------------------------------
// The correctness generators
// -----------------------------------------------------------------------------

/// Which document a generator builds.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ProseFixture {
    /// A heading, a paragraph with two marks and a two by two table. Then a
    /// header and a footer, a footnote, and a comment with a reply. One of
    /// each register a migration can touch.
    StyledDocument,
}

/// Prose's half of the shipping registry (`ambre-logic`'s `REGISTRY`).
pub const PROSE_FIXTURES: &[&dyn FixtureGen] = &[&ProseFixture::StyledDocument];

impl FixtureGen for ProseFixture {
    fn name(&self) -> FixtureName {
        FixtureName::declared(match self {
            Self::StyledDocument => "prose/styled_document",
        })
    }

    fn doc_type(&self) -> DocType {
        DocType::Prose
    }

    fn seed(&self) -> FixtureSeed {
        FixtureSeed::new(match self {
            Self::StyledDocument => 0x5EED_9905_0000_0001,
        })
    }

    fn generator_version(&self) -> Version<Generator> {
        Version::<Generator>::new(1)
    }

    fn build(&self) -> ambre_core::Payload<ambre_core::PlaintextContainer> {
        let seed = self.seed();
        let built = match self {
            Self::StyledDocument => build_styled_document(seed),
        };
        // An empty artifact fails the corpus gate by name, which is the
        // answer a refused command deserves here: no panic, and no plausible
        // half-built file.
        built
            .inspect_err(|e| tracing::error!("Prose fixture: {e}"))
            .ok()
            .and_then(|state| state.export_ambre_plaintext(seeded_document_id(seed)))
            .unwrap_or_else(|| {
                ambre_core::Payload::<ambre_core::PlaintextContainer>::new(Vec::new())
            })
    }
}

/// The author's identity, pinned apart from the peer: two routes to one
/// document share an author and differ in peer.
const AUTHOR_SEED: FixtureSeed = FixtureSeed::new(0x5EED_9905_A117_0001);

/// The comment's clock, pinned: a real one would differ run to run.
const COMMENT_AT: Instant<Authored> = Instant::<Authored>::new(1_700_000_000_000);

/// Why the fixture document was not built.
#[derive(Debug)]
enum FixtureError {
    /// The engine refused a command the fixture runs.
    Refused {
        command: crate::events::CommandName,
        reason: ProseRefusal,
    },
    /// A part the fixture writes into was not there. The sentence names it.
    Missing(&'static str),
    /// The author's name was refused.
    Name(ambre_core::InvalidDisplayName),
}

impl std::fmt::Display for FixtureError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Refused { command, reason } => write!(f, "{command} was refused: {reason:?}"),
            Self::Missing(what) => f.write_str(what),
            Self::Name(error) => write!(f, "{error}"),
        }
    }
}

/// Run one command, and name it if the engine refuses it.
fn run(state: &mut ProseState, command: ProseCommand) -> Result<(), FixtureError> {
    let name = command.name();
    let outcome = state.handle_command(command);
    match outcome.display.iter().find_map(|e| match e {
        ProseEvent::Error(e) => Some(e.reason.clone()),
        _ => None,
    }) {
        Some(reason) => Err(FixtureError::Refused {
            command: name,
            reason,
        }),
        None => Ok(()),
    }
}

/// The block that follows `after` at the root.
fn root_after(state: &ProseState, after: TreeID) -> Result<TreeID, FixtureError> {
    let roots = state.root_blocks();
    roots
        .iter()
        .position(|b| tree_id(*b) == after)
        .and_then(|i| roots.get(i + 1))
        .map(|b| tree_id(*b))
        .ok_or(FixtureError::Missing("no block after the one just written"))
}

fn build_styled_document(seed: FixtureSeed) -> Result<ProseState, FixtureError> {
    let mut state =
        ProseState::with_peer(seed.peer()).ok_or(FixtureError::Missing("no document"))?;
    state.set_user_id(Id::<User>::from_uuid(
        seeded_document_id(AUTHOR_SEED).uuid(),
    ));
    state.set_display_name(
        ambre_core::Validated::<ambre_core::DisplayName>::parse("Fixture Author")
            .map_err(FixtureError::Name)?,
    );

    let title = tree_id(
        *state
            .root_blocks()
            .first()
            .ok_or(FixtureError::Missing("no root block"))?,
    );
    run(
        &mut state,
        ProseCommand::InsertText {
            block_id: block_id(title),
            pos: Pos::<DocChar>::ZERO,
            text: "Quarterly report".to_owned(),
        },
    )?;
    run(
        &mut state,
        ProseCommand::SetHeadingLevel {
            block_id: block_id(title),
            level: Some(crate::events::HeadingLevel::H1),
        },
    )?;

    run(
        &mut state,
        ProseCommand::InsertBlock {
            parent_id: None,
            after_id: Some(block_id(title)),
            init: BlockInit::Paragraph,
            style_id: None,
        },
    )?;
    let body = root_after(&state, title)?;
    let text = "Revenue grew by twelve percent this quarter.";
    run(
        &mut state,
        ProseCommand::InsertText {
            block_id: block_id(body),
            pos: Pos::<DocChar>::ZERO,
            text: text.to_owned(),
        },
    )?;
    run(
        &mut state,
        ProseCommand::ApplyMark {
            block_id: block_id(body),
            range: Span::<DocChar>::from_usize_range(0..7),
            key: "b".into(),
            value: None,
        },
    )?;
    run(
        &mut state,
        ProseCommand::ApplyMark {
            block_id: block_id(body),
            range: Span::<DocChar>::from_usize_range(16..30),
            key: "i".into(),
            value: None,
        },
    )?;

    run(
        &mut state,
        ProseCommand::InsertTable {
            parent_id: None,
            after_id: Some(block_id(body)),
            rows: Len::<Row>::new(2),
            cols: Len::<Col>::new(2),
        },
    )?;
    let table = root_after(&state, body)?;
    let grid = super::tables::Grid::read(&doc::blocks(&state.doc), &table)
        .ok_or(FixtureError::Missing("no grid"))?;
    for (row, col, cell_text) in [
        (0, 0, "Region"),
        (0, 1, "Growth"),
        (1, 0, "North"),
        (1, 1, "12%"),
    ] {
        let cell = grid
            .cell(CellPos::new(Pos::<Row>::new(row), Pos::<Col>::new(col)))
            .ok_or(FixtureError::Missing("no cell"))?;
        let tree = doc::blocks(&state.doc);
        let paragraph = *doc::children_of(&tree, Some(cell))
            .first()
            .ok_or(FixtureError::Missing("an empty cell"))?;
        run(
            &mut state,
            ProseCommand::InsertText {
                block_id: block_id(paragraph),
                pos: Pos::<DocChar>::ZERO,
                text: cell_text.to_owned(),
            },
        )?;
    }

    run(
        &mut state,
        ProseCommand::SetHeaderEnabled {
            shown: crate::events::BandState::On,
        },
    )?;
    run(
        &mut state,
        ProseCommand::SetFooterEnabled {
            shown: crate::events::BandState::On,
        },
    )?;
    let header = state
        .bands()
        .header
        .ok_or(FixtureError::Missing("no header block"))?;
    run(
        &mut state,
        ProseCommand::InsertText {
            block_id: header,
            pos: Pos::<DocChar>::ZERO,
            text: "Confidential".to_owned(),
        },
    )?;
    run(
        &mut state,
        ProseCommand::InsertFooterField {
            field: HeaderField::PageNumber,
            target: HfTarget::Regular,
            pos: Pos::<DocChar>::ZERO,
        },
    )?;

    let end = Pos::<DocChar>::ZERO + ambre_core::char_len(text).as_axis::<DocChar>();
    run(
        &mut state,
        ProseCommand::InsertFootnote {
            block_id: block_id(body),
            pos: end,
            note_text: "Against the same quarter last year.".to_owned(),
        },
    )?;
    run(
        &mut state,
        ProseCommand::AddComment {
            block_id: block_id(body),
            range: Span::<DocChar>::from_usize_range(0..7),
            text: "Is this net or gross?".to_owned(),
            now: COMMENT_AT,
        },
    )?;
    let thread = state
        .comments()
        .first()
        .map(|t| t.comment_id)
        .ok_or(FixtureError::Missing("no thread"))?;
    run(
        &mut state,
        ProseCommand::ReplyToComment {
            comment_id: thread,
            text: "Net.".to_owned(),
            now: COMMENT_AT.plus(TimeSpan::<Ms>::new(60_000)),
        },
    )?;
    Ok(state)
}

#[cfg(test)]
mod tests {
    use ambre_bus::app::prose::DocChar;
    use ambre_core::FixtureGen;
    use ambre_core::index::{Len, Pos};

    use super::{ProseFixture, build_styled_document};
    use crate::events::ProseCommand;
    use crate::logic::ProseState;

    #[test]
    fn the_shape_counts_the_blocks_and_the_characters() {
        let mut state =
            ProseState::with_peer(ambre_core::PeerId::new(7)).expect("a document is built");
        let block = state.root_blocks()[0];
        state.handle_command(ProseCommand::InsertText {
            block_id: block,
            pos: Pos::<DocChar>::ZERO,
            text: "hello".to_owned(),
        });
        let shape = state.fixture_shape();
        assert_eq!(shape.peers, ambre_core::Count::<ambre_core::Peers>::new(1));
        assert_eq!(
            shape.body,
            ambre_core::ShapeBody::Prose {
                blocks: ambre_core::Count::<ambre_core::DocBlocks>::new(1),
                characters: Len::<ambre_core::Char>::new(5)
            }
        );
    }

    #[test]
    fn two_documents_holding_the_same_content_dump_alike() {
        let mut first =
            ProseState::with_peer(ambre_core::PeerId::new(1)).expect("a document is built");
        let mut second =
            ProseState::with_peer(ambre_core::PeerId::new(2)).expect("a document is built");
        for state in [&mut first, &mut second] {
            let block = state.root_blocks()[0];
            state.handle_command(ProseCommand::InsertText {
                block_id: block,
                pos: Pos::<DocChar>::ZERO,
                text: "one route".to_owned(),
            });
        }
        assert_eq!(first.canonical_dump(), second.canonical_dump());
        assert_ne!(first.export_snapshot(), second.export_snapshot());
    }

    /// Every command the generator sends lands: a refused one would leave a
    /// plausible, thinner document behind.
    #[test]
    fn the_styled_document_builds_every_register() {
        let state = build_styled_document(ProseFixture::StyledDocument.seed())
            .expect("every command lands");
        let dump = state.canonical_dump();
        for needle in [
            "Quarterly report",
            "b=\"\"",
            "i=\"\"",
            "\"Region\"",
            "\"Confidential\"",
            "Against the same quarter last year.",
            "Is this net or gross?",
            "Net.",
        ] {
            assert!(dump.contains(needle), "the dump misses {needle}:\n{dump}");
        }
    }

    /// `FIXTURE-002`: the generator pins its ids with its peer, so a note and
    /// a thread do not move the bytes between two builds.
    #[test]
    fn the_styled_document_is_byte_identical_twice() {
        let first = ProseFixture::StyledDocument.build();
        assert!(!first.is_empty());
        assert_eq!(first, ProseFixture::StyledDocument.build());
    }

    /// `FIXTURE-016`: the same content reached from two peers dumps alike,
    /// though every block and annotation id differs.
    #[test]
    fn the_dump_is_blind_to_the_ids_a_route_mints() {
        let first =
            build_styled_document(ambre_core::FixtureSeed::new(11)).expect("every command lands");
        let second =
            build_styled_document(ambre_core::FixtureSeed::new(12)).expect("every command lands");
        assert_eq!(first.canonical_dump(), second.canonical_dump());
        assert_ne!(first.export_snapshot(), second.export_snapshot());
    }
}
