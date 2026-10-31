//! The two cross-reference commands
//! (`features/ambre-spec-prose-cross-references.md`).
//!
//! A reference is written as one carrier character in the block that holds
//! it. Nothing is written at the target (`PROSE-FEAT-135`), and following a
//! reference writes nothing at all (`PROSE-UI-086`).

use loro::TreeID;

use ambre_bus::app::prose::DocOffset;

use crate::doc;
use crate::events::{CrossRefKind, ProseEvent, ProseOutcome, ProseRefusal, block_id};
use crate::xref;

use super::ProseState;

impl ProseState {
    /// A reference to `target` at `pos` in `block` (`PROSE-FEAT-134`).
    pub(super) fn insert_cross_ref(
        &mut self,
        block: &TreeID,
        pos: DocOffset,
        target: &TreeID,
        kind: &CrossRefKind,
    ) -> ProseOutcome {
        if doc::node_of(&doc::blocks(&self.doc), target).is_none() {
            return Self::err_no_block(target);
        }
        let Some(body) = self.body_of(block) else {
            return Self::err_no_block(block);
        };
        let at = pos;
        if at.to_usize() > body.len_unicode() {
            return Self::err_bad_range();
        }
        let value = xref::encode(*target, kind);
        let anchor = super::notes::OwnedAnchor::CrossRef(&value);
        if let Err(e) = Self::write_carrier(&body, at, &anchor) {
            return Self::err_write(e);
        }
        let mut outcome = self.commit_with(Vec::new());
        outcome.display.push(self.cross_refs_event());
        outcome
    }

    /// Bring the target of the reference at `pos` in `block` into view
    /// (`PROSE-UI-085`). A broken reference has nowhere to go and says so.
    pub(super) fn follow_cross_ref(&mut self, block: &TreeID, pos: DocOffset) -> ProseOutcome {
        let found = doc::anchors(&self.doc, crate::mark::MarkRoot::CrossRef, xref::decode)
            .into_iter()
            .find(|anchor| anchor.block == *block && anchor.offset == pos)
            .map(|anchor| anchor.value);
        let Some((target, _)) = found else {
            return ProseOutcome::refused(ProseRefusal::NoCrossRef);
        };
        if doc::node_of(&doc::blocks(&self.doc), &target).is_none() {
            return Self::err_no_block(&target);
        }
        let page = self
            .paginate(&[])
            .blocks
            .iter()
            .find(|placed| placed.block_id == target)
            .map_or(crate::events::PageIndex::ZERO, |placed| placed.page_start);
        ProseOutcome::display_only(vec![ProseEvent::RevealBlock {
            block: block_id(target),
            page,
        }])
    }

    /// Every reference, resolved against the layout on hand.
    pub(super) fn cross_refs_event(&self) -> ProseEvent {
        let layout = self.paginator.as_ref().map(|paginator| paginator.layout());
        ProseEvent::CrossRefsUpdated {
            refs: xref::resolve_all(&self.doc, layout),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::HeadingLevel;
    use crate::events::{BlockId, BlockInit, ProseCommand, ResolvedCrossRef, tree_id};
    use ambre_bus::app::prose::DocSpan;

    /// A heading "Intro", a paragraph, and a second heading "Details".
    fn document() -> (ProseState, Vec<BlockId>) {
        let mut state = ProseState::with_peer(ambre_core::PeerId::new(1)).expect("a document");
        state.load_faces(&crate::layout::fonts::shipped_faces());
        let mut ids = state.root_blocks();
        for init in [
            BlockInit::Paragraph,
            BlockInit::Heading {
                level: HeadingLevel::H1,
            },
        ] {
            state.handle_command(ProseCommand::InsertBlock {
                parent_id: None,
                after_id: ids.last().copied(),
                init,
                style_id: None,
            });
            ids = state.root_blocks();
        }
        state.handle_command(ProseCommand::SetHeadingLevel {
            block_id: ids[0],
            level: Some(HeadingLevel::H1),
        });
        for (id, text) in ids.iter().zip(["Intro", "See ", "Details"]) {
            state.handle_command(ProseCommand::InsertText {
                block_id: *id,
                pos: DocOffset::ZERO,
                text: text.to_owned(),
            });
        }
        (state, ids)
    }

    fn refer(state: &mut ProseState, at: BlockId, target: BlockId, kind: CrossRefKind) {
        state.handle_command(ProseCommand::InsertCrossRef {
            block_id: at,
            pos: DocOffset::new(4),
            target_block_id: target,
            kind,
        });
    }

    fn shown(state: &mut ProseState) -> Vec<Option<String>> {
        let _ = state.paginate(&[]);
        match state.cross_refs_event() {
            ProseEvent::CrossRefsUpdated { refs } => refs.into_iter().map(|r| r.shown).collect(),
            _ => Vec::new(),
        }
    }

    #[test]
    fn a_reference_names_its_target_by_identity_and_writes_nothing_there() {
        let (mut state, ids) = document();
        let heading_text = state.text_of(&ids[2]);
        let heading_marks = doc::get_block_text(state.document(), &tree_id(ids[2])).to_delta();
        refer(&mut state, ids[1], ids[2], CrossRefKind::HeadingNumber);
        let anchors = doc::anchors(state.document(), crate::mark::MarkRoot::CrossRef, |value| {
            Some(value.to_owned())
        });
        assert_eq!(anchors.len(), 1);
        let (named, _) = xref::decode(&anchors[0].value).expect("a record");
        assert_eq!(named, tree_id(ids[2]));
        assert_eq!(state.text_of(&ids[2]), heading_text);
        assert_eq!(
            doc::get_block_text(state.document(), &tree_id(ids[2])).to_delta(),
            heading_marks
        );
        state.handle_command(ProseCommand::InsertText {
            block_id: ids[2],
            pos: DocOffset::ZERO,
            text: "More ".to_owned(),
        });
        assert_eq!(shown(&mut state), vec![Some("2".to_owned())]);
    }

    #[test]
    fn a_reference_shows_the_heading_number_and_the_page_of_its_target() {
        let (mut state, ids) = document();
        refer(&mut state, ids[1], ids[2], CrossRefKind::HeadingNumber);
        refer(&mut state, ids[1], ids[0], CrossRefKind::PageNumber);
        assert_eq!(
            shown(&mut state),
            vec![Some("1".to_owned()), Some("2".to_owned())],
            "the page reference sits first in the text, the section second"
        );
        state.handle_command(ProseCommand::MoveBlock {
            block_id: ids[0],
            new_parent: None,
            after_id: Some(ids[2]),
        });
        assert_eq!(
            shown(&mut state),
            vec![Some("1".to_owned()), Some("1".to_owned())],
            "the moved heading keeps the reference and takes its new number"
        );
    }

    #[test]
    fn a_page_reference_follows_its_target_to_a_later_page() {
        let (mut state, ids) = document();
        refer(&mut state, ids[1], ids[2], CrossRefKind::PageNumber);
        assert_eq!(shown(&mut state), vec![Some("1".to_owned())]);
        state.handle_command(ProseCommand::InsertBlock {
            parent_id: None,
            after_id: Some(ids[1]),
            init: BlockInit::PageBreak,
            style_id: None,
        });
        let stored = state.document().get_deep_value();
        assert_eq!(shown(&mut state), vec![Some("2".to_owned())]);
        assert_eq!(
            state.document().get_deep_value(),
            stored,
            "nothing written back"
        );
    }

    #[test]
    fn a_target_an_undo_restores_resolves_again_on_the_other_peer() {
        let (mut alice, ids) = document();
        refer(&mut alice, ids[1], ids[2], CrossRefKind::HeadingNumber);
        let mut bob = ProseState::with_peer(ambre_core::PeerId::new(2)).expect("a document");
        bob.load_faces(&crate::layout::fonts::shipped_faces());
        bob.import_snapshot(&alice.export_snapshot())
            .expect("the seed imports");
        assert_eq!(shown(&mut bob), vec![Some("2".to_owned())]);

        let deleted = alice
            .handle_command(ProseCommand::DeleteBlock { block_id: ids[2] })
            .update
            .expect("a delta");
        bob.apply_remote(&deleted);
        assert_eq!(shown(&mut bob), vec![None], "the target is gone");

        let undone = alice
            .handle_command(ProseCommand::Undo)
            .update
            .expect("a delta");
        bob.apply_remote(&undone);
        assert_eq!(
            shown(&mut bob),
            vec![Some("2".to_owned())],
            "the reference resolves again"
        );
        assert_eq!(bob.text_of(&ids[2]).as_deref(), Some("Details"));
        assert_eq!(bob.root_blocks(), alice.root_blocks());
    }

    #[test]
    fn a_gone_target_breaks_the_reference_and_the_reference_stays() {
        let (mut state, ids) = document();
        refer(&mut state, ids[1], ids[2], CrossRefKind::HeadingNumber);
        refer(
            &mut state,
            ids[1],
            ids[2],
            CrossRefKind::CustomText("see: below".to_owned()),
        );
        state.handle_command(ProseCommand::DeleteBlock { block_id: ids[2] });
        assert_eq!(
            shown(&mut state),
            vec![Some("see: below".to_owned()), None],
            "the label survives, the number breaks and resolves to no neighbour"
        );
        assert_eq!(
            doc::anchors(
                state.document(),
                crate::mark::MarkRoot::CrossRef,
                xref::decode
            )
            .len(),
            2,
            "both references stay in the document"
        );
    }

    #[test]
    fn a_reference_is_one_character_to_the_caret() {
        let (mut state, ids) = document();
        refer(
            &mut state,
            ids[1],
            ids[2],
            CrossRefKind::CustomText("a long label".to_owned()),
        );
        assert_eq!(state.text_of(&ids[1]).as_deref(), Some("See \u{fffc}"));
        state.handle_command(ProseCommand::DeleteText {
            block_id: ids[1],
            range: DocSpan::from_usize_range(4..5),
        });
        assert!(
            doc::anchors(
                state.document(),
                crate::mark::MarkRoot::CrossRef,
                xref::decode
            )
            .is_empty()
        );
    }

    #[test]
    fn following_a_reference_reveals_its_target_and_writes_nothing() {
        let (mut state, ids) = document();
        refer(&mut state, ids[1], ids[2], CrossRefKind::PageNumber);
        let before = state.document().oplog_vv();
        let outcome = state.handle_command(ProseCommand::FollowCrossRef {
            block_id: ids[1],
            pos: DocOffset::new(4),
        });
        assert_eq!(
            outcome.display,
            vec![ProseEvent::RevealBlock {
                block: ids[2],
                page: crate::events::PageIndex::ZERO,
            }]
        );
        assert!(outcome.update.is_none());
        assert_eq!(state.document().oplog_vv(), before);
    }

    #[test]
    fn the_resolved_list_names_where_each_reference_sits() {
        let (mut state, ids) = document();
        refer(&mut state, ids[1], ids[2], CrossRefKind::HeadingNumber);
        let _ = state.paginate(&[]);
        assert_eq!(
            state.cross_refs_event(),
            ProseEvent::CrossRefsUpdated {
                refs: vec![ResolvedCrossRef {
                    block_id: ids[1],
                    offset: DocOffset::new(4),
                    target: ids[2],
                    shown: Some("2".to_owned()),
                }]
            }
        );
    }
}
