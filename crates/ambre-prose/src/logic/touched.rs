//! The blocks an edit touched, whoever made it
//! (`features/ambre-spec-prose-pagination.md section 2`).
//!
//! The paginator re-measures only the blocks it is told about (`PROSE-083`).
//! A command knows the block it edits, but an import, an undo or a comment
//! anchor refresh does not say. So the document itself reports what changed.
//! A subscription notes the block behind every container a commit or an import
//! wrote to, and the next layout re-measures those (`PROSE-123`).

use ambre_core::index::{Delta, Len, Pos};
use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use crate::message::DocChar;
use loro::TextDelta;
use loro::event::{ContainerDiff, Diff, DiffEvent};
use loro::{ContainerID, Index, LoroDoc, Subscription, TreeExternalDiff, TreeID, TreeParentId};

use crate::doc;
use ambre_core::crdt::CrdtName as _;

type Seen = Arc<Mutex<HashSet<TreeID>>>;
type Moved = Arc<Mutex<TextMoves>>;

/// What the text did since the last command answered: the runs each body
/// gained or lost, and the blocks deleted. The view shifts its draft and its
/// caret by it (`PROSE-UI-028`).
#[derive(Debug, Default)]
pub(crate) struct TextMoves {
    /// A block, where a run starts in the text the runs before it left, and
    /// its signed length in code points.
    pub(crate) edits: Vec<(TreeID, Pos<DocChar>, Delta<DocChar>)>,
    pub(crate) removed: Vec<TreeID>,
}

/// The blocks written to since the last layout took them.
pub(crate) struct Touched {
    seen: Seen,
    moved: Moved,
    /// Set by any change beyond text typed or erased in a body. A tree move, a
    /// map write, a mark set or cleared and a footnote call inserted all set
    /// it. It starts set, since nothing is known of what came before.
    structural: Arc<AtomicBool>,
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
        let structural = Arc::new(AtomicBool::new(true));
        let flag = Arc::clone(&structural);
        let subscription = document.subscribe_root(Arc::new(move |event: DiffEvent<'_>| {
            let mut blocks = sink.lock().unwrap_or_else(PoisonError::into_inner);
            let mut text = moves.lock().unwrap_or_else(PoisonError::into_inner);
            for change in &event.events {
                note(change, &mut blocks);
                record(change, &mut text);
                if !plain_text(change) {
                    flag.store(true, Ordering::Relaxed);
                }
            }
        }));
        Self {
            seen,
            moved,
            structural,
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

    /// Whether anything but plain body text changed since the last call.
    pub(crate) fn take_structural(&self) -> bool {
        self.structural.swap(false, Ordering::Relaxed)
    }

    /// Whether every change since the last layout only typed body text:
    /// nothing structural, and no character erased. Such a change cannot
    /// leave a block empty, so the tree needs no repair. It reads the state
    /// and leaves it for the next layout.
    pub(crate) fn only_typed(&self) -> bool {
        if self.structural.load(Ordering::Relaxed) {
            return false;
        }
        let moved = self.moved.lock().unwrap_or_else(PoisonError::into_inner);
        moved.removed.is_empty()
            && moved
                .edits
                .iter()
                .all(|&(_, _, delta)| delta > Delta::<DocChar>::ZERO)
    }
}

impl Touched {
    /// The blocks whose body lost characters since the last layout, when every
    /// change since then typed or erased body text and nothing else. `None`
    /// when anything structural changed, or a block was deleted.
    pub(crate) fn erased(&self) -> Option<Vec<TreeID>> {
        if self.structural.load(Ordering::Relaxed) {
            return None;
        }
        let moved = self.moved.lock().unwrap_or_else(PoisonError::into_inner);
        moved.removed.is_empty().then(|| {
            moved
                .edits
                .iter()
                .filter(|&&(_, _, delta)| delta < Delta::<DocChar>::ZERO)
                .map(|&(block, _, _)| block)
                .collect()
        })
    }
}

/// Whether one container change is text typed or erased in a body, and
/// nothing else: no mark set or cleared, and no footnote call inserted.
///
/// Such a change leaves the tree, the settings and the note numbering as they
/// were. It may still have written into a band or a note, which the paginator
/// checks against the blocks it names.
fn plain_text(change: &ContainerDiff<'_>) -> bool {
    let Diff::Text(delta) = &change.diff else {
        return false;
    };
    let ContainerID::Root { name, .. } = change.target else {
        return false;
    };
    if doc::parse_body_key(name).is_none() {
        return false;
    }
    delta.iter().all(|item| match item {
        TextDelta::Retain { attributes, .. } => attributes.as_ref().is_none_or(|a| a.is_empty()),
        TextDelta::Insert { attributes, .. } => attributes
            .as_ref()
            .is_none_or(|a| !a.contains_key(crate::mark::MarkRoot::Footnote.as_str())),
        TextDelta::Delete { .. } => true,
    })
}

/// The runs a body text gained or lost, and the blocks a change deleted or
/// hid: a candidate the caller checks against the tree.
fn record(change: &ContainerDiff<'_>, moves: &mut TextMoves) {
    match &change.diff {
        Diff::Text(delta) => {
            let ContainerID::Root { name, .. } = change.target else {
                return;
            };
            let Some(block) = doc::parse_body_key(name) else {
                return;
            };
            // The walk counts positions in the text as the runs before
            // leave it, so each run reads after the previous one.
            let mut at = Pos::<DocChar>::ZERO;
            for item in delta {
                match item {
                    TextDelta::Retain { retain, .. } => {
                        at += Len::<DocChar>::of(*retain);
                    }
                    TextDelta::Insert { insert, .. } => {
                        let len = ambre_core::char_len(insert).as_axis::<DocChar>();
                        moves
                            .edits
                            .push((block, at, Delta::<DocChar>::from_len(len)));
                        at += len;
                    }
                    TextDelta::Delete { delete } => {
                        let len = Len::<DocChar>::of(*delete);
                        moves
                            .edits
                            .push((block, at, Delta::<DocChar>::from_len(len).reversed()));
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
        Diff::Map(map) if map.updated.contains_key(doc::NodeKey::Hidden.as_str()) => {
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
