//! The blocks an edit touched, whoever made it
//! (`features/ambre-spec-prose-pagination.md section 2`).
//!
//! The paginator re-measures only the blocks it is told about (`PROSE-083`).
//! A command knows the block it edits, but an import, an undo or a comment
//! anchor refresh does not say. So the document itself reports what changed.
//! A subscription notes the block behind every container a commit or an import
//! wrote to, and the next layout re-measures those (`PROSE-123`).

use std::collections::HashSet;
use std::sync::{Arc, Mutex, PoisonError};

use loro::TextDelta;
use loro::event::{ContainerDiff, Diff, DiffEvent};
use loro::{ContainerID, Index, LoroDoc, Subscription, TreeExternalDiff, TreeID, TreeParentId};

use crate::doc;

type Seen = Arc<Mutex<HashSet<TreeID>>>;
type Moved = Arc<Mutex<TextMoves>>;

/// What the text did since the last command answered: the runs each body
/// gained or lost, and the blocks deleted. The view shifts its draft and its
/// caret by it (`PROSE-UI-028`).
#[derive(Debug, Default)]
pub(crate) struct TextMoves {
    /// A block, where a run starts in the text before it, and its signed
    /// length in code points.
    pub(crate) edits: Vec<(TreeID, u32, i64)>,
    pub(crate) removed: Vec<TreeID>,
}

/// The blocks written to since the last layout took them.
pub(crate) struct Touched {
    seen: Seen,
    moved: Moved,
    /// Held so the subscription lives as long as the document state.
    _subscription: Subscription,
}

impl Touched {
    /// Start noting what `document` is written, from now on.
    pub(crate) fn watch(document: &LoroDoc) -> Self {
        let seen: Seen = Arc::default();
        let moved: Moved = Arc::default();
        let sink = Arc::clone(&seen);
        let moves = Arc::clone(&moved);
        let subscription = document.subscribe_root(Arc::new(move |event: DiffEvent<'_>| {
            let mut blocks = sink.lock().unwrap_or_else(PoisonError::into_inner);
            let mut text = moves.lock().unwrap_or_else(PoisonError::into_inner);
            for change in &event.events {
                note(change, &mut blocks);
                record(change, &mut text);
            }
        }));
        Self {
            seen,
            moved,
            _subscription: subscription,
        }
    }

    /// The blocks written to since the last call, each with its ancestors.
    ///
    /// A table is measured whole, so an edit in one of its cells re-measures
    /// the table: the ancestors carry the edit up to it.
    pub(crate) fn take(&self, document: &LoroDoc) -> Vec<TreeID> {
        let noted: Vec<TreeID> = {
            let mut blocks = self.seen.lock().unwrap_or_else(PoisonError::into_inner);
            blocks.drain().collect()
        };
        if noted.is_empty() {
            return noted;
        }
        let tree = doc::blocks(document);
        let mut out: HashSet<TreeID> = HashSet::with_capacity(noted.len());
        for id in noted {
            let mut current = Some(id);
            while let Some(block) = current {
                if !out.insert(block) {
                    break;
                }
                current = match tree.parent(block) {
                    Some(TreeParentId::Node(parent)) => Some(parent),
                    _ => None,
                };
            }
        }
        out.into_iter().collect()
    }
}

impl Touched {
    /// What the text did since the last call.
    pub(crate) fn take_moves(&self) -> TextMoves {
        std::mem::take(&mut *self.moved.lock().unwrap_or_else(PoisonError::into_inner))
    }
}

/// The runs a body text gained or lost, and the blocks a change deleted or
/// hid: a candidate the caller checks against the tree.
/// A position is counted in the text as it stood before the run, which is
/// how a delta walks it.
fn record(change: &ContainerDiff<'_>, moves: &mut TextMoves) {
    match &change.diff {
        Diff::Text(delta) => {
            let ContainerID::Root { name, .. } = change.target else {
                return;
            };
            let Some(block) = doc::parse_body_key(name) else {
                return;
            };
            let mut at: u32 = 0;
            for item in delta {
                match item {
                    TextDelta::Retain { retain, .. } => {
                        at = at.saturating_add(u32::try_from(*retain).unwrap_or(u32::MAX));
                    }
                    TextDelta::Insert { insert, .. } => {
                        let len = insert.chars().count();
                        moves
                            .edits
                            .push((block, at, i64::try_from(len).unwrap_or(i64::MAX)));
                    }
                    // An insertion is not in the text before, so the walk
                    // stays put. A deletion is, and the walk steps over it.
                    TextDelta::Delete { delete } => {
                        let len = u32::try_from(*delete).unwrap_or(u32::MAX);
                        moves.edits.push((block, at, -i64::from(len)));
                        at = at.saturating_add(len);
                    }
                }
            }
        }
        Diff::Tree(tree) => {
            for item in &tree.diff {
                if let TreeExternalDiff::Delete { .. } = item.action {
                    moves.removed.push(item.target);
                }
            }
        }
        // Deleting a block hides it: its node map gains the marker.
        Diff::Map(map) if map.updated.contains_key(doc::K_HIDDEN) => {
            let owner = change.path.iter().rev().find_map(|(_, index)| match index {
                Index::Node(id) => Some(*id),
                _ => None,
            });
            moves.removed.extend(owner);
        }
        _ => {}
    }
}

/// The block one container change belongs to. A body text names it in its
/// key, and a node's own map sits under it in the path. A tree change names
/// the block it moved, with both its parents.
fn note(change: &ContainerDiff<'_>, blocks: &mut HashSet<TreeID>) {
    if let ContainerID::Root { name, .. } = change.target
        && let Some(id) = doc::parse_body_key(name)
    {
        blocks.insert(id);
        return;
    }
    if let Diff::Tree(tree) = &change.diff {
        for item in &tree.diff {
            blocks.insert(item.target);
            let parents = match &item.action {
                TreeExternalDiff::Create { parent, .. } => [Some(*parent), None],
                TreeExternalDiff::Move {
                    parent, old_parent, ..
                } => [Some(*parent), Some(*old_parent)],
                TreeExternalDiff::Delete { old_parent, .. } => [Some(*old_parent), None],
            };
            for parent in parents.into_iter().flatten() {
                if let TreeParentId::Node(parent) = parent {
                    blocks.insert(parent);
                }
            }
        }
        return;
    }
    let owner = change.path.iter().rev().find_map(|(_, index)| match index {
        Index::Node(id) => Some(*id),
        _ => None,
    });
    if let Some(id) = owner {
        blocks.insert(id);
    }
}
