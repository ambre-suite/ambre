//! Where the caret lands after an undo or a redo
//! (`features/ambre-spec-prose-undo.md section 2`).
//!
//! The edit point is read from the change the step committed, never stored:
//! the caret is this author's view state (`PROSE-UI-074`). The engine reads it
//! because the view holds no document to read it from (`PROSE-UI-001`).

use ambre_bus::app::prose::DocChar;
use ambre_core::index::Pos;
use loro::event::Diff;
use loro::{ContainerID, Frontiers, LoroDoc, TextDelta, TreeExternalDiff};

use crate::doc;
use crate::events::{BlockId, block_id};

/// The block and the code-point offset an undo or a redo just changed, from
/// the diff between `before` and the current state (`PROSE-UI-045`).
///
/// A text edit wins over a structural one. The caret goes to the end of what
/// was put back, or to where text was taken out. A step that only created a
/// block puts the caret at that block's start. `None` when the step touched no
/// live block, which leaves the caret where it was.
pub(super) fn edit_point(
    document: &LoroDoc,
    before: &Frontiers,
) -> Option<(BlockId, Pos<DocChar>)> {
    let after = document.state_frontiers();
    let diff = document.diff(before, &after).ok()?;
    let tree = doc::blocks(document);
    let mut created = None;
    for (container, change) in diff.iter() {
        match change {
            Diff::Text(deltas) => {
                let ContainerID::Root { name, .. } = container else {
                    continue;
                };
                let Some(id) = doc::parse_body_key(name) else {
                    continue;
                };
                if doc::node_of(&tree, &id).is_none() {
                    continue;
                }
                if let Some(offset) = text_point(deltas) {
                    return Some((block_id(id), offset));
                }
            }
            Diff::Tree(tree_diff) if created.is_none() => {
                created = tree_diff.diff.iter().find_map(|item| match item.action {
                    TreeExternalDiff::Create { .. } | TreeExternalDiff::Move { .. } => {
                        doc::node_of(&tree, &item.target).map(|_| item.target)
                    }
                    TreeExternalDiff::Delete { .. } => None,
                });
            }
            _ => {}
        }
    }
    created.map(|id| (block_id(id), Pos::<DocChar>::ZERO))
}

/// The offset one text diff leaves the caret at. That is after the first
/// insertion, at the first deletion, or at the start of the first restyled run.
fn text_point(deltas: &[TextDelta]) -> Option<Pos<DocChar>> {
    let mut at: usize = 0;
    for delta in deltas {
        match delta {
            TextDelta::Retain { retain, attributes } => {
                if attributes.is_some() {
                    return Pos::<DocChar>::from_index(at);
                }
                at = at.saturating_add(*retain);
            }
            TextDelta::Insert { insert, .. } => {
                return Pos::<DocChar>::from_index(at.saturating_add(insert.chars().count()));
            }
            TextDelta::Delete { .. } => return Pos::<DocChar>::from_index(at),
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use crate::events::{CursorPosition, ProseCommand, ProseEvent};
    use crate::logic::ProseState;
    use crate::ui::ProseUiState;
    use ambre_bus::app::prose::DocChar;
    use ambre_core::index::{Pos, Span};

    fn placed(display: &[ProseEvent]) -> Option<(crate::events::BlockId, Pos<DocChar>)> {
        display.iter().find_map(|event| match event {
            ProseEvent::CaretPlaced { block, offset } => Some((*block, *offset)),
            _ => None,
        })
    }

    fn typed() -> (ProseState, crate::events::BlockId) {
        let mut state = ProseState::with_peer(ambre_core::PeerId::new(1)).expect("a document");
        let block = *state.root_blocks().first().expect("a root block");
        state.handle_command(ProseCommand::InsertText {
            block_id: block,
            pos: Pos::<DocChar>::ZERO,
            text: "h\u{e9}llo".to_owned(),
        });
        (state, block)
    }

    #[test]
    fn an_undo_puts_the_caret_where_the_text_was_taken_out() {
        let (mut state, block) = typed();
        state.handle_command(ProseCommand::InsertText {
            block_id: block,
            pos: Pos::<DocChar>::new(2),
            text: "XY".to_owned(),
        });
        let undone = state.handle_command(ProseCommand::Undo);
        assert_eq!(
            placed(&undone.display),
            Some((block, Pos::<DocChar>::new(2)))
        );
        let redone = state.handle_command(ProseCommand::Redo);
        assert_eq!(
            placed(&redone.display),
            Some((block, Pos::<DocChar>::new(4)))
        );
    }

    #[test]
    fn an_undone_deletion_puts_the_caret_after_the_restored_text() {
        let (mut state, block) = typed();
        state.handle_command(ProseCommand::DeleteText {
            block_id: block,
            range: Span::<DocChar>::from_usize_range(1..3),
        });
        let undone = state.handle_command(ProseCommand::Undo);
        assert_eq!(
            placed(&undone.display),
            Some((block, Pos::<DocChar>::new(3)))
        );
    }

    #[test]
    fn an_open_selection_collapses_to_the_placed_caret() {
        let (mut state, block) = typed();
        state.handle_command(ProseCommand::InsertText {
            block_id: block,
            pos: Pos::<DocChar>::new(5),
            text: "!".to_owned(),
        });
        let mut ui = ProseUiState::default();
        ui.cursor = Some(CursorPosition {
            block_id: block,
            offset: Pos::<DocChar>::ZERO,
            anchor: Some((block, Pos::<DocChar>::new(4))),
        });
        for event in &state.handle_command(ProseCommand::Undo).display {
            ui.apply_event(event);
        }
        assert_eq!(
            ui.cursor,
            Some(CursorPosition {
                block_id: block,
                offset: Pos::<DocChar>::new(5),
                anchor: None,
            })
        );
    }

    #[test]
    fn placing_the_caret_writes_nothing_beyond_the_inverse() {
        let (mut state, block) = typed();
        state.handle_command(ProseCommand::InsertText {
            block_id: block,
            pos: Pos::<DocChar>::ZERO,
            text: "a".to_owned(),
        });
        let peer = loro::LoroDoc::new();
        peer.import(state.export_snapshot().as_bytes())
            .expect("the peer opens the document");
        let undone = state.handle_command(ProseCommand::Undo);
        assert!(placed(&undone.display).is_some());
        peer.import(undone.update.expect("the inverse travels").as_bytes())
            .expect("the peer takes the inverse");
        assert_eq!(
            peer.get_deep_value(),
            state.document().get_deep_value(),
            "the peer holds what this author holds, and no caret"
        );
    }
}
