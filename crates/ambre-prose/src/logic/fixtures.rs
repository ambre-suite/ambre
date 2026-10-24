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

use std::collections::{BTreeMap, HashMap};

use ambre_bus::app::grid::{CellPos, Col, ColCount, Row, RowCount};
use ambre_bus::app::prose::{DocCount, DocOffset, DocSpan};
use ambre_core::{
    DocType, FixtureGen, FixtureShape, ShapeBody, UserId, fixture_peer_count, seeded_document_id,
};
use loro::{LoroTree, LoroValue, TextDelta, TreeID};

use crate::doc;
use crate::events::{
    BlockInit, HeaderField, HfTarget, ProseCommand, ProseEvent, ProseRefusal, block_id, tree_id,
};

use super::ProseState;

impl ProseState {
    /// The coarse shape of this document, as a manifest records it.
    ///
    /// Counted from the CRDT rather than declared by a generator. A generator
    /// declaring its own shape would assert only that it agrees with itself.
    #[must_use]
    pub fn fixture_shape(&self) -> FixtureShape {
        let blocks = u32::try_from(self.document_order().len()).unwrap_or(u32::MAX);
        let characters = self
            .document_order()
            .iter()
            .filter_map(|id| self.body_of(id))
            .fold(0u32, |total, body| {
                total.saturating_add(u32::try_from(body.len_unicode()).unwrap_or(u32::MAX))
            });
        FixtureShape {
            // The loader's own pinned peer is not an author of the document.
            // `u64::MAX` is never a peer: without a scaffolding identity,
            // nothing is subtracted.
            peers: fixture_peer_count(
                self.doc.oplog_vv().keys().copied().map(ambre_core::PeerId),
                super::scaffold_identity().unwrap_or(ambre_core::PeerId(u64::MAX)),
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
        let names: HashMap<String, String> = order
            .iter()
            .enumerate()
            .map(|(i, id)| (id.to_string(), format!("#{i}")))
            .collect();

        let mut out = String::from("prose\n");
        for (i, id) in order.iter().enumerate() {
            let meta = doc::node_of(&tree, id)
                .map(|node| node.map().get_deep_value())
                .unwrap_or(LoroValue::Null);
            let parent = match tree.parent(*id) {
                Some(loro::TreeParentId::Node(parent)) => rename(&parent.to_string(), &names),
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
                .map(|(k, v)| (k.clone(), render(v, &names)))
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
                    && let Some(uuid) = ambre_core::FootnoteId::parse_str(note.as_str()).ok()
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

/// A block id by its place, or the string unchanged.
fn rename(value: &str, names: &HashMap<String, String>) -> String {
    names
        .get(value)
        .cloned()
        .unwrap_or_else(|| format!("{value:?}"))
}

/// Name every annotation by its first mention in `dump`, then list them.
///
/// An annotation id is a UUID, so a plain substring replacement cannot hit
/// anything but itself. One nobody mentions is listed last, by content.
fn rename_annotations(mut dump: String, annots: Vec<(String, String)>) -> String {
    let mut mentioned: Vec<(usize, String, String)> = Vec::new();
    let mut silent: Vec<String> = Vec::new();
    for (id, body) in annots {
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
fn render_delta(delta: &[TextDelta], names: &HashMap<String, String>) -> String {
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
fn render(value: &LoroValue, names: &HashMap<String, String>) -> String {
    match value {
        LoroValue::Null => "null".to_owned(),
        LoroValue::Bool(b) => b.to_string(),
        LoroValue::Double(d) => format!("{d}"),
        LoroValue::I64(i) => i.to_string(),
        LoroValue::Binary(bytes) => format!("{} bytes", bytes.len()),
        LoroValue::String(s) => rename(s.as_str(), names),
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
    fn name(&self) -> &'static str {
        match self {
            Self::StyledDocument => "prose/styled_document",
        }
    }

    fn doc_type(&self) -> DocType {
        DocType::Prose
    }

    fn seed(&self) -> u64 {
        match self {
            Self::StyledDocument => 0x5EED_9905_0000_0001,
        }
    }

    fn generator_version(&self) -> u32 {
        1
    }

    fn build(&self) -> ambre_core::PlaintextContainer {
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
            .unwrap_or_else(|| ambre_core::PlaintextContainer::new(Vec::new()))
    }
}

/// The author's identity, pinned apart from the peer: two routes to one
/// document share an author and differ in peer.
const AUTHOR_SEED: u64 = 0x5EED_9905_A117_0001;

/// The comment's clock, pinned: a real one would differ run to run.
const COMMENT_AT: ambre_core::Timestamp = ambre_core::Timestamp(1_700_000_000_000);

/// Why the fixture document was not built.
#[derive(Debug)]
enum FixtureError {
    /// The engine refused a command the fixture runs.
    Refused {
        command: &'static str,
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

fn build_styled_document(seed: u64) -> Result<ProseState, FixtureError> {
    let mut state = ProseState::with_peer(ambre_core::PeerId(seed))
        .ok_or(FixtureError::Missing("no document"))?;
    state.set_user_id(UserId::from_uuid(seeded_document_id(AUTHOR_SEED).into()));
    state.set_display_name(
        ambre_core::DisplayName::new("Fixture Author").map_err(FixtureError::Name)?,
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
            pos: DocOffset::ZERO,
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
            pos: DocOffset::ZERO,
            text: text.to_owned(),
        },
    )?;
    run(
        &mut state,
        ProseCommand::ApplyMark {
            block_id: block_id(body),
            range: DocSpan::from_usize_range(0..7),
            key: "b".into(),
            value: None,
        },
    )?;
    run(
        &mut state,
        ProseCommand::ApplyMark {
            block_id: block_id(body),
            range: DocSpan::from_usize_range(16..30),
            key: "i".into(),
            value: None,
        },
    )?;

    run(
        &mut state,
        ProseCommand::InsertTable {
            parent_id: None,
            after_id: Some(block_id(body)),
            rows: RowCount::new(2),
            cols: ColCount::new(2),
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
            .cell(CellPos::new(Row::new(row), Col::new(col)))
            .ok_or(FixtureError::Missing("no cell"))?;
        let tree = doc::blocks(&state.doc);
        let paragraph = *doc::children_of(&tree, Some(cell))
            .first()
            .ok_or(FixtureError::Missing("an empty cell"))?;
        run(
            &mut state,
            ProseCommand::InsertText {
                block_id: block_id(paragraph),
                pos: DocOffset::ZERO,
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
            pos: DocOffset::ZERO,
            text: "Confidential".to_owned(),
        },
    )?;
    run(
        &mut state,
        ProseCommand::InsertFooterField {
            field: HeaderField::PageNumber,
            target: HfTarget::Regular,
            pos: DocOffset::ZERO,
        },
    )?;

    let end = DocOffset::ZERO + DocCount::of(text);
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
            range: DocSpan::from_usize_range(0..7),
            text: "Is this net or gross?".to_owned(),
            now_ms: COMMENT_AT,
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
            now_ms: COMMENT_AT.plus(ambre_core::Millis(60_000)),
        },
    )?;
    Ok(state)
}

#[cfg(test)]
mod tests {
    use ambre_bus::app::prose::DocOffset;
    use ambre_core::FixtureGen;

    use super::{ProseFixture, build_styled_document};
    use crate::events::ProseCommand;
    use crate::logic::ProseState;

    #[test]
    fn the_shape_counts_the_blocks_and_the_characters() {
        let mut state = ProseState::with_peer(ambre_core::PeerId(7)).expect("a document is built");
        let block = state.root_blocks()[0];
        state.handle_command(ProseCommand::InsertText {
            block_id: block,
            pos: DocOffset::ZERO,
            text: "hello".to_owned(),
        });
        let shape = state.fixture_shape();
        assert_eq!(shape.peers, 1);
        assert_eq!(
            shape.body,
            ambre_core::ShapeBody::Prose {
                blocks: 1,
                characters: 5
            }
        );
    }

    #[test]
    fn two_documents_holding_the_same_content_dump_alike() {
        let mut first = ProseState::with_peer(ambre_core::PeerId(1)).expect("a document is built");
        let mut second = ProseState::with_peer(ambre_core::PeerId(2)).expect("a document is built");
        for state in [&mut first, &mut second] {
            let block = state.root_blocks()[0];
            state.handle_command(ProseCommand::InsertText {
                block_id: block,
                pos: DocOffset::ZERO,
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
        let first = build_styled_document(11).expect("every command lands");
        let second = build_styled_document(12).expect("every command lands");
        assert_eq!(first.canonical_dump(), second.canonical_dump());
        assert_ne!(first.export_snapshot(), second.export_snapshot());
    }
}
