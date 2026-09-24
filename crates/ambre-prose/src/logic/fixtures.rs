//! Prose's half of the fixture loader surface (`ambre-spec-fixtures.md`).
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
//!
//! **No generator yet.** The corpus carries no Prose artifact: no `.ambre`
//! container in it holds a Prose document yet. The generator lands with the "A
//! Prose fixture corpus" Story (`ambre-roadmap.md` E5), and this module answers
//! the pair its loader needs.

use ambre_core::{FixtureShape, ShapeBody, fixture_peer_count};

use crate::doc;

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
            peers: fixture_peer_count(self.doc.oplog_vv().keys().copied()),
            body: ShapeBody::Prose { blocks, characters },
        }
    }

    /// The canonical semantic form: every user-visible value, in document
    /// order, as text.
    ///
    /// **Not** the CRDT. Two documents agreeing on content still differ in
    /// operation history, peer ids and commit timestamps. A byte comparison
    /// cannot express "same document by a different route".
    #[must_use]
    pub fn canonical_dump(&self) -> String {
        let tree = doc::blocks(&self.doc);
        let mut out = String::new();
        for id in self.document_order() {
            let block_type = doc::block_type_of(&tree, &id);
            let style = doc::style_of(&tree, &id).unwrap_or_default();
            let text = self
                .text_of(&crate::events::block_id(id))
                .unwrap_or_default();
            out.push_str(&format!("{block_type:?}\t{style}\t{text}\n"));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use crate::events::ProseCommand;
    use crate::logic::ProseState;

    #[test]
    fn the_shape_counts_the_blocks_and_the_characters() {
        let mut state = ProseState::with_peer(7).expect("a document is built");
        let block = state.root_blocks()[0];
        state.handle_command(ProseCommand::InsertText {
            block_id: block,
            pos: 0,
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
        let mut first = ProseState::with_peer(1).expect("a document is built");
        let mut second = ProseState::with_peer(2).expect("a document is built");
        for state in [&mut first, &mut second] {
            let block = state.root_blocks()[0];
            state.handle_command(ProseCommand::InsertText {
                block_id: block,
                pos: 0,
                text: "one route".to_owned(),
            });
        }
        assert_eq!(first.canonical_dump(), second.canonical_dump());
        assert_ne!(first.export_snapshot(), second.export_snapshot());
    }
}
