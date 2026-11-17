//! The tree half of a step: the nodes a commit created and the nodes it
//! moved (`ambre-spec-undo.md section 3`).
//!
//! Loro refuses to raise a deleted node, and it mints a new identity for each
//! node it creates. So a replay never deletes a node and never creates one.
//! An undo of a creation hides the node under the marker the application
//! names, and its redo takes the marker off the same node. A move goes back
//! after the sibling that stood before the node.

use ambre_core::hash::HashMap;

use ambre_core::index::Pos;
use loro::{LoroDoc, LoroTree, LoroValue, TreeExternalDiff, TreeID, TreeParentId};

use super::text::Direction;
use super::{Hide, Slot};

/// The plain copy of one tree: the children of each parent, in order.
#[derive(Debug, Default)]
pub(super) struct Forest {
    children: HashMap<TreeParentId, Vec<TreeID>>,
}

/// Where a node stands: its parent and the sibling just before it.
#[derive(Debug, Clone)]
pub(super) struct Spot {
    parent: TreeParentId,
    pred: Option<TreeID>,
    at: Pos<Slot>,
}

/// One node a commit moved, with its spot on each side of the commit.
#[derive(Debug, Clone)]
pub(super) struct NodeMove {
    node: TreeID,
    from: Spot,
    to: Spot,
}

/// What one commit did to one tree.
#[derive(Debug, Clone, Default)]
pub(super) struct TreeWrite {
    pub(super) tree: Option<loro::ContainerID>,
    /// The nodes the commit created, each with the marker value a replay
    /// hid it under, `None` while it shows.
    pub(super) born: Vec<(TreeID, Option<LoroValue>)>,
    pub(super) moved: Vec<NodeMove>,
}

ambre_core::define_flag! {
    /// Whether a tree change is one a step replays.
    pub(super) enum Kept {
        /// Every change is a creation or a move.
        Whole,
        /// A node was deleted: Loro cannot raise it again.
        Lost,
    }
}

impl Forest {
    /// The copy of what `tree` holds now.
    pub(super) fn read(tree: &LoroTree) -> Self {
        let mut forest = Self::default();
        let mut parents = vec![TreeParentId::Root];
        while let Some(parent) = parents.pop() {
            let kids = tree.children(parent).unwrap_or_default();
            parents.extend(kids.iter().map(|id| TreeParentId::Node(*id)));
            forest.children.insert(parent, kids);
        }
        forest
    }

    /// Every node of the copy.
    pub(super) fn nodes(&self) -> impl Iterator<Item = &TreeID> {
        self.children.values().flatten()
    }

    /// The node at `at` under `parent`, before which a node stands.
    fn pred(&self, parent: TreeParentId, at: Pos<Slot>) -> Option<TreeID> {
        let before = at.prev()?;
        self.children.get(&parent)?.get(before.to_usize()).copied()
    }

    /// Take `node` out from under `parent`, where it stands at `at`.
    fn take_out(&mut self, node: TreeID, parent: TreeParentId, at: Pos<Slot>) {
        if let Some(kids) = self.children.get_mut(&parent) {
            match kids.get(at.to_usize()) {
                Some(id) if *id == node => {
                    kids.remove(at.to_usize());
                }
                _ => kids.retain(|id| *id != node),
            }
        }
    }

    /// Put `node` under `parent` at `at`.
    fn put_in(&mut self, node: TreeID, parent: TreeParentId, at: Pos<Slot>) {
        let kids = self.children.entry(parent).or_default();
        kids.insert(at.to_usize().min(kids.len()), node);
    }

    /// Apply one tree event to the copy, and return what it did. The nodes
    /// it deleted come back in `gone`, so the mirror drops what they held.
    pub(super) fn take(
        &mut self,
        diff: &loro::TreeDiff,
        gone: &mut Vec<TreeID>,
    ) -> (TreeWrite, Kept) {
        let mut write = TreeWrite::default();
        let mut kept = Kept::Whole;
        for item in &diff.diff {
            match &item.action {
                TreeExternalDiff::Create { parent, index, .. } => {
                    self.put_in(item.target, *parent, Pos::from_usize(*index));
                    write.born.push((item.target, None));
                }
                TreeExternalDiff::Move {
                    parent,
                    index,
                    old_parent,
                    old_index,
                    ..
                } => {
                    let from_at = Pos::from_usize(*old_index);
                    let from = Spot {
                        parent: *old_parent,
                        pred: self.pred(*old_parent, from_at),
                        at: from_at,
                    };
                    self.take_out(item.target, *old_parent, from_at);
                    let to_at = Pos::from_usize(*index);
                    self.put_in(item.target, *parent, to_at);
                    let to = Spot {
                        parent: *parent,
                        pred: self.pred(*parent, to_at),
                        at: to_at,
                    };
                    write.moved.push(NodeMove {
                        node: item.target,
                        from,
                        to,
                    });
                }
                TreeExternalDiff::Delete {
                    old_parent,
                    old_index,
                } => {
                    self.take_out(item.target, *old_parent, Pos::from_usize(*old_index));
                    self.drop_below(item.target, gone);
                    gone.push(item.target);
                    kept = Kept::Lost;
                }
            }
        }
        (write, kept)
    }

    /// Forget the children of `node`, which went with it.
    fn drop_below(&mut self, node: TreeID, gone: &mut Vec<TreeID>) {
        let mut stack = vec![node];
        while let Some(id) = stack.pop() {
            if let Some(kids) = self.children.remove(&TreeParentId::Node(id)) {
                gone.extend(kids.iter().copied());
                stack.extend(kids);
            }
        }
    }
}

/// Whether `node` stands in `tree`: created and not deleted.
fn stands(tree: &LoroTree, node: TreeID) -> bool {
    matches!(tree.is_node_deleted(&node), Ok(false))
}

impl NodeMove {
    /// Move the node back to `from` on an undo, or to `to` again on a redo.
    /// A node a peer moved since stays where the peer put it (`UNDO-007`).
    fn write(&self, tree: &LoroTree, undo: Direction) {
        let (now, then) = match undo {
            Direction::Back => (&self.to, &self.from),
            Direction::Forward => (&self.from, &self.to),
        };
        if !stands(tree, self.node) || tree.parent(self.node) != Some(now.parent) {
            return;
        }
        if let TreeParentId::Node(parent) = then.parent
            && !stands(tree, parent)
        {
            return;
        }
        let moved = match then.pred {
            Some(pred) if tree.parent(pred) == Some(then.parent) && pred != self.node => {
                tree.mov_after(self.node, pred)
            }
            Some(_) => {
                let len = tree.children_num(then.parent).unwrap_or_default();
                tree.mov_to(self.node, then.parent, then.at.to_usize().min(len))
            }
            None => tree.mov_to(self.node, then.parent, 0),
        };
        // A refused move leaves the node where it stands, and the replay
        // goes on with the rest of the step.
        let _ = moved;
    }
}

impl TreeWrite {
    /// Replay the write in `doc`: each move back or forward, then each node
    /// the commit created hidden on an undo, shown again on a redo.
    pub(super) fn write(&mut self, doc: &LoroDoc, undo: Direction, hide: Option<&Hide>) {
        let Some(id) = self.tree.clone() else {
            return;
        };
        let tree = doc.get_tree(id);
        let moves: Vec<&NodeMove> = match undo {
            Direction::Back => self.moved.iter().rev().collect(),
            Direction::Forward => self.moved.iter().collect(),
        };
        for step in moves {
            step.write(&tree, undo);
        }
        let Some(hide) = hide else {
            return;
        };
        for (node, hidden) in &mut self.born {
            if !stands(&tree, *node) {
                continue;
            }
            let Ok(meta) = tree.get_meta(*node) else {
                continue;
            };
            match undo {
                Direction::Back => {
                    let value = (hide.value)(doc);
                    if meta.insert(hide.key.as_str(), value.clone()).is_ok() {
                        *hidden = Some(value);
                    }
                }
                Direction::Forward => {
                    // The marker this replay wrote comes off; a marker a peer
                    // wrote since stays (`UNDO-007`).
                    let now = meta
                        .get(hide.key.as_str())
                        .and_then(|v| v.into_value().ok());
                    if now.is_some() && now == *hidden {
                        let _ = meta.delete(hide.key.as_str());
                    }
                    *hidden = None;
                }
            }
        }
    }

    /// Give back the room the two lists hold past their length: a step
    /// keeps the write for as long as the history does.
    pub(super) fn shrink_to_fit(&mut self) {
        self.born.shrink_to_fit();
        self.moved.shrink_to_fit();
    }

    /// The meta map of each node the commit created: a write into it rides
    /// the creation, and a replay skips it.
    pub(super) fn born_metas(&self) -> impl Iterator<Item = loro::ContainerID> + '_ {
        self.born
            .iter()
            .map(|(node, _)| node.associated_meta_container())
    }
}
