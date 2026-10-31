//! The ephemeral cursor frame, outbound and inbound
//! (`1-Foundations/ambre-spec-presence.md`).
//!
//! A caret never enters the CRDT, never persists and is not an undo item
//! (`PROSE-058`, `PROSE-UI-010`). Outbound, a local position becomes an encoded
//! `loro::Cursor` inside a postcard frame. Inbound, a frame is keyed by the
//! sender's canonical `UserId`. `ambre-sync` resolves that id from the
//! authenticated public key before the frame reaches this thread, so a peer
//! cannot assert another user's caret (`PRESENCE-002`, `PRESENCE-003`).
//!
//! **Why a cursor and not an offset** (`PROSE-004`). An offset names a
//! different character on a peer that has applied an insertion this one has
//! not. A `loro::Cursor` rides the operation it was taken against, so after
//! convergence every peer resolves it to the same absolute position.

use loro::cursor::{Cursor, Side};
use loro::{ContainerTrait, TreeID};

use ambre_core::UserId;

use ambre_bus::app::prose::{DocOffset, EncodedCursor};

use crate::doc;
use crate::events::{
    BlockId, CursorPosition, PresencePayload, ProseEvent, ProseOutcome, RemoteCursor, tree_id,
};

use super::ProseState;

impl ProseState {
    /// Build the wire frame for the local caret, and return it as the outcome's
    /// ephemeral half.
    ///
    /// The local caret is drawn by the view itself, so nothing repaints here.
    /// The display name is stamped by this thread, which already holds it; it is
    /// cosmetic and decides nothing (`PRESENCE-004`).
    pub(super) fn update_cursor(&mut self, pos: &CursorPosition) -> ProseOutcome {
        let Some(cursor) = self.cursor_at(&tree_id(pos.block_id), pos.offset) else {
            // A caret in a block this document does not carry is no caret at
            // all, and a frame is best-effort: the next sample corrects it.
            return ProseOutcome::nothing();
        };
        let anchor = pos.anchor.and_then(|(block_id, offset)| {
            self.cursor_at(&tree_id(block_id), offset)
                .map(|cursor| (block_id, EncodedCursor::new(cursor.encode())))
        });
        let payload = PresencePayload {
            block_id: pos.block_id,
            cursor: EncodedCursor::new(cursor.encode()),
            anchor,
            display_name: self.my_name.clone(),
        };
        match postcard::to_allocvec(&payload) {
            Ok(bytes) => {
                let bytes = ambre_core::Payload::<ambre_core::EphemeralBytes>::new(bytes);
                self.last_frame = Some(bytes.clone());
                ProseOutcome::ephemeral_only(bytes)
            }
            Err(e) => {
                tracing::warn!("Prose: the presence sample could not be encoded: {e}");
                ProseOutcome::nothing()
            }
        }
    }

    /// The sample that tells the other sessions this one is open: the last one
    /// sent, or a caret at the start of the first block. Sent once the document
    /// is ready, so a session that never moves its caret still counts as
    /// running (`PROSE-121`).
    pub fn announce(&mut self) -> Option<ambre_core::Payload<ambre_core::EphemeralBytes>> {
        if let Some(frame) = &self.last_frame {
            return Some(frame.clone());
        }
        let first = *self.root_blocks().first()?;
        self.update_cursor(&CursorPosition {
            block_id: first,
            offset: DocOffset::ZERO,
            anchor: None,
        })
        .ephemeral
    }

    /// The answer owed to a session that joined since this one last spoke:
    /// it has not heard of this one yet. Taken once per newcomer.
    pub fn take_greeting(&mut self) -> Option<ambre_core::Payload<ambre_core::EphemeralBytes>> {
        if std::mem::take(&mut self.owe_greeting) == super::Greeting::Given {
            return None;
        }
        self.announce()
    }

    /// Whether a session of `user_id` is open on this document, as far as this
    /// peer knows.
    #[must_use]
    pub(crate) fn is_live(&self, user_id: &UserId) -> bool {
        self.live.contains(user_id)
    }

    /// Apply a remote presence sample.
    ///
    /// The author is the Sync-resolved `from_user_id`, never read from the
    /// payload. Nothing here touches the document, its history or the export
    /// cursor (`PROSE-058`). A frame naming a block this peer has not received,
    /// or one already tombstoned, resolves to nothing. The caret is then
    /// dropped rather than raised (`PROSE-057`), and it reappears when the
    /// delta lands.
    pub fn apply_ephemeral(
        &mut self,
        from_user_id: UserId,
        payload: &ambre_core::Payload<ambre_core::EphemeralBytes>,
    ) -> Vec<ProseEvent> {
        let frame: PresencePayload = match postcard::from_bytes(payload.as_bytes()) {
            Ok(frame) => frame,
            Err(e) => {
                tracing::warn!("Prose: a presence sample was malformed: {e}");
                return Vec::new();
            }
        };
        // A session this one has not met has not heard of it either.
        if self.live.insert(from_user_id) {
            self.owe_greeting = super::Greeting::Owed;
        }
        let Some(position) = self.resolve(&frame) else {
            // Dropped, not raised: the peer keeps the caret it last had until a
            // frame this document can resolve arrives.
            self.presence.remove(&from_user_id);
            return vec![self.live_presence_event()];
        };
        self.presence
            .insert(from_user_id, (position, frame.display_name));
        vec![self.live_presence_event()]
    }

    /// Forget a peer's caret, when the session says they left.
    pub fn drop_presence(&mut self, user_id: UserId) -> Vec<ProseEvent> {
        self.live.remove(&user_id);
        if self.presence.remove(&user_id).is_none() {
            return Vec::new();
        }
        vec![self.live_presence_event()]
    }

    /// One remote peer's last resolved caret, for a test and for the overlay's
    /// own reads.
    #[must_use]
    pub fn presence_of(&self, user_id: UserId) -> Option<CursorPosition> {
        self.presence.get(&user_id).map(|(position, _)| *position)
    }

    /// The whole live peer set, which the view adopts whole: a departure is the
    /// absence of an entry (`PRESENCE-009`).
    fn live_presence_event(&self) -> ProseEvent {
        let mut cursors: Vec<RemoteCursor> = self
            .presence
            .iter()
            .map(|(user_id, (position, display_name))| RemoteCursor {
                user_id: *user_id,
                display_name: display_name.clone(),
                position: *position,
            })
            .collect();
        // By user, so two peers hand the overlay the same list in one order.
        cursors.sort_by_key(|cursor| *cursor.user_id.as_bytes());
        ProseEvent::RemoteCursors { cursors }
    }

    /// The absolute position a frame names on this replica, or `None` when this
    /// replica cannot resolve it (`PROSE-057`).
    fn resolve(&self, frame: &PresencePayload) -> Option<CursorPosition> {
        let offset = self.resolve_one(frame.block_id, &frame.cursor)?;
        let anchor = match &frame.anchor {
            Some((block, bytes)) => Some((*block, self.resolve_one(*block, bytes)?)),
            None => None,
        };
        Some(CursorPosition {
            block_id: frame.block_id,
            offset,
            anchor,
        })
    }

    /// One encoded cursor, resolved against this replica's own document.
    fn resolve_one(&self, block: BlockId, encoded: &EncodedCursor) -> Option<DocOffset> {
        // A block this peer has not received, or one already tombstoned, is not
        // a place a caret can sit.
        self.body_of(&tree_id(block))?;
        let cursor = Cursor::decode(encoded.as_bytes()).ok()?;
        let resolved = self.doc.get_cursor_pos(&cursor).ok()?;
        u32::try_from(resolved.current.pos).ok().map(DocOffset::new)
    }

    /// The encoded cursor for a local offset in a live block.
    fn cursor_at(&self, block_id: &TreeID, offset: DocOffset) -> Option<Cursor> {
        let body = self.body_of(block_id)?;
        let offset = offset.to_usize();
        if offset > body.len_unicode() {
            return None;
        }
        // A caret at the end of an empty body has no operation to ride, and
        // Loro answers `None`. It is still a place the view draws, so the frame
        // carries a cursor pinned to the container instead.
        body.get_cursor(offset, Side::Left).or_else(|| {
            Some(Cursor::new(
                None,
                doc::get_block_text(&self.doc, block_id).id(),
                Side::Left,
                offset,
            ))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::ProseCommand;
    use crate::logic::ProseState;

    fn a_user(byte: u8) -> UserId {
        UserId::from_bytes([byte; 16])
    }

    #[test]
    fn a_caret_is_never_written_to_the_document() {
        let mut state = ProseState::new().expect("a document is built");
        let block = state.root_blocks()[0];
        state.handle_command(ProseCommand::InsertText {
            block_id: block,
            pos: DocOffset::ZERO,
            text: "hello".to_owned(),
        });
        let before = state.export_snapshot();
        let outcome = state.handle_command(ProseCommand::UpdateCursor {
            pos: CursorPosition {
                block_id: block,
                offset: DocOffset::new(3),
                anchor: None,
            },
        });
        assert!(outcome.update.is_none());
        assert!(outcome.ephemeral.is_some());
        assert_eq!(state.export_snapshot().len(), before.len());
    }

    #[test]
    fn a_caret_resolves_to_the_same_place_on_both_peers() {
        let mut alice = ProseState::new().expect("a document is built");
        let block = alice.root_blocks()[0];
        let seed = alice.export_snapshot();
        let mut bob = ProseState::new().expect("a document is built");
        bob.import_snapshot(&seed).expect("the seed imports");

        let hello = alice
            .handle_command(ProseCommand::InsertText {
                block_id: block,
                pos: DocOffset::ZERO,
                text: "hello".to_owned(),
            })
            .update
            .expect("a delta");
        bob.apply_remote(&hello);

        let frame = alice
            .handle_command(ProseCommand::UpdateCursor {
                pos: CursorPosition {
                    block_id: block,
                    offset: DocOffset::new(5),
                    anchor: None,
                },
            })
            .ephemeral
            .expect("a frame");
        // Bob inserts in front of Alice's caret before he reads her frame. An
        // offset would name the wrong character; the cursor does not.
        let shove = bob
            .handle_command(ProseCommand::InsertText {
                block_id: block,
                pos: DocOffset::ZERO,
                text: "oh ".to_owned(),
            })
            .update
            .expect("a delta");
        alice.apply_remote(&shove);
        bob.apply_ephemeral(a_user(1), &frame);
        let bob_view = bob.presence_of(a_user(1)).expect("Alice's caret");
        assert_eq!(bob_view.offset, DocOffset::new(8));
    }

    #[test]
    fn a_sample_naming_a_block_this_peer_does_not_hold_is_dropped() {
        let mut alice = ProseState::new().expect("a document is built");
        // A block Alice mints: the root paragraph is shared scaffolding, which
        // every peer holds (`PROSE-129`).
        let root = alice.root_blocks()[0];
        alice.handle_command(ProseCommand::SplitBlock {
            block_id: root,
            pos: DocOffset::ZERO,
        });
        let block = alice
            .root_blocks()
            .into_iter()
            .find(|b| *b != root)
            .expect("the split mints a block");
        let frame = alice
            .handle_command(ProseCommand::UpdateCursor {
                pos: CursorPosition {
                    block_id: block,
                    offset: DocOffset::ZERO,
                    anchor: None,
                },
            })
            .ephemeral
            .expect("a frame");
        // A second document that never received Alice's block at all.
        let mut stranger = ProseState::new().expect("a document is built");
        stranger.apply_ephemeral(a_user(2), &frame);
        assert!(stranger.presence_of(a_user(2)).is_none());
    }

    #[test]
    fn a_malformed_sample_is_dropped_rather_than_raised() {
        let mut state = ProseState::new().expect("a document is built");
        let events = state.apply_ephemeral(a_user(3), &vec![0xFF, 0x00].into());
        assert!(events.is_empty());
    }
}
