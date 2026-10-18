// ambre-logic/src/channels.rs
// ============================================================================
// Murmure channels: route a channel command to its engine, and a remote delta
// to the channel that holds its document. A channel is a set of documents,
// never an entry of `docs` (the Murmure engine spec).
// ============================================================================

use super::*;
use ambre_bus::{
    ChannelActivity, ChannelDocument, ChannelDocumentKind, ChannelId, ChannelMatches,
    DocumentActivity, MetaId, MurmureCommand, MurmureEvent,
};
use ambre_bus::{MessageId, SegmentId, Seq, SignedDelta, UnreadCount};
use ambre_murmure::{ChannelState, MurmureError, Opening, OpeningChannel, Position, Query, Signer};

/// This user's read cursor on one channel document (the Murmure unread spec).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ReadCursor {
    /// The relay number of the last update read.
    pub(crate) read: Seq,
    /// Whether the document holds an update past `read`: the relay's yes or
    /// no, the one thing known of a segment this client does not hold.
    pub(crate) active: bool,
}

/// What the Logic thread keeps of a channel whose view closed. Its documents
/// are light subscriptions, and their summaries count its new messages.
#[derive(Debug)]
pub(crate) struct ClosedChannel {
    /// The documents released at close, each a light subscription now.
    documents: Vec<DocumentId>,
    /// The final segments not held at close, whose cursors a drop forgets.
    finals: Vec<DocumentId>,
    /// The unread count at close, `None` when it was not known then.
    base: Option<UnreadCount>,
    /// The messages each document's summaries counted since the close.
    counted: HashMap<DocumentId, u32>,
    /// The horizon of each document's copy in the local store, until a
    /// reopen asks for it.
    copies: HashMap<DocumentId, u64>,
    /// The copies a reopen is reading back, each with its horizon.
    reading: HashMap<DocumentId, u64>,
}

/// Every document of an open channel that may carry a read cursor: those it
/// holds, and its final segments.
pub(crate) fn cursored_documents(channel: &ChannelState) -> Vec<DocumentId> {
    let mut documents = channel.document_ids();
    for segment in channel.final_segments() {
        if !documents.contains(&segment.document_id()) {
            documents.push(segment.document_id());
        }
    }
    documents
}

impl ClosedChannel {
    /// Whether the channel held `document_id` at close.
    pub(crate) fn holds(&self, document_id: DocumentId) -> bool {
        self.documents.contains(&document_id)
    }

    /// Every document whose read cursor the channel may carry.
    pub(crate) fn cursored(&self) -> Vec<DocumentId> {
        self.documents.iter().chain(&self.finals).copied().collect()
    }

    /// The channel's mark: the count at close plus the messages counted
    /// since. With no count at close, a counted message marks it active, and
    /// none leaves the mark the view holds: `None`.
    fn mark(&self, channel_id: ChannelId) -> Option<ChannelActivity> {
        let since = UnreadCount(
            self.counted
                .values()
                .fold(0, |sum, n| sum.saturating_add(*n)),
        );
        match self.base {
            Some(base) => {
                let unread = base.plus(since);
                Some(ChannelActivity {
                    channel_id,
                    active: !unread.is_zero(),
                    unread: Some(unread),
                })
            }
            None => (!since.is_zero()).then_some(ChannelActivity {
                channel_id,
                active: true,
                unread: None,
            }),
        }
    }
}

/// A command that writes a channel or a space document, held until that
/// document may be written (`AUTHOR-026`).
#[derive(Debug)]
pub(crate) enum HeldCommand {
    /// A command of an open channel.
    Channel(ChannelId, MurmureCommand),
    /// A command of an open space.
    Space(ambre_bus::SpaceId, ambre_bus::SpaceCommand),
}

/// The unread count of an open channel, `None` when a part of it is unknown
/// (the Murmure unread spec). Unknown is a held segment with no cursor yet,
/// or a final segment not loaded that may hold unread messages.
fn unread_of(
    channel: &ChannelState,
    cursors: &HashMap<DocumentId, ReadCursor>,
    me: UserId,
) -> Option<UnreadCount> {
    let held = channel.held_segments();
    let mut count = UnreadCount::ZERO;
    for segment in &held {
        let cursor = cursors.get(&segment.document_id())?;
        count = count.plus(channel.unread(*segment, cursor.read, me).ok()?);
    }
    let unknown_final = channel
        .final_segments()
        .into_iter()
        .filter(|segment| !held.contains(segment))
        .any(|segment| cursors.get(&segment.document_id()).is_none_or(|c| c.active));
    (!unknown_final).then_some(count)
}

impl LogicState {
    /// The mark of an open channel, counted, or `None` when its count is
    /// not known yet.
    fn counted(&self, channel_id: ChannelId) -> Option<ChannelActivity> {
        let channel = self.channels.get(&channel_id)?;
        let unread = unread_of(channel, &self.read_cursors, self.my_user_id?)?;
        Some(ChannelActivity {
            channel_id,
            active: !unread.is_zero(),
            unread: Some(unread),
        })
    }

    /// Hand the view the count of an open channel, once it is known.
    fn emit_count(&self, channel_id: ChannelId) {
        if let Some(mark) = self.counted(channel_id) {
            self.send_to_ui(UiEvent::ChannelActivity {
                channels: vec![mark],
            });
        }
    }

    /// Ask the relay for the documents of a channel (the Murmure engine spec).
    /// A channel already open or opening is left as it is.
    pub(crate) fn open_channel(&mut self, channel_id: ChannelId) {
        if self.channels.contains_key(&channel_id) || self.opening.contains_key(&channel_id) {
            return;
        }
        // A channel closed earlier keeps its light subscriptions while it
        // opens: each document reopens from its local copy (the Murmure
        // engine spec). Its read cursors stay where they were (`RESID-013`).
        self.opening.insert(channel_id, None);
        // A channel a catalogue lists is public, and its log admits a member
        // who joined alone. Any other channel's log admits only invites
        // (`SYNC-106`).
        let self_join = self.lists_channel(channel_id);
        self.send_to_sync(SyncCommand::OpenChannel {
            channel_id,
            self_join,
        });
    }

    /// Hold a channel the relay just created, and open the session of its two
    /// documents. Each is written again under its PeerID once its identity
    /// lands. Its whole content leaves as its first update once the Sync
    /// worker holds the document.
    pub(crate) fn hold_created(&mut self, channel: ChannelState) {
        for document_id in [
            channel.meta_id().document_id(),
            channel.active_id().document_id(),
        ] {
            self.send_to_sync(SyncCommand::OpenDocument {
                document_id,
                doc_type: DocType::Murmure,
            });
        }
        self.channels.insert(channel.channel_id(), channel);
    }

    /// Note the session identity Sync issued for a channel or a space
    /// document. An open channel or space holding it adopts it (`AUTHOR-002`). An identity no channel or space expects is
    /// stale, and dropped.
    pub(crate) fn adopt_murmure_session(
        &mut self,
        document_id: DocumentId,
        peer: u64,
        entry: Vec<u8>,
    ) {
        let space = ambre_bus::SpaceId::from_document(document_id);
        let expected = self.channels.values().any(|c| c.holds(document_id))
            || self
                .opening
                .values()
                .flatten()
                .any(|o| o.expects(document_id))
            || self.spaces.contains_key(&space)
            || self.space_opening.contains_key(&space);
        if !expected {
            return;
        }
        self.murmure_identities.insert(document_id, (peer, entry));
        self.settle_murmure_session(document_id);
    }

    /// Adopt the identity waiting for `document_id` when an open channel or
    /// space now holds it, then run the commands that waited on it.
    ///
    /// A document this client created comes back whole, written under its
    /// PeerID: its first update, sent once the Sync worker holds it.
    pub(crate) fn settle_murmure_session(&mut self, document_id: DocumentId) {
        let Some((peer, entry)) = self.murmure_identities.get(&document_id).cloned() else {
            return;
        };
        let adopted =
            if let Some(channel) = self.channels.values_mut().find(|c| c.holds(document_id)) {
                channel.adopt_session(document_id, peer, entry)
            } else if let Some(adopted) = self.adopt_space_session(document_id, peer, entry) {
                adopted
            } else {
                // Its channel or its space is still opening.
                return;
            };
        self.murmure_identities.remove(&document_id);
        match adopted {
            Ok(Some(first)) => {
                self.unsent.insert(document_id, first);
            }
            Ok(None) => {}
            Err(error) => tracing::warn!("Logic: identity of {document_id} refused: {error}"),
        }
        self.release_waiting(document_id);
    }

    /// Whether a command may write `document_id` now: its identity landed,
    /// and a document this client created has left whole. A delta sent
    /// before that first update would reach a relay that holds no document.
    pub(crate) fn murmure_writable(&self, document_id: DocumentId, identified: bool) -> bool {
        identified && !self.unsent.contains_key(&document_id)
    }

    /// Hold a command until `document_id` may be written. Past the cap the
    /// command is dropped and logged, so the ones held keep their order.
    pub(crate) fn wait_for_session(&mut self, document_id: DocumentId, command: HeldCommand) {
        let waiting = self.murmure_waiting.entry(document_id).or_default();
        if waiting.len() >= HELD_EDITS_LIMIT {
            tracing::warn!("Logic: too many edits wait on {document_id}; dropped");
            return;
        }
        waiting.push(command);
    }

    /// Run, in their order, the commands that waited on `document_id`. One
    /// that still cannot write waits again.
    pub(crate) fn release_waiting(&mut self, document_id: DocumentId) {
        for waiting in self
            .murmure_waiting
            .remove(&document_id)
            .unwrap_or_default()
        {
            match waiting {
                HeldCommand::Channel(channel_id, command) => {
                    self.handle_channel_command(channel_id, command);
                }
                HeldCommand::Space(space_id, command) => {
                    self.handle_space_command(space_id, command);
                }
            }
        }
    }

    /// Forget what waited on a document whose session ended.
    pub(crate) fn forget_murmure_session(&mut self, document_id: DocumentId) {
        self.murmure_identities.remove(&document_id);
        self.murmure_waiting.remove(&document_id);
        self.unsent.remove(&document_id);
    }

    /// The view of a channel closed. Each document of an open channel reaches
    /// the local store, then leaves memory as a light subscription. Its
    /// summaries count the channel's new messages (`RESID-004`, `RESID-006`).
    /// A channel still opening ends the session
    /// of each document it asked for.
    pub(crate) fn close_channel(&mut self, channel_id: ChannelId) {
        let Some(channel) = self.channels.remove(&channel_id) else {
            return self.end_channel(channel_id);
        };
        let base = self
            .my_user_id
            .and_then(|me| unread_of(&channel, &self.read_cursors, me));
        let documents = channel.document_ids();
        let finals = cursored_documents(&channel)
            .into_iter()
            .filter(|id| !documents.contains(id))
            .collect();
        let mut copies = HashMap::new();
        for &document_id in &documents {
            let horizon = self
                .channel_ledgers
                .remove(&document_id)
                .map_or(0, |ledger| ledger.contiguous_end());
            self.forget_murmure_session(document_id);
            // The copy carries the relay's numbers it knows, so a reopen
            // counts from it. One that fails to export is fetched whole.
            let applied = match channel.snapshot(document_id) {
                Ok(copy) => {
                    self.send_to_persist(document_id, PersistWrite::Snapshot(copy));
                    self.send_to_persist(document_id, PersistWrite::Hidden);
                    copies.insert(document_id, horizon);
                    horizon
                }
                Err(error) => {
                    tracing::warn!("Logic: no local copy of {document_id}: {error}");
                    0
                }
            };
            self.send_to_sync(SyncCommand::ReleaseDocument {
                document_id,
                applied_seq: applied,
            });
        }
        self.closed_channels.insert(
            channel_id,
            ClosedChannel {
                documents,
                finals,
                base,
                counted: HashMap::new(),
                copies,
                reading: HashMap::new(),
            },
        );
    }

    /// Ask the Sync worker for one document of an opening channel. A document
    /// the channel left in the local store is read back first, and opens
    /// once its copy lands (the Murmure engine spec).
    fn open_channel_document(&mut self, channel_id: ChannelId, document_id: DocumentId) {
        if let Some(closed) = self.closed_channels.get_mut(&channel_id)
            && let Some(horizon) = closed.copies.remove(&document_id)
        {
            closed.reading.insert(document_id, horizon);
            self.send_to_persist(
                document_id,
                PersistWrite::ReadBack {
                    doc_type: DocType::Murmure,
                },
            );
            return;
        }
        self.send_to_sync(SyncCommand::OpenDocument {
            document_id,
            doc_type: DocType::Murmure,
        });
    }

    /// The local copy of a reopening channel's document landed. The opening
    /// takes it as a snapshot at its horizon, and the Sync worker replays
    /// only what follows (`RESID-010`). An empty copy drops the light
    /// subscription, so the open fetches the document whole (`RESID-012`).
    pub(crate) fn channel_copy_landed(
        &mut self,
        document_id: DocumentId,
        snapshot: Option<Vec<u8>>,
        deltas: &[Vec<u8>],
    ) {
        let Some((&channel_id, closed)) = self
            .closed_channels
            .iter_mut()
            .find(|(_, closed)| closed.reading.contains_key(&document_id))
        else {
            return;
        };
        let horizon = closed.reading.remove(&document_id).unwrap_or(0);
        // An opening that ended meanwhile closed the subscription already.
        let Some(opening) = self
            .opening
            .get_mut(&channel_id)
            .and_then(Option::as_mut)
            .filter(|opening| opening.expects(document_id))
        else {
            return;
        };
        if let Some(copy) = snapshot {
            opening.receive_at(document_id, Some(Order::Snapshot(Seq(horizon))), &copy);
            for delta in deltas {
                opening.receive(document_id, delta);
            }
            self.channel_ledgers
                .entry(document_id)
                .or_default()
                .raise_floor(horizon);
        } else {
            tracing::warn!("Logic: no usable local copy of {document_id}, fetching it");
            self.send_to_sync(SyncCommand::CloseDocument { document_id });
        }
        self.send_to_sync(SyncCommand::OpenDocument {
            document_id,
            doc_type: DocType::Murmure,
        });
    }

    /// Whether a reopening channel is reading back the copy of `document_id`.
    pub(crate) fn reads_channel_copy(&self, document_id: DocumentId) -> bool {
        self.closed_channels
            .values()
            .any(|closed| closed.reading.contains_key(&document_id))
    }

    /// A reopened channel is open: the record of its close goes, and the
    /// light subscription of each document it no longer holds ends.
    fn settle_reopen(&mut self, channel_id: ChannelId) {
        let Some(closed) = self.closed_channels.remove(&channel_id) else {
            return;
        };
        let Some(channel) = self.channels.get(&channel_id) else {
            return;
        };
        let left: Vec<DocumentId> = closed
            .documents
            .into_iter()
            .filter(|id| !channel.holds(*id))
            .collect();
        for document_id in left {
            self.send_to_sync(SyncCommand::CloseDocument { document_id });
        }
    }

    /// End a channel, open, opening or closed: the session or the light
    /// subscription of each of its documents ends, so Sync forgets its key.
    pub(crate) fn end_channel(&mut self, channel_id: ChannelId) {
        self.end_light_channel(channel_id);
        let documents = match (
            self.channels.remove(&channel_id),
            self.opening.remove(&channel_id),
        ) {
            (Some(channel), _) => channel.document_ids(),
            (None, Some(Some(opening))) => opening.document_ids(),
            (None, _) => Vec::new(),
        };
        for document_id in documents {
            self.channel_ledgers.remove(&document_id);
            self.forget_murmure_session(document_id);
            self.send_to_sync(SyncCommand::CloseDocument { document_id });
        }
    }

    /// End the light subscriptions a closed channel left, if it left any.
    pub(crate) fn end_light_channel(&mut self, channel_id: ChannelId) {
        let Some(closed) = self.closed_channels.remove(&channel_id) else {
            return;
        };
        for document_id in closed.documents {
            self.send_to_sync(SyncCommand::CloseDocument { document_id });
        }
    }

    /// Count the activity of a closed channel's document for its channel,
    /// and hand the view the channel's new mark (the Murmure unread spec).
    /// Returns whether a closed channel holds the document.
    pub(crate) fn count_closed(
        &mut self,
        document_id: DocumentId,
        counters: ambre_core::activity::ActivityCounters,
    ) -> bool {
        let Some((&channel_id, closed)) = self
            .closed_channels
            .iter_mut()
            .find(|(_, closed)| closed.holds(document_id))
        else {
            return false;
        };
        closed.counted.insert(document_id, counters.content);
        if let Some(mark) = closed.mark(channel_id) {
            self.send_to_ui(UiEvent::ChannelActivity {
                channels: vec![mark],
            });
        }
        true
    }

    /// The relay listed the documents of a channel: open its `meta` first,
    /// which names the live segments (the Murmure engine spec).
    pub(crate) fn channel_listed(&mut self, channel_id: ChannelId, documents: &[ChannelDocument]) {
        // A listing for a channel closed meanwhile, or listed twice, is stale.
        let Some(slot @ None) = self.opening.get_mut(&channel_id) else {
            return;
        };
        let Some(meta) = documents
            .iter()
            .find(|d| d.kind == ChannelDocumentKind::Meta)
        else {
            self.opening.remove(&channel_id);
            self.channel_open_failed(channel_id, "the channel has no meta document".into());
            return;
        };
        *slot = Some(OpeningChannel::new(
            channel_id,
            MetaId::from_document(meta.document_id),
        ));
        self.open_channel_document(channel_id, meta.document_id);
    }

    /// Tell the view a channel could not open. Nothing of it is held.
    pub(crate) fn channel_open_failed(&self, channel_id: ChannelId, message: String) {
        tracing::warn!("Logic: channel {channel_id} failed to open: {message}");
        self.send_to_ui(UiEvent::Channel {
            channel_id,
            event: MurmureEvent::OpenFailed { message },
        });
    }

    /// The relay refused the listing for want of membership. The user left
    /// the channel, or it went, by a request whose answer never came back.
    /// Nothing of it was held yet, so the view alone learns it is gone.
    pub(crate) fn channel_refused(&mut self, channel_id: ChannelId) {
        if self.opening.remove(&channel_id).is_none() {
            return;
        }
        self.end_light_channel(channel_id);
        tracing::info!("Logic: channel {channel_id} refused its open");
        self.send_to_ui(UiEvent::Channel {
            channel_id,
            event: MurmureEvent::Left,
        });
    }

    /// A document of a channel has all its bytes. Returns `false` when no
    /// channel, open or opening, asked for it.
    pub(crate) fn channel_document_ready(&mut self, document_id: DocumentId) -> bool {
        if let Some((&channel_id, channel)) =
            self.channels.iter_mut().find(|(_, c)| c.holds(document_id))
        {
            // An older segment the view asked for. A second
            // ready for a live document, after a reconnect, changes nothing.
            if channel.is_pending(document_id) {
                match channel.older_ready(document_id) {
                    Ok(event) => {
                        self.send_to_ui(UiEvent::Channel { channel_id, event });
                        self.emit_count(channel_id);
                    }
                    Err(error) => {
                        tracing::warn!("Logic: older segment {document_id} refused: {error}");
                        self.send_to_sync(SyncCommand::CloseDocument { document_id });
                    }
                }
            }
            return true;
        }
        let Some((&channel_id, opening)) = self.opening.iter_mut().find_map(|(id, o)| {
            o.as_mut()
                .filter(|o| o.expects(document_id))
                .map(|o| (id, o))
        }) else {
            return false;
        };
        match opening.ready(document_id) {
            Ok(Opening::Waiting) => {}
            Ok(Opening::OpenSegments(segments)) => {
                for segment in segments {
                    self.open_channel_document(channel_id, segment.document_id());
                }
            }
            Ok(Opening::Open(state)) => {
                self.opening.remove(&channel_id);
                self.send_to_ui(UiEvent::Channel {
                    channel_id,
                    event: MurmureEvent::Named { name: state.name() },
                });
                // The closing segment first, so the view draws the history in
                // order.
                for segment in [state.closing_id(), Some(state.active_id())]
                    .into_iter()
                    .flatten()
                {
                    match state.messages(segment) {
                        Ok(messages) => self.send_to_ui(UiEvent::Channel {
                            channel_id,
                            event: MurmureEvent::Messages { segment, messages },
                        }),
                        Err(error) => tracing::warn!("Logic: segment {segment}: {error}"),
                    }
                }
                let documents = state.document_ids();
                self.channels.insert(channel_id, *state);
                self.settle_reopen(channel_id);
                // Each identity landed while the channel was opening.
                for document_id in documents {
                    self.settle_murmure_session(document_id);
                }
                self.emit_count(channel_id);
            }
            Err(error) => self.end_opening(channel_id, error.to_string()),
        }
        true
    }

    /// A document of a channel could not open. Returns `false` when no
    /// channel asked for it.
    pub(crate) fn channel_document_failed(
        &mut self,
        document_id: DocumentId,
        message: &str,
    ) -> bool {
        if let Some(channel) = self.channels.values_mut().find(|c| c.holds(document_id)) {
            // An older segment is given up, so the view can ask again. A live
            // document of an open channel keeps what it holds.
            channel.cancel_older(document_id);
            tracing::warn!("Logic: channel document {document_id} failed to open: {message}");
            return true;
        }
        let Some(channel_id) = self.opening.iter().find_map(|(id, o)| {
            o.as_ref()
                .is_some_and(|o| o.expects(document_id))
                .then_some(*id)
        }) else {
            return false;
        };
        self.end_opening(channel_id, message.to_owned());
        true
    }

    /// Drop an opening that failed, end the session of each document it asked
    /// for, and tell the view.
    fn end_opening(&mut self, channel_id: ChannelId, message: String) {
        if let Some(Some(opening)) = self.opening.remove(&channel_id) {
            for document_id in opening.document_ids() {
                self.forget_murmure_session(document_id);
                self.send_to_sync(SyncCommand::CloseDocument { document_id });
            }
        }
        self.end_light_channel(channel_id);
        self.channel_open_failed(channel_id, message);
    }

    /// The snapshot of a channel document, for the relay's compaction, with
    /// the last delta it folds in (`SRV-083`).
    pub(crate) fn channel_snapshot(&self, document_id: DocumentId) -> Option<(Vec<u8>, u64)> {
        let channel = self.channels.values().find(|c| c.holds(document_id))?;
        let payload = channel
            .snapshot(document_id)
            .map_err(|e| tracing::warn!("Logic: snapshot of {document_id} failed: {e}"))
            .ok()?;
        let covered = self
            .channel_ledgers
            .get(&document_id)
            .map_or(0, snapshot_check::SeqLedger::contiguous_end);
        Some((payload, covered))
    }

    /// Note the relay's number of a delta for a channel or a space document,
    /// open or opening. A document neither holds is not noted.
    pub(crate) fn record_channel_seq(
        &mut self,
        document_id: DocumentId,
        seq: u64,
        payload: Option<&[u8]>,
    ) {
        if self.holds_murmure(document_id) {
            self.channel_ledgers
                .entry(document_id)
                .or_default()
                .record(seq, payload);
        }
    }

    /// Note a snapshot the relay served for a channel or a space document.
    /// A reopen resumes after the deltas it covers (`RESID-010`).
    pub(crate) fn note_murmure_snapshot(&mut self, document_id: DocumentId, covered: u64) {
        if self.holds_murmure(document_id) {
            self.channel_ledgers
                .entry(document_id)
                .or_default()
                .raise_floor(covered);
        }
    }

    /// Whether an open or opening channel or space holds `document_id`.
    fn holds_murmure(&self, document_id: DocumentId) -> bool {
        let space = ambre_bus::SpaceId::from_document(document_id);
        self.channels.values().any(|c| c.holds(document_id))
            || self
                .opening
                .values()
                .flatten()
                .any(|o| o.expects(document_id))
            || self.spaces.contains_key(&space)
            || self.space_opening.contains_key(&space)
    }

    /// Run one command on an open channel, and emit what it produced
    /// (`BUS-027`).
    ///
    /// The delta leaves for the Sync worker under the document it names, with
    /// its activity summary (the Murmure messages spec). The event goes to the view.
    pub(crate) fn handle_channel_command(
        &mut self,
        channel_id: ChannelId,
        command: MurmureCommand,
    ) {
        let Some(channel) = self.channels.get_mut(&channel_id) else {
            tracing::warn!("Logic: command for channel {channel_id}, which is not open; dropped");
            return;
        };
        // A write before its document may be written waits: under any other
        // PeerID it would never travel (`AUTHOR-026`, `AUTHOR-029`).
        let target = match &command {
            MurmureCommand::SendMessage { .. } => Some(channel.active_id().document_id()),
            MurmureCommand::Name { .. } => Some(channel.meta_id().document_id()),
            MurmureCommand::LoadOlder { .. }
            | MurmureCommand::MarkRead
            | MurmureCommand::MarkUnread { .. } => None,
        };
        if let Some(document_id) = target {
            let identified = channel.identified(document_id);
            if !self.murmure_writable(document_id, identified) {
                return self
                    .wait_for_session(document_id, HeldCommand::Channel(channel_id, command));
            }
        }
        let Some(channel) = self.channels.get_mut(&channel_id) else {
            return;
        };
        match command {
            MurmureCommand::SendMessage { draft, now } => {
                // The author is this session's user. Before `SessionReady`
                // there is none, and a message with no author is refused
                // rather than written under a placeholder (the Murmure schema).
                let Some(author) = self.my_user_id else {
                    tracing::warn!(
                        "Logic: message for channel {channel_id} before the session; dropped"
                    );
                    return;
                };
                match channel.send(&draft, author, now) {
                    Ok(sent) => {
                        self.send_to_sync(SyncCommand::SendUpdate {
                            document_id: sent.segment.document_id(),
                            payload: sent.delta,
                            summary: Some(sent.summary),
                        });
                        self.send_to_ui(UiEvent::Channel {
                            channel_id,
                            event: sent.event,
                        });
                    }
                    Err(error) => {
                        tracing::warn!("Logic: send on channel {channel_id} refused: {error}");
                    }
                }
            }
            MurmureCommand::LoadOlder { segment } => {
                // Nothing to ask for when the segment is the oldest, or the
                // one before it is loaded or on its way.
                if let Some(older) = channel.request_older(segment) {
                    self.send_to_sync(SyncCommand::OpenDocument {
                        document_id: older.document_id(),
                        doc_type: DocType::Murmure,
                    });
                }
            }
            MurmureCommand::MarkRead => {
                // The latest message is on screen: every segment before it is
                // read too, loaded or not (the Murmure unread spec). A relay record, so
                // a viewer moves its cursor as a writer does.
                let live = [Some(channel.active_id()), channel.closing_id()]
                    .into_iter()
                    .flatten()
                    .collect();
                let finals = channel.final_segments();
                // The relay moves each cursor to its document's last number.
                // This client knows the last it was delivered, so its count
                // runs over until the next answer, never under.
                for segment in channel.held_segments() {
                    let document_id = segment.document_id();
                    self.read_cursors.insert(
                        document_id,
                        ReadCursor {
                            read: channel.delivered(document_id),
                            active: false,
                        },
                    );
                }
                for segment in &finals {
                    let cursor =
                        self.read_cursors
                            .entry(segment.document_id())
                            .or_insert(ReadCursor {
                                read: Seq::ZERO,
                                active: false,
                            });
                    cursor.active = false;
                }
                self.send_to_sync(SyncCommand::MarkRead { live, finals });
                let mark = self.counted(channel_id).unwrap_or(ChannelActivity {
                    channel_id,
                    active: false,
                    unread: None,
                });
                self.send_to_ui(UiEvent::ChannelActivity {
                    channels: vec![mark],
                });
            }
            MurmureCommand::Name { name } => match channel.name_direct(name.as_deref()) {
                Ok(named) => {
                    self.send_to_sync(SyncCommand::SendUpdate {
                        document_id: named.meta.document_id(),
                        payload: named.delta,
                        summary: None,
                    });
                    self.send_to_ui(UiEvent::Channel {
                        channel_id,
                        event: named.event,
                    });
                }
                Err(error) => {
                    tracing::warn!("Logic: name of channel {channel_id} refused: {error}");
                }
            },
            MurmureCommand::MarkUnread { segment, message } => {
                self.mark_unread(channel_id, segment, message);
            }
        }
    }

    /// Mark `message` unread: it and every message after it in the channel
    /// read as unread, and no cursor of another channel moves.
    ///
    /// The cursor of its segment moves to just before its number, and the
    /// cursor of each segment listed after it to the start. A message the
    /// relay has not numbered exactly cannot place a cursor, so the mark is
    /// refused rather than guessed.
    fn mark_unread(&mut self, channel_id: ChannelId, segment: SegmentId, message: MessageId) {
        let Some(channel) = self.channels.get(&channel_id) else {
            return;
        };
        let seq = match channel.position(segment, message) {
            Some(Position::Exact(seq)) => seq,
            other => {
                tracing::warn!(
                    "Logic: mark unread on {message} in {segment}: no exact number ({other:?}); refused"
                );
                return;
            }
        };
        let mut cursors = vec![(segment, seq.before())];
        cursors.extend(
            channel
                .segments_after(segment)
                .into_iter()
                .map(|later| (later, Seq::ZERO)),
        );
        for &(moved, read) in &cursors {
            self.read_cursors
                .insert(moved.document_id(), ReadCursor { read, active: true });
        }
        self.send_to_sync(SyncCommand::MoveReadCursors { cursors });
        self.emit_count(channel_id);
    }

    /// Hand the view the relay's activity answer, one mark per channel: a
    /// channel is active when any of its documents is (the Murmure
    /// unread spec).
    ///
    /// Opens nothing: the answer is a yes or no per document, with this
    /// user's own cursor on it. An open channel counts its unread messages
    /// from those cursors.
    pub(crate) fn channel_activity(&mut self, documents: &[DocumentActivity]) {
        let mut channels: Vec<ChannelActivity> = Vec::new();
        let mut index: HashMap<ChannelId, usize> = HashMap::new();
        for document in documents {
            self.read_cursors.insert(
                document.document_id,
                ReadCursor {
                    read: document.read,
                    active: document.active,
                },
            );
            match index.get(&document.channel_id) {
                Some(&i) => {
                    if let Some(channel) = channels.get_mut(i) {
                        channel.active |= document.active;
                    }
                }
                None => {
                    index.insert(document.channel_id, channels.len());
                    channels.push(ChannelActivity {
                        channel_id: document.channel_id,
                        active: document.active,
                        unread: None,
                    });
                }
            }
        }
        for mark in &mut channels {
            if let Some(counted) = self.counted(mark.channel_id) {
                *mark = counted;
            }
        }
        self.send_to_ui(UiEvent::ChannelActivity { channels });
    }

    /// Apply remote payloads to the channel that holds `document_id`
    /// (the Murmure engine spec), each with where the relay placed it.
    /// Returns `false` when no open channel holds it, so the caller reports
    /// the drop as it does for a document.
    pub(crate) fn apply_channel_remote(
        &mut self,
        document_id: DocumentId,
        payloads: &[(Option<Order>, &[u8])],
    ) -> bool {
        // A channel still opening keeps the bytes until the document is ready.
        if let Some(opening) = self
            .opening
            .values_mut()
            .flatten()
            .find(|o| o.expects(document_id))
        {
            for &(order, payload) in payloads {
                opening.receive_at(document_id, order, payload);
            }
            return true;
        }
        let Some((&channel_id, channel)) = self
            .channels
            .iter_mut()
            .find(|(_, channel)| channel.holds(document_id))
        else {
            return false;
        };
        let mut events = Vec::new();
        for &(order, payload) in payloads {
            match channel.apply_remote_at(document_id, order, payload) {
                Ok(produced) => events.extend(produced),
                Err(error) => {
                    tracing::warn!("Logic: delta for {document_id} refused: {error}");
                }
            }
        }
        // One segment answers with its whole list, so only the last answer of
        // a batch is news to the view.
        if let Some(event) = events.pop() {
            self.send_to_ui(UiEvent::Channel { channel_id, event });
            self.emit_count(channel_id);
        }
        true
    }

    /// Apply signed deltas to the channel that holds `document_id`, each
    /// through the authorship check of the engine (the Murmure engine spec).
    /// A refused delta changes nothing, and still counts as delivered. One
    /// from a newer build asks the user to update. Returns `false` when no
    /// open or opening channel holds the document.
    pub(crate) fn apply_channel_signed(
        &mut self,
        document_id: DocumentId,
        deltas: &[SignedDelta],
    ) -> bool {
        let signer = |delta: &SignedDelta| Signer {
            peer: delta.peer,
            user: delta.user,
        };
        let mut verdicts = Vec::with_capacity(deltas.len());
        let mut open = None;
        if let Some(opening) = self
            .opening
            .values_mut()
            .flatten()
            .find(|o| o.expects(document_id))
        {
            // A segment still opening is checked once it is ready, against
            // its registry then.
            for delta in deltas {
                let kept = opening.receive_signed(
                    document_id,
                    Seq(delta.seq),
                    &delta.payload,
                    signer(delta),
                );
                verdicts.push(kept.map(|()| Vec::new()));
            }
        } else if let Some((&channel_id, channel)) = self
            .channels
            .iter_mut()
            .find(|(_, channel)| channel.holds(document_id))
        {
            for delta in deltas {
                verdicts.push(channel.apply_signed(
                    document_id,
                    Seq(delta.seq),
                    &delta.payload,
                    signer(delta),
                ));
            }
            open = Some(channel_id);
        } else {
            return false;
        }
        let mut events = Vec::new();
        let mut outdated = false;
        for (delta, verdict) in deltas.iter().zip(verdicts) {
            let accepted = verdict.is_ok();
            self.record_channel_seq(
                document_id,
                delta.seq,
                accepted.then_some(delta.payload.as_slice()),
            );
            match verdict {
                Ok(produced) => events.extend(produced),
                Err(error) => {
                    outdated |= matches!(&error, MurmureError::Refused(r) if r.asks_update());
                    // The refusal names the rule, never a byte of the
                    // payload (`ZK-001`).
                    tracing::warn!(
                        "Logic: refused a delta on doc {document_id} (user {:?}): {error}",
                        delta.user
                    );
                }
            }
        }
        if outdated {
            self.send_to_ui(UiEvent::UpdateRequired);
        }
        // One segment answers with its whole list, so only the last answer of
        // a batch is news to the view.
        if let (Some(channel_id), Some(event)) = (open, events.pop()) {
            self.send_to_ui(UiEvent::Channel { channel_id, event });
            self.emit_count(channel_id);
        }
        true
    }

    /// Search every open channel for `query` (the Murmure search spec). Each
    /// engine reads the segments it holds, and nothing reaches the Sync
    /// worker: the query never leaves the client, and no segment loads.
    pub(crate) fn search_messages(&self, query: String) {
        let mut channels: Vec<ChannelMatches> = match Query::parse(&query) {
            Some(parsed) => self
                .channels
                .values()
                .map(|channel| channel.search(&parsed))
                .collect(),
            None => Vec::new(),
        };
        channels.sort_by_key(|matches| matches.channel_id.to_string());
        self.send_to_ui(UiEvent::MessageSearch { query, channels });
    }

    /// Hold an open channel, as the staged open does, without the relay's
    /// round trips. Each document is under its session identity, and the
    /// relay already holds it.
    #[cfg(test)]
    pub(crate) fn hold_channel(&mut self, channel: ChannelState) {
        let documents = channel.document_ids();
        self.channels.insert(channel.channel_id(), channel);
        for document_id in documents {
            self.grant_identity(document_id);
            self.unsent.remove(&document_id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ambre_bus::{
        ActivityKind, MentionTarget, MessageDraft, MurmureEvent, RestEvent, SegmentId, Timestamp,
    };
    use ambre_murmure::ChannelType;

    const NOW: Timestamp = Timestamp(1_700_000_000_000);

    fn harness() -> (LogicState, Receiver<UiEvent>, Receiver<SyncCommand>) {
        let (state, ui, sync, store) = harness_with_store();
        std::mem::forget(store);
        (state, ui, sync)
    }

    /// What Logic hands the local store.
    type StoreRx = Receiver<(DocumentId, PersistWrite)>;

    /// The harness, with what Logic hands the local store.
    fn harness_with_store() -> (
        LogicState,
        Receiver<UiEvent>,
        Receiver<SyncCommand>,
        StoreRx,
    ) {
        let (to_ui, ui_rx) = test_link();
        let (to_sync, sync_rx) = test_link();
        let (to_persist, persist_rx) = test_link();
        (
            LogicState::new(to_ui, to_sync, to_persist),
            ui_rx,
            sync_rx,
            persist_rx,
        )
    }

    fn signed_in(state: &mut LogicState, user: UserId) {
        state.handle_logic_event(LogicEvent::Rest {
            id: RequestId(0),
            evt: RestEvent::SessionReady {
                user_id: user,
                display_name: "alice".to_owned(),
                email: "alice@example.com".to_owned(),
            },
        });
    }

    fn open_channel(state: &mut LogicState) -> (ChannelId, SegmentId) {
        let channel_id = ChannelId::new();
        let channel =
            ChannelState::create(channel_id, ChannelType::Public, None, NOW).expect("create");
        let active = channel.active_id();
        state.hold_channel(channel);
        (channel_id, active)
    }

    fn send(text: &str, mentions: Vec<MentionTarget>) -> MurmureCommand {
        MurmureCommand::SendMessage {
            draft: MessageDraft {
                text: text.to_owned(),
                mentions,
            },
            now: NOW,
        }
    }

    #[test]
    fn a_channel_command_reaches_its_engine_and_leaves_as_a_delta_and_an_event() {
        let (mut state, ui, sync) = harness();
        let alice = UserId::new();
        signed_in(&mut state, alice);
        let _ = std::iter::from_fn(|| ui.try_recv().ok()).count();
        let (channel_id, active) = open_channel(&mut state);
        let bob = UserId::new();

        state.handle_logic_command(LogicCommand::Channel {
            channel_id,
            command: send("hi @bob", vec![MentionTarget::User(bob)]),
        });

        let sent: Vec<SyncCommand> = std::iter::from_fn(|| sync.try_recv().ok()).collect();
        let [
            SyncCommand::SendUpdate {
                document_id,
                payload,
                summary: Some(summary),
            },
        ] = sent.as_slice()
        else {
            panic!("one update with a summary, got {sent:?}");
        };
        assert_eq!(*document_id, active.document_id());
        assert!(!payload.is_empty());
        assert_eq!(summary.kind, ActivityKind::Content);
        assert_eq!(summary.mentions, [bob]);

        let shown: Vec<UiEvent> = std::iter::from_fn(|| ui.try_recv().ok()).collect();
        let [
            UiEvent::Channel {
                channel_id: shown_channel,
                event: MurmureEvent::Messages { segment, messages },
            },
        ] = shown.as_slice()
        else {
            panic!("one channel event, got {shown:?}");
        };
        assert_eq!(*shown_channel, channel_id);
        assert_eq!(*segment, active);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].author, alice);
        assert_eq!(messages[0].text, "hi @bob");
    }

    #[test]
    fn a_message_before_the_session_is_dropped() {
        let (mut state, ui, sync) = harness();
        let (channel_id, _) = open_channel(&mut state);
        state.handle_logic_command(LogicCommand::Channel {
            channel_id,
            command: send("too early", Vec::new()),
        });
        assert!(sync.try_recv().is_err());
        assert!(ui.try_recv().is_err());
    }

    /// A message sent before its segment's identity lands waits. It leaves
    /// once the segment went up whole, after that first update, and both are
    /// written under the session's PeerID alone (`AUTHOR-026`, `AUTHOR-029`).
    #[test]
    fn a_message_before_the_identity_waits_then_leaves_under_it() {
        let (mut state, ui, sync) = harness();
        signed_in(&mut state, UserId::new());
        let _ = drain(&ui);
        let channel_id = ChannelId::new();
        let channel =
            ChannelState::create(channel_id, ChannelType::Public, None, NOW).expect("create");
        let active = channel.active_id().document_id();
        state.hold_created(channel);
        let _ = drain(&sync);

        state.handle_logic_command(LogicCommand::Channel {
            channel_id,
            command: send("early", Vec::new()),
        });
        assert!(
            drain(&sync).is_empty(),
            "nothing leaves before the identity"
        );
        assert!(drain(&ui).is_empty(), "nothing is written either");

        let peer = state.grant_identity(active);
        assert!(
            drain(&sync).is_empty(),
            "nothing leaves before the Sync worker holds the document"
        );
        state.handle_logic_event(LogicEvent::DocumentReady {
            document_id: active,
            role: ambre_bus::Role::Owner,
        });
        let sent = drain(&sync);
        let [
            SyncCommand::SendUpdate {
                document_id: first_to,
                payload: first,
                summary: None,
            },
            SyncCommand::SendUpdate {
                document_id: message_to,
                payload: message,
                summary: Some(_),
            },
        ] = sent.as_slice()
        else {
            panic!("the whole segment, then the message, got {sent:?}");
        };
        assert_eq!((*first_to, *message_to), (active, active));
        assert!(crate::sent_by(first, peer));
        assert!(crate::sent_by(message, peer));
        assert!(drain(&ui).iter().any(|e| matches!(
            e,
            UiEvent::Channel { event: MurmureEvent::Messages { messages, .. }, .. }
                if messages.len() == 1 && messages[0].text == "early"
        )));
    }

    #[test]
    fn a_remote_delta_reaches_the_channel_that_holds_its_document() {
        let (mut alice, _, alice_sync) = harness();
        let author = UserId::new();
        signed_in(&mut alice, author);
        let channel_id = ChannelId::new();
        alice.hold_channel(
            ChannelState::create(channel_id, ChannelType::Public, None, NOW).expect("create"),
        );
        // Bob copies the documents as they leave alice: under her PeerIDs.
        let original = &alice.channels[&channel_id];
        let active = original.active_id();
        let segments = std::collections::HashMap::from([(
            active,
            original.snapshot(active.document_id()).expect("active"),
        )]);
        let meta = original
            .snapshot(original.meta_id().document_id())
            .expect("meta");
        let copy =
            ChannelState::open(channel_id, original.meta_id(), &meta, &segments).expect("open");
        let (mut bob, bob_ui, _) = harness();
        bob.hold_channel(copy);

        alice.handle_logic_command(LogicCommand::Channel {
            channel_id,
            command: send("over the wire", Vec::new()),
        });
        let Ok(SyncCommand::SendUpdate {
            document_id,
            payload,
            ..
        }) = alice_sync.try_recv()
        else {
            panic!("alice sends one update");
        };
        // The PeerID Sync would derive from alice's signer.
        let peer = alice.channels[&channel_id]
            .peer(document_id)
            .expect("a session");
        bob.handle_logic_event(LogicEvent::RemoteUpdate {
            document_id,
            delta: ambre_bus::SignedDelta {
                seq: 1,
                payload,
                nonce: ambre_core::SessionNonce([0; 16]),
                peer,
                user: Some(author),
                summary: None,
            },
        });

        let Ok(UiEvent::Channel {
            event: MurmureEvent::Messages { messages, .. },
            ..
        }) = bob_ui.try_recv()
        else {
            panic!("bob's view hears of the message");
        };
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].text, "over the wire");
    }

    /// An open channel keeps its documents while its view shows it (the
    /// Murmure engine spec). Closing one channel releases its documents alone, as
    /// light subscriptions, and the other stays whole.
    #[test]
    fn closing_a_channel_releases_its_documents_and_no_others() {
        let (mut state, _, sync) = harness();
        let (channel_id, active) = open_channel(&mut state);
        let (shown, shown_active) = open_channel(&mut state);
        let released = state.channels[&channel_id].document_ids();
        drain(&sync);
        state.handle_logic_command(LogicCommand::CloseChannel { channel_id });
        assert!(!state.channels.contains_key(&channel_id));
        assert!(!state.apply_channel_remote(active.document_id(), &[(None, &[])]));
        let sent = sent(&sync);
        assert_eq!(
            sent,
            released
                .iter()
                .map(|id| Sent::Release(*id))
                .collect::<Vec<_>>()
        );
        assert!(state.channels[&shown].holds(shown_active.document_id()));
        assert!(!sent.contains(&Sent::Release(shown_active.document_id())));
    }

    /// A closed channel with a cursor on each held segment: its count at
    /// close was zero.
    fn closed_counted(state: &mut LogicState) -> (ChannelId, SegmentId, MetaId) {
        signed_in(state, UserId::new());
        let (channel_id, active) = open_channel(state);
        let meta = state.channels[&channel_id].meta_id();
        state.read_cursors.insert(
            active.document_id(),
            ReadCursor {
                read: Seq(0),
                active: false,
            },
        );
        state.handle_logic_command(LogicCommand::CloseChannel { channel_id });
        (channel_id, active, meta)
    }

    fn activity(content: u32) -> ambre_core::activity::ActivityCounters {
        ambre_core::activity::ActivityCounters {
            active: true,
            content,
            ..Default::default()
        }
    }

    /// The summaries of a closed channel's documents give it a count, over
    /// the count it had at close (the Murmure unread spec). A document's own
    /// badge is not shown, and an update with no message leaves the count.
    #[test]
    fn a_closed_channel_counts_its_unread_from_the_summaries() {
        let (mut state, ui, _sync) = harness();
        let (channel_id, active, meta) = closed_counted(&mut state);
        drain(&ui);
        for (document_id, content) in [
            (active.document_id(), 2),
            (active.document_id(), 3),
            (meta.document_id(), 0),
        ] {
            state.handle_logic_event(LogicEvent::Activity {
                document_id,
                counters: activity(content),
            });
        }
        let marks: Vec<_> = drain(&ui)
            .into_iter()
            .map(|event| match event {
                UiEvent::ChannelActivity { channels } => channels,
                other => panic!("a document badge leaked: {other:?}"),
            })
            .collect();
        let mark = |unread| {
            vec![ChannelActivity {
                channel_id,
                active: true,
                unread: Some(UnreadCount(unread)),
            }]
        };
        assert_eq!(marks, vec![mark(2), mark(3), mark(3)]);
    }

    /// A channel whose count was unknown at close keeps its mark until a
    /// summary counts a message, then is marked with no count.
    #[test]
    fn a_closed_channel_with_no_count_is_marked_by_a_counted_message() {
        let (mut state, ui, _sync) = harness();
        signed_in(&mut state, UserId::new());
        let (channel_id, active) = open_channel(&mut state);
        state.handle_logic_command(LogicCommand::CloseChannel { channel_id });
        drain(&ui);
        state.handle_logic_event(LogicEvent::Activity {
            document_id: active.document_id(),
            counters: activity(0),
        });
        assert!(
            drain(&ui).is_empty(),
            "an update with no message moved the mark"
        );
        state.handle_logic_event(LogicEvent::Activity {
            document_id: active.document_id(),
            counters: activity(1),
        });
        let shown = drain(&ui);
        let [UiEvent::ChannelActivity { channels }] = shown.as_slice() else {
            panic!("expected the channel's mark, got {shown:?}");
        };
        assert_eq!(
            channels,
            &[ChannelActivity {
                channel_id,
                active: true,
                unread: None,
            }]
        );
    }

    /// Holds `RESID-013`: a reopen asks the relay for the channel and moves
    /// no read cursor. Until it opens, its summaries still count it.
    #[test]
    fn reopening_a_channel_leaves_its_read_cursor_where_it_was() {
        let (mut state, ui, sync) = harness();
        let (channel_id, active, _) = closed_counted(&mut state);
        let cursors = state.read_cursors.clone();
        drain(&sync);
        state.handle_logic_command(LogicCommand::OpenChannel { channel_id });
        assert_eq!(sent(&sync), [Sent::List(channel_id)]);
        assert_eq!(state.read_cursors, cursors);
        drain(&ui);
        state.handle_logic_event(LogicEvent::Activity {
            document_id: active.document_id(),
            counters: activity(1),
        });
        assert!(
            matches!(drain(&ui).as_slice(), [UiEvent::ChannelActivity { .. }]),
            "a reopening channel stopped counting from its summaries"
        );
    }

    /// A closed channel reopens from the copy of its documents the local
    /// store holds (the Murmure engine spec). Each copy reaches the store at close,
    /// and the light subscription keeps the horizon it covers. The reopen
    /// reads the copy back before the Sync worker opens the document, so
    /// the relay replays only what follows. A document with no copy is
    /// fetched whole.
    #[test]
    fn a_closed_channel_reopens_from_its_local_copy() {
        let (mut state, ui, sync, store) = harness_with_store();
        let (channel_id, stored) = stored_channel();
        let meta = stored.meta_id().document_id();
        let active = stored.active_id().document_id();
        let listed = || LogicEvent::ChannelDocuments {
            channel_id,
            documents: listing(&stored),
        };
        state.handle_logic_command(LogicCommand::OpenChannel { channel_id });
        state.handle_logic_event(listed());
        serve(&mut state, &stored, meta);
        serve(&mut state, &stored, active);
        drain(&sync);
        drain(&store);

        state.handle_logic_command(LogicCommand::CloseChannel { channel_id });
        let copies: HashMap<DocumentId, Vec<u8>> = drain(&store)
            .into_iter()
            .filter_map(|(id, write)| match write {
                PersistWrite::Snapshot(copy) => Some((id, copy)),
                _ => None,
            })
            .collect();
        assert_eq!(copies.len(), 2, "each document reaches the store");
        let horizons: HashMap<DocumentId, u64> = drain(&sync)
            .into_iter()
            .filter_map(|command| match command {
                SyncCommand::ReleaseDocument {
                    document_id,
                    applied_seq,
                } => Some((document_id, applied_seq)),
                _ => None,
            })
            .collect();
        assert_eq!(
            horizons,
            HashMap::from([(meta, 1), (active, 1)]),
            "each light subscription keeps its horizon"
        );

        state.handle_logic_command(LogicCommand::OpenChannel { channel_id });
        state.handle_logic_event(listed());
        assert_eq!(
            sent(&sync),
            [Sent::List(channel_id)],
            "meta waits on its copy"
        );
        let read_back = |rx: &Receiver<(DocumentId, PersistWrite)>, id: DocumentId| {
            matches!(
                drain(rx).as_slice(),
                [(asked, PersistWrite::ReadBack { doc_type: DocType::Murmure })] if *asked == id
            )
        };
        assert!(read_back(&store, meta));
        state.handle_logic_command(LogicCommand::RestoreLocalSnapshot {
            document_id: meta,
            doc_type: DocType::Murmure,
            snapshot: Some(copies[&meta].clone()),
            deltas: Vec::new(),
            sessions: Vec::new(),
        });
        assert_eq!(sent(&sync), [Sent::Open(meta)]);
        // The relay holds nothing past the horizon: only its ready comes.
        state.handle_logic_event(LogicEvent::DocumentReady {
            document_id: meta,
            role: ambre_bus::Role::Editor,
        });
        assert!(read_back(&store, active));
        state.handle_logic_command(LogicCommand::RestoreLocalSnapshot {
            document_id: active,
            doc_type: DocType::Murmure,
            snapshot: None,
            deltas: Vec::new(),
            sessions: Vec::new(),
        });
        assert_eq!(
            sent(&sync),
            [Sent::Close(active), Sent::Open(active)],
            "a document with no copy is fetched whole"
        );
        drain(&ui);
        serve(&mut state, &stored, active);
        assert!(drain(&ui).iter().any(|event| matches!(
            event,
            UiEvent::Channel { event: MurmureEvent::Messages { messages, .. }, .. }
                if messages.first().is_some_and(|m| m.text == "stored")
        )));
        assert!(state.channels.contains_key(&channel_id));
        assert!(!state.closed_channels.contains_key(&channel_id));
    }

    /// The relay removing the user from a closed channel ends its light
    /// subscriptions and forgets its read cursors.
    #[test]
    fn a_closed_channel_the_user_left_ends_its_light_subscriptions() {
        let (mut state, _ui, sync) = harness();
        let (channel_id, active, meta) = closed_counted(&mut state);
        drain(&sync);
        state.channel_revoked(meta.document_id());
        assert!(!state.closed_channels.contains_key(&channel_id));
        assert!(!state.read_cursors.contains_key(&active.document_id()));
        assert!(sent(&sync).contains(&Sent::Close(active.document_id())));
    }

    // --- The staged open over the relay ---

    fn drain<T>(rx: &Receiver<T>) -> Vec<T> {
        std::iter::from_fn(|| rx.try_recv().ok()).collect()
    }

    /// A channel as the relay stores it: one message in its active segment.
    fn stored_channel() -> (ChannelId, ChannelState) {
        let channel_id = ChannelId::new();
        let mut channel = crate::identified(
            ChannelState::create(channel_id, ChannelType::Public, None, NOW).expect("create"),
        );
        channel
            .send(
                &MessageDraft {
                    text: "stored".to_owned(),
                    mentions: Vec::new(),
                },
                UserId::new(),
                NOW,
            )
            .expect("send");
        (channel_id, channel)
    }

    fn listing(channel: &ChannelState) -> Vec<ChannelDocument> {
        vec![
            ChannelDocument {
                document_id: channel.meta_id().document_id(),
                kind: ChannelDocumentKind::Meta,
            },
            ChannelDocument {
                document_id: channel.active_id().document_id(),
                kind: ChannelDocumentKind::Segment,
            },
        ]
    }

    /// The relay serves a document: its snapshot, then its ready signal.
    fn serve(state: &mut LogicState, stored: &ChannelState, document_id: DocumentId) {
        state.handle_logic_event(LogicEvent::RemoteSnapshot {
            document_id,
            covered_seq: 1,
            payload: stored.snapshot(document_id).expect("snapshot"),
        });
        state.handle_logic_event(LogicEvent::DocumentReady {
            document_id,
            role: ambre_bus::Role::Editor,
        });
    }

    /// What Logic asked the Sync worker, reduced to the calls these tests
    /// follow: `SyncCommand` carries secrets, so it has no `PartialEq`.
    #[derive(Debug, PartialEq)]
    enum Sent {
        List(ChannelId),
        Open(DocumentId),
        Close(DocumentId),
        Release(DocumentId),
        Other,
    }

    fn sent(rx: &Receiver<SyncCommand>) -> Vec<Sent> {
        drain(rx)
            .into_iter()
            .map(|c| match c {
                SyncCommand::OpenChannel { channel_id, .. } => Sent::List(channel_id),
                SyncCommand::OpenDocument {
                    document_id,
                    doc_type: DocType::Murmure,
                } => Sent::Open(document_id),
                SyncCommand::CloseDocument { document_id } => Sent::Close(document_id),
                SyncCommand::ReleaseDocument { document_id, .. } => Sent::Release(document_id),
                _ => Sent::Other,
            })
            .collect()
    }

    #[test]
    fn a_channel_opens_its_meta_then_its_live_segments_then_shows_them() {
        let (mut state, ui, sync) = harness();
        let (channel_id, stored) = stored_channel();

        state.handle_logic_command(LogicCommand::OpenChannel { channel_id });
        assert_eq!(sent(&sync), [Sent::List(channel_id)]);

        state.handle_logic_event(LogicEvent::ChannelDocuments {
            channel_id,
            documents: listing(&stored),
        });
        assert_eq!(
            sent(&sync),
            [Sent::Open(stored.meta_id().document_id())],
            "meta first"
        );

        serve(&mut state, &stored, stored.meta_id().document_id());
        assert_eq!(sent(&sync), [Sent::Open(stored.active_id().document_id())]);
        assert!(drain(&ui).is_empty(), "nothing of the channel shows yet");

        serve(&mut state, &stored, stored.active_id().document_id());
        let shown = drain(&ui);
        let [
            UiEvent::Channel {
                event: MurmureEvent::Named { name },
                ..
            },
            UiEvent::Channel {
                channel_id: shown_channel,
                event: MurmureEvent::Messages { segment, messages },
            },
        ] = shown.as_slice()
        else {
            panic!("the channel's name, then its active segment's messages, got {shown:?}");
        };
        assert_eq!(*name, stored.name());
        assert_eq!(*shown_channel, channel_id);
        assert_eq!(*segment, stored.active_id());
        assert_eq!(messages[0].text, "stored");
        assert!(state.channels.contains_key(&channel_id));
        assert!(state.opening.is_empty());
    }

    /// A channel no catalogue of this client lists admits invites alone, so
    /// its member list counts no self-join.
    #[test]
    fn a_channel_no_catalogue_lists_admits_no_self_join() {
        let (mut state, _, sync) = harness();
        let channel_id = ChannelId::new();
        state.handle_logic_command(LogicCommand::OpenChannel { channel_id });
        assert!(matches!(
            drain(&sync).as_slice(),
            [SyncCommand::OpenChannel {
                self_join: false,
                ..
            }]
        ));
    }

    #[test]
    fn a_refused_listing_tells_the_view_and_holds_nothing() {
        let (mut state, ui, _) = harness();
        let channel_id = ChannelId::new();
        state.handle_logic_command(LogicCommand::OpenChannel { channel_id });
        state.handle_logic_event(LogicEvent::ChannelOpenFailed {
            channel_id,
            message: "403".to_owned(),
        });
        assert!(matches!(
            drain(&ui).as_slice(),
            [UiEvent::Channel {
                event: MurmureEvent::OpenFailed { .. },
                ..
            }]
        ));
        assert!(state.opening.is_empty());
    }

    /// A listing refused for want of membership drops the channel: the view
    /// learns it is gone, not that it failed. A second refusal, or one for a
    /// channel never opened, tells nothing.
    #[test]
    fn a_channel_the_relay_refuses_to_open_is_dropped() {
        let (mut state, ui, sync) = harness();
        let channel_id = ChannelId::new();
        state.handle_logic_command(LogicCommand::OpenChannel { channel_id });
        let _ = drain(&sync);
        state.handle_logic_event(LogicEvent::ChannelRefused { channel_id });
        assert!(matches!(
            drain(&ui).as_slice(),
            [UiEvent::Channel {
                channel_id: id,
                event: MurmureEvent::Left,
            }] if *id == channel_id
        ));
        assert!(state.opening.is_empty());
        assert!(drain(&sync).is_empty(), "no session was open to end");

        state.handle_logic_event(LogicEvent::ChannelRefused { channel_id });
        state.handle_logic_event(LogicEvent::ChannelRefused {
            channel_id: ChannelId::new(),
        });
        assert!(drain(&ui).is_empty());
    }

    #[test]
    fn a_document_that_fails_to_open_ends_the_opening() {
        let (mut state, ui, sync) = harness();
        let (channel_id, stored) = stored_channel();
        state.handle_logic_command(LogicCommand::OpenChannel { channel_id });
        state.handle_logic_event(LogicEvent::ChannelDocuments {
            channel_id,
            documents: listing(&stored),
        });
        let _ = drain(&sync);

        state.handle_logic_event(LogicEvent::DocumentOpenFailed {
            document_id: stored.meta_id().document_id(),
            message: "cannot unwrap document key".to_owned(),
        });
        assert_eq!(sent(&sync), [Sent::Close(stored.meta_id().document_id())]);
        assert!(
            matches!(
                drain(&ui).as_slice(),
                [UiEvent::Channel {
                    event: MurmureEvent::OpenFailed { .. },
                    ..
                }]
            ),
            "the channel fails, and no bare auth error"
        );
        assert!(state.opening.is_empty());
    }

    #[test]
    fn a_meta_of_another_channel_ends_the_opening() {
        let (mut state, ui, sync) = harness();
        let (_, stored) = stored_channel();
        let channel_id = ChannelId::new();
        state.handle_logic_command(LogicCommand::OpenChannel { channel_id });
        state.handle_logic_event(LogicEvent::ChannelDocuments {
            channel_id,
            documents: listing(&stored),
        });
        let _ = drain(&sync);
        serve(&mut state, &stored, stored.meta_id().document_id());
        assert_eq!(sent(&sync), [Sent::Close(stored.meta_id().document_id())]);
        assert!(matches!(
            drain(&ui).as_slice(),
            [UiEvent::Channel {
                event: MurmureEvent::OpenFailed { .. },
                ..
            }]
        ));
        assert!(state.channels.is_empty());
    }

    #[test]
    fn closing_a_channel_still_opening_ends_its_documents() {
        let (mut state, _, sync) = harness();
        let (channel_id, stored) = stored_channel();
        state.handle_logic_command(LogicCommand::OpenChannel { channel_id });
        state.handle_logic_event(LogicEvent::ChannelDocuments {
            channel_id,
            documents: listing(&stored),
        });
        let _ = drain(&sync);
        state.handle_logic_command(LogicCommand::CloseChannel { channel_id });
        assert_eq!(sent(&sync), [Sent::Close(stored.meta_id().document_id())]);
        // A listing that lands after the close is stale.
        state.handle_logic_event(LogicEvent::ChannelDocuments {
            channel_id,
            documents: listing(&stored),
        });
        assert!(drain(&sync).is_empty());
    }

    #[test]
    fn asking_for_the_segment_before_the_oldest_asks_nothing() {
        let (mut state, _, sync) = harness();
        let (channel_id, active) = open_channel(&mut state);
        state.handle_logic_command(LogicCommand::Channel {
            channel_id,
            command: MurmureCommand::LoadOlder { segment: active },
        });
        assert!(drain(&sync).is_empty());
    }

    #[test]
    fn marking_a_channel_read_moves_its_live_segments_and_clears_its_mark() {
        let (mut state, ui, sync) = harness();
        signed_in(&mut state, UserId::new());
        let _ = drain(&ui);
        let (channel_id, active) = open_channel(&mut state);

        state.handle_logic_command(LogicCommand::Channel {
            channel_id,
            command: MurmureCommand::MarkRead,
        });

        let sent = drain(&sync);
        let [SyncCommand::MarkRead { live, finals }] = sent.as_slice() else {
            panic!("one mark read, got {sent:?}");
        };
        assert_eq!(*live, [active]);
        assert!(finals.is_empty());
        let shown: Vec<UiEvent> = std::iter::from_fn(|| ui.try_recv().ok()).collect();
        let [UiEvent::ChannelActivity { channels }] = shown.as_slice() else {
            panic!("one activity event, got {shown:?}");
        };
        assert_eq!(
            *channels,
            [ChannelActivity {
                channel_id,
                active: false,
                unread: Some(UnreadCount::ZERO),
            }]
        );
    }

    #[test]
    fn an_activity_answer_marks_each_channel_once_and_opens_nothing() {
        let (mut state, ui, sync) = harness();
        let (quiet, busy) = (ChannelId::new(), ChannelId::new());
        let entry = |channel_id, active| DocumentActivity {
            channel_id,
            document_id: DocumentId::new(),
            active,
            read: Seq::ZERO,
        };

        state.handle_logic_event(LogicEvent::ChannelActivity {
            documents: vec![
                entry(busy, false),
                entry(quiet, false),
                entry(busy, true),
                entry(quiet, false),
            ],
        });

        let shown: Vec<UiEvent> = std::iter::from_fn(|| ui.try_recv().ok()).collect();
        let [UiEvent::ChannelActivity { channels }] = shown.as_slice() else {
            panic!("one activity event, got {shown:?}");
        };
        assert_eq!(
            *channels,
            [
                ChannelActivity {
                    channel_id: busy,
                    active: true,
                    unread: None,
                },
                ChannelActivity {
                    channel_id: quiet,
                    active: false,
                    unread: None,
                },
            ]
        );
        assert!(
            drain(&sync).is_empty(),
            "an activity answer opens no document"
        );
    }

    /// The view's ask for the user's channels reaches Sync, and the answer
    /// reaches the view whole. It opens no channel and asks for no document.
    #[test]
    fn the_users_channels_reach_the_view_and_open_nothing() {
        let (mut state, ui, sync) = harness();
        state.handle_logic_command(LogicCommand::ListMemberChannels);
        let sent = drain(&sync);
        assert!(
            matches!(sent.as_slice(), [SyncCommand::ListMemberChannels]),
            "got {sent:?}"
        );

        let channels = vec![
            ambre_bus::MemberChannel {
                channel_id: ChannelId::new(),
                place: ambre_bus::ChannelPlace::Private(ambre_bus::SpaceId::new()),
            },
            ambre_bus::MemberChannel {
                channel_id: ChannelId::new(),
                place: ambre_bus::ChannelPlace::Direct,
            },
        ];
        state.handle_logic_event(LogicEvent::MemberChannels {
            channels: channels.clone(),
        });
        let shown: Vec<UiEvent> = std::iter::from_fn(|| ui.try_recv().ok()).collect();
        assert!(
            matches!(shown.as_slice(), [UiEvent::MemberChannels { channels: c }] if *c == channels),
            "got {shown:?}"
        );
        assert!(
            drain(&sync).is_empty(),
            "learning of a channel opens nothing"
        );
        assert!(state.channels.is_empty(), "no channel state is held");
    }

    #[test]
    fn the_activity_commands_reach_the_sync_worker() {
        let (mut state, _, sync) = harness();
        state.handle_logic_command(LogicCommand::FetchActivity);
        state.handle_logic_command(LogicCommand::MarkAllRead);
        let sent = drain(&sync);
        assert!(
            matches!(
                sent.as_slice(),
                [SyncCommand::FetchActivity, SyncCommand::MarkAllRead]
            ),
            "got {sent:?}"
        );
    }

    // --- Unread counts and mark unread ---

    /// Alice writes on the engine; Bob is a signed-in client holding a copy
    /// of the same channel.
    struct Members {
        alice: ChannelState,
        /// The user Alice writes as, and signs as.
        author: UserId,
        bob: LogicState,
        bob_ui: Receiver<UiEvent>,
        bob_sync: Receiver<SyncCommand>,
        channel_id: ChannelId,
        active: SegmentId,
    }

    fn members() -> Members {
        let channel_id = ChannelId::new();
        let alice = crate::identified(
            ChannelState::create(channel_id, ChannelType::Public, None, NOW).expect("create"),
        );
        let active = alice.active_id();
        let segments = std::collections::HashMap::from([(
            active,
            alice.snapshot(active.document_id()).expect("active"),
        )]);
        let meta = alice.snapshot(alice.meta_id().document_id()).expect("meta");
        let copy = ChannelState::open(channel_id, alice.meta_id(), &meta, &segments).expect("open");
        let (mut bob, bob_ui, bob_sync) = harness();
        signed_in(&mut bob, UserId::new());
        bob.hold_channel(copy);
        let _ = drain(&bob_ui);
        Members {
            alice,
            author: UserId::new(),
            bob,
            bob_ui,
            bob_sync,
            channel_id,
            active,
        }
    }

    impl Members {
        /// Alice sends `text`; the relay delivers it to Bob as update `seq`.
        fn deliver(&mut self, text: &str, seq: u64) {
            let sent = self
                .alice
                .send(
                    &MessageDraft {
                        text: text.to_owned(),
                        mentions: Vec::new(),
                    },
                    self.author,
                    NOW,
                )
                .expect("send");
            let document_id = sent.segment.document_id();
            self.bob.handle_logic_event(LogicEvent::RemoteUpdate {
                document_id,
                delta: ambre_bus::SignedDelta {
                    peer: self.alice.peer(document_id).expect("a session"),
                    user: Some(self.author),
                    ..crate::remote_delta(seq, sent.delta)
                },
            });
        }

        /// The relay's activity answer: Bob's cursor on the active segment.
        fn answer(&mut self, read: u64) {
            self.bob.handle_logic_event(LogicEvent::ChannelActivity {
                documents: vec![DocumentActivity {
                    channel_id: self.channel_id,
                    document_id: self.active.document_id(),
                    active: true,
                    read: Seq(read),
                }],
            });
        }

        /// Every count the view was handed for the channel since the last
        /// call.
        fn counts(&self) -> Vec<Option<u32>> {
            drain(&self.bob_ui)
                .into_iter()
                .filter_map(|e| match e {
                    UiEvent::ChannelActivity { channels } => Some(channels),
                    _ => None,
                })
                .flatten()
                .filter(|c| c.channel_id == self.channel_id)
                .map(|c| c.unread.map(|count| count.0))
                .collect()
        }

        fn message_ids(&self) -> Vec<MessageId> {
            self.bob.channels[&self.channel_id]
                .messages(self.active)
                .expect("messages")
                .iter()
                .map(|m| m.id)
                .collect()
        }

        fn mark_unread(&mut self, message: MessageId) {
            self.bob.handle_logic_command(LogicCommand::Channel {
                channel_id: self.channel_id,
                command: MurmureCommand::MarkUnread {
                    segment: self.active,
                    message,
                },
            });
        }

        /// The cursors Bob asked the relay to move since the last call.
        fn moved(&self) -> Vec<Vec<(SegmentId, Seq)>> {
            drain(&self.bob_sync)
                .into_iter()
                .filter_map(|c| match c {
                    SyncCommand::MoveReadCursors { cursors } => Some(cursors),
                    _ => None,
                })
                .collect()
        }
    }

    /// Each delivered message carries the relay's number, and the count
    /// reads it against the cursor.
    #[test]
    fn a_delivered_message_carries_the_relay_number_its_count_reads() {
        let mut m = members();
        m.answer(0);
        m.deliver("one", 5);
        m.deliver("two", 6);
        let ids = m.message_ids();
        let channel = &m.bob.channels[&m.channel_id];
        assert_eq!(
            ids.iter()
                .map(|id| channel.position(m.active, *id))
                .collect::<Vec<_>>(),
            [Some(Position::Exact(Seq(5))), Some(Position::Exact(Seq(6)))]
        );
    }

    /// An open channel's count comes from its messages and the cursor, and
    /// none shows while the cursor is unknown.
    #[test]
    fn an_open_channel_counts_the_messages_past_its_cursor() {
        let mut m = members();
        m.deliver("before any answer", 1);
        assert_eq!(m.counts(), [], "no cursor yet, so no count");

        m.answer(0);
        assert_eq!(m.counts(), [Some(1)]);
        m.deliver("two", 2);
        assert_eq!(m.counts(), [Some(2)]);
        m.answer(1);
        assert_eq!(m.counts(), [Some(1)]);
    }

    /// Marking a message unread moves the cursor to just before it, so it
    /// and every later message count again.
    #[test]
    fn marking_a_message_unread_moves_the_cursor_to_just_before_it() {
        let mut m = members();
        m.deliver("one", 5);
        m.deliver("two", 6);
        m.answer(6);
        assert_eq!(m.counts(), [Some(0)]);
        let ids = m.message_ids();

        m.mark_unread(ids[1]);
        assert_eq!(m.moved(), [vec![(m.active, Seq(5))]]);
        assert_eq!(m.counts(), [Some(1)]);

        m.mark_unread(ids[0]);
        assert_eq!(m.moved(), [vec![(m.active, Seq(4))]]);
        assert_eq!(m.counts(), [Some(2)]);
    }

    /// A delta passes the engine's authorship check before it is imported,
    /// and a refused one changes nothing the view shows (the Murmure engine
    /// spec).
    #[test]
    fn a_delta_the_authorship_check_refuses_reaches_no_view() {
        let mut m = members();
        let forged = m
            .alice
            .send(
                &MessageDraft {
                    text: "as somebody else".to_owned(),
                    mentions: Vec::new(),
                },
                UserId::new(),
                NOW,
            )
            .expect("send");
        let document_id = forged.segment.document_id();
        m.bob.handle_logic_event(LogicEvent::RemoteUpdate {
            document_id,
            delta: ambre_bus::SignedDelta {
                peer: m.alice.peer(document_id).expect("a session"),
                user: Some(m.author),
                ..crate::remote_delta(1, forged.delta)
            },
        });
        assert!(m.message_ids().is_empty());
        // Refused, and still delivered: the count reads past it.
        assert_eq!(m.bob.channels[&m.channel_id].delivered(document_id), Seq(1));
        assert!(
            !drain(&m.bob_ui)
                .iter()
                .any(|e| matches!(e, UiEvent::Channel { .. } | UiEvent::UpdateRequired)),
            "nothing reaches the view"
        );
    }

    /// A delta from a newer build is refused, and the user is asked to
    /// update (the Murmure messages spec).
    #[test]
    fn a_delta_from_a_newer_build_asks_the_user_to_update() {
        let mut m = members();
        let sent = m
            .alice
            .send(
                &MessageDraft {
                    text: "from the future".to_owned(),
                    mentions: Vec::new(),
                },
                m.author,
                NOW,
            )
            .expect("send");
        let mut update: serde_json::Value = serde_json::from_slice(&sent.delta).expect("json");
        update["schema_version"] = serde_json::Value::from(ambre_murmure::JSON_SCHEMA_VERSION + 1);
        let document_id = sent.segment.document_id();
        m.bob.handle_logic_event(LogicEvent::RemoteUpdate {
            document_id,
            delta: ambre_bus::SignedDelta {
                peer: m.alice.peer(document_id).expect("a session"),
                user: Some(m.author),
                ..crate::remote_delta(1, serde_json::to_vec(&update).expect("bytes"))
            },
        });
        assert!(m.message_ids().is_empty());
        assert!(
            drain(&m.bob_ui)
                .iter()
                .any(|e| matches!(e, UiEvent::UpdateRequired))
        );
    }

    /// A message the relay has not numbered exactly cannot place a cursor.
    #[test]
    fn a_message_with_no_exact_number_is_not_marked_unread() {
        let mut m = members();
        m.answer(0);
        m.mark_unread(MessageId::new());
        assert_eq!(m.moved(), Vec::<Vec<(SegmentId, Seq)>>::new());
    }

    /// Marking a message unread moves no cursor of another channel.
    #[test]
    fn marking_a_message_unread_moves_no_cursor_of_another_channel() {
        let mut m = members();
        let (other, other_active) = open_channel(&mut m.bob);
        m.deliver("one", 5);
        let ids = m.message_ids();
        let other_documents = m.bob.channels[&other].document_ids();

        m.mark_unread(ids[0]);
        let moved = m.moved();
        assert_eq!(moved, [vec![(m.active, Seq(4))]]);
        assert!(
            moved
                .iter()
                .flatten()
                .all(|(segment, _)| !other_documents.contains(&segment.document_id())),
            "no document of {other} (active {other_active}) moves"
        );
    }

    /// A search answers from every open channel, each with the segments it
    /// read, and asks the Sync worker nothing (the Murmure search spec).
    #[test]
    fn a_search_covers_every_open_channel_and_leaves_nothing() {
        let (mut state, ui, sync) = harness();
        signed_in(&mut state, UserId::new());
        let first = open_channel(&mut state);
        let second = open_channel(&mut state);
        for ((channel_id, _), text) in [(first, "the budget"), (second, "lunch")] {
            state.handle_logic_command(LogicCommand::Channel {
                channel_id,
                command: send(text, Vec::new()),
            });
        }
        drain(&sync);
        drain(&ui);

        state.handle_logic_command(LogicCommand::SearchMessages {
            query: "Budget".to_owned(),
        });
        assert!(drain(&sync).is_empty(), "the query never leaves the client");
        let shown = drain(&ui);
        let [UiEvent::MessageSearch { query, channels }] = shown.as_slice() else {
            panic!("one answer, got {shown:?}");
        };
        assert_eq!(query, "Budget");
        let mut answered: Vec<(ChannelId, Vec<SegmentId>, usize, Vec<String>)> = channels
            .iter()
            .map(|c| {
                let texts = c.matches.iter().map(|m| m.message.text.clone()).collect();
                (c.channel_id, c.covered.clone(), c.listed, texts)
            })
            .collect();
        answered.sort_by_key(|(id, ..)| *id != first.0);
        assert_eq!(
            answered,
            [
                (first.0, vec![first.1], 1, vec!["the budget".to_owned()]),
                (second.0, vec![second.1], 1, Vec::new()),
            ]
        );

        state.handle_logic_command(LogicCommand::SearchMessages {
            query: " ? ".to_owned(),
        });
        assert!(matches!(
            drain(&ui).as_slice(),
            [UiEvent::MessageSearch { channels, .. }] if channels.is_empty()
        ));
    }
}
