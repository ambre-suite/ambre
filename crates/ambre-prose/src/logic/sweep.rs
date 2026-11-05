//! The removal for good of a hidden block whose session has ended
//! (`ambre-spec-prose-schema.md` section 2, `PROSE-121`).
//!
//! A deletion hides a block, so the session that hid it can undo it. Once that
//! session has ended, no undo can bring the block back, and the node only
//! weighs on the document. A compaction runs this pass: it deletes every such
//! node from the tree and empties the bodies under it.
//!
//! A session runs when it is this peer's own, or when its user has a session
//! open on the document. The registry names the user behind a PeerID. Every
//! session announces itself once the document is ready and answers each
//! newcomer once, so a silent session is known. It leaves on the relay's word
//! (`PRESENCE-014`). A user's two sessions read as one, so one of them closing
//! may let the other's hidden block go: its undo then brings back nothing.

use ambre_core::authorship::PeerEntry;
use loro::{LoroValue, TreeID, ValueOrContainer};

use crate::doc;
use crate::events::{ProseEvent, ProseOutcome};

use super::ProseState;

impl ProseState {
    /// Remove for good every hidden block whose session has ended, and every
    /// block under it (`PROSE-121`).
    ///
    /// A subtree that still holds a marker of a running session stays whole.
    /// The removal is an ordinary delta of this session, so the next snapshot
    /// folds it in like any other edit.
    pub fn sweep_hidden(&mut self) -> ProseOutcome {
        let tree = doc::blocks(&self.doc);
        let ended: Vec<TreeID> = tree
            .nodes()
            .into_iter()
            .filter(|id| !tree.is_node_deleted(id).unwrap_or(true))
            .filter(|id| {
                self.marker_of(id)
                    .is_some_and(|peer| !self.session_runs(peer))
            })
            .filter(|id| {
                Self::subtree(&tree, *id).iter().all(|node| {
                    self.marker_of(node)
                        .is_none_or(|peer| !self.session_runs(peer))
                })
            })
            .collect();
        if ended.is_empty() {
            return ProseOutcome::nothing();
        }
        for id in ended {
            // An ancestor removed earlier in the loop took this node with it.
            if tree.is_node_deleted(&id).unwrap_or(true) {
                continue;
            }
            for node in Self::subtree(&tree, id) {
                let body = doc::get_block_text(&self.doc, &node);
                let len = body.len_unicode();
                if len > 0
                    && let Err(e) = body.delete(0, len)
                {
                    tracing::warn!("Prose: a removed block kept its body: {e:?}");
                }
            }
            if let Err(e) = tree.delete(id) {
                tracing::warn!("Prose: the tree refused to remove a hidden block: {e:?}");
            }
        }
        self.doc
            .set_next_commit_origin(ambre_core::crdt::CommitOrigin::Sweep.as_str());
        self.doc.commit();
        let update = self.flush_delta();
        let display = update
            .clone()
            .map(|delta| {
                vec![ProseEvent::DocUpdated {
                    payload: ambre_core::LoroPayload::Delta(delta),
                }]
            })
            .unwrap_or_default();
        ProseOutcome {
            display,
            update,
            ephemeral: None,
        }
    }

    /// The PeerID a block's own hiding marker names, or `None` when the block
    /// carries no marker or one that reads as no PeerID.
    fn marker_of(&self, id: &TreeID) -> Option<ambre_core::PeerId> {
        let node = doc::Fields::new(doc::blocks(&self.doc).get_meta(*id).ok()?);
        match node.get(doc::NodeKey::Hidden)? {
            ValueOrContainer::Value(LoroValue::String(peer)) => {
                peer.parse().ok().map(ambre_core::PeerId::new)
            }
            _ => None,
        }
    }

    /// Whether the session behind `peer` runs: it is this one, or the user the
    /// registry names for it has a session open on this document.
    fn session_runs(&self, peer: ambre_core::PeerId) -> bool {
        if peer == self.peer {
            return true;
        }
        let entry = match self
            .doc
            .get_map(ambre_core::crdt::FoundationRoot::Peers.as_str())
            .get(ambre_core::PeerKey::of(peer).as_str())
        {
            Some(ValueOrContainer::Value(LoroValue::Binary(bytes))) => {
                PeerEntry::from_bytes(&bytes)
            }
            _ => None,
        };
        entry.is_some_and(|entry| self.is_live(&entry.user))
    }

    /// The node and every node under it, in no promised order.
    fn subtree(tree: &loro::LoroTree, id: TreeID) -> Vec<TreeID> {
        let mut out = Vec::new();
        let mut stack = vec![id];
        while let Some(node) = stack.pop() {
            out.push(node);
            stack.extend(tree.children(node).unwrap_or_default());
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::{BlockId, BlockInit, CursorPosition, ProseCommand, tree_id};
    use crate::message::DocChar;
    use ambre_core::FixedBytes;
    use ambre_core::id::{Id, User};
    use ambre_core::index::Pos;

    /// Bob's user.
    const BOB: Id<User> = Id::from_bytes([2; 16]);

    /// Alice on one document, and Bob on the same one under a session the
    /// registry ties to his user. Bob adds a block and hides it, and Alice
    /// reads both writes.
    fn bob_hid_a_block() -> (ProseState, ProseState, BlockId) {
        let mut alice =
            ProseState::with_peer(ambre_core::PeerId::new(1)).expect("a document is built");
        let mut bob =
            ProseState::with_peer(ambre_core::PeerId::new(2)).expect("a document is built");
        bob.import_snapshot(&alice.export_snapshot())
            .expect("the seed imports");
        let entry = PeerEntry {
            user: BOB,
            nonce: FixedBytes::from_array([9; 16]),
            signature: FixedBytes::from_array([0; 64]),
        };
        assert!(bob.adopt_session(ambre_core::PeerId::new(22), entry));
        let first = bob.root_blocks()[0];
        bob.handle_command(ProseCommand::InsertBlock {
            parent_id: None,
            after_id: Some(first),
            init: BlockInit::Paragraph,
            style_id: None,
        });
        let hidden = bob.root_blocks()[1];
        bob.handle_command(ProseCommand::InsertText {
            block_id: hidden,
            pos: Pos::<DocChar>::ZERO,
            text: "gone".to_owned(),
        });
        bob.handle_command(ProseCommand::DeleteBlock { block_id: hidden });
        alice
            .import_snapshot(&bob.export_snapshot())
            .expect("Bob's writes import");
        (alice, bob, hidden)
    }

    /// Whether the block's node is still in the tree, hidden or not.
    fn kept(state: &ProseState, block: BlockId) -> bool {
        let tree = doc::blocks(state.document());
        tree.is_node_deleted(&tree_id(block)) == Ok(false)
    }

    #[test]
    fn a_block_hidden_by_a_session_that_ended_is_removed_for_good() {
        let (mut alice, mut bob, hidden) = bob_hid_a_block();
        let delta = alice.sweep_hidden().update.expect("the removal travels");

        assert!(!kept(&alice, hidden));
        assert_eq!(
            doc::get_block_text(alice.document(), &tree_id(hidden)).to_string(),
            "",
            "the body goes with the node"
        );
        bob.apply_remote(&delta);
        assert!(!kept(&bob, hidden), "every peer reads the removal");
        assert!(
            alice.sweep_hidden().update.is_none(),
            "a second pass writes nothing"
        );
    }

    #[test]
    fn a_block_hidden_by_a_running_session_stays() {
        let (mut alice, mut bob, hidden) = bob_hid_a_block();
        // Bob's own pass: the session that hid the block is his.
        assert!(bob.sweep_hidden().update.is_none());
        assert!(kept(&bob, hidden));

        // Alice holds Bob's caret, so his session runs.
        let first = bob.root_blocks()[0];
        let frame = bob
            .handle_command(ProseCommand::UpdateCursor {
                pos: CursorPosition {
                    block_id: first,
                    offset: Pos::<DocChar>::ZERO,
                    anchor: None,
                },
            })
            .ephemeral
            .expect("a frame");
        alice.apply_ephemeral(BOB, &frame);
        assert!(alice.sweep_hidden().update.is_none());
        assert!(kept(&alice, hidden));

        // Bob can still undo, and the block comes back.
        bob.handle_command(ProseCommand::Undo);
        assert!(bob.root_blocks().contains(&hidden));
    }

    #[test]
    fn a_silent_session_runs_once_it_has_announced_itself() {
        let (mut alice, mut bob, hidden) = bob_hid_a_block();
        // Bob never moved his caret: his announce still reaches Alice.
        let hello = bob.announce().expect("a frame");
        alice.apply_ephemeral(BOB, &hello);
        assert!(alice.sweep_hidden().update.is_none());
        assert!(kept(&alice, hidden));

        // A caret Alice cannot place still says the session runs.
        let first = bob.root_blocks()[0];
        bob.handle_command(ProseCommand::InsertBlock {
            parent_id: None,
            after_id: Some(first),
            init: BlockInit::Paragraph,
            style_id: None,
        });
        let unseen = bob.root_blocks()[1];
        let frame = bob
            .handle_command(ProseCommand::UpdateCursor {
                pos: CursorPosition {
                    block_id: unseen,
                    offset: Pos::<DocChar>::ZERO,
                    anchor: None,
                },
            })
            .ephemeral
            .expect("a frame");
        alice.apply_ephemeral(BOB, &frame);
        assert!(alice.presence_of(BOB).is_none());
        assert!(alice.sweep_hidden().update.is_none());
    }

    #[test]
    fn a_newcomer_is_answered_once() {
        let (mut alice, mut bob, _) = bob_hid_a_block();
        assert!(alice.take_greeting().is_none(), "no one has joined yet");
        let hello = bob.announce().expect("a frame");
        alice.apply_ephemeral(BOB, &hello);
        let answer = alice.take_greeting().expect("Alice answers Bob");
        assert!(alice.take_greeting().is_none(), "once");

        alice.apply_ephemeral(BOB, &hello);
        assert!(alice.take_greeting().is_none(), "Bob is no newcomer now");
        bob.apply_ephemeral(Id::from_bytes([1; 16]), &answer);
        assert!(bob.take_greeting().is_some(), "Bob answers in turn");
    }

    #[test]
    fn a_session_whose_user_left_has_ended() {
        let (mut alice, mut bob, hidden) = bob_hid_a_block();
        let hello = bob.announce().expect("a frame");
        alice.apply_ephemeral(BOB, &hello);
        alice.drop_presence(BOB);
        assert!(alice.sweep_hidden().update.is_some());
        assert!(!kept(&alice, hidden));
    }

    #[test]
    fn the_removal_is_no_step_of_the_undo_stack() {
        let (mut alice, _bob, hidden) = bob_hid_a_block();
        alice.sweep_hidden();
        alice.handle_command(ProseCommand::Undo);
        assert!(!kept(&alice, hidden));
    }
}
